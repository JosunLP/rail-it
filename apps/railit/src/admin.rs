//! Verwaltung: Auditprotokoll prüfen, Zugangstoken erzeugen.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::process::ExitCode;

use railit_journal::audit::verify_audit;
use railit_journal::to_hex;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{EXIT_INVALID, Output, fail_usage, print_json};

/// Prüft die Hashkette eines Auditprotokolls (REQ-SEC-004).
pub(crate) fn audit_verify(
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
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => return fail_usage(&format!("{}: {e}", path.display())),
    };
    let result = match &key {
        Some(k) => railit_journal::audit::verify_audit_sealed(BufReader::new(file), k),
        None => verify_audit(BufReader::new(file)),
    };
    let result = match result {
        Ok(s) if require_seal && s.seals == 0 => Err(railit_journal::JournalError::Seal {
            line: 0,
            message: "Auditprotokoll ist nicht gesiegelt".into(),
        }),
        other => other,
    };
    match result {
        Ok(s) => {
            match output {
                Output::Json => print_json(&json!({
                    "valid": true, "records": s.records, "last_seq": s.last_seq,
                    "head": s.head, "truncated": s.truncated,
                    "seals": s.seals, "seal_key": s.seal_key, "unsealed": s.unsealed,
                })),
                Output::Text => {
                    println!(
                        "Auditprotokoll gültig: {} Einträge, Kopf {}",
                        s.records, s.head
                    );
                    match &s.seal_key {
                        Some(k) => println!(
                            "{} Siegel mit Schlüssel {}…, {} Einträge nach dem letzten Siegel",
                            s.seals,
                            k.get(..16).unwrap_or(k),
                            s.unsealed
                        ),
                        None => println!("nicht gesiegelt"),
                    }
                    if s.truncated {
                        println!("Hinweis: letzte Zeile unvollständig (Abbruch beim Schreiben).");
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            match output {
                Output::Json => print_json(&json!({ "valid": false, "error": e.to_string() })),
                Output::Text => println!("Auditprotokoll UNGÜLTIG: {e}"),
            }
            ExitCode::from(EXIT_INVALID)
        }
    }
}

/// Erzeugt ein zufälliges Zugangstoken und seinen SHA-256-Hash für die Benutzerdatei.
pub(crate) fn token(name: &str, output: Output) -> ExitCode {
    let mut bytes = [0u8; 32];
    if let Err(e) = getrandom::fill(&mut bytes) {
        return fail_usage(&format!("Zufallsquelle: {e}"));
    }
    let token = format!("rit_{}", to_hex(&bytes));
    let hash = to_hex(&Sha256::digest(token.as_bytes()));
    match output {
        Output::Json => print_json(&json!({ "user": name, "token": token, "token_sha256": hash })),
        Output::Text => {
            println!("Token (nur jetzt sichtbar, sicher weitergeben):\n  {token}\n");
            println!("Eintrag für die Benutzerdatei:\n");
            println!(
                "[[user]]\nname = \"{name}\"\nroles = [\"beobachter\"]\ntoken_sha256 = \"{hash}\""
            );
        }
    }
    ExitCode::SUCCESS
}
