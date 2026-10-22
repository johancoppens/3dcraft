# 4. Concepts

A few ideas explain nearly everything 3DCraft does, and nearly every message it gives.

## The history is the source of truth

A document is **variables, sketches, datums and an ordered list of features**, plus assertions and named
sets. That is what a `.3dcraft` file holds. The solid bodies are *derived*: 3DCraft rebuilds them from the
history (it *regenerates*) after every change, and rebuilds only the features downstream of what changed.
There is no way to push a face around behind the history's back: to change the model you change a
variable, a sketch or a feature's parameters, and the history is replayed.

Commands that change the history (`feature.edit`, `feature.suppress`, `feature.move`, `feature.delete`,
`feature.rename`, `history.rollback`):

- `feature.edit {"feature": "Holes", "count": 8}` changes some parameters of a feature, keeping its place
  and its name; it also repairs a feature that lost what it was made from (give the new sketch, target or
  edges).
- `feature.suppress` switches a feature off without deleting it; what is made from it fails until it is
  switched on again.
- `history.rollback {"feature": "Bored"}` rebuilds only up to that feature, and new features go in right
  after it: a way to insert something early in the history. `{"feature": null}` removes the marker.
- `edit.undo` and `edit.redo` undo and redo any change, one command at a time.

## Variables, expressions and units

Every length, angle and count may be an **expression** in the document's variables:

```json
[
 ["variable.set", {"name": "width", "expr": "60 mm"}],
 ["variable.set", {"name": "holes", "expr": "4"}],
 ["feature.block", {"size": ["width", "width * 2 / 3", "8 mm"], "name": "Plate"}]
]
```

- Units: `um`, `mm`, `cm`, `m`, `in`, `ft` for lengths, `deg` and `rad` for angles; a number without a
  unit is in the document's length unit (`file.new {"units": "mm"}`) where a length is expected. Units mix
  freely (`1 in + 3 mm` is 28.4 mm). An expression that cannot be worked out (`width + 3 deg`, an unknown
  name) is kept, but the variable *fails*: its result and `document.check` carry the error, and what uses
  it fails too until it is fixed.
- Functions: `sqrt`, `abs`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `min`, `max`, `floor`,
  `ceil`, `round`, `trunc`, `mod`, the constant `pi`, and `^` for powers.
- `variable.set` on an existing name changes it, and every feature that uses it is rebuilt.
  `variable.rename` renames it everywhere it is used. `variable.delete` refuses a variable that is still
  used and names what uses it; with `"force": true` it deletes it anyway, and what used it then fails
  (undo brings it back).

## Sketches and constraints

A sketch is a flat drawing on a plane: `xy`, `yz` or `zx` (each with an optional `offset`), a datum plane
(`datum.plane`), or a flat face of a body (`"face": "F2:endCap()"`). In the sketch, coordinates are the
plane's own `[x, y]`: on `xy` they are X and Y, on `yz` Y and Z, on `zx` Z and X; the sketch's normal is
the third axis (+Z, +X, +Y).

Entities are added with `sketch.point`, `sketch.line`, `sketch.polyline`, `sketch.rect`, `sketch.circle`,
`sketch.arc`, `sketch.path` (lines and arcs end to end), `sketch.spline` and `sketch.pattern`. Each gets an
id, `e1`, `e2`, …, returned by the command. Then constraints (`sketch.constrain`) tie them down:
`coincident`, `pointOn`, `horizontal`, `vertical`, `parallel`, `perpendicular`, `tangent`, `equal`,
`concentric`, `symmetric`, `midpoint`, `fixed`, `x`, `y`, `distance`, `angle`, `radius`, `diameter`.
3DCraft's own solver solves the sketch at every regeneration.

**Coordinates written as expressions are held.** In `"points": [["0 mm", "0 mm"], [0, "leg_x"]]` each
point that has an expression among its coordinates is held where the expressions put it (by `x` and `y`
constraints, which follow the variables); a point given only as plain numbers, `[0, 0]`, is a starting
place the solver may move. `sketch.inspect` reports `dof`, the degrees of freedom left: 0 means fully
constrained, which is what you want for a part that must follow its variables.

## Features

A feature makes or changes a body: primitives (`feature.block`, `feature.cylinder`, `feature.sphere`,
`feature.cone`, `feature.torus`), features from sketches (`feature.extrude`, `feature.revolve`,
`feature.sweep`, `feature.loft`, `feature.fill`), combinations (`feature.boolean`, `feature.pattern`,
`feature.mirror`, `feature.transform`), and operations on faces and edges (`feature.fillet`,
`feature.chamfer`, `feature.shell`, `feature.offsetFace`, `feature.moveFace`, `feature.draft`,
`feature.deleteFace`, `feature.thicken`, `feature.thread`). Chapter 6 goes through them.

Each feature has an id (`F1`, `F2`, … in order) and a name (given with `name`, or made up). Most take the
body of another feature and *consume* it: after `feature.boolean {"target": "Disc", "tool": "Bore", "name":
"Bored"}` there is one body, `Bored`; `Disc` and `Bore` are steps on the way. Build on the newest one.

## Persistent names

Features refer to faces and edges by **names that say how the face was made**, never by a number in the
current body:

| Name | Means |
|---|---|
| `F1:face(1)` | Face 1 of primitive F1 (for a block: 0 bottom, 1 top, 2 front, 3 back, 4 left, 5 right) |
| `F2:side(e9)` | The side an extrusion or revolution F2 swept from sketch curve `e9` |
| `F2:startCap()`, `F2:endCap()` | The flat faces where extrusion F2 starts and ends |
| `F3:fillet(0)` | The first round fillet F3 made |
| `F6:copy(1)/F4:face(0)` | Face 0 of F4 as it is in F6's body, where F4 came in as the tool (instance 1) |
| `F6:copy(1,3)/F4:face(0)` | The same in instance 3 of a pattern |

An edge is named by the two faces it lies between: a fillet takes `"edges": [["F2:side(e9)",
"F2:side(e10)"]]`. A feature that works on a body (a fillet, a chamfer, a shell, a thread) may use the
names of an earlier body of its chain; they are followed to the newest one (`["F1:face(0)", "F1:face(2)"]`
on the flange after two booleans).

You rarely type these: `document.inspect` lists every body's `faceNames`, a click in the window gives the
name of the face, and `select.query` finds faces by what they are (`{"hole": true, "radius": "4 mm"}`,
chapter 8).

**What happens when you edit upstream.** Because names come from how a face was made, an edit upstream
(a longer leg, more holes, a different radius) keeps every name that still means something, and the
fillet stays on its edge. When an edit takes away what a feature refers to, the feature **fails and says
so**, naming what it lost; it is never quietly moved onto another face.

## Failed features

A feature that cannot be built is kept in the history with the status `failed` and a reason, and
regeneration carries on with the rest:

```text
variable.set {"name": "inner_r", "expr": "30 mm"}
document.check
-> {"ok": false, "problems": [
     {"feature": "F3", "name": "Rounded", "problem": "radius: too large for edges.0 (between F2:side(e9) and F2:side(e10)): with a radius of 30 mm the line along which the fillet touches the face F2:side(e10) leaves that face; the largest radius that fits there is below 26 mm"},
     {"feature": "F6", "name": "Drilled", "causedBy": "F3", "problem": "target: `Rounded` failed; fix it first"},
     {"feature": "F9", "name": "Part", "causedBy": "F3", "problem": "target: `Drilled` failed; fix it first"}]}
```

The reason starts with the parameter at fault (`radius`, `edges.0`, `target`), and `causedBy` points at the
first failure in a chain: fix that one (here, a smaller `inner_r`, or `edit.undo`) and the rest is rebuilt.
