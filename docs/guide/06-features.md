# 6. Modelling features

What each kind of feature does, with the parameters that matter. The full list of parameters of any
command is in `command_list` (or `craft3d-cli commands --filter <id>`), with a summary that also says
what each refuses. The [examples](../examples/README.md) show every feature below in a part that checks
its own volume.

## Primitives

`feature.block` (`size`, `position` of its low corner), `feature.cylinder` and `feature.cone` (`radius`,
`height`, `position` of the centre of the base, `axis`), `feature.sphere`, `feature.torus` (`major`,
`minor`). Their faces are numbered in a fixed order: a block's 0 bottom, 1 top, 2 front, 3 back, 4 left,
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
faces into a solid layer.

## Several bodies

A document may hold any number of bodies; every feature that does not consume another body adds one.
`body.split` splits a body that fell into pieces (a cut through the middle) into one body per piece.
`body.appearance` and `face.appearance` give colours, `body.material` a density (then `document.inspect`
gives the mass), `document.interference` says whether bodies overlap.

## Hiding and looking back

`feature.visible` with `"visible": false` hides a feature's body, sketch or datum in the view (the
window, `view.render`, `view.pick`); `true` shows it again. It is not `feature.suppress`: the feature is
still built, measured and exported, and later features are still made from it. It is saved with the
document and undone like a colour; `document.inspect` marks it `"hidden": true`.

`view.scrub` shows the model as it was after one step (`{"step": 3}` or `{"feature": "Plate"}`;
`{"feature": null}` shows it as it is again), without changing the document: the timeline in the window
does the same. To build on from that step, move the real marker there with `history.rollback`.

## Jigs: parameters

A **Jig** is a document that others set without knowing how it is modelled: a box sized by its width,
depth and height, a pulley by its tooth count. You make one by **publishing** the variables that drive it.
A published variable is a public parameter, with a label, a group, a range and a step:

```json
[
  ["variable.publish", {"name": "width", "label": "Width", "description": "Outside width", "group": "Size",
                        "min": "40 mm", "max": "120 mm", "step": "1 mm"}],
  ["variable.publish", {"name": "lid", "label": "Thick lid", "group": "Finish", "kind": "boolean"}],
  ["variable.publish", {"name": "corners", "label": "Corners", "group": "Finish", "kind": "choice",
                        "choices": ["sharp", "round"]}],
  ["document.info", {"title": "Storage box", "author": "You", "licence": "CC-BY-4.0"}]
]
```

`kind` is `number` (a value of its unit), `integer`, `boolean` (0 or 1) or `choice` (1 for the first
label). Its value must be a value (`40 mm`), not a formula; other variables may be formulas of it
(`radius = 0.5 mm + (corners - 1) * 4.5 mm`). `param.list` lists the parameters as a form shows them;
`param.set {"values": {"width": 90}}` sets several at once and **refuses** a value outside its range, with
the range, so an agent can correct it; `variable.unpublish` makes one an ordinary variable again.

**In the window** a Jig opens on its **Parameters** tab, beside **Features**: the title and description
from `document.info`, the groups (click a group to fold it), and a field per parameter. Drag a number
across its field (the bar is where it lies in its range; Shift is finer, Ctrl goes in steps), click the
‹ › at its ends, or press Enter over it and type a value with or without its unit. A boolean is a
checkbox, a choice a list. The model follows as you drag; the whole drag is **one undo step**. In the
browser the model follows when you let go, because there a regeneration holds up the page.

The form keeps a value in its range: typing 150 into a width that goes up to 120 sets 120 and says
*clamped to 120 mm* under the field for a few seconds (and *rounded to …* for a value between two steps).
A parameter that is not at its default has a ↺ to put it back; **Reset all** puts them all back
(`param.set {"reset": true}`).

**Publishing and using a Jig.** A Jig is designed in an ordinary `.3dcraft` document. When it is ready,
**File › Export as Jig…** (`jig.export {"path": "box.3dcraftjig"}`) writes it as a `.3dcraftjig` file, with its
info and a picture; this is the file you hand out. Opening a `.3dcraftjig` opens the Jig **to use**: the
Parameters tab shows, and only the parameters change. The feature list is there to look at (and the
timeline to play), not to edit; an agent that tries gets *this is a Jig: … save it as a model to edit it*.
**Save** saves it as a `.3dcraft` model under a name you choose, never over the `.3dcraftjig`; **Open for
editing** (on the Parameters tab or in the File menu, `jig.edit`) makes a new untitled model of it, set as
it is, to change as you like. Closing a Jig whose parameters you changed asks whether to **save the result as a
model**; the Jig's file stays as it is.

When a value breaks the model (a fillet that no longer fits, a shell thicker than the box, an assertion
that fails) the field turns red with the reason, and a banner over the model says *This value breaks the
model* with a **Back to last good** button that sets the values that built. The viewport shows the model
as it is, broken or not. A parameter whose range no longer works (a variable it
used was deleted) shows the problem under its field in the attention colour.
