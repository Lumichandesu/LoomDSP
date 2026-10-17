//! The editable patch graph, owned by the control thread.

use crate::descriptor::{ModuleDescriptor, ModuleRole, Polarity};
use crate::midi_map::{MidiControl, MidiMapping, MidiMappingId};
use std::collections::BTreeMap;
use std::fmt;

/// Stable identifier of a module in a patch. Saved in patch files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(pub u32);

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// An output port: module plus output index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputRef {
    /// The module.
    pub module: ModuleId,
    /// Output index.
    pub port: usize,
}

/// An input port: module plus input index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputRef {
    /// The module.
    pub module: ModuleId,
    /// Input index.
    pub port: usize,
}

/// Stable identifier of one modulation assignment. Saved only for the
/// session: patch files identify an assignment by its source and target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssignmentId(pub u32);

impl fmt::Display for AssignmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One modulation assignment: an output moving a parameter, without a cable.
///
/// Spec: `docs/design/modulation.md`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Assignment {
    /// The output that drives the parameter.
    pub source: OutputRef,
    /// The module whose parameter is moved.
    pub target: ModuleId,
    /// Parameter index on [`Self::target`].
    pub param: usize,
    /// Share of the control's full range, from -1 to 1.
    pub depth: f32,
    /// Forces a polarity. `None` follows the source output.
    pub polarity: Option<Polarity>,
    /// False skips this assignment without removing the scheduling edge.
    pub enabled: bool,
}

/// A module in the graph.
#[derive(Debug)]
pub struct Node {
    /// The module type.
    pub descriptor: &'static ModuleDescriptor,
    /// Current parameter values, one per [`ModuleDescriptor::params`].
    pub params: Vec<f32>,
    /// Engine slot holding the instance. `None` for engine-handled roles.
    pub(crate) slot: Option<usize>,
}

impl Node {
    /// True for modules the engine runs itself (unit delay, audio output).
    pub fn is_intrinsic(&self) -> bool {
        !matches!(self.descriptor.role, ModuleRole::Dsp(_))
    }
}

/// Modules, cables, modulation assignments, and MIDI mappings. Each input has
/// at most one cable.
#[derive(Debug, Default)]
pub struct Graph {
    pub(crate) nodes: BTreeMap<ModuleId, Node>,
    /// Cables, keyed by the input they feed.
    pub(crate) cables: BTreeMap<InputRef, OutputRef>,
    /// Modulation assignments, in id order. That order is the order their
    /// depths are added.
    pub(crate) assignments: BTreeMap<AssignmentId, Assignment>,
    /// MIDI mappings. A parameter has at most one, and so does a control.
    pub(crate) midi_mappings: BTreeMap<MidiMappingId, MidiMapping>,
}

impl Graph {
    /// All modules, sorted by ID.
    pub fn nodes(&self) -> impl Iterator<Item = (ModuleId, &Node)> {
        self.nodes.iter().map(|(id, node)| (*id, node))
    }

    /// One module.
    pub fn node(&self, id: ModuleId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// All cables as `(from, to)`, sorted by input.
    pub fn cables(&self) -> impl Iterator<Item = (OutputRef, InputRef)> + '_ {
        self.cables.iter().map(|(to, from)| (*from, *to))
    }

    /// The cable feeding an input, if any.
    pub fn source_of(&self, input: InputRef) -> Option<OutputRef> {
        self.cables.get(&input).copied()
    }

    /// All modulation assignments, sorted by id.
    pub fn assignments(&self) -> impl Iterator<Item = (AssignmentId, &Assignment)> {
        self.assignments.iter().map(|(id, assignment)| (*id, assignment))
    }

    /// One modulation assignment.
    pub fn assignment(&self, id: AssignmentId) -> Option<&Assignment> {
        self.assignments.get(&id)
    }

    /// All MIDI mappings, sorted by id.
    pub fn midi_mappings(&self) -> impl Iterator<Item = (MidiMappingId, &MidiMapping)> {
        self.midi_mappings.iter().map(|(id, mapping)| (*id, mapping))
    }

    /// The MIDI mapping on a parameter, if any.
    pub fn midi_mapping_of(&self, module: ModuleId, param: usize) -> Option<(MidiMappingId, &MidiMapping)> {
        self.midi_mappings()
            .find(|(_, mapping)| mapping.target == module && mapping.param == param)
    }

    /// The MIDI mapping on a hardware control, if any.
    pub fn midi_mapping_on(&self, control: MidiControl) -> Option<(MidiMappingId, &MidiMapping)> {
        self.midi_mappings().find(|(_, mapping)| mapping.control == control)
    }

    /// Number of modules.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when there are no modules.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
