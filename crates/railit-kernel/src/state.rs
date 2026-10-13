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
    Aspect, AuxId, AuxOperation, BorderMessage, LxReport, Occupancy, OperatorId, PointReport,
    RoutePhase, SafeStateReason, SignalReport,
};

/// Empfohlene Anzahl gleichzeitig offener Hilfshandlungen.
pub const AUX_SLOTS: usize = 8;

/// Zustand einer Weiche.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TvdState {
    pub(crate) occupancy: Option<Occupancy>,
    pub(crate) reported_at: Tick,
    pub(crate) owner: Option<RouteId>,
    pub(crate) overlap_owner: Option<RouteId>,
    pub(crate) seen_occupied: bool,
    /// Anzahl der Grenzen, deren Annäherungsabschnitt den Abschnitt verschließt.
    pub(crate) border_locks: u8,
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

    /// Ist der Abschnitt als Annäherungsabschnitt einer Grenze verschlossen (Züge aus dem
    /// Nachbarbereich dürfen kommen)?
    pub const fn border_locked(&self) -> bool {
        self.border_locks > 0
    }
}

/// Zustand eines Signals.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

/// Zustand einer Grenze zu einem Nachbarbereich (Erlaubniswechsel).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
// Die Merkmale sind voneinander unabhängige Protokollzustände.
#[allow(clippy::struct_excessive_bools)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BorderState {
    /// Dieser Kern hält die Erlaubnis.
    pub(crate) holding: bool,
    /// Laufende Nummer der Erlaubnis.
    pub(crate) seq: u32,
    /// Die Erlaubnis wurde abgegeben und wird mit `seq` angeboten.
    pub(crate) offering: bool,
    /// Die Erlaubnis ist angefordert (Bedienung oder Zuglenkung).
    pub(crate) want: bool,
    /// Hilfshandlung „Erlaubnis zurückholen“ wartet auf die Einschwingzeit.
    pub(crate) take_back: bool,
    /// Der Annäherungsabschnitt ist verschlossen (gilt genau dann, wenn die Erlaubnis nicht
    /// gehalten wird).
    pub(crate) lock: bool,
    /// Letzte Meldung des Nachbarn.
    pub(crate) peer: Option<BorderMessage>,
    pub(crate) peer_at: Tick,
    /// Seit wann der Nachbar ununterbrochen meldet, die Erlaubnis nicht zu halten.
    pub(crate) peer_free_since: Option<Tick>,
    pub(crate) lost_alarmed: bool,
    pub(crate) conflict_alarmed: bool,
}

impl BorderState {
    /// Hält dieser Kern die Erlaubnis?
    pub const fn holding(&self) -> bool {
        self.holding
    }

    /// Laufende Nummer der Erlaubnis.
    pub const fn seq(&self) -> u32 {
        self.seq
    }

    /// Wird die Erlaubnis gerade angeboten (abgegeben, noch nicht übernommen)?
    pub const fn offering(&self) -> bool {
        self.offering
    }

    /// Ist die Erlaubnis angefordert?
    pub const fn requested(&self) -> bool {
        self.want
    }

    /// Ist der Annäherungsabschnitt verschlossen?
    pub const fn approach_locked(&self) -> bool {
        self.lock
    }

    /// Letzte Meldung des Nachbarn und ihr Zeitpunkt.
    pub const fn peer(&self) -> Option<(BorderMessage, Tick)> {
        match self.peer {
            Some(m) => Some((m, self.peer_at)),
            None => None,
        }
    }
}

/// Zustand einer Fahrstraße.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Meta {
    pub(crate) cycle: u64,
    pub(crate) safe_state: Option<SafeStateReason>,
    pub(crate) aux_counter: u64,
    pub(crate) next_aux: u32,
    /// Zeitpunkt des ersten Zyklus nach dem (Neu-)Start.
    pub(crate) started_at: Tick,
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

/// Ein Element des Kernzustands für die Zustandsübertragung an einen Spiegel (Host des sicheren
/// Rechners, `railit-embedded`).
///
/// Der Index ist die Position im jeweiligen Speicher. [`KernelState::part`] zählt alle Teile in
/// fester Reihenfolge auf, [`KernelState::apply`] setzt einen Teil in einen Speicher gleicher
/// Größe ein. Nach allen Teilen gleichen sich beide Zustände (und ihre Hashes).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum StatePart {
    /// Weiche.
    Point(u16, PointState),
    /// Gleisfreimeldeabschnitt.
    Tvd(u16, TvdState),
    /// Signal.
    Signal(u16, SignalState),
    /// Fahrstraße.
    Route(u16, RouteState),
    /// Bahnübergang.
    Lx(u16, LxState),
    /// Grenze.
    Border(u16, BorderState),
    /// Platz für eine offene Hilfshandlung.
    Aux(u16, Option<AuxSlot>),
    /// Wort der Bitmenge eingestellter Fahrstraßen.
    Active(u16, u64),
    /// Globaler Zustand.
    Meta(Meta),
}

/// Veränderliche Sicht auf den Zustandsspeicher.
///
/// `Hash` deckt den gesamten Zustand ab; zwei Kanäle mit demselben Zustand haben denselben
/// Hash (Voter, `railit-safety`).
#[derive(Debug, Hash)]
pub struct KernelState<'s> {
    pub(crate) points: &'s mut [PointState],
    pub(crate) tvds: &'s mut [TvdState],
    pub(crate) signals: &'s mut [SignalState],
    pub(crate) routes: &'s mut [RouteState],
    pub(crate) lxs: &'s mut [LxState],
    pub(crate) borders: &'s mut [BorderState],
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
    /// Grenzen, Länge = Anzahl Grenzen im Plan.
    pub borders: &'s mut [BorderState],
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
            borders,
            aux,
            active,
            meta,
        } = buffers;
        if points.len() != plan.point_count()
            || tvds.len() != plan.tvd_count()
            || signals.len() != plan.signal_count()
            || routes.len() != plan.route_count()
            || lxs.len() != plan.lx_count()
            || borders.len() != plan.border_count()
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
            borders,
            aux,
            active,
            meta,
        })
    }

    /// Versetzt den Zustand von außen in den sicheren Zustand (Sicherheitsschicht): alle
    /// Signale Halt, Ersatzsignale aus. Danach lehnt der Kern alle Anforderungen ab, bis der
    /// Zustand zurückgesetzt wird. Ein bestehender Grund bleibt erhalten.
    pub fn enter_safe_state(&mut self, reason: SafeStateReason, now: Tick) {
        if self.meta.safe_state.is_none() {
            self.meta.safe_state = Some(reason);
        }
        for s in self.signals.iter_mut() {
            if s.commanded != crate::io::Aspect::Stop {
                s.commanded = crate::io::Aspect::Stop;
                s.commanded_at = now;
            }
            s.substitute = false;
        }
    }

    /// Kürzere Sicht auf denselben Speicher.
    pub fn reborrow(&mut self) -> KernelState<'_> {
        KernelState {
            points: self.points,
            tvds: self.tvds,
            signals: self.signals,
            routes: self.routes,
            lxs: self.lxs,
            borders: self.borders,
            aux: self.aux,
            active: self.active,
            meta: self.meta,
        }
    }

    /// Übernimmt den gesamten Zustand eines anderen Speichers gleicher Größe (Wiederaufnahme
    /// eines Kanals, REQ-SAF-003).
    ///
    /// # Errors
    /// [`SafeStateReason::StateMismatch`], wenn die Größen verschieden sind; dann bleibt der
    /// Zustand unverändert.
    pub fn copy_from(&mut self, other: &KernelState<'_>) -> Result<(), SafeStateReason> {
        if self.points.len() != other.points.len()
            || self.tvds.len() != other.tvds.len()
            || self.signals.len() != other.signals.len()
            || self.routes.len() != other.routes.len()
            || self.lxs.len() != other.lxs.len()
            || self.borders.len() != other.borders.len()
            || self.aux.len() != other.aux.len()
            || self.active.len() != other.active.len()
        {
            return Err(SafeStateReason::StateMismatch);
        }
        self.points.copy_from_slice(other.points);
        self.tvds.copy_from_slice(other.tvds);
        self.signals.copy_from_slice(other.signals);
        self.routes.copy_from_slice(other.routes);
        self.lxs.copy_from_slice(other.lxs);
        self.borders.copy_from_slice(other.borders);
        self.aux.copy_from_slice(other.aux);
        self.active.copy_from_slice(other.active);
        *self.meta = *other.meta;
        Ok(())
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
        self.borders.fill(BorderState::default());
        self.aux.fill(None);
        self.active.fill(0);
        *self.meta = Meta::default();
        self.meta.aux_counter = counter;
    }

    /// Anzahl der Teile für die Zustandsübertragung ([`StatePart`]).
    pub fn part_count(&self) -> usize {
        [
            self.points.len(),
            self.tvds.len(),
            self.signals.len(),
            self.routes.len(),
            self.lxs.len(),
            self.borders.len(),
            self.aux.len(),
            self.active.len(),
            1,
        ]
        .iter()
        .fold(0usize, |sum, n| sum.saturating_add(*n))
    }

    /// Teil `index` (0 … [`Self::part_count`]) für die Zustandsübertragung.
    pub fn part(&self, index: usize) -> Option<StatePart> {
        fn take<T: Copy>(slice: &[T], index: &mut usize) -> Option<(u16, T)> {
            if let Some(item) = slice.get(*index) {
                return Some((u16::try_from(*index).ok()?, *item));
            }
            *index = index.checked_sub(slice.len())?;
            None
        }
        let mut i = index;
        if let Some((k, v)) = take(self.points, &mut i) {
            return Some(StatePart::Point(k, v));
        }
        if let Some((k, v)) = take(self.tvds, &mut i) {
            return Some(StatePart::Tvd(k, v));
        }
        if let Some((k, v)) = take(self.signals, &mut i) {
            return Some(StatePart::Signal(k, v));
        }
        if let Some((k, v)) = take(self.routes, &mut i) {
            return Some(StatePart::Route(k, v));
        }
        if let Some((k, v)) = take(self.lxs, &mut i) {
            return Some(StatePart::Lx(k, v));
        }
        if let Some((k, v)) = take(self.borders, &mut i) {
            return Some(StatePart::Border(k, v));
        }
        if let Some((k, v)) = take(self.aux, &mut i) {
            return Some(StatePart::Aux(k, v));
        }
        if let Some((k, v)) = take(self.active, &mut i) {
            return Some(StatePart::Active(k, v));
        }
        (i == 0).then_some(StatePart::Meta(*self.meta))
    }

    /// Setzt einen Teil der Zustandsübertragung ein.
    ///
    /// # Errors
    /// [`SafeStateReason::StateMismatch`], wenn der Index außerhalb des Speichers liegt.
    pub fn apply(&mut self, part: StatePart) -> Result<(), SafeStateReason> {
        fn put<T>(slice: &mut [T], index: u16, value: T) -> Result<(), SafeStateReason> {
            let slot = slice
                .get_mut(usize::from(index))
                .ok_or(SafeStateReason::StateMismatch)?;
            *slot = value;
            Ok(())
        }
        match part {
            StatePart::Point(i, v) => put(self.points, i, v),
            StatePart::Tvd(i, v) => put(self.tvds, i, v),
            StatePart::Signal(i, v) => put(self.signals, i, v),
            StatePart::Route(i, v) => put(self.routes, i, v),
            StatePart::Lx(i, v) => put(self.lxs, i, v),
            StatePart::Border(i, v) => put(self.borders, i, v),
            StatePart::Aux(i, v) => put(self.aux, i, v),
            StatePart::Active(i, v) => put(self.active, i, v),
            StatePart::Meta(m) => {
                *self.meta = m;
                Ok(())
            }
        }
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

    /// Grenzzustände, Index = `BorderId`.
    pub fn borders(&self) -> &[BorderState] {
        self.borders
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
        AUX_SLOTS, AuxSlot, BorderState, KernelState, LxState, Meta, PointState, RouteState,
        SignalState, TvdState,
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
        borders: Vec<BorderState>,
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
                borders: vec![BorderState::default(); plan.border_count()],
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
                borders: &mut self.borders,
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

        /// Grenzzustände.
        pub fn borders(&self) -> &[BorderState] {
            &self.borders
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
