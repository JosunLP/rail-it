//! Betriebsjournal prüfen und abspielen.

use std::io::BufReader;
use std::path::Path;
use std::process::ExitCode;

use railit_journal::{JournalError, Summary};
use railit_kernel::{Aspect, Occupancy, RoutePhase};
use railit_runtime::{NamedCommand, NamedEvent, Snapshot};
use railit_types::{PointPosition, Tick};
use serde_json::json;

use crate::{EXIT_INVALID, Output, fail_usage, print_json};

fn open(path: &Path) -> Result<BufReader<std::fs::File>, ExitCode> {
    std::fs::File::open(path)
        .map(BufReader::new)
        .map_err(|e| fail_usage(&format!("{}: {e}", path.display())))
}

/// Ohne Siegel ungültig, wenn eines verlangt ist.
fn require(
    result: Result<Summary, JournalError>,
    require_seal: bool,
) -> Result<Summary, JournalError> {
    match result {
        Ok(s) if require_seal && s.seals == 0 => Err(JournalError::Seal {
            line: 0,
            message: "Journal ist nicht gesiegelt".into(),
        }),
        other => other,
    }
}

fn report(result: Result<Summary, JournalError>, what: &str, output: Output) -> ExitCode {
    match (result, output) {
        (Ok(s), Output::Json) => {
            print_json(&json!({
                "ok": true,
                "network": s.header.network,
                "plan_digest": s.header.plan_digest,
                "cycles": s.cycles,
                "checkpoints": s.checkpoints,
                "snapshots": s.snapshots,
                "resyncs": s.resyncs,
                "last_ms": s.last_ms,
                "head": s.head,
                "truncated": s.truncated,
                "seals": s.seals,
                "seal_key": s.seal_key,
                "unsealed": s.unsealed,
            }));
            ExitCode::SUCCESS
        }
        (Ok(s), Output::Text) => {
            #[allow(clippy::cast_precision_loss)]
            let t = s.last_ms as f64 / 1000.0;
            println!(
                "✔ {what}: Netz „{}“, {} Zyklen bis t = {t:.1} s, {} Prüfpunkte, {} Zustandsabzüge",
                s.header.network, s.cycles, s.checkpoints, s.snapshots
            );
            if s.resyncs > 0 {
                println!(
                    "  {} Zustandsübernahmen (Abgleich mit dem Knoten des sicheren Rechners)",
                    s.resyncs
                );
            }
            println!("  Kopf der Kette: {}", s.head);
            match &s.seal_key {
                Some(key) => println!(
                    "  {} Siegel mit Schlüssel {}…, {} Einträge nach dem letzten Siegel",
                    s.seals,
                    key.get(..16).unwrap_or(key),
                    s.unsealed
                ),
                None => println!("  nicht gesiegelt"),
            }
            if s.truncated {
                println!(
                    "  Hinweis: letzte Zeile unvollständig (Abbruch beim Schreiben), ignoriert"
                );
            }
            ExitCode::SUCCESS
        }
        (Err(e), Output::Json) => {
            print_json(&json!({ "ok": false, "error": e.to_string() }));
            ExitCode::from(EXIT_INVALID)
        }
        (Err(e), Output::Text) => {
            println!("✘ {what}: {e}");
            ExitCode::from(EXIT_INVALID)
        }
    }
}

/// `railit journal verify`
pub(crate) fn verify(
    path: &Path,
    key: Option<&Path>,
    require_seal: bool,
    output: Output,
) -> ExitCode {
    let key = match key
        .map(railit_runtime::keys::read_verifying_key)
        .transpose()
    {
        Ok(k) => k,
        Err(e) => return fail_usage(&e),
    };
    match open(path) {
        Ok(r) => {
            let result = match &key {
                Some(k) => railit_journal::verify_sealed(r, k),
                None => railit_journal::verify(r),
            };
            let what = if key.is_some() {
                "Hashkette und Siegel intakt"
            } else {
                "Hashkette intakt"
            };
            report(require(result, require_seal), what, output)
        }
        Err(code) => code,
    }
}

fn load_plan(plan_path: &Path) -> Result<railit_plan::PlanView<'static>, ExitCode> {
    let bytes = std::fs::read(plan_path)
        .map_err(|e| fail_usage(&format!("{}: {e}", plan_path.display())))?;
    railit_runtime::load_plan(bytes, None).map_err(|e| {
        eprintln!("Fehler: Plan ungültig: {e}");
        ExitCode::from(EXIT_INVALID)
    })
}

/// Sekunden als Millisekunden (negativ: 0).
fn ms(seconds: f64) -> u64 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ms = (seconds.max(0.0) * 1000.0).round() as u64;
    ms
}

fn secs(ms: u64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let s = ms as f64 / 1000.0;
    s
}

fn fail_journal(e: &JournalError) -> ExitCode {
    eprintln!("✘ {e}");
    ExitCode::from(EXIT_INVALID)
}

/// `railit journal state`
pub(crate) fn state(path: &Path, plan_path: &Path, at: f64, output: Output) -> ExitCode {
    let plan = match load_plan(plan_path) {
        Ok(p) => p,
        Err(code) => return code,
    };
    let at_ms = ms(at);
    let mut last_ms = None;
    let result = match open(path) {
        Ok(r) => railit_journal::replay_window(r, &plan, at_ms, at_ms, |c| {
            last_ms = Some(c.now_ms);
        }),
        Err(code) => return code,
    };
    let state = match result {
        Ok(s) => s,
        Err(e) => return fail_journal(&e),
    };
    let snap = Snapshot::new(&plan, &state, Tick::from_millis(at_ms));
    match output {
        Output::Json => print_json(&json!({ "at_ms": at_ms, "state": snap })),
        Output::Text => print_state(&snap, at),
    }
    ExitCode::SUCCESS
}

fn print_state(snap: &Snapshot, at: f64) {
    println!(
        "Zustand bei t = {at:.1} s (Zyklus {}) – Netz „{}“",
        snap.cycle, snap.network
    );
    if let Some(reason) = &snap.safe_state {
        println!("  SICHERER ZUSTAND: {reason}");
    }
    let routes: Vec<String> = snap
        .routes
        .iter()
        .filter(|r| r.phase != RoutePhase::Idle)
        .map(|r| format!("{} ({})", r.name, phase_text(r.phase)))
        .collect();
    let proceed: Vec<&str> = snap
        .signals
        .iter()
        .filter(|s| s.commanded == Aspect::Proceed || s.substitute)
        .map(|s| s.name.as_str())
        .collect();
    let occupied: Vec<&str> = snap
        .tvds
        .iter()
        .filter(|t| !matches!(t.occupancy, Some(Occupancy::Clear)))
        .map(|t| t.name.as_str())
        .collect();
    let locked: Vec<String> = snap
        .points
        .iter()
        .filter(|p| p.locked_position.is_some())
        .map(|p| {
            let side = match p.locked_position {
                Some(PointPosition::Left) => "links",
                _ => "rechts",
            };
            format!("{} {side}", p.name)
        })
        .collect();
    let list = |v: &[String]| {
        if v.is_empty() {
            "–".to_string()
        } else {
            v.join(", ")
        }
    };
    println!("  Fahrstraßen:            {}", list(&routes));
    println!(
        "  Signale auf Fahrt:      {}",
        list(&proceed.iter().map(ToString::to_string).collect::<Vec<_>>())
    );
    println!(
        "  Abschnitte nicht frei:  {}",
        list(&occupied.iter().map(ToString::to_string).collect::<Vec<_>>())
    );
    println!("  Weichen verschlossen:   {}", list(&locked));
    println!(
        "  Hilfshandlungen:        {} ausgeführt, {} offen",
        snap.aux_counter,
        snap.pending_aux.len()
    );
}

fn phase_text(phase: RoutePhase) -> &'static str {
    match phase {
        RoutePhase::Idle => "aufgelöst",
        RoutePhase::Setting => "wird eingestellt",
        RoutePhase::Locked => "festgelegt",
        RoutePhase::Proceed => "Fahrt",
        RoutePhase::Occupied => "befahren",
        RoutePhase::Arrived => "angekommen",
        RoutePhase::Cancelling => "wird zurückgenommen",
        RoutePhase::Halted => "haltgefallen",
    }
}

fn event_text(e: &NamedEvent) -> String {
    let at = |element: &Option<String>| {
        element
            .as_ref()
            .map(|e| format!(" ({e})"))
            .unwrap_or_default()
    };
    match e {
        NamedEvent::Accepted { request } => format!("Anforderung {request} angenommen"),
        NamedEvent::Rejected {
            request,
            reason,
            element,
        } => format!("Anforderung {request} abgelehnt: {reason}{}", at(element)),
        NamedEvent::RoutePhase { route, phase } => {
            format!("Fahrstraße {route}: {}", phase_text(*phase))
        }
        NamedEvent::Alarm { alarm, element } => format!("Meldung {alarm}{}", at(element)),
        NamedEvent::AuxPending { aux, op, operator } => format!(
            "Hilfshandlung {aux} angefordert von Bediener {operator}: {}",
            serde_json::to_string(op).unwrap_or_default()
        ),
        NamedEvent::AuxExecuted { aux, counter, .. } => {
            format!("Hilfshandlung {aux} ausgeführt, Zählwerk {counter}")
        }
        NamedEvent::AuxExpired { aux } => format!("Hilfshandlung {aux} verfallen"),
        NamedEvent::Permission { border, holding } => format!(
            "Erlaubnis an {border} {}",
            if *holding { "übernommen" } else { "abgegeben" }
        ),
    }
}

/// `railit journal events`
pub(crate) fn events(
    path: &Path,
    plan_path: &Path,
    from: f64,
    to: Option<f64>,
    output: Output,
) -> ExitCode {
    let plan = match load_plan(plan_path) {
        Ok(p) => p,
        Err(code) => return code,
    };
    let (from_ms, to_ms) = (ms(from), to.map_or(u64::MAX, ms));
    let mut rows = Vec::new();
    let result = match open(path) {
        Ok(r) => railit_journal::replay_window(r, &plan, from_ms, to_ms, |c| {
            if c.commands.is_empty() && c.events.is_empty() {
                return;
            }
            let commands: Vec<NamedCommand> = c
                .commands
                .iter()
                .map(|cmd| NamedCommand::new(&plan, cmd))
                .collect();
            let events: Vec<NamedEvent> = c
                .events
                .iter()
                .map(|e| NamedEvent::new(&plan, "", e))
                .collect();
            rows.push((c.cycle, c.now_ms, commands, events));
        }),
        Err(code) => return code,
    };
    if let Err(e) = result {
        return fail_journal(&e);
    }
    match output {
        Output::Json => print_json(&json!(
            rows.iter()
                .map(|(cycle, now_ms, commands, events)| json!({
                    "cycle": cycle,
                    "now_ms": now_ms,
                    "commands": commands,
                    "events": events,
                }))
                .collect::<Vec<_>>()
        )),
        Output::Text => {
            for (cycle, now_ms, commands, events) in &rows {
                println!("t = {:>8.1} s  Zyklus {cycle}", secs(*now_ms));
                for c in commands {
                    println!("    → {c}");
                }
                for e in events {
                    println!("    ● {}", event_text(e));
                }
            }
            println!(
                "{} Zyklen mit Befehlen oder Ereignissen, alle gegen das Journal geprüft",
                rows.len()
            );
        }
    }
    ExitCode::SUCCESS
}

/// `railit journal replay`
pub(crate) fn replay(path: &Path, plan_path: &Path, output: Output) -> ExitCode {
    let plan = match load_plan(plan_path) {
        Ok(p) => p,
        Err(code) => return code,
    };
    match open(path) {
        Ok(r) => report(
            railit_journal::replay(r, &plan).map(|(s, _)| s),
            "Abspielen ohne Abweichung",
            output,
        ),
        Err(code) => code,
    }
}
