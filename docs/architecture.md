# 3DCraft architecture

> **The map.** What the parts are, how they fit, and the rules that always hold. It changes only when the structure changes; a finding, a fix or a number goes into the detail document of its part under [`design/`](design/README.md) (§10). **Last updated:** 2026-10-15 (this map replaces the single file that mixed the plan with the history of every finding; the original plan of 2026-10-12 is kept as written in [`design/plan.md`](design/plan.md)).

## 0. In one paragraph

3DCraft is a parametric CAD modeller with its own geometry kernel. A **document** is a recipe: variables, sketches and an ordered list of features. **Regeneration** turns the recipe into **bodies** (exact boundary representations: faces on surfaces, edges on curves). The **tessellator** turns bodies into triangle **meshes** for the screen and for export. Everything a user or an agent can do is a **command**, and every client (window, CLI, control channel, MCP) runs the same commands on the same engine.

```text
 command ──► Document (recipe) ──► regen ──► Body (exact B-rep) ──► tess ──► Mesh ──► render / export
              history, variables,     per feature,      checked, named          display or export
              sketches, appearance    cached            faces/edges/vertices    quality
```

## 1. The rules that always hold

These are the invariants. Every part is built so that they hold by construction, and tests check them. A change that would break one is a decision for the developer, recorded in [`decisions.md`](decisions.md).

| # | Rule | What it means in practice |
|---|---|---|
| 1 | **The history is the truth** | Bodies and meshes are derived and may be thrown away at any time. Nothing is stored that cannot be recomputed. |
| 2 | **Never a wrong body** | An operation returns a valid body or an error that names what it could not do. A refusal is work owed; a wrong result is a bug. |
| 3 | **Never a crash** | Bad input of any kind gives an error, never a panic. |
| 4 | **Deterministic** | The same document gives bit-identical bodies and meshes on any machine and any number of threads. |
| 5 | **No built-in sizes** | Tolerance and units are data of the document. No algorithm has a length of its own; a model ten times larger gives the same result ten times larger. |
| 6 | **Stable names** | Every face, edge and vertex has a persistent name derived from how it was made. Features refer to names, never to numbers, so an upstream edit never silently retargets a downstream feature. |
| 7 | **Exact where possible** | Planes, cylinders, cones, spheres and tori stay what they are; splines (NURBS) are first-class from the start. No code assumes "all faces are flat". |
| 8 | **General methods, not special cases** | A fix must work for any geometry by construction. If only a rule for one configuration is possible, the case stays refused and open, with the general method named. *(The developer, 2026-10-15.)* |
| 9 | **One oracle per question** | The CPU renderer is the reference for pictures; closed forms, the boolean corpus and FreeCAD are the references for geometry ([`testing.md`](testing.md)). |
| 10 | **Layers point down** | A crate uses only crates below it (§2). The core never knows about the window or a UI toolkit, and compiles to WebAssembly. |

## 2. The parts and their layers

```text
 L8  apps          craft3d (window), craft3d-cli, craft3d-fuzz
 L7  clients       ui-egui (the shell), automation (control server, MCP)
 L6  engine        Session: open documents, commands, undo, scenes
 L5  io            bodies and meshes <-> STEP, STL, 3MF, OBJ, DXF, SVG
 L4  derivation    regen (history -> bodies), render (CPU oracle), gpu (wgpu), format (.3dcraft)
 L3  algorithms    kernel (operations on bodies), tess (meshes), solve (sketch solver), ops (undo history)
 L2  document      doc (pure data: the recipe)
 L1  model data    topo (B-rep, checks, measures), expr (units, formulas)
 L0  foundation    math, geom (curves, surfaces, intersections); step, meshio, drawing, zipstore (standalone formats)
```

`cargo xtask layers` enforces this from `xtask/src/layers.rs`: a crate depends only on lower layers, except the same-layer pairs listed there (`geom` → `math`; `gpu` → `render`, whose camera and frame it shares so the two compare pixel by pixel; `kernel`'s tests mesh with `tess`); the standalone format crates depend on no workspace crate (except `meshio` → `zipstore`, for 3MF); nothing below L7 uses egui, eframe, winit or rfd; `testkit` is a dev-dependency only; every crate opts into the workspace lints. L0 to L7 build for `wasm32-unknown-unknown` (`cargo xtask wasm`).

| Crate | Layer | Owns | Never |
|---|---|---|---|
| `math` | L0 | `f64` vectors, boxes, frames, transforms, `Tolerance` as data, quadrature, the exact bits of values (`Bits`) | knows a curve or a surface |
| `geom` | L0 | the `Curve3` and `Surface` enums (analytic kinds and NURBS): evaluation, projection, poles, closure, knot refinement, every surface/surface and curve/surface intersection | knows topology |
| `step` | L0 | ISO 10303-21 reader and writer with its own record model | knows CAD entities (that is `io`'s) |
| `meshio` | L0 | STL, 3MF and OBJ writers with their own mesh model | depends on a workspace crate but `zipstore` |
| `drawing` | L0 | DXF and SVG writers with their own 2D model | depends on a workspace crate |
| `zipstore` | L0 | a stored (uncompressed) ZIP container with a hardened reader | depends on a workspace crate |
| `topo` | L1 | the B-rep (`Body`), `check`, genus, rigid transforms, boxes, mass properties and inertia by boundary integrals | builds geometry by a modelling operation |
| `expr` | L1 | dimensions and units, the formula parser and evaluator, `VariableTable` | knows features |
| `doc` | L2 | the document as pure data: variables, features (sketches, datums, solids, operations), persistent names as data, appearance and material | runs a geometry algorithm |
| `ops` | L3 | the undo history, as `Arc<Document>` snapshots | knows bodies |
| `solve` | L3 | the 2D constraint solver, on plain numbers | knows the document |
| `kernel` | L3 | every modelling operation on bodies (§5): primitives, profiles, sweeps, booleans, blends, face operations; an origin for every entity it makes | meshes, reads the document, or registers a command |
| `tess` | L3 | meshes of bodies and edges, the triangle budgets, the mesh self-intersection and self-touch checks | knows the kernel's operations |
| `regen` | L4 | history → bodies: expressions, references and names, the cache, statuses and reasons, the self-intersection check per feature | changes the document (the engine writes solved sketches back) |
| `render` | L4 | the CPU reference renderer (the display oracle), picking, highlight, see-through, section | is used by `kernel` or `tess` |
| `gpu` | L4 | the wgpu renderer of the same pictures, offscreen and on the window's device | is the reference |
| `format` | L4 | `.3dcraft` read and write, versions and migrations, an exhaustive conversion of every `doc` field | saves a derived body |
| `io` | L5 | the mapping of bodies, meshes and outlines onto `step`, `meshio` and `drawing` | holds a format's syntax |
| `engine` | L6 | `Session`: documents, the command registry, undo, scenes, autosave and recovery, file roots | knows a UI toolkit or a GPU |
| `ui-egui` | L7 | the shell: viewport, feature tree, view cube, its state (`state.rs`, serde), theme tokens | edits a document except through a command |
| `automation` | L7 | the line-based control server and the MCP server over one `Session` | adds behaviour of its own |
| `testkit` | dev | reference solids, mesh checks, image diff, fixtures | is a normal dependency |
| `apps/craft3d` | L8 | the window: eframe, the engine thread, the control listener, file dialogs | holds modelling logic |
| `apps/craft3d-cli` | L8 | the headless CLI: `run`, `info`, `render`, `convert`, `replay`, `commands`, `serve`, `mcp` | holds modelling logic |
| `apps/craft3d-fuzz` | L8 | random multi-step designs held to the oracles, one seed or reproducer per process ([`testing.md`](testing.md) › Oracle 11) | holds modelling logic; is a dependency of anything |
| `xtask` | — | `layers`, `wasm`, `ci`, `stats`, `health`, `freecad`, `boolean-corpus`, `fuzz` | is shipped |

## 3. The data

- **Document** (`doc`): units, tolerance, variables, the ordered features (sketches, datums, primitives, extrusions, revolutions, lofts, sweeps, booleans, transforms, mirrors, patterns, blends, face operations, threads), a rollback marker, and beside the history the bodies' appearance and material (`Document::bodies`). Every numeric parameter is an expression kept as source text. Pure data; no geometry code. Details: [`design/document.md`](design/document.md).
- **Body** (`topo`): vertices, edges, loops of coedges, faces, shells; each face on a surface, each edge on a curve. Parameter-space curves are not stored (they are recovered by projection), and entities carry no tolerance of their own yet. `topo::check` decides validity; `topo` also measures (area, volume, centroid, inertia) by integrating over the exact geometry. Details: [`design/topology.md`](design/topology.md).
- **Origins and names**: every kernel operation reports, for every entity of its result, where it came from. `regen` turns that into persistent names: by role and source for what a feature makes (`F2:side(e7)`), as a copy for what it keeps (`F9:copy(0)/F1:face(1)`), by the faces around them for edges and vertices after a boolean (`F7:between[…]`). Details: [`design/names.md`](design/names.md).
- **Regeneration result**: per feature a status (`Ok`, `Suppressed`, `RolledBack`, `Failed(reason)`), its body, datum or solved sketch, its names, and which later feature took its body.
- **Mesh and scene**: triangles per face and polylines per edge, with the face's and the edge's name, at a quality the caller chooses.

## 4. The pipelines

**An edit.** A command changes a copy of the document; if it succeeds and changes something it becomes one undo step (`ops::History`). Then regeneration runs, and the engine writes solved sketches back.

**Regeneration** (`regen`), per feature in order:
1. evaluate the feature's expressions (the variables first, in a deterministic order);
2. resolve its references (persistent names, followed to the newest body where the feature takes that body) against the results so far;
3. reuse the cached result if nothing it depends on changed, otherwise call the kernel;
4. check the result (`topo::check`, and for every operation that can make such a body, the self-intersection and self-touch check near what changed or over the whole body);
5. record a status: ok, or failed with a reason that names the parameter, the feature and the cause; a feature made from a failed one fails naming it (`causedBy`).

A failure does not stop the model: everything before it stays usable. Details: [`design/regeneration.md`](design/regeneration.md).

**Showing and exporting.** The engine asks the tessellator for a scene per document revision: a quick first picture (1e-3 of the diagonal of the bodies' box), then the display quality (2e-4 of the model's size and an angle of 12°) within a million triangles, built beside the engine thread. A picture (`view.render`, `view.pick`, `view.replay`) has its own tolerance, from the size of a pixel for the camera it is taken with. Export uses the tolerance asked for or the automatic one (1e-3 of the model's size), and refuses rather than coarsens. Details: [`design/tessellator.md`](design/tessellator.md), [`design/engine.md`](design/engine.md).

## 5. The kernel: its methods

The kernel is a small number of general methods, each a pure function: bodies and parameters in; a body with an origin for every entity, or an error naming the entities and the reason, out. Each is one paragraph here; how it works and what it still refuses is in its detail document. What every operation promises: [`design/kernel.md`](design/kernel.md).

1. **Geometry and intersections** (`geom`). Every pair of surface kinds can be intersected (`meet_surfaces`): in closed form where one exists, otherwise by a marcher that finds every branch with a quadtree certified by derivative bounds and follows it within the tolerance. Hard places (poles, crossings, tangency, tacnodes and cusps) are found and handled explicitly, or refused with the reason. [`design/geometry.md`](design/geometry.md)
2. **Booleans** (`kernel::boolean`). Section: intersect every pair of faces that can meet and cut the curves at the edges. Imprint and regions: split the faces along the curves. Classify: one point per region, by a winding number in a chart of its surface and by ray casting against the other body. Select, sew and merge into a manifold body, checked. Held to a corpus of 3996 pairs (11,988 outcomes) against FreeCAD's answers: none wrong, none invalid, 249 refused, all among the cases built within a few tolerances of contact. [`design/booleans.md`](design/booleans.md)
3. **Blends: fillets and chamfers** (`kernel::blend`, with the front ends `kernel::chamfer` and `kernel::fillet`). Two methods today, being merged into one (the in-place construction, its surface exact where a motion carries the edge and both faces into themselves). Closed-form blends (along straight and circular edges between planes, cylinders and cones) are tool solids applied with the boolean, except the classes already moved in place. Freeform blends (`blend::rolling`, a rolling ball) are built in place along the edge as a spline surface within the tolerance, with chains and meets. *Structural debt: two systems that solve the same problem (§9).* [`design/blends.md`](design/blends.md)
4. **Sweeps** (`kernel::profile`, `extrude`, `extrude_to`, `revolve`, `loft`, `sweep`, `helix`, `thread`): a profile moved along a path or through sections; exact surfaces where the motion allows, splines within half the tolerance otherwise. [`design/sweeps.md`](design/sweeps.md)
5. **Face operations** (`kernel::tweak` and its users: `offset_face`, `shell`, `move_face`, `draft`, `thicken`, `delete_face`): change the surfaces of some faces and re-intersect their neighbours, keeping every entity and its number; shell and thicken add a boolean. [`design/face-operations.md`](design/face-operations.md)
6. **Checks** (`topo::check`, `tess::body_self_intersections_near`, `tess::body_touches_itself`): every result is valid and does not pass through or touch itself, or it is refused. The kernel does not run the mesh checks itself (`kernel` and `tess` share a layer); regeneration does. [`design/topology.md`](design/topology.md), [`design/regeneration.md`](design/regeneration.md)
7. **Measure** (`topo`): area, volume, centroid, mass and inertia by boundary integrals over the exact surfaces, refined until two sums agree, refused when they do not. [`design/topology.md`](design/topology.md)

## 6. The tessellator

Edges first: each edge becomes points once, and both faces that share it use them, so a closed body gives a closed mesh by construction (and this is checked per face, chord by chord). Each face is triangulated in its parameter plane by our own constrained Delaunay triangulation, seeded with a grid sized from bounds on the surface's derivatives, and refined until every triangle is within the chordal tolerance, and optionally within an angle. Faces that go round their surface are cut along a seam; poles, apexes and spline surfaces that close on themselves are handled explicitly. No length of its own (rule 5); parallel per face, merged in face order (rule 4); a work budget per triangulation and a triangle budget per scene, so nothing can run away. Details: [`design/tessellator.md`](design/tessellator.md).

## 7. The engine and its clients

- **Session** (`engine`): open documents, the command registry (71 commands in eleven families: `file`, `recovery`, `edit`, `variable`, `sketch`, `datum`, `feature`, `body`, `history`, `document`, `view`), undo, autosave and crash recovery, scenes per document revision (kept within a memory budget), read and write roots for files.
- **Commands**: an id, a parameter string, a summary and a `run` function, in the `SPECS` of `commands.rs` or of an `<area>_cmds.rs` module; the UI, CLI, control channel and MCP all dispatch the same ones. A failed command changes nothing; a panic inside one is caught and reported. `panic_hunt` runs every command with hostile parameters.
- **The window** (`apps/craft3d`, `ui-egui`): a thin shell. Commands, regeneration and meshing run on the engine thread; the viewport draws on the GPU (`gpu::Onscreen`), with the CPU renderer as the fallback and as the oracle for screenshots. The control channel (loopback TCP, a token) lets an agent drive the window the developer watches, also over MCP (`craft3d-cli mcp --bridge`). Details: [`design/engine.md`](design/engine.md), [`design/rendering.md`](design/rendering.md), [`control-protocol.md`](control-protocol.md).

## 8. Files

`.3dcraft` (a stored ZIP with a manifest: the history, versioned with migrations, format 17, never silently dropping a field); STEP AP214 export with colours; STL, 3MF (with colours) and OBJ meshes; DXF and SVG of flat outlines. Formats live in standalone crates; the mapping to our model lives in `io`. Details: [`design/files.md`](design/files.md), [`file-formats.md`](file-formats.md).

## 9. Known structural debt

Kept here on purpose, short, so the developer sees where the structure is weakest. Each item points to its detail document; the consolidation rounds work through this list.

| Where | The debt | The general method it should become |
|---|---|---|
| Blends: what fits ([`blends.md`](design/blends.md)) | Decided by building since 2026-10-15 (`blend::fit`): the rules of the estimate are gone. Left: a refusal costs several builds (bounded by 48), and the largest size that fits is an event of the faces found to within the builds made either side of it (their curved edges sampled), or bisected to a few tolerances. | Events bounded where the geometry gives a bound, as for the other sampled checks. |
| Blends: two systems ([`blends.md`](design/blends.md)) | Closed-form tools plus the boolean, and rolling-ball surfaces built in place, solve the same problem twice (`blend::mixed` runs both when one feature needs both); since 2026-10-16 an isolated straight edge between two planes, or next to a cylinder along it, goes in place with an exact surface (steps 1 to 3 of the migration in `blends.md`); `blend::join` still joins chain tools although the boolean now takes the contact it was written to avoid. | One blend construction; closed forms as an exact special *surface*, not a separate pipeline. |
| Boolean: tolerance snapping ([`booleans.md`](design/booleans.md)) | Rules that decide by distance: points within half the tolerance are one vertex, refused up to twice it; a hit within twice the tolerance of an edge's end is that end (`edge_cuts`); two places each within the tolerance of both edges are one vertex (`one_meeting`). No entity carries a tolerance of its own. | Tolerant modelling: entities that carry their own tolerance (item 4 of the boolean checklist in [`roadmap.md`](roadmap.md)). |
| Sampled, not proven ([`geometry.md`](design/geometry.md), [`blends.md`](design/blends.md), [`tessellator.md`](design/tessellator.md)) | Checks that hold at samples only: a blend surface's interior against the exact sections, held at midpoints between stations, and a fillet's tangency along its borders (its fitted borders, cuts, trims and edges to corners are proven on their surfaces since 2026-10-16, `geom::certify`); the tacnode model, checked on 32 lines a side; the marcher's rings round a tangency and its curvature estimates for a freeform field; the side of a surface at a pole, by continuity; a triangle's deviation, at its centroid and side middles. | Bounds wherever the geometry gives one, as the marcher's quadtree and the tessellator's seed grid already have. |
| Face operations ([`face-operations.md`](design/face-operations.md), [`regeneration.md`](design/regeneration.md)) | offset face, shell and thicken check the whole body for self-intersection; move face and draft only near the change. | One local check for all face operations. |
| Docs | `architecture.md` mixed the map with the history of every finding. | Done 2026-10-15: this map plus detail documents (§10). |

## 10. Where the details are

| Document | What it holds |
|---|---|
| [`design/geometry.md`](design/geometry.md) | curves, surfaces, intersections, poles, tangency, tacnodes, the marcher |
| [`design/topology.md`](design/topology.md) | the B-rep, `check`, genus, the self-intersection check, mass properties, transforms |
| [`design/kernel.md`](design/kernel.md) | what every operation promises; the list of operations |
| [`design/booleans.md`](design/booleans.md) | imprint, classify, assemble; freeform operands; what is refused; speed; the corpus |
| [`design/blends.md`](design/blends.md) | closed-form blends, rolling-ball blends, chains, meets, ends |
| [`design/sweeps.md`](design/sweeps.md) | profiles, extrude and its ends, revolve, loft, sweep, helix, thread |
| [`design/face-operations.md`](design/face-operations.md) | offset face, shell, move face, draft, thicken, delete face |
| [`design/names.md`](design/names.md) | persistent names: how they are made, followed and resolved |
| [`design/document.md`](design/document.md) | the document model, expressions, sketches and the solver, datums, appearance and material |
| [`design/regeneration.md`](design/regeneration.md) | the cache, long rows, failed features, the self-intersection check per operation |
| [`design/tessellator.md`](design/tessellator.md) | edges first, faces, seams, poles, angle, budgets, display and picture quality |
| [`design/rendering.md`](design/rendering.md) | the CPU oracle, the GPU renderer and their tolerances, section, see-through, pointing, the viewport |
| [`design/engine.md`](design/engine.md) | Session, commands, threads, scenes, recovery, automation |
| [`design/files.md`](design/files.md) | the formats as built (their specification: [`file-formats.md`](file-formats.md)) |
| [`design/plan.md`](design/plan.md) | the design plan of 2026-10-12 as written, and the revision history of the old single file |
| [`file-formats.md`](file-formats.md), [`control-protocol.md`](control-protocol.md), [`testing.md`](testing.md) | as before |

Each detail document starts with **Method** (how it works now, as if written fresh) and then **Findings and history** (dated entries, with the tests that pin them). When a method changes, its Method section is rewritten, not appended to. [`design/README.md`](design/README.md) says where each section of the old single file went.
