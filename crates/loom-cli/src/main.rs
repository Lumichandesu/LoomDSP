//! `loom`: the LoomDSP command-line tool.
//!
//! Phase 0 has no UI yet. This tool proves the engine works end to end:
//! load a patch, play it with a MIDI controller, render it to a file, and
//! stress-test live editing.

use clap::{Parser, Subcommand};
use loom_core::{Controller, EngineConfig, EngineParts, MidiEvent, ModuleId, create_engine};
use loom_host_desktop::{AudioOptions, AudioOutput, StatsSnapshot};
use loom_patch::Patch;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "loom", version, about = "LoomDSP command-line tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List available modules.
    Modules {
        /// Show ports and parameters.
        #[arg(short, long)]
        verbose: bool,
    },
    /// List audio output devices and MIDI input ports.
    Devices,
    /// Load a patch and report what the compiler found (feedback loops,
    /// inserted delays).
    Check {
        /// Patch file (.ron).
        patch: PathBuf,
    },
    /// Render a patch to a 32-bit float stereo WAV file.
    Render {
        /// Patch file (.ron).
        patch: PathBuf,
        /// Output WAV file.
        #[arg(short, long)]
        output: PathBuf,
        /// Length in seconds.
        #[arg(long, default_value_t = 4.0)]
        seconds: f32,
        /// Sample rate in Hz.
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Host buffer size used while rendering. The result does not depend
        /// on it; this option exists to prove that.
        #[arg(long, default_value_t = 128)]
        block: usize,
        /// MIDI notes to play one after another (repeat the option).
        #[arg(long = "note")]
        notes: Vec<u8>,
        /// Length of each note in seconds.
        #[arg(long, default_value_t = 1.0)]
        note_length: f32,
    },
    /// Play a patch live with a MIDI controller.
    Play {
        /// Patch file (.ron).
        patch: PathBuf,
        #[command(flatten)]
        audio: AudioArgs,
        /// Hold this MIDI note, for testing without a controller.
        #[arg(long)]
        note: Option<u8>,
    },
    /// Play a patch while editing its structure many times per second, and
    /// report dropouts (P0 exit criterion).
    Stress {
        /// Patch file (.ron).
        patch: PathBuf,
        #[command(flatten)]
        audio: AudioArgs,
        /// How long to run, in seconds.
        #[arg(long, default_value_t = 60.0)]
        seconds: f32,
        /// Structural edits per second.
        #[arg(long, default_value_t = 20.0)]
        rate: f32,
        /// Hold this MIDI note (default: middle C), so there is sound to hear.
        #[arg(long, default_value_t = 60)]
        note: u8,
        /// Run without an audio device, as fast as possible, and report how
        /// long each buffer took compared with its real-time budget.
        #[arg(long)]
        offline: bool,
    },
}

#[derive(clap::Args)]
struct AudioArgs {
    /// Audio output device (part of the name). Default: system default.
    #[arg(long)]
    device: Option<String>,
    /// Sample rate in Hz. Default: device default.
    #[arg(long)]
    sample_rate: Option<u32>,
    /// Buffer size in frames. Default: device default.
    #[arg(long)]
    buffer: Option<u32>,
    /// MIDI input port (part of the name). Default: first port found.
    #[arg(long)]
    midi: Option<String>,
    /// Do not open any MIDI input.
    #[arg(long)]
    no_midi: bool,
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Modules { verbose } => modules(verbose),
        Command::Devices => devices(),
        Command::Check { patch } => check(&patch),
        Command::Render {
            patch,
            output,
            seconds,
            sample_rate,
            block,
            notes,
            note_length,
        } => render(&patch, &output, seconds, sample_rate, block, &notes, note_length),
        Command::Play { patch, audio, note } => play(&patch, &audio, note),
        Command::Stress {
            patch,
            audio,
            seconds,
            rate,
            note,
            offline,
        } => {
            if offline {
                stress_offline(&patch, seconds, rate, note)
            } else {
                stress_live(&patch, &audio, seconds, rate, note)
            }
        }
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn load_engine(path: &Path, sample_rate: f32) -> Result<(Patch, EngineParts)> {
    let patch = Patch::load(path)?;
    let mut parts = create_engine(loom_modules::registry(), EngineConfig::with_sample_rate(sample_rate));
    patch.load_into(&mut parts.controller)?;
    Ok((patch, parts))
}

fn modules(verbose: bool) -> Result<ExitCode> {
    let registry = loom_modules::registry();
    for d in registry.iter() {
        println!("{:<28} {:<16} {}", d.kind(), d.name, d.description);
        if verbose {
            for p in d.inputs {
                println!("    in   {:<12} {}", p.name, p.signal.name());
            }
            for p in d.outputs {
                println!("    out  {:<12} {}", p.name, p.signal.name());
            }
            for p in d.params {
                println!(
                    "    param {:<11} {} .. {} (default {}) {}",
                    p.name, p.min, p.max, p.default, p.unit
                );
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn devices() -> Result<ExitCode> {
    println!("Audio outputs:");
    for name in loom_host_desktop::output_device_names()? {
        println!("  {name}");
    }
    println!("MIDI inputs:");
    let midi = loom_host_desktop::midi_input_names()?;
    if midi.is_empty() {
        println!("  (none)");
    }
    for name in midi {
        println!("  {name}");
    }
    Ok(ExitCode::SUCCESS)
}

fn describe(controller: &Controller, id: ModuleId) -> String {
    controller
        .graph()
        .node(id)
        .map(|n| format!("{} {id}", n.descriptor.name))
        .unwrap_or_else(|| id.to_string())
}

fn check(path: &Path) -> Result<ExitCode> {
    let (patch, parts) = load_engine(path, 48_000.0)?;
    let c = &parts.controller;
    let report = c.last_report();
    println!("{}: \"{}\"", path.display(), patch.name);
    println!("  {} modules, {} cables", report.modules, c.graph().cables().count());
    if report.loops.is_empty() {
        println!("  no feedback loops");
    }
    for (i, feedback) in report.loops.iter().enumerate() {
        let names: Vec<String> = feedback.modules.iter().map(|&id| describe(c, id)).collect();
        println!(
            "  feedback loop {} (runs one sample at a time): {}",
            i + 1,
            names.join(", ")
        );
        if feedback.inserted_delays.is_empty() && feedback.inserted_assignment_delays.is_empty() {
            println!("    broken by your 1-sample delay module(s)");
        }
        for id in &feedback.inserted_assignment_delays {
            if let Some(assignment) = c.graph().assignment(*id) {
                println!(
                    "    inserted a 1-sample delay on modulation: {} -> {}",
                    describe(c, assignment.source.module),
                    describe(c, assignment.target)
                );
            }
        }
        for (from, to) in &feedback.inserted_delays {
            let graph = c.graph();
            let out_name = graph
                .node(from.module)
                .map(|n| n.descriptor.outputs[from.port].name)
                .unwrap_or("?");
            let in_name = graph
                .node(to.module)
                .map(|n| n.descriptor.inputs[to.port].name)
                .unwrap_or("?");
            println!(
                "    inserted a 1-sample delay: {} `{out_name}` -> {} `{in_name}`",
                describe(c, from.module),
                describe(c, to.module)
            );
        }
    }
    // Notes do not fail the check: the patch still loads and plays.
    let notes = c.registry().notes();
    if !notes.is_empty() {
        println!("  module notes:");
        for note in notes {
            println!("    {note}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn render(
    path: &Path,
    output: &Path,
    seconds: f32,
    sample_rate: u32,
    block: usize,
    notes: &[u8],
    note_length: f32,
) -> Result<ExitCode> {
    let (_, mut parts) = load_engine(path, sample_rate as f32)?;
    let frames = (seconds * sample_rate as f32) as usize;
    let note_frames = ((note_length * sample_rate as f32) as usize).max(2);
    // Each note plays for 90% of its slot, so notes retrigger cleanly.
    let events: Vec<(usize, MidiEvent)> = notes
        .iter()
        .enumerate()
        .flat_map(|(i, &note)| {
            let on = i * note_frames;
            let off = on + note_frames * 9 / 10;
            [
                (on, MidiEvent::note_on(0, 0, note, 100)),
                (off, MidiEvent::note_off(0, 0, note)),
            ]
        })
        .collect();

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(output, spec)?;
    let mut left = vec![0.0; block.max(1)];
    let mut right = vec![0.0; block.max(1)];
    let mut position = 0;
    let mut peak = 0.0f32;
    let started = Instant::now();
    while position < frames {
        let n = left.len().min(frames - position);
        for (at, event) in &events {
            if (position..position + n).contains(at) {
                parts.midi.send(MidiEvent {
                    frame: (at - position) as u32,
                    ..*event
                });
            }
        }
        parts.engine.process(&mut left[..n], &mut right[..n]);
        for (l, r) in left[..n].iter().zip(&right[..n]) {
            writer.write_sample(*l)?;
            writer.write_sample(*r)?;
            peak = peak.max(l.abs()).max(r.abs());
        }
        position += n;
    }
    writer.finalize()?;
    let elapsed = started.elapsed().as_secs_f32();
    println!(
        "wrote {} ({seconds} s, peak {:.2}, rendered {:.0}x faster than real time)",
        output.display(),
        peak,
        seconds / elapsed.max(1e-6)
    );
    if notes.is_empty() {
        println!("hint: most example patches need a note to make sound, for example `--note 60`");
    }
    Ok(ExitCode::SUCCESS)
}

struct Live {
    _audio: loom_host_desktop::RunningAudio,
    _midi: Option<loom_host_desktop::MidiConnection>,
    stats: Arc<loom_host_desktop::CallbackStats>,
    controller: Controller,
}

fn start_live(path: &Path, args: &AudioArgs, hold_note: Option<u8>) -> Result<Live> {
    let output = AudioOutput::open(&AudioOptions {
        device: args.device.clone(),
        sample_rate: args.sample_rate,
        buffer_size: args.buffer,
    })?;
    let (patch, parts) = load_engine(path, output.sample_rate() as f32)?;
    let EngineParts {
        controller,
        engine,
        midi: mut sender,
        ..
    } = parts;
    println!("patch:  {} ({})", patch.name, path.display());
    println!(
        "audio:  {} at {} Hz, {} channels, buffer {}",
        output.name(),
        output.sample_rate(),
        output.channels(),
        output
            .buffer_size()
            .map_or("device default".to_string(), |b| format!("{b} frames"))
    );

    let midi = if let Some(note) = hold_note {
        sender.send(MidiEvent::note_on(0, 0, note, 100));
        println!("midi:   holding note {note} (no MIDI input opened)");
        None
    } else if args.no_midi {
        println!("midi:   off");
        None
    } else {
        match loom_host_desktop::connect_midi(args.midi.as_deref(), loom_host_desktop::share_midi(sender))? {
            Some(connection) => {
                println!("midi:   {}", connection.port_name());
                Some(connection)
            }
            None => {
                println!("midi:   no MIDI input found (use --note 60 to test without one)");
                None
            }
        }
    };

    let audio = output.start(engine)?;
    Ok(Live {
        stats: audio.stats().clone(),
        _audio: audio,
        _midi: midi,
        controller,
    })
}

fn format_stats(s: &StatsSnapshot) -> String {
    format!(
        "load {:>5.1}% (max {:>5.1}%), buffers {}..{} frames, overruns {}, stream errors {}",
        s.last_load * 100.0,
        s.max_load * 100.0,
        s.smallest_buffer,
        s.largest_buffer,
        s.overruns,
        s.stream_errors
    )
}

fn play(path: &Path, args: &AudioArgs, note: Option<u8>) -> Result<ExitCode> {
    let mut live = start_live(path, args, note)?;
    println!("playing. press Enter to stop.");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = stop.clone();
    std::thread::spawn(move || {
        let _ = std::io::stdin().lock().lines().next();
        stop_reader.store(true, Ordering::Relaxed);
    });
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(500));
        live.controller.collect_garbage();
        eprint!("\r{}   ", format_stats(&live.stats.snapshot()));
    }
    eprintln!();
    Ok(ExitCode::SUCCESS)
}

/// Makes random structural edits that do not change the sound: it adds and
/// removes "noise -> multiply by zero -> audio out" chains and reconnects
/// their cables. The topology changes every time; the output must not.
struct Stressor {
    rng: u32,
    chains: Vec<[ModuleId; 3]>,
    edits: u64,
}

impl Stressor {
    fn new() -> Self {
        Self {
            rng: 0x1234_5678,
            chains: Vec::new(),
            edits: 0,
        }
    }

    fn random(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    fn edit(&mut self, c: &mut Controller) -> Result<()> {
        let choice = self.random() % 3;
        if choice == 0 || self.chains.is_empty() {
            let noise = c.add_module("loom.osc.noise")?;
            let gain = c.add_module("loom.math.mul")?;
            let out = c.add_module("loom.io.audio-out")?;
            c.set_param_named(gain, "gain", 0.0)?;
            c.connect_named(noise, "out", gain, "a")?;
            c.connect_named(gain, "out", out, "left")?;
            self.chains.push([noise, gain, out]);
        } else if choice == 1 || self.chains.len() > 32 {
            let index = self.random() as usize % self.chains.len();
            for id in self.chains.swap_remove(index) {
                c.remove_module(id)?;
            }
        } else {
            let index = self.random() as usize % self.chains.len();
            let [noise, gain, _] = self.chains[index];
            let input = c.input_ref(gain, "a")?;
            if c.disconnect(input)?.is_none() {
                c.connect_named(noise, "out", gain, "a")?;
            }
        }
        c.commit()?;
        self.edits += 1;
        Ok(())
    }
}

fn stress_live(path: &Path, args: &AudioArgs, seconds: f32, rate: f32, note: u8) -> Result<ExitCode> {
    let mut live = start_live(path, args, Some(note))?;
    println!("stress: {rate} structural edits per second for {seconds} s");
    let mut stressor = Stressor::new();
    let interval = Duration::from_secs_f32(1.0 / rate.max(0.1));
    let started = Instant::now();
    let mut last_print = Instant::now();
    while started.elapsed().as_secs_f32() < seconds {
        stressor.edit(&mut live.controller)?;
        std::thread::sleep(interval);
        if last_print.elapsed() >= Duration::from_secs(1) {
            last_print = Instant::now();
            eprint!(
                "\r{:>5.0} s  edits {:>6}  {}   ",
                started.elapsed().as_secs_f32(),
                stressor.edits,
                format_stats(&live.stats.snapshot())
            );
        }
    }
    eprintln!();
    let s = live.stats.snapshot();
    let engine = live.controller.stats();
    let leaked = engine.leaked.load(Ordering::Relaxed);
    println!("edits:          {}", stressor.edits);
    println!("schedule swaps: {}", engine.schedule_swaps.load(Ordering::Relaxed));
    println!("callbacks:      {}", s.callbacks);
    println!("max load:       {:.1}%", s.max_load * 100.0);
    println!("overruns:       {}", s.overruns);
    println!("stream errors:  {}", s.stream_errors);
    println!("leaked objects: {leaked}");
    let passed = s.overruns == 0 && s.stream_errors == 0 && leaked == 0;
    println!("result:         {}", if passed { "PASS" } else { "FAIL" });
    Ok(if passed { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn stress_offline(path: &Path, seconds: f32, rate: f32, note: u8) -> Result<ExitCode> {
    const SAMPLE_RATE: f32 = 48_000.0;
    const BLOCK: usize = 128;
    let (_, mut parts) = load_engine(path, SAMPLE_RATE)?;
    parts.midi.send(MidiEvent::note_on(0, 0, note, 100));
    let mut stressor = Stressor::new();
    let blocks = (seconds * SAMPLE_RATE / BLOCK as f32) as usize;
    let blocks_per_edit = ((SAMPLE_RATE / BLOCK as f32) / rate.max(0.1)).max(1.0) as usize;
    let budget = BLOCK as f64 / f64::from(SAMPLE_RATE);
    let mut left = vec![0.0; BLOCK];
    let mut right = vec![0.0; BLOCK];
    let mut max_load = 0.0f64;
    let mut max_after_edit = 0.0f64;
    let mut max_other = 0.0f64;
    let mut total = 0.0f64;
    let mut over_budget = 0u64;
    let _ftz = loom_rt::ScopedFlushDenormals::new();
    for i in 0..blocks {
        let edited = i % blocks_per_edit == 0;
        if edited {
            stressor.edit(&mut parts.controller)?;
        }
        let started = Instant::now();
        parts.engine.process(&mut left, &mut right);
        let load = started.elapsed().as_secs_f64() / budget;
        max_load = max_load.max(load);
        if edited {
            max_after_edit = max_after_edit.max(load);
        } else {
            max_other = max_other.max(load);
        }
        total += load;
        if load > 1.0 {
            over_budget += 1;
        }
    }
    let leaked = parts.controller.stats().leaked.load(Ordering::Relaxed);
    println!("simulated:      {seconds} s at {SAMPLE_RATE} Hz, {BLOCK}-frame buffers ({blocks} buffers)");
    println!("edits:          {}", stressor.edits);
    println!("mean load:      {:.2}%", total / blocks.max(1) as f64 * 100.0);
    println!("max load:       {:.2}%", max_load * 100.0);
    println!("  after an edit:  {:.2}%", max_after_edit * 100.0);
    println!("  other buffers:  {:.2}%", max_other * 100.0);
    println!("over budget:    {over_budget}");
    println!("leaked objects: {leaked}");
    println!("note: offline timing includes OS scheduling noise; use the live mode for the real test.");
    Ok(if leaked == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
