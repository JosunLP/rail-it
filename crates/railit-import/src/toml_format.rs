//! rail-it-TOML: kompaktes, handschreibbares Netzformat.
//!
//! ```toml
//! [network]
//! name = "Mini"
//!
//! [[terminal]]
//! name = "W"
//! kind = "boundary"          # oder "buffer_stop", "border" (Grenze zu einem Nachbarbereich)
//! initial = true             # nur bei "border": dieser Bereich beginnt mit der Erlaubnis
//!
//! [[track]]
//! name = "1"
//! length_m = 250.5           # Meter, auf Millimeter gerundet
//!
//! [[point]]
//! name = "W1"
//! diverging = "right"        # optional: abzweigender Strang
//! diverging_speed_kmh = 40   # optional, Standard 40
//!
//! [[link]]                   # Ports: a, b (Gleis) · tip, left, right (Weiche) · end (Netzende)
//! from = "W.end"
//! to = "1.a"
//!
//! [[signal]]
//! name = "A"
//! track = "1"
//! position_m = 200           # ab Port a
//! direction = "a_to_b"       # oder "b_to_a"
//! kind = "main"              # main, distant, combined, shunting, etcs_stop_marker
//! system = "ks"              # optional: ks (Standard) oder hv
//! auto = true                # optional: Selbstblocksignal (Zuglenkung stellt selbsttätig)
//!
//! [[level_crossing]]         # Bahnübergang
//! name = "BÜ1"
//! track = "1"
//! position_m = 120
//!
//! [[crossing]]              # Kreuzung ohne bewegliche Teile: Gleise schneiden sich
//! name = "Kr1"               # beide Kreuzungsstellen im selben Abschnitt (REQ-TOP-016)
//! legs = [{ track = "K1", position_m = 10 }, { track = "K2", position_m = 10 }]
//!
//! [[tvd]]
//! name = "T1"
//! elements = ["1"]           # Gleise und Weichen
//!
//! [[area]]                   # optional: Stellbereiche (je einer pro Sicherheitskern)
//! name = "Adorf"
//! tvds = ["T1"]
//!
//! [[station]]                # optional: Betriebsstellen für die Betriebsführung
//! name = "Adorf"
//! platforms = [{ name = "1", signals = ["N1", "P1"] }]
//! ```
//!
//! Ein vollständiges Beispiel liegt unter `examples/beispielstadt.toml`.

use serde::Deserialize;

use railit_topology::{
    DEFAULT_DIVERGING_SPEED, Direction, ElementRef, Length, NodeRef, PointPosition, Port, PortRef,
    SignalKind, SignalSystem, Speed, TerminalKind, TopologyBuilder, TopologyError, TvdMember,
    Validated,
};

use crate::{Diagnostic, ImportError};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    network: NetworkSection,
    #[serde(default)]
    terminal: Vec<TerminalEntry>,
    #[serde(default)]
    track: Vec<TrackEntry>,
    #[serde(default)]
    point: Vec<PointEntry>,
    #[serde(default)]
    link: Vec<LinkEntry>,
    #[serde(default)]
    signal: Vec<SignalEntry>,
    #[serde(default)]
    tvd: Vec<TvdEntry>,
    #[serde(default)]
    level_crossing: Vec<LevelCrossingEntry>,
    #[serde(default)]
    crossing: Vec<CrossingEntry>,
    #[serde(default)]
    area: Vec<AreaEntry>,
    #[serde(default)]
    station: Vec<StationEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AreaEntry {
    name: String,
    tvds: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StationEntry {
    name: String,
    #[serde(default)]
    platforms: Vec<PlatformEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformEntry {
    name: String,
    signals: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NetworkSection {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalEntry {
    name: String,
    kind: TerminalKindEntry,
    #[serde(default)]
    initial: bool,
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TerminalKindEntry {
    BufferStop,
    Boundary,
    Border,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackEntry {
    name: String,
    length_m: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PointEntry {
    name: String,
    #[serde(default)]
    diverging: Option<PositionEntry>,
    #[serde(default)]
    diverging_speed_kmh: Option<u16>,
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PositionEntry {
    Left,
    Right,
}

#[derive(Copy, Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SystemEntry {
    #[default]
    Ks,
    Hv,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LevelCrossingEntry {
    name: String,
    track: String,
    position_m: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CrossingEntry {
    name: String,
    legs: [CrossingLeg; 2],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CrossingLeg {
    track: String,
    position_m: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkEntry {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalEntry {
    name: String,
    track: String,
    position_m: f64,
    direction: DirectionEntry,
    kind: SignalKindEntry,
    #[serde(default)]
    system: SystemEntry,
    #[serde(default)]
    auto: bool,
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DirectionEntry {
    AToB,
    BToA,
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SignalKindEntry {
    Main,
    Distant,
    Combined,
    Shunting,
    EtcsStopMarker,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TvdEntry {
    name: String,
    elements: Vec<String>,
}

/// Liest ein Netz im rail-it-TOML-Format und validiert es vollständig.
///
/// # Errors
/// [`ImportError::Syntax`] bei ungültigem TOML oder unbekannten Feldern,
/// [`ImportError::Invalid`] mit allen Fehlern, wenn das beschriebene Netz ungültig ist.
pub fn parse(source: &str) -> Result<Validated, ImportError> {
    let doc: Document = toml::from_str(source).map_err(|e| ImportError::Syntax(e.to_string()))?;
    let mut errors = Vec::new();
    let mut b = TopologyBuilder::new(doc.network.name);

    add_terminals(&mut b, doc.terminal, &mut errors);
    add_tracks(&mut b, doc.track, &mut errors);
    add_points(&mut b, doc.point, &mut errors);
    add_links(&mut b, &doc.link, &mut errors);
    add_signals(&mut b, doc.signal, &mut errors);
    add_tvds(&mut b, doc.tvd, &mut errors);
    add_level_crossings(&mut b, doc.level_crossing, &mut errors);
    add_crossings(&mut b, doc.crossing, &mut errors);
    add_areas(&mut b, doc.area, &mut errors);
    add_stations(&mut b, doc.station, &mut errors);

    match b.build() {
        Ok(validated) if errors.is_empty() => Ok(validated),
        Ok(validated) => Err(ImportError::Invalid {
            errors,
            warnings: validated.warnings.iter().map(ToString::to_string).collect(),
        }),
        Err(report) => {
            errors.extend(report.errors.iter().map(|e| model_error(String::new(), e)));
            Err(ImportError::Invalid {
                errors,
                warnings: report.warnings.iter().map(ToString::to_string).collect(),
            })
        }
    }
}

fn add_terminals(
    b: &mut TopologyBuilder,
    entries: Vec<TerminalEntry>,
    errors: &mut Vec<Diagnostic>,
) {
    for t in entries {
        let context = format!("terminal „{}“", t.name);
        let kind = match t.kind {
            TerminalKindEntry::BufferStop => TerminalKind::BufferStop,
            TerminalKindEntry::Boundary => TerminalKind::Boundary,
            TerminalKindEntry::Border => TerminalKind::Border { initial: t.initial },
        };
        if t.initial && !kind.is_border() {
            errors.push(Diagnostic {
                context: context.clone(),
                message: "initial ist nur bei kind = \"border\" zulässig".into(),
                requirement: None,
            });
        }
        if let Err(e) = b.add_terminal(t.name, kind) {
            errors.push(model_error(context, &e));
        }
    }
}

fn add_tracks(b: &mut TopologyBuilder, entries: Vec<TrackEntry>, errors: &mut Vec<Diagnostic>) {
    for t in entries {
        let context = format!("track „{}“", t.name);
        match metres(t.length_m) {
            Some(length) => {
                if let Err(e) = b.add_track(t.name, length) {
                    errors.push(model_error(context, &e));
                }
            }
            None => errors.push(invalid_metres(context, "length_m", t.length_m)),
        }
    }
}

fn add_points(b: &mut TopologyBuilder, entries: Vec<PointEntry>, errors: &mut Vec<Diagnostic>) {
    for p in entries {
        let context = format!("point „{}“", p.name);
        if p.diverging.is_none() && p.diverging_speed_kmh.is_some() {
            errors.push(Diagnostic {
                context: context.clone(),
                message: "diverging_speed_kmh ohne diverging".into(),
                requirement: None,
            });
        }
        let id = match b.add_point(p.name) {
            Ok(id) => id,
            Err(e) => {
                errors.push(model_error(context, &e));
                continue;
            }
        };
        if let Some(d) = p.diverging {
            let pos = match d {
                PositionEntry::Left => PointPosition::Left,
                PositionEntry::Right => PointPosition::Right,
            };
            let speed = p
                .diverging_speed_kmh
                .map_or(DEFAULT_DIVERGING_SPEED, Speed::from_kmh);
            if let Err(e) = b.set_point_diverging(id, pos, speed) {
                errors.push(model_error(context, &e));
            }
        }
    }
}

fn add_links(b: &mut TopologyBuilder, entries: &[LinkEntry], errors: &mut Vec<Diagnostic>) {
    for (i, l) in entries.iter().enumerate() {
        let context = format!("link[{i}] ({} – {})", l.from, l.to);
        match (resolve_port(b, &l.from), resolve_port(b, &l.to)) {
            (Ok(from), Ok(to)) => {
                if let Err(e) = b.connect(from, to) {
                    errors.push(model_error(context, &e));
                }
            }
            (from, to) => {
                for message in [from.err(), to.err()].into_iter().flatten() {
                    errors.push(Diagnostic {
                        context: context.clone(),
                        message,
                        requirement: None,
                    });
                }
            }
        }
    }
}

fn add_signals(b: &mut TopologyBuilder, entries: Vec<SignalEntry>, errors: &mut Vec<Diagnostic>) {
    for s in entries {
        let context = format!("signal „{}“", s.name);
        let Some(ElementRef::Track(track)) = b.lookup(&s.track) else {
            errors.push(Diagnostic {
                context,
                message: format!("„{}“ ist kein bekanntes Gleis", s.track),
                requirement: None,
            });
            continue;
        };
        let Some(position) = metres(s.position_m) else {
            errors.push(invalid_metres(context, "position_m", s.position_m));
            continue;
        };
        let direction = match s.direction {
            DirectionEntry::AToB => Direction::AToB,
            DirectionEntry::BToA => Direction::BToA,
        };
        let kind = match s.kind {
            SignalKindEntry::Main => SignalKind::Main,
            SignalKindEntry::Distant => SignalKind::Distant,
            SignalKindEntry::Combined => SignalKind::Combined,
            SignalKindEntry::Shunting => SignalKind::Shunting,
            SignalKindEntry::EtcsStopMarker => SignalKind::EtcsStopMarker,
        };
        let system = match s.system {
            SystemEntry::Ks => SignalSystem::Ks,
            SystemEntry::Hv => SignalSystem::Hv,
        };
        match b.add_signal(s.name, track, position, direction, kind) {
            Ok(id) => {
                if let Err(e) = b
                    .set_signal_system(id, system)
                    .and_then(|()| b.set_signal_auto(id, s.auto))
                {
                    errors.push(model_error(context, &e));
                }
            }
            Err(e) => errors.push(model_error(context, &e)),
        }
    }
}

fn add_level_crossings(
    b: &mut TopologyBuilder,
    entries: Vec<LevelCrossingEntry>,
    errors: &mut Vec<Diagnostic>,
) {
    for l in entries {
        let context = format!("level_crossing „{}“", l.name);
        let Some(ElementRef::Track(track)) = b.lookup(&l.track) else {
            errors.push(Diagnostic {
                context,
                message: format!("„{}“ ist kein bekanntes Gleis", l.track),
                requirement: None,
            });
            continue;
        };
        let Some(position) = metres(l.position_m) else {
            errors.push(invalid_metres(context, "position_m", l.position_m));
            continue;
        };
        if let Err(e) = b.add_level_crossing(l.name, track, position) {
            errors.push(model_error(context, &e));
        }
    }
}

fn add_crossings(
    b: &mut TopologyBuilder,
    entries: Vec<CrossingEntry>,
    errors: &mut Vec<Diagnostic>,
) {
    for c in entries {
        let context = format!("crossing „{}“", c.name);
        let mut legs = Vec::with_capacity(2);
        for leg in &c.legs {
            let Some(ElementRef::Track(track)) = b.lookup(&leg.track) else {
                errors.push(Diagnostic {
                    context: context.clone(),
                    message: format!("„{}“ ist kein bekanntes Gleis", leg.track),
                    requirement: None,
                });
                continue;
            };
            let Some(position) = metres(leg.position_m) else {
                errors.push(invalid_metres(
                    context.clone(),
                    "position_m",
                    leg.position_m,
                ));
                continue;
            };
            legs.push((track, position));
        }
        if let [x, y] = legs[..]
            && let Err(e) = b.add_crossing(c.name, [x, y])
        {
            errors.push(model_error(context, &e));
        }
    }
}

fn add_tvds(b: &mut TopologyBuilder, entries: Vec<TvdEntry>, errors: &mut Vec<Diagnostic>) {
    for t in entries {
        let context = format!("tvd „{}“", t.name);
        let mut members = Vec::with_capacity(t.elements.len());
        let mut ok = true;
        for name in &t.elements {
            let message = match b.lookup(name) {
                Some(ElementRef::Track(id)) => {
                    members.push(TvdMember::Track(id));
                    continue;
                }
                Some(ElementRef::Point(id)) => {
                    members.push(TvdMember::Point(id));
                    continue;
                }
                Some(_) => format!("„{name}“ ist weder Gleis noch Weiche"),
                None => format!("unbekanntes Element „{name}“"),
            };
            ok = false;
            errors.push(Diagnostic {
                context: context.clone(),
                message,
                requirement: None,
            });
        }
        if ok && let Err(e) = b.add_tvd(t.name, &members) {
            errors.push(model_error(context, &e));
        }
    }
}

fn add_areas(b: &mut TopologyBuilder, entries: Vec<AreaEntry>, errors: &mut Vec<Diagnostic>) {
    for a in entries {
        let context = format!("area „{}“", a.name);
        let mut tvds = Vec::with_capacity(a.tvds.len());
        for name in &a.tvds {
            match b.lookup(name) {
                Some(ElementRef::Tvd(id)) => tvds.push(id),
                _ => errors.push(Diagnostic {
                    context: context.clone(),
                    message: format!("„{name}“ ist kein bekannter Gleisfreimeldeabschnitt"),
                    requirement: None,
                }),
            }
        }
        if let Err(e) = b.add_area(a.name, &tvds) {
            errors.push(model_error(context, &e));
        }
    }
}

fn add_stations(b: &mut TopologyBuilder, entries: Vec<StationEntry>, errors: &mut Vec<Diagnostic>) {
    for s in entries {
        let context = format!("station „{}“", s.name);
        let mut platforms = Vec::with_capacity(s.platforms.len());
        for p in s.platforms {
            let mut signals = Vec::new();
            for name in &p.signals {
                match b.lookup(name) {
                    Some(ElementRef::Signal(id)) => signals.push(id),
                    _ => errors.push(Diagnostic {
                        context: format!("{context}, Bahnsteig „{}“", p.name),
                        message: format!("„{name}“ ist kein bekanntes Signal"),
                        requirement: None,
                    }),
                }
            }
            platforms.push((p.name, signals));
        }
        if let Err(e) = b.add_station(s.name, &platforms) {
            errors.push(model_error(context, &e));
        }
    }
}

/// Löst `"<element>.<port>"` auf. Elementnamen dürfen selbst Punkte enthalten.
fn resolve_port(b: &TopologyBuilder, spec: &str) -> Result<PortRef, String> {
    let (name, port) = spec
        .rsplit_once('.')
        .ok_or_else(|| format!("„{spec}“ hat nicht die Form <element>.<port>"))?;
    let port = Port::parse(port)
        .ok_or_else(|| format!("„{port}“ ist kein Port (erlaubt: a, b, tip, left, right, end)"))?;
    let node = match b.lookup(name) {
        Some(ElementRef::Track(id)) => NodeRef::Track(id),
        Some(ElementRef::Point(id)) => NodeRef::Point(id),
        Some(ElementRef::Terminal(id)) => NodeRef::Terminal(id),
        Some(_) => return Err(format!("„{name}“ hat keine Ports")),
        None => return Err(format!("unbekanntes Element „{name}“")),
    };
    Ok(PortRef::new(node, port))
}

/// Meter (Dezimalzahl) → [`Length`] in Millimetern; `None` bei negativen, nicht endlichen
/// oder zu großen Werten.
pub(crate) fn metres(value: f64) -> Option<Length> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let mm = (value * 1000.0).round();
    if mm > f64::from(u32::MAX) {
        return None;
    }
    // Wertebereich oben geprüft: 0 ≤ mm ≤ u32::MAX und ganzzahlig.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(Length::from_mm(mm as u32))
}

fn invalid_metres(context: String, field: &str, value: f64) -> Diagnostic {
    Diagnostic {
        context,
        message: format!("{field} = {value} ist keine gültige Meterangabe"),
        requirement: None,
    }
}

pub(crate) fn model_error(context: String, e: &TopologyError) -> Diagnostic {
    Diagnostic {
        context,
        message: e.to_string(),
        requirement: Some(e.requirement()),
    }
}
