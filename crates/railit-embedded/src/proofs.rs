//! Formale Nachweise mit Kani (`cargo kani -p railit-embedded`), REQ-SAF-006.
//!
//! Das Dekodieren beliebiger Bytes ist für Bounded Model Checking zu tief verschachtelt
//! (serde/postcard); seine Robustheit zeigt das Fuzz-Ziel `node_wire`.

#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use railit_kernel::{Input, Occupancy};
use railit_rasta::MAX_PAYLOAD;
use railit_types::TvdId;

use crate::wire::{HostMsg, Packer};

/// Der Packer erzeugt nie Nutzdaten über der RaSTA-Grenze und verliert keine Nachricht.
#[kani::proof]
#[kani::unwind(8)]
fn packer_respects_the_payload_limit() {
    let mut packer = Packer::new();
    let mut flushed = 0usize;
    let mut too_long = false;
    let mut flush = |p: &[u8]| {
        too_long |= p.len() > MAX_PAYLOAD || p.is_empty();
        flushed += 1;
        true
    };
    let n: usize = kani::any();
    kani::assume(n <= 6);
    for _ in 0..n {
        let msg = HostMsg::Input(Input::Tvd {
            tvd: TvdId::new(kani::any()),
            occupancy: Occupancy::Occupied,
        });
        assert!(packer.push(&msg, &mut flush).is_ok());
    }
    assert!(packer.flush(&mut flush).is_ok());
    assert!(!too_long);
    assert_eq!(flushed, usize::from(n > 0));
}
