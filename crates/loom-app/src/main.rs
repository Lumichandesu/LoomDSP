//! `loom-app`: the LoomDSP desktop app.
//!
//! Phase 1 prototype: the patch canvas (ADR-0010), an inspector for the
//! selected module, live audio and MIDI, and a stats bar for measuring the
//! canvas against the performance gates in NFR-PERF-04.

mod alerts;
mod layout;
mod lights;
mod live;
mod monitor;
mod param;
mod scene;

use clap::Parser;
use iced::widget::{
    button, column, container, pick_list, row, rule, scrollable, slider, space, text, toggler, tooltip,
};
use iced::{Color, Element, Length, Point, Subscription, Theme};
use live::Live;
use loom_canvas::{CanvasEvent, HoverText, PanelValues, PatchCanvas, Scene};
use loom_core::{
    Controller, EngineConfig, EngineParts, InputRef, MidiEvent, ModuleId, OutputRef, ProbeId, create_engine,
};
use loom_host_desktop::{AudioOptions, AudioOutput, CallbackStats, MidiConnection, RunningAudio, SharedMidi};
use loom_patch::Patch;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
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
        .window_size((1440.0, 900.0))
        .antialiasing(false)
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    Canvas(CanvasEvent),
    Param {
        module: u32,
        index: usize,
        position: f32,
    },
    Pick(String),
    Add,
    HoldNote,
    Save,
    Measure(bool),
    Animate(bool),
    Frame(Instant),
    Tick,
    /// New brightness for lights on screen.
    Lights(Vec<(lights::Light, f32)>),
    /// New levels for meter probes.
    Meters(Vec<(ProbeId, live::Level)>),
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
    name: String,
    save_path: PathBuf,
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
    kinds: Vec<String>,
    picked: Option<String>,
    frame_key: u64,
    meter: FrameMeter,
    measuring: bool,
    animating: bool,
}

impl App {
    fn new(args: Args) -> Self {
        let output = if args.no_audio {
            Err("audio off".to_string())
        } else {
            AudioOutput::open(&AudioOptions::default()).map_err(|e| e.to_string())
        };
        let sample_rate = output.as_ref().map(|o| o.sample_rate() as f32).unwrap_or(48_000.0);
        let EngineParts {
            mut controller,
            engine,
            midi: sender,
            telemetry,
        } = create_engine(loom_modules::registry(), EngineConfig::with_sample_rate(sample_rate));

        let mut status = String::new();
        let mut positions = BTreeMap::new();
        let (name, save_path) = match (&args.patch, args.generate) {
            (_, Some(n)) => {
                if let Err(e) = layout::generate(&mut controller, n) {
                    status = format!("generate failed: {e}");
                }
                layout::generated_layout(&controller, &mut positions);
                (
                    format!("Generated ({n} modules)"),
                    PathBuf::from(format!("generated-{n}.ron")),
                )
            }
            (Some(path), None) => match Patch::load(path).and_then(|p| p.load_into(&mut controller).map(|()| p)) {
                Ok(patch) => {
                    for m in &patch.modules {
                        if let Some((x, y)) = m.position {
                            positions.insert(m.id, Point::new(x, y));
                        }
                    }
                    // Saves go next to the file, so examples keep their comments.
                    (patch.name.clone(), path.with_extension("saved.ron"))
                }
                Err(e) => {
                    status = format!("could not open {}: {e}", path.display());
                    ("Untitled".to_string(), PathBuf::from("untitled.ron"))
                }
            },
            (None, None) => ("Untitled".to_string(), PathBuf::from("untitled.ron")),
        };
        layout::auto_layout(&controller, &mut positions);

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
            controller,
            positions,
            scene,
            values,
            lights: BTreeMap::new(),
            meter_levels: std::collections::HashMap::new(),
            visible_displays: Vec::new(),
            selection: BTreeSet::new(),
            name,
            save_path,
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
            monitor: monitor::Monitor::new(telemetry),
            live: Live::new(),
            dropouts: alerts::Dropouts::default(),
            midi_ports,
            audio_name,
            midi,
            holding: false,
            status,
            kinds: loom_modules::registry().iter().map(|d| d.id.to_string()).collect(),
            picked: None,
            frame_key: 0,
            meter: FrameMeter::default(),
            measuring: args.bench,
            animating: args.bench,
        };
        if app.status.is_empty() {
            app.report();
        }
        app
    }

    fn title(&self) -> String {
        format!("{} - LoomDSP", self.name)
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
        Subscription::batch([main, self.light_watch(), telemetry])
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
        };
        Subscription::run_with(watch, lights::stream).map(Message::Lights)
    }

    /// Rebuilds the scene after the graph changed.
    fn refresh(&mut self) {
        layout::auto_layout(&self.controller, &mut self.positions);
        self.scene = scene::build(&self.controller, &self.positions);
        self.refresh_values();
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
    fn set_param(&mut self, module: u32, index: usize, value: f32) {
        let clear_on = self.controller.graph().node(ModuleId(module)).and_then(|node| {
            let spec = node.descriptor.params.get(index)?;
            let next = spec.clamp(value);
            scene::switches_button_to_momentary(node.descriptor.id, spec.name, node.params[index], next)
                .then(|| node.descriptor.param_index("on"))
                .flatten()
        });
        if let Err(e) = self.controller.set_param(ModuleId(module), index, value) {
            self.status = format!("error: {e}");
        } else if let Some(on) = clear_on
            && let Err(e) = self.controller.set_param(ModuleId(module), on, 0.0)
        {
            self.status = format!("error: {e}");
        }
        self.refresh_values();
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

    fn commit(&mut self) {
        // Probes follow the graph: a removed module loses its probes.
        let wanted = self.metered_outputs();
        self.live.sync(&mut self.controller, &wanted);
        match self.controller.commit() {
            Ok(_) => self.report(),
            Err(e) => self.status = format!("error: {e}"),
        }
        self.refresh();
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Canvas(event) => self.canvas_event(event),
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
                    self.set_param(module, index, value);
                }
            }
            Message::Pick(kind) => self.picked = Some(kind),
            Message::Add => {
                if let Some(kind) = self.picked.clone() {
                    match self.controller.add_module(&kind) {
                        Ok(id) => {
                            self.selection = BTreeSet::from([id.0]);
                            self.commit();
                        }
                        Err(e) => self.status = format!("error: {e}"),
                    }
                }
            }
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
            Message::Save => {
                let mut patch = Patch::from_graph(&self.name, self.controller.graph());
                for m in &mut patch.modules {
                    m.position = self.positions.get(&m.id).map(|p| (p.x.round(), p.y.round()));
                }
                self.status = match patch.save(&self.save_path) {
                    Ok(()) => format!("saved {}", self.save_path.display()),
                    Err(e) => format!("save failed: {e}"),
                };
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
        }
    }

    fn canvas_event(&mut self, event: CanvasEvent) {
        match event {
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
                    Ok(_) => self.commit(),
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
            CanvasEvent::ResetControl { module, widget } => {
                if let Some((index, _, spec)) = scene::control_param(&self.controller, module, widget) {
                    self.set_param(module, index, spec.default);
                }
            }
            CanvasEvent::VisibleDisplays(ids) => {
                self.visible_displays = ids;
                self.sync_probes();
                self.refresh_values();
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
            .animate(self.animating);

        let body = row![
            container(canvas).width(Length::Fill).height(Length::Fill),
            rule::vertical(1),
            self.inspector(),
        ];

        column![
            self.toolbar(),
            rule::horizontal(1),
            body,
            rule::horizontal(1),
            self.stats_bar()
        ]
        .into()
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let hold = button(text(if self.holding { "Release note" } else { "Hold note" }).size(13))
            .on_press_maybe(self.midi.as_ref().map(|_| Message::HoldNote));
        row![
            text(&self.name).size(15),
            space::horizontal(),
            pick_list(self.kinds.as_slice(), self.picked.as_ref(), Message::Pick)
                .placeholder("Module...")
                .text_size(13),
            button(text("Add").size(13)).on_press_maybe(self.picked.as_ref().map(|_| Message::Add)),
            hold,
            button(text("Save").size(13)).on_press(Message::Save),
        ]
        .spacing(8)
        .padding([6, 10])
        .align_y(iced::Alignment::Center)
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
                        items = items.push(
                            column![
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
                                .step(param::slider_step(spec)),
                            ]
                            .spacing(2),
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

const HELP: &str = "Drag from a port to patch a cable. Drag a connected input to move or remove its cable.\n\
Drag tiles to move them. Shift-click or drag a box to select several.\n\
Drag a knob up or down; Shift for fine steps; double-click resets it.\n\
With a module selected, Tab moves between its controls and the arrow keys change them.\n\
Right-drag or two-finger scroll to pan. Wheel or Cmd/Ctrl-scroll to zoom.\n\
F or Home frames the patch. Delete removes the selection.";

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
