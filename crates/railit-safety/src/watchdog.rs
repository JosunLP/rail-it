//! Watchdog für die Zyklusfrist (REQ-SAF-005).

use crate::Trip;

/// Überwacht die Rechenzeit eines Zyklus und den Abstand zwischen zwei Zyklen.
///
/// Der Watchdog hat keine eigene Uhr: Der Aufrufer meldet Beginn und Ende jedes Zyklus in
/// Mikrosekunden einer monotonen Uhr. Überschreitet die Rechenzeit die Frist oder der Abstand
/// zum vorigen Zyklusbeginn die Grenze, meldet er einen [`Trip`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Watchdog {
    deadline_us: u64,
    gap_limit_us: u64,
    last_start: Option<u64>,
}

impl Watchdog {
    /// Frist für die Rechenzeit eines Zyklus und höchster Abstand zweier Zyklusbeginne.
    pub const fn new(deadline_us: u64, gap_limit_us: u64) -> Self {
        Self {
            deadline_us,
            gap_limit_us,
            last_start: None,
        }
    }

    /// Frist für die Rechenzeit.
    pub const fn deadline_us(&self) -> u64 {
        self.deadline_us
    }

    /// Höchster Abstand zweier Zyklusbeginne.
    pub const fn gap_limit_us(&self) -> u64 {
        self.gap_limit_us
    }

    /// Prüft einen Zyklus, der bei `start_us` begann und bei `end_us` endete.
    ///
    /// # Errors
    /// [`Trip::DeadlineExceeded`] bzw. [`Trip::CycleGap`]. Eine rückwärts laufende Uhr gilt
    /// als Fristüberschreitung.
    pub fn check(&mut self, start_us: u64, end_us: u64) -> Result<(), Trip> {
        let previous = self.last_start.replace(start_us);
        if let Some(prev) = previous {
            let gap = start_us.checked_sub(prev).ok_or(Trip::DeadlineExceeded {
                elapsed_us: u64::MAX,
                deadline_us: self.deadline_us,
            })?;
            if gap > self.gap_limit_us {
                return Err(Trip::CycleGap {
                    gap_us: gap,
                    limit_us: self.gap_limit_us,
                });
            }
        }
        // Eine rückwärts laufende Uhr ist immer ein Fehler, auch bei unbegrenzter Frist
        // (gefunden mit Kani).
        let Some(elapsed) = end_us.checked_sub(start_us) else {
            return Err(Trip::DeadlineExceeded {
                elapsed_us: u64::MAX,
                deadline_us: self.deadline_us,
            });
        };
        if elapsed > self.deadline_us {
            return Err(Trip::DeadlineExceeded {
                elapsed_us: elapsed,
                deadline_us: self.deadline_us,
            });
        }
        Ok(())
    }

    /// Vergisst den letzten Zyklusbeginn (nach Rücksetzen oder Pause).
    pub fn restart(&mut self) {
        self.last_start = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_and_gap() {
        let mut w = Watchdog::new(1_000, 150_000);
        assert_eq!(w.check(0, 900), Ok(()));
        assert_eq!(w.check(100_000, 100_500), Ok(()));
        assert_eq!(
            w.check(200_000, 201_001),
            Err(Trip::DeadlineExceeded {
                elapsed_us: 1_001,
                deadline_us: 1_000
            })
        );
        assert_eq!(
            w.check(400_000, 400_001),
            Err(Trip::CycleGap {
                gap_us: 200_000,
                limit_us: 150_000
            })
        );
        assert!(w.check(300_000, 300_001).is_err(), "Uhr läuft rückwärts");
        w.restart();
        assert_eq!(w.check(10, 20), Ok(()));
    }

    #[test]
    fn backward_clock_trips_even_without_deadline() {
        let mut w = Watchdog::new(u64::MAX, u64::MAX);
        assert!(w.check(20, 10).is_err());
    }
}
