//! Fehler und Warnungen der Netzvalidierung.
//!
//! Jeder Fehler verweist auf die Anforderung, die er absichert
//! (siehe `docs/requirements/topologie.md`).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use railit_types::Length;

use crate::model::Port;

/// Verstoß gegen eine Regel des Netzmodells.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TopologyError {
    /// Ein Name ist bereits vergeben.
    DuplicateName {
        /// Doppelt verwendeter Name.
        name: String,
    },
    /// Ein Gleisabschnitt hat die Länge null.
    ZeroLength {
        /// Gleisabschnitt.
        track: String,
    },
    /// Eine ID stammt nicht aus diesem Netz.
    UnknownId {
        /// Art des Elements.
        kind: &'static str,
        /// Rohwert der ID.
        raw: u32,
    },
    /// Der Port existiert an dieser Elementart nicht (z. B. `tip` an einem Gleis).
    InvalidPort {
        /// Element.
        element: String,
        /// Ungültiger Port.
        port: Port,
    },
    /// Ein Port soll mit sich selbst verbunden werden.
    SelfConnection {
        /// Element.
        element: String,
        /// Port.
        port: Port,
    },
    /// Ein Port ist bereits verbunden.
    PortAlreadyConnected {
        /// Element.
        element: String,
        /// Port.
        port: Port,
    },
    /// Ein Port ist weder verbunden noch durch ein Netzende abgeschlossen.
    OpenPort {
        /// Element.
        element: String,
        /// Offener Port.
        port: Port,
    },
    /// Ein Punktobjekt liegt außerhalb seines Gleisabschnitts.
    PositionOutsideTrack {
        /// Punktobjekt.
        object: String,
        /// Gleisabschnitt.
        track: String,
        /// Angegebene Position.
        position: Length,
        /// Länge des Gleisabschnitts.
        length: Length,
    },
    /// Ein Gleisfreimeldeabschnitt hat keine Elemente.
    EmptyTvd {
        /// Abschnitt.
        tvd: String,
    },
    /// Ein Gleis oder eine Weiche gehört zu keinem Gleisfreimeldeabschnitt.
    NotInTvd {
        /// Element.
        element: String,
    },
    /// Ein Gleis oder eine Weiche gehört zu mehreren Gleisfreimeldeabschnitten.
    InMultipleTvds {
        /// Element.
        element: String,
        /// Betroffene Abschnitte.
        tvds: Vec<String>,
    },
    /// Das Netz überschreitet die maximale Anzahl Elemente einer Art (`u32::MAX`).
    TooManyElements {
        /// Art des Elements.
        kind: &'static str,
    },
}

impl TopologyError {
    /// Kennung der Anforderung, die durch diese Prüfung abgesichert wird.
    pub const fn requirement(&self) -> &'static str {
        match self {
            Self::DuplicateName { .. } => "REQ-TOP-001",
            Self::OpenPort { .. } | Self::PortAlreadyConnected { .. } => "REQ-TOP-002",
            Self::InvalidPort { .. } | Self::SelfConnection { .. } | Self::UnknownId { .. } => {
                "REQ-TOP-003"
            }
            Self::ZeroLength { .. } | Self::TooManyElements { .. } => "REQ-TOP-004",
            Self::PositionOutsideTrack { .. } => "REQ-TOP-005",
            Self::EmptyTvd { .. } | Self::NotInTvd { .. } | Self::InMultipleTvds { .. } => {
                "REQ-TOP-006"
            }
        }
    }
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName { name } => write!(f, "Name „{name}“ ist mehrfach vergeben"),
            Self::ZeroLength { track } => write!(f, "Gleis „{track}“ hat die Länge 0"),
            Self::UnknownId { kind, raw } => {
                write!(f, "{kind} mit ID {raw} gehört nicht zu diesem Netz")
            }
            Self::InvalidPort { element, port } => {
                write!(f, "Element „{element}“ hat keinen Port „{port}“")
            }
            Self::SelfConnection { element, port } => {
                write!(
                    f,
                    "Port „{element}.{port}“ kann nicht mit sich selbst verbunden werden"
                )
            }
            Self::PortAlreadyConnected { element, port } => {
                write!(f, "Port „{element}.{port}“ ist bereits verbunden")
            }
            Self::OpenPort { element, port } => write!(
                f,
                "Port „{element}.{port}“ ist offen (verbinden oder mit Netzende abschließen)"
            ),
            Self::PositionOutsideTrack {
                object,
                track,
                position,
                length,
            } => write!(
                f,
                "„{object}“ liegt bei {position}, Gleis „{track}“ ist aber nur {length} lang"
            ),
            Self::EmptyTvd { tvd } => write!(f, "Gleisfreimeldeabschnitt „{tvd}“ ist leer"),
            Self::NotInTvd { element } => {
                write!(f, "„{element}“ gehört zu keinem Gleisfreimeldeabschnitt")
            }
            Self::InMultipleTvds { element, tvds } => {
                write!(
                    f,
                    "„{element}“ gehört zu mehreren Gleisfreimeldeabschnitten: "
                )?;
                for (i, tvd) in tvds.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "„{tvd}“")?;
                }
                Ok(())
            }
            Self::TooManyElements { kind } => write!(f, "Zu viele Elemente der Art {kind}"),
        }
    }
}

impl core::error::Error for TopologyError {}

/// Auffälligkeit, die kein Fehler ist, aber geprüft werden sollte.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warning {
    /// Das Netz zerfällt in mehrere nicht verbundene Teile (REQ-TOP-010).
    Disconnected {
        /// Anzahl der Teilnetze.
        components: usize,
    },
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected { components } => {
                write!(f, "Netz besteht aus {components} nicht verbundenen Teilen")
            }
        }
    }
}

/// Ergebnis einer fehlgeschlagenen Validierung: alle gefundenen Fehler und Warnungen
/// (REQ-TOP-009).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Fehler; nicht leer, wenn die Validierung fehlgeschlagen ist.
    pub errors: Vec<TopologyError>,
    /// Warnungen.
    pub warnings: Vec<Warning>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for e in &self.errors {
            writeln!(f, "Fehler [{}]: {e}", e.requirement())?;
        }
        for w in &self.warnings {
            writeln!(f, "Warnung: {w}")?;
        }
        Ok(())
    }
}

impl core::error::Error for Report {}
