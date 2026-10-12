//! Betriebsführung an der Nordbahn: Fahrplan, Fahrwege, Zugnummernmeldung, Zuglenkung.

use std::collections::{HashMap, HashSet};

use railit_kernel::{Event, Request, RoutePhase};
use railit_ops::{
    Ops, OpsConfig, OpsEvent, OpsNet, OpsView, RouteNext, Timetable, format_time, parse_time,
};
use railit_plan::PlanView;
use railit_planning::{PlanningConfig, compile_areas};
use railit_topology::Topology;
use railit_types::{BorderId, RouteId, Tick};

const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");
const FAHRPLAN: &str = include_str!("../../../examples/nordbahn-fahrplan.toml");

struct Fixture {
    topology: Topology,
    images: Vec<(String, Vec<u8>)>,
}

impl Fixture {
    fn new() -> Self {
        let topology = railit_import::toml_format::parse(NORDBAHN)
            .unwrap()
            .topology;
        let images = compile_areas(&topology, None, &PlanningConfig::default())
            .unwrap()
            .into_iter()
            .map(|a| (a.name, a.compiled.image))
            .collect();
        Self { topology, images }
    }

    fn plans(&self) -> Vec<(&str, PlanView<'_>)> {
        self.images
            .iter()
            .map(|(n, i)| (n.as_str(), PlanView::parse(i).unwrap()))
            .collect()
    }

    /// Graph; Grenzen werden über ihre Namen gepaart („>Strecke“ in Adorf ↔ „>Adorf“ in
    /// der Strecke).
    fn net(&self) -> OpsNet {
        let plans = self.plans();
        let refs: Vec<(&str, &PlanView<'_>)> = plans.iter().map(|(n, p)| (*n, p)).collect();
        let peer = |area: usize, b: BorderId| {
            let (name, plan) = refs.get(area)?;
            let neighbour = plan.border_name(b)?.strip_prefix('>')?;
            let pa = refs.iter().position(|(n, _)| *n == neighbour)?;
            let pb = refs.get(pa)?.1.find_border(&format!(">{name}"))?;
            Some((pa, pb))
        };
        OpsNet::new(&refs, peer, &self.topology)
    }

    fn route(&self, name: &str) -> (usize, RouteId) {
        let plans = self.plans();
        plans
            .iter()
            .enumerate()
            .find_map(|(a, (_, p))| p.find_route(name).map(|r| (a, r)))
            .unwrap()
    }

    fn border(&self, address: &str) -> (usize, BorderId) {
        let (area, name) = address.split_once('>').unwrap();
        let plans = self.plans();
        let a = plans.iter().position(|(n, _)| *n == area).unwrap();
        (a, plans[a].1.find_border(&format!(">{name}")).unwrap())
    }
}

/// Kerne, deren Fahrstraßen alle in Grundstellung sind; Erlaubnisse nach Vorgabe.
#[derive(Default)]
struct View {
    busy: HashSet<(usize, RouteId)>,
    holding: HashMap<(usize, BorderId), bool>,
}

impl OpsView for View {
    fn route_phase(&self, area: usize, route: RouteId) -> Option<RoutePhase> {
        Some(if self.busy.contains(&(area, route)) {
            RoutePhase::Locked
        } else {
            RoutePhase::Idle
        })
    }

    fn holding(&self, area: usize, border: BorderId) -> Option<bool> {
        Some(self.holding.get(&(area, border)).copied().unwrap_or(false))
    }
}

fn secs(s: u64) -> Tick {
    Tick::from_millis(s * 1000)
}

#[test]
fn times_are_parsed_and_formatted() {
    assert_eq!(parse_time("00:01:30").unwrap(), secs(90));
    assert_eq!(parse_time("1:05").unwrap(), secs(3900));
    assert!(parse_time("00:61:00").is_err());
    assert!(parse_time("abc").is_err());
    assert_eq!(format_time(secs(3725)), "01:02:05");
}

/// REQ-OPS-002: Der Fahrplan wird gegen das Netz geprüft.
#[test]
fn timetable_is_validated_against_the_network() {
    let f = Fixture::new();
    let net = f.net();
    let tt = Timetable::parse(FAHRPLAN).unwrap();
    assert_eq!(tt.trains.len(), 2);
    tt.validate(&net).unwrap();

    let bad = Timetable::parse(
        r#"
        [[train]]
        number = "X1"
        enter = { signal = "Nirgends", at = "00:01:00", length_m = 50, speed_kmh = 40 }
        stops = [
            { station = "Adorf", platform = "9", arr = "00:05:00" },
            { station = "Zstadt", platform = "1", arr = "00:04:00" },
        ]
        [[train]]
        number = "X1"
        "#,
    )
    .unwrap();
    let errors = bad.validate(&net).unwrap_err();
    assert_eq!(errors.len(), 5, "{errors:?}");
    assert!(Timetable::parse("[[train]]\nnumber = \"X\"\nunbekannt = 1").is_err());
    assert!(Timetable::parse("[[train]]\nnumber = \"X\"\nstops = [{ station = \"A\", platform = \"1\", arr = \"25:99\" }]").is_err());
}

/// Fahrwege führen über die Bereichsgrenzen zum Einfahrsignal des Nachbarn.
#[test]
fn paths_cross_area_borders() {
    let f = Fixture::new();
    let net = f.net();
    let route = |name: &str| {
        net.routes()
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .clone()
    };
    assert_eq!(route("AN1->Strecke").next, RouteNext::Signal("Sbk1".into()));
    assert_eq!(route("Sbk1->Bstadt").next, RouteNext::Signal("BA".into()));
    assert_eq!(route("BN1-GE").next, RouteNext::Leave);

    let target = net.platform_signals("Bstadt", "2").unwrap().to_vec();
    let path = net
        .path(
            "AN1",
            |n| matches!(n, RouteNext::Signal(s) if target.contains(s)),
        )
        .unwrap();
    let names: Vec<&str> = path
        .iter()
        .map(|&r| net.route(r).unwrap().name.as_str())
        .collect();
    assert_eq!(names, ["AN1->Strecke", "Sbk1->Bstadt", "BA-BN2"]);
    assert!(net.path("BN1", |n| *n == RouteNext::BufferStop).is_none());
    assert!(net.is_auto("Sbk1") && !net.is_auto("AN1"));
}

/// REQ-OPS-001: Zugnummern wandern mit den Fahrstraßen, auch über Bereichsgrenzen.
#[test]
fn train_numbers_follow_the_routes() {
    let f = Fixture::new();
    let mut ops = Ops::new(f.net(), OpsConfig::default(), Timetable::default()).unwrap();
    assert!(ops.describe("RE9", "Nirgends").is_err());
    ops.describe("RE9", "AA").unwrap();
    assert_eq!(ops.train_at("AA"), Some("RE9"));

    let occupied = |ops: &mut Ops, name: &str| {
        let (area, r) = f.route(name);
        ops.observe(secs(1), area, &Event::RoutePhase(r, RoutePhase::Occupied));
    };
    // Andere Phasen schalten nicht weiter.
    let (area, r) = f.route("AA-AN1");
    ops.observe(secs(1), area, &Event::RoutePhase(r, RoutePhase::Proceed));
    assert_eq!(ops.berth_of("RE9"), Some("AA"));

    for (route, berth) in [
        ("AA-AN1", "AN1"),
        ("AN1->Strecke", "Sbk1"),
        ("Sbk1->Bstadt", "BA"),
        ("BA-BN1", "BN1"),
    ] {
        occupied(&mut ops, route);
        assert_eq!(ops.berth_of("RE9"), Some(berth), "nach {route}");
    }
    occupied(&mut ops, "BN1-GE");
    assert_eq!(ops.berth_of("RE9"), None);
    let events = ops.take_events();
    assert!(events.contains(&OpsEvent::TrainLeft {
        train: "RE9".into(),
        from: "BN1".into()
    }));
    assert!(ops.snapshot().trains[0].left);
    ops.erase("RE9").unwrap();
    assert!(ops.snapshot().trains.is_empty());
}

/// REQ-OPS-003: Die Zuglenkung fährt nicht vor der Abfahrtszeit ab und fordert Fahrstraße
/// und Erlaubnisse entlang des Weges an.
#[test]
fn ars_departs_on_time_and_requests_permissions() {
    let f = Fixture::new();
    let tt = Timetable::parse(
        r#"
        [[train]]
        number = "RE1"
        stops = [
            { station = "Adorf", platform = "1", dep = "00:01:00", dwell_s = 10 },
            { station = "Bstadt", platform = "2", arr = "00:05:00" },
        ]
        "#,
    )
    .unwrap();
    let cfg = OpsConfig {
        ars: true,
        ..OpsConfig::default()
    };
    let mut ops = Ops::new(f.net(), cfg, tt).unwrap();
    ops.describe("RE1", "AN1").unwrap();
    let mut view = View::default();
    view.holding.insert(f.border("Adorf>Strecke"), true);

    assert!(
        ops.plan(secs(5), &view).requests.is_empty(),
        "vor der Abfahrt"
    );
    assert_eq!(ops.snapshot().trains[0].at_stop.as_deref(), Some("Adorf/1"));

    let actions = ops.plan(secs(60), &view);
    let (area, exit) = f.route("AN1->Strecke");
    let (sa, sb) = f.border("Strecke>Bstadt");
    assert!(
        actions
            .requests
            .contains(&(sa, Request::RequestPermission(sb)))
    );
    assert!(actions.requests.contains(&(area, Request::SetRoute(exit))));

    // Keine Wiederholung innerhalb der Wartezeit.
    assert!(ops.plan(secs(61), &view).requests.is_empty());

    // Zurückgehalten: keine Anforderung.
    ops.hold("RE1", true).unwrap();
    assert!(ops.plan(secs(120), &view).requests.is_empty());
    ops.hold("RE1", false).unwrap();
    assert!(!ops.plan(secs(120), &view).requests.is_empty());

    // Abfahrt erfasst, Verspätung gemessen.
    ops.observe(
        secs(130),
        area,
        &Event::RoutePhase(exit, RoutePhase::Occupied),
    );
    let events = ops.take_events();
    assert!(events.iter().any(|e| matches!(
        e,
        OpsEvent::Departed {
            delay_s: Some(70),
            ..
        }
    )));
    assert_eq!(ops.berth_of("RE1"), Some("Sbk1"));
}

/// REQ-OPS-003: Ohne Erlaubnis keine Ausfahrt über die Grenze; der Gleiswechsel des
/// Disponenten ändert das Ziel.
#[test]
fn ars_waits_for_permission_and_follows_platform_changes() {
    let f = Fixture::new();
    let tt = Timetable::parse(
        r#"
        [[train]]
        number = "RE1"
        stops = [{ station = "Bstadt", platform = "2" }]
        "#,
    )
    .unwrap();
    let cfg = OpsConfig {
        ars: true,
        ..OpsConfig::default()
    };
    let mut ops = Ops::new(f.net(), cfg, tt).unwrap();
    assert!(ops.set_platform("RE1", "Bstadt", "7").is_err());
    assert!(ops.set_platform("RE1", "Adorf", "1").is_err());
    ops.set_platform("RE1", "Bstadt", "1").unwrap();
    ops.describe("RE1", "BA").unwrap();

    let mut view = View::default();
    let actions = ops.plan(secs(1), &view);
    let (area, r) = f.route("BA-BN1");
    assert_eq!(actions.requests, [(area, Request::SetRoute(r))]);

    // An der Grenze ohne Erlaubnis nur die Anforderung der Erlaubnis.
    ops.describe("RE1", "AN1").unwrap();
    ops.set_platform("RE1", "Bstadt", "2").unwrap();
    let actions = ops.plan(secs(10), &view);
    let (exit_area, exit) = f.route("AN1->Strecke");
    assert!(
        !actions
            .requests
            .contains(&(exit_area, Request::SetRoute(exit)))
    );
    let (ba, bb) = f.border("Adorf>Strecke");
    assert!(
        actions
            .requests
            .contains(&(ba, Request::RequestPermission(bb)))
    );

    view.holding.insert((ba, bb), true);
    let actions = ops.plan(secs(20), &view);
    assert!(
        actions
            .requests
            .contains(&(exit_area, Request::SetRoute(exit)))
    );

    // Eingestellte Fahrstraße wird nicht erneut angefordert.
    view.busy.insert((exit_area, exit));
    let actions = ops.plan(secs(40), &view);
    assert!(
        !actions
            .requests
            .contains(&(exit_area, Request::SetRoute(exit)))
    );
}

/// REQ-OPS-004: Selbstblock stellt für jeden gemeldeten Zug die Fahrstraße ab dem
/// Selbstblocksignal ein, ohne Fahrplan und ohne Zuglenkung.
#[test]
fn auto_signals_clear_for_any_described_train() {
    let f = Fixture::new();
    let mut ops = Ops::new(f.net(), OpsConfig::default(), Timetable::default()).unwrap();
    ops.describe("Lz1", "Sbk1").unwrap();
    ops.describe("Lz2", "AN1").unwrap();
    let mut view = View::default();
    view.holding.insert(f.border("Strecke>Bstadt"), true);
    let actions = ops.plan(secs(1), &view);
    let (area, r) = f.route("Sbk1->Bstadt");
    assert_eq!(actions.requests, [(area, Request::SetRoute(r))]);

    let off = OpsConfig {
        auto_signals: false,
        ..OpsConfig::default()
    };
    let mut ops = Ops::new(f.net(), off, Timetable::default()).unwrap();
    ops.describe("Lz1", "Sbk1").unwrap();
    assert!(ops.plan(secs(1), &view).requests.is_empty());
}

/// Einsetzen nach Fahrplan.
#[test]
fn trains_are_spawned_on_time() {
    let f = Fixture::new();
    let tt = Timetable::parse(FAHRPLAN).unwrap();
    let mut ops = Ops::new(f.net(), OpsConfig::default(), tt).unwrap();
    let view = View::default();
    let spawns = ops.plan(Tick::ZERO, &view).spawns;
    assert_eq!(spawns.len(), 1);
    assert_eq!(
        (spawns[0].train.as_str(), spawns[0].signal.as_str()),
        ("RE2", "BP1")
    );
    assert!(ops.plan(secs(5), &view).spawns.is_empty());
    let spawns = ops.plan(secs(10), &view).spawns;
    assert_eq!(spawns[0].train, "RE1");
    assert_eq!(ops.train_at("AA"), Some("RE1"));
    ops.spawn_failed("RE1", "besetzt".into());
    assert_eq!(ops.train_at("AA"), None);
}
