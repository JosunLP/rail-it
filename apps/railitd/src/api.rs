//! REST/JSON-API `v1` und Ereignisstrom (Server-Sent Events).
//!
//! | Methode | Pfad | Zweck |
//! |---------|------|-------|
//! | GET | `/v1/health` | Lebenszeichen (ohne Anmeldung) |
//! | GET | `/ui/` | Bedienplatz (Spurplan-UI, ohne Anmeldung; siehe [`crate::ui`]) |
//! | GET | `/v1/plan` | Netz, Stellbereiche, Fahrstraßen, Elemente, Prüfsummen |
//! | GET | `/v1/layout` | Spurplan-Geometrie (automatisch berechnet) |
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
//! | GET | `/v1/ops` | Betriebsführung: Züge, Zugnummern, nächster Halt, Verspätung |
//! | POST | `/v1/ops/ars` | Zuglenkung ein/aus (`{"on":true}`) |
//! | POST | `/v1/ops/trains/{train}/describe` | Zugnummer eintragen (`{"signal":"AA"}`) |
//! | DELETE | `/v1/ops/trains/{train}` | Zugnummer löschen |
//! | POST | `/v1/ops/trains/{train}/hold` | Zug zurückhalten |
//! | POST | `/v1/ops/trains/{train}/release` | Zug freigeben |
//! | POST | `/v1/ops/trains/{train}/platform` | Gleiswechsel (`{"station":…,"platform":…}`) |
//! | POST | `/v1/ops/trains/{train}/delay` | Wartezeit verfügen (`{"s":60}`) |
//! | GET | `/v1/events` | Ereignisstrom (SSE, JSON je Ereignis) |
//! | GET | `/v1/sim/trains` | Züge der Simulation |
//! | POST | `/v1/sim/trains` | Zug einsetzen |
//! | DELETE | `/v1/sim/trains/{name}` | Zug entfernen |
//! | POST | `/v1/sim/faults` | Störung setzen/aufheben |
//!
//! | GET | `/v1/field/links` | Übertragungsstrecken zu den Objektsteuerungen (EULYNX/RaSTA) |
//! | POST | `/v1/field/links/{oc}` | Strecke stören (`{"down":true,"loss_percent":20,…}`) |
//! | GET | `/v1/etcs` | ETCS: angemeldete Züge mit Betriebsart und Fahrerlaubnis |
//! | GET | `/v1/whoami` | Angemeldeter Benutzer mit Rollen |
//! | GET | `/v1/audit` | Auditprotokoll prüfen, letzte Einträge (`?last=50`) |
//!
//! **Anmeldung** (siehe [`crate::auth`]): Mit Benutzerdatei meldet sich ein Client mit
//! Client-Zertifikat (mTLS) oder mit `Authorization: Bearer <token>` an; jede Handlung
//! verlangt das Recht ihrer Rolle (sonst 403) und wird im Auditprotokoll festgehalten. Mit
//! einem gemeinsamen Token (`--token`) hat jeder Inhaber alle Rechte; ohne beides ist der
//! Dienst offen (nur Entwicklung). `/v1/health` ist immer ohne Anmeldung erreichbar.
//!
//! Hilfshandlungen: Mit Benutzerdatei gilt die Bedienernummer des Benutzers, sonst der Kopf
//! `X-Railit-Operator: <Nummer>` (Zwei-Personen-Regel).

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Extension, Path, Query, Request as HttpRequest, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use railit_runtime::{NamedAux, NamedEvent, Snapshot};
use railit_types::PointPosition;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::auth::{AuthMode, Permission, Principal};
use crate::engine::{Answer, OpsCommand};
use crate::service::{FaultKind, NewTrain, SafetyAction, Service, ServiceError};

/// Gemeinsamer Zustand der Handler.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) service: Service,
    pub(crate) auth: Arc<AuthMode>,
}

/// Name aus dem Client-Zertifikat der Verbindung (mTLS), von der TLS-Schicht gesetzt.
#[derive(Clone, Debug, Default)]
pub(crate) struct PeerIdentity(pub(crate) Option<String>);

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
            ServiceError::Forbidden(_) => StatusCode::FORBIDDEN,
            ServiceError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ServiceError::AuditUnavailable | ServiceError::Unavailable => {
                StatusCode::SERVICE_UNAVAILABLE
            }
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
        .route("/v1/layout", get(layout))
        .route("/v1/state", get(snapshot))
        .route("/v1/areas", get(areas))
        .route("/v1/routes/{name}/set", post(set_route))
        .route("/v1/routes/{name}/cancel", post(cancel_route))
        .route("/v1/points/{name}/move", post(move_point))
        .route("/v1/borders/{address}/request", post(request_permission))
        .route("/v1/borders/{address}/withdraw", post(withdraw_permission))
        .route("/v1/aux", get(aux_list).post(aux_request))
        .route("/v1/aux/{id}/confirm", post(aux_confirm))
        .route("/v1/ops", get(ops))
        .route("/v1/ops/ars", post(ops_ars))
        .route("/v1/ops/trains/{train}", delete(ops_erase))
        .route("/v1/ops/trains/{train}/describe", post(ops_describe))
        .route("/v1/ops/trains/{train}/hold", post(ops_hold))
        .route("/v1/ops/trains/{train}/release", post(ops_release))
        .route("/v1/ops/trains/{train}/platform", post(ops_platform))
        .route("/v1/ops/trains/{train}/delay", post(ops_delay))
        .route("/v1/events", get(events))
        .route("/v1/sim/trains", get(trains).post(add_train))
        .route("/v1/sim/trains/{name}", delete(remove_train))
        .route("/v1/sim/faults", post(fault))
        .route("/v1/etcs", get(etcs))
        .route("/v1/field/links", get(links))
        .route("/v1/field/links/{controller}", post(link_faults))
        .route("/v1/safety", get(safety))
        .route("/v1/safety/faults", post(safety_fault))
        .route("/v1/safety/reintegrate", post(safety_reintegrate))
        .route("/v1/safety/reset", post(safety_reset))
        .route("/v1/whoami", get(whoami))
        .route("/v1/audit", get(audit))
        .with_state(state.clone())
        // gRPC `railit.v1` auf demselben Port, hinter derselben Anmeldung.
        .merge(crate::grpc::router(&state.service))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));
    Router::new()
        .route("/v1/health", get(health))
        .with_state(state)
        .merge(crate::ui::router())
        .merge(protected)
}

async fn auth(State(state): State<AppState>, mut request: HttpRequest, next: Next) -> Response {
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let cn = request
        .extensions()
        .get::<PeerIdentity>()
        .and_then(|p| p.0.as_deref());
    // gRPC-Clients erwarten einen gRPC-Status statt einer JSON-Fehlerantwort.
    let grpc = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/grpc"));
    let Some(principal) = state.auth.authenticate(bearer, cn) else {
        if grpc {
            return tonic::Status::unauthenticated("Anmeldung erforderlich").into_http();
        }
        return ApiError(StatusCode::UNAUTHORIZED, "Anmeldung erforderlich".into()).into_response();
    };
    if !principal.may(Permission::Read) {
        if grpc {
            return tonic::Status::permission_denied("keine Leseberechtigung").into_http();
        }
        return ApiError(StatusCode::FORBIDDEN, "keine Leseberechtigung".into()).into_response();
    }
    request.extensions_mut().insert(principal);
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

async fn layout(State(state): State<AppState>) -> Json<Value> {
    Json(state.service.layout().as_ref().clone())
}

async fn snapshot(State(state): State<AppState>) -> Json<Snapshot> {
    Json(state.service.snapshot().as_ref().clone())
}

async fn areas(State(state): State<AppState>) -> Json<Value> {
    Json(json!(state.service.snapshot().areas))
}

async fn set_route(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(name): Path<String>,
) -> ApiResult {
    Ok(answer(state.service.set_route(&who, &name).await?))
}

async fn cancel_route(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(name): Path<String>,
) -> ApiResult {
    Ok(answer(state.service.cancel_route(&who, &name).await?))
}

#[derive(Deserialize)]
struct MoveBody {
    position: PointPosition,
}

async fn move_point(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(name): Path<String>,
    Json(body): Json<MoveBody>,
) -> ApiResult {
    Ok(answer(
        state.service.move_point(&who, &name, body.position).await?,
    ))
}

async fn request_permission(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(address): Path<String>,
) -> ApiResult {
    Ok(answer(
        state
            .service
            .request_permission(&who, &address, true)
            .await?,
    ))
}

async fn withdraw_permission(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(address): Path<String>,
) -> ApiResult {
    Ok(answer(
        state
            .service
            .request_permission(&who, &address, false)
            .await?,
    ))
}

/// Bedienernummer aus `X-Railit-Operator`, falls angegeben.
fn operator(headers: &HeaderMap) -> Option<u32> {
    headers
        .get("x-railit-operator")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u32>().ok())
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
    Extension(who): Extension<Principal>,
    headers: HeaderMap,
    Json(body): Json<NamedAux>,
) -> ApiResult {
    Ok(answer(
        state
            .service
            .aux_request(&who, &body, operator(&headers))
            .await?,
    ))
}

#[derive(Deserialize)]
struct AreaQuery {
    area: Option<String>,
}

/// Bestätigung: Hilfshandlung, wie der zweite Bediener sie nennt, und Prüfcode der Anzeige.
#[derive(Deserialize)]
struct ConfirmBody {
    #[serde(flatten)]
    op: NamedAux,
    check: String,
}

async fn aux_confirm(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    Query(query): Query<AreaQuery>,
    Json(body): Json<ConfirmBody>,
) -> ApiResult {
    Ok(answer(
        state
            .service
            .aux_confirm(
                &who,
                query.area.as_deref(),
                id,
                operator(&headers),
                &body.op,
                &body.check,
            )
            .await?,
    ))
}

async fn ops(State(state): State<AppState>) -> ApiResult {
    match &state.service.snapshot().ops {
        Some(ops) => Ok(Json(ops).into_response()),
        None => Err(ApiError(
            StatusCode::NOT_FOUND,
            "Betriebsführung nicht eingeschaltet".into(),
        )),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArsBody {
    on: bool,
}

async fn ops_ars(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Json(body): Json<ArsBody>,
) -> ApiResult {
    state.service.ops(&who, OpsCommand::SetArs(body.on)).await?;
    Ok(done())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescribeBody {
    signal: String,
}

async fn ops_describe(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(train): Path<String>,
    Json(body): Json<DescribeBody>,
) -> ApiResult {
    state
        .service
        .ops(
            &who,
            OpsCommand::Describe {
                train,
                signal: body.signal,
            },
        )
        .await?;
    Ok(done())
}

async fn ops_erase(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(train): Path<String>,
) -> ApiResult {
    state.service.ops(&who, OpsCommand::Erase(train)).await?;
    Ok(done())
}

async fn ops_hold(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(train): Path<String>,
) -> ApiResult {
    state
        .service
        .ops(&who, OpsCommand::Hold(train, true))
        .await?;
    Ok(done())
}

async fn ops_release(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(train): Path<String>,
) -> ApiResult {
    state
        .service
        .ops(&who, OpsCommand::Hold(train, false))
        .await?;
    Ok(done())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformBody {
    station: String,
    platform: String,
}

async fn ops_platform(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(train): Path<String>,
    Json(body): Json<PlatformBody>,
) -> ApiResult {
    state
        .service
        .ops(
            &who,
            OpsCommand::Platform {
                train,
                station: body.station,
                platform: body.platform,
            },
        )
        .await?;
    Ok(done())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DelayBody {
    s: u32,
}

async fn ops_delay(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(train): Path<String>,
    Json(body): Json<DelayBody>,
) -> ApiResult {
    state
        .service
        .ops(&who, OpsCommand::Delay(train, body.s))
        .await?;
    Ok(done())
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

async fn add_train(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Json(body): Json<NewTrain>,
) -> ApiResult {
    state.service.add_train(&who, body).await?;
    Ok(done())
}

async fn remove_train(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(name): Path<String>,
) -> ApiResult {
    state.service.remove_train(&who, name).await?;
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

async fn fault(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Json(body): Json<FaultBody>,
) -> ApiResult {
    state
        .service
        .fault(&who, body.kind, &body.element, body.active)
        .await?;
    Ok(done())
}

async fn etcs(State(state): State<AppState>) -> Json<Value> {
    Json(json!(state.service.snapshot().etcs))
}

async fn links(State(state): State<AppState>) -> ApiResult {
    Ok(Json(state.service.links().await?).into_response())
}

async fn link_faults(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Path(controller): Path<String>,
    Json(faults): Json<railit_runtime::railit_adapters::LinkFaults>,
) -> ApiResult {
    state.service.link_faults(&who, controller, faults).await?;
    Ok(done())
}

async fn safety(State(state): State<AppState>) -> Json<Value> {
    Json(state.service.safety())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SafetyFaultBody {
    area: Option<String>,
    channel: usize,
    fault: railit_runtime::railit_safety::Fault,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelBody {
    area: Option<String>,
    channel: usize,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct AreaBody {
    area: Option<String>,
}

async fn safety_fault(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Json(body): Json<SafetyFaultBody>,
) -> ApiResult {
    let action = SafetyAction::Fault {
        channel: body.channel,
        fault: body.fault,
    };
    state
        .service
        .safety_command(&who, body.area.as_deref(), action)
        .await?;
    Ok(done())
}

async fn safety_reintegrate(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Json(body): Json<ChannelBody>,
) -> ApiResult {
    let action = SafetyAction::Reintegrate {
        channel: body.channel,
    };
    state
        .service
        .safety_command(&who, body.area.as_deref(), action)
        .await?;
    Ok(done())
}

async fn safety_reset(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    body: Option<Json<AreaBody>>,
) -> ApiResult {
    let area = body.and_then(|Json(b)| b.area);
    state
        .service
        .safety_command(&who, area.as_deref(), SafetyAction::Reset)
        .await?;
    Ok(done())
}

async fn whoami(Extension(who): Extension<Principal>) -> Json<Principal> {
    Json(who)
}

#[derive(Deserialize)]
struct AuditQuery {
    last: Option<usize>,
}

async fn audit(
    State(state): State<AppState>,
    Extension(who): Extension<Principal>,
    Query(query): Query<AuditQuery>,
) -> ApiResult {
    let log = state.service.audit_log(&who, query.last.unwrap_or(50))?;
    Ok(Json(log).into_response())
}
