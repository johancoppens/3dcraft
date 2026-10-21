# Selection and selecting by description

The selection the developer and an agent share, and the query that selects by describing (`select.query`). Decided by the developer on 2026-10-21 ([`../decisions.md`](../decisions.md), B1 to B6); the protocol is in [`../control-protocol.md`](../control-protocol.md) › *Selection*. Code: `crates/engine/src/selection.rs` (the list and what names mean), `crates/engine/src/select_cmds.rs` (`select.get`, `select.set`, `select.query`), `crates/ui-egui/src/shell/selection.rs` (the window), `crates/automation/src/mcp.rs` (the resource and the tool).

## Method

### The selection

- **One ordered list per open document**, held by the engine's `OpenDocument` beside the history, not in it: not saved, not undone (B1). Every frontend sees the same list: the window draws it, the commands read and set it, MCP serves it as the resource `craft3d://selection` and the tool `selection_get`. Holding it in the engine rather than in the window's `UiState` is what lets a headless agent (`craft3d-cli`, MCP without a window) use it too, and keeps one list when the window and an agent both change it; the window keeps a copy as of its last sync for drawing.
- **An item is `{kind, name, feature}`.** A body, feature, sketch or datum is its feature, by id (`F3`), with the feature's name for reading; a face, edge or vertex is its persistent name ([`names.md`](names.md)) within the body of `feature`. Two items are the same when kind, feature and (for an entity) name agree, so a renamed feature stays selected.
- **What a name means** (`Resolver`): a feature by its name or id; a face by its persistent name or an alias (stored as its canonical name); an edge by its own name or an alias; a vertex by its own name. Without a kind it is read from the name. Lookups go through a per-body table built once per question, so selecting thousands of faces costs one pass over each body's names.
- **Through regeneration**: before it is read (and before it is changed) the list is checked against the document's current regeneration, once per revision. What resolves stays, in its order; what does not drops out and is listed under `dropped` ("face F7:copy(0)/F2:face(1): no body has a face …") until the selection is set again. A face edited upstream keeps its name and stays selected (the hole of 6 mm made 6.5 mm is still selected); a deleted feature's faces drop out; undo does not bring them back, since the selection is not history.
- **Modes**: replace, add (in order, no duplicates), remove, toggle (Shift+click), clear. At most 10000 items.

### In the window

- A click selects what *Pointing* says it points at (vertex, else edge, else face, each when it has a persistent name), replacing the selection; a click on nothing clears it; Shift toggles; a double click selects the body. The highlight of the last click (`state.highlight*`) is kept as before, and what is both highlighted and selected is drawn once (`Marks::with_selection`).
- **Picking through**: `craft3d_render::pick_all` is the renderer's own test of one sample, `pick_cut`'s, keeping every triangle under the position instead of the nearest, sorted by the depth key and one per face. A click within 3 pixels of the last one, with the camera and the model unchanged, or an Alt+click, selects the next face behind the last, round to the front. A right click lists the same faces (at most 24); hovering lights an entry's face.
- **The box** (Ctrl or Cmd and a drag; with Shift added): the scene's mesh points are projected with the renderer's `Viewpoint`; a body is selected when every point of it lands inside, otherwise its faces each of whose points does. Hidden faces count: a box selects what lies inside it, not what is seen (as a window selection in other modellers). Chosen over "touching the box" because a box around a hole should not take the plate.
- **Drawing**: the faces of selected bodies and faces join the highlight's face ids (`craft3d_render::Highlight`, the copper `face_highlight` of the Mission theme, eight runs of ids: a body's faces are one run, so whole bodies always show); the outlines of selected bodies and faces (their edges), selected edges and selected vertices are overlay marks, the highlight's line and dot, drawn dimmed where something is in front, so what is selected behind something else still shows. Outlines are capped at 40000 segments. The status bar says "3 bodies, 2 faces selected" and how many dropped out. `selectedFeature` (the feature list's row, the sketch drawn as selected) follows the first body, feature, sketch or datum of the selection; `ui.set {selectedFeature}` and a click on a row set the selection.

### `select.query`

JSON filters (B4), parsed and checked before anything is looked at: an unknown key, a value of the wrong dimension ("3 deg" for a radius, "3 mm" for an area), an axis without direction, more than 1000 filters or nesting deeper than 32 is an error that names it. Candidates are enumerated in document order (bodies, features, sketches, datums in history order; faces, edges, vertices body by body in the order of `document.inspect` and `document.edges`), at most 4 million, so the answer is deterministic. The result is cut at `limit` (default 1000, at most 10000) with `count` and `truncated` saying so. A filter that does not apply to an item (a volume of a face) does not hold for it.

| Filter | Holds for |
|---|---|
| `all`, `any`, `not`; an object with several keys, or an array | the combinations |
| `kind` | the kinds given |
| `name` | a glob (`*`, `?`) on the name; a feature-like item also on its id |
| `type` | a glob on the feature's type (`extrude`, `boolean`, …), of the body's feature for an entity |
| `in` | an entity of that body, or that body |
| `madeBy` | an entity whose persistent name has that feature anywhere along its way (`F6:copy(0)/F5:copy(1)/F3:copy(0)/F2:face(0)` is made by F6, F5, F3 and F2); a body or feature made from it, all the way through the features' sources |
| `copyOf` (`instance`) | an entity that is a copy made by that pattern or mirror (of that instance): its name's way is walked feature by feature through the instances (names.md, the role copy), as the appearance of a face is; that feature's body |
| `color` (`within`) | the colour of the face (as the viewport shades it, `face_appearances`) or of the body, within that distance in RGB; `null`: no colour of its own |
| `material` | a glob on the body's material; `null`: none |
| `volume`, `area`, `radius`, `length` | the exact measure within the range: a body's volume and area and a face's area by the integrals `document.measure` sums (`craft3d_topo::face_areas`), a radius of a cylindrical or spherical face or a circular edge, an edge's length (exact for lines and circular arcs). A value is a number in the document's unit or an expression with units (`"6.5 mm / 2"`); `{eq, tol}` (tol defaults to a millionth of eq plus the document's tolerance) or `{min, max}` |
| `surface` | the kind of a face's surface; `freeform` is a Bézier or NURBS surface |
| `hole` | a cylindrical face whose outward normal points at its axis (its material is outside it: the wall of a hole). A countersink (a cone) is not one yet |
| `normal` | `along`: a flat face whose outward normal is within `tol` (0.5 degrees) of the axis; `perpendicular`: a flat face whose normal is square to it, or a cylinder or cone whose axis lies along it |
| `axis` | a cylinder, cone or torus face, or a circular edge, whose axis lies along it, either way round |
| `inside`, `above`, `below` | the item's box (the B-rep's held box: exact for flat faces and straight edges, certainly containing a curved one) wholly within the box, at or above, at or below a level of x, y or z, with the document's tolerance as slack |

Each face, edge and vertex in the answer also has `body` and `origin` (the feature at the end of its name's way, the one that first made it: the cylinder that drilled a hole), which is what an agent edits to change it: "select all 6 mm holes and make them 6.5 mm" is a query for `{hole: true, radius: "3 mm"}` and a `feature.edit` of each origin's radius (`crates/engine/tests/selection.rs`).

### Not yet

- **Named sets** (B2): a query kept in the document under a name and computed again at every regeneration, like a formula: the next slice. The query's JSON is already the stored form.
- **Relations and extremes**: neighbours of, the same size as, symmetric to, coaxial with, the highest or largest or nearest *n*; distance to an item; faces of an edge's body side. Spatial words relative to the camera are not built (B3); directions are world axes.
- A cone (countersink) as part of a hole; edges' direction for `normal`/`axis`; the section's cap in the list of hits; picking datums and sketches by clicking.
- Preview and confirmation of large AI actions (B6).

## Findings and history

- **2026-10-21, slice 1** (branch `selection`): the shared selection, `select.get`/`select.set`/`select.query`, `ui.select`, `ui.get` › `selection`, `ui.pick`/`view.pick` with `all`, the window's click, Shift, double click, Alt and same-spot click-through, the right-click list, the box and Esc, the status bar, the MCP resource and tool. Oracles: closed-form counts and measures on a drilled plate, a block and stacked blocks; the showcase housing's six bolt holes of 7 mm, two oil holes and the three mirrored ones.
