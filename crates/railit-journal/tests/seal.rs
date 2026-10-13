//! Siegel für Journal und Auditprotokoll (REQ-JRN-004).

use ed25519_dalek::SigningKey;
use railit_journal::audit::{AuditRecord, AuditWriter, verify_audit, verify_audit_sealed};
use railit_journal::{
    Cycle, Entry, Header, JournalError, JournalWriter, Rules, verify, verify_sealed,
};

fn header() -> Header {
    Header {
        format: railit_journal::FORMAT.into(),
        version: railit_journal::VERSION,
        network: "Test".into(),
        plan_digest: "00".repeat(32),
        cycle_ms: 100,
        rules: Rules::from(&railit_rules::GenericRules::default()),
        checkpoint_every: 0,
    }
}

fn cycle(n: u64) -> Entry {
    Entry::Cycle(Cycle {
        cycle: n,
        now_ms: n * 100,
        inputs: Vec::new(),
        outputs: "00".repeat(32),
        state: None,
    })
}

/// Schreibt ein Journal mit `cycles` Zyklen; mit Schlüssel gesiegelt nach dem Kopf und am Ende.
fn journal(key: Option<&SigningKey>, cycles: u64) -> Vec<u8> {
    let mut w = match key {
        Some(k) => JournalWriter::sealed(Vec::new(), header(), k.clone(), 1000).unwrap(),
        None => JournalWriter::new(Vec::new(), header()).unwrap(),
    };
    for n in 1..=cycles {
        w.append(&cycle(n)).unwrap();
    }
    w.seal().unwrap();
    w.into_inner()
}

#[test]
fn sealed_journal_verifies_with_the_station_key() {
    let station = SigningKey::from_bytes(&[7; 32]);
    let bytes = journal(Some(&station), 5);
    let s = verify_sealed(bytes.as_slice(), &station.verifying_key()).unwrap();
    assert_eq!((s.cycles, s.seals, s.unsealed), (5, 2, 0));
    assert!(s.seal_key.is_some());
    // Ohne Schlüssel: Kette und Siegel sind in sich stimmig.
    assert_eq!(verify(bytes.as_slice()).unwrap().seals, 2);

    // Ungesiegeltes Journal: gültig, aber ohne Siegel – das zeigt die Zusammenfassung.
    let plain = journal(None, 5);
    let s = verify_sealed(plain.as_slice(), &station.verifying_key()).unwrap();
    assert_eq!((s.seals, s.unsealed), (0, 5));
}

/// Eine komplett neu geschriebene Kette ist stimmig, trägt aber kein Siegel des Stellwerks.
#[test]
fn rewritten_journal_cannot_carry_the_station_seal() {
    let station = SigningKey::from_bytes(&[7; 32]);
    let attacker = SigningKey::from_bytes(&[8; 32]);
    let forged = journal(Some(&attacker), 5);
    assert!(verify(forged.as_slice()).is_ok(), "in sich stimmig");
    let err = verify_sealed(forged.as_slice(), &station.verifying_key()).unwrap_err();
    assert!(
        matches!(&err, JournalError::Seal { line: 2, message } if message.contains("fremdem")),
        "{err}"
    );
}

/// Eine veränderte Signatur fällt auf, auch wenn die Kette neu berechnet ist.
#[test]
fn tampered_signature_is_detected() {
    let station = SigningKey::from_bytes(&[7; 32]);
    let text = String::from_utf8(journal(Some(&station), 3)).unwrap();
    // Siegel der letzten Zeile verändern und die Kette dieser Zeile neu berechnen.
    let mut lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let last = lines.last_mut().unwrap();
    let sig = last["entry"]["signature"].as_str().unwrap().to_string();
    let flipped = format!(
        "{}{}",
        if sig.starts_with('0') { '1' } else { '0' },
        &sig[1..]
    );
    last["entry"]["signature"] = flipped.into();
    let prev = railit_journal_test_hash(&last["prev"], &last["entry"]);
    last["hash"] = prev.into();
    let forged: String = lines
        .iter()
        .map(|l| serde_json::to_string(l).unwrap() + "\n")
        .collect();
    let err = verify(forged.as_bytes()).unwrap_err();
    assert!(
        matches!(&err, JournalError::Seal { message, .. } if message.contains("Signatur")),
        "{err}"
    );
}

/// Kettenglied wie im Journal: SHA-256(prev ‖ JSON(entry)).
fn railit_journal_test_hash(prev: &serde_json::Value, entry: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let prev = prev.as_str().unwrap();
    let bytes: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&prev[i * 2..i * 2 + 2], 16).unwrap())
        .collect();
    // Gleiche Feldreihenfolge wie beim Schreiben: über die typisierte Struktur kodieren.
    let entry: Entry = serde_json::from_value(entry.clone()).unwrap();
    let mut h = Sha256::new();
    h.update(&bytes);
    h.update(serde_json::to_vec(&entry).unwrap());
    railit_journal::to_hex(&h.finalize())
}

fn record(user: &str) -> AuditRecord {
    AuditRecord {
        seq: 0,
        time_ms: 1,
        user: user.into(),
        action: "set_route".into(),
        target: Some("A-N1".into()),
        area: None,
        outcome: "ok".into(),
    }
}

#[test]
fn sealed_audit_log() {
    let station = SigningKey::from_bytes(&[7; 32]);
    let mut w = AuditWriter::new_sealed(Vec::new(), station.clone());
    w.append(record("fdl")).unwrap();
    w.append(record("fdl2")).unwrap();
    let bytes = w.into_inner();
    let s = verify_audit_sealed(bytes.as_slice(), &station.verifying_key()).unwrap();
    assert_eq!((s.records, s.seals, s.unsealed), (2, 2, 0));
    assert_eq!(verify_audit(bytes.as_slice()).unwrap().records, 2);

    let attacker = SigningKey::from_bytes(&[8; 32]);
    let mut w = AuditWriter::new_sealed(Vec::new(), attacker);
    w.append(record("fdl")).unwrap();
    let forged = w.into_inner();
    assert!(verify_audit_sealed(forged.as_slice(), &station.verifying_key()).is_err());

    // Ungesiegeltes Protokoll: lesbar, die Einträge sind als ungesiegelt gezählt.
    let mut w = AuditWriter::new(Vec::new());
    w.append(record("fdl")).unwrap();
    let plain = w.into_inner();
    let s = verify_audit_sealed(plain.as_slice(), &station.verifying_key()).unwrap();
    assert_eq!((s.seals, s.unsealed), (0, 1));
}

/// Ein vorhandenes Protokoll mit fremden Siegeln verhindert den Start mit dem eigenen Schlüssel.
#[test]
fn foreign_seals_block_reopening() {
    let dir = std::env::temp_dir().join(format!("railit-seal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("audit.log");
    let _ = std::fs::remove_file(&path);
    let station = SigningKey::from_bytes(&[7; 32]);
    let other = SigningKey::from_bytes(&[9; 32]);
    let mut w = AuditWriter::open_sealed(&path, other).unwrap();
    w.append(record("x")).unwrap();
    drop(w);
    assert!(AuditWriter::open_sealed(&path, station.clone()).is_err());
    std::fs::remove_file(&path).unwrap();
    let mut w = AuditWriter::open_sealed(&path, station.clone()).unwrap();
    w.append(record("y")).unwrap();
    drop(w);
    let mut w = AuditWriter::open_sealed(&path, station).unwrap();
    assert_eq!(w.append(record("z")).unwrap(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}
