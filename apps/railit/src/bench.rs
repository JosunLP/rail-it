//! `railit bench` – Leistungsmessung gegen die Ziele aus dem Konzept (Abschnitt 15;
//! REQ-IXL-024, REQ-JRN-006).
//!
//! Erzeugt einen Korridor ([`railit_import::generate::corridor`]), projektiert ihn und misst:
//!
//! - **Kernzyklus** unter Last: jeder Zyklus mit Meldungen aller Elemente (ungünstigste
//!   Eingabemenge), dazu laufend Fahrstraßenanforderungen und Rücknahmen; Ziel p99 < 1 ms bei
//!   5000 Elementen (Voreinstellung: 236 Bahnhöfe, 5004 Elemente).
//! - **Zulassungsprüfung** einer Fahrstraße: Mehrzeit eines Zyklus mit einer Anforderung
//!   gegenüber demselben Zyklus ohne, gepaart auf zwei gleichen Kopien des Zustands (Median);
//!   Ziel < 10 µs.
//! - **Replay**: Abspielen des dabei geschriebenen Journals (mit Zustandsabzügen, parallel),
//!   hochgerechnet auf 24 h Betrieb bei 100 ms Zykluszeit; Ziel < 60 s.
//!
//! Mit `--check` endet der Befehl mit Exit-Code 1, wenn ein Ziel verfehlt ist (CI).

use std::process::ExitCode;
use std::time::{Duration, Instant};

use railit_kernel::{
    Aspect, Command, Input, Interlocking, Occupancy, OwnedState, PointReport, Request, RequestId,
    SignalReport, VecOutputs,
};
use railit_plan::PlanView;
use railit_rules::GenericRules;
use railit_types::{LxId, PointId, PointPosition, RouteId, SignalId, Tick, TvdId};
use serde_json::json;

use crate::{EXIT_INVALID, Output, fail_usage, print_json};

/// Ziele aus dem Konzept, Abschnitt 15.
const CYCLE_P99_BUDGET: Duration = Duration::from_millis(1);
const ADMISSION_BUDGET: Duration = Duration::from_micros(10);
const REPLAY_24H_BUDGET: Duration = Duration::from_mins(1);
/// Zyklen in 24 h bei 100 ms.
const CYCLES_24H: f64 = 864_000.0;

/// Feld, das Stellbefehlen sofort folgt.
struct Field {
    points: Vec<PointPosition>,
    signals: Vec<Aspect>,
    lx_closed: Vec<bool>,
    tvds: usize,
}

impl Field {
    fn new(plan: &PlanView<'_>) -> Self {
        Self {
            points: vec![PointPosition::Right; plan.point_count()],
            signals: vec![Aspect::Stop; plan.signal_count()],
            lx_closed: vec![false; plan.lx_count()],
            tvds: plan.tvd_count(),
        }
    }

    fn reports(&self, out: &mut Vec<Input>) {
        out.clear();
        for (i, p) in self.points.iter().enumerate() {
            if let Some(point) = PointId::from_index(i) {
                out.push(Input::Point {
                    point,
                    report: PointReport::from(*p),
                });
            }
        }
        for (i, s) in self.signals.iter().enumerate() {
            if let Some(signal) = SignalId::from_index(i) {
                let report = match s {
                    Aspect::Stop => SignalReport::Stop,
                    Aspect::Proceed => SignalReport::Proceed,
                };
                out.push(Input::Signal { signal, report });
            }
        }
        for i in 0..self.tvds {
            if let Some(tvd) = TvdId::from_index(i) {
                out.push(Input::Tvd {
                    tvd,
                    occupancy: Occupancy::Clear,
                });
            }
        }
        for (i, closed) in self.lx_closed.iter().enumerate() {
            if let Some(lx) = LxId::from_index(i) {
                out.push(Input::LevelCrossing {
                    lx,
                    report: if *closed {
                        railit_kernel::LxReport::Closed
                    } else {
                        railit_kernel::LxReport::Open
                    },
                });
            }
        }
    }

    fn apply(&mut self, commands: &[Command]) {
        for c in commands {
            match *c {
                Command::MovePoint { point, position } => {
                    if let Some(p) = point.index().and_then(|i| self.points.get_mut(i)) {
                        *p = position;
                    }
                }
                Command::SetSignal { signal, aspect } => {
                    if let Some(s) = signal.index().and_then(|i| self.signals.get_mut(i)) {
                        *s = aspect;
                    }
                }
                Command::SetLevelCrossing { lx, close } => {
                    if let Some(l) = lx.index().and_then(|i| self.lx_closed.get_mut(i)) {
                        *l = close;
                    }
                }
                _ => {}
            }
        }
    }
}

/// Einfacher, deterministischer Zufall (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap_or(1)).unwrap_or(0)
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted.get(i).copied().unwrap_or_default()
}

struct Measured {
    elements: usize,
    routes: usize,
    generate: Duration,
    compile: Duration,
    p50: Duration,
    p99: Duration,
    max: Duration,
    admission: Duration,
    replay_24h: Duration,
    /// Lesen und Prüfen der Kette allein, hochgerechnet auf 24 h.
    read_24h: Duration,
    threads: usize,
    segments: u64,
    journal_bytes: usize,
    accepted: u64,
}

#[allow(clippy::too_many_lines)]
fn measure(stations: usize, cycles: usize) -> Result<Measured, String> {
    let t = Instant::now();
    let source = railit_import::generate::corridor(stations, 5);
    let net = railit_import::toml_format::parse(&source)
        .map_err(|e| format!("Korridor ungültig: {e}"))?
        .topology;
    let generate = t.elapsed();

    let t = Instant::now();
    let compiled = railit_planning::compile(&net, None).map_err(|e| e.to_string())?;
    let compile = t.elapsed();
    let image: &'static [u8] = Box::leak(compiled.image.into_boxed_slice());
    let plan = PlanView::parse(image).map_err(|e| e.to_string())?;
    let elements = plan.point_count()
        + plan.signal_count()
        + plan.tvd_count()
        + plan.lx_count()
        + plan.route_count();

    let kernel = Interlocking::new(plan, GenericRules::default());
    let mut state = OwnedState::new(&plan);
    let mut field = Field::new(&plan);
    let mut out = VecOutputs::default();
    let mut inputs = Vec::new();
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    // Im Dienst liegt alle 6000 Zyklen ein Zustandsabzug, 24 h haben also 144 Abschnitte.
    // Die kurze Messung bekommt so viele Abzüge, dass jeder Kern zwei Abschnitte abspielt;
    // der Aufwand je Abzug wiegt dabei schwerer als im Dienst (sichere Seite).
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let total = cycles.saturating_add(50);
    let snapshot_every = u64::try_from(total / threads.saturating_mul(2).max(1))
        .unwrap_or(u64::MAX)
        .max(50);
    let header = railit_journal::Header::new(
        &plan,
        &GenericRules::default(),
        railit_types::Duration::from_millis(100),
        600,
    )
    .with_snapshots(snapshot_every);
    let mut journal =
        railit_journal::JournalWriter::new(Vec::new(), header).map_err(|e| e.to_string())?;
    let mut times = Vec::with_capacity(cycles);
    // Zulassung: Mehrzeit eines Zyklus mit Anforderung gegenüber demselben Zyklus ohne,
    // gepaart auf zwei gleichen Kopien des Zustands (in Nanosekunden, mit Vorzeichen).
    let mut admission_ns: Vec<f64> = Vec::new();
    let mut accepted = 0u64;
    let mut next_id = 0u64;
    let routes = plan.route_count();
    let warmup = 50;
    for cycle in 0..cycles.saturating_add(warmup) {
        field.reports(&mut inputs);
        // Abwechselnd: Anforderung, Rücknahme, ruhiger Zyklus.
        let request = match cycle % 3 {
            0 => RouteId::from_index(rng.below(routes)).map(Request::SetRoute),
            1 => RouteId::from_index(rng.below(routes)).map(Request::CancelRoute),
            _ => None,
        };
        if let Some(request) = request {
            next_id += 1;
            inputs.push(Input::Request {
                id: RequestId(next_id),
                request,
            });
        }
        let now = Tick::from_millis(u64::try_from(cycle).unwrap_or(0).saturating_mul(100));
        if cycle >= warmup
            && let Some(Request::SetRoute(_)) = request
            && let Some((_, quiet_inputs)) = inputs.split_last()
        {
            let mut scratch = VecOutputs::default();
            let mut step_on_copy = |inputs: &[Input]| {
                let mut copy = state.clone();
                scratch.clear();
                let start = Instant::now();
                let _ = kernel.step(&mut copy.view(), now, inputs, &mut scratch);
                start.elapsed().as_secs_f64() * 1e9
            };
            // Reihenfolge wechseln, damit der Cache keine Seite bevorzugt.
            let (with, without) = if cycle % 2 == 0 {
                let w = step_on_copy(&inputs);
                (w, step_on_copy(quiet_inputs))
            } else {
                let wo = step_on_copy(quiet_inputs);
                (step_on_copy(&inputs), wo)
            };
            admission_ns.push(with - without);
        }
        out.clear();
        let start = Instant::now();
        let _ = kernel.step(&mut state.view(), now, &inputs, &mut out);
        let took = start.elapsed();
        accepted += u64::try_from(
            out.events
                .iter()
                .filter(|e| matches!(e, railit_kernel::Event::Accepted(_)))
                .count(),
        )
        .unwrap_or(0);
        field.apply(&out.commands);
        journal
            .record(now, &inputs, &out, &state)
            .map_err(|e| e.to_string())?;
        if cycle >= warmup {
            times.push(took);
        }
    }
    times.sort_unstable();
    admission_ns.sort_unstable_by(f64::total_cmp);
    let admission = admission_ns
        .get(admission_ns.len() / 2)
        .map_or(Duration::ZERO, |ns| {
            Duration::from_secs_f64(ns.max(0.0) / 1e9)
        });

    let bytes = journal.into_inner();
    let t = Instant::now();
    railit_journal::verify(bytes.as_slice()).map_err(|e| e.to_string())?;
    let read = t.elapsed();
    let t = Instant::now();
    let (summary, _) =
        railit_journal::replay(bytes.as_slice(), &plan).map_err(|e| e.to_string())?;
    let replay = t.elapsed();
    #[allow(clippy::cast_precision_loss)]
    let per_cycle = replay.as_secs_f64() / (cycles.saturating_add(warmup)) as f64;
    let replay_24h = Duration::from_secs_f64(per_cycle * CYCLES_24H);
    #[allow(clippy::cast_precision_loss)]
    let read_24h = Duration::from_secs_f64(
        read.as_secs_f64() / (cycles.saturating_add(warmup)) as f64 * CYCLES_24H,
    );

    Ok(Measured {
        elements,
        routes,
        generate,
        compile,
        p50: percentile(&times, 0.5),
        p99: percentile(&times, 0.99),
        max: times.last().copied().unwrap_or_default(),
        admission,
        replay_24h,
        read_24h,
        threads,
        segments: summary.snapshots.saturating_add(1),
        journal_bytes: bytes.len(),
        accepted,
    })
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// `railit bench`
pub(crate) fn bench(stations: usize, cycles: usize, check: bool, output: Output) -> ExitCode {
    if stations == 0 || cycles == 0 {
        return fail_usage("--stations und --cycles müssen größer als 0 sein");
    }
    let m = match measure(stations, cycles) {
        Ok(m) => m,
        Err(e) => return fail_usage(&e),
    };
    let checks = [
        ("Kernzyklus p99", m.p99 <= CYCLE_P99_BUDGET),
        ("Zulassungsprüfung", m.admission <= ADMISSION_BUDGET),
        ("Replay 24 h", m.replay_24h <= REPLAY_24H_BUDGET),
    ];
    let ok = checks.iter().all(|(_, ok)| *ok);
    match output {
        Output::Json => print_json(&json!({
            "stations": stations,
            "elements": m.elements,
            "routes": m.routes,
            "cycles": cycles,
            "accepted": m.accepted,
            "generate_ms": m.generate.as_secs_f64() * 1e3,
            "compile_ms": m.compile.as_secs_f64() * 1e3,
            "cycle_us": { "p50": us(m.p50), "p99": us(m.p99), "max": us(m.max) },
            "admission_us": us(m.admission),
            "replay_24h_s": m.replay_24h.as_secs_f64(),
            "read_24h_s": m.read_24h.as_secs_f64(),
            "replay_threads": m.threads,
            "replay_segments": m.segments,
            "journal_bytes_per_cycle": m.journal_bytes / cycles.saturating_add(50).max(1),
            "budgets_met": ok,
        })),
        Output::Text => {
            let mark = |b: bool| if b { "✔" } else { "✘" };
            println!(
                "Korridor mit {stations} Bahnhöfen: {} Elemente einschließlich {} Fahrstraßen",
                m.elements, m.routes
            );
            println!(
                "  Netz erzeugen und prüfen   {:>10.1} ms",
                m.generate.as_secs_f64() * 1e3
            );
            println!(
                "  Projektierung              {:>10.1} ms",
                m.compile.as_secs_f64() * 1e3
            );
            println!(
                "  Kernzyklus p50 / p99 / max {:>7.1} / {:.1} / {:.1} µs   {} Ziel p99 < 1000 µs",
                us(m.p50),
                us(m.p99),
                us(m.max),
                mark(checks[0].1)
            );
            println!(
                "  Zulassungsprüfung          {:>10.2} µs   {} Ziel < 10 µs",
                us(m.admission),
                mark(checks[1].1)
            );
            println!(
                "  Replay 24 h (hochgerechnet){:>10.1} s    {} Ziel < 60 s",
                m.replay_24h.as_secs_f64(),
                mark(checks[2].1)
            );
            println!(
                "    davon Lesen und Kette     {:>10.1} s;  {} Abschnitte auf {} Kernen, {} Byte je Zyklus",
                m.read_24h.as_secs_f64(),
                m.segments,
                m.threads,
                m.journal_bytes / cycles.saturating_add(50).max(1)
            );
            println!("  {cycles} Zyklen, {} Fahrstraßen angenommen", m.accepted);
        }
    }
    if check && !ok {
        return ExitCode::from(EXIT_INVALID);
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    /// REQ-IXL-024: Die Messung läuft durch (Netz erzeugen, projektieren, Zyklen unter Last,
    /// Journal mit Abzügen schreiben und parallel abspielen). Die Ziele selbst prüft der
    /// CI-Job „Leistung“ im Release-Build.
    #[test]
    fn bench_measures_a_small_corridor() {
        let m = super::measure(4, 120).unwrap();
        assert!(m.elements > 50 && m.routes > 10);
        assert!(m.accepted > 0, "keine Fahrstraße angenommen");
        assert!(m.segments > 1, "Replay ohne Abschnitte");
        assert!(m.p50 <= m.p99 && m.p99 <= m.max);
    }
}
