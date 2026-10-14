//! Kreuzungsweichen und gekoppelte Weichen (REQ-TOP-017, REQ-PLN-021).

#![allow(clippy::many_single_char_names)]

use railit_plan::{PlanView, Protection};
use railit_planning::{PlanningError, compile, generate, verify};
use railit_topology::{
    Length, PointPosition, SlipKind, Speed, TerminalKind, Topology, TopologyBuilder, TopologyError,
};

const DKW: &str = include_str!("../../../examples/dkw.toml");
const CROSSOVER: &str = include_str!("../../railit-import/tests/data/weichenverbindung.toml");

fn net(source: &str) -> Topology {
    railit_import::toml_format::parse(source)
        .unwrap_or_else(|e| panic!("{e}"))
        .topology
}

fn point(net: &Topology, name: &str) -> railit_types::PointId {
    match net.lookup(name) {
        Some(railit_topology::ElementRef::Point(p)) => p,
        _ => panic!("{name}"),
    }
}

fn signal(net: &Topology, name: &str) -> railit_types::SignalId {
    match net.lookup(name) {
        Some(railit_topology::ElementRef::Signal(s)) => s,
        _ => panic!("{name}"),
    }
}

/// REQ-TOP-017: Aufbau einer DKW aus vier gekoppelten Zungenpaaren, vier Gleisen, einer
/// Kreuzung und einem Abschnitt.
#[test]
fn double_slip_is_built_from_coupled_points() {
    let n = net(DKW);
    let slip = n.slips().next().unwrap();
    assert_eq!(slip.kind, SlipKind::Double);
    assert_eq!(slip.points.len(), 4);
    assert_eq!(slip.tracks.len(), 4);
    assert_eq!(n.couplings().len(), 2);
    let a = point(&n, "DKW1a");
    let b = point(&n, "DKW1b");
    let c = point(&n, "DKW1c");
    let d = point(&n, "DKW1d");
    // Gegenläufig je Ende.
    assert_eq!(
        n.coupled(a, PointPosition::Left),
        Some((b, PointPosition::Right))
    );
    assert_eq!(
        n.coupled(d, PointPosition::Right),
        Some((c, PointPosition::Left))
    );
    assert_eq!(n.crossings().next().unwrap().name, "DKW1.x");
    let tvd = n.tvd_of(railit_topology::TvdMember::Point(a)).unwrap();
    assert_eq!(n.tvd(tvd).unwrap().name, "DKW1");
}

/// REQ-PLN-021: Jede Stellung der Antriebe gibt genau einen Weg frei; jede Fahrstraße verlangt
/// alle vier Zungenpaare, und der Flankenschutz reicht über die DKW hinaus zu den Signalen vor
/// den anderen Enden.
#[test]
fn routes_over_a_double_slip_require_all_four_points() {
    let n = net(DKW);
    let plan = generate(&n).unwrap();
    verify(&n, &plan).unwrap();
    let over: Vec<_> = plan
        .routes
        .iter()
        .filter(|r| {
            r.tvds
                .iter()
                .chain(&r.overlap_tvds)
                .any(|t| n.tvd(*t).unwrap().name == "DKW1")
        })
        .collect();
    // A1, A2, P1, P3 je zwei Wege, F1-P1 und F3-P3 mit Durchrutschweg über die DKW.
    assert_eq!(over.len(), 10);
    for r in &over {
        let mut all: Vec<_> = r
            .points
            .iter()
            .chain(&r.overlap_points)
            .map(|(p, _)| *p)
            .collect();
        all.extend(r.flank.iter().filter_map(|f| match f.protection {
            Protection::Point(p, _) => Some(p),
            Protection::Signal(_) => None,
        }));
        all.sort_unstable();
        assert_eq!(all.len(), 4, "{}", r.name);
    }
    let a1_n1 = plan.routes.iter().find(|r| r.name == "A1-N1").unwrap();
    let signals: Vec<_> = a1_n1
        .flank
        .iter()
        .filter_map(|f| match f.protection {
            Protection::Signal(s) => Some(s),
            Protection::Point(..) => None,
        })
        .collect();
    assert_eq!(signals, [signal(&n, "P3"), signal(&n, "A2")]);
    let a1_n3 = plan.routes.iter().find(|r| r.name == "A1-N3").unwrap();
    assert_eq!(a1_n3.speed, Some(Speed::from_kmh(40)));
    assert!(
        a1_n1.conflicts.contains(
            &railit_types::RouteId::from_index(
                plan.routes.iter().position(|r| r.name == "A2-N3").unwrap()
            )
            .unwrap()
        )
    );
    // Plan-Image: Kopplungen symmetrisch, Partner verlangt.
    let compiled = compile(&n, None).unwrap();
    let view = PlanView::parse(&compiled.image).unwrap();
    assert_eq!(
        view.point_coupling(point(&n, "DKW1c")),
        Some((point(&n, "DKW1d"), true))
    );
}

/// REQ-PLN-021: Bei der EKW gibt es keinen Bogen b–c; ihre Zungenpaare laufen gleichläufig.
#[test]
fn single_slip_has_one_curve() {
    let n = net(&DKW.replace("kind = \"double\"", "kind = \"single\""));
    assert_eq!(n.couplings().len(), 1);
    let plan = generate(&n).unwrap();
    verify(&n, &plan).unwrap();
    let names: Vec<_> = plan.routes.iter().map(|r| r.name.as_str()).collect();
    for want in ["A1-N1", "A1-N3", "A2-N3", "P1-GW1", "P3-GW2", "P3-GW1"] {
        assert!(names.contains(&want), "{want} fehlt in {names:?}");
    }
    assert!(
        !names.contains(&"A2-N1") && !names.contains(&"P1-GW2"),
        "{names:?}"
    );
}

/// REQ-PLN-021: Eine Weichenverbindung an einem Antrieb: Die Fahrt geradeaus über W1 legt
/// W2 mit; die Fahrt über die Verbindung verlangt beide abzweigend.
#[test]
fn crossover_points_move_together() {
    let n = net(CROSSOVER);
    let plan = generate(&n).unwrap();
    verify(&n, &plan).unwrap();
    let w1 = point(&n, "W1");
    let w2 = point(&n, "W2");
    let straight = plan.routes.iter().find(|r| r.name == "A-G2").unwrap();
    assert_eq!(straight.points, [(w1, PointPosition::Left)]);
    assert!(
        straight
            .flank
            .iter()
            .any(|f| f.protection == Protection::Point(w2, PointPosition::Left))
    );
    let over = plan.routes.iter().find(|r| r.name == "A-G4").unwrap();
    assert_eq!(
        over.points,
        [(w1, PointPosition::Right), (w2, PointPosition::Right)]
    );
    // B-G4 legt W1 geradeaus mit und ist damit der Fahrt über die Verbindung feind.
    let b = plan.routes.iter().position(|r| r.name == "B-G4").unwrap();
    assert!(over.conflicts.iter().any(|c| c.index() == Some(b)));
}

/// REQ-PLN-021: Plan-Images, in denen eine Fahrstraße die Partnerweiche nicht verlangt oder
/// die Kopplung nicht symmetrisch ist, lädt kein Kern; die Gegenprüfung findet beides.
#[test]
fn tampered_couplings_are_rejected() {
    let n = net(DKW);
    let plan = generate(&n).unwrap();

    let mut no_partner = plan.clone();
    let r = no_partner
        .routes
        .iter_mut()
        .find(|r| r.name == "A1-N1")
        .unwrap();
    r.flank
        .retain(|f| !matches!(f.protection, Protection::Point(..)));
    let err = no_partner.to_image().unwrap_err().to_string();
    assert!(err.contains("Kopplung"), "{err}");
    assert!(verify(&n, &no_partner).is_err());

    let mut asymmetric = plan.clone();
    asymmetric.points[0].coupled = None;
    let err = asymmetric.to_image().unwrap_err().to_string();
    assert!(err.contains("Kopplung"), "{err}");
    let err = verify(&n, &asymmetric).unwrap_err();
    assert!(
        err.errors
            .iter()
            .any(|e| matches!(e, PlanningError::Verification(m) if m.contains("Kopplung"))),
        "{err}"
    );
}

/// REQ-TOP-017: Kopplungen nur zwischen zwei verschiedenen, noch nicht gekoppelten Weichen;
/// gekoppelte Weichen gehören demselben Stellbereich an.
#[test]
fn invalid_couplings_are_rejected() {
    let mut b = TopologyBuilder::new("K");
    let p = b.add_point("P").unwrap();
    let q = b.add_point("Q").unwrap();
    let r = b.add_point("R").unwrap();
    assert!(matches!(
        b.couple_points(p, p, false),
        Err(TopologyError::InvalidCoupling { .. })
    ));
    b.couple_points(p, q, false).unwrap();
    let err = b.couple_points(q, r, true).unwrap_err();
    assert_eq!(err.requirement(), "REQ-TOP-017");

    let split = CROSSOVER.to_string()
        + "\n[[area]]\nname = \"West\"\ntvds = [\"TA1\", \"TB1\", \"TW1\"]\n\n[[area]]\nname = \"Ost\"\ntvds = [\"TA2\", \"TB2\", \"TW2\"]\n";
    let err = railit_import::toml_format::parse(&split)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("REQ-TOP-017") && err.contains("Stellbereichen"),
        "{err}"
    );
}

/// REQ-TOP-017: Kreuzungsweichen über den Builder; Namen dürfen nicht doppelt vorkommen.
#[test]
fn slip_names_are_checked_first() {
    let mut b = TopologyBuilder::new("S");
    b.add_track("K.1", Length::from_mm(1000)).unwrap();
    let err = b
        .add_slip(
            "K",
            SlipKind::Double,
            Length::from_mm(30_000),
            Speed::from_kmh(40),
        )
        .unwrap_err();
    assert!(matches!(err, TopologyError::DuplicateName { .. }));
    // Nichts halb angelegt.
    assert!(b.lookup("Ka").is_none());
    let t = b.add_terminal("E", TerminalKind::BufferStop).unwrap();
    let _ = t;
}

/// REQ-PLN-021: Gekoppelte Weichen in einem Annäherungsabschnitt an einer Bereichsgrenze
/// projektiert rail-it nicht.
#[test]
fn coupled_points_must_not_lie_in_an_approach() {
    let border = CROSSOVER.replace(
        "name = \"G1\"\nkind = \"boundary\"",
        "name = \"G1\"\nkind = \"border\"",
    );
    // A steht in Richtung W1; der Annäherungsabschnitt von G1 führt bis A und dessen
    // Durchrutschweg über W1.
    let n = net(&border);
    let err = generate(&n).unwrap_err();
    assert!(
        err.errors
            .iter()
            .any(|e| e.to_string().contains("gekoppelte Weiche")),
        "{err}"
    );
}
