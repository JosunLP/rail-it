//! rail-it-TOML-Import.

use railit_import::{ImportError, toml_format};
use railit_topology::{ElementRef, Length, PointPosition, SignalKind, SignalSystem, Speed};

const EXAMPLE: &str = include_str!("../../../examples/beispielstadt.toml");

#[test]
fn example_network_imports() {
    let v = toml_format::parse(EXAMPLE).unwrap();
    assert!(v.warnings.is_empty());
    let net = v.topology;
    assert_eq!(net.name(), "Beispielstadt");
    assert_eq!(
        (
            net.tracks().len(),
            net.points().len(),
            net.signals().len(),
            net.tvds().len()
        ),
        (4, 2, 8, 6)
    );
    assert_eq!(net.level_crossings().len(), 1);
    let Some(ElementRef::Signal(a)) = net.lookup("A") else {
        panic!("Signal A fehlt")
    };
    let a = net.signal(a).unwrap();
    assert_eq!(a.position, Length::from_m(1000).unwrap());
    assert_eq!((a.kind, a.system), (SignalKind::Combined, SignalSystem::Ks));
    let Some(ElementRef::Signal(f)) = net.lookup("F") else {
        panic!("Signal F fehlt")
    };
    assert_eq!(net.signal(f).unwrap().system, SignalSystem::Hv);
    let (_, w1) = net.points().next().unwrap();
    assert_eq!(
        (w1.diverging, w1.diverging_speed),
        (Some(PointPosition::Right), Speed::from_kmh(40))
    );
    let (_, bue) = net.level_crossings().next().unwrap();
    assert_eq!(
        (bue.name.as_str(), bue.position),
        ("BÜ1", Length::from_m(500).unwrap())
    );
}

#[test]
fn diverging_speed_requires_diverging_branch() {
    let src = "[network]\nname = \"n\"\n[[point]]\nname = \"W\"\ndiverging_speed_kmh = 60\n";
    let ImportError::Invalid { errors, .. } = toml_format::parse(src).unwrap_err() else {
        panic!("Validierungsfehler erwartet")
    };
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("diverging_speed_kmh ohne diverging"))
    );
}

#[test]
fn decimal_metres_are_rounded_to_millimetres() {
    let src = r#"
        [network]
        name = "n"
        [[terminal]]
        name = "E1"
        kind = "buffer_stop"
        [[terminal]]
        name = "E2"
        kind = "buffer_stop"
        [[track]]
        name = "1"
        length_m = 12.3456
        [[link]]
        from = "E1.end"
        to = "1.a"
        [[link]]
        from = "1.b"
        to = "E2.end"
        [[tvd]]
        name = "T"
        elements = ["1"]
    "#;
    let net = toml_format::parse(src).unwrap().topology;
    let (_, t) = net.tracks().next().unwrap();
    assert_eq!(t.length, Length::from_mm(12_346));
}

#[test]
fn syntax_and_unknown_fields_are_reported() {
    assert!(matches!(
        toml_format::parse("[network"),
        Err(ImportError::Syntax(_))
    ));
    let err = toml_format::parse("[network]\nname = \"n\"\ncolour = \"rot\"").unwrap_err();
    let ImportError::Syntax(msg) = err else {
        panic!("Syntaxfehler erwartet")
    };
    assert!(msg.contains("colour"), "{msg}");
}

#[test]
fn all_errors_are_collected_with_context() {
    let src = r#"
        [network]
        name = "kaputt"
        [[track]]
        name = "1"
        length_m = -5
        [[track]]
        name = "2"
        length_m = 100
        [[track]]
        name = "2"
        length_m = 100
        [[point]]
        name = "W"
        [[terminal]]
        name = "E"
        kind = "boundary"
        [[link]]
        from = "2.x"
        to = "W.tip"
        [[link]]
        from = "2.a"
        to = "nirgends.end"
        [[signal]]
        name = "S"
        track = "2"
        position_m = 150
        direction = "a_to_b"
        kind = "main"
        [[tvd]]
        name = "T"
        elements = ["2", "E", "S"]
    "#;
    let ImportError::Invalid { errors, .. } = toml_format::parse(src).unwrap_err() else {
        panic!("Validierungsfehler erwartet")
    };
    let text: Vec<String> = errors.iter().map(ToString::to_string).collect();
    let expect = [
        "track „1“: length_m = -5 ist keine gültige Meterangabe",
        "[REQ-TOP-001] track „2“: Name „2“ ist mehrfach vergeben",
        "link[0] (2.x – W.tip): „x“ ist kein Port",
        "link[1] (2.a – nirgends.end): unbekanntes Element „nirgends“",
        "[REQ-TOP-005] signal „S“: „S“ liegt bei 150 m, Gleis „2“ ist aber nur 100 m lang",
        "tvd „T“: „E“ ist weder Gleis noch Weiche",
        "tvd „T“: unbekanntes Element „S“",
        "[REQ-TOP-002] Port „2.a“ ist offen",
        "[REQ-TOP-006] „2“ gehört zu keinem Gleisfreimeldeabschnitt",
    ];
    for e in expect {
        assert!(
            text.iter().any(|t| t.starts_with(e)),
            "fehlt: {e}\nvorhanden: {text:#?}"
        );
    }
}

const KREUZUNG: &str = include_str!("../../../examples/kreuzung.toml");

/// REQ-TOP-016: Beide Kreuzungsstellen liegen auf verschiedenen Gleisen im selben Abschnitt.
#[test]
fn crossing_legs_share_one_tvd() {
    let net = railit_import::toml_format::parse(KREUZUNG)
        .unwrap()
        .topology;
    let c = net.crossings().next().unwrap();
    assert_eq!(c.name, "Kr1");
    let kw = net.lookup("kw");
    assert!(
        matches!(kw, Some(railit_topology::ElementRef::Track(t)) if net.crossings_on(t).count() == 1)
    );

    let split = KREUZUNG.replace(
        "name = \"TKr\"\nelements = [\"kw\", \"kn\"]",
        "name = \"TKr\"\nelements = [\"kw\"]\n\n[[tvd]]\nname = \"TKr2\"\nelements = [\"kn\"]",
    );
    let err = railit_import::toml_format::parse(&split)
        .unwrap_err()
        .to_string();
    assert!(err.contains("REQ-TOP-016") && err.contains("TKr2"), "{err}");

    let same = KREUZUNG.replace(
        "{ track = \"kn\", position_m = 20 }",
        "{ track = \"kw\", position_m = 30 }",
    );
    let err = railit_import::toml_format::parse(&same)
        .unwrap_err()
        .to_string();
    assert!(err.contains("REQ-TOP-016"), "{err}");

    let outside = KREUZUNG.replace(
        "{ track = \"kn\", position_m = 20 }",
        "{ track = \"kn\", position_m = 41 }",
    );
    let err = railit_import::toml_format::parse(&outside)
        .unwrap_err()
        .to_string();
    assert!(err.contains("Kr1") && err.contains("kn"), "{err}");

    let dup = KREUZUNG.replace("name = \"Kr1\"", "name = \"A\"");
    assert!(railit_import::toml_format::parse(&dup).is_err());
}
