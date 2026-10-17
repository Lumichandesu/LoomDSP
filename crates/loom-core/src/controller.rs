//! The control-thread side of the engine.

use crate::compile::{CompileReport, compile};
use crate::descriptor::{ModuleConfig, ModuleDescriptor, ModuleRole, Polarity};
use crate::display::{DisplayReader, DisplaySource, DisplayTable};
use crate::engine::{Command, Engine, EngineConfig, EngineStats, Garbage};
use crate::error::LoomError;
use crate::graph::{Assignment, AssignmentId, Graph, InputRef, ModuleId, Node, OutputRef};
use crate::midi::MidiEvent;
use crate::midi_map::{
    ControlChangeSeen, MidiControl, MidiFeedback, MidiMapping, MidiMappingId, MidiReader, control_value,
};
use crate::registry::Registry;
use crate::telemetry::{ProbeId, ProbeKind, Telemetry, TelemetrySender};
use crate::transport::{TimeSignature, TransportSettings, clamp_tempo};
use rtrb::{Consumer, Producer, RingBuffer};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The handles returned by [`create_engine`].
pub struct EngineParts {
    /// Edits the graph. Keep it on a control thread.
    pub controller: Controller,
    /// Renders audio. Move it to the audio thread.
    pub engine: Engine,
    /// Sends MIDI to the engine. Move it to the MIDI input thread.
    pub midi: MidiSender,
    /// Meter and scope data from the engine. Read it on a control thread.
    pub telemetry: Telemetry,
}

/// Creates a connected controller, engine, MIDI sender, and telemetry reader.
pub fn create_engine(registry: Arc<Registry>, config: EngineConfig) -> EngineParts {
    let (command_tx, command_rx) = RingBuffer::new(config.command_capacity);
    // Every command can produce at most one piece of garbage, so a garbage
    // queue as large as the command queue cannot overflow as long as the
    // controller drains it before sending more commands.
    let (garbage_tx, garbage_rx) = RingBuffer::new(config.command_capacity);
    let (midi_tx, midi_rx) = RingBuffer::new(config.midi_capacity);
    let (meter_tx, meter_rx) = RingBuffer::new(config.meter_capacity);
    let (scope_tx, scope_rx) = RingBuffer::new(config.scope_capacity);
    let stats = Arc::new(EngineStats::default());
    let displays = Arc::new(DisplayTable::new(config.max_modules));
    let midi_feedback = Arc::new(MidiFeedback::new());
    let engine = Engine::new(
        config,
        command_rx,
        garbage_tx,
        (midi_rx, midi_feedback.clone()),
        TelemetrySender {
            meters: meter_tx,
            scope: scope_tx,
        },
        displays.clone(),
        stats.clone(),
    );
    let controller = Controller {
        registry,
        graph: Graph::default(),
        commands: command_tx,
        garbage: garbage_rx,
        free_slots: (0..config.max_modules).rev().collect(),
        pending_removals: Vec::new(),
        probes: BTreeMap::new(),
        next_probe: 1,
        next_id: 1,
        next_assignment: 1,
        next_midi_mapping: 1,
        midi_seen: BTreeMap::new(),
        midi_feedback,
        dirty: false,
        last_report: CompileReport::default(),
        transport: TransportSettings::default(),
        displays,
        config,
        stats,
    };
    EngineParts {
        controller,
        engine,
        midi: MidiSender { tx: midi_tx },
        telemetry: Telemetry::new(meter_rx, scope_rx),
    }
}

/// Sends MIDI events to the engine from any single thread.
pub struct MidiSender {
    tx: Producer<MidiEvent>,
}

impl MidiSender {
    /// Queues an event. Returns false if the queue is full (event dropped).
    pub fn send(&mut self, event: MidiEvent) -> bool {
        self.tx.push(event).is_ok()
    }
}

/// Edits the graph and keeps the engine in sync.
///
/// Structural edits (add, remove, connect, disconnect, add or remove a
/// modulation assignment) take effect when you call [`Controller::commit`].
/// Batch several edits, then commit once. Parameter changes, and changes to
/// an assignment's depth, polarity, or enabled flag, take effect at the next
/// buffer without a recompile.
pub struct Controller {
    registry: Arc<Registry>,
    graph: Graph,
    commands: Producer<Command>,
    garbage: Consumer<Garbage>,
    free_slots: Vec<usize>,
    pending_removals: Vec<usize>,
    probes: BTreeMap<ProbeId, (OutputRef, ProbeKind)>,
    next_probe: u32,
    next_id: u32,
    next_assignment: u32,
    next_midi_mapping: u32,
    /// The last feedback sequence number applied, per MIDI mapping.
    midi_seen: BTreeMap<MidiMappingId, u32>,
    /// What the engine's MIDI mappings did (MIDI learn, rules 2 and 3).
    midi_feedback: Arc<MidiFeedback>,
    dirty: bool,
    last_report: CompileReport,
    /// What the engine's transport is set to, as of the last command sent.
    transport: TransportSettings,
    /// Display values from the engine (ADR-0013).
    displays: Arc<DisplayTable>,
    config: EngineConfig,
    stats: Arc<EngineStats>,
}

impl Controller {
    /// The module registry.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// The current graph (including uncommitted edits).
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Engine settings.
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Engine counters.
    pub fn stats(&self) -> &Arc<EngineStats> {
        &self.stats
    }

    /// The report from the last successful [`Controller::commit`].
    pub fn last_report(&self) -> &CompileReport {
        &self.last_report
    }

    /// True if there are structural or probe edits not yet committed.
    pub fn has_uncommitted_changes(&self) -> bool {
        self.dirty
    }

    /// Adds a module by kind (`"id@version"` or `"id"`). Returns its new ID.
    pub fn add_module(&mut self, kind: &str) -> Result<ModuleId, LoomError> {
        let descriptor = self.registry.resolve(kind)?;
        let id = ModuleId(self.next_id);
        self.add_descriptor(id, descriptor)?;
        Ok(id)
    }

    /// Adds a module with a specific ID, for example when loading a patch.
    pub fn add_module_with_id(&mut self, id: ModuleId, kind: &str) -> Result<(), LoomError> {
        let descriptor = self.registry.resolve(kind)?;
        self.add_descriptor(id, descriptor)
    }

    fn add_descriptor(&mut self, id: ModuleId, descriptor: &'static ModuleDescriptor) -> Result<(), LoomError> {
        if self.graph.nodes.contains_key(&id) {
            return Err(LoomError::DuplicateModuleId(id));
        }
        self.collect_garbage();
        let slot = match descriptor.role {
            ModuleRole::Dsp(factory) => {
                let slot = self
                    .free_slots
                    .pop()
                    .ok_or(LoomError::TooManyModules(self.config.max_modules))?;
                let mut module = factory(&ModuleConfig {
                    sample_rate: self.config.sample_rate,
                    max_block: self.config.max_block,
                });
                for (index, param) in descriptor.params.iter().enumerate() {
                    module.set_param(index, param.default);
                }
                // The new slot is not in the running schedule yet, so
                // inserting it now is harmless and spreads the work out.
                let insert = Command::InsertModule {
                    slot,
                    module,
                    id: id.0,
                    displays: descriptor.displays.len() as u8,
                };
                if let Err(rtrb::PushError::Full(_)) = self.commands.push(insert) {
                    self.free_slots.push(slot);
                    return Err(LoomError::QueueFull);
                }
                Some(slot)
            }
            ModuleRole::UnitDelay | ModuleRole::AudioOutput => None,
        };
        self.graph.nodes.insert(
            id,
            Node {
                descriptor,
                params: descriptor.default_params(),
                slot,
            },
        );
        self.next_id = self.next_id.max(id.0 + 1);
        self.dirty = true;
        Ok(())
    }

    /// Removes a module, every cable touching it, every modulation assignment
    /// it is part of, and every probe on it.
    pub fn remove_module(&mut self, id: ModuleId) -> Result<(), LoomError> {
        let node = self.graph.nodes.remove(&id).ok_or(LoomError::UnknownModule(id))?;
        self.graph
            .cables
            .retain(|to, from| to.module != id && from.module != id);
        self.graph
            .assignments
            .retain(|_, assignment| assignment.source.module != id && assignment.target != id);
        self.probes.retain(|_, (from, _)| from.module != id);
        self.graph.midi_mappings.retain(|_, mapping| mapping.target != id);
        if let Some(slot) = node.slot {
            // The running schedule still uses this slot. The instance is
            // removed after the next schedule swap (see `commit`).
            self.pending_removals.push(slot);
        }
        self.dirty = true;
        Ok(())
    }

    /// Connects an output to an input. Any cable already on that input is
    /// replaced and returned.
    pub fn connect(&mut self, from: OutputRef, to: InputRef) -> Result<Option<OutputRef>, LoomError> {
        self.check_output(from)?;
        self.check_input(to)?;
        self.dirty = true;
        Ok(self.graph.cables.insert(to, from))
    }

    /// Connects ports by name.
    pub fn connect_named(
        &mut self,
        from: ModuleId,
        output: &str,
        to: ModuleId,
        input: &str,
    ) -> Result<Option<OutputRef>, LoomError> {
        let from = self.output_ref(from, output)?;
        let to = self.input_ref(to, input)?;
        self.connect(from, to)
    }

    /// Assigns an output to a parameter. Depth is clamped to -1..1.
    /// Takes effect on the next [`Controller::commit`].
    ///
    /// The new assignment follows the source output's polarity and starts
    /// enabled, with the given depth.
    pub fn add_assignment(
        &mut self,
        source: OutputRef,
        target: ModuleId,
        param: usize,
        depth: f32,
    ) -> Result<AssignmentId, LoomError> {
        self.check_output(source)?;
        self.check_param(target, param)?;
        let id = AssignmentId(self.next_assignment);
        self.next_assignment += 1;
        self.graph.assignments.insert(
            id,
            Assignment {
                source,
                target,
                param,
                depth: clamp_depth(depth),
                polarity: None,
                enabled: true,
            },
        );
        self.dirty = true;
        Ok(id)
    }

    /// Assigns an output to a parameter, both named.
    pub fn add_assignment_named(
        &mut self,
        source: ModuleId,
        output: &str,
        target: ModuleId,
        param: &str,
        depth: f32,
    ) -> Result<AssignmentId, LoomError> {
        let source = self.output_ref(source, output)?;
        let param = self.param_index(target, param)?;
        self.add_assignment(source, target, param, depth)
    }

    /// Removes one modulation assignment. Takes effect on the next commit.
    pub fn remove_assignment(&mut self, id: AssignmentId) -> Result<Assignment, LoomError> {
        let removed = self
            .graph
            .assignments
            .remove(&id)
            .ok_or(LoomError::UnknownAssignment(id))?;
        self.dirty = true;
        Ok(removed)
    }

    /// Sets an assignment's depth, clamped to -1..1. Takes effect at the next
    /// buffer, without recompiling.
    pub fn set_assignment_depth(&mut self, id: AssignmentId, depth: f32) -> Result<f32, LoomError> {
        let depth = clamp_depth(depth);
        self.assignment_mut(id)?.depth = depth;
        self.send_assignment(id)?;
        Ok(depth)
    }

    /// Forces an assignment's polarity, or `None` to follow the source output.
    /// Takes effect at the next buffer, without recompiling.
    pub fn set_assignment_polarity(&mut self, id: AssignmentId, polarity: Option<Polarity>) -> Result<(), LoomError> {
        self.assignment_mut(id)?.polarity = polarity;
        self.send_assignment(id)
    }

    /// Turns an assignment on or off. The scheduling edge stays, so this does
    /// not recompile. Takes effect at the next buffer.
    pub fn set_assignment_enabled(&mut self, id: AssignmentId, enabled: bool) -> Result<(), LoomError> {
        self.assignment_mut(id)?.enabled = enabled;
        self.send_assignment(id)
    }

    fn assignment_mut(&mut self, id: AssignmentId) -> Result<&mut Assignment, LoomError> {
        self.graph
            .assignments
            .get_mut(&id)
            .ok_or(LoomError::UnknownAssignment(id))
    }

    fn send_assignment(&mut self, id: AssignmentId) -> Result<(), LoomError> {
        let assignment = *self.assignment_mut(id)?;
        self.send(Command::UpdateAssignment {
            id,
            depth: assignment.depth,
            enabled: assignment.enabled,
            polarity: assignment.polarity,
        })
    }

    /// Maps a hardware control to a parameter. Takes effect on the next
    /// [`Controller::commit`].
    ///
    /// A parameter has one mapping, and so does a control, so this first
    /// removes any mapping on the parameter and any mapping on the control.
    /// Those are returned, at most two, so the UI can say what moved.
    /// Spec: `docs/design/midi-learn.md`.
    pub fn map_midi(
        &mut self,
        control: MidiControl,
        target: ModuleId,
        param: usize,
    ) -> Result<(MidiMappingId, Vec<MidiMapping>), LoomError> {
        if !control.is_valid() {
            return Err(LoomError::InvalidMidiControl(control));
        }
        self.check_param(target, param)?;
        let replaced: Vec<MidiMappingId> = self
            .graph
            .midi_mappings()
            .filter(|(_, mapping)| mapping.control == control || (mapping.target == target && mapping.param == param))
            .map(|(id, _)| id)
            .collect();
        let removed = replaced
            .into_iter()
            .filter_map(|id| {
                self.midi_seen.remove(&id);
                self.graph.midi_mappings.remove(&id)
            })
            .collect();
        let id = MidiMappingId(self.next_midi_mapping);
        self.next_midi_mapping += 1;
        self.graph
            .midi_mappings
            .insert(id, MidiMapping { control, target, param });
        self.dirty = true;
        Ok((id, removed))
    }

    /// Maps a hardware control to a parameter by name.
    pub fn map_midi_named(
        &mut self,
        control: MidiControl,
        target: ModuleId,
        param: &str,
    ) -> Result<(MidiMappingId, Vec<MidiMapping>), LoomError> {
        let param = self.param_index(target, param)?;
        self.map_midi(control, target, param)
    }

    /// Finishes MIDI learn: maps the control that moved to a parameter, and
    /// sets the parameter from that control change, so the user hears at
    /// once that the link works. Commit afterwards.
    pub fn learn_midi(
        &mut self,
        seen: ControlChangeSeen,
        target: ModuleId,
        param: usize,
    ) -> Result<(MidiMappingId, Vec<MidiMapping>), LoomError> {
        let result = self.map_midi(seen.control, target, param)?;
        let spec = &self.node(target)?.descriptor.params[param];
        self.set_param(target, param, control_value(spec, seen.value))?;
        Ok(result)
    }

    /// Removes the MIDI mapping on a parameter, if there is one. Takes effect
    /// on the next [`Controller::commit`].
    pub fn unmap_midi(&mut self, target: ModuleId, param: usize) -> Result<Option<MidiMapping>, LoomError> {
        self.check_param(target, param)?;
        let Some((id, _)) = self.graph.midi_mapping_of(target, param) else {
            return Ok(None);
        };
        self.midi_seen.remove(&id);
        self.dirty = true;
        Ok(self.graph.midi_mappings.remove(&id))
    }

    /// The last control change the engine received, from any device, for
    /// MIDI learn. `None` before the first one.
    pub fn last_control_change(&self) -> Option<ControlChangeSeen> {
        self.midi_feedback.last()
    }

    /// A handle that reads the last control change from any thread.
    pub fn midi_reader(&self) -> MidiReader {
        MidiReader::new(self.midi_feedback.clone())
    }

    /// Copies the values MIDI mappings set in the engine into the graph, so
    /// the UI shows and saves what plays. Call it whenever the UI draws.
    /// Returns the parameters that changed.
    pub fn sync_midi(&mut self) -> Vec<(ModuleId, usize)> {
        let mut changed = Vec::new();
        for (&id, mapping) in &self.graph.midi_mappings {
            let entry = self.midi_feedback.read(mapping.control);
            if entry.id != id || self.midi_seen.get(&id) == Some(&entry.sequence) {
                continue;
            }
            self.midi_seen.insert(id, entry.sequence);
            if let Some(node) = self.graph.nodes.get_mut(&mapping.target)
                && let Some(spec) = node.descriptor.params.get(mapping.param)
            {
                node.params[mapping.param] = control_value(spec, entry.value);
                changed.push((mapping.target, mapping.param));
            }
        }
        changed
    }

    /// Removes the cable on an input. Returns where it came from.
    pub fn disconnect(&mut self, to: InputRef) -> Result<Option<OutputRef>, LoomError> {
        self.check_input(to)?;
        let removed = self.graph.cables.remove(&to);
        if removed.is_some() {
            self.dirty = true;
        }
        Ok(removed)
    }

    /// Resolves an output port by name.
    pub fn output_ref(&self, module: ModuleId, name: &str) -> Result<OutputRef, LoomError> {
        let node = self.node(module)?;
        let port = node
            .descriptor
            .output_index(name)
            .ok_or_else(|| LoomError::NoSuchPort {
                module,
                direction: "output",
                port: name.to_string(),
            })?;
        Ok(OutputRef { module, port })
    }

    /// Resolves an input port by name.
    pub fn input_ref(&self, module: ModuleId, name: &str) -> Result<InputRef, LoomError> {
        let node = self.node(module)?;
        let port = node.descriptor.input_index(name).ok_or_else(|| LoomError::NoSuchPort {
            module,
            direction: "input",
            port: name.to_string(),
        })?;
        Ok(InputRef { module, port })
    }

    /// Sets a parameter by index. Takes effect at the next buffer.
    /// The value is clamped to the parameter's range.
    pub fn set_param(&mut self, module: ModuleId, index: usize, value: f32) -> Result<f32, LoomError> {
        let node = self
            .graph
            .nodes
            .get_mut(&module)
            .ok_or(LoomError::UnknownModule(module))?;
        let spec = node
            .descriptor
            .params
            .get(index)
            .ok_or_else(|| LoomError::NoSuchParam {
                module,
                param: index.to_string(),
            })?;
        let value = spec.clamp(value);
        node.params[index] = value;
        if let Some(slot) = node.slot
            && self.commands.push(Command::SetParam { slot, index, value }).is_err()
        {
            return Err(LoomError::QueueFull);
        }
        Ok(value)
    }

    /// Sets a parameter by name.
    pub fn set_param_named(&mut self, module: ModuleId, name: &str, value: f32) -> Result<f32, LoomError> {
        let index = self
            .node(module)?
            .descriptor
            .param_index(name)
            .ok_or_else(|| LoomError::NoSuchParam {
                module,
                param: name.to_string(),
            })?;
        self.set_param(module, index, value)
    }

    /// Clears a module's internal state.
    pub fn reset_module(&mut self, module: ModuleId) -> Result<(), LoomError> {
        if let Some(slot) = self.node(module)?.slot {
            self.commands
                .push(Command::ResetModule { slot })
                .map_err(|_| LoomError::QueueFull)?;
        }
        Ok(())
    }

    /// Display value `index` of a module, as of the last block the engine
    /// ran, for a light on its panel (ADR-0013). `None` if the module does
    /// not exist or has no such value. A module the engine has not run yet
    /// reads 0.
    pub fn display(&self, module: ModuleId, index: usize) -> Option<f32> {
        self.display_source(module, index)
            .map(|source| self.display_reader().read(source))
    }

    /// Where display value `index` of a module lives, for reading it with a
    /// [`DisplayReader`] on another thread. `None` if the module does not
    /// exist or has no such value.
    pub fn display_source(&self, module: ModuleId, index: usize) -> Option<DisplaySource> {
        let node = self.graph.nodes.get(&module)?;
        let slot = node.slot?;
        (index < node.descriptor.displays.len()).then(|| DisplaySource::new(slot, module.0, index))
    }

    /// A handle that reads display values from any thread.
    pub fn display_reader(&self) -> DisplayReader {
        DisplayReader::new(self.displays.clone())
    }

    /// The transport settings last sent to the engine (ADR-0015).
    pub fn transport(&self) -> TransportSettings {
        self.transport
    }

    /// Sets the tempo in beats per minute, clamped to 20 to 300. Returns the
    /// tempo used. Takes effect at the next buffer, and the position does
    /// not jump.
    pub fn set_tempo(&mut self, bpm: f64) -> Result<f64, LoomError> {
        let tempo = clamp_tempo(bpm);
        self.send(Command::SetTempo(tempo))?;
        self.transport.tempo = tempo;
        Ok(tempo)
    }

    /// Sets the time signature. One out of range is refused with
    /// [`LoomError::InvalidTimeSignature`], and the transport does not
    /// change.
    pub fn set_time_signature(&mut self, time_signature: TimeSignature) -> Result<(), LoomError> {
        if !time_signature.is_valid() {
            return Err(LoomError::InvalidTimeSignature(time_signature));
        }
        self.send(Command::SetTimeSignature(time_signature))?;
        self.transport.time_signature = time_signature;
        Ok(())
    }

    /// Starts or stops the transport at the next buffer. Stopping holds the
    /// position; starting continues from there.
    pub fn set_running(&mut self, running: bool) -> Result<(), LoomError> {
        self.send(Command::SetRunning(running))?;
        self.transport.running = running;
        Ok(())
    }

    /// Returns the transport to bar 1 at the next buffer, running or stopped.
    /// Modules see it as a jump ([`TransportInfo::jumped`](crate::TransportInfo::jumped)).
    pub fn reset_transport(&mut self) -> Result<(), LoomError> {
        self.send(Command::ResetTransport)
    }

    fn send(&mut self, command: Command) -> Result<(), LoomError> {
        self.commands.push(command).map_err(|_| LoomError::QueueFull)
    }

    /// Starts watching an output port. The engine sends its telemetry from
    /// the next [`Controller::commit`] on (FR-VIS-04).
    ///
    /// Probes are view state: they are not part of the graph and are not
    /// saved in patches. Removing the module removes its probes.
    pub fn add_probe(&mut self, output: OutputRef, kind: ProbeKind) -> Result<ProbeId, LoomError> {
        self.check_output(output)?;
        let id = ProbeId(self.next_probe);
        self.next_probe += 1;
        self.probes.insert(id, (output, kind));
        self.dirty = true;
        Ok(id)
    }

    /// Stops watching a port, from the next [`Controller::commit`] on.
    /// Telemetry already in the queues may still arrive.
    pub fn remove_probe(&mut self, id: ProbeId) -> Result<(), LoomError> {
        self.probes.remove(&id).ok_or(LoomError::UnknownProbe(id))?;
        self.dirty = true;
        Ok(())
    }

    /// The port and kind of a probe.
    pub fn probe(&self, id: ProbeId) -> Option<(OutputRef, ProbeKind)> {
        self.probes.get(&id).copied()
    }

    /// Every probe, sorted by ID.
    pub fn probes(&self) -> impl Iterator<Item = (ProbeId, OutputRef, ProbeKind)> + '_ {
        self.probes.iter().map(|(&id, &(from, kind))| (id, from, kind))
    }

    /// Compiles the graph and sends it to the engine. The engine switches to
    /// it at the start of its next buffer, with no dropout (FR-GRAPH-02).
    pub fn commit(&mut self) -> Result<&CompileReport, LoomError> {
        self.collect_garbage();
        if !self.dirty {
            return Ok(&self.last_report);
        }
        // Make sure the swap and every removal fit before sending anything,
        // so the engine never sees half an update.
        if self.commands.slots() < 1 + self.pending_removals.len() {
            return Err(LoomError::QueueFull);
        }
        let (schedule, report) = compile(
            &self.graph,
            &self.probes,
            self.config.sample_rate,
            self.config.max_block,
        );
        // Order matters: swap first (the new schedule no longer uses the
        // removed slots), then remove the instances.
        let pushed = self.commands.push(Command::SwapSchedule(Box::new(schedule))).is_ok()
            && self
                .pending_removals
                .iter()
                .all(|&slot| self.commands.push(Command::RemoveModule { slot }).is_ok());
        debug_assert!(pushed, "capacity was checked above");
        self.free_slots.append(&mut self.pending_removals);
        self.dirty = false;
        self.last_report = report;
        Ok(&self.last_report)
    }

    /// Frees memory the engine has handed back. Called automatically by
    /// edits and commits. Call it now and then when idle.
    pub fn collect_garbage(&mut self) {
        while let Ok(garbage) = self.garbage.pop() {
            drop(garbage);
        }
    }

    fn node(&self, id: ModuleId) -> Result<&Node, LoomError> {
        self.graph.nodes.get(&id).ok_or(LoomError::UnknownModule(id))
    }

    fn check_output(&self, output: OutputRef) -> Result<(), LoomError> {
        let node = self.node(output.module)?;
        if output.port >= node.descriptor.outputs.len() {
            return Err(LoomError::NoSuchPort {
                module: output.module,
                direction: "output",
                port: output.port.to_string(),
            });
        }
        Ok(())
    }

    fn param_index(&self, module: ModuleId, name: &str) -> Result<usize, LoomError> {
        self.node(module)?
            .descriptor
            .param_index(name)
            .ok_or_else(|| LoomError::NoSuchParam {
                module,
                param: name.to_string(),
            })
    }

    fn check_param(&self, module: ModuleId, index: usize) -> Result<(), LoomError> {
        let node = self.node(module)?;
        if index >= node.descriptor.params.len() {
            return Err(LoomError::NoSuchParam {
                module,
                param: index.to_string(),
            });
        }
        Ok(())
    }

    fn check_input(&self, input: InputRef) -> Result<(), LoomError> {
        let node = self.node(input.module)?;
        if input.port >= node.descriptor.inputs.len() {
            return Err(LoomError::NoSuchPort {
                module: input.module,
                direction: "input",
                port: input.port.to_string(),
            });
        }
        Ok(())
    }
}

/// Clamps a modulation depth to -1..1. NaN becomes 0, which adds nothing.
fn clamp_depth(depth: f32) -> f32 {
    if depth.is_nan() { 0.0 } else { depth.clamp(-1.0, 1.0) }
}
