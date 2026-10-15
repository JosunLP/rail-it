//! `railit-node` – Knoten des sicheren Rechners auf einem Server (Konzept, Abschnitt 13).
//!
//! Rechnet den Kern eines Stellbereichs in einem, zwei oder drei Kanälen mit Voter und
//! Watchdog ([`railit_runtime::remote::ServerNode`]) im eigenen Takt und spricht über RaSTA auf
//! UDP mit `railitd` (`--node ADRESSE`). Auf dem Zielsystem übernimmt diese Rolle ein
//! Cortex-R mit `railit-embedded`; das Programm hier ist dieselbe Logik auf einem Server
//! (Prüfstand, Labor, Ausbildung).
//!
//! ```text
//! railit-node --plan beispielstadt.rplan --trusted-key railit.pub --listen 127.0.0.1:7979
//! railitd … --node 127.0.0.1:7979
//! ```
//!
//! Ohne `--peer` antwortet der Knoten an die Adresse des zuletzt empfangenen Datagramms; die
//! Echtheit jeder PDU prüft RaSTA. Für den Betrieb außerhalb eines geschützten Netzes die
//! Adresse von `railitd` mit `--peer` festlegen.

use std::io::Write;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::Parser;
use railit_rasta::MAX_PDU;
use railit_rules::GenericRules;
use railit_runtime::keys::PlanTrust;
use railit_runtime::keys::to_hex;
use railit_runtime::railit_safety::Redundancy;
use railit_runtime::remote::{NodeSetup, ServerNode, with_node};

#[derive(Debug, Parser)]
#[command(
    name = "railit-node",
    version,
    about = "Knoten des sicheren Rechners: Kern mit Voter und Watchdog, RaSTA über UDP"
)]
struct Args {
    /// Plan-Image des Stellbereichs (`railit plan compile`).
    #[arg(long)]
    plan: PathBuf,
    /// Öffentlicher Schlüssel; der Plan muss damit signiert sein.
    #[arg(
        long,
        required_unless_present = "allow_unsigned",
        conflicts_with = "allow_unsigned"
    )]
    trusted_key: Option<PathBuf>,
    /// Unsignierte Pläne zulassen (nur Simulation und Test).
    #[arg(long)]
    allow_unsigned: bool,
    /// Öffentlicher Schlüssel eines Prüfers (mehrfach angebbar, REQ-PLN-022).
    #[arg(long = "approver-key", value_name = "DATEI")]
    approver_keys: Vec<PathBuf>,
    /// Vier-Augen-Prinzip: Der Plan braucht so viele Freigaben verschiedener Prüfer.
    #[arg(long, default_value_t = 0, requires = "trusted_key")]
    approvals: usize,
    /// UDP-Adresse des Knotens.
    #[arg(long, default_value = "127.0.0.1:7979")]
    listen: SocketAddr,
    /// Adresse von `railitd` (sonst die des zuletzt empfangenen Datagramms).
    #[arg(long)]
    peer: Option<SocketAddr>,
    /// Kanäle: 1 (ohne Vergleich), 2 (2-von-2) oder 3 (2-von-3).
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=3))]
    channels: u8,
    /// Zykluszeit in Millisekunden.
    #[arg(long, default_value_t = 100)]
    cycle_ms: u64,
    /// Watchdog: Frist für die Rechenzeit eines Zyklus in Millisekunden.
    #[arg(long, default_value_t = 50)]
    deadline_ms: u64,
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("railit-node: {msg}");
    ExitCode::from(2)
}

fn micros(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// RaSTA-Zeit in Millisekunden (läuft nach 49 Tagen über, wie RaSTA es vorsieht).
fn rasta_ms(start: Instant) -> u32 {
    u32::try_from(micros(start).checked_div(1000).unwrap_or(0) & u64::from(u32::MAX)).unwrap_or(0)
}

/// Hauptschleife: Datagramme bis zum nächsten Zyklus empfangen, Zyklus rechnen, senden.
fn serve(
    node: &mut ServerNode<'static, '_>,
    socket: &UdpSocket,
    fixed_peer: Option<SocketAddr>,
    cycle: Duration,
) -> Result<(), String> {
    let start = Instant::now();
    let mut peer = fixed_peer;
    let mut next = cycle;
    let mut buf = [0u8; MAX_PDU];
    let mut was_up = false;
    let mut last_report = Duration::ZERO;
    loop {
        let now = start.elapsed();
        if now >= next {
            let begin = micros(start);
            node.tick(begin, &mut || micros(start));
            next = next.saturating_add(cycle);
            if next < now {
                // Zyklen versäumt (Rechner überlastet): Der Watchdog meldet den Abstand.
                next = now.saturating_add(cycle);
            }
            transmit(node, socket, peer, &mut buf);
            let up = node.link_up();
            if up != was_up {
                eprintln!(
                    "railit-node: Verbindung zu railitd {}",
                    if up { "steht" } else { "getrennt" }
                );
                was_up = up;
            }
            if now.saturating_sub(last_report) >= Duration::from_mins(1) {
                last_report = now;
                let s = node.stats();
                let status = node.status();
                eprintln!(
                    "railit-node: {} Zyklen, Kanäle {:?}, Abschaltung {:?}, Abbrüche {}, Protokollfehler {}",
                    s.cycles, status.active, status.trip, s.link_losses, s.protocol_errors
                );
            }
            continue;
        }
        let wait = next.saturating_sub(now).max(Duration::from_micros(100));
        socket
            .set_read_timeout(Some(wait))
            .map_err(|e| format!("UDP: {e}"))?;
        match socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                if fixed_peer.is_some_and(|p| p != from) {
                    continue;
                }
                peer = Some(from);
                node.receive(rasta_ms(start), buf.get(..n).unwrap_or_default());
                transmit(node, socket, peer, &mut buf);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(format!("UDP: {e}")),
        }
    }
}

fn transmit(
    node: &mut ServerNode<'static, '_>,
    socket: &UdpSocket,
    peer: Option<SocketAddr>,
    buf: &mut [u8; MAX_PDU],
) {
    while let Some(n) = node.poll_transmit(buf) {
        if let Some(peer) = peer {
            // Verlorene Datagramme holt RaSTA nach.
            let _ = socket.send_to(buf.get(..n).unwrap_or_default(), peer);
        }
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    let trust = match PlanTrust::from_files(
        args.trusted_key.as_deref(),
        &args.approver_keys,
        args.approvals,
    ) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let plan = match trust.load(&args.plan) {
        Ok((p, _)) => p,
        Err(e) => return fail(&e),
    };
    if trust.creator.is_none() {
        eprintln!("railit-node: unsignierter Plan zugelassen – nur für Simulation und Test");
    }
    let socket = match UdpSocket::bind(args.listen) {
        Ok(s) => s,
        Err(e) => return fail(&format!("{}: {e}", args.listen)),
    };
    let addr = socket
        .local_addr()
        .map_or_else(|_| args.listen.to_string(), |a| a.to_string());
    let cycle = Duration::from_millis(args.cycle_ms.max(1));
    let cycle_us = u64::try_from(cycle.as_micros()).unwrap_or(u64::MAX);
    let setup = NodeSetup {
        redundancy: Redundancy::from_channels(usize::from(args.channels)).unwrap_or_default(),
        deadline_us: args.deadline_ms.saturating_mul(1000),
        gap_limit_us: cycle_us.saturating_mul(5),
    };
    // Erste Zeile auf stdout: Adresse für Skripte und Tests.
    println!(
        "railit-node: udp://{addr} {} ({} Kanäle, Plan {})",
        plan.network_name(),
        args.channels,
        to_hex(&plan.digest()).get(..16).unwrap_or_default()
    );
    let _ = std::io::stdout().flush();
    // Der Knoten hält Puffer fester Größe für die Kanäle: eigener Thread mit großem Stapel.
    let peer = args.peer;
    let worker = std::thread::Builder::new()
        .name("knoten".into())
        .stack_size(64 << 20)
        .spawn(move || {
            with_node(plan, GenericRules::default(), setup, |node| {
                serve(node, &socket, peer, cycle)
            })
            .and_then(|r| r)
        });
    let result = match worker {
        Ok(handle) => handle
            .join()
            .unwrap_or_else(|_| Err("Knoten-Thread abgebrochen".into())),
        Err(e) => Err(format!("Knoten-Thread: {e}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}
