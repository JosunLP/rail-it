//! Signalbegriffe am Bahnhof Beispielstadt.
//!
//! ```text
//!        a(Ks)        A(Ks)                        N1(Ks)               F(H/V)        f(H/V)
//!   GW ─ TW ─────────▶ TW1(W1) ┬─ T1 ─▶ ┬ TW2(W2) ◀──────── TE(BÜ1) ◀──── GE
//!                              └─ T2 ─▶ ┘   N2(Ks)
//!                     P1, P2 (H/V) ◀ in Gegenrichtung an den Gleisanfängen
//! ```
//!
//! W1 und W2 zweigen rechts mit 40 km/h ab: A-N1 und N1-GE führen gerade, A-N2, N2-GE,
//! F-P2 und P2-GW abzweigend.

use proptest::prelude::*;
use railit_kernel::{
    Command, Event, Input, Interlocking, LxReport, Occupancy, OwnedState, PointReport, Request,
    RequestId, RoutePhase, SignalReport, VecOutputs,
};
use railit_movement::{DistantAspect, MainAspect, Picture, Pictures};
use railit_plan::{PlanView, RouteEnd};
use railit_rules::GenericRules;
use railit_types::{Duration, LxId, PointId, SignalId, SignalKind, Tick, TvdId};

const EXAMPLE: &str = include_str!("../../../examples/beispielstadt.toml");

fn plan() -> PlanView<'static> {
    let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
    let image = railit_planning::compile(&net, None)
        .unwrap()
        .plan
        .to_image()
        .unwrap();
    PlanView::parse(Box::leak(image.into_boxed_slice())).unwrap()
}

/// Kern mit idealem Feld: Weichen, Signale und Bahnübergänge folgen sofort.
struct Station {
    kernel: Interlocking<'static, GenericRules>,
    state: OwnedState,
    points: Vec<PointReport>,
    tvds: Vec<Occupancy>,
    signals: Vec<SignalReport>,
    lxs: Vec<LxReport>,
    now: Tick,
    next: u64,
    out: VecOutputs,
}

impl Station {
    fn new() -> Self {
        let plan = plan();
        let mut s = Self {
            kernel: Interlocking::new(plan, GenericRules::default()),
            state: OwnedState::new(&plan),
            points: vec![PointReport::Right; plan.point_count()],
            tvds: vec![Occupancy::Clear; plan.tvd_count()],
            signals: vec![SignalReport::Stop; plan.signal_count()],
            lxs: vec![LxReport::Open; plan.lx_count()],
            now: Tick::ZERO,
            next: 0,
            out: VecOutputs::default(),
        };
        s.step(&[]);
        s
    }

    fn plan(&self) -> PlanView<'static> {
        *self.kernel.plan()
    }

    fn step(&mut self, requests: &[Request]) {
        self.now = self.now.checked_add(Duration::from_millis(100)).unwrap();
        let mut inputs = Vec::new();
        for (i, r) in self.points.iter().enumerate() {
            inputs.push(Input::Point {
                point: PointId::from_index(i).unwrap(),
                report: *r,
            });
        }
        for (i, o) in self.tvds.iter().enumerate() {
            inputs.push(Input::Tvd {
                tvd: TvdId::from_index(i).unwrap(),
                occupancy: *o,
            });
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
        for r in requests {
            self.next += 1;
            inputs.push(Input::Request {
                id: RequestId(self.next),
                request: *r,
            });
        }
        self.out.clear();
        self.kernel
            .step(&mut self.state.view(), self.now, &inputs, &mut self.out)
            .unwrap();
        for c in &self.out.commands {
            match *c {
                Command::MovePoint { point, position } => {
                    self.points[point.index().unwrap()] = position.into();
                }
                Command::SetSignal { signal, aspect } => {
                    self.signals[signal.index().unwrap()] = match aspect {
                        railit_kernel::Aspect::Stop => SignalReport::Stop,
                        railit_kernel::Aspect::Proceed => SignalReport::Proceed,
                    };
                }
                Command::SetLevelCrossing { lx, close } => {
                    self.lxs[lx.index().unwrap()] = if close {
                        LxReport::Closed
                    } else {
                        LxReport::Open
                    };
                }
                Command::SetSubstitute { .. } | Command::ResetTvd(_) | Command::Border { .. } => {}
            }
        }
    }

    fn set(&mut self, route: &str) {
        let r = self.plan().find_route(route).unwrap();
        self.step(&[Request::SetRoute(r)]);
        assert!(
            self.out
                .events
                .iter()
                .any(|e| matches!(e, Event::Accepted(_))),
            "{route}: {:?}",
            self.out.events
        );
        for _ in 0..3 {
            self.step(&[]);
        }
    }

    fn pictures(&self) -> Pictures<'_, 'static> {
        Pictures::new(
            self.kernel.plan(),
            self.state.signals(),
            self.state.routes(),
        )
    }

    fn picture(&self, signal: &str) -> Picture {
        let id = self.plan().find_signal(signal).unwrap();
        self.pictures().picture(id).unwrap()
    }

    fn shows(&self, signal: &str) -> String {
        self.picture(signal).to_string()
    }
}

#[test]
fn all_stop_at_start() {
    let s = Station::new();
    assert_eq!(s.shows("A"), "Hp0");
    assert_eq!(s.shows("N1"), "Hp0");
    assert_eq!(s.shows("F"), "Hp0");
    assert_eq!(s.shows("P1"), "Hp0");
    assert_eq!(s.shows("a"), "Ks2");
    assert_eq!(s.shows("f"), "Vr0");
}

#[test]
fn ks_entry_straight_then_through() {
    let mut s = Station::new();
    s.set("A-N1");
    // Gerade, N1 zeigt Halt: Ks2 ohne Zs3; Vorsignal a kündigt Fahrt an.
    assert_eq!(s.shows("A"), "Ks2");
    assert_eq!(s.shows("a"), "Ks1");
    s.set("N1-GE");
    // Ausfahrt auf die Strecke (Netzgrenze): Halt erwarten.
    assert_eq!(s.shows("N1"), "Ks2");
    assert_eq!(s.shows("A"), "Ks1");
}

#[test]
fn ks_diverging_entry_shows_zs3_and_is_announced() {
    let mut s = Station::new();
    s.set("A-N2");
    let a = s.picture("A");
    assert_eq!(a.main, Some(MainAspect::Ks2));
    assert_eq!(a.zs3, Some(4));
    assert_eq!(s.shows("A"), "Ks2 Zs3(4)");
    // Vorsignal: Fahrt erwarten mit 40 km/h.
    let dist = s.picture("a");
    assert_eq!(dist.distant, Some(DistantAspect::Ks1));
    assert_eq!(dist.zs3v, Some(4));
    assert!(dist.flashing);
    assert_eq!(s.shows("a"), "Ks1 blinkend Zs3v(4)");
}

#[test]
fn ks_main_announces_restriction_of_next_route() {
    let mut s = Station::new();
    s.set("A-N2");
    s.set("N2-GE");
    // A zeigt selbst schon 40 km/h; N2-GE hat ebenfalls 40: keine weitere Ankündigung.
    assert_eq!(s.shows("A"), "Ks1 Zs3(4)");
    assert_eq!(s.shows("N2"), "Ks2 Zs3(4)");
}

#[test]
fn hv_combined_and_distant() {
    let mut s = Station::new();
    s.set("F-P1");
    // Gerade: Hp1, Vorsignal am Mast: Halt erwarten (P1 zeigt Halt).
    assert_eq!(s.shows("F"), "Hp1 Vr0");
    assert_eq!(s.shows("f"), "Vr1");
    s.set("P1-GW");
    assert_eq!(s.shows("P1"), "Hp1");
    assert_eq!(s.shows("F"), "Hp1 Vr1");
}

#[test]
fn hv_diverging_is_hp2_and_vr2() {
    let mut s = Station::new();
    s.set("F-P2");
    assert_eq!(s.shows("F"), "Hp2 Vr0");
    assert_eq!(s.shows("f"), "Vr2");
    s.set("P2-GW");
    assert_eq!(s.shows("P2"), "Hp2");
    assert_eq!(s.shows("F"), "Hp2 Vr2");
}

#[test]
fn signal_drop_turns_announcement_back() {
    let mut s = Station::new();
    s.set("A-N1");
    assert_eq!(s.shows("a"), "Ks1");
    let t1 = s.plan().find_tvd("T1").unwrap();
    s.tvds[t1.index().unwrap()] = Occupancy::Occupied;
    s.step(&[]);
    assert_eq!(s.shows("A"), "Hp0");
    assert_eq!(s.shows("a"), "Ks2");
}

#[test]
fn display_of_dark_and_substitute() {
    assert_eq!(Picture::default().to_string(), "dunkel");
    let zs1 = Picture {
        main: Some(MainAspect::Hp0),
        zs1: true,
        ..Picture::default()
    };
    assert_eq!(zs1.to_string(), "Hp0 Zs1");
    assert!(zs1.permits_passing());
}

/// Prüft die Konsistenz aller Signalbilder mit dem Kernzustand.
fn check_consistency(s: &Station) {
    let plan = s.plan();
    let pictures = s.pictures();
    for (i, state) in s.state.signals().iter().enumerate() {
        let id = SignalId::from_index(i).unwrap();
        let picture = pictures.picture(id).unwrap();
        let kind = plan.signal_kind(id).unwrap();
        let proceed = state.commanded() == railit_kernel::Aspect::Proceed;
        match kind {
            SignalKind::Main | SignalKind::Combined => {
                // Fahrtbegriff genau dann, wenn der Kern Fahrt befohlen hat.
                assert_eq!(
                    picture.main != Some(MainAspect::Hp0),
                    proceed,
                    "{id}: {picture}"
                );
                if proceed {
                    let route = plan
                        .routes()
                        .find(|r| {
                            r.start() == id
                                && s.state.routes()[r.id().index().unwrap()].phase()
                                    == RoutePhase::Proceed
                        })
                        .unwrap();
                    let next_proceed = match route.end() {
                        RouteEnd::Signal(n) => {
                            s.state.signals()[n.index().unwrap()].commanded()
                                == railit_kernel::Aspect::Proceed
                        }
                        _ => false,
                    };
                    // Ks1 bzw. Vr1/Vr2 nur, wenn das nächste Hauptsignal Fahrt zeigt.
                    if picture.main == Some(MainAspect::Ks1) {
                        assert!(next_proceed, "{id}: {picture}");
                    }
                    if picture.main == Some(MainAspect::Ks2) {
                        assert!(!next_proceed, "{id}: {picture}");
                    }
                    if let Some(d) = picture.distant {
                        assert_eq!(d.expects_proceed(), next_proceed, "{id}: {picture}");
                    }
                    // Zs3 nie höher als die Fahrstraße erlaubt.
                    if let (Some(v), Some(limit)) = (picture.zs3, route.speed()) {
                        assert!(u16::from(v) * 10 <= limit.as_kmh(), "{id}: {picture}");
                    }
                    if route.speed().is_some() {
                        assert!(
                            picture.zs3.is_some() || picture.main == Some(MainAspect::Hp2),
                            "{id}: Beschränkung nicht angezeigt: {picture}"
                        );
                    }
                }
            }
            SignalKind::Distant => {
                let main = plan.signal_announces(id).unwrap();
                let main_proceed = s.state.signals()[main.index().unwrap()].commanded()
                    == railit_kernel::Aspect::Proceed;
                let d = picture.distant.unwrap();
                assert_eq!(d.expects_proceed(), main_proceed, "{id}: {picture}");
            }
            SignalKind::Shunting | SignalKind::EtcsStopMarker => {}
        }
    }
}

proptest! {
    /// REQ-MOV-002: Vorsignale und Vorsignalschirme kündigen nie eine Fahrt an, die das
    /// nächste Hauptsignal nicht zeigt; Hauptsignale zeigen nie einen Fahrtbegriff ohne
    /// Fahrt des Kerns.
    #[test]
    fn pictures_are_consistent(
        actions in prop::collection::vec((0u32..3, 0usize..8, 0usize..6, any::<bool>()), 1..80)
    ) {
        let mut s = Station::new();
        for (kind, route, tvd, occupied) in actions {
            let r = railit_plan::RouteId::from_index(route).unwrap();
            match kind {
                0 => s.step(&[Request::SetRoute(r)]),
                1 => s.step(&[Request::CancelRoute(r)]),
                _ => {
                    s.tvds[tvd] = if occupied { Occupancy::Occupied } else { Occupancy::Clear };
                    s.step(&[]);
                }
            }
            check_consistency(&s);
            s.step(&[]);
            check_consistency(&s);
        }
    }
}

// ── ETCS Level 2 (REQ-MOV-004) ──────────────────────────────────────────────────────────

impl Station {
    fn ma(&self, signal: &str, passed: bool) -> Option<railit_movement::MovementAuthority> {
        let id = self.plan().find_signal(signal).unwrap();
        railit_movement::authority(
            self.kernel.plan(),
            self.state.signals(),
            self.state.routes(),
            id,
            passed,
        )
    }

    fn route_names(&self, ma: &railit_movement::MovementAuthority) -> Vec<String> {
        ma.sections()
            .iter()
            .map(|s| self.plan().route_name(s.route).unwrap().to_string())
            .collect()
    }
}

#[test]
fn etcs_authority_follows_the_routes_with_proceed() {
    use railit_movement::MaEnd;
    use railit_types::{Length, Speed};
    let mut s = Station::new();
    assert!(
        s.ma("A", false).is_none(),
        "ohne Fahrstraße keine Fahrerlaubnis"
    );

    s.set("A-N1");
    let ma = s.ma("A", false).unwrap();
    let a_n1 = s
        .plan()
        .route(s.plan().find_route("A-N1").unwrap())
        .unwrap();
    assert_eq!(s.route_names(&ma), ["A-N1"]);
    assert_eq!(ma.end(), MaEnd::Signal(s.plan().find_signal("N1").unwrap()));
    assert_eq!(ma.eoa(), a_n1.length());
    assert!(
        ma.overlap().as_mm() > 0,
        "Gefahrpunkt hinter dem Zielsignal"
    );
    assert_eq!(
        ma.danger_point().as_mm(),
        ma.eoa().as_mm() + ma.overlap().as_mm()
    );
    assert_eq!(ma.speed_at(Length::from_mm(0)), None);
    assert_eq!(
        ma.speed_at(ma.eoa()),
        Some(Speed::from_kmh(0)),
        "hinter dem EoA"
    );

    // Verlängerung bis zur Netzgrenze.
    s.set("N1-GE");
    let ma = s.ma("A", false).unwrap();
    assert_eq!(s.route_names(&ma), ["A-N1", "N1-GE"]);
    assert_eq!(ma.end(), MaEnd::Boundary);
    assert_eq!(ma.overlap().as_mm(), 0);

    // Rücknahme verkürzt sofort.
    let r = s.plan().find_route("N1-GE").unwrap();
    s.step(&[Request::CancelRoute(r)]);
    let ma = s.ma("A", false).unwrap();
    assert_eq!(s.route_names(&ma), ["A-N1"]);
}

#[test]
fn etcs_authority_carries_speed_and_survives_passing() {
    use railit_types::{Length, Speed};
    let mut s = Station::new();
    s.set("A-N2");
    let ma = s.ma("A", false).unwrap();
    assert_eq!(ma.speed_at(Length::from_mm(1)), Some(Speed::from_kmh(40)));

    // Der Zug fährt ein: A fällt auf Halt, die Fahrerlaubnis gilt für den Zug weiter.
    let tw1 = s.plan().find_tvd("TW1").unwrap();
    s.tvds[tw1.index().unwrap()] = Occupancy::Occupied;
    s.step(&[]);
    s.step(&[]);
    assert_eq!(
        s.state.routes()[s.plan().find_route("A-N2").unwrap().index().unwrap()].phase(),
        RoutePhase::Occupied
    );
    assert!(
        s.ma("A", false).is_none(),
        "für einen neuen Zug keine Fahrt"
    );
    let ma = s.ma("A", true).unwrap();
    assert_eq!(s.route_names(&ma), ["A-N2"]);
}
