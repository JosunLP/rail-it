//! Import von Netzbeschreibungen in das rail-it-Netzmodell.
//!
//! Unterstützt:
//!
//! - **rail-it-TOML** (siehe [`toml_format`]), das kompakte Format für handgeschriebene
//!   Netze, Tests und Beispiele;
//! - **railML 3** (siehe [`railml`]), Import und Export der Infrastruktur;
//! - **PlanPro** 1.9 und 1.10 (siehe [`planpro`]), das deutsche Planungsformat für ESTW,
//!   Import und Export der Topologie und der Außenanlagen.
//!
//! [`parse`] wählt das Format nach der Dateiendung.
//!
//! Jeder Importer meldet **alle** Fehler auf einmal, jeweils mit Fundstelle.

pub mod generate;
pub mod planpro;
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
    /// PlanPro.
    PlanPro,
}

impl Format {
    /// Format nach der Dateiendung: `.xml` und `.railml` sind railML, `.ppxml` und
    /// `.planpro` PlanPro, alles andere TOML.
    pub fn of_path(path: &str) -> Self {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("xml" | "railml") => Self::RailMl,
            Some("ppxml" | "planpro") => Self::PlanPro,
            _ => Self::Toml,
        }
    }
}

/// Liest eine Netzbeschreibung im Format, das die Dateiendung von `path` angibt.
///
/// # Errors
/// Wie [`toml_format::parse`], [`railml::parse`] bzw. [`planpro::parse`].
pub fn parse(path: &str, source: &str) -> Result<railit_topology::Validated, ImportError> {
    parse_with_warnings(path, source).map(|(v, _)| v)
}

/// Wie [`parse`], gibt zusätzlich die Warnungen des Imports zurück (übergangene Elemente).
///
/// # Errors
/// Wie [`parse`].
pub fn parse_with_warnings(
    path: &str,
    source: &str,
) -> Result<(railit_topology::Validated, Vec<String>), ImportError> {
    match Format::of_path(path) {
        Format::Toml => toml_format::parse(source).map(|v| (v, Vec::new())),
        Format::RailMl => railml::parse_with_warnings(source),
        Format::PlanPro => planpro::parse_with_warnings(source),
    }
}
