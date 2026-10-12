# 3DCraft architecture

> **Last updated:** 2026-10-12 · **Status:** design plan v1; first crates built · **Change:** minor (sketches and the solver in the current state)

## Current state (2026-10-12)

What exists on `main` today; where the code differs from the plan below, this summary wins.

- **Workspace:** 21 library crates (the 20 of the plan plus `zipstore`), all registered in `xtask/src/layers.rs`. With code: `math`, `geom`, `topo`, `expr`, `doc`, `ops`, `solve`, `kernel`, `tess`, `regen`, `render`, `zipstore`, `meshio`, `format`, `io`, `engine`, `automation`, `testkit`. Empty stubs: `step`, `gpu`. `apps/craft3d-cli` is the headless CLI and `apps/craft3d` the desktop app. `xtask` has `layers` (layering and lint opt-in), `wasm`, `ci`, `stats` and `freecad`.
- **The one path works end to end** for primitive solids: command → document → regeneration → B-rep (checked) → tessellation → render or mesh file, and FreeCAD reads the result as a closed solid of the right volume.
- **Maths:** `math` has `Vec3`, `Bbox3`, `Frame` (placement of analytic geometry), `Transform` (affine, with inverse), `Tolerance` (linear and angular, as data) and Gauss–Legendre quadrature. Not built: robust orientation predicates (first needed by booleans, M7), intervals, root finding.
- **Geometry:** `geom` has the `Curve3` enum (line, circle, ellipse, NURBS) and the `Surface` enum (plane, cylinder, cone, sphere, torus, Bézier patch, NURBS), each with evaluation, first derivatives, parameter domain and periodicity, normals, and closest-point projection (closed form for the analytic kinds, sampled Newton / Gauss–Newton for ellipses and freeform). NURBS are de Boor on homogeneous points with hodograph derivatives. Not built: 2D parameter-space curves, trimmed and offset curves, extrusion/revolution/offset surfaces, second derivatives, intersections, transforming geometry.
- **Topology:** `topo` has the B-rep (`Body` with vertices, edges, loops of coedges, faces, shells), the validity checker `check` (references, finite numbers, vertices on edges, closed loops, edges on their faces' surfaces, each edge of a solid used once each way, Euler–Poincaré with genus) and `mass_properties` (area, volume, centroid by boundary integrals over the exact geometry). Fourteen reference solids (six primitives and hemisphere, and eight with trimmed faces: holes, half walls, a spherical cap, a frustum, an ellipse edge, Bézier and NURBS roofs) match their closed forms at three scales and in tilted frames. `mass_properties` refuses, rather than gets wrong, a torus face bounded by loops around the tube (a pipe elbow) and a boundary that passes through a pole or apex (a sphere octant). Deviations from the plan below: parameter-space curves are not stored (recovered by projection), no per-entity tolerances, no persistent names yet (M6), no inertia tensor, and the checker does not test self-intersection or that each face is bounded by its loops.
- **Expressions:** `expr` has dimensions (length and angle exponents) and units (`um mm cm m in ft`, `deg rad`), a parser and evaluator for formula text (`+ - * / ^`, functions, `pi`, variables, a unit after a number), and `VariableTable` (deterministic evaluation order, cycle detection, partial failure, rename that rewrites references). Source length, nesting depth and table size are capped. Not built: printing an expression back to text, storing the evaluation order (computed on demand; storing it is `doc`/`format`'s job).
- **Document:** `doc` has `Document` (units, tolerance, variables, ordered features, rollback marker, next feature id), features `Block`, `Cylinder`, `Sphere`, `Cone`, `Torus` whose parameters are expressions kept as source text, an `Orientation` (axis and angle) on all but the sphere, stable never-reused feature ids, and `validate()`. Sketches are features too (`doc::sketch`): a plane (a world plane with an offset), point, line, circle and arc entities with ids that are never reused, sixteen kinds of constraint in the order they were added, and the last solution as the stored coordinates. Not built: the other feature kinds, references between features.
- **Sketch solver:** `solve` takes a sketch as plain numbers (points, lines, circles, arcs; constraints with evaluated values) and returns solved coordinates, the degrees of freedom left, and per constraint whether it is satisfied, redundant or conflicting (the later constraint is blamed). Newton-type with minimum-norm steps from the given start, so a solved sketch does not move and a free part moves as little as possible; every loop is capped. It knows nothing about the document: `regen` maps one onto the other.
- **Undo:** `ops::History` keeps snapshots (`Arc<Document>`) rather than reversible ops: an edit is applied to a copy and becomes one labelled step only if it succeeds and changes something.
- **Kernel:** `kernel::primitives` builds block, cylinder, cone, sphere and torus as B-rep. `kernel::profile` finds the closed regions of a set of 2D curves (loops of lines and arcs joined end to end, circles; even-odd nesting gives holes and islands; open, branching, crossing or touching curves are errors that name them). `kernel::extrude` sweeps profiles along a plane's normal into one solid with a shell per profile, checked before it is returned, and reports for every face, edge and vertex where it came from (`Origins`).
- **Regeneration:** `regen::regenerate` evaluates variables and builds each feature's body, with a status per feature (`Ok`, `Suppressed`, `RolledBack`, `Failed(reason)`); every body is run through `topo::check`. A bare number in a length parameter is taken in the document's unit. A sketch feature has no body: its result is the solved sketch (degrees of freedom, a state per constraint, solved coordinates, its frame in space), and `regen::settle_sketches` writes a solution back into the document, which the engine does at the end of every edit. No caching: a full rebuild per document revision, every sketch solved again.
- **Tessellation:** two entry points. `tessellate_patches` meshes untrimmed Bézier patches on one uniform grid (the teapot). `tessellate_body` meshes any B-rep body edges first: every edge (line, circle, ellipse, NURBS) becomes points once, within the chordal tolerance, and every face that uses it uses those points, so the welded mesh of a valid solid is a closed manifold. Each face is triangulated in its surface's parameter space by our own constrained Delaunay triangulation (holes by the even–odd rule), with interior points added until every triangle is within tolerance of the surface; loops that wind around an axis are cut along a seam whose two sides share their 3D points; poles, apexes and collapsed patch edges become one point. Faces are tessellated in parallel (`std::thread::scope`) and merged in face order: 1, 2 and 8 threads give byte-identical meshes. Planar faces get no interior points (a block is 12 triangles). Still `Unsupported`: a loop winding more than once, or around both the axis and the tube of a torus; a face around an axis with no room for a straight seam.
- **Mesh export:** `meshio` writes binary and ASCII STL, 3MF and OBJ; `io` welds a tessellated mesh, classifies it as solid or surface and encodes one or more objects. FreeCAD reads the box, the teapot and all fourteen tessellated reference solids (primitives, hemisphere, and the solids with trimmed faces) back as closed solids with the same counts, area and volume (`cargo xtask freecad`). No readers, no colours.
- **Native format:** `format` reads and writes `.3dcraft`: a stored ZIP (`zipstore`, with a reader hardened against hostile archives) holding a pretty-printed `manifest.json`, now at `format_version` 4 (version 1 to 3 files still open, through the migration chain). The file's types are separate from `doc`'s and converted by exhaustive destructuring, so a new document field does not compile until it is saved and loaded (verified by adding one). Saves are atomic and refuse an invalid document; loads validate. Not built: `thumb.png`, `blobs/`, the directory layout, autosave and crash recovery.
- **Engine:** `engine::Session` holds open documents and runs 36 commands (`file.*`, `edit.*`, `variable.*`, `feature.*`, `sketch.*`, `history.rollback`, `document.inspect`, `document.check`, `document.measure`, `view.render`) by id with JSON parameters. Edits are atomic undo steps; a failed command changes nothing; a panic inside a command is caught and reported. File access goes through explicit read and write roots. `tests/panic_hunt.rs` runs every command with adversarial parameters in four document states. Deviations from the plan: synchronous (no job pool, no events, no `arc-swap` snapshots), no parameter schemas beyond the documented params string, no `enabled` predicate beyond "needs a document".
- **Desktop app and shell:** `ui-egui` is the shell as a library (serde state with the orbit camera, theme tokens, the viewport showing the CPU renderer's image as a texture with sketches drawn over it, a read-only feature tree and a status bar); `apps/craft3d` puts it in an eframe window and adds the control listener (TCP on 127.0.0.1, token, the `serve` protocol plus `ui.*`), whose requests are answered on the UI thread between frames on the one `Session`. `engine::Session::scene` gives the shell meshes and sketch polylines per document revision. `automation::bridge` is an MCP server that passes every message to that window (`mcp.message`).
- **Rendering:** `render` is the CPU reference renderer (perspective and orthographic, depth buffer, smooth two-sided shading, id buffer, supersampling, PNG). No edge overlay yet, no near-plane clipping. `gpu` does not exist.
- **Demo:** `cargo run --release -p craft3d-render --example teapot` renders the Utah teapot (32 patches) to `target/evidence/teapot.png`.

---

This plan sets out:
- the Cargo workspace,
- the rules that keep the engine independent of the UI,
- the document model and how bodies are derived from it,
- how the kernel, tessellator, renderers, file formats and automation fit together.

When code exists and differs from this plan, add a dated "Current state" section at the top; that section wins over the plan below.

---

## 1. Goals and principles

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

### 1.1 Avoiding the usual CAD holes

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
| **File formats** | Format quirks leaking into core types | Formats live in standalone crates with their own format-level model; mapping to the document lives in `io`. Adding a format touches a leaf crate | `step`, `meshio`, `io` |
| **UI toolkit** | A core that calls into its toolkit | Nothing below L7 knows a toolkit exists | layering rule 2 |

The test is simple: **adding a surface kind, a feature type, or a file format should touch leaf crates, not the foundation.**

---

## 2. Workspace layout

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
│  ├─ step/                    craft3d-step       ISO 10303-21 reader/writer and typed AP entities; its OWN model
│  ├─ meshio/                  craft3d-meshio     STL (ASCII/binary), 3MF, OBJ writers (readers later); its OWN mesh model
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

## 3. Dependency layering (enforced)

```text
 L8  apps/*                         (binaries: wire everything together)
 L7  ui-egui · automation
 L6  engine
 L5  io
 L4  regen · render · gpu · format
 L3  ops · solve · kernel · tess
 L2  doc
 L1  topo · expr
 L0  math · geom                    step, meshio (standalone, no workspace deps)
```

**Rules** (checked by `cargo xtask layers`, which parses `cargo metadata`):

1. A crate may depend only on crates in **lower** layers. No cycles. Sideways dependencies only where listed in `SAME_LAYER` in `xtask/src/layers.rs` (today: `geom` → `math`).
2. **Nothing below L7 may depend on any UI toolkit, winit, eframe or rfd.** Platform services (file dialogs, clipboard) reach the engine as injected functions or traits.
3. **`step` and `meshio` depend on no workspace crate.** The mapping between their models and ours lives in `io`. This keeps them publishable and fuzzable in isolation.
4. **`doc` contains no geometry algorithms.** It may use `topo` and `expr` *types* (names, expressions), nothing from L3.
5. **`kernel` and `tess` never depend on `gpu` or `render`.** Geometry has one implementation.
6. **wasm gate:** every crate in L0–L7 must build for `wasm32-unknown-unknown` (`cargo xtask wasm`).
7. `gpu` is optional for `engine`: the headless CLI and CI tests run `cpu-only`.

---

## 4. Document model (`craft3d-doc`)

A document is the recipe. Sketch of the types (final names are settled in M1/M2):

```rust
pub struct Document {
    pub id: DocId,
    pub units: Units,                    // length + angle unit
    pub tolerance: Tolerance,            // linear + angular modelling tolerance
    pub variables: VariableTable,        // name -> expression, in stored evaluation order
    pub features: Vec<Feature>,          // THE history: explicit order of application
    pub rollback: Option<FeatureId>,     // history marker: regenerate up to here
    pub metadata: Metadata,
}

pub struct Feature {
    pub id: FeatureId,                   // stable, never reused within a document
    pub name: String,
    pub suppressed: bool,
    pub kind: FeatureKind,
}

pub enum FeatureKind {
    DatumPlane(DatumPlane), DatumAxis(DatumAxis), DatumPoint(DatumPoint),
    Sketch(Sketch),
    Primitive(Primitive),                // box, cylinder, cone, sphere, torus
    Extrude(Extrude), Revolve(Revolve), Sweep(Sweep), Loft(Loft),
    Boolean(Boolean),                    // union / subtract / intersect between bodies
    Fillet(Fillet), Chamfer(Chamfer), Shell(Shell), Draft(Draft),
    Pattern(Pattern), Mirror(Mirror), Transform(TransformBody),
    Import(ImportedBody),                // a history-free body from STEP, stored as a blob
}

pub struct Sketch {                      // as built in M5: crates/doc/src/sketch.rs
    pub plane: SketchPlane,              // World { plane: Xy|Yz|Zx, offset: Expr }; datum planes and faces join in M6
    pub entities: Vec<Entity>,           // id, construction flag, geometry: Point {x, y}, Line, Circle, Arc on point ids
    pub constraints: Vec<Constraint>,    // id + kind, in the order they were applied
    pub next_entity_id: u64,             // ids are never reused within a sketch
    pub next_constraint_id: u64,
}                                        // the point coordinates and circle radii ARE the last solution

pub enum Constraint {
    Coincident(..), Horizontal(..), Vertical(..), Parallel(..), Perpendicular(..), Tangent(..),
    Equal(..), Concentric(..), Symmetric(..), Midpoint(..), Fixed(..),
    Distance { a: EntRef, b: EntRef, value: Expr }, Angle { .. , value: Expr },
    Radius { .. , value: Expr }, Diameter { .. , value: Expr },
}

pub struct Expr { pub source: String /* "width / 2 + 3 mm" */, /* parsed form cached, not serialised */ }

pub enum Ref {                           // how a feature points at geometry
    Feature(FeatureId),                  // a datum or a whole body
    Name(PersistentName),                // a face / edge / vertex
}
```

- **Every numeric parameter is an `Expr`,** not a number. `Extrude { distance: Expr }` may be `"25 mm"` or `"height - 2 * wall"`. The expression's source text is what is saved.
- **Order is explicit.** `features` is the order of application. `variables` keeps a stored topological order; `constraints` keeps the order in which they were added. Saving and reopening must reproduce the same model, so nothing that affects the result may depend on map iteration or solver start-up luck.
- **Solved sketch state is saved.** Constraint systems have several solutions (mirror branches). The last solution is stored and used as the solver's starting point, so a reopened sketch does not flip.
- **No bodies in the document.** Bodies live in the regeneration result, keyed by feature id.

### 4.1 Persistent names

A `PersistentName` identifies a topological entity by its origin, for example "the face made by extruding sketch entity `e12` of feature `F3`", "the cap face at the end of `F3`", "the edge between faces *A* and *B*", "the part of face *A* that survived boolean `F7`, on the side of operand 2". Names are produced by `kernel` operations (each operation reports, for every entity of its result, where it came from) and resolved by `regen` against the body at that point in the history.

Resolution outcomes are explicit: `One(entity)`, `Many(entities)` (the entity was split; the feature's policy decides whether that is acceptable) or `Missing`. A `Missing` or unexpected `Many` marks the feature as failed with a message naming the reference; the features before it remain valid.

The naming scheme is the hardest design problem in this plan and is prototyped in M6 before anything depends on it. Its rules and known limits get their own section in this document when that prototype lands.

**As built so far (M6, first slice).** An extrusion names what it makes by role and by the sketch entities involved, written `F2:side(e7)`: the caps are `startCap` and `endCap` with the entity ids of the profile's outer loop; a side face is `side(curve)`; the cap edges are `startEdge(curve)` and `endEdge(curve)`; the upright edge swept by a corner is `lateralEdge(incoming, outgoing)` in the direction the loop runs through that corner, and the corners are `startVertex` and `endVertex` with the same pair. The names do not depend on the order of the curves or on what else is in the sketch: drawing a hole afterwards adds names and changes none. Known limits: drawing a loop *around* an existing one turns it into a hole and reverses it, so its corner and upright-edge names get their pair the other way round (the old names go missing rather than land elsewhere), and the caps are renamed after the new outer loop. **Resolution (M6).** `doc::PersistentName` is the name as data (feature, role, sources; `F2:side(e7)` as text). `regen` resolves it against the features regenerated so far: the feature must exist, come earlier in the history, have succeeded, and have a face whose origin is exactly that name. Anything else fails the referring feature with a reason that says which of those it was and lists the names that do exist; a name never resolves to a different face. The first consumer is a sketch on a face (`SketchPlane::Face`), and the suite in `crates/engine/tests/naming.rs` holds the scenarios.

**The frame of a sketch on a face** is the face's own, seen from outside the body (z is the outward normal, moved by the sketch's offset): on the end cap of an extrusion, the axes and origin of the extruded sketch, moved to the end; on the start cap the same origin and x axis with y reversed; on a side face made from a line, the line's start with x along the line (in the direction the profile's loop runs) and y along the extruded sketch's normal, measured from the plane of that sketch, so the sketch keeps its place when the extrusion gets deeper or becomes symmetric. "Start" is the cap in the sketch plane and "end" the far one, whichever way the extrusion goes. Known limits: turning an extrusion round (a negative distance) puts its end cap on the other side facing the other way, so a sketch on it is mirrored in y, and a sketch on a side face stays on its side of the sketch plane rather than following the body; primitives do not name their faces yet; a curved face cannot carry a sketch.

---

## 5. Kernel

### 5.1 Geometry (`geom`)

```rust
pub enum Curve3 { Line(..), Circle(..), Ellipse(..), Nurbs(..), Trimmed(..), Offset(..), SurfaceIso(..), Intersection(..) }
pub enum Curve2 { Line(..), Circle(..), Ellipse(..), Nurbs(..) }           // parameter-space curves on a surface
pub enum Surface { Plane(..), Cylinder(..), Cone(..), Sphere(..), Torus(..), Extrusion(..), Revolution(..), Nurbs(..), Offset(..) }
```

Every kind implements evaluation with derivatives, parameter range and periodicity, bounding box, closest-point projection and inversion, and reversal. Intersections (curve/curve, curve/surface, surface/surface) are dispatched on the pair of kinds: closed-form where a formula exists (plane/plane, plane/cylinder, …), numeric marching with a tolerance otherwise. A pair that is not implemented returns `Unsupported`, and the test suite lists which pairs are covered.

### 5.2 Topology (`topo`)

```text
Body ── Shell ── Face ── Loop ── Coedge ── Edge ── Vertex
                  │                 │        │        │
               Surface           Curve2    Curve3   Point     (+ orientation flags, tolerances, persistent names)
```

- A **body** is a set of shells: a *solid* (closed, oriented shells), a *sheet* (open faces) or a *wire* (edges only).
- A **face** is a surface trimmed by loops; a **coedge** is the use of an edge by a face, with a direction and a parameter-space curve.
- Storage is arena-based with typed ids, and persistent: an operation builds a new body that shares unchanged parts with its input.
- **Validity checker** (`topo::check`): Euler–Poincaré, every edge of a solid used by exactly two coedges with opposite directions, loops closed, coedge curves on their surface within tolerance, vertices on their edges, consistent orientation, no self-intersection (the expensive checks behind a level flag). Every kernel operation is tested to return bodies that pass it.
- **Mass properties:** volume, surface area, centroid and inertia tensor, computed from the exact B-rep (surface integrals), independent of the tessellator. These are the backbone of the test oracles.

### 5.3 Operations (`kernel`)

Each operation is a pure function: bodies and parameters in, `Result<OpResult, KernelError>` out, where `OpResult` holds the new body and the **origin map** (for every result entity: which input entities and which role produced it), which feeds persistent naming.

| Group | Operations | Milestone |
|---|---|---|
| Primitives | box, cylinder, cone, sphere, torus | M3 |
| From sketches | profile detection (closed regions from sketch entities), extrude, revolve | M6 |
| Booleans | union, subtract, intersect on solids; imprint, split | M7 |
| Local | fillet, chamfer (constant radius first), shell, draft, offset face | M9 |
| Sweeps | sweep along a path, loft between profiles | M9 |
| Arrangement | transform, mirror, linear and circular pattern | M6–M9 |

**Rules for every operation:**
- **Signature:** `fn(inputs, params: &P, tol: &Tolerance, cancel: &CancelToken) -> Result<OpResult, KernelError>`; `P: Serialize + Deserialize`.
- **Never an invalid body.** The result passes `topo::check` or the operation returns `Err`. Errors name the entities and the reason ("faces F3/cap and F5/side are tangent along a curve; tangent booleans are not supported yet").
- **Deterministic:** no dependence on thread count or hash order.
- **Not registered by itself:** `engine` registers commands; `kernel` stays engine-agnostic.

### 5.4 Sketch solver (`solve`)

A 2D geometric constraint solver: entities become unknowns, constraints become equations, solved numerically (Newton-type with a rank-revealing step) from the stored previous solution. It reports degrees of freedom, which constraints are redundant or conflicting, and never loops forever (iteration cap → `DidNotConverge`). Dragging (later, with the UI) is the same solve with a soft target. Decomposition into small sub-problems is an optimisation for later; correctness and diagnostics come first.

As built (M5): the unknowns are point coordinates and circle radii; gradients are exact (forward-mode dual numbers); the Jacobian is factorised row by row in the order the constraints were added, so a row that depends on earlier ones marks its constraint as redundant (it holds) or conflicting (it does not). When the whole system does not converge, constraints are added one at a time from the solution of the earlier ones, and one that cannot be added is blamed. That diagnosis is this solver's verdict, not a proof. A constraint that has no value where the others put the sketch (an angle on a line that shrank to a point) is conflicting too, with the reason. `solve` repeats its pass from its own result until nothing changes, so `Ok` means that solving again gives the same coordinates, bit for bit, and the same verdict. `regen::settle_sketches` still does not store a solution with a conflict in it: a collapsed line no longer says which way it pointed, and the stored shape is what brings the sketch back on the same side. Steps are minimum-norm with a line search, so free geometry moves as little as possible and mirror choices (which side of a line, inside or outside a circle) follow the starting point. An angle is signed: counter-clockwise from the first line's direction to the second's.

### 5.5 Regeneration (`regen`)

`regen` walks `Document::features` in order and produces, per feature, a status (`Ok`, `Suppressed`, `Failed { reason }`, `Skipped` because an input failed) and the bodies that exist after it.

- **Incremental.** Each feature's result is cached under a key made of its own parameters (after expression evaluation) and the keys of its inputs. Editing feature *k* reuses 0..k-1 untouched and recomputes only the features that depend on what changed.
- **Variables first.** The variable table is evaluated in its stored order; a cycle or a dimension error fails only the features that use the affected variables.
- **Partial success is normal.** A failed feature does not discard the model: bodies up to the failure stay available and renderable, and the failure is data an agent can read (`document.inspect`).
- **Background jobs.** Regeneration runs on the job pool from a snapshot and commits its result if the document has not changed in a conflicting way.

---

## 6. Tessellator (`tess`)

`tessellate(&Body, &TessParams, cancel) -> Result<Mesh, TessError>` with `TessParams { chordal_tolerance, angular_tolerance, max_edge_length }`.

- **Edges first, then faces.** Each edge is discretised once; both faces sharing it use the same points, so the mesh of a solid is **watertight by construction**: no cracks, no T-junctions.
- **Faces** are triangulated in parameter space (constrained Delaunay over the trimmed domain) with refinement until the surface deviation is within the chordal tolerance, then mapped to 3D. Planar faces need no refinement.
- **Output** keeps, per triangle, the face it came from (and per polyline, the edge), so the renderers can colour, select and pick by entity, and exporters can group.
- **Parallel per face** (rayon), merged in face order, so the result is identical for any thread count.
- **Our own triangulator.** No external triangulation or meshing library; this is part of "custom tessellator".

Mesh invariants are tested, not hoped for: closed 2-manifold for a solid, consistent outward orientation, no degenerate triangles, deviation from the surface within tolerance, mesh volume converging to the B-rep volume as the tolerance shrinks.

---

## 7. Rendering

Display is the one place with two implementations.

### 7.1 Draw list

`render` and `gpu` consume the same data: meshes and edge polylines from `tess` with entity ids, a camera (orthographic or perspective), lights, a display mode (shaded, shaded with edges, wireframe, hidden-line later) and per-entity colours and selection state. Nothing about display leaks into `kernel` or `tess`.

### 7.2 Backends

- **CPU (`render`).** The reference implementation: a software rasteriser with a depth buffer, flat/smooth shading, edge overlay, and an **id buffer** (which face or edge is under each pixel). Used by tests, the CLI (`craft3d-cli render`), MCP previews and as the fallback when no GPU is available. It favours exactness and readability over speed.
- **GPU (`gpu`).** wgpu (Vulkan, Metal, DX12, WebGPU) pipelines for the same draw list; picking reads the id buffer.
- **Parity tests.** For a fixed set of scenes and cameras, the GPU result must match the CPU result: identical id buffers except within one pixel of silhouettes and edges, colours within a small tolerance (target ≤ 2/255; settled with measurements in M4). Runs in CI on a software adapter (lavapipe).

### 7.3 What the oracle is for

In PhotoCraft the CPU compositor defines what the image *is*. Here the model's truth is the B-rep and its mass properties, not the picture. The CPU renderer is the oracle for **display** only: it lets agents and the developer see a model without a GPU, and it catches GPU regressions. Geometric correctness is tested geometrically ([`testing.md`](testing.md)).

---

## 8. Engine (`craft3d-engine`)

### 8.1 `Session`: commands in, events out

```rust
pub struct Session { /* docs, registry, job pool, prefs, services */ }

impl Session {
    pub fn new(services: Services) -> Self;                         // platform functions injected
    pub fn dispatch(&self, doc: Option<DocId>, cmd: CommandInvocation) -> Result<JobHandle, CommandError>;
    pub fn registry(&self) -> &CommandRegistry;
    pub fn snapshot(&self, doc: DocId) -> Arc<DocSnapshot>;         // lock-free read: document + regeneration result
    pub fn events(&self) -> EventReceiver;                          // DocChanged, RegenProgress, FeatureFailed, …
}

pub struct CommandInvocation { pub id: CommandId, pub params: serde_json::Value /* validated against the schema */ }
```

### 8.2 `CommandRegistry`

Each command registers `id` (`"feature.extrude"`), `label`, an optional `menu_path` and shortcut (unused in phase 1), a parameter schema with a one-line params doc agents can read, `enabled(&DocSnapshot) -> bool` with a reason when disabled, and `run`. `run` returns a JSON result and records exactly **one** history step.

Command families (ids are settled when each is implemented, and are stable afterwards):

| Family | Examples |
|---|---|
| `file.*` | `file.new`, `file.open`, `file.save`, `file.export.step`, `file.export.stl`, `file.export.3mf`, `file.export.obj` |
| `edit.*` | `edit.undo`, `edit.redo` |
| `variable.*` | `variable.set`, `variable.delete`, `variable.rename` |
| `sketch.*` | `sketch.create`, `sketch.addLine`, `sketch.addArc`, `sketch.addCircle`, `sketch.constrain`, `sketch.solve` |
| `feature.*` | `feature.primitive.box`, `feature.extrude`, `feature.revolve`, `feature.boolean`, `feature.fillet`, `feature.edit`, `feature.suppress`, `feature.delete`, `feature.reorder` |
| `history.*` | `history.rollback`, `history.regenerate` |
| `document.*` | `document.inspect`, `document.measure` (volume, area, centroid, bounding box, distances), `document.check` (validity report) |
| `view.*` | `view.render` (PNG from a named or explicit camera, CPU or GPU) |

**Inspection is a first-class command.** `document.inspect` returns the feature tree with per-feature status, variables with values, sketches with their degrees of freedom, and bodies with face/edge counts, persistent names and mass properties. An agent must be able to verify what it did from JSON alone, and use `view.render` when it needs to look.

### 8.3 Operations and history (`ops`)

Every mutation of a document is an `Op` (`SetVariable`, `AddFeature`, `EditFeature`, `ReorderFeature`, `AddConstraint`, …). A command produces a labelled `Transaction` of ops. Undo history stores transactions; because documents are persistent data, a "before" state costs only what changed. Undo history (session-only, what the user did) is distinct from the **design history** (the feature list, saved in the file).

### 8.4 Runtime model

```text
 Client (UI / CLI / control / MCP) ──dispatch()──► Engine thread (owns mutable docs, applies ops, publishes snapshots)
        ▲                                               │ spawns
        │ events (changed, progress, failed)            ▼
        │                                   Job pool (rayon): regeneration, tessellation, export, autosave
        │                                   read snapshots, return results
 Viewport: render(snapshot) on GPU or CPU
```

Every job takes a `CancelToken`. Services injected into `Session`: file system (native fs / in-memory for tests / browser later), clock, notifier.

### 8.5 The UI-swap seam

A frontend needs four things from the core: `Session` (commands in, events out), the `CommandRegistry` (metadata), snapshots with view-models (feature tree, variable table, inspector as plain structs), and a renderer it can host (`gpu` into a `wgpu::TextureView`, or `render` into an RGBA buffer). Interactive tools (sketching with the mouse, dragging a face) come after phase 1 and follow PhotoCraft's pattern: pointer events in, ops and overlay primitives out, so the control channel can replay them.

**Litmus test:** `craft3d-cli` and the automation server can do everything the GUI does.

---

## 9. Files

Detail in [`file-formats.md`](file-formats.md).

- **Native `.3dcraft`** (`format`): a bundle (ZIP of STORE entries, or a directory with the same layout) with a versioned `manifest.json` holding the document, and content-addressed blobs for imported bodies and optional regeneration caches. Atomic saves, autosave and crash recovery through the same writer.
- **STEP** (`step` + `io`): export of exact B-rep (AP214/AP242 geometry and topology) first; import later. STEP carries the result, not the history.
- **Meshes** (`meshio` + `io`): STL, 3MF and OBJ export from `tess` output at a caller-chosen tolerance.

---

## 10. Automation

Detail in [`control-protocol.md`](control-protocol.md).

- **CLI** (`craft3d-cli`): `convert`, `info`, `run` (a list of commands against a document), `commands` (the registry), `render`, `serve` (JSON lines on stdio or loopback TCP) and `mcp`.
- **Control channel:** the desktop app listens on loopback with a token; agents send `engine.execute`, `ui.inspect`, `ui.screenshot`. This is also how the developer watches an agent work in a live window.
- **MCP** (`automation`): `doc_*`, `command_list`, `command_run`, `command_batch`, `doc_render_preview`, generated from the registry; file access limited to explicit read and write roots.
- **Scripts:** a recorded session is `Vec<CommandInvocation>`; replaying it rebuilds the model. This is the natural format for agent-written test scenarios.

---

## 11. Testing and quality

Detail in [`testing.md`](testing.md).

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

## 12. Cargo workspace sketch

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

Which general-purpose crates the kernel may use (linear algebra, big-number or interval arithmetic for robust predicates) is an open decision ([`decisions.md`](decisions.md)). The default is: write it, it is part of the kernel.

---

## 13. Open questions in this plan

Tracked in [`decisions.md`](decisions.md): target app (if any), licence, UI toolkit confirmation, external maths crates, exact vs tolerant predicates, STEP application protocol, use of open-source tools as external test oracles.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | Current state after M5 (sketches in the document model, the solver, sketch results in regeneration) |
| 2026-10-12 | minor | Current state after M3 (general tessellator, cone and torus, orientation) |
| 2026-10-12 | minor | Current state after M1 (math, geom, topo, expr) |
| 2026-10-12 | major | Design plan v1, derived from `BRAINSTORM.md` and PhotoCraft's architecture plan |
