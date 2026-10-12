//! Stellbereiche, Betriebsstellen und Teilnetze (REQ-TOP-011 … REQ-TOP-013).

use railit_import::{ImportError, toml_format};
use railit_topology::{NodeRef, Port, PortRef, TerminalKind};

const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");

fn messages(err: ImportError) -> Vec<String> {
    match err {
        ImportError::Invalid { errors, .. } => errors.into_iter().map(|d| d.message).collect(),
        other @ ImportError::Syntax(_) => panic!("{other:?}"),
    }
}

#[test]
fn nordbahn_has_three_areas_and_two_links() {
    let net = toml_format::parse(NORDBAHN).unwrap().topology;
    let names: Vec<&str> = net.areas().map(|(_, a)| a.name.as_str()).collect();
    assert_eq!(names, ["Adorf", "Strecke", "Bstadt"]);
    let links = net.area_links();
    assert_eq!(links.len(), 2);
    let adorf = net.find_area("Adorf").unwrap();
    let strecke = net.find_area("Strecke").unwrap();
    assert!(
        links
            .iter()
            .any(|l| (l.a, l.b) == (adorf, strecke) || (l.a, l.b) == (strecke, adorf))
    );

    let station = net.station(net.find_station("Bstadt").unwrap()).unwrap();
    assert_eq!(station.platforms.len(), 2);
    let sbk1 = net.signals().find(|(_, s)| s.name == "Sbk1").unwrap().1;
    assert!(sbk1.auto);
}

#[test]
fn area_cut_replaces_neighbours_by_borders() {
    let net = toml_format::parse(NORDBAHN).unwrap().topology;
    let strecke = net.find_area("Strecke").unwrap();
    let cut = net.area_cut(strecke).unwrap();
    let sub = &cut.topology;
    assert_eq!(sub.tracks().len(), 2);
    assert_eq!(sub.tvds().len(), 2);
    assert_eq!(sub.signals().len(), 2);
    assert_eq!(cut.borders.len(), 2);

    let border = |name: &str| {
        let (id, t) = sub.terminals().find(|(_, t)| t.name == name).unwrap();
        (id, t.kind)
    };
    // Adorf (ID 0) beginnt mit der Erlaubnis zwischen Adorf und Strecke, die Strecke (ID 1)
    // zwischen Strecke und Bstadt.
    assert_eq!(border(">Adorf").1, TerminalKind::Border { initial: false });
    assert_eq!(border(">Bstadt").1, TerminalKind::Border { initial: true });

    // Die Grenze schließt an S1.a an; das ist im Gesamtnetz Ao.b ↔ S1.a.
    let (term, _) = border(">Adorf");
    let inside = sub
        .linked(PortRef::new(NodeRef::Terminal(term), Port::End))
        .unwrap();
    let NodeRef::Track(local_s1) = inside.node else {
        panic!()
    };
    assert_eq!(sub.track(local_s1).unwrap().name, "S1");
    let b = cut.borders.iter().find(|b| b.terminal == term).unwrap();
    assert_eq!(b.neighbour, net.find_area("Adorf").unwrap());
    let global_s1 = cut.tracks[local_s1.index().unwrap()];
    assert_eq!(b.inside, PortRef::new(NodeRef::Track(global_s1), Port::A));
    let NodeRef::Track(ao) = b.outside.node else {
        panic!()
    };
    assert_eq!(net.track(ao).unwrap().name, "Ao");

    // Abbildung der Signale zurück auf das Gesamtnetz.
    for (local, s) in sub.signals() {
        let global = cut.signals[local.index().unwrap()];
        assert_eq!(net.signal(global).unwrap().name, s.name);
    }

    let adorf = net.area_cut(net.find_area("Adorf").unwrap()).unwrap();
    assert_eq!(adorf.borders.len(), 1);
    assert!(
        adorf
            .topology
            .terminals()
            .any(|(_, t)| t.name == ">Strecke" && t.kind == TerminalKind::Border { initial: true })
    );
    assert!(adorf.topology.terminals().any(|(_, t)| t.name == "GW"));
}

#[test]
fn every_tvd_needs_exactly_one_area() {
    let missing = NORDBAHN.replace(r#"tvds = ["ST1", "ST2"]"#, r#"tvds = ["ST1"]"#);
    let errors = messages(toml_format::parse(&missing).unwrap_err());
    assert!(
        errors
            .iter()
            .any(|m| m.contains("„ST2“ gehört zu keinem Stellbereich")),
        "{errors:?}"
    );

    let twice = NORDBAHN.replace(
        r#"tvds = ["ST1", "ST2"]"#,
        r#"tvds = ["ST1", "ST2", "ATO"]"#,
    );
    let errors = messages(toml_format::parse(&twice).unwrap_err());
    assert!(
        errors.iter().any(|m| m.contains("mehreren Stellbereichen")),
        "{errors:?}"
    );
}

#[test]
fn platforms_need_route_signals() {
    let bad = NORDBAHN.replace(
        r#"{ name = "1", signals = ["AN1", "AP1"] }"#,
        r#"{ name = "1", signals = ["aA"] }"#,
    );
    let errors = messages(toml_format::parse(&bad).unwrap_err());
    assert!(
        errors.iter().any(|m| m.contains("keine Fahrstraße endet")),
        "{errors:?}"
    );
}

#[test]
fn border_terminals_can_be_written_by_hand() {
    let src = r#"
[network]
name = "Teil"
[[terminal]]
name = ">Nachbar"
kind = "border"
initial = true
[[terminal]]
name = "E"
kind = "buffer_stop"
[[track]]
name = "1"
length_m = 100
[[link]]
from = ">Nachbar.end"
to = "1.a"
[[link]]
from = "1.b"
to = "E.end"
[[tvd]]
name = "T"
elements = ["1"]
"#;
    let net = toml_format::parse(src).unwrap().topology;
    assert!(
        net.terminals()
            .any(|(_, t)| t.kind == TerminalKind::Border { initial: true })
    );
    let bad = src.replace("kind = \"border\"", "kind = \"boundary\"");
    let errors = messages(toml_format::parse(&bad).unwrap_err());
    assert!(errors.iter().any(|m| m.contains("initial")), "{errors:?}");
}
