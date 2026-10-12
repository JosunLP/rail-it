//! Tests der Phase-2-Funktionen am Bahnhof Beispielstadt: Durchrutschweg, Flankenschutz,
//! Bahnübergänge, unerlaubte Besetzung, Hilfshandlungen und Ersatzsignal.
//!
//! ```text
//!   GW ─ TW ─ A▶ TW1(W1) ┬─ T1 ─▶N1 ┬ TW2(W2) ◀F ─ TE(BÜ1) ─ GE
//!                        └─ T2 ─▶N2 ┘
//! ```
//!
//! A-N1: Fahrweg TW1, T1; Durchrutschweg TW2, TE mit W2 links; Flankenschutz P2 (für TW1)
//! und N2 (für TW2). N1-GE: Fahrweg TW2, TE mit BÜ1.

mod common;

use common::{Harness, accepted, rejected};
use railit_kernel::*;
use railit_types::PointPosition;

fn ready() -> Harness {
    let mut h = Harness::new();
    h.step(&[]);
    h
}

fn alarms(events: &[Event]) -> Vec<Alarm> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Alarm(a) => Some(*a),
            _ => None,
        })
        .collect()
}

// ── Durchrutschweg ───────────────────────────────────────────────────────────────────────

/// REQ-IXL-012: Der Durchrutschweg wird verschlossen, seine Weichen gestellt.
#[test]
fn overlap_is_locked_and_points_are_set() {
    let mut h = ready();
    h.set_point("W2", PointReport::Right);
    let events = h.set_route("A-N1");
    assert!(accepted(&events));
    let a_n1 = h.route("A-N1");
    assert_eq!(h.tvd_state("TW2").overlap_owner(), Some(a_n1));
    assert_eq!(h.tvd_state("TE").overlap_owner(), Some(a_n1));
    assert!(h.out.commands.contains(&Command::MovePoint {
        point: h.point("W2"),
        position: PointPosition::Left
    }));
    let w2 = h.point_state("W2");
    assert_eq!(w2.owner(), None);
    assert_eq!(w2.holds(), 1);
    assert_eq!(w2.locked_position(), Some(PointPosition::Left));

    // Die Durchrutschweg-Weiche ist für Einzelstellung gesperrt.
    let w2 = h.point("W2");
    let events = h.step(&[Request::MovePoint(w2, PointPosition::Right)]);
    assert_eq!(rejected(&events), Some(Rejection::PointLocked(w2)));

    h.run(3);
    assert_eq!(h.aspect("A"), Aspect::Proceed);
}

/// REQ-IXL-012: Ein besetzter Durchrutschweg verhindert die Fahrt und lässt das Signal
/// fallen.
#[test]
fn occupied_overlap_blocks_proceed() {
    let mut h = ready();
    h.set_tvd("TE", Occupancy::Occupied);
    assert!(accepted(&h.set_route("A-N1")));
    h.run(3);
    assert_eq!(h.phase("A-N1"), RoutePhase::Locked);
    assert_eq!(h.aspect("A"), Aspect::Stop);

    h.set_tvd("TE", Occupancy::Clear);
    h.run(2);
    assert_eq!(h.aspect("A"), Aspect::Proceed);

    h.set_tvd("TW2", Occupancy::Disturbed);
    let events = h.step(&[]);
    assert!(alarms(&events).contains(&Alarm::SignalDropped(h.route("A-N1"))));
    assert_eq!(h.aspect("A"), Aspect::Stop);
}

/// REQ-IXL-012: Die anschließende Fahrstraße darf den Durchrutschweg als Fahrweg nutzen;
/// feindliche Fahrstraßen nicht.
#[test]
fn continuation_may_use_the_overlap() {
    let mut h = ready();
    assert!(accepted(&h.set_route("A-N1")));
    assert_eq!(rejected(&h.set_route("F-P1")), Some(Rejection::Conflict));
    assert!(accepted(&h.set_route("N1-GE")));
    let n1_ge = h.route("N1-GE");
    assert_eq!(h.tvd_state("TW2").owner(), Some(n1_ge));
    assert_eq!(h.tvd_state("TW2").overlap_owner(), Some(h.route("A-N1")));
    h.run(3);
    assert_eq!(h.aspect("A"), Aspect::Proceed);
    assert_eq!(h.aspect("N1"), Aspect::Proceed);
}

/// REQ-IXL-012: Der Durchrutschweg wird erst nach der Auflösezeit und nur ohne
/// unverschlossene Besetzung aufgelöst.
#[test]
fn overlap_release_waits_for_time_and_vacancy() {
    let mut h = ready();
    h.clear_route("A-N1");
    h.set_tvd("TW1", Occupancy::Occupied);
    h.step(&[]);
    h.set_tvd("T1", Occupancy::Occupied);
    h.step(&[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Arrived);

    // Zug überfährt N1 und gerät in den Durchrutschweg.
    h.set_tvd("TW2", Occupancy::Occupied);
    let events = h.step(&[]);
    assert!(alarms(&events).contains(&Alarm::UnauthorizedOccupancy(h.tvd("TW2"))));

    h.step_after(40_000, &[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Arrived);
    h.set_tvd("TW2", Occupancy::Clear);
    h.step(&[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Idle);
}

// ── Flankenschutz ────────────────────────────────────────────────────────────────────────

/// REQ-IXL-013: Schutzsignale werden in Halt gehalten; ein Signal, das Fahrt meldet,
/// verhindert die Fahrerlaubnis.
#[test]
fn flank_signals_are_held_at_stop() {
    let mut h = ready();
    h.set_signal("P2", SignalReport::Proceed);
    assert!(accepted(&h.set_route("A-N1")));
    assert_eq!(h.signal_state("P2").stop_holds(), 1);
    assert_eq!(h.signal_state("N2").stop_holds(), 1);
    h.run(3);
    assert_eq!(h.phase("A-N1"), RoutePhase::Locked);

    h.set_signal("P2", SignalReport::Stop);
    h.run(2);
    assert_eq!(h.aspect("A"), Aspect::Proceed);

    // Fahrstraßen ab einem Schutzsignal sind feindlich.
    assert_eq!(rejected(&h.set_route("P2-GW")), Some(Rejection::Conflict));
}

/// REQ-IXL-013: Flankenschutz wird mit dem Abschnitt aufgelöst, den er schützt.
#[test]
fn flank_protection_is_released_with_its_section() {
    let mut h = ready();
    h.clear_route("A-N1");
    h.set_tvd("TW1", Occupancy::Occupied);
    h.step(&[]);
    h.set_tvd("T2", Occupancy::Clear);
    h.set_tvd("T1", Occupancy::Occupied);
    h.step(&[]);
    // Fahrweg aufgelöst: P2 (schützt TW1) frei, N2 (schützt TW2 im Durchrutschweg) nicht.
    assert_eq!(h.signal_state("P2").stop_holds(), 0);
    assert_eq!(h.signal_state("N2").stop_holds(), 1);
    h.set_tvd("TW1", Occupancy::Clear);
    h.step_after(30_000, &[]);
    assert_eq!(h.signal_state("N2").stop_holds(), 0);
}

// ── Bahnübergänge ────────────────────────────────────────────────────────────────────────

/// REQ-IXL-016: Der Bahnübergang wird geschlossen; Fahrt erst, wenn er geschlossen meldet.
#[test]
fn level_crossing_must_be_closed_before_proceed() {
    let mut h = ready();
    let bue = h.lx("BÜ1");
    h.stuck_lxs.push(bue);
    assert!(accepted(&h.set_route("N1-GE")));
    assert!(h.out.commands.contains(&Command::SetLevelCrossing {
        lx: bue,
        close: true
    }));
    assert!(h.lx_state("BÜ1").closed_commanded());
    h.run(3);
    assert_eq!(h.phase("N1-GE"), RoutePhase::Locked);
    assert_eq!(h.aspect("N1"), Aspect::Stop);

    h.stuck_lxs.clear();
    h.set_lx("BÜ1", LxReport::Closed);
    h.run(2);
    assert_eq!(h.aspect("N1"), Aspect::Proceed);

    // Störung des Bahnübergangs: Signal fällt.
    h.set_lx("BÜ1", LxReport::Disturbed);
    h.step(&[]);
    assert_eq!(h.aspect("N1"), Aspect::Stop);
    assert_eq!(h.phase("N1-GE"), RoutePhase::Halted);
}

/// REQ-IXL-016: Der Bahnübergang öffnet erst, wenn der Zug seinen Abschnitt geräumt hat.
#[test]
fn level_crossing_opens_after_train_has_left() {
    let mut h = ready();
    h.clear_route("N1-GE");
    h.set_tvd("TW2", Occupancy::Occupied);
    h.step(&[]);
    h.set_tvd("TE", Occupancy::Occupied);
    h.step(&[]);
    assert_eq!(h.phase("N1-GE"), RoutePhase::Idle);
    assert_eq!(h.lx_state("BÜ1").holds(), 0);
    assert!(h.lx_state("BÜ1").closed_commanded());
    assert_eq!(h.lxs[h.lx("BÜ1").index().unwrap()], LxReport::Closed);

    h.set_tvd("TW2", Occupancy::Clear);
    h.set_tvd("TE", Occupancy::Clear);
    h.step(&[]);
    assert!(!h.lx_state("BÜ1").closed_commanded());
    assert_eq!(h.lxs[h.lx("BÜ1").index().unwrap()], LxReport::Open);
}

// ── Unerlaubte Besetzung ─────────────────────────────────────────────────────────────────

/// REQ-IXL-011: Besetzung ohne erlaubte Fahrt wird gemeldet; Einfahrabschnitte nicht.
#[test]
fn unauthorized_occupancy_is_alarmed() {
    let mut h = ready();
    h.set_tvd("TW", Occupancy::Occupied);
    let events = h.step(&[]);
    assert!(alarms(&events).is_empty(), "Einfahrabschnitt: {events:?}");

    h.set_tvd("T2", Occupancy::Occupied);
    let events = h.step(&[]);
    assert_eq!(
        alarms(&events),
        vec![Alarm::UnauthorizedOccupancy(h.tvd("T2"))]
    );

    // Gestört → besetzt ist kein neuer Übergang aus „frei“.
    h.set_tvd("T1", Occupancy::Disturbed);
    h.step(&[]);
    h.set_tvd("T1", Occupancy::Occupied);
    assert!(alarms(&h.step(&[])).is_empty());
}

/// REQ-IXL-011: Haltüberfahrung an einer festgelegten Fahrstraße.
#[test]
fn signal_passed_at_danger() {
    let mut h = ready();
    h.stuck_lxs.push(h.lx("BÜ1"));
    assert!(accepted(&h.set_route("N1-GE")));
    h.run(2);
    assert_eq!(h.phase("N1-GE"), RoutePhase::Locked);
    h.set_tvd("TW2", Occupancy::Occupied);
    let events = h.step(&[]);
    let n1_ge = h.route("N1-GE");
    let alarms = alarms(&events);
    assert!(alarms.contains(&Alarm::UnauthorizedOccupancy(h.tvd("TW2"))));
    assert!(alarms.contains(&Alarm::SignalPassedAtDanger(n1_ge)));
    assert_eq!(h.phase("N1-GE"), RoutePhase::Occupied);
    assert_eq!(h.last, Ok(()));
}

// ── Hilfshandlungen ──────────────────────────────────────────────────────────────────────

/// REQ-IXL-014: Zwei Bediener, Zählwerk, Frist.
#[test]
fn aux_needs_a_second_operator_in_time() {
    let mut h = ready();
    h.clear_route("A-N1");
    let op = AuxOperation::ReleaseRoute(h.route("A-N1"));
    let events = h.step(&[Request::AuxRequest {
        op,
        operator: OperatorId(1),
    }]);
    assert!(accepted(&events));
    let aux = events
        .iter()
        .find_map(|e| match e {
            Event::AuxPending { aux, .. } => Some(*aux),
            _ => None,
        })
        .unwrap();

    // Derselbe Bediener darf nicht bestätigen.
    let events = h.step(&[Request::AuxConfirm {
        aux,
        operator: OperatorId(1),
        op,
    }]);
    assert_eq!(rejected(&events), Some(Rejection::SameOperator));
    assert_eq!(h.phase("A-N1"), RoutePhase::Proceed);

    // Ein zweiter Bediener bestätigt: Fahrstraße sofort aufgelöst, Zählwerk erhöht.
    let events = h.step(&[Request::AuxConfirm {
        aux,
        operator: OperatorId(2),
        op,
    }]);
    assert!(accepted(&events));
    assert!(events.contains(&Event::AuxExecuted {
        aux,
        op,
        counter: 1
    }));
    assert_eq!(h.phase("A-N1"), RoutePhase::Idle);
    assert_eq!(h.state.meta().aux_counter(), 1);

    // Erneute Bestätigung: unbekannt.
    let events = h.step(&[Request::AuxConfirm {
        aux,
        operator: OperatorId(2),
        op,
    }]);
    assert_eq!(rejected(&events), Some(Rejection::AuxUnknown));

    // Nicht anwendbar: Fahrstraße ist nicht eingestellt.
    let events = h.step(&[Request::AuxRequest {
        op,
        operator: OperatorId(1),
    }]);
    assert_eq!(rejected(&events), Some(Rejection::AuxNotApplicable));

    // Verfall nach der Frist.
    h.clear_route("A-N1");
    let events = h.step(&[Request::AuxRequest {
        op,
        operator: OperatorId(1),
    }]);
    let aux = events
        .iter()
        .find_map(|e| match e {
            Event::AuxPending { aux, .. } => Some(*aux),
            _ => None,
        })
        .unwrap();
    let events = h.step_after(31_000, &[]);
    assert!(events.contains(&Event::AuxExpired(aux)));
    assert_eq!(h.state.pending_aux().count(), 0);
    assert_eq!(h.state.meta().aux_counter(), 1);
}

/// REQ-IXL-014: Fahrstraßenhilfsauflösung auch bei befahrener Fahrstraße.
#[test]
fn aux_release_of_occupied_route() {
    let mut h = ready();
    h.clear_route("A-N1");
    h.set_tvd("TW1", Occupancy::Occupied);
    h.step(&[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Occupied);
    let events = h.aux(AuxOperation::ReleaseRoute(h.route("A-N1")));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::AuxExecuted { .. }))
    );
    assert_eq!(h.phase("A-N1"), RoutePhase::Idle);
    assert!(h.state.points().iter().all(|p| !p.is_locked()));
    assert!(h.state.tvds().iter().all(|t| t.owner().is_none()));
    assert_eq!(h.last, Ok(()));
}

/// REQ-IXL-014: Achszählgrundstellung nur für gestörte, unverschlossene Abschnitte.
#[test]
fn aux_reset_tvd() {
    let mut h = ready();
    let t2 = h.tvd("T2");
    let events = h.aux(AuxOperation::ResetTvd(t2));
    assert_eq!(rejected(&events), Some(Rejection::AuxNotApplicable));

    h.set_tvd("T2", Occupancy::Disturbed);
    h.step(&[]);
    let events = h.aux(AuxOperation::ResetTvd(t2));
    assert!(
        h.out.commands.contains(&Command::ResetTvd(t2)),
        "{events:?}"
    );
    h.step(&[]);
    assert!(accepted(&h.set_route("A-N2")));
}

/// REQ-IXL-014: Weiche in gestörtem Abschnitt umstellen.
#[test]
fn aux_force_point() {
    let mut h = ready();
    h.set_tvd("TW1", Occupancy::Disturbed);
    h.step(&[]);
    let w1 = h.point("W1");
    let events = h.step(&[Request::MovePoint(w1, PointPosition::Left)]);
    assert_eq!(rejected(&events), Some(Rejection::PointNotOperable(w1)));

    h.aux(AuxOperation::ForcePoint(w1, PointPosition::Left));
    assert!(h.out.commands.contains(&Command::MovePoint {
        point: w1,
        position: PointPosition::Left
    }));
    assert_eq!(h.position("W1"), PointPosition::Left);
    assert_eq!(h.last, Ok(()));
}

// ── Ersatzsignal ─────────────────────────────────────────────────────────────────────────

/// REQ-IXL-015: Zs1 bei gestörter Gleisfreimeldung; erlischt bei Einfahrt.
#[test]
fn substitute_signal_lets_train_pass() {
    let mut h = ready();
    h.clear_route("A-N1");
    h.set_tvd("T1", Occupancy::Disturbed);
    h.step(&[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Halted);

    let a_n1 = h.route("A-N1");
    let events = h.aux(AuxOperation::Substitute(a_n1));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::AuxExecuted { .. }))
    );
    let a = h.signal("A");
    assert!(h.substitutes.contains(&a));
    assert!(h.signal_state("A").substitute());
    assert_eq!(h.aspect("A"), Aspect::Stop);

    // Zug fährt am Zs1 vorbei: kein Alarm, Zs1 aus.
    h.set_tvd("TW1", Occupancy::Occupied);
    let events = h.step(&[]);
    assert!(alarms(&events).is_empty(), "{events:?}");
    assert_eq!(h.phase("A-N1"), RoutePhase::Occupied);
    assert!(!h.substitutes.contains(&a));
    assert_eq!(h.last, Ok(()));
}

/// REQ-IXL-015: Zs1 erlischt nach seiner Höchstdauer und wenn eine Weiche die Lage
/// verliert.
#[test]
fn substitute_signal_times_out_and_drops() {
    let mut h = ready();
    h.clear_route("A-N1");
    h.set_tvd("T1", Occupancy::Disturbed);
    h.step(&[]);
    let a_n1 = h.route("A-N1");
    h.aux(AuxOperation::Substitute(a_n1));
    let a = h.signal("A");
    assert!(h.substitutes.contains(&a));
    h.step_after(90_000, &[]);
    assert!(!h.substitutes.contains(&a));

    h.aux(AuxOperation::Substitute(a_n1));
    assert!(h.substitutes.contains(&a));
    h.set_point("W1", PointReport::NoEndPosition);
    h.step(&[]);
    assert!(!h.substitutes.contains(&a));
    assert_eq!(h.last, Ok(()));

    // Nicht anwendbar, solange die Weiche nicht in Lage ist.
    let events = h.aux(AuxOperation::Substitute(a_n1));
    assert_eq!(rejected(&events), Some(Rejection::AuxNotApplicable));
}

/// REQ-IXL-006: Rücknahme bei leuchtendem Zs1 unterliegt dem Zeitverschluss.
#[test]
fn cancel_with_substitute_uses_time_lock() {
    let mut h = ready();
    h.clear_route("A-N1");
    h.set_tvd("T1", Occupancy::Disturbed);
    h.step(&[]);
    h.aux(AuxOperation::Substitute(h.route("A-N1")));
    assert!(accepted(&h.cancel_route("A-N1")));
    assert_eq!(h.phase("A-N1"), RoutePhase::Cancelling);
    assert!(h.substitutes.is_empty());
}

/// REQ-IXL-022: Der zweite Bediener nennt eine andere Hilfshandlung – nichts wird ausgeführt,
/// die angeforderte verfällt.
#[test]
fn confirmation_must_restate_the_operation() {
    let mut h = Harness::new();
    h.set_route("A-N1");
    h.run(30);
    assert_eq!(h.phase("A-N1"), RoutePhase::Proceed);
    let op = AuxOperation::ReleaseRoute(h.route("A-N1"));
    let events = h.step(&[Request::AuxRequest {
        op,
        operator: OperatorId(1),
    }]);
    let aux = events
        .iter()
        .find_map(|e| match e {
            Event::AuxPending { aux, .. } => Some(*aux),
            _ => None,
        })
        .unwrap();
    let other = AuxOperation::ReleaseRoute(h.route("A-N2"));
    let events = h.step(&[Request::AuxConfirm {
        aux,
        operator: OperatorId(2),
        op: other,
    }]);
    assert_eq!(rejected(&events), Some(Rejection::AuxMismatch));
    assert!(events.contains(&Event::AuxExpired(aux)));
    assert_eq!(h.phase("A-N1"), RoutePhase::Proceed, "nichts ausgeführt");
    assert_eq!(h.state.meta().aux_counter(), 0);
    assert_eq!(h.state.pending_aux().count(), 0);
    // Auch die richtige Bestätigung kommt danach zu spät.
    let events = h.step(&[Request::AuxConfirm {
        aux,
        operator: OperatorId(2),
        op,
    }]);
    assert_eq!(rejected(&events), Some(Rejection::AuxUnknown));
}
