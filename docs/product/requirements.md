# Functional Requirements

Priorities: **Must**, **Should**, **Could**, **Won't (now)**. Phases refer to the [roadmap](roadmap.md):

- P0 Engine foundations
- P1 Desktop patcher and live basics
- P2 Encapsulation, panels, performance
- P3 Living systems (loopers, groups, logic, scenes, sequencers)
- P4 Sharing and ecosystem
- Later: not scheduled. Plugin and standalone export, and the web target. Revisit after P3.

Each requirement has a short rationale. If the rationale stops being true, the requirement should be reviewed.

---

## 1. Signal model (`SIG`)

See ADR-0007 for the full signal model.

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-SIG-01 | All cables carry 32-bit float samples at the engine sample rate. There is no separate control rate. | Must | P0 | Any output can drive any input without conversion. This is the basis of free patching. |
| FR-SIG-02 | Every port has a signal type: Audio, Modulation, Pitch, Phase, Gate. Each type has a colour and a documented, normalised value range. | Must | P1 | Types help users read a patch and let modules pick sensible defaults. |
| FR-SIG-03 | Signals are polymorphic: any output connects to any input with no conversion module. Ranges are chosen so that cross-type connections give musically useful results. | Must | P1 | "Patch anything to anything", as in Eurorack and Bitwig Grid. |
| FR-SIG-04 | Phase signals wrap to the range 0.0 to 1.0. Oscillators and sequencers can be driven by an external phase input. | Must | P1 | Separating timing from wave shape enables phase distortion, sync, and step indexing with one concept. |
| FR-SIG-05 | Pitch signals use LoomDSP's own documented scale (proposal: 0.1 per octave, 0.0 at a reference note). Helpers convert to and from Hz and MIDI note numbers. | Must | P1 | 0.1 per octave keeps 20 octaves inside the normalised -1 to 1 range, so pitch can modulate anything and anything can modulate pitch. |
| FR-SIG-06 | Gate signals treat values above 0.5 as high. A trigger output is high for one sample; trigger inputs count rising edges (ADR-0007). | Must | P1 | Simple, documented threshold. Any signal can act as a trigger. |
| FR-SIG-07 | Channel count (mono or stereo) is inferred per cable when the graph is built. Stereo flows only where a stereo source exists. Mono inputs receiving stereo are handled per module (sum or process both channels). | Must | P1 | Stereo matters mainly for audio. Computing two channels on every modulation cable wastes CPU. |
| FR-SIG-08 | Cables can carry a polyphonic bundle (one value per active voice). | Must | P2 | Polyphony is required for most instruments. |
| FR-SIG-09 | Every port declares a nominal range (for example 0 to 1 or -1 to 1), by default its signal type's convention. The range is a hint: it is shown to the player, and never enforced or clipped. | Should | P1 | Players need to know what a cable usually carries. Modulation also needs each output's polarity ([port ranges](../design/port-ranges.md)). |

## 2. DSP graph and engine (`GRAPH`)

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-GRAPH-01 | The engine runs a directed graph of modules that can change at runtime (add, remove, connect, disconnect) while audio plays. | Must | P0 | Live patching is the main interaction. |
| FR-GRAPH-02 | Graph changes are applied at a buffer boundary, with no audible dropout. | Must | P0 | Edits during performance must be safe. |
| FR-GRAPH-03 | The graph may contain cycles. Each cycle must pass through at least one single-sample delay. The engine inserts one automatically if the user does not, and shows where. | Must | P1 | Feedback is essential for FM, physical models, and filters. See ADR-0002. |
| FR-GRAPH-04 | Feedback loops are sample-accurate (one-sample delay), not one-buffer. | Must | P1 | Buffer-size feedback changes the sound when the buffer size changes. |
| FR-GRAPH-05 | New or removed connections fade over a short time (for example 5 ms) to avoid clicks. | Should | P1 | Hard switches create clicks on stage. |
| FR-GRAPH-06 | The engine can render offline, faster than real time, to a file or buffer. | Must | P0 | Needed for tests, sound parity checks, and bouncing. |
| FR-GRAPH-07 | The engine supports sample rates from 44.1 kHz to 192 kHz and any host buffer size. | Must | P0 | Plugins and audio interfaces vary. |
| FR-GRAPH-08 | Modules can report their latency. The engine sums path latency and reports total latency to the host. | Should | Later | Hosts need it for delay compensation. |
| FR-GRAPH-09 | Modules that are not connected to an output are not processed. | Should | P1 | Saves CPU in large patches. |

## 3. Polyphony and voices (`VOICE`)

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-VOICE-01 | A patch or sub-patch can be marked polyphonic with a voice count. | Must | P2 | Standard instrument need. |
| FR-VOICE-02 | Voice allocation supports at least oldest-steal, quietest-steal, and mono/legato modes. | Must | P2 | Different instruments need different behaviour. |
| FR-VOICE-03 | Modules exist to merge voices into one signal and to send one signal to all voices. | Must | P2 | Users need to mix voices and share global modulation. |
| FR-VOICE-04 | Per-voice values for MIDI note, velocity, pressure, and MPE dimensions are available as signals. | Should | P2 | Expressive control is a key reason to build custom instruments. |

## 4. Modulation (`MOD`)

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-MOD-01 | Every module parameter has a base value plus zero or more modulation sources, each with its own depth. | Must | P1 | Removes the need for many attenuator and mixer modules. Keeps the canvas readable. |
| FR-MOD-02 | Modulation depth can itself be modulated (for example velocity scales envelope depth). | Should | P2 | Common expressive pattern. |
| FR-MOD-03 | Modulation is evaluated per sample and the final value is clamped to the parameter range. | Must | P1 | Prevents unstable filters and keeps behaviour predictable. |
| FR-MOD-04 | Parameter knobs show the live modulated value and the modulation range. | Must | P1 | Users must see what modulation does. |
| FR-MOD-05 | Parameter changes from the UI and MIDI are smoothed to avoid zipper noise. | Must | P1 | Audible artefacts are unacceptable on stage. |
| FR-MOD-06 | Any output can modulate any parameter in the same patch level through a modulation assignment, without a cable. To reach inside a sub-patch, expose the parameter (FR-ENCAP-03). Made by dragging an output onto a control; shown as a depth arc on the control and a badge on the source (ADR-0014). | Must | P1 | Keeps the cables for the signal path, so patches stay readable. |
| FR-MOD-07 | A modulation matrix lists every assignment in a patch or sub-patch, editable by mouse and keyboard. | Must | P1 | One place to see and fix all routing, including routing without cables. |
| FR-MOD-08 | Modulation depth is applied in the control's 0 to 1 range, using the parameter's own linear or logarithmic mapping. | Must | P1 | Depth means the same on every control, and the arc shows what the engine does. |
| FR-MOD-09 | Wide linear parameters, such as Multiply's gain, offer a choice of ranges (for example -0.1 to 0.1 or 0 to 10). The player picks one per module in the inspector. Controls and modulation work inside it, and patches save it. | Should | P1 | One knob over -10 to 10 is too coarse for fine jobs such as FM depth. A smaller range makes the knob precise and safe ([control ranges](../design/control-ranges.md)). |
| FR-MOD-10 | A parameter that offers ranges can suggest the range that fits what its module's output feeds. The player applies it with one click. Ranges and values never change automatically when cables change. | Could | P2 | Helps the player pick a safe range, without making a patch's sound depend on what is connected ([port ranges](../design/port-ranges.md)). |

## 5. Module library (`LIB`)

The first library should be small and composable. Phase 1 needs enough to build a full subtractive synth from primitives.

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-LIB-01 | Phase: phase generator, phase scaler, phase wrap/fold. Phase reset (MT-PHASE-04) comes in P2. | Must | P1 | Core of the phase-driven model. The tutorial does not need phase reset, so it waits for P2 (#155). |
| FR-LIB-02 | Oscillators/shapers: sine, saw, pulse, triangle (band-limited), wavetable reader, noise. | Must | P1 | Basic sound sources. |
| FR-LIB-03 | Filters: state variable filter (LP/HP/BP/notch), one-pole, ladder-style lowpass. | Must | P1 | Basic subtractive synthesis. |
| FR-LIB-04 | Envelopes and LFOs: ADSR, AD, LFO (phase driven), sample and hold, slew. | Must | P1 | Basic modulation. |
| FR-LIB-05 | Math and logic: add, multiply, mix, crossfade, clamp, scale/offset, compare, AND/OR/NOT, flip-flop, counter. | Must | P1 | Building blocks for anything else. |
| FR-LIB-06 | Delay and time: one-sample delay, delay line with interpolation, allpass. | Must | P1 | Needed for feedback, effects, and physical models. |
| FR-LIB-07 | I/O: audio in/out, MIDI note/CC/pitch bend, transport and clock (tempo, position; see FR-LIB-25 to 27). | Must | P1 | Connects the patch to the outside world. |
| FR-LIB-08 | Sequencing: step sequencer, Euclidean generator. (Clock Divide is FR-LIB-27.) | Should | P2 | Common in performance patches. |
| FR-LIB-09 | Effects: reverb, chorus, distortion/waveshaper, compressor. | Should | P2 | Users expect usable effects out of the box. |
| FR-LIB-10 | Spectral: FFT-based modules. | Could | P4 | Higher complexity and latency. |
| FR-LIB-11 | Every built-in module has a help page with a short description, port list, and example patch. | Must | P1 | Discoverability is essential for learners. |

### Library scope: everything is a module (ADR-0011)

LoomDSP is a grid for building living musical systems: patches that play, evolve, and arrange themselves through modulation, randomness, texture, sequencing, looping, and logic. The long-term library is planned in the [module catalogue](module-catalogue.md): our own list of module types, traits, and roles, each with a stable ID and a phase. It is built from primitives, shared traits, and open abstractions (ADR-0017), designed from first principles (ADR-0018). It grows by phase and through the module SDK.

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-LIB-12 | Modulators as modules, covering these families: audio-driven (envelope follower, audio-rate), envelopes (ADSR, AHDSR, drawn multi-segment curves, ramps), interface (macros, buttons, XY and vector pads), LFOs (tempo-synced, drawn curves, random, wavetable), modifiers (math, quantise, sample and hold, curve shaping), note-driven (expressions, note counter, keytracking, per-pitch-class), and sequence (step, per-step parameter sequences, multi-stage loops). | Must | P2 to P4 | Modulation and randomness are how living systems move. As modules, every modulator connects like anything else. |
| FR-LIB-13 | Note and MIDI effects as modules: arpeggiator, strum, echo, harmonise, humanise, randomise, latch, note length, note repeats, quantise, transpose and transpose maps, velocity curve, multi-note, and novel ones such as bouncing-ball and stepwise generators. | Should | P3 to P4 | Note effects are central to generative and live work. Depends on how notes travel on cables (OQ-16). |
| FR-LIB-14 | Sequencers as modules, including polymetric multi-output step sequencers: several lanes of different lengths, rates, and directions in one module, each lane with its own pitch, gate, and modulation outputs. Patterns are stored as module data. | Must | P3 | Polymetric sequencing is a core way to build evolving patterns. Several outputs per module let one sequencer drive notes and modulation at once. |
| FR-LIB-15 | Device-style processors as modules: EQ, dynamics (compressor, limiter, gate, transient control), reverb, delays, modulation effects (chorus, flanger, phaser, tremolo), distortion, filters (comb, resonator bank), pitch and frequency shifting, spectral splitting, sampler, granular. | Should | P2 to P4 | Users expect finished-sounding tools, not only primitives. Some are built from primitives, others as optimised single modules. |
| FR-LIB-16 | Modulation curves that run over bars or sections, built on the P1 transport and clock modules (FR-LIB-25 to 27), so patches can shape themselves over time. There is no arranger timeline. | Must | P3 | Arrangement happens inside the grid (ADR-0011). In plugins, the host transport drives the clock. |
| FR-LIB-17 | Display modules: oscilloscope, spectrum, level meter, XY scope, value readout, drawn live inside the module on the canvas. | Should | P1 to P2 | Seeing signals is part of patching. Bitwig's Grid shows small live displays inside modules. |
| FR-LIB-18 | A module declares its panel as data: controls bound to parameters, and displays (scope, light, step grid, Euclidean ring, envelope shape, looper waveform) fed by engine telemetry, display values, or module data. Uses the same widgets and format as front panels (ADR-0013). | Must | P1 | Rich module views are what make the grid feel like an instrument. Infrastructure first (P1), individual views follow their modules. |
| FR-LIB-19 | Modules can store data beyond parameters (patterns, loops, wavetables, samples). It is saved in the patch, covered by undo, and handed to the engine without breaking real-time rules. | Must | P2 | Sequencers, loopers, and samplers cannot work without it (OQ-17). |
| FR-LIB-20 | Note and audio cycles (loopers) as modules: they read the clock's bar phase, play a stored pattern or loop that is a set number of bars long, start from their beginning or join in sync when their play gate rises, and output an end trigger when their loop ends (while playing). Recording, overdub, and clear come with OQ-15. | Must | P3 | Phase-driven loops stay in sync and change tempo together, and their end triggers drive arrangement logic (ADR-0012). |
| FR-LIB-21 | Groups and next actions as built-in abstractions made from primitives (ADR-0012): a Looper Group rotates through its cycles (round robin, random, or by input), with N loops each, and outputs a trigger when a round completes. Users can open and change it. | Must | P3 | Like the clip launcher's next actions, but patchable and open, so any rule can be built. |
| FR-LIB-22 | Counters and arrangement logic as modules: Counter (counts triggers and outputs a phase, so step sequencers can follow it), Step Select (one gate high per step, with an enable input), comparators, and probability gates. They work with Clock Divide (FR-LIB-27, P1). Example: play group A round robin, each loop twice; after two rounds, play group B the same way. | Must | P3 | This form of arrangement is not possible in a timeline or a fixed clip launcher. It is what makes LoomDSP grid-centred from the start (ADR-0012). The logic modules are small and can be built in P1. |
| FR-LIB-23 | Scenes as modules: a scene stores the state of a chosen set of modules (parameters, active groups, sequencer patterns) and recalls it on a trigger, with an optional morph time. | Should | P3 | Scenes change sections of a piece in one move, and logic can trigger them. |
| FR-LIB-24 | Built-in abstractions ship as open patches made from primitives (for example Looper Group, Part, Song Sequencer). They work as one module, can be opened to learn from or change, and use the same format as user abstractions. | Must | P3 | Keeps users close to their musical ideas without hiding how things work. Users' own saved abstractions grow the same library (ADR-0012). |
| FR-LIB-25 | One transport per patch: tempo, time signature, run state, and position, from the app in P1; MIDI clock in P2 (FR-MIDI-07) and a plugin host with export (FR-NATIVE-*) use the same transport (ADR-0015). Its controls (Run, Reset, tempo) live on one Transport module, with run and reset inputs. | Must | P1 | Every synced module must agree on where the bar starts. |
| FR-LIB-26 | Inputs can default to transport signals (bar phase, beat phase, playing) while unconnected. A cable overrides the default. | Must | P1 | Synced modules work without clock cables, and time can still be bent by patching. |
| FR-LIB-27 | Clock modules: Transport (the patch's one transport, with Run and Reset on its panel and as inputs; bar and beat phase, play gate, triggers), Button (toggle or momentary gate), Clock (phase and trigger at musical rates), Clock Divide (triggers), Phase Scale (multiply and divide a phase, locked), and Phase Trigger (a trigger on each phase wrap). | Must | P1 | Puts time on cables for patching, counting, and polyrhythm. |
| FR-LIB-28 | Generators as modules: melodies and rhythms from a chosen, seeded algorithm (random notes in a scale, a random walk, a looping pattern that slowly mutates, Markov chains, and rule systems such as L-systems and cellular automata), following the patch scale (MT-SEQ-04). | Should | P3 | Living systems need material that varies but repeats when the patch loads (NFR-SND-04). |
| FR-LIB-29 | A patch-wide scale and tuning (equal divisions, just ratios, imported tuning files) that pitch quantisers, sequencers, generators, and note effects follow unless overridden, and that a trigger can change live (MT-PITCH-03, trait TR-08). | Should | P2 | One place to change key keeps every part of a patch in tune together. |

## 6. Encapsulation (`ENCAP`)

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-ENCAP-01 | Any selection of modules can be collapsed into a sub-patch. Connections crossing the boundary become ports. | Must | P2 | Main tool for managing complexity. |
| FR-ENCAP-02 | Sub-patches can nest to any depth. | Must | P2 | Real instruments have several levels (voice, oscillator, filter). |
| FR-ENCAP-03 | A sub-patch can expose internal parameters as its own parameters, with a new name and range. | Must | P2 | Lets a sub-patch behave like a built-in module. |
| FR-ENCAP-04 | Users can open, inspect, and edit any sub-patch. | Must | P2 | Learning and debugging. |
| FR-ENCAP-05 | A sub-patch can be saved to the user library and reused in other patches. | Must | P2 | Reuse is the point of encapsulation. |
| FR-ENCAP-06 | Library sub-patches can be linked (updates propagate) or copied (independent). The user chooses. | Should | P2 | Both workflows are common. |
| FR-ENCAP-07 | Encapsulation has no runtime cost. The engine flattens sub-patches before execution. | Must | P2 | Users should not trade clarity for CPU. |
| FR-ENCAP-08 | An abstraction can mark a place as a slot for a role (ADR-0017). The user swaps what fills the slot from the tile or panel. Cables stay connected, values carry across by role parameter name, and values the new part does not have stay stored, so swapping back restores them. The swap is click-free. | Must | P2 | Familiar devices become open and adaptable without new DSP: change the filter, keep the rest. |
| FR-ENCAP-09 | Any sub-patch, including a user's own, can declare the roles it fills, so it can fill matching slots in any abstraction. | Should | P2 | The community extends abstractions without writing code. |
| FR-ENCAP-10 | Sub-patch modes: single; layer (all members run and are mixed); select (one member runs, chosen by a control); stack (N copies, each with its own spread values). | Should | P2 | Layers, switchable sections, and unison are one feature for every kind of module, not a feature of each oscillator. |

## 7. Patching canvas (`CANVAS`)

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-CANVAS-01 | A zoomable, pannable canvas where users add, move, and delete modules. | Must | P1 | Basic interaction. |
| FR-CANVAS-02 | Connect ports by dragging. Cables are coloured by signal type. | Must | P1 | Basic interaction. |
| FR-CANVAS-03 | Module browser with search and categories. Add modules by typing a name at the cursor. | Must | P1 | Fast patching for experienced users. |
| FR-CANVAS-04 | Undo and redo for every edit, including parameter changes. | Must | P1 | Users experiment. Mistakes must be cheap. |
| FR-CANVAS-05 | Copy, paste, and duplicate, including across patches. | Must | P1 | Basic editing. |
| FR-CANVAS-06 | Keyboard navigation and shortcuts for common actions. | Should | P1 | Speed, and part of accessibility. |
| FR-CANVAS-07 | Comments and colour groups on the canvas. | Should | P2 | Documentation inside the patch. |
| FR-CANVAS-08 | Insert a module onto an existing cable by dropping it there. | Could | P2 | Common workflow in Bitwig Grid. |
| FR-CANVAS-09 | Our own canvas widget (ADR-0010): compact module tiles on a snap grid, ports on tile edges, signal flowing left to right. | Must | P1 | The canvas is the centre of the product. Its look and speed must be ours to control. |
| FR-CANVAS-10 | Each tile shows its module's panel: controls and displays, edited directly on the canvas (ADR-0013). An inspector shows every parameter of the selected module, for exact values and settings that do not fit on the tile. | Must | P1 | Direct manipulation: users edit things where they see them, as in Bitwig's Grid and Eurorack. |
| FR-CANVAS-11 | Level of detail: when zoomed out, modules draw as simple tiles without text or live views. | Should | P1 | Large patches stay fast and readable. |
| FR-CANVAS-12 | Cable styles show meaning: colour by signal type, thicker for stereo and polyphonic signals, a visible mark where the engine inserted a 1-sample delay. | Should | P2 | Users should read signal flow from the cables alone. |

## 8. Visual feedback (`VIS`)

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-VIS-01 | Ports show whether they carry signal, as a steady state updated a few times a second. Cables do not pulse or change brightness with the signal. | Should | P1 | Shows where signal flows without constant motion, which tires the eyes and hides what matters. |
| FR-VIS-02 | A scope module, plus a one-step "Add scope" on any cable or output that places one already connected. | Must | P1 | Core debugging tool. As a module it saves with the patch and can go on a front panel (ADR-0013). |
| FR-VIS-03 | Spectrum analyser and level meter modules. | Should | P2 | Common sound design need. |
| FR-VIS-04 | Visual data reaches the UI without blocking the audio thread and without allocation. | Must | P1 | See NFR-RT-01. |
| FR-VIS-05 | The app estimates the range each port receives or sends, from cables, parameters, and nominal ranges, and shows it in the inspector. It warns about likely mistakes, such as a gate input that never goes above 0.5, without blocking anything. | Could | P2 | Shows what a cable carries without a scope, and teaches why a patch misbehaves ([port ranges](../design/port-ranges.md)). |

## 9. Front panels (`PANEL`)

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-PANEL-01 | Each patch and sub-patch can have a front panel, separate from the cable view, designed with the same editor. | Must | P2 | Hides complexity. Core of performance mode. |
| FR-PANEL-02 | Panel widgets: knob, horizontal/vertical slider, toggle, radio buttons, button, selector, XY pad, label, light, meter, scope, step grid, graph. | Must | P2 | Covers common instrument controls. |
| FR-PANEL-03 | Widgets bind to exposed parameters. One parameter can have several widgets. | Must | P2 | Flexible layout. |
| FR-PANEL-04 | Widgets support value mappings: linear, logarithmic frequency, decibels, stepped/enumerated. | Must | P2 | Controls must feel right. A linear cutoff knob is unusable. |
| FR-PANEL-05 | Panel editor with grid snapping, alignment, grouping, and resizing. | Must | P2 | Panels must look tidy without pixel-pushing. |
| FR-PANEL-06 | Themes: colours, fonts, and widget styles. Light and dark defaults. | Should | P2 | Stage visibility and branding for exports. |
| FR-PANEL-07 | Vector rendering that scales cleanly on high-DPI screens. Custom images (SVG, PNG) as backgrounds or widget skins. | Should | P2 | Professional look. |
| FR-PANEL-08 | A sub-patch panel can be embedded in its parent panel. | Should | P2 | Panels compose the same way patches do. |
| FR-PANEL-09 | One panel model: module tiles, sub-patch tiles, and patch front panels use the same widgets, the same layout format, and the same panel editor. | Must | P2 | Users learn one vocabulary. Abstraction feels the same at every level (ADR-0013). |
| FR-PANEL-10 | A sub-patch's front panel is what its tile shows in the parent patch. | Must | P2 | A finished sub-patch looks and works like a built-in module. |
| FR-PANEL-11 | A widget can bind to a slot's role parameter, so it keeps working after the slot's contents change. Parameters only one part has appear in a sub-panel that follows the slot. | Must | P2 | A Cutoff knob on a front panel should not break when the user tries a different filter (ADR-0017). |

## 10. Live performance (`PERF`)

Live performance is the top priority after the core patcher works.

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-PERF-01 | Performance mode: shows only the panel, locks editing, uses large controls. | Must | P2 | Prevents accidental edits on stage. |
| FR-PERF-02 | Presets (snapshots of parameter values) with click-free switching. | Must | P2 | Song changes during a set. |
| FR-PERF-03 | Preset morphing between two snapshots. | Could | P4 | Expressive performance tool. |
| FR-PERF-04 | CPU load meter and dropout counter visible at all times. | Must | P1 | Performers need warning before problems become audible. |
| FR-PERF-05 | A "panic" action silences all voices and resets stuck notes immediately. Mappable to a MIDI control. | Must | P1 | Stuck notes happen on stage. Recovery must be one press. |
| FR-PERF-06 | A set list: an ordered list of patches and presets. Switching between them is click-free and loads in the background. | Should | P2 | A live set uses several instruments. |
| FR-PERF-07 | Several patches can run at once (layers or splits across the keyboard). | Should | P2 | Common live setup, for example bass on the left hand and pad on the right. |

## 11. MIDI and control input (`MIDI`)

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-MIDI-01 | Receive MIDI notes, CC, pitch bend, and aftertouch from hardware (standalone) or host (plugin). | Must | P1 | Basic instrument need. |
| FR-MIDI-02 | MIDI learn for any parameter: touch a control, move a knob, done. | Must | P1 | Fast controller mapping is central to live use. |
| FR-MIDI-03 | MPE support. | Should | P2 | Expressive controllers are common among the target audience. |
| FR-MIDI-04 | Send MIDI out from a patch (notes, CC, clock). | Should | P2 | Live rigs drive other gear from sequencers. |
| FR-MIDI-05 | OSC input. | Could | P4 | Common in live coding and installation work. |
| FR-MIDI-06 | Controller mappings are saved per device, separate from patches, and restored when the device connects. | Must | P2 | The same controller is used across many patches. Remapping every time is slow. |
| FR-MIDI-07 | Sync tempo to incoming MIDI clock, and send MIDI clock. | Should | P2 | Playing with drum machines and other performers. |
| FR-MIDI-08 | MIDI devices can connect and disconnect while the app runs, without a restart. | Must | P1 | Cables get unplugged on stage. |
| FR-MIDI-09 | Mapping modes for relative encoders, value pickup (soft takeover), and value scaling/inversion. | Should | P2 | Prevents parameter jumps when a knob position does not match the current value. |

## 12. Desktop application (`APP`)

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-APP-01 | Desktop app for macOS, Windows, and Linux with audio device and MIDI device selection. | Must | P1 | The main product. |
| FR-APP-02 | Browser version of the app, with patching, example patches, and sub-patch inspection. | Could | Later | Lets educators share a link so students can patch with no install (Sanna, the educator). Pursue only if the UI framework and WASM build perform well in browsers. |
| FR-APP-05 | Open a patch from a URL in the browser version. | Could | Later | Teachers share example patches as links. |
| FR-APP-03 | Audio input processing (use LoomDSP as a live effect). | Must | P1 | Effects are half of the use cases. |
| FR-APP-04 | Audio device loss (unplugged interface) is handled without a crash. The app shows a clear message and can reconnect. | Must | P1 | Hardware problems happen on stage. |

## 13. Files and patch format (`FILE`)

See ADR-0008.

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-FILE-01 | Patches save as RON (Rusty Object Notation), a human-readable text format. | Must | P0 | Keeps Rust enums and structs exact through Serde. Works with git. |
| FR-FILE-02 | The format has a version number. Older patches load in newer versions through migrations. | Must | P0 | Users must not lose work after upgrades. Applies from the first release. During the early alpha the format stays at version 1 and old patches may stop loading. |
| FR-FILE-03 | The format is documented. | Should | P2 | Enables third-party tools. |
| FR-FILE-04 | Patches reference modules by stable ID and version, not by display name. | Must | P0 | Renaming a module must not break patches. A display name can change at any time. During the early alpha the ID itself may also change, and then old patches may stop loading. |
| FR-FILE-05 | Loading a patch with an unknown module shows a placeholder and keeps the data. | Should | P1 | Graceful handling of missing modules from newer versions. |
| FR-FILE-06 | A patch can embed assets (wavetables, samples, panel images) or reference them. | Should | P2 | Exported plugins and shared patches must be self-contained. |
| FR-FILE-07 | A JSON export of the patch format, for tools outside the Rust world. | Could | P4 | Only if third-party tools ask for it. |

## 14. Native export (`NATIVE`), deferred

Not scheduled (phase "Later"). LoomDSP first focuses on generative live performance and sharing patches, as VCV Rack does. Plugin and standalone export will be revisited after living systems (P3) are proven. The requirements below record the intent for when they are.

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-NATIVE-01 | Export a patch as a CLAP instrument or effect for macOS, Windows, and Linux. | Must | Later | CLAP is an open, permissively licensed standard. |
| FR-NATIVE-02 | Export a patch as a VST3 plugin. | Must | Later | VST3 has the widest DAW support. The SDK is MIT-licensed since 3.8. We build our own adapter (FR-SDK-06). |
| FR-NATIVE-03 | Exported plugins expose parameters, report latency, support state save/restore, and receive MIDI. | Must | Later | Basic plugin contract. |
| FR-NATIVE-04 | Exported plugins show the patch front panel as their GUI. | Must | Later | The panel is the product. |
| FR-NATIVE-05 | Export does not require a Rust toolchain on the user's machine. | Must | Later | Most users are not developers. See ADR-0004. |
| FR-NATIVE-06 | A LoomDSP host plugin can load any patch inside a DAW. | Should | Later | Lets users patch directly inside their DAW. |
| FR-NATIVE-07 | AU (Audio Unit) export. | Won't (now) | | Possible later via a CLAP-to-AU wrapper. |
| FR-NATIVE-08 | Export a patch as a standalone app that opens straight into its front panel. | Should | Later | Lets performers use an instrument without learning LoomDSP. |
| FR-NATIVE-09 | Exported plugins include the Apache-2.0 licence and NOTICE for the runtime. The patch author chooses the licence for their patch. | Must | Later | Meets Apache-2.0 attribution terms. Patch authors keep their rights. |

## 15. Web export (`WAM`), optional

Only pursued if the UI and WASM build perform well in browsers (deferred; see the roadmap and ADR-0003).

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-WAM-01 | Export a patch as a WAM2 package: descriptor, JS glue, AudioWorklet processor, WASM engine, patch data, and panel GUI. | Could | Later | Opens the browser plugin ecosystem. |
| FR-WAM-02 | The whole patch runs inside one AudioWorkletProcessor and one WASM instance. | Must (if web) | Later | Separate Web Audio nodes add 128-sample delays and break feedback. |
| FR-WAM-03 | Exposed panel parameters become WAM2 parameters. MIDI and automation events are sample-accurate. State save/restore works. | Must (if web) | Later | Basic WAM2 contract. |
| FR-WAM-04 | Works without cross-origin isolation, with reduced visual update quality. | Should (if web) | Later | Many hosts cannot set COOP/COEP headers. |

## 16. Sharing (`SHARE`)

The sharing model follows VCV Rack: patches are files that people swap, and an ecosystem of modules grows around the core. No accounts or hosted marketplace in v1 (see the vision's non-goals).

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-SHARE-01 | A patch can be saved as one self-contained file that includes its assets (loops, samples, wavetables, panel images). | Must | P4 | Sharing a living system means sharing its recorded loops too. One file is easy to send, post, and archive. |
| FR-SHARE-02 | Opening a patch that uses modules this build does not have lists the missing modules, with their version and where they come from, and keeps their data (FR-FILE-05). | Must | P4 | Shared patches will travel between people with different module sets. |
| FR-SHARE-03 | Record the live output to an audio file while performing. | Should | P3 | Sharing results matters as much as sharing patches. Recording must not risk the audio thread. |
| FR-SHARE-04 | Browse and open example and community patches from inside the app. The library is a public git repository, so there are no accounts. | Could | P4 | Lowers the barrier to learning from others. A git repository is free to host and easy to contribute to. |
| FR-SHARE-05 | Third-party modules can be distributed and installed without rebuilding LoomDSP from source. | Should | P4 | VCV Rack's ecosystem depends on this. How to do it safely is open (OQ-18). |

## 17. Module SDK and published crates (`SDK`)

| ID | Requirement | Priority | Phase | Rationale |
|---|---|---|---|---|
| FR-SDK-01 | A public Rust trait for modules: describe ports and parameters, process a block, process one sample, reset, report latency. | Must | P0 | Built-in modules use the same API. This keeps it honest. |
| FR-SDK-02 | A test harness: render a module with given inputs and compare to reference output. | Must | P1 | Quality and regression safety. |
| FR-SDK-03 | Module template, examples, and contributor guide. | Must | P4 | Lowers the barrier for module developers. |
| FR-SDK-04 | Third-party modules can be compiled into the main build from source, at least until OQ-18 is decided. | Must | P4 | The simplest safe and portable way to start an ecosystem. |
| FR-SDK-05 | Import Faust DSP code as a module. | Could | P4 | Large existing library of DSP code. Check Faust's licence terms for generated code first. |
| FR-SDK-06 | Reusable primitives are separate crates with their own docs and semver, and are published to crates.io when stable. Candidates: real-time queues, VST3 adapter, CLAP adapter, plugin GUI embedding, node-graph widget extensions. | Should | Later | Gives back to the Rust audio ecosystem and gives contributors smaller, self-contained projects. |
| FR-SDK-07 | Traits: shared parameter sets and behaviour (such as timebase, retrigger, seed, spread, character, and smoothing) are defined once in the SDK and declared by modules (module catalogue, TR-01 to TR-11). | Should | P2 | Users learn an option once, it looks the same everywhere, and a fix reaches every module that uses it (ADR-0017). |
| FR-SDK-08 | Modules and sub-patches can declare the roles they fill. The registry checks that they have the ports and parameters each role requires. | Must | P2 | Slots can only be trusted if every part that claims a role really fits it (ADR-0017). |
