# Working with an AI agent

*Describe a part, let the agent build it in 3DCraft while you watch, and check what it made.*

An agent connected to 3DCraft ([Install › Connect an AI agent](install.md#connect-an-ai-agent-over-mcp))
works the way a careful draughtsman would. It names the sizes as variables, adds features one by one,
and reads back what it built.

Your part is to say what you want, to watch, and to check. You do not need to know the commands: the
agent looks them up itself.

## What to ask

Ask for a part the way you would describe it to a colleague, with the numbers that matter and what they
are measured from:

> Make a bolt flange: a disc 80 mm across and 10 mm thick with a 30 mm bore, and six 8 mm bolt holes on a
> 60 mm pitch circle. Drive it by variables, so that I can change the number of holes. Assert its volume.
> Save it as flange.3dcraft and export a STEP file.

A few words from that request, for readers new to CAD:

| Word | Means |
|---|---|
| **Bore** | The large hole through the middle, for a shaft or a pipe |
| **Pitch circle** (or bolt circle) | The circle the centres of the bolt holes lie on |
| **Variable** | A named size, such as `n` for the number of holes, that the model follows when it changes |
| **Assert** | Keep a check in the design: here, that its volume is what a formula says |
| **Fillet** (or round), **chamfer** | An edge rounded off, or cut off at an angle |
| **STEP** | The file format other CAD programs read: exact solids, not triangles |

Things that help:

- **Name the sizes** you want to change later ("the thickness", "the number of holes").
- **Say where the part sits**: "the corner at the origin", "the bottom on the XY plane". 3DCraft's axes
  are X, Y and Z, with Z up.
- **Ask for checks** ([Measuring and assertions](measure.md)): "the volume must be …", "the holes must go
  through", "the wall at least 2 mm". 3DCraft keeps them in the document and checks them again after
  every change, so they do not depend on the agent's memory.
- **Ask for a check and a picture** at the end: "run document.check and show me a preview".
- **Point instead of describing** when it is easier: click a face in the window and say "this face"
  ([The window › Pointing at things](window.md#pointing-at-things)).
- **Change by sizes**: "make it 8 holes" is one change of a variable, and everything that depends on it
  follows.

### More things to ask

| You say | The agent … |
|---|---|
| "Round the outside edges of the top by 2 mm." | adds a fillet on the edges it finds by their shape and place |
| "Make the hole I selected 6.5 mm." | reads your selection, finds the size that drives that hole and changes it |
| "Why did the fillet fail?" | reads the reason 3DCraft gave (often with the largest size that fits) and proposes a fix |
| "Show me the history step by step." | plays the history in your window |
| "Make this a Jig with width, depth and height between 50 and 200 mm." | publishes those variables as parameters and exports a `.3dcraftjig` ([Making Jigs](making-jigs.md)) |
| "Export the outline of the top for the laser cutter." | writes a DXF or SVG of that face at true size ([Files, export and drawings](files.md)) |
| "Something went wrong; give me the bug report." | returns the report of the last failed command ([Troubleshooting](troubleshooting.md#reporting-a-problem)) |

## A full session

This is what an agent sent for the request above, and what came back. The results are shortened (`...`),
but every key and number shown is real; a test runs this session again at every change of 3DCraft. The
agent ran headless here, `craft3d-cli mcp --automation-read-root work --automation-write-root work`.

**1. Learn the commands.** `command_list` with a filter, for the commands it needs; the parameters come
with each. (`brief: true` gives every command with its parameters and the first sentence of its summary,
a fraction of the size; `id` then gives one command whole.)

```text
command_list {"filter": "pattern"}
-> {"commands": [{"id": "sketch.pattern", ...}, {"id": "feature.pattern",
     "params": "{\"source\": id or name, \"count\": expr, \"step\": [expr, expr, expr] or, with angle, expr, \"angle\": expr, ...}",
     "summary": "...", "enabled": false, "disabledReason": "no document is open; run file.new or file.open first"}]}
```

**2. A document, then the variables, an assertion and the features in one batch.** The assertion comes
first: it names `Flange` before that exists, is in error until it does, and passes once it is built.

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
             "assertionSummary": {"count": 1, "passed": 1, "broken": [], "unproven": [], "pending": 0}}}}
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

**5. Change it.** Eight holes instead of six. The result of the edit carries the assertions' summary:
still passing, because the expected volume is written in `n` too.

```text
command_run {"id": "variable.set", "params": {"name": "n", "expr": "8"}}
-> {"name": "n", "value": 8.0, "assertionSummary": {"count": 1, "passed": 1, "broken": [], ...}}

command_run {"id": "measure.thickness", "params": {"of": "Flange"}}
-> {"thickness": 6.0, "atLeast": 5.99999999999968,
    "at": {"face": "F6:copy(1)/F4:face(0)", "point": [34.0, ...]},
    "to": {"face": "F6:copy(0)/F1:face(0)", "point": [40.0, ...]}}
```

The thinnest wall is 6 mm, between a bolt hole and the rim. `atLeast` is what is proven: no wall is
thinner than that anywhere ([Measuring and assertions](measure.md#measures)).

**6. A refusal, and undo.** Asked for a 20 mm round on the rim of a 10 mm disc, the fillet is added but
fails. `command_run` does not report an error, so the agent reads `status`:

```text
command_run {"id": "feature.fillet", "params": {"edges": [["F1:face(0)", "F1:face(2)"]], "radius": "20 mm", "name": "Round"}}
-> {"id": "F7", "name": "Round", "status": "failed",
    "error": "radius: not built for edges.0 (between F6:copy(0)/F1:face(0) and F6:copy(0)/F1:face(2)): with a radius of 20 mm the fillet face would be a torus that reaches its own axis ...which is not built yet; so it is for every radius from 20 mm up to 40 mm..."}

command_run {"id": "edit.undo", "params": {}}
-> {"undone": "Add fillet"}
```

**7. Look, save, export.**

```text
command_run {"id": "document.check", "params": {}}
-> {"ok": true, "problems": [], "bodiesChecked": 1, "assertionSummary": {"passed": 1, ...}}
doc_render_preview {"view": "iso"}
-> (a PNG image) and {"coverage": ..., "triangles": ..., "bodies": [...]}
command_run {"id": "file.save", "params": {"path": "flange.3dcraft"}}
-> {"path": "flange.3dcraft"}
command_run {"id": "file.export", "params": {"path": "flange.step"}}
-> {"format": "step", "bodies": 1, "solids": 1, "faces": 12, "unit": "mm", ...}
```

The [flange lesson](../examples/flange.json) is the same part with a chamfer and a named set of bolt
holes.

## Checking the agent's work

Do not take "done" on trust. 3DCraft gives you the means to check, without reading code:

1. **The feature list** in the window ([The window](window.md#features)): every feature built (✔), none
   failed (⚠), every check passed.
2. **The checks** (assertions): they are the agent's claims about the part, written as numbers you can
   read ("volume = π((d/2)² − …) t"). `assert.list` shows them; ask for the ones you care about.
3. **Your own numbers**: ask for `measure.distance`, `measure.thickness` or `measure.through` between
   the faces you point at, or for the volume and bounding box `document.inspect` gives, and compare them
   with what you expect.
4. **Change a variable** and see that the part follows, and the checks still pass.
5. **Look**: the window, or `doc_render_preview` with `projection: "orthographic"` and a `view` such as
   `top` for a picture to scale, or `mode: "ids"` (one colour per face) to see which face is which.
6. **Reopen the file** (`file.open`): a `.3dcraft` file rebuilds bit for bit to the same bodies.
7. **Play the history** (the player under the feature list, or ask the agent to play it): each step
   should make sense on its own.

## Tips for a good session

- **Start simple, then add.** "A plate 100 × 60 × 5 mm" first, then "four holes 10 mm from the corners",
  then "round the corners". Each step is easy to check.
- **Keep the window in sight.** You see every step appear, and you can stop the agent when it goes the
  wrong way.
- **Ask it to save** now and then, under a name you choose. The agent can also undo (`edit.undo`), one
  step at a time, if something goes wrong.
- **When 3DCraft refuses something,** the message says why, and often what would work. Ask the agent to
  read it to you; it is meant to be read.

## What the agent gets back

You do not need this to work with an agent, but it helps to read along. Every command answers with JSON.

- **A command that fails** changes nothing. Its error starts with the command and the parameter, such as
  *feature.boolean: there is no feature named 'Sheet'; features: Block*.
- **A feature that fails** is different: the command did what was asked (the feature is in the history),
  but the feature could not be built. Its result says `"status": "failed"`, with an `error` that names the
  parameter, the entities and often the largest value that would fit
  ([How 3DCraft thinks › Failed features](concepts.md#failed-features)).
- **`assertionSummary`**, in the result of every feature command and of `variable.set`, `param.set` and
  `document.check`: how many checks passed and which broke. A broken check never stops a feature from
  being built; it tells you the design no longer meets what was asked.
- **`document.inspect`** (the tool `doc_inspect`) is the whole state: variables, features with their
  status, bodies with volume, area and faces, and the checks.

[Programs on 3DCraft › What a command answers](programs.md#what-a-command-answers) has the full rules,
for people who write their own programs or read an agent's work closely.

## Try this first: five tasks

Five tasks to start with, each with the answer 3DCraft should give. Each takes 5 to 20 minutes. Give the
agent the text in quotes, in your own words if you like, and in your own language if you prefer. Then
check its work as [above](#checking-the-agents-work): look at the window, click faces, and ask for the
volume.

A refusal ("not built", "does not fit") is not a failure of the task. It says what 3DCraft cannot do yet,
and that is worth reporting ([Troubleshooting](troubleshooting.md#reporting-a-problem)).

**Task 1: a block, to see the connection work.** *"Make a 40 × 20 × 10 mm block in 3DCraft, with a corner
at the origin. Tell me its volume."* The block appears in the window, and the answer is **8000 mm³**.

**Task 2: an L-bracket.** *"In 3DCraft, make an L-shaped angle bracket of 4 mm stock, 25 mm wide: one leg
lies flat on the XY plane and runs 40 mm along +X, the other stands up along +Z to a height of 30 mm,
both legs measured to the outside of the corner, which is at the origin. Drive it by the variables leg_x,
leg_z, thickness and width. Tell me its volume."* The right answer is **6600 mm³**. Then ask it to round
the inside corner with a 4 mm radius, and to make the bracket 30 mm wide: the rounding should follow.

**Task 3: a strip with a counted row of holes.** *"Make a flat strip 120 × 20 × 3 mm (corner at the
origin, along +X, +Y, +Z) with a row of 5 mm holes along its centre line at a 10 mm pitch: as many holes
as fit with every hole centre at least 7.5 mm from the ends, the row centred on the strip. The number of
holes must follow the length."* That is **11 holes and 6552.05 mm³**. Now ask for a strip of 150 mm:
**14 holes and 8175.33 mm³**, without the agent rebuilding anything by hand.

This is one way to build it, as a script ([The command line and scripts](command-line.md) runs it with
`craft3d-cli run --new '{"units":"mm"}' --script strip.json --out strip.3dcraft`). Its two assertions
check the volume and the number of holes at every change:

```json
[
 ["variable.set", {"name": "length", "expr": "120 mm"}],
 ["variable.set", {"name": "width", "expr": "20 mm"}],
 ["variable.set", {"name": "thickness", "expr": "3 mm"}],
 ["variable.set", {"name": "hole_d", "expr": "5 mm"}],
 ["variable.set", {"name": "pitch", "expr": "10 mm"}],
 ["variable.set", {"name": "margin", "expr": "7.5 mm"}],
 ["variable.set", {"name": "n", "expr": "floor((length - 2 * margin) / pitch) + 1"}],
 ["feature.block", {"size": ["length", "width", "thickness"], "name": "Strip"}],
 ["feature.cylinder", {"radius": "hole_d / 2", "height": "thickness", "position": ["(length - (n - 1) * pitch) / 2", "width / 2", 0], "name": "HoleTool"}],
 ["feature.pattern", {"source": "HoleTool", "count": "n", "step": ["pitch", 0, 0], "name": "HoleTools"}],
 ["feature.boolean", {"op": "subtract", "target": "Strip", "tool": "HoleTools", "name": "Perforated"}],
 ["assert.add", {"name": "volume", "measure": "volume", "of": "Perforated", "eq": "(length * width - n * pi / 4 * hole_d^2) * thickness"}],
 ["assert.add", {"name": "holes", "measure": "count", "of": {"kind": "face", "where": {"hole": true}}, "eq": "n"}]
]
```

**Task 4: a case, and a rule that guards it.** *"Make a moulded case: outside 70 × 50 × 25 mm (corner at
the origin), the top edges and the four upright corners rounded with a 2 mm radius, the bottom edges
sharp, hollowed to a 2 mm wall and open at the bottom. Drive it by variables. Then add an assertion that
the thinnest wall is at least 1.5 mm."* The volume is **17385.50 mm³**, and at 2 mm the assertion passes:
3DCraft proves the wall is at least 1.5 mm everywhere, round corners included. Now ask for a 1 mm wall:
the window shows the check failing, and the agent should tell you so
([Measuring and assertions](measure.md#assertions)).

**Task 5: make a Jig of it.** *"Publish length and pitch of the strip from task 3 as parameters (length
from 40 to 300 mm, pitch from 8 to 20 mm) and export it as a Jig."* Open the `.3dcraftjig` in 3DCraft:
the **Parameters** tab shows the two values. Drag them and watch the holes follow, then export an STL, or
a DXF of the top face for a laser cutter ([Making Jigs](making-jigs.md)).

After these, **try a part of your own**: something from your workshop, your classroom or your hobby.

**Next:** [The window](window.md), or straight to [Using a Jig](jigs.md).
