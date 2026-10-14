//! Feldanbindung des Dienstes: eingebaut (Simulator direkt oder über EULYNX/RaSTA) oder
//! entfernt über einen Feldrechner (`railit-field`, REQ-COM-008).
//!
//! Beim entfernten Feld liegt die Simulation im Feldrechner; Simulationsbefehle (Züge,
//! Störungen) gibt der Dienst an dessen Steuerung weiter ([`FieldControl`]).

use std::net::SocketAddr;

use railit_kernel::{Command, Input};
use railit_runtime::railit_adapters::LinkStatus;
use railit_runtime::railit_adapters::net::UdpField;
use railit_runtime::{DynField, FieldAdapter, SimulatedField};
use railit_sim::{Fault, SimEvent, TrainInfo, TrainSpec};
use railit_topology::Topology;
use railit_types::{SignalId, Tick};
use serde_json::{Value, json};

use crate::engine::{SimCommand, SimReply};
use crate::standby::encode;

/// Hinweis bei Simulationsbefehlen ohne eingebautes Feld.
pub(crate) const REMOTE: &str =
    "Feld liegt im Feldrechner: Simulation dort steuern (railitd --field-control)";

/// Feld des Dienstes.
pub(crate) enum ServiceField {
    /// Simulator im Dienst.
    Local(DynField),
    /// Feldrechner über RaSTA/UDP.
    Remote(Box<UdpField>),
}

impl ServiceField {
    /// Entferntes Feld des Netzes `net` unter `addr` mit der RaSTA-Kennung `id`.
    pub(crate) fn remote(net: &Topology, addr: SocketAddr, id: u32) -> Result<Self, String> {
        Ok(Self::Remote(Box::new(UdpField::connect(net, addr, id)?)))
    }

    /// Das eingebaute Feld.
    pub(crate) fn local_mut(&mut self) -> Option<&mut DynField> {
        match self {
            Self::Local(f) => Some(f),
            Self::Remote(_) => None,
        }
    }

    /// Zustand der Verbindungen zu den Objektsteuerungen.
    pub(crate) fn links(&self) -> Vec<LinkStatus> {
        match self {
            Self::Local(f) => f.links(),
            Self::Remote(f) => f.client().links(),
        }
    }

    /// Führt dieses Stellwerk am Feld (alle Verbindungen stehen)? Das eingebaute Feld gehört
    /// immer dem Dienst.
    pub(crate) fn connected(&self) -> bool {
        match self {
            Self::Local(_) => true,
            Self::Remote(f) => f.client().connected(),
        }
    }

    /// Ereignisse des eingebauten Simulators.
    pub(crate) fn take_sim_events(&mut self) -> Vec<SimEvent> {
        match self {
            Self::Local(f) => f.simulator_mut().take_events(),
            Self::Remote(_) => Vec::new(),
        }
    }
}

impl FieldAdapter for ServiceField {
    fn advance(&mut self, now: Tick) {
        match self {
            Self::Local(f) => f.advance(now),
            // Der Feldrechner hat seine eigene Uhr; RaSTA läuft mit der Wanduhr.
            Self::Remote(f) => f.advance(),
        }
    }

    fn reports(&self, out: &mut Vec<Input>) {
        match self {
            Self::Local(f) => f.reports(out),
            Self::Remote(f) => out.extend(f.reports()),
        }
    }

    fn apply(&mut self, commands: &[Command]) {
        match self {
            Self::Local(f) => f.apply(commands),
            Self::Remote(f) => f.apply(commands),
        }
    }

    fn spawn_train(
        &mut self,
        spawn: &railit_runtime::railit_ops::Spawn,
        signal: SignalId,
    ) -> Result<(), String> {
        match self {
            Self::Local(f) => f.spawn_train(spawn, signal),
            Self::Remote(_) => Err(REMOTE.into()),
        }
    }

    fn etcs_reports(&self) -> Vec<(String, SignalId, bool)> {
        match self {
            Self::Local(f) => f.etcs_reports(),
            Self::Remote(_) => Vec::new(),
        }
    }

    fn set_cab_authority(&mut self, train: &str, authority: Option<Vec<SignalId>>) {
        if let Self::Local(f) = self {
            f.set_cab_authority(train, authority);
        }
    }
}

/// Steuerung des Feldrechners (HTTP) für Simulationsbefehle.
#[derive(Clone)]
pub(crate) struct FieldControl {
    url: String,
    token: Option<String>,
    agent: ureq::Agent,
}

impl FieldControl {
    pub(crate) fn new(url: &str, token: Option<String>) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            token,
            agent: ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(std::time::Duration::from_secs(5)))
                .build()
                .into(),
        }
    }

    /// Netz, das der Feldrechner simuliert.
    pub(crate) fn network(&self) -> Result<String, String> {
        let v = self.request("GET", "/v1/health", None)?;
        v.get("network")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "Feldrechner: Antwort ohne Netz".into())
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let url = format!("{}{path}", self.url);
        let auth = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let result = match (method, body) {
            ("POST", Some(body)) => {
                let mut r = self.agent.post(&url);
                if let Some(a) = &auth {
                    r = r.header("Authorization", a);
                }
                r.send_json(body)
            }
            ("DELETE", _) => {
                let mut r = self.agent.delete(&url);
                if let Some(a) = &auth {
                    r = r.header("Authorization", a);
                }
                r.call()
            }
            _ => {
                let mut r = self.agent.get(&url);
                if let Some(a) = &auth {
                    r = r.header("Authorization", a);
                }
                r.call()
            }
        };
        let mut response = result.map_err(|e| format!("Feldrechner: {e}"))?;
        let status = response.status();
        let value: Value = response.body_mut().read_json().unwrap_or(Value::Null);
        if status.is_success() {
            Ok(value)
        } else {
            Err(value
                .get("error")
                .and_then(Value::as_str)
                .map_or_else(|| format!("Feldrechner: HTTP {status}"), str::to_string))
        }
    }

    /// Führt einen Simulationsbefehl im Feldrechner aus.
    pub(crate) async fn call(&self, command: SimCommand, topology: &Topology) -> SimReply {
        let request = match to_request(&command, topology) {
            Ok(r) => r,
            Err(e) => return SimReply::Error(e),
        };
        let this = self.clone();
        let done = tokio::task::spawn_blocking(move || {
            let (method, path, body) = request;
            this.request(method, &path, body)
        })
        .await;
        match (command, done) {
            (_, Err(e)) => SimReply::Error(e.to_string()),
            (_, Ok(Err(e))) => SimReply::Error(e),
            (SimCommand::Trains, Ok(Ok(v))) => {
                SimReply::Trains(serde_json::from_value::<Vec<TrainInfo>>(v).unwrap_or_default())
            }
            (_, Ok(Ok(_))) => SimReply::Done,
        }
    }
}

type HttpRequest = (&'static str, String, Option<Value>);

fn to_request(command: &SimCommand, topology: &Topology) -> Result<HttpRequest, String> {
    let signal_name = |s: SignalId| topology.signal(s).map(|x| x.name.clone());
    Ok(match command {
        SimCommand::AddTrain(TrainSpec {
            name,
            signal,
            length,
            speed,
            ignore_signals,
            etcs,
        }) => (
            "POST",
            "/v1/sim/trains".into(),
            Some(json!({
                "name": name,
                "signal": signal_name(*signal).ok_or("Signal unbekannt")?,
                "length_m": length.as_mm() / 1000,
                "speed_kmh": speed.as_kmh(),
                "ignore_signals": ignore_signals,
                "etcs": etcs,
            })),
        ),
        SimCommand::RemoveTrain(name) => {
            ("DELETE", format!("/v1/sim/trains/{}", encode(name)), None)
        }
        SimCommand::Trains => ("GET", "/v1/sim/trains".into(), None),
        SimCommand::Fault(fault, active) => {
            let (kind, element) = match *fault {
                Fault::PointDisturbed(p) => {
                    ("point_disturbed", topology.point(p).map(|x| x.name.clone()))
                }
                Fault::TvdDisturbed(t) => {
                    ("tvd_disturbed", topology.tvd(t).map(|x| x.name.clone()))
                }
                Fault::TvdOccupied(t) => ("tvd_occupied", topology.tvd(t).map(|x| x.name.clone())),
                Fault::SignalDefective(s) => ("signal_defective", signal_name(s)),
                Fault::LxDisturbed(l) => (
                    "lx_disturbed",
                    topology.level_crossing(l).map(|x| x.name.clone()),
                ),
            };
            (
                "POST",
                "/v1/sim/faults".into(),
                Some(json!({
                    "kind": kind,
                    "element": element.ok_or("Element unbekannt")?,
                    "active": active,
                })),
            )
        }
        SimCommand::Link(..)
        | SimCommand::Links
        | SimCommand::IlsLink(..)
        | SimCommand::IlsLinks => {
            return Err(
                "Störungen der Übertragungsstrecke nur beim eingebauten Feld (--field eulynx)"
                    .into(),
            );
        }
    })
}
