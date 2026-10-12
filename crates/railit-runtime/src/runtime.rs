//! Zyklische Kopplung von Kern und Feld.

use std::io::Write;

use railit_journal::{Header, JournalWriter};
use railit_kernel::{
    Command, Event, Input, Interlocking, OwnedState, Request, RequestId, SafeStateReason,
    VecOutputs,
};
use railit_plan::PlanView;
use railit_rules::GenericRules;
use railit_types::{Duration, Tick};

use crate::field::FieldAdapter;
use crate::names::{NamedEvent, Snapshot};

/// Ergebnis eines Zyklus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CycleOutput {
    /// Zeitpunkt des Zyklus.
    pub now: Tick,
    /// Ereignisse des Kerns.
    pub events: Vec<Event>,
    /// Stellbefehle des Kerns.
    pub commands: Vec<Command>,
    /// Grund, falls der Kern im sicheren Zustand ist.
    pub safe_state: Option<SafeStateReason>,
}

/// Runtime: ein Kern, ein Feld, ein fester Zyklus.
pub struct Runtime<F> {
    kernel: Interlocking<'static, GenericRules>,
    state: OwnedState,
    field: F,
    now: Tick,
    cycle: Duration,
    out: VecOutputs,
    inputs: Vec<Input>,
    journal: Option<JournalWriter<Box<dyn Write + Send>>>,
    journal_error: Option<String>,
}

impl<F: FieldAdapter> Runtime<F> {
    /// Neue Runtime im Anfangszustand.
    pub fn new(plan: PlanView<'static>, rules: GenericRules, field: F, cycle: Duration) -> Self {
        Self {
            kernel: Interlocking::new(plan, rules),
            state: OwnedState::new(&plan),
            field,
            now: Tick::ZERO,
            cycle,
            out: VecOutputs::default(),
            inputs: Vec::new(),
            journal: None,
            journal_error: None,
        }
    }

    /// Beginnt ein Journal (REQ-JRN-001): Ab dem nächsten Zyklus werden Eingaben,
    /// Ausgaben-Hash und alle `checkpoint_every` Zyklen der Zustands-Hash geschrieben.
    ///
    /// Das Journal muss vor dem ersten Zyklus beginnen, damit es sich abspielen lässt.
    ///
    /// # Errors
    /// Schreibfehler beim Kopf.
    pub fn start_journal(
        &mut self,
        out: Box<dyn Write + Send>,
        checkpoint_every: u64,
    ) -> std::io::Result<()> {
        let header = Header::new(
            self.kernel.plan(),
            self.kernel.rules(),
            self.cycle,
            checkpoint_every,
        );
        let mut journal = JournalWriter::new(out, header)?;
        journal.flush()?;
        self.journal = Some(journal);
        self.journal_error = None;
        Ok(())
    }

    /// Fehler, an dem das Journal abgebrochen ist. Der Betrieb läuft weiter; das Journal
    /// ist ab diesem Zyklus unvollständig.
    pub fn journal_error(&self) -> Option<&str> {
        self.journal_error.as_deref()
    }

    /// Hash der letzten Journalzeile, falls ein Journal läuft.
    pub fn journal_head(&self) -> Option<String> {
        self.journal.as_ref().map(JournalWriter::head)
    }

    /// Führt einen Zyklus aus: Feld fortschreiben, Meldungen und Anforderungen an den Kern,
    /// Stellbefehle ans Feld.
    pub fn cycle(&mut self, requests: &[(RequestId, Request)]) -> CycleOutput {
        self.now = self.now.checked_add(self.cycle).unwrap_or(self.now);
        self.field.advance(self.now);

        self.inputs.clear();
        self.field.reports(&mut self.inputs);
        self.inputs
            .extend(requests.iter().map(|(id, request)| Input::Request {
                id: *id,
                request: *request,
            }));

        self.out.clear();
        let result = self.kernel.step(
            &mut self.state.view(),
            self.now,
            &self.inputs,
            &mut self.out,
        );
        self.field.apply(&self.out.commands);

        if let Some(journal) = &mut self.journal {
            let written = journal
                .record(self.now, &self.inputs, &self.out, &self.state)
                .and_then(|()| journal.flush());
            if let Err(e) = written {
                self.journal_error = Some(e.to_string());
                self.journal = None;
            }
        }

        CycleOutput {
            now: self.now,
            events: self.out.events.clone(),
            commands: self.out.commands.clone(),
            safe_state: result.err(),
        }
    }

    /// Der geladene Plan.
    pub fn plan(&self) -> &PlanView<'static> {
        self.kernel.plan()
    }

    /// Zustand des Kerns.
    pub fn state(&self) -> &OwnedState {
        &self.state
    }

    /// Die Feldebene.
    pub fn field(&self) -> &F {
        &self.field
    }

    /// Die Feldebene (veränderlich, z. B. für Simulationsbefehle).
    pub fn field_mut(&mut self) -> &mut F {
        &mut self.field
    }

    /// Zeit des letzten Zyklus.
    pub fn now(&self) -> Tick {
        self.now
    }

    /// Zykluszeit.
    pub fn cycle_time(&self) -> Duration {
        self.cycle
    }

    /// Zustandsbild mit fachlichen Namen.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot::new(self.kernel.plan(), &self.state, self.now)
    }

    /// Ereignis mit fachlichen Namen.
    pub fn name_event(&self, event: &Event) -> NamedEvent {
        NamedEvent::new(self.kernel.plan(), event)
    }
}
