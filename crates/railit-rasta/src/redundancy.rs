//! RaSTA-Redundanzschicht (DIN VDE V 0831-200, vereinfacht, REQ-COM-011).
//!
//! Unter der Sicherheitsschicht überträgt die Redundanzschicht jede PDU gleichzeitig über
//! bis zu [`MAX_CHANNELS`] unabhängige Kanäle (getrennte Netze). Fällt ein Kanal aus oder
//! verliert er Nachrichten, kommen sie über einen anderen an; die Sicherheitsschicht merkt
//! davon nichts.
//!
//! **Rahmen** (little-endian): Länge u16 (des ganzen Rahmens), Reserve u16 (0),
//! Sequenznummer u32, dann die PDU der Sicherheitsschicht. Einen Prüfwert trägt der Rahmen
//! nicht: Die PDU schützt sich selbst (Sicherheitscode), die Redundanzschicht ist nicht
//! sicherheitsrelevant – sie filtert nur Doppel und stellt die Reihenfolge her.
//!
//! **Empfang:** Jede Sequenznummer wird höchstens einmal ausgeliefert, in aufsteigender
//! Reihenfolge. Ein Rahmen vor seiner Zeit wird bis zu `t_seq` zurückgestellt (höchstens
//! [`DEFER`] Rahmen); kommt die Lücke bis dahin nicht über einen anderen Kanal, geht es ohne
//! sie weiter (die Sicherheitsschicht fordert sie dann wieder an). Liegt eine Nummer weit
//! hinter der erwarteten, hat der Sender neu begonnen: Die Schicht synchronisiert sich auf ihn.
//!
//! **Diagnose:** Je Kanal zählt sie empfangene Rahmen; nach je `n_diagnosis` ausgelieferten
//! Nachrichten hält sie fest, wie viele davon auf jedem Kanal fehlten.

use crate::MAX_PDU;

/// Höchstzahl der Kanäle.
pub const MAX_CHANNELS: usize = 4;
/// Länge des Rahmenkopfs.
pub const RED_HEADER_LEN: usize = 8;
/// Höchstlänge eines Rahmens.
pub const MAX_FRAME: usize = RED_HEADER_LEN + MAX_PDU;
/// Plätze für zurückgestellte Rahmen (`N_defer`).
pub const DEFER: usize = 4;
/// Liegt eine Nummer so weit hinter der erwarteten, gilt der Sender als neu begonnen.
const RESYNC: u32 = 64;

/// Einstellungen der Redundanzschicht.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RedConfig {
    /// Zahl der Kanäle (1 … [`MAX_CHANNELS`]).
    pub channels: usize,
    /// Höchste Wartezeit auf eine Lücke (ms).
    pub t_seq: u32,
    /// Diagnosefenster in ausgelieferten Nachrichten.
    pub n_diagnosis: u32,
}

impl Default for RedConfig {
    fn default() -> Self {
        Self {
            channels: 2,
            t_seq: 50,
            n_diagnosis: 200,
        }
    }
}

/// Ergebnis eines empfangenen Rahmens.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RedReceived {
    /// Ausgeliefert (und gegebenenfalls zurückgestellte Nachfolger).
    Delivered,
    /// Doppel: schon über einen anderen Kanal angekommen.
    Duplicate,
    /// Vor seiner Zeit: zurückgestellt.
    Deferred,
    /// Kein gültiger Rahmen oder unbekannter Kanal.
    Invalid,
}

/// Diagnose eines Kanals.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelDiagnosis {
    /// Empfangene Rahmen insgesamt.
    pub received: u64,
    /// Fehlende Rahmen im letzten abgeschlossenen Diagnosefenster.
    pub missed_last_window: u32,
    /// Empfangene Rahmen im laufenden Fenster.
    window: u32,
}

#[derive(Copy, Clone, Debug)]
struct Deferred {
    seq: u32,
    at: u32,
    len: usize,
    pdu: [u8; MAX_PDU],
}

/// Redundanzschicht einer Verbindung (beide Richtungen).
#[derive(Clone, Debug)]
pub struct Redundancy {
    cfg: RedConfig,
    seq_tx: u32,
    /// Nächste erwartete Nummer (`None`: noch nichts empfangen).
    seq_rx: Option<u32>,
    deferred: [Option<Deferred>; DEFER],
    diagnosis: [ChannelDiagnosis; MAX_CHANNELS],
    delivered_window: u32,
    /// Ausgelieferte Nachrichten insgesamt.
    delivered: u64,
    /// Übersprungene Lücken.
    gaps: u64,
}

impl Redundancy {
    /// Neue Schicht; die Kanalzahl wird auf 1 … [`MAX_CHANNELS`] begrenzt.
    #[must_use]
    pub const fn new(mut cfg: RedConfig) -> Self {
        if cfg.channels == 0 {
            cfg.channels = 1;
        }
        if cfg.channels > MAX_CHANNELS {
            cfg.channels = MAX_CHANNELS;
        }
        Self {
            cfg,
            seq_tx: 0,
            seq_rx: None,
            deferred: [None; DEFER],
            diagnosis: [ChannelDiagnosis {
                received: 0,
                missed_last_window: 0,
                window: 0,
            }; MAX_CHANNELS],
            delivered_window: 0,
            delivered: 0,
            gaps: 0,
        }
    }

    /// Einstellungen.
    #[must_use]
    pub const fn config(&self) -> RedConfig {
        self.cfg
    }

    /// Verpackt eine PDU der Sicherheitsschicht in einen Rahmen, der unverändert über alle
    /// Kanäle geht. `None`, wenn die PDU zu lang oder `out` zu kurz ist.
    pub fn wrap(&mut self, pdu: &[u8], out: &mut [u8]) -> Option<usize> {
        if pdu.len() > MAX_PDU {
            return None;
        }
        let len = RED_HEADER_LEN.checked_add(pdu.len())?;
        let frame = out.get_mut(..len)?;
        let (head, body) = frame.split_at_mut(RED_HEADER_LEN);
        head.get_mut(0..2)?
            .copy_from_slice(&u16::try_from(len).ok()?.to_le_bytes());
        head.get_mut(2..4)?.copy_from_slice(&[0, 0]);
        head.get_mut(4..8)?
            .copy_from_slice(&self.seq_tx.to_le_bytes());
        body.copy_from_slice(pdu);
        self.seq_tx = self.seq_tx.wrapping_add(1);
        Some(len)
    }

    /// Nimmt einen Rahmen von Kanal `channel` an und liefert PDUs in Reihenfolge aus.
    pub fn receive(
        &mut self,
        now: u32,
        channel: usize,
        frame: &[u8],
        deliver: &mut dyn FnMut(&[u8]),
    ) -> RedReceived {
        if channel >= self.cfg.channels {
            return RedReceived::Invalid;
        }
        let Some((seq, pdu)) = parse(frame) else {
            return RedReceived::Invalid;
        };
        if let Some(d) = self.diagnosis.get_mut(channel) {
            d.received = d.received.saturating_add(1);
            d.window = d.window.saturating_add(1);
        }
        let expected = *self.seq_rx.get_or_insert(seq);
        let ahead = seq.wrapping_sub(expected);
        if ahead == 0 {
            self.deliver(pdu, deliver);
            self.release(deliver);
            return RedReceived::Delivered;
        }
        if ahead > u32::MAX / 2 {
            // Hinter der erwarteten Nummer: Doppel – oder der Sender hat neu begonnen.
            if expected.wrapping_sub(seq) <= RESYNC {
                return RedReceived::Duplicate;
            }
            self.deferred = [None; DEFER];
            self.seq_rx = Some(seq);
            self.deliver(pdu, deliver);
            return RedReceived::Delivered;
        }
        if self.deferred.iter().flatten().any(|d| d.seq == seq) {
            return RedReceived::Duplicate;
        }
        if let Some(slot) = self.deferred.iter_mut().find(|d| d.is_none()) {
            let mut buf = [0u8; MAX_PDU];
            if let Some(dst) = buf.get_mut(..pdu.len()) {
                dst.copy_from_slice(pdu);
            }
            *slot = Some(Deferred {
                seq,
                at: now,
                len: pdu.len(),
                pdu: buf,
            });
            return RedReceived::Deferred;
        }
        // Kein Platz mehr: Lücke überspringen, Zurückgestelltes und diesen Rahmen ausliefern.
        self.skip_to_oldest();
        self.release(deliver);
        let expected = self.seq_rx.unwrap_or(seq);
        if seq == expected {
            self.deliver(pdu, deliver);
            self.release(deliver);
            RedReceived::Delivered
        } else {
            self.receive_again(now, channel, frame, deliver)
        }
    }

    /// Zweiter Versuch nach dem Überspringen (zählt nicht doppelt in der Diagnose).
    fn receive_again(
        &mut self,
        now: u32,
        channel: usize,
        frame: &[u8],
        deliver: &mut dyn FnMut(&[u8]),
    ) -> RedReceived {
        if let Some(d) = self.diagnosis.get_mut(channel) {
            d.received = d.received.saturating_sub(1);
            d.window = d.window.saturating_sub(1);
        }
        self.receive(now, channel, frame, deliver)
    }

    /// Liefert Zurückgestelltes aus, dessen Lücke länger als `t_seq` besteht.
    pub fn tick(&mut self, now: u32, deliver: &mut dyn FnMut(&[u8])) {
        let overdue = self
            .deferred
            .iter()
            .flatten()
            .any(|d| now.wrapping_sub(d.at) >= self.cfg.t_seq);
        if overdue {
            self.skip_to_oldest();
        }
        self.release(deliver);
    }

    /// Diagnose je Kanal.
    #[must_use]
    pub fn diagnosis(&self) -> &[ChannelDiagnosis] {
        self.diagnosis.get(..self.cfg.channels).unwrap_or_default()
    }

    /// Ausgelieferte Nachrichten und übersprungene Lücken.
    #[must_use]
    pub const fn counters(&self) -> (u64, u64) {
        (self.delivered, self.gaps)
    }

    fn deliver(&mut self, pdu: &[u8], deliver: &mut dyn FnMut(&[u8])) {
        deliver(pdu);
        self.seq_rx = self.seq_rx.map(|s| s.wrapping_add(1));
        self.delivered = self.delivered.saturating_add(1);
        self.delivered_window = self.delivered_window.saturating_add(1);
        if self.delivered_window >= self.cfg.n_diagnosis.max(1) {
            let window = self.delivered_window;
            for d in &mut self.diagnosis {
                d.missed_last_window = window.saturating_sub(d.window);
                d.window = 0;
            }
            self.delivered_window = 0;
        }
    }

    /// Liefert zurückgestellte Rahmen aus, die jetzt an der Reihe sind.
    fn release(&mut self, deliver: &mut dyn FnMut(&[u8])) {
        while let Some(expected) = self.seq_rx {
            let Some(slot) = self
                .deferred
                .iter_mut()
                .find(|d| d.is_some_and(|d| d.seq == expected))
            else {
                break;
            };
            let Some(d) = slot.take() else {
                break;
            };
            self.deliver(d.pdu.get(..d.len).unwrap_or_default(), deliver);
        }
    }

    /// Überspringt die Lücke bis zum ältesten zurückgestellten Rahmen.
    fn skip_to_oldest(&mut self) {
        let Some(expected) = self.seq_rx else {
            return;
        };
        let oldest = self
            .deferred
            .iter()
            .flatten()
            .map(|d| d.seq)
            .min_by_key(|s| s.wrapping_sub(expected));
        if let Some(oldest) = oldest {
            self.gaps = self.gaps.saturating_add(1);
            self.seq_rx = Some(oldest);
        }
    }
}

/// Zerlegt einen Rahmen in Sequenznummer und PDU.
#[must_use]
pub fn parse(frame: &[u8]) -> Option<(u32, &[u8])> {
    let len = usize::from(u16::from_le_bytes(frame.get(0..2)?.try_into().ok()?));
    if len != frame.len() || len < RED_HEADER_LEN || frame.get(2..4)? != [0, 0] {
        return None;
    }
    let seq = u32::from_le_bytes(frame.get(4..8)?.try_into().ok()?);
    Some((seq, frame.get(RED_HEADER_LEN..)?))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;

    use super::*;

    fn frames(red: &mut Redundancy, n: u8) -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| {
                let mut out = [0u8; MAX_FRAME];
                let len = red.wrap(&[i; 20], &mut out).unwrap();
                out[..len].to_vec()
            })
            .collect()
    }

    fn rx() -> (Redundancy, Vec<u8>) {
        (Redundancy::new(RedConfig::default()), Vec::new())
    }

    #[test]
    fn duplicates_from_the_second_channel_are_filtered() {
        let mut tx = Redundancy::new(RedConfig::default());
        let f = frames(&mut tx, 5);
        let (mut red, mut got) = rx();
        for frame in &f {
            assert_eq!(
                red.receive(0, 0, frame, &mut |p| got.push(p[0])),
                RedReceived::Delivered
            );
            assert_eq!(
                red.receive(0, 1, frame, &mut |p| got.push(p[0])),
                RedReceived::Duplicate
            );
        }
        assert_eq!(got, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_lost_frame_comes_over_the_other_channel() {
        let mut tx = Redundancy::new(RedConfig::default());
        let f = frames(&mut tx, 4);
        let (mut red, mut got) = rx();
        // Kanal 0 verliert Rahmen 1, Kanal 1 bringt ihn verspätet.
        red.receive(0, 0, &f[0], &mut |p| got.push(p[0]));
        assert_eq!(
            red.receive(1, 0, &f[2], &mut |p| got.push(p[0])),
            RedReceived::Deferred
        );
        red.receive(2, 0, &f[3], &mut |p| got.push(p[0]));
        assert_eq!(got, [0]);
        red.receive(3, 1, &f[1], &mut |p| got.push(p[0]));
        assert_eq!(got, [0, 1, 2, 3]);
        for frame in &f {
            red.receive(4, 1, frame, &mut |p| got.push(p[0]));
        }
        assert_eq!(got, [0, 1, 2, 3], "alles Weitere sind Doppel");
    }

    #[test]
    fn a_gap_is_skipped_after_t_seq() {
        let mut tx = Redundancy::new(RedConfig::default());
        let f = frames(&mut tx, 3);
        let (mut red, mut got) = rx();
        red.receive(0, 0, &f[0], &mut |p| got.push(p[0]));
        red.receive(10, 0, &f[2], &mut |p| got.push(p[0]));
        red.tick(40, &mut |p| got.push(p[0]));
        assert_eq!(got, [0]);
        red.tick(60, &mut |p| got.push(p[0]));
        assert_eq!(got, [0, 2]);
        assert_eq!(red.counters(), (2, 1));
        // Der verspätete Rahmen 1 ist jetzt ein Doppel.
        assert_eq!(
            red.receive(70, 1, &f[1], &mut |p| got.push(p[0])),
            RedReceived::Duplicate
        );
    }

    #[test]
    fn a_full_defer_queue_skips_the_gap() {
        let mut tx = Redundancy::new(RedConfig::default());
        let f = frames(&mut tx, 8);
        let (mut red, mut got) = rx();
        red.receive(0, 0, &f[0], &mut |p| got.push(p[0]));
        for frame in &f[2..7] {
            red.receive(1, 0, frame, &mut |p| got.push(p[0]));
        }
        assert_eq!(got, [0, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn a_restarted_sender_is_followed() {
        let mut tx = Redundancy::new(RedConfig::default());
        let f = frames(&mut tx, 100);
        let (mut red, mut got) = rx();
        for frame in &f {
            red.receive(0, 0, frame, &mut |p| got.push(p[0]));
        }
        let mut fresh = Redundancy::new(RedConfig::default());
        let g = frames(&mut fresh, 2);
        got.clear();
        for frame in &g {
            assert_eq!(
                red.receive(1, 0, frame, &mut |p| got.push(p[0])),
                RedReceived::Delivered
            );
        }
        assert_eq!(got, [0, 1]);
    }

    #[test]
    fn diagnosis_counts_missing_frames_per_channel() {
        let cfg = RedConfig {
            n_diagnosis: 10,
            ..RedConfig::default()
        };
        let mut tx = Redundancy::new(cfg);
        let f = frames(&mut tx, 10);
        let mut red = Redundancy::new(cfg);
        for (i, frame) in f.iter().enumerate() {
            red.receive(0, 0, frame, &mut |_| {});
            if i % 2 == 0 {
                red.receive(0, 1, frame, &mut |_| {});
            }
        }
        let d = red.diagnosis();
        assert_eq!((d[0].missed_last_window, d[1].missed_last_window), (0, 5));
        assert_eq!(d[1].received, 5);
    }

    #[test]
    fn malformed_frames_are_rejected() {
        let (mut red, mut got) = rx();
        let mut tx = Redundancy::new(RedConfig::default());
        let mut f = frames(&mut tx, 1).remove(0);
        assert_eq!(
            red.receive(0, 2, &f, &mut |p| got.push(p[0])),
            RedReceived::Invalid,
            "nur zwei Kanäle"
        );
        f[2] = 1;
        assert_eq!(
            red.receive(0, 0, &f, &mut |p| got.push(p[0])),
            RedReceived::Invalid
        );
        assert_eq!(
            red.receive(0, 0, &f[..5], &mut |p| got.push(p[0])),
            RedReceived::Invalid
        );
        assert!(got.is_empty());
    }
}
