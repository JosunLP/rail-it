//! SHA-256 über Zustand und Ausgaben mit plattformunabhängiger Kodierung.

use core::hash::{Hash, Hasher};

use railit_kernel::KernelState;
use sha2::{Digest as _, Sha256};

/// SHA-256-Wert.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Digest(pub [u8; 32]);

/// Speist `Hash`-Werte in einen SHA-256. Ganzzahlen werden little-endian kodiert,
/// `usize`/`isize` als 64 Bit: Der Digest hängt nicht von Zielplattform oder Byte-Reihenfolge
/// ab.
pub(crate) struct Feed<'a>(pub(crate) &'a mut Sha256);

impl Hasher for Feed<'_> {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn write_u8(&mut self, i: u8) {
        self.write(&[i]);
    }

    fn write_u16(&mut self, i: u16) {
        self.write(&i.to_le_bytes());
    }

    fn write_u32(&mut self, i: u32) {
        self.write(&i.to_le_bytes());
    }

    fn write_u64(&mut self, i: u64) {
        self.write(&i.to_le_bytes());
    }

    fn write_u128(&mut self, i: u128) {
        self.write(&i.to_le_bytes());
    }

    fn write_usize(&mut self, i: usize) {
        self.write_u64(u64::try_from(i).unwrap_or(u64::MAX));
    }

    fn write_i8(&mut self, i: i8) {
        self.write(&i.to_le_bytes());
    }

    fn write_i16(&mut self, i: i16) {
        self.write(&i.to_le_bytes());
    }

    fn write_i32(&mut self, i: i32) {
        self.write(&i.to_le_bytes());
    }

    fn write_i64(&mut self, i: i64) {
        self.write(&i.to_le_bytes());
    }

    fn write_i128(&mut self, i: i128) {
        self.write(&i.to_le_bytes());
    }

    fn write_isize(&mut self, i: isize) {
        self.write_i64(i64::try_from(i).unwrap_or(i64::MAX));
    }
}

/// Hängt `value` an den SHA-256 an.
pub(crate) fn feed<T: Hash + ?Sized>(sha: &mut Sha256, value: &T) {
    value.hash(&mut Feed(sha));
}

/// Schließt einen SHA-256 ab.
pub(crate) fn finish(sha: Sha256) -> Digest {
    Digest(sha.finalize().into())
}

/// SHA-256 über den gesamten Kernzustand.
pub fn state_digest(state: &KernelState<'_>) -> Digest {
    let mut sha = Sha256::new();
    feed(&mut sha, state);
    finish(sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_is_little_endian_and_width_independent() {
        let mut a = Sha256::new();
        feed(&mut a, &7_usize);
        let mut b = Sha256::new();
        b.update(7_u64.to_le_bytes());
        assert_eq!(finish(a), finish(b));
    }
}
