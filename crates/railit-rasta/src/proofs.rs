//! Formale Nachweise mit Kani (`cargo kani -p railit-rasta -Z stubbing`), REQ-SAF-006.
//!
//! - MD4 gerät für beliebigen Inhalt in keinen Panikpfad (feste Längen: kürzeste PDU,
//!   längster Kopf vor einem zweiten Block).
//! - Prüfung und Verarbeitung beliebiger empfangener Bytes bis zu einer festen Länge geraten
//!   in keinen Panikpfad, keinen Überlauf und keinen Zugriff außerhalb von Grenzen. MD4 ist
//!   dabei durch eine einfache deterministische Prüfsumme ersetzt (Stub): Der Nachweis gilt
//!   für jede Prüfsummenfunktion, die selbst nicht in Panik gerät.
//!
//! Die aufgebaute Verbindung unter beliebigen PDUs ist für Bounded Model Checking zu groß
//! (Sende- und Empfangspuffer); sie prüfen das Fuzz-Ziel `rasta` und der Property-Test
//! `delivery_is_an_in_order_prefix`.

#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use crate::{Config, Connection, Role};

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
#[kani::unwind(64)]
fn closed_connection_never_delivers() {
    let mut server = Connection::new(Config::new(OC, IXL, Role::Server));
    let mut buf = [0u8; LEN];
    let pdu = any_pdu(&mut buf);
    let mut delivered = false;
    let _ = server.receive(kani::any(), pdu, &mut |_| delivered = true);
    assert!(!delivered);
}
