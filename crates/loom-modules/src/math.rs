//! Math building blocks (FR-LIB-05).
//!
//! Unconnected inputs fall back to a sensible default (0 for add, 1 for
//! multiply), so a module is useful with only one cable.

use loom_core::{
    Module, ModuleConfig, ModuleDescriptor, ModuleRole, Panel, ParamOptions, ParamSpec, Polarity, PortDefault, PortIo,
    PortSpec, ProcessContext, SignalType, Widget,
};
use loom_rt::Smoothed;

/// `loom.math.add`: `a + b + offset`.
pub static ADD: ModuleDescriptor = ModuleDescriptor {
    id: "loom.math.add",
    version: 1,
    name: "Add",
    category: "Math",
    description: "Adds two signals and an offset.",
    inputs: &[
        PortSpec {
            name: "a",
            label: "A",
            signal: SignalType::Modulation,
            description: "The first signal to add.",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        },
        PortSpec {
            name: "b",
            label: "B",
            signal: SignalType::Modulation,
            description: "The second signal to add.",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        },
    ],
    outputs: &[PortSpec {
        name: "out",
        label: "Out",
        signal: SignalType::Modulation,
        description: "A plus B plus the offset.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Bipolar,
    }],
    params: &[ParamSpec {
        name: "offset",
        label: "Offset",
        description: "A fixed amount added to the sum.",
        min: -10.0,
        max: 10.0,
        default: 0.0,
        unit: "",
        options: ParamOptions::DEFAULT,
    }
    .smooth(0.005)],
    role: ModuleRole::Dsp(|config| {
        Box::new(Add {
            offset: Smoothed::new(0.0, config.sample_rate, ADD.params[0].options.smooth_seconds),
        })
    }),
    panel: Panel::new(3, 3, &[Widget::knob(0, 0, "offset")]),
    displays: &[],
};

struct Add {
    offset: Smoothed,
}

impl Module for Add {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let modulated = io.modulated(0);
        let ([a, b], [out]) = io.split::<2, 1>();
        for (i, ((out, &a), &b)) in out.iter_mut().zip(a).zip(b).enumerate() {
            let offset = if let Some(samples) = modulated {
                let _ = self.offset.tick();
                samples[i]
            } else {
                self.offset.tick()
            };
            *out = a + b + offset;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.offset.set_target(value);
        }
    }
}

/// `loom.math.mul`: `a * b * gain`. An unconnected `b` counts as 1.
pub static MUL: ModuleDescriptor = ModuleDescriptor {
    id: "loom.math.mul",
    version: 1,
    name: "Multiply",
    category: "Math",
    description: "Multiplies two signals and a gain. Use it as a VCA or ring modulator.",
    inputs: &[
        PortSpec {
            name: "a",
            label: "A",
            signal: SignalType::Audio,
            description: "The signal to scale.",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        },
        PortSpec {
            name: "b",
            label: "B",
            signal: SignalType::Modulation,
            description: "Scales A. Reads 1 while no cable is connected.",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        },
    ],
    outputs: &[PortSpec {
        name: "out",
        label: "Out",
        signal: SignalType::Audio,
        description: "A times B times the gain.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Bipolar,
    }],
    params: &[ParamSpec {
        name: "gain",
        label: "Gain",
        description: "A fixed amount the product is multiplied by.",
        min: -10.0,
        max: 10.0,
        default: 1.0,
        unit: "",
        options: ParamOptions::DEFAULT,
    }
    .smooth(0.005)],
    role: ModuleRole::Dsp(|config| {
        Box::new(Mul {
            gain: Smoothed::new(1.0, config.sample_rate, MUL.params[0].options.smooth_seconds),
        })
    }),
    panel: Panel::new(3, 3, &[Widget::knob(0, 0, "gain")]),
    displays: &[],
};

struct Mul {
    gain: Smoothed,
}

impl Module for Mul {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let b_connected = io.is_connected(1);
        let modulated = io.modulated(0);
        let ([a, b], [out]) = io.split::<2, 1>();
        for (i, ((out, &a), &b)) in out.iter_mut().zip(a).zip(b).enumerate() {
            let b = if b_connected { b } else { 1.0 };
            let gain = if let Some(samples) = modulated {
                let _ = self.gain.tick();
                samples[i]
            } else {
                self.gain.tick()
            };
            *out = a * b * gain;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.gain.set_target(value);
        }
    }
}

/// `loom.math.constant`: a fixed value, set by a parameter.
pub static CONSTANT: ModuleDescriptor = ModuleDescriptor {
    id: "loom.math.constant",
    version: 1,
    name: "Constant",
    category: "Math",
    description: "Outputs a constant value. Useful for pitch offsets, gates, and testing.",
    inputs: &[],
    outputs: &[PortSpec {
        name: "out",
        label: "Out",
        signal: SignalType::Modulation,
        description: "The value set by the parameter.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Bipolar,
    }],
    params: &[ParamSpec {
        name: "value",
        label: "Value",
        description: "The number the output holds.",
        min: -10.0,
        max: 10.0,
        default: 0.0,
        unit: "",
        options: ParamOptions::DEFAULT,
    }
    .smooth(0.005)],
    role: ModuleRole::Dsp(|config: &ModuleConfig| {
        Box::new(Constant {
            value: Smoothed::new(0.0, config.sample_rate, CONSTANT.params[0].options.smooth_seconds),
        })
    }),
    panel: Panel::new(3, 3, &[Widget::knob(0, 0, "value")]),
    displays: &[],
};

struct Constant {
    value: Smoothed,
}

impl Module for Constant {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let modulated = io.modulated(0);
        let ([], [out]) = io.split::<0, 1>();
        for (i, out) in out.iter_mut().enumerate() {
            *out = if let Some(samples) = modulated {
                let _ = self.value.tick();
                samples[i]
            } else {
                self.value.tick()
            };
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.value.set_target(value);
        }
    }

    fn reset(&mut self) {
        let target = self.value.target();
        self.value.reset(target);
    }
}
