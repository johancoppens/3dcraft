# 3DCraft design: The engine and its clients

The engine (`craft3d-engine`): `Session`, the command registry, undo, scenes, autosave and recovery; and its clients: the window's engine thread, the CLI, the control server and MCP (`craft3d-automation`). §7 of the map ([`../architecture.md`](../architecture.md)).

## Method

**Session.** A `Session` holds the open documents (one active), each with its undo history (`ops::History`: `Arc<Document>` snapshots), its regeneration cache and its scenes, plus the read and write roots that every file path must lie beneath. It runs commands synchronously: `Session::execute(id, params)` → JSON result or `CommandError`. An edit applies to a copy of the document; it becomes one labelled undo step only if it succeeds and changes something, and a failed command changes nothing. A panic inside a command is caught, reported as an error, and the document is kept (autosaved at once).

**Commands.** A `CommandSpec` is an id, a label, a parameter string in the notation agents read, a summary, whether it needs a document, and `run`. Specs live in `commands.rs` and in one `<area>_cmds.rs` module per area, listed in `Session::commands`; there are 71 today, in the families `file`, `recovery`, `edit`, `variable`, `sketch`, `datum`, `feature`, `body`, `history`, `document` and `view`. `document.inspect`, `document.edges`, `document.check`, `document.measure` and `document.interference` (clashes between bodies, read-only) let an agent verify from JSON what it did; `view.render`, `view.pick` and `view.replay` let it look. `panic_hunt` runs every command with adversarial parameters in four document states.

**Scenes.** `Session::scene_with(SceneQuality)` gives meshes, edge polylines, sketches, datums, names and colours per document revision ([`tessellator.md`](tessellator.md) for the qualities); scenes of every open document and of the states its undo history holds are kept within a memory budget, so an undo or a switch of document shows one again without meshing. The mesh of a body is kept by the body (`meshes.rs`: regeneration hands on the same `Arc<Body>` for what an edit did not touch) within a second budget of the same size, which the kept scenes' meshes overlap (the memory is at most the two budgets together), so a scene built after an edit meshes only the bodies the edit changed, and the scenes of several states share the meshes of their common bodies ([`tessellator.md`](tessellator.md) › *A scene rebuild meshes only what changed*). A host that draws while the display scene is built gets a quick first picture and builds the display scene elsewhere (`refinement`, `accept_refined`). Mass and inertia are measured once per body, when asked.

**Recovery.** A session given a recovery directory notes after every command which documents changed and writes them with the `.3dcraft` writer on one thread of its own, at most once per interval; a clean end removes them, and the next session offers what a killed one left (`recovery.*`).

**Clients.** The window (`apps/craft3d`) hands the session to its engine thread with one task at a time (a request, a file from a dialog, or only the scene to show) and draws the last scene meanwhile with the progress reported; its control listener (loopback TCP, a token) answers the `serve` protocol plus `ui.*`. `craft3d-cli` runs commands headless (`run`, `info`, `render`, `convert`, `replay`, `commands`) and serves them (`serve`, `mcp`, and `mcp --bridge` to a running window). The protocol is in [`../control-protocol.md`](../control-protocol.md). No client adds behaviour of its own: the CLI and the automation server can do everything the window does.

## Findings and history

> Moved verbatim from `docs/architecture.md` on 2026-10-15. Section numbers (§4.1, §5.3, …) in the text are those of that file before the split; [README.md](README.md) says where each one is now.

### From *Current state (2026-10-12)*

- **Undo:** `ops::History` keeps snapshots (`Arc<Document>`) rather than reversible ops: an edit is applied to a copy and becomes one labelled step only if it succeeds and changes something.

- **Engine:** `engine::Session` holds open documents and runs 72 commands (64 when this was written) (`file.*`, `recovery.*`, `edit.*`, `variable.*`, `feature.*`, `sketch.*`, `datum.*`, `history.rollback`, `document.inspect`, `document.check`, `document.measure`, `view.render`, `view.pick`, `view.replay`) by id with JSON parameters. Edits are atomic undo steps; a failed command changes nothing; a panic inside a command is caught and reported. File access goes through explicit read and write roots. `tests/panic_hunt.rs` runs every command with adversarial parameters in four document states. Deviations from the plan: synchronous (no job pool, no events, no `arc-swap` snapshots), no parameter schemas beyond the documented params string, no `enabled` predicate beyond "needs a document". **Autosave and crash recovery** (2026-10-13, `engine/src/recovery.rs`, `recovery_store.rs`, `recovery_cmds.rs`; not in a browser): a session that its host gave a recovery directory notes after every command which documents changed (an `Arc` clone of the document, under a microsecond) and writes them with the `.3dcraft` writer on the one thread the engine starts, at most once per interval per document (30 s) and at once after a caught panic; a 300-feature document of 1.8 MB takes 40 to 90 ms there and nothing on the command's thread. The host ends it on a clean quit (`Session::end_recovery`), which removes the files; a killed process leaves them, and the next session's scan offers them through `recovery.list`, `recovery.open` and `recovery.discard`. The layout is in [`file-formats.md`](../file-formats.md).

### 8. Engine (`craft3d-engine`)

#### 8.1 `Session`: commands in, events out

*(Planned, not built: `dispatch` with a `JobHandle`, `snapshot`, `events`, `DocId`, `CommandInvocation` with a schema. As built: `Session::execute(id, params)`, synchronous; see Method.)*

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

#### 8.2 `CommandRegistry`

Each command registers `id` (`"feature.extrude"`), `label`, an optional `menu_path` and shortcut (unused in phase 1), a parameter schema with a one-line params doc agents can read, `enabled(&DocSnapshot) -> bool` with a reason when disabled, and `run`. *(As built: `id`, `label`, `params` (the one-line doc), `summary`, `needs_document` and `run`; menu paths, schemas and an `enabled` predicate are planned.)* `run` returns a JSON result and records exactly **one** history step.

Command families (ids are settled when each is implemented, and are stable afterwards):

| Family | Examples |
|---|---|
| `file.*` | `file.new`, `file.open`, `file.save`, `file.close`, `file.list`, `file.activate`, `file.export` (the format from the file name or `format`: STEP or a mesh format for every body, DXF or SVG for the outline of a `sketch` or of a flat `face`) |
| `edit.*` | `edit.undo`, `edit.redo` |
| `variable.*` | `variable.set`, `variable.delete`, `variable.rename` |
| `sketch.*` | `sketch.create`, `sketch.edit`, `sketch.delete`, `sketch.inspect`, `sketch.point`, `sketch.line`, `sketch.polyline`, `sketch.circle`, `sketch.arc`, `sketch.spline`, `sketch.path`, `sketch.constrain` |
| `datum.*` | `datum.plane`, `datum.axis`, `datum.point`, `datum.helix` (reference geometry, §4.1 › *Reference geometry*), `datum.references` (what a body offers a datum, with the names) |
| `feature.*` | `feature.block`, `feature.cylinder`, `feature.sphere`, `feature.cone`, `feature.torus`, `feature.extrude`, `feature.revolve`, `feature.loft`, `feature.sweep`, `feature.thread`, `feature.boolean`, `feature.transform`, `feature.mirror`, `feature.pattern`, `feature.chamfer` and `feature.fillet` (built 2026-10-14, `engine/src/blend_cmds.rs`), `feature.offsetFace`, `feature.moveFace`, `feature.draft`, `feature.shell`, `feature.thicken`, `feature.deleteFace`, `feature.edit`, `feature.rename`, `feature.suppress`, `feature.move`, `feature.delete` |
| `history.*` | `history.rollback` |
| `body.*` | `body.appearance`, `body.material` |
| `recovery.*` | `recovery.list`, `recovery.open`, `recovery.discard` |
| `document.*` | `document.inspect`, `document.measure` (volume, area, centroid, bounding box, distances), `document.check` (validity report), `document.edges` (edges and vertices with their names), `document.interference` (clear, touching or overlapping bodies, read-only) |
| `view.*` | `view.render` (PNG from a named or explicit camera, CPU), `view.pick`, `view.replay` |

**Inspection is a first-class command.** `document.inspect` returns the feature tree with per-feature status, variables with values, sketches with their degrees of freedom, and bodies with face/edge counts, persistent names and mass properties. An agent must be able to verify what it did from JSON alone, and use `view.render` when it needs to look.

#### 8.3 Operations and history (`ops`)

*(Planned; as built `ops::History` keeps a snapshot, `Arc<Document>`, per labelled step, and there are no `Op`s.)* Every mutation of a document is an `Op` (`SetVariable`, `AddFeature`, `EditFeature`, `ReorderFeature`, `AddConstraint`, …). A command produces a labelled `Transaction` of ops. Undo history stores transactions; because documents are persistent data, a "before" state costs only what changed. Undo history (session-only, what the user did) is distinct from the **design history** (the feature list, saved in the file).

#### 8.4 Runtime model

```text
 Client (UI / CLI / control / MCP) ──dispatch()──► Engine thread (owns mutable docs, applies ops, publishes snapshots)
        ▲                                               │ spawns
        │ events (changed, progress, failed)            ▼
        │                                   Job pool (planned): regeneration, tessellation, export, autosave
        │                                   read snapshots, return results
 Viewport: render(snapshot) on GPU or CPU
```

*(Planned, not built:)* Every job takes a `CancelToken`. Services injected into `Session`: file system (native fs / in-memory for tests / browser later), clock, notifier.

*As built (2026-10-14):* the desktop app has the engine thread of the picture, without the snapshots: `apps/craft3d/src/worker.rs` takes the `Session` itself with one task at a time (a request line, a file chosen in a dialog, or only the scene the window shows), runs it, builds the scene at the shell's display quality, and hands the session back; the window draws the scene of the last sync meanwhile and the progress the task reports. One task at a time keeps requests in order. A panic in a task is caught there, autosaved and reported. On wasm (no app yet) the shell keeps its synchronous path: `Shell::ui` with the session, as tests and examples use it. The scenes of every open document are kept, and of the earlier states its undo history holds, within a memory budget (1.5 GiB by default, the least recently shown first out, the active document's never), so `file.activate` or an undo shows one again without meshing it, in well under a millisecond (a scene's meshes are shared by `Arc`); the display scene is built beside the engine thread while the window shows a quick first picture (`apps/craft3d/src/refine.rs`, §6 › *Coarse first, then sharp*), so a command is never queued behind it; a body's area, volume and centroid are measured once per body (`engine/src/mass.rs`, by the body's `Arc`), and its faces are measured on several threads and summed in face order (`topo::mass_properties`), which is what made opening a model with spline faces slow (the display tolerance is a fraction of the model's size, and the size is the square root of its area). *2026-10-19:* the meshes and the facts of a body are kept by its `Arc` the same way (`engine/src/meshes.rs`), and the display rule's and the first picture's tolerances stand on a ladder of rungs 9 % apart, so a rollback step on the developer's pier (307 bodies) went from 2.6–3.5 s to 75–128 ms ([`tessellator.md`](tessellator.md) › *A scene rebuild meshes only what changed*, with why an empty window held 1.9 GB after the pier: glibc keeps what the engine freed).

#### 8.5 The UI-swap seam

A frontend needs four things from the core: `Session` (commands in; events *(planned)*), the `CommandRegistry` (metadata), *(planned)* snapshots with view-models (feature tree, variable table, inspector as plain structs), and a renderer it can host (`gpu::Onscreen` on the host's own device into a texture it shows, or `render` into an RGBA buffer; the egui shell today hosts the first as `ui_egui::direct::Direct` when the window lends it the device, and otherwise `ui_egui::backend::BodyRenderer`, which gives it a `Frame` from the GPU read back or from the CPU). Interactive tools (sketching with the mouse, dragging a face) come after phase 1 and follow PhotoCraft's pattern: pointer events in, ops and overlay primitives out, so the control channel can replay them.

**Litmus test:** `craft3d-cli` and the automation server can do everything the GUI does.

---

### 10. Automation

Detail in [`control-protocol.md`](../control-protocol.md).

- **CLI** (`craft3d-cli`): `convert`, `info`, `run` (a list of commands against a document), `commands` (the registry), `render`, `serve` (JSON lines on stdio or loopback TCP) and `mcp`.
- **Control channel:** the desktop app listens on loopback with a token; agents send `engine.execute`, `ui.inspect`, `ui.screenshot`. This is also how the developer watches an agent work in a live window.
- **MCP** (`automation`): `doc_*`, `command_list`, `command_run`, `command_batch`, `doc_render_preview`, generated from the registry; file access limited to explicit read and write roots.
- **Scripts:** a recorded session is `Vec<CommandInvocation>`; replaying it rebuilds the model. This is the natural format for agent-written test scenarios.

### Inspecting a model of hundreds of bodies, and closing the window (2026-10-21)

*Measures remembered for every body.* The measure of a body is remembered by the body (`engine::mass`), but the store held 256 bodies and forgot the oldest first: the Garabit pier (307 bodies, 57,000 faces) measured every body again for each `document.inspect` (5.4 s after a `document.measure`, and 5.2 s again; now 1.1 s, of which the boxes' meshes most, then 0.1 s). It now holds 16,384 (`MAX_REMEMBERED`), keyed by the address of the body's `Arc` in a sorted map (a `Weak` keeps the address from being reused while it is remembered), and forgets bodies that are gone every 64 bodies it takes, and the oldest quarter at once when full (1,024 in a browser). The box `document.inspect` lists per body (`bbox`, from a mesh at the automatic tolerance) is remembered the same way, per body and tolerance (`mesh_box_of`). `document.inspect` measures the bodies with a material first, together with their inertia, and only then the others for the automatic tolerance, so no body is integrated twice. *`document.inspect {measures: false}`* lists everything but what takes measuring (volume, area, centroid, mass, inertia, bbox): it measures nothing, which a test counts (`inspect_without_measures_measures_nothing`), and its rest is the full answer's to the byte. On the valley just regenerated: 0.09 s, against 12.1 s for the full first inspect. The slowest single face is faster too ([`topology.md`](topology.md) › *One slow face on every thread*).

*The window stays responsive.* Commands, `ui.get` while one runs, and the scene a document needs already run on the window's engine thread (2026-10-14, [`control-protocol.md`](../control-protocol.md) › *The window keeps answering*); a long `document.measure` shows its work over the viewport while the camera, the tree and picking go on. Checked on a window of its own (release, the valley opened over the channel, `document.measure` on one connection and `ui.get` every quarter second on another): 37 answers while it ran for 11.9 s, the slowest in 0.15 s. What the 50 to 120 s were is the measure itself, on one thread, and measured again for each inspect of a model over 256 bodies.

*Closing the window asks first* (roadmap R1). Before, the close button ended the application and removed the recovery files of documents it had never saved. Now the shell turns the close (`Shell::request_window_close`, from `ViewportCommand::CancelClose` in the window) into the question a tab's close button asks, one document with unsaved changes after the other, each answer done as the tab's (`file.save` then `file.close`, the save dialog first for a document without a file, or `file.close {discard}`), and the desk takes the next step whenever the session is home and nothing is pending (`Shell::advance_quit`). Pressed while a command runs, nothing is decided on the tabs from before it: the window waits for the command and then asks about what it left unsaved (review, 2026-10-21). `ui.quit {save: "save"}` saves through the automation write root like a request's `file.save` and refuses, with nothing saved, a document whose file lies outside it: only the person's own Save writes with the person's access (review, 2026-10-21). When an answer closed nothing (a failed save, a cancelled save dialog) or the person pressed Cancel, the window stays. The application ends once no document asks; recovery files are then removed only when nothing unsaved is left, or when `ui.quit` dropped it on purpose: a document changed by work that finished while the window closed keeps its autosave, as after a crash. Over the channel `ui.quit {save}`: `discard` (the default, as before), `ask` (as the close button), `save` (every document saved to its file on the engine thread, refused while one was never saved). Held to `apps/craft3d/src/tabs.rs` (save, Don't Save, Cancel, a never-saved document through the dialog, a cancelled dialog, several tabs, the channel's three forms and bad params) and `apps/craft3d/tests/killed_window.rs` (a killed window still recovers; `ui.quit` still leaves nothing).
