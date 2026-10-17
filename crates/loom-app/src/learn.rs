//! MIDI learn in the app: learn mode, the armed parameter, the labels on
//! tiles, and a watcher that wakes the UI when MIDI control changes arrive.
//! Spec: `docs/design/midi-learn.md`.

use iced::futures::Stream;
use iced::futures::channel::mpsc;
use loom_canvas::LearnView;
use loom_core::{Controller, MidiMapping, MidiReader, ModuleId};
use std::hash::{Hash, Hasher};
use std::time::Duration;

/// How often the watcher looks for new control changes. A hardware knob
/// then moves the knob on screen within about one frame.
pub const INTERVAL: Duration = Duration::from_millis(15);

/// A parameter: module ID and parameter index.
pub type Param = (u32, usize);

/// The armed parameter, and the serial of the last control change when it
/// was armed. Only a newer control change maps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Armed {
    pub param: Param,
    pub serial: Option<u32>,
}

/// Watches for control changes. One for the whole app.
#[derive(Clone)]
pub struct Watch {
    pub reader: MidiReader,
}

impl Hash for Watch {
    // The reader is the same for the whole app, so the subscription never
    // restarts while it is wanted.
    fn hash<H: Hasher>(&self, state: &mut H) {
        "midi-learn".hash(state);
    }
}

/// Yields the serial of each new control change, checked every
/// [`INTERVAL`]. Several changes between checks give one message. The thread
/// stops when the subscription ends.
pub fn stream(watch: &Watch) -> impl Stream<Item = u32> + use<> {
    let (mut tx, rx) = mpsc::channel(1);
    let reader = watch.reader.clone();
    std::thread::spawn(move || {
        // Start from "nothing seen", so a control change that arrives while
        // the thread starts is still reported. The app ignores any that came
        // before a parameter was armed.
        let mut last = None;
        while !tx.is_closed() {
            let now = reader.last_control_change().map(|seen| seen.serial);
            if let Some(serial) = now.filter(|_| now != last) {
                match tx.try_send(serial) {
                    Ok(()) => last = now,
                    // The UI has not taken the last one yet: it reads fresh
                    // values when it does, so try again next time.
                    Err(e) if e.is_full() => {}
                    Err(_) => break,
                }
            }
            std::thread::sleep(INTERVAL);
        }
    });
    rx
}

/// What learn mode shows on the canvas: the armed control and a label on
/// every mapped control.
pub fn view(controller: &Controller, armed: Option<Armed>) -> LearnView {
    let graph = controller.graph();
    let mut view = LearnView::default();
    for (id, node) in graph.nodes() {
        let d = node.descriptor;
        for (widget, w) in d.panel.widgets.iter().enumerate() {
            if !w.is_control() {
                continue;
            }
            let Some(param) = d.param_index(w.bind) else {
                continue;
            };
            if armed.is_some_and(|a| a.param == (id.0, param)) {
                view.armed = Some((id.0, widget));
            }
            if let Some((_, mapping)) = graph.midi_mapping_of(id, param) {
                view.labels
                    .insert((id.0, widget), format!("CC {}", mapping.control.controller));
            }
        }
    }
    view
}

/// A parameter as the status bar names it: "Filter #3 Cutoff".
pub fn param_name(controller: &Controller, (module, param): Param) -> String {
    controller
        .graph()
        .node(ModuleId(module))
        .and_then(|node| {
            let spec = node.descriptor.params.get(param)?;
            Some(format!("{} #{module} {}", node.descriptor.name, spec.label))
        })
        .unwrap_or_else(|| format!("#{module}"))
}

/// The status after a mapping is made. Names the parameter a control left,
/// if it was mapped elsewhere before (spec, Making a mapping).
pub fn mapped_status(controller: &Controller, mapping: MidiMapping, removed: &[MidiMapping]) -> String {
    let target = (mapping.target.0, mapping.param);
    let mut status = format!("Mapped {} to {}", mapping.control, param_name(controller, target));
    if let Some(left) = removed
        .iter()
        .find(|old| old.control == mapping.control && (old.target.0, old.param) != target)
    {
        status.push_str(&format!(
            ". It no longer moves {}",
            param_name(controller, (left.target.0, left.param))
        ));
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use loom_core::{EngineConfig, MidiControl, create_engine};

    const CC74: MidiControl = MidiControl {
        channel: 0,
        controller: 74,
    };

    #[test]
    fn labels_follow_mappings_and_the_armed_control() {
        let mut parts = create_engine(loom_modules::registry(), EngineConfig::default());
        let controller = &mut parts.controller;
        let filter = controller.add_module("loom.filter.svf").unwrap();
        let d = controller.graph().node(filter).unwrap().descriptor;
        let cutoff = d.param_index("cutoff").unwrap();
        let resonance = d.param_index("resonance").unwrap();
        let widget_of = |param: usize| {
            d.panel
                .widgets
                .iter()
                .position(|w| d.param_index(w.bind) == Some(param))
                .unwrap()
        };
        controller.map_midi(CC74, filter, cutoff).unwrap();

        let view = view(
            controller,
            Some(Armed {
                param: (filter.0, resonance),
                serial: None,
            }),
        );
        assert_eq!(view.armed, Some((filter.0, widget_of(resonance))));
        assert_eq!(view.labels.len(), 1);
        assert_eq!(view.labels[&(filter.0, widget_of(cutoff))], "CC 74");
    }

    #[test]
    fn the_status_names_the_parameter_a_control_left() {
        let mut parts = create_engine(loom_modules::registry(), EngineConfig::default());
        let controller = &mut parts.controller;
        let filter = controller.add_module("loom.filter.svf").unwrap();
        controller.map_midi_named(CC74, filter, "cutoff").unwrap();
        let (_, removed) = controller.map_midi_named(CC74, filter, "resonance").unwrap();
        let (_, mapping) = controller.graph().midi_mapping_on(CC74).unwrap();
        let status = mapped_status(controller, *mapping, &removed);
        assert!(status.starts_with("Mapped CC 74 · ch 1 to "), "{status}");
        assert!(status.contains("no longer moves"), "{status}");
        assert!(status.ends_with("Cutoff"), "{status}");

        // A new mapping on a free control says nothing about a move.
        let (_, removed) = controller
            .map_midi_named(MidiControl { controller: 71, ..CC74 }, filter, "cutoff")
            .unwrap();
        let (_, mapping) = controller.graph().midi_mapping_of(filter, 0).unwrap();
        let status = mapped_status(controller, *mapping, &removed);
        assert!(!status.contains("no longer"), "{status}");
    }
}
