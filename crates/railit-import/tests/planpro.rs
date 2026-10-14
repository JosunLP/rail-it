//! PlanPro: Rundreise über den Export und Import sowie ein handgeschriebenes Dokument im Stil
//! einer PlanPro-1.9-Planung (REQ-TOP-015).

use std::collections::BTreeSet;

use railit_import::{ImportError, planpro, toml_format};
use railit_topology::{NodeRef, Port, PortRef, SignalKind, SignalSystem, Topology, TvdMember};

const BEISPIELSTADT: &str = include_str!("../../../examples/beispielstadt.toml");
const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");
const HANDMADE: &str = include_str!("../../../examples/abzweig.ppxml");

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

fn roundtrip(net: &Topology) {
    let (xml, warnings) = planpro::export(net).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let (back, warnings) =
        planpro::parse_with_warnings(&xml).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(describe(net), describe(&back.topology));
    // Erneuter Export ist identisch.
    assert_eq!(planpro::export(&back.topology).unwrap().0, xml);
}

/// REQ-TOP-015: Export und erneuter Import ergeben dasselbe Netz.
#[test]
fn beispielstadt_survives_the_roundtrip() {
    roundtrip(&toml_format::parse(BEISPIELSTADT).unwrap().topology);
}

/// REQ-TOP-015: auch ein erzeugter Korridor mit vielen Bahnhöfen.
#[test]
fn generated_corridor_survives_the_roundtrip() {
    let net = railit_import::generate::corridor(6, 2);
    let net = toml_format::parse(&net).unwrap().topology;
    roundtrip(&net);
}

/// Stellbereiche bildet der Export nicht ab.
#[test]
fn areas_are_not_exported() {
    let net = toml_format::parse(NORDBAHN).unwrap().topology;
    assert!(planpro::export(&net).unwrap_err().contains("Stellbereich"));
}

/// REQ-TOP-015: Handgeschriebene Planung im Stil von PlanPro 1.9; der Startzustand wird nicht
/// gelesen, nicht abbildbare Objekte werden gemeldet.
#[test]
fn handmade_planpro_is_imported() {
    let (v, warnings) = planpro::parse_with_warnings(HANDMADE).unwrap_or_else(|e| panic!("{e}"));
    let net = &v.topology;
    assert_eq!(net.name(), "Abzweig Musterhausen");
    let d = describe(net);
    println!("{d:#?}\n{warnings:#?}");
    assert!(d.contains("point W1 Some(Right) 60"), "{d:#?}");
    assert!(d.contains("terminal GW Boundary"));
    assert!(d.contains("terminal Pb3 BufferStop"));
    let signal = |name: &str| net.signals().find(|(_, s)| s.name == name).unwrap().1;
    assert_eq!(signal("A").kind, SignalKind::Combined);
    assert_eq!(signal("a").kind, SignalKind::Distant);
    assert_eq!(signal("F").system, SignalSystem::Hv);
    assert_eq!(net.signals().count(), 4);
    assert!(net.level_crossings().any(|(_, l)| l.name == "BÜ Feldweg"));
    let tvds: BTreeSet<&str> = net.tvds().map(|(_, t)| t.name.as_str()).collect();
    assert_eq!(tvds, BTreeSet::from(["TW", "TW1", "TO", "X"]));
    let has = |w: &str| warnings.iter().any(|x| x.contains(w));
    assert!(has("Gleissperre") && has("fiktives Signal"), "{warnings:?}");
    // Die Planung lässt sich projektieren.
    assert!(railit_import::parse("abzweig.ppxml", HANDMADE).is_ok());
}

/// REQ-TOP-015: Fehlende Angaben sind Fehler mit Fundstelle; nichts wird geraten.
#[test]
fn errors_are_collected_with_location() {
    let broken = HANDMADE
        .replace("<Wirkrichtung><Wert>gegen</Wert></Wirkrichtung>", "")
        .replace(
            "<Signalsystem><Wert>HV</Wert></Signalsystem>",
            "<Signalsystem><Wert>Hl</Wert></Signalsystem>",
        )
        .replace("<TOP_Laenge><Wert>400.000</Wert></TOP_Laenge>", "");
    let ImportError::Invalid { errors, .. } = planpro::parse(&broken).unwrap_err() else {
        panic!("Fehler erwartet");
    };
    let text = errors.iter().map(ToString::to_string).collect::<Vec<_>>();
    let has = |a: &str, b: &str| text.iter().any(|e| e.contains(a) && e.contains(b));
    assert!(has("„F“", "Hl"), "{text:?}");
    assert!(
        has("7C000000-0000-4000-8000-0000000000B3", "TOP_Laenge"),
        "{text:?}"
    );
    assert!(matches!(planpro::parse("<x"), Err(ImportError::Syntax(_))));
    assert!(matches!(
        planpro::parse("<railML/>"),
        Err(ImportError::Syntax(_))
    ));
}

#[test]
fn format_follows_the_file_extension() {
    assert_eq!(
        railit_import::Format::of_path("anlage.PPXML"),
        railit_import::Format::PlanPro
    );
    assert_eq!(
        railit_import::Format::of_path("anlage.planpro"),
        railit_import::Format::PlanPro
    );
}
