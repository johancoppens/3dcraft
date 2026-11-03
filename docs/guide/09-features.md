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

## Text

`sketch.text` puts a text in a sketch: the outlines of its letters, closed profiles that extrude, cut and
emboss as any profile does. Text may hold formulas in braces, written again at every regeneration: a
length in the document's unit, an angle in its angle unit, a number as it is (`{{` and `}}` write a
brace). `size` is the height of the capitals (or, with `sizeOf: "em"`, the em). A raised serial number
on a plate:

```json
[
 ["variable.set", {"name": "partNo", "expr": "42"}],
 ["feature.block", {"size": [60, 20, 3], "name": "Plate"}],
 ["sketch.create", {"name": "Label", "face": "F1:face(1)"}],
 ["sketch.text", {"sketch": "Label", "text": "No. {partNo}", "size": "8 mm", "at": [30, 10], "align": "center", "anchor": "middle"}],
 ["feature.extrude", {"sketch": "Label", "distance": "1 mm", "join": "Plate", "name": "Raised"}]
]
```

- **Fonts.** `sans` (Noto Sans) and `mono` (JetBrains Mono) come with 3DCraft; `stroke` is the
  single-stroke font, for engraving, CNC, lasers and pen plotters: its letters are open strokes, exported
  with `file.export` as DXF or SVG of the sketch, not extruded. `font.add` adds any OpenType or
  TrueType file under a name (`font.list` lists them all). The document keeps the font, so it opens the
  same on another machine, unless the font forbids that (its embedding permission): then the document only
  names the font, and asks for it again when it is opened.
- **Placing it.** The text stands on its point `at` (a point of the sketch, so constraints can move it):
  `anchor` says which line sits there (the first baseline, the top of the capitals, the middle, the last
  baseline), `align` whether the lines go right of it, around it or left of it, and `rotation` turns it.
  Lines break at newlines (`\n` in JSON), and with a `width` also at spaces. `lineSpacing` scales the
  font's own line height.
- **Along a curve.** `sketch.textOnCurve` sets the letters upright along a line, arc, circle or spline of
  the sketch: each letter keeps its shape, its baseline along the curve; `start` is the length along the
  curve where it starts, `offset` lifts it off the curve; the letters stand on the left of the curve as it runs (inside a
  circle, which runs counter-clockwise), or with `side: "right"` on its right (outside a circle, still
  reading left to right); `reversed` runs the curve the other way.
- **Names.** Each outline of a letter is named after the text, the letter's place in the text and the
  letter: the side of the B of `"AB"` extruded is `F3:side(e2.glyph(1,U+0042).contour(0))`. Change the text
  and a face of a letter that is no longer at its place fails by its name; it is never handed to another
  letter. `sketch.textEdit` changes the text, font, size or anything else; `sketch.inspect` lists the
  glyphs with their contours.
- **Measuring.** The measure `textExtent` (`what`: `width`, `height`, `capHeight`, `lines`, `inkWidth`,
  `inkHeight`) holds a text to a size in an assertion, or makes it an output for formulas.

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
(`a` and `b`): each a sketch whose curves make one open chain (or one closed loop), or `{"edges": [...]}`,
edges of earlier bodies in order (the chines of a hull, for instance). Straight lines (*rulings*) join the
two curves where both curves' tangents and the line lie in one plane, which is what makes the sheet
developable; where the ruling from an end does not reach the other curve's end, a fan (a cone from that
end) fills the corner. Where a curve has a corner, a fan from the corner turns it, so the sheet stays smooth
(where both curves have a corner on one ruling, the sheet is creased along it). Two closed curves give a
tube, cut open along one ruling when it is unfolded.
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

- The feature's result says under `developable` how many rulings were found (`rulings`), whether `b` was
  run the other way (`reversedB`), how far the faces were found from the true developable (`deviation`,
  within half the document's tolerance), the largest curvature found (`curvature`) and `unresolved`: how
  far the stored faces' tangent planes at the rulings where faces meet were found from the planes they were
  built with, times the ruling's length. That is what the rounding of the numbers leaves unresolved, an
  estimate and not part of the deviation: a small part of the tolerance near the origin, up to some tens
  of tolerances for narrow faces billions of tolerances away from it (a 5 m boat at a tolerance of a
  millionth of a millimetre). Positions are not affected; model nearer the origin to bring it down.
- `measure.developability` tells how far faces are from developable: the largest Gaussian curvature found
  on each face (in 1/mm²; a sphere of radius r has 1/r² everywhere, planes, cylinders and cones 0) and
  whether the stretch a flat sheet would need over the face is within the document's tolerance. As
  assertions: `"measure": "curvature"` with `max`, and `"measure": "developable"` with `is`.
- A face that is not developable (a sphere, a torus, a double-curved loft) is not unfolded: the reason
  gives its curvature and where. Two faces whose shared edge bends differently on its two sides (two hull
  panels at a chine, a fold) do not lie flat as one piece: unfold each on its own.
- The faces of a developable are named by where they are, not by how many come before them: `face(0)`
  is the first run of rulings, `face(1)` and `face(2)` the fans at the start and the end. Where the rulings
  turn back, a fan closes the gap: the `n`-th such fan (from 0) in the first half of the sheet, counted
  from the start, is `face(10000000000 + n)`, in the second half, counted from the end,
  `face(20000000000 + n)`; a fan from a corner of `a` is `face(30000000000,c)` (of `b`,
  `face(40000000000,c)`), `c` the id of the sketch curve after the corner, or the place of the edge after
  it in the list from 0. A run of rulings after one of these is `face(100000000000 + m)` with `m` that
  fan's first number (a corner where the faces part counts as its fan's, its `c` following:
  `face(130000000000,c)`), one after a fan counted from the end is named by the next one,
  `face(200000000000 + m)`, and the run that reaches the end is `face(300000000000)`. So a fan that
  appears at one end leaves the names at the other as they were. The count has a limit: one more fold
  nearer the same end (or a fold that moves across the middle) renumbers the folds counted from that end,
  and a reference to `face(20000000000)` then names the new fold's fan, without an error. Check the faces
  a later feature takes after an edit that may add a fold near an end. A fold at the very middle (as
  symmetric curves give) counts from the start, also when rounding puts it a hair past the middle.
- A developable is refused, with where, when its rulings would cross (a curve that winds about the
  other), when the curves lie in one plane over a stretch only, when they meet between their ends, when a
  curve turns back on itself, and when one curve is closed and the other is not. Two closed curves that
  are the same are refused, and so are two closed curves in one plane that touch, cross or lie side by
  side. The fans at the ends and at the corners unfold, and so does each fan where the rulings turned
  back; the strip of rulings beside such a fan does not yet, laid flat with the rest or on its own (where
  the rulings turned it has a corner the layout does not handle yet), so unfolding the faces one by one
  leaves that strip out. Cut the curves short of the fold to unfold the rest in one piece.
## Surfaces from curve networks

For flowing shapes (a mouse, a kettle, a handle) draw the curves the surface must pass through and let
3DCraft lay a smooth sheet through them. `feature.surface.network` takes **u curves**, which follow each
other across **v curves** (2 to 64 of each): every u curve must cross every v curve, and the first and
last curve of each family are the sheet's edges, so the others end on them. A curve is a sketch whose
curves make one open chain, or `{"edges": [...]}`. With two curves each way it is a Coons patch, which
`feature.surface.coons` makes from four curves round the patch:

```json
[
 ["sketch.create", {"name": "Low"}],
 ["sketch.arc", {"sketch": "Low", "center": [0, 0], "start": [20, 0], "end": [0, 20]}],
 ["sketch.create", {"name": "High", "offset": "30 mm"}],
 ["sketch.arc", {"sketch": "High", "center": [0, 0], "start": [20, 0], "end": [0, 20]}],
 ["sketch.create", {"name": "Left", "plane": "zx"}],
 ["sketch.line", {"sketch": "Left", "from": [0, 20], "to": [30, 20]}],
 ["sketch.create", {"name": "Right", "plane": "yz"}],
 ["sketch.line", {"sketch": "Right", "from": [20, 0], "to": [20, 30]}],
 ["feature.surface.coons", {"curves": ["Low", "Right", "High", "Left"], "name": "Wall"}],
 ["feature.plate", {"faces": ["F5:side(e1,e2,e3,e4)"], "thickness": "2 mm", "side": "out", "name": "Tube"}]
]
```

Two arcs and two lines give exactly a quarter of a cylinder, and the plate makes it a quarter tube. The
sheet passes through every curve; the result says under `network` how far at most the sheet passes from
any of them (`mismatch`, a proven bound, never more than the tolerance). The curves may be drawn either way
round.

An edge can be held: `"edges": {"uFirst": {"kind": "tangent", "face": "F7:face(0)", "weight": 1}}`
makes the sheet meet that face with one tangent plane all along the edge (G1, no crease; `tangency` in the
result is the largest angle found between the two), the curves crossing the edge bent to leave it along
the face, their speed there times `weight` (larger is fuller). `"kind": "position"` only checks that the
edge lies on the face. The face may be a plane, cylinder, cone or sphere, or a spline face along any
curve on it, such as the edge of another network sheet or of a trimmed face: two sheets then join smoothly.
`feature.loft` with `guides` lays a sheet through open sections along guide curves the same way (its
sections take no `start`).

Refused, with the curves named: curves that do not cross (and how close they come), curves crossing out of
order, a curve running past an edge, a curve leaving a tangent edge square to its face or along it, curves
leaving it to opposite sides (the sheet would fold over the edge), two held edges whose corner asks two
tangent planes or two twists, and a network too large for one sheet (more than 2^20 control points: many
curves, each crossed at its own parameters, each bring their own knots). The face is named
after its four edge curves (`F5:side(e1,e2,e3,e4)`: the ids of their sketches), so editing a curve or
adding one inside the network keeps its name.

### Curvature-continuous seams (G2)

A tangent seam (G1) has no crease, but the curvature still jumps across it: reflections bend there. For
class-A work hold the edge in **curvature** instead: `"kind": "curvature"` makes the sheet meet the face
with one tangent plane *and* the same normal curvature across the edge (G2), so reflections run on
smoothly. The curves crossing the edge are bent to take the face's curvature in their direction, and the
sheet's second derivative across the edge is put exactly where the face asks it, all along:

```json
[
 ["feature.cylinder", {"radius": "20 mm", "height": "30 mm", "name": "Post"}],
 ["sketch.create", {"name": "Seam", "plane": "zx"}],
 ["sketch.line", {"sketch": "Seam", "from": [0, 20], "to": [30, 20]}],
 ["sketch.create", {"name": "Far", "plane": "yz", "offset": "35 mm"}],
 ["sketch.line", {"sketch": "Far", "from": [25, 0], "to": [25, 30]}],
 ["sketch.create", {"name": "Bottom"}],
 ["sketch.spline", {"sketch": "Bottom", "through": [[20, 0], [23, 6.5], [27, 13], [35, 25]]}],
 ["sketch.create", {"name": "Top", "offset": "30 mm"}],
 ["sketch.spline", {"sketch": "Top", "through": [[20, 0], [23, 6.5], [27, 13], [35, 25]]}],
 ["feature.surface.network", {"u": ["Seam", "Far"], "v": ["Bottom", "Top"], "edges": {"uFirst": {"kind": "curvature", "face": "F1:face(0)"}}, "name": "Fin"}],
 ["measure.continuity", {"edge": "F6:startEdge(e0,e2)", "face": "F1:face(0)"}]
]
```

The result says under `network` › `continuity` how the seam was proven to meet the face, and
`measure.continuity` measures any such seam (below). The face may be a plane, cylinder, cone or sphere, or a
spline face along any curve on it: another network sheet's edge, or a trimmed face's edge (a flank cut from a
larger spline surface by `feature.surface.trim`), where the face is written across the curve exactly. Along a
spline face the curves crossing the edge take their crossing parameters from the face's line, so that both
sides of the seam share one parametrisation; where both edges of a family are held in curvature to spline
faces, the crossing curves must cross both lines at the same parameters. The sheet is refused unless the
measure proves G2, with the gaps it found; a torus is refused as for tangency.

**A smoother face to hold to.** A network interpolates each family across the other with cubics by
default, so where a sheet is held in curvature to a network sheet made of cubic curves, the face's curvature
across the seam has a kink at each of its knots, and the held sheet has to take that kink somewhere: it puts
it on a narrow band beside the seam (a crease there, within the tolerances). Give the face's network
`"acrossDegree": 5` and draw its curves with `"degree": 5` (`sketch.spline`), and the face is smooth enough
that the held sheet needs no band. The degree used is the one asked, but never more than the family's number of
curves less one (with five curves: 3 by default, 4 when 5 is asked); `feature.edit` with `"acrossDegree": null` goes back to the default, 3.

### A class-A mouse

Sheets whose edges meet are joined by `feature.boolean` with `"op": "sew"`: their edges are sewn where
they run along one another, nothing is cut, and the result is a solid once every edge has its partner.
A union intersects the sheets first, and two sheets in G2 contact along their seam are too nearly one
surface beside it to intersect. Build each sheet from the edges of the ones before it (`{"edges": [...]}`
as a network's curve) and the edges meet exactly. The start screen's mouse
([`mouse.json`](../examples/mouse.json), under Showcase) is built that way:

1. *The flanks.* A flat side whose top edge is the seam, a live spline through points written in the
   variables (`length`, `width`, `height`, `hump`, `taper`), filled (`feature.fill`) and turned in
   toward the nose by `taper` (`feature.transform`). The thumb's flank is drawn lower than the pinky's,
   so the shoulder over it is fuller, and mirrored so that it faces out.
2. *The top.* One Gordon surface (`feature.surface.network`): the two seams and the crown as u curves,
   the rear, four sections and the nose as v curves; both seams held in curvature (`"kind":
   "curvature"`) to the flanks. Each section is two superellipses meeting on the crown, with a smooth
   skew toward the thumb. The crown and the seams have ten points each and are crossed at the same ones,
   so no curve is re-parametrised and the surface stays cubic. The rear and the nose leave each flank
   along it with three control points in line, so they have no curvature across it, as the G2 seam asks,
   and the hold leaves them as drawn. The fourth section, just behind the nose's corners, keeps the nose
   fair: without it the Gaussian curvature map showed a thin saddle across the top there.
3. *The underside.* The flanks' feet and the same rear and nose curves in one sketch, filled, and
   mirrored in its own plane so that it faces down, out of the solid (sewing takes sheets that face
   one way).
4. *Sewn* into a solid (three sews), rounded along its foot (`feature.fillet` with `chain`), a slim slot
   cut down as a stadium (`feature.extrude` with `cut`), a hair-thin split line from the slot to the
   nose, and a slim wheel.

Its assertions hold both seams G2 (`measure.continuity` proves it at the default tolerances), a closed
solid of a mouse's volume and a wheel that turns free. Published (`variable.publish` with ranges),
`craft3d-cli check-range` proves the seams G2 at every corner of length 105 to 130 mm, width 56 to 70,
height 32 to 40, hump 0.36 to 0.44 and taper 2° to 5°. With zebra stripes (`ui.set {"analysis":
{"mode": "zebra"}}`) the stripes on the top run on into the flat flanks without a kink or a jump in width;
with the curvature map (`"mode": "curvature"`) the curvature rises from zero at each seam.

## Trimmed faces and N-sided fills

`feature.surface.trim` makes a face on the surface of a face you already have, the surface itself, bounded by
curves on it: an opening, a flank, a label area. `outer` is a loop of curves end to end (a sketch's closed
loop or chains, or edges), `holes` more loops inside it. Curves drawn in space are taken onto the surface,
to their closest points or, with `along`, along a direction (a sketch on a side plane projected onto a
curved body); with `"space": "parameter"` a sketch is read as the surface's own parameters, its x and y in
percent of the surface's domain. Every edge lies on the surface exactly, on spline surfaces, planes,
cylinders, cones, spheres and tori. A window
projected onto the quarter cylinder made above:

```json
[
 ["sketch.create", {"plane": "yz", "offset": "40 mm", "name": "Window"}],
 ["sketch.circle", {"sketch": "Window", "center": [9, 15], "radius": "6 mm"}],
 ["feature.surface.trim", {"face": "F5:side(e1,e2,e3,e4)", "outer": "Window", "along": [-1, 0, 0], "name": "Patch"}]
]
```

`feature.surface.nsided` fills a closed loop of 3 to 6 curves with a smooth sheet: one four-sided patch per
curve around a centre point, so no corner is pinched. `edges` holds each curve as a network holds its
edges: `tangent` to a face makes the fill meet it with one tangent plane, `curvature` with its curvature
too. `"inner": "curvature"` makes the patches meet each other in curvature as well, so zebra stripes run
on without a kink. The result says how every seam and curve was proven (`fill.seams`, `fill.edges`, as
`measure.continuity` measures); where one asked in curvature cannot be proven, the fill is built tangent
and says why.

## Plates and joints

`feature.plate` gives faces a thickness: a sheet (a fill, a loft of open sections, a developable) becomes a
**plate**, a solid of its own beside the sheet. `side` says where the material goes along the faces'
normal: `in` behind them (the faces are the plate's outside, as when a hull is drawn as its outside), `out`
in front, `mid` half each way. Along each edge of its outline a plate ends square unless `joints` says
otherwise: `bevel` (turned by `angle`), or, with the neighbouring plate's sheet (`with`, a face of it whose
edge the plate's edge runs along), `mitre` (both end on one surface, the bisector for equal thicknesses),
`touch` (square ends whose inner edges just touch, as stitch-and-glue panels), and `over` with `under`
(one plate covers the other's edge: a deck resting on the side panel), and `onto` (the plate ends on a
surface given as `onto`: `xy`, a datum plane or any face by its name, a stem too; two panels whose ends
lie on one surface meet there in one corner, as at the bow of a hull). Give both plates the matching
joint and they meet face to face, with no gap and no overlap. A floor and a wall in an L, mitred:

```json
[
 ["variable.set", {"name": "t", "expr": "4 mm"}],
 ["sketch.create", {"plane": "xy", "name": "FloorSketch"}],
 ["sketch.rect", {"sketch": "FloorSketch", "from": [0, 0], "to": [300, 150]}],
 ["feature.fill", {"sketch": "FloorSketch", "name": "FloorSheet"}],
 ["sketch.create", {"plane": "yz", "name": "WallSketch"}],
 ["sketch.rect", {"sketch": "WallSketch", "from": [0, 0], "to": [150, 200]}],
 ["feature.fill", {"sketch": "WallSketch", "name": "WallSheet"}],
 ["feature.plate", {"faces": ["FloorSheet:startCap()"], "thickness": "t", "side": "out", "name": "Floor",
   "joints": [{"edge": "FloorSheet:startEdge(e8)", "kind": "mitre", "with": "WallSheet:startCap()"}]}],
 ["feature.plate", {"faces": ["WallSheet:startCap()"], "thickness": "t", "side": "out", "name": "Wall",
   "joints": [{"edge": "WallSheet:startEdge(e5)", "kind": "mitre", "with": "FloorSheet:startCap()"}]}],
 ["document.interference", {"pairs": [["Floor", "Wall"]]}]
]
```

`document.edges` lists the edges of a sheet to give a joint (here the floor's edge along x = 0 and the
wall's along z = 0); `document.interference` says the two plates touch on the mitre face and do not
overlap. The plate's faces are named `F9:copy(0)/…` (the front skin, the side the normal points to),
`F9:copy(1)/…` (the back) and `F9:rim(…)/…` (the end along an edge of the sheet). To cut a plate from
sheet stock, `feature.unfold` lays one of its skins flat; the outline follows the joints. Refused, with the
edge: faces that meet at a crease (plate them apart and join them), plates in line with each other (no
corner to join at), an edge that does not run along the neighbour's outline, a plate so thick that its
ends run into each other.

## Sheet metal

`feature.sheetMetal` makes a part bent from one sheet. The sketch's closed profile is the **base** (holes
allowed); each **flange** is bent from a straight edge of it (`edge`, the line's entity id) or from the far
edge of an earlier flange (`tip`, its place in the list: a Z, a U on a U), by `angle`, `up` (towards the
side the sketch's normal points to) or `down`. Every bend has an inner `radius` and a `kFactor`, the
neutral fibre's place across the thickness (0 the inside, 1 the outside), where the sheet keeps its length:
the flat pattern grows by the bend allowance, angle × (radius + kFactor × thickness), per bend. A flange's
`length` is its flat beyond the bend, or with `"from": "mould"` measured from the virtual sharp of the
outside surfaces. `relief` cuts the base's corners where two flanged edges meet (`rect`, `round` or
`tear`, reaching `size` along each edge). A tray with four sides, its corners relieved:

```json
[
 ["variable.set", {"name": "t", "expr": "1.5 mm"}],
 ["sketch.create", {"plane": "xy", "name": "TrayBase"}],
 ["sketch.rect", {"sketch": "TrayBase", "from": [0, 0], "to": [120, 80]}],
 ["sketch.circle", {"sketch": "TrayBase", "center": [60, 12], "radius": 3}],
 ["feature.sheetMetal", {"sketch": "TrayBase", "name": "Tray", "thickness": "t", "radius": "2 mm",
   "kFactor": 0.42, "relief": {"shape": "round", "size": "4 mm"},
   "flanges": [{"edge": "e5", "length": "20 mm", "angle": "90 deg"},
               {"edge": "e6", "length": "20 mm", "angle": "90 deg"},
               {"edge": "e7", "length": "20 mm", "angle": "90 deg"},
               {"edge": "e8", "length": "20 mm", "angle": "90 deg"}]}],
 ["sheetMetal.bends", {"feature": "Tray"}],
 ["table.set", {"name": "steel", "columns": ["grade", {"name": "minRadius", "unit": "mm"}],
   "rows": [["DC01", 1.5], ["S355", 2.25]]}],
 ["assert.add", {"name": "Radius", "measure": "bendRadius", "of": "Tray", "min": "steel[\"DC01\"].minRadius"}],
 ["assert.add", {"name": "HoleClear", "measure": "holeToBend", "of": "Tray", "min": "2 * t"}],
 ["assert.add", {"name": "Relieved", "measure": "relief", "of": "Tray", "is": true}],
 ["file.export", {"path": "tray.dxf", "sheetMetal": "Tray"}]
]
```

A **jog** (`jogs`) steps a flat: draw a construction line across the base (or across a flange's flat,
where it lies beside its edge as if its bend had no width) and give the step's `offset`, the bends'
`angle` and which way the first turns (`direction`). The flat is cut along the line; the side on the
line's left (on a flange, the side its own bend is on) stays, and the other side becomes a flat of its
own, lifted `offset` by two opposite bends with a web between them, keeping its holes, its flanges and
its edges' names. The press-brake check takes a jog as made by an offset die (`formed`).

```json
[
 ["sketch.create", {"plane": "xy", "name": "StepBase"}],
 ["sketch.rect", {"sketch": "StepBase", "from": [0, 0], "to": [100, 40]}],
 ["sketch.line", {"sketch": "StepBase", "from": [50, -5], "to": [50, 45], "construction": true}],
 ["feature.sheetMetal", {"sketch": "StepBase", "name": "Step", "thickness": "1 mm", "radius": "1 mm",
   "kFactor": 0.4, "flanges": [{"edge": "e6", "length": "12 mm", "angle": "90 deg"}],
   "jogs": [{"line": "e11", "offset": "8 mm", "angle": "60 deg"}]}],
 ["sheetMetal.bends", {"feature": "Step"}]
]
```

A **formed louvre** (`formedLouvres`) is a vent pressed by a tool: draw its lance as a construction
line where it lies in the flat pattern and give the hood's `width` (how far back from the line it
reaches, on the line's left), `height` (the opening's), `ends` (`closed`, rounded, the default, or
`open`) and `direction`. The hood is not in the flat pattern, which shows the lance as a cut and where
the tool lands on the layer `form`; the part has the hood. `louvreStretch` measures how far the tool
stretches the sheet (the hood's length across over its width, less 1), for your material's limit.

```json
[
 ["sketch.create", {"plane": "xy", "name": "VentBase"}],
 ["sketch.rect", {"sketch": "VentBase", "from": [0, 0], "to": [100, 60]}],
 ["sketch.line", {"sketch": "VentBase", "from": [30, 20], "to": [70, 20], "construction": true}],
 ["feature.sheetMetal", {"sketch": "VentBase", "name": "Vented", "thickness": "1 mm", "radius": "1 mm",
   "kFactor": 0.4, "formedLouvres": [{"line": "e11", "width": "8 mm", "height": "3 mm"}]}],
 ["assert.add", {"name": "Louvre stretch", "measure": "louvreStretch", "of": "Vented", "max": "0.25"}]
]
```

The bends are folded by the same mechanism as `feature.fold` (a joint with a radius), and the folded sheet
is given its thickness as `feature.plate` does, so a bend's skins are the cylinders of the inner radius and
of the inner radius plus the thickness. `sheetMetal.bends` is the **bend table**: per bend its angle and
inner radius measured on the part, the K-factor, the bend allowance (in closed form, and as `developed`:
the bend's neutral band laid flat by the unfold), the outside setback and the bend deduction, the
flange's length; and the flat pattern's area and size. The **manufacturability** measures for assertions
are `bendRadius` (the smallest inner radius), `flangeLength` (the shortest flange; `"measured": "mould"`
from the virtual sharp), `holeToBend` (the least distance from a hole to a bend's tangent line) and
`relief` (every corner where two bends meet, and every end of a bend inside its edge, is relieved) `cornerGap` (the least gap at the closed corners), and `flatExtent` (`axis` x or y: the width or height of the flat pattern, the blank it needs) and `flatArea`. 3DCraft has no material catalogue: the limits
for a material and a thickness are yours, as expressions or a table, as above. `file.export` with
`sheetMetal` writes the flat pattern (.dxf or .svg): the outline on the layer `outline`, the bend lines
on `bendUp` (dashed) and `bendDown` (dash-dot). `sheetMetal.bendSequence` checks whether the part can be
bent on a press brake with your punch and die (each an outline in the plane square to the bend: the punch's
tip centre and the die's top at the origin, `y` up into the punch; a `{"through": [x, y]}` between two
points makes that side the arc through the point, so a punch's round tip of radius `r` is
`[[-r, 50], [-r, 0], {"through": [0, -r]}, [r, 0], [r, 50]]`, exactly) and finds an order of the bends: every
step is proven to keep the punch at least `clearance` from every piece of the part but the bend and its
two flats, and the die from every piece but the bend, over the whole stroke (the sheet may rest on the
die's top, not go into it), with the part as it lies or turned end for end (`turned` in the step, for a
gooseneck whose throat faces the other way), and a step that does not is listed by bend, by the bends made before it, by
tool and by piece; `unchecked` says what is not looked at (`certain` per step and `infeasibleProven` say
whether no order is proven or only not found). As an assertion, `bendable` keeps that check with the part:
`is` true holds when an order is found for every part named, fails when none exists (every step tried
collides for certain), and is *notProven* otherwise, never passed: a step that is neither proven clear nor
colliding, or a search that used up its `maxWork` (distances measured, 20000 at most, shared by the parts).
For one part, `order` checks a sequence you give instead of searching (its `maxWork` may then be larger),
and `formed` lists flanges made before the press brake by a tool of their own (a louvre on a punch press,
a jog by an offset die): they are made from the start and not checked themselves. `hem` presses
flanges bent 180 degrees flat in two strokes, an air bend to `preBend` degrees with the punch and die,
then a flatten stroke between a flat die under the part and a `pad` (an outline as the punch's, its body
above its `y = 0`) that stands on the hem: `"hem": {"flanges": [3], "preBend": 150, "pad": [[-20, 0], [20, 0], [20, 30], [-20, 30]]}`;
the steps list both (`stage` `pre-bend` and `flatten`). A part with a hole
across a bend is checked too: a strip not yet bent that a hole may cross (by the boxes of the hole and
the strip) stands as its whole slab, so a clearance to it is proven, while a collision with it is never
counted as certain (the tool may pass through the hole): such a part is then *notProven*, never `false`.
With an `order`, the measure may measure up to a million distances (`maxWork`; about 2500 a second, so
some minutes), the command `sheetMetal.bendSequence` ten million.
The tools' points may be formulas, so a Jig keeps its tools in a table and picks one with a choice:

```json
[
 ["table.set", {"name": "punches", "columns": ["tool", {"name": "half", "unit": "mm"}, {"name": "h", "unit": "mm"}],
                "rows": [["straight", 5, 100], ["wide", 30, 100]]}],
 ["variable.set", {"name": "tool", "expr": "1"}],
 ["assert.add", {"name": "Bendable", "measure": "bendable", "of": "Box",
   "punch": [["-punches[tool].half", 0], ["punches[tool].half", 0], ["punches[tool].half", "punches[tool].h"], ["-punches[tool].half", "punches[tool].h"]],
   "die": [[-15, -20], [15, -20], [15, 0], [-15, 0]], "length": "60 mm", "maxWork": 10000, "is": true}]
]
```

Each check of it searches the orders again (only when the part, the tools or the bounds change): on a
tray of four flanges some 2600 distances when an order is found and 7400 when none is, a second or two.
Bends on one line of one flat with the same
angle, direction and radius (two tabs on one edge) are made in one stroke; the tools are then a segment
per bend. By default the tools are one piece reaching past the whole part; `segmented: true` makes each
bend's tools as long as the bend (segmented tooling set up per bend), and `length` gives one length for all,
or a list of one per flange (`null` for the bend's own, segmented). `order` checks the order you give
(each step a flange, or a list of flanges bent in one stroke; with one-piece tools the flanges of one line
always together) instead of searching for one, and `maxWork`
raises the search's cap of 20000 distances. `sheetMetal.edit` changes a part (flanges as a
whole list). Each flange has an `id` (`sheetMetal.bends` lists them, 1, 2, … in the order the part was
made, a new flange's never one used before): every name of the flange's faces and edges is numbered by it
(its bend `face(2 · id − 1)`, its flat `face(2 · id)`, its rims and edges), not by its place in the list,
so a fillet, a colour or a set on it stays with it whatever else you add, delete or move. Give the id back
with the flange in `sheetMetal.edit`; a flange sent back unchanged keeps its id without it, and one changed
without it is a new flange (its old names then name nothing, an error where used). `tip`, `side` and `corners` name
flanges by their places in the list you send; corners you do not send again are kept and follow their
flanges by id to their new places (one whose flange you deleted is refused, naming it). A `radius` of 0 is a **sharp bend**: a sharp inside corner (the inner skin is the line on the bend's axis) and an outside rounded to the thickness, with the bend allowance angle × kFactor × thickness, so the flat pattern and the volume follow the same rules as any bend; its faces are those of the bend with a radius less its inner skin, so a radius set to 0 and back keeps every other name. A sharp bend is refused at or near 180 degrees (a closed hem), at a kFactor of 0, where another wall would touch its inner skin (two sharp flanges meeting at an open corner without a relief, a wall leaning in beside a sharp square wall, a flange on a side of a sharp wall), trimmed by a closed corner, and in a part with a hole across a bend; a cut made later through a sharp bend is not carried onto the exported flat pattern (the export lists it under notCarried). A formed louvre whose hood comes within about the thickness of a sharp bend's inside is refused too: its hood is checked by a box round it, which errs on the safe side. Refused, with the flange: a radius below zero, a K-factor
outside 0 to 1, an angle outside 0 to 180 degrees, an edge that is not a straight line of the base's
outline (a flange on a curved edge is a formed flange, not a bend: it has no flat pattern that keeps
lengths), flanged edges meeting at an inside corner, a relief that does not fit, and flanges that run into
each other or into the base when bent. With a K-factor of ½ the part's volume is the flat area times the
thickness; with another K-factor the part (whose bends keep the skins at the two radii) differs from that
by angle × width × thickness² × (½ − K) per bend.

**Holes in flanges.** The base's sketch is drawn in the plane of the flat pattern, so its other closed
profiles (beside the base) are holes and cut-outs of the flange whose flat each lies in: a flange's flat
starts the bend allowance beyond its edge. `cutouts` adds the closed profiles of other sketches in the same
plane (any origin and axes) the same way, and each sketch switches with its own `when`, so a vent or a
cable hole can be an option. The holes are cut in the flat pattern and in the folded part, and
`holeToBend` measures each from the bends beside its own flat. A hole drawn across a bend's strip (a slot
through the bend) is cut through the bend where the bend allowance puts it; one that only touches a
bend's tangent line, crosses it with a spline, or runs past the pattern's edge is refused, and so is the
part when a `cutouts` sketch fails (fix the sketch, or switch it off). A hole cut later by another feature
(a boolean) is carried back onto the exported flat pattern when it goes through the sheet square to it
(`laterCuts` in the export's result); one cut at a slant fails the export with the reason, and a pocket, a
part joined on or a notch at the edge is listed in `notCarried` (the bend table and the export say which
feature took the part as `changedBy`). A flange can also be bent out of a cut-out: `cutout` names the
sketch and a straight edge of a cut-out in it (`{"cutout": {"sketch": "Vents", "edge": "e7"}, ...}`), the
flange grows into the hole, and the cut-out may lie in a wall, so a louvre or a tab is bent from a slot in
a flange. The mini-PC chassis example (`docs/examples/chassis.json`) has eight. A panel with a hole in
its flange and an optional vent:

```json
[
 ["variable.set", {"name": "vent", "expr": "1"}],
 ["sketch.create", {"plane": "xy", "name": "PanelBase"}],
 ["sketch.rect", {"sketch": "PanelBase", "from": [0, 0], "to": [100, 60]}],
 ["sketch.circle", {"sketch": "PanelBase", "center": [109, 30], "radius": 3}],
 ["sketch.create", {"plane": "xy", "name": "Vent"}],
 ["sketch.circle", {"sketch": "Vent", "center": [50, 30], "radius": 8}],
 ["feature.when", {"feature": "Vent", "expr": "vent"}],
 ["feature.sheetMetal", {"sketch": "PanelBase", "name": "Panel", "thickness": "1.5 mm", "radius": "2 mm",
   "kFactor": 0.42, "cutouts": ["Vent"],
   "flanges": [{"edge": "e6", "length": "20 mm", "angle": "90 deg"}]}],
 ["assert.add", {"name": "HoleClear", "measure": "holeToBend", "of": "Panel", "min": "1.5 mm"}],
 ["file.export", {"path": "panel.svg", "sheetMetal": "Panel"}]
]
```

**Closed corners.** Where two flanges meet at a corner of the base, the corner is open: each flange ends
where its bend ends. `corner` closes it: the flanges' flats run on sideways past their bends (the bends
stay apart; the relief, if you give one, is the small hole where they would cross) up to the other
flange, `gap` apart. `overlap`: one flange covers the corner, its end flush with the other's outside
face, and the other stops `gap` short of its inside face. `butt`: the same, with the covering flange
also stopping `gap` short of flush. `miter`: both run to the plane that halves the angle between the
walls, `gap` apart there (their ends bevelled; `mitre`, as plates spell it, is accepted too). At each corner the flange on the edge that arrives at it
(going round the base counter-clockwise) covers it; `corners` treats one corner its own way, named by
its two flanges, with `over` for the flange that covers it. Bend angles and corner angles other than 90
degrees close the same way (the ends are then slanted in the flat pattern); where the end comes nearer
than the bend's end (walls that lean towards each other, a wide corner without a relief) the flange is cut
back to it, its tip shortened and, where the end crosses the bend, the bend trimmed by it too. `kind` and the relief's
`shape` may be expressions that pick by number, as a Jig's choice parameter counts (`corner`: 1 open, 2
butt, 3 overlap, 4 miter; `shape`: 1 rect, 2 round, 3 tear), so a form can offer them. Opening or closing a corner keeps every face's name: a
side's end is named as the side was. The measure
`cornerGap` is the least gap at the closed corners, measured on the part. Refused, with the reason: a
gap that is not more than the tolerance (a butt's must also be less than the thickness), two flanges
bent opposite ways, a corner whose end would cut one skin of a bend and not the other (give it a relief),
and extensions that overlap in the flat pattern or run into each other when bent. A tray with closed
corners, one of them covered the other way round:

```json
[
 ["variable.set", {"name": "seam", "expr": "3"}],
 ["sketch.create", {"plane": "xy", "name": "BoxBase"}],
 ["sketch.rect", {"sketch": "BoxBase", "from": [0, 0], "to": [120, 80]}],
 ["feature.sheetMetal", {"sketch": "BoxBase", "name": "Box", "thickness": "1.5 mm", "radius": "2 mm",
   "kFactor": 0.42, "relief": {"shape": "round", "size": "4 mm"},
   "corner": {"kind": "seam", "gap": "0.3 mm"},
   "corners": [{"flanges": [0, 1], "kind": "overlap", "over": 1}],
   "flanges": [{"edge": "e5", "length": "20 mm", "angle": "90 deg"},
               {"edge": "e6", "length": "20 mm", "angle": "90 deg"},
               {"edge": "e7", "length": "20 mm", "angle": "90 deg"},
               {"edge": "e8", "length": "20 mm", "angle": "90 deg"}]}],
 ["assert.add", {"name": "Seams", "measure": "cornerGap", "of": "Box", "eq": "0.3 mm"}],
 ["file.export", {"path": "box.dxf", "sheetMetal": "Box"}]
]
```

**Inset flanges, tabs and outlines.** A flange need not span its whole edge: `insetStart` and `insetEnd`
stand its bend back from the edge's start and end (the edge's direction is the base's, counter-clockwise
round the outline), so one edge can carry several tabs, kept apart. A flange can sit on a `side` of an
earlier flange (`{"flange": 0, "end": "start"}` or `"end"`, the side at its bend's start or end: a tab that
wraps round a corner), and on a straight edge of a hole of the base, where it grows into the hole (a tab
bent out of a cut-out, a louver; give it insets, or its bend's ends lie along the hole's sides). With
`outline` a flange's flat is the closed profile of a sketch of its own (holes allowed) instead of a
rectangle `length` long: draw it in the base sketch's plane as if the bend had no width (on a flange's tip
or side, as if every bend from the base to it had none, so it follows a change of thickness or radius), one line of it
on the edge's line and the rest beyond; the bend spans that line and the flat is laid out the bend
allowance further on (give no `length`). Such a flange has no tip or sides; a later flange sits on one of
its outline's lines instead, `"line": {"flange": 0, "edge": "e14"}` (the line's id in the outline's
sketch; not the line on the edge, which is the bend): a lip on a rail's leg. The flat pattern is checked as a whole, so tabs whose flats would
overlap in it are refused naming them. A flange added later changes no name the part had but the end
along the edge it is bent from. Corner reliefs and closed corners are for flanges that reach a corner of
the base; a corner with an inset flange is not one, while a flange with an outline reaches it when its
outline's line on the edge ends there (with a relief, the relief's size short of it). Where a bend ends inside its edge (an inset flange, a
tab), `bendRelief` (`{"shape": "rect", "size": "1.5 mm"}`, the shapes of `relief`) cuts a notch beside the
bend's end, `size` along the edge and `size` deep into the flat it is bent from, so the sheet does not tear
there (a tear, the disc tangent to the edge and the bend's end, reaches twice `size` both ways); the
bend table lists each bend's `ends` inside its edge with their relief, and its `length`. A flange on a
wall's tip that spans the tip, where a closed corner runs the wall on past it, gets its notch in that
extension. A notch that would cut across another edge of its flat (a tear on a short wall with a large
bend radius) is refused, naming the flange: make it smaller or use round or rect. A
wall whose tip carries tabs (screw tabs) closes its corners like any other; a side that carries a flange
moves to the corner's end with its flange when that end lies along it (walls square to the base and
to each other: a tab round the corner over the other wall), and is refused where the end slants across
the wall. An outlined flange closes
its corner too: its side there is replaced by the corner's end and its outline's next line run on to it. A cover with an inset wall, a tab on its side, an ear
of its own outline with a hole, and a tab bent down out of a cut-out:

```json
[
 ["sketch.create", {"plane": "xy", "name": "CoverBase"}],
 ["sketch.rect", {"sketch": "CoverBase", "from": [0, 0], "to": [100, 60]}],
 ["sketch.rect", {"sketch": "CoverBase", "from": [40, 15], "to": [60, 40]}],
 ["sketch.create", {"plane": "xy", "name": "Ear"}],
 ["sketch.rect", {"sketch": "Ear", "from": [40, 60], "to": [60, 75]}],
 ["sketch.circle", {"sketch": "Ear", "center": [50, 68], "radius": 3}],
 ["feature.sheetMetal", {"sketch": "CoverBase", "name": "Cover", "thickness": "1 mm", "radius": "1 mm",
   "kFactor": 0.42,
   "flanges": [{"edge": "e5", "length": "15 mm", "angle": "90 deg", "insetStart": "10 mm", "insetEnd": "10 mm"},
               {"side": {"flange": 0, "end": "end"}, "length": "10 mm", "angle": "90 deg"},
               {"edge": "e7", "outline": "Ear", "angle": "90 deg"},
               {"edge": "e13", "length": "12 mm", "angle": "90 deg", "direction": "down",
                "insetStart": "3 mm", "insetEnd": "3 mm"}]}],
 ["file.export", {"path": "cover.svg", "sheetMetal": "Cover"}]
]
```

A tray closed by overlap whose front wall carries two screw tabs with bend reliefs, checked on a press
brake in the order a shop would bend it (the walls, then both tabs):

```json
[
 ["sketch.create", {"plane": "xy", "name": "TrayBase"}],
 ["sketch.rect", {"sketch": "TrayBase", "from": [0, 0], "to": [60, 40]}],
 ["feature.sheetMetal", {"sketch": "TrayBase", "name": "Tray", "thickness": "1 mm", "radius": "1 mm",
   "kFactor": 0.42, "relief": {"shape": "round", "size": "3 mm"},
   "corner": {"kind": "overlap", "gap": "0.25 mm"}, "bendRelief": {"shape": "rect", "size": "1.5 mm"},
   "flanges": [{"edge": "e5", "length": "10 mm", "angle": "90 deg"},
               {"edge": "e6", "length": "10 mm", "angle": "90 deg"},
               {"edge": "e7", "length": "10 mm", "angle": "90 deg"},
               {"edge": "e8", "length": "10 mm", "angle": "90 deg"},
               {"tip": 0, "length": "6 mm", "angle": "90 deg", "insetStart": "8 mm", "insetEnd": "36 mm"},
               {"tip": 0, "length": "6 mm", "angle": "90 deg", "insetStart": "36 mm", "insetEnd": "8 mm"}]}],
 ["assert.add", {"name": "Relieved", "measure": "relief", "of": "Tray", "is": true}],
 ["sheetMetal.bends", {"feature": "Tray"}]
]
```

The measures read a part as it was built, by its feature's name, also after a boolean took its body; with
no `of` they measure every part that is switched on, and a part named while it is switched off is an error
that says so. `file.export` with a list in `sheetMetal` puts several parts' flat patterns side by side on
one sheet (`gap`, `width`, as a sheet of a set's faces). In the window, File › Export to .dxf or .svg
offers "Flat pattern of" each part, "Crease pattern of" each fold, and with two parts or more "Flat
patterns of every part" on one sheet; with a part's body selected the export is its flat pattern.

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
