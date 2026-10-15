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
                "sealed_ms": s.sealed_ms,
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
            if let Some((first, last)) = s.sealed_ms {
                println!(
                    "  Siegel von {} bis {} (Uhr des Stellwerks)",
                    railit_journal::witness::format_utc(first),
                    railit_journal::witness::format_utc(last)
                );
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

/// Zeugnisse in `<Datei>.witness` prüfen (REQ-JRN-010).
pub(crate) fn check_witnesses(
    path: &Path,
    key: &Path,
) -> Result<Vec<railit_journal::witness::Witnessed>, String> {
    let key = railit_runtime::keys::read_verifying_key(key)?;
    let mut wpath = path.as_os_str().to_owned();
    wpath.push(".witness");
    let wpath = std::path::PathBuf::from(wpath);
    let witnesses = std::fs::File::open(&wpath)
        .map(BufReader::new)
        .map_err(|e| format!("{}: {e}", wpath.display()))?;
    let file = std::fs::File::open(path)
        .map(BufReader::new)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    railit_journal::witness::verify_witnesses(file, witnesses, &key)
}

/// Zeigt geprüfte Zeugnisse an.
pub(crate) fn print_witnesses(list: &[railit_journal::witness::Witnessed]) {
    for w in list {
        println!(
            "  bezeugt: Zeile {} bestand am {}",
            w.line,
            railit_journal::witness::format_utc(w.time_ms)
        );
    }
}

/// `railit journal witness`: bezeugt den letzten Stand einer Datei (REQ-JRN-010).
pub(crate) fn witness(path: &Path, key: &Path, output: Output) -> ExitCode {
    use std::io::Write as _;
    let key = match railit_runtime::keys::read_signing_key(key) {
        Ok(k) => k,
        Err(e) => return fail_usage(&e),
    };
    let head = match open(path).map(railit_journal::witness::last_hash) {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => return fail_usage(&format!("{}: {e}", path.display())),
        Err(code) => return code,
    };
    let now = railit_journal::witness::now_ms();
    let w = railit_journal::witness::witness(&key, &head, now);
    let mut wpath = path.as_os_str().to_owned();
    wpath.push(".witness");
    let wpath = std::path::PathBuf::from(wpath);
    let line = serde_json::to_string(&w).unwrap_or_default();
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&wpath)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = written {
        return fail_usage(&format!("{}: {e}", wpath.display()));
    }
    match output {
        Output::Json => print_json(&json!({"head": w.head, "time_ms": w.time_ms, "file": wpath})),
        Output::Text => println!(
            "Stand {}… bezeugt am {} ({})",
            w.head.get(..16).unwrap_or(&w.head),
            railit_journal::witness::format_utc(w.time_ms),
            wpath.display()
        ),
    }
    ExitCode::SUCCESS
}

/// `railit journal verify`
pub(crate) fn verify(
    path: &Path,
    key: Option<&Path>,
    require_seal: bool,
    witness_key: Option<&Path>,
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
            let ok = result.is_ok();
            let code = report(require(result, require_seal), what, output);
            match (witness_key, ok) {
                (Some(wk), true) => match check_witnesses(path, wk) {
                    Ok(list) => {
                        if output == Output::Text {
                            print_witnesses(&list);
                        }
                        code
                    }
                    Err(e) => {
                        eprintln!("✘ Zeugnisse: {e}");
                        ExitCode::from(EXIT_INVALID)
                    }
                },
                _ => code,
            }
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

/// `railit journal follow`
pub(crate) fn follow(
    path: &Path,
    plan_path: &Path,
    key: Option<&Path>,
    once: bool,
    poll_ms: u64,
    output: Output,
) -> ExitCode {
    use std::io::{Read, Seek, SeekFrom};

    let plan = match load_plan(plan_path) {
        Ok(p) => p,
        Err(code) => return code,
    };
    let key = match key
        .map(railit_runtime::keys::read_verifying_key)
        .transpose()
    {
        Ok(k) => k,
        Err(e) => return fail_usage(&e),
    };
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return fail_usage(&format!("{}: {e}", path.display())),
    };
    let mut follower = railit_journal::Follower::new(&plan, key.as_ref());
    let mut offset = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    let mut reported = None;
    loop {
        // Kürzer geworden: Die Datei wurde ersetzt oder abgeschnitten.
        let len = file.metadata().map_or(0, |m| m.len());
        if len < offset {
            return fail_journal(&JournalError::Io(format!(
                "{} ist kürzer geworden ({len} < {offset} Byte)",
                path.display()
            )));
        }
        if let Err(e) = file.seek(SeekFrom::Start(offset)) {
            return fail_usage(&format!("{}: {e}", path.display()));
        }
        loop {
            let n = match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return fail_usage(&format!("{}: {e}", path.display())),
            };
            offset = offset.saturating_add(u64::try_from(n).unwrap_or(0));
            if let Err(e) = follower.feed(buf.get(..n).unwrap_or_default()) {
                return fail_journal(&e);
            }
        }
        let summary = follower.summary();
        let cycles = summary.as_ref().map(|s| s.cycles);
        if cycles != reported
            && let (Some(s), Some(state)) = (&summary, follower.state())
        {
            reported = cycles;
            let active = state
                .routes()
                .iter()
                .filter(|r| r.phase() != RoutePhase::Idle)
                .count();
            match output {
                Output::Json => println!(
                    "{}",
                    json!({
                        "cycles": s.cycles,
                        "last_ms": s.last_ms,
                        "cycle": state.meta().cycle(),
                        "routes_active": active,
                        "safe_state": state.meta().safe_state().map(|r| format!("{r:?}")),
                        "snapshots": s.snapshots,
                        "resyncs": s.resyncs,
                        "seals": s.seals,
                        "head": s.head,
                    })
                ),
                Output::Text => println!(
                    "✔ t = {:>8.1} s  {} Zyklen geprüft, {active} Fahrstraßen eingestellt{}",
                    secs(s.last_ms),
                    s.cycles,
                    state
                        .meta()
                        .safe_state()
                        .map(|r| format!(", SICHERER ZUSTAND: {r:?}"))
                        .unwrap_or_default()
                ),
            }
        }
        if once {
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(std::time::Duration::from_millis(poll_ms.max(10)));
    }
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
