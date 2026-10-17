//! Panel controls that put a performer's gesture on a cable (FR-LIB-27).
//!
//! The button's mode changes the gesture only. The module copies `on` to
//! its gate. Spec: `docs/design/clock.md`, Button details.

use loom_core::{
    Module, ModuleDescriptor, ModuleRole, Panel, ParamOptions, ParamSpec, Polarity, PortDefault, PortIo, PortSpec,
    ProcessContext, SignalType, Widget,
};

/// `loom.ui.button`: a gate from a large button.
///
/// Toggle keeps the gate where the last press left it. Momentary is high
/// only while the button is held. The canvas chooses which, from `mode`.
/// This module does not.
pub static BUTTON: ModuleDescriptor = ModuleDescriptor {
    id: "loom.ui.button",
    version: 1,
    name: "Button",
    category: "Control",
    description: "Sends a gate from a large button. Toggle keeps it. Momentary is high only while held.",
    inputs: &[],
    outputs: &[PortSpec {
        name: "gate",
        label: "Gate",
        signal: SignalType::Gate,
        description: "High while the button is on.",
        default: PortDefault::Value(0.0),
        polarity: Polarity::Unipolar,
    }],
    params: &[
        ParamSpec {
            name: "mode",
            label: "Mode",
            description: "Toggle keeps the gate. Momentary is high only while the button is held.",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            unit: "",
            options: ParamOptions::DEFAULT,
        }
        .integer(),
        ParamSpec {
            name: "on",
            label: "On",
            description: "The gate the button sends.",
            min: 0.0,
            max: 1.0,
            default: 0.0,
            unit: "",
            options: ParamOptions::DEFAULT,
        }
        .integer(),
    ],
    role: ModuleRole::Dsp(|_| Box::new(Button { on: 0.0 })),
    panel: Panel::new(6, 3, &[Widget::button(0, 0, "on", "mode")]),
    displays: &[],
};

/// Index of `on`. `mode` is the other parameter, and `process` ignores it.
const ON: usize = 1;

struct Button {
    on: f32,
}

impl Module for Button {
    fn process(&mut self, _ctx: &ProcessContext<'_>, io: &mut PortIo<'_>) {
        // Whole-number parameters arrive already rounded (modulation rule 5),
        // so this copy is 0 or 1. There is no second threshold here.
        let modulated = io.modulated(ON);
        let ([], [out]) = io.split::<0, 1>();
        for (i, sample) in out.iter_mut().enumerate() {
            *sample = modulated.map_or(self.on, |samples| samples[i]);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index == ON {
            self.on = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loom_core::testing::run;

    fn render(on: f32) -> Vec<f32> {
        let mut module = Button { on: 0.0 };
        // The engine clamps and rounds before `set_param`. The module copies
        // what it is given.
        module.set_param(ON, BUTTON.params[ON].clamp(on));
        let mut out = vec![0.0; 8];
        run(&mut module, 48_000.0, &[], &[], &mut [&mut out], &[]);
        out
    }

    #[test]
    fn the_gate_equals_on() {
        assert_eq!(render(0.0), vec![0.0; 8]);
        assert_eq!(render(1.0), vec![1.0; 8]);
        // 0.6 rounds to 1, and 0.4 rounds to 0, before the module sees them.
        assert_eq!(render(0.6), vec![1.0; 8]);
        assert_eq!(render(0.4), vec![0.0; 8]);
    }

    #[test]
    fn mode_does_not_change_the_gate() {
        let mut module = Button { on: 0.0 };
        module.set_param(0, 1.0);
        module.set_param(ON, 1.0);
        let mut out = vec![0.0; 4];
        run(&mut module, 48_000.0, &[], &[], &mut [&mut out], &[]);
        assert_eq!(out, vec![1.0; 4]);
    }

    #[test]
    fn every_sample_is_off_or_on() {
        for &on in &[0.0, 1.0] {
            for sample in render(on) {
                assert!(sample == 0.0 || sample == 1.0, "{sample}");
            }
        }
    }
}
