//! Spurplan der Beispielnetze (REQ-UI-001).

use railit_layout::{LANE, Layout, layout, to_svg};

const BEISPIELSTADT: &str = include_str!("../../../examples/beispielstadt.toml");
const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");
const ABZWEIG: &str = include_str!("../../railit-import/tests/data/abzweig.railml");

fn of(source: &str) -> Layout {
    layout(&railit_import::toml_format::parse(source).unwrap().topology)
}

fn track<'a>(l: &'a Layout, name: &str) -> &'a railit_layout::TrackShape {
    l.tracks.iter().find(|t| t.name == name).unwrap()
}

fn signal<'a>(l: &'a Layout, name: &str) -> &'a railit_layout::SignalShape {
    l.signals.iter().find(|s| s.name == name).unwrap()
}

#[test]
fn beispielstadt_is_a_station_with_two_tracks() {
    let l = of(BEISPIELSTADT);
    let west = track(&l, "west");
    let one = track(&l, "1");
    let two = track(&l, "2");
    let east = track(&l, "east");
    // Von West nach Ost, Gleis 1 auf der Hauptspur, Gleis 2 eine Spur darunter.
    assert!(west.line[0][0] < one.line[0][0] && one.line[0][0] < east.line[0][0]);
    assert_eq!(west.line[0][1], one.line[0][1]);
    assert_eq!(two.line[0][1], one.line[0][1] + LANE);
    assert_eq!(
        one.line[0][0], two.line[0][0],
        "beide Bahnsteiggleise beginnen gleich"
    );
    assert_eq!(one.line[1][0], two.line[1][0], "und enden gleich");
    // Weichen verbinden die Gleise lückenlos.
    let w1 = l.points.iter().find(|p| p.name == "W1").unwrap();
    assert_eq!(w1.tip, *west.line.last().unwrap());
    assert!(w1.left == one.line[0] || w1.right == one.line[0]);
    assert!(w1.left == two.line[0] || w1.right == two.line[0]);
    // Signale zeigen in ihre Wirkrichtung.
    assert_eq!(signal(&l, "A").facing, "right");
    assert_eq!(signal(&l, "F").facing, "left");
    assert_eq!(signal(&l, "N2").at[1], two.line[0][1]);
    assert_eq!(l.level_crossings.len(), 1);
    // Alle Elemente liegen im Bild.
    for t in &l.tracks {
        for p in &t.line {
            assert!(
                p[0] >= 0 && p[0] <= l.width && p[1] >= 0 && p[1] <= l.height,
                "{t:?}"
            );
        }
    }
}

#[test]
fn nordbahn_has_three_areas_from_west_to_east() {
    let l = of(NORDBAHN);
    let names: Vec<_> = l.areas.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Adorf", "Strecke", "Bstadt"]);
    let adorf = &l.areas[0];
    let strecke = &l.areas[1];
    let bstadt = &l.areas[2];
    assert!(adorf.x1 <= strecke.x0 && strecke.x1 <= bstadt.x0);
    assert_eq!(l.borders.len(), 2);
    assert_eq!(
        (l.borders[0].left.as_str(), l.borders[0].right.as_str()),
        ("Adorf", "Strecke")
    );
    assert_eq!(l.stations.len(), 2);
    assert!(l.stations.iter().all(|s| s.at[1] > 0));
    assert_eq!(signal(&l, "Sbk1").facing, "right");
    assert_eq!(signal(&l, "Sbk2").facing, "left");
}

#[test]
fn railml_branch_and_svg() {
    let net = railit_import::railml::parse(ABZWEIG).unwrap().topology;
    let l = layout(&net);
    assert_eq!(l.points.len(), 1);
    let svg = to_svg(&l);
    assert!(svg.starts_with("<svg") && svg.contains("W1"));
    let json = serde_json::to_value(&l).unwrap();
    assert!(json["tracks"].as_array().unwrap().len() >= 4);
}
