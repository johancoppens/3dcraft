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

The bends are folded by the same mechanism as `feature.fold` (a joint with a radius), and the folded sheet
is given its thickness as `feature.plate` does, so a bend's skins are the cylinders of the inner radius and
of the inner radius plus the thickness. `sheetMetal.bends` is the **bend table**: per bend its angle and
inner radius measured on the part, the K-factor, the bend allowance (in closed form, and as `developed`:
the bend's neutral band laid flat by the unfold), the outside setback and the bend deduction, the
flange's length; and the flat pattern's area and size. The **manufacturability** measures for assertions
are `bendRadius` (the smallest inner radius), `flangeLength` (the shortest flange; `"measured": "mould"`
from the virtual sharp), `holeToBend` (the least distance from a hole to a bend's tangent line) and
`relief` (every corner where two bends meet is relieved). 3DCraft has no material catalogue: the limits
for a material and a thickness are yours, as expressions or a table, as above. `file.export` with
`sheetMetal` writes the flat pattern (.dxf or .svg): the outline on the layer `outline`, the bend lines
on `bendUp` (dashed) and `bendDown` (dash-dot). `sheetMetal.edit` changes a part (flanges as a whole
list). Refused, with the flange: a radius below zero or of zero (a sharp bend is not supported), a K-factor
outside 0 to 1, an angle outside 0 to 180 degrees, an edge that is not a straight line of the base's
outline, flanged edges meeting at an inside corner, a relief that does not fit, and flanges that run into
each other or into the base when bent. With a K-factor of ½ the part's volume is the flat area times the
thickness; with another K-factor the part (whose bends keep the skins at the two radii) differs from that
by angle × width × thickness² × (½ − K) per bend.

**Holes in flanges.** The base's sketch is drawn in the plane of the flat pattern, so its other closed
profiles (beside the base) are holes and cut-outs of the flange whose flat each lies in: a flange's flat
starts the bend allowance beyond its edge. `cutouts` adds the closed profiles of other sketches in the same
plane (any origin and axes) the same way, and each sketch switches with its own `when`, so a vent or a
cable hole can be an option. The holes are cut in the flat pattern and in the folded part, and
`holeToBend` measures each from the bends beside its own flat. A hole across a bend's strip or past the
pattern's edge is refused, and so is the part when a `cutouts` sketch fails (fix the sketch, or switch it
off). A hole cut later by another feature (a boolean) is not in the flat pattern
(the bend table and the export say which feature took the part as `changedBy`): draw it in the part's
sketch or in a `cutouts` sketch instead. A panel with a hole in its flange and an optional vent:

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
