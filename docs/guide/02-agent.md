# 2. Working with an agent

An agent connected to 3DCraft (chapter 1) builds a model the way a careful draughtsman would: it names the
dimensions as variables, adds features one by one, and reads back what it built. Your part is to say what
you want, to watch, and to check.

## What to ask

Ask for a part the way you would describe it to a colleague, with the numbers that matter and what they
are measured from:

> Make a bolt flange: a disc 80 mm across and 10 mm thick with a 30 mm bore, and six 8 mm bolt holes on a
> 60 mm pitch circle. Drive it by variables, so that I can change the number of holes. Assert its volume.
> Save it as flange.3dcraft and export a STEP file.

Things that help:

- **Name the variables** you want to change later (`n`, `thickness`), and say where the part sits ("the
  corner at the origin", "the bottom on the XY plane"). 3DCraft's axes are X, Y and Z, Z up.
- **Ask for assertions** (chapter 7): "the volume must be …", "the holes must go through", "the wall at
  least 2 mm". An assertion is checked again after every change, by 3DCraft, not by the agent's memory.
- **Ask for a check and a picture** at the end: `document.check` and `doc_render_preview`.
- **Point instead of describing** when it is easier: click a face in the window and say "this face"
  (chapter 3).
- **Change by variables**: "make it 8 holes" is one `variable.set`; everything that depends on `n`
  follows.

## A full session

This is what an agent sent for the request above, and what came back (results shortened; the numbers are
real). It ran headless, `craft3d-cli mcp --automation-read-root work --automation-write-root work`.

**1. Learn the commands.** `command_list` with a filter, for the commands it needs; the parameters come
with each:

```text
command_list {"filter": "pattern"}
-> {"commands": [{"id": "sketch.pattern", ...}, {"id": "feature.pattern",
     "params": "{\"source\": id or name, \"count\": expr, \"step\": [expr, expr, expr] or, with angle, expr, \"angle\": expr, ...}",
     "summary": "...", "enabled": false, "disabledReason": "no document is open; run file.new or file.open first"}]}
```

**2. A document, then the variables, an assertion and the features in one batch.** The assertion is
written first: it names `Flange` before that exists, is in error until it does, and passes once built.

```text
command_run {"id": "file.new", "params": {"units": "mm"}}
-> {"document": 0, "units": "mm"}

command_batch {"terse": true, "steps": [
  {"id": "variable.set", "params": {"name": "flange_d", "expr": "80 mm"}},
  {"id": "variable.set", "params": {"name": "thickness", "expr": "10 mm"}},
  {"id": "variable.set", "params": {"name": "bore_d", "expr": "30 mm"}},
  {"id": "variable.set", "params": {"name": "pcd", "expr": "60 mm"}},
  {"id": "variable.set", "params": {"name": "hole_d", "expr": "8 mm"}},
  {"id": "variable.set", "params": {"name": "n", "expr": "6"}},
  {"id": "assert.add", "params": {"name": "volume", "measure": "volume", "of": "Flange",
      "eq": "pi * ((flange_d / 2)^2 - (bore_d / 2)^2 - n * (hole_d / 2)^2) * thickness"}},
  {"id": "feature.cylinder", "params": {"radius": "flange_d / 2", "height": "thickness", "name": "Disc"}},
  {"id": "feature.cylinder", "params": {"radius": "bore_d / 2", "height": "thickness + 2 mm",
      "position": [0, 0, "-1 mm"], "name": "Bore"}},
  {"id": "feature.boolean", "params": {"op": "subtract", "target": "Disc", "tool": "Bore", "name": "Bored"}},
  {"id": "feature.cylinder", "params": {"radius": "hole_d / 2", "height": "thickness + 2 mm",
      "position": ["pcd / 2", 0, "-1 mm"], "name": "BoltHole"}},
  {"id": "feature.pattern", "params": {"source": "BoltHole", "count": "n", "angle": "360 deg / n", "name": "BoltHoles"}},
  {"id": "feature.boolean", "params": {"op": "subtract", "target": "Bored", "tool": "BoltHoles", "name": "Flange"}}]}
-> {"completed": 13, "failed": 0, "errors": [],
    "last": {"step": 12, "result": {"id": "F6", "name": "Flange", "status": "ok",
             "assertions": {"count": 1, "passed": 1, "broken": [], "unproven": [], "pending": 0}}}}
```

**3. Check.** `assert.list` says what was measured against what was expected:

```text
command_run {"id": "assert.list", "params": {}}
-> {"ok": true, "count": 1, "passed": 1, "failed": 0, "notProven": 0, "errors": 0,
    "assertions": [{"name": "volume", "status": "passed", "measured": 40180.97003941347,
                    "expected": "40180.97 mm^3 ± 0.04018097", ...}]}
```

**4. Find the holes and what drives them.** `select.query` with `context` lists every hole with its
radius, its axis and the edit that changes it:

```text
command_run {"id": "select.query", "params": {"kind": "face", "where": {"hole": true}, "context": true}}
-> {"count": 7, "items": [
     {"name": "F6:copy(0)/F3:copy(1)/F2:face(0)", "origin": {"name": "Bore", "type": "cylinder"},
      "surface": {"type": "cylinder", "radius": 15.0, "hole": true, ...},
      "drivers": [{"param": "radius", "expr": "bore_d / 2",
                   "edit": {"command": "feature.edit", "params": {"feature": "F2", "radius": "bore_d / 2"}}}]},
     {"name": "F6:copy(1)/F4:face(0)", "copyOf": [{"name": "BoltHoles", "instance": 0}], ...}, ...]}
```

**5. Change it.** Eight holes instead of six. The result of the edit carries the assertions, still
passing because the expected volume is written in `n` too:

```text
command_run {"id": "variable.set", "params": {"name": "n", "expr": "8"}}
-> {"name": "n", "value": 8.0, "assertions": {"count": 1, "passed": 1, "broken": [], ...}}

command_run {"id": "measure.thickness", "params": {"of": "Flange"}}
-> {"thickness": 6.0, "atLeast": 5.752661301804291,
    "at": {"face": "F6:copy(1)/F4:face(0)", "point": [34.0, 0.0, 10.0]},
    "to": {"face": "F6:copy(0)/F1:face(0)", "point": [40.0, 0.0, 10.0]}}
```

The thinnest wall is 6 mm, between a bolt hole and the rim.

**6. A refusal, and undo.** Asked for a 20 mm round on the rim of a 10 mm disc, the fillet is added but
fails; `command_run` does not report an error, so the agent reads `status`:

```text
command_run {"id": "feature.fillet", "params": {"edges": [["F1:face(0)", "F1:face(2)"]], "radius": "20 mm", "name": "Round"}}
-> {"id": "F7", "name": "Round", "status": "failed",
    "error": "radius: not built for edges.0 (between F6:copy(0)/F1:face(0) and F6:copy(0)/F1:face(2)): with a radius of 20 mm the fillet face would be a torus that reaches its own axis ..., which is not built yet; so it is for every radius from 20 mm up to 40 mm ..."}

command_run {"id": "edit.undo", "params": {}}
-> {"undone": "Add fillet"}
```

**7. Look, save, export.**

```text
command_run {"id": "document.check", "params": {}}
-> {"ok": true, "problems": [], "bodiesChecked": 1, "assertions": {"passed": 1, ...}}
doc_render_preview {"view": "iso"}
-> (a PNG image) and {"coverage": ..., "triangles": ..., "bodies": [...]}
command_run {"id": "file.save", "params": {"path": "flange.3dcraft"}}
-> {"path": "flange.3dcraft"}
command_run {"id": "file.export", "params": {"path": "flange.step"}}
-> {"format": "step", "bodies": 1, "solids": 1, "faces": 12, "unit": "mm", ...}
```

The [flange example](../examples/flange.json) is the same part with a chamfer and a named set of bolt
holes.

## Reading what the agent gets back

Every command answers with JSON. The parts worth knowing:

- **A command that fails** changes nothing and returns an error that starts with the command and the
  parameter, such as *feature.boolean: there is no feature named 'Sheet'; features: Block*. Over MCP it is a tool
  result with `isError: true`. In a batch, steps before it stay applied (a batch is not atomic; undo with
  `edit.undo` once per step).
- **A feature that fails** is different: the command succeeded (the feature is in the history) but the
  feature could not be built. Its result has `"status": "failed"` and an `error` that names the parameter,
  the entities and often the largest value that would fit ("the largest radius that fits there is below
  26 mm"). Features built from it fail too, each with `causedBy`: the one to fix. See chapter 4. A
  `command_batch` with `terse` lists under `featuresFailed` the features that still fail when the batch
  ends and that one of its steps added, edited or broke (each with that step and its `causedBy`); they are
  not counted in its `failed`, which counts commands that were refused. A failed assertion is not listed
  there: read `assert.list` or `document.check`.
- **`failed`** in the result of `variable.set` and of the edits that can break other features
  (`variable.delete` with `force`, `feature.edit`, `feature.suppress`, `feature.move`, `feature.delete`, `history.rollback`): the features
  the change made fail, each `{feature, id, error}` (and `causedBy`), left out when there are none. "Make
  the plate 2 mm thick" that breaks a fillet says so in its own result.
- **`assertions`** in the result of every feature command and `variable.set`: how many passed and which
  broke. A broken assertion never stops a feature from being built; it tells you the design no longer
  meets what was asked.
- **`doc_inspect`** (the command `document.inspect`) is the whole state: variables with values, every
  feature with its status and parameters, every body with `volume`, `area`, `centroid`, `bbox`, its faces
  by name (`faceNames`), and the assertions. `document.inspect {"measures": false}` is the quick version
  for a large model (through `command_run`: the tool `doc_inspect` takes no arguments).
- **`document.check`** lists every problem in one place: failed variables and features, invalid bodies,
  broken assertions (`ok` is false), and assertions not proven (warnings). `{"deep": true}` checks more
  (self-intersections, every boolean against its operands) and takes longer.

## Checking the agent's work

Do not take "done" on trust; 3DCraft gives you the means to check, without reading code:

1. **The tree** in the window (chapter 3): every feature built (✔), none failed (⚠).
2. **The assertions**: they are the agent's claims about the part, written as numbers you can read
   ("volume = π((d/2)² − …) t"). `assert.list` shows them; ask for the ones you care about.
3. **Your own numbers**: ask for `measure.distance`, `measure.thickness`, `measure.through` between the
   faces you point at, or for `document.inspect`'s volume and bounding box, and compare with what you
   expect.
4. **Change a variable** and see that the part follows (and the assertions still pass).
5. **Look**: the window, or `doc_render_preview` with `projection: "orthographic"` and a `view` such as
   `top` for a picture to scale, or `mode: "ids"` (one colour per face) to see which face is which.
6. **Reopen the file** (`file.open`): a `.3dcraft` file regenerates bit for bit to the same bodies.
