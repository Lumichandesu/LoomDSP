//! LoomDSP engine core.
//!
//! The crate is split along the thread boundary described in ADR-0001:
//!
//! - [`Controller`] lives on a control thread (UI, CLI, patch loader). It owns
//!   the editable [`Graph`], compiles it into an execution schedule, and sends
//!   changes to the engine. It may allocate and block.
//! - [`Engine`] lives on the audio thread. It runs the current schedule and
//!   applies changes at block boundaries. It never allocates, frees, locks,
//!   or waits.
//!
//! They talk through bounded single-producer, single-consumer queues.
//!
//! The core has no threads of its own and no audio or file I/O, so it builds
//! for any target, including `wasm32-unknown-unknown` (NFR-PORT-01, 02).

#![warn(missing_docs)]

mod compile;
mod controller;
mod descriptor;
mod display;
mod engine;
mod error;
mod graph;
mod midi;
mod midi_map;
mod module;
mod registry;
pub mod signal;
mod telemetry;
pub mod transport;

pub use compile::{CompileReport, FeedbackLoop};
pub use controller::{Controller, EngineParts, MidiSender, create_engine};
pub use descriptor::{
    MAX_DISPLAYS, MissingText, ModuleConfig, ModuleDescriptor, ModuleRole, Panel, ParamMapping, ParamOptions,
    ParamSpec, Polarity, PortDefault, PortSpec, SpecKind, Undocumented, Widget, WidgetKind, assert_documented,
};
pub use display::{DisplayReader, DisplaySource};
pub use engine::{Engine, EngineConfig, EngineStats};
pub use error::LoomError;
pub use graph::{Assignment, AssignmentId, Graph, InputRef, ModuleId, Node, OutputRef};
pub use midi::{MidiEvent, MidiMessage};
pub use midi_map::{ControlChangeSeen, MidiControl, MidiMapping, MidiMappingId, MidiReader, control_value};
pub use module::{Module, PortIo, ProcessContext, testing};
pub use registry::{ModuleKind, Registry, RegistryNote};
pub use signal::SignalType;
pub use telemetry::{MeterReading, ProbeId, ProbeKind, SCOPE_CHUNK, ScopeChunk, Telemetry};
pub use transport::{TimeSignature, TransportInfo, TransportSettings};
