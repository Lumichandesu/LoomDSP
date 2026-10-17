//! MIDI input modules (FR-LIB-07).

use loom_core::signal::pitch;
use loom_core::{
    MidiMessage, Module, ModuleDescriptor, ModuleRole, Panel, ParamOptions, ParamSpec, Polarity, PortDefault, PortIo,
    PortSpec, ProcessContext, SignalType, Widget,
};

/// `loom.midi.note`: monophonic MIDI note input with last-note priority.
pub static NOTE_IN: ModuleDescriptor = ModuleDescriptor {
    id: "loom.midi.note",
    version: 1,
    name: "MIDI Note",
    category: "MIDI",
    description: "Monophonic note input (last-note priority). Pitch includes pitch bend.",
    inputs: &[],
    outputs: &[
        PortSpec {
            name: "pitch",
            label: "Pitch",
            signal: SignalType::Pitch,
            description: "Pitch of the last note held, with pitch bend. 0 is middle C.",
            default: PortDefault::Value(0.0),
            polarity: Polarity::Bipolar,
        },
        PortSpec {
            name: "gate",
            label: "Gate",
            signal: SignalType::Gate,
            description: "High while any note is held.",
            default: PortDefault::Value(0.0),
            polarity: Polarity::Unipolar,
        },
        PortSpec {
            name: "velocity",
            label: "Velocity",
            signal: SignalType::Modulation,
            description: "How hard the last note was played, from 0 to 1.",
            default: PortDefault::Value(0.0),
            polarity: Polarity::Unipolar,
        },
    ],
    params: &[
        ParamSpec {
            name: "channel",
            label: "Channel",
            description: "The MIDI channel to listen to, from 1 to 16. 0 listens to all.",
            min: 0.0,
            max: 16.0,
            default: 0.0,
            unit: "",
            options: ParamOptions::DEFAULT,
        }
        .integer(),
        ParamSpec {
            name: "bend-range",
            label: "Bend Range",
            description: "How far a full pitch bend moves the pitch.",
            min: 0.0,
            max: 48.0,
            default: 2.0,
            unit: "st",
            options: ParamOptions::DEFAULT,
        }
        .integer(),
    ],
    role: ModuleRole::Dsp(|_| Box::new(NoteIn::default())),
    panel: Panel::new(3, 2, &[Widget::light(0, 0, "gate")]),
    displays: &["gate"],
};

const MAX_HELD: usize = 16;

#[derive(Default)]
struct NoteIn {
    held: [u8; MAX_HELD],
    held_len: usize,
    note: u8,
    velocity: f32,
    bend: f32,
    channel: u8,
    bend_range: f32,
}

impl NoteIn {
    fn accepts(&self, channel: u8) -> bool {
        self.channel == 0 || self.channel == channel + 1
    }

    fn release(&mut self, note: u8) {
        if let Some(i) = self.held[..self.held_len].iter().position(|&n| n == note) {
            self.held.copy_within(i + 1..self.held_len, i);
            self.held_len -= 1;
        }
    }

    fn press(&mut self, note: u8, velocity: u8) {
        self.release(note);
        if self.held_len == MAX_HELD {
            // Forget the oldest held note.
            self.held.copy_within(1.., 0);
            self.held_len -= 1;
        }
        self.held[self.held_len] = note;
        self.held_len += 1;
        self.note = note;
        self.velocity = f32::from(velocity) / 127.0;
    }

    fn handle(&mut self, message: MidiMessage) {
        match message {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if self.accepts(channel) => self.press(note, velocity),
            MidiMessage::NoteOff { channel, note } if self.accepts(channel) => {
                self.release(note);
                if self.held_len > 0 {
                    self.note = self.held[self.held_len - 1];
                }
            }
            MidiMessage::PitchBend { channel, value } if self.accepts(channel) => {
                self.bend = f32::from(value) / 8192.0;
            }
            _ => {}
        }
    }
}

impl Module for NoteIn {
    fn process(&mut self, ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let channel = io.modulated(0);
        let bend_range = io.modulated(1);
        let ([], [pitch_out, gate_out, velocity_out]) = io.split::<0, 3>();
        let mut events = ctx.midi().peekable();
        for i in 0..pitch_out.len() {
            if let Some(samples) = channel {
                self.channel = samples[i].round() as u8;
            }
            if let Some(samples) = bend_range {
                self.bend_range = samples[i];
            }
            while let Some((frame, event)) = events.peek() {
                if *frame > i {
                    break;
                }
                self.handle(event.message());
                events.next();
            }
            let bend = self.bend * self.bend_range * pitch::PER_SEMITONE;
            pitch_out[i] = pitch::from_midi_note(f32::from(self.note)) + bend;
            gate_out[i] = if self.held_len > 0 { 1.0 } else { 0.0 };
            velocity_out[i] = self.velocity;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.channel = value.round() as u8,
            1 => self.bend_range = value,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.held_len = 0;
        self.bend = 0.0;
    }

    fn displays(&self, values: &mut [f32]) {
        values[0] = if self.held_len > 0 { 1.0 } else { 0.0 };
    }
}
