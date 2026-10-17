# MIDI Learn Specification

How a performer links a hardware knob or fader to any parameter in a few seconds, and how those links are saved, shown, and removed.

- **Kind:** living specification.
- **Status:** Draft. Do not build from it yet. The [open points](#open-points) need answers first.
- **Decision behind it:** none of its own. Builds on [ADR-0001](../adr/0001-lock-free-control-audio-boundary.md) (control and audio threads), [ADR-0004](../adr/0004-export-as-runtime-plus-patch-data.md) (export as runtime plus patch data), [ADR-0008](../adr/0008-patch-format-ron.md) (patch format), and [ADR-0013](../adr/0013-modules-carry-their-own-panel.md) (the inspector holds MIDI learn).
- **Use case and requirements:** [UC3, perform live](../product/personas-and-use-cases.md#uc3-perform-live-tomas-highest-priority-use-case), FR-MIDI-02. Related, not built here: FR-MIDI-06 (per-device mappings), FR-MIDI-09 (pickup, relative encoders, scaling).
- **Background:** [parameter mapping](modulation.md#parameter-mapping) (value to position and back).
- **Prior art:** most DAWs and software modular tools have a "learn" gesture: arm a control, move a hardware control, and the two are linked. We use the idea, in our own words.
- **Issues:** [#24](https://github.com/brylie/LoomDSP/issues/24).
- **Code:** not built yet.

<!-- Spec first: agree on this before building. Keep it true as the code is
     built. See README.md in this folder. -->

## Goal

> I touch the filter cutoff, turn knob 3 on my controller, and from then on knob 3 moves the cutoff. When I save the patch and open it tomorrow, knob 3 still moves the cutoff. If I change my mind, I remove the link in the inspector.

## Terms

| Term | Meaning |
|---|---|
| **Mapping** | One link from a MIDI control change (a channel and a controller number) to one parameter. |
| **Learn** | The state where the app waits for the next control change, to make a mapping for one armed parameter. |
| **Armed parameter** | The one parameter that the next control change will be mapped to. |

## Behaviour

### Making a mapping

| Action | Result |
|---|---|
| Inspector: press **Learn** next to a parameter | That parameter is armed. Its row is highlighted and the button reads **Waiting for MIDI…**. Any parameter armed before is disarmed. |
| Move a hardware knob or fader while a parameter is armed | A mapping is made from that control change (channel and controller number) to the armed parameter. Learn ends. The row shows the mapping, for example `CC 74 · ch 1`. The status bar says which control was mapped. |
| Press **Learn** again, press Escape, or select another module | Learn ends with no change. |

- **Only one parameter is armed at a time.** *Why:* the next control change must have exactly one target, so the result is never a surprise.
- **Learning a parameter that already has a mapping replaces it.** A parameter has at most one mapping. *Why:* two hardware controls on one parameter fight each other and make the value jump.
- **One hardware control can drive several parameters.** Learning a control that already has a mapping adds a second mapping. It does not move the first. *Why:* one knob that opens a filter and raises a drive at the same time is a common live gesture. Removing the link is one click if it was a mistake.
- **The control change that ends learn also sets the value.** *Why:* the user sees and hears at once that the link works.

### Playing

- A mapped control change sets the parameter's **base value**, the same value the knob on the tile sets. Modulation still adds on top of it ([modulation](modulation.md), rule 2).
- The value 0 to 127 becomes a position from 0 to 1 (`value / 127`), and then a value with `ParamSpec::denormalize`. So 0 is the minimum, 127 is the maximum, and a logarithmic parameter feels the same on hardware as on screen. Whole-number parameters round, as `denormalize` already does.
- The tile knob and the inspector follow the hardware, so the screen always shows the value that plays.
- Moving the knob on screen still works. The last change wins, from either side. A mapped parameter can jump when the hardware control moves after a screen change. Pickup modes that prevent the jump are FR-MIDI-09, P2.
- A parameter with smoothing (`smooth_seconds`) glides to the new value as it does for a screen knob. *Why:* a 7-bit control change moves in steps of about 0.8 %. Without smoothing, a step on a cutoff or a level can be heard as a click.

### Seeing and removing mappings

- The inspector shows a parameter's mapping on its row, next to the **Learn** button, as `CC <number> · ch <channel>`, with channels counted from 1 as on hardware. A **Remove** button deletes the mapping.
- Deleting a module deletes its mappings.

## Rules

1. **Mappings run in the engine.** The controller compiles the patch's mappings into a table and sends it to the audio thread with the schedule. The engine reads each incoming control change, looks it up, and sets the base value. *Why:* the same patch must respond to the same controller in the desktop app, in `loom play`, and in an exported plugin (ADR-0004), none of which can rely on the app's UI thread. It also adds no latency beyond the audio buffer.
2. **The engine reports values back to the controller.** After each block, the engine writes the latest value of each mapped parameter that changed into a latest-value table, like display values ([panels](panels.md)). The controller reads it and updates its graph. *Why:* the graph is what the UI draws and what is saved. A latest-value table loses nothing that matters if the UI is slow, and never makes the audio thread wait.
3. **Learn reads control changes from the engine, too.** The engine records the last control change it received (channel, controller number, and a counter that goes up with each one). The controller reads it while a parameter is armed. *Why:* every host sends MIDI to the engine already. Learn then works the same way in every host, with no second MIDI path.
4. **A mapped control change applies at the start of the block it arrives in**, like a screen knob change. *Why:* an unmodulated parameter is constant for a block, and the desktop host stamps live MIDI at frame 0 today. NFR-PERF-07 allows timestamps inside the buffer. Output stays deterministic for the same input and block size. See the open point on timing.
5. **Mappings are saved with the patch.** A new list in the patch file, empty when missing. The patch format stays at version 1. *Why:* the issue asks for it, and a patch should play the same way when opened again. Per-device mappings that live outside patches are FR-MIDI-06, P2.
6. **Mapped control changes still reach modules.** A MIDI input module (#30) still sees every control change. *Why:* hiding messages that a mapping uses is a surprise that is hard to debug. If this causes trouble in use, a later change can add a "consume" option.
7. **Real-time rules hold.** The mapping table is built on the control thread. The engine only reads it, with no allocation, lock, or wait (hard rule 1). A full latest-value table is not possible: it has one entry per mapping.

### Patch format

```ron
midi: [
    (target: (3, "cutoff"), channel: 0, controller: 74),
    (target: (3, "resonance"), channel: 0, controller: 71),
],
```

- `channel` is 0 to 15, as in MIDI. The inspector shows 1 to 16.
- A mapping whose module or parameter does not exist is a load error, the same as a cable to a port that does not exist.
- Two mappings on the same parameter are a load error. *Why:* the learn gesture never makes them (a parameter has one mapping), so a file with them was edited by hand by mistake.

## Acceptance criteria

- [ ] Given an armed parameter, when a control change arrives, then one mapping is made, learn ends, and the parameter takes the control's value.
- [ ] Given a mapping, when its control change arrives, then the parameter's base value is `denormalize(value / 127)`: the minimum at 0, the maximum at 127, and the middle of a logarithmic range at 64 (within one step).
- [ ] A control change on another channel or controller number does not move the parameter.
- [ ] Learning a mapped parameter replaces its mapping. Learning a control that is already mapped adds a second mapping, and both parameters follow it.
- [ ] Escape, a second press of **Learn**, or selecting another module ends learn with no change.
- [ ] The tile knob and the inspector show the value set by the hardware within one UI frame.
- [ ] A patch with mappings saves and loads with the same mappings. A patch without a `midi` list loads with none.
- [ ] Removing a mapping in the inspector stops the control from moving the parameter. Deleting a module removes its mappings.
- [ ] `loom play` with a patch that has mappings responds to the controller with no UI.
- [ ] Processing a mapped control change does not allocate (`tests/no_alloc.rs`).

## Out of scope

- Per-device mappings saved outside the patch (FR-MIDI-06, P2).
- Pickup (soft takeover), relative encoders, value range, and inversion (FR-MIDI-09, P2).
- Learning notes, pitch bend, aftertouch, program change, 14-bit control changes, and NRPN. Pitch bend and aftertouch reach a patch through the MIDI input module (#30).
- Learning from the tile with a right-click or a global learn mode. The inspector is the place for MIDI learn (ADR-0013).
- Undo of mapping edits. They join undo when [#12](https://github.com/brylie/LoomDSP/issues/12) lands.
- Mapping to modulation depth, transport controls, or app actions.
- MIDI out and feedback to motorised faders or LED rings.

## Open points

1. **Engine or app?** Rule 1 puts mappings in the engine so `loom play` and exported plugins work. The other way is to read control changes on the app side and call `Controller::set_param`, which is less work now but works only in the desktop app. *Leaning:* engine.
2. **Learn gesture.** The issue says "touch a parameter". This draft arms from an inspector button only. Should touching a tile knob arm it while a global learn mode is on? *Leaning:* inspector only for P1, a tile gesture later.
3. **One control, several parameters.** This draft allows it (Behaviour). The other way is that learning a control already in use moves its mapping. *Leaning:* allow it.
4. **Do mapped control changes still reach modules?** Rule 6 says yes. *Leaning:* yes.
5. **Timing inside a block.** Rule 4 applies a control change at the start of its block. An offline render that feeds control changes at frame offsets would then depend on the block size. Today no render path feeds control changes, so this only matters later. *Leaning:* block start now. Revisit when renders take MIDI files.
6. **Any channel?** This draft maps one channel and controller number. Some users want "this controller number on any channel". *Leaning:* one channel; most controllers send on one channel.
