//! Test modules and helpers for the transport tests.

#![allow(dead_code)]

use loom_core::{
    EngineConfig, EngineParts, Module, ModuleDescriptor, ModuleId, ModuleRole, Polarity, PortDefault, PortIo, PortSpec,
    ProbeId, ProbeKind, ProcessContext, Registry, SignalType, create_engine,
};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const SAMPLE_RATE: f32 = 48_000.0;

/// A port for test descriptors.
const fn port(name: &'static str, signal: SignalType) -> PortSpec {
    PortSpec {
        name,
        label: name,
        signal,
        description: "A test port.",
        default: PortDefault::Value(0.0),
        polarity: match signal {
            SignalType::Gate | SignalType::Phase => Polarity::Unipolar,
            SignalType::Audio | SignalType::Pitch | SignalType::Modulation => Polarity::Bipolar,
        },
    }
}

/// Output indices of [`FOLLOWER`].
pub mod out {
    pub const BAR: usize = 0;
    pub const BEAT: usize = 1;
    pub const PLAY: usize = 2;
    pub const VALUE: usize = 3;
    pub const CONNECTED: usize = 4;
    pub const JUMPED: usize = 5;
    pub const CTX_BAR: usize = 6;
    pub const FEEDBACK: usize = 7;
}

/// `test.follower`: copies inputs that follow the transport to its outputs,
/// and reports what it sees in its `ProcessContext`.
pub static FOLLOWER: ModuleDescriptor = ModuleDescriptor {
    id: "test.follower",
    version: 1,
    name: "Follower",
    category: "Test",
    description: "Copies transport defaults to outputs.",
    inputs: &[
        port("bar", SignalType::Phase).with_default(PortDefault::BarPhase),
        port("beat", SignalType::Phase).with_default(PortDefault::BeatPhase),
        port("play", SignalType::Gate).with_default(PortDefault::Playing),
        port("value", SignalType::Modulation).with_default(PortDefault::Value(0.25)),
        port("feedback", SignalType::Modulation),
    ],
    outputs: &[
        port("bar", SignalType::Phase),
        port("beat", SignalType::Phase),
        port("play", SignalType::Gate),
        port("value", SignalType::Modulation),
        port("connected", SignalType::Gate),
        port("jumped", SignalType::Gate),
        port("ctx_bar", SignalType::Phase),
        port("feedback", SignalType::Modulation),
    ],
    params: &[],
    role: ModuleRole::Dsp(|_| Box::new(Follower)),
    panel: loom_core::Panel::EMPTY,
    displays: &[],
};

struct Follower;

impl Module for Follower {
    fn process(&mut self, ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let connected = io.is_connected(0);
        let transport = *ctx.transport();
        let ([bar, beat, play, value, feedback], outputs) = io.split::<5, 8>();
        let [
            o_bar,
            o_beat,
            o_play,
            o_value,
            o_connected,
            o_jumped,
            o_ctx_bar,
            o_feedback,
        ] = outputs;
        o_bar.copy_from_slice(bar);
        o_beat.copy_from_slice(beat);
        o_play.copy_from_slice(play);
        o_value.copy_from_slice(value);
        o_feedback.copy_from_slice(feedback);
        o_connected.fill(if connected { 1.0 } else { 0.0 });
        o_jumped.fill(0.0);
        if transport.jumped() {
            o_jumped[0] = 1.0;
        }
        for (i, x) in o_ctx_bar.iter_mut().enumerate() {
            *x = transport.bar_phase_at(i);
        }
    }
}

/// `test.bar`: the smallest follower, for long runs.
pub static BAR_COPY: ModuleDescriptor = ModuleDescriptor {
    id: "test.bar",
    version: 1,
    name: "Bar",
    category: "Test",
    description: "Copies the bar phase.",
    inputs: &[port("bar", SignalType::Phase).with_default(PortDefault::BarPhase)],
    outputs: &[port("bar", SignalType::Phase)],
    params: &[],
    role: ModuleRole::Dsp(|_| Box::new(Bar)),
    panel: loom_core::Panel::EMPTY,
    displays: &[],
};

struct Bar;

impl Module for Bar {
    fn process(&mut self, _: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let ([bar], [out]) = io.split::<1, 1>();
        out.copy_from_slice(bar);
    }
}

pub fn registry() -> Arc<Registry> {
    let mut registry = Registry::new();
    registry.register(&FOLLOWER).unwrap();
    registry.register(&BAR_COPY).unwrap();
    Arc::new(registry)
}

/// An engine with the test registry. `max_block` is also the block size.
pub fn engine(max_block: usize) -> EngineParts {
    let config = EngineConfig {
        max_block,
        // Room for every probed chunk of a 1024-frame buffer.
        scope_capacity: 4096,
        ..EngineConfig::with_sample_rate(SAMPLE_RATE)
    };
    create_engine(registry(), config)
}

/// Probes outputs and collects what they send. One capture drains the scope
/// queue, so use one per engine.
pub struct Capture {
    probes: Vec<ProbeId>,
    samples: BTreeMap<ProbeId, Vec<f32>>,
}

impl Capture {
    /// Probes each `(module, output)`. Takes effect at the next commit.
    pub fn new(parts: &mut EngineParts, outputs: &[(ModuleId, usize)]) -> Self {
        let probes: Vec<ProbeId> = outputs
            .iter()
            .map(|&(module, port)| {
                parts
                    .controller
                    .add_probe(loom_core::OutputRef { module, port }, ProbeKind::Scope)
                    .unwrap()
            })
            .collect();
        Self {
            samples: probes.iter().map(|&p| (p, Vec::new())).collect(),
            probes,
        }
    }

    /// Reads all queued scope chunks into the right probe's samples.
    pub fn drain(&mut self, parts: &mut EngineParts) {
        for chunk in parts.telemetry.scope() {
            if let Some(samples) = self.samples.get_mut(&chunk.probe) {
                samples.extend_from_slice(&chunk.samples[..chunk.len as usize]);
            }
        }
    }

    /// The samples of the `index`-th probed output.
    pub fn get(&self, index: usize) -> &[f32] {
        &self.samples[&self.probes[index]]
    }
}

/// Renders `frames` frames in buffers of `block` frames. `between` runs on
/// the "control thread" before each buffer, with the first frame's index.
pub fn render(
    parts: &mut EngineParts,
    capture: &mut Capture,
    frames: usize,
    block: usize,
    mut between: impl FnMut(usize, &mut EngineParts),
) {
    let mut left = vec![0.0; block];
    let mut right = vec![0.0; block];
    let mut position = 0;
    while position < frames {
        let n = block.min(frames - position);
        between(position, parts);
        parts.engine.process(&mut left[..n], &mut right[..n]);
        capture.drain(parts);
        position += n;
    }
}
