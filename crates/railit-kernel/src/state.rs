//! Zustand des Kerns.
//!
//! Der Kern allokiert nicht. Den Speicher stellt der Aufrufer bereit: auf Embedded als
//! statische Arrays, deren Größe der Plan vorgibt, auf dem Server über [`OwnedState`]
//! (Feature `alloc`). [`KernelState`] ist die veränderliche Sicht darauf, die dem Kern je
//! Zyklus übergeben wird.
//!
//! Verschlüsse (Konzept, Abschnitt 10.3):
//!
//! - **Fahrweg**: Abschnitte und Weichen im Fahrweg gehören genau einer Fahrstraße
//!   (`owner`).
//! - **Durchrutschweg**: Abschnitte gehören genau einer Fahrstraße (`overlap_owner`); ein
//!   Abschnitt kann zugleich Fahrweg einer anschließenden Fahrstraße sein.
//! - **Geteilte Verschlüsse**: Weichen im Durchrutschweg und Flankenschutzweichen werden
//!   lagebezogen verschlossen; mehrere Fahrstraßen dürfen dieselbe Lage verlangen
//!   (`holds`, `locked_pos`). Schutzsignale (`stop_holds`) und Bahnübergänge (`holds`)
//!   zählen ihre Halter ebenso.

use railit_plan::PlanView;
use railit_types::{PointPosition, RouteId, Tick};

use crate::io::{
    Aspect, AuxId, AuxOperation, LxReport, Occupancy, OperatorId, PointReport, RoutePhase,
    SafeStateReason, SignalReport,
};

/// Empfohlene Anzahl gleichzeitig offener Hilfshandlungen.
pub const AUX_SLOTS: usize = 8;

/// Zustand einer Weiche.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PointState {
    pub(crate) report: Option<PointReport>,
    pub(crate) reported_at: Tick,
    pub(crate) commanded: Option<PointPosition>,
    pub(crate) owner: Option<RouteId>,
    pub(crate) holds: u8,
    pub(crate) locked_pos: Option<PointPosition>,
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

    /// Fahrstraße, die die Weiche im Fahrweg verschlossen hat.
    pub const fn owner(&self) -> Option<RouteId> {
        self.owner
    }

    /// Anzahl geteilter Verschlüsse (Durchrutschweg, Flankenschutz).
    pub const fn holds(&self) -> u8 {
        self.holds
    }

    /// Verschlossene Lage, falls die Weiche verschlossen ist.
    pub const fn locked_position(&self) -> Option<PointPosition> {
        self.locked_pos
    }

    /// Ist die Weiche in irgendeiner Form verschlossen?
    pub const fn is_locked(&self) -> bool {
        self.owner.is_some() || self.holds > 0
    }
}

/// Zustand eines Gleisfreimeldeabschnitts.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TvdState {
    pub(crate) occupancy: Option<Occupancy>,
    pub(crate) reported_at: Tick,
    pub(crate) owner: Option<RouteId>,
    pub(crate) overlap_owner: Option<RouteId>,
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

    /// Fahrstraße, die den Abschnitt im Fahrweg verschlossen hat.
    pub const fn owner(&self) -> Option<RouteId> {
        self.owner
    }

    /// Fahrstraße, deren Durchrutschweg den Abschnitt verschlossen hat.
    pub const fn overlap_owner(&self) -> Option<RouteId> {
        self.overlap_owner
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
    /// Zwischenwert je Zyklus: Ersatzsignal gewünscht.
    pub(crate) want_substitute: bool,
    /// Abweichung „meldet Fahrt, Halt befohlen“ bereits gemeldet.
    pub(crate) mismatch_alarmed: bool,
    /// Anzahl der Fahrstraßen, die das Signal als Flankenschutz in Halt halten.
    pub(crate) stop_holds: u8,
    /// Ersatzsignal (Zs1) eingeschaltet.
    pub(crate) substitute: bool,
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

    /// Anzahl der Fahrstraßen, die das Signal als Flankenschutz in Halt halten.
    pub const fn stop_holds(&self) -> u8 {
        self.stop_holds
    }

    /// Ist das Ersatzsignal (Zs1) eingeschaltet?
    pub const fn substitute(&self) -> bool {
        self.substitute
    }
}

/// Zustand eines Bahnübergangs.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LxState {
    pub(crate) report: Option<LxReport>,
    pub(crate) reported_at: Tick,
    pub(crate) holds: u8,
    pub(crate) closed_commanded: bool,
}

impl LxState {
    /// Letzte Meldung.
    pub const fn report(&self) -> Option<LxReport> {
        self.report
    }

    /// Zeitpunkt der letzten Meldung.
    pub const fn reported_at(&self) -> Tick {
        self.reported_at
    }

    /// Anzahl der Fahrstraßen, die den Bahnübergang geschlossen halten.
    pub const fn holds(&self) -> u8 {
        self.holds
    }

    /// Ist das Schließen befohlen?
    pub const fn closed_commanded(&self) -> bool {
        self.closed_commanded
    }
}

/// Zustand einer Fahrstraße.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RouteState {
    pub(crate) phase: RoutePhase,
    pub(crate) since: Tick,
    /// Index des ersten noch nicht aufgelösten Abschnitts des Fahrwegs.
    pub(crate) next_release: u32,
    /// Seit wann das Ersatzsignal für diese Fahrstraße leuchtet.
    pub(crate) substitute_since: Option<Tick>,
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

    /// Anzahl bereits aufgelöster Abschnitte des Fahrwegs.
    pub const fn released(&self) -> u32 {
        self.next_release
    }

    /// Zeitpunkt, seit dem das Ersatzsignal für diese Fahrstraße leuchtet.
    pub const fn substitute_since(&self) -> Option<Tick> {
        self.substitute_since
    }
}

/// Offene, noch nicht bestätigte Hilfshandlung.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AuxSlot {
    pub(crate) id: AuxId,
    pub(crate) op: AuxOperation,
    pub(crate) operator: OperatorId,
    pub(crate) since: Tick,
}

impl AuxSlot {
    /// Kennung.
    pub const fn id(&self) -> AuxId {
        self.id
    }

    /// Hilfshandlung.
    pub const fn op(&self) -> AuxOperation {
        self.op
    }

    /// Anfordernder Bediener.
    pub const fn operator(&self) -> OperatorId {
        self.operator
    }

    /// Zeitpunkt der Anforderung.
    pub const fn since(&self) -> Tick {
        self.since
    }
}

/// Globaler Zustand des Kerns.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Meta {
    pub(crate) cycle: u64,
    pub(crate) safe_state: Option<SafeStateReason>,
    pub(crate) aux_counter: u64,
    pub(crate) next_aux: u32,
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

    /// Zählwerk der ausgeführten Hilfshandlungen.
    pub const fn aux_counter(&self) -> u64 {
        self.aux_counter
    }
}

/// Veränderliche Sicht auf den Zustandsspeicher.
#[derive(Debug)]
pub struct KernelState<'s> {
    pub(crate) points: &'s mut [PointState],
    pub(crate) tvds: &'s mut [TvdState],
    pub(crate) signals: &'s mut [SignalState],
    pub(crate) routes: &'s mut [RouteState],
    pub(crate) lxs: &'s mut [LxState],
    /// Plätze für offene Hilfshandlungen; die Anzahl wählt der Aufrufer.
    pub(crate) aux: &'s mut [Option<AuxSlot>],
    /// Bitmenge der eingestellten Fahrstraßen.
    pub(crate) active: &'s mut [u64],
    pub(crate) meta: &'s mut Meta,
}

/// Vom Aufrufer bereitgestellter Speicher für [`KernelState::new`].
#[derive(Debug)]
pub struct Buffers<'s> {
    /// Weichen, Länge = Anzahl Weichen im Plan.
    pub points: &'s mut [PointState],
    /// Abschnitte, Länge = Anzahl Abschnitte im Plan.
    pub tvds: &'s mut [TvdState],
    /// Signale, Länge = Anzahl Signale im Plan.
    pub signals: &'s mut [SignalState],
    /// Fahrstraßen, Länge = Anzahl Fahrstraßen im Plan.
    pub routes: &'s mut [RouteState],
    /// Bahnübergänge, Länge = Anzahl Bahnübergänge im Plan.
    pub lxs: &'s mut [LxState],
    /// Plätze für offene Hilfshandlungen (beliebige Anzahl, empfohlen [`AUX_SLOTS`]).
    pub aux: &'s mut [Option<AuxSlot>],
    /// Bitmenge der eingestellten Fahrstraßen, Länge = `PlanView::route_set_words`.
    pub active: &'s mut [u64],
    /// Globaler Zustand.
    pub meta: &'s mut Meta,
}

impl<'s> KernelState<'s> {
    /// Sicht auf vom Aufrufer bereitgestellten Speicher.
    ///
    /// # Errors
    /// [`SafeStateReason::StateMismatch`], wenn die Größen nicht zum Plan passen.
    pub fn new(plan: &PlanView<'_>, buffers: Buffers<'s>) -> Result<Self, SafeStateReason> {
        let Buffers {
            points,
            tvds,
            signals,
            routes,
            lxs,
            aux,
            active,
            meta,
        } = buffers;
        if points.len() != plan.point_count()
            || tvds.len() != plan.tvd_count()
            || signals.len() != plan.signal_count()
            || routes.len() != plan.route_count()
            || lxs.len() != plan.lx_count()
            || active.len() != plan.route_set_words()
        {
            return Err(SafeStateReason::StateMismatch);
        }
        Ok(Self {
            points,
            tvds,
            signals,
            routes,
            lxs,
            aux,
            active,
            meta,
        })
    }

    /// Setzt den gesamten Zustand auf den Anfangszustand zurück: keine Meldungen, alle
    /// Signale Halt, keine Fahrstraßen, keine offenen Hilfshandlungen. Das Zählwerk bleibt
    /// erhalten.
    pub fn reset(&mut self) {
        let counter = self.meta.aux_counter;
        self.points.fill(PointState::default());
        self.tvds.fill(TvdState::default());
        self.signals.fill(SignalState::default());
        self.routes.fill(RouteState::default());
        self.lxs.fill(LxState::default());
        self.aux.fill(None);
        self.active.fill(0);
        *self.meta = Meta::default();
        self.meta.aux_counter = counter;
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

    /// Bahnübergangszustände, Index = `LxId`.
    pub fn level_crossings(&self) -> &[LxState] {
        self.lxs
    }

    /// Offene Hilfshandlungen.
    pub fn pending_aux(&self) -> impl Iterator<Item = &AuxSlot> {
        self.aux.iter().flatten()
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

    use super::{
        AUX_SLOTS, AuxSlot, KernelState, LxState, Meta, PointState, RouteState, SignalState,
        TvdState,
    };
    use crate::io::{Command, Event, Outputs};
    use railit_plan::PlanView;

    /// Zustandsspeicher auf dem Heap (Server).
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub struct OwnedState {
        points: Vec<PointState>,
        tvds: Vec<TvdState>,
        signals: Vec<SignalState>,
        routes: Vec<RouteState>,
        lxs: Vec<LxState>,
        aux: Vec<Option<AuxSlot>>,
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
                lxs: vec![LxState::default(); plan.lx_count()],
                aux: vec![None; AUX_SLOTS],
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
                lxs: &mut self.lxs,
                aux: &mut self.aux,
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

        /// Bahnübergangszustände.
        pub fn level_crossings(&self) -> &[LxState] {
            &self.lxs
        }

        /// Offene Hilfshandlungen.
        pub fn pending_aux(&self) -> impl Iterator<Item = &AuxSlot> {
            self.aux.iter().flatten()
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
