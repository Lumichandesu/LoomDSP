//! Headless UI tests: real pointer and key events sent to the app view.
//!
//! The other app tests call `App::update` with messages. These go one step
//! earlier and check that Iced routes each event to the right widget. The
//! [`Ui`] harness does what the Iced runtime does for each frame: build a
//! `UserInterface` from `App::view`, send it events with a cursor position,
//! apply the messages it returns with `App::update`, and keep the widget
//! cache for the next frame, so the canvas remembers a drag in progress.
//!
//! The renderer is Iced's headless renderer. It uses wgpu when a GPU
//! adapter is found and falls back to tiny-skia otherwise, so these tests
//! run on CI machines without a GPU. Events do not depend on which one runs.
//! Set `ICED_TEST_BACKEND=tiny-skia` to force the fallback.
//!
//! Tests find things by name, not by pixel: [`Ui::output`], [`Ui::input`],
//! [`Ui::control`], and [`Ui::text`] return window coordinates from the
//! canvas bounds, its camera, and the scene. So a test keeps working when a
//! tile moves or the inspector gets wider.
//!
//! Tasks that `App::update` returns run as the runtime runs them: widget
//! operations (such as focusing the browser's search field) on the new
//! view, clipboard reads and writes on an in-memory clipboard, and exit as
//! [`Ui::exited`]. A message a task produces is applied like any other. A
//! task the harness cannot run, such as a window command or one that waits
//! on a timer or a file dialog, panics with a message, so a test never
//! passes because something was quietly skipped.
//!
//! Subscriptions do not run, except the two key listeners the app uses:
//! `keyboard::listen` and, while the browser is open, `browser_key`.
//!
//! Time is the wall clock, and steps take microseconds. So two clicks on
//! the same control or on empty canvas in one test count as a double-click
//! (a control resets, the browser opens). Click something else in between
//! when a test needs two single clicks.
//!
//! Not covered: what the operating system and winit deliver, for example
//! events while the pointer is outside the window.

use crate::{App, Args, Message, screenshot};
use iced::advanced::Clipboard;
use iced::advanced::clipboard::Kind;
use iced::advanced::renderer::{Headless, Style};
use iced::advanced::widget::Id;
use iced::advanced::widget::Operation;
use iced::advanced::widget::operation::Outcome;
use iced::{Event, Point, Rectangle, Size, Task, event, keyboard, mouse, window};
use iced_runtime::Action;
use iced_runtime::futures::futures::{FutureExt, StreamExt};
use iced_runtime::user_interface::{Cache, UserInterface};
use loom_canvas::{CanvasView, PortRef};
use loom_core::{ModuleId, OutputSafety};
use std::any::Any;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

/// The window size the app opens with.
pub const WINDOW: Size = Size::new(1440.0, 900.0);

/// The app view, driven by events the way the Iced runtime drives it.
pub struct Ui {
    pub app: App,
    renderer: iced::Renderer,
    cache: Option<Cache>,
    cursor: mouse::Cursor,
    /// The clipboard, shared by widgets and tasks. One text for both the
    /// standard and the primary clipboard.
    pub clipboard: Memory,
    /// True once a task asked the app to exit.
    pub exited: bool,
}

/// An in-memory clipboard.
#[derive(Debug, Default)]
pub struct Memory(pub Option<String>);

impl Clipboard for Memory {
    fn read(&self, _kind: Kind) -> Option<String> {
        self.0.clone()
    }

    fn write(&mut self, _kind: Kind, contents: String) {
        self.0 = Some(contents);
    }
}

impl Ui {
    /// Opens a patch from `examples/patches` with no audio, as the app does
    /// in a window of [`WINDOW`] size, and draws the first frame.
    pub fn open(patch: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/patches")
            .join(patch);
        let app = App::new(Args {
            patch: Some(path),
            generate: None,
            no_audio: true,
            bench: false,
            output_safety: OutputSafety::LowLatency,
            view: screenshot::Options::default(),
        });
        // `ICED_TEST_BACKEND=tiny-skia` forces the renderer CI falls back
        // to without a GPU, as for Iced's own test crate.
        let backend = std::env::var("ICED_TEST_BACKEND").ok();
        let renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
            iced::Font::DEFAULT,
            iced::Pixels(16.0),
            backend.as_deref(),
        ))
        .expect("Iced's headless renderer falls back to tiny-skia without a GPU");
        let mut ui = Self {
            app,
            renderer,
            cache: Some(Cache::default()),
            cursor: mouse::Cursor::Unavailable,
            clipboard: Memory::default(),
            exited: false,
        };
        // The canvas frames the patch on its first redraw.
        ui.redraw();
        ui
    }

    /// One frame: the events go to the view at the current cursor, the
    /// messages they produce are applied, their tasks run, and the new view
    /// is drawn. Returns every message, including those tasks produced.
    pub fn step(&mut self, events: &[Event]) -> Vec<Message> {
        let mut messages = Vec::new();
        let browser_open = self.app.browser.is_some();
        let mut clipboard = std::mem::take(&mut self.clipboard);
        self.with_ui(|ui, renderer, cursor| {
            let (_, statuses) = ui.update(events, cursor, renderer, &mut clipboard, &mut messages);
            for (event, status) in events.iter().zip(statuses) {
                // `keyboard::listen` passes on the key events no widget
                // captured.
                if let (Event::Keyboard(key), event::Status::Ignored) = (event, status) {
                    messages.push(Message::Key(key.clone()));
                }
                // The open browser listens to keys whether or not a widget
                // used them.
                if browser_open && let Some(message) = crate::browser_key(event.clone(), status, window::Id::unique()) {
                    messages.push(message);
                }
            }
        });
        self.clipboard = clipboard;
        let mut queue: VecDeque<Message> = messages.iter().cloned().collect();
        while let Some(message) = queue.pop_front() {
            let task = self.app.update(message);
            self.run(task, &mut queue, &mut messages);
        }
        let theme = self.app.theme();
        let style = Style {
            text_color: theme.palette().text,
        };
        self.with_ui(|ui, renderer, cursor| ui.draw(renderer, &theme, &style, cursor));
        messages
    }

    /// Runs a task to the end. Messages it produces go to `queue` and to
    /// `log`. Widget operations run on a view built from the app as it is
    /// now, so they see what the update just changed.
    fn run(&mut self, task: Task<Message>, queue: &mut VecDeque<Message>, log: &mut Vec<Message>) {
        let Some(mut stream) = iced_runtime::task::into_stream(task) else {
            return;
        };
        loop {
            // A stream can return Pending once while its parts hand over,
            // for example after a clipboard reply. A few polls tell that
            // apart from a task that waits on the outside world.
            let polled = (0..8).find_map(|_| stream.next().now_or_never());
            let action = match polled {
                Some(Some(action)) => action,
                Some(None) => return,
                None => panic!("Ui: a task waits on something the harness cannot give, such as a timer or a dialog"),
            };
            match action {
                Action::Output(message) => {
                    log.push(message.clone());
                    queue.push_back(message);
                }
                Action::Widget(operation) => {
                    let mut next = Some(operation);
                    while let Some(mut operation) = next.take() {
                        self.with_ui(|ui, renderer, _| ui.operate(renderer, operation.as_mut()));
                        if let Outcome::Chain(chained) = operation.finish() {
                            next = Some(chained);
                        }
                    }
                }
                Action::Clipboard(iced_runtime::clipboard::Action::Read { channel, .. }) => {
                    let _ = channel.send(self.clipboard.0.clone());
                }
                Action::Clipboard(iced_runtime::clipboard::Action::Write { contents, .. }) => {
                    self.clipboard.0 = Some(contents);
                }
                Action::Exit => self.exited = true,
                other => panic!("Ui: the harness cannot run this task: {other:?}"),
            }
        }
    }

    /// Builds the view, runs `f` on it, and keeps its cache.
    fn with_ui(
        &mut self,
        f: impl FnOnce(&mut UserInterface<'_, Message, iced::Theme, iced::Renderer>, &mut iced::Renderer, mouse::Cursor),
    ) {
        let cache = self.cache.take().expect("the cache is put back after every frame");
        let mut ui = UserInterface::build(self.app.view(), WINDOW, cache, &mut self.renderer);
        f(&mut ui, &mut self.renderer, self.cursor);
        self.cache = Some(ui.into_cache());
    }

    /// A redraw, as the runtime sends before each frame. The canvas uses it
    /// to frame the patch and to report which lights are visible.
    pub fn redraw(&mut self) -> Vec<Message> {
        self.step(&[Event::Window(window::Event::RedrawRequested(Instant::now()))])
    }

    /// Moves the pointer to a window point.
    pub fn move_to(&mut self, p: Point) -> Vec<Message> {
        self.cursor = mouse::Cursor::Available(p);
        self.step(&[Event::Mouse(mouse::Event::CursorMoved { position: p })])
    }

    /// Presses the left button where the pointer is.
    pub fn press(&mut self) -> Vec<Message> {
        self.step(&[Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))])
    }

    /// Releases the left button where the pointer is.
    pub fn release(&mut self) -> Vec<Message> {
        self.step(&[Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))])
    }

    /// Presses at `from`, moves to `to` in `steps` even moves, and releases
    /// there. Returns every message along the way.
    pub fn drag(&mut self, from: Point, to: Point, steps: usize) -> Vec<Message> {
        let mut messages = self.move_to(from);
        messages.extend(self.press());
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            messages.extend(self.move_to(Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t)));
        }
        messages.extend(self.release());
        messages
    }

    /// Presses and releases a key with modifiers held.
    pub fn tap(&mut self, key: keyboard::Key, modifiers: keyboard::Modifiers) -> Vec<Message> {
        let physical = keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified);
        let mut messages = self.step(&[Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key.clone(),
            physical_key: physical,
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })]);
        messages.extend(self.step(&[Event::Keyboard(keyboard::Event::KeyReleased {
            key: key.clone(),
            modified_key: key,
            physical_key: physical,
            location: keyboard::Location::Standard,
            modifiers,
        })]));
        messages
    }

    /// Types text, one key press and release per character, as a keyboard
    /// does. Only the widget with focus takes it.
    pub fn typewrite(&mut self, text: &str) -> Vec<Message> {
        let mut messages = Vec::new();
        for c in text.chars() {
            let key = keyboard::Key::Character(c.to_string().into());
            let physical = keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified);
            messages.extend(self.step(&[Event::Keyboard(keyboard::Event::KeyPressed {
                key: key.clone(),
                modified_key: key.clone(),
                physical_key: physical,
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::default(),
                text: Some(c.to_string().into()),
                repeat: false,
            })]));
            messages.extend(self.step(&[Event::Keyboard(keyboard::Event::KeyReleased {
                key: key.clone(),
                modified_key: key,
                physical_key: physical,
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::default(),
            })]));
        }
        messages
    }

    /// Where the canvas is in the window, and its camera.
    pub fn canvas(&mut self) -> CanvasView {
        #[derive(Default)]
        struct Find(Option<CanvasView>);
        impl Operation for Find {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }
            fn custom(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Any) {
                if let Some(view) = state.downcast_ref::<CanvasView>() {
                    self.0 = Some(*view);
                }
            }
        }
        let mut find = Find::default();
        self.with_ui(|ui, renderer, _| ui.operate(renderer, &mut find));
        find.0.expect("the app view has a patch canvas")
    }

    /// The centre of a module's output port, by its stable name.
    pub fn output(&mut self, module: u32, name: &str) -> Point {
        self.port(module, name, true)
    }

    /// The centre of a module's input port, by its stable name.
    pub fn input(&mut self, module: u32, name: &str) -> Point {
        self.port(module, name, false)
    }

    fn port(&mut self, module: u32, name: &str, output: bool) -> Point {
        let node = self
            .app
            .controller
            .graph()
            .node(ModuleId(module))
            .expect("module in the patch");
        let ports = if output {
            node.descriptor.outputs
        } else {
            node.descriptor.inputs
        };
        let index = ports
            .iter()
            .position(|p| p.name == name)
            .unwrap_or_else(|| panic!("module {module} has no port `{name}`"));
        let world = self
            .app
            .scene
            .port_position(PortRef { module, output, index })
            .expect("the scene has every port in the graph");
        self.canvas().to_window(world)
    }

    /// The centre of the panel control bound to a parameter, by the
    /// parameter's stable name.
    pub fn control(&mut self, module: u32, param: &str) -> Point {
        let target = screenshot::Named {
            module,
            name: param.to_string(),
        };
        let (_, widget) = screenshot::find_control(&self.app.controller, &target).unwrap_or_else(|e| panic!("{e}"));
        let rect = self
            .app
            .scene
            .module(module)
            .and_then(|m| m.widget_bounds(widget))
            .expect("the scene has every panel widget");
        self.canvas().to_window(rect.center())
    }

    /// The bounds of the first text widget that reads exactly `content`,
    /// for example an inspector row label. `None` if no widget shows it.
    pub fn text(&mut self, content: &str) -> Option<Rectangle> {
        struct Find<'a>(&'a str, Option<Rectangle>);
        impl Operation for Find<'_> {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                if self.1.is_none() {
                    operate(self);
                }
            }
            fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
                if self.1.is_none() && text == self.0 {
                    self.1 = Some(bounds);
                }
            }
            fn finish(&self) -> Outcome<()> {
                Outcome::None
            }
        }
        let mut find = Find(content, None);
        self.with_ui(|ui, renderer, _| ui.operate(renderer, &mut find));
        find.1
    }
}

mod tests {
    use super::*;
    use loom_canvas::CanvasEvent;
    use loom_core::{InputRef, OutputRef};

    fn connected(ui: &Ui, from: (u32, usize), to: (u32, usize)) -> bool {
        let output = OutputRef {
            module: ModuleId(from.0),
            port: from.1,
        };
        let input = InputRef {
            module: ModuleId(to.0),
            port: to.1,
        };
        ui.app.controller.graph().cables().any(|cable| cable == (output, input))
    }

    /// A press on an output port, moves onto an input port, and a release
    /// there make a cable, through the canvas's own event handling.
    #[test]
    fn dragging_from_an_output_to_an_input_makes_a_cable() {
        let mut ui = Ui::open("subtractive.ron");
        let from = ui.output(8, "out");
        let to = ui.input(9, "right");
        assert!(!connected(&ui, (8, 0), (9, 1)));
        let cables = ui.app.controller.graph().cables().count();

        ui.drag(from, to, 12);

        assert!(
            connected(&ui, (8, 0), (9, 1)),
            "the drag made a cable from VCA out to right"
        );
        assert_eq!(ui.app.controller.graph().cables().count(), cables + 1);
    }

    /// Ctrl+Z (Cmd+Z on macOS) reaches the app through the canvas and
    /// undoes the cable the drag made.
    #[test]
    fn undo_key_removes_a_dragged_cable() {
        let mut ui = Ui::open("subtractive.ron");
        let from = ui.output(8, "out");
        let to = ui.input(9, "right");
        ui.drag(from, to, 12);
        assert!(connected(&ui, (8, 0), (9, 1)));

        ui.tap(keyboard::Key::Character("z".into()), keyboard::Modifiers::COMMAND);

        assert!(!connected(&ui, (8, 0), (9, 1)), "undo removed the cable");
    }

    /// A over empty canvas opens the browser. Its focus task gives the
    /// search field focus, so typed text goes there, and Enter adds the
    /// first match.
    #[test]
    fn the_browser_search_field_takes_typing() {
        let mut ui = Ui::open("subtractive.ron");
        let modules = ui.app.controller.graph().nodes().count();
        let canvas = ui.canvas();
        ui.move_to(Point::new(canvas.bounds.x + 8.0, canvas.bounds.y + 8.0));
        ui.tap(keyboard::Key::Character("a".into()), keyboard::Modifiers::default());
        assert!(ui.app.browser.is_some(), "A opens the browser");

        ui.typewrite("svf");
        ui.tap(
            keyboard::Key::Named(keyboard::key::Named::Enter),
            keyboard::Modifiers::default(),
        );

        assert!(ui.app.browser.is_none(), "Enter adds the match and closes the browser");
        assert_eq!(ui.app.controller.graph().nodes().count(), modules + 1);
    }

    /// Copy writes the selected module to the clipboard through a task, and
    /// paste reads it back through another.
    #[test]
    fn copy_and_paste_go_through_the_clipboard() {
        let mut ui = Ui::open("subtractive.ron");
        let modules = ui.app.controller.graph().nodes().count();
        let _ = ui.app.update(Message::Canvas(CanvasEvent::Select(vec![4])));
        let canvas = ui.canvas();
        ui.move_to(Point::new(canvas.bounds.x + 8.0, canvas.bounds.y + 8.0));

        ui.tap(keyboard::Key::Character("c".into()), keyboard::Modifiers::COMMAND);
        assert!(ui.clipboard.0.is_some(), "copy wrote the clipboard");
        ui.tap(keyboard::Key::Character("v".into()), keyboard::Modifiers::COMMAND);

        assert_eq!(ui.app.controller.graph().nodes().count(), modules + 1);
    }

    /// The same drag released on empty canvas makes no cable.
    #[test]
    fn dragging_onto_empty_canvas_makes_no_cable() {
        let mut ui = Ui::open("subtractive.ron");
        let from = ui.output(8, "out");
        let cables = ui.app.controller.graph().cables().count();
        let canvas = ui.canvas();
        let empty = Point::new(canvas.bounds.x + 4.0, canvas.bounds.y + 4.0);

        ui.drag(from, empty, 12);

        assert_eq!(ui.app.controller.graph().cables().count(), cables);
    }

    #[test]
    fn finds_controls_and_text_by_name() {
        let mut ui = Ui::open("subtractive.ron");
        let canvas = ui.canvas();
        assert!(canvas.bounds.width > 0.0 && canvas.bounds.height > 0.0);
        let knob = ui.control(4, "cutoff");
        assert!(canvas.bounds.contains(knob), "{knob:?} is on the canvas {canvas:?}");
        assert!(ui.text("no widget shows this").is_none());

        // A click on the filter's title selects it, and the inspector then
        // shows its parameter rows.
        let title = {
            let module = ui.app.scene.module(4).expect("filter in the scene");
            let b = module.bounds();
            Point::new(b.x + b.width * 0.5, b.y + 6.0)
        };
        let title = ui.canvas().to_window(title);
        ui.move_to(title);
        ui.press();
        ui.release();
        assert_eq!(ui.app.selection, [4].into());
        let row = ui.text("Cutoff").expect("the inspector shows the Cutoff row");
        assert!(!canvas.bounds.intersects(&row), "the row {row:?} is beside the canvas");
    }
}
