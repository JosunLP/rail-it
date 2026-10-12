//! Sicherheitsinvarianten, geprüft nach jedem Zyklus (REQ-IXL-008).
//!
//! Die Prüfungen sind bewusst unabhängig von der Logik in [`crate::kernel`] formuliert:
//! Sie beschreiben, was **nie** gelten darf, nicht wie der Kern es verhindert. Insbesondere
//! werden alle Verschlüsse aus den Phasen und dem Auflösefortschritt der Fahrstraßen neu
//! berechnet und mit den Zählern im Zustand verglichen. Eine Verletzung führt in den
//! sicheren Zustand.

use railit_plan::{PlanView, Protection, RouteEnd, RouteView};
use railit_types::bitset::BitSet;
use railit_types::{
    BorderId, Duration, LxId, PointId, PointPosition, RouteId, SignalId, Tick, TvdId,
};

use crate::io::{Aspect, Occupancy, RoutePhase, SafeStateReason, SignalReport};
use crate::kernel::{
    border, effective_occupancy, effective_position, fresh, lx, lx_closed, point, route, signal,
    tvd,
};
use crate::state::{KernelState, RouteState};

type Res<T> = Result<T, SafeStateReason>;

const IDX: SafeStateReason = SafeStateReason::InternalIndex;
const HOLD: SafeStateReason = SafeStateReason::HoldMismatch;

fn route_ids(plan: &PlanView<'_>) -> impl Iterator<Item = RouteId> {
    (0..plan.route_count()).filter_map(RouteId::from_index)
}

fn border_ids(plan: &PlanView<'_>) -> impl Iterator<Item = BorderId> {
    (0..plan.border_count()).filter_map(BorderId::from_index)
}

/// Grenzen, deren Annäherungsabschnitt verschlossen ist.
fn locked_borders<'a>(
    plan: &'a PlanView<'_>,
    st: &'a KernelState<'_>,
) -> impl Iterator<Item = BorderId> + 'a {
    border_ids(plan).filter(move |b| border(st, *b).is_ok_and(|s| s.lock))
}

/// Eingestellte Fahrstraßen mit Zustand und Plansicht.
fn active_routes<'a, 'p>(
    plan: &'a PlanView<'p>,
    st: &'a KernelState<'_>,
) -> impl Iterator<Item = (RouteId, RouteState, RouteView<'p>)> + 'a {
    route_ids(plan).filter_map(move |r| {
        let state = route(st, r).ok()?;
        if state.phase == RoutePhase::Idle {
            return None;
        }
        Some((r, state, plan.route(r)?))
    })
}

/// Wurde der Abschnitt `t` des Fahrwegs bereits aufgelöst?
fn released(view: &RouteView<'_>, state: &RouteState, t: TvdId) -> bool {
    let n = usize::try_from(state.next_release).unwrap_or(usize::MAX);
    view.tvds().take(n).any(|x| x == t)
}

/// Prüft alle Invarianten.
pub(crate) fn check(
    plan: &PlanView<'_>,
    st: &KernelState<'_>,
    now: Tick,
    timeout: Duration,
) -> Res<()> {
    active_set_matches_phases(plan, st)?;
    no_conflicting_routes(plan, st)?;
    tvd_locks_match_routes(plan, st)?;
    point_locks_match_routes(plan, st)?;
    signal_holds_match_routes(plan, st)?;
    level_crossings_match_routes(plan, st)?;
    borders_are_consistent(plan, st)?;
    substitute_only_when_justified(plan, st, now, timeout)?;
    proceed_only_when_safe(plan, st, now, timeout)
}

/// Die Bitmenge der eingestellten Fahrstraßen stimmt mit den Phasen überein.
fn active_set_matches_phases(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    let active = BitSet::new(st.active);
    for r in route_ids(plan) {
        let is_active = route(st, r)?.phase != RoutePhase::Idle;
        if is_active != active.contains(r.index().ok_or(IDX)?) {
            return Err(SafeStateReason::OrphanLock);
        }
    }
    Ok(())
}

/// Keine zwei feindlichen Fahrstraßen sind gleichzeitig eingestellt.
fn no_conflicting_routes(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    let active = BitSet::new(st.active);
    for a in &active {
        let ra = RouteId::from_index(a).ok_or(IDX)?;
        let view = plan.route(ra).ok_or(IDX)?;
        for b in active.iter().filter(|b| *b > a) {
            let rb = RouteId::from_index(b).ok_or(IDX)?;
            if view.conflicts_with(rb) {
                return Err(SafeStateReason::ConflictingRoutes(ra, rb));
            }
        }
    }
    Ok(())
}

/// Vergleicht einen Verschluss im Zustand mit dem erwarteten.
fn lock_matches(actual: Option<RouteId>, expected: Option<RouteId>) -> Res<()> {
    if actual == expected {
        return Ok(());
    }
    Err(expected.map_or(SafeStateReason::OrphanLock, SafeStateReason::MissingLock))
}

/// Setzt den erwarteten exklusiven Verschluss; zwei Halter sind ein Widerspruch.
fn claim(slot: &mut Option<RouteId>, r: RouteId) -> Res<()> {
    if slot.replace(r).is_some() {
        return Err(HOLD);
    }
    Ok(())
}

/// Jeder Abschnitt ist genau von den Fahrstraßen verschlossen, deren noch nicht
/// aufgelöster Fahrweg bzw. deren Durchrutschweg ihn enthält.
fn tvd_locks_match_routes(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    for (index, state) in st.tvds.iter().enumerate() {
        let t = TvdId::from_index(index).ok_or(IDX)?;
        let mut owner = None;
        let mut overlap = None;
        for (r, rs, view) in active_routes(plan, st) {
            if view.tvds().any(|x| x == t) && !released(&view, &rs, t) {
                claim(&mut owner, r)?;
            }
            if view.overlap_tvds().any(|x| x == t) {
                claim(&mut overlap, r)?;
            }
        }
        lock_matches(state.owner, owner)?;
        lock_matches(state.overlap_owner, overlap)?;
    }
    Ok(())
}

fn merge_position(locked: &mut Option<PointPosition>, want: PointPosition) -> Res<()> {
    if locked.is_some_and(|p| p != want) {
        return Err(HOLD);
    }
    *locked = Some(want);
    Ok(())
}

/// Jede Weiche ist genau von den Fahrstraßen verschlossen, die sie im Fahrweg,
/// Durchrutschweg oder Flankenschutz brauchen, und alle verlangen dieselbe Lage.
fn point_locks_match_routes(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    for (index, state) in st.points.iter().enumerate() {
        let p = PointId::from_index(index).ok_or(IDX)?;
        let mut owner = None;
        let mut holds: u8 = 0;
        let mut locked = None;
        for (r, rs, view) in active_routes(plan, st) {
            for (q, want) in view.points() {
                let held = plan.point_tvd(q).is_none_or(|t| !released(&view, &rs, t));
                if q == p && held {
                    claim(&mut owner, r)?;
                    merge_position(&mut locked, want)?;
                }
            }
            for (q, want) in view.overlap_points() {
                if q == p {
                    holds = holds.checked_add(1).ok_or(HOLD)?;
                    merge_position(&mut locked, want)?;
                }
            }
            for f in view.flank() {
                if let Protection::Point(q, want) = f.protection
                    && q == p
                    && !released(&view, &rs, f.protects)
                {
                    holds = holds.checked_add(1).ok_or(HOLD)?;
                    merge_position(&mut locked, want)?;
                }
            }
        }
        for b in locked_borders(plan, st) {
            for (q, want) in plan.border_approach_points(b) {
                if q == p {
                    holds = holds.checked_add(1).ok_or(HOLD)?;
                    merge_position(&mut locked, want)?;
                }
            }
        }
        lock_matches(state.owner, owner)?;
        if state.holds != holds || state.locked_pos != locked {
            return Err(HOLD);
        }
    }
    Ok(())
}

/// Grenzen (REQ-IXL-017 … REQ-IXL-020):
///
/// - Der Annäherungsabschnitt ist genau dann verschlossen, wenn die Erlaubnis nicht gehalten
///   wird, und die Zähler der Abschnitte stimmen damit überein.
/// - Eine eingestellte Fahrstraße über eine Grenze setzt die Erlaubnis voraus.
/// - Verschlossene Annäherungsabschnitte gehören nur Fahrstraßen ab dem Einfahrsignal.
fn borders_are_consistent(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    for b in border_ids(plan) {
        let state = border(st, b)?;
        if state.lock == state.holding || (state.holding && state.offering) {
            return Err(HOLD);
        }
    }
    for (index, state) in st.tvds.iter().enumerate() {
        let t = TvdId::from_index(index).ok_or(IDX)?;
        let mut count: u8 = 0;
        for b in locked_borders(plan, st) {
            if plan.border_approach_tvds(b).any(|x| x == t) {
                count = count.checked_add(1).ok_or(HOLD)?;
                let entry = plan.border_entry_signal(b);
                for owner in [state.owner, state.overlap_owner].into_iter().flatten() {
                    if plan.route(owner).map(|r| r.start()) != entry {
                        return Err(HOLD);
                    }
                }
            }
        }
        if state.border_locks != count {
            return Err(HOLD);
        }
    }
    for (_, _, view) in active_routes(plan, st) {
        if let RouteEnd::Border(b) = view.end()
            && !border(st, b)?.holding
        {
            return Err(SafeStateReason::MissingLock(view.id()));
        }
    }
    Ok(())
}

/// Die Haltezähler der Schutzsignale stimmen mit den Fahrstraßen überein.
fn signal_holds_match_routes(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    for (index, state) in st.signals.iter().enumerate() {
        let s = SignalId::from_index(index).ok_or(IDX)?;
        let mut holds: u8 = 0;
        for (_, rs, view) in active_routes(plan, st) {
            for f in view.flank() {
                if f.protection == Protection::Signal(s) && !released(&view, &rs, f.protects) {
                    holds = holds.checked_add(1).ok_or(HOLD)?;
                }
            }
        }
        if state.stop_holds != holds {
            return Err(HOLD);
        }
    }
    Ok(())
}

/// Die Haltezähler der Bahnübergänge stimmen mit den Fahrstraßen überein, und jeder
/// gebrauchte Bahnübergang ist zum Schließen angesteuert.
fn level_crossings_match_routes(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    for (index, state) in st.lxs.iter().enumerate() {
        let l = LxId::from_index(index).ok_or(IDX)?;
        let t = plan.lx_tvd(l).ok_or(IDX)?;
        let mut holds: u8 = 0;
        for (_, rs, view) in active_routes(plan, st) {
            if view.level_crossings().any(|x| x == l) && !released(&view, &rs, t) {
                holds = holds.checked_add(1).ok_or(HOLD)?;
            }
        }
        if state.holds != holds {
            return Err(HOLD);
        }
        if holds > 0 && !state.closed_commanded {
            return Err(SafeStateReason::UnsafeLevelCrossing(l));
        }
    }
    Ok(())
}

/// Weichen, Schutzsignale und Bahnübergänge einer Fahrstraße sind gesichert.
fn secured(st: &KernelState<'_>, view: &RouteView<'_>, now: Tick, timeout: Duration) -> Res<bool> {
    let mut ok = signal(st, view.start())?.stop_holds == 0;
    for (p, pos) in view.points().chain(view.overlap_points()) {
        ok &= effective_position(&point(st, p)?, now, timeout) == Some(pos);
    }
    for f in view.flank() {
        match f.protection {
            Protection::Point(p, pos) => {
                ok &= effective_position(&point(st, p)?, now, timeout) == Some(pos);
            }
            Protection::Signal(s) => {
                let state = signal(st, s)?;
                ok &=
                    state.commanded == Aspect::Stop && state.report != Some(SignalReport::Proceed);
            }
        }
    }
    for l in view.level_crossings() {
        let state = lx(st, l)?;
        ok &= state.closed_commanded && lx_closed(&state, now, timeout);
    }
    Ok(ok)
}

/// Das Ersatzsignal leuchtet nur für eine festgelegte (oder haltgefallene) Fahrstraße ab
/// diesem Signal, deren Weichen, Schutzsignale und Bahnübergänge gesichert sind.
fn substitute_only_when_justified(
    plan: &PlanView<'_>,
    st: &KernelState<'_>,
    now: Tick,
    timeout: Duration,
) -> Res<()> {
    for r in route_ids(plan) {
        let state = route(st, r)?;
        let view = plan.route(r).ok_or(IDX)?;
        if state.substitute_since.is_some()
            && !matches!(state.phase, RoutePhase::Locked | RoutePhase::Halted)
        {
            return Err(SafeStateReason::UnsafeSubstitute(view.start()));
        }
    }
    for (index, s) in st.signals.iter().enumerate() {
        if !s.substitute {
            continue;
        }
        let id = SignalId::from_index(index).ok_or(IDX)?;
        let mut justified = false;
        for (_, rs, view) in active_routes(plan, st) {
            if view.start() == id
                && rs.substitute_since.is_some()
                && matches!(rs.phase, RoutePhase::Locked | RoutePhase::Halted)
            {
                justified |= secured(st, &view, now, timeout)?;
            }
        }
        if !justified {
            return Err(SafeStateReason::UnsafeSubstitute(id));
        }
    }
    Ok(())
}

/// Ein Signal ist nur auf Fahrt gestellt, wenn eine Fahrstraße ab diesem Signal die
/// Fahrerlaubnis hat und alle Bedingungen **jetzt** erfüllt sind: Fahrweg und
/// Durchrutschweg frei und verschlossen, alle Weichen in Lage, Schutzsignale in Halt,
/// Bahnübergänge geschlossen.
fn proceed_only_when_safe(
    plan: &PlanView<'_>,
    st: &KernelState<'_>,
    now: Tick,
    timeout: Duration,
) -> Res<()> {
    for (index, s) in st.signals.iter().enumerate() {
        if s.commanded != Aspect::Proceed {
            continue;
        }
        let id = SignalId::from_index(index).ok_or(IDX)?;
        let mut justified = false;
        for (r, rs, view) in active_routes(plan, st) {
            if view.start() != id || rs.phase != RoutePhase::Proceed {
                continue;
            }
            let mut ok =
                s.report != Some(SignalReport::Defective) && secured(st, &view, now, timeout)?;
            for t in view.tvds() {
                let state = tvd(st, t)?;
                ok &= effective_occupancy(&state, now, timeout) == Occupancy::Clear;
                ok &= state.owner == Some(r);
            }
            for t in view.overlap_tvds() {
                let state = tvd(st, t)?;
                ok &= effective_occupancy(&state, now, timeout) == Occupancy::Clear;
                ok &= state.overlap_owner == Some(r);
            }
            if let RouteEnd::Border(b) = view.end() {
                let state = border(st, b)?;
                ok &= state.holding
                    && state.peer.is_some_and(|p| p.approach_clear && !p.holding)
                    && fresh(state.peer_at, now, timeout);
            }
            justified |= ok;
        }
        if !justified {
            return Err(SafeStateReason::UnsafeProceed(id));
        }
    }
    Ok(())
}
