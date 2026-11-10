# Selection and named sets

*Find faces, edges and bodies by what they are, share a selection with the agent, and keep groups of
them under a name.*

## Finding things by what they are

`select.query` finds bodies, faces, edges, vertices, features, sketches or datums by description, so an
agent never has to guess a face's name:

```json
[
 ["select.query", {"kind": "face", "where": {"hole": true, "radius": "hole_d / 2"}}],
 ["select.query", {"kind": "face", "where": {"normal": {"along": "z"}, "above": {"z": "10 mm"}}}],
 ["select.query", {"kind": "edge", "where": {"madeBy": "Rounded"}}],
 ["select.query", {"kind": "body", "where": {"volume": {"min": 1000}}}]
]
```

Filters: `name` (a glob such as `F2:side(*)`), `type` (of the feature: `extrude`, `boolean`, …), `in` (of
that body), `madeBy` (made by that feature, anywhere along the way), `copyOf` (a pattern's or mirror's
copies), `hole`, `surface` (`plane`, `cylinder`, `cone`, `sphere`, `torus`, `freeform`), `normal`
(`along` or `perpendicular` an axis), `axis`, `radius`, `length`, `area`, `volume`, `thickness` (a
value, `{eq, tol}` or `{min, max}`), `inside`, `above`, `below`, `color`, `material`, `set`, and `all`, `any`, `not` to combine
them. Several keys in one object must all hold.

With `"context": true` each item comes with its surface (radius, axis, normal), the feature that first
made it (`origin`), the pattern instance it is a copy of, and its `drivers`: the parameter that sets its
size with the exact `feature.edit` that changes it. "Make all 8 mm holes 8.5 mm" is one query and one
edit per driver.

## The selection

Each open document has one selection, shared by everyone: what you click in the window and what an agent
selects. `select.get` reads it (over MCP also the tool `selection_get`), `select.set` sets it (`items`,
`names`, or a `set`, with `mode` `replace`, `add`, `remove`, `toggle` or `clear`), and `select.query` with
`"then": "replace"` selects what it found. Commands that act on faces can take the selection
(`face.appearance {"selection": true, …}`).

The selection follows the design: a face you selected stays selected when an upstream edit changes its
size, is followed into the body a later boolean or pattern made of it, and drops out (with a reason under
`dropped`) when the edit takes it away. It is not saved and not undone.

## Named sets

A named set is a query kept in the document, computed again whenever it is used:

```json
[
 ["set.define", {"name": "bolt holes", "kind": "face", "where": {"hole": true, "radius": "hole_d / 2"}}],
 ["select.set", {"set": "bolt holes"}],
 ["assert.add", {"name": "one bolt hole each", "measure": "count", "of": {"set": "bolt holes"}, "eq": "n"}]
]
```

Add two holes and the set holds two more.

**Plates by thickness.** `thickness` is the plate thickness of a flat face: the distance through the body
to the flat face directly opposite it, parallel and facing the other way, measured along the inward
normal from points all over the face. A cut list by sheet thickness is then a set, and it follows the
variables:

```json
[
 ["variable.set", {"name": "tA", "expr": "4 mm"}],
 ["variable.set", {"name": "tB", "expr": "4 mm"}],
 ["feature.block", {"name": "A", "size": ["200 mm", "100 mm", "tA"]}],
 ["feature.block", {"name": "B", "size": ["200 mm", "100 mm", "tB"], "position": ["0 mm", "0 mm", "50 mm"]}],
 ["set.define", {"name": "Cut sheet: 4 mm", "kind": "face", "where": {"normal": {"along": "z"}, "thickness": "4 mm"}}],
 ["set.define", {"name": "Like A", "kind": "face", "where": {"normal": {"along": "z"}, "thickness": "tA"}}],
 ["variable.set", {"name": "tB", "expr": "3 mm"}],
 ["set.list", {}]
]
```

After `tB` = 3 mm, B leaves *Cut sheet: 4 mm*; give it 4 mm again and it is back. A value is matched
within the document's tolerance (and a millionth of it), `{min, max}` takes a range, and an expression of
the variables (`tA`) is worked out again whenever the set is. A face has a thickness only when every line
into the material from it is equally long and leaves through such an opposite face: a curved face, a face
over a step, a pocket or a slanted back has none, and never fits. This is not the measure `thickness` of `measure.thickness` and assertions ([Measuring and assertions](measure.md)), which finds the thinnest place of a whole body; the filter asks whether one flat face is a plate of one thickness. `set.list` says how many items each set holds now; `set.edit`
and `set.remove` change and remove them. Sets are saved with the document and renamed features keep
them working.

**Next:** [Files, export and drawings](files.md).
