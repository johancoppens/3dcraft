# Sheet metal

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

A **curved flange** (`curvedFlanges`) is bent along an arc or a circle of the base (its `edge`): the rim
of a round pan, a collar round a round hole, the rounded corners of a tray. In the part it is exact (every
section across the edge is a straight bend's, so the bend is a piece of a torus and the flat a piece of a
cone). A torus cannot be laid flat without stretching, so the flat pattern is **approximated**: along every
line across the edge it is laid out as a straight bend is, and the sheet is compressed along the edge (a
flange growing away from the arc's centre) or stretched (towards it). How much is proven, not estimated:
the bend table gives each curved flange's `strain`, the export says the pattern is approximated and by how
much, `bendStrain` measures it for your material's limit, and `maxStrain` on the part refuses a flange that
would need more. Where a curved flange meets a straight one tangent to it with the same angle, radius,
K-factor and length, they are one wall: a tray with rounded corners has no seam at its corners and its flat
pattern is one outline.

```json
[
 ["sketch.create", {"plane": "xy", "name": "PanBase"}],
 ["sketch.line", {"sketch": "PanBase", "from": [25, 0], "to": [75, 0]}],
 ["sketch.line", {"sketch": "PanBase", "from": [100, 25], "to": [100, 35]}],
 ["sketch.line", {"sketch": "PanBase", "from": [75, 60], "to": [25, 60]}],
 ["sketch.line", {"sketch": "PanBase", "from": [0, 35], "to": [0, 25]}],
 ["sketch.arc", {"sketch": "PanBase", "center": [75, 25], "start": [75, 0], "end": [100, 25]}],
 ["sketch.arc", {"sketch": "PanBase", "center": [75, 35], "start": [100, 35], "end": [75, 60]}],
 ["sketch.arc", {"sketch": "PanBase", "center": [25, 35], "start": [25, 60], "end": [0, 35]}],
 ["sketch.arc", {"sketch": "PanBase", "center": [25, 25], "start": [0, 25], "end": [25, 0]}],
 ["feature.sheetMetal", {"sketch": "PanBase", "name": "Pan", "thickness": "1 mm", "radius": "2 mm",
   "kFactor": 0.4, "maxStrain": 0.3,
   "flanges": [{"edge": "e3", "length": "8 mm", "angle": "90 deg"},
               {"edge": "e6", "length": "8 mm", "angle": "90 deg"},
               {"edge": "e9", "length": "8 mm", "angle": "90 deg"},
               {"edge": "e12", "length": "8 mm", "angle": "90 deg"}],
   "curvedFlanges": [{"edge": "e16", "length": "8 mm", "angle": "90 deg"},
                     {"edge": "e20", "length": "8 mm", "angle": "90 deg"},
                     {"edge": "e24", "length": "8 mm", "angle": "90 deg"},
                     {"edge": "e28", "length": "8 mm", "angle": "90 deg"}]}],
 ["sheetMetal.bends", {"feature": "Pan"}],
 ["assert.add", {"name": "Corner strain", "measure": "bendStrain", "of": "Pan", "max": "0.3"}]
]
```

![A tray with corners rounded to 25 mm: four walls and four curved corners, one wall all round](images/curved-tray.png)

Its corners, a wall 8 mm high (10 mm to the rim) on a radius of 25 mm, compress the top of the wall by up
to 27 per cent along the corner (the bend table's `strain`): a drawn corner, not one a press brake can
fold. A larger corner radius or a lower wall lowers it. A drawn part's blank is in practice sized by equal
area and corrected by trial; the flat pattern here keeps the lengths across the bend and states the strain
along it.

The bends are folded by the same mechanism as `feature.fold` (a joint with a radius), and the folded sheet
is given its thickness as `feature.plate` does, so a bend's skins are the cylinders of the inner radius and
of the inner radius plus the thickness. `sheetMetal.bends` is the **bend table**: per bend its angle and
inner radius measured on the part, the K-factor, the bend allowance (in closed form, and as `developed`:
the bend's neutral band laid flat by the unfold), the outside setback and the bend deduction, the
flange's length; and the flat pattern's area and size. The **manufacturability** measures for assertions
are `bendRadius` (the smallest inner radius), `flangeLength` (the shortest flange; `"measured": "mould"`
from the virtual sharp), `holeToBend` (the least distance from a hole to a bend's tangent line) and
`relief` (every corner where two bends meet, and every end of a bend inside its edge, is relieved) `cornerGap` (the least gap at the closed corners), and `flatExtent` (`axis` x or y: the width or height of the flat pattern, the blank it needs), `flatArea` and `bendStrain`. 3DCraft has no material catalogue: the limits
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
changes the search's cap of 1 000 000 distances, some 5 to 8 minutes at most (a box of 7 bends takes
some 80 000 with one-piece tools and 700 000 segmented, 2000 to 3500 a second; a search that runs out
says about how much the whole search would take, so give that as `maxWork`, up to 10 000 000). The measure `bendable` keeps its cap of 20000. `sheetMetal.edit` changes a part (flanges as a
whole list). Each flange has an `id` (`sheetMetal.bends` lists them, 1, 2, … in the order the part was
made, a new flange's never one used before): every name of the flange's faces and edges is numbered by it
(its bend `face(2 · id − 1)`, its flat `face(2 · id)`, its rims and edges), not by its place in the list,
so a fillet, a colour or a set on it stays with it whatever else you add, delete or move. Give the id back
with the flange in `sheetMetal.edit`; a flange sent back unchanged keeps its id without it, and one changed
without it is a new flange (its old names then name nothing, an error where used). `tip`, `side` and `corners` name
flanges by their places in the list you send; corners you do not send again are kept and follow their
flanges by id to their new places (one whose flange you deleted is refused, naming it). A `radius` of 0 is a **sharp bend**: a sharp inside corner (the inner skin is the line on the bend's axis) and an outside rounded to the thickness, with the bend allowance angle × kFactor × thickness, so the flat pattern and the volume follow the same rules as any bend; its faces are those of the bend with a radius less its inner skin, so a radius set to 0 and back keeps every other name. A sharp bend is refused at or near 180 degrees (a closed hem), at a kFactor of 0, where another wall would touch its inner skin (two sharp flanges meeting at an open corner without a relief, a wall leaning in beside a sharp square wall, a flange on a side of a sharp wall), trimmed by a closed corner, and where a hole crosses it other than straight across (a slot or notch whose sides run square across the bend line is cut through it); a cut made later through a sharp bend is not carried onto the exported flat pattern (the export lists it under notCarried). A formed louvre whose hood comes within about the thickness of a sharp bend's inside is refused too: its hood is checked by a box round it, which errs on the safe side. Refused, with the flange: a radius below zero, a K-factor
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
through the bend) is cut through the bend where the bend allowance puts it, also beside a bend trimmed by a closed corner, and a hole drawn across a jog's line is cut across the jog's bends; one that only touches a
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
