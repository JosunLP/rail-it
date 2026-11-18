//! Rangierstraßen (REQ-IXL-028) am Bahnhof „Rangieren“ (`examples/rangieren.toml`).
//!
//! ```text
//!   GW ─ TW ─ A▶ TV ─ Ra5▷ TW1(W1) ┬─ T1 ─▶N1 ─ TE ─ GE
//!            ◁Ra4                  └─ T3 ─|PB3   (◁Ra3 am Anfang von T3)
//! ```
//!
//! Ra5-PB3: Fahrweg TW1, T3 mit W1 rechts, ohne Durchrutschweg. Die Weichen melden anfangs
//! rechts.

mod common;

use common::{Harness, accepted, leak, rejected};
use railit_kernel::*;

const NETWORK: &str = include_str!("../../../examples/rangieren.toml");

fn ready() -> Harness {
    let net = railit_import::toml_format::parse(NETWORK).unwrap().topology;
    let plan = railit_planning::compile(&net, None).unwrap().plan;
    let mut h = Harness::with_plan(leak(&plan));
    h.step(&[]);
    h
}

/// Rangierstraßen haben keinen Durchrutschweg und sind als solche gekennzeichnet.
#[test]
fn shunting_routes_in_plan() {
    let h = ready();
    let view = h.plan().route(h.route("Ra5-PB3")).unwrap();
    assert!(view.is_shunting());
    assert_eq!(view.overlap_tvds().len(), 0);
    let train = h.plan().route(h.route("A-PB3")).unwrap();
    assert!(!train.is_shunting());
    assert!(view.conflicts_with(h.route("A-N1")));
}

/// Eine Rangierstraße darf in ein besetztes Zielgleis führen, eine Zugstraße nicht.
#[test]
fn shunting_into_occupied_track() {
    let mut h = ready();
    h.set_tvd("T3", Occupancy::Occupied);
    let t3 = h.tvd("T3");
    assert_eq!(
        rejected(&h.set_route("A-PB3")),
        Some(Rejection::TvdNotClear(t3))
    );
    h.clear_route("Ra5-PB3");
    assert_eq!(h.aspect("Ra5"), Aspect::Proceed);
    assert_eq!(h.last, Ok(()));
}

/// Nur das Zielgleis darf besetzt sein; ein besetzter Abschnitt davor verhindert die
/// Rangierstraße, ein gestörtes Zielgleis ebenso.
#[test]
fn only_the_target_may_be_occupied() {
    let mut h = ready();
    h.set_tvd("TW1", Occupancy::Occupied);
    let tw1 = h.tvd("TW1");
    assert_eq!(
        rejected(&h.set_route("Ra5-PB3")),
        Some(Rejection::TvdNotClear(tw1))
    );
    h.set_tvd("TW1", Occupancy::Clear);
    h.set_tvd("T3", Occupancy::Disturbed);
    let t3 = h.tvd("T3");
    h.run(1);
    assert_eq!(
        rejected(&h.set_route("Ra5-PB3")),
        Some(Rejection::TvdNotClear(t3))
    );
}

/// Ins besetzte Zielgleis: Die Rangierstraße löst erst auf, wenn die Fahrt den Abschnitt
/// davor geräumt hat – solange bleibt die Weiche verschlossen.
#[test]
fn release_waits_until_the_movement_has_left_the_points() {
    let mut h = ready();
    h.set_tvd("T3", Occupancy::Occupied);
    h.clear_route("Ra5-PB3");
    let r = h.route("Ra5-PB3");

    // Die Rangierfahrt fährt in TW1 ein (T3 ist weiter besetzt).
    h.set_tvd("TW1", Occupancy::Occupied);
    h.run(3);
    assert_eq!(h.phase("Ra5-PB3"), RoutePhase::Occupied);
    assert_eq!(h.point_state("W1").owner(), Some(r));
    assert_eq!(h.aspect("Ra5"), Aspect::Stop);

    // Erst wenn TW1 wieder frei ist, steht die Fahrt im Zielgleis.
    h.set_tvd("TW1", Occupancy::Clear);
    h.run(2);
    assert_eq!(h.phase("Ra5-PB3"), RoutePhase::Idle);
    assert_eq!(h.point_state("W1").owner(), None);
    assert_eq!(h.tvd_state("T3").owner(), None);
    assert_eq!(h.last, Ok(()));
}

/// Wird das Zielgleis gestört, fällt das Rangiersignal auf Halt.
#[test]
fn disturbed_target_drops_proceed() {
    let mut h = ready();
    h.set_tvd("T3", Occupancy::Occupied);
    h.clear_route("Ra5-PB3");
    h.set_tvd("T3", Occupancy::Disturbed);
    h.run(1);
    assert_eq!(h.aspect("Ra5"), Aspect::Stop);
    assert_eq!(h.phase("Ra5-PB3"), RoutePhase::Halted);
}

/// Rangierstraße ins freie Gleis bis zum Hauptsignal N1 und zugbewirkte Auflösung.
#[test]
fn shunting_into_clear_track() {
    let mut h = ready();
    h.clear_route("Ra5-N1");
    // Feindlich zur Zugstraße A-N1.
    assert_eq!(rejected(&h.set_route("A-N1")), Some(Rejection::Conflict));
    h.set_tvd("TW1", Occupancy::Occupied);
    h.run(1);
    h.set_tvd("T1", Occupancy::Occupied);
    h.run(1);
    assert_eq!(h.phase("Ra5-N1"), RoutePhase::Occupied);
    h.set_tvd("TW1", Occupancy::Clear);
    h.run(2);
    assert_eq!(h.phase("Ra5-N1"), RoutePhase::Idle);
    h.set_tvd("T1", Occupancy::Clear);
    h.run(1);
    assert!(accepted(&h.set_route("A-N1")));
}
