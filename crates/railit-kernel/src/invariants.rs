//! Sicherheitsinvarianten, geprüft nach jedem Zyklus (REQ-IXL-008).
//!
//! Die Prüfungen sind bewusst unabhängig von der Logik in [`crate::kernel`] formuliert:
//! Sie beschreiben, was **nie** gelten darf, nicht wie der Kern es verhindert. Eine
//! Verletzung führt in den sicheren Zustand.

use railit_plan::PlanView;
use railit_types::bitset::BitSet;
use railit_types::{Duration, RouteId, SignalId, Tick};

use crate::io::{Aspect, Occupancy, RoutePhase, SafeStateReason, SignalReport};
use crate::kernel::{effective_occupancy, effective_position, point, route, signal, tvd};
use crate::state::KernelState;

type Res<T> = Result<T, SafeStateReason>;

const IDX: SafeStateReason = SafeStateReason::InternalIndex;

fn route_ids(plan: &PlanView<'_>) -> impl Iterator<Item = RouteId> {
    (0..plan.route_count()).filter_map(RouteId::from_index)
}

/// Prüft alle Invarianten.
pub(crate) fn check(
    plan: &PlanView<'_>,
    st: &KernelState<'_>,
    now: Tick,
    timeout: Duration,
) -> Res<()> {
    active_set_matches_phases(plan, st)?;
    no_orphan_locks(st)?;
    active_routes_hold_their_locks(plan, st)?;
    no_conflicting_routes(plan, st)?;
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

/// Kein Element ist von einer nicht eingestellten Fahrstraße verschlossen.
fn no_orphan_locks(st: &KernelState<'_>) -> Res<()> {
    let owners = st
        .tvds
        .iter()
        .map(|t| t.owner)
        .chain(st.points.iter().map(|p| p.owner));
    for owner in owners.flatten() {
        if route(st, owner)?.phase == RoutePhase::Idle {
            return Err(SafeStateReason::OrphanLock);
        }
    }
    Ok(())
}

/// Jede eingestellte Fahrstraße hält alle noch nicht aufgelösten Abschnitte und deren Weichen.
fn active_routes_hold_their_locks(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    for r in route_ids(plan) {
        let state = route(st, r)?;
        if state.phase == RoutePhase::Idle {
            continue;
        }
        let view = plan.route(r).ok_or(IDX)?;
        let released = usize::try_from(state.next_release).map_err(|_| IDX)?;
        let unreleased = || view.tvds().skip(released);
        if unreleased().any(|t| tvd(st, t).map_or(true, |s| s.owner != Some(r))) {
            return Err(SafeStateReason::MissingLock(r));
        }
        for (p, _) in view.points() {
            let in_unreleased = plan
                .point_tvd(p)
                .is_some_and(|pt| unreleased().any(|t| t == pt));
            if in_unreleased && point(st, p)?.owner != Some(r) {
                return Err(SafeStateReason::MissingLock(r));
            }
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

/// Ein Signal ist nur auf Fahrt gestellt, wenn eine Fahrstraße ab diesem Signal die
/// Fahrerlaubnis hat und alle Bedingungen **jetzt** erfüllt sind.
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
        for r in route_ids(plan) {
            let view = plan.route(r).ok_or(IDX)?;
            if view.start() != id || route(st, r)?.phase != RoutePhase::Proceed {
                continue;
            }
            let mut ok = signal(st, id)?.report != Some(SignalReport::Defective);
            for (p, pos) in view.points() {
                ok &= effective_position(&point(st, p)?, now, timeout) == Some(pos);
            }
            for t in view.tvds() {
                ok &= effective_occupancy(&tvd(st, t)?, now, timeout) == Occupancy::Clear;
                ok &= tvd(st, t)?.owner == Some(r);
            }
            justified |= ok;
        }
        if !justified {
            return Err(SafeStateReason::UnsafeProceed(id));
        }
    }
    Ok(())
}
