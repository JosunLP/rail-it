//! Sicherheitsschicht von rail-it: redundante Kanäle, Voter, Watchdog und sicherer Zustand
//! (Konzept, Abschnitte 4 und 15).
//!
//! ```text
//!                 ┌─▶ Kanal A: Kern + eigener Zustand ─▶ Digest A ─┐
//!  Eingaben ──────┼─▶ Kanal B: Kern + eigener Zustand ─▶ Digest B ─┼─▶ Voter ─▶ Ausgaben
//!                 └─▶ Kanal C: Kern + eigener Zustand ─▶ Digest C ─┘      │
//!                                                                 sicherer Zustand
//! ```
//!
//! - **Kanäle:** Jeder Kanal führt denselben deterministischen Kern mit eigenem
//!   Zustandsspeicher aus. Sein **Digest** ist ein SHA-256 über das Ergebnis, alle
//!   Stellbefehle und Ereignisse des Zyklus sowie – alle `state_check_every` Zyklen – über den
//!   gesamten Zustand. Die Kodierung ist plattformunabhängig ([`state_digest`]).
//! - **Voter:** Bei **2-von-2** werden Ausgaben nur weitergegeben, wenn beide Digests
//!   übereinstimmen; jede Abweichung führt in den sicheren Zustand (REQ-SAF-001). Bei
//!   **2-von-3** entscheidet die Mehrheit; ein abweichender Kanal wird ausgeschlossen, der
//!   Betrieb läuft als 2-von-2 weiter. Ohne Mehrheit folgt der sichere Zustand (REQ-SAF-002).
//!   Ein ausgeschlossener Kanal kann den Zustand eines gesunden Kanals übernehmen und wieder
//!   mitwählen (REQ-SAF-003).
//! - **Sicherer Zustand:** Die Schicht gibt dann keine Ausgabe eines Kanals mehr weiter,
//!   sondern stellt aus dem Plan selbst alle Signale auf Halt, schaltet Ersatzsignale aus und
//!   lehnt alle Anforderungen ab, bis sie zurückgesetzt wird; danach beginnen alle Kanäle wie
//!   nach einem Neustart (REQ-SAF-004).
//! - **Watchdog:** überwacht Rechenzeit und Abstand der Zyklen (REQ-SAF-005).
//!
//! Die Schicht allokiert nicht. Den Zustandsspeicher ([`StateStore`]) und die Ausgabepuffer
//! ([`ChannelOutputs`]) stellt der Aufrufer: auf dem Server `OwnedState` und `VecOutputs`
//! (Feature `alloc`), auf Embedded statische Puffer und [`FixedOutputs`].

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

mod channels;
mod digest;
mod outputs;
#[cfg(kani)]
mod proofs;
mod watchdog;

pub use channels::{
    Channels, ConfigError, Divergence, Fault, MAX_CHANNELS, Redundancy, ReintegrateError,
    SafetyEvent, StateStore, Status, Trip, Verdict,
};
pub use digest::{Digest, state_digest};
pub use outputs::{ChannelOutputs, FixedOutputs};
pub use watchdog::Watchdog;
