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

- `feature.edit {"feature": "Holes", "count": "n"}` changes some parameters of a feature, keeping its place
  and its name; it also repairs a feature that lost what it was made from (give the new sketch, target or
  edges). A parameter given as a plain number (`"count": 8`) replaces the expression that was there: the
  feature no longer follows the variable.
- `feature.suppress` switches a feature off without deleting it; what is made from it fails until it is
  switched on again. That includes a suppressed boolean, fillet or chamfer: it does not hand the body it
  changes on to the features after it, so they fail too (other modellers pass the body through).
- `feature.move {"feature": "Round", "position": 3}` moves a feature to that index in `document.inspect`'s
  list of features (sketches and datums count); one moved before what it is made from fails and says so.
- `feature.rename` renames a feature or a sketch (sketches are features); `variable.rename` a variable,
  everywhere it is used, assertions included.
- `history.rollback {"feature": "Bored"}` rebuilds only up to that feature, and new features go in right
  after it: a way to insert something early in the history. `{"feature": null}` removes the marker. A
  feature inserted there that takes a body (a fillet, a chamfer, a boolean) takes it from the features
  after the marker that name that body (a boolean's `target` or `tool`, the `source` of a transform, mirror,
  pattern or shell): when the marker is removed they fail with *the body of `Bored` already went into
  `Bevel`*, and the message gives the `feature.edit` that points them at the new feature
  (`{"feature": "Drilled", "target": "Bevel"}`). A fillet, a chamfer or a face operation after the marker
  names faces, not a body, and follows them into the new body by itself; there is nothing to re-point.
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

- Units: `um`, `mm`, `cm`, `m`, `in`, `ft` for lengths, `deg` and `rad` for angles, in lower case; a
  number without a unit is in the document's length unit (`file.new {"units": "mm"}`) where a length is
  expected. Units mix freely (`1 in + 3 mm` is 28.4 mm). An expression that cannot be worked out
  (`width + 3 deg`, an unknown name) is kept, but the variable *fails*: its result and `document.check`
  carry the error, and what uses it fails too until it is fixed. A variable cannot be defined from itself:
  "make the wall 1 mm thicker" is `{"name": "wall", "expr": "3 mm"}`, not `wall + 1 mm` (refused). Only
  that direct self-reference is refused: a cycle through other variables (`a` = `b + 1 mm`, then `b` =
  `a + 1 mm`) is kept, and every variable on it fails with *circular definition* until one is changed.
- **A unit belongs to the number just before it.** `1/16 in` is 1 / (16 in), an inverse length, and
  `pi / 4 rad` is pi / (4 rad): write `1 in / 16` and `pi / 4 * 1 rad`. A unit after a parenthesis or a
  name is refused (`(pi / 4) rad`); multiply by one instead (`(pi / 4) * 1 rad`). When a variable comes
  out as an inverse length or angle because a `/` is followed by a number with a unit, its result has a
  `warning`; an inverse written as meant (`1 / (2 mm)`, `10 mm / (360 deg)`) has none.
- **Angles.** A number without a unit where an angle is expected (a pattern's `angle`, a transform's) is
  in degrees. `pi` is a plain number, so `2 * pi / n` there is a few degrees, not a turn: write
  `360 deg / n`. Inside `sin`, `cos` and `tan` a plain number is in radians: write `sin(30 deg)`. An angle
  variable's `value` is in radians (`angleUnit: "rad"`); its result also gives `degrees`.
- The variables and their values are listed by `document.inspect` (`variables`); there is no separate
  list command.
- Functions: `sqrt`, `abs`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `min`, `max`, `floor`,
  `ceil`, `round`, `trunc`, `mod`, `exp`, `ln`, `log10`, `pow`, `sinh`, `cosh`, `tanh`, the conditions
  below, the constant `pi`, and `^` for powers (a whole-number exponent; `pow(x, y)` takes any exponent of
  a plain number).

### Conditions

A comparison (`<`, `<=`, `>`, `>=`, `==`, `!=`) is 1 when it holds and 0 when it does not; both sides
have the same dimension. `&&` (and), `||` (or) and `!` (not) combine them. Three functions choose:

```json
[
 ["variable.set", {"name": "length", "expr": "240 mm"}],
 ["variable.set", {"name": "size", "expr": "2"}],
 ["variable.set", {"name": "ribs", "expr": "if(length > 200 mm, 4, 2)"}],
 ["variable.set", {"name": "wall", "expr": "clamp(length / 50, 1.2 mm, 5 mm)"}],
 ["variable.set", {"name": "bore", "expr": "choose(size, 3 mm, 4 mm, 5 mm)"}]
]
```

- `if(condition, a, b)` is `a` when the condition holds, else `b`. Only the chosen one is worked out, so
  the other may fail: `if(n > 0, total / n, 0 mm)` does not divide by zero. `a` and `b` have the same
  dimension; the branch not taken is only checked for that when it can be worked out. A name that is
  no variable at all (a typo) is refused in either branch, so it shows before the condition flips.
- `clamp(x, low, high)` keeps `x` between the bounds; `low` may not be above `high`.
- `choose(i, a, b, ...)` is the `i`-th value, counted from 1 (`i` a whole number).
- `==` treats two values as equal when they differ by less than a part in 10^12, so `0.1 + 0.2 == 0.3`
  holds. Compare values with each other, not their difference with zero: `sin(pi) == 0` is false.
- Order, loosest first: `||`, then `&&`, then `!`, then the comparisons, then `+ -`, `* /` and `^`:
  `a > 1 && b > 1 || c` is `((a > 1) && (b > 1)) || c`, `!a > b` is `!(a > b)` and `!a + b` is
  `!(a + b)`: put the operand of `!` in parentheses when you mean less (`(!a) + b`). `a < b < c` is
  refused: write `a < b && b < c`.
- `variable.set` on an existing name changes it, and every feature that uses it is rebuilt.
  `variable.rename` renames it everywhere it is used. `variable.delete` refuses a variable that is still
  used and names what uses it; with `"force": true` it deletes it anyway, and what used it then fails
  (undo brings it back).

### Curves from formulas

`sketch.curve` draws the curve `x(t)`, `y(t)` for `t` over a range, in the sketch's axes. `x` and `y` are
expressions in `t` and the document's variables; inside them `t` is the curve's own parameter, also when
a variable is called `t` (that variable is not reached there, and a variable used there cannot be renamed
to `t`). The range `t: [from, to]` is two expressions in the variables, both plain numbers, both angles
or both lengths. A plain `t` inside `sin`, `cos` and `tan` is in radians. An Archimedean spiral of one
turn, closed along the x axis:

```json
[
 ["variable.set", {"name": "a", "expr": "10 mm"}],
 ["variable.set", {"name": "b", "expr": "3 mm"}],
 ["sketch.create", {"name": "Spiral"}],
 ["sketch.curve", {"sketch": "Spiral", "x": "(a + b * t) * cos(t)", "y": "(a + b * t) * sin(t)", "t": [0, "2 * pi"]}],
 ["sketch.line", {"sketch": "Spiral", "from": "e2", "to": "e1"}],
 ["feature.extrude", {"sketch": "Spiral", "distance": "2 mm"}]
]
```

The flank of an involute gear tooth, from the base circle `rb` to the tip circle `ra`, is
`x = rb (cos(t) + t sin(t))`, `y = rb (sin(t) - t cos(t))` for `t` from 0 to `sqrt((ra / rb)^2 - 1)`;
the [gear example](../examples/gear.json) turns it to its place by adding an angle `g` to `t` inside the
`sin` and `cos`, and mirrors it for the other flank.

- The curve is fitted again at every regeneration, so it follows the variables: a cubic B-spline within
  half the document's tolerance of the formulas at every point checked (three in every span between
  samples, refined until they hold), with its ends exactly where the formulas put them. `sketch.curve`
  and `sketch.inspect` give the `fit`: `controlPoints` and `deviation`, the largest distance found.
- It ends on two points (`e1` and `e2` above; or `start` and `end`, existing points), which the curve holds
  in place: lines, arcs and splines built on them close a profile with it. A constraint that would move
  them fails the sketch. When the ends meet the curve is closed (one point) and a profile on its own.
- A formula that fails somewhere in the range (`1 / (t - 1)`), is not finite, gives a single point, or
  jumps cannot be fitted: the command, or later the sketch, says why and near which `t`. A kink is
  rounded within the tolerance or refused; split the curve there into two for a sharp corner.
- A formula curve has no unknowns and takes no `tangent` or `pointOn` constraint yet.

**A spline through computed points.** `sketch.spline` with `"live": true` and `through` keeps the points
as they are and fits the spline through them again at every regeneration, so points held by expressions
(`["r * cos(30 deg)", "r * sin(30 deg)"]`) or by constraints move it; without `live` the spline is fitted
once and then shaped by its control points.

## Sketches and constraints

A sketch is a flat drawing on a plane: `xy`, `yz` or `zx` (each with an optional `offset`), a datum plane
(`datum.plane`), or a flat face of a body (`"face": "F2:endCap()"`). In the sketch, coordinates are the
plane's own `[x, y]`: on `xy` they are X and Y, on `yz` Y and Z, on `zx` Z and X; the sketch's normal is
the third axis (+Z, +X, +Y). On a face the axes follow the face, not the world: on the bottom of a block
the sketch's x is world Y, its y world X, and its normal points down, out of the body. `sketch.create`
answers with the `frame` (origin, x and y in world coordinates) and the `normal`; read them before
placing anything on a face. To cut through a part from below, a sketch on the `xy` plane is often
simpler.

Entities are added with `sketch.point`, `sketch.line`, `sketch.polyline`, `sketch.rect`, `sketch.circle`,
`sketch.arc`, `sketch.path` (lines and arcs end to end), `sketch.spline`, `sketch.curve` (a curve from
formulas, above) and `sketch.pattern`. Each gets an id, `e1`, `e2`, …, returned by the command. Then
constraints (`sketch.constrain`) tie them down:
`coincident`, `pointOn`, `horizontal`, `vertical`, `parallel`, `perpendicular`, `tangent`, `equal`,
`concentric`, `symmetric`, `midpoint`, `fixed`, `x`, `y`, `distance`, `angle`, `radius`, `diameter`.
3DCraft's own solver solves the sketch at every regeneration.

**Coordinates written as expressions are held.** In `"points": [["0 mm", "0 mm"], [0, "leg_x"]]` each
point that has an expression among its coordinates is held where the expressions put it (by `x` and `y`
constraints, which follow the variables); a point given only as plain numbers, `[0, 0]`, is a starting
place the solver may move. `sketch.inspect` reports `dof`, the degrees of freedom left: 0 means fully
constrained, which is what you want for a part that must follow its variables. A sketch with freedom
left builds without a warning, so check `dof` yourself.

A constraint that repeats what others already say is `redundant` (listed, harmless while they agree); one
that contradicts them is `conflicting`: the sketch fails, and so does what is made from it. Its result
names it (`"conflicting": ["c10"]`); change its value with `sketch.edit {"sketch": "R", "constraint":
"c10", "value": "40 mm"}` or remove it with `sketch.delete {"sketch": "R", "constraint": "c10"}`. A
redundant constraint becomes a conflicting one when a variable changes (a length written as `40 mm` next
to a rectangle drawn to `w`), so prefer one constraint per measure.

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
