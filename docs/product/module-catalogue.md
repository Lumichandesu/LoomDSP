# Module Catalogue

The module catalogue lists every kind of module LoomDSP plans to have, in our own words. It is the plan for the module library: what to build, in which phase, and for which requirement. It also lists the shared **traits** that many modules use, and the **roles** that let parts of an abstraction be swapped.

Status: **Draft** (2026-10-17). Open for discussion.

- **Decision behind it:** [ADR-0017](../adr/0017-primitives-traits-and-abstractions.md) (primitives, traits, and open abstractions).
- **How it was written:** [ADR-0018](../adr/0018-clean-room-module-development.md) (clean-room development).
- **Evidence it rests on:** the [prior-art survey](../research/2026-10-prior-art-survey.md), a dated snapshot of 15 other environments.

## How to read it

Each entry is a **module type**, not a module ID. A type says what a module does. A built module has a released ID such as `loom.osc.sine`, and released IDs never change (AGENTS.md, hard rule 5).

- One type can be built as more than one module. For example, the Oscillator type is built today as `loom.osc.sine` and `loom.osc.saw`. A cheap special-purpose module is often worth keeping next to a general one.
- **Catalogue IDs** look like `MT-FILTER-02` (MT for "module type"). They are stable, like requirement IDs: if an entry is dropped, mark it **Withdrawn** instead of reusing the ID. Issues say "Implements MT-FILTER-02", so the link survives renames.
- **Primitive** entries are modules with one job. **Abstraction** entries are open patches made from primitives (FR-LIB-24): they work as one module, and you can open and change them.
- **Traits** are shared options, listed by name. They are defined in [Traits](#traits).
- **Phase** follows the use case the type unblocks: P1 for the subtractive tutorial (UC1), P2 for panels, sub-patches, and live play (UC2, UC3), P3 for self-arranging sets (UC8), P4 for the rest.
- **Req.** links to the requirement that covers the type, or says **New** when no requirement covers it yet.
- **Status** is **Built** (with its module IDs), **Built in part** (some of the described behaviour is built; the status names what is missing), **Specified** (in a design specification), or **Planned**.

Why types and not modules: a plan written as module IDs would fix names and port lists before anyone designs them. A type fixes only the job, so the design stays open until its specification is written.

## Changing the catalogue

- Add or change an entry in a pull request, in the same style: a plain name and one sentence on what the module does.
- Write descriptions from first principles. Do not copy or paraphrase another product's documentation (ADR-0018).
- Before adding a type, check whether an existing type, a trait, or an abstraction already covers it. The merge rule is in [ADR-0017](../adr/0017-primitives-traits-and-abstractions.md#decision-proposed).
- When a type is built, update its status with the module IDs.

---

## Time, transport, and phase

Timing is phase-driven: a ramp from 0 to 1 sets where in a cycle a module is ([Phase and cycles](../learn/phase-and-cycles.md)).

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-CLOCK-01 | Transport | The patch's one transport: run, reset, and tempo on its panel; bar phase, beat phase, play gate, and bar and beat triggers as outputs. | | P1 | FR-LIB-27 | Specified (`loom.clock.transport`) |
| MT-CLOCK-02 | Clock | A phase, trigger, and gate at musical rates. Later: several outputs, each with its own division, multiplication, and swing. | Timebase | P1 | FR-LIB-27 | Specified (`loom.clock.clock`) |
| MT-CLOCK-03 | Clock divide | Fires on every Nth incoming trigger, with an offset. | | P1 | FR-LIB-27 | Built (`loom.logic.divide`) |
| MT-CLOCK-04 | Quantise to clock | Holds a trigger until the next clock pulse or bar line, so launches land on time. | | P3 | New | Planned |
| MT-CLOCK-05 | Start and reset hub | Sends one start, stop, and reset to many modules, with an optional delay after a patch loads. | | P3 | New | Planned |
| MT-PHASE-01 | Phasor | A rising 0 to 1 ramp at a set rate or musical division. | Timebase, Retrigger | P1 | FR-LIB-01 | Built in part (`loom.phase.generator`: rate set by pitch; musical divisions to come) |
| MT-PHASE-02 | Phase scale | Runs a phase faster or slower by a whole-number ratio, and stays locked to it. | | P1 | FR-LIB-27 | Specified (`loom.phase.scale`) |
| MT-PHASE-03 | Phase trigger | Fires a trigger each time a phase wraps. | | P1 | FR-LIB-27 | Built (`loom.phase.trigger`) |
| MT-PHASE-04 | Phase reset | Restarts a phase at zero on a trigger. | | P1 | FR-LIB-01 | Planned |
| MT-PHASE-05 | Phase warp | Changes how a phase moves within each cycle without changing its period: offset, bend, S-curve, skew, reverse, mirror, and smoothing that keeps the wrap point. | | P2 | FR-LIB-01 | Planned |
| MT-PHASE-06 | Phase split | Divides one cycle into N parts and outputs a phase for each part. | | P3 | New | Planned |
| MT-PHASE-08 | Phase wrap and fold | Brings any signal into the 0 to 1 phase range, by wrapping it around or by folding it back at the edges. | | P1 | FR-LIB-01 | Planned |
| MT-PHASE-07 | Span phase | A phase that covers N bars and is anchored to bar 1, so it has the same value at the same musical time after any jump. | | P3 | New | Planned |

## Sound sources

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-SRC-01 | Oscillator | A band-limited periodic wave with pitch, phase, and frequency modulation inputs, hard sync, and a sub output. The shape comes from the shape source trait. Can be read by an external phase, so it also works as a lookup table. | Shape source, Timebase, Retrigger, Quality | P1 | FR-LIB-02 | Built in part (`loom.osc.sine`, `loom.osc.saw`) |
| MT-SRC-02 | Noise | White, pink, and other noise colours. | Seed | P1 | FR-LIB-02 | Built in part (`loom.osc.noise`: white noise only) |
| MT-SRC-03 | Additive oscillator | Builds a tone from many sine partials, with level control per partial and per group. | Quality | P4 | New | Planned |
| MT-SRC-04 | Chaotic source | Signals from chaotic systems, such as iterated maps, attractors, and stochastic waveforms. | Seed | P4 | New | Planned |
| MT-SRC-05 | Swept sine | Sweeps a sine between two frequencies, for tests, risers, and effects. | Timebase | P4 | New | Planned |
| MT-SRC-06 | Modal resonator | A bank of tuned resonant modes that rings like a bar, plate, or membrane when excited. | Character | P4 | FR-LIB-15 | Planned |
| MT-SRC-07 | FM operator (abstraction) | An oscillator with a phase modulation input, its own envelope, and its own level, for building FM voices. | | P2 | New | Planned |
| MT-SRC-08 | Synth voice (abstraction) | A subtractive voice with slots for oscillators, a filter, an amplitude envelope, and a modulation envelope. Stackable for unison. | | P2 | FR-ENCAP-08 | Planned |
| MT-SRC-09 | Drum voice (abstraction) | One template for kick, snare, hat, and percussion: slots for an exciter (oscillator, noise, or sample), a shaper, a filter, and an envelope. | | P3 | New | Planned |
| MT-SRC-10 | String model (abstraction) | A plucked or bowed string from a tuned delay line, a filter, and feedback. | | P4 | New | Planned |
| MT-SRC-11 | Endless glissando (abstraction) | A tone that seems to rise or fall forever, from octave-spaced partials with level shaped by position. | | P4 | New | Planned |
| MT-SRC-12 | Cross-modulated pair (abstraction) | Two oscillators that sync, frequency-modulate, and ring-modulate each other. | | P4 | New | Planned |

## Samples, buffers, and loops

Sampling is built from small parts, so one set of primitives covers one-shot playback, multi-samples, slicing, and granular sound.

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-SAMPLE-01 | Buffer | Holds audio in memory. It can be loaded from a file, recorded into, and shared by readers. Saved with the patch or referenced. | | P2 | FR-LIB-19 | Planned |
| MT-SAMPLE-02 | Buffer reader | Reads a buffer by phase or by position, at a pitch, with loop points and a crossfade at the loop. | Timebase, Retrigger, Quality | P2 | FR-LIB-15 | Planned |
| MT-SAMPLE-03 | Buffer writer | Records a signal into a buffer, with overdub and clear, into memory allocated before recording starts. | | P3 | FR-LIB-20 | Planned |
| MT-SAMPLE-04 | Slice map | Turns a slice number into the start and end of a slice, so a reader plays one slice per trigger. | | P3 | New | Planned |
| MT-SAMPLE-05 | Zone map | Picks a buffer by pitch and velocity range, for multi-sampled instruments. | | P4 | FR-LIB-15 | Planned |
| MT-SAMPLE-06 | Grain engine | Plays many short, overlapping, windowed reads of one buffer, with control over size, position, density, spread, and pitch. Reading a buffer that is still recording gives live granulation. | Seed, Quality | P4 | FR-LIB-15 | Planned |
| MT-SAMPLE-07 | Time stretch | Changes speed and pitch of a buffer independently. | Quality | P4 | New | Planned |
| MT-SAMPLE-08 | File streamer | Streams a long audio file from disk in step with the transport, through a buffer filled on a control thread. | | P4 | New | Planned |
| MT-SAMPLE-09 | Recorder | Captures a signal to a file while it passes through, without risk to the audio thread. | | P3 | FR-SHARE-03 | Planned |
| MT-SAMPLE-10 | Sampler (abstraction) | A playable sampler with slots for a zone or slice map, a reader or grain engine, a filter, and an envelope. | | P4 | FR-LIB-15 | Planned |

## Filters

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-FILTER-01 | State variable filter | A resonant filter with low-pass, high-pass, band-pass, and notch outputs at once. | Character | P1 | FR-LIB-03 | Built in part (`loom.filter.svf`: no notch output yet) |
| MT-FILTER-02 | Ladder low-pass | A four-pole resonant low-pass with saturation in its feedback path. | Character, Quality | P1 | FR-LIB-03 | Planned |
| MT-FILTER-03 | One-pole filter | First-order low-pass and high-pass, for smoothing and gentle tone control. | | P1 | FR-LIB-03 | Planned |
| MT-FILTER-04 | Slope filter | Low-pass, high-pass, or all-pass with a selectable steepness. | | P2 | FR-LIB-03 | Planned |
| MT-FILTER-05 | Comb filter | A tuned delay with feedback that boosts or cuts a harmonic series. | | P2 | FR-LIB-15 | Planned |
| MT-FILTER-06 | Low-pass gate | A filter and an amplifier closed together by one control, for plucked and percussive tones. | Character | P2 | New | Planned |
| MT-FILTER-07 | Crossover | Splits a signal into two or three frequency bands that sum back to the original. | | P2 | New | Planned |
| MT-FILTER-08 | Formant filter | Shapes a sound toward vowel sounds, with a morph between vowels. | | P4 | New | Planned |
| MT-FILTER-09 | Filter bank | Many band-pass filters in parallel, fixed or tuned. With high resonance it is a resonator bank. | | P4 | FR-LIB-15 | Planned |
| MT-FILTER-10 | Hilbert pair | Splits a signal into two outputs 90 degrees apart, for frequency shifting and envelope detection. | Quality | P4 | New | Planned |
| MT-FILTER-11 | Biquad | A general two-pole, two-zero filter set by coefficients, for building custom filters in a patch. | | P4 | New | Planned |

## Shapers and distortion

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-SHAPE-01 | Shaper | Bends a signal through a transfer curve to add harmonics. Curves include soft and hard clipping, saturation, folding, rectifying, polynomial curves that pick a harmonic of a sine, and a user-drawn curve. | Character, Quality, Mix and output | P2 | FR-LIB-09 | Planned |
| MT-SHAPE-02 | Bit and rate reducer | Lowers bit depth and sample rate on purpose, for digital grit. | Mix and output | P2 | New | Planned |
| MT-SHAPE-03 | Ring and amplitude modulator | Multiplies two signals, with a blend from dry through amplitude modulation to ring modulation. | | P2 | New | Planned |
| MT-SHAPE-04 | Subharmonic generator | Derives tones at whole-number fractions of the input frequency. | | P4 | New | Planned |

## Level, mixing, and routing

Modulation assignments already scale and offset any parameter (ADR-0014), so these modules are for shaping signals that feed several places or a switch.

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-LEVEL-01 | Constant | Outputs a fixed value in a chosen unit: level, decibels, pitch, ratio, or plain number. | Smoothing | P1 | FR-LIB-05 | Built in part (`loom.math.constant`: plain number only, no unit choice) |
| MT-LEVEL-02 | Amplifier | Scales a signal by a control signal, with linear or exponential response. | | P1 | FR-LIB-05 | Planned |
| MT-LEVEL-03 | Scale | Multiplies by a factor from -1 to +1 (so it can invert) and adds an offset, or maps one range onto another, including bipolar to unipolar and back. | | P1 | FR-LIB-05 | Planned |
| MT-LEVEL-04 | Slew | Limits how fast a signal can rise or fall, with separate times for each direction. | | P1 | FR-LIB-04 | Planned |
| MT-MIX-01 | Mixer | Sums several inputs, with level, pan, and mute per channel. At unity gain it is a quick merge. | | P1 | FR-LIB-05 | Planned |
| MT-MIX-02 | Crossfade | Blends two signals by a control, with an equal-power or linear law. | | P1 | FR-LIB-05 | Planned |
| MT-MIX-03 | Stereo tool | Pans, sets width, converts between left/right and mid/side, and splits or joins channels. | | P2 | New | Planned |
| MT-ROUTE-01 | Selector | Picks one of N inputs. It steps to the next on a trigger, jumps to the input a control value points at, or picks at random. | Seed, Smoothing | P2 | New | Planned |
| MT-ROUTE-02 | Distributor | Sends one input to one of N outputs, with the same three ways to choose. | Seed | P2 | New | Planned |
| MT-ROUTE-03 | Mute | Silences a signal on a button or gate, with a short fade so it never clicks. | | P2 | New | Planned |
| MT-ROUTE-04 | Matrix | Connects N inputs to M outputs with a level and a mute at each crossing. Presets cover a plain multiple, a mutable multiple, and a router that rotates by one place on each trigger. | | P3 | New | Planned |
| MT-ROUTE-05 | Send and receive | Passes a signal by name without a cable. Needs a decision first, because hidden routing makes patches harder to read (ADR-0014 weighed the same risk). | | P4 | New | Planned |

## Voices

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-VOICE-01 | Voice merge | Turns a polyphonic signal into one signal: sum, average, last voice, lowest, or highest. | | P2 | FR-VOICE-03 | Planned |
| MT-VOICE-02 | Voice split and fill | Separates the voices of a polyphonic signal, sets a fixed value per voice, or copies one signal to every voice. | | P2 | FR-VOICE-03 | Planned |
| MT-VOICE-03 | Voice spread | Gives each voice a different value from one control, by an even spread, a series, or a seeded random set. | Spread, Seed | P2 | New | Planned |
| MT-VOICE-04 | Voice assigner | Redistributes notes across voices, or doubles them into unison voices. | | P2 | FR-VOICE-02 | Planned |

## Envelopes and modulation sources

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-ENV-01 | Envelope | Shapes a level from a gate or trigger: AD, AR, ADSR, and ADSR with delay and hold, with curve control and an optional loop. The character trait gives plucked and slow, lagging responses. | Timebase, Character | P1 | FR-LIB-04 | Built in part (`loom.env.adsr`) |
| MT-ENV-02 | Segment envelope | An envelope drawn as any number of points, with a hold point or a loop between two points. Uses the shared curve data. | Timebase, Retrigger | P2 | FR-LIB-12 | Planned |
| MT-ENV-03 | Function generator | A rise and a fall that can be triggered once, looped, or used to slew an input. | Timebase | P2 | New | Planned |
| MT-ENV-04 | Envelope follower | Outputs the loudness of an audio signal as a control signal. | Sidechain | P2 | FR-LIB-12 | Planned |
| MT-ENV-05 | Slope detector | Outputs whether a signal is rising, falling, or still. | | P4 | New | Planned |
| MT-MOD-01 | LFO | A slow periodic signal, free or synced, with extra outputs at fixed phase offsets. The shape comes from the shape source trait. | Shape source, Timebase, Retrigger, Spread | P1 | FR-LIB-04 | Planned |
| MT-MOD-02 | Curve reader | Reads the shared curve data in one of three ways: by phase (it repeats), by time over seconds or bars (it runs once), or by value (it maps an input to an output). This one type covers drawn LFOs, section curves, velocity curves, and keytracking curves. | Timebase, Retrigger | P2 | FR-LIB-12, FR-LIB-16 | Planned |
| MT-MOD-03 | Note values | Turns note information into control signals: keytracking, note count, and one gate per pitch class. | | P2 | FR-LIB-12 | Planned |
| MT-MOD-04 | Vibrato (abstraction) | An LFO with a delay and a fade-in, scaled by mod wheel or pressure. | | P2 | FR-LIB-12 | Planned |

## Randomness

All random modules take a seed, so a patch sounds the same each time it loads (NFR-SND-04).

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-RAND-01 | Random | A new random value on each trigger, within a range. In walk mode it moves by small steps instead, in one or two dimensions. | Seed, Smoothing | P2 | FR-LIB-12 | Planned |
| MT-RAND-02 | Sample and hold | Captures the input on a trigger and holds it, or follows the input while a gate is high. | | P1 | FR-LIB-04 | Planned |
| MT-RAND-03 | Chance | Passes each trigger with a set probability, or sends it to output A or B by chance. Several lanes in one module. | Seed | P2 | FR-LIB-22 | Planned |

## Math and logic

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-MATH-01 | Arithmetic | Adds, subtracts, multiplies, or divides two or more signals. | | P1 | FR-LIB-05 | Built in part (`loom.math.add`, `loom.math.mul`) |
| MT-MATH-02 | Function | Applies one chosen function: power, root, exponential, logarithm, rounding, absolute value and sign, minimum, maximum, mean, or trigonometry. The tile shows the symbol. | | P2 | New | Planned |
| MT-MATH-03 | Convert | Converts between units: linear and decibels, frequency and pitch. | | P1 | FR-SIG-05 | Planned |
| MT-MATH-04 | Expression | Evaluates a short typed formula on its inputs. The formula is data, parsed and checked when the patch loads, never run as code (vision, principle 7). | | P4 | New | Planned |
| MT-LOGIC-01 | Compare | Outputs a gate when one signal is above, below, or equal to another, inside a window, or on a rising or falling edge. | | P1 | FR-LIB-05 | Planned |
| MT-LOGIC-02 | Logic | AND, OR, NOT, XOR, and their inverses, on two or more gates. | | P1 | FR-LIB-05 | Planned |
| MT-LOGIC-03 | Memory | Stores a gate state: set and reset, toggle, data on a clock, and a latch that cycles through N states. | | P1 | FR-LIB-05 | Planned |
| MT-LOGIC-04 | Counter | Counts triggers from 0 to N-1, outputs the count as a phase, and fires when it wraps. Covers countdowns and "after N times". | | P3 | FR-LIB-22 | Specified |
| MT-LOGIC-05 | Step select | N gate outputs, one high at a time, chosen by a phase. | | P3 | FR-LIB-22 | Specified |
| MT-LOGIC-06 | Shift register | Passes values or gates down a chain of stages on each clock. | | P3 | New | Planned |
| MT-LOGIC-07 | State machine | Moves between named states on triggers and conditions, with one gate out per state. | | P3 | FR-LIB-22 | Planned |
| MT-LOGIC-08 | Value stack | Stores values and gives them back first-in-first-out or last-in-first-out. | | P4 | New | Planned |
| MT-LOGIC-09 | Binary code | Converts a number to one gate per bit, and back. | | P4 | New | Planned |

## Gates and rhythm

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-GATE-01 | Gate shaper | Sets the length of a gate, delays it, or turns its start or end into a trigger. | Timebase | P2 | New | Planned |
| MT-GATE-02 | Euclidean rhythm | Spreads N hits as evenly as possible over M steps, with rotation. | | P2 | FR-LIB-08 | Planned |
| MT-GATE-03 | Repeater | Repeats a gate at a rate while it is high, or fires a quick burst of N triggers from one trigger. | Timebase | P3 | New | Planned |

## Pitch and scale

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-PITCH-01 | Pitch math | Moves a pitch by octaves, semitones, scale steps, or a frequency ratio; sums several pitches; or spreads a pair around one pitch. | Scale awareness | P2 | FR-LIB-13 | Planned |
| MT-PITCH-02 | Pitch quantiser | Snaps a pitch signal to the nearest note of a scale, or of the notes being held. | Scale awareness | P2 | FR-LIB-29 | Planned |
| MT-PITCH-03 | Scale and tuning | Sets the patch-wide scale and tuning (equal divisions, just ratios, imported tuning files) that pitch modules follow. Can be changed live by a trigger. | | P2 | FR-LIB-29 | Planned |
| MT-PITCH-04 | Pitch tracker | Estimates the pitch of incoming audio. | | P4 | New | Planned |

## Sequencers and generators

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-SEQ-01 | Multi-lane sequencer | Steps through stored values in one or more lanes. Each lane has a type (value, gate, pitch, or curve), a length, a direction, and a read mode: by clock, by phase, or by a control value that points at a step. Steps can carry accent, chance, and repeat. P2 builds lanes that share one length and rate (FR-LIB-08). Lanes with their own length, rate, and direction, which give polymetric patterns, come in P3 (FR-LIB-14). | Timebase, Retrigger, Seed, Scale awareness | P2 | FR-LIB-08, FR-LIB-14 | Planned |
| MT-SEQ-02 | Parameter sequencer | Each step sets its own group of parameter values elsewhere in the patch. | Timebase | P3 | FR-LIB-12 | Planned |
| MT-SEQ-03 | Piano roll | Notes of any length and pitch in a loop, drawn or recorded. | Timebase, Scale awareness | P3 | New | Planned |
| MT-SEQ-04 | Generator | Makes melodies or rhythms from a chosen algorithm: random notes in a scale, a random walk, a looping pattern that slowly mutates, a Markov chain, or rule systems such as L-systems and cellular automata. | Seed, Scale awareness, Timebase | P3 | FR-LIB-28 | Planned |
| MT-SEQ-05 | Note file player | Plays a stored note file in step with the transport. | Timebase | P4 | New | Planned |
| MT-SEQ-06 | Style generator (abstraction) | A bass line, chord part, or drum part from a seed and a style setting, built from a generator and sequencer lanes. | Seed | P4 | FR-LIB-28 | Planned |

## Note effects

These depend on how notes travel on cables (OQ-16).

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-NOTE-01 | Arpeggiator | Plays held notes one at a time in a pattern. | Timebase, Seed | P3 | FR-LIB-13 | Planned |
| MT-NOTE-02 | Chord | Adds notes at set intervals, or scale steps, to each incoming note. | Scale awareness | P3 | FR-LIB-13 | Planned |
| MT-NOTE-03 | Strum | Spreads the notes of a chord in time. | | P3 | FR-LIB-13 | Planned |
| MT-NOTE-04 | Note repeat | Repeats notes: as fading echoes, at a steady rate while held, or speeding up or slowing down like a bouncing ball. | Timebase, Scale awareness | P3 | FR-LIB-13 | Planned |
| MT-NOTE-05 | Note variation | Small random changes to timing, velocity, and pitch, and a chance that a note plays at all. | Seed, Scale awareness | P3 | FR-LIB-13 | Planned |
| MT-NOTE-06 | Note hold | Sets or scales note lengths, or holds notes after the keys are released. | | P3 | FR-LIB-13 | Planned |
| MT-NOTE-07 | Note filter and split | Passes or routes notes by pitch, velocity, or channel. | | P3 | New | Planned |

## Time effects

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-DELAY-01 | One-sample delay | Delays by exactly one sample, to close feedback loops. | | P0 | FR-LIB-06 | Built (`loom.time.delay1`) |
| MT-DELAY-02 | Delay line | An interpolated delay with a modulated time, for building effects and models. | Quality | P1 | FR-LIB-06 | Planned |
| MT-DELAY-03 | All-pass | All-pass filters for phase effects and for building reverbs. | | P1 | FR-LIB-06 | Planned |
| MT-DELAY-04 | Delay | A finished delay with several taps, feedback, filtering, ping-pong, and a choice of how it reacts when the time changes (pitch glide, crossfade, or jump). | Timebase, Character, Mix and output | P2 | FR-LIB-15 | Planned |
| MT-DELAY-05 | Modulated delay | Short delays moved by an LFO, for chorus, ensemble, and flanging. | Spread, Mix and output | P2 | FR-LIB-09 | Planned |
| MT-FX-01 | Reverb | A room, hall, plate, or spring sound from a network of delays and diffusers. | Character, Mix and output | P2 | FR-LIB-09 | Planned |
| MT-FX-02 | Phaser | Swept all-pass stages that move notches through the sound. | Mix and output | P2 | FR-LIB-15 | Planned |
| MT-FX-03 | Tremolo and auto-pan (abstraction) | An LFO moving level or stereo position. | | P2 | FR-LIB-15 | Planned |
| MT-FX-04 | Buffer repeat | Captures a short slice of the input and repeats it, with reverse and a tape-stop slowdown. A live performance tool. | Timebase, Mix and output | P3 | New | Planned |
| MT-FX-05 | Frequency shifter | Moves every partial up or down by a fixed number of hertz. | Mix and output | P4 | FR-LIB-15 | Planned |
| MT-FX-06 | Pitch shifter | Changes pitch without changing length. | Quality, Mix and output | P4 | FR-LIB-15 | Planned |
| MT-FX-07 | Varispeed | Changes playback speed and pitch together, with tape-style wobble. | Mix and output | P4 | New | Planned |
| MT-FX-08 | Freeze | Holds the current sound as an endless texture. | Mix and output | P4 | New | Planned |
| MT-FX-09 | Convolution | Applies a recorded impulse response, for real spaces and unusual filters. | Mix and output | P4 | New | Planned |
| MT-FX-10 | Spatialiser | Moves a sound around a stereo or multichannel field. | | P4 | New | Planned |
| MT-FX-11 | Rotary speaker (abstraction) | The pitch wobble, level change, and filtering of a spinning speaker. | | P4 | New | Planned |
| MT-FX-12 | Amp and cabinet (abstraction) | Amplifier-style drive followed by speaker colouring. | | P4 | New | Planned |

## Dynamics, EQ, and spectral

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-DYN-01 | Dynamics | Compresses, limits, gates, expands, or ducks, with a sidechain input that can come from another signal. | Sidechain, Mix and output | P2 | FR-LIB-09 | Planned |
| MT-DYN-02 | Transient shaper | Raises or lowers the attack and sustain of hits. | Mix and output | P4 | FR-LIB-15 | Planned |
| MT-DYN-03 | Multiband dynamics (abstraction) | A crossover with a dynamics slot on each band. | | P4 | FR-LIB-15 | Planned |
| MT-EQ-01 | EQ | Bands of peak, shelf, and cut filters. Presets give a one-knob tilt and a three-band cut. | | P2 | FR-LIB-15 | Planned |
| MT-SPEC-01 | Spectral filter | Filters and reshapes sound in the frequency domain. | Quality, Mix and output | P4 | FR-LIB-10 | Planned |
| MT-SPEC-02 | Spectral freeze and blur | Holds or smears spectral frames over time. | Mix and output | P4 | FR-LIB-10 | Planned |
| MT-SPEC-03 | Spectral split | Separates tonal from noisy parts, or attacks from sustain. | | P4 | FR-LIB-15 | Planned |
| MT-SPEC-04 | Vocoder (abstraction) | Puts the spectrum of one signal onto another, with two filter banks and envelope followers. | Sidechain | P4 | New | Planned |

## Displays and controls

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-VIEW-01 | Probe | Shows signals on the tile: over time (scope), one against another (XY), or as a number in chosen units. | | P1 | FR-LIB-17 | Specified (scope module, [panels](../design/panels.md#scope-module)) |
| MT-VIEW-02 | Level meter | Shows peak and average level. | | P1 | FR-LIB-17 | Built (`loom.display.meter`) |
| MT-VIEW-03 | Spectrum | Shows the frequency content of a signal. | | P2 | FR-LIB-17 | Planned |
| MT-VIEW-04 | Note monitor | Shows incoming notes and controller values, for checking a setup. | | P2 | New | Planned |
| MT-VIEW-05 | Tuner | Shows the pitch of a signal against the nearest note. | | P4 | New | Planned |
| MT-UI-01 | Button | A large button that outputs a toggle or momentary gate. | | P1 | FR-LIB-27 | Built (`loom.ui.button`) |
| MT-UI-02 | Macro | A knob or slider whose value is a signal. One macro can drive many parameters through modulation assignments. | Smoothing | P2 | FR-LIB-12 | Planned |
| MT-UI-03 | XY and vector pad | A two-axis control, or one point that blends between several sources. | Smoothing | P2 | FR-LIB-12 | Planned |
| MT-UI-04 | Keyboard | Plays notes from the canvas or a front panel. | | P2 | New | Planned |

## Structure, arrangement, and state

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-STRUCT-01 | Sub-patch | A group of modules collapsed into one, with its own ports and panel. Modes: single, layer (all members run and mix), select (one member runs, chosen by a control), and stack (N copies, each with its own spread values). | Spread | P2 | FR-ENCAP-01, FR-ENCAP-10 | Planned |
| MT-STRUCT-02 | Patch input and output | Ports inside a sub-patch that become its outer ports. | | P2 | FR-ENCAP-01 | Planned |
| MT-STRUCT-03 | Slot | A place inside an abstraction that holds any module or sub-patch that fills a given role. Swapped from a menu, with cables kept. | | P2 | FR-ENCAP-08 | Planned |
| MT-STRUCT-04 | Effect chain (abstraction) | Several effect slots in series, each with its own mix. | | P2 | FR-ENCAP-08 | Planned |
| MT-ARR-01 | Note cycle | Plays a stored note pattern that is N bars long, read from the bar phase, and fires an end trigger. | | P3 | FR-LIB-20 | Specified ([arrangement](../design/arrangement.md)) |
| MT-ARR-02 | Audio cycle | Plays a stored audio loop that is N bars long, read from the bar phase, and fires an end trigger. | | P3 | FR-LIB-20 | Specified ([arrangement](../design/arrangement.md)) |
| MT-ARR-03 | Looper group (abstraction) | Rotates through its cycles, N loops each, and fires a trigger when a round completes. | | P3 | FR-LIB-21 | Specified ([arrangement](../design/arrangement.md)) |
| MT-ARR-04 | Part (abstraction) | A looper group plus a rounds counter. | | P3 | FR-LIB-24 | Specified ([arrangement](../design/arrangement.md)) |
| MT-ARR-05 | Song sequencer (abstraction) | Steps through parts as each one finishes. | | P3 | FR-LIB-24 | Specified ([arrangement](../design/arrangement.md)) |
| MT-ARR-06 | Fade controller (abstraction) | Fades the whole patch in or out on cue, and can start and stop a recorder. | | P3 | New | Planned |
| MT-ARR-07 | Conductor (abstraction) | A list of sections with lengths in bars. Each section switches sequencer patterns and recalls scenes. | | P4 | New | Planned |
| MT-ARR-08 | Canon looper (abstraction) | Plays several copies of a live loop, each delayed or at a different speed. | | P4 | New | Planned |
| MT-STATE-01 | Scene | Stores the state of chosen modules and recalls it on a trigger, with an optional morph time. | | P3 | FR-LIB-23 | Planned |
| MT-STATE-02 | Scene morph | Places stored scenes on a line or a 2D surface and blends them by position. | | P4 | FR-PERF-03 | Planned |
| MT-MULTI-01 | Multiband processor (abstraction) | A crossover with an effect slot on each band. | | P4 | FR-ENCAP-08 | Planned |

## Input and output

| ID | Type | What it does | Traits | Phase | Req. | Status |
|---|---|---|---|---|---|---|
| MT-IO-01 | Audio output | Sends a signal to an interface channel or host output. | | P0 | FR-LIB-07 | Built (`loom.io.audio-out`) |
| MT-IO-02 | Audio input | Brings audio from an interface channel or host input into the patch. | | P1 | FR-LIB-07 | Planned |
| MT-IO-03 | Note input | Turns incoming notes into gate, pitch, and velocity signals, per voice. | | P0 | FR-LIB-07 | Built in part (`loom.midi.note`: one voice; per-voice output comes with polyphony in P2) |
| MT-IO-04 | Controller input | Outputs a chosen MIDI controller, pitch bend, or aftertouch as a signal. | Smoothing | P1 | FR-LIB-07 | Planned |
| MT-IO-05 | Expression input | Outputs per-note pressure, timbre, and slide as signals. | | P2 | FR-VOICE-04 | Planned |
| MT-IO-06 | Note output | Builds outgoing notes from gate, pitch, and velocity signals. | | P2 | FR-MIDI-04 | Planned |
| MT-IO-07 | Controller output | Sends a signal out as a MIDI controller value. | | P2 | FR-MIDI-04 | Planned |
| MT-IO-08 | Key state | Reports whether a given key is held, or how many keys are held. | | P2 | New | Planned |
| MT-IO-09 | OSC input and output | Receives or sends values over Open Sound Control. | | P4 | FR-MIDI-05 | Planned |
| MT-IO-10 | Hardware CV | Exchanges DC-coupled control voltages with modular hardware through an audio interface. | | P4 | New | Planned |

---

## Traits

A trait is a set of parameters and behaviour defined once in the module SDK and declared by any module that needs it (FR-SDK-07). Users learn a trait once, and it looks and behaves the same on every module.

Why traits: many module types differ only in an option that many other types also have, such as "synced to the bar or free". Building that option once keeps the library small and the controls consistent, and it means a fix or an improvement reaches every module at once.

| ID | Trait | What it adds |
|---|---|---|
| TR-01 | Shape source | A choice of wave shape: the basic shapes, a blend of them, a wavetable, or a drawn curve. |
| TR-02 | Timebase | How rate or time is set: in hertz or seconds, in musical divisions locked to the transport, or by an external phase input. |
| TR-03 | Retrigger | What restarts the module: nothing (free running), each new note, a gate, or the transport. |
| TR-04 | Spread | A different value of any chosen parameter for each copy in a stack, each voice, or each side of a stereo pair. |
| TR-05 | Character | A choice of response model, such as clean, analog-style, tape-style, or slow and lagging. |
| TR-06 | Quality | Oversampling and anti-aliasing level, traded against CPU. |
| TR-07 | Seed | A stored random seed, so random results repeat when the patch loads (NFR-SND-04). |
| TR-08 | Scale awareness | Follows the patch-wide scale and tuning (MT-PITCH-03) unless overridden. |
| TR-09 | Sidechain | A second input that drives detection instead of the main input. |
| TR-10 | Mix and output | Dry and wet balance and output level, with an equal-loudness option. |
| TR-11 | Smoothing | Glides between output values, so changes do not step or click. |

## Roles

A role is a contract: a set of port names and core parameter names that a module or sub-patch promises to have (ADR-0017). A slot in an abstraction accepts anything that fills its role, so the user can swap it and keep the cables and the main settings. Role names and versions are an open question (OQ-20).

| Role | Ports | Core parameters | Filled by, for example |
|---|---|---|---|
| Sound source | pitch in, audio out | level | Oscillator, Noise, Buffer reader, Grain engine, String model |
| Filter | audio in, audio out | cutoff, resonance, drive | State variable filter, Ladder low-pass, Comb filter, Formant filter |
| Shaper | audio in, audio out | drive, mix | Shaper, Bit and rate reducer |
| Envelope | gate in, level out | attack, release | Envelope, Segment envelope, Function generator |
| Modulator | level out | rate, amount | LFO, Random, Curve reader |
| Effect | audio in, audio out | mix | Delay, Reverb, Modulated delay, Dynamics, EQ |
| Voice | note in, audio out | level | Synth voice, Drum voice, Sampler, user sub-patches |

A user's own sub-patch can declare a role and then fill any matching slot (FR-ENCAP-09).

---

## Summary

| Phase | Types | Of which new (no requirement yet) |
|---|---|---|
| P0 | 3 | 0 |
| P1 | 34 | 0 |
| P2 | 55 | 16 |
| P3 | 32 | 13 |
| P4 | 46 | 30 |
| **Total** | **170** | **59** |

Plus 11 traits and 7 initial roles. The summary is counted from the tables above; update it when entries change.
