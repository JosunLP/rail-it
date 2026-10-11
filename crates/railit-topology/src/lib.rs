//! Infrastrukturmodell von rail-it.
//!
//! Das Netz besteht aus **Knoten mit Ports**:
//!
//! | Knoten       | Ports                      |
//! |--------------|----------------------------|
//! | [`Track`]    | `a`, `b`                   |
//! | [`Point`]    | `tip`, `left`, `right`     |
//! | [`Terminal`] | `end`                      |
//!
//! Verbindungen bestehen immer zwischen zwei Ports. Dadurch ist die Fahrtrichtung durch
//! eine Weiche eindeutig, und unzulässige Wege (links ↔ rechts) existieren strukturell
//! nicht (REQ-TOP-007). Punktobjekte wie [`Signal`]e liegen mit Position und Wirkrichtung
//! auf einem Gleisabschnitt; [`TvdSection`]s fassen Gleise und Weichen zu
//! Gleisfreimeldeabschnitten zusammen.
//!
//! Ein Netz entsteht über den [`TopologyBuilder`]. Erst [`TopologyBuilder::build`] liefert
//! nach vollständiger Validierung eine unveränderliche [`Topology`] (REQ-TOP-008).
//!
//! Das Crate ist `no_std` und benötigt nur `alloc`.

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

extern crate alloc;

mod builder;
mod error;
mod model;
mod topology;

pub use builder::{TopologyBuilder, Validated};
pub use error::{Report, TopologyError, Warning};
pub use model::{
    DEFAULT_DIVERGING_SPEED, Direction, ElementRef, LevelCrossing, NodeRef, Point, PointPosition,
    Port, PortRef, Signal, SignalKind, SignalSystem, Terminal, TerminalKind, Track, TvdMember,
    TvdSection,
};
pub use topology::{Exit, Exits, Topology};

pub use railit_types::{Length, LxId, PointId, SignalId, Speed, TerminalId, TrackId, TvdId};
