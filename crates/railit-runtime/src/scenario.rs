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
//! `add_train = { name, signal, length_m, speed_kmh, ignore_signals }`,
//! `fault` / `clear_fault = { kind, element }` mit `kind` aus `point_disturbed`,
//! `tvd_disturbed`, `tvd_occupied`, `signal_defective`, `lx_disturbed`,
//! `aux = { kind, element, position, operator }` (Hilfshandlung anfordern; `kind` aus
//! `release_route`, `reset_tvd`, `substitute`, `force_point`),
//! `confirm_aux = { operator }` (zuletzt angeforderte Hilfshandlung bestätigen).
//!
//! **Bedingungen** (`until`, `expect`): `route`, `signal` (befohlener Begriff),
//! `substitute` (Ersatzsignal ein/aus), `picture` (Signalbild, z. B. `"Ks1 Zs3(4)"`), `point`, `tvd` (Meldung des Felds), `lx` (Meldung
//! des Bahnübergangs), `train`, `accepted`, `rejected` (Grund), `alarm` / `no_alarm`
//! (Listen), `sim_event` (Liste), `aux_counter`, `safe_state`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use railit_kernel::{
    Aspect, AuxId, LxReport, Occupancy, OperatorId, PointReport, Request, RequestId, RoutePhase,
};
use railit_rules::GenericRules;
use railit_sim::{Fault, SimConfig, SimEvent, Simulator, TrainSpec, TrainState};
use railit_types::{Duration, Length, PointPosition, Speed};
use serde::Deserialize;

use crate::names::{NamedAux, NamedEvent};
use crate::runtime::Runtime;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioFile {
    scenario: Header,
    #[serde(default)]
    step: Vec<Step>,
}

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
    wait_s: Option<f64>,
    #[serde(default)]
    until: Option<Conditions>,
    #[serde(default = "default_timeout_s")]
    timeout_s: f64,
    #[serde(default)]
    expect: Option<Conditions>,
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
}

// `deny_unknown_fields` verträgt sich nicht mit `flatten`.
#[derive(Debug, Deserialize)]
struct AuxEntry {
    #[serde(flatten)]
    op: NamedAux,
    operator: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmEntry {
    operator: u32,
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
    let source = std::fs::read_to_string(path)
        .map_err(|e| ScenarioError::Io(format!("{}: {e}", path.display())))?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    run_str(&source, base)
}

/// Führt ein Szenario aus; relative Pfade beziehen sich auf `base`.
///
/// # Errors
/// Wie [`run_file`].
pub fn run_str(source: &str, base: &Path) -> Result<Outcome, ScenarioError> {
    let file: ScenarioFile =
        toml::from_str(source).map_err(|e| ScenarioError::Syntax(e.to_string()))?;
    let header = file.scenario;

    let net_path = base.join(&header.network);
    let net_source = std::fs::read_to_string(&net_path)
        .map_err(|e| ScenarioError::Io(format!("{}: {e}", net_path.display())))?;
    let net = railit_import::toml_format::parse(&net_source)
        .map_err(|e| ScenarioError::Network(e.to_string()))?
        .topology;
    let compiled =
        railit_planning::compile(&net, None).map_err(|e| ScenarioError::Planning(e.to_string()))?;
    let plan = crate::load_plan(compiled.image, None)
        .map_err(|e| ScenarioError::Planning(e.to_string()))?;

    let mut cfg = SimConfig::default();
    if let Some(ms) = header.point_move_time_ms {
        cfg.point_move_time = Duration::from_millis(ms);
    }
    if let Some(ms) = header.lx_close_time_ms {
        cfg.lx_close_time = Duration::from_millis(ms);
    }
    let mut sim = Simulator::new(net, cfg);
    for (name, pos) in &header.points {
        let id = plan
            .find_point(name)
            .ok_or_else(|| ScenarioError::Syntax(format!("unbekannte Weiche „{name}“")))?;
        sim.set_point_position(id, *pos);
    }

    let mut rules = GenericRules::default();
    if let Some(ms) = header.rules.point_move_timeout_ms {
        rules.point_move_timeout = Duration::from_millis(ms);
    }
    if let Some(ms) = header.rules.report_timeout_ms {
        rules.report_timeout = Duration::from_millis(ms);
    }
    if let Some(ms) = header.rules.cancel_time_lock_ms {
        rules.cancel_time_lock = Duration::from_millis(ms);
    }
    if let Some(ms) = header.rules.overlap_release_time_ms {
        rules.overlap_release_time = Duration::from_millis(ms);
    }
    if let Some(ms) = header.rules.aux_confirm_timeout_ms {
        rules.aux_confirm_timeout = Duration::from_millis(ms);
    }
    if let Some(ms) = header.rules.substitute_duration_ms {
        rules.substitute_duration = Duration::from_millis(ms);
    }

    let mut ctx = Ctx {
        rt: Runtime::new(
            plan,
            rules,
            sim,
            Duration::from_millis(header.cycle_ms.max(1)),
        ),
        next_request: 0,
        pending: Vec::new(),
        events: Vec::new(),
        sim_events: Vec::new(),
        requests: Vec::new(),
        last_aux: None,
    };
    // Erster Zyklus: Grundstellung (alle Signale Halt, Meldungen übernommen).
    ctx.cycle();

    let mut steps = Vec::new();
    for (i, step) in file.step.iter().enumerate() {
        steps.push(ctx.run_step(i.saturating_add(1), step));
    }
    let passed = steps.iter().all(|s| s.passed);
    Ok(Outcome {
        name: header.name,
        description: header.description,
        steps,
        passed,
    })
}

struct Ctx {
    rt: Runtime<Simulator>,
    next_request: u64,
    pending: Vec<(RequestId, Request)>,
    events: Vec<NamedEvent>,
    sim_events: Vec<SimEvent>,
    requests: Vec<u64>,
    /// Zuletzt angeforderte Hilfshandlung (für `confirm_aux`).
    last_aux: Option<u32>,
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

impl Ctx {
    fn cycle(&mut self) {
        let out = self.rt.cycle(&self.pending);
        self.pending.clear();
        for e in &out.events {
            let named = self.rt.name_event(e);
            if let NamedEvent::AuxPending { aux, .. } = named {
                self.last_aux = Some(aux);
            }
            self.events.push(named);
        }
        self.sim_events.extend(self.rt.field_mut().take_events());
    }

    fn request(&mut self, request: Request) {
        self.next_request = self.next_request.saturating_add(1);
        self.pending.push((RequestId(self.next_request), request));
        self.requests.push(self.next_request);
    }

    fn run_step(&mut self, index: usize, step: &Step) -> StepOutcome {
        self.events.clear();
        self.sim_events.clear();
        self.requests.clear();
        let mut failures = self.actions(step);

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
        let plan = *self.rt.plan();
        if let Some(name) = &step.set_route {
            match plan.find_route(name) {
                Some(r) => self.request(Request::SetRoute(r)),
                None => failures.push(format!("unbekannte Fahrstraße „{name}“")),
            }
        }
        if let Some(name) = &step.cancel_route {
            match plan.find_route(name) {
                Some(r) => self.request(Request::CancelRoute(r)),
                None => failures.push(format!("unbekannte Fahrstraße „{name}“")),
            }
        }
        if let Some(m) = &step.move_point {
            match plan.find_point(&m.point) {
                Some(p) => self.request(Request::MovePoint(p, m.position)),
                None => failures.push(format!("unbekannte Weiche „{}“", m.point)),
            }
        }
        if let Some(t) = &step.add_train {
            let spec =
                plan.find_signal(&t.signal)
                    .zip(metres(t.length_m))
                    .map(|(signal, length)| TrainSpec {
                        name: t.name.clone(),
                        signal,
                        length,
                        speed: Speed::from_kmh(t.speed_kmh),
                        ignore_signals: t.ignore_signals,
                    });
            match spec.map(|s| self.rt.field_mut().add_train(s)) {
                Some(Ok(())) => {}
                Some(Err(e)) => failures.push(format!("Zug „{}“: {e}", t.name)),
                None => failures.push(format!(
                    "Zug „{}“: unbekanntes Signal oder ungültige Länge",
                    t.name
                )),
            }
        }
        for (entry, on) in [(&step.fault, true), (&step.clear_fault, false)] {
            let Some(entry) = entry else { continue };
            let fault = match entry.kind {
                FaultKind::PointDisturbed => {
                    plan.find_point(&entry.element).map(Fault::PointDisturbed)
                }
                FaultKind::TvdDisturbed => plan.find_tvd(&entry.element).map(Fault::TvdDisturbed),
                FaultKind::TvdOccupied => plan.find_tvd(&entry.element).map(Fault::TvdOccupied),
                FaultKind::SignalDefective => {
                    plan.find_signal(&entry.element).map(Fault::SignalDefective)
                }
                FaultKind::LxDisturbed => plan.find_lx(&entry.element).map(Fault::LxDisturbed),
            };
            match (fault, on) {
                (Some(f), true) => self.rt.field_mut().inject(f),
                (Some(f), false) => self.rt.field_mut().clear(f),
                (None, _) => failures.push(format!("unbekanntes Element „{}“", entry.element)),
            }
        }
        if let Some(a) = &step.aux {
            match a.op.resolve(&plan) {
                Ok(op) => self.request(Request::AuxRequest {
                    op,
                    operator: OperatorId(a.operator),
                }),
                Err(e) => failures.push(format!("Hilfshandlung: {e}")),
            }
        }
        if let Some(c) = &step.confirm_aux {
            match self.last_aux {
                Some(aux) => self.request(Request::AuxConfirm {
                    aux: AuxId(aux),
                    operator: OperatorId(c.operator),
                }),
                None => failures.push("keine Hilfshandlung angefordert".into()),
            }
        }
        failures
    }

    /// Prüft die Zustände einzelner Elemente.
    fn check_elements(&self, c: &Conditions) -> Vec<String> {
        let mut failures = Vec::new();
        let plan = *self.rt.plan();
        let state = self.rt.state();
        let sim = self.rt.field();

        for (name, want) in &c.route {
            let got = plan
                .find_route(name)
                .and_then(railit_plan::RouteId::index)
                .and_then(|i| state.routes().get(i))
                .map(railit_kernel::RouteState::phase);
            if got != Some(*want) {
                failures.push(format!("Fahrstraße {name}: erwartet {want:?}, ist {got:?}"));
            }
        }
        for (name, want) in &c.signal {
            let got = plan
                .find_signal(name)
                .and_then(railit_plan::SignalId::index)
                .and_then(|i| state.signals().get(i))
                .map(railit_kernel::SignalState::commanded);
            if got != Some(*want) {
                failures.push(format!("Signal {name}: erwartet {want:?}, ist {got:?}"));
            }
        }
        for (name, want) in &c.substitute {
            let got = plan
                .find_signal(name)
                .and_then(railit_plan::SignalId::index)
                .and_then(|i| state.signals().get(i))
                .map(railit_kernel::SignalState::substitute);
            if got != Some(*want) {
                failures.push(format!("Ersatzsignal {name}: erwartet {want}, ist {got:?}"));
            }
        }
        let pictures = railit_movement::Pictures::new(&plan, state.signals(), state.routes());
        for (name, want) in &c.picture {
            let got = plan
                .find_signal(name)
                .and_then(|s| pictures.picture(s))
                .map(|p| p.to_string());
            if got.as_deref() != Some(want.as_str()) {
                failures.push(format!("Signalbild {name}: erwartet {want}, ist {got:?}"));
            }
        }
        for (name, want) in &c.lx {
            let got = plan.find_lx(name).and_then(|l| sim.lx_report(l));
            if got != Some(*want) {
                failures.push(format!(
                    "Bahnübergang {name}: erwartet {want:?}, ist {got:?}"
                ));
            }
        }
        for (name, want) in &c.point {
            let got = plan.find_point(name).and_then(|p| sim.point_report(p));
            if got != Some(*want) {
                failures.push(format!("Weiche {name}: erwartet {want:?}, ist {got:?}"));
            }
        }
        for (name, want) in &c.tvd {
            let got = plan.find_tvd(name).and_then(|t| sim.occupancy(t));
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

    /// Prüft Bedingungen; gibt die Abweichungen zurück.
    fn check(&self, c: &Conditions) -> Vec<String> {
        let mut failures = self.check_elements(c);
        let state = self.rt.state();
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
            let got = state.meta().aux_counter();
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
            let got = state.meta().safe_state().is_some();
            if got != want {
                failures.push(format!("sicherer Zustand: erwartet {want}, ist {got}"));
            }
        }
        failures
    }
}
