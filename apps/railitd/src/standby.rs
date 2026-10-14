//! Bereitschaftsrechner (REQ-SVC-011): `railitd --standby URL` liest die Journale eines
//! Hauptrechners über `/v1/journal/stream` mit, prüft und rechnet jede Zeile nach
//! ([`railit_journal::Follower`]) und zeigt den so gewonnenen Zustand über dieselbe API und
//! denselben Bedienplatz an – ohne Feld, ohne Kern im Betrieb, ohne Bedienung.
//!
//! Bricht die Verbindung ab, setzt der Bereitschaftsrechner an der letzten Position fort.
//! Passt das Journal dann nicht mehr (der Hauptrechner hat neu begonnen), liest er von
//! vorn. Die Übernahme des Betriebs durch den Bereitschaftsrechner ist nicht Teil dieser
//! Stufe.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use railit_journal::Follower;
use railit_kernel::OwnedState;
use railit_plan::PlanView;
use railit_runtime::Snapshot;
use railit_types::Tick;
use serde::Serialize;
use tokio::sync::watch;

/// Wartezeit vor einem neuen Verbindungsversuch.
const RETRY: Duration = Duration::from_secs(1);

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
    /// Letzte Störung (Verbindung, Kette, Abweichung).
    pub(crate) problem: Option<String>,
}

/// Stand des Bereitschaftsrechners.
#[derive(Debug)]
pub(crate) struct Standby {
    pub(crate) primary: String,
    areas: Mutex<Vec<AreaStatus>>,
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
}

/// Zustände aller Bereiche; aus ihnen entsteht das Zustandsbild.
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
    let standby = Arc::new(Standby {
        primary: primary.clone(),
        areas: Mutex::new(
            areas
                .iter()
                .map(|(area, _)| AreaStatus {
                    area: area.clone(),
                    ..AreaStatus::default()
                })
                .collect(),
        ),
    });
    let shared = Arc::new(Shared {
        network,
        states: Mutex::new(vec![None; areas.len()]),
        areas,
        snapshot,
    });
    for index in 0..shared.areas.len() {
        let (standby, shared, primary, token) = (
            Arc::clone(&standby),
            Arc::clone(&shared),
            primary.clone(),
            token.map(str::to_string),
        );
        let spawned = std::thread::Builder::new()
            .name(format!("bereitschaft-{index}"))
            .spawn(move || follow(index, &primary, token.as_deref(), &standby, &shared));
        if let Err(e) = spawned {
            tracing::error!(%e, "Bereitschaftsrechner: Faden startet nicht");
        }
    }
    standby
}

/// Liest das Journal eines Bereichs mit, solange der Dienst läuft.
fn follow(index: usize, primary: &str, token: Option<&str>, standby: &Standby, shared: &Shared) {
    let Some((area, plan)) = shared.areas.get(index).cloned() else {
        return;
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut follower = Follower::new(&plan, None);
    let mut offset = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let url = format!(
            "{primary}/v1/journal/stream?area={}&offset={offset}",
            encode(&area)
        );
        let mut request = agent.get(&url);
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
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    offset = offset.saturating_add(u64::try_from(n).unwrap_or(0));
                    if let Err(e) = follower.feed(buf.get(..n).unwrap_or_default()) {
                        // Passt nicht (mehr): von vorn lesen.
                        tracing::warn!(area, %e, "Bereitschaftsrechner: Journal passt nicht, lese neu");
                        standby.update(index, |s| s.problem = Some(e.to_string()));
                        follower = Follower::new(&plan, None);
                        offset = 0;
                        break;
                    }
                    if let (Some(state), Some(summary)) = (follower.state(), follower.summary()) {
                        shared.publish(index, state, summary.last_ms);
                        standby.update(index, |s| {
                            s.cycles = summary.cycles;
                            s.last_ms = summary.last_ms;
                            s.resyncs = summary.resyncs;
                            s.snapshots = summary.snapshots;
                            s.bytes = offset;
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
        std::thread::sleep(RETRY);
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
