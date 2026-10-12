//! Property-Tests: Unter beliebigen Folgen von Meldungen und Anforderungen gerät der Kern nie
//! in einen unsicheren Zustand (REQ-IXL-008).
//!
//! Geprüft wird von außen, unabhängig von den Invarianten im Kern:
//!
//! - Der Kern geht nie in den sicheren Zustand (die internen Invarianten halten).
//! - Jedes Signal mit Fahrtbegriff gehört zu einer Fahrstraße, deren Abschnitte laut Feld
//!   frei und deren Weichen laut Feld in Lage sind.
//! - Nie sind zwei feindliche Fahrstraßen gleichzeitig eingestellt.
//! - Keine Weiche wird umgestellt, während ihr Abschnitt besetzt ist (außer durch die
//!   bestätigte Hilfshandlung) oder eine Fahrstraße über sie festgelegt ist.
//! - Fahrt zeigt ein Signal nur bei freiem Durchrutschweg, Weichen in Lage (auch Flanken-
//!   schutz) und geschlossenen Bahnübergängen; das Ersatzsignal nur bei gesichertem Fahrweg.

mod common;

use common::Harness;
use proptest::prelude::*;
use railit_kernel::*;
use railit_plan::{PointId, Protection, RouteId, RouteView, TvdId};
use railit_types::PointPosition;

#[derive(Clone, Debug)]
enum Action {
    Tvd(usize, Occupancy),
    Point(usize, PointReport),
    Signal(usize, SignalReport),
    Set(usize),
    Cancel(usize),
    Move(usize, PointPosition),
    Lx(LxReport),
    Aux(AuxOperation, u32),
    Confirm(u32, u32),
    Wait(u64),
}

fn action() -> impl Strategy<Value = Action> {
    let occupancy = prop_oneof![
        6 => Just(Occupancy::Clear),
        3 => Just(Occupancy::Occupied),
        1 => Just(Occupancy::Disturbed),
    ];
    let point = prop_oneof![
        Just(PointReport::Left),
        Just(PointReport::Right),
        Just(PointReport::NoEndPosition),
        Just(PointReport::Disturbed),
    ];
    let signal = prop_oneof![
        8 => Just(SignalReport::Stop),
        1 => Just(SignalReport::Proceed),
        1 => Just(SignalReport::Defective),
    ];
    let pos = prop_oneof![Just(PointPosition::Left), Just(PointPosition::Right)];
    let lx = prop_oneof![
        Just(LxReport::Open),
        Just(LxReport::Closing),
        Just(LxReport::Closed),
        Just(LxReport::Disturbed),
    ];
    let route = (0u32..8).prop_map(RouteId::new);
    let aux = prop_oneof![
        route.clone().prop_map(AuxOperation::ReleaseRoute),
        route.prop_map(AuxOperation::Substitute),
        (0u32..6).prop_map(|t| AuxOperation::ResetTvd(TvdId::new(t))),
        (0u32..2, pos.clone()).prop_map(|(p, pos)| AuxOperation::ForcePoint(PointId::new(p), pos)),
    ];
    prop_oneof![
        1 => lx.prop_map(Action::Lx),
        1 => (aux, 1u32..3).prop_map(|(op, who)| Action::Aux(op, who)),
        1 => (0u32..8, 1u32..3).prop_map(|(id, who)| Action::Confirm(id, who)),
        3 => (0usize..6, occupancy).prop_map(|(i, o)| Action::Tvd(i, o)),
        1 => (0usize..2, point).prop_map(|(i, r)| Action::Point(i, r)),
        1 => (0usize..6, signal).prop_map(|(i, r)| Action::Signal(i, r)),
        4 => (0usize..8).prop_map(Action::Set),
        2 => (0usize..8).prop_map(Action::Cancel),
        1 => (0usize..2, pos).prop_map(|(i, p)| Action::Move(i, p)),
        2 => (0u64..70_000).prop_map(Action::Wait),
    ]
}

/// Ist die Fahrstraße laut Feld gesichert (Weichen, Schutzsignale, Bahnübergänge)?
fn secured_in_field(h: &Harness, r: &RouteView<'_>, field_points: &[PointReport]) -> bool {
    let in_pos = |p: PointId, pos| field_points[p.index().unwrap()].position() == Some(pos);
    r.points()
        .chain(r.overlap_points())
        .all(|(p, pos)| in_pos(p, pos))
        && r.flank().all(|f| match f.protection {
            Protection::Point(p, pos) => in_pos(p, pos),
            Protection::Signal(s) => {
                h.state.signals()[s.index().unwrap()].commanded() == Aspect::Stop
            }
        })
        && r.level_crossings()
            .all(|l| h.lxs_before[l.index().unwrap()] == LxReport::Closed)
}

fn check_outputs(h: &Harness, field_points: &[PointReport], field_tvds: &[Occupancy]) {
    let plan = *h.plan();
    let clear = |t: TvdId| field_tvds[t.index().unwrap()] == Occupancy::Clear;
    let forced: Vec<PointId> = h
        .out
        .events
        .iter()
        .filter_map(|e| match e {
            Event::AuxExecuted {
                op: AuxOperation::ForcePoint(p, _),
                ..
            } => Some(*p),
            _ => None,
        })
        .collect();

    for c in &h.out.commands {
        match *c {
            Command::SetSignal {
                signal,
                aspect: Aspect::Proceed,
            } => {
                let justified = plan.routes().any(|r| {
                    r.start() == signal
                        && h.state.routes()[r.id().index().unwrap()].phase() == RoutePhase::Proceed
                        && r.tvds().chain(r.overlap_tvds()).all(clear)
                        && secured_in_field(h, &r, field_points)
                });
                assert!(justified, "Fahrt an {signal} ohne Rechtfertigung");
            }
            Command::SetSubstitute { signal, on: true } => {
                let justified = plan.routes().any(|r| {
                    let state = h.state.routes()[r.id().index().unwrap()];
                    r.start() == signal
                        && matches!(state.phase(), RoutePhase::Locked | RoutePhase::Halted)
                        && state.substitute_since().is_some()
                        && secured_in_field(h, &r, field_points)
                });
                assert!(justified, "Ersatzsignal an {signal} ohne Rechtfertigung");
            }
            Command::MovePoint { point, .. } => {
                let tvd = plan.point_tvd(point).unwrap();
                if !forced.contains(&point) {
                    assert!(
                        clear(tvd),
                        "Weiche {point} in nicht freiem Abschnitt gestellt"
                    );
                }
                let owner = h.state.points()[point.index().unwrap()].owner();
                if let Some(r) = owner {
                    assert_eq!(
                        h.state.routes()[r.index().unwrap()].phase(),
                        RoutePhase::Setting,
                        "Weiche {point} unter festgelegter Fahrstraße gestellt"
                    );
                }
            }
            Command::SetLevelCrossing { lx, close: false } => {
                let needed = plan.routes().any(|r| {
                    h.state.routes()[r.id().index().unwrap()].phase() != RoutePhase::Idle
                        && r.level_crossings().any(|l| l == lx)
                        && h.state.level_crossings()[lx.index().unwrap()].holds() > 0
                });
                assert!(!needed, "Bahnübergang {lx} geöffnet, obwohl gebraucht");
                assert!(
                    clear(plan.lx_tvd(lx).unwrap()),
                    "Bahnübergang {lx} geöffnet, obwohl sein Abschnitt nicht frei ist"
                );
            }
            Command::SetSignal { .. }
            | Command::SetSubstitute { .. }
            | Command::SetLevelCrossing { .. }
            | Command::ResetTvd(_)
            | Command::Border { .. } => {}
        }
    }

    let active: Vec<RouteId> = (0..plan.route_count())
        .filter(|&i| h.state.routes()[i].phase() != RoutePhase::Idle)
        .map(|i| RouteId::from_index(i).unwrap())
        .collect();
    for &a in &active {
        for &b in &active {
            assert!(
                a == b || !plan.route(a).unwrap().conflicts_with(b),
                "{a} und {b} feindlich"
            );
        }
    }

    for (i, s) in h.state.signals().iter().enumerate() {
        if s.commanded() == Aspect::Proceed {
            let sig = railit_plan::SignalId::from_index(i).unwrap();
            assert!(
                plan.routes().any(|r| r.start() == sig
                    && h.state.routes()[r.id().index().unwrap()].phase() == RoutePhase::Proceed),
                "Signal {sig} zeigt Fahrt ohne Fahrstraße"
            );
        }
    }
}

proptest! {
    // Standard: 256 Fälle; mehr über die Umgebungsvariable PROPTEST_CASES.
    #[test]
    fn kernel_never_becomes_unsafe(actions in prop::collection::vec(action(), 1..200)) {
        let mut h = Harness::new();
        h.step(&[]);
        for a in actions {
            let mut requests = Vec::new();
            let mut dt = 100;
            match a {
                Action::Tvd(i, o) => h.tvds[i] = o,
                Action::Point(i, r) => h.points[i] = r,
                Action::Signal(i, r) => h.signals[i] = r,
                Action::Set(i) => requests.push(Request::SetRoute(RouteId::from_index(i).unwrap())),
                Action::Cancel(i) => requests.push(Request::CancelRoute(RouteId::from_index(i).unwrap())),
                Action::Move(i, p) => requests.push(Request::MovePoint(PointId::from_index(i).unwrap(), p)),
                Action::Lx(r) => h.lxs[0] = r,
                Action::Aux(op, who) => requests.push(Request::AuxRequest { op, operator: OperatorId(who) }),
                Action::Confirm(id, who) => requests.push(Request::AuxConfirm { aux: AuxId(id), operator: OperatorId(who) }),
                Action::Wait(ms) => dt = ms,
            }
            let (field_points, field_tvds) = (h.points.clone(), h.tvds.clone());
            h.lxs_before = h.lxs.clone();
            h.step_after(dt, &requests);
            prop_assert_eq!(h.last, Ok(()), "sicherer Zustand nach {:?}", a);
            check_outputs(&h, &field_points, &field_tvds);
        }
    }
}
