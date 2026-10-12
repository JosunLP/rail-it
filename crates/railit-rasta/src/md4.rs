//! MD4 (RFC 1320) mit einstellbaren Anfangswerten, wie ihn RaSTA für den Sicherheitscode
//! verwendet. MD4 dient hier nicht als kryptografischer Schutz, sondern als Prüfcode gegen
//! Verfälschung (EN 50159, Kategorie 2: geschlossenes Übertragungssystem).

/// Standard-Anfangswerte nach RFC 1320.
pub const STANDARD_IV: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];

/// Konstante der zweiten Runde.
const R2: u32 = 0x5a82_7999;
/// Konstante der dritten Runde.
const R3: u32 = 0x6ed9_eba1;

const fn f(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | (!x & z)
}

const fn g(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | (x & z) | (y & z)
}

const fn h(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}

// Bezeichner wie in RFC 1320.
#[allow(clippy::many_single_char_names)]
fn block(state: &mut [u32; 4], chunk: &[u8; 64]) {
    let mut x = [0u32; 16];
    for (word, bytes) in x.iter_mut().zip(chunk.chunks_exact(4)) {
        *word = u32::from_le_bytes([
            bytes.first().copied().unwrap_or(0),
            bytes.get(1).copied().unwrap_or(0),
            bytes.get(2).copied().unwrap_or(0),
            bytes.get(3).copied().unwrap_or(0),
        ]);
    }
    let w = |i: usize| x.get(i).copied().unwrap_or(0);
    let [mut a, mut b, mut c, mut d] = *state;

    // Runde 1
    for &(k, s) in &[(0, 3), (4, 3), (8, 3), (12, 3)] {
        a = a.wrapping_add(f(b, c, d)).wrapping_add(w(k)).rotate_left(s);
        d = d
            .wrapping_add(f(a, b, c))
            .wrapping_add(w(k.wrapping_add(1)))
            .rotate_left(7);
        c = c
            .wrapping_add(f(d, a, b))
            .wrapping_add(w(k.wrapping_add(2)))
            .rotate_left(11);
        b = b
            .wrapping_add(f(c, d, a))
            .wrapping_add(w(k.wrapping_add(3)))
            .rotate_left(19);
    }
    // Runde 2
    for k in 0..4 {
        a = a
            .wrapping_add(g(b, c, d))
            .wrapping_add(w(k))
            .wrapping_add(R2)
            .rotate_left(3);
        d = d
            .wrapping_add(g(a, b, c))
            .wrapping_add(w(k.wrapping_add(4)))
            .wrapping_add(R2)
            .rotate_left(5);
        c = c
            .wrapping_add(g(d, a, b))
            .wrapping_add(w(k.wrapping_add(8)))
            .wrapping_add(R2)
            .rotate_left(9);
        b = b
            .wrapping_add(g(c, d, a))
            .wrapping_add(w(k.wrapping_add(12)))
            .wrapping_add(R2)
            .rotate_left(13);
    }
    // Runde 3
    for &k in &[0, 2, 1, 3] {
        a = a
            .wrapping_add(h(b, c, d))
            .wrapping_add(w(k))
            .wrapping_add(R3)
            .rotate_left(3);
        d = d
            .wrapping_add(h(a, b, c))
            .wrapping_add(w(k.wrapping_add(8)))
            .wrapping_add(R3)
            .rotate_left(9);
        c = c
            .wrapping_add(h(d, a, b))
            .wrapping_add(w(k.wrapping_add(4)))
            .wrapping_add(R3)
            .rotate_left(11);
        b = b
            .wrapping_add(h(c, d, a))
            .wrapping_add(w(k.wrapping_add(12)))
            .wrapping_add(R3)
            .rotate_left(15);
    }
    let [sa, sb, sc, sd] = state;
    *sa = sa.wrapping_add(a);
    *sb = sb.wrapping_add(b);
    *sc = sc.wrapping_add(c);
    *sd = sd.wrapping_add(d);
}

/// MD4 über `data` mit den Anfangswerten `iv`.
pub fn md4(iv: [u32; 4], data: &[u8]) -> [u8; 16] {
    let mut state = iv;
    let mut chunks = data.chunks_exact(64);
    for chunk in &mut chunks {
        let mut buf = [0u8; 64];
        buf.copy_from_slice(chunk);
        block(&mut state, &buf);
    }
    let rest = chunks.remainder();
    let mut tail = [0u8; 128];
    let n = rest.len();
    if let Some(dst) = tail.get_mut(..n) {
        dst.copy_from_slice(rest);
    }
    if let Some(b) = tail.get_mut(n) {
        *b = 0x80;
    }
    let total: usize = if n < 56 { 64 } else { 128 };
    let bits = u64::try_from(data.len())
        .unwrap_or(u64::MAX)
        .wrapping_mul(8);
    if let Some(dst) = tail.get_mut(total.saturating_sub(8)..total) {
        dst.copy_from_slice(&bits.to_le_bytes());
    }
    for chunk in tail.get(..total).unwrap_or(&[]).chunks_exact(64) {
        let mut buf = [0u8; 64];
        buf.copy_from_slice(chunk);
        block(&mut state, &buf);
    }
    let mut out = [0u8; 16];
    for (dst, word) in out.chunks_exact_mut(4).zip(state) {
        dst.copy_from_slice(&word.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(d: [u8; 16]) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, b) in d.iter().enumerate() {
            let digits = b"0123456789abcdef";
            out[i * 2] = digits[usize::from(b >> 4)];
            out[i * 2 + 1] = digits[usize::from(b & 15)];
        }
        out
    }

    /// Testvektoren aus RFC 1320.
    #[test]
    fn rfc1320_vectors() {
        let cases: [(&[u8], &[u8; 32]); 7] = [
            (b"", b"31d6cfe0d16ae931b73c59d7e0c089c0"),
            (b"a", b"bde52cb31de33e46245e05fbdbd6fb24"),
            (b"abc", b"a448017aaf21d8525fc10ae87aa6729d"),
            (b"message digest", b"d9130a8164549fe818874806e1c7014b"),
            (
                b"abcdefghijklmnopqrstuvwxyz",
                b"d79e1c308aa5bbcdeea8ed63df412da9",
            ),
            (
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                b"043f8582f241db351ce627e153e7f0e4",
            ),
            (
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                b"e33b4ddc9c38f2199c3e7b164fcc0536",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(&hex(md4(STANDARD_IV, input)), want);
        }
    }
}
