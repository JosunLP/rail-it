//! Zustandsspeicher fester Größe für einen Kanal.

use railit_kernel::{
    AUX_SLOTS, AuxSlot, BorderState, Buffers, KernelState, LxState, Meta, PointState, RouteState,
    SignalState, TvdState,
};
use railit_plan::PlanView;

/// Zustandsspeicher eines Kanals für Pläne mit höchstens `N` Elementen je Art (Weichen,
/// Abschnitte, Signale, Fahrstraßen, Bahnübergänge, Grenzen).
///
/// Der Speicher liegt dort, wo der Aufrufer ihn anlegt – im statischen Speicher oder auf dem
/// Stapel einer Funktion, die nie zurückkehrt. Er wird einmal angelegt und nie vergrößert.
#[derive(Clone, Debug)]
pub struct ChannelMemory<const N: usize> {
    points: [PointState; N],
    tvds: [TvdState; N],
    signals: [SignalState; N],
    routes: [RouteState; N],
    lxs: [LxState; N],
    borders: [BorderState; N],
    aux: [Option<AuxSlot>; AUX_SLOTS],
    active: [u64; N],
    meta: Meta,
}

impl<const N: usize> Default for ChannelMemory<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> ChannelMemory<N> {
    /// Leerer Speicher.
    pub fn new() -> Self {
        Self {
            points: [PointState::default(); N],
            tvds: [TvdState::default(); N],
            signals: [SignalState::default(); N],
            routes: [RouteState::default(); N],
            lxs: [LxState::default(); N],
            borders: [BorderState::default(); N],
            aux: [None; AUX_SLOTS],
            active: [0; N],
            meta: Meta::default(),
        }
    }

    /// Passt ein Plan in diesen Speicher?
    pub fn fits(plan: &PlanView<'_>) -> bool {
        [
            plan.point_count(),
            plan.tvd_count(),
            plan.signal_count(),
            plan.route_count(),
            plan.lx_count(),
            plan.border_count(),
            plan.route_set_words(),
        ]
        .iter()
        .all(|n| *n <= N)
    }

    /// Sicht auf den Speicher für `plan`.
    ///
    /// # Errors
    /// `None`, wenn der Plan mehr Elemente einer Art hat, als der Speicher fasst.
    pub fn state(&mut self, plan: &PlanView<'_>) -> Option<KernelState<'_>> {
        let buffers = Buffers {
            points: self.points.get_mut(..plan.point_count())?,
            tvds: self.tvds.get_mut(..plan.tvd_count())?,
            signals: self.signals.get_mut(..plan.signal_count())?,
            routes: self.routes.get_mut(..plan.route_count())?,
            lxs: self.lxs.get_mut(..plan.lx_count())?,
            borders: self.borders.get_mut(..plan.border_count())?,
            aux: &mut self.aux,
            active: self.active.get_mut(..plan.route_set_words())?,
            meta: &mut self.meta,
        };
        KernelState::new(plan, buffers).ok()
    }
}
