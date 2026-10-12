//! Zugnummernmeldung, Zuglenkung und Disposition.

use std::collections::{BTreeMap, HashMap, VecDeque};

use railit_kernel::{Event, Request, RoutePhase};
use railit_types::{BorderId, RouteId, Tick};
use serde::Serialize;

use crate::net::{OpsNet, RouteNext};
use crate::timetable::{Stop, Timetable};

/// Einstellungen der Betriebsführung.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct OpsConfig {
    /// Zuglenkung: Fahrstraßen für Züge mit Fahrplan selbsttätig einstellen.
    pub ars: bool,
    /// Selbstblock: Fahrstraßen ab Selbstblocksignalen für jeden gemeldeten Zug einstellen.
    pub auto_signals: bool,
    /// Wartezeit, bevor eine nicht ausgeführte Anforderung wiederholt wird (ms).
    pub retry_ms: u64,
    /// Mindesthaltezeit an Bahnsteigen (ms), sofern der Fahrplan keine angibt.
    pub min_dwell_ms: u64,
}

impl Default for OpsConfig {
    fn default() -> Self {
        Self {
            ars: false,
            auto_signals: true,
            retry_ms: 5_000,
            min_dwell_ms: 20_000,
        }
    }
}

/// Zugriff auf den Zustand der Kerne.
pub trait OpsView {
    /// Phase einer Fahrstraße.
    fn route_phase(&self, area: usize, route: RouteId) -> Option<RoutePhase>;
    /// Hält der Bereich die Erlaubnis an der Grenze?
    fn holding(&self, area: usize, border: BorderId) -> Option<bool>;
}

/// Zug, den die Simulation einsetzen soll.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Spawn {
    /// Zugnummer.
    pub train: String,
    /// Der Zug steht mit der Spitze vor diesem Signal.
    pub signal: String,
    /// Zuglänge in Metern.
    pub length_m: u32,
    /// Geschwindigkeit in km/h.
    pub speed_kmh: u16,
    /// Zug mit ETCS-Fahrzeuggerät.
    pub etcs: bool,
}

/// Ergebnis eines Planungsschritts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpsActions {
    /// Anforderungen an die Kerne: (Bereich, Anforderung).
    pub requests: Vec<(usize, Request)>,
    /// Einzusetzende Züge.
    pub spawns: Vec<Spawn>,
}

/// Ereignis der Betriebsführung.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpsEvent {
    /// Zugnummer an einem Signal eingetragen.
    TrainDescribed {
        /// Zugnummer.
        train: String,
        /// Signal.
        berth: String,
    },
    /// Zugnummer weitergeschaltet.
    TrainMoved {
        /// Zugnummer.
        train: String,
        /// Bisheriges Signal.
        from: String,
        /// Neues Signal (oder `"<Fahrstraße>/Prellbock"`).
        to: String,
    },
    /// Zug hat das Netz verlassen.
    TrainLeft {
        /// Zugnummer.
        train: String,
        /// Letztes Signal.
        from: String,
    },
    /// Zugnummer gelöscht.
    TrainErased {
        /// Zugnummer.
        train: String,
    },
    /// Ankunft an einem Halt.
    Arrived {
        /// Zugnummer.
        train: String,
        /// Betriebsstelle.
        station: String,
        /// Bahnsteiggleis.
        platform: String,
        /// Verspätung gegenüber der planmäßigen Ankunft in Sekunden.
        delay_s: Option<i64>,
    },
    /// Abfahrt von einem Halt.
    Departed {
        /// Zugnummer.
        train: String,
        /// Betriebsstelle.
        station: String,
        /// Bahnsteiggleis.
        platform: String,
        /// Verspätung gegenüber der planmäßigen Abfahrt in Sekunden.
        delay_s: Option<i64>,
    },
    /// Die Zuglenkung hat eine Fahrstraße angefordert.
    RouteRequested {
        /// Zugnummer.
        train: String,
        /// Fahrstraße.
        route: String,
    },
    /// Die Zuglenkung hat eine Erlaubnis angefordert.
    PermissionRequested {
        /// Zugnummer.
        train: String,
        /// Grenze (`Bereich>Nachbar`).
        border: String,
    },
    /// Zug in die Simulation eingesetzt.
    Spawned {
        /// Zugnummer.
        train: String,
        /// Signal.
        signal: String,
    },
    /// Einsetzen fehlgeschlagen.
    SpawnFailed {
        /// Zugnummer.
        train: String,
        /// Grund.
        reason: String,
    },
}

impl OpsEvent {
    /// Art des Ereignisses wie im Feld `type` der JSON-Darstellung.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::TrainDescribed { .. } => "train_described",
            Self::TrainMoved { .. } => "train_moved",
            Self::TrainLeft { .. } => "train_left",
            Self::TrainErased { .. } => "train_erased",
            Self::Arrived { .. } => "arrived",
            Self::Departed { .. } => "departed",
            Self::RouteRequested { .. } => "route_requested",
            Self::PermissionRequested { .. } => "permission_requested",
            Self::Spawned { .. } => "spawned",
            Self::SpawnFailed { .. } => "spawn_failed",
        }
    }
}

/// Zustand eines Zuges in der Betriebsführung.
#[derive(Clone, Debug, Default)]
// Unabhängige Merkmale eines Zuges.
#[allow(clippy::struct_excessive_bools)]
struct TrainState {
    berth: Option<String>,
    /// Über eine Fahrstraße an das Signal gekommen (dann zählt die Ankunft erst mit dem
    /// Erreichen des Ziels).
    by_route: bool,
    next_stop: usize,
    arrived: Option<Tick>,
    hold: bool,
    extra_delay_ms: u64,
    platforms: BTreeMap<usize, String>,
    delay_s: Option<i64>,
    left: bool,
    spawned: bool,
}

/// Zug im Zustandsbild der Betriebsführung.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TrainSnapshot {
    /// Zugnummer.
    pub number: String,
    /// Signal, an dem die Nummer steht.
    pub berth: Option<String>,
    /// Hat das Netz verlassen.
    pub left: bool,
    /// Hat einen Fahrplan.
    pub timetabled: bool,
    /// Von der Zuglenkung zurückgehalten.
    pub hold: bool,
    /// Nächster Halt (`Betriebsstelle/Gleis`).
    pub next_stop: Option<String>,
    /// Steht am Halt (`Betriebsstelle/Gleis`).
    pub at_stop: Option<String>,
    /// Zuletzt gemessene Verspätung in Sekunden.
    pub delay_s: Option<i64>,
    /// Vom Disponenten verfügte zusätzliche Wartezeit in Sekunden.
    pub extra_delay_s: u64,
}

/// Zustandsbild der Betriebsführung.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct OpsSnapshot {
    /// Zuglenkung eingeschaltet.
    pub ars: bool,
    /// Name des Fahrplans.
    pub timetable: String,
    /// Züge.
    pub trains: Vec<TrainSnapshot>,
}

/// Betriebsführung: Zugnummernmeldung, Zuglenkung und Disposition (REQ-OPS-001 … 005).
///
/// Sie stellt nur **Anforderungen**; ob eine Fahrstraße eingestellt oder eine Erlaubnis
/// abgegeben wird, entscheidet allein der Kern.
#[derive(Clone, Debug)]
pub struct Ops {
    net: OpsNet,
    cfg: OpsConfig,
    timetable: Timetable,
    trains: BTreeMap<String, TrainState>,
    berths: BTreeMap<String, VecDeque<String>>,
    route_requests: HashMap<usize, (Tick, String)>,
    permission_requests: HashMap<(usize, BorderId), Tick>,
    events: Vec<OpsEvent>,
}

fn since_ms(now: Tick, then: Tick) -> u64 {
    now.saturating_since(then).as_millis()
}

fn delay_s(now: Tick, planned: Tick) -> i64 {
    let (now, planned) = (now.as_millis(), planned.as_millis());
    let s = |ms: u64| i64::try_from(ms / 1000).unwrap_or(i64::MAX);
    if now >= planned {
        s(now - planned)
    } else {
        -s(planned - now)
    }
}

fn add_ms(t: Tick, ms: u64) -> Tick {
    Tick::from_millis(t.as_millis().saturating_add(ms))
}

impl Ops {
    /// Betriebsführung für ein Netz mit Fahrplan.
    ///
    /// # Errors
    /// Abweichungen des Fahrplans vom Netz.
    pub fn new(net: OpsNet, cfg: OpsConfig, timetable: Timetable) -> Result<Self, Vec<String>> {
        timetable.validate(&net)?;
        Ok(Self {
            net,
            cfg,
            timetable,
            trains: BTreeMap::new(),
            berths: BTreeMap::new(),
            route_requests: HashMap::new(),
            permission_requests: HashMap::new(),
            events: Vec::new(),
        })
    }

    /// Fahrstraßengraph.
    pub fn net(&self) -> &OpsNet {
        &self.net
    }

    /// Einstellungen.
    pub fn config(&self) -> OpsConfig {
        self.cfg
    }

    /// Zuglenkung ein- oder ausschalten.
    pub fn set_ars(&mut self, on: bool) {
        self.cfg.ars = on;
    }

    /// Fahrplan.
    pub fn timetable(&self) -> &Timetable {
        &self.timetable
    }

    /// Ereignisse seit dem letzten Aufruf.
    pub fn take_events(&mut self) -> Vec<OpsEvent> {
        std::mem::take(&mut self.events)
    }

    /// Stellt Ereignisse zurück, damit sie mit dem nächsten Abruf ausgeliefert werden.
    pub fn requeue_events(&mut self, mut events: Vec<OpsEvent>) {
        events.append(&mut self.events);
        self.events = events;
    }

    /// Zugnummer an einem Signal (`None`: keine).
    pub fn train_at(&self, signal: &str) -> Option<&str> {
        self.berths
            .get(signal)
            .and_then(VecDeque::front)
            .map(String::as_str)
    }

    /// Signal, an dem die Zugnummer steht.
    pub fn berth_of(&self, train: &str) -> Option<&str> {
        self.trains.get(train)?.berth.as_deref()
    }

    /// Zustand eines gemeldeten Zuges oder eines Zuges aus dem Fahrplan (auch vor dem
    /// Einsetzen, damit der Disponent vorab verfügen kann).
    fn train_mut(&mut self, train: &str) -> Result<&mut TrainState, String> {
        if !self.trains.contains_key(train) && self.timetable.train(train).is_some() {
            self.trains.insert(train.to_string(), TrainState::default());
        }
        self.trains
            .get_mut(train)
            .ok_or_else(|| format!("Zug „{train}“ nicht bekannt"))
    }

    fn unplace(&mut self, train: &str) {
        if let Some(berth) = self.trains.get(train).and_then(|t| t.berth.clone())
            && let Some(q) = self.berths.get_mut(&berth)
        {
            q.retain(|t| t != train);
            if q.is_empty() {
                self.berths.remove(&berth);
            }
        }
    }

    fn place(&mut self, train: &str, berth: &str, by_route: bool) {
        self.berths
            .entry(berth.to_string())
            .or_default()
            .push_back(train.to_string());
        let t = self.trains.entry(train.to_string()).or_default();
        t.berth = Some(berth.to_string());
        t.by_route = by_route;
        t.left = false;
        t.arrived = None;
    }

    /// Zugnummer an einem Signal eintragen (Zugnummernmeldung, REQ-OPS-001).
    ///
    /// # Errors
    /// Signal unbekannt.
    pub fn describe(&mut self, train: &str, signal: &str) -> Result<(), String> {
        if !self.net.has_signal(signal) {
            return Err(format!("Signal „{signal}“ nicht gefunden"));
        }
        if train.is_empty() {
            return Err("Zugnummer fehlt".into());
        }
        self.unplace(train);
        self.place(train, signal, false);
        self.events.push(OpsEvent::TrainDescribed {
            train: train.to_string(),
            berth: signal.to_string(),
        });
        Ok(())
    }

    /// Zugnummer löschen.
    ///
    /// # Errors
    /// Zug unbekannt.
    pub fn erase(&mut self, train: &str) -> Result<(), String> {
        if !self.trains.contains_key(train) {
            return Err(format!("Zug „{train}“ nicht bekannt"));
        }
        self.unplace(train);
        self.trains.remove(train);
        self.events.push(OpsEvent::TrainErased {
            train: train.to_string(),
        });
        Ok(())
    }

    /// Zug von der Zuglenkung zurückhalten oder freigeben (REQ-OPS-005).
    ///
    /// # Errors
    /// Zug unbekannt.
    pub fn hold(&mut self, train: &str, on: bool) -> Result<(), String> {
        self.train_mut(train)?.hold = on;
        Ok(())
    }

    /// Zusätzliche Wartezeit vor den folgenden Abfahrten verfügen (REQ-OPS-005).
    ///
    /// # Errors
    /// Zug unbekannt.
    pub fn delay(&mut self, train: &str, seconds: u32) -> Result<(), String> {
        let t = self.train_mut(train)?;
        t.extra_delay_ms = t
            .extra_delay_ms
            .saturating_add(u64::from(seconds).saturating_mul(1000));
        Ok(())
    }

    /// Gleiswechsel: Den nächsten Halt an `station` an ein anderes Bahnsteiggleis legen
    /// (REQ-OPS-005).
    ///
    /// # Errors
    /// Zug ohne Fahrplan, kein offener Halt an der Betriebsstelle oder Gleis unbekannt.
    pub fn set_platform(
        &mut self,
        train: &str,
        station: &str,
        platform: &str,
    ) -> Result<(), String> {
        if self.net.platform_signals(station, platform).is_none() {
            return Err(format!("Bahnsteig „{platform}“ in „{station}“ unbekannt"));
        }
        let run = self
            .timetable
            .train(train)
            .ok_or_else(|| format!("Zug „{train}“ hat keinen Fahrplan"))?;
        let next = self.trains.get(train).map_or(0, |t| t.next_stop);
        let index = run
            .stops
            .iter()
            .enumerate()
            .skip(next)
            .find(|(_, s)| s.station == station)
            .map(|(i, _)| i)
            .ok_or_else(|| format!("Zug „{train}“ hält nicht mehr in „{station}“"))?;
        self.trains
            .entry(train.to_string())
            .or_default()
            .platforms
            .insert(index, platform.to_string());
        Ok(())
    }

    /// Halt `index` des Zuges mit verfügtem Gleis.
    fn stop(&self, train: &str, index: usize) -> Option<(Stop, Vec<String>)> {
        let mut stop = self.timetable.train(train)?.stops.get(index)?.clone();
        if let Some(p) = self.trains.get(train).and_then(|t| t.platforms.get(&index)) {
            stop.platform.clone_from(p);
        }
        let signals = self.net.platform_signals(&stop.station, &stop.platform)?;
        Some((stop, signals.to_vec()))
    }

    /// Verarbeitet ein Ereignis eines Kerns: Zugnummern weiterschalten, Ankünfte erfassen.
    pub fn observe(&mut self, now: Tick, area: usize, event: &Event) {
        let Event::RoutePhase(id, phase) = *event else {
            return;
        };
        let Some(route) = self
            .net
            .find_kernel_route(area, id)
            .and_then(|r| self.net.route(r))
            .cloned()
        else {
            return;
        };
        match phase {
            RoutePhase::Occupied => self.advance(now, &route.start, &route.name, &route.next),
            RoutePhase::Arrived => {
                if let RouteNext::Signal(s) = &route.next {
                    self.arrive(now, s);
                }
            }
            _ => {}
        }
    }

    /// Ein Zug fährt in eine Fahrstraße ein: Nummer zum Ziel weiterschalten.
    fn advance(&mut self, now: Tick, start: &str, route: &str, next: &RouteNext) {
        let Some(train) = self.berths.get_mut(start).and_then(VecDeque::pop_front) else {
            return;
        };
        if self.berths.get(start).is_some_and(VecDeque::is_empty) {
            self.berths.remove(start);
        }
        // Abfahrt vom nächsten Halt?
        let index = self.trains.get(&train).map_or(0, |t| t.next_stop);
        if let Some((stop, signals)) = self.stop(&train, index)
            && signals.iter().any(|s| s == start)
        {
            let extra = self.trains.get(&train).map_or(0, |t| t.extra_delay_ms);
            let delay = stop.dep.map(|d| delay_s(now, add_ms(d, extra)));
            if let Some(t) = self.trains.get_mut(&train) {
                t.next_stop = index.saturating_add(1);
                t.delay_s = delay.or(t.delay_s);
            }
            self.events.push(OpsEvent::Departed {
                train: train.clone(),
                station: stop.station,
                platform: stop.platform,
                delay_s: delay,
            });
        }
        match next {
            RouteNext::Signal(to) => {
                self.place(&train, to, true);
                self.events.push(OpsEvent::TrainMoved {
                    train,
                    from: start.to_string(),
                    to: to.clone(),
                });
            }
            RouteNext::BufferStop => {
                let to = format!("{route}/Prellbock");
                self.place(&train, &to, true);
                self.events.push(OpsEvent::TrainMoved {
                    train,
                    from: start.to_string(),
                    to,
                });
            }
            RouteNext::Leave => {
                if let Some(t) = self.trains.get_mut(&train) {
                    t.berth = None;
                    t.left = true;
                }
                self.events.push(OpsEvent::TrainLeft {
                    train,
                    from: start.to_string(),
                });
            }
        }
    }

    /// Ein Zug hat das Ziel seiner Fahrstraße erreicht.
    fn arrive(&mut self, now: Tick, signal: &str) {
        let Some(train) = self.train_at(signal).map(str::to_string) else {
            return;
        };
        let index = self.trains.get(&train).map_or(0, |t| t.next_stop);
        if let Some((stop, signals)) = self.stop(&train, index)
            && signals.iter().any(|s| s == signal)
        {
            self.mark_arrived(now, &train, stop);
        }
    }

    fn mark_arrived(&mut self, now: Tick, train: &str, stop: Stop) {
        let delay = stop.arr.map(|a| delay_s(now, a));
        if let Some(t) = self.trains.get_mut(train) {
            if t.arrived.is_some() {
                return;
            }
            t.arrived = Some(now);
            t.delay_s = delay.or(t.delay_s);
        }
        self.events.push(OpsEvent::Arrived {
            train: train.to_string(),
            station: stop.station,
            platform: stop.platform,
            delay_s: delay,
        });
    }

    /// Meldung der Simulation, dass ein Zug nicht eingesetzt werden konnte.
    pub fn spawn_failed(&mut self, train: &str, reason: String) {
        self.unplace(train);
        if let Some(t) = self.trains.get_mut(train) {
            t.berth = None;
        }
        self.events.push(OpsEvent::SpawnFailed {
            train: train.to_string(),
            reason,
        });
    }

    /// Ein Planungsschritt: Züge einsetzen, Fahrstraßen und Erlaubnisse anfordern
    /// (Zuglenkung REQ-OPS-003, Selbstblock REQ-OPS-004).
    pub fn plan(&mut self, now: Tick, view: &dyn OpsView) -> OpsActions {
        let mut actions = OpsActions::default();
        self.spawn_due(now, &mut actions);
        if !(self.cfg.ars || self.cfg.auto_signals) {
            return actions;
        }
        let fronts: Vec<(String, String)> = self
            .berths
            .iter()
            .filter_map(|(s, q)| q.front().map(|t| (s.clone(), t.clone())))
            .collect();
        for (signal, train) in fronts {
            if let Some(path) = self.desired_path(now, &signal, &train) {
                self.request_path(now, &train, &path, view, &mut actions);
            }
        }
        actions
    }

    fn spawn_due(&mut self, now: Tick, actions: &mut OpsActions) {
        let due: Vec<_> = self
            .timetable
            .trains
            .iter()
            .filter_map(|run| {
                let e = run.enter.as_ref()?;
                let spawned = self.trains.get(&run.number).is_some_and(|t| t.spawned);
                (!spawned && now >= e.at).then(|| Spawn {
                    train: run.number.clone(),
                    signal: e.signal.clone(),
                    length_m: e.length_m,
                    speed_kmh: e.speed_kmh,
                    etcs: e.etcs,
                })
            })
            .collect();
        for spawn in due {
            self.unplace(&spawn.train);
            self.place(&spawn.train, &spawn.signal, false);
            if let Some(t) = self.trains.get_mut(&spawn.train) {
                t.spawned = true;
            }
            self.events.push(OpsEvent::Spawned {
                train: spawn.train.clone(),
                signal: spawn.signal.clone(),
            });
            actions.spawns.push(spawn);
        }
    }

    /// Fahrweg, den der Zug am Signal als Nächstes befahren soll.
    fn desired_path(&mut self, now: Tick, signal: &str, train: &str) -> Option<Vec<usize>> {
        let state = self.trains.get(train)?;
        if state.hold {
            return None;
        }
        let run = if self.cfg.ars {
            self.timetable.train(train)
        } else {
            None
        };
        let Some(run) = run else {
            // Selbstblock: die einzige Fahrstraße ab dem Signal.
            let routes = self.net.routes_from(signal);
            return (self.cfg.auto_signals && self.net.is_auto(signal) && routes.len() == 1)
                .then(|| routes.to_vec());
        };
        let exit = run.exit;
        let (by_route, arrived, extra) = (state.by_route, state.arrived, state.extra_delay_ms);
        let mut target = state.next_stop;
        if let Some((stop, signals)) = self.stop(train, target)
            && signals.iter().any(|s| s == signal)
        {
            // Am Bahnsteig des nächsten Halts: Haltezeit und Abfahrtszeit abwarten.
            let arrived = match arrived {
                Some(t) => t,
                // Ohne Fahrstraße hierher gekommen (eingesetzt oder eingetragen).
                None if !by_route => {
                    self.mark_arrived(now, train, stop.clone());
                    now
                }
                None => return None,
            };
            let dwell = stop.dwell_ms.unwrap_or(self.cfg.min_dwell_ms);
            if since_ms(now, arrived) < dwell || stop.dep.is_some_and(|d| now < add_ms(d, extra)) {
                return None;
            }
            target = target.saturating_add(1);
        }
        if let Some((_, signals)) = self.stop(train, target) {
            self.net.path(
                signal,
                |n| matches!(n, RouteNext::Signal(s) if signals.contains(s)),
            )
        } else if exit {
            self.net.path(signal, |n| *n == RouteNext::Leave)
        } else {
            None
        }
    }

    fn request_path(
        &mut self,
        now: Tick,
        train: &str,
        path: &[usize],
        view: &dyn OpsView,
        actions: &mut OpsActions,
    ) {
        // Erlaubnisse für alle Grenzen auf dem Weg zum nächsten Ziel anfordern; der Kern
        // gibt sie nur ab, wenn es sicher ist.
        let permission_retry = self.cfg.retry_ms.saturating_mul(5);
        for &r in path {
            let Some(route) = self.net.route(r) else {
                continue;
            };
            let Some(b) = route.border else {
                continue;
            };
            if view.holding(route.area, b) != Some(false) {
                continue;
            }
            let key = (route.area, b);
            if self
                .permission_requests
                .get(&key)
                .is_some_and(|&t| since_ms(now, t) < permission_retry)
            {
                continue;
            }
            self.permission_requests.insert(key, now);
            actions
                .requests
                .push((route.area, Request::RequestPermission(b)));
            self.events.push(OpsEvent::PermissionRequested {
                train: train.to_string(),
                border: self.net.border_address(route.area, b),
            });
        }
        let Some(&first) = path.first() else {
            return;
        };
        let Some(route) = self.net.route(first) else {
            return;
        };
        if view.route_phase(route.area, route.id) != Some(RoutePhase::Idle) {
            return;
        }
        if let Some(b) = route.border
            && view.holding(route.area, b) != Some(true)
        {
            return;
        }
        let previous = self.route_requests.get(&first);
        if previous.is_some_and(|(t, _)| since_ms(now, *t) < self.cfg.retry_ms) {
            return;
        }
        let announce = previous.is_none_or(|(_, t)| t != train);
        let (area, id, name) = (route.area, route.id, route.name.clone());
        self.route_requests.insert(first, (now, train.to_string()));
        actions.requests.push((area, Request::SetRoute(id)));
        if announce {
            self.events.push(OpsEvent::RouteRequested {
                train: train.to_string(),
                route: name,
            });
        }
    }

    /// Zustandsbild.
    pub fn snapshot(&self) -> OpsSnapshot {
        let label = |s: &Stop| format!("{}/{}", s.station, s.platform);
        OpsSnapshot {
            ars: self.cfg.ars,
            timetable: self.timetable.name.clone(),
            trains: self
                .trains
                .iter()
                .map(|(number, t)| {
                    let next = self.stop(number, t.next_stop).map(|(s, _)| s);
                    TrainSnapshot {
                        number: number.clone(),
                        berth: t.berth.clone(),
                        left: t.left,
                        timetabled: self.timetable.train(number).is_some(),
                        hold: t.hold,
                        next_stop: next.as_ref().map(label),
                        at_stop: next.as_ref().filter(|_| t.arrived.is_some()).map(label),
                        delay_s: t.delay_s,
                        extra_delay_s: t.extra_delay_ms / 1000,
                    }
                })
                .collect(),
        }
    }
}
