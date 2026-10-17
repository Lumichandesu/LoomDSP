//! Module display values (ADR-0013, `docs/design/panels.md`, Display values):
//! they reach the controller after the block that wrote them, a reused slot
//! never shows the old module's values, and nothing allocates on the audio
//! thread.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use loom_core::{
    EngineConfig, EngineParts, Module, ModuleDescriptor, ModuleRole, Panel, ParamOptions, ParamSpec, Polarity,
    PortDefault, PortIo, PortSpec, ProcessContext, Registry, SignalType, Widget, create_engine,
};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// `test.lamp`: outputs its level, and publishes it and its call count.
static LAMP: ModuleDescriptor = ModuleDescriptor {
    id: "test.lamp",
    version: 1,
    name: "Lamp",
    category: "Test",
    description: "Publishes display values.",
    inputs: &[],
    outputs: &[PortSpec {
        name: "out",
        label: "Out",
        signal: SignalType::Modulation,
        description: "The level.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Bipolar,
    }],
    params: &[ParamSpec {
        name: "level",
        label: "Level",
        description: "The level to output and publish.",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        unit: "",
        options: ParamOptions::DEFAULT,
    }],
    role: ModuleRole::Dsp(|_| Box::new(Lamp { level: 0.0, calls: 0 })),
    panel: Panel::new(3, 2, &[Widget::light(0, 0, "level")]),
    displays: &["level", "calls"],
};

struct Lamp {
    level: f32,
    calls: u32,
}

impl Module for Lamp {
    fn process(&mut self, _: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        io.output(0).fill(self.level);
        self.calls += 1;
    }

    fn set_param(&mut self, _: usize, value: f32) {
        self.level = value;
    }

    fn displays(&self, values: &mut [f32]) {
        values[0] = self.level;
        values[1] = self.calls as f32;
    }
}

fn engine(max_modules: usize) -> EngineParts {
    let mut registry = Registry::new();
    registry.register(&LAMP).unwrap();
    let config = EngineConfig {
        max_modules,
        max_block: 64,
        ..EngineConfig::with_sample_rate(48_000.0)
    };
    create_engine(Arc::new(registry), config)
}

fn process(parts: &mut EngineParts, frames: usize) -> Vec<f32> {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    parts.engine.process(&mut left, &mut right);
    left
}

#[test]
fn values_reach_the_controller_after_the_block() {
    let mut parts = engine(4);
    let lamp = parts.controller.add_module("test.lamp").unwrap();
    parts.controller.set_param_named(lamp, "level", 0.75).unwrap();
    parts.controller.commit().unwrap();
    // Not run yet: 0.
    assert_eq!(parts.controller.display(lamp, 0), Some(0.0));

    // 128 frames in blocks of 64: two calls.
    process(&mut parts, 128);
    assert_eq!(parts.controller.display(lamp, 0), Some(0.75));
    assert_eq!(parts.controller.display(lamp, 1), Some(2.0));

    // No such value, no such module.
    assert_eq!(parts.controller.display(lamp, 2), None);
    let missing = loom_core::ModuleId(99);
    assert_eq!(parts.controller.display(missing, 0), None);
}

#[test]
fn a_reused_slot_starts_at_zero() {
    // One slot, so the second module must reuse the first one's.
    let mut parts = engine(1);
    let first = parts.controller.add_module("test.lamp").unwrap();
    parts.controller.set_param_named(first, "level", 1.0).unwrap();
    parts.controller.commit().unwrap();
    process(&mut parts, 64);
    assert_eq!(parts.controller.display(first, 0), Some(1.0));

    parts.controller.remove_module(first).unwrap();
    parts.controller.commit().unwrap();
    let second = parts.controller.add_module("test.lamp").unwrap();
    parts.controller.commit().unwrap();
    // The engine still holds the first module's values, but they are not
    // the second module's.
    assert_eq!(parts.controller.display(second, 0), Some(0.0));
    assert_eq!(parts.controller.display(second, 1), Some(0.0));

    process(&mut parts, 64);
    assert_eq!(parts.controller.display(second, 0), Some(0.5));
    assert_eq!(parts.controller.display(second, 1), Some(1.0));
}

#[test]
fn reading_values_does_not_change_the_audio() {
    let render = |read: bool| {
        let mut parts = engine(4);
        let lamp = parts.controller.add_module("test.lamp").unwrap();
        let out = parts.controller.add_module("test.lamp").unwrap();
        parts.controller.set_param_named(out, "level", 0.25).unwrap();
        parts.controller.commit().unwrap();
        let mut audio = Vec::new();
        for _ in 0..10 {
            audio.extend(process(&mut parts, 100));
            if read {
                let _ = parts.controller.display(lamp, 0);
                let _ = parts.controller.display(out, 1);
            }
        }
        audio
    };
    assert_eq!(render(true), render(false));
}

#[test]
fn display_values_never_allocate_on_the_audio_thread() {
    let mut parts = engine(64);
    let mut left = vec![0.0; 1024];
    let mut right = vec![0.0; 1024];
    let mut lamps = Vec::new();
    for i in 0..200usize {
        // Control thread: allocation allowed here.
        if i % 3 == 0 {
            lamps.push(parts.controller.add_module("test.lamp").unwrap());
        }
        if i % 5 == 0
            && lamps.len() > 2
            && let Some(id) = lamps.pop()
        {
            parts.controller.remove_module(id).unwrap();
        }
        parts.controller.commit().unwrap();
        for &lamp in &lamps {
            let _ = parts.controller.display(lamp, 0);
        }
        let frames = [1, 63, 64, 65, 1024][i % 5];

        // Audio thread: no allocation allowed.
        assert_no_alloc(|| {
            parts.engine.process(&mut left[..frames], &mut right[..frames]);
        });
    }
}
