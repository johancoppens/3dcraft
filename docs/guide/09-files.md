# 9. Files

| Format | Read | Write | Holds |
|---|---|---|---|
| `.3dcraft` | `file.open` | `file.save` | The design history: units, tolerance, variables, sketches, features, assertions, sets, colours |
| STEP (`.step`, `.stp`) | `file.import` | `file.export` | Exact solids (B-rep), units, names, colours; no history |
| STL, 3MF, OBJ | no | `file.export` | Triangle meshes, for printing and rendering |
| DXF, SVG | no | `file.export` | The flat outline of a sketch or a flat face, at true size, for laser cutting and drawings |

Paths are relative to the read and write roots the server or window was started with (chapter 1);
`craft3d-cli` on its own reads and writes anywhere you can.

## The native format and its promise

A `.3dcraft` file holds the history, not the geometry: opening it regenerates the bodies, bit for bit the
same as when it was saved. Saving is atomic (a full disk or a crash leaves the previous file intact).

**The promise:** every `.3dcraft` file written by a released version of 3DCraft opens in every later
version and regenerates to the bodies it was saved with. A file from a *newer* version is refused by an
older one with a message that names both versions, never guessed at. Each format version has a frozen
file written by its own build that the tests open at every change.

## STEP

```json
[
 ["file.export", {"path": "part.step"}],
 ["file.import", {"path": "supplier-part.step"}]
]
```

Export writes every body as an exact solid (AP214) with the document's unit, its tolerance as the file's
uncertainty, and each body under its feature's name and colour. Import reads AP203, AP214 and AP242 solids
and sheets (assemblies too, one feature per placed solid, with their colours), converts the units, and
keeps the file's bytes inside the document so it regenerates without the file. An imported body is a
feature like any other: cut it, fillet it, measure it. Its faces are named by their order in the file
(`F5:face(0)`). What cannot be read is refused naming the STEP entity.

To export some of the bodies, name them: `"bodies": ["Bracket", "F4"]`, each by the name or id of the
feature that made it (as `document.inspect` and `select.get` name it), in STEP, STL, 3MF and OBJ alike.
A name that is not a body now (unknown, failed, suppressed, or taken by a later feature such as a boolean)
is refused with the list of bodies there are. In the window, Export… with bodies selected writes those.

## Meshes

```json
[
 ["file.export", {"path": "part.3mf"}],
 ["file.export", {"path": "part.stl", "quality": "fine"}],
 ["file.export", {"path": "part.stl", "tolerance": 0.01}]
]
```

`quality` is `coarse`, `normal` (the default) or `fine`: how closely the triangles follow the model.
Each sets a distance chosen from the model's size (2, 1 or ¼ thousandth of it) and an angle (20°, 10°
or 5°): no triangle's side turns the surface by more, so small holes and thin bent tubes stay round on a
big part. `tolerance` (in the document's unit) and `angle` (degrees; 0 for none, else 1 to 90) set either one
yourself; a `tolerance` alone, with no `quality` or `angle`, makes the mesh it always made (no angle).
The result says `tolerance`, `angle`, `quality` and how many meshes are `closed` (watertight). 3MF
carries the unit, the bodies' names and colours; STL carries triangles only (`"format": "stl-ascii"` for
text). A mesh of more than 20 million triangles, all bodies together, is refused: choose a coarser
quality. In the window, Export… to a mesh asks for the quality (Coarse, Normal, Fine; the last one you
chose is offered) before it writes; `craft3d-cli convert part.3dcraft part.stl --quality fine` (or
`--angle`, `--tolerance`) does the same from a shell.

When the selection does not fit the format you chose (a face selected and a STEP or STL file chosen),
the window says so before anything is written and offers **Export all bodies**; for DXF or SVG with no
flat face or sketch selected it says what to select. A failed Open, Save, Import or Export in the window
opens a dialog with what went wrong and **Copy details** (the report for a bug); what an agent's command
did stays in the status bar.

## Flat outlines: DXF and SVG

For a laser cutter or a drawing: the outline of a sketch, or of a flat face of a body, with its holes, at
true size; lines, arcs, circles and splines stay what they are.

```json
[
 ["file.export", {"path": "panel.dxf", "sketch": "Outline"}],
 ["file.export", {"path": "panel.svg", "face": "F2:endCap()"}],
 ["file.export", {"path": "parts.dxf", "faces": ["F2:endCap()", "F2:startCap()"], "gap": 5}]
]
```

A face is drawn as seen from outside the body, never mirrored. `faces` lays several faces out on one
sheet, each on its own layer, `gap` apart (a length like any other: `5`, `"5 mm"` or `"kerf * 4"`).
`feature.offsetProfile` grows a sketch by half the kerf before exporting it.

## From the command line

`craft3d-cli convert IN OUT` does the same for a saved file; the extension of `OUT` picks the format:

```sh
craft3d-cli convert bracket.3dcraft bracket.step
craft3d-cli convert panel.3dcraft panel.dxf --sketch Outline
craft3d-cli convert panel.3dcraft panel.svg --face 'F2:endCap()'
craft3d-cli info bracket.3dcraft          # the whole state as JSON
craft3d-cli render bracket.3dcraft --out bracket.png --size 800x600 --view iso
```

## Files in the web version

The web version reads and writes files on your computer; nothing is uploaded.

- **Chrome, Edge and Chromebooks:** Open…, Import…, Save As… and Export… show your system's file
  dialog. Export… first asks for the format in a small dialog of its own (the last one you used is
  chosen); *Export…* there makes the file and then shows the system's dialog to say where it goes. An
  export that cannot be made (DXF or SVG with nothing flat selected) says why before any file is
  touched. Save writes back to the file the document was opened from or last saved to, without asking
  again; the first time, the browser asks whether the page may change that file. A tab you close with
  *Save* stays open until its file is written; if the file cannot be written, the document stays open
  and unsaved. An OBJ with colours then offers its material library (`.mtl`) with a *Save…* button:
  save it next to the `.obj`.
- **Firefox and Safari** have no such dialogs for web pages: Save and Export ask for a name in a small
  dialog of their own and the file is a download (in the browser's downloads, often the *Downloads*
  folder); Open and Import use the browser's file chooser. Add `?download` to the address to work this
  way in Chrome too.
- The status bar says where each file went (*saved `part.3dcraft`*, *downloaded `part.step`: see the
  browser's downloads*) or why it was not written. Closing a file dialog does nothing and says nothing.
- There is no autosave in the browser: a document lives in the page until you save it, and the browser
  asks before a page with unsaved changes is closed or reloaded.

## Autosave and recovery

The window autosaves documents with unsaved changes 30 s after a change, into a recovery directory in
your state directory (Linux: `~/.local/state/3dcraft/recovery`). After a crash or a kill, the next start
shows a bar with what was left: *Recover*, *Discard*, *Later*. The same is available as commands
(`recovery.list`, `recovery.open`, `recovery.discard`), also in `craft3d-cli serve` and `mcp` when they are
started with `--recovery-dir DIR`. Closing the window normally asks, per document with unsaved changes,
whether to save it.
