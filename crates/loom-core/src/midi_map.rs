//! MIDI learn: control changes that set parameters.
//!
//! Spec: `docs/design/midi-learn.md`.
//!
//! The engine applies mappings (rule 1) and reports back through one table of
//! atomics, allocated once with the engine (rules 2 and 3). There is one
//! entry per MIDI control, 16 channels times 128 controller numbers, because
//! a control has at most one mapping. Each entry holds the id of the mapping
//! that wrote it, a sequence number, and the 7-bit value. The controller
//! applies an entry only when the id matches and the sequence number moved,
//! so a write from a mapping that no longer exists is never applied, and the
//! same value sent twice is still seen twice.
//!
//! Screen edits and control changes can race: the controller changes the
//! graph at once, but the engine applies the edit at its next buffer. So a
//! screen edit on a mapped parameter carries an edit token, and the engine
//! stores that token with its own sequence number in a second table when it
//! applies the edit. Control changes and edits share one sequence, so the
//! controller can tell which came last in the engine (MIDI learn, rule 2).

use crate::descriptor::ParamSpec;
use crate::graph::ModuleId;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Number of MIDI channels.
const CHANNELS: usize = 16;
/// Number of controller numbers on a channel.
const CONTROLLERS: usize = 128;

/// One hardware control: a MIDI channel and a controller number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MidiControl {
    /// Channel, 0 to 15. Shown to users as 1 to 16.
    pub channel: u8,
    /// Controller number, 0 to 127.
    pub controller: u8,
}

impl MidiControl {
    /// True when the channel and controller number are in range.
    pub fn is_valid(self) -> bool {
        usize::from(self.channel) < CHANNELS && usize::from(self.controller) < CONTROLLERS
    }

    /// Index into the feedback table. Only for valid controls.
    fn index(self) -> usize {
        usize::from(self.channel) * CONTROLLERS + usize::from(self.controller)
    }
}

impl fmt::Display for MidiControl {
    /// As the inspector shows it, with channels counted from 1.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CC {} · ch {}", self.controller, u16::from(self.channel) + 1)
    }
}

/// Stable identifier of one MIDI mapping, for this session only. Patch files
/// identify a mapping by its control and target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MidiMappingId(pub u32);

/// One link from a hardware control to a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiMapping {
    /// The hardware control.
    pub control: MidiControl,
    /// The module whose parameter it sets.
    pub target: ModuleId,
    /// Parameter index on [`Self::target`].
    pub param: usize,
}

/// The last control change the engine received, for learning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlChangeSeen {
    /// Which control moved.
    pub control: MidiControl,
    /// Its value, 0 to 127.
    pub value: u8,
    /// Goes up by one with each control change. Compare two readings to see
    /// whether a new one arrived in between.
    pub serial: u32,
}

/// The parameter value a control change sets: `value / 127` as a position,
/// mapped through the parameter (spec, Playing).
pub fn control_value(spec: &ParamSpec, value: u8) -> f32 {
    spec.denormalize(f32::from(value.min(127)) / 127.0)
}

/// One compiled mapping, as the engine reads it.
pub(crate) struct MidiMapOp {
    pub control: MidiControl,
    pub id: MidiMappingId,
    /// The feedback sequence number the graph held when this schedule was
    /// compiled. A newer one at the swap means a control change arrived in
    /// between, and the engine applies it again on the new schedule.
    pub synced: Option<u32>,
    pub slot: u32,
    pub param: u16,
    pub spec: &'static ParamSpec,
}

/// What the engine reports to the controller. See the module docs.
pub(crate) struct MidiFeedback {
    entries: Box<[AtomicU64]>,
    /// The last screen edit the engine applied, per control:
    /// `token << 32 | sequence`.
    edits: Box<[AtomicU64]>,
    /// `serial << 32 | channel << 16 | controller << 8 | value`. 0 is none.
    last: AtomicU64,
}

/// One feedback entry, unpacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FeedbackEntry {
    pub id: MidiMappingId,
    pub sequence: u32,
    pub value: u8,
}

impl MidiFeedback {
    pub fn new() -> Self {
        Self {
            entries: (0..CHANNELS * CONTROLLERS).map(|_| AtomicU64::new(0)).collect(),
            edits: (0..CHANNELS * CONTROLLERS).map(|_| AtomicU64::new(0)).collect(),
            last: AtomicU64::new(0),
        }
    }

    /// Engine: mapping `id` set its parameter from `value`. Only the low 24
    /// bits of `sequence` are kept; the reader only compares them.
    pub fn write(&self, control: MidiControl, id: MidiMappingId, sequence: u32, value: u8) {
        let packed = (u64::from(id.0) << 32) | (u64::from(sequence & 0x00FF_FFFF) << 8) | u64::from(value & 0x7F);
        self.entries[control.index()].store(packed, Ordering::Release);
    }

    /// Controller: the latest write for `control`. Mapping ids start at 1, so
    /// an entry never written reads as id 0.
    pub fn read(&self, control: MidiControl) -> FeedbackEntry {
        let packed = self.entries[control.index()].load(Ordering::Acquire);
        FeedbackEntry {
            id: MidiMappingId((packed >> 32) as u32),
            sequence: ((packed >> 8) & 0x00FF_FFFF) as u32,
            value: (packed & 0x7F) as u8,
        }
    }

    /// Engine: the screen edit with `token` reached the mapped parameter of
    /// `control` at `sequence`.
    pub fn write_edit(&self, control: MidiControl, token: u32, sequence: u32) {
        let packed = (u64::from(token) << 32) | u64::from(sequence & 0x00FF_FFFF);
        self.edits[control.index()].store(packed, Ordering::Release);
    }

    /// Controller: the last screen edit applied for `control`, as
    /// `(token, sequence)`. Tokens start at 1, so none reads as token 0.
    pub fn read_edit(&self, control: MidiControl) -> (u32, u32) {
        let packed = self.edits[control.index()].load(Ordering::Acquire);
        ((packed >> 32) as u32, (packed & 0x00FF_FFFF) as u32)
    }

    /// Engine: a control change arrived. `serial` is never 0.
    pub fn record(&self, control: MidiControl, value: u8, serial: u32) {
        let packed = (u64::from(serial) << 32)
            | (u64::from(control.channel) << 16)
            | (u64::from(control.controller) << 8)
            | u64::from(value & 0x7F);
        self.last.store(packed, Ordering::Release);
    }

    /// Controller: the last control change, or `None` before the first.
    pub fn last(&self) -> Option<ControlChangeSeen> {
        let packed = self.last.load(Ordering::Acquire);
        let serial = (packed >> 32) as u32;
        (serial != 0).then_some(ControlChangeSeen {
            control: MidiControl {
                channel: ((packed >> 16) & 0x0F) as u8,
                controller: ((packed >> 8) & 0x7F) as u8,
            },
            value: (packed & 0x7F) as u8,
            serial,
        })
    }
}

/// True when 24-bit sequence number `a` came after `b`. Sequences wrap, so
/// "after" means less than half the range ahead.
pub(crate) fn later(a: u32, b: u32) -> bool {
    let ahead = a.wrapping_sub(b) & 0x00FF_FFFF;
    ahead != 0 && ahead < 0x0080_0000
}

/// Reads the last control change from any thread, for example a UI watcher
/// that wakes the UI only when MIDI arrives. Cheap to clone. Get one from
/// [`Controller::midi_reader`](crate::Controller::midi_reader).
#[derive(Clone)]
pub struct MidiReader {
    feedback: Arc<MidiFeedback>,
}

impl fmt::Debug for MidiReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MidiReader")
    }
}

impl MidiReader {
    pub(crate) fn new(feedback: Arc<MidiFeedback>) -> Self {
        Self { feedback }
    }

    /// The last control change the engine received, or `None` before the
    /// first. Its serial changes with every control change, mapped or not.
    pub fn last_control_change(&self) -> Option<ControlChangeSeen> {
        self.feedback.last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_round_trips() {
        let table = MidiFeedback::new();
        let control = MidiControl {
            channel: 15,
            controller: 127,
        };
        assert_eq!(table.read(control).id, MidiMappingId(0));
        table.write(control, MidiMappingId(9), 0x0123_4567, 100);
        assert_eq!(
            table.read(control),
            FeedbackEntry {
                id: MidiMappingId(9),
                sequence: 0x23_4567,
                value: 100
            }
        );
    }

    #[test]
    fn sequences_compare_across_the_wrap() {
        assert!(later(5, 4));
        assert!(!later(4, 5));
        assert!(!later(4, 4));
        assert!(later(1, 0x00FF_FFFF));
        assert!(!later(0x00FF_FFFF, 1));
    }

    #[test]
    fn last_control_change_round_trips() {
        let table = MidiFeedback::new();
        assert_eq!(table.last(), None);
        let control = MidiControl {
            channel: 3,
            controller: 74,
        };
        table.record(control, 64, 7);
        assert_eq!(
            table.last(),
            Some(ControlChangeSeen {
                control,
                value: 64,
                serial: 7
            })
        );
    }

    #[test]
    fn shows_channels_from_one() {
        let control = MidiControl {
            channel: 0,
            controller: 74,
        };
        assert_eq!(control.to_string(), "CC 74 · ch 1");
    }
}
