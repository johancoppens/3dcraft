# 3DCraft design: A body cut by planes

> **Status: a design, not built (2026-11-20).** The command `feature.planeCut` *(planned)*, the feature kind, the name role `cut` and the kernel module `halfspace` named below do not exist yet. This document is the proposal handed to the coordinator before the first slice is built; when a slice lands, what it built moves under **Method**, written as built, and the rest stays under **Design** until it is built or dropped. Roadmap: [`../roadmap.md`](../roadmap.md) › *M7* › *Open*; decision: [`../decisions.md`](../decisions.md), 2026-11-20; asked in [issue 3](https://github.com/johancoppens/3dcraft/issues/3) of the public repository.

A body cut by one or more planes in one feature, one side of each kept. Today each plane costs a datum plane, a sketch, a rectangle, an extrusion and a boolean, and the person has to size the rectangle and the extrusion "large enough"; a prism trimmed by 24 measured planes carries a history of about a hundred entries (measured below: 98). It serves any shape (rule 14): a corner cut off a block, a roof, a cutaway of a finished model, a part trimmed to a datum.

## Design

### The command

```text
feature.planeCut
  params:  {"target": id or name, "planes": [plane, ...] where plane is "xy|yz|zx" or datum plane (id or name) or a flat face by name, or {plane: one of those, offset: expr = 0, side: below|above = below, id: number = a new one}, or {point: [expr, expr, expr], normal: [expr, expr, expr], offset: expr = 0, side: below|above = below, id: number = a new one}, "keep": boolean = false, "name": string = automatic, "when": expr = always on}
  summary: Adds a feature whose body is the body of target cut by planes, one side of each kept: what lies on the kept side of every plane (several planes: the intersection). No tool is drawn or sized: the cut reaches as far as the body does. A plane is a base plane, a datum plane (the cut follows it), a flat face of an earlier body by its name (its normal points out of that body), or a point and a normal given as expressions (the point three lengths, the normal three plain numbers of any length, so both can be variables; a plane a x + b y + c z = d is normal [a, b, c] through the point [a d / s, b d / s, c d / s] with s = a² + b² + c²), moved by offset along its normal. side says what is kept: below (the default) is the side the normal points away from, as measure.below has it, above the side it points to. One plane may be given alone instead of a list. The target's body goes into this feature; with keep it keeps its body and this is a cut copy (cut a kept copy once on each side to have both parts). Each plane has an id (given, or the next free one from 1; the result lists them under planes, with cuts: whether it left a face), and the face it cuts is named by it, "F9:cut(3)", so the name stays with its plane when others are added, deleted or put in another order; a plane that cuts the body in several places gives several faces of that name. What is left of the target's faces keeps its names with a prefix, "F9:copy(0)/...". A plane that does not meet the body cuts nothing and is listed under warnings; one that leaves nothing of the body fails the feature, naming the plane. Nothing is snapped: planes that nearly meet in a point leave the small faces between them, and where faces or points come between half and twice the document's tolerance of each other the feature fails with the plane and the faces (planes meant to meet in one point: give them the same point). Refused as well: a sheet as the target, a face that is not flat, a normal of no length, parts left touching along an edge or in a point. feature.edit changes target, planes (the whole list; a plane given with its id stays that plane) and keep.
```

- **`target`**: the feature whose body is cut, as `feature.boolean` takes its target. The body goes into this feature; with `keep` the target keeps its body and this feature is a cut copy (as `feature.transform` and `feature.mirror` keep theirs), which is how both sides of one cut, or two different cuts of one body, are had.
- **`planes`**: 1 to 256 planes; one plane may be given alone instead of a list. Each is one of:
  - a base plane (`"xy"`, `"yz"`, `"zx"`), a datum plane by id or name, or a flat face of an earlier body by its persistent name, exactly as `datum.plane`'s `from`, `feature.draft`'s `neutral` and `measure.below`'s `plane` read them (`datum_cmds::plane_value`, stored as `doc::PlaneRef`); the cut follows the datum or the face. A face named as an earlier body had it is that face as it was there (the rule for every reference that does not take the face's body, [`names.md`](names.md) › *References that follow a body*), so a body can be cut by the plane of one of its own faces moved by `offset`;
  - `{point, normal}`: the plane through `point` (three lengths) square to `normal` (three plain numbers or three lengths, any length: only where it points counts, as `feature.block`'s `axis`). All six are expressions, kept as source text, so they follow variables and a Jig's parameters: a plane given through variables is changed with `variable.set`, without sending the list again. A plane known by its coefficients `a x + b y + c z = d` is `normal: [a, b, c]`, `point: [a d / s, b d / s, c d / s]` with `s = a² + b² + c²`; the summary says so.
  - Either form may carry `offset` (a length expression: the plane moved along its normal, as `sketch.create` and `measure.below` have it), `side` and `id`.
- **`side`**, per plane: `below` (the default) keeps what lies on the side the normal points away from, `above` what lies on the side it points to. The words and their meaning are `measure.below`'s, so the volume `measure.below` reports for a plane is the volume this feature leaves. A base plane's normal is its third axis (xy: +z), a datum plane's its own, a face's points out of the body the face is of (so `below` keeps what lies behind the face), a `{point, normal}` plane's is the one given.
- **Several planes mean the intersection of the kept half-spaces**: the result is every point of the target that is on the kept side of every plane. The planes are applied in the order given; the point set does not depend on the order, the names do not either (below), and only which refusal is met first can.
- **`id`**: every plane of a feature has an id of its own, a number from 1, which names the face it cuts (below). Without `id` a plane gets the next free one (1, 2, 3, … in the order given when the feature is made); an id is never given out twice in one feature, also not after its plane was deleted (the feature keeps the next free id, as a sketch keeps `next_entity_id`). The result and `document.inspect` list the ids.
- **`when`** and **`name`** as for every feature. Switched off, the feature passes its target's body on, named as when it is built (`F9:copy(0)/…`), the rule for every feature that changes a body ([`regeneration.md`](regeneration.md) › *Switched off*); with `keep` it passes nothing on.
- **`feature.edit`** changes `target`, `planes` and `keep`. `planes` is the whole list, by the rule `sheetMetal.edit` has for flanges: a plane given with its id is the feature's plane of that id (an id the feature has no plane of, or one given twice, is refused); one given without is the plane it equals in every field, and else a new plane with a new id. So a plane inserted in the middle, one deleted, and a list in another order leave every other plane its id.
- **The result** has `id`, `name`, `status` and `planes`: per plane its `id` and `cuts` (whether it left a face); a plane that cuts nothing is also listed under `warnings`.

Rejected:

| Alternative | Why not |
|---|---|
| `feature.cut` | "Cut" already means two other things here: `feature.extrude`'s `cut` (an extrusion taken away) and the `cut.*` family (parts cut from sheet stock); and in FreeCAD, whose users will read it so, *Cut* is the boolean difference. A command of that name would be expected to take a tool. |
| `feature.trim` (the issue's word) | `feature.surface.trim` bounds a face's surface by curves, and "a sheet trimmed by a solid" is the boolean's own term ([`booleans.md`](booleans.md) › *Sheets*); a later trim of a solid by a surface would want the word. |
| `body.split` with a plane | `body.split` makes one feature per lump of what is already apart; it does not cut. |
| One plane per feature | 24 history entries and 24 kept bodies where one is asked for; it stays possible (a list of one). |
| `op: keepBelow \| keepAbove` for the whole feature | A cut by a face and by a datum at once needs a side per plane; and `feature.edit`'s `keep` is already a boolean (a mirror's), so the side cannot be called `keep`. |
| Coefficients `[a, b, c, d]` as a third form | `d` is a length only when the normal is a unit vector, which measured coefficients need not be; the conversion is two lines for the caller and one sentence in the summary. Open question 7. |
| "Any" instead of "all" (keep what is below at least one plane) | A union of half-spaces is the target less a convex wedge; roofs that are not convex (an L-shaped hip, crossing gables) are unions of *intersections*, which neither form gives alone: cut a kept copy per convex part and unite them with `feature.boolean`. Open question 8. |

### Names

Three kinds, nothing new in how names are followed or resolved ([`names.md`](names.md)):

- **The face a plane leaves: `F9:cut(3)`**, the face plane 3 of feature F9 cut. A new role `cut`, a face, with one plain number: the plane's id. Where a plane cuts the body in several places (two legs of a table, a ring cut across) the pieces share the name, by the rule for a face in pieces: accepted where one surface is meant (a sketch's plane, a datum, a pair of faces for a fillet or a chamfer, which takes every edge between them), and told apart per lump by `body.split`. Its frame, for a sketch on it: the plane's own (a datum plane's origin and axes, a face's, a base plane's; for `{point, normal}` the origin is the point and x the world axis most square to the normal made square, `measure.below`'s default), moved by `offset`; with `side: above` seen from the other side (x stays, y and z turn round, `Frame::reversed_at`, as for any face whose outward normal is against its surface's). The normal always points out of the kept body. For `{point, normal}` the x axis is a choice among the world's axes, so it jumps when the normal passes a diagonal; a sketch that must not turn is put on a datum plane, or given `sketch.create`'s `xAxis`.
- **What is left of the target's faces: copies in instance 0**, `F9:copy(0)/F2:side(e71)`, as after every feature that takes a body; a face cut away is not among the names.
- **Edges and vertices: by their faces**, as after a boolean: `F9:between[F9:copy(0)/F2:side(e71),F9:cut(3)]` where a wall meets a cut face, `F9:between[F9:cut(3),F9:cut(4)]` a hip between two roof planes, `F9:corner[F9:cut(3),F9:cut(4),F9:cut(5)]` an apex; an edge or a vertex the target already had, kept whole, also answers to the copy of its old name (`F9:copy(0)/F1:edge(4)`), and the kernel's report of kept edges is stored so that an edge's own name is followed through this feature (`follow::edge`).

*A plane on a face of the target.* Where a piece the plane cuts joins a face of the target that lies in the plane and faces the same way (a stepped block cut at its shoulder), the two are one face, as after any boolean: named after the target's face, the copy, and answering to `F9:cut(k)` as an alias (the kernel lists both under `BooleanResult::merged`). Where the plane cuts nothing at all (on the top of a block, the block below it; a plane given twice) there is no `cut(k)`: the target's face keeps its copy name and the plane is listed as cutting nothing. So a reference to `cut(k)` holds while the plane cuts something and fails by name when it no longer does, and never moves to another face; whether the target's face in the plane should answer to `cut(k)` there too (the kernel knows which faces coincide) is left to slice 2, with a measurement of what refers to such faces.

*What an edit does* (each a scenario of the naming suite, below):

| Edit | Names |
|---|---|
| a plane moved or turned, the topology the same | every name stays on its entity |
| a plane added, in the middle of the list or at its end | every other plane keeps its id, so every name stays; the new plane's face is `cut(new id)`; an edge the new plane cuts in two loses its bare name (two longer ones appear), as after any cut |
| a plane deleted | references to its face and to edges on it fail with the name; its id is not given out again, so nothing else can take the name |
| the list given in another order | nothing changes |
| a plane no longer meets the body | `cut(id)` is gone: what refers to it fails naming it and saying that plane `id` cuts nothing now; moving the plane back repairs it |
| a plane now cuts a referenced face away | the reference fails with the name, never lands on another face |
| the target edited upstream (a hole added, a size changed) | cut faces keep `cut(id)`; copies follow their faces as through any taker |
| `target` pointed at another body | cut faces keep `cut(id)`; the copies are of the new target's names, and a reference to a face of the old one fails by name |

*A reference to a face of the target follows into the result* as through any feature that takes a body: a blend or a face operation after the cut that names `F1:face(1)` is followed to `F9:copy(0)/F1:face(1)` (`follow::face` looks for the copy in instance 0); gone fails with the name and the faces that are there.

*Proven identity.* `cut(n)` of a plane cut goes on the list of kinds proven to be identity (`doc::identity`, [`names.md`](names.md) › *Which parts of a name are proven to be identity*): the id is stored with the plane and never reused, the argument a flange's id has. So `F9:between[F9:cut(3),F9:cut(4)]` is an edge name a fillet takes by itself. A `between[…](n)` among several edges on the same faces stays unproven, as everywhere.

Rejected: `F9:face(n)` with the plane's id (no new role, as sheet metal numbers its faces): `face(n)` on a block means a fixed side, and a name that reads `F9:face(3)` beside `F9:copy(0)/F1:face(3)` says less than `F9:cut(3)` in every error that lists names. `F9:cut(k)` with `k` the plane's place in the list: an inserted plane would give its neighbours' names to other faces without a failure, the fault found in a blend's `chamfer(k)` on 2026-11-13. Copies of a hidden tool (`F9:copy(1)/…`): there is no tool feature to name.

### Method

**The kept side is the boolean intersection of the body with a block the feature builds itself, plane by plane.** This is what `measure.below` has done since 2026-10-26 (`kernel::below::part_one`, [`sections-and-levels.md`](sections-and-levels.md) › *Below the plane, exactly*), made a function of its own and given names:

1. `kernel::halfspace::keep_below(body, plane, tolerance, options)`, a pure function. `plane` is a frame: its origin on the plane, its z the normal, pointing to the side cut away (the caller turns it for `side: above`).
2. *By extent first.* The body's extent along the normal (`below::body_extent`: `Body::held_bbox` of the body turned so that the normal is z, tight for analytic faces, a hull for freeform ones, never too small). Wholly below the plane by more than twice the tolerance: the body as it is, no boolean. Wholly above by more than that: `EmptyResult`. Nearer than that the boolean decides, so every rule of touch and tolerance is the boolean's own, in one place.
3. *The tool.* With `c` the centre of the body's box (`Body::bbox`, the box the booleans measure their own margins against), `r` half its diagonal and `m = r/4 + 32 tolerances`: a block in the plane's frame whose top face is the plane itself (its surface the frame given, not one worked out from a far corner, so two cuts by one datum have the same plane bit for bit and a sketch on the cut face has the datum's frame), reaching `r + m` to every side of `c` dropped onto the plane, and down to `r + m` below `c`. (`below.rs` takes `r/4 + 10 tolerances` today; 32 is twice the boolean's reach, below.)
4. `boolean_with(body, tool, Intersect, …)`: the ordinary boolean, with its own check of the result.
5. *Sources.* Every face of the result is a piece of a face of the body (`Kept(face)`) or of the tool's top (`Cap`); a face with a piece of any other face of the tool means the block did not hold the body and is an internal error, named as a bug, never a body (the check `below.rs` already makes). Edges and vertices are reported kept or new as the boolean reports them.

Regeneration (`regen/src/plane_cut.rs`) resolves each plane (`datum::plane_ref`, or the six expressions), applies `offset` and `side`, calls `keep_below` on what the plane before left, carries the names along (a face of the running body keeps its name; a cap is `cut(id)`), composes the kept edges and vertices back to the target's, names the entities once at the end (`Names::name_entities`), and checks the final body as a boolean's result is checked (`not_through_itself`). A failure says which plane: `planes.6 (plane 7): …`, the parameter first as every reason starts.

**Why the tool is safe at every scale, and never sized by the person.**

- *It holds the body.* Every point of the body is within `r` of `c`; the block reaches `r + m` every way but up, where it ends on the plane.
- *Its far faces take no part.* Each is at least `m` from every point of the body. The boolean's rules read at most 16 tolerances (`section::APART`, `section::EDGE_REACH`); `m` is twice that at the least and an eighth of the body's diagonal besides, so every pair of a far face with a face of the body is dropped at the first box test, at the cost of that test (five faces in a tree of six).
- *No length of its own* (rule 5): `r` is the body's, the tolerance the document's. A body of 1e-3 and one of 1e6 get the same tool, scaled.
- *No digits lost.* The tool's corners lie within `√3 (r + m)`, about `2.2 r`, of the body's centre, so they are numbers of the body's own size and round as the body's vertices do. A rectangle sized "large enough" by hand, a million times the body, puts its corners where a unit in the last place is a million times coarser: the feature never does that.
- *Checked, not trusted:* step 5. The body's box is itself not a proof in one place (whether a cone's face surrounds its apex is read at sampled points, `Body::bbox`), which is why a far face in the result is looked for and not assumed away.

**The alternatives, and what each costs to build and to trust.**

| Method | Build | Trust | Verdict |
|---|---|---|---|
| The boolean with a block from the body's bounds | A kernel function of some fifty lines beside `below.rs`'s, naming, the feature | The most travelled path there is: solid against solid, the corpus's 11,988 outcomes, `below.rs`'s suite at three scales with freeform bodies. Every contact case tried in 0.2.1 answered right (measured below) | **Chosen** |
| The cut by a sheet that reaches across (`boolean/cut.rs`) with a rectangle the feature makes | The same naming, a rectangle instead of a block | Refuses what the block answers: a plane that only touches the body, or misses it, is "the sheet does not pass through the solid" although the answer is the whole body (measured: a plane on an edge, at a vertex, tangent to a cylinder and to a ball, above the body; and the 23rd of 24 planes through one shared point, which touches the body only there); and a body of several lumps, or with a cavity, the sheet does not cross is refused (`NOT_CROSSED`, owed). Each would need a rule of its own on top | Rejected: more cases to add than it saves |
| A true half-space operand: the plane unbounded, the body's regions told by the sign of their distance to it, the cap from the section's loops alone, no rays | A new operand kind through section, regions, classify, sew and the boolean's own check | Less to go wrong in principle (no far faces, no ray that grazes), none of it travelled yet | Not now. It is the general method to move to **if** a measurement shows the block costs (speed below); what it would save per cut is the rays against six faces, which are cheap |
| One convex tool (the block cut by all planes first), then one boolean | The tool by the same function on a six-faced block, then one boolean | Planes that nearly meet *outside* the body but inside its box would refuse a cut whose result has no such place | Not first; a candidate for speed on bodies of thousands of faces (below) |

**The contact cases** (what the 0.2.1 kernel answers through a box tool is measured below; the feature adds nothing to it):

| The plane | Result |
|---|---|
| through an existing vertex, along an existing edge, or both, and across the body | built: the vertex or the edge is shared, nothing is matched by position |
| on a face of the body, the body behind it (`below`) | the body unchanged, plane `k` listed as cutting nothing |
| on a face of the body and across more of it (a stepped block at its shoulder) | built: the piece cut and the face are one face, the face's name with `cut(k)` as an alias |
| on a face of the body, the body in front of it; or wholly above it | fails: plane `k` leaves nothing of the body |
| touching the body from outside along an edge, at a vertex, or tangent to a cylinder, a cone, a ball or a torus | the body unchanged, plane `k` listed as cutting nothing; with the other side, leaves nothing |
| missing the body | as touching |
| through a cone's apex or a ball's pole, along a cylinder's line or its seam | built ([`booleans.md`](booleans.md), the table of contact) |
| tangent to a face *inside* it and through the body (a plane on the inner equator of a torus, on a saddle) | built where the surfaces' meeting has a closed form or is a tacnode the marcher handles; otherwise the boolean's refusal with its reason |
| between half and twice the tolerance from a face it is over, or leaving a piece thinner than about eight tolerances | refused, "at the edge of tolerance", with the plane and the faces by name |
| leaving parts that touch along an edge or in a point (a plane through the waist of an hourglass) | refused as zero thickness, with the place |
| leaving several lumps | one body of several shells, as after any boolean; `body.split` gives a body per lump |

**What is refused rather than built.** A plane that leaves nothing of the body (a feature has a body or fails; the reason names the plane and says which side the body lies on). A reference that is not a plane (a face that is not flat, a datum that is not a plane), a normal of no length, a point or an offset that is no length. A target that is a sheet, in the first slice: the same boolean trims a sheet by a solid today, so it is a matter of naming the new boundary edges, owed as slice 2. A zero-thickness or edge-of-tolerance state after *any* plane of the list, also one a later plane would cut away: each step is a whole boolean with its refusals on, because the boolean finds a touch from where its operands meet (the section's touch points and the places an edge meets the other body's surface), which regeneration's own look at the finished body, by its vertices and edges, does not have ([`booleans.md`](booleans.md) › *Zero thickness*); the reason names the plane and says that a plane further down the list that removes the place can be put first. More than 256 planes. Work past the bound of one operation, per plane (`OPERATION_WORK`).

**A plane that cuts nothing is not an error.** The body is passed to the next plane as it is; the result says so per plane and under `warnings`. A subtraction whose tool misses its target is no error today either, and a Jig whose chamfer plane clears the part at some sizes has to keep building. A reference to that plane's face fails by name.

**Planes from measured data.** Planes that should meet in one point and, as measured, meet in several a millimetre apart are cut as they are: the small facets and the short edges between them are in the body, exactly, and in every mesh of it. **This feature does not snap, merge or move anything**; merging what lies within a given distance is item 4 of the boolean checklist ([`../roadmap.md`](../roadmap.md)) and belongs to the boolean, where every operation gets it. What the person sees: at a tolerance far below the scatter, a body with the extra facets (the issue counts 44 to 86 triangles where 26 were meant); where what the scatter leaves falls under the boolean's rules of tolerance (points or faces between half and twice the tolerance apart; a piece of a face within the tolerance of a face it is not on, which neighbouring planes that are nearly one plane give over a strip as wide as the tolerance divided by the angle between them), a failed feature whose reason names the plane and the faces and says what the tolerance is (measured below at a tolerance of 1e-6 mm: 24 planes round a cone scattered by a millionth and by a ten-thousandth of a millimetre and of three degrees are refused, by a hundredth and more built). One thing the caller can do today, and the summary will say: planes meant to meet in one point are given through that point, the same numbers for each (`point` shared, the normals as measured). They then meet there to rounding, far within half the tolerance, and are one vertex (measured below: 24 planes through one point with normals scattered by degrees are built; ten of them touch the body only at that point and cut nothing).

### Cost and bounds

*The issue's case* (a prism of 70 outline vertices, 24 planes; measured the five-feature way in the released 0.2.1, below):

| | Five features per plane (measured) | One feature (expected) |
|---|---|---|
| History | 98 entries (4 per plane; a rectangle is a command, not an entry) | 3: the sketch, the prism, the cut |
| Built command by command | 147 to 170 ms | the prism, then one command |
| Per plane | 3.4 to 4.1 ms for the extrusion and its boolean, 0.7 ms for the datum plane, the sketch and the rectangle | at most the boolean's share, about 3.3 ms: no sketch solved, no profile found, no tool extruded, names made once instead of 24 times |
| The whole cut | 85 to 100 ms | 60 to 80 ms; to be measured in slice 1, counted in the benchmark |
| One plane's number changed | 33 to 40 ms for the plane in the middle: the 12 cuts after it | the whole feature, 60 to 80 ms: every plane again |

So the feature makes 96 history entries one and a build somewhat faster, and makes an edit of one plane dearer than today for a plane late in the list: regeneration caches per feature, and one feature is rebuilt whole. Nothing before it is rebuilt, and what follows it only when its body changed. That is the honest price of one entry; slice 3 removes it if a measurement asks for it (the bodies after each plane kept with the feature's cache entry, so a rebuild starts at the first plane whose inputs changed).

*The bound.* At most 256 planes, each one boolean held to the bound on one operation's work, refused past it; so the feature cannot run on. Each cut costs the faces the plane crosses (their sections) plus a share for every face of the body (the operand's check, answered from memory for a body the kernel just returned; one ray against six faces per face not crossed; sewing; the result's check), and the body gains at most a face per plane and lump, so `n` planes on `F` faces cost about `n (F + n)` of the second kind. Measured in 0.2.1 on a plate of 906 faces: 62 ms for a plane across one corner (the share per face: 0.07 ms), 264 ms for a level plane through 900 holes, 2.0 s for a tilted one (an ellipse per hole, 2.3 ms each: the boolean's own speed, not this feature's). Many planes on a body of thousands of faces is where the convex tool, or the half-space operand, would pay: 25 planes there cost 25 times the share, 1.5 s on that plate, where one boolean would cost it once.

### Oracles

- **Closed forms** (oracle 1), each at 1e-3, 1 and 1e6: a block with a corner cut off by a plane through three chosen points (volume `V − abc/6`, the area by the triangle and the three right triangles it replaces, the centroid from the tetrahedron's); by a plane through three of its vertices; by the plane through two opposite edges; a cylinder cut along its axis, across it at a distance (the segment), tilted (`π r²` times the mean height); a ball's cap (`π h² (3r − h) / 3`); a cone cut level (a frustum); a torus halved by the plane of its ring and by a plane through its axis (two lumps, one body). The roof by two planes (`19200` for a 40 × 30 block with eaves at 12 and a ridge at 20).
- **Membership** (independent of the boolean): 400 to 4000 seeded points; a point is in the result exactly when it is in the target (`contains_points`) and on the kept side of every plane, which is a dot product.
- **Identities** (oracle 3): the two sides of one plane have volumes that add up to the target's, first moments that add up, and areas that add up to the target's plus twice the section's, where the section's area is `measure.section`'s (Green's theorem along the edges, another integrator); the planes in any order give the same volume, area, centroid and the same set of names; a plane given twice changes nothing; `{plane: P, offset: d}` is the datum plane at `d` from `P`; `n` planes in one feature are, bit for bit, `n` features of one plane each; the same cut done the five-feature way has the same volume, area, centroid, counts of faces, edges and vertices and the same seeded points inside.
- **Analytic and freeform**: every identity above on a block with a spline top, a lofted body and a rational patch that is a piece of a cylinder, the last against its analytic twin; `measure.below` of the same plane gives the same volume (it shares the tool, so this holds the plumbing, not the geometry).
- **Validity**: `topo::check`, the genus and the shells of every result, a watertight mesh (oracle 4), no body through itself (oracle 9), the boolean's full check (oracle 10).
- **The contact table above**, row by row, at three scales: what is built to its closed form, what is refused to its reason.
- **Determinism and round trips** (oracle 5): incremental against full regeneration after every command; undo and redo; save, reopen, the same bodies bit for bit; the format fixture of the new version; one thread and many.
- **The naming suite extended** (`crates/engine/tests/plane_cut_names.rs`, with `naming_entities.rs`'s property that no two edges and no two vertices of a body share a name): a fillet on the edge between a cut face and a wall, a sketch with a boss on a cut face, a datum on one, an edge by its own name, a mate's face; then every row of the table under *Names*, the entity located by closed form after each edit, a failure held to its text; `identity.rs`'s list with `cut(n)`.
- **Never a crash**: `panic_hunt` picks the command up from its params string; besides it, no planes, 257 planes, an id twice, id 0 and 2³², a normal of zero, NaN and 1e308, a point at 1e300, a curved face as a plane, a target that is a sketch, a datum, a sheet, the feature itself, a plane by a face of a later feature.
- **Outside** (oracle 8): the STEP of the issue's case and of a cylinder's segment read by FreeCAD, its volume against ours (`step_freecad.rs`).
- **Work, not time**: the issue's case as a benchmark model (built, then one plane's variable changed), its counts in `baseline.txt`; the work per plane the same at 8 and at 32 planes.
- **A picture looked at** (rule 10): the roof, and a cutaway of a showcase model.

### Slices

1. **The feature on solids.** `feature.planeCut` as specified above with `keep`; planes by base plane, datum plane, flat face and `{point, normal}`, with `offset`, `side` and `id`; the names; `feature.edit`; the format's new version with its fixture and a migration that changes nothing (a new feature kind: an older build says a newer 3DCraft is needed); `kernel::halfspace` (and `below.rs` on it only if its suite stays bit for bit; otherwise it keeps its own block and this document says so); the oracles above; the guide's chapter and reference entry with the issue's case as a checked example; this document's Method as built, [`names.md`](names.md), [`kernel.md`](kernel.md), [`file-formats.md`](../file-formats.md), the roadmap's item moved to *Done* with what is left listed under *Open*, the release notes. Useful by itself: it is everything the issue asked for under its item 3. A sheet as the target is refused and listed as owed.
2. **Breadth.** A sheet trimmed by planes (the new boundary edges named by the plane's id); a condition per plane (`when` in a plane's object: left out, it renumbers nothing, since names go by id); one plane changed by its id without the list (`feature.edit {plane: {id, …}}`); the fuzzer's step (random planes through points of the body's box, oracle 11); a kind in the action matrix (each flat face's plane moved in by a tenth: the volume falls and is `measure.below`'s).
3. **Speed, after measuring.** The bodies after each plane kept with the cache entry, so that one changed plane redoes the planes from it on; for many planes on a body of thousands of faces, the convex tool or the half-space operand, whichever the benchmark and the corpus support.

Not this feature's: merging within a distance (the boolean checklist, item 4); a prism command, face provenance per triangle, machine-readable failure reasons (noted in the decision of 2026-11-20, not decided).

### Open questions

Each with a recommendation; the first three are the developer's or the coordinator's word on names that are stable once released.

1. **The id.** `feature.planeCut` (recommended), `feature.cut` or `feature.trim`: the table under *The command*.
2. **The cut face's name.** `F9:cut(3)`, a new role (recommended), or `F9:face(3)`, no new role.
3. **The side's words.** `side: below | above` with `measure.below`'s meaning (recommended); the issue wrote `op: keepBelow | keepAbove` for the whole feature.
4. **A plane that cuts nothing** is listed and warned about, not an error (recommended); a plane that leaves nothing fails the feature.
5. **Refusals per plane, in list order** (recommended, with the reason naming the plane): a state a later plane would have removed can refuse the feature. The other way, refusing only what the final body has, needs regeneration's check to see every touch the boolean sees.
6. **One changed plane rebuilds the whole feature** in slice 1 (recommended); slice 3 if it is felt.
7. **Coefficients as a form of their own**: no (recommended); a sentence in the summary.
8. **Roofs that are not convex** (the issue names an L-shaped hip and crossing gables) are not one intersection of half-spaces. With this feature: a cut of a kept copy per convex part, united (`keep`, then `feature.boolean`). Recommended: build the intersection, which every other combination is made of, and ask the issue's author how each plane's tool is bounded in those cases today before adding anything.
9. **The cap of 256 planes**: recommended; the issue's largest case has 25.

## Measured (2026-11-20)

All with the released `craft3d-cli` 0.2.1 (f6d88a043f5c) on the developer's laptop at a load of 1 to 1.6, one process at a time, capped; times are the server's own `ms` per command and **indicative only**. No run on the build server: nothing here needed one. The scripts are not in the repository (generated; their parameters are given).

**Contact cases, the five-feature way (an extruded box cut away) and the sheet way (a filled rectangle as the boolean's tool), both sides of each plane:** a 40 × 30 × 20 block cut at a corner, through three vertices, through two opposite edges, through a vertex and across; touched along an edge and at a vertex; with the plane on its top face, above it, and 0.4, 1 and 3 tolerances under it; a cylinder (r 10, h 30) with a tangent plane, a plane across, through its axis and tilted; a ball with a tangent plane, through its centre, and a cap. 72 results. Every body built has its closed-form volume to 1e-15 relative (the cut 3 tolerances under the top to 1.6e-10). The box: every case answered as the table above says, "leaves nothing" where nothing is left, and one refusal, both sides of the plane 1 tolerance under the top ("two faces between half and twice the tolerance apart"). The sheet: the same where the plane passes through the body or lies on a face; refused, for both sides, wherever the plane only touches or misses it (on an edge, at a vertex, above the body, tangent to the cylinder and to the ball: "the sheet does not pass through the solid"), where the box way leaves the whole body.

**The issue's case:** an outline of 70 vertices (a radius of 20 with a five-lobed wave of 3 and a scatter of 1), extruded 30; 24 planes round a cone of about 30°, each through a point at radius 10 and height 18, scattered by `j` times (0.02 rad in azimuth, 3° in tilt, 1 in height); document in mm, tolerance 1e-6.

| Variant | Result | Extrusion and boolean per plane | All commands | The plane in the middle changed |
|---|---|---|---|---|
| `j` = 0.3, box | 94 faces, check ok | 4.1 ms | 170 ms | 38 ms |
| `j` = 0.3, sheet | the same volume, area and counts | 3.5 ms (the boolean) + 0.2 (the fill) | 154 ms | 35 ms |
| `j` = 1, box | 89 faces (6 planes cut nothing) | 3.5 ms | 147 ms | 35 ms |
| `j` = 0 (the planes meet in one point to rounding) | 95 faces, check ok | 3.5 ms | 151 ms | 37 ms |
| `j` = 1e-2 | 95 faces, check ok | 3.6 ms | 158 ms | 40 ms |
| `j` = 1e-4 | refused at plane 21, "a piece of a face lies within tolerance of a face it is not coincident with (at the edge of tolerance)" | | | |
| `j` = 1e-6 | refused at plane 6, the same reason | | | |
| `j` = 0.3, every length × 1000 | as at 1, the volume × 1e9 to 15 digits | 3.4 ms | 149 ms | 38 ms |
| `j` = 0.3, every length × 0.001 (a part of 0.04 mm at a tolerance of 1e-6 mm) | refused at plane 15, the same reason; built after the middle plane was moved | | | |
| every plane through the point (0.3, −0.2, 22), the normals scattered with `j` = 1, box | 85 faces, check ok: 10 planes cut nothing | 3.6 ms | 149 ms | 33 ms |
| the same, sheet | refused at plane 23: "the sheet does not pass through the solid" | | | |

The history is 98 features in every variant (the outline's sketch, the prism, and per plane a datum plane, a sketch, an extrusion, or a fill for the sheet way, and a boolean).

**A body of many faces:** a plate with `n × n` holes (a block less a grid of cylinders), then three planes the five-feature way.

| | 100 holes (106 faces) | 900 holes (906 faces) |
|---|---|---|
| a tilted plane across every hole | 225 ms | 2038 ms |
| an upright plane across one corner | 8 ms | 62 ms |
| a level plane across every hole | 35 ms | 264 ms |
