# 9. Modelling features

*What each kind of feature does, with the parameters that matter: a reference to come back to.*

What each kind of feature does, with the parameters that matter. The full list of parameters of any
command is in `command_list` (or `craft3d-cli commands --filter <id>`), with a summary that also says
what each refuses. The [examples](../examples/README.md) show every feature below in a part that checks
its own volume.

## Primitives

`feature.block` (`size`, `position` of its low corner), `feature.cylinder` and `feature.cone` (`radius`,
`height`, `position` of the centre of the base, `axis`), `feature.sphere`, `feature.torus` (`major`,
`minor`). An `axis` (and the axis or normal of a transform, mirror, pattern or face move) is three
expressions, plain numbers or lengths as a position takes them, so a part can stay square to a wall
whose angle depends on parameters: `"axis": ["cos(a)", "sin(a)", 0]` or `["dx", "dy", 0]`. Only where it
points counts; one of no length fails the feature, and so does one that mixes lengths with plain
numbers other than 0 (`[1, "25 mm", 0]`: give all as lengths or all as numbers). Their faces are numbered in a fixed order: a block's 0 bottom, 1 top, 2 front, 3 back, 4 left,
5 right; a cylinder's 0 the wall, 1 the base, 2 the top.

## From sketches

- **Extrude** (`feature.extrude`): the closed loops of a sketch along its normal; a loop inside another is a
  hole. One end: `distance` (negative goes the other way; `symmetric` puts half on each side),
  `throughAll`, or `upTo` a face. `cut`, `join` or `intersect` combines it with a body in the same step.
- **Revolve** (`feature.revolve`): about the sketch's `x` or `y` axis, one of its lines, or a datum axis;
  `angle` defaults to a full turn. Example: [bushing](../examples/bushing.json).
- **Sweep** (`feature.sweep`): a profile carried along a `path`, a sketch of lines, arcs and splines end to
  end, square to the profile's plane at its start, or a helix (`datum.helix`). Examples:
  [pipe](../examples/pipe.json), [spring](../examples/spring.json).
- **Loft** (`feature.loft`): through two or more profiles on planes one after the other; smooth, or
  `ruled` (straight from each to the next). Example: [hopper](../examples/hopper.json).
- **Offset profile** (`feature.offsetProfile`): every loop of a sketch grown or shrunk by a distance;
  extrude it, or export it as a laser's path with its kerf.

## Booleans

`feature.boolean` with `op` `union`, `subtract` or `intersect` combines the body of `target` with the
body of `tool`; both go into the new feature (`keepTool` keeps the tool's body to use again). The kernel is
3DCraft's own and handles planes, cylinders, cones, spheres, tori and freeform faces; what it cannot do
yet it **refuses with the reason**, never with a wrong solid. Faces that touch or lie within the
tolerance of each other are the hardest cases; a cutting tool made a little longer than the part avoids
them and says what is meant (the examples use `thickness + 2 mm` from `-1 mm`).

## Patterns, mirror, transform

- `feature.pattern`: `count` copies of a body; linear with `step` (`[dx, dy, dz]`), circular with `angle`
  about `axis` through `point` (`"360 deg / n"` with `count: "n"`), a screw with both. A pattern of a
  pattern makes a grid. Example: [flange](../examples/flange.json).
- `sketch.pattern` repeats curves inside a sketch (`count`, `step`, `count2`, `step2`, or `angle` about a
  `center`): one extrusion then makes all the holes. Example: [panel](../examples/panel.json).
- `feature.mirror` in a plane (`normal` and `point`, or a datum `plane`); `feature.transform` moves or turns
  a body (`translate`, `angle`, `axis`, `pivot`). With `keep` the source keeps its body and you get a copy.

## Fillets and chamfers

`feature.fillet` (`radius`) and `feature.chamfer` (`distance`; or `distance` and `second`; or `distance`
and `angle`) take `edges` as pairs of faces: `[["F1:face(2)", "F1:face(4)"], …]`. `chain: true` adds the
edges that continue an edge smoothly. Straight edges between flat faces and circular edges round an axis
are exact; other edges get a spline face within the tolerance. A radius that does not fit fails with the
largest that does. `document.edges` lists every edge of a body with its two faces. Examples:
[bracket](../examples/bracket.json), [case](../examples/case.json), [flange](../examples/flange.json).

## Shell and face operations

- `feature.shell` makes a body hollow with walls of `thickness`; the faces in `open` are taken away. A
  rounded box gets inner rounds of the radius less the wall. Example: [case](../examples/case.json).
- `feature.offsetFace` moves faces along their normals (a boss taller, a bore wider); `feature.moveFace`
  moves or turns them; `feature.draft` tilts them about a `neutral` plane; `feature.deleteFace` takes faces
  out as if they had never been made (a hole filled, a fillet back to a sharp edge); `feature.thicken` makes a layer of `thickness` on faces.

## Threads

`feature.thread` puts an ISO metric thread on a cylindrical face: external on a shank, internal in a hole.
`face` is the cylinder, `start` the flat face at the end where it starts, `size` an ISO size such as `M10`
(or left out: the size of the face), `length`. It is **cosmetic** by default (the body is unchanged and
the face carries the thread, as other modellers do); `modeled: true` cuts the real groove, which takes
seconds. Example: [bushing](../examples/bushing.json).

## Datums

Construction geometry that makes no body: `datum.plane` (at a `distance` from a plane or a face, turned by
an `angle` `about` an axis, or `through` three points), `datum.axis`, `datum.point`, `datum.helix`. Sketch
on a datum plane, mirror in it, pattern about a datum axis. A datum plane turned `about` an axis goes
through that axis: to turn a plane about a line that is not a world axis, make that line a `datum.axis`
first, as the [wedge](../examples/wedge.json) does. `datum.references` lists what a feature can be placed
by.

## Sheets

`feature.fill` makes a **sheet**: flat faces with no volume, bounded by the loops of a sketch. A sheet is a
cutting tool: `feature.boolean` of a solid with a sheet that reaches across it keeps the part in front of
the sheet (`subtract`, the side its normal points to) or behind it (`intersect`). Example:
[wedge](../examples/wedge.json). An open chain in a loft gives a sheet too, and `feature.thicken` turns
faces into a solid layer. `feature.fold` folds a sheet along a crease pattern ([chapter 16](16-origami.md)).

## Developable surfaces and unfolding

A **developable** surface is one a flat sheet bends onto without stretching: plywood panels of a boat,
sheet metal, paper, a lampshade. `feature.developable` makes the developable sheet between two curves
(`a` and `b`): each a sketch whose curves make one open chain, or `{"edges": [...]}`, edges of earlier
bodies in order (the chines of a hull, for instance). Straight lines (*rulings*) join the two curves where
both curves' tangents and the line lie in one plane, which is what makes the sheet developable; where the
ruling from an end does not reach the other curve's end, a fan (a cone from that end) fills the corner.
The sheet's edges are the curves themselves, so two panels on one curve meet edge to edge. `feature.unfold`
lays faces flat as one piece, the pattern to cut, with every length, angle and area kept:

```json
[
 ["sketch.create", {"name": "Low"}],
 ["sketch.arc", {"sketch": "Low", "center": [0, 0], "start": [20, 0], "end": [0, 20]}],
 ["sketch.create", {"name": "High", "offset": "15 mm"}],
 ["sketch.arc", {"sketch": "High", "center": [0, 0], "start": [10, 0], "end": [0, 10]}],
 ["feature.developable", {"a": "Low", "b": "High", "name": "Panel"}],
 ["feature.unfold", {"faces": ["F3:face(0)"], "at": [0, "-40 mm", 0], "name": "Pattern"}],
 ["unfold.map", {"unfold": "Pattern", "points": [[10.6, 10.6, 7.5]], "to": "flat"}]
]
```

The two arcs make a piece of a cone; unfolded it is a piece of a flat ring. The unfold's result says how
far the pattern is from the part (`lengthChange`, `areaChange`, `mapError`), and `unfold.map` takes points
from the part to the pattern (`"to": "flat"`) or back (`"to": "part"`): a hole drawn on the pattern is found
on the panel, and the other way round.

- `measure.developability` tells how far faces are from developable: the largest Gaussian curvature found
  on each face (in 1/mm²; a sphere of radius r has 1/r² everywhere, planes, cylinders and cones 0) and
  whether the stretch a flat sheet would need over the face is within the document's tolerance. As
  assertions: `"measure": "curvature"` with `max`, and `"measure": "developable"` with `is`.
- A face that is not developable (a sphere, a torus, a double-curved loft) is not unfolded: the reason
  gives its curvature and where. Two faces whose shared edge bends differently on its two sides (two hull
  panels at a chine, a fold) do not lie flat as one piece: unfold each on its own.
- A developable is refused, with where, when its rulings would cross (an inflection of a curve, a curve
  that winds about the other), when the curves lie in one plane over a stretch only, when they meet
  between their ends, and for closed curves and curves with a corner (not supported yet).

## Several bodies

A document may hold any number of bodies; every feature that does not consume another body adds one.
`body.split` splits a body that fell into pieces (a cut through the middle) into one body per piece.
`body.appearance` and `face.appearance` give colours, `body.material` a density (then `document.inspect`
gives the mass), `document.interference` says whether bodies overlap. A body's colour channels and its
density may be expressions of the variables (`["if(dark, 60, 200)", 120, 40]`, `"sheets[sheet].density
g/cm3"`), so they follow a parameter.

## Hiding and looking back

`feature.visible` with `"visible": false` hides a feature's body, sketch or datum in the view (the
window, `view.render`, `view.pick`); `true` shows it again. It is not `feature.suppress`: the feature is
still built, measured and exported, and later features are still made from it. It is saved with the
document and undone like a colour; `document.inspect` marks it `"hidden": true`.

`view.scrub` shows the model as it was after one step (`{"step": 3}` or `{"feature": "Plate"}`;
`{"feature": null}` shows it as it is again), without changing the document: the timeline in the window
does the same. To build on from that step, move the real marker there with `history.rollback`.

## Switching a feature on or off

`feature.suppress {"feature": "Round"}` switches a feature off by hand; `"suppressed": false` switches it
on again. `feature.when` switches it by a condition, such as a yes-or-no parameter of a Jig. Off, a feature
behaves as if it were not in the history: one that changes a body passes it on unchanged (an inactive
transform leaves the body where it was), one that makes a body makes none, and its faces and edges are not
there for the fillets and face operations after it. [Chapter 8](08-formulas.md#switching-parts-on-and-off) explains both in full.

**Next:** [Measuring and checking](10-measure.md).
