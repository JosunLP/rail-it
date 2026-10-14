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
    let journal = railit_fuzz::example_journal();
    assert!(journal.len() > 1000, "Beispieljournal");
    railit_fuzz::journal(&journal);
    railit_fuzz::rasta(&[]);
    railit_fuzz::sci(&[]);
    railit_fuzz::sci_ils(&[]);
    railit_fuzz::node_wire(&[]);
    railit_fuzz::mirror_state(&[]);
    railit_fuzz::field_server(&[]);
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
        railit_fuzz::sci_ils(&data);
        railit_fuzz::node_wire(&data);
        railit_fuzz::mirror_state(&data);
        railit_fuzz::field_server(&data);
    }
}

proptest! {
    /// Verfälschte gültige SCI-ILS-Telegramme: Gelesen wird nur, was genau so kodiert.
    #[test]
    fn mutated_ils_telegrams(
        flips in proptest::collection::vec((any::<proptest::sample::Index>(), any::<u8>()), 0..4),
        seq in any::<u32>(),
        flags in 0u8..32,
        cut in 0usize..3,
    ) {
        use railit_adapters::ils::IlsTelegram;
        let t = IlsTelegram {
            sender: "Adorf".into(),
            receiver: "Strecke".into(),
            border: 1,
            message: railit_kernel::BorderMessage {
                holding: flags & 1 != 0,
                seq,
                offer: flags & 2 != 0,
                approach_clear: flags & 4 != 0,
                request: flags & 8 != 0,
                exclusive: flags & 16 != 0,
            },
        };
        let mut raw = t.encode().unwrap();
        for (i, b) in flips {
            let at = i.index(raw.len());
            raw[at] ^= b;
        }
        raw.truncate(raw.len() - cut);
        railit_fuzz::sci_ils(&raw);
    }

    /// Verfälschte echte Journale (einzelne Bytes, abgeschnitten): Prüfen, Abspielen und
    /// Mitlesen melden höchstens Fehler (REQ-SAF-007, REQ-JRN-009).
    #[test]
    fn mutated_journals(
        flips in proptest::collection::vec((any::<proptest::sample::Index>(), 1u8..=255), 0..4),
        cut in any::<proptest::sample::Index>(),
    ) {
        let mut journal = railit_fuzz::example_journal();
        for (i, b) in flips {
            let at = i.index(journal.len());
            journal[at] ^= b;
        }
        let end = cut.index(journal.len() + 1);
        journal.truncate(end.max(1));
        railit_fuzz::journal(&journal);
    }
}

/// Anzahl der Zahlenwerte eines JSON-Werts.
fn count_numbers(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Number(_) => 1,
        serde_json::Value::Array(items) => items.iter().map(count_numbers).sum(),
        serde_json::Value::Object(map) => map.values().map(count_numbers).sum(),
        _ => 0,
    }
}

/// Ändert den `n`-ten Zahlenwert eines JSON-Werts (Tiefensuche) auf `value`.
fn set_nth_number(v: &mut serde_json::Value, n: &mut usize, value: u64) -> bool {
    match v {
        serde_json::Value::Number(_) => {
            if *n == 0 {
                *v = value.into();
                return true;
            }
            *n -= 1;
            false
        }
        serde_json::Value::Array(items) => items.iter_mut().any(|x| set_nth_number(x, n, value)),
        serde_json::Value::Object(map) => map.values_mut().any(|x| set_nth_number(x, n, value)),
        _ => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Zustandsabzüge mit verfälschtem Zustand, die Kette neu berechnet (wie ein Angreifer
    /// mit Schreibzugriff ohne Siegel): Replay, Zeitfenster und Mitleser übernehmen nichts
    /// Ungeprüftes und laufen in keine Panik; ein verfälschter Abzug fällt als Abweichung
    /// auf, sofern er nicht zufällig gleich bleibt.
    #[test]
    fn rechained_snapshot_mutations(
        pick in any::<proptest::sample::Index>(),
        field in 0usize..400,
        value in prop_oneof![Just(0u64), Just(1), Just(255), Just(65_535), Just(u64::from(u32::MAX)), any::<u64>()],
    ) {
        use railit_journal::{Entry, JournalWriter};
        let bytes = railit_fuzz::example_journal();
        let mut entries = railit_journal::entries(bytes.as_slice()).unwrap();
        let snapshots: Vec<usize> = (0..entries.len())
            .filter(|i| matches!(entries[*i], Entry::Snapshot(_)))
            .collect();
        prop_assume!(!snapshots.is_empty());
        let at = snapshots[pick.index(snapshots.len())];
        let Entry::Snapshot(snap) = &entries[at] else { unreachable!() };
        let mut value_json = serde_json::to_value(&snap.state).unwrap();
        let numbers = count_numbers(&value_json);
        prop_assume!(numbers > 0);
        let mut n = field % numbers;
        prop_assert!(set_nth_number(&mut value_json, &mut n, value));
        let Ok(state) = serde_json::from_value(value_json) else {
            return Ok(());
        };
        let changed = state != snap.state;
        entries[at] = Entry::Snapshot(Box::new(railit_journal::Snapshot { cycle: snap.cycle, state }));
        let Entry::Header(header) = entries[0].clone() else { unreachable!() };
        let mut w = JournalWriter::new(Vec::new(), header).unwrap();
        for e in &entries[1..] {
            w.append(e).unwrap();
        }
        let forged = w.into_inner();
        railit_fuzz::journal(&forged);
        if changed {
            prop_assert!(railit_journal::replay(forged.as_slice(), railit_fuzz::example_plan_view()).is_err());
        }
    }
}
