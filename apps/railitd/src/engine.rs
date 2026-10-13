//! Engine-Task: besitzt die Runtime und führt den Zyklus aus.
//!
//! Alle Zugriffe auf Kerne und Simulator laufen über Nachrichten an diesen einen Task. Damit
//! gibt es keine geteilten veränderlichen Daten und keine Sperren um die Kerne.

use std::collections::HashMap;
use std::sync::Arc;

use railit_kernel::{Request, RequestId};
use railit_runtime::railit_adapters::{LinkFaults, LinkStatus};
use railit_runtime::railit_safety::SafetyEvent;
use railit_runtime::{Addressed, AreaSafetyEvent, NamedEvent, Runtime, Snapshot};
use railit_runtime::{DynField, SimulatedField};
use railit_sim::{Fault, SimEvent, TrainInfo, TrainSpec};

use crate::metrics::Counters;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::{Duration, MissedTickBehavior};

/// Simulationsbefehl.
#[derive(Debug)]
pub(crate) enum SimCommand {
    AddTrain(TrainSpec),
    RemoveTrain(String),
    Trains,
    Fault(Fault, bool),
    /// Störungen der Übertragungsstrecke zu einer Objektsteuerung.
    Link(String, LinkFaults),
    /// Zustand der Übertragungsstrecken.
    Links,
}

/// Befehl an die Betriebsführung (Zugnummern, Disposition).
#[derive(Debug)]
pub(crate) enum OpsCommand {
    Describe {
        train: String,
        signal: String,
    },
    Erase(String),
    Hold(String, bool),
    Platform {
        train: String,
        station: String,
        platform: String,
    },
    Delay(String, u32),
    SetArs(bool),
}

/// Befehl an die Sicherheitsschicht eines Stellbereichs.
#[derive(Debug)]
pub(crate) enum SafetyCommand {
    /// Fehler in einen Kanal einbringen (Prüfung).
    Fault {
        area: usize,
        channel: usize,
        fault: railit_runtime::railit_safety::Fault,
    },
    /// Ausgeschlossenen Kanal wieder aufnehmen.
    Reintegrate { area: usize, channel: usize },
    /// Sicheren Zustand zurücksetzen (Neubeginn wie nach einem Neustart).
    Reset { area: usize },
}

/// Antwort auf einen Simulationsbefehl.
#[derive(Debug)]
pub(crate) enum SimReply {
    Done,
    Trains(Vec<TrainInfo>),
    Links(Vec<LinkStatus>),
    Error(String),
}

/// Antwort eines Kerns auf eine Anforderung.
#[derive(Clone, Debug)]
pub(crate) struct Answer {
    /// Stellbereich, der geantwortet hat.
    pub(crate) area: String,
    /// Angenommen oder abgelehnt.
    pub(crate) event: NamedEvent,
    /// Zugehöriges Ereignis einer Hilfshandlung (angefordert, ausgeführt, verfallen).
    pub(crate) aux: Option<NamedEvent>,
}

/// Nachricht an die Engine.
#[derive(Debug)]
pub(crate) enum EngineMsg {
    /// Bedienanforderung an den Kern eines Bereichs; Antwort nach dem nächsten Zyklus.
    Request {
        area: usize,
        request: Request,
        reply: oneshot::Sender<Answer>,
    },
    /// Simulationsbefehl; Antwort sofort.
    Sim {
        command: SimCommand,
        reply: oneshot::Sender<SimReply>,
    },
    /// Befehl an die Betriebsführung; Antwort sofort.
    Ops {
        command: OpsCommand,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Befehl an die Sicherheitsschicht; Antwort sofort.
    Safety {
        command: SafetyCommand,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// Kanäle der Engine.
pub(crate) struct Channels {
    pub(crate) rx: mpsc::Receiver<EngineMsg>,
    pub(crate) snapshot: watch::Sender<Arc<Snapshot>>,
    pub(crate) events: broadcast::Sender<Arc<str>>,
    pub(crate) metrics: watch::Sender<Arc<Counters>>,
}

fn event_json(time_ms: u64, area: Option<&str>, mut value: Value) -> Arc<str> {
    if let Value::Object(map) = &mut value {
        map.insert("time_ms".into(), json!(time_ms));
        if let Some(area) = area {
            map.insert("area".into(), json!(area));
        }
    }
    Arc::from(value.to_string())
}

/// Zustand der Engine zwischen den Zyklen.
struct Engine {
    rt: Runtime<DynField>,
    ch: Channels,
    pending: Vec<Addressed>,
    waiting: HashMap<u64, oneshot::Sender<Answer>>,
    next_id: u64,
    safe_state_logged: bool,
    journal_error_logged: bool,
    period: Duration,
    counters: Counters,
}

impl Engine {
    fn tick(&mut self) {
        let started = std::time::Instant::now();
        let out = self.rt.cycle_addressed(&self.pending);
        self.counters.observe_cycle(started.elapsed(), self.period);
        self.pending.clear();
        let time_ms = out.now.as_millis();
        if let Some(error) = self.rt.journal_error()
            && !self.journal_error_logged
        {
            tracing::error!(
                error,
                "Journal abgebrochen; der Betrieb läuft ohne Journal weiter"
            );
            self.journal_error_logged = true;
            self.counters.journal_failed = true;
        }
        let multi = self.rt.area_count() > 1;

        // Ein Kern meldet Ereignisse einer Hilfshandlung unmittelbar vor der Antwort auf die
        // Anforderung, zu der sie gehören.
        let mut aux_event = None;
        for e in &out.events {
            let named = self.rt.name_event(e);
            self.counters.observe_event(&named);
            let area = self.rt.area_name(e.area).unwrap_or("?").to_string();
            match &named {
                NamedEvent::Alarm { alarm, element } => {
                    tracing::warn!(area, alarm, ?element, "Alarm");
                }
                NamedEvent::AuxExecuted { aux, op, counter } => {
                    tracing::warn!(area, aux, ?op, counter, "Hilfshandlung ausgeführt");
                    aux_event = Some(named.clone());
                }
                NamedEvent::AuxPending { .. } | NamedEvent::AuxExpired { .. } => {
                    aux_event = Some(named.clone());
                }
                NamedEvent::Accepted { request } | NamedEvent::Rejected { request, .. } => {
                    let aux = aux_event.take();
                    if let Some(reply) = self.waiting.remove(request) {
                        let _ = reply.send(Answer {
                            area: area.clone(),
                            event: named.clone(),
                            aux,
                        });
                    }
                }
                NamedEvent::Permission { border, holding } => {
                    tracing::info!(border, holding, "Erlaubniswechsel");
                }
                NamedEvent::RoutePhase { .. } => {}
            }
            let value = serde_json::to_value(&named).unwrap_or(Value::Null);
            let _ = self
                .ch
                .events
                .send(event_json(time_ms, multi.then_some(area.as_str()), value));
        }
        for e in &out.etcs {
            if let railit_runtime::railit_etcs::RbcEvent::EmergencyStop { train, reason } = e {
                self.counters.etcs_emergency_stops =
                    self.counters.etcs_emergency_stops.saturating_add(1);
                tracing::warn!(train, reason, "ETCS-Notbremsung");
            }
            let mut value = serde_json::to_value(e).unwrap_or(Value::Null);
            if let Value::Object(map) = &mut value {
                map.insert("source".into(), json!("etcs"));
            }
            let _ = self.ch.events.send(event_json(time_ms, None, value));
        }
        for e in &out.ops {
            let mut value = serde_json::to_value(e).unwrap_or(Value::Null);
            if let Value::Object(map) = &mut value {
                map.insert("source".into(), json!("ops"));
            }
            let _ = self.ch.events.send(event_json(time_ms, None, value));
        }
        for e in self.rt.field_mut().simulator_mut().take_events() {
            if matches!(
                e,
                SimEvent::TrainDerailed { .. }
                    | SimEvent::SignalPassedAtDanger { .. }
                    | SimEvent::PointTrailed { .. }
                    | SimEvent::LevelCrossingNotClosed { .. }
            ) {
                tracing::warn!(?e, "Simulation");
            }
            let mut value = serde_json::to_value(&e).unwrap_or(Value::Null);
            if let Value::Object(map) = &mut value {
                map.insert("type".into(), json!("sim"));
            }
            let _ = self.ch.events.send(event_json(time_ms, None, value));
        }
        self.publish_safety(time_ms, &out.safety);
        if let Some((area, reason)) = out.safe_state
            && !self.safe_state_logged
        {
            let area = self.rt.area_name(area).unwrap_or("?");
            tracing::error!(area, ?reason, "Kern im sicheren Zustand: alle Signale Halt");
            self.safe_state_logged = true;
        }
        self.ch.snapshot.send_replace(Arc::new(self.rt.snapshot()));
        self.ch
            .metrics
            .send_replace(Arc::new(self.counters.clone()));
    }

    /// Meldungen der Sicherheitsschichten: protokollieren und als Ereignis senden.
    fn publish_safety(&mut self, time_ms: u64, events: &[AreaSafetyEvent]) {
        for e in events {
            match e.event {
                SafetyEvent::ChannelExcluded { .. } => {
                    self.counters.channel_exclusions =
                        self.counters.channel_exclusions.saturating_add(1);
                }
                SafetyEvent::Tripped(_) => {
                    self.counters.safety_trips = self.counters.safety_trips.saturating_add(1);
                }
            }
            let area = self.rt.area_name(e.area).unwrap_or("?").to_string();
            let value = match e.event {
                SafetyEvent::ChannelExcluded {
                    channel,
                    divergence,
                } => {
                    tracing::warn!(area, channel, ?divergence, "Kanal ausgeschlossen");
                    json!({"type": "safety", "event": "channel_excluded",
                           "channel": channel, "divergence": divergence})
                }
                SafetyEvent::Tripped(trip) => {
                    tracing::error!(area, ?trip, "Sicherheitsschicht abgeschaltet");
                    json!({"type": "safety", "event": "tripped", "trip": trip})
                }
            };
            let _ = self
                .ch
                .events
                .send(event_json(time_ms, Some(area.as_str()), value));
        }
    }

    fn message(&mut self, msg: EngineMsg) {
        match msg {
            EngineMsg::Request {
                area,
                request,
                reply,
            } => {
                self.next_id = self.next_id.saturating_add(1);
                self.pending.push(Addressed {
                    area,
                    id: RequestId(self.next_id),
                    request,
                });
                self.waiting.insert(self.next_id, reply);
            }
            EngineMsg::Sim { command, reply } => {
                let _ = reply.send(sim_command(&mut self.rt, command));
            }
            EngineMsg::Safety { command, reply } => {
                let result = match command {
                    SafetyCommand::Fault {
                        area,
                        channel,
                        fault,
                    } => self.rt.inject_channel_fault(area, channel, fault),
                    SafetyCommand::Reintegrate { area, channel } => {
                        self.rt.reintegrate_channel(area, channel)
                    }
                    SafetyCommand::Reset { area } => {
                        let result = self.rt.reset_safety(area);
                        if result.is_ok() {
                            tracing::warn!(area, "sicherer Zustand zurückgesetzt");
                            self.safe_state_logged = false;
                        }
                        result
                    }
                };
                self.ch.snapshot.send_replace(Arc::new(self.rt.snapshot()));
                let _ = reply.send(result);
            }
            EngineMsg::Ops { command, reply } => {
                let result = ops_command(&mut self.rt, command);
                // Wer nach der Antwort das Zustandsbild liest, sieht die Änderung.
                self.ch.snapshot.send_replace(Arc::new(self.rt.snapshot()));
                let _ = reply.send(result);
            }
        }
    }
}

/// Führt die Engine aus, bis alle Sender geschlossen sind.
pub(crate) async fn run(rt: Runtime<DynField>, ch: Channels, period: Duration) {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut engine = Engine {
        rt,
        ch,
        pending: Vec::new(),
        waiting: HashMap::new(),
        next_id: 0,
        safe_state_logged: false,
        journal_error_logged: false,
        period,
        counters: Counters::default(),
    };
    loop {
        tokio::select! {
            _ = interval.tick() => engine.tick(),
            msg = engine.ch.rx.recv() => match msg {
                None => break,
                Some(msg) => engine.message(msg),
            },
        }
    }
    engine.rt.seal_journals();
}

/// Führt einen Befehl an die Betriebsführung aus.
fn ops_command(rt: &mut Runtime<DynField>, command: OpsCommand) -> Result<(), String> {
    let ops = rt.ops_mut().ok_or("Betriebsführung nicht eingeschaltet")?;
    match command {
        OpsCommand::Describe { train, signal } => ops.describe(&train, &signal),
        OpsCommand::Erase(train) => ops.erase(&train),
        OpsCommand::Hold(train, on) => ops.hold(&train, on),
        OpsCommand::Platform {
            train,
            station,
            platform,
        } => ops.set_platform(&train, &station, &platform),
        OpsCommand::Delay(train, s) => ops.delay(&train, s),
        OpsCommand::SetArs(on) => {
            ops.set_ars(on);
            Ok(())
        }
    }
}

/// Führt einen Simulationsbefehl aus. Neue Züge erhalten ihre Zugnummer am Signal.
fn sim_command(rt: &mut Runtime<DynField>, command: SimCommand) -> SimReply {
    match command {
        SimCommand::AddTrain(spec) => add_train(rt, spec),
        SimCommand::Link(controller, faults) => {
            match rt.field_mut().set_link_faults(&controller, faults) {
                Ok(()) => SimReply::Done,
                Err(e) => SimReply::Error(e),
            }
        }
        SimCommand::Links => SimReply::Links(rt.field().links()),
        SimCommand::RemoveTrain(name) => {
            if rt.field_mut().simulator_mut().remove_train(&name) {
                SimReply::Done
            } else {
                SimReply::Error(format!("Zug „{name}“ nicht gefunden"))
            }
        }
        SimCommand::Trains => SimReply::Trains(rt.field().simulator().trains()),
        SimCommand::Fault(fault, true) => {
            rt.field_mut().simulator_mut().inject(fault);
            SimReply::Done
        }
        SimCommand::Fault(fault, false) => {
            rt.field_mut().simulator_mut().clear(fault);
            SimReply::Done
        }
    }
}

fn add_train(rt: &mut Runtime<DynField>, spec: TrainSpec) -> SimReply {
    let name = spec.name.clone();
    let signal = rt
        .field()
        .simulator()
        .topology()
        .signal(spec.signal)
        .map(|s| s.name.clone());
    if let Err(e) = rt.field_mut().simulator_mut().add_train(spec) {
        return SimReply::Error(e.to_string());
    }
    if let (Some(ops), Some(signal)) = (rt.ops_mut(), signal)
        && let Err(e) = ops.describe(&name, &signal)
    {
        return SimReply::Error(e);
    }
    SimReply::Done
}
