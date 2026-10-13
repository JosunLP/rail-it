//! Fuzz-Ziele von rail-it (Konzept, Abschnitt 16.2).
//!
//! Jede Funktion nimmt beliebige Bytes und darf weder in einen Panikpfad laufen noch hängen.
//! `cargo fuzz` (Verzeichnis `fuzz/`, Nightly) ruft sie mit libFuzzer auf; die Tests dieses
//! Crates rufen sie auf stabilem Rust mit einem Startkorpus und Zufallseingaben auf.
//!
//! | Ziel | Prüft |
//! |------|-------|
//! | [`plan_image`] | Plan-Image: Strukturprüfung, alle Zugriffe, Kern, Signalbilder, ETCS-Fahrerlaubnis auf jedem angenommenen Plan |
//! | [`toml_network`], [`railml_network`] | Import, Validierung und Projektierung jedes angenommenen Netzes |
//! | [`timetable`] | Fahrplan |
//! | [`journal`] | Prüfung der Hashkette eines Journals |
//! | [`rasta`] | RaSTA-Verbindung nach dem Aufbau unter beliebigen PDUs |
//! | [`sci`] | EULYNX-SCI-Telegramme (Rundreise) |
//! | [`node_wire`] | Binärprotokoll und Knoten des sicheren Rechners |
//! | [`mirror_state`] | Spiegel: beliebige übertragene Zustandsteile, danach Kernzyklen |

use railit_kernel::{
    Input, Interlocking, LxReport, Occupancy, OwnedState, PointReport, Request, RequestId,
    SignalReport, VecOutputs,
};
use railit_plan::PlanView;
use railit_rules::GenericRules;
use railit_types::{BorderId, LxId, PointId, RouteId, SignalId, Tick, TvdId};
use sha2::{Digest, Sha256};

/// Höchstzahl der Elemente je Art, die das Ziel [`plan_image`] noch durchrechnet.
const MAX_ELEMENTS: usize = 256;

/// Trägt den SHA-256 neu ein, damit veränderte Images die Strukturprüfung erreichen.
fn rehash(image: &mut [u8]) {
    let Some(len) = image.get(8..12) else {
        return;
    };
    let body_len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
    let Some(body) = image.get(..body_len) else {
        return;
    };
    let digest = Sha256::digest(body);
    if let Some(slot) = image.get_mut(body_len..body_len.saturating_add(32)) {
        slot.copy_from_slice(&digest);
    }
}

/// Plan-Image: Bytes werden (mit neu berechnetem Hash) geprüft; jeder angenommene Plan wird
/// vollständig gelesen und mit Kern, Signalbildern und ETCS-Fahrerlaubnis durchgerechnet.
/// Gibt zurück, ob der Plan angenommen wurde.
pub fn plan_image(data: &[u8]) -> bool {
    let mut image = data.to_vec();
    rehash(&mut image);
    let Ok(plan) = PlanView::parse(&image) else {
        return false;
    };
    if [
        plan.point_count(),
        plan.signal_count(),
        plan.tvd_count(),
        plan.route_count(),
        plan.lx_count(),
        plan.border_count(),
    ]
    .iter()
    .any(|n| *n > MAX_ELEMENTS)
    {
        return true;
    }
    read_all(&plan);
    run_kernel(&plan, data);
    true
}

fn read_all(plan: &PlanView<'_>) {
    let _ = plan.network_name();
    for i in 0..plan.point_count() {
        let p = PointId::from_index(i);
        let _ = p.map(|p| (plan.point_name(p), plan.point_tvd(p)));
    }
    for i in 0..plan.signal_count() {
        if let Some(s) = SignalId::from_index(i) {
            let _ = (
                plan.signal_name(s),
                plan.signal_kind(s),
                plan.signal_system(s),
                plan.signal_announces(s),
            );
        }
    }
    for i in 0..plan.tvd_count() {
        if let Some(t) = TvdId::from_index(i) {
            let _ = (plan.tvd_name(t), plan.tvd_is_entry(t));
        }
    }
    for i in 0..plan.lx_count() {
        if let Some(l) = LxId::from_index(i) {
            let _ = (plan.lx_name(l), plan.lx_tvd(l));
        }
    }
    for i in 0..plan.border_count() {
        if let Some(b) = BorderId::from_index(i) {
            let _ = (
                plan.border_name(b),
                plan.border_initial(b),
                plan.border_entry_signal(b),
                plan.border_approach_length(b),
                plan.border_entry_overlap(b),
            );
            let _ = plan.border_approach_tvds(b).count();
            let _ = plan.border_approach_points(b).count();
        }
    }
    for r in plan.routes() {
        let _ = (r.id(), r.name(), r.start(), r.end(), r.speed(), r.length());
        let _ = r.overlap_length();
        let _ = r.points().count();
        let _ = r.tvds().count();
        let _ = r.overlap_tvds().count();
        let _ = r.overlap_points().count();
        let _ = r.flank().count();
        let _ = r.level_crossings().count();
        let _ = r.conflicts().count();
    }
}

/// Einige Zyklen Kern auf dem Plan; Meldungen und Anforderungen aus den Bytes.
fn run_kernel(plan: &PlanView<'_>, data: &[u8]) {
    run_kernel_on(plan, OwnedState::new(plan), data);
}

/// Wie [`run_kernel`], aber ab einem gegebenen Zustand.
fn run_kernel_on(plan: &PlanView<'_>, mut state: OwnedState, data: &[u8]) {
    let kernel = Interlocking::new(*plan, GenericRules::default());
    let mut out = VecOutputs::default();
    let mut bytes = data.iter().copied().cycle();
    let mut next = move || bytes.next().unwrap_or(0);
    for cycle in 0..16u64 {
        let mut inputs = Vec::new();
        for i in 0..plan.point_count() {
            let report = match next() % 4 {
                0 => PointReport::Left,
                1 => PointReport::Right,
                2 => PointReport::NoEndPosition,
                _ => PointReport::Disturbed,
            };
            if let Some(point) = PointId::from_index(i) {
                inputs.push(Input::Point { point, report });
            }
        }
        for i in 0..plan.tvd_count() {
            let occupancy = match next() % 3 {
                0 => Occupancy::Clear,
                1 => Occupancy::Occupied,
                _ => Occupancy::Disturbed,
            };
            if let Some(tvd) = TvdId::from_index(i) {
                inputs.push(Input::Tvd { tvd, occupancy });
            }
        }
        for i in 0..plan.signal_count() {
            let report = if next() % 2 == 0 {
                SignalReport::Stop
            } else {
                SignalReport::Proceed
            };
            if let Some(signal) = SignalId::from_index(i) {
                inputs.push(Input::Signal { signal, report });
            }
        }
        for i in 0..plan.lx_count() {
            if let Some(lx) = LxId::from_index(i) {
                inputs.push(Input::LevelCrossing {
                    lx,
                    report: LxReport::Closed,
                });
            }
        }
        let route = usize::from(next());
        if let Some(r) = RouteId::from_index(route % plan.route_count().max(1)) {
            inputs.push(Input::Request {
                id: RequestId(cycle),
                request: Request::SetRoute(r),
            });
        }
        out.clear();
        let now = Tick::from_millis(cycle.saturating_mul(100));
        let _ = kernel.step(&mut state.view(), now, &inputs, &mut out);
        let pictures = railit_movement::Pictures::new(plan, state.signals(), state.routes());
        for i in 0..plan.signal_count() {
            if let Some(s) = SignalId::from_index(i) {
                let _ = pictures.picture(s);
                let _ = railit_movement::authority(plan, state.signals(), state.routes(), s, false);
            }
        }
    }
}

/// Netz im rail-it-TOML; jedes gültige Netz wird projektiert (mit Gegenprüfung).
pub fn toml_network(data: &[u8]) {
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(net) = railit_import::toml_format::parse(text)
    {
        compile(&net.topology);
    }
}

/// Netz in railML 3; jedes gültige Netz wird projektiert (mit Gegenprüfung).
pub fn railml_network(data: &[u8]) {
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(net) = railit_import::railml::parse(text)
    {
        compile(&net.topology);
    }
}

fn compile(net: &railit_topology::Topology) {
    if net.areas().len() == 0 {
        let _ = railit_planning::compile(net, None);
    } else {
        let _ =
            railit_planning::compile_areas(net, None, &railit_planning::PlanningConfig::default());
    }
}

/// Fahrplan.
pub fn timetable(data: &[u8]) {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = railit_ops::Timetable::parse(text);
    }
}

/// Journal: Prüfung der Hashkette.
pub fn journal(data: &[u8]) {
    let _ = railit_journal::verify(data);
}

/// RaSTA nach dem Verbindungsaufbau: Die Bytes werden in PDUs (Längenbyte + Inhalt) zerlegt
/// und zugestellt.
pub fn rasta(data: &[u8]) {
    use railit_rasta::{Config, Connection, MAX_PDU, Role};
    let mut client = Connection::new(Config::new(1, 2, Role::Client));
    let mut server = Connection::new(Config::new(2, 1, Role::Server));
    let mut buf = [0u8; MAX_PDU];
    client.open(0);
    for _ in 0..2 {
        while let Some(n) = client.poll_transmit(&mut buf) {
            server.receive(0, buf.get(..n).unwrap_or(&[]), &mut |_| {});
        }
        while let Some(n) = server.poll_transmit(&mut buf) {
            client.receive(0, buf.get(..n).unwrap_or(&[]), &mut |_| {});
        }
    }
    let mut rest = data;
    let mut now = 0u32;
    while let Some((&len, tail)) = rest.split_first() {
        let n = usize::from(len).min(tail.len());
        let (pdu, tail) = tail.split_at(n);
        now = now.wrapping_add(7);
        server.receive(now, pdu, &mut |_| {});
        let _ = server.tick(now);
        while server.poll_transmit(&mut buf).is_some() {}
        rest = tail;
    }
}

/// EULYNX SCI: Jedes lesbare Telegramm übersteht die Rundreise unverändert.
///
/// # Panics
/// Wenn ein gelesenes Telegramm nach dem Kodieren anders gelesen wird (Fund für den Fuzzer).
pub fn sci(data: &[u8]) {
    use railit_adapters::sci::Telegram;
    if let Ok(t) = Telegram::decode(data)
        && let Ok(raw) = t.encode()
    {
        assert_eq!(Telegram::decode(&raw).ok(), Some(t), "SCI-Rundreise");
    }
}

/// Binärprotokoll des sicheren Rechners: Dekodieren beider Richtungen.
pub fn node_wire(data: &[u8]) {
    use railit_embedded::wire::{HostMsg, NodeMsg, decode};
    let _ = decode::<HostMsg>(data, &mut |_| {});
    let _ = decode::<NodeMsg>(data, &mut |_| {});
}

/// Spiegel des Hosts: beliebige Zustandsteile (postcard) in einen Zustand der Beispielstadt
/// einsetzen, dann Kernzyklen darauf rechnen. Der Kern darf auch mit beliebigem Zustand in
/// keinen Panikpfad laufen (er geht höchstens in den sicheren Zustand).
pub fn mirror_state(data: &[u8]) {
    use std::sync::OnceLock;
    static PLAN: OnceLock<Option<PlanView<'static>>> = OnceLock::new();
    let plan = PLAN.get_or_init(|| {
        let net =
            railit_import::toml_format::parse(include_str!("../../../examples/beispielstadt.toml"))
                .ok()?
                .topology;
        let image = railit_planning::compile(&net, None).ok()?.image;
        PlanView::parse(Box::leak(image.into_boxed_slice())).ok()
    });
    let Some(plan) = plan else {
        return;
    };
    let mut state = OwnedState::new(plan);
    let mut rest = data;
    while let Ok((part, tail)) = postcard::take_from_bytes::<railit_kernel::StatePart>(rest) {
        let _ = state.view().apply(part);
        rest = tail;
    }
    run_kernel_on(plan, state, rest);
}
