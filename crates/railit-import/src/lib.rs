//! Import von Netzbeschreibungen in das rail-it-Netzmodell.
//!
//! Derzeit unterstützt: **rail-it-TOML** (siehe [`toml_format`]), das kompakte Format für
//! handgeschriebene Netze, Tests und Beispiele. railML 3 und PlanPro folgen (Konzept,
//! Abschnitt 6).
//!
//! Jeder Importer meldet **alle** Fehler auf einmal, jeweils mit Fundstelle.

pub mod toml_format;

use std::fmt;

/// Einzelne Meldung mit Fundstelle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Fundstelle in der Quelle, z. B. `link[3]` oder `signal „A“`.
    pub context: String,
    /// Beschreibung.
    pub message: String,
    /// Abgesicherte Anforderung, falls es sich um eine Regel des Netzmodells handelt.
    pub requirement: Option<&'static str>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(req) = self.requirement {
            write!(f, "[{req}] ")?;
        }
        if !self.context.is_empty() {
            write!(f, "{}: ", self.context)?;
        }
        f.write_str(&self.message)
    }
}

/// Fehlgeschlagener Import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// Die Quelle ist syntaktisch kein gültiges Dokument des Formats.
    Syntax(String),
    /// Die Quelle ist lesbar, beschreibt aber kein gültiges Netz.
    Invalid {
        /// Alle gefundenen Fehler.
        errors: Vec<Diagnostic>,
        /// Warnungen.
        warnings: Vec<String>,
    },
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(msg) => write!(f, "Syntaxfehler: {msg}"),
            Self::Invalid { errors, warnings } => {
                writeln!(f, "Netz ist ungültig ({} Fehler):", errors.len())?;
                for e in errors {
                    writeln!(f, "  Fehler: {e}")?;
                }
                for w in warnings {
                    writeln!(f, "  Warnung: {w}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ImportError {}
