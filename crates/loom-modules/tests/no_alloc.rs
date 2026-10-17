//! NFR-RT-06: the audio path must never allocate or free.
//!
//! This test binary installs an allocator that aborts if any allocation or
//! deallocation happens inside `assert_no_alloc`. Every call to
//! `Engine::process` runs inside it, while the control side edits the graph
//! between buffers (where allocation is allowed). Every output is probed, so
//! telemetry runs too, both while the UI drains it and while its queues are
//! full (FR-VIS-04, NFR-RT-07).
//!
//! The check is active in debug builds, which is how `cargo test` runs.

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use loom_core::{MidiControl, MidiEvent, OutputRef, ProbeKind, TimeSignature};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

#[test]
fn audio_thread_never_allocates() {
    for patch in [
        "sine.ron",
        "subtractive.ron",
        "fm-feedback.ron",
        "fm-feedback-delay1.ron",
    ] {
        let mut parts = load(patch);
        let outputs: Vec<OutputRef> = parts
            .controller
            .graph()
            .nodes()
            .flat_map(|(module, node)| (0..node.descriptor.outputs.len()).map(move |port| OutputRef { module, port }))
            .collect();
        for output in outputs {
            parts.controller.add_probe(output, ProbeKind::MeterAndScope).unwrap();
        }
        let mut left = vec![0.0; 1024];
        let mut right = vec![0.0; 1024];
        let mut extra = Vec::new();
        for i in 0..400usize {
            // Control thread: allocation allowed here.
            if i % 7 == 0 {
                let id = parts.controller.add_module("loom.filter.svf").unwrap();
                parts.controller.connect_named(id, "lowpass", id, "in").unwrap();
                let lowpass = parts.controller.output_ref(id, "lowpass").unwrap();
                parts.controller.add_probe(lowpass, ProbeKind::Scope).unwrap();
                extra.push(id);
            }
            if i % 11 == 0
                && let Some(id) = extra.pop()
            {
                parts.controller.remove_module(id).unwrap();
            }
            // Assignments compile into the schedule and are filled on the
            // audio thread, so adding and removing them here must not make
            // `process` allocate.
            if i % 9 == 0
                && let Some(&id) = extra.last()
            {
                parts
                    .controller
                    .add_assignment_named(id, "lowpass", id, "cutoff", 0.2)
                    .unwrap();
            }
            // MIDI mappings compile into the schedule too, and control
            // changes set parameters on the audio thread.
            if i % 9 == 4
                && let Some(&id) = extra.last()
            {
                let control = MidiControl {
                    channel: 0,
                    controller: (i / 9 % 128) as u8,
                };
                parts.controller.map_midi_named(control, id, "cutoff").unwrap();
            }
            if i % 3 == 0 {
                parts
                    .midi
                    .send(MidiEvent::control_change(0, 0, (i / 9 % 128) as u8, (i % 128) as u8));
            }
            if i % 17 == 0 {
                let id = parts.controller.graph().assignments().next().map(|(id, _)| id);
                if let Some(id) = id {
                    parts.controller.set_assignment_depth(id, -0.15).unwrap();
                }
            }
            parts.controller.commit().unwrap();
            if i % 50 == 0 {
                parts.midi.send(MidiEvent::note_on(3, 0, 60 + (i % 12) as u8, 90));
            }
            if i % 50 == 25 {
                parts.midi.send(MidiEvent::note_off(0, 0, 60 + ((i - 25) % 12) as u8));
            }
            // Transport changes are applied on the audio thread too.
            match i % 13 {
                3 => {
                    parts.controller.set_tempo(60.0 + i as f64).unwrap();
                }
                5 => parts.controller.set_running(i % 2 == 0).unwrap(),
                8 => parts.controller.reset_transport().unwrap(),
                11 => parts.controller.set_time_signature(TimeSignature::new(7, 8)).unwrap(),
                _ => {}
            }
            // Drain telemetry for a while, then let the queues fill up.
            if i < 200 {
                parts.telemetry.meters().for_each(drop);
                parts.telemetry.scope().for_each(drop);
            }
            let frames = [1, 32, 128, 129, 1024][i % 5];

            // Audio thread: no allocation allowed.
            assert_no_alloc(|| {
                parts.engine.process(&mut left[..frames], &mut right[..frames]);
            });
        }
        let stats = parts.controller.stats();
        assert!(stats.dropped_scope.load(std::sync::atomic::Ordering::Relaxed) > 0);
    }
}
