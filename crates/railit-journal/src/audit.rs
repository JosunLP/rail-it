//! Auditprotokoll: wer hat wann was angefordert (REQ-SEC-004).
//!
//! Anders als das Betriebsjournal hält das Auditprotokoll **Personen** und ihre Bedienungen
//! fest: angemeldeter Benutzer, Handlung, Ziel, Stellbereich und Ergebnis – auch abgewiesene
//! Zugriffe. Format wie das Journal: JSON Lines mit Hashkette
//! (`{"record": …, "prev": "<hex>", "hash": "<hex>"}`, `hash = SHA-256(prev ‖ JSON(record))`).
//!
//! Das Protokoll überdauert Neustarts: [`AuditWriter::open`] prüft eine vorhandene Datei und
//! setzt ihre Kette fort.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Hash32, JournalError, from_hex32, to_hex};

/// Eintrag im Auditprotokoll.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// Laufende Nummer (ab 1, über Neustarts fortlaufend).
    pub seq: u64,
    /// Zeitpunkt in Millisekunden seit 1970 (UTC).
    pub time_ms: u64,
    /// Angemeldeter Benutzer.
    pub user: String,
    /// Handlung, z. B. `"set_route"`.
    pub action: String,
    /// Ziel der Handlung (Fahrstraße, Weiche, Zug, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Stellbereich.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// Ergebnis: `"ok"`, `"rejected: <Grund>"`, `"denied"` oder `"error: <Beschreibung>"`.
    pub outcome: String,
}

#[derive(Serialize, Deserialize)]
struct AuditLine {
    record: AuditRecord,
    prev: String,
    hash: String,
}

fn chain(prev: &Hash32, record: &AuditRecord) -> io::Result<Hash32> {
    let json = serde_json::to_vec(record).map_err(io::Error::other)?;
    let mut h = Sha256::new();
    h.update(prev);
    h.update(&json);
    Ok(h.finalize().into())
}

/// Ergebnis der Prüfung eines Auditprotokolls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditSummary {
    /// Anzahl der Einträge.
    pub records: u64,
    /// Laufende Nummer des letzten Eintrags.
    pub last_seq: u64,
    /// Hash der letzten Zeile.
    pub head: String,
    /// Die letzte Zeile war unvollständig (Abbruch beim Schreiben) und wurde ignoriert.
    pub truncated: bool,
}

/// Prüft die Hashkette und die fortlaufende Nummerierung eines Auditprotokolls.
///
/// # Errors
/// [`JournalError`] bei Lese-, Format- oder Kettenfehlern.
pub fn verify_audit<R: BufRead>(reader: R) -> Result<AuditSummary, JournalError> {
    let mut prev: Hash32 = [0; 32];
    let mut records = 0u64;
    let mut last_seq = 0u64;
    let mut truncated = false;
    let mut lines = reader.lines().enumerate().peekable();
    while let Some((index, line)) = lines.next() {
        let number = index.saturating_add(1);
        let text = line.map_err(|e| JournalError::Io(e.to_string()))?;
        if text.trim().is_empty() {
            continue;
        }
        let parsed: AuditLine = match serde_json::from_str(&text) {
            Ok(l) => l,
            Err(e) if e.is_eof() && lines.peek().is_none() => {
                truncated = true;
                break;
            }
            Err(e) => {
                return Err(JournalError::Syntax {
                    line: number,
                    message: e.to_string(),
                });
            }
        };
        let hash = chain(&prev, &parsed.record).map_err(|e| JournalError::Io(e.to_string()))?;
        if from_hex32(&parsed.prev) != Some(prev) || from_hex32(&parsed.hash) != Some(hash) {
            return Err(JournalError::Chain { line: number });
        }
        if parsed.record.seq != last_seq.saturating_add(1) {
            return Err(JournalError::Format(format!(
                "Zeile {number}: Nummer {} statt {}",
                parsed.record.seq,
                last_seq.saturating_add(1)
            )));
        }
        prev = hash;
        last_seq = parsed.record.seq;
        records = records.saturating_add(1);
    }
    Ok(AuditSummary {
        records,
        last_seq,
        head: to_hex(&prev),
        truncated,
    })
}

/// Schreibt das Auditprotokoll; jeder Eintrag wird sofort auf das Medium geschrieben.
#[derive(Debug)]
pub struct AuditWriter<W: Write> {
    out: W,
    prev: Hash32,
    seq: u64,
}

impl AuditWriter<File> {
    /// Öffnet ein Auditprotokoll zum Anhängen. Eine vorhandene Datei wird zuerst geprüft;
    /// ist ihre Kette gebrochen, startet der Dienst nicht.
    ///
    /// # Errors
    /// Lese- oder Schreibfehler, gebrochene Kette oder unvollständige letzte Zeile.
    pub fn open(path: &Path) -> io::Result<Self> {
        let (prev, seq) = if path.exists() {
            let summary = verify_audit(BufReader::new(File::open(path)?))
                .map_err(|e| io::Error::other(format!("Auditprotokoll ungültig: {e}")))?;
            if summary.truncated {
                return Err(io::Error::other(
                    "Auditprotokoll endet mit einer unvollständigen Zeile",
                ));
            }
            let head = from_hex32(&summary.head)
                .ok_or_else(|| io::Error::other("Auditprotokoll: Kopf ungültig"))?;
            (head, summary.last_seq)
        } else {
            ([0; 32], 0)
        };
        let out = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { out, prev, seq })
    }
}

impl<W: Write> AuditWriter<W> {
    /// Neues Protokoll in `out`.
    pub fn new(out: W) -> Self {
        Self {
            out,
            prev: [0; 32],
            seq: 0,
        }
    }

    /// Hängt einen Eintrag an; `seq` wird vergeben. Gibt die laufende Nummer zurück.
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn append(&mut self, mut record: AuditRecord) -> io::Result<u64> {
        record.seq = self.seq.saturating_add(1);
        let hash = chain(&self.prev, &record)?;
        let line = AuditLine {
            record,
            prev: to_hex(&self.prev),
            hash: to_hex(&hash),
        };
        let mut bytes = serde_json::to_vec(&line).map_err(io::Error::other)?;
        bytes.push(b'\n');
        self.out.write_all(&bytes)?;
        self.out.flush()?;
        self.prev = hash;
        self.seq = line.record.seq;
        Ok(self.seq)
    }

    /// Hash der letzten Zeile.
    pub fn head(&self) -> String {
        to_hex(&self.prev)
    }

    /// Gibt das Ziel zurück.
    pub fn into_inner(self) -> W {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(user: &str, action: &str) -> AuditRecord {
        AuditRecord {
            seq: 0,
            time_ms: 1,
            user: user.into(),
            action: action.into(),
            target: Some("A-N1".into()),
            area: None,
            outcome: "ok".into(),
        }
    }

    #[test]
    fn chain_detects_tampering() {
        let mut w = AuditWriter::new(Vec::new());
        assert_eq!(w.append(record("fdl", "set_route")).unwrap(), 1);
        assert_eq!(w.append(record("fdl", "cancel_route")).unwrap(), 2);
        let head = w.head();
        let bytes = w.into_inner();
        let summary = verify_audit(bytes.as_slice()).unwrap();
        assert_eq!((summary.records, summary.last_seq), (2, 2));
        assert_eq!(summary.head, head);

        let text = String::from_utf8(bytes.clone()).unwrap();
        let forged = text.replacen("\"fdl\"", "\"admin\"", 1);
        assert_eq!(
            verify_audit(forged.as_bytes()),
            Err(JournalError::Chain { line: 1 })
        );
        let dropped = text.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert!(verify_audit(dropped.as_bytes()).is_err());
    }

    #[test]
    fn reopened_log_continues_the_chain() {
        let dir = std::env::temp_dir().join(format!("railit-audit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.log");
        let _ = std::fs::remove_file(&path);
        let mut w = AuditWriter::open(&path).unwrap();
        w.append(record("a", "x")).unwrap();
        drop(w);
        let mut w = AuditWriter::open(&path).unwrap();
        assert_eq!(w.append(record("b", "y")).unwrap(), 2);
        drop(w);
        let summary = verify_audit(BufReader::new(File::open(&path).unwrap())).unwrap();
        assert_eq!(summary.records, 2);

        // Manipulierte Datei: Der Dienst startet nicht.
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replacen("\"b\"", "\"c\"", 1)).unwrap();
        assert!(AuditWriter::open(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
