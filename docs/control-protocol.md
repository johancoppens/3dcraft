# Control protocol and MCP

> **Last updated:** 2026-10-12 · **Status:** the headless JSON-lines server (`craft3d-cli serve`), the MCP server (`craft3d-cli mcp`), the desktop control server (`craft3d --control`) and the MCP bridge to a running window (`craft3d-cli mcp --bridge`) exist. The sections below describe the target; **what is implemented today is listed first and wins where they differ.**

## Implemented today (2026-10-12)

**JSON lines** (`craft3d-cli serve`): the envelope below, on stdin/stdout only (no TCP, so no auth). Methods: `engine.execute {command, params?}`, `engine.commands {filter?}` → `{"commands": [...]}`, `batch {steps: [{command, params?}], stopOnError?}` → `{completed, failed, results}`, `methods`. Everything else goes through `engine.execute` with a command id: there are no `doc.*`, `session.*` or `jobs.*` methods, no `wait`, and batch steps are commands only. Unknown keys are refused, also at the top level of a request. A batch is checked as a whole before its first step runs.

**MCP** (`craft3d-cli mcp`): `initialize`, `ping`, `tools/list`, `tools/call`; protocol versions 2025-11-25, 2025-06-18, 2025-03-26 and 2024-11-05. Tools: `command_list`, `command_run`, `command_batch`, `doc_inspect` (returns `{"document": null}` when nothing is open), `doc_render_preview` (an image block plus a text block). No resources, no `doc_new`/`doc_open`/`doc_save`/`doc_export` tools (use `command_run` with `file.new`, `file.open`, `file.save`, `file.export`), no bridge mode. Tested against the specification, not yet against a real MCP client.

**Desktop control** (`craft3d --control PORT --control-token-file FILE [--automation-read-root DIR] [--automation-write-root DIR]`): the same envelope and methods as `craft3d-cli serve`, on a TCP listener bound to 127.0.0.1, executed on the document the window shows. The first request of every connection must be `auth {token}`; a failed attempt closes the connection, a connection silent for 30 seconds is closed, 16 connections at most. The token file is created (mode 0600 on Unix) when it does not exist. Added methods: `ui.get`, `ui.set {view?, camera?, panels?, selectedFeature?, notice?}`, `ui.fit {view?}`, `ui.screenshot {path?, width?, height?, scale?}` (a PNG of the viewport as framed: the CPU reference render plus the overlay of body edges, sketches and axes, without the panels), `ui.notice {text}`, `ui.quit`, and `mcp.message {line}` (below). Not there: `ui.inspect` (it is `ui.get`), `ui.focus`, display modes, themes.

The window makes its sharp image on a thread of its own and shows a quick preview until it is ready; `ui.get` says which is on screen (`render.sharp`). The viewport is drawn by the wgpu renderer on the window's own device when the machine has a graphics adapter (directly into the texture that is shown, multisampled, every frame sharp: then `render.sharp` is always true), by the wgpu renderer on a device of its own, read back, when that direct path cannot be set up, and by the CPU reference renderer otherwise, or when the window was started with `--renderer cpu` (`CRAFT3D_RENDERER=cpu`), or after the GPU failed: `ui.get` reports the renderer of the image on screen (`render.renderer`, `gpu` or `cpu`, with `render.ssaa` and `render.msaa`, the samples per pixel) and, as `renderer {backend, adapter, fallback, direct, msaa}`, which one the viewport uses, the adapter's name, why it is not the direct GPU path although that was wanted (`null` otherwise), whether it draws directly on the window's device, and with how many samples per pixel. `ui.screenshot` does not depend on any of that: it renders its own picture of the state as it is when the request is answered (camera, document, selection), at full quality and always with the CPU reference renderer (the oracle the GPU viewport is checked against, docs/architecture.md §7), so it never returns a preview or the image of an earlier state. In the overlay, the part of a body's edge behind a body is left out, and the part of a sketch behind a body is drawn dimmed.

**MCP bridge** (`craft3d-cli mcp --bridge PORT --control-token-file FILE`): an MCP server on stdin/stdout without a session of its own. Each message is passed unchanged to the window's `mcp.message`, answered there by the same MCP code as `craft3d-cli mcp` on the window's document, and the reply is passed back: same tools, same limits, and the viewport shows every change. One short connection per message. `PORT` may be `127.0.0.1:PORT`; an address that is not this machine is refused, because the token is sent as it is. The file roots are the window's. When the window is not running, every request gets a JSON-RPC error (code -32000) that says how to start it, and the bridge keeps serving. The bridge-only tools of the table further down (`ui_screenshot`, …) do not exist: use `doc_render_preview`.

**Commands** (44): `craft3d-cli commands` prints the registry with parameters and summaries; it is the source of truth.

**Limits enforced:** 1 MiB per request line, 8 MiB per reply, 256 steps per batch, previews at most 2048 px on the long side, renders 16 to 4096 px per side. Not enforced yet: the triangle budget for tessellation.

**`document.inspect`** returns the shape shown further down with these differences: feature parameters are under `"params"` with a `"type"` field; a failed feature carries `"error"`; bodies are listed per feature with `"feature"` and `"name"`, and carry a `"bbox"` taken from a tessellation (exact for flat-faced bodies, inside the true box by at most `"bboxTolerance"` for curved ones); `"history"` holds the undo and redo labels as arrays; `"path"`, `"dirty"`, `"ok"` and `"document"` (its index) are added.

Three transports, one command registry:

| Transport | Process | Use |
|---|---|---|
| **Headless server** | `craft3d-cli serve` | Fastest way for a script or agent to make many edits; no window, no GPU |
| **Desktop control** | `craft3d --control <port>` | Drive and screenshot the running app while the developer watches |
| **MCP** | `craft3d-cli mcp [--bridge host:port]` | Agent clients; wraps either of the above |

## Envelope

Each request is one JSON line; each reply is one JSON line with the same `id`:

```json
{"id": 1, "method": "engine.execute", "params": {"command": "document.measure", "params": {}}}
{"id": 1, "ok": true, "result": { "volume": 8000.0, "unit": "mm^3" }}
{"id": 2, "ok": false, "error": "unknown command `feature.extrud`; did you mean `feature.extrude`?"}
```

Errors are sentences an agent can act on: they name the command, the parameter and the entity involved. `params` of a command must be a JSON object (or omitted); anything else is an error in every transport.

## Authentication (TCP only)

Both TCP listeners bind `127.0.0.1` only. Start with a token file so the credential is not on the command line:

```sh
craft3d --control 7878 --control-token-file /private/path/craft3d-control.token \
  --automation-read-root /work/project --automation-write-root /work/project
```

If the file does not exist it is created with a fresh 256-bit token (mode `0600` on Unix). The first request on every connection must authenticate; nothing else is dispatched before it succeeds:

```json
{"id": "auth", "method": "auth", "params": {"token": "<64 hexadecimal characters>"}}
```

Stdio transports need no handshake: access is inherited from the process pipe.

## File access

Automation never gets ambient filesystem access. Paths in requests are relative to, or absolute beneath, an explicit **read root** and **write root** given at launch (`--automation-read-root`, `--automation-write-root`); the two are independent, and omitting one denies that direction. Parent traversal, paths outside the root and link escapes are rejected before any file effect. Interactive file pickers in the desktop app keep normal user-selected access.

How the window's own File menu fits in (`apps/craft3d/src/files.rs`): New, Open, Save and Save As are the commands `file.new`, `file.open` and `file.save`, run by the application for the person at the window. For a file chosen in the system's dialog the application grants that one command access to the directory of the chosen file, in one direction (reading to open, writing to save), and restores the automation roots before anything else runs. Requests are answered on the same thread, one at a time, so no request ever runs under the grant, and there is no method, parameter or tool that asks for it. What follows for a client: a document the person opened from outside the roots is the active document like any other and can be inspected and edited over the channel, but `file.save` without a `path` is refused for it (its own path is outside the write root); save a copy with a `path` beneath the write root. When pointer and key replay arrive (`ui.pointer`, `ui.key`), they must not be able to reach the File menu's dialogs.

## Methods

### Engine (all transports)

| Method | Params |
|---|---|
| `engine.execute` | `{command, params?, wait?}`: run any command by id. `wait: false` starts a long one (regeneration, export) as a background job: `{job, pending}` |
| `engine.commands` | `{filter?}`: the registry with params docs and enablement (`enabled`, `disabledReason`, `needsDocument`) |
| `jobs.list` / `jobs.cancel` | `{}` / `{job?}` |
| `session.list` | open documents and the active index |
| `doc.new` / `doc.open` / `doc.close` / `doc.select` | `{units?, tolerance?}` / `{path}` / `{index?}` / `{index}` |
| `doc.save` | `{path?, index?}`: native `.3dcraft`, atomic |
| `doc.export` | `{path, format?, tolerance?, bodies?, index?}`: by extension: `.step`/`.stp`, `.stl`, `.3mf`, `.obj` |
| `doc.inspect` | `{index?, detail?}`: same JSON as the command `document.inspect` |
| `doc.render` | `{index?, view?, size?, mode?, path?}`: PNG to `path`, else `{mime, base64}`; CPU renderer in headless mode. Today this is the command `view.render` `{path?, width?, height?, view?, projection?, zoom?, box?, mode?, tolerance?}`: `projection` is `perspective` (default) or `orthographic` (lengths across the view to scale); `box` is two opposite corners `[[x, y, z], [x, y, z]]` in document units, framed instead of the whole model and seen from outside the model; `zoom` (0.01 to 1000) magnifies about the centre of the picture. The result repeats `projection`, `zoom` and the `box` that was framed |
| `batch` | `{steps: [{command, params?} \| {method, params?}], stopOnError? (true), terse? (false)}` → `{completed, failed, results}`, or with `terse` `{completed, failed, errors: [{step, error}], last: {step, result}}`. Not a transaction: earlier steps stay applied |
| `methods` | this list |

```sh
printf '%s\n' \
  '{"id":1,"method":"doc.new","params":{"units":"mm"}}' \
  '{"id":2,"method":"batch","params":{"steps":[
      {"command":"feature.primitive.box","params":{"size":["40 mm","20 mm","10 mm"]}},
      {"command":"document.measure"}]}}' \
  '{"id":3,"method":"doc.export","params":{"path":"box.step"}}' | \
  craft3d-cli serve --automation-write-root /work/project
```

### UI (desktop control only)

| Method | Params |
|---|---|
| `ui.inspect` | Full UI state: camera, display mode, panels, selection, window size, renderer (`gpu`/`cpu`) and timings. Today `ui.get`: `renderer {backend, adapter, fallback, direct, msaa}` is the viewport's renderer, `render.renderer` the one that made the image on screen, `render.msaa` its samples per pixel |
| `ui.set` | `{view?, camera?, displayMode?, panels?, selection?, theme?}`: change UI state; an unknown field is an error, checked before anything changes |
| `ui.screenshot` | `{path?, focus?}`: PNG of our own window |
| `ui.notice` | `{text}`: show a one-line caption in the status bar (for narrated demos) |
| `ui.focus` | raise the window |

Pointer and key replay (`ui.pointer`, `ui.key`) arrive with the first interactive tools, after phase 1.

## What `document.inspect` returns

The contract that lets an agent verify its work without a picture:

```json
{
  "units": {"length": "mm", "angle": "deg"},
  "tolerance": {"linear": 1e-6, "angular": 1e-9},
  "variables": [{"name": "width", "expr": "40 mm", "value": 40.0, "dimension": "length"}],
  "features": [
    {"id": "F1", "name": "Sketch1", "kind": "sketch", "status": "ok", "dof": 0, "entities": 4, "constraints": 6},
    {"id": "F2", "name": "Extrude1", "kind": "extrude", "status": "ok", "inputs": ["F1"], "params": {"distance": "10 mm"}},
    {"id": "F3", "name": "Fillet1", "kind": "fillet", "status": "failed",
     "error": "edge reference `F2/side(e3)∩side(e4)` no longer exists after Sketch1 changed"}
  ],
  "rollback": null,
  "bodies": [
    {"id": "B1", "kind": "solid", "valid": true, "faces": 6, "edges": 12, "vertices": 8,
     "volume": 8000.0, "area": 2800.0, "centroid": [20.0, 10.0, 5.0],
     "bbox": [[0, 0, 0], [40, 20, 10]]}
  ],
  "history": {"undo": 3, "redo": 0}
}
```

`detail: "topology"` adds every face, edge and vertex with its persistent name, geometry kind and tolerance. Field names are part of the protocol: changing one is a breaking change recorded in this document.

## MCP

`craft3d-cli mcp` serves MCP on stdio. Tool names use underscores (the dotted names belong to the JSON protocol above).

| Tool | Notes |
|---|---|
| `session_list`, `doc_new`, `doc_open`, `doc_save`, `doc_export`, `doc_select`, `doc_close` | File paths resolve beneath the roots |
| `doc_inspect` | The JSON above; an empty session returns `document: null` |
| `doc_render_preview` | PNG image content; `view`, `max_side` (default 1024, maximum 2048), `mode` (`shaded`, `shadedEdges`, `wireframe`, `ids`) |
| `command_list` | `filter`, `enabled_only` |
| `command_run` | `id`, `params`, `wait` |
| `command_batch` | ordered `steps`, `stop_on_error` (default true), `terse` (default false: only the failures and the last result come back); not atomic |
| `jobs_list`, `jobs_cancel` | Background jobs |
| bridge only: `ui_inspect`, `ui_screenshot`, `ui_set`, `control_call` | Need a running desktop app |

Resources `craft3d://document` and `craft3d://commands` return the same JSON as `doc_inspect` and `command_list`.

Conventions (from PhotoCraft's MCP server): unknown tool arguments return JSON-RPC `-32602` naming the key, before execution; tool failures return `isError: true`; a worker panic becomes a tool error and the session keeps serving; invalid JSON lines get `-32700` with `id: null` and the next line is still processed. Before experiments, duplicate the document (`file.duplicate`) and work on the copy. Recover from a failed batch by inspecting, then `edit.undo` once per applied step.

## Limits

Enforced before dispatch, so a hostile or mistaken request cannot exhaust the process:

- 1 MiB maximum request line; 8 MiB maximum reply
- at most 16 simultaneous connections per listener; 30-second socket timeouts
- at most 256 steps per batch
- previews: longest edge ≤ 2048 px; PNG ≤ 5 MiB
- tessellation for export or preview refuses a tolerance that would exceed a triangle budget (default 20 million), with an error naming the tolerance that would fit

A request over a limit gets an error reply; the session and its documents keep serving.
