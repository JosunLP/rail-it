//! Deklarative Betriebsszenarien.
//!
//! Ein Szenario beschreibt in TOML, was im Betrieb passiert und was danach gelten muss. Es
//! läuft deterministisch mit Kern und Simulator in simulierter Zeit.
//!
//! ```toml
//! [scenario]
//! name = "Einfahrt Gleis 1"
//! network = "../../examples/beispielstadt.toml"   # relativ zur Szenariodatei
//! cycle_ms = 100                                  # optional
//!
//! [[step]]
//! note = "Fahrstraße einstellen"
//! set_route = "A-N1"
//! until = { route = { "A-N1" = "proceed" } }       # so lange laufen, bis das gilt
//! timeout_s = 30
//! expect = { accepted = true, signal = { A = "proceed" } }
//!
//! [[step]]
//! add_train = { name = "RE1", signal = "A", length_m = 120, speed_kmh = 60 }
//! wait_s = 20                                      # feste Zeit laufen
//! expect = { tvd = { T1 = "occupied" }, train = { RE1 = "running" } }
//! ```
//!
//! **Aktionen:** `set_route`, `cancel_route`, `move_point = { point, position }`,
//! `add_train = { name, signal, length_m, speed_kmh, ignore_signals, etcs }`,
//! `fault` / `clear_fault = { kind, element }` mit `kind` aus `point_disturbed`,
//! `tvd_disturbed`, `tvd_occupied`, `signal_defective`, `lx_disturbed`,
//! `aux = { kind, element, position, operator }` (Hilfshandlung anfordern; `kind` aus
//! `release_route`, `reset_tvd`, `substitute`, `force_point`),
//! `confirm_aux = { operator }` (zuletzt angeforderte Hilfshandlung bestätigen; mit `kind`,
//! `element`, `position` nennt der zweite Bediener sie selbst – weicht sie ab, lehnt der Kern
//! mit `aux_mismatch` ab und die Hilfshandlung verfällt).
//!
//! Betriebsführung: `[scenario] timetable = "…"` (Fahrplan, relativ zur Szenariodatei),
//! `ars = true` (Zuglenkung) oder `ops = true` (nur Zugnummernmeldung und Selbstblock)
//! schalten sie ein. Aktionen dazu: `describe = { train, signal }`, `erase = "RE1"`,
//! `hold = "RE1"`, `release = "RE1"`, `platform = { train, station, platform }`,
//! `delay = { train, s }`, `set_ars = true|false`. Mit eingeschalteter Betriebsführung
//! trägt `add_train` die Zugnummer am Signal ein.
//!
//! **Bedingungen** (`until`, `expect`): `route`, `signal` (befohlener Begriff),
//! `substitute` (Ersatzsignal ein/aus), `picture` (Signalbild, z. B. `"Ks1 Zs3(4)"`), `point`, `tvd` (Meldung des Felds), `lx` (Meldung
//! des Bahnübergangs), `train`, `accepted`, `rejected` (Grund), `alarm` / `no_alarm`
//! (Listen), `sim_event` (Liste), `aux_counter`, `safe_state`, `permission` (Grenze →
//! Erlaubnis gehalten), `berth` (Zugnummer → Signal, `"left"` nach Verlassen des Netzes),
//! `at_stop` (Zugnummer → `"Betriebsstelle/Gleis"`), `ops_event` (Liste der Ereignisarten
//! der Betriebsführung), `etcs` (Zug → Betriebsart des Fahrzeuggeräts: `stand_by`,
//! `full_supervision`, `trip`), `etcs_end` (Zug → Ende der Fahrerlaubnis, z. B. Signalname
//! oder `"Netzgrenze"`), `etcs_event` (Liste der Ereignisarten des RBC), `link_up`
//! (Objektsteuerung → Verbindung steht).
//!
//! Feldanbindung: `[scenario] field = "eulynx"` führt alle Stellbefehle und Meldungen über
//! EULYNX SCI und RaSTA zu Objektsteuerungen vor dem Simulator. Die Aktion
//! `link = { controller = "OC-Weichen", down = true, loss_percent, corrupt_percent, delay_ms }`
//! stört die Übertragungsstrecke.
//!
//! Nachbarstellwerke: `[scenario] borders = "ils"` überträgt die Grenzmeldungen zwischen den
//! Stellbereichen über EULYNX SCI-ILS und RaSTA (eine Verbindung je Paar benachbarter
//! Bereiche, Name `A-B`). Die Aktion
//! `ils_link = { link = "Adorf-Strecke", down = true, loss_percent, corrupt_percent, delay_ms }`
//! stört die Strecke; die Bedingung `ils_up = { "Adorf-Strecke" = true }` prüft die
//! Verbindung.
//!
//! Redundanz: `[scenario] channels = 2` (2-von-2) oder `3` (2-von-3) lässt jeden Stellbereich
//! über die Sicherheitsschicht mit so vielen Kanälen laufen. Aktionen:
//! `channel_fault = { channel = 2, fault = "stall" }` (`drop_inputs`, `stray_command`,
//! `stall`; optional `area`), `reintegrate = { channel = 2 }` (optional `area`),
//! `reset_safety = true` (alle Bereiche) bzw. `reset_safety = "Adorf"`. Bedingungen:
//! `safety_trip` (eine Sicherheitsschicht ist abgeschaltet), `safety_event` (Liste:
//! `channel_excluded`, `tripped`), `active_channels` (mitwählende Kanäle des ersten bzw.
//! einzigen Bereichs, z. B. `[true, true, false]`).
//!
//! Sicherer Rechner: `[scenario] node = true` lässt jeden Stellbereich auf einem Knoten
//! (`railit-embedded`) im Gleichschritt rechnen, angebunden über RaSTA mit Spiegel und
//! Zustandsabgleich (siehe [`crate::remote`]). Vor dem ersten Schritt wartet das Szenario,
//! bis alle Spiegel mit ihren Knoten abgeglichen sind.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use railit_kernel::{
    Aspect, AuxId, LxReport, Occupancy, OperatorId, PointReport, Request, RequestId, RoutePhase,
};
use railit_rules::GenericRules;
use railit_sim::{Fault, SimConfig, SimEvent, TrainSpec, TrainState};
use railit_topology::ElementRef;
use railit_types::{Duration, Length, PointPosition, Speed};
use serde::Deserialize;

use railit_adapters::{EulynxField, LinkFaults};
use railit_ops::{OpsConfig, OpsEvent, Timetable};

use railit_safety::{Fault as ChannelFault, Redundancy, SafetyEvent};

use crate::field::{FieldAdapter, SimulatedField};
use crate::names::{NamedAux, NamedEvent};
use crate::runtime::{Addressed, AreaSafetyEvent, Runtime, SafetyConfig};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioFile {
    scenario: Header,
    #[serde(default)]
    step: Vec<Step>,
}

// Die Schalter des Szenarioformats sind voneinander unabhängig.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    name: String,
    #[serde(default)]
    description: Option<String>,
    network: PathBuf,
    #[serde(default = "default_cycle_ms")]
    cycle_ms: u64,
    #[serde(default)]
    point_move_time_ms: Option<u64>,
    #[serde(default)]
    lx_close_time_ms: Option<u64>,
    #[serde(default)]
    points: BTreeMap<String, PointPosition>,
    #[serde(default)]
    rules: RulesEntry,
    #[serde(default)]
    timetable: Option<PathBuf>,
    #[serde(default)]
    field: FieldKind,
    #[serde(default)]
    ars: bool,
    #[serde(default)]
    ops: bool,
    #[serde(default = "default_channels")]
    channels: usize,
    #[serde(default)]
    node: bool,
    #[serde(default)]
    borders: BorderKind,
    /// Jeden Bereich mitschreiben und am Ende abspielen (nur über [`RunOptions`]).
    #[serde(skip)]
    journal: bool,
}

/// Vorgaben für Szenarien ohne eigene Angabe (z. B. um alle Szenarien auch mit 2-von-3 oder
/// über Knoten laufen zu lassen).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct RunOptions {
    /// Redundanz.
    pub channels: Option<Redundancy>,
    /// Alle Bereiche auf Knoten im Gleichschritt.
    pub node: bool,
    /// Grenzmeldungen aller Szenarien über SCI-ILS und RaSTA.
    pub ils: bool,
    /// Jeden Bereich im Journal mitschreiben (mit häufigen Zustandsabzügen) und am Ende
    /// abspielen: Das Replay muss genau zum Endzustand jedes Bereichs führen (REQ-JRN-002,
    /// REQ-JRN-006, REQ-JRN-007).
    pub journal: bool,
}

/// Abstand der Zustandsabzüge, wenn Szenarien mitgeschrieben werden.
const SCENARIO_SNAPSHOT_EVERY: u64 = 40;

/// Journal im Speicher.
#[derive(Clone, Default)]
struct MemoryJournal(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for MemoryJournal {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("Journal gesperrt"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Höchstzahl der Zyklen für den Anlauf der Knoten.
const NODE_STARTUP_CYCLES: u32 = 200;

const fn default_channels() -> usize {
    1
}

// Die Einheit gehört zum Feldnamen im Szenarioformat.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesEntry {
    point_move_timeout_ms: Option<u64>,
    report_timeout_ms: Option<u64>,
    cancel_time_lock_ms: Option<u64>,
    overlap_release_time_ms: Option<u64>,
    aux_confirm_timeout_ms: Option<u64>,
    substitute_duration_ms: Option<u64>,
    border_settle_time_ms: Option<u64>,
}

impl RulesEntry {
    fn apply(&self, mut rules: GenericRules) -> GenericRules {
        let set = |target: &mut Duration, ms: Option<u64>| {
            if let Some(ms) = ms {
                *target = Duration::from_millis(ms);
            }
        };
        set(&mut rules.point_move_timeout, self.point_move_timeout_ms);
        set(&mut rules.report_timeout, self.report_timeout_ms);
        set(&mut rules.cancel_time_lock, self.cancel_time_lock_ms);
        set(
            &mut rules.overlap_release_time,
            self.overlap_release_time_ms,
        );
        set(&mut rules.aux_confirm_timeout, self.aux_confirm_timeout_ms);
        set(&mut rules.substitute_duration, self.substitute_duration_ms);
        set(&mut rules.border_settle_time, self.border_settle_time_ms);
        rules
    }
}

const fn default_cycle_ms() -> u64 {
    100
}

const fn default_timeout_s() -> f64 {
    300.0
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    set_route: Option<String>,
    #[serde(default)]
    cancel_route: Option<String>,
    #[serde(default)]
    move_point: Option<MovePoint>,
    #[serde(default)]
    request_permission: Option<String>,
    #[serde(default)]
    add_train: Option<TrainEntry>,
    #[serde(default)]
    fault: Option<FaultEntry>,
    #[serde(default)]
    clear_fault: Option<FaultEntry>,
    #[serde(default)]
    aux: Option<AuxEntry>,
    #[serde(default)]
    confirm_aux: Option<ConfirmEntry>,
    #[serde(default)]
    describe: Option<DescribeEntry>,
    #[serde(default)]
    erase: Option<String>,
    #[serde(default)]
    hold: Option<String>,
    #[serde(default)]
    release: Option<String>,
    #[serde(default)]
    platform: Option<PlatformEntry>,
    #[serde(default)]
    delay: Option<DelayEntry>,
    #[serde(default)]
    set_ars: Option<bool>,
    #[serde(default)]
    link: Option<LinkEntry>,
    #[serde(default)]
    ils_link: Option<IlsLinkEntry>,
    #[serde(default)]
    channel_fault: Option<ChannelFaultEntry>,
    #[serde(default)]
    reintegrate: Option<ChannelEntry>,
    #[serde(default)]
    reset_safety: Option<AreaSelection>,
    #[serde(default)]
    wait_s: Option<f64>,
    #[serde(default)]
    until: Option<Conditions>,
    #[serde(default = "default_timeout_s")]
    timeout_s: f64,
    #[serde(default)]
    expect: Option<Conditions>,
}

/// Anbindung der Feldebene im Szenario.
#[derive(Copy, Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FieldKind {
    /// Simulator direkt am Kern.
    #[default]
    Direct,
    /// Über EULYNX SCI und RaSTA mit Objektsteuerungen vor dem Simulator.
    Eulynx,
}

/// Übertragung der Grenzmeldungen im Szenario.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BorderKind {
    /// Ideal im Prozess.
    #[default]
    Direct,
    /// Über EULYNX SCI-ILS und RaSTA mit störbarer Strecke.
    Ils,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelFaultEntry {
    #[serde(default)]
    area: Option<String>,
    channel: usize,
    fault: ChannelFault,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelEntry {
    #[serde(default)]
    area: Option<String>,
    channel: usize,
}

/// `true` (alle Bereiche) oder Name eines Bereichs.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AreaSelection {
    All(bool),
    Area(String),
}

// `deny_unknown_fields` verträgt sich nicht mit `flatten`.
#[derive(Debug, Deserialize)]
struct IlsLinkEntry {
    link: String,
    #[serde(flatten)]
    faults: LinkFaults,
}

// `deny_unknown_fields` verträgt sich nicht mit `flatten`.
#[derive(Debug, Deserialize)]
struct LinkEntry {
    controller: String,
    #[serde(flatten)]
    faults: LinkFaults,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MovePoint {
    point: String,
    position: PointPosition,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrainEntry {
    name: String,
    signal: String,
    length_m: f64,
    speed_kmh: u16,
    #[serde(default)]
    ignore_signals: bool,
    #[serde(default)]
    etcs: bool,
}

// `deny_unknown_fields` verträgt sich nicht mit `flatten`.
#[derive(Debug, Deserialize)]
struct AuxEntry {
    #[serde(flatten)]
    op: NamedAux,
    operator: u32,
}

// `deny_unknown_fields` verträgt sich nicht mit `flatten`.
#[derive(Debug, Deserialize)]
struct ConfirmEntry {
    operator: u32,
    /// Hilfshandlung, wie der zweite Bediener sie nennt; ohne Angabe die angeforderte.
    #[serde(flatten)]
    op: Option<NamedAux>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DescribeEntry {
    train: String,
    signal: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformEntry {
    train: String,
    station: String,
    platform: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DelayEntry {
    train: String,
    s: u32,
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FaultKind {
    PointDisturbed,
    TvdDisturbed,
    TvdOccupied,
    SignalDefective,
    LxDisturbed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FaultEntry {
    kind: FaultKind,
    element: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Conditions {
    #[serde(default)]
    route: BTreeMap<String, RoutePhase>,
    #[serde(default)]
    signal: BTreeMap<String, Aspect>,
    #[serde(default)]
    substitute: BTreeMap<String, bool>,
    #[serde(default)]
    picture: BTreeMap<String, String>,
    #[serde(default)]
    permission: BTreeMap<String, bool>,
    #[serde(default)]
    lx: BTreeMap<String, LxReport>,
    #[serde(default)]
    point: BTreeMap<String, PointReport>,
    #[serde(default)]
    tvd: BTreeMap<String, Occupancy>,
    #[serde(default)]
    train: BTreeMap<String, TrainState>,
    #[serde(default)]
    accepted: Option<bool>,
    #[serde(default)]
    rejected: Option<String>,
    #[serde(default)]
    alarm: Vec<String>,
    #[serde(default)]
    no_alarm: Vec<String>,
    #[serde(default)]
    sim_event: Vec<String>,
    #[serde(default)]
    aux_counter: Option<u64>,
    #[serde(default)]
    safe_state: Option<bool>,
    #[serde(default)]
    berth: BTreeMap<String, String>,
    #[serde(default)]
    at_stop: BTreeMap<String, String>,
    #[serde(default)]
    ops_event: Vec<String>,
    #[serde(default)]
    etcs: BTreeMap<String, String>,
    #[serde(default)]
    etcs_end: BTreeMap<String, String>,
    #[serde(default)]
    etcs_event: Vec<String>,
    #[serde(default)]
    link_up: BTreeMap<String, bool>,
    #[serde(default)]
    ils_up: BTreeMap<String, bool>,
    #[serde(default)]
    safety_trip: Option<bool>,
    #[serde(default)]
    safety_event: Vec<String>,
    #[serde(default)]
    active_channels: Option<Vec<bool>>,
}

/// Szenario konnte nicht ausgeführt werden (im Unterschied zu einem fehlgeschlagenen Schritt).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScenarioError {
    /// Datei nicht lesbar.
    Io(String),
    /// Szenario ist kein gültiges TOML oder enthält unbekannte Felder.
    Syntax(String),
    /// Netz ungültig.
    Network(String),
    /// Projektierung fehlgeschlagen.
    Planning(String),
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(m) => write!(f, "Datei: {m}"),
            Self::Syntax(m) => write!(f, "Szenario: {m}"),
            Self::Network(m) => write!(f, "Netz: {m}"),
            Self::Planning(m) => write!(f, "Projektierung: {m}"),
        }
    }
}

impl std::error::Error for ScenarioError {}

/// Ergebnis eines Schritts.
#[derive(Clone, Debug, PartialEq)]
pub struct StepOutcome {
    /// Nummer (ab 1).
    pub index: usize,
    /// Notiz aus dem Szenario.
    pub note: Option<String>,
    /// Bestanden?
    pub passed: bool,
    /// Abweichungen.
    pub failures: Vec<String>,
    /// Simulationszeit am Ende des Schritts.
    pub time_ms: u64,
}

/// Ergebnis eines Szenarios.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// Name.
    pub name: String,
    /// Beschreibung.
    pub description: Option<String>,
    /// Schritte.
    pub steps: Vec<StepOutcome>,
    /// Alle Schritte bestanden?
    pub passed: bool,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mark = if self.passed { "✔" } else { "✘" };
        writeln!(f, "{mark} {}", self.name)?;
        for s in &self.steps {
            let mark = if s.passed { "✔" } else { "✘" };
            #[allow(clippy::cast_precision_loss)]
            let t = s.time_ms as f64 / 1000.0;
            writeln!(
                f,
                "  {mark} Schritt {} (t = {t:.1} s){}",
                s.index,
                s.note
                    .as_ref()
                    .map(|n| format!(": {n}"))
                    .unwrap_or_default()
            )?;
            for msg in &s.failures {
                writeln!(f, "      {msg}")?;
            }
        }
        Ok(())
    }
}

/// Führt die Szenariodatei `path` aus.
///
/// # Errors
/// [`ScenarioError`], wenn Datei, Szenario, Netz oder Projektierung unbrauchbar sind.
pub fn run_file(path: &Path) -> Result<Outcome, ScenarioError> {
    run_file_with(path, None)
}

/// Führt die Szenariodatei `path` aus; `channels` gibt die Redundanz für Szenarien ohne
/// eigene Angabe vor (z. B. um alle Szenarien auch mit 2-von-3 laufen zu lassen).
///
/// # Errors
/// Wie [`run_file`].
pub fn run_file_with(path: &Path, channels: Option<Redundancy>) -> Result<Outcome, ScenarioError> {
    run_file_opts(
        path,
        RunOptions {
            channels,
            ..RunOptions::default()
        },
    )
}

/// Führt die Szenariodatei `path` mit Vorgaben aus.
///
/// # Errors
/// Wie [`run_file`].
pub fn run_file_opts(path: &Path, opts: RunOptions) -> Result<Outcome, ScenarioError> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| ScenarioError::Io(format!("{}: {e}", path.display())))?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    run_str_with(&source, base, opts)
}

/// Führt ein Szenario aus; relative Pfade beziehen sich auf `base`.
///
/// # Errors
/// Wie [`run_file`].
pub fn run_str(source: &str, base: &Path) -> Result<Outcome, ScenarioError> {
    run_str_with(source, base, RunOptions::default())
}

fn run_str_with(source: &str, base: &Path, opts: RunOptions) -> Result<Outcome, ScenarioError> {
    let file: ScenarioFile =
        toml::from_str(source).map_err(|e| ScenarioError::Syntax(e.to_string()))?;
    let mut header = file.scenario;
    header.node |= opts.node;
    if opts.ils {
        header.borders = BorderKind::Ils;
    }
    header.journal = opts.journal;
    if let Some(r) = opts.channels
        && header.channels == default_channels()
    {
        header.channels = r.channels();
    }
    if Redundancy::from_channels(header.channels).is_none() {
        return Err(ScenarioError::Syntax(format!(
            "channels = {}: erlaubt sind 1, 2 und 3",
            header.channels
        )));
    }

    let net_path = base.join(&header.network);
    let net_source = std::fs::read_to_string(&net_path)
        .map_err(|e| ScenarioError::Io(format!("{}: {e}", net_path.display())))?;
    let net = railit_import::parse(&header.network.to_string_lossy(), &net_source)
        .map_err(|e| ScenarioError::Network(e.to_string()))?
        .topology;
    let mut cfg = SimConfig::default();
    if let Some(ms) = header.point_move_time_ms {
        cfg.point_move_time = Duration::from_millis(ms);
    }
    if let Some(ms) = header.lx_close_time_ms {
        cfg.lx_close_time = Duration::from_millis(ms);
    }
    let rules = header.rules.apply(GenericRules::default());
    let cycle = Duration::from_millis(header.cycle_ms.max(1));
    match header.field {
        FieldKind::Direct => {
            let rt = crate::simulation(net, rules, cfg, cycle).map_err(ScenarioError::Planning)?;
            run_with(rt, header, &file.step, base)
        }
        FieldKind::Eulynx => {
            let rt = crate::simulation_with(net, rules, cfg, cycle, EulynxField::new)
                .map_err(ScenarioError::Planning)?;
            run_with(rt, header, &file.step, base)
        }
    }
}

/// Führt die Schritte mit einer fertig aufgebauten Runtime aus.
fn run_with<F: SimulatedField>(
    mut rt: Runtime<F>,
    header: Header,
    steps: &[Step],
    base: &Path,
) -> Result<Outcome, ScenarioError> {
    let redundancy = Redundancy::from_channels(header.channels).unwrap_or_default();
    rt.set_safety(SafetyConfig {
        redundancy,
        ..SafetyConfig::default()
    })
    .map_err(ScenarioError::Planning)?;
    if header.node {
        rt.use_lockstep_nodes().map_err(ScenarioError::Planning)?;
    }
    if header.borders == BorderKind::Ils {
        rt.use_ils().map_err(ScenarioError::Planning)?;
    }
    let mut journals = Vec::new();
    if header.journal {
        rt.set_journal_snapshots(SCENARIO_SNAPSHOT_EVERY);
        for area in 0..rt.area_count() {
            let buf = MemoryJournal::default();
            rt.start_journal(area, Box::new(buf.clone()), 10)
                .map_err(|e| ScenarioError::Io(format!("Journal: {e}")))?;
            journals.push(buf);
        }
    }
    for (name, pos) in &header.points {
        let Some(ElementRef::Point(id)) = rt.field().simulator().topology().lookup(name) else {
            return Err(ScenarioError::Syntax(format!("unbekannte Weiche „{name}“")));
        };
        rt.field_mut().simulator_mut().set_point_position(id, *pos);
    }
    if header.ops || header.ars || header.timetable.is_some() {
        let timetable = match &header.timetable {
            Some(path) => {
                let path = base.join(path);
                let source = std::fs::read_to_string(&path)
                    .map_err(|e| ScenarioError::Io(format!("{}: {e}", path.display())))?;
                Timetable::parse(&source)
                    .map_err(|e| ScenarioError::Syntax(format!("{}: {e}", path.display())))?
            }
            None => Timetable::default(),
        };
        let topology = rt.field().simulator().topology().clone();
        let cfg = OpsConfig {
            ars: header.ars,
            ..OpsConfig::default()
        };
        rt.enable_ops(&topology, cfg, timetable)
            .map_err(ScenarioError::Planning)?;
    }

    let mut ctx = Ctx {
        rt,
        next_request: 0,
        pending: Vec::new(),
        events: Vec::new(),
        sim_events: Vec::new(),
        ops_events: Vec::new(),
        etcs_events: Vec::new(),
        safety_events: Vec::new(),
        requests: Vec::new(),
        last_aux: None,
    };
    // Erster Zyklus: Grundstellung (alle Signale Halt, Meldungen übernommen).
    ctx.cycle();
    if header.node {
        // Anlauf: RaSTA-Verbindungen und Zustandsabgleich aller Knoten.
        let mut cycles = 0;
        while !ctx.rt.nodes_synced() {
            if cycles >= NODE_STARTUP_CYCLES {
                return Err(ScenarioError::Planning(
                    "Knoten: Anlauf ohne Zustandsabgleich".into(),
                ));
            }
            ctx.cycle();
            cycles += 1;
        }
    }

    let mut outcomes = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        outcomes.push(ctx.run_step(i.saturating_add(1), step));
    }
    if header.journal {
        outcomes.push(replay_journals(&ctx.rt, &journals, outcomes.len()));
    }
    let passed = outcomes.iter().all(|s| s.passed);
    Ok(Outcome {
        name: header.name,
        description: header.description,
        steps: outcomes,
        passed,
    })
}

/// Spielt die Journale aller Bereiche ab und vergleicht mit dem Endzustand (zusätzlicher
/// Schritt im Ergebnis).
fn replay_journals<F: FieldAdapter>(
    rt: &Runtime<F>,
    journals: &[MemoryJournal],
    steps: usize,
) -> StepOutcome {
    let mut failures = Vec::new();
    if let Some(e) = rt.journal_error() {
        failures.push(format!("Journal abgebrochen: {e}"));
    }
    for (area, buf) in journals.iter().enumerate() {
        let name = rt.area_name(area).unwrap_or("?");
        let bytes = buf.0.lock().map(|b| b.clone()).unwrap_or_default();
        let (Some(plan), Some(state)) = (rt.plan_of(area), rt.state_of(area)) else {
            failures.push(format!("Bereich {name}: unbekannt"));
            continue;
        };
        match railit_journal::replay(bytes.as_slice(), plan) {
            Ok((_, replayed)) if &replayed == state => {}
            Ok((summary, _)) => failures.push(format!(
                "Bereich {name}: Replay von {} Zyklen endet in einem anderen Zustand",
                summary.cycles
            )),
            Err(e) => failures.push(format!("Bereich {name}: Replay: {e}")),
        }
    }
    StepOutcome {
        index: steps.saturating_add(1),
        note: Some("Journal abspielen".into()),
        passed: failures.is_empty(),
        failures,
        time_ms: rt.now().as_millis(),
    }
}

struct Ctx<F> {
    rt: Runtime<F>,
    next_request: u64,
    pending: Vec<Addressed>,
    events: Vec<NamedEvent>,
    sim_events: Vec<SimEvent>,
    ops_events: Vec<OpsEvent>,
    etcs_events: Vec<railit_etcs::RbcEvent>,
    safety_events: Vec<AreaSafetyEvent>,
    requests: Vec<u64>,
    /// Zuletzt angeforderte Hilfshandlung mit Bereich (für `confirm_aux`).
    last_aux: Option<(usize, AuxId, railit_kernel::AuxOperation)>,
}

const fn sim_event_kind(e: &SimEvent) -> &'static str {
    match e {
        SimEvent::TrainStopped { .. } => "train_stopped",
        SimEvent::TrainStarted { .. } => "train_started",
        SimEvent::TrainLeft { .. } => "train_left",
        SimEvent::TrainArrived { .. } => "train_arrived",
        SimEvent::TrainDerailed { .. } => "train_derailed",
        SimEvent::PointTrailed { .. } => "point_trailed",
        SimEvent::SignalPassedAtDanger { .. } => "signal_passed_at_danger",
        SimEvent::SubstitutePassed { .. } => "substitute_passed",
        SimEvent::LevelCrossingNotClosed { .. } => "level_crossing_not_closed",
        SimEvent::EmergencyBrake { .. } => "emergency_brake",
    }
}

fn metres(m: f64) -> Option<Length> {
    if !m.is_finite() || m < 0.0 || m * 1000.0 > f64::from(u32::MAX) {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(Length::from_mm((m * 1000.0).round() as u32))
}

fn millis(s: f64) -> u64 {
    if !s.is_finite() || s <= 0.0 {
        return 0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ms = (s * 1000.0).round() as u64;
    ms
}

impl<F: SimulatedField> Ctx<F> {
    fn cycle(&mut self) {
        let out = self.rt.cycle_addressed(&self.pending);
        self.pending.clear();
        for e in &out.events {
            if let railit_kernel::Event::AuxPending { aux, op, .. } = e.event {
                self.last_aux = Some((e.area, aux, op));
            }
            self.events.push(self.rt.name_event(e));
        }
        self.sim_events
            .extend(self.rt.field_mut().simulator_mut().take_events());
        self.ops_events.extend(out.ops);
        self.etcs_events.extend(out.etcs);
        self.safety_events.extend(out.safety);
    }

    fn request(&mut self, area: usize, request: Request) {
        self.next_request = self.next_request.saturating_add(1);
        self.pending.push(Addressed {
            area,
            id: RequestId(self.next_request),
            request,
        });
        self.requests.push(self.next_request);
    }

    /// Element des Gesamtnetzes (für Simulator und Feldmeldungen).
    fn global(&self, name: &str) -> Option<ElementRef> {
        self.rt.field().simulator().topology().lookup(name)
    }

    fn run_step(&mut self, index: usize, step: &Step) -> StepOutcome {
        self.events.clear();
        self.sim_events.clear();
        self.ops_events.clear();
        self.etcs_events.clear();
        self.safety_events.clear();
        self.requests.clear();
        let mut failures = self.actions(step);
        failures.extend(self.safety_actions(step));

        let cycle_ms = self.rt.cycle_time().as_millis().max(1);
        if let Some(until) = &step.until {
            let deadline = self
                .rt
                .now()
                .as_millis()
                .saturating_add(millis(step.timeout_s));
            loop {
                self.cycle();
                if self.check(until).is_empty() {
                    break;
                }
                if self.rt.now().as_millis() >= deadline {
                    failures.push(format!("nach {} s nicht erreicht:", step.timeout_s));
                    failures.extend(self.check(until).into_iter().map(|m| format!("  {m}")));
                    break;
                }
            }
        } else {
            let cycles = millis(step.wait_s.unwrap_or(0.0)).div_ceil(cycle_ms).max(1);
            for _ in 0..cycles {
                self.cycle();
            }
        }

        if let Some(expect) = &step.expect {
            failures.extend(self.check(expect));
        }
        StepOutcome {
            index,
            note: step.note.clone(),
            passed: failures.is_empty(),
            failures,
            time_ms: self.rt.now().as_millis(),
        }
    }

    fn actions(&mut self, step: &Step) -> Vec<String> {
        let mut failures = Vec::new();
        if let Some(name) = &step.set_route {
            match self.rt.find_route(name) {
                Some((a, r)) => self.request(a, Request::SetRoute(r)),
                None => failures.push(format!("unbekannte Fahrstraße „{name}“")),
            }
        }
        if let Some(name) = &step.cancel_route {
            match self.rt.find_route(name) {
                Some((a, r)) => self.request(a, Request::CancelRoute(r)),
                None => failures.push(format!("unbekannte Fahrstraße „{name}“")),
            }
        }
        if let Some(m) = &step.move_point {
            match self.rt.find_point(&m.point) {
                Some((a, p)) => self.request(a, Request::MovePoint(p, m.position)),
                None => failures.push(format!("unbekannte Weiche „{}“", m.point)),
            }
        }
        if let Some(address) = &step.request_permission {
            match self.rt.find_border(address) {
                Some((a, b)) => self.request(a, Request::RequestPermission(b)),
                None => failures.push(format!("unbekannte Grenze „{address}“")),
            }
        }
        if let Some(t) = &step.add_train {
            failures.extend(self.add_train(t));
        }
        for (entry, on) in [(&step.fault, true), (&step.clear_fault, false)] {
            let Some(entry) = entry else { continue };
            let fault = match (entry.kind, self.global(&entry.element)) {
                (FaultKind::PointDisturbed, Some(ElementRef::Point(p))) => {
                    Some(Fault::PointDisturbed(p))
                }
                (FaultKind::TvdDisturbed, Some(ElementRef::Tvd(t))) => Some(Fault::TvdDisturbed(t)),
                (FaultKind::TvdOccupied, Some(ElementRef::Tvd(t))) => Some(Fault::TvdOccupied(t)),
                (FaultKind::SignalDefective, Some(ElementRef::Signal(s))) => {
                    Some(Fault::SignalDefective(s))
                }
                (FaultKind::LxDisturbed, Some(ElementRef::LevelCrossing(l))) => {
                    Some(Fault::LxDisturbed(l))
                }
                _ => None,
            };
            match (fault, on) {
                (Some(f), true) => self.rt.field_mut().simulator_mut().inject(f),
                (Some(f), false) => self.rt.field_mut().simulator_mut().clear(f),
                (None, _) => failures.push(format!("unbekanntes Element „{}“", entry.element)),
            }
        }
        if let Some(a) = &step.aux {
            match self.rt.resolve_aux(&a.op) {
                Ok((area, op)) => self.request(
                    area,
                    Request::AuxRequest {
                        op,
                        operator: OperatorId(a.operator),
                    },
                ),
                Err(e) => failures.push(format!("Hilfshandlung: {e}")),
            }
        }
        failures.extend(self.ops_actions(step));
        if let Some(l) = &step.link
            && let Err(e) = self.rt.field_mut().set_link_faults(&l.controller, l.faults)
        {
            failures.push(e);
        }
        if let Some(l) = &step.ils_link
            && let Err(e) = self.rt.set_ils_faults(&l.link, l.faults)
        {
            failures.push(e);
        }
        if let Some(c) = &step.confirm_aux {
            match self.last_aux {
                Some((area, aux, pending)) => {
                    let op = match &c.op {
                        None => Ok(pending),
                        Some(named) => self.rt.resolve_aux(named).map(|(_, op)| op),
                    };
                    match op {
                        Ok(op) => self.request(
                            area,
                            Request::AuxConfirm {
                                aux,
                                operator: OperatorId(c.operator),
                                op,
                            },
                        ),
                        Err(e) => failures.push(format!("Hilfshandlung: {e}")),
                    }
                }
                None => failures.push("keine Hilfshandlung angefordert".into()),
            }
        }
        failures
    }

    /// Setzt einen Zug ein; mit Betriebsführung auch seine Zugnummer.
    fn add_train(&mut self, t: &TrainEntry) -> Vec<String> {
        let mut failures = Vec::new();
        let signal = match self.global(&t.signal) {
            Some(ElementRef::Signal(s)) => Some(s),
            _ => None,
        };
        let spec = signal
            .zip(metres(t.length_m))
            .map(|(signal, length)| TrainSpec {
                name: t.name.clone(),
                signal,
                length,
                speed: Speed::from_kmh(t.speed_kmh),
                ignore_signals: t.ignore_signals,
                etcs: t.etcs,
            });
        match spec.map(|s| self.rt.field_mut().simulator_mut().add_train(s)) {
            Some(Ok(())) => {
                if let Some(ops) = self.rt.ops_mut()
                    && let Err(e) = ops.describe(&t.name, &t.signal)
                {
                    failures.push(e);
                }
            }
            Some(Err(e)) => failures.push(format!("Zug „{}“: {e}", t.name)),
            None => failures.push(format!(
                "Zug „{}“: unbekanntes Signal oder ungültige Länge",
                t.name
            )),
        }
        failures
    }

    /// Aktionen der Betriebsführung.
    fn ops_actions(&mut self, step: &Step) -> Vec<String> {
        let wanted = step.describe.is_some()
            || step.erase.is_some()
            || step.hold.is_some()
            || step.release.is_some()
            || step.platform.is_some()
            || step.delay.is_some()
            || step.set_ars.is_some();
        if !wanted {
            return Vec::new();
        }
        let Some(ops) = self.rt.ops_mut() else {
            return vec!["Betriebsführung nicht eingeschaltet".into()];
        };
        let results = [
            step.describe
                .as_ref()
                .map(|d| ops.describe(&d.train, &d.signal)),
            step.erase.as_ref().map(|t| ops.erase(t)),
            step.hold.as_ref().map(|t| ops.hold(t, true)),
            step.release.as_ref().map(|t| ops.hold(t, false)),
            step.platform
                .as_ref()
                .map(|p| ops.set_platform(&p.train, &p.station, &p.platform)),
            step.delay.as_ref().map(|d| ops.delay(&d.train, d.s)),
            step.set_ars.map(|on| {
                ops.set_ars(on);
                Ok(())
            }),
        ];
        results
            .into_iter()
            .flatten()
            .filter_map(Result::err)
            .collect()
    }

    /// Prüft Bedingungen des RBC.
    fn check_etcs(&self, c: &Conditions) -> Vec<String> {
        let mut failures = Vec::new();
        let rbc = self.rt.rbc();
        for (train, want) in &c.etcs {
            let got = rbc.mode(train).map(|m| match m {
                railit_etcs::Mode::StandBy => "stand_by",
                railit_etcs::Mode::FullSupervision => "full_supervision",
                railit_etcs::Mode::Trip => "trip",
            });
            if got != Some(want.as_str()) {
                failures.push(format!("ETCS {train}: erwartet {want}, ist {got:?}"));
            }
        }
        for (train, want) in &c.etcs_end {
            let got = rbc.authority(train).map(|a| a.end.as_str());
            if got != Some(want.as_str()) {
                failures.push(format!(
                    "Fahrerlaubnis {train}: Ende erwartet {want}, ist {got:?}"
                ));
            }
        }
        for kind in &c.etcs_event {
            if !self.etcs_events.iter().any(|e| e.kind() == kind) {
                failures.push(format!("RBC-Ereignis „{kind}“ erwartet"));
            }
        }
        failures
    }

    /// Prüft Bedingungen der Betriebsführung.
    fn check_ops(&self, c: &Conditions) -> Vec<String> {
        let mut failures = Vec::new();
        if c.berth.is_empty() && c.at_stop.is_empty() && c.ops_event.is_empty() {
            return failures;
        }
        let Some(ops) = self.rt.ops() else {
            return vec!["Betriebsführung nicht eingeschaltet".into()];
        };
        let snap = ops.snapshot();
        let train = |n: &str| snap.trains.iter().find(|t| t.number == n);
        for (name, want) in &c.berth {
            let got = train(name).map(|t| {
                if t.left {
                    "left".to_string()
                } else {
                    t.berth.clone().unwrap_or_default()
                }
            });
            if got.as_deref() != Some(want.as_str()) {
                failures.push(format!("Zugnummer {name}: erwartet {want}, ist {got:?}"));
            }
        }
        for (name, want) in &c.at_stop {
            let got = train(name).and_then(|t| t.at_stop.clone());
            if got.as_deref() != Some(want.as_str()) {
                failures.push(format!("Zug {name} am Halt: erwartet {want}, ist {got:?}"));
            }
        }
        for kind in &c.ops_event {
            let seen = self.ops_events.iter().any(|e| e.kind() == kind);
            if !seen {
                failures.push(format!("Ereignis der Betriebsführung „{kind}“ erwartet"));
            }
        }
        failures
    }

    /// Prüft die Zustände einzelner Elemente.
    #[allow(clippy::too_many_lines)]
    fn check_elements(&self, c: &Conditions) -> Vec<String> {
        let mut failures = Vec::new();
        let rt = &self.rt;
        let sim = rt.field().simulator();

        for (name, want) in &c.route {
            let got = rt.find_route(name).and_then(|(a, r)| {
                rt.state_of(a)?
                    .routes()
                    .get(r.index()?)
                    .map(railit_kernel::RouteState::phase)
            });
            if got != Some(*want) {
                failures.push(format!("Fahrstraße {name}: erwartet {want:?}, ist {got:?}"));
            }
        }
        let signal = |name: &str| {
            rt.find_signal(name)
                .and_then(|(a, s)| Some((a, s, *rt.state_of(a)?.signals().get(s.index()?)?)))
        };
        for (name, want) in &c.signal {
            let got = signal(name).map(|(_, _, s)| s.commanded());
            if got != Some(*want) {
                failures.push(format!("Signal {name}: erwartet {want:?}, ist {got:?}"));
            }
        }
        for (name, want) in &c.substitute {
            let got = signal(name).map(|(_, _, s)| s.substitute());
            if got != Some(*want) {
                failures.push(format!("Ersatzsignal {name}: erwartet {want}, ist {got:?}"));
            }
        }
        for (name, want) in &c.picture {
            let got = rt.find_signal(name).and_then(|(a, s)| {
                let state = rt.state_of(a)?;
                let plan = rt.plan_of(a)?;
                railit_movement::Pictures::new(plan, state.signals(), state.routes())
                    .picture(s)
                    .map(|p| p.to_string())
            });
            if got.as_deref() != Some(want.as_str()) {
                failures.push(format!("Signalbild {name}: erwartet {want}, ist {got:?}"));
            }
        }
        for (address, want) in &c.permission {
            let got = rt.find_border(address).and_then(|(a, b)| {
                rt.state_of(a)?
                    .borders()
                    .get(b.index()?)
                    .map(railit_kernel::BorderState::holding)
            });
            if got != Some(*want) {
                failures.push(format!("Erlaubnis {address}: erwartet {want}, ist {got:?}"));
            }
        }
        for (name, want) in &c.lx {
            let got = match self.global(name) {
                Some(ElementRef::LevelCrossing(l)) => sim.lx_report(l),
                _ => None,
            };
            if got != Some(*want) {
                failures.push(format!(
                    "Bahnübergang {name}: erwartet {want:?}, ist {got:?}"
                ));
            }
        }
        for (name, want) in &c.point {
            let got = match self.global(name) {
                Some(ElementRef::Point(p)) => sim.point_report(p),
                _ => None,
            };
            if got != Some(*want) {
                failures.push(format!("Weiche {name}: erwartet {want:?}, ist {got:?}"));
            }
        }
        for (name, want) in &c.tvd {
            let got = match self.global(name) {
                Some(ElementRef::Tvd(t)) => sim.occupancy(t),
                _ => None,
            };
            if got != Some(*want) {
                failures.push(format!("Abschnitt {name}: erwartet {want:?}, ist {got:?}"));
            }
        }
        let trains = sim.trains();
        for (name, want) in &c.train {
            // Züge, die das Netz verlassen haben, werden entfernt.
            let got = trains
                .iter()
                .find(|t| &t.name == name)
                .map_or(TrainState::Left, |t| t.state);
            if got != *want {
                failures.push(format!("Zug {name}: erwartet {want:?}, ist {got:?}"));
            }
        }
        failures
    }

    /// Bereich nach Name; ohne Namen der erste.
    fn area(&self, name: Option<&String>) -> Result<usize, String> {
        name.map_or(Ok(0), |n| {
            self.rt
                .find_area(n)
                .ok_or_else(|| format!("unbekannter Stellbereich „{n}“"))
        })
    }

    fn safety_actions(&mut self, step: &Step) -> Vec<String> {
        let mut failures = Vec::new();
        if let Some(f) = &step.channel_fault {
            let result = self
                .area(f.area.as_ref())
                .and_then(|a| self.rt.inject_channel_fault(a, f.channel, f.fault));
            failures.extend(result.err());
        }
        if let Some(r) = &step.reintegrate {
            let result = self
                .area(r.area.as_ref())
                .and_then(|a| self.rt.reintegrate_channel(a, r.channel));
            failures.extend(result.err());
        }
        match &step.reset_safety {
            Some(AreaSelection::All(true)) => {
                for a in 0..self.rt.area_count() {
                    failures.extend(self.rt.reset_safety(a).err());
                }
            }
            Some(AreaSelection::Area(name)) => {
                let result = self.area(Some(name)).and_then(|a| self.rt.reset_safety(a));
                failures.extend(result.err());
            }
            Some(AreaSelection::All(false)) | None => {}
        }
        failures
    }

    fn check_safety(&self, c: &Conditions) -> Vec<String> {
        let mut failures = Vec::new();
        let status: Vec<_> = (0..self.rt.area_count())
            .filter_map(|a| self.rt.safety_status(a))
            .collect();
        if let Some(want) = c.safety_trip {
            let got = status.iter().any(|s| s.trip.is_some());
            if got != want {
                failures.push(format!(
                    "Sicherheitsschicht abgeschaltet: erwartet {want}, ist {got}"
                ));
            }
        }
        for kind in &c.safety_event {
            let got = self.safety_events.iter().any(|e| {
                matches!(
                    (kind.as_str(), e.event),
                    ("channel_excluded", SafetyEvent::ChannelExcluded { .. })
                        | ("tripped", SafetyEvent::Tripped(_))
                )
            });
            if !got {
                failures.push(format!(
                    "Meldung der Sicherheitsschicht „{kind}“ erwartet, gemeldet {:?}",
                    self.safety_events
                ));
            }
        }
        if let Some(want) = &c.active_channels {
            let got: Vec<bool> = status.first().map_or_else(Vec::new, |s| {
                s.active
                    .iter()
                    .take(s.redundancy.channels())
                    .copied()
                    .collect()
            });
            if &got != want {
                failures.push(format!(
                    "mitwählende Kanäle: erwartet {want:?}, ist {got:?}"
                ));
            }
        }
        failures
    }

    /// Prüft Bedingungen; gibt die Abweichungen zurück.
    fn check(&self, c: &Conditions) -> Vec<String> {
        let mut failures = self.check_elements(c);
        failures.extend(self.check_safety(c));
        failures.extend(self.check_ops(c));
        failures.extend(self.check_etcs(c));
        let links = self.rt.field().links();
        for (controller, want) in &c.link_up {
            let got = links
                .iter()
                .find(|l| &l.controller == controller)
                .map(|l| l.up);
            if got != Some(*want) {
                failures.push(format!(
                    "Verbindung {controller}: erwartet {want}, ist {got:?}"
                ));
            }
        }
        let ils = self.rt.ils_links();
        for (link, want) in &c.ils_up {
            let got = ils.iter().find(|l| &l.link == link).map(|l| l.up);
            if got != Some(*want) {
                failures.push(format!(
                    "SCI-ILS-Verbindung {link}: erwartet {want}, ist {got:?}"
                ));
            }
        }
        let states: Vec<&railit_kernel::OwnedState> = (0..self.rt.area_count())
            .filter_map(|a| self.rt.state_of(a))
            .collect();
        let answers: Vec<&NamedEvent> = self
            .events
            .iter()
            .filter(|e| match e {
                NamedEvent::Accepted { request } | NamedEvent::Rejected { request, .. } => {
                    self.requests.contains(request)
                }
                _ => false,
            })
            .collect();
        if let Some(want) = c.accepted {
            let got = answers
                .iter()
                .any(|e| matches!(e, NamedEvent::Accepted { .. }));
            if got != want || answers.is_empty() {
                failures.push(format!(
                    "Anforderung angenommen: erwartet {want}, Antworten {answers:?}"
                ));
            }
        }
        if let Some(reason) = &c.rejected {
            let got = answers
                .iter()
                .any(|e| matches!(e, NamedEvent::Rejected { reason: r, .. } if r == reason));
            if !got {
                failures.push(format!(
                    "Ablehnung „{reason}“ erwartet, Antworten {answers:?}"
                ));
            }
        }
        for alarm in &c.alarm {
            let got = self
                .events
                .iter()
                .any(|e| matches!(e, NamedEvent::Alarm { alarm: a, .. } if a == alarm));
            if !got {
                failures.push(format!("Alarm „{alarm}“ erwartet"));
            }
        }
        for alarm in &c.no_alarm {
            let got = self
                .events
                .iter()
                .any(|e| matches!(e, NamedEvent::Alarm { alarm: a, .. } if a == alarm));
            if got {
                failures.push(format!("Alarm „{alarm}“ nicht erwartet"));
            }
        }
        if let Some(want) = c.aux_counter {
            let got: u64 = states.iter().map(|s| s.meta().aux_counter()).sum();
            if got != want {
                failures.push(format!("Zählwerk: erwartet {want}, ist {got}"));
            }
        }
        for kind in &c.sim_event {
            if !self.sim_events.iter().any(|e| sim_event_kind(e) == kind) {
                failures.push(format!("Simulationsereignis „{kind}“ erwartet"));
            }
        }
        if let Some(want) = c.safe_state {
            let got = states.iter().any(|s| s.meta().safe_state().is_some());
            if got != want {
                failures.push(format!("sicherer Zustand: erwartet {want}, ist {got}"));
            }
        }
        failures
    }
}
