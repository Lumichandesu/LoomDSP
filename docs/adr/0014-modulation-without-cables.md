# ADR-0014: Modulation without cables

- **Status:** Accepted (2026-10-17)
- **Date:** 2026-10-15
- **Related requirements:** FR-MOD-01 to FR-MOD-08, FR-LIB-12, FR-ENCAP-03
- **Related decisions:** ADR-0002 (graph execution), ADR-0007 (signal model), ADR-0011 (everything is a module), ADR-0013 (modules carry their own panel)
- **Specification:** [Modulation](../design/modulation.md)

## Context

FR-MOD-01 asks for every parameter to have a base value plus any number of modulation sources, each with its own depth. In a cable-only modular, each of those is a cable, often with an attenuator module in between. A synth with a few LFOs and envelopes on a dozen parameters becomes a web of cables that hides the audio path.

Bitwig shows another way. You drag from a modulator onto a knob, and the knob shows the depth as an arc. Plugins such as Serum and Vital add a modulation matrix: one table that lists every source, target, and depth.

LoomDSP still has to stay one system. ADR-0011 says modulators are modules, and ADR-0007 says any output can drive any input.

## Options considered

1. **Cables only.** Modulate a parameter by patching a cable into an input, with attenuators for depth. Simple and explicit. But busy patches become unreadable, and every module needs an input for every parameter.
2. **Modulators built into each module,** as in many soft synths (an LFO slot inside the filter). Quick to use, but it breaks "everything is a module". The LFO cannot also drive a sequencer or be shared.
3. **Wireless send and receive modules,** as in Max. Removes long cables, but it still needs a module pair per route and gives no depth control or overview.
4. **Modulation assignments.** Any output can be assigned to any parameter, with a depth, without a cable. The assignment is part of the graph: it is saved, undone, compiled, and exported like a cable. It is shown on the control (a depth arc) and listed in a modulation matrix.

## Decision (proposed)

Option 4. **A parameter can be modulated by any output through a modulation assignment, without a cable.**

- An assignment links one output to one parameter, with a depth (and polarity). Any number of assignments can target one parameter.
- Sources are ordinary outputs of ordinary modules. Nothing is a "modulator" by type. An LFO, an envelope, a macro knob, a sequencer, or an audio oscillator can all be sources.
- Assignments are edges in the graph. The compiler orders and schedules them like cables, including feedback (ADR-0002).
- The engine combines the base value and its assignments per sample, in the parameter's knob range, and clamps the result.
- Assignments are made by dragging onto a control, and shown as a depth arc on it (ADR-0013). The **modulation matrix** lists every assignment in a patch or sub-patch as a table you can edit.
- Cables stay for signal flow. Modulation can still use a cable where a module has an input for it.

## Consequences

- Patches stay readable. The cables show the signal path; the controls show what is modulated.
- Hidden routing is the risk. Hover, the source's badge, and the matrix must make every assignment easy to find. This is a hard requirement in the specification, not polish.
- The module API changes: a modulated parameter becomes a per-sample buffer, not one value per block. Every module is updated once. Unmodulated parameters stay cheap.
- Assignments do not cross sub-patch boundaries. To modulate something inside a sub-patch, expose the parameter (FR-ENCAP-03). This keeps sub-patches self-contained.
- The patch format gains a list of assignments (ADR-0008). Older files without it still load, so no migration step is needed.
- Exported plugins carry their modulation unchanged, because it is part of the patch.
