# Programs on 3DCraft

*Build a program of your own on 3DCraft's engine: drive it from any language over `serve` or the
window's control channel, give it to an AI agent over MCP, or link the Rust library.*

Everything 3DCraft does is a command. So a gear generator, a furniture configurator or a CAD interface
with buttons can each be a separate program that sends 3DCraft's commands. 3DCraft is open source under
the MIT licence: you may build on it, also in a program you do not publish, as long as you keep its
copyright and licence notice with it.

Before you write a program, see whether a script does what you need: a list of commands in a file, run by
`craft3d-cli run` ([The command line and scripts](command-line.md)). And a design that others should set
by its sizes may only need to be a Jig ([Making Jigs](making-jigs.md)).

## Four ways in

| Way | For | It speaks |
|---|---|---|
| `craft3d-cli serve` | A program in any language, with no window | JSON lines on standard input and output |
| The window's control channel | A program that drives the window a person is watching | JSON lines over a local network connection, with a token |
| `craft3d-cli mcp` | An AI agent ([Install › Connect an AI agent](install.md#connect-an-ai-agent-over-mcp)) | MCP over standard input and output |
| The Rust library `craft3d-engine` | A program written in Rust | Function calls |

All four run the same commands, with the same parameters and the same results. The full description of
the protocols, every method and every limit is in [`control-protocol.md`](../control-protocol.md).

## `serve`: JSON lines

`craft3d-cli serve` reads one request per line on its standard input and writes one reply per line on its
standard output. Each request has an `id`, a `method` and its `params`; the reply carries the same `id`:

```text
{"id": 1, "method": "engine.execute", "params": {"command": "file.new", "params": {"units": "mm"}}}
{"id": 1, "ok": true, "result": {"document": 0, "units": "mm"}, "ms": 0.1}
{"id": 2, "method": "engine.execute", "params": {"command": "feature.block", "params": {"size": ["40 mm", "20 mm", "10 mm"], "name": "Block"}}}
{"id": 2, "ok": true, "result": {"id": "F1", "name": "Block", "status": "ok"}, "ms": 0.3}
{"id": 3, "method": "engine.execute", "params": {"command": "feature.block", "params": {"sise": ["40 mm", "20 mm", "10 mm"]}}}
{"id": 3, "ok": false, "error": "feature.block: unknown parameter `sise`; its parameters are: size, position, axis, angle, name", "ms": 0.0}
```

The methods are few, because everything goes through `engine.execute` with a command:

| Method | Does |
|---|---|
| `engine.execute` | Runs one command: `{"command": "feature.block", "params": {...}}`. The reply has `result` or `error`, and `ms`, how long it took |
| `engine.commands` | The commands with their parameters and summaries (`filter`, `id`, `brief` as for `command_list`) |
| `batch` | Runs up to 256 commands in order: `{"steps": [{"command": …, "params": …}, …]}` |
| `methods` | The methods, their parameters and the limits |

From a shell, three requests at once:

```sh
printf '%s\n' \
  '{"id":1,"method":"engine.execute","params":{"command":"file.new","params":{"units":"mm"}}}' \
  '{"id":2,"method":"batch","params":{"steps":[{"command":"feature.block","params":{"size":["40 mm","20 mm","10 mm"]}},{"command":"document.measure"}]}}' \
  '{"id":3,"method":"engine.execute","params":{"command":"file.export","params":{"path":"box.step"}}}' | \
  craft3d-cli serve --automation-write-root /work/project
```

A program may read and write files only beneath the folders `serve` was started with:
`--automation-read-root DIR` and `--automation-write-root DIR`. Without them it reads and writes none.
`--recovery-dir DIR` lets it autosave and recover documents; without it, `serve` keeps no autosave.

## The window's control channel

A program can also drive the window a person is watching: open a file, set a Jig's parameter, fly the
camera, take a picture of the window. It speaks the same JSON lines, over a connection to the window on
the same computer, plus the window's own methods (`ui.get`, `ui.set`, `ui.fit`, `ui.screenshot`, …).

The connection is guarded:

- the window listens only while **Allow AI agents** is on, or when it was started with `--control PORT`;
- it listens only on the computer itself (`127.0.0.1`), never on the network;
- the first request on every connection must prove that it knows the window's token, which is kept in a
  file only the person can read. A script that starts a window of its own gives it a token file:

  ```sh
  craft3d --control 7951 --control-token-file /private/path/script.token \
      --automation-read-root /work/project --automation-write-root /work/project --recovery-dir none
  ```

  and opens with `{"id": "auth", "method": "auth", "params": {"token": "<the file's 64 characters>",
  "client": "my script"}}`. The name you give as `client` is what the window's agent pill shows.

`--recovery-dir none` keeps a window a script starts out of the person's recovery bar: what it leaves
behind is not offered for recovery at the next start. A program that drives the person's own window
should not need the token at all: `craft3d-cli mcp --bridge` finds the window and proves it holds the
token for you. [`control-protocol.md`](../control-protocol.md) › *Authentication* describes the token and
the challenge that keeps it off the connection.

## MCP

`craft3d-cli mcp` is the MCP server: an AI agent's tool starts it, and the agent gets the tools listed in
[Install › What the agent gets](install.md#what-the-agent-gets). With `--bridge` it passes every message
to the window the person allowed agents in; without it, it keeps a document of its own (headless). Any
program that speaks MCP can use it the same way an agent does.

## The Rust library

The engine is the crate `craft3d-engine`. A `Session` holds the open documents and runs commands by id,
with JSON parameters, and answers with JSON (here the session is called `cad`):

```rust
use craft3d_engine::{Session, Workspace};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cad = Session::new(Workspace::unrestricted());
    cad.execute("file.new", &json!({"units": "mm"}))?;
    cad.execute("variable.set", &json!({"name": "width", "expr": "60 mm"}))?;
    cad.execute("feature.block", &json!({"size": ["width", "40 mm", "8 mm"], "name": "Plate"}))?;
    let measured = cad.execute("document.measure", &json!({"feature": "Plate"}))?;
    println!("volume: {} mm³", measured["volume"]);
    cad.execute("file.export", &json!({"path": "plate.step"}))?;
    Ok(())
}
```

Add it to your program's `Cargo.toml` from the source repository, at the version you build against:

```toml
[dependencies]
craft3d-engine = { git = "https://github.com/johancoppens/3dcraft", tag = "v0.1.0" }
serde_json = "1"
```

<!-- CHECK: craft3d-engine compiles as a git dependency at the tag v0.1.0, in a program outside the workspace -->

Text in a built-in font needs the font files, which are not compiled into the engine: point the
environment variable `CRAFT3D_FONTS` at a folder that holds them (an installed 3DCraft has one, beside the
programs).

`Workspace::unrestricted()` lets commands read and write anywhere the program may. To keep them to one
folder, give `Workspace { read: Access::Root(dir.clone()), write: Access::Root(dir) }` instead (`Access`
is in `craft3d_engine` too). A command that cannot do what was asked returns `Err` with a message that
names the command and the parameter; it never panics. `Session::commands()` lists every command with its
parameters and summary.

The engine is the same code the window and `craft3d-cli` run, and so is every rule this guide describes.
How it is built, and the rules a change to it must follow, are in the source repository's developer
documentation, starting at [`docs/README.md`](../README.md).

## What a command answers

Every command answers with JSON. These rules hold for all four ways in; where one way differs, the rule
says so.

- **A command that fails** changes nothing and returns an error that starts with the command and the
  parameter, such as *feature.boolean: there is no feature named `Sheet`; features: Block*. Over MCP it
  is a tool result with `isError: true`; over JSON lines, `"ok": false` with `error`. In a batch, the
  steps before it stay applied: a batch is not atomic, and `edit.undo` takes back one step at a time.
- **A feature that fails** is different: the command succeeded (the feature is in the history), but the
  feature could not be built. Its result has `"status": "failed"` and an `error` that names the
  parameter, the entities and often the largest value that would fit ("the largest radius that fits there
  is below 26 mm"). Features built from it fail too, each with `causedBy`: the one to fix
  ([How 3DCraft thinks › Failed features](concepts.md#failed-features)).
- **A batch is judged at the end.** `command_batch` (and the method `batch`) lists under `featuresFailed`
  the features that still fail when the batch ends and that one of its steps broke. Over MCP the tool
  result is then an error (`isError: true`); over JSON lines (`serve`, the window's channel) the reply
  stays `"ok": true`: read `featuresFailed` in its result. A break that a later step repairs is only
  noted. A failed check is not listed there: read `assert.list`
  or `document.check`.
- **`failed`**, in the result of `variable.set` and of every edit that can break other features
  (`variable.delete` with `force`, `feature.edit`, `feature.suppress`, `feature.when`, `feature.move`,
  `feature.delete`, `history.rollback`, `param.set`, `table.set`, and every `sketch.*` command that
  changes an existing sketch): the features the change made fail, each `{feature, id, error}` (and
  `causedBy`), left out when there are none. "Make the plate 2 mm thick" that breaks a fillet says so in
  its own result, and so does deleting a line that leaves an extruded profile open.
- **`assertionSummary`**, in the result of every feature command and of `variable.set`, `param.set` and
  `document.check` when the document has checks: `{count, passed, broken, unproven, pending, inactive}`. A broken check never stops a
  feature from being built; it tells you the design no longer meets what was asked. An edit's result waits
  up to two seconds for the checks it touched and counts the rest under `pending`; `"wait": true` waits
  for all of them ([Measuring and assertions › Assertions](measure.md#assertions)).
- **`document.inspect`** (the MCP tool `doc_inspect`) is the whole state: variables with their values,
  every feature with its status and parameters, every body with `volume`, `area`, `centroid`, `bbox` and
  its faces by name (`faceNames`), and the checks. `{"measures": false}` is the quick version for a large
  model; `{"wait": true}` waits for checks still being measured.
- **`document.check`** lists every problem in one place: failed variables and features, invalid bodies,
  broken checks (then `ok` is false), and as `warnings` the checks not proven and other things worth a
  look. `{"deep": true}` checks more (bodies that pass through themselves, every boolean against its
  operands) and takes longer.
- **`session.report`** gives a bug report of the last failed command: the version, the commit it was
  built from, the platform, and the command with its parameters and error
  ([Troubleshooting › Reporting a problem](troubleshooting.md#reporting-a-problem)).

**Next:** [Troubleshooting](troubleshooting.md).
