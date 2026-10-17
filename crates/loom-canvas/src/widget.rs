//! The patch canvas widget: input handling and drawing.

use crate::camera::Camera;
use crate::controls::{ControlGeometry, PanelValues, focus_ring, knob_drag, knob_step, radio_option};
use crate::gpu::{Cable, CanvasPrimitive, Geometry, Overlay, Shape};
use crate::hover::{HOVER_PADDING, HOVER_TEXT_PX, HoverTarget, HoverTimer};
use crate::scene::{GRID, Hit, HoverText, PORT_HIT_RADIUS, PORT_RADIUS, PortRef, Scene, TILE_WIDTH, WidgetViewKind};
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
    animate: bool,
    learn: Option<&'a LearnView>,
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
            animate: false,
            learn: None,
        }
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

    /// Moves the camera continuously in a slow loop. For measuring frame
    /// rate while panning and zooming (ADR-0010 performance gates).
    pub fn animate(mut self, animate: bool) -> Self {
        self.animate = animate;
        self
    }

    fn emit(&self, shell: &mut Shell<'_, Message>, event: CanvasEvent) {
        shell.publish((self.on_event)(event));
    }
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
    /// When hover text shows (panels.md, Labels and hover text).
    hover_text: HoverTimer,
    /// The last press on a control, for double-click.
    last_click: Option<(Instant, u32, usize)>,
    /// Buttons held from the keyboard, one slot for Space and one for Enter.
    ///
    /// Each key is its own hold. Releasing one leaves a button high while the
    /// other key is still down on it. Replacing a key's hold releases the
    /// button that key was holding, when nothing else is holding it.
    key_holds: [Option<KeyHold>; 2],
    /// Modules with lights on screen, as last reported.
    reported_lights: Option<Vec<u32>>,
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
        tree::State::new(State::default())
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
            Interaction::Turning { .. } => mouse::Interaction::ResizingVertically,
            Interaction::Holding { .. } => mouse::Interaction::Pointer,
            Interaction::Wiring { .. } => mouse::Interaction::Crosshair,
            Interaction::Selecting { .. } => mouse::Interaction::Crosshair,
            Interaction::Idle if cursor.is_over(layout.bounds()) => match state.hover {
                Some(Hit::Port(_)) => mouse::Interaction::Pointer,
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

        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                if state.framed_key != Some(self.frame_key) && bounds.width > 0.0 {
                    if let Some(world) = self.scene.bounds() {
                        state.camera.frame(world, bounds.size());
                    }
                    state.framed_key = Some(self.frame_key);
                    state.animation = None;
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
                let world = state.camera.to_world(screen);
                match button {
                    mouse::Button::Middle | mouse::Button::Right => {
                        state.interaction = Interaction::Panning { last: screen };
                    }
                    mouse::Button::Left => {
                        self.press(state, world, screen, shell);
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
                let world = screen.map(|s| state.camera.to_world(s));
                match interaction {
                    Interaction::Wiring { from } => {
                        if let Some(Hit::Port(to)) = world.map(|w| self.scene.hit(w, state.hit_radius())) {
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
                    | Interaction::Turning { .. } => {}
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

    fn press(&self, state: &mut State, world: Point, screen: Point, shell: &mut Shell<'_, Message>) {
        let shift = state.modifiers.shift();
        let hit = self.scene.hit(world, state.hit_radius());
        state.tab_focus = false;
        state.focus_first = false;
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
                        state.interaction = Interaction::Turning {
                            module,
                            widget,
                            last_y: screen.y,
                            position,
                        };
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
                state.interaction = Interaction::Moving {
                    ids: vec![module],
                    start: world,
                    applied: Vector::ZERO,
                    click: click.map(|position| (module, widget, position)),
                };
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
                    self.emit(shell, CanvasEvent::Disconnect(port));
                    state.interaction = Interaction::Wiring { from: source };
                } else {
                    state.interaction = Interaction::Wiring { from: port };
                }
            }
            Hit::Port(port) => {
                state.interaction = Interaction::Wiring { from: port };
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
                    state.interaction = Interaction::Moving {
                        ids: selection.into_iter().collect(),
                        start: world,
                        applied: Vector::ZERO,
                        click: None,
                    };
                }
            }
            Hit::Cable(_) | Hit::Nothing => {
                let base = if shift { self.selection.clone() } else { BTreeSet::new() };
                state.interaction = Interaction::Selecting {
                    start: world,
                    current: world,
                    base,
                };
            }
        }
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
            Interaction::Wiring { from } => {
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
            Interaction::Wiring { from: port },
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
}
