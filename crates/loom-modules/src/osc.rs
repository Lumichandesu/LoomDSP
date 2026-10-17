//! Wave shapers and noise (FR-LIB-02).
//!
//! Shapers take a phase input instead of a frequency. Connect a Phase module
//! to drive them, or any other signal to bend the wave shape.

use loom_core::{
    Module, ModuleConfig, ModuleDescriptor, ModuleRole, Panel, ParamOptions, ParamSpec, Polarity, PortDefault, PortIo,
    PortSpec, ProcessContext, SignalType,
};
use std::f32::consts::TAU;

/// `loom.osc.sine`: sine of the phase input.
pub static SINE: ModuleDescriptor = ModuleDescriptor {
    id: "loom.osc.sine",
    version: 1,
    name: "Sine",
    category: "Oscillator",
    description: "Sine wave shaped from a phase input.",
    inputs: &[PortSpec {
        name: "phase",
        label: "Phase",
        signal: SignalType::Phase,
        description: "Position in the wave, from 0 to 1. One ramp is one cycle.",
        default: PortDefault::Value(0.0),

        polarity: Polarity::Unipolar,
    }],
    outputs: &[PortSpec {
        name: "out",
        label: "Out",
        signal: SignalType::Audio,
        description: "Sine wave from -1 to 1.",
        default: PortDefault::Value(0.0),

        polarity: Polarity::Bipolar,
    }],
    params: &[],
    role: ModuleRole::Dsp(|_| Box::new(Sine)),
    panel: Panel::EMPTY,
    displays: &[],
};

struct Sine;

impl Module for Sine {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let ([phase], [out]) = io.split::<1, 1>();
        for (out, &p) in out.iter_mut().zip(phase) {
            *out = (TAU * (p - p.floor())).sin();
        }
    }
}

/// `loom.osc.saw`: band-limited saw shaped from a phase input.
pub static SAW: ModuleDescriptor = ModuleDescriptor {
    id: "loom.osc.saw",
    version: 1,
    name: "Saw",
    category: "Oscillator",
    description: "Band-limited saw wave shaped from a phase input (polyBLEP).",
    inputs: &[PortSpec {
        name: "phase",
        label: "Phase",
        signal: SignalType::Phase,
        description: "Position in the wave, from 0 to 1. One ramp is one cycle.",
        default: PortDefault::Value(0.0),

        polarity: Polarity::Unipolar,
    }],
    outputs: &[PortSpec {
        name: "out",
        label: "Out",
        signal: SignalType::Audio,
        description: "Saw wave from -1 to 1.",
        default: PortDefault::Value(0.0),

        polarity: Polarity::Bipolar,
    }],
    params: &[],
    role: ModuleRole::Dsp(|_| Box::new(Saw { previous: 0.0 })),
    panel: Panel::EMPTY,
    displays: &[],
};

struct Saw {
    previous: f32,
}

/// Polynomial band-limited step. Smooths the jump at the wrap point, which
/// removes most of the aliasing a naive saw produces (NFR-SND-01).
#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        2.0 * x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}

impl Module for Saw {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let ([phase], [out]) = io.split::<1, 1>();
        for (out, &p) in out.iter_mut().zip(phase) {
            let t = p - p.floor();
            // The phase step tells us the frequency, so the shaper does not
            // need a separate frequency input.
            let mut dt = t - self.previous;
            dt -= dt.floor();
            self.previous = t;
            let mut y = 2.0 * t - 1.0;
            if dt > 0.0 && dt < 0.5 {
                y -= poly_blep(t, dt);
            }
            *out = y;
        }
    }

    fn reset(&mut self) {
        self.previous = 0.0;
    }
}

/// `loom.osc.noise`: white noise from a seeded generator.
pub static NOISE: ModuleDescriptor = ModuleDescriptor {
    id: "loom.osc.noise",
    version: 1,
    name: "Noise",
    category: "Oscillator",
    description: "White noise. The seed makes renders repeatable (NFR-SND-04).",
    inputs: &[],
    outputs: &[PortSpec {
        name: "out",
        label: "Out",
        signal: SignalType::Audio,
        description: "White noise from -1 to 1.",
        default: PortDefault::Value(0.0),

        polarity: Polarity::Bipolar,
    }],
    params: &[ParamSpec {
        name: "seed",
        label: "Seed",
        description: "Picks the noise pattern. The same seed gives the same noise.",
        min: 1.0,
        max: 65_535.0,
        default: 1.0,
        unit: "",
        options: ParamOptions::DEFAULT,
    }
    .integer()],
    role: ModuleRole::Dsp(|config: &ModuleConfig| {
        let _ = config;
        Box::new(Noise {
            state: 1,
            seed: 1,
            active: 1,
        })
    }),
    panel: Panel::EMPTY,
    displays: &[],
};

struct Noise {
    state: u32,
    /// The knob's base seed. Only `set_param` writes it.
    seed: u32,
    /// The seed `state` was last started from: the modulated seed while an
    /// assignment is active, otherwise the base.
    active: u32,
}

impl Module for Noise {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let seed = io.modulated(0);
        let ([], [out]) = io.split::<0, 1>();
        for (i, out) in out.iter_mut().enumerate() {
            // Restart the pattern when the seed in use changes, including
            // back to the base when an assignment is removed.
            let next = seed.map_or(self.seed, |samples| samples[i].round().max(1.0) as u32);
            if next != self.active {
                self.active = next;
                self.state = next;
            }
            // xorshift32: fast, deterministic, good enough for audio noise.
            let mut x = self.state;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.state = x;
            *out = (x as f32 / u32::MAX as f32) * 2.0 - 1.0;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.seed = value.round().max(1.0) as u32;
            self.active = self.seed;
            self.state = self.seed;
        }
    }

    fn reset(&mut self) {
        self.state = self.active;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loom_core::testing::run;

    #[test]
    fn saw_is_bounded_and_rises() {
        let config = ModuleConfig {
            sample_rate: 48_000.0,
            max_block: 64,
        };
        let ModuleRole::Dsp(create) = SAW.role else {
            unreachable!()
        };
        let mut saw = create(&config);
        let phase: Vec<f32> = (0..64).map(|i| (i as f32 * 0.01) % 1.0).collect();
        let mut out = vec![0.0; 64];
        run(saw.as_mut(), 48_000.0, &[&phase], &[true], &mut [&mut out], &[]);
        assert!(out.iter().all(|v| v.abs() <= 1.0 + 1e-6));
        assert!(out[10] > out[5]);
    }
}
