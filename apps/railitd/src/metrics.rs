//! Metriken für den Betrieb im Prometheus-Textformat (`GET /v1/metrics`, REQ-SVC-009).
//!
//! Zähler und das Histogramm der Zykluszeit führt die Engine ([`Counters`]); Momentanwerte
//! (sicherer Zustand, Kanäle, Knoten, Fahrstraßen) stammen beim Abruf aus dem aktuellen
//! Zustandsbild. SIL 0: nur Beobachtung, keine Rückwirkung auf den Betrieb.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use railit_kernel::RoutePhase;
use railit_runtime::{NamedEvent, NodeSnapshot, Snapshot};

/// Obergrenzen der Histogrammklassen der Zykluszeit in Sekunden.
const CYCLE_BUCKETS: [f64; 10] = [
    0.000_1, 0.000_25, 0.000_5, 0.001, 0.002_5, 0.005, 0.01, 0.025, 0.05, 0.1,
];

/// Zähler der Engine seit dem Start des Dienstes.
#[derive(Clone, Debug, Default)]
pub(crate) struct Counters {
    /// Zyklen.
    pub(crate) cycles: u64,
    /// Zyklen, die länger als die Zykluszeit gerechnet haben.
    pub(crate) overruns: u64,
    /// Zyklen je Histogrammklasse (nicht kumuliert; die letzte Klasse ist „darüber“).
    buckets: [u64; CYCLE_BUCKETS.len() + 1],
    /// Summe der Zykluszeiten in Sekunden.
    cycle_seconds: f64,
    /// Längster Zyklus in Sekunden.
    cycle_max: f64,
    /// Angenommene und abgelehnte Anforderungen.
    pub(crate) accepted: u64,
    pub(crate) rejected: BTreeMap<String, u64>,
    /// Meldungen des Kerns nach Art.
    pub(crate) alarms: BTreeMap<String, u64>,
    /// Ausgeführte Hilfshandlungen.
    pub(crate) aux_executed: u64,
    /// Ausschlüsse von Kanälen und Abschaltungen der Sicherheitsschichten.
    pub(crate) channel_exclusions: u64,
    pub(crate) safety_trips: u64,
    /// ETCS-Notbremsungen.
    pub(crate) etcs_emergency_stops: u64,
    /// Ein Journal ist abgebrochen.
    pub(crate) journal_failed: bool,
}

impl Counters {
    /// Hält die Dauer eines Zyklus fest.
    pub(crate) fn observe_cycle(&mut self, took: Duration, period: Duration) {
        let s = took.as_secs_f64();
        self.cycles = self.cycles.saturating_add(1);
        if took > period {
            self.overruns = self.overruns.saturating_add(1);
        }
        let class = CYCLE_BUCKETS
            .iter()
            .position(|le| s <= *le)
            .unwrap_or(CYCLE_BUCKETS.len());
        if let Some(n) = self.buckets.get_mut(class) {
            *n = n.saturating_add(1);
        }
        self.cycle_seconds += s;
        self.cycle_max = self.cycle_max.max(s);
    }

    /// Zählt ein Ereignis des Kerns.
    pub(crate) fn observe_event(&mut self, event: &NamedEvent) {
        let bump = |map: &mut BTreeMap<String, u64>, key: &str| {
            let n = map.entry(key.to_string()).or_default();
            *n = n.saturating_add(1);
        };
        match event {
            NamedEvent::Accepted { .. } => self.accepted = self.accepted.saturating_add(1),
            NamedEvent::Rejected { reason, .. } => bump(&mut self.rejected, reason),
            NamedEvent::Alarm { alarm, .. } => bump(&mut self.alarms, alarm),
            NamedEvent::AuxExecuted { .. } => {
                self.aux_executed = self.aux_executed.saturating_add(1);
            }
            NamedEvent::RoutePhase { .. }
            | NamedEvent::AuxPending { .. }
            | NamedEvent::AuxExpired { .. }
            | NamedEvent::Permission { .. } => {}
        }
    }
}

/// Text mit Kopfzeilen je Metrik.
struct Out(String);

impl Out {
    fn head(&mut self, name: &str, kind: &str, help: &str) {
        let _ = writeln!(self.0, "# HELP {name} {help}");
        let _ = writeln!(self.0, "# TYPE {name} {kind}");
    }

    fn line(&mut self, name: &str, labels: &[(&str, &str)], value: impl std::fmt::Display) {
        let _ = write!(self.0, "{name}");
        if !labels.is_empty() {
            let inner: Vec<String> = labels
                .iter()
                .map(|(k, v)| format!("{k}=\"{}\"", escape(v)))
                .collect();
            let _ = write!(self.0, "{{{}}}", inner.join(","));
        }
        let _ = writeln!(self.0, " {value}");
    }

    fn single(&mut self, name: &str, kind: &str, help: &str, value: impl std::fmt::Display) {
        self.head(name, kind, help);
        self.line(name, &[], value);
    }
}

/// Maskiert einen Labelwert nach dem Textformat.
fn escape(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn flag(b: bool) -> u8 {
    u8::from(b)
}

/// Alle Metriken im Prometheus-Textformat (Version 0.0.4).
pub(crate) fn render(c: &Counters, snap: &Snapshot) -> String {
    let mut o = Out(String::new());
    o.single(
        "railit_cycles_total",
        "counter",
        "Zyklen seit dem Start des Dienstes.",
        c.cycles,
    );
    o.single(
        "railit_cycle_overruns_total",
        "counter",
        "Zyklen, die länger als die Zykluszeit gerechnet haben.",
        c.overruns,
    );
    o.head(
        "railit_cycle_seconds",
        "histogram",
        "Rechenzeit eines Zyklus des Dienstes (alle Stellbereiche, Feld, Betriebsführung).",
    );
    let mut cumulative = 0u64;
    for (le, n) in CYCLE_BUCKETS.iter().zip(&c.buckets) {
        cumulative = cumulative.saturating_add(*n);
        o.line(
            "railit_cycle_seconds_bucket",
            &[("le", &le.to_string())],
            cumulative,
        );
    }
    o.line("railit_cycle_seconds_bucket", &[("le", "+Inf")], c.cycles);
    o.line("railit_cycle_seconds_sum", &[], c.cycle_seconds);
    o.line("railit_cycle_seconds_count", &[], c.cycles);
    o.single(
        "railit_cycle_seconds_max",
        "gauge",
        "Längster Zyklus seit dem Start in Sekunden.",
        c.cycle_max,
    );

    o.head(
        "railit_requests_total",
        "counter",
        "Anforderungen an die Kerne nach Ergebnis und Ablehnungsgrund.",
    );
    o.line(
        "railit_requests_total",
        &[("result", "accepted"), ("reason", "")],
        c.accepted,
    );
    for (reason, n) in &c.rejected {
        o.line(
            "railit_requests_total",
            &[("result", "rejected"), ("reason", reason)],
            n,
        );
    }
    o.head(
        "railit_alarms_total",
        "counter",
        "Störungs- und Gefahrenmeldungen der Kerne nach Art.",
    );
    for (alarm, n) in &c.alarms {
        o.line("railit_alarms_total", &[("alarm", alarm)], n);
    }
    o.single(
        "railit_aux_executed_total",
        "counter",
        "Ausgeführte Hilfshandlungen seit dem Start des Dienstes.",
        c.aux_executed,
    );
    o.single(
        "railit_channel_exclusions_total",
        "counter",
        "Vom Voter ausgeschlossene Kanäle.",
        c.channel_exclusions,
    );
    o.single(
        "railit_safety_trips_total",
        "counter",
        "Abschaltungen einer Sicherheitsschicht.",
        c.safety_trips,
    );
    o.single(
        "railit_etcs_emergency_stops_total",
        "counter",
        "ETCS-Notbremsungen.",
        c.etcs_emergency_stops,
    );
    o.single(
        "railit_journal_ok",
        "gauge",
        "1, solange kein Journal abgebrochen ist.",
        flag(!c.journal_failed),
    );
    o.single(
        "railit_time_seconds",
        "gauge",
        "Betriebszeit (Simulationszeit) in Sekunden.",
        Duration::from_millis(snap.time_ms).as_secs_f64(),
    );
    area_gauges(&mut o, snap);
    o.0
}

/// Momentanwerte je Stellbereich aus dem Zustandsbild.
fn area_gauges(o: &mut Out, snap: &Snapshot) {
    o.head(
        "railit_safe_state",
        "gauge",
        "1, wenn der Stellbereich im sicheren Zustand ist.",
    );
    for a in &snap.areas {
        o.line(
            "railit_safe_state",
            &[("area", &a.name)],
            flag(a.safe_state.is_some()),
        );
    }
    o.head(
        "railit_channels_active",
        "gauge",
        "Mitwählende Kanäle je Stellbereich.",
    );
    for a in &snap.areas {
        let n = a.safety.channels.iter().filter(|c| c.active).count();
        o.line("railit_channels_active", &[("area", &a.name)], n);
    }
    o.head(
        "railit_aux_counter",
        "gauge",
        "Zählwerk der ausgeführten Hilfshandlungen je Stellbereich.",
    );
    for a in &snap.areas {
        o.line("railit_aux_counter", &[("area", &a.name)], a.aux_counter);
    }
    o.head(
        "railit_routes",
        "gauge",
        "Eingestellte Fahrstraßen je Stellbereich und Phase.",
    );
    let mut routes: BTreeMap<(&str, String), u64> = BTreeMap::new();
    for r in snap.routes.iter().filter(|r| r.phase != RoutePhase::Idle) {
        let phase = serde_json::to_value(r.phase)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let n = routes.entry((r.area.as_str(), phase)).or_default();
        *n = n.saturating_add(1);
    }
    for ((area, phase), n) in &routes {
        o.line("railit_routes", &[("area", area), ("phase", phase)], n);
    }
    node_gauges(o, snap);
}

/// Name, Beschreibung und Wert einer Metrik eines Knotens.
type NodeMetric = (&'static str, &'static str, fn(&NodeSnapshot) -> u64);

/// Anbindung der Knoten des sicheren Rechners.
fn node_gauges(o: &mut Out, snap: &Snapshot) {
    let nodes: Vec<_> = snap
        .areas
        .iter()
        .filter_map(|a| a.safety.node.as_ref().map(|n| (a.name.as_str(), n)))
        .collect();
    if nodes.is_empty() {
        return;
    }
    let gauges: [NodeMetric; 3] = [
        (
            "railit_node_up",
            "1, wenn die RaSTA-Verbindung zum Knoten steht.",
            |n| u64::from(n.up),
        ),
        (
            "railit_node_available",
            "1, wenn der Knoten Zyklen liefert (sonst Rückfallebene).",
            |n| u64::from(n.available),
        ),
        (
            "railit_node_synced",
            "1, wenn der Spiegel mit dem Knoten übereinstimmt.",
            |n| u64::from(n.synced),
        ),
    ];
    for (name, help, get) in gauges {
        o.head(name, "gauge", help);
        for (area, n) in &nodes {
            o.line(name, &[("area", area)], get(n));
        }
    }
    let counters: [NodeMetric; 5] = [
        (
            "railit_node_cycles_total",
            "Empfangene Zyklen des Knotens.",
            |n| n.cycles,
        ),
        (
            "railit_node_syncs_total",
            "Abgeschlossene Zustandsabgleiche.",
            |n| u64::from(n.syncs),
        ),
        (
            "railit_node_divergences_total",
            "Abweichungen des Spiegels vom Knoten.",
            |n| u64::from(n.divergences),
        ),
        (
            "railit_node_link_losses_total",
            "Verbindungsabbrüche.",
            |n| u64::from(n.link_losses),
        ),
        (
            "railit_node_protocol_errors_total",
            "Unvollständige oder widersprüchliche Nachrichten.",
            |n| u64::from(n.protocol_errors),
        ),
    ];
    for (name, help, get) in counters {
        o.head(name, "counter", help);
        for (area, n) in &nodes {
            o.line(name, &[("area", area)], get(n));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_is_cumulative_and_consistent() {
        let mut c = Counters::default();
        let period = Duration::from_millis(100);
        for us in [50, 300, 300, 2_000, 150_000] {
            c.observe_cycle(Duration::from_micros(us), period);
        }
        let mut o = Out(String::new());
        let mut cumulative = 0u64;
        for (le, n) in CYCLE_BUCKETS.iter().zip(&c.buckets) {
            cumulative += n;
            o.line("b", &[("le", &le.to_string())], cumulative);
        }
        assert!(o.0.contains("b{le=\"0.0001\"} 1"), "{}", o.0);
        assert!(o.0.contains("b{le=\"0.0005\"} 3"), "{}", o.0);
        assert!(o.0.contains("b{le=\"0.1\"} 4"), "{}", o.0);
        assert_eq!(c.cycles, 5);
        assert_eq!(c.overruns, 1);
    }

    #[test]
    fn label_values_are_escaped() {
        assert_eq!(escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }
}
