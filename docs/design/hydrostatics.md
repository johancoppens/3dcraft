# Hydrostatics

How 3DCraft floats bodies: the displaced volume below a waterplane, the centre of buoyancy, the waterplane's area and second moments, the waterline for a weight, the metacentre and the righting arm. For any closed bodies, not only hulls: a buoy, a pontoon, a float of a model, a bottle. The kernel part is `crates/kernel/src/hydro.rs`; the command and the assertion measure are `crates/engine/src/hydro_cmds.rs` (`measure.hydrostatics`, `assert.add {measure: "hydrostatic"}`); the user's view is the guide, [chapter 10](../guide/10-measure.md#hydrostatics).

## Method

**The water.** A plane `{p : p · up = level}` with its unit normal `up` (out of the water) and a unit direction `along` in it (the length of a ship). `across = up × along`. A heel turns the water by an angle about `along` (Rodrigues; `along` stays square to `up`), which is the body heeled the other way in still water: a positive heel tips `up` towards `−across`, so the side of the body towards `across` goes down. Heights (KB, KG, KM, the draft) are measured along the water's normal from the keel point K, `keel` along the upright `up` from the origin (default the origin).

**Below the water, exactly.** For each body, a box whose top face lies in the plane and that holds all of the body below it (wider than the body's box by a quarter of its half-diagonal on every side, and deeper) is intersected with the body by the boolean kernel. What comes back is an exact B-rep of the part below the water:

- the displaced volume ∇ and the centre of buoyancy B are its volume and centroid from the exact integrator (`craft3d_topo::mass_properties`: the divergence theorem over the faces, each sum held to 1e-12 of what it sums; [`topology.md`](topology.md));
- the waterplane is the faces of the result that are pieces of the box's top face (`BooleanResult::merged`), flat faces in the plane bounded by the curves in which the plane meets the body. A piece of any other face of the box in the result is an error (the box did not hold the body), never a silent number;
- the wetted surface is the result's area less the waterplane's.

A body wholly above the plane (by its box) displaces nothing; one wholly below displaces all of itself and has no waterplane, so BM is 0. Several bodies are summed (a catamaran): volumes, first moments, and the waterplane integrals about one common point; bodies that overlap are counted once each, so give bodies that do not.

**The waterplane's moments.** Green's theorem in the plane, with `a` along `along` and `b` along `across` from a point `o` of the plane over the middle of the bodies (so a small waterplane far from the origin loses nothing to cancellation):

| Integral | Boundary form |
|---|---|
| A | ∮ a db |
| ∫a dA | ∮ a²/2 db |
| ∫b dA | −∮ b²/2 da |
| ∫a² dA | ∮ a³/3 db |
| ∫ab dA | ∮ a²b/2 db |
| ∫b² dA | −∮ b³/3 da |

along every coedge of the waterplane's faces, in the direction the loops run about the surface's normal (counter-clockwise), signed by which way that normal faces against `up`. Per edge the panels end where its curve is not smooth (the knots of a spline; along a circle or an ellipse the first panels are a sixteenth of a turn, only as a starting point), each panel is summed with the 8- and the 16-node Gauss–Legendre rule, and a panel whose two sums differ by more than 1e-13 of the size to the integral's power (times the panel's share of the size, plus the rounding of the terms) is halved; at most 40 halvings of a panel and 65,536 panels per body, past which the waterplane is an error ("do not settle"). On a line every integrand is a polynomial of degree 3 at most and both rules are exact; on the cubic Hermite splines the boolean fits to the curve where the plane meets a surface, degree 11 per span, exact for the 8-node rule already. Moved to the waterplane's centroid F by the parallel-axis rule: `iTransverse = ∫b² − A b̄²` (about the axis along `along` through F, which gives the transverse metacentre), `iLongitudinal = ∫a² − A ā²`, `iProduct = ∫ab − A ā b̄`.

No tessellation is integrated anywhere. The error bound is that of the B-rep: the integrals are held to 1e-12 (volume) and 1e-13 per panel (waterplane), and the curve in which the plane meets a freeform surface lies on both within the modelling tolerance, so a waterplane bounded by such curves is exact to the tolerance times its perimeter, and the volume to the tolerance times the wetted area.

**The waterline.** The level at which the volume below is the displacement `m / ρ`. ∇(level) grows with the level and its derivative is the waterplane area, so it is Newton's method on the area, inside a bracket from the bodies' extent along `up` that every measured level narrows: a step that would leave the bracket, a waterplane of no area, or a step after one that did not halve the residual is a halving of the bracket instead. It stops when the volume is within 1e-10 of the displacement (`SETTLED`), or when the bracket is as narrow as an `f64` makes it; at most 64 steps (`MAX_STEPS`: halving alone takes the bracket below an `f64`'s resolution by then), after which "did not converge" is an error with the bracket and the residual. The first level is where the displacement would be for a prism of the bodies' extent. A displacement of zero or less is refused (*nothing to float*), so is one at least the bodies' whole volume (*the bodies sink*, also when it equals it: they hover). The search measures one level per step, each a boolean and an exact integration; on the analytic oracles it settles in 2 to 6 steps.

**Stability.** `BM = iTransverse / ∇`, `BMl = iLongitudinal / ∇`, `KM = KB + BM`, `GM = KM − KG` (and the longitudinal ones), `GZ = (B − G) · (up × along)`, positive when the moment of buoyancy about G turns the body back. G is `centerOfGravity` when given, else the centre of mass of the bodies with their materials (`crates/engine/src/mass.rs`) and the `loads`; `kg` sets its height above K, and with no centre known puts G on the line along `up` through the origin. All of these are in the water as it lies, heeled or not: the waterline at a heel is solved again for the same displacement, at fixed trim (the water turns about `along` only).

## Sources

- A. B. Biran, R. López-Pulido, *Ship Hydrostatics and Stability*, 2nd ed., Butterworth-Heinemann, Oxford 2014: the chapters on basic hydrostatics (displacement, centre of buoyancy, the waterplane's moments, the box barge), on small-angle stability (the metacentre, BM = I/∇, GM = KB + BM − KG) and on large-angle stability (the righting arm GZ).
- E. C. Tupper, *Introduction to Naval Architecture*, 5th ed., Butterworth-Heinemann, Oxford 2013, the chapter on stability: the wall-sided formula GZ = sin φ (GM + ½ BM tan² φ), exact for a wall-sided body while its waterline stays on the walls.
- The spherical cap and the circular segment (volume, area and centroid) used in the oracles: standard closed forms of any table of solids.
- Green's theorem for the area moments of a plane region bounded by curves: any calculus text (e.g. T. M. Apostol, *Calculus*, vol. II, 2nd ed., Wiley 1969, §11.19–11.22).

## Oracles

`crates/kernel/src/hydro/tests.rs`, each at 1e-3, 1 and 1e6 model units with the tolerance scaled along, to 1e-9 relative (analytic) or to the modelling tolerance (freeform):

- a box barge L × B × D: ∇ = L B T, KB = T/2, A = L B, I_T = L B³/12, I_L = B L³/12, BM = B²/(12 T), wetted = L B + 2 (L + B) T; floated by its displacement to T = m / (ρ L B) in at most 8 steps;
- a sphere at three levels: the spherical cap's volume, centroid 3 (2R − s)² / (4 (3R − s)) from the centre, the circle's area and πa⁴/4, the cap's area 2πR s, and KM = R (the metacentre of a sphere is its centre at every draft);
- a horizontal cylinder at two levels: the circular segment's area R²(θ − sin θ)/2 and centroid 4R sin³(θ/2) / (3 (θ − sin θ)), the rectangle's moments, the wetted area;
- a V prism (a square prism on an edge): ∇ = L T², KB = 2T/3, BM = 2T/3;
- a spline cone apex down (a surface a cubic spline sweeps about an axis): the cone's volume, KB = 3h/4, the circle's moments, the lateral area, and floated back to its level;
- the barge moved and turned in space, measured in water turned with it: the same ∇, KB, I and wetted area;
- a heeled box at ±5° and ±10°: GZ by the wall-sided formula;
- GM's sign on each side of a box's stability limit B/D = √(6 s (1 − s)) (relative density s, G at its middle);
- two hulls (a catamaran): the sum, and I by the parallel-axis rule;
- the errors: sinks, no displacement, not finite, no bodies, a sheet, directions that are zero or not square.

`crates/engine/tests/hydrostatics.rs` holds the command and the assertion measure to the same closed forms through the document (materials, the default sea water, `"1 g/cm3"`, heels as `"10 deg"` and `10`, levels, given masses and heights, loads, a keel below the origin), an assertion on GM that fails on a barge too narrow and passes once it is widened, variables renamed inside an assertion's parameters, and every refusal.

## Limits and what is owed

- **Trim is held.** The water turns about `along` only; the longitudinal balance (LCB under LCG) is not solved, so a body whose G is not over its B along its length is reported at the trim it was given. Owed: a second unknown (the trim angle) solved with the level, Newton on the 2 × 2 system of the waterplane area and its first moment.
- **The GZ curve** is one heel per call; a curve is a row of calls (or of assertions).
- **Bodies are solids.** A sheet holds no water out and is refused; an open hull must be closed first (a deck, or a sheet capped).
- **Overlapping bodies** are counted once each.
- **Every level is a boolean.** A body the boolean refuses at some level (a face of it lying in the waterplane within the tolerance, say a flat deck exactly at the waterline) gives that error, with the level; the search does not step around it.

### Every rule that is a condition, and why it is general

- *A body wholly above or below the plane by its box* is measured without a boolean: the box certainly holds the body (`Body::bbox`), so nothing is below or all is; this is a shortcut with the same answer, not another method.
- *The first panels of a circle or an ellipse are a sixteenth of a turn*, and those of a spline end at its knots: starting points for the same adaptive check on every curve; no curve is trusted without it.
- *A panel settles when its 8- and 16-node sums agree to 1e-13 of the size to the integral's power, in proportion to its length, plus 64 ulps of its terms*: the same for every curve and at every scale (relative to the bodies' size).
- *The waterline stops at a volume within 1e-10 of the displacement* (relative), or at a bracket no `f64` can narrow: both relative, the same for every body.
- *`along` must be square to `up` within 1e-9 of the cosine*: refused, not projected.

## Findings and history

- 2026-10-26: built (r1-hydro). The kernel's oracles hold to 1e-9 at all three scales on the first run; the waterline settles in at most 8 steps on them.
- 2026-10-26: the acceptance cross-check, the sea-kayak Jig's hull (branch `jig-sea-kayak`, its `build.json` at the defaults: three chines, 99.9 kg in sea water) floated by `measure.hydrostatics {of: "Env4", mass: "Delta", kg: "KG", density: "rho"}`, against the numbers its formulas computed outside 3DCraft (Clenshaw–Curtis over 49 stations): draft 107.577 mm (formula 107.577, 0.001 mm apart), KB 67.090 (67.091), BM 204.01 (203.98), KM 271.10 (README 271), GM 23.16 (formula 23.13, README 23), the waterplane 1.44497 m² (formula 1.44292, the kernel's area of its water body's top 1.4450), I_T 1.98854e10 mm⁴ (1.98824e10), LCB −124.0 mm (LCG −124.0). Solved in 4 steps, 2.4 s on the Strix (one level 0.8 s).
