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
    from_hex::<32>(text)
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

/// Hex → Bytes fester Länge.
fn from_hex<const N: usize>(text: &str) -> Result<[u8; N], String> {
    let text = text.trim();
    if text.len() != N * 2 || !text.is_ascii() {
        return Err(format!("erwartet werden {} Hexadezimalzeichen", N * 2));
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        let pair = text.get(i * 2..i * 2 + 2).ok_or("ungültige Länge")?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| format!("„{pair}“ ist kein Hex"))?;
    }
    Ok(out)
}

/// Freigabedatei zu einem Plan-Image: `<plan>.approvals` (REQ-PLN-022).
#[must_use]
pub fn approvals_path(plan: &Path) -> std::path::PathBuf {
    let mut name = plan.as_os_str().to_owned();
    name.push(".approvals");
    name.into()
}

/// Eine Zeile der Freigabedatei: `<Schlüssel hex> <Signatur hex> <Name des Prüfers …>`.
#[must_use]
pub fn approval_line(approval: &railit_plan::Approval, name: &str) -> String {
    format!(
        "{} {} {}\n",
        to_hex(approval.approver.as_bytes()),
        to_hex(&approval.signature.to_bytes()),
        name.replace(['\n', '\r'], " ")
    )
}

/// Liest eine Freigabedatei. Fehlt sie, gibt es keine Freigaben. Leerzeilen und Zeilen mit
/// `#` am Anfang werden übergangen; der Name des Prüfers ist nur Auskunft.
///
/// # Errors
/// Datei nicht lesbar oder eine Zeile ungültig.
pub fn read_approvals(path: &Path) -> Result<Vec<(railit_plan::Approval, String)>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |e: String| format!("{}:{}: {e}", path.display(), n + 1);
        let mut parts = line.splitn(3, char::is_whitespace);
        let key = from_hex::<32>(parts.next().unwrap_or_default()).map_err(at)?;
        let sig = from_hex::<64>(parts.next().unwrap_or_default()).map_err(at)?;
        let approver = VerifyingKey::from_bytes(&key)
            .map_err(|_| at("kein gültiger öffentlicher Schlüssel".into()))?;
        out.push((
            railit_plan::Approval {
                approver,
                signature: railit_plan::Signature::from_bytes(&sig),
            },
            parts.next().unwrap_or_default().trim().to_string(),
        ));
    }
    Ok(out)
}

/// Prüft die Freigaben eines Plans aus `<plan>.approvals` (REQ-PLN-022). `creator` ist der
/// Schlüssel, gegen den die Signatur des Plans geprüft wurde.
///
/// # Errors
/// Beschreibung, wenn die Datei ungültig ist oder der Plan nicht ausreichend freigegeben ist.
pub fn check_plan_approvals(
    plan_path: &Path,
    plan: &railit_plan::PlanView<'_>,
    creator: Option<&VerifyingKey>,
    approvers: &[VerifyingKey],
    required: usize,
) -> Result<Vec<String>, String> {
    let file = approvals_path(plan_path);
    let entries = read_approvals(&file)?;
    let approvals: Vec<_> = entries.iter().map(|(a, _)| a.clone()).collect();
    railit_plan::check_approvals(plan, creator, &approvals, approvers, required)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    let mut names: Vec<String> = Vec::new();
    for (a, name) in &entries {
        let label = if name.is_empty() {
            to_hex(a.approver.as_bytes())
                .get(..16)
                .unwrap_or_default()
                .to_string()
        } else {
            name.clone()
        };
        if !names.contains(&label) {
            names.push(label);
        }
    }
    Ok(names)
}

/// Vertrauen in Plan-Images: Schlüssel des Erstellers und Freigaben (REQ-PLN-004, REQ-PLN-022).
#[derive(Clone, Debug, Default)]
pub struct PlanTrust {
    /// Der Plan muss mit diesem Schlüssel signiert sein (`None`: unsignierte Pläne zulassen).
    pub creator: Option<VerifyingKey>,
    /// Vertrauenswürdige Prüfer.
    pub approvers: Vec<VerifyingKey>,
    /// Verlangte Freigaben verschiedener Prüfer.
    pub required: usize,
}

impl PlanTrust {
    /// Aus Dateipfaden (`--trusted-key`, `--approver-key`, `--approvals`).
    ///
    /// # Errors
    /// Schlüssel nicht lesbar, zu wenige Prüfer für `required` oder Freigaben ohne Signatur.
    pub fn from_files(
        creator: Option<&Path>,
        approvers: &[std::path::PathBuf],
        required: usize,
    ) -> Result<Self, String> {
        let creator = creator.map(read_verifying_key).transpose()?;
        let approvers = approvers
            .iter()
            .map(|p| read_verifying_key(p))
            .collect::<Result<Vec<_>, _>>()?;
        if approvers.len() < required {
            return Err(format!(
                "{required} Freigaben verlangt, aber nur {} Prüferschlüssel (--approver-key)",
                approvers.len()
            ));
        }
        if required > 0 && creator.is_none() {
            return Err("Freigaben setzen einen signierten Plan voraus (--trusted-key)".into());
        }
        if creator.is_some_and(|c| approvers.contains(&c)) {
            return Err(
                "Vier-Augen-Prinzip: Der Schlüssel des Erstellers ist kein Prüferschlüssel".into(),
            );
        }
        Ok(Self {
            creator,
            approvers,
            required,
        })
    }

    /// Lädt ein Plan-Image und prüft Signatur und Freigaben. Gibt den Plan und die Namen der
    /// Prüfer zurück, die ihn freigegeben haben.
    ///
    /// # Errors
    /// Beschreibung, wenn der Plan nicht lesbar, ungültig oder nicht freigegeben ist.
    pub fn load(
        &self,
        path: &Path,
    ) -> Result<(railit_plan::PlanView<'static>, Vec<String>), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let plan = crate::load_plan(bytes, self.creator.as_ref())
            .map_err(|e| format!("Plan {} abgewiesen: {e}", path.display()))?;
        let mut names = Vec::new();
        if self.required > 0 {
            names = check_plan_approvals(
                path,
                &plan,
                self.creator.as_ref(),
                &self.approvers,
                self.required,
            )
            .map_err(|e| format!("Plan {} abgewiesen: {e}", path.display()))?;
        }
        Ok((plan, names))
    }
}
