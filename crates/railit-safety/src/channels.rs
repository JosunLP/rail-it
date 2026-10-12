//! Redundante Kanäle und Voter.

use railit_kernel::{
    Aspect, Command, Event, Input, Interlocking, KernelState, Outputs, Rejection, SafeStateReason,
};
use railit_rules::RuleSet;
use railit_types::{SignalId, Tick};
use sha2::{Digest as _, Sha256};

use crate::digest::{Digest, feed, finish, state_digest};
use crate::outputs::{ChannelOutputs, Tap};

/// Höchstzahl der Kanäle.
pub const MAX_CHANNELS: usize = 3;

/// Im sicheren Zustand werden die Haltbefehle alle so viele Zyklen wiederholt.
const RESEND_EVERY: u64 = 10;

/// Redundanz eines Stellbereichs.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Redundancy {
    /// Ein Kanal, kein Vergleich (Simulation, Ausbildung).
    #[default]
    Single,
    /// Zwei Kanäle; jede Abweichung führt in den sicheren Zustand.
    TwoOutOfTwo,
    /// Drei Kanäle mit Mehrheitsentscheid; ein abweichender Kanal wird ausgeschlossen.
    TwoOutOfThree,
}

impl Redundancy {
    /// Anzahl der Kanäle.
    pub const fn channels(self) -> usize {
        match self {
            Self::Single => 1,
            Self::TwoOutOfTwo => 2,
            Self::TwoOutOfThree => 3,
        }
    }

    /// Redundanz für `n` Kanäle.
    pub const fn from_channels(n: usize) -> Option<Self> {
        match n {
            1 => Some(Self::Single),
            2 => Some(Self::TwoOutOfTwo),
            3 => Some(Self::TwoOutOfThree),
            _ => None,
        }
    }
}

/// Worin ein Kanal von der Mehrheit abweicht.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Divergence {
    /// Ergebnis, Stellbefehle oder Ereignisse.
    Outputs,
    /// Bei gleichen Ausgaben im Zustand.
    State,
    /// Der Kanal hat kein Ergebnis geliefert.
    Missing,
}

/// Grund, aus dem die Sicherheitsschicht in den sicheren Zustand gegangen ist.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Trip {
    /// Die beiden verbliebenen Kanäle stimmen nicht überein (2-von-2).
    Disagreement(Divergence),
    /// Keine zwei von drei Kanälen stimmen überein.
    NoMajority,
    /// Der Ausgabepuffer war zu klein.
    OutputOverflow,
    /// Die Rechenzeit eines Zyklus hat die Frist überschritten.
    DeadlineExceeded {
        /// Rechenzeit in Mikrosekunden.
        elapsed_us: u64,
        /// Frist in Mikrosekunden.
        deadline_us: u64,
    },
    /// Zwei Zyklen lagen zu weit auseinander.
    CycleGap {
        /// Abstand in Mikrosekunden.
        gap_us: u64,
        /// Grenze in Mikrosekunden.
        limit_us: u64,
    },
}

/// Fehler, der zu Prüfzwecken in einen Kanal eingebracht wird (Simulation, Prüfstand).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Fault {
    /// Der Kanal verliert die Eingaben eines Zyklus (Zustand weicht ab).
    DropInputs,
    /// Der Kanal gibt in einem Zyklus zusätzlich „Fahrt“ für das erste Signal aus.
    StrayCommand,
    /// Der Kanal rechnet nicht mehr (bis zur Wiederaufnahme oder zum Rücksetzen).
    Stall,
}

/// Meldung der Sicherheitsschicht.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SafetyEvent {
    /// Ein Kanal wurde von der Mehrheit überstimmt und ausgeschlossen.
    ChannelExcluded {
        /// Kanal (0, 1, 2).
        channel: u8,
        /// Art der Abweichung.
        divergence: Divergence,
    },
    /// Übergang in den sicheren Zustand.
    Tripped(Trip),
}

/// Ergebnis eines Zyklus.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// Ergebnis des Kerns im abgestimmten Kanal (`Ok` im sicheren Zustand der Schicht).
    pub kernel: Result<(), SafeStateReason>,
    /// Sicherer Zustand der Schicht, falls aktiv.
    pub trip: Option<Trip>,
    events: [Option<SafetyEvent>; MAX_CHANNELS],
}

impl Verdict {
    const fn new() -> Self {
        Self {
            kernel: Ok(()),
            trip: None,
            events: [None; MAX_CHANNELS],
        }
    }

    fn push(&mut self, event: SafetyEvent) {
        if let Some(slot) = self.events.iter_mut().find(|e| e.is_none()) {
            *slot = Some(event);
        }
    }

    /// Meldungen dieses Zyklus.
    pub fn events(&self) -> impl Iterator<Item = SafetyEvent> + '_ {
        self.events.iter().filter_map(|e| *e)
    }
}

/// Fehler beim Zusammenstellen der Kanäle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// Kein Kanal angegeben.
    NoChannel,
    /// Kanäle müssen lückenlos ab Kanal 0 angegeben sein.
    Gap,
    /// Die Kanäle beginnen nicht mit demselben Zustand.
    StatesDiffer,
}

/// Fehler bei der Wiederaufnahme eines Kanals.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ReintegrateError {
    /// Die Schicht ist im sicheren Zustand; erst zurücksetzen.
    Tripped,
    /// Diesen Kanal gibt es nicht.
    NoSuchChannel,
    /// Der Kanal wählt bereits mit.
    AlreadyActive,
    /// Es gibt keinen gesunden Kanal, dessen Zustand übernommen werden kann.
    NoHealthyChannel,
    /// Die Zustandsspeicher sind verschieden groß.
    StateMismatch,
}

/// Speicher für den Zustand eines Kanals.
pub trait StateStore {
    /// Veränderliche Sicht auf den Zustand.
    fn state(&mut self) -> KernelState<'_>;
}

impl StateStore for KernelState<'_> {
    fn state(&mut self) -> KernelState<'_> {
        self.reborrow()
    }
}

#[cfg(feature = "alloc")]
impl StateStore for railit_kernel::OwnedState {
    fn state(&mut self) -> KernelState<'_> {
        self.view()
    }
}

/// Zustand der Sicherheitsschicht für Anzeige und Diagnose.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// Projektierte Redundanz.
    pub redundancy: Redundancy,
    /// Je Kanal: wählt mit.
    pub active: [bool; MAX_CHANNELS],
    /// Sicherer Zustand der Schicht, falls aktiv.
    pub trip: Option<Trip>,
    /// Anzahl der Zyklen seit dem Start.
    pub cycle: u64,
}

#[derive(Debug)]
struct Slot<S, O> {
    state: S,
    out: O,
    active: bool,
    stalled: bool,
    fault: Option<Fault>,
    result: Result<(), SafeStateReason>,
}

/// Ausgaben und Zustand eines Kanals in einem Zyklus.
type Fingerprint = (Digest, Digest);

/// Entscheidung des Voters.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Die Ausgaben von `chosen` gelten; abweichende Kanäle werden ausgeschlossen.
    Agree {
        chosen: usize,
        excluded: [Option<Divergence>; MAX_CHANNELS],
    },
    /// Sicherer Zustand.
    Trip(Trip),
}

fn classify<D: Eq>(other: Option<&(D, D)>, reference: &(D, D)) -> Divergence {
    match other {
        None => Divergence::Missing,
        Some((outputs, _)) if *outputs != reference.0 => Divergence::Outputs,
        Some(_) => Divergence::State,
    }
}

/// Abstimmung über die Ergebnisse der mitwählenden Kanäle (REQ-SAF-001/002).
///
/// Weitergegeben werden nur Ausgaben, die alle mitwählenden Kanäle (ein Kanal, 2-von-2) bzw.
/// mindestens zwei von drei Kanälen geliefert haben.
pub(crate) fn vote<D: Copy + Eq>(
    results: &[Option<(D, D)>; MAX_CHANNELS],
    active: [bool; MAX_CHANNELS],
) -> Decision {
    let voters = active.iter().filter(|a| **a).count();
    let agreeing = |reference: &(D, D)| {
        results
            .iter()
            .zip(&active)
            .filter(|(r, a)| **a && r.as_ref() == Some(reference))
            .count()
    };
    let mut best: Option<(usize, usize)> = None;
    for (index, (result, is_active)) in results.iter().zip(&active).enumerate() {
        let (true, Some(reference)) = (*is_active, result) else {
            continue;
        };
        let count = agreeing(reference);
        if best.is_none_or(|(_, c)| count > c) {
            best = Some((index, count));
        }
    }
    let Some((chosen, count)) = best else {
        return Decision::Trip(if voters >= MAX_CHANNELS {
            Trip::NoMajority
        } else {
            Trip::Disagreement(Divergence::Missing)
        });
    };
    let Some(Some(reference)) = results.get(chosen) else {
        return Decision::Trip(Trip::NoMajority);
    };
    if count == voters {
        return Decision::Agree {
            chosen,
            excluded: [None; MAX_CHANNELS],
        };
    }
    if voters == MAX_CHANNELS && count == 2 {
        let mut excluded = [None; MAX_CHANNELS];
        for ((slot, result), is_active) in excluded.iter_mut().zip(results).zip(&active) {
            if *is_active && result.as_ref() != Some(reference) {
                *slot = Some(classify(result.as_ref(), reference));
            }
        }
        return Decision::Agree { chosen, excluded };
    }
    if voters == MAX_CHANNELS {
        return Decision::Trip(Trip::NoMajority);
    }
    let other = results
        .iter()
        .zip(&active)
        .enumerate()
        .find(|(i, (_, a))| **a && *i != chosen)
        .and_then(|(_, (r, _))| r.as_ref());
    Decision::Trip(Trip::Disagreement(classify(other, reference)))
}

/// Ein Kern in bis zu drei Kanälen mit Voter und sicherem Zustand.
#[derive(Debug)]
pub struct Channels<'p, R, S, O> {
    kernel: Interlocking<'p, R>,
    slots: [Option<Slot<S, O>>; MAX_CHANNELS],
    redundancy: Redundancy,
    trip: Option<Trip>,
    announce: bool,
    since_trip: u64,
    state_check_every: u64,
    cycle: u64,
}

impl<'p, R: RuleSet, S: StateStore, O: ChannelOutputs> Channels<'p, R, S, O> {
    /// Kanäle für einen Kern. `channels` enthält je Kanal Zustandsspeicher und Ausgabepuffer,
    /// lückenlos ab Kanal 0; die Anzahl bestimmt die Redundanz.
    ///
    /// # Errors
    /// [`ConfigError`], wenn kein Kanal angegeben ist, Kanäle fehlen oder die Kanäle nicht mit
    /// demselben Zustand beginnen.
    pub fn new(
        kernel: Interlocking<'p, R>,
        channels: [Option<(S, O)>; MAX_CHANNELS],
    ) -> Result<Self, ConfigError> {
        let count = channels.iter().take_while(|c| c.is_some()).count();
        if channels.iter().filter(|c| c.is_some()).count() != count {
            return Err(ConfigError::Gap);
        }
        let redundancy = Redundancy::from_channels(count).ok_or(ConfigError::NoChannel)?;
        let mut slots = channels.map(|c| {
            c.map(|(state, out)| Slot {
                state,
                out,
                active: true,
                stalled: false,
                fault: None,
                result: Ok(()),
            })
        });
        let mut first: Option<Digest> = None;
        for slot in slots.iter_mut().flatten() {
            let digest = state_digest(&slot.state.state());
            if first.is_some_and(|f| f != digest) {
                return Err(ConfigError::StatesDiffer);
            }
            first = Some(digest);
        }
        Ok(Self {
            kernel,
            slots,
            redundancy,
            trip: None,
            announce: false,
            since_trip: 0,
            state_check_every: 1,
            cycle: 0,
        })
    }

    /// Vergleicht den Zustand nur alle `every` Zyklen (Vorgabe: jeden Zyklus; `0` nie).
    /// Ausgaben werden immer verglichen.
    #[must_use]
    pub const fn with_state_check_every(mut self, every: u64) -> Self {
        self.state_check_every = every;
        self
    }

    /// Der Kern (Plan und Regelwerk).
    pub const fn kernel(&self) -> &Interlocking<'p, R> {
        &self.kernel
    }

    /// Zustand für Anzeige und Diagnose.
    pub fn status(&self) -> Status {
        let mut active = [false; MAX_CHANNELS];
        for (a, slot) in active.iter_mut().zip(&self.slots) {
            *a = slot.as_ref().is_some_and(|s| s.active);
        }
        Status {
            redundancy: self.redundancy,
            active,
            trip: self.trip,
            cycle: self.cycle,
        }
    }

    /// Zustandsspeicher des ersten mitwählenden Kanals (bzw. von Kanal 0).
    pub fn primary(&self) -> Option<&S> {
        self.slots
            .iter()
            .flatten()
            .find(|s| s.active)
            .or_else(|| self.slots.iter().flatten().next())
            .map(|s| &s.state)
    }

    /// Bringt einen Fehler in einen Kanal ein. `false`, wenn es den Kanal nicht gibt.
    pub fn inject(&mut self, channel: usize, fault: Fault) -> bool {
        match self.slots.get_mut(channel) {
            Some(Some(slot)) => {
                slot.fault = Some(fault);
                true
            }
            _ => false,
        }
    }

    /// Geht von außen in den sicheren Zustand (Watchdog). Ein bestehender Grund bleibt.
    pub fn trip(&mut self, trip: Trip) {
        if self.trip.is_none() {
            self.trip = Some(trip);
            self.announce = true;
            self.since_trip = 0;
        }
    }

    /// Verlässt den sicheren Zustand: Alle Kanäle beginnen wie nach einem Neustart (alle
    /// Elemente unbekannt, keine Fahrstraßen) und wählen wieder mit (REQ-SAF-004).
    pub fn reset(&mut self) {
        for slot in self.slots.iter_mut().flatten() {
            slot.state.state().reset();
            slot.out.clear();
            slot.active = true;
            slot.stalled = false;
            slot.fault = None;
            slot.result = Ok(());
        }
        self.trip = None;
        self.announce = false;
        self.since_trip = 0;
    }

    /// Nimmt einen ausgeschlossenen Kanal wieder auf: Er übernimmt den Zustand des ersten
    /// gesunden Kanals und wählt ab dem nächsten Zyklus wieder mit (REQ-SAF-003).
    ///
    /// # Errors
    /// [`ReintegrateError`].
    pub fn reintegrate(&mut self, channel: usize) -> Result<(), ReintegrateError> {
        if self.trip.is_some() {
            return Err(ReintegrateError::Tripped);
        }
        match self.slots.get(channel) {
            Some(Some(s)) if s.active => return Err(ReintegrateError::AlreadyActive),
            Some(Some(_)) => {}
            _ => return Err(ReintegrateError::NoSuchChannel),
        }
        let source = self
            .slots
            .iter()
            .position(|s| s.as_ref().is_some_and(|s| s.active))
            .ok_or(ReintegrateError::NoHealthyChannel)?;
        let (src, dst) = if source < channel {
            let (low, high) = self.slots.split_at_mut(channel);
            (low.get_mut(source), high.first_mut())
        } else {
            let (low, high) = self.slots.split_at_mut(source);
            (high.first_mut(), low.get_mut(channel))
        };
        let (Some(Some(src)), Some(Some(dst))) = (src, dst) else {
            return Err(ReintegrateError::NoSuchChannel);
        };
        dst.state
            .state()
            .copy_from(&src.state.state())
            .map_err(|_| ReintegrateError::StateMismatch)?;
        dst.out.clear();
        dst.active = true;
        dst.stalled = false;
        dst.fault = None;
        dst.result = src.result;
        Ok(())
    }

    /// Führt einen Zyklus in allen mitwählenden Kanälen aus und gibt die abgestimmten
    /// Ausgaben an `out` weiter – oder im sicheren Zustand die Ausgaben der Schicht.
    pub fn step<X: Outputs + ?Sized>(
        &mut self,
        now: Tick,
        inputs: &[Input],
        out: &mut X,
    ) -> Verdict {
        self.cycle = self.cycle.saturating_add(1);
        let mut verdict = Verdict::new();
        if let Some(trip) = self.trip {
            self.since_trip = self.since_trip.saturating_add(1);
            let all = core::mem::take(&mut self.announce)
                || self.since_trip.checked_rem(RESEND_EVERY) == Some(0);
            self.safe_outputs(inputs, all, out);
            verdict.trip = Some(trip);
            return verdict;
        }

        let check_state = self
            .cycle
            .checked_rem(self.state_check_every)
            .is_some_and(|r| r == 0);
        let mut results: [Option<Fingerprint>; MAX_CHANNELS] = [None; MAX_CHANNELS];
        let mut active = [false; MAX_CHANNELS];
        for ((slot, result), is_active) in self.slots.iter_mut().zip(&mut results).zip(&mut active)
        {
            let Some(slot) = slot.as_mut().filter(|s| s.active) else {
                continue;
            };
            *is_active = true;
            *result = Self::run_channel(&self.kernel, slot, now, inputs, check_state);
        }

        match vote(&results, active) {
            Decision::Agree { chosen, excluded } => {
                for (index, (slot, divergence)) in self.slots.iter_mut().zip(excluded).enumerate() {
                    if let (Some(slot), Some(divergence)) = (slot, divergence) {
                        slot.active = false;
                        verdict.push(SafetyEvent::ChannelExcluded {
                            channel: u8::try_from(index).unwrap_or(u8::MAX),
                            divergence,
                        });
                    }
                }
                match self.slots.get(chosen) {
                    Some(Some(slot)) if !slot.out.overflowed() => {
                        slot.out.replay(out);
                        verdict.kernel = slot.result;
                    }
                    _ => self.enter(Trip::OutputOverflow, inputs, out, &mut verdict),
                }
            }
            Decision::Trip(trip) => self.enter(trip, inputs, out, &mut verdict),
        }
        verdict
    }

    fn run_channel(
        kernel: &Interlocking<'p, R>,
        slot: &mut Slot<S, O>,
        now: Tick,
        inputs: &[Input],
        check_state: bool,
    ) -> Option<Fingerprint> {
        slot.out.clear();
        let fault = slot.fault.take();
        if fault == Some(Fault::Stall) {
            slot.stalled = true;
        }
        if slot.stalled {
            return None;
        }
        let inputs = if fault == Some(Fault::DropInputs) {
            &[]
        } else {
            inputs
        };
        let mut state = slot.state.state();
        let mut tap = Tap {
            inner: &mut slot.out,
            sha: Sha256::new(),
        };
        slot.result = kernel.step(&mut state, now, inputs, &mut tap);
        if fault == Some(Fault::StrayCommand) {
            tap.command(Command::SetSignal {
                signal: SignalId::new(0),
                aspect: Aspect::Proceed,
            });
        }
        let Tap { inner, mut sha } = tap;
        feed(&mut sha, &(slot.result, inner.overflowed()));
        let state_digest = if check_state {
            state_digest(&state)
        } else {
            Digest::default()
        };
        Some((finish(sha), state_digest))
    }

    fn enter<X: Outputs + ?Sized>(
        &mut self,
        trip: Trip,
        inputs: &[Input],
        out: &mut X,
        verdict: &mut Verdict,
    ) {
        self.trip = Some(trip);
        self.announce = false;
        self.since_trip = 0;
        verdict.trip = Some(trip);
        verdict.push(SafetyEvent::Tripped(trip));
        self.safe_outputs(inputs, true, out);
    }

    /// Ausgaben im sicheren Zustand, allein aus dem Plan: alle Signale Halt, Ersatzsignale
    /// aus, alle Anforderungen abgelehnt.
    fn safe_outputs<X: Outputs + ?Sized>(&self, inputs: &[Input], all: bool, out: &mut X) {
        if all {
            for index in 0..self.kernel.plan().signal_count() {
                let Some(signal) = SignalId::from_index(index) else {
                    continue;
                };
                out.command(Command::SetSignal {
                    signal,
                    aspect: Aspect::Stop,
                });
                out.command(Command::SetSubstitute { signal, on: false });
            }
        }
        for input in inputs {
            if let Input::Request { id, .. } = *input {
                out.event(Event::Rejected(id, Rejection::SafeState));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Option<(u8, u8)> = Some((1, 1));
    const B: Option<(u8, u8)> = Some((2, 1));
    const C: Option<(u8, u8)> = Some((1, 2));

    #[test]
    fn two_out_of_two() {
        let act = [true, true, false];
        assert!(matches!(
            vote(&[A, A, None], act),
            Decision::Agree { chosen: 0, .. }
        ));
        assert_eq!(
            vote(&[A, B, None], act),
            Decision::Trip(Trip::Disagreement(Divergence::Outputs))
        );
        assert_eq!(
            vote(&[A, C, None], act),
            Decision::Trip(Trip::Disagreement(Divergence::State))
        );
        assert_eq!(
            vote(&[A, None, None], act),
            Decision::Trip(Trip::Disagreement(Divergence::Missing))
        );
        assert_eq!(
            vote::<u8>(&[None, None, None], act),
            Decision::Trip(Trip::Disagreement(Divergence::Missing))
        );
    }

    #[test]
    fn two_out_of_three() {
        let all = [true; 3];
        assert_eq!(
            vote(&[A, A, A], all),
            Decision::Agree {
                chosen: 0,
                excluded: [None; 3]
            }
        );
        assert_eq!(
            vote(&[B, A, A], all),
            Decision::Agree {
                chosen: 1,
                excluded: [Some(Divergence::Outputs), None, None]
            }
        );
        assert_eq!(
            vote(&[A, None, A], all),
            Decision::Agree {
                chosen: 0,
                excluded: [None, Some(Divergence::Missing), None]
            }
        );
        assert_eq!(vote(&[A, B, C], all), Decision::Trip(Trip::NoMajority));
        assert_eq!(
            vote(&[A, None, None], all),
            Decision::Trip(Trip::NoMajority)
        );
        // Nach einem Ausschluss: 2-von-2 über die übrigen Kanäle.
        assert_eq!(
            vote(&[B, A, B], [false, true, true]),
            Decision::Trip(Trip::Disagreement(Divergence::Outputs))
        );
    }

    #[test]
    fn single_channel() {
        let act = [true, false, false];
        assert!(matches!(
            vote(&[A, None, None], act),
            Decision::Agree { .. }
        ));
        assert!(matches!(vote(&[None, A, A], act), Decision::Trip(_)));
    }
}
