//! `loom-app`: the LoomDSP desktop app.
//!
//! Phase 1 prototype: the patch canvas (ADR-0010), an inspector for the
//! selected module, live audio and MIDI, and a stats bar for measuring the
//! canvas against the performance gates in NFR-PERF-04.

mod alerts;
mod browser;
mod files;
mod layout;
mod learn;
mod lights;
mod live;
mod modulation;
mod monitor;
mod param;
mod scene;
mod spike_line;
mod undo;

use clap::Parser;
use iced::widget::{
    button, column, container, mouse_area, row, rule, scrollable, slider, space, stack, text, toggler, tooltip,
};
use iced::{Color, Element, Length, Point, Subscription, Task, Theme};
use live::Live;
use loom_canvas::{CanvasEvent, HoverText, LearnView, PanelValues, PatchCanvas, Scene};
use loom_core::{
    Controller, EngineConfig, EngineParts, InputRef, MidiEvent, ModuleDescriptor, ModuleId, OutputRef, ProbeId,
    create_engine,
};
use loom_host_desktop::{AudioOptions, AudioOutput, CallbackStats, MidiConnection, RunningAudio, SharedMidi};
use loom_patch::Patch;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Parser, Debug, Clone)]
#[command(name = "loom-app", version, about = "LoomDSP desktop app")]
struct Args {
    /// Patch file (.ron) to open.
    patch: Option<PathBuf>,
    /// Instead of a file, generate a test patch with about this many modules.
    #[arg(long, value_name = "N")]
    generate: Option<usize>,
    /// Do not open an audio device.
    #[arg(long)]
    no_audio: bool,
    /// Start measuring frame rate with the camera moving (performance gate).
    #[arg(long)]
    bench: bool,
}

fn main() -> iced::Result {
    let args = Args::parse();
    iced::application(move || App::new(args.clone()), App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .exit_on_close_request(false)
        .window_size((1440.0, 900.0))
        .antialiasing(false)
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    Canvas(CanvasEvent),
    /// SPIKE #186: the pointer entered or left an inspector parameter row.
    RowEnter(u32, usize),
    RowExit(u32, usize),
    Param {
        module: u32,
        index: usize,
        position: f32,
    },
    /// The toolbar asks for the module browser at the centre of the canvas.
    OpenBrowser,
    /// A message for the open module browser.
    Browser(browser::Message),
    HoldNote,
    /// File commands (`docs/design/files.md`).
    New,
    Open,
    Save,
    SaveAs,
    /// The window asked to close.
    CloseRequested,
    Measure(bool),
    Animate(bool),
    Frame(Instant),
    Tick,
    /// New brightness for lights on screen.
    Lights(Vec<(lights::Light, f32)>),
    /// New levels for meter probes.
    Meters(Vec<(ProbeId, live::Level)>),
    /// Turn MIDI learn mode on or off.
    ToggleLearn,
    /// In learn mode, arm a parameter from the inspector.
    LearnParam {
        module: u32,
        index: usize,
    },
    /// Remove a parameter's MIDI mapping.
    Unmap {
        module: u32,
        index: usize,
    },
    /// A MIDI control change arrived.
    Midi,
    /// A key: the undo and redo shortcuts, and Escape in learn mode. Ignored
    /// while the module browser is open.
    Key(iced::keyboard::Event),
    /// Take back the last step (`docs/design/undo.md`).
    Undo,
    /// Replay the step undo took back.
    Redo,
    /// An inspector slider was let go.
    SliderReleased,
}

struct App {
    controller: Controller,
    positions: BTreeMap<u32, Point>,
    scene: Scene,
    /// Live values of panel controls and lights, apart from the scene so a
    /// knob turn does not rebuild tiles or cables.
    values: PanelValues,
    /// Last brightness of each light, by module and widget.
    lights: BTreeMap<lights::Light, f32>,
    /// Latest level of each meter probe.
    meter_levels: std::collections::HashMap<ProbeId, live::Level>,
    /// Modules whose lights or meters are on screen, as the canvas reports.
    visible_displays: Vec<u32>,
    selection: BTreeSet<u32>,
    /// The name saved inside the patch.
    name: String,
    /// The file the patch was opened from or saved to (`docs/design/files.md`).
    file: Option<PathBuf>,
    /// Counts the engines built. Background watchers restart when it changes.
    generation: u64,
    sample_rate: f32,
    no_audio: bool,
    /// Set when the app should exit after this update.
    quit: bool,
    theme: Theme,
    _audio: Option<RunningAudio>,
    /// Meter and scope data from the engine, drained on the telemetry
    /// thread so the queues never fill.
    monitor: monitor::Monitor,
    live: Live,
    /// When the dropout count last grew, for the stats bar warning.
    dropouts: alerts::Dropouts,
    /// Open MIDI inputs. Input stops when they are dropped.
    midi_ports: Vec<MidiConnection>,
    audio_stats: Option<Arc<CallbackStats>>,
    audio_name: String,
    /// Note input shared by every MIDI port and the "Hold note" button.
    midi: Option<SharedMidi>,
    holding: bool,
    status: String,
    /// Every registered module, for the browser.
    modules: Vec<&'static ModuleDescriptor>,
    /// The module browser, while it is open.
    browser: Option<browser::Browser>,
    /// Changed by the toolbar button; the canvas then opens the browser at
    /// its centre.
    browser_key: u64,
    frame_key: u64,
    meter: FrameMeter,
    measuring: bool,
    animating: bool,
    /// MIDI learn mode (`docs/design/midi-learn.md`).
    learning: bool,
    /// The parameter the next control change maps to.
    armed: Option<learn::Armed>,
    /// What learn mode shows on the canvas.
    learn_view: LearnView,
    /// Undo and redo (`docs/design/undo.md`).
    history: undo::History,
    /// The patch before the edit in progress, if there is one.
    pending: Option<undo::Snapshot>,
    /// True between a gesture's start and end. Undo waits for it.
    gesture: bool,
    /// True when the open gesture began on an inspector slider. A slider
    /// sends no start, and arrow keys on it send no end, so the next edit
    /// or an undo closes it too.
    slider_gesture: bool,
    /// True after a commit failed, until the graph is sent. The graph holds a
    /// change the engine does not, so no step is recorded: undo would send
    /// that change along with a later one.
    history_off: bool,
    /// Parameters this app set during the edit in progress. A change on a
    /// mapped parameter is part of the step only if it is listed here.
    touched: BTreeSet<(u32, usize)>,
    /// SPIKE #186: the output being dragged, its port, and the pointer, in
    /// window coordinates.
    assign_drag: Option<(loom_canvas::PortRef, Point, Option<Point>)>,
    /// SPIKE #186: the inspector row under the pointer, from mouse_area.
    drop_row: Option<(u32, usize)>,
}

/// Where a patch comes from.
enum Source<'a> {
    Empty,
    Generated(usize),
    File(&'a Path),
}

/// An engine and the patch loaded into it, before audio is attached.
struct Session {
    controller: Controller,
    engine: loom_core::Engine,
    midi: loom_core::MidiSender,
    telemetry: loom_core::Telemetry,
    positions: BTreeMap<u32, Point>,
    name: String,
    file: Option<PathBuf>,
}

impl App {
    fn new(args: Args) -> Self {
        let output = Self::open_output(args.no_audio);
        let sample_rate = output.as_ref().map(|o| o.sample_rate() as f32).unwrap_or(48_000.0);
        let source = match (&args.patch, args.generate) {
            (_, Some(n)) => Source::Generated(n),
            (Some(path), None) => Source::File(path),
            (None, None) => Source::Empty,
        };
        let mut status = String::new();
        let session = Self::session(sample_rate, source).unwrap_or_else(|e| {
            status = e;
            Self::session(sample_rate, Source::Empty).expect("an empty patch always loads")
        });
        let mut app = Self::assemble(output, sample_rate, session, 1, &args);
        if !status.is_empty() {
            app.status = status;
        }
        app
    }

    fn open_output(no_audio: bool) -> Result<AudioOutput, String> {
        if no_audio {
            Err("audio off".to_string())
        } else {
            AudioOutput::open(&AudioOptions::default()).map_err(|e| e.to_string())
        }
    }

    /// Builds an engine and loads a patch into it. On failure nothing else
    /// is touched, so a failed open leaves the running patch alone.
    fn session(sample_rate: f32, source: Source) -> Result<Session, String> {
        let EngineParts {
            mut controller,
            engine,
            midi,
            telemetry,
        } = create_engine(loom_modules::registry(), EngineConfig::with_sample_rate(sample_rate));
        let mut positions = BTreeMap::new();
        let (name, file) = match source {
            Source::Generated(n) => {
                layout::generate(&mut controller, n).map_err(|e| format!("generate failed: {e}"))?;
                layout::generated_layout(&controller, &mut positions);
                (format!("Generated ({n} modules)"), None)
            }
            Source::File(path) => {
                let patch = Patch::load(path).map_err(|e| e.to_string())?;
                patch.load_into(&mut controller).map_err(|e| e.to_string())?;
                for m in &patch.modules {
                    if let Some((x, y)) = m.position {
                        positions.insert(m.id, Point::new(x, y));
                    }
                }
                (patch.name.clone(), Some(path.to_path_buf()))
            }
            Source::Empty => ("Untitled".to_string(), None),
        };
        layout::auto_layout(&controller, &mut positions);
        Ok(Session {
            controller,
            engine,
            midi,
            telemetry,
            positions,
            name,
            file,
        })
    }

    /// Attaches audio and MIDI to a session and builds the app around it.
    fn assemble(
        output: Result<AudioOutput, String>,
        sample_rate: f32,
        session: Session,
        generation: u64,
        args: &Args,
    ) -> Self {
        let Session {
            controller,
            engine,
            midi: sender,
            telemetry,
            positions,
            name,
            file,
        } = session;
        let mut status = String::new();

        // Every MIDI input plays the patch, and the "Hold note" button works
        // alongside them, so a keyboard is never hidden behind another port.
        let midi = loom_host_desktop::share_midi(sender);
        let midi_ports = match loom_host_desktop::connect_all_midi(&midi) {
            Ok((ports, errors)) => {
                if let Some(error) = errors.first() {
                    status = format!("MIDI: {error}");
                }
                ports
            }
            Err(e) => {
                status = format!("MIDI: {e}");
                Vec::new()
            }
        };

        let (audio, midi, audio_name) = match output {
            Ok(output) => {
                let label = format!("{} at {} Hz", output.name(), output.sample_rate());
                match output.start(engine) {
                    Ok(audio) => (Some(audio), Some(midi), label),
                    Err(e) => (None, None, format!("no audio ({e})")),
                }
            }
            Err(e) => (None, None, format!("no audio ({e})")),
        };

        let scene = scene::build(&controller, &positions);
        let mut values = PanelValues::new();
        scene::update_panel_values(&mut values, &controller, &BTreeMap::new());
        let mut app = Self {
            assign_drag: None,
            drop_row: None,
            controller,
            positions,
            scene,
            values,
            lights: BTreeMap::new(),
            meter_levels: std::collections::HashMap::new(),
            visible_displays: Vec::new(),
            selection: BTreeSet::new(),
            name,
            file,
            generation,
            sample_rate,
            no_audio: args.no_audio,
            quit: false,
            theme: Theme::custom(
                "Loom",
                iced::theme::Palette {
                    background: Color::from_rgb8(0x22, 0x24, 0x2A),
                    text: Color::from_rgb8(0xE6, 0xE8, 0xEE),
                    primary: Color::from_rgb8(0xF5, 0xA6, 0x23),
                    success: Color::from_rgb8(0x4C, 0xB0, 0x6E),
                    warning: Color::from_rgb8(0xE8, 0xD2, 0x4A),
                    danger: Color::from_rgb8(0xE0, 0x4F, 0x4F),
                },
            ),
            audio_stats: audio.as_ref().map(|a| a.stats().clone()),
            _audio: audio,
            monitor: monitor::Monitor::new(telemetry, generation),
            live: Live::new(),
            dropouts: alerts::Dropouts::default(),
            midi_ports,
            audio_name,
            midi,
            holding: false,
            status,
            modules: loom_modules::registry().iter().collect(),
            browser: None,
            browser_key: 0,
            frame_key: 0,
            meter: FrameMeter::default(),
            measuring: args.bench,
            animating: args.bench,
            learning: false,
            armed: None,
            learn_view: LearnView::default(),
            history: undo::History::default(),
            pending: None,
            gesture: false,
            slider_gesture: false,
            history_off: false,
            touched: BTreeSet::new(),
        };
        // A patch just loaded is the one on disk.
        app.history.mark_saved();
        if app.status.is_empty() {
            app.report();
        }
        app
    }

    /// True when the patch differs from the file (`docs/design/files.md`).
    fn unsaved(&self) -> bool {
        !self.history.is_saved()
    }

    /// Writes the patch to `path`. On success the path becomes the current
    /// file and the unsaved mark clears.
    fn write_to(&mut self, path: PathBuf) -> bool {
        let mut patch = Patch::from_graph(&self.name, self.controller.graph());
        for m in &mut patch.modules {
            m.position = self.positions.get(&m.id).map(|p| (p.x.round(), p.y.round()));
        }
        match patch.save(&path) {
            Ok(()) => {
                self.status = format!("saved {}", path.display());
                self.file = Some(path);
                self.history.mark_saved();
                true
            }
            Err(e) => {
                let message = format!(
                    "Could not save \"{}\": {e}. Your changes are not saved.",
                    files::display_name(Some(&path))
                );
                files::show_error(&message);
                self.status = message;
                false
            }
        }
    }

    /// Save: to the current file, or Save As when there is none. Returns
    /// false if nothing was saved.
    fn save(&mut self) -> bool {
        self.finish_edits();
        match self.file.clone() {
            Some(path) => self.write_to(path),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) -> bool {
        self.finish_edits();
        match files::pick_save(self.file.as_deref()) {
            Some(path) => self.write_to(path),
            None => false,
        }
    }

    /// Ends any edit in progress, so the history and the saved mark see it.
    /// A canvas drag stays open: the rest of it becomes a step of its own
    /// when it ends.
    fn finish_edits(&mut self) {
        self.close_slider_gesture();
        if self.gesture {
            self.record();
        }
    }

    /// Asks what to do with unsaved changes. Returns true if the caller may
    /// go on and drop them.
    fn can_discard(&mut self) -> bool {
        self.finish_edits();
        if !self.unsaved() {
            return true;
        }
        match files::confirm_discard(&files::display_name(self.file.as_deref())) {
            files::Choice::Save => self.save(),
            files::Choice::DontSave => true,
            files::Choice::Cancel => false,
            files::Choice::Unavailable => {
                self.status =
                    "Unsaved changes. No dialog program (zenity or kdialog) is installed: save with Ctrl+S first."
                        .to_string();
                false
            }
        }
    }

    /// Replaces the patch (and its engine) with `source`. If the source does
    /// not load, the running patch is unchanged.
    fn replace(&mut self, source: Source) -> bool {
        // Choosing the device does not start a stream, so the old patch keeps
        // playing. The engine is built at the rate of the device that will
        // play it.
        let output = Self::open_output(self.no_audio);
        // Playing now and no device to play on afterwards: keep what plays.
        if let (Err(e), true) = (&output, self._audio.is_some()) {
            let message = format!(
                "Could not open the patch: the audio device is not available ({e}). The current patch is unchanged."
            );
            files::show_error(&message);
            self.status = message;
            return false;
        }
        let sample_rate = output
            .as_ref()
            .map(|o| o.sample_rate() as f32)
            .unwrap_or(self.sample_rate);
        let session = match Self::session(sample_rate, source) {
            Ok(session) => session,
            Err(e) => {
                let message = format!("Could not open the patch: {e}. The current patch is unchanged.");
                files::show_error(&message);
                self.status = message;
                return false;
            }
        };
        // Nothing can fail from here: free the audio device and MIDI inputs
        // before the new engine takes them.
        self._audio = None;
        self.audio_stats = None;
        self.midi = None;
        self.midi_ports.clear();
        let args = Args {
            patch: None,
            generate: None,
            no_audio: self.no_audio,
            bench: self.measuring,
        };
        let mut fresh = Self::assemble(output, sample_rate, session, self.generation + 1, &args);
        fresh.animating = self.animating;
        // The canvas keeps its state across the rebuild and compares these
        // keys with the ones it saw: they must keep counting up. A new frame
        // key also resets pan and zoom to show the whole patch.
        fresh.browser_key = self.browser_key;
        fresh.frame_key = self.frame_key + 1;
        *self = fresh;
        true
    }

    fn new_patch(&mut self) {
        if self.can_discard() && self.replace(Source::Empty) {
            self.status = "new patch".to_string();
        }
    }

    fn open(&mut self) {
        if !self.can_discard() {
            return;
        }
        let Some(path) = files::pick_open(self.file.as_deref()) else {
            return;
        };
        if self.replace(Source::File(&path)) {
            self.status = format!("opened {}", path.display());
        }
    }

    fn title(&self) -> String {
        files::window_title(self.file.as_deref(), self.unsaved())
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn subscription(&self) -> Subscription<Message> {
        let main = if self.measuring {
            // Redraw every frame, so the counter measures real render cost.
            iced::window::frames().map(Message::Frame)
        } else {
            // Otherwise the window redraws only when something changes. A
            // slow tick keeps the audio numbers fresh.
            iced::time::every(Duration::from_millis(500)).map(|_| Message::Tick)
        };
        let telemetry = Subscription::run_with(self.monitor.clone(), monitor::stream).map(Message::Meters);
        let close = iced::window::close_requests().map(|_| Message::CloseRequested);
        Subscription::batch([main, self.light_watch(), telemetry, self.midi_watch(), close])
    }

    /// Wakes the UI when control changes arrive, while there is something to
    /// update: mapped parameters to follow, or learn mode waiting for MIDI.
    /// In learn mode, it also listens for Escape.
    fn midi_watch(&self) -> Subscription<Message> {
        let mapped = self.controller.graph().midi_mappings().next().is_some();
        let watch = (mapped || self.learning).then(|| {
            let watch = learn::Watch {
                reader: self.controller.midi_reader(),
                generation: self.generation,
            };
            Subscription::run_with(watch, learn::stream).map(|_| Message::Midi)
        });
        let keys = Some(iced::keyboard::listen().map(Message::Key));
        // The search field takes Escape for itself, so the browser listens
        // to keys whether or not a widget used them.
        let browser = self.browser.is_some().then(|| iced::event::listen_with(browser_key));
        Subscription::batch(watch.into_iter().chain(keys).chain(browser))
    }

    /// Watches the lights on screen, and sends a message only when one
    /// visibly changes. No lights on screen: no watcher.
    fn light_watch(&self) -> Subscription<Message> {
        let graph = self.controller.graph();
        let watched: Vec<_> = self
            .visible_displays
            .iter()
            .filter_map(|&id| Some((id, graph.node(ModuleId(id))?.descriptor)))
            .flat_map(|(id, d)| {
                d.panel.widgets.iter().enumerate().filter_map(move |(index, w)| {
                    let display = (!w.is_control()).then(|| d.display_index(w.bind))??;
                    Some(((id, index), display))
                })
            })
            .filter_map(|(light, display)| {
                let source = self.controller.display_source(ModuleId(light.0), display)?;
                Some((light, source))
            })
            .collect();
        if watched.is_empty() {
            return Subscription::none();
        }
        let watch = lights::Watch {
            reader: self.controller.display_reader(),
            lights: watched,
            generation: self.generation,
        };
        Subscription::run_with(watch, lights::stream).map(Message::Lights)
    }

    /// Rebuilds the scene after the graph changed.
    fn refresh(&mut self) {
        layout::auto_layout(&self.controller, &mut self.positions);
        self.scene = scene::build(&self.controller, &self.positions);
        self.refresh_values();
        self.refresh_learn();
    }

    /// Rebuilds what learn mode shows, after mappings or the armed
    /// parameter changed. A removed module disarms its parameter.
    fn refresh_learn(&mut self) {
        if let Some(armed) = self.armed
            && self.controller.graph().node(ModuleId(armed.param.0)).is_none()
        {
            self.armed = None;
        }
        self.learn_view = learn::view(&self.controller, self.armed);
    }

    /// Arms a parameter in learn mode, or disarms it if it is armed.
    fn arm(&mut self, param: learn::Param) {
        if self.armed.is_some_and(|armed| armed.param == param) {
            self.armed = None;
            self.status = "MIDI learn: click a control to map it".to_string();
        } else {
            self.armed = Some(learn::Armed {
                param,
                serial: self.controller.last_control_change().map(|seen| seen.serial),
            });
            self.status = format!(
                "Move a MIDI control to map it to {}",
                learn::param_name(&self.controller, param)
            );
        }
        self.refresh_learn();
    }

    /// Follows control changes: mapped parameters on screen, and the armed
    /// parameter in learn mode.
    fn midi_arrived(&mut self) {
        if !self.controller.sync_midi().is_empty() {
            self.refresh_values();
        }
        let Some(armed) = self.armed else { return };
        let Some(seen) = self.controller.last_control_change() else {
            return;
        };
        if Some(seen.serial) == armed.serial {
            return;
        }
        let (module, index) = armed.param;
        self.begin_edit();
        self.touched.insert((module, index));
        let learned = self.controller.learn_midi(seen, ModuleId(module), index);
        match learned {
            Ok((id, removed)) => {
                let mapping = self
                    .controller
                    .graph()
                    .midi_mappings()
                    .find(|(i, _)| *i == id)
                    .map(|(_, m)| *m);
                self.armed = None;
                self.commit();
                if let Some(mapping) = mapping {
                    self.status = learn::mapped_status(&self.controller, mapping, &removed);
                }
            }
            Err(e) => {
                self.armed = None;
                self.status = format!("error: {e}");
                self.refresh_learn();
            }
        }
        if !self.gesture {
            self.record();
        }
    }

    fn stop_learning(&mut self) {
        self.learning = false;
        self.armed = None;
        self.status = "MIDI learn off".to_string();
        self.refresh_learn();
    }

    /// Rebuilds the control, light, and meter values. Only changed values
    /// change what the canvas uploads.
    fn refresh_values(&mut self) {
        let mut shown: BTreeMap<lights::Light, (f32, f32)> = self
            .lights
            .iter()
            .map(|(&light, &level)| (light, (level, 0.0)))
            .collect();
        let modules: Vec<u32> = self.controller.graph().nodes().map(|(id, _)| id.0).collect();
        for (widget, output) in scene::meter_outputs(&self.controller, &modules) {
            let level = self
                .live
                .probe(output)
                .and_then(|probe| self.meter_levels.get(&probe))
                .copied()
                .unwrap_or_default();
            shown.insert(widget, (level.rms, level.peak));
        }
        scene::update_panel_values(&mut self.values, &self.controller, &shown);
    }

    /// Takes the latest dropout count for the stats bar warning. Runs on
    /// every tick and, in Measure mode, on every frame.
    fn update_dropouts(&mut self, now: Instant) {
        if let Some(stats) = &self.audio_stats {
            let s = stats.snapshot();
            self.dropouts.update(s.overruns + s.stream_errors, now);
        }
    }

    /// The outputs of the meters on screen: each needs a probe.
    fn metered_outputs(&self) -> BTreeSet<OutputRef> {
        scene::meter_outputs(&self.controller, &self.visible_displays)
            .into_iter()
            .map(|(_, output)| output)
            .collect()
    }

    /// Adds probes for meters that came on screen and removes those that
    /// left. Only probes change, so the scene is not rebuilt.
    fn sync_probes(&mut self) {
        let wanted = self.metered_outputs();
        self.live.sync(&mut self.controller, &wanted);
        if let Err(e) = self.controller.commit() {
            self.status = format!("error: {e}");
        }
    }

    /// Sets a parameter from a control on a tile or in the inspector.
    ///
    /// A switch to momentary also clears that button's gate. Both writes go
    /// in one step, so a full command queue leaves the mode and the gate as
    /// they were.
    fn set_param(&mut self, module: u32, index: usize, value: f32) {
        let changes = self
            .controller
            .graph()
            .node(ModuleId(module))
            .map(|node| scene::param_edits(node, index, value))
            .unwrap_or_else(|| vec![(index, value)]);
        if let Err(e) = self.controller.set_params(ModuleId(module), &changes) {
            self.status = format!("error: {e}");
        }
        self.touched.extend(changes.iter().map(|&(i, _)| (module, i)));
        self.refresh_values();
    }

    /// Applies a button gesture. The canvas names the gesture. [`loom_core::ButtonMode`]
    /// names the value, from the mode parameter stored on the module.
    fn apply_button(&mut self, module: u32, widget: usize, change: impl Fn(loom_core::ButtonMode, f32) -> Option<f32>) {
        let edit = self.controller.graph().node(ModuleId(module)).and_then(|node| {
            let panel_widget = node.descriptor.panel.widgets.get(widget)?;
            let loom_core::WidgetKind::Button { mode } = panel_widget.kind else {
                return None;
            };
            let mode_index = node.descriptor.param_index(mode)?;
            let on_index = node.descriptor.param_index(panel_widget.bind)?;
            let next = change(
                loom_core::ButtonMode::from_value(node.params[mode_index]),
                node.params[on_index],
            )?;
            Some((on_index, next))
        });
        if let Some((index, value)) = edit {
            self.set_param(module, index, value);
        }
    }

    fn report(&mut self) {
        let report = self.controller.last_report();
        let inserted = report.inserted_delays().count();
        self.status = match report.loops.len() {
            0 => "no feedback loops".to_string(),
            n => format!(
                "{n} feedback {}, {inserted} automatic 1-sample {} (dashed)",
                if n == 1 { "loop" } else { "loops" },
                if inserted == 1 { "delay" } else { "delays" },
            ),
        };
    }

    /// Starts recording an edit: keeps the patch as it is now. Does nothing
    /// if one is already open, so a gesture's first call is the one that
    /// counts.
    fn begin_edit(&mut self) {
        // Another path (probe sync) may have sent the graph since the failure.
        if !self.gesture {
            self.history_back_on();
        }
        if self.pending.is_none() && !self.history_off {
            self.pending = Some(undo::Snapshot::take(&self.controller, &self.positions));
            self.touched.clear();
        }
    }

    /// Ends the recording and adds a step if the patch changed.
    fn record(&mut self) {
        let Some(before) = self.pending.take() else {
            return;
        };
        let after = undo::Snapshot::take(&self.controller, &self.positions);
        if let Some(step) = undo::diff(&before, &after, &self.touched) {
            self.history.push(step);
        }
        self.touched.clear();
    }

    /// Runs one edit as one undo step, unless a gesture is open: then the
    /// gesture's end makes the step (undo.md, rule 1 and 2).
    fn edit(&mut self, change: impl FnOnce(&mut Self)) {
        self.begin_edit();
        change(self);
        if !self.gesture {
            self.record();
        }
    }

    /// Closes a gesture that began on an inspector slider.
    fn close_slider_gesture(&mut self) {
        if self.slider_gesture {
            self.slider_gesture = false;
            self.gesture = false;
            self.record();
            self.history_back_on();
        }
    }

    /// Records steps again once the engine has the whole graph.
    fn history_back_on(&mut self) {
        if !self.controller.has_uncommitted_changes() {
            self.history_off = false;
        }
    }

    fn undo(&mut self) {
        self.close_slider_gesture();
        if !self.gesture {
            self.step(false);
        }
    }

    fn redo(&mut self) {
        self.close_slider_gesture();
        if !self.gesture {
            self.step(true);
        }
    }

    /// Undoes (`forward` false) or redoes the next step.
    fn step(&mut self, forward: bool) {
        let step = if forward {
            self.history.redo_step()
        } else {
            self.history.undo_step()
        };
        let Some(step) = step.cloned() else { return };
        let verb = if forward { "Redo" } else { "Undo" };
        if let Err(why) = undo::check(&step, forward, &self.controller) {
            self.status = format!("Cannot {} {}: {why}", verb.to_lowercase(), step.name());
            return;
        }
        // An armed parameter is a mode, not an edit: undo disarms it first.
        self.armed = None;
        let applied = undo::apply(&step, forward, &mut self.controller, &mut self.positions);
        match applied {
            Ok(()) => {
                if forward {
                    self.history.redone();
                } else {
                    self.history.undone();
                }
                // What the step touched, if it still exists, is selected.
                let touched: BTreeSet<u32> = step
                    .touched_modules()
                    .filter(|&id| self.controller.graph().node(ModuleId(id)).is_some())
                    .collect();
                if !touched.is_empty() {
                    self.selection = touched;
                }
                self.selection
                    .retain(|&id| self.controller.graph().node(ModuleId(id)).is_some());
                if self.commit() {
                    self.status = format!("{verb}: {}", step.name());
                } else {
                    // The graph has the change but the engine does not. The
                    // history is already cleared by `commit`.
                    self.status = format!(
                        "{verb} could not reach the engine ({}); undo history cleared",
                        self.status
                    );
                }
            }
            Err(e) => {
                self.history.clear();
                self.commit();
                self.status = format!("{verb} failed ({e}); undo history cleared");
            }
        }
    }

    /// Sends the graph to the engine. Returns false if that failed; the
    /// status bar then holds the error.
    fn commit(&mut self) -> bool {
        // Probes follow the graph: a removed module loses its probes.
        let wanted = self.metered_outputs();
        self.live.sync(&mut self.controller, &wanted);
        let sent = match self.controller.commit() {
            Ok(_) => {
                self.report();
                // Inside a gesture, wait for its end: the rest of it was not
                // recorded, so it stays out of the history as a whole.
                if !self.gesture {
                    self.history_off = false;
                }
                true
            }
            Err(e) => {
                self.status = format!("error: {e}");
                // The graph holds a change the engine does not. Recording it
                // would put a step in the history for something that does not
                // play. The controller stays dirty, so the next commit
                // retries the change; until then the history cannot be
                // trusted.
                self.pending = None;
                self.touched.clear();
                self.history.clear();
                self.history_off = true;
                false
            }
        };
        self.refresh();
        sent
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let was_open = self.browser.is_some();
        let task = match message {
            Message::Browser(message) => self.browser_message(message),
            message => {
                self.handle(message);
                Task::none()
            }
        };
        if self.quit {
            Task::batch([task, iced::exit()])
        } else if !was_open && self.browser.is_some() {
            Task::batch([task, browser::Browser::focus()])
        } else {
            task
        }
    }

    /// Handles a browser message: keep it open, close it, or add a module.
    fn browser_message(&mut self, message: browser::Message) -> Task<Message> {
        let Some(open) = self.browser.as_mut() else {
            return Task::none();
        };
        let (outcome, task) = open.update(message);
        match outcome {
            browser::Outcome::Stay => {}
            browser::Outcome::Close => self.browser = None,
            browser::Outcome::Add(descriptor, world) => {
                self.browser = None;
                self.add_module(descriptor, world);
            }
        }
        task.map(Message::Browser)
    }

    /// Adds a module with its top-left corner at a world point and selects
    /// it, as one undo step.
    fn add_module(&mut self, descriptor: &'static ModuleDescriptor, world: Point) {
        self.close_slider_gesture();
        self.edit(|app| match app.controller.add_module(descriptor.id) {
            Ok(id) => {
                app.positions.insert(id.0, world);
                app.selection = BTreeSet::from([id.0]);
                app.commit();
            }
            Err(e) => app.status = format!("Could not add {}: {e}", descriptor.name),
        });
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Browser(_) => {}
            Message::Key(_) if self.browser.is_some() => {}
            Message::Canvas(event) => self.canvas_event(event),
            Message::RowEnter(module, index) => {
                eprintln!(
                    "SPIKE app: row enter #{module}:{index}, dragging {}",
                    self.assign_drag.is_some()
                );
                self.drop_row = Some((module, index));
            }
            Message::RowExit(module, index) => {
                eprintln!(
                    "SPIKE app: row exit #{module}:{index}, dragging {}",
                    self.assign_drag.is_some()
                );
                if self.drop_row == Some((module, index)) {
                    self.drop_row = None;
                }
            }
            Message::Param {
                module,
                index,
                position,
            } => {
                if let Some(spec) = self
                    .controller
                    .graph()
                    .node(ModuleId(module))
                    .and_then(|n| n.descriptor.params.get(index))
                {
                    let value = spec.denormalize(position);
                    // A slider sends no start. Its first change opens the
                    // gesture, and letting go closes it.
                    if !self.gesture {
                        self.gesture = true;
                        self.slider_gesture = true;
                    }
                    self.begin_edit();
                    self.set_param(module, index, value);
                }
            }
            Message::SliderReleased => self.close_slider_gesture(),
            Message::Undo => self.undo(),
            Message::Redo => self.redo(),
            Message::OpenBrowser => self.browser_key += 1,
            Message::HoldNote => {
                if let Some(midi) = &self.midi {
                    self.holding = !self.holding;
                    loom_host_desktop::send_midi(
                        midi,
                        if self.holding {
                            MidiEvent::note_on(0, 0, 57, 100)
                        } else {
                            MidiEvent::note_off(0, 0, 57)
                        },
                    );
                }
            }
            Message::New => self.new_patch(),
            Message::Open => self.open(),
            Message::Save => {
                self.save();
            }
            Message::SaveAs => {
                self.save_as();
            }
            Message::CloseRequested => {
                if self.can_discard() {
                    self.quit = true;
                }
            }
            Message::Measure(on) => {
                self.measuring = on;
                self.meter = FrameMeter::default();
                if !on {
                    self.animating = false;
                }
            }
            Message::Animate(on) => {
                self.animating = on;
                if on {
                    self.measuring = true;
                }
                self.meter = FrameMeter::default();
            }
            Message::Frame(now) => {
                self.meter.record(now);
                self.controller.collect_garbage();
                self.update_dropouts(now);
            }
            Message::Tick => {
                self.controller.collect_garbage();
                self.update_dropouts(Instant::now());
            }
            Message::Meters(levels) => {
                self.meter_levels.extend(levels);
                self.refresh_values();
            }
            Message::Lights(readings) => {
                self.lights.extend(readings);
                self.refresh_values();
            }
            Message::ToggleLearn => {
                if self.learning {
                    self.stop_learning();
                } else {
                    self.learning = true;
                    self.status = "MIDI learn: click a control to map it".to_string();
                    self.refresh_learn();
                }
            }
            Message::LearnParam { module, index } => {
                if self.learning {
                    self.arm((module, index));
                }
            }
            Message::Unmap { module, index } => {
                self.close_slider_gesture();
                self.edit(|app| match app.controller.unmap_midi(ModuleId(module), index) {
                    Ok(Some(mapping)) => {
                        app.commit();
                        app.status = format!(
                            "Removed {} from {}",
                            mapping.control,
                            learn::param_name(&app.controller, (module, index))
                        );
                    }
                    Ok(None) => {}
                    Err(e) => app.status = format!("error: {e}"),
                });
            }
            Message::Midi => self.midi_arrived(),
            Message::Key(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            }) if self.learning => self.stop_learning(),
            Message::Key(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) if modifiers.command() => {
                // Ctrl+Z or Cmd+Z undoes; adding Shift redoes. Ctrl+Y also
                // redoes, except on macOS.
                match key.as_ref() {
                    iced::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("z") && modifiers.shift() => {
                        self.redo()
                    }
                    iced::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("z") => self.undo(),
                    iced::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("n") => self.new_patch(),
                    iced::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("o") => self.open(),
                    iced::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("s") && modifiers.shift() => {
                        self.save_as();
                    }
                    iced::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("s") => {
                        self.save();
                    }
                    iced::keyboard::Key::Character(c) if c.eq_ignore_ascii_case("y") && !cfg!(target_os = "macos") => {
                        self.redo()
                    }
                    _ => {}
                }
            }
            Message::Key(_) => {}
        }
    }

    fn canvas_event(&mut self, event: CanvasEvent) {
        let edits = undo::is_edit(&self.controller, &event);
        if edits {
            self.close_slider_gesture();
            self.edit(|app| app.apply_canvas_event(event));
        } else {
            self.apply_canvas_event(event);
        }
    }

    fn apply_canvas_event(&mut self, event: CanvasEvent) {
        match event {
            CanvasEvent::AssignDrag { from, start, at } => {
                if self.assign_drag.is_none() {
                    eprintln!("SPIKE app: drag start from {from:?}");
                }
                if at.is_none() {
                    eprintln!("SPIKE app: drag move with no cursor position");
                }
                self.assign_drag = Some((from, start, at));
            }
            CanvasEvent::AssignDrop { from, at } => {
                match self.drop_row {
                    Some((module, index)) => {
                        eprintln!("SPIKE app: DROP {from:?} on #{module} param {index} at {at:?}");
                        self.status = format!(
                            "Spike: would assign output {} to #{module} parameter {index}",
                            from.index
                        );
                    }
                    None => eprintln!("SPIKE app: drop on nothing at {at:?}"),
                }
                self.assign_drag = None;
            }
            CanvasEvent::SpikeView { .. } => {}
            CanvasEvent::AssignCancel => {
                eprintln!("SPIKE app: cancel");
                self.assign_drag = None;
            }
            CanvasEvent::GestureStart => {
                self.close_slider_gesture();
                self.gesture = true;
                self.begin_edit();
            }
            CanvasEvent::GestureEnd => {
                self.gesture = false;
                self.record();
                self.history_back_on();
            }
            CanvasEvent::Select(ids) => self.selection = ids.into_iter().collect(),
            CanvasEvent::Move { ids, delta } => {
                for id in &ids {
                    if let Some(p) = self.positions.get_mut(id) {
                        *p += delta;
                    }
                }
                self.scene.move_modules(&ids, delta);
            }
            CanvasEvent::Connect { from, to } => {
                let output = OutputRef {
                    module: ModuleId(from.module),
                    port: from.index,
                };
                let input = InputRef {
                    module: ModuleId(to.module),
                    port: to.index,
                };
                match self.controller.connect(output, input) {
                    Ok(_) => {
                        self.commit();
                    }
                    Err(e) => self.status = format!("error: {e}"),
                }
            }
            CanvasEvent::Disconnect(port) => {
                let input = InputRef {
                    module: ModuleId(port.module),
                    port: port.index,
                };
                if self.controller.disconnect(input).is_ok() {
                    self.commit();
                }
            }
            CanvasEvent::SetControl {
                module,
                widget,
                position,
            } => {
                if let Some((index, kind, spec)) = scene::control_param(&self.controller, module, widget) {
                    let value = scene::control_value(kind, &spec, position);
                    self.set_param(module, index, value);
                }
            }
            CanvasEvent::ButtonPressed { module, widget } => {
                self.apply_button(module, widget, loom_core::ButtonMode::press);
            }
            CanvasEvent::ButtonReleased { module, widget } => {
                self.apply_button(module, widget, |mode, _current| mode.release());
            }
            CanvasEvent::ButtonArrow { module, widget, high } => {
                self.apply_button(module, widget, |mode, current| mode.arrow(current, high));
            }
            CanvasEvent::ResetControl { module, widget } => {
                if let Some((index, _, spec)) = scene::control_param(&self.controller, module, widget) {
                    self.set_param(module, index, spec.default);
                }
            }
            CanvasEvent::Learn { module, widget } => {
                if let Some((index, _, _)) = scene::control_param(&self.controller, module, widget) {
                    self.arm((module, index));
                }
            }
            CanvasEvent::OpenBrowser { world, at, area } => {
                self.close_slider_gesture();
                self.browser = Some(browser::Browser::open(self.modules.clone(), world, at, area));
            }
            CanvasEvent::VisibleDisplays(ids) => {
                self.visible_displays = ids;
                self.sync_probes();
                self.refresh_values();
            }
            CanvasEvent::ToggleRun => {
                let running = !self.controller.transport().running;
                if let Err(e) = self.controller.set_running(running) {
                    self.status = format!("error: {e}");
                }
            }
            CanvasEvent::Delete(ids) => {
                for id in &ids {
                    if self.controller.remove_module(ModuleId(*id)).is_ok() {
                        self.positions.remove(id);
                        self.selection.remove(id);
                    }
                }
                self.commit();
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let canvas = PatchCanvas::new(&self.scene, &self.selection, Message::Canvas)
            .values(&self.values)
            .frame_key(self.frame_key)
            .browser_key(self.browser_key)
            .interactive(self.browser.is_none())
            .animate(self.animating)
            .learn(self.learning.then_some(&self.learn_view));

        let body = row![
            container(canvas).width(Length::Fill).height(Length::Fill),
            rule::vertical(1),
            self.inspector(),
        ];

        let main = column![
            self.toolbar(),
            rule::horizontal(1),
            body,
            rule::horizontal(1),
            self.stats_bar()
        ];
        // Always a stack, so opening the browser keeps the canvas's state
        // (its camera) in the same place in the widget tree.
        let mut layers = stack![main];
        if let Some((_, start, Some(end))) = self.assign_drag {
            layers = layers.push(spike_line::DragLine { start, end });
        }
        if let Some(open) = &self.browser {
            let outside = mouse_area(space().width(Length::Fill).height(Length::Fill))
                .on_press(Message::Browser(browser::Message::Close));
            layers = layers.push(outside).push(open.view().map(Message::Browser));
        }
        layers.into()
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let hold = button(text(if self.holding { "Release note" } else { "Hold note" }).size(13))
            .on_press_maybe(self.midi.as_ref().map(|_| Message::HoldNote));
        row![
            text(format!(
                "{}{}",
                if self.unsaved() { "* " } else { "" },
                files::display_name(self.file.as_deref())
            ))
            .size(15),
            button(text("New").size(13)).on_press(Message::New),
            button(text("Open").size(13)).on_press(Message::Open),
            button(text("Save").size(13)).on_press(Message::Save),
            button(text("Save As").size(13)).on_press(Message::SaveAs),
            space::horizontal(),
            tooltip(
                button(text("Add module").size(13)).on_press(Message::OpenBrowser),
                container(text("Or press A over the canvas, or double-click empty canvas").size(12))
                    .padding(6)
                    .style(container::rounded_box),
                tooltip::Position::Bottom,
            ),
            hold,
            self.history_button("Undo", "Undo", self.history.undo_step(), Message::Undo),
            self.history_button("Redo", "Redo", self.history.redo_step(), Message::Redo),
            button(text("MIDI learn").size(13))
                .on_press(Message::ToggleLearn)
                .style(if self.learning {
                    button::primary
                } else {
                    button::secondary
                }),
        ]
        .spacing(8)
        .padding([6, 10])
        .align_y(iced::Alignment::Center)
        .into()
    }

    /// An Undo or Redo button. It is disabled with nothing to do, and its
    /// hover text names the step.
    fn history_button<'a>(
        &self,
        label: &'a str,
        verb: &str,
        step: Option<&undo::Step>,
        message: Message,
    ) -> Element<'a, Message> {
        let hint = match step {
            Some(step) => format!("{verb}: {}", step.name()),
            None => format!("Nothing to {}", verb.to_lowercase()),
        };
        tooltip(
            button(text(label).size(13)).on_press_maybe(step.map(|_| message)),
            container(text(hint).size(12)).padding(6).style(container::rounded_box),
            tooltip::Position::Bottom,
        )
        .into()
    }

    fn inspector(&self) -> Element<'_, Message> {
        let graph = self.controller.graph();
        let content: Element<'_, Message> = match self.selection.iter().next() {
            Some(&id) if self.selection.len() == 1 => match graph.node(ModuleId(id)) {
                Some(node) => {
                    let d = node.descriptor;
                    let mut items = column![
                        text(d.name).size(17),
                        text(format!("{} #{id}", d.kind())).size(11).color(DIM),
                        text(d.description).size(12),
                    ]
                    .spacing(6);
                    if d.params.is_empty() {
                        items = items.push(text("No parameters.").size(12).color(DIM));
                    }
                    for (index, (spec, value)) in d.params.iter().zip(&node.params).enumerate() {
                        // Hovering the label shows what the canvas hover
                        // text shows (panels.md, Labels and hover text).
                        let hover = HoverText::new(spec.label, spec.description, spec.name);
                        let mut tip = column![text(spec.description).size(12)].spacing(4);
                        if let Some(name) = hover.name {
                            tip = tip.push(text(name).size(11).color(DIM));
                        }
                        let mapping = graph.midi_mapping_of(ModuleId(id), index).map(|(_, m)| m.control);
                        if self.learning {
                            // In learn mode a row arms its parameter, and its
                            // value does not change (midi-learn.md).
                            let armed = self.armed.is_some_and(|a| a.param == (id, index));
                            let detail = match (armed, mapping) {
                                (true, _) => "Waiting for MIDI...".to_string(),
                                (false, Some(control)) => control.to_string(),
                                (false, None) => param::format(spec, *value),
                            };
                            let arm = button(row![
                                text(spec.label).size(12),
                                space::horizontal(),
                                text(detail).size(12)
                            ])
                            .width(Length::Fill)
                            .style(if armed { button::primary } else { button::secondary })
                            .on_press(Message::LearnParam { module: id, index });
                            // A mapping can be removed in learn mode too.
                            let mut line = row![arm].spacing(4).align_y(iced::Alignment::Center);
                            if mapping.is_some() {
                                line = line.push(remove_button(
                                    "Remove MIDI mapping",
                                    Message::Unmap { module: id, index },
                                ));
                            }
                            items = items.push(line);
                            continue;
                        }
                        let mut row_items = column![
                            row![
                                tooltip(
                                    text(spec.label).size(12),
                                    container(tip).padding(6).max_width(260).style(container::rounded_box),
                                    tooltip::Position::Top,
                                ),
                                space::horizontal(),
                                text(param::format(spec, *value)).size(12).color(DIM),
                            ],
                            slider(0.0..=1.0, spec.normalize(*value), move |position| {
                                Message::Param {
                                    module: id,
                                    index,
                                    position,
                                }
                            })
                            .on_release(Message::SliderReleased)
                            .step(param::slider_step(spec)),
                        ]
                        .spacing(2);
                        if let Some(control) = mapping {
                            row_items = row_items.push(
                                row![
                                    text(control.to_string()).size(11).color(DIM),
                                    space::horizontal(),
                                    remove_button("Remove MIDI mapping", Message::Unmap { module: id, index }),
                                ]
                                .align_y(iced::Alignment::Center),
                            );
                        }
                        // SPIKE #186: the app learns which row the pointer
                        // is over, and highlights it during a drag.
                        let lit = self.assign_drag.is_some() && self.drop_row == Some((id, index));
                        let boxed = container(row_items).padding(2).style(move |_| {
                            if lit {
                                container::Style::default()
                                    .background(Color::from_rgba(0.95, 0.6, 0.2, 0.25))
                                    .border(
                                        iced::Border::default()
                                            .rounded(4)
                                            .width(1)
                                            .color(Color::from_rgb(0.95, 0.6, 0.2)),
                                    )
                            } else {
                                container::Style::default()
                            }
                        });
                        items = items.push(
                            mouse_area(boxed)
                                .on_enter(Message::RowEnter(id, index))
                                .on_exit(Message::RowExit(id, index)),
                        );
                    }
                    items.into()
                }
                None => text("").into(),
            },
            Some(_) => text(format!("{} modules selected", self.selection.len()))
                .size(13)
                .into(),
            None => column![
                text("Inspector").size(17),
                text("Select a module to edit its parameters.").size(12).color(DIM),
                text(HELP).size(11).color(DIM),
            ]
            .spacing(8)
            .into(),
        };
        container(scrollable(container(content).padding(12)))
            .width(280)
            .height(Length::Fill)
            .into()
    }

    fn stats_bar(&self) -> Element<'_, Message> {
        let graph = self.controller.graph();
        let frames = if self.measuring {
            self.meter.summary()
        } else {
            "redraws on change".to_string()
        };
        // Load and dropouts are always shown, and turn amber or red before
        // a problem is audible (FR-PERF-04).
        let audio: Element<'_, Message> = match &self.audio_stats {
            Some(stats) => {
                let s = stats.snapshot();
                let engine = self.controller.stats();
                let dropped = engine.dropped_meters.load(std::sync::atomic::Ordering::Relaxed)
                    + engine.dropped_scope.load(std::sync::atomic::Ordering::Relaxed);
                let load = bar_text(format!(
                    "load {:.0}% (max {:.0}%)",
                    s.last_load * 100.0,
                    s.max_load * 100.0
                ))
                .color(severity_colour(alerts::load_severity(s.last_load)));
                let dropouts = bar_text(format!("dropouts {}", self.dropouts.count()))
                    .color(severity_colour(self.dropouts.severity(Instant::now())));
                let mut items = row![bar_text(&self.audio_name).color(DIM), load, dropouts].spacing(10);
                if dropped > 0 {
                    items = items.push(bar_text(format!("telemetry dropped {dropped}")).color(DIM));
                }
                items.into()
            }
            None => bar_text(&self.audio_name).color(DIM).into(),
        };
        let midi = match self.midi_ports.as_slice() {
            [] => "MIDI: no inputs".to_string(),
            ports => format!(
                "MIDI: {}",
                ports
                    .iter()
                    .map(MidiConnection::port_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        row![
            toggler(self.measuring)
                .label("Measure")
                .on_toggle(Message::Measure)
                .size(14)
                .text_size(12),
            toggler(self.animating)
                .label("Move camera")
                .on_toggle(Message::Animate)
                .size(14)
                .text_size(12),
            bar_text(frames),
            bar_text(format!("{} modules, {} cables", graph.len(), graph.cables().count())),
            audio,
            bar_text(midi).color(DIM),
            // The status takes the remaining width and is cut off rather than
            // wrapped, so the bar stays one line high. Hover shows all of it.
            tooltip(
                container(bar_text(&self.status).color(DIM))
                    .width(Length::Fill)
                    .align_x(iced::Alignment::End)
                    .clip(true),
                container(text(&self.status).size(12))
                    .padding(6)
                    .max_width(480)
                    .style(container::rounded_box),
                tooltip::Position::Top,
            ),
        ]
        .spacing(16)
        .padding([4, 10])
        .align_y(iced::Alignment::Center)
        .into()
    }
}

/// The inspector's remove button for an item in a list, such as a MIDI
/// mapping or a modulation assignment: a small × at the end of its row,
/// with hover text that names what it removes (panels.md, The inspector).
fn remove_button<'a>(what: &'a str, on_press: Message) -> Element<'a, Message> {
    tooltip(
        button(text("×").size(14))
            .padding([0, 6])
            .style(button::text)
            .on_press(on_press),
        container(text(what).size(12)).padding(6).style(container::rounded_box),
        tooltip::Position::Top,
    )
    .into()
}

/// Stats bar text: one line, never wrapped.
fn bar_text<'a>(content: impl text::IntoFragment<'a>) -> iced::widget::Text<'a> {
    text(content).size(12).wrapping(text::Wrapping::None)
}

const DIM: Color = Color::from_rgb(0.62, 0.65, 0.71);

/// Stats bar colour for a warning level.
fn severity_colour(severity: alerts::Severity) -> Color {
    match severity {
        alerts::Severity::Normal => DIM,
        alerts::Severity::Warning => Color::from_rgb8(0xF5, 0xA6, 0x23),
        alerts::Severity::Alarm => Color::from_rgb8(0xE0, 0x4F, 0x4F),
    }
}

const HELP: &str = "A or a double-click on empty canvas opens the module browser at the pointer; type, then Enter adds.\n\
Drag from a port to patch a cable. Drag a connected input to move or remove its cable.\n\
Drag tiles to move them. Shift-click or drag a box to select several.\n\
Drag a knob up or down; Shift for fine steps; double-click resets it.\n\
With a module selected, Tab moves between its controls and the arrow keys change them.\n\
Right-drag or two-finger scroll to pan. Wheel or Cmd/Ctrl-scroll to zoom.\n\
F or Home frames the patch. Delete removes the selection.\n\
MIDI learn: click a control, then move a knob on your controller. Escape ends it.";

/// Frame timing over the last second.
#[derive(Default)]
struct FrameMeter {
    times: VecDeque<Instant>,
    seen: u64,
    worst: Duration,
}

impl FrameMeter {
    fn record(&mut self, now: Instant) {
        self.seen += 1;
        if let Some(&last) = self.times.back()
            && self.seen > 10
        {
            // Skip the first frames, which include start-up work.
            self.worst = self.worst.max(now - last);
        }
        self.times.push_back(now);
        while self.times.front().is_some_and(|&t| now - t > Duration::from_secs(1)) {
            self.times.pop_front();
        }
    }

    fn summary(&self) -> String {
        let fps = self.times.len().saturating_sub(1);
        let average = match (self.times.front(), self.times.back()) {
            (Some(first), Some(last)) if fps > 0 => (*last - *first).as_secs_f64() * 1000.0 / fps as f64,
            _ => 0.0,
        };
        format!(
            "{fps} fps | frame {average:.1} ms (worst {:.1} ms)",
            self.worst.as_secs_f64() * 1000.0
        )
    }
}

/// The keys the open browser handles, whether or not a widget used them.
fn browser_key(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    use iced::keyboard::{Event, Key, key::Named};
    let iced::Event::Keyboard(Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    let nav = match key {
        Key::Named(Named::ArrowUp) => browser::Nav::Up,
        Key::Named(Named::ArrowDown) => browser::Nav::Down,
        Key::Named(Named::PageUp) => browser::Nav::PageUp,
        Key::Named(Named::PageDown) => browser::Nav::PageDown,
        Key::Named(Named::Escape) => browser::Nav::Escape,
        _ => return None,
    };
    Some(Message::Browser(browser::Message::Key(nav)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::Rectangle;

    fn app() -> App {
        App::new(Args {
            patch: None,
            generate: None,
            no_audio: true,
            bench: false,
        })
    }

    fn open_at(app: &mut App, world: Point) {
        let _ = app.update(Message::Canvas(CanvasEvent::OpenBrowser {
            world,
            at: Point::ORIGIN,
            area: Rectangle::new(Point::ORIGIN, iced::Size::new(1000.0, 800.0)),
        }));
    }

    fn type_query(app: &mut App, query: &str) {
        let _ = app.update(Message::Browser(browser::Message::Query(query.to_string())));
    }

    #[test]
    fn adding_from_the_browser_is_one_undo_step_at_the_snapped_point() {
        let mut app = app();
        let before = app.controller.graph().nodes().count();
        let undo = app.history.undo_step().map(|s| s.name().to_string());

        // Opening, typing, and closing are not edits.
        open_at(&mut app, Point::new(250.0, 130.0));
        type_query(&mut app, "svf");
        let _ = app.update(Message::Browser(browser::Message::Key(browser::Nav::Escape)));
        assert!(app.browser.is_none());
        assert_eq!(app.history.undo_step().map(|s| s.name().to_string()), undo);

        open_at(&mut app, Point::new(250.0, 130.0));
        type_query(&mut app, "svf");
        let _ = app.update(Message::Browser(browser::Message::Submit));
        assert!(app.browser.is_none());
        assert_eq!(app.controller.graph().nodes().count(), before + 1);
        let id = *app.selection.iter().next().expect("the new module is selected");
        assert_eq!(app.selection.len(), 1);
        assert_eq!(app.positions[&id], Point::new(252.0, 132.0));

        app.undo();
        assert_eq!(app.controller.graph().nodes().count(), before);
        assert_eq!(app.history.undo_step().map(|s| s.name().to_string()), undo);
    }

    #[test]
    fn keys_go_to_the_browser_not_the_shortcuts() {
        let mut app = app();
        let before = app.controller.graph().nodes().count();
        open_at(&mut app, Point::ORIGIN);
        type_query(&mut app, "saw");
        let _ = app.update(Message::Browser(browser::Message::Submit));
        assert_eq!(app.controller.graph().nodes().count(), before + 1);

        // Cmd+Z or Ctrl+Z while the browser is open does not undo.
        open_at(&mut app, Point::ORIGIN);
        let _ = app.update(Message::Key(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character("z".into()),
            modified_key: iced::keyboard::Key::Character("z".into()),
            physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::COMMAND,
            text: None,
            repeat: false,
        }));
        assert_eq!(app.controller.graph().nodes().count(), before + 1);
    }
}

/// SPIKE #186: drives the real app view headlessly, the way the Iced runtime
/// does: build the UI, feed events with a cursor, apply the messages, and
/// rebuild. It checks Iced's widget routing only. What winit and macOS
/// deliver outside the window is not covered here.
#[cfg(test)]
mod spike_186 {
    use super::*;
    use iced::advanced::clipboard;
    use iced::advanced::renderer::{Headless, Style};
    use iced::{Event, Size, mouse, window};
    use iced_runtime::user_interface::{Cache, UserInterface};
    use loom_canvas::PortRef;

    const SIZE: Size = Size::new(1440.0, 900.0);

    struct Harness {
        app: App,
        renderer: iced::Renderer,
        cache: Option<Cache>,
        log: Vec<String>,
    }

    impl Harness {
        fn new() -> Self {
            let app = App::new(Args {
                patch: Some(PathBuf::from("../../examples/patches/subtractive.ron")),
                generate: None,
                no_audio: true,
                bench: false,
            });
            let renderer = iced_runtime::futures::futures::executor::block_on(<iced::Renderer as Headless>::new(
                iced::Font::DEFAULT,
                iced::Pixels(16.0),
                None,
            ))
            .expect("headless renderer");
            Harness {
                app,
                renderer,
                cache: Some(Cache::default()),
                log: Vec::new(),
            }
        }

        /// One runtime cycle: events in, messages applied, then a draw.
        fn step(&mut self, events: &[Event], cursor: mouse::Cursor) -> Vec<Message> {
            let mut messages = Vec::new();
            {
                let view = self.app.view();
                let mut ui = UserInterface::build(view, SIZE, self.cache.take().unwrap(), &mut self.renderer);
                let _ = ui.update(events, cursor, &mut self.renderer, &mut clipboard::Null, &mut messages);
                self.cache = Some(ui.into_cache());
            }
            for m in &messages {
                match m {
                    Message::Canvas(CanvasEvent::SpikeView { .. } | CanvasEvent::VisibleDisplays(_)) => {}
                    other => self.log.push(format!("{cursor:?} -> {other:?}")),
                }
                let _ = self.app.update(m.clone());
            }
            // Draw too, so the overlay line and the row highlight are built.
            let view = self.app.view();
            let mut ui = UserInterface::build(view, SIZE, self.cache.take().unwrap(), &mut self.renderer);
            ui.draw(
                &mut self.renderer,
                &self.app.theme,
                &Style {
                    text_color: Color::WHITE,
                },
                cursor,
            );
            self.cache = Some(ui.into_cache());
            messages
        }

        fn at(p: Point) -> mouse::Cursor {
            mouse::Cursor::Available(p)
        }

        fn move_to(&mut self, p: Point) -> Vec<Message> {
            self.step(&[Event::Mouse(mouse::Event::CursorMoved { position: p })], Self::at(p))
        }

        /// The source port, ADSR #5 output 0, in window coordinates.
        fn source(&mut self) -> Point {
            let redraw = Event::Window(window::Event::RedrawRequested(Instant::now()));
            let mut found = None;
            for _ in 0..3 {
                for m in self.step(std::slice::from_ref(&redraw), mouse::Cursor::Unavailable) {
                    if let Message::Canvas(CanvasEvent::SpikeView { bounds, camera }) = m {
                        found = Some((bounds, camera));
                    }
                }
            }
            let (bounds, camera) = found.expect("canvas reports its view");
            let world = self
                .app
                .scene
                .port_position(PortRef {
                    module: 5,
                    output: true,
                    index: 0,
                })
                .unwrap();
            let s = camera.to_screen(world);
            Point::new(bounds.x + s.x, bounds.y + s.y)
        }

        fn drags(messages: &[Message]) -> Vec<Option<Point>> {
            messages
                .iter()
                .filter_map(|m| match m {
                    Message::Canvas(CanvasEvent::AssignDrag { at, .. }) => Some(*at),
                    _ => None,
                })
                .collect()
        }
    }

    /// Selects SVF #4, presses on the ADSR output, and sweeps the pointer
    /// down the inspector. Returns the y of the first row the app saw.
    fn start_drag(h: &mut Harness) -> (Point, f32) {
        let _ = h.app.update(Message::Canvas(CanvasEvent::Select(vec![4])));
        let port = h.source();
        h.move_to(port);
        h.step(
            &[Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))],
            Harness::at(port),
        );
        assert!(h.app.assign_drag.is_none(), "no drag before the pointer moves");
        let x = SIZE.width - 140.0;
        let mut first_row = None;
        for i in 0..=60 {
            let p = Point::new(port.x + (x - port.x) * (i as f32 / 60.0), port.y);
            let out = h.move_to(p);
            let drag = Harness::drags(&out);
            assert_eq!(drag, vec![Some(p)], "the canvas reports every move, at {p:?}");
        }
        for y in (0..SIZE.height as i32).step_by(4) {
            h.move_to(Point::new(x, y as f32));
            if first_row.is_none() && h.app.drop_row.is_some() {
                first_row = Some(y as f32);
            }
        }
        (
            port,
            first_row.expect("the app saw a row while the canvas owned the drag"),
        )
    }

    #[test]
    fn drop_on_a_row() {
        let mut h = Harness::new();
        let (_, y) = start_drag(&mut h);
        let x = SIZE.width - 140.0;
        h.move_to(Point::new(x, y + 2.0));
        let row = h.app.drop_row;
        assert!(row.is_some_and(|(m, _)| m == 4));
        let lit = h.app.assign_drag.is_some();
        assert!(lit, "the row would be highlighted");
        let out = h.step(
            &[Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))],
            Harness::at(Point::new(x, y + 2.0)),
        );
        assert!(
            out.iter()
                .any(|m| matches!(m, Message::Canvas(CanvasEvent::AssignDrop { .. }))),
            "release over the inspector reaches the canvas"
        );
        assert!(h.app.status.starts_with("Spike: would assign"), "{}", h.app.status);
        assert!(h.app.assign_drag.is_none());
        // Mid sweep log, for the report.
        let enters: Vec<_> = h.log.iter().filter(|l| l.contains("Row")).take(6).collect();
        eprintln!("row messages: {enters:#?}");
        eprintln!("dropped on {row:?}");
    }

    #[test]
    fn escape_cancels() {
        let mut h = Harness::new();
        let (_, y) = start_drag(&mut h);
        let x = SIZE.width - 140.0;
        h.move_to(Point::new(x, y + 2.0));
        let key = iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape);
        let esc = Event::Keyboard(iced::keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::Escape),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        });
        let out = h.step(&[esc], Harness::at(Point::new(x, y + 2.0)));
        assert!(
            out.iter()
                .any(|m| matches!(m, Message::Canvas(CanvasEvent::AssignCancel)))
        );
        assert!(h.app.assign_drag.is_none());
        let out = h.step(
            &[Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))],
            Harness::at(Point::new(x, y + 2.0)),
        );
        assert!(
            !out.iter()
                .any(|m| matches!(m, Message::Canvas(CanvasEvent::AssignDrop { .. }))),
            "{out:?}"
        );
        assert!(!h.app.status.starts_with("Spike"));
    }

    /// What winit sends when the pointer leaves the window with the button
    /// held: CursorLeft (Iced then has no cursor), then, on macOS, moves at
    /// positions outside the window, then the release.
    #[test]
    fn release_outside_the_window() {
        let mut h = Harness::new();
        let (_, y) = start_drag(&mut h);
        let x = SIZE.width - 140.0;
        h.move_to(Point::new(x, y + 2.0));
        assert!(h.app.drop_row.is_some());
        let out = h.step(&[Event::Mouse(mouse::Event::CursorLeft)], mouse::Cursor::Unavailable);
        eprintln!("after CursorLeft: {out:?}, drop_row {:?}", h.app.drop_row);
        let outside = Point::new(SIZE.width + 60.0, y);
        let out = h.move_to(outside);
        eprintln!("move outside: {out:?}, drop_row {:?}", h.app.drop_row);
        assert_eq!(Harness::drags(&out), vec![Some(outside)]);
        assert!(h.app.drop_row.is_none(), "the row exits when the pointer leaves");
        let out = h.step(
            &[Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))],
            Harness::at(outside),
        );
        assert!(
            out.iter()
                .any(|m| matches!(m, Message::Canvas(CanvasEvent::AssignDrop { .. })))
        );
        assert!(!h.app.status.starts_with("Spike"), "a drop outside does nothing");
        assert!(h.app.assign_drag.is_none());
    }
}
