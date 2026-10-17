//! Modules the engine runs itself: the unit delay and the audio output.

use loom_core::{ModuleDescriptor, ModuleRole, Panel, Polarity, PortDefault, PortSpec, SignalType};

/// `loom.time.delay1`: delays its input by exactly one sample.
///
/// Put one in a feedback loop to choose where the loop's delay sits. A loop
/// without one gets a delay inserted automatically (ADR-0002).
pub static UNIT_DELAY: ModuleDescriptor = ModuleDescriptor {
    id: "loom.time.delay1",
    version: 1,
    name: "1-Sample Delay",
    category: "Time",
    description: "Delays the input by one sample. Marks where a feedback loop is broken.",
    inputs: &[PortSpec {
        name: "in",
        label: "In",
        signal: SignalType::Audio,
        description: "The signal to delay.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Bipolar,
    }],
    outputs: &[PortSpec {
        name: "out",
        label: "Out",
        signal: SignalType::Audio,
        description: "The input, one sample later.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Bipolar,
    }],
    params: &[],
    role: ModuleRole::UnitDelay,
    panel: Panel::EMPTY,
    displays: &[],
};

/// `loom.io.audio-out`: sends audio to the speakers or host.
pub static AUDIO_OUT: ModuleDescriptor = ModuleDescriptor {
    id: "loom.io.audio-out",
    version: 1,
    name: "Audio Out",
    category: "I/O",
    description: "Sends audio out. A cable on only one side plays on both sides.",
    inputs: &[
        PortSpec {
            name: "left",
            label: "Left",
            signal: SignalType::Audio,
            description: "The left channel. Plays on both sides if Right has no cable.",
            default: PortDefault::Value(0.0),
            polarity: Polarity::Bipolar,
        },
        PortSpec {
            name: "right",
            label: "Right",
            signal: SignalType::Audio,
            description: "The right channel. Plays on both sides if Left has no cable.",
            default: PortDefault::Value(0.0),
            polarity: Polarity::Bipolar,
        },
    ],
    outputs: &[],
    params: &[],
    role: ModuleRole::AudioOutput,
    panel: Panel::EMPTY,
    displays: &[],
};
