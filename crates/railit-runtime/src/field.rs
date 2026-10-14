//! Anbindung der Feldebene.

use railit_adapters::{EulynxField, LinkFaults, LinkStatus};
use railit_kernel::{Command, Input};
use railit_ops::Spawn;
use railit_sim::{Simulator, TrainSpec};
use railit_types::{Length, SignalId, Speed, Tick};

/// Feldebene aus Sicht der Runtime: Simulator direkt oder über EULYNX SCI und RaSTA
/// ([`EulynxField`]).
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
            pzb: false,
        })
        .map_err(|e| e.to_string())
    }
}

/// Feld mit Simulator dahinter (direkt oder über EULYNX/RaSTA): Züge, Störungen und
/// Übertragungsstrecken für Szenarien, Ausbildung und Prüfstände.
pub trait SimulatedField: FieldAdapter {
    /// Der Simulator.
    fn simulator(&self) -> &Simulator;
    /// Der Simulator.
    fn simulator_mut(&mut self) -> &mut Simulator;
    /// Störungen der Übertragungsstrecke zu einer Objektsteuerung.
    ///
    /// # Errors
    /// Beschreibung, wenn das Feld keine Übertragungsstrecken hat.
    fn set_link_faults(&mut self, controller: &str, faults: LinkFaults) -> Result<(), String> {
        let _ = (controller, faults);
        Err("die Feldebene ist direkt angebunden (ohne EULYNX)".into())
    }
    /// Zustand der Übertragungsstrecken.
    fn links(&self) -> Vec<LinkStatus> {
        Vec::new()
    }
}

impl SimulatedField for Simulator {
    fn simulator(&self) -> &Simulator {
        self
    }

    fn simulator_mut(&mut self) -> &mut Simulator {
        self
    }
}

impl FieldAdapter for EulynxField {
    fn advance(&mut self, now: Tick) {
        EulynxField::advance(self, now);
    }

    fn reports(&self, out: &mut Vec<Input>) {
        out.extend(EulynxField::reports(self));
    }

    fn apply(&mut self, commands: &[Command]) {
        EulynxField::apply(self, commands);
    }

    fn spawn_train(&mut self, spawn: &Spawn, signal: SignalId) -> Result<(), String> {
        self.simulator_mut().spawn_train(spawn, signal)
    }

    fn etcs_reports(&self) -> Vec<(String, SignalId, bool)> {
        self.simulator().etcs_reports()
    }

    fn set_cab_authority(&mut self, train: &str, authority: Option<Vec<SignalId>>) {
        self.simulator_mut().set_cab_authority(train, authority);
    }
}

impl SimulatedField for EulynxField {
    fn simulator(&self) -> &Simulator {
        EulynxField::simulator(self)
    }

    fn simulator_mut(&mut self) -> &mut Simulator {
        EulynxField::simulator_mut(self)
    }

    fn set_link_faults(&mut self, controller: &str, faults: LinkFaults) -> Result<(), String> {
        EulynxField::set_link_faults(self, controller, faults)
    }

    fn links(&self) -> Vec<LinkStatus> {
        EulynxField::links(self)
    }
}

/// Feld zur Laufzeit gewählt (direkt oder über EULYNX), z. B. im Dienst.
pub type DynField = Box<dyn SimulatedField + Send>;

impl FieldAdapter for DynField {
    fn advance(&mut self, now: Tick) {
        (**self).advance(now);
    }

    fn reports(&self, out: &mut Vec<Input>) {
        (**self).reports(out);
    }

    fn apply(&mut self, commands: &[Command]) {
        (**self).apply(commands);
    }

    fn spawn_train(&mut self, spawn: &Spawn, signal: SignalId) -> Result<(), String> {
        (**self).spawn_train(spawn, signal)
    }

    fn etcs_reports(&self) -> Vec<(String, SignalId, bool)> {
        (**self).etcs_reports()
    }

    fn set_cab_authority(&mut self, train: &str, authority: Option<Vec<SignalId>>) {
        (**self).set_cab_authority(train, authority);
    }
}

impl SimulatedField for DynField {
    fn simulator(&self) -> &Simulator {
        (**self).simulator()
    }

    fn simulator_mut(&mut self) -> &mut Simulator {
        (**self).simulator_mut()
    }

    fn set_link_faults(&mut self, controller: &str, faults: LinkFaults) -> Result<(), String> {
        (**self).set_link_faults(controller, faults)
    }

    fn links(&self) -> Vec<LinkStatus> {
        (**self).links()
    }
}
