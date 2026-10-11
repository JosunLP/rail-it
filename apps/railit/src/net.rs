//! Offline-Befehle für Netzbeschreibungen: `railit net validate|info`.

use std::path::Path;
use std::process::ExitCode;

use railit_import::{ImportError, toml_format};
use railit_topology::{
    Direction, NodeRef, Port, PortRef, SignalKind, SignalSystem, TerminalKind, Topology, TvdMember,
    Validated,
};
use serde_json::{Value, json};

use crate::{EXIT_INVALID, Output, fail_usage, print_json};

/// Liest und importiert eine Netzbeschreibung.
fn load(file: &Path) -> Result<Result<Validated, ImportError>, String> {
    let source = std::fs::read_to_string(file)
        .map_err(|e| format!("{} kann nicht gelesen werden: {e}", file.display()))?;
    Ok(toml_format::parse(&source))
}

pub(crate) fn validate(file: &Path, output: Output) -> ExitCode {
    let result = match load(file) {
        Ok(result) => result,
        Err(msg) => return fail_usage(&msg),
    };
    match (output, &result) {
        (Output::Json, _) => print_json(&validation_json(&result)),
        (Output::Text, Ok(v)) => {
            let net = &v.topology;
            println!(
                "Netz „{}“ ist gültig: {} Gleise, {} Weichen, {} Netzenden, {} Signale, {} Gleisfreimeldeabschnitte.",
                net.name(),
                net.tracks().len(),
                net.points().len(),
                net.terminals().len(),
                net.signals().len(),
                net.tvds().len()
            );
            for w in &v.warnings {
                println!("Warnung: {w}");
            }
        }
        (Output::Text, Err(e)) => print!("{e}"),
    }
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_INVALID)
    }
}

pub(crate) fn info(file: &Path, output: Output) -> ExitCode {
    let validated = match load(file) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            match output {
                Output::Json => print_json(&validation_json(&Err(e))),
                Output::Text => print!("{e}"),
            }
            return ExitCode::from(EXIT_INVALID);
        }
        Err(msg) => return fail_usage(&msg),
    };
    let net = &validated.topology;
    match output {
        Output::Json => print_json(&info_json(net)),
        Output::Text => print_info_text(net),
    }
    ExitCode::SUCCESS
}

fn validation_json(result: &Result<Validated, ImportError>) -> Value {
    match result {
        Ok(v) => json!({
            "valid": true,
            "network": v.topology.name(),
            "errors": [],
            "warnings": v.warnings.iter().map(ToString::to_string).collect::<Vec<_>>(),
        }),
        Err(ImportError::Syntax(msg)) => json!({
            "valid": false,
            "errors": [{ "context": "", "message": msg, "requirement": null }],
            "warnings": [],
        }),
        Err(ImportError::Invalid { errors, warnings }) => json!({
            "valid": false,
            "errors": errors.iter().map(|e| json!({
                "context": e.context,
                "message": e.message,
                "requirement": e.requirement,
            })).collect::<Vec<_>>(),
            "warnings": warnings,
        }),
    }
}

fn metres(mm: u32) -> f64 {
    f64::from(mm) / 1000.0
}

fn tvd_name(net: &Topology, member: TvdMember) -> &str {
    net.tvd_of(member)
        .and_then(|id| net.tvd(id))
        .map_or("–", |t| t.name.as_str())
}

/// `"<element>.<port>"` des Gegenports von `port`.
fn linked_name(net: &Topology, port: PortRef) -> String {
    net.linked(port).map_or_else(
        || "–".to_string(),
        |other| {
            let name = net.element_name(other.node.into()).unwrap_or("?");
            format!("{name}.{}", other.port)
        },
    )
}

const fn direction_str(d: Direction) -> &'static str {
    match d {
        Direction::AToB => "a_to_b",
        Direction::BToA => "b_to_a",
    }
}

const fn signal_kind_str(k: SignalKind) -> &'static str {
    match k {
        SignalKind::Main => "main",
        SignalKind::Distant => "distant",
        SignalKind::Combined => "combined",
        SignalKind::Shunting => "shunting",
        SignalKind::EtcsStopMarker => "etcs_stop_marker",
    }
}

const fn signal_kind_label(k: SignalKind) -> &'static str {
    match k {
        SignalKind::Main => "Hauptsignal",
        SignalKind::Distant => "Vorsignal",
        SignalKind::Combined => "Mehrabschnittssignal",
        SignalKind::Shunting => "Rangiersignal",
        SignalKind::EtcsStopMarker => "ETCS-Halt-Tafel",
    }
}

const fn system_str(s: SignalSystem) -> &'static str {
    match s {
        SignalSystem::Ks => "ks",
        SignalSystem::Hv => "hv",
    }
}

const fn terminal_kind_str(k: TerminalKind) -> &'static str {
    match k {
        TerminalKind::BufferStop => "buffer_stop",
        TerminalKind::Boundary => "boundary",
    }
}

fn info_json(net: &Topology) -> Value {
    let name_of = |e| net.element_name(e).unwrap_or("?");
    json!({
        "network": net.name(),
        "components": net.components(),
        "tracks": net.tracks().map(|(id, t)| json!({
            "name": t.name,
            "length_m": metres(t.length.as_mm()),
            "tvd": tvd_name(net, TvdMember::Track(id)),
        })).collect::<Vec<_>>(),
        "points": net.points().map(|(id, p)| json!({
            "name": p.name,
            "tvd": tvd_name(net, TvdMember::Point(id)),
            "diverging": p.diverging.map(railit_topology::PointPosition::as_str),
            "diverging_speed_kmh": p.diverging.map(|_| p.diverging_speed.as_kmh()),
        })).collect::<Vec<_>>(),
        "terminals": net.terminals().map(|(_, t)| json!({
            "name": t.name,
            "kind": terminal_kind_str(t.kind),
        })).collect::<Vec<_>>(),
        "signals": net.signals().map(|(_, s)| json!({
            "name": s.name,
            "track": net.track(s.track).map_or("?", |t| t.name.as_str()),
            "position_m": metres(s.position.as_mm()),
            "direction": direction_str(s.direction),
            "kind": signal_kind_str(s.kind),
            "system": system_str(s.system),
        })).collect::<Vec<_>>(),
        "level_crossings": net.level_crossings().map(|(_, l)| json!({
            "name": l.name,
            "track": net.track(l.track).map_or("?", |t| t.name.as_str()),
            "position_m": metres(l.position.as_mm()),
        })).collect::<Vec<_>>(),
        "tvds": net.tvds().map(|(_, t)| json!({
            "name": t.name,
            "elements": t.members.iter().map(|m| name_of((*m).into())).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn print_info_text(net: &Topology) {
    println!("Netz „{}“ ({} Teilnetz(e))", net.name(), net.components());

    println!("\nGleise ({}):", net.tracks().len());
    for (id, t) in net.tracks() {
        println!(
            "  {:<12} {:>10}   TVD {}",
            t.name,
            t.length.to_string(),
            tvd_name(net, TvdMember::Track(id))
        );
    }

    println!("\nWeichen ({}):", net.points().len());
    for (id, p) in net.points() {
        let node = NodeRef::Point(id);
        let ends = [
            (Port::Tip, "Spitze"),
            (Port::Left, "links"),
            (Port::Right, "rechts"),
        ]
        .map(|(port, label)| format!("{label} → {}", linked_name(net, PortRef::new(node, port))))
        .join(", ");
        let diverging = p.diverging.map_or_else(String::new, |d| {
            let side = if d == railit_topology::PointPosition::Left {
                "links"
            } else {
                "rechts"
            };
            format!(" · abzweigend {side} ({})", p.diverging_speed)
        });
        println!(
            "  {:<12} TVD {:<8} {ends}{diverging}",
            p.name,
            tvd_name(net, TvdMember::Point(id))
        );
    }

    println!("\nNetzenden ({}):", net.terminals().len());
    for (_, t) in net.terminals() {
        let kind = match t.kind {
            TerminalKind::BufferStop => "Prellbock",
            TerminalKind::Boundary => "Netzgrenze",
        };
        println!("  {:<12} {kind}", t.name);
    }

    println!("\nSignale ({}):", net.signals().len());
    for (_, s) in net.signals() {
        let track = net.track(s.track).map_or("?", |t| t.name.as_str());
        let dir = match s.direction {
            Direction::AToB => "a→b",
            Direction::BToA => "b→a",
        };
        println!(
            "  {:<12} {:>10} auf {track:<8} {dir}   {} ({})",
            s.name,
            s.position.to_string(),
            signal_kind_label(s.kind),
            system_str(s.system).to_uppercase()
        );
    }

    if net.level_crossings().len() > 0 {
        println!("\nBahnübergänge ({}):", net.level_crossings().len());
        for (_, l) in net.level_crossings() {
            let track = net.track(l.track).map_or("?", |t| t.name.as_str());
            println!(
                "  {:<12} {:>10} auf {track}",
                l.name,
                l.position.to_string()
            );
        }
    }

    println!("\nGleisfreimeldeabschnitte ({}):", net.tvds().len());
    for (_, t) in net.tvds() {
        let members: Vec<_> = t
            .members
            .iter()
            .map(|m| net.element_name((*m).into()).unwrap_or("?"))
            .collect();
        println!("  {:<12} {}", t.name, members.join(", "));
    }
}
