//! rail-it-TOML-Import.

use railit_import::{ImportError, toml_format};
use railit_topology::{ElementRef, Length};

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
        (4, 2, 6, 6)
    );
    let Some(ElementRef::Signal(a)) = net.lookup("A") else {
        panic!("Signal A fehlt")
    };
    assert_eq!(
        net.signal(a).unwrap().position,
        Length::from_m(950).unwrap()
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
