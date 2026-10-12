//! Anbindung der Feldebene.

use railit_kernel::{Command, Input};
use railit_ops::Spawn;
use railit_sim::{Simulator, TrainSpec};
use railit_types::{Length, SignalId, Speed, Tick};

/// Feldebene aus Sicht der Runtime: Simulator, später EULYNX-Adapter über RaSTA.
pub trait FieldAdapter {
    /// Schreibt das Feld bis `now` fort (Simulator) bzw. holt eingegangene Telegramme ab.
    fn advance(&mut self, now: Tick);
    /// Hängt die aktuellen Meldungen aller Elemente an `out` an.
    fn reports(&self, out: &mut Vec<Input>);
    /// Übergibt die Stellbefehle des letzten Zyklus.
    fn apply(&mut self, commands: &[Command]);
    /// Setzt einen Zug vor dem Signal ein (nur Simulation; Fahrplan der Betriebsführung).
    ///
    /// # Errors
    /// Beschreibung, wenn das Feld keine Züge einsetzen kann.
    fn spawn_train(&mut self, spawn: &Spawn, signal: SignalId) -> Result<(), String> {
        let _ = (spawn, signal);
        Err("die Feldebene setzt keine Züge ein".into())
    }
    /// Positionsmeldungen der ETCS-Fahrzeuggeräte: (Zug, Bezugspunkt, passiert).
    fn etcs_reports(&self) -> Vec<(String, SignalId, bool)> {
        Vec::new()
    }
    /// Fahrerlaubnis an ein Fahrzeuggerät: freigegebene Signale bzw. `None`.
    fn set_cab_authority(&mut self, train: &str, authority: Option<Vec<SignalId>>) {
        let _ = (train, authority);
    }
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

    fn etcs_reports(&self) -> Vec<(String, SignalId, bool)> {
        Simulator::etcs_reports(self)
    }

    fn set_cab_authority(&mut self, train: &str, authority: Option<Vec<SignalId>>) {
        Simulator::set_cab_authority(self, train, authority);
    }

    fn spawn_train(&mut self, spawn: &Spawn, signal: SignalId) -> Result<(), String> {
        self.add_train(TrainSpec {
            name: spawn.train.clone(),
            signal,
            length: Length::from_m(spawn.length_m).ok_or("Zuglänge zu groß")?,
            speed: Speed::from_kmh(spawn.speed_kmh),
            ignore_signals: false,
            etcs: spawn.etcs,
        })
        .map_err(|e| e.to_string())
    }
}
