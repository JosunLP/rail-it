//! Binärprotokoll zwischen `railitd` (Host) und dem sicheren Rechner (Knoten) über RaSTA.
//!
//! Jede RaSTA-Nachricht trägt eine oder mehrere vollständige Nachrichten dieses Protokolls,
//! kodiert mit postcard (kompakt, selbstbegrenzend, ohne Allokation). Eine Nachricht wird nie
//! auf zwei RaSTA-Nachrichten verteilt.
//!
//! - **Host → Knoten** ([`HostMsg`]): Eingaben für den nächsten Zyklus (Feld- und
//!   Grenzmeldungen, Anforderungen) sowie Handlungen an der Sicherheitsschicht.
//! - **Knoten → Host** ([`NodeMsg`]): je Zyklus `Begin`, alle Stellbefehle, Ereignisse und
//!   Meldungen der Sicherheitsschicht, dann `End` mit Anzahlen und Zustand. Der Host wendet die
//!   Befehle eines Zyklus erst an, wenn `End` vollständig und stimmig angekommen ist.
//!
//! **Zustandsabgleich:** Der Host führt einen Spiegel des Kernzustands (Anzeige,
//! Betriebsführung, RBC). Jedes `End` trägt die Anzahl der verwendeten und verworfenen
//! Eingaben und einen Digest des Zustands; damit rechnet der Host den Spiegel mit denselben
//! Eingaben nach und prüft ihn. Auf [`HostMsg::Sync`] kopiert der Knoten seinen Zustand am Ende
//! des Zyklus in einen Abgleichspeicher und überträgt ihn in den folgenden Zyklen in Teilen
//! ([`NodeMsg::SyncBegin`], [`NodeMsg::Sync`]); der Spiegel ist SIL 0, der Knoten bleibt
//! maßgeblich.
//!
//! Eine Nachricht, die sich nicht dekodieren lässt, ist ein Protokollfehler: Die Verbindung
//! wird abgebaut, die Eingaben fehlen, der Kern behandelt die Elemente nach der Meldezeit als
//! gestört (fail-safe).

use railit_kernel::{Command, Event, Input, SafeStateReason, StatePart};
use railit_rasta::MAX_PAYLOAD;
use railit_safety::{Fault, MAX_CHANNELS, SafetyEvent, Trip};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Höchstlänge einer kodierten Nachricht.
pub const MAX_MESSAGE: usize = 64;

/// Nachricht vom Host an den Knoten.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostMsg {
    /// Eingabe für den nächsten Zyklus.
    Input(Input),
    /// Sicheren Zustand zurücksetzen (vor dem nächsten Zyklus).
    Reset,
    /// Ausgeschlossenen Kanal wieder aufnehmen (vor dem nächsten Zyklus).
    Reintegrate(u8),
    /// Fehler in einen Kanal einbringen (Prüfstand).
    InjectFault(u8, Fault),
    /// Zustand für den Spiegel des Hosts übertragen (nach dem nächsten Zyklus).
    Sync,
}

/// Abschluss eines Zyklus.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleEnd {
    /// Anzahl der Stellbefehle des Zyklus.
    pub commands: u16,
    /// Anzahl der Ereignisse des Zyklus.
    pub events: u16,
    /// Sicherer Zustand des Kerns, falls aktiv.
    pub kernel: Option<SafeStateReason>,
    /// Sicherer Zustand der Sicherheitsschicht, falls aktiv.
    pub trip: Option<Trip>,
    /// Je Kanal: wählt mit.
    pub active: [bool; MAX_CHANNELS],
    /// Anzahl der Kanäle des Knotens (Redundanz).
    pub channels: u8,
    /// Eingaben gingen verloren (zu viele für den Puffer).
    pub inputs_lost: bool,
    /// Anzahl der in diesem Zyklus verwendeten Eingaben (in Empfangsreihenfolge).
    pub inputs: u16,
    /// Anzahl der danach empfangenen, verworfenen Eingaben.
    pub dropped: u16,
    /// Digest des Zustands nach dem Zyklus (erste 8 Bytes von `railit_safety::state_digest`).
    pub digest: [u8; 8],
}

/// Nachricht vom Knoten an den Host.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeMsg {
    /// Beginn der Ausgaben eines Zyklus.
    Begin {
        /// Zyklusnummer des Knotens.
        cycle: u64,
        /// Zeit des Knotens in Millisekunden.
        now_ms: u64,
    },
    /// Stellbefehl.
    Command(Command),
    /// Ereignis des Kerns.
    Event(Event),
    /// Meldung der Sicherheitsschicht.
    Safety(SafetyEvent),
    /// Ende der Ausgaben eines Zyklus.
    End(CycleEnd),
    /// Beginn eines Zustandsabgleichs: Zustand am Ende von Zyklus `cycle`.
    SyncBegin {
        /// Zyklus, dessen Endzustand übertragen wird.
        cycle: u64,
        /// Anzahl der folgenden Teile.
        parts: u32,
        /// Digest dieses Zustands (wie in [`CycleEnd::digest`]).
        digest: [u8; 8],
        /// Erste 8 Bytes des Plan-Hashes: Spiegel und Knoten müssen denselben Plan haben.
        plan: [u8; 8],
    },
    /// Teil des Zustandsabgleichs (in Reihenfolge).
    Sync(StatePart),
}

/// Fehler beim Kodieren oder Dekodieren.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    /// Nachricht nicht dekodierbar.
    Malformed,
    /// Nachricht zu lang.
    TooLong,
    /// Die Nutzdaten ließen sich nicht senden.
    Send,
}

/// Packt Nachrichten in Nutzdaten von höchstens [`MAX_PAYLOAD`] Bytes.
#[derive(Clone, Debug)]
pub struct Packer {
    buf: [u8; MAX_PAYLOAD],
    len: usize,
}

impl Default for Packer {
    fn default() -> Self {
        Self::new()
    }
}

impl Packer {
    /// Leerer Packer.
    pub const fn new() -> Self {
        Self {
            buf: [0; MAX_PAYLOAD],
            len: 0,
        }
    }

    /// Hängt `msg` an. Passt sie nicht mehr hinein, gehen die bisherigen Nutzdaten an
    /// `flush`.
    ///
    /// # Errors
    /// [`WireError::TooLong`] oder [`WireError::Send`], wenn `flush` scheitert.
    pub fn push<T: Serialize>(
        &mut self,
        msg: &T,
        flush: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), WireError> {
        let mut tmp = [0u8; MAX_MESSAGE];
        let encoded = postcard::to_slice(msg, &mut tmp).map_err(|_| WireError::TooLong)?;
        let n = encoded.len();
        if self.len.saturating_add(n) > MAX_PAYLOAD {
            self.flush(flush)?;
        }
        let end = self.len.checked_add(n).ok_or(WireError::TooLong)?;
        self.buf
            .get_mut(self.len..end)
            .ok_or(WireError::TooLong)?
            .copy_from_slice(encoded);
        self.len = end;
        Ok(())
    }

    /// Gibt angefangene Nutzdaten an `flush`.
    ///
    /// # Errors
    /// [`WireError::Send`], wenn `flush` scheitert.
    pub fn flush(&mut self, flush: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), WireError> {
        if self.len == 0 {
            return Ok(());
        }
        let ok = flush(self.buf.get(..self.len).unwrap_or(&[]));
        self.len = 0;
        if ok { Ok(()) } else { Err(WireError::Send) }
    }
}

/// Dekodiert alle Nachrichten einer Nutzlast in Reihenfolge.
///
/// # Errors
/// [`WireError::Malformed`] bei der ersten nicht dekodierbaren Nachricht; die Nachrichten
/// davor wurden bereits an `each` gegeben.
pub fn decode<T: DeserializeOwned>(
    payload: &[u8],
    each: &mut dyn FnMut(T),
) -> Result<(), WireError> {
    let mut rest = payload;
    while !rest.is_empty() {
        let (msg, tail) = postcard::take_from_bytes::<T>(rest).map_err(|_| WireError::Malformed)?;
        each(msg);
        rest = tail;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use railit_kernel::{Aspect, Occupancy, Request, RequestId};
    use railit_types::{RouteId, SignalId, TvdId};

    #[test]
    fn roundtrip_across_payloads() {
        let msgs = [
            HostMsg::Input(Input::Tvd {
                tvd: TvdId::new(3),
                occupancy: Occupancy::Occupied,
            }),
            HostMsg::Input(Input::Request {
                id: RequestId(u64::MAX),
                request: Request::SetRoute(RouteId::new(7)),
            }),
            HostMsg::Reintegrate(2),
            HostMsg::Sync,
            HostMsg::InjectFault(1, Fault::Stall),
            HostMsg::Reset,
        ];
        let mut payloads: [[u8; MAX_PAYLOAD]; 64] = [[0; MAX_PAYLOAD]; 64];
        let mut lens = [0usize; 64];
        let mut count = 0;
        let mut packer = Packer::new();
        let mut flush = |p: &[u8]| {
            payloads[count][..p.len()].copy_from_slice(p);
            lens[count] = p.len();
            count += 1;
            true
        };
        for _ in 0..40 {
            for m in &msgs {
                packer.push(m, &mut flush).unwrap();
            }
        }
        packer.flush(&mut flush).unwrap();
        assert!(count > 1, "mehrere Nutzlasten");
        let mut back = 0;
        for i in 0..count {
            decode::<HostMsg>(&payloads[i][..lens[i]], &mut |m| {
                assert_eq!(m, msgs[back % msgs.len()]);
                back += 1;
            })
            .unwrap();
        }
        assert_eq!(back, 40 * msgs.len());
    }

    #[test]
    fn largest_messages_fit() {
        let mut tmp = [0u8; MAX_MESSAGE];
        let end = NodeMsg::End(CycleEnd {
            commands: u16::MAX,
            events: u16::MAX,
            kernel: Some(SafeStateReason::ConflictingRoutes(
                RouteId::new(u32::MAX),
                RouteId::new(u32::MAX),
            )),
            trip: Some(Trip::DeadlineExceeded {
                elapsed_us: u64::MAX,
                deadline_us: u64::MAX,
            }),
            active: [true; MAX_CHANNELS],
            channels: 3,
            inputs_lost: true,
            inputs: u16::MAX,
            dropped: u16::MAX,
            digest: [0xff; 8],
        });
        assert!(postcard::to_slice(&end, &mut tmp).is_ok());
        let begin = NodeMsg::SyncBegin {
            cycle: u64::MAX,
            parts: u32::MAX,
            digest: [0xff; 8],
            plan: [0xff; 8],
        };
        assert!(postcard::to_slice(&begin, &mut tmp).is_ok());
        // Größte Zustandsteile: Grenze mit Nachbarmeldung, offene Hilfshandlung, Meta.
        // Felder wie `BorderState` (postcard kodiert Strukturen als Tupel).
        let tick = railit_types::Tick::from_millis(u64::MAX);
        let message = railit_kernel::BorderMessage {
            holding: true,
            seq: u32::MAX,
            offer: true,
            approach_clear: true,
            request: true,
            exclusive: true,
        };
        let fields = (
            (true, u32::MAX, true, true, true, true),
            (Some(message), tick, Some(tick), true, true),
        );
        let mut raw = [0u8; MAX_MESSAGE];
        let raw = postcard::to_slice(&fields, &mut raw).unwrap();
        let border: railit_kernel::BorderState = postcard::from_bytes(raw).unwrap();
        let part = NodeMsg::Sync(StatePart::Border(u16::MAX, border));
        assert!(postcard::to_slice(&part, &mut tmp).is_ok(), "{border:?}");
        // Felder wie `AuxSlot` (postcard kodiert Strukturen als Tupel).
        let fields = (
            railit_kernel::AuxId(u32::MAX),
            railit_kernel::AuxOperation::ForcePoint(
                railit_types::PointId::new(u32::MAX),
                railit_types::PointPosition::Left,
            ),
            railit_kernel::OperatorId(u32::MAX),
            railit_types::Tick::from_millis(u64::MAX),
        );
        let mut raw = [0u8; MAX_MESSAGE];
        let raw = postcard::to_slice(&fields, &mut raw).unwrap();
        let aux: railit_kernel::AuxSlot = postcard::from_bytes(raw).unwrap();
        let part = NodeMsg::Sync(StatePart::Aux(u16::MAX, Some(aux)));
        assert!(postcard::to_slice(&part, &mut tmp).is_ok());
        let cmd = NodeMsg::Command(Command::SetSignal {
            signal: SignalId::new(u32::MAX),
            aspect: Aspect::Proceed,
        });
        assert!(postcard::to_slice(&cmd, &mut tmp).is_ok());
    }

    #[test]
    fn garbage_is_malformed() {
        assert_eq!(
            decode::<HostMsg>(&[0xff, 0xff, 0xff], &mut |_| {}),
            Err(WireError::Malformed)
        );
    }
}
