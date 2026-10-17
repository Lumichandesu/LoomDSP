//! Engine acceptance tests for parameter modulation.

use loom_core::{
    Controller, EngineConfig, EngineParts, Module, ModuleConfig, ModuleDescriptor, ModuleId, ModuleRole, Panel,
    ParamOptions, ParamSpec, Polarity, PortDefault, PortIo, PortSpec, ProbeKind, ProcessContext, SignalType,
    create_engine,
};
use loom_modules::register_all;
use std::sync::Arc;

struct Mirror {
    value: f32,
}

impl Module for Mirror {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        let samples = io.modulated(0);
        let ([], [out]) = io.split::<0, 1>();
        for (i, sample) in out.iter_mut().enumerate() {
            *sample = samples.map(|values| values[i]).unwrap_or(self.value);
        }
    }

    fn set_param(&mut self, _index: usize, value: f32) {
        self.value = value;
    }
}

fn mirror(_config: &ModuleConfig) -> Box<dyn Module> {
    Box::new(Mirror { value: 0.0 })
}

const fn test_param(name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> ParamSpec {
    ParamSpec {
        name,
        label: name,
        description: "A test parameter.",
        min,
        max,
        default,
        unit,
        options: ParamOptions::DEFAULT,
    }
}

macro_rules! mirror_module {
    ($name:ident, $id:literal, $param:expr) => {
        static $name: ModuleDescriptor = ModuleDescriptor {
            id: $id,
            version: 1,
            name: $id,
            category: "Test",
            description: "Writes its parameter, or the modulated buffer.",
            inputs: &[],
            outputs: &[PortSpec {
                name: "out",
                label: "Out",
                signal: SignalType::Modulation,
                description: "The parameter, or its modulated value.",
                default: PortDefault::Value(0.0),
                polarity: Polarity::Bipolar,
            }],
            params: &[$param],
            role: ModuleRole::Dsp(mirror),
            panel: Panel::EMPTY,
            displays: &[],
        };
    };
}

mirror_module!(LINEAR, "test.linear", test_param("level", 0.0, 1.0, 0.0, ""));
mirror_module!(
    LOG,
    "test.log",
    test_param("cutoff", 20.0, 20_000.0, 1_000.0, "Hz").logarithmic()
);
mirror_module!(STEPS, "test.steps", test_param("steps", 0.0, 10.0, 0.0, "").integer());

fn engine() -> EngineParts {
    let mut registry = loom_core::Registry::new();
    register_all(&mut registry).expect("built-ins");
    registry.register(&LINEAR).expect("linear");
    registry.register(&LOG).expect("log");
    registry.register(&STEPS).expect("steps");
    create_engine(Arc::new(registry), EngineConfig::with_sample_rate(48_000.0))
}

fn render(parts: &mut EngineParts, frames: usize, block: usize) -> Vec<f32> {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let mut position = 0;
    while position < frames {
        let n = block.min(frames - position);
        parts
            .engine
            .process(&mut left[position..position + n], &mut right[position..position + n]);
        position += n;
    }
    left
}

/// Reads a module output through a scope probe, so values outside -1 to 1
/// are not clipped the way the final audio output is.
fn capture(parts: &mut EngineParts, module: ModuleId, frames: usize, block: usize) -> Vec<f32> {
    let output = parts.controller.output_ref(module, "out").unwrap();
    parts.controller.add_probe(output, ProbeKind::Scope).unwrap();
    parts.controller.commit().unwrap();
    let mut left = vec![0.0; block];
    let mut right = vec![0.0; block];
    let mut done = 0;
    while done < frames {
        let n = block.min(frames - done);
        parts.engine.process(&mut left[..n], &mut right[..n]);
        done += n;
    }
    let mut got = Vec::with_capacity(frames);
    for chunk in parts.telemetry.scope() {
        got.extend_from_slice(chunk.samples());
    }
    got
}

fn listen(controller: &mut Controller, module: ModuleId, port: &str) {
    let audio = controller.add_module("loom.io.audio-out").unwrap();
    controller.connect_named(module, port, audio, "left").unwrap();
}

fn module(controller: &mut Controller, kind: &str, param: &str, value: f32) -> ModuleId {
    let id = controller.add_module(kind).unwrap();
    controller.set_param_named(id, param, value).unwrap();
    id
}

#[test]
fn constant_source_lands_on_the_midpoint() {
    let mut parts = engine();
    let source = module(&mut parts.controller, "test.linear", "level", 1.0);
    let sink = parts.controller.add_module("test.linear").unwrap();
    parts
        .controller
        .add_assignment_named(source, "out", sink, "level", 0.5)
        .unwrap();
    listen(&mut parts.controller, sink, "out");
    parts.controller.commit().unwrap();

    let left = render(&mut parts, 32, 32);
    assert!(left.iter().all(|&sample| sample == 0.5), "{left:?}");
}

#[test]
fn cutoff_depth_moves_a_tenth_of_the_knob() {
    let spec = test_param("cutoff", 20.0, 20_000.0, 1_000.0, "Hz").logarithmic();
    let expected = spec.denormalize(spec.normalize(1_000.0) + 0.1);

    let mut parts = engine();
    let source = module(&mut parts.controller, "test.linear", "level", 1.0);
    let sink = parts.controller.add_module("test.log").unwrap();
    parts
        .controller
        .add_assignment_named(source, "out", sink, "cutoff", 0.1)
        .unwrap();
    let left = capture(&mut parts, sink, 8, 8);
    assert!(left.iter().all(|&sample| sample == expected), "{left:?} vs {expected}");
}

#[test]
fn assignments_sum_then_clamp_and_integers_stay_whole() {
    let mut parts = engine();
    let a = module(&mut parts.controller, "test.linear", "level", 1.0);
    let b = module(&mut parts.controller, "test.linear", "level", 1.0);
    let sink = parts.controller.add_module("test.linear").unwrap();
    parts
        .controller
        .add_assignment_named(a, "out", sink, "level", 0.25)
        .unwrap();
    parts
        .controller
        .add_assignment_named(b, "out", sink, "level", 0.25)
        .unwrap();
    listen(&mut parts.controller, sink, "out");
    parts.controller.commit().unwrap();
    assert!(render(&mut parts, 4, 4).iter().all(|&sample| sample == 0.5));

    let mut parts = engine();
    let a = module(&mut parts.controller, "test.linear", "level", 1.0);
    let b = module(&mut parts.controller, "test.linear", "level", 1.0);
    let sink = parts.controller.add_module("test.linear").unwrap();
    parts
        .controller
        .add_assignment_named(a, "out", sink, "level", 0.75)
        .unwrap();
    parts
        .controller
        .add_assignment_named(b, "out", sink, "level", 0.75)
        .unwrap();
    listen(&mut parts.controller, sink, "out");
    parts.controller.commit().unwrap();
    assert!(render(&mut parts, 4, 4).iter().all(|&sample| sample == 1.0));

    let spec = test_param("steps", 0.0, 10.0, 0.0, "").integer();
    let expected = spec.denormalize(0.125);
    assert_eq!(expected, 1.0);

    let mut parts = engine();
    let source = module(&mut parts.controller, "test.linear", "level", 1.0);
    let sink = parts.controller.add_module("test.steps").unwrap();
    parts
        .controller
        .add_assignment_named(source, "out", sink, "steps", 0.125)
        .unwrap();
    listen(&mut parts.controller, sink, "out");
    parts.controller.commit().unwrap();
    assert!(render(&mut parts, 4, 4).iter().all(|&sample| sample == expected));
}

#[test]
fn depth_zero_matches_no_assignment() {
    fn run(assign: bool) -> Vec<f32> {
        let mut parts = engine();
        let source = parts.controller.add_module("loom.phase.generator").unwrap();
        let sink = parts.controller.add_module("test.log").unwrap();
        if assign {
            parts
                .controller
                .add_assignment_named(source, "phase", sink, "cutoff", 0.0)
                .unwrap();
        }
        capture(&mut parts, sink, 256, 64)
    }

    let with = run(true);
    let without = run(false);
    assert_eq!(with, without);
    assert!(with.iter().all(|&sample| sample == 1_000.0));
}

#[test]
fn modulated_output_is_independent_of_block_size() {
    fn run(block: usize) -> Vec<f32> {
        let mut parts = engine();
        let source = parts.controller.add_module("loom.phase.generator").unwrap();
        let sink = parts.controller.add_module("test.linear").unwrap();
        parts
            .controller
            .add_assignment_named(source, "phase", sink, "level", 0.2)
            .unwrap();
        listen(&mut parts.controller, sink, "out");
        parts.controller.commit().unwrap();
        render(&mut parts, 512, block)
    }

    let small = run(1);
    let large = run(128);
    assert_eq!(small, large);
    assert_ne!(small[0], small[200]);
}

#[test]
fn assignment_loop_delays_the_assignment_and_plays() {
    let mut parts = engine();
    let source = module(&mut parts.controller, "test.linear", "level", 0.5);
    let filter = parts.controller.add_module("loom.math.add").unwrap();
    parts.controller.connect_named(source, "out", filter, "a").unwrap();
    parts
        .controller
        .add_assignment_named(filter, "out", source, "level", 0.5)
        .unwrap();
    listen(&mut parts.controller, filter, "out");
    let report = parts.controller.commit().unwrap();
    assert_eq!(report.inserted_assignment_delays().count(), 1);
    assert_eq!(report.inserted_delays().count(), 0);

    let left = render(&mut parts, 8, 8);
    assert_eq!(left[0], 0.5);
    assert_eq!(left[1], 0.75);
    assert_eq!(left[2], 0.875);
    assert!(left.iter().all(|sample| sample.is_finite()));
}

#[test]
fn self_assignment_is_a_one_sample_loop() {
    let mut parts = engine();
    let sink = module(&mut parts.controller, "test.linear", "level", 0.5);
    parts
        .controller
        .add_assignment_named(sink, "out", sink, "level", 1.0)
        .unwrap();
    listen(&mut parts.controller, sink, "out");
    let report = parts.controller.commit().unwrap();
    assert_eq!(report.inserted_assignment_delays().count(), 1);
    assert_eq!(report.inserted_delays().count(), 0);

    let left = render(&mut parts, 4, 4);
    assert_eq!(&left[..3], &[0.5, 1.0, 1.0]);
}

#[test]
fn polarity_override_remaps_a_bipolar_source() {
    fn run(polarity: Option<Polarity>) -> f32 {
        let mut parts = engine();
        let source = parts.controller.add_module("test.linear").unwrap();
        let sink = parts.controller.add_module("test.linear").unwrap();
        let id = parts
            .controller
            .add_assignment_named(source, "out", sink, "level", 1.0)
            .unwrap();
        if let Some(polarity) = polarity {
            parts.controller.set_assignment_polarity(id, Some(polarity)).unwrap();
        }
        listen(&mut parts.controller, sink, "out");
        parts.controller.commit().unwrap();
        render(&mut parts, 1, 1)[0]
    }

    // Source rests at 0, which is the middle of a bipolar output.
    assert_eq!(run(None), 0.0);
    assert_eq!(run(Some(Polarity::Unipolar)), 0.5);
}

#[test]
fn depth_and_enabled_apply_at_the_next_buffer() {
    let mut parts = engine();
    let source = module(&mut parts.controller, "test.linear", "level", 1.0);
    let sink = parts.controller.add_module("test.linear").unwrap();
    let id = parts
        .controller
        .add_assignment_named(source, "out", sink, "level", 0.5)
        .unwrap();
    listen(&mut parts.controller, sink, "out");
    parts.controller.commit().unwrap();
    assert!(render(&mut parts, 4, 4).iter().all(|&sample| sample == 0.5));

    parts.controller.set_assignment_depth(id, 0.25).unwrap();
    assert!(render(&mut parts, 4, 4).iter().all(|&sample| sample == 0.25));

    parts.controller.set_assignment_enabled(id, false).unwrap();
    assert!(render(&mut parts, 4, 4).iter().all(|&sample| sample == 0.0));
}

#[test]
fn deleting_a_module_removes_its_assignments() {
    let mut parts = engine();
    let source = parts.controller.add_module("test.linear").unwrap();
    let sink = parts.controller.add_module("test.linear").unwrap();
    parts
        .controller
        .add_assignment_named(source, "out", sink, "level", 0.5)
        .unwrap();
    parts.controller.remove_module(source).unwrap();
    assert!(parts.controller.graph().assignments().next().is_none());

    let source = parts.controller.add_module("test.linear").unwrap();
    parts
        .controller
        .add_assignment_named(source, "out", sink, "level", 0.5)
        .unwrap();
    parts.controller.remove_module(sink).unwrap();
    assert!(parts.controller.graph().assignments().next().is_none());
}
