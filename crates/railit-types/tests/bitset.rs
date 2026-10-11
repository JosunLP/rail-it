//! Eigenschaften der Bitmengen, geprüft gegen ein einfaches Referenzmodell (`BTreeSet`).

use std::collections::BTreeSet;

use proptest::prelude::*;
use railit_types::bitset::{BitSet, BitSetMut, OutOfRange, words_for};

const BITS: usize = 200;

fn model_to_words(model: &BTreeSet<usize>) -> Vec<u64> {
    let mut words = vec![0u64; words_for(BITS)];
    let mut set = BitSetMut::new(&mut words);
    for &i in model {
        set.insert(i).unwrap();
    }
    words
}

fn arb_model() -> impl Strategy<Value = BTreeSet<usize>> {
    prop::collection::btree_set(0..BITS, 0..40)
}

proptest! {
    #[test]
    fn iter_and_contains_match_model(model in arb_model()) {
        let words = model_to_words(&model);
        let set = BitSet::new(&words);
        prop_assert_eq!(set.iter().collect::<BTreeSet<_>>(), model.clone());
        prop_assert_eq!(set.count(), model.len());
        prop_assert_eq!(set.is_empty(), model.is_empty());
        for i in 0..set.capacity() + 10 {
            prop_assert_eq!(set.contains(i), model.contains(&i));
        }
    }

    #[test]
    fn intersects_matches_model(a in arb_model(), b in arb_model()) {
        let (wa, wb) = (model_to_words(&a), model_to_words(&b));
        prop_assert_eq!(
            BitSet::new(&wa).intersects(&BitSet::new(&wb)),
            !a.is_disjoint(&b)
        );
    }

    #[test]
    fn subset_matches_model(a in arb_model(), b in arb_model()) {
        let (wa, wb) = (model_to_words(&a), model_to_words(&b));
        prop_assert_eq!(BitSet::new(&wa).is_subset_of(&BitSet::new(&wb)), a.is_subset(&b));
    }

    #[test]
    fn union_and_difference_match_model(a in arb_model(), b in arb_model()) {
        let (mut wa, wb) = (model_to_words(&a), model_to_words(&b));
        BitSetMut::new(&mut wa).union_with(&BitSet::new(&wb)).unwrap();
        prop_assert_eq!(BitSet::new(&wa).iter().collect::<BTreeSet<_>>(), &a | &b);

        let mut wa = model_to_words(&a);
        BitSetMut::new(&mut wa).difference_with(&BitSet::new(&wb));
        prop_assert_eq!(BitSet::new(&wa).iter().collect::<BTreeSet<_>>(), &a - &b);
    }
}

#[test]
fn out_of_range_is_rejected_without_change() {
    let mut words = [0u64; 2];
    let mut set = BitSetMut::new(&mut words);
    assert_eq!(
        set.insert(128),
        Err(OutOfRange {
            index: 128,
            capacity: 128
        })
    );
    assert_eq!(
        set.remove(usize::MAX),
        Err(OutOfRange {
            index: usize::MAX,
            capacity: 128
        })
    );
    assert!(set.as_set().is_empty());
}

#[test]
fn union_with_larger_set_is_rejected_without_change() {
    let mut small = [0b1u64];
    let large = [0u64, 0b100];
    let err = BitSetMut::new(&mut small)
        .union_with(&BitSet::new(&large))
        .unwrap_err();
    assert_eq!(
        err,
        OutOfRange {
            index: 66,
            capacity: 64
        }
    );
    assert_eq!(small, [0b1]);
}

#[test]
fn empty_set_behaves() {
    let set = BitSet::new(&[]);
    assert_eq!(set.capacity(), 0);
    assert!(set.is_empty());
    assert_eq!(set.iter().next(), None);
    assert!(set.is_subset_of(&BitSet::new(&[0])));
}
