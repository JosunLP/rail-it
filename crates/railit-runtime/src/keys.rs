//! Schlüssel zum Signieren und Prüfen von Plan-Images.
//!
//! Dateiformat: eine Zeile mit 64 Hexadezimalzeichen (32 Byte), für den geheimen wie für den
//! öffentlichen Schlüssel.

use std::path::Path;

use railit_plan::{SigningKey, VerifyingKey};

/// Bytes als Kleinbuchstaben-Hex.
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Hex → 32 Byte.
///
/// # Errors
/// Beschreibung, wenn die Eingabe keine 64 Hex-Zeichen sind.
pub fn from_hex32(text: &str) -> Result<[u8; 32], String> {
    let text = text.trim();
    if text.len() != 64 || !text.is_ascii() {
        return Err("erwartet werden 64 Hexadezimalzeichen".into());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let pair = text.get(i * 2..i * 2 + 2).ok_or("ungültige Länge")?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| format!("„{pair}“ ist kein Hex"))?;
    }
    Ok(out)
}

/// Erzeugt einen neuen Signaturschlüssel aus dem Zufallsgenerator des Betriebssystems.
///
/// # Errors
/// Wenn kein Zufall verfügbar ist.
pub fn generate() -> Result<SigningKey, String> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| format!("kein Zufall verfügbar: {e}"))?;
    Ok(SigningKey::from_bytes(&seed))
}

/// Liest einen geheimen Schlüssel.
///
/// # Errors
/// Datei nicht lesbar oder Format ungültig.
pub fn read_signing_key(path: &Path) -> Result<SigningKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(SigningKey::from_bytes(
        &from_hex32(&text).map_err(|e| format!("{}: {e}", path.display()))?,
    ))
}

/// Liest einen öffentlichen Schlüssel.
///
/// # Errors
/// Datei nicht lesbar, Format ungültig oder kein gültiger Ed25519-Punkt.
pub fn read_verifying_key(path: &Path) -> Result<VerifyingKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = from_hex32(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| format!("{}: kein gültiger öffentlicher Schlüssel", path.display()))
}
