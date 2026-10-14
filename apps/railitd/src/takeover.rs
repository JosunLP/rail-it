//! Übernahme des Betriebs durch den Bereitschaftsrechner (REQ-SVC-012/013/014).
//!
//! `railitd --standby URL --field remote --takeover-after-ms MS`: Schweigt der Strom des
//! Hauptrechners länger als `MS`, versucht der Bereitschaftsrechner, am Feldrechner zu führen.
//! Der Umschalter dort lässt ihn erst zu, wenn alle Verbindungen des Hauptrechners abgebaut
//! sind; meldet sich der Hauptrechner vorher wieder, bricht der Versuch ab.
//!
//! Führt er, entscheidet die Zusage des Hauptrechners:
//!
//! - **warm** – die letzte Zusage galt noch: Der Hauptrechner hat bis zu seinem Ausfall nichts
//!   freigegeben, was der Bereitschaftsrechner nicht kennt. Er übernimmt den mitgelesenen
//!   Zustand, restriktiv angeglichen (REQ-IXL-025): Verschlüsse bleiben, Fahrt fällt auf Halt.
//! - **kalt** – sonst: Er beginnt wie nach einem Neustart (alle Elemente unbekannt, keine
//!   Fahrstraßen).
//!
//! Danach rechnet die Engine mit diesem Zustand, das Journal hält die Übernahme fest, und der
//! Dienst nimmt Bedienungen an.

use std::sync::Arc;
use std::time::{Duration, Instant};

use railit_runtime::{FieldAdapter, Runtime};
use railit_types::Tick;
use tokio::sync::oneshot;

use crate::field::ServiceField;
use crate::standby::{Phase, Standby};

/// Takt der Versuche, am Feldrechner zu führen.
const POLL: Duration = Duration::from_millis(20);

/// Vorbereitung nach der Entscheidung (Journale beginnen, Ausgabesperre); vor der Übernahme
/// des Zustands, damit das Journal sie festhält.
pub(crate) type Prepare = Box<dyn FnOnce(&mut Runtime<ServiceField>) -> Result<(), String> + Send>;

/// Startet die Überwachung des Hauptrechners in einem eigenen Faden.
pub(crate) fn start(
    standby: Arc<Standby>,
    rt: Runtime<ServiceField>,
    after: Duration,
    prepare: Prepare,
    wake: oneshot::Sender<Runtime<ServiceField>>,
) {
    let spawned = std::thread::Builder::new()
        .name("uebernahme".into())
        .spawn(move || watch(&standby, rt, after, prepare, wake));
    if let Err(e) = spawned {
        tracing::error!(%e, "Übernahme: Faden startet nicht");
    }
}

/// Schweigt der Hauptrechner?
fn silent(standby: &Standby, after: Duration) -> bool {
    standby.last_activity().is_some_and(|t| t.elapsed() > after)
}

fn watch(
    standby: &Standby,
    mut rt: Runtime<ServiceField>,
    after: Duration,
    prepare: Prepare,
    wake: oneshot::Sender<Runtime<ServiceField>>,
) {
    loop {
        std::thread::sleep(POLL);
        if !silent(standby, after) {
            continue;
        }
        tracing::warn!("Hauptrechner schweigt: versuche, am Feldrechner zu führen");
        standby.set_phase(Phase::Acquiring {
            since_ms: standby.ms(),
        });
        if acquire(standby, &mut rt, after) {
            break;
        }
        tracing::info!("Hauptrechner meldet sich wieder: lese weiter mit");
        standby.set_phase(Phase::Following);
    }
    // Führung erhalten: Ab jetzt erreicht der Hauptrechner das Feld nicht mehr.
    let led = Instant::now();
    standby.stop();
    let covered = standby.covered_until().is_some_and(|until| led < until);
    let states = standby.states().filter(|_| covered);
    let detail = if states.is_some() {
        "Übernahme mit dem mitgelesenen Zustand (gedeckt durch die Zusage des Hauptrechners)"
            .to_string()
    } else if covered {
        "Neustart: nicht alle Stellbereiche mitgelesen".to_string()
    } else {
        "Neustart: keine gültige Zusage des Hauptrechners".to_string()
    };
    if let Err(e) = prepare(&mut rt) {
        tracing::error!(e, "Übernahme: Vorbereitung gescheitert");
    }
    let warm = match &states {
        Some(states) => {
            // Zeitbasis: letzter Zyklus des Hauptrechners plus die Zeit seither.
            let last_ms = standby.areas().iter().map(|a| a.last_ms).max().unwrap_or(0);
            let silence = standby
                .last_activity()
                .map_or(Duration::ZERO, |t| t.elapsed());
            let now = Tick::from_millis(
                last_ms.saturating_add(u64::try_from(silence.as_millis()).unwrap_or(0)),
            );
            match rt.take_over(states, now, &detail) {
                Ok(out) => {
                    if let Some((area, reason)) = out.safe_state {
                        tracing::error!(area, ?reason, "Übernahme: sicherer Zustand");
                    }
                    true
                }
                Err(e) => {
                    tracing::error!(e, "Übernahme gescheitert: Neustart");
                    false
                }
            }
        }
        None => false,
    };
    tracing::warn!(warm, detail, "Bereitschaftsrechner führt");
    standby.set_phase(Phase::Leading {
        warm,
        at_ms: standby.ms(),
        detail,
    });
    let _ = wake.send(rt);
}

/// Versucht, am Feldrechner zu führen: Verbindungen zu allen Objektsteuerungen. `false`, wenn
/// der Hauptrechner sich vorher wieder meldet.
fn acquire(standby: &Standby, rt: &mut Runtime<ServiceField>, after: Duration) -> bool {
    loop {
        rt.field_mut().advance(Tick::ZERO);
        if rt.field().connected() {
            return true;
        }
        if !silent(standby, after) {
            return false;
        }
        std::thread::sleep(POLL);
    }
}
