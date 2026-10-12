//! REST/JSON-API `v1` und Ereignisstrom (Server-Sent Events).
//!
//! | Methode | Pfad | Zweck |
//! |---------|------|-------|
//! | GET | `/v1/health` | Lebenszeichen (ohne Anmeldung) |
//! | GET | `/v1/plan` | Netz, Stellbereiche, Fahrstraßen, Elemente, Prüfsummen |
//! | GET | `/v1/state` | Zustandsbild aller Stellbereiche |
//! | GET | `/v1/areas` | Stellbereiche mit Grenzen und Erlaubnissen |
//! | POST | `/v1/routes/{name}/set` | Fahrstraße einstellen |
//! | POST | `/v1/routes/{name}/cancel` | Fahrstraße zurücknehmen |
//! | POST | `/v1/points/{name}/move` | Weiche umstellen (`{"position":"left"}`) |
//! | POST | `/v1/borders/{address}/request` | Erlaubnis an einer Grenze anfordern |
//! | POST | `/v1/borders/{address}/withdraw` | Anforderung zurückziehen |
//! | GET | `/v1/aux` | Offene Hilfshandlungen und Zählwerk |
//! | POST | `/v1/aux` | Hilfshandlung anfordern (`{"kind":…,"element":…}`) |
//! | POST | `/v1/aux/{id}/confirm` | Hilfshandlung bestätigen (`?area=` bei mehreren Bereichen) |
//! | GET | `/v1/events` | Ereignisstrom (SSE, JSON je Ereignis) |
//! | GET | `/v1/sim/trains` | Züge der Simulation |
//! | POST | `/v1/sim/trains` | Zug einsetzen |
//! | DELETE | `/v1/sim/trains/{name}` | Zug entfernen |
//! | POST | `/v1/sim/faults` | Störung setzen/aufheben |
//!
//! Ist ein Token konfiguriert, verlangen alle Pfade außer `/v1/health` den Kopf
//! `Authorization: Bearer <token>`.
//!
//! Hilfshandlungen verlangen zusätzlich den Kopf `X-Railit-Operator: <Nummer>`. Er
//! **identifiziert** den Bediener für die Zwei-Personen-Regel.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, Request as HttpRequest, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use railit_kernel::OperatorId;
use railit_runtime::{NamedAux, NamedEvent, Snapshot};
use railit_types::PointPosition;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::engine::Answer;
use crate::service::{FaultKind, NewTrain, Service, ServiceError};

/// Gemeinsamer Zustand der Handler.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) service: Service,
    pub(crate) token: Option<Arc<str>>,
}

/// Fehlerantwort `{"error": "..."}`.
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<ServiceError> for ApiError {
    fn from(e: ServiceError) -> Self {
        let status = match e {
            ServiceError::NotFound(_) => StatusCode::NOT_FOUND,
            ServiceError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ServiceError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ServiceError::Timeout => StatusCode::GATEWAY_TIMEOUT,
        };
        Self(status, e.message())
    }
}

type ApiResult = Result<Response, ApiError>;

/// Router mit allen Pfaden.
pub(crate) fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/v1/plan", get(plan))
        .route("/v1/state", get(snapshot))
        .route("/v1/areas", get(areas))
        .route("/v1/routes/{name}/set", post(set_route))
        .route("/v1/routes/{name}/cancel", post(cancel_route))
        .route("/v1/points/{name}/move", post(move_point))
        .route("/v1/borders/{address}/request", post(request_permission))
        .route("/v1/borders/{address}/withdraw", post(withdraw_permission))
        .route("/v1/aux", get(aux_list).post(aux_request))
        .route("/v1/aux/{id}/confirm", post(aux_confirm))
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

/// Antwort eines Kerns als HTTP-Antwort: 200 angenommen, 409 abgelehnt.
fn answer(a: Answer) -> Response {
    let Answer { area, event, aux } = a;
    let status = match event {
        NamedEvent::Accepted { .. } => StatusCode::OK,
        _ => StatusCode::CONFLICT,
    };
    let mut body = serde_json::to_value(&event).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut body {
        map.insert("area".into(), json!(area));
        if let Some(aux) = aux {
            map.insert(
                "aux".into(),
                serde_json::to_value(aux).unwrap_or(Value::Null),
            );
        }
    }
    (status, Json(body)).into_response()
}

fn done() -> Response {
    Json(json!({ "ok": true })).into_response()
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    let snap = state.service.snapshot();
    Json(json!({
        "status": if snap.safe_state.is_some() { "safe_state" } else { "ok" },
        "network": snap.network,
        "areas": snap.areas.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
        "cycle": snap.cycle,
        "time_ms": snap.time_ms,
        "safe_state": snap.safe_state,
    }))
}

async fn plan(State(state): State<AppState>) -> Json<Value> {
    Json(state.service.plan_info().as_ref().clone())
}

async fn snapshot(State(state): State<AppState>) -> Json<Snapshot> {
    Json(state.service.snapshot().as_ref().clone())
}

async fn areas(State(state): State<AppState>) -> Json<Value> {
    Json(json!(state.service.snapshot().areas))
}

async fn set_route(State(state): State<AppState>, Path(name): Path<String>) -> ApiResult {
    Ok(answer(state.service.set_route(&name).await?))
}

async fn cancel_route(State(state): State<AppState>, Path(name): Path<String>) -> ApiResult {
    Ok(answer(state.service.cancel_route(&name).await?))
}

#[derive(Deserialize)]
struct MoveBody {
    position: PointPosition,
}

async fn move_point(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<MoveBody>,
) -> ApiResult {
    Ok(answer(
        state.service.move_point(&name, body.position).await?,
    ))
}

async fn request_permission(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult {
    Ok(answer(
        state.service.request_permission(&address, true).await?,
    ))
}

async fn withdraw_permission(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult {
    Ok(answer(
        state.service.request_permission(&address, false).await?,
    ))
}

/// Bedienerkennung aus `X-Railit-Operator`.
fn operator(headers: &HeaderMap) -> Result<OperatorId, ApiError> {
    headers
        .get("x-railit-operator")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(OperatorId)
        .ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "Hilfshandlungen verlangen den Kopf X-Railit-Operator (Bedienernummer)".into(),
            )
        })
}

async fn aux_list(State(state): State<AppState>) -> Json<Value> {
    let snap = state.service.snapshot();
    Json(json!({
        "pending": snap.pending_aux,
        "counter": snap.aux_counter,
    }))
}

async fn aux_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<NamedAux>,
) -> ApiResult {
    let operator = operator(&headers)?;
    Ok(answer(state.service.aux_request(&body, operator).await?))
}

#[derive(Deserialize)]
struct AreaQuery {
    area: Option<String>,
}

async fn aux_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    Query(query): Query<AreaQuery>,
) -> ApiResult {
    let operator = operator(&headers)?;
    Ok(answer(
        state
            .service
            .aux_confirm(query.area.as_deref(), id, operator)
            .await?,
    ))
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(state.service.subscribe()).filter_map(|item| match item {
        Ok(json) => Some(Ok(SseEvent::default().data(json.as_ref()))),
        // Zu langsame Empfänger verpassen Ereignisse, statt die Kerne zu bremsen.
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn trains(State(state): State<AppState>) -> ApiResult {
    Ok(Json(state.service.trains().await?).into_response())
}

async fn add_train(State(state): State<AppState>, Json(body): Json<NewTrain>) -> ApiResult {
    state.service.add_train(body).await?;
    Ok(done())
}

async fn remove_train(State(state): State<AppState>, Path(name): Path<String>) -> ApiResult {
    state.service.remove_train(name).await?;
    Ok(done())
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

async fn fault(State(state): State<AppState>, Json(body): Json<FaultBody>) -> ApiResult {
    state
        .service
        .fault(body.kind, &body.element, body.active)
        .await?;
    Ok(done())
}
