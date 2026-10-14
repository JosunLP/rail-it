//! Ausgabe erst nach Quittung (REQ-SVC-012).
//!
//! Liest ein Bereitschaftsrechner das Journal mit (REQ-SVC-011), soll er bei einer Übernahme
//! jede Wirkung kennen, die das Stellwerk draußen erzielt hat. Deshalb hält die Runtime die
//! Ausgaben eines Zyklus – Stellbefehle ans Feld, Grenzmeldungen an die Nachbarbereiche,
//! Fahrerlaubnisse an ETCS-Fahrzeuge – zurück, bis der Bereitschaftsrechner die Journalzeilen
//! dieses Zyklus in allen Bereichen geprüft und quittiert hat. Eine Quittung nennt Zyklus und
//! Hash der Journalzeile; sie gilt nur, wenn beides zu dem passt, was die Runtime geschrieben
//! hat.
//!
//! Bleiben Quittungen länger als die Frist aus, gilt der Bereitschaftsrechner als verloren:
//! Die Runtime gibt alles frei und arbeitet ohne Sperre weiter (Verfügbarkeit). Sie sperrt
//! wieder, sobald der Bereitschaftsrechner aufgeholt hat. „Gedeckt“ ist der
//! Bereitschaftsrechner nur, solange die Sperre besteht und er alles Freigegebene quittiert
//! hat; nur dann sichert die Runtime zu, die Sperre noch mindestens [`Coverage::hold_ms`]
//! aufrechtzuerhalten. Diese Zusage ist die Grundlage der Übernahme mit dem mitgelesenen
//! Zustand.

use std::collections::VecDeque;

use railit_kernel::{BorderMessage, Command};
use railit_types::{BorderId, SignalId};
use serde::Serialize;

/// Anzahl der zuletzt geschriebenen Zyklen je Bereich, gegen die Quittungen geprüft werden.
const MARKS: usize = 4096;

/// Uhr für die Frist.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ReplicationClock {
    /// Monotone Wanduhr (Betrieb: Frist und Zusage gelten in echter Zeit).
    #[default]
    Wall,
    /// Zeit der Runtime (deterministisch, für Tests und Simulation).
    Cycle,
}

/// Einstellungen der Ausgabesperre.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ReplicationConfig {
    /// Längste Wartezeit einer Ausgabe auf ihre Quittung in Millisekunden.
    pub grace_ms: u64,
    /// Höchster Rückstand des Bereitschaftsrechners in Zyklen, bei dem die Sperre nach einem
    /// Verlust wieder greift.
    pub rearm_lag: u64,
    /// Uhr der Frist.
    pub clock: ReplicationClock,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            grace_ms: 2_000,
            rearm_lag: 5,
            clock: ReplicationClock::Wall,
        }
    }
}

/// Zusage an den Bereitschaftsrechner (Antwort auf eine Quittung).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    /// Die Sperre besteht und alles Freigegebene ist quittiert.
    pub covered: bool,
    /// So lange (ab jetzt) gibt die Runtime mindestens nichts ohne Quittung frei; 0, wenn
    /// nicht gedeckt.
    pub hold_ms: u64,
}

/// Zustand der Ausgabesperre.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReplicationStatus {
    /// Ausgaben warten auf Quittungen.
    pub gated: bool,
    /// Zusage an den Bereitschaftsrechner.
    pub coverage: Coverage,
    /// Zurückgehaltene Zyklen.
    pub held: usize,
    /// Alter der ältesten zurückgehaltenen Ausgabe in Millisekunden.
    pub oldest_ms: u64,
    /// Zuletzt quittierter Zyklus je Bereich.
    pub acked: Vec<Option<u64>>,
    /// Zuletzt geschriebener Zyklus je Bereich.
    pub written: Vec<Option<u64>>,
    /// Gültige Quittungen.
    pub acks: u64,
    /// Quittungen, die zu keinem geschriebenen Stand passten.
    pub rejected_acks: u64,
    /// Wie oft die Sperre wegen ausbleibender Quittungen aufgehoben wurde.
    pub lapses: u64,
}

/// Stand eines Bereichs nach einem Zyklus: Zyklus des Kerns und Hash der letzten Journalzeile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mark {
    pub(crate) cycle: u64,
    pub(crate) head: String,
}

/// Ausgaben eines Zyklus.
#[derive(Debug, Default)]
pub(crate) struct Batch {
    /// Je Bereich der Stand nach dem Zyklus (`None`: ohne Journal, nie quittierbar).
    pub(crate) marks: Vec<Option<Mark>>,
    /// Stellbefehle (global).
    pub(crate) commands: Vec<Command>,
    /// Grenzmeldungen an (Bereich, Grenze).
    pub(crate) mail: Vec<((usize, BorderId), BorderMessage)>,
    /// Fahrerlaubnisse an ETCS-Fahrzeuge (global).
    pub(crate) cab: Vec<(String, Option<Vec<SignalId>>)>,
}

#[derive(Debug)]
struct Held {
    batch: Batch,
    created_ms: u64,
}

/// Ausgabesperre über alle Bereiche einer Runtime.
#[derive(Debug)]
pub(crate) struct Gate {
    cfg: ReplicationConfig,
    gated: bool,
    held: VecDeque<Held>,
    /// Zuletzt geschriebene Stände je Bereich (für die Prüfung der Quittungen).
    marks: Vec<VecDeque<Mark>>,
    acked: Vec<Option<u64>>,
    /// Höchster Zyklus je Bereich, dessen Ausgaben freigegeben sind.
    released: Vec<Option<u64>>,
    acks: u64,
    rejected_acks: u64,
    lapses: u64,
}

impl Gate {
    /// Sperre für `areas` Bereiche; sie greift ab dem ersten Zyklus.
    pub(crate) fn new(cfg: ReplicationConfig, areas: usize) -> Self {
        Self {
            cfg,
            gated: true,
            held: VecDeque::new(),
            marks: vec![VecDeque::new(); areas],
            acked: vec![None; areas],
            released: vec![None; areas],
            acks: 0,
            rejected_acks: 0,
            lapses: 0,
        }
    }

    pub(crate) const fn clock(&self) -> ReplicationClock {
        self.cfg.clock
    }

    /// Nimmt die Ausgaben eines Zyklus auf und gibt zurück, was jetzt hinaus darf.
    pub(crate) fn push(&mut self, batch: Batch, now_ms: u64) -> Vec<Batch> {
        for (marks, mark) in self.marks.iter_mut().zip(&batch.marks) {
            if let Some(mark) = mark {
                if marks.len() >= MARKS {
                    marks.pop_front();
                }
                marks.push_back(mark.clone());
            }
        }
        self.held.push_back(Held {
            batch,
            created_ms: now_ms,
        });
        self.release(now_ms)
    }

    /// Prüft eine Quittung für `area`: Sie gilt, wenn Zyklus und Hash zu einem geschriebenen
    /// Stand passen. Liefert, ob sie galt.
    pub(crate) fn acknowledge(&mut self, area: usize, cycle: u64, head: &str) -> bool {
        let valid = self
            .marks
            .get(area)
            .is_some_and(|m| m.iter().any(|k| k.cycle == cycle && k.head == head));
        if !valid {
            self.rejected_acks = self.rejected_acks.saturating_add(1);
            return false;
        }
        self.acks = self.acks.saturating_add(1);
        if let Some(a) = self.acked.get_mut(area) {
            *a = Some(a.map_or(cycle, |old| old.max(cycle)));
        }
        if !self.gated && self.caught_up() {
            self.gated = true;
        }
        true
    }

    /// Hat der Bereitschaftsrechner in allen Bereichen bis auf den erlaubten Rückstand
    /// aufgeholt?
    fn caught_up(&self) -> bool {
        self.marks
            .iter()
            .zip(&self.acked)
            .all(|(marks, acked)| match (marks.back(), acked) {
                (None, _) => true,
                (Some(latest), Some(acked)) => {
                    latest.cycle.saturating_sub(*acked) <= self.cfg.rearm_lag
                }
                (Some(_), None) => false,
            })
    }

    /// Ist der Zyklus `held` in allen Bereichen quittiert?
    fn is_acked(&self, batch: &Batch) -> bool {
        batch
            .marks
            .iter()
            .zip(&self.acked)
            .all(|(mark, acked)| match (mark, acked) {
                (Some(m), Some(a)) => m.cycle <= *a,
                _ => false,
            })
    }

    /// Gibt quittierte Zyklen in Reihenfolge frei; nach Ablauf der Frist alles.
    pub(crate) fn release(&mut self, now_ms: u64) -> Vec<Batch> {
        let lapsed = self.gated
            && self
                .held
                .front()
                .is_some_and(|h| now_ms.saturating_sub(h.created_ms) > self.cfg.grace_ms);
        if lapsed {
            self.gated = false;
            self.lapses = self.lapses.saturating_add(1);
        }
        let mut out = Vec::new();
        while let Some(front) = self.held.front() {
            if self.gated && !self.is_acked(&front.batch) {
                break;
            }
            let Some(h) = self.held.pop_front() else {
                break;
            };
            for (released, mark) in self.released.iter_mut().zip(&h.batch.marks) {
                if let Some(m) = mark {
                    *released = Some(m.cycle);
                }
            }
            out.push(h.batch);
        }
        out
    }

    /// Zusage an den Bereitschaftsrechner.
    pub(crate) fn coverage(&self, now_ms: u64) -> Coverage {
        let all_acked = self
            .released
            .iter()
            .zip(&self.acked)
            .all(|(released, acked)| match (released, acked) {
                (None, _) => true,
                (Some(r), Some(a)) => r <= a,
                (Some(_), None) => false,
            });
        if !self.gated || !all_acked {
            return Coverage::default();
        }
        let hold_ms = self.held.front().map_or(self.cfg.grace_ms, |h| {
            self.cfg
                .grace_ms
                .saturating_sub(now_ms.saturating_sub(h.created_ms))
        });
        Coverage {
            covered: true,
            hold_ms,
        }
    }

    pub(crate) fn status(&self, now_ms: u64) -> ReplicationStatus {
        ReplicationStatus {
            gated: self.gated,
            coverage: self.coverage(now_ms),
            held: self.held.len(),
            oldest_ms: self
                .held
                .front()
                .map_or(0, |h| now_ms.saturating_sub(h.created_ms)),
            acked: self.acked.clone(),
            written: self
                .marks
                .iter()
                .map(|m| m.back().map(|k| k.cycle))
                .collect(),
            acks: self.acks,
            rejected_acks: self.rejected_acks,
            lapses: self.lapses,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ReplicationConfig {
        ReplicationConfig {
            grace_ms: 1_000,
            rearm_lag: 2,
            clock: ReplicationClock::Cycle,
        }
    }

    fn batch(cycles: &[u64]) -> Batch {
        Batch {
            marks: cycles
                .iter()
                .map(|c| {
                    Some(Mark {
                        cycle: *c,
                        head: format!("h{c}"),
                    })
                })
                .collect(),
            ..Batch::default()
        }
    }

    #[test]
    fn outputs_wait_for_all_areas() {
        let mut g = Gate::new(cfg(), 2);
        assert!(g.push(batch(&[1, 1]), 0).is_empty());
        assert!(g.push(batch(&[2, 2]), 100).is_empty());
        assert!(g.acknowledge(0, 2, "h2"));
        assert!(g.release(150).is_empty(), "Bereich 1 fehlt");
        // Falscher Hash: gilt nicht.
        assert!(!g.acknowledge(1, 1, "h2"));
        assert!(g.acknowledge(1, 1, "h1"));
        assert_eq!(g.release(160).len(), 1);
        assert!(g.coverage(160).covered);
        assert_eq!(g.coverage(160).hold_ms, 1_000 - 60);
        assert!(g.acknowledge(1, 2, "h2"));
        assert_eq!(g.release(170).len(), 1);
        assert_eq!(g.coverage(170).hold_ms, 1_000);
        assert_eq!(g.status(170).rejected_acks, 1);
    }

    #[test]
    fn lapse_releases_everything_and_rearms_after_catching_up() {
        let mut g = Gate::new(cfg(), 1);
        assert!(g.push(batch(&[1]), 0).is_empty());
        assert!(g.push(batch(&[2]), 500).is_empty());
        // Frist abgelaufen: alles frei, ohne Sperre weiter.
        assert_eq!(g.push(batch(&[3]), 1_001).len(), 3);
        assert!(!g.status(1_001).gated);
        assert!(!g.coverage(1_001).covered);
        assert_eq!(g.push(batch(&[4]), 1_100).len(), 1);
        assert_eq!(g.status(1_100).lapses, 1);
        // Zu weit zurück: keine neue Sperre.
        assert!(g.acknowledge(0, 1, "h1"));
        assert!(!g.status(1_100).gated);
        // Aufgeholt (Rückstand 2): Sperre greift, gedeckt erst, wenn alles Freigegebene
        // quittiert ist.
        assert!(g.acknowledge(0, 2, "h2"));
        assert!(g.status(1_100).gated);
        assert!(!g.coverage(1_100).covered);
        assert!(g.push(batch(&[5]), 1_200).is_empty());
        assert!(g.acknowledge(0, 4, "h4"));
        assert!(g.coverage(1_200).covered);
        assert!(g.acknowledge(0, 5, "h5"));
        assert_eq!(g.release(1_300).len(), 1);
    }

    #[test]
    fn area_without_journal_lapses() {
        let mut g = Gate::new(cfg(), 2);
        let mut b = batch(&[1, 1]);
        b.marks[1] = None;
        assert!(g.push(b, 0).is_empty());
        assert!(g.acknowledge(0, 1, "h1"));
        assert!(g.release(500).is_empty());
        assert_eq!(g.release(1_001).len(), 1);
        assert!(!g.coverage(1_001).covered);
    }
}
