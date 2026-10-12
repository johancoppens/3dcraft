# Decisions

> **Last updated:** 2026-10-12

Decisions that shape the project, newest first, each with its reason. An agent that needs something listed under *Open decisions* asks the developer; it does not pick silently. When one is settled, move it up with the date.

## Taken

### 2026-10-12: Persistent names, first version (M6)

Made by the agent while building the first consumer (a sketch on a face); the developer can overturn any of them. The scheme itself is described in [`architecture.md`](architecture.md) §4.1.

- **A name is made of the feature, the role and the input entities** (`F2:side(e7)`), never of an index or a position. It resolves to exactly the face with that origin or to nothing.
- **A reference that cannot be resolved is stored and fails,** with the reason and the names that do exist, like every other feature whose input is not there. It is never repaired by guessing.
- **Start and end of an extrusion are about the sketch plane,** not about which side is lower: the start cap lies in the sketch plane, the end cap at the far side. A symmetric extrusion starts on the side the sketch normal points away from.
- **A sketch on a face looks at the face from outside,** with the frame described in the architecture document. The price: turning an extrusion round mirrors a sketch on its end cap. Keeping the position instead would mean a sketch whose normal points into the body.
- **A sketch on a side face is measured from the plane of the extruded sketch,** not from where the extrusion starts, so it does not move when the extrusion gets deeper or symmetric.
- **A cap is named after all curves of its profile's outer loop.** Adding or replacing a side of that loop renames the cap, and a reference to it then fails; a hole drawn inside does not. Whether a cap should be found by a looser match is left until a real model needs it.

### 2026-10-12: Sketches (start of M5)

Decided by the developer, from options the agent put forward:

- **A sketch lies on a world plane with an offset.** `xy`, `yz` or `zx`, moved along the normal by an expression. The plane is stored as a tagged enum, so datum planes and planar faces of bodies can join it in M6 without breaking files (a face of an extrusion did, the same day). On `xy` the sketch axes are world X and Y, on `yz` they are Y and Z, on `zx` they are Z and X: always right-handed, with the normal as the third axis.
- **The full constraint list from the first version:** coincident, point on curve, horizontal, vertical, parallel, perpendicular, tangent, equal, concentric, symmetric, midpoint, fixed, distance (also horizontal and vertical), angle, radius, diameter. One format version (3), no later migration for the missing ones.
- **Profile detection moves to M6,** next to extrude, its first user. M5 ends at a solved sketch with degrees of freedom and diagnosis.

- **The sketch constraint solver is our own** (`craft3d-solve`), like the kernel and the tessellator: no SolveSpace, PlaneGCS or other solver library, and its linear algebra is written in the crate (the default of D-open-5). The brainstorm named only the kernel and the tessellator; the developer confirmed on 2026-10-12 that the solver belongs in the same list.

Made by the agent while building it; the developer can overturn any of them:

- **Coordinates in a sketch are plain numbers,** in the document's length unit, not expressions: they say where something was drawn and are overwritten by the solver. Only dimensions (and the plane offset) are expressions.
- **The stored coordinates are the last solution.** Every edit ends by solving all sketches and storing the result in the same undo step, so a changed variable moves the sketches that use it, a saved file holds the solved shape, and the next solve starts from where the sketch is (no flip to a mirror solution).
- **A constraint that conflicts is stored, not refused,** like a feature whose parameter cannot be evaluated: the sketch's status is `failed` and names the constraint; the rest of the sketch stays solved. A redundant constraint is reported but is not an error. While a sketch has a conflict its stored coordinates stay as they last were without one, so it comes back the way it was (same side, same branch) when the conflict is removed.
- **Horizontal and vertical distances are not signed:** the points keep the side of each other they are on.
- **Lines, circles and arcs are built on point entities** (a shared corner is one point), so "coincident" is rarely needed for a closed profile and a point cannot be deleted while something is built on it.
- **Unconstrained parts move as little as possible** (minimum-norm step), so adding a constraint to a rough drawing disturbs only what it has to.

### 2026-10-12: CI weekly and on demand

Decided by the developer: while nobody uses the software, the GitHub workflow runs on Mondays and when started by hand, not on every push. The local `cargo xtask ci` is the gate. Revisit when there are users or outside contributors.

### 2026-10-12: Decisions made while building M2

Made by the agent; the developer can overturn any of them.

- **A bare number in a length parameter is in the document's unit.** `{"size": [40, 20, 5]}` in a millimetre document means millimetres, as typing a dimension does in any CAD system. Inside a formula the expression language stays strict: `10 mm + 5` is still a dimension error.
- **A feature whose parameters cannot be evaluated is stored, not refused.** `feature.block` with an unknown variable succeeds and returns `"status": "failed"` with the reason; the recipe is the user's, and fixing the variable fixes the feature. A command is refused only when the request itself is malformed.
- **Undo history is snapshots,** not reversible operations (see `crates/ops`). Chosen because the document is a small recipe, and because an edit applied to a copy cannot leave the document half-changed. Two known limits, both on the [roadmap](roadmap.md): each step copies the whole document, so large imported bodies must be shared between snapshots before M10; and snapshots cannot merge concurrent edits, which would need reversible operations (M12+ candidate). The developer agreed to this on 2026-10-12.
- **Saving validates.** `.3dcraft` files are only written from documents that pass `Document::validate()`, so everything written can be read back. A document made invalid by a bug can therefore not be saved; that is preferred over writing a file that will not open.
- **`craft3d-zipstore`** is a new standalone crate: the ZIP code is shared by 3MF export (`meshio`) and the native format. `meshio` → `zipstore` is the one listed exception to "standalone crates depend on no workspace crate".
- **`serde` and `serde_json`** are workspace dependencies (document model, file format, command parameters).
- **Default export and render tolerance** is a thousandth of the side of a square with the bodies' total area, so it follows the size of the model; `tolerance` overrides it.
- **The roadmap's M2 and the start of M3 were built together** (primitives as features, regeneration, B-rep tessellation), because a document without bodies could not be checked by any oracle.

### 2026-10-12: Developer's answers to the first open questions

- **Licence: MIT** (`LICENSE`). (was D-open-2)
- **Repository:** git, remote https://github.com/johancoppens/3dcraft, managed with the `gh` CLI. Created private; the developer decides when it becomes public. (was D-open-3)
- **No target app: AI-first with a minimal UI.** Commands are named for clarity to agents; there will be no parity documents against another CAD package. (was D-open-1)
- **External tools as test oracles are allowed.** FreeCAD is available on the developer's machine as an AppImage and may be run as a separate program to validate exports (never linked, vendored or copied from). Tests find it through `CRAFT3D_FREECAD`, are opt-in, and skip with a message when it is unset. (was D-open-7)
- **Fast track to a first picture.** The developer asked for a teapot render as soon as possible, so a vertical slice (Bézier patches → tessellator → CPU renderer) was built ahead of the milestone order. It lives in the planned crates and follows the planned layering; it did not add throw-away code.
- **Toolchain:** Rust stable installed with rustup on the developer's machine.
- **`png` crate** for PNG encoding in `render` (pure Rust, MIT OR Apache-2.0): image encoding is not modelling code. (settles D-open-10 for PNG; compression for `.3dcraft`/3MF follows the same reasoning)

### 2026-10-12: Starting points from the brainstorm

From `BRAINSTORM.md`, the developer's own statements:

- **Pure Rust; own CAD kernel and tessellator.** No existing geometry kernel or mesher.
- **Everything is a command;** the same engine behind UI, CLI, JSON control channel and MCP.
- **Phase 1 has (almost) no UI:** load and save only.
- **Native format stores the design history:** sketches, constraints, formulas, variables and the order in which they were applied. Own document model.
- **Export:** STEP, and STL/3MF/OBJ meshes.
- **Clean-room:** public specs and observed behaviour only.
- **wgpu on the GPU, with a CPU reference implementation as the oracle,** tested against each other. No Electron, no webview.
- **AI writes tests, runs them, checks, improves;** the developer watches along during development and automated testing.
- **Dutch prompts, English repository.**

### 2026-10-12: Interpretations made while writing the docs

Proposed by the agent that wrote the first docs; the developer can overturn any of them.

- **"Two compositors" becomes "two renderers".** The brainstorm's compositor, copy-on-write tiles and multithreaded filters come from PhotoCraft's description. For a CAD modeller the equivalents are: a CPU reference renderer and a wgpu renderer for *display*; persistent, `Arc`-shared documents and bodies; parallel tessellation and regeneration. Kernel and tessellator have a single CPU implementation, and geometric correctness is tested with mathematical oracles rather than pictures ([`testing.md`](testing.md)).
- **Machine-name prefix `craft3d`.** A Rust crate name cannot start with a digit, so `3dcraft-*` is impossible. Product name stays **3DCraft**; crates, binaries and environment variables use `craft3d`. File extension `.3dcraft`.
- **Persistent topological names from the first feature** ([`architecture.md`](architecture.md) §1.1, §4.1).
- **Tolerance and units are document data,** kernel maths is `f64`, NURBS are in the geometry enums from M1.
- **UI toolkit: egui/eframe on wgpu,** as in PhotoCraft, for the phase-1 viewer. The engine seam keeps it replaceable.
- **Not adopted from the storytold apps for now:** the private `craftrules` standards (not available next to this checkout), `craft-libs`, `craft-fonts`, the contributor-credits machinery, release packaging and the parity/scorecard documents.

## Open decisions

| # | Question | Options | Needed by | Default if nobody decides |
|---|---|---|---|---|
| D-open-4 | **Is this project part of storytold's `*Craft` family** (naming, bundle ids, shared standards), or an independent project that follows the same principles? | Family member; independent | Before any release or public repo | Independent, same principles |
| D-open-5 | **General-purpose maths dependencies in the kernel** (linear algebra, interval or exact arithmetic) | Write everything; allow specific permissive pure-Rust crates | M1 | Write it; revisit with a measured reason |
| D-open-6 | **Robust predicates:** tolerant floating point only, or exact/adaptive predicates for orientation tests | Tolerant only; adaptive exact for a few core predicates | M1 (`math`), decisive by M7 | Adaptive exact for orientation predicates, tolerant elsewhere |
| D-open-8 | **STEP application protocol** to declare on export | AP214; AP242; AP203 | M8 | AP214 schema declaration with the common B-rep subset |
| D-open-9 | **Default units and tolerance** of a new document | mm / deg / 1e-6 mm; other | M2 | mm, degrees, 1e-6 mm linear, 1e-9 rad angular |
| D-open-10 | **Compression crate for 3MF/`.3dcraft`** (DEFLATE, zstd) | Permissive pure-Rust crates; write our own | M2/M3 | Use permissive pure-Rust crates: not modelling code |
| D-open-11 | **Coordinate convention** | Z up (CAD/3D-printing convention); Y up | M2 | Z up, right-handed |
