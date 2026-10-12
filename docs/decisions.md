# Decisions

> **Last updated:** 2026-10-12

Decisions that shape the project, newest first, each with its reason. An agent that needs something listed under *Open decisions* asks the developer; it does not pick silently. When one is settled, move it up with the date.

## Taken

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
