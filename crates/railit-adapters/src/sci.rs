//! EULYNX-SCI-Telegramme (vereinfacht nach Baseline 4).
//!
//! ```text
//! Protokoll u8 · Nachrichtentyp u16 (big-endian) · Absender 20 Byte · Empfänger 20 Byte · Daten
//! ```
//!
//! Kennungen sind die Elementnamen (UTF-8, höchstens 20 Byte), mit `_` auf 20 Byte aufgefüllt.
//!
//! | Schnittstelle | Richtung | Typ | Nachricht | Daten |
//! |---------------|----------|-----|-----------|-------|
//! | SCI-P (0x40)  | → OC | 0x0001 | Weiche umstellen | 0x01 rechts, 0x02 links |
//! |               | ← OC | 0x000B | Weichenlage | 0x01 rechts, 0x02 links, 0x03 keine Endlage, 0x04 gestört |
//! | SCI-LS (0x30) | → OC | 0x0001 | Signalbegriff | 0x01 Halt, 0x04 Fahrt |
//! |               | → OC | 0x0002 | Ersatzsignal | 0x01 ein, 0x02 aus |
//! |               | ← OC | 0x0003 | Signalzustand | 0x01 Halt, 0x04 Fahrt, 0xFE gestört |
//! | SCI-TDS (0x20)| → OC | 0x0001 | Grundstellung (FC) | 0x01 |
//! |               | ← OC | 0x0007 | Belegung | 0x01 frei, 0x02 besetzt, 0x03 gestört |
//! | SCI-LX (0x50) | → OC | 0x0001 | Bahnübergang | 0x01 schließen, 0x02 öffnen |
//! |               | ← OC | 0x0002 | Zustand | 0x01 offen, 0x02 schließt, 0x03 geschlossen, 0x04 gestört |

use std::fmt;

use railit_kernel::{LxReport, Occupancy, PointReport, SignalReport};
use railit_types::PointPosition;

/// Länge einer Kennung.
pub const ID_LEN: usize = 20;
/// Länge des Kopfs.
pub const HEADER_LEN: usize = 3 + 2 * ID_LEN;

/// Teilschnittstelle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// Gleisfreimeldung (SCI-TDS).
    Tds,
    /// Lichtsignal (SCI-LS).
    Ls,
    /// Weiche (SCI-P).
    P,
    /// Bahnübergang (SCI-LX).
    Lx,
}

impl Protocol {
    const fn code(self) -> u8 {
        match self {
            Self::Tds => 0x20,
            Self::Ls => 0x30,
            Self::P => 0x40,
            Self::Lx => 0x50,
        }
    }

    const fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            0x20 => Self::Tds,
            0x30 => Self::Ls,
            0x40 => Self::P,
            0x50 => Self::Lx,
            _ => return None,
        })
    }
}

/// Inhalt eines Telegramms.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Weiche umstellen.
    MovePoint(PointPosition),
    /// Weichenlage.
    PointPosition(PointReport),
    /// Signalbegriff: Fahrt (`true`) oder Halt.
    IndicateAspect(bool),
    /// Ersatzsignal ein oder aus.
    Substitute(bool),
    /// Signalzustand.
    SignalStatus(SignalReport),
    /// Grundstellung des Gleisfreimeldeabschnitts (Achszählgrundstellung).
    ResetTvps,
    /// Belegung.
    Occupancy(Occupancy),
    /// Bahnübergang schließen (`true`) oder öffnen.
    LxCommand(bool),
    /// Zustand des Bahnübergangs.
    LxStatus(LxReport),
}

impl Message {
    /// Teilschnittstelle der Nachricht.
    pub const fn protocol(&self) -> Protocol {
        match self {
            Self::MovePoint(_) | Self::PointPosition(_) => Protocol::P,
            Self::IndicateAspect(_) | Self::Substitute(_) | Self::SignalStatus(_) => Protocol::Ls,
            Self::ResetTvps | Self::Occupancy(_) => Protocol::Tds,
            Self::LxCommand(_) | Self::LxStatus(_) => Protocol::Lx,
        }
    }

    const fn encode(self) -> (u16, u8) {
        match self {
            Self::MovePoint(p) => (0x0001, point_code(p)),
            Self::PointPosition(r) => (
                0x000B,
                match r {
                    PointReport::Right => 0x01,
                    PointReport::Left => 0x02,
                    PointReport::NoEndPosition => 0x03,
                    PointReport::Disturbed => 0x04,
                },
            ),
            Self::IndicateAspect(proceed) => (0x0001, if proceed { 0x04 } else { 0x01 }),
            Self::Substitute(on) => (0x0002, if on { 0x01 } else { 0x02 }),
            Self::SignalStatus(r) => (
                0x0003,
                match r {
                    SignalReport::Stop => 0x01,
                    SignalReport::Proceed => 0x04,
                    SignalReport::Defective => 0xFE,
                },
            ),
            Self::ResetTvps => (0x0001, 0x01),
            Self::Occupancy(o) => (
                0x0007,
                match o {
                    Occupancy::Clear => 0x01,
                    Occupancy::Occupied => 0x02,
                    Occupancy::Disturbed => 0x03,
                },
            ),
            Self::LxCommand(close) => (0x0001, if close { 0x01 } else { 0x02 }),
            Self::LxStatus(r) => (
                0x0002,
                match r {
                    LxReport::Open => 0x01,
                    LxReport::Closing => 0x02,
                    LxReport::Closed => 0x03,
                    LxReport::Disturbed => 0x04,
                },
            ),
        }
    }

    fn decode(protocol: Protocol, kind: u16, data: u8) -> Option<Self> {
        Some(match (protocol, kind, data) {
            (Protocol::P, 0x0001, 0x01) => Self::MovePoint(PointPosition::Right),
            (Protocol::P, 0x0001, 0x02) => Self::MovePoint(PointPosition::Left),
            (Protocol::P, 0x000B, d) => Self::PointPosition(match d {
                0x01 => PointReport::Right,
                0x02 => PointReport::Left,
                0x03 => PointReport::NoEndPosition,
                0x04 => PointReport::Disturbed,
                _ => return None,
            }),
            (Protocol::Ls, 0x0001, 0x01) => Self::IndicateAspect(false),
            (Protocol::Ls, 0x0001, 0x04) => Self::IndicateAspect(true),
            (Protocol::Ls, 0x0002, 0x01) => Self::Substitute(true),
            (Protocol::Ls, 0x0002, 0x02) => Self::Substitute(false),
            (Protocol::Ls, 0x0003, d) => Self::SignalStatus(match d {
                0x01 => SignalReport::Stop,
                0x04 => SignalReport::Proceed,
                0xFE => SignalReport::Defective,
                _ => return None,
            }),
            (Protocol::Tds, 0x0001, 0x01) => Self::ResetTvps,
            (Protocol::Tds, 0x0007, d) => Self::Occupancy(match d {
                0x01 => Occupancy::Clear,
                0x02 => Occupancy::Occupied,
                0x03 => Occupancy::Disturbed,
                _ => return None,
            }),
            (Protocol::Lx, 0x0001, 0x01) => Self::LxCommand(true),
            (Protocol::Lx, 0x0001, 0x02) => Self::LxCommand(false),
            (Protocol::Lx, 0x0002, d) => Self::LxStatus(match d {
                0x01 => LxReport::Open,
                0x02 => LxReport::Closing,
                0x03 => LxReport::Closed,
                0x04 => LxReport::Disturbed,
                _ => return None,
            }),
            _ => return None,
        })
    }
}

const fn point_code(p: PointPosition) -> u8 {
    match p {
        PointPosition::Right => 0x01,
        PointPosition::Left => 0x02,
    }
}

/// Telegramm zwischen Stellwerk und Objektsteuerung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Telegram {
    /// Absender (Elementname bzw. Kennung des Stellwerks).
    pub sender: String,
    /// Empfänger.
    pub receiver: String,
    /// Inhalt.
    pub message: Message,
}

/// Telegramm nicht lesbar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SciError {
    /// Länge falsch.
    Length,
    /// Unbekannte Teilschnittstelle oder Nachricht.
    Unknown,
    /// Kennung zu lang oder nicht ASCII.
    Identifier,
}

impl fmt::Display for SciError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Length => "Telegrammlänge falsch",
            Self::Unknown => "unbekannte Nachricht",
            Self::Identifier => "Kennung ungültig",
        })
    }
}

impl std::error::Error for SciError {}

fn put_id(out: &mut Vec<u8>, id: &str) -> Result<(), SciError> {
    if id.len() > ID_LEN || id.contains('_') {
        return Err(SciError::Identifier);
    }
    out.extend_from_slice(id.as_bytes());
    out.resize(out.len() + (ID_LEN - id.len()), b'_');
    Ok(())
}

fn get_id(raw: &[u8]) -> Result<String, SciError> {
    let text = std::str::from_utf8(raw).map_err(|_| SciError::Identifier)?;
    Ok(text.trim_end_matches('_').to_string())
}

impl Telegram {
    /// Kodiert das Telegramm.
    ///
    /// # Errors
    /// [`SciError::Identifier`] bei zu langen oder unzulässigen Kennungen.
    pub fn encode(&self) -> Result<Vec<u8>, SciError> {
        let (kind, data) = self.message.encode();
        let mut out = Vec::with_capacity(HEADER_LEN + 1);
        out.push(self.message.protocol().code());
        out.extend_from_slice(&kind.to_be_bytes());
        put_id(&mut out, &self.sender)?;
        put_id(&mut out, &self.receiver)?;
        out.push(data);
        Ok(out)
    }

    /// Liest ein Telegramm.
    ///
    /// # Errors
    /// [`SciError`], wenn Länge, Teilschnittstelle, Nachricht oder Kennung ungültig sind.
    pub fn decode(raw: &[u8]) -> Result<Self, SciError> {
        if raw.len() != HEADER_LEN + 1 {
            return Err(SciError::Length);
        }
        let protocol = raw
            .first()
            .and_then(|c| Protocol::from_code(*c))
            .ok_or(SciError::Unknown)?;
        let kind = u16::from_be_bytes([
            *raw.get(1).ok_or(SciError::Length)?,
            *raw.get(2).ok_or(SciError::Length)?,
        ]);
        let sender = get_id(raw.get(3..3 + ID_LEN).ok_or(SciError::Length)?)?;
        let receiver = get_id(raw.get(3 + ID_LEN..HEADER_LEN).ok_or(SciError::Length)?)?;
        let data = *raw.get(HEADER_LEN).ok_or(SciError::Length)?;
        let message = Message::decode(protocol, kind, data).ok_or(SciError::Unknown)?;
        Ok(Self {
            sender,
            receiver,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegrams_roundtrip() {
        let all = [
            Message::MovePoint(PointPosition::Left),
            Message::PointPosition(PointReport::NoEndPosition),
            Message::IndicateAspect(true),
            Message::Substitute(false),
            Message::SignalStatus(SignalReport::Defective),
            Message::ResetTvps,
            Message::Occupancy(Occupancy::Occupied),
            Message::LxCommand(true),
            Message::LxStatus(LxReport::Closing),
        ];
        for message in all {
            let t = Telegram {
                sender: "STW-Adorf".into(),
                receiver: "AW1".into(),
                message,
            };
            let raw = t.encode().unwrap();
            assert_eq!(raw.len(), HEADER_LEN + 1);
            assert_eq!(Telegram::decode(&raw).unwrap(), t);
        }
        let mut raw = Telegram {
            sender: "A".into(),
            receiver: "B".into(),
            message: Message::ResetTvps,
        }
        .encode()
        .unwrap();
        raw[0] = 0x99;
        assert_eq!(Telegram::decode(&raw), Err(SciError::Unknown));
        assert_eq!(Telegram::decode(&raw[..10]), Err(SciError::Length));
        let long = Telegram {
            sender: "x".repeat(21),
            receiver: "B".into(),
            message: Message::ResetTvps,
        };
        assert_eq!(long.encode(), Err(SciError::Identifier));
    }
}
