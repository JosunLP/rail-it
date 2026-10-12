//! Dienstschicht: fachliche Operationen unabhängig vom Transport (REST, gRPC).
//!
//! Die Schicht löst Namen auf (über alle Stellbereiche), stellt Anforderungen an die Kerne
//! und wartet auf deren Antwort. Rechteprüfung und Audit kommen hier an einer Stelle zusammen,
//! damit beide Schnittstellen dieselben Regeln haben.

use std::sync::Arc;

use railit_kernel::{AuxId, OperatorId, Request};
use railit_plan::{PlanView, Protection, RouteEnd};
use railit_runtime::keys::to_hex;
use railit_runtime::{Directory, NamedAux, Snapshot};
use railit_sim::{Fault, TrainInfo, TrainSpec};
use railit_topology::{ElementRef, Topology};
use railit_types::{Length, PointPosition, Speed};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::engine::{Answer, EngineMsg, SimCommand, SimReply};

/// Fehler einer Operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ServiceError {
    /// Element unbekannt.
    NotFound(String),
    /// Eingabe ungültig.
    BadRequest(String),
    /// Engine nicht erreichbar.
    Unavailable,
    /// Keine Antwort des Kerns.
    Timeout,
}

impl ServiceError {
    pub(crate) fn message(&self) -> String {
        match self {
            Self::NotFound(m) | Self::BadRequest(m) => m.clone(),
            Self::Unavailable => "Engine nicht erreichbar".into(),
            Self::Timeout => "keine Antwort des Kerns".into(),
        }
    }
}

type Res<T> = Result<T, ServiceError>;

fn not_found(what: &str, name: &str) -> ServiceError {
    ServiceError::NotFound(format!("{what} „{name}“ nicht gefunden"))
}

/// Art einer simulierten Störung.
#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FaultKind {
    PointDisturbed,
    TvdDisturbed,
    TvdOccupied,
    SignalDefective,
    LxDisturbed,
}

/// Neuer Zug in der Simulation.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewTrain {
    pub(crate) name: String,
    pub(crate) signal: String,
    pub(crate) length_m: u32,
    pub(crate) speed_kmh: u16,
    #[serde(default)]
    pub(crate) ignore_signals: bool,
}

/// Gemeinsamer Zugang zu Engine, Zustandsbild und Ereignissen.
#[derive(Clone)]
pub(crate) struct Service {
    directory: Directory,
    topology: Arc<Topology>,
    tx: mpsc::Sender<EngineMsg>,
    snapshot: watch::Receiver<Arc<Snapshot>>,
    events: broadcast::Sender<Arc<str>>,
    plan_info: Arc<Value>,
}

impl Service {
    pub(crate) fn new(
        directory: Directory,
        topology: Arc<Topology>,
        tx: mpsc::Sender<EngineMsg>,
        snapshot: watch::Receiver<Arc<Snapshot>>,
        events: broadcast::Sender<Arc<str>>,
    ) -> Self {
        let plan_info = Arc::new(plan_info(&directory, topology.name()));
        Self {
            directory,
            topology,
            tx,
            snapshot,
            events,
            plan_info,
        }
    }

    /// Aktuelles Zustandsbild.
    pub(crate) fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.borrow().clone()
    }

    /// Plan(e) mit Fahrstraßen.
    pub(crate) fn plan_info(&self) -> Arc<Value> {
        self.plan_info.clone()
    }

    /// Neuer Abonnent des Ereignisstroms.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Arc<str>> {
        self.events.subscribe()
    }

    /// Stellt eine Anforderung und wartet auf die Antwort des Kerns.
    async fn request(&self, area: usize, request: Request) -> Res<Answer> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(EngineMsg::Request {
                area,
                request,
                reply,
            })
            .await
            .map_err(|_| ServiceError::Unavailable)?;
        tokio::time::timeout(std::time::Duration::from_secs(10), rx)
            .await
            .map_err(|_| ServiceError::Timeout)?
            .map_err(|_| ServiceError::Unavailable)
    }

    pub(crate) async fn set_route(&self, name: &str) -> Res<Answer> {
        let (area, r) = self
            .directory
            .find_route(name)
            .ok_or_else(|| not_found("Fahrstraße", name))?;
        self.request(area, Request::SetRoute(r)).await
    }

    pub(crate) async fn cancel_route(&self, name: &str) -> Res<Answer> {
        let (area, r) = self
            .directory
            .find_route(name)
            .ok_or_else(|| not_found("Fahrstraße", name))?;
        self.request(area, Request::CancelRoute(r)).await
    }

    pub(crate) async fn move_point(&self, name: &str, position: PointPosition) -> Res<Answer> {
        let (area, p) = self
            .directory
            .find_point(name)
            .ok_or_else(|| not_found("Weiche", name))?;
        self.request(area, Request::MovePoint(p, position)).await
    }

    pub(crate) async fn request_permission(&self, address: &str, want: bool) -> Res<Answer> {
        let (area, b) = self
            .directory
            .find_border(address)
            .ok_or_else(|| not_found("Grenze", address))?;
        let request = if want {
            Request::RequestPermission(b)
        } else {
            Request::WithdrawPermissionRequest(b)
        };
        self.request(area, request).await
    }

    pub(crate) async fn aux_request(&self, aux: &NamedAux, operator: OperatorId) -> Res<Answer> {
        let (area, op) = self
            .directory
            .resolve_aux(aux)
            .map_err(ServiceError::NotFound)?;
        self.request(area, Request::AuxRequest { op, operator })
            .await
    }

    pub(crate) async fn aux_confirm(
        &self,
        area: Option<&str>,
        aux: u32,
        operator: OperatorId,
    ) -> Res<Answer> {
        let area = match area {
            Some(name) => self
                .directory
                .find_area(name)
                .ok_or_else(|| not_found("Stellbereich", name))?,
            None if self.directory.len() == 1 => 0,
            None => {
                return Err(ServiceError::BadRequest(
                    "das Netz hat mehrere Stellbereiche: Bereich angeben".into(),
                ));
            }
        };
        self.request(
            area,
            Request::AuxConfirm {
                aux: AuxId(aux),
                operator,
            },
        )
        .await
    }

    async fn sim(&self, command: SimCommand) -> Res<SimReply> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(EngineMsg::Sim { command, reply })
            .await
            .map_err(|_| ServiceError::Unavailable)?;
        rx.await.map_err(|_| ServiceError::Unavailable)
    }

    async fn sim_done(&self, command: SimCommand) -> Res<()> {
        match self.sim(command).await? {
            SimReply::Error(e) => Err(ServiceError::BadRequest(e)),
            SimReply::Done | SimReply::Trains(_) => Ok(()),
        }
    }

    pub(crate) async fn trains(&self) -> Res<Vec<TrainInfo>> {
        match self.sim(SimCommand::Trains).await? {
            SimReply::Trains(t) => Ok(t),
            SimReply::Error(e) => Err(ServiceError::BadRequest(e)),
            SimReply::Done => Ok(Vec::new()),
        }
    }

    pub(crate) async fn add_train(&self, train: NewTrain) -> Res<()> {
        let Some(ElementRef::Signal(signal)) = self.topology.lookup(&train.signal) else {
            return Err(not_found("Signal", &train.signal));
        };
        let length = Length::from_m(train.length_m)
            .ok_or_else(|| ServiceError::BadRequest("Zuglänge zu groß".into()))?;
        self.sim_done(SimCommand::AddTrain(TrainSpec {
            name: train.name,
            signal,
            length,
            speed: Speed::from_kmh(train.speed_kmh),
            ignore_signals: train.ignore_signals,
        }))
        .await
    }

    pub(crate) async fn remove_train(&self, name: String) -> Res<()> {
        self.sim_done(SimCommand::RemoveTrain(name)).await
    }

    pub(crate) async fn fault(&self, kind: FaultKind, element: &str, active: bool) -> Res<()> {
        let fault = match (kind, self.topology.lookup(element)) {
            (FaultKind::PointDisturbed, Some(ElementRef::Point(p))) => Fault::PointDisturbed(p),
            (FaultKind::TvdDisturbed, Some(ElementRef::Tvd(t))) => Fault::TvdDisturbed(t),
            (FaultKind::TvdOccupied, Some(ElementRef::Tvd(t))) => Fault::TvdOccupied(t),
            (FaultKind::SignalDefective, Some(ElementRef::Signal(s))) => Fault::SignalDefective(s),
            (FaultKind::LxDisturbed, Some(ElementRef::LevelCrossing(l))) => Fault::LxDisturbed(l),
            _ => return Err(not_found("Element", element)),
        };
        self.sim_done(SimCommand::Fault(fault, active)).await
    }
}

fn route_json(p: &PlanView<'_>, area: &str, r: &railit_plan::RouteView<'_>) -> Value {
    let point = |(pt, pos): (railit_types::PointId, PointPosition)| json!({"point": p.point_name(pt), "position": pos});
    json!({
        "area": area,
        "name": r.name(),
        "start": p.signal_name(r.start()),
        "end": match r.end() {
            RouteEnd::Signal(s) => p.signal_name(s).unwrap_or("?").to_string(),
            RouteEnd::Boundary => "Netzgrenze".into(),
            RouteEnd::BufferStop => "Prellbock".into(),
            RouteEnd::Border(b) => format!("{area}{}", p.border_name(b).unwrap_or("?")),
        },
        "speed_kmh": r.speed().map(Speed::as_kmh),
        "points": r.points().map(point).collect::<Vec<_>>(),
        "tvds": r.tvds().map(|t| p.tvd_name(t)).collect::<Vec<_>>(),
        "overlap": {
            "tvds": r.overlap_tvds().map(|t| p.tvd_name(t)).collect::<Vec<_>>(),
            "points": r.overlap_points().map(point).collect::<Vec<_>>(),
        },
        "flank": r.flank().map(|f| match f.protection {
            Protection::Point(pt, pos) => json!({"point": p.point_name(pt), "position": pos, "protects": p.tvd_name(f.protects)}),
            Protection::Signal(s) => json!({"signal": p.signal_name(s), "protects": p.tvd_name(f.protects)}),
        }).collect::<Vec<_>>(),
        "level_crossings": r.level_crossings().map(|l| p.lx_name(l)).collect::<Vec<_>>(),
        "conflicts": r.conflicts().map(|c| p.route_name(c)).collect::<Vec<_>>(),
    })
}

/// Pläne aller Bereiche als JSON (einmal beim Start berechnet).
fn plan_info(directory: &Directory, network: &str) -> Value {
    let names = |n: usize, f: &dyn Fn(usize) -> Option<&'static str>| -> Vec<&'static str> {
        (0..n).filter_map(f).collect()
    };
    let mut routes = Vec::new();
    let mut points = Vec::new();
    let mut signals = Vec::new();
    let mut tvds = Vec::new();
    let mut lxs = Vec::new();
    let mut areas = Vec::new();
    for (area, p) in directory.areas() {
        let p = *p;
        routes.extend(p.routes().map(|r| route_json(&p, area, &r)));
        points.extend(names(p.point_count(), &|i| {
            railit_types::PointId::from_index(i).and_then(|id| p.point_name(id))
        }));
        signals.extend(names(p.signal_count(), &|i| {
            railit_types::SignalId::from_index(i).and_then(|id| p.signal_name(id))
        }));
        tvds.extend(names(p.tvd_count(), &|i| {
            railit_types::TvdId::from_index(i).and_then(|id| p.tvd_name(id))
        }));
        lxs.extend(names(p.lx_count(), &|i| {
            railit_types::LxId::from_index(i).and_then(|id| p.lx_name(id))
        }));
        let borders: Vec<Value> = (0..p.border_count())
            .filter_map(railit_types::BorderId::from_index)
            .map(|b| {
                json!({
                    "name": format!("{area}{}", p.border_name(b).unwrap_or("?")),
                    "entry_signal": p.border_entry_signal(b).and_then(|s| p.signal_name(s)),
                    "initial": p.border_initial(b),
                    "approach": p.border_approach_tvds(b).map(|t| p.tvd_name(t)).collect::<Vec<_>>(),
                })
            })
            .collect();
        areas.push(json!({
            "name": area,
            "signed": p.is_signed(),
            "digest": to_hex(&p.digest()),
            "routes": p.route_count(),
            "borders": borders,
        }));
    }
    let first = directory.area(0).map(|(_, p)| *p);
    json!({
        "network": network,
        "signed": directory.areas().all(|(_, p)| p.is_signed()),
        "digest": first.map(|p| to_hex(&p.digest())),
        "areas": areas,
        "points": points,
        "signals": signals,
        "tvds": tvds,
        "level_crossings": lxs,
        "routes": routes,
    })
}
