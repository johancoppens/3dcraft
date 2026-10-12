# Contributing

> **Last updated:** 2026-10-12 · **Status:** pre-M0

- **Language:** Rust only. No JavaScript or TypeScript. On the web, `wasm-bindgen` generates a small loader; never hand-write JS.
- **Repository language:** English for everything committed (code, comments, docs, commit messages, error messages), whatever language the conversation with the developer is in.
- **Licence:** MIT (`LICENSE`).
- **Own kernel:** no dependency on an existing geometry kernel, mesher or triangulator, and no C/C++ in the modelling path. A new general-purpose dependency in `math`, `geom`, `topo`, `kernel` or `tess` needs a recorded decision.
- **Clean-room:** do not copy code, shaders, icons or other assets from proprietary software, and do not copy or transliterate code from copyleft kernels (OpenCascade, CGAL, …). Work from public specs, textbooks, papers and observed behaviour; cite the source (title, section) in a comment where an algorithm comes from one. Only use assets with permissive licences, keep their licence next to the asset, and list every asset in `ATTRIBUTION.md`. Never commit font files or corpus files.
- **Never crash:** non-test code must not panic: no `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`/`unimplemented!` and no `unsafe`. Errors go through `Result` and `?`; input-derived indices, sizes and floating-point values are checked; every crash fix ships with a regression test. See *Never crash* in [`AGENTS.md`](../AGENTS.md).
- **Commands, not handlers:** new behaviour is an engine command with tests; the UI, CLI and agents call it (checklist below).
- **History first:** a command changes the document (variables, sketches, features); bodies follow by regeneration. No command edits a body directly.
- **Layering:** `cargo xtask layers` must pass. Register new crates in `xtask/src/layers.rs`.
- **Tests:** required for every change, written to the oracles in [`testing.md`](testing.md). Geometry is tested with analytic and freeform inputs, at several scales, and with degenerate inputs.
- **Determinism:** no result may depend on thread count, hash order, time or unseeded randomness.
- **Style:** `cargo fmt`, and `cargo clippy -- -D warnings`. Match surrounding code. Comments explain *why*, and name the tolerance and the reference for numeric code.
- **UI:** use `theme::Tokens`. Verify visually (offscreen snapshot or the control channel) and look at the picture.
- **Commits:** small, focused, with a clear subject line. Commit or push only when the developer asks.

## Adding a command

1. **Pick the id.** `<family>.<verbNoun>` in the style of the existing ones ([`architecture.md`](architecture.md) §8.2), stable once released. No menu path is needed in phase 1.
2. **Algorithm** goes in the lowest crate that fits (`geom`, `topo`, `solve`, `kernel`, `tess`, `expr`), as a pure function with unit tests. It takes its tolerance as an argument, handles every geometry kind or returns `Unsupported`, and is deterministic.
3. **Command** goes in an engine module (`crates/engine/src/<area>_cmds.rs`) exposing `specs()`, registered with `v.extend(...)` in `commands.rs`. Fill in:
   - `id`, `label`,
   - a params doc string such as `{"sketch":name,"distance":expr="10 mm","direction":"normal|reverse|symmetric"="normal"}` (this is what agents read through `commands` / `command_list`),
   - an `enabled` predicate with a reason when disabled,
   - `run`, which validates params, records exactly **one** history step and returns a JSON result an agent can verify (ids and names of what was created, feature status).
4. **Lengths and angles are expressions.** A numeric parameter that belongs to the design accepts an expression string (`"width / 2"`, `"25 mm"`); the command stores the source text.
5. **Tests** in the module: behaviour (checked through `document.inspect` / mass properties, not internals), undo/redo, disabled states, bad params (empty, out of range, wrong type → `Err`), a save → load → regenerate round trip.
6. Run the gate in [`AGENTS.md`](../AGENTS.md) §5, including `panic_hunt`.

## Adding a feature type

A feature type is data in the document plus an evaluation in the kernel.

1. **Data:** add the variant and its parameter struct to `FeatureKind` in `crates/doc`. Parameters are expressions and references (`Ref`), never resolved geometry.
2. **Format:** the build now fails in `crates/format` on purpose. Add the fields to `manifest.rs` and `convert.rs` with `#[serde(default)]`; add a fixture of the previous format version to the migration tests.
3. **Kernel:** implement the operation in `crates/kernel`, returning the body **and the origin map** for persistent naming. Tests per [`testing.md`](testing.md): mass properties, validity, identities, failure cases.
4. **Regeneration:** evaluate the variant in `crates/regen` (resolve references, evaluate expressions, call the kernel, map errors to a feature status). Add its cache key. Add scenarios to the topological naming suite if the feature creates or consumes named entities.
5. **Commands:** create/edit commands per the checklist above.
6. **Export:** check that STEP and mesh export handle any new surface or curve kind the feature can produce.
7. **Docs:** add the feature to the tables in `architecture.md` and update `roadmap.md`.

## Adding a geometry kind

1. Add the variant to `Curve3` / `Curve2` / `Surface` in `crates/geom` with evaluation, derivatives, range, bounding box, projection and reversal, each tested against closed forms or by round trip.
2. Extend the intersection dispatch: every pair with the new kind either has an implementation with tests or returns `Unsupported` and appears in the coverage table test.
3. Tessellation (`tess`), STEP mapping (`io`), mass-property integration (`topo`) and the CPU renderer's needs (normals) each get the new kind in the same change, or an explicit `Unsupported` with a test.
