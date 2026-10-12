//! Ende-zu-Ende-Test der gRPC-Schnittstelle `railit.v1` (HTTP/2 ohne TLS, gleicher Port wie
//! REST, gleiche Anmeldung und Rechte).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tonic::{Code, Request};

#[allow(missing_docs, unreachable_pub, clippy::all, clippy::pedantic)]
mod pb {
    include!(concat!(env!("OUT_DIR"), "/railit.v1.rs"));
}

use pb::diagnostics_client::DiagnosticsClient;
use pb::events_client::EventsClient;
use pb::interlocking_client::InterlockingClient;

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/beispielstadt.toml"
);
const FDL: &str = "fdl-token-0123456789";
const OBS: &str = "obs-token-0123456789";

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

fn start() -> Daemon {
    let net = railit_import::toml_format::parse(&std::fs::read_to_string(EXAMPLE).unwrap())
        .unwrap()
        .topology;
    let plan = tmp("grpc.rplan");
    std::fs::write(&plan, railit_planning::compile(&net, None).unwrap().image).unwrap();
    let hash = |t: &str| railit_journal::to_hex(&Sha256::digest(t.as_bytes()));
    let users = tmp("grpc-users.toml");
    std::fs::write(
        &users,
        format!(
            "[[user]]\nname = \"fdl\"\nroles = [\"fahrdienstleiter\"]\noperator = 1\n\
             token_sha256 = \"{}\"\n\n[[user]]\nname = \"obs\"\nroles = [\"beobachter\"]\n\
             token_sha256 = \"{}\"\n",
            hash(FDL),
            hash(OBS)
        ),
    )
    .unwrap();
    let audit = tmp("grpc-audit.log");
    let _ = std::fs::remove_file(&audit);
    let mut child = Command::new(env!("CARGO_BIN_EXE_railitd"))
        .args(["--network", EXAMPLE, "--plan"])
        .arg(&plan)
        .arg("--users")
        .arg(&users)
        .arg("--audit")
        .arg(&audit)
        .args([
            "--allow-unsigned",
            "--listen",
            "127.0.0.1:0",
            "--speed",
            "20",
        ])
        .env_remove("RAILIT_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let url = line.trim().strip_prefix("railitd: ").unwrap().to_string();
    Daemon { child, url }
}

fn with_token<T>(message: T, token: &str) -> Request<T> {
    let mut r = Request::new(message);
    r.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    r
}

fn route(name: &str) -> pb::RouteRequest {
    pb::RouteRequest { route: name.into() }
}

/// REQ-SVC-008: gRPC mit denselben Rechten, Antworten und Ereignissen wie REST.
#[tokio::test]
async fn interlocking_over_grpc() {
    let d = start();
    let channel = Channel::from_shared(d.url.clone())
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut ixl = InterlockingClient::new(channel.clone());
    let mut diag = DiagnosticsClient::new(channel.clone());
    let mut events = EventsClient::new(channel);

    // Anmeldung über Metadaten.
    let err = ixl
        .set_route(Request::new(route("A-N1")))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated, "{err:?}");
    let me = diag
        .who_am_i(with_token(pb::WhoAmIRequest {}, FDL))
        .await
        .unwrap()
        .into_inner();
    assert_eq!((me.name.as_str(), me.operator), ("fdl", Some(1)));
    assert_eq!(me.roles, ["fahrdienstleiter"]);

    // Rechte: Beobachter darf nicht bedienen.
    let err = ixl
        .set_route(with_token(route("A-N1"), OBS))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    let mut stream = events
        .subscribe(with_token(
            pb::SubscribeRequest {
                types: vec!["route_phase".into()],
            },
            OBS,
        ))
        .await
        .unwrap()
        .into_inner();

    let a = ixl
        .set_route(with_token(route("A-N1"), FDL))
        .await
        .unwrap()
        .into_inner();
    assert!(a.accepted, "{a:?}");
    // Ablehnung durch den Kern ist kein gRPC-Fehler.
    let a = ixl
        .set_route(with_token(route("A-N2"), FDL))
        .await
        .unwrap()
        .into_inner();
    assert_eq!((a.accepted, a.reason.as_str()), (false, "conflict"));
    let err = ixl
        .set_route(with_token(route("X-Y"), FDL))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    let err = ixl
        .move_point(with_token(
            pb::MovePointRequest {
                point: "W1".into(),
                position: "schräg".into(),
            },
            FDL,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    let event = tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(event.r#type, "route_phase");
    assert!(event.json.contains("A-N1"), "{}", event.json);

    let state = ixl
        .get_state(with_token(pb::GetStateRequest {}, OBS))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(state.network, "Beispielstadt");
    let r = state.routes.iter().find(|r| r.name == "A-N1").unwrap();
    assert_ne!(r.phase, "idle");
    assert!(state.json.contains("\"routes\""));

    // Hilfshandlung mit der Bedienernummer des Benutzers.
    let a = ixl
        .request_aux(with_token(
            pb::AuxRequest {
                kind: "release_route".into(),
                element: "A-N1".into(),
                ..Default::default()
            },
            FDL,
        ))
        .await
        .unwrap()
        .into_inner();
    let aux = a.aux.unwrap();
    assert_eq!((aux.r#type.as_str(), aux.operator), ("aux_pending", 1));

    // Auditprotokoll nur mit Recht.
    let err = diag
        .get_audit(with_token(pb::GetAuditRequest { last: 10 }, FDL))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
}
