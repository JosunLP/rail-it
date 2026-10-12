//! Erlaubniswechsel an Bereichsgrenzen (Streckenblock, Konzept Abschnitt 10.4).
//!
//! An jeder Grenze darf nur **eine** Seite Züge hinüberschicken: die Seite, die die
//! **Erlaubnis** hält. Die andere Seite hält ihren **Annäherungsabschnitt** (von der Grenze
//! bis zum Einfahrsignal samt dessen Durchrutschweg) verschlossen und meldet, ob er frei ist.
//!
//! Protokoll (beide Kerne senden in jedem Zyklus eine [`BorderMessage`]):
//!
//! - **Abgeben:** Der Halter gibt die Erlaubnis ab, wenn der Nachbar sie anfordert, keine
//!   Fahrstraße über die Grenze eingestellt ist, beide Annäherungsabschnitte frei sind und er
//!   sie nicht selbst angefordert hat. Er erhöht die laufende Nummer, verschließt seinen
//!   Annäherungsabschnitt **im selben Zyklus** und bietet die Erlaubnis mit der neuen Nummer an.
//! - **Übernehmen:** Ein Kern übernimmt die Erlaubnis nur aus einem Angebot mit einer
//!   größeren Nummer als seiner eigenen. Alte, noch unterwegs befindliche Angebote werden so
//!   nie ein zweites Mal angenommen.
//! - **Quittung:** Meldet der Nachbar, die angebotene Erlaubnis zu halten, endet das Angebot.
//! - **Anlauf und Hilfshandlung:** Ohne Angebot übernimmt ein Kern die Erlaubnis nur, wenn er
//!   beim Anlauf dafür bestimmt ist oder die Hilfshandlung „Erlaubnis zurückholen“ bestätigt
//!   wurde, und erst nachdem der Nachbar die Einschwingzeit lang ununterbrochen gemeldet hat,
//!   sie nicht zu halten. Ein Angebot, das vor einem Neustart verschickt wurde, wäre in dieser
//!   Zeit angekommen und angenommen worden.
//! - **Neustart:** Nach dem (Neu-)Start nimmt ein Kern erst nach der Einschwingzeit Angebote
//!   an. Seine laufende Nummer beginnt wieder bei 0; ein altes Angebot des Nachbarn, das noch
//!   unterwegs ist, könnte sonst angenommen werden, während der Nachbar ein Angebot aus der
//!   Zeit vor dem Neustart übernimmt.
//!
//! - **Überdeckte Annäherungsabschnitte:** Hat ein Bereich mehrere Grenzen, deren
//!   Annäherungsabschnitte sich überdecken (eingleisige Strecke zwischen zwei Bahnhöfen), gibt
//!   er eine Erlaubnis nur ab, solange kein anderer Annäherungsabschnitt den eigenen
//!   verschließt. Ist ein Abschnitt doppelt verschlossen (etwa nach dem Anlauf), meldet er ihn
//!   keinem Nachbarn als ausschließlich verschlossen; eine Ausfahrt in ihn zeigt keinen
//!   Fahrtbegriff (REQ-IXL-021).
//!
//! Damit halten nie beide Seiten zugleich die Erlaubnis (REQ-IXL-017), sofern die Laufzeit
//! einer Meldung kleiner als die Einschwingzeit ist, und nie schicken zwei Nachbarn Züge in
//! denselben Abschnitt.

use railit_plan::RouteEnd;
use railit_rules::RuleSet;
use railit_types::{BorderId, SignalId, Tick, TvdId};

use super::{
    HOLD, IDX, Interlocking, Res, effective_occupancy, fresh, inc, point, point_mut, route, tvd,
    tvd_mut,
};
use crate::io::{Alarm, BorderMessage, Event, Occupancy, Outputs, Rejection, RoutePhase};
use crate::state::{BorderState, KernelState};

pub(crate) fn border(st: &KernelState<'_>, id: BorderId) -> Res<BorderState> {
    id.index()
        .and_then(|i| st.borders.get(i))
        .copied()
        .ok_or(IDX)
}

fn border_mut<'a>(st: &'a mut KernelState<'_>, id: BorderId) -> Res<&'a mut BorderState> {
    id.index().and_then(|i| st.borders.get_mut(i)).ok_or(IDX)
}

impl<R: RuleSet> Interlocking<'_, R> {
    pub(super) fn border_ids(&self) -> impl Iterator<Item = BorderId> + use<R> {
        (0..self.plan.border_count()).filter_map(BorderId::from_index)
    }

    /// Aktuelle Meldung des Nachbarn, falls sie nicht veraltet ist.
    pub(super) fn peer(&self, state: &BorderState, now: Tick) -> Option<BorderMessage> {
        state
            .peer
            .filter(|_| fresh(state.peer_at, now, self.rules.report_timeout()))
    }

    /// Übernimmt eine Meldung des Nachbarn. `false`, wenn die Grenze unbekannt ist.
    pub(super) fn record_border(
        st: &mut KernelState<'_>,
        now: Tick,
        id: BorderId,
        message: BorderMessage,
    ) -> bool {
        match border_mut(st, id) {
            Ok(b) => {
                b.peer = Some(message);
                b.peer_at = now;
                true
            }
            Err(_) => false,
        }
    }

    /// Anforderung der Erlaubnis setzen oder zurückziehen.
    pub(super) fn request_permission(
        &self,
        st: &mut KernelState<'_>,
        id: BorderId,
        want: bool,
    ) -> Res<Result<(), Rejection>> {
        if id.index().is_none_or(|i| i >= self.plan.border_count()) {
            return Ok(Err(Rejection::UnknownElement));
        }
        let b = border_mut(st, id)?;
        b.want = want && !b.holding;
        Ok(Ok(()))
    }

    /// Hilfshandlung „Erlaubnis zurückholen“ ist möglich, wenn dieser Kern sie nicht hält und
    /// der Nachbar aktuell meldet, sie ebenfalls nicht zu halten.
    pub(super) fn take_back_applicable(
        &self,
        st: &KernelState<'_>,
        id: BorderId,
        now: Tick,
    ) -> Res<Option<bool>> {
        if id.index().is_none_or(|i| i >= self.plan.border_count()) {
            return Ok(None);
        }
        let b = border(st, id)?;
        Ok(Some(
            !b.holding && self.peer(&b, now).is_some_and(|p| !p.holding),
        ))
    }

    pub(super) fn take_back(st: &mut KernelState<'_>, id: BorderId) -> Res<()> {
        border_mut(st, id)?.take_back = true;
        Ok(())
    }

    /// Ein Zyklus des Erlaubniswechsels für alle Grenzen (REQ-IXL-017, REQ-IXL-018).
    pub(super) fn step_borders<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        out: &mut O,
    ) -> Res<()> {
        let settle = self.rules.border_settle_time();
        for id in self.border_ids() {
            let state = border(st, id)?;
            let peer = self.peer(&state, now);

            // Verbindung überwachen; während des Anlaufs kann noch keine Meldung da sein.
            let starting = now.as_millis() <= self.rules.report_timeout().as_millis();
            let lost = peer.is_none() && !starting;
            if lost && !state.lost_alarmed {
                out.event(Event::Alarm(Alarm::BorderLost(id)));
            }
            let conflict = state.holding && peer.is_some_and(|p| p.holding);
            if conflict && !state.conflict_alarmed {
                out.event(Event::Alarm(Alarm::PermissionConflict(id)));
            }
            {
                let b = border_mut(st, id)?;
                b.lost_alarmed = lost;
                b.conflict_alarmed = conflict;
                b.peer_free_since = match peer {
                    Some(p) if !p.holding => Some(b.peer_free_since.unwrap_or(now)),
                    _ => None,
                };
            }

            // Quittung: Der Nachbar hält die angebotene Erlaubnis.
            if state.offering && peer.is_some_and(|p| p.holding && p.seq >= state.seq) {
                border_mut(st, id)?.offering = false;
            }

            let state = border(st, id)?;
            if state.holding {
                if self.may_give(st, id, now)? {
                    let b = border_mut(st, id)?;
                    b.holding = false;
                    b.offering = true;
                    b.seq = b.seq.checked_add(1).ok_or(HOLD)?;
                    out.event(Event::Permission {
                        border: id,
                        holding: false,
                    });
                }
                continue;
            }

            // Übernehmen aus einem Angebot mit größerer Nummer, nicht unmittelbar nach dem Start.
            let booted = now.saturating_since(st.meta.started_at) >= settle;
            if let Some(p) = peer
                && booted
                && p.offer
                && p.seq > state.seq
            {
                Self::become_holder(st, id, p.seq, out)?;
                continue;
            }

            // Übernehmen beim Anlauf oder nach der Hilfshandlung, nach der Einschwingzeit.
            let entitled = state.take_back || (self.plan.border_initial(id) && !state.offering);
            let settled = state
                .peer_free_since
                .is_some_and(|since| now.saturating_since(since) >= settle);
            if entitled && settled {
                let peer_seq = peer.map_or(0, |p| p.seq);
                let seq = state.seq.max(peer_seq).checked_add(1).ok_or(HOLD)?;
                Self::become_holder(st, id, seq, out)?;
            }
        }
        Ok(())
    }

    fn become_holder<O: Outputs>(
        st: &mut KernelState<'_>,
        id: BorderId,
        seq: u32,
        out: &mut O,
    ) -> Res<()> {
        let b = border_mut(st, id)?;
        b.holding = true;
        b.offering = false;
        b.want = false;
        b.take_back = false;
        b.seq = seq;
        out.event(Event::Permission {
            border: id,
            holding: true,
        });
        Ok(())
    }

    /// Darf der Halter die Erlaubnis jetzt abgeben?
    fn may_give(&self, st: &KernelState<'_>, id: BorderId, now: Tick) -> Res<bool> {
        let state = border(st, id)?;
        let Some(peer) = self.peer(&state, now) else {
            return Ok(false);
        };
        if !peer.request || state.want || peer.holding || !peer.approach_clear {
            return Ok(false);
        }
        // Keine Fahrstraße über die Grenze eingestellt.
        for r in self.plan.routes() {
            if r.end() == RouteEnd::Border(id) && route(st, r.id())?.phase != RoutePhase::Idle {
                return Ok(false);
            }
        }
        // Eigener Annäherungsabschnitt frei und nicht anderweitig verschlossen.
        let entry = self.plan.border_entry_signal(id).ok_or(IDX)?;
        let timeout = self.rules.report_timeout();
        for t in self.plan.border_approach_tvds(id) {
            let ts = tvd(st, t)?;
            // Verschlösse die Abgabe den Abschnitt ein zweites Mal, dürften zwei Nachbarn
            // Züge hineinschicken (REQ-IXL-021). Die eigene Sperre ist hier offen.
            if ts.border_locks != 0 {
                return Ok(false);
            }
            let continuation = ts
                .owner
                .is_none_or(|o| self.plan.route(o).is_some_and(|r| r.start() == entry));
            if !continuation
                || ts.overlap_owner.is_some()
                || effective_occupancy(&ts, now, timeout) != Occupancy::Clear
            {
                return Ok(false);
            }
        }
        for (p, pos) in self.plan.border_approach_points(id) {
            if point(st, p)?.locked_pos.is_some_and(|lp| lp != pos) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Verschließt oder öffnet die Annäherungsabschnitte so, dass genau die Grenzen ohne
    /// Erlaubnis verschlossen sind (REQ-IXL-019).
    pub(super) fn sync_border_locks<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        now: Tick,
        out: &mut O,
    ) -> Res<()> {
        for id in self.border_ids() {
            let state = border(st, id)?;
            let want_lock = !state.holding;
            if state.lock == want_lock {
                continue;
            }
            if want_lock {
                for t in self.plan.border_approach_tvds(id) {
                    let ts = tvd_mut(st, t)?;
                    ts.border_locks = inc(ts.border_locks)?;
                }
                for (p, pos) in self.plan.border_approach_points(id) {
                    let ps = point_mut(st, p)?;
                    if ps.locked_pos.is_some_and(|lp| lp != pos) {
                        return Err(HOLD);
                    }
                    ps.holds = inc(ps.holds)?;
                    ps.locked_pos = Some(pos);
                }
                self.set_approach_points(st, id, now, out)?;
            } else {
                for t in self.plan.border_approach_tvds(id) {
                    let ts = tvd_mut(st, t)?;
                    ts.border_locks = ts.border_locks.checked_sub(1).ok_or(HOLD)?;
                }
                for (p, _) in self.plan.border_approach_points(id) {
                    Self::unhold_point(st, p)?;
                }
            }
            border_mut(st, id)?.lock = want_lock;
        }
        Ok(())
    }

    /// Stellt Weichen des Annäherungsabschnitts in ihre Lage, sofern sie frei sind.
    fn set_approach_points<O: Outputs>(
        &self,
        st: &mut KernelState<'_>,
        id: BorderId,
        now: Tick,
        out: &mut O,
    ) -> Res<()> {
        for (p, pos) in self.plan.border_approach_points(id) {
            let ps = point(st, p)?;
            if ps.owner.is_none()
                && !self.in_position(st, p, pos, now)?
                && self.point_tvd_clear(st, p, now)?
            {
                self.command_point(st, now, p, pos, None, false, out)?;
            }
        }
        Ok(())
    }

    /// Ist der eigene Annäherungsabschnitt verschlossen, frei und gesichert?
    pub(super) fn approach_clear(
        &self,
        st: &KernelState<'_>,
        id: BorderId,
        now: Tick,
    ) -> Res<bool> {
        if !border(st, id)?.lock {
            return Ok(false);
        }
        for t in self.plan.border_approach_tvds(id) {
            if !self.tvd_clear(st, t, now)? {
                return Ok(false);
            }
        }
        for (p, pos) in self.plan.border_approach_points(id) {
            if !self.in_position(st, p, pos, now)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Ist der Annäherungsabschnitt verschlossen und von keiner anderen Grenze mit
    /// verschlossen (REQ-IXL-021)?
    pub(super) fn approach_exclusive(&self, st: &KernelState<'_>, id: BorderId) -> Res<bool> {
        if !border(st, id)?.lock {
            return Ok(false);
        }
        for t in self.plan.border_approach_tvds(id) {
            if tvd(st, t)?.border_locks != 1 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Sendet jedem Nachbarn den Stand dieser Seite.
    pub(super) fn send_border_messages<O: Outputs>(
        &self,
        st: &KernelState<'_>,
        now: Tick,
        out: &mut O,
    ) -> Res<()> {
        for id in self.border_ids() {
            let state = border(st, id)?;
            out.command(crate::io::Command::Border {
                border: id,
                message: BorderMessage {
                    holding: state.holding,
                    seq: state.seq,
                    offer: state.offering,
                    approach_clear: self.approach_clear(st, id, now)?,
                    request: state.want && !state.holding,
                    exclusive: self.approach_exclusive(st, id)?,
                },
            });
        }
        Ok(())
    }

    /// Ist der Abschnitt `t` durch einen Annäherungsabschnitt für eine Fahrstraße ab `start`
    /// gesperrt? Nur Fahrstraßen ab dem Einfahrsignal der Grenze dürfen ihn benutzen.
    pub(super) fn border_blocks(
        &self,
        st: &KernelState<'_>,
        t: TvdId,
        start: SignalId,
    ) -> Res<bool> {
        if tvd(st, t)?.border_locks == 0 {
            return Ok(false);
        }
        for id in self.border_ids() {
            if border(st, id)?.lock
                && self.plan.border_approach_tvds(id).any(|x| x == t)
                && self.plan.border_entry_signal(id) != Some(start)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Bedingungen der Grenze für eine Fahrstraße, die an ihr endet: Erlaubnis gehalten und –
    /// für den Fahrtbegriff – der Annäherungsabschnitt des Nachbarn frei und ausschließlich
    /// verschlossen gemeldet (REQ-IXL-020, REQ-IXL-021).
    pub(super) fn exit_permitted(
        &self,
        st: &KernelState<'_>,
        id: BorderId,
        now: Tick,
        need_clear: bool,
    ) -> Res<bool> {
        let state = border(st, id)?;
        if !state.holding {
            return Ok(false);
        }
        if !need_clear {
            return Ok(true);
        }
        Ok(self
            .peer(&state, now)
            .is_some_and(|p| p.approach_clear && p.exclusive && !p.holding))
    }
}
