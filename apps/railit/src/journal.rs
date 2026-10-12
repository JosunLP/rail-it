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

fn report(result: Result<Summary, JournalError>, what: &str, output: Output) -> ExitCode {
    match (result, output) {
        (Ok(s), Output::Json) => {
            print_json(&json!({
                "ok": true,
                "network": s.header.network,
                "plan_digest": s.header.plan_digest,
                "cycles": s.cycles,
                "checkpoints": s.checkpoints,
                "last_ms": s.last_ms,
                "head": s.head,
                "truncated": s.truncated,
            }));
            ExitCode::SUCCESS
        }
        (Ok(s), Output::Text) => {
            #[allow(clippy::cast_precision_loss)]
            let t = s.last_ms as f64 / 1000.0;
            println!(
                "✔ {what}: Netz „{}“, {} Zyklen bis t = {t:.1} s, {} Prüfpunkte",
                s.header.network, s.cycles, s.checkpoints
            );
            println!("  Kopf der Kette: {}", s.head);
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
pub(crate) fn verify(path: &Path, output: Output) -> ExitCode {
    match open(path) {
        Ok(r) => report(railit_journal::verify(r), "Hashkette intakt", output),
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
