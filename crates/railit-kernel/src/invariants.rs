//! Sicherheitsinvarianten, geprüft nach jedem Zyklus (REQ-IXL-008).
//!
//! Die Prüfungen sind bewusst unabhängig von der Logik in [`crate::kernel`] formuliert:
//! Sie beschreiben, was **nie** gelten darf, nicht wie der Kern es verhindert. Insbesondere
//! werden alle Verschlüsse aus den Phasen und dem Auflösefortschritt der Fahrstraßen neu
//! berechnet und mit den Zählern im Zustand verglichen. Eine Verletzung führt in den
//! sicheren Zustand.
//!
//! # Laufzeit (REQ-IXL-023)
//!
//! Die Prüfung läuft in jedem Zyklus und ist **linear** in der Größe des Plans und der
//! eingestellten Fahrstraßen (Konzept, Abschnitt 15). Statt für jedes Element alle
//! Fahrstraßen zu durchsuchen, wird jede Aussage in zwei Richtungen geprüft:
//!
//! - **Erwartet ⇒ vorhanden**: Jede eingestellte Fahrstraße prüft ihre eigenen Verschlüsse.
//! - **Vorhanden ⇒ erwartet**: Exklusive Verschlüsse werden gezählt; stimmt die Anzahl der
//!   verschlossenen Elemente mit der Anzahl der geprüften Verschlüsse überein, gibt es keinen
//!   verwaisten. Geteilte Verschlüsse (Zähler) werden **durch Abziehen** geprüft: Jeder
//!   erwartete Halter zieht eins vom Zähler ab, danach müssen alle Zähler null sein, dann
//!   wird alles wieder hinzugezählt (siehe [`shared_locks_match_routes`]).
//!
//! Die Aussagen sind dieselben wie in der direkten, quadratischen Fassung; ein Test vergleicht
//! beide auf zufälligen und gezielt verfälschten Zuständen.

use railit_plan::{PlanView, Protection, RouteEnd, RouteView};
use railit_types::bitset::BitSet;
use railit_types::{BorderId, Duration, LxId, RouteId, SignalId, Tick, TvdId};

use crate::io::{Aspect, Occupancy, RoutePhase, SafeStateReason, SignalReport};
use crate::kernel::{
    border, effective_occupancy, effective_position, fresh, lx, lx_closed, point, route, signal,
    tvd,
};
use crate::state::{KernelState, RouteState};

#[cfg(test)]
mod reference;

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
///
/// Läuft über die Bitmenge der eingestellten Fahrstraßen statt über alle Fahrstraßen; dass
/// sie genau den Phasen entspricht, prüft [`routes_match_phases`] zuerst. Die Phase wird
/// trotzdem je Fahrstraße erneut geprüft.
fn active_routes<'a, 'p>(
    plan: &'a PlanView<'p>,
    active: &'a [u64],
    routes: &'a [RouteState],
) -> impl Iterator<Item = (RouteId, RouteState, RouteView<'p>)> + 'a {
    BitSet::new(active).into_iter().filter_map(move |i| {
        let r = RouteId::from_index(i)?;
        let state = *routes.get(i)?;
        if state.phase == RoutePhase::Idle {
            return None;
        }
        Some((r, state, plan.route(r)?))
    })
}

/// Wurde der Abschnitt `t` des Fahrwegs bereits aufgelöst?
fn released(view: &RouteView<'_>, state: &RouteState, t: TvdId) -> bool {
    view.tvds().take(released_count(state)).any(|x| x == t)
}

/// Anzahl der bereits aufgelösten Abschnitte am Anfang des Fahrwegs.
fn released_count(state: &RouteState) -> usize {
    usize::try_from(state.next_release).unwrap_or(usize::MAX)
}

/// Zählt einen Verschluss; ein Überlauf ist ein Widerspruch.
fn tally(count: &mut usize) -> Res<()> {
    *count = count.checked_add(1).ok_or(HOLD)?;
    Ok(())
}

/// Prüft alle Invarianten.
///
/// Der Zustand wird nur vorübergehend verändert (Zähler beim Abziehen) und ist danach
/// bitgleich wie zuvor, auch wenn eine Verletzung gefunden wird.
pub(crate) fn check(
    plan: &PlanView<'_>,
    st: &mut KernelState<'_>,
    now: Tick,
    timeout: Duration,
) -> Res<()> {
    routes_match_phases(plan, st)?;
    no_conflicting_routes(plan, st)?;
    let found = route_locks(plan, st, now, timeout)?;
    let census = census(plan, st)?;
    exclusive_locks_have_owners(&census, &found)?;
    shared_locks_match_routes(plan, st, &census)?;
    borders_are_consistent(plan, st)?;
    substitute_only_when_justified(plan, st, &census, &found, now, timeout)?;
    proceed_only_when_safe(plan, st, &census, &found, now, timeout)
}

/// Bestandsaufnahme des Zustands: ein Durchlauf je Elementart für alles, was die Richtung
/// „vorhanden ⇒ erwartet“ braucht.
#[derive(Default)]
struct Census {
    /// Verschlossene Abschnitte (Fahrweg, Durchrutschweg) und Weichen (Fahrweg).
    owned_path: usize,
    owned_overlap: usize,
    owned_points: usize,
    /// Summe aller Zähler geteilter Verschlüsse.
    holds: u64,
    /// Eine Weiche hat eine verschlossene Lage ohne Verschluss oder umgekehrt.
    stray_position: bool,
    /// Erster Bahnübergang mit Haltern, der nicht zum Schließen angesteuert ist.
    open_lx: Option<LxId>,
    /// Signale mit Fahrtbegriff bzw. eingeschaltetem Ersatzsignal.
    proceed: usize,
    substitute: usize,
}

fn census(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<Census> {
    let mut c = Census::default();
    let count = |n: &mut usize, yes: bool| *n = n.saturating_add(usize::from(yes));
    for p in st.points.iter() {
        count(&mut c.owned_points, p.owner.is_some());
        c.holds = c.holds.saturating_add(u64::from(p.holds));
        c.stray_position |= p.locked_pos.is_some() != (p.owner.is_some() || p.holds > 0);
    }
    for t in st.tvds.iter() {
        count(&mut c.owned_path, t.owner.is_some());
        count(&mut c.owned_overlap, t.overlap_owner.is_some());
        c.holds = c.holds.saturating_add(u64::from(t.border_locks));
    }
    for s in st.signals.iter() {
        count(&mut c.proceed, s.commanded == Aspect::Proceed);
        count(&mut c.substitute, s.substitute);
        c.holds = c.holds.saturating_add(u64::from(s.stop_holds));
    }
    for (index, l) in st.lxs.iter().enumerate() {
        let id = LxId::from_index(index).ok_or(IDX)?;
        plan.lx_tvd(id).ok_or(IDX)?;
        c.holds = c.holds.saturating_add(u64::from(l.holds));
        if l.holds > 0 && !l.closed_commanded && c.open_lx.is_none() {
            c.open_lx = Some(id);
        }
    }
    Ok(c)
}

/// Die Bitmenge der eingestellten Fahrstraßen stimmt mit den Phasen überein, und ein
/// Ersatzsignal gilt nur für festgelegte oder haltgefallene Fahrstraßen.
fn routes_match_phases(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    let active = BitSet::new(st.active);
    for r in route_ids(plan) {
        let state = route(st, r)?;
        if (state.phase != RoutePhase::Idle) != active.contains(r.index().ok_or(IDX)?) {
            return Err(SafeStateReason::OrphanLock);
        }
        if state.substitute_since.is_some()
            && !matches!(state.phase, RoutePhase::Locked | RoutePhase::Halted)
        {
            let view = plan.route(r).ok_or(IDX)?;
            return Err(SafeStateReason::UnsafeSubstitute(view.start()));
        }
    }
    Ok(())
}

/// Keine zwei feindlichen Fahrstraßen sind gleichzeitig eingestellt.
///
/// Je eingestellter Fahrstraße ein Abgleich ihrer Zeile der Feindlichkeitsmatrix mit der
/// Bitmenge (die Diagonale ist leer, die Matrix symmetrisch; beides prüft der Plan beim
/// Laden). Erst im Fehlerfall wird das Paar gesucht; die erste Fahrstraße mit einem
/// feindlichen Partner hat nur Partner mit größerer Nummer.
fn no_conflicting_routes(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    let active = BitSet::new(st.active);
    for a in &active {
        let ra = RouteId::from_index(a).ok_or(IDX)?;
        let view = plan.route(ra).ok_or(IDX)?;
        if !view.conflicts_with_any(&active) {
            continue;
        }
        for b in active.iter().filter(|b| *b > a) {
            let rb = RouteId::from_index(b).ok_or(IDX)?;
            if view.conflicts_with(rb) {
                return Err(SafeStateReason::ConflictingRoutes(ra, rb));
            }
        }
        return Err(SafeStateReason::ConflictingRoutes(ra, ra));
    }
    Ok(())
}

/// Was der Durchlauf über die eingestellten Fahrstraßen gezählt hat.
#[derive(Default)]
struct Found {
    /// Geprüfte exklusive Verschlüsse: Abschnitte im Fahrweg, im Durchrutschweg, Weichen.
    path: usize,
    overlap: usize,
    points: usize,
    /// Signale mit Fahrtbegriff bzw. Ersatzsignal, die eine Fahrstraße rechtfertigt.
    proceed: usize,
    substitute: usize,
}

/// Ein Durchlauf über die eingestellten Fahrstraßen (Richtung „erwartet ⇒ vorhanden“).
///
/// Exklusive Verschlüsse: Jeder Abschnitt und jede Weiche im noch nicht aufgelösten Fahrweg
/// gehört genau dieser Fahrstraße, jeder Abschnitt ihres Durchrutschwegs ebenso; jede Weiche,
/// die sie verschließt (Fahrweg, Durchrutschweg, Flanke), liegt in der verlangten Lage
/// verschlossen. Eine Fahrstraße über eine Grenze setzt die Erlaubnis voraus. Dazu zählt der
/// Durchlauf die Verschlüsse und die gerechtfertigten Fahrt- und Ersatzsignale für die
/// Rückrichtung.
fn route_locks(
    plan: &PlanView<'_>,
    st: &KernelState<'_>,
    now: Tick,
    timeout: Duration,
) -> Res<Found> {
    let mut found = Found::default();
    for (r, rs, view) in active_routes(plan, st.active, st.routes) {
        let missing = SafeStateReason::MissingLock(r);
        for t in view.tvds().skip(released_count(&rs)) {
            if tvd(st, t)?.owner != Some(r) {
                return Err(missing);
            }
            tally(&mut found.path)?;
        }
        for t in view.overlap_tvds() {
            if tvd(st, t)?.overlap_owner != Some(r) {
                return Err(missing);
            }
            tally(&mut found.overlap)?;
        }
        for (p, want) in view.points() {
            if plan.point_tvd(p).is_some_and(|t| released(&view, &rs, t)) {
                continue;
            }
            let state = point(st, p)?;
            if state.owner != Some(r) {
                return Err(missing);
            }
            if state.locked_pos != Some(want) {
                return Err(HOLD);
            }
            tally(&mut found.points)?;
        }
        for (p, want) in view.overlap_points() {
            if point(st, p)?.locked_pos != Some(want) {
                return Err(HOLD);
            }
        }
        for f in view.flank() {
            if let Protection::Point(p, want) = f.protection
                && !released(&view, &rs, f.protects)
                && point(st, p)?.locked_pos != Some(want)
            {
                return Err(HOLD);
            }
        }
        if let RouteEnd::Border(b) = view.end()
            && !border(st, b)?.holding
        {
            return Err(missing);
        }
        let start = signal(st, view.start())?;
        if start.commanded == Aspect::Proceed && justifies_proceed(st, r, &rs, &view, now, timeout)?
        {
            tally(&mut found.proceed)?;
        }
        if start.substitute && justifies_substitute(st, &rs, &view, now, timeout)? {
            tally(&mut found.substitute)?;
        }
    }
    Ok(found)
}

/// Richtung „vorhanden ⇒ erwartet“ für exklusive Verschlüsse: Jeder in [`route_locks`]
/// geprüfte Verschluss betrifft ein anderes Element (ein Element hat nur einen Eigentümer,
/// und die Listen einer Fahrstraße enthalten kein Element doppelt). Gibt es mehr
/// verschlossene Elemente als geprüfte Verschlüsse, ist einer verwaist. Doppelte Einträge im
/// Plan würden umgekehrt als Verletzung erkannt (sichere Seite).
fn exclusive_locks_have_owners(census: &Census, found: &Found) -> Res<()> {
    if census.owned_path != found.path
        || census.owned_overlap != found.overlap
        || census.owned_points != found.points
    {
        return Err(SafeStateReason::OrphanLock);
    }
    Ok(())
}

/// Ein Zähler im Zustand, auf den ein erwarteter geteilter Verschluss entfällt.
#[derive(Copy, Clone)]
enum Counter {
    /// Weiche im Durchrutschweg, als Flankenschutz oder im Annäherungsabschnitt einer Grenze.
    Point(railit_types::PointId),
    /// Schutzsignal einer Fahrstraße.
    Signal(SignalId),
    /// Bahnübergang im noch nicht aufgelösten Teil einer Fahrstraße.
    Lx(LxId),
    /// Annäherungsabschnitt einer Grenze, deren Erlaubnis abgegeben ist.
    Border(TvdId),
}

impl Counter {
    fn slot<'s>(self, st: &'s mut KernelState<'_>) -> Res<&'s mut u8> {
        let slot = match self {
            Self::Point(p) => p
                .index()
                .and_then(|i| st.points.get_mut(i))
                .map(|s| &mut s.holds),
            Self::Signal(s) => s
                .index()
                .and_then(|i| st.signals.get_mut(i))
                .map(|s| &mut s.stop_holds),
            Self::Lx(l) => l
                .index()
                .and_then(|i| st.lxs.get_mut(i))
                .map(|s| &mut s.holds),
            Self::Border(t) => t
                .index()
                .and_then(|i| st.tvds.get_mut(i))
                .map(|s| &mut s.border_locks),
        };
        slot.ok_or(IDX)
    }
}

/// Wendet `step` auf den Zähler jedes erwarteten geteilten Verschlusses an und gibt deren
/// Anzahl zurück.
///
/// Welche Verschlüsse erwartet werden, hängt nur vom Plan, den Fahrstraßenzuständen und den
/// Grenzverschlüssen ab, nie von den Zählern selbst. Zwei Läufe besuchen deshalb dieselben
/// Zähler in derselben Reihenfolge und brechen bei einem Indexfehler an derselben Stelle ab.
fn each_shared_lock(plan: &PlanView<'_>, st: &mut KernelState<'_>, step: fn(u8) -> u8) -> Res<u64> {
    let mut count: u64 = 0;
    let mut apply = |st: &mut KernelState<'_>, c: Counter| -> Res<()> {
        let slot = c.slot(st)?;
        *slot = step(*slot);
        count = count.checked_add(1).ok_or(HOLD)?;
        Ok(())
    };
    // Die Bitmenge wird wortweise kopiert, weil die Zähler im selben Zustand liegen.
    for w in 0..st.active.len() {
        let mut bits = st.active.get(w).copied().ok_or(IDX)?;
        while bits != 0 {
            let bit = usize::try_from(bits.trailing_zeros()).map_err(|_| IDX)?;
            bits &= bits.wrapping_sub(1);
            let index = w
                .checked_mul(64)
                .and_then(|i| i.checked_add(bit))
                .ok_or(IDX)?;
            let r = RouteId::from_index(index).ok_or(IDX)?;
            let rs = route(st, r)?;
            if rs.phase != RoutePhase::Idle {
                route_shared_locks(plan, st, r, &rs, &mut apply)?;
            }
        }
    }
    for b in border_ids(plan) {
        if !border(st, b)?.lock {
            continue;
        }
        for (p, _) in plan.border_approach_points(b) {
            apply(st, Counter::Point(p))?;
        }
        for t in plan.border_approach_tvds(b) {
            apply(st, Counter::Border(t))?;
        }
    }
    Ok(count)
}

/// Die geteilten Verschlüsse einer eingestellten Fahrstraße: Weichen im Durchrutschweg,
/// Flankenschutz und Bahnübergänge, soweit ihr Abschnitt noch nicht aufgelöst ist.
fn route_shared_locks(
    plan: &PlanView<'_>,
    st: &mut KernelState<'_>,
    r: RouteId,
    rs: &RouteState,
    apply: &mut impl FnMut(&mut KernelState<'_>, Counter) -> Res<()>,
) -> Res<()> {
    let view = plan.route(r).ok_or(IDX)?;
    for (p, _) in view.overlap_points() {
        apply(st, Counter::Point(p))?;
    }
    for f in view.flank() {
        if released(&view, rs, f.protects) {
            continue;
        }
        match f.protection {
            Protection::Point(p, _) => apply(st, Counter::Point(p))?,
            Protection::Signal(s) => apply(st, Counter::Signal(s))?,
        }
    }
    for l in view.level_crossings() {
        if !released(&view, rs, plan.lx_tvd(l).ok_or(IDX)?) {
            apply(st, Counter::Lx(l))?;
        }
    }
    Ok(())
}

/// Sind alle Zähler geteilter Verschlüsse null?
fn all_counters_zero(st: &KernelState<'_>) -> bool {
    st.points.iter().all(|s| s.holds == 0)
        && st.signals.iter().all(|s| s.stop_holds == 0)
        && st.lxs.iter().all(|s| s.holds == 0)
        && st.tvds.iter().all(|s| s.border_locks == 0)
}

/// Geteilte Verschlüsse: Die Haltezähler der Weichen (Durchrutschweg, Flankenschutz,
/// Annäherungsabschnitte), der Schutzsignale, der Bahnübergänge und der
/// Annäherungsabschnitte an Grenzen stimmen genau mit den erwarteten Haltern überein; jede
/// verschlossene Weiche hat eine verschlossene Lage, jede andere keine; jeder gebrauchte
/// Bahnübergang ist zum Schließen angesteuert.
///
/// Prüfung durch Abziehen: Jeder erwartete Halter zieht eins von seinem Zähler ab (modulo
/// 256), danach müssen alle Zähler null sein; dann wird alles wieder hinzugezählt. Weil
/// Addition und Subtraktion modulo 256 umkehrbar sind, ist der Zustand danach bitgleich wie
/// zuvor. „Alle null“ heißt, dass jeder Zähler modulo 256 mit seiner Erwartung übereinstimmt;
/// kein Zähler kann seine Erwartung um 256 oder mehr übersteigen, und stimmen zusätzlich die
/// Summen überein, kann ihn auch keiner um 256 verfehlen – die Zähler sind also exakt.
fn shared_locks_match_routes(
    plan: &PlanView<'_>,
    st: &mut KernelState<'_>,
    census: &Census,
) -> Res<()> {
    let expected = each_shared_lock(plan, st, |h| h.wrapping_sub(1));
    let zero = all_counters_zero(st);
    let restored = each_shared_lock(plan, st, |h| h.wrapping_add(1));
    restored?;
    if expected? != census.holds || !zero {
        return Err(HOLD);
    }
    // Erst mit exakten Zählern aussagekräftig.
    if census.stray_position {
        return Err(HOLD);
    }
    census
        .open_lx
        .map_or(Ok(()), |l| Err(SafeStateReason::UnsafeLevelCrossing(l)))
}

/// Grenzen (REQ-IXL-017 … REQ-IXL-020):
///
/// - Der Annäherungsabschnitt ist genau dann verschlossen, wenn die Erlaubnis nicht gehalten
///   wird; die Zähler der Abschnitte prüft [`shared_locks_match_routes`].
/// - Eine eingestellte Fahrstraße über eine Grenze setzt die Erlaubnis voraus (geprüft in
///   [`route_locks`]).
/// - Verschlossene Annäherungsabschnitte gehören nur Fahrstraßen ab dem Einfahrsignal, ihre
///   Weichen liegen in der verlangten Lage verschlossen.
fn borders_are_consistent(plan: &PlanView<'_>, st: &KernelState<'_>) -> Res<()> {
    for b in border_ids(plan) {
        let state = border(st, b)?;
        if state.lock == state.holding || (state.holding && state.offering) {
            return Err(HOLD);
        }
    }
    for b in locked_borders(plan, st) {
        let entry = plan.border_entry_signal(b);
        for t in plan.border_approach_tvds(b) {
            let state = tvd(st, t)?;
            for owner in [state.owner, state.overlap_owner].into_iter().flatten() {
                if plan.route(owner).map(|r| r.start()) != entry {
                    return Err(HOLD);
                }
            }
        }
        for (p, want) in plan.border_approach_points(b) {
            if point(st, p)?.locked_pos != Some(want) {
                return Err(HOLD);
            }
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

/// Rechtfertigt die eingestellte Fahrstraße das Ersatzsignal an ihrem Startsignal?
fn justifies_substitute(
    st: &KernelState<'_>,
    rs: &RouteState,
    view: &RouteView<'_>,
    now: Tick,
    timeout: Duration,
) -> Res<bool> {
    Ok(rs.substitute_since.is_some()
        && matches!(rs.phase, RoutePhase::Locked | RoutePhase::Halted)
        && secured(st, view, now, timeout)?)
}

/// Rechtfertigt die eingestellte Fahrstraße den Fahrtbegriff an ihrem Startsignal?
fn justifies_proceed(
    st: &KernelState<'_>,
    r: RouteId,
    rs: &RouteState,
    view: &RouteView<'_>,
    now: Tick,
    timeout: Duration,
) -> Res<bool> {
    if rs.phase != RoutePhase::Proceed {
        return Ok(false);
    }
    let start = signal(st, view.start())?;
    let mut ok = start.report != Some(SignalReport::Defective) && secured(st, view, now, timeout)?;
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
            && state
                .peer
                .is_some_and(|p| p.approach_clear && p.exclusive && !p.holding)
            && fresh(state.peer_at, now, timeout);
    }
    Ok(ok)
}

/// Ein Signal mit Fahrtbegriff oder Ersatzsignal, das keine eingestellte Fahrstraße
/// rechtfertigt (Fehlerpfad: sucht das Signal für die Meldung).
fn unjustified(
    plan: &PlanView<'_>,
    st: &KernelState<'_>,
    shown: impl Fn(&crate::state::SignalState) -> bool,
    justifies: impl Fn(RouteId, &RouteState, &RouteView<'_>) -> Res<bool>,
) -> Res<Option<SignalId>> {
    for (index, s) in st.signals.iter().enumerate() {
        if !shown(s) {
            continue;
        }
        let id = SignalId::from_index(index).ok_or(IDX)?;
        let mut found = false;
        for (r, rs, view) in active_routes(plan, st.active, st.routes) {
            if view.start() == id {
                found |= justifies(r, &rs, &view)?;
            }
        }
        if !found {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Das Ersatzsignal leuchtet nur für eine festgelegte (oder haltgefallene) Fahrstraße ab
/// diesem Signal, deren Weichen, Schutzsignale und Bahnübergänge gesichert sind.
///
/// Linear durch Zählen: Fahrstraßen mit gleichem Startsignal sind feindlich (der Plan prüft
/// das beim Laden), also rechtfertigt jede eingestellte Fahrstraße höchstens ein eigenes
/// Signal. Sind so viele Signale gerechtfertigt ([`route_locks`]), wie Ersatzsignale
/// leuchten, ist jedes gerechtfertigt.
fn substitute_only_when_justified(
    plan: &PlanView<'_>,
    st: &KernelState<'_>,
    census: &Census,
    found: &Found,
    now: Tick,
    timeout: Duration,
) -> Res<()> {
    if census.substitute == found.substitute {
        return Ok(());
    }
    let id = unjustified(
        plan,
        st,
        |s| s.substitute,
        |_, rs, view| justifies_substitute(st, rs, view, now, timeout),
    )?;
    Err(id.map_or(HOLD, SafeStateReason::UnsafeSubstitute))
}

/// Ein Signal ist nur auf Fahrt gestellt, wenn eine Fahrstraße ab diesem Signal die
/// Fahrerlaubnis hat und alle Bedingungen **jetzt** erfüllt sind: Fahrweg und
/// Durchrutschweg frei und verschlossen, alle Weichen in Lage, Schutzsignale in Halt,
/// Bahnübergänge geschlossen.
///
/// Linear durch Zählen wie bei [`substitute_only_when_justified`].
fn proceed_only_when_safe(
    plan: &PlanView<'_>,
    st: &KernelState<'_>,
    census: &Census,
    found: &Found,
    now: Tick,
    timeout: Duration,
) -> Res<()> {
    if census.proceed == found.proceed {
        return Ok(());
    }
    let id = unjustified(
        plan,
        st,
        |s| s.commanded == Aspect::Proceed,
        |r, rs, view| justifies_proceed(st, r, rs, view, now, timeout),
    )?;
    Err(id.map_or(HOLD, SafeStateReason::UnsafeProceed))
}

#[cfg(all(test, feature = "alloc"))]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]
mod tests {
    //! Die lineare Prüfung meldet auf zufälligen Betriebszuständen und deren gezielten
    //! Verfälschungen genau dann eine Verletzung, wenn die direkte Referenzfassung es tut,
    //! und lässt den Zustand unverändert.

    extern crate alloc;
    extern crate std;

    use alloc::vec::Vec;

    use railit_plan::PlanView;
    use railit_rules::{GenericRules, RuleSet};
    use railit_types::{PointId, PointPosition, RouteId, Tick};

    use crate::io::{
        Aspect, Command, Input, LxReport, Occupancy, PointReport, Request, RequestId, RoutePhase,
        SignalReport,
    };
    use crate::{Interlocking, OwnedState, VecOutputs};

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap()).unwrap()
        }

        fn chance(&mut self, percent: u64) -> bool {
            self.next() % 100 < percent
        }
    }

    fn leak(image: Vec<u8>) -> PlanView<'static> {
        PlanView::parse(alloc::boxed::Box::leak(image.into_boxed_slice())).unwrap()
    }

    fn plans() -> Vec<PlanView<'static>> {
        let parse = |src: &str| railit_import::toml_format::parse(src).unwrap().topology;
        let mut out = Vec::new();
        let town = parse(include_str!("../../../examples/beispielstadt.toml"));
        out.push(leak(railit_planning::compile(&town, None).unwrap().image));
        let corridor = parse(&railit_import::generate::corridor(4, 2));
        out.push(leak(
            railit_planning::compile(&corridor, None).unwrap().image,
        ));
        let nordbahn = parse(include_str!("../../../examples/nordbahn.toml"));
        let cfg = railit_planning::PlanningConfig::default();
        for area in railit_planning::compile_areas(&nordbahn, None, &cfg).unwrap() {
            out.push(leak(area.compiled.image));
        }
        out
    }

    /// Verfälscht ein zufälliges Feld des Zustands.
    fn corrupt(st: &mut OwnedState, plan: &PlanView<'_>, rng: &mut Rng) {
        let v = st.view();
        let any_route = |rng: &mut Rng| {
            if rng.chance(30) {
                None
            } else {
                RouteId::from_index(rng.below(plan.route_count()))
            }
        };
        let delta = |rng: &mut Rng, x: u8| {
            if rng.chance(50) {
                x.wrapping_add(1)
            } else {
                x.wrapping_sub(1)
            }
        };
        match rng.below(14) {
            0 if !v.points.is_empty() => {
                let i = rng.below(v.points.len());
                v.points[i].holds = delta(rng, v.points[i].holds);
            }
            1 if !v.points.is_empty() => {
                let i = rng.below(v.points.len());
                v.points[i].owner = any_route(rng);
            }
            2 if !v.points.is_empty() => {
                let i = rng.below(v.points.len());
                v.points[i].locked_pos = match rng.below(3) {
                    0 => None,
                    1 => Some(PointPosition::Left),
                    _ => Some(PointPosition::Right),
                };
            }
            3 => {
                let i = rng.below(v.tvds.len());
                v.tvds[i].owner = any_route(rng);
            }
            4 => {
                let i = rng.below(v.tvds.len());
                v.tvds[i].overlap_owner = any_route(rng);
            }
            5 => {
                let i = rng.below(v.tvds.len());
                v.tvds[i].border_locks = delta(rng, v.tvds[i].border_locks);
            }
            6 => {
                let i = rng.below(v.signals.len());
                v.signals[i].stop_holds = delta(rng, v.signals[i].stop_holds);
            }
            7 => {
                let i = rng.below(v.signals.len());
                v.signals[i].commanded = match v.signals[i].commanded {
                    Aspect::Stop => Aspect::Proceed,
                    Aspect::Proceed => Aspect::Stop,
                };
            }
            8 => {
                let i = rng.below(v.signals.len());
                v.signals[i].substitute = !v.signals[i].substitute;
            }
            9 if !v.lxs.is_empty() => {
                let i = rng.below(v.lxs.len());
                v.lxs[i].holds = delta(rng, v.lxs[i].holds);
            }
            10 if !v.lxs.is_empty() => {
                let i = rng.below(v.lxs.len());
                v.lxs[i].closed_commanded = !v.lxs[i].closed_commanded;
            }
            11 => {
                let i = rng.below(v.routes.len());
                let n = v.routes[i].next_release;
                v.routes[i].next_release = if rng.chance(50) {
                    n.wrapping_add(1)
                } else {
                    n.saturating_sub(1)
                };
            }
            12 => {
                let i = rng.below(v.routes.len());
                v.routes[i].phase = match rng.below(4) {
                    0 => RoutePhase::Locked,
                    1 => RoutePhase::Proceed,
                    2 => RoutePhase::Occupied,
                    _ => RoutePhase::Idle,
                };
            }
            _ => {
                if !v.borders.is_empty() {
                    let i = rng.below(v.borders.len());
                    v.borders[i].lock = !v.borders[i].lock;
                }
            }
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn linear_check_agrees_with_reference() {
        let rules = GenericRules::default();
        let timeout = rules.report_timeout();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut agreed_ok, mut agreed_err) = (0u32, 0u32);
        let (mut clean, mut clean_ok, mut releasing, mut proceeding) = (0u32, 0u32, 0u32, 0u32);
        for plan in plans() {
            let kernel = Interlocking::new(plan, GenericRules::default());
            let mut state = OwnedState::new(&plan);
            let mut points = alloc::vec![PointPosition::Right; plan.point_count()];
            let mut signals = alloc::vec![Aspect::Stop; plan.signal_count()];
            let mut lxs = alloc::vec![false; plan.lx_count()];
            // Züge: Fahrstraße und Index des besetzten Abschnitts im Fahrweg.
            let mut trains: Vec<(RouteId, usize)> = Vec::new();
            let mut out = VecOutputs::default();
            let mut next_id = 0;
            for cycle in 0..600u64 {
                let now = Tick::from_millis(cycle * 100);
                let mut occupied = alloc::vec![false; plan.tvd_count()];
                trains.retain_mut(|(r, i)| {
                    *i += 1;
                    let view = plan.route(*r).unwrap();
                    match view.tvds().nth(*i - 1) {
                        Some(t) => {
                            occupied[t.index().unwrap()] = true;
                            true
                        }
                        None => false,
                    }
                });
                let mut inputs = Vec::new();
                for (i, p) in points.iter().enumerate() {
                    let point = PointId::from_index(i).unwrap();
                    inputs.push(Input::Point {
                        point,
                        report: PointReport::from(*p),
                    });
                }
                for (i, s) in signals.iter().enumerate() {
                    let report = match s {
                        Aspect::Stop => SignalReport::Stop,
                        Aspect::Proceed => SignalReport::Proceed,
                    };
                    let signal = railit_types::SignalId::from_index(i).unwrap();
                    inputs.push(Input::Signal { signal, report });
                }
                for (i, o) in occupied.iter().enumerate() {
                    let occupancy = if *o {
                        Occupancy::Occupied
                    } else {
                        Occupancy::Clear
                    };
                    let tvd = railit_types::TvdId::from_index(i).unwrap();
                    inputs.push(Input::Tvd { tvd, occupancy });
                }
                for (i, closed) in lxs.iter().enumerate() {
                    let report = if *closed {
                        LxReport::Closed
                    } else {
                        LxReport::Open
                    };
                    let lx = railit_types::LxId::from_index(i).unwrap();
                    inputs.push(Input::LevelCrossing { lx, report });
                }
                let request = match rng.below(4) {
                    0 | 1 => Some(Request::SetRoute(
                        RouteId::from_index(rng.below(plan.route_count())).unwrap(),
                    )),
                    2 => Some(Request::CancelRoute(
                        RouteId::from_index(rng.below(plan.route_count())).unwrap(),
                    )),
                    _ => None,
                };
                if let Some(request) = request {
                    next_id += 1;
                    inputs.push(Input::Request {
                        id: RequestId(next_id),
                        request,
                    });
                }
                out.clear();
                let _ = kernel.step(&mut state.view(), now, &inputs, &mut out);
                for c in &out.commands {
                    match *c {
                        Command::MovePoint { point, position } => {
                            points[point.index().unwrap()] = position;
                        }
                        Command::SetSignal { signal, aspect } => {
                            signals[signal.index().unwrap()] = aspect;
                        }
                        Command::SetLevelCrossing { lx, close } => {
                            lxs[lx.index().unwrap()] = close;
                        }
                        _ => {}
                    }
                }
                // Züge fahren auf Fahrstraßen mit Fahrtbegriff los.
                for (i, rs) in state.routes().iter().enumerate() {
                    let r = RouteId::from_index(i).unwrap();
                    if rs.phase() == RoutePhase::Proceed
                        && rng.chance(20)
                        && !trains.iter().any(|(t, _)| *t == r)
                    {
                        trains.push((r, 0));
                    }
                }

                clean += 1;
                if state.view().meta.safe_state.is_none() {
                    clean_ok += 1;
                }
                if state.routes().iter().any(|r| r.next_release > 0) {
                    releasing += 1;
                }
                if state
                    .signals()
                    .iter()
                    .any(|s| s.commanded == Aspect::Proceed)
                {
                    proceeding += 1;
                }

                // Unverfälscht und mehrfach verfälscht: beide Fassungen stimmen überein.
                for round in 0..6 {
                    let mut probe = state.clone();
                    for _ in 0..round.min(2) {
                        corrupt(&mut probe, &plan, &mut rng);
                    }
                    let before = probe.clone();
                    let expected =
                        super::reference::check(&plan, &probe.clone().view(), now, timeout);
                    let actual = super::check(&plan, &mut probe.view(), now, timeout);
                    assert_eq!(probe, before, "Prüfung hat den Zustand verändert");
                    assert_eq!(
                        actual.is_ok(),
                        expected.is_ok(),
                        "Zyklus {cycle}, Runde {round}: linear {actual:?}, Referenz {expected:?}"
                    );
                    if actual.is_ok() {
                        agreed_ok += 1;
                    } else {
                        agreed_err += 1;
                    }
                }
            }
        }
        std::println!(
            "übereinstimmend: {agreed_ok} gültig, {agreed_err} verletzt; {clean_ok}/{clean} \
             Zyklen ohne sicheren Zustand, {releasing} mit Teilauflösung, {proceeding} mit Fahrt"
        );
        assert!(agreed_ok > 1000 && agreed_err > 1000);
        assert!(clean_ok * 10 > clean * 9 && releasing > 100 && proceeding > 100);
    }
}
