//! Fehler der Projektierung.

use std::fmt;

use railit_plan::PlanError;

/// Einzelner Projektierungsfehler.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanningError {
    /// Ein Hauptsignal steht nicht am Gleisende in seiner Wirkrichtung.
    SignalNotAtTrackEnd {
        /// Signal.
        signal: String,
    },
    /// Hinter einem Hauptsignal beginnt kein neuer Gleisfreimeldeabschnitt.
    SignalNotAtTvdBoundary {
        /// Signal.
        signal: String,
    },
    /// Hinter einem Hauptsignal liegt unmittelbar ein Netzende.
    SignalFacesTerminal {
        /// Signal.
        signal: String,
    },
    /// Mehrere Hauptsignale derselben Richtung am selben Gleisende.
    DuplicateSignals {
        /// Gleis.
        track: String,
    },
    /// Ein Weg ab einem Signal kehrt zu einem bereits befahrenen Element zurück, ohne ein
    /// Zielsignal zu erreichen.
    Loop {
        /// Startsignal.
        signal: String,
    },
    /// Eine Fahrstraße betritt denselben Gleisfreimeldeabschnitt zweimal.
    RepeatedTvd {
        /// Fahrstraße.
        route: String,
        /// Abschnitt.
        tvd: String,
    },
    /// Ein Element des Netzes ist inkonsistent (z. B. ohne Gleisfreimeldeabschnitt).
    Inconsistent(String),
    /// Für eine Weiche lässt sich kein Flankenschutz bestimmen.
    UnprotectedFlank {
        /// Fahrstraße.
        route: String,
        /// Weiche, deren Flanke ungeschützt bleibt.
        point: String,
        /// Grund.
        reason: String,
    },
    /// Ein Vorsignal kündigt kein eindeutiges Hauptsignal an.
    DistantSignal {
        /// Vorsignal.
        signal: String,
        /// Grund.
        reason: String,
    },
    /// Die unabhängige Gegenprüfung weicht vom Generator ab.
    Verification(String),
    /// Das Plan-Image konnte nicht geschrieben oder gelesen werden.
    Image(PlanError),
}

impl PlanningError {
    /// Kennung der Anforderung, die diese Prüfung absichert.
    pub const fn requirement(&self) -> &'static str {
        match self {
            Self::SignalNotAtTrackEnd { .. }
            | Self::SignalNotAtTvdBoundary { .. }
            | Self::SignalFacesTerminal { .. }
            | Self::DuplicateSignals { .. } => "REQ-PLN-010",
            Self::Loop { .. } | Self::RepeatedTvd { .. } => "REQ-PLN-014",
            Self::Inconsistent(_) => "REQ-PLN-011",
            Self::Verification(_) => "REQ-PLN-013",
            Self::UnprotectedFlank { .. } => "REQ-PLN-016",
            Self::DistantSignal { .. } => "REQ-PLN-017",
            Self::Image(_) => "REQ-PLN-002",
        }
    }
}

impl fmt::Display for PlanningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignalNotAtTrackEnd { signal } => write!(
                f,
                "Signal „{signal}“ steht nicht am Gleisende in Wirkrichtung \
                 (Hauptsignale müssen an einer Gleisfreimeldegrenze stehen)"
            ),
            Self::SignalNotAtTvdBoundary { signal } => write!(
                f,
                "hinter Signal „{signal}“ beginnt kein neuer Gleisfreimeldeabschnitt"
            ),
            Self::SignalFacesTerminal { signal } => {
                write!(f, "Signal „{signal}“ steht unmittelbar vor einem Netzende")
            }
            Self::DuplicateSignals { track } => write!(
                f,
                "am Gleis „{track}“ stehen mehrere Hauptsignale derselben Richtung am selben Ende"
            ),
            Self::Loop { signal } => {
                write!(
                    f,
                    "Weg ab Signal „{signal}“ bildet eine Schleife ohne Zielsignal"
                )
            }
            Self::RepeatedTvd { route, tvd } => write!(
                f,
                "Fahrstraße „{route}“ befährt Gleisfreimeldeabschnitt „{tvd}“ mehrfach"
            ),
            Self::Inconsistent(msg) => write!(f, "Netz inkonsistent: {msg}"),
            Self::UnprotectedFlank {
                route,
                point,
                reason,
            } => write!(
                f,
                "Fahrstraße „{route}“: Flanke der Weiche „{point}“ ist nicht schützbar ({reason})"
            ),
            Self::DistantSignal { signal, reason } => {
                write!(f, "Vorsignal „{signal}“: {reason}")
            }
            Self::Verification(msg) => write!(f, "Gegenprüfung fehlgeschlagen: {msg}"),
            Self::Image(e) => write!(f, "Plan-Image: {e}"),
        }
    }
}

impl std::error::Error for PlanningError {}

/// Alle Fehler eines Projektierungslaufs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanningReport {
    /// Fehler; nie leer.
    pub errors: Vec<PlanningError>,
}

impl PlanningReport {
    pub(crate) fn single(error: PlanningError) -> Self {
        Self {
            errors: vec![error],
        }
    }
}

impl fmt::Display for PlanningReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for e in &self.errors {
            writeln!(f, "Fehler [{}]: {e}", e.requirement())?;
        }
        Ok(())
    }
}

impl std::error::Error for PlanningReport {}
