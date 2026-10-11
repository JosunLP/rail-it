//! Sicherheitskern von rail-it: Fahrwegsicherung.
//!
//! Der Kern ist eine deterministische Zustandsmaschine (Konzept, Abschnitt 10):
//!
//! ```text
//! (Zustand, Zeit, Eingaben) ──step──▶ (Zustand, Stellbefehle, Ereignisse)
//! ```
//!
//! - **Eingaben** ([`Input`]): Meldungen von Weichen, Signalen und Gleisfreimeldung sowie
//!   Bedienanforderungen.
//! - **Ausgaben** ([`Outputs`]): Stellbefehle ([`Command`]) und Ereignisse ([`Event`]).
//! - **Zustand** ([`KernelState`]): liegt in vom Aufrufer bereitgestelltem Speicher; der Kern
//!   allokiert nicht. Auf dem Server liefert [`OwnedState`] (Feature `alloc`) den Speicher.
//!
//! Sicherheitsprinzipien:
//!
//! - Nur positiv bestätigte Zustände gelten als sicher. Fehlende oder veraltete Meldungen
//!   wirken wie „gestört“ (REQ-IXL-009).
//! - Eine Fahrstraße wird nur zugelassen, wenn keine feindliche Fahrstraße eingestellt ist,
//!   alle Abschnitte frei und alle Elemente unverschlossen sind (REQ-IXL-002/003).
//! - Fahrt zeigt ein Signal nur, solange alle Bedingungen gelten; sonst fällt es sofort auf
//!   Halt (REQ-IXL-005).
//! - Nach jedem Zyklus werden Invarianten geprüft. Eine Verletzung führt in den sicheren
//!   Zustand: alle Signale Halt, keine Stellbefehle, alle Anforderungen abgelehnt
//!   (REQ-IXL-008).
//!
//! Phase 1 umfasst Weichen, Signale und Gleisfreimeldung. Durchrutschwege, Flankenschutz,
//! Hilfshandlungen und Bahnübergänge folgen in Phase 2.

#![no_std]
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::float_arithmetic,
    clippy::as_conversions
)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod invariants;
mod io;
mod kernel;
mod state;

pub use io::{
    Alarm, Aspect, Command, Event, Input, Occupancy, Outputs, PointReport, Rejection, Request,
    RequestId, RoutePhase, SafeStateReason, SignalReport,
};
pub use kernel::Interlocking;
pub use state::{KernelState, Meta, PointState, RouteState, SignalState, TvdState};
#[cfg(feature = "alloc")]
pub use state::{OwnedState, VecOutputs};
