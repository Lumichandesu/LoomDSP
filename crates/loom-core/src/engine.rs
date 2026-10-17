//! The audio-thread side of the engine.
//!
//! Real-time rules (NFR-RT-01): nothing in this file allocates, frees, locks,
//! or waits once the engine is running. Memory arrives pre-allocated through
//! the command queue and leaves through the garbage queue. Telemetry leaves
//! through its own queues and is dropped when they are full (NFR-RT-07).

use crate::compile::{AssignmentRuntime, ModuleOp, ParamFill, Schedule, Step};
use crate::descriptor::{MAX_DISPLAYS, Polarity};
use crate::display::DisplayTable;
use crate::graph::AssignmentId;
use crate::midi::MidiEvent;
use crate::module::{Module, PortIo, ProcessContext};
use crate::telemetry::{MeterReading, SCOPE_CHUNK, ScopeChunk, TelemetrySender, measure};
use crate::transport::{TimeSignature, Transport};
use rtrb::{Consumer, Producer};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Engine settings, fixed for the engine's lifetime.
#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    /// Sample rate in Hz.
    pub sample_rate: f32,
    /// Internal block size. Host buffers of any size are split into blocks of
    /// at most this many frames.
    pub max_block: usize,
    /// Maximum number of DSP module instances.
    pub max_modules: usize,
    /// Capacity of the control-to-audio command queue.
    pub command_capacity: usize,
    /// Capacity of the MIDI input queue and per-buffer MIDI event list.
    pub midi_capacity: usize,
    /// Capacity of the meter queue, in readings. One reading per metered
    /// probe per block.
    pub meter_capacity: usize,
    /// Capacity of the scope queue, in chunks of
    /// [`SCOPE_CHUNK`](crate::SCOPE_CHUNK) samples.
    pub scope_capacity: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            max_block: 128,
            max_modules: 1024,
            command_capacity: 1024,
            midi_capacity: 1024,
            meter_capacity: 1024,
            scope_capacity: 1024,
        }
    }
}

impl EngineConfig {
    /// Default settings at the given sample rate.
    pub fn with_sample_rate(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            ..Self::default()
        }
    }
}

/// Counters the engine updates and the control thread reads.
#[derive(Debug, Default)]
pub struct EngineStats {
    /// Buffers processed.
    pub buffers: AtomicU64,
    /// Schedules swapped in.
    pub schedule_swaps: AtomicU64,
    /// MIDI events dropped because the per-buffer list was full.
    pub dropped_midi: AtomicU64,
    /// Objects the engine could not return for freeing (garbage queue full).
    /// They are leaked rather than freed on the audio thread.
    pub leaked: AtomicU64,
    /// Meter readings dropped because the meter queue was full.
    pub dropped_meters: AtomicU64,
    /// Scope chunks dropped because the scope queue was full.
    pub dropped_scope: AtomicU64,
}

pub(crate) enum Command {
    InsertModule {
        slot: usize,
        module: Box<dyn Module>,
        /// The module's ID, which owns its display values.
        id: u32,
        /// How many display values it publishes.
        displays: u8,
    },
    RemoveModule {
        slot: usize,
    },
    SwapSchedule(Box<Schedule>),
    SetParam {
        slot: usize,
        index: usize,
        value: f32,
    },
    /// Depth, polarity, and enabled flag of one assignment. Ignored when the
    /// running schedule has no assignment with this id.
    UpdateAssignment {
        id: AssignmentId,
        depth: f32,
        enabled: bool,
        polarity: Option<Polarity>,
    },
    ResetModule {
        slot: usize,
    },
    /// Already clamped by the controller.
    SetTempo(f64),
    /// Already checked by the controller.
    SetTimeSignature(TimeSignature),
    SetRunning(bool),
    ResetTransport,
}

pub(crate) enum Garbage {
    Module(#[allow(dead_code)] Box<dyn Module>),
    Schedule(#[allow(dead_code)] Box<Schedule>),
}

/// The audio-thread half of the engine. Call [`Engine::process`] from the
/// audio callback.
pub struct Engine {
    slots: Box<[Option<Box<dyn Module>>]>,
    /// Display values per slot (0 for most modules), and where they go.
    display_counts: Box<[u8]>,
    displays: Arc<DisplayTable>,
    schedule: Box<Schedule>,
    commands: Consumer<Command>,
    garbage: Producer<Garbage>,
    midi_in: Consumer<MidiEvent>,
    midi: Vec<MidiEvent>,
    telemetry: TelemetrySender,
    /// The patch transport (ADR-0015). Not part of the schedule, so editing
    /// the patch never moves it.
    transport: Transport,
    /// Frames rendered since the engine started. Stamps telemetry and
    /// anchors the transport.
    frame: u64,
    config: EngineConfig,
    stats: Arc<EngineStats>,
}

impl Engine {
    pub(crate) fn new(
        config: EngineConfig,
        commands: Consumer<Command>,
        garbage: Producer<Garbage>,
        midi_in: Consumer<MidiEvent>,
        telemetry: TelemetrySender,
        displays: Arc<DisplayTable>,
        stats: Arc<EngineStats>,
    ) -> Self {
        Self {
            slots: (0..config.max_modules).map(|_| None).collect(),
            display_counts: vec![0; config.max_modules].into_boxed_slice(),
            displays,
            schedule: Box::new(Schedule::empty()),
            commands,
            garbage,
            midi_in,
            midi: Vec::with_capacity(config.midi_capacity),
            telemetry,
            transport: Transport::new(config.sample_rate),
            frame: 0,
            config,
            stats,
        }
    }

    /// Engine settings.
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Renders `left.len()` frames (both slices must have the same length).
    ///
    /// Applies pending changes first, at this buffer boundary (FR-GRAPH-02).
    /// MIDI events are stamped relative to the start of this buffer.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        self.apply_commands();
        self.collect_midi(frames);

        left[..frames].fill(0.0);
        right[..frames].fill(0.0);

        let mut position = 0;
        while position < frames {
            let n = (frames - position).min(self.config.max_block);
            self.run_block(
                position,
                n,
                &mut left[position..position + n],
                &mut right[position..position + n],
            );
            self.send_telemetry(n);
            self.publish_displays();
            self.frame += n as u64;
            position += n;
        }
        self.stats.buffers.fetch_add(1, Ordering::Relaxed);
    }

    /// Asks every module that has display values for them, after a block.
    fn publish_displays(&self) {
        for (slot, &count) in self.display_counts.iter().enumerate() {
            if count == 0 {
                continue;
            }
            if let Some(module) = self.slots[slot].as_ref() {
                let mut values = [0.0; MAX_DISPLAYS];
                let values = &mut values[..usize::from(count)];
                module.displays(values);
                self.displays.write(slot, values);
            }
        }
    }

    fn apply_commands(&mut self) {
        while let Ok(command) = self.commands.pop() {
            match command {
                Command::InsertModule {
                    slot,
                    module,
                    id,
                    displays,
                } => {
                    if let Some(old) = self.slots[slot].replace(module) {
                        self.discard(Garbage::Module(old));
                    }
                    self.display_counts[slot] = displays;
                    self.displays.claim(slot, id);
                }
                Command::RemoveModule { slot } => {
                    if let Some(old) = self.slots[slot].take() {
                        self.discard(Garbage::Module(old));
                    }
                    self.display_counts[slot] = 0;
                    self.displays.release(slot);
                }
                Command::SwapSchedule(mut schedule) => {
                    schedule.take_tap_state_from(&self.schedule);
                    let slots = &self.slots;
                    schedule.take_smooth_state_from(&self.schedule, |slot, param| {
                        slots[slot].as_ref().and_then(|module| module.param_glide(param))
                    });
                    let old = std::mem::replace(&mut self.schedule, schedule);
                    self.discard(Garbage::Schedule(old));
                    self.stats.schedule_swaps.fetch_add(1, Ordering::Relaxed);
                }
                Command::SetParam { slot, index, value } => {
                    if let Some(module) = self.slots[slot].as_mut() {
                        module.set_param(index, value);
                    }
                    let slot = slot as u32;
                    let index = index as u16;
                    for fill in &mut self.schedule.fills {
                        if fill.slot == slot && fill.param == index {
                            fill.base = value;
                            if let Some(smooth) = fill.smooth.as_mut() {
                                smooth.set_target(fill.spec.normalize(value));
                            }
                            break;
                        }
                    }
                }
                Command::UpdateAssignment {
                    id,
                    depth,
                    enabled,
                    polarity,
                } => {
                    if let Ok(index) = self
                        .schedule
                        .assignments
                        .binary_search_by_key(&id, |assignment| assignment.id)
                    {
                        let assignment = &mut self.schedule.assignments[index];
                        assignment.depth = depth;
                        assignment.enabled = enabled;
                        assignment.polarity = polarity;
                    }
                }
                Command::ResetModule { slot } => {
                    if let Some(module) = self.slots[slot].as_mut() {
                        module.reset();
                    }
                }
                Command::SetTempo(tempo) => self.transport.set_tempo(self.frame, tempo),
                Command::SetTimeSignature(time_signature) => self.transport.set_time_signature(time_signature),
                Command::SetRunning(running) => self.transport.set_running(self.frame, running),
                Command::ResetTransport => self.transport.reset(self.frame),
            }
        }
    }

    /// Hands memory back to the control thread for freeing.
    fn discard(&mut self, garbage: Garbage) {
        if let Err(rtrb::PushError::Full(garbage)) = self.garbage.push(garbage) {
            // Never free on the audio thread. Leaking is the lesser evil, and
            // the counter makes it visible. The garbage queue has the same
            // capacity as the command queue, so this should not happen.
            std::mem::forget(garbage);
            self.stats.leaked.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Reads the probed buffers after a block of `frames` frames and queues
    /// meter readings and scope chunks. Drops what does not fit.
    fn send_telemetry(&mut self, frames: usize) {
        for probe in &self.schedule.probes {
            // SAFETY: probe buffers are output buffers of the schedule,
            // `max_block` floats long, and `frames <= max_block`. No module
            // is running, so nothing writes to them now.
            let samples = unsafe { std::slice::from_raw_parts(probe.buffer, frames) };
            // Skip the work for a queue that is already full. The push below
            // still counts a drop if the queue fills in between.
            if probe.kind.meter() && self.telemetry.meters.is_full() {
                self.stats.dropped_meters.fetch_add(1, Ordering::Relaxed);
            } else if probe.kind.meter() {
                let (peak, rms) = measure(samples);
                let reading = MeterReading {
                    probe: probe.id,
                    frame: self.frame,
                    frames: frames as u32,
                    peak,
                    rms,
                };
                if self.telemetry.meters.push(reading).is_err() {
                    self.stats.dropped_meters.fetch_add(1, Ordering::Relaxed);
                }
            }
            if probe.kind.scope() {
                for (i, part) in samples.chunks(SCOPE_CHUNK).enumerate() {
                    if self.telemetry.scope.is_full() {
                        self.stats.dropped_scope.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let mut chunk = ScopeChunk {
                        probe: probe.id,
                        frame: self.frame + (i * SCOPE_CHUNK) as u64,
                        len: part.len() as u32,
                        samples: [0.0; SCOPE_CHUNK],
                    };
                    chunk.samples[..part.len()].copy_from_slice(part);
                    if self.telemetry.scope.push(chunk).is_err() {
                        self.stats.dropped_scope.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }

    fn collect_midi(&mut self, frames: usize) {
        self.midi.clear();
        let last = frames.saturating_sub(1) as u32;
        while let Ok(mut event) = self.midi_in.pop() {
            if self.midi.len() == self.midi.capacity() {
                self.stats.dropped_midi.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            event.frame = event.frame.min(last);
            // Insertion sort by frame (stable). Lists are short, and this
            // keeps the audio thread free of allocation.
            let at = self.midi.partition_point(|e| e.frame <= event.frame);
            self.midi.insert(at, event);
        }
    }

    fn run_block(&mut self, position: usize, frames: usize, left: &mut [f32], right: &mut [f32]) {
        let sample_rate = self.config.sample_rate;
        let midi = &self.midi[..];
        let slots = &mut self.slots;
        let transport = self.transport.info(self.frame);
        let Schedule {
            input_ptrs,
            output_ptrs,
            param_ptrs,
            ops,
            taps,
            loop_ops,
            loop_taps,
            steps,
            outputs,
            transport: transport_buffers,
            fills,
            assignments,
            ..
        } = &mut *self.schedule;

        // Transport signals first, once per block, for every input that
        // follows them. Each sample's value comes from the anchor, so it is
        // the same for any block size (rule 5).
        if let Some(buffer) = transport_buffers.bar_phase {
            // SAFETY: transport buffers are schedule buffers of `max_block`
            // floats, `frames <= max_block`, modules only read them, and no
            // module is running yet.
            let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, frames) };
            for (i, value) in buffer.iter_mut().enumerate() {
                *value = transport.bar_phase_at(i);
            }
        }
        if let Some(buffer) = transport_buffers.beat_phase {
            // SAFETY: as for the bar phase buffer above.
            let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, frames) };
            for (i, value) in buffer.iter_mut().enumerate() {
                *value = transport.beat_phase_at(i);
            }
        }
        if let Some(buffer) = transport_buffers.playing {
            // SAFETY: as for the bar phase buffer above.
            let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, frames) };
            buffer.fill(if transport.playing() { 1.0 } else { 0.0 });
        }

        let mut run_op = |op: &ModuleOp, offset: usize, len: usize| {
            for index in op.fills.0..op.fills.1 {
                fill_param(&mut fills[index as usize], assignments, offset, len);
            }
            if let Some(module) = slots[op.slot as usize].as_mut() {
                let ctx = ProcessContext {
                    sample_rate,
                    midi,
                    midi_base: position + offset,
                    frames: len,
                    transport: transport.later(offset),
                };
                // SAFETY: the compiler gives every output its own buffer of
                // `max_block` floats. Inputs point at other modules' outputs,
                // a unit delay buffer, the zero buffer, or a default buffer.
                // Cycles always pass through a unit delay, so no module reads
                // its own output. Parameter pointers address the buffers just
                // filled above. `offset + len <= frames <= max_block`.
                let mut io = unsafe {
                    PortIo::new(
                        &input_ptrs[op.inputs.0 as usize..op.inputs.1 as usize],
                        &output_ptrs[op.outputs.0 as usize..op.outputs.1 as usize],
                        &param_ptrs[op.params.0 as usize..op.params.1 as usize],
                        op.connected,
                        offset,
                        len,
                    )
                };
                module.process(&ctx, &mut io);
            }
        };

        for step in steps.iter() {
            match *step {
                Step::Module(op) => run_op(&ops[op as usize], 0, frames),
                Step::Tap(t) => {
                    let tap = &mut taps[t as usize];
                    // SAFETY: unit delay buffers are `max_block` long, and the
                    // input and output buffers are distinct.
                    unsafe {
                        let input = std::slice::from_raw_parts(tap.input, frames);
                        let output = std::slice::from_raw_parts_mut(tap.output, frames);
                        output[0] = tap.state;
                        output[1..].copy_from_slice(&input[..frames - 1]);
                        tap.state = input[frames - 1];
                    }
                }
                Step::Loop {
                    ops: op_range,
                    taps: tap_range,
                } => {
                    let region_ops = &loop_ops[op_range.0 as usize..op_range.1 as usize];
                    let region_taps = &loop_taps[tap_range.0 as usize..tap_range.1 as usize];
                    for i in 0..frames {
                        // Unit delays emit last sample's value first...
                        for &t in region_taps {
                            let tap = &taps[t as usize];
                            // SAFETY: `i < frames <= max_block`.
                            unsafe { *tap.output.add(i) = tap.state };
                        }
                        // ...then the loop's modules run for one sample...
                        for &op in region_ops {
                            run_op(&ops[op as usize], i, 1);
                        }
                        // ...then unit delays capture this sample's value.
                        for &t in region_taps {
                            let tap = &mut taps[t as usize];
                            // SAFETY: `i < frames <= max_block`.
                            tap.state = unsafe { *tap.input.add(i) };
                        }
                    }
                }
            }
        }

        for output in outputs.iter() {
            // SAFETY: output pointers reference schedule buffers of
            // `max_block` floats.
            let (l, r) = unsafe {
                (
                    std::slice::from_raw_parts(output.left, frames),
                    std::slice::from_raw_parts(output.right, frames),
                )
            };
            for (out, value) in left.iter_mut().zip(l) {
                *out += value;
            }
            for (out, value) in right.iter_mut().zip(r) {
                *out += value;
            }
        }
        clip_output(left);
        clip_output(right);

        self.transport.end_block();
    }
}

/// Writes the modulated value of one parameter for `len` frames at `offset`.
///
/// A settled base with nothing added keeps the base bits, so depth 0 matches
/// no assignment. Otherwise the sum is mapped back through the parameter.
fn fill_param(fill: &mut ParamFill, assignments: &[AssignmentRuntime], offset: usize, len: usize) {
    let spec = fill.spec;
    let base = fill.base;
    // Constant for the call. Used when the base has no smoother.
    let base_position = spec.normalize(base);
    // SAFETY: the compiler allocated `buffer` for `max_block` floats, and
    // `offset + len` stays inside that. The audio thread does not alias it
    // with a source buffer: sources are output buffers or delay taps.
    let buf = unsafe { std::slice::from_raw_parts_mut(fill.buffer.add(offset), len) };
    for (i, value) in buf.iter_mut().enumerate() {
        let mut added = 0.0;
        let mut contributed = false;
        for source in &fill.sources {
            let assignment = &assignments[source.state as usize];
            if !assignment.enabled || assignment.depth == 0.0 {
                continue;
            }
            // SAFETY: `source.buffer` is an output or delay tap of `max_block`
            // floats, and `offset + i < max_block`.
            let sample = unsafe { *source.buffer.add(offset + i) };
            let sample = match assignment.polarity {
                Some(polarity) => polarity.remap(assignment.source_polarity, sample),
                None => sample,
            };
            added += assignment.depth * sample;
            contributed = true;
        }
        let position = if let Some(smooth) = fill.smooth.as_mut() {
            smooth.tick()
        } else {
            base_position
        };
        let settled = fill
            .smooth
            .as_ref()
            .is_none_or(|smooth| smooth.current() == smooth.target());
        *value = if !contributed && settled {
            base
        } else {
            spec.denormalize(position + added)
        };
    }
}

/// The final output, as ADR-0007 asks: audio is not clipped on cables, only
/// here, to its nominal range of -1 to 1. NaN and infinity become silence,
/// so a broken patch can never send them to the device (#121).
fn clip_output(samples: &mut [f32]) {
    for sample in samples {
        *sample = if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_output_limits_the_range_and_silences_non_finite_samples() {
        let mut samples = [0.25, -0.5, 1.5, -7.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        clip_output(&mut samples);
        assert_eq!(samples, [0.25, -0.5, 1.0, -1.0, 0.0, 0.0, 0.0]);
    }
}
