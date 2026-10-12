//! Sicherheitskern von rail-it: Fahrwegsicherung.
//!
//! Der Kern ist eine deterministische Zustandsmaschine (Konzept, Abschnitt 10):
//!
//! ```text
//! (Zustand, Zeit, Eingaben) ──step──▶ (Zustand, Stellbefehle, Ereignisse)
//! ```
//!
//! - **Eingaben** ([`Input`]): Meldungen von Weichen, Signalen, Gleisfreimeldung und
//!   Bahnübergängen sowie Bedienanforderungen.
//! - **Ausgaben** ([`Outputs`]): Stellbefehle ([`Command`]) und Ereignisse ([`Event`]).
//! - **Zustand** ([`KernelState`]): liegt in vom Aufrufer bereitgestelltem Speicher; der Kern
//!   allokiert nicht. Auf dem Server liefert `OwnedState` (Feature `alloc`) den Speicher.
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
//! - Durchrutschweg, Flankenschutz und Bahnübergänge gehören zur Fahrstraße: Sie werden mit
//!   ihr verschlossen, sind Bedingung für die Fahrerlaubnis und werden mit dem Abschnitt
//!   aufgelöst, den sie schützen (REQ-IXL-012, REQ-IXL-013, REQ-IXL-016).
//! - Ein Abschnitt, der ohne erlaubte Fahrt besetzt wird, löst einen Alarm aus
//!   (REQ-IXL-011).
//! - Hilfshandlungen heben Schutzmechanismen nur nach Bestätigung durch einen zweiten
//!   Bediener auf und werden gezählt (REQ-IXL-014, REQ-IXL-015).

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
    Alarm, Aspect, AuxId, AuxOperation, BorderMessage, Command, Event, Input, LxReport, Occupancy,
    OperatorId, Outputs, PointReport, Rejection, Request, RequestId, RoutePhase, SafeStateReason,
    SignalReport,
};
pub use kernel::Interlocking;
pub use state::{
    AUX_SLOTS, AuxSlot, BorderState, Buffers, KernelState, LxState, Meta, PointState, RouteState,
    SignalState, TvdState,
};
#[cfg(feature = "alloc")]
pub use state::{OwnedState, VecOutputs};
