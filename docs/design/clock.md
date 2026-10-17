# Clock and Transport Specification

How musical time reaches every module: the patch transport, inputs that follow it without cables, and the modules that put time on cables and divide it.

- **Kind:** living specification.
- **Status:** Sections have their own status. **Implemented** for the engine slice ([#72](https://github.com/brylie/LoomDSP/issues/72)): the transport fields, signals derived from the transport, default inputs, rules 1 to 6 (except saving run state, which needs the Transport module), and engine and API. Also **Implemented**: the Clock Divide ([#48](https://github.com/brylie/LoomDSP/issues/48)) and Phase Trigger modules. Everything else is Draft: review it before building.
- **Decision behind it:** [ADR-0015](../adr/0015-one-transport-per-patch.md) (one transport per patch). Builds on [ADR-0007](../adr/0007-polymorphic-signal-model.md) (phase and gate signals) and [ADR-0012](../adr/0012-arrangement-from-phase-driven-primitives.md) (arrangement from phase).
- **Requirements:** FR-LIB-07, FR-LIB-16, FR-LIB-22, FR-LIB-25 to FR-LIB-27, FR-MIDI-07.
- **Background:** [Phase and cycles](../learn/phase-and-cycles.md) explains phase signals and how the end of a cycle is found.
- **Prior art:** VCV Rack clock modules (run and reset on the panel and as inputs), Bitwig's Button modulator.
- **Issues:** [#41](https://github.com/brylie/LoomDSP/issues/41) (transport and clock), [#72](https://github.com/brylie/LoomDSP/issues/72) (engine transport and default inputs), [#73](https://github.com/brylie/LoomDSP/issues/73) (Transport module), [#48](https://github.com/brylie/LoomDSP/issues/48) (Clock Divide), [#36](https://github.com/brylie/LoomDSP/issues/36) (LFO), [#47](https://github.com/brylie/LoomDSP/issues/47) (triggers).

## Goal

> I set the tempo to 96. My LFOs, sequencers, and loops all follow it, and I never patched a clock cable. Then I patch a half-speed phase into one sequencer, and it plays against the others.

## The transport

**Status:** the fields table is Implemented. "Controls live on the Transport module" and "Starting and stopping" are Draft ([#73](https://github.com/brylie/LoomDSP/issues/73)).

The engine keeps one transport per patch.

| Field | Range | Default | Notes |
|---|---|---|---|
| Tempo | 20 to 300 BPM | 120 | Quarter-note beats per minute. |
| Time signature | 1 to 32 over 1, 2, 4, 8, 16 | 4/4 | Sets the length of a bar. |
| Running | on, off | on | Saved in the patch. See rule 6. |
| Position | beats, double precision | 0 | Quarter-note beats since bar 1. |
| Source | internal, host, MIDI clock | internal | Host in plugins; MIDI clock in P2. |

### Controls live on the Transport module

Every patch has exactly one **Transport** module, as it has one audio output. Its panel is the transport's control surface, as a clock module is in VCV Rack:

- a **Run** toggle, a **Reset** button, a **tempo** knob with a typed value, and the time signature;
- a **run** input (gate) and a **reset** input (trigger), so anything in the patch can start and stop time.

There are no transport controls in the app toolbar. The Space key toggles Run when no panel control is focused, as a shortcut to the same control. When a panel control has focus, Space and Enter act on that control instead ([panels](panels.md#interaction-68)).

*Why on a module:* it follows ADR-0011 and ADR-0013. Time is controlled where you see it, it can go on a front panel, and it can be patched. Starting the clock from a pad, a sequencer, or a logic module is ordinary patching.

### Starting and stopping

| How | What happens |
|---|---|
| Click Run on the Transport panel, or press Space when no panel control is focused | Toggles running. |
| A gate into the run input | High runs, low stops. While the input is connected, it decides, and the Run toggle shows its state. |
| A trigger into reset, or the Reset button | Returns to bar 1 and fires the jump trigger. While stopped, the next start begins at bar 1. |

A **Button** module (FR-LIB-12) gives a performance control for this. Its panel has one large button, and its output is a gate. In toggle mode, each press flips the gate, like Bitwig's Button modulator; in momentary mode, the gate is high while held. Patch a toggle Button into the run input, put the Button on the front panel, and the performer starts and stops the piece from there.

*Why the run input is a gate, not a trigger:* a level cannot drift out of step. After a patch reload or a missed pulse, a toggle trigger could leave the clock stopped when the pad shows "on". With a gate, the input is always the truth.

## Signals derived from the transport

**Status:** Implemented. The rate phase row belongs to the Clock module ([#74](https://github.com/brylie/LoomDSP/issues/74)) and stays Draft.

| Signal | Value |
|---|---|
| Beat phase | `fract(position)` |
| Bar phase | `fract(position / beats_per_bar)`, with `beats_per_bar = numerator × 4 / denominator` |
| Rate phase | `fract(position / rate_in_beats)`, for a Clock module's rate |
| Playing | gate: 1.0 while playing, 0.0 while stopped |

Every phase is computed from the absolute position, never by adding up steps. *Why:* phases stay aligned with bar 1 after a stop, a tempo change, or a host jump, and two modules that compute the same rate always agree.

Phases are computed in `f64` and converted to `f32` last. A phase that rounds to 1.0 in `f32` becomes 0.0. *Why:* a phase is always in [0, 1). The wrap then comes less than one `f32` step early, far less than one sample.

## Default inputs

**Status:** Implemented, except the clock mark, which is UI work for later and stays Draft.

A module can declare a **default** for any input. The default is used while nothing is connected.

| Default | Meaning |
|---|---|
| Value | A fixed number. Most inputs use 0. |
| Bar phase, beat phase | The transport's phase. |
| Playing | The transport's play gate. |

- Connecting a cable replaces the default for that module. Removing the cable brings it back.
- An input that is following the transport shows a small clock mark next to the port. *Why:* behaviour without a cable must still be visible.
- The engine computes each transport signal once per block into a shared buffer, and points unconnected inputs at it. *Why:* hundreds of synced modules cost one buffer, not hundreds.
- The default is part of the module's static descriptor. It does not change with the module's parameters. A module that uses an input only in some modes ignores it in the others. *Why:* the compiler binds inputs when the patch changes, not when a parameter moves, so binding stays off the audio thread.
- `PortIo::is_connected` still reports cables only. An input that follows its default reports false. *Why:* a module can still tell whether the user patched something, for example to show it or to pick a different behaviour.

Examples: the LFO's phase input in sync mode defaults to bar phase, with a rate parameter that scales it. Note Cycle and Audio Cycle default their bar phase input to the transport (FR-LIB-20).

## Modules

**Status:** Clock Divide ([#48](https://github.com/brylie/LoomDSP/issues/48)), Phase Trigger, and Button ([#75](https://github.com/brylie/LoomDSP/issues/75)) are Implemented; see their details below the table. The other modules are Draft.

| Module | Inputs | Outputs | Parameters | Behaviour |
|---|---|---|---|---|
| **Transport** `loom.clock.transport` | run (gate), reset (trigger) | bar phase, beat phase, playing (gate), bar (trigger), beat (trigger), jump (trigger) | tempo, time signature, run | Exactly one per patch, and cannot be deleted. Its panel controls the transport (see above). Jump fires when the position moves by anything other than normal playback (a reset, a host loop). |
| **Clock** `loom.clock.clock` | none | phase, trigger, gate | rate, gate width | A phase at a musical rate: 4 bars, 2 bars, 1 bar, 1/2, 1/4, 1/8, 1/16, 1/32, each straight, dotted, or triplet. Trigger fires on each wrap. Gate is high for the first part of each cycle (gate width, 1 to 99 percent). |
| **Button** `loom.ui.button` | none | gate | mode (toggle, momentary), on | A large button. The gate equals `on`. Toggle flips `on` on each press. Momentary holds `on` high while the button is held. |
| **Clock Divide** `loom.logic.divide` | trigger, reset | trigger | N (1 to 64), offset (0 to N-1) | Counts rising edges. Fires on the triggers numbered N, 2N, 3N after a reset; offset k moves each firing k triggers earlier. With offset N-1 it fires on the first trigger. |
| **Phase Scale** `loom.phase.scale` | phase, reset | phase | multiply (1 to 16), divide (1 to 16) | Scales a phase by multiply ÷ divide, and stays locked to it. Divide counts the input's wraps, so a divide of 2 wraps once every two input cycles, starting on the same bar. |
| **Phase Trigger** `loom.phase.trigger` | phase | trigger | none | Fires a trigger each time the input phase wraps: forward from near 1 to near 0, or backward from near 0 to near 1. Turns any phase into triggers: a scaled phase, an LFO's phase, or the transport's bar phase. |

*Why both Clock Divide and Phase Scale:* triggers are events, for counting and arrangement logic (ADR-0012). Phases are continuous, for anything that should move smoothly and in sync: LFOs, sequencers, cycles. Each domain needs its own divider. Phase Scale also gives polyrhythm: multiply 3, divide 2 plays three against two.

*Why Phase Trigger:* it is how to multiply a clock. Phase Scale multiplies a phase, and Phase Trigger turns the result into triggers, for example to drive a Clock Divide or an envelope. A trigger-based multiplier, as in many VCV Rack modules, would have to measure the time between input triggers and guess when the next one comes. It would stumble on the first trigger and whenever the tempo changes. A multiplied phase is computed from the input, so it never guesses and stays locked. This follows Bitwig Grid, where timing is built from phase signals.

Trigger outputs are high for one sample, and trigger inputs count rising edges ([ADR-0007](../adr/0007-polymorphic-signal-model.md#triggers)).

### Clock Divide details

These details were settled while building the module.

- **Parameter names** are `n` (default 2) and `offset` (default 0). Both are whole numbers.
- **Offset above N-1.** The `offset` parameter ranges from 0 to 63, because a parameter's range is fixed in the descriptor. An offset of N or more counts as N-1. *Why:* offset N-1 means "fire on the first trigger", and lowering N keeps that meaning.
- **Reset and trigger on the same frame.** The reset comes first, so that trigger counts as the first one after the reset. *Why:* a loop's end trigger and a section reset often arrive together, and the new section must start counting at once.
- **Changing N.** The module counts every trigger since the reset, not the count modulo N. So after changing N, it fires as if N had been set at the reset.
- **No latency.** The output fires on the same frame as the input trigger.

### Phase Trigger details

- **What counts as a wrap.** The phase jumps by more than 0.5 from one frame to the next, in either direction. *Why:* a phase moving at any rate below half the sample rate changes by less than 0.5 per frame, so only a wrap can jump that far. A small wobble does not fire.
- **Both directions.** A phase running forward wraps from near 1 to near 0. One running backward wraps from near 0 to near 1. Both start a new cycle, so both fire.
- **No latency.** The trigger fires on the first frame of the new cycle. That frame is also the end of the old cycle, so this is the same frame an end-of-cycle trigger marks.
- **The first frame** fires only when the phase is exactly 0. *Why:* a phase that starts on a bar line marks bar 1, but a module added halfway through a cycle must not fire.
- **Jumps.** A reset to 0 from above 0.5, or any other jump of more than half a cycle, looks the same on the cable as a wrap, and fires once. A smaller jump does not fire.
- **Crossings and turnarounds.** To fire when a phase passes another position *p*, shift the phase by *-p* first, then use Phase Trigger. A ping-pong phase never wraps; take its turnarounds from the phase that drives it. [Phase and cycles](../learn/phase-and-cycles.md#phases-that-never-wrap) explains both with examples.
- **Shared rule.** The rule lives in `loom_core::signal::WrapDetector`. Every trigger that marks the end of a cycle should use it: Phase Trigger now, and later the Clock trigger, the Transport's bar and beat triggers, and the end trigger of cycles (FR-LIB-20). *Why:* they must all agree on where a cycle ends.

### Button details

**Status:** Agreed ([#75](https://github.com/brylie/LoomDSP/issues/75)). Code: `loom.ui.button` in `loom-modules/src/ui.rs`, and `WidgetKind::Button` in `loom-core`.

- **Parameters.** `mode` is a whole number: 0 is Toggle (the default), 1 is Momentary. `on` is a whole number, 0 or 1, default 0. Both are saved. The output `gate` equals `on`. `process` ignores `mode`. *Why:* the gesture is what differs. The cable always carries the same gate.
- **No extra threshold.** A whole-number parameter is rounded after the clamp ([modulation](modulation.md), rule 5), so the module only sees 0 or 1. The gate is that value, with no smoothing.
- **Panel.** One button, 6 by 3 grid units, bound to `on`. Its fill shows the gate. Its caption is "On". `mode` stays in the inspector. *Why:* mode is a setup choice. A control for it on the tile would be easy to hit by mistake on a front panel.
- **Toggle.** Pointer down flips `on`. Space or Enter flips it. Arrow keys set 0 or 1, as for a toggle. A double-click is two flips, not a reset. *Why press, not release:* the gate should land on the beat. Waiting for release would add the length of the press as delay.
- **Momentary.** Pointer down sets `on` to 1. Pointer up sets it to 0, including after the pointer has left the button. Space or Enter holds it high while the key is down. Arrow keys do nothing.
- **The button does not drag the tile.** In both modes, pointer down on the button leaves the tile where it is. Drag the tile from its header.
- **Key repeat is ignored.** A held key does not flip the gate again.
- **Switching to Momentary clears `on`.** The editor does this when `mode` changes to 1. Setting Momentary again, while it is already Momentary, does not clear a hold. *Why:* the light and the gate must agree. A latched gate with no finger on the button would lie.
- **Space.** When a panel control has keyboard focus, Space and Enter act on that control and do not toggle Run. Space toggles Run only when no panel control is focused. *Why:* if both ran, a focused Button patched into run would be asked to change twice. The focused control is the one being played. Run's handler, when the Transport module is built ([#73](https://github.com/brylie/LoomDSP/issues/73)), must not also see a key the canvas used.
- **A saved hold stays on.** If a patch is saved while a momentary button is held, it loads with `on` at 1. The next tap clears it. That takes Space held during a save, so P1 accepts it. A general way to leave a parameter out of the file, if one is ever needed, gets its own spec.

## Rules

**Status:** rules 1 to 6 are Implemented in the engine; saving run state in the patch waits for the Transport module ([#73](https://github.com/brylie/LoomDSP/issues/73)). Rules 7 to 9 are Draft.

1. **One transport, owned by the engine.** Modules read it; only the transport source changes it. *Why:* every synced module must agree on where the bar starts.
2. **Position is double precision.** Phases are converted to `f32` only at the end. *Why:* in `f32`, position loses sample accuracy after a few minutes. An hour-long set must stay in step.
3. **Tempo changes keep the position.** A new tempo changes the speed from the next block on; phases never jump. *Why:* changing tempo while playing must not glitch loops or sequencers.
4. **The transport updates at block starts.** Inside a block, position advances by a fixed amount per sample. *Why:* blocks are at most `max_block` frames (2.7 ms at 48 kHz with 128), well under what anyone hears, and modules can compute phases in a tight loop.
5. **Nothing depends on block size.** The derived signals at a given sample are the same, bit for bit, for any block size. *Why:* block-size invariance (AGENTS.md hard rule 2). To make this hold, position is computed from an anchor, not by adding a step per block (see Engine and API).
6. **Run state is part of the patch.** A new patch starts running. Stopping holds the position, and synced modules hold their phase. Running again continues from there; Reset returns to bar 1. *Why:* a patch opens the way it was saved. Holding when stopped matches hardware clocks in VCV Rack and Eurorack: no clock, no movement, so stopping really stops the music.
7. **A host or MIDI clock source replaces the internal one completely.** Tempo, play state, and position come from it. *Why:* in a DAW, the patch must follow the song.
8. **Position jumps fire no skipped triggers.** When the position jumps, bar and beat triggers do not fire for the bars that were skipped, and the jump trigger fires once. *Why:* a host looping four bars must not burst out dozens of triggers.
9. **Sub-patches share the patch transport.** A sub-patch can take a different phase through an input. *Why:* one time base, but time can still be bent by patching.

## Engine and API

**Status:** Implemented ([#72](https://github.com/brylie/LoomDSP/issues/72)), except the patch format, which belongs to [#73](https://github.com/brylie/LoomDSP/issues/73).

### Transport state

- The engine owns the transport. It is not part of the schedule, so editing the patch never moves it. *Why:* adding a module while playing must not restart the bar.
- A new engine starts running, at 120 BPM, in 4/4, at position 0.
- The control thread changes the transport through the `Controller`: set tempo, set time signature, set running, and reset. Each change goes through the command queue, like a parameter change, and takes effect at the start of the next buffer. *Why:* no locks, and one path for every change. The Transport module ([#73](https://github.com/brylie/LoomDSP/issues/73)) and the app will use the same calls.
- Tempo is clamped to 20 to 300 BPM, as a parameter is. A time signature out of range is refused with an error, and the transport does not change. *Why:* a clamped tempo is still a sensible tempo. There is no sensible nearest value for 7/3.

### Position

- `beats_per_sample = tempo / 60 / sample_rate`, in `f64`.
- Position at a sample is `anchor_position + (sample - anchor_sample) × beats_per_sample`. `sample` counts frames since the engine started, as an integer. *Why:* adding a step per block rounds differently at different block sizes. With an anchor, a given sample gets the same position for any block size, bit for bit (rule 5), and rounding does not build up over an hour (rule 2).
- The anchor moves to the current sample and position whenever the tempo changes, the transport starts or stops, or the position is reset. So a tempo change keeps the position (rule 3).
- While stopped, the position does not move (rule 6).

### Jumps

- In P1, the only jump is a reset. Starting, stopping, and tempo changes are not jumps.
- A reset sets the position to 0. It counts as a jump at the first frame of the next buffer, whether the transport is running or stopped. A stopped transport stays stopped, and starts from bar 1 later.

### `ProcessContext`

Every call to `Module::process` gets the transport:

| Field | Type | Meaning |
|---|---|---|
| playing | `bool` | True while the transport runs. |
| tempo | `f64` | Beats per minute. |
| position | `f64` | Position, in beats, at the first frame of this call. |
| beats per sample | `f64` | How far the position moves per frame in this call. 0 while stopped. |
| beats per bar | `f64` | `numerator × 4 / denominator`. |
| jumped | `bool` | True when the position jumped at the first frame of this call. |

- Inside a feedback loop, each call is one frame. Position and jumped then describe that frame. Only the call for the first frame of the block can see `jumped`. *Why:* a module then sees the same values for any grouping of frames (rule 5).
- For the position at frame `i` of a call, a module uses `TransportInfo::position_at(i)` (or `bar_phase_at`, `beat_phase_at`). These compute from the anchor, exactly as the engine's shared buffers do. *Why:* adding `i × beats_per_sample` to the position rounds differently at different block sizes, which would break rule 5 inside the module.
- *Why beats per sample is 0 while stopped:* any position computed from it then holds still with no special case, as rule 6 asks.
- `ProcessContext::new`, used to test modules alone, gives a stopped transport at position 0, 120 BPM, 4/4. Tests can set another transport.

### `PortSpec` default

- A `PortSpec` gains a default: a value (`f32`), bar phase, beat phase, or playing.
- The default default is the value 0. *Why:* every existing module keeps its behaviour.

### Schedule

- The compiler lists the transport signals that at least one unconnected input uses, and gives each one buffer. The engine fills these buffers at the start of each block, before any module runs. *Why:* a signal that nothing uses costs nothing.
- An input with a value default other than 0 reads a constant buffer, one per distinct value, filled once when the schedule is compiled. Inputs with default 0 keep sharing the zero buffer.
- Filling the buffers is real-time safe: no allocation, no locks.

### Patch format

The Transport module's parameters (tempo, time signature, run) are saved like any module's ([#73](https://github.com/brylie/LoomDSP/issues/73)). Until then, a patch file does not store the transport.

## P1 slice

### Acceptance criteria: engine ([#72](https://github.com/brylie/LoomDSP/issues/72))

"Block size" below means both the buffer length passed to `Engine::process` and `max_block`.

- [x] After one hour at 120 BPM and 48 kHz, bar phase at the last sample matches the exact value within one sample. The exact value comes from integer arithmetic on the sample count.
- [x] Changing tempo while playing keeps the position continuous: no jump in any phase.
- [x] The bar phase, beat phase, and playing buffers are bit-identical at block sizes 1, 32, and 1024, over a run that includes a tempo change, a stop, a start, and a reset at the same samples.
- [x] An unconnected input with a bar phase default reads bar phase. A cable overrides it; removing the cable returns to the transport. `is_connected` is false while it follows the default.
- [x] An unconnected input with a value default reads that value.
- [x] Stopping holds the position and every phase. Starting continues from there.
- [x] Reset sets the position to 0, and `jumped` is true for exactly one call per module: the one that covers the first frame after the reset.
- [x] Out-of-range tempo is clamped. An out-of-range time signature is refused and changes nothing.
- [x] Many inputs that follow the same transport signal read one shared buffer.
- [x] The allocation tests (`loom-core/tests/transport_no_alloc.rs`, `loom-modules/tests/no_alloc.rs`) cover a patch with transport defaults and transport changes.

### Acceptance criteria: modules and UI (Draft)

- [ ] Clock at 1/4 in 4/4 fires four triggers per bar, in line with the bar phase wrap, at block sizes 1, 32, and 1024.
- [x] Clock Divide with N = 2, offset 0, fires on triggers 2, 4, 6. Offset 1 fires on 1, 3, 5. Reset starts the count again.
- [ ] Phase Scale with divide 2 wraps once per two input wraps, aligned with the input's bar. Multiply 3, divide 2 wraps three times per two input wraps.
- [x] Phase Trigger fires one sample on each wrap of its input phase, forward or backward, and on the first frame only when the phase is exactly 0. The output is the same for any block size.
- [ ] Phase Scale ([#31](https://github.com/brylie/LoomDSP/issues/31)) with multiply 4 into Phase Trigger fires four triggers per input cycle. The first is on the input's wrap. The output is the same at block sizes 1, 32, and 1024.
- [ ] An LFO in sync mode with nothing connected follows bar phase. A cable overrides it; removing the cable returns to the transport.
- [ ] Ports that follow the transport show the clock mark.
- [ ] Every patch has one Transport module. It cannot be deleted, and a second one cannot be added.
- [ ] Run on the Transport panel toggles running. Space does the same when no panel control is focused. A focused control takes Space for itself. Stopping holds synced phases; running continues from there.
- [x] The Button gate equals `on`, and every sample is 0 or 1. A toggle press flips `on`. A momentary press sets it to 1, and release sets it to 0, including when the pointer has left the button. Switching `mode` to Momentary clears `on`.
- [ ] Reset returns to bar 1 and fires the jump trigger.
- [ ] A toggle Button patched into run starts and stops the transport. While connected, the Run toggle follows the input.
- [ ] Run state, tempo, and time signature survive save and load.

### Out of scope for P1

- Host transport (with plugin export, later; FR-LIB-25 covers it once export exists).
- MIDI clock in and out (FR-MIDI-07, P2).
- Swing and shuffle.
- Tempo changes over time, such as modulation curves over bars (FR-LIB-16, P3).
- Tap tempo.

## Open points

- How tempo goes on a cable, if at all (a scale for BPM in the signal range).
- Swing: a Clock parameter, a separate module that bends a phase, or both.
- Should Clock have a phase input of its own, so it can follow a bent phase instead of the transport? Proposed answer: no. Clock stays on the transport, and a bent phase goes into Phase Trigger, which now exists. Settle this when building Clock ([#74](https://github.com/brylie/LoomDSP/issues/74)).
- The Transport module's run and reset inputs ([#73](https://github.com/brylie/LoomDSP/issues/73)) are signals, but the transport updates at block starts (rule 4). If the engine reads them only at block starts, the response to a gate edge depends on block size, which breaks rule 5. Options: split the block at the edge so the change is sample accurate, or read the inputs on a fixed grid of frames that does not depend on block size. Decide before building #73.
- What happens when the time signature changes while playing ([#73](https://github.com/brylie/LoomDSP/issues/73) exposes it as a parameter). Today the engine applies it at the next buffer. Bar phase is position divided by beats per bar, counted from bar 1, so the whole bar grid moves and bar phase jumps (in the first bar, 4/4 to 3/4 at beat 3 moves it from 0.5 to 0.67). `jumped` stays false. A module that watches for the bar phase wrap may then fire an extra bar trigger or miss one. Rule 3 covers tempo only. Options: apply it at once and count it as a jump; apply it at the next bar line, as most DAWs do; or keep the silent jump. Decide before building #73.
