//! Filters (FR-LIB-03).

use loom_core::signal::pitch;
use loom_core::{
    Module, ModuleConfig, ModuleDescriptor, ModuleRole, Panel, ParamOptions, ParamSpec, Polarity, PortDefault, PortIo,
    PortSpec, ProcessContext, SignalType, Widget,
};
use loom_rt::Smoothed;
use std::f32::consts::PI;

/// `loom.filter.svf`: state variable filter with lowpass, bandpass, and
/// highpass outputs.
pub static SVF: ModuleDescriptor = ModuleDescriptor {
    id: "loom.filter.svf",
    version: 1,
    name: "SVF",
    category: "Filter",
    description: "State variable filter (trapezoidal integration). The cutoff input adds pitch: +0.1 doubles the cutoff.",
    inputs: &[
        PortSpec {
            name: "in",
            label: "In",
            signal: SignalType::Audio,
            description: "The audio to filter.",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        },
        PortSpec {
            name: "cutoff",
            label: "Cutoff",
            signal: SignalType::Pitch,
            description: "Moves the cutoff as pitch: +0.1 doubles it.",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        },
    ],
    outputs: &[
        PortSpec {
            name: "lowpass",
            label: "Lowpass",
            signal: SignalType::Audio,
            description: "Keeps the frequencies below the cutoff.",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        },
        PortSpec {
            name: "bandpass",
            label: "Bandpass",
            signal: SignalType::Audio,
            description: "Keeps the frequencies near the cutoff.",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        },
        PortSpec {
            name: "highpass",
            label: "Highpass",
            signal: SignalType::Audio,
            description: "Keeps the frequencies above the cutoff.",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        },
    ],
    params: &[
        ParamSpec {
            name: "cutoff",
            label: "Cutoff",
            description: "The frequency where the filter starts to cut.",
            min: 20.0,
            max: 20_000.0,
            default: 1_000.0,
            unit: "Hz",
            options: ParamOptions::DEFAULT,
        }
        .logarithmic()
        .smooth(0.01),
        ParamSpec {
            name: "resonance",
            label: "Resonance",
            description: "Boosts the frequencies near the cutoff. High values ring.",
            min: 0.0,
            max: 1.0,
            default: 0.2,
            unit: "",
            options: ParamOptions::DEFAULT,
        }
        .smooth(0.01),
    ],
    role: ModuleRole::Dsp(|config| Box::new(Svf::new(config))),
    panel: Panel::new(6, 3, &[Widget::knob(0, 0, "cutoff"), Widget::knob(3, 0, "resonance")]),
    displays: &[],
};

struct Svf {
    sample_rate: f32,
    cutoff: Smoothed,
    resonance: Smoothed,
    ic1: f32,
    ic2: f32,
}

impl Svf {
    fn new(config: &ModuleConfig) -> Self {
        Self {
            sample_rate: config.sample_rate,
            // Smooth cutoff in the pitch domain, so sweeps sound even.
            // The time matches `ParamSpec::smooth_seconds`, which the engine
            // uses once an assignment is modulating this knob.
            cutoff: Smoothed::new(
                pitch::from_hz(1_000.0),
                config.sample_rate,
                SVF.params[0].options.smooth_seconds,
            ),
            resonance: Smoothed::new(0.2, config.sample_rate, SVF.params[1].options.smooth_seconds),
            ic1: 0.0,
            ic2: 0.0,
        }
    }
}

impl Module for Svf {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let cutoff_samples = io.modulated(0);
        let resonance_samples = io.modulated(1);
        let ([input, cutoff_mod], [lp, bp, hp]) = io.split::<2, 3>();
        let nyquist_limit = self.sample_rate * 0.49;
        for i in 0..input.len() {
            // Tick even when modulated, so removing the assignment continues
            // the glide from here. The heard value is the engine buffer.
            let cutoff_pitch = self.cutoff.tick();
            let cutoff_hz = if let Some(samples) = cutoff_samples {
                pitch::to_hz(pitch::from_hz(samples[i]) + cutoff_mod[i])
            } else {
                pitch::to_hz(cutoff_pitch + cutoff_mod[i])
            }
            .clamp(5.0, nyquist_limit);
            let resonance = if let Some(samples) = resonance_samples {
                let _ = self.resonance.tick();
                samples[i]
            } else {
                self.resonance.tick()
            };
            // Resonance 0..1 maps to damping k = 2..0.02 (Q from 0.5 to 50).
            let k = 2.0 - 1.98 * resonance;
            let g = (PI * cutoff_hz / self.sample_rate).tan();
            let a1 = 1.0 / (1.0 + g * (g + k));
            let a2 = g * a1;
            let a3 = g * a2;

            let x = input[i];
            let v3 = x - self.ic2;
            let v1 = a1 * self.ic1 + a2 * v3;
            let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
            self.ic1 = 2.0 * v1 - self.ic1;
            self.ic2 = 2.0 * v2 - self.ic2;

            lp[i] = v2;
            bp[i] = v1;
            hp[i] = x - k * v1 - v2;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.cutoff.set_target(pitch::from_hz(value)),
            1 => self.resonance.set_target(value),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}
