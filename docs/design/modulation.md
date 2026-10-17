# Modulation Specification

How any output modulates any parameter without a cable, and how users make, see, and edit those assignments.

- **Kind:** living specification.
- **Status:** Draft for making, seeing, and editing assignments ([#70](https://github.com/brylie/LoomDSP/issues/70), [#71](https://github.com/brylie/LoomDSP/issues/71)). The [engine rules](#rules), [engine and API](#engine-and-api), and [engine acceptance criteria](#acceptance-criteria-engine) are Agreed ([#19](https://github.com/brylie/LoomDSP/issues/19)). [Parameter mapping](#parameter-mapping) is Agreed.
- **Decision behind it:** [ADR-0014](../adr/0014-modulation-without-cables.md) (modulation without cables). Builds on [ADR-0013](../adr/0013-modules-carry-their-own-panel.md) (panels) and [ADR-0007](../adr/0007-polymorphic-signal-model.md) (signals).
- **Requirements:** FR-MOD-01 to FR-MOD-08, FR-LIB-12.
- **Issues:** [#19](https://github.com/brylie/LoomDSP/issues/19) (built-in modulation), [#20](https://github.com/brylie/LoomDSP/issues/20) (modulation range on controls), [#66](https://github.com/brylie/LoomDSP/issues/66) (parameter mapping).

## Goal

> Two LFOs, an envelope, and a macro knob shape the filter, the oscillator mix, and the delay time. The patch still shows the audio path clearly, and I can see at a glance what moves what.

## Terms

| Term | Meaning |
|---|---|
| **Assignment** | One link from an output (the source) to a parameter (the target), with a depth and a polarity. |
| **Base value** | The parameter's value set by its control, before modulation. |
| **Depth** | How far the source moves the parameter, from -1 to 1, as a share of the control's full range. |
| **Modulation matrix** | A table of every assignment in the current patch or sub-patch. |

## Behaviour

### Making an assignment

| Action | Result |
|---|---|
| Drag from an output port and drop on a control (on a tile or in the inspector) | Creates an assignment with depth 0.25. Dropping on a port still makes a cable. |
| Select a module, press **M** (modulate mode), then drag any control | Sets that module's first output as a source on the dragged control. The drag sets the depth. Press M or Escape to leave the mode. |
| Matrix: **Add**, then pick a source and a target | Creates an assignment from the table, by mouse or keyboard. |

### Seeing assignments

- **On the control:** the base value, an arc for each assignment's range, and a dot for the live value (FR-MOD-04). Unipolar assignments draw the arc on one side of the base. Bipolar assignments draw it on both sides.
- **On the source:** a small badge on the output port shows how many assignments it drives.
- **On hover:** pointing at a modulated control, or at a source badge, draws thin dashed lines between the source port and its targets, and highlights the tiles. They disappear when the pointer leaves. Selecting a module shows its lines too, following the same focus rules as cables.
- **In the inspector:** each parameter lists its assignments, with a depth slider and a remove button.
- **In the matrix:** every assignment, one per row.

### Editing and removing

- Drag an assignment's arc on the control to change its depth. Shift for fine steps.
- Right-click an arc, or press Delete in the matrix, to remove it.
- Deleting a module removes the assignments it is part of. Undo brings them back.

### The modulation matrix

A panel beside the inspector. Columns: on, source (module and output), target (module and parameter), depth, polarity. Sort and filter by module, or show only the selected module. Every cell can be edited by keyboard (NFR-UX-01).

*Why a matrix as well as arcs:* arcs are best for one control, the matrix for the whole patch. It is also the place to check for hidden routing, and the keyboard path for people who do not use a mouse.

## Rules

1. **Any output can be a source.** LFOs, envelopes, macros, sequencers, MIDI values, and audio oscillators alike. *Why:* "patch anything to anything" (ADR-0007). Audio-rate modulation is a sound, not a mistake.
2. **Modulation works in the control's range.** The engine maps the base value to the control's 0 to 1 range, adds `depth × source` for each assignment, clamps to 0 to 1, and maps back. *Why:* depth 0.1 means "a tenth of the knob" on every parameter. On a logarithmic parameter such as cutoff, that is a musical interval, not a few hertz. The arc on the knob shows exactly what the engine does.
3. **One mapping, shared.** Whether a parameter is linear or logarithmic is part of its `ParamSpec`, not guessed by the app. The inspector, panel controls, and engine all use the same mapping functions in `loom-core`. *Why:* if they disagreed, the arc would lie.
4. **Polarity comes from the source, unless the user overrides it.** Each output declares unipolar (0 to 1) or bipolar (-1 to 1). Defaults: envelopes, gates, and phases are unipolar; LFOs, audio, and pitch are bipolar. An assignment can force either, and the engine then remaps the source sample `s` before applying depth:

   | Source declares | Assignment uses | Value used |
   |---|---|---|
   | same as the assignment | (no override) | `s` |
   | bipolar | unipolar | `(s + 1) / 2` |
   | unipolar | bipolar | `2s - 1` |

   The arc follows the value used: a unipolar assignment draws from the base to `base + depth`, a bipolar one from `base - depth` to `base + depth`. Sources are never clamped; only the final parameter value is (rule 5). *Why:* an envelope should push a cutoff up from its base, an LFO should swing around it. A fixed, simple remap keeps the arc honest.
5. **Sum, then clamp.** Several assignments on one parameter add up before the clamp (FR-MOD-03). Whole-number parameters are rounded after the clamp. *Why:* predictable, and the clamp keeps filters stable.
6. **Per sample, only where needed.** A parameter with at least one assignment gets a per-sample buffer. Others stay one value per block. *Why:* modulation must be smooth at audio rate (FR-MOD-03), but most parameters are still most of the time.
7. **Smoothing applies to the base value only.** When a parameter has assignments and its `ParamSpec` sets a smooth time, the engine smooths the base in the control's 0 to 1 position, then adds the modulation unsmoothed (FR-MOD-05). The module reads that buffer and does not smooth it again. A parameter with no assignments keeps the smoothing it already had inside the module, so a knob that is not modulated sounds the same as before. The time lives on `ParamOptions`, set with `ParamSpec::smooth`. Built-in times: the filter cutoff and resonance are 10 ms; the add offset, multiply gain, and constant value are 5 ms. Every other parameter is unsmoothed. *Why:* a sharp envelope or audio-rate source must stay sharp. Smoothing it would change the sound. Keeping the old path for unmodulated knobs means existing patches do not change.
8. **Assignments are graph edges.** The compiler orders the source before the target, and treats a loop through an assignment like any feedback loop: it inserts a one-sample delay where needed (ADR-0002). When a cycle must be broken, it delays the assignment before it delays a cable, so a modulation loop does not slip the audio path. A cycle of cables only still needs a cable delay, so the compiler breaks those first. Then it does not delay an assignment that the cable delay has already taken out of the loop. A module assigned to its own parameter is a self-loop and runs one sample at a time. *Why:* the engine has one way to schedule, and feedback through modulation is a valid patch.
9. **Depth 0 adds nothing.** On a patch where the assignment does not insert a delay on a cable, the output is bit-identical to the same patch without that assignment, as long as the knob is not gliding. While a smoothed knob glides, the engine glides it in the control's 0 to 1 position (rule 7), and the module glides an unmodulated knob in its own units. For the built-in modules both follow the same curve, but rounding can differ in the last bits until the glide ends. When the first assignment on a parameter is added during a glide, the engine starts its glide from the value the module reports through `Module::param_glide`, so the parameter does not step. A module that does not report it steps to the knob's new value. *Why:* users set depth to 0 to compare, and expect silence from it.
10. **Assignments stay inside their patch level.** Source and target must be in the same patch or sub-patch. To reach inside a sub-patch, expose the parameter (FR-ENCAP-03). *Why:* a sub-patch must work the same wherever it is used.
11. **Assignments are part of the patch.** They are saved, undone, copied with their modules (when both ends are copied), and exported. *Why:* they change the sound.
12. **Inputs and assignments add up, in a fixed order.** Some modules have an input for the same quantity as a parameter, such as the filter's cutoff input for keyboard tracking. The order is:
    1. The engine computes the modulated value: base, plus assignments in the 0 to 1 range, clamped, mapped back, rounded for whole numbers (rules 2 and 5).
    2. The module applies its input to that value in the input's own units (for the filter, pitch at 0.1 per octave), then clamps to the parameter range again.

    The module's help page states how its inputs apply. *Why:* both are common, and neither should silently replace the other. A fixed order means the same base, cable, and assignments give the same value in every module.

## Parameter mapping

**Status:** Agreed ([#66](https://github.com/brylie/LoomDSP/issues/66)).

Every parameter maps between its value and its control's position, from 0 to 1. Rules 2 and 3 use this mapping.

| Mapping | Position `t` to value | Use |
|---|---|---|
| Linear (the default) | `min + t × (max - min)` | Most parameters. Whole-number parameters are always linear. |
| Logarithmic | `min × (max / min)^t` | Frequencies and times. Equal steps of the control give equal ratios, so a tenth of a 20 Hz to 20 kHz knob is about one octave anywhere on it. |

- **The mapping is part of `ParamSpec`.** A `ParamSpec` with `ParamOptions::DEFAULT` is linear; `.logarithmic()` makes it logarithmic. Nothing is guessed from the unit. *Why:* the module author knows how the parameter should feel. A guess can be wrong, and then the arc lies.
- **`loom-core` has the two functions:** `ParamSpec::normalize(value)` gives the position, and `ParamSpec::denormalize(position)` gives the value. The inspector, panel controls, and the engine's modulation use only these. *Why:* rule 3.
- **Every result is valid.** `normalize` clamps the value into range and returns a position from 0 to 1. `denormalize` clamps the position to 0 to 1 and returns the value as `ParamSpec::clamp` gives it: in range, and rounded for whole numbers. NaN in either becomes the default value. *Why:* callers never need to clamp again.
- **The registry refuses impossible mappings:** a logarithmic parameter that is a whole number, or whose minimum is 0 or less. *Why:* there is no logarithm of 0, and whole numbers must step evenly, one value per step.
- **Built-in parameters in Hz or seconds are logarithmic:** the filter cutoff and the envelope's attack, decay, and release times. All others are linear.
- **The mapping is not saved in patch files.** Patches store values, not positions. *Why:* changing a parameter's mapping later never changes how a saved patch sounds.

### Acceptance criteria: parameter mapping

- [x] `denormalize(normalize(v))` returns `v` (within 1e-4, relative) at the minimum, default, and maximum of linear and logarithmic parameters.
- [x] For cutoff (20 Hz to 20 kHz), position 0.5 is about 632 Hz, and each step of 0.1 multiplies the value by about 2.
- [x] Whole-number parameters round, and step one value at a time across the whole range.
- [x] Out-of-range values, positions, and NaN give valid results.
- [x] The registry refuses a logarithmic whole-number parameter and a logarithmic parameter with a minimum of 0 or less.
- [x] Every built-in parameter in Hz or seconds is logarithmic.
- [x] The inspector uses `normalize` and `denormalize`. The app no longer guesses a mapping.

### Out of scope

- Decibel and stepped mappings (FR-PANEL-04, P2).
- Skewed or custom curves.
- The engine's modulation itself ([#19](https://github.com/brylie/LoomDSP/issues/19)). It will use these functions.

## Engine and API

**Status:** Agreed ([#19](https://github.com/brylie/LoomDSP/issues/19)).

- The graph holds a list of assignments: source `OutputRef`, target module and parameter index, depth (-1 to 1), optional polarity override, enabled. Each assignment has a stable id. Several assignments on one parameter are applied in id order. Patch files keep assignments in that order, and loading gives them ids in file order, so a reloaded patch adds them the same way. Float sums depend on order.
- Each output declares a polarity on its `PortSpec`. Gate and phase default to unipolar. Audio, pitch, and modulation default to bipolar. An envelope output and a MIDI velocity output are unipolar. The engine remaps a source sample only when the assignment overrides that declaration (rule 4).
- The compiler gives each modulated parameter a buffer of `max_block` samples, allocated with the schedule. No allocation on the audio thread.
- Before a target module runs, the engine fills its modulated parameter buffers from the base value (smoothed in control space when the parameter has a smooth time) and the source buffers. Inside a feedback loop the fill is one frame; otherwise it is the whole block. A delayed assignment reads the one-sample tap, not the live output.
- Modules read a modulated parameter through `PortIo` as a per-sample slice. `PortIo::param_at` gives the value at a frame. A parameter with no assignments stays the value from `set_param`, one per block.
- A module that smooths a parameter reports the value it is gliding through with `Module::param_glide`, or `None` once it has settled. The engine reads it on the audio thread when a schedule swap gives that parameter its first assignment, and starts its own glide there (rule 9).
- Adding or removing an assignment recompiles the graph. Changing depth, polarity, or enabled takes effect at the next buffer, like a parameter change, and does not allocate on the audio thread.
- Deleting a module removes the assignments it is part of.
- Patch format (RON), using the same `(module id, port or parameter name)` pairs as cables. Old files omit `modulation` and still load. `polarity` is omitted when the assignment follows the source. `enabled: false` is written only when the assignment is off. The format stays at version 1. *Why:* LoomDSP is a pre-release alpha, so no released build needs to refuse the new field. The cost: an older build loads a modulated patch without its assignments and plays it unmodulated. Bump the version for format changes once LoomDSP has a release.

```ron
modulation: [
    (source: (1, "out"), target: (2, "cutoff"), depth: 0.25),
    (source: (3, "out"), target: (2, "cutoff"), depth: 0.4, polarity: Unipolar),
]
```

## Example: a macro

A **Macro** module (FR-LIB-12) has one knob on its panel and one output equal to that knob. Assign its output to cutoff, resonance, and drive with different depths. One knob now opens the whole sound, and it can go on the front panel.

## P1 slice

### Acceptance criteria: engine

**Status:** Agreed ([#19](https://github.com/brylie/LoomDSP/issues/19)).

- [x] A constant source of 1.0 with depth 0.5 on a linear parameter with base 0 gives the middle of the range.
- [x] Depth 0.1 on cutoff (20 Hz to 20 kHz, logarithmic) moves it by a tenth of the knob, about one octave.
- [x] Two assignments sum before the clamp. The result never leaves the parameter range. Whole-number parameters stay whole.
- [x] Depth 0 output is bit-identical to no assignment, on a patch where the assignment does not delay a cable.
- [x] Output is the same at any block size, with assignments present.
- [x] Adding and removing assignments during playback allocates nothing on the audio thread (`no_alloc` test).
- [x] A loop through an assignment gets a one-sample delay and plays.
- [x] Assignments survive save and load. A patch saved before modulation still loads.
- [x] Deleting a source or target module removes its assignments.

### Acceptance criteria: editing and display

**Status:** Draft. UI issues [#70](https://github.com/brylie/LoomDSP/issues/70) and [#71](https://github.com/brylie/LoomDSP/issues/71).

- [ ] Dragging an output onto a control creates an assignment. Undo removes it.
- [ ] Modulated controls show base, range arc, and live value.
- [ ] Hovering a modulated control draws its source lines. The source port shows a badge.
- [ ] The matrix lists every assignment, and depth can be edited by keyboard.
- [ ] Undo restores assignments removed by deleting a module.

### Out of scope for P1

- Modulating an assignment's depth (FR-MOD-02, P2).
- Per-voice modulation (with polyphony, P2).
- Response curves on assignments (for example exponential depth).
- Assignments across sub-patch boundaries. Not planned: expose the parameter instead.
- MIDI learn. It maps a controller to a base value, a separate feature ([#24](https://github.com/brylie/LoomDSP/issues/24)).

## Open points

- The M key and the default depth of 0.25 are guesses. Try them with users.
- Should an assignment be able to target another assignment's depth directly, or through a "depth" parameter on a VCA-style module? Decide with FR-MOD-02.
- How the matrix shows assignments inside sub-patches: hidden, or listed read-only under the sub-patch.
