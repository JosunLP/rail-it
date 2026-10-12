//! Formale Nachweise mit Kani (`cargo kani -p railit-types`), REQ-SAF-006.

#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use crate::bitset::{BitSet, BitSetMut};
use crate::time::{Duration, Tick};

/// Einfügen und Löschen wirken genau auf ein Bit innerhalb der Kapazität; außerhalb bleibt die
/// Menge unverändert und der Fehler wird gemeldet.
#[kani::proof]
#[kani::unwind(20)]
fn bitset_insert_remove_are_exact() {
    let mut words: [u64; 2] = kani::any();
    let before = words;
    let index: usize = kani::any();
    let other: usize = kani::any();
    kani::assume(other < 128 && other != index);
    let had_other = BitSet::new(&before).contains(other);

    let mut set = BitSetMut::new(&mut words);
    let inserted = set.insert(index);
    assert_eq!(inserted.is_ok(), index < 128);
    if inserted.is_ok() {
        assert!(set.as_set().contains(index));
    } else {
        let w = set.as_set().words();
        assert!(w[0] == before[0] && w[1] == before[1]);
    }
    assert_eq!(set.as_set().contains(other), had_other);

    let removed = set.remove(index);
    assert_eq!(removed.is_ok(), index < 128);
    if removed.is_ok() {
        assert!(!set.as_set().contains(index));
    }
    assert_eq!(set.as_set().contains(other), had_other);
}

/// Vereinigung: Ergebnis enthält genau die Bits beider Mengen; zu große Mengen werden
/// abgewiesen, ohne die Menge zu ändern.
#[kani::proof]
#[kani::unwind(70)]
fn bitset_union_is_exact() {
    let mut a: [u64; 1] = kani::any();
    let b: [u64; 2] = kani::any();
    let before = a;
    let i: usize = kani::any();
    kani::assume(i < 64);
    let mut set = BitSetMut::new(&mut a);
    let result = set.union_with(&BitSet::new(&b));
    assert_eq!(result.is_ok(), b[1] == 0);
    if result.is_ok() {
        assert_eq!(
            set.as_set().contains(i),
            BitSet::new(&before).contains(i) || BitSet::new(&b).contains(i)
        );
    } else {
        assert_eq!(a[0], before[0]);
    }
}

/// Zeit läuft nie über: Addition meldet den Überlauf, die vergangene Zeit sättigt bei null.
#[kani::proof]
fn time_never_overflows() {
    let t = Tick::from_millis(kani::any());
    let d = Duration::from_millis(kani::any());
    match t.checked_add(d) {
        Some(later) => {
            assert!(later >= t);
            assert_eq!(later.saturating_since(t), d);
        }
        None => assert!(t.as_millis().checked_add(d.as_millis()).is_none()),
    }
    let earlier = Tick::from_millis(kani::any());
    let since = t.saturating_since(earlier);
    if earlier > t {
        assert_eq!(since.as_millis(), 0);
    }
}
