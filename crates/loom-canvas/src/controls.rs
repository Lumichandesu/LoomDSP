//! Panel controls on tiles: their live values and their GPU instances
//! (ADR-0013). Spec: `docs/design/panels.md`, Module panels on tiles.
//!
//! Values live apart from the [`Scene`]. Changing one rebuilds only the
//! controls' instances, never the tiles, ports, or cables (ADR-0010).

use crate::gpu::{Arc as ArcShape, Shape};
use crate::scene::{GRID, Scene, WidgetViewKind};
use crate::theme::Palette;
use iced::{Color, Point, Rectangle, Size};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// A knob's ring starts here, in radians clockwise from the top.
pub(crate) const KNOB_START: f32 = -0.75 * std::f32::consts::PI;
/// A knob's ring spans this angle.
pub(crate) const KNOB_SWEEP: f32 = 1.5 * std::f32::consts::PI;
/// Radius of a knob's ring, in world units.
const KNOB_RADIUS: f32 = 9.0;

/// The live value of one widget.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WidgetValue {
    /// Knob: position from 0 to 1. Toggle and button: 0 or 1. Radio: option
    /// index. Light: brightness from 0 to 1. Meter: RMS level from 0 to 1.
    pub position: f32,
    /// Meter: peak level from 0 to 1. Unused by other widgets.
    pub peak: f32,
    /// The value as text, with its unit, shown while a knob is dragged or
    /// focused.
    pub text: String,
}

/// Live values for every widget, by module ID and widget index.
#[derive(Debug, Clone, Default)]
pub struct PanelValues {
    values: HashMap<(u32, usize), WidgetValue>,
    generation: u64,
}

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

impl PanelValues {
    /// No values: every widget draws at 0.
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Sets a widget's value. The generation changes only if the value did,
    /// so setting the same value does not redraw.
    pub fn set(&mut self, module: u32, widget: usize, value: WidgetValue) {
        if self.values.get(&(module, widget)) != Some(&value) {
            self.values.insert((module, widget), value);
            self.generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// A widget's value, if set.
    pub fn get(&self, module: u32, widget: usize) -> Option<&WidgetValue> {
        self.values.get(&(module, widget))
    }

    /// The position of a widget's value, or 0.
    pub fn position(&self, module: u32, widget: usize) -> f32 {
        self.get(module, widget).map_or(0.0, |v| v.position)
    }

    /// The peak of a meter's value, or 0.
    pub fn peak(&self, module: u32, widget: usize) -> f32 {
        self.get(module, widget).map_or(0.0, |v| v.peak)
    }

    /// Changes whenever a value changes. Unique across all value sets.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// GPU instances for every widget in a scene.
#[derive(Debug, Default)]
pub(crate) struct ControlGeometry {
    /// The scene and value generations this was built from.
    pub key: (u64, u64),
    /// Knob rings, drawn first.
    pub arcs: Vec<ArcShape>,
    /// Knob bodies and pointers, toggles, radio buttons, and lights.
    pub shapes: Vec<Shape>,
}

impl ControlGeometry {
    pub fn build(scene: &Scene, values: &PanelValues, palette: &Palette) -> Self {
        let mut arcs = Vec::new();
        let mut shapes = Vec::new();
        for module in scene.modules() {
            for (index, widget) in module.panel.widgets.iter().enumerate() {
                let Some(rect) = module.widget_bounds(index) else {
                    continue;
                };
                let position = values.position(module.id, index);
                match &widget.kind {
                    WidgetViewKind::Knob { .. } => {
                        let centre = knob_centre(rect);
                        let t = position.clamp(0.0, 1.0);
                        arcs.push(ArcShape::new(
                            centre,
                            KNOB_RADIUS,
                            2.5,
                            KNOB_START,
                            KNOB_SWEEP,
                            palette.control_track,
                        ));
                        if t > 0.0 {
                            arcs.push(ArcShape::new(
                                centre,
                                KNOB_RADIUS,
                                2.5,
                                KNOB_START,
                                KNOB_SWEEP * t,
                                palette.control_value,
                            ));
                        }
                        shapes.push(circle(centre, 6.5, palette.control_body, palette.tile_border, 1.0));
                        let angle = KNOB_START + KNOB_SWEEP * t;
                        let tip = Point::new(centre.x + 4.0 * angle.sin(), centre.y - 4.0 * angle.cos());
                        shapes.push(circle(tip, 1.5, palette.text, Color::TRANSPARENT, 0.0));
                    }
                    WidgetViewKind::Toggle => {
                        let on = position > 0.5;
                        let track = toggle_track(rect);
                        let fill = if on {
                            palette.control_value
                        } else {
                            palette.control_track
                        };
                        shapes.push(Shape::new(track, 5.5, fill, Color::TRANSPARENT, 0.0));
                        let x = if on { track.x + track.width - 5.5 } else { track.x + 5.5 };
                        shapes.push(circle(
                            Point::new(x, track.y + 5.5),
                            4.0,
                            palette.text,
                            Color::TRANSPARENT,
                            0.0,
                        ));
                    }
                    WidgetViewKind::Radio { options } => {
                        let chosen = position.round() as usize;
                        for i in 0..options.len() {
                            let centre = radio_centre(rect, i);
                            let fill = if i == chosen {
                                palette.control_value
                            } else {
                                palette.tile
                            };
                            shapes.push(circle(centre, 3.5, fill, palette.control_track, 1.2));
                        }
                    }
                    WidgetViewKind::Light => {
                        let b = position.clamp(0.0, 1.0);
                        let fill = mix(palette.light_off, palette.light_on, b);
                        shapes.push(circle(light_centre(rect), 5.0, fill, palette.tile_border, 1.0));
                    }
                    WidgetViewKind::Button => {
                        let on = position > 0.5;
                        let fill = if on {
                            palette.control_value
                        } else {
                            palette.control_track
                        };
                        shapes.push(Shape::new(button_body(rect), 6.0, fill, Color::TRANSPARENT, 0.0));
                    }
                    WidgetViewKind::Meter => {
                        let track = meter_track(rect);
                        shapes.push(Shape::new(track, 2.0, palette.light_off, palette.tile_border, 1.0));
                        let rms = position.clamp(0.0, 1.0);
                        if rms > 0.0 {
                            let height = track.height * rms;
                            let bar = Rectangle::new(
                                Point::new(track.x, track.y + track.height - height),
                                Size::new(track.width, height),
                            );
                            shapes.push(Shape::new(
                                bar,
                                2.0,
                                meter_colour(rms, palette),
                                Color::TRANSPARENT,
                                0.0,
                            ));
                        }
                        // The peak never shows below the RMS bar.
                        let peak = values.peak(module.id, index).clamp(rms, 1.0);
                        if peak > 0.0 {
                            let y = track.y + track.height * (1.0 - peak);
                            let line = Rectangle::new(Point::new(track.x, y - 0.75), Size::new(track.width, 1.5));
                            shapes.push(Shape::new(line, 0.0, palette.text, Color::TRANSPARENT, 0.0));
                        }
                    }
                }
            }
        }
        Self {
            key: (scene.generation(), values.generation()),
            arcs,
            shapes,
        }
    }
}

/// Centre of a knob's dial: in the top two grid rows of its 3 × 3 cell.
pub(crate) fn knob_centre(rect: Rectangle) -> Point {
    Point::new(rect.x + rect.width * 0.5, rect.y + GRID - 1.0)
}

/// Centre of a light: in the top row of its 3 × 2 cell.
pub(crate) fn light_centre(rect: Rectangle) -> Point {
    Point::new(rect.x + rect.width * 0.5, rect.y + GRID * 0.5 + 1.0)
}

/// The bar of a meter: a narrow column in its 3 × 6 cell, above the label
/// row.
pub(crate) fn meter_track(rect: Rectangle) -> Rectangle {
    Rectangle::new(
        Point::new(rect.x + rect.width * 0.5 - 4.0, rect.y + 1.0),
        Size::new(8.0, rect.height - GRID - 2.0),
    )
}

/// Meter level, from 0 to 1, for a decibel value (-60 dB to 0 dB).
fn level_of_db(db: f32) -> f32 {
    (db + 60.0) / 60.0
}

/// Green below -12 dB, amber up to -3 dB, red above (panels.md, Meter
/// widget).
pub(crate) fn meter_colour(level: f32, palette: &Palette) -> Color {
    if level < level_of_db(-12.0) {
        palette.light_on
    } else if level < level_of_db(-3.0) {
        palette.control_value
    } else {
        palette.warning
    }
}

/// The keyboard focus ring of a control, and its corner radius: around the
/// control itself, never its caption, so a value shown there stays clear
/// (panels.md, Interaction).
pub(crate) fn focus_ring(kind: &WidgetViewKind, rect: Rectangle) -> (Rectangle, f32) {
    match kind {
        WidgetViewKind::Knob { .. } => {
            let r = KNOB_RADIUS + 3.5;
            let c = knob_centre(rect);
            (
                Rectangle::new(Point::new(c.x - r, c.y - r), Size::new(2.0 * r, 2.0 * r)),
                r,
            )
        }
        WidgetViewKind::Toggle => (toggle_track(rect).expand(2.5), 8.0),
        WidgetViewKind::Button => (button_body(rect).expand(2.5), 8.0),
        // The options, below the caption row.
        WidgetViewKind::Radio { .. } => (
            Rectangle::new(
                Point::new(rect.x, rect.y + GRID),
                Size::new(rect.width, rect.height - GRID),
            )
            .expand(1.0),
            4.0,
        ),
        // Displays never take focus.
        WidgetViewKind::Light | WidgetViewKind::Meter => (rect.expand(1.0), 4.0),
    }
}

/// The face of a button: the cell above its caption row.
fn button_body(rect: Rectangle) -> Rectangle {
    Rectangle::new(
        Point::new(rect.x + 4.0, rect.y + 2.0),
        Size::new(rect.width - 8.0, rect.height - GRID - 4.0),
    )
}

/// The track of a toggle switch: in the top row of its 3 × 2 cell.
fn toggle_track(rect: Rectangle) -> Rectangle {
    Rectangle::new(Point::new(rect.x + 7.0, rect.y + 1.0), Size::new(22.0, 11.0))
}

/// Centre of radio option `i`: one row each, below the label row.
pub(crate) fn radio_centre(rect: Rectangle, i: usize) -> Point {
    Point::new(rect.x + 6.0, rect.y + GRID * (i as f32 + 1.5))
}

/// The radio option under a world point, if any.
pub(crate) fn radio_option(rect: Rectangle, options: usize, point: Point) -> Option<usize> {
    let row = ((point.y - rect.y) / GRID).floor() as isize - 1;
    (row >= 0 && (row as usize) < options).then_some(row as usize)
}

fn circle(centre: Point, radius: f32, fill: Color, border: Color, border_width: f32) -> Shape {
    let rect = Rectangle::new(
        Point::new(centre.x - radius, centre.y - radius),
        Size::new(radius * 2.0, radius * 2.0),
    );
    Shape::new(rect, radius, fill, border, border_width)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

/// A knob's new position after the pointer moved `dy` screen pixels (up is
/// negative). The full range is 200 pixels at any zoom; Shift is 10 times
/// finer.
pub(crate) fn knob_drag(position: f32, dy: f32, fine: bool) -> f32 {
    let scale = if fine { 0.1 } else { 1.0 };
    (position - dy / 200.0 * scale).clamp(0.0, 1.0)
}

/// A knob's new position after one arrow key press: 1% of the range, 0.1%
/// with Shift, or one step for a whole-number parameter.
pub(crate) fn knob_step(position: f32, up: bool, fine: bool, steps: u32) -> f32 {
    let step = if steps > 0 {
        1.0 / steps as f32
    } else if fine {
        0.001
    } else {
        0.01
    };
    let next = if up { position + step } else { position - step };
    next.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{ModuleView, PanelView, WidgetView};

    fn scene() -> Scene {
        let widget = |kind, x| WidgetView {
            kind,
            x,
            y: 0,
            label: String::new(),
            hover: crate::scene::HoverText::default(),
        };
        Scene::new(
            vec![ModuleView {
                id: 1,
                title: String::new(),
                accent: Color::BLACK,
                position: Point::ORIGIN,
                inputs: Vec::new(),
                outputs: Vec::new(),
                panel: PanelView {
                    width: 9,
                    height: 4,
                    widgets: vec![
                        widget(WidgetViewKind::Knob { steps: 0 }, 0),
                        widget(WidgetViewKind::Toggle, 3),
                        widget(
                            WidgetViewKind::Radio {
                                options: vec!["a".into(), "b".into(), "c".into()],
                            },
                            6,
                        ),
                    ],
                },
            }],
            Vec::new(),
        )
    }

    #[test]
    fn values_change_only_the_control_geometry() {
        let scene = scene();
        let palette = Palette::default();
        let mut values = PanelValues::new();
        let before = ControlGeometry::build(&scene, &values, &palette);
        let scene_generation = scene.generation();

        values.set(
            1,
            0,
            WidgetValue {
                position: 0.5,
                text: "0.5".into(),
                ..WidgetValue::default()
            },
        );
        let after = ControlGeometry::build(&scene, &values, &palette);
        // The scene, and so the tile, port, and cable buffers, did not change.
        assert_eq!(scene.generation(), scene_generation);
        assert_ne!(after.key, before.key);
        // A value arc appeared on the knob.
        assert_eq!(after.arcs.len(), before.arcs.len() + 1);
    }

    #[test]
    fn setting_the_same_value_keeps_the_generation() {
        let mut values = PanelValues::new();
        let value = WidgetValue {
            position: 0.25,
            ..WidgetValue::default()
        };
        values.set(1, 0, value.clone());
        let generation = values.generation();
        values.set(1, 0, value);
        assert_eq!(values.generation(), generation);
    }

    #[test]
    fn knob_drag_is_200_pixels_and_fine_with_shift() {
        // Up is negative screen y.
        assert_eq!(knob_drag(0.0, -100.0, false), 0.5);
        assert!((knob_drag(0.0, -100.0, true) - 0.05).abs() < 1e-6);
        assert_eq!(knob_drag(0.9, -100.0, false), 1.0);
        assert_eq!(knob_drag(0.1, 100.0, false), 0.0);
    }

    #[test]
    fn knob_steps() {
        assert!((knob_step(0.5, true, false, 0) - 0.51).abs() < 1e-6);
        assert!((knob_step(0.5, false, true, 0) - 0.499).abs() < 1e-6);
        // A whole-number parameter from 0 to 16 steps by 1/16.
        assert_eq!(knob_step(0.5, true, false, 16), 0.5625);
        assert_eq!(knob_step(1.0, true, false, 0), 1.0);
    }

    #[test]
    fn meter_colours_follow_the_decibel_zones() {
        let palette = Palette::default();
        assert_eq!(meter_colour(level_of_db(-30.0), &palette), palette.light_on);
        assert_eq!(meter_colour(level_of_db(-6.0), &palette), palette.control_value);
        assert_eq!(meter_colour(level_of_db(-1.0), &palette), palette.warning);
    }

    #[test]
    fn the_peak_line_is_never_below_the_bar() {
        let meter = Scene::new(
            vec![ModuleView {
                id: 1,
                title: String::new(),
                accent: Color::BLACK,
                position: Point::ORIGIN,
                inputs: Vec::new(),
                outputs: Vec::new(),
                panel: PanelView {
                    width: 3,
                    height: 6,
                    widgets: vec![WidgetView {
                        kind: WidgetViewKind::Meter,
                        x: 0,
                        y: 0,
                        label: String::new(),
                        hover: crate::scene::HoverText::default(),
                    }],
                },
            }],
            Vec::new(),
        );
        let mut values = PanelValues::new();
        // A stale peak below the RMS level.
        values.set(
            1,
            0,
            WidgetValue {
                position: 0.8,
                peak: 0.2,
                ..WidgetValue::default()
            },
        );
        let geometry = ControlGeometry::build(&meter, &values, &Palette::default());
        // Track, bar, peak line.
        assert_eq!(geometry.shapes.len(), 3);
        let bar = geometry.shapes[1].rect();
        let line = geometry.shapes[2].rect();
        assert!(
            line[1] + line[3] * 0.5 <= bar[1] + 0.01,
            "peak line {line:?} below bar {bar:?}"
        );
    }

    #[test]
    fn radio_rows() {
        let rect = Rectangle::new(Point::ORIGIN, Size::new(36.0, 48.0));
        assert_eq!(radio_option(rect, 3, Point::new(5.0, 5.0)), None);
        assert_eq!(radio_option(rect, 3, Point::new(5.0, GRID + 1.0)), Some(0));
        assert_eq!(radio_option(rect, 3, Point::new(5.0, 3.0 * GRID + 1.0)), Some(2));
        assert_eq!(radio_option(rect, 3, Point::new(5.0, 4.0 * GRID + 1.0)), None);
    }
}
