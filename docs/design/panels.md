# Panels Specification

How modules, sub-patches, and patches show controls and displays, and how users build them.

- **Kind:** living specification. It changes as panels are built and used.
- **Status:** Draft, except [Module panels on tiles](#module-panels-on-tiles-p1) and [Level meters and audio load](#level-meters-and-audio-load-p1), which are Implemented. [Labels and hover text](#labels-and-hover-text) is Implemented.
- **Decision behind it:** [ADR-0013](../adr/0013-modules-carry-their-own-panel.md) (modules carry their own panel).
- **Requirements:** FR-PANEL-*, FR-CANVAS-10, FR-CANVAS-11, FR-LIB-18, FR-VIS-01 to 04, FR-ENCAP-03, NFR-PERF-04, NFR-UX-01.
- **Builds on:** [Telemetry](telemetry.md) for live data, [ADR-0010](../adr/0010-own-patch-canvas.md) for the canvas, [parameter mapping](modulation.md#parameter-mapping) for control values.
- **Issues:** [#7](https://github.com/brylie/LoomDSP/issues/7) (umbrella), [#67](https://github.com/brylie/LoomDSP/issues/67) (panel declarations), [#68](https://github.com/brylie/LoomDSP/issues/68) (controls on tiles), [#69](https://github.com/brylie/LoomDSP/issues/69) (display values), [#9](https://github.com/brylie/LoomDSP/issues/9) (level meter, audio load).

## One panel model, three places

| Place | Who builds the panel | Where it shows |
|---|---|---|
| Built-in module | The module author, as data in the module's descriptor | On the module's tile |
| Sub-patch | The user, in the panel editor | On the sub-patch's tile in the parent patch, and as the sub-patch's front panel |
| Patch | The user, in the same panel editor | As the patch's front panel: performance mode, and later the plugin GUI and standalone app |

The rule that ties them together: **a sub-patch's front panel is its tile.** When you finish a sub-patch, its tile in the parent looks and works like a built-in module. *Why:* abstraction should feel the same at every level. Hiding wiring behind good controls is the same job whether the result is one filter or a whole instrument.

## Widgets

Every panel uses the same widget set.

**Controls** (bound to a parameter):

| Widget | Use |
|---|---|
| Knob | Continuous values. Shows modulation depth as an arc (FR-MOD-04). |
| Slider, horizontal or vertical | Continuous values where a long throw helps. |
| Toggle | On or off. |
| Radio buttons | A few choices shown at once, such as filter slopes 6, 12, 18, 24 dB. |
| Selector | Many choices, as a menu. |
| Button | A momentary action or trigger. |
| XY pad | Two parameters at once. |

**Displays** (bound to live data or module data):

| Widget | Shows |
|---|---|
| Light | One value as brightness, or on and off. |
| Meter | A level, from a port probe or a display value. |
| Scope | A signal over time, from a port probe. |
| Step grid | A pattern, and the playing step. Editable when bound to module data. |
| Graph | A shape: envelope, curve, filter response. Editable when bound to module data. |
| Label | Static text. |

*Why one set:* users learn each control once. A new widget is added to the set, not drawn by one module, so every module and every panel can use it.

## Bindings

- **Control to parameter.** A control takes its range, unit, and mapping (linear, logarithmic, whole numbers) from the parameter's `ParamSpec`, through `ParamSpec::normalize` and `denormalize` ([parameter mapping](modulation.md#parameter-mapping), Agreed). *Why:* the slider mapping work in the inspector (logarithmic Hz, snapping integers) then applies everywhere for free.
- **Display to port.** A scope or meter reads a port probe through the telemetry queues.
- **Display to module value.** A light or step position reads a **display value**: a small, fixed set of numbers a module publishes once per block. *Why:* a step sequencer's position is internal state, not a port, and it should not need a cable to be seen.
- **Editor to module data.** A step grid or graph that edits a pattern writes module data (OQ-17), with undo.
- **One parameter, many controls.** A parameter can have a knob on its tile and another on the front panel (FR-PANEL-03). Both move together.

## Layout

- Panels are laid out on the same snap grid as the canvas. A tile's size is its panel's size, in grid units.
- Ports stay on the tile's left and right edges, outside the panel area. *Why:* cables must stay easy to read, whatever the panel holds.
- A tile can have a compact and an expanded size. The scope module, for example, can be resized to show more detail.
- A sub-patch panel can be placed inside its parent's panel (FR-PANEL-08), as one more widget.

## The panel editor

Used for sub-patches and patches alike.

- Open it from a patch or a sub-patch. The same tools appear in both cases.
- Place widgets on the grid; snap, align, group, resize (FR-PANEL-05).
- Bind a control by dragging a parameter from the inspector or from a module onto it. Binding a control inside a sub-patch exposes that parameter as the sub-patch's own (FR-ENCAP-03).
- Leaving the editor shows the result at once: on the tile in the parent, or as the front panel.

Built-in module panels are written as data in code, in the same format the editor saves.

## Interaction on the canvas

| Gesture | On a control | On a display or empty panel area | On the tile header |
|---|---|---|---|
| Drag | Changes the value | Moves the tile | Moves the tile |
| Click | Toggle, radio, button: acts | Selects the tile | Selects the tile |
| Double-click | Resets to the default | Opens the larger editor, if the module has one | Opens a sub-patch |
| Shift-drag | Fine adjustment | | |

- Every control is reachable by keyboard, with visible focus (NFR-UX-01).
- Editing a control on a tile also selects that module, so the inspector follows.

## Level of detail

| Zoom | Controls | Displays |
|---|---|---|
| Close | Full widget with label and value | Full detail |
| Middle | Widget shape only, no text | Simplified (a scope becomes its level) |
| Far | Not drawn; the tile is a plain shape | Not drawn |

*Why:* large patches stay readable and fast (FR-CANVAS-11). At far zoom you read the structure, not the values.

## Redraw

**Draw a frame when something visible changes.**

- Graph edits, camera moves, hover, and control drags redraw.
- A display redraws when its data changes and the tile is on screen at a zoom where the display is drawn.
- A scope on screen changes every frame, so it animates every frame. A step light changes a few times a second, so it redraws a few times a second.
- Nothing on screen changing means no redraw.

*Why:* GPU time is not the limit (the canvas holds 120 fps with 300 modules). Battery and heat are, and a laptop on stage often runs on battery. Drawing only what changed keeps the cost in line with what the user is looking at.

Measure on the reference laptop: idle CPU and GPU with nothing changing, and CPU, GPU, and power with four scopes on screen.

## Cables do not animate

Cables do not pulse or change brightness with the signal they carry. Signal activity (FR-VIS-01) is shown on ports, as a steady state such as "carrying signal" or "silent", updated a few times a second. *Why:* when everything moves, nothing stands out, and constant motion tires the eyes. Hover focus already brings the cables you care about to the front.

## The inspector

The inspector stays, as the place for:

- every parameter, including those not on the tile;
- typing exact values;
- modulation ranges and other settings that do not fit on a control;
- MIDI mappings. Learn mode can arm controls on tiles or in the inspector. The inspector shows each mapping and can remove it.

### Removing an item

**Status:** Agreed.

Lists in the inspector, such as a parameter's MIDI mapping or its modulation assignments, share one remove control.

- A small **×** button at the end of the item's row removes that item. Hovering it shows what it removes, for example "Remove MIDI mapping". *Why:* one small, familiar mark in the same place on every row is quick to find and takes little room in a narrow inspector. The hover text says exactly what goes, because a bare × does not.
- It acts at once, with no confirmation. *Why:* removing a mapping or an assignment is quick to redo, and undo ([#12](https://github.com/brylie/LoomDSP/issues/12)) will bring it back.
- Used by [MIDI learn](midi-learn.md#seeing-and-removing-mappings) and by modulation's [remove button](modulation.md#seeing-assignments).

## Scope module

The first display module.

- One audio input, and an optional pass-through output so it can sit inline.
- A resizable tile with a scope display. Parameters: time window and trigger level.
- **Add scope:** a one-step gesture on a cable or an output that places a scope module next to it, already connected. *Why:* a probe should be as quick as a glance, but the result is an ordinary module you can keep, move into a sub-patch, or put on a front panel.

## Module panels on tiles (P1)

**Status:** Agreed ([#7](https://github.com/brylie/LoomDSP/issues/7)). This section turns the general rules above into what P1 builds for built-in modules. Where the two differ, this section wins for P1.

### Panel declaration ([#67](https://github.com/brylie/LoomDSP/issues/67))

**Status:** Implemented. Code: `Panel`, `Widget`, and `ModuleDescriptor::check_panel` in `loom-core/src/descriptor.rs`.

- A `ModuleDescriptor` has a `panel`: its size in grid units (width and height), and a list of widgets. A module with nothing on its tile has an empty panel.
- Each widget has a kind, a grid position inside the panel, and a binding by name: a parameter for controls, a display value for lights. *Why names:* parameter names are stable IDs (AGENTS.md rule 5), and the P2 panel editor will save bindings the same way.
- P1 widgets. Most are 3 grid units wide. The button is wider, so a performance control can be large.

| Widget | Size (grid units) | Binds to | Behaviour |
|---|---|---|---|
| Knob | 3 × 3 | Any parameter | A dial with its label below. Uses the parameter's mapping (`ParamSpec::normalize`). |
| Toggle | 3 × 2 | A whole-number parameter from 0 to 1 | Off or on, with its label below. |
| Radio buttons | 3 × (options + 1) | A whole-number parameter with exactly one value per option | One row per option, top option = minimum value. Label above. |
| Button | 6 × 3 | A whole-number parameter from 0 to 1, and a different whole-number `mode` parameter from 0 to 1 | A large button. Its fill shows the bound parameter. Gestures are under [Interaction](#interaction-68). |
| Light | 3 × 2 | A display value | Brightness is the value clamped to 0 to 1. Label below. |

- **The registry refuses a panel that does not fit its module:** a binding to a parameter or display value the module does not have, a toggle, radio, or button on a parameter that does not match it, a button whose mode is missing, out of range, or the same parameter, a widget outside the panel, or two widgets that overlap. *Why:* a third-party module gets the same checks as a built-in one, at registration, not as a broken tile later.
- **Built-in panels:** a knob for every parameter except the noise seed, MIDI Note's channel and bend range, and Button's `mode`, which stay in the inspector. Button's `on` is the button, not a knob. MIDI Note has a light for its gate, and ADSR a light for its level. Toggle and radio buttons are built and tested, but no P1 module uses them yet.

### Display values ([#69](https://github.com/brylie/LoomDSP/issues/69))

**Status:** Implemented. Code: `loom-core/src/display.rs`, `Module::displays`, `Controller::display`.

- A descriptor lists its display values by name, at most 4. A module writes them in `Module::displays`, which the engine calls once after each block for every module that has any.
- They travel in a **latest-value table**: 4 atomic floats per module slot, allocated once when the engine is created. The engine stores, the UI loads whenever it draws. Nothing waits, nothing allocates, and the table can never fill up. *Why not the meter queue:* only the latest value matters for a light or a step position, as [telemetry.md](telemetry.md) notes for this kind of data.
- When a module is added to a slot, its values are set to 0 first, so a new module never shows the values of the one that was there before.
- The UI reads them by module ID and index through the `Controller`.
- Display values never change the audio. *Why:* determinism (AGENTS.md rule 3).

### Tile layout ([#68](https://github.com/brylie/LoomDSP/issues/68))

**Status:** Implemented. Code: `ModuleView::width`, `widget_bounds`, and `tile_size` in `loom-canvas/src/scene.rs`.

- **The panel sits between the port columns.** Each side has a port label column 4 grid units wide. A label longer than its column is cut with "…", and the port's [hover text](#labels-and-hover-text) shows it in full.
- Tile width is 4 + panel width + 4 grid units, and at least 11 (today's width), so titles still fit. Tile height is the header plus the taller of the port rows and the panel.
- A tile with an empty panel keeps the old label layout: each side gets half the tile. *Why:* nothing sits between the columns, so cutting labels would hide text for no reason.
- *Why:* tiles stay compact, and ports and cables keep their place at the tile edges.

### Interaction ([#68](https://github.com/brylie/LoomDSP/issues/68))

**Status:** Implemented. Code: `loom-canvas/src/widget.rs`, `controls.rs`.

| Gesture | Knob | Toggle, radio | Light, empty panel area | Header |
|---|---|---|---|---|
| Drag | Up raises, down lowers. The full range is 200 screen pixels at any zoom. | Moves the tile | Moves the tile | Moves the tile |
| Shift-drag | 10 times finer | | | |
| Click | Selects the module | Toggles, or picks the option | Selects the module | Selects the module |
| Double-click | Resets to the default | Resets to the default | | |
| Mouse wheel | Zooms the canvas | Zooms the canvas | Zooms the canvas | Zooms the canvas |

- Knob drags work in the control's 0 to 1 range, through the parameter's mapping, so a logarithmic cutoff moves by equal intervals.
- Editing a control selects its module, so the inspector follows.
- **Button** ([#75](https://github.com/brylie/LoomDSP/issues/75)). The canvas reports a press, a release, or an arrow. It does not read `mode`, and it does not move the tile. Drag the tile from its header. The editor turns the report into a value ([clock.md](clock.md#button-details)). In toggle mode a press flips the bound parameter, and a release does nothing. A double-click is two presses, not a reset. In momentary mode the parameter is 1 while the pointer or Space or Enter is held, and 0 on release, including when the pointer has left the button, when another mouse button is pressed, or when the window loses focus. Releasing a mouse button other than the left one does not end a hold the left button is still keeping. Space and Enter each keep their own hold. The gate stays high while the pointer or either key is still down. Arrow keys set 0 or 1 in toggle mode and do nothing in momentary mode. A repeated key event does nothing.
- **Keyboard:** with a module selected, Tab moves focus to its next control and Shift-Tab to the previous one. Up and Right raise the value by 1% of the range, Down and Left lower it; with Shift, 0.1%. Whole-number parameters step by 1. Space or Enter toggles or picks the next option. When a control is focused, Space and Enter act on it and do not toggle Run. Space toggles Run only when no panel control is focused ([clock.md](clock.md#controls-live-on-the-transport-module)). Escape clears the focus. A control focused with Tab has a visible ring (NFR-UX-01). The ring goes around the control itself, not its caption, so the value shown there stays clear. A click or drag also focuses the control, so the arrow keys work after it, but shows no ring, as `:focus-visible` works on the web. *Why:* after a mouse gesture the pointer already shows where the user is, and a ring around the whole cell cut through the value text ([#92](https://github.com/brylie/LoomDSP/issues/92)).
- While a knob is dragged or focused, its label shows the value with its unit.
- *Why the wheel always zooms:* scrolling over a tile to zoom must never change the sound by accident.

### Level of detail ([#68](https://github.com/brylie/LoomDSP/issues/68))

**Status:** Implemented. Code: `detail` in `loom-canvas/src/widget.rs`, `tile_text.rs`.

The canvas already hides port labels and titles below a size on screen. Panels use the same two thresholds:

| Zoom | Controls and lights | Text |
|---|---|---|
| Port labels shown | Full widgets, with labels and values | All text |
| Titles shown, port labels hidden | Shapes only: knob and value arc, toggle, button, and light states | Titles. Port labels and captions are bars. |
| Titles hidden | Not drawn | Titles and port labels are bars |

- **Text too small to read leaves a faint bar.** A rounded bar, as wide as the text and half as tall as its letters, sits where the text would be. *Why:* a tile that loses all its text looks empty from far out, but a real panel seen from across a room still shows where things are written ([#92](https://github.com/brylie/LoomDSP/issues/92)). Text that small cannot be read anyway, and drawing it would mean thousands of glyph layouts in a large patch.
- Bars are shapes in the scene's instanced buffer. They are built when the scene changes, never per frame, so they cost almost nothing at any zoom.
- Text, bars, and hover on port labels all take their places from one list (`tile_text.rs`), so a bar always sits exactly where its text would be.

### Redraw ([#68](https://github.com/brylie/LoomDSP/issues/68))

**Status:** Implemented. Code: `ControlGeometry` in `loom-canvas/src/controls.rs`, `loom-app/src/lights.rs`.

- **Control values have their own GPU buffer.** Changing a value rewrites only that buffer, never the tiles, ports, or cables. *Why:* ADR-0010's rule that GPU data for the patch is rebuilt only when the patch changes.
- Lights read the display table on a timer of 30 Hz, but only while a module with lights is on screen at a zoom where lights are drawn. A frame is drawn only when a shown brightness changes by at least 1/256.
- The timer runs on its own thread and sends the UI a message only when a light changes. *Why:* in Iced every message redraws the window, so a plain 30 Hz timer message would draw 30 frames a second even when nothing moves. The canvas reports which modules with lights are on screen, so the app watches only those.
- Nothing visible changing means no frame.

### Acceptance criteria: module panels

#67, panel declarations:

- [x] Every built-in module declares a panel. A test checks that every widget binds to something its module has, fits inside the panel, and overlaps no other widget.
- [x] The registry refuses each kind of bad panel listed above, with a message that names the widget.

#69, display values:

- [x] A module's display values reach the `Controller` after the block that wrote them.
- [x] A module added to a reused slot reads 0 until it writes, never the old module's values.
- [x] Display values allocate nothing on the audio thread (`no_alloc` test).
- [x] Output audio is bit-identical with and without display values being read.

#68, controls on tiles:

- [x] Dragging a knob on a tile changes the parameter. The inspector shows the same value, and saving the patch keeps it. (Checked by hand in [#92](https://github.com/brylie/LoomDSP/issues/92).)
- [x] Dragging a control never moves the tile. Dragging the header always does. (Checked by hand in [#92](https://github.com/brylie/LoomDSP/issues/92).)
- [x] Shift-drag is 10 times finer. Double-click resets to the default.
- [x] A logarithmic knob moves by equal ratios: halfway up a 20 Hz to 20 kHz cutoff knob is about 632 Hz.
- [x] Every control is reachable by keyboard: Tab, Shift-Tab, arrows, Space, Escape, with a visible focus ring. The ring shows for Tab focus only, and stays clear of the caption.
- [x] Changing a control value does not rebuild the GPU data for tiles, ports, or cables.
- [x] The MIDI Note gate light is lit while a key is held. The ADSR light follows its level. The tests in `loom-modules/tests/panels.rs` play a note through `subtractive.ron` and read the display values the lights draw, at block sizes 1, 77, and 512.
- [x] Zoomed far out, controls and lights are not drawn. At middle zoom, they have no text.
- [x] Text hidden by zoom is replaced by a bar at the place and width of its text: port labels and captions at middle zoom, titles and port labels far out.
- [ ] Manual, on the reference laptop: with `--generate 300 --bench`, the frame rate far out with bars matches the rate without them.
- [x] Lights add no frames while their brightness stays the same: the light watcher sends nothing until a shown level changes, and runs only while lights are on screen.
- [ ] Manual, on the reference laptop: knobs, the toggle, and lights look right at close and middle zoom, and a cut port label shows in full on hover.

The whole app still redraws twice a second for the stats bar's audio numbers. "No redraw when idle" for the whole app stays a P1 criterion below.

## Level meters and audio load (P1)

**Status:** Implemented ([#9](https://github.com/brylie/LoomDSP/issues/9)). Code: `WidgetKind::Meter`, `loom-modules/src/display.rs`, `loom-app/src/monitor.rs`, `live.rs`, `alerts.rs`. Requirements: FR-LIB-17, FR-VIS-03 (the level meter module, brought forward from P2), FR-PERF-04.

### Meter widget

- A fifth P1 widget, 3 × 6 grid units. It binds to one of its module's **outputs** by name. The registry refuses a meter bound to an output the module does not have.
- It shows the output's level from -60 dB (empty) to 0 dB (full), bottom to top: a bar for the RMS level and a thin line for the peak. Both rise at once and fall by the whole range in about a second. The bar is green below -12 dB, amber up to -3 dB, and red above. Its label is below it.
- A meter is a display: pressing or dragging it selects or moves the tile, as with a light.
- *Why levels fall over a second:* a short peak stays visible long enough to see, and the bar does not flicker.

### Level meter module

- **`loom.display.meter`**, category Display. One audio input `in` and one output `out` that passes `in` through, sample for sample. No parameters. Its panel is one meter bound to `out`.
- Mono. For stereo, use two.
- *Why a pass-through output:* probes read outputs, and the module can sit inline in a signal path or hang off an output, without changing the sound.

### How levels reach a meter

- While a meter is on screen at a zoom where it is drawn, the app keeps a `Meter` probe on its output ([telemetry](telemetry.md)). When the meter leaves the screen, the probe is removed at the next commit. *Why:* a probe costs engine work, and a meter nobody sees needs none.
- Telemetry is drained on a background thread every 33 ms, not on the UI thread. The thread keeps each probe's peak and RMS levels and sends the UI a message only when a level changes by at least 1 of 256 steps. *Why:* as for lights, every message redraws the window, so a silent patch with meters on screen must send none.
- Probes stay view state: not saved, and they never change the sound.

### Audio load and dropouts in the stats bar

- The stats bar always shows the audio load (the latest callback as a share of its time budget, and the highest so far) and the dropout count: callbacks that overran their buffer, plus errors reported by the audio device.
- Load is shown normally below 70%, in amber from 70%, and in red from 90%.
- The dropout count is red for 10 seconds after it grows.
- *Why:* a performer needs a warning before a problem is audible (FR-PERF-04), and must notice when one just happened. A count that stayed red forever would soon be ignored.

### Acceptance criteria: level meters and audio load

- [x] The registry refuses a meter bound to an output the module does not have.
- [x] The meter module passes its input through, bit for bit. It has a reference test and a block-size check.
- [x] A meter on screen gets a probe on its output. When it leaves the screen, or the canvas zooms far out, the probe is removed.
- [x] Levels map -60 dB and below to empty and 0 dB to full. They rise at once and fall by the full range in a second. The peak line is never below the RMS bar.
- [x] The telemetry thread sends nothing while the shown levels stay the same.
- [x] Load below 70% is normal, from 70% amber, from 90% red. The dropout count is red for 10 seconds after it grows.
- [x] A meter module after the VCA in `subtractive.ron` rises when a note plays and falls after it stops. A meter on the filter's output stays up with no note, because the saw runs all the time and only the VCA is gated. Tested twice: as meter readings in `loom-modules/tests/panels.rs`, and as drawn levels and colour bands in `loom-app/src/live.rs`. (This was a manual check that said "after the filter"; [#92](https://github.com/brylie/LoomDSP/issues/92) found that wording wrong.)

### Out of scope

- Stereo meters, and a one-step "Add meter" gesture on a cable (it comes with "Add scope").
- Peak hold numbers and clip indicators.
- Spectrum and scope displays.

## Labels and hover text

**Status:** Implemented, except the manual check below. Code: `loom-app/src/scene.rs` (labels), `loom-canvas/src/hover.rs` (timing and placement), `draw_hover_text` in `loom-canvas/src/widget.rs`, and the inspector in `loom-app/src/main.rs`. Builds on [port and parameter text](port-and-parameter-text.md) ([ADR-0016](../adr/0016-ports-and-parameters-carry-their-own-text.md)): every port and parameter has a `label` and a one-sentence `description`. This section says where the label and description appear.

### Goal

A user zooms in on Clock Divide. The knob says "Division", not `n`. They rest the mouse on it and read "Fires on every Nth trigger." They never leave the canvas to learn what a control does (product principle 11).

### Where each text shows

| Place | Showed before | Shows now |
|---|---|---|
| Port label on a tile | name | label, cut with "…" to fit its column as now |
| Caption of a knob, toggle, button, or radio group | parameter name | parameter label, cut with "…" to fit the widget |
| Caption of a meter | output name | output label |
| Caption of a light | display value name | display value name (display values have no label yet) |
| Inspector parameter row | name | label |
| Hover text on a port or control | full name, only when the label is cut | label and description, always |

- **Names stay out of sight, except in hover text.** The hover text shows the stable name below the description, dimmed, when it differs from the label other than in letter case. So `n` shows under "Division", but `cutoff` does not repeat under "Cutoff". *Why:* names are what patch files and `loom modules` use, so a user who edits a RON file can find the port. Everywhere else, the label is the better text.
- **Captions are cut, never wrapped.** A label that does not fit is cut with "…", and the hover text shows it in full. Widths are estimated per character, with narrow letters counting less, so "Division" fits under a knob, while "Resonance" is cut. *Why:* the grid layout of tiles does not change with text length, and estimating keeps text layout out of every frame.

### Hover text

- **When it shows.** After the pointer rests on a port or a control for 500 ms, every time. It hides as soon as the pointer leaves, a drag starts, or the canvas pans or zooms. *Why the delay:* while wiring, the pointer passes over many ports. Text that flashed up under every one would get in the way. 500 ms is what most desktop apps use.
- **During a drag it does not show.** That covers dragging a cable, a knob, or a tile. *Why:* the knob label already shows the value while it is dragged, and a cable drag needs a clear view of the target ports.
- **Keyboard focus shows it too.** When Tab moves focus to a control, its hover text shows after the same delay, and hides when focus moves on or Escape clears it. Right after a Tab press, the focused control's text wins over a port or control under a resting pointer, until the pointer moves. *Why:* the last thing the user did decides what they want to read. *Why:* keyboard users need the same help as mouse users (NFR-UX-01).
- **What it shows.** The label on the first line, in normal text colour. The description below it, in dim text, wrapped. The name last, dimmed, if it differs from the label (see above).
- **Size and place.** Fixed screen size, 12 px text, at most 260 px wide, whatever the zoom. On a canvas narrower than that, the box narrows to fit and the text wraps. It sits above the port or control, or below it when there is no room above, and always inside the canvas. *Why fixed size:* at middle zoom tile text is hidden because it is too small to read; the hover text is exactly what the user needs there, so it must stay readable.
- **A port's label hovers like the port.** Resting the pointer on a port's label, or on the bar that stands for it, shows the port's hover text. Pressing a label still selects or drags the tile, as before. *Why:* a cut label is what makes the user want the full text, so the label is where they point ([#92](https://github.com/brylie/LoomDSP/issues/92)).
- **At every zoom where the port or control can be hovered.** At middle zoom, labels are hidden, but hovering a port still shows its hover text.
- **Lights, meters, and empty panel areas have no hover text.** Meters are displays, like lights. Display values have no description yet. A tile can therefore mix labelled captions, such as "Attack", with a light that shows its display value name, such as `level`. *Why accept it:* giving display values labels is a separate change, and the mix is small.
- **It replaces the full-label overlay** from [Tile layout](#tile-layout-68): a cut label shows in full as the first line of the hover text.

### Inspector

- Each parameter row shows the label, and the value with its unit as now.
- Hovering the row's label shows the description and the name, in an Iced tooltip with the same content as the canvas hover text.

### Redraw and GPU data

- Showing or hiding hover text redraws once. While it is shown and nothing else changes, nothing redraws. The 500 ms wait is a single timer, not a frame loop.
- Labels are part of the scene. They change only when the patch changes, so they follow the existing rule: GPU data is rebuilt only when the scene's generation changes.
- Hover text is drawn as Iced text and quads in the overlay layer, like the full-label overlay today. It adds nothing to the instanced GPU buffers.

### Acceptance criteria: labels and hover text

- [x] Port labels, control captions, and meter captions show labels. A test builds the scene for Clock Divide and finds "Division", not `n`.
- [ ] Manual: inspector rows show labels, and hovering one shows its description, and its name when it differs.
- [x] Resting the pointer on a port or control for 500 ms shows its label, description, and (when it differs) name. Leaving hides it at once.
- [x] Resting on a port's label shows the port's hover text. Resting on empty tile space shows none.
- [x] No hover text shows during a cable, knob, or tile drag.
- [x] A control focused with Tab shows its hover text after the delay.
- [x] Hover text stays inside the canvas for a port at each edge of the window.
- [x] With hover text shown and nothing changing, hover text asks for no frames (`HoverTimer` tests).
- [ ] Manual: with Measure on, the frame rate with hover text shown matches the rate without it. (The stats bar still redraws twice a second, see above.)
- [ ] Manual, on the reference laptop: hover text is readable at close and middle zoom, and does not get in the way while wiring.

### Out of scope

- Translation (NFR-UX-04). Text is looked up by module ID and name when it comes.
- Labels and descriptions for display values and panel widgets, so lights keep showing names.
- Help pages (#43), and module descriptions in the module picker.
- Touch: long-press for hover text.
- A shorter delay for the next hover text once one has shown. Consider it if reading a column of ports feels slow in use.

## P1 slice

What P1 builds. The rest of this spec is P2.

### Acceptance criteria

- [ ] Module panels on tiles: see [their acceptance criteria](#acceptance-criteria-module-panels) above.
- [ ] A scope module fed by a 440 Hz oscillator shows a stable trace.
- [ ] "Add scope" on a cable places a connected scope module next to it. Undo removes it.
- [ ] Cable opacity never depends on the signal. A test checks that the cable geometry is the same with and without telemetry.
- [ ] With no displays on screen and no input, the app does not redraw. Check: the frame counter stays still for 10 seconds.
- [ ] Zoomed far out, controls and displays are not drawn.
- [ ] Reference laptop, full window: 60 fps with four scopes on screen. Record CPU and GPU in the benchmark doc.

### Out of scope for P1

- The panel editor, sub-patch panels, and front panels (P2, FR-PANEL-*).
- Step grid and graph editing on the tile. They need module data (OQ-17, P2).
- XY pad, themes, custom skins (P2).
- Restyling built-in panels.
- Docked views at the window edge. Displays are modules.

## Open points

- Can users restyle a built-in module's panel, or only their own sub-patches?
- How third-party modules add display kinds through the SDK.
- How a sub-patch tile looks before its panel has been designed. Leaning: its exposed parameters as knobs, in order.
