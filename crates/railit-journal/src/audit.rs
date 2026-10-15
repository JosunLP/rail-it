//! Auditprotokoll: wer hat wann was angefordert (REQ-SEC-004).
//!
//! Anders als das Betriebsjournal hält das Auditprotokoll **Personen** und ihre Bedienungen
//! fest: angemeldeter Benutzer, Handlung, Ziel, Stellbereich und Ergebnis – auch abgewiesene
//! Zugriffe. Format wie das Journal: JSON Lines mit Hashkette
//! (`{"record": …, "prev": "<hex>", "hash": "<hex>"}`, `hash = SHA-256(prev ‖ JSON(record))`).
//!
//! Das Protokoll überdauert Neustarts: [`AuditWriter::open`] prüft eine vorhandene Datei und
//! setzt ihre Kette fort. Mit Schlüssel ([`AuditWriter::open_sealed`]) folgt jedem Eintrag ein
//! Siegel (`{"seal": …, "prev", "hash"}`, siehe [`crate::Seal`]; REQ-JRN-004).

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::seal::{self, SealKind};
use crate::{Hash32, JournalError, Seal, from_hex32, to_hex};

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

/// Zeile: entweder ein Eintrag oder ein Siegel.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditLine {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record: Option<AuditRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seal: Option<Seal>,
    prev: String,
    hash: String,
}

fn chain<T: Serialize>(prev: &Hash32, item: &T) -> io::Result<Hash32> {
    let json = serde_json::to_vec(item).map_err(io::Error::other)?;
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
    /// Anzahl gültiger Siegel.
    pub seals: u64,
    /// Schlüssel der Siegel (Hex), falls gesiegelt.
    pub seal_key: Option<String>,
    /// Einträge nach dem letzten Siegel.
    pub unsealed: u64,
    /// Uhrzeit des ersten und des letzten Siegels mit Zeitstempel (UTC, ms seit 1970).
    pub sealed_ms: Option<(u64, u64)>,
}

/// Prüft die Hashkette und die fortlaufende Nummerierung eines Auditprotokolls.
///
/// # Errors
/// [`JournalError`] bei Lese-, Format- oder Kettenfehlern.
pub fn verify_audit<R: BufRead>(reader: R) -> Result<AuditSummary, JournalError> {
    read_audit(reader, None)
}

/// Prüft Hashkette, Nummerierung und Siegel eines Auditprotokolls; jedes Siegel muss mit
/// `key` erstellt sein (REQ-JRN-004).
///
/// # Errors
/// [`JournalError`] bei Lese-, Format-, Ketten- oder Siegelfehlern.
pub fn verify_audit_sealed<R: BufRead>(
    reader: R,
    key: &VerifyingKey,
) -> Result<AuditSummary, JournalError> {
    read_audit(reader, Some(key))
}

fn read_audit<R: BufRead>(
    reader: R,
    trusted: Option<&VerifyingKey>,
) -> Result<AuditSummary, JournalError> {
    let mut prev: Hash32 = [0; 32];
    let mut records = 0u64;
    let mut last_seq = 0u64;
    let mut truncated = false;
    let mut seals = 0u64;
    let mut seal_key = None;
    let mut seal_times = seal::SealTimes::default();
    let mut unsealed = 0u64;
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
        let hash = match (&parsed.record, &parsed.seal) {
            (Some(r), None) => chain(&prev, r),
            (None, Some(s)) => chain(&prev, s),
            _ => {
                return Err(JournalError::Syntax {
                    line: number,
                    message: "weder Eintrag noch Siegel".into(),
                });
            }
        }
        .map_err(|e| JournalError::Io(e.to_string()))?;
        if from_hex32(&parsed.prev) != Some(prev) || from_hex32(&parsed.hash) != Some(hash) {
            return Err(JournalError::Chain { line: number });
        }
        if let Some(s) = &parsed.seal {
            seal::check(SealKind::Audit, s, &prev, trusted, &mut seal_key)
                .and_then(|()| seal_times.add(s))
                .map_err(|message| JournalError::Seal {
                    line: number,
                    message,
                })?;
            seals = seals.saturating_add(1);
            unsealed = 0;
            prev = hash;
            continue;
        }
        let Some(record) = &parsed.record else {
            continue;
        };
        if record.seq != last_seq.saturating_add(1) {
            return Err(JournalError::Format(format!(
                "Zeile {number}: Nummer {} statt {}",
                record.seq,
                last_seq.saturating_add(1)
            )));
        }
        prev = hash;
        last_seq = record.seq;
        records = records.saturating_add(1);
        unsealed = unsealed.saturating_add(1);
    }
    Ok(AuditSummary {
        records,
        last_seq,
        head: to_hex(&prev),
        truncated,
        seals,
        seal_key,
        unsealed,
        sealed_ms: seal_times.first.zip(seal_times.last),
    })
}

/// Schreibt das Auditprotokoll; jeder Eintrag wird sofort auf das Medium geschrieben.
#[derive(Debug)]
pub struct AuditWriter<W: Write> {
    out: W,
    prev: Hash32,
    seq: u64,
    sealer: Option<SigningKey>,
}

impl AuditWriter<File> {
    /// Öffnet ein Auditprotokoll zum Anhängen. Eine vorhandene Datei wird zuerst geprüft;
    /// ist ihre Kette gebrochen, startet der Dienst nicht.
    ///
    /// # Errors
    /// Lese- oder Schreibfehler, gebrochene Kette oder unvollständige letzte Zeile.
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::open_with(path, None)
    }

    /// Wie [`Self::open`], aber jedem Eintrag folgt ein Siegel mit `key`. Eine vorhandene Datei
    /// darf nur Siegel dieses Schlüssels enthalten.
    ///
    /// # Errors
    /// Wie [`Self::open`], dazu fremde oder falsche Siegel.
    pub fn open_sealed(path: &Path, key: SigningKey) -> io::Result<Self> {
        Self::open_with(path, Some(key))
    }

    fn open_with(path: &Path, sealer: Option<SigningKey>) -> io::Result<Self> {
        let (prev, seq) = if path.exists() {
            let reader = BufReader::new(File::open(path)?);
            let trusted = sealer.as_ref().map(SigningKey::verifying_key);
            let summary = read_audit(reader, trusted.as_ref())
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
        Ok(Self {
            out,
            prev,
            seq,
            sealer,
        })
    }
}

impl<W: Write> AuditWriter<W> {
    /// Neues Protokoll in `out`.
    pub fn new(out: W) -> Self {
        Self {
            out,
            prev: [0; 32],
            seq: 0,
            sealer: None,
        }
    }

    /// Neues gesiegeltes Protokoll in `out`.
    pub fn new_sealed(out: W, key: SigningKey) -> Self {
        let mut w = Self::new(out);
        w.sealer = Some(key);
        w
    }

    fn write_line(&mut self, line: &AuditLine, hash: Hash32) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(line).map_err(io::Error::other)?;
        bytes.push(b'\n');
        self.out.write_all(&bytes)?;
        self.out.flush()?;
        self.prev = hash;
        Ok(())
    }

    /// Hängt einen Eintrag an; `seq` wird vergeben. Gibt die laufende Nummer zurück.
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn append(&mut self, mut record: AuditRecord) -> io::Result<u64> {
        record.seq = self.seq.saturating_add(1);
        let seq = record.seq;
        let hash = chain(&self.prev, &record)?;
        let line = AuditLine {
            record: Some(record),
            seal: None,
            prev: to_hex(&self.prev),
            hash: to_hex(&hash),
        };
        self.write_line(&line, hash)?;
        self.seq = seq;
        if let Some(key) = &self.sealer {
            let s = seal::make(SealKind::Audit, key, &self.prev, seal::now_ms());
            let hash = chain(&self.prev, &s)?;
            let line = AuditLine {
                record: None,
                seal: Some(s),
                prev: to_hex(&self.prev),
                hash: to_hex(&hash),
            };
            self.write_line(&line, hash)?;
        }
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
