# Measuring and assertions

*Ask the design questions (how heavy, how thick, does it fit) and keep the answers that matter as
checks that run at every change.*

An agent should check a design by asking it questions, not by looking at a picture. 3DCraft answers on
the exact geometry. It can also keep the questions in the document as **assertions**, which the window
calls **checks**: they are measured again after every change.

Questions of engineering (a section's stiffness, a fill level, where a hull floats, a stress, a clash in
a motion) have a chapter of their own: [Engineering checks](engineering-checks.md).

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
| `measure.curvature {"of", "at"}` | The curvature of the surface at the point nearest to `at`: Gaussian, mean and the principal curvatures, exact |
| `measure.draft {"faces", "direction", "minDraft"}` | The least draft angle of faces against a pull direction, and the share of their area below `minDraft` ([surface quality](#surface-quality)) |
| `measure.below`, `measure.section`, `measure.levelFor`, `measure.floating` | The volume below a plane, a section's area and second moments, the level for a volume, the level and tilt a body floats at ([Engineering checks](engineering-checks.md#below-a-plane-sections-and-levels)) |

A subject (`of`, `from`, `to`) is a body by its feature's name, a list of them, or a query as
`select.query` takes it ([Selection and named sets](selection.md)): `{"hole": true, "radius": "4 mm"}` for
the 8 mm holes, `{"kind": "face", "where": {"normal": {"along": "z"}}}` for the faces facing up. A named
set, `{"set": "Blocks"}`, is what the set holds: a set of bodies gives `area` the area of those bodies.

A body switched off by its condition (`when`) or suppressed is left out of a subject that names it, so one
assertion holds for every option of a Jig. A `measure.*` command's answer lists such bodies under
`switchedOffLeftOut`. When every body a subject names is off, there is nothing to check, and the assertion
is *inactive*, saying so. In an assembly, a subject names instances
([Assemblies › Packing, export and checks](assemblies.md#packing-export-and-checks)).

`distance` is measured between surfaces: a body (or a face) wholly inside another is as far from it as from its
nearest face, not 0. To know whether bodies touch or overlap, use `clearance`.

What is searched for (a thickness, a distance) comes with what is **proven**: `atLeast`, a bound nothing
is below, beside the value found at a real place. On the flange of
[Working with an AI agent](agent.md#a-full-session), `measure.thickness` answers `"thickness": 6.0,
"atLeast": 5.99999999999968`: the thinnest wall found is 6 mm, and no wall is thinner than 5.99999999999968
mm anywhere.

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
  `sectionArea`, `sectionCentroid`, `sectionInertia`, `sectionTorsion`, `sectionShearCentre`, `sectionExtent`, `levelFor`, `floatingOffset`, `floatingHeel`,
  `floatingTrim`, `floatingGM`, `floatingGML`, `floatingStable` ([Engineering checks](engineering-checks.md#below-a-plane-sections-and-levels)), the
  sheet-metal measures and `bendable` (the press-brake check, [Sheet metal](sheet-metal.md#checking-on-a-press-brake)),
  `value`, a formula of the variables and outputs (`expr`, [below](#outputs-measures-in-formulas)), and
  `continuity` (`edge`, `face`: the order a seam reaches as a number, −1 none, 0 G0, 1 G1, 2 G2; `"min": 2`
  asserts G2, [below](#continuity-across-a-seam)).
- Bounds: `eq` (with `tol`, by default a millionth of the value; `0` is allowed), or `min` and/or
  `max`; `is` for the yes-or-no measures (`true` by default, except for `overlap`, which wants it
  said). Bounds are expressions in the document's variables, so they follow the design: the case's
  volume is right for any `wall`.
- A bound on a length is met within the document's tolerance (an area within its square, a volume
  within its cube, an angle within the angular tolerance): what the geometry cannot tell apart does not
  fail it. A `min`, `max` or `tol`
  finer than that (other than 0) passes the same way, with a warning on the assertion that says what
  really passes: at a tolerance of 0.01 mm, *the bound 0.000015 mm is finer than the document's
  tolerance of 0.01 mm: anything up to 0.010015 mm passes*.
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

Where they show:

- in the result of every feature command, `variable.set`, `param.set` and `document.check`, as
  `assertionSummary`: `{count, passed, broken, unproven, pending, inactive}`;
- in `assert.list`, each with what was measured, what was expected and its message;
- in `document.check`, where a broken one makes `ok` false, and in `document.inspect`;
- in the window, under the feature list, as **Checks**.

`assert.edit` and `assert.delete` change and remove them.

**Pending after an edit.** An edit's result waits up to two seconds for the assertions it touched; the
rest count under `pending`, and `whenPending` then says how to have them. A script that checks its
assertions after a change does one of these:

- it gives the edit `"wait": true` (`param.set`, `variable.set`), which waits until every assertion is
  measured, a minute at most (what takes longer stays `pending`, with `whenPending`);
- it runs `assert.list` next, which waits by default;
- it reads `document.inspect {"wait": true}`, which waits for them too.

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
  "x", "min": "-w / 2 - 6.5 mm"}`. Coordinates are in the model's frame.

  The box of a body with spline faces is found on a fine mesh: the
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
  12 mm^3, expected no overlap*. `is: true` asserts a press fit.

One trap: in an expression a power binds to the number with its unit, so `50 cm^3` would mean
(50 cm)³; it is refused wherever a bound is read. Write `50 * (1 cm)^3`, or a plain number in the
document's unit (mm³ in a mm document). Only `0` and `1` may be written so (`"tol": "0 mm^3"`): for them
the two readings are the same.

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

## Outputs: measures in formulas

An **output** is a variable 3DCraft works out *after* the model is built, from a measure, or from a
formula of other outputs. You read it and check it; you never set it. The weight of an aluminium plate:

```json
[
 ["variable.set", {"name": "density", "expr": "2.7 g / (1 cm)^3"}],
 ["feature.block", {"name": "Plate", "size": ["100 mm", "60 mm", "5 mm"]}],
 ["variable.measure", {"name": "vol", "measure": "volume", "of": "Plate"}],
 ["variable.output", {"name": "weight", "expr": "vol * density", "label": "Weight", "unit": "g"}],
 ["assert.add", {"name": "light enough", "measure": "value", "expr": "weight", "max": "100 g"}],
 ["output.list", {}]
]
```

`output.list` says *Weight 81 g* (30 000 mm³ at 2.7 g/cm³), and *light enough* passes. Make the plate steel,
`density` 7.85 g/cm³, and the weight is 235.5 g: *light enough* fails. Only the output and the check are
worked out again: the volume did not change.

[Engineering checks](engineering-checks.md) has outputs of another kind: a hull's stability worked out
from where it floats, and a bar's bending stress.

- `variable.measure` takes a measure and its parameters as `assert.add` does ([the measures](#assertions)).
  The output is the number with its unit: a length, an area, a volume, mm⁴; a mass in kg, as `2 kg` is; a
  count as a plain number; yes or no as 1 or 0. Its formulas (an `offset`, a `volume`) may use other
  outputs, so a hull's centre of buoyancy can be measured at the waterline another output found.
- `variable.output` is a formula of variables and outputs. `variable.measure` or `variable.output` on an
  existing output changes it; `variable.rename` and `variable.delete` take outputs as they take variables.
- **A label and a unit to show it in.** `label` is what the Parameters tab and `output.list` show
  instead of the name, `unit` the unit ([Engineering checks › Forces, stresses and
  moments](engineering-checks.md#forces-stresses-and-moments) says which units may be given). Both are only
  how it is shown, and both are kept when an output is changed without them (`null` takes one away).
- **Only outputs and assertions may use an output.** A variable, a feature, a sketch, a condition, a named
  set or a material that uses one is refused, and the message names it (*the variable `beam` uses `GM`, an
  output*): the model is built first, then measured, so a measure can never depend on itself. Give such a
  size a variable of its own.
- An output that would depend on itself is refused with the loop named: *`B` would depend on itself
  (B → A → B)*. A name in a branch of `if` that is not taken counts too: `A = if(on == 1, B, 0 mm)` with
  `B = if(on == 1, 2 mm, A)` is refused as a loop, although at `on` = 1 neither needs the other.
- `variable.set`, `param.set` and `variable.publish` refuse an output: it is read-only.
- **Unknown, never a guess.** When a measure fails (a volume the hull cannot hold), or what it proves is
  wider than the document's tolerance (a wall found at 2 mm but proven only above 1.96 mm), the output is
  *unknown* with the reason, and so is every output that needs it; an assertion that needs one is
  *notProven*, with that reason. *Needs* is what the formula uses as it is worked out: in
  `if(corner == 1, gap, cg)` with `corner` at 1, `cg` is not needed, and its being unknown changes nothing.
- `output.list` gives each output with `status` (*known*, *unknown*, or *pending* while it is measured),
  its `value` in the document's units, `unit` and `message`; so do `document.inspect` (under `outputs`)
  and a Jig's Parameters tab, below the parameters. Outputs are saved with the document and are the same
  every time it is built.

## Surface quality

The analyses of the window ([The window](window.md#surface-analysis)) have measures behind them.
`measure.curvature` with `at` gives the curvature of the face nearest to a point, from the exact
surface: on a torus of radii 10 and 3 the outer equator has a Gaussian curvature of 1/39 and the
inner one −1/21.

`measure.draft` gives the least draft of faces against a pull direction, in degrees: the angle
between the face and the pull, positive where the face leaves the mould along it. A plane's is exact.
On a cylinder, a sphere or a torus the least is found at the vertices of a fine mesh (where the
normals are the exact surface's) and proven to within what the normal can turn between them; on a
cone or a freeform face it is sampled, and says so (`proven: false`). `shareBelow` is the share of
the faces' area below `minDraft`.

As assertions, `draft` takes the same parameters (the least draft is `minDraft`, because `min` is
the assertion's bound) and `curvature` takes `at`:

```text
assert.add {"name": "releases", "measure": "draft", "faces": {"kind": "face", "where": {"name": "F2:*"}}, "direction": "z", "minDraft": 1, "min": "1 deg"}
assert.add {"name": "dome", "measure": "curvature", "at": [0, 0, 40], "max": 0.001}
```

## Continuity across a seam

`measure.continuity {"edge": "F6:startEdge(e0,e2)", "face": "F1:face(0)"}` says how a sheet meets a face
along an edge (here the fin of [Surfaces › Curvature-continuous seams](surfaces.md#curvature-continuous-seams-g2)) (without `face`, the edge's two faces). `order` is the highest of **G0** (in position), **G1**
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

The faces are compared at corresponding points, and the correspondence is local. A point of the first
face's edge corresponds to the nearest point of the second face's own edge, looked for only near it (in
the stretch of the edge being bounded and as far again into its neighbours) and only within the edge (never
past its ends). How far apart the two edges lie is the position gap; the second order counts how far along
the seam the corresponding point may have moved, never a point of the second face off its edge. Against a
plane, cylinder, cone or sphere the corresponding point is the face's foot point.

```json
[
 ["assert.add", {"name": "fin is G2", "measure": "continuity", "edge": "F6:startEdge(e0,e2)", "face": "F1:face(0)", "min": 2}]
]
```

**Next:** [Engineering checks](engineering-checks.md).
