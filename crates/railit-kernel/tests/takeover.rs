//! Übernahme des Betriebs durch einen Bereitschaftsrechner (REQ-IXL-025): Der mitgelesene
//! Zustand wird restriktiv angeglichen – Fahrt fällt auf Halt, Verschlüsse bleiben.

mod common;

use common::{Harness, accepted, rejected};
use railit_kernel::*;

/// Weichen, Abschnitte und Bahnübergänge mit ihren Verschlüssen, um sie vor und nach der
/// Übernahme zu vergleichen.
fn locks(h: &Harness) -> (Vec<PointState>, Vec<TvdState>, Vec<LxState>) {
    (
        h.state.points().to_vec(),
        h.state.tvds().to_vec(),
        h.state.level_crossings().to_vec(),
    )
}

#[test]
fn proceed_falls_to_halt_and_locks_remain() {
    let mut h = Harness::new();
    h.step(&[]);
    h.clear_route("A-N1");
    assert_eq!(h.aspect("A"), Aspect::Proceed);
    let before = locks(&h);

    let events = h.take_over_after(300);
    assert_eq!(h.last, Ok(()));
    assert_eq!(h.phase("A-N1"), RoutePhase::Halted);
    assert_eq!(h.aspect("A"), Aspect::Stop);
    assert!(events.contains(&Event::RoutePhase(h.route("A-N1"), RoutePhase::Halted)));
    assert_eq!(locks(&h), before, "Verschlüsse unverändert");
    // Das Stellbild geht vollständig an das Feld: jedes Signal auf Halt, Ersatzsignale aus.
    let stops = h
        .out
        .commands
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::SetSignal {
                    aspect: Aspect::Stop,
                    ..
                }
            )
        })
        .count();
    assert_eq!(stops, h.plan().signal_count());
    assert!(
        h.out
            .commands
            .iter()
            .any(|c| matches!(c, Command::SetSubstitute { on: false, .. }))
    );

    // Der übernehmende Kern setzt fort: Die Fahrstraße bleibt verschlossen und auf Halt,
    // feindliche Fahrstraßen bleiben abgewiesen; Rücknahme mit Zeitverschluss.
    h.run(5);
    assert_eq!(h.last, Ok(()));
    assert_eq!(h.phase("A-N1"), RoutePhase::Halted);
    assert_eq!(h.aspect("A"), Aspect::Stop);
    assert_eq!(rejected(&h.set_route("A-N2")), Some(Rejection::Conflict));
    assert!(accepted(&h.cancel_route("A-N1")));
    assert_eq!(h.phase("A-N1"), RoutePhase::Cancelling);
    h.step_after(60_000, &[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Idle);
    assert!(accepted(&h.set_route("A-N2")));
    h.run(3);
    assert_eq!(h.phase("A-N2"), RoutePhase::Proceed);
}

#[test]
fn locked_route_and_train_in_route_continue() {
    let mut h = Harness::new();
    h.step(&[]);
    // Ein Zug fährt in A-N1 ein; die Fahrstraße löst hinter ihm auf.
    h.clear_route("A-N1");
    h.set_tvd("TW1", Occupancy::Occupied);
    h.step(&[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Occupied);

    h.take_over_after(100);
    assert_eq!(h.last, Ok(()));
    assert_eq!(h.phase("A-N1"), RoutePhase::Occupied);
    assert_eq!(h.point_state("W1").owner(), Some(h.route("A-N1")));

    // Der Zug fährt weiter, der übernehmende Kern löst hinter ihm auf.
    h.set_tvd("T1", Occupancy::Occupied);
    h.step(&[]);
    h.set_tvd("TW1", Occupancy::Clear);
    h.step(&[]);
    assert_eq!(h.point_state("W1").owner(), None);
    assert_eq!(h.last, Ok(()));
}

#[test]
fn pending_aux_expires_and_substitute_goes_out() {
    let mut h = Harness::new();
    h.step(&[]);
    h.clear_route("A-N1");
    let r = h.route("A-N1");
    let op = AuxOperation::ReleaseRoute(r);
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
        .expect("Hilfshandlung offen");

    let events = h.take_over_after(100);
    assert!(events.contains(&Event::AuxExpired(aux)));
    assert_eq!(h.state.pending_aux().count(), 0);
    // Eine Bestätigung nach der Übernahme greift ins Leere.
    let events = h.step(&[Request::AuxConfirm {
        aux,
        operator: OperatorId(2),
        op,
    }]);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::AuxExecuted { .. }))
    );
    assert_eq!(h.phase("A-N1"), RoutePhase::Halted);

    // Ersatzsignal: nach der Übernahme aus, bis es neu angefordert wird.
    h.set_tvd("T1", Occupancy::Disturbed);
    h.step(&[]);
    h.aux(AuxOperation::Substitute(r));
    assert!(h.signal_state("A").substitute());
    h.take_over_after(100);
    assert_eq!(h.last, Ok(()));
    assert!(!h.signal_state("A").substitute());
    assert!(!h.substitutes.contains(&h.signal("A")));
    assert_eq!(
        h.state.routes()[r.index().unwrap()].substitute_since(),
        None
    );
    h.run(3);
    assert!(!h.signal_state("A").substitute());
    assert_eq!(h.last, Ok(()));
}

#[test]
fn safe_state_is_kept() {
    let mut h = Harness::new();
    h.step(&[]);
    h.clear_route("A-N1");
    // Zustand verfälschen: Weiche ohne Eigentümer, obwohl die Fahrstraße steht.
    let w1 = u16::try_from(h.point("W1").index().unwrap()).unwrap();
    h.state
        .view()
        .apply(StatePart::Point(w1, PointState::default()))
        .unwrap();
    h.take_over_after(100);
    assert!(
        h.last.is_err(),
        "verfälschter Zustand wird nicht übernommen"
    );
    assert!(h.state.meta().safe_state().is_some());
    assert!(h.out.commands.iter().all(|c| !matches!(
        c,
        Command::SetSignal {
            aspect: Aspect::Proceed,
            ..
        }
    )));
    // Bleibt im sicheren Zustand.
    h.take_over_after(100);
    assert!(h.last.is_err());
}
