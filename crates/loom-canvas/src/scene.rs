//! What the canvas shows: modules, ports, and cables in world coordinates.
//!
//! The application builds a [`Scene`] from its own data model and passes it to
//! the canvas. The canvas never edits the scene. It reports gestures as
//! messages, and the application changes its model and the scene.

use iced::{Color, Point, Rectangle, Vector};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Size of the snap grid, in world units.
pub const GRID: f32 = 12.0;
/// Width of a module tile with a small or empty panel: 11 grid units.
pub const TILE_WIDTH: f32 = 132.0;
/// Width of each port label column beside a panel, in grid units.
pub const LABEL_COLUMNS: u8 = 4;
/// Space between the header and the panel.
const PANEL_TOP: f32 = 4.0;
/// Height of the title row at the top of a tile.
pub const HEADER: f32 = 22.0;
/// Height of one port row.
pub const ROW: f32 = 16.0;
/// Space below the last port row.
pub const PADDING: f32 = 6.0;
/// Radius of a port marker.
pub const PORT_RADIUS: f32 = 4.5;
/// How far from a port's centre a click still hits it.
pub const PORT_HIT_RADIUS: f32 = 9.0;

/// How a port marker is drawn. Shape is a second cue next to colour, so
/// signal types do not rely on colour alone (NFR-UX-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortShape {
    /// A circle.
    Round,
    /// A square with slightly rounded corners.
    Square,
}

/// The extra text that shows when the pointer rests on a port or control
/// (panels.md, Labels and hover text). Its first line is the label.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HoverText {
    /// What it does, in one sentence.
    pub description: String,
    /// The stable name used in patch files, or `None` when it reads the
    /// same as the label.
    pub name: Option<String>,
}

impl HoverText {
    /// Hover text for a port or parameter. The name is kept only when it
    /// differs from the label other than in letter case, so "cutoff" does
    /// not repeat under "Cutoff".
    pub fn new(label: &str, description: &str, name: &str) -> Self {
        Self {
            description: description.to_string(),
            name: (!name.eq_ignore_ascii_case(label)).then(|| name.to_string()),
        }
    }
}

/// One input or output port.
#[derive(Debug, Clone)]
pub struct PortView {
    /// Label shown next to the port when zoomed in.
    pub label: String,
    /// Shown below the label when the pointer rests on the port.
    pub hover: HoverText,
    /// Port and cable colour.
    pub colour: Color,
    /// Marker shape.
    pub shape: PortShape,
}

/// What a widget on a panel is. Mirrors `loom_core::WidgetKind`, so the
/// canvas does not depend on the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetViewKind {
    /// A dial. `steps` is the number of steps across its range for a
    /// whole-number parameter, or 0 for a continuous one.
    Knob {
        /// Steps across the range, or 0.
        steps: u32,
    },
    /// Off or on.
    Toggle,
    /// One button per option, top option first.
    Radio {
        /// Option labels.
        options: Vec<String>,
    },
    /// A brightness.
    Light,
    /// A level meter: an RMS bar and a peak line.
    Meter,
    /// A large button. Its gesture depends on the mode stored with its value.
    Button,
}

/// One widget on a panel, placed in grid units from the panel's corner.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetView {
    /// What it is.
    pub kind: WidgetViewKind,
    /// Column of its top-left corner.
    pub x: u8,
    /// Row of its top-left corner.
    pub y: u8,
    /// Label drawn with it.
    pub label: String,
    /// Shown below the label when the pointer rests on a control. Empty
    /// for lights and meters, which have no hover text.
    pub hover: HoverText,
}

impl WidgetView {
    /// Width and height in grid units. Must match `Widget::size` in
    /// `loom-core`.
    pub fn size(&self) -> (u8, u8) {
        match &self.kind {
            WidgetViewKind::Knob { .. } => (3, 3),
            WidgetViewKind::Meter => (3, 6),
            WidgetViewKind::Toggle | WidgetViewKind::Light => (3, 2),
            WidgetViewKind::Button => (6, 3),
            WidgetViewKind::Radio { options } => (3, options.len() as u8 + 1),
        }
    }

    /// True for widgets the user operates (all but lights and meters).
    pub fn is_control(&self) -> bool {
        !matches!(self.kind, WidgetViewKind::Light | WidgetViewKind::Meter)
    }
}

/// The controls and displays between a tile's port columns (ADR-0013).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PanelView {
    /// Width in grid units.
    pub width: u8,
    /// Height in grid units.
    pub height: u8,
    /// Widgets, in keyboard focus order.
    pub widgets: Vec<WidgetView>,
}

/// Size of a tile in world units, for layout code that places tiles before
/// building a scene.
pub fn tile_size(inputs: usize, outputs: usize, panel_width: u8, panel_height: u8) -> iced::Size {
    let units = (2 * LABEL_COLUMNS + panel_width).max((TILE_WIDTH / GRID) as u8);
    let rows = inputs.max(outputs).max(1) as f32;
    let ports = rows * ROW + PADDING;
    let panel = if panel_height > 0 {
        PANEL_TOP + f32::from(panel_height) * GRID + PADDING
    } else {
        0.0
    };
    let height = ((HEADER + ports.max(panel)) / GRID).ceil() * GRID;
    iced::Size::new(f32::from(units) * GRID, height)
}

/// One module tile.
#[derive(Debug, Clone)]
pub struct ModuleView {
    /// Stable ID, chosen by the application.
    pub id: u32,
    /// Title shown in the header.
    pub title: String,
    /// Header colour, usually by module category.
    pub accent: Color,
    /// Top-left corner in world coordinates.
    pub position: Point,
    /// Inputs, drawn on the left edge from the top.
    pub inputs: Vec<PortView>,
    /// Outputs, drawn on the right edge from the top.
    pub outputs: Vec<PortView>,
    /// Controls and displays between the port columns.
    pub panel: PanelView,
}

impl ModuleView {
    fn size(&self) -> iced::Size {
        tile_size(
            self.inputs.len(),
            self.outputs.len(),
            self.panel.width,
            self.panel.height,
        )
    }

    /// Tile width: the panel between two port label columns, and at least
    /// [`TILE_WIDTH`].
    pub fn width(&self) -> f32 {
        self.size().width
    }

    /// Tile height: the header plus the taller of the port rows and the
    /// panel, rounded up to the grid so tiles stack neatly.
    pub fn height(&self) -> f32 {
        self.size().height
    }

    /// The tile rectangle in world coordinates.
    pub fn bounds(&self) -> Rectangle {
        Rectangle::new(self.position, self.size())
    }

    /// Top-left corner of the panel. A narrow panel is centred between the
    /// port columns.
    pub fn panel_origin(&self) -> Point {
        let room = self.width() - f32::from(2 * LABEL_COLUMNS) * GRID;
        let spare = (room - f32::from(self.panel.width) * GRID).max(0.0);
        Point::new(
            self.position.x + f32::from(LABEL_COLUMNS) * GRID + spare * 0.5,
            self.position.y + HEADER + PANEL_TOP,
        )
    }

    /// A widget's rectangle in world coordinates.
    pub fn widget_bounds(&self, index: usize) -> Option<Rectangle> {
        let widget = self.panel.widgets.get(index)?;
        let origin = self.panel_origin();
        let (w, h) = widget.size();
        Some(Rectangle::new(
            Point::new(
                origin.x + f32::from(widget.x) * GRID,
                origin.y + f32::from(widget.y) * GRID,
            ),
            iced::Size::new(f32::from(w) * GRID, f32::from(h) * GRID),
        ))
    }

    /// Width available for a port label, in world units: half the tile
    /// without a panel, the label column with one.
    pub fn label_width(&self) -> f32 {
        if self.panel.widgets.is_empty() {
            self.width() * 0.5 - 10.0
        } else {
            f32::from(LABEL_COLUMNS) * GRID - 11.0
        }
    }

    /// Centre of an input port.
    pub fn input_position(&self, index: usize) -> Point {
        Point::new(self.position.x, self.position.y + HEADER + ROW * (index as f32 + 0.5))
    }

    /// Centre of an output port.
    pub fn output_position(&self, index: usize) -> Point {
        Point::new(
            self.position.x + self.width(),
            self.position.y + HEADER + ROW * (index as f32 + 0.5),
        )
    }
}

/// A port address: module ID, direction, and index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortRef {
    /// Module ID.
    pub module: u32,
    /// True for outputs.
    pub output: bool,
    /// Port index within its direction.
    pub index: usize,
}

/// A cable from an output to an input.
#[derive(Debug, Clone)]
pub struct CableView {
    /// Source module and output index.
    pub from: (u32, usize),
    /// Destination module and input index.
    pub to: (u32, usize),
    /// Colour at the source end.
    pub colour_from: Color,
    /// Colour at the destination end.
    pub colour_to: Color,
    /// True if the engine runs this cable through a one-sample delay to
    /// break a feedback loop. Drawn dashed.
    pub delayed: bool,
}

/// Everything the canvas draws.
///
/// Each change gives the scene a new generation number. The canvas rebuilds
/// its GPU data only when the generation changes, so a still patch costs
/// almost nothing to draw.
#[derive(Debug, Clone)]
pub struct Scene {
    modules: Vec<ModuleView>,
    index: HashMap<u32, usize>,
    cables: Vec<CableView>,
    generation: u64,
}

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

impl Default for Scene {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

impl Scene {
    /// Creates a scene. Modules are drawn in this order, last on top.
    /// Cables that refer to unknown modules or ports are skipped when drawn.
    pub fn new(modules: Vec<ModuleView>, cables: Vec<CableView>) -> Self {
        let index = modules.iter().enumerate().map(|(i, m)| (m.id, i)).collect();
        Self {
            modules,
            index,
            cables,
            generation: next_generation(),
        }
    }

    /// Changes whenever the scene changes. Unique across all scenes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// All modules, in drawing order.
    pub fn modules(&self) -> &[ModuleView] {
        &self.modules
    }

    /// All cables.
    pub fn cables(&self) -> &[CableView] {
        &self.cables
    }

    /// One module by ID.
    pub fn module(&self, id: u32) -> Option<&ModuleView> {
        self.index.get(&id).map(|&i| &self.modules[i])
    }

    /// Moves modules by a world offset.
    pub fn move_modules(&mut self, ids: &[u32], delta: Vector) {
        for id in ids {
            if let Some(&i) = self.index.get(id) {
                self.modules[i].position += delta;
            }
        }
        self.generation = next_generation();
    }

    /// Centre of a port, if it exists.
    pub fn port_position(&self, port: PortRef) -> Option<Point> {
        let module = self.module(port.module)?;
        if port.output {
            (port.index < module.outputs.len()).then(|| module.output_position(port.index))
        } else {
            (port.index < module.inputs.len()).then(|| module.input_position(port.index))
        }
    }

    /// The port view, if it exists.
    pub fn port(&self, port: PortRef) -> Option<&PortView> {
        let module = self.module(port.module)?;
        if port.output {
            module.outputs.get(port.index)
        } else {
            module.inputs.get(port.index)
        }
    }

    /// The bounding box of all modules, or `None` for an empty scene.
    pub fn bounds(&self) -> Option<Rectangle> {
        self.modules.iter().map(ModuleView::bounds).reduce(|a, b| a.union(&b))
    }

    /// What is under a world point. Ports win over tiles, because they sit
    /// on the tile edges. Tiles drawn later win over earlier ones.
    pub fn hit(&self, point: Point, port_radius: f32) -> Hit {
        let r2 = port_radius * port_radius;
        let near = |p: Point| {
            let d = point - p;
            d.x * d.x + d.y * d.y <= r2
        };
        let mut best: Option<(f32, PortRef)> = None;
        for module in self.modules.iter().rev() {
            let b = module.bounds();
            // Quick reject: ports stick out of the tile by at most the radius.
            if point.x < b.x - port_radius
                || point.x > b.x + b.width + port_radius
                || point.y < b.y
                || point.y > b.y + b.height
            {
                continue;
            }
            let ports = (0..module.inputs.len())
                .map(|i| {
                    (
                        module.input_position(i),
                        PortRef {
                            module: module.id,
                            output: false,
                            index: i,
                        },
                    )
                })
                .chain((0..module.outputs.len()).map(|i| {
                    (
                        module.output_position(i),
                        PortRef {
                            module: module.id,
                            output: true,
                            index: i,
                        },
                    )
                }));
            for (p, port) in ports {
                if near(p) {
                    let d = point - p;
                    let dist = d.x * d.x + d.y * d.y;
                    if best.is_none_or(|(bd, _)| dist < bd) {
                        best = Some((dist, port));
                    }
                }
            }
        }
        if let Some((_, port)) = best {
            return Hit::Port(port);
        }
        for module in self.modules.iter().rev() {
            if module.bounds().contains(point) {
                // Controls take the press; lights and empty panel space
                // belong to the tile, so dragging them moves it.
                let control = (0..module.panel.widgets.len()).find(|&i| {
                    module.panel.widgets[i].is_control() && module.widget_bounds(i).is_some_and(|b| b.contains(point))
                });
                return match control {
                    Some(widget) => Hit::Widget {
                        module: module.id,
                        widget,
                    },
                    None => Hit::Module(module.id),
                };
            }
        }
        // Cables last, with a tighter tolerance than ports: they are thin,
        // and empty canvas near a cable should still start a selection box.
        if let Some(index) = self.cable_at(point, port_radius * 0.5) {
            return Hit::Cable(index);
        }
        Hit::Nothing
    }

    /// The topmost cable within `tolerance` of a world point.
    fn cable_at(&self, point: Point, tolerance: f32) -> Option<usize> {
        let t2 = tolerance * tolerance;
        self.cables.iter().enumerate().rev().find_map(|(index, cable)| {
            let from = self.port_position(PortRef {
                module: cable.from.0,
                output: true,
                index: cable.from.1,
            })?;
            let to = self.port_position(PortRef {
                module: cable.to.0,
                output: false,
                index: cable.to.1,
            })?;
            let curve = cable_curve(from, to);
            // Quick reject: a cubic Bezier lies inside its control points' box.
            let (min_x, max_x) = curve
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
            let (min_y, max_y) = curve
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
            if point.x < min_x - tolerance
                || point.x > max_x + tolerance
                || point.y < min_y - tolerance
                || point.y > max_y + tolerance
            {
                return None;
            }
            let mut previous = from;
            for i in 1..=CABLE_SEGMENTS {
                let next = bezier(&curve, i as f32 / CABLE_SEGMENTS as f32);
                if distance_squared_to_segment(point, previous, next) <= t2 {
                    return Some(index);
                }
                previous = next;
            }
            None
        })
    }

    /// IDs of modules whose tiles intersect a world rectangle.
    pub fn modules_in(&self, area: Rectangle) -> Vec<u32> {
        self.modules
            .iter()
            .filter(|m| m.bounds().intersects(&area))
            .map(|m| m.id)
            .collect()
    }
}

/// Result of [`Scene::hit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// A port.
    Port(PortRef),
    /// A module tile (not on a port or control).
    Module(u32),
    /// A control on a module's panel.
    Widget {
        /// Module ID.
        module: u32,
        /// Index in the module's panel widgets.
        widget: usize,
    },
    /// A cable, by index in [`Scene::cables`].
    Cable(usize),
    /// Empty canvas.
    Nothing,
}

/// Segments per cable. Must match `SEGMENTS` in `canvas.wgsl`.
pub(crate) const CABLE_SEGMENTS: u32 = 32;

/// Control points of a cable's cubic Bezier curve.
///
/// Must match `vs_cable` in `canvas.wgsl`, so a cable is hit where it is
/// drawn. Horizontal tangents at both ends; cables that run backwards (right
/// to left, usually feedback) loop out and sag below the modules.
pub(crate) fn cable_curve(from: Point, to: Point) -> [Point; 4] {
    let span = (to.x - from.x).abs();
    let (reach, sag) = if to.x < from.x {
        (40.0 + span * 0.15, 30.0 + (span * 0.2).min(120.0))
    } else {
        ((span * 0.5).max(30.0), 0.0)
    };
    [
        from,
        Point::new(from.x + reach, from.y + sag),
        Point::new(to.x - reach, to.y + sag),
        to,
    ]
}

fn bezier(c: &[Point; 4], t: f32) -> Point {
    let u = 1.0 - t;
    let (a, b, cc, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(
        a * c[0].x + b * c[1].x + cc * c[2].x + d * c[3].x,
        a * c[0].y + b * c[1].y + cc * c[2].y + d * c[3].y,
    )
}

fn distance_squared_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let length2 = ab.x * ab.x + ab.y * ab.y;
    let t = if length2 > 0.0 {
        ((ap.x * ab.x + ap.y * ab.y) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest = Point::new(a.x + ab.x * t, a.y + ab.y * t);
    let d = p - closest;
    d.x * d.x + d.y * d.y
}

/// A label cut to fit `width` world units at `size`, with "…" when it is
/// too long. Used for port labels and panel captions.
///
/// Widths are estimated per character, not measured, so drawing never waits
/// on text layout. Narrow letters such as "i" count for less, so a short
/// word such as "Division" is not cut where an average width would cut it.
pub fn fit_label(label: &str, width: f32, size: f32) -> std::borrow::Cow<'_, str> {
    if text_width(label, size) <= width {
        return std::borrow::Cow::Borrowed(label);
    }
    let room = width / size - glyph_width('…');
    let mut used = 0.0;
    let kept: String = label
        .chars()
        .take_while(|&c| {
            used += glyph_width(c);
            used <= room
        })
        .collect();
    std::borrow::Cow::Owned(format!("{kept}…"))
}

/// Estimated width of `text` at font size `size`, in the same units as
/// `size`. See [`fit_label`].
pub fn text_width(text: &str, size: f32) -> f32 {
    text.chars().map(glyph_width).sum::<f32>() * size
}

/// Rough width of a character in a proportional sans font, as a share of
/// the font size. A little generous, so a label that fits by this count
/// fits on screen.
fn glyph_width(c: char) -> f32 {
    match c {
        'i' | 'j' | 'l' | 't' | 'f' | 'r' | 'I' | '.' | ',' | ':' | ';' | '\'' | '!' | '|' | ' ' => 0.28,
        '…' | 'M' | 'W' => 1.0,
        'm' => 0.82,
        'w' => 0.72,
        c if c.is_uppercase() => 0.66,
        _ => 0.53,
    }
}

/// Rounds a world point to the snap grid.
pub fn snap(point: Point) -> Point {
    Point::new((point.x / GRID).round() * GRID, (point.y / GRID).round() * GRID)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(label: &str) -> PortView {
        PortView {
            label: label.into(),
            hover: HoverText::default(),
            colour: Color::WHITE,
            shape: PortShape::Round,
        }
    }

    fn module(id: u32, x: f32, y: f32) -> ModuleView {
        ModuleView {
            id,
            title: format!("m{id}"),
            accent: Color::BLACK,
            position: Point::new(x, y),
            inputs: vec![port("a"), port("b")],
            outputs: vec![port("out")],
            panel: PanelView::default(),
        }
    }

    fn knob(x: u8, y: u8) -> WidgetView {
        WidgetView {
            kind: WidgetViewKind::Knob { steps: 0 },
            x,
            y,
            label: "k".into(),
            hover: HoverText::default(),
        }
    }

    #[test]
    fn panels_widen_and_heighten_tiles() {
        let mut m = module(1, 0.0, 0.0);
        assert_eq!(m.width(), TILE_WIDTH);
        m.panel = PanelView {
            width: 6,
            height: 8,
            widgets: vec![knob(0, 0), knob(3, 0)],
        };
        // 4 + 6 + 4 grid units.
        assert_eq!(m.width(), 14.0 * GRID);
        assert!(m.height() >= HEADER + 8.0 * GRID);
        assert_eq!(m.height() % GRID, 0.0);
        // Outputs move to the new right edge.
        assert_eq!(m.output_position(0).x, 14.0 * GRID);
        // The panel starts after the left label column.
        let first = m.widget_bounds(0).unwrap();
        assert_eq!(first.x, 4.0 * GRID);
        assert_eq!(first.width, 3.0 * GRID);
        assert_eq!(m.widget_bounds(1).unwrap().x, 7.0 * GRID);
    }

    #[test]
    fn narrow_panels_are_centred() {
        let mut m = module(1, 0.0, 0.0);
        m.panel = PanelView {
            width: 1,
            height: 1,
            widgets: Vec::new(),
        };
        // 11 units wide, 8 for labels, 1 for the panel, 1 spare on each side.
        assert_eq!(m.panel_origin().x, 5.0 * GRID);
    }

    #[test]
    fn controls_take_the_press_and_lights_do_not() {
        let mut m = module(1, 0.0, 0.0);
        m.panel = PanelView {
            width: 6,
            height: 3,
            widgets: vec![
                knob(0, 0),
                WidgetView {
                    kind: WidgetViewKind::Light,
                    x: 3,
                    y: 0,
                    label: "l".into(),
                    hover: HoverText::default(),
                },
            ],
        };
        let knob_centre = m.widget_bounds(0).unwrap().center();
        let light_centre = m.widget_bounds(1).unwrap().center();
        let scene = Scene::new(vec![m], Vec::new());
        assert_eq!(
            scene.hit(knob_centre, PORT_HIT_RADIUS),
            Hit::Widget { module: 1, widget: 0 }
        );
        assert_eq!(scene.hit(light_centre, PORT_HIT_RADIUS), Hit::Module(1));
    }

    #[test]
    fn long_labels_are_cut() {
        assert_eq!(fit_label("in", 37.0, 9.0), "in");
        let cut = fit_label("a-very-long-label", 37.0, 9.0);
        assert!(cut.ends_with('…'), "{cut}");
        assert!(cut.chars().count() <= 8);
    }

    #[test]
    fn narrow_letters_leave_room() {
        // A knob caption is 3 grid units wide (panels.md, Labels and hover
        // text): "Division" fits, "Bandpass" and "Resonance" are cut.
        let knob = 3.0 * GRID;
        assert_eq!(fit_label("Division", knob, 9.0), "Division");
        assert_eq!(fit_label("Bandpass", knob, 9.0), "Bandp…");
        assert!(fit_label("Resonance", knob, 9.0).ends_with('…'));
        assert_eq!(fit_label("", knob, 9.0), "");
        // A port label column beside a panel is 37 world units.
        assert_eq!(fit_label("Lowpass", 37.0, 9.0), "Lowpass");
        assert_eq!(fit_label("Velocity", 37.0, 9.0), "Velocity");
    }

    #[test]
    fn height_snaps_to_grid() {
        let m = module(1, 0.0, 0.0);
        assert_eq!(m.height() % GRID, 0.0);
        assert!(m.height() >= HEADER + 2.0 * ROW);
    }

    #[test]
    fn hits_ports_before_tiles() {
        let scene = Scene::new(vec![module(1, 0.0, 0.0), module(2, 300.0, 0.0)], Vec::new());
        let m = scene.module(1).unwrap();
        let input = m.input_position(1);
        assert_eq!(
            scene.hit(input + Vector::new(2.0, 1.0), PORT_HIT_RADIUS),
            Hit::Port(PortRef {
                module: 1,
                output: false,
                index: 1
            })
        );
        let output = m.output_position(0);
        assert_eq!(
            scene.hit(output, PORT_HIT_RADIUS),
            Hit::Port(PortRef {
                module: 1,
                output: true,
                index: 0
            })
        );
        assert_eq!(scene.hit(Point::new(60.0, 10.0), PORT_HIT_RADIUS), Hit::Module(1));
        assert_eq!(scene.hit(Point::new(200.0, 10.0), PORT_HIT_RADIUS), Hit::Nothing);
    }

    #[test]
    fn later_tiles_win() {
        let scene = Scene::new(vec![module(1, 0.0, 0.0), module(2, 40.0, 0.0)], Vec::new());
        assert_eq!(scene.hit(Point::new(60.0, 10.0), PORT_HIT_RADIUS), Hit::Module(2));
    }

    #[test]
    fn moving_changes_generation() {
        let mut scene = Scene::new(vec![module(1, 0.0, 0.0)], Vec::new());
        let before = scene.generation();
        scene.move_modules(&[1], Vector::new(GRID, 0.0));
        assert_ne!(scene.generation(), before);
        assert_eq!(scene.module(1).unwrap().position, Point::new(GRID, 0.0));
    }

    #[test]
    fn hits_cables_where_they_are_drawn() {
        let cable = CableView {
            from: (1, 0),
            to: (2, 0),
            colour_from: Color::WHITE,
            colour_to: Color::WHITE,
            delayed: false,
        };
        let scene = Scene::new(vec![module(1, 0.0, 0.0), module(2, 400.0, 200.0)], vec![cable]);
        let from = scene.module(1).unwrap().output_position(0);
        let to = scene.module(2).unwrap().input_position(0);
        // The middle of a forward curve is the midpoint of its ends.
        let middle = Point::new((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
        assert_eq!(scene.hit(middle, PORT_HIT_RADIUS), Hit::Cable(0));
        assert_eq!(
            scene.hit(middle + Vector::new(0.0, 40.0), PORT_HIT_RADIUS),
            Hit::Nothing
        );
    }

    #[test]
    fn box_selection() {
        let scene = Scene::new(vec![module(1, 0.0, 0.0), module(2, 300.0, 0.0)], Vec::new());
        let area = Rectangle::new(Point::new(-10.0, -10.0), iced::Size::new(150.0, 20.0));
        assert_eq!(scene.modules_in(area), vec![1]);
    }

    #[test]
    fn snapping() {
        assert_eq!(snap(Point::new(13.0, -7.0)), Point::new(12.0, -12.0));
    }
}
