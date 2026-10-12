//! Zyklus des Sicherheitskerns.

use railit_plan::{PlanView, Protection, RouteView};
use railit_rules::RuleSet;
use railit_types::bitset::{BitSet, BitSetMut};
use railit_types::{LxId, PointId, PointPosition, RouteId, SignalId, Tick, TvdId};

use crate::invariants;
use crate::io::{
    Alarm, Aspect, AuxId, AuxOperation, Command, Event, Input, LxReport, Occupancy, OperatorId,
    Outputs, Rejection, Request, RequestId, RoutePhase, SafeStateReason, SignalReport,
};
use crate::state::{AuxSlot, KernelState, LxState, PointState, RouteState, SignalState, TvdState};

type Res<T> = Result<T, SafeStateReason>;

const IDX: SafeStateReason = SafeStateReason::InternalIndex;
const HOLD: SafeStateReason = SafeStateReason::HoldMismatch;

pub(crate) fn point(st: &KernelState<'_>, id: PointId) -> Res<PointState> {
    id.index()
        .and_then(|i| st.points.get(i))
        .copied()
        .ok_or(IDX)
}

fn point_mut<'a>(st: &'a mut KernelState<'_>, id: PointId) -> Res<&'a mut PointState> {
    id.index().and_then(|i| st.points.get_mut(i)).ok_or(IDX)
}

pub(crate) fn tvd(st: &KernelState<'_>, id: TvdId) -> Res<TvdState> {
    id.index().and_then(|i| st.tvds.get(i)).copied().ok_or(IDX)
}

fn tvd_mut<'a>(st: &'a mut KernelState<'_>, id: TvdId) -> Res<&'a mut TvdState> {
    id.index().and_then(|i| st.tvds.get_mut(i)).ok_or(IDX)
}

pub(crate) fn signal(st: &KernelState<'_>, id: SignalId) -> Res<SignalState> {
    id.index()
        .and_then(|i| st.signals.get(i))
        .copied()
        .ok_or(IDX)
}

fn signal_mut<'a>(st: &'a mut KernelState<'_>, id: SignalId) -> Res<&'a mut SignalState> {
    id.index().and_then(|i| st.signals.get_mut(i)).ok_or(IDX)
}

pub(crate) fn route(st: &KernelState<'_>, id: RouteId) -> Res<RouteState> {
    id.index()
        .and_then(|i| st.routes.get(i))
        .copied()
        .ok_or(IDX)
}

fn route_mut<'a>(st: &'a mut KernelState<'_>, id: RouteId) -> Res<&'a mut RouteState> {
    id.index().and_then(|i| st.routes.get_mut(i)).ok_or(IDX)
}

pub(crate) fn lx(st: &KernelState<'_>, id: LxId) -> Res<LxState> {
    id.index().and_then(|i| st.lxs.get(i)).copied().ok_or(IDX)
}

fn lx_mut<'a>(st: &'a mut KernelState<'_>, id: LxId) -> Res<&'a mut LxState> {
    id.index().and_then(|i| st.lxs.get_mut(i)).ok_or(IDX)
}

/// Ist eine Meldung vom Zeitpunkt `at` zum Zeitpunkt `now` noch gültig?
pub(crate) fn fresh(at: Tick, now: Tick, timeout: railit_types::Duration) -> bool {
    now.saturating_since(at) <= timeout
}

/// Wirksame Belegung: fehlende oder veraltete Meldung ⇒ gestört.
pub(crate) fn effective_occupancy(
    t: &TvdState,
    now: Tick,
    timeout: railit_types::Duration,
) -> Occupancy {
    match t.occupancy {
        Some(o) if fresh(t.reported_at, now, timeout) => o,
        _ => Occupancy::Disturbed,
    }
}

/// Wirksame Weichenlage: nur eine aktuelle, überwachte Endlage zählt.
pub(crate) fn effective_position(
    p: &PointState,
    now: Tick,
    timeout: railit_types::Duration,
) -> Option<PointPosition> {
    match p.report {
        Some(r) if fresh(p.reported_at, now, timeout) => r.position(),
        _ => None,
    }
}

/// Ist der Bahnübergang aktuell als geschlossen gemeldet?
pub(crate) fn lx_closed(l: &LxState, now: Tick, timeout: railit_types::Duration) -> bool {
    l.report == Some(LxReport::Closed) && fresh(l.reported_at, now, timeout)
}

/// Weichen, die eine Fahrstraße lagebezogen und geteilt verschließt: Durchrutschweg und
/// Flankenschutz.
fn shared_points<'a>(
    view: &RouteView<'a>,
) -> impl Iterator<Item = (PointId, PointPosition)> + use<'a> {
    view.overlap_points()
        .chain(view.flank().filter_map(|f| match f.protection {
            Protection::Point(p, pos) => Some((p, pos)),
            Protection::Signal(_) => None,
        }))
}

/// Alle Weichen einer Fahrstraße mit ihrer geforderten Lage.
fn all_points<'a>(
    view: &RouteView<'a>,
) -> impl Iterator<Item = (PointId, PointPosition)> + use<'a> {
    view.points().chain(shared_points(view))
}

/// Schutzsignale einer Fahrstraße.
fn flank_signals<'a>(view: &RouteView<'a>) -> impl Iterator<Item = SignalId> + use<'a> {
    view.flank().filter_map(|f| match f.protection {
        Protection::Signal(s) => Some(s),
        Protection::Point(..) => None,
    })
}

/// Liegt `t` im Fahrweg der Fahrstraße?
fn in_path(view: &RouteView<'_>, t: TvdId) -> bool {
    view.tvds().any(|x| x == t)
}

fn released_count(state: &RouteState) -> Res<usize> {
    usize::try_from(state.next_release).map_err(|_| IDX)
}

fn inc(v: u8) -> Res<u8> {
    v.checked_add(1).ok_or(HOLD)
}

fn dec(v: u8) -> Res<u8> {
    v.checked_sub(1).ok_or(HOLD)
}

/// Der Sicherheitskern: Plan und Regelwerk. Der Zustand wird je Zyklus übergeben.
///
/// Der Kern ist eine reine Funktion `(Zustand, Zeit, Eingaben) → (Zustand, Befehle,
/// Ereignisse)`: keine I/O, keine Uhr, keine Allokation, keine Panikpfade (REQ-IXL-001).
#[derive(Clone, Debug)]
pub struct Interlocking<'p, R> {
    plan: PlanView<'p>,
    rules: R,
}

impl<'p, R: RuleSet> Interlocking<'p, R> {
    /// Kern für einen geprüften Plan und ein Regelwerk.
    pub const fn new(plan: PlanView<'p>, rules: R) -> Self {
        Self { plan, rules }
    }

    /// Der Plan.
    pub const fn plan(&self) -> &PlanView<'p> {
        &self.plan
    }

    /// Das Regelwerk.
    pub const fn rules(&self) -> &R {
        &self.rules
    }

    /// Führt einen Zyklus aus.
    ///
    /// Reihenfolge: Meldungen übernehmen → Anforderungen bearbeiten → Hilfshandlungen
    /// verfallen lassen → Fahrstraßen fortschalten → Bahnübergänge und Signale stellen →
    /// Invarianten prüfen.
    ///
    /// # Errors
    /// [`SafeStateReason`], wenn der Kern im sicheren Zustand ist oder in diesem Zyklus
    /// hineingeht. Dann zeigen alle Signale Halt, Ersatzsignale sind aus, es werden keine
    /// Weichen mehr gestellt, Bahnübergänge bleiben, wie sie sind, und alle Anforderungen
    /// werden abgelehnt, bis der Zustand zurückgesetzt wird (REQ-IXL-008).
    pub fn step<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        inputs: &[Input],
        out: &mut O,
    ) -> Result<(), SafeStateReason> {
        if let Some(reason) = st.meta.safe_state {
            self.record_reports(st, now, inputs, out);
            Self::reject_all(inputs, Rejection::SafeState, out);
            Self::hold_stop(st, now, false, out);
            st.meta.cycle = st.meta.cycle.saturating_add(1);
            return Err(reason);
        }

        let result = self.run(st, now, inputs, out);
        st.meta.cycle = st.meta.cycle.saturating_add(1);
        if let Err(reason) = result {
            st.meta.safe_state = Some(reason);
            Self::hold_stop(st, now, true, out);
            return Err(reason);
        }
        Ok(())
    }

    fn run<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        inputs: &[Input],
        out: &mut O,
    ) -> Res<()> {
        let first_cycle = st.meta.cycle == 0;
        self.record_reports(st, now, inputs, out);

        for input in inputs {
            if let Input::Request { id, request } = *input {
                self.handle(st, now, id, request, out)?;
            }
        }
        self.expire_aux(st, now, out);

        for index in 0..self.plan.route_count() {
            let r = RouteId::from_index(index).ok_or(IDX)?;
            self.progress(st, now, r, out)?;
        }

        self.drive_level_crossings(st, now, first_cycle, out)?;
        self.drive_signals(st, now, first_cycle, out)?;
        invariants::check(&self.plan, st, now, self.rules.report_timeout())
    }

    // ── Meldungen ────────────────────────────────────────────────────────────────────────

    fn record_reports<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        inputs: &[Input],
        out: &mut O,
    ) {
        for input in inputs {
            let known = match *input {
                Input::Point { point: id, report } => point_mut(st, id)
                    .map(|p| {
                        p.report = Some(report);
                        p.reported_at = now;
                    })
                    .is_ok(),
                Input::Tvd { tvd: id, occupancy } => match tvd(st, id) {
                    Ok(old) => {
                        let entered = old.occupancy == Some(Occupancy::Clear)
                            && occupancy == Occupancy::Occupied;
                        if entered && !self.occupancy_authorized(st, id, &old) {
                            out.event(Event::Alarm(Alarm::UnauthorizedOccupancy(id)));
                        }
                        if let Ok(t) = tvd_mut(st, id) {
                            t.occupancy = Some(occupancy);
                            t.reported_at = now;
                        }
                        true
                    }
                    Err(_) => false,
                },
                Input::Signal { signal: id, report } => match signal_mut(st, id) {
                    Ok(s) => {
                        let newly_defective = report == SignalReport::Defective
                            && s.report != Some(SignalReport::Defective);
                        s.report = Some(report);
                        s.reported_at = now;
                        if newly_defective {
                            out.event(Event::Alarm(Alarm::SignalDefective(id)));
                        }
                        true
                    }
                    Err(_) => false,
                },
                Input::LevelCrossing { lx: id, report } => lx_mut(st, id)
                    .map(|l| {
                        l.report = Some(report);
                        l.reported_at = now;
                    })
                    .is_ok(),
                Input::Request { .. } => true,
            };
            if !known {
                out.event(Event::Alarm(Alarm::UnknownElement));
            }
        }
    }

    /// Darf ein bisher freier Abschnitt jetzt besetzt werden? Nur, wenn eine Fahrt
    /// hineinführt, die erlaubt ist (oder war), oder wenn es ein unverschlossener
    /// Einfahrabschnitt an der Netzgrenze ist (REQ-IXL-011).
    fn occupancy_authorized(&self, st: &KernelState<'_>, t: TvdId, state: &TvdState) -> bool {
        match state.owner {
            Some(owner) => route(st, owner).is_ok_and(|r| match r.phase {
                RoutePhase::Proceed
                | RoutePhase::Occupied
                | RoutePhase::Halted
                | RoutePhase::Cancelling
                | RoutePhase::Arrived => true,
                RoutePhase::Locked => r.substitute_since.is_some(),
                RoutePhase::Idle | RoutePhase::Setting => false,
            }),
            None => state.overlap_owner.is_none() && self.plan.tvd_is_entry(t),
        }
    }

    // ── Anforderungen ────────────────────────────────────────────────────────────────────

    fn handle<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        id: RequestId,
        request: Request,
        out: &mut O,
    ) -> Res<()> {
        let result = match request {
            Request::SetRoute(r) => self.set_route(st, now, r, out)?,
            Request::CancelRoute(r) => self.cancel_route(st, now, r, out)?,
            Request::MovePoint(p, pos) => self.move_point(st, now, p, pos, out)?,
            Request::AuxRequest { op, operator } => self.aux_request(st, now, op, operator, out)?,
            Request::AuxConfirm { aux, operator } => {
                self.aux_confirm(st, now, aux, operator, out)?
            }
        };
        out.event(match result {
            Ok(()) => Event::Accepted(id),
            Err(reason) => Event::Rejected(id, reason),
        });
        Ok(())
    }

    fn tvd_clear(&self, st: &KernelState<'_>, t: TvdId, now: Tick) -> Res<bool> {
        Ok(effective_occupancy(&tvd(st, t)?, now, self.rules.report_timeout()) == Occupancy::Clear)
    }

    fn point_tvd_clear(&self, st: &KernelState<'_>, p: PointId, now: Tick) -> Res<bool> {
        self.tvd_clear(st, self.plan.point_tvd(p).ok_or(IDX)?, now)
    }

    fn in_position(
        &self,
        st: &KernelState<'_>,
        p: PointId,
        pos: PointPosition,
        now: Tick,
    ) -> Res<bool> {
        Ok(effective_position(&point(st, p)?, now, self.rules.report_timeout()) == Some(pos))
    }

    /// Zulassungsprüfung (REQ-IXL-002, REQ-IXL-003, REQ-IXL-012/013).
    fn admissible(
        &self,
        st: &KernelState<'_>,
        now: Tick,
        view: &RouteView<'_>,
    ) -> Res<Result<(), Rejection>> {
        if route(st, view.id())?.phase != RoutePhase::Idle {
            return Ok(Err(Rejection::RouteActive));
        }
        if view.conflicts_with_any(&BitSet::new(st.active)) {
            return Ok(Err(Rejection::Conflict));
        }
        if signal(st, view.start())?.stop_holds > 0 {
            return Ok(Err(Rejection::SignalHeld(view.start())));
        }
        for t in view.tvds() {
            if tvd(st, t)?.owner.is_some() {
                return Ok(Err(Rejection::Conflict));
            }
            if !self.tvd_clear(st, t, now)? {
                return Ok(Err(Rejection::TvdNotClear(t)));
            }
        }
        for t in view.overlap_tvds() {
            if tvd(st, t)?.overlap_owner.is_some() {
                return Ok(Err(Rejection::Conflict));
            }
        }
        for (p, pos) in view.points() {
            let state = point(st, p)?;
            if state.owner.is_some() || state.locked_pos.is_some_and(|lp| lp != pos) {
                return Ok(Err(Rejection::PointLocked(p)));
            }
        }
        for (p, pos) in shared_points(view) {
            let state = point(st, p)?;
            if state.locked_pos.is_some_and(|lp| lp != pos) {
                return Ok(Err(Rejection::PointLocked(p)));
            }
            let must_move = state.owner.is_none() && !self.in_position(st, p, pos, now)?;
            if must_move && !self.point_tvd_clear(st, p, now)? {
                return Ok(Err(Rejection::PointNotOperable(p)));
            }
        }
        Ok(Ok(()))
    }

    /// Zulassung und Einstellen.
    fn set_route<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        r: RouteId,
        out: &mut O,
    ) -> Res<Result<(), Rejection>> {
        let Some(view) = self.plan.route(r) else {
            return Ok(Err(Rejection::UnknownElement));
        };
        if let Err(reason) = self.admissible(st, now, &view)? {
            return Ok(Err(reason));
        }

        // Annehmen: erst Phase, dann Verschlüsse, dann Stellbefehle.
        Self::set_phase(st, now, r, RoutePhase::Setting, out)?;
        let state = route_mut(st, r)?;
        state.next_release = 0;
        state.substitute_since = None;
        BitSetMut::new(st.active)
            .insert(r.index().ok_or(IDX)?)
            .map_err(|_| IDX)?;
        for t in view.tvds() {
            let state = tvd_mut(st, t)?;
            state.owner = Some(r);
            state.seen_occupied = false;
        }
        for t in view.overlap_tvds() {
            tvd_mut(st, t)?.overlap_owner = Some(r);
        }
        for (p, pos) in view.points() {
            let state = point_mut(st, p)?;
            state.owner = Some(r);
            state.locked_pos = Some(pos);
        }
        for (p, pos) in shared_points(&view) {
            let state = point_mut(st, p)?;
            state.holds = inc(state.holds)?;
            state.locked_pos = Some(pos);
        }
        for s in flank_signals(&view) {
            let state = signal_mut(st, s)?;
            state.stop_holds = inc(state.stop_holds)?;
        }
        for l in view.level_crossings() {
            let state = lx_mut(st, l)?;
            state.holds = inc(state.holds)?;
        }

        for (p, pos) in view.points() {
            if !self.in_position(st, p, pos, now)? {
                self.command_point(st, now, p, pos, Some(r), false, out)?;
            }
        }
        for (p, pos) in shared_points(&view) {
            if point(st, p)?.owner.is_none() && !self.in_position(st, p, pos, now)? {
                self.command_point(st, now, p, pos, Some(r), false, out)?;
            }
        }
        Ok(Ok(()))
    }

    fn cancel_route<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        r: RouteId,
        out: &mut O,
    ) -> Res<Result<(), Rejection>> {
        if self.plan.route(r).is_none() {
            return Ok(Err(Rejection::UnknownElement));
        }
        let state = route(st, r)?;
        match state.phase {
            RoutePhase::Idle => Ok(Err(Rejection::RouteNotActive)),
            RoutePhase::Cancelling => Ok(Err(Rejection::RouteActive)),
            RoutePhase::Occupied | RoutePhase::Arrived => Ok(Err(Rejection::TrainInRoute)),
            // Signal hat noch keine Fahrt (und kein Ersatzsignal) gezeigt: sofort auflösen.
            RoutePhase::Setting | RoutePhase::Locked if state.substitute_since.is_none() => {
                self.release_route(st, now, r, out)?;
                Ok(Ok(()))
            }
            // Signal hat Fahrt gezeigt: Halt und Zeitverschluss (REQ-IXL-006).
            RoutePhase::Setting | RoutePhase::Locked | RoutePhase::Proceed | RoutePhase::Halted => {
                Self::set_phase(st, now, r, RoutePhase::Cancelling, out)?;
                Ok(Ok(()))
            }
        }
    }

    fn move_point<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        p: PointId,
        pos: PointPosition,
        out: &mut O,
    ) -> Res<Result<(), Rejection>> {
        let (Some(t), Ok(state)) = (self.plan.point_tvd(p), point(st, p)) else {
            return Ok(Err(Rejection::UnknownElement));
        };
        if state.is_locked() {
            return Ok(Err(Rejection::PointLocked(p)));
        }
        if !self.tvd_clear(st, t, now)? {
            return Ok(Err(Rejection::PointNotOperable(p)));
        }
        self.command_point(st, now, p, pos, None, false, out)?;
        Ok(Ok(()))
    }

    /// Stellbefehl an eine Weiche (REQ-IXL-004). Zulässig nur, wenn
    ///
    /// - die Weiche nicht im Fahrweg verschlossen ist – außer durch die anfordernde
    ///   Fahrstraße, solange sie sich einstellt,
    /// - kein Verschluss eine andere Lage verlangt und
    /// - ihr Abschnitt frei ist (außer bei der Hilfshandlung „Weiche umstellen“).
    #[allow(clippy::too_many_arguments)]
    fn command_point<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        p: PointId,
        pos: PointPosition,
        by: Option<RouteId>,
        forced: bool,
        out: &mut O,
    ) -> Res<()> {
        let state = point(st, p)?;
        if let Some(owner) = state.owner
            && (by != Some(owner) || route(st, owner)?.phase != RoutePhase::Setting)
        {
            return Err(SafeStateReason::LockedPointCommanded(p));
        }
        if state.locked_pos.is_some_and(|lp| lp != pos) {
            return Err(SafeStateReason::LockedPointCommanded(p));
        }
        if !forced && !self.point_tvd_clear(st, p, now)? {
            return Err(SafeStateReason::PointMovedUnderOccupancy(p));
        }
        point_mut(st, p)?.commanded = Some(pos);
        out.command(Command::MovePoint {
            point: p,
            position: pos,
        });
        Ok(())
    }

    // ── Hilfshandlungen ──────────────────────────────────────────────────────────────────

    /// Voraussetzungen einer Hilfshandlung (REQ-IXL-014).
    fn aux_applicable(
        &self,
        st: &KernelState<'_>,
        now: Tick,
        op: AuxOperation,
    ) -> Res<Result<(), Rejection>> {
        let applicable = match op {
            AuxOperation::ReleaseRoute(r) => {
                if self.plan.route(r).is_none() {
                    return Ok(Err(Rejection::UnknownElement));
                }
                route(st, r)?.phase != RoutePhase::Idle
            }
            AuxOperation::ResetTvd(t) => {
                let Ok(state) = tvd(st, t) else {
                    return Ok(Err(Rejection::UnknownElement));
                };
                state.occupancy == Some(Occupancy::Disturbed)
                    && state.owner.is_none()
                    && state.overlap_owner.is_none()
            }
            AuxOperation::Substitute(r) => {
                let Some(view) = self.plan.route(r) else {
                    return Ok(Err(Rejection::UnknownElement));
                };
                matches!(route(st, r)?.phase, RoutePhase::Locked | RoutePhase::Halted)
                    && self.substitute_possible(st, &view, now)?
            }
            AuxOperation::ForcePoint(p, _) => {
                let (Some(t), Ok(state)) = (self.plan.point_tvd(p), point(st, p)) else {
                    return Ok(Err(Rejection::UnknownElement));
                };
                let occupancy = effective_occupancy(&tvd(st, t)?, now, self.rules.report_timeout());
                !state.is_locked() && occupancy == Occupancy::Disturbed
            }
        };
        Ok(if applicable {
            Ok(())
        } else {
            Err(Rejection::AuxNotApplicable)
        })
    }

    fn aux_request<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        op: AuxOperation,
        operator: OperatorId,
        out: &mut O,
    ) -> Res<Result<(), Rejection>> {
        if let Err(reason) = self.aux_applicable(st, now, op)? {
            return Ok(Err(reason));
        }
        let Some(slot) = st.aux.iter_mut().find(|s| s.is_none()) else {
            return Ok(Err(Rejection::AuxBusy));
        };
        let id = AuxId(st.meta.next_aux);
        *slot = Some(AuxSlot {
            id,
            op,
            operator,
            since: now,
        });
        st.meta.next_aux = st.meta.next_aux.wrapping_add(1);
        out.event(Event::AuxPending {
            aux: id,
            op,
            operator,
        });
        Ok(Ok(()))
    }

    fn aux_confirm<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        aux: AuxId,
        operator: OperatorId,
        out: &mut O,
    ) -> Res<Result<(), Rejection>> {
        let Some(slot) = st
            .aux
            .iter_mut()
            .find(|s| matches!(s, Some(s) if s.id == aux))
        else {
            return Ok(Err(Rejection::AuxUnknown));
        };
        let Some(pending) = *slot else {
            return Err(IDX);
        };
        if now.saturating_since(pending.since) > self.rules.aux_confirm_timeout() {
            *slot = None;
            out.event(Event::AuxExpired(aux));
            return Ok(Err(Rejection::AuxExpired));
        }
        if pending.operator == operator {
            return Ok(Err(Rejection::SameOperator));
        }
        *slot = None;
        if let Err(reason) = self.aux_applicable(st, now, pending.op)? {
            return Ok(Err(reason));
        }
        self.aux_execute(st, now, pending.op, out)?;
        st.meta.aux_counter = st.meta.aux_counter.saturating_add(1);
        out.event(Event::AuxExecuted {
            aux,
            op: pending.op,
            counter: st.meta.aux_counter,
        });
        Ok(Ok(()))
    }

    fn aux_execute<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        op: AuxOperation,
        out: &mut O,
    ) -> Res<()> {
        match op {
            AuxOperation::ReleaseRoute(r) => self.release_route(st, now, r, out),
            AuxOperation::ResetTvd(t) => {
                out.command(Command::ResetTvd(t));
                Ok(())
            }
            AuxOperation::Substitute(r) => {
                // Das Signal schaltet `drive_signals` ein, nachdem es die Bedingungen erneut
                // geprüft hat.
                route_mut(st, r)?.substitute_since = Some(now);
                Ok(())
            }
            AuxOperation::ForcePoint(p, pos) => {
                self.command_point(st, now, p, pos, None, true, out)
            }
        }
    }

    /// Nicht bestätigte Hilfshandlungen verfallen nach der Bestätigungsfrist.
    fn expire_aux<O: Outputs>(&self, st: &mut KernelState<'_>, now: Tick, out: &mut O) {
        let timeout = self.rules.aux_confirm_timeout();
        for slot in st.aux.iter_mut() {
            if let Some(pending) = *slot
                && now.saturating_since(pending.since) > timeout
            {
                *slot = None;
                out.event(Event::AuxExpired(pending.id));
            }
        }
    }

    // ── Fahrstraßen fortschalten ─────────────────────────────────────────────────────────

    fn set_phase<O: Outputs>(
        st: &mut KernelState<'_>,
        now: Tick,
        r: RouteId,
        phase: RoutePhase,
        out: &mut O,
    ) -> Res<()> {
        let state = route_mut(st, r)?;
        if state.phase != phase {
            state.phase = phase;
            state.since = now;
            out.event(Event::RoutePhase(r, phase));
        }
        Ok(())
    }

    fn view(&self, r: RouteId) -> Res<RouteView<'p>> {
        self.plan.route(r).ok_or(IDX)
    }

    /// Alle Weichen der Fahrstraße (Fahrweg, Durchrutschweg, Flankenschutz) liegen
    /// überwacht in der geforderten Lage.
    fn points_in_position(
        &self,
        st: &KernelState<'_>,
        view: &RouteView<'_>,
        now: Tick,
    ) -> Res<bool> {
        for (p, pos) in all_points(view) {
            if !self.in_position(st, p, pos, now)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Bedingungen, die Fahrtbegriff und Ersatzsignal gemeinsam haben: Weichen in Lage,
    /// Schutzsignale in Halt, Bahnübergänge geschlossen, Startsignal nicht gehalten.
    fn route_secured(&self, st: &KernelState<'_>, view: &RouteView<'_>, now: Tick) -> Res<bool> {
        let timeout = self.rules.report_timeout();
        if !self.points_in_position(st, view, now)? {
            return Ok(false);
        }
        for s in flank_signals(view) {
            let state = signal(st, s)?;
            if state.stop_holds == 0 || state.report == Some(SignalReport::Proceed) {
                return Ok(false);
            }
        }
        for l in view.level_crossings() {
            if !lx_closed(&lx(st, l)?, now, timeout) {
                return Ok(false);
            }
        }
        Ok(signal(st, view.start())?.stop_holds == 0)
    }

    /// Alle Bedingungen für die Fahrerlaubnis (REQ-IXL-005).
    fn proceed_conditions(
        &self,
        st: &KernelState<'_>,
        view: &RouteView<'_>,
        now: Tick,
    ) -> Res<bool> {
        if !self.route_secured(st, view, now)? {
            return Ok(false);
        }
        for t in view.tvds().chain(view.overlap_tvds()) {
            if !self.tvd_clear(st, t, now)? {
                return Ok(false);
            }
        }
        Ok(signal(st, view.start())?.report != Some(SignalReport::Defective))
    }

    /// Bedingungen für das Ersatzsignal: wie Fahrt, aber ohne Gleisfreimeldung und
    /// unabhängig vom Zustand des Hauptsignals (REQ-IXL-015).
    fn substitute_possible(
        &self,
        st: &KernelState<'_>,
        view: &RouteView<'_>,
        now: Tick,
    ) -> Res<bool> {
        self.route_secured(st, view, now)
    }

    /// Hat ein Zug den ersten Abschnitt der Fahrstraße erreicht?
    fn train_entered(&self, st: &KernelState<'_>, view: &RouteView<'_>, now: Tick) -> Res<bool> {
        let Some(first) = view.tvds().next() else {
            return Err(IDX);
        };
        Ok(
            effective_occupancy(&tvd(st, first)?, now, self.rules.report_timeout())
                == Occupancy::Occupied,
        )
    }

    /// Ist irgendein Abschnitt des Fahrwegs besetzt? Nach Ersatzsignal kann der erste
    /// Abschnitt gestört sein; der Zug wird dann im nächsten erkannt.
    fn any_path_occupied(
        &self,
        st: &KernelState<'_>,
        view: &RouteView<'_>,
        now: Tick,
    ) -> Res<bool> {
        let timeout = self.rules.report_timeout();
        for t in view.tvds() {
            if effective_occupancy(&tvd(st, t)?, now, timeout) == Occupancy::Occupied {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn enter<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        r: RouteId,
        view: &RouteView<'_>,
        out: &mut O,
    ) -> Res<()> {
        route_mut(st, r)?.substitute_since = None;
        Self::set_phase(st, now, r, RoutePhase::Occupied, out)?;
        self.release_behind_train(st, now, r, view, out)
    }

    fn progress<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        r: RouteId,
        out: &mut O,
    ) -> Res<()> {
        let state = route(st, r)?;
        if state.phase == RoutePhase::Idle {
            return Ok(());
        }
        let view = self.view(r)?;
        let elapsed = now.saturating_since(state.since);
        let substitute = state.substitute_since.is_some();
        match state.phase {
            RoutePhase::Idle => {}
            RoutePhase::Setting => {
                if self.train_entered(st, &view, now)? {
                    out.event(Event::Alarm(Alarm::SignalPassedAtDanger(r)));
                    self.enter(st, now, r, &view, out)?;
                } else if self.points_in_position(st, &view, now)? {
                    Self::set_phase(st, now, r, RoutePhase::Locked, out)?;
                } else if elapsed > self.rules.point_move_timeout() {
                    out.event(Event::Alarm(Alarm::SettingTimeout(r)));
                    self.release_route(st, now, r, out)?;
                }
            }
            RoutePhase::Locked => {
                if substitute && self.any_path_occupied(st, &view, now)? {
                    self.enter(st, now, r, &view, out)?;
                } else if self.train_entered(st, &view, now)? {
                    out.event(Event::Alarm(Alarm::SignalPassedAtDanger(r)));
                    self.enter(st, now, r, &view, out)?;
                } else if self.proceed_conditions(st, &view, now)? {
                    route_mut(st, r)?.substitute_since = None;
                    Self::set_phase(st, now, r, RoutePhase::Proceed, out)?;
                }
            }
            RoutePhase::Proceed => {
                if self.train_entered(st, &view, now)? {
                    self.enter(st, now, r, &view, out)?;
                } else if !self.proceed_conditions(st, &view, now)? {
                    out.event(Event::Alarm(Alarm::SignalDropped(r)));
                    Self::set_phase(st, now, r, RoutePhase::Halted, out)?;
                }
            }
            RoutePhase::Halted => {
                if self.train_entered(st, &view, now)?
                    || (substitute && self.any_path_occupied(st, &view, now)?)
                {
                    self.enter(st, now, r, &view, out)?;
                }
            }
            RoutePhase::Cancelling => {
                if self.train_entered(st, &view, now)? {
                    out.event(Event::Alarm(Alarm::TrainEnteredCancelledRoute(r)));
                    self.enter(st, now, r, &view, out)?;
                } else if elapsed >= self.rules.cancel_time_lock() {
                    self.release_route(st, now, r, out)?;
                }
            }
            RoutePhase::Occupied => self.release_behind_train(st, now, r, &view, out)?,
            RoutePhase::Arrived => {
                if elapsed >= self.rules.overlap_release_time()
                    && self.overlap_vacated(st, &view, now)?
                {
                    self.release_route(st, now, r, out)?;
                }
            }
        }
        Ok(())
    }

    /// Kein Abschnitt des Durchrutschwegs ist besetzt – außer durch eine Fahrt auf einer
    /// anschließenden Fahrstraße, die ihn selbst verschlossen hat.
    fn overlap_vacated(&self, st: &KernelState<'_>, view: &RouteView<'_>, now: Tick) -> Res<bool> {
        for t in view.overlap_tvds() {
            if tvd(st, t)?.owner.is_none() && !self.tvd_clear(st, t, now)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Zugbewirkte Auflösung (REQ-IXL-007).
    ///
    /// - Ein Abschnitt wird freigegeben, wenn der Zug ihn befahren und wieder verlassen hat
    ///   und bereits im folgenden Abschnitt ist; mit ihm seine Weichen, die ihn schützenden
    ///   Flankenschutzmaßnahmen und die Bahnübergänge darin.
    /// - Erreicht der Zug den letzten Abschnitt, wird der ganze Fahrweg freigegeben; die
    ///   Besetzung schützt die Abschnitte danach weiter. Ohne Durchrutschweg ist die
    ///   Fahrstraße damit aufgelöst, sonst wechselt sie nach „angekommen“ und hält den
    ///   Durchrutschweg bis zu dessen Auflösezeit.
    fn release_behind_train<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        r: RouteId,
        view: &RouteView<'_>,
        out: &mut O,
    ) -> Res<()> {
        let timeout = self.rules.report_timeout();
        for t in view.tvds() {
            let state = tvd_mut(st, t)?;
            if state.owner == Some(r)
                && effective_occupancy(state, now, timeout) == Occupancy::Occupied
            {
                state.seen_occupied = true;
            }
        }

        let count = view.tvds().len();
        let last = view.tvds().last().ok_or(IDX)?;
        let last_state = tvd(st, last)?;
        if last_state.owner == Some(r) && last_state.seen_occupied {
            return self.arrive(st, now, r, view, out);
        }

        loop {
            let i = released_count(&route(st, r)?)?;
            let next_index = i.checked_add(1).ok_or(IDX)?;
            if next_index >= count {
                break;
            }
            let current = view.tvds().nth(i).ok_or(IDX)?;
            let next = view.tvds().nth(next_index).ok_or(IDX)?;
            let seen = tvd(st, current)?.seen_occupied;
            let clear = self.tvd_clear(st, current, now)?;
            if !(seen && clear && tvd(st, next)?.seen_occupied) {
                break;
            }
            self.release_path(st, r, view, next_index)?;
        }
        Ok(())
    }

    fn arrive<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        r: RouteId,
        view: &RouteView<'_>,
        out: &mut O,
    ) -> Res<()> {
        if view.overlap_tvds().len() == 0 {
            return self.release_route(st, now, r, out);
        }
        self.release_path(st, r, view, view.tvds().len())?;
        Self::set_phase(st, now, r, RoutePhase::Arrived, out)
    }

    /// Gibt die Abschnitte des Fahrwegs bis (ausschließlich) `upto` frei.
    fn release_path(
        &self,
        st: &mut KernelState<'_>,
        r: RouteId,
        view: &RouteView<'_>,
        upto: usize,
    ) -> Res<()> {
        let from = released_count(&route(st, r)?)?;
        for t in view.tvds().take(upto).skip(from) {
            self.release_tvd(st, r, view, t)?;
        }
        if upto > from {
            route_mut(st, r)?.next_release = u32::try_from(upto).map_err(|_| IDX)?;
        }
        Ok(())
    }

    /// Gibt einen Abschnitt des Fahrwegs frei, mit seinen Weichen, den Flankenschutz-
    /// maßnahmen, die ihn schützen, und den Bahnübergängen darin.
    fn release_tvd(
        &self,
        st: &mut KernelState<'_>,
        r: RouteId,
        view: &RouteView<'_>,
        t: TvdId,
    ) -> Res<()> {
        let state = tvd_mut(st, t)?;
        if state.owner != Some(r) {
            return Err(SafeStateReason::MissingLock(r));
        }
        state.owner = None;
        state.seen_occupied = false;
        for (p, _) in view.points() {
            if self.plan.point_tvd(p) == Some(t) {
                let ps = point_mut(st, p)?;
                if ps.owner != Some(r) {
                    return Err(SafeStateReason::MissingLock(r));
                }
                ps.owner = None;
                if ps.holds == 0 {
                    ps.locked_pos = None;
                }
            }
        }
        for f in view.flank() {
            if f.protects == t {
                Self::release_protection(st, f.protection)?;
            }
        }
        for l in view.level_crossings() {
            if self.plan.lx_tvd(l) == Some(t) {
                let ls = lx_mut(st, l)?;
                ls.holds = dec(ls.holds)?;
            }
        }
        Ok(())
    }

    fn release_protection(st: &mut KernelState<'_>, protection: Protection) -> Res<()> {
        match protection {
            Protection::Point(p, _) => Self::unhold_point(st, p),
            Protection::Signal(s) => {
                let state = signal_mut(st, s)?;
                state.stop_holds = dec(state.stop_holds)?;
                Ok(())
            }
        }
    }

    fn unhold_point(st: &mut KernelState<'_>, p: PointId) -> Res<()> {
        let state = point_mut(st, p)?;
        state.holds = dec(state.holds)?;
        if state.holds == 0 && state.owner.is_none() {
            state.locked_pos = None;
        }
        Ok(())
    }

    /// Gibt den Durchrutschweg frei, mit den Flankenschutzmaßnahmen und Bahnübergängen,
    /// die nicht an einem Abschnitt des Fahrwegs hängen.
    fn release_overlap(
        &self,
        st: &mut KernelState<'_>,
        r: RouteId,
        view: &RouteView<'_>,
    ) -> Res<()> {
        for t in view.overlap_tvds() {
            let state = tvd_mut(st, t)?;
            if state.overlap_owner != Some(r) {
                return Err(SafeStateReason::MissingLock(r));
            }
            state.overlap_owner = None;
        }
        for (p, _) in view.overlap_points() {
            Self::unhold_point(st, p)?;
        }
        for f in view.flank() {
            if !in_path(view, f.protects) {
                Self::release_protection(st, f.protection)?;
            }
        }
        for l in view.level_crossings() {
            if !self.plan.lx_tvd(l).is_some_and(|t| in_path(view, t)) {
                let ls = lx_mut(st, l)?;
                ls.holds = dec(ls.holds)?;
            }
        }
        Ok(())
    }

    /// Löst die Fahrstraße vollständig auf: restlicher Fahrweg, Durchrutschweg,
    /// Flankenschutz, Bahnübergänge, Ersatzsignal.
    fn release_route<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        r: RouteId,
        out: &mut O,
    ) -> Res<()> {
        let view = self.view(r)?;
        self.release_path(st, r, &view, view.tvds().len())?;
        self.release_overlap(st, r, &view)?;
        BitSetMut::new(st.active)
            .remove(r.index().ok_or(IDX)?)
            .map_err(|_| IDX)?;
        let state = route_mut(st, r)?;
        state.next_release = 0;
        state.substitute_since = None;
        Self::set_phase(st, now, r, RoutePhase::Idle, out)
    }

    // ── Bahnübergänge ────────────────────────────────────────────────────────────────────

    /// Ein Bahnübergang ist geschlossen, solange eine Fahrstraße ihn braucht, und bleibt
    /// danach geschlossen, bis sein Abschnitt frei ist (REQ-IXL-016).
    fn drive_level_crossings<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        first_cycle: bool,
        out: &mut O,
    ) -> Res<()> {
        for index in 0..self.plan.lx_count() {
            let id = LxId::from_index(index).ok_or(IDX)?;
            let t = self.plan.lx_tvd(id).ok_or(IDX)?;
            let occupied = !self.tvd_clear(st, t, now)?;
            let state = lx_mut(st, id)?;
            let close = state.holds > 0 || (state.closed_commanded && occupied);
            if first_cycle || close != state.closed_commanded {
                state.closed_commanded = close;
                out.command(Command::SetLevelCrossing { lx: id, close });
            }
        }
        Ok(())
    }

    // ── Signale ──────────────────────────────────────────────────────────────────────────

    fn drive_signals<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        first_cycle: bool,
        out: &mut O,
    ) -> Res<()> {
        for s in st.signals.iter_mut() {
            s.want = Aspect::Stop;
            s.want_substitute = false;
        }
        for index in 0..self.plan.route_count() {
            let r = RouteId::from_index(index).ok_or(IDX)?;
            let state = route(st, r)?;
            if state.phase == RoutePhase::Idle {
                continue;
            }
            let view = self.view(r)?;
            if let Some(since) = state.substitute_since {
                let valid = matches!(state.phase, RoutePhase::Locked | RoutePhase::Halted)
                    && now.saturating_since(since) < self.rules.substitute_duration()
                    && self.substitute_possible(st, &view, now)?;
                if valid {
                    signal_mut(st, view.start())?.want_substitute = true;
                } else {
                    route_mut(st, r)?.substitute_since = None;
                }
            }
            // Doppelte Absicherung: Fahrt nur bei aktuell erfüllten Bedingungen.
            if state.phase == RoutePhase::Proceed && self.proceed_conditions(st, &view, now)? {
                signal_mut(st, view.start())?.want = Aspect::Proceed;
            }
        }

        let timeout = self.rules.report_timeout();
        for index in 0..st.signals.len() {
            let id = SignalId::from_index(index).ok_or(IDX)?;
            let s = signal_mut(st, id)?;
            if first_cycle || s.want != s.commanded {
                s.commanded = s.want;
                s.commanded_at = now;
                s.mismatch_alarmed = false;
                out.command(Command::SetSignal {
                    signal: id,
                    aspect: s.want,
                });
            }
            if first_cycle || s.want_substitute != s.substitute {
                s.substitute = s.want_substitute;
                out.command(Command::SetSubstitute {
                    signal: id,
                    on: s.substitute,
                });
            }
            let mismatch = s.commanded == Aspect::Stop
                && s.report == Some(SignalReport::Proceed)
                && !fresh(s.commanded_at, now, timeout);
            if mismatch && !s.mismatch_alarmed {
                s.mismatch_alarmed = true;
                out.event(Event::Alarm(Alarm::SignalMismatch(id)));
            }
        }
        Ok(())
    }

    // ── Sicherer Zustand ─────────────────────────────────────────────────────────────────

    fn reject_all<O: Outputs>(inputs: &[Input], reason: Rejection, out: &mut O) {
        for input in inputs {
            if let Input::Request { id, .. } = *input {
                out.event(Event::Rejected(id, reason));
            }
        }
    }

    /// Alle Signale auf Halt, Ersatzsignale aus; beim Eintritt in den sicheren Zustand für
    /// jedes Signal.
    fn hold_stop<O: Outputs>(st: &mut KernelState<'_>, now: Tick, all: bool, out: &mut O) {
        for (index, s) in st.signals.iter_mut().enumerate() {
            let Some(id) = SignalId::from_index(index) else {
                continue;
            };
            if all || s.commanded != Aspect::Stop {
                s.commanded = Aspect::Stop;
                s.commanded_at = now;
                out.command(Command::SetSignal {
                    signal: id,
                    aspect: Aspect::Stop,
                });
            }
            if all || s.substitute {
                s.substitute = false;
                out.command(Command::SetSubstitute {
                    signal: id,
                    on: false,
                });
            }
        }
    }
}
