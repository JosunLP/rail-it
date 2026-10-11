//! Zyklus des Sicherheitskerns.

use railit_plan::{PlanView, RouteView};
use railit_rules::RuleSet;
use railit_types::bitset::{BitSet, BitSetMut};
use railit_types::{PointId, PointPosition, RouteId, SignalId, Tick, TvdId};

use crate::invariants;
use crate::io::{
    Alarm, Aspect, Command, Event, Input, Occupancy, Outputs, Rejection, Request, RequestId,
    RoutePhase, SafeStateReason, SignalReport,
};
use crate::state::{KernelState, PointState, RouteState, SignalState, TvdState};

type Res<T> = Result<T, SafeStateReason>;

const IDX: SafeStateReason = SafeStateReason::InternalIndex;

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
    /// Reihenfolge: Meldungen übernehmen → Anforderungen bearbeiten → Fahrstraßen
    /// fortschalten → Signale stellen → Invarianten prüfen.
    ///
    /// # Errors
    /// [`SafeStateReason`], wenn der Kern im sicheren Zustand ist oder in diesem Zyklus
    /// hineingeht. Dann zeigen alle Signale Halt, es werden keine Weichen mehr gestellt und
    /// alle Anforderungen abgelehnt, bis der Zustand zurückgesetzt wird (REQ-IXL-008).
    pub fn step<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        inputs: &[Input],
        out: &mut O,
    ) -> Result<(), SafeStateReason> {
        if let Some(reason) = st.meta.safe_state {
            Self::record_reports(st, now, inputs, out);
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
        Self::record_reports(st, now, inputs, out);

        for input in inputs {
            if let Input::Request { id, request } = *input {
                self.handle(st, now, id, request, out)?;
            }
        }

        for index in 0..self.plan.route_count() {
            let r = RouteId::from_index(index).ok_or(IDX)?;
            self.progress(st, now, r, out)?;
        }

        self.drive_signals(st, now, first_cycle, out)?;
        invariants::check(&self.plan, st, now, self.rules.report_timeout())
    }

    // ── Meldungen ────────────────────────────────────────────────────────────────────────

    fn record_reports<O: Outputs>(
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
                Input::Tvd { tvd: id, occupancy } => tvd_mut(st, id)
                    .map(|t| {
                        t.occupancy = Some(occupancy);
                        t.reported_at = now;
                    })
                    .is_ok(),
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
                Input::Request { .. } => true,
            };
            if !known {
                out.event(Event::Alarm(Alarm::UnknownElement));
            }
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
        };
        out.event(match result {
            Ok(()) => Event::Accepted(id),
            Err(reason) => Event::Rejected(id, reason),
        });
        Ok(())
    }

    /// Zulassungsprüfung und Einstellen (REQ-IXL-002, REQ-IXL-003).
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
        if route(st, r)?.phase != RoutePhase::Idle {
            return Ok(Err(Rejection::RouteActive));
        }
        if view.conflicts_with_any(&BitSet::new(st.active)) {
            return Ok(Err(Rejection::Conflict));
        }
        let timeout = self.rules.report_timeout();
        for t in view.tvds() {
            let state = tvd(st, t)?;
            if state.owner.is_some() {
                return Ok(Err(Rejection::Conflict));
            }
            if effective_occupancy(&state, now, timeout) != Occupancy::Clear {
                return Ok(Err(Rejection::TvdNotClear(t)));
            }
        }
        for (p, _) in view.points() {
            if point(st, p)?.owner.is_some() {
                return Ok(Err(Rejection::PointLocked(p)));
            }
        }

        // Annehmen: erst Phase, dann Verschlüsse, dann Stellbefehle.
        Self::set_phase(st, now, r, RoutePhase::Setting, out)?;
        route_mut(st, r)?.next_release = 0;
        BitSetMut::new(st.active)
            .insert(r.index().ok_or(IDX)?)
            .map_err(|_| IDX)?;
        for t in view.tvds() {
            let state = tvd_mut(st, t)?;
            state.owner = Some(r);
            state.seen_occupied = false;
        }
        for (p, pos) in view.points() {
            point_mut(st, p)?.owner = Some(r);
            if effective_position(&point(st, p)?, now, timeout) != Some(pos) {
                Self::command_point(st, p, pos, out)?;
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
        match route(st, r)?.phase {
            RoutePhase::Idle => Ok(Err(Rejection::RouteNotActive)),
            RoutePhase::Cancelling => Ok(Err(Rejection::RouteActive)),
            RoutePhase::Occupied => Ok(Err(Rejection::TrainInRoute)),
            // Signal hat noch keine Fahrt gezeigt: sofort auflösen.
            RoutePhase::Setting | RoutePhase::Locked => {
                Self::release_route(st, now, r, out)?;
                Ok(Ok(()))
            }
            // Signal hat Fahrt gezeigt: Halt und Zeitverschluss (REQ-IXL-006).
            RoutePhase::Proceed | RoutePhase::Halted => {
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
        if state.owner.is_some() {
            return Ok(Err(Rejection::PointLocked(p)));
        }
        if effective_occupancy(&tvd(st, t)?, now, self.rules.report_timeout()) != Occupancy::Clear {
            return Ok(Err(Rejection::PointNotOperable(p)));
        }
        Self::command_point(st, p, pos, out)?;
        Ok(Ok(()))
    }

    /// Stellbefehl an eine Weiche – nur, wenn sie frei ist oder gerade von einer Fahrstraße
    /// in Einstellung reserviert wurde (REQ-IXL-004).
    fn command_point<O: Outputs>(
        st: &mut KernelState<'_>,
        p: PointId,
        pos: PointPosition,
        out: &mut O,
    ) -> Res<()> {
        if let Some(owner) = point(st, p)?.owner
            && route(st, owner)?.phase != RoutePhase::Setting
        {
            return Err(SafeStateReason::LockedPointCommanded(p));
        }
        point_mut(st, p)?.commanded = Some(pos);
        out.command(Command::MovePoint {
            point: p,
            position: pos,
        });
        Ok(())
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

    fn points_in_position(
        &self,
        st: &KernelState<'_>,
        view: &RouteView<'_>,
        now: Tick,
    ) -> Res<bool> {
        let timeout = self.rules.report_timeout();
        for (p, pos) in view.points() {
            if effective_position(&point(st, p)?, now, timeout) != Some(pos) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Alle Bedingungen für die Fahrerlaubnis (REQ-IXL-005).
    fn proceed_conditions(
        &self,
        st: &KernelState<'_>,
        view: &RouteView<'_>,
        now: Tick,
    ) -> Res<bool> {
        let timeout = self.rules.report_timeout();
        if !self.points_in_position(st, view, now)? {
            return Ok(false);
        }
        for t in view.tvds() {
            if effective_occupancy(&tvd(st, t)?, now, timeout) != Occupancy::Clear {
                return Ok(false);
            }
        }
        Ok(signal(st, view.start())?.report != Some(SignalReport::Defective))
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
        match state.phase {
            RoutePhase::Idle => {}
            RoutePhase::Setting => {
                if self.points_in_position(st, &view, now)? {
                    Self::set_phase(st, now, r, RoutePhase::Locked, out)?;
                } else if elapsed > self.rules.point_move_timeout() {
                    out.event(Event::Alarm(Alarm::SettingTimeout(r)));
                    Self::release_route(st, now, r, out)?;
                }
            }
            RoutePhase::Locked => {
                if self.proceed_conditions(st, &view, now)? {
                    Self::set_phase(st, now, r, RoutePhase::Proceed, out)?;
                }
            }
            RoutePhase::Proceed => {
                if self.train_entered(st, &view, now)? {
                    Self::set_phase(st, now, r, RoutePhase::Occupied, out)?;
                    self.release_behind_train(st, now, r, &view, out)?;
                } else if !self.proceed_conditions(st, &view, now)? {
                    out.event(Event::Alarm(Alarm::SignalDropped(r)));
                    Self::set_phase(st, now, r, RoutePhase::Halted, out)?;
                }
            }
            RoutePhase::Halted => {
                if self.train_entered(st, &view, now)? {
                    Self::set_phase(st, now, r, RoutePhase::Occupied, out)?;
                    self.release_behind_train(st, now, r, &view, out)?;
                }
            }
            RoutePhase::Cancelling => {
                if self.train_entered(st, &view, now)? {
                    out.event(Event::Alarm(Alarm::TrainEnteredCancelledRoute(r)));
                    Self::set_phase(st, now, r, RoutePhase::Occupied, out)?;
                    self.release_behind_train(st, now, r, &view, out)?;
                } else if elapsed >= self.rules.cancel_time_lock() {
                    Self::release_route(st, now, r, out)?;
                }
            }
            RoutePhase::Occupied => self.release_behind_train(st, now, r, &view, out)?,
        }
        Ok(())
    }

    /// Zugbewirkte Teilauflösung (REQ-IXL-007): Ein Abschnitt wird freigegeben, wenn der Zug
    /// ihn befahren und wieder verlassen hat und bereits im folgenden Abschnitt ist. Der
    /// letzte Abschnitt wird freigegeben, sobald der Zug ihn erreicht hat; seine Besetzung
    /// schützt ihn danach weiter.
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
        loop {
            let i = usize::try_from(route(st, r)?.next_release).map_err(|_| IDX)?;
            let Some(current) = view.tvds().nth(i) else {
                break;
            };
            let seen = tvd(st, current)?.seen_occupied;
            let next_index = i.checked_add(1).ok_or(IDX)?;
            if next_index == count {
                if seen {
                    Self::release_route(st, now, r, out)?;
                }
                break;
            }
            let next = view.tvds().nth(next_index).ok_or(IDX)?;
            let clear = effective_occupancy(&tvd(st, current)?, now, timeout) == Occupancy::Clear;
            if !(seen && clear && tvd(st, next)?.seen_occupied) {
                break;
            }
            self.release_tvd(st, r, current)?;
            let state = route_mut(st, r)?;
            state.next_release = state.next_release.checked_add(1).ok_or(IDX)?;
        }
        Ok(())
    }

    /// Gibt einen Abschnitt und die darin liegenden Weichen der Fahrstraße frei.
    fn release_tvd(&self, st: &mut KernelState<'_>, r: RouteId, t: TvdId) -> Res<()> {
        let state = tvd_mut(st, t)?;
        if state.owner == Some(r) {
            state.owner = None;
            state.seen_occupied = false;
        }
        for (p, _) in self.view(r)?.points() {
            if self.plan.point_tvd(p) == Some(t) {
                let ps = point_mut(st, p)?;
                if ps.owner == Some(r) {
                    ps.owner = None;
                }
            }
        }
        Ok(())
    }

    /// Löst die Fahrstraße vollständig auf.
    fn release_route<O: Outputs>(
        st: &mut KernelState<'_>,
        now: Tick,
        r: RouteId,
        out: &mut O,
    ) -> Res<()> {
        for t in st.tvds.iter_mut().filter(|t| t.owner == Some(r)) {
            t.owner = None;
            t.seen_occupied = false;
        }
        for p in st.points.iter_mut().filter(|p| p.owner == Some(r)) {
            p.owner = None;
        }
        BitSetMut::new(st.active)
            .remove(r.index().ok_or(IDX)?)
            .map_err(|_| IDX)?;
        route_mut(st, r)?.next_release = 0;
        Self::set_phase(st, now, r, RoutePhase::Idle, out)
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
        }
        for index in 0..self.plan.route_count() {
            let r = RouteId::from_index(index).ok_or(IDX)?;
            if route(st, r)?.phase != RoutePhase::Proceed {
                continue;
            }
            let view = self.view(r)?;
            // Doppelte Absicherung: Fahrt nur bei aktuell erfüllten Bedingungen.
            if self.proceed_conditions(st, &view, now)? {
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

    /// Alle Signale auf Halt; beim Eintritt in den sicheren Zustand für jedes Signal.
    fn hold_stop<O: Outputs>(st: &mut KernelState<'_>, now: Tick, all: bool, out: &mut O) {
        for (index, s) in st.signals.iter_mut().enumerate() {
            if all || s.commanded != Aspect::Stop {
                s.commanded = Aspect::Stop;
                s.commanded_at = now;
                if let Some(id) = SignalId::from_index(index) {
                    out.command(Command::SetSignal {
                        signal: id,
                        aspect: Aspect::Stop,
                    });
                }
            }
        }
    }
}
