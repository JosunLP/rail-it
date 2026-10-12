//! gRPC-Schnittstelle `railit.v1` (Vertrag: `proto/railit/v1/railit.proto`).
//!
//! Die Dienste sind dünne Übersetzer auf die [`Service`]-Schicht: dieselben Rechte, dasselbe
//! Auditprotokoll, dieselben Antworten wie die REST-API. Der angemeldete Benutzer kommt aus
//! der gemeinsamen Anmelde-Middleware (Erweiterung der Anfrage).

use std::pin::Pin;

use railit_runtime::NamedAux;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};

use crate::auth::Principal;
use crate::engine::{Answer, OpsCommand};
use crate::service::{FaultKind, NewTrain, SafetyAction, Service, ServiceError};

/// Aus dem Vertrag erzeugter Code.
#[allow(
    missing_docs,
    unreachable_pub,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]
pub(crate) mod pb {
    tonic::include_proto!("railit.v1");
}

use pb::diagnostics_server::{Diagnostics, DiagnosticsServer};
use pb::etcs_server::{Etcs, EtcsServer};
use pb::events_server::{Events, EventsServer};
use pb::infrastructure_server::{Infrastructure, InfrastructureServer};
use pb::interlocking_server::{Interlocking, InterlockingServer};
use pb::operations_server::{Operations, OperationsServer};
use pb::safety_server::{Safety, SafetyServer};
use pb::simulation_server::{Simulation, SimulationServer};

/// Alle gRPC-Dienste als Router (unter `/railit.v1.<Dienst>/<Methode>`).
pub(crate) fn router(service: &Service) -> axum::Router {
    let api = Grpc {
        service: service.clone(),
    };
    tonic::service::Routes::new(InterlockingServer::new(api.clone()))
        .add_service(OperationsServer::new(api.clone()))
        .add_service(SimulationServer::new(api.clone()))
        .add_service(EventsServer::new(api.clone()))
        .add_service(EtcsServer::new(api.clone()))
        .add_service(SafetyServer::new(api.clone()))
        .add_service(InfrastructureServer::new(api.clone()))
        .add_service(DiagnosticsServer::new(api))
        .into_axum_router()
}

#[derive(Clone)]
struct Grpc {
    service: Service,
}

type Res<T> = Result<Response<T>, Status>;

impl From<ServiceError> for Status {
    fn from(e: ServiceError) -> Self {
        let message = e.message();
        match e {
            ServiceError::NotFound(_) => Self::not_found(message),
            ServiceError::BadRequest(_) => Self::invalid_argument(message),
            ServiceError::Forbidden(_) => Self::permission_denied(message),
            ServiceError::AuditUnavailable | ServiceError::Unavailable => {
                Self::unavailable(message)
            }
            ServiceError::Timeout => Self::deadline_exceeded(message),
        }
    }
}

fn who<T>(request: &Request<T>) -> Result<Principal, Status> {
    request
        .extensions()
        .get::<Principal>()
        .cloned()
        .ok_or_else(|| Status::unauthenticated("Anmeldung erforderlich"))
}

/// Wert aus einer Zeichenkette wie in der REST-API (z. B. `"left"`).
fn parse<T: DeserializeOwned>(what: &str, text: &str) -> Result<T, Status> {
    serde_json::from_value(json!(text))
        .map_err(|_| Status::invalid_argument(format!("{what} „{text}“ ungültig")))
}

/// Aufzählungswert als Zeichenkette wie in der REST-API.
fn name<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(s)) => s,
        Ok(Value::Null) | Err(_) => String::new(),
        Ok(other) => other.to_string(),
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn answer(a: Answer) -> pb::Answer {
    let event = serde_json::to_value(&a.event).unwrap_or(Value::Null);
    pb::Answer {
        area: a.area,
        accepted: text(&event, "type") == "accepted",
        reason: text(&event, "reason"),
        element: text(&event, "element"),
        aux: a.aux.map(|aux| {
            let v = serde_json::to_value(&aux).unwrap_or(Value::Null);
            pb::AuxEvent {
                r#type: text(&v, "type"),
                aux: u32::try_from(number(&v, "aux")).unwrap_or_default(),
                operator: u32::try_from(number(&v, "operator")).unwrap_or_default(),
                counter: number(&v, "counter"),
            }
        }),
    }
}

fn done() -> Response<pb::Done> {
    Response::new(pb::Done {})
}

#[tonic::async_trait]
impl Interlocking for Grpc {
    async fn get_state(&self, _request: Request<pb::GetStateRequest>) -> Res<pb::State> {
        let s = self.service.snapshot();
        Ok(Response::new(pb::State {
            network: s.network.clone(),
            time_ms: s.time_ms,
            cycle: s.cycle,
            safe_state: s.safe_state.clone().unwrap_or_default(),
            areas: s
                .areas
                .iter()
                .map(|a| pb::AreaState {
                    name: a.name.clone(),
                    cycle: a.cycle,
                    safe_state: a.safe_state.clone().unwrap_or_default(),
                    plan_digest: a.plan_digest.clone(),
                    borders: a
                        .borders
                        .iter()
                        .map(|b| pb::BorderState {
                            name: b.name.clone(),
                            holding: b.holding,
                            seq: b.seq,
                            offering: b.offering,
                            requested: b.requested,
                            approach_locked: b.approach_locked,
                        })
                        .collect(),
                    aux_counter: a.aux_counter,
                })
                .collect(),
            routes: s
                .routes
                .iter()
                .map(|r| pb::RouteState {
                    area: r.area.clone(),
                    name: r.name.clone(),
                    start: r.start.clone(),
                    end: r.end.clone(),
                    phase: name(&r.phase),
                })
                .collect(),
            signals: s
                .signals
                .iter()
                .map(|x| pb::SignalState {
                    area: x.area.clone(),
                    name: x.name.clone(),
                    commanded: name(&x.commanded),
                    picture: x.picture.clone(),
                    substitute: x.substitute,
                })
                .collect(),
            points: s
                .points
                .iter()
                .map(|p| pb::PointState {
                    area: p.area.clone(),
                    name: p.name.clone(),
                    report: name(&p.report),
                    locked_by: p.locked_by.clone().unwrap_or_default(),
                })
                .collect(),
            tvds: s
                .tvds
                .iter()
                .map(|t| pb::TvdState {
                    area: t.area.clone(),
                    name: t.name.clone(),
                    occupancy: name(&t.occupancy),
                    locked_by: t.locked_by.clone().unwrap_or_default(),
                })
                .collect(),
            aux_counter: s.aux_counter,
            json: serde_json::to_string(s.as_ref()).unwrap_or_default(),
        }))
    }

    async fn set_route(&self, request: Request<pb::RouteRequest>) -> Res<pb::Answer> {
        let who = who(&request)?;
        let a = self
            .service
            .set_route(&who, &request.get_ref().route)
            .await?;
        Ok(Response::new(answer(a)))
    }

    async fn cancel_route(&self, request: Request<pb::RouteRequest>) -> Res<pb::Answer> {
        let who = who(&request)?;
        let a = self
            .service
            .cancel_route(&who, &request.get_ref().route)
            .await?;
        Ok(Response::new(answer(a)))
    }

    async fn move_point(&self, request: Request<pb::MovePointRequest>) -> Res<pb::Answer> {
        let who = who(&request)?;
        let r = request.get_ref();
        let position = parse("Lage", &r.position)?;
        let a = self.service.move_point(&who, &r.point, position).await?;
        Ok(Response::new(answer(a)))
    }

    async fn request_permission(&self, request: Request<pb::BorderRequest>) -> Res<pb::Answer> {
        let who = who(&request)?;
        let a = self
            .service
            .request_permission(&who, &request.get_ref().border, true)
            .await?;
        Ok(Response::new(answer(a)))
    }

    async fn withdraw_permission(&self, request: Request<pb::BorderRequest>) -> Res<pb::Answer> {
        let who = who(&request)?;
        let a = self
            .service
            .request_permission(&who, &request.get_ref().border, false)
            .await?;
        Ok(Response::new(answer(a)))
    }

    async fn request_aux(&self, request: Request<pb::AuxRequest>) -> Res<pb::Answer> {
        let who = who(&request)?;
        let r = request.get_ref();
        let aux = NamedAux {
            kind: parse("Hilfshandlung", &r.kind)?,
            element: r.element.clone(),
            position: if r.position.is_empty() {
                None
            } else {
                Some(parse("Lage", &r.position)?)
            },
        };
        let a = self.service.aux_request(&who, &aux, r.operator).await?;
        Ok(Response::new(answer(a)))
    }

    async fn confirm_aux(&self, request: Request<pb::ConfirmAuxRequest>) -> Res<pb::Answer> {
        let who = who(&request)?;
        let r = request.get_ref();
        let area = (!r.area.is_empty()).then_some(r.area.as_str());
        let a = self
            .service
            .aux_confirm(&who, area, r.aux, r.operator)
            .await?;
        Ok(Response::new(answer(a)))
    }
}

#[tonic::async_trait]
impl Operations for Grpc {
    async fn get_operations(
        &self,
        _request: Request<pb::GetOperationsRequest>,
    ) -> Res<pb::OperationsState> {
        let snap = self.service.snapshot();
        let ops = snap
            .ops
            .as_ref()
            .ok_or_else(|| Status::not_found("Betriebsführung nicht eingeschaltet"))?;
        Ok(Response::new(pb::OperationsState {
            ars: ops.ars,
            timetable: ops.timetable.clone(),
            trains: ops
                .trains
                .iter()
                .map(|t| pb::TrainOperation {
                    number: t.number.clone(),
                    berth: t.berth.clone().unwrap_or_default(),
                    left: t.left,
                    timetabled: t.timetabled,
                    hold: t.hold,
                    next_stop: t.next_stop.clone().unwrap_or_default(),
                    at_stop: t.at_stop.clone().unwrap_or_default(),
                    delay_s: t.delay_s,
                    extra_delay_s: t.extra_delay_s,
                })
                .collect(),
        }))
    }

    async fn set_ars(&self, request: Request<pb::SetArsRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let on = request.get_ref().on;
        self.service.ops(&who, OpsCommand::SetArs(on)).await?;
        Ok(done())
    }

    async fn describe(&self, request: Request<pb::DescribeRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let r = request.into_inner();
        let command = OpsCommand::Describe {
            train: r.train,
            signal: r.signal,
        };
        self.service.ops(&who, command).await?;
        Ok(done())
    }

    async fn erase(&self, request: Request<pb::TrainRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let train = request.into_inner().train;
        self.service.ops(&who, OpsCommand::Erase(train)).await?;
        Ok(done())
    }

    async fn hold(&self, request: Request<pb::TrainRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let train = request.into_inner().train;
        self.service
            .ops(&who, OpsCommand::Hold(train, true))
            .await?;
        Ok(done())
    }

    async fn release(&self, request: Request<pb::TrainRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let train = request.into_inner().train;
        self.service
            .ops(&who, OpsCommand::Hold(train, false))
            .await?;
        Ok(done())
    }

    async fn set_platform(&self, request: Request<pb::SetPlatformRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let r = request.into_inner();
        let command = OpsCommand::Platform {
            train: r.train,
            station: r.station,
            platform: r.platform,
        };
        self.service.ops(&who, command).await?;
        Ok(done())
    }

    async fn delay(&self, request: Request<pb::DelayRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let r = request.into_inner();
        self.service
            .ops(&who, OpsCommand::Delay(r.train, r.seconds))
            .await?;
        Ok(done())
    }
}

#[tonic::async_trait]
impl Simulation for Grpc {
    async fn list_trains(&self, _request: Request<pb::ListTrainsRequest>) -> Res<pb::TrainList> {
        let trains = self.service.trains().await?;
        Ok(Response::new(pb::TrainList {
            json: serde_json::to_string(&trains).unwrap_or_default(),
        }))
    }

    async fn add_train(&self, request: Request<pb::AddTrainRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let r = request.into_inner();
        let train = NewTrain {
            name: r.name,
            signal: r.signal,
            length_m: r.length_m,
            speed_kmh: u16::try_from(r.speed_kmh)
                .map_err(|_| Status::invalid_argument("Geschwindigkeit zu groß"))?,
            ignore_signals: r.ignore_signals,
            etcs: r.etcs,
        };
        self.service.add_train(&who, train).await?;
        Ok(done())
    }

    async fn remove_train(&self, request: Request<pb::TrainRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let train = request.into_inner().train;
        self.service.remove_train(&who, train).await?;
        Ok(done())
    }

    async fn set_fault(&self, request: Request<pb::FaultRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let r = request.get_ref();
        let kind: FaultKind = parse("Störung", &r.kind)?;
        self.service.fault(&who, kind, &r.element, r.active).await?;
        Ok(done())
    }
}

#[tonic::async_trait]
impl Etcs for Grpc {
    async fn get_trains(&self, _request: Request<pb::GetEtcsTrainsRequest>) -> Res<pb::EtcsTrains> {
        let snap = self.service.snapshot();
        Ok(Response::new(pb::EtcsTrains {
            trains: snap
                .etcs
                .iter()
                .map(|t| pb::EtcsTrain {
                    train: t.train.clone(),
                    mode: name(&t.mode),
                    reference: t.reference.clone(),
                    passed: t.passed,
                    authority: t.authority.as_ref().map(|a| pb::EtcsAuthority {
                        sections: a
                            .sections
                            .iter()
                            .map(|s| pb::EtcsSection {
                                area: s.area.clone(),
                                route: s.route.clone().unwrap_or_default(),
                                length_mm: s.length_mm,
                                speed_kmh: s.speed_kmh.map(u32::from),
                            })
                            .collect(),
                        end: a.end.clone(),
                        eoa_mm: a.eoa_mm,
                        danger_point_mm: a.danger_point_mm,
                    }),
                })
                .collect(),
        }))
    }
}

#[tonic::async_trait]
impl Infrastructure for Grpc {
    async fn get_layout(&self, _request: Request<pb::GetLayoutRequest>) -> Res<pb::Layout> {
        Ok(Response::new(pb::Layout {
            json: self.service.layout().to_string(),
        }))
    }
}

fn area_of(area: &str) -> Option<&str> {
    (!area.is_empty()).then_some(area)
}

fn channel_of(channel: u32) -> Result<usize, Status> {
    usize::try_from(channel).map_err(|_| Status::invalid_argument("Kanal zu groß"))
}

#[tonic::async_trait]
impl Safety for Grpc {
    async fn get_safety(&self, _request: Request<pb::GetSafetyRequest>) -> Res<pb::SafetyState> {
        let snap = self.service.snapshot();
        Ok(Response::new(pb::SafetyState {
            areas: snap
                .areas
                .iter()
                .map(|a| pb::AreaSafety {
                    area: a.name.clone(),
                    redundancy: name(&a.safety.redundancy),
                    active: a.safety.channels.iter().map(|c| c.active).collect(),
                    trip: a
                        .safety
                        .trip
                        .map(|t| serde_json::to_string(&t).unwrap_or_default())
                        .unwrap_or_default(),
                })
                .collect(),
        }))
    }

    async fn inject_fault(&self, request: Request<pb::SafetyFaultRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let r = request.get_ref();
        let action = SafetyAction::Fault {
            channel: channel_of(r.channel)?,
            fault: parse("Kanalfehler", &r.fault)?,
        };
        self.service
            .safety_command(&who, area_of(&r.area), action)
            .await?;
        Ok(done())
    }

    async fn reintegrate(&self, request: Request<pb::ChannelRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let r = request.get_ref();
        let action = SafetyAction::Reintegrate {
            channel: channel_of(r.channel)?,
        };
        self.service
            .safety_command(&who, area_of(&r.area), action)
            .await?;
        Ok(done())
    }

    async fn reset(&self, request: Request<pb::SafetyResetRequest>) -> Res<pb::Done> {
        let who = who(&request)?;
        let r = request.get_ref();
        self.service
            .safety_command(&who, area_of(&r.area), SafetyAction::Reset)
            .await?;
        Ok(done())
    }
}

type EventStream = Pin<Box<dyn Stream<Item = Result<pb::EventMessage, Status>> + Send>>;

#[tonic::async_trait]
impl Events for Grpc {
    type SubscribeStream = EventStream;

    async fn subscribe(&self, request: Request<pb::SubscribeRequest>) -> Res<EventStream> {
        let types = request.into_inner().types;
        let stream = BroadcastStream::new(self.service.subscribe()).filter_map(move |item| {
            // Zu langsame Empfänger verpassen Ereignisse, statt die Kerne zu bremsen.
            let raw = item.ok()?;
            let value: Value = serde_json::from_str(&raw).ok()?;
            let kind = text(&value, "type");
            if !types.is_empty() && !types.contains(&kind) {
                return None;
            }
            Some(Ok(pb::EventMessage {
                time_ms: number(&value, "time_ms"),
                area: text(&value, "area"),
                r#type: kind,
                json: raw.to_string(),
            }))
        });
        Ok(Response::new(Box::pin(stream)))
    }
}

#[tonic::async_trait]
impl Diagnostics for Grpc {
    async fn who_am_i(&self, request: Request<pb::WhoAmIRequest>) -> Res<pb::User> {
        let who = who(&request)?;
        Ok(Response::new(pb::User {
            name: who.name.clone(),
            roles: who.roles.iter().map(name).collect(),
            operator: who.operator,
            areas: who.areas.clone().unwrap_or_default(),
        }))
    }

    async fn get_plan(&self, _request: Request<pb::GetPlanRequest>) -> Res<pb::Plan> {
        Ok(Response::new(pb::Plan {
            json: self.service.plan_info().to_string(),
        }))
    }

    async fn get_audit(&self, request: Request<pb::GetAuditRequest>) -> Res<pb::Audit> {
        let who = who(&request)?;
        let last = usize::try_from(request.get_ref().last.max(1)).unwrap_or(50);
        let log = self.service.audit_log(&who, last)?;
        Ok(Response::new(pb::Audit {
            records: number(&log, "records"),
            head: text(&log, "head"),
            json: log.get("last").map(Value::to_string).unwrap_or_default(),
        }))
    }
}
