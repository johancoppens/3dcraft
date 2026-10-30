# 10. Measuring and checking

*Ask the design questions (how heavy, how thick, does it fit) and keep the answers that matter as
checks that run at every change.*

An agent should check a design by asking it questions, not by looking at a picture. 3DCraft answers on
the exact geometry, and can keep the questions in the document as **assertions** that are checked again
after every change.

## Measures

| Command | Answers |
|---|---|
| `document.inspect`, `document.measure` | Volume, area, centroid, bounding box of every body; mass and inertia with a material |
| `measure.distance {"from", "to"}` | The least distance between faces or bodies, and where |
| `measure.clearance {"from", "to"}` | The gap between bodies; an overlap has none |
| `measure.thickness {"of"}` | The thinnest wall of a body (or from chosen faces), and where |
| `measure.through {"of"}` | Whether holes (cylindrical or conical faces) go clear through along their axis |
| `measure.ray {"from", "direction"}` | Every place a line meets the bodies, entering or leaving |
| `measure.symmetry {"of", "plane"}` | How far the bodies are from symmetric in a plane |
| `document.interference` | For each pair of bodies: clear, touch or overlap (with the common volume) |
| `measure.continuity {"edge", "face"}` | How two faces meet along an edge: G0, G1 or G2, with the largest gaps in position, tangent-plane angle, curvature and twist, proven ([continuity](#continuity-across-a-seam)) |
| `measure.below`, `measure.section`, `measure.levelFor`, `measure.floating` | The volume below a plane, a section's area and second moments, the level for a volume, the level and tilt a body floats at ([below](#below-a-plane-sections-and-levels)) |

A subject (`of`, `from`, `to`) is a body by its feature's name, a list of them, or a query as
`select.query` takes it ([chapter 11](11-selection.md)): `{"hole": true, "radius": "4 mm"}` for the 8 mm holes,
`{"kind": "face", "where": {"normal": {"along": "z"}}}` for the faces facing up.

What is searched for (a thickness, a distance) comes with what is **proven**: `atLeast`, a bound nothing
is below, beside the value found at a real place. On the flange of [chapter 3](03-agent.md#a-full-session): `"thickness": 6.0,
"atLeast": 5.75`.

## Below a plane, sections and levels

Three questions about a plane through closed bodies, answered on the exact geometry (the part below
the plane is cut off by the kernel and integrated; the section's moments come from the curves in which
the plane cuts the bodies). The plane is given as `measure.symmetry` takes it (`xy`, `yz`, `zx`, a datum
plane, `{"point", "normal"}`) or as a flat face by its name (`"F1:face(1)"`, with its outward normal),
moved by `offset` along its normal; the default is the xy plane. *Below* is the side the normal points
away from.

| Command | Answers |
|---|---|
| `measure.below {"of", "plane", "offset"}` | The `volume` below, its `centroid`, the `surface` below, and the `section` |
| `measure.section {"of", "plane", "offset"}` | The section: `area`, `centroid`, `ix` and `iy` (second moments about its centroid along `xAxis` and `yAxis`), `ixy`, the principal `major` and `minor` with `majorAxis` |
| `measure.levelFor {"of", "plane", "volume"}` | The `offset` at which the volume below is `volume`, and what `measure.below` gives there |
| `measure.floating {"of", "plane", "volume", "centerOfGravity"}` | The level and tilt a body floats at: the equilibria, each stable, unstable or neutral ([floating free](#below-a-plane-sections-and-levels)) |

```json
[
 ["feature.block", {"size": [4000, 1300, 1000], "position": [-2000, -650, 0], "name": "Tank"}],
 ["measure.below", {"of": "Tank", "offset": "300 mm"}],
 ["measure.levelFor", {"of": "Tank", "volume": "1.56 * (1 m)^3"}],
 ["measure.section", {"of": "Tank", "plane": "yz"}]
]
```

The first answers `volume` 1.56e9 (mm³), `centroid` [0, 0, 150]; the second, the fill level for 1.56 m³ of
liquid, `offset` 300 (the kernel finds it by Newton's method, `steps` saying how many levels it measured);
the third, the cross-section square to x: `area` 1.3e6, with `xAxis` y and `yAxis` z, `ix` = 1300 × 1000³/12
(about the axis along y) and `iy` = 1000 × 1300³/12.

- `offset` is a length, `volume` a volume, both expressions of the variables; `xAxis` sets the
  section's x axis (square to the normal; by default the world axis most square to it).
- Errors you can act on: *outside (0, total)* (no plane leaves that volume below), *a sheet has no
  inside* (close it first), *did not converge* after 64 steps.
- Bodies that overlap are counted once each.
- Floating objects: a body of mass m in a liquid of density ρ settles where `levelFor` of m/ρ puts the
  plane; a Jig computes what it needs from the centroid and the section's moments (the worked example
  is in [`design/sections-and-levels.md`](../design/sections-and-levels.md)).

**Floating free.** `measure.floating` finds the plane a floating body settles at when its centre of
gravity is known too: the level *and* the tilt at which the volume below is `volume` (or `mass` /
`density`, kg and kg/m³) and the centroid below lies straight under the centre of gravity. `plane` is the
upright waterplane (default xy, up +z); `heel` and `trim` tilt the start.

```json
[
 ["feature.block", {"size": [1000, 200, 200], "position": [-500, -100, 0], "name": "Pontoon"}],
 ["measure.floating", {"of": "Pontoon", "mass": 20, "density": 1000, "centerOfGravity": [0, 0, 50]}],
 ["measure.floating", {"of": "Pontoon", "mass": 20, "density": 1000, "centerOfGravity": [0, 0, 90]}]
]
```

The first answers one equilibrium, upright at `offset` 100, `stable`, with its two metacentric heights
under `principal` (`gm` 33.3 mm about the length, 833 mm about the width). The second has its centre of
gravity too high: upright is an equilibrium but `unstable`, and the two it falls to follow, heeled 32.3°
to each side (the box's angle of loll); `floats` is the index of the first stable one. Each equilibrium
gives `heel`, `trim` and `tilt` in degrees, `normal`, `centroid`, `arm` (how far the centre of gravity is
off the vertical through the centroid) and what `measure.below` gives there. The search starts from the
upright plane (or `heel`, `trim`), finds the nearest equilibrium by Newton's method and, where that one is
unstable, the ones the body falls to from it; other equilibria are not searched. `neutral` says that the
first one turns freely in some direction (a sphere with its centre of gravity at its centre).

As assertions: `volumeBelow`, `centroidBelow` (with `axis`), `sectionArea`, `sectionCentroid` (with
`axis`), `sectionInertia` (with `about`: `x`, `y`, `xy`, `major` or `minor`), `sectionExtent` (with
`axis`: `x`, `y`, `z` or `[x, y, z]`; how far the section reaches along it, the length of a hull's
waterline at its level) and `levelFor` (with `volume`), each with the same `of`, `plane`, `offset`
(not for `levelFor`) and `xAxis` (not for `sectionExtent`):

```json
[
 ["assert.add", {"name": "stiff enough", "measure": "sectionInertia", "of": "Tank", "plane": "yz", "about": "minor", "min": "1e11"}],
 ["assert.add", {"name": "fill level", "measure": "levelFor", "of": "Tank", "volume": "1.56 * (1 m)^3", "max": "350 mm"}]
]
```

**Floating, as assertions and outputs.** What `measure.floating` finds can be asserted too, so a design that
floats upright keeps doing so as it changes: `floatingOffset` (the waterplane's offset along its normal),
`floatingHeel` and `floatingTrim` (its angles from the upright plane; write the bounds with a unit, `"3 deg"`),
`floatingGM` and `floatingGML` (the metacentric heights for a heel, a turn about the plane's x axis, and for a
trim, about its y axis) and `floatingStable` (`is`: stable both ways; *notProven* when it is neutral within measurement), all of the
equilibrium reached from the start. They take the parameters of `measure.floating`, and every value may be a formula of the variables and
the outputs, so the centre of gravity can be computed by the design:

```json
[
 ["feature.block", {"size": [2000, 600, 400], "position": [-1000, -300, 0], "name": "Barge"}],
 ["variable.set", {"name": "KG", "expr": "180 mm"}],
 ["variable.set", {"name": "xg", "expr": "64 mm"}],
 ["variable.output", {"name": "zG", "expr": "KG"}],
 ["assert.add", {"name": "upright", "measure": "floatingGM", "of": "Barge", "mass": 300, "density": 1000, "centerOfGravity": ["xg", 0, "zG"], "min": "0 mm"}],
 ["assert.add", {"name": "trim", "measure": "floatingTrim", "of": "Barge", "mass": 300, "density": 1000, "centerOfGravity": ["xg", 0, "zG"], "min": "-3 deg", "max": "3 deg"}],
 ["variable.measure", {"name": "GMf", "measure": "floatingGM", "of": "Barge", "mass": 300, "density": 1000, "centerOfGravity": ["xg", 0, "zG"]}]
]
```

The barge settles 2.86° down towards +x, where its centre of gravity lies (a trim of tan⁻¹ 0.05, the wall-sided closed form) with a GM of
66.75 mm, so both pass; with `KG` at 300 mm its GM is negative and `upright` fails. A search for the
equilibrium that does not settle makes such an assertion *notProven*, with the reason, never passed, and
an output of it unknown; a displacement more than the body holds is an error (it sinks). Each assertion
solves the equilibrium itself, and again only when what it depends on changes.

## Assertions

`assert.add` keeps a measure and what it must be:

```json
[
 ["assert.add", {"name": "volume", "measure": "volume", "of": "Case", "eq": "(length * width - (4 - pi) * corner_r^2) * height - ((length - 2 * wall) * (width - 2 * wall) - (4 - pi) * (corner_r - wall)^2) * (height - wall)"}],
 ["assert.add", {"name": "walls", "measure": "thickness", "of": "Case", "min": "wall - 0.1 mm"}],
 ["assert.add", {"name": "holes go through", "measure": "through", "of": {"hole": true}}],
 ["assert.add", {"name": "six bolt holes", "measure": "count", "of": {"set": "bolt holes"}, "eq": 6}]
]
```

- Measures: `volume`, `area`, `mass`, `count`, `extent` (size along `axis`), `boxMin` and `boxMax`
  (where the box begins and ends along `axis`, so each side is bounded on its own), `centerOfMass`
  (its coordinate along `axis`; every body needs a material), `distance`, `clearance`, `thickness`,
  `through`, `overlap` (whether bodies overlap), `symmetric`, and `volumeBelow`, `centroidBelow`,
  `sectionArea`, `sectionCentroid`, `sectionInertia`, `sectionExtent`, `levelFor`, `floatingOffset`, `floatingHeel`,
  `floatingTrim`, `floatingGM`, `floatingGML`, `floatingStable` ([above](#below-a-plane-sections-and-levels)), the
  sheet-metal measures and `bendable` (the press-brake check, [chapter 9](09-features.md#sheet-metal)),
  `value`, a formula of the variables and outputs (`expr`, [below](#outputs-measures-in-formulas)), and
  `continuity` (`edge`, `face`: the order a seam reaches as a number, −1 none, 0 G0, 1 G1, 2 G2; `"min": 2`
  asserts G2, [below](#continuity-across-a-seam)).
- Bounds: `eq` (with `tol`, by default a millionth of the value; `0` is allowed), or `min` and/or
  `max`; `is` for the yes-or-no measures (`true` by default, except for `overlap`, which wants it
  said). Bounds are expressions in the document's variables, so they follow the design: the case's
  volume is right for any `wall`.
- An assertion may be written before the model meets it, even before its subject exists (it is in
  *error* until then).
- An assertion names a feature's body as it is at that feature. A feature added on top of it later (a
  hole cut into `Case`, giving `Vented`) takes that body, and the assertion turns to *error*: *the body
  of `Case` went into F6, `Vented`; name that feature*. Point it at the new end of the chain, with the
  bound changed if the change was meant: `assert.edit {"name": "volume", "of": "Vented", "eq": "…"}`.
  Or let the subject follow the body: `{"body": "Case"}` (short for `{"body": "Case", "follow": true}`)
  is the newest body that grew from `Case`, through every feature that took it since (a hole, a fillet,
  a move, a boolean with it as the target or as a union's tool), so it needs no edit when one is added.
  A body that went into a subtraction or an intersection as the tool, or was split into pieces, did not
  grow into one body: that is an error that says so. Queries (`{"hole": true}`, a named set) look at
  whatever bodies there are and need no edit.

Each assertion has a status: **passed**; **failed** (with what was measured and expected);
**notProven** (a thickness or distance whose found value is within the bound but whose proven bound is
not: a warning, never a pass); **error** (it could not be measured: its subject names nothing, or a body
is missing); **inactive** (its condition `when` is 0: [below](#assertions-that-apply-only-sometimes));
**pending** (still being measured, on a large model). They are measured off to the side and
only when what they depend on changed, so they never slow modelling down or stop a feature.

Where they show: in the result of every feature command, `variable.set` and `param.set` (`assertions:
{count, passed, broken, unproven, pending, inactive}`), in `assert.list` (each with its message), in `document.check` (a broken one makes
`ok` false), in `document.inspect`, and in the window's tree. `assert.edit` and `assert.delete` change
and remove them.

**Pending after an edit.** An edit's result waits up to two seconds for the assertions it touched; the
rest count under `pending`, and `whenPending` then says how to have them. A script that checks its
assertions after a change either gives the edit `"wait": true` (`param.set`, `variable.set`), which waits
until every assertion is measured (a minute at most: what takes longer stays `pending`, with
`whenPending`), or runs `assert.list` next, which waits by default:

```json
[
  ["param.set", {"values": {"teeth": 31}, "wait": true}],
  ["assert.list", {}]
]
```

A real *notProven*: with the bound written as `"min": "wall - 0.01 mm"`, the case's thickness answers
*thickness: not proven: found 2 mm (at least 1.95750083 mm), expected ≥ 1.99 mm*. The wall is 2 mm, but
the proof only reaches 1.9575 mm near the rounded corners; a bound of `wall - 0.1 mm` passes.

Three that need a word:

- **The centre of mass** of the bodies weighs each body's centroid by its mass, so different materials
  pull it their way. A CubeSat's centre of mass must lie within 20 mm of its geometric centre in x and
  y and 70 mm in z: with that centre in variables, `{"name": "cg x", "measure": "centerOfMass",
  "axis": "x", "min": "cx - 20 mm", "max": "cx + 20 mm"}`, and the same for y and z. A body without a
  material is an *error*, as for `mass`: *`Lid` has no material, so no mass; give it one with
  body.material*.
- **Each side of the box.** `extent` bounds the size, the sum of both sides. To let nothing stick out
  more than 6.5 mm beyond rails of width `w` centred on the origin, on +x and on -x separately:
  `{"measure": "boxMax", "axis": "x", "max": "w / 2 + 6.5 mm"}` and `{"measure": "boxMin", "axis":
  "x", "min": "-w / 2 - 6.5 mm"}`. Coordinates are in the model's frame. The box of a body with spline faces is found on a fine mesh: the
  value is a side the body reaches, and it stays within twice the mesh's tolerance beyond it (within
  what the tessellator measures of its own accuracy; the box of the spline's whole control net is the
  outer limit that always holds). A bound that falls in between is looked at again on finer meshes,
  down to ten times the document's tolerance; what is still in between is *notProven*. So an `eq` on
  such a body wants a `tol` of a few ten-thousandths of its size (or a `min`/`max` with that room); a
  body too large to mesh within two million triangles gives only the outer limit, which settles a
  `max` on `boxMax` or `extent` and a `min` on `boxMin` and leaves the rest *notProven*.
- **Overlap** is true when bodies overlap, so `{"measure": "overlap", "of": ["Bolt", "Plate"], "is":
  false}` asserts they do not (`to` compares `of` with other bodies instead of with each other; leave
  both out for all bodies). Its message says it in words: *no overlap*, or *Bolt overlaps Plate by
  12 mm^3, expected no overlap*. `is: true` asserts a press fit. Before 2026-10-25 this measure was
  `interference`, true when *none* overlap; files that have it are brought up to date when they open
  (it becomes `overlap` with the opposite `is`, meaning the same), and a command that still sends it
  is taken the same way.

One trap: in an expression a power binds to the number with its unit, so `50 cm^3` would mean
(50 cm)³; it is refused wherever a bound is read. Write `50 * (1 cm)^3`, or a plain number in the
document's unit (mm³ in a mm document). Only `0` and `1` may be written so (`"tol": "0 mm^3"`): for them
the two readings are the same.

## Continuity across a seam

`measure.continuity {"edge": "F7:startEdge(0,2)", "face": "F1:side(0)"}` says how a sheet meets a face
along an edge (without `face`, the edge's two faces). `order` is the highest of **G0** (in position), **G1**
(one tangent plane) and **G2** (the same curvature too: normal curvature across and along the edge, and
twist) whose gaps are *proven* within their thresholds all along the edge, not only where it was looked at:
`position`, `angle` (radians), and per length unit `curvature` (the difference of the normal curvatures
across the edge), `twist` (of how fast the normal turns about the edge along it) and `along` (of the normal
curvatures along the edge), all measured along the second face's normal, each give `bound` (nothing along the edge is
beyond it), `found` (the largest seen, with `param` and `at`), `threshold` and `within`.

The thresholds are the document's: the tolerance, the angular tolerance, and for curvature, twist and along
the smaller of angular / L and 2 × tolerance / L², L the edge's length (what turns the normals apart by the
angular tolerance, or opens a gap of the tolerance, over that length). Nearly G2 is not G2: a seam whose
curvature differs by more than that is G1, and so is one whose cross tangent turns about the edge within the
angular tolerance but enough to twist it beyond the curvature threshold.
One whose bounds cannot be brought within the thresholds is not reached either (`exhausted` says when the
work allowed ran out first). One of the faces must be a spline face the edge runs along a parameter line of (a network sheet's
edge is); the other a plane, cylinder, cone or sphere, or a spline face in the same parametrisation along the
seam. Two analytic faces and a torus are refused, with the reason.

```json
[
 ["assert.add", {"name": "fin is G2", "measure": "continuity", "edge": "F7:startEdge(0,2)", "face": "F1:side(0)", "min": 2}]
]
```

## Outputs: measures in formulas

Some answers are formulas of measures: a boat's stability, the metacentric height GM = KB + BM − KG,
comes from where it floats, the centre of what is under water and the waterplane's moment. An **output**
is a variable 3DCraft works out *after* the model is built, from a measure or from a formula of other
outputs; you read it and check it, you never set it.

```json
[
 ["variable.set", {"name": "KG", "expr": "180 mm"}],
 ["variable.set", {"name": "V", "expr": "0.3 * (1 m)^3"}],
 ["feature.block", {"size": ["2000 mm", "600 mm", "400 mm"], "position": ["-1000 mm", "-300 mm", "0 mm"], "name": "Hull"}],
 ["variable.measure", {"name": "T", "measure": "levelFor", "of": "Hull", "volume": "V"}],
 ["variable.measure", {"name": "KB", "measure": "centroidBelow", "of": "Hull", "offset": "T", "axis": "z"}],
 ["variable.measure", {"name": "I", "measure": "sectionInertia", "of": "Hull", "offset": "T", "about": "x", "xAxis": "x"}],
 ["variable.output", {"name": "BM", "expr": "I / V"}],
 ["variable.output", {"name": "GM", "expr": "KB + BM - KG"}],
 ["assert.add", {"name": "stable", "measure": "value", "expr": "GM", "min": "0 mm"}],
 ["output.list", {}]
]
```

The hull floats at `T` = 250 mm (the level below which it holds 0.3 m³), `KB` = 125 mm, `I` = 2000 × 600³/12
mm⁴, so `BM` = 120 mm and `GM` = 65 mm: the assertion *stable* passes. Set `KG` to 300 mm and GM is −55 mm,
and *stable* fails; only the assertion is measured again, since the waterline does not depend on `KG`.

- `variable.measure` takes a measure and its parameters as `assert.add` does ([the measures](#assertions));
  the output is the number with its unit (a length, an area, a volume, mm⁴; a mass in kg and a count as
  plain numbers, yes-or-no as 1 or 0). Its formulas (`offset`, `volume`) may use other outputs: `KB` is
  measured at the waterline `T`.
- `variable.output` is a formula of variables and outputs. `variable.measure` or `variable.output` on an
  existing output changes it; `variable.rename` and `variable.delete` take outputs as they take variables.
- **Only outputs and assertions may use an output.** A variable, a feature, a sketch, a condition, a named
  set or a material that uses one is refused, and the message names it (*the variable `beam` uses `GM`, an
  output*): the model is built first, then measured, so a measure can never depend on itself. Give such a
  size a variable of its own.
- An output that would depend on itself is refused with the loop named: *`B` would depend on itself
  (B → A → B)*. A name in a branch of `if` that is not taken counts too: `A = if(on == 1, B, 0 mm)` with
  `B = if(on == 1, 2 mm, A)` is refused as a loop, although at `on` = 1 neither needs the other. `variable.set`, `param.set` and `variable.publish` refuse an output: it is read-only.
- **Unknown, never a guess.** When a measure fails (a volume the hull cannot hold), or what it proves is
  wider than the document's tolerance (a wall found at 2 mm but proven only above 1.96 mm), the output is
  *unknown* with the reason, and so is every output that needs it; an assertion that needs one is
  *notProven*, with that reason. *Needs* is what the formula uses as it is worked out: in
  `if(corner == 1, gap, cg)` with `corner` at 1, `cg` is not needed, and its being unknown changes nothing.
- `output.list` gives each output with `status` (*known*, *unknown*, or *pending* while it is measured),
  its `value` in the document's units, `unit` and `message`; so do `document.inspect` (under `outputs`)
  and a Jig's Parameters tab, below the parameters. Outputs are saved with the document and are the same
  every time it is built.

## Over a motion

A design that moves (its timeline `t`, see [The window](04-window.md#playing-a-motion)) is checked over the
whole motion with `motion.check`: every assertion is measured at evenly spaced moments of `t` (samples), on a
copy, and the result says per assertion whether it held at the samples, the first sample it failed at
(`firstFailure.t`, with `after`, the last sample before it that passed: the failure begins between the two),
and its least and greatest value at the samples with their `t`. It proves what held at the samples, no more:
a collision shorter than the spacing of the samples can be missed, so the result says `sampled: true`; take
more samples, or a narrower `t` around a suspect moment, to look closer. Any measure works: `overlap` with
`is` false gives the first sample that collides, `clearance` the least gap at the samples and where it is. `measure` checks an assertion written in place without keeping it.

```json
[
 ["variable.set", {"name": "t", "expr": "0"}],
 ["feature.block", {"name": "Wall", "size": ["10 mm", "10 mm", "10 mm"], "position": ["-10 mm", "0 mm", "0 mm"]}],
 ["feature.block", {"name": "Slider", "size": ["10 mm", "10 mm", "10 mm"],
   "position": ["15 mm + 10 mm * cos(360 deg * t)", "0 mm", "0 mm"]}],
 ["assert.add", {"name": "gap", "measure": "clearance", "from": "Wall", "to": "Slider", "min": "6 mm"}],
 ["motion.check", {"samples": 33}],
 ["motion.check", {"measure": {"measure": "overlap", "of": "Wall", "to": "Slider", "is": false}}]
]
```

The first check fails at `t` = 0.4375 with the least clearance 5 mm at `t` = 0.5 (the slider swings to within
5 mm of the wall); the second passes. A collision shorter than the spacing of the samples can be missed: give
more `samples` where it matters.

## Assertions that apply only sometimes

A Jig's choices change what there is to check. With the corners of a sheet-metal box open, there is no
closed corner, so `cornerGap` has no value: the least gap over no corners is no number, and the measure
says so as an *error* rather than pass. Give such an assertion a condition, `when`, as a feature has
one: while it is 0 the assertion is **inactive**. It is not measured, and it counts as neither passed nor
failed nor unproven, in `assert.list` (`inactive` counts them; `ok` is true when every other one passed),
in `document.check` and in `craft3d-cli check-range`.

```json
[
 ["variable.set", {"name": "corner", "expr": "1"}],
 ["variable.set", {"name": "gap", "expr": "0.5 mm"}],
 ["sketch.create", {"plane": "xy", "name": "BoxBase"}],
 ["sketch.rect", {"sketch": "BoxBase", "from": [0, 0], "to": [120, 80]}],
 ["feature.sheetMetal", {"sketch": "BoxBase", "name": "Box", "thickness": "1.5 mm", "radius": "2 mm", "kFactor": 0.42,
   "relief": {"shape": "round", "size": "4 mm"}, "corner": {"kind": "corner", "gap": "gap"},
   "flanges": [{"edge": "e5", "length": "20 mm", "angle": "90 deg"}, {"edge": "e6", "length": "20 mm", "angle": "90 deg"},
               {"edge": "e7", "length": "20 mm", "angle": "90 deg"}, {"edge": "e8", "length": "20 mm", "angle": "90 deg"}]}],
 ["assert.add", {"name": "gap", "measure": "cornerGap", "of": "Box", "eq": "gap", "when": "corner != 1"}],
 ["variable.measure", {"name": "cg", "measure": "cornerGap", "of": "Box"}],
 ["assert.add", {"name": "gap or open", "measure": "value", "expr": "if(corner == 1, gap, cg)", "eq": "gap"}],
 ["assert.list", {}]
]
```

`corner` picks the corners' kind (1 open, 2 butt, 3 overlap, 4 mitre). At 1, *gap* is inactive (*inactive:
its condition `corner != 1` is 0*), the output `cg` is unknown, and *gap or open* passes all the same: the
branch of `if` that is taken does not need `cg`. Set `corner` to 2 and the corners close with a 0.5 mm gap:
*gap* is measured and passes, and so does *gap or open*, now on `cg`.

- `when` is an expression of the variables and outputs, true when it is a number other than 0
  (`corner != 1`, `lid`, `holes && size > 20 mm`). A condition with a unit, or one that does not work
  out, is the assertion's *error*. `assert.edit` changes it, and `"when": null` takes it away.
- `holeToBend` is the same: a part with no hole on a flat beside a bend has no least distance, and the
  error says what the part has. A part's holes are those it was built with (its sketch's inner loops and
  its `cutouts`), not a hole a later feature cuts into its body; an assertion on a hole that only some of a Jig's
  choices make takes those choices' condition.

**Next:** [Selection and named sets](11-selection.md).
