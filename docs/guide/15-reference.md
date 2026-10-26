# 15. Reference

*Every command by family, the keys and mouse, units and functions, and the programs' options, in one
place to look things up.*

The full description of any command, with every value it takes and what it refuses, is one command away:
`command_list` for an agent, `craft3d-cli commands --filter <id>` in a terminal.

## Commands

3DCraft 0.1.0 has 118 commands. A command's id is its family and its name: `feature.fillet`.

### Documents and files

| Command | Does |
|---|---|
| `file.new` | A new, empty document (`units`, `tolerance`) |
| `file.open`, `file.save`, `file.close` | Open a `.3dcraft` or `.3dcraftjig`, save, close |
| `file.list`, `file.activate` | List the open documents, switch to one |
| `file.import` | Read a STEP file into the document |
| `file.export` | Write STEP, STL, 3MF, OBJ, DXF or SVG ([chapter 12](12-files.md)) |
| `edit.undo`, `edit.redo` | Undo and redo, one change at a time |
| `recovery.list`, `recovery.open`, `recovery.discard` | What was autosaved before a crash |
| `session.report` | A bug report of the last failed command |

### Variables, tables and parameters

| Command | Does |
|---|---|
| `variable.set`, `variable.delete`, `variable.rename` | Define, change, delete and rename variables ([chapter 6](06-concepts.md#variables-expressions-and-units)) |
| `table.set`, `table.delete`, `table.rename`, `table.list` | Tables of values to look up ([chapter 8](08-formulas.md#tables)) |
| `variable.publish`, `variable.unpublish` | Make a variable a Jig's parameter, or an ordinary variable again ([chapter 5](05-jigs.md#making-a-jig)) |
| `param.list`, `param.set` | List and set a Jig's parameters |
| `document.info` | A title, description, author and licence |
| `jig.export`, `jig.edit` | Publish a `.3dcraftjig`; make an editable model of a Jig |
| `document.sample` | Prove a Jig over its parameter range |

### Sketches

| Command | Does |
|---|---|
| `sketch.create` | A sketch on a plane, a datum plane or a flat face |
| `sketch.point`, `sketch.line`, `sketch.polyline`, `sketch.rect`, `sketch.circle`, `sketch.arc`, `sketch.path`, `sketch.spline` | Draw |
| `sketch.curve` | A curve from formulas ([chapter 8](08-formulas.md#curves-from-formulas)) |
| `sketch.pattern` | Repeat entities in a row, a grid or a circle |
| `sketch.constrain`, `sketch.edit`, `sketch.delete` | Add, change and remove constraints and entities |
| `sketch.inspect` | Every entity, constraint and the freedom left |

### Features

| Command | Does |
|---|---|
| `feature.block`, `feature.cylinder`, `feature.sphere`, `feature.cone`, `feature.torus` | Primitives |
| `feature.extrude`, `feature.revolve`, `feature.sweep`, `feature.loft` | Solids from sketches |
| `feature.fill`, `feature.thicken` | Sheets, and a layer on faces |
| `feature.fold`, `fold.edit`, `fold.inspect`, `fold.check` | Origami: a crease pattern folded rigidly, its solved angles, Kawasaki and Maekawa ([chapter 16](16-origami.md)) |
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
| `body.appearance`, `face.appearance`, `body.material` | Colours and density |
| `body.split` | One body per piece |

### Looking, measuring and checking

| Command | Does |
|---|---|
| `document.inspect` | The whole state (`doc_inspect` over MCP) |
| `document.check` | Every problem in one list |
| `document.measure`, `document.edges`, `document.interference` | Volumes and areas, edges, clashes |
| `measure.distance`, `measure.clearance`, `measure.thickness`, `measure.through`, `measure.ray`, `measure.symmetry`, `measure.below`, `measure.section`, `measure.levelFor` | Questions to the geometry ([chapter 10](10-measure.md)) |
| `assert.add`, `assert.edit`, `assert.delete`, `assert.list` | Checks kept in the document |
| `select.get`, `select.set`, `select.query` | The selection, and finding things by description ([chapter 11](11-selection.md)) |
| `set.define`, `set.edit`, `set.remove`, `set.list` | Named sets |
| `view.render`, `view.pick` | A picture, and what is at a point of it |
| `view.scrub`, `view.replay` | The model at a step of its history; a video of how it was built |

## MCP tools

What an agent gets over MCP ([chapter 1](01-install.md#what-the-agent-gets)): `command_list`, `command_run`,
`command_batch`, `doc_inspect`, `doc_render_preview`, `selection_get`, and, bridged to the window,
`control_call`.

## Keys and mouse

The window lists these under **Help › Keyboard shortcuts** (F1). On a Mac, ⌘ takes the place of Ctrl.

| Keys | Do |
|---|---|
| Ctrl+N | New document |
| Ctrl+O | Open a document |
| Ctrl+S | Save |
| Ctrl+Shift+S | Save as |
| Ctrl+I | Import a STEP file |
| Ctrl+E | Export the bodies or the selection |
| Ctrl+W | Close the document |
| Ctrl+Tab, Ctrl+Shift+Tab | Next, previous document |
| Ctrl+Z | Undo the last change to the document (not while typing in a field) |
| Ctrl+Shift+Z, Ctrl+Y | Redo the change last undone |
| F | Fly to the selection, or frame everything |
| Escape | Clear the selection |
| F1 | Show the shortcuts |
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

In a Jig's number field: drag across it (Shift finer, Ctrl in steps), the wheel or the ‹ › to step it,
double-click or Enter to type a value, Escape to give up.

## Units, functions and names

| | |
|---|---|
| Lengths | `um`, `mm`, `cm`, `m`, `in`, `ft` (lower case, right after the number: `3 mm`) |
| Angles | `deg`, `rad`; a plain number where an angle is expected is in degrees, inside `sin`, `cos`, `tan` in radians |
| Functions | `sqrt`, `abs`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `min`, `max`, `floor`, `ceil`, `round`, `trunc`, `mod`, `exp`, `ln`, `log10`, `log2`, `pow`, `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`, `hypot`, `sign`, `if`, `choose`, `clamp` |
| Operators | `+ - * / ^`, comparisons `< <= > >= == !=`, `&&`, `\|\|`, `!` |
| Constant | `pi` (a plain number) |
| Reserved | `pi`, the units and the functions above cannot name a variable (`m` is the metre: call a module `module` or `mn`) |
| Tables | `belt[2].pitch`, `belt["GT2"].pitch` |
| Axes | X, Y and Z, with Z up; planes `xy`, `yz`, `zx` |
| Faces | `F1:face(1)`, `F2:side(e9)`, `F2:startCap()`, `F2:endCap()`, `F3:fillet(0)`; a feature's name for its id: `Plate:face(1)`, `Plate:endCap` ([chapter 6](06-concepts.md#persistent-names)) |

## The programs

**`craft3d`**, the window:

```text
craft3d [FILE] [--control PORT --control-token-file FILE]
        [--automation-read-root DIR] [--automation-write-root DIR]
        [--renderer auto|cpu] [--recovery-dir DIR|none] [--autosave-interval SECONDS]
        [--recent-file FILE|none] [--window-size WIDTHxHEIGHT]
```

`FILE` is a `.3dcraft` document or a `.3dcraftjig` Jig. `--control` makes the window listen for an agent
(with the token file); the roots are the folders an agent may read and write; `--renderer cpu` draws the
model without the graphics card; `--recovery-dir none` switches autosave off.

**`craft3d-cli`**, the command line ([chapter 13](13-command-line.md)):

| Subcommand | Does |
|---|---|
| `run` | Open or create a document, run commands or a script, save it |
| `info` | Print a document's state as JSON |
| `render` | Draw a document into a PNG |
| `convert` | Convert a `.3dcraft` file to STEP, STL, 3MF, OBJ, DXF or SVG |
| `replay` | A video of how a document is built |
| `check-range` | Prove a Jig over its parameter range |
| `score` | Score a script against a task of the task set |
| `commands` | List every command with its values and summary |
| `serve` | The JSON-lines protocol on standard input and output |
| `mcp` | The MCP server for AI agents |

## File types

| Extension | What |
|---|---|
| `.3dcraft` | A 3DCraft document (a ZIP file with the design's history, and a picture for a Jig) |
| `.3dcraftjig` | A published Jig: the same format, opened to set its parameters |
| `.step`, `.stp` | STEP, exact solids for other CAD programs |
| `.stl`, `.3mf`, `.obj` (`.mtl`) | Meshes for 3D printing and rendering |
| `.dxf`, `.svg` | Outlines and drawings for laser cutters and drawing programs |

The technical details of every format are in [`file-formats.md`](../file-formats.md), and of the control
channel and MCP in [`control-protocol.md`](../control-protocol.md).
