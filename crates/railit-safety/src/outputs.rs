//! Ausgabepuffer der Kanäle.

use railit_kernel::{Command, Event, Outputs};
use sha2::Sha256;

use crate::digest::feed;

/// Ausgabepuffer eines Kanals: nimmt die Ausgaben eines Zyklus auf und gibt sie nach der
/// Abstimmung weiter.
pub trait ChannelOutputs: Outputs {
    /// Leert den Puffer vor einem Zyklus.
    fn clear(&mut self);
    /// Der Puffer war zu klein; Ausgaben sind verloren gegangen.
    fn overflowed(&self) -> bool;
    /// Gibt alle Ausgaben in der aufgenommenen Reihenfolge an `out` weiter.
    fn replay<O: Outputs + ?Sized>(&self, out: &mut O);
}

/// Ausgabepuffer fester Größe ohne Allokation: bis zu `C` Stellbefehle und `E` Ereignisse je
/// Zyklus. Mehr Ausgaben setzen [`ChannelOutputs::overflowed`]; der Voter geht dann in den
/// sicheren Zustand.
#[derive(Clone, Debug)]
pub struct FixedOutputs<const C: usize, const E: usize> {
    commands: [Option<Command>; C],
    events: [Option<Event>; E],
    command_count: usize,
    event_count: usize,
    overflow: bool,
}

impl<const C: usize, const E: usize> FixedOutputs<C, E> {
    /// Leerer Puffer.
    pub const fn new() -> Self {
        Self {
            commands: [None; C],
            events: [None; E],
            command_count: 0,
            event_count: 0,
            overflow: false,
        }
    }

    /// Stellbefehle des Zyklus.
    pub fn commands(&self) -> impl Iterator<Item = Command> + '_ {
        self.commands
            .iter()
            .take(self.command_count)
            .filter_map(|c| *c)
    }

    /// Ereignisse des Zyklus.
    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.iter().take(self.event_count).filter_map(|e| *e)
    }
}

impl<const C: usize, const E: usize> Default for FixedOutputs<C, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const C: usize, const E: usize> Outputs for FixedOutputs<C, E> {
    fn command(&mut self, command: Command) {
        if let Some(slot) = self.commands.get_mut(self.command_count) {
            *slot = Some(command);
            self.command_count = self.command_count.saturating_add(1);
        } else {
            self.overflow = true;
        }
    }

    fn event(&mut self, event: Event) {
        if let Some(slot) = self.events.get_mut(self.event_count) {
            *slot = Some(event);
            self.event_count = self.event_count.saturating_add(1);
        } else {
            self.overflow = true;
        }
    }
}

impl<const C: usize, const E: usize> ChannelOutputs for FixedOutputs<C, E> {
    fn clear(&mut self) {
        self.command_count = 0;
        self.event_count = 0;
        self.overflow = false;
    }

    fn overflowed(&self) -> bool {
        self.overflow
    }

    fn replay<O: Outputs + ?Sized>(&self, out: &mut O) {
        for c in self.commands() {
            out.command(c);
        }
        for e in self.events() {
            out.event(e);
        }
    }
}

#[cfg(feature = "alloc")]
impl ChannelOutputs for railit_kernel::VecOutputs {
    fn clear(&mut self) {
        railit_kernel::VecOutputs::clear(self);
    }

    fn overflowed(&self) -> bool {
        false
    }

    fn replay<O: Outputs + ?Sized>(&self, out: &mut O) {
        for c in &self.commands {
            out.command(*c);
        }
        for e in &self.events {
            out.event(*e);
        }
    }
}

/// Leitet Ausgaben an den Puffer eines Kanals weiter und hasht sie dabei.
pub(crate) struct Tap<'a, O> {
    pub(crate) inner: &'a mut O,
    pub(crate) sha: Sha256,
}

impl<O: Outputs> Outputs for Tap<'_, O> {
    fn command(&mut self, command: Command) {
        feed(&mut self.sha, &(0_u8, command));
        self.inner.command(command);
    }

    fn event(&mut self, event: Event) {
        feed(&mut self.sha, &(1_u8, event));
        self.inner.event(event);
    }
}
