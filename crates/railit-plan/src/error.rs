//! Fehler beim Lesen, Prüfen und Schreiben von Plan-Images.

use core::fmt;

/// Ein Plan-Image ist unbrauchbar.
///
/// Jeder dieser Fehler führt dazu, dass der Plan nicht geladen wird. Der Kern arbeitet nie mit
/// einem teilweise gültigen Plan (REQ-PLN-002).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanError {
    /// Das Image ist kürzer als der Kopf.
    TooShort,
    /// Die Kennung am Anfang fehlt: kein Plan-Image.
    BadMagic,
    /// Formatversion wird nicht unterstützt.
    UnsupportedVersion(u32),
    /// Die Gesamtlänge passt nicht zu den Angaben im Kopf.
    LengthMismatch,
    /// Ein Abschnitt liegt (teilweise) außerhalb des Images.
    SectionOutOfBounds(&'static str),
    /// Die Prüfsumme stimmt nicht: Das Image wurde verändert oder beschädigt.
    DigestMismatch,
    /// Ein Verweis zeigt auf ein nicht existierendes Element.
    InvalidReference {
        /// Art des Verweises.
        what: &'static str,
        /// Index des verweisenden Eintrags.
        index: usize,
    },
    /// Eine Fahrstraße hat keinen Gleisfreimeldeabschnitt.
    EmptyRoute(usize),
    /// Die Feindlichkeitsmatrix ist nicht symmetrisch oder eine Fahrstraße ist sich selbst feind.
    ConflictMatrix(usize),
    /// Ein Name ist kein gültiges UTF-8 oder liegt außerhalb der Namenstabelle.
    InvalidName(usize),
    /// Das Image ist nicht signiert, eine Signatur wird aber verlangt.
    SignatureMissing,
    /// Die Signatur passt nicht zum Schlüssel oder zum Inhalt.
    SignatureInvalid,
    /// Beim Schreiben: Der Plan überschreitet die Grenzen des Formats.
    TooLarge,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => f.write_str("Plan-Image ist zu kurz"),
            Self::BadMagic => f.write_str("keine Plan-Image-Kennung (RPLN)"),
            Self::UnsupportedVersion(v) => write!(f, "Formatversion {v} wird nicht unterstützt"),
            Self::LengthMismatch => f.write_str("Länge des Plan-Images passt nicht zum Kopf"),
            Self::SectionOutOfBounds(s) => write!(f, "Abschnitt „{s}“ liegt außerhalb des Images"),
            Self::DigestMismatch => {
                f.write_str("Prüfsumme falsch: Plan-Image wurde verändert oder ist beschädigt")
            }
            Self::InvalidReference { what, index } => {
                write!(f, "ungültiger Verweis ({what}) in Eintrag {index}")
            }
            Self::EmptyRoute(r) => write!(f, "Fahrstraße {r} hat keinen Gleisfreimeldeabschnitt"),
            Self::ConflictMatrix(r) => {
                write!(f, "Feindlichkeitsmatrix inkonsistent bei Fahrstraße {r}")
            }
            Self::InvalidName(i) => write!(f, "Name {i} ist ungültig"),
            Self::SignatureMissing => f.write_str("Plan-Image ist nicht signiert"),
            Self::SignatureInvalid => f.write_str("Signatur des Plan-Images ist ungültig"),
            Self::TooLarge => f.write_str("Plan überschreitet die Grenzen des Formats"),
        }
    }
}

impl core::error::Error for PlanError {}
