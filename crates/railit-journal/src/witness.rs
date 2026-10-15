//! Zeugen: unabhängige Zeitstempel für Hashketten (REQ-JRN-010).
//!
//! Ein Siegel trägt die Uhrzeit des Stellwerks – das Stellwerk bezeugt sich damit aber selbst.
//! Ein **Zeuge** mit eigenem Schlüssel (etwa ein Zeitserver oder die Revision) bestätigt, dass
//! eine Zeile der Kette zu einer Zeit bereits bestand: Er signiert
//! `railit-witness-v1\n<Zeit>\n<Hash>` mit dem Hash einer Zeile und seiner Uhrzeit. Weil jede
//! Zeile alle vorigen einschließt, ist damit der ganze Stand bis dahin bezeugt. Wer die Datei
//! später neu schreibt, kann alte Zeugnisse nicht nachmachen.
//!
//! Zeugnisse stehen getrennt von der Datei in `<Datei>.witness`, eine JSON-Zeile je Zeugnis.

use std::collections::HashMap;
use std::io::BufRead;

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::seal::from_hex64;
use crate::{Hash32, from_hex32, to_hex};

/// Zeugnis über den Stand einer Hashkette.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    /// Hash der bezeugten Zeile (Hex).
    pub head: String,
    /// Uhrzeit des Zeugen (UTC, ms seit 1970).
    pub time_ms: u64,
    /// Öffentlicher Schlüssel des Zeugen (Hex).
    pub key: String,
    /// Ed25519-Signatur (Hex).
    pub signature: String,
}

fn message(head: &Hash32, time_ms: u64) -> Vec<u8> {
    let mut m = b"railit-witness-v1\n".to_vec();
    m.extend_from_slice(time_ms.to_string().as_bytes());
    m.push(b'\n');
    m.extend_from_slice(head);
    m
}

/// Bezeugt den Stand `head` zur Zeit `time_ms`.
#[must_use]
pub fn witness(key: &SigningKey, head: &Hash32, time_ms: u64) -> Witness {
    Witness {
        head: to_hex(head),
        time_ms,
        key: to_hex(key.verifying_key().as_bytes()),
        signature: to_hex(&key.sign(&message(head, time_ms)).to_bytes()),
    }
}

/// Uhrzeit (UTC, ms seit 1970).
#[must_use]
pub fn now_ms() -> u64 {
    crate::seal::now_ms()
}

/// Zeit als `JJJJ-MM-TT hh:mm:ss UTC` (proleptischer gregorianischer Kalender).
#[must_use]
pub fn format_utc(ms: u64) -> String {
    let secs = ms / 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(i64::MAX);
    let rest = secs % 86_400;
    // Tage seit 1970-01-01 → Datum (Algorithmus von H. Hinnant).
    let z = days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Geprüftes Zeugnis: bezeugte Zeile (ab 1) und Zeit.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Witnessed {
    /// Zeilennummer der bezeugten Zeile in der Datei.
    pub line: u64,
    /// Zeit des Zeugen.
    pub time_ms: u64,
}

/// Hash der Zeile (Feld `hash` jeder Zeile von Journal und Auditprotokoll).
#[derive(Deserialize)]
struct LineHash {
    hash: String,
}

/// Prüft Zeugnisse gegen eine Datei, deren Hashkette schon geprüft ist: Jedes Zeugnis muss
/// von `trusted` signiert sein und eine Zeile der Datei bezeugen; je späterer Zeile darf die
/// Zeit nicht früher sein.
///
/// # Errors
/// Beschreibung des ersten ungültigen Zeugnisses.
pub fn verify_witnesses<R: BufRead, W: BufRead>(
    file: R,
    witnesses: W,
    trusted: &VerifyingKey,
) -> Result<Vec<Witnessed>, String> {
    let mut lines: HashMap<Hash32, u64> = HashMap::new();
    for (i, line) in file.lines().enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(h) = serde_json::from_str::<LineHash>(&line)
            .ok()
            .and_then(|l| from_hex32(&l.hash))
        {
            lines.insert(h, u64::try_from(i).unwrap_or(u64::MAX).saturating_add(1));
        }
    }
    let mut out: Vec<Witnessed> = Vec::new();
    for (i, line) in witnesses.lines().enumerate() {
        let n = i.saturating_add(1);
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let w: Witness = serde_json::from_str(&line).map_err(|e| format!("Zeugnis {n}: {e}"))?;
        if from_hex32(&w.key).as_ref() != Some(trusted.as_bytes()) {
            return Err(format!("Zeugnis {n}: fremder Zeuge"));
        }
        let head = from_hex32(&w.head).ok_or(format!("Zeugnis {n}: Hash ungültig"))?;
        let sig = from_hex64(&w.signature).ok_or(format!("Zeugnis {n}: Signatur ungültig"))?;
        trusted
            .verify(&message(&head, w.time_ms), &Signature::from_bytes(&sig))
            .map_err(|_| format!("Zeugnis {n}: Signatur falsch"))?;
        let at = *lines
            .get(&head)
            .ok_or(format!("Zeugnis {n}: Stand nicht in der Datei"))?;
        out.push(Witnessed {
            line: at,
            time_ms: w.time_ms,
        });
    }
    out.sort_by_key(|w| w.line);
    for pair in out.windows(2) {
        if let [a, b] = pair
            && b.time_ms < a.time_ms
        {
            return Err(format!(
                "Zeugnis für Zeile {} ist älter als das für Zeile {}",
                b.line, a.line
            ));
        }
    }
    Ok(out)
}

/// Hash der letzten vollständigen Zeile einer Datei (der zu bezeugende Stand).
///
/// # Errors
/// Lesefehler oder keine Zeile mit Hash.
pub fn last_hash<R: BufRead>(file: R) -> Result<Hash32, String> {
    let mut last = None;
    for line in file.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(h) = serde_json::from_str::<LineHash>(&line)
            .ok()
            .and_then(|l| from_hex32(&l.hash))
        {
            last = Some(h);
        }
    }
    last.ok_or_else(|| "keine Zeile mit Hash".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(n: u8) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for i in 1..=n {
            let _ = writeln!(out, "{{\"hash\":\"{}\",\"x\":1}}", to_hex(&[i; 32]));
        }
        out
    }

    #[test]
    fn times_are_formatted_in_utc() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_utc(951_782_400_000), "2000-02-29 00:00:00 UTC");
        assert_eq!(format_utc(1_791_000_000_000), "2026-10-03 04:00:00 UTC");
    }

    #[test]
    fn witnesses_attest_lines_in_order() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let other = SigningKey::from_bytes(&[6; 32]);
        let text = file(5);
        let line = |w: &Witness| serde_json::to_string(w).unwrap() + "\n";
        let good = line(&witness(&key, &[2; 32], 1000)) + &line(&witness(&key, &[5; 32], 2000));
        let ok = verify_witnesses(text.as_bytes(), good.as_bytes(), &key.verifying_key());
        assert_eq!(
            ok,
            Ok(vec![
                Witnessed {
                    line: 2,
                    time_ms: 1000
                },
                Witnessed {
                    line: 5,
                    time_ms: 2000
                }
            ])
        );
        assert_eq!(last_hash(text.as_bytes()), Ok([5; 32]));
        // Rückwärts laufende Zeit, fremder Zeuge, unbekannter Stand, verfälschte Zeit.
        let back = line(&witness(&key, &[2; 32], 3000)) + &line(&witness(&key, &[5; 32], 2000));
        assert!(verify_witnesses(text.as_bytes(), back.as_bytes(), &key.verifying_key()).is_err());
        let foreign = line(&witness(&other, &[2; 32], 1000));
        assert!(
            verify_witnesses(text.as_bytes(), foreign.as_bytes(), &key.verifying_key()).is_err()
        );
        let unknown = line(&witness(&key, &[9; 32], 1000));
        assert!(
            verify_witnesses(text.as_bytes(), unknown.as_bytes(), &key.verifying_key()).is_err()
        );
        let mut forged = witness(&key, &[2; 32], 1000);
        forged.time_ms = 1;
        assert!(
            verify_witnesses(
                text.as_bytes(),
                line(&forged).as_bytes(),
                &key.verifying_key()
            )
            .is_err()
        );
    }
}
