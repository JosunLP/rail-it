//! Binärformat des Plan-Images, Version 1.
//!
//! Alle Zahlen sind little-endian. Offsets sind Byte-Offsets ab Anfang des Images.
//!
//! ```text
//! ┌──────────────────────────────┐ 0
//! │ Kopf: 16 × u32               │
//! ├──────────────────────────────┤ route_off
//! │ Fahrstraßen: n × 8 × u32     │
//! ├──────────────────────────────┤ pool_off
//! │ Pool: u32 (Weichen, TVDs)    │
//! ├──────────────────────────────┤ point_tvd_off
//! │ TVD je Weiche: n × u32       │
//! ├──────────────────────────────┤ conflict_off
//! │ Feindlichkeiten: n × w × u64 │   w = ⌈Anzahl Fahrstraßen / 64⌉
//! ├──────────────────────────────┤ names_off
//! │ Namen: k × (off, len) u32    │   k = 1 + Weichen + Signale + TVDs + Fahrstraßen
//! │        UTF-8-Daten           │   Reihenfolge: Netz, Weichen, Signale, TVDs, Fahrstraßen
//! ├──────────────────────────────┤ body_len
//! │ SHA-256 über [0, body_len)   │ 32 Byte
//! │ Ed25519-Signatur über Digest │ 64 Byte, nur wenn FLAG_SIGNED
//! └──────────────────────────────┘
//! ```
//!
//! **Kopf** (Index des u32-Worts):
//!
//! | # | Inhalt | # | Inhalt |
//! |---|--------|---|--------|
//! | 0 | Kennung `RPLN` | 8 | `pool_off` |
//! | 1 | Formatversion | 9 | Anzahl Pool-Wörter |
//! | 2 | `body_len` | 10 | `conflict_off` |
//! | 3 | Anzahl Weichen | 11 | `point_tvd_off` |
//! | 4 | Anzahl Signale | 12 | `names_off` |
//! | 5 | Anzahl TVDs | 13 | Länge Namensabschnitt in Byte |
//! | 6 | Anzahl Fahrstraßen | 14 | Flags (Bit 0: signiert) |
//! | 7 | `route_off` | 15 | reserviert (0) |
//!
//! **Fahrstraße** (8 × u32): Startsignal, Zielart (0 Signal, 1 Netzgrenze, 2 Prellbock),
//! Zielsignal (oder `u32::MAX`), Pool-Start und Anzahl der Weichen, Pool-Start und Anzahl der
//! TVDs (in Fahrreihenfolge), reserviert.
//!
//! **Weicheneintrag** im Pool: `(Weichen-ID << 1) | Lage` mit Lage 0 = links, 1 = rechts.

/// Kennung am Anfang jedes Plan-Images.
pub const MAGIC: [u8; 4] = *b"RPLN";
/// Unterstützte Formatversion.
pub const VERSION: u32 = 1;
/// Länge des Kopfs in Byte.
pub const HEADER_LEN: usize = 64;
/// Länge eines Fahrstraßeneintrags in Byte.
pub const ROUTE_LEN: usize = 32;
/// Länge der SHA-256-Prüfsumme in Byte.
pub const DIGEST_LEN: usize = 32;
/// Länge der Ed25519-Signatur in Byte.
pub const SIGNATURE_LEN: usize = 64;
/// Flag: Das Image trägt eine Signatur.
pub const FLAG_SIGNED: u32 = 1;

pub(crate) const H_VERSION: usize = 1;
pub(crate) const H_BODY_LEN: usize = 2;
pub(crate) const H_POINTS: usize = 3;
pub(crate) const H_SIGNALS: usize = 4;
pub(crate) const H_TVDS: usize = 5;
pub(crate) const H_ROUTES: usize = 6;
pub(crate) const H_ROUTE_OFF: usize = 7;
pub(crate) const H_POOL_OFF: usize = 8;
pub(crate) const H_POOL_WORDS: usize = 9;
pub(crate) const H_CONFLICT_OFF: usize = 10;
pub(crate) const H_POINT_TVD_OFF: usize = 11;
pub(crate) const H_NAMES_OFF: usize = 12;
pub(crate) const H_NAMES_LEN: usize = 13;
pub(crate) const H_FLAGS: usize = 14;

pub(crate) const END_SIGNAL: u32 = 0;
pub(crate) const END_BOUNDARY: u32 = 1;
pub(crate) const END_BUFFER_STOP: u32 = 2;
pub(crate) const NO_SIGNAL: u32 = u32::MAX;

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
