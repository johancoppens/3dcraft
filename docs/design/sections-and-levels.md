# Sections and levels

What lies on one side of a plane, for any closed bodies: the volume and centroid of the part below a plane, the area, centroid and second moments of the section by it, and the offset of a plane at which the volume below it is a given volume. A tank's fill level and the centre of mass of its liquid, a beam's cross-section, the cut of a part at a height, the plane a floating object settles at. The kernel part is `crates/kernel/src/below.rs`; the commands and the assertion measures are `crates/engine/src/below_cmds.rs` (`measure.below`, `measure.section`, `measure.levelFor`; `volumeBelow`, `centroidBelow`, `sectionArea`, `sectionCentroid`, `sectionInertia`, `levelFor`); the user's view is the guide, [chapter 10](../guide/10-measure.md#below-a-plane-sections-and-levels).

## Method

**The plane.** `{p : p · normal = offset}` with a unit `normal`; *below* is the side the normal points away from. The commands take it as `measure.symmetry` takes a plane (a base plane, a datum plane, `{point, normal}`) or as a flat face by its persistent name (its exact surface's frame, with the outward normal), moved by `offset` along the normal; the offset they return is from the plane given. The section's axes are `xAxis` (in the plane: given, which must be square to the normal within 1e-9 of the cosine, or by default the world axis least parallel to the normal, x before y before z, made square to it) and `yAxis = normal × xAxis`.

**Below the plane, exactly.** For each body, a box whose top face lies in the plane and that holds all of the body below it (wider than the body's box by a quarter of its half-diagonal on every side, and deeper) is intersected with the body by the boolean kernel. What comes back is an exact B-rep of the part below:

- its volume and centroid are those of the exact integrator (`craft3d_topo::mass_properties`: the divergence theorem over the faces, each sum held to 1e-12 of what it sums; [`topology.md`](topology.md));
- the section is the faces of the result that are pieces of the box's top face (`BooleanResult::merged`), flat faces in the plane bounded by the curves in which the plane meets the body. A piece of any other face of the box in the result is an error (the box did not hold the body), never a silent number;
- the surface below is the result's area less the section's.

A body wholly above the plane (by its extent along the normal: the held box, `Body::held_bbox`, of the body turned so that the normal is z, tight for analytic faces, so a plane tangent to a sphere along any direction meets nothing) has nothing below; one wholly below has all of itself below and no section. Extents, boxes and whole volumes are measured once per call and kept across the steps of a level search. Where the boolean finds nothing below (an empty result, or one of no volume) although a vertex of the body lies more than the tolerance below the plane, that is an error (*not measured*), never a silent zero. Several bodies are summed: volumes, first moments, and the section's integrals about one common point; bodies that overlap are counted once each.

**The section's moments.** Green's theorem in the plane, with `a` along `xAxis` and `b` along `yAxis` from a point `o` of the plane:

| Integral | Boundary form |
|---|---|
| A | ∮ a db |
| ∫a dA | ∮ a²/2 db |
| ∫b dA | −∮ b²/2 da |
| ∫a² dA | ∮ a³/3 db |
| ∫ab dA | ∮ a²b/2 db |
| ∫b² dA | −∮ b³/3 da |

in two passes: the area and the first moments about the middle of the section's own box (the boxes of its faces), which give the centroid, then the second moments about the centroid itself, so a small section far from the bodies' middle (a thin rod cut near one end) loses nothing to cancellation; along every coedge of the section's faces, in the direction the loops run about the surface's normal (counter-clockwise), signed by which way that normal faces against the plane's. Per edge the panels end where its curve is not smooth (the knots of a spline; along a circle or an ellipse the first panels are a sixteenth of a turn, only as a starting point), each panel is summed with the 8- and the 16-node Gauss–Legendre rule, and a panel whose two sums differ by more than 1e-13 of the section's own size (half its box's diagonal) to the integral's power (times the panel's share of the size, plus 64 ulps of its terms) is halved; at most 40 halvings of a panel and 65,536 panels per body, past which the section is an error ("do not settle"). On a line every integrand is a polynomial of degree 3 at most and both rules are exact; on the cubic Hermite splines the boolean fits to the curve where the plane meets a surface, degree 11 per span, exact for the 8-node rule already. About the centroid, with the parallel-axis rule taking out what rounding leaves of the first moments: `ix = ∫b² − A b̄²` (about the axis along `xAxis` through the centroid), `iy = ∫a² − A ā²`, `ixy = ∫ab − A ā b̄`; the principal moments `(ix + iy)/2 ± √(((ix − iy)/2)² + ixy²)`, the major one about the axis at `θ = ½ atan2(−ixy, (ix − iy)/2)` from `xAxis` (where the two are equal every axis is principal and that gives `xAxis`).

No tessellation is integrated anywhere. The error bound is that of the B-rep: the integrals are held to 1e-12 (volume) and 1e-13 per panel (section), and the curve in which the plane meets a freeform surface lies on both within the modelling tolerance, so a section bounded by such curves is exact to the tolerance times its perimeter, and the volume to the tolerance times the surface below.

**The level for a volume.** The offset at which the volume below is the volume asked for. The volume below grows with the offset and its derivative is the section's area, so it is Newton's method on the area, inside a bracket from the bodies' extent along the normal that every measured offset narrows: a step that would leave the bracket, a section of no area, or a step after one that did not halve the residual is a halving of the bracket instead. It stops when the volume is within 1e-10 of the one asked for (`SETTLED`); where the bracket is as narrow as an `f64` makes it, the offset there is the answer only within 1e-7 (`UNSETTLED`), and otherwise "did not converge". An offset the boolean refuses (a face lying in the plane where a Newton step landed) is stepped around once by halving the bracket; refused there too, it is the error; at most 64 steps (`MAX_STEPS`: halving alone takes the bracket below an `f64`'s resolution by then), after which "did not converge" is an error with the bracket and the residual. The first offset is where the volume would be for a prism of the bodies' extent. A volume not strictly between 0 and the bodies' whole volume is refused (*outside (0, total)*). Each step is one boolean and one exact integration; on the oracles it settles in 2 to 6 steps.

**For a Jig: a floating body.** With ρ the liquid's density, a body of mass m floats with V = m/ρ below the plane square to up: `measure.levelFor {volume: V}` gives the draft T (offset) and the centre of buoyancy B (centroid), and the section is the waterplane. Then KB is B's height above the keel, BM = ix / V (ix about the axis along the length), KM = KB + BM, and GM = KM − KG for the centre of gravity G the Jig knows; heeled by φ, the plane's normal turned by φ about the length and `levelFor` again gives B at the heel, and the righting arm is GZ = (B − G) · (normal × xAxis) (Biran & López-Pulido, *Ship Hydrostatics and Stability*). `crates/engine/tests/plane_measures.rs` does exactly that for a box (GM changing sign at B/D = √(6 s (1 − s))), and the kernel's tests hold the centroid below a turned plane to the wall-sided closed form. The naval quantities themselves belong in Jigs (rule 14).

## Sources

- Green's theorem for the area moments of a plane region bounded by curves: any calculus text (e.g. T. M. Apostol, *Calculus*, vol. II, 2nd ed., Wiley 1969, §11.19–11.22); principal moments of area: any mechanics of materials text (e.g. J. M. Gere, B. J. Goodno, *Mechanics of Materials*, the appendix on moments of inertia of areas).
- The spherical cap and the circular segment (volume, area and centroid) used in the oracles: standard closed forms of any table of solids.
- The floating-body example: A. B. Biran, R. López-Pulido, *Ship Hydrostatics and Stability*, 2nd ed., Butterworth-Heinemann 2014; E. C. Tupper, *Introduction to Naval Architecture*, 5th ed., Butterworth-Heinemann 2013 (the wall-sided formula GZ = sin φ (GM + ½ BM tan² φ), used as an oracle).

## Oracles

`crates/kernel/src/below/tests.rs`, each at 1e-3, 1 and 1e6 model units with the tolerance scaled along, to 1e-9 relative (analytic) or to the modelling tolerance (freeform):

- a thin rod (r, 1000 r) tilted 0.5 rad and cut near its lower end: the ellipse's ix = π a b³/4 and iy = π a³ b/4 to 1e-9 of themselves;
- a tube: the annulus (the inner loop a hole), A = π (R² − r²), ix = iy = π (R⁴ − r⁴)/4;
- a U: one body, two section faces, iy by the parallel-axis rule;
- a stepped block: the level for a volume of 2 is the shoulder, where Newton lands on the step's flat face, and the measure exactly there;
- a sphere cut along (1, 1, 1): its extent is its own, the tangent plane cuts nothing, a plane a tenth inside it cuts the cap;
- a box L × B × D below a level T: volume L B T, centroid T/2, area L B, ix = L B³/12, iy = B L³/12, surface below L B + 2 (L + B) T; the level for that volume back to T in at most 8 steps;
- a sphere at three levels: the spherical cap's volume, centroid 3 (2R − s)² / (4 (3R − s)) from the centre, the circle's area and πa⁴/4, the cap's area 2πR s; ix / V + the centroid's height is the centre at every level;
- a horizontal cylinder at two levels: the circular segment's area R²(θ − sin θ)/2 and centroid 4R sin³(θ/2) / (3 (θ − sin θ)), the rectangle's moments, the surface below;
- a V prism (a square prism on an edge): volume L T², centroid 2T/3, ix / V = 2T/3;
- a spline cone apex down (a surface a cubic spline sweeps about an axis): the cone's volume, centroid 3h/4, the circle's moments, the lateral area, and the level for its volume back;
- a rectangle turned in the plane: its product of inertia mixes, its principal moments come back, the major axis square to the long side;
- the box moved and turned in space, measured in a plane turned with it: the same volume, centroid height, moments and surface;
- a box under a plane turned by ±5° and ±10° about x with the same volume below: the centroid's lever about a point on its axis by the wall-sided closed form;
- two bodies apart: the sum, and ix by the parallel-axis rule;
- the errors: outside (0, total), not finite, no bodies, a sheet, directions that are zero or not square.

`crates/engine/tests/plane_measures.rs` holds the commands and the six assertion measures to the same closed forms through the document (offsets and volumes as expressions with units, a plane facing down, an assertion on a section's moment that fails on a block too narrow and passes once it is widened, a level that moves when the tank widens, a variable renamed inside an assertion), every refusal, and the floating-body worked example.

## Limits

- **Bodies are solids.** A sheet has no inside and is refused; an open shape must be closed first.
- **Overlapping bodies** are counted once each.
- **Every offset is a boolean.** A plane on a flat face of a body is coincident faces, which the booleans handle ([`booleans.md`](booleans.md), the table of contact): the stepped block cut exactly at its shoulder, and the level search landing there, are measured (`a_level_on_a_flat_face_of_the_body_is_found`). A body the boolean refuses at some offset gives that error with the offset; the level search steps around one such offset by halving.
- **One plane at a time.** A turned plane (a tank tilted, a heel) is another call with another normal; a level with two unknowns (offset and tilt for a given centroid) is a Jig's loop over `levelFor`.

### Every rule that is a condition, and why it is general

- *A body wholly above or below the plane by its extent* is measured without a boolean: the held box of the turned body certainly holds it (`Body::held_bbox`), so nothing is below or all is; a shortcut with the same answer, not another method.
- *Nothing found below with a vertex more than the tolerance below the plane* is an error: the tolerance is the document's, the same rule for every body (a body without vertices, such as a whole sphere, has its extent alone to go by).
- *A level search steps around a refused offset by one halving*: the same bracket rule as every other step.
- *The first panels of a circle or an ellipse are a sixteenth of a turn*, and those of a spline end at its knots: starting points for the same adaptive check on every curve; no curve is trusted without it.
- *A panel settles when its 8- and 16-node sums agree to 1e-13 of the size to the integral's power, in proportion to its length, plus 64 ulps of its terms*: the same for every curve and at every scale (relative to the bodies' size).
- *The level stops at a volume within 1e-10 of the one asked for* (relative), or at a bracket no `f64` can narrow when it is within 1e-7: both relative, the same for every body.
- *The section's panels are held to the section's own size*, not the bodies': relative to what is summed, at every scale.
- *The x axis must be square to the normal within 1e-9 of the cosine*: refused, not projected. *The default x axis is the world axis least parallel to the normal, made square*: a choice of axes only; the principal moments do not depend on it.

## Findings and history

- 2026-10-26: built (r1-hydro); first with naval quantities, narrowed the same day to these general measures at the developer's decision (the naval quantities belong in Jigs). The kernel's oracles hold to 1e-9 at all three scales on the first run.
- 2026-10-26: the acceptance cross-check, the sea-kayak Jig's hull (branch `jig-sea-kayak`, its `build.json` at the defaults, 99.9 kg in sea water of 1025 kg/m³), its hull body cut by the plane square to z, against the numbers the Jig's own formulas computed outside 3DCraft (Clenshaw–Curtis over 49 stations): the level for its volume 107.577 mm (formula 107.577, 0.001 mm apart), the centroid's height 67.090 (67.091), the section's area 1.44497 m² (formula 1.44292), its ix 1.98854e10 mm⁴ (1.98824e10), the centroid's x −124.0 mm (the Jig's centre of gravity −124.0). From these the floating-body example above gives 271.10 mm for KM and 23.16 mm for GM (the Jig's README: 271 and 23). The level in 4 steps, 2.4 s on the Strix (one offset 0.8 s).
- 2026-10-26, review of 9e26d7fb: the section's moments were taken about the middle of the bodies and moved by subtraction, with panels held to the bodies' size; a thin rod cut near its end lost about 1e-4 of its moments. Now two passes about the section itself, held to its own size (the rod's test). Extents along the normal from the held box of the turned body; an empty cut with a vertex below is an error; a bracket that cannot narrow far from the volume is "did not converge"; a refused offset in the level search is stepped around; boxes and whole volumes kept across the steps. Tests for a hole, two section faces in one body, a level on a flat face (coincident faces: measured), an oblique tangent plane.
