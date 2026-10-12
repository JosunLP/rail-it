//! Engine-Task: besitzt die Runtime und führt den Zyklus aus.
//!
//! Alle Zugriffe auf Kerne und Simulator laufen über Nachrichten an diesen einen Task. Damit
//! gibt es keine geteilten veränderlichen Daten und keine Sperren um die Kerne.

use std::collections::HashMap;
use std::sync::Arc;

use railit_kernel::{Request, RequestId};
use railit_runtime::{Addressed, NamedEvent, Runtime, Snapshot};
use railit_sim::{Fault, SimEvent, Simulator, TrainInfo, TrainSpec};
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
}

/// Antwort auf einen Simulationsbefehl.
#[derive(Debug)]
pub(crate) enum SimReply {
    Done,
    Trains(Vec<TrainInfo>),
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
}

/// Kanäle der Engine.
pub(crate) struct Channels {
    pub(crate) rx: mpsc::Receiver<EngineMsg>,
    pub(crate) snapshot: watch::Sender<Arc<Snapshot>>,
    pub(crate) events: broadcast::Sender<Arc<str>>,
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
    rt: Runtime<Simulator>,
    ch: Channels,
    pending: Vec<Addressed>,
    waiting: HashMap<u64, oneshot::Sender<Answer>>,
    next_id: u64,
    safe_state_logged: bool,
    journal_error_logged: bool,
}

impl Engine {
    fn tick(&mut self) {
        let out = self.rt.cycle_addressed(&self.pending);
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
        }
        let multi = self.rt.area_count() > 1;

        // Ein Kern meldet Ereignisse einer Hilfshandlung unmittelbar vor der Antwort auf die
        // Anforderung, zu der sie gehören.
        let mut aux_event = None;
        for e in &out.events {
            let named = self.rt.name_event(e);
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
        for e in self.rt.field_mut().take_events() {
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
        if let Some((area, reason)) = out.safe_state
            && !self.safe_state_logged
        {
            let area = self.rt.area_name(area).unwrap_or("?");
            tracing::error!(area, ?reason, "Kern im sicheren Zustand: alle Signale Halt");
            self.safe_state_logged = true;
        }
        self.ch.snapshot.send_replace(Arc::new(self.rt.snapshot()));
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
                let _ = reply.send(sim_command(self.rt.field_mut(), command));
            }
        }
    }
}

/// Führt die Engine aus, bis alle Sender geschlossen sind.
pub(crate) async fn run(rt: Runtime<Simulator>, ch: Channels, period: Duration) {
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
}

/// Führt einen Simulationsbefehl aus.
fn sim_command(sim: &mut Simulator, command: SimCommand) -> SimReply {
    match command {
        SimCommand::AddTrain(spec) => match sim.add_train(spec) {
            Ok(()) => SimReply::Done,
            Err(e) => SimReply::Error(e.to_string()),
        },
        SimCommand::RemoveTrain(name) => {
            if sim.remove_train(&name) {
                SimReply::Done
            } else {
                SimReply::Error(format!("Zug „{name}“ nicht gefunden"))
            }
        }
        SimCommand::Trains => SimReply::Trains(sim.trains()),
        SimCommand::Fault(fault, true) => {
            sim.inject(fault);
            SimReply::Done
        }
        SimCommand::Fault(fault, false) => {
            sim.clear(fault);
            SimReply::Done
        }
    }
}
