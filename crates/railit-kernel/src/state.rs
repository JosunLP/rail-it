//! Zustand des Kerns.
//!
//! Der Kern allokiert nicht. Den Speicher stellt der Aufrufer bereit: auf Embedded als
//! statische Arrays, deren Größe der Plan vorgibt, auf dem Server über [`OwnedState`]
//! (Feature `alloc`). [`KernelState`] ist die veränderliche Sicht darauf, die dem Kern je
//! Zyklus übergeben wird.

use railit_plan::PlanView;
use railit_types::{PointPosition, RouteId, Tick};

use crate::io::{Aspect, Occupancy, PointReport, RoutePhase, SafeStateReason, SignalReport};

/// Zustand einer Weiche.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PointState {
    pub(crate) report: Option<PointReport>,
    pub(crate) reported_at: Tick,
    pub(crate) commanded: Option<PointPosition>,
    pub(crate) owner: Option<RouteId>,
}

impl PointState {
    /// Letzte Meldung (ohne Berücksichtigung ihres Alters).
    pub const fn report(&self) -> Option<PointReport> {
        self.report
    }

    /// Zeitpunkt der letzten Meldung.
    pub const fn reported_at(&self) -> Tick {
        self.reported_at
    }

    /// Zuletzt befohlene Lage.
    pub const fn commanded(&self) -> Option<PointPosition> {
        self.commanded
    }

    /// Fahrstraße, die die Weiche verschlossen hat.
    pub const fn owner(&self) -> Option<RouteId> {
        self.owner
    }
}

/// Zustand eines Gleisfreimeldeabschnitts.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TvdState {
    pub(crate) occupancy: Option<Occupancy>,
    pub(crate) reported_at: Tick,
    pub(crate) owner: Option<RouteId>,
    pub(crate) seen_occupied: bool,
}

impl TvdState {
    /// Letzte Meldung (ohne Berücksichtigung ihres Alters).
    pub const fn occupancy(&self) -> Option<Occupancy> {
        self.occupancy
    }

    /// Zeitpunkt der letzten Meldung.
    pub const fn reported_at(&self) -> Tick {
        self.reported_at
    }

    /// Fahrstraße, die den Abschnitt verschlossen hat.
    pub const fn owner(&self) -> Option<RouteId> {
        self.owner
    }
}

/// Zustand eines Signals.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SignalState {
    pub(crate) report: Option<SignalReport>,
    pub(crate) reported_at: Tick,
    pub(crate) commanded: Aspect,
    pub(crate) commanded_at: Tick,
    /// Zwischenwert je Zyklus: gewünschter Begriff.
    pub(crate) want: Aspect,
    /// Abweichung „meldet Fahrt, Halt befohlen“ bereits gemeldet.
    pub(crate) mismatch_alarmed: bool,
}

impl SignalState {
    /// Letzte Meldung.
    pub const fn report(&self) -> Option<SignalReport> {
        self.report
    }

    /// Befohlener Begriff.
    pub const fn commanded(&self) -> Aspect {
        self.commanded
    }
}

/// Zustand einer Fahrstraße.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RouteState {
    pub(crate) phase: RoutePhase,
    pub(crate) since: Tick,
    /// Index des ersten noch nicht aufgelösten Abschnitts.
    pub(crate) next_release: u32,
}

impl RouteState {
    /// Aktuelle Phase.
    pub const fn phase(&self) -> RoutePhase {
        self.phase
    }

    /// Beginn der aktuellen Phase.
    pub const fn since(&self) -> Tick {
        self.since
    }

    /// Anzahl bereits aufgelöster Abschnitte.
    pub const fn released(&self) -> u32 {
        self.next_release
    }
}

/// Globaler Zustand des Kerns.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Meta {
    pub(crate) cycle: u64,
    pub(crate) safe_state: Option<SafeStateReason>,
}

impl Meta {
    /// Anzahl ausgeführter Zyklen.
    pub const fn cycle(&self) -> u64 {
        self.cycle
    }

    /// Grund des sicheren Zustands, falls der Kern sich darin befindet.
    pub const fn safe_state(&self) -> Option<SafeStateReason> {
        self.safe_state
    }
}

/// Veränderliche Sicht auf den Zustandsspeicher.
#[derive(Debug)]
pub struct KernelState<'s> {
    pub(crate) points: &'s mut [PointState],
    pub(crate) tvds: &'s mut [TvdState],
    pub(crate) signals: &'s mut [SignalState],
    pub(crate) routes: &'s mut [RouteState],
    /// Bitmenge der eingestellten Fahrstraßen.
    pub(crate) active: &'s mut [u64],
    pub(crate) meta: &'s mut Meta,
}

impl<'s> KernelState<'s> {
    /// Sicht auf vom Aufrufer bereitgestellten Speicher.
    ///
    /// # Errors
    /// [`SafeStateReason::StateMismatch`], wenn die Größen nicht zum Plan passen.
    pub fn new(
        plan: &PlanView<'_>,
        points: &'s mut [PointState],
        tvds: &'s mut [TvdState],
        signals: &'s mut [SignalState],
        routes: &'s mut [RouteState],
        active: &'s mut [u64],
        meta: &'s mut Meta,
    ) -> Result<Self, SafeStateReason> {
        if points.len() != plan.point_count()
            || tvds.len() != plan.tvd_count()
            || signals.len() != plan.signal_count()
            || routes.len() != plan.route_count()
            || active.len() != plan.route_set_words()
        {
            return Err(SafeStateReason::StateMismatch);
        }
        Ok(Self {
            points,
            tvds,
            signals,
            routes,
            active,
            meta,
        })
    }

    /// Setzt den gesamten Zustand auf den Anfangszustand zurück: keine Meldungen, alle
    /// Signale Halt, keine Fahrstraßen.
    pub fn reset(&mut self) {
        self.points.fill(PointState::default());
        self.tvds.fill(TvdState::default());
        self.signals.fill(SignalState::default());
        self.routes.fill(RouteState::default());
        self.active.fill(0);
        *self.meta = Meta::default();
    }

    /// Weichenzustände, Index = `PointId`.
    pub fn points(&self) -> &[PointState] {
        self.points
    }

    /// Abschnittszustände, Index = `TvdId`.
    pub fn tvds(&self) -> &[TvdState] {
        self.tvds
    }

    /// Signalzustände, Index = `SignalId`.
    pub fn signals(&self) -> &[SignalState] {
        self.signals
    }

    /// Fahrstraßenzustände, Index = `RouteId`.
    pub fn routes(&self) -> &[RouteState] {
        self.routes
    }

    /// Globaler Zustand.
    pub fn meta(&self) -> &Meta {
        self.meta
    }
}

#[cfg(feature = "alloc")]
mod owned {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{KernelState, Meta, PointState, RouteState, SignalState, TvdState};
    use crate::io::{Command, Event, Outputs};
    use railit_plan::PlanView;

    /// Zustandsspeicher auf dem Heap (Server).
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub struct OwnedState {
        points: Vec<PointState>,
        tvds: Vec<TvdState>,
        signals: Vec<SignalState>,
        routes: Vec<RouteState>,
        active: Vec<u64>,
        meta: Meta,
    }

    impl OwnedState {
        /// Anfangszustand passend zu `plan`.
        pub fn new(plan: &PlanView<'_>) -> Self {
            Self {
                points: vec![PointState::default(); plan.point_count()],
                tvds: vec![TvdState::default(); plan.tvd_count()],
                signals: vec![SignalState::default(); plan.signal_count()],
                routes: vec![RouteState::default(); plan.route_count()],
                active: vec![0; plan.route_set_words()],
                meta: Meta::default(),
            }
        }

        /// Veränderliche Sicht für einen Zyklus.
        pub fn view(&mut self) -> KernelState<'_> {
            KernelState {
                points: &mut self.points,
                tvds: &mut self.tvds,
                signals: &mut self.signals,
                routes: &mut self.routes,
                active: &mut self.active,
                meta: &mut self.meta,
            }
        }

        /// Weichenzustände.
        pub fn points(&self) -> &[PointState] {
            &self.points
        }

        /// Abschnittszustände.
        pub fn tvds(&self) -> &[TvdState] {
            &self.tvds
        }

        /// Signalzustände.
        pub fn signals(&self) -> &[SignalState] {
            &self.signals
        }

        /// Fahrstraßenzustände.
        pub fn routes(&self) -> &[RouteState] {
            &self.routes
        }

        /// Globaler Zustand.
        pub fn meta(&self) -> &Meta {
            &self.meta
        }
    }

    /// Ausgabepuffer auf dem Heap.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct VecOutputs {
        /// Stellbefehle in Erzeugungsreihenfolge.
        pub commands: Vec<Command>,
        /// Ereignisse in Erzeugungsreihenfolge.
        pub events: Vec<Event>,
    }

    impl VecOutputs {
        /// Leert beide Puffer.
        pub fn clear(&mut self) {
            self.commands.clear();
            self.events.clear();
        }
    }

    impl Outputs for VecOutputs {
        fn command(&mut self, command: Command) {
            self.commands.push(command);
        }

        fn event(&mut self, event: Event) {
            self.events.push(event);
        }
    }
}

#[cfg(feature = "alloc")]
pub use owned::{OwnedState, VecOutputs};
