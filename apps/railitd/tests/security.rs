//! Ende-zu-Ende-Test der Sicherheit: Rollen, Stellbereiche, Auditprotokoll, TLS und mTLS
//! (REQ-SEC-001 … REQ-SEC-004).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ureq::tls::{Certificate, ClientCert, PrivateKey, RootCerts, TlsConfig, TlsProvider};

#[allow(missing_docs, unreachable_pub, clippy::all, clippy::pedantic)]
mod pb {
    include!(concat!(env!("OUT_DIR"), "/railit.v1.rs"));
}

const BEISPIELSTADT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/beispielstadt.toml"
);
const NORDBAHN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/nordbahn.toml");

struct Daemon {
    child: Child,
    url: String,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tmp(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

fn hash(token: &str) -> String {
    railit_journal::to_hex(&Sha256::digest(token.as_bytes()))
}

fn start(args: &[&str]) -> Daemon {
    let mut child = Command::new(env!("CARGO_BIN_EXE_railitd"))
        .args(args)
        .args(["--speed", "20", "--allow-unsigned"])
        .env_remove("RAILIT_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let url = line
        .trim()
        .strip_prefix("railitd: ")
        .expect(&line)
        .to_string();
    Daemon { child, url }
}

/// Unsignierter Plan der Beispielstadt bzw. Planverzeichnis der Nordbahn.
fn plan(network: &str, tag: &str) -> PathBuf {
    let net = railit_import::toml_format::parse(&std::fs::read_to_string(network).unwrap())
        .unwrap()
        .topology;
    if net.areas().len() == 0 {
        let path = tmp(&format!("{tag}.rplan"));
        let compiled = railit_planning::compile(&net, None).unwrap();
        std::fs::write(&path, compiled.image).unwrap();
        return path;
    }
    let dir = tmp(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = railit_planning::PlanningConfig::default();
    for a in railit_planning::compile_areas(&net, None, &cfg).unwrap() {
        std::fs::write(dir.join(format!("{}.rplan", a.name)), a.compiled.image).unwrap();
    }
    dir
}

fn call(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Result<(u16, Value), ureq::Error> {
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut resp = match (method, body) {
        ("GET", _) => {
            let mut r = agent.get(url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.call()?
        }
        (_, body) => {
            let mut r = agent.post(url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            match body {
                Some(b) => r.send_json(b)?,
                None => r.send_empty()?,
            }
        }
    };
    Ok((
        resp.status().as_u16(),
        resp.body_mut().read_json().unwrap_or(Value::Null),
    ))
}

fn plain_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

/// REQ-SEC-001, REQ-SEC-003, REQ-SEC-004: Benutzer mit Token, Rollen und Stellbereichen;
/// jede Handlung und jede Abweisung steht im Auditprotokoll.
#[test]
#[allow(clippy::too_many_lines)]
fn roles_areas_and_audit_with_tokens() {
    let plans = plan(NORDBAHN, "sec-nordbahn");
    let users = tmp("sec-users.toml");
    std::fs::write(
        &users,
        format!(
            r#"
            [[user]]
            name = "fdl-adorf"
            roles = ["fahrdienstleiter"]
            operator = 1
            areas = ["Adorf"]
            token_sha256 = "{}"

            [[user]]
            name = "beobachter"
            roles = ["beobachter"]
            token_sha256 = "{}"

            [[user]]
            name = "dispo"
            roles = ["disponent"]
            token_sha256 = "{}"

            [[user]]
            name = "pruefer"
            roles = ["projektierer"]
            token_sha256 = "{}"
            "#,
            hash("fdl-token-0123456789"),
            hash("obs-token-0123456789"),
            hash("dispo-token-012345678"),
            hash("pruef-token-012345678"),
        ),
    )
    .unwrap();
    let audit = tmp("sec-audit.log");
    let _ = std::fs::remove_file(&audit);
    let d = start(&[
        "--network",
        NORDBAHN,
        "--plan",
        plans.to_str().unwrap(),
        "--users",
        users.to_str().unwrap(),
        "--audit",
        audit.to_str().unwrap(),
        "--listen",
        "127.0.0.1:0",
    ]);
    let a = plain_agent();
    let url = |p: &str| format!("{}{p}", d.url);
    let (fdl, obs, dispo, pruef) = (
        Some("fdl-token-0123456789"),
        Some("obs-token-0123456789"),
        Some("dispo-token-012345678"),
        Some("pruef-token-012345678"),
    );

    assert_eq!(
        call(&a, "GET", &url("/v1/state"), None, None).unwrap().0,
        401
    );
    assert_eq!(
        call(&a, "GET", &url("/v1/state"), Some("falsch"), None)
            .unwrap()
            .0,
        401
    );
    let (status, me) = call(&a, "GET", &url("/v1/whoami"), obs, None).unwrap();
    assert_eq!((status, me["name"].as_str()), (200, Some("beobachter")));

    // Beobachter darf lesen, nicht bedienen.
    assert_eq!(
        call(&a, "GET", &url("/v1/state"), obs, None).unwrap().0,
        200
    );
    let (status, body) = call(&a, "POST", &url("/v1/routes/AA-AN1/set"), obs, None).unwrap();
    assert_eq!(status, 403, "{body}");

    // Fahrdienstleiter Adorf: nur in Adorf.
    let (status, body) = call(&a, "POST", &url("/v1/routes/AF-AP1/set"), fdl, None).unwrap();
    assert_eq!(
        (status, body["area"].as_str()),
        (200, Some("Adorf")),
        "{body}"
    );
    assert_eq!(
        call(&a, "POST", &url("/v1/routes/BA-BN1/set"), fdl, None)
            .unwrap()
            .0,
        403
    );
    // Hilfshandlung mit der Bedienernummer des Benutzers; kein Kopf nötig.
    let op = json!({"kind": "release_route", "element": "AF-AP1"});
    let (status, body) = call(&a, "POST", &url("/v1/aux"), fdl, Some(op)).unwrap();
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["aux"]["operator"], 1);

    // Disponent: Betriebsführung, kein Stellwerk.
    let ars = json!({"on": true});
    assert_eq!(
        call(&a, "POST", &url("/v1/ops/ars"), dispo, Some(ars.clone()))
            .unwrap()
            .0,
        200
    );
    assert_eq!(
        call(&a, "POST", &url("/v1/ops/ars"), fdl, Some(ars))
            .unwrap()
            .0,
        403
    );
    assert_eq!(
        call(&a, "POST", &url("/v1/routes/AA-AN1/cancel"), dispo, None)
            .unwrap()
            .0,
        403
    );

    // Auditprotokoll: Projektierer darf lesen, Beobachter nicht.
    assert_eq!(
        call(&a, "GET", &url("/v1/audit"), obs, None).unwrap().0,
        403
    );
    let (status, log) = call(&a, "GET", &url("/v1/audit?last=100"), pruef, None).unwrap();
    assert_eq!(status, 200, "{log}");
    let records = log["last"].as_array().unwrap();
    let find = |user: &str, action: &str, outcome: &str| {
        records.iter().any(|r| {
            r["user"] == user && r["action"] == action && r["outcome"].as_str() == Some(outcome)
        })
    };
    assert!(find("beobachter", "set_route", "denied"), "{log}");
    assert!(find("fdl-adorf", "set_route", "ok"), "{log}");
    assert!(find("fdl-adorf", "aux_request", "ok"), "{log}");
    assert!(find("dispo", "ops", "ok"), "{log}");
    assert!(find("beobachter", "read_audit", "denied"), "{log}");
    drop(d);

    let text = std::fs::read_to_string(&audit).unwrap();
    let summary = railit_journal::audit::verify_audit(text.as_bytes()).unwrap();
    assert!(summary.records >= 8, "{summary:?}");

    // Manipuliertes Protokoll: Der Dienst startet nicht.
    std::fs::write(&audit, text.replacen("beobachter", "niemand", 1)).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_railitd"))
        .args(["--network", NORDBAHN, "--plan", plans.to_str().unwrap()])
        .args(["--users", users.to_str().unwrap(), "--audit"])
        .arg(&audit)
        .args(["--allow-unsigned", "--listen", "127.0.0.1:0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Auditprotokoll"));
}

/// Ohne Anmeldung lauscht der Dienst nur lokal.
#[test]
fn open_mode_only_on_loopback() {
    let plan = plan(BEISPIELSTADT, "sec-open");
    let out = Command::new(env!("CARGO_BIN_EXE_railitd"))
        .args(["--network", BEISPIELSTADT, "--plan", plan.to_str().unwrap()])
        .args(["--allow-unsigned", "--listen", "0.0.0.0:0"])
        .env_remove("RAILIT_TOKEN")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Loopback"));
}

struct Pki {
    ca: String,
    server_cert: PathBuf,
    server_key: PathBuf,
    ca_file: PathBuf,
}

fn client(pki: &Pki, issuer: &CertifiedIssuer<'_, KeyPair>, cn: Option<&str>) -> ureq::Agent {
    let ca = Certificate::from_pem(pki.ca.as_bytes()).unwrap();
    let mut tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::Specific(Arc::new(vec![ca])))
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()));
    if let Some(cn) = cn {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, cn);
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let cert = params.signed_by(&key, issuer).unwrap();
        let chain = [Certificate::from_pem(cert.pem().as_bytes()).unwrap()];
        let key = PrivateKey::from_pem(key.serialize_pem().as_bytes()).unwrap();
        tls = tls.client_cert(Some(ClientCert::new_with_certs(&chain, key)));
    }
    ureq::Agent::config_builder()
        .tls_config(tls.build())
        .http_status_as_error(false)
        .build()
        .into()
}

/// REQ-SEC-002: TLS mit gegenseitiger Anmeldung; der CN des Client-Zertifikats bestimmt
/// den Benutzer. Ohne Zertifikat kommt keine Verbindung zustande.
#[test]
#[allow(clippy::too_many_lines)]
fn mutual_tls_identifies_users_by_certificate() {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "rail-it Test-CA");
    let issuer = CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();

    let server_key = KeyPair::generate().unwrap();
    let mut server_params =
        CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server = server_params.signed_by(&server_key, &issuer).unwrap();

    let pki = Pki {
        ca: issuer.pem(),
        server_cert: tmp("sec-server.pem"),
        server_key: tmp("sec-server.key"),
        ca_file: tmp("sec-ca.pem"),
    };
    std::fs::write(&pki.server_cert, server.pem()).unwrap();
    std::fs::write(&pki.server_key, server_key.serialize_pem()).unwrap();
    std::fs::write(&pki.ca_file, &pki.ca).unwrap();

    let users = tmp("sec-mtls-users.toml");
    std::fs::write(
        &users,
        r#"
        [[user]]
        name = "fdl1"
        roles = ["fahrdienstleiter"]
        operator = 1
        cert_cn = "fdl1"

        [[user]]
        name = "fdl2"
        roles = ["fahrdienstleiter"]
        operator = 2
        cert_cn = "fdl2"

        [[user]]
        name = "beobachter"
        roles = ["beobachter"]
        cert_cn = "beobachter"
        "#,
    )
    .unwrap();
    let audit = tmp("sec-mtls-audit.log");
    let _ = std::fs::remove_file(&audit);
    let plan = plan(BEISPIELSTADT, "sec-mtls");
    let d = start(&[
        "--network",
        BEISPIELSTADT,
        "--plan",
        plan.to_str().unwrap(),
        "--users",
        users.to_str().unwrap(),
        "--audit",
        audit.to_str().unwrap(),
        "--tls-cert",
        pki.server_cert.to_str().unwrap(),
        "--tls-key",
        pki.server_key.to_str().unwrap(),
        "--tls-client-ca",
        pki.ca_file.to_str().unwrap(),
        "--listen",
        "127.0.0.1:0",
    ]);
    assert!(d.url.starts_with("https://"), "{}", d.url);
    let url = |p: &str| format!("{}{p}", d.url);

    // Ohne Client-Zertifikat: Handshake abgewiesen.
    assert!(
        call(
            &client(&pki, &issuer, None),
            "GET",
            &url("/v1/state"),
            None,
            None
        )
        .is_err()
    );
    // Gesundheit bleibt mit Zertifikat erreichbar; unbekannter CN wird nicht angemeldet.
    let stranger = client(&pki, &issuer, Some("fremd"));
    assert_eq!(
        call(&stranger, "GET", &url("/v1/health"), None, None)
            .unwrap()
            .0,
        200
    );
    assert_eq!(
        call(&stranger, "GET", &url("/v1/state"), None, None)
            .unwrap()
            .0,
        401
    );

    let fdl1 = client(&pki, &issuer, Some("fdl1"));
    let fdl2 = client(&pki, &issuer, Some("fdl2"));
    let obs = client(&pki, &issuer, Some("beobachter"));
    let (_, me) = call(&fdl1, "GET", &url("/v1/whoami"), None, None).unwrap();
    assert_eq!(me["name"], "fdl1");
    assert_eq!(
        call(&obs, "POST", &url("/v1/routes/A-N1/set"), None, None)
            .unwrap()
            .0,
        403
    );
    let (status, body) = call(&fdl1, "POST", &url("/v1/routes/A-N1/set"), None, None).unwrap();
    assert_eq!(status, 200, "{body}");

    // Zwei-Personen-Regel mit den Bedienernummern der Zertifikatsinhaber.
    let op = json!({"kind": "release_route", "element": "A-N1"});
    let (status, body) = call(&fdl1, "POST", &url("/v1/aux"), None, Some(op)).unwrap();
    assert_eq!(status, 200, "{body}");
    let id = body["aux"]["aux"].as_u64().unwrap();
    let confirm = url(&format!("/v1/aux/{id}/confirm"));
    let (status, body) = call(&fdl1, "POST", &confirm, None, None).unwrap();
    assert_eq!(
        (status, body["reason"].as_str()),
        (409, Some("same_operator"))
    );
    let (status, body) = call(&fdl2, "POST", &confirm, None, None).unwrap();
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["aux"]["type"], "aux_executed");

    // gRPC über dieselbe TLS-Verbindung (ALPN h2) mit Client-Zertifikat.
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.distinguished_name.push(DnType::CommonName, "fdl2");
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let cert = params.signed_by(&key, &issuer).unwrap();
    let name = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let tls = tonic::transport::ClientTlsConfig::new()
                .ca_certificate(tonic::transport::Certificate::from_pem(&pki.ca))
                .identity(tonic::transport::Identity::from_pem(
                    cert.pem(),
                    key.serialize_pem(),
                ))
                .domain_name("localhost");
            let channel = tonic::transport::Channel::from_shared(d.url.clone())
                .unwrap()
                .tls_config(tls)
                .unwrap()
                .connect()
                .await
                .unwrap();
            pb::diagnostics_client::DiagnosticsClient::new(channel)
                .who_am_i(pb::WhoAmIRequest {})
                .await
                .unwrap()
                .into_inner()
                .name
        });
    assert_eq!(name, "fdl2");
    drop(d);

    let text = std::fs::read_to_string(&audit).unwrap();
    let summary = railit_journal::audit::verify_audit(text.as_bytes()).unwrap();
    assert_eq!(summary.records, 5, "{text}");
    assert!(text.contains("\"user\":\"fdl2\",\"action\":\"aux_confirm\""));
}
