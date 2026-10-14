//! `railit` – Kommandozeile von rail-it.
//!
//! **Offline** (ohne Dienst):
//!
//! ```text
//! railit net validate|info NETZ.toml
//! railit plan keygen PRÄFIX
//! railit plan compile NETZ.toml --out PLAN.rplan|VERZEICHNIS [--key PRÄFIX.key]
//! railit plan verify PLAN.rplan [--key PRÄFIX.pub] [--network NETZ.toml]
//! railit plan info PLAN.rplan
//! railit scenario run SZENARIO.toml…
//! railit journal verify JOURNAL
//! railit journal replay JOURNAL --plan PLAN.rplan
//! railit audit verify AUDITPROTOKOLL
//! railit auth token NAME
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
//! railit aux request ART ELEMENT [--position left|right] [--operator NR]
//! railit aux confirm ID ART ELEMENT [--position left|right] [--area BEREICH] [--operator NR]
//! railit areas
//! railit border request|withdraw BEREICH>NACHBAR
//! railit ops list|ars on|off|describe|erase|hold|release|platform|delay …
//! railit etcs
//! railit whoami
//! railit audit show [--last N]
//! railit watch
//! ```
//!
//! Mit TLS: `--server https://…`, `--ca` (Server-CA), für mTLS `--cert` und `--key`.
//!
//! Exit-Codes: `0` Erfolg, `1` ungültig bzw. abgelehnt, `2` Bedien-, Lese- oder
//! Verbindungsfehler.

mod admin;
mod bench;
mod journal;
mod net;
mod online;
mod plan;
mod selfcmd;

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

    /// CA des Serverzertifikats (PEM) für `https://`.
    #[arg(long, global = true, env = "RAILIT_CA")]
    ca: Option<PathBuf>,

    /// Client-Zertifikat (PEM) für die Anmeldung mit mTLS.
    #[arg(long, global = true, env = "RAILIT_CERT", requires = "key")]
    cert: Option<PathBuf>,

    /// Privater Schlüssel des Client-Zertifikats (PEM).
    #[arg(long, global = true, env = "RAILIT_KEY", requires = "cert")]
    key: Option<PathBuf>,

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
    /// Leistungsmessung: Kernzyklus, Zulassungsprüfung und Replay gegen die Ziele.
    Bench {
        /// Bahnhöfe des erzeugten Korridors (236 ≈ 5000 Elemente, Bezugsgröße des Konzepts).
        #[arg(long, default_value_t = 236)]
        stations: usize,
        /// Gemessene Zyklen.
        #[arg(long, default_value_t = 6000)]
        cycles: usize,
        /// Exit-Code 1, wenn ein Ziel verfehlt ist.
        #[arg(long)]
        check: bool,
    },
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
    /// Stellbereiche mit Grenzen und Erlaubnissen (online).
    Areas,
    /// Erlaubnis an einer Bereichsgrenze (online).
    #[command(subcommand)]
    Border(BorderCommand),
    /// Betriebsführung: Zugnummern, Zuglenkung, Disposition (online).
    #[command(subcommand)]
    Ops(OpsCommand),
    /// ETCS: angemeldete Züge mit Betriebsart und Fahrerlaubnis (online).
    Etcs,
    /// Sicherheitsschicht: redundante Kanäle, Voter, sicherer Zustand (online). Ohne
    /// Unterbefehl: Zustand.
    Safety {
        /// Befehl; ohne Angabe `status`.
        #[command(subcommand)]
        command: Option<SafetyCommand>,
    },
    /// Angemeldeter Benutzer (online).
    Whoami,
    /// Auditprotokoll prüfen (offline) oder anzeigen (online).
    #[command(subcommand)]
    Audit(AuditCommand),
    /// Zugangsdaten verwalten.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Installation: Version, neue Releases, aktualisieren, entfernen.
    #[command(name = "self", subcommand)]
    SelfCmd(SelfCommand),
}

#[derive(Debug, Subcommand)]
enum SelfCommand {
    /// Version und Installation.
    Info,
    /// Gibt es ein neueres Release? (Exit-Code 1, wenn ja.)
    Check,
    /// Auf das neueste (oder ein bestimmtes) Release aktualisieren.
    Update {
        /// Bestimmte Version statt der neuesten.
        #[arg(long)]
        version: Option<String>,
    },
    /// rail-it entfernen (alle Programme und Daten der Installation).
    Uninstall,
}

#[derive(Debug, Subcommand)]
enum BorderCommand {
    /// Erlaubnis anfordern.
    Request {
        /// Adresse `Bereich>Nachbar`, z. B. `Strecke>Adorf`.
        address: String,
    },
    /// Anforderung zurückziehen.
    Withdraw {
        /// Adresse `Bereich>Nachbar`.
        address: String,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Switch {
    On,
    Off,
}

#[derive(Debug, Subcommand)]
enum SafetyCommand {
    /// Redundanz, mitwählende Kanäle und Abschaltung je Stellbereich.
    Status,
    /// Fehler in einen Kanal einbringen (Prüfung; Rolle Instandhaltung).
    Fault {
        /// Kanal (0, 1, 2).
        channel: usize,
        /// Fehler.
        #[arg(value_enum)]
        fault: ChannelFault,
        /// Stellbereich (bei Netzen mit mehreren Bereichen).
        #[arg(long)]
        area: Option<String>,
    },
    /// Ausgeschlossenen Kanal wieder aufnehmen.
    Reintegrate {
        /// Kanal (0, 1, 2).
        channel: usize,
        /// Stellbereich (bei Netzen mit mehreren Bereichen).
        #[arg(long)]
        area: Option<String>,
    },
    /// Sicheren Zustand zurücksetzen: Neubeginn wie nach einem Neustart.
    Reset {
        /// Stellbereich (bei Netzen mit mehreren Bereichen).
        #[arg(long)]
        area: Option<String>,
    },
}

/// Fehler für einen Kanal.
#[derive(Copy, Clone, Debug, ValueEnum)]
enum ChannelFault {
    /// Eingaben eines Zyklus verlieren.
    DropInputs,
    /// Falscher Fahrtbefehl in einem Zyklus.
    StrayCommand,
    /// Kanal rechnet nicht mehr.
    Stall,
}

impl ChannelFault {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DropInputs => "drop_inputs",
            Self::StrayCommand => "stray_command",
            Self::Stall => "stall",
        }
    }
}

#[derive(Debug, Subcommand)]
enum OpsCommand {
    /// Züge mit Zugnummer, Halt und Verspätung.
    List,
    /// Zuglenkung ein- oder ausschalten.
    Ars {
        /// Ein oder aus.
        #[arg(value_enum)]
        state: Switch,
    },
    /// Zugnummer an einem Signal eintragen.
    Describe {
        /// Zugnummer.
        train: String,
        /// Signal.
        signal: String,
    },
    /// Zugnummer löschen.
    Erase {
        /// Zugnummer.
        train: String,
    },
    /// Zug von der Zuglenkung zurückhalten.
    Hold {
        /// Zugnummer.
        train: String,
    },
    /// Zurückgehaltenen Zug freigeben.
    Release {
        /// Zugnummer.
        train: String,
    },
    /// Gleiswechsel für den nächsten Halt an einer Betriebsstelle.
    Platform {
        /// Zugnummer.
        train: String,
        /// Betriebsstelle.
        station: String,
        /// Bahnsteiggleis.
        platform: String,
    },
    /// Zusätzliche Wartezeit vor den folgenden Abfahrten.
    Delay {
        /// Zugnummer.
        train: String,
        /// Sekunden.
        seconds: u32,
    },
}

#[derive(Debug, Subcommand)]
enum AuditCommand {
    /// Prüft Hashkette und Siegel eines Auditprotokolls (offline).
    Verify {
        /// Auditprotokoll (`railitd --audit`).
        file: PathBuf,
        /// Öffentlicher Schlüssel des Stellwerks: Siegel müssen damit erstellt sein.
        #[arg(long)]
        key: Option<PathBuf>,
        /// Ungültig, wenn kein Siegel vorhanden ist.
        #[arg(long)]
        require_seal: bool,
    },
    /// Letzte Einträge vom Dienst (online; Rolle mit Auditrecht).
    Show {
        /// Anzahl.
        #[arg(long, default_value_t = 20)]
        last: usize,
    },
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// Erzeugt ein Zugangstoken und den Eintrag für die Benutzerdatei.
    Token {
        /// Benutzername.
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum NetCommand {
    /// Prüft eine Netzbeschreibung vollständig und meldet alle Fehler.
    Validate {
        /// Netzbeschreibung: rail-it-TOML oder railML 3 (`.xml`, `.railml`).
        file: PathBuf,
    },
    /// Beschreibt ein gültiges Netz: Elemente, Signale, Gleisfreimeldeabschnitte.
    Info {
        /// Netzbeschreibung: rail-it-TOML oder railML 3 (`.xml`, `.railml`).
        file: PathBuf,
    },
    /// Spurplan eines Netzes als SVG (mit `--output json` die Geometrie als JSON).
    Layout {
        /// Netzbeschreibung: rail-it-TOML oder railML 3.
        file: PathBuf,
        /// Zieldatei; ohne Angabe Standardausgabe.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Erzeugt einen Korridor mit vielen Bahnhöfen (Leistungsmessung, Lasttests).
    Generate {
        /// Anzahl der Bahnhöfe.
        #[arg(long, default_value_t = 10)]
        stations: usize,
        /// Jeder wievielte Streckenabschnitt einen Bahnübergang hat (0 = keiner).
        #[arg(long, default_value_t = 5)]
        lx_every: usize,
        /// Zieldatei; ohne Angabe Standardausgabe.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Schreibt ein gültiges Netz als railML 3.
    Export {
        /// Netzbeschreibung: rail-it-TOML oder railML 3.
        file: PathBuf,
        /// Zieldatei; ohne Angabe Standardausgabe.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum PlanCommand {
    /// Erzeugt ein Schlüsselpaar PRÄFIX.key (geheim) und PRÄFIX.pub (öffentlich).
    Keygen {
        /// Dateipräfix.
        prefix: PathBuf,
    },
    /// Erzeugt Fahrstraßen, prüft sie unabhängig und schreibt das Plan-Image (bei Netzen
    /// mit Stellbereichen je Bereich `<Bereich>.rplan` in das Verzeichnis `--out`).
    Compile {
        /// Netzbeschreibung.
        network: PathBuf,
        /// Ausgabedatei bzw. -verzeichnis.
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
        /// Jeden Stellbereich auf einem Knoten des sicheren Rechners rechnen lassen (im
        /// Gleichschritt, über RaSTA mit Spiegel und Zustandsabgleich).
        #[arg(long)]
        node: bool,
        /// Kanäle je Stellbereich für Szenarien ohne eigene Angabe (1, 2 oder 3).
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=3))]
        channels: Option<u8>,
        /// Jeden Stellbereich im Journal mitschreiben und am Ende abspielen; das Replay muss
        /// genau den Endzustand ergeben.
        #[arg(long)]
        journal: bool,
    },
}

#[derive(Debug, Subcommand)]
enum JournalCommand {
    /// Prüft Hashkette und Siegel eines Journals.
    Verify {
        /// Journaldatei (`railitd --journal`).
        file: PathBuf,
        /// Öffentlicher Schlüssel des Stellwerks: Siegel müssen damit erstellt sein.
        #[arg(long)]
        key: Option<PathBuf>,
        /// Ungültig, wenn kein Siegel vorhanden ist.
        #[arg(long)]
        require_seal: bool,
    },
    /// Spielt ein Journal gegen das Plan-Image ab und vergleicht Ausgaben und Prüfpunkte.
    Replay {
        /// Journaldatei.
        file: PathBuf,
        /// Plan-Image, mit dem das Journal aufgezeichnet wurde.
        #[arg(long)]
        plan: PathBuf,
    },
    /// Liest ein wachsendes Journal mit: prüft und rechnet jede neue Zeile sofort nach und
    /// meldet laufend den Stand; endet bei der ersten Abweichung mit Exit-Code 1.
    Follow {
        /// Journaldatei.
        file: PathBuf,
        /// Plan-Image, mit dem das Journal aufgezeichnet wird.
        #[arg(long)]
        plan: PathBuf,
        /// Öffentlicher Schlüssel des Stellwerks: Siegel müssen damit erstellt sein.
        #[arg(long)]
        key: Option<PathBuf>,
        /// Nur den vorhandenen Inhalt prüfen und enden.
        #[arg(long)]
        once: bool,
        /// Abstand der Prüfungen auf neue Zeilen in Millisekunden.
        #[arg(long, default_value_t = 200)]
        poll_ms: u64,
    },
    /// Zustand zu einem Zeitpunkt, aus dem Journal rekonstruiert (ab dem letzten
    /// Zustandsabzug davor).
    State {
        /// Journaldatei.
        file: PathBuf,
        /// Plan-Image, mit dem das Journal aufgezeichnet wurde.
        #[arg(long)]
        plan: PathBuf,
        /// Zeitpunkt in Sekunden seit Beginn des Journals.
        #[arg(long)]
        at: f64,
    },
    /// Stellbefehle und Ereignisse eines Zeitfensters, neu berechnet und gegen das Journal
    /// geprüft.
    Events {
        /// Journaldatei.
        file: PathBuf,
        /// Plan-Image, mit dem das Journal aufgezeichnet wurde.
        #[arg(long)]
        plan: PathBuf,
        /// Beginn in Sekunden.
        #[arg(long, default_value_t = 0.0)]
        from: f64,
        /// Ende in Sekunden (ohne Angabe: bis zum Ende).
        #[arg(long)]
        to: Option<f64>,
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
        /// Bedienernummer (nur ohne Benutzerverwaltung des Dienstes).
        #[arg(long)]
        operator: Option<u32>,
    },
    /// Hilfshandlung bestätigen (zweiter, anderer Bediener). Der zweite Bediener nennt die
    /// Hilfshandlung noch einmal; der Kern führt sie nur aus, wenn sie der angeforderten
    /// entspricht.
    Confirm {
        /// Kennung aus der Anforderung.
        id: u32,
        /// Art, wie der zweite Bediener sie nennt.
        #[arg(value_enum)]
        kind: AuxKind,
        /// Element, wie der zweite Bediener es nennt.
        element: String,
        /// Ziellage (nur force-point).
        #[arg(long, value_enum)]
        position: Option<Position>,
        /// Stellbereich (bei mehreren Bereichen).
        #[arg(long)]
        area: Option<String>,
        /// Bedienernummer (nur ohne Benutzerverwaltung des Dienstes).
        #[arg(long)]
        operator: Option<u32>,
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
        /// Zug mit ETCS-Fahrzeuggerät (fährt nach der Fahrerlaubnis des RBC).
        #[arg(long)]
        etcs: bool,
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
        Command::Net(NetCommand::Export { file, out }) => net::export(&file, out.as_deref()),
        Command::Net(NetCommand::Generate {
            stations,
            lx_every,
            out,
        }) => net::generate(stations, lx_every, out.as_deref()),
        Command::Bench {
            stations,
            cycles,
            check,
        } => bench::bench(stations, cycles, check, output),
        Command::Net(NetCommand::Layout { file, out }) => {
            net::layout(&file, out.as_deref(), output)
        }
        Command::Plan(PlanCommand::Keygen { prefix }) => plan::keygen(&prefix, output),
        Command::Plan(PlanCommand::Compile { network, out, key }) => {
            plan::compile(&network, &out, key.as_deref(), output)
        }
        Command::Plan(PlanCommand::Verify { plan, key, network }) => {
            plan::verify(&plan, key.as_deref(), network.as_deref(), output)
        }
        Command::Plan(PlanCommand::Info { plan }) => plan::info(&plan, output),
        Command::Scenario(ScenarioCommand::Run {
            files,
            node,
            channels,
            journal,
        }) => plan::run_scenarios(&files, node, channels, journal, output),
        Command::Journal(JournalCommand::Verify {
            file,
            key,
            require_seal,
        }) => journal::verify(&file, key.as_deref(), require_seal, output),
        Command::Journal(JournalCommand::Follow {
            file,
            plan,
            key,
            once,
            poll_ms,
        }) => journal::follow(&file, &plan, key.as_deref(), once, poll_ms, output),
        Command::Journal(JournalCommand::State { file, plan, at }) => {
            journal::state(&file, &plan, at, output)
        }
        Command::Journal(JournalCommand::Events {
            file,
            plan,
            from,
            to,
        }) => journal::events(&file, &plan, from, to, output),
        Command::Journal(JournalCommand::Replay { file, plan }) => {
            journal::replay(&file, &plan, output)
        }
        Command::Audit(AuditCommand::Verify {
            file,
            key,
            require_seal,
        }) => admin::audit_verify(&file, key.as_deref(), require_seal, output),
        Command::Auth(AuthCommand::Token { name }) => admin::token(&name, output),
        Command::SelfCmd(SelfCommand::Info) => selfcmd::info(output),
        Command::SelfCmd(SelfCommand::Check) => selfcmd::check(output),
        Command::SelfCmd(SelfCommand::Update { version }) => selfcmd::update(version.as_deref()),
        Command::SelfCmd(SelfCommand::Uninstall) => selfcmd::uninstall(),
        online => {
            let tls = online::TlsFiles {
                ca: cli.ca,
                cert: cli.cert,
                key: cli.key,
            };
            online::Client::new(&cli.server, cli.token, &tls)
                .and_then(|client| run_online(&client, online, output))
                .unwrap_or_else(online::Failure::exit)
        }
    }
}

fn run_online(
    c: &online::Client,
    command: Command,
    output: Output,
) -> Result<ExitCode, online::Failure> {
    let path = online::segment;
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
        Command::Sim(cmd) => run_sim(c, cmd, output),
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
        Command::Aux(AuxCommand::Confirm {
            id,
            kind,
            element,
            position,
            area,
            operator,
        }) => {
            let mut restated = json!({ "kind": kind.as_str(), "element": element });
            if let Some(p) = position {
                restated["position"] = json!(p.as_str());
            }
            online::aux_confirm(c, id, area.as_deref(), restated, operator, output)
        }
        Command::Watch => online::watch(c, output),
        Command::Areas => online::areas(c, output),
        Command::Whoami => online::whoami(c, output),
        Command::Etcs => online::etcs(c, output),
        Command::Safety { command } => {
            run_safety(c, command.unwrap_or(SafetyCommand::Status), output)
        }
        Command::Border(b) => {
            let (address, verb, what) = match b {
                BorderCommand::Request { address } => (address, "request", "anfordern"),
                BorderCommand::Withdraw { address } => (address, "withdraw", "zurückziehen"),
            };
            online::request(
                c,
                &format!("/v1/borders/{}/{verb}", path(&address)),
                None,
                &format!("Erlaubnis {address} {what}"),
                output,
            )
        }
        Command::Ops(cmd) => run_ops(c, cmd, output),
        Command::Audit(AuditCommand::Show { last }) => online::audit_show(c, last, output),
        Command::Net(_)
        | Command::Plan(_)
        | Command::Scenario(_)
        | Command::Bench { .. }
        | Command::Journal(_)
        | Command::Audit(AuditCommand::Verify { .. })
        | Command::Auth(_)
        | Command::SelfCmd(_) => Ok(ExitCode::SUCCESS),
    }
}

/// Simulationsbefehle.
fn run_sim(
    c: &online::Client,
    command: SimCommand,
    output: Output,
) -> Result<ExitCode, online::Failure> {
    let path = online::segment;
    match command {
        SimCommand::Train(TrainCommand::Add {
            name,
            at,
            length,
            speed,
            ignore_signals,
            etcs,
        }) => {
            let body = json!({
                "name": name, "signal": at, "length_m": length, "speed_kmh": speed,
                "ignore_signals": ignore_signals, "etcs": etcs,
            });
            online::sim_done(
                c.post("/v1/sim/trains", Some(body)),
                &format!("Zug {name} steht vor {at}."),
                output,
            )
        }
        SimCommand::Train(TrainCommand::List) => online::trains(c, output),
        SimCommand::Train(TrainCommand::Remove { name }) => online::sim_done(
            c.delete(&format!("/v1/sim/trains/{}", path(&name)))
                .map(|v| (200, v)),
            &format!("Zug {name} entfernt."),
            output,
        ),
        SimCommand::Fault(f) => {
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
    }
}

/// Befehle der Betriebsführung.
fn run_ops(
    c: &online::Client,
    cmd: OpsCommand,
    output: Output,
) -> Result<ExitCode, online::Failure> {
    let train = |t: &str, action: &str| format!("/v1/ops/trains/{}/{action}", online::segment(t));
    let (result, what) = match cmd {
        OpsCommand::List => return online::ops_list(c, output),
        OpsCommand::Ars { state } => {
            let on = matches!(state, Switch::On);
            (
                c.post("/v1/ops/ars", Some(json!({ "on": on }))),
                format!("Zuglenkung {}.", if on { "ein" } else { "aus" }),
            )
        }
        OpsCommand::Describe { train: t, signal } => (
            c.post(&train(&t, "describe"), Some(json!({ "signal": signal }))),
            format!("Zugnummer {t} an {signal} eingetragen."),
        ),
        OpsCommand::Erase { train: t } => (
            c.delete(&format!("/v1/ops/trains/{}", online::segment(&t)))
                .map(|v| (200, v)),
            format!("Zugnummer {t} gelöscht."),
        ),
        OpsCommand::Hold { train: t } => (
            c.post(&train(&t, "hold"), None),
            format!("Zug {t} zurückgehalten."),
        ),
        OpsCommand::Release { train: t } => (
            c.post(&train(&t, "release"), None),
            format!("Zug {t} freigegeben."),
        ),
        OpsCommand::Platform {
            train: t,
            station,
            platform,
        } => (
            c.post(
                &train(&t, "platform"),
                Some(json!({ "station": station, "platform": platform })),
            ),
            format!("Zug {t} hält in {station} an Gleis {platform}."),
        ),
        OpsCommand::Delay { train: t, seconds } => (
            c.post(&train(&t, "delay"), Some(json!({ "s": seconds }))),
            format!("Zug {t} wartet zusätzlich {seconds} s."),
        ),
    };
    online::done(result, &what, output)
}

/// Befehle an die Sicherheitsschicht.
fn run_safety(
    c: &online::Client,
    cmd: SafetyCommand,
    output: Output,
) -> Result<ExitCode, online::Failure> {
    let with_area = |mut body: Value, area: Option<String>| {
        if let Some(a) = area {
            body["area"] = json!(a);
        }
        body
    };
    let (result, what) = match cmd {
        SafetyCommand::Status => return online::safety(c, output),
        SafetyCommand::Fault {
            channel,
            fault,
            area,
        } => (
            c.post(
                "/v1/safety/faults",
                Some(with_area(
                    json!({ "channel": channel, "fault": fault.as_str() }),
                    area,
                )),
            ),
            format!("Fehler {} in Kanal {channel} eingebracht.", fault.as_str()),
        ),
        SafetyCommand::Reintegrate { channel, area } => (
            c.post(
                "/v1/safety/reintegrate",
                Some(with_area(json!({ "channel": channel }), area)),
            ),
            format!("Kanal {channel} wieder aufgenommen."),
        ),
        SafetyCommand::Reset { area } => (
            c.post("/v1/safety/reset", Some(with_area(json!({}), area))),
            "Sicherer Zustand zurückgesetzt; Neubeginn wie nach einem Neustart.".to_string(),
        ),
    };
    online::done(result, &what, output)
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
