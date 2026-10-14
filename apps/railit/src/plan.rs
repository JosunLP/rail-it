//! Offline-Befehle der Projektierung: `railit plan keygen|compile|verify|info`.

use std::path::Path;
use std::process::ExitCode;

use railit_plan::{Plan, PlanView, Protection, RouteEnd};
use railit_runtime::keys::{generate, read_signing_key, read_verifying_key, to_hex};
use serde_json::json;

use crate::{EXIT_INVALID, Output, fail_usage, print_json};

/// Erzeugt ein Schlüsselpaar `<prefix>.key` (geheim) und `<prefix>.pub` (öffentlich).
pub(crate) fn keygen(prefix: &Path, output: Output) -> ExitCode {
    let secret_path = prefix.with_extension("key");
    let public_path = prefix.with_extension("pub");
    for p in [&secret_path, &public_path] {
        if p.exists() {
            return fail_usage(&format!(
                "{} existiert bereits und wird nicht überschrieben",
                p.display()
            ));
        }
    }
    let key = match generate() {
        Ok(k) => k,
        Err(e) => return fail_usage(&e),
    };
    let public = to_hex(key.verifying_key().as_bytes());
    if let Err(e) = write_secret(&secret_path, &to_hex(&key.to_bytes())) {
        return fail_usage(&format!("{}: {e}", secret_path.display()));
    }
    if let Err(e) = std::fs::write(&public_path, format!("{public}\n")) {
        return fail_usage(&format!("{}: {e}", public_path.display()));
    }
    match output {
        Output::Json => print_json(&json!({
            "secret": secret_path, "public": public_path, "public_key": public,
        })),
        Output::Text => {
            println!(
                "Geheimer Schlüssel:     {} (sicher verwahren!)",
                secret_path.display()
            );
            println!("Öffentlicher Schlüssel: {}", public_path.display());
            println!("                        {public}");
        }
    }
    ExitCode::SUCCESS
}

/// Schreibt den geheimen Schlüssel nur für den Eigentümer lesbar.
fn write_secret(path: &Path, hex: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    writeln!(file, "{hex}")
}

fn load_network(path: &Path) -> Result<railit_topology::Topology, (u8, String)> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| (crate::EXIT_USAGE, format!("{}: {e}", path.display())))?;
    railit_import::parse(&path.to_string_lossy(), &source)
        .map(|v| v.topology)
        .map_err(|e| (EXIT_INVALID, format!("Netz ungültig:\n{e}")))
}

fn fail(code: u8, msg: &str) -> ExitCode {
    eprintln!("Fehler: {msg}");
    ExitCode::from(code)
}

/// Erzeugt, prüft und schreibt ein Plan-Image.
pub(crate) fn compile(network: &Path, out: &Path, key: Option<&Path>, output: Output) -> ExitCode {
    let net = match load_network(network) {
        Ok(n) => n,
        Err((code, msg)) => return fail(code, &msg),
    };
    let key = match key.map(read_signing_key).transpose() {
        Ok(k) => k,
        Err(e) => return fail_usage(&e),
    };
    if net.areas().len() > 0 {
        return compile_areas(&net, out, key.as_ref(), output);
    }
    let compiled = match railit_planning::compile(&net, key.as_ref()) {
        Ok(c) => c,
        Err(report) => {
            return fail(
                EXIT_INVALID,
                &format!("Projektierung fehlgeschlagen:\n{report}"),
            );
        }
    };
    if let Err(e) = std::fs::write(out, &compiled.image) {
        return fail_usage(&format!("{}: {e}", out.display()));
    }
    let digest = PlanView::parse(&compiled.image)
        .map(|v| to_hex(&v.digest()))
        .unwrap_or_default();
    match output {
        Output::Json => print_json(&json!({
            "plan": out, "network": compiled.plan.network, "routes": compiled.plan.routes.len(),
            "signed": key.is_some(), "digest": digest, "bytes": compiled.image.len(),
        })),
        Output::Text => {
            println!(
                "Plan „{}“ geschrieben: {} ({} Fahrstraßen, {} Byte, {})",
                compiled.plan.network,
                out.display(),
                compiled.plan.routes.len(),
                compiled.image.len(),
                if key.is_some() {
                    "signiert"
                } else {
                    "unsigniert"
                }
            );
            println!("Prüfsumme: {digest}");
            println!("Generator und unabhängige Gegenprüfung stimmen überein.");
        }
    }
    ExitCode::SUCCESS
}

/// Projektiert ein Netz mit Stellbereichen: je Bereich ein Plan-Image `<Bereich>.rplan` im
/// Verzeichnis `out` (Eingabe für `railitd --plan <Verzeichnis>`).
fn compile_areas(
    net: &railit_topology::Topology,
    out: &Path,
    key: Option<&railit_plan::SigningKey>,
    output: Output,
) -> ExitCode {
    let cfg = railit_planning::PlanningConfig::default();
    let areas = match railit_planning::compile_areas(net, key, &cfg) {
        Ok(a) => a,
        Err(report) => {
            return fail(
                EXIT_INVALID,
                &format!("Projektierung fehlgeschlagen:\n{report}"),
            );
        }
    };
    if let Err(e) = std::fs::create_dir_all(out) {
        return fail_usage(&format!("{}: {e}", out.display()));
    }
    let mut written = Vec::new();
    for a in &areas {
        let path = out.join(format!("{}.rplan", a.name));
        if let Err(e) = std::fs::write(&path, &a.compiled.image) {
            return fail_usage(&format!("{}: {e}", path.display()));
        }
        let digest = PlanView::parse(&a.compiled.image)
            .map(|v| to_hex(&v.digest()))
            .unwrap_or_default();
        written.push(json!({
            "area": a.name, "plan": path, "routes": a.compiled.plan.routes.len(),
            "borders": a.compiled.plan.borders.len(), "digest": digest,
        }));
    }
    match output {
        Output::Json => print_json(&json!({
            "network": net.name(), "signed": key.is_some(), "areas": written,
        })),
        Output::Text => {
            println!(
                "Netz „{}“: {} Stellbereiche nach {} ({})",
                net.name(),
                areas.len(),
                out.display(),
                if key.is_some() {
                    "signiert"
                } else {
                    "unsigniert"
                }
            );
            for w in &written {
                println!(
                    "  {:<12} {} Fahrstraßen, {} Grenzen, Prüfsumme {}",
                    w["area"].as_str().unwrap_or("?"),
                    w["routes"],
                    w["borders"],
                    w["digest"].as_str().unwrap_or("?")
                );
            }
            println!("Generator und unabhängige Gegenprüfung stimmen je Bereich überein.");
        }
    }
    ExitCode::SUCCESS
}

/// Prüft ein Plan-Image: Struktur, Prüfsumme, optional Signatur und Übereinstimmung mit
/// einem Netz (unabhängige Gegenprüfung).
pub(crate) fn verify(
    plan_path: &Path,
    key: Option<&Path>,
    network: Option<&Path>,
    output: Output,
) -> ExitCode {
    let bytes = match std::fs::read(plan_path) {
        Ok(b) => b,
        Err(e) => return fail_usage(&format!("{}: {e}", plan_path.display())),
    };
    let key = match key.map(read_verifying_key).transpose() {
        Ok(k) => k,
        Err(e) => return fail_usage(&e),
    };
    let view = match key.as_ref().map_or_else(
        || PlanView::parse(&bytes),
        |k| PlanView::parse_signed(&bytes, k),
    ) {
        Ok(v) => v,
        Err(e) => return fail(EXIT_INVALID, &format!("Plan ungültig: {e}")),
    };
    let mut checks = vec!["Struktur und Verweise", "Prüfsumme (SHA-256)"];
    if key.is_some() {
        checks.push("Signatur (Ed25519)");
    }
    if let Some(network) = network {
        let net = match load_network(network) {
            Ok(n) => n,
            Err((code, msg)) => return fail(code, &msg),
        };
        // Plan eines Stellbereichs („Netz/Bereich“): gegen das Teilnetz prüfen.
        let cut = match view.network_name().split_once('/') {
            Some((_, area)) => match net.find_area(area).map(|id| net.area_cut(id)) {
                Some(Ok(cut)) => Some(cut),
                Some(Err(report)) => return fail(EXIT_INVALID, &report.to_string()),
                None => {
                    return fail(
                        EXIT_INVALID,
                        &format!("Stellbereich „{area}“ nicht im Netz"),
                    );
                }
            },
            None => None,
        };
        let target = cut.as_ref().map_or(&net, |c| &c.topology);
        if let Err(report) = railit_planning::verify(target, &Plan::from_view(&view)) {
            return fail(
                EXIT_INVALID,
                &format!("Plan passt nicht zum Netz:\n{report}"),
            );
        }
        checks.push("Gegenprüfung gegen das Netz");
    }
    match output {
        Output::Json => print_json(&json!({
            "valid": true, "network": view.network_name(), "signed": view.is_signed(),
            "digest": to_hex(&view.digest()), "checks": checks,
        })),
        Output::Text => {
            println!(
                "Plan „{}“ ist gültig ({}).",
                view.network_name(),
                checks.join(", ")
            );
            if !view.is_signed() {
                println!("Hinweis: Der Plan ist nicht signiert.");
            }
        }
    }
    ExitCode::SUCCESS
}

/// Beschreibt die Fahrstraßen eines Plans.
pub(crate) fn info(plan_path: &Path, output: Output) -> ExitCode {
    let bytes = match std::fs::read(plan_path) {
        Ok(b) => b,
        Err(e) => return fail_usage(&format!("{}: {e}", plan_path.display())),
    };
    let view = match PlanView::parse(&bytes) {
        Ok(v) => v,
        Err(e) => return fail(EXIT_INVALID, &format!("Plan ungültig: {e}")),
    };
    let end_name = |end| match end {
        RouteEnd::Signal(s) => view.signal_name(s).unwrap_or("?").to_string(),
        RouteEnd::Boundary => "Netzgrenze".into(),
        RouteEnd::BufferStop => "Prellbock".into(),
        RouteEnd::Border(b) => view.border_name(b).unwrap_or("?").to_string(),
    };
    match output {
        Output::Json => print_json(&json!({
            "network": view.network_name(),
            "signed": view.is_signed(),
            "digest": to_hex(&view.digest()),
            "routes": view.routes().map(|r| json!({
                "name": r.name(),
                "start": view.signal_name(r.start()),
                "end": end_name(r.end()),
                "speed_kmh": r.speed().map(railit_plan::Speed::as_kmh),
                "points": r.points().map(|(p, pos)| json!({"point": view.point_name(p), "position": pos})).collect::<Vec<_>>(),
                "tvds": r.tvds().map(|t| view.tvd_name(t)).collect::<Vec<_>>(),
                "overlap": {
                    "tvds": r.overlap_tvds().map(|t| view.tvd_name(t)).collect::<Vec<_>>(),
                    "points": r.overlap_points().map(|(p, pos)| json!({"point": view.point_name(p), "position": pos})).collect::<Vec<_>>(),
                },
                "flank": r.flank().map(|f| match f.protection {
                    Protection::Point(p, pos) => json!({"point": view.point_name(p), "position": pos, "protects": view.tvd_name(f.protects)}),
                    Protection::Signal(s) => json!({"signal": view.signal_name(s), "protects": view.tvd_name(f.protects)}),
                }).collect::<Vec<_>>(),
                "level_crossings": r.level_crossings().map(|l| view.lx_name(l)).collect::<Vec<_>>(),
                "conflicts": r.conflicts().map(|c| view.route_name(c)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })),
        Output::Text => {
            println!(
                "Plan „{}“ ({}, Prüfsumme {}…)",
                view.network_name(),
                if view.is_signed() {
                    "signiert"
                } else {
                    "unsigniert"
                },
                to_hex(&view.digest()).get(..16).unwrap_or_default()
            );
            print_routes(&view);
        }
    }
    ExitCode::SUCCESS
}

/// Fahrstraßen eines Plans als Text.
fn print_routes(view: &PlanView<'_>) {
    let end_name = |end| match end {
        RouteEnd::Signal(s) => view.signal_name(s).unwrap_or("?").to_string(),
        RouteEnd::Boundary => "Netzgrenze".into(),
        RouteEnd::BufferStop => "Prellbock".into(),
        RouteEnd::Border(b) => view.border_name(b).unwrap_or("?").to_string(),
    };
    println!("\nFahrstraßen ({}):", view.route_count());
    let point_text = |(p, pos): (railit_plan::PointId, railit_plan::PointPosition)| {
        let side = if pos == railit_plan::PointPosition::Left {
            "links"
        } else {
            "rechts"
        };
        format!("{} {side}", view.point_name(p).unwrap_or("?"))
    };
    let list = |items: Vec<String>| {
        if items.is_empty() {
            "–".to_string()
        } else {
            items.join(", ")
        }
    };
    for r in view.routes() {
        let points: Vec<String> = r.points().map(point_text).collect();
        let tvds: Vec<&str> = r.tvds().map(|t| view.tvd_name(t).unwrap_or("?")).collect();
        let conflicts: Vec<&str> = r
            .conflicts()
            .map(|c| view.route_name(c).unwrap_or("?"))
            .collect();
        println!(
            "  {:<10} {} → {}",
            r.name(),
            view.signal_name(r.start()).unwrap_or("?"),
            end_name(r.end())
        );
        println!(
            "             Weichen:    {}",
            if points.is_empty() {
                "–".into()
            } else {
                points.join(", ")
            }
        );
        println!("             Abschnitte: {}", tvds.join(" → "));
        let speed = r.speed().map_or_else(
            || "Streckengeschwindigkeit".into(),
            |v| format!("{} km/h", v.as_kmh()),
        );
        println!("             Geschw.:    {speed}");
        let overlap: Vec<String> = r
            .overlap_tvds()
            .map(|t| view.tvd_name(t).unwrap_or("?").to_string())
            .chain(r.overlap_points().map(point_text))
            .collect();
        println!("             D-Weg:      {}", list(overlap));
        let flank: Vec<String> = r
            .flank()
            .map(|f| {
                let what = match f.protection {
                    Protection::Point(p, pos) => point_text((p, pos)),
                    Protection::Signal(s) => view.signal_name(s).unwrap_or("?").to_string(),
                };
                format!("{what} (für {})", view.tvd_name(f.protects).unwrap_or("?"))
            })
            .collect();
        println!("             Flanke:     {}", list(flank));
        let lxs: Vec<String> = r
            .level_crossings()
            .map(|l| view.lx_name(l).unwrap_or("?").to_string())
            .collect();
        if !lxs.is_empty() {
            println!("             BÜ:         {}", list(lxs));
        }
        println!(
            "             feindlich:  {}",
            if conflicts.is_empty() {
                "–".into()
            } else {
                conflicts.join(", ")
            }
        );
    }
}

/// Führt Szenarien aus.
pub(crate) fn run_scenarios(
    files: &[std::path::PathBuf],
    node: bool,
    channels: Option<u8>,
    journal: bool,
    ils: bool,
    output: Output,
) -> ExitCode {
    let opts = railit_runtime::scenario::RunOptions {
        channels: channels
            .and_then(|c| railit_runtime::railit_safety::Redundancy::from_channels(usize::from(c))),
        node,
        journal,
        ils,
    };
    let mut all_passed = true;
    let mut results = Vec::new();
    for file in files {
        match railit_runtime::scenario::run_file_opts(file, opts) {
            Ok(outcome) => {
                all_passed &= outcome.passed;
                match output {
                    Output::Text => print!("{outcome}"),
                    Output::Json => results.push(json!({
                        "file": file,
                        "name": outcome.name,
                        "passed": outcome.passed,
                        "steps": outcome.steps.iter().map(|s| json!({
                            "index": s.index, "note": s.note, "passed": s.passed,
                            "failures": s.failures, "time_ms": s.time_ms,
                        })).collect::<Vec<_>>(),
                    })),
                }
            }
            Err(e) => return fail_usage(&format!("{}: {e}", file.display())),
        }
    }
    if output == Output::Json {
        print_json(&json!({ "passed": all_passed, "scenarios": results }));
    }
    if all_passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_INVALID)
    }
}
