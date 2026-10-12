//! Formale Nachweise mit Kani (`cargo kani -p railit-rasta -Z stubbing`), REQ-SAF-006.
//!
//! - MD4 gerät für beliebigen Inhalt in keinen Panikpfad (feste Längen: kürzeste PDU,
//!   längster Kopf vor einem zweiten Block).
//! - Die Verbindungslogik gerät für beliebige empfangene Bytes bis zu einer festen Länge in
//!   keinen Panikpfad, keinen Überlauf und keinen Zugriff außerhalb von Grenzen – geschlossen
//!   wie aufgebaut. MD4 ist dabei durch eine einfache deterministische Prüfsumme ersetzt
//!   (Stub): Der Nachweis gilt für jede Prüfsummenfunktion, die selbst nicht in Panik gerät.

#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use crate::{Config, Connection, MAX_PDU, Role, State};

const OC: u32 = 0x0B00_0001;
const IXL: u32 = 0x0A00_0001;
const LEN: usize = 48;

/// Einfache deterministische Prüfsumme anstelle von MD4.
fn checksum_stub(iv: [u32; 4], data: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, w) in iv.iter().enumerate() {
        out[i] = w.to_le_bytes()[0];
    }
    for (i, b) in data.iter().enumerate() {
        out[i % 16] ^= *b;
    }
    out
}

fn any_pdu(buf: &mut [u8; LEN]) -> &[u8] {
    *buf = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= LEN);
    &buf[..len]
}

#[kani::proof]
#[kani::unwind(20)]
fn md4_never_panics_on_shortest_pdu() {
    let data: [u8; 28] = kani::any();
    let _ = crate::md4::md4(crate::md4::STANDARD_IV, &data);
}

#[kani::proof]
#[kani::unwind(20)]
fn md4_never_panics_on_two_blocks() {
    let data: [u8; 60] = kani::any();
    let _ = crate::md4::md4(kani::any(), &data);
}

/// Eine geschlossene Verbindung stellt nie Nutzdaten zu und gerät in keinen Panikpfad.
#[kani::proof]
#[kani::stub(crate::md4::md4, checksum_stub)]
#[kani::unwind(50)]
fn closed_connection_never_delivers() {
    let mut server = Connection::new(Config::new(OC, IXL, Role::Server));
    let mut buf = [0u8; LEN];
    let pdu = any_pdu(&mut buf);
    let mut delivered = false;
    let _ = server.receive(kani::any(), pdu, &mut |_| delivered = true);
    assert!(!delivered);
}

/// Nach dem Aufbau: beliebige Bytes führen in keinen Panikpfad; die Verbindung bleibt in
/// einem definierten Zustand.
#[kani::proof]
#[kani::stub(crate::md4::md4, checksum_stub)]
#[kani::unwind(50)]
fn established_connection_survives_any_bytes() {
    let mut client = Connection::new(Config::new(IXL, OC, Role::Client));
    let mut server = Connection::new(Config::new(OC, IXL, Role::Server));
    let mut pdu = [0u8; MAX_PDU];
    client.open(0);
    while let Some(n) = client.poll_transmit(&mut pdu) {
        server.receive(0, &pdu[..n], &mut |_| {});
    }
    while let Some(n) = server.poll_transmit(&mut pdu) {
        client.receive(0, &pdu[..n], &mut |_| {});
    }
    while let Some(n) = client.poll_transmit(&mut pdu) {
        server.receive(0, &pdu[..n], &mut |_| {});
    }
    assert!(server.state() == State::Up);

    let mut buf = [0u8; LEN];
    let bytes = any_pdu(&mut buf);
    let _ = server.receive(kani::any(), bytes, &mut |_| {});
    assert!(matches!(
        server.state(),
        State::Up | State::Opening | State::Closed
    ));
}
