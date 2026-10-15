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
//!
//! Sicherer Rechner: Mit `--node` rechnet der Kern eines Stellbereichs auf einem Knoten
//! (`railit-node`, RaSTA über UDP); der Dienst führt einen geprüften Spiegel für Anzeige und
//! Betrieb (siehe `railit_runtime::remote`).
//!
//! Sicherheit (siehe [`auth`], [`tls`]): `--users` mit Rollen und Auditprotokoll
//! (`--audit`), TLS mit `--tls-cert`/`--tls-key`, gegenseitige Anmeldung mit
//! `--tls-client-ca`. Ohne Anmeldung lauscht der Dienst nur auf der Loopback-Adresse.

mod api;
mod auth;
mod engine;
mod field;
mod grpc;
mod metrics;
mod review;
mod service;
mod standby;
mod takeover;
mod tls;
mod ui;

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use auth::{AuthMode, Users};
use clap::Parser;
use field::{FieldControl, ServiceField};
use railit_plan::PlanView;
use railit_rules::GenericRules;
use railit_runtime::DynField;
use railit_runtime::keys::{PlanTrust, to_hex};
use railit_runtime::railit_adapters::EulynxField;
use railit_runtime::railit_ops::{OpsConfig, Timetable};
use railit_runtime::railit_safety::Redundancy;
use railit_runtime::{Runtime, SafetyConfig, check_correspondence};
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
    /// Öffentlicher Schlüssel eines Prüfers (mehrfach angebbar, REQ-PLN-022).
    #[arg(long = "approver-key", value_name = "DATEI")]
    approver_keys: Vec<PathBuf>,
    /// Vier-Augen-Prinzip: Jeder Plan braucht so viele Freigaben verschiedener Prüfer
    /// (`<plan>.approvals`, `railit plan approve`).
    #[arg(long, default_value_t = 0, requires = "trusted_key")]
    approvals: usize,
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
    /// Bereitschaftsrechner: liest die Journale des Hauptrechners unter dieser Adresse mit
    /// (`http://host:port`), rechnet nach, quittiert und zeigt dessen Zustand an – ohne Feld,
    /// ohne Kern im Betrieb, ohne Bedienung. `--journal` gilt erst nach einer Übernahme.
    #[arg(long, value_name = "URL", conflicts_with = "nodes")]
    standby: Option<String>,
    /// Zugangstoken beim Hauptrechner (mit `--standby`; Rolle `bereitschaft`).
    #[arg(long, value_name = "TOKEN", requires = "standby")]
    standby_token: Option<String>,
    /// Übernahme: Schweigt der Hauptrechner so lange (ms), führt der Bereitschaftsrechner am
    /// Feldrechner, sobald der Umschalter ihn lässt (mit `--standby` und `--field remote`).
    #[arg(long, value_name = "MS", requires = "standby")]
    takeover_after_ms: Option<u64>,
    /// Ausgabesperre für einen Bereitschaftsrechner: Ausgaben gehen erst nach seiner Quittung
    /// hinaus und warten darauf höchstens so lange (ms). Muss die Umschaltzeit des
    /// Feldrechners, `t_max` von RaSTA und `--takeover-after-ms` deutlich übersteigen.
    #[arg(long, value_name = "MS", requires = "journal")]
    standby_gate_ms: Option<u64>,
    /// Abstand der Zustands-Prüfpunkte im Journal (Zyklen).
    #[arg(long, default_value_t = 600)]
    checkpoint_every: u64,
    /// Geheimer Schlüssel des Stellwerks (`railit plan keygen`): Journal und Auditprotokoll
    /// werden damit gesiegelt (Prüfung mit `railit journal verify --key`).
    #[arg(long)]
    seal_key: Option<PathBuf>,
    /// Abstand der Siegel im Journal (Zyklen).
    #[arg(long, default_value_t = 600)]
    seal_every: u64,
    /// Anbindung der (simulierten) Feldebene: direkt, über EULYNX SCI und RaSTA oder an einen
    /// Feldrechner (`railit-field`) über RaSTA/UDP.
    #[arg(long, value_enum, default_value_t = FieldArg::Direct)]
    field: FieldArg,
    /// Adresse des Feldrechners (UDP, mit `--field remote`). Mehrfach angegeben: je Adresse
    /// ein Kanal der RaSTA-Redundanzschicht (in derselben Reihenfolge wie `--listen` des
    /// Feldrechners).
    #[arg(long, value_name = "ADRESSE", required_if_eq("field", "remote"))]
    field_addr: Vec<SocketAddr>,
    /// RaSTA-Kennung dieses Stellwerks beim Feldrechner (hexadezimal); Vorgabe: Hauptrechner
    /// 0A000001, mit `--standby` Bereitschaftsrechner 0A000002.
    #[arg(long, value_name = "ID", value_parser = parse_id)]
    field_id: Option<u32>,
    /// Steuerung des Feldrechners (HTTP) für Simulationsbefehle, z. B. `http://127.0.0.1:7901`.
    #[arg(long, value_name = "URL")]
    field_control: Option<String>,
    /// Zugangstoken der Steuerung des Feldrechners (auch über `RAILIT_FIELD_TOKEN`).
    #[arg(long, env = "RAILIT_FIELD_TOKEN", hide_env_values = true)]
    field_token: Option<String>,
    /// Übertragung der Grenzmeldungen zwischen den Stellbereichen: direkt im Prozess oder
    /// über EULYNX SCI-ILS und RaSTA mit störbarer Strecke (`/v1/ils`).
    #[arg(long, value_enum, default_value_t = BordersArg::Direct)]
    borders: BordersArg,
    /// Nur diese Stellbereiche rechnen (kommagetrennt); die übrigen rechnen in anderen
    /// Prozessen und sind über SCI-ILS auf UDP angebunden (`--ils-listen`, `--ils-peer`).
    #[arg(long, value_delimiter = ',', requires = "ils_listen")]
    areas: Vec<String>,
    /// UDP-Adresse für SCI-ILS zu den Stellbereichen anderer Prozesse.
    #[arg(long, value_name = "ADRESSE", requires = "areas")]
    ils_listen: Option<SocketAddr>,
    /// Adresse eines Nachbarbereichs in einem anderen Prozess: `BEREICH=ADRESSE` (je Bereich
    /// und Rechner; hat der Nachbar einen Bereitschaftsrechner, beide Adressen).
    #[arg(long = "ils-peer", value_name = "BEREICH=ADRESSE", requires = "areas")]
    ils_peers: Vec<String>,
    /// Kanäle je Stellbereich: 1 (ohne Vergleich), 2 (2-von-2) oder 3 (2-von-3).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=3))]
    channels: u8,
    /// Watchdog: Frist für die Rechenzeit eines Stellbereichs je Zyklus in Millisekunden
    /// (Wanduhr). Überschreitung oder ausbleibende Zyklen führen in den sicheren Zustand.
    #[arg(long)]
    watchdog_ms: Option<u64>,
    /// Fahrplan (TOML, siehe `railit_ops`); setzt Züge nach Plan ein.
    #[arg(long)]
    timetable: Option<PathBuf>,
    /// Zuglenkung einschalten (Fahrstraßen für Züge mit Fahrplan selbsttätig anfordern).
    #[arg(long)]
    ars: bool,
    /// Gemeinsames Zugangstoken mit allen Rechten (auch über `RAILIT_TOKEN`).
    #[arg(
        long,
        env = "RAILIT_TOKEN",
        hide_env_values = true,
        conflicts_with = "users"
    )]
    token: Option<String>,
    /// Benutzerdatei mit Rollen, Token-Hashes und Zertifikatsnamen.
    #[arg(long, requires = "audit")]
    users: Option<PathBuf>,
    /// Auditprotokoll (wird fortgesetzt; eine manipulierte Datei verhindert den Start).
    #[arg(long)]
    audit: Option<PathBuf>,
    /// Serverzertifikat (PEM, mit Kette) für TLS.
    #[arg(long, requires = "tls_key")]
    tls_cert: Option<PathBuf>,
    /// Privater Schlüssel des Servers (PEM).
    #[arg(long, requires = "tls_cert")]
    tls_key: Option<PathBuf>,
    /// CA für Client-Zertifikate: verlangt mTLS; der CN identifiziert den Benutzer.
    #[arg(long, requires_all = ["tls_cert", "users"])]
    tls_client_ca: Option<PathBuf>,
    /// Sperrliste (CRL, PEM) der Client-CA (mehrfach angebbar). Wird bei Änderung neu
    /// gelesen; abgelaufene oder unlesbare Sperrlisten weisen jede Anmeldung ab.
    #[arg(long = "tls-crl", value_name = "DATEI", requires = "tls_client_ca")]
    tls_crls: Vec<PathBuf>,
    /// Stellbereich auf einem Knoten des sicheren Rechners (`railit-node`): `ADRESSE` bei
    /// einem Stellbereich, sonst `BEREICH=ADRESSE` je Bereich; `lockstep` lässt alle Bereiche
    /// auf Knoten im Dienst rechnen (Gleichschritt, für Prüfung und Ausbildung).
    #[arg(long = "node", value_name = "[BEREICH=]ADRESSE|lockstep")]
    nodes: Vec<String>,
}

/// Anbindung der Feldebene.
#[derive(Copy, Clone, Debug, clap::ValueEnum)]
enum FieldArg {
    /// Simulator direkt am Kern.
    Direct,
    /// Objektsteuerungen über EULYNX SCI und RaSTA vor dem Simulator.
    Eulynx,
    /// Feldrechner (`railit-field`) über RaSTA/UDP; die Simulation läuft dort.
    Remote,
}

/// Übertragung der Grenzmeldungen.
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
enum BordersArg {
    /// Ideal im Prozess.
    Direct,
    /// Über EULYNX SCI-ILS und RaSTA, je Paar benachbarter Stellbereiche eine Verbindung.
    Ils,
}

fn parse_id(text: &str) -> Result<u32, String> {
    u32::from_str_radix(text.trim_start_matches("0x"), 16).map_err(|e| e.to_string())
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("railitd: {msg}");
    ExitCode::from(2)
}

/// Lädt ein Plan-Image und prüft Signatur und Freigaben.
fn read_plan(path: &Path, trust: &PlanTrust) -> Result<PlanView<'static>, String> {
    let (plan, approved_by) = trust.load(path)?;
    tracing::info!(
        approved_by = approved_by.join(", "),
        plan = plan.network_name(),
        routes = plan.route_count(),
        borders = plan.border_count(),
        signed = plan.is_signed(),
        digest = %to_hex(&plan.digest()),
        "Plan geladen"
    );
    Ok(plan)
}

/// Kerne aus den Plänen: ein Plan für ein Netz ohne Stellbereiche, sonst je (lokalem)
/// Stellbereich einer.
fn runtime_for(
    args: &Args,
    net: &Topology,
    trust: &PlanTrust,
    sim: ServiceField,
    cycle: railit_types::Duration,
) -> Result<Runtime<ServiceField>, String> {
    Ok(if net.areas().len() == 0 {
        if !args.areas.is_empty() {
            return Err(format!(
                "Netz „{}“ hat keine Stellbereiche (--areas)",
                net.name()
            ));
        }
        let plan = read_plan(&args.plan, trust)?;
        check_correspondence(&plan, net).map_err(|e| format!("Plan passt nicht zum Netz: {e}"))?;
        Runtime::new(plan, GenericRules::default(), sim, cycle)
    } else {
        if !args.plan.is_dir() {
            return Err(format!(
                "Netz „{}“ hat Stellbereiche: --plan muss ein Verzeichnis mit <Bereich>.rplan sein",
                net.name()
            ));
        }
        for name in &args.areas {
            if !net.areas().any(|(_, a)| a.name == *name) {
                return Err(format!("Stellbereich „{name}“ nicht im Netz"));
            }
        }
        let mut areas = Vec::with_capacity(net.areas().len());
        for (id, area) in net.areas() {
            if !args.areas.is_empty() && !args.areas.contains(&area.name) {
                continue;
            }
            let plan = read_plan(&args.plan.join(format!("{}.rplan", area.name)), trust)?;
            let cut = net
                .area_cut(id)
                .map_err(|e| format!("Stellbereich „{}“:\n{e}", area.name))?;
            areas.push((plan, cut));
        }
        Runtime::for_network(net, areas, GenericRules::default(), sim, cycle)
            .map_err(|e| format!("Pläne passen nicht zum Netz: {e}"))?
    })
}

/// Lädt Netz und Plan(e), prüft beide und baut die Runtime.
///
/// Hat das Netz Stellbereiche, ist `--plan` ein Verzeichnis mit `<Bereich>.rplan` je
/// Stellbereich (`railit plan compile --out <Verzeichnis>`).
fn load(args: &Args) -> Result<(Topology, Runtime<ServiceField>), String> {
    let source = std::fs::read_to_string(&args.network)
        .map_err(|e| format!("{}: {e}", args.network.display()))?;
    let net = railit_import::parse(&args.network.to_string_lossy(), &source)
        .map_err(|e| format!("Netz ungültig:\n{e}"))?
        .topology;

    let trust = PlanTrust::from_files(
        args.trusted_key.as_deref(),
        &args.approver_keys,
        args.approvals,
    )?;
    if trust.creator.is_none() {
        tracing::warn!(
            "unsignierte Pläne zugelassen (--allow-unsigned) – nur für Simulation und Test"
        );
    }
    let cycle = railit_types::Duration::from_millis(args.cycle_ms.max(1));
    let sim = Simulator::new(net.clone(), SimConfig::default());
    let sim = match (args.field, args.field_addr.first()) {
        (FieldArg::Direct, _) => ServiceField::Local(Box::new(sim) as DynField),
        (FieldArg::Eulynx, _) => ServiceField::Local(Box::new(EulynxField::new(sim)?)),
        (FieldArg::Remote, Some(_)) => {
            let addrs = &args.field_addr;
            if args.speed != 1 {
                return Err(
                    "der Feldrechner läuft in Echtzeit: --field remote nur mit --speed 1".into(),
                );
            }
            let id = args.field_id.unwrap_or(if args.standby.is_some() {
                railit_runtime::railit_adapters::net::STANDBY_ID
            } else {
                railit_runtime::railit_adapters::net::PRIMARY_ID
            });
            tracing::info!(
                addr = ?addrs,
                channels = addrs.len(),
                id = format!("{id:08X}"),
                "Feld über den Feldrechner"
            );
            if !args.areas.is_empty() && args.field_id.is_none() {
                tracing::warn!(
                    "verteilte Stellbereiche am Feldrechner: jeder Dienst braucht eine eigene \
                     RaSTA-Kennung (--field-id)"
                );
            }
            let areas = (!args.areas.is_empty()).then_some(args.areas.as_slice());
            ServiceField::remote(&net, addrs, id, areas, field_control(args))?
        }
        (FieldArg::Remote, None) => return Err("--field remote verlangt --field-addr".into()),
    };
    let mut rt = runtime_for(args, &net, &trust, sim, cycle)?;
    let period_us = args
        .cycle_ms
        .max(1)
        .saturating_mul(1000)
        .checked_div(u64::from(args.speed.max(1)))
        .unwrap_or(1);
    rt.set_safety(SafetyConfig {
        redundancy: Redundancy::from_channels(usize::from(args.channels)).unwrap_or_default(),
        // Ausbleibende Zyklen: mehr als vier Zyklen plus Frist ohne Zyklusbeginn.
        watchdog: args.watchdog_ms.map(|ms| {
            let deadline = ms.saturating_mul(1000);
            (
                deadline,
                period_us.saturating_mul(4).saturating_add(deadline),
            )
        }),
        ..SafetyConfig::default()
    })?;
    if args.channels > 1 {
        tracing::info!(channels = args.channels, "redundante Kanäle mit Voter");
    }
    attach_nodes(&mut rt, args)?;
    use_borders(&mut rt, args)?;
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

/// Übertragung der Grenzmeldungen (`--borders`, `--areas` mit `--ils-listen` und
/// `--ils-peer`).
fn use_borders(rt: &mut Runtime<ServiceField>, args: &Args) -> Result<(), String> {
    if let Some(listen) = args.ils_listen {
        let mut peers = Vec::new();
        for p in &args.ils_peers {
            let (area, addr) = p
                .split_once('=')
                .ok_or_else(|| format!("--ils-peer {p}: erwartet BEREICH=ADRESSE"))?;
            let addr = addr.parse().map_err(|e| format!("--ils-peer {p}: {e}"))?;
            peers.push((area.to_string(), addr));
        }
        let addr = rt.use_ils_udp(listen, peers)?;
        tracing::info!(%addr, areas = ?args.areas, "Stellbereiche verteilt, SCI-ILS über UDP");
        if matches!(args.field, FieldArg::Direct | FieldArg::Eulynx) {
            tracing::warn!(
                "eingebautes Feld bei verteilten Stellbereichen: Züge fahren nicht über die \
                 Prozessgrenze (gemeinsames Feld: --field remote)"
            );
        }
    } else if args.borders == BordersArg::Ils {
        rt.use_ils()?;
        tracing::info!(links = rt.ils_links().len(), "Grenzmeldungen über SCI-ILS");
    }
    Ok(())
}

/// Bindet Stellbereiche an Knoten des sicheren Rechners an (`--node`).
fn attach_nodes(rt: &mut Runtime<ServiceField>, args: &Args) -> Result<(), String> {
    if args.nodes.is_empty() {
        return Ok(());
    }
    if args.nodes.iter().any(|n| n == "lockstep") {
        if args.nodes.len() > 1 {
            return Err("--node lockstep gilt für alle Stellbereiche".into());
        }
        rt.use_lockstep_nodes()?;
        tracing::info!("Stellbereiche auf Knoten im Gleichschritt");
        return Ok(());
    }
    if args.speed != 1 {
        return Err("Knoten rechnen in Echtzeit: --node nur mit --speed 1".into());
    }
    for spec in &args.nodes {
        let (area, addr) = match spec.split_once('=') {
            Some((name, addr)) => (
                rt.find_area(name)
                    .ok_or_else(|| format!("--node {spec}: Stellbereich „{name}“ unbekannt"))?,
                addr,
            ),
            None if rt.area_count() == 1 => (0, spec.as_str()),
            None => {
                return Err(format!(
                    "--node {spec}: bei mehreren Stellbereichen BEREICH=ADRESSE angeben"
                ));
            }
        };
        let addr: SocketAddr = addr
            .parse()
            .map_err(|e| format!("--node {spec}: Adresse: {e}"))?;
        let link =
            railit_runtime::UdpLink::connect(addr).map_err(|e| format!("--node {spec}: {e}"))?;
        rt.attach_node(area, Box::new(link))?;
        tracing::info!(
            area = rt.area_name(area).unwrap_or("?"),
            %addr,
            "Stellbereich auf Knoten des sicheren Rechners"
        );
    }
    Ok(())
}

/// Startet die Journale: eine Datei bei einem Stellbereich, sonst ein Verzeichnis mit
/// `<Bereich>.journal` je Stellbereich.
fn start_journals(
    rt: &mut Runtime<ServiceField>,
    path: &Path,
    every: u64,
    seal: Option<(&railit_plan::SigningKey, u64)>,
) -> Result<(), String> {
    if rt.area_count() > 1 {
        std::fs::create_dir_all(path).map_err(|e| format!("Journal {}: {e}", path.display()))?;
    }
    let files = review::files(path, &area_plans(rt));
    for (area, file) in files.iter().map(|f| &f.path).enumerate() {
        let out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(file)
            .map_err(|e| format!("Journal {}: {e}", file.display()))?;
        let out = Box::new(std::io::BufWriter::new(out));
        match seal {
            Some((key, seal_every)) => {
                rt.start_journal_sealed(area, out, every, key.clone(), seal_every)
            }
            None => rt.start_journal(area, out, every),
        }
        .map_err(|e| format!("Journal {}: {e}", file.display()))?;
        tracing::info!(path = %file.display(), "Journal läuft");
    }
    Ok(())
}

/// Name und Plan je Stellbereich.
fn area_plans(rt: &Runtime<ServiceField>) -> Vec<(String, railit_plan::PlanView<'static>)> {
    (0..rt.area_count())
        .filter_map(|i| Some((rt.area_name(i)?.to_string(), *rt.plan_of(i)?)))
        .collect()
}

/// Bereitschaftsrechner (`--standby`): Mitleser je Stellbereich.
fn start_standby(
    args: &Args,
    rt: &Runtime<ServiceField>,
    snapshot: &watch::Sender<Arc<railit_runtime::Snapshot>>,
) -> Option<Arc<standby::Standby>> {
    let url = args.standby.as_deref()?;
    let areas = area_plans(rt);
    let network = areas
        .first()
        .map(|(_, p)| p.network_name().to_string())
        .unwrap_or_default();
    tracing::info!(primary = url, "Bereitschaftsrechner: liest das Journal mit");
    Some(standby::start(
        url,
        args.standby_token.as_deref(),
        network,
        areas,
        snapshot.clone(),
    ))
}

/// Vorbereitung des Bereitschaftsrechners auf die Führung: Journale beginnen, Ausgabesperre
/// für einen neuen Bereitschaftsrechner.
fn prepare(args: &Args) -> Result<takeover::Prepare, String> {
    let seal = args
        .seal_key
        .as_deref()
        .map(railit_runtime::keys::read_signing_key)
        .transpose()?;
    let journal = args.journal.clone();
    let (every, seal_every, gate) = (args.checkpoint_every, args.seal_every, args.standby_gate_ms);
    Ok(Box::new(move |rt| {
        if let Some(path) = &journal {
            start_journals(rt, path, every, seal.as_ref().map(|k| (k, seal_every)))?;
        }
        if let Some(ms) = gate {
            enable_gate(rt, ms)?;
        }
        Ok(())
    }))
}

/// Ausgabesperre für einen Bereitschaftsrechner (`--standby-gate-ms`).
fn enable_gate(rt: &mut Runtime<ServiceField>, grace_ms: u64) -> Result<(), String> {
    rt.enable_replication(railit_runtime::ReplicationConfig {
        grace_ms,
        ..railit_runtime::ReplicationConfig::default()
    })?;
    tracing::info!(
        grace_ms,
        "Ausgabe erst nach Quittung des Bereitschaftsrechners"
    );
    Ok(())
}

/// Steuerung des Feldrechners (`--field-control`).
fn field_control(args: &Args) -> Option<FieldControl> {
    args.field_control
        .as_deref()
        .map(|url| FieldControl::new(url, args.field_token.clone()))
}

/// Auditprotokoll (`--audit`), gesiegelt mit `--seal-key`.
fn open_audit(args: &Args) -> Result<Option<Arc<service::Audit>>, String> {
    let seal_key = args
        .seal_key
        .as_deref()
        .map(railit_runtime::keys::read_signing_key)
        .transpose()?;
    Ok(args
        .audit
        .as_deref()
        .map(|path| service::Audit::open(path, seal_key))
        .transpose()?
        .map(Arc::new))
}

/// Startet Engine und API und läuft bis Strg+C.
async fn serve(
    args: Args,
    net: Topology,
    rt: Runtime<ServiceField>,
    security: Security,
) -> Result<(), String> {
    let period = std::time::Duration::from_micros(
        (args.cycle_ms.max(1) * 1000 / u64::from(args.speed.max(1))).max(1000),
    );
    let (tx, rx) = mpsc::channel(256);
    let (snapshot_tx, snapshot_rx) = watch::channel(Arc::new(rt.snapshot()));
    let (events_tx, _) = broadcast::channel(4096);
    let directory = rt.directory().clone();
    let (metrics_tx, metrics_rx) = watch::channel(Arc::new(metrics::Counters::default()));
    let journals = Arc::new(
        args.journal
            .as_deref()
            .map(|path| review::files(path, &area_plans(&rt)))
            .unwrap_or_default(),
    );
    let standby = start_standby(&args, &rt, &snapshot_tx);
    let channels = engine::Channels {
        rx,
        snapshot: snapshot_tx,
        events: events_tx.clone(),
        metrics: metrics_tx,
    };
    let mut engine = match &standby {
        // Bis zu einer Übernahme scheitert jede Bedienung; das Zustandsbild speisen die
        // Mitleser.
        Some(s) => {
            let (wake_tx, wake_rx) = tokio::sync::oneshot::channel();
            if let Some(ms) = args.takeover_after_ms {
                let after = std::time::Duration::from_millis(ms);
                takeover::start(Arc::clone(s), rt, after, prepare(&args)?, wake_tx);
            }
            tokio::spawn(engine::standby(channels, wake_rx, period))
        }
        None => tokio::spawn(engine::run(rt, channels, period)),
    };

    let audit = open_audit(&args)?;
    let operators_from_users = security.auth.operators_from_users();
    let state = api::AppState {
        service: service::Service::new(
            directory,
            Arc::new(net),
            tx,
            snapshot_rx,
            events_tx,
            audit,
            operators_from_users,
        )
        .with_field_control(field_control(&args)),
        auth: Arc::new(security.auth),
        metrics: metrics_rx,
        journals,
        standby,
    };
    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .map_err(|e| format!("{}: {e}", args.listen))?;
    let addr = listener
        .local_addr()
        .map_or_else(|_| args.listen.to_string(), |a| a.to_string());
    let scheme = if security.tls.is_some() {
        "https"
    } else {
        "http"
    };
    tracing::info!(%addr, scheme, cycle_ms = args.cycle_ms, speed = args.speed, "API bereit");
    // Erste Zeile auf stdout: Adresse für Skripte und Tests.
    println!("railitd: {scheme}://{addr}");
    let _ = std::io::stdout().flush();

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("wird beendet");
    };
    let app = api::router(state);
    let served = match security.tls {
        Some(config) => {
            tls::serve(listener, config, app, shutdown).await;
            Ok(())
        }
        None => axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|e| format!("Server: {e}")),
    };
    // Mit dem Server sind alle Sender fort: Die Engine siegelt die Journale und endet.
    if tokio::time::timeout(std::time::Duration::from_secs(3), &mut engine)
        .await
        .is_err()
    {
        engine.abort();
    }
    served
}

/// Anmeldung und Transportsicherung.
struct Security {
    auth: AuthMode,
    tls: Option<Arc<rustls::ServerConfig>>,
}

/// Prüft die Sicherheitseinstellungen vor dem Start.
fn security(args: &Args, net: &Topology) -> Result<Security, String> {
    let auth = match (&args.users, &args.token) {
        (Some(path), _) => {
            let mut areas: Vec<String> = net.areas().map(|(_, a)| a.name.clone()).collect();
            if areas.is_empty() {
                areas.push(net.name().to_string());
            }
            AuthMode::Users(Users::load(path, &areas)?)
        }
        (None, Some(token)) => {
            if token.len() < 16 {
                tracing::warn!("Zugangstoken kürzer als 16 Zeichen");
            }
            AuthMode::Token(token.clone())
        }
        (None, None) => {
            if !args.listen.ip().is_loopback() {
                return Err(
                    "ohne --users oder --token lauscht der Dienst nur auf der Loopback-Adresse"
                        .into(),
                );
            }
            tracing::warn!("keine Anmeldung: jeder lokale Zugriff hat alle Rechte");
            AuthMode::Open
        }
    };
    if args.users.is_none() && args.audit.is_some() {
        tracing::info!("Auditprotokoll ohne Benutzerdatei: Benutzer sind nicht unterscheidbar");
    }
    let tls = if let (Some(cert), Some(key)) = (&args.tls_cert, &args.tls_key) {
        Some(tls::server_config(
            cert,
            key,
            args.tls_client_ca.as_deref(),
            &args.tls_crls,
        )?)
    } else {
        if !args.listen.ip().is_loopback() {
            tracing::warn!("ohne TLS: Token und Daten gehen unverschlüsselt über das Netz");
        }
        None
    };
    Ok(Security { auth, tls })
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
    if let Some(url) = &args.field_control {
        match FieldControl::new(url, args.field_token.clone()).network() {
            Ok(name) if name == net.name() => {}
            Ok(name) => {
                return fail(&format!(
                    "Feldrechner simuliert „{name}“, nicht „{}“",
                    net.name()
                ));
            }
            Err(e) => return fail(&e),
        }
    }
    let seal_key = match args
        .seal_key
        .as_deref()
        .map(railit_runtime::keys::read_signing_key)
        .transpose()
    {
        Ok(k) => k,
        Err(e) => return fail(&e),
    };
    if args.takeover_after_ms.is_some() && !matches!(args.field, FieldArg::Remote) {
        return fail("--takeover-after-ms verlangt --field remote (Umschalter im Feldrechner)");
    }
    // Der Bereitschaftsrechner beginnt seine Journale erst bei der Übernahme.
    if args.standby.is_none() {
        if let Some(path) = &args.journal
            && let Err(e) = start_journals(
                &mut rt,
                path,
                args.checkpoint_every,
                seal_key.as_ref().map(|k| (k, args.seal_every)),
            )
        {
            return fail(&e);
        }
        if let Some(ms) = args.standby_gate_ms
            && let Err(e) = enable_gate(&mut rt, ms)
        {
            return fail(&e);
        }
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => return fail(&format!("Tokio: {e}")),
    };
    let security = match security(&args, &net) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    match runtime.block_on(serve(args, net, rt, security)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}
