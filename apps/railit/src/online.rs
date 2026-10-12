//! Online-Befehle: Client der API von `railitd`.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use serde_json::{Value, json};
use ureq::tls::{
    Certificate, ClientCert, PemItem, PrivateKey, RootCerts, TlsConfig, TlsProvider, parse_pem,
};

use crate::{EXIT_INVALID, EXIT_USAGE, Output, print_json};

/// Verbindung zu `railitd`.
pub(crate) struct Client {
    base: String,
    token: Option<String>,
    agent: ureq::Agent,
}

/// Fehler mit Exit-Code.
pub(crate) struct Failure(u8, String);

impl Failure {
    pub(crate) fn exit(self) -> ExitCode {
        eprintln!("Fehler: {}", self.1);
        ExitCode::from(self.0)
    }
}

type Res<T> = Result<T, Failure>;

/// TLS-Einstellungen des Clients.
#[derive(Debug, Default)]
pub(crate) struct TlsFiles {
    /// CA, der das Serverzertifikat vertrauen muss (sonst die eingebauten Wurzeln).
    pub(crate) ca: Option<PathBuf>,
    /// Client-Zertifikat für mTLS.
    pub(crate) cert: Option<PathBuf>,
    /// Privater Schlüssel des Client-Zertifikats.
    pub(crate) key: Option<PathBuf>,
}

fn read(path: &Path) -> Res<Vec<u8>> {
    std::fs::read(path).map_err(|e| Failure(EXIT_USAGE, format!("{}: {e}", path.display())))
}

fn tls_config(files: &TlsFiles) -> Res<TlsConfig> {
    let bad = |e: ureq::Error| Failure(EXIT_USAGE, format!("TLS: {e}"));
    let mut tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()));
    if let Some(ca) = &files.ca {
        let certs = parse_pem(&read(ca)?)
            .filter_map(|item| match item {
                Ok(PemItem::Certificate(c)) => Some(Ok(c.to_owned())),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(bad)?;
        tls = tls.root_certs(RootCerts::Specific(Arc::new(certs)));
    }
    match (&files.cert, &files.key) {
        (Some(cert), Some(key)) => {
            let chain = [Certificate::from_pem(&read(cert)?).map_err(bad)?];
            let key = PrivateKey::from_pem(&read(key)?).map_err(bad)?;
            tls = tls.client_cert(Some(ClientCert::new_with_certs(&chain, key)));
        }
        (None, None) => {}
        _ => {
            return Err(Failure(
                EXIT_USAGE,
                "--cert und --key nur gemeinsam angeben".into(),
            ));
        }
    }
    Ok(tls.build())
}

/// Kodiert einen Namen als Pfadsegment (z. B. `AN1->Strecke`, `Adorf>Strecke`).
pub(crate) fn segment(name: &str) -> String {
    name.bytes()
        .fold(String::with_capacity(name.len()), |mut out, b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                out.push(char::from(b));
            } else {
                let _ = write!(out, "%{b:02X}");
            }
            out
        })
}

impl Client {
    pub(crate) fn new(server: &str, token: Option<String>, tls: &TlsFiles) -> Res<Self> {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .tls_config(tls_config(tls)?)
            .build()
            .into();
        Ok(Self {
            base: server.trim_end_matches('/').to_string(),
            token,
            agent,
        })
    }

    fn auth<B>(&self, req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        match &self.token {
            Some(t) => req.header("Authorization", format!("Bearer {t}")),
            None => req,
        }
    }

    fn finish(
        &self,
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Res<(u16, Value)> {
        let mut resp = result.map_err(|e| {
            Failure(
                EXIT_USAGE,
                format!("railitd unter {} nicht erreichbar: {e}", self.base),
            )
        })?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_json::<Value>().unwrap_or(Value::Null);
        match status {
            401 => Err(Failure(
                EXIT_USAGE,
                "Anmeldung abgewiesen (Token oder Client-Zertifikat prüfen)".into(),
            )),
            403 => Err(Failure(
                EXIT_INVALID,
                format!(
                    "keine Berechtigung: {}",
                    body["error"].as_str().unwrap_or("Rolle oder Stellbereich")
                ),
            )),
            404 | 400 => Err(Failure(
                EXIT_INVALID,
                body["error"]
                    .as_str()
                    .unwrap_or("nicht gefunden")
                    .to_string(),
            )),
            s if s >= 500 => Err(Failure(EXIT_USAGE, format!("Serverfehler {s}: {body}"))),
            _ => Ok((status, body)),
        }
    }

    pub(crate) fn get(&self, path: &str) -> Res<Value> {
        let req = self.auth(self.agent.get(format!("{}{path}", self.base)));
        self.finish(req.call()).map(|(_, body)| body)
    }

    pub(crate) fn post(&self, path: &str, body: Option<Value>) -> Res<(u16, Value)> {
        let req = self.auth(self.agent.post(format!("{}{path}", self.base)));
        let result = match body {
            Some(b) => req.send_json(b),
            None => req.send_empty(),
        };
        self.finish(result)
    }

    /// POST mit Bedienerkennung (Hilfshandlungen ohne Benutzerverwaltung).
    pub(crate) fn post_as(
        &self,
        path: &str,
        body: Option<Value>,
        operator: Option<u32>,
    ) -> Res<(u16, Value)> {
        let mut req = self.auth(self.agent.post(format!("{}{path}", self.base)));
        if let Some(op) = operator {
            req = req.header("X-Railit-Operator", op.to_string());
        }
        let result = match body {
            Some(b) => req.send_json(b),
            None => req.send_empty(),
        };
        self.finish(result)
    }

    pub(crate) fn delete(&self, path: &str) -> Res<Value> {
        let req = self.auth(self.agent.delete(format!("{}{path}", self.base)));
        self.finish(req.call()).map(|(_, body)| body)
    }
}

/// Deutscher Text zu einer Fahrstraßenphase.
fn phase_text(phase: &str) -> &str {
    match phase {
        "idle" => "Grundstellung",
        "setting" => "wird eingestellt",
        "locked" => "festgelegt",
        "proceed" => "Fahrt",
        "occupied" => "befahren",
        "arrived" => "Ziel erreicht (Durchrutschweg festgelegt)",
        "cancelling" => "Rücknahme (Zeitverschluss)",
        "halted" => "Signalhaltfall",
        other => other,
    }
}

/// Deutscher Text zu einem Ablehnungsgrund.
fn reason_text(reason: &str, element: Option<&str>) -> String {
    let e = element.unwrap_or("?");
    match reason {
        "conflict" => "feindliche Fahrstraße eingestellt".into(),
        "tvd_not_clear" => format!("Abschnitt {e} nicht frei"),
        "point_locked" => format!("Weiche {e} verschlossen"),
        "point_not_operable" => format!("Weiche {e} liegt in einem nicht freien Abschnitt"),
        "route_active" => "Fahrstraße bereits eingestellt".into(),
        "route_not_active" => "Fahrstraße nicht eingestellt".into(),
        "train_in_route" => "ein Zug befährt die Fahrstraße".into(),
        "safe_state" => "Kern im sicheren Zustand".into(),
        "unknown_element" => "unbekanntes Element".into(),
        "signal_held" => format!("Signal {e} wird als Flankenschutz in Halt gehalten"),
        "aux_busy" => "zu viele offene Hilfshandlungen".into(),
        "aux_unknown" => "Hilfshandlung unbekannt oder bereits erledigt".into(),
        "aux_expired" => "Bestätigungsfrist abgelaufen".into(),
        "same_operator" => "Bestätigung muss ein anderer Bediener geben".into(),
        "aux_not_applicable" => "Voraussetzungen der Hilfshandlung nicht erfüllt".into(),
        other => other.into(),
    }
}

fn show(v: &Value) -> &str {
    v.as_str().unwrap_or("–")
}

/// Zustandsbild.
pub(crate) fn state(c: &Client, output: Output) -> Res<ExitCode> {
    let s = c.get("/v1/state")?;
    if output == Output::Json {
        print_json(&s);
        return Ok(ExitCode::SUCCESS);
    }
    #[allow(clippy::cast_precision_loss)]
    let t = s["time_ms"].as_u64().unwrap_or(0) as f64 / 1000.0;
    println!(
        "Stellwerk „{}“ · t = {t:.1} s · Zyklus {}",
        show(&s["network"]),
        s["cycle"]
    );
    if let Some(reason) = s["safe_state"].as_str() {
        println!("!!! SICHERER ZUSTAND: {reason} – alle Signale Halt");
    }
    println!("\nSignale:");
    for sig in s["signals"].as_array().into_iter().flatten() {
        let aspect = if sig["commanded"] == "proceed" {
            "Fahrt"
        } else {
            "Halt"
        };
        let held = match sig["flank_holds"].as_u64() {
            Some(n) if n > 0 => " · Flankenschutz",
            _ => "",
        };
        println!(
            "  {:<8} {aspect:<6} {:<22} (Meldung: {}){held}",
            show(&sig["name"]),
            show(&sig["picture"]),
            show(&sig["report"])
        );
    }
    println!("\nWeichen:");
    for p in s["points"].as_array().into_iter().flatten() {
        let mut lock = p["locked_by"]
            .as_str()
            .map(|r| format!(" · verschlossen durch {r}"))
            .unwrap_or_default();
        if let Some(n) = p["holds"].as_u64().filter(|n| *n > 0) {
            let _ = write!(
                lock,
                " · {n}× verschlossen (Durchrutschweg/Flankenschutz, {})",
                show(&p["locked_position"])
            );
        }
        println!("  {:<8} {}{lock}", show(&p["name"]), show(&p["report"]));
    }
    println!("\nGleisfreimeldeabschnitte:");
    for t in s["tvds"].as_array().into_iter().flatten() {
        let mut lock = t["locked_by"]
            .as_str()
            .map(|r| format!(" · verschlossen durch {r}"))
            .unwrap_or_default();
        if let Some(r) = t["overlap_of"].as_str() {
            let _ = write!(lock, " · Durchrutschweg von {r}");
        }
        println!("  {:<8} {}{lock}", show(&t["name"]), show(&t["occupancy"]));
    }
    print_level_crossings_and_aux(&s);
    let active: Vec<&Value> = s["routes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["phase"] != "idle")
        .collect();
    println!("\nEingestellte Fahrstraßen ({}):", active.len());
    for r in active {
        println!(
            "  {:<10} {}",
            show(&r["name"]),
            phase_text(show(&r["phase"]))
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Bahnübergänge und Hilfshandlungen des Zustandsbilds.
fn print_level_crossings_and_aux(s: &Value) {
    let lxs = s["level_crossings"].as_array().cloned().unwrap_or_default();
    if !lxs.is_empty() {
        println!("\nBahnübergänge:");
        for l in lxs {
            let cmd = if l["close_commanded"] == true {
                "schließen"
            } else {
                "öffnen"
            };
            println!(
                "  {:<8} {:<10} (befohlen: {cmd})",
                show(&l["name"]),
                show(&l["report"])
            );
        }
    }
    let pending = s["pending_aux"].as_array().cloned().unwrap_or_default();
    println!(
        "\nHilfshandlungen: Zählwerk {}, offen {}",
        s["aux_counter"],
        pending.len()
    );
    for a in pending {
        println!(
            "  #{} {} {} (Bediener {})",
            a["aux"],
            show(&a["op"]["kind"]),
            show(&a["op"]["element"]),
            a["operator"]
        );
    }
}

/// Fahrstraßen mit Phase.
pub(crate) fn route_list(c: &Client, output: Output) -> Res<ExitCode> {
    let s = c.get("/v1/state")?;
    if output == Output::Json {
        print_json(&s["routes"]);
        return Ok(ExitCode::SUCCESS);
    }
    for r in s["routes"].as_array().into_iter().flatten() {
        println!(
            "  {:<10} {:>6} → {:<10} {}",
            show(&r["name"]),
            show(&r["start"]),
            show(&r["end"]),
            phase_text(show(&r["phase"]))
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Bedienanforderung und Antwort des Kerns.
pub(crate) fn request(
    c: &Client,
    path: &str,
    body: Option<Value>,
    what: &str,
    output: Output,
) -> Res<ExitCode> {
    let (status, answer) = c.post(path, body)?;
    if output == Output::Json {
        print_json(&answer);
    } else if status == 200 {
        println!("{what}: angenommen");
    } else {
        let reason = reason_text(show(&answer["reason"]), answer["element"].as_str());
        println!("{what}: abgelehnt – {reason}");
    }
    Ok(if status == 200 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_INVALID)
    })
}

/// Antwort eines Simulationsbefehls.
pub(crate) fn sim_done(result: Res<(u16, Value)>, what: &str, output: Output) -> Res<ExitCode> {
    let (_, body) = result?;
    if output == Output::Json {
        print_json(&body);
    } else {
        println!("{what}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Züge der Simulation.
pub(crate) fn trains(c: &Client, output: Output) -> Res<ExitCode> {
    let t = c.get("/v1/sim/trains")?;
    if output == Output::Json {
        print_json(&t);
        return Ok(ExitCode::SUCCESS);
    }
    let list = t.as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        println!("Keine Züge.");
    }
    for z in list {
        let occupied: Vec<&str> = z["occupied"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        println!(
            "  {:<8} {:<8} {:>3} km/h · Spitze auf {} bei {:.0} m · belegt: {}",
            show(&z["name"]),
            show(&z["state"]),
            z["speed_kmh"],
            show(&z["head_element"]),
            z["head_offset_m"].as_f64().unwrap_or(0.0),
            occupied.join(", ")
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Ereignisstrom ausgeben, bis die Verbindung endet.
pub(crate) fn watch(c: &Client, output: Output) -> Res<ExitCode> {
    let req = c.auth(c.agent.get(format!("{}/v1/events", c.base)));
    let resp = req
        .call()
        .map_err(|e| Failure(EXIT_USAGE, format!("railitd nicht erreichbar: {e}")))?;
    match resp.status().as_u16() {
        200 => {}
        401 => return Err(Failure(EXIT_USAGE, "Anmeldung abgewiesen".into())),
        s => return Err(Failure(EXIT_USAGE, format!("Status {s}"))),
    }
    let reader = BufReader::new(resp.into_body().into_reader());
    for line in reader.lines() {
        let line =
            line.map_err(|e| Failure(EXIT_USAGE, format!("Verbindung unterbrochen: {e}")))?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if output == Output::Json {
            println!("{data}");
            continue;
        }
        let e: Value = serde_json::from_str(data).unwrap_or(json!({}));
        #[allow(clippy::cast_precision_loss)]
        let t = e["time_ms"].as_u64().unwrap_or(0) as f64 / 1000.0;
        let text = match show(&e["type"]) {
            "route_phase" => format!(
                "Fahrstraße {}: {}",
                show(&e["route"]),
                phase_text(show(&e["phase"]))
            ),
            "alarm" => format!("ALARM {}: {}", show(&e["alarm"]), show(&e["element"])),
            "accepted" => format!("Anforderung {} angenommen", e["request"]),
            "rejected" => format!(
                "Anforderung {} abgelehnt – {}",
                e["request"],
                reason_text(show(&e["reason"]), e["element"].as_str())
            ),
            "sim" => format!("Simulation: {} {}", show(&e["event"]), show(&e["train"])),
            "aux_pending" => format!(
                "Hilfshandlung #{} {} {} angefordert von Bediener {} – Bestätigung nötig",
                e["aux"],
                show(&e["op"]["kind"]),
                show(&e["op"]["element"]),
                e["operator"]
            ),
            "aux_executed" => format!(
                "HILFSHANDLUNG #{} {} {} ausgeführt · Zählwerk {}",
                e["aux"],
                show(&e["op"]["kind"]),
                show(&e["op"]["element"]),
                e["counter"]
            ),
            "aux_expired" => format!("Hilfshandlung #{} verfallen", e["aux"]),
            _ => data.to_string(),
        };
        println!("[{t:>8.1} s] {text}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Offene Hilfshandlungen und Zählwerk.
pub(crate) fn aux_list(c: &Client, output: Output) -> Res<ExitCode> {
    let a = c.get("/v1/aux")?;
    if output == Output::Json {
        print_json(&a);
        return Ok(ExitCode::SUCCESS);
    }
    println!("Zählwerk: {}", a["counter"]);
    let pending = a["pending"].as_array().cloned().unwrap_or_default();
    if pending.is_empty() {
        println!("Keine offenen Hilfshandlungen.");
    }
    for p in pending {
        println!(
            "  #{} {} {} · Bediener {} · angefordert bei t = {:.1} s",
            p["aux"],
            show(&p["op"]["kind"]),
            show(&p["op"]["element"]),
            p["operator"],
            p["since_ms"].as_f64().unwrap_or(0.0) / 1000.0
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Hilfshandlung anfordern oder bestätigen.
pub(crate) fn aux(
    c: &Client,
    path: &str,
    body: Option<Value>,
    operator: Option<u32>,
    output: Output,
) -> Res<ExitCode> {
    let (status, answer) = c.post_as(path, body, operator)?;
    if output == Output::Json {
        print_json(&answer);
    } else if status == 200 {
        let aux = &answer["aux"];
        match show(&aux["type"]) {
            "aux_pending" => println!(
                "Hilfshandlung #{} angefordert. Ein zweiter Bediener bestätigt mit:\n  railit aux confirm {} --operator <NUMMER>",
                aux["aux"], aux["aux"]
            ),
            "aux_executed" => println!(
                "Hilfshandlung #{} ausgeführt. Zählwerk: {}",
                aux["aux"], aux["counter"]
            ),
            _ => println!("angenommen"),
        }
    } else {
        let reason = reason_text(show(&answer["reason"]), answer["element"].as_str());
        println!("abgelehnt – {reason}");
    }
    Ok(if status == 200 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_INVALID)
    })
}

/// Angemeldeter Benutzer.
pub(crate) fn whoami(c: &Client, output: Output) -> Res<ExitCode> {
    let me = c.get("/v1/whoami")?;
    if output == Output::Json {
        print_json(&me);
        return Ok(ExitCode::SUCCESS);
    }
    let roles: Vec<&str> = me["roles"]
        .as_array()
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    println!("Benutzer: {}", show(&me["name"]));
    println!("Rollen:   {}", roles.join(", "));
    if let Some(op) = me["operator"].as_u64() {
        println!("Bediener: {op}");
    }
    if let Some(areas) = me["areas"].as_array() {
        let areas: Vec<&str> = areas.iter().filter_map(Value::as_str).collect();
        println!("Bereiche: {}", areas.join(", "));
    }
    Ok(ExitCode::SUCCESS)
}

/// Stellbereiche mit Grenzen und Erlaubnissen.
pub(crate) fn areas(c: &Client, output: Output) -> Res<ExitCode> {
    let areas = c.get("/v1/areas")?;
    if output == Output::Json {
        print_json(&areas);
        return Ok(ExitCode::SUCCESS);
    }
    for a in areas.as_array().cloned().unwrap_or_default() {
        let safe = a["safe_state"]
            .as_str()
            .map(|r| format!(" · SICHERER ZUSTAND: {r}"))
            .unwrap_or_default();
        println!(
            "{} (Zyklus {}, Zählwerk {}){safe}",
            show(&a["name"]),
            a["cycle"],
            a["aux_counter"]
        );
        for b in a["borders"].as_array().cloned().unwrap_or_default() {
            let state = if b["holding"] == true {
                "hält Erlaubnis"
            } else if b["offering"] == true {
                "bietet Erlaubnis an"
            } else {
                "ohne Erlaubnis"
            };
            let requested = if b["requested"] == true {
                ", angefordert"
            } else {
                ""
            };
            println!("  {:<20} {state}{requested}", show(&b["name"]));
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Betriebsführung: Züge mit Zugnummer, Halt und Verspätung.
pub(crate) fn ops_list(c: &Client, output: Output) -> Res<ExitCode> {
    let ops = c.get("/v1/ops")?;
    if output == Output::Json {
        print_json(&ops);
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "Fahrplan: {} · Zuglenkung {}",
        ops["timetable"]
            .as_str()
            .filter(|t| !t.is_empty())
            .unwrap_or("–"),
        if ops["ars"] == true { "ein" } else { "aus" }
    );
    for t in ops["trains"].as_array().cloned().unwrap_or_default() {
        let place = if t["left"] == true {
            "hat das Netz verlassen".to_string()
        } else {
            format!("an {}", show(&t["berth"]))
        };
        let stop = match (t["at_stop"].as_str(), t["next_stop"].as_str()) {
            (Some(at), _) => format!(" · hält in {at}"),
            (None, Some(next)) => format!(" · nächster Halt {next}"),
            _ => String::new(),
        };
        let delay = t["delay_s"]
            .as_i64()
            .map(|d| format!(" · {:+} min", d / 60))
            .unwrap_or_default();
        let hold = if t["hold"] == true {
            " · ZURÜCKGEHALTEN"
        } else {
            ""
        };
        println!("  {:<8} {place}{stop}{delay}{hold}", show(&t["number"]));
    }
    Ok(ExitCode::SUCCESS)
}

/// Befehl ohne fachliche Antwort (Betriebsführung).
pub(crate) fn done(result: Res<(u16, Value)>, what: &str, output: Output) -> Res<ExitCode> {
    sim_done(result, what, output)
}

/// Auditprotokoll des Dienstes.
pub(crate) fn audit_show(c: &Client, last: usize, output: Output) -> Res<ExitCode> {
    let log = c.get(&format!("/v1/audit?last={last}"))?;
    if output == Output::Json {
        print_json(&log);
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "Auditprotokoll: {} Einträge, Kette gültig, Kopf {}",
        log["records"],
        show(&log["head"])
    );
    for r in log["last"].as_array().cloned().unwrap_or_default() {
        let area = r["area"]
            .as_str()
            .map(|a| format!(" [{a}]"))
            .unwrap_or_default();
        println!(
            "  #{:<5} {:<14} {:<20} {}{area} → {}",
            r["seq"],
            show(&r["user"]),
            show(&r["action"]),
            show(&r["target"]),
            show(&r["outcome"])
        );
    }
    Ok(ExitCode::SUCCESS)
}
