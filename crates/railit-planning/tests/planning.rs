//! Projektierung: Generator, Gegenprüfung, Kompilierung.

#![allow(clippy::many_single_char_names)]

use std::collections::BTreeSet;

use proptest::prelude::*;
use railit_plan::{PlanView, RouteEnd, SigningKey};
use railit_planning::{PlanningError, compile, generate, verify};
use railit_topology::*;
use railit_types::{RouteId, TvdId};

type Tamper = (&'static str, fn(&mut railit_plan::Plan));

const EXAMPLE: &str = include_str!("../../../examples/beispielstadt.toml");

fn example() -> Topology {
    railit_import::toml_format::parse(EXAMPLE).unwrap().topology
}

fn port(node: NodeRef, port: Port) -> PortRef {
    PortRef::new(node, port)
}

#[test]
fn example_routes_are_generated() {
    let plan = generate(&example()).unwrap();
    let names: Vec<_> = plan.routes.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "A-N1", "A-N2", "N1-GE", "N2-GE", "F-P1", "F-P2", "P1-GW", "P2-GW"
        ]
    );

    let tvd = |name: &str| {
        TvdId::from_index(plan.tvds.iter().position(|t| t.name == name).unwrap()).unwrap()
    };
    let a_n1 = &plan.routes[0];
    assert_eq!(a_n1.tvds, [tvd("TW1"), tvd("T1")]);
    assert_eq!(a_n1.points, [(PointId::new(0), PointPosition::Left)]);
    assert!(matches!(a_n1.end, RouteEnd::Signal(_)));
    assert_eq!(plan.routes[2].end, RouteEnd::Boundary);

    // REQ-PLN-012
    let conflicts: BTreeSet<&str> = a_n1
        .conflicts
        .iter()
        .map(|c| plan.routes[c.index().unwrap()].name.as_str())
        .collect();
    assert_eq!(
        conflicts,
        BTreeSet::from(["A-N2", "F-P1", "P1-GW", "P2-GW"])
    );
    let n1_ge = &plan.routes[2];
    let conflicts: BTreeSet<&str> = n1_ge
        .conflicts
        .iter()
        .map(|c| plan.routes[c.index().unwrap()].name.as_str())
        .collect();
    assert_eq!(conflicts, BTreeSet::from(["N2-GE", "F-P1", "F-P2"]));
}

#[test]
fn example_compiles_signed_and_unsigned() {
    let net = example();
    let unsigned = compile(&net, None).unwrap();
    assert!(!PlanView::parse(&unsigned.image).unwrap().is_signed());

    let key = SigningKey::from_bytes(&[42; 32]);
    let signed = compile(&net, Some(&key)).unwrap();
    let view = PlanView::parse_signed(&signed.image, &key.verifying_key()).unwrap();
    assert_eq!(view.route_count(), 8);
    assert_eq!(view.find_route("F-P2"), Some(RouteId::new(5)));
}

/// REQ-PLN-013: Die Gegenprüfung erkennt jede Verfälschung des Plans.
#[test]
fn verification_detects_tampering() {
    let net = example();
    let plan = generate(&net).unwrap();
    verify(&net, &plan).unwrap();

    let tamper: [Tamper; 6] = [
        ("Fahrstraße entfernt", |p| {
            p.routes.remove(7);
            for r in &mut p.routes {
                r.conflicts.retain(|c| c.index() != Some(7));
            }
        }),
        ("Weichenlage vertauscht", |p| {
            p.routes[0].points[0].1 = PointPosition::Right;
        }),
        ("Weiche fehlt", |p| p.routes[0].points.clear()),
        ("Abschnitt fehlt", |p| {
            p.routes[0].tvds.pop();
        }),
        ("Feindlichkeit fehlt", |p| {
            p.routes[0].conflicts.retain(|c| c.index() != Some(4));
            p.routes[4].conflicts.retain(|c| c.index() != Some(0));
        }),
        ("Weiche im falschen Abschnitt", |p| {
            p.points[0].tvd = TvdId::new(0);
        }),
    ];
    for (what, f) in tamper {
        let mut bad = plan.clone();
        f(&mut bad);
        let report = verify(&net, &bad).expect_err(what);
        assert!(
            report
                .errors
                .iter()
                .all(|e| e.requirement() == "REQ-PLN-013"),
            "{what}: {report}"
        );
    }
}

fn line(signal_pos: impl Fn(Length) -> Length, beyond: TerminalKind, same_tvd: bool) -> Topology {
    let mut b = TopologyBuilder::new("Strecke");
    let m = |m| Length::from_m(m).unwrap();
    let s = NodeRef::Terminal(b.add_terminal("S", TerminalKind::Boundary).unwrap());
    let e = NodeRef::Terminal(b.add_terminal("E", beyond).unwrap());
    let t1 = b.add_track("1", m(100)).unwrap();
    let t2 = b.add_track("2", m(100)).unwrap();
    b.connect(port(s, Port::End), port(NodeRef::Track(t1), Port::A))
        .unwrap();
    b.connect(
        port(NodeRef::Track(t1), Port::B),
        port(NodeRef::Track(t2), Port::A),
    )
    .unwrap();
    b.connect(port(NodeRef::Track(t2), Port::B), port(e, Port::End))
        .unwrap();
    b.add_signal(
        "X",
        t1,
        signal_pos(m(100)),
        Direction::AToB,
        SignalKind::Main,
    )
    .unwrap();
    if same_tvd {
        b.add_tvd("T", &[TvdMember::Track(t1), TvdMember::Track(t2)])
            .unwrap();
    } else {
        b.add_tvd("T1", &[TvdMember::Track(t1)]).unwrap();
        b.add_tvd("T2", &[TvdMember::Track(t2)]).unwrap();
    }
    b.build().unwrap().topology
}

/// REQ-PLN-010
#[test]
fn signals_must_stand_at_tvd_boundaries() {
    let ok = generate(&line(|len| len, TerminalKind::BufferStop, false)).unwrap();
    assert_eq!(ok.routes.len(), 1);
    assert_eq!(ok.routes[0].name, "X-E");
    assert_eq!(ok.routes[0].end, RouteEnd::BufferStop);

    let err = generate(&line(
        |_| Length::from_m(50).unwrap(),
        TerminalKind::Boundary,
        false,
    ))
    .unwrap_err();
    assert_eq!(
        err.errors,
        [PlanningError::SignalNotAtTrackEnd { signal: "X".into() }]
    );

    let err = generate(&line(|len| len, TerminalKind::Boundary, true)).unwrap_err();
    assert_eq!(
        err.errors,
        [PlanningError::SignalNotAtTvdBoundary { signal: "X".into() }]
    );
    assert!(err.to_string().contains("REQ-PLN-010"));
}

/// REQ-PLN-014: Kehrschleife ohne Signal.
#[test]
fn loops_without_signal_are_rejected() {
    let mut b = TopologyBuilder::new("Schleife");
    let m = |m| Length::from_m(m).unwrap();
    let s = NodeRef::Terminal(b.add_terminal("S", TerminalKind::Boundary).unwrap());
    let t0 = b.add_track("0", m(100)).unwrap();
    let w = b.add_point("W").unwrap();
    let l = b.add_track("L", m(500)).unwrap();
    let (n0, nw, nl) = (NodeRef::Track(t0), NodeRef::Point(w), NodeRef::Track(l));
    b.connect(port(s, Port::End), port(n0, Port::A)).unwrap();
    b.connect(port(n0, Port::B), port(nw, Port::Tip)).unwrap();
    b.connect(port(nw, Port::Left), port(nl, Port::A)).unwrap();
    b.connect(port(nl, Port::B), port(nw, Port::Right)).unwrap();
    b.add_signal("X", t0, m(100), Direction::AToB, SignalKind::Main)
        .unwrap();
    b.add_tvd("T0", &[TvdMember::Track(t0)]).unwrap();
    b.add_tvd("TW", &[TvdMember::Point(w)]).unwrap();
    b.add_tvd("TL", &[TvdMember::Track(l)]).unwrap();
    let net = b.build().unwrap().topology;

    let report = generate(&net).unwrap_err();
    assert!(!report.errors.is_empty());
    assert!(
        report
            .errors
            .iter()
            .all(|e| e.requirement() == "REQ-PLN-014"),
        "{report}"
    );
}

/// Bahnhof mit `k` Bahnsteiggleisen als Weichenleiter auf beiden Seiten.
fn ladder(k: usize) -> Topology {
    let mut b = TopologyBuilder::new(format!("Leiter{k}"));
    let m = |m| Length::from_m(m).unwrap();
    let gw = NodeRef::Terminal(b.add_terminal("GW", TerminalKind::Boundary).unwrap());
    let ge = NodeRef::Terminal(b.add_terminal("GE", TerminalKind::Boundary).unwrap());
    let west = b.add_track("west", m(1000)).unwrap();
    let east = b.add_track("east", m(1000)).unwrap();
    b.connect(port(gw, Port::End), port(NodeRef::Track(west), Port::A))
        .unwrap();
    b.connect(port(NodeRef::Track(east), Port::B), port(ge, Port::End))
        .unwrap();
    b.add_signal("A", west, m(1000), Direction::AToB, SignalKind::Main)
        .unwrap();
    b.add_signal("F", east, Length::ZERO, Direction::BToA, SignalKind::Main)
        .unwrap();
    b.add_tvd("TW", &[TvdMember::Track(west)]).unwrap();
    b.add_tvd("TE", &[TvdMember::Track(east)]).unwrap();

    let mut west_end = port(NodeRef::Track(west), Port::B);
    let mut east_end = port(NodeRef::Track(east), Port::A);
    for i in 1..=k {
        let t = b.add_track(i.to_string(), m(600)).unwrap();
        b.add_signal(
            format!("N{i}"),
            t,
            m(600),
            Direction::AToB,
            SignalKind::Main,
        )
        .unwrap();
        b.add_signal(
            format!("P{i}"),
            t,
            Length::ZERO,
            Direction::BToA,
            SignalKind::Main,
        )
        .unwrap();
        b.add_tvd(format!("T{i}"), &[TvdMember::Track(t)]).unwrap();
        let (ta, tb) = (
            port(NodeRef::Track(t), Port::A),
            port(NodeRef::Track(t), Port::B),
        );
        if i < k {
            let ww = b.add_point(format!("WW{i}")).unwrap();
            let we = b.add_point(format!("WE{i}")).unwrap();
            b.add_tvd(format!("TWW{i}"), &[TvdMember::Point(ww)])
                .unwrap();
            b.add_tvd(format!("TWE{i}"), &[TvdMember::Point(we)])
                .unwrap();
            b.connect(west_end, port(NodeRef::Point(ww), Port::Tip))
                .unwrap();
            b.connect(port(NodeRef::Point(ww), Port::Right), ta)
                .unwrap();
            b.connect(east_end, port(NodeRef::Point(we), Port::Tip))
                .unwrap();
            b.connect(port(NodeRef::Point(we), Port::Right), tb)
                .unwrap();
            west_end = port(NodeRef::Point(ww), Port::Left);
            east_end = port(NodeRef::Point(we), Port::Left);
        } else {
            b.connect(west_end, ta).unwrap();
            b.connect(east_end, tb).unwrap();
        }
    }
    b.build().unwrap().topology
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// Generator und Gegenprüfung stimmen für Bahnhöfe beliebiger Größe überein.
    #[test]
    fn ladder_stations_plan_and_verify(k in 1usize..12) {
        let net = ladder(k);
        let compiled = compile(&net, None).unwrap();
        let routes = &compiled.plan.routes;
        prop_assert_eq!(routes.len(), 4 * k);
        // Alle Fahrstraßen ab A sind untereinander feindlich (gleiches Startsignal).
        let from_a: Vec<usize> = routes.iter().enumerate()
            .filter(|(_, r)| r.name.starts_with("A-")).map(|(i, _)| i).collect();
        prop_assert_eq!(from_a.len(), k);
        for &a in &from_a {
            for &b in &from_a {
                if a != b {
                    prop_assert!(routes[a].conflicts.contains(&RouteId::from_index(b).unwrap()));
                }
            }
        }
    }
}
