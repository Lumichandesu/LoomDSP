//! Static module descriptions.

use crate::module::Module;
use crate::signal::SignalType;

/// Whether an output swings around zero or sits above it.
///
/// Modulation uses this to decide how a source moves a control (ADR-0014).
/// Gate and phase outputs are unipolar. Audio, pitch, and modulation outputs
/// are bipolar unless the module says otherwise. An envelope is unipolar even
/// though its signal type is modulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// Nominally 0 to 1, such as an envelope or a gate.
    Unipolar,
    /// Nominally -1 to 1, such as an LFO or an oscillator.
    Bipolar,
}

impl Polarity {
    /// Remaps `sample` from the polarity the output declares to this one.
    ///
    /// The same polarity returns `sample` unchanged. A bipolar source forced
    /// unipolar becomes `(sample + 1) / 2`. A unipolar source forced bipolar
    /// becomes `2 * sample - 1`. The sample is not clamped.
    pub fn remap(self, declared: Self, sample: f32) -> f32 {
        if self == declared {
            return sample;
        }
        match self {
            Self::Unipolar => (sample + 1.0) / 2.0,
            Self::Bipolar => sample * 2.0 - 1.0,
        }
    }
}

/// One input or output port.
///
/// Write it as a struct literal, so every field is named and a missing one
/// is a compile error:
///
/// ```
/// use loom_core::{Polarity, PortDefault, PortSpec, SignalType};
///
/// const RESET: PortSpec = PortSpec {
///     name: "reset",
///     label: "Reset",
///     signal: SignalType::Gate,
///     description: "Starts the count again.",
///     default: PortDefault::Value(0.0),
///     polarity: Polarity::Unipolar,
/// };
/// ```
///
/// The name is a stable ID: patch files store it, so never rename a
/// released port. The label and description are text for people. They can
/// be reworded at any time, because patch files never store them.
#[derive(Debug, Clone, Copy)]
pub struct PortSpec {
    /// Stable name used in patch files. Never rename a released port.
    pub name: &'static str,
    /// Short display name, for example "Reset". Not stored in patches.
    pub label: &'static str,
    /// Signal type, for colour and defaults.
    pub signal: SignalType,
    /// What the port does, in one short sentence that fits a tooltip.
    /// Not stored in patches. Longer theory goes in `docs/learn/`.
    pub description: &'static str,
    /// What an input reads while no cable is connected. Ignored on outputs;
    /// write `PortDefault::Value(0.0)` there.
    pub default: PortDefault,
    /// How an output moves a modulated control. Ignored on inputs.
    /// Gate and phase default to unipolar. Audio, pitch, and modulation
    /// default to bipolar.
    pub polarity: Polarity,
}

impl PortSpec {
    /// Sets what the input reads while no cable is connected.
    pub const fn with_default(mut self, default: PortDefault) -> Self {
        self.default = default;
        self
    }

    /// Sets the polarity an output declares for modulation.
    pub const fn with_polarity(mut self, polarity: Polarity) -> Self {
        self.polarity = polarity;
        self
    }
}

/// What an unconnected input reads (ADR-0015).
///
/// The default is static: it does not change with the module's parameters.
/// The compiler binds inputs when the patch changes, not when a parameter
/// moves, so binding stays off the audio thread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PortDefault {
    /// A fixed number. Must be finite.
    Value(f32),
    /// The transport's bar phase, from 0 up to 1.
    BarPhase,
    /// The transport's beat phase (quarter notes), from 0 up to 1.
    BeatPhase,
    /// The transport's play gate: 1.0 while playing, 0.0 while stopped.
    Playing,
}

/// One parameter.
///
/// Write it as a struct literal, so every field is named and a missing one
/// is a compile error. Optional settings live in [`ParamOptions`]; start
/// from [`ParamOptions::DEFAULT`] and use the builder methods:
///
/// ```
/// use loom_core::{ParamOptions, ParamSpec};
///
/// const N: ParamSpec = ParamSpec {
///     name: "n",
///     label: "Division",
///     description: "Fires on every Nth trigger.",
///     min: 1.0,
///     max: 64.0,
///     default: 2.0,
///     unit: "",
///     options: ParamOptions::DEFAULT,
/// }
/// .integer();
/// ```
///
/// The name is a stable ID: patch files store it, so never rename a
/// released parameter. The label and description are text for people. They
/// can be reworded at any time, because patch files never store them.
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    /// Stable name used in patch files. Never rename a released parameter.
    pub name: &'static str,
    /// Short display name, for example "Cutoff". Not stored in patches.
    pub label: &'static str,
    /// What the parameter does, in one short sentence that fits a tooltip.
    /// Not stored in patches. Longer theory goes in `docs/learn/`.
    pub description: &'static str,
    /// Minimum value.
    pub min: f32,
    /// Maximum value.
    pub max: f32,
    /// Default value.
    pub default: f32,
    /// Display unit, for example "Hz", "s", or "".
    pub unit: &'static str,
    /// Optional settings. Write [`ParamOptions::DEFAULT`] and change them
    /// with the builder methods, such as [`ParamSpec::integer`].
    pub options: ParamOptions,
}

/// Optional settings of a [`ParamSpec`].
///
/// It is a separate struct with a [`ParamOptions::DEFAULT`] constant, so a
/// new option later breaks no module. `ParamSpec` as a whole has no default,
/// because its label and description must always be written. A constant,
/// not `Default::default()`, because descriptors are `static`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ParamOptions {
    /// True for whole-number parameters, such as a MIDI channel or a seed.
    /// Values are rounded, and editors step by 1.
    pub integer: bool,
    /// How the value maps to its control's position.
    pub mapping: ParamMapping,
    /// Seconds the engine spends smoothing this parameter's base while it
    /// is modulated. Zero means the base steps immediately. The glide is in
    /// the control's 0 to 1 position, and the modulation added on top is not
    /// smoothed. Spec: `docs/design/modulation.md`, rule 7.
    pub smooth_seconds: f32,
}

impl ParamOptions {
    /// No options: a linear parameter with any value in range.
    pub const DEFAULT: Self = Self {
        integer: false,
        mapping: ParamMapping::Linear,
        smooth_seconds: 0.0,
    };
}

impl Default for ParamOptions {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How a parameter's value maps to its control's position, from 0 to 1.
///
/// The inspector, panel controls, and the engine's modulation all map
/// through [`ParamSpec::normalize`] and [`ParamSpec::denormalize`], so a
/// modulation arc on a knob shows exactly what the engine does.
/// Spec: `docs/design/modulation.md`, Parameter mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ParamMapping {
    /// Equal steps of the control give equal steps of the value.
    #[default]
    Linear,
    /// Equal steps of the control give equal ratios of the value, as suits
    /// frequencies and times. Needs a minimum above 0, and not a whole
    /// number; the registry refuses others.
    Logarithmic,
}

impl ParamSpec {
    /// Marks the parameter as a whole number.
    pub const fn integer(mut self) -> Self {
        self.options.integer = true;
        self
    }

    /// Maps the parameter logarithmically, for frequencies and times.
    pub const fn logarithmic(mut self) -> Self {
        self.options.mapping = ParamMapping::Logarithmic;
        self
    }

    /// Smooths the base value over about `seconds` while the parameter is
    /// modulated. The time is the same one-pole glide modules already use
    /// for an unmodulated knob.
    pub const fn smooth(mut self, seconds: f32) -> Self {
        self.options.smooth_seconds = seconds;
        self
    }

    /// Clamps a value into range, rounding whole-number parameters.
    /// NaN becomes the default.
    pub fn clamp(&self, value: f32) -> f32 {
        if value.is_nan() {
            self.default
        } else if self.options.integer {
            value.round().clamp(self.min, self.max)
        } else {
            value.clamp(self.min, self.max)
        }
    }

    /// Maps a value to its control's position, from 0 to 1. The value is
    /// clamped first, as by [`ParamSpec::clamp`].
    pub fn normalize(&self, value: f32) -> f32 {
        let value = self.clamp(value);
        if self.max <= self.min {
            return 0.0;
        }
        let position = match self.options.mapping {
            ParamMapping::Linear => (value - self.min) / (self.max - self.min),
            ParamMapping::Logarithmic => (value / self.min).ln() / (self.max / self.min).ln(),
        };
        position.clamp(0.0, 1.0)
    }

    /// Maps a control's position, from 0 to 1, to a value. The position is
    /// clamped to 0 to 1, and the result is in range and rounded for whole
    /// numbers, as by [`ParamSpec::clamp`]. NaN gives the default.
    pub fn denormalize(&self, position: f32) -> f32 {
        if position.is_nan() {
            return self.default;
        }
        let t = position.clamp(0.0, 1.0);
        let value = match self.options.mapping {
            ParamMapping::Linear => self.min + t * (self.max - self.min),
            ParamMapping::Logarithmic => self.min * (self.max / self.min).powf(t),
        };
        self.clamp(value)
    }
}

/// Settings passed to a module factory.
#[derive(Debug, Clone, Copy)]
pub struct ModuleConfig {
    /// Sample rate in Hz.
    pub sample_rate: f32,
    /// Largest number of frames the module will be asked to process at once.
    pub max_block: usize,
}

/// How the engine runs a module.
#[derive(Clone, Copy)]
pub enum ModuleRole {
    /// A normal module created by this factory.
    Dsp(fn(&ModuleConfig) -> Box<dyn Module>),
    /// A one-sample delay, handled by the engine itself. It can break
    /// feedback loops without adding extra delay (ADR-0002).
    /// Must have exactly one input and one output.
    UnitDelay,
    /// Sends its two inputs (left, right) to the engine output.
    /// Must have exactly two inputs and no outputs.
    AudioOutput,
}

impl std::fmt::Debug for ModuleRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModuleRole::Dsp(_) => f.write_str("Dsp"),
            ModuleRole::UnitDelay => f.write_str("UnitDelay"),
            ModuleRole::AudioOutput => f.write_str("AudioOutput"),
        }
    }
}

/// Everything the engine and editor need to know about a module type.
#[derive(Debug)]
pub struct ModuleDescriptor {
    /// Stable, namespaced ID, for example `"loom.osc.sine"`.
    pub id: &'static str,
    /// Version of the module's ports and parameters. Increase it when they
    /// change in a way that needs a patch migration.
    pub version: u32,
    /// Display name.
    pub name: &'static str,
    /// Category for the module browser.
    pub category: &'static str,
    /// One-line description.
    pub description: &'static str,
    /// Inputs (at most 64).
    pub inputs: &'static [PortSpec],
    /// Outputs.
    pub outputs: &'static [PortSpec],
    /// Parameters.
    pub params: &'static [ParamSpec],
    /// How the engine runs it.
    pub role: ModuleRole,
    /// What the module's tile shows between its port columns (ADR-0013).
    pub panel: Panel,
    /// Names of the values the module publishes for displays, at most
    /// [`MAX_DISPLAYS`]. It writes them in
    /// [`Module::displays`](crate::Module::displays).
    pub displays: &'static [&'static str],
}

/// Most display values one module can publish.
pub const MAX_DISPLAYS: usize = 4;

/// A module's panel: the controls and displays on its tile, laid out on the
/// canvas grid (ADR-0013). Spec: `docs/design/panels.md`, Module panels on
/// tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panel {
    /// Width in grid units.
    pub width: u8,
    /// Height in grid units.
    pub height: u8,
    /// Widgets, in keyboard focus order.
    pub widgets: &'static [Widget],
}

impl Panel {
    /// A panel with nothing on it.
    pub const EMPTY: Self = Self::new(0, 0, &[]);

    /// A panel of `width` by `height` grid units.
    pub const fn new(width: u8, height: u8, widgets: &'static [Widget]) -> Self {
        Self { width, height, widgets }
    }
}

/// One control or display on a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Widget {
    /// What it is.
    pub kind: WidgetKind,
    /// Column of its top-left corner, in grid units from the panel's left.
    pub x: u8,
    /// Row of its top-left corner, in grid units from the panel's top.
    pub y: u8,
    /// What it shows: a parameter name for controls, a display value name
    /// for lights, an output name for meters.
    pub bind: &'static str,
}

impl Widget {
    /// A knob for parameter `param`.
    pub const fn knob(x: u8, y: u8, param: &'static str) -> Self {
        Self {
            kind: WidgetKind::Knob,
            x,
            y,
            bind: param,
        }
    }

    /// A toggle for a whole-number parameter from 0 to 1.
    pub const fn toggle(x: u8, y: u8, param: &'static str) -> Self {
        Self {
            kind: WidgetKind::Toggle,
            x,
            y,
            bind: param,
        }
    }

    /// Radio buttons for a whole-number parameter with one value per option.
    /// The first option is the minimum value.
    pub const fn radio(x: u8, y: u8, param: &'static str, options: &'static [&'static str]) -> Self {
        Self {
            kind: WidgetKind::Radio { options },
            x,
            y,
            bind: param,
        }
    }

    /// A light for display value `display`.
    pub const fn light(x: u8, y: u8, display: &'static str) -> Self {
        Self {
            kind: WidgetKind::Light,
            x,
            y,
            bind: display,
        }
    }

    /// A level meter on output `output`, fed by a port probe.
    pub const fn meter(x: u8, y: u8, output: &'static str) -> Self {
        Self {
            kind: WidgetKind::Meter,
            x,
            y,
            bind: output,
        }
    }

    /// Width and height in grid units.
    pub const fn size(&self) -> (u8, u8) {
        match self.kind {
            WidgetKind::Knob => (3, 3),
            WidgetKind::Meter => (3, 6),
            WidgetKind::Toggle | WidgetKind::Light => (3, 2),
            WidgetKind::Radio { options } => (3, options.len() as u8 + 1),
        }
    }

    /// True for widgets bound to a parameter.
    pub const fn is_control(&self) -> bool {
        !matches!(self.kind, WidgetKind::Light | WidgetKind::Meter)
    }
}

/// The kinds of widget a panel can hold. More come with later phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WidgetKind {
    /// A dial for any parameter.
    Knob,
    /// Off or on, for a whole-number parameter from 0 to 1.
    Toggle,
    /// One button per option, for a whole-number parameter.
    Radio {
        /// Option labels, from the minimum value up.
        options: &'static [&'static str],
    },
    /// A display value as brightness, clamped to 0 to 1.
    Light,
    /// The level of one of the module's outputs, from a port probe: an RMS
    /// bar and a peak line, -60 dB to 0 dB.
    Meter,
}

impl ModuleDescriptor {
    /// Index of the input with this name.
    pub fn input_index(&self, name: &str) -> Option<usize> {
        self.inputs.iter().position(|p| p.name == name)
    }

    /// Index of the output with this name.
    pub fn output_index(&self, name: &str) -> Option<usize> {
        self.outputs.iter().position(|p| p.name == name)
    }

    /// Index of the parameter with this name.
    pub fn param_index(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p.name == name)
    }

    /// Index of a display value by name.
    pub fn display_index(&self, name: &str) -> Option<usize> {
        self.displays.iter().position(|&d| d == name)
    }

    /// Checks the display values and the panel: every widget binds to
    /// something this module has, fits its binding, lies inside the panel,
    /// and overlaps no other widget. The registry runs this check.
    pub fn check_panel(&self) -> Result<(), String> {
        if self.displays.len() > MAX_DISPLAYS {
            return Err(format!("more than {MAX_DISPLAYS} display values"));
        }
        let panel = &self.panel;
        for (i, widget) in panel.widgets.iter().enumerate() {
            let name = |problem: &str| format!("panel widget `{}` ({:?}): {problem}", widget.bind, widget.kind);
            if widget.is_control() {
                let Some(index) = self.param_index(widget.bind) else {
                    return Err(name("no such parameter"));
                };
                let param = &self.params[index];
                let values = param.max - param.min + 1.0;
                let integer = param.options.integer;
                match widget.kind {
                    WidgetKind::Toggle if !(integer && param.min == 0.0 && param.max == 1.0) => {
                        return Err(name("a toggle needs a whole-number parameter from 0 to 1"));
                    }
                    WidgetKind::Radio { options } if !(integer && values == options.len() as f32) => {
                        return Err(name(
                            "radio buttons need a whole-number parameter with one value per option",
                        ));
                    }
                    _ => {}
                }
            } else if widget.kind == WidgetKind::Meter {
                if !self.outputs.iter().any(|port| port.name == widget.bind) {
                    return Err(name("no such output"));
                }
            } else if self.display_index(widget.bind).is_none() {
                return Err(name("no such display value"));
            }
            let (w, h) = widget.size();
            if u16::from(widget.x) + u16::from(w) > u16::from(panel.width)
                || u16::from(widget.y) + u16::from(h) > u16::from(panel.height)
            {
                return Err(name("outside the panel"));
            }
            for other in &panel.widgets[..i] {
                let (ow, oh) = other.size();
                let (x, y, w, h) = (u16::from(widget.x), u16::from(widget.y), u16::from(w), u16::from(h));
                let (ox, oy, ow, oh) = (u16::from(other.x), u16::from(other.y), u16::from(ow), u16::from(oh));
                let apart = x >= ox + ow || ox >= x + w || y >= oy + oh || oy >= y + h;
                if !apart {
                    return Err(name(&format!("overlaps `{}`", other.bind)));
                }
            }
        }
        Ok(())
    }

    /// The first port or parameter with an empty (or blank) label or
    /// description, in the order inputs, outputs, parameters.
    pub const fn first_undocumented(&self) -> Option<Undocumented> {
        let mut index = 0;
        while let Some(text) = self.text(index) {
            if is_blank(text.value) {
                return Some(text.undocumented);
            }
            index += 1;
        }
        None
    }

    /// Every port and parameter with an empty (or blank) label or
    /// description, in the order inputs, outputs, parameters.
    pub fn undocumented(&self) -> Vec<Undocumented> {
        (0..)
            .map_while(|index| self.text(index))
            .filter(|text| is_blank(text.value))
            .map(|text| text.undocumented)
            .collect()
    }

    /// Text field number `index`, counting the label and then the
    /// description of each input, output, and parameter. One list for both
    /// checks above, so they cannot disagree.
    const fn text(&self, index: usize) -> Option<Text> {
        let (port, field) = (index / 2, index % 2);
        let (kind, name, label, description) = if port < self.inputs.len() {
            let spec = &self.inputs[port];
            (SpecKind::Input, spec.name, spec.label, spec.description)
        } else if port < self.inputs.len() + self.outputs.len() {
            let spec = &self.outputs[port - self.inputs.len()];
            (SpecKind::Output, spec.name, spec.label, spec.description)
        } else if port < self.inputs.len() + self.outputs.len() + self.params.len() {
            let spec = &self.params[port - self.inputs.len() - self.outputs.len()];
            (SpecKind::Parameter, spec.name, spec.label, spec.description)
        } else {
            return None;
        };
        let (missing, value) = if field == 0 {
            (MissingText::Label, label)
        } else {
            (MissingText::Description, description)
        };
        Some(Text {
            undocumented: Undocumented { kind, name, missing },
            value,
        })
    }

    /// The `"id@version"` string used in patch files.
    pub fn kind(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }

    /// Default parameter values.
    pub fn default_params(&self) -> Vec<f32> {
        self.params.iter().map(|p| p.default).collect()
    }
}

/// A port or parameter with an empty label or description.
///
/// Every built-in module is checked at compile time with
/// [`assert_documented`]. The `loom-test` harness fails on one, and the
/// registry loads the module but keeps a note ([`Registry::notes`](crate::Registry::notes)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Undocumented {
    /// Whether it is an input, an output, or a parameter.
    pub kind: SpecKind,
    /// Its stable name.
    pub name: &'static str,
    /// Which text is empty.
    pub missing: MissingText,
}

/// Whether a spec is an input, an output, or a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecKind {
    /// An input port.
    Input,
    /// An output port.
    Output,
    /// A parameter.
    Parameter,
}

impl SpecKind {
    /// "input", "output", or "parameter".
    pub const fn as_str(self) -> &'static str {
        match self {
            SpecKind::Input => "input",
            SpecKind::Output => "output",
            SpecKind::Parameter => "parameter",
        }
    }
}

/// Which text of a port or parameter is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingText {
    /// The `label` field.
    Label,
    /// The `description` field.
    Description,
}

impl MissingText {
    /// "label" or "description".
    pub const fn as_str(self) -> &'static str {
        match self {
            MissingText::Label => "label",
            MissingText::Description => "description",
        }
    }
}

impl std::fmt::Display for Undocumented {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} `{}` has no {}",
            self.kind.as_str(),
            self.name,
            self.missing.as_str()
        )
    }
}

/// One text field of a port or parameter, for the checks.
struct Text {
    undocumented: Undocumented,
    value: &'static str,
}

const fn is_blank(text: &str) -> bool {
    text.trim_ascii().is_empty()
}

/// Panics if any port or parameter of `modules` has an empty label or
/// description. Call it in a `const` item, so a missing text is a compile
/// error:
///
/// ```
/// # static ALL: &[&loom_core::ModuleDescriptor] = &[];
/// const _: () = loom_core::assert_documented(ALL);
/// ```
pub const fn assert_documented(modules: &[&ModuleDescriptor]) {
    let mut i = 0;
    while i < modules.len() {
        let module = modules[i];
        if let Some(missing) = module.first_undocumented() {
            // A `const` cannot format, so the message is copied into a
            // fixed buffer by hand.
            let message = ConstMessage::new()
                .push(module.id)
                .push(": ")
                .push(missing.kind.as_str())
                .push(" `")
                .push(missing.name)
                .push("` has no ")
                .push(missing.missing.as_str())
                .push(". Every port and parameter needs a short label and a one-sentence description.");
            panic!("{}", message.as_str(module.id));
        }
        i += 1;
    }
}

/// A message built in a `const fn`, cut at 256 bytes.
struct ConstMessage {
    bytes: [u8; 256],
    len: usize,
}

impl ConstMessage {
    const fn new() -> Self {
        Self {
            bytes: [0; 256],
            len: 0,
        }
    }

    const fn push(mut self, text: &str) -> Self {
        let text = text.as_bytes();
        let mut i = 0;
        while i < text.len() && self.len < self.bytes.len() {
            self.bytes[self.len] = text[i];
            self.len += 1;
            i += 1;
        }
        self
    }

    /// The message, or `fallback` if the cut split a character.
    const fn as_str<'a>(&'a self, fallback: &'a str) -> &'a str {
        match std::str::from_utf8(self.bytes.split_at(self.len).0) {
            Ok(text) => text,
            Err(_) => fallback,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A parameter for test descriptors.
    const fn param(name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> ParamSpec {
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

    const CUTOFF: ParamSpec = param("cutoff", 20.0, 20_000.0, 1_000.0, "Hz").logarithmic();
    const GAIN: ParamSpec = param("gain", -10.0, 10.0, 1.0, "");
    const SEED: ParamSpec = param("seed", 1.0, 65_535.0, 1.0, "").integer();

    #[test]
    fn mappings_round_trip() {
        for spec in [CUTOFF, GAIN, SEED] {
            for value in [spec.min, spec.default, spec.max] {
                let back = spec.denormalize(spec.normalize(value));
                assert!(
                    (back - value).abs() <= value.abs() * 1e-4 + 1e-4,
                    "{}: {value} -> {back}",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn linear_is_the_default() {
        assert_eq!(GAIN.options.mapping, ParamMapping::Linear);
        assert_eq!(GAIN.denormalize(0.5), 0.0);
        assert_eq!(GAIN.normalize(5.0), 0.75);
    }

    #[test]
    fn logarithmic_steps_are_equal_ratios() {
        // The middle of the knob is the geometric middle of the range.
        assert!((CUTOFF.denormalize(0.5) - 632.46).abs() < 0.1);
        // A tenth of the knob is about one octave, anywhere on it.
        for start in [0.0, 0.3, 0.9] {
            let ratio = CUTOFF.denormalize(start + 0.1) / CUTOFF.denormalize(start);
            assert!((ratio - 1_000f32.powf(0.1)).abs() < 1e-3, "{ratio}");
            assert!((ratio - 2.0).abs() < 0.01);
        }
    }

    #[test]
    fn whole_numbers_round_and_step_evenly() {
        assert_eq!(SEED.denormalize(0.5), 32_768.0);
        assert_eq!(SEED.denormalize(SEED.normalize(65_000.0)), 65_000.0);
        let step = 1.0 / (SEED.max - SEED.min);
        assert_eq!(SEED.denormalize(SEED.normalize(100.0) + step), 101.0);
    }

    #[test]
    fn polarity_remap_follows_the_override() {
        assert_eq!(Polarity::Bipolar.remap(Polarity::Bipolar, -0.4), -0.4);
        assert_eq!(Polarity::Unipolar.remap(Polarity::Bipolar, -1.0), 0.0);
        assert_eq!(Polarity::Unipolar.remap(Polarity::Bipolar, 1.0), 1.0);
        assert_eq!(Polarity::Bipolar.remap(Polarity::Unipolar, 0.0), -1.0);
        assert_eq!(Polarity::Bipolar.remap(Polarity::Unipolar, 1.0), 1.0);
        // A source past the nominal range is kept. Only the parameter is clamped.
        assert_eq!(Polarity::Bipolar.remap(Polarity::Bipolar, 3.0), 3.0);
    }

    #[test]
    fn results_are_always_valid() {
        for spec in [CUTOFF, GAIN, SEED] {
            assert_eq!(spec.normalize(spec.max * 10.0), 1.0);
            assert_eq!(spec.normalize(-1e9), 0.0);
            assert_eq!(spec.denormalize(2.0), spec.max);
            assert_eq!(spec.denormalize(-1.0), spec.min);
            assert_eq!(spec.denormalize(f32::NAN), spec.default);
            assert_eq!(spec.normalize(f32::NAN), spec.normalize(spec.default));
        }
        let fixed = param("fixed", 1.0, 1.0, 1.0, "");
        assert_eq!(fixed.normalize(1.0), 0.0);
        assert_eq!(fixed.denormalize(0.5), 1.0);
    }

    static DOCUMENTED: ModuleDescriptor = ModuleDescriptor {
        id: "test.documented",
        version: 1,
        name: "Documented",
        category: "Test",
        description: "",
        inputs: &[],
        outputs: &[],
        params: &[GAIN],
        role: ModuleRole::UnitDelay,
        panel: Panel::EMPTY,
        displays: &[],
    };

    static UNDOCUMENTED: ModuleDescriptor = ModuleDescriptor {
        id: "test.undocumented",
        version: 1,
        name: "Undocumented",
        category: "Test",
        description: "",
        inputs: &[PortSpec {
            name: "in",
            label: "In",
            signal: SignalType::Audio,
            description: "",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        }],
        outputs: &[PortSpec {
            name: "out",
            label: "\t",
            signal: SignalType::Audio,
            description: "The input.",
            default: PortDefault::Value(0.0),

            polarity: Polarity::Bipolar,
        }],
        params: &[ParamSpec {
            label: "",
            description: "",
            ..GAIN
        }],
        role: ModuleRole::UnitDelay,
        panel: Panel::EMPTY,
        displays: &[],
    };

    #[test]
    fn lists_empty_labels_and_descriptions_in_order() {
        assert_eq!(DOCUMENTED.first_undocumented(), None);
        assert!(DOCUMENTED.undocumented().is_empty());
        let missing: Vec<String> = UNDOCUMENTED.undocumented().iter().map(ToString::to_string).collect();
        assert_eq!(
            missing,
            [
                "input `in` has no description",
                "output `out` has no label",
                "parameter `gain` has no label",
                "parameter `gain` has no description",
            ]
        );
        assert_eq!(
            UNDOCUMENTED.first_undocumented(),
            Some(Undocumented {
                kind: SpecKind::Input,
                name: "in",
                missing: MissingText::Description,
            })
        );
    }

    #[test]
    fn assert_documented_passes_documented_modules() {
        assert_documented(&[&DOCUMENTED]);
    }

    #[test]
    #[should_panic(expected = "test.undocumented: input `in` has no description. Every port and parameter needs")]
    fn assert_documented_names_the_first_missing_text() {
        assert_documented(&[&DOCUMENTED, &UNDOCUMENTED]);
    }
}
