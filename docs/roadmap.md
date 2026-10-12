# Roadmap

> **Last updated:** 2026-10-12 · **Stage:** M0 in progress; teapot renders · **Change:** minor (teapot track done, decisions settled)

Forward-looking plan: milestones with a definition of done, and the **Current focus** list agents pick from. There is no parity percentage: 3DCraft is AI-first with a minimal UI and has no target app ([`decisions.md`](decisions.md)). Progress is measured by milestones whose acceptance tests pass.

Status legend: ✅ done · 🟡 in progress · ⬜ not started. Put the date next to a status when it changes. No effort estimates yet: they are added once M0–M2 give a calibration from this repository's own history.

## Phase 1: a model you can build, save, see and export, without a UI

Phase 1 is M0–M8. Its acceptance scenario, run entirely by an agent over MCP with the developer watching the viewer:

> Create a document. Define variables `width`, `height`, `thickness`, `hole_d`. Sketch an L-shaped profile constrained by those variables, extrude it, cut two holes, save as `.3dcraft`. Reopen it, change `width`, regenerate: the holes follow. Export STEP and 3MF. `document.inspect` reports the volume predicted by hand calculation; an independent program opens the STEP file and reports the same volume.

## Current focus

1. **Finish M0.** Still open: a CI workflow on GitHub (fmt, clippy, tests, layers, wasm), `xtask stats`, stubs for the remaining crates as each is first needed (not all up front).
2. **Mesh export of what we can already tessellate** (pulled forward from M3): `meshio` with STL, 3MF and OBJ writers, and the first FreeCAD-validated export test (the teapot mesh opens in FreeCAD with the same bounding box and area).
3. **M1 Foundation.** `math` (transforms, tolerance type, predicates), `geom` (curves; analytic surfaces; the `Curve`/`Surface` enums; NURBS with knots and weights on top of the Bézier code), `topo` with the validity checker and mass properties, `expr`.
4. **A `craft3d-cli render`** so a picture no longer needs a cargo example (small; can ride along with 2).

## Milestones

| M | Name | Status | Scope (key items) | DoD / acceptance |
|---|---|---|---|---|
| **T0** | Teapot (fast track) | ✅ 2026-10-12 | The shortest path to a picture, cutting across M0, M1, M3 and M4: Bézier patches in `geom`, a uniform-grid tessellator in `tess`, the CPU reference renderer in `render`, the Utah teapot fixture in `testkit` | `cargo run --release -p craft3d-render --example teapot` writes a correct teapot; 35 tests green (deviation within tolerance at three scales, seam vertices identical, no pinholes, deterministic output); clippy, layers and wasm pass |
| **M0** | Skeleton | 🟡 2026-10-12 (workspace, never-crash lints, `xtask layers`/`wasm`/`ci`, git + GitHub remote; CI workflow and `stats` open) | Workspace, all crate stubs, lints, `xtask` (layers, wasm, ci, stats), CI matrix, testkit stub | `cargo test --workspace` green; `cargo xtask layers` and `cargo xtask wasm` pass; every crate carries the deny-panic attribute |
| **M1** | Foundation | ⬜ | `math` (vectors, transforms, tolerance, predicates), `geom` (line, circle, ellipse, NURBS curves; plane, cylinder, cone, sphere, torus, NURBS surfaces: evaluate, derive, project), `topo` (B-rep arena, checker, mass properties), `expr` (units, parser, evaluator, dependency order) | Closed-form tests for every kind; checker detects each seeded defect; hand-built box/cylinder/sphere bodies give analytic volume and area to 1e-12 relative; expression fuzz target runs clean |
| **M2** | Document, format, engine, CLI | ⬜ | `doc` (variables, feature list, sketch and constraint data), `ops` (undo), `format` (`.3dcraft`, atomic save, migrations scaffold), `engine` (Session, registry, snapshots), `craft3d-cli` (`info`, `run`, `commands`, `serve`), `automation` (MCP headless), `panic_hunt` | Load/save round trip of every doc type; a document built by commands over MCP saves and reopens identically; every command survives `panic_hunt` |
| **M3** | Primitives, tessellator, mesh export | ⬜ | `kernel` primitives as features; `regen` v1 (linear, full); `tess` (edge-first, CDT in parameter space, refinement); `meshio` + `io` (STL, 3MF, OBJ); `document.measure` | Oracle 1 and 4 for all primitives at three scales; exported meshes closed and oriented; mesh volume converges to the analytic value; byte-identical output for 1/2/N threads |
| **M4** | Renderers and viewer | ⬜ | `render` (CPU: shaded, edges, depth, id buffer), `gpu` (wgpu), `ui-egui` + `apps/craft3d` (viewport with orbit/pan/zoom, feature tree read-only, open/save), desktop control server, `ui.screenshot`, MCP bridge, offscreen snapshot example, CPU fallback | CPU goldens blessed by the developer; GPU vs CPU parity within tolerance on lavapipe; the developer watches an agent build a primitive scene live; app starts without a GPU |
| **M5** | Sketches and constraint solver | ⬜ | Sketch entities and constraints as commands; `solve` (Newton-type, DOF analysis, redundancy/conflict diagnosis, stored solution); expressions in dimensions; profile detection | Oracle 6; a catalogue of 20 sketches solves to hand-checked coordinates; changing a variable re-solves without flipping; no sketch makes the solver hang |
| **M6** | Extrude, revolve, persistent names | ⬜ | `kernel` extrude/revolve from profiles; origin maps; `PersistentName` scheme and resolution; incremental `regen` with per-feature cache; datums on faces; transform, mirror, pattern | Oracle 1 for extrusions and revolutions (Pappus); incremental = full regeneration; topological naming suite v1 passes |
| **M7** | Booleans | ⬜ | Surface/surface intersection for analytic pairs; imprint, classify, sew; union/subtract/intersect; cut-extrude and holes; tolerant handling of coincident and tangent cases, or a clear `Unsupported` | Oracle 3 on generated inputs; box-minus-cylinder family exact; naming suite extended through booleans; no invalid body ever returned (proptest) |
| **M8** | STEP export | ⬜ | `step` Part 21 writer/reader core, typed B-rep entities, `io` mapping, units and uncertainty | Parse-back equality; external validator opens every test model and reports matching volume; **phase 1 acceptance scenario passes** |
| **M9** | Local operations and sweeps | ⬜ | Fillet and chamfer (constant radius), shell, draft, sweep, loft; NURBS in booleans | Oracle 1 where closed forms exist (filleted box edges); validity on generated inputs |
| **M10** | STEP import, corpus, web | ⬜ | STEP reader to bodies (`Import` feature); pinned public STEP corpus; `craft3d-web`; performance budgets and a generated scorecard | Corpus import floor set and enforced; web build loads and views a model |
| **M11+** | Interactive UI | ⬜ | Sketching and feature editing with the mouse, menus, selection, shortcuts, measurement tools; later: assemblies, drawings | Defined when phase 1 is done; depends on whether a target app is chosen |

## Risks, in order

1. **Booleans (M7)** are where CAD kernels fail. Mitigation: analytic pairs first, strict "valid or `Err`", property tests from day one, no feature may depend on a boolean case that is not covered by tests.
2. **Persistent naming (M6)** is a design problem, not an implementation one. Mitigation: prototype with the naming suite before M7 builds on it; record the scheme in `architecture.md`.
3. **Numerical robustness** everywhere. Mitigation: tolerance as data, guarded floating point (never-crash rules), scale tests, the predicates decision in [`decisions.md`](decisions.md).
4. **STEP acceptance by other systems (M8).** Mitigation: the external validator in the loop as early as M8 starts.
5. **Scope.** A kernel is years of work for a human team. Mitigation: phase 1 is deliberately narrow (prismatic parts with holes), and every milestone ends with something the developer can see and an agent can verify.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | T0 teapot track done; M0 in progress; current focus rewritten after the developer's decisions (MIT, AI-first, FreeCAD as oracle) |
| 2026-10-12 | major | First version, from `BRAINSTORM.md` |
