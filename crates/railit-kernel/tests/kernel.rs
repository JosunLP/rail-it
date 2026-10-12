//! Funktionale Tests des Sicherheitskerns am Bahnhof Beispielstadt.
//!
//! ```text
//!   GW ─ TW ─ A▶ TW1(W1) ┬─ T1 ─▶N1 ┬ TW2(W2) ◀F ─ TE ─ GE
//!                        └─ T2 ─▶N2 ┘
//! ```

mod common;

use common::{Harness, accepted, example_plan, leak, rejected};
use railit_kernel::*;
use railit_plan::RouteId;
use railit_types::PointPosition;

/// REQ-IXL-008: Beim Start zeigen alle Signale Halt.
#[test]
fn first_cycle_commands_stop_everywhere() {
    let mut h = Harness::new();
    h.step(&[]);
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
    assert_eq!(h.last, Ok(()));
}

/// REQ-IXL-009: Ohne Meldungen ist nichts frei.
#[test]
fn nothing_is_clear_without_reports() {
    let mut h = Harness::new();
    let a_n1 = h.route("A-N1");
    h.out.clear();
    let mut out = VecOutputs::default();
    h.kernel
        .step(
            &mut h.state.view(),
            h.now,
            &[Input::Request {
                id: RequestId(1),
                request: Request::SetRoute(a_n1),
            }],
            &mut out,
        )
        .unwrap();
    assert!(matches!(
        rejected(&out.events),
        Some(Rejection::TvdNotClear(_))
    ));
}

/// Kompletter Ablauf: Einstellen, Festlegen, Fahrt, Befahren mit Teilauflösung, Auflösung.
#[test]
fn route_lifecycle_with_partial_release() {
    let mut h = Harness::new();
    h.step(&[]);
    assert_eq!(h.position("W1"), PointPosition::Right);

    let events = h.set_route("A-N1");
    assert!(accepted(&events));
    assert_eq!(h.phase("A-N1"), RoutePhase::Setting);
    assert!(h.out.commands.contains(&Command::MovePoint {
        point: h.point("W1"),
        position: PointPosition::Left
    }));
    assert_eq!(h.aspect("A"), Aspect::Stop);

    h.step(&[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Locked);
    h.step(&[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Proceed);
    assert_eq!(h.aspect("A"), Aspect::Proceed);

    // Zug fährt in TW1 ein: Signal fällt auf Halt, Fahrstraße wird befahren.
    h.set_tvd("TW", Occupancy::Occupied);
    h.step(&[]);
    h.set_tvd("TW1", Occupancy::Occupied);
    h.step(&[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Occupied);
    assert_eq!(h.aspect("A"), Aspect::Stop);

    // W1 bleibt verschlossen, solange der Zug darauf steht.
    let w1 = h.point("W1");
    let events = h.step(&[Request::MovePoint(w1, PointPosition::Right)]);
    assert_eq!(rejected(&events), Some(Rejection::PointLocked(w1)));

    // Zug räumt TW: TW1 bleibt verschlossen, solange der Zug darauf steht.
    h.set_tvd("TW", Occupancy::Clear);
    h.step(&[]);
    assert_eq!(h.tvd_state("TW1").owner(), Some(h.route("A-N1")));

    // Zug erreicht den Zielabschnitt T1: der ganze Fahrweg wird aufgelöst, der
    // Durchrutschweg (TW2, TE, W2) bleibt festgelegt.
    h.set_tvd("T1", Occupancy::Occupied);
    let events = h.step(&[]);
    let a_n1 = h.route("A-N1");
    assert!(events.contains(&Event::RoutePhase(a_n1, RoutePhase::Arrived)));
    assert!(h.state.tvds().iter().all(|t| t.owner().is_none()));
    assert_eq!(h.tvd_state("TW2").overlap_owner(), Some(a_n1));
    assert_eq!(h.tvd_state("TE").overlap_owner(), Some(a_n1));
    assert!(h.point_state("W2").is_locked());
    assert!(!h.point_state("W1").is_locked());
    assert_eq!(
        rejected(&h.cancel_route("A-N1")),
        Some(Rejection::TrainInRoute)
    );

    // Nach der Auflösezeit des Durchrutschwegs ist alles frei.
    h.set_tvd("TW1", Occupancy::Clear);
    let events = h.step_after(30_000, &[]);
    assert!(events.contains(&Event::RoutePhase(a_n1, RoutePhase::Idle)));
    assert!(h.state.tvds().iter().all(|t| t.overlap_owner().is_none()));
    assert!(h.state.points().iter().all(|p| !p.is_locked()));
    assert!(h.state.signals().iter().all(|s| s.stop_holds() == 0));

    // Der Zug steht in T1: eine neue Fahrstraße dorthin ist nicht möglich.
    let events = h.set_route("A-N1");
    assert_eq!(rejected(&events), Some(Rejection::TvdNotClear(h.tvd("T1"))));
}

/// REQ-IXL-002: Feindliche Fahrstraßen werden abgelehnt, verträgliche angenommen.
#[test]
fn conflicting_routes_are_rejected() {
    let mut h = Harness::new();
    h.step(&[]);
    h.clear_route("A-N1");
    assert_eq!(rejected(&h.set_route("A-N2")), Some(Rejection::Conflict));
    assert_eq!(rejected(&h.set_route("F-P1")), Some(Rejection::Conflict));
    assert_eq!(rejected(&h.set_route("P2-GW")), Some(Rejection::Conflict));
    assert_eq!(rejected(&h.set_route("A-N1")), Some(Rejection::RouteActive));
    // N2-GE kreuzt den Durchrutschweg von A-N1; N1-GE setzt A-N1 fort und ist verträglich.
    assert_eq!(rejected(&h.set_route("N2-GE")), Some(Rejection::Conflict));
    h.clear_route("N1-GE");
    assert_eq!(h.aspect("A"), Aspect::Proceed);
    assert_eq!(h.aspect("N1"), Aspect::Proceed);
}

/// REQ-IXL-003: Besetzte oder gestörte Abschnitte verhindern das Einstellen.
#[test]
fn occupied_or_disturbed_sections_block_routes() {
    let mut h = Harness::new();
    h.set_tvd("T1", Occupancy::Occupied);
    h.step(&[]);
    assert_eq!(
        rejected(&h.set_route("A-N1")),
        Some(Rejection::TvdNotClear(h.tvd("T1")))
    );
    h.set_tvd("T1", Occupancy::Clear);
    h.set_tvd("TW1", Occupancy::Disturbed);
    assert_eq!(
        rejected(&h.set_route("A-N1")),
        Some(Rejection::TvdNotClear(h.tvd("TW1")))
    );
}

/// REQ-IXL-005: Signalhaltfall bei Verlust der Weichenüberwachung.
#[test]
fn signal_drops_when_point_loses_supervision() {
    let mut h = Harness::new();
    h.step(&[]);
    h.clear_route("A-N1");
    h.set_point("W1", PointReport::NoEndPosition);
    let events = h.step(&[]);
    assert_eq!(h.aspect("A"), Aspect::Stop);
    assert_eq!(h.phase("A-N1"), RoutePhase::Halted);
    assert!(events.contains(&Event::Alarm(Alarm::SignalDropped(h.route("A-N1")))));

    // Auch nach Wiederkehr der Endlage kein automatischer Fahrtbegriff.
    h.set_point("W1", PointReport::Left);
    h.run(5);
    assert_eq!(h.aspect("A"), Aspect::Stop);
}

/// REQ-IXL-005: Signalhaltfall bei Besetzung eines Abschnitts vor dem Zug.
#[test]
fn signal_drops_when_section_ahead_is_occupied() {
    let mut h = Harness::new();
    h.step(&[]);
    h.clear_route("A-N1");
    h.set_tvd("T1", Occupancy::Occupied);
    h.step(&[]);
    assert_eq!(h.aspect("A"), Aspect::Stop);
    assert_eq!(h.phase("A-N1"), RoutePhase::Halted);
}

/// REQ-IXL-009: Bleiben Meldungen aus, gilt das Element als gestört.
#[test]
fn stale_reports_drop_the_signal() {
    let mut h = Harness::new();
    h.step(&[]);
    h.clear_route("A-N1");
    let t1 = h.tvd("T1");
    h.silent_tvds.push(t1);
    h.step_after(1_000, &[]);
    assert_eq!(
        h.aspect("A"),
        Aspect::Proceed,
        "Meldung noch nicht veraltet"
    );
    h.step_after(1_100, &[]);
    assert_eq!(h.aspect("A"), Aspect::Stop);
    assert_eq!(h.phase("A-N1"), RoutePhase::Halted);
}

/// REQ-IXL-006: Rücknahme vor und nach Fahrtstellung.
#[test]
fn cancelling_respects_time_lock() {
    let mut h = Harness::new();
    h.step(&[]);
    // Vor der Fahrtstellung: sofort aufgelöst.
    assert!(accepted(&h.set_route("A-N1")));
    assert!(accepted(&h.cancel_route("A-N1")));
    assert_eq!(h.phase("A-N1"), RoutePhase::Idle);

    // Nach der Fahrtstellung: Halt, Zeitverschluss 60 s.
    h.clear_route("A-N1");
    assert!(accepted(&h.cancel_route("A-N1")));
    assert_eq!(h.phase("A-N1"), RoutePhase::Cancelling);
    assert_eq!(h.aspect("A"), Aspect::Stop);
    assert_eq!(rejected(&h.set_route("A-N2")), Some(Rejection::Conflict));
    h.step_after(59_000, &[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Cancelling);
    h.step_after(1_000, &[]);
    assert_eq!(h.phase("A-N1"), RoutePhase::Idle);
    assert!(accepted(&h.set_route("A-N2")));
}

/// Fährt ein Zug trotz Rücknahme ein, bleibt alles verschlossen und es wird alarmiert.
#[test]
fn train_entering_cancelled_route_keeps_locks() {
    let mut h = Harness::new();
    h.step(&[]);
    h.clear_route("A-N1");
    h.cancel_route("A-N1");
    h.set_tvd("TW1", Occupancy::Occupied);
    let events = h.step(&[]);
    assert!(
        events.contains(&Event::Alarm(Alarm::TrainEnteredCancelledRoute(
            h.route("A-N1")
        )))
    );
    assert_eq!(h.phase("A-N1"), RoutePhase::Occupied);
    assert_eq!(
        rejected(&h.cancel_route("A-N1")),
        Some(Rejection::TrainInRoute)
    );
    assert_eq!(h.state.points()[0].owner(), Some(h.route("A-N1")));
}

/// Weichen, die ihre Lage nicht erreichen, brechen die Einstellung ab.
#[test]
fn setting_times_out_when_points_do_not_move() {
    let mut h = Harness::new();
    h.step(&[]);
    let w1 = h.point("W1");
    h.stuck_points.push(w1);
    assert!(accepted(&h.set_route("A-N1")));
    h.step_after(10_000, &[]);
    assert_eq!(
        h.phase("A-N1"),
        RoutePhase::Setting,
        "genau 10 s sind noch zulässig"
    );
    let events = h.step_after(100, &[]);
    assert!(events.contains(&Event::Alarm(Alarm::SettingTimeout(h.route("A-N1")))));
    assert_eq!(h.phase("A-N1"), RoutePhase::Idle);
    assert!(h.state.points().iter().all(|p| p.owner().is_none()));
}

/// REQ-IXL-004: Einzelweichen nur unverschlossen und in freiem Abschnitt.
#[test]
fn single_point_operation() {
    let mut h = Harness::new();
    h.step(&[]);
    let w1 = h.point("W1");
    assert!(accepted(
        &h.step(&[Request::MovePoint(w1, PointPosition::Left)])
    ));
    assert_eq!(h.position("W1"), PointPosition::Left);

    h.set_tvd("TW1", Occupancy::Occupied);
    let events = h.step(&[Request::MovePoint(w1, PointPosition::Right)]);
    assert_eq!(rejected(&events), Some(Rejection::PointNotOperable(w1)));
    h.set_tvd("TW1", Occupancy::Clear);

    h.clear_route("A-N2");
    let events = h.step(&[Request::MovePoint(w1, PointPosition::Left)]);
    assert_eq!(rejected(&events), Some(Rejection::PointLocked(w1)));
}

#[test]
fn unknown_elements_are_rejected_and_reported() {
    let mut h = Harness::new();
    h.step(&[]);
    let events = h.step(&[Request::SetRoute(RouteId::new(99))]);
    assert_eq!(rejected(&events), Some(Rejection::UnknownElement));
    let mut out = VecOutputs::default();
    h.kernel
        .step(
            &mut h.state.view(),
            h.now,
            &[Input::Tvd {
                tvd: railit_types::TvdId::new(99),
                occupancy: Occupancy::Clear,
            }],
            &mut out,
        )
        .unwrap();
    assert!(out.events.contains(&Event::Alarm(Alarm::UnknownElement)));
}

#[test]
fn defective_and_mismatching_signals_are_alarmed() {
    let mut h = Harness::new();
    h.step(&[]);
    h.set_signal("A", SignalReport::Defective);
    let events = h.step(&[]);
    assert!(events.contains(&Event::Alarm(Alarm::SignalDefective(h.signal("A")))));
    assert!(accepted(&h.set_route("A-N1")));
    h.run(5);
    assert_eq!(
        h.phase("A-N1"),
        RoutePhase::Locked,
        "defektes Signal zeigt nie Fahrt"
    );
    assert_eq!(h.aspect("A"), Aspect::Stop);

    // Signal N1 bleibt nach Haltbefehl auf Fahrt stehen.
    h.clear_route("N1-GE");
    let n1 = h.signal("N1");
    h.stuck_signals.push(n1);
    h.cancel_route("N1-GE");
    let events = h.step_after(2_100, &[]);
    assert!(events.contains(&Event::Alarm(Alarm::SignalMismatch(n1))));
}

/// REQ-IXL-008: Verletzte Invariante ⇒ sicherer Zustand.
///
/// Der Kern lässt keine feindlichen Fahrstraßen zu. Um das Sicherheitsnetz zu prüfen, wird
/// der Plan während des Betriebs gegen einen ausgetauscht, in dem zwei bereits eingestellte
/// Fahrstraßen plötzlich feindlich sind.
#[test]
fn invariant_violation_enters_safe_state() {
    let mut h = Harness::new();
    h.step(&[]);
    h.clear_route("A-N1");
    h.clear_route("N1-GE");

    let mut plan = example_plan();
    let (a, b) = (h.route("A-N1"), h.route("N1-GE"));
    plan.routes[a.index().unwrap()].conflicts.push(b);
    plan.routes[b.index().unwrap()].conflicts.push(a);
    h.kernel = Interlocking::new(leak(&plan), *h.kernel.rules());

    h.step(&[]);
    assert_eq!(h.last, Err(SafeStateReason::ConflictingRoutes(a, b)));
    assert_eq!(h.aspect("A"), Aspect::Stop);
    assert_eq!(h.aspect("N1"), Aspect::Stop);
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
    assert_eq!(
        stops,
        h.plan().signal_count(),
        "Beim Eintritt erhalten alle Signale Halt"
    );

    // Danach wird jede Anforderung abgelehnt.
    let events = h.set_route("F-P2");
    assert_eq!(rejected(&events), Some(Rejection::SafeState));
    assert_eq!(
        h.state.meta().safe_state(),
        Some(SafeStateReason::ConflictingRoutes(a, b))
    );
}

#[test]
fn state_buffers_must_match_plan() {
    let h = Harness::new();
    let plan = *h.plan();
    let mut points = vec![PointState::default(); 1];
    let mut tvds = vec![TvdState::default(); plan.tvd_count()];
    let mut signals = vec![SignalState::default(); plan.signal_count()];
    let mut routes = vec![RouteState::default(); plan.route_count()];
    let mut lxs = vec![LxState::default(); plan.lx_count()];
    let mut borders = vec![BorderState::default(); plan.border_count()];
    let mut aux = vec![None; AUX_SLOTS];
    let mut active = vec![0u64; plan.route_set_words()];
    let mut meta = Meta::default();
    let err = KernelState::new(
        &plan,
        Buffers {
            points: &mut points,
            tvds: &mut tvds,
            signals: &mut signals,
            routes: &mut routes,
            lxs: &mut lxs,
            borders: &mut borders,
            aux: &mut aux,
            active: &mut active,
            meta: &mut meta,
        },
    )
    .unwrap_err();
    assert_eq!(err, SafeStateReason::StateMismatch);
}

/// REQ-IXL-001: Gleiche Eingaben ⇒ gleiche Ausgaben und gleicher Zustand.
#[test]
fn kernel_is_deterministic() {
    let run = || {
        let mut h = Harness::new();
        let mut trace = Vec::new();
        h.step(&[]);
        h.clear_route("A-N1");
        h.set_tvd("TW1", Occupancy::Occupied);
        trace.push(h.step(&[]));
        h.set_tvd("T1", Occupancy::Occupied);
        trace.push(h.step(&[]));
        h.set_tvd("TW1", Occupancy::Clear);
        trace.push(h.step(&[]));
        trace.push(h.set_route("F-P2"));
        h.run(3);
        (trace, h.state.clone())
    };
    assert_eq!(run(), run());
}
