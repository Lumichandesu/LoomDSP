//! Where text goes on a tile: the title, port labels, and panel captions
//! (`docs/design/panels.md`, Level of detail).
//!
//! One list serves three uses: the text drawn close up, the faint bars that
//! stand in for it further out, and hover on port labels. Keeping them in
//! one place means a bar always sits exactly where its text would be.
//!
//! Positions are anchors, as Iced reads them: centred text is centred on
//! its anchor, and right-aligned text ends at it.

use crate::controls::{knob_centre, light_centre, radio_centre};
use crate::scene::{GRID, HEADER, ModuleView, PortRef, ROW, WidgetViewKind, fit_label, text_width};
use iced::{Point, Rectangle, Size};
use std::borrow::Cow;

/// Title size in world units.
pub const TITLE_SIZE: f32 = 11.0;
/// Port label and caption size in world units.
pub const LABEL_SIZE: f32 = 9.0;

/// How text sits on its anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Starts at the anchor.
    Left,
    /// Centred on the anchor.
    Center,
    /// Ends at the anchor.
    Right,
}

/// What a piece of tile text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    /// The module title, in the header.
    Title,
    /// A port's label.
    Port(PortRef),
    /// The caption of a panel widget, by index.
    Caption(usize),
    /// Option `.1` of the radio group at widget index `.0`.
    Option(usize, usize),
}

/// One piece of text on a tile.
#[derive(Debug, Clone, PartialEq)]
pub struct TextItem<'a> {
    /// What it is.
    pub kind: TextKind,
    /// The text, already cut with "…" to fit where needed.
    pub content: Cow<'a, str>,
    /// World point the text is aligned to, at its vertical centre.
    pub anchor: Point,
    /// How the text sits on the anchor.
    pub align: Align,
    /// Font size in world units.
    pub size: f32,
    /// Room for the text in world units. Longer text is clipped.
    pub width: f32,
}

impl TextItem<'_> {
    /// The estimated box of the text, in world units.
    pub fn bounds(&self) -> Rectangle {
        let width = text_width(&self.content, self.size).min(self.width);
        let x = match self.align {
            Align::Left => self.anchor.x,
            Align::Center => self.anchor.x - width * 0.5,
            Align::Right => self.anchor.x - width,
        };
        let height = self.size * 1.2;
        Rectangle::new(Point::new(x, self.anchor.y - height * 0.5), Size::new(width, height))
    }

    /// The faint bar that stands in for the text when it is too small to
    /// draw: as wide as the text, and half as tall as its font size, about
    /// the height of its lower-case letters.
    pub fn bar(&self) -> Rectangle {
        let text = self.bounds();
        let height = self.size * 0.5;
        Rectangle::new(
            Point::new(text.x, self.anchor.y - height * 0.5),
            Size::new(text.width, height),
        )
    }
}

/// Calls `f` with every piece of text on a module's tile, in drawing order:
/// title, port labels, then panel text.
pub fn for_each<'a>(module: &'a ModuleView, mut f: impl FnMut(TextItem<'a>)) {
    let tile = module.bounds();
    let pad = 7.0;
    f(TextItem {
        kind: TextKind::Title,
        content: Cow::Borrowed(&module.title),
        anchor: Point::new(tile.x + pad, tile.y + HEADER * 0.5),
        align: Align::Left,
        size: TITLE_SIZE,
        width: module.width() - 2.0 * pad,
    });

    let label_width = module.label_width();
    let ports = module
        .inputs
        .iter()
        .enumerate()
        .map(|(i, p)| (false, i, p))
        .chain(module.outputs.iter().enumerate().map(|(i, p)| (true, i, p)));
    for (output, index, port) in ports {
        let y = tile.y + HEADER + ROW * (index as f32 + 0.5);
        let (x, align) = if output {
            (tile.x + module.width() - 9.0, Align::Right)
        } else {
            (tile.x + 9.0, Align::Left)
        };
        f(TextItem {
            kind: TextKind::Port(PortRef {
                module: module.id,
                output,
                index,
            }),
            content: fit_label(&port.label, label_width, LABEL_SIZE),
            anchor: Point::new(x, y),
            align,
            size: LABEL_SIZE,
            width: label_width,
        });
    }

    for (index, widget) in module.panel.widgets.iter().enumerate() {
        let Some(rect) = module.widget_bounds(index) else {
            continue;
        };
        let y = match &widget.kind {
            WidgetViewKind::Knob { .. } => knob_centre(rect).y + 1.5 * GRID,
            WidgetViewKind::Meter => rect.y + rect.height - GRID * 0.5,
            WidgetViewKind::Toggle | WidgetViewKind::Light => light_centre(rect).y + GRID,
            WidgetViewKind::Button => rect.y + rect.height - GRID * 0.5,
            WidgetViewKind::Radio { .. } => rect.y + GRID * 0.5,
        };
        f(TextItem {
            kind: TextKind::Caption(index),
            content: fit_label(&widget.label, rect.width, LABEL_SIZE),
            anchor: Point::new(rect.center_x(), y),
            align: Align::Center,
            size: LABEL_SIZE,
            width: rect.width,
        });
        if let WidgetViewKind::Radio { options } = &widget.kind {
            for (i, option) in options.iter().enumerate() {
                let centre = radio_centre(rect, i);
                f(TextItem {
                    kind: TextKind::Option(index, i),
                    content: Cow::Borrowed(option),
                    anchor: Point::new(centre.x + 6.0, centre.y),
                    align: Align::Left,
                    size: LABEL_SIZE,
                    width: rect.width - 12.0,
                });
            }
        }
    }
}

/// The port whose label is under a world point. Text is drawn above every
/// tile, so a label can show over a tile that overlaps its own. Every tile
/// under the point is searched, topmost first.
pub fn port_label_at(scene: &crate::scene::Scene, point: Point) -> Option<PortRef> {
    scene
        .modules()
        .iter()
        .rev()
        .filter(|module| module.bounds().contains(point))
        .find_map(|module| {
            let mut found = None;
            for_each(module, |item| {
                if let TextKind::Port(port) = item.kind
                    && item.bounds().contains(point)
                {
                    found = Some(port);
                }
            });
            found
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{HoverText, PanelView, PortShape, PortView, Scene, WidgetView};
    use iced::Color;

    fn port(label: &str) -> PortView {
        PortView {
            label: label.into(),
            hover: HoverText::default(),
            colour: Color::WHITE,
            shape: PortShape::Round,
        }
    }

    fn widget(kind: WidgetViewKind, x: u8, y: u8, label: &str) -> WidgetView {
        WidgetView {
            kind,
            x,
            y,
            label: label.into(),
            hover: HoverText::default(),
        }
    }

    fn module() -> ModuleView {
        ModuleView {
            id: 1,
            title: "Filter".into(),
            accent: Color::BLACK,
            position: Point::new(24.0, 36.0),
            inputs: vec![port("In"), port("Cutoff")],
            outputs: vec![port("Lowpass"), port("Bandpass")],
            panel: PanelView {
                width: 9,
                height: 8,
                widgets: vec![
                    widget(WidgetViewKind::Knob { steps: 0 }, 0, 0, "Cutoff"),
                    widget(WidgetViewKind::Knob { steps: 0 }, 3, 0, "Resonance"),
                    widget(WidgetViewKind::Light, 6, 0, "level"),
                    widget(WidgetViewKind::Toggle, 0, 3, "Drive"),
                    widget(
                        WidgetViewKind::Radio {
                            options: vec!["12".into(), "24".into()],
                        },
                        3,
                        3,
                        "Slope",
                    ),
                    widget(WidgetViewKind::Meter, 6, 2, "Out"),
                ],
            },
        }
    }

    fn items(module: &ModuleView) -> Vec<TextItem<'_>> {
        let mut items = Vec::new();
        for_each(module, |item| items.push(item));
        items
    }

    #[test]
    fn captions_are_centred_on_their_widgets() {
        // #92: captions were anchored at the widget's left edge, and Iced
        // centred them on it.
        let module = module();
        for item in items(&module) {
            if let TextKind::Caption(index) = item.kind {
                let widget = module.widget_bounds(index).unwrap();
                let text = item.bounds();
                assert!(
                    (text.center_x() - widget.center_x()).abs() < 1e-3,
                    "{}: text centre {} vs widget centre {}",
                    item.content,
                    text.center_x(),
                    widget.center_x()
                );
                assert!(text.width <= widget.width, "{} fits its widget", item.content);
            }
        }
    }

    #[test]
    fn every_text_stays_inside_its_tile() {
        let module = module();
        let tile = module.bounds();
        for item in items(&module) {
            let text = item.bounds();
            assert!(
                text.x >= tile.x
                    && text.x + text.width <= tile.x + tile.width
                    && text.y >= tile.y
                    && text.y + text.height <= tile.y + tile.height,
                "{:?} {:?} spills out of {tile:?}: {text:?}",
                item.kind,
                item.content
            );
        }
    }

    #[test]
    fn port_labels_sit_on_their_rows_beside_their_ports() {
        let module = module();
        for item in items(&module) {
            let TextKind::Port(port) = item.kind else {
                continue;
            };
            let at = if port.output {
                module.output_position(port.index)
            } else {
                module.input_position(port.index)
            };
            assert_eq!(item.anchor.y, at.y, "{} on its port's row", item.content);
            let text = item.bounds();
            assert!(
                text.width <= module.label_width() + 1e-3,
                "{} fits its column",
                item.content
            );
            // Inputs read from the left edge, outputs end at the right edge.
            if port.output {
                assert!(text.x + text.width <= at.x);
            } else {
                assert!(text.x >= at.x);
            }
        }
    }

    #[test]
    fn bars_sit_inside_their_text() {
        let module = module();
        for item in items(&module) {
            let (text, bar) = (item.bounds(), item.bar());
            assert_eq!((bar.x, bar.width), (text.x, text.width), "{}", item.content);
            assert!(bar.y > text.y && bar.y + bar.height < text.y + text.height);
            assert!(bar.width > 0.0 && bar.height > 0.0);
        }
    }

    #[test]
    fn labels_are_found_under_the_pointer() {
        let scene = Scene::new(vec![module()], Vec::new());
        let module = &scene.modules()[0];
        let lowpass = PortRef {
            module: 1,
            output: true,
            index: 0,
        };
        let mut label = None;
        for_each(module, |item| {
            if item.kind == TextKind::Port(lowpass) {
                label = Some(item.bounds());
            }
        });
        let label = label.unwrap();
        assert_eq!(port_label_at(&scene, label.center()), Some(lowpass));
        // Empty panel space and the header are not labels.
        let knob = module.widget_bounds(0).unwrap();
        assert_eq!(port_label_at(&scene, knob.center()), None);
        assert_eq!(
            port_label_at(&scene, Point::new(module.bounds().center_x(), module.bounds().y + 2.0)),
            None
        );
        assert_eq!(port_label_at(&scene, Point::new(-500.0, -500.0)), None);

        // A tile on top that covers the label but has no label there
        // itself: the label still hovers, as it is drawn above both.
        let mut cover = self::module();
        cover.id = 2;
        cover.inputs.clear();
        cover.outputs.clear();
        cover.position = Point::new(label.x - 60.0, label.y - 40.0);
        let overlapped = Scene::new(vec![self::module(), cover], Vec::new());
        assert!(overlapped.modules()[1].bounds().contains(label.center()));
        assert_eq!(port_label_at(&overlapped, label.center()), Some(lowpass));
    }
}
