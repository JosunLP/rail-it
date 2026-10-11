//! Bitmengen über vom Aufrufer bereitgestellten Speicher.
//!
//! Belegungen, Verschlüsse und Feindlichkeiten werden im Kern als Bitmengen geführt. Eine
//! Konfliktprüfung zwischen zwei Fahrstraßen ist damit eine Handvoll `AND`-Operationen.
//!
//! Die Typen besitzen keinen Speicher, sondern arbeiten auf `&[u64]` bzw. `&mut [u64]`.
//! So funktionieren sie unverändert mit statischen Puffern (Embedded), mit `Vec<u64>`
//! (Server) und mit Slices, die direkt aus einem Plan-Image gelesen werden.

use core::fmt;

/// Anzahl Bits pro Speicherwort.
pub const WORD_BITS: usize = 64;

/// Anzahl Wörter, die für `bits` Bits benötigt werden.
pub const fn words_for(bits: usize) -> usize {
    bits.div_ceil(WORD_BITS)
}

/// Ein Bitindex liegt außerhalb der Kapazität der Menge.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct OutOfRange {
    /// Angefragter Index.
    pub index: usize,
    /// Kapazität der Menge in Bits.
    pub capacity: usize,
}

impl fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Bitindex {} außerhalb der Kapazität {}",
            self.index, self.capacity
        )
    }
}

/// Zerlegt einen Bitindex in Wortindex und Maske.
fn locate(index: usize) -> Option<(usize, u64)> {
    let word = index.checked_div(WORD_BITS)?;
    let bit = u32::try_from(index.checked_rem(WORD_BITS)?).ok()?;
    Some((word, 1u64.checked_shl(bit)?))
}

/// Unveränderliche Sicht auf eine Bitmenge.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct BitSet<'a> {
    words: &'a [u64],
}

impl<'a> BitSet<'a> {
    /// Sicht auf die gegebenen Wörter.
    pub const fn new(words: &'a [u64]) -> Self {
        Self { words }
    }

    /// Zugrunde liegende Wörter.
    pub const fn words(&self) -> &'a [u64] {
        self.words
    }

    /// Kapazität in Bits.
    pub const fn capacity(&self) -> usize {
        self.words.len().saturating_mul(WORD_BITS)
    }

    /// Ist das Bit `index` gesetzt? Indizes außerhalb der Kapazität gelten als nicht gesetzt.
    pub fn contains(&self, index: usize) -> bool {
        locate(index)
            .and_then(|(word, mask)| self.words.get(word).map(|w| w & mask != 0))
            .unwrap_or(false)
    }

    /// Ist kein Bit gesetzt?
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    /// Anzahl gesetzter Bits.
    pub fn count(&self) -> usize {
        self.words.iter().fold(0usize, |acc, w| {
            acc.saturating_add(usize::try_from(w.count_ones()).unwrap_or(usize::MAX))
        })
    }

    /// Haben beide Mengen mindestens ein gemeinsames Element?
    ///
    /// Mengen unterschiedlicher Länge werden mit Nullen aufgefüllt gedacht.
    pub fn intersects(&self, other: &BitSet<'_>) -> bool {
        self.words.iter().zip(other.words).any(|(a, b)| a & b != 0)
    }

    /// Ist jedes Element dieser Menge auch in `other` enthalten?
    pub fn is_subset_of(&self, other: &BitSet<'_>) -> bool {
        self.words.iter().enumerate().all(|(i, a)| {
            let b = other.words.get(i).copied().unwrap_or(0);
            a & !b == 0
        })
    }

    /// Iterator über die Indizes aller gesetzten Bits in aufsteigender Reihenfolge.
    pub fn iter(&self) -> Ones<'a> {
        Ones {
            words: self.words,
            word_index: 0,
            current: self.words.first().copied().unwrap_or(0),
        }
    }
}

impl fmt::Debug for BitSet<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<'a> IntoIterator for &BitSet<'a> {
    type Item = usize;
    type IntoIter = Ones<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for BitSet<'a> {
    type Item = usize;
    type IntoIter = Ones<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator über gesetzte Bits, siehe [`BitSet::iter`].
#[derive(Clone, Debug)]
pub struct Ones<'a> {
    words: &'a [u64],
    word_index: usize,
    current: u64,
}

impl Iterator for Ones<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        loop {
            if self.current != 0 {
                let bit = usize::try_from(self.current.trailing_zeros()).ok()?;
                // Niedrigstes gesetztes Bit löschen.
                self.current &= self.current.wrapping_sub(1);
                return self.word_index.checked_mul(WORD_BITS)?.checked_add(bit);
            }
            self.word_index = self.word_index.checked_add(1)?;
            self.current = *self.words.get(self.word_index)?;
        }
    }
}

/// Veränderliche Sicht auf eine Bitmenge.
#[derive(PartialEq, Eq, Hash)]
pub struct BitSetMut<'a> {
    words: &'a mut [u64],
}

impl<'a> BitSetMut<'a> {
    /// Veränderliche Sicht auf die gegebenen Wörter.
    pub fn new(words: &'a mut [u64]) -> Self {
        Self { words }
    }

    /// Unveränderliche Sicht auf dieselbe Menge.
    pub fn as_set(&self) -> BitSet<'_> {
        BitSet::new(self.words)
    }

    /// Kapazität in Bits.
    pub fn capacity(&self) -> usize {
        self.as_set().capacity()
    }

    /// Setzt das Bit `index`.
    ///
    /// # Errors
    /// [`OutOfRange`], wenn `index` außerhalb der Kapazität liegt; die Menge bleibt unverändert.
    pub fn insert(&mut self, index: usize) -> Result<(), OutOfRange> {
        let capacity = self.capacity();
        let (word, mask) = locate(index).ok_or(OutOfRange { index, capacity })?;
        let w = self
            .words
            .get_mut(word)
            .ok_or(OutOfRange { index, capacity })?;
        *w |= mask;
        Ok(())
    }

    /// Löscht das Bit `index`.
    ///
    /// # Errors
    /// [`OutOfRange`], wenn `index` außerhalb der Kapazität liegt; die Menge bleibt unverändert.
    pub fn remove(&mut self, index: usize) -> Result<(), OutOfRange> {
        let capacity = self.capacity();
        let (word, mask) = locate(index).ok_or(OutOfRange { index, capacity })?;
        let w = self
            .words
            .get_mut(word)
            .ok_or(OutOfRange { index, capacity })?;
        *w &= !mask;
        Ok(())
    }

    /// Löscht alle Bits.
    pub fn clear(&mut self) {
        self.words.iter_mut().for_each(|w| *w = 0);
    }

    /// Fügt alle Elemente von `other` hinzu.
    ///
    /// # Errors
    /// [`OutOfRange`], wenn `other` ein Bit außerhalb der Kapazität enthält; die Menge bleibt
    /// dann unverändert.
    pub fn union_with(&mut self, other: &BitSet<'_>) -> Result<(), OutOfRange> {
        let len = self.words.len();
        if let Some(index) = other.words.get(len..).and_then(|rest| {
            BitSet::new(rest)
                .iter()
                .next()
                .map(|i| i.saturating_add(self.capacity()))
        }) {
            return Err(OutOfRange {
                index,
                capacity: self.capacity(),
            });
        }
        self.words
            .iter_mut()
            .zip(other.words)
            .for_each(|(a, b)| *a |= b);
        Ok(())
    }

    /// Entfernt alle Elemente von `other`.
    pub fn difference_with(&mut self, other: &BitSet<'_>) {
        self.words
            .iter_mut()
            .zip(other.words)
            .for_each(|(a, b)| *a &= !b);
    }
}

impl fmt::Debug for BitSetMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_set().fmt(f)
    }
}
