//! Anbindung der Feldebene.

use railit_kernel::{Command, Input};
use railit_sim::Simulator;
use railit_types::Tick;

/// Feldebene aus Sicht der Runtime: Simulator, später EULYNX-Adapter über RaSTA.
pub trait FieldAdapter {
    /// Schreibt das Feld bis `now` fort (Simulator) bzw. holt eingegangene Telegramme ab.
    fn advance(&mut self, now: Tick);
    /// Hängt die aktuellen Meldungen aller Elemente an `out` an.
    fn reports(&self, out: &mut Vec<Input>);
    /// Übergibt die Stellbefehle des letzten Zyklus.
    fn apply(&mut self, commands: &[Command]);
}

impl FieldAdapter for Simulator {
    fn advance(&mut self, now: Tick) {
        Simulator::advance(self, now);
    }

    fn reports(&self, out: &mut Vec<Input>) {
        out.extend(Simulator::reports(self));
    }

    fn apply(&mut self, commands: &[Command]) {
        Simulator::apply(self, commands);
    }
}
