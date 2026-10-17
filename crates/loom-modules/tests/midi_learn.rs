//! Engine acceptance tests for MIDI learn. Spec: `docs/design/midi-learn.md`.

use loom_core::{
    ControlChangeSeen, EngineConfig, EngineParts, MidiControl, MidiEvent, MidiMessage, Module, ModuleConfig,
    ModuleDescriptor, ModuleId, ModuleRole, Panel, ParamOptions, ParamSpec, Polarity, PortDefault, PortIo, PortSpec,
    ProbeKind, ProcessContext, SignalType, control_value, create_engine,
};
use loom_modules::register_all;
use std::sync::Arc;

/// Writes its first parameter on `out`, and the number of control changes it
/// has seen on `ccs`.
struct Probe {
    values: [f32; 2],
    ccs: f32,
}

impl Module for Probe {
    fn process(&mut self, ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        for (_, event) in ctx.midi() {
            if matches!(event.message(), MidiMessage::ControlChange { .. }) {
                self.ccs += 1.0;
            }
        }
        let ([], [out, ccs]) = io.split::<0, 2>();
        out.fill(self.values[0]);
        ccs.fill(self.ccs);
    }

    fn set_param(&mut self, index: usize, value: f32) {
        self.values[index] = value;
    }
}

fn probe(_config: &ModuleConfig) -> Box<dyn Module> {
    Box::new(Probe {
        values: [0.0; 2],
        ccs: 0.0,
    })
}

const fn output(name: &'static str) -> PortSpec {
    PortSpec {
        name,
        label: name,
        signal: SignalType::Modulation,
        description: "A test output.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Unipolar,
    }
}

static PROBE: ModuleDescriptor = ModuleDescriptor {
    id: "test.probe",
    version: 1,
    name: "test.probe",
    category: "Test",
    description: "Writes its parameter and counts control changes.",
    inputs: &[],
    outputs: &[output("out"), output("ccs")],
    params: &[
        ParamSpec {
            name: "level",
            label: "Level",
            description: "A test parameter.",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            unit: "",
            options: ParamOptions::DEFAULT,
        },
        ParamSpec {
            name: "other",
            label: "Other",
            description: "Another test parameter.",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            unit: "",
            options: ParamOptions::DEFAULT,
        },
    ],
    role: ModuleRole::Dsp(probe),
    panel: Panel::EMPTY,
    displays: &[],
};

const CC74: MidiControl = MidiControl {
    channel: 0,
    controller: 74,
};

const CC71: MidiControl = MidiControl {
    channel: 0,
    controller: 71,
};

fn engine() -> EngineParts {
    let mut registry = loom_core::Registry::new();
    register_all(&mut registry).expect("built-ins");
    registry.register(&PROBE).expect("probe");
    create_engine(Arc::new(registry), EngineConfig::with_sample_rate(48_000.0))
}

/// Adds a probe module with scope probes on both outputs.
fn add_probe(parts: &mut EngineParts) -> ModuleId {
    let id = parts.controller.add_module("test.probe").unwrap();
    for port in ["out", "ccs"] {
        let output = parts.controller.output_ref(id, port).unwrap();
        parts.controller.add_probe(output, ProbeKind::Scope).unwrap();
    }
    parts.controller.commit().unwrap();
    id
}

fn send_cc(parts: &mut EngineParts, control: MidiControl, value: u8) {
    assert!(
        parts
            .midi
            .send(MidiEvent::control_change(0, control.channel, control.controller, value))
    );
}

/// Runs one buffer and returns the last `out` and `ccs` values.
fn step(parts: &mut EngineParts) -> (f32, f32) {
    let mut left = [0.0; 64];
    let mut right = [0.0; 64];
    parts.engine.process(&mut left, &mut right);
    let chunks: Vec<_> = parts.telemetry.scope().collect();
    let last = |probe: usize| {
        chunks
            .iter()
            .filter(|chunk| chunk.probe.0 as usize == probe)
            .flat_map(|chunk| chunk.samples().to_vec())
            .last()
            .unwrap()
    };
    // Probes are numbered from 1 in the order they were added.
    (last(1), last(2))
}

#[test]
fn a_mapped_control_change_sets_the_base_value() {
    let mut parts = engine();
    let id = add_probe(&mut parts);
    parts.controller.map_midi(CC74, id, 0).unwrap();
    parts.controller.commit().unwrap();

    send_cc(&mut parts, CC74, 127);
    assert_eq!(step(&mut parts).0, 1.0);
    send_cc(&mut parts, CC74, 0);
    assert_eq!(step(&mut parts).0, 0.0);
    send_cc(&mut parts, CC74, 64);
    let expected = 64.0 / 127.0;
    assert_eq!(step(&mut parts).0, expected);

    // The graph follows, so the UI shows and saves what plays.
    assert_eq!(parts.controller.sync_midi(), vec![(id, 0)]);
    assert_eq!(parts.controller.graph().node(id).unwrap().params[0], expected);
    assert!(parts.controller.sync_midi().is_empty(), "nothing new");
}

#[test]
fn other_channels_and_controllers_do_not_move_it() {
    let mut parts = engine();
    let id = add_probe(&mut parts);
    parts.controller.map_midi(CC74, id, 0).unwrap();
    parts.controller.commit().unwrap();

    send_cc(&mut parts, MidiControl { channel: 1, ..CC74 }, 127);
    send_cc(&mut parts, CC71, 127);
    assert_eq!(step(&mut parts).0, 0.0);
    assert!(parts.controller.sync_midi().is_empty());
}

#[test]
fn values_follow_the_parameter_mapping() {
    let registry = loom_modules::registry();
    let svf = registry.resolve("loom.filter.svf").unwrap();
    let cutoff = &svf.params[svf.param_index("cutoff").unwrap()];
    assert_eq!(control_value(cutoff, 0), cutoff.min);
    assert_eq!(control_value(cutoff, 127), cutoff.max);
    // The middle of a logarithmic range is the geometric mean, within one
    // step of 127.
    let middle = (cutoff.min * cutoff.max).sqrt();
    let below = control_value(cutoff, 63);
    let above = control_value(cutoff, 64);
    assert!(below < middle && middle < above, "{below} < {middle} < {above}");
}

#[test]
fn a_parameter_and_a_control_each_have_one_mapping() {
    let mut parts = engine();
    let id = add_probe(&mut parts);
    let controller = &mut parts.controller;

    // Learning a mapped parameter again replaces its mapping.
    controller.map_midi(CC74, id, 0).unwrap();
    let (_, removed) = controller.map_midi(CC71, id, 0).unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].control, CC74);
    assert_eq!(controller.graph().midi_mapping_of(id, 0).unwrap().1.control, CC71);

    // Learning a mapped control moves it, and names the parameter it left.
    let (_, removed) = controller.map_midi(CC71, id, 1).unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!((removed[0].target, removed[0].param), (id, 0));
    assert!(controller.graph().midi_mapping_of(id, 0).is_none());
    assert_eq!(controller.graph().midi_mappings().count(), 1);
    controller.commit().unwrap();

    send_cc(&mut parts, CC71, 127);
    assert_eq!(step(&mut parts).0, 0.0, "the old parameter no longer follows");
    assert_eq!(parts.controller.graph().node(id).unwrap().params[0], 0.0);
    assert_eq!(parts.controller.sync_midi(), vec![(id, 1)]);
}

#[test]
fn learning_uses_the_last_control_change_and_sets_the_value() {
    let mut parts = engine();
    let id = add_probe(&mut parts);
    assert_eq!(parts.controller.last_control_change(), None);

    send_cc(&mut parts, CC74, 100);
    step(&mut parts);
    let seen = parts.controller.last_control_change().unwrap();
    assert_eq!(seen.control, CC74);
    assert_eq!(seen.value, 100);

    parts.controller.learn_midi(seen, id, 0).unwrap();
    parts.controller.commit().unwrap();
    let expected = 100.0 / 127.0;
    assert_eq!(parts.controller.graph().node(id).unwrap().params[0], expected);
    assert_eq!(step(&mut parts).0, expected);

    // Each control change has a new serial, even with the same value.
    send_cc(&mut parts, CC74, 100);
    step(&mut parts);
    let again = parts.controller.last_control_change().unwrap();
    assert_eq!(
        again,
        ControlChangeSeen {
            serial: seen.serial + 1,
            ..seen
        }
    );
}

#[test]
fn the_same_value_after_a_screen_change_is_applied_again() {
    let mut parts = engine();
    let id = add_probe(&mut parts);
    parts.controller.map_midi(CC74, id, 0).unwrap();
    parts.controller.commit().unwrap();
    send_cc(&mut parts, CC74, 127);
    step(&mut parts);
    parts.controller.sync_midi();

    // The knob on screen moves, then the hardware sends the same value.
    parts.controller.set_param(id, 0, 0.25).unwrap();
    assert_eq!(step(&mut parts).0, 0.25);
    assert!(parts.controller.sync_midi().is_empty(), "the screen change stays");
    send_cc(&mut parts, CC74, 127);
    assert_eq!(step(&mut parts).0, 1.0);
    assert_eq!(parts.controller.sync_midi(), vec![(id, 0)]);
    assert_eq!(parts.controller.graph().node(id).unwrap().params[0], 1.0);
}

#[test]
fn mapped_control_changes_still_reach_modules() {
    let mut parts = engine();
    let id = add_probe(&mut parts);
    parts.controller.map_midi(CC74, id, 0).unwrap();
    parts.controller.commit().unwrap();
    send_cc(&mut parts, CC74, 10);
    send_cc(&mut parts, CC71, 10);
    assert_eq!(step(&mut parts).1, 2.0);
}

#[test]
fn removed_mappings_stop_working() {
    let mut parts = engine();
    let id = add_probe(&mut parts);
    parts.controller.map_midi(CC74, id, 0).unwrap();
    parts.controller.commit().unwrap();
    let removed = parts.controller.unmap_midi(id, 0).unwrap().unwrap();
    assert_eq!(removed.control, CC74);
    assert_eq!(parts.controller.unmap_midi(id, 0).unwrap(), None);
    parts.controller.commit().unwrap();
    send_cc(&mut parts, CC74, 127);
    assert_eq!(step(&mut parts).0, 0.0);

    // Deleting a module deletes its mappings.
    parts.controller.map_midi(CC74, id, 0).unwrap();
    parts.controller.remove_module(id).unwrap();
    assert_eq!(parts.controller.graph().midi_mappings().count(), 0);
}

#[test]
fn invalid_controls_are_refused() {
    let mut parts = engine();
    let id = add_probe(&mut parts);
    let bad = MidiControl {
        channel: 16,
        controller: 1,
    };
    assert!(parts.controller.map_midi(bad, id, 0).is_err());
    let bad = MidiControl {
        channel: 0,
        controller: 128,
    };
    assert!(parts.controller.map_midi(bad, id, 0).is_err());
    assert!(parts.controller.map_midi(CC74, id, 9).is_err());
}
