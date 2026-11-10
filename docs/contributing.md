# Contributing

> **Last updated:** 2026-11-10 · **Status:** in force; the checklists below match the code of 0.1.0

This page holds the rules for the code and the checklists. How to report a problem and how to send a change (an issue first, then a pull request), the clean-room rule and the licence of contributions are in [`CONTRIBUTING.md`](../CONTRIBUTING.md) at the root, which is the source for that process; the rules behind everything are in [`AGENTS.md`](../AGENTS.md).

- **Language:** Rust only. No JavaScript or TypeScript. On the web, `wasm-bindgen` generates a small loader; never hand-write JS.
- **Repository language:** English for everything committed (code, comments, docs, commit messages, error messages), whatever language you work in.
- **Licence:** MIT (`LICENSE`).
- **Own kernel:** no dependency on an existing geometry kernel, mesher or triangulator, and no C/C++ in the modelling path. A new general-purpose dependency in `math`, `geom`, `topo`, `kernel` or `tess` needs a recorded decision.
- **Clean-room:** do not copy code, shaders, icons or other assets from proprietary software, and do not copy or transliterate code from copyleft kernels (OpenCascade, CGAL, …). Work from public specs, textbooks, papers and observed behaviour; cite the source (title, section) in a comment where an algorithm comes from one. Only use assets with permissive licences, keep their licence next to the asset, and list every asset in `ATTRIBUTION.md`. Never commit font files or corpus files.
- **Never crash:** non-test code must not panic: no `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`/`unimplemented!` and no `unsafe`. Errors go through `Result` and `?`; input-derived indices, sizes and floating-point values are checked; every crash fix ships with a regression test. See *Never crash* in [`AGENTS.md`](../AGENTS.md).
- **Commands, not handlers:** new behaviour is an engine command with tests; the UI, CLI and agents call it (checklist below).
- **History first:** a command changes the document (variables, sketches, features); bodies follow by regeneration. No command edits a body directly.
- **Layering:** `cargo xtask layers` must pass. Register new crates in `xtask/src/layers.rs`. It also refuses a clock of `std` (`Instant::now`, `SystemTime::now`, which panic in the browser) in a crate the browser build compiles, unless it is in a native-only module, a test module, a file that takes `Instant` from `web_time`, or says why the browser never gets there with a `// native clock: …` comment.
- **Tests:** required for every change, written to the oracles in [`testing.md`](testing.md). Geometry is tested with analytic and freeform inputs, at several scales, and with degenerate inputs.
- **Determinism:** no result may depend on thread count, hash order, time or unseeded randomness.
- **Style:** `cargo fmt`, and `cargo clippy -- -D warnings`. Match surrounding code. Comments explain *why*, and name the tolerance and the reference for numeric code.
- **UI:** use `theme::Tokens`. Verify visually (offscreen snapshot or the control channel) and look at the picture.
- **Commits:** small, focused, with a clear subject line. An agent commits or pushes only when the person it works for asks. A change from outside the workshop starts with an issue and comes as a pull request ([`CONTRIBUTING.md`](../CONTRIBUTING.md)).

## Adding a command

1. **Pick the id.** `<family>.<verbNoun>` in the style of the existing ones ([`design/engine.md`](design/engine.md)), stable once released. No menu path is needed: there are (almost) no menus, and whether 3DCraft ever gets an interactive modelling interface is open ([`decisions.md`](decisions.md) › *Open decisions*).
2. **Algorithm** goes in the lowest crate that fits (`geom`, `topo`, `solve`, `kernel`, `tess`, `expr`), as a pure function with unit tests. It takes its tolerance as an argument, handles every geometry kind or returns `Unsupported`, and is deterministic.
3. **Command** goes in an engine module (`crates/engine/src/<area>_cmds.rs`, a new module for a new area) with its own `pub(crate) static SPECS: &[CommandSpec]`, each made with the `spec!` macro of `commands.rs`; a new module's `SPECS` is added to the list in `Session::commands` (`crates/engine/src/lib.rs`). Fill in:
   - `id`, `label`,
   - a params doc string such as `{"sketch": id or name, "distance": expr, "symmetric": boolean = false}` (this is what agents read through `craft3d-cli commands` and the MCP tool `command_list`, and what `panic_hunt` and the guide's test read the parameter names from),
   - a summary that says what the command does, what it returns and what it refuses, written for an agent to act on,
   - `doc: true` when it needs an open document (the only "enabled" condition there is; `command_list` then says why it is disabled),
   - `run`, which validates params (`Params::new` names the keys it takes, so an unknown key is refused), records exactly **one** history step and returns a JSON result an agent can verify (ids and names of what was created, feature status, `failed` for features the change broke).
4. **Lengths and angles are expressions.** A numeric parameter that belongs to the design accepts an expression string (`"width / 2"`, `"25 mm"`); the command stores the source text.
5. **Tests** in the module: behaviour (checked through `document.inspect` / mass properties, not internals), undo/redo, disabled states, bad params (empty, out of range, wrong type → `Err`), a save → load → regenerate round trip.
6. **Docs:** a command a user or an agent will use gets its place in the [user guide](guide/README.md) (its chapter, and the command list of [`guide/reference.md`](guide/reference.md)); `guide_examples` then checks the id and every parameter of the JSON steps the guide writes ([`testing.md`](testing.md) › *The user guide and its examples*). A `ui.*` method or field goes into [`control-protocol.md`](control-protocol.md).
7. Run the quick check per commit and the full gate before `main` moves ([`AGENTS.md`](../AGENTS.md) §5), including `panic_hunt` when commands changed; it picks a new command up from its params string. In the workshop heavy runs go to the Strix, the workshop's build server (§6); from outside, run them on your machine and say in the pull request what you ran ([`CONTRIBUTING.md`](../CONTRIBUTING.md)).

## Adding a feature type

A feature type is data in the document plus an evaluation in the kernel.

1. **Data:** add the variant and its parameter struct to `FeatureKind` in `crates/doc`. Parameters are expressions and references (`Ref`), never resolved geometry.
2. **Format:** the build now fails in `crates/format` on purpose. Add the fields to `manifest.rs` and `convert.rs` with `#[serde(default)]`; bump `FORMAT_VERSION` with a migration when the meaning of existing data changes, and keep the frozen fixture of every released format version opening and regenerating to what it made ([`file-formats.md`](file-formats.md) › *The promise*).
3. **Kernel:** implement the operation in `crates/kernel`, returning the body **and the origin map** for persistent naming. Tests per [`testing.md`](testing.md): mass properties, validity, identities, failure cases.
4. **Regeneration:** evaluate the variant in `crates/regen` (resolve references, evaluate expressions, call the kernel, map errors to a feature status). Add its cache key. Add scenarios to the topological naming suite if the feature creates or consumes named entities.
5. **Commands:** create/edit commands per the checklist above.
6. **Export:** check that STEP and mesh export handle any new surface or curve kind the feature can produce.
7. **Docs:** add the feature to its detail document under `design/` (its Method, and a dated entry under Findings and history) and update `roadmap.md`. In the roadmap a table cell holds at most one sentence; what was done goes as one line with its date under **Done** in the milestone's section, what is left as a checkbox under **Open**; how it was built (algorithms, thresholds, measurements, internal paths) goes in its detail document under `design/`, and the roadmap links to it. **The map (docs/architecture.md) changes only when the structure changes; a finding or fix goes into its detail document under docs/design/.**

## Adding a geometry kind

1. Add the variant to `Curve3` / `Curve2` / `Surface` in `crates/geom` with evaluation, derivatives, range, bounding box, projection and reversal, each tested against closed forms or by round trip.
2. Extend the intersection dispatch: every pair with the new kind either has an implementation with tests or returns `Unsupported` and appears in the coverage table test.
3. Tessellation (`tess`), STEP mapping (`io`), mass-property integration (`topo`) and the CPU renderer's needs (normals) each get the new kind in the same change, or an explicit `Unsupported` with a test.
