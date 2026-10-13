//! Rückschau aus den eigenen Journalen (REQ-SVC-010): Zustand zu einem Zeitpunkt und
//! Stellbefehle und Ereignisse eines Zeitfensters, rekonstruiert ab dem letzten
//! Zustandsabzug davor und gegen das Journal geprüft (REQ-JRN-008).
//!
//! Nur Anzeige (SIL 0): Die Rückschau liest die Journaldateien, sie hat keine Verbindung zum
//! laufenden Betrieb.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use railit_plan::PlanView;
use railit_runtime::{NamedCommand, NamedEvent, Snapshot};
use railit_types::Tick;
use serde_json::{Value, json};

/// Höchstzahl der Zyklen in einer Antwort von [`events`].
pub(crate) const MAX_EVENT_ROWS: usize = 5_000;

/// Journaldatei eines Stellbereichs.
#[derive(Clone, Debug)]
pub(crate) struct JournalFile {
    /// Name des Stellbereichs.
    pub(crate) area: String,
    pub(crate) path: PathBuf,
    pub(crate) plan: PlanView<'static>,
}

/// Journaldateien je Stellbereich: eine Datei bei einem Bereich, sonst ein Verzeichnis mit
/// `<Bereich>.journal` je Bereich.
pub(crate) fn files(path: &Path, areas: &[(String, PlanView<'static>)]) -> Vec<JournalFile> {
    if let [(area, plan)] = areas {
        return vec![JournalFile {
            area: area.clone(),
            path: path.to_path_buf(),
            plan: *plan,
        }];
    }
    areas
        .iter()
        .map(|(area, plan)| JournalFile {
            area: area.clone(),
            path: path.join(format!("{area}.journal")),
            plan: *plan,
        })
        .collect()
}

/// Journal eines Bereichs (ohne Angabe: der einzige).
pub(crate) fn pick<'a>(
    files: &'a [JournalFile],
    area: Option<&str>,
) -> Result<&'a JournalFile, String> {
    match (area, files) {
        (None, [only]) => Ok(only),
        (None, []) => Err("kein Journal (railitd --journal)".into()),
        (None, _) => Err("mehrere Stellbereiche: area angeben".into()),
        (Some(name), _) => files
            .iter()
            .find(|f| f.area == name)
            .ok_or_else(|| format!("kein Journal für Stellbereich „{name}“")),
    }
}

fn open(file: &JournalFile) -> Result<BufReader<std::fs::File>, String> {
    std::fs::File::open(&file.path)
        .map(BufReader::new)
        .map_err(|e| format!("{}: {e}", file.path.display()))
}

/// Übersicht der Journale.
pub(crate) fn overview(files: &[JournalFile]) -> Value {
    json!({
        "journals": files.iter().map(|f| json!({
            "area": f.area,
            "file": f.path.display().to_string(),
            "bytes": std::fs::metadata(&f.path).map_or(0, |m| m.len()),
        })).collect::<Vec<_>>(),
    })
}

/// Zustandsbild zum Zeitpunkt `at_ms`.
pub(crate) fn state_at(file: &JournalFile, at_ms: u64) -> Result<Value, String> {
    let state = railit_journal::replay_window(open(file)?, &file.plan, at_ms, at_ms, |_| {})
        .map_err(|e| e.to_string())?;
    let snap = Snapshot::network(
        file.plan.network_name(),
        Tick::from_millis(at_ms),
        &[(file.area.as_str(), &file.plan, &state)],
    );
    Ok(json!({ "area": file.area, "at_ms": at_ms, "state": snap }))
}

/// Stellbefehle und Ereignisse zwischen `from_ms` und `to_ms` (höchstens
/// [`MAX_EVENT_ROWS`] Zyklen mit Befehlen oder Ereignissen; `truncated` zeigt den Rest an).
pub(crate) fn events(file: &JournalFile, from_ms: u64, to_ms: u64) -> Result<Value, String> {
    let mut rows = Vec::new();
    let mut truncated = false;
    let plan = file.plan;
    railit_journal::replay_window(open(file)?, &plan, from_ms, to_ms, |c| {
        if c.commands.is_empty() && c.events.is_empty() {
            return;
        }
        if rows.len() >= MAX_EVENT_ROWS {
            truncated = true;
            return;
        }
        rows.push(json!({
            "cycle": c.cycle,
            "now_ms": c.now_ms,
            "commands": c.commands.iter().map(|x| NamedCommand::new(&plan, x)).collect::<Vec<_>>(),
            "events": c.events.iter().map(|e| NamedEvent::new(&plan, "", e)).collect::<Vec<_>>(),
        }));
    })
    .map_err(|e| e.to_string())?;
    Ok(json!({
        "area": file.area,
        "from_ms": from_ms,
        "to_ms": to_ms,
        "cycles": rows,
        "truncated": truncated,
    }))
}
