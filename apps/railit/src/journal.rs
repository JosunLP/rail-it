//! Betriebsjournal prüfen und abspielen.

use std::io::BufReader;
use std::path::Path;
use std::process::ExitCode;

use railit_journal::{JournalError, Summary};
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

/// `railit journal replay`
pub(crate) fn replay(path: &Path, plan_path: &Path, output: Output) -> ExitCode {
    let bytes = match std::fs::read(plan_path) {
        Ok(b) => b,
        Err(e) => return fail_usage(&format!("{}: {e}", plan_path.display())),
    };
    let plan = match railit_runtime::load_plan(bytes, None) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Fehler: Plan ungültig: {e}");
            return ExitCode::from(EXIT_INVALID);
        }
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
