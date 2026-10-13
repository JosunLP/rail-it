//! RaSTA gegen die Bedrohungen nach EN 50159 (REQ-COM-001 … REQ-COM-005).

use proptest::prelude::*;
use railit_rasta::{Config, Connection, Fault, MAX_PDU, Received, Role, State};

const OC: u32 = 0x0B00_0001;
const IXL: u32 = 0x0A00_0001;

fn pair() -> (Connection, Connection) {
    let client = Connection::new(Config::new(IXL, OC, Role::Client));
    let mut server_cfg = Config::new(OC, IXL, Role::Server);
    server_cfg.initial_sn = 1000;
    (client, Connection::new(server_cfg))
}

/// Holt alle PDUs aus `from`.
fn drain(from: &mut Connection) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; MAX_PDU];
    while let Some(n) = from.poll_transmit(&mut buf) {
        out.push(buf[..n].to_vec());
    }
    out
}

/// Stellt PDUs zu; gibt die gelieferten Nutzdaten und Ergebnisse zurück.
fn deliver(to: &mut Connection, now: u32, pdus: &[Vec<u8>]) -> (Vec<Vec<u8>>, Vec<Received>) {
    let mut data = Vec::new();
    let mut results = Vec::new();
    for p in pdus {
        let r = to.receive(now, p, &mut |d| data.push(d.to_vec()));
        results.push(r);
    }
    (data, results)
}

/// Aufbau und ein Austausch, bis beide `Up` sind.
fn connect(c: &mut Connection, s: &mut Connection, now: u32) {
    c.open(now);
    let p = drain(c);
    deliver(s, now, &p);
    let p = drain(s);
    deliver(c, now, &p);
    let p = drain(c);
    deliver(s, now, &p);
    assert_eq!((c.state(), s.state()), (State::Up, State::Up));
}

#[test]
fn connection_and_data_in_both_directions() {
    let (mut c, mut s) = pair();
    connect(&mut c, &mut s, 0);
    c.send(10, b"Weiche links").unwrap();
    c.send(10, b"Signal Fahrt").unwrap();
    let p = drain(&mut c);
    let (data, _) = deliver(&mut s, 12, &p);
    assert_eq!(data, [b"Weiche links".to_vec(), b"Signal Fahrt".to_vec()]);
    s.send(20, b"Lage links").unwrap();
    let p = drain(&mut s);
    let (data, _) = deliver(&mut c, 21, &p);
    assert_eq!(data, [b"Lage links".to_vec()]);
    assert!(c.send(30, &[0u8; 300]).is_err(), "Nutzdaten zu lang");
}

#[test]
fn heartbeats_keep_the_connection_alive() {
    let (mut c, mut s) = pair();
    connect(&mut c, &mut s, 0);
    for now in (100..10_000).step_by(100) {
        assert_eq!(c.tick(now), None);
        assert_eq!(s.tick(now), None);
        let p = drain(&mut c);
        deliver(&mut s, now, &p);
        let p = drain(&mut s);
        deliver(&mut c, now, &p);
    }
    assert!(c.is_up() && s.is_up());
}

/// REQ-COM-001: Verfälschung wird am Sicherheitscode erkannt.
#[test]
fn corruption_is_detected() {
    let (mut c, mut s) = pair();
    connect(&mut c, &mut s, 0);
    c.send(10, b"Weiche links").unwrap();
    let mut p = drain(&mut c);
    p[0][HEADER_DATA] ^= 0x01;
    let (data, results) = deliver(&mut s, 11, &p);
    assert!(data.is_empty());
    assert_eq!(results, [Received::Discarded(Fault::SafetyCode)]);
    assert_eq!(s.discarded(), 1);
}

const HEADER_DATA: usize = 30;

/// REQ-COM-002: Fremde Absender werden abgewiesen (Einfügung, Maskerade).
#[test]
fn foreign_sender_is_rejected() {
    let (mut c, mut s) = pair();
    connect(&mut c, &mut s, 0);
    let mut stranger = Connection::new(Config::new(0x0C00_0001, OC, Role::Client));
    stranger.open(5);
    let p = drain(&mut stranger);
    let (_, results) = deliver(&mut s, 5, &p);
    assert_eq!(results, [Received::Discarded(Fault::Address)]);
    // Anderer Sicherheitsschlüssel (MD4-Anfangswerte): ungültiger Sicherheitscode.
    let mut cfg = Config::new(IXL, OC, Role::Client);
    cfg.md4_iv = [1, 2, 3, 4];
    let mut other_key = Connection::new(cfg);
    other_key.open(6);
    let p = drain(&mut other_key);
    let (_, results) = deliver(&mut s, 6, &p);
    assert_eq!(results, [Received::Discarded(Fault::SafetyCode)]);
}

/// REQ-COM-003: Wiederholte Nachrichten werden nur einmal geliefert.
#[test]
fn replayed_messages_are_discarded() {
    let (mut c, mut s) = pair();
    connect(&mut c, &mut s, 0);
    c.send(10, b"Weiche links").unwrap();
    let p = drain(&mut c);
    let (data, _) = deliver(&mut s, 11, &p);
    assert_eq!(data.len(), 1);
    let (data, results) = deliver(&mut s, 12, &p);
    assert!(data.is_empty());
    assert_eq!(results, [Received::Discarded(Fault::Duplicate)]);
}

/// REQ-COM-003: Verlust wird über die Sequenznummer erkannt und durch Wiederholung behoben.
#[test]
fn lost_messages_are_retransmitted_in_order() {
    let (mut c, mut s) = pair();
    connect(&mut c, &mut s, 0);
    for m in [&b"eins"[..], b"zwei", b"drei"] {
        c.send(10, m).unwrap();
    }
    let mut p = drain(&mut c);
    p.remove(0); // „eins“ geht verloren
    let (data, _) = deliver(&mut s, 11, &p);
    assert!(data.is_empty(), "nichts außer der Reihe");
    assert_eq!(s.state(), State::RetransmissionRequested);
    let req = drain(&mut s);
    deliver(&mut c, 12, &req);
    let again = drain(&mut c);
    let (data, _) = deliver(&mut s, 13, &again);
    assert_eq!(data, [b"eins".to_vec(), b"zwei".to_vec(), b"drei".to_vec()]);
    assert_eq!(s.state(), State::Up);
}

/// REQ-COM-003: Geht die letzte Nachricht verloren, zeigt das nächste Lebenszeichen die
/// Lücke (es nennt den nächsten Datenindex des Senders); die Nachricht wird wiederholt.
#[test]
fn lost_last_message_is_noticed_by_heartbeat() {
    let (mut c, mut s) = pair();
    connect(&mut c, &mut s, 0);
    c.send(10, b"Weiche links").unwrap();
    drain(&mut c); // verloren
    let mut data = Vec::new();
    for now in (100..1_500).step_by(100) {
        c.tick(now);
        s.tick(now);
        let p = drain(&mut c);
        data.extend(deliver(&mut s, now, &p).0);
        let p = drain(&mut s);
        deliver(&mut c, now, &p);
    }
    assert_eq!(data, [b"Weiche links".to_vec()]);
    assert_eq!((c.state(), s.state()), (State::Up, State::Up));
}

/// REQ-COM-004: Zu alte Nachrichten bauen die Verbindung ab (Verzögerung).
#[test]
fn delayed_messages_break_the_connection() {
    let (mut c, mut s) = pair();
    connect(&mut c, &mut s, 0);
    s.send(10, b"Lage links").unwrap();
    let p = drain(&mut s);
    // Die Nachricht bestätigt einen Zeitstempel von t = 0; sie trifft nach 2,5 s ein.
    let (data, results) = deliver(&mut c, 2_500, &p);
    assert!(data.is_empty());
    assert_eq!(results, [Received::Disconnected(Fault::Timeliness)]);
    assert_eq!(c.state(), State::Closed);
}

/// REQ-COM-005: Ohne Meldung der Gegenstelle wird die Verbindung nach `t_max` abgebaut.
#[test]
fn silence_times_out() {
    let (mut c, mut s) = pair();
    connect(&mut c, &mut s, 0);
    assert_eq!(c.tick(1_500), None);
    assert_eq!(c.tick(1_900), Some(Fault::Timeout));
    assert_eq!(c.state(), State::Closed);
    // Der Abbau wird gemeldet.
    let p = drain(&mut c);
    let (_, results) = deliver(&mut s, 1_901, &p);
    assert!(results.contains(&Received::Disconnected(Fault::PeerDisconnected)));
    assert!(c.send(2_000, b"x").is_err());
}

#[derive(Clone, Debug)]
enum Net {
    Deliver,
    Drop,
    Duplicate,
    Swap,
}

fn net() -> impl Strategy<Value = Net> {
    prop_oneof![
        8 => Just(Net::Deliver),
        1 => Just(Net::Drop),
        1 => Just(Net::Duplicate),
        1 => Just(Net::Swap),
    ]
}

/// Verlust, Verdopplung und Vertauschung auf der Strecke.
fn manipulate<'a>(mut p: Vec<Vec<u8>>, net: &mut impl Iterator<Item = &'a Net>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(pdu) = (!p.is_empty()).then(|| p.remove(0)) {
        match net.next().unwrap() {
            Net::Deliver => out.push(pdu),
            Net::Drop => {}
            Net::Duplicate => {
                out.push(pdu.clone());
                out.push(pdu);
            }
            Net::Swap => {
                if p.is_empty() {
                    out.push(pdu);
                } else {
                    let following = p.remove(0);
                    out.push(following);
                    out.push(pdu);
                }
            }
        }
    }
    out
}

proptest! {
    /// REQ-COM-003: Unter Verlust, Verdopplung und Vertauschung erhält der Empfänger nur
    /// Nachrichten in Sendereihenfolge, ohne Lücke und ohne Doppel – oder die Verbindung
    /// wird abgebaut.
    #[test]
    fn delivery_is_an_in_order_prefix(
        sends in prop::collection::vec(0u8..4, 1..40),
        nets in prop::collection::vec(net(), 1..200),
    ) {
        let (mut c, mut s) = pair();
        connect(&mut c, &mut s, 0);
        let mut sent: Vec<Vec<u8>> = Vec::new();
        let mut received: Vec<Vec<u8>> = Vec::new();
        let mut net = nets.iter().cycle();
        let mut now = 0u32;
        for (i, burst) in sends.iter().enumerate() {
            now += 50;
            for k in 0..*burst {
                let msg = vec![u8::try_from(i % 256).unwrap(), k];
                if c.send(now, &msg).is_ok() {
                    sent.push(msg);
                }
            }
            for _ in 0..3 {
                c.tick(now);
                s.tick(now);
                let p = drain(&mut c);
                let manipulated = manipulate(p, &mut net);
                let (data, _) = deliver(&mut s, now, &manipulated);
                received.extend(data);
                let back = manipulate(drain(&mut s), &mut net);
                deliver(&mut c, now, &back);
                now += 10;
            }
            prop_assert!(received.len() <= sent.len());
            prop_assert_eq!(&received[..], &sent[..received.len()]);
            if !c.is_up() || !s.is_up() {
                break;
            }
        }
    }
}

proptest! {
    /// REQ-COM-003: Daten in beide Richtungen unter Verlust, Verdopplung und Vertauschung:
    /// Jede Seite erhält nur Nachrichten der anderen in deren Sendereihenfolge, ohne Lücke und
    /// ohne Doppel – auch wenn sich Wiederholanforderungen kreuzen oder wiederholt werden.
    #[test]
    fn both_directions_deliver_in_order_prefixes(
        sends in prop::collection::vec((0u8..4, 0u8..4), 1..40),
        nets in prop::collection::vec(net(), 1..200),
    ) {
        let (mut c, mut s) = pair();
        connect(&mut c, &mut s, 0);
        let (mut sent_c, mut sent_s) = (Vec::new(), Vec::new());
        let (mut got_c, mut got_s) = (Vec::new(), Vec::new());
        let mut net = nets.iter().cycle();
        let mut now = 0u32;
        for (i, (bc, bs)) in sends.iter().enumerate() {
            now += 50;
            for k in 0..*bc {
                let msg = vec![b'c', u8::try_from(i % 256).unwrap(), k];
                if c.send(now, &msg).is_ok() {
                    sent_c.push(msg);
                }
            }
            for k in 0..*bs {
                let msg = vec![b's', u8::try_from(i % 256).unwrap(), k];
                if s.send(now, &msg).is_ok() {
                    sent_s.push(msg);
                }
            }
            for _ in 0..3 {
                c.tick(now);
                s.tick(now);
                let to_s = manipulate(drain(&mut c), &mut net);
                got_s.extend(deliver(&mut s, now, &to_s).0);
                let to_c = manipulate(drain(&mut s), &mut net);
                got_c.extend(deliver(&mut c, now, &to_c).0);
                now += 10;
            }
            prop_assert_eq!(&got_s[..], &sent_c[..got_s.len()]);
            prop_assert_eq!(&got_c[..], &sent_s[..got_c.len()]);
            if !c.is_up() || !s.is_up() {
                break;
            }
        }
    }
}

/// REQ-COM-007: Bei 20 % Verlust in beide Richtungen bleibt die Verbindung bestehen und
/// liefert alle Nachrichten (Wiederholung statt Abbau). Beide Seiten senden je Zyklus, wie
/// Stellwerk und Objektsteuerung bzw. Dienst und Knoten.
#[test]
fn twenty_percent_loss_does_not_break_the_connection() {
    let (mut c, mut s) = pair();
    connect(&mut c, &mut s, 0);
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut lose = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % 100 < 20
    };
    let (mut sent, mut got) = (0u32, Vec::new());
    for step in 0..6000u32 {
        let now = step * 10;
        if step % 10 == 0 && c.send(now, &sent.to_le_bytes()).is_ok() {
            sent += 1;
        }
        if step % 10 == 5 {
            let _ = s.send(now, b"Meldung");
        }
        assert_eq!(c.tick(now), None, "Client bei {now} ms");
        assert_eq!(s.tick(now), None, "Server bei {now} ms");
        let to_s: Vec<_> = drain(&mut c).into_iter().filter(|_| !lose()).collect();
        got.extend(deliver(&mut s, now, &to_s).0);
        let to_c: Vec<_> = drain(&mut s).into_iter().filter(|_| !lose()).collect();
        deliver(&mut c, now, &to_c);
    }
    assert!(c.is_up() && s.is_up());
    let expected: Vec<Vec<u8>> = (0..u32::try_from(got.len()).unwrap())
        .map(|i| i.to_le_bytes().to_vec())
        .collect();
    assert_eq!(got, expected, "lückenlos in Reihenfolge");
    assert!(
        got.len() + 20 >= usize::try_from(sent).unwrap(),
        "{} von {sent}",
        got.len()
    );
}
