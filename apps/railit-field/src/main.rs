//! `railit-field` – Feldrechner von rail-it (REQ-COM-008, REQ-SVC-013).
//!
//! Simulierte Objektsteuerungen hinter EULYNX SCI und RaSTA über UDP; vor ihnen ein
//! Umschalter, der immer nur ein Stellwerk stellen lässt (siehe
//! `railit_adapters::net::FieldServer`). Haupt- und Bereitschaftsrechner (`railitd --field
//! remote`) verbinden sich mit ihren RaSTA-Kennungen.
//!
//! ```text
//! railit-field --network examples/beispielstadt.toml --listen 127.0.0.1:7900 \
//!              --control 127.0.0.1:7901
//! ```
//!
//! Über `--control` (HTTP) lässt sich die Simulation steuern: Züge, Störungen, Zustand des
//! Umschalters. Es ist eine Prüfstands- und Ausbildungsschnittstelle; ohne `--token` lauscht
//! sie nur auf der Loopback-Adresse.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use clap::Parser;
use railit_adapters::net::{FieldServer, PRIMARY_ID, STANDBY_ID, SWITCH_MS};
use railit_adapters::redundant::RedundantUdp;
use railit_sim::{Fault, SimConfig, Simulator, TrainSpec};
use railit_topology::{ElementRef, Topology};
use railit_types::{Length, Speed, Tick};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Parser)]
#[command(
    name = "railit-field",
    version,
    about = "Feldrechner von rail-it: simulierte Objektsteuerungen über RaSTA/UDP mit Umschalter"
)]
struct Args {
    /// Netzbeschreibung (rail-it-TOML).
    #[arg(long)]
    network: PathBuf,
    /// UDP-Adresse für RaSTA. Mehrfach angegeben: je Adresse ein Kanal der
    /// RaSTA-Redundanzschicht (REQ-COM-011).
    #[arg(long, default_value = "127.0.0.1:7900")]
    listen: Vec<SocketAddr>,
    /// HTTP-Adresse der Simulationssteuerung.
    #[arg(long, default_value = "127.0.0.1:7901")]
    control: SocketAddr,
    /// Zugelassene Stellwerke (RaSTA-Kennung, hexadezimal); Vorgabe: Haupt- und
    /// Bereitschaftsrechner.
    #[arg(long = "interlocking", value_name = "ID", value_parser = parse_id)]
    interlockings: Vec<u32>,
    /// Umschaltzeit: so lange müssen alle Verbindungen des führenden Stellwerks abgebaut
    /// sein, bevor ein anderes führen darf (ms).
    #[arg(long, default_value_t = SWITCH_MS)]
    switch_ms: u64,
    /// Takt des Feldrechners (ms).
    #[arg(long, default_value_t = 20)]
    step_ms: u64,
    /// Zugangstoken der Simulationssteuerung (auch über `RAILIT_FIELD_TOKEN`).
    #[arg(long, env = "RAILIT_FIELD_TOKEN", hide_env_values = true)]
    token: Option<String>,
}

fn parse_id(text: &str) -> Result<u32, String> {
    u32::from_str_radix(text.trim_start_matches("0x"), 16).map_err(|e| e.to_string())
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("railit-field: {msg}");
    ExitCode::from(2)
}

/// Gemeinsamer Zustand von RaSTA-Takt und Steuerung.
struct Field {
    server: Mutex<FieldServer>,
    link: Mutex<RedundantUdp>,
    topology: Topology,
    token: Option<String>,
}

impl Field {
    fn lock(&self) -> MutexGuard<'_, FieldServer> {
        self.server
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn link(&self) -> MutexGuard<'_, RedundantUdp> {
        self.link
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// RaSTA-Takt: PDUs annehmen, im festen Takt das Feld fortschreiben und senden.
fn run_rasta(field: &Field, step: Duration) {
    let clock = Instant::now();
    let mut next = Duration::ZERO;
    loop {
        let mut received = Vec::new();
        field
            .link()
            .receive(&mut |from, pdu| received.push((from, pdu.to_vec())));
        if !received.is_empty() {
            let mut server = field.lock();
            for (from, pdu) in &received {
                server.receive(*from, pdu);
            }
        }
        let elapsed = clock.elapsed();
        if elapsed >= next {
            next = elapsed + step;
            let ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            let out = field.lock().tick(Tick::from_millis(ms));
            let mut link = field.link();
            for (_, pdu) in out {
                link.send(&pdu);
            }
        } else if received.is_empty() {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

type ApiResult = Result<Response, (StatusCode, Json<Value>)>;

fn error(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": msg.into() })))
}

async fn health(State(field): State<Arc<Field>>) -> Json<Value> {
    Json(json!({ "status": "ok", "network": field.topology.name() }))
}

async fn status(State(field): State<Arc<Field>>) -> Json<Value> {
    let server = field.lock();
    Json(json!({
        "network": field.topology.name(),
        "field": server.status(),
        "links": server.links(),
    }))
}

async fn links(State(field): State<Arc<Field>>) -> Json<Value> {
    Json(json!(field.lock().links()))
}

/// Kanäle der RaSTA-Redundanzschicht.
async fn channels(State(field): State<Arc<Field>>) -> Json<Value> {
    Json(json!(field.link().status()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelFault {
    down: bool,
}

/// Stört einen Kanal oder hebt die Störung auf (Prüfstand).
async fn channel_fault(
    State(field): State<Arc<Field>>,
    Path(channel): Path<usize>,
    Json(body): Json<ChannelFault>,
) -> ApiResult {
    field
        .link()
        .set_down(channel, body.down)
        .map_err(|e| error(StatusCode::NOT_FOUND, e))?;
    Ok(Json(json!({ "ok": true })).into_response())
}

async fn trains(State(field): State<Arc<Field>>) -> Json<Value> {
    Json(json!(field.lock().simulator().trains()))
}

/// Neuer Zug (wie `POST /v1/sim/trains` von `railitd`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewTrain {
    name: String,
    signal: String,
    length_m: u32,
    speed_kmh: u16,
    #[serde(default)]
    ignore_signals: bool,
    #[serde(default)]
    etcs: bool,
    #[serde(default)]
    pzb: bool,
}

async fn add_train(State(field): State<Arc<Field>>, Json(t): Json<NewTrain>) -> ApiResult {
    let Some(ElementRef::Signal(signal)) = field.topology.lookup(&t.signal) else {
        return Err(error(
            StatusCode::NOT_FOUND,
            format!("Signal „{}“ nicht gefunden", t.signal),
        ));
    };
    let length = Length::from_m(t.length_m)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Zuglänge zu groß"))?;
    field
        .lock()
        .simulator_mut()
        .add_train(TrainSpec {
            name: t.name,
            signal,
            length,
            speed: Speed::from_kmh(t.speed_kmh),
            ignore_signals: t.ignore_signals,
            etcs: t.etcs,
            pzb: t.pzb,
        })
        .map_err(|e| error(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "status": "ok" })).into_response())
}

async fn remove_train(State(field): State<Arc<Field>>, Path(name): Path<String>) -> ApiResult {
    if field.lock().simulator_mut().remove_train(&name) {
        Ok(Json(json!({ "status": "ok" })).into_response())
    } else {
        Err(error(
            StatusCode::NOT_FOUND,
            format!("Zug „{name}“ nicht gefunden"),
        ))
    }
}

/// Störung (wie `POST /v1/sim/faults` von `railitd`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FaultBody {
    kind: String,
    element: String,
    #[serde(default = "yes")]
    active: bool,
}

const fn yes() -> bool {
    true
}

async fn fault(State(field): State<Arc<Field>>, Json(body): Json<FaultBody>) -> ApiResult {
    let fault = match (body.kind.as_str(), field.topology.lookup(&body.element)) {
        ("point_disturbed", Some(ElementRef::Point(p))) => Fault::PointDisturbed(p),
        ("tvd_disturbed", Some(ElementRef::Tvd(t))) => Fault::TvdDisturbed(t),
        ("tvd_occupied", Some(ElementRef::Tvd(t))) => Fault::TvdOccupied(t),
        ("signal_defective", Some(ElementRef::Signal(s))) => Fault::SignalDefective(s),
        ("lx_disturbed", Some(ElementRef::LevelCrossing(l))) => Fault::LxDisturbed(l),
        _ => {
            return Err(error(
                StatusCode::NOT_FOUND,
                format!("Störung {} an „{}“ nicht möglich", body.kind, body.element),
            ));
        }
    };
    let mut server = field.lock();
    if body.active {
        server.simulator_mut().inject(fault);
    } else {
        server.simulator_mut().clear(fault);
    }
    Ok(Json(json!({ "status": "ok" })).into_response())
}

async fn authorize(State(field): State<Arc<Field>>, request: Request, next: Next) -> Response {
    if let Some(token) = &field.token {
        let presented = request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if presented != Some(token.as_str()) && request.uri().path() != "/v1/health" {
            return error(StatusCode::UNAUTHORIZED, "Anmeldung erforderlich").into_response();
        }
    }
    next.run(request).await
}

fn router(field: Arc<Field>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/status", get(status))
        .route("/v1/field/links", get(links))
        .route("/v1/field/channels", get(channels))
        .route("/v1/field/channels/{channel}", post(channel_fault))
        .route("/v1/sim/trains", get(trains).post(add_train))
        .route("/v1/sim/trains/{name}", delete(remove_train))
        .route("/v1/sim/faults", post(fault))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&field),
            authorize,
        ))
        .with_state(field)
}

fn load(args: &Args) -> Result<Topology, String> {
    let source = std::fs::read_to_string(&args.network)
        .map_err(|e| format!("{}: {e}", args.network.display()))?;
    Ok(
        railit_import::parse(&args.network.to_string_lossy(), &source)
            .map_err(|e| format!("Netz ungültig:\n{e}"))?
            .topology,
    )
}

fn main() -> ExitCode {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    if args.token.is_none() && !args.control.ip().is_loopback() {
        return fail("ohne --token lauscht die Steuerung nur auf der Loopback-Adresse");
    }
    let topology = match load(&args) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let interlockings = if args.interlockings.is_empty() {
        vec![PRIMARY_ID, STANDBY_ID]
    } else {
        args.interlockings.clone()
    };
    let sim = Simulator::new(topology.clone(), SimConfig::default());
    let server = match FieldServer::new(sim, &interlockings, args.switch_ms) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let link = match RedundantUdp::bind(&args.listen) {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    let udp = link
        .local_addrs()
        .iter()
        .zip(&args.listen)
        .map(|(a, l)| a.map_or_else(|| l.to_string(), |a| a.to_string()))
        .collect::<Vec<_>>()
        .join(",");
    let field = Arc::new(Field {
        server: Mutex::new(server),
        link: Mutex::new(link),
        topology,
        token: args.token.clone(),
    });
    let step = Duration::from_millis(args.step_ms.max(1));
    {
        let field = Arc::clone(&field);
        let spawned = std::thread::Builder::new()
            .name("rasta".into())
            .spawn(move || run_rasta(&field, step));
        if let Err(e) = spawned {
            return fail(&format!("Faden: {e}"));
        }
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => return fail(&format!("Tokio: {e}")),
    };
    let result = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(args.control)
            .await
            .map_err(|e| format!("{}: {e}", args.control))?;
        let http = listener
            .local_addr()
            .map_or_else(|_| args.control.to_string(), |a| a.to_string());
        tracing::info!(%udp, %http, ?interlockings, "Feldrechner bereit");
        // Erste Zeile auf stdout: Adressen für Skripte und Tests.
        println!("railit-field: udp://{udp} http://{http}");
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        axum::serve(listener, router(field))
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|e| format!("Server: {e}"))
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}
