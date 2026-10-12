//! railML 3: Rundreise über den Export und Import eines handgeschriebenen Dokuments.

use std::collections::BTreeSet;

use railit_import::{ImportError, railml, toml_format};
use railit_topology::{NodeRef, Port, PortRef, SignalKind, TerminalKind, Topology, TvdMember};

const BEISPIELSTADT: &str = include_str!("../../../examples/beispielstadt.toml");
const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");
const HANDMADE: &str = include_str!("data/abzweig.railml");

fn node_name(net: &Topology, n: NodeRef) -> String {
    match n {
        NodeRef::Track(t) => net.track(t).unwrap().name.clone(),
        NodeRef::Point(p) => net.point(p).unwrap().name.clone(),
        NodeRef::Terminal(t) => net.terminal(t).unwrap().name.clone(),
    }
}

/// Beschreibung des Netzes über Namen, unabhängig von der Reihenfolge der Elemente.
fn describe(net: &Topology) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    out.insert(format!("network {}", net.name()));
    for (t, track) in net.tracks() {
        out.insert(format!("track {} {}", track.name, track.length.as_mm()));
        for port in [Port::A, Port::B] {
            let here = PortRef::new(NodeRef::Track(t), port);
            let there = net.linked(here).unwrap();
            let mut ends = [
                format!("{}.{port:?}", track.name),
                format!("{}.{:?}", node_name(net, there.node), there.port),
            ];
            ends.sort();
            out.insert(format!("link {} {}", ends[0], ends[1]));
        }
    }
    for (_, p) in net.points() {
        out.insert(format!(
            "point {} {:?} {}",
            p.name,
            p.diverging,
            p.diverging_speed.as_kmh()
        ));
    }
    for (_, t) in net.terminals() {
        out.insert(format!("terminal {} {:?}", t.name, t.kind));
    }
    for (_, s) in net.signals() {
        out.insert(format!(
            "signal {} {} {} {:?} {:?} {:?} {}",
            s.name,
            net.track(s.track).unwrap().name,
            s.position.as_mm(),
            s.direction,
            s.kind,
            s.system,
            s.auto
        ));
    }
    for (_, lx) in net.level_crossings() {
        out.insert(format!(
            "lx {} {} {}",
            lx.name,
            net.track(lx.track).unwrap().name,
            lx.position.as_mm()
        ));
    }
    for (_, tvd) in net.tvds() {
        let mut members = tvd
            .members
            .iter()
            .map(|m| match *m {
                TvdMember::Track(t) => net.track(t).unwrap().name.clone(),
                TvdMember::Point(p) => net.point(p).unwrap().name.clone(),
            })
            .collect::<Vec<_>>();
        members.sort();
        out.insert(format!("tvd {} {members:?}", tvd.name));
    }
    for (_, a) in net.areas() {
        let mut tvds = a
            .tvds
            .iter()
            .map(|t| net.tvd(*t).unwrap().name.clone())
            .collect::<Vec<_>>();
        tvds.sort();
        out.insert(format!("area {} {tvds:?}", a.name));
    }
    for (_, s) in net.stations() {
        for p in &s.platforms {
            let signals = p
                .signals
                .iter()
                .map(|x| net.signal(*x).unwrap().name.clone())
                .collect::<Vec<_>>();
            out.insert(format!("platform {} {} {signals:?}", s.name, p.name));
        }
    }
    out
}

fn roundtrip(toml: &str) {
    let original = toml_format::parse(toml).unwrap().topology;
    let xml = railml::export(&original).unwrap();
    let (back, warnings) =
        railml::parse_with_warnings(&xml).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(describe(&original), describe(&back.topology));
    // Erneuter Export ist identisch.
    assert_eq!(railml::export(&back.topology).unwrap(), xml);
}

#[test]
fn beispielstadt_survives_the_roundtrip() {
    roundtrip(BEISPIELSTADT);
}

#[test]
fn nordbahn_with_areas_and_stations_survives_the_roundtrip() {
    roundtrip(NORDBAHN);
}

#[test]
fn handmade_railml_is_imported() {
    let (v, warnings) = railml::parse_with_warnings(HANDMADE).unwrap_or_else(|e| panic!("{e}"));
    let net = &v.topology;
    assert_eq!(net.name(), "Abzweig Kleinstadt");
    // Der Achszähler in der Mitte von „Strecke“ teilt sie in zwei Gleisabschnitte.
    let names = net
        .tracks()
        .map(|(_, t)| t.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["Strecke/1", "Strecke/2", "1", "2"]);
    let lengths = net
        .tracks()
        .map(|(_, t)| t.length.as_mm())
        .collect::<Vec<_>>();
    assert_eq!(lengths, [700_000, 300_000, 450_000, 400_000]);
    // Abschnittsnamen aus den tvdSections, sonst erzeugt.
    let tvds = net
        .tvds()
        .map(|(_, t)| t.name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        tvds,
        BTreeSet::from(["1101", "1102", "1103", "1104", "T-2"]),
        "Abschnitte: {:?}",
        describe(net)
    );
    let w1 = net.point(railit_topology::PointId::new(0)).unwrap();
    assert_eq!(w1.name, "W1");
    assert_eq!(w1.diverging, Some(railit_topology::PointPosition::Right));
    assert_eq!(w1.diverging_speed.as_kmh(), 60);
    // Hauptsignal mit Vorsignal am Mast wird Mehrabschnittssignal; Signal ohne Funktion für
    // Zugfahrten wird übergangen.
    let kinds = net
        .signals()
        .map(|(_, s)| (s.name.as_str(), s.kind, s.position.as_mm()))
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            ("A", SignalKind::Combined, 700_000),
            ("N1", SignalKind::Main, 0),
            ("N2", SignalKind::Main, 0),
        ]
    );
    assert!(warnings.iter().any(|w| w.contains("lf7")), "{warnings:?}");
    let kinds = net.terminals().map(|(_, t)| t.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            TerminalKind::Boundary,
            TerminalKind::BufferStop,
            TerminalKind::BufferStop
        ]
    );
    assert_eq!(net.level_crossings().len(), 1);
}

#[test]
fn errors_are_collected_with_location() {
    let broken = HANDMADE.replace("length=\"450\"", "").replace(
        "netElementRef=\"ne_2\" applicationDirection=\"reverse\"",
        "netElementRef=\"ne_2\"",
    );
    let ImportError::Invalid { errors, .. } = railml::parse(&broken).unwrap_err() else {
        panic!("Fehler erwartet");
    };
    let text = errors.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert!(
        text.iter()
            .any(|e| e.contains("ne_1") && e.contains("Länge")),
        "{text:?}"
    );
    assert!(
        text.iter()
            .any(|e| e.contains("sig_n2") && e.contains("Wirkrichtung")),
        "{text:?}"
    );
    assert!(matches!(
        railml::parse("<railML"),
        Err(ImportError::Syntax(_))
    ));
    assert!(matches!(
        railml::parse("<foo/>"),
        Err(ImportError::Syntax(_))
    ));
}

#[test]
fn format_follows_the_file_extension() {
    assert!(railit_import::parse("netz.railml", HANDMADE).is_ok());
    assert!(railit_import::parse("netz.XML", HANDMADE).is_ok());
    assert!(railit_import::parse("netz.toml", BEISPIELSTADT).is_ok());
}
