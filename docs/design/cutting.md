# Cutting: plate parts, the cut list and their outlines

The platform side of cutting parts from sheet stock: which bodies of a design are plates, what each is (thickness, outline, holes, the rectangle it is cut from, its area), which are the same part, and their outlines to DXF and SVG with a kerf. Roadmap: [`../roadmap.md`](../roadmap.md) › *R1*. Code: `crates/io/src/cut/` (recognition and the 2D work on parts), `crates/engine/src/cut_cmds.rs` (`cut.list`, `cut.export`). Guide: [`../guide/12-files.md`](../guide/12-files.md) › *Cut lists*. Tests: `crates/io/src/cut/tests.rs`, `crates/engine/tests/cut.rs`.

**Not here: optimisation.** Nesting parts on a sheet to waste the least, a panel saw's order of cuts, a laser's lead-ins and order of contours are fabrication work on top of these commands (a separate module, a "Fab", decided 2026-11-02). `cut.export` lays parts out in rows only, to have something to cut and to look at.

## Method

### A plate

A body is a **plate** when it is a prism of constant thickness: its top and bottom two parallel flat faces facing away from each other, every other face square to them. The test, per axis (`crates/io/src/cut/plate.rs`):

1. The **axes**: the directions of the body's flat faces, each once up to sign (parallel within the angular tolerance). An axis is a candidate when the body has a flat face facing `+axis` at its highest level along it and one facing `−axis` at its lowest (levels over the body's vertices); the **thickness** is the difference.
2. Candidates are tried thinnest first (at most 1024). Along one: the top and the bottom are each **one** flat face (several in one plane are refused, naming them); then every other face's **surface** is checked square to the axis: a plane whose normal is square to it (one parallel to it is a pocket's floor, a step or a counterbore, refused with its depth below the top; one at another angle a chamfer, a bevel or a draft, with the angle), a cylinder along it (another a round or a hole at a slant), a freeform face that is a straight wall: every point of its edges (five per edge) carried along the axis to nine levels from the bottom to the top lies on the surface (within ten tolerances) with the normal there square to the axis (within the angular tolerance), so a wall that bulges between its edges is refused; a cone, a sphere and a torus are refused. Then every face's **corners** must lie in the top's plane or the bottom's (within ten tolerances plus 1e-12 of the body's size): a corner between them is a step, a pocket or a chamfer. Surfaces first and corners second, so the face named is the one at a slant, not its neighbour whose corner it moved. Last, the top and the bottom must have the same number of loops and the same area.
3. The first axis that passes is the plate's. A box is a plate of its smallest side. When a thinner axis failed, the part says why in `notes`: a 100 × 50 × 6 bar with one long edge chamfered is a prism *along* that edge, 100 thick, its outline the chamfered section, and the list says it is not a plate 6 thick, naming the chamfer. When none passes, the reason is the thinnest candidate's, naming the face.

Which side is the **top**: the one whose outward normal points most up, by the first of its z, y and x that is not zero. Its outline is drawn by `face_outline_in` exactly as `file.export {face}` draws a face: seen from outside, never mirrored, every curve as it is. The outer loop is the loop of largest area, the others are holes.

A turned rod is a disc as thick as the rod is long: a prism all the same. Faces split in one plane (a top that a boolean left in two pieces) are refused rather than merged: the list says so.

### A sheet-metal part

The body of a sheet-metal part (`feature.sheetMetal`), also after a boolean drilled it (the chain of `consumed_by` from the part's feature ends at the body), is a part by its **flat pattern**: the outline, holes and later cuts `file.export {sheetMetal}` draws (`sheetmetal_cmds::one_pattern`; [`sheetmetal.md`](sheetmetal.md)), its thickness the part's, drawn in its sketch's plane. Its marks come with it: bend lines up and down, a louvre's lance (a cut that is no loop) and its footprint. What the pattern does not carry (a later feature's pocket or notch) is in `notes`.

### The part's own frame and its rectangle

Each part is put in its **own frame**: x along the length of the rectangle of least area around its outline, the rectangle's corner at the origin, so the part lies in `[0, length] × [0, width]` (`crates/io/src/cut/rect.rs`). The rectangle of least area around a convex outline has a side along an edge of its convex hull (Freeman and Shapira 1975): the hull is taken of the outline's points (every segment's ends and 32 steps along each curved piece), each hull edge's direction is tried (turned by quarter turns into [0°, 90°), at most 8192), and in each the outline's extent is measured exactly for lines, arcs, circles and ellipses (their extreme points) and for splines as an upper bound within 1e-9 of the spline's size: its Bézier pieces halved by de Casteljau (on the weighted points) where their control points reach further than any point found on the curve, at most 512 halvings per extreme, the control points left counting when they run out (so the rectangle is never too small; with splines at most 1024 directions). Of rectangles with the same area within 1e-12 the one turned least is kept; the longer side is the length. For an outline whose hull has a straight side on the least rectangle (any polygon, a rounded rectangle, a slot) the result is exact; for a round hull it is the least over the directions of the hull's chords. The frame's place in the model (origin, x, y, normal) is in the list.

### The same part

Two parts are the same when one is carried onto the other by a rigid motion of the sheet, **turned over included** (a plate cut from sheet stock is the same part either side up; turned over, a bend up is a bend down, so a left and a right bracket are two parts, but a plate and its mirror image are one), to within `max(16 tol, 1e-9 × the rectangle's diagonal)` (`crates/io/src/cut/same.rs`). The test is deterministic:

- Each shape is a set of **pieces**: every segment of the outline and the holes and every mark, each with its ends (in either order), centre and size (an arc's length and the point half way along it, which tells a half circle from its opposite with the same ends, centre and length; an elliptic arc's axis, ratio and half-way point; a circle's radius; an ellipse's axis and ratio, the ends of its longer axis key points; a spline's control points, knots on [0, 1] and weights, all in order or all reversed), so neither where a loop starts nor which way it runs matters.
- Cheap measures a motion keeps are compared first: the number of pieces of each kind and mark (bends up and down together), the number of holes, the area, the cut length.
- The motion: the centroid of the material (from the loops' points, sampled in each curve's own parameter, so congruent shapes give congruent points) and the key point (an end or a centre) farthest from it. For every key point of the other shape at the same distance from its centroid, mirrored or not, the one motion that takes centroid to centroid and point to point is tried; a shape whose key points all sit at the centroid (a disc, rings) is tried unturned. A motion is accepted when it takes every piece onto a piece of the other, one to one, found through a grid of cells four tolerances wide (a `BTreeMap`).
- Bounded: each part's pieces, counts, area, cut length, centroid and key points are worked out once; every comparison, index entry built, piece moved, key point tried and cell candidate looked at is charged to one budget of 5 million per list; past it a part is counted apart and named under `uncompared`.

Parts are grouped by thickness (within ten tolerances plus 1e-9 of it) and material name (`body.material`, followed through booleans as the mass is); groups by thickness, then material (none first); parts in a group in document order; a part's name is the first body's.

### The kerf

`cut.export {kerf}` offsets the outline outwards and every hole inwards by `kerf`, the distance from the part's edge to the beam's centre (half the width the beam cuts away), with **round** corners: every point of the path at the kerf's distance from the part, as a round beam goes round an outside corner. It is the kernel's one offset of closed profiles (`craft3d_kernel::offset_profile`, which `feature.offsetProfile` uses: lines stay lines, arcs arcs about the same centre), not an offset of its own (`crates/io/src/cut/kerf.rs`). An inner round of a smaller radius than the kerf has no offset: the beam cannot follow it, as it cannot follow a sharp inner corner, and both come out rounded at the kerf's radius; this is accepted and said in the part's `notes` ("segment 3 of the outline, an inner round of radius 1, comes out at the kerf's radius 2"). Refused, naming the part: a kerf that changes the loops (a hole that closes, a loop that splits), with the largest that fits; a kerf with which another curve of the part has no offset on the path, or more than one (a line that vanishes: a notch, a slot or an inner corner narrower than the beam: the beam cannot cut it; the largest kerf that cuts every curve found by bisection, 12 offsets); an ellipse or a spline (their offsets are no ellipse or spline; not built). Marks stay where they are.

### Output

`cut.list` returns JSON (groups, parts, notPlates, uncompared), or with `format: "csv"` one CSV text (RFC 4180 quoting) with the header `material,thickness (u),name,count,length (u),width (u),area (u^2),cut length (u),holes,kind,bodies,reason` in the document's unit, a row per distinct part, then a row per body that is not a part. Numbers are the shortest decimal text that reads back as the same `f64`; a name that starts like a formula (`=`, `+`, `-`, `@`) gets a `'` in front, so a spreadsheet does not run it.

`cut.export` draws each part in its own frame on a layer named after it (the drawing crate's layer names: `file.export`'s rule, `-2`, `-3` for a name taken), its outline, holes and cut marks on it; bend lines on `bendUp` (blue, dashed) and `bendDown` (red, dash-dot), a louvre's footprint on `form`, shared by every part. One part of each kind (with `count`), or every body with `copies`. `layout: "sheet"` places them by their outlines' boxes left to right in the list's order, `gap` apart (5 mm by default), in rows of at most `width` (a new row above the last; a part longer than the width refused); `layout: "each"` writes one file per part, named after the path with the layer after a dash. Every file is encoded and every path resolved before any is written, so a part that cannot be drawn or a path outside the root writes nothing; a disk that fails half way can leave the files written before it, each whole. DXF and SVG are written by `craft3d-drawing` as every other outline ([`../file-formats.md`](../file-formats.md) › *Flat outlines*), so FreeCAD reads them as it reads those. The same document gives the same bytes.

## Oracles (held to)

- Closed forms at three scales (1e-3, 1, 1e3): a box (thickness, 100 × 50, area, cut length, frame), a plate with a round hole and a slot (area `5000 − 25π − (240 + 16π)`, cut length `300 + 10π + 60 + 8π`), an L (100 × 60 from an L standing on its short side, area 2800), a D of a line and a cubic (its freeform side wall square to it; width 45, the cubic's highest point; area 3600), a sheet-metal L bracket by its pattern (`w × (d + θ(R + K T) + l)`).
- The same part: a box lying, standing on its side, turned 30° about z and tilted about (1, 2, 3) are one part of count 4; an L and its mirror image one part; a box of steel and one of no material apart from the birch ones; the same box with a hole another part; in the unit tests a hole moved (same counts, area and length) another part, a bend up turned over a bend down, and a test out of work `None`.
- Refused: a pocket (its floor named, with the depth), a block with two chamfered top edges along x and y (the chamfer named, 45°), a sphere (no flat face); a block with one chamfered long edge listed 100 thick with the note.
- Kerf in closed form: the rectangle `(100 + 2k) × (50 + 2k)`, its outer area by Steiner `A + P k + π k²`, the round hole's radius `5 − k`, the slot's ends `4 − k`, four corner arcs of radius `k`, read back from the DXF; a notch 2 wide refused at a kerf of 1.5 with the largest kerf ≈ 1; a slot that closes refused.
- Read back from the DXF (LINE, ARC, CIRCLE per layer) and the SVG: per part length and box, the second part `gap` to the right of the first, rows at a width, the same bytes twice; one file per part.
- Two plates with D holes whose half circles bulge the other way are two parts (same ends, centres, lengths, areas); a spline with other knots or weights is another piece.
- A barrel lofted through three squares (half-widths 20, 25, 20) (its wall bulging between flat top and bottom) is refused, naming the freeform face.
- An L whose inner round of radius 1 meets a kerf of 2: accepted with the note, the path's corner where the lines' offsets meet.
- The guide's shelf runs as written and gives the CSV the guide prints.
- `panic_hunt` runs both commands with adversarial parameters in its five document states.

## Limits and what is owed

- No nesting, no panel-saw sequence, no lead-ins (a fabrication module's work).
- Every part is taken to be cut either side up: a sheet with one good side, or with a grain that must run along a part, is not looked at (the list counts a part and its mirror image as one; `frame` says which side is drawn).
- No text on the drawing: the part's name is its layer (the drawing crate has no text entity).
- A kerf on an ellipse or a spline is refused (the kernel's offset does not offset them).
- An assembly is refused: list the parts of its part documents.
- A plate's top split into several coplanar faces is refused, not merged.
- The bounding rectangle of a round hull is the least over its hull's chord directions, not the analytic least.
- A sheet-metal part whose body a union joined to another is refused with the pattern's reason, not recognised as a plate.

## Findings and history

- 2026-11-03 (review): a half circle and its opposite were the same piece (same ends, centre, length): pieces now carry their half-way point; spline knots and weights compared; freeform walls proven straight along the axis, not only along their edges; a spline's extent an upper bound; the work budget covers everything; inner rounds smaller than the kerf accepted with a note; CSV formula guard.
- 2026-11-03: built (r1-cutting). A single chamfered long edge makes a box a prism along that edge, which the first test missed: the thinnest axis is now preferred and a part found along a thicker one says why it is not the thinner plate.

## Sources

- H. Freeman, R. Shapira, "Determining the minimum-area encasing rectangle for an arbitrary closed curve", *Communications of the ACM* 18(7), 1975.
- A. M. Andrew, "Another efficient algorithm for convex hulls in two dimensions", *Information Processing Letters* 9(5), 1979 (the monotone chain).
- RFC 4180, *Common Format and MIME Type for Comma-Separated Values (CSV) Files*.
