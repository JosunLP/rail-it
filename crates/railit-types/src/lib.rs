//! Grundtypen von rail-it.
//!
//! Dieses Crate ist Teil des sicherheitsrelevanten Pfads (SIL 4, siehe `docs/KONZEPT.md`).
//! Es ist `no_std`, benötigt keinen Allokator und enthält keine Panikpfade:
//!
//! - [`id`]: typsichere Indizes für alle Infrastrukturelemente,
//! - [`units`]: Längen und Geschwindigkeiten als Ganzzahlen,
//! - [`time`]: monotone Zeit des Stellwerkszyklus,
//! - [`bitset`]: Bitmengen über vom Aufrufer bereitgestellten Speicher.

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

pub mod bitset;
pub mod id;
pub mod time;
pub mod units;

pub use bitset::{BitSet, BitSetMut, OutOfRange};
pub use id::{PointId, SignalId, TerminalId, TrackId, TvdId};
pub use time::{Duration, Tick};
pub use units::{Length, Speed};
