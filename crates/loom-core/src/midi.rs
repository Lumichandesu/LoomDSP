//! MIDI events as seen by modules.

/// A short MIDI message with a frame offset.
///
/// The frame is relative to the start of the buffer passed to
/// [`Engine::process`](crate::Engine::process). Live input usually uses 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiEvent {
    /// Frame offset within the next processed buffer.
    pub frame: u32,
    /// Raw bytes. Unused bytes are zero.
    pub data: [u8; 3],
    /// Number of valid bytes in `data` (1 to 3).
    pub len: u8,
}

/// A decoded channel message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMessage {
    /// Note on with velocity above zero.
    NoteOn {
        /// Channel, 0 to 15.
        channel: u8,
        /// Note number.
        note: u8,
        /// Velocity, 1 to 127.
        velocity: u8,
    },
    /// Note off (or note on with velocity zero).
    NoteOff {
        /// Channel, 0 to 15.
        channel: u8,
        /// Note number.
        note: u8,
    },
    /// Control change.
    ControlChange {
        /// Channel, 0 to 15.
        channel: u8,
        /// Controller number.
        controller: u8,
        /// Value, 0 to 127.
        value: u8,
    },
    /// Pitch bend, -8192 to 8191.
    PitchBend {
        /// Channel, 0 to 15.
        channel: u8,
        /// Bend amount.
        value: i16,
    },
    /// Anything else.
    Other,
}

impl MidiEvent {
    /// Builds an event from raw bytes. Extra bytes beyond 3 are ignored.
    pub fn from_bytes(frame: u32, bytes: &[u8]) -> Self {
        let mut data = [0u8; 3];
        let len = bytes.len().min(3);
        data[..len].copy_from_slice(&bytes[..len]);
        Self {
            frame,
            data,
            len: len as u8,
        }
    }

    /// A note-on event.
    pub fn note_on(frame: u32, channel: u8, note: u8, velocity: u8) -> Self {
        Self::from_bytes(frame, &[0x90 | (channel & 0x0F), note & 0x7F, velocity & 0x7F])
    }

    /// A note-off event.
    pub fn note_off(frame: u32, channel: u8, note: u8) -> Self {
        Self::from_bytes(frame, &[0x80 | (channel & 0x0F), note & 0x7F, 0])
    }

    /// A control change event.
    pub fn control_change(frame: u32, channel: u8, controller: u8, value: u8) -> Self {
        Self::from_bytes(frame, &[0xB0 | (channel & 0x0F), controller & 0x7F, value & 0x7F])
    }

    /// Decodes the event.
    pub fn message(&self) -> MidiMessage {
        if self.len < 2 {
            return MidiMessage::Other;
        }
        let status = self.data[0] & 0xF0;
        let channel = self.data[0] & 0x0F;
        let a = self.data[1];
        let b = self.data[2];
        match status {
            0x90 if self.len == 3 && b > 0 => MidiMessage::NoteOn {
                channel,
                note: a,
                velocity: b,
            },
            0x90 | 0x80 if self.len == 3 => MidiMessage::NoteOff { channel, note: a },
            0xB0 if self.len == 3 => MidiMessage::ControlChange {
                channel,
                controller: a,
                value: b,
            },
            0xE0 if self.len == 3 => MidiMessage::PitchBend {
                channel,
                value: ((i16::from(b) << 7) | i16::from(a)) - 8192,
            },
            _ => MidiMessage::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_notes() {
        assert_eq!(
            MidiEvent::note_on(0, 1, 60, 100).message(),
            MidiMessage::NoteOn {
                channel: 1,
                note: 60,
                velocity: 100
            }
        );
        assert_eq!(
            MidiEvent::note_on(0, 0, 60, 0).message(),
            MidiMessage::NoteOff { channel: 0, note: 60 }
        );
        assert_eq!(
            MidiEvent::from_bytes(0, &[0xE0, 0x00, 0x40]).message(),
            MidiMessage::PitchBend { channel: 0, value: 0 }
        );
    }
}
