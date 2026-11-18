//! Sperren (REQ-IXL-027): Weichensperre, Signalsperre und Gleissperrung am Bahnhof
//! Beispielstadt.
//!
//! ```text
//!   GW ─ TW ─ A▶ TW1(W1) ┬─ T1 ─▶N1 ┬ TW2(W2) ◀F ─ TE(BÜ1) ─ GE
//!                        └─ T2 ─▶N2 ┘
//! ```
//!
//! A-N1: Fahrweg TW1, T1 mit W1 links; Durchrutschweg TW2, TE mit W2 links.
//! A-N2: Fahrweg TW1, T2 mit W1 rechts. Die Weichen melden anfangs rechts.

mod common;

use common::{Harness, accepted, rejected};
use railit_kernel::*;
use railit_types::PointPosition;

fn ready() -> Harness {
    let mut h = Harness::new();
    h.step(&[]);
    h
}

fn point(h: &Harness, name: &str) -> BlockTarget {
    BlockTarget::Point(h.point(name))
}

fn signal(h: &Harness, name: &str) -> BlockTarget {
    BlockTarget::Signal(h.signal(name))
}

fn track(h: &Harness, name: &str) -> BlockTarget {
    BlockTarget::Tvd(h.tvd(name))
}

fn moves(h: &Harness) -> usize {
    h.out
        .commands
        .iter()
        .filter(|c| matches!(c, Command::MovePoint { .. }))
        .count()
}

// ── Weichensperre ────────────────────────────────────────────────────────────────────────

/// Eine gesperrte Weiche wird weder einzeln noch durch eine Fahrstraße umgestellt.
#[test]
fn blocked_point_is_not_moved() {
    let mut h = ready();
    let w1 = h.point("W1");
    assert!(accepted(&h.step(&[Request::Block(point(&h, "W1"))])));
    assert!(h.point_state("W1").blocked());

    let events = h.step(&[Request::MovePoint(w1, PointPosition::Left)]);
    assert_eq!(rejected(&events), Some(Rejection::PointBlocked(w1)));
    assert_eq!(moves(&h), 0);

    // A-N1 verlangt W1 links: abgelehnt.
    let events = h.set_route("A-N1");
    assert_eq!(rejected(&events), Some(Rejection::PointBlocked(w1)));
    assert_eq!(h.phase("A-N1"), RoutePhase::Idle);
}

/// Über eine gesperrte Weiche in ihrer vorhandenen Lage ist eine Fahrstraße möglich.
#[test]
fn route_over_blocked_point_in_position() {
    let mut h = ready();
    h.step(&[Request::Block(point(&h, "W1"))]);
    h.clear_route("A-N2");
    assert_eq!(h.aspect("A"), Aspect::Proceed);
    assert_eq!(moves(&h), 0);
}

/// Eine Durchrutschweg-Weiche in der falschen Lage, die gesperrt ist, verhindert die
/// Fahrstraße.
#[test]
fn blocked_overlap_point_prevents_route() {
    let mut h = ready();
    h.set_point("W1", PointReport::Left);
    let w2 = h.point("W2");
    h.step(&[Request::Block(point(&h, "W2"))]);
    let events = h.set_route("A-N1");
    assert_eq!(rejected(&events), Some(Rejection::PointBlocked(w2)));
}

/// Entsperren ist ohne Hilfshandlung möglich; danach läuft die Weiche wieder um.
#[test]
fn unblock_point() {
    let mut h = ready();
    let w1 = h.point("W1");
    h.step(&[Request::Block(point(&h, "W1"))]);
    assert!(accepted(&h.step(&[Request::Unblock(point(&h, "W1"))])));
    assert!(!h.point_state("W1").blocked());
    let events = h.step(&[Request::MovePoint(w1, PointPosition::Left)]);
    assert!(accepted(&events));
    assert_eq!(moves(&h), 1);
}

/// Die Hilfshandlung „Weiche umstellen“ ist bei gesperrter Weiche nicht zulässig.
#[test]
fn force_point_respects_block() {
    let mut h = ready();
    h.set_tvd("TW1", Occupancy::Disturbed);
    h.step(&[Request::Block(point(&h, "W1"))]);
    let w1 = h.point("W1");
    let events = h.aux(AuxOperation::ForcePoint(w1, PointPosition::Left));
    assert_eq!(rejected(&events), Some(Rejection::AuxNotApplicable));
    assert_eq!(moves(&h), 0);
}

// ── Signalsperre ─────────────────────────────────────────────────────────────────────────

/// An einem gesperrten Signal beginnt keine Fahrstraße.
#[test]
fn blocked_signal_rejects_route() {
    let mut h = ready();
    let a = h.signal("A");
    h.step(&[Request::Block(signal(&h, "A"))]);
    let events = h.set_route("A-N2");
    assert_eq!(rejected(&events), Some(Rejection::SignalBlocked(a)));
}

/// Wird das Startsignal einer Fahrstraße mit Fahrt gesperrt, fällt es sofort auf Halt.
#[test]
fn blocking_signal_drops_proceed() {
    let mut h = ready();
    h.clear_route("A-N2");
    h.step(&[Request::Block(signal(&h, "A"))]);
    assert_eq!(h.aspect("A"), Aspect::Stop);
    assert_eq!(h.phase("A-N2"), RoutePhase::Halted);
    assert_eq!(h.last, Ok(()));

    // Auch das Ersatzsignal ist an einem gesperrten Signal nicht möglich.
    let r = h.route("A-N2");
    let events = h.aux(AuxOperation::Substitute(r));
    assert_eq!(rejected(&events), Some(Rejection::AuxNotApplicable));
    assert!(h.substitutes.is_empty());

    // Nach dem Entsperren nur Rücknahme (Halt bleibt Halt).
    h.step(&[Request::Unblock(signal(&h, "A"))]);
    h.run(3);
    assert_eq!(h.aspect("A"), Aspect::Stop);
}

// ── Gleissperrung ────────────────────────────────────────────────────────────────────────

/// Keine Fahrstraße führt mit Fahrweg oder Durchrutschweg durch einen gesperrten Abschnitt.
#[test]
fn track_closure_rejects_routes_through_it() {
    let mut h = ready();
    let t1 = h.tvd("T1");
    let te = h.tvd("TE");
    h.step(&[Request::Block(track(&h, "T1"))]);
    assert!(h.tvd_state("T1").blocked());
    assert_eq!(
        rejected(&h.set_route("A-N1")),
        Some(Rejection::TvdBlocked(t1))
    );
    // Durchrutschweg von A-N2 läuft durch TE.
    h.step(&[Request::Block(track(&h, "TE"))]);
    assert_eq!(
        rejected(&h.set_route("A-N2")),
        Some(Rejection::TvdBlocked(te))
    );
    h.step(&[Request::Unblock(track(&h, "T1"))]);
    assert!(
        h.tvd_state("T1").blocked(),
        "Gleissperrung nur per Hilfshandlung"
    );
    // Nach Aufheben der Sperrung von TE ist A-N2 möglich.
    h.aux(AuxOperation::UnblockTvd(te));
    h.clear_route("A-N2");
}

/// Weichen in einem gesperrten Abschnitt werden nicht gestellt.
#[test]
fn track_closure_blocks_points_inside() {
    let mut h = ready();
    let w1 = h.point("W1");
    let tw1 = h.tvd("TW1");
    h.step(&[Request::Block(track(&h, "TW1"))]);
    let events = h.step(&[Request::MovePoint(w1, PointPosition::Left)]);
    assert_eq!(rejected(&events), Some(Rejection::TvdBlocked(tw1)));
    assert_eq!(moves(&h), 0);
}

/// Wird ein Abschnitt einer Fahrstraße mit Fahrt gesperrt, fällt das Signal auf Halt.
#[test]
fn track_closure_drops_proceed() {
    let mut h = ready();
    h.clear_route("A-N2");
    h.step(&[Request::Block(track(&h, "T2"))]);
    assert_eq!(h.aspect("A"), Aspect::Stop);
    assert_eq!(h.phase("A-N2"), RoutePhase::Halted);
    assert_eq!(h.last, Ok(()));
}

/// Eine Gleissperrung hebt nur die zählpflichtige Hilfshandlung auf; die einfache
/// Anforderung wird mit Hinweis auf die Hilfshandlung abgelehnt.
#[test]
fn track_closure_is_lifted_by_aux_only() {
    let mut h = ready();
    let t1 = h.tvd("T1");
    h.step(&[Request::Block(track(&h, "T1"))]);
    let events = h.step(&[Request::Unblock(track(&h, "T1"))]);
    assert_eq!(rejected(&events), Some(Rejection::AuxRequired));

    let before = h.state.meta().aux_counter();
    let events = h.aux(AuxOperation::UnblockTvd(t1));
    assert!(
        events.iter().any(
            |e| matches!(e, Event::AuxExecuted { op: AuxOperation::UnblockTvd(t), .. } if *t == t1)
        ),
        "{events:?}"
    );
    assert_eq!(h.state.meta().aux_counter(), before + 1);
    assert!(!h.tvd_state("T1").blocked());
    h.clear_route("A-N1");

    // Nicht gesperrt: Hilfshandlung nicht zulässig.
    let events = h.aux(AuxOperation::UnblockTvd(t1));
    assert_eq!(rejected(&events), Some(Rejection::AuxNotApplicable));
}

/// Sperren unbekannter Elemente wird abgelehnt.
#[test]
fn unknown_elements() {
    let mut h = ready();
    let bad = BlockTarget::Tvd(railit_types::TvdId::new(999));
    assert_eq!(
        rejected(&h.step(&[Request::Block(bad)])),
        Some(Rejection::UnknownElement)
    );
    assert_eq!(
        rejected(&h.step(&[Request::Unblock(bad)])),
        Some(Rejection::UnknownElement)
    );
}

/// Sperren bleiben bei der Übernahme durch einen Bereitschaftsrechner bestehen.
#[test]
fn blocks_survive_take_over() {
    let mut h = ready();
    h.step(&[Request::Block(point(&h, "W1"))]);
    h.step(&[Request::Block(signal(&h, "N1"))]);
    h.step(&[Request::Block(track(&h, "T2"))]);
    h.take_over_after(500);
    assert_eq!(h.last, Ok(()));
    assert!(h.point_state("W1").blocked());
    assert!(h.signal_state("N1").blocked());
    assert!(h.tvd_state("T2").blocked());
}
