//! The patch canvas widget: input handling and drawing.

use crate::camera::Camera;
use crate::controls::{ControlGeometry, PanelValues, focus_ring, knob_drag, knob_step, radio_option};
use crate::gpu::{Cable, CanvasPrimitive, Geometry, Overlay, Shape};
use crate::hover::{HOVER_PADDING, HOVER_TEXT_PX, HoverTarget, HoverTimer};
use crate::scene::{
    BADGE_TEXT_SIZE, GRID, Hit, HoverText, PORT_HIT_RADIUS, PORT_RADIUS, PortRef, Scene, TILE_WIDTH, WidgetView,
    WidgetViewKind,
};
use crate::theme::Palette;
use crate::tile_text::{self, Align, LABEL_SIZE, TITLE_SIZE, TextKind};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::{self, Paragraph as _, Text};
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Shell, Widget};
use iced::{Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size, Vector};
use iced::{alignment, keyboard, mouse, window};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Two presses on the same control within this time reset it.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// How far in screen pixels the pointer may move between the two clicks of
/// a double-click.
const DOUBLE_CLICK_SLOP: f32 = 4.0;

/// Titles are drawn when they would be at least this tall on screen.
const TITLE_MIN_PX: f32 = 6.5;
/// Port labels are drawn when they would be at least this tall on screen.
const LABEL_MIN_PX: f32 = 7.5;

/// Opacity of cables that are not in focus. It never changes with hover or
/// selection, so moving the pointer only changes the cables it points at,
/// never the whole patch.
const CABLE_OPACITY: f32 = 0.4;

/// A gesture on the canvas. The application applies it to its model.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasEvent {
    /// Replace the selection.
    Select(Vec<u32>),
    /// Move modules by a world offset (always whole grid steps).
    Move {
        /// Modules to move.
        ids: Vec<u32>,
        /// Offset in world units.
        delta: Vector,
    },
    /// Connect an output to an input (replacing any cable into that input).
    Connect {
        /// The output.
        from: PortRef,
        /// The input.
        to: PortRef,
    },
    /// Remove the cable into this input.
    Disconnect(PortRef),
    /// The range arc of a knob was dragged: change the depth of one of its
    /// assignments by `delta`, as a share of the knob from -1 to 1
    /// (`docs/design/modulation.md`, Arc drag). The app picks the
    /// assignment, from `pinned` first. Sent between a
    /// [`CanvasEvent::GestureStart`] and [`CanvasEvent::GestureEnd`].
    DragDepth {
        /// Module ID of the knob.
        module: u32,
        /// Index in the module's panel widgets.
        widget: usize,
        /// The output whose lines are pinned, if any.
        pinned: Option<PortRef>,
        /// Change of depth since the last event.
        delta: f32,
    },
    /// An output was dropped on a control that takes modulation: assign the
    /// output to the control's parameter (`docs/design/modulation.md`,
    /// Making an assignment). Not sent in MIDI learn mode.
    Assign {
        /// The output.
        from: PortRef,
        /// Module ID of the control.
        module: u32,
        /// Index in the module's panel widgets.
        widget: usize,
    },
    /// Delete modules.
    Delete(Vec<u32>),
    /// Set a control on a module's panel. `position` is the knob position
    /// from 0 to 1, 0 or 1 for a toggle, or the option index for radio
    /// buttons. A button reports [`CanvasEvent::ButtonPressed`] instead.
    SetControl {
        /// Module ID.
        module: u32,
        /// Index in the module's panel widgets.
        widget: usize,
        /// The new position.
        position: f32,
    },
    /// A button was pressed, by the pointer or by Space or Enter.
    ///
    /// The app decides the parameter value from the button's mode. The
    /// canvas does not.
    ButtonPressed {
        /// Module ID.
        module: u32,
        /// Index in the module's panel widgets.
        widget: usize,
    },
    /// A button hold ended.
    ///
    /// Pointer up sends this, including after the pointer has left the
    /// button. So does another mouse button, key up, and the window losing
    /// focus. The app ignores it for a toggle and clears a momentary gate.
    ButtonReleased {
        /// Module ID.
        module: u32,
        /// Index in the module's panel widgets.
        widget: usize,
    },
    /// An arrow key on a focused button. `high` is up or right.
    ButtonArrow {
        /// Module ID.
        module: u32,
        /// Index in the module's panel widgets.
        widget: usize,
        /// Up or right. Down or left is false.
        high: bool,
    },
    /// Reset a control to its parameter's default (double-click).
    ResetControl {
        /// Module ID.
        module: u32,
        /// Index in the module's panel widgets.
        widget: usize,
    },
    /// Open the module browser (`docs/design/module-browser.md`): `A` was
    /// pressed with the pointer over the canvas, empty canvas was
    /// double-clicked, or the browser key changed.
    OpenBrowser {
        /// Where a new module goes, in world units, not yet snapped.
        world: Point,
        /// Where the browser opens, in window coordinates.
        at: Point,
        /// The canvas area, in window coordinates, so the browser can stay
        /// inside it.
        area: Rectangle,
    },
    /// Ctrl+V or Cmd+V: paste the clipboard (`docs/design/clipboard.md`).
    /// `world` is the pointer when it is over the canvas, and the centre of
    /// the view when it is not, in world units, not yet snapped.
    Paste {
        /// Where the top-left corner of the pasted modules goes.
        world: Point,
    },
    /// The modules whose lights or meters are on screen, at a zoom where
    /// they are drawn, changed. Only these need their values read.
    VisibleDisplays(Vec<u32>),
    /// In MIDI learn mode, a control was clicked: arm it, or disarm it if it
    /// is armed. Its value does not change (`docs/design/midi-learn.md`).
    Learn {
        /// Module ID.
        module: u32,
        /// Index in the module's panel widgets.
        widget: usize,
    },
    /// Space was pressed and no panel control is focused. The app toggles
    /// transport Run. Enter does not send this, and neither does Space while
    /// a control is focused.
    ToggleRun,
    /// A drag began: turning a knob, moving tiles, or pulling a cable. Every
    /// edit until [`CanvasEvent::GestureEnd`] is one undo step
    /// (`docs/design/undo.md`, Gestures). A held arrow key on a focused
    /// control is a gesture too.
    GestureStart,
    /// The gesture ended. It is always sent after a start, also when the
    /// pointer is released outside the window or the window loses focus.
    GestureEnd,
}

/// What MIDI learn mode shows on the canvas. While the canvas has one, a
/// click on a control arms it instead of changing it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LearnView {
    /// The armed control: module ID and widget index.
    pub armed: Option<(u32, usize)>,
    /// Mapping labels, such as `CC 74`, by module ID and widget index. They
    /// show in place of the control's caption.
    pub labels: BTreeMap<(u32, usize), String>,
}

/// The patch canvas. Draws a [`Scene`] and reports gestures as
/// [`CanvasEvent`]s.
pub struct PatchCanvas<'a, Message> {
    scene: &'a Scene,
    selection: &'a BTreeSet<u32>,
    values: Option<&'a PanelValues>,
    palette: Palette,
    on_event: Box<dyn Fn(CanvasEvent) -> Message + 'a>,
    frame_key: u64,
    browser_key: u64,
    animate: bool,
    interactive: bool,
    learn: Option<&'a LearnView>,
    still: Option<Still>,
}

/// How a still image of the canvas starts: its camera and the control with
/// keyboard focus. See [`PatchCanvas::still`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Still {
    /// The view. It replaces the framing the canvas does when it first
    /// appears.
    pub camera: Camera,
    /// Module ID and widget index of the control with keyboard focus. It
    /// draws as after a Tab press: with the focus ring and its value.
    pub focus: Option<(u32, usize)>,
}

/// Where the canvas is in the window and how it looks at the world. The
/// canvas reports it to widget operations as custom state, so an operation
/// can turn a world point (a port, a control) into window coordinates, for
/// example to send pointer events in a headless test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasView {
    /// The canvas bounds in the window.
    pub bounds: Rectangle,
    /// The current camera.
    pub camera: Camera,
}

impl CanvasView {
    /// A world point in window coordinates.
    pub fn to_window(&self, world: Point) -> Point {
        let s = self.camera.to_screen(world);
        Point::new(self.bounds.x + s.x, self.bounds.y + s.y)
    }
}

impl<'a, Message> PatchCanvas<'a, Message> {
    /// Creates a canvas for a scene and the current selection.
    pub fn new(scene: &'a Scene, selection: &'a BTreeSet<u32>, on_event: impl Fn(CanvasEvent) -> Message + 'a) -> Self {
        Self {
            scene,
            selection,
            values: None,
            palette: Palette::default(),
            on_event: Box::new(on_event),
            frame_key: 0,
            browser_key: 0,
            animate: false,
            interactive: true,
            learn: None,
            still: None,
        }
    }

    /// Starts the canvas with this camera and focus instead of framing the
    /// scene, so one drawn frame shows a chosen state. For saving the
    /// canvas as an image without a window (`loom-app --screenshot`).
    /// Only the first appearance uses it: later frames keep the state as
    /// usual.
    pub fn still(mut self, still: Still) -> Self {
        self.still = Some(still);
        self
    }

    /// Turns on MIDI learn mode, with what it shows.
    pub fn learn(mut self, learn: Option<&'a LearnView>) -> Self {
        self.learn = learn;
        self
    }

    /// Sets the live values of panel controls and lights.
    pub fn values(mut self, values: &'a PanelValues) -> Self {
        self.values = Some(values);
        self
    }

    /// Sets the colours.
    pub fn palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }

    /// The canvas frames the whole scene when it first appears and whenever
    /// this key changes (for example after loading a patch).
    pub fn frame_key(mut self, key: u64) -> Self {
        self.frame_key = key;
        self
    }

    /// The canvas sends [`CanvasEvent::OpenBrowser`] at the centre of the
    /// view whenever this key changes, but not when it first appears. The
    /// app uses it for a toolbar button, because only the canvas knows its
    /// camera.
    pub fn browser_key(mut self, key: u64) -> Self {
        self.browser_key = key;
        self
    }

    /// While false, the canvas ignores the keyboard and the pointer, and
    /// lets go of any hold or drag. For a pop-up that takes the keys, such
    /// as the module browser.
    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    /// Moves the camera continuously in a slow loop. For measuring frame
    /// rate while panning and zooming (ADR-0010 performance gates).
    pub fn animate(mut self, animate: bool) -> Self {
        self.animate = animate;
        self
    }

    /// True if dropping `from` on this control makes an assignment: an
    /// output, a control that takes modulation, and not in MIDI learn mode,
    /// where touching a control arms it instead.
    fn takes_assignment(&self, from: PortRef, module: u32, widget: usize) -> bool {
        from.output
            && self.learn.is_none()
            && self
                .scene
                .module(module)
                .and_then(|m| m.panel.widgets.get(widget))
                .is_some_and(WidgetView::accepts_modulation)
    }

    fn emit(&self, shell: &mut Shell<'_, Message>, event: CanvasEvent) {
        shell.publish((self.on_event)(event));
    }

    /// Starts a gesture and sets the interaction that lasts through it.
    fn begin(&self, state: &mut State, interaction: Interaction, shell: &mut Shell<'_, Message>) {
        self.emit(shell, CanvasEvent::GestureStart);
        state.interaction = interaction;
    }

    /// Ends the gesture of the current interaction, if it has one, and the
    /// gesture of a held arrow key. Used when the interaction is replaced or
    /// cut off, so a start always has an end.
    fn end_gestures(&self, state: &mut State, shell: &mut Shell<'_, Message>) {
        if matches!(
            state.interaction,
            Interaction::Moving { .. }
                | Interaction::Turning { .. }
                | Interaction::Wiring { .. }
                | Interaction::Depth { .. }
        ) {
            state.interaction = Interaction::Idle;
            self.emit(shell, CanvasEvent::GestureEnd);
        }
        if std::mem::take(&mut state.arrow_gesture) {
            self.emit(shell, CanvasEvent::GestureEnd);
        }
    }
}

/// What the modulation lines depend on: the scene (assignments and module
/// positions), the pin, what the pointer is on, and the selection.
#[derive(Debug, Clone, PartialEq)]
struct LinesKey {
    scene: u64,
    pinned: Option<PortRef>,
    hover: Option<Hit>,
    selection: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
enum Interaction {
    #[default]
    Idle,
    Panning {
        last: Point,
    },
    Moving {
        ids: Vec<u32>,
        start: Point,
        applied: Vector,
        /// A toggle or radio press: applied on release if the tile did not
        /// move, so dragging these moves the tile.
        click: Option<(u32, usize, f32)>,
    },
    Turning {
        module: u32,
        widget: usize,
        /// Pointer height on screen at the last move.
        last_y: f32,
        position: f32,
    },
    /// A button is held. Pointer up reports a release, wherever the pointer
    /// is, so a release always follows the press. The tile does not move.
    /// The app decides whether that release changes a parameter.
    Holding {
        module: u32,
        widget: usize,
    },
    Wiring {
        from: PortRef,
        /// True when the drag began by pulling a cable off an input. Then a
        /// drop on a control assigns nothing: the user is discarding the
        /// cable (modulation.md, Making an assignment).
        pulled: bool,
    },
    /// Dragging a knob's range arc: its depth, not its value.
    Depth {
        module: u32,
        widget: usize,
        /// Pointer height on screen at the last move.
        last_y: f32,
    },
    Selecting {
        start: Point,
        current: Point,
        base: BTreeSet<u32>,
    },
}

#[derive(Default)]
struct State {
    camera: Camera,
    framed_key: Option<u64>,
    interaction: Interaction,
    hover: Option<Hit>,
    /// Cursor in world coordinates, while over the canvas or dragging.
    cursor: Option<Point>,
    modifiers: keyboard::Modifiers,
    geometry: RefCell<Option<Arc<Geometry>>>,
    controls: RefCell<Option<Arc<ControlGeometry>>>,
    animation: Option<(Instant, Camera)>,
    /// The control with keyboard focus: module ID and widget index.
    focus: Option<(u32, usize)>,
    /// True when the focus came from Tab, so its hover text shows. A click
    /// also focuses a control, but the pointer is on it then.
    tab_focus: bool,
    /// True from a Tab press until the pointer moves: the focused control's
    /// hover text wins over the port or control under a resting pointer.
    focus_first: bool,
    /// The output whose modulation lines stay on screen: its badge was
    /// clicked (`docs/design/modulation.md`, Lines).
    pinned: Option<PortRef>,
    /// The modulation lines and outlines last built, and what they were
    /// built from. They are rebuilt only when that changes, not every
    /// frame (ADR-0010).
    lines: RefCell<Option<(LinesKey, Overlay)>>,
    /// When hover text shows (panels.md, Labels and hover text).
    hover_text: HoverTimer,
    /// The last press on a control, for double-click.
    last_click: Option<(Instant, u32, usize)>,
    /// The last press on empty canvas, in screen coordinates, for
    /// double-click.
    last_empty_click: Option<(Instant, Point)>,
    /// The browser key last seen.
    browser_key: Option<u64>,
    /// Buttons held from the keyboard, one slot for Space and one for Enter.
    ///
    /// Each key is its own hold. Releasing one leaves a button high while the
    /// other key is still down on it. Replacing a key's hold releases the
    /// button that key was holding, when nothing else is holding it.
    key_holds: [Option<KeyHold>; 2],
    /// Modules with lights on screen, as last reported.
    reported_lights: Option<Vec<u32>>,
    /// True while an arrow key is down on a control: one gesture from key
    /// down to key up, however often it repeats.
    arrow_gesture: bool,
}

impl State {
    fn hit_radius(&self) -> f32 {
        // At least 7 screen pixels, so ports stay easy to grab when zoomed
        // out. At most a quarter of a tile's width, so the middle of a tile
        // always grabs the tile, not a port.
        PORT_HIT_RADIUS.max(7.0 / self.camera.zoom).min(TILE_WIDTH * 0.25)
    }
}

/// One key holding one button down.
#[derive(Clone, Copy)]
struct KeyHold {
    key: keyboard::key::Named,
    module: u32,
    widget: usize,
}

/// True when a press at `screen` is the second click of a double-click
/// after the `last` press.
fn double_click(last: Option<(Instant, Point)>, screen: Point) -> bool {
    last.is_some_and(|(at, point)| at.elapsed() < DOUBLE_CLICK && point.distance(screen) < DOUBLE_CLICK_SLOP)
}

fn relative(cursor: mouse::Cursor, bounds: Rectangle) -> Option<Point> {
    cursor.position().map(|p| Point::new(p.x - bounds.x, p.y - bounds.y))
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for PatchCanvas<'_, Message>
where
    Renderer: iced_wgpu::primitive::Renderer + text::Renderer<Font = Font>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        let state = match self.still {
            Some(still) => State {
                camera: still.camera,
                // Already framed, so the first redraw keeps this camera.
                framed_key: Some(self.frame_key),
                focus: still.focus,
                tab_focus: still.focus.is_some(),
                ..State::default()
            },
            None => State::default(),
        };
        tree::State::new(state)
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        self.handle(state, event, bounds, cursor, shell);
        self.sync_hover_text(state, event, shell);
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let mut view = CanvasView {
            bounds: layout.bounds(),
            camera: state.camera,
        };
        operation.custom(None, layout.bounds(), &mut view);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        match state.interaction {
            Interaction::Panning { .. } | Interaction::Moving { .. } => mouse::Interaction::Grabbing,
            Interaction::Turning { .. } | Interaction::Depth { .. } => mouse::Interaction::ResizingVertically,
            Interaction::Holding { .. } => mouse::Interaction::Pointer,
            Interaction::Wiring { .. } => mouse::Interaction::Crosshair,
            Interaction::Selecting { .. } => mouse::Interaction::Crosshair,
            Interaction::Idle if cursor.is_over(layout.bounds()) => match state.hover {
                Some(Hit::Port(_) | Hit::Badge(_)) => mouse::Interaction::Pointer,
                Some(Hit::Widget { module, widget }) => match self.widget_kind(module, widget) {
                    Some(WidgetViewKind::Knob { .. }) => mouse::Interaction::ResizingVertically,
                    _ => mouse::Interaction::Pointer,
                },
                Some(Hit::Module(_)) => mouse::Interaction::Grab,
                _ => mouse::Interaction::default(),
            },
            Interaction::Idle => mouse::Interaction::default(),
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();

        let geometry = {
            let mut cache = state.geometry.borrow_mut();
            match cache.as_ref() {
                Some(g) if g.generation == self.scene.generation() => g.clone(),
                _ => {
                    let built = Arc::new(Geometry::build(self.scene, &self.palette));
                    *cache = Some(built.clone());
                    built
                }
            }
        };

        // Controls follow the titles: drawn as shapes once titles show, and
        // not at all when zoomed further out (panels.md, Level of detail).
        let empty = PanelValues::default();
        let values = self.values.unwrap_or(&empty);
        let controls = (detail(state.camera.zoom) != Detail::Far).then(|| {
            let key = (self.scene.generation(), values.generation());
            let mut cache = state.controls.borrow_mut();
            match cache.as_ref() {
                Some(c) if c.key == key => c.clone(),
                _ => {
                    let built = Arc::new(ControlGeometry::build(self.scene, values, &self.palette));
                    *cache = Some(built.clone());
                    built
                }
            }
        });

        // Text too small to draw leaves a bar in its place.
        let bars = match detail(state.camera.zoom) {
            Detail::Close => geometry.bars(false, false),
            Detail::Middle => geometry.bars(true, false),
            Detail::Far => geometry.bars(true, true),
        };

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..renderer::Quad::default()
            },
            self.palette.background,
        );
        renderer.draw_primitive(
            bounds,
            CanvasPrimitive {
                geometry,
                controls,
                overlay: self.overlay_in(state, state.camera.visible(bounds.size())),
                bars,
                camera: state.camera,
                cable_dim: CABLE_OPACITY,
            },
        );
        self.draw_text(state, values, renderer, bounds);
        self.draw_hover_text(state, renderer, bounds);
    }
}

impl<Message> PatchCanvas<'_, Message> {
    /// Handles one event: everything but hover text.
    fn handle(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, Message>,
    ) {
        let over = cursor.is_over(bounds);
        let screen = relative(cursor, bounds);

        if !self.interactive {
            // Modifiers still count, so Shift held while the pop-up closes
            // is known on the next click.
            if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
                state.modifiers = *modifiers;
            }
            // A pop-up has the keys. Nothing may stay held or half dragged
            // behind it. This runs on any event, redraws too, because the
            // pop-up takes the pointer and key events before the canvas
            // sees them. Each step does nothing once nothing is held.
            self.release_pointer_hold(state, shell);
            self.release_every_key_hold(state, shell);
            self.end_gestures(state, shell);
            state.interaction = Interaction::Idle;
            if !matches!(event, Event::Window(_)) {
                return;
            }
        }

        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                if state
                    .browser_key
                    .replace(self.browser_key)
                    .is_some_and(|key| key != self.browser_key)
                {
                    let centre = Point::new(bounds.width / 2.0, bounds.height / 2.0);
                    self.open_browser(state.camera.to_world(centre), centre, bounds, shell);
                }
                if state.framed_key != Some(self.frame_key) && bounds.width > 0.0 {
                    if let Some(world) = self.scene.bounds() {
                        state.camera.frame(world, bounds.size());
                    }
                    state.framed_key = Some(self.frame_key);
                    // A new frame key means a new patch: its lights have to
                    // be reported again, even if the list looks the same.
                    state.reported_lights = None;
                    state.animation = None;
                    // Module IDs repeat between patches, so an old pin could
                    // land on an unrelated output.
                    state.pinned = None;
                    shell.request_redraw();
                }
                if self.animate {
                    let (start, base) = *state.animation.get_or_insert((*now, state.camera));
                    let t = (*now - start).as_secs_f32();
                    state.camera = animated(base, bounds.size(), t);
                    shell.request_redraw();
                } else if let Some((_, base)) = state.animation.take() {
                    state.camera = base;
                    shell.request_redraw();
                }
                let lights = visible_lights(self.scene, &state.camera, bounds.size());
                if state.reported_lights.as_ref() != Some(&lights) {
                    state.reported_lights = Some(lights.clone());
                    self.emit(shell, CanvasEvent::VisibleDisplays(lights));
                }
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key, modifiers, repeat, ..
            }) if self.focus_keys(state, key, *modifiers, *repeat, over, shell) => {}
            Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) => {
                self.release_key(state, key, shell);
            }
            // Paste works with the pointer anywhere in the window. Only the
            // canvas knows where the pointer is in the patch, and where the
            // view's centre is.
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(c),
                modifiers,
                ..
            }) if modifiers.command() && c.eq_ignore_ascii_case("v") => {
                let at = screen
                    .filter(|_| over)
                    .unwrap_or(Point::new(bounds.width / 2.0, bounds.height / 2.0));
                self.emit(
                    shell,
                    CanvasEvent::Paste {
                        world: state.camera.to_world(at),
                    },
                );
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) if over => {
                use keyboard::key::Named;
                match key.as_ref() {
                    keyboard::Key::Named(Named::Delete | Named::Backspace) if !self.selection.is_empty() => {
                        self.emit(shell, CanvasEvent::Delete(self.selection.iter().copied().collect()));
                        shell.capture_event();
                    }
                    keyboard::Key::Named(Named::Home) | keyboard::Key::Character("f") => {
                        if let Some(world) = self.scene.bounds() {
                            state.camera.frame(world, bounds.size());
                            shell.request_redraw();
                        }
                        shell.capture_event();
                    }
                    keyboard::Key::Character("a") if modifiers.command() => {
                        let all = self.scene.modules().iter().map(|m| m.id).collect();
                        self.emit(shell, CanvasEvent::Select(all));
                        shell.capture_event();
                    }
                    keyboard::Key::Character("a") if modifiers.is_empty() => {
                        if let Some(screen) = screen {
                            self.open_browser(state.camera.to_world(screen), screen, bounds, shell);
                        }
                        shell.capture_event();
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if over => {
                let Some(screen) = screen else { return };
                match *delta {
                    // A mouse wheel zooms.
                    mouse::ScrollDelta::Lines { y, .. } => {
                        state.camera.zoom_at(screen, 1.15_f32.powf(y));
                    }
                    // A trackpad pans, or zooms with Ctrl or Cmd held.
                    mouse::ScrollDelta::Pixels { x, y } => {
                        if state.modifiers.command() || state.modifiers.control() {
                            state.camera.zoom_at(screen, (y * 0.01).exp());
                        } else {
                            state.camera.pan(Vector::new(x, y));
                        }
                    }
                }
                state.cursor = Some(state.camera.to_world(screen));
                // The view moved under the pointer: hover text hides, and
                // shows again only after the pointer rests anew.
                state.hover_text.restart(Instant::now());
                state.hover = Some(self.scene.hit(state.camera.to_world(screen), state.hit_radius()));
                shell.request_redraw();
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonPressed(button)) if over => {
                let Some(screen) = screen else { return };
                // A second button must end a hold. Replacing the interaction
                // without a release would leave a momentary gate high.
                self.release_pointer_hold(state, shell);
                self.end_gestures(state, shell);
                let world = state.camera.to_world(screen);
                match button {
                    mouse::Button::Middle | mouse::Button::Right => {
                        state.interaction = Interaction::Panning { last: screen };
                    }
                    mouse::Button::Left => {
                        self.press(state, world, screen, bounds, shell);
                    }
                    _ => {}
                }
                shell.request_redraw();
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                state.focus_first = false;
                let Some(screen) = screen else { return };
                let world = state.camera.to_world(screen);
                state.cursor = Some(world);
                let mut redraw = false;
                match &mut state.interaction {
                    Interaction::Panning { last } => {
                        let delta = screen - *last;
                        *last = screen;
                        state.camera.pan(delta);
                        redraw = true;
                    }
                    Interaction::Moving {
                        ids, start, applied, ..
                    } => {
                        let total = world - *start;
                        let snapped = Vector::new((total.x / GRID).round() * GRID, (total.y / GRID).round() * GRID);
                        if snapped != *applied {
                            let delta = snapped - *applied;
                            *applied = snapped;
                            let ids = ids.clone();
                            self.emit(shell, CanvasEvent::Move { ids, delta });
                        }
                    }
                    Interaction::Turning {
                        module,
                        widget,
                        last_y,
                        position,
                    } => {
                        let next = knob_drag(*position, screen.y - *last_y, state.modifiers.shift());
                        *last_y = screen.y;
                        if next != *position {
                            *position = next;
                            let event = CanvasEvent::SetControl {
                                module: *module,
                                widget: *widget,
                                position: next,
                            };
                            self.emit(shell, event);
                        }
                    }
                    Interaction::Depth { module, widget, last_y } => {
                        let dy = screen.y - *last_y;
                        *last_y = screen.y;
                        // The knob's own drag scale: up raises, down lowers.
                        let delta = knob_drag(0.5, dy, state.modifiers.shift()) - 0.5;
                        if delta != 0.0 {
                            let event = CanvasEvent::DragDepth {
                                module: *module,
                                widget: *widget,
                                pinned: self.pin(state),
                                delta,
                            };
                            self.emit(shell, event);
                        }
                    }
                    Interaction::Holding { .. } | Interaction::Idle => {}
                    Interaction::Wiring { .. } => redraw = true,
                    Interaction::Selecting { current, .. } => {
                        *current = world;
                        redraw = true;
                    }
                }
                let hover = over.then(|| self.scene.hit(world, state.hit_radius()));
                if hover != state.hover {
                    state.hover = hover;
                    redraw = true;
                }
                if redraw {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                // The left button is what started the hold. Releasing another
                // button, one that was already down, must not end it.
                if *button != mouse::Button::Left && matches!(state.interaction, Interaction::Holding { .. }) {
                    return;
                }
                let interaction = std::mem::take(&mut state.interaction);
                let gesture = matches!(
                    interaction,
                    Interaction::Moving { .. }
                        | Interaction::Turning { .. }
                        | Interaction::Wiring { .. }
                        | Interaction::Depth { .. }
                );
                let world = screen.map(|s| state.camera.to_world(s));
                match interaction {
                    Interaction::Wiring { from, pulled } => {
                        match world.map(|w| self.scene.hit(w, state.hit_radius())) {
                            Some(Hit::Port(to)) => {
                                let pair = if from.output { (from, to) } else { (to, from) };
                                if pair.0.output && !pair.1.output {
                                    self.emit(
                                        shell,
                                        CanvasEvent::Connect {
                                            from: pair.0,
                                            to: pair.1,
                                        },
                                    );
                                }
                            }
                            Some(Hit::Widget { module, widget })
                                if !pulled && self.takes_assignment(from, module, widget) =>
                            {
                                self.emit(shell, CanvasEvent::Assign { from, module, widget });
                            }
                            _ => {}
                        }
                    }
                    Interaction::Selecting { start, current, base } => {
                        let area = Rectangle::new(
                            Point::new(start.x.min(current.x), start.y.min(current.y)),
                            Size::new((start.x - current.x).abs(), (start.y - current.y).abs()),
                        );
                        let mut selection = base;
                        selection.extend(self.scene.modules_in(area));
                        self.emit(shell, CanvasEvent::Select(selection.into_iter().collect()));
                    }
                    Interaction::Moving {
                        applied,
                        click: Some((module, widget, position)),
                        ..
                    } if applied == Vector::ZERO => {
                        self.emit(
                            shell,
                            CanvasEvent::SetControl {
                                module,
                                widget,
                                position,
                            },
                        );
                    }
                    Interaction::Holding { module, widget } => {
                        if !Self::key_holds_button(state, module, widget) {
                            self.emit(shell, CanvasEvent::ButtonReleased { module, widget });
                        }
                    }
                    Interaction::Idle
                    | Interaction::Panning { .. }
                    | Interaction::Moving { .. }
                    | Interaction::Turning { .. }
                    | Interaction::Depth { .. } => {}
                }
                if gesture {
                    self.emit(shell, CanvasEvent::GestureEnd);
                }
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                self.release_pointer_hold(state, shell);
                if state.hover.take().is_some() {
                    shell.request_redraw();
                }
            }
            Event::Window(window::Event::Unfocused) => {
                // A key release never arrives if the window loses focus
                // while Space or Enter is held. End every hold here.
                self.release_pointer_hold(state, shell);
                self.release_every_key_hold(state, shell);
                self.end_gestures(state, shell);
                shell.request_redraw();
            }
            _ => {}
        }
    }

    /// Updates hover text after an event (panels.md, Labels and hover
    /// text). It asks for a frame only when the text appears or hides, and
    /// for a single timed frame while it waits, so it never draws while
    /// nothing changes.
    fn sync_hover_text(&self, state: &mut State, event: &Event, shell: &mut Shell<'_, Message>) {
        let (now, redrawing) = match event {
            Event::Window(window::Event::RedrawRequested(now)) => (*now, true),
            _ => (Instant::now(), false),
        };
        let target = self.hover_target(state);
        let mut changed = state.hover_text.rest(target, now);
        // This frame is being drawn already, so text due now needs no other.
        changed |= state.hover_text.tick(now) && !redrawing;
        if changed {
            shell.request_redraw();
        }
        if let Some(due) = state.hover_text.due() {
            shell.request_redraw_at(due);
        }
    }

    /// What hover text would be about now: the port or control under the
    /// pointer, or else the control focused with Tab. Right after a Tab
    /// press, the focused control comes first, until the pointer moves.
    /// Nothing during a drag.
    fn hover_target(&self, state: &State) -> Option<HoverTarget> {
        if !matches!(state.interaction, Interaction::Idle) {
            return None;
        }
        let focused = state.focus.filter(|_| state.tab_focus).and_then(|(module, widget)| {
            let selected = self.selection.len() == 1 && self.selection.contains(&module);
            (selected && self.is_control(module, widget)).then_some(HoverTarget::Control(module, widget))
        });
        if state.focus_first && focused.is_some() {
            return focused;
        }
        match state.hover {
            // A port in hit range wins over a label under the pointer, even
            // another port's label. That port has the hover ring, and a
            // press starts a cable from it, so the hover text must name it
            // too.
            Some(Hit::Port(port)) => Some(HoverTarget::Port(port)),
            Some(Hit::Widget { module, widget }) if self.is_control(module, widget) => {
                Some(HoverTarget::Control(module, widget))
            }
            // A port's label, or the bar standing in for it, hovers like
            // the port (panels.md, Hover text).
            Some(Hit::Module(_)) => state
                .cursor
                .and_then(|world| tile_text::port_label_at(self.scene, world))
                .map(HoverTarget::Port)
                .or(focused),
            _ => focused,
        }
    }

    fn is_control(&self, module: u32, widget: usize) -> bool {
        self.scene
            .module(module)
            .and_then(|m| m.panel.widgets.get(widget))
            .is_some_and(|w| w.is_control())
    }

    fn widget_kind(&self, module: u32, widget: usize) -> Option<&WidgetViewKind> {
        Some(&self.scene.module(module)?.panel.widgets.get(widget)?.kind)
    }

    /// The pinned output, while its badge shows. A pin on an output that
    /// lost its last assignment, or whose module is gone, counts as none:
    /// it draws nothing, so it must not take Escape or pick an arc drag.
    fn pin(&self, state: &State) -> Option<PortRef> {
        state.pinned.filter(|p| {
            self.scene
                .module(p.module)
                .and_then(|m| m.badge_bounds(p.index))
                .is_some()
        })
    }

    /// The modulated knob whose range ring is under `world`, if any.
    fn range_ring_at(&self, world: Point) -> Option<(u32, usize)> {
        let module = self.scene.modules().iter().rev().find(|m| m.bounds().contains(world))?;
        (0..module.panel.widgets.len())
            .find(|&w| self.on_range_ring(module.id, w, world))
            .map(|w| (module.id, w))
    }

    /// True if `world` is on the ring around a knob where its range arc is
    /// drawn, and the knob has an enabled assignment: there a drag changes
    /// depth, not the value. This holds when the arc has zero length and
    /// the knob shows the dot, so depth 0 can be dragged back out of.
    fn on_range_ring(&self, module: u32, widget: usize, world: Point) -> bool {
        let Some(m) = self.scene.module(module) else {
            return false;
        };
        let (Some(rect), Some(WidgetViewKind::Knob { .. })) =
            (m.widget_bounds(widget), self.widget_kind(module, widget))
        else {
            return false;
        };
        let modulated = self.values.and_then(|v| v.get(module, widget)).is_some_and(|v| {
            matches!(
                v.modulation,
                crate::controls::ModulationMark::Range { .. } | crate::controls::ModulationMark::Dot
            )
        });
        let d = world - crate::controls::knob_centre(rect);
        let distance = (d.x * d.x + d.y * d.y).sqrt();
        modulated && (crate::controls::RANGE_RING_INNER..=crate::controls::RANGE_RING_OUTER).contains(&distance)
    }

    fn position(&self, module: u32, widget: usize) -> f32 {
        self.values.map_or(0.0, |v| v.position(module, widget))
    }

    /// True when Space or Enter is still holding this button.
    fn key_holds_button(state: &State, module: u32, widget: usize) -> bool {
        state
            .key_holds
            .iter()
            .flatten()
            .any(|hold| hold.module == module && hold.widget == widget)
    }

    /// True when the pointer is still down on this button.
    fn pointer_holds_button(state: &State, module: u32, widget: usize) -> bool {
        matches!(
            state.interaction,
            Interaction::Holding {
                module: held,
                widget: held_widget,
            } if held == module && held_widget == widget
        )
    }

    /// Ends a pointer hold, if there is one. The release is reported only
    /// when no key is still holding that button.
    fn release_pointer_hold(&self, state: &mut State, shell: &mut Shell<'_, Message>) {
        if let Interaction::Holding { module, widget } = state.interaction {
            state.interaction = Interaction::Idle;
            if !Self::key_holds_button(state, module, widget) {
                self.emit(shell, CanvasEvent::ButtonReleased { module, widget });
            }
        }
    }

    /// Space or Enter, the keys that hold a button.
    fn button_key(key: &keyboard::Key) -> Option<keyboard::key::Named> {
        use keyboard::key::Named;
        match key.as_ref() {
            keyboard::Key::Named(named @ (Named::Space | Named::Enter)) => Some(named),
            _ => None,
        }
    }

    /// Starts a hold for one key. If that key was holding another button, and
    /// nothing else is, that button is released first.
    fn press_button_key(
        &self,
        state: &mut State,
        key: keyboard::key::Named,
        module: u32,
        widget: usize,
        shell: &mut Shell<'_, Message>,
    ) {
        let same = state
            .key_holds
            .iter()
            .flatten()
            .any(|hold| hold.key == key && hold.module == module && hold.widget == widget);
        if same {
            return;
        }
        self.release_button_key(state, key, shell);
        self.emit(shell, CanvasEvent::ButtonPressed { module, widget });
        if let Some(slot) = state.key_holds.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(KeyHold { key, module, widget });
        }
    }

    /// Ends one key's hold. The button stays down while another key holds it.
    fn release_button_key(&self, state: &mut State, key: keyboard::key::Named, shell: &mut Shell<'_, Message>) {
        let Some(index) = state
            .key_holds
            .iter()
            .position(|slot| slot.is_some_and(|hold| hold.key == key))
        else {
            return;
        };
        let Some(hold) = state.key_holds[index].take() else {
            return;
        };
        let still_held = Self::key_holds_button(state, hold.module, hold.widget)
            || Self::pointer_holds_button(state, hold.module, hold.widget);
        if !still_held {
            self.emit(
                shell,
                CanvasEvent::ButtonReleased {
                    module: hold.module,
                    widget: hold.widget,
                },
            );
            shell.request_redraw();
            shell.capture_event();
        }
    }

    /// Ends every keyboard hold. Used when the window loses focus, because
    /// the key releases never arrive.
    fn release_every_key_hold(&self, state: &mut State, shell: &mut Shell<'_, Message>) {
        let holds = std::mem::take(&mut state.key_holds);
        let mut reported = [(0_u32, 0_usize); 2];
        let mut count = 0;
        for hold in holds.into_iter().flatten() {
            if reported[..count].contains(&(hold.module, hold.widget))
                || Self::pointer_holds_button(state, hold.module, hold.widget)
            {
                continue;
            }
            reported[count] = (hold.module, hold.widget);
            count += 1;
            self.emit(
                shell,
                CanvasEvent::ButtonReleased {
                    module: hold.module,
                    widget: hold.widget,
                },
            );
        }
        if count > 0 {
            shell.request_redraw();
            shell.capture_event();
        }
    }

    /// Ends the hold for the key that came up.
    fn release_key(&self, state: &mut State, key: &keyboard::Key, shell: &mut Shell<'_, Message>) {
        if let Some(named) = Self::button_key(key) {
            self.release_button_key(state, named, shell);
        }
        if state.arrow_gesture
            && matches!(
                key.as_ref(),
                keyboard::Key::Named(
                    keyboard::key::Named::ArrowUp
                        | keyboard::key::Named::ArrowRight
                        | keyboard::key::Named::ArrowDown
                        | keyboard::key::Named::ArrowLeft
                )
            )
        {
            state.arrow_gesture = false;
            self.emit(shell, CanvasEvent::GestureEnd);
        }
    }

    /// Selects the module of a control being edited, so the inspector
    /// follows.
    fn select_for_edit(&self, module: u32, shell: &mut Shell<'_, Message>) {
        if self.selection.len() != 1 || !self.selection.contains(&module) {
            self.emit(shell, CanvasEvent::Select(vec![module]));
        }
    }

    fn set_control(&self, shell: &mut Shell<'_, Message>, module: u32, widget: usize, position: f32) {
        self.emit(
            shell,
            CanvasEvent::SetControl {
                module,
                widget,
                position,
            },
        );
    }

    /// Pointer down on a button. The tile stays put. Pointer up reports a
    /// release, whichever mode the app has stored.
    fn press_button(&self, state: &mut State, module: u32, widget: usize, shell: &mut Shell<'_, Message>) {
        self.emit(shell, CanvasEvent::ButtonPressed { module, widget });
        state.interaction = Interaction::Holding { module, widget };
    }

    /// The keyboard path to panel controls (NFR-UX-01). Returns true if the
    /// key was used.
    fn focus_keys(
        &self,
        state: &mut State,
        key: &keyboard::Key,
        modifiers: keyboard::Modifiers,
        repeat: bool,
        over: bool,
        shell: &mut Shell<'_, Message>,
    ) -> bool {
        use keyboard::key::Named;
        // Focus is only valid on a control of the one selected module.
        let selected = (self.selection.len() == 1).then(|| *self.selection.iter().next().unwrap_or(&0));
        if state.focus.is_some_and(|(m, w)| {
            Some(m) != selected
                || !self
                    .scene
                    .module(m)
                    .and_then(|module| module.panel.widgets.get(w))
                    .is_some_and(|widget| widget.is_control())
        }) {
            state.focus = None;
        }
        let used = match key.as_ref() {
            // Escape hides pinned lines before it does anything else
            // (modulation.md, Lines).
            keyboard::Key::Named(Named::Escape) if self.pin(state).is_some() => {
                state.pinned = None;
                true
            }
            keyboard::Key::Named(Named::Tab) if over || state.focus.is_some() => {
                let Some(module) = selected.and_then(|id| self.scene.module(id)) else {
                    return false;
                };
                let current = state.focus.map(|(_, w)| w);
                match next_focus(&module.panel, current, modifiers.shift()) {
                    Some(widget) => {
                        state.focus = Some((module.id, widget));
                        state.tab_focus = true;
                        state.focus_first = true;
                    }
                    None => return false,
                }
                true
            }
            keyboard::Key::Named(Named::Escape) if state.focus.is_some() => {
                state.focus = None;
                // In MIDI learn mode, Escape also ends the mode, so the app
                // must see it too.
                self.learn.is_none()
            }
            // In MIDI learn mode the keyboard arms the focused control, like
            // a click, and never changes a value (midi-learn.md).
            // With nothing focused, Space still toggles Run below: that is
            // the transport, not a parameter.
            keyboard::Key::Named(Named::Space | Named::Enter) if self.learn.is_some() && state.focus.is_some() => {
                if !repeat && let Some((module, widget)) = state.focus {
                    self.emit(shell, CanvasEvent::Learn { module, widget });
                }
                true
            }
            keyboard::Key::Named(Named::ArrowUp | Named::ArrowRight | Named::ArrowDown | Named::ArrowLeft)
                if self.learn.is_some() =>
            {
                false
            }
            keyboard::Key::Named(
                named @ (Named::ArrowUp | Named::ArrowRight | Named::ArrowDown | Named::ArrowLeft),
            ) => {
                let Some((module, widget)) = state.focus else {
                    return false;
                };
                let up = matches!(named, Named::ArrowUp | Named::ArrowRight);
                if matches!(self.widget_kind(module, widget), Some(WidgetViewKind::Button)) {
                    self.emit(
                        shell,
                        CanvasEvent::ButtonArrow {
                            module,
                            widget,
                            high: up,
                        },
                    );
                } else {
                    let position = self.position(module, widget);
                    let next = match self.widget_kind(module, widget) {
                        Some(WidgetViewKind::Knob { steps }) => knob_step(position, up, modifiers.shift(), *steps),
                        Some(WidgetViewKind::Toggle) => {
                            if up {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        Some(WidgetViewKind::Radio { options }) => {
                            let last = options.len().saturating_sub(1) as f32;
                            (position.round() + if up { 1.0 } else { -1.0 }).clamp(0.0, last)
                        }
                        _ => return false,
                    };
                    if !state.arrow_gesture {
                        state.arrow_gesture = true;
                        self.emit(shell, CanvasEvent::GestureStart);
                    }
                    if next != position {
                        self.set_control(shell, module, widget, next);
                    }
                }
                true
            }
            keyboard::Key::Named(named @ (Named::Space | Named::Enter)) => {
                if let Some((module, widget)) = state.focus {
                    let position = self.position(module, widget);
                    match self.widget_kind(module, widget) {
                        Some(WidgetViewKind::Toggle) => {
                            self.set_control(shell, module, widget, if position > 0.5 { 0.0 } else { 1.0 })
                        }
                        // A repeated key must not press again. The first press is
                        // still held, so key up can release it.
                        Some(WidgetViewKind::Button) if repeat => {}
                        Some(WidgetViewKind::Button) => {
                            self.press_button_key(state, named, module, widget, shell);
                        }
                        Some(WidgetViewKind::Radio { options }) => {
                            let next = (position.round() as usize + 1) % options.len().max(1);
                            self.set_control(shell, module, widget, next as f32);
                        }
                        _ => {}
                    }
                    true
                } else if matches!(named, Named::Space) {
                    // Space with nothing focused is the Run shortcut. A
                    // repeat is swallowed so holding the key does not toggle
                    // again. Enter is not that shortcut.
                    if !repeat {
                        self.emit(shell, CanvasEvent::ToggleRun);
                    }
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if used {
            shell.request_redraw();
            shell.capture_event();
        }
        used
    }

    fn press(&self, state: &mut State, world: Point, screen: Point, bounds: Rectangle, shell: &mut Shell<'_, Message>) {
        let shift = state.modifiers.shift();
        let hit = self.scene.hit(world, state.hit_radius());
        state.tab_focus = false;
        state.focus_first = false;
        // The ring of a modulated knob reaches a little past the knob's
        // cell, so it is found before the hit test's answer is used.
        if self.learn.is_none()
            && matches!(hit, Hit::Widget { .. } | Hit::Module(_))
            && let Some((module, widget)) = self.range_ring_at(world)
        {
            // The selection stays as it is: a selected source module is one
            // way the drag picks its assignment (modulation.md, Arc drag).
            self.begin(
                state,
                Interaction::Depth {
                    module,
                    widget,
                    last_y: screen.y,
                },
                shell,
            );
            return;
        }
        if !matches!(hit, Hit::Widget { .. }) {
            state.focus = None;
        }
        match hit {
            Hit::Widget { module, widget } if self.learn.is_some() && self.is_control(module, widget) => {
                self.select_for_edit(module, shell);
                // The clicked control takes the keyboard focus, so Space or
                // Enter acts on it next, not on a control focused earlier.
                state.focus = Some((module, widget));
                self.emit(shell, CanvasEvent::Learn { module, widget });
            }
            Hit::Badge(port) => {
                // A click pins this output's lines, or unpins them. Only one
                // output is pinned at a time.
                state.pinned = if state.pinned == Some(port) { None } else { Some(port) };
            }
            Hit::Widget { module, widget } => {
                self.select_for_edit(module, shell);
                state.focus = Some((module, widget));
                let now = Instant::now();
                let double = state
                    .last_click
                    .is_some_and(|(at, m, w)| m == module && w == widget && now - at < DOUBLE_CLICK);
                // A button's second press is another press. Other controls
                // reset to the default (panels.md, Interaction).
                let button = matches!(self.widget_kind(module, widget), Some(WidgetViewKind::Button));
                if double && !button {
                    state.last_click = None;
                    self.emit(shell, CanvasEvent::ResetControl { module, widget });
                    return;
                }
                state.last_click = Some((now, module, widget));
                if button {
                    self.press_button(state, module, widget, shell);
                    return;
                }
                let position = self.position(module, widget);
                let click = match self.widget_kind(module, widget) {
                    Some(WidgetViewKind::Knob { .. }) => {
                        self.begin(
                            state,
                            Interaction::Turning {
                                module,
                                widget,
                                last_y: screen.y,
                                position,
                            },
                            shell,
                        );
                        return;
                    }
                    Some(WidgetViewKind::Toggle) => Some(if position > 0.5 { 0.0 } else { 1.0 }),
                    Some(WidgetViewKind::Radio { options }) => self
                        .scene
                        .module(module)
                        .and_then(|m| m.widget_bounds(widget))
                        .and_then(|rect| radio_option(rect, options.len(), world))
                        .map(|option| option as f32),
                    _ => None,
                };
                // Toggles and radio buttons act on release, so a drag that
                // starts on them still moves the tile.
                self.begin(
                    state,
                    Interaction::Moving {
                        ids: vec![module],
                        start: world,
                        applied: Vector::ZERO,
                        click: click.map(|position| (module, widget, position)),
                    },
                    shell,
                );
            }
            Hit::Port(port) if !port.output => {
                // Grabbing a connected input picks up its cable, like pulling
                // a patch cable out of a jack.
                let source = self
                    .scene
                    .cables()
                    .iter()
                    .find(|c| c.to == (port.module, port.index))
                    .map(|c| PortRef {
                        module: c.from.0,
                        output: true,
                        index: c.from.1,
                    });
                if let Some(source) = source {
                    // Pulling a cable out and dropping it elsewhere is one
                    // step, so the gesture starts before the disconnect.
                    self.emit(shell, CanvasEvent::GestureStart);
                    self.emit(shell, CanvasEvent::Disconnect(port));
                    state.interaction = Interaction::Wiring {
                        from: source,
                        pulled: true,
                    };
                } else {
                    self.begin(
                        state,
                        Interaction::Wiring {
                            from: port,
                            pulled: false,
                        },
                        shell,
                    );
                }
            }
            Hit::Port(port) => {
                self.begin(
                    state,
                    Interaction::Wiring {
                        from: port,
                        pulled: false,
                    },
                    shell,
                );
            }
            Hit::Module(id) => {
                let mut selection = self.selection.clone();
                if shift {
                    if !selection.remove(&id) {
                        selection.insert(id);
                    }
                } else if !selection.contains(&id) {
                    selection = BTreeSet::from([id]);
                }
                if &selection != self.selection {
                    self.emit(shell, CanvasEvent::Select(selection.iter().copied().collect()));
                }
                if selection.contains(&id) {
                    self.begin(
                        state,
                        Interaction::Moving {
                            ids: selection.into_iter().collect(),
                            start: world,
                            applied: Vector::ZERO,
                            click: None,
                        },
                        shell,
                    );
                }
            }
            Hit::Nothing if double_click(state.last_empty_click.take(), screen) => {
                self.open_browser(world, screen, bounds, shell);
            }
            Hit::Cable(_) | Hit::Nothing => {
                if matches!(hit, Hit::Nothing) {
                    state.last_empty_click = Some((Instant::now(), screen));
                }
                let base = if shift { self.selection.clone() } else { BTreeSet::new() };
                state.interaction = Interaction::Selecting {
                    start: world,
                    current: world,
                    base,
                };
            }
        }
    }

    /// Sends [`CanvasEvent::OpenBrowser`] for a point given in world and
    /// canvas-relative screen coordinates.
    fn open_browser(&self, world: Point, screen: Point, bounds: Rectangle, shell: &mut Shell<'_, Message>) {
        self.emit(
            shell,
            CanvasEvent::OpenBrowser {
                world,
                at: Point::new(bounds.x + screen.x, bounds.y + screen.y),
                area: bounds,
            },
        );
    }

    /// The module tile under the cursor, if any (not counting its ports).
    fn hovered_module(state: &State) -> Option<u32> {
        match state.hover? {
            Hit::Module(id) => Some(id),
            _ => None,
        }
    }

    /// The port under the cursor, if any.
    fn hovered_port(state: &State) -> Option<PortRef> {
        match state.hover? {
            Hit::Port(port) => Some(port),
            _ => None,
        }
    }

    /// The cable under the cursor, if any.
    fn hovered_cable(state: &State) -> Option<usize> {
        match state.hover? {
            Hit::Cable(index) => Some(index),
            _ => None,
        }
    }

    /// The overlay for a view whose visible world area is `visible`. Learn
    /// mode outlines only the controls there, so its cost follows what is on
    /// screen, not the size of the patch.
    fn overlay_in(&self, state: &State, visible: Rectangle) -> Overlay {
        let mut overlay = self.overlay(state);
        let p = &self.palette;
        // MIDI learn mode outlines every control on screen, and the armed
        // one more strongly, so the user sees what a click will do.
        if let Some(learn) = self.learn {
            let visible_modules = self
                .scene
                .modules()
                .iter()
                .filter(|module| module.bounds().intersects(&visible));
            for module in visible_modules {
                for (index, view) in module.panel.widgets.iter().enumerate() {
                    if !view.is_control() {
                        continue;
                    }
                    let Some(rect) = module.widget_bounds(index) else {
                        continue;
                    };
                    let (ring, radius) = focus_ring(&view.kind, rect);
                    let (colour, width) = if learn.armed == Some((module.id, index)) {
                        (p.selection, 2.5)
                    } else {
                        (p.focus, 1.0)
                    };
                    overlay
                        .shapes
                        .push(Shape::new(ring, radius, Color::TRANSPARENT, colour, width));
                }
            }
        }
        overlay
    }

    fn overlay(&self, state: &State) -> Overlay {
        let mut overlay = Overlay::default();
        let p = &self.palette;

        // Cables in focus come to the front at full opacity: the cable under
        // the cursor, the cables on the port under the cursor, and the cables
        // of the module under the cursor and of selected modules. Everything
        // else stays at the same dim level, so the patch never flickers.
        let hovered = Self::hovered_module(state);
        let hovered_port = Self::hovered_port(state);
        let hovered_cable = Self::hovered_cable(state);
        let module_in_focus = |id: u32| self.selection.contains(&id) || hovered == Some(id);
        let on_port = |cable: &crate::scene::CableView| {
            hovered_port.is_some_and(|p| {
                if p.output {
                    cable.from == (p.module, p.index)
                } else {
                    cable.to == (p.module, p.index)
                }
            })
        };
        if !self.selection.is_empty() || hovered.is_some() || hovered_port.is_some() || hovered_cable.is_some() {
            for (index, cable) in self.scene.cables().iter().enumerate() {
                let focused = hovered_cable == Some(index)
                    || on_port(cable)
                    || module_in_focus(cable.from.0)
                    || module_in_focus(cable.to.0);
                if !focused {
                    continue;
                }
                let from = PortRef {
                    module: cable.from.0,
                    output: true,
                    index: cable.from.1,
                };
                let to = PortRef {
                    module: cable.to.0,
                    output: false,
                    index: cable.to.1,
                };
                if let (Some(a), Some(b)) = (self.scene.port_position(from), self.scene.port_position(to)) {
                    overlay
                        .cables
                        .push(Cable::new(a, b, cable.colour_from, cable.colour_to, cable.delayed).focused());
                }
            }
        }

        self.modulation_lines(state, &mut overlay);

        for id in self.selection {
            if let Some(module) = self.scene.module(*id) {
                let b = module.bounds().expand(3.0);
                overlay
                    .shapes
                    .push(Shape::new(b, 7.0, Color::TRANSPARENT, p.selection, 1.5));
            }
        }
        if let Some(Hit::Port(port)) = state.hover
            && let Some(centre) = self.scene.port_position(port)
        {
            let r = PORT_RADIUS + 3.0;
            let rect = Rectangle::new(Point::new(centre.x - r, centre.y - r), Size::new(2.0 * r, 2.0 * r));
            overlay
                .shapes
                .push(Shape::new(rect, r, Color::TRANSPARENT, p.hover, 1.2));
        }
        // The focus ring shows for keyboard focus only. After a click or a
        // drag the pointer already shows where the user is, and the ring
        // would only crowd the value text (#92).
        if let Some((module, widget)) = state.focus
            && state.tab_focus
            && self.selection.contains(&module)
            && let Some(m) = self.scene.module(module)
            && let (Some(rect), Some(view)) = (m.widget_bounds(widget), m.panel.widgets.get(widget))
        {
            let (ring, radius) = focus_ring(&view.kind, rect);
            overlay
                .shapes
                .push(Shape::new(ring, radius, Color::TRANSPARENT, p.focus, 1.5));
        }
        match &state.interaction {
            Interaction::Wiring { from, pulled } => {
                // A control that would take this output as an assignment is
                // highlighted, like an input port for a cable.
                if let Some(Hit::Widget { module, widget }) = state.hover
                    && !pulled
                    && self.takes_assignment(*from, module, widget)
                    && let Some(m) = self.scene.module(module)
                    && let (Some(rect), Some(view)) = (m.widget_bounds(widget), m.panel.widgets.get(widget))
                {
                    let (ring, radius) = focus_ring(&view.kind, rect);
                    overlay
                        .shapes
                        .push(Shape::new(ring, radius, Color::TRANSPARENT, p.hover, 1.5));
                }
                if let (Some(start), Some(port), Some(cursor)) =
                    (self.scene.port_position(*from), self.scene.port(*from), state.cursor)
                {
                    // Snap the loose end onto a port under the cursor.
                    let end = match state.hover {
                        Some(Hit::Port(target)) if target.output != from.output => {
                            self.scene.port_position(target).unwrap_or(cursor)
                        }
                        _ => cursor,
                    };
                    let cable = if from.output {
                        Cable::new(start, end, port.colour, port.colour, false)
                    } else {
                        Cable::new(end, start, port.colour, port.colour, false)
                    }
                    .focused();
                    overlay.cables.push(cable);
                }
            }
            Interaction::Selecting { start, current, .. } => {
                let rect = Rectangle::new(
                    Point::new(start.x.min(current.x), start.y.min(current.y)),
                    Size::new((start.x - current.x).abs(), (start.y - current.y).abs()),
                );
                overlay
                    .shapes
                    .push(Shape::new(rect, 2.0, p.marquee, p.selection, 1.0 / state.camera.zoom));
            }
            _ => {}
        }
        overlay
    }

    /// Modulation lines in focus (modulation.md, Lines): thin dashed lines
    /// from a source port to its targets, for the pinned output, the output
    /// or badge under the pointer, the control under the pointer, and the
    /// modules in focus as for cables. The tiles that a pinned or pointed
    /// at source drives are outlined. A pinned badge gets a ring.
    fn modulation_lines(&self, state: &State, overlay: &mut Overlay) {
        let key = LinesKey {
            scene: self.scene.generation(),
            pinned: self.pin(state),
            hover: state.hover,
            selection: self.selection.iter().copied().collect(),
        };
        let mut cache = state.lines.borrow_mut();
        if cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let mut lines = Overlay::default();
            self.build_modulation_lines(state, &mut lines);
            *cache = Some((key, lines));
        }
        if let Some((_, lines)) = cache.as_ref() {
            overlay.cables.extend_from_slice(&lines.cables);
            overlay.shapes.extend_from_slice(&lines.shapes);
        }
    }

    fn build_modulation_lines(&self, state: &State, overlay: &mut Overlay) {
        let p = &self.palette;
        if let Some(port) = self.pin(state)
            && let Some(badge) = self.scene.module(port.module).and_then(|m| m.badge_bounds(port.index))
        {
            let ring = badge.expand(1.5);
            overlay
                .shapes
                .push(Shape::new(ring, ring.height * 0.5, Color::TRANSPARENT, p.text, 1.0));
        }
        let pointed = match state.hover {
            Some(Hit::Port(port) | Hit::Badge(port)) if port.output => Some(port),
            _ => None,
        };
        let sources: Vec<(u32, usize)> = [self.pin(state), pointed]
            .into_iter()
            .flatten()
            .map(|port| (port.module, port.index))
            .collect();
        let control = match state.hover {
            Some(Hit::Widget { module, widget }) => Some((module, widget)),
            _ => None,
        };
        let hovered = Self::hovered_module(state);
        let module_in_focus = |id: u32| self.selection.contains(&id) || hovered == Some(id);
        let mut outlined: Vec<u32> = Vec::new();
        for m in self.scene.modulations() {
            let by_source = sources.contains(&m.from);
            let by_control = control.is_some_and(|(module, widget)| m.to == module && m.widget == Some(widget));
            if !(by_source || by_control || module_in_focus(m.from.0) || module_in_focus(m.to)) {
                continue;
            }
            let from = PortRef {
                module: m.from.0,
                output: true,
                index: m.from.1,
            };
            let (Some(start), Some(end)) = (self.scene.port_position(from), self.scene.modulation_end(m)) else {
                continue;
            };
            overlay.cables.push(
                Cable::new(start, end, p.modulation, p.modulation, true)
                    .thin()
                    .focused(),
            );
            if (by_source || by_control) && !outlined.contains(&m.to) {
                outlined.push(m.to);
            }
        }
        for id in outlined {
            if let Some(module) = self.scene.module(id) {
                overlay.shapes.push(Shape::new(
                    module.bounds().expand(3.0),
                    7.0,
                    Color::TRANSPARENT,
                    p.modulation,
                    1.2,
                ));
            }
        }
    }

    fn draw_text<Renderer>(&self, state: &State, values: &PanelValues, renderer: &mut Renderer, bounds: Rectangle)
    where
        Renderer: text::Renderer<Font = Font>,
    {
        let zoom = state.camera.zoom;
        // Quantise sizes so zooming does not create a new text layout every
        // frame for every label.
        let quantise = |size: f32| (size * 2.0).round() / 2.0;
        let title_px = quantise(TITLE_SIZE * zoom);
        if title_px < TITLE_MIN_PX {
            return;
        }
        let label_px = quantise(LABEL_SIZE * zoom);
        let show_labels = label_px >= LABEL_MIN_PX;
        let visible = state.camera.visible(bounds.size());
        let font = renderer.default_font();
        let to_screen = |world: Point| {
            let s = state.camera.to_screen(world);
            Point::new(bounds.x + s.x, bounds.y + s.y)
        };
        let text = |content: &str, size: f32, width: f32, align: text::Alignment| Text {
            content: content.to_string(),
            bounds: Size::new(width, size * 1.4),
            size: Pixels(size),
            line_height: text::LineHeight::Relative(1.2),
            font,
            align_x: align,
            align_y: alignment::Vertical::Center,
            shaping: text::Shaping::Basic,
            wrapping: text::Wrapping::None,
        };
        // The control whose value shows instead of its label.
        let active = match state.interaction {
            Interaction::Turning { module, widget, .. } => Some((module, widget)),
            _ => state.focus,
        };

        for module in self.scene.modules() {
            let tile = module.bounds();
            if !tile.intersects(&visible) {
                continue;
            }
            let screen_tile = Rectangle::new(to_screen(tile.position()), tile.size() * zoom);
            let Some(clip) = screen_tile.intersection(&bounds) else {
                continue;
            };
            tile_text::for_each(module, |item| {
                let (size, colour) = match item.kind {
                    TextKind::Title => (title_px, self.palette.text),
                    _ if !show_labels => return,
                    TextKind::Option(..) => (label_px, self.palette.text),
                    _ => (label_px, self.palette.text_dim),
                };
                // A focused or turning knob shows its value instead of its
                // caption.
                // In MIDI learn mode, a mapped control shows its mapping.
                let learned = match item.kind {
                    TextKind::Caption(widget) => self
                        .learn
                        .and_then(|learn| learn.labels.get(&(module.id, widget)))
                        .map(String::as_str),
                    _ => None,
                };
                let value = match item.kind {
                    _ if learned.is_some() => learned,
                    TextKind::Caption(widget) if active == Some((module.id, widget)) => values
                        .get(module.id, widget)
                        .filter(|_| matches!(module.panel.widgets[widget].kind, WidgetViewKind::Knob { .. }))
                        .map(|v| v.text.as_str()),
                    _ => None,
                };
                let align = match item.align {
                    Align::Left => text::Alignment::Left,
                    Align::Center => text::Alignment::Center,
                    Align::Right => text::Alignment::Right,
                };
                renderer.fill_text(
                    text(value.unwrap_or(&item.content), size, item.width * zoom, align),
                    to_screen(item.anchor),
                    if value.is_some() { self.palette.text } else { colour },
                    clip,
                );
            });
        }

        // Badge numbers sit outside their tile, so they are checked against
        // their own bounds, not the tile's, and clip to the canvas. Too small
        // to read, the pill alone shows.
        let badge_px = quantise(BADGE_TEXT_SIZE * zoom);
        if badge_px < LABEL_MIN_PX * 0.8 {
            return;
        }
        for module in self.scene.modules() {
            // A badge reaches past its tile's right edge by at most this
            // much: the gap after the port and a pill with several digits.
            const BADGE_REACH: f32 = 40.0;
            if !module.bounds().expand(BADGE_REACH).intersects(&visible) {
                continue;
            }
            for (index, port) in module.outputs.iter().enumerate() {
                if let Some(rect) = module.badge_bounds(index).filter(|r| r.intersects(&visible)) {
                    let label = port.badge.to_string();
                    renderer.fill_text(
                        text(&label, badge_px, rect.width * zoom, text::Alignment::Center),
                        to_screen(rect.center()),
                        self.palette.background,
                        bounds,
                    );
                }
            }
        }
    }

    /// The label, hover text, and world bounds of what hover text is about.
    fn hover_content(&self, target: HoverTarget) -> Option<(&str, &HoverText, Rectangle)> {
        match target {
            HoverTarget::Port(port) => {
                let view = self.scene.port(port)?;
                let at = self.scene.port_position(port)?;
                let anchor = Rectangle::new(
                    Point::new(at.x - PORT_RADIUS, at.y - PORT_RADIUS),
                    Size::new(2.0 * PORT_RADIUS, 2.0 * PORT_RADIUS),
                );
                Some((&view.label, &view.hover, anchor))
            }
            HoverTarget::Control(module, widget) => {
                let module = self.scene.module(module)?;
                let view = module.panel.widgets.get(widget)?;
                Some((&view.label, &view.hover, module.widget_bounds(widget)?))
            }
        }
    }

    /// Hover text (panels.md, Labels and hover text): the label, the
    /// description, and the name when it differs, at a fixed screen size in
    /// a layer above the tiles. Drawn at any zoom, because hover text is
    /// what the user needs where tile text is too small to read.
    fn draw_hover_text<Renderer>(&self, state: &State, renderer: &mut Renderer, bounds: Rectangle)
    where
        Renderer: text::Renderer<Font = Font>,
    {
        let Some((label, hover, world)) = state.hover_text.shown().and_then(|t| self.hover_content(t)) else {
            return;
        };
        let font = renderer.default_font();
        let inner = crate::hover::text_width(bounds.width);
        // Each line is measured as a paragraph, then drawn as text in that
        // size, wrapped at word boundaries.
        let line = |content: &str, size: f32, colour: Color| {
            let text = Text {
                content: content.to_string(),
                bounds: Size::new(inner, f32::INFINITY),
                size: Pixels(size),
                line_height: text::LineHeight::Relative(1.3),
                font,
                align_x: text::Alignment::Left,
                align_y: alignment::Vertical::Top,
                shaping: text::Shaping::Basic,
                wrapping: text::Wrapping::Word,
            };
            let measured = Renderer::Paragraph::with_text(text.as_ref()).min_bounds();
            (
                Text {
                    bounds: Size::new(measured.width.ceil() + 1.0, measured.height.ceil()),
                    ..text
                },
                colour,
            )
        };
        let mut lines = vec![line(label, HOVER_TEXT_PX, self.palette.text)];
        if !hover.description.is_empty() {
            lines.push(line(&hover.description, HOVER_TEXT_PX, self.palette.text_dim));
        }
        if let Some(name) = &hover.name {
            lines.push(line(name, HOVER_TEXT_PX - 1.0, self.palette.text_dim));
        }
        const LINE_GAP: f32 = 3.0;
        let width = lines.iter().map(|(t, _)| t.bounds.width).fold(0.0, f32::max);
        let height = lines.iter().map(|(t, _)| t.bounds.height).sum::<f32>() + LINE_GAP * (lines.len() - 1) as f32;
        let size = Size::new(width + 2.0 * HOVER_PADDING, height + 2.0 * HOVER_PADDING);

        let zoom = state.camera.zoom;
        let corner = state.camera.to_screen(world.position());
        let anchor = Rectangle::new(
            Point::new(bounds.x + corner.x, bounds.y + corner.y),
            world.size() * zoom,
        );
        let at = crate::hover::place(size, anchor, bounds);
        renderer.with_layer(bounds, |renderer| {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle::new(at, size),
                    border: iced::Border {
                        color: self.palette.tile_border,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..renderer::Quad::default()
                },
                self.palette.tile,
            );
            let mut y = at.y + HOVER_PADDING;
            for (text, colour) in lines {
                let height = text.bounds.height;
                renderer.fill_text(text, Point::new(at.x + HOVER_PADDING, y), colour, bounds);
                y += height + LINE_GAP;
            }
        });
    }
}

/// How much a zoom level shows (panels.md, Level of detail).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Detail {
    /// Port labels and panel text.
    Close,
    /// Titles and control shapes, no other text.
    Middle,
    /// Plain tiles: no text, no controls.
    Far,
}

fn detail(zoom: f32) -> Detail {
    let quantise = |size: f32| (size * 2.0).round() / 2.0;
    if quantise(LABEL_SIZE * zoom) >= LABEL_MIN_PX {
        Detail::Close
    } else if quantise(TITLE_SIZE * zoom) >= TITLE_MIN_PX {
        Detail::Middle
    } else {
        Detail::Far
    }
}

/// Modules with a light or meter on screen, at a zoom where they are drawn.
fn visible_lights(scene: &Scene, camera: &Camera, size: Size) -> Vec<u32> {
    if detail(camera.zoom) == Detail::Far {
        return Vec::new();
    }
    let visible = camera.visible(size);
    scene
        .modules()
        .iter()
        .filter(|m| m.panel.widgets.iter().any(|w| !w.is_control()) && m.bounds().intersects(&visible))
        .map(|m| m.id)
        .collect()
}

/// The control after (or with `back`, before) `current` in focus order,
/// wrapping around. Lights are skipped. `None` if the panel has no
/// controls.
fn next_focus(panel: &crate::scene::PanelView, current: Option<usize>, back: bool) -> Option<usize> {
    let controls: Vec<usize> = (0..panel.widgets.len())
        .filter(|&i| panel.widgets[i].is_control())
        .collect();
    if controls.is_empty() {
        return None;
    }
    let at = current.and_then(|c| controls.iter().position(|&i| i == c));
    let next = match (at, back) {
        (None, false) => 0,
        (None, true) => controls.len() - 1,
        (Some(i), false) => (i + 1) % controls.len(),
        (Some(i), true) => (i + controls.len() - 1) % controls.len(),
    };
    Some(controls[next])
}

/// A slow figure-eight pan with a zoom swing, around the starting view.
fn animated(base: Camera, size: Size, t: f32) -> Camera {
    let centre = base.to_world(Point::new(size.width * 0.5, size.height * 0.5));
    let zoom = (base.zoom * (1.0 + 0.4 * (t * 0.45).sin())).clamp(crate::camera::MIN_ZOOM, crate::camera::MAX_ZOOM);
    let reach = 0.25 * size.width / base.zoom;
    let target = Point::new(
        centre.x + reach * (t * 0.31).sin(),
        centre.y + 0.5 * reach * (t * 0.62).sin(),
    );
    Camera {
        origin: Point::new(target.x - size.width * 0.5 / zoom, target.y - size.height * 0.5 / zoom),
        zoom,
    }
}

impl<'a, Message, Theme, Renderer> From<PatchCanvas<'a, Message>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_wgpu::primitive::Renderer + text::Renderer<Font = Font> + 'a,
    Theme: 'a,
{
    fn from(canvas: PatchCanvas<'a, Message>) -> Self {
        Element::new(canvas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{CableView, ModuleView, PanelView, PortShape, PortView, WidgetView};

    fn module(id: u32, x: f32) -> ModuleView {
        let port = || PortView {
            label: "p".into(),
            hover: crate::scene::HoverText::default(),
            colour: Color::WHITE,
            shape: PortShape::Round,
            badge: 0,
        };
        ModuleView {
            id,
            title: String::new(),
            accent: Color::BLACK,
            position: Point::new(x, 0.0),
            inputs: vec![port()],
            outputs: vec![port()],
            panel: PanelView::default(),
        }
    }

    fn widget(kind: WidgetViewKind, x: u8) -> WidgetView {
        WidgetView {
            kind,
            x,
            y: 0,
            label: String::new(),
            hover: crate::scene::HoverText::default(),
        }
    }

    /// A knob, a light, and a toggle.
    fn panel() -> PanelView {
        PanelView {
            width: 9,
            height: 3,
            widgets: vec![
                widget(WidgetViewKind::Knob { steps: 0 }, 0),
                widget(WidgetViewKind::Light, 3),
                widget(WidgetViewKind::Toggle, 6),
            ],
        }
    }

    #[test]
    fn tab_moves_focus_over_controls_and_skips_lights() {
        let panel = panel();
        assert_eq!(next_focus(&panel, None, false), Some(0));
        assert_eq!(next_focus(&panel, Some(0), false), Some(2));
        assert_eq!(next_focus(&panel, Some(2), false), Some(0));
        assert_eq!(next_focus(&panel, Some(0), true), Some(2));
        assert_eq!(next_focus(&panel, None, true), Some(2));
        assert_eq!(next_focus(&PanelView::default(), None, false), None);
    }

    #[test]
    fn detail_follows_the_text_thresholds() {
        assert_eq!(detail(1.0), Detail::Close);
        // Port labels hidden, titles shown.
        assert_eq!(detail(0.7), Detail::Middle);
        assert_eq!(detail(0.3), Detail::Far);
    }

    #[test]
    fn only_lights_on_screen_are_reported() {
        let mut lit = module(1, 0.0);
        lit.panel = panel();
        let far_away = ModuleView {
            panel: panel(),
            ..module(2, 5_000.0)
        };
        let plain = module(3, 200.0);
        let scene = Scene::new(vec![lit, far_away, plain], Vec::new());
        let camera = Camera {
            origin: Point::new(-50.0, -50.0),
            zoom: 1.0,
        };
        let size = Size::new(800.0, 600.0);
        assert_eq!(visible_lights(&scene, &camera, size), vec![1]);
        // Zoomed far out, lights are not drawn, so none need reading.
        let far = Camera { zoom: 0.3, ..camera };
        assert!(visible_lights(&scene, &far, size).is_empty());
    }

    #[test]
    fn a_control_focused_by_keyboard_has_a_ring_clear_of_its_caption() {
        let mut m = module(1, 0.0);
        m.panel = panel();
        let scene = Scene::new(vec![m], Vec::new());
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let unfocused = canvas.overlay(&State::default()).shapes.len();
        let tabbed = State {
            focus: Some((1, 0)),
            tab_focus: true,
            ..State::default()
        };
        let overlay = canvas.overlay(&tabbed);
        assert_eq!(overlay.shapes.len(), unfocused + 1);
        // The ring stays above the caption, where the value shows.
        let ring = overlay.shapes.last().unwrap().rect();
        let module = &scene.modules()[0];
        let mut caption = None;
        tile_text::for_each(module, |item| {
            if item.kind == TextKind::Caption(0) {
                caption = Some(item.bounds());
            }
        });
        let caption = caption.unwrap();
        assert!(
            ring[1] + ring[3] <= caption.y,
            "ring {ring:?} clear of caption {caption:?}"
        );
        // Focus from a click or a drag shows no ring (#92).
        let clicked = State {
            focus: Some((1, 0)),
            ..State::default()
        };
        assert_eq!(canvas.overlay(&clicked).shapes.len(), unfocused);
    }

    #[test]
    fn hover_text_is_about_the_port_or_control_under_the_pointer() {
        let mut m = module(1, 0.0);
        m.panel = panel();
        let scene = Scene::new(vec![m], Vec::new());
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        let port = PortRef {
            module: 1,
            output: true,
            index: 0,
        };
        let on = |hover| State {
            hover: Some(hover),
            ..State::default()
        };
        assert_eq!(canvas.hover_target(&on(Hit::Port(port))), Some(HoverTarget::Port(port)));
        let knob = Hit::Widget { module: 1, widget: 0 };
        assert_eq!(canvas.hover_target(&on(knob)), Some(HoverTarget::Control(1, 0)));
        assert_eq!(canvas.hover_target(&on(Hit::Module(1))), None);
        assert_eq!(canvas.hover_target(&on(Hit::Nothing)), None);
        // On a port's label: the port's hover text (#92).
        let mut label = None;
        tile_text::for_each(&scene.modules()[0], |item| {
            if item.kind == TextKind::Port(port) {
                label = Some(item.bounds().center());
            }
        });
        let on_label = State {
            cursor: label,
            ..on(Hit::Module(1))
        };
        assert_eq!(canvas.hover_target(&on_label), Some(HoverTarget::Port(port)));
    }

    #[test]
    fn no_hover_text_during_a_drag() {
        let mut m = module(1, 0.0);
        m.panel = panel();
        let scene = Scene::new(vec![m], Vec::new());
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        let port = PortRef {
            module: 1,
            output: true,
            index: 0,
        };
        let drags = [
            Interaction::Wiring {
                from: port,
                pulled: false,
            },
            Interaction::Turning {
                module: 1,
                widget: 0,
                last_y: 0.0,
                position: 0.5,
            },
            Interaction::Moving {
                ids: vec![1],
                start: Point::ORIGIN,
                applied: Vector::ZERO,
                click: None,
            },
            Interaction::Panning { last: Point::ORIGIN },
        ];
        for interaction in drags {
            let state = State {
                hover: Some(Hit::Port(port)),
                interaction: interaction.clone(),
                ..State::default()
            };
            assert_eq!(canvas.hover_target(&state), None, "{interaction:?}");
        }
    }

    #[test]
    fn tab_focus_shows_hover_text_but_a_click_does_not() {
        let mut m = module(1, 0.0);
        m.panel = panel();
        let scene = Scene::new(vec![m], Vec::new());
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let focused = |tab_focus| State {
            focus: Some((1, 2)),
            tab_focus,
            ..State::default()
        };
        assert_eq!(canvas.hover_target(&focused(true)), Some(HoverTarget::Control(1, 2)));
        assert_eq!(canvas.hover_target(&focused(false)), None);
        // With the pointer resting on the knob, a Tab press to the toggle
        // shows the toggle's text until the pointer moves.
        let knob = Hit::Widget { module: 1, widget: 0 };
        let tabbed = State {
            hover: Some(knob),
            focus_first: true,
            ..focused(true)
        };
        assert_eq!(canvas.hover_target(&tabbed), Some(HoverTarget::Control(1, 2)));
        let moved = State {
            hover: Some(knob),
            focus_first: false,
            ..focused(true)
        };
        assert_eq!(canvas.hover_target(&moved), Some(HoverTarget::Control(1, 0)));
        // Focus on a module that is no longer selected shows nothing.
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        assert_eq!(canvas.hover_target(&focused(true)), None);
    }

    fn cable(from: u32, to: u32) -> CableView {
        CableView {
            from: (from, 0),
            to: (to, 0),
            colour_from: Color::WHITE,
            colour_to: Color::WHITE,
            delayed: false,
        }
    }

    /// Modules 1 -> 2 -> 3, so selecting 1 focuses one cable and selecting
    /// 2 focuses both.
    fn chain() -> Scene {
        Scene::new(
            vec![module(1, 0.0), module(2, 200.0), module(3, 400.0)],
            vec![cable(1, 2), cable(2, 3)],
        )
    }

    #[test]
    fn selected_modules_bring_their_cables_forward() {
        let scene = chain();
        let state = State::default();

        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        assert!(canvas.overlay(&state).cables.is_empty());

        let first = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &first, |e| e);
        assert_eq!(canvas.overlay(&state).cables.len(), 1);

        let middle = BTreeSet::from([2]);
        let canvas = PatchCanvas::new(&scene, &middle, |e| e);
        assert_eq!(canvas.overlay(&state).cables.len(), 2);
    }

    #[test]
    fn tiles_stay_draggable_when_zoomed_out() {
        // At the smallest zoom, the port hit radius must not cover the whole
        // tile, or pressing anywhere on it would start a cable.
        let scene = chain();
        let mut state = State::default();
        state.camera.zoom = crate::camera::MIN_ZOOM;
        let centre = scene.module(2).unwrap().bounds().center();
        assert_eq!(scene.hit(centre, state.hit_radius()), Hit::Module(2));
    }

    #[test]
    fn hovering_a_module_brings_its_cables_forward() {
        let scene = chain();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        let state = State {
            hover: Some(Hit::Module(3)),
            ..State::default()
        };
        assert_eq!(canvas.overlay(&state).cables.len(), 1);
    }

    #[test]
    fn hovering_a_port_brings_only_its_cables_forward() {
        let scene = chain();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        // Module 2's input carries cable 1 -> 2; its output carries 2 -> 3.
        let state = State {
            hover: Some(Hit::Port(PortRef {
                module: 2,
                output: false,
                index: 0,
            })),
            ..State::default()
        };
        assert_eq!(canvas.overlay(&state).cables.len(), 1);
        // A port with no cable highlights nothing.
        let state = State {
            hover: Some(Hit::Port(PortRef {
                module: 1,
                output: false,
                index: 0,
            })),
            ..State::default()
        };
        assert!(canvas.overlay(&state).cables.is_empty());
    }

    #[test]
    fn hovering_a_cable_brings_only_that_cable_forward() {
        let scene = chain();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        let state = State {
            hover: Some(Hit::Cable(1)),
            ..State::default()
        };
        assert_eq!(canvas.overlay(&state).cables.len(), 1);
    }

    fn button_module() -> ModuleView {
        let mut module = module(1, 0.0);
        module.panel = PanelView {
            width: 6,
            height: 3,
            widgets: vec![widget(WidgetViewKind::Button, 0)],
        };
        module
    }

    fn drive(canvas: &PatchCanvas<'_, CanvasEvent>, state: &mut State, event: &Event, at: Point) -> Vec<CanvasEvent> {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        canvas.handle(state, event, bounds, mouse::Cursor::Available(at), &mut shell);
        messages
    }

    fn click(button: mouse::Button) -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(button))
    }

    /// A drag in progress when a pop-up opens ends at the next redraw,
    /// which reaches the canvas even when the pop-up takes every pointer
    /// and key event. A button held by the pointer is released too.
    #[test]
    fn a_redraw_ends_gestures_and_holds_when_the_canvas_is_not_interactive() {
        let redraw = Event::Window(window::Event::RedrawRequested(Instant::now()));

        let scene = chain();
        let selected = BTreeSet::from([1]);
        let tile = scene.module(1).unwrap().bounds().center();
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State::default();
        let pressed = drive(&canvas, &mut state, &click(mouse::Button::Left), tile);
        assert_eq!(pressed, vec![CanvasEvent::GestureStart]);
        let blocked = PatchCanvas::new(&scene, &selected, |e| e).interactive(false);
        let events = drive(&blocked, &mut state, &redraw, tile);
        assert!(events.contains(&CanvasEvent::GestureEnd), "{events:?}");
        assert!(matches!(state.interaction, Interaction::Idle));
        // Nothing is held now, so later redraws send nothing more.
        let again = drive(&blocked, &mut state, &redraw, tile);
        assert!(!again.contains(&CanvasEvent::GestureEnd), "{again:?}");

        let scene = Scene::new(vec![button_module()], Vec::new());
        let centre = scene.module(1).unwrap().widget_bounds(0).unwrap().center();
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State::default();
        drive(&canvas, &mut state, &click(mouse::Button::Left), centre);
        let blocked = PatchCanvas::new(&scene, &selected, |e| e).interactive(false);
        let events = drive(&blocked, &mut state, &redraw, centre);
        assert!(
            events.contains(&CanvasEvent::ButtonReleased { module: 1, widget: 0 }),
            "{events:?}"
        );
    }

    fn key_a(modifiers: keyboard::Modifiers) -> Event {
        let key = keyboard::Key::Character("a".into());
        Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyA),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    /// `A` alone opens the browser at the pointer. With any modifier it does
    /// not (module-browser.md, Rules).
    #[test]
    fn only_a_with_no_modifier_opens_the_browser() {
        let scene = chain();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        let at = Point::new(100.0, 300.0);
        let mut state = State::default();
        let events = drive(&canvas, &mut state, &key_a(keyboard::Modifiers::default()), at);
        assert!(
            matches!(events.as_slice(), [CanvasEvent::OpenBrowser { at: p, .. }] if *p == at),
            "{events:?}"
        );
        for modifiers in [
            keyboard::Modifiers::SHIFT,
            keyboard::Modifiers::CTRL,
            keyboard::Modifiers::ALT,
            keyboard::Modifiers::LOGO,
        ] {
            let events = drive(&canvas, &mut state, &key_a(modifiers), at);
            assert!(
                !events.iter().any(|e| matches!(e, CanvasEvent::OpenBrowser { .. })),
                "{modifiers:?}: {events:?}"
            );
        }
    }

    /// Ctrl+V or Cmd+V pastes at the pointer over the canvas, and at the
    /// centre of the view with the pointer elsewhere (clipboard.md).
    #[test]
    fn paste_goes_to_the_pointer_or_the_centre_of_the_view() {
        let scene = chain();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        let mut state = State::default();
        let key = keyboard::Key::Character("v".into());
        let paste = Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyV),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::COMMAND,
            text: None,
            repeat: false,
        });
        let at = Point::new(100.0, 300.0);
        let events = drive(&canvas, &mut state, &paste, at);
        assert_eq!(
            events,
            vec![CanvasEvent::Paste {
                world: state.camera.to_world(at)
            }]
        );
        let events = drive(&canvas, &mut state, &paste, Point::new(900.0, 700.0));
        assert_eq!(
            events,
            vec![CanvasEvent::Paste {
                world: state.camera.to_world(Point::new(400.0, 300.0))
            }]
        );
        // While a pop-up has the keys, paste is the pop-up's.
        let blocked = PatchCanvas::new(&scene, &none, |e| e).interactive(false);
        assert!(drive(&blocked, &mut state, &paste, at).is_empty());
    }

    /// Shift pressed while a pop-up has the keys still extends the
    /// selection on the first click after it closes.
    #[test]
    fn modifiers_stay_current_while_the_canvas_is_not_interactive() {
        let scene = chain();
        let selected = BTreeSet::from([1]);
        let tile = scene.module(2).unwrap().bounds().center();
        let shift = Event::Keyboard(keyboard::Event::ModifiersChanged(keyboard::Modifiers::SHIFT));
        let mut state = State::default();

        let blocked = PatchCanvas::new(&scene, &selected, |e| e).interactive(false);
        assert!(drive(&blocked, &mut state, &shift, tile).is_empty());
        assert!(state.modifiers.shift());

        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let events = drive(&canvas, &mut state, &click(mouse::Button::Left), tile);
        assert!(events.contains(&CanvasEvent::Select(vec![1, 2])), "{events:?}");
    }

    /// Pointer down reports a press and does not reset on a second press.
    /// The app turns the press into a flip or a hold.
    #[test]
    fn a_button_reports_press_and_a_second_press_is_not_a_reset() {
        let scene = Scene::new(vec![button_module()], Vec::new());
        let centre = scene.module(1).unwrap().widget_bounds(0).unwrap().center();
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State::default();
        let first = drive(&canvas, &mut state, &click(mouse::Button::Left), centre);
        assert_eq!(first, vec![CanvasEvent::ButtonPressed { module: 1, widget: 0 }]);
        assert!(matches!(
            state.interaction,
            Interaction::Holding { module: 1, widget: 0 }
        ));

        let second = drive(&canvas, &mut state, &click(mouse::Button::Left), centre);
        assert_eq!(
            second,
            vec![
                CanvasEvent::ButtonReleased { module: 1, widget: 0 },
                CanvasEvent::ButtonPressed { module: 1, widget: 0 },
            ]
        );
        assert!(
            second
                .iter()
                .all(|event| !matches!(event, CanvasEvent::ResetControl { .. }))
        );
    }

    /// Release is reported even after the pointer has left the button.
    /// Dragging does not move the tile.
    #[test]
    fn a_button_reports_release_outside() {
        let scene = Scene::new(vec![button_module()], Vec::new());
        let centre = scene.module(1).unwrap().widget_bounds(0).unwrap().center();
        let outside = Point::new(centre.x + 400.0, centre.y);
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State::default();

        let down = drive(&canvas, &mut state, &click(mouse::Button::Left), centre);
        assert_eq!(down, vec![CanvasEvent::ButtonPressed { module: 1, widget: 0 }]);

        let moved = drive(
            &canvas,
            &mut state,
            &Event::Mouse(mouse::Event::CursorMoved { position: outside }),
            outside,
        );
        assert!(moved.iter().all(|event| !matches!(event, CanvasEvent::Move { .. })));

        let up = drive(
            &canvas,
            &mut state,
            &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            outside,
        );
        assert_eq!(up, vec![CanvasEvent::ButtonReleased { module: 1, widget: 0 }]);
        assert!(matches!(state.interaction, Interaction::Idle));
    }

    /// Another mouse button ends the hold. Leaving the interaction as a pan
    /// without a release would stick a momentary gate high.
    #[test]
    fn another_mouse_button_releases_a_hold() {
        let scene = Scene::new(vec![button_module()], Vec::new());
        let centre = scene.module(1).unwrap().widget_bounds(0).unwrap().center();
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State::default();
        drive(&canvas, &mut state, &click(mouse::Button::Left), centre);

        let right = drive(&canvas, &mut state, &click(mouse::Button::Right), centre);
        assert_eq!(right, vec![CanvasEvent::ButtonReleased { module: 1, widget: 0 }]);
        assert!(matches!(state.interaction, Interaction::Panning { .. }));
    }

    /// Losing the window while a key or the pointer is held reports a release.
    /// The key-up event never arrives after the window is unfocused.
    #[test]
    fn losing_the_window_releases_a_hold() {
        let scene = Scene::new(vec![button_module()], Vec::new());
        let centre = scene.module(1).unwrap().widget_bounds(0).unwrap().center();
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State {
            interaction: Interaction::Holding { module: 1, widget: 0 },
            key_holds: [
                Some(KeyHold {
                    key: keyboard::key::Named::Space,
                    module: 1,
                    widget: 0,
                }),
                None,
            ],
            ..State::default()
        };
        let events = drive(&canvas, &mut state, &Event::Window(window::Event::Unfocused), centre);
        assert_eq!(events, vec![CanvasEvent::ButtonReleased { module: 1, widget: 0 }]);
        assert!(matches!(state.interaction, Interaction::Idle));
        assert!(state.key_holds.iter().all(Option::is_none));
    }

    fn key_event(named: keyboard::key::Named, pressed: bool) -> Event {
        let key = keyboard::Key::Named(named);
        let code = match named {
            keyboard::key::Named::Enter => keyboard::key::Code::Enter,
            _ => keyboard::key::Code::Space,
        };
        let physical_key = keyboard::key::Physical::Code(code);
        if pressed {
            Event::Keyboard(keyboard::Event::KeyPressed {
                modified_key: key.clone(),
                key,
                physical_key,
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::default(),
                text: None,
                repeat: false,
            })
        } else {
            Event::Keyboard(keyboard::Event::KeyReleased {
                modified_key: key.clone(),
                key,
                physical_key,
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::default(),
            })
        }
    }

    /// Space on one button and Enter on another. Releasing one key releases
    /// only the button that key was holding.
    #[test]
    fn each_key_releases_only_its_own_hold() {
        let mut module = button_module();
        module.panel.width = 12;
        module.panel.widgets.push(widget(WidgetViewKind::Button, 6));
        let scene = Scene::new(vec![module], Vec::new());
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State {
            focus: Some((1, 0)),
            ..State::default()
        };
        let at = Point::new(20.0, 20.0);

        let space = drive(&canvas, &mut state, &key_event(keyboard::key::Named::Space, true), at);
        assert_eq!(space, vec![CanvasEvent::ButtonPressed { module: 1, widget: 0 }]);

        state.focus = Some((1, 1));
        let enter = drive(&canvas, &mut state, &key_event(keyboard::key::Named::Enter, true), at);
        assert_eq!(enter, vec![CanvasEvent::ButtonPressed { module: 1, widget: 1 }]);

        let up_space = drive(&canvas, &mut state, &key_event(keyboard::key::Named::Space, false), at);
        assert_eq!(up_space, vec![CanvasEvent::ButtonReleased { module: 1, widget: 0 }]);

        let up_enter = drive(&canvas, &mut state, &key_event(keyboard::key::Named::Enter, false), at);
        assert_eq!(up_enter, vec![CanvasEvent::ButtonReleased { module: 1, widget: 1 }]);
    }

    /// Both keys can hold one button. Releasing the first leaves it held.
    #[test]
    fn releasing_one_key_keeps_a_button_the_other_key_holds() {
        let scene = Scene::new(vec![button_module()], Vec::new());
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State {
            focus: Some((1, 0)),
            ..State::default()
        };
        let at = Point::new(20.0, 20.0);
        drive(&canvas, &mut state, &key_event(keyboard::key::Named::Space, true), at);
        drive(&canvas, &mut state, &key_event(keyboard::key::Named::Enter, true), at);

        let up_space = drive(&canvas, &mut state, &key_event(keyboard::key::Named::Space, false), at);
        assert!(up_space.is_empty());

        let up_enter = drive(&canvas, &mut state, &key_event(keyboard::key::Named::Enter, false), at);
        assert_eq!(up_enter, vec![CanvasEvent::ButtonReleased { module: 1, widget: 0 }]);
    }

    /// A right-button release does not end a hold the left button started.
    #[test]
    fn releasing_another_mouse_button_keeps_the_left_hold() {
        let scene = Scene::new(vec![button_module()], Vec::new());
        let centre = scene.module(1).unwrap().widget_bounds(0).unwrap().center();
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State::default();
        drive(&canvas, &mut state, &click(mouse::Button::Left), centre);

        let right = drive(
            &canvas,
            &mut state,
            &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)),
            centre,
        );
        assert!(right.is_empty());
        assert!(matches!(
            state.interaction,
            Interaction::Holding { module: 1, widget: 0 }
        ));
    }

    /// The gate stays down only when neither the pointer nor a key is holding.
    #[test]
    fn a_key_hold_survives_pointer_up_and_the_pointer_survives_a_key_up() {
        let scene = Scene::new(vec![button_module()], Vec::new());
        let centre = scene.module(1).unwrap().widget_bounds(0).unwrap().center();
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State {
            focus: Some((1, 0)),
            ..State::default()
        };
        drive(&canvas, &mut state, &click(mouse::Button::Left), centre);
        drive(
            &canvas,
            &mut state,
            &key_event(keyboard::key::Named::Space, true),
            centre,
        );

        let pointer_up = drive(
            &canvas,
            &mut state,
            &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            centre,
        );
        assert!(pointer_up.is_empty());
        assert!(matches!(state.interaction, Interaction::Idle));

        state.interaction = Interaction::Holding { module: 1, widget: 0 };
        let key_up = drive(
            &canvas,
            &mut state,
            &key_event(keyboard::key::Named::Space, false),
            centre,
        );
        assert!(key_up.is_empty());
        assert!(matches!(
            state.interaction,
            Interaction::Holding { module: 1, widget: 0 }
        ));

        let pointer_up = drive(
            &canvas,
            &mut state,
            &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            centre,
        );
        assert_eq!(pointer_up, vec![CanvasEvent::ButtonReleased { module: 1, widget: 0 }]);
    }

    /// Space with no focused control asks the app to toggle Run. Enter does
    /// not, a repeat does not, and a focused button still takes Space.
    #[test]
    fn space_with_no_focus_toggles_run() {
        let scene = Scene::new(vec![button_module()], Vec::new());
        let at = Point::new(10.0, 10.0);
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State::default();

        let space = drive(&canvas, &mut state, &key_event(keyboard::key::Named::Space, true), at);
        assert_eq!(space, vec![CanvasEvent::ToggleRun]);

        let mut repeat = key_event(keyboard::key::Named::Space, true);
        if let Event::Keyboard(keyboard::Event::KeyPressed { repeat, .. }) = &mut repeat {
            *repeat = true;
        }
        let again = drive(&canvas, &mut state, &repeat, at);
        assert!(again.is_empty());

        let enter = drive(&canvas, &mut state, &key_event(keyboard::key::Named::Enter, true), at);
        assert!(enter.is_empty());

        state.focus = Some((1, 0));
        let focused = drive(&canvas, &mut state, &key_event(keyboard::key::Named::Space, true), at);
        assert_eq!(focused, vec![CanvasEvent::ButtonPressed { module: 1, widget: 0 }]);
    }

    fn learning<'a>(
        scene: &'a Scene,
        selected: &'a BTreeSet<u32>,
        learn: &'a LearnView,
    ) -> PatchCanvas<'a, CanvasEvent> {
        PatchCanvas::new(scene, selected, |e| e).learn(Some(learn))
    }

    fn arrow_up() -> Event {
        let key = keyboard::Key::Named(keyboard::key::Named::ArrowUp);
        Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::ArrowUp),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        })
    }

    /// In MIDI learn mode a click arms a knob or a button and takes the
    /// keyboard focus. It never turns the knob or presses the button.
    #[test]
    fn a_learn_click_arms_and_focuses_without_changing_a_value() {
        let mut m = module(1, 0.0);
        m.panel = panel();
        let button = ModuleView {
            id: 2,
            position: Point::new(400.0, 0.0),
            ..button_module()
        };
        let scene = Scene::new(vec![m, button], Vec::new());
        let selected = BTreeSet::from([1]);
        let learn = LearnView::default();
        let canvas = learning(&scene, &selected, &learn);
        let mut state = State {
            focus: Some((1, 2)),
            tab_focus: true,
            ..State::default()
        };
        let knob = scene.module(1).unwrap().widget_bounds(0).unwrap().center();
        let events = drive(&canvas, &mut state, &click(mouse::Button::Left), knob);
        assert_eq!(events, vec![CanvasEvent::Learn { module: 1, widget: 0 }]);
        assert!(matches!(state.interaction, Interaction::Idle));
        assert_eq!(state.focus, Some((1, 0)));

        let button = scene.module(2).unwrap().widget_bounds(0).unwrap().center();
        let events = drive(&canvas, &mut state, &click(mouse::Button::Left), button);
        assert!(
            events.contains(&CanvasEvent::Learn { module: 2, widget: 0 }),
            "{events:?}"
        );
        assert!(
            events.iter().all(|e| !matches!(e, CanvasEvent::ButtonPressed { .. })),
            "{events:?}"
        );
    }

    /// In MIDI learn mode Space and Enter arm the focused control, arrows do
    /// nothing, and Escape is left for the app, which ends the mode.
    #[test]
    fn learn_mode_keys_arm_and_never_change_a_value() {
        let mut m = module(1, 0.0);
        m.panel = panel();
        let scene = Scene::new(vec![m], Vec::new());
        let selected = BTreeSet::from([1]);
        let learn = LearnView::default();
        let canvas = learning(&scene, &selected, &learn);
        let at = Point::new(10.0, 10.0);
        let mut state = State {
            focus: Some((1, 0)),
            tab_focus: true,
            ..State::default()
        };
        for key in [keyboard::key::Named::Space, keyboard::key::Named::Enter] {
            let events = drive(&canvas, &mut state, &key_event(key, true), at);
            assert_eq!(events, vec![CanvasEvent::Learn { module: 1, widget: 0 }]);
        }
        assert!(drive(&canvas, &mut state, &arrow_up(), at).is_empty());

        let escape = {
            let key = keyboard::Key::Named(keyboard::key::Named::Escape);
            Event::Keyboard(keyboard::Event::KeyPressed {
                modified_key: key.clone(),
                key: key.clone(),
                physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Escape),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::default(),
                text: None,
                repeat: false,
            })
        };
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
        canvas.handle(&mut state, &escape, bounds, mouse::Cursor::Available(at), &mut shell);
        assert_eq!(
            shell.event_status(),
            iced::event::Status::Ignored,
            "the app must see Escape"
        );
        assert_eq!(state.focus, None);

        // With nothing focused, Space is still the Run shortcut.
        let events = drive(&canvas, &mut state, &key_event(keyboard::key::Named::Space, true), at);
        assert_eq!(events, vec![CanvasEvent::ToggleRun]);
    }

    /// Learn mode outlines controls on screen only.
    #[test]
    fn learn_outlines_cover_only_visible_tiles() {
        let mut near = module(1, 0.0);
        near.panel = panel();
        let far = ModuleView {
            panel: panel(),
            ..module(2, 5_000.0)
        };
        let scene = Scene::new(vec![near, far], Vec::new());
        let none = BTreeSet::new();
        let learn = LearnView::default();
        let canvas = learning(&scene, &none, &learn);
        let plain = canvas.overlay(&State::default()).shapes.len();
        let view = Rectangle::new(Point::new(-50.0, -50.0), Size::new(800.0, 600.0));
        // Two controls (a knob and a toggle) on the one visible tile.
        assert_eq!(canvas.overlay_in(&State::default(), view).shapes.len(), plain + 2);
    }

    fn knob_scene() -> (Scene, Point) {
        let mut m = module(1, 0.0);
        m.panel = panel();
        let scene = Scene::new(vec![m], Vec::new());
        let knob = scene.module(1).unwrap().widget_bounds(0).unwrap().center();
        (scene, knob)
    }

    fn moved(to: Point) -> Event {
        Event::Mouse(mouse::Event::CursorMoved { position: to })
    }

    /// A source tile with one output, and a tile with a knob, a light, and
    /// a toggle further right.
    fn assign_scene() -> Scene {
        let target = ModuleView {
            panel: panel(),
            ..module(2, 300.0)
        };
        Scene::new(vec![module(1, 0.0), target], Vec::new())
    }

    const OUT: PortRef = PortRef {
        module: 1,
        output: true,
        index: 0,
    };

    /// Drags from the source output to widget `widget` on tile 2 and lets
    /// go there. Returns the events and the overlay shape count while over
    /// the widget.
    fn drop_on(canvas: &PatchCanvas<'_, CanvasEvent>, scene: &Scene, widget: usize) -> (Vec<CanvasEvent>, usize) {
        let mut state = State::default();
        let start = scene.port_position(OUT).unwrap();
        drive(canvas, &mut state, &click(mouse::Button::Left), start);
        let over = scene.module(2).unwrap().widget_bounds(widget).unwrap().center();
        drive(canvas, &mut state, &moved(over), over);
        let shapes = canvas.overlay(&state).shapes.len();
        (drive(canvas, &mut state, &released(), over), shapes)
    }

    /// Dropping an output on a knob or toggle assigns it, and the control is
    /// highlighted while the drag is over it. A light takes nothing
    /// (modulation.md, Making an assignment).
    #[test]
    fn dropping_an_output_on_a_control_assigns_it() {
        let scene = assign_scene();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        let (events, on_knob) = drop_on(&canvas, &scene, 0);
        assert_eq!(
            events,
            vec![
                CanvasEvent::Assign {
                    from: OUT,
                    module: 2,
                    widget: 0
                },
                CanvasEvent::GestureEnd
            ]
        );
        let (events, _) = drop_on(&canvas, &scene, 2);
        assert!(events.contains(&CanvasEvent::Assign {
            from: OUT,
            module: 2,
            widget: 2
        }));
        let (events, on_light) = drop_on(&canvas, &scene, 1);
        assert_eq!(events, vec![CanvasEvent::GestureEnd]);
        assert_eq!(
            on_knob,
            on_light + 1,
            "only a control that takes the drop is highlighted"
        );
    }

    /// In MIDI learn mode a drop on a control makes no assignment, and
    /// nothing is highlighted.
    #[test]
    fn learn_mode_takes_no_assignment_drop() {
        let scene = assign_scene();
        let none = BTreeSet::new();
        let learn = LearnView::default();
        let canvas = learning(&scene, &none, &learn);
        let (events, _) = drop_on(&canvas, &scene, 0);
        assert_eq!(events, vec![CanvasEvent::GestureEnd]);
    }

    /// The assign scene with one assignment from the source output to the
    /// knob on tile 2, and the knob showing its range.
    fn modulated_scene() -> (Scene, PanelValues) {
        let mut source = module(1, 0.0);
        source.outputs[0].badge = 1;
        let target = ModuleView {
            panel: panel(),
            ..module(2, 300.0)
        };
        let scene = Scene::new(vec![source, target], Vec::new()).with_modulations(vec![crate::scene::ModulationView {
            from: (1, 0),
            to: 2,
            widget: Some(0),
        }]);
        let mut values = PanelValues::new();
        values.set(
            2,
            0,
            crate::controls::WidgetValue {
                position: 0.5,
                modulation: crate::controls::ModulationMark::Range { low: 0.5, high: 0.75 },
                ..Default::default()
            },
        );
        (scene, values)
    }

    fn escape() -> Event {
        let key = keyboard::Key::Named(keyboard::key::Named::Escape);
        Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Escape),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        })
    }

    /// Lines show while the pointer is on the source output, its badge, or
    /// the modulated control, and not elsewhere. A badge click pins them;
    /// a second click or Escape unpins, and Escape is used up only then.
    #[test]
    fn modulation_lines_follow_hover_and_the_pinned_badge() {
        let (scene, values) = modulated_scene();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e).values(&values);
        let lines = |state: &State| canvas.overlay(state).cables.len();
        let badge = scene.module(1).unwrap().badge_bounds(0).unwrap().center();
        assert_eq!(scene.hit(badge, PORT_HIT_RADIUS), Hit::Badge(OUT));
        let knob = scene.module(2).unwrap().widget_bounds(0).unwrap().center();
        let port = scene.port_position(OUT).unwrap();
        let empty = Point::new(200.0, 400.0);

        let mut state = State::default();
        for (at, shown) in [(port, 1), (badge, 1), (knob, 1), (empty, 0)] {
            drive(&canvas, &mut state, &moved(at), at);
            assert_eq!(lines(&state), shown, "pointer at {at:?}");
        }

        drive(&canvas, &mut state, &moved(badge), badge);
        assert!(drive(&canvas, &mut state, &click(mouse::Button::Left), badge).is_empty());
        drive(&canvas, &mut state, &released(), badge);
        drive(&canvas, &mut state, &moved(empty), empty);
        assert_eq!(state.pinned, Some(OUT));
        assert_eq!(lines(&state), 1, "pinned lines stay");

        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
        canvas.handle(
            &mut state,
            &escape(),
            bounds,
            mouse::Cursor::Available(empty),
            &mut shell,
        );
        assert_eq!(shell.event_status(), iced::event::Status::Captured);
        assert_eq!(state.pinned, None);
        assert_eq!(lines(&state), 0);
        let mut shell = Shell::new(&mut messages);
        canvas.handle(
            &mut state,
            &escape(),
            bounds,
            mouse::Cursor::Available(empty),
            &mut shell,
        );
        assert_eq!(
            shell.event_status(),
            iced::event::Status::Ignored,
            "with nothing pinned the app sees Escape"
        );

        // Two clicks on the badge pin and unpin.
        drive(&canvas, &mut state, &click(mouse::Button::Left), badge);
        drive(&canvas, &mut state, &click(mouse::Button::Left), badge);
        assert_eq!(state.pinned, None);
    }

    /// A drag on the ring around a modulated knob changes depth, up to
    /// raise it, as one gesture, and carries the pinned output. A drag on
    /// the knob body still turns it.
    #[test]
    fn dragging_the_range_ring_changes_depth() {
        let (scene, values) = modulated_scene();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e).values(&values);
        let rect = scene.module(2).unwrap().widget_bounds(0).unwrap();
        let centre = crate::controls::knob_centre(rect);
        let ring = Point::new(centre.x, centre.y - 12.0);

        let mut state = State {
            pinned: Some(OUT),
            ..State::default()
        };
        let down = drive(&canvas, &mut state, &click(mouse::Button::Left), ring);
        assert!(down.contains(&CanvasEvent::GestureStart), "{down:?}");
        // The selection stays: a selected source picks the assignment.
        assert!(!down.iter().any(|e| matches!(e, CanvasEvent::Select(_))), "{down:?}");
        let up = Point::new(ring.x, ring.y - 20.0);
        let moved_events = drive(&canvas, &mut state, &moved(up), up);
        let [
            CanvasEvent::DragDepth {
                module: 2,
                widget: 0,
                pinned: Some(pinned),
                delta,
            },
        ] = moved_events.as_slice()
        else {
            panic!("{moved_events:?}");
        };
        assert_eq!(*pinned, OUT);
        assert!((delta - 0.1).abs() < 1e-6, "{delta}");
        assert_eq!(
            drive(&canvas, &mut state, &released(), up),
            vec![CanvasEvent::GestureEnd]
        );

        let mut state = State::default();
        drive(&canvas, &mut state, &click(mouse::Button::Left), centre);
        let up = Point::new(centre.x, centre.y - 20.0);
        let turned = drive(&canvas, &mut state, &moved(up), up);
        assert!(
            matches!(turned.as_slice(), [CanvasEvent::SetControl { .. }]),
            "{turned:?}"
        );
    }

    /// A pin whose badge is gone (its last assignment removed) counts as
    /// none: Escape goes to the app, and an arc drag carries no pin.
    #[test]
    fn a_pin_without_its_badge_takes_no_escape() {
        let (mut scene, values) = modulated_scene();
        let modules = scene.modules().to_vec();
        let mut source = modules[0].clone();
        source.outputs[0].badge = 0;
        scene = Scene::new(vec![source, modules[1].clone()], Vec::new());
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e).values(&values);
        let mut state = State {
            pinned: Some(OUT),
            ..State::default()
        };
        assert_eq!(canvas.pin(&state), None);
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
        canvas.handle(
            &mut state,
            &escape(),
            bounds,
            mouse::Cursor::Available(Point::ORIGIN),
            &mut shell,
        );
        assert_eq!(shell.event_status(), iced::event::Status::Ignored);
    }

    /// A new patch (a new frame key) drops the pin: module IDs repeat
    /// between patches, so it could land on an unrelated output.
    #[test]
    fn a_new_patch_clears_the_pin() {
        let (scene, values) = modulated_scene();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e).values(&values).frame_key(2);
        let mut state = State {
            pinned: Some(OUT),
            framed_key: Some(1),
            ..State::default()
        };
        let redraw = Event::Window(window::Event::RedrawRequested(Instant::now()));
        drive(&canvas, &mut state, &redraw, Point::ORIGIN);
        assert_eq!(state.pinned, None);
    }

    /// The lines are built once for a given pin, pointer, and selection,
    /// and reused while those stay the same.
    #[test]
    fn modulation_lines_are_reused_until_their_inputs_change() {
        let (scene, values) = modulated_scene();
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e).values(&values);
        let state = State::default();
        let first = canvas.overlay(&state).cables.len();
        let key = state.lines.borrow().as_ref().map(|(k, _)| k.clone());
        assert_eq!(canvas.overlay(&state).cables.len(), first);
        assert_eq!(state.lines.borrow().as_ref().map(|(k, _)| k.clone()), key);
        assert_eq!(first, 1, "the selected source's line");
    }

    /// A knob whose arc has zero length shows the dot, and its ring still
    /// takes a depth drag, so depth 0 can be dragged back out of.
    #[test]
    fn a_zero_length_arc_still_takes_a_depth_drag() {
        let (scene, _) = modulated_scene();
        let mut values = PanelValues::new();
        values.set(
            2,
            0,
            crate::controls::WidgetValue {
                modulation: crate::controls::ModulationMark::Dot,
                ..Default::default()
            },
        );
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e).values(&values);
        let centre = crate::controls::knob_centre(scene.module(2).unwrap().widget_bounds(0).unwrap());
        let mut state = State::default();
        drive(
            &canvas,
            &mut state,
            &click(mouse::Button::Left),
            Point::new(centre.x + 12.0, centre.y),
        );
        assert!(matches!(state.interaction, Interaction::Depth { .. }));
    }

    /// A knob with no enabled assignment has no ring to grab: a press there
    /// turns the knob.
    #[test]
    fn an_unmodulated_knob_has_no_range_ring() {
        let (scene, _) = modulated_scene();
        let values = PanelValues::new();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e).values(&values);
        let centre = crate::controls::knob_centre(scene.module(2).unwrap().widget_bounds(0).unwrap());
        // Beside the knob, inside its cell, where the ring would be.
        let ring = Point::new(centre.x + 12.0, centre.y);
        let mut state = State::default();
        drive(&canvas, &mut state, &click(mouse::Button::Left), ring);
        assert!(matches!(state.interaction, Interaction::Turning { .. }));
    }

    /// A cable pulled off a connected input and dropped on a control is
    /// discarded. It does not become an assignment.
    #[test]
    fn a_pulled_cable_dropped_on_a_control_does_not_assign() {
        let cable = CableView {
            from: (1, 0),
            to: (2, 0),
            colour_from: Color::WHITE,
            colour_to: Color::WHITE,
            delayed: false,
        };
        let target = ModuleView {
            panel: panel(),
            ..module(2, 300.0)
        };
        let scene = Scene::new(vec![module(1, 0.0), target], vec![cable]);
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        let mut state = State::default();
        let input = PortRef {
            module: 2,
            output: false,
            index: 0,
        };
        let start = scene.port_position(input).unwrap();
        let down = drive(&canvas, &mut state, &click(mouse::Button::Left), start);
        assert!(down.contains(&CanvasEvent::Disconnect(input)), "{down:?}");
        let knob = scene.module(2).unwrap().widget_bounds(0).unwrap().center();
        drive(&canvas, &mut state, &moved(knob), knob);
        let up = drive(&canvas, &mut state, &released(), knob);
        assert_eq!(up, vec![CanvasEvent::GestureEnd]);
    }

    /// A drag that starts on an input never assigns.
    #[test]
    fn an_input_drag_does_not_assign() {
        let scene = assign_scene();
        let none = BTreeSet::new();
        let canvas = PatchCanvas::new(&scene, &none, |e| e);
        let mut state = State::default();
        let input = PortRef { output: false, ..OUT };
        let start = scene.port_position(input).unwrap();
        drive(&canvas, &mut state, &click(mouse::Button::Left), start);
        let knob = scene.module(2).unwrap().widget_bounds(0).unwrap().center();
        drive(&canvas, &mut state, &moved(knob), knob);
        assert_eq!(
            drive(&canvas, &mut state, &released(), knob),
            vec![CanvasEvent::GestureEnd]
        );
    }

    fn released() -> Event {
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
    }

    /// A knob drag is one gesture: start at pointer down, end at pointer up
    /// (undo.md, Gestures).
    #[test]
    fn a_knob_drag_is_bracketed_by_a_gesture_start_and_end() {
        let (scene, knob) = knob_scene();
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State::default();
        let down = drive(&canvas, &mut state, &click(mouse::Button::Left), knob);
        assert_eq!(down, vec![CanvasEvent::GestureStart]);
        let up = Point::new(knob.x, knob.y - 20.0);
        let during = drive(&canvas, &mut state, &moved(up), up);
        assert!(
            matches!(during.as_slice(), [CanvasEvent::SetControl { .. }]),
            "{during:?}"
        );
        let end = drive(&canvas, &mut state, &released(), up);
        assert_eq!(end, vec![CanvasEvent::GestureEnd]);
    }

    /// A drag cut off by the window losing focus still ends its gesture, once.
    #[test]
    fn losing_the_window_ends_a_drag_gesture() {
        let (scene, knob) = knob_scene();
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State::default();
        drive(&canvas, &mut state, &click(mouse::Button::Left), knob);
        let lost = drive(&canvas, &mut state, &Event::Window(window::Event::Unfocused), knob);
        assert_eq!(lost, vec![CanvasEvent::GestureEnd]);
        let later = drive(&canvas, &mut state, &released(), knob);
        assert!(later.is_empty(), "{later:?}");
    }

    /// A second button pressed in a drag replaces the interaction, and must
    /// end the gesture first.
    #[test]
    fn another_button_during_a_drag_ends_the_gesture() {
        let (scene, knob) = knob_scene();
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State::default();
        drive(&canvas, &mut state, &click(mouse::Button::Left), knob);
        let events = drive(&canvas, &mut state, &click(mouse::Button::Right), knob);
        assert_eq!(events, vec![CanvasEvent::GestureEnd]);
    }

    /// A held arrow key repeats. The whole hold, key down to key up, is one
    /// gesture.
    #[test]
    fn a_held_arrow_key_is_one_gesture() {
        let (scene, knob) = knob_scene();
        let selected = BTreeSet::from([1]);
        let canvas = PatchCanvas::new(&scene, &selected, |e| e);
        let mut state = State {
            focus: Some((1, 0)),
            ..State::default()
        };
        let first = drive(&canvas, &mut state, &arrow_up(), knob);
        assert_eq!(first.first(), Some(&CanvasEvent::GestureStart), "{first:?}");
        let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            location,
            modifiers,
            text,
            ..
        }) = arrow_up()
        else {
            unreachable!()
        };
        let again = Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key,
            physical_key,
            location,
            modifiers,
            text,
            repeat: true,
        });
        let repeat = drive(&canvas, &mut state, &again, knob);
        assert!(
            repeat.iter().all(|e| !matches!(e, CanvasEvent::GestureStart)),
            "{repeat:?}"
        );
        let up = Event::Keyboard(keyboard::Event::KeyReleased {
            modified_key: key.clone(),
            key,
            physical_key,
            location: keyboard::Location::Standard,
            modifiers,
        });
        let end = drive(&canvas, &mut state, &up, knob);
        assert_eq!(end, vec![CanvasEvent::GestureEnd]);
    }
}
