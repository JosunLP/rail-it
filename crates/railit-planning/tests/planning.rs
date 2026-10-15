//! Projektierung: Generator, Gegenprüfung, Kompilierung.

#![allow(clippy::many_single_char_names)]

use std::collections::BTreeSet;

use proptest::prelude::*;
use railit_plan::{Flank, PlanView, Protection, RouteEnd, SigningKey};
use railit_planning::{PlanningConfig, PlanningError, compile, compile_with, generate, verify};
use railit_topology::*;
use railit_types::{LxId, RouteId, SignalId, Speed, TrackId, TvdId};

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
    let names_of = |ids: &[RouteId]| -> BTreeSet<String> {
        ids.iter()
            .map(|c| plan.routes[c.index().unwrap()].name.clone())
            .collect()
    };
    let set = |v: &[&str]| v.iter().map(|s| (*s).to_string()).collect::<BTreeSet<_>>();
    assert_eq!(
        names_of(&a_n1.conflicts),
        set(&["A-N2", "N2-GE", "F-P1", "F-P2", "P1-GW", "P2-GW"]),
        "N1-GE setzt A-N1 fort und ist verträglich"
    );
    assert_eq!(
        names_of(&plan.routes[2].conflicts),
        set(&["A-N2", "N2-GE", "F-P1", "F-P2"])
    );
}

/// REQ-PLN-015 … REQ-PLN-018: Durchrutschweg, Flankenschutz, Vorsignale, Bahnübergänge.
#[test]
fn example_phase2_data() {
    let net = example();
    let plan = generate(&net).unwrap();
    let tvd = |name: &str| {
        TvdId::from_index(plan.tvds.iter().position(|t| t.name == name).unwrap()).unwrap()
    };
    let signal = |name: &str| {
        SignalId::from_index(plan.signals.iter().position(|s| s.name == name).unwrap()).unwrap()
    };
    let route = |name: &str| plan.routes.iter().find(|r| r.name == name).unwrap();

    // Durchrutschweg hinter N1: über W2 (stumpf, links) bis zum Ende von TE.
    let a_n1 = route("A-N1");
    assert_eq!(a_n1.overlap_tvds, [tvd("TW2"), tvd("TE")]);
    assert_eq!(
        a_n1.overlap_points,
        [(PointId::new(1), PointPosition::Left)]
    );
    // Flankenschutz: W1 links ⇒ P2 auf Halt; W2 links (Durchrutschweg) ⇒ N2 auf Halt.
    assert_eq!(
        a_n1.flank,
        [
            Flank {
                protection: Protection::Signal(signal("P2")),
                protects: tvd("TW1")
            },
            Flank {
                protection: Protection::Signal(signal("N2")),
                protects: tvd("TW2")
            },
        ]
    );
    // Geschwindigkeit nur über den abzweigenden Strang.
    assert_eq!(a_n1.speed, None);
    assert_eq!(route("A-N2").speed, Some(Speed::from_kmh(40)));
    // BÜ1 liegt im Fahrweg der Ausfahrten nach Osten, nicht im 100-m-Durchrutschweg von A-N1.
    let bue = LxId::new(0);
    assert_eq!(route("N1-GE").level_crossings, [bue]);
    assert!(a_n1.level_crossings.is_empty());
    assert!(route("F-P1").level_crossings.is_empty());
    // Ausfahrten an die Netzgrenze haben keinen Durchrutschweg.
    assert!(route("N1-GE").overlap_tvds.is_empty());

    // Vorsignale.
    assert_eq!(
        plan.signals[signal("a").index().unwrap()].announces,
        Some(signal("A"))
    );
    assert_eq!(
        plan.signals[signal("f").index().unwrap()].announces,
        Some(signal("F"))
    );
    assert_eq!(plan.signals[signal("A").index().unwrap()].announces, None);

    // Einfahrabschnitte an den Netzgrenzen.
    let entries: Vec<&str> = plan
        .tvds
        .iter()
        .filter(|t| t.entry)
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(entries, ["TW", "TE"]);
}

/// Mit längerem Durchrutschweg liegt BÜ1 im Durchrutschweg von A-N1.
#[test]
fn level_crossing_within_long_overlap() {
    let cfg = PlanningConfig {
        overlap_length: Length::from_m(600).unwrap(),
        ..PlanningConfig::default()
    };
    let compiled = compile_with(&example(), None, &cfg).unwrap();
    let a_n1 = compiled
        .plan
        .routes
        .iter()
        .find(|r| r.name == "A-N1")
        .unwrap();
    assert_eq!(a_n1.level_crossings, [LxId::new(0)]);
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

    let tamper: [Tamper; 13] = [
        ("Länge verfälscht", |p| {
            p.routes[0].length = Length::from_mm(p.routes[0].length.as_mm() + 1);
        }),
        ("Durchrutschweg zu lang", |p| {
            p.routes[0].overlap_length = Length::from_mm(p.routes[0].overlap_length.as_mm() + 1);
        }),
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
        ("Durchrutschweg verkürzt", |p| {
            p.routes[0].overlap_tvds.pop();
        }),
        ("Flankenschutz fehlt", |p| {
            p.routes[0].flank.pop();
        }),
        ("Geschwindigkeit fehlt", |p| p.routes[1].speed = None),
        ("Bahnübergang fehlt", |p| {
            p.routes[2].level_crossings.clear();
        }),
        ("Vorsignal falsch", |p| {
            p.signals[6].announces = Some(SignalId::new(3));
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

/// Gabelung: S ─ t0 ─X▶ W ┬─ t1 ─ t1b ─ E1
///                         └─ t2 ─ E2
fn fork(with_flank_signal: bool, distant_before_fork: bool) -> Topology {
    let mut b = TopologyBuilder::new("Gabel");
    let m = |m| Length::from_m(m).unwrap();
    let ends: Vec<NodeRef> = ["S", "E1", "E2"]
        .iter()
        .map(|n| NodeRef::Terminal(b.add_terminal(*n, TerminalKind::Boundary).unwrap()))
        .collect();
    let tracks: Vec<TrackId> = ["t0", "t1", "t1b", "t2"]
        .iter()
        .map(|n| b.add_track(*n, m(300)).unwrap())
        .collect();
    let w = b.add_point("W").unwrap();
    let (t0, t1, t1b, t2) = (
        NodeRef::Track(tracks[0]),
        NodeRef::Track(tracks[1]),
        NodeRef::Track(tracks[2]),
        NodeRef::Track(tracks[3]),
    );
    let pw = NodeRef::Point(w);
    for (x, y) in [
        (port(ends[0], Port::End), port(t0, Port::A)),
        (port(t0, Port::B), port(pw, Port::Tip)),
        (port(pw, Port::Left), port(t1, Port::A)),
        (port(t1, Port::B), port(t1b, Port::A)),
        (port(t1b, Port::B), port(ends[1], Port::End)),
        (port(pw, Port::Right), port(t2, Port::A)),
        (port(t2, Port::B), port(ends[2], Port::End)),
    ] {
        b.connect(x, y).unwrap();
    }
    b.add_signal("X", tracks[0], m(300), Direction::AToB, SignalKind::Main)
        .unwrap();
    b.add_signal("Y", tracks[1], m(300), Direction::AToB, SignalKind::Main)
        .unwrap();
    if with_flank_signal {
        for (name, track) in [("Z1", tracks[1]), ("Z2", tracks[3])] {
            b.add_signal(name, track, m(100), Direction::BToA, SignalKind::Shunting)
                .unwrap();
        }
    }
    if distant_before_fork {
        b.add_signal(
            "v",
            tracks[0],
            Length::ZERO,
            Direction::BToA,
            SignalKind::Distant,
        )
        .unwrap();
        b.add_signal(
            "v2",
            tracks[2],
            Length::ZERO,
            Direction::BToA,
            SignalKind::Distant,
        )
        .unwrap();
    }
    for (i, t) in tracks.iter().enumerate() {
        b.add_tvd(format!("T{i}"), &[TvdMember::Track(*t)]).unwrap();
    }
    b.add_tvd("TW", &[TvdMember::Point(w)]).unwrap();
    b.build().unwrap().topology
}

/// REQ-PLN-016
#[test]
fn flanks_must_be_protectable() {
    let report = generate(&fork(false, false)).unwrap_err();
    assert!(
        report.errors.iter().any(|e| matches!(e, PlanningError::UnprotectedFlank { route, point, .. } if route == "X-Y" && point == "W")),
        "{report}"
    );
    assert!(
        report
            .errors
            .iter()
            .all(|e| e.requirement() == "REQ-PLN-016")
    );

    // Rangiersignale auf beiden Strängen schützen die Flanken.
    let compiled = compile(&fork(true, false), None).unwrap();
    let x_y = compiled
        .plan
        .routes
        .iter()
        .find(|r| r.name == "X-Y")
        .unwrap();
    assert!(matches!(x_y.flank[0].protection, Protection::Signal(_)));
}

/// REQ-PLN-017
#[test]
fn distant_signals_need_a_unique_main_signal() {
    let report = generate(&fork(true, true)).unwrap_err();
    let names: Vec<&str> = report
        .errors
        .iter()
        .filter_map(|e| match e {
            PlanningError::DistantSignal { signal, .. } => Some(signal.as_str()),
            _ => None,
        })
        .collect();
    // v zeigt zur Netzgrenze S, v2 über die spitz befahrene Weiche W.
    assert_eq!(names, ["v", "v2"], "{report}");
}

const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");

/// REQ-PLN-019: Projektierung je Stellbereich mit Grenzen und Annäherungsabschnitten.
#[test]
fn nordbahn_compiles_per_area() {
    use railit_plan::{PlanView, RouteEnd};
    let net = railit_import::toml_format::parse(NORDBAHN)
        .unwrap()
        .topology;
    let areas = railit_planning::compile_areas(&net, None, &PlanningConfig::default()).unwrap();
    assert_eq!(areas.len(), 3);

    let plan = |name: &str| {
        let a = areas.iter().find(|a| a.name == name).unwrap();
        PlanView::parse(&a.compiled.image).unwrap();
        a.compiled.plan.clone()
    };
    let tvd = |p: &railit_plan::Plan, n: &str| {
        railit_types::TvdId::from_index(p.tvds.iter().position(|t| t.name == n).unwrap()).unwrap()
    };
    let signal = |p: &railit_plan::Plan, n: &str| {
        railit_types::SignalId::from_index(p.signals.iter().position(|s| s.name == n).unwrap())
            .unwrap()
    };

    let strecke = plan("Strecke");
    let names: Vec<&str> = strecke.routes.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Sbk1->Bstadt", "Sbk2->Adorf"]);
    assert!(matches!(strecke.routes[0].end, RouteEnd::Border(_)));
    assert!(strecke.routes[0].overlap_tvds.is_empty());
    let borders: Vec<(&str, bool)> = strecke
        .borders
        .iter()
        .map(|b| (b.name.as_str(), b.initial))
        .collect();
    assert_eq!(borders, [(">Adorf", false), (">Bstadt", true)]);
    let west = &strecke.borders[0];
    assert_eq!(west.entry_signal, signal(&strecke, "Sbk1"));
    assert_eq!(
        west.approach_tvds,
        [tvd(&strecke, "ST1"), tvd(&strecke, "ST2")]
    );

    let adorf = plan("Adorf");
    let exit = adorf
        .routes
        .iter()
        .find(|r| r.name == "AN1->Strecke")
        .unwrap();
    assert_eq!(
        exit.tvds,
        [tvd(&adorf, "ATW2"), tvd(&adorf, "ATO1"), tvd(&adorf, "ATO")]
    );
    let border = &adorf.borders[0];
    assert!(border.initial);
    assert_eq!(border.entry_signal, signal(&adorf, "AF"));
    assert_eq!(
        border.approach_tvds,
        [tvd(&adorf, "ATO"), tvd(&adorf, "ATO1")]
    );
    assert!(border.approach_points.is_empty());
    // Die beiden Ausfahrten zur Strecke sind feindlich, Einfahrt AF-AP1 ebenfalls.
    let id = |n: &str| {
        railit_types::RouteId::from_index(adorf.routes.iter().position(|r| r.name == n).unwrap())
            .unwrap()
    };
    assert!(exit.conflicts.contains(&id("AN2->Strecke")));
    assert!(exit.conflicts.contains(&id("AF-AP1")));

    let bstadt = plan("Bstadt");
    assert!(!bstadt.borders[0].initial);
    assert!(bstadt.routes.iter().any(|r| r.name == "BP2->Strecke"));
}

/// REQ-PLN-019: Ohne Einfahrsignal hinter einer Grenze ist der Plan ungültig; ein
/// manipulierter Annäherungsabschnitt fällt der Gegenprüfung auf.
#[test]
fn borders_need_an_entry_signal() {
    let net = railit_import::toml_format::parse(NORDBAHN)
        .unwrap()
        .topology;
    let without = NORDBAHN.replace(
        "[[signal]]\nname = \"AF\"\ntrack = \"Ao\"\nposition_m = 0\ndirection = \"b_to_a\"\nkind = \"main\"\n",
        "",
    );
    assert_ne!(without, NORDBAHN);
    let bad = railit_import::toml_format::parse(&without)
        .unwrap()
        .topology;
    let err = railit_planning::compile_areas(&bad, None, &PlanningConfig::default()).unwrap_err();
    assert!(
        err.errors.iter().any(|e| e.requirement() == "REQ-PLN-019"),
        "{err}"
    );

    let strecke = net.area_cut(net.find_area("Strecke").unwrap()).unwrap();
    let mut plan = railit_planning::generate(&strecke.topology).unwrap();
    railit_planning::verify(&strecke.topology, &plan).unwrap();
    plan.borders[0].approach_tvds.pop();
    assert!(railit_planning::verify(&strecke.topology, &plan).is_err());
}

/// Plan v4: Längen von Fahrweg und Durchrutschweg für die Fahrerlaubnis nach ETCS.
#[test]
fn route_and_overlap_lengths() {
    let net = railit_import::toml_format::parse(NORDBAHN)
        .unwrap()
        .topology;
    let areas = railit_planning::compile_areas(&net, None, &PlanningConfig::default()).unwrap();
    let length = |area: &str, route: &str| {
        let a = areas.iter().find(|a| a.name == area).unwrap();
        let r = a
            .compiled
            .plan
            .routes
            .iter()
            .find(|r| r.name == route)
            .unwrap();
        (r.length.as_mm() / 1000, r.overlap_length.as_mm() / 1000)
    };
    // Einfahrt: Weiche AW1 (ohne Länge) und Gleis A1; Durchrutschweg über AW2 bis Ende Ao1.
    assert_eq!(length("Adorf", "AA-AN1"), (600, 120));
    // Ausfahrt bis zur Bereichsgrenze: Ao1 und Ao; kein Durchrutschweg.
    assert_eq!(length("Adorf", "AN1->Strecke"), (420, 0));
    assert_eq!(length("Strecke", "Sbk1->Bstadt"), (1500, 0));
    let a = areas.iter().find(|a| a.name == "Adorf").unwrap();
    let view = PlanView::parse(&a.compiled.image).unwrap();
    let r = view.route(view.find_route("AA-AN1").unwrap()).unwrap();
    assert_eq!(
        (r.length().as_mm(), r.overlap_length().as_mm()),
        (600_000, 120_000)
    );
    // Grenze „>Strecke“ der Strecke: von Adorf bis Sbk1 über S1, Durchrutschweg über S2.
    let s = areas.iter().find(|a| a.name == "Strecke").unwrap();
    let b = s
        .compiled
        .plan
        .borders
        .iter()
        .find(|b| b.name == ">Adorf")
        .unwrap();
    assert_eq!(
        (b.approach_length.as_mm(), b.entry_overlap_length.as_mm()),
        (1_500_000, 1_500_000)
    );
}

/// REQ-TOP-015: Ein über PlanPro ausgetauschtes Netz ergibt ein bitgleiches Plan-Image.
#[test]
fn planpro_roundtrip_yields_identical_plans() {
    let original = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
    let (xml, _) = railit_import::planpro::export(&original).unwrap();
    let back = railit_import::planpro::parse(&xml).unwrap().topology;
    assert_eq!(
        compile(&original, None).unwrap().image,
        compile(&back, None).unwrap().image
    );
}

/// REQ-TOP-014: Ein über railML 3 ausgetauschtes Netz ergibt bitgleiche Plan-Images.
#[test]
fn railml_roundtrip_yields_identical_plans() {
    for source in [EXAMPLE, NORDBAHN] {
        let original = railit_import::toml_format::parse(source).unwrap().topology;
        let xml = railit_import::railml::export(&original).unwrap();
        let back = railit_import::railml::parse(&xml).unwrap().topology;
        let cfg = PlanningConfig::default();
        let images = |net: &Topology| {
            if net.areas().len() == 0 {
                return vec![(String::new(), compile(net, None).unwrap().image)];
            }
            railit_planning::compile_areas(net, None, &cfg)
                .unwrap()
                .into_iter()
                .map(|a| (a.name, a.compiled.image))
                .collect::<Vec<_>>()
        };
        assert_eq!(images(&original), images(&back));
    }
}

/// REQ-PLN-010: Ein Hauptsignal darf bis zu `signal_gap` vor der Freimeldegrenze stehen. Die
/// Längen zählen dann ab dem Signal: Fahrweg mit dem Rest des Startgleises und ohne das Stück
/// hinter dem Zielsignal, Durchrutschweg ab dem Zielsignal. Liegt ein Bahnübergang im Stück
/// dahinter oder ist der Abstand größer, ist das ein Fehler.
#[test]
fn signals_may_stand_shortly_before_the_tvd_boundary() {
    // A steht 1000 m auf „west“ (am Ende), N1 auf Gleis 1 (600 m) am Ende.
    let exact = generate(&example()).unwrap();
    let moved = EXAMPLE
        .replace(
            "name = \"N1\"\ntrack = \"1\"\nposition_m = 600",
            "name = \"N1\"\ntrack = \"1\"\nposition_m = 588",
        )
        .replace(
            "name = \"A\"\ntrack = \"west\"\nposition_m = 1000",
            "name = \"A\"\ntrack = \"west\"\nposition_m = 975",
        );
    let net = railit_import::toml_format::parse(&moved).unwrap().topology;
    let plan = generate(&net).unwrap();
    verify(&net, &plan).unwrap();
    let route =
        |p: &railit_plan::Plan, n: &str| p.routes.iter().find(|r| r.name == n).cloned().unwrap();
    let (before, after) = (route(&exact, "A-N1"), route(&plan, "A-N1"));
    assert_eq!(
        after.length.as_mm(),
        before.length.as_mm() + 25_000 - 12_000
    );
    assert_eq!(
        after.overlap_length.as_mm(),
        before.overlap_length.as_mm() + 12_000
    );
    assert_eq!(after.tvds, before.tvds);

    let far = EXAMPLE.replace(
        "name = \"N1\"\ntrack = \"1\"\nposition_m = 600",
        "name = \"N1\"\ntrack = \"1\"\nposition_m = 560",
    );
    let err = generate(&railit_import::toml_format::parse(&far).unwrap().topology).unwrap_err();
    assert!(err.errors.contains(&PlanningError::SignalNotAtTrackEnd {
        signal: "N1".into()
    }));
    let strict = PlanningConfig {
        signal_gap: Length::ZERO,
        ..PlanningConfig::default()
    };
    assert!(railit_planning::generate_with(&net, &strict).is_err());
}
