# Roadmap

> **Last updated:** 2026-10-18 · **Stage:** M7 (booleans) and M8.5 in progress, M9 started, M4 nearly done; M0–M3, M5, M6 and M8 done · **Change:** minor (boolean checklist item 5: sheet bodies, `feature.fill`, a sheet trimmed by a solid and a solid cut by a sheet)

Forward-looking plan: the **Current focus** agents pick from, one line per milestone, and per milestone what it is for, what is done, what is open and what "done" means. There is no parity percentage: 3DCraft is AI-first with a minimal UI and has no target app ([`decisions.md`](decisions.md)). Progress is measured by milestones whose acceptance tests pass. How something was built (algorithms, thresholds, measurements, file paths of internals) is in the detail documents under [`design/`](design/README.md) (the structure in [`architecture.md`](architecture.md)), and how it is tested in [`testing.md`](testing.md).

Status legend: ✅ done · 🟡 in progress · ⬜ not started. Put the date next to a status when it changes. No effort estimates yet: they are added once M0–M2 give a calibration from this repository's own history.

**Keeping this file readable:** a table cell holds at most one sentence; what was done goes as one line with its date under **Done** in the milestone's section; what is left goes as a checkbox under **Open**, in priority order; how it was built goes in its detail document under `design/`, and the roadmap links to it.

## Current focus

1. **The boolean kernel (M7), the developer's first priority** (2026-10-13): a refusal is work owed. Next: the rest of the [boolean checklist](#the-boolean-checklist) in its order (the speed of a cusp's axis on a block edge, the meshes of rational faces with weights a thousand apart).
2. **M9, the rest:** replace face, and what the blends and the face operations leave open, beside M7 since none of it touches the boolean kernel ([M9](#m9-local-operations-and-sweeps)).
3. **R1, the first release:** the pinned set of modelling tasks, installers, the format promise, material and colour per body, a user guide ([R1](#r1-first-release)).

## Milestones

| M | Name | Status | Goal | Section |
|---|---|---|---|---|
| T0 | Teapot (fast track) | ✅ 2026-10-12 | The shortest path to a picture: Bézier patches, a grid tessellator and the CPU renderer draw the Utah teapot. | [T0](#t0-teapot-fast-track) |
| T1 | Mesh export | ✅ 2026-10-12 | STL, 3MF and OBJ export that FreeCAD reads back as the same solid. | [T1](#t1-mesh-export) |
| M0 | Skeleton | ✅ 2026-10-12 | The workspace, every crate stub, the lints and the `xtask` checks. | [M0](#m0-skeleton) |
| M1 | Foundation | ✅ 2026-10-12 | Maths, curves and surfaces, B-rep topology and expressions, each held to closed forms. | [M1](#m1-foundation) |
| M2 | Document, format, engine, CLI | ✅ 2026-10-12 | A document built by commands saves as `.3dcraft` and reopens identically. | [M2](#m2-document-format-engine-cli) |
| M3 | Primitives, tessellator, mesh export | ✅ 2026-10-12 | Primitives as features, regenerated into bodies, meshed edges first and exported. | [M3](#m3-primitives-tessellator-mesh-export) |
| M4 | Renderers and viewer | 🟡 2026-10-12 | A window where the developer watches an agent model, drawn by the CPU or the wgpu renderer. | [M4](#m4-renderers-and-viewer) |
| M5 | Sketches and constraint solver | ✅ 2026-10-12 | Sketches with constraints solved by our own solver, with a diagnosis of what is wrong. | [M5](#m5-sketches-and-constraint-solver) |
| M6 | Extrude, revolve, persistent names | ✅ 2026-10-13 | Profiles extruded and revolved into solids whose faces keep their names through edits. | [M6](#m6-extrude-revolve-persistent-names) |
| M7 | Booleans | 🟡 2026-10-13 | Union, subtract and intersect for every surface kind in any position, valid or a clear error. | [M7](#m7-booleans) |
| M8 | STEP export | ✅ 2026-10-13 | STEP files that an independent reader opens with our volume. | [M8](#m8-step-export) |
| M8.5 | A history that survives edits, datums, pointing | 🟡 2026-10-14 | Failed features that can be repaired, reference geometry, extrude ends and pointing in the viewport. | [M8.5](#m85-a-history-that-survives-edits-datums-pointing) |
| M9 | Local operations and sweeps | 🟡 2026-10-14 | Blends, face operations, sweeps and threads, the features of everyday mechanical parts. | [M9](#m9-local-operations-and-sweeps) |
| R1 | First release | 🟡 2026-10-15 | The first version for people outside the project, after M9. | [R1](#r1-first-release) |
| V1 | Design verification | ⬜ | An agent checks a design by queries and assertions kept in the document. | [V1](#v1-design-verification) |
| M10 | STEP import, corpus, web | ⬜ | STEP files read into the document, a public STEP corpus and the browser build. | [M10](#m10-step-import-corpus-web) |
| M11+ | Interactive UI | ⬜ | Sketching and editing with the mouse, once phase 1 is done. | [M11+](#m11-interactive-ui) |
| M12+ | Later candidates | ⬜ | Ideas that are not planned yet. | [Ideas](#ideas-m12) |

## Phase 1: a model you can build, save, see and export, without a UI

Phase 1 is M0–M8. Its acceptance scenario, run entirely by an agent over MCP with the developer watching the viewer:

> Create a document. Define variables `width`, `height`, `thickness`, `hole_d`. Sketch an L-shaped profile constrained by those variables, extrude it, cut two holes, save as `.3dcraft`. Reopen it, change `width`, regenerate: the holes follow. Export STEP and 3MF. `document.inspect` reports the volume predicted by hand calculation; an independent program opens the STEP file and reports the same volume.

## The boolean checklist

The developer's first priority (2026-10-13): what makes 3DCraft pro. Each item is measured by the share of seeded random pairs that is refused (by reason), with zero wrong or invalid results. Two independent oracles belong to it: a pinned corpus of cases with FreeCAD's answers ([`testing.md`](testing.md) › *The boolean corpus*, with its history), and a mesh-level self-intersection detector. How the kernel does it: [`design/booleans.md`](design/booleans.md) › *Booleans as built*.

1. 🟡 **Every surface kind, NURBS included, in any position.** Analytic, polynomial and rational freeform faces, faces with a pole and faces on closed spline surfaces are operands.
2. 🟡 **Every contact case:** coincident, tangent along a line or at a point, along an edge, at a vertex. Tangent contact with freeform faces and contact along a curve are built; the cases below are owed.
3. ✅ 2026-10-13 **Results without scars:** faces and edges a boolean split needlessly are merged.
4. ⬜ **Tolerant modelling:** approximated intersection curves, bodies with gaps as STEP import will bring them.
5. 🟡 **Several lumps, cavities, sheet bodies.** Lumps and cavities are built (2026-10-17); sheets are made by `feature.fill`, trimmed by a solid and cut a solid (2026-10-18, [`design/booleans.md`](design/booleans.md) › *Sheets*); two sheets and a split into both pieces are owed.
6. ✅ 2026-10-14 **Entities through booleans:** edges by the faces they lie between (2026-10-13), vertices by the faces they meet in (`document.edges`).
7. 🟡 **Speed on bodies with thousands of faces** (a spatial index). Since 2026-10-15 a face's outline (and the index of a freeform surface) is made only when the boolean needs it: a bead on a shade of 2000 faces 2.2 GB instead of 7.2 GB.
8. ✅ 2026-10-15 **Never an invalid body:** every operation is checked for crossing, overlapping and self-touching shells near what it changed, or proven unable to make them ([`design/regeneration.md`](design/regeneration.md) › *Never through itself*).

**Measured (2026-10-14, morning; the same on 2026-10-15 after the helmet's findings, after poles at a corner and coincident freeform faces, and after fourth-order contact):** 11573 of 11988 corpus outcomes agree, 249 refused (all in the category near, about 250 being the tolerance rule itself: cases built one tolerance apart or into each other), 0 wrong, 0 invalid. Speed (2026-10-13): 1024 holes one by one in 7.1 s, a cylinder through a plate of 1030 faces in 36.5 ms; the corpus test takes about 230 s on 14 threads on an idle machine (640 s at noon on 2026-10-13) and is the gate's longest binary.

**Measured (2026-10-15, after the lamp's and the impeller's findings: the pieces of a curve a box cuts refined; outlines made when needed; two places where the same two edges meet one vertex):** the same counts, digest `19af10963b6f12c2` (the same results bit for bit after all three); of the 249 refusals 21 now refuse one step later, as two faces between half and twice the tolerance apart rather than two points the tolerance cannot tell apart.

**Owed, in order:**

- [x] A cusp against a face that holds its axis through the point: built and meshed (2026-10-17).
- [ ] A cusp's axis on a block edge takes 21 to 46 s for its first boolean (47 to 86 s before 2026-10-17), in lower bounds that split the ring around the axis, and a tolerance ten times tighter for the size refuses the half ([`design/booleans.md`](design/booleans.md) › *A cusp against a face that holds its axis*, *Speed of freeform booleans and of their measures*).
- [x] Rational weights far apart measured: `topo::mass_properties` settles on weights ten thousand apart a reparametrisation evens out, and a thousand apart one cannot (2026-10-17).
- [ ] Rational weights a thousand apart in booleans: built 2026-10-17, not meshed (the tessellator does not mesh every face skewed a hundredfold and more); ten thousand apart along a degree-one direction and the plate at ten thousand still refused ([`design/booleans.md`](design/booleans.md) › *A thousand apart in booleans*).
- [x] The contact a fillet's boolean refused: tangent contact along an edge with no closed form imprinted from the edge, contact along a curve judged on its valley; `blend::join` not needed but not bit for bit, kept ([`design/booleans.md`](design/booleans.md) › *Tangent contact along an edge*, 2026-10-18).
- [x] Several lumps and cavities (item 5): one rule for which shell is which (`kernel::lumps`), shells checked one by one, held to closed forms, genus per shell, meshes, the identities of sets and FreeCAD at three scales (2026-10-17).
- [x] Sheet bodies (item 5): the check of an open shell, `feature.fill`, a sheet trimmed by a solid and a solid cut by a sheet, held to closed forms, Euler with boundary loops, meshes open only at the boundary, the identities of areas and volumes and FreeCAD at three scales (2026-10-18).
- [ ] A piece of a sheet on a face of the solid counts as inside (decided 2026-10-18, refused until built).
- [ ] Sheets, still refused: two sheets, a lump or cavity the sheet does not cross; sheets from other commands (an open profile extruded, a face copied, a loft without caps).
- [ ] `body.split`: one body per lump when asked, and a name for the piece of a face in one lump (decided 2026-10-18, [`decisions.md`](decisions.md)).
- [ ] The check of offset face, shell and delete face near what they changed (they check the whole body), and a tessellation of long straight edges that does not fan across a plate (the first cut of 400 holes at once is checked in 140 ms).
- [ ] Speed: freeform booleans and their measures 1.1 to 2.3 times faster, bit for bit (2026-10-17), measures and the self-check's meshes up to a third faster again (2026-10-18); left: the lower bounds near a cusp, the boundaries of faces worked out before their mesh is found ([`design/booleans.md`](design/booleans.md) › *Speed of freeform booleans and of their measures*).
- [ ] Tolerant modelling (item 4), before STEP import: a tolerance per vertex and edge, the kernel at the body's largest, a cap of 1000 times the tolerance (decided 2026-10-18, [`decisions.md`](decisions.md)); first slice owed.

**Oracles owed** (the developer, 2026-10-17; approaches seen in another project's test setup, built our own way):

- [x] Random multi-step designs: a seeded generator of feature sequences that places operands on a grid and on the body's own faces, so coincidence and tangency are common; each seed in its own process with a timeout, shrunk automatically to the fewest steps that still fail; a refusal counts apart from a wrong result, and the rate of each goes into the morning health report (2026-10-17: `craft3d-fuzz` and `cargo xtask fuzz`, 20 seeds in the gate; the first run, 14563 seeds, found 19 hangs or minute-long commands in offset face, shell and chamfer on blended freeform bodies, a union touching itself along a tangent line and a chamfer whose body cannot be measured, reproducers in `target/fuzz/`; [`testing.md`](testing.md) › *Oracle 11*).
- [x] An action matrix as a ratchet: every local operation (fillet, chamfer, shell, offset, delete face, mirror, …) tried on every face and edge of the showcase models; an action that succeeded before and fails now is a regression, one that fails before and succeeds now raises the floor (2026-10-17: `cargo xtask actions`, the ratchet in `crates/engine/tests/action_matrix/`, a gated subset on a small model; [`testing.md`](testing.md) › *Oracle 12*).
- [x] A cheap check after every boolean, beside `topo::check`: outside the tool's reach the result equals the target, a subtract never adds volume and a union never loses any (2026-10-17: reach and far faces in every boolean, volumes and seeded points in the tests and `document.check` deep; [`design/booleans.md`](design/booleans.md) › *The boolean's own check*).
- [ ] A second reference kernel, after the two above: NVIDIA's SMLib (`usd-brep`, Apache-2.0) as a black box beside FreeCAD, never a dependency or a source; `cargo xtask smlib` runs it in a container outside the workspace and pins its outcomes in a fixture, so tests and the gate never need it; it votes only on gross differences (shell counts, empty or not, volume off by more than a few percent), because its own measures are not exact; it scores cases where FreeCAD's reference is unreliable. A trial of 288 corpus cases (2026-10-17): all three kernels agree on 839 of 864 outcomes, SMLib is clearly wrong on 7, and 3DCraft disagrees with neither.

## Milestones in detail

### T0: Teapot (fast track)

#### Goal
The shortest path to a picture, cutting across M0, M1, M3 and M4.

#### Done
- Bézier patches in `geom`, a uniform-grid tessellator in `tess`, the CPU reference renderer in `render`, the Utah teapot fixture in `testkit` (2026-10-12)

#### Open
Nothing.

#### Acceptance
`cargo run --release -p craft3d-render --example teapot` writes a correct teapot; 35 tests green (deviation within tolerance at three scales, seam vertices identical, no pinholes, deterministic output); clippy, layers and wasm pass.

### T1: Mesh export

#### Goal
Mesh export, pulled forward from M3, so that an independent reader checks our solids early.

#### Done
- `meshio`: binary and ASCII STL, 3MF with its own ZIP writer, OBJ (2026-10-12)
- `io`: exact welding, solid/surface classification, `export_mesh` (2026-10-12)
- Exact seam and collapsed-edge vertices in `tess` (2026-10-12)

#### Open
Nothing.

#### Acceptance
FreeCAD reads a box as a solid of volume 24 and the teapot with identical facet and point counts, area and bounding box, in all four encodings (`cargo xtask freecad`).

### M0: Skeleton

#### Goal
The workspace with every crate stub, the lints, `xtask` (layers, wasm, ci, stats), the CI matrix and a testkit stub.

#### Done
- All crate stubs, the full layer table, the lint opt-in check, `xtask stats`, a pinned toolchain, CI on GitHub (2026-10-12)

#### Open
Nothing.

#### Acceptance
`cargo test --workspace` green; `cargo xtask layers` and `cargo xtask wasm` pass; every crate carries the deny-panic attribute.

### M1: Foundation

#### Goal
`math` (vectors, transforms, tolerance, predicates), `geom` (line, circle, ellipse, NURBS curves; plane, cylinder, cone, sphere, torus, NURBS surfaces: evaluate, derive, project), `topo` (B-rep arena, checker, mass properties) and `expr` (units, parser, evaluator, dependency order).

#### Done
- The four crates, held to closed forms (2026-10-12)
- A seeded random robustness test, standing in for a cargo-fuzz target (2026-10-12)
- The inertia tensor from the exact B-rep (`craft3d_topo::inertia`), for R1's material per body (2026-10-15)
- Whole-number functions in expressions: `floor`, `ceil`, `round`, `trunc` (of a plain number, or to a step) and `mod`, from the laser box's finding L1; format version 17 ([`design/document.md`](design/document.md)) (2026-10-16)

#### Open
- [ ] Robust predicates (moved to M7, where they are first needed)
- [ ] Parameter-space curves

#### Acceptance
Closed-form tests for every kind; the checker detects each seeded defect; hand-built box, cylinder and sphere bodies give analytic volume and area to 1e-12 relative; the expression fuzz target runs clean.

### M2: Document, format, engine, CLI

#### Goal
`doc` (variables, feature list, sketch and constraint data), `ops` (undo), `format` (`.3dcraft`, atomic save, migrations scaffold), `engine` (Session, registry, snapshots), `craft3d-cli` (`info`, `run`, `commands`, `serve`), `automation` (MCP headless) and `panic_hunt`.

#### Done
- The six parts above, with the start of M3: primitives as features, regeneration, B-rep tessellation for the five primitives (2026-10-12)
- Deviations: a synchronous engine without jobs or events; undo as snapshots ([`decisions.md`](decisions.md)) (2026-10-12)
- `file.list` and `file.activate`, from the first field test (2026-10-13)
- `craft3d-cli run --keep-going`: a failing command is reported on its line, the rest run and `--out` saves what was built (2026-10-15)

#### Open
- [ ] `thumb.png` in `.3dcraft`
- [ ] An `enabled` predicate per command

#### Acceptance
Load/save round trip of every doc type; a document built by commands over MCP saves and reopens identically; every command survives `panic_hunt`.

### M3: Primitives, tessellator, mesh export

#### Goal
`kernel` primitives as features, `regen` v1 (linear, full), `tess` (edge-first, CDT in parameter space, refinement), `meshio` and `io` (STL, 3MF, OBJ) and `document.measure`.

#### Done
- The parts above (2026-10-12); deviation: regeneration was a full rebuild per revision until the per-feature cache of M6
- `Surface::project` is right near a collapsed patch edge, across a seam and at a border (2026-10-13)

#### Open
- [ ] `mass_properties` for a torus face bounded by loops around the tube and for a boundary through a pole or apex of an analytic surface (refused today, with ignored tests that hold the expected values)
- [ ] Thread scaling of the tessellator on a body with many faces (unmeasured)

#### Acceptance
Oracles 1 and 4 for all primitives at three scales; exported meshes closed and oriented; mesh volume converges to the analytic value; byte-identical output for 1, 2 and N threads.

### M4: Renderers and viewer

#### Goal
`render` (CPU: shaded, edges, depth, id buffer), `gpu` (wgpu), `ui-egui` and `apps/craft3d` (viewport with orbit, pan and zoom, a read-only feature tree, open/save), the desktop control server, `ui.screenshot`, the MCP bridge, an offscreen snapshot example and a CPU fallback.

#### Done
- The desktop app `craft3d`: bodies, sketches as lines, orbit/pan/zoom, a read-only feature tree, a status bar (2026-10-12)
- The control server with `ui.*` methods and `ui.screenshot`, and the MCP bridge (`craft3d-cli mcp --bridge`), used by a real MCP client in the [field test](field-test/2026-10-12/README.md) (2026-10-12)
- The viewport renders with the wgpu renderer (`--renderer auto|cpu`, `CRAFT3D_RENDERER`), the CPU renderer as the fallback, parity within the tolerances of [`design/rendering.md`](design/rendering.md); `ui.get` reports the renderer and adapter (2026-10-13)
- The viewport draws without readback on eframe's own device, 4× MSAA, the overlay on the GPU; parity on Intel Vulkan, lavapipe and Intel GL (`crates/gpu/tests/onscreen.rs`, `crates/ui-egui/tests/gpu_viewport.rs`) (2026-10-13)
- The viewport tessellates at its own display quality (`ui.set quality`): a 4 mm hole is round instead of a twelve-gon; `view.render`, previews and export keep the automatic tolerance (2026-10-13)
- An orbit gizmo (2026-10-13), replaced by a view cube (2026-10-14); a see-through slider (2026-10-13), drawn by the wgpu renderer too (2026-10-14)
- The view cube and the controls in the approved glass style: rounded faces, capsules, corner circles, names in the faces' planes; one glass bar and a section plate with switches, choices and Blender-like number fields that take expressions with units (2026-10-16)
- The controls over the viewport rearranged as the developer chose from three mockups: the cube top right over the camera controls, a display bar at the bottom with the section's options inline only while it is on, the triad bottom left, one size system; a status bar whose items elide and drop in a fixed order (2026-10-18)
- Tabs of the open documents: switch with a click, Ctrl+Tab and Ctrl+Shift+Tab, close with `file.close` after Save / Don't Save / Cancel, Ctrl+W; the open documents in the File menu; `ui.get` › `tabs` and `closing` ([`development.md`](development.md) › *Tabs of the open documents*) (2026-10-14)
- Every open document's scene kept within a memory budget, a quick first picture refined beside the engine thread (2026-10-14)
- An angle criterion and a triangle budget inside a face in the tessellator, a body coarsened to its budget; the display quality at 2e-4 of the size and 12 degrees, a fifth to a sixth of the triangles (2026-10-14)
- The angle criterion on thin fillets costs what it needs: tori and spheres refined in the plane of their grid, the grid's diagonals held to the angle; the turbofan's display mesh 129,532 triangles instead of 579,314 (2.0 times its mesh without the angle, was 9.1) ([`design/tessellator.md`](design/tessellator.md), *The angle on a thin fillet*) (2026-10-15)
- Every solid of the showcase, the turbofan and the engine block meshes closed at the display quality and at export tolerances (`display_quality.rs` strict): the hull's bow got a wall, a seam no longer runs along a straight edge at the wrap of a cylinder (2026-10-15)
- Long straight edges of flat faces are cut where holes run along them, so a plate with a row of holes is no longer fans of slivers across its length: the self-intersection check of a 4 m plate with 400 holes 13 times faster, export meshes unchanged but for 27 of 440 reference meshes (two triangles more each) ([`design/tessellator.md`](design/tessellator.md), *Long straight chords of flat faces*) (2026-10-15)
- The angle criterion on spline faces in a plane and on a grid graded to where the surface turns: the turbofan's spline faces 30,720 triangles instead of 35,302, a twisted blade within 1.4 times a grid that meets both criteria unrefined (was 1.9) ([`design/tessellator.md`](design/tessellator.md), *The angle on a spline face*) (2026-10-15)
- A spline surface's pole is shaded with its limit normal, and `view.render`, `view.pick` and `view.replay` mesh at a quarter to half a pixel of the picture within a total triangle budget, never finer than the display's floor, coarsening a body too fine for the tessellator instead of refusing ([`design/tessellator.md`](design/tessellator.md)) (2026-10-15)
- `view.replay` and `craft3d-cli replay`: a showcase video (frames, MP4, GIF) of how a document is built, one fixed camera, an optional orbit, captions as subtitles; the document is not touched (2026-10-14)

#### Open
- [ ] A presentation path when no GPU adapter is found (eframe still needs one for the window)
- [ ] Golden images blessed by the developer
- [ ] An automated test with a real MCP client
- [ ] `view.render`: orthographic views and a look-at box (it has only perspective views without zoom; from the field test)
- [ ] The readback path is slower than the CPU for light models on this laptop's Intel driver ([`decisions.md`](decisions.md), waiting for the developer)

#### Acceptance
CPU goldens blessed by the developer; GPU against CPU parity within tolerance on lavapipe; the developer watches an agent build a primitive scene live; the app starts without a GPU.

### M5: Sketches and constraint solver

#### Goal
Sketch entities and constraints as commands; `solve` (Newton-type, DOF analysis, redundancy and conflict diagnosis, stored solution); expressions in dimensions.

#### Done
- Sketches, constraints and our own solver, hardened so that solving twice gives the same verdict (2026-10-12)
- Deviations: profile detection moved to M6 by the developer; sketches lay on world planes with an offset until datum planes (M8.5, 2026-10-13); splines moved to M9 (2026-10-12)
- A terse result mode for batches (`terse`), and `sketch.path`: a chain of lines and arcs through shared corners in one undo step, from coordinates alone (2026-10-13)
- A pattern inside a sketch whose count is a formula (`sketch.pattern`: rows, grids, chained runs and rings as rigid images of the solved seed, image points `e5[last]` for the solver, names `e7[3]` and caps `e7[*]`, format 18; [`design/document.md`](design/document.md), *Patterns inside a sketch*) (2026-10-16)
- The offset of a sketch's closed profiles (`feature.offsetProfile`: exact for lines and arcs, round or sharp corners, splits reported and vanishing loops refused with the largest distance that fits, names `F9:offset(e7[3])`, format 19; kerf as one feature; [`design/document.md`](design/document.md), *Offsets of profiles*) (2026-10-16)
- One result whatever the edits before it: image points' places stored with the solution and a sketch carried along by continuation when an edit moves them (a line after a run's end no longer flips when the count jumps; format 20; [`design/document.md`](design/document.md), *Carrying a sketch along when its images jump*) (2026-10-16)
- A sketch whose carry stops part of the way, or cannot be carried, fails with the reason and stores nothing, so undo or returning the variables gives the start bit for bit (the developer's decision; [`design/document.md`](design/document.md), *Carrying a sketch along when its images jump*) (2026-10-17)
- Profile detection scales: pairs that may meet come from trees of boxes, judged in the order the old all-pairs sweep judged them (same result, oracle kept in the tests); the cap rises from 4096 to 262 144 curves, a 70 × 70 perforated sheet extrudes ([`design/sweeps.md`](design/sweeps.md), *Profile detection at scale*) (2026-10-17)
- The offset of profiles scales the same way: its cut and keep steps search trees of boxes, judged as the old sweeps judged them (same outline, bit for bit; oracle kept in the tests); the cap rises from 4096 to 262 144 pieces and a 70 × 70 sheet offset by its kerf extrudes ([`design/sweeps.md`](design/sweeps.md), *Offset of profiles at scale*) (2026-10-17)
- Nested sketch patterns: a pattern may repeat whole patterns (a tree, three levels at most), names `e3[2][1]` innermost first so wrapping renames nothing, caps `e3[*]` and `e3[*][1]`, carrying through every level; the welding table's tabs per cell is a variable (finding W3, format 21; [`design/document.md`](design/document.md), *Patterns inside a sketch*) (2026-10-17)
- Skipping instances of a sketch pattern on its own level (never instance 0; `last` is the highest instance there is; skips past the count kept and inactive; not yet in chained runs and rings) (2026-10-17)

#### Open
- [ ] Which constraint is blamed in a nonlinear conflict is the solver's heuristic, not a proof, and about 8% of dense random sketches end as "did not converge" instead of a diagnosis; to be judged again on real models
- [ ] A sketch that refers to geometry of other features (projected edges, coincident with a body vertex; from the field test)
- [ ] A picture of a sketch: `view.render` draws bodies only
- [ ] An offset of profiles: splines (refused, "not supported yet"; owed: a fit within tol/4 refused where the curvature radius is below the distance, as `geom::offset` proves for faces), an offset as a loft section or a sweep's path, an offset of an offset, a choice of regions
- [ ] A sketch pattern's later steps: a pattern along a curve, instance curves in line or round slots, image points in `sketch.path` and splines, skips in chained runs and rings, a skip by formula and of one cell's instance (full-path skips), a level inserted into the middle of a tree, a loft section made by nested patterns ([`design/document.md`](design/document.md), *Patterns inside a sketch*)
- [ ] `panic_hunt` takes 2.5 minutes
- [ ] Two seeded breakages of the solver's rank test are caught only by unit tests or the random run, not by a small natural sketch

#### Acceptance
Oracle 6; a catalogue of 20 sketches solves to hand-checked coordinates; changing a variable re-solves without flipping; no sketch makes the solver hang.

### M6: Extrude, revolve, persistent names

#### Goal
Profile detection; `kernel` extrude and revolve from profiles; origin maps; the `PersistentName` scheme and its resolution; incremental `regen` with a per-feature cache; datums on faces; transform, mirror, pattern.

#### Done
- Profiles, extrude and origin names (2026-10-12)
- Persistent names and a sketch on a face (2026-10-12)
- A sketch on a cap keeps its plane when the cap's outline changes; a sketch on a side face is measured from the extruded sketch's origin dropped onto the face (2026-10-13)
- Revolve (`feature.revolve`, Pappus through the command layer) (2026-10-13)
- The per-feature regeneration cache; incremental = full is asserted after every command of the engine suites ([`design/regeneration.md`](design/regeneration.md)) (2026-10-13)
- Transform, mirror and pattern, names through copies, names for the entities of primitives (2026-10-13)
- Regeneration on long rows: a feature whose inputs did not change is not compared input by input, so adding a feature costs the length of the history, not its square; the boolean's check memo keys bodies by their bits instead of their text ([`design/regeneration.md`](design/regeneration.md), *Long rows*) (2026-10-15)
- Deviation: no datum planes; they came with M8.5 (2026-10-13)

#### Open
Nothing.

#### Acceptance
Oracle 1 for extrusions and revolutions (Pappus); incremental = full regeneration; the topological naming suite v1 passes.

### M7: Booleans

#### Goal
Surface/surface intersection, imprint, classify and sew; union, subtract and intersect; cut-extrude and holes; tolerant handling of coincident and tangent cases, or a clear `Unsupported`. The developer's first priority: the [boolean checklist](#the-boolean-checklist) is its measure.

#### Done
- Booleans for planes and cylinders, and the boolean feature (`feature.boolean`) with names through it for faces (2026-10-13)
- Every analytic surface kind (plane, cylinder, cone, sphere, torus) in any position, coincident faces included, merged where a boolean split them needlessly (2026-10-13)
- Cut-extrude (`feature.extrude` with `cut`) and a pattern of cutters (2026-10-13)
- Names of merged faces resolve through any contributor (aliases, `document.inspect` › `faceAliases`) (2026-10-13)
- A self-intersection and shell-overlap detector on the mesh (`tess::body_self_intersections`, `document.check` with `deep`) (2026-10-13)
- Polynomial Bézier and B-spline faces as operands, measured on 1871 corpus pairs (`PatchTree`; [`design/booleans.md`](design/booleans.md)) (2026-10-13)
- Rational freeform faces as operands (`derivative_bounds`, `weighted_normal_net`), held against analytic twins (`crates/kernel/tests/boolean_rational.rs`) (2026-10-13)
- Tangent contact with freeform faces, surfaces that touch along a curve, slivers ten tolerances thick (2026-10-13)
- Contact along a curve in closed form: a cylinder that goes on in a torus, cones along a ruling or sharing an apex (`geom::intersect`) (2026-10-14)
- A torus that goes on in a torus, a rim within the tolerance of an edge (`near/cone-dome/005`), an edge along a cone's ruling (`contact/cone-dome/002`; `near/cone-cone/002` now meets the sliver refusal), a rational face on the analytic surface it equals (2026-10-14)
- Freeform faces with a pole, measured (`topo::mass_properties`) and cut; `feature.revolve` no longer refuses a spline that ends on the axis ([`design/booleans.md`](design/booleans.md)) (2026-10-14)
- Every edge and vertex named after a boolean by its faces (`F7:between[…]`, `F7:corner[…]`) (2026-10-14)
- What is asked again is answered from memory (pure results remembered) (2026-10-14)
- The Voronoi helmet's findings: a lofted tool against a spline shell with a pole, pieces of curve between a freeform face's borders, a pole border that is no curve, a loop through a cut, a bound below the distance that is one; the octant against a torus through its pole at 1e6 ([`design/booleans.md`](design/booleans.md)) (2026-10-15)
- Poles at a corner of a patch's parameters (a lune as the stereographic image of a quadrant): a chart, the side at the pole, the marcher and the tessellator; the eight cuts through a pole the tessellator declined were already meshed (2026-10-15)
- Coincident freeform faces: a patch on its analytic twin moved along itself, two patches of one surface ([`design/booleans.md`](design/booleans.md)) (2026-10-15)
- The self-intersection check inside every operation, near what it changed, and a shell that touches itself refused (2026-10-15)
- A cusp as a boolean operand: the wedge and the disc drawn to the pole's order, the pieces at the point put in order further along ([`design/booleans.md`](design/booleans.md) › *Cusps*) (2026-10-15)
- A cusp against faces that hold its axis (planes, a block edge, a cylinder) cut and measured, not yet meshed: the chart stops where the needle is wide, bounds to the cusp's order, curves drawn on to the point ([`design/booleans.md`](design/booleans.md) › *A cusp against a face that holds its axis*) (2026-10-17)
- The cuts along a cusp's axis and a spline cone's meshed: a parameter free only at the pole, triangles without area cut ([`design/tessellator.md`](design/tessellator.md)) (2026-10-17)
- Rational faces with weights a hundred apart as operands: the winding numbers take the chart's speed near each stretch, held by point membership against the plain block ([`design/booleans.md`](design/booleans.md) › *Rational faces*) (2026-10-15)
- Faces with rational weights far apart measured: inner integrals in the Möbius parameter that evens their weights, boundary panels halved only where they disagree ([`design/topology.md`](design/topology.md) › *Rational weights far apart*) (2026-10-17)
- Rational faces with weights a thousand apart as operands, built, not meshed: the marcher's cells and the winding numbers take bounds of the pieces under them, a step across a knot where the speed jumps, a chart drawn evenly by Möbius maps; held against the analytic twins at three scales and the plain block ([`design/booleans.md`](design/booleans.md) › *A thousand apart in booleans*) (2026-10-17)
- Fourth-order contact: a tacnode or cusp of the intersection is a vertex whose tangent arms are made as graphs around it, a cylinder touching the top of a torus' tube from inside held to its closed-form volume ([`design/geometry.md`](design/geometry.md) › *Tacnodes and cusps*) (2026-10-15)
- A row of booleans on a body of many freeform faces indexes each surface once (the impeller: a chamfer of the bore 12 s to 1 s); a material measures nothing, mass and inertia are measured at once ([`design/booleans.md`](design/booleans.md), [`design/regeneration.md`](design/regeneration.md)) (2026-10-15)
- Zero thickness refused: a result whose parts touch along an edge, a line or in a point (`KernelError::ZeroThickness`, the developer's decision); the trees of freeform surfaces no open document holds let go (2026-10-15)

- Each pair of faces intersected in the box of the two faces (the impeller's blades with rounded leading edges, in a pattern and on a hub); two traces of one branch are one ([`design/booleans.md`](design/booleans.md) › *The box of the two faces*) (2026-10-15)
- Only the part of a chart that can be inside the box is walked; a march step goes as far as it was asked; a face on a torus has a box of its own size (the bike's inset core and mirrored stays, the turbofan's 0.1 mm rounds on discs; [`design/geometry.md`](design/geometry.md), [`design/booleans.md`](design/booleans.md)) (2026-10-15)
- Many lumps and faces with many edges cost work in proportion: the faces a face, a ray or a point can meet and the edges near the other face come from trees of boxes in the order of testing all, bit for bit the same (2500 lumps 5.4 s to 0.9 s; the welding table's finding W2; [`design/booleans.md`](design/booleans.md) › *Many lumps*) (2026-10-17)
- Freeform booleans and their measures 1.1 to 2.3 times faster, bit for bit: projection samples kept with the surface, chart speeds and lower bounds remembered, fine pieces sharing their corners, many bodies measured as one row of faces, checks not repeated by regeneration ([`design/booleans.md`](design/booleans.md) › *Speed of freeform booleans and of their measures*) (2026-10-17)
- Measures and the self-check's meshes faster, bit for bit: lines of a spline evaluated once per line, walks keeping their blend factors, projections and face meshes remembered, every operation's check remembered ([`design/booleans.md`](design/booleans.md) › *Second round*) (2026-10-18)
- Several lumps and cavities proven and checked: shells one by one in `topo::check`, one rule for outsides and cavities (`kernel::lumps`, used by the STEP export and `document.check` deep), `shells` in `document.inspect` ([`design/booleans.md`](design/booleans.md) › *Lumps and cavities*) (2026-10-17)

#### Open
The [boolean checklist](#the-boolean-checklist) › *Owed*.

#### Acceptance
Oracle 3 on generated inputs; the box-minus-cylinder family exact; the naming suite extended through booleans; no invalid body ever returned (proptest).

### M8: STEP export

#### Goal
The `step` Part 21 writer and reader core, typed B-rep entities, the `io` mapping, units and uncertainty.

#### Done
- STEP export (AP214 B-rep, `crates/step` and `crates/io`), which FreeCAD reads back as valid solids of our volume (2026-10-13)
- The Part 21 reader core; typed reading is M10 (2026-10-13)
- Deviation: no typed entity layer in `step`; the mapping builds records in `io` (2026-10-13)

#### Open
Nothing (typed reading belongs to M10).

#### Acceptance
Parse-back equality; an external validator opens every test model and reports matching volume; **the phase 1 acceptance scenario passes**.

### M8.5: A history that survives edits, datums, pointing

#### Goal
What M9 needs before it starts. A feature that cannot be built is marked failed with the reason and the reference it lost, the features that do not depend on it are still built, and it is repaired by pointing its reference elsewhere; reference geometry; extrude ends; pointing at faces, edges and vertices between the developer and an agent, and a section plane.

#### Done
- Reference geometry: datum planes (at a distance, at an angle, through three points), axes (two points, along an edge, of a cylinder, cone or torus face, where two planes meet) and points (coordinates, a vertex, the centre of a circular edge): `datum.plane`, `datum.axis`, `datum.point`, as the plane of a sketch and a mirror and the axis of a revolve and a circular pattern (2026-10-13)
- Extrude ends: symmetric, through all, up to a face on a plane, cylinder, sphere, torus or polynomial freeform surface (2026-10-13)
- Pointing at faces: a click, `ui.pick` and the command `view.pick` give the face's persistent name, the body and the point; `ui.highlight` shows a face by its name; an MCP client reaches `ui.*` through the tool `control_call` ([`control-protocol.md`](control-protocol.md), *Pointing*) (2026-10-13)
- Pointing at edges: the edge near the pointer as `document.edges` lists it; `ui.highlight {edge | between}` (2026-10-13)
- Pointing at vertices: as `document.edges` lists them under `vertices`; `ui.highlight {vertex | corner}` (2026-10-13)
- The section plane: `ui.set {"section": {…}}` (`state.section`), the controls beside the view cube and `view.render {"section": {…}}`; `view.pick` takes it too; the two renderers agree under the tolerances of [`design/rendering.md`](design/rendering.md) and the cap has the area of the cross-section (2026-10-14)
- Failed features: every reference of every feature kind fails its feature with the parameter, the feature and the name it lost; what is made from it fails naming it and carries `causedBy`; repair by `feature.edit`, `sketch.edit` or a datum's `replace`; `feature.move` takes any order and fails what is out of order; roll back gives the document that ends at the marker (2026-10-14)
- A feature added while rolled back goes in at the marker, which moves onto it (2026-10-14)
- The window's feature tree shows failed, suppressed and rolled-back features, the reason on hover and the cause as a link that selects it (2026-10-14)
- Every edge and vertex of every body has a name of its own, so a datum at a vertex of a body a boolean or a joined pattern made works ([`design/names.md`](design/names.md)) (2026-10-14)
- Datums drawn in the viewport in both renderers, hidden with `ui.set {"show": {"datums": false}}` (2026-10-14)
- Rows of features that each make their body from the one before have no length limit: a copy of a copy is one link per run of instance 0 or of other instances, so 1000 holes cut one after another regenerate; format 15 ([`design/names.md`](design/names.md), *Copies of copies*) (2026-10-14)

#### Open
- [ ] A way that switches between target and tool more than about 30 times in a row still hits the 32-link cap of a name
- [ ] An extrude that ends at a cone face
- [ ] Datums in `view.render`; picking a datum
- [ ] The section plane: hatching, more than one plane, a cap where two bodies overlap

#### Acceptance
A lost reference never stops the features that do not depend on it and never panics; repairing the reference gives the same body as building it that way from the start; reorder and roll back agree with a full regeneration; Oracle 1 for sketches on datum planes and for each extrude end; a click on a rendered face returns the name that `document.inspect` reports for it, checked on generated views.

### M9: Local operations and sweeps

#### Goal
The blends first (chamfer and fillet, the first consumers of edge references), then the face operations, which share one recipe (give a face a new surface, extend the neighbours, intersect again, sew), and the sweeps with the splines their profiles and paths need; threads as the other modellers treat them. Each is a feature in the history, never an edit of a body behind it.

#### Done
- Splines in sketches, first step: `sketch.spline` (a polynomial B-spline on control points that are sketch points, or through given points), format version 6, shaped by constraints on its points, tangent to a neighbour, extruded into a B-spline wall that booleans cut (2026-10-13)
- Splines, second step: a profile with a spline revolved; a spline that closes on itself and a polyline (degree 1) are profiles on their own; further faces named `F2:piece(1)/F2:side(e7)`; a point held on a spline (`pointOn`) (2026-10-14)
- Chamfers (`feature.chamfer`): one distance, two, a distance and an angle; mitres and flat corners; a chain of tangent edges picked as one; the edge held as the two faces it lies between; format version 7 ([`design/blends.md`](design/blends.md) › *Chamfers as built*) (2026-10-14)
- Loft (`feature.loft`): closed profiles on offset or datum planes, ruled or smooth, open or closed into a ring; format version 8 (2026-10-14)
- Fillets with a constant radius (`feature.fillet`): convex and concave, run-outs, a piece of a ball where three meet, chains of tangent edges; one kernel module for both blends (`blend`); format version 9 (§5.3 › *Fillets as built*) (2026-10-14)
- Sweep along a path in a plane (`feature.sweep`): a closed profile, holes and all, along lines, arcs and splines, mitred corners, open or closed; format version 10 (2026-10-14)
- Offset face (`feature.offsetFace`) and shell (`feature.shell`), by a local operation that keeps every entity (`kernel::tweak`), the hollow's faces named `F9:copy(1)/…` after the faces they lie behind; FreeCAD's `makeThickness` agrees; format version 11 (§5.3 › *Face operations as built*) (2026-10-14)
- Delete face (`feature.deleteFace`): holes, bosses, pockets and cavities filled, a single chamfer or fillet collapsed into its sharp edge; format version 12 (2026-10-14)
- Move face (`feature.moveFace`): faces moved by one rigid motion (shift and turn), the neighbours following, what lies between moved faces carried exactly; a sketch on a turned face turns with it (§5.3 › *Face operations as built*) (2026-10-15)
- Thicken (`feature.thicken`): faces of a solid given a thickness outside, inside or on both sides, as a body of its own bounded at its rim by the faces round them (two offsets and the boolean); there are no sheet bodies yet (2026-10-15)
- Draft (`feature.draft`): flat faces and cylinders along the pull direction tilted about a neutral plane (a face, a world or a datum plane) into planes and cones, the neighbours following; a drafted box is a frustum, a rounded box a rounded frustum (2026-10-15)
- References that follow a body: a face of an earlier body (`F1:face(0)`) resolves to its newest form, so no `copy(0)` prefixes; `document.edges` lists `short` names (2026-10-14)
- Helix and thread: `datum.helix`, `feature.sweep` with a helix as its `path`, `feature.thread` (external ISO metric, by designation such as `"M10"` and pitch); format version 13 ([`design/sweeps.md`](design/sweeps.md) › *Helix, and the sweep along it*, *Thread*) (2026-10-14)
- A thread is cosmetic unless `modeled: true`, as in other modellers (2026-10-14)
- A thread's size and pitch follow the model: `size` left out (the size of the face), a length (`"d"`) or a designation; the coarse pitch of the size unless given; `v13_to_v14`; format version 14 (2026-10-14)
- A face that winds along a thread is meshed; every triangulation has a work budget (2026-10-14)
- A smooth loft through slender twisted sections builds (the rotor blade with its tip right after 18 m, also from a circle): a face whose triangles cannot follow a chord of its boundary has that chord cut finer (2026-10-14)
- A sweep whose path is off square to the profile's plane by a lean that moves the profile no more than the tolerance is built, the profile laid square; `sketch.path` warns of an arc end off its circle (the turbofan's F6, still refused at the default tolerance: see Open) (2026-10-15)
- Blends along a straight edge next to a cylinder (the cut edges of a halved rod or tube, the tip of a lens of two arcs), worked out in the plane across the edge; both tip rims of a lens blade, after two surgical changes to the boolean ([`design/blends.md`](design/blends.md) › *Blends next to a cylinder*) (2026-10-14)
- Blends along freeform edges (a straight edge next to a cone, the edge between two lofted faces, a spline wall's edge): a rolling-ball fillet or a ruled chamfer as a spline surface within the tolerance, built in place, each end a corner of three faces or the edge closed (a slanted hole's mouth) (§5.3 › *Blends along freeform edges*) (2026-10-14)
- The turbofan's tube (F6) builds at the default tolerance: `sketch.path` puts a `center` arc's end given within 1 % of the radius onto its circle (`snapped`), and a sweep takes a kink of the path as tangent when turning the profile through it stays within the tolerance (2026-10-15)
- The sweep's kink rule measured against the exact sweep both ways (line→arc, arc→line, arc→arc, arc→spline; round and rectangular profiles; three scales): half the tolerance at the largest kink admitted ([`design/sweeps.md`](design/sweeps.md) › *Kinks*) (2026-10-15)
- Internal threads: `feature.thread` on a hole is a tapped hole, its size from the tap drill, cosmetic or modeled (the nut's groove cut into the wall) (2026-10-15)
- The ISO 261 table complete from M1 to M68: every coarse pitch (M11: 1.5 mm), and the fine pitches named when a size has none (2026-10-15)
- A file opened by a path relative to the working directory keeps it absolute, so the window saves it back to itself under a write root (2026-10-15)
- The engine block's blend findings (E1–E8): the estimate of what fits follows a face's boundary round tangent arcs and across a cylinder's seam, takes a cut line that runs exactly into a corner, and refuses a face that pinches to nothing; a blend ends in the plane of a face it runs into at a slant; equal-distance chamfer chains are one tool; a boolean hit next to an edge's end on the surface is that end (§5.3 › *Blend findings of the engine block*) (2026-10-15)
- Freeform blends along chains of tangent edges, one family of balls with a face per edge, also where a face changes across a smooth edge (the turbofan casing's cut edge through its flange root, the engine block's boss foot E6 in its first form), and freeform with closed-form edges in one feature ([`design/blends.md`](design/blends.md) › *Chains and meets of freeform blends*) (2026-10-15)
- Two freeform fillets that meet at a vertex whose third edge stays sharp, convex or concave (the tip of a lofted blade rounded all round) (§5.3 › *Chains and meets of freeform blends*) (2026-10-15)
- The impeller's and the lamp's blend findings: freeform meets found where the borders cross, blends that run into a wall or end past a corner (edges lengthened, spline faces grown), up to 4096 samples by a banded solve, face changes at joints judged at the vertex (§5.3 › *Blend findings of the impeller and the lamp*) (2026-10-15)
- What fits is decided by building the blend and checking it, not by an estimate; the largest size that fits is found by more builds ([`design/blends.md`](design/blends.md) › *What fits, decided by building*) (2026-10-15)
- Offset of freeform faces: offset face, shell and thicken take Bézier and NURBS faces, offset into splines within a quarter of the tolerance after proving they do not fold ([`design/face-operations.md`](design/face-operations.md) › *Offset of freeform faces*) (2026-10-16)
- The bike's and the turbofan's blend findings: welds of a tube into a tube (a ring's balls in planes square to their centres' path), a refinement that no longer creeps, a border that crosses an edge on a curved face refused, lofts tangent where their sections are ([`design/blends.md`](design/blends.md) › *Blend findings of the bike and the turbofan*) (2026-10-15)
- One blend construction, steps 1 to 3: a straight edge between planes, or next to a cylinder along it, built in place with an exact face; a border carried across a smooth edge of its face, the face it leaves gone when it lies wholly in the blend ([`design/blends.md`](design/blends.md) › *One blend construction*, *Borders carried across a smooth edge*) (2026-10-16)
- Rolling-ball fillets along the rims of faces an offset or a shell made (the bike's saddle shelled first, all four rims): the fitted offsets' normals continuous at their borders and knots ([`design/face-operations.md`](design/face-operations.md) › *Fillets along the rims of offset faces*) (2026-10-16)
- The offset of a fillet face is the fillet rebuilt between the offset faces it rolls along, radius r ± d: the bike's saddle shells with its fillets ([`design/face-operations.md`](design/face-operations.md) › *Offset of a fillet: rebuilt*) (2026-10-18)

#### Open
- [ ] Replace face
- [ ] Draft: cones, spheres and freeform faces, faces not along the pull direction, a parting line instead of a neutral plane, a face whose neighbour is tangent to it and not drafted
- [ ] Thicken: a sheet body (`feature.fill` makes one since 2026-10-18), walls along the faces' normals at a rim that is not square to its neighbours
- [ ] Move face: a cylinder or cone tilted against a flat neighbour (the edge would be an ellipse), a moved face whose neighbours would have to gain or lose an edge
- [ ] Vertex blends: three fillets of different radii, with a circular edge, with a fourth edge
- [ ] Freeform blends that are still refused: a blend whose end runs over more than one face (F9), a joint creased by more than the tolerance allows at the radius (the lamp's cells, B4), a chamfer chain whose face changes at a joint (B5); a border carried across a smooth edge of its face in a chain, a ring or a meet (the engine block's E6 in the full block, next to the waist round), across more than one from an end, or across one that it enters and leaves through the border; concave fillets that meet round a sharp convex edge, or convex ones round a concave edge (a blade's root loop, the turbofan's F3 and F11: the ball rolls over the edge); a weld whose ball runs past the end of a face onto the next (the bike's B2); a chain whose ends meet at one vertex; chamfers whose face changes at a joint or that meet; a freeform chamfer by distance and angle; a vertex patch with tangency (a setback) in place of the crease where two fillets meet
- [ ] A blend that ends in a face of its own in the middle of a chain
- [ ] Chamfers: a reference to one of several edges between the same two faces
- [ ] Fillets: an outside and an inside fillet that meet; a fillet on a circular rim past half its radius (a spindle torus face: R/2 is the kernel's limit, which the hint for what fits reports, not the geometric one); a rim fillet along an earlier round of the same radius or more (the torus would reach its axis: all edges of a prism in two steps); variable radius (later)
- [ ] Blends whose touch line runs over a tangent edge onto the next face (a rib's root that reaches the round of a rail below it), and a blend ending on a face that is not a plane
- [ ] Offset face: a neighbour tangent to the moved face that does not move with it, a vertex of four or more faces that would become an edge, a line or circle between analytic faces that would become another curve; freeform faces with a pole, the band just short of a radius of curvature the bounds cannot prove, tangent faces other than a fillet and its faces whose fitted tangency, times the distance, exceeds a quarter of the tolerance (decided 2026-10-16 in [`decisions.md`](decisions.md): no deviation accepted); a fillet moved without the faces it rolls along (a shell open at one of them); a freeform chamfer's offset is fitted, not rebuilt (it is no chamfer), and stays dear
- [ ] Shell: a thickness at a rounded edge's radius or more
- [ ] Delete face, refused with the faces today: the corner where blends meet, a blend whose end runs into another blend or into a vertex of more than three faces (chains, mitres), several blend faces at once, neighbours that do not meet in a line or a circle
- [ ] Threads as the other modellers treat them (agreed with the developer on 2026-10-14): a cosmetic thread shown in the viewport (a texture or fine rings, both renderers), its data (designation, pitch, class, length) listed by `document.inspect`, carried to drawings and hole tables and checkable by V1's assertions
- [ ] A hole feature (drill, thread, depth in one; the internal thread itself is there)
- [ ] Other thread standards, multi-start threads, a thread ending against a shoulder, a cosmetic thread drawn or written to STEP (AP242 thread PMI later, when we write AP242)
- [ ] Sweep: a path not in one plane (a 3D polyline or spline, rotation-minimising frames), twist and scale along the path, guide curves
- [ ] Loft: guide curves, tangency at the ends, a tip in a point, profiles with holes
- [ ] Splines: the other constraints on the curve itself (parallel, equal, midpoint), rational and non-uniform splines

#### Acceptance
Oracle 1 where closed forms exist (chamfered and filleted box edges, a shelled box, an offset cylinder, a drafted box as a frustum); validity on generated inputs; a radius, distance or offset that is too large, or a neighbour that vanishes, gives an `Err` that names the entity, never an invalid body; the naming suite extended to edges and faces consumed by these features under upstream edits.

### R1: First release

#### Goal
The first version for people outside the project; it follows M9 and does not wait for M10. A pinned set of modelling tasks built by an agent over MCP is the measure of the product, as the boolean corpus is the measure of the kernel.

#### Done
- Autosave and crash recovery in the app and in `craft3d-cli serve`/`mcp` (`recovery.list`, `recovery.open`, `recovery.discard`; `crates/engine/src/recovery.rs`) (2026-10-13)
- DXF and SVG export of a sketch or a flat face, for laser cutting (`file.export` to `.dxf` and `.svg`, `crates/drawing`, [`file-formats.md`](file-formats.md) › *Flat outlines*) (2026-10-13)
- Flat faces exported by followed names, in a documented upright frame that is never mirrored, and several faces laid out on one sheet (`file.export {faces, gap, width, xAxis, origin}`, [`file-formats.md`](file-formats.md) › *Flat outlines*) (2026-10-17)
- A colour and a material per body (`body.appearance`, `body.material`; format 16), following a body through booleans and moves with tool faces keeping their own colour, drawn by both renderers and in 3MF and STEP exports ([`design/document.md`](design/document.md)) (2026-10-15)
- Mass and the inertia tensor about the centroid from the exact B-rep for a body with a density, in `document.inspect` and `document.measure` (2026-10-15)

#### Open
- [ ] A pinned set of about twenty modelling tasks (each with a drawing or description and a known volume), grown out of the field tests
- [ ] Installers for Linux, macOS and Windows
- [ ] A promise that a `.3dcraft` file of this version stays readable, with a migration test per format version
- [ ] Colours in OBJ (`.mtl`), metallic and roughness in the viewport's shading, and an independent reader that checks the colours of 3MF and STEP files (FreeCAD's command-line import keeps none)
- [ ] A user guide and example models
- [ ] A clean quit also drops the autosave of a document that was not saved, because the window does not ask before closing yet
- [ ] What the first field test did not touch: history commands, conflicting sketches, the other constraint kinds, negative distances, other units

#### Acceptance
Every task of the set is built by an agent over MCP and `document.inspect` reports its known volume; a file saved by this version loads in the next; a killed app recovers the document on restart (✅ 2026-10-13: `apps/craft3d/tests/killed_window.rs` and `apps/craft3d-cli/tests/recovery.rs`, both with SIGKILL); FreeCAD or another independent reader opens the DXF and SVG exports with the same outline.

### V1: Design verification

#### Goal
An agent checks a design by asking it questions, not by looking at a picture: the design intent written down as assertions checked at every regeneration, the way the kernel is tested by oracles. After R1 (decided by the developer on 2026-10-14). The agent writes the assertions first and builds the model to meet them; the developer reads the assertions instead of measuring every detail.

#### Done
- Interference between bodies (`document.interference`): clear, touch (on a face, an edge or a point) or overlap with its volume, read-only, boxes first ([`design/booleans.md`](design/booleans.md) › *Interference*) (2026-10-16)

#### Open
- [ ] Queries: the distance and the least clearance between two faces or bodies; a section as geometry with its measures (area, outline, the widths across it); a ray cast through the model (the faces it meets and where); symmetry about a plane; wall thickness (the thinnest place and where); the least inside radius
- [ ] `document.assert`: an assertion kept in the document (a measure against a value with a tolerance, a clearance at least, no interference, symmetric about, thickness at least), checked after every regeneration; a broken one shows like a failed feature, with the measure it found, in `document.check`, the tree and over MCP

#### Acceptance
Each query against closed forms at three scales; the morning's models (the aircraft, the M10 screw, the villa) carry a set of assertions that a deliberate error breaks.

### M10: STEP import, corpus, web

#### Goal
STEP files read into the document as bodies (an `Import` feature), a pinned public STEP corpus, `craft3d-web`, performance budgets and a generated scorecard.

#### Done
- The tessellator meshes faces on spline surfaces that close on themselves and closed surfaces of every kind with their seams as edges, also from pole to pole or to an apex (`crates/tess/tests/closed_freeform.rs`, `periodic.rs` › `closed_faces_with_their_seams_as_edges`) (2026-10-14)
- The exact measure follows such faces, closed in u, in v, both ways and to a point (before, it gave a wrong volume without an error for a face that closes in v) (2026-10-14)
- The booleans cut such faces, across the seam, along it, through the poles and around the axis (`crates/kernel/tests/boolean_closed_twins.rs`, `boolean_closed_freeform.rs`; [`design/booleans.md`](design/booleans.md) › *Freeform faces that close on themselves*) (2026-10-14)

#### Open
- [ ] Share large blobs between undo snapshots instead of copying them, before imported bodies enter the document (undo keeps a whole document per step, [`decisions.md`](decisions.md))
- [ ] The tessellator on what such booleans leave (a closed spline face that is everything outside its loops, a ball's face cut along its seam: 77 of 594 results declined, listed by that test)
- [ ] The measure of a boundary through both poles of a spline surface that also crosses its seam (declined by `topo`)
- [ ] The tessellator on an egg without loops that closes in v and on a NURBS torus around a window across both seams (ignored reproducer `closed_freeform.rs` › `what_the_tessellator_declines_on_closed_freeform`)
- [ ] A period for spline surfaces in `craft3d-geom`, so that knots that are not clamped are covered (`geom::closure` reads clamped knots only)
- [ ] The STEP reader to bodies (`Import` feature) and typed reading
- [ ] A pinned public STEP corpus
- [ ] `craft3d-web`
- [ ] Performance budgets and a generated scorecard

#### Acceptance
A corpus import floor set and enforced; the web build loads and views a model.

### M11+: Interactive UI

#### Goal
Sketching and feature editing with the mouse, menus, selection, shortcuts and measurement tools; later assemblies and drawings.

#### Done
Nothing yet (the camera, the tabs, pointing and the section plane are M4 and M8.5).

#### Open
- [ ] Defined when phase 1 is done

#### Acceptance
Defined when phase 1 is done; depends on whether a target app is chosen.

## Ideas (M12+)

Not planned in detail; listed so they are not forgotten. Each is decided when a feature needs it.

- **Surfacing: sheets and surface features** (agreed with the developer on 2026-10-14, after R1; it widens 3DCraft from mechanical CAD towards design and art: jewellery, objects, sculpture, architecture): open sheets as bodies of their own (trim, extend, sew into a solid, thicken; thicken in M9 needs them anyway); surfaces from curves as parametric features (network, boundary, blend between two edges with G1 or G2 continuity); continuity checks an agent can read (G0/G1/G2 along an edge, curvature and zebra analysis) as oracles. Not as the basis: dragging control points without a history (it breaks rule 2); if it comes, as an explicit direct-edit feature that fails clearly when what it rests on changes. SubD is a separate, larger question.
- **Fitting a mesh into a B-rep** (an import, so part of 3DCraft by the rule of 2026-10-14): segment a mesh's triangles by curvature, fit each region with a plane, cylinder, cone, sphere, torus or else a NURBS surface, and close the body by intersecting the fitted surfaces with our own kernel (clean-room, from the reverse-engineering literature).
- **Undo as reversible operations** instead of snapshots: needed only if edits must be merged (two users at once, or a background job that finishes after the document moved on), or to show exactly what a step changed. It touches the `ops` crate only: commands use `History::apply`/`undo`/`redo` and do not know how it works inside.

## Projects on 3DCraft (separate repositories)

Applications built on 3DCraft, not part of it (the rule in `docs/decisions.md`, 2026-10-14: *does it work on any shape, or does it make one kind of product?*). They use 3DCraft through its commands, MCP or the engine crate; what they lack is added to 3DCraft in its general form.

- **Generators: parametric apps that run without AI** (an idea of the developer, 2026-10-14; maybe a milestone later): an agent builds a parametric model once (a toothed-belt pulley, a gear, a laser-cut box), proves it over its whole parameter range with V1's assertions, and it is deployed as an app with a form of public parameters (unit, range, step, a choice from a table), the viewport and export buttons, native or in the browser (the wasm build), with no AI at run time. Needs: public parameters on variables; curves from formulas (an involute, an airfoil), counts and conditions from variables, lookup tables (belt profiles GT2, HTD, T5; ISO modules); speed per slider step. Pilot: the toothed-belt pulley, against the developer's earlier replicad version.
- **Reverse engineering by an agent** (an idea of the developer, 2026-10-14): from a B-rep fitted to a scan (the import above), an agent recognises holes, patterns, shells, extrusions and revolutions, rebuilds the model with commands and variables for the measures that matter, and checks it against the scan (volume, the largest distance between the surfaces, each face against its fitted surface: V1's assertions). External generators of STEP files (research models such as BrepGen, GPL-3.0 and built on OpenCascade, so only as separate tools) can feed STEP import and the boolean corpus.

## Risks, in order

1. **Booleans (M7)** are where CAD kernels fail. Mitigation: analytic pairs first, strict "valid or `Err`", property tests from day one, no feature may depend on a boolean case that is not covered by tests.
2. **Persistent naming (M6)** is a design problem, not an implementation one. Mitigation: prototype with the naming suite before M7 builds on it; record the scheme in `architecture.md` (now [`design/names.md`](design/names.md)).
3. **Numerical robustness** everywhere. Mitigation: tolerance as data, guarded floating point (never-crash rules), scale tests, the predicates decision in [`decisions.md`](decisions.md).
4. **STEP acceptance by other systems (M8).** Mitigation: the external validator in the loop as early as M8 starts.
5. **Scope.** A kernel is years of work for a human team. Mitigation: phase 1 is deliberately narrow (prismatic parts with holes), and every milestone ends with something the developer can see and an agent can verify.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-18 | minor | Boolean checklist item 5: sheet bodies (`feature.fill`, trim and cut by subtract and intersect); format version 22 |
| 2026-10-16 | minor | M1: `floor`, `ceil`, `round`, `trunc` and `mod` in expressions; format version 17 renames old variables with those names |
| 2026-10-16 | minor | M9: fillets along the rims of offset and shelled freeform faces |
| 2026-10-16 | minor | M9: offset face, shell and thicken of freeform faces |
| 2026-10-15 | minor | M9: move face, thicken and draft done (`feature.moveFace`, `feature.thicken`, `feature.draft`); new feature kinds in format version 16 |
| 2026-10-14 | minor | Restructured: current focus on top, one line per milestone, a section per milestone (Goal, Done, Open, Acceptance), the boolean checklist on its own, ideas apart; how things were built left to `architecture.md` |
| 2026-10-15 | minor | M9: the engine block's blend findings E1–E8 (estimate of what fits, blends ending on a slanted face, chamfer chains joined, a boolean hit snapped to an edge's end) |
| 2026-10-15 | minor | M4: the angle criterion on thin fillets without slivers (turbofan 9.1 to 2.0 times the triangles of the plain mesh); every showcase solid meshes closed (the hull's bow, a seam along a line at the wrap) |
| 2026-10-15 | minor | M9: a sweep's right-angle test uses the document's tolerance; `sketch.path` warns of an arc end off its circle |
| 2026-10-15 | minor | M9: `sketch.path` snaps a `center` arc's end onto its circle; a sweep's tangency test at a kink uses the tolerance (F6 builds) |
| 2026-10-15 | minor | M9: freeform blends along chains of tangent edges and with closed-form edges in one feature; two freeform fillets that meet where a third edge stays sharp |
| 2026-10-14 | minor | M9: a thread is cosmetic unless `modeled`, as in other modellers |
| 2026-10-14 | minor | M9: a thread's size and pitch follow the model (the size of the face, a designation or a length; the coarse pitch of the size unless given); format version 14 |
| 2026-10-14 | minor | M9: helix and thread done (`datum.helix`, the sweep along a helix, `feature.thread`); format version 13; the showcase has a coil spring and a socket head cap screw with its thread |
| 2026-10-14 | minor | M9: references that follow a body: a blend's or a face operation's faces given by the names of an earlier body resolve in the newest body of its chain |
| 2026-10-14 | minor | M9: delete face done (`feature.deleteFace`); format version 12; a sketch on a face of a subtraction's tool (a pocket's floor) looks out of the material |
| 2026-10-14 | minor | M9: offset face and shell done (`feature.offsetFace`, `feature.shell`), the first face operations; format version 11; the showcase has a shelled tray |
| 2026-10-14 | minor | M9: sweep along a path (`feature.sweep`, format version 10), the last of the sweeps; the showcase has a bent pipe with a flange, a handle on a plate, a gasket and a hose |
| 2026-10-14 | minor | Booleans: a torus that goes on in a torus (closed form); a touch that dips through by less than half the tolerance, a circle about a cone's axis and an edge along a cone's ruling; a million marcher cells; a rational face on the analytic surface it equals; corpus 11573 of 11988, 249 refused |
| 2026-10-14 | minor | Booleans: a cylinder that goes on in a torus and cones along a ruling are built (closed forms in `geom::intersect`); an arm of a near-crossing is no longer drawn twice; corpus 11564 of 11988, 264 refused |
| 2026-10-14 | minor | M9: fillets with a constant radius done (the second blend, on the chamfer's kernel module); format version 9 |
| 2026-10-14 | minor | M9: loft (`feature.loft`, format version 8), the first of the sweeps; the showcase has a lofted hull and a twisted blade |
| 2026-10-14 | minor | M9 started: chamfers done (the first blend and the first consumer of edge references); format version 7 |
| 2026-10-14 | minor | M8.5: failed features done (uniform reasons, `causedBy`, repair by `feature.edit` for sketch, source, target, tool and revolve axis, reorder and roll back held to a full regeneration in every position, a random walk of breaking and repairing) |
| 2026-10-14 | minor | M8.5: the section plane done (both renderers, `view.render`, `view.pick`, the window); pointing is complete |
| 2026-10-14 | minor | Splines in sketches, the second step: revolved, closed on themselves, with corners; a point on a spline |
| 2026-10-13 | minor | Splines in sketches, the first step of M9's sweeps: drawn, constrained by their points, extruded; loft and sweep are not started |
| 2026-10-13 | minor | The viewport has a see-through slider (the developer's request): done in the CPU renderer and the shell |
| 2026-10-14 | minor | The viewport has a view cube in place of the orbit gizmo (the developer's request): faces, edge and corner facets as views, the projection on its ring |
| 2026-10-16 | minor | The viewport's controls in three groups, the cube in patches with an axis triad, a Persp/Ortho toggle and zoom at the pointer (the developer's request) |
| 2026-10-16 | minor | The view cube and the controls restyled as the developer approved: glass cube of rounded faces, capsules and corner circles, a glass bar and section plate, number fields that take expressions (the developer's request) |
| 2026-10-14 | minor | See-through bodies are drawn by the wgpu renderer (two depth layers and a blend, both the readback renderer and the viewport's, within the tolerances of [`design/rendering.md`](design/rendering.md)): the window no longer hands those frames to the CPU |
| 2026-10-13 | minor | The viewport has an orbit gizmo instead of the view buttons (the developer's request) |
| 2026-10-13 | minor | M8.5: pointing at vertices done; the section plane open |
| 2026-10-13 | minor | M8.5: pointing at edges done; vertices and the section plane open |
| 2026-10-13 | minor | M8.5: pointing at faces done (pick, highlight, over the control channel and MCP); edges, vertices and the section plane open |
| 2026-10-13 | minor | M8.5: reference geometry done (datum planes, axes and points as features; a sketch and a mirror on a datum plane, a revolve and a circular pattern about a datum axis) |
| 2026-10-13 | minor | M8.5 added (failed features, datums, extrude ends, pointing in the viewport) and R1, the first release (modelling task set, installers, format promise, material and colour, DXF/SVG); splines moved into M9 |
| 2026-10-13 | minor | Freeform (Bézier and B-spline) faces as boolean operands, measured on 1871 corpus pairs; the viewport renders on the GPU; names through merged faces; the gate in under four minutes |
| 2026-10-13 | minor | M9 spelled out: chamfer and fillet first as must-haves, then offset, move/delete/replace face, shell and draft, with their acceptance |
| 2026-10-13 | minor | M6 done; M7 started (booleans for planes and cylinders, the boolean feature); the boolean kernel is the first priority, with a checklist |
| 2026-10-13 | minor | Boolean kernel generalised to every analytic surface kind; corpus numbers and the order of what is next |
| 2026-10-12 | minor | First field test over MCP recorded; its findings added to the current focus |
| 2026-10-12 | minor | M4 first slice merged (viewer, control server, MCP bridge); M6 second slice (persistent names, sketch on a face) |
| 2026-10-12 | minor | M6 first slice (profiles, extrude, origin names); M4 first slice started in parallel |
| 2026-10-12 | minor | M5 done (sketches, constraints, own solver, hardened so that solving twice gives the same verdict); profile detection moved to M6; current focus is extrude |
| 2026-10-12 | minor | M3 done; current focus is sketches (M5) and extrude (M6) |
| 2026-10-12 | minor | M2 done, M3 partly; current focus rewritten |
| 2026-10-12 | minor | Undo follow-ups recorded: shared blobs before M10, reversible operations as an M12+ candidate |
| 2026-10-12 | minor | M1 done; current focus is M2 |
| 2026-10-12 | minor | T1 mesh export done and validated with FreeCAD |
| 2026-10-12 | minor | M0 done: all crate stubs, full layer table, lint opt-in check, `xtask stats`, pinned toolchain, CI on GitHub |
| 2026-10-12 | minor | T0 teapot track done; M0 in progress; current focus rewritten after the developer's decisions (MIT, AI-first, FreeCAD as oracle) |
| 2026-10-12 | major | First version, from `BRAINSTORM.md` |
| 2026-10-14 | minor | V1, design verification (queries and `document.assert`), added after R1; generators (parametric apps without AI) listed as an idea under M12+; document tabs, helix and thread, references that follow a body (the developer's requests of the day) |
