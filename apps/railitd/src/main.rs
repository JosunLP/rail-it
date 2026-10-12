//! `railitd` – Stellwerksdienst von rail-it.
//!
//! Lädt Netz und Plan-Image, prüft Signatur und Zusammengehörigkeit, startet Kern und
//! Simulator im festen Zyklus und stellt die REST/JSON-API bereit (siehe [`api`]).
//!
//! ```text
//! railitd --network examples/beispielstadt.toml --plan beispielstadt.rplan \
//!         --trusted-key railit.pub --listen 127.0.0.1:7878
//! ```
//!
//! Die Feldebene ist bisher ausschließlich simuliert. Unsignierte Pläne sind nur mit
//! `--allow-unsigned` zulässig. Mit `--journal` schreibt der Dienst ein Betriebsjournal,
//! das sich mit `railit journal verify|replay` prüfen und abspielen lässt.

mod api;
mod engine;
mod service;

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use railit_plan::{PlanView, VerifyingKey};
use railit_rules::GenericRules;
use railit_runtime::keys::{read_verifying_key, to_hex};
use railit_runtime::railit_ops::{OpsConfig, Timetable};
use railit_runtime::{Runtime, check_correspondence, load_plan};
use railit_sim::{SimConfig, Simulator};
use railit_topology::Topology;
use tokio::sync::{broadcast, mpsc, watch};

#[derive(Debug, Parser)]
#[command(
    name = "railitd",
    version,
    about = "Stellwerksdienst von rail-it (simulierte Feldebene)"
)]
struct Args {
    /// Netzbeschreibung (rail-it-TOML) für Simulator und Abgleich mit dem Plan.
    #[arg(long)]
    network: PathBuf,
    /// Plan-Image (`railit plan compile`); bei Netzen mit Stellbereichen ein Verzeichnis
    /// mit `<Bereich>.rplan` je Stellbereich.
    #[arg(long)]
    plan: PathBuf,
    /// Öffentlicher Schlüssel; der Plan muss damit signiert sein.
    #[arg(
        long,
        required_unless_present = "allow_unsigned",
        conflicts_with = "allow_unsigned"
    )]
    trusted_key: Option<PathBuf>,
    /// Unsignierte Pläne zulassen (nur Simulation und Test).
    #[arg(long)]
    allow_unsigned: bool,
    /// Adresse der API.
    #[arg(long, default_value = "127.0.0.1:7878")]
    listen: SocketAddr,
    /// Zykluszeit des Kerns in Millisekunden (Simulationszeit).
    #[arg(long, default_value_t = 100)]
    cycle_ms: u64,
    /// Zeitraffer: Die Simulation läuft so viel schneller als Echtzeit.
    #[arg(long, default_value_t = 1)]
    speed: u32,
    /// Betriebsjournal: Eingaben jedes Zyklus mit Hashkette (neue Datei; eine vorhandene
    /// wird nicht überschrieben). Bei mehreren Stellbereichen ein Verzeichnis mit
    /// `<Bereich>.journal` je Bereich. Prüfen und abspielen mit `railit journal`.
    #[arg(long)]
    journal: Option<PathBuf>,
    /// Abstand der Zustands-Prüfpunkte im Journal (Zyklen).
    #[arg(long, default_value_t = 600)]
    checkpoint_every: u64,
    /// Fahrplan (TOML, siehe `railit_ops`); setzt Züge nach Plan ein.
    #[arg(long)]
    timetable: Option<PathBuf>,
    /// Zuglenkung einschalten (Fahrstraßen für Züge mit Fahrplan selbsttätig anfordern).
    #[arg(long)]
    ars: bool,
    /// Zugangstoken der API (auch über `RAILIT_TOKEN`).
    #[arg(long, env = "RAILIT_TOKEN", hide_env_values = true)]
    token: Option<String>,
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("railitd: {msg}");
    ExitCode::from(2)
}

/// Lädt ein Plan-Image und prüft die Signatur.
fn read_plan(path: &Path, key: Option<&VerifyingKey>) -> Result<PlanView<'static>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let plan =
        load_plan(bytes, key).map_err(|e| format!("Plan {} abgewiesen: {e}", path.display()))?;
    tracing::info!(
        plan = plan.network_name(),
        routes = plan.route_count(),
        borders = plan.border_count(),
        signed = plan.is_signed(),
        digest = %to_hex(&plan.digest()),
        "Plan geladen"
    );
    Ok(plan)
}

/// Lädt Netz und Plan(e), prüft beide und baut die Runtime.
///
/// Hat das Netz Stellbereiche, ist `--plan` ein Verzeichnis mit `<Bereich>.rplan` je
/// Stellbereich (`railit plan compile --out <Verzeichnis>`).
fn load(args: &Args) -> Result<(Topology, Runtime<Simulator>), String> {
    let source = std::fs::read_to_string(&args.network)
        .map_err(|e| format!("{}: {e}", args.network.display()))?;
    let net = railit_import::toml_format::parse(&source)
        .map_err(|e| format!("Netz ungültig:\n{e}"))?
        .topology;

    let key = args
        .trusted_key
        .as_deref()
        .map(read_verifying_key)
        .transpose()?;
    if key.is_none() {
        tracing::warn!(
            "unsignierte Pläne zugelassen (--allow-unsigned) – nur für Simulation und Test"
        );
    }
    let cycle = railit_types::Duration::from_millis(args.cycle_ms.max(1));
    let sim = Simulator::new(net.clone(), SimConfig::default());
    let rt = if net.areas().len() == 0 {
        let plan = read_plan(&args.plan, key.as_ref())?;
        check_correspondence(&plan, &net).map_err(|e| format!("Plan passt nicht zum Netz: {e}"))?;
        Runtime::new(plan, GenericRules::default(), sim, cycle)
    } else {
        if !args.plan.is_dir() {
            return Err(format!(
                "Netz „{}“ hat Stellbereiche: --plan muss ein Verzeichnis mit <Bereich>.rplan sein",
                net.name()
            ));
        }
        let mut areas = Vec::with_capacity(net.areas().len());
        for (id, area) in net.areas() {
            let plan = read_plan(
                &args.plan.join(format!("{}.rplan", area.name)),
                key.as_ref(),
            )?;
            let cut = net
                .area_cut(id)
                .map_err(|e| format!("Stellbereich „{}“:\n{e}", area.name))?;
            areas.push((plan, cut));
        }
        Runtime::for_network(&net, areas, GenericRules::default(), sim, cycle)
            .map_err(|e| format!("Pläne passen nicht zum Netz: {e}"))?
    };
    let mut rt = rt;
    let timetable = match &args.timetable {
        Some(path) => {
            let source =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Timetable::parse(&source).map_err(|e| format!("Fahrplan {}: {e}", path.display()))?
        }
        None => Timetable::default(),
    };
    let cfg = OpsConfig {
        ars: args.ars,
        ..OpsConfig::default()
    };
    rt.enable_ops(&net, cfg, timetable)?;
    Ok((net, rt))
}

/// Startet die Journale: eine Datei bei einem Stellbereich, sonst ein Verzeichnis mit
/// `<Bereich>.journal` je Stellbereich.
fn start_journals(rt: &mut Runtime<Simulator>, path: &Path, every: u64) -> Result<(), String> {
    let files: Vec<(usize, PathBuf)> = if rt.area_count() == 1 {
        vec![(0, path.to_path_buf())]
    } else {
        std::fs::create_dir_all(path).map_err(|e| format!("Journal {}: {e}", path.display()))?;
        (0..rt.area_count())
            .map(|i| {
                let name = rt.area_name(i).unwrap_or("?");
                (i, path.join(format!("{name}.journal")))
            })
            .collect()
    };
    for (area, file) in files {
        let out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)
            .map_err(|e| format!("Journal {}: {e}", file.display()))?;
        rt.start_journal(area, Box::new(std::io::BufWriter::new(out)), every)
            .map_err(|e| format!("Journal {}: {e}", file.display()))?;
        tracing::info!(path = %file.display(), "Journal läuft");
    }
    Ok(())
}

/// Startet Engine und API und läuft bis Strg+C.
async fn serve(args: Args, net: Topology, rt: Runtime<Simulator>) -> Result<(), String> {
    let period = std::time::Duration::from_micros(
        (args.cycle_ms.max(1) * 1000 / u64::from(args.speed.max(1))).max(1000),
    );
    let (tx, rx) = mpsc::channel(256);
    let (snapshot_tx, snapshot_rx) = watch::channel(Arc::new(rt.snapshot()));
    let (events_tx, _) = broadcast::channel(4096);
    let directory = rt.directory().clone();
    let channels = engine::Channels {
        rx,
        snapshot: snapshot_tx,
        events: events_tx.clone(),
    };
    let engine = tokio::spawn(engine::run(rt, channels, period));

    let state = api::AppState {
        service: service::Service::new(directory, Arc::new(net), tx, snapshot_rx, events_tx),
        token: args.token.map(Arc::from),
    };
    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .map_err(|e| format!("{}: {e}", args.listen))?;
    let addr = listener
        .local_addr()
        .map_or_else(|_| args.listen.to_string(), |a| a.to_string());
    tracing::info!(%addr, cycle_ms = args.cycle_ms, speed = args.speed, "API bereit");
    // Erste Zeile auf stdout: Adresse für Skripte und Tests.
    println!("railitd: http://{addr}");
    let _ = std::io::stdout().flush();

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("wird beendet");
    };
    let served = axum::serve(listener, api::router(state))
        .with_graceful_shutdown(shutdown)
        .await;
    engine.abort();
    served.map_err(|e| format!("Server: {e}"))
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

    let (net, mut rt) = match load(&args) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    if let Some(path) = &args.journal
        && let Err(e) = start_journals(&mut rt, path, args.checkpoint_every)
    {
        return fail(&e);
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => return fail(&format!("Tokio: {e}")),
    };
    match runtime.block_on(serve(args, net, rt)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}
