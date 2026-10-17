//! The LoomDSP patch file format (ADR-0008).
//!
//! Patches are RON (Rusty Object Notation) text files. Why RON: it keeps
//! Rust's data model (structs, enums, tuples) exactly, allows comments, and
//! diffs well in git.
//!
//! ```ron
//! Patch(
//!     version: 1,
//!     name: "Simple sine",
//!     modules: [
//!         (id: 1, kind: "loom.midi.note@1"),
//!         (id: 2, kind: "loom.phase.generator@1"),
//!         (id: 3, kind: "loom.osc.sine@1"),
//!         (id: 4, kind: "loom.io.audio-out@1"),
//!     ],
//!     cables: [
//!         (from: (1, "pitch"), to: (2, "pitch")),
//!         (from: (2, "phase"), to: (3, "phase")),
//!         (from: (3, "out"), to: (4, "left")),
//!     ],
//! )
//! ```
//!
//! Saved files are canonical: modules sorted by ID, cables sorted by input,
//! parameters sorted by name, MIDI mappings sorted by target. This keeps git diffs small. Modulation keeps
//! the order the assignments were made in, because the engine adds several
//! assignments on one parameter in that order, and float sums depend on it.

#![warn(missing_docs)]

use loom_core::{Controller, Graph, InputRef, LoomError, MidiControl, ModuleId, OutputRef, Polarity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The patch format version this build writes.
pub const CURRENT_VERSION: u32 = 1;

/// A patch file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "Patch")]
pub struct Patch {
    /// Format version. See [`CURRENT_VERSION`].
    pub version: u32,
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// Free text for the patch author.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Modules.
    #[serde(default)]
    pub modules: Vec<PatchModule>,
    /// Cables.
    #[serde(default)]
    pub cables: Vec<PatchCable>,
    /// Modulation assignments. Omitted when empty, so older files still load.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modulation: Vec<PatchModulation>,
    /// MIDI mappings (MIDI learn). Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub midi: Vec<PatchMidiMapping>,
}

/// A module in a patch file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchModule {
    /// Stable module ID within the patch.
    pub id: u32,
    /// Module type as `"id@version"`.
    pub kind: String,
    /// Optional user label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Parameter values by name. Missing parameters use their defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, f32>,
    /// Position on the canvas, for the editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<(f32, f32)>,
}

/// A cable in a patch file: `(module id, port name)` to `(module id, port name)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchCable {
    /// Output side.
    pub from: (u32, String),
    /// Input side.
    pub to: (u32, String),
}

/// A modulation assignment in a patch file.
///
/// `source` is `(module id, output name)`. `target` is `(module id, parameter name)`.
/// `polarity` is omitted when the assignment follows the source output.
/// `enabled: false` is written only when the assignment is off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchModulation {
    /// Output that drives the parameter.
    pub source: (u32, String),
    /// Parameter that is moved.
    pub target: (u32, String),
    /// Share of the control's range, from -1 to 1.
    pub depth: f32,
    /// Forces a polarity. Absent means follow the source output.
    /// Written as `Unipolar` or `Bipolar`, not as `Some(...)`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_polarity",
        deserialize_with = "deserialize_polarity"
    )]
    pub polarity: Option<PatchPolarity>,
    /// False skips the assignment. Missing means on.
    #[serde(default = "enabled_by_default", skip_serializing_if = "is_enabled")]
    pub enabled: bool,
}

/// A MIDI mapping in a patch file: a hardware control that sets a parameter.
///
/// `target` is `(module id, parameter name)`. `channel` is 0 to 15, as in
/// MIDI. Spec: `docs/design/midi-learn.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchMidiMapping {
    /// Parameter that is set.
    pub target: (u32, String),
    /// MIDI channel, 0 to 15.
    pub channel: u8,
    /// Controller number, 0 to 127.
    pub controller: u8,
}

/// Polarity stored in a patch file. Matches [`Polarity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatchPolarity {
    /// Nominally 0 to 1.
    Unipolar,
    /// Nominally -1 to 1.
    Bipolar,
}

fn enabled_by_default() -> bool {
    true
}

fn is_enabled(enabled: &bool) -> bool {
    *enabled
}

fn serialize_polarity<S>(value: &Option<PatchPolarity>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(polarity) => polarity.serialize(serializer),
        None => serializer.serialize_none(),
    }
}

fn deserialize_polarity<'de, D>(deserializer: D) -> Result<Option<PatchPolarity>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    PatchPolarity::deserialize(deserializer).map(Some)
}

impl From<Polarity> for PatchPolarity {
    fn from(polarity: Polarity) -> Self {
        match polarity {
            Polarity::Unipolar => Self::Unipolar,
            Polarity::Bipolar => Self::Bipolar,
        }
    }
}

impl From<PatchPolarity> for Polarity {
    fn from(polarity: PatchPolarity) -> Self {
        match polarity {
            PatchPolarity::Unipolar => Self::Unipolar,
            PatchPolarity::Bipolar => Self::Bipolar,
        }
    }
}

/// Patch loading and saving errors.
#[derive(Debug, thiserror::Error)]
pub enum PatchError {
    /// The file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// File path.
        path: String,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The text is not a valid patch.
    #[error("invalid patch: {0}")]
    Parse(#[from] ron::error::SpannedError),
    /// Serialisation failed.
    #[error("could not write patch: {0}")]
    Write(#[from] ron::Error),
    /// The file was written by a newer LoomDSP.
    #[error("patch format version {found} is newer than this build supports ({supported})")]
    TooNew {
        /// Version in the file.
        found: u32,
        /// Highest supported version.
        supported: u32,
    },
    /// The patch refers to something the engine rejects.
    #[error(transparent)]
    Engine(#[from] LoomError),
    /// Two MIDI mappings share a parameter or a control. Learning never makes
    /// that, so the file was edited by hand.
    #[error("two MIDI mappings use {what}")]
    DuplicateMidiMapping {
        /// The shared parameter or control.
        what: String,
    },
    /// A parameter name does not exist on a module.
    #[error("module {module}: unknown parameter `{param}`")]
    UnknownParam {
        /// Module ID in the patch.
        module: u32,
        /// Parameter name.
        param: String,
    },
}

impl Patch {
    /// An empty patch.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: CURRENT_VERSION,
            name: name.into(),
            description: String::new(),
            modules: Vec::new(),
            cables: Vec::new(),
            modulation: Vec::new(),
            midi: Vec::new(),
        }
    }

    /// Parses RON text and migrates it to the current version.
    pub fn from_ron(text: &str) -> Result<Self, PatchError> {
        let patch: Patch = ron::from_str(text)?;
        patch.migrate()
    }

    /// Reads a patch file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PatchError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| PatchError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_ron(&text)
    }

    /// Canonical RON text.
    pub fn to_ron(&self) -> Result<String, PatchError> {
        let mut canonical = self.clone();
        canonical.canonicalise();
        let config = ron::ser::PrettyConfig::new()
            .struct_names(true)
            .depth_limit(2)
            .separate_tuple_members(false)
            .indentor("    ");
        let mut text = ron::ser::to_string_pretty(&canonical, config)?;
        text.push('\n');
        Ok(text)
    }

    /// Writes a patch file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PatchError> {
        let path = path.as_ref();
        std::fs::write(path, self.to_ron()?).map_err(|source| PatchError::Io {
            path: path.display().to_string(),
            source,
        })
    }

    /// Sorts modules, cables, and parameters into canonical order.
    ///
    /// Modulation is not sorted. Loading gives assignments ids in file order,
    /// and the engine adds them in id order, so a reloaded patch sums its
    /// assignments the same way as the patch that was saved.
    pub fn canonicalise(&mut self) {
        self.modules.sort_by_key(|m| m.id);
        self.cables.sort_by(|a, b| (a.to.0, &a.to.1).cmp(&(b.to.0, &b.to.1)));
        self.midi
            .sort_by(|a, b| (a.target.0, &a.target.1).cmp(&(b.target.0, &b.target.1)));
    }

    /// Upgrades an older patch to [`CURRENT_VERSION`].
    ///
    /// Each format change adds one step here, plus a test that loads a
    /// fixture saved in the old format (FR-FILE-02).
    pub fn migrate(mut self) -> Result<Self, PatchError> {
        if self.version > CURRENT_VERSION {
            return Err(PatchError::TooNew {
                found: self.version,
                supported: CURRENT_VERSION,
            });
        }
        // Version 0 was never released. Treat it as version 1.
        if self.version == 0 {
            self.version = 1;
        }
        Ok(self)
    }

    /// Adds every module and cable to a controller and commits.
    ///
    /// The controller should be empty. Module IDs from the file are kept.
    pub fn load_into(&self, controller: &mut Controller) -> Result<(), PatchError> {
        for module in &self.modules {
            let id = ModuleId(module.id);
            controller.add_module_with_id(id, &module.kind)?;
            for (name, &value) in &module.params {
                controller
                    .set_param_named(id, name, value)
                    .map_err(|error| match error {
                        LoomError::NoSuchParam { .. } => PatchError::UnknownParam {
                            module: module.id,
                            param: name.clone(),
                        },
                        other => PatchError::Engine(other),
                    })?;
            }
        }
        for cable in &self.cables {
            controller.connect_named(ModuleId(cable.from.0), &cable.from.1, ModuleId(cable.to.0), &cable.to.1)?;
        }
        for assignment in &self.modulation {
            let id = controller.add_assignment_named(
                ModuleId(assignment.source.0),
                &assignment.source.1,
                ModuleId(assignment.target.0),
                &assignment.target.1,
                assignment.depth,
            )?;
            if let Some(polarity) = assignment.polarity {
                controller.set_assignment_polarity(id, Some(polarity.into()))?;
            }
            if !assignment.enabled {
                controller.set_assignment_enabled(id, false)?;
            }
        }
        self.check_midi()?;
        for mapping in &self.midi {
            let control = MidiControl {
                channel: mapping.channel,
                controller: mapping.controller,
            };
            controller
                .map_midi_named(control, ModuleId(mapping.target.0), &mapping.target.1)
                .map_err(|error| match error {
                    LoomError::NoSuchParam { .. } => PatchError::UnknownParam {
                        module: mapping.target.0,
                        param: mapping.target.1.clone(),
                    },
                    other => PatchError::Engine(other),
                })?;
        }
        controller.commit()?;
        Ok(())
    }

    /// Refuses two MIDI mappings on one parameter or one control. The
    /// controller would quietly keep the last one, which hides the mistake.
    fn check_midi(&self) -> Result<(), PatchError> {
        let mut targets = std::collections::BTreeSet::new();
        let mut controls = std::collections::BTreeSet::new();
        for mapping in &self.midi {
            if !targets.insert(&mapping.target) {
                return Err(PatchError::DuplicateMidiMapping {
                    what: format!("parameter `{}` of module {}", mapping.target.1, mapping.target.0),
                });
            }
            if !controls.insert((mapping.channel, mapping.controller)) {
                return Err(PatchError::DuplicateMidiMapping {
                    what: format!("controller {} on channel {}", mapping.controller, mapping.channel),
                });
            }
        }
        Ok(())
    }

    /// Builds a patch from a graph. Only non-default parameters are saved.
    pub fn from_graph(name: impl Into<String>, graph: &Graph) -> Self {
        let mut patch = Patch::new(name);
        for (id, node) in graph.nodes() {
            let params = node
                .descriptor
                .params
                .iter()
                .zip(&node.params)
                .filter(|(spec, value)| **value != spec.default)
                .map(|(spec, value)| (spec.name.to_string(), *value))
                .collect();
            patch.modules.push(PatchModule {
                id: id.0,
                kind: node.descriptor.kind(),
                label: None,
                params,
                position: None,
            });
        }
        for (from, to) in graph.cables() {
            patch.cables.push(PatchCable {
                from: (from.module.0, port_name(graph, from)),
                to: (to.module.0, input_name(graph, to)),
            });
        }
        for (_, assignment) in graph.assignments() {
            patch.modulation.push(PatchModulation {
                source: (assignment.source.module.0, port_name(graph, assignment.source)),
                target: (
                    assignment.target.0,
                    param_name(graph, assignment.target, assignment.param),
                ),
                depth: assignment.depth,
                polarity: assignment.polarity.map(PatchPolarity::from),
                enabled: assignment.enabled,
            });
        }
        for (_, mapping) in graph.midi_mappings() {
            patch.midi.push(PatchMidiMapping {
                target: (mapping.target.0, param_name(graph, mapping.target, mapping.param)),
                channel: mapping.control.channel,
                controller: mapping.control.controller,
            });
        }
        patch.canonicalise();
        patch
    }
}

fn port_name(graph: &Graph, output: OutputRef) -> String {
    graph
        .node(output.module)
        .map(|n| n.descriptor.outputs[output.port].name.to_string())
        .unwrap_or_default()
}

fn input_name(graph: &Graph, input: InputRef) -> String {
    graph
        .node(input.module)
        .map(|n| n.descriptor.inputs[input.port].name.to_string())
        .unwrap_or_default()
}

fn param_name(graph: &Graph, module: ModuleId, index: usize) -> String {
    graph
        .node(module)
        .and_then(|n| n.descriptor.params.get(index))
        .map(|param| param.name.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use loom_core::{EngineConfig, create_engine};

    const SIMPLE: &str = r#"
// Comments are allowed.
Patch(
    version: 1,
    name: "Simple sine",
    modules: [
        (id: 3, kind: "loom.osc.sine@1"),
        (id: 1, kind: "loom.midi.note@1"),
        (id: 2, kind: "loom.phase.generator@1", params: {"tune": 12.0}),
        (id: 4, kind: "loom.io.audio-out@1"),
    ],
    cables: [
        (from: (2, "phase"), to: (3, "phase")),
        (from: (1, "pitch"), to: (2, "pitch")),
        (from: (3, "out"), to: (4, "left")),
    ],
)
"#;

    #[test]
    fn loads_and_round_trips() {
        let patch = Patch::from_ron(SIMPLE).unwrap();
        let mut parts = create_engine(loom_modules::registry(), EngineConfig::default());
        patch.load_into(&mut parts.controller).unwrap();

        let saved = Patch::from_graph("Simple sine", parts.controller.graph());
        let mut expected = patch.clone();
        expected.canonicalise();
        assert_eq!(saved, expected);

        // Canonical text is stable across a parse/print cycle.
        let text = saved.to_ron().unwrap();
        assert_eq!(Patch::from_ron(&text).unwrap().to_ron().unwrap(), text);
    }

    #[test]
    fn rejects_newer_versions() {
        let text = SIMPLE.replace("version: 1", "version: 99");
        assert!(matches!(
            Patch::from_ron(&text),
            Err(PatchError::TooNew { found: 99, .. })
        ));
    }

    const MODULATED: &str = r#"
Patch(
    version: 1,
    name: "Modulated",
    modules: [
        (id: 1, kind: "loom.math.constant@1", params: {"value": 1.0}),
        (id: 2, kind: "loom.env.adsr@1"),
        (id: 3, kind: "loom.filter.svf@1"),
    ],
    modulation: [
        (source: (2, "out"), target: (3, "resonance"), depth: 0.4, polarity: Unipolar, enabled: false),
        (source: (1, "out"), target: (3, "cutoff"), depth: 0.25),
    ],
)
"#;

    #[test]
    fn modulation_round_trips_and_old_patches_load() {
        let old = Patch::from_ron(SIMPLE).unwrap();
        assert!(old.modulation.is_empty());
        let mut parts = create_engine(loom_modules::registry(), EngineConfig::default());
        old.load_into(&mut parts.controller).unwrap();
        let saved = Patch::from_graph("Simple sine", parts.controller.graph());
        assert!(saved.modulation.is_empty());
        assert!(!saved.to_ron().unwrap().contains("modulation"));

        let patch = Patch::from_ron(MODULATED).unwrap();
        let mut parts = create_engine(loom_modules::registry(), EngineConfig::default());
        patch.load_into(&mut parts.controller).unwrap();
        let saved = Patch::from_graph("Modulated", parts.controller.graph());
        let mut expected = patch.clone();
        expected.canonicalise();
        assert_eq!(saved, expected);
        // File order is kept, not sorted by target.
        assert_eq!(saved.modulation[0].target.1, "resonance");
        assert_eq!(saved.modulation[0].polarity, Some(PatchPolarity::Unipolar));
        assert!(!saved.modulation[0].enabled);
        assert_eq!(saved.modulation[1].target.1, "cutoff");
        assert!(saved.modulation[1].polarity.is_none());
        assert!(saved.modulation[1].enabled);

        let text = saved.to_ron().unwrap();
        assert!(text.contains("modulation:"));
        assert!(text.contains("enabled: false"));
        assert!(!text.contains("enabled: true"));
        assert_eq!(Patch::from_ron(&text).unwrap().to_ron().unwrap(), text);
    }

    /// The engine adds assignments on one parameter in id order. Float sums
    /// depend on order, so a reloaded patch must keep it (review on #133).
    #[test]
    fn assignment_order_survives_save_and_load() {
        let mut parts = create_engine(loom_modules::registry(), EngineConfig::default());
        let controller = &mut parts.controller;
        let low = controller.add_module("loom.math.constant").unwrap();
        let high = controller.add_module("loom.math.constant").unwrap();
        let filter = controller.add_module("loom.filter.svf").unwrap();
        // Made from the higher module id first, so sorting by source would
        // swap them.
        controller
            .add_assignment_named(high, "out", filter, "cutoff", 0.5)
            .unwrap();
        controller
            .add_assignment_named(low, "out", filter, "cutoff", -0.25)
            .unwrap();
        let order = |graph: &Graph| -> Vec<(OutputRef, f32)> {
            graph.assignments().map(|(_, a)| (a.source, a.depth)).collect()
        };
        let before = order(controller.graph());

        let text = Patch::from_graph("Order", controller.graph()).to_ron().unwrap();
        let mut reloaded = create_engine(loom_modules::registry(), EngineConfig::default());
        Patch::from_ron(&text)
            .unwrap()
            .load_into(&mut reloaded.controller)
            .unwrap();
        assert_eq!(order(reloaded.controller.graph()), before);
    }

    const MAPPED: &str = r#"
Patch(
    version: 1,
    name: "Mapped",
    modules: [
        (id: 1, kind: "loom.filter.svf@1"),
    ],
    midi: [
        (target: (1, "resonance"), channel: 0, controller: 71),
        (target: (1, "cutoff"), channel: 2, controller: 74),
    ],
)
"#;

    #[test]
    fn midi_mappings_round_trip_and_old_patches_load() {
        let old = Patch::from_ron(SIMPLE).unwrap();
        assert!(old.midi.is_empty());
        let mut parts = create_engine(loom_modules::registry(), EngineConfig::default());
        old.load_into(&mut parts.controller).unwrap();
        assert_eq!(parts.controller.graph().midi_mappings().count(), 0);
        let text = Patch::from_graph("Simple sine", parts.controller.graph())
            .to_ron()
            .unwrap();
        assert!(!text.contains("midi:"));

        let patch = Patch::from_ron(MAPPED).unwrap();
        let mut parts = create_engine(loom_modules::registry(), EngineConfig::default());
        patch.load_into(&mut parts.controller).unwrap();
        let saved = Patch::from_graph("Mapped", parts.controller.graph());
        let mut expected = patch.clone();
        expected.canonicalise();
        assert_eq!(saved, expected);
        // Sorted by target.
        assert_eq!(saved.midi[0].target.1, "cutoff");
        assert_eq!((saved.midi[0].channel, saved.midi[0].controller), (2, 74));
        let text = saved.to_ron().unwrap();
        assert_eq!(Patch::from_ron(&text).unwrap().to_ron().unwrap(), text);
    }

    #[test]
    fn rejects_bad_midi_mappings() {
        let load = |text: &str| {
            let mut parts = create_engine(loom_modules::registry(), EngineConfig::default());
            Patch::from_ron(text).unwrap().load_into(&mut parts.controller)
        };
        let same_param = MAPPED.replace("\"resonance\"", "\"cutoff\"");
        assert!(matches!(
            load(&same_param),
            Err(PatchError::DuplicateMidiMapping { .. })
        ));
        let same_control = MAPPED.replace("channel: 2, controller: 74", "channel: 0, controller: 71");
        assert!(matches!(
            load(&same_control),
            Err(PatchError::DuplicateMidiMapping { .. })
        ));
        let unknown = MAPPED.replace("\"cutoff\"", "\"cut\"");
        assert!(matches!(
            load(&unknown),
            Err(PatchError::UnknownParam { module: 1, .. })
        ));
        let bad_channel = MAPPED.replace("channel: 2", "channel: 16");
        assert!(matches!(
            load(&bad_channel),
            Err(PatchError::Engine(LoomError::InvalidMidiControl(_)))
        ));
    }

    #[test]
    fn rejects_unknown_params() {
        let text = SIMPLE.replace("\"tune\"", "\"tuning\"");
        let patch = Patch::from_ron(&text).unwrap();
        let mut parts = create_engine(loom_modules::registry(), EngineConfig::default());
        assert!(matches!(
            patch.load_into(&mut parts.controller),
            Err(PatchError::UnknownParam { module: 2, .. })
        ));
    }
}
