//! Siegel: Signatur des Stellwerks über den Stand einer Hashkette (REQ-JRN-004).
//!
//! Die Hashkette zeigt jede Änderung an einer Zeile – aber nicht, ob jemand die Datei ganz neu
//! geschrieben hat; eine neu berechnete Kette ist wieder stimmig. Ein Siegel bindet den Stand
//! an einen Schlüssel: Es enthält den Hash der vorhergehenden Zeile und eine Ed25519-Signatur
//! darüber. Damit sind diese Zeile und alle davor bestätigt. Das Siegel ist selbst Glied der
//! Kette.
//!
//! Signiert wird `railit-seal-v1\n<Art>\n<Hash>` (Art: `journal` oder `audit`), damit ein
//! Siegel des Journals nicht als Siegel des Auditprotokolls gilt. Siegel mit Zeitstempel
//! (REQ-JRN-010) tragen die Uhrzeit des Stellwerks (UTC, ms seit 1970) und signieren
//! `railit-seal-v2\n<Art>\n<Zeit>\n<Hash>`; die Zeiten der Siegel einer Datei dürfen nicht
//! zurücklaufen. Dass ein Stand zu einer Zeit bestand, bestätigt unabhängig davon ein Zeuge
//! ([`crate::witness`]).

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{Hash32, from_hex32, to_hex};

/// Was gesiegelt wird.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum SealKind {
    /// Betriebsjournal.
    Journal,
    /// Auditprotokoll.
    Audit,
}

impl SealKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Journal => "journal",
            Self::Audit => "audit",
        }
    }
}

/// Siegel über den Stand einer Hashkette.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seal {
    /// Hash der vorhergehenden Zeile (Hex): Sie und alle davor sind bestätigt.
    pub head: String,
    /// Öffentlicher Schlüssel des Siegelnden (Hex).
    pub key: String,
    /// Ed25519-Signatur (Hex, 64 Byte).
    pub signature: String,
    /// Uhrzeit des Stellwerks beim Siegeln (UTC, ms seit 1970), Teil der Signatur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<u64>,
}

fn message(kind: SealKind, head: &Hash32, time_ms: Option<u64>) -> Vec<u8> {
    let mut m = match time_ms {
        None => b"railit-seal-v1\n".to_vec(),
        Some(_) => b"railit-seal-v2\n".to_vec(),
    };
    m.extend_from_slice(kind.label().as_bytes());
    m.push(b'\n');
    if let Some(t) = time_ms {
        m.extend_from_slice(t.to_string().as_bytes());
        m.push(b'\n');
    }
    m.extend_from_slice(head);
    m
}

/// Uhrzeit (UTC, ms seit 1970).
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Siegelt den Stand `head` mit der Uhrzeit `time_ms`.
pub(crate) fn make(kind: SealKind, key: &SigningKey, head: &Hash32, time_ms: u64) -> Seal {
    let signature = key.sign(&message(kind, head, Some(time_ms)));
    Seal {
        head: to_hex(head),
        key: to_hex(key.verifying_key().as_bytes()),
        signature: to_hex(&signature.to_bytes()),
        time_ms: Some(time_ms),
    }
}

/// Zeitspanne der Siegel einer Datei; die Zeiten dürfen nicht zurücklaufen.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SealTimes {
    pub(crate) first: Option<u64>,
    pub(crate) last: Option<u64>,
}

impl SealTimes {
    pub(crate) fn add(&mut self, seal: &Seal) -> Result<(), String> {
        let Some(t) = seal.time_ms else {
            return Ok(());
        };
        if self.last.is_some_and(|l| t < l) {
            return Err("Zeit des Siegels liegt vor der des vorherigen".into());
        }
        self.first.get_or_insert(t);
        self.last = Some(t);
        Ok(())
    }
}

/// Prüft ein Siegel über den Stand `head`. `trusted`: Das Siegel muss mit diesem Schlüssel
/// erstellt sein. `seen`: Schlüssel der früheren Siegel derselben Datei – alle Siegel einer
/// Datei tragen denselben Schlüssel.
pub(crate) fn check(
    kind: SealKind,
    seal: &Seal,
    head: &Hash32,
    trusted: Option<&VerifyingKey>,
    seen: &mut Option<String>,
) -> Result<(), String> {
    if from_hex32(&seal.head) != Some(*head) {
        return Err("Siegel bezieht sich nicht auf die vorhergehende Zeile".into());
    }
    let key_bytes = from_hex32(&seal.key).ok_or("Schlüssel ungültig")?;
    let key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| "Schlüssel ungültig")?;
    if let Some(t) = trusted
        && t.as_bytes() != key.as_bytes()
    {
        return Err("Siegel mit fremdem Schlüssel".into());
    }
    if seen.as_ref().is_some_and(|k| *k != seal.key) {
        return Err("Siegel mit wechselndem Schlüssel".into());
    }
    let sig = from_hex64(&seal.signature).ok_or("Signatur ungültig")?;
    key.verify(
        &message(kind, head, seal.time_ms),
        &Signature::from_bytes(&sig),
    )
    .map_err(|_| "Signatur falsch".to_string())?;
    *seen = Some(seal.key.clone());
    Ok(())
}

pub(crate) fn from_hex64(text: &str) -> Option<[u8; 64]> {
    let (a, b) = (text.get(..64)?, text.get(64..)?);
    let (a, b) = (from_hex32(a)?, from_hex32(b)?);
    let mut out = [0u8; 64];
    out.get_mut(..32)?.copy_from_slice(&a);
    out.get_mut(32..)?.copy_from_slice(&b);
    Some(out)
}
