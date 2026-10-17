//! Engine acceptance tests for parameter modulation.

use loom_core::{
    AssignmentId, Controller, EngineConfig, EngineParts, MidiEvent, Module, ModuleConfig, ModuleDescriptor, ModuleId,
    ModuleRole, Panel, ParamOptions, ParamSpec, Polarity, PortDefault, PortIo, PortSpec, ProbeKind, ProcessContext,
    SignalType, create_engine,
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

/// How a test takes the modulation away again.
#[derive(Clone, Copy, Debug)]
enum Removal {
    Assignment,
    SourceModule,
}

const REMOVALS: [Removal; 2] = [Removal::Assignment, Removal::SourceModule];

/// A sink module with base values, plus a constant source at 1.0 for its
/// assignments. Sending no assignment builds the unmodulated reference.
struct Rig {
    parts: EngineParts,
    sink: ModuleId,
    source: ModuleId,
    assignments: Vec<AssignmentId>,
}

impl Rig {
    fn new(kind: &str, base: &[(&str, f32)]) -> Self {
        let mut parts = engine();
        let sink = parts.controller.add_module(kind).unwrap();
        for &(param, value) in base {
            parts.controller.set_param_named(sink, param, value).unwrap();
        }
        let source = module(&mut parts.controller, "test.linear", "level", 1.0);
        Self {
            parts,
            sink,
            source,
            assignments: Vec::new(),
        }
    }

    fn assign(&mut self, param: &str, depth: f32) {
        let id = self
            .parts
            .controller
            .add_assignment_named(self.source, "out", self.sink, param, depth)
            .unwrap();
        self.assignments.push(id);
    }

    /// Plays `left` and `right` of the sink to the audio output.
    fn listen(&mut self, left: &str, right: &str) {
        let audio = self.parts.controller.add_module("loom.io.audio-out").unwrap();
        let controller = &mut self.parts.controller;
        controller.connect_named(self.sink, left, audio, "left").unwrap();
        controller.connect_named(self.sink, right, audio, "right").unwrap();
        controller.commit().unwrap();
    }

    fn remove(&mut self, how: Removal) {
        match how {
            Removal::Assignment => {
                for id in self.assignments.drain(..) {
                    self.parts.controller.remove_assignment(id).unwrap();
                }
            }
            Removal::SourceModule => {
                self.parts.controller.remove_module(self.source).unwrap();
                self.assignments.clear();
            }
        }
        self.parts.controller.commit().unwrap();
    }

    fn render(&mut self, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut left = vec![0.0; frames];
        let mut right = vec![0.0; frames];
        for (l, r) in left.chunks_mut(64).zip(right.chunks_mut(64)) {
            self.parts.engine.process(l, r);
        }
        (left, right)
    }
}

/// Regression for a module that wrote modulated samples into the fields
/// `set_param` owns: after the assignment went away it kept the last
/// modulated value, because nothing calls `set_param` again.
#[test]
fn adsr_returns_to_base_sustain() {
    for how in REMOVALS {
        let mut rig = Rig::new(
            "loom.env.adsr",
            &[("attack", 0.0005), ("decay", 0.0005), ("sustain", 0.5)],
        );
        let gate = module(&mut rig.parts.controller, "test.linear", "level", 1.0);
        rig.parts
            .controller
            .connect_named(gate, "out", rig.sink, "gate")
            .unwrap();
        rig.assign("sustain", 0.5);
        rig.listen("out", "out");

        let (modulated, _) = rig.render(4_800);
        let last = modulated[modulated.len() - 1];
        assert!((last - 1.0).abs() < 1e-3, "{how:?}: modulated sustain, got {last}");

        rig.remove(how);
        let (base, _) = rig.render(4_800);
        let last = base[base.len() - 1];
        assert!((last - 0.5).abs() < 1e-3, "{how:?}: base sustain, got {last}");
    }
}

#[test]
fn divide_returns_to_base_n_and_offset() {
    fn run(how: Option<Removal>) -> (Vec<f32>, Vec<f32>) {
        let mut rig = Rig::new("loom.logic.divide", &[("n", 2.0), ("offset", 0.0)]);
        let controller = &mut rig.parts.controller;
        let phase = controller.add_module("loom.phase.generator").unwrap();
        let triggers = controller.add_module("loom.phase.trigger").unwrap();
        controller.connect_named(phase, "phase", triggers, "phase").unwrap();
        controller
            .connect_named(triggers, "trigger", rig.sink, "trigger")
            .unwrap();
        if how.is_some() {
            // n goes to 4 and offset to 1.
            rig.assign("n", 2.0 / 63.0);
            rig.assign("offset", 1.0 / 63.0);
        }
        rig.listen("trigger", "trigger");
        let before = rig.render(9_600).0;
        if let Some(how) = how {
            rig.remove(how);
        }
        (before, rig.render(9_600).0)
    }

    let (reference_before, reference) = run(None);
    assert!(reference.contains(&1.0), "the reference fires");
    for how in REMOVALS {
        let (before, after) = run(Some(how));
        assert_ne!(before, reference_before, "{how:?}: modulation changes the output");
        assert_eq!(after, reference, "{how:?}");
    }
}

#[test]
fn note_in_returns_to_base_channel_and_bend_range() {
    fn run(how: Option<Removal>) -> (Vec<f32>, Vec<f32>) {
        let mut rig = Rig::new("loom.midi.note", &[("channel", 0.0), ("bend-range", 2.0)]);
        if how.is_some() {
            // The channel goes to 2, which ignores channel 1 below, and the
            // bend range goes to 12 semitones.
            rig.assign("channel", 2.0 / 16.0);
            rig.assign("bend-range", 10.0 / 48.0);
        }
        rig.listen("pitch", "gate");
        rig.render(256);
        if let Some(how) = how {
            rig.remove(how);
        }
        // Channel 1 (0 on the wire): a note, then a bend halfway up.
        assert!(rig.parts.midi.send(MidiEvent::note_on(10, 0, 67, 100)));
        assert!(rig.parts.midi.send(MidiEvent::from_bytes(20, &[0xE0, 0x00, 0x60])));
        rig.render(256)
    }

    let reference = run(None);
    assert!(reference.1.contains(&1.0), "the reference plays the note");
    for how in REMOVALS {
        assert_eq!(run(Some(how)), reference, "{how:?}");
    }
}

#[test]
fn noise_returns_to_base_seed() {
    fn run(how: Option<Removal>) -> (Vec<f32>, Vec<f32>) {
        let mut rig = Rig::new("loom.osc.noise", &[("seed", 1.0)]);
        if how.is_some() {
            // The seed goes to 101.
            rig.assign("seed", 100.0 / 65_534.0);
        }
        rig.listen("out", "out");
        let before = rig.render(256).0;
        if let Some(how) = how {
            rig.remove(how);
        }
        (before, rig.render(256).0)
    }

    // Back on the base seed, the pattern starts again from that seed.
    let (reference, _) = run(None);
    for how in REMOVALS {
        let (before, after) = run(Some(how));
        assert_ne!(before, reference, "{how:?}: modulation changes the seed");
        assert_eq!(after, reference, "{how:?}");
    }
}

/// A cycle of cables needs a cable delay whatever the compiler does. Once
/// that delay is in, the assignment below is no longer in a loop, so it must
/// not be delayed too (review on #133).
#[test]
fn cable_cycle_is_broken_before_an_assignment_that_needs_no_delay() {
    let mut parts = engine();
    let controller = &mut parts.controller;
    let first = controller.add_module("loom.math.add").unwrap();
    let second = controller.add_module("loom.math.add").unwrap();
    controller.connect_named(first, "out", second, "a").unwrap();
    controller.connect_named(second, "out", first, "a").unwrap();
    controller
        .add_assignment_named(first, "out", second, "offset", 0.5)
        .unwrap();
    listen(controller, second, "out");
    let report = controller.commit().unwrap();
    let delays: Vec<_> = report.inserted_delays().collect();
    assert_eq!(delays.len(), 1, "{delays:?}");
    assert_eq!(delays[0].1.module, first, "the cable into the first add is delayed");
    assert_eq!(report.inserted_assignment_delays().count(), 0);
}

/// Turns a knob, then adds its first assignment halfway through the glide.
/// The depth is 0, so the output must follow the same glide as with no
/// assignment, instead of stepping to the new value (review on #133).
fn glide_with_depth_zero(
    kind: &str,
    param: &str,
    from: f32,
    to: f32,
    output: &str,
    setup: impl Fn(&mut Rig),
    assign: bool,
) -> Vec<f32> {
    let mut rig = Rig::new(kind, &[(param, from)]);
    setup(&mut rig);
    rig.listen(output, output);
    rig.render(256);
    rig.parts.controller.set_param_named(rig.sink, param, to).unwrap();
    let mut out = rig.render(64).0;
    if assign {
        rig.assign(param, 0.0);
    }
    rig.parts.controller.commit().unwrap();
    out.extend(rig.render(1_024).0);
    out
}

fn assert_close(got: &[f32], expected: &[f32], tolerance: f32) {
    assert_eq!(got.len(), expected.len());
    for (frame, (g, e)) in got.iter().zip(expected).enumerate() {
        assert!((g - e).abs() <= tolerance, "frame {frame}: {g} vs {e}");
    }
}

#[test]
fn depth_zero_added_during_a_glide_continues_the_glide() {
    let run = |assign| glide_with_depth_zero("loom.math.constant", "value", 0.0, 1.0, "out", |_| {}, assign);
    let without = run(false);
    let with = run(true);
    // Still gliding when the assignment arrives, so a step would show.
    assert!(without[64] < 0.9, "{}", without[64]);
    assert_close(&with, &without, 1e-5);

    let noise = |rig: &mut Rig| {
        let noise = rig.parts.controller.add_module("loom.osc.noise").unwrap();
        rig.parts
            .controller
            .connect_named(noise, "out", rig.sink, "in")
            .unwrap();
    };
    let run = |assign| glide_with_depth_zero("loom.filter.svf", "cutoff", 200.0, 8_000.0, "lowpass", noise, assign);
    assert_close(&run(true), &run(false), 1e-4);
}
