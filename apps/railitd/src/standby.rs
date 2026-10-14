//! Bereitschaftsrechner (REQ-SVC-011, REQ-SVC-012): `railitd --standby URL` liest die
//! Journale eines Hauptrechners über `/v1/journal/stream` mit, prüft und rechnet jede Zeile
//! nach ([`railit_journal::Follower`]) und zeigt den so gewonnenen Zustand über dieselbe API
//! und denselben Bedienplatz an – ohne Feld, ohne Kern im Betrieb, ohne Bedienung.
//!
//! Jeden geprüften Stand quittiert er dem Hauptrechner (`/v1/journal/ack`). Arbeitet der
//! Hauptrechner mit Ausgabesperre, antwortet er mit einer Zusage: Bis zu deren Ablauf gibt er
//! nichts frei, was der Bereitschaftsrechner nicht kennt. Die Übernahme des Betriebs
//! (`--takeover-after-ms`, siehe [`crate::takeover`]) stützt sich darauf.
//!
//! Bricht die Verbindung ab, setzt der Bereitschaftsrechner an der letzten Position fort.
//! Passt das Journal dann nicht mehr (der Hauptrechner hat neu begonnen), liest er von vorn.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use railit_journal::Follower;
use railit_kernel::OwnedState;
use railit_plan::PlanView;
use railit_runtime::Snapshot;
use railit_types::Tick;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::watch;

/// Wartezeit vor einem neuen Verbindungsversuch.
const RETRY: Duration = Duration::from_secs(1);
/// Sicherheitsabstand zur Zusage des Hauptrechners (Gangunterschied der Uhren, Laufzeit).
const PROMISE_MARGIN: Duration = Duration::from_millis(200);

/// Stand des Mitlesens je Stellbereich.
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct AreaStatus {
    pub(crate) area: String,
    /// Strom vom Hauptrechner verbunden.
    pub(crate) connected: bool,
    /// Geprüfte Zyklen.
    pub(crate) cycles: u64,
    /// Zeitpunkt des letzten geprüften Zyklus in Millisekunden.
    pub(crate) last_ms: u64,
    /// Zustandsübernahmen und -abzüge im gelesenen Journal.
    pub(crate) resyncs: u64,
    pub(crate) snapshots: u64,
    /// Gelesene Bytes.
    pub(crate) bytes: u64,
    /// Neue Verbindungen nach einem Abbruch.
    pub(crate) reconnects: u32,
    /// Zuletzt quittierter Zyklus des Kerns.
    pub(crate) acked: Option<u64>,
    /// Letzte Störung (Verbindung, Kette, Abweichung).
    pub(crate) problem: Option<String>,
}

/// Abschnitt des Bereitschaftsrechners.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub(crate) enum Phase {
    /// Liest mit.
    Following,
    /// Hauptrechner schweigt: versucht, die Führung am Feldrechner zu erhalten.
    Acquiring {
        /// Seit wann (ms seit dem Start des Dienstes).
        since_ms: u64,
    },
    /// Hat übernommen und führt.
    Leading {
        /// Mit dem mitgelesenen Zustand (warm) oder wie nach einem Neustart (kalt).
        warm: bool,
        /// Zeitpunkt (ms seit dem Start des Dienstes).
        at_ms: u64,
        /// Begründung.
        detail: String,
    },
}

/// Zusage des Hauptrechners.
#[derive(Copy, Clone, Debug)]
struct Promise {
    /// Absendezeit der Quittung, auf die sie antwortet.
    sent: Instant,
    /// Bis dahin gedeckt (`None`: nicht gedeckt).
    until: Option<Instant>,
}

/// Stand des Bereitschaftsrechners.
#[derive(Debug)]
pub(crate) struct Standby {
    pub(crate) primary: String,
    areas: Mutex<Vec<AreaStatus>>,
    activity: Mutex<Vec<Option<Instant>>>,
    promise: Mutex<Option<Promise>>,
    phase: Mutex<Phase>,
    stop: AtomicBool,
    started: Instant,
    shared: Arc<Shared>,
}

impl Standby {
    /// Stand aller Bereiche.
    pub(crate) fn areas(&self) -> Vec<AreaStatus> {
        self.areas.lock().map(|a| a.clone()).unwrap_or_default()
    }

    fn update(&self, index: usize, f: impl FnOnce(&mut AreaStatus)) {
        if let Ok(mut areas) = self.areas.lock()
            && let Some(a) = areas.get_mut(index)
        {
            f(a);
        }
    }

    /// Abschnitt.
    pub(crate) fn phase(&self) -> Phase {
        self.phase.lock().map_or(Phase::Following, |p| p.clone())
    }

    pub(crate) fn set_phase(&self, phase: Phase) {
        if let Ok(mut p) = self.phase.lock() {
            *p = phase;
        }
    }

    /// Führt dieser Rechner nach einer Übernahme?
    pub(crate) fn leading(&self) -> bool {
        matches!(self.phase(), Phase::Leading { .. })
    }

    /// Millisekunden seit dem Start des Dienstes.
    pub(crate) fn ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Beendet das Mitlesen.
    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn touch(&self, index: usize) {
        if let Ok(mut a) = self.activity.lock()
            && let Some(slot) = a.get_mut(index)
        {
            *slot = Some(Instant::now());
        }
    }

    /// Letzter Empfang vom Hauptrechner (über alle Bereiche).
    pub(crate) fn last_activity(&self) -> Option<Instant> {
        self.activity
            .lock()
            .ok()
            .and_then(|a| a.iter().flatten().max().copied())
    }

    /// Nimmt eine Zusage an; ältere Antworten überschreiben keine neueren.
    fn promise(&self, sent: Instant, until: Option<Instant>) {
        if let Ok(mut p) = self.promise.lock()
            && p.is_none_or(|old| old.sent <= sent)
        {
            *p = Some(Promise { sent, until });
        }
    }

    /// Bis wann die Zusage des Hauptrechners eine Übernahme mit dem mitgelesenen Zustand
    /// deckt.
    pub(crate) fn covered_until(&self) -> Option<Instant> {
        self.promise
            .lock()
            .ok()
            .and_then(|p| p.and_then(|p| p.until))
    }

    /// Mitgelesene Zustände aller Bereiche (vollständig oder keine).
    pub(crate) fn states(&self) -> Option<Vec<OwnedState>> {
        self.shared
            .states
            .lock()
            .ok()?
            .iter()
            .map(Clone::clone)
            .collect()
    }

    /// Zustand für `/v1/health`.
    pub(crate) fn health(&self) -> Value {
        let now = Instant::now();
        json!({
            "primary": self.primary,
            "areas": self.areas(),
            "phase": self.phase(),
            "covered_ms": self
                .covered_until()
                .map(|u| u64::try_from(u.saturating_duration_since(now).as_millis()).unwrap_or(0)),
        })
    }
}

/// Zustände aller Bereiche; aus ihnen entsteht das Zustandsbild.
#[derive(Debug)]
struct Shared {
    network: String,
    areas: Vec<(String, PlanView<'static>)>,
    states: Mutex<Vec<Option<OwnedState>>>,
    snapshot: watch::Sender<Arc<Snapshot>>,
}

impl Shared {
    fn publish(&self, index: usize, state: &OwnedState, last_ms: u64) {
        let Ok(mut states) = self.states.lock() else {
            return;
        };
        if let Some(slot) = states.get_mut(index) {
            *slot = Some(state.clone());
        }
        let parts: Vec<(&str, &PlanView<'static>, &OwnedState)> = self
            .areas
            .iter()
            .zip(states.iter())
            .filter_map(|((name, plan), s)| s.as_ref().map(|s| (name.as_str(), plan, s)))
            .collect();
        let previous = self.snapshot.borrow().time_ms;
        let now = Tick::from_millis(last_ms.max(previous));
        self.snapshot
            .send_replace(Arc::new(Snapshot::network(&self.network, now, &parts)));
    }
}

/// Text als Wert eines Abfrageparameters oder Pfadteil.
pub(crate) fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Startet je Stellbereich einen Faden, der das Journal des Hauptrechners mitliest.
pub(crate) fn start(
    primary: &str,
    token: Option<&str>,
    network: String,
    areas: Vec<(String, PlanView<'static>)>,
    snapshot: watch::Sender<Arc<Snapshot>>,
) -> Arc<Standby> {
    let primary = primary.trim_end_matches('/').to_string();
    let count = areas.len();
    let shared = Arc::new(Shared {
        network,
        states: Mutex::new(vec![None; count]),
        areas,
        snapshot,
    });
    let standby = Arc::new(Standby {
        primary: primary.clone(),
        areas: Mutex::new(
            shared
                .areas
                .iter()
                .map(|(area, _)| AreaStatus {
                    area: area.clone(),
                    ..AreaStatus::default()
                })
                .collect(),
        ),
        activity: Mutex::new(vec![None; count]),
        promise: Mutex::new(None),
        phase: Mutex::new(Phase::Following),
        stop: AtomicBool::new(false),
        started: Instant::now(),
        shared,
    });
    for index in 0..count {
        let (standby, primary, token) = (
            Arc::clone(&standby),
            primary.clone(),
            token.map(str::to_string),
        );
        let spawned = std::thread::Builder::new()
            .name(format!("bereitschaft-{index}"))
            .spawn(move || follow(index, &primary, token.as_deref(), &standby));
        if let Err(e) = spawned {
            tracing::error!(%e, "Bereitschaftsrechner: Faden startet nicht");
        }
    }
    standby
}

/// Verbindung zum Hauptrechner.
struct Primary<'a> {
    agent: ureq::Agent,
    url: &'a str,
    token: Option<&'a str>,
}

impl Primary<'_> {
    /// Quittiert den geprüften Stand und liefert die Zusage (`None`: nicht gedeckt).
    fn acknowledge(&self, area: &str, cycle: u64, head: &str) -> Option<Instant> {
        let sent = Instant::now();
        let mut request = self.agent.post(format!("{}/v1/journal/ack", self.url));
        if let Some(t) = self.token {
            request = request.header("Authorization", format!("Bearer {t}"));
        }
        let mut response = request
            .send_json(json!({ "area": area, "cycle": cycle, "head": head }))
            .ok()?;
        if response.status() != 200 {
            return None;
        }
        let v: Value = response.body_mut().read_json().ok()?;
        if v.get("covered").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        let hold = Duration::from_millis(v.get("hold_ms").and_then(Value::as_u64)?);
        // Zehn Prozent und ein fester Abstand für Gangunterschied und Laufzeit.
        let hold = hold.saturating_sub(hold / 10).checked_sub(PROMISE_MARGIN)?;
        Some(sent + hold)
    }
}

/// Liest das Journal eines Bereichs mit, bis das Mitlesen beendet wird.
fn follow(index: usize, url: &str, token: Option<&str>, standby: &Standby) {
    let Some((area, plan)) = standby.shared.areas.get(index).cloned() else {
        return;
    };
    let primary = Primary {
        agent: ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(2)))
            .build()
            .into(),
        url,
        token,
    };
    let mut follower = Follower::new(&plan, None);
    let mut offset = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    while !standby.stopped() {
        let stream = format!(
            "{url}/v1/journal/stream?area={}&offset={offset}",
            encode(&area)
        );
        let mut request = primary.agent.get(&stream);
        if let Some(t) = token {
            request = request.header("Authorization", format!("Bearer {t}"));
        }
        match request.call() {
            Ok(response) if response.status() == 200 => {
                standby.update(index, |s| {
                    s.connected = true;
                    s.problem = None;
                });
                let mut reader = response.into_body().into_reader();
                while !standby.stopped() {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    standby.touch(index);
                    offset = offset.saturating_add(u64::try_from(n).unwrap_or(0));
                    if let Err(e) = follower.feed(buf.get(..n).unwrap_or_default()) {
                        // Passt nicht (mehr): von vorn lesen.
                        tracing::warn!(area, %e, "Bereitschaftsrechner: Journal passt nicht, lese neu");
                        standby.update(index, |s| s.problem = Some(e.to_string()));
                        follower = Follower::new(&plan, None);
                        offset = 0;
                        break;
                    }
                    if standby.stopped() {
                        break;
                    }
                    if let (Some(state), Some(summary)) = (follower.state(), follower.summary()) {
                        standby.shared.publish(index, state, summary.last_ms);
                        let cycle = state.meta().cycle();
                        let sent = Instant::now();
                        let until = primary.acknowledge(&area, cycle, &summary.head);
                        standby.promise(sent, until);
                        standby.update(index, |s| {
                            s.cycles = summary.cycles;
                            s.last_ms = summary.last_ms;
                            s.resyncs = summary.resyncs;
                            s.snapshots = summary.snapshots;
                            s.bytes = offset;
                            if until.is_some() {
                                s.acked = Some(cycle);
                            }
                        });
                    }
                }
            }
            Ok(response) => {
                let status = response.status();
                standby.update(index, |s| {
                    s.problem = Some(format!("Hauptrechner: HTTP {status}"));
                });
            }
            Err(e) => standby.update(index, |s| s.problem = Some(format!("Hauptrechner: {e}"))),
        }
        standby.update(index, |s| {
            s.connected = false;
            s.reconnects = s.reconnects.saturating_add(1);
        });
        if !standby.stopped() {
            std::thread::sleep(RETRY);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn area_names_are_encoded() {
        assert_eq!(super::encode("Bstadt Süd"), "Bstadt%20S%C3%BCd");
        assert_eq!(super::encode("Adorf-1"), "Adorf-1");
    }
}
