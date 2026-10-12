//! Dienstschicht: fachliche Operationen unabhängig vom Transport (REST, gRPC).
//!
//! Die Schicht löst Namen auf (über alle Stellbereiche), stellt Anforderungen an die Kerne
//! und wartet auf deren Antwort. Rechteprüfung und Audit kommen hier an einer Stelle zusammen,
//! damit beide Schnittstellen dieselben Regeln haben.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use railit_journal::audit::{AuditRecord, AuditWriter, verify_audit};
use railit_kernel::{AuxId, OperatorId, Request};
use railit_plan::{PlanView, Protection, RouteEnd};
use railit_runtime::keys::to_hex;
use railit_runtime::{Directory, NamedAux, NamedEvent, Snapshot};
use railit_sim::{Fault, TrainInfo, TrainSpec};
use railit_topology::{ElementRef, Topology};
use railit_types::{Length, PointPosition, Speed};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::auth::{Permission, Principal};
use crate::engine::{Answer, EngineMsg, OpsCommand, SimCommand, SimReply};
use railit_runtime::railit_adapters::{LinkFaults, LinkStatus};

/// Fehler einer Operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ServiceError {
    /// Element unbekannt.
    NotFound(String),
    /// Eingabe ungültig.
    BadRequest(String),
    /// Keine Berechtigung.
    Forbidden(String),
    /// Auditprotokoll nicht schreibbar: Bedienungen gesperrt.
    AuditUnavailable,
    /// Engine nicht erreichbar.
    Unavailable,
    /// Keine Antwort des Kerns.
    Timeout,
}

impl ServiceError {
    pub(crate) fn message(&self) -> String {
        match self {
            Self::NotFound(m) | Self::BadRequest(m) | Self::Forbidden(m) => m.clone(),
            Self::AuditUnavailable => {
                "Auditprotokoll nicht schreibbar: Bedienungen gesperrt".into()
            }
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
    #[serde(default)]
    pub(crate) etcs: bool,
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
    audit: Option<Arc<Audit>>,
    operators_from_users: bool,
}

impl Service {
    pub(crate) fn new(
        directory: Directory,
        topology: Arc<Topology>,
        tx: mpsc::Sender<EngineMsg>,
        snapshot: watch::Receiver<Arc<Snapshot>>,
        events: broadcast::Sender<Arc<str>>,
        audit: Option<Arc<Audit>>,
        operators_from_users: bool,
    ) -> Self {
        let plan_info = Arc::new(plan_info(&directory, topology.name()));
        Self {
            directory,
            topology,
            tx,
            snapshot,
            events,
            plan_info,
            audit,
            operators_from_users,
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

    /// Prüft das Recht und den Stellbereich; eine Abweisung wird protokolliert.
    fn authorize(
        &self,
        who: &Principal,
        permission: Permission,
        area: Option<&str>,
        action: &str,
        target: &str,
    ) -> Res<()> {
        if who.may(permission) && who.covers(area) {
            if self.audit.as_ref().is_some_and(|a| a.failed()) {
                return Err(ServiceError::AuditUnavailable);
            }
            return Ok(());
        }
        self.audit(who, action, Some(target), area, "denied".into());
        Err(ServiceError::Forbidden(format!(
            "„{}“ darf „{action}“{} nicht ausführen",
            who.name,
            area.map(|a| format!(" in {a}")).unwrap_or_default()
        )))
    }

    /// Protokolliert eine Handlung im Auditprotokoll (REQ-SEC-004).
    fn audit(
        &self,
        who: &Principal,
        action: &str,
        target: Option<&str>,
        area: Option<&str>,
        outcome: String,
    ) {
        if let Some(audit) = &self.audit {
            audit.append(AuditRecord {
                seq: 0,
                time_ms: unix_ms(),
                user: who.name.clone(),
                action: action.into(),
                target: target.map(str::to_string),
                area: area.map(str::to_string),
                outcome,
            });
        }
    }

    fn area_name(&self, area: usize) -> Option<String> {
        self.directory.area(area).map(|(n, _)| n.to_string())
    }

    /// Anforderung an einen Kern mit Rechteprüfung und Audit.
    async fn operate(
        &self,
        who: &Principal,
        permission: Permission,
        action: &str,
        target: &str,
        area: usize,
        request: Request,
    ) -> Res<Answer> {
        let area_name = self.area_name(area);
        self.authorize(who, permission, area_name.as_deref(), action, target)?;
        let result = self.request(area, request).await;
        let outcome = match &result {
            Ok(a) => match &a.event {
                NamedEvent::Accepted { .. } => "ok".to_string(),
                NamedEvent::Rejected { reason, .. } => format!("rejected: {reason}"),
                other => format!("{other:?}"),
            },
            Err(e) => format!("error: {}", e.message()),
        };
        self.audit(who, action, Some(target), area_name.as_deref(), outcome);
        result
    }

    /// Handlung ohne Kern (Simulation, Betriebsführung) mit Rechteprüfung und Audit.
    async fn act<T>(
        &self,
        who: &Principal,
        permission: Permission,
        action: &str,
        target: &str,
        run: impl Future<Output = Res<T>>,
    ) -> Res<T> {
        self.authorize(who, permission, None, action, target)?;
        let result = run.await;
        let outcome = match &result {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {}", e.message()),
        };
        self.audit(who, action, Some(target), None, outcome);
        result
    }

    /// Bedienernummer für Hilfshandlungen: aus der Benutzerdatei oder – ohne
    /// Benutzerverwaltung – aus dem Kopf `X-Railit-Operator`.
    fn operator(&self, who: &Principal, header: Option<u32>) -> Res<OperatorId> {
        if self.operators_from_users {
            return who.operator.map(OperatorId).ok_or_else(|| {
                ServiceError::Forbidden(format!("„{}“ hat keine Bedienernummer", who.name))
            });
        }
        header.map(OperatorId).ok_or_else(|| {
            ServiceError::BadRequest(
                "Hilfshandlungen verlangen den Kopf X-Railit-Operator (Bedienernummer)".into(),
            )
        })
    }

    pub(crate) async fn set_route(&self, who: &Principal, name: &str) -> Res<Answer> {
        let (area, r) = self
            .directory
            .find_route(name)
            .ok_or_else(|| not_found("Fahrstraße", name))?;
        self.operate(
            who,
            Permission::Operate,
            "set_route",
            name,
            area,
            Request::SetRoute(r),
        )
        .await
    }

    pub(crate) async fn cancel_route(&self, who: &Principal, name: &str) -> Res<Answer> {
        let (area, r) = self
            .directory
            .find_route(name)
            .ok_or_else(|| not_found("Fahrstraße", name))?;
        let request = Request::CancelRoute(r);
        self.operate(
            who,
            Permission::Operate,
            "cancel_route",
            name,
            area,
            request,
        )
        .await
    }

    pub(crate) async fn move_point(
        &self,
        who: &Principal,
        name: &str,
        position: PointPosition,
    ) -> Res<Answer> {
        let (area, p) = self
            .directory
            .find_point(name)
            .ok_or_else(|| not_found("Weiche", name))?;
        let request = Request::MovePoint(p, position);
        self.operate(who, Permission::Operate, "move_point", name, area, request)
            .await
    }

    pub(crate) async fn request_permission(
        &self,
        who: &Principal,
        address: &str,
        want: bool,
    ) -> Res<Answer> {
        let (area, b) = self
            .directory
            .find_border(address)
            .ok_or_else(|| not_found("Grenze", address))?;
        let (action, request) = if want {
            ("request_permission", Request::RequestPermission(b))
        } else {
            ("withdraw_permission", Request::WithdrawPermissionRequest(b))
        };
        self.operate(who, Permission::Operate, action, address, area, request)
            .await
    }

    pub(crate) async fn aux_request(
        &self,
        who: &Principal,
        aux: &NamedAux,
        header_operator: Option<u32>,
    ) -> Res<Answer> {
        let operator = self.operator(who, header_operator)?;
        let (area, op) = self
            .directory
            .resolve_aux(aux)
            .map_err(ServiceError::NotFound)?;
        let target = format!("{aux:?}");
        let request = Request::AuxRequest { op, operator };
        self.operate(who, Permission::Aux, "aux_request", &target, area, request)
            .await
    }

    pub(crate) async fn aux_confirm(
        &self,
        who: &Principal,
        area: Option<&str>,
        aux: u32,
        header_operator: Option<u32>,
    ) -> Res<Answer> {
        let operator = self.operator(who, header_operator)?;
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
        let request = Request::AuxConfirm {
            aux: AuxId(aux),
            operator,
        };
        let target = aux.to_string();
        self.operate(who, Permission::Aux, "aux_confirm", &target, area, request)
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
            SimReply::Done | SimReply::Trains(_) | SimReply::Links(_) => Ok(()),
        }
    }

    pub(crate) async fn trains(&self) -> Res<Vec<TrainInfo>> {
        match self.sim(SimCommand::Trains).await? {
            SimReply::Trains(t) => Ok(t),
            SimReply::Error(e) => Err(ServiceError::BadRequest(e)),
            SimReply::Done | SimReply::Links(_) => Ok(Vec::new()),
        }
    }

    pub(crate) async fn add_train(&self, who: &Principal, train: NewTrain) -> Res<()> {
        let Some(ElementRef::Signal(signal)) = self.topology.lookup(&train.signal) else {
            return Err(not_found("Signal", &train.signal));
        };
        let length = Length::from_m(train.length_m)
            .ok_or_else(|| ServiceError::BadRequest("Zuglänge zu groß".into()))?;
        let target = format!("{} an {}", train.name, train.signal);
        let command = SimCommand::AddTrain(TrainSpec {
            name: train.name,
            signal,
            length,
            speed: Speed::from_kmh(train.speed_kmh),
            ignore_signals: train.ignore_signals,
            etcs: train.etcs,
        });
        self.act(
            who,
            Permission::Simulate,
            "add_train",
            &target,
            self.sim_done(command),
        )
        .await
    }

    pub(crate) async fn remove_train(&self, who: &Principal, name: String) -> Res<()> {
        let target = name.clone();
        let command = SimCommand::RemoveTrain(name);
        self.act(
            who,
            Permission::Simulate,
            "remove_train",
            &target,
            self.sim_done(command),
        )
        .await
    }

    /// Zustand der Übertragungsstrecken zu den Objektsteuerungen (EULYNX/RaSTA).
    pub(crate) async fn links(&self) -> Res<Vec<LinkStatus>> {
        match self.sim(SimCommand::Links).await? {
            SimReply::Links(l) => Ok(l),
            SimReply::Error(e) => Err(ServiceError::BadRequest(e)),
            _ => Ok(Vec::new()),
        }
    }

    /// Störungen einer Übertragungsstrecke setzen (Simulation, Prüfstand).
    pub(crate) async fn link_faults(
        &self,
        who: &Principal,
        controller: String,
        faults: LinkFaults,
    ) -> Res<()> {
        let target = format!("{controller} {faults:?}");
        let command = SimCommand::Link(controller, faults);
        self.act(
            who,
            Permission::Simulate,
            "link_faults",
            &target,
            self.sim_done(command),
        )
        .await
    }

    /// Befehl an die Betriebsführung.
    pub(crate) async fn ops(&self, who: &Principal, command: OpsCommand) -> Res<()> {
        let target = format!("{command:?}");
        let run = async {
            let (reply, rx) = oneshot::channel();
            self.tx
                .send(EngineMsg::Ops { command, reply })
                .await
                .map_err(|_| ServiceError::Unavailable)?;
            rx.await
                .map_err(|_| ServiceError::Unavailable)?
                .map_err(ServiceError::BadRequest)
        };
        self.act(who, Permission::Dispatch, "ops", &target, run)
            .await
    }

    pub(crate) async fn fault(
        &self,
        who: &Principal,
        kind: FaultKind,
        element: &str,
        active: bool,
    ) -> Res<()> {
        let fault = match (kind, self.topology.lookup(element)) {
            (FaultKind::PointDisturbed, Some(ElementRef::Point(p))) => Fault::PointDisturbed(p),
            (FaultKind::TvdDisturbed, Some(ElementRef::Tvd(t))) => Fault::TvdDisturbed(t),
            (FaultKind::TvdOccupied, Some(ElementRef::Tvd(t))) => Fault::TvdOccupied(t),
            (FaultKind::SignalDefective, Some(ElementRef::Signal(s))) => Fault::SignalDefective(s),
            (FaultKind::LxDisturbed, Some(ElementRef::LevelCrossing(l))) => Fault::LxDisturbed(l),
            _ => return Err(not_found("Element", element)),
        };
        let action = if active { "fault" } else { "clear_fault" };
        let target = format!("{kind:?} {element}");
        let command = SimCommand::Fault(fault, active);
        self.act(
            who,
            Permission::Simulate,
            action,
            &target,
            self.sim_done(command),
        )
        .await
    }

    /// Prüft das Auditprotokoll und liefert die letzten Einträge.
    pub(crate) fn audit_log(&self, who: &Principal, last: usize) -> Res<Value> {
        self.authorize(who, Permission::Audit, None, "read_audit", "audit")?;
        let audit = self
            .audit
            .as_ref()
            .ok_or_else(|| ServiceError::NotFound("kein Auditprotokoll".into()))?;
        audit.read(last).map_err(ServiceError::BadRequest)
    }
}

/// Auditprotokoll des Dienstes. Ein Schreibfehler sperrt alle weiteren Bedienungen
/// (fail closed), bis der Dienst neu gestartet ist.
pub(crate) struct Audit {
    path: PathBuf,
    writer: Mutex<AuditWriter<File>>,
    failed: AtomicBool,
}

impl Audit {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let writer = AuditWriter::open(path)
            .map_err(|e| format!("Auditprotokoll {}: {e}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: Mutex::new(writer),
            failed: AtomicBool::new(false),
        })
    }

    fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    fn append(&self, record: AuditRecord) {
        let written = self
            .writer
            .lock()
            .map_err(|_| std::io::Error::other("Sperre vergiftet"))
            .and_then(|mut w| w.append(record));
        if let Err(e) = written {
            tracing::error!(error = %e, "Auditprotokoll nicht schreibbar: Bedienungen gesperrt");
            self.failed.store(true, Ordering::SeqCst);
        }
    }

    fn read(&self, last: usize) -> Result<Value, String> {
        let _guard = self
            .writer
            .lock()
            .map_err(|_| "Sperre vergiftet".to_string())?;
        let text = std::fs::read_to_string(&self.path).map_err(|e| e.to_string())?;
        let summary = verify_audit(text.as_bytes()).map_err(|e| e.to_string())?;
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let records: Vec<Value> = lines
            .iter()
            .skip(lines.len().saturating_sub(last))
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .map(|v| v.get("record").cloned().unwrap_or(Value::Null))
            .collect();
        Ok(json!({
            "records": summary.records,
            "head": summary.head,
            "last": records,
        }))
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
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
