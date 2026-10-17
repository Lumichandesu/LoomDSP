# MIDI Learn Specification

How a performer links a hardware knob or fader to any parameter in a few seconds, and how those links are saved, shown, and removed.

- **Kind:** living specification.
- **Status:** Agreed.
- **Decision behind it:** none of its own. Builds on [ADR-0001](../adr/0001-lock-free-control-audio-boundary.md) (control and audio threads), [ADR-0004](../adr/0004-export-as-runtime-plus-patch-data.md) (export as runtime plus patch data), [ADR-0008](../adr/0008-patch-format-ron.md) (patch format), and [ADR-0013](../adr/0013-modules-carry-their-own-panel.md) (the inspector shows and removes mappings; learn mode extends the gesture to tile controls).
- **Use case and requirements:** [UC3, perform live](../product/personas-and-use-cases.md#uc3-perform-live-tomas-highest-priority-use-case), FR-MIDI-02. Related, not built here: FR-MIDI-06 (per-device mappings), FR-MIDI-09 (pickup, relative encoders, scaling).
- **Background:** [parameter mapping](modulation.md#parameter-mapping) (value to position and back).
- **Prior art:** most DAWs and software modular tools have a "learn" gesture: arm a control, move a hardware control, and the two are linked. We use the idea, in our own words.
- **Issues:** [#24](https://github.com/brylie/LoomDSP/issues/24).
- **Code:** `crates/loom-core/src/midi_map.rs` (mappings and feedback), `Engine::apply_midi_maps`, the `Controller` MIDI methods, the `midi` list in `loom-patch`, `crates/loom-app/src/learn.rs` (learn mode), and `LearnView` in `loom-canvas`. Tests: `crates/loom-modules/tests/midi_learn.rs`, and the learn mode tests in `loom-canvas`'s `widget.rs`.

<!-- Spec first: agree on this before building. Keep it true as the code is
     built. See README.md in this folder. -->

## Goal

> I touch the filter cutoff, turn knob 3 on my controller, and from then on knob 3 moves the cutoff. When I save the patch and open it tomorrow, knob 3 still moves the cutoff. If I change my mind, I remove the link in the inspector, or learn knob 3 onto another control.

## Terms

| Term | Meaning |
|---|---|
| **Mapping** | One link from a MIDI control change (a channel and a controller number) to one parameter. |
| **Learn mode** | A mode of the app where touching a control picks it for mapping instead of changing its value. |
| **Armed parameter** | The one parameter that the next control change will be mapped to. |

## Behaviour

### Making a mapping

The user touches the control on screen, then moves the hardware control. Learn mode makes the touch safe: it arms the control and does not change its value.

| Action | Result |
|---|---|
| Press **MIDI learn** in the toolbar | Learn mode starts. The button stays lit. Every control bound to a parameter, on tiles and in the inspector, gets a learn outline. Mapped controls show their mapping as a small label, for example `CC 74`. |
| In learn mode, click a knob, toggle, or radio button on a tile, or a parameter row in the inspector | That parameter is armed. The control is highlighted and the status bar says **Move a MIDI control to map it to <module> <parameter>**. The value does not change. Any parameter armed before is disarmed. |
| Move a hardware knob or fader while a parameter is armed | A mapping is made from that control change (channel and controller number) to the armed parameter. The status bar says which control was mapped. Nothing is armed now, and learn mode stays on, so the next control can be mapped at once. |
| In learn mode, click an armed control again | It is disarmed. |
| In learn mode, press Space or Enter on a control focused with Tab | The same as a click: it arms or disarms the control. The arrow keys do nothing. With no control focused, Space still starts and stops the transport, because that is not a parameter. |
| Press **MIDI learn** again, or press Escape | Learn mode ends. An armed parameter is disarmed with no change. |

- **Touch first, then the hardware.** *Why:* the user points at what they want to control and then shows which hardware control does it. This is direct manipulation, and it is the order the user thinks in. Learning from a list in the inspector alone is harder to follow.
- **In learn mode, nothing on screen changes a value.** Not a click, a drag, or a key. Only the hardware control that makes or uses a mapping does. *Why:* the mode exists so that touching a control on screen is safe.
- **Learn mode stays on after a mapping.** *Why:* the common task is to map several knobs in a row (the P1 goal maps four). Leaving the mode after each one would add a click per knob.
- **Outside learn mode, touching a control only changes its value,** as today. *Why:* moving a hardware control after touching a screen knob is normal in play. It must never remap anything by accident.
- **Only one parameter is armed at a time.** *Why:* the next control change must have exactly one target, so the result is never a surprise.
- **One hardware control drives one parameter, and a parameter has one hardware control.** Learning a parameter that already has a mapping replaces it. Learning a hardware control that already has a mapping moves it to the new parameter, and the status bar names the parameter it left. *Why:* this is the least confusing rule. What a hardware knob does is always one thing, and the label on screen tells the whole story. Two hardware controls on one parameter would fight and make the value jump. One knob that drives many parameters is a job for the Macro module (MT-UI-02, P2): map the hardware to the macro, and the macro drives many parameters through modulation, where depth and the modulation views show the fan-out.
- **The control change that makes a mapping also sets the value.** *Why:* the user sees and hears at once that the link works.

### Playing

- A mapped control change sets the parameter's **base value**, the same value the knob on the tile sets. Modulation still adds on top of it ([modulation](modulation.md), rule 2).
- The value 0 to 127 becomes a position from 0 to 1 (`value / 127`), and then a value with `ParamSpec::denormalize`. So 0 is the minimum, 127 is the maximum, and a logarithmic parameter feels the same on hardware as on screen. Whole-number parameters round, as `denormalize` already does.
- The tile knob and the inspector follow the hardware, so the screen always shows the value that plays.
- Moving the knob on screen still works. The last change wins, from either side. A mapped parameter can jump when the hardware control moves after a screen change. Pickup modes that prevent the jump are FR-MIDI-09, P2.
- A parameter with smoothing (`smooth_seconds`) glides to the new value as it does for a screen knob. *Why:* a 7-bit control change moves in steps of about 0.8 %. Without smoothing, a step on a cutoff or a level can be heard as a click.

### Seeing and removing mappings

- The inspector shows a parameter's mapping on its row as `CC <number> · ch <channel>`, with channels counted from 1 as on hardware. A small **×** button next to it deletes the mapping ([removing an item](panels.md#removing-an-item)). This works in and out of learn mode.
- In learn mode, the labels on the tiles show every mapping in view at a glance. *Why:* with one parameter per hardware control, the labels are a full picture of the mappings. A separate mapping list is not needed in P1.
- Deleting a module deletes its mappings.

## Rules

1. **Mappings run in the engine.** The controller compiles the patch's mappings into a table and sends it to the audio thread with the schedule. The engine reads each incoming control change, looks it up, and sets the base value. *Why:* the same patch must respond to the same controller in the desktop app, in `loom play`, and in an exported plugin (ADR-0004), none of which can rely on the app's UI thread. It also adds no latency beyond the audio buffer.
2. **The engine reports values back to the controller.** When a mapped control change arrives, the engine writes it into a latest-value table: one entry per control (16 channels × 128 controller numbers), holding the mapping's id, a sequence number, and the 7-bit value. The controller reads the table, turns the value into a parameter value with the same mapping, and updates its graph. Three ordering rules keep the graph and the engine in step:
   - The controller reads the table before it compiles, because compiled base values come from the graph.
   - A screen edit on a mapped parameter carries a token. When the engine applies it, it stores the token with a sequence number that it shares with control changes. Until then, the controller ignores control changes for that parameter, and after it, applies only those the engine applied later. So an older control change never overwrites a newer edit, and a newer one always wins.
   - A control change that reaches the engine between a compile and the schedule swap is applied again to the new schedule. Screen edits made in that time are queued after the swap, so they still win.

   *Why:* the graph is what the UI draws and what is saved. A latest-value table loses nothing that matters if the UI is slow, and never makes the audio thread wait. The id says which mapping wrote an entry, so a write from a mapping that no longer exists is ignored. The sequence number shows the same value sent twice.
3. **Learn reads control changes from the engine, too.** The engine records the last control change it received (channel, controller number, and a counter that goes up with each one). The controller reads it while a parameter is armed. *Why:* every host sends MIDI to the engine already. Learn then works the same way in every host, with no second MIDI path.
4. **A mapped control change applies at the start of the buffer it arrives in**, like a screen knob change. *Why:* an unmodulated parameter is constant for a block, and one block is a few milliseconds, too short to hear on a knob move. The desktop host stamps live MIDI at frame 0 today anyway. Output stays deterministic for the same input and block size. An offline render that feeds control changes at frame offsets would depend on the block size. No render path feeds control changes yet; revisit this rule when one does.
5. **A mapping matches one channel and one controller number,** the ones the user moved while learning. *Why:* what the user moved is what works. Most controllers send each knob on one channel, so this needs no setting. Two controllers that send the same controller number on different channels stay apart.
6. **Mapped control changes still reach modules.** A MIDI input module (#30) still sees every control change. *Why:* hiding messages that a mapping uses is a surprise that is hard to debug. If this causes trouble in use, a later change can add a "consume" option.
7. **Mappings are saved with the patch.** A new list in the patch file, empty when missing. The patch format stays at version 1. *Why:* the issue asks for it, and a patch should play the same way when opened again. Per-device mappings that live outside patches are FR-MIDI-06, P2.
8. **Real-time rules hold.** The mapping table is built on the control thread and sent with the schedule. The latest-value table is a fixed table of atomics, allocated once when the engine is created. The engine reads the mapping table and stores into the latest-value table with no allocation, lock, or wait (hard rule 1).

### Patch format

```ron
midi: [
    (target: (3, "cutoff"), channel: 0, controller: 74),
    (target: (3, "resonance"), channel: 0, controller: 71),
],
```

- `channel` is 0 to 15, as in MIDI. The inspector shows 1 to 16.
- A mapping whose module or parameter does not exist is a load error, the same as a cable to a port that does not exist.
- Two mappings on the same parameter, or two on the same channel and controller number, are a load error. *Why:* learning never makes them (one hardware control, one parameter), so a file with them was edited by hand by mistake.

## Acceptance criteria

- [x] Pressing **MIDI learn** starts learn mode. Clicking a tile control or an inspector row in learn mode arms its parameter and does not change its value.
- [x] Given an armed parameter, when a control change arrives, then one mapping is made, the parameter takes the control's value, nothing is armed, and learn mode stays on.
- [ ] Outside learn mode, clicking a control and then moving a hardware control makes no mapping.
- [x] Given a mapping, when its control change arrives, then the parameter's base value is `denormalize(value / 127)`: the minimum at 0, the maximum at 127, and the middle of a logarithmic range at 64 (within one step).
- [x] A control change on another channel or controller number does not move the parameter.
- [x] Learning a mapped parameter replaces its mapping. Learning a hardware control that is already mapped moves the mapping to the new parameter, and the status bar names the old one.
- [x] Clicking an armed control disarms it. Escape or a second press of **MIDI learn** ends learn mode with no change.
- [ ] In learn mode, every mapped control in view shows its `CC` label.
- [ ] The tile knob and the inspector show the value set by the hardware within one UI frame.
- [x] A mapped control change still reaches a MIDI input module.
- [x] A patch with mappings saves and loads with the same mappings. A patch without a `midi` list loads with none. A patch with two mappings on one parameter, or on one channel and controller number, fails to load with a clear error.
- [x] Removing a mapping in the inspector stops the control from moving the parameter. Deleting a module removes its mappings.
- [ ] `loom play` with a patch that has mappings responds to the controller with no UI.
- [x] Processing a mapped control change does not allocate (`tests/no_alloc.rs`).

## Out of scope

- One hardware control driving many parameters. Use the Macro module (MT-UI-02, P2) with modulation.
- A mapping list or matrix. In P1 the labels in learn mode and the inspector rows are enough, because each hardware control has one parameter.
- Per-device mappings saved outside the patch (FR-MIDI-06, P2).
- Pickup (soft takeover), relative encoders, value range, and inversion (FR-MIDI-09, P2).
- Learning notes, pitch bend, aftertouch, program change, 14-bit control changes, and NRPN. Pitch bend and aftertouch reach a patch through the MIDI input module (#30).
- Mapping on any channel ("omni").
- Undo of mapping edits. They join undo when [#12](https://github.com/brylie/LoomDSP/issues/12) lands.
- Mapping to modulation depth, transport controls, or app actions.
- MIDI out and feedback to motorised faders or LED rings.

## Open points

None. Answered in review: mappings run in the engine; learning starts by touching the control in learn mode; one hardware control maps to one parameter, with the Macro module for one to many; mapped control changes still reach modules; changes apply at the start of the block; a mapping matches one channel.
