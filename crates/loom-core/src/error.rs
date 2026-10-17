use crate::graph::{AssignmentId, ModuleId};
use crate::midi_map::MidiControl;
use crate::telemetry::ProbeId;
use crate::transport::TimeSignature;

/// Errors from the control side of the engine.
#[derive(Debug, thiserror::Error)]
pub enum LoomError {
    /// A kind string could not be parsed.
    #[error("invalid module kind `{0}` (expected `id@version`)")]
    InvalidKind(String),
    /// No module type with this ID is registered.
    #[error("unknown module kind `{0}`")]
    UnknownModuleKind(String),
    /// The patch needs a newer module version than this build has.
    #[error("module `{kind}` is newer than this version of LoomDSP supports (up to version {supported})")]
    UnsupportedModuleVersion {
        /// The requested kind.
        kind: String,
        /// Highest supported version.
        supported: u32,
    },
    /// A module descriptor is invalid.
    #[error("invalid module descriptor `{id}`: {reason}")]
    InvalidDescriptor {
        /// Module ID.
        id: String,
        /// What is wrong.
        reason: String,
    },
    /// No module with this ID in the graph.
    #[error("no module with id {0}")]
    UnknownModule(ModuleId),
    /// A module ID is already used.
    #[error("module id {0} is already in use")]
    DuplicateModuleId(ModuleId),
    /// A port name or index does not exist.
    #[error("module {module} has no {direction} port `{port}`")]
    NoSuchPort {
        /// The module.
        module: ModuleId,
        /// "input" or "output".
        direction: &'static str,
        /// The port name or index.
        port: String,
    },
    /// No modulation assignment with this id.
    #[error("no modulation assignment {0}")]
    UnknownAssignment(AssignmentId),
    /// A parameter name or index does not exist.
    #[error("module {module} has no parameter `{param}`")]
    NoSuchParam {
        /// The module.
        module: ModuleId,
        /// The parameter name or index.
        param: String,
    },
    /// A MIDI channel or controller number is out of range.
    #[error("invalid MIDI control: channel {} and controller {} (channels are 0 to 15, controllers 0 to 127)", .0.channel, .0.controller)]
    InvalidMidiControl(MidiControl),
    /// The patch has more modules than the engine was configured for.
    #[error("too many modules (limit is {0})")]
    TooManyModules(usize),
    /// No probe with this ID.
    #[error("no {0}")]
    UnknownProbe(ProbeId),
    /// A time signature is out of range. The transport did not change.
    #[error("invalid time signature {0} (the top number must be 1 to 32, the bottom 1, 2, 4, 8, or 16)")]
    InvalidTimeSignature(TimeSignature),
    /// The command queue to the audio thread is full. Try again after the
    /// engine has processed a block.
    #[error("the engine command queue is full; is the audio engine running?")]
    QueueFull,
}
