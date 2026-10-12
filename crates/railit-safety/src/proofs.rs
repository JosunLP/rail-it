//! Formale Nachweise mit Kani (`cargo kani -p railit-safety`), REQ-SAF-006.
//!
//! Die Nachweise decken **alle** Kombinationen der Eingaben ab (Bounded Model Checking), nicht
//! nur Stichproben.

#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use crate::channels::{Decision, MAX_CHANNELS, vote};
use crate::outputs::ChannelOutputs;
use crate::{FixedOutputs, Watchdog};
use railit_kernel::{Command, Outputs};
use railit_types::TvdId;

/// Der Voter ist vollständig spezifiziert:
///
/// - Er gibt nur Ausgaben weiter, die alle mitwählenden Kanäle (ein Kanal, 2-von-2) bzw.
///   mindestens zwei von drei Kanälen geliefert haben; der gewählte Kanal wählt mit.
/// - Nach einem Ausschluss stimmen alle verbliebenen Kanäle überein; ausgeschlossen wird nur
///   bei drei Kanälen und nur ein mitwählender Kanal.
/// - Er schaltet genau dann ab, wenn es keine solche Übereinstimmung gibt.
#[kani::proof]
fn voter_forwards_only_confirmed_outputs() {
    let results: [Option<(u8, u8)>; MAX_CHANNELS] = kani::any();
    let active: [bool; MAX_CHANNELS] = kani::any();
    let voters = active.iter().filter(|a| **a).count();
    kani::assume(voters >= 1);
    let agree = |i: usize, j: usize| results[i].is_some() && results[i] == results[j];
    let majority = (0..MAX_CHANNELS).any(|i| {
        active[i]
            && results[i].is_some()
            && (0..MAX_CHANNELS)
                .filter(|&j| active[j] && agree(i, j))
                .count()
                >= voters.min(2)
    });
    let all_agree = (0..MAX_CHANNELS).any(|i| {
        active[i] && results[i].is_some() && (0..MAX_CHANNELS).all(|j| !active[j] || agree(i, j))
    });
    let enough = if voters == MAX_CHANNELS {
        majority
    } else {
        all_agree
    };

    match vote(&results, active) {
        Decision::Agree { chosen, excluded } => {
            assert!(enough);
            assert!(active[chosen] && excluded[chosen].is_none());
            let reference = results[chosen];
            assert!(reference.is_some());
            let confirmed = (0..MAX_CHANNELS)
                .filter(|&i| active[i] && results[i] == reference)
                .count();
            assert!(confirmed >= voters.min(2));
            for i in 0..MAX_CHANNELS {
                if excluded[i].is_some() {
                    assert!(active[i] && voters == MAX_CHANNELS);
                    assert!(results[i] != reference);
                } else if active[i] {
                    assert!(results[i] == reference);
                }
            }
        }
        Decision::Trip(_) => assert!(!enough),
    }
}

/// Der Watchdog erkennt jede Fristüberschreitung und jeden zu großen Zyklusabstand und meldet
/// sonst nichts.
#[kani::proof]
fn watchdog_detects_every_overrun() {
    let deadline: u64 = kani::any();
    let gap: u64 = kani::any();
    let mut w = Watchdog::new(deadline, gap);
    let (s1, e1, s2, e2): (u64, u64, u64, u64) = kani::any();
    let first = w.check(s1, e1);
    assert_eq!(first.is_ok(), e1 >= s1 && e1 - s1 <= deadline);
    let second = w.check(s2, e2);
    assert_eq!(
        second.is_ok(),
        s2 >= s1 && s2 - s1 <= gap && e2 >= s2 && e2 - s2 <= deadline
    );
}

/// Ein Ausgabepuffer verliert keinen Befehl unbemerkt: Entweder sind alle Befehle in
/// Reihenfolge enthalten, oder er meldet den Überlauf.
#[kani::proof]
#[kani::unwind(6)]
fn fixed_outputs_never_lose_silently() {
    let n: usize = kani::any();
    kani::assume(n <= 4);
    let mut out = FixedOutputs::<2, 1>::new();
    for i in 0..n {
        out.command(Command::ResetTvd(TvdId::new(u32::try_from(i).unwrap_or(0))));
    }
    assert_eq!(out.overflowed(), n > 2);
    let mut k = 0;
    for c in out.commands() {
        assert_eq!(
            c,
            Command::ResetTvd(TvdId::new(u32::try_from(k).unwrap_or(0)))
        );
        k += 1;
    }
    assert_eq!(k, n.min(2));
    out.clear();
    assert!(!out.overflowed() && out.commands().next().is_none());
}
