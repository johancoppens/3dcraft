# Reference

*Every command by family, the keys and mouse, units and functions, and the programs' options, in one
place to look things up.*

The full description of any command, with every value it takes and what it refuses, is one command away:
`command_list` for an agent, `craft3d-cli commands --filter <id>` in a terminal.

## Commands

3DCraft 0.1.1 has 174 commands. A command's id is its family and its name: `feature.fillet`.

### Documents and files

| Command | Does |
|---|---|
| `file.new` | A new, empty document (`units`, `tolerance`) |
| `file.open`, `file.save`, `file.close` | Open a `.3dcraft`, `.3dcraftjig` or `.3dcraftasm`, save, close |
| `file.list`, `file.activate` | List the open documents, switch to one |
| `file.import` | Read a STEP file, a FreeCAD document (`.FCStd`) or a BRep file into the document |
| `file.export` | Write STEP, STL, 3MF, OBJ, DXF or SVG ([Files, export and drawings](files.md)) |
| `cut.list`, `cut.export` | The cut list of the plates and sheet-metal parts (JSON or CSV), their outlines to DXF or SVG with a kerf ([Files, export and drawings](files.md#cut-lists-the-plates-of-a-design)) |
| `edit.undo`, `edit.redo` | Undo and redo, one change at a time |
| `recovery.list`, `recovery.open`, `recovery.discard` | What was autosaved before a crash |
| `session.report` | A bug report of the last failed command |

### Variables, tables and parameters

| Command | Does |
|---|---|
| `variable.set`, `variable.delete`, `variable.rename` | Define, change, delete and rename variables ([How 3DCraft thinks](concepts.md#variables-expressions-and-units)) |
| `document.setUnit` | Change the length unit, keeping the numbers or the sizes ([How 3DCraft thinks](concepts.md#variables-expressions-and-units)) |
| `document.setTolerance` | Change the linear and angular tolerances (what counts as one place and one direction, and what seams are proven to) |
| `table.set`, `table.delete`, `table.rename`, `table.list` | Tables of values to look up ([Formulas, tables and conditions](formulas.md#tables)) |
| `variable.publish`, `variable.unpublish` | Make a variable a Jig's parameter, or an ordinary variable again ([Making Jigs](making-jigs.md)) |
| `param.list`, `param.set` | List and set a Jig's parameters |
| `document.info` | A title, description, author and licence |
| `jig.export`, `jig.edit` | Publish a `.3dcraftjig`; make an editable model of a Jig |
| `document.sample` | Prove a Jig over its parameter range |

### Sketches

| Command | Does |
|---|---|
| `sketch.create` | A sketch on a plane, a datum plane or a flat face |
| `sketch.point`, `sketch.line`, `sketch.polyline`, `sketch.rect`, `sketch.circle`, `sketch.arc`, `sketch.path`, `sketch.spline` | Draw |
| `sketch.curve` | A curve from formulas ([Formulas, tables and conditions](formulas.md#curves-from-formulas)) |
| `sketch.pattern` | Repeat entities in a row, a grid or a circle |
| `sketch.text`, `sketch.textOnCurve`, `sketch.textEdit` | Text as profiles, straight or along a curve ([Modelling features](features.md#text)) |
| `font.add`, `font.list` | The fonts a document keeps, and the built-in ones |
| `sketch.constrain`, `sketch.edit`, `sketch.delete` | Add, change and remove constraints and entities |
| `sketch.inspect` | Every entity, constraint and the freedom left |

### Features

| Command | Does |
|---|---|
| `feature.block`, `feature.cylinder`, `feature.sphere`, `feature.cone`, `feature.torus` | Primitives |
| `feature.extrude`, `feature.revolve`, `feature.sweep`, `feature.loft` | Solids from sketches |
| `feature.fill`, `feature.thicken` | Sheets, and a layer on faces |
| `feature.fold`, `fold.edit`, `fold.inspect`, `fold.check` | Origami: a crease pattern folded rigidly, its solved angles, Kawasaki and Maekawa ([Origami](origami.md)) |
| `feature.developable`, `feature.unfold`, `unfold.map`, `measure.developability` | Developable sheets between two curves, flat patterns ([Surfaces and developables](surfaces.md#developable-surfaces-and-unfolding)) |
| `feature.surface.network`, `feature.surface.coons` | Sheets through curve networks, edges held tangent to faces ([Surfaces and developables](surfaces.md#surfaces-from-curve-networks)) |
| `feature.surface.trim`, `feature.surface.nsided` | A face of a surface bounded by curves on it; a smooth patch inside a loop of 3 to 6 curves ([Surfaces and developables](surfaces.md#trimmed-faces-and-n-sided-fills)) |
| `feature.plate` | A plate of a sheet, with joints to its neighbours |
| `feature.sheetMetal`, `sheetMetal.edit`, `sheetMetal.bends`, `sheetMetal.bendSequence` | Sheet metal: a base with flanges bent by radius and K-factor, reliefs, the bend table, the press-brake check ([Sheet metal](sheet-metal.md)) |
| `feature.offsetProfile` | A sketch's profiles grown or shrunk |
| `feature.boolean` | Union, subtract, intersect |
| `feature.pattern`, `feature.mirror`, `feature.transform` | Copies, mirror, move and turn |
| `feature.fillet`, `feature.chamfer` | Round and bevel edges |
| `feature.shell`, `feature.offsetFace`, `feature.moveFace`, `feature.draft`, `feature.deleteFace` | Operations on faces |
| `feature.thread` | An ISO metric thread |
| `feature.edit`, `feature.rename`, `feature.move`, `feature.delete` | Change the history |
| `feature.suppress`, `feature.when` | Switch a feature off by hand or by a condition |
| `feature.visible` | Hide a feature in the view |
| `history.rollback` | Build only up to a feature; insert after it |
| `datum.plane`, `datum.axis`, `datum.point`, `datum.helix`, `datum.references` | Construction geometry |
| `body.appearance`, `face.appearance`, `body.material` | Colours, material layers (anisotropy, clear coat, transmission, subsurface) and density |
| `material.presets`, `material.preset` | The material library: steel, brushed aluminium, ABS, glass, car paint, wood |
| `body.split` | One body per piece |

### Assemblies

| Command | Does |
|---|---|
| `assembly.new` | A new, empty assembly ([Assemblies](assemblies.md)) |
| `instance.add`, `instance.edit`, `instance.delete`, `instance.open` | Place a part, a Jig or an assembly; change, remove, open its source |
| `mate.add`, `mate.edit`, `mate.delete` | Join two instances (fixed, revolute, slider, cylindrical, planar, ball) |
| `coupling.add`, `coupling.delete` | Gears, racks and screws between two mates; linear ties of any number of motions |
| `joint.set` | Set a mate's free motion (the variable `<mate>_angle`, or `<mate>_distance` for a slide) |
| `assembly.solve`, `assembly.inspect` | How the mates stand; the instances and configurations |
| `measure.joint`, `measure.pose` | A joint's solved value; where an instance or a point through it is |
| `assembly.reload`, `assembly.follow`, `assembly.pack`, `assembly.unpack` | Follow changed parts (by hand; what changed, of every open assembly); carry the parts in the file, or write them out |

### Looking, measuring and checking

| Command | Does |
|---|---|
| `document.inspect` | The whole state (`doc_inspect` over MCP) |
| `document.check` | Every problem in one list |
| `document.measure`, `document.edges`, `document.interference` | Volumes and areas, edges, clashes |
| `measure.distance`, `measure.clearance`, `measure.thickness`, `measure.through`, `measure.ray`, `measure.symmetry`, `measure.below`, `measure.section`, `measure.levelFor`, `measure.floating`, `measure.continuity`, `measure.curvature`, `measure.draft` | Questions to the geometry ([Measuring and assertions](measure.md)) |
| `assert.add`, `assert.edit`, `assert.delete`, `assert.list` | Checks kept in the document; their measures include the floating equilibrium (`floatingGM`, `floatingTrim`, …) and the press-brake check (`bendable`) ([Measuring and assertions](measure.md#assertions)) |
| `variable.measure`, `variable.output`, `output.list` | Outputs: read-only variables computed after regeneration from measures and formulas, for assertions ([Measuring and assertions](measure.md#outputs-measures-in-formulas)) |
| `select.get`, `select.set`, `select.query` | The selection, and finding things by description ([Selection and named sets](selection.md)) |
| `agent.log` | What agents did to the document, newest first, with the intent they gave, and which agent made each feature |
| `set.define`, `set.edit`, `set.remove`, `set.list` | Named sets |
| `view.render`, `view.pick` | A picture, and what is at a point of it |
| `view.scrub`, `view.replay` | The model at a step of its history; a video of how it was built |
| `view.motion`, `view.trace`, `motion.check`, `motion.bake`, `view.drive` | The model at a moment `t` of its motion (collisions in red); a point's path; assertions checked over the motion (`view.replay {motion}` films it); the motion's moments made ahead, so it plays smoothly; an assembly with one joint held by hand for the view, the others following |

## MCP tools

What an agent gets over MCP ([Install 3DCraft and connect an AI agent](install.md#what-the-agent-gets)): `command_list`, `command_run`,
`command_batch`, `doc_inspect`, `doc_render_preview`, `selection_get`, and, bridged to the window,
`control_call`.

## Keys and mouse

The window lists these under **Help › Keyboard shortcuts** (F1). On a Mac, ⌘ takes the place of Ctrl,
except in Ctrl+Tab and Ctrl+Shift+Tab, which stay as they are.

| Keys | Do |
|---|---|
| Ctrl+N | New part |
| Ctrl+Shift+N | New assembly |
| Ctrl+O | Open a document |
| Ctrl+S | Save |
| Ctrl+Shift+S | Save as |
| Ctrl+I | Import a STEP file or a FreeCAD document |
| Ctrl+E | Export the bodies or the selection |
| Ctrl+W | Close the document |
| Ctrl+Tab, Ctrl+Shift+Tab | Next, previous document |
| Ctrl+Z | Undo the last change to the document (not while typing in a field) |
| Ctrl+Shift+Z, Ctrl+Y | Redo the change last undone |
| F | Fly to the selection, or frame everything |
| Escape | Fold the Studio or Analysis options, else let go of a joint held by hand, else clear the selection |
| F1 | Show the shortcuts |
| Ctrl+B | Show or put away the side panel |
| F11 | Full screen (the desktop app) |
| Space | Play or pause the history |
| ← / → | A step back / forward in the history |
| Home | The first step |
| End | The end of the history (in the view only: the rollback marker stays) |

The history keys work while the pointer is over the view or the feature list.

| Mouse | Does |
|---|---|
| Drag | Orbit |
| Right or middle drag, Shift+drag | Pan |
| Wheel, pinch | Zoom about the point under the pointer |
| Click | Select a vertex, an edge or a face |
| Shift+click | Add to or take from the selection |
| Click again, Alt+click | Select the next face behind |
| Double-click in the viewport | Select the whole body |
| Right click | List every face under the pointer |
| Ctrl+drag | Select what lies inside a box |
| Double-click a feature, or its ⌖ | Select the feature and fly to what it made |
| Click the view cube | Turn to that view |

On a slider (a Jig's parameters, See-through, the section's Offset, the Studio and Analysis options): press
the rail to put the knob there and drag it (Shift finer, Ctrl in whole steps); once clicked, the arrow keys or
the wheel step it (Shift: ten steps); click its value (or press Enter over it) to type a value or an expression
with units, Escape to give up; double-click the knob for its default. A whole number with a few values is a
stepper instead: **−** and **+** beside the value ([Using a Jig › The form](jigs.md#the-form)).

## The window, for agents and programs

What a person does with the window's buttons, an agent or a program does with `ui.set` and the other
`ui.*` methods over the control channel ([Programs on 3DCraft](programs.md#the-windows-control-channel)),
and `craft3d-cli render` takes the same looks. [`control-protocol.md`](../control-protocol.md) lists every
field.

**The appearance**: `ui.set {"appearance": "dark"}` (`"light"`, `"system"`).

**The studio look** ([The window › The studio look](window.md#the-studio-look)):

```text
ui.set {"look": {"mode": "studio", "environment": "outdoor", "exposure": 0.5, "rotation": 90}}
craft3d-cli render part.3dcraft --out part.png --look studio --environment workshop
```

| Field | Does |
|---|---|
| `environment` | `studio` (the default), `outdoor` or `workshop` |
| `exposure` | Brighter in stops; negative is darker |
| `rotation` | Turns the environment about the vertical axis, in degrees |
| `ground`, `ao`, `shadows` | `false` switches off the ground, the darkening in corners, the shadows |
| `tonemap` | How bright light is fitted into the picture: `agx` (the default), `aces` or `linear` |
| `background` | `environment` (the default) shows the environment; `plain` keeps the window's own background behind the model, while the environment still lights it |
| `reflections` | Glossy faces (roughness under 0.4) reflect the model where it is in the picture, and the environment elsewhere (on by default) |
| `groundReflection` | How much the ground reflects, 0 to 1 (0 by default). Towards the horizon the ground then mirrors the sky; it reflects only what is in the picture, so under a model whose undersides you cannot see, the reflection comes out in pieces |
| `contactShadows` | Short, dark shadows where parts touch or stand on each other (on by default) |
| `specularShadow` | A face in the key light's shadow loses that light's reflection too (on by default) |
| `bloom` | How much bright parts glow into their surroundings: the share of the light spread, 0.04 by default |
| `dof` | Depth of field: `{"fstop": 2.8, "focus": …}`. `focus` is `"selection"`, `"picked"` (the point clicked last), `"centre"`, a number (a distance) or `[x, y, z]` (a point); `ui.get` says under `lens` where it focuses now |
| `accumulate` | How many frames a still picture averages, 1 to 256 (16 by default) |
| `edges` | `soft` (the default of the studio look: the edge lines fade where the lens blurs the model), `sharp`, or `off` (the **Edges** toggle, `ui.set {"show": {"edges": false}}`, hides them in every look) |

```text
ui.set {"look": {"mode": "studio", "dof": {"fstop": 2, "focus": "selection"}, "edges": "soft", "bloom": 0.1}}
craft3d-cli render part.3dcraft --out part.png --look studio --fstop 4 --accumulate 32
```

- `view.render` focuses `selection` on the session's selection. `craft3d-cli render` has no selection or
  click, so give it a point (`--focus X,Y,Z`, from `select.get` or `ui.pick`), a distance, or `centre`.
- `view.render` and `craft3d-cli render` draw one frame unless `accumulate` (`--accumulate`) asks for
  more. They refuse a still whose frames together would take too long: all frames' samples over 2^26,
  about 34 frames of the default 800 × 600 picture with the CPU renderer; on the graphics card, up to 256
  at any size.
- `ui.screenshot` with `window: true` waits until the window has averaged its frames, so the picture is
  the settled one; `ui.set {"look": {"accumulate": 4}}` settles sooner, `64` smoother. `view.replay`
  averages 16 per frame unless its `look` says otherwise.

**Surface analysis** ([The window › Surface analysis](window.md#surface-analysis)); `view.render` takes the
same object:

```text
ui.set {"analysis": {"mode": "zebra", "stripes": 32, "direction": 30}}
ui.set {"analysis": {"mode": "curvature", "curvature": "mean", "combs": {"edges": true, "density": 60}}}
ui.set {"analysis": {"mode": "draft", "pull": [0, 0, 1], "minDraft": 2}}
ui.set {"analysis": null}
```

Zebra takes `stripes` (how many per turn), `direction` (turns the tunnel on the screen), `width` (the
dark share of each stripe) and `offset` (slides them along). Curvature takes `curvature` (`gaussian`,
`mean` or `max`) and `range` (by default the 95th percentile of the values). Draft takes `pull` and
`minDraft` (degrees). `combs` draws the curvature of edges, or of iso-curves of a face.

**A motion** ([The window › Playing a motion](window.md#playing-a-motion)):

- `view.motion {"t": 0.5, "collisions": true}` shows the moment `t`, with the parts that collide in red.
- `view.trace` draws a point's path; `"grow": true` draws it only as far as the moment shown, so it grows
  while the motion plays. `file.export {"path": "path.svg", "trace": "Trace 1"}` exports it.
- In the window a trace is worked out in the background, so the motion keeps playing: the answer says
  `"pending": true`. `view.trace {"name": "Trace 1", "wait": true}` gives the points once they are there,
  or why they could not be made; it waits at most 2 s in the window, so ask again. `ui.get` lists every
  trace under `traces`, with `pending` or `error`. `"background": false` works it out in the command.
- `motion.check` checks the checks over the motion
  ([Engineering checks › Over a motion](engineering-checks.md#over-a-motion)), and
  `view.replay {"dir": "frames", "motion": {"frames": 120}}` films it.
- `ui.set {"timeline": {"playQuality": "full"}}` keeps the full look while a motion plays.
  `ui.get` › `timeline.heavy` says whether the motion is heavy (`msPerMoment`, `suggestBake`).

**Recording the window.** `ui.record` records the window itself, panels included, even while other
windows cover it; with `ui.advance`, frame by frame, so every step of a played history is complete in the
video.

## Units, functions and names

| | |
|---|---|
| Lengths | `um`, `mm`, `cm`, `m`, `in`, `ft` (lower case, right after the number: `3 mm`) |
| Angles | `deg`, `rad`; a plain number where an angle is expected is in degrees, inside `sin`, `cos`, `tan` in radians |
| Mechanics | `g`, `kg` (mass), `s` (time), `N`, `kN` (force), `Pa`, `kPa`, `MPa`, `GPa` (pressure, stress), `J` (energy); a force unit then a length unit is a moment (`20 N m`); not reserved: units only right after a number, a parenthesis or `pi` |
| Shown units | an output's `unit`: units multiplied by a space, `*` or `·`, to powers (`^2`, `²`), with one `/`: `N/mm²`, `kN m`, `kg/m^3` |
| Functions | `sqrt`, `abs`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `min`, `max`, `floor`, `ceil`, `round`, `trunc`, `mod`, `exp`, `ln`, `log10`, `log2`, `pow`, `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`, `hypot`, `sign`, `if`, `choose`, `clamp` |
| Operators | `+ - * / ^`, comparisons `< <= > >= == !=`, `&&`, `\|\|`, `!` |
| Constant | `pi` (a plain number) |
| Reserved | `pi`, the length and angle units and the functions above cannot name a variable (`m` is the metre: call a module `module` or `mn`) |
| Tables | `belt[2].pitch`, `belt["GT2"].pitch` |
| Axes | X, Y and Z, with Z up; planes `xy`, `yz`, `zx` |
| Faces | `F1:face(1)`, `F2:side(e9)`, `F2:startCap()`, `F2:endCap()`, `F3:fillet(0)`; a feature's name for its id: `Plate:face(1)`, `Plate:endCap` ([How 3DCraft thinks](concepts.md#persistent-names)) |

## The programs

**`craft3d`**, the window:

```text
craft3d [FILE] [--control PORT] [--control-token-file FILE]
        [--automation-read-root DIR] [--automation-write-root DIR]
        [--state-dir DIR] [--renderer auto|cpu]
        [--recovery-dir DIR|none] [--autosave-interval SECONDS]
        [--recent-file FILE|none] [--window-size WIDTHxHEIGHT]
```

| Option | Does |
|---|---|
| `FILE` | A document, a Jig or an assembly to open |
| `--control PORT` | Listen for a program from the start, as if **Allow AI agents** were on (`0` picks a free port) |
| `--control-token-file FILE` | The file with the control token (made with a fresh token when missing) |
| `--automation-read-root DIR`, `--automation-write-root DIR` | The folders an agent or a program may read and write, instead of the work folder |
| `--state-dir DIR` | 3DCraft's own folder: the control token, the remembered switch, and the default autosaves and recent files |
| `--renderer auto\|cpu` | `cpu` draws the model with the reference renderer instead of the graphics card |
| `--recovery-dir DIR\|none` | Where documents are autosaved; `none` switches autosave off |
| `--autosave-interval SECONDS` | How long after a change a document is autosaved (30 by default) |
| `--recent-file FILE\|none` | Where the list of recent files is kept |
| `--window-size WIDTHxHEIGHT` | The window's size when it opens (1280x800 by default) |

The state folder is `3dcraft` in your state directory: on Linux `$XDG_STATE_HOME` or `~/.local/state`, on
macOS `~/Library/Application Support`, on Windows `%LOCALAPPDATA%`. Environment variables stand in for
the options of the same name: `CRAFT3D_CONTROL_PORT`, `CRAFT3D_CONTROL_TOKEN_FILE`, `CRAFT3D_STATE_DIR`,
`CRAFT3D_AUTOMATION_READ_ROOT`, `CRAFT3D_AUTOMATION_WRITE_ROOT`, `CRAFT3D_RENDERER`,
`CRAFT3D_RECOVERY_DIR`, `CRAFT3D_AUTOSAVE_INTERVAL`, `CRAFT3D_RECENT_FILE` and `CRAFT3D_WINDOW_SIZE`.
`CRAFT3D_AGENT_PORT` is the first port **Allow AI agents** tries (7878 by default).

**`craft3d-cli`**, the command line ([The command line and scripts](command-line.md)):

| Subcommand | Does |
|---|---|
| `run` | Open or create a document, run commands or a script, save it |
| `info` | Print a document's state as JSON |
| `render` | Draw a document into a PNG |
| `convert` | Convert a `.3dcraft` file to STEP, STL, 3MF, OBJ, DXF or SVG |
| `replay` | A video of how a document is built |
| `check-range` | Prove a Jig over its parameter range |
| `score` | Score a script against a task of the task set (in a source checkout) |
| `commands` | List every command with its values and summary |
| `serve` | The JSON-lines protocol on standard input and output |
| `mcp` | The MCP server for AI agents |

## File types

| Extension | What |
|---|---|
| `.3dcraft` | A 3DCraft document (a ZIP file with the design's history, and a picture for a Jig) |
| `.3dcraftjig` | A published Jig: the same format, opened to set its parameters |
| `.3dcraftasm` | An assembly: its instances, mates and couplings, and the parts it carries when packed ([Assemblies](assemblies.md)) |
| `.step`, `.stp` | STEP, exact solids for other CAD programs |
| `.stl`, `.3mf`, `.obj` (`.mtl`) | Meshes for 3D printing and rendering |
| `.dxf`, `.svg` | Outlines and drawings for laser cutters and drawing programs |

The technical details of every format are in [`file-formats.md`](../file-formats.md), and of the control
channel and MCP in [`control-protocol.md`](../control-protocol.md).

**Back to:** [Start here](README.md).
