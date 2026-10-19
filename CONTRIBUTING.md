# Contributing to LoomDSP

Thank you for helping. This guide explains how to build, test, and send changes.

## Before you start

- Look for an issue first. If none exists, open one and describe what you want to change.
- Issues reference requirement IDs from [docs/product/requirements.md](docs/product/requirements.md), for example `FR-MIDI-02`. Mention the ID in your pull request too. This links code to the reason it exists.
- New features start with a [specification](docs/design/README.md#spec-first): what the user sees, the rules, acceptance criteria, and what is out of scope. Agree on the spec, then build to it, and update it in the same pull request when the code teaches you something.
- Big design changes need an [ADR](docs/adr/README.md) first. The ADR records the decision only. Details that will change (ports, parameters, examples) go in a [design specification](docs/design/README.md), which you update with the code. See [kinds of documents](docs/README.md#kinds-of-documents).
- AI-assisted contributions are welcome. Read the [AI contribution policy](AI_POLICY.md).

Good starting points: issues labelled [`good first issue`](https://github.com/brylie/LoomDSP/labels/good%20first%20issue) or [`help wanted`](https://github.com/brylie/LoomDSP/labels/help%20wanted). New modules are often a good first contribution: they are small, self-contained, and easy to test. Pick an issue without `needs spec agreed`: the others still have open questions ([ready to build](docs/design/README.md#ready-to-build)). The current phase's work is in the [P1 milestone](https://github.com/brylie/LoomDSP/milestone/1), roughly in priority order.

To understand the code first, read [docs/dev/architecture.md](docs/dev/architecture.md).

### Labels

| Kind | Labels |
|---|---|
| Type | `feature`, `bug`, `docs`, `research`, `adr`, `chore` |
| Area | `area:engine`, `area:modules`, `area:ui`, `area:midi`, `area:crates`, `area:infra`, `area:export` |
| Priority | `must`, `should`, `could` (same as the requirement priority) |
| Newcomers | `good first issue`, `help wanted`, `hacktoberfest` |
| Readiness | `needs spec agreed`: not ready to build yet (see [Ready to build](docs/design/README.md#ready-to-build)) |

Comment on an issue before you start, so two people do not do the same work.

## Build and test

You need Rust 1.88 or newer. On Linux, install the ALSA development package (`libasound2-dev`).

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo bench -p loom-modules          # optional, takes a few minutes
```

Run the desktop app and the command-line tool:

```sh
cargo run --release -p loom-app -- examples/patches/fm-feedback.ron
cargo run --release -p loom-cli -- play examples/patches/subtractive.ron
```

### Git hooks (recommended)

The repository uses [prek](https://prek.j178.dev) to run quick checks before each commit, so you hear about problems while the change is fresh. Tools come from [mise](https://mise.jdx.dev):

```sh
mise install    # Rust, prek, typos, cargo-deny (from mise.toml)
prek install    # installs the pre-commit and pre-push hooks
```

| When | Checks |
|---|---|
| Every commit (a few seconds) | Whitespace, line endings, TOML/YAML syntax, large files, private keys, `cargo fmt`, `cargo clippy`, `typos`, and no direct commits to `main` |
| Before push | `cargo test`, `cargo deny check` |

`cargo fmt` and the whitespace hooks fix files for you. When they do, the commit stops: review the changes, stage them, and commit again. Run everything by hand with `prek run --all-files`. The configuration is in `prek.toml`.

### What CI checks

Every pull request runs these jobs. All must pass before merging (the "CI success" check).

| Job | What it checks | Run it locally |
|---|---|---|
| Format | `rustfmt` style | `cargo fmt --all --check` |
| Clippy | Lint groups set in the workspace `Cargo.toml`: correctness, suspicious, complexity, perf, style, cargo | `cargo clippy --workspace --all-targets -- -D warnings` |
| Test | All tests on Linux, macOS (Apple Silicon), and Windows, plus benchmarks compile | `cargo test --workspace` |
| Coverage | Line coverage, shown in the job summary | `cargo llvm-cov --workspace` (install [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)) |
| Docs | API docs build without warnings | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` |
| MSRV | Still builds with Rust 1.88 | `cargo +1.88 check --workspace` |
| WebAssembly | Engine crates build for `wasm32-unknown-unknown` | `cargo build --target wasm32-unknown-unknown -p loom-core -p loom-modules -p loom-patch -p loom-rt` |
| Licences | Only permissive licences, no known vulnerabilities | `cargo deny check` (install [cargo-deny](https://github.com/EmbarkStudios/cargo-deny)) |
| Spelling | Typos in code and docs | `typos` (install [typos-cli](https://github.com/crate-ci/typos)) |

Why warnings fail CI but not your local build: a warning while you are still editing is noise, but a warning on `main` is a problem someone else inherits.

Dependabot opens weekly pull requests for dependency updates.

## Rules for audio code

The audio thread has a hard deadline. At 48 kHz with 128-frame buffers, each buffer must finish in 2.67 ms, every time. So code that runs inside `Module::process` or `Engine::process` must not:

- allocate or free memory (no `Vec::push` past capacity, no `Box::new`, no `String`),
- take a lock or wait on another thread,
- make system calls (no printing, logging, or file access).

Why: each of these can take an unpredictable amount of time, and one late buffer is an audible dropout. The test `crates/loom-modules/tests/no_alloc.rs` aborts if the audio path allocates.

Modules must also give the same output whether they process 1 frame or 128 frames at a time. Inside feedback loops the engine calls them one frame at a time (ADR-0002). The test `output_does_not_depend_on_host_buffer_size` checks this for whole patches, and the module test harness checks it for each module.

## Adding a module

Start from the module's entry in the [module catalogue](docs/product/module-catalogue.md), and name its catalogue ID (such as `MT-FILTER-02`) in the issue and pull request. If the module has no entry, propose one first. Design the module in a clean room ([ADR-0018](docs/adr/0018-clean-room-module-development.md)): work from its purpose and from textbooks, papers, and standards, and cite them. Do not read other audio projects' source code, except the source code of a dependency accepted under ADR-0006 when using that dependency. Do not copy their help text, control names, or panel layouts. Why: LoomDSP's modules must be its own, so every patch can be shared under a permissive licence without doubt.

1. Add a `static` `ModuleDescriptor` and a struct implementing `Module` in `crates/loom-modules/src/`.
2. Add it to `ALL` in `crates/loom-modules/src/lib.rs`.
3. Give it a stable, namespaced ID (`loom.<category>.<name>`) and version 1. Never rename a released ID, port, or parameter. Add a new version instead. During the early alpha nothing is released, so renames are allowed when they improve the design ([AGENTS.md](AGENTS.md#project-stage-early-alpha)).
4. Write each port and parameter as a struct literal, with a short `label` and a one-sentence `description`. These are text for people, not IDs, so you can reword them later. The crate does not compile if one is empty ([port and parameter text](docs/design/port-and-parameter-text.md)).

   ```rust
   inputs: &[PortSpec {
       name: "reset",
       label: "Reset",
       signal: SignalType::Gate,
       description: "Starts the count again.",
       default: PortDefault::Value(0.0),
       polarity: Polarity::Unipolar,
       channels: Channels::Downmix,
   }],
   params: &[
       ParamSpec {
           name: "n",
           label: "Division",
           description: "Fires on every Nth trigger.",
           min: 1.0,
           max: 64.0,
           default: 2.0,
           unit: "",
           options: ParamOptions::DEFAULT,
       }
       .integer(),
   ],
   ```

   Every port names its `polarity` ([port ranges](docs/design/port-ranges.md)) and what it does with stereo, its `channels` ([stereo cables](docs/design/stereo.md#declaring-stereo-behaviour)). Most modules write mono code and need no more than the usual choice: audio inputs are `Channels::PerChannel`, so the engine runs the module once per side when stereo arrives; other inputs are `Channels::Downmix`; outputs are `Channels::PerChannel`. Use `Channels::Stereo` only for a module that must see both sides at once, such as a panner.
   Mark whole-number parameters (channels, counts, seeds) with `.integer()` on their `ParamSpec`, so they round and the inspector snaps to whole values.
   Mark frequencies and times with `.logarithmic()`, so equal steps of the control give equal ratios. Controls and modulation both use this mapping ([parameter mapping](docs/design/modulation.md#parameter-mapping)).
5. Give it a panel: a `Panel` in its descriptor with a knob for each parameter a performer would turn, and lights for display values. The registry refuses a panel that does not fit (see [module panels on tiles](docs/design/panels.md#module-panels-on-tiles-p1)).
6. Add a reference test to `crates/loom-modules/tests/reference.rs` with the [module test harness](docs/design/module-test-harness.md) (`loom-test`). It checks that block size does not change the output and compares with a short reference render. Create the file with `LOOM_UPDATE_REFERENCES=1 cargo test -p loom-modules --test reference`, listen to it, and commit it. Reviewers can listen to it too. For smaller checks, write unit tests with `loom_core::testing::run`.
7. Run the full test suite, then try the module in the app: `cargo run --release -p loom-app`, pick it in the toolbar, and click **Add**.

## Working on the canvas and app

The canvas (`crates/loom-canvas`) must stay fast on high-resolution displays. Its cost should follow the number of things drawn, not the number of pixels. So:

- Do not rebuild GPU data every frame. It is rebuilt only when the scene's generation changes; pan and zoom change one uniform.
- Do not add shaders that run over the whole window for each shape.
- In `canvas.wgsl`, avoid names that are reserved in Metal or HLSL (`half`, `in`, `out`, `sample`, and others). The `shader_names` test checks the list.

To check performance, run `cargo run --release -p loom-app -- --generate 300 --bench` and read the bottom bar. Results and the method are in [docs/benchmarks/p1-canvas.md](docs/benchmarks/p1-canvas.md). For UI changes, add a screenshot to the pull request. `loom-app <patch> --screenshot out.png` saves one without a window; `--zoom`, `--center`, `--scale`, and `--focus` set what it shows (see `loom-app --help`). Other options show a state that would need a person at the screen: `--pointer` puts the pointer on a point or a named control or port, `--hover-text` shows its hover text at once, `--pin` pins an output's modulation badge, `--wire` shows a cable being dragged to the pointer, and `--depth` shows a knob's range arc being dragged.

## Dependencies

Only permissively licensed dependencies are allowed (MIT, Apache-2.0, BSD, ISC, Zlib, and similar). No GPL, LGPL, AGPL, or other copyleft licences. See [ADR-0006](docs/adr/0006-dependency-and-licence-policy.md). Add a short dependency review to the pull request for any new dependency.

## Pull requests

- Keep them focused: one change per pull request.
- Include tests.
- For a feature, link its specification and say which acceptance criteria the change meets. If the change differs from the spec, update the spec in the same pull request.
- Explain what changed and why, in plain language.
- CI must pass.

## Writing style for docs

Short, direct sentences. Plain words. Explain the reason behind a rule, not only the rule. Many contributors read English as a second language.

## Licence

By contributing, you agree that your contribution is licensed under Apache-2.0.
