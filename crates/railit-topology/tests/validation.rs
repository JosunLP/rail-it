//! Validierungsregeln des Netzmodells, je Anforderung (siehe `docs/requirements/topologie.md`).

use proptest::prelude::*;
use railit_topology::*;

fn port(node: NodeRef, port: Port) -> PortRef {
    PortRef::new(node, port)
}

/// Bahnhof mit zwei Gleisen zwischen zwei Weichen:
///
/// ```text
///  GW ─ west ─ W1 ┬─ 1 ─┬ W2 ─ east ─ GE
///                 └─ 2 ─┘
/// ```
fn station() -> TopologyBuilder {
    let mut b = TopologyBuilder::new("Beispielstadt");
    let m = |m| Length::from_m(m).unwrap();
    let gw = NodeRef::Terminal(b.add_terminal("GW", TerminalKind::Boundary).unwrap());
    let ge = NodeRef::Terminal(b.add_terminal("GE", TerminalKind::Boundary).unwrap());
    let west = b.add_track("west", m(1000)).unwrap();
    let t1 = b.add_track("1", m(600)).unwrap();
    let t2 = b.add_track("2", m(600)).unwrap();
    let east = b.add_track("east", m(1000)).unwrap();
    let w1 = b.add_point("W1").unwrap();
    let w2 = b.add_point("W2").unwrap();
    let (nw, n1, n2, ne) = (
        NodeRef::Track(west),
        NodeRef::Track(t1),
        NodeRef::Track(t2),
        NodeRef::Track(east),
    );
    let (pw1, pw2) = (NodeRef::Point(w1), NodeRef::Point(w2));

    for (a, b_) in [
        (port(gw, Port::End), port(nw, Port::A)),
        (port(nw, Port::B), port(pw1, Port::Tip)),
        (port(pw1, Port::Left), port(n1, Port::A)),
        (port(pw1, Port::Right), port(n2, Port::A)),
        (port(n1, Port::B), port(pw2, Port::Left)),
        (port(n2, Port::B), port(pw2, Port::Right)),
        (port(pw2, Port::Tip), port(ne, Port::A)),
        (port(ne, Port::B), port(ge, Port::End)),
    ] {
        b.connect(a, b_).unwrap();
    }

    b.add_signal("A", west, m(950), Direction::AToB, SignalKind::Main)
        .unwrap();
    b.add_signal("N1", t1, m(580), Direction::AToB, SignalKind::Main)
        .unwrap();
    b.add_signal("N2", t2, m(580), Direction::AToB, SignalKind::Main)
        .unwrap();
    b.add_signal("F", east, m(50), Direction::BToA, SignalKind::Main)
        .unwrap();
    b.add_signal("P1", t1, m(20), Direction::BToA, SignalKind::Main)
        .unwrap();
    b.add_signal("P2", t2, m(20), Direction::BToA, SignalKind::Main)
        .unwrap();

    b.add_tvd("TW", &[TvdMember::Track(west)]).unwrap();
    b.add_tvd("TW1", &[TvdMember::Point(w1)]).unwrap();
    b.add_tvd("T1", &[TvdMember::Track(t1)]).unwrap();
    b.add_tvd("T2", &[TvdMember::Track(t2)]).unwrap();
    b.add_tvd("TW2", &[TvdMember::Point(w2)]).unwrap();
    b.add_tvd("TE", &[TvdMember::Track(east)]).unwrap();
    b
}

#[test]
fn valid_station_builds_without_warnings() {
    let v = station().build().unwrap();
    assert!(v.warnings.is_empty());
    let net = v.topology;
    assert_eq!(net.name(), "Beispielstadt");
    assert_eq!(net.tracks().len(), 4);
    assert_eq!(net.points().len(), 2);
    assert_eq!(net.terminals().len(), 2);
    assert_eq!(net.signals().len(), 6);
    assert_eq!(net.tvds().len(), 6);
    assert_eq!(net.components(), 1);

    let Some(ElementRef::Track(t1)) = net.lookup("1") else {
        panic!("Gleis 1 fehlt")
    };
    let tvd = net.tvd_of(TvdMember::Track(t1)).unwrap();
    assert_eq!(net.tvd(tvd).unwrap().name, "T1");
    let on_1: Vec<_> = net.signals_on(t1).map(|(_, s)| s.name.as_str()).collect();
    assert_eq!(on_1, ["N1", "P1"]);
}

/// REQ-TOP-001
#[test]
fn duplicate_names_are_rejected_across_kinds() {
    let mut b = station();
    assert_eq!(
        b.add_point("1"),
        Err(TopologyError::DuplicateName { name: "1".into() })
    );
    let err = b.add_tvd("W1", &[]).unwrap_err();
    assert_eq!(err.requirement(), "REQ-TOP-001");
}

/// REQ-TOP-002
#[test]
fn open_and_double_connected_ports_are_rejected() {
    let mut b = TopologyBuilder::new("offen");
    let t = NodeRef::Track(b.add_track("1", Length::from_mm(1)).unwrap());
    let e = NodeRef::Terminal(b.add_terminal("E", TerminalKind::BufferStop).unwrap());
    let e2 = NodeRef::Terminal(b.add_terminal("E2", TerminalKind::BufferStop).unwrap());
    b.connect(port(t, Port::A), port(e, Port::End)).unwrap();
    assert_eq!(
        b.connect(port(e2, Port::End), port(t, Port::A)),
        Err(TopologyError::PortAlreadyConnected {
            element: "1".into(),
            port: Port::A
        })
    );
    let Some(ElementRef::Track(tid)) = b.lookup("1") else {
        unreachable!()
    };
    b.add_tvd("T", &[TvdMember::Track(tid)]).unwrap();

    let report = b.build().unwrap_err();
    assert_eq!(
        report.errors,
        [
            TopologyError::OpenPort {
                element: "1".into(),
                port: Port::B
            },
            TopologyError::OpenPort {
                element: "E2".into(),
                port: Port::End
            },
        ]
    );
    assert!(
        report
            .errors
            .iter()
            .all(|e| e.requirement() == "REQ-TOP-002")
    );
    // Zwei Teilnetze: {1, E} und {E2}.
    assert_eq!(report.warnings, [Warning::Disconnected { components: 2 }]);
}

/// REQ-TOP-003
#[test]
fn invalid_ports_and_self_connections_are_rejected() {
    let mut b = TopologyBuilder::new("ports");
    let t = NodeRef::Track(b.add_track("1", Length::from_mm(1)).unwrap());
    let w = NodeRef::Point(b.add_point("W").unwrap());
    assert_eq!(
        b.connect(port(t, Port::Tip), port(w, Port::Tip)),
        Err(TopologyError::InvalidPort {
            element: "1".into(),
            port: Port::Tip
        })
    );
    assert_eq!(
        b.connect(port(w, Port::Left), port(w, Port::Left)),
        Err(TopologyError::SelfConnection {
            element: "W".into(),
            port: Port::Left
        })
    );
    let foreign = NodeRef::Track(TrackId::new(99));
    assert_eq!(
        b.connect(port(foreign, Port::A), port(t, Port::A)),
        Err(TopologyError::UnknownId {
            kind: "Gleis",
            raw: 99
        })
    );
    // Fehlgeschlagene Aufrufe verändern nichts: Port 1.a ist weiterhin frei.
    b.connect(port(t, Port::A), port(w, Port::Tip)).unwrap();
}

/// REQ-TOP-004
#[test]
fn zero_length_tracks_are_rejected() {
    let mut b = TopologyBuilder::new("null");
    assert_eq!(
        b.add_track("1", Length::ZERO),
        Err(TopologyError::ZeroLength { track: "1".into() })
    );
    assert_eq!(b.lookup("1"), None);
}

/// REQ-TOP-005
#[test]
fn signals_outside_their_track_are_rejected() {
    let mut b = TopologyBuilder::new("signal");
    let t = b.add_track("1", Length::from_m(100).unwrap()).unwrap();
    b.add_signal("S0", t, Length::ZERO, Direction::AToB, SignalKind::Main)
        .unwrap();
    b.add_signal(
        "S1",
        t,
        Length::from_m(100).unwrap(),
        Direction::AToB,
        SignalKind::Main,
    )
    .unwrap();
    let err = b
        .add_signal(
            "S2",
            t,
            Length::from_mm(100_001),
            Direction::AToB,
            SignalKind::Main,
        )
        .unwrap_err();
    assert_eq!(err.requirement(), "REQ-TOP-005");
    assert_eq!(b.lookup("S2"), None);
}

/// REQ-TOP-006
#[test]
fn tvd_coverage_must_be_complete_and_unique() {
    let mut b = TopologyBuilder::new("tvd");
    let t1 = b.add_track("1", Length::from_mm(1)).unwrap();
    let t2 = b.add_track("2", Length::from_mm(1)).unwrap();
    let ends: Vec<_> = (0..2)
        .map(|i| {
            NodeRef::Terminal(
                b.add_terminal(format!("E{i}"), TerminalKind::Boundary)
                    .unwrap(),
            )
        })
        .collect();
    let (n1, n2) = (NodeRef::Track(t1), NodeRef::Track(t2));
    b.connect(port(ends[0], Port::End), port(n1, Port::A))
        .unwrap();
    b.connect(port(n1, Port::B), port(n2, Port::A)).unwrap();
    b.connect(port(n2, Port::B), port(ends[1], Port::End))
        .unwrap();

    assert_eq!(
        b.add_tvd("leer", &[]),
        Err(TopologyError::EmptyTvd { tvd: "leer".into() })
    );
    b.add_tvd("TA", &[TvdMember::Track(t1)]).unwrap();
    b.add_tvd("TB", &[TvdMember::Track(t1), TvdMember::Track(t1)])
        .unwrap();

    let report = b.build().unwrap_err();
    assert_eq!(
        report.errors,
        [
            TopologyError::InMultipleTvds {
                element: "1".into(),
                tvds: vec!["TA".into(), "TB".into()]
            },
            TopologyError::NotInTvd {
                element: "2".into()
            },
        ]
    );
    assert!(report.to_string().contains("Fehler [REQ-TOP-006]"));
}

/// REQ-TOP-007
#[test]
fn points_can_only_be_traversed_tip_to_branch() {
    let net = station().build().unwrap().topology;
    let Some(ElementRef::Point(w1)) = net.lookup("W1") else {
        unreachable!()
    };
    let p = NodeRef::Point(w1);

    let from_tip: Vec<_> = net.exits(port(p, Port::Tip)).collect();
    assert_eq!(
        from_tip,
        [
            Exit {
                port: port(p, Port::Left),
                requires: Some((w1, PointPosition::Left))
            },
            Exit {
                port: port(p, Port::Right),
                requires: Some((w1, PointPosition::Right))
            },
        ]
    );
    let from_left: Vec<_> = net.exits(port(p, Port::Left)).collect();
    assert_eq!(
        from_left,
        [Exit {
            port: port(p, Port::Tip),
            requires: Some((w1, PointPosition::Left))
        }]
    );
    assert_eq!(net.exits(port(p, Port::A)).count(), 0);
}

#[test]
fn walking_the_station_from_west_reaches_both_platform_tracks() {
    let net = station().build().unwrap().topology;
    let Some(ElementRef::Terminal(gw)) = net.lookup("GW") else {
        unreachable!()
    };

    // Von der Netzgrenze West bis zum Ende jedes Bahnhofsgleises laufen.
    let mut reached = Vec::new();
    let mut frontier = vec![net.linked(port(NodeRef::Terminal(gw), Port::End)).unwrap()];
    while let Some(entry) = frontier.pop() {
        for exit in net.exits(entry) {
            if let NodeRef::Track(t) = exit.port.node {
                reached.push(net.track(t).unwrap().name.clone());
            }
            if let Some(next) = net.linked(exit.port) {
                frontier.push(next);
            }
        }
    }
    reached.sort();
    // "east" wird über Gleis 1 und über Gleis 2 erreicht.
    assert_eq!(reached, ["1", "2", "east", "east", "west"]);
}

proptest! {
    /// Jede beliebig lange, korrekt abgeschlossene Gleiskette ist gültig und zusammenhängend.
    #[test]
    fn chains_of_tracks_are_valid(lengths in prop::collection::vec(1u32..1_000_000, 1..50)) {
        let mut b = TopologyBuilder::new("Kette");
        let start = b.add_terminal("S", TerminalKind::BufferStop).unwrap();
        let mut prev = port(NodeRef::Terminal(start), Port::End);
        for (i, len) in lengths.iter().enumerate() {
            let t = b.add_track(format!("t{i}"), Length::from_mm(*len)).unwrap();
            b.connect(prev, port(NodeRef::Track(t), Port::A)).unwrap();
            b.add_tvd(format!("T{i}"), &[TvdMember::Track(t)]).unwrap();
            prev = port(NodeRef::Track(t), Port::B);
        }
        let end = b.add_terminal("E", TerminalKind::BufferStop).unwrap();
        b.connect(prev, port(NodeRef::Terminal(end), Port::End)).unwrap();

        let v = b.build().unwrap();
        prop_assert!(v.warnings.is_empty());
        prop_assert_eq!(v.topology.tracks().len(), lengths.len());
        prop_assert_eq!(v.topology.components(), 1);
    }

    /// Ausgänge bleiben immer am selben Knoten und führen nie zurück zum Eingang.
    #[test]
    fn exits_stay_on_node(node_kind in 0u8..3, port_index in 0usize..6) {
        let net = station().build().unwrap().topology;
        let node = match node_kind {
            0 => NodeRef::Track(TrackId::new(1)),
            1 => NodeRef::Point(PointId::new(0)),
            _ => NodeRef::Terminal(TerminalId::new(0)),
        };
        let all = [Port::A, Port::B, Port::Tip, Port::Left, Port::Right, Port::End];
        let entry = port(node, all[port_index]);
        for exit in net.exits(entry) {
            prop_assert_eq!(exit.port.node, node);
            prop_assert_ne!(exit.port.port, entry.port);
            prop_assert!(!(matches!(entry.port, Port::Left | Port::Right)
                && matches!(exit.port.port, Port::Left | Port::Right)));
        }
    }
}
