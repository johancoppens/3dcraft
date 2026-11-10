# Modelling features

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
  [pipe](../examples/pipe.json), [spring](../examples/spring.json). With a second `rail` it is a sweep
  along two rails (below).
- **Loft** (`feature.loft`): through two or more profiles on planes one after the other; smooth, or
  `ruled` (straight from each to the next). Example: [hopper](../examples/hopper.json).
- **Offset profile** (`feature.offsetProfile`): every loop of a sketch grown or shrunk by a distance;
  extrude it, or export it as a laser's path with its kerf.

## Sweep along two rails

`feature.sweep` with `rail` carries the profile along two curves at once: two of its points follow the
two rails, so it grows and shrinks as they part and close in. `path` and `rail` are each a sketch of one
open chain (lines, arcs and splines end to end), and each starts on the outline of the profile (or both
close into rings, below). The two
are walked together by the fraction of each one's length, the profile's x axis along the line between
them and its y axis as near to the normal of the `path`'s sketch as that allows: for rails drawn in one
plane, the sections stand square to it, so draw the profile on the plane through both rails' starts
that stands square to theirs. `scale` is `"uniform"` (the default: a circle stays a circle) or
`"width"` (only along the line between the rails; the height stays as drawn). A tapering spout, a ring
whose hole tapers with it:

```json
[
 ["variable.set", {"name": "r0", "expr": "15 mm"}],
 ["variable.set", {"name": "r1", "expr": "6 mm"}],
 ["variable.set", {"name": "ri", "expr": "12 mm"}],
 ["variable.set", {"name": "len", "expr": "60 mm"}],
 ["sketch.create", {"name": "Mouth", "plane": "zx"}],
 ["sketch.circle", {"sketch": "Mouth", "center": [0, "r0"], "radius": "r0"}],
 ["sketch.circle", {"sketch": "Mouth", "center": [0, "r0"], "radius": "ri"}],
 ["sketch.create", {"name": "Left", "plane": "xy"}],
 ["sketch.polyline", {"sketch": "Left", "points": [[0, 0], ["r0 - r1", "len"]]}],
 ["sketch.create", {"name": "Right", "plane": "xy"}],
 ["sketch.polyline", {"sketch": "Right", "points": [["2 * r0", 0], ["r0 + r1", "len"]]}],
 ["feature.sweep", {"sketch": "Mouth", "path": "Left", "rail": "Right", "name": "Spout"}],
 ["assert.add", {"name": "volume", "measure": "volume", "of": "Spout", "eq": "pi * len / 3 * (r0^2 + r0 * r1 + r1^2) * (1 - (ri / r0)^2)"}]
]
```

![The spout: a ring between two rails that close in](images/two-rail-spout.png)

Where both rails are parallel lines of the same length the faces are exact planes and cylinders.
Elsewhere they are spline surfaces, and the result says under `path` how far they are from the true
sweep at most (`deviation`): a bound that is proven, never more than half the document's tolerance (a face
within an eighth of the tolerance of a plane is made that plane, and a profile drawn a little off its
plane is laid into it, which moves it by at most the tolerance). A tolerance finer than 4096 units in the
last place of the model's size is refused: below that, rounding is not bounded. A
face is named after the curve of the profile and the curves of the two rails it runs between
(`F4:side(e2,e3,e3)`), so a rail can be redrawn without losing what is built on the faces. Refused, with
the place: a rail that does not start on the profile, rails that meet or cross, rails that start at one
point, and a sweep that would fold or pass through itself.

**Rings.** Both rails may close: a sketch of lines, arcs and splines that close into one loop, or one
circle. Each ring has to pass through the outline of the profile, and the body is a closed ring
without ends (`closed` in the result). A circle carried round two concentric circles is a torus:

```json
[
 ["variable.set", {"name": "major", "expr": "40 mm"}],
 ["variable.set", {"name": "minor", "expr": "6 mm"}],
 ["sketch.create", {"name": "Section", "plane": "zx"}],
 ["sketch.circle", {"sketch": "Section", "center": [0, "major"], "radius": "minor"}],
 ["sketch.create", {"name": "Inner", "plane": "xy"}],
 ["sketch.circle", {"sketch": "Inner", "center": [0, 0], "radius": "major - minor"}],
 ["sketch.create", {"name": "Outer", "plane": "xy"}],
 ["sketch.circle", {"sketch": "Outer", "center": [0, 0], "radius": "major + minor"}],
 ["feature.sweep", {"sketch": "Section", "path": "Inner", "rail": "Outer", "name": "Ring"}],
 ["assert.add", {"name": "torus", "measure": "volume", "of": "Ring", "eq": "2 * pi * major * pi * minor^2"}]
]
```

![A circle carried round two rings: a torus](images/two-rail-ring.png)

**The up direction.** `up` sets the direction the profile stands in with the line between the rails,
instead of the normal of the `path`'s sketch: `"x"`, `"y"`, `"z"`, two face names for the straight edge
between them, or a datum axis (which way it points does not matter; `null` in `feature.edit` goes back
to the default). Draw the profile on the plane through both rails' starts that holds it. The profile
stands in the plane of the line between the rails and the up direction, so tilting the up direction
leans it along the rails' run. A wall 4 mm thick between two straight rails 60 mm long, drawn on the zx
plane tilted 30° about x and standing along the line where that plane meets the yz plane, leans back by
30° towards the start of the rails (its top is h sin 30° = 15 mm behind its foot), on a ground plate;
its volume is its section times its run times cos 30°:

```json
[
 ["variable.set", {"name": "h", "expr": "30 mm"}],
 ["datum.plane", {"from": "zx", "angle": "30 deg", "about": "x", "name": "Tilt"}],
 ["datum.axis", {"planes": ["Tilt", "yz"], "name": "Lean"}],
 ["sketch.create", {"name": "Blade", "plane": "Tilt"}],
 ["sketch.rect", {"sketch": "Blade", "from": [0, 0], "to": ["h", 4]}],
 ["sketch.create", {"name": "Near", "plane": "xy"}],
 ["sketch.polyline", {"sketch": "Near", "points": [[0, 0], [0, 60]]}],
 ["sketch.create", {"name": "Far", "plane": "xy"}],
 ["sketch.polyline", {"sketch": "Far", "points": [[4, 0], [4, 60]]}],
 ["feature.sweep", {"sketch": "Blade", "path": "Near", "rail": "Far", "up": "Lean", "name": "Wall"}],
 ["feature.block", {"size": [24, 100, 2], "position": [-10, -30, -2], "name": "Ground"}],
 ["assert.add", {"name": "wall", "measure": "volume", "of": "Wall", "eq": "4 mm * h * 60 mm * cos(30 deg)"}]
]
```

![The wall seen from the side: it leans back by 30° along its run](images/two-rail-lean.png)

An up direction along the rails would lay the profile into their course; that is refused as the fold
it makes.

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
  rounded box gets inner rounds of the radius less the wall. Example: [case](../examples/case.json). Shell
  before a modelled thread: a thread's flanks curve too tightly to be offset, and the shell after it
  fails and says so.
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
faces into a solid layer. `feature.fold` folds a sheet along a crease pattern ([Origami](origami.md)).

## Several bodies

A document may hold any number of bodies; every feature that does not consume another body adds one.
`body.split` splits a body that fell into pieces (a cut through the middle) into one body per piece.
`body.appearance` and `face.appearance` give colours, `body.material` a density (then `document.inspect`
gives the mass), `document.interference` says whether bodies overlap. A body's colour channels and its
density may be expressions of the variables (`["if(dark, 60, 200)", 120, 40]`, `"sheets[sheet].density
g/cm3"`), so they follow a parameter.

**A face's own colour wins over its body's**, wherever the body's is given: a body colour given to a later
feature (a transform, a boolean) colours the other faces and leaves the faces coloured earlier as they are.
To give the whole body its colour, clear its faces' own colours in one step:
`face.appearance {"body": "Moved", "clear": true}` takes away every face colour the body's faces carry,
wherever along its history it was given (an imported STEP part with a colour on every face too), as one
undo step, and lists what it cleared (a pattern's instances are one body: the seed's colour goes from
every instance). A colour that also colours another body's face (a copy kept by a mirror, the other piece
of a `body.split`) is kept, since clearing one body never changes another; the answer says for which body
and gives, under `kept`, the command that clears it from both: the face by its own name at the feature it
was given on, such as `face.appearance {"body": "F1", "faces": ["F1:face(1)"], "clear": true}`. Single
faces are cleared the same way. In the window, the body's card in the Bodies tab has **Clear face
colours** when its faces carry colours of their own, and a **Shared** row (*1 colour, with Twin*)
when some are shared.

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
there for the fillets and face operations after it. [Formulas, tables and conditions](formulas.md#switching-parts-on-and-off) explains both in full.

**Next:** [Measuring and checking](measure.md).
