# Roadmap

> **Last updated:** 2026-10-12 · **Stage:** M5 done: sketches are drawn, constrained and solved through commands; nothing is built from them yet · **Change:** minor (M5 done)

Forward-looking plan: milestones with a definition of done, and the **Current focus** list agents pick from. There is no parity percentage: 3DCraft is AI-first with a minimal UI and has no target app ([`decisions.md`](decisions.md)). Progress is measured by milestones whose acceptance tests pass.

Status legend: ✅ done · 🟡 in progress · ⬜ not started. Put the date next to a status when it changes. No effort estimates yet: they are added once M0–M2 give a calibration from this repository's own history.

## Phase 1: a model you can build, save, see and export, without a UI

Phase 1 is M0–M8. Its acceptance scenario, run entirely by an agent over MCP with the developer watching the viewer:

> Create a document. Define variables `width`, `height`, `thickness`, `hole_d`. Sketch an L-shaped profile constrained by those variables, extrude it, cut two holes, save as `.3dcraft`. Reopen it, change `width`, regenerate: the holes follow. Export STEP and 3MF. `document.inspect` reports the volume predicted by hand calculation; an independent program opens the STEP file and reports the same volume.

## Current focus

1. **M6 Extrude and revolve**: the first bodies that are not primitives. Starts with profile detection (closed regions from a sketch's entities, moved here from M5 by the developer), then extrude with oracle 1 (volume = profile area × height). This is where persistent names have to be designed, and where regeneration gets its per-feature cache (today every edit solves every sketch again, twice).
2. **Left open by M5,** in order of weight: (a) which constraint is blamed in a nonlinear conflict is the solver's heuristic, not a proof, and about 8% of dense random sketches (up to 40 arbitrary constraints) end as "did not converge" instead of a diagnosis; to be judged again on real models; (b) there is no picture of a sketch: `view.render` draws bodies only; (c) `panic_hunt` now takes 2.5 minutes; (d) two seeded breakages of the solver's rank test are caught only by unit tests or the random run, not by a small natural sketch.
3. **M4 Viewer** when the developer wants to watch live: the desktop app with the control server. The CPU renderer already gives pictures (`craft3d-cli render`), so this does not block modelling work.
4. **Gaps found during M3,** to close before booleans (M7) can produce such faces: `mass_properties` for a torus face bounded by loops around the tube and for a boundary through a pole or apex (both are refused today, with ignored tests that hold the expected values); `Surface::project` near a collapsed patch edge (returns a coarse sample; the tessellator works around it); a self-intersection check.
5. Small things left open: a real MCP client test, `thumb.png` in `.3dcraft`, autosave and crash recovery, an `enabled` predicate per command, thread scaling of the tessellator on a body with many faces (unmeasured).

## Milestones

| M | Name | Status | Scope (key items) | DoD / acceptance |
|---|---|---|---|---|
| **T0** | Teapot (fast track) | ✅ 2026-10-12 | The shortest path to a picture, cutting across M0, M1, M3 and M4: Bézier patches in `geom`, a uniform-grid tessellator in `tess`, the CPU reference renderer in `render`, the Utah teapot fixture in `testkit` | `cargo run --release -p craft3d-render --example teapot` writes a correct teapot; 35 tests green (deviation within tolerance at three scales, seam vertices identical, no pinholes, deterministic output); clippy, layers and wasm pass |
| **T1** | Mesh export (pulled forward from M3) | ✅ 2026-10-12 | `meshio`: binary/ASCII STL, 3MF with its own ZIP writer, OBJ; `io`: exact welding, solid/surface classification, `export_mesh`; exact seam and collapsed-edge vertices in `tess` | FreeCAD reads a box as a solid of volume 24 and the teapot with identical facet and point counts, area and bounding box, in all four encodings (`cargo xtask freecad`) |
| **M0** | Skeleton | ✅ 2026-10-12 | Workspace, all crate stubs, lints, `xtask` (layers, wasm, ci, stats), CI matrix, testkit stub | `cargo test --workspace` green; `cargo xtask layers` and `cargo xtask wasm` pass; every crate carries the deny-panic attribute |
| **M1** | Foundation | ✅ 2026-10-12 (deviations: robust predicates moved to M7 where they are first needed; a seeded random robustness test stands in for a cargo-fuzz target; no inertia tensor, no parameter-space curves yet) | `math` (vectors, transforms, tolerance, predicates), `geom` (line, circle, ellipse, NURBS curves; plane, cylinder, cone, sphere, torus, NURBS surfaces: evaluate, derive, project), `topo` (B-rep arena, checker, mass properties), `expr` (units, parser, evaluator, dependency order) | Closed-form tests for every kind; checker detects each seeded defect; hand-built box/cylinder/sphere bodies give analytic volume and area to 1e-12 relative; expression fuzz target runs clean |
| **M2** | Document, format, engine, CLI | ✅ 2026-10-12 (with the start of M3: primitives as features, regeneration, B-rep tessellation for the five primitives. Deviations: synchronous engine without jobs or events; undo as snapshots; no autosave or crash recovery) | `doc` (variables, feature list, sketch and constraint data), `ops` (undo), `format` (`.3dcraft`, atomic save, migrations scaffold), `engine` (Session, registry, snapshots), `craft3d-cli` (`info`, `run`, `commands`, `serve`), `automation` (MCP headless), `panic_hunt` | Load/save round trip of every doc type; a document built by commands over MCP saves and reopens identically; every command survives `panic_hunt` |
| **M3** | Primitives, tessellator, mesh export | ✅ 2026-10-12 (deviation: regeneration is still a full rebuild per revision; the per-feature cache moves to M6, where features first depend on each other) | `kernel` primitives as features; `regen` v1 (linear, full); `tess` (edge-first, CDT in parameter space, refinement); `meshio` + `io` (STL, 3MF, OBJ); `document.measure` | Oracle 1 and 4 for all primitives at three scales; exported meshes closed and oriented; mesh volume converges to the analytic value; byte-identical output for 1/2/N threads |
| **M4** | Renderers and viewer | ⬜ | `render` (CPU: shaded, edges, depth, id buffer), `gpu` (wgpu), `ui-egui` + `apps/craft3d` (viewport with orbit/pan/zoom, feature tree read-only, open/save), desktop control server, `ui.screenshot`, MCP bridge, offscreen snapshot example, CPU fallback | CPU goldens blessed by the developer; GPU vs CPU parity within tolerance on lavapipe; the developer watches an agent build a primitive scene live; app starts without a GPU |
| **M5** | Sketches and constraint solver | ✅ 2026-10-12 (deviations: profile detection moved to M6 by the developer; sketches lie on world planes with an offset until datum planes exist; no splines) | Sketch entities and constraints as commands; `solve` (Newton-type, DOF analysis, redundancy/conflict diagnosis, stored solution); expressions in dimensions | Oracle 6; a catalogue of 20 sketches solves to hand-checked coordinates; changing a variable re-solves without flipping; no sketch makes the solver hang |
| **M6** | Extrude, revolve, persistent names | ⬜ | Profile detection (closed regions from sketch entities); `kernel` extrude/revolve from profiles; origin maps; `PersistentName` scheme and resolution; incremental `regen` with per-feature cache; datums on faces; transform, mirror, pattern | Oracle 1 for extrusions and revolutions (Pappus); incremental = full regeneration; topological naming suite v1 passes |
| **M7** | Booleans | ⬜ | Surface/surface intersection for analytic pairs; imprint, classify, sew; union/subtract/intersect; cut-extrude and holes; tolerant handling of coincident and tangent cases, or a clear `Unsupported` | Oracle 3 on generated inputs; box-minus-cylinder family exact; naming suite extended through booleans; no invalid body ever returned (proptest) |
| **M8** | STEP export | ⬜ | `step` Part 21 writer/reader core, typed B-rep entities, `io` mapping, units and uncertainty | Parse-back equality; external validator opens every test model and reports matching volume; **phase 1 acceptance scenario passes** |
| **M9** | Local operations and sweeps | ⬜ | Fillet and chamfer (constant radius), shell, draft, sweep, loft; NURBS in booleans | Oracle 1 where closed forms exist (filleted box edges); validity on generated inputs |
| **M10** | STEP import, corpus, web | ⬜ | **Before imported bodies enter the document:** share large blobs between undo snapshots instead of copying them (undo keeps a whole document per step, see [decisions](decisions.md)). STEP reader to bodies (`Import` feature); pinned public STEP corpus; `craft3d-web`; performance budgets and a generated scorecard | Corpus import floor set and enforced; web build loads and views a model |
| **M11+** | Interactive UI | ⬜ | Sketching and feature editing with the mouse, menus, selection, shortcuts, measurement tools; later: assemblies, drawings | Defined when phase 1 is done; depends on whether a target app is chosen |
| **M12+** | Later candidates | ⬜ | Not planned in detail; listed so they are not forgotten. **Undo as reversible operations** instead of snapshots: needed only if edits must be merged (two users at once, or a background job that finishes after the document moved on), or to show exactly what a step changed. It touches the `ops` crate only: commands use `History::apply`/`undo`/`redo` and do not know how it works inside | Decided when a feature needs it |

## Risks, in order

1. **Booleans (M7)** are where CAD kernels fail. Mitigation: analytic pairs first, strict "valid or `Err`", property tests from day one, no feature may depend on a boolean case that is not covered by tests.
2. **Persistent naming (M6)** is a design problem, not an implementation one. Mitigation: prototype with the naming suite before M7 builds on it; record the scheme in `architecture.md`.
3. **Numerical robustness** everywhere. Mitigation: tolerance as data, guarded floating point (never-crash rules), scale tests, the predicates decision in [`decisions.md`](decisions.md).
4. **STEP acceptance by other systems (M8).** Mitigation: the external validator in the loop as early as M8 starts.
5. **Scope.** A kernel is years of work for a human team. Mitigation: phase 1 is deliberately narrow (prismatic parts with holes), and every milestone ends with something the developer can see and an agent can verify.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | M5 done (sketches, constraints, own solver, hardened so that solving twice gives the same verdict); profile detection moved to M6; current focus is extrude |
| 2026-10-12 | minor | M3 done; current focus is sketches (M5) and extrude (M6) |
| 2026-10-12 | minor | M2 done, M3 partly; current focus rewritten |
| 2026-10-12 | minor | Undo follow-ups recorded: shared blobs before M10, reversible operations as an M12+ candidate |
| 2026-10-12 | minor | M1 done; current focus is M2 |
| 2026-10-12 | minor | T1 mesh export done and validated with FreeCAD |
| 2026-10-12 | minor | M0 done: all crate stubs, full layer table, lint opt-in check, `xtask stats`, pinned toolchain, CI on GitHub |
| 2026-10-12 | minor | T0 teapot track done; M0 in progress; current focus rewritten after the developer's decisions (MIT, AI-first, FreeCAD as oracle) |
| 2026-10-12 | major | First version, from `BRAINSTORM.md` |
