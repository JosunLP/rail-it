//! `railit` – Kommandozeile von rail-it.
//!
//! **Offline** (ohne Dienst):
//!
//! ```text
//! railit net validate|info NETZ.toml
//! railit plan keygen PRÄFIX
//! railit plan compile NETZ.toml --out PLAN.rplan [--key PRÄFIX.key]
//! railit plan verify PLAN.rplan [--key PRÄFIX.pub] [--network NETZ.toml]
//! railit plan info PLAN.rplan
//! railit scenario run SZENARIO.toml…
//! railit journal verify JOURNAL
//! railit journal replay JOURNAL --plan PLAN.rplan
//! ```
//!
//! **Online** (Client von `railitd`, Adresse über `--server` bzw. `RAILIT_SERVER`):
//!
//! ```text
//! railit state
//! railit route list|set|cancel [NAME]
//! railit point move NAME left|right
//! railit sim train add|list|remove …
//! railit sim fault set|clear ART ELEMENT
//! railit aux list
//! railit aux request ART ELEMENT [--position left|right] --operator NR
//! railit aux confirm ID --operator NR
//! railit watch
//! ```
//!
//! Exit-Codes: `0` Erfolg, `1` ungültig bzw. abgelehnt, `2` Bedien-, Lese- oder
//! Verbindungsfehler.

mod journal;
mod net;
mod online;
mod plan;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};

#[derive(Debug, Parser)]
#[command(
    name = "railit",
    version,
    about = "Netz- und Stellwerkmanagement für Bahnsysteme"
)]
struct Cli {
    /// Ausgabeformat.
    #[arg(long, short, global = true, value_enum, default_value_t = Output::Text)]
    output: Output,

    /// Adresse von railitd (Online-Befehle).
    #[arg(
        long,
        global = true,
        env = "RAILIT_SERVER",
        default_value = "http://127.0.0.1:7878"
    )]
    server: String,

    /// Zugangstoken für railitd.
    #[arg(long, global = true, env = "RAILIT_TOKEN", hide_env_values = true)]
    token: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Output {
    /// Lesbarer Text.
    Text,
    /// JSON für Skripte und andere Werkzeuge.
    Json,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Netzbeschreibungen prüfen und auswerten.
    #[command(subcommand)]
    Net(NetCommand),
    /// Projektierung: Schlüssel, Plan-Images erzeugen und prüfen.
    #[command(subcommand)]
    Plan(PlanCommand),
    /// Betriebsszenarien ausführen.
    #[command(subcommand)]
    Scenario(ScenarioCommand),
    /// Betriebsjournal prüfen und abspielen.
    #[command(subcommand)]
    Journal(JournalCommand),
    /// Zustandsbild des Stellwerks (online).
    State,
    /// Fahrstraßen (online).
    #[command(subcommand)]
    Route(RouteCommand),
    /// Weichen (online).
    #[command(subcommand)]
    Point(PointCommand),
    /// Simulation steuern (online).
    #[command(subcommand)]
    Sim(SimCommand),
    /// Zählpflichtige Hilfshandlungen mit Zwei-Personen-Regel (online).
    #[command(subcommand)]
    Aux(AuxCommand),
    /// Ereignisse fortlaufend ausgeben (online).
    Watch,
}

#[derive(Debug, Subcommand)]
enum NetCommand {
    /// Prüft eine Netzbeschreibung vollständig und meldet alle Fehler.
    Validate {
        /// Netzbeschreibung im rail-it-TOML-Format.
        file: PathBuf,
    },
    /// Beschreibt ein gültiges Netz: Elemente, Signale, Gleisfreimeldeabschnitte.
    Info {
        /// Netzbeschreibung im rail-it-TOML-Format.
        file: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum PlanCommand {
    /// Erzeugt ein Schlüsselpaar PRÄFIX.key (geheim) und PRÄFIX.pub (öffentlich).
    Keygen {
        /// Dateipräfix.
        prefix: PathBuf,
    },
    /// Erzeugt Fahrstraßen, prüft sie unabhängig und schreibt das Plan-Image.
    Compile {
        /// Netzbeschreibung.
        network: PathBuf,
        /// Ausgabedatei.
        #[arg(short = 'O', long = "out")]
        out: PathBuf,
        /// Geheimer Schlüssel zum Signieren.
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Prüft ein Plan-Image (Struktur, Prüfsumme, Signatur, optional gegen ein Netz).
    Verify {
        /// Plan-Image.
        plan: PathBuf,
        /// Öffentlicher Schlüssel; dann muss der Plan signiert sein.
        #[arg(long)]
        key: Option<PathBuf>,
        /// Netz für die unabhängige Gegenprüfung.
        #[arg(long)]
        network: Option<PathBuf>,
    },
    /// Listet die Fahrstraßen eines Plan-Images.
    Info {
        /// Plan-Image.
        plan: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum ScenarioCommand {
    /// Führt Szenariodateien aus.
    Run {
        /// Szenarien (TOML).
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum JournalCommand {
    /// Prüft die Hashkette eines Journals.
    Verify {
        /// Journaldatei (`railitd --journal`).
        file: PathBuf,
    },
    /// Spielt ein Journal gegen das Plan-Image ab und vergleicht Ausgaben und Prüfpunkte.
    Replay {
        /// Journaldatei.
        file: PathBuf,
        /// Plan-Image, mit dem das Journal aufgezeichnet wurde.
        #[arg(long)]
        plan: PathBuf,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum AuxKind {
    /// Fahrstraßenhilfsauflösung (Element: Fahrstraße).
    ReleaseRoute,
    /// Achszählgrundstellung (Element: Abschnitt).
    ResetTvd,
    /// Ersatzsignal Zs1 (Element: Fahrstraße).
    Substitute,
    /// Weiche im gestörten Abschnitt umstellen (Element: Weiche, mit --position).
    ForcePoint,
}

impl AuxKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ReleaseRoute => "release_route",
            Self::ResetTvd => "reset_tvd",
            Self::Substitute => "substitute",
            Self::ForcePoint => "force_point",
        }
    }
}

#[derive(Debug, Subcommand)]
enum AuxCommand {
    /// Offene Hilfshandlungen und Zählwerk.
    List,
    /// Hilfshandlung anfordern (erster Bediener).
    Request {
        /// Art.
        #[arg(value_enum)]
        kind: AuxKind,
        /// Element.
        element: String,
        /// Ziellage (nur force-point).
        #[arg(long, value_enum)]
        position: Option<Position>,
        /// Bedienernummer.
        #[arg(long)]
        operator: u32,
    },
    /// Hilfshandlung bestätigen (zweiter, anderer Bediener).
    Confirm {
        /// Kennung aus der Anforderung.
        id: u32,
        /// Bedienernummer.
        #[arg(long)]
        operator: u32,
    },
}

#[derive(Debug, Subcommand)]
enum RouteCommand {
    /// Alle Fahrstraßen mit Phase.
    List,
    /// Fahrstraße einstellen.
    Set {
        /// Name, z. B. A-N1.
        name: String,
    },
    /// Fahrstraße zurücknehmen.
    Cancel {
        /// Name.
        name: String,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Position {
    Left,
    Right,
}

impl Position {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

#[derive(Debug, Subcommand)]
enum PointCommand {
    /// Einzelne Weiche umstellen.
    Move {
        /// Name, z. B. W1.
        name: String,
        /// Ziellage.
        #[arg(value_enum)]
        position: Position,
    },
}

#[derive(Debug, Subcommand)]
enum SimCommand {
    /// Züge.
    #[command(subcommand)]
    Train(TrainCommand),
    /// Störungen.
    #[command(subcommand)]
    Fault(FaultCommand),
}

#[derive(Debug, Subcommand)]
enum TrainCommand {
    /// Zug vor ein Signal setzen.
    Add {
        /// Zugnummer.
        name: String,
        /// Signal, vor dem der Zug steht.
        #[arg(long)]
        at: String,
        /// Zuglänge in Metern.
        #[arg(long, default_value_t = 120)]
        length: u32,
        /// Geschwindigkeit in km/h.
        #[arg(long, default_value_t = 60)]
        speed: u16,
        /// Halt zeigende Signale überfahren (Störungsfall).
        #[arg(long)]
        ignore_signals: bool,
    },
    /// Alle Züge.
    List,
    /// Zug entfernen.
    Remove {
        /// Zugnummer.
        name: String,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum FaultKind {
    PointDisturbed,
    TvdDisturbed,
    TvdOccupied,
    SignalDefective,
    LxDisturbed,
}

impl FaultKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PointDisturbed => "point_disturbed",
            Self::TvdDisturbed => "tvd_disturbed",
            Self::TvdOccupied => "tvd_occupied",
            Self::SignalDefective => "signal_defective",
            Self::LxDisturbed => "lx_disturbed",
        }
    }
}

#[derive(Debug, Subcommand)]
enum FaultCommand {
    /// Störung setzen.
    Set {
        /// Art.
        #[arg(value_enum)]
        kind: FaultKind,
        /// Element.
        element: String,
    },
    /// Störung aufheben.
    Clear {
        /// Art.
        #[arg(value_enum)]
        kind: FaultKind,
        /// Element.
        element: String,
    },
}

pub(crate) const EXIT_INVALID: u8 = 1;
pub(crate) const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let output = cli.output;
    match cli.command {
        Command::Net(NetCommand::Validate { file }) => net::validate(&file, output),
        Command::Net(NetCommand::Info { file }) => net::info(&file, output),
        Command::Plan(PlanCommand::Keygen { prefix }) => plan::keygen(&prefix, output),
        Command::Plan(PlanCommand::Compile { network, out, key }) => {
            plan::compile(&network, &out, key.as_deref(), output)
        }
        Command::Plan(PlanCommand::Verify { plan, key, network }) => {
            plan::verify(&plan, key.as_deref(), network.as_deref(), output)
        }
        Command::Plan(PlanCommand::Info { plan }) => plan::info(&plan, output),
        Command::Scenario(ScenarioCommand::Run { files }) => plan::run_scenarios(&files, output),
        Command::Journal(JournalCommand::Verify { file }) => journal::verify(&file, output),
        Command::Journal(JournalCommand::Replay { file, plan }) => {
            journal::replay(&file, &plan, output)
        }
        online => {
            let client = online::Client::new(&cli.server, cli.token);
            run_online(&client, online, output).unwrap_or_else(online::Failure::exit)
        }
    }
}

fn run_online(
    c: &online::Client,
    command: Command,
    output: Output,
) -> Result<ExitCode, online::Failure> {
    let path = |segment: &str| segment.replace('/', "%2F");
    match command {
        Command::State => online::state(c, output),
        Command::Route(RouteCommand::List) => online::route_list(c, output),
        Command::Route(RouteCommand::Set { name }) => online::request(
            c,
            &format!("/v1/routes/{}/set", path(&name)),
            None,
            &format!("Fahrstraße {name} einstellen"),
            output,
        ),
        Command::Route(RouteCommand::Cancel { name }) => online::request(
            c,
            &format!("/v1/routes/{}/cancel", path(&name)),
            None,
            &format!("Fahrstraße {name} zurücknehmen"),
            output,
        ),
        Command::Point(PointCommand::Move { name, position }) => {
            let pos = position.as_str();
            let label = match position {
                Position::Left => "links",
                Position::Right => "rechts",
            };
            online::request(
                c,
                &format!("/v1/points/{}/move", path(&name)),
                Some(json!({ "position": pos })),
                &format!("Weiche {name} nach {label}"),
                output,
            )
        }
        Command::Sim(SimCommand::Train(TrainCommand::Add {
            name,
            at,
            length,
            speed,
            ignore_signals,
        })) => {
            let body = json!({
                "name": name, "signal": at, "length_m": length, "speed_kmh": speed,
                "ignore_signals": ignore_signals,
            });
            online::sim_done(
                c.post("/v1/sim/trains", Some(body)),
                &format!("Zug {name} steht vor {at}."),
                output,
            )
        }
        Command::Sim(SimCommand::Train(TrainCommand::List)) => online::trains(c, output),
        Command::Sim(SimCommand::Train(TrainCommand::Remove { name })) => online::sim_done(
            c.delete(&format!("/v1/sim/trains/{}", path(&name)))
                .map(|v| (200, v)),
            &format!("Zug {name} entfernt."),
            output,
        ),
        Command::Sim(SimCommand::Fault(f)) => {
            let (kind, element, active) = match f {
                FaultCommand::Set { kind, element } => (kind, element, true),
                FaultCommand::Clear { kind, element } => (kind, element, false),
            };
            let body = json!({ "kind": kind.as_str(), "element": element, "active": active });
            let what = format!(
                "Störung {} an {element} {}.",
                kind.as_str(),
                if active { "gesetzt" } else { "aufgehoben" }
            );
            online::sim_done(c.post("/v1/sim/faults", Some(body)), &what, output)
        }
        Command::Aux(AuxCommand::List) => online::aux_list(c, output),
        Command::Aux(AuxCommand::Request {
            kind,
            element,
            position,
            operator,
        }) => {
            let mut body = json!({ "kind": kind.as_str(), "element": element });
            if let Some(p) = position {
                body["position"] = json!(p.as_str());
            }
            online::aux(c, "/v1/aux", Some(body), operator, output)
        }
        Command::Aux(AuxCommand::Confirm { id, operator }) => {
            online::aux(c, &format!("/v1/aux/{id}/confirm"), None, operator, output)
        }
        Command::Watch => online::watch(c, output),
        Command::Net(_) | Command::Plan(_) | Command::Scenario(_) | Command::Journal(_) => {
            Ok(ExitCode::SUCCESS)
        }
    }
}

pub(crate) fn fail_usage(msg: &str) -> ExitCode {
    eprintln!("Fehler: {msg}");
    ExitCode::from(EXIT_USAGE)
}

pub(crate) fn print_json(value: &Value) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("Fehler: JSON-Ausgabe fehlgeschlagen: {e}"),
    }
}
