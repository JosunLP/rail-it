//! Fuzz-Ziele auf stabilem Rust: Startkorpus, gezielte Veränderungen und Zufallseingaben
//! (REQ-SAF-007). Die eigentliche Suche läuft mit `cargo fuzz` (siehe `fuzz/`).

use proptest::prelude::*;

const BEISPIELSTADT: &str = include_str!("../../../examples/beispielstadt.toml");
const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");
const FAHRPLAN: &str = include_str!("../../../examples/nordbahn-fahrplan.toml");
const ABZWEIG: &str = include_str!("../../railit-import/tests/data/abzweig.railml");

fn images() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let net = railit_import::toml_format::parse(BEISPIELSTADT)
        .unwrap()
        .topology;
    out.push(railit_planning::compile(&net, None).unwrap().image);
    let net = railit_import::toml_format::parse(NORDBAHN)
        .unwrap()
        .topology;
    for a in railit_planning::compile_areas(&net, None, &railit_planning::PlanningConfig::default())
        .unwrap()
    {
        out.push(a.compiled.image);
    }
    out
}

#[test]
fn seeds_pass() {
    for image in images() {
        assert!(railit_fuzz::plan_image(&image));
    }
    railit_fuzz::toml_network(BEISPIELSTADT.as_bytes());
    railit_fuzz::toml_network(NORDBAHN.as_bytes());
    railit_fuzz::railml_network(ABZWEIG.as_bytes());
    railit_fuzz::timetable(FAHRPLAN.as_bytes());
    railit_fuzz::journal(b"");
    railit_fuzz::rasta(&[]);
    railit_fuzz::sci(&[]);
    railit_fuzz::node_wire(&[]);
    railit_fuzz::mirror_state(&[]);
}

/// Jedes Wort des Kopfs und jedes 16. Wort des Rumpfs wird auf typische Grenzwerte gesetzt;
/// mit neu berechnetem Hash erreicht jedes veränderte Image die Strukturprüfung.
#[test]
fn plan_mutation_sweep() {
    let (mut accepted, mut total) = (0, 0);
    for image in images().into_iter().take(2) {
        let words = image.len() / 4;
        let beyond = u32::try_from(image.len()).unwrap() + 4;
        let positions = (0..24).chain((24..words).step_by(16));
        for w in positions {
            for value in [0u32, 1, 0xff, 0xffff, u32::MAX, beyond] {
                let mut m = image.clone();
                m[w * 4..w * 4 + 4].copy_from_slice(&value.to_le_bytes());
                accepted += usize::from(railit_fuzz::plan_image(&m));
                total += 1;
            }
        }
    }
    // Viele Veränderungen (z. B. in Namen, Längen, Geschwindigkeiten) bleiben strukturell
    // gültig und erreichen so Kern, Signalbilder und Fahrerlaubnis.
    eprintln!("{accepted} von {total} veränderten Images angenommen");
    assert!(accepted * 10 > total, "{accepted} von {total}");
}

/// Abgeschnittene Netze und Fahrpläne.
#[test]
fn truncated_sources() {
    for source in [BEISPIELSTADT, NORDBAHN, FAHRPLAN, ABZWEIG] {
        for cut in (0..source.len()).step_by(97) {
            let bytes = &source.as_bytes()[..cut];
            railit_fuzz::toml_network(bytes);
            railit_fuzz::railml_network(bytes);
            railit_fuzz::timetable(bytes);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn random_plan_mutations(
        flips in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8),
        which in 0usize..4,
    ) {
        let images = images();
        let mut image = images[which % images.len()].clone();
        for (i, b) in flips {
            let at = i.index(image.len());
            image[at] ^= b;
        }
        let _ = railit_fuzz::plan_image(&image);
    }

    #[test]
    fn random_bytes(data in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = railit_fuzz::plan_image(&data);
        railit_fuzz::toml_network(&data);
        railit_fuzz::railml_network(&data);
        railit_fuzz::timetable(&data);
        railit_fuzz::journal(&data);
        railit_fuzz::rasta(&data);
        railit_fuzz::sci(&data);
        railit_fuzz::node_wire(&data);
        railit_fuzz::mirror_state(&data);
    }
}
