//! Plan-Image von rail-it.
//!
//! Die Projektierung (`railit-planning`) übersetzt ein Netz in Tabellen, mit denen der
//! Sicherheitskern arbeitet: Fahrstraßen mit Weichenlagen und Gleisfreimeldeabschnitten,
//! feindliche Fahrstraßen, Zuordnung der Weichen zu Gleisfreimeldeabschnitten. Diese Tabellen
//! werden in ein **Plan-Image** (`.rplan`) geschrieben:
//!
//! - binär, versioniert, little-endian,
//! - gelesen **ohne Kopie und ohne Allokator** ([`PlanView`]), auf Embedded direkt aus dem Flash,
//! - beim Lesen vollständig auf Struktur und Verweise geprüft,
//! - durch SHA-256 gegen Verfälschung und optional durch Ed25519 gegen Unterschieben
//!   geschützt (REQ-PLN-001 … REQ-PLN-004).
//!
//! Mit dem Feature `alloc` stehen außerdem das Eigentümer-Modell `Plan` sowie das
//! Schreiben und Signieren zur Verfügung.
//!
//! Das Format ist in [`format`] beschrieben.

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

mod error;
pub mod format;
#[cfg(feature = "alloc")]
mod model;
mod view;

pub use ed25519_dalek::{SigningKey, VerifyingKey};
pub use error::PlanError;
#[cfg(feature = "alloc")]
pub use model::{BorderDef, LxDef, Plan, PointDef, RouteDef, SignalDef, TvdDef};
pub use view::{
    Flank, FlankEntries, LxEntries, PlanView, PointEntries, Protection, RouteEnd, RouteView,
    TvdEntries,
};

pub use railit_types::{
    BorderId, LxId, PointId, PointPosition, RouteId, SignalId, SignalKind, SignalSystem, Speed,
    TvdId,
};
