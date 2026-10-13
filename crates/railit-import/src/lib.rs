//! Import von Netzbeschreibungen in das rail-it-Netzmodell.
//!
//! Unterstützt:
//!
//! - **rail-it-TOML** (siehe [`toml_format`]), das kompakte Format für handgeschriebene
//!   Netze, Tests und Beispiele;
//! - **railML 3** (siehe [`railml`]), Import und Export der Infrastruktur.
//!
//! PlanPro folgt (Konzept, Abschnitt 6). [`parse`] wählt das Format nach der Dateiendung.
//!
//! Jeder Importer meldet **alle** Fehler auf einmal, jeweils mit Fundstelle.

pub mod generate;
pub mod railml;
pub mod toml_format;

use toml_format::{metres, model_error};

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

/// Format einer Netzbeschreibung.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Format {
    /// rail-it-TOML.
    Toml,
    /// railML 3.
    RailMl,
}

impl Format {
    /// Format nach der Dateiendung: `.xml` und `.railml` sind railML, alles andere TOML.
    pub fn of_path(path: &str) -> Self {
        let railml = std::path::Path::new(path)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xml") || e.eq_ignore_ascii_case("railml"));
        if railml { Self::RailMl } else { Self::Toml }
    }
}

/// Liest eine Netzbeschreibung im Format, das die Dateiendung von `path` angibt.
///
/// # Errors
/// Wie [`toml_format::parse`] bzw. [`railml::parse`].
pub fn parse(path: &str, source: &str) -> Result<railit_topology::Validated, ImportError> {
    match Format::of_path(path) {
        Format::Toml => toml_format::parse(source),
        Format::RailMl => railml::parse(source),
    }
}
