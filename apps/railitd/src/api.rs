//! REST/JSON-API `v1` und Ereignisstrom (Server-Sent Events).
//!
//! | Methode | Pfad | Zweck |
//! |---------|------|-------|
//! | GET | `/v1/health` | Lebenszeichen (ohne Anmeldung) |
//! | GET | `/v1/plan` | Netz, Fahrstraßen, Elemente, Prüfsumme |
//! | GET | `/v1/state` | Zustandsbild |
//! | POST | `/v1/routes/{name}/set` | Fahrstraße einstellen |
//! | POST | `/v1/routes/{name}/cancel` | Fahrstraße zurücknehmen |
//! | POST | `/v1/points/{name}/move` | Weiche umstellen (`{"position":"left"}`) |
//! | GET | `/v1/events` | Ereignisstrom (SSE, JSON je Ereignis) |
//! | GET | `/v1/sim/trains` | Züge der Simulation |
//! | POST | `/v1/sim/trains` | Zug einsetzen |
//! | DELETE | `/v1/sim/trains/{name}` | Zug entfernen |
//! | POST | `/v1/sim/faults` | Störung setzen/aufheben |
//!
//! Ist ein Token konfiguriert, verlangen alle Pfade außer `/v1/health` den Kopf
//! `Authorization: Bearer <token>`.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Request as HttpRequest, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use railit_kernel::Request;
use railit_plan::{PlanView, RouteEnd};
use railit_runtime::keys::to_hex;
use railit_runtime::{NamedEvent, Snapshot};
use railit_sim::{Fault, TrainSpec};
use railit_types::{Length, PointPosition, Speed};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::engine::{EngineMsg, SimCommand, SimReply};

/// Gemeinsamer Zustand der Handler.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) plan: PlanView<'static>,
    pub(crate) tx: mpsc::Sender<EngineMsg>,
    pub(crate) snapshot: watch::Receiver<Arc<Snapshot>>,
    pub(crate) events: broadcast::Sender<Arc<str>>,
    pub(crate) token: Option<Arc<str>>,
}

/// Fehlerantwort `{"error": "..."}`.
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn not_found(what: &str, name: &str) -> ApiError {
    ApiError(
        StatusCode::NOT_FOUND,
        format!("{what} „{name}“ nicht gefunden"),
    )
}

fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Engine nicht erreichbar".into(),
    )
}

/// Router mit allen Pfaden.
pub(crate) fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/v1/plan", get(plan))
        .route("/v1/state", get(snapshot))
        .route("/v1/routes/{name}/set", post(set_route))
        .route("/v1/routes/{name}/cancel", post(cancel_route))
        .route("/v1/points/{name}/move", post(move_point))
        .route("/v1/events", get(events))
        .route("/v1/sim/trains", get(trains).post(add_train))
        .route("/v1/sim/trains/{name}", delete(remove_train))
        .route("/v1/sim/faults", post(fault))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));
    Router::new()
        .route("/v1/health", get(health))
        .merge(protected)
        .with_state(state)
}

async fn auth(State(state): State<AppState>, request: HttpRequest, next: Next) -> Response {
    if let Some(token) = &state.token {
        let given = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if given != Some(token.as_ref()) {
            return ApiError(StatusCode::UNAUTHORIZED, "Anmeldung erforderlich".into())
                .into_response();
        }
    }
    next.run(request).await
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    let snap = state.snapshot.borrow().clone();
    Json(json!({
        "status": if snap.safe_state.is_some() { "safe_state" } else { "ok" },
        "network": snap.network,
        "cycle": snap.cycle,
        "time_ms": snap.time_ms,
        "safe_state": snap.safe_state,
    }))
}

async fn plan(State(state): State<AppState>) -> Json<Value> {
    let p = state.plan;
    let names = |n: usize, f: &dyn Fn(usize) -> Option<&'static str>| -> Vec<&'static str> {
        (0..n).filter_map(f).collect()
    };
    Json(json!({
        "network": p.network_name(),
        "signed": p.is_signed(),
        "digest": to_hex(&p.digest()),
        "points": names(p.point_count(), &|i| railit_types::PointId::from_index(i).and_then(|id| p.point_name(id))),
        "signals": names(p.signal_count(), &|i| railit_types::SignalId::from_index(i).and_then(|id| p.signal_name(id))),
        "tvds": names(p.tvd_count(), &|i| railit_types::TvdId::from_index(i).and_then(|id| p.tvd_name(id))),
        "routes": p.routes().map(|r| json!({
            "name": r.name(),
            "start": p.signal_name(r.start()),
            "end": match r.end() {
                RouteEnd::Signal(s) => p.signal_name(s).unwrap_or("?"),
                RouteEnd::Boundary => "Netzgrenze",
                RouteEnd::BufferStop => "Prellbock",
            },
            "points": r.points().map(|(pt, pos)| json!({"point": p.point_name(pt), "position": pos})).collect::<Vec<_>>(),
            "tvds": r.tvds().map(|t| p.tvd_name(t)).collect::<Vec<_>>(),
            "conflicts": r.conflicts().map(|c| p.route_name(c)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    }))
}

async fn snapshot(State(state): State<AppState>) -> Json<Snapshot> {
    Json(state.snapshot.borrow().as_ref().clone())
}

/// Stellt eine Anforderung und wartet auf die Antwort des Kerns.
async fn request(state: &AppState, request: Request) -> Result<Response, ApiError> {
    let (reply, rx) = oneshot::channel();
    state
        .tx
        .send(EngineMsg::Request { request, reply })
        .await
        .map_err(|_| unavailable())?;
    let answer = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::GATEWAY_TIMEOUT,
                "keine Antwort des Kerns".into(),
            )
        })?
        .map_err(|_| unavailable())?;
    let status = match answer {
        NamedEvent::Accepted { .. } => StatusCode::OK,
        _ => StatusCode::CONFLICT,
    };
    Ok((status, Json(answer)).into_response())
}

async fn set_route(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let r = state
        .plan
        .find_route(&name)
        .ok_or_else(|| not_found("Fahrstraße", &name))?;
    request(&state, Request::SetRoute(r)).await
}

async fn cancel_route(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let r = state
        .plan
        .find_route(&name)
        .ok_or_else(|| not_found("Fahrstraße", &name))?;
    request(&state, Request::CancelRoute(r)).await
}

#[derive(Deserialize)]
struct MoveBody {
    position: PointPosition,
}

async fn move_point(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<MoveBody>,
) -> Result<Response, ApiError> {
    let p = state
        .plan
        .find_point(&name)
        .ok_or_else(|| not_found("Weiche", &name))?;
    request(&state, Request::MovePoint(p, body.position)).await
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(state.events.subscribe()).filter_map(|item| match item {
        Ok(json) => Some(Ok(SseEvent::default().data(json.as_ref()))),
        // Zu langsame Empfänger verpassen Ereignisse, statt den Kern zu bremsen.
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn sim(state: &AppState, command: SimCommand) -> Result<SimReply, ApiError> {
    let (reply, rx) = oneshot::channel();
    state
        .tx
        .send(EngineMsg::Sim { command, reply })
        .await
        .map_err(|_| unavailable())?;
    rx.await.map_err(|_| unavailable())
}

fn sim_result(reply: SimReply) -> Result<Response, ApiError> {
    match reply {
        SimReply::Done => Ok(Json(json!({ "ok": true })).into_response()),
        SimReply::Trains(t) => Ok(Json(t).into_response()),
        SimReply::Error(e) => Err(ApiError(StatusCode::BAD_REQUEST, e)),
    }
}

async fn trains(State(state): State<AppState>) -> Result<Response, ApiError> {
    sim_result(sim(&state, SimCommand::Trains).await?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrainBody {
    name: String,
    signal: String,
    length_m: u32,
    speed_kmh: u16,
    #[serde(default)]
    ignore_signals: bool,
}

async fn add_train(
    State(state): State<AppState>,
    Json(body): Json<TrainBody>,
) -> Result<Response, ApiError> {
    let signal = state
        .plan
        .find_signal(&body.signal)
        .ok_or_else(|| not_found("Signal", &body.signal))?;
    let length = Length::from_m(body.length_m)
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "Zuglänge zu groß".into()))?;
    let spec = TrainSpec {
        name: body.name,
        signal,
        length,
        speed: Speed::from_kmh(body.speed_kmh),
        ignore_signals: body.ignore_signals,
    };
    sim_result(sim(&state, SimCommand::AddTrain(spec)).await?)
}

async fn remove_train(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    sim_result(sim(&state, SimCommand::RemoveTrain(name)).await?)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum FaultKind {
    PointDisturbed,
    TvdDisturbed,
    TvdOccupied,
    SignalDefective,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FaultBody {
    kind: FaultKind,
    element: String,
    #[serde(default = "yes")]
    active: bool,
}

const fn yes() -> bool {
    true
}

async fn fault(
    State(state): State<AppState>,
    Json(body): Json<FaultBody>,
) -> Result<Response, ApiError> {
    let p = state.plan;
    let fault = match body.kind {
        FaultKind::PointDisturbed => p.find_point(&body.element).map(Fault::PointDisturbed),
        FaultKind::TvdDisturbed => p.find_tvd(&body.element).map(Fault::TvdDisturbed),
        FaultKind::TvdOccupied => p.find_tvd(&body.element).map(Fault::TvdOccupied),
        FaultKind::SignalDefective => p.find_signal(&body.element).map(Fault::SignalDefective),
    }
    .ok_or_else(|| not_found("Element", &body.element))?;
    sim_result(sim(&state, SimCommand::Fault(fault, body.active)).await?)
}
