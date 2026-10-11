//! Engine-Task: besitzt die Runtime und führt den Zyklus aus.
//!
//! Alle Zugriffe auf Kern und Simulator laufen über Nachrichten an diesen einen Task. Damit
//! gibt es keine geteilten veränderlichen Daten und keine Sperren um den Kern.

use std::collections::HashMap;
use std::sync::Arc;

use railit_kernel::{Request, RequestId};
use railit_runtime::{NamedEvent, Runtime, Snapshot};
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

/// Nachricht an die Engine.
#[derive(Debug)]
pub(crate) enum EngineMsg {
    /// Bedienanforderung; Antwort nach dem nächsten Zyklus.
    Request {
        request: Request,
        reply: oneshot::Sender<NamedEvent>,
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

fn event_json(time_ms: u64, mut value: Value) -> Arc<str> {
    if let Value::Object(map) = &mut value {
        map.insert("time_ms".into(), json!(time_ms));
    }
    Arc::from(value.to_string())
}

/// Führt die Engine aus, bis alle Sender geschlossen sind.
pub(crate) async fn run(mut rt: Runtime<Simulator>, mut ch: Channels, period: Duration) {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut pending: Vec<(RequestId, Request)> = Vec::new();
    let mut waiting: HashMap<u64, oneshot::Sender<NamedEvent>> = HashMap::new();
    let mut next_id = 0u64;
    let mut safe_state_logged = false;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let out = rt.cycle(&pending);
                pending.clear();
                let time_ms = out.now.as_millis();

                for e in &out.events {
                    let named = rt.name_event(e);
                    if let NamedEvent::Alarm { alarm, element } = &named {
                        tracing::warn!(alarm, ?element, "Alarm");
                    }
                    if let NamedEvent::Accepted { request } | NamedEvent::Rejected { request, .. } = &named
                        && let Some(reply) = waiting.remove(request)
                    {
                        let _ = reply.send(named.clone());
                    }
                    let value = serde_json::to_value(&named).unwrap_or(Value::Null);
                    let _ = ch.events.send(event_json(time_ms, value));
                }
                for e in rt.field_mut().take_events() {
                    if matches!(e, SimEvent::TrainDerailed { .. } | SimEvent::SignalPassedAtDanger { .. } | SimEvent::PointTrailed { .. }) {
                        tracing::warn!(?e, "Simulation");
                    }
                    let mut value = serde_json::to_value(&e).unwrap_or(Value::Null);
                    if let Value::Object(map) = &mut value {
                        map.insert("type".into(), json!("sim"));
                    }
                    let _ = ch.events.send(event_json(time_ms, value));
                }
                if let Some(reason) = out.safe_state
                    && !safe_state_logged
                {
                    tracing::error!(?reason, "Kern im sicheren Zustand: alle Signale Halt");
                    safe_state_logged = true;
                }
                ch.snapshot.send_replace(Arc::new(rt.snapshot()));
            }
            msg = ch.rx.recv() => match msg {
                None => break,
                Some(EngineMsg::Request { request, reply }) => {
                    next_id = next_id.saturating_add(1);
                    pending.push((RequestId(next_id), request));
                    waiting.insert(next_id, reply);
                }
                Some(EngineMsg::Sim { command, reply }) => {
                    let sim = rt.field_mut();
                    let result = match command {
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
                    };
                    let _ = reply.send(result);
                }
            },
        }
    }
}
