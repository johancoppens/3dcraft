# Decisions

> **Last updated:** 2026-10-12

Decisions that shape the project, newest first, each with its reason. An agent that needs something listed under *Open decisions* asks the developer; it does not pick silently. When one is settled, move it up with the date.

## Taken

### 2026-10-13: Decided by the developer

- **The boolean kernel is the first priority:** it is what makes 3DCraft a professional tool. A case the kernel refuses is work owed, not an end state; the checklist is in [`roadmap.md`](roadmap.md).
- **The kernel's tests may use the tessellator** (a dev-dependency inside one layer, listed as `SAME_LAYER_DEV` in `xtask/src/layers.rs`): every body an operation makes is also meshed as an independent check. The kernel's own code still does not depend on `tess`.
- **Where FreeCAD and the kernel disagree in the boolean corpus:** eleven cases stay in `REFERENCE_OFF` (the reference is off, with a closed form, a direct integration or an identity as evidence); `near/cylinder-torus/009` is `UNDECIDED`, because nothing independent measures our result there. Building that measurement is not worth the time now.

### 2026-10-13: Waiting for the developer (presented, not yet decided)

Each stands as described until the developer says otherwise.

- **A ball that dips half a tolerance to two tolerances into a block is refused.** "Touches" is also defensible; the refusal costs 18 corpus cases.
- **Meshes are bit-identical under scaling by powers of two, not by powers of ten** (Delaunay ties on cocircular points fall differently; the triangle count stays within 10%). A wider tie band in `in_circle` would remove most of it but changes the meshes at scale 1.
- **Pinched solids pass the check:** parts of one body that meet only along an edge or at a vertex.
- **Two tests that pinned a refusal were replaced by stricter tests of the value** when the refusal was lifted (the developer agreed to the layer exception that goes with it; the replacement itself was explained, not explicitly approved).

### 2026-10-13: Made during the night of 12 to 13 October (the developer was asleep)

Made by the agent on the developer's instruction to decide by recommendation; the developer can overturn any of them.

- **A sketch on a cap survives a change of the cap's outline** (field test, finding 1). All caps of one role of one extrusion lie in one plane with one frame, so for the plane of a sketch a cap name resolves to that plane as long as the feature has a cap of that role, also when the outline in the name is gone. `F2:endCap()` is accepted as "where F2 ends". This is not a retarget: the sketch lands exactly where it was. It replaces "a cap is named after all curves of its outer loop, and a reference then fails" below, for sketch planes only; a later consumer that needs the face itself (not its plane) still gets the exact match or nothing. The stored name is not rewritten.
- **A sketch on a side face is measured from the origin of the extruded sketch, dropped onto the face** (field test, finding 2), not from the start of the line that made the face. Files written on 12 October with a sketch on a side face whose line does not start at that point place the sketch differently now; format 4 was never released.
- **STEP export declares AP214** (was D-open-8, the default taken): `AUTOMOTIVE_DESIGN` with the common B-rep subset (`ADVANCED_BREP_SHAPE_REPRESENTATION`). Units and the uncertainty come from the document; angles are written in the document's angle unit; the file carries no time stamp, so the same document gives the same bytes; no parameter-space curves, colours, assemblies or entity names yet. FreeCAD reads it; AP242 was not tried.
- **A revolution turns about a line in its sketch's plane:** the sketch's x or y axis, or one of its lines by entity id (usually a construction line), whose direction from start to end sets which way a positive angle goes (counter-clockwise seen from its tip). "Start" is the cap in the sketch plane, as for an extrusion; a full turn has no caps.
- **Robust predicates (was D-open-6): tolerant floating point only, for now.** The boolean kernel for planes and cylinders takes no decision on the sign of a number that could be rounding: membership is a winding number or a parity, points merge at half the tolerance, and input within a band of the tolerance (up to twice) or within 1e-6 rad of tangent or parallel is refused as unsupported. Measured: 0 wrong or invalid results in 13,500 seeded random pairs at scales 1e-3, 1 and 1e6. The default in the table below was "adaptive exact for orientation predicates"; revisit when general polyhedra or freeform faces enter booleans.
- **Booleans: one feature kind with an operation** (`feature.boolean` with `op`: union, subtract, intersect), a target and a tool. Both bodies go into the result; `keepTool` leaves the tool its body. `feature.extrude` and `feature.revolve` take `cut`, `join` or `intersect` with a target and then add the sweep and the boolean as two features in one undo step.
- **`document.check` takes `deep`:** it then also looks for bodies that pass through themselves (`tess::body_self_intersections`, on a mesh, independent of the kernel). A finding is reported only when it is deeper than the mesh and the tolerance could explain; the rest is counted as marginal. Two shells of one body that lie face on face count as a finding.
- **An edge is referred to by the two faces it lies between,** not by a name of its own: that works for every body (primitives, sweeps, copies, booleans) with the one naming scheme faces already have, and gives a list, because two faces can meet in several edges. `document.edges` answers it today; fillets and chamfers will take the same pair.
- **Names through a boolean are copies** like those of a transform: instance 0 the target, 1 the tool. Pieces of one face share one name, and a sketch accepts that (one plane). Edges and vertices of a boolean are not named yet.
- **What the boolean kernel cannot do is an error, never a body:** cones, spheres, tori, freeform faces, cylinders that cross at an angle, tangent contact, contact only along an edge or at a vertex, and input at the edge of the tolerance. Faces are never merged after a boolean (a union of flush boxes keeps the seam).
- **The entities of a primitive are named by their number** (`F3:face(1)`): a primitive is built in a fixed order, so the number says where the entity came from. The order is part of the kernel's contract.
- **A feature made from another feature's body takes that body** (transform, mirror, pattern; later booleans). The source feature stays in the history and its names still resolve, but its body is no longer listed, rendered or exported; `keep` makes a copy instead. A body can be taken once: the second taker fails and says who has it. The alternative, features that edit a shared body in place, would have made the order of independent features matter.
- **A pattern counts the source as instance 0** and gives one body with all instances. A circular pattern takes the angle between neighbours (`360 deg / n` goes evenly round), a linear one the step between neighbours; both are expressions, and so is the count. Instances that touch or overlap are joined by the kernel's union, one after the other (2026-10-13; before, they were put side by side unchecked): side by side they would leave faces inside the material. When the union cannot be made the pattern fails with the kernel's reason. Instances that are apart are merged as they are and keep their edge and vertex names.
- **Names of copied entities are the original name with a prefix** (`F5:copy(2)/F2:side(e7)`), so one scheme serves transforms, mirrors, patterns and, later, booleans. File format 5.
- **`file.list` and `file.activate`** (field test, finding 5): a document is referred to by its number in the list of open documents, which is what `file.new` and `file.open` already returned, or by its path. Numbers shift when a document before it is closed; a stable document id was not introduced.

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
| D-open-9 | **Default units and tolerance** of a new document | mm / deg / 1e-6 mm; other | M2 | mm, degrees, 1e-6 mm linear, 1e-9 rad angular |
| D-open-10 | **Compression crate for 3MF/`.3dcraft`** (DEFLATE, zstd) | Permissive pure-Rust crates; write our own | M2/M3 | Use permissive pure-Rust crates: not modelling code |
| D-open-11 | **Coordinate convention** | Z up (CAD/3D-printing convention); Y up | M2 | Z up, right-handed |
