//! Zyklische Speicherprüfung (REQ-SAF-008).
//!
//! Ein Bitfehler im Speicher darf nicht unbemerkt bleiben, auch nicht in Daten, die der Kern
//! nur liest. Der Kernzustand ist durch die Kanäle abgedeckt (jeder Kanal hat einen eigenen
//! Speicher, der Voter vergleicht). Zwei Bereiche prüft diese Schicht zusätzlich, in kleinen
//! Teilen je Zyklus, damit die Rechenzeit begrenzt bleibt:
//!
//! - **Plan:** SHA-256 über das Plan-Image im Speicher, verglichen mit der Prüfsumme, die beim
//!   Laden festgehalten wurde. Ein Durchlauf dauert `ceil(Länge / plan_chunk)` Zyklen.
//! - **RAM:** ein Marsch-Test über einen eigenen Prüfbereich (Schreiben und Lesen von
//!   Mustern und deren Komplement je Wort), der Haftfehler und Kopplungen in diesem Bereich
//!   und im Speicherpfad aufdeckt.
//!
//! Jede Abweichung ist ein Grund für den sicheren Zustand ([`crate::Trip::MemoryFault`]).

use sha2::{Digest as _, Sha256};

/// Geprüfter Speicherbereich.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum MemoryRegion {
    /// Plan-Image.
    Plan,
    /// RAM-Prüfbereich.
    Ram,
}

/// Fortschritt der Speicherprüfung.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryStats {
    /// Vollständige Durchläufe über den Plan.
    pub plan_passes: u64,
    /// Vollständige Durchläufe über den RAM-Prüfbereich.
    pub ram_passes: u64,
}

/// Muster des Marsch-Tests je Wort.
const PATTERNS: [u32; 4] = [0x5555_5555, 0xAAAA_AAAA, 0x0F0F_0F0F, 0xF0F0_F0F0];

/// Zyklische Prüfung von Plan und RAM-Prüfbereich.
#[derive(Clone, Debug)]
pub struct MemoryCheck {
    expected: [u8; 32],
    body_len: usize,
    plan_chunk: usize,
    ram_chunk: usize,
    hasher: Sha256,
    plan_offset: usize,
    ram_offset: usize,
    stats: MemoryStats,
}

impl MemoryCheck {
    /// Prüfung für einen Plan, dessen erste `body_len` Bytes beim Laden die Prüfsumme
    /// `expected` hatten. Je Zyklus werden `plan_chunk` Bytes des Plans und `ram_chunk`
    /// Wörter des RAM-Prüfbereichs geprüft (mindestens je eins).
    #[must_use]
    pub fn new(expected: [u8; 32], body_len: usize, plan_chunk: usize, ram_chunk: usize) -> Self {
        Self {
            expected,
            body_len,
            plan_chunk: plan_chunk.max(1),
            ram_chunk: ram_chunk.max(1),
            hasher: Sha256::new(),
            plan_offset: 0,
            ram_offset: 0,
            stats: MemoryStats::default(),
        }
    }

    /// Fortschritt.
    #[must_use]
    pub const fn stats(&self) -> MemoryStats {
        self.stats
    }

    /// Ein Prüfschritt: der nächste Teil des Plans `image` und des Prüfbereichs `ram`.
    ///
    /// # Errors
    /// Der Bereich, in dem ein Fehler gefunden wurde.
    pub fn step(&mut self, image: &[u8], ram: &mut [u32]) -> Result<(), MemoryRegion> {
        self.step_plan(image)?;
        self.step_ram(ram)
    }

    fn step_plan(&mut self, image: &[u8]) -> Result<(), MemoryRegion> {
        let body = image.get(..self.body_len).ok_or(MemoryRegion::Plan)?;
        let end = self
            .plan_offset
            .saturating_add(self.plan_chunk)
            .min(body.len());
        let part = body.get(self.plan_offset..end).ok_or(MemoryRegion::Plan)?;
        self.hasher.update(part);
        self.plan_offset = end;
        if end >= body.len() {
            let digest = core::mem::take(&mut self.hasher).finalize();
            self.plan_offset = 0;
            if digest.as_slice() != self.expected {
                return Err(MemoryRegion::Plan);
            }
            self.stats.plan_passes = self.stats.plan_passes.saturating_add(1);
        }
        Ok(())
    }

    fn step_ram(&mut self, ram: &mut [u32]) -> Result<(), MemoryRegion> {
        if ram.is_empty() {
            return Ok(());
        }
        let end = self
            .ram_offset
            .saturating_add(self.ram_chunk)
            .min(ram.len());
        let words = ram.get_mut(self.ram_offset..end).ok_or(MemoryRegion::Ram)?;
        // Aufsteigend Muster schreiben, dann absteigend lesen und invertieren (Marsch-Schritte).
        for pattern in PATTERNS {
            for w in words.iter_mut() {
                *w = pattern;
            }
            for w in words.iter_mut().rev() {
                if core::hint::black_box(*w) != pattern {
                    return Err(MemoryRegion::Ram);
                }
                *w = !pattern;
            }
            for w in words.iter() {
                if core::hint::black_box(*w) != !pattern {
                    return Err(MemoryRegion::Ram);
                }
            }
        }
        for w in words.iter_mut() {
            *w = 0;
        }
        self.ram_offset = end;
        if end >= ram.len() {
            self.ram_offset = 0;
            self.stats.ram_passes = self.stats.ram_passes.saturating_add(1);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    #[test]
    fn intact_memory_passes_and_counts_passes() {
        let image = [7u8; 1000];
        let mut ram = [0u32; 64];
        let mut check = MemoryCheck::new(digest(&image[..900]), 900, 256, 16);
        for _ in 0..8 {
            assert_eq!(check.step(&image, &mut ram), Ok(()));
        }
        assert_eq!(
            check.stats(),
            MemoryStats {
                plan_passes: 2,
                ram_passes: 2
            }
        );
    }

    #[test]
    fn a_flipped_plan_bit_is_found_within_one_pass() {
        let mut image = [7u8; 1000];
        let mut ram = [0u32; 4];
        let mut check = MemoryCheck::new(digest(&image[..900]), 900, 256, 4);
        assert_eq!(check.step(&image, &mut ram), Ok(()));
        image[10] ^= 0x04; // schon geprüfter Teil: fällt im nächsten Durchlauf auf
        let mut found = None;
        for cycle in 0..8 {
            if let Err(region) = check.step(&image, &mut ram) {
                found = Some((cycle, region));
                break;
            }
        }
        assert_eq!(found, Some((6, MemoryRegion::Plan)));
    }

    #[test]
    fn a_short_image_is_a_fault() {
        let mut check = MemoryCheck::new([0; 32], 100, 10, 1);
        assert_eq!(check.step(&[0; 50], &mut []), Err(MemoryRegion::Plan));
    }
}
