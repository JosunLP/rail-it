//! Simulator: Weichen, Züge, Störungen.

#![allow(clippy::many_single_char_names)]

use railit_kernel::{Aspect, Command, Occupancy, PointReport, SignalReport};
use railit_sim::*;
use railit_topology::{
    Direction, Length, NodeRef, Port, PortRef, SignalKind, TerminalKind, TopologyBuilder, TvdMember,
};
use railit_types::{Duration, PointPosition, SignalId, Speed};

const EXAMPLE: &str = include_str!("../../../examples/beispielstadt.toml");

fn sim() -> Simulator {
    let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
    Simulator::new(net, SimConfig::default())
}

fn id<T>(
    net: &railit_topology::Topology,
    name: &str,
    f: impl Fn(railit_topology::ElementRef) -> Option<T>,
) -> T {
    f(net.lookup(name).unwrap()).unwrap()
}

fn signal(s: &Simulator, name: &str) -> SignalId {
    id(s.topology(), name, |e| match e {
        railit_topology::ElementRef::Signal(s) => Some(s),
        _ => None,
    })
}

fn run(s: &mut Simulator, seconds: u64) {
    for _ in 0..seconds * 10 {
        let now = s.now().checked_add(Duration::from_millis(100)).unwrap();
        s.advance(now);
    }
}

fn train(name: &str, signal: SignalId, ignore_signals: bool) -> TrainSpec {
    TrainSpec {
        name: name.into(),
        signal,
        length: Length::from_m(100).unwrap(),
        speed: Speed::from_kmh(72),
        ignore_signals,
    }
}

#[test]
fn points_need_time_to_move() {
    let mut s = sim();
    let w1 = s.topology().points().next().unwrap().0;
    s.apply(&[Command::MovePoint {
        point: w1,
        position: PointPosition::Left,
    }]);
    assert_eq!(s.point_report(w1), Some(PointReport::NoEndPosition));
    run(&mut s, 3);
    assert_eq!(s.point_report(w1), Some(PointReport::NoEndPosition));
    run(&mut s, 1);
    assert_eq!(s.point_report(w1), Some(PointReport::Left));
}

#[test]
fn trains_wait_at_stop_and_run_on_proceed() {
    let mut s = sim();
    let a = signal(&s, "A");
    s.add_train(train("T", a, false)).unwrap();
    run(&mut s, 5);
    assert_eq!(s.trains()[0].state, TrainState::Waiting);
    assert_eq!(
        s.take_events(),
        [SimEvent::TrainStopped { train: "T".into() }]
    );

    s.apply(&[Command::SetSignal {
        signal: a,
        aspect: Aspect::Proceed,
    }]);
    assert_eq!(s.signal_report(a), Some(SignalReport::Proceed));
    run(&mut s, 1);
    // W1 liegt rechts: der Zug fährt nach Gleis 2.
    let info = &s.trains()[0];
    assert_eq!(info.state, TrainState::Running);
    assert!(info.occupied.contains(&"TW1".to_string()), "{info:?}");
    run(&mut s, 10);
    assert!(s.trains()[0].occupied.contains(&"T2".to_string()));
}

#[test]
fn moving_point_derails_train() {
    let mut s = sim();
    let a = signal(&s, "A");
    let w1 = s.topology().points().next().unwrap().0;
    s.add_train(train("T", a, true)).unwrap();
    s.apply(&[Command::MovePoint {
        point: w1,
        position: PointPosition::Left,
    }]);
    run(&mut s, 1);
    assert_eq!(s.trains()[0].state, TrainState::Derailed);
    assert!(
        s.take_events()
            .iter()
            .any(|e| matches!(e, SimEvent::TrainDerailed { .. }))
    );
}

#[test]
fn trailing_a_point_disturbs_it() {
    let mut s = sim();
    // Zug ab N1 (Gleis 1 → W2 über den linken Strang); W2 liegt rechts.
    let n1 = signal(&s, "N1");
    let w2 = s.topology().points().nth(1).unwrap().0;
    s.add_train(train("T", n1, true)).unwrap();
    run(&mut s, 2);
    assert!(
        s.take_events()
            .iter()
            .any(|e| matches!(e, SimEvent::PointTrailed { .. }))
    );
    assert_eq!(s.point_report(w2), Some(PointReport::Disturbed));
}

#[test]
fn trains_stop_at_buffer_stops_and_leave_at_boundaries() {
    let mut b = TopologyBuilder::new("Stumpf");
    let m = |m| Length::from_m(m).unwrap();
    let start = NodeRef::Terminal(b.add_terminal("S", TerminalKind::Boundary).unwrap());
    let end = NodeRef::Terminal(b.add_terminal("E", TerminalKind::BufferStop).unwrap());
    let t1 = b.add_track("1", m(200)).unwrap();
    let t2 = b.add_track("2", m(300)).unwrap();
    b.connect(
        PortRef::new(start, Port::End),
        PortRef::new(NodeRef::Track(t1), Port::A),
    )
    .unwrap();
    b.connect(
        PortRef::new(NodeRef::Track(t1), Port::B),
        PortRef::new(NodeRef::Track(t2), Port::A),
    )
    .unwrap();
    b.connect(
        PortRef::new(NodeRef::Track(t2), Port::B),
        PortRef::new(end, Port::End),
    )
    .unwrap();
    let x = b
        .add_signal("X", t1, m(200), Direction::AToB, SignalKind::Main)
        .unwrap();
    let y = b
        .add_signal("Y", t1, Length::ZERO, Direction::BToA, SignalKind::Main)
        .unwrap();
    b.add_tvd("T1", &[TvdMember::Track(t1)]).unwrap();
    b.add_tvd("T2", &[TvdMember::Track(t2)]).unwrap();
    let net = b.build().unwrap().topology;
    let t2_tvd = net.tvd_of(TvdMember::Track(t2)).unwrap();

    let mut s = Simulator::new(net, SimConfig::default());
    s.apply(&[
        Command::SetSignal {
            signal: x,
            aspect: Aspect::Proceed,
        },
        Command::SetSignal {
            signal: y,
            aspect: Aspect::Proceed,
        },
    ]);
    s.add_train(train("ZUM_PRELLBOCK", x, false)).unwrap();
    s.add_train(train("RAUS", y, false)).unwrap();
    run(&mut s, 60);
    let trains = s.trains();
    assert_eq!(trains.len(), 1, "RAUS hat das Netz verlassen: {trains:?}");
    assert_eq!(trains[0].state, TrainState::Arrived);
    assert_eq!(s.occupancy(t2_tvd), Some(Occupancy::Occupied));
    let events = s.take_events();
    assert!(events.contains(&SimEvent::TrainLeft {
        train: "RAUS".into()
    }));
    assert!(events.contains(&SimEvent::TrainArrived {
        train: "ZUM_PRELLBOCK".into()
    }));
}

#[test]
fn faults_override_reports() {
    let mut s = sim();
    let a = signal(&s, "A");
    let (t1, _) = s.topology().tvds().nth(2).unwrap();
    s.inject(Fault::TvdDisturbed(t1));
    assert_eq!(s.occupancy(t1), Some(Occupancy::Disturbed));
    s.clear(Fault::TvdDisturbed(t1));
    assert_eq!(s.occupancy(t1), Some(Occupancy::Clear));
    s.inject(Fault::SignalDefective(a));
    s.apply(&[Command::SetSignal {
        signal: a,
        aspect: Aspect::Proceed,
    }]);
    assert_eq!(s.signal_report(a), Some(SignalReport::Defective));
    assert_eq!(s.add_train(train("A1", a, false)), Ok(()));
    assert_eq!(
        s.add_train(train("A1", a, false)),
        Err(SimError::DuplicateTrain("A1".into()))
    );
}
