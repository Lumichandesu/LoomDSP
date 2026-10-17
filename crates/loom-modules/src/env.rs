//! Envelopes (FR-LIB-04).

use loom_core::signal::gate_is_high;
use loom_core::{
    Module, ModuleConfig, ModuleDescriptor, ModuleRole, Panel, ParamOptions, ParamSpec, Polarity, PortDefault, PortIo,
    PortSpec, ProcessContext, SignalType, Widget,
};

/// `loom.env.adsr`: attack, decay, sustain, release envelope.
pub static ADSR: ModuleDescriptor = ModuleDescriptor {
    id: "loom.env.adsr",
    version: 1,
    name: "ADSR",
    category: "Envelope",
    description: "Envelope from 0 to 1, triggered by a gate. Linear attack, exponential decay and release.",
    inputs: &[PortSpec {
        name: "gate",
        label: "Gate",
        signal: SignalType::Gate,
        description: "Going high starts the envelope. Going low releases it.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Unipolar,
    }],
    outputs: &[PortSpec {
        name: "out",
        label: "Out",
        signal: SignalType::Modulation,
        description: "The envelope level, from 0 to 1.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Unipolar,
    }],
    params: &[
        ParamSpec {
            name: "attack",
            label: "Attack",
            description: "Time to rise from 0 to 1 after the gate goes high.",
            min: 0.0005,
            max: 10.0,
            default: 0.01,
            unit: "s",
            options: ParamOptions::DEFAULT,
        }
        .logarithmic(),
        ParamSpec {
            name: "decay",
            label: "Decay",
            description: "Time to fall from 1 to the sustain level.",
            min: 0.0005,
            max: 10.0,
            default: 0.2,
            unit: "s",
            options: ParamOptions::DEFAULT,
        }
        .logarithmic(),
        ParamSpec {
            name: "sustain",
            label: "Sustain",
            description: "The level held while the gate stays high.",
            min: 0.0,
            max: 1.0,
            default: 0.7,
            unit: "",
            options: ParamOptions::DEFAULT,
        },
        ParamSpec {
            name: "release",
            label: "Release",
            description: "Time to fall to 0 after the gate goes low.",
            min: 0.0005,
            max: 10.0,
            default: 0.3,
            unit: "s",
            options: ParamOptions::DEFAULT,
        }
        .logarithmic(),
    ],
    role: ModuleRole::Dsp(|config| Box::new(Adsr::new(config))),
    panel: Panel::new(
        6,
        8,
        &[
            Widget::knob(0, 0, "attack"),
            Widget::knob(3, 0, "decay"),
            Widget::knob(0, 3, "sustain"),
            Widget::knob(3, 3, "release"),
            Widget::light(0, 6, "level"),
        ],
    ),
    displays: &["level"],
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Release,
}

struct Adsr {
    sample_rate: f32,
    stage: Stage,
    level: f32,
    gate_was_high: bool,
    attack_step: f32,
    decay_coefficient: f32,
    sustain: f32,
    release_coefficient: f32,
}

/// Coefficient for an exponential segment that falls by 60 dB in `seconds`.
fn exp_coefficient(seconds: f32, sample_rate: f32) -> f32 {
    (-(1000.0f32).ln() / (seconds * sample_rate).max(1.0)).exp()
}

impl Adsr {
    fn new(config: &ModuleConfig) -> Self {
        let sample_rate = config.sample_rate;
        Self {
            sample_rate,
            stage: Stage::Idle,
            level: 0.0,
            gate_was_high: false,
            attack_step: 1.0 / (0.01 * sample_rate),
            decay_coefficient: exp_coefficient(0.2, sample_rate),
            sustain: 0.7,
            release_coefficient: exp_coefficient(0.3, sample_rate),
        }
    }
}

impl Module for Adsr {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let attack = io.modulated(0);
        let decay = io.modulated(1);
        let sustain = io.modulated(2);
        let release = io.modulated(3);
        let ([gate], [out]) = io.split::<1, 1>();
        for (i, (out, &g)) in out.iter_mut().zip(gate).enumerate() {
            if attack.is_some() || decay.is_some() || sustain.is_some() || release.is_some() {
                if let Some(samples) = attack {
                    self.attack_step = 1.0 / (samples[i] * self.sample_rate).max(1.0);
                }
                if let Some(samples) = decay {
                    self.decay_coefficient = exp_coefficient(samples[i], self.sample_rate);
                }
                if let Some(samples) = sustain {
                    self.sustain = samples[i];
                }
                if let Some(samples) = release {
                    self.release_coefficient = exp_coefficient(samples[i], self.sample_rate);
                }
            }
            let high = gate_is_high(g);
            if high && !self.gate_was_high {
                // Start from the current level: no click on retrigger.
                self.stage = Stage::Attack;
            } else if !high && self.gate_was_high {
                self.stage = Stage::Release;
            }
            self.gate_was_high = high;

            match self.stage {
                Stage::Idle => self.level = 0.0,
                Stage::Attack => {
                    self.level += self.attack_step;
                    if self.level >= 1.0 {
                        self.level = 1.0;
                        self.stage = Stage::Decay;
                    }
                }
                Stage::Decay => {
                    self.level = self.sustain + (self.level - self.sustain) * self.decay_coefficient;
                }
                Stage::Release => {
                    self.level *= self.release_coefficient;
                    if self.level < 1e-5 {
                        self.level = 0.0;
                        self.stage = Stage::Idle;
                    }
                }
            }
            *out = self.level;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.attack_step = 1.0 / (value * self.sample_rate).max(1.0),
            1 => self.decay_coefficient = exp_coefficient(value, self.sample_rate),
            2 => self.sustain = value,
            3 => self.release_coefficient = exp_coefficient(value, self.sample_rate),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.level = 0.0;
        self.gate_was_high = false;
    }

    fn displays(&self, values: &mut [f32]) {
        values[0] = self.level;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loom_core::testing::run;

    #[test]
    fn follows_gate() {
        let config = ModuleConfig {
            sample_rate: 1_000.0,
            max_block: 2_000,
        };
        let ModuleRole::Dsp(create) = ADSR.role else {
            unreachable!()
        };
        let mut env = create(&config);
        for (i, p) in ADSR.params.iter().enumerate() {
            env.set_param(i, p.default);
        }
        let mut gate = vec![1.0; 2_000];
        gate[1_000..].fill(0.0);
        let mut out = vec![0.0; 2_000];
        run(env.as_mut(), 1_000.0, &[&gate], &[true], &mut [&mut out], &[]);
        assert!((out[999] - 0.7).abs() < 0.01, "sustain level, got {}", out[999]);
        assert!(out[1_999] < 0.01, "released, got {}", out[1_999]);
        assert!(out[5] > 0.0 && out[5] < 1.0, "attack ramps");
    }
}
