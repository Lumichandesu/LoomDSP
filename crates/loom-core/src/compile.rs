//! Turns a [`Graph`] into an execution [`Schedule`] (ADR-0002).
//!
//! This runs on the control thread and may allocate freely.
//!
//! Steps:
//! 1. Give every output port its own buffer.
//! 2. Find strongly connected components (SCCs) with Tarjan's algorithm.
//!    An SCC with more than one module, or a module wired to itself, is a
//!    feedback loop.
//! 3. Order the SCCs topologically. Modules outside loops run a whole block
//!    at a time. Each loop runs one sample at a time.
//! 4. Inside a loop, user-placed unit delays break the cycle. If cycles
//!    remain, the compiler inserts its own one-sample delays and reports
//!    where, so the UI can show them. A modulation assignment is broken
//!    before a cable, so a modulation loop does not delay the audio path.
//! 5. Give each modulated parameter a buffer, filled from its base value
//!    and its sources just before the target module runs.
//! 6. Bind unconnected inputs to their default (ADR-0015): the zero buffer,
//!    a constant buffer per distinct value, or one shared buffer per
//!    transport signal that the engine fills at the start of each block.

use crate::descriptor::{ModuleRole, ParamSpec, Polarity, PortDefault};
use crate::graph::{AssignmentId, Graph, InputRef, ModuleId, OutputRef};
use crate::midi_map::{MidiMapOp, MidiMappingId};
use crate::telemetry::{ProbeId, ProbeKind};
use loom_rt::Smoothed;
use std::collections::{BTreeMap, BTreeSet};

/// Marks tap keys created by the compiler, so they never clash with user
/// unit delay keys (which are module IDs).
const AUTO_TAP: u64 = 1 << 63;

/// Marks a tap inserted on a modulation assignment. Bit 62 keeps these clear
/// of cable taps, whose keys use the module id shifted by 16 under `AUTO_TAP`.
const ASSIGN_TAP: u64 = AUTO_TAP | (1 << 62);

/// A feedback loop found by the compiler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackLoop {
    /// Every module in the loop, sorted by ID.
    pub modules: Vec<ModuleId>,
    /// Cables where the compiler inserted a one-sample delay. Empty when the
    /// loop already contains a user unit delay in every cycle.
    pub inserted_delays: Vec<(OutputRef, InputRef)>,
    /// Modulation assignments where the compiler inserted a one-sample delay.
    pub inserted_assignment_delays: Vec<AssignmentId>,
}

/// What the compiler found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompileReport {
    /// Number of modules in the schedule.
    pub modules: usize,
    /// Feedback loops, each run one sample at a time.
    pub loops: Vec<FeedbackLoop>,
}

impl CompileReport {
    /// All delays the compiler inserted, across all loops.
    pub fn inserted_delays(&self) -> impl Iterator<Item = &(OutputRef, InputRef)> {
        self.loops.iter().flat_map(|l| l.inserted_delays.iter())
    }

    /// All assignment delays the compiler inserted, across all loops.
    pub fn inserted_assignment_delays(&self) -> impl Iterator<Item = &AssignmentId> {
        self.loops.iter().flat_map(|l| l.inserted_assignment_delays.iter())
    }
}

pub(crate) struct ModuleOp {
    pub slot: u32,
    pub inputs: (u32, u32),
    pub outputs: (u32, u32),
    pub connected: u64,
    /// Range into [`Schedule::fills`].
    pub fills: (u32, u32),
    /// Range into [`Schedule::param_ptrs`], one entry per modulated parameter.
    pub params: (u32, u32),
}

/// One source of a modulated parameter. `state` indexes [`Schedule::assignments`].
pub(crate) struct ModSource {
    pub state: u32,
    pub buffer: *const f32,
}

/// A parameter the engine fills before its module runs.
pub(crate) struct ParamFill {
    /// `(module id << 32) | param index`. Matches smoother state across swaps.
    pub key: u64,
    pub slot: u32,
    pub param: u16,
    pub spec: &'static ParamSpec,
    pub base: f32,
    pub smooth: Option<Smoothed>,
    pub buffer: *mut f32,
    pub sources: Vec<ModSource>,
}

/// Depth, polarity, and enabled flag. The audio thread updates these without
/// recompiling. Sorted by [`AssignmentRuntime::id`].
pub(crate) struct AssignmentRuntime {
    pub id: AssignmentId,
    pub depth: f32,
    pub enabled: bool,
    pub polarity: Option<Polarity>,
    pub source_polarity: Polarity,
}

pub(crate) struct Tap {
    pub key: u64,
    pub input: *const f32,
    pub output: *mut f32,
    pub state: f32,
}

pub(crate) struct OutputOp {
    pub left: *const f32,
    pub right: *const f32,
}

/// A probed output buffer. Read after each block for telemetry.
pub(crate) struct ProbeOp {
    pub id: ProbeId,
    pub buffer: *const f32,
    pub kind: ProbeKind,
}

/// Buffers for transport signals that at least one unconnected input reads.
/// `None` when nothing reads that signal, so it costs nothing.
#[derive(Default)]
pub(crate) struct TransportBuffers {
    pub bar_phase: Option<*mut f32>,
    pub beat_phase: Option<*mut f32>,
    pub playing: Option<*mut f32>,
}

/// Identifies one shared default buffer. Value defaults are keyed by their
/// bits, so each distinct value gets one buffer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DefaultKey {
    Value(u32),
    BarPhase,
    BeatPhase,
    Playing,
}

impl DefaultKey {
    /// `None` for defaults that read the shared zero buffer. The pattern
    /// compares with `==`, so -0.0 reads it too.
    fn of(default: PortDefault) -> Option<Self> {
        match default {
            PortDefault::Value(0.0) => None,
            PortDefault::Value(value) => Some(Self::Value(value.to_bits())),
            PortDefault::BarPhase => Some(Self::BarPhase),
            PortDefault::BeatPhase => Some(Self::BeatPhase),
            PortDefault::Playing => Some(Self::Playing),
        }
    }
}

pub(crate) enum Step {
    /// Run a module over the whole block.
    Module(u32),
    /// Run a unit delay over the whole block.
    Tap(u32),
    /// Run a feedback loop one sample at a time.
    Loop { ops: (u32, u32), taps: (u32, u32) },
}

/// A compiled, ready-to-run graph. Built on the control thread, then moved to
/// the audio thread whole.
pub(crate) struct Schedule {
    // Owned buffer memory. The raw pointers below point into these boxes.
    // Moving the `Schedule` moves the boxes, not the heap data, so the
    // pointers stay valid.
    _buffers: Vec<Box<[f32]>>,
    _zero: Box<[f32]>,
    pub input_ptrs: Vec<*const f32>,
    pub output_ptrs: Vec<*mut f32>,
    pub ops: Vec<ModuleOp>,
    /// Sorted by key, so tap state can be carried across schedule swaps.
    pub taps: Vec<Tap>,
    pub loop_ops: Vec<u32>,
    pub loop_taps: Vec<u32>,
    pub steps: Vec<Step>,
    pub outputs: Vec<OutputOp>,
    pub probes: Vec<ProbeOp>,
    pub transport: TransportBuffers,
    pub fills: Vec<ParamFill>,
    pub assignments: Vec<AssignmentRuntime>,
    pub param_ptrs: Vec<(u16, *const f32)>,
    /// MIDI mappings, sorted by control.
    pub midi_maps: Vec<MidiMapOp>,
}

// SAFETY: the raw pointers only point into buffers owned by this schedule.
// The schedule is only ever used by one thread at a time (built on the control
// thread, then moved to the audio thread, then moved back for dropping).
unsafe impl Send for Schedule {}

impl Schedule {
    pub fn empty() -> Self {
        Self {
            _buffers: Vec::new(),
            _zero: Box::new([]),
            input_ptrs: Vec::new(),
            output_ptrs: Vec::new(),
            ops: Vec::new(),
            taps: Vec::new(),
            loop_ops: Vec::new(),
            loop_taps: Vec::new(),
            steps: Vec::new(),
            outputs: Vec::new(),
            probes: Vec::new(),
            transport: TransportBuffers::default(),
            fills: Vec::new(),
            assignments: Vec::new(),
            param_ptrs: Vec::new(),
            midi_maps: Vec::new(),
        }
    }

    /// Copies unit delay state from the previous schedule, matched by key.
    /// Runs on the audio thread: no allocation, linear time.
    pub fn take_tap_state_from(&mut self, old: &Schedule) {
        let mut j = 0;
        for tap in &mut self.taps {
            while j < old.taps.len() && old.taps[j].key < tap.key {
                j += 1;
            }
            if j < old.taps.len() && old.taps[j].key == tap.key {
                tap.state = old.taps[j].state;
            }
        }
    }

    /// Copies in-flight base smoothing onto the new schedule, matched by key.
    /// A fill that is new, because its parameter just got its first
    /// assignment, starts from `glide(slot, param)` instead: the value the
    /// module itself was gliding through, so the parameter does not step.
    /// Runs on the audio thread: no allocation.
    pub fn take_smooth_state_from(&mut self, old: &Schedule, glide: impl Fn(usize, usize) -> Option<f32>) {
        for fill in &mut self.fills {
            let Some(smooth) = fill.smooth.as_mut() else {
                continue;
            };
            let previous = old
                .fills
                .iter()
                .find(|previous| previous.key == fill.key)
                .map(|previous| previous.smooth);
            match previous {
                Some(Some(previous)) => *smooth = previous,
                Some(None) => {}
                None => {
                    if let Some(value) = glide(fill.slot as usize, usize::from(fill.param)) {
                        smooth.reset(fill.spec.normalize(value));
                        smooth.set_target(fill.spec.normalize(fill.base));
                    }
                }
            }
        }
    }
}

struct TapSpec {
    key: u64,
    input: Option<usize>,
    output: usize,
}

enum StepSpec {
    Module(usize),
    Tap(usize),
    Loop { modules: Vec<usize>, taps: Vec<usize> },
}

/// An edge inside a feedback loop. Assignment edges are broken before cables.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dep {
    Cable(InputRef),
    Assignment(AssignmentId),
}

struct CompiledAssignment {
    id: AssignmentId,
    source: OutputRef,
    target_node: usize,
    param: usize,
    depth: f32,
    enabled: bool,
    polarity: Option<Polarity>,
    source_polarity: Polarity,
}

struct SourceSpec {
    /// Index into the compiled assignment list.
    assignment: usize,
    /// Buffer index the fill reads. A delay tap when the assignment was broken.
    buffer: usize,
}

struct FillSpec {
    param: usize,
    buffer: usize,
    sources: Vec<SourceSpec>,
}

/// Compiles the graph. `max_block` is the buffer length. `probes` are output
/// ports to read for telemetry; probes on unknown modules or ports are
/// skipped. `sample_rate` sets the base-value smoother for modulated parameters.
pub(crate) fn compile(
    graph: &Graph,
    probes: &BTreeMap<ProbeId, (OutputRef, ProbeKind)>,
    midi_synced: &BTreeMap<MidiMappingId, u32>,
    sample_rate: f32,
    max_block: usize,
) -> (Schedule, CompileReport) {
    let ids: Vec<ModuleId> = graph.nodes.keys().copied().collect();
    let index_of: BTreeMap<ModuleId, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let nodes: Vec<&crate::graph::Node> = graph.nodes.values().collect();
    let n = ids.len();

    // 1. One buffer per output port.
    let mut buffer_count = 0usize;
    let out_buf: Vec<Vec<usize>> = nodes
        .iter()
        .map(|node| {
            (0..node.descriptor.outputs.len())
                .map(|_| {
                    buffer_count += 1;
                    buffer_count - 1
                })
                .collect()
        })
        .collect();
    let source_buffer = |from: OutputRef| out_buf[index_of[&from.module]][from.port];

    // Assignments the graph can actually schedule. Dangling ones are dropped.
    let mut compiled = Vec::new();
    for (id, assignment) in graph.assignments() {
        let Some(&target_node) = index_of.get(&assignment.target) else {
            continue;
        };
        let Some(&source_node) = index_of.get(&assignment.source.module) else {
            continue;
        };
        let outputs = nodes[source_node].descriptor.outputs;
        let params = nodes[target_node].descriptor.params;
        if assignment.source.port >= outputs.len() || assignment.param >= params.len() {
            continue;
        }
        compiled.push(CompiledAssignment {
            id,
            source: assignment.source,
            target_node,
            param: assignment.param,
            depth: assignment.depth,
            enabled: assignment.enabled,
            polarity: assignment.polarity,
            source_polarity: outputs[assignment.source.port].polarity,
        });
    }

    // 2. SCCs. Cables and assignments are both edges.
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut self_loop = vec![false; n];
    for (to, from) in &graph.cables {
        let u = index_of[&from.module];
        let v = index_of[&to.module];
        succ[u].push(v);
        if u == v {
            self_loop[u] = true;
        }
    }
    for assignment in &compiled {
        let u = index_of[&assignment.source.module];
        let v = assignment.target_node;
        succ[u].push(v);
        if u == v {
            self_loop[u] = true;
        }
    }
    for s in &mut succ {
        s.sort_unstable();
        s.dedup();
    }
    let mut sccs = tarjan(&succ);
    // Tarjan emits SCCs in reverse topological order.
    sccs.reverse();

    // 3 and 4. Steps.
    let mut taps: Vec<TapSpec> = Vec::new();
    let mut overrides: BTreeMap<InputRef, usize> = BTreeMap::new();
    // Assignment id to the delay buffer the fill should read.
    let mut delayed: BTreeMap<AssignmentId, usize> = BTreeMap::new();
    let mut steps: Vec<StepSpec> = Vec::new();
    let mut output_nodes: Vec<usize> = Vec::new();
    let mut report = CompileReport {
        modules: n,
        loops: Vec::new(),
    };

    let user_tap = |v: usize| TapSpec {
        key: u64::from(ids[v].0),
        input: graph
            .source_of(InputRef {
                module: ids[v],
                port: 0,
            })
            .map(source_buffer),
        output: out_buf[v][0],
    };

    for scc in &sccs {
        let is_loop = scc.len() > 1 || self_loop[scc[0]];
        if !is_loop {
            let v = scc[0];
            match nodes[v].descriptor.role {
                ModuleRole::Dsp(_) => steps.push(StepSpec::Module(v)),
                ModuleRole::UnitDelay => {
                    taps.push(user_tap(v));
                    steps.push(StepSpec::Tap(taps.len() - 1));
                }
                ModuleRole::AudioOutput => output_nodes.push(v),
            }
            continue;
        }

        let mut loop_taps = Vec::new();
        let members: BTreeSet<usize> = scc
            .iter()
            .copied()
            .filter(|&v| match nodes[v].descriptor.role {
                ModuleRole::Dsp(_) => true,
                ModuleRole::UnitDelay => {
                    taps.push(user_tap(v));
                    loop_taps.push(taps.len() - 1);
                    false
                }
                // Has no outputs, so it can never be part of a cycle.
                ModuleRole::AudioOutput => false,
            })
            .collect();

        // Edges between DSP members only. Edges into or out of unit delays
        // do not constrain the order: delays emit before and capture after.
        let mut indegree: BTreeMap<usize, usize> = members.iter().map(|&v| (v, 0)).collect();
        let mut incoming: BTreeMap<usize, Vec<(Dep, OutputRef)>> = BTreeMap::new();
        let mut outgoing: BTreeMap<usize, Vec<(Dep, usize)>> = BTreeMap::new();
        let add_edge = |indegree: &mut BTreeMap<usize, usize>,
                        incoming: &mut BTreeMap<usize, Vec<(Dep, OutputRef)>>,
                        outgoing: &mut BTreeMap<usize, Vec<(Dep, usize)>>,
                        dep: Dep,
                        from: OutputRef,
                        v: usize| {
            *indegree.get_mut(&v).expect("member") += 1;
            incoming.entry(v).or_default().push((dep, from));
            outgoing.entry(index_of[&from.module]).or_default().push((dep, v));
        };
        for (to, from) in &graph.cables {
            let u = index_of[&from.module];
            let v = index_of[&to.module];
            if members.contains(&u) && members.contains(&v) {
                add_edge(&mut indegree, &mut incoming, &mut outgoing, Dep::Cable(*to), *from, v);
            }
        }
        for assignment in &compiled {
            let u = index_of[&assignment.source.module];
            let v = assignment.target_node;
            if members.contains(&u) && members.contains(&v) {
                add_edge(
                    &mut indegree,
                    &mut incoming,
                    &mut outgoing,
                    Dep::Assignment(assignment.id),
                    assignment.source,
                    v,
                );
            }
        }

        let mut ordered = Vec::with_capacity(members.len());
        let mut done: BTreeSet<usize> = BTreeSet::new();
        let mut broken: BTreeSet<Dep> = BTreeSet::new();
        let mut ready: BTreeSet<usize> = indegree.iter().filter(|(_, d)| **d == 0).map(|(v, _)| *v).collect();
        let mut inserted = Vec::new();
        let mut inserted_assignments = Vec::new();

        let unresolved = |v: usize, dep_is: fn(&Dep) -> bool, done: &BTreeSet<usize>, broken: &BTreeSet<Dep>| {
            incoming
                .get(&v)
                .into_iter()
                .flatten()
                .any(|(dep, from)| dep_is(dep) && !broken.contains(dep) && !done.contains(&index_of[&from.module]))
        };

        loop {
            while let Some(v) = ready.pop_first() {
                ordered.push(v);
                done.insert(v);
                for (dep, w) in outgoing.get(&v).into_iter().flatten() {
                    if broken.contains(dep) {
                        continue;
                    }
                    let d = indegree.get_mut(w).expect("member");
                    *d -= 1;
                    if *d == 0 && !done.contains(w) {
                        ready.insert(*w);
                    }
                }
            }
            if ordered.len() == members.len() {
                break;
            }
            let remaining = || members.iter().copied().filter(|v| !done.contains(v));
            let is_cable = |dep: &Dep| matches!(dep, Dep::Cable(_));
            let is_assignment = |dep: &Dep| matches!(dep, Dep::Assignment(_));
            // True when `from` reaches `to` over cables that are not yet
            // broken, through modules that have not run yet.
            let cable_reaches = |from: usize, to: usize, done: &BTreeSet<usize>, broken: &BTreeSet<Dep>| {
                let mut seen = BTreeSet::from([from]);
                let mut stack = vec![from];
                while let Some(u) = stack.pop() {
                    for (dep, w) in outgoing.get(&u).into_iter().flatten() {
                        if !is_cable(dep) || broken.contains(dep) || done.contains(w) {
                            continue;
                        }
                        if *w == to {
                            return true;
                        }
                        if seen.insert(*w) {
                            stack.push(*w);
                        }
                    }
                }
                false
            };
            // An unbroken cable into `v` that lies on a cycle of cables only.
            // Only a cable delay can break such a cycle.
            let on_cable_cycle =
                |v: usize, dep: &Dep, from: &OutputRef, done: &BTreeSet<usize>, broken: &BTreeSet<Dep>| {
                    let u = index_of[&from.module];
                    is_cable(dep)
                        && !broken.contains(dep)
                        && !done.contains(&u)
                        && (u == v || cable_reaches(v, u, done, broken))
                };
            let assignments_left = remaining().any(|v| unresolved(v, is_assignment, &done, &broken));
            let cable_cycle_at = |v: usize| {
                incoming
                    .get(&v)
                    .into_iter()
                    .flatten()
                    .any(|(dep, from)| on_cable_cycle(v, dep, from, &done, &broken))
            };
            // Prefer to delay an assignment. A modulation loop then keeps the
            // audio cables on their current sample. A cycle of cables only
            // needs a cable delay anyway, so break those first: that cable
            // may also resolve the modulation loop, and then no assignment
            // is delayed for nothing.
            if assignments_left && !remaining().any(cable_cycle_at) {
                let v = remaining()
                    .filter(|&v| unresolved(v, is_assignment, &done, &broken))
                    .min_by_key(|v| {
                        let count = incoming
                            .get(v)
                            .into_iter()
                            .flatten()
                            .filter(|(dep, _)| is_assignment(dep))
                            .count();
                        (count, *v)
                    })
                    .expect("an assignment is left");
                for (dep, from) in incoming.get(&v).into_iter().flatten() {
                    let Dep::Assignment(id) = *dep else {
                        continue;
                    };
                    if done.contains(&index_of[&from.module]) || broken.contains(dep) {
                        continue;
                    }
                    buffer_count += 1;
                    let output = buffer_count - 1;
                    taps.push(TapSpec {
                        key: ASSIGN_TAP | u64::from(id.0),
                        input: Some(source_buffer(*from)),
                        output,
                    });
                    loop_taps.push(taps.len() - 1);
                    delayed.insert(id, output);
                    broken.insert(*dep);
                    inserted_assignments.push(id);
                    *indegree.get_mut(&v).expect("member") -= 1;
                }
                if indegree[&v] == 0 {
                    ready.insert(v);
                }
                continue;
            }
            // Stuck on a cycle with no unit delay. Pick the remaining module
            // with the fewest unresolved inputs (lowest ID on ties) and put a
            // one-sample delay on each of those inputs. With assignments
            // left, only modules and cables on a cable-only cycle count, so
            // no cable is delayed where an assignment delay would do.
            let v = remaining()
                .filter(|&v| !assignments_left || cable_cycle_at(v))
                .min_by_key(|v| (indegree[v], *v))
                .expect("remaining member");
            let victims: Vec<(Dep, OutputRef)> = incoming
                .get(&v)
                .into_iter()
                .flatten()
                .filter(|(dep, from)| {
                    if assignments_left {
                        on_cable_cycle(v, dep, from, &done, &broken)
                    } else {
                        is_cable(dep) && !broken.contains(dep) && !done.contains(&index_of[&from.module])
                    }
                })
                .copied()
                .collect();
            for (dep, from) in victims {
                let Dep::Cable(input) = dep else {
                    continue;
                };
                buffer_count += 1;
                let output = buffer_count - 1;
                taps.push(TapSpec {
                    key: AUTO_TAP | (u64::from(input.module.0) << 16) | (input.port as u64 & 0xFFFF),
                    input: Some(source_buffer(from)),
                    output,
                });
                loop_taps.push(taps.len() - 1);
                overrides.insert(input, output);
                broken.insert(dep);
                inserted.push((from, input));
                *indegree.get_mut(&v).expect("member") -= 1;
            }
            if indegree[&v] == 0 {
                ready.insert(v);
            }
        }

        let mut loop_modules: Vec<ModuleId> = scc.iter().map(|&v| ids[v]).collect();
        loop_modules.sort_unstable();
        report.loops.push(FeedbackLoop {
            modules: loop_modules,
            inserted_delays: inserted,
            inserted_assignment_delays: inserted_assignments,
        });
        steps.push(StepSpec::Loop {
            modules: ordered,
            taps: loop_taps,
        });
    }

    // 5. One buffer per modulated parameter. Sources in assignment id order.
    let mut groups: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (i, assignment) in compiled.iter().enumerate() {
        if matches!(nodes[assignment.target_node].descriptor.role, ModuleRole::Dsp(_)) {
            groups
                .entry((assignment.target_node, assignment.param))
                .or_default()
                .push(i);
        }
    }
    let mut fills_of: Vec<Vec<FillSpec>> = (0..n).map(|_| Vec::new()).collect();
    for ((node, param), sources) in groups {
        buffer_count += 1;
        let buffer = buffer_count - 1;
        let sources = sources
            .into_iter()
            .map(|i| {
                let assignment = &compiled[i];
                let buffer = delayed
                    .get(&assignment.id)
                    .copied()
                    .unwrap_or_else(|| source_buffer(assignment.source));
                SourceSpec { assignment: i, buffer }
            })
            .collect();
        fills_of[node].push(FillSpec { param, buffer, sources });
    }

    // 6. One buffer per distinct default that an unconnected input reads.
    // Only DSP modules read their inputs through pointers.
    let default_of = |input: InputRef| nodes[index_of[&input.module]].descriptor.inputs[input.port].default;
    let mut default_buf: BTreeMap<DefaultKey, usize> = BTreeMap::new();
    for (v, node) in nodes.iter().enumerate() {
        if !matches!(node.descriptor.role, ModuleRole::Dsp(_)) {
            continue;
        }
        for port in 0..node.descriptor.inputs.len() {
            let input = InputRef { module: ids[v], port };
            if graph.cables.contains_key(&input) {
                continue;
            }
            if let Some(key) = DefaultKey::of(default_of(input)) {
                default_buf.entry(key).or_insert_with(|| {
                    buffer_count += 1;
                    buffer_count - 1
                });
            }
        }
    }

    // Allocate buffers and resolve pointers.
    let mut buffers: Vec<Box<[f32]>> = (0..buffer_count)
        .map(|_| vec![0.0; max_block].into_boxed_slice())
        .collect();
    for (key, &buf) in &default_buf {
        if let DefaultKey::Value(bits) = *key {
            // Filled once here. Inputs are read-only, so nothing changes it.
            buffers[buf].fill(f32::from_bits(bits));
        }
    }
    let mut zero: Box<[f32]> = vec![0.0; max_block].into_boxed_slice();
    let zero_ptr: *const f32 = zero.as_mut_ptr();
    let buf_ptrs: Vec<*mut f32> = buffers.iter_mut().map(|b| b.as_mut_ptr()).collect();
    let transport_buffer = |key: DefaultKey| default_buf.get(&key).map(|&buf| buf_ptrs[buf]);
    let transport = TransportBuffers {
        bar_phase: transport_buffer(DefaultKey::BarPhase),
        beat_phase: transport_buffer(DefaultKey::BeatPhase),
        playing: transport_buffer(DefaultKey::Playing),
    };

    let resolve_input = |input: InputRef| -> (*const f32, bool) {
        let connected = graph.cables.contains_key(&input);
        if let Some(&buf) = overrides.get(&input) {
            (buf_ptrs[buf], connected)
        } else if let Some(from) = graph.cables.get(&input) {
            (buf_ptrs[source_buffer(*from)], connected)
        } else if let Some(key) = DefaultKey::of(default_of(input)) {
            (buf_ptrs[default_buf[&key]], false)
        } else {
            (zero_ptr, false)
        }
    };

    let mut input_ptrs = Vec::new();
    let mut output_ptrs = Vec::new();
    let mut param_ptrs = Vec::new();
    let mut fills = Vec::new();
    let mut ops = Vec::new();
    let mut op_of_node = vec![u32::MAX; n];
    let mut make_op = |v: usize| -> u32 {
        if op_of_node[v] != u32::MAX {
            return op_of_node[v];
        }
        let node = nodes[v];
        let in_start = input_ptrs.len() as u32;
        let mut connected = 0u64;
        for port in 0..node.descriptor.inputs.len() {
            let (ptr, is_connected) = resolve_input(InputRef { module: ids[v], port });
            input_ptrs.push(ptr);
            if is_connected {
                connected |= 1 << port;
            }
        }
        let out_start = output_ptrs.len() as u32;
        output_ptrs.extend(out_buf[v].iter().map(|&b| buf_ptrs[b]));
        let fill_start = fills.len() as u32;
        let param_start = param_ptrs.len() as u32;
        for spec in &fills_of[v] {
            let param_spec = &node.descriptor.params[spec.param];
            let base = node.params.get(spec.param).copied().unwrap_or(param_spec.default);
            let smooth = (param_spec.options.smooth_seconds > 0.0).then(|| {
                Smoothed::new(
                    param_spec.normalize(base),
                    sample_rate,
                    param_spec.options.smooth_seconds,
                )
            });
            let sources = spec
                .sources
                .iter()
                .map(|source| ModSource {
                    state: source.assignment as u32,
                    buffer: buf_ptrs[source.buffer] as *const f32,
                })
                .collect();
            param_ptrs.push((spec.param as u16, buf_ptrs[spec.buffer] as *const f32));
            fills.push(ParamFill {
                key: (u64::from(ids[v].0) << 32) | (spec.param as u64),
                slot: node.slot.expect("DSP modules have a slot") as u32,
                param: spec.param as u16,
                spec: param_spec,
                base,
                smooth,
                buffer: buf_ptrs[spec.buffer],
                sources,
            });
        }
        ops.push(ModuleOp {
            slot: node.slot.expect("DSP modules have a slot") as u32,
            inputs: (in_start, input_ptrs.len() as u32),
            outputs: (out_start, output_ptrs.len() as u32),
            connected,
            fills: (fill_start, fills.len() as u32),
            params: (param_start, param_ptrs.len() as u32),
        });
        op_of_node[v] = (ops.len() - 1) as u32;
        op_of_node[v]
    };

    // Taps, sorted by key.
    let mut tap_order: Vec<usize> = (0..taps.len()).collect();
    tap_order.sort_by_key(|&i| taps[i].key);
    let mut tap_index = vec![0u32; taps.len()];
    for (sorted, &original) in tap_order.iter().enumerate() {
        tap_index[original] = sorted as u32;
    }
    let final_taps: Vec<Tap> = tap_order
        .iter()
        .map(|&i| Tap {
            key: taps[i].key,
            input: taps[i].input.map_or(zero_ptr, |b| buf_ptrs[b] as *const f32),
            output: buf_ptrs[taps[i].output],
            state: 0.0,
        })
        .collect();

    let mut loop_ops = Vec::new();
    let mut loop_taps = Vec::new();
    let mut final_steps = Vec::with_capacity(steps.len());
    for step in steps {
        final_steps.push(match step {
            StepSpec::Module(v) => Step::Module(make_op(v)),
            StepSpec::Tap(t) => Step::Tap(tap_index[t]),
            StepSpec::Loop { modules, taps } => {
                let ops_start = loop_ops.len() as u32;
                for v in modules {
                    let op = make_op(v);
                    loop_ops.push(op);
                }
                let taps_start = loop_taps.len() as u32;
                loop_taps.extend(taps.iter().map(|&t| tap_index[t]));
                Step::Loop {
                    ops: (ops_start, loop_ops.len() as u32),
                    taps: (taps_start, loop_taps.len() as u32),
                }
            }
        });
    }

    let outputs = output_nodes
        .iter()
        .filter_map(|&v| {
            let left = graph.cables.get(&InputRef {
                module: ids[v],
                port: 0,
            });
            let right = graph.cables.get(&InputRef {
                module: ids[v],
                port: 1,
            });
            let ptr = |from: &OutputRef| buf_ptrs[source_buffer(*from)] as *const f32;
            match (left, right) {
                (Some(l), Some(r)) => Some(OutputOp {
                    left: ptr(l),
                    right: ptr(r),
                }),
                // Mono into one side plays on both sides.
                (Some(m), None) | (None, Some(m)) => Some(OutputOp {
                    left: ptr(m),
                    right: ptr(m),
                }),
                (None, None) => None,
            }
        })
        .collect();

    let probes = probes
        .iter()
        .filter_map(|(&id, &(from, kind))| {
            let buffer = *out_buf.get(*index_of.get(&from.module)?)?.get(from.port)?;
            Some(ProbeOp {
                id,
                buffer: buf_ptrs[buffer] as *const f32,
                kind,
            })
        })
        .collect();

    let schedule = Schedule {
        _buffers: buffers,
        _zero: zero,
        input_ptrs,
        output_ptrs,
        ops,
        taps: final_taps,
        loop_ops,
        loop_taps,
        steps: final_steps,
        outputs,
        probes,
        transport,
        fills,
        assignments: compiled
            .iter()
            .map(|assignment| AssignmentRuntime {
                id: assignment.id,
                depth: assignment.depth,
                enabled: assignment.enabled,
                polarity: assignment.polarity,
                source_polarity: assignment.source_polarity,
            })
            .collect(),
        param_ptrs,
        midi_maps: compile_midi_maps(graph, midi_synced),
    };
    (schedule, report)
}

/// The graph's MIDI mappings, sorted by control so the engine can find a
/// control change's mapping with a binary search.
fn compile_midi_maps(graph: &Graph, synced: &BTreeMap<MidiMappingId, u32>) -> Vec<MidiMapOp> {
    let mut maps: Vec<MidiMapOp> = graph
        .midi_mappings
        .iter()
        .filter_map(|(&id, mapping)| {
            let node = graph.nodes.get(&mapping.target)?;
            Some(MidiMapOp {
                control: mapping.control,
                id,
                synced: synced.get(&id).copied(),
                slot: node.slot? as u32,
                param: u16::try_from(mapping.param).ok()?,
                spec: node.descriptor.params.get(mapping.param)?,
            })
        })
        .collect();
    maps.sort_by_key(|map| map.control);
    maps
}

/// Tarjan's strongly connected components. Returns SCCs in reverse
/// topological order, each sorted.
fn tarjan(succ: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct State<'a> {
        succ: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }

    impl State<'_> {
        fn visit(&mut self, v: usize) {
            self.index[v] = Some(self.next);
            self.low[v] = self.next;
            self.next += 1;
            self.stack.push(v);
            self.on_stack[v] = true;
            for i in 0..self.succ[v].len() {
                let w = self.succ[v][i];
                match self.index[w] {
                    None => {
                        self.visit(w);
                        self.low[v] = self.low[v].min(self.low[w]);
                    }
                    Some(wi) if self.on_stack[w] => self.low[v] = self.low[v].min(wi),
                    Some(_) => {}
                }
            }
            if Some(self.low[v]) == self.index[v] {
                let mut component = Vec::new();
                loop {
                    let w = self.stack.pop().expect("tarjan stack");
                    self.on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                component.sort_unstable();
                self.out.push(component);
            }
        }
    }

    let n = succ.len();
    let mut state = State {
        succ,
        index: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for v in 0..n {
        if state.index[v].is_none() {
            state.visit(v);
        }
    }
    state.out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A port for test descriptors.
    const fn port(name: &'static str, signal: crate::SignalType) -> crate::PortSpec {
        crate::PortSpec {
            name,
            label: name,
            signal,
            description: "A test port.",
            default: crate::PortDefault::Value(0.0),
            polarity: match signal {
                crate::SignalType::Gate | crate::SignalType::Phase => crate::Polarity::Unipolar,
                crate::SignalType::Audio | crate::SignalType::Pitch | crate::SignalType::Modulation => {
                    crate::Polarity::Bipolar
                }
            },
        }
    }

    #[test]
    fn tarjan_finds_cycles_in_topological_order() {
        // 0 -> 1 -> 2 -> 1, 2 -> 3
        let succ = vec![vec![1], vec![2], vec![1, 3], vec![]];
        let mut sccs = tarjan(&succ);
        sccs.reverse();
        assert_eq!(sccs, vec![vec![0], vec![1, 2], vec![3]]);
    }

    struct Silent;

    impl crate::Module for Silent {
        fn process(&mut self, _: &crate::ProcessContext<'_>, _: &mut crate::PortIo<'_>) {}
    }

    static FOLLOWER: crate::ModuleDescriptor = crate::ModuleDescriptor {
        id: "test.follower",
        version: 1,
        name: "Follower",
        category: "Test",
        description: "Inputs with every kind of default.",
        inputs: &[
            port("bar", crate::SignalType::Phase).with_default(PortDefault::BarPhase),
            port("bar2", crate::SignalType::Phase).with_default(PortDefault::BarPhase),
            port("half", crate::SignalType::Modulation).with_default(PortDefault::Value(0.5)),
            port("zero", crate::SignalType::Modulation),
        ],
        outputs: &[port("out", crate::SignalType::Modulation)],
        params: &[],
        role: ModuleRole::Dsp(|_| Box::new(Silent)),
        panel: crate::Panel::EMPTY,
        displays: &[],
    };

    fn followers(count: u32) -> Graph {
        let mut graph = Graph::default();
        for i in 0..count {
            graph.nodes.insert(
                ModuleId(i + 1),
                crate::Node {
                    descriptor: &FOLLOWER,
                    params: Vec::new(),
                    slot: Some(i as usize),
                },
            );
        }
        graph
    }

    fn input_ptr(schedule: &Schedule, op: usize, port: usize) -> *const f32 {
        schedule.input_ptrs[schedule.ops[op].inputs.0 as usize + port]
    }

    #[test]
    fn inputs_that_follow_the_same_default_share_one_buffer() {
        let schedule = compile(&followers(100), &BTreeMap::new(), &BTreeMap::new(), 48_000.0, 64).0;
        // 100 outputs, one bar phase buffer, one buffer holding 0.5.
        assert_eq!(schedule._buffers.len(), 102);
        let bar = schedule.transport.bar_phase.expect("bar phase is used") as *const f32;
        assert!(schedule.transport.beat_phase.is_none());
        assert!(schedule.transport.playing.is_none());
        let half = input_ptr(&schedule, 0, 2);
        // SAFETY: `half` points at a schedule buffer of 64 floats.
        assert_eq!(unsafe { std::slice::from_raw_parts(half, 64) }, &[0.5; 64]);
        for op in 0..100 {
            assert_eq!(input_ptr(&schedule, op, 0), bar);
            assert_eq!(input_ptr(&schedule, op, 1), bar);
            assert_eq!(input_ptr(&schedule, op, 2), half);
            assert_eq!(input_ptr(&schedule, op, 3), schedule._zero.as_ptr());
            assert_eq!(schedule.ops[op].connected, 0);
        }
    }

    #[test]
    fn a_cable_replaces_the_default() {
        let mut graph = followers(2);
        let from = OutputRef {
            module: ModuleId(2),
            port: 0,
        };
        let to = InputRef {
            module: ModuleId(1),
            port: 0,
        };
        graph.cables.insert(to, from);
        let schedule = compile(&graph, &BTreeMap::new(), &BTreeMap::new(), 48_000.0, 64).0;
        // Module 2 runs first; module 1 reads module 2's output.
        let source = schedule.output_ptrs[schedule.ops[0].outputs.0 as usize] as *const f32;
        assert_eq!(input_ptr(&schedule, 1, 0), source);
        assert_eq!(schedule.ops[1].connected, 1);
        // Its second bar input and module 2's still follow the transport.
        let bar = schedule.transport.bar_phase.expect("still used") as *const f32;
        assert_eq!(input_ptr(&schedule, 1, 1), bar);
        assert_eq!(input_ptr(&schedule, 0, 0), bar);
    }
}
