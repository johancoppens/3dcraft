# 3DCraft design: The design plan of 2026-10-12

The design plan v1 of 2026-10-12 as it was written, with the parts of the old single `architecture.md` that belonged to no one part: goals and principles, the CAD holes to avoid, the workspace layout and layering as first planned, the test layers, the workspace sketch, the open questions, and that file's revision history.

> **The original target design of 2026-10-12, kept as written.** Where it differs from the code, the map ([`../architecture.md`](../architecture.md)) and the Method sections of the detail documents win.

## Method

This document is history. The structure that holds now is the map, [`../architecture.md`](../architecture.md): where the two differ, the map and the detail documents win. Most of the plan was built as written; where it was not (the plan's `Session` with jobs, events and lock-free snapshots; persistent bodies that share unchanged parts; a stored evaluation order of variables; per-entity tolerances; `Skipped` statuses; rayon; content-addressed blobs; a `fuzz/` directory), the detail documents describe what was built instead, and the map's structural-debt table lists what is still owed.

## Findings and history

> Moved verbatim from `docs/architecture.md` on 2026-10-15. Section numbers (§4.1, §5.3, …) in the text are those of that file before the split; [README.md](README.md) says where each one is now.

> **Last updated:** 2026-10-15 · **Status:** design plan v1; first crates built · **Change:** minor (showcase findings, §5.3 › *Showcase findings (2026-10-15)*: a curve a box cuts is refined, outlines made when needed, one vertex where two edges meet, the mass of a face settled part by part with its work bounded by its first sums)

### Current state (2026-10-12)

What exists on `main` today; where the code differs from the plan below, this summary wins.

- **Workspace:** 22 library crates (the 20 of the plan plus `zipstore` and `drawing`), all registered in `xtask/src/layers.rs`. With code: `math`, `geom`, `topo`, `expr`, `doc`, `ops`, `solve`, `kernel`, `tess`, `regen`, `render`, `zipstore`, `meshio`, `step`, `drawing`, `format`, `io`, `engine`, `automation`, `testkit`, `gpu`. `apps/craft3d-cli` is the headless CLI and `apps/craft3d` the desktop app. `xtask` has `layers` (layering and lint opt-in), `wasm`, `ci`, `stats` and `freecad`.
- **The one path works end to end** for primitive solids: command → document → regeneration → B-rep (checked) → tessellation → render or mesh file, and FreeCAD reads the result as a closed solid of the right volume.

---

This plan sets out:
- the Cargo workspace,
- the rules that keep the engine independent of the UI,
- the document model and how bodies are derived from it,
- how the kernel, tessellator, renderers, file formats and automation fit together.

When code exists and differs from this plan, add a dated "Current state" section at the top; that section wins over the plan below.

---

### 1. Goals and principles

**Goals**
- A native, portable (Linux / macOS / Windows), fully open-source parametric 3D CAD modeller with its **own kernel and tessellator**, in pure Rust.
- **Built to be driven by AI.** Every action is a command with a stable id and serialisable parameters. An agent can build, inspect, render and export a model without a window.
- **Phase 1 has (almost) no UI:** load, save, view. The modelling surface is the command registry.
- **The design history is saved,** not just the result: sketches, constraints, formulas, variables, and the order in which they were applied.
- **Export** to STEP (exact B-rep) and to STL / 3MF / OBJ (meshes).
- **The UI can be swapped** without touching the core, and the core compiles to `wasm32`.

**Principles**

1. **Engine-first, headless-first.** Every feature is reachable without a GUI: from tests, the CLI, the control channel and MCP. The GUI is one client of the engine.
2. **Everything is a command.** Each action has a stable `CommandId` (such as `feature.extrude`) with typed, serialisable parameters. The UI, shortcuts, CLI, control channel, MCP and recorded scripts all dispatch the same commands.
3. **History is the source of truth; geometry is derived.** The document stores *how* the model is built. B-rep bodies and meshes are caches of that recipe.
4. **One algorithm, one implementation, one parameter struct.** Kernel and tessellator are CPU-only, deterministic Rust. Only *display* has a second (GPU) implementation, and it is checked against the CPU one.
5. **Immutable snapshots, structural sharing.** Documents and bodies are persistent data: `Arc`-shared, copy-on-write. Taking a snapshot is O(1); undo, background regeneration, autosave and UI reads are cheap and lock-free. (This is PhotoCraft's "copy-on-write tiles" applied to features and faces instead of pixels.)
6. **Determinism.** The same document gives bit-identical bodies and meshes across runs, thread counts and platforms.
7. **Pure Rust.** No C/C++ dependencies in the kernel, tessellator, formats or engine. System libraries appear only at the windowing/GPU edge (through winit and wgpu).
8. **Clean-room.** Public specs, textbooks, papers and observed behaviour only. No proprietary code, shaders or assets; no code from copyleft kernels.

#### 1.1 Avoiding the usual CAD holes

PhotoCraft's plan starts from GIMP's lesson: properties baked into the core as assumptions (8-bit pixels, RGB only) cost years to undo. CAD systems have their own versions of that mistake. Each rule below is cheap on day one and a rewrite later.

| Property | The hole | Rule we adopt from day one | Where |
|---|---|---|---|
| **Topological naming** | Features that refer to "face 7" of the current body break or silently retarget when an upstream edit renumbers the faces. Open-source modellers have spent years retrofitting a fix | Every face, edge and vertex carries a **persistent name** derived from its origin (the feature that made it, the sketch entity it came from, the operation and operands that split or merged it). Features store names, never indices. A name that no longer resolves is a reported error on that feature, never a guess | `topo`, `kernel`, `regen`, `doc` |
| **Tolerance** | A global `EPSILON` works until a model is very small, very large, or imported from a system with another convention | Tolerance is **data**: a document-level linear and angular tolerance, plus per-entity tolerances on vertices and edges where an operation or import needed them. Every predicate takes a tolerance argument. No `const EPS` in public APIs | `math`, `topo`, `kernel` |
| **Units** | Unitless numbers that mean millimetres, until a file in inches arrives | The document has a length unit and an angle unit; expressions carry dimensions (`10 mm + 1 in` is valid, `10 mm + 30 deg` is an error). The kernel works in the document's unit; conversion happens at the format edges | `expr`, `doc`, `io` |
| **Geometry kinds** | Starting with planes and cylinders "for now" and discovering that every operation has to be reopened for freeform surfaces | `Curve` and `Surface` are enums that include NURBS from M1. An operation that does not yet handle a pair of kinds returns `Unsupported { op, kinds }`; it never assumes the case away. Analytic kinds stay analytic (a cylinder is not stored as a NURBS) so results stay exact where they can be | `geom`, `kernel` |
| **Precision** | `f32` in the modelling path, or mixing display and model precision | Kernel maths is `f64` throughout. `f32` exists only in vertex buffers handed to the GPU, converted relative to a per-view origin so large coordinates keep precision | `math`, `gpu` |
| **Result vs recipe** | Saving only the final solid, so a design cannot be edited | The native format stores the history. Cached bodies in a file are optional and verified by hash; a file with every cache removed opens to the same model | `doc`, `format` |
| **Single body, manifold only** | Assuming one solid per document, or that every result is a closed manifold | A document holds any number of bodies. A body is a list of shells and may be a solid, a sheet or a wire; validity is checked and reported, not assumed | `topo`, `doc` |
| **Regeneration order** | Implicit ordering by creation time or hash-map order, so reopening a file gives a different model | The feature list is an explicit ordered sequence; variables and formulas form an explicit dependency graph with a stored, deterministic evaluation order; the solver's constraint order is stored | `doc`, `regen`, `solve` |
| **File formats** | Format quirks leaking into core types | Formats live in standalone crates with their own format-level model; mapping to the document lives in `io`. Adding a format touches a leaf crate | `step`, `meshio`, `drawing`, `io` |
| **UI toolkit** | A core that calls into its toolkit | Nothing below L7 knows a toolkit exists | layering rule 2 |

The test is simple: **adding a surface kind, a feature type, or a file format should touch leaf crates, not the foundation.**

---

### 2. Workspace layout

```text
3dcraft/
├─ Cargo.toml                  # [workspace], shared deps, lints, profiles
├─ crates/
│  │  ── foundation (wasm-safe, no I/O) ──
│  ├─ math/                    craft3d-math       f64 vectors/points/matrices, transforms, bounding boxes, intervals,
│  │                                              tolerance type, robust orientation predicates, root finding
│  ├─ geom/                    craft3d-geom       curves (line, circle, ellipse, NURBS, trimmed, offset) and surfaces
│  │                                              (plane, cylinder, cone, sphere, torus, extrusion, revolution, NURBS):
│  │                                              evaluation, derivatives, projection, inversion, intersection
│  │  ── standalone formats (no workspace deps; publishable) ──
│  ├─ step/                    craft3d-step       ISO 10303-21 reader/writer with its OWN model (the entities are io's business)
│  ├─ meshio/                  craft3d-meshio     STL (ASCII/binary), 3MF, OBJ writers (readers later); its OWN mesh model
│  ├─ drawing/                 craft3d-drawing    DXF and SVG writers for flat outlines; its OWN 2D model (lines, arcs,
│  │                                              ellipses, B-splines)
│  │  ── model data ──
│  ├─ topo/                    craft3d-topo       B-rep: vertex, edge, coedge, loop, face, shell, body; persistent names;
│  │                                              validity checker; mass properties
│  ├─ expr/                    craft3d-expr       units and dimensions, expression parser/evaluator, variable table,
│  │                                              dependency graph with cycle detection
│  ├─ doc/                     craft3d-doc        document model: variables, sketches, constraints, features, order
│  │                                              (pure data + serde; no geometry code)
│  │  ── algorithms (CPU, deterministic, rayon) ──
│  ├─ ops/                     craft3d-ops        reversible operations on a doc, transactions, history/undo
│  ├─ solve/                   craft3d-solve      2D geometric constraint solver (later 3D mates)
│  ├─ kernel/                  craft3d-kernel     modelling operations on B-rep: primitives, extrude, revolve, sweep, loft,
│  │                                              booleans, fillet, chamfer, shell, offset, split, transform, pattern
│  ├─ tess/                    craft3d-tess       tessellator: B-rep -> watertight triangle mesh within a chordal tolerance;
│  │                                              edge polylines for display
│  │  ── derivation and display ──
│  ├─ regen/                   craft3d-regen      evaluates the feature history into bodies; incremental, cached per feature;
│  │                                              resolves persistent names; reports per-feature status
│  ├─ render/                  craft3d-render     CPU reference renderer (the oracle): shaded faces, edges, depth, id buffer
│  ├─ gpu/                     craft3d-gpu        wgpu renderer for the same draw list; picking via id buffer
│  ├─ format/                  craft3d-format     native document format (.3dcraft): manifest + content-addressed blobs
│  │  ── I/O ──
│  ├─ io/                      craft3d-io         import/export orchestration: bodies <-> STEP, meshes -> STL/3MF/OBJ
│  │  ── the façade every frontend talks to ──
│  ├─ engine/                  craft3d-engine     Session: open docs, command registry + dispatch, jobs, events, snapshots
│  │  ── frontends ──
│  ├─ ui-egui/                 craft3d-ui-egui    the GUI shell: viewport widget, feature tree, inspector, theme
│  ├─ automation/              craft3d-automation MCP server + JSON-RPC over the command registry
│  └─ testkit/                 craft3d-testkit    analytic reference solids, mesh checks, image diff, fixtures
├─ apps/
│  ├─ craft3d/                 desktop binary (eframe + wgpu + ui-egui), TCP control server
│  ├─ craft3d-cli/             headless CLI (convert / info / run / commands / serve / mcp)
│  └─ craft3d-web/             wasm32 binary (later milestone)
├─ fuzz/                       cargo-fuzz targets (step, format, expr)
├─ xtask/                      build tasks: layers, wasm, ci, stats
├─ docs/                       this directory
├─ plan/  log/  corpus/        local, gitignored
```

**Crate granularity:** start with the crates above. Split `kernel` (for example `-boolean`, `-blend`, `-sweep`) once a module passes about 10k lines or compile times hurt; keep its module boundaries along those lines from the start.

**Naming:** directories are short and packages are prefixed `craft3d-` (a crate name cannot start with a digit). `craft3d-step` and `craft3d-meshio` get neutral names if they are ever published.

---

### 3. Dependency layering (enforced)

```text
 L8  apps/*                         (binaries: wire everything together)
 L7  ui-egui · automation
 L6  engine
 L5  io
 L4  regen · render · gpu · format
 L3  ops · solve · kernel · tess
 L2  doc
 L1  topo · expr
 L0  math · geom                    step, meshio, drawing (standalone, no workspace deps)
```

**Rules** (checked by `cargo xtask layers`, which parses `cargo metadata`):

1. A crate may depend only on crates in **lower** layers. No cycles. Sideways dependencies only where listed in `SAME_LAYER` in `xtask/src/layers.rs` (today: `geom` → `math`).
2. **Nothing below L7 may depend on any UI toolkit, winit, eframe or rfd.** Platform services (file dialogs, clipboard) reach the engine as injected functions or traits.
3. **`step`, `meshio` and `drawing` depend on no workspace crate.** The mapping between their models and ours lives in `io`. This keeps them publishable and fuzzable in isolation.
4. **`doc` contains no geometry algorithms.** It may use `topo` and `expr` *types* (names, expressions), nothing from L3.
5. **`kernel` and `tess` never depend on `gpu` or `render`.** Geometry has one implementation.
6. **wasm gate:** every crate in L0–L7 must build for `wasm32-unknown-unknown` (`cargo xtask wasm`).
7. `gpu` is optional for `engine`: the headless CLI and CI tests run `cpu-only`.

---

---

### 11. Testing and quality

Detail in [`testing.md`](../testing.md).

| Layer | What |
|---|---|
| Unit / property | per crate; geometry against closed-form values; proptest for predicates, expression evaluation, persistent structures |
| Kernel invariants | every operation result passes `topo::check`; mass properties against analytic values; boolean identities |
| Tessellation | watertightness, orientation, deviation, volume convergence |
| Regeneration | determinism, incremental = full, save → load → regenerate equality, replay |
| Formats | round trips, malformed-input sweeps, fuzz targets, external validators |
| Renderers | CPU golden images; GPU vs CPU parity |
| Engine | command-level tests; `panic_hunt` fuzzing every command |
| Portability | CI: Linux x64, macOS arm64, Windows x64, wasm32 build check |

---

### 12. Cargo workspace sketch

```toml
[workspace]
resolver = "3"
members = ["crates/*", "apps/*", "xtask"]

[workspace.package]
edition = "2024"
license = "MIT"
# rust-version: pinned to the current stable when M0 scaffolds

[workspace.dependencies]             # versions pinned at scaffold time
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
rayon = "1"
arc-swap = "1"
smallvec = "1"
blake3 = "1"
bytemuck = { version = "1", features = ["derive"] }
wgpu = "*"                           # one version shared by gpu and ui
# deliberately absent: any geometry kernel, meshing or triangulation library, nalgebra-style maths
# (math is ours and small; revisit only by a recorded decision)

[workspace.lints.rust]
unsafe_code = "forbid"

[profile.release]
lto = "thin"
codegen-units = 1

[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
opt-level = 2                        # geometry code is unusable at opt-level 0
```

Which general-purpose crates the kernel may use (linear algebra, big-number or interval arithmetic for robust predicates) is an open decision ([`decisions.md`](../decisions.md)). The default is: write it, it is part of the kernel.

---

### 13. Open questions in this plan

Tracked in [`decisions.md`](../decisions.md): target app (if any), licence, UI toolkit confirmation, external maths crates, exact vs tolerant predicates, STEP application protocol, use of open-source tools as external test oracles.

### Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | minor | Showcase findings (§5.3 › *Showcase findings (2026-10-15)*): the marcher refines the pieces of a curve a box cuts; a boolean makes a face's outline only when it needs it; two places where the same two edges meet are one vertex; the mass of a face is settled part by part, its work bounded by its first sums; the box of a pair of faces is owed |
| 2026-10-15 | minor | Move face, thicken and draft (M9): a rigid motion of faces with what lies between them carried exactly (`tweak::resurface_rigid`), a layer of faces as a body of its own by two offsets and the boolean, faces tilted about a neutral plane into planes and cones; a sketch on a turned face turns with it (§5.3 › *Face operations as built*); 66 commands |
| 2026-10-14 | minor | Copies of copies (§4.1): a name has one link per run of instance 0 or of other instances along the way, not per feature, so rows of body-from-body features have no length limit; long names read into the short form; format 15 |
| 2026-10-14 | minor | A thread is cosmetic unless `modeled` (§4.1 › *Thread*), as in other modellers; `cosmetic` still accepted by the commands; version-13 threads that were cut are `modeled` |
| 2026-10-14 | minor | A thread's size and pitch follow the model (§4.1 › *Thread*): the size from the face's diameter, a designation or a length; the pitch the coarse one of the size unless given, both found at every regeneration; format version 14 |
| 2026-10-14 | minor | Helix and thread (M9): `datum.helix`, the sweep along a helix by its screw motion, `feature.thread` (ISO metric, external, cut by the boolean, or cosmetic), the name role `thread` (§4.1 › *Helix, and the sweep along it*, *Thread*); 63 commands; format version 13 |
| 2026-10-14 | minor | Delete face (M9): holes, bosses and pockets filled from the faces round them, a chamfer or a fillet collapsed into the sharp edge (§5.3 › *Face operations as built*); a sketch on a face of a subtraction's tool looks out of the material; 61 commands; format version 12 |
| 2026-10-14 | minor | Face operations (M9): offset face and shell, by a local operation that keeps every entity and, for the shell, the boolean (§5.3 › *Face operations as built*); 60 commands |
| 2026-10-14 | minor | Sweep along a path (M9): the feature, the kernel operation (`kernel::sweep`: exact along lines, arcs and mitres, within half the tolerance along a spline), what is refused, the names of its faces (§1.1 › *Sweep along a path*); 58 commands |
| 2026-10-14 | minor | Intersections: closed forms for a cylinder that goes on in a torus and for cones along a shared ruling or with one apex; a marched branch ends on a crossing point from the radius of its arms (§5.1); the boolean takes those contacts (§5.3) |
| 2026-10-14 | minor | References that follow a body: a blend's or a face operation's faces given by the names of an earlier body are followed through the features that took it (§4.1), `document.edges` lists `short` names |
| 2026-10-14 | minor | Fillets with a constant radius (M9): the feature and the names of what it makes (§4.1); the kernel operation, which shares `kernel::blend` with the chamfer: the section, the piece of a ball in a corner, chains by a joined tool, the tangency check, what is refused, and the one contact the boolean refused (§5.3); format version 9; 57 commands |
| 2026-10-14 | minor | Chamfers (M9): the feature, what it holds of an edge and the names of what it makes (§4.1); the kernel operation by tool solids and the boolean, the ends, the corners, what is refused (§5.3) |
| 2026-10-13 | minor | Pointing at faces (M8.5): `render::pick`, `render::Highlight`, face names in the engine's scene, `view.pick`, the shell's click, `ui.pick` and `ui.highlight` |
| 2026-10-13 | minor | Reference geometry: datum planes, axes and points as features, what they are placed by, where they are used (§4.1) |
| 2026-10-13 | minor | See-through bodies: `RenderParams::opacity` in the CPU renderer (the front surface over the one behind it), `UiState::transparency` and a slider; the wgpu renderer refuses such a frame and the window draws it on the CPU (the GPU's second layer is not built) |
| 2026-10-14 | minor | The view cube (`ui-egui/src/view_cube.rs`) replaces the orbit gizmo: 26 facets for the standard views and the views between them, an animated turn, the projection on its ring; shell only |
| 2026-10-14 | minor | See-through bodies on the GPU (§7.2): two depth layers in the readback renderer and in the viewport's, the comparison of the layers and why it has no bias, a key per sample where the picture is multisampled, the CPU's blend as a table, the tolerance with two layers; the window no longer hands a see-through frame to the CPU |
| 2026-10-14 | minor | The section plane (M8.5): `RenderParams::section` in both renderers (the cut, the count of what is cut away and the cap), `render::pick_cut`, `engine/src/section.rs`, the parameter `section` of `view.render` and `view.pick`, `UiState::section` and its controls beside the view cube |
| 2026-10-13 | minor | The orbit gizmo in the viewport's corner replaces the row of view buttons (`ui-egui/src/gizmo.rs`): view state of the shell only, no command and nothing in the control protocol |
| 2026-10-13 | minor | Pointing at vertices (M8.5): vertices in the scene and under `vertices` of `document.edges`, `Scene::vertices_named`, `UiState::highlight_vertex` |
| 2026-10-14 | minor | Revolved splines (rational surfaces of revolution, a full turn in two halves), closed splines and corners as profiles, `piece` in origins and names (§4.1) |
| 2026-10-13 | minor | Pointing at edges (M8.5): `render::Viewpoint` and `Neighbourhood`, edge names and faces in the scene, `Scene::point_at` and `edges_named`, `viewport::Marks`, `UiState::highlight_edge` |
| 2026-10-12 | minor | Current state after M5 (sketches in the document model, the solver, sketch results in regeneration) |
| 2026-10-12 | minor | Current state after M3 (general tessellator, cone and torus, orientation) |
| 2026-10-12 | minor | Current state after M1 (math, geom, topo, expr) |
| 2026-10-12 | major | Design plan v1, derived from the developer's original brief and PhotoCraft's architecture plan |
