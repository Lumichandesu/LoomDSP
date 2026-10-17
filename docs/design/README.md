# Design Specifications

Living specifications: how a feature works in detail. What the user sees and does, rules and their reasons, acceptance criteria, what is out of scope, and open points.

The decision behind each one is recorded in an [ADR](../adr/README.md), which does not change. The specification does change, as the work teaches us.

## Spec first

We write the specification before the code, and keep it true as the code is built.

1. **Write or update the spec first.** A feature issue links to its spec, or to the section that covers it. If there is none, the first step is a spec pull request, not code.
2. **Agree on it.** A spec, or a section of one, is ready to build when its status is **Agreed**. Review is cheap at this point: changing a paragraph costs minutes, changing code costs days.
3. **Build to it.** The code does what the spec says, no more. Behaviour the spec does not mention is a question for the spec, not a guess in the code. Things under "Out of scope" are not built.
4. **Let it evolve.** Building teaches things the spec missed. When the code shows the spec is wrong or incomplete, change the spec in the same pull request, and say so in the description. The spec and the code never disagree on the main branch.
5. **Check against it.** Each acceptance criterion becomes a test, or a manual check listed in the pull request.

**Why:** a clear spec keeps people and AI agents building the same thing. Without one, a code agent fills every gap with its own guess, and the result can be well made but not what was wanted. A spec turns those guesses into questions, asked before the work starts.

**Not every change needs a spec.** Bug fixes, refactoring, tests, and small changes that do not change behaviour go straight to code. A new module follows the [module checklist](../../CONTRIBUTING.md#adding-a-module) and its help page. It needs a spec only when it brings new rules, as the arrangement modules do.

## Status

| Status | Meaning |
|---|---|
| **Draft** | Being written or discussed. Do not build from it yet. |
| **Agreed** | Ready to build. Changes still happen, but through review. |
| **Implemented** | The code matches it. Keep both in step. |

A spec can give sections their own status when parts move at different speeds.

## Writing one

Copy [template.md](template.md). Keep it to one feature. Link the ADR, requirements, use case, and issues. Give every rule its reason, so later readers can judge when it should change. When a rule rests on theory a newcomer would not know, explain the theory in an [explainer](../learn/README.md) and link it from the spec's header.

## Index

| Specification | Decision | Status |
|---|---|---|
| [Arrangement](arrangement.md) | [ADR-0012](../adr/0012-arrangement-from-phase-driven-primitives.md) | Draft, P3 |
| [Telemetry](telemetry.md) | [ADR-0001](../adr/0001-lock-free-control-audio-boundary.md) | Implemented, P1 |
| [Panels](panels.md) | [ADR-0013](../adr/0013-modules-carry-their-own-panel.md) | Module panels on tiles, level meters, and audio load Implemented (P1); labels and hover text Implemented; rest Draft |
| [Modulation](modulation.md) | [ADR-0014](../adr/0014-modulation-without-cables.md) | Engine and parameter mapping Agreed (P1); editing and display Draft |
| [Control ranges](control-ranges.md) | None (extends parameter mapping) | Draft, P1 |
| [Module test harness](module-test-harness.md) | [ADR-0002](../adr/0002-graph-execution-and-feedback.md) | Implemented, P1 |
| [Clock and transport](clock.md) | [ADR-0015](../adr/0015-one-transport-per-patch.md) | Engine sections, Clock Divide, and Phase Trigger Implemented (P1); rest Draft, P1 and P2 |
| [Port and parameter text](port-and-parameter-text.md) | [ADR-0016](../adr/0016-ports-and-parameters-carry-their-own-text.md) | Implemented, P1 |
