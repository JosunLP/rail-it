//! Testgerüst: Kern mit Beispielstadt-Plan und einem einfachen Feldmodell.
//!
//! Das Feld meldet in jedem Zyklus alle Elemente, setzt Signalbefehle sofort um und stellt
//! Weichen sofort um (sofern nicht blockiert).

#![allow(dead_code, unreachable_pub)]

use railit_kernel::*;
use railit_plan::{Plan, PlanView, RouteId, SignalId};
use railit_rules::GenericRules;
use railit_types::{BorderId, Duration, LxId, PointId, PointPosition, Tick, TvdId};

const EXAMPLE: &str = include_str!("../../../../examples/beispielstadt.toml");

/// Plan der Beispielstadt.
pub fn example_plan() -> Plan {
    let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
    railit_planning::compile(&net, None).unwrap().plan
}

/// Image dauerhaft im Speicher ablegen (Testprozess).
pub fn leak(plan: &Plan) -> PlanView<'static> {
    let image: &'static [u8] = Box::leak(plan.to_image().unwrap().into_boxed_slice());
    PlanView::parse(image).unwrap()
}

pub struct Harness {
    pub kernel: Interlocking<'static, GenericRules>,
    pub state: OwnedState,
    pub now: Tick,
    pub points: Vec<PointReport>,
    pub tvds: Vec<Occupancy>,
    pub signals: Vec<SignalReport>,
    pub lxs: Vec<LxReport>,
    /// Bahnübergangsmeldungen vor dem letzten Zyklus (für Prüfungen).
    pub lxs_before: Vec<LxReport>,
    /// Signale mit eingeschaltetem Ersatzsignal.
    pub substitutes: Vec<SignalId>,
    /// Bahnübergänge, die Befehle ignorieren.
    pub stuck_lxs: Vec<LxId>,
    /// Meldungen des Nachbarn je Grenze, in jedem Zyklus gesendet (`None`: keine Meldung).
    pub border_in: Vec<Option<BorderMessage>>,
    /// Zuletzt gesendete Meldungen an die Nachbarn.
    pub border_out: Vec<Option<BorderMessage>>,
    /// Weichen, die Stellbefehle ignorieren.
    pub stuck_points: Vec<PointId>,
    /// Abschnitte, die nicht mehr melden.
    pub silent_tvds: Vec<TvdId>,
    /// Signale, die Befehle ignorieren.
    pub stuck_signals: Vec<SignalId>,
    pub out: VecOutputs,
    pub last: Result<(), SafeStateReason>,
    next_request: u64,
}

impl Harness {
    pub fn new() -> Self {
        Self::with_plan(leak(&example_plan()))
    }

    pub fn with_plan(plan: PlanView<'static>) -> Self {
        Self {
            kernel: Interlocking::new(plan, GenericRules::default()),
            state: OwnedState::new(&plan),
            now: Tick::ZERO,
            points: vec![PointReport::Right; plan.point_count()],
            tvds: vec![Occupancy::Clear; plan.tvd_count()],
            signals: vec![SignalReport::Stop; plan.signal_count()],
            lxs: vec![LxReport::Open; plan.lx_count()],
            lxs_before: vec![LxReport::Open; plan.lx_count()],
            substitutes: Vec::new(),
            stuck_lxs: Vec::new(),
            border_in: vec![None; plan.border_count()],
            border_out: vec![None; plan.border_count()],
            stuck_points: Vec::new(),
            silent_tvds: Vec::new(),
            stuck_signals: Vec::new(),
            out: VecOutputs::default(),
            last: Ok(()),
            next_request: 0,
        }
    }

    pub fn plan(&self) -> &PlanView<'static> {
        self.kernel.plan()
    }

    pub fn route(&self, name: &str) -> RouteId {
        self.plan()
            .find_route(name)
            .unwrap_or_else(|| panic!("Fahrstraße {name}"))
    }

    pub fn tvd(&self, name: &str) -> TvdId {
        self.plan().find_tvd(name).unwrap()
    }

    pub fn point(&self, name: &str) -> PointId {
        self.plan().find_point(name).unwrap()
    }

    pub fn signal(&self, name: &str) -> SignalId {
        self.plan().find_signal(name).unwrap()
    }

    pub fn lx(&self, name: &str) -> LxId {
        self.plan().find_lx(name).unwrap()
    }

    pub fn set_lx(&mut self, name: &str, report: LxReport) {
        let i = self.lx(name).index().unwrap();
        self.lxs[i] = report;
    }

    pub fn lx_state(&self, name: &str) -> LxState {
        self.state.level_crossings()[self.lx(name).index().unwrap()]
    }

    pub fn point_state(&self, name: &str) -> PointState {
        self.state.points()[self.point(name).index().unwrap()]
    }

    pub fn tvd_state(&self, name: &str) -> TvdState {
        self.state.tvds()[self.tvd(name).index().unwrap()]
    }

    pub fn signal_state(&self, name: &str) -> SignalState {
        self.state.signals()[self.signal(name).index().unwrap()]
    }

    pub fn phase(&self, route: &str) -> RoutePhase {
        self.state.routes()[self.route(route).index().unwrap()].phase()
    }

    pub fn aspect(&self, signal: &str) -> Aspect {
        self.state.signals()[self.signal(signal).index().unwrap()].commanded()
    }

    pub fn set_tvd(&mut self, name: &str, occupancy: Occupancy) {
        let i = self.tvd(name).index().unwrap();
        self.tvds[i] = occupancy;
    }

    pub fn set_point(&mut self, name: &str, report: PointReport) {
        let i = self.point(name).index().unwrap();
        self.points[i] = report;
    }

    pub fn set_signal(&mut self, name: &str, report: SignalReport) {
        let i = self.signal(name).index().unwrap();
        self.signals[i] = report;
    }

    /// Meldungen des Felds für diesen Zyklus.
    pub fn reports(&self) -> Vec<Input> {
        let mut inputs = Vec::new();
        for (i, r) in self.points.iter().enumerate() {
            inputs.push(Input::Point {
                point: PointId::from_index(i).unwrap(),
                report: *r,
            });
        }
        for (i, o) in self.tvds.iter().enumerate() {
            let tvd = TvdId::from_index(i).unwrap();
            if !self.silent_tvds.contains(&tvd) {
                inputs.push(Input::Tvd { tvd, occupancy: *o });
            }
        }
        for (i, r) in self.signals.iter().enumerate() {
            inputs.push(Input::Signal {
                signal: SignalId::from_index(i).unwrap(),
                report: *r,
            });
        }
        for (i, r) in self.lxs.iter().enumerate() {
            inputs.push(Input::LevelCrossing {
                lx: LxId::from_index(i).unwrap(),
                report: *r,
            });
        }
        for (i, m) in self.border_in.iter().enumerate() {
            if let Some(message) = m {
                inputs.push(Input::Border {
                    border: BorderId::from_index(i).unwrap(),
                    message: *message,
                });
            }
        }
        inputs
    }

    /// Ein Zyklus nach `dt` Millisekunden mit den gegebenen Anforderungen.
    pub fn step_after(&mut self, dt: u64, requests: &[Request]) -> Vec<Event> {
        self.now = self.now.checked_add(Duration::from_millis(dt)).unwrap();
        let mut inputs = self.reports();
        for r in requests {
            self.next_request += 1;
            inputs.push(Input::Request {
                id: RequestId(self.next_request),
                request: *r,
            });
        }
        self.out.clear();
        self.last = self
            .kernel
            .step(&mut self.state.view(), self.now, &inputs, &mut self.out);
        self.apply_commands();
        self.out.events.clone()
    }

    /// Ein Zyklus nach 100 ms.
    pub fn step(&mut self, requests: &[Request]) -> Vec<Event> {
        self.step_after(100, requests)
    }

    /// Mehrere Zyklen ohne Anforderungen.
    pub fn run(&mut self, cycles: usize) {
        for _ in 0..cycles {
            self.step(&[]);
        }
    }

    fn apply_commands(&mut self) {
        for c in self.out.commands.clone() {
            match c {
                Command::MovePoint { point, position } => {
                    if !self.stuck_points.contains(&point) {
                        self.points[point.index().unwrap()] = position.into();
                    }
                }
                Command::SetSignal { signal, aspect } => {
                    if !self.stuck_signals.contains(&signal) {
                        let i = signal.index().unwrap();
                        if self.signals[i] != SignalReport::Defective {
                            self.signals[i] = match aspect {
                                Aspect::Stop => SignalReport::Stop,
                                Aspect::Proceed => SignalReport::Proceed,
                            };
                        }
                    }
                }
                Command::SetSubstitute { signal, on } => {
                    self.substitutes.retain(|s| *s != signal);
                    if on {
                        self.substitutes.push(signal);
                    }
                }
                Command::SetLevelCrossing { lx, close } => {
                    if !self.stuck_lxs.contains(&lx) {
                        self.lxs[lx.index().unwrap()] = if close {
                            LxReport::Closed
                        } else {
                            LxReport::Open
                        };
                    }
                }
                Command::ResetTvd(tvd) => {
                    self.tvds[tvd.index().unwrap()] = Occupancy::Clear;
                }
                Command::Border { border, message } => {
                    self.border_out[border.index().unwrap()] = Some(message);
                }
            }
        }
    }

    /// Hilfshandlung anfordern (Bediener 1) und bestätigen (Bediener 2).
    pub fn aux(&mut self, op: AuxOperation) -> Vec<Event> {
        let mut events = self.step(&[Request::AuxRequest {
            op,
            operator: OperatorId(1),
        }]);
        let Some(aux) = events.iter().find_map(|e| match e {
            Event::AuxPending { aux, .. } => Some(*aux),
            _ => None,
        }) else {
            return events;
        };
        events.extend(self.step(&[Request::AuxConfirm {
            aux,
            operator: OperatorId(2),
        }]));
        events
    }

    pub fn set_route(&mut self, name: &str) -> Vec<Event> {
        let r = self.route(name);
        self.step(&[Request::SetRoute(r)])
    }

    pub fn cancel_route(&mut self, name: &str) -> Vec<Event> {
        let r = self.route(name);
        self.step(&[Request::CancelRoute(r)])
    }

    /// Fahrstraße einstellen und warten, bis das Signal Fahrt zeigt.
    pub fn clear_route(&mut self, name: &str) {
        let events = self.set_route(name);
        assert!(
            events.iter().any(|e| matches!(e, Event::Accepted(_))),
            "{name} nicht angenommen: {events:?}"
        );
        self.run(3);
        assert_eq!(self.phase(name), RoutePhase::Proceed, "{name}");
    }

    pub fn position(&self, name: &str) -> PointPosition {
        self.points[self.point(name).index().unwrap()]
            .position()
            .unwrap()
    }
}

pub fn rejected(events: &[Event]) -> Option<Rejection> {
    events.iter().find_map(|e| match e {
        Event::Rejected(_, r) => Some(*r),
        _ => None,
    })
}

pub fn accepted(events: &[Event]) -> bool {
    events.iter().any(|e| matches!(e, Event::Accepted(_)))
}
