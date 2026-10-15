# 3DCraft design: The engine and its clients

> Moved verbatim from `docs/architecture.md` on 2026-10-15. Section numbers (§4.1, §5.3, …) in the text are those of that file before the split; [README.md](README.md) says where each one is now.

## From *Current state (2026-10-12)*

- **Undo:** `ops::History` keeps snapshots (`Arc<Document>`) rather than reversible ops: an edit is applied to a copy and becomes one labelled step only if it succeeds and changes something.

## From *Current state (2026-10-12)*

- **Engine:** `engine::Session` holds open documents and runs 64 commands (`file.*`, `recovery.*`, `edit.*`, `variable.*`, `feature.*`, `sketch.*`, `datum.*`, `history.rollback`, `document.inspect`, `document.check`, `document.measure`, `view.render`, `view.pick`, `view.replay`) by id with JSON parameters. Edits are atomic undo steps; a failed command changes nothing; a panic inside a command is caught and reported. File access goes through explicit read and write roots. `tests/panic_hunt.rs` runs every command with adversarial parameters in four document states. Deviations from the plan: synchronous (no job pool, no events, no `arc-swap` snapshots), no parameter schemas beyond the documented params string, no `enabled` predicate beyond "needs a document". **Autosave and crash recovery** (2026-10-13, `engine/src/recovery.rs`, `recovery_store.rs`, `recovery_cmds.rs`; not in a browser): a session that its host gave a recovery directory notes after every command which documents changed (an `Arc` clone of the document, under a microsecond) and writes them with the `.3dcraft` writer on the one thread the engine starts, at most once per interval per document (30 s) and at once after a caught panic; a 300-feature document of 1.8 MB takes 40 to 90 ms there and nothing on the command's thread. The host ends it on a clean quit (`Session::end_recovery`), which removes the files; a killed process leaves them, and the next session's scan offers them through `recovery.list`, `recovery.open` and `recovery.discard`. The layout is in [`file-formats.md`](../file-formats.md).

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
| `file.*` | `file.new`, `file.open`, `file.save`, `file.close`, `file.export` (the format from the file name or `format`: STEP or a mesh format for every body, DXF or SVG for the outline of a `sketch` or of a flat `face`) |
| `edit.*` | `edit.undo`, `edit.redo` |
| `variable.*` | `variable.set`, `variable.delete`, `variable.rename` |
| `sketch.*` | `sketch.create`, `sketch.addLine`, `sketch.addArc`, `sketch.addCircle`, `sketch.constrain`, `sketch.solve` |
| `datum.*` | `datum.plane`, `datum.axis`, `datum.point` (reference geometry, §4.1 › *Reference geometry*), `datum.references` (what a body offers a datum, with the names) |
| `feature.*` | `feature.primitive.box`, `feature.extrude`, `feature.revolve`, `feature.boolean`, `feature.chamfer` and `feature.fillet` (built 2026-10-14, `engine/src/blend_cmds.rs`), `feature.edit`, `feature.suppress`, `feature.delete`, `feature.reorder` |
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

*As built (2026-10-14):* the desktop app has the engine thread of the picture, without the snapshots: `apps/craft3d/src/worker.rs` takes the `Session` itself with one task at a time (a request line, a file chosen in a dialog, or only the scene the window shows), runs it, builds the scene at the shell's display quality, and hands the session back; the window draws the scene of the last sync meanwhile and the progress the task reports. One task at a time keeps requests in order. A panic in a task is caught there, autosaved and reported. On wasm (no app yet) the shell keeps its synchronous path: `Shell::ui` with the session, as tests and examples use it. The scenes of every open document are kept, and of the earlier states its undo history holds, within a memory budget (1.5 GiB by default, the least recently shown first out, the active document's never), so `file.activate` or an undo shows one again without meshing it, in well under a millisecond (a scene's meshes are shared by `Arc`); the display scene is built beside the engine thread while the window shows a quick first picture (`apps/craft3d/src/refine.rs`, §6 › *Coarse first, then sharp*), so a command is never queued behind it; a body's area, volume and centroid are measured once per body (`engine/src/mass.rs`, by the body's `Arc`), and its faces are measured on several threads and summed in face order (`topo::mass_properties`), which is what made opening a model with spline faces slow (the display tolerance is a fraction of the model's size, and the size is the square root of its area).

### 8.5 The UI-swap seam

A frontend needs four things from the core: `Session` (commands in, events out), the `CommandRegistry` (metadata), snapshots with view-models (feature tree, variable table, inspector as plain structs), and a renderer it can host (`gpu::Onscreen` on the host's own device into a texture it shows, or `render` into an RGBA buffer; the egui shell today hosts the first as `ui_egui::direct::Direct` when the window lends it the device, and otherwise `ui_egui::backend::BodyRenderer`, which gives it a `Frame` from the GPU read back or from the CPU). Interactive tools (sketching with the mouse, dragging a face) come after phase 1 and follow PhotoCraft's pattern: pointer events in, ops and overlay primitives out, so the control channel can replay them.

**Litmus test:** `craft3d-cli` and the automation server can do everything the GUI does.

---

## 10. Automation

Detail in [`control-protocol.md`](../control-protocol.md).

- **CLI** (`craft3d-cli`): `convert`, `info`, `run` (a list of commands against a document), `commands` (the registry), `render`, `serve` (JSON lines on stdio or loopback TCP) and `mcp`.
- **Control channel:** the desktop app listens on loopback with a token; agents send `engine.execute`, `ui.inspect`, `ui.screenshot`. This is also how the developer watches an agent work in a live window.
- **MCP** (`automation`): `doc_*`, `command_list`, `command_run`, `command_batch`, `doc_render_preview`, generated from the registry; file access limited to explicit read and write roots.
- **Scripts:** a recorded session is `Vec<CommandInvocation>`; replaying it rebuilds the model. This is the natural format for agent-written test scenarios.
