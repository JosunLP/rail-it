//! Sperren im Kern (REQ-IXL-027): Weichensperre, Gleissperrung, Signalsperre.
//!
//! ```text
//!   GW ─ TW ─ A▶ TW1(W1) ┬─ T1 ─▶N1 ┬ TW2(W2) ◀F ─ TE ─ GE
//!                        └─ T2 ─▶N2 ┘
//! ```

mod common;

use common::{Harness, accepted, rejected};
use railit_kernel::*;
use railit_types::PointPosition;

fn harness() -> Harness {
    let mut h = Harness::new();
    h.step(&[]);
    h
}

#[test]
fn blocked_point_is_never_moved() {
    let mut h = harness();
    assert_eq!(h.position("W1"), PointPosition::Right);
    let w1 = BlockTarget::Point(h.point("W1"));
    let events = h.step(&[Request::Block(w1)]);
    assert!(accepted(&events));
    assert!(events.contains(&Event::Blocked {
        target: w1,
        blocked: true
    }));
    assert!(h.state.blocked(w1));

    // Einzeln umstellen und Fahrstraße, die die andere Lage braucht: abgelehnt.
    let events = h.step(&[Request::MovePoint(h.point("W1"), PointPosition::Left)]);
    assert_eq!(rejected(&events), Some(Rejection::Blocked(w1)));
    let events = h.set_route("A-N1");
    assert_eq!(rejected(&events), Some(Rejection::Blocked(w1)));
    assert!(
        !h.out
            .commands
            .iter()
            .any(|c| matches!(c, Command::MovePoint { .. }))
    );

    // Über die Weiche in ihrer Lage ist die Fahrstraße zulässig.
    h.clear_route("A-N2");

    // Hilfshandlung „Weiche umstellen“ ist für eine gesperrte Weiche nicht anwendbar.
    h.cancel_route("A-N2");
    h.set_tvd("TW1", Occupancy::Disturbed);
    let events = h.aux(AuxOperation::ForcePoint(h.point("W1"), PointPosition::Left));
    assert_eq!(rejected(&events), Some(Rejection::AuxNotApplicable));

    // Entsperren ist eine einfache Bedienung.
    let events = h.step(&[Request::Unblock(w1)]);
    assert!(accepted(&events));
    assert!(!h.state.blocked(w1));
    assert_eq!(h.last, Ok(()));
}

#[test]
fn track_closure_drops_signal_and_needs_aux_to_lift() {
    let mut h = harness();
    h.clear_route("A-N1");
    let t1 = BlockTarget::Tvd(h.tvd("T1"));

    // Sperren mitten im Betrieb: Fahrt fällt auf Halt, nichts wird aufgelöst.
    let events = h.step(&[Request::Block(t1)]);
    assert!(accepted(&events));
    assert_eq!(h.phase("A-N1"), RoutePhase::Halted);
    assert_eq!(h.aspect("A"), Aspect::Stop);
    assert_eq!(h.tvd_state("T1").owner(), Some(h.route("A-N1")));

    // Kein Ersatzsignal über ein gesperrtes Gleis.
    let events = h.aux(AuxOperation::Substitute(h.route("A-N1")));
    assert_eq!(rejected(&events), Some(Rejection::AuxNotApplicable));

    // Auch nach einiger Zeit bleibt das Signal auf Halt.
    h.run(5);
    assert_eq!(h.phase("A-N1"), RoutePhase::Halted);
    assert_eq!(h.aspect("A"), Aspect::Stop);

    // Aufheben nur als Hilfshandlung.
    let events = h.step(&[Request::Unblock(t1)]);
    assert_eq!(rejected(&events), Some(Rejection::AuxRequired));
    let before = h.state.meta().aux_counter();
    let events = h.aux(AuxOperation::UnblockTvd(h.tvd("T1")));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::AuxExecuted { .. }))
    );
    assert_eq!(h.state.meta().aux_counter(), before + 1);
    assert!(!h.state.blocked(t1));

    // Die haltgefallene Fahrstraße bleibt auf Halt; neu einstellen nach Rücknahme.
    let events = h.aux(AuxOperation::UnblockTvd(h.tvd("T1")));
    assert_eq!(rejected(&events), Some(Rejection::AuxNotApplicable));
    assert_eq!(h.last, Ok(()));
}

#[test]
fn blocked_signal_is_no_start() {
    let mut h = harness();
    let a = BlockTarget::Signal(h.signal("A"));
    assert!(accepted(&h.step(&[Request::Block(a)])));
    let events = h.set_route("A-N2");
    assert_eq!(rejected(&events), Some(Rejection::Blocked(a)));
    // Blockieren ist idempotent und meldet nichts neu.
    let events = h.step(&[Request::Block(a)]);
    assert!(accepted(&events));
    assert!(!events.iter().any(|e| matches!(e, Event::Blocked { .. })));
    assert!(accepted(&h.step(&[Request::Unblock(a)])));
    h.clear_route("A-N2");
}

#[test]
fn blocks_survive_reset_and_take_over() {
    let mut h = harness();
    let t2 = BlockTarget::Tvd(h.tvd("T2"));
    h.step(&[Request::Block(t2)]);
    h.state.view().reset();
    assert!(h.state.blocked(t2), "Neustart gibt das Gleis nicht frei");
    h.step(&[]);
    h.take_over_after(100);
    assert_eq!(h.last, Ok(()));
    assert!(h.state.blocked(t2));
    h.run(3);
    let events = h.set_route("A-N2");
    assert_eq!(rejected(&events), Some(Rejection::Blocked(t2)));
}

#[test]
fn unknown_elements_are_rejected() {
    let mut h = harness();
    let bogus = BlockTarget::Tvd(railit_types::TvdId::new(999));
    assert_eq!(
        rejected(&h.step(&[Request::Block(bogus)])),
        Some(Rejection::UnknownElement)
    );
    assert_eq!(
        rejected(&h.step(&[Request::Unblock(bogus)])),
        Some(Rejection::UnknownElement)
    );
    assert_eq!(h.last, Ok(()));
}

#[test]
fn state_transfer_carries_blocks() {
    let mut h = harness();
    let w2 = BlockTarget::Point(h.point("W2"));
    h.step(&[Request::Block(w2)]);
    let mut copy = OwnedState::new(h.plan());
    {
        let src = h.state.clone();
        let mut src = src;
        let view = src.view();
        let mut dst = copy.view();
        for i in 0..view.part_count() {
            dst.apply(view.part(i).unwrap()).unwrap();
        }
    }
    assert_eq!(copy, h.state);
    assert!(copy.blocked(w2));
}
