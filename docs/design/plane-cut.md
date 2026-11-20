# 3DCraft design: A body cut by planes

A body cut by one or more planes in one feature, one side of each kept (`feature.planeCut`): a corner cut off a block, a roof, a cutaway of a finished model, a part trimmed to a datum. Before it each plane cost a datum plane, a sketch, a rectangle, an extrusion and a boolean, and the person had to size the rectangle and the extrusion "large enough"; a prism trimmed by 24 planes carried a history of 98 entries. Asked in [issue 3](https://github.com/johancoppens/3dcraft/issues/3) of the public repository by the first person to build on 3DCraft as a library; decided on 2026-11-20 ([`../decisions.md`](../decisions.md)); roadmap: [`../roadmap.md`](../roadmap.md) › *M7*. Code: `crates/kernel/src/halfspace.rs` (the cut), `crates/doc/src/plane_cut.rs` (the feature as data), `crates/regen/src/plane_cut.rs` (regeneration and names), `crates/engine/src/plane_cut_cmds.rs` (the command). Guide: [`../guide/features.md`](../guide/features.md#cut-by-planes).

> **Built: the first slice (2026-11-20), the feature on solids.** Everything under *Method* is built and held to the tests named under *Oracles*. What is not built is under *Not built yet*, marked *(planned)*.

## Method

### The command

```text
feature.planeCut
  params:  {"target": id or name, "planes": [plane, ...] where plane is "xy|yz|zx" or datum plane (id or name) or a flat face by name, or {plane: one of those, offset: expr = 0, side: below|above = below, id: number = a new one}, or {point: [expr, expr, expr] = [0, 0, 0], normal: [expr, expr, expr], offset: expr = 0, side: below|above = below, id: number = a new one}, "keep": boolean = false, "name": string = automatic, "when": expr = always on}
```

- **`target`**: the feature whose body is cut, as `feature.boolean` takes its target. The body goes into this feature; with `keep` the target keeps its body and this feature is a cut copy (as `feature.transform` and `feature.mirror` keep theirs), which is how both sides of one cut, or two different cuts of one body, are had.
- **`planes`**: 1 to 256 planes (`doc::MAX_CUT_PLANES`); one plane may be given alone instead of a list. Each is one of:
  - a base plane (`"xy"`, `"yz"`, `"zx"`), a datum plane by id or name, or a flat face of an earlier body by its persistent name, exactly as `datum.plane`'s `from`, `feature.draft`'s `neutral` and `measure.below`'s `plane` read them (`datum_cmds::plane_value`, stored as `doc::PlaneRef`); the cut follows the datum or the face. A face named as an earlier body had it is that face as it was there (the rule for every reference that does not take the face's body, [`names.md`](names.md) › *References that follow a body*), so a body can be cut by the plane of one of its own faces moved by `offset`;
  - `{point, normal}`: the plane through `point` (three lengths; the origin when left out) square to `normal` (three plain numbers or three lengths, any length: only where it points counts, as `feature.block`'s `axis`). All are expressions, kept as source text, so they follow variables and a Jig's parameters: a plane given through variables is changed with `variable.set`, without sending the list again. A plane known by its coefficients `a x + b y + c z = d` is `normal: [a, b, c]`, `point: [a d / s, b d / s, c d / s]` with `s = a² + b² + c²`; the summary says so.
  - Either form may carry `offset` (a length expression: the plane moved along its normal, as `sketch.create` and `measure.below` have it), `side` and `id`.
- **`side`**, per plane: `below` (the default) keeps what lies on the side the normal points away from, `above` what lies on the side it points to. The words and their meaning are `measure.below`'s. A base plane's normal is its third axis (xy: +z), a datum plane's its own, a face's points out of the body the face is of (so `below` keeps what lies behind the face), a `{point, normal}` plane's is the one given.
- **Several planes mean the intersection of the kept half-spaces**: the result is every point of the target that is on the kept side of every plane. The planes are applied in the order given; the point set does not depend on the order, the names do not either (below), and only which refusal is met first can.
- **`id`**: every plane of a feature has an id of its own, a number from 1, which names the face it cuts (below). Without `id` a plane gets the next free one, in the order of the list, above every id given; an id is never given out twice in one feature, also not after its plane was deleted (the feature keeps the next free id, `PlaneCut::next_plane`, as a sketch keeps `next_entity_id`).
- **`when`** and **`name`** as for every feature. Switched off, the feature passes its target's body on, named as when it is built (`F9:copy(0)/…`), the rule for every feature that changes a body ([`regeneration.md`](regeneration.md) › *Switched off*); with `keep` it passes nothing on.
- **`feature.edit`** changes `target`, `planes` and `keep`. `planes` is the whole list, by the rule `sheetMetal.edit` has for flanges: a plane given with its id is the feature's plane of that id (an id the feature has no plane of, or one given twice, is refused); one given without is the plane it equals in every field (the first not taken yet), and else a new plane with a new id. So a plane inserted in the middle, one deleted, and a list in another order leave every other plane its id.
- **The result**, and `document.inspect` per feature, has `planes`: per plane, in the order of the list, its `id`, `faces` (how many faces of the cut body are pieces of it, by its name or as an alias) and `cuts` (whether there is one). That is a property of the cut body, the same whatever the order of the list: a plane has no face when it does not meet the body, lies in a face of it with the body behind, or when what it cut was cut away by the other planes. The command's result lists each such plane under `warnings` too.

Rejected:

| Alternative | Why not |
|---|---|
| `feature.cut` | "Cut" already means two other things here: `feature.extrude`'s `cut` (an extrusion taken away) and the `cut.*` family (parts cut from sheet stock); and in FreeCAD, whose users will read it so, *Cut* is the boolean difference. A command of that name would be expected to take a tool. |
| `feature.trim` (the issue's word) | `feature.surface.trim` bounds a face's surface by curves, and "a sheet trimmed by a solid" is the boolean's own term ([`booleans.md`](booleans.md) › *Sheets*); a later trim of a solid by a surface would want the word. |
| `body.split` with a plane | `body.split` makes one feature per lump of what is already apart; it does not cut. |
| One plane per feature | 24 history entries and 24 kept bodies where one is asked for; it stays possible (a list of one). |
| `op: keepBelow \| keepAbove` for the whole feature | A cut by a face and by a datum at once needs a side per plane; and `feature.edit`'s `keep` is already a boolean (a mirror's), so the side cannot be called `keep`. |
| Coefficients `[a, b, c, d]` as a third form | `d` is a length only when the normal is a unit vector, which measured coefficients need not be; the conversion is two lines for the caller and one sentence in the summary. |
| "Any" instead of "all" (keep what is below at least one plane) | A union of half-spaces is the target less a convex wedge; roofs that are not convex (an L-shaped hip, crossing gables) are unions of *intersections*, which neither form gives alone: cut a kept copy per convex part and unite them with `feature.boolean`. Whether more is wanted is a question to the issue's author (the coordinator's, 2026-11-20). |

### Names

Three kinds, nothing new in how names are followed or resolved ([`names.md`](names.md)):

- **The face a plane leaves: `F9:cut(3)`**, the face plane 3 of feature F9 cut. A role of its own, `cut` (`doc::NameRole::Cut`), a face, with one plain number: the plane's id. Where a plane cuts the body in several places (the two legs of a slotted block) the pieces share the name, by the rule for a face in pieces: accepted where one surface is meant (a sketch's plane, a datum, a pair of faces for a fillet or a chamfer, which takes every edge between them), and told apart per lump by `body.split`. Its frame, for a sketch on it, is the plane's own, bit for bit (a datum plane's origin and axes, a face's, a base plane's; for `{point, normal}` the origin is the point and x the world axis least along the normal made square to it, `Frame::from_z`, `measure.below`'s default), moved by `offset`; with `side: above` seen from the other side (x stays, y and z turn round, `Frame::reversed_at`). The normal always points out of the kept body. For `{point, normal}` the x axis is a choice among the world's axes, so it jumps when the normal passes a diagonal; a sketch that must not turn is put on a datum plane, or given `sketch.create`'s `xAxis`.
- **What is left of the target's faces: copies in instance 0**, `F9:copy(0)/F2:side(e71)`, as after every feature that takes a body; a face cut away is not among the names.
- **Edges and vertices: by their faces**, as after a boolean (`Names::name_entities`): `F9:between[F9:copy(0)/F2:side(e71),F9:cut(3)]` where a wall meets a cut face, `F9:between[F9:cut(3),F9:cut(4)]` a ridge between two roof planes, `F9:corner[F9:cut(3),F9:cut(4),F9:cut(5)]` an apex; an edge or a vertex the target already had, kept whole, also answers to the copy of its old name (`F9:copy(0)/F1:edge(4)`), and the kernel's report of kept edges, carried back through the planes to the target's, is stored so that an edge's own name is followed through this feature (`follow::edge`).

*A plane in a face of the target.* Where a piece the plane cuts joins a face of the target that lies in the plane and faces the same way (a stepped block cut at its shoulder), the two are one face, as after any boolean: named after the target's face, the copy, and answering to `F9:cut(k)` as an alias (the kernel lists both under `BooleanResult::merged`); a sketch and a datum find it by either name (`sketch::made_face` looks among the aliases too). Where the plane cuts nothing at all (in the top of a block, the block below it; a plane given twice) there is no `cut(k)`: the target's face keeps its copy name and the plane is reported as cutting nothing. So a reference to `cut(k)` holds while the plane cuts something and fails by name when it no longer does, and never moves to another face. *(planned, slice 2)*: whether the target's face in the plane should answer to `cut(k)` there too.

*What an edit does* (each a scenario of `crates/engine/tests/plane_cut_names.rs`):

| Edit | Names |
|---|---|
| a plane moved or turned, the topology the same | every name stays on its entity |
| a plane added, in the middle of the list or at its end | every other plane keeps its id, so every name stays; the new plane's face is `cut(new id)`; an edge the new plane cuts in two loses its bare name (two longer ones appear), as after any cut |
| a plane deleted | references to its face and to edges on it fail with the name; its id is not given out again, so nothing else can take the name |
| the list given in another order | nothing changes |
| a plane no longer meets the body | `cut(id)` is gone: what refers to it fails naming it; moving the plane back repairs it |
| a plane now cuts a referenced face away | the reference fails with the name, never lands on another face |
| the target edited upstream (a size changed) | cut faces keep `cut(id)`; copies follow their faces as through any taker |
| `target` pointed at another body | cut faces keep `cut(id)`; the copies are of the new target's names, and a reference to a face of the old one fails by name |

*A reference to a face of the target follows into the result* as through any feature that takes a body: a blend or a face operation after the cut that names `F1:face(0)` is followed to `F9:copy(0)/F1:face(0)` (`follow::face` looks for the copy in instance 0); gone fails with the name and the faces that are there.

*Proven identity.* `cut(n)` is on the list of kinds proven to be identity (`doc::identity`, [`names.md`](names.md) › *Which parts of a name are proven to be identity*): the id is stored with the plane and never reused, the argument a flange's id has. So `F9:between[F9:cut(3),F9:cut(4)]` is an edge name a fillet takes by itself. A `between[…](n)` among several edges on the same faces stays unproven, as everywhere.

Rejected: `F9:face(n)` with the plane's id (no new role, as sheet metal numbers its faces): `face(n)` on a block means a fixed side, and a name that reads `F9:face(3)` beside `F9:copy(0)/F1:face(3)` says less than `F9:cut(3)` in every error that lists names. `F9:cut(k)` with `k` the plane's place in the list: an inserted plane would give its neighbours' names to other faces without a failure, the fault found in a blend's `chamfer(k)` on 2026-11-13. Copies of a hidden tool (`F9:copy(1)/…`): there is no tool feature to name.

### The cut in the kernel

**The kept side is the boolean intersection of the body with a block the kernel makes from the body's own box, plane by plane** (`kernel::halfspace`). It is the cut `measure.below` has made since 2026-10-26 to measure what lies below a plane (`kernel::below::part_one`, [`sections-and-levels.md`](sections-and-levels.md) › *Below the plane, exactly*), as a function that returns the body with the source of every entity:

1. `halfspace::keep_below(body, plane, tolerance, options)`, a pure function. `plane` is a frame: its origin on the plane, its z the normal, pointing to the side cut away (regeneration turns it for `side: above`).
2. *By the box first* (`halfspace::tool`). The eight corners of the body's box (`Body::bbox`, which holds the body and is the box the booleans measure their own margins against) are measured against the plane. All below it by more than twice the tolerance: the body as it is, no boolean (`Kept::Whole`). All above by more than that: `EmptyResult`. Nearer than that the boolean decides, so every rule of touch and tolerance is the boolean's own, in one place.
3. *The block.* With `c` the centre of that box, `r` half its diagonal and `m = r/4 + 32 tolerances`: a block in the plane's frame whose top face lies in the plane, reaching `r + m` to every side of `c` dropped onto the plane, and down to `r + m` below `c`. The top face's surface is the frame given, bit for bit, not one worked out from the block's corners: two cuts by one datum have one surface, and what is placed on a cut face has the plane's own origin and axes.
4. `boolean_with(body, block, Intersect, …)`: the ordinary boolean, with its own check of the result.
5. *Sources.* Every face of the result is a piece of a face of the body (operand A) or of the block's top (operand B, `halfspace::CAP`); a face with a piece of any other face of the block means the block did not hold the body and is refused in the words of a bug, never returned as a body (the check `below.rs` makes of its own block). When no face has a piece of the top, the plane cut nothing and the body itself is the answer (`Kept::Whole`), not a copy the boolean made of it.

Regeneration (`regen/src/plane_cut.rs`) resolves each plane (`datum::plane_ref`, or the expressions: `derive::lengths`, `derive::direction`), applies `offset` and `side`, calls `keep_below` on what the plane before left, and carries along, per face, its name and the ids of the planes it has a piece of, and per edge and vertex the entity of the target it is. At the end the entities are named once (`Names::name_entities`), and the body is looked at for places where it passes through itself near the cut faces, as a boolean's result is (`selfcheck::crossing_near`, the cut faces as new). A failure says which plane, the parameter first as every reason starts: `planes.6: plane 7 leaves nothing: …`.

**Why the block is safe at every scale, and never sized by the person.**

- *It holds the body.* Every point of the body is within `r` of `c`; the block reaches `r + m` every way but up, where it ends on the plane.
- *Its far faces take no part.* Each is at least `m` from every point of the body. The boolean's rules read at most 16 tolerances (`section::APART`, `section::EDGE_REACH`); `m` is twice that at the least and an eighth of the body's diagonal besides.
- *No length of its own* (rule 5): `r` is the body's, the tolerance the document's. A body of 1e-3 and one of 1e6 get the same block, scaled (`halfspace::tests`, at three scales).
- *No digits lost.* The block's corners lie within `√3 (r + m)`, about `2.2 r`, of the body's centre, so they are numbers of the body's own size and round as the body's vertices do. A rectangle sized "large enough" by hand, a million times the body, puts its corners where a unit in the last place is a million times coarser.
- *Checked, not trusted:* step 5. The body's box is itself not a proof in one place (whether a cone's face surrounds its apex is read at sampled points, `Body::bbox`), which is why a far face in the result is looked for and not assumed away.

**`kernel::below` keeps its own block.** The design proposed moving `measure.below` onto `halfspace::keep_below` if its suite stayed bit for bit. The two blocks differ where it shows in the last bits (the top's surface is the frame given here and a frame worked out from the block's corners there; the margin is 32 tolerances here and 10 there), so `below.rs` is as it was and every body `measure.below` cuts is the body it cut before; one construction for both is left for a round that can compare every bit of that suite.

**The alternatives, and what each costs to build and to trust.**

| Method | Build | Trust | Verdict |
|---|---|---|---|
| The boolean with a block from the body's box | A kernel function of a hundred lines, naming, the feature | The most travelled path there is: solid against solid, the corpus's 11,988 outcomes, `below.rs`'s suite at three scales with freeform bodies | **Built** |
| The cut by a sheet that reaches across (`boolean/cut.rs`) with a rectangle the feature makes | The same naming, a rectangle instead of a block | Refuses what the block answers: a plane that only touches the body, or misses it, is "the sheet does not pass through the solid" although the answer is the whole body (measured: a plane on an edge, at a vertex, tangent to a cylinder and to a ball, above the body; and the 23rd of 24 planes through one shared point, which touches the body only there); and a body of several lumps, or with a cavity, the sheet does not cross is refused (`NOT_CROSSED`, owed). Each would need a rule of its own on top | Rejected: more cases to add than it saves |
| A true half-space operand: the plane unbounded, the body's regions told by the sign of their distance to it, the cap from the section's loops alone, no rays | A new operand kind through section, regions, classify, sew and the boolean's own check | Less to go wrong in principle (no far faces, no ray that grazes), none of it travelled yet | *(planned, slice 3, if measured)*: the general method to move to if the block costs; what it would save per cut is the rays against six faces, which are cheap |
| One convex tool (the block cut by all planes first), then one boolean | The tool by the same function on a six-faced block, then one boolean | Planes that nearly meet *outside* the body but inside its box would refuse a cut whose result has no such place | *(planned, slice 3, if measured)*: a candidate for speed on bodies of thousands of faces |

**The contact cases** (each a case of `crates/kernel/tests/halfspace.rs` at 1e-3, 1 and 1e6, or of `crates/engine/tests/plane_cut.rs`):

| The plane | Result |
|---|---|
| through an existing vertex, along an existing edge, or both, and across the body | built: the vertex or the edge is shared, nothing is matched by position |
| in a face of the body, the body behind it (`below`); also within half a tolerance of it | the body unchanged, plane `k` reported as cutting nothing |
| in a face of the body and across more of it (a stepped block at its shoulder) | built: the piece cut and the face are one face, the face's name with `cut(k)` as an alias |
| in a face of the body, the body in front of it; or wholly above it | fails: plane `k` leaves nothing |
| touching the body from outside along an edge, at a vertex, or tangent to a cylinder or a ball | the body unchanged, plane `k` reported as cutting nothing; with the other side, leaves nothing |
| missing the body | as touching |
| through a cylinder's axis, across a ring's hole, through the plane a ring lies in | built, to the closed form |
| tangent to a face *inside* it and through the body (a plane on the inner equator of a torus, on a saddle) | built where the surfaces' meeting has a closed form or is a tacnode the marcher handles; otherwise the boolean's refusal with its reason (not among the tests) |
| between half and twice the tolerance from a face it is over, or leaving a piece thinner than about eight tolerances | refused, "at the edge of tolerance", with the plane and the faces by name; three tolerances under a face it cuts a skin |
| leaving parts that touch along an edge or in a point | refused as zero thickness, with the place |
| leaving several lumps (the slab between two planes through a half ring) | one body of several shells, as after any boolean; `body.split` gives a body per lump |

**What is refused rather than built.** A plane that leaves nothing of the body (a feature has a body or fails; the reason names the plane and says which side the body lies on). A reference that is not a plane (a face that is not flat, a datum that is not a plane), a normal of no length, a point or an offset that is no length. A target that is a sheet. A zero-thickness or edge-of-tolerance state after *any* plane of the list, also one a later plane would cut away: each step is a whole boolean with its refusals on, because the boolean finds a touch from where its operands meet (the section's touch points and the places an edge meets the other body's surface), which regeneration's own look at the finished body, by its vertices and edges, does not have ([`booleans.md`](booleans.md) › *Zero thickness*); the reason names the plane and says that a plane further down the list that removes the place can be put first. So an intermediate state can refuse a cut whose final body would be fine. More than 256 planes. Work past the bound of one operation, per plane (`OPERATION_WORK`).

**A plane that cuts nothing is not an error.** The body is passed to the next plane as it is; the result says so per plane and under `warnings`. A subtraction whose tool misses its target is no error either, and a Jig whose chamfer plane clears the part at some sizes has to keep building. A reference to that plane's face fails by name.

**Planes from measured data.** Planes that should meet in one point and, as measured, meet in several a millimetre apart are cut as they are: the small facets and the short edges between them are in the body, exactly, and in every mesh of it. **This feature does not snap, merge or move anything**; merging what lies within a given distance is item 4 of the boolean checklist ([`../roadmap.md`](../roadmap.md)) and belongs to the boolean, where every operation gets it. What the person sees: at a tolerance far below the scatter, a body with the extra facets (the issue counts 44 to 86 triangles where 26 were meant); where what the scatter leaves falls under the boolean's rules of tolerance (points or faces between half and twice the tolerance apart; a piece of a face within the tolerance of a face it is not on, which neighbouring planes that are nearly one plane give over a strip as wide as the tolerance divided by the angle between them), a failed feature whose reason names the plane and the faces and says what the tolerance is. One thing the caller can do, and the summary says: planes meant to meet in one point are given through that point, the same numbers for each (`point` shared, the normals as measured). They then meet there to rounding, far within half the tolerance, and are one vertex (`plane_cut_names.rs`: three planes through one point meet in the vertex `F2:corner[F2:cut(1),F2:cut(2),F2:cut(3)]`; the guide's tower: four).

### Data and the file

`doc::PlaneCut { target, planes, next_plane, keep }`, each plane a `doc::CutPlane { id, by, offset, side }` with `by` a `CutPlaneBy::Plane { plane: PlaneRef }` or `CutPlaneBy::PointNormal { point, normal }`. A document whose cut has no plane, more than 256, an id of 0, an id twice or an id not below `next_plane` is not a document (`PlaneCut::fault`, held by `Document::validate`, so by every read and every save). **Format version 78** adds the feature kind `planeCut` and the name role `cut`; `v77_to_v78` is the identity, raised so that an older build says a newer 3DCraft is needed ([`../file-formats.md`](../file-formats.md)).

### Cost and bounds

*The bound.* At most 256 planes, each one boolean held to the bound on one operation's work, refused past it; so the feature cannot run on. Each cut costs the faces the plane crosses (their sections) plus a share for every face of the body (the operand's check, answered from memory for a body the kernel just returned; one ray against six faces per face not crossed; sewing; the result's check), and the body gains at most a face per plane and lump, so `n` planes on `F` faces cost about `n (F + n)` of the second kind.

*One changed plane rebuilds the whole feature*: regeneration caches per feature, and one feature is rebuilt whole, every plane again. Nothing before it is rebuilt, and what follows it only when its body changed. That is the price of one history entry; it is counted below, and slice 3 removes it if it is felt.

## Oracles

- **Closed forms** at 1e-3, 1 and 1e6 with the tolerance scaled along (`crates/kernel/tests/halfspace.rs`): a block with a corner cut off through three points (volume, area and centroid of both parts), by a plane through three of its vertices, through two opposite edges, through a vertex and across; a cylinder cut at half its radius (the segment, with its area), through its axis and on the slant; a ball's cap; a ring halved flat and upright, quartered, a cap cut off it, and the slab of two lumps between two planes; a gable roof by two planes in either order (19,200 and 4,480 on a 40 × 30 × 20 block), its ridge in the plane of the block's top.
- **Membership, independent of the boolean**: 600 seeded points per case; a point is in what the planes leave exactly when it is in the body (`contains_points`) and below every plane, which is a dot product. A block under three planes, a cylinder under two, a spline cone under one.
- **Every body built**: `topo::check`, the boolean's own check in full against the block it was cut with, and the sources the kernel promises (every face a piece of the body's or of the plane; a piece of the plane on the plane as given, bit for bit, facing the side cut away).
- **Identities**: the two sides of a plane add up to the body in volume and first moments (through the commands, with `keep`), and their areas to the body's and twice the section; on a spline cone the two sides add up to a part in a million; a plane given twice cuts nothing the second time; the same cut made the five-feature way has the same volume, area, centroid and counts of faces, edges, vertices and shells (`plane_cut.rs`, three planes at three scales; `plane_cut_cost.rs`, the issue's case).
- **The contact table above**, case by case.
- **Through the commands** (`crates/engine/tests/plane_cut.rs`, each command also held to "regenerating from nothing gives the same"): planes by point and normal, datum plane, base plane and flat face, with offset and side; the report and its warnings; the refusals with the plane named; the command's refusals of bad parameters; `feature.edit` by ids, undo and redo; switched off and inactive; a name of the target followed into the cut body; planes through variables; save and reopen.
- **The naming suite** (`crates/engine/tests/plane_cut_names.rs`): a fillet between two cut faces, a chamfer by an edge's own name, a sketch and a datum on a cut face, through every row of the table under *Names*; no two edges and no two vertices of a cut body share a name, also where a plane cuts in two places and where three planes meet in a point; the alias of a piece that joins a face of the target; `doc::identity`'s list with `cut(n)`.
- **The file** (`crates/format/tests/format.rs` › `a_plane_cut_is_written_like_this_and_round_trips`; the fixture `v78.3dcraft`, whose roof is held to its closed form by `crates/engine/tests/format_versions.rs`).
- **Never a crash**: `panic_hunt` picks the command up from its params string.
- **The guide's examples** run as written and meet their volumes (`guide_examples.rs`).
- **Work, not time** (`crates/engine/tests/plane_cut_cost.rs`): below.

## Not built yet

*(planned)*, in this order unless a report says otherwise:

2. **Breadth.** A sheet trimmed by planes (the new boundary edges named by the plane's id; refused today: "a sheet is trimmed by a solid with feature.boolean"); a condition per plane (`when` in a plane's object: left out, it renumbers nothing, since names go by id); one plane changed by its id without the list (`feature.edit {plane: {id, …}}`); the fuzzer's step (random planes through points of the body's box, oracle 11); a kind in the action matrix (each flat face's plane moved in by a tenth: the volume falls and is `measure.below`'s); an icon of its own in the feature list (it shares the split's); whether a face of the target that lies in a plane answers to `cut(k)` where the plane cuts nothing.
3. **Speed, after measuring.** The bodies after each plane kept with the cache entry, so that one changed plane redoes the planes from it on; for many planes on a body of thousands of faces, the convex tool or the half-space operand, whichever the benchmark and the corpus support.

Not this feature's: merging within a distance (the boolean checklist, item 4); a prism command, face provenance per triangle, machine-readable failure reasons (noted in the decision of 2026-11-20, not decided).

## Findings and history

### As built: the first slice (2026-11-20)

**What changed against the design.** `kernel::below` keeps its own block (*The cut in the kernel*, above). A body clear of a plane is told from the eight corners of its box, not from the tighter box of the body turned into the plane's frame that `below.rs` takes: looser only means the boolean is asked a little more often, and the body is not copied once per plane to be turned. `point` may be left out (the origin), so a plane at a distance along a direction is `{normal, offset}`. A plane's `cuts` says whether the cut body has a face of it, not whether it took something away at its turn: the first is the same whatever the order of the list. The feature list shows the cut with the split's icon; one of its own is owed (slice 2).

**The issue's case, counted** (`crates/engine/tests/plane_cut_cost.rs`; the workshop's Check run 44747074632 on ab290359, the build server; counts of work, the same on every machine and run): a prism of 70 outline vertices, 24 planes round a cone of about 30 degrees scattered as measured ones are, built with one `feature.planeCut` and with a datum plane, a sketch, a rectangle, an extrusion and a boolean per plane. Both give the same body (volume and area to 1e-9, the same counts of faces, edges, vertices and shells), before and after one plane is raised by 0.25 mm.

| | History | Booleans | Face pairs intersected | Face candidates | Edge candidates | Features rebuilt | Regenerations |
|---|---|---|---|---|---|---|---|
| one feature, built | 3 | 24 | 558 | 42,542 | 5,715 | 1 | 1 |
| five features a plane, built | 98 | 24 | 558 | 42,478 | 5,715 | 120 | 96 |
| one feature, the plane in the middle changed | | 24 | 557 | 42,535 | 5,704 | 1 | 1 |
| five features a plane, the plane in the middle changed | | 12 | 190 | 21,324 | 1,993 | 15 | 1 |
| one feature of 8 planes, built | 3 | 8 | 221 | 13,498 | 2,246 | 1 | 1 |

So the kernel does the same work either way (the same 24 booleans on the same bodies: 558 pairs of faces intersected in both), and what one feature saves is everything round it: 119 features built and named, 95 regenerations, 95 history entries. What it costs is the edit: one changed plane rebuilds the whole feature, 24 booleans where the five-feature way redoes the 12 from the changed plane on, as the design said it would. The work per plane does not grow with the list beyond the body's own growth: 1,687 face candidates a plane at 8 planes, 1,773 at 24.

**The fixture of format version 78** (`crates/format/tests/fixtures/versions/v78.3dcraft`, 3,770 bytes, FNV-1a `ba69a85d2569940a`) was written by `craft3d-cli` of ab290359 on the build server, by a test of that commit that ran the CLI as `versions.rs` says a fixture is written and printed the file to the run's log (the test is not kept): the roof of 18,240 mm³ and the kept copy above 6 mm of 11,400 mm³ are their closed forms to the last digits the integrator gives (18240.000000000007, 11400.000000000004).

**Checks on the build server while it was built** (the workshop's Check runs): 44743170888 (the kernel's cut: 12 tests, at the first run), 44745418825 and 44746230759 (the doc crate's tests; clippy of doc, format, kernel and regen with every target; the naming suite at its first run; two expectations of mine in `plane_cut.rs` wrong, a second target placed where the planes leave nothing of it and a roof whose ridge I had kept at 20 while its planes rise 8 over the eaves, both corrected to what the model is), 44747074632 (the fixture, the cost, `plane_cut`, `guide_examples`, `panic_hunt`, `commands`, the formatting).

### The design's measurements (2026-11-20, before anything was built)

All with the released `craft3d-cli` 0.2.1 (f6d88a043f5c) on the developer's laptop at a load of 1 to 1.6, one process at a time, capped; times are the server's own `ms` per command and **indicative only**. The scripts are not in the repository (generated; their parameters are given).

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

So the share of a cut per face of the body is about 0.07 ms there, and 25 planes on that plate cost 25 times it, 1.5 s, where one boolean would cost it once: where the convex tool or the half-space operand of slice 3 would pay.
