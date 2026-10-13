//! Embedded-Runtime von rail-it für den sicheren Rechner (Konzept, Abschnitt 13).
//!
//! Einsatzvariante „verteilt“: Kern und Sicherheitsschicht laufen auf einem sicheren Rechner
//! (Cortex-R im Lockstep, `armv7r-none-eabihf`), `railitd` mit Betriebsführung, API und
//! Feldanbindung auf einem Linux-Server. Beide sind über eine RaSTA-Verbindung gekoppelt.
//!
//! ```text
//!  railitd (Host)                         sicherer Rechner (Knoten)
//!  ┌──────────────┐  HostMsg: Eingaben   ┌──────────────────────────────────────┐
//!  │ Feld, API,   │ ──── RaSTA ───────▶ │ Eingangspuffer ─▶ Kanäle A/B/C ─▶ Voter│
//!  │ Betrieb      │ ◀─── RaSTA ──────── │ Ausgaben je Zyklus (Begin … End)       │
//!  └──────────────┘  NodeMsg: Ausgaben  └──────────────────────────────────────┘
//!                                          ▲ Timer-Interrupt: Node::tick
//! ```
//!
//! - [`Node`]: Kern in bis zu drei Kanälen mit Voter und Watchdog ([`railit_safety`]). Der
//!   Zyklus wird vom Timer des Knotens angestoßen, nicht vom Host: Bleiben Meldungen aus,
//!   wirken sie wie gestört und der Kern nimmt die Fahrt zurück.
//! - [`ChannelMemory`]: Zustandsspeicher fester Größe je Kanal.
//! - [`wire`]: Binärtelegramme (postcard) in RaSTA-Nutzdaten.
//! - `NodeClient` (Feature `host`): Gegenstelle für `railitd` und Tests.
//!
//! Das Crate ist `#![no_std]` und bindet keinen Allokator ein (außer mit Feature `host`):
//! Der Zyklus kann nicht allokieren. Das Plan-Image wird ohne Kopie aus dem Speicher gelesen,
//! auf dem Zielsystem direkt aus dem Flash.
//!
//! **Einbindung auf dem Zielsystem** (Skizze; Startcode, Uhr und Netzwerk stellt das BSP):
//!
//! ```ignore
//! fn main_loop(image: &'static [u8], bsp: &mut Bsp) -> ! {
//!     let plan = PlanView::parse_signed(image, &TRUSTED_KEY).unwrap_or_else(|_| bsp.halt());
//!     let mut a = ChannelMemory::<512>::new();
//!     let mut b = ChannelMemory::<512>::new();
//!     let states = [a.state(&plan), b.state(&plan), None];
//!     let mut node: Node<_, 1024, 2048, 1024> =
//!         Node::new(Interlocking::new(plan, GenericRules::default()), states, CONFIG)?;
//!     loop {
//!         while let Some(pdu) = bsp.receive() { node.receive(bsp.now_ms(), pdu); }
//!         if bsp.timer_elapsed() { node.tick(bsp.now_us(), &mut || bsp.now_us()); }
//!         while let Some(n) = node.poll_transmit(&mut buf) { bsp.send(&buf[..n]); }
//!     }
//! }
//! ```

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
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::unwrap_used
    )
)]

#[cfg(feature = "host")]
extern crate alloc;

#[cfg(feature = "host")]
mod host;
mod memory;
mod node;
#[cfg(kani)]
mod proofs;
pub mod wire;

#[cfg(feature = "host")]
pub use host::{NodeClient, NodeCycle, NodeEvent, NodeSync};
pub use memory::ChannelMemory;
pub use node::{Node, NodeConfig, NodeError, NodeStats, SYNC_PARTS_PER_TICK, TickReport};
