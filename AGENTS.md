# AGENTS.md: guide for AI agents and contributors

3DCraft is an open-source, native, parametric 3D CAD modeller built for AI: every action is a command, so the same engine is driven from the UI, the CLI, a JSON control channel or an MCP server. It is written in **Rust only** (no JavaScript or TypeScript), with its **own CAD kernel, sketch constraint solver and tessellator**: no OpenCascade, Parasolid, ACIS, CGAL or other C/C++ geometry kernel, and no third-party constraint solver (SolveSpace, PlaneGCS, …), in any form. **No Tauri, Electron or webview shells:** the desktop app is native egui/eframe on wgpu, and a later web build is the same Rust compiled to WebAssembly. Never add Tauri (or any webview/JS UI framework) as a dependency, build step or packaging target.

3DCraft follows the principles of the storytold `*Craft` apps (PhotoCraft, VectorCraft, …). The product name is always written **3DCraft** in user-facing text: UI, window titles, docs prose. Machine names are lowercase and use the prefix **`craft3d`**, because a Rust crate name cannot start with a digit: crates (`craft3d-*`), binaries (`craft3d`, `craft3d-cli`), environment variables (`CRAFT3D_*`). The native file extension is `.3dcraft`.

**Language.** The developer prompts in Dutch. Answer the developer in Dutch, but write everything that lands in the repository in English: code, comments, identifiers, commit messages, docs, the dev log, test names and error messages.

**Status (2026-10-14): M0–M3, M5, M6 and M8 done; M4 nearly done; M7 (booleans) and M8.5 in progress; M9 started (blends, face operations, sweeps, helix and thread); R1, V1, M10 and later not started.** One path works end to end: a command edits the document (variables, sketches solved by our own constraint solver, datums and an ordered feature history), regeneration rebuilds only what an edit touched into checked B-rep bodies whose faces, edges and vertices keep persistent names, and the tessellator meshes them for the CPU or wgpu renderer or for export (STL, 3MF, OBJ, STEP; DXF and SVG of flat outlines); documents save as `.3dcraft` and are autosaved for crash recovery. 72 commands are reachable through `craft3d-cli` (`run`, `info`, `render`, `convert`, `replay`, `commands`, `serve`, `mcp`), and the desktop app `craft3d` has a control server, so an agent drives the window the developer is watching, also over MCP (`craft3d-cli mcp --bridge`). **The boolean kernel is the developer's first priority** ("what makes 3DCraft pro"): what it refuses is work owed, not an end state (the boolean checklist in `docs/roadmap.md`). What is done and what is open, milestone by milestone: `docs/roadmap.md`, starting at **Current focus**; how it is built: `docs/architecture.md` (the map) and the detail documents under `docs/design/`. Everything else in this file and in `docs/` is the target design. Commands and paths marked *(planned)* do not exist until the milestone that creates them (`docs/roadmap.md`); when you build one, remove its marker here and in the docs. Licence: MIT. Remote: https://github.com/johancoppens/3dcraft (use the `gh` CLI).

Read this file first, then `docs/`.

## 1. Orientation (5 minutes)

| Read | Why |
|---|---|
| `BRAINSTORM.md` | The developer's original intent, in a dozen lines |
| `docs/README.md` | Index of the docs |
| `docs/architecture.md` | The map: invariants, crates and layers, data, pipelines, the kernel's methods, structural debt |
| `docs/design/` | One detail document per part (geometry, booleans, blends, sweeps, names, regeneration, tessellator, rendering, engine, …): its Method, then its dated findings |
| `docs/development.md` | Build, test, run, drive the app programmatically, let the developer watch along |
| `docs/testing.md` | The AI test loop and the oracles that make a CAD kernel checkable without a human |
| `docs/contributing.md` | Rules: clean-room, tests, layering, style, commits; the "add a command" checklist |
| `docs/control-protocol.md` | JSON control channel and MCP: how agents drive and screenshot the running app |
| `docs/file-formats.md` | The native `.3dcraft` format, STEP export, STL/3MF/OBJ export, and the specs they come from |
| `docs/roadmap.md` | Milestones, **current focus**, definition of done per milestone |
| `docs/decisions.md` | Decisions taken, and the open ones only the developer can settle |
| `log/devlog.md` *(local, gitignored)* | What the previous agent landed and what is still open |

## 2. Workspace map

Every crate below exists. `apps/craft3d-web` is *(planned)*.

```text
crates/
  math geom                  L0 foundation (f64 vectors, transforms, tolerances; curves and surfaces incl. NURBS)
  step meshio zipstore       L0 standalone format crates (publishable): STEP Part 21, STL/3MF/OBJ, stored ZIP
  drawing                    L0 standalone format crate: DXF and SVG of flat outlines, with its own 2D model
  topo expr                  L1 B-rep topology (pure data + validity checks); units, variables, formulas
  doc                        L2 document model (variables, sketches, constraints, feature history: pure data)
  ops solve kernel tess      L3 history/undo, sketch constraint solver, modelling operations, tessellator
  regen render gpu format    L4 history -> B-rep regeneration, CPU reference renderer (the oracle), wgpu renderer, .3dcraft
  io                         L5 document/bodies <-> STEP and mesh formats
  engine                     L6 Session + command registry (every action is a command)
  ui-egui automation         L7 egui shell (thin: all actions go through the engine); MCP server
  testkit                    test helpers (analytic solids, mesh checks, image diff)
apps/
  craft3d                    desktop app (eframe/wgpu): viewer in phase 1, TCP control server
  craft3d-cli                headless CLI (convert/info/run/commands/serve/mcp)
  craft3d-fuzz               random multi-step designs, one seed per process (cargo xtask fuzz)
  craft3d-web                the same app in the browser (later milestone)
xtask/                       cargo xtask layers | wasm | ci | stats | health | freecad | fuzz
```

**Layering is enforced** by `cargo xtask layers`. A crate may depend only on lower layers (the few same-layer exceptions, such as `geom` → `math`, are listed in `xtask/src/layers.rs`). `testkit` may only be a dev-dependency. `step`, `meshio` and `drawing` depend on nothing in the workspace. Nothing below `ui-egui` may use egui, eframe, winit or rfd. A new crate must be registered in `xtask/src/layers.rs`.

## 3. Golden rules

### Never crash (outranks feature work)

People trust 3DCraft with their designs, and a crash loses them. A malformed file, a bad command or MCP param, a degenerate sketch, an over-constrained system, a boolean between touching faces or a full disk must produce an error the user or agent can act on, never a panic. Don't ship a feature by adding a panic path; fix a crash before building on top of it.

- **Non-test code never panics.** No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or `unimplemented!`. Return the crate's error type and propagate with `?`; use `ok_or(..)?`, `let .. else { return Err(..) }`, `if let`, or `unwrap_or*` where a fallback is truly correct (never one that silently corrupts a document or a body). Unfinished features return an "unsupported" error. The only exception is a provably infallible literal: `#[allow(clippy::expect_used)]` plus `.expect("why it can't fail")`.
- **No `unsafe`.** The workspace sets `unsafe_code = "forbid"`. There is no exception today; adding one is a developer decision recorded in `docs/decisions.md`.
- **Input-derived numbers are hostile.** Use `get()` rather than `[i]`/`[a..b]` for indices from files, params, selections or arithmetic on them; use `checked_*`/`saturating_*` for lengths, offsets and counts; cap allocations sized by input.
- **Floating point is hostile too.** Geometry code meets NaN, infinity, zero-length vectors, zero-area faces, tangent and coincident configurations as a matter of course. Normalising, dividing, taking `sqrt`/`acos` and casting to an integer are all guarded. A kernel operation that cannot produce a valid result returns `Err` with the reason (which entities, which tolerance); it never returns an invalid body.
- **Bound recursion and iteration.** Depth limits or seen-sets for trees and graphs (feature dependencies can be cyclic in a bad file); iteration caps on every numeric solver, with "did not converge" as a normal error.
- **Don't cascade.** Handle lock poisoning (`lock().unwrap_or_else(PoisonError::into_inner)`) and treat thread joins as `Result`s.
- **Last-resort guard.** The app shell and the automation servers catch an escaped panic around command dispatch and file import/export, report it as an error and keep the document. It's a safety net, not a licence to panic. Keep `panic = "unwind"`.
- **Prove it.** Every crash fix comes with a small synthetic regression test that panicked before the fix.
- **Enforced by clippy.** The workspace denies `unwrap_used`, `expect_used`, `panic`, `unimplemented`, `todo`, `unreachable` and `indexing_slicing` (`[workspace.lints.clippy]` in the root `Cargo.toml`); `clippy.toml` allows them in tests only. Every crate opts in with `[lints] workspace = true`; `cargo xtask layers` fails for a crate that does not.

1. **Everything is a command.** New user-visible behaviour = a command in the engine (a `CommandSpec` in the `SPECS` of `crates/engine/src/commands.rs` or of an `<area>_cmds.rs` module such as `sketch_cmds.rs`; a new module's `SPECS` is added to the list in `Session::commands`) with id, label, params doc, summary and `run`, plus tests. The UI, CLI, control channel and MCP all dispatch commands by id. In phase 1 there are (almost) no menus: a command needs no menu path to exist. Only pure view state (camera, panels) belongs to the shell.
2. **The design history is the source of truth.** A document is variables, sketches, constraints, formulas and an ordered list of features. Bodies (B-rep) and meshes are *derived* from it by regeneration and may be thrown away at any time. Never store a result that cannot be recomputed from the history, and never let a command edit a body behind the history's back.
3. **No geometry, tolerance or unit assumptions.** Tolerance and length unit are runtime data carried by the document, never a global constant. Kernel maths is `f64`; `f32` exists only at the GPU boundary. Curves and surfaces are open enums that include NURBS from day one: no code path may assume "all faces are planar" or "all edges are lines or arcs" in a public API. Test with analytic *and* freeform geometry, at several scales (1e-3 to 1e6 model units).
4. **Stable references.** Features refer to faces, edges and vertices through persistent names derived from how the entity was created, never through an index into the current B-rep. An edit upstream must not silently retarget a downstream fillet. This is designed in from the first feature, not retrofitted ([`docs/design/names.md`](docs/design/names.md)).
5. **Determinism.** The same document gives bit-identical bodies and meshes across runs, thread counts and platforms: ordered iteration (no `HashMap` order in results), no fast-math, seeded randomness only, parallel work merged in a fixed order.
6. **Clean-room.** Implement from public specs (ISO 10303 STEP, the 3MF core specification, the STL and OBJ format descriptions), textbooks and papers, and observed behaviour of other modellers. Never copy code, shaders or assets from proprietary software. Do not copy from copyleft kernels either (OpenCascade is LGPL, CGAL is GPL/LGPL): reading their documentation to learn what an operation should do is fine, pasting or transliterating their code is not. Third-party assets must be permissively licensed, keep their licence file next to them, and get a row in `ATTRIBUTION.md` in the same change. Never commit font files.
7. **Tests are the gate, and agents close the loop.** Every change comes with tests. The working cycle is: write the test, run it, read the result, improve, repeat, until green (`docs/testing.md`). Prefer oracles a machine can check: closed-form volumes and areas, topological invariants, watertight meshes, boolean identities, round trips, CPU against GPU. Never weaken or delete a failing test to get green; fix the code, or record the limitation and return an "unsupported" error. You wrote both the code and the test, so when they disagree do not assume the test is the one that is wrong: an expectation may change only under the four conditions in `docs/testing.md` › *Changing an expectation* (a reason that is a fact, not weaker than needed, still pinned by something independent of the code, and logged in the dev log for the developer to review).
8. **Two renderers, one oracle.** The CPU renderer (`render`) is the reference; the wgpu renderer (`gpu`) must match it within the documented tolerance. Kernel and tessellator have no GPU path at all.
9. **The UI is thin and data-driven.** UI state lives in `ui-egui/src/state.rs` (serde), so the control channel can read and drive it. Colours and radii come from `theme::Tokens`, never hard-coded.
10. **Let the developer watch.** The developer follows development and automated testing live. Anything visual is verified by looking at a picture: render offscreen, or launch with `--control` and take `ui.screenshot`, then actually read the PNG. Leave evidence where the developer can see it (`docs/development.md` › Watching along) and say in Dutch what you are about to do before a long run.
11. **Never break wasm.** L0–L7 must `cargo check --target wasm32-unknown-unknown` (run `cargo xtask wasm`). File-system code is `cfg(not(target_arch = "wasm32"))` or goes through the platform services.
12. **Performance is a feature.** Benchmark heavy operations in release on a model of realistic size (thousands of faces, a million triangles). Regenerate only the features downstream of an edit, tessellate faces in parallel (`std::thread::scope`, merged in face order), cache per feature revision, and record before/after timings in the dev log.
13. **Never panic on input** (see *Never crash* above). A command's `run` closure and anything it calls must return `Err`, never panic, for *any* params or document state. The `panic_hunt` integration test (`crates/engine/tests/panic_hunt.rs`) runs every command of the registry with adversarial params in four document states and must stay green; a new command is picked up automatically from its `params` string.
14. **Platform, not applications.** 3DCraft is the platform and must become complete; applications built on it (a gear or pulley generator, a laser-box maker, an agent that reverse-engineers scans) are separate projects that use 3DCraft through its commands, MCP or the engine crate. When in doubt ask: *Does it work on any shape, for anyone who designs with 3DCraft, or does it make one kind of product?* Any shape: it belongs here, in its general form. One kind of product, or one way of working on top of the commands: a separate project (`docs/decisions.md`, 2026-10-14).

## 4. Picking work

Priorities: important infrastructure first, then the shortest path to a model that loads, regenerates, renders and exports, then breadth.

1. Read `docs/roadmap.md` → **Current focus**. Pick from the top unless the developer asked for something else.
2. Read `log/devlog.md` → the "Still open" bullets of recent entries.
3. Check `docs/decisions.md` → *Open decisions*. If your work depends on one, ask the developer instead of choosing silently.

When you finish a milestone item, update its status in `docs/roadmap.md` with the date. Keep the roadmap readable: a table cell holds at most one sentence; what was done goes as one line with its date under **Done** in the milestone's section; what is left goes under **Open**; how it was built goes in its detail document under `docs/design/` (the roadmap links to it). **The map (docs/architecture.md) changes only when the structure changes; a finding or fix goes into its detail document under docs/design/.** The same holds for the status paragraph at the top of this file: it names stages, not features.

## 5. Before you finish a task

```sh
cargo fmt --all
cargo test -p <crates you touched>
cargo clippy -p <crates> --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm            # if you touched L0–L7
cargo xtask freecad         # if you touched tess, meshio, step, drawing, io or file.export: FreeCAD reads our mesh, STEP, DXF and SVG exports (needs CRAFT3D_FREECAD)
cargo test -p craft3d-engine --test panic_hunt                # if you added/changed commands (seconds; also part of the full test run)
```

`cargo xtask ci` runs fmt, clippy, tests, layers and wasm in one go; the tests include the performance benchmark, which counts work against `crates/engine/tests/bench_models/baseline.txt` (updating it is an expectation change, `docs/testing.md` › Performance tests). It is the gate: CI on GitHub (which adds macOS and Windows) runs only weekly and on demand (`gh workflow run CI`), so a commit is never checked there automatically.

**Two steps, so the gate is not paid for every commit** (the developer, 2026-10-13):

- **Per commit, the quick check:** `cargo fmt --all --check`, `cargo clippy -p <crates you touched> --all-targets -- -D warnings`, `cargo test -p <crates you touched>`, and the tests of the crates that depend on them when you changed behaviour or a public API. A commit that changes only docs, comments or the data of one test needs just the tests it can affect. Such commits go on a work branch, not on `main`.
- **Before `main` moves, the full gate:** `cargo xtask ci` on the exact commit, once per finished piece of work (a merged branch, a finished round), not once per commit. `main` only fast-forwards to a commit the full gate passed. Run it on a separate checkout so work can go on meanwhile, and read its exit status, not its last lines. If `cargo` is not on `PATH` in your shell, it is in `~/.cargo/bin`.

While iterating, run less and keep the full commands above as the final gate: `cargo test -p <crate> --lib --tests` skips examples and doctests, and `cargo test -p <crate> --test <file>` runs one integration-test file.

Commands must **never panic** on bad input (Rule 13). New commands come with a graceful-failure test (empty/out-of-range/wrong-type params → `Err`, not a crash).

Then append a terse entry to `log/devlog.md` (what landed, numbers, what's still open). Sessions can end abruptly (crashes, context limits), so the dev log plus a green tree is how the next agent picks up. Keep the tree building at every step.

## 6. Parallel agents

- Use your own target dir (`CARGO_TARGET_DIR=target/agent-<name>`) to avoid the Cargo build lock, and edit only the files you own. Shared files (`engine/src/lib.rs` with the list of `SPECS` in `Session::commands`, `engine/src/commands.rs`, `ui-egui/src/state.rs`) get small, surgical edits; re-read before editing.
- Put new commands in a **new module** (`engine/src/<area>_cmds.rs` with its own `SPECS`) rather than growing a shared file.
- If someone else's in-progress edit breaks the build, wait and retry; don't fix their files.
- Keep every `Cargo.toml` valid at all times: the `crates/*` glob means one broken manifest breaks everyone's build. **Create or rewrite manifests atomically**: write to a temp file outside `crates/`, then `mv` it into place.
- A window or server you start for a test gets `--recovery-dir none` or a directory of its own: autosave is on by default in the developer's state directory, and a window you kill would otherwise show up in the developer's recovery bar.
- Disk space: target dirs grow to several GB each. Delete `target/agent-*` dirs of finished agents.

## 7. Where things are tracked

- `docs/roadmap.md`: milestones, status and the current focus.
- `docs/decisions.md`: decisions taken and open.
- `plan/` (local, gitignored): research, plans, estimates.
- `log/` (local, gitignored): the dev log.

## 8. Keeping the native format complete

`craft3d-format` deliberately fails to compile when a `craft3d-doc` struct gains a field (exhaustive destructuring in `convert.rs`), so nothing is silently dropped from `.3dcraft` saves. When you add a doc field, add it to `crates/format/src/manifest.rs` and `convert.rs` with `#[serde(default)]` so older files still load, and bump the format version with a migration when the meaning of existing data changes. A saved file must regenerate to the same bodies it was saved from: the order in which features, constraints and formulas were applied is part of the file.
