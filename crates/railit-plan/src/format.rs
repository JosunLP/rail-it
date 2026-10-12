//! Binärformat des Plan-Images, Version 4.
//!
//! Alle Zahlen sind little-endian. Offsets sind Byte-Offsets ab Anfang des Images.
//!
//! ```text
//! ┌──────────────────────────────────┐ 0
//! │ Kopf: 24 × u32                   │
//! ├──────────────────────────────────┤ route_off
//! │ Fahrstraßen: n × 18 × u32        │
//! ├──────────────────────────────────┤ pool_off
//! │ Pool: u32 (Weichen, TVDs, Flanken,│
//! │       Bahnübergänge)             │
//! ├──────────────────────────────────┤ point_tvd_off
//! │ TVD je Weiche: n × u32           │
//! ├──────────────────────────────────┤ signal_off
//! │ Signale: n × 3 × u32             │   Art, System, angekündigtes Signal
//! ├──────────────────────────────────┤ tvd_flags_off
//! │ TVD-Merkmale: n × u32            │   Bit 0: Einfahrabschnitt (an Netzgrenze)
//! ├──────────────────────────────────┤ lx_tvd_off
//! │ TVD je Bahnübergang: n × u32     │
//! ├──────────────────────────────────┤ border_off
//! │ Grenzen: n × 8 × u32             │   Merkmale, Einfahrsignal, Annäherungsabschnitt
//! │                                  │   (TVDs, Weichen) als Pool-Listen
//! ├──────────────────────────────────┤ conflict_off
//! │ Feindlichkeiten: n × w × u64     │   w = ⌈Anzahl Fahrstraßen / 64⌉
//! ├──────────────────────────────────┤ names_off
//! │ Namen: k × (off, len) u32        │   Netz, Weichen, Signale, TVDs, Fahrstraßen, BÜ,
//! │                                  │   Grenzen
//! │        UTF-8-Daten               │
//! ├──────────────────────────────────┤ body_len
//! │ SHA-256 über [0, body_len)       │ 32 Byte
//! │ Ed25519-Signatur über Digest     │ 64 Byte, nur wenn FLAG_SIGNED
//! └──────────────────────────────────┘
//! ```
//!
//! **Kopf** (Index des u32-Worts):
//!
//! | # | Inhalt | # | Inhalt |
//! |---|--------|---|--------|
//! | 0 | Kennung `RPLN` | 12 | `point_tvd_off` |
//! | 1 | Formatversion | 13 | `signal_off` |
//! | 2 | `body_len` | 14 | `tvd_flags_off` |
//! | 3 | Anzahl Weichen | 15 | `lx_tvd_off` |
//! | 4 | Anzahl Signale | 16 | `names_off` |
//! | 5 | Anzahl TVDs | 17 | Länge Namensabschnitt in Byte |
//! | 6 | Anzahl Fahrstraßen | 18 | Flags (Bit 0: signiert) |
//! | 7 | Anzahl Bahnübergänge | 19 | Anzahl Grenzen |
//! | | | 20 | `border_off` |
//! | | | 21–23 | reserviert (0) |
//! | 8 | `route_off` | | |
//! | 9 | `pool_off` | | |
//! | 10 | Anzahl Pool-Wörter | | |
//! | 11 | `conflict_off` | | |
//!
//! **Fahrstraße** (18 × u32): Startsignal, Zielart (0 Signal, 1 Netzgrenze, 2 Prellbock,
//! 3 Bereichsgrenze), Zielsignal bzw. Grenze (oder `u32::MAX`), Geschwindigkeit in km/h (0 = keine Beschränkung), dann je
//! Pool-Start und Anzahl für: Weichen, TVDs (Fahrreihenfolge), TVDs des Durchrutschwegs,
//! Weichen des Durchrutschwegs, Flankenschutz-Einträge, Bahnübergänge; zuletzt die Länge
//! vom Startsignal bis zum Ziel und die Länge des Durchrutschwegs, beide in mm (seit v4, für
//! die Fahrerlaubnis nach ETCS).
//!
//! **Weicheneintrag** im Pool: `(Weichen-ID << 1) | Lage` mit Lage 0 = links, 1 = rechts.
//!
//! **Flankenschutz-Eintrag** im Pool (3 × u32): Art (0 Weiche in Lage, 1 Signal in Halt),
//! Weicheneintrag bzw. Signal-ID, geschützter TVD der Fahrstraße.
//!
//! **Grenze** (8 × u32): Merkmale (Bit 0: übernimmt beim Anlauf die Erlaubnis),
//! Einfahrsignal, dann je Pool-Start und Anzahl für die Abschnitte und die Weichen des
//! Annäherungsabschnitts (von der Grenze bis zum Einfahrsignal und dessen Durchrutschweg),
//! zuletzt die Länge von der Grenze bis zum Einfahrsignal und die Länge seines
//! Durchrutschwegs in mm (seit v4).

/// Kennung am Anfang jedes Plan-Images.
pub const MAGIC: [u8; 4] = *b"RPLN";
/// Unterstützte Formatversion.
pub const VERSION: u32 = 4;
/// Länge des Kopfs in Byte.
pub const HEADER_LEN: usize = 96;
/// Länge eines Fahrstraßeneintrags in Byte.
pub const ROUTE_LEN: usize = 72;
/// Wörter je Signaleintrag.
pub const SIGNAL_WORDS: usize = 3;
/// Wörter je Flankenschutz-Eintrag.
pub const FLANK_WORDS: usize = 3;
/// Wörter je Grenzeintrag.
pub const BORDER_WORDS: usize = 8;
/// Grenzmerkmal: Dieser Bereich übernimmt beim Anlauf die Erlaubnis.
pub const BORDER_INITIAL: u32 = 1;
/// Länge der SHA-256-Prüfsumme in Byte.
pub const DIGEST_LEN: usize = 32;
/// Länge der Ed25519-Signatur in Byte.
pub const SIGNATURE_LEN: usize = 64;
/// Flag: Das Image trägt eine Signatur.
pub const FLAG_SIGNED: u32 = 1;
/// TVD-Merkmal: Einfahrabschnitt an einer Netzgrenze.
pub const TVD_ENTRY: u32 = 1;

pub(crate) const H_VERSION: usize = 1;
pub(crate) const H_BODY_LEN: usize = 2;
pub(crate) const H_POINTS: usize = 3;
pub(crate) const H_SIGNALS: usize = 4;
pub(crate) const H_TVDS: usize = 5;
pub(crate) const H_ROUTES: usize = 6;
pub(crate) const H_LXS: usize = 7;
pub(crate) const H_ROUTE_OFF: usize = 8;
pub(crate) const H_POOL_OFF: usize = 9;
pub(crate) const H_POOL_WORDS: usize = 10;
pub(crate) const H_CONFLICT_OFF: usize = 11;
pub(crate) const H_POINT_TVD_OFF: usize = 12;
pub(crate) const H_SIGNAL_OFF: usize = 13;
pub(crate) const H_TVD_FLAGS_OFF: usize = 14;
pub(crate) const H_LX_TVD_OFF: usize = 15;
pub(crate) const H_NAMES_OFF: usize = 16;
pub(crate) const H_NAMES_LEN: usize = 17;
pub(crate) const H_FLAGS: usize = 18;
pub(crate) const H_BORDERS: usize = 19;
pub(crate) const H_BORDER_OFF: usize = 20;

/// Wortindex in einem Fahrstraßeneintrag.
pub(crate) mod r {
    pub(crate) const START: usize = 0;
    pub(crate) const END_KIND: usize = 1;
    pub(crate) const END_SIGNAL: usize = 2;
    pub(crate) const SPEED: usize = 3;
    pub(crate) const POINTS: usize = 4;
    pub(crate) const TVDS: usize = 6;
    pub(crate) const OVERLAP_TVDS: usize = 8;
    pub(crate) const OVERLAP_POINTS: usize = 10;
    pub(crate) const FLANK: usize = 12;
    pub(crate) const LXS: usize = 14;
    pub(crate) const LENGTH: usize = 16;
    pub(crate) const OVERLAP_LENGTH: usize = 17;
}

pub(crate) const END_SIGNAL: u32 = 0;
pub(crate) const END_BOUNDARY: u32 = 1;
pub(crate) const END_BUFFER_STOP: u32 = 2;
pub(crate) const END_BORDER: u32 = 3;
pub(crate) const NONE: u32 = u32::MAX;
pub(crate) const FLANK_POINT: u32 = 0;
pub(crate) const FLANK_SIGNAL: u32 = 1;

/// Liest ein little-endian `u32` an Byte-Offset `off`.
pub(crate) fn read_u32(bytes: &[u8], off: usize) -> Option<u32> {
    let raw = bytes.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes(raw.try_into().ok()?))
}

/// Liest ein little-endian `u64` an Byte-Offset `off`.
pub(crate) fn read_u64(bytes: &[u8], off: usize) -> Option<u64> {
    let raw = bytes.get(off..off.checked_add(8)?)?;
    Some(u64::from_le_bytes(raw.try_into().ok()?))
}

/// Liest das u32-Wort Nummer `word` ab Byte-Offset `base`.
pub(crate) fn read_word(bytes: &[u8], base: usize, word: usize) -> Option<u32> {
    read_u32(bytes, base.checked_add(word.checked_mul(4)?)?)
}

/// Kodiert einen Weicheneintrag des Pools.
#[cfg(feature = "alloc")]
pub(crate) fn encode_point(
    point: railit_types::PointId,
    pos: railit_types::PointPosition,
) -> Option<u32> {
    let shifted = point.raw().checked_mul(2)?;
    Some(match pos {
        railit_types::PointPosition::Left => shifted,
        railit_types::PointPosition::Right => shifted | 1,
    })
}

/// Dekodiert einen Weicheneintrag des Pools.
pub(crate) fn decode_point(entry: u32) -> (railit_types::PointId, railit_types::PointPosition) {
    let pos = if entry & 1 == 0 {
        railit_types::PointPosition::Left
    } else {
        railit_types::PointPosition::Right
    };
    (railit_types::PointId::new(entry >> 1), pos)
}
