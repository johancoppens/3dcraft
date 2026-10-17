//! Area, volume and centroid from the exact B-rep, independent of any
//! tessellation: the backbone of the test oracles (docs/testing.md).
//!
//! Method. By the divergence theorem the volume and its first moments are
//! surface integrals over the faces; with the surface's own parametrisation
//! each becomes `∬_D f(u, v) du dv` over the face's parameter domain `D`:
//!
//! - area:    `f = |Sᵤ × Sᵥ|`
//! - volume:  `f = S · (Sᵤ × Sᵥ) / 3`
//! - moments: `f = S_k² (Sᵤ × Sᵥ)_k / 2` for each coordinate `k`
//! - second moments ([`inertia`] only): `f = S_k³ (Sᵤ × Sᵥ)_k / 3` for
//!   `∫x_k²`, and `f = S_k² S_l (Sᵤ × Sᵥ)_k / 2` for `∫x_k x_l`
//!
//! `D` is only known through its boundary, the face's loops. Green's theorem
//! turns the domain integral into a boundary one:
//! `∬_D f du dv = -∮ G du`, with `G(u, v) = ∫ f(u, t) dt` from a reference
//! `v₀` to `v`. Both integrals are Gauss–Legendre sums; the boundary is
//! followed along the edge curves, projected onto the surface.
//!
//! Five things make `D` more than a region of the plane.
//!
//! **Loops around the axis.** On a surface closed around its axis a loop may
//! wind around it (the rim of a cone, the two ends of a cylinder wall). `G`
//! is periodic in u, so nothing changes, except that a face whose boundary
//! winds once in total reaches to the end of the parameter range on one
//! side (an apex): `v₀` is then that end, where the missing boundary
//! contributes nothing.
//!
//! **The apex of a cone.** At the apex a whole stretch of the edge of the
//! parameter range is one point, and where the boundary passes through it
//! no edge of the body stands for that stretch. With `v₀` at the apex `G`
//! is zero all along it, so nothing is missing; better still, `G du` is
//! then smooth however close to the apex the boundary passes (it is
//! `x dy − y dx` across the axis, times a polynomial), where with any
//! other `v₀` it has a peak as narrow as the boundary is close. So `v₀` is
//! the apex whenever the boundary comes nearer to it than half as far as
//! it reaches from it, and otherwise a point of the boundary itself, so
//! that a band far from the apex is not measured as the difference of two
//! large cones.
//!
//! **The poles of a sphere.** The same holds at the pole `v₀` is put at,
//! but a sphere has two, and no choice of `G` is smooth at both: the area
//! of a sphere is not the boundary integral of anything smooth all over
//! it. A sphere, however, is the same surface whichever way its poles
//! point, and the integrals do not depend on the parametrisation. So a
//! sphere face is measured in a frame of its own, chosen for its boundary
//! and not taken from the surface: the south pole as far from the boundary
//! as it gets (among the axes of the boundary's circles, the direction of
//! its mean point, and the axes and diagonals of the surface's own frame),
//! which puts the north pole, where `v₀` is, in the middle of the
//! boundary. What the boundary integral then gives is the face, less the
//! whole sphere when the face holds the south pole; the area, which lies
//! between zero and the area of the sphere, says which.
//!
//! **Surfaces closed both ways (the torus).** There `f du dv` is not the
//! derivative of anything periodic in both directions: `G` gains the
//! integral over a whole turn of the tube each time v goes around. Loops
//! that do not wind around the tube are followed with v unrolled
//! continuously; for loops that do (the ends of a pipe elbow) the roles of
//! u and v are swapped: `∬_D f du dv = ∮ H dv` with
//! `H(u, v) = ∫ f(t, v) dt`, and u unrolled. Either way the result is only
//! determined up to a whole number of times the integral over the entire
//! torus: which turn of the unrolled parameter a loop starts on is
//! arbitrary, and so is which side of its loops the face is on when that
//! is the larger one. The area settles it: it lies between zero and the
//! area of the torus, and the same whole number applies to all five
//! integrals.
//!
//! **Spline surfaces.** A spline surface is smooth only between its knot
//! lines, and so are `f` and `G`. The inner integral is summed stretch by
//! stretch between the surface's knots in its direction, and the boundary
//! panels end where an edge crosses a knot line (found by a change of side
//! along the edge and bisection) as well as at the edge's own knots, and
//! where an edge within the tolerance of a Bézier or NURBS surface, not
//! on it, runs off the surface's domain: past the border its points are
//! projected onto the border, and the parameters along it have a kink
//! there ([`run_off`]). `f`
//! is a square root of a polynomial or rational function there, for which
//! no panel count is known to do: the inner panels are doubled along with
//! the boundary's, so the sums below only agree once both have settled.
//! On a rational surface each stretch of an inner integral is summed in the
//! parameter of the Möbius map that makes the surface's weights at its two
//! ends equal ([`moebius`], since 2026-10-17): the same integral, with the
//! nodes where the surface is drawn evenly, so that weights far apart cost
//! nothing where a reparametrisation can take them out (every surface of
//! revolution) and as little as they can elsewhere. Weights that stay more
//! than [`MAX_UNEVEN`] apart within a span however the span is
//! reparametrised are refused ([`MassError::UnevenWeights`]). A
//! spline surface may also close on itself around u without saying so (a
//! sphere or a can swept from a rational circle). A boundary that then
//! goes around once in total is the rim of a face that reaches to an end
//! of v: a face only if the surface is a single point there, where `G`
//! then runs from, as from an apex. A face without
//! loops is the whole surface, summed between its knot lines both ways
//! until two sums agree.
//!
//! **Spline surfaces that close on themselves.** A Bézier or NURBS surface
//! closes in a direction when its control net ends there on the row it
//! starts with, on clamped knots (`spline_closure`); `craft3d-geom` gives
//! it no period. Where the face goes over the seam, its region in the
//! parameter rectangle is bounded by its loops *and* by stretches of the
//! seam lines, which no edge stands for. A seam line of constant u adds
//! nothing to `-∮ G du`, so a surface that closes in u needs nothing (the
//! samples' parameters may jump across the seam: `du` is a rate, and `G`
//! is evaluated where each sample is). A seam line of constant v does add
//! to it; so where the boundary jumps in v by more than half the domain
//! between two samples in a row, the roles are exchanged: `∮ H dv` with
//! `H(u, v) = ∫ f(t, v) dt` from a value of u, to which the seam adds
//! nothing, with everything else as for a surface closed in u (a
//! boundary that goes around in v once in total is the rim of a face
//! that reaches to a pole at an end of u). Where it jumps both ways (a
//! NURBS torus with a window across both seams), u and v are unrolled
//! along each loop as on an analytic torus, the inner integrals sum each
//! turn beyond the domain inside it, and the area settles which whole
//! number of entire surfaces to take away. A face whose loops run the
//! wrong way around what they enclose is everything outside them on a
//! surface that closes, or ends in a pole, on every side
//! (`closed_all_around`), and that is the whole surface less what they
//! enclose. A face whose loops only walk seam edges there and back (each
//! edge once each way: a STEP file's whole torus or egg, or a sphere with
//! its meridian seam) is the whole surface, on such a surface or on a
//! sphere or a torus: the loops enclose no area on one side, and a face
//! has an area. Before 2026-10-14 a face that closes in v was integrated
//! against `du` alone and measured wrong without an error.
//!
//! **The poles of a spline surface.** A border of a Bézier or NURBS
//! surface may be one point in space (every control point of it the
//! same): the pole of a dome swept about an axis, the apex of a ruled
//! cone, the top of the teapot's lid. Like an apex it is a whole side of
//! the parameter range that no edge of the body stands for, and a
//! boundary that passes through it (at the end of an edge or on an edge's
//! way, where the edge is then cut so that its corner in parameter space
//! falls between two panels) is closed in parameter space by a stretch of
//! that border. With the inner integral running from the pole, `G` is
//! zero all along that stretch and nothing is missing; the integrand of
//! the inner integral is smooth up to the pole (the normal's length goes
//! to zero there in proportion to the distance from the border, which is
//! what makes the area element integrable). For a pole on a border of
//! constant u the roles of u and v are exchanged, as on a torus.
//!
//! A boundary may pass through the poles at both ends (the face a spline
//! sweeps that starts and ends on the axis runs from pole to pole between
//! two meridians and has no other edge: every `du` along it is zero, and
//! all of its area lies between the two stretches no edge stands for).
//! No `G` is zero at both. The face is therefore divided along a line of
//! constant parameter between the poles: on the near side of it the
//! inner integral runs from the near pole, beyond it from the far pole.
//! The two differ by the integral over the whole way between the poles,
//! `W`, which depends on the other parameter alone; along a stretch of
//! the boundary beyond the line that difference sums to the integral of
//! `W` between where the stretch goes over the line and where it comes
//! back, which is the integral of `f` over a rectangle and is summed as
//! one. The crossings are found from the samples (two in a row on
//! different sides) by bisection along the edge, and the panels of the
//! boundary end at them. Where the boundary goes around the pole or
//! which way the edges leave it never enters: both integrals vanish at
//! their own pole however the boundary passes through it. A boundary
//! through two poles that also crosses the seam of a surface that closes
//! on itself is declined (the rectangle would have to go around).
//!
//! A boundary that only comes close to a pole without passing through it
//! (further than [`AT_APEX`] of its reach from it) is measured as on any
//! other spline surface, from a point of the boundary itself: its sums
//! have a peak as narrow as the boundary is close, and settle or are
//! declined.
//!
//! **Nothing is taken on trust.** The boundary integral of a face is
//! computed twice, first with half the panels its edges need and then with
//! all of them, and again with twice as many until two in a row agree to
//! 1e-12 of the size of what was summed (or to the rounding of the face's
//! coordinates, where that is more: the rounding over how far the
//! boundary reaches across, and for a strip over how wide it is, which
//! is all its sums can agree to); a face for which they never do is an
//! error ([`MassError::Unsupported`]), not a number. Only a plane face
//! bounded by lines, circles and ellipses is exempt. There `G du` is a
//! polynomial of degree four at most, in the parameter of a line or in
//! the sine and cosine of an angle that spans no more than an eighth of a
//! turn per panel; the rule's sixteen nodes are exact for the one and
//! within (π/8 · 4)³² / 32! ≈ 1e-29 of the other. Along a spline the
//! panels end at its knots: it is smooth only between them.
//!
//! A spline edge of many knot spans (the curve in which two surfaces
//! meet is fitted as a cubic Hermite spline, hundreds of spans long) is
//! refined in a different order: its first sum takes the rule of half
//! the nodes on one panel per span, the second the full rule on the same
//! panel, and only the third twice the panels. A span that is one of
//! [`MANY_STRETCHES`] or more is short against anything the integrand
//! does, and the eight-node rule is far beyond 1e-12 on it already; the
//! sixteen-node rule is then its check, at a quarter of the nodes two
//! sums over doubled panels would take. Two Gauss–Legendre sums of
//! different order on a panel converge at their own rates, so their
//! agreement certifies the finer as the doubled panels' would; where the
//! sum does not settle the panels double as for any other edge. Whatever
//! the order, the panels across the face (the inner integrals) are
//! doubled with every sum, so that the two sums that agree differ in
//! everything that is refined.
//!
//! **Part by part.** The boundary integral is a sum of one term per
//! sample, each from the same reference, so it is the sum of one integral
//! per piece of the boundary, and (unless the boundary passes through two
//! poles, where a term belongs to the whole boundary, or the face is a
//! plane bounded by lines and circles, exact at once) each piece is
//! refined on its own: a piece whose two sums in a row agree is not summed
//! again, and the face is settled when every piece is. On a surface closed
//! both ways a loop is unrolled as a whole, and the parts are the loops. A
//! part's sums are held to their own allowance ([`CONVERGED`] of their
//! size, and the rounding: [`Outline::noise`] of what was summed for the
//! part), and the face to the sum of those: the face's sum is the sum of
//! the parts' and carries the rounding of each.
//!
//! **Panel by panel** (2026-10-17). Within a piece the panels are refined
//! adaptively ([`cell_by_cell`]): where a piece's sums do not agree, each
//! of its panels is held to its own halves, and only those that disagree
//! are halved again, past the seventh doubling if need be. An integrand
//! that gathers on a short stretch of an edge (a rational patch with its
//! weights a thousand apart puts most of its area on a millionth of an
//! edge next to a heavy corner) takes twenty halvings there and none
//! elsewhere, where doubling every panel ran out of doublings and budget
//! alike. A piece is given up when more of its panels are open at once
//! than it had at the seventh doubling: its sums then scatter wherever
//! they are looked at, which finer panels do not help. (Not on a surface
//! closed both ways, whose loops are unrolled as wholes and doubled as
//! before, nor where the boundary goes together.)
//!
//! **The work is bounded.** Every doubling is four times the work of the
//! one before on a spline surface (twice the boundary panels, each node
//! with twice the inner panels), and all seven on a boundary of spline
//! edges take minutes. So a face is also given up, with the same error,
//! once it has taken [`BUDGET`] evaluations of the integrand, those on a
//! spline surface counted for [`FREEFORM`] and the projection of a
//! boundary point for [`PROJECTION`]. That is eight times what the
//! slowest face that settles takes in the test suites of this crate, the
//! test kit and the kernel (4.6 million on an analytic surface; 1.0
//! million, counted for eight each, on a spline surface), and a hundred
//! times what a face takes that goes through all seven doublings on an
//! analytic surface. It is a count and not a clock, so that the same
//! body is measured or refused alike on every machine; on a spline
//! surface it comes to some twenty seconds unoptimised.
//!
//! A fixed budget does not fit a face of any size: the first sums of a
//! lampshade with 48 holes take 2.3 budgets alone, and the whole face
//! 10.7, each part settling at its second sum. So a face refined part by
//! part takes its first sums (the work of the boundary at its coarsest: a
//! fixed number of nodes per knot span of each edge, each with a fixed
//! number of inner panels per knot span of the surface, so in proportion
//! to the size of the face) up to [`FIRST_SUMS_WORK`], sixteen budgets, and is
//! then given up once it has taken [`PARTS_WORK`] times what those took,
//! or [`BUDGET`] where that is more. The bound grows with the face up to
//! a ceiling, never with what its sums do: a part that never settles
//! stops the face at that cap.
//!
//! **Following the boundary.** Every node of a boundary panel is a point
//! of an edge curve, projected onto the surface for its parameters. On a
//! Bézier or NURBS surface a projection from nothing is a search over the
//! whole surface (hundreds of evaluations), and a spline edge that
//! follows the curve in which the surface meets another has hundreds of
//! knot spans, so thousands of nodes a doubling. Nodes along one piece of
//! an edge are close together, so each is projected from where the last
//! one landed ([`Surface::project_from`], a few evaluations), and the
//! search is run only for the first node of a piece and wherever the
//! short way did not plainly land on the node's foot: the point found
//! must be within [`ON_SURFACE`] of the body's size from the node (an
//! edge lies on its face within the modelling tolerance, which is finer
//! than that), with the node's offset from it square to both tangents to
//! rounding ([`SQUARE`]). Two such feet of one node are the same point,
//! as long as the surface curves with radii larger than that distance and
//! does not come that close to itself: the chord between two feet would
//! have to run along the normal, and a chord of a surface that short runs
//! along the surface. So the short way gives the parameters the search
//! would, to rounding, or is not taken. The same goes for the places
//! along an edge that are looked at for crossings of knot lines.

use crate::{Body, BodyKind, Coedge, Face};
use craft3d_geom::{Curve3, GeomError, Sphere, Surface};
use craft3d_math::quadrature::Rule;
use craft3d_math::{Bbox3, Frame, Vec3};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MassProperties {
    /// Total surface area.
    pub area: f64,
    /// Enclosed volume; 0 for a sheet. Negative means the faces point
    /// inward.
    pub volume: f64,
    /// Centre of volume; `None` for a sheet or a zero volume.
    pub centroid: Option<Vec3>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MassError {
    /// A face refers to a loop or edge that does not exist; run `check`.
    Invalid {
        face: usize,
    },
    /// A face is not bounded: no loops on an open surface, or a loop that
    /// winds around a surface with no end on that side.
    UnboundedFace {
        face: usize,
    },
    /// A configuration the integrator does not handle yet.
    Unsupported {
        face: usize,
        what: &'static str,
    },
    Geometry {
        face: usize,
        error: GeomError,
    },
    /// A face on a rational surface whose weights within one span are
    /// further apart than any reparametrisation evens out, by more than
    /// the measure is held for ([`MAX_UNEVEN`]): `ratio` is how far.
    UnevenWeights {
        face: usize,
        ratio: f64,
    },
}

impl fmt::Display for MassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MassError::Invalid { face } => write!(
                f,
                "face {face} refers to a loop or edge that does not exist; check the body first"
            ),
            MassError::UnboundedFace { face } => {
                write!(f, "face {face} is not bounded by its loops")
            }
            MassError::Unsupported { face, what } => {
                write!(f, "face {face}: not supported yet: {what}")
            }
            MassError::Geometry { face, error } => write!(f, "face {face}: {error}"),
            MassError::UnevenWeights { face, ratio } => write!(
                f,
                "face {face}: not supported yet: rational weights {ratio:.3e} apart within one span after evening them out by a reparametrisation (at most {MAX_UNEVEN:e} is measured)"
            ),
        }
    }
}

impl std::error::Error for MassError {}

/// How many sums there are, and how many of them are the area, the volume
/// and the moments (the first of [`Sums::parts`]).
const SUMS: usize = 11;
const FIRST_SUMS: usize = 5;

/// The integrands at once: area, volume, the three moments, and, when
/// asked for ([`inertia`]), the six second moments: `∫x²`, `∫y²`, `∫z²`,
/// `∫xy`, `∫yz`, `∫zx` over the volume (zero otherwise).
#[derive(Clone, Copy, Debug, Default)]
struct Sums {
    area: f64,
    volume: f64,
    moment: Vec3,
    second: [f64; 6],
}

impl Sums {
    fn add_scaled(&mut self, o: Sums, s: f64) {
        self.area += o.area * s;
        self.volume += o.volume * s;
        self.moment += o.moment * s;
        for (mine, theirs) in self.second.iter_mut().zip(o.second) {
            *mine += theirs * s;
        }
    }

    fn parts(self) -> [f64; SUMS] {
        let [xx, yy, zz, xy, yz, zx] = self.second;
        [
            self.area,
            self.volume,
            self.moment.x,
            self.moment.y,
            self.moment.z,
            xx,
            yy,
            zz,
            xy,
            yz,
            zx,
        ]
    }
}

/// `f(u, v)` for the five integrals, and how large each is before
/// anything cancels: the position times the length of the normal where
/// `f` has their dot product, which is zero all over a cone with its apex
/// at the origin, and the whole normal where it has one component, which
/// is rounding all over a face that a coordinate axis lies in.
///
/// Positions are measured from `about`, the middle of the body: the
/// volume of a body is what is left of terms the size of a face's area
/// times its distance from that point, and for a small body far from the
/// origin of the model that distance would be the whole way there (a
/// prism of 1e-10 at 2 from the origin: 4e5 times its volume cancelling,
/// and a volume good to 1e-6 where its vertices are good to 1e-11).
///
/// With `second`, also the second moments: by the divergence theorem
/// `∫x² dV = ∮ x³/3 n_x dA` and `∫xy dV = ∮ x²y/2 n_x dA`, and so on
/// around the axes; their sizes are the same products of the absolute
/// coordinates and the length of the normal.
fn integrand(surface: &Surface, u: f64, v: f64, about: Vec3, second: bool) -> Total {
    let (at, su, sv) = surface.derivatives(u, v);
    let s = at - about;
    let n = su.cross(sv);
    let (seconds, second_sizes) = if second {
        // Each offset counted as at least its share of the coordinate,
        // as for the first moments.
        let [ax, ay, az] = [
            s.x.abs() + POINT_ROUNDING * at.x.abs(),
            s.y.abs() + POINT_ROUNDING * at.y.abs(),
            s.z.abs() + POINT_ROUNDING * at.z.abs(),
        ];
        let l = n.length();
        (
            [
                s.x * s.x * s.x * n.x / 3.0,
                s.y * s.y * s.y * n.y / 3.0,
                s.z * s.z * s.z * n.z / 3.0,
                s.x * s.x * s.y * n.x * 0.5,
                s.y * s.y * s.z * n.y * 0.5,
                s.z * s.z * s.x * n.z * 0.5,
            ],
            [
                ax * ax * ax * l / 3.0,
                ay * ay * ay * l / 3.0,
                az * az * az * l / 3.0,
                ax * ax * ay * l * 0.5,
                ay * ay * az * l * 0.5,
                az * az * ax * l * 0.5,
            ],
        )
    } else {
        ([0.0; 6], [0.0; 6])
    };
    let sum = Sums {
        area: n.length(),
        volume: s.dot(n) / 3.0,
        moment: Vec3::new(s.x * s.x * n.x, s.y * s.y * n.y, s.z * s.z * n.z) * 0.5,
        second: seconds,
    };
    Total {
        sum,
        size: Sums {
            second: second_sizes,
            area: n.length(),
            // The offset from `about` is known no better than the point
            // is, a few units in the last place of its coordinates: that
            // is [`CONVERGED`] of a thousandth of them, which each offset
            // is therefore counted as being at least.
            volume: (s.length() + POINT_ROUNDING * at.length()) * n.length() / 3.0,
            moment: Vec3::new(
                (s.x.abs() + POINT_ROUNDING * at.x.abs()).powi(2),
                (s.y.abs() + POINT_ROUNDING * at.y.abs()).powi(2),
                (s.z.abs() + POINT_ROUNDING * at.z.abs()).powi(2),
            ) * (0.5 * n.length()),
        },
    }
}

/// The share of a point's coordinates that its offset from the middle of
/// the body counts for at least, in what a sum's terms are worth before
/// they cancel (see [`integrand`]).
const POINT_ROUNDING: f64 = 1e-3;
const NODES: usize = 16;
/// Whole turns an unrolled inner integral may span on a spline surface
/// that closes on itself, at most: an unrolled parameter stays within a
/// few turns.
const MAX_TURNS: f64 = 8.0;
/// Panels for an inner integral, at least.
const INNER_PANELS: usize = 2;
/// And at most: an unrolled parameter stays within a few turns.
const MAX_INNER_PANELS: usize = 64;
/// Panels per knot stretch of an inner integral on a spline surface, at
/// most: as many as the last doubling of the boundary asks for
/// (`2^MAX_DOUBLINGS`), so that every doubling refines them too. (Until
/// 2026-10-17 they were held to [`MAX_INNER_PANELS`], so the last two sums
/// had the same inner panels and could agree in what those missed: a
/// cylinder whose rational weights were skewed a thousandfold along its
/// axis measured 7e-6 short, and settled.)
const MAX_SPLINE_INNER_PANELS: usize = 1 << MAX_DOUBLINGS;
/// How often the panels along the boundary are doubled, starting from half
/// of what an edge needs, before a face whose sums still change is given
/// up; where the boundary is refined panel by panel ([`cell_by_cell`]),
/// how often a panel is halved with its inner panels doubled along.
const MAX_DOUBLINGS: u32 = 7;
/// Panels along one edge, at most, however often they are doubled.
const MAX_PANELS: usize = 1 << 16;
/// Two sums in a row agree when they differ by no more than this share of
/// the size of what was summed: a thousand times the rounding of the sums,
/// and, the sums converging geometrically, far more than the second of the
/// two is off by.
const CONVERGED: f64 = 1e-12;
/// A face of a closed surface whose area is a smaller share than this of
/// what was summed for it may as well be the whole surface less that: the
/// two differ by the sign of a sum that is zero to rounding.
const TOO_THIN: f64 = 1e-9;
/// An edge that passes this close to an apex, relative to how far the
/// face's boundary reaches from it, passes through it, and is cut there so
/// that the corner it has in parameter space falls between two panels.
/// Taking a near miss for a touch costs nothing: the cut is in the
/// panels, not in what is integrated.
const AT_APEX: f64 = 1e-6;
/// Points along each edge at which a sphere face's boundary is looked at
/// to choose its frame.
const FRAME_SAMPLES: u32 = 32;
/// A spline edge with this many stretches between its knots (and knot
/// crossings) or more is summed with the coarse rule first (see the
/// module documentation).
const MANY_STRETCHES: usize = 8;
/// A boundary point this share of the body's size from the point found
/// for it on a freeform surface is on the surface there, for the warm
/// start of the projections (see the module documentation). An edge lies
/// on its face within the modelling tolerance, which `mass_properties` is
/// not told; the kernel's is a millionth of the model's scale
/// (`Tolerance::default`), and the body's size is the scale this
/// integrator can see. Ten times that is well within the radius any
/// surface curves with that the search projection handles at all, and a
/// point further off than this only takes the long way.
const ON_SURFACE: f64 = 1e-5;
/// A residual along a tangent of this many units in the last place of the
/// point's size and the surface's reach is rounding: the walk of a
/// projection stops when its steps are four of them, and the residual of
/// a true foot is computed to a few.
const SQUARE: f64 = 64.0 * f64::EPSILON;
/// The work one face may take before it is given up as one whose sums do
/// not settle (see the module documentation), in evaluations of the
/// integrand on an analytic surface.
const BUDGET: u64 = 1 << 26;
/// How many times the work of its first sums a face whose boundary is
/// refined part by part may take, where that is more than [`BUDGET`]
/// ([`part_by_part`]). A part that settles at the second sum costs five
/// times its first (each sum four times the one before), at the third
/// twenty-one: this lets every part go to its third sum, and some deeper.
/// Measured: the lampshade's faces of 48 and 53 holes take 4.2 to 4.6
/// times their first sums.
const PARTS_WORK: u64 = 64;
/// The most the first sums of a face refined part by part may take
/// ([`part_by_part`]): sixteen budgets. Measured: the first sums of the
/// lampshade's faces of 48 and 53 holes take 2.3 and 2.4. Past it the
/// face is given up as any other whose sums do not settle, and with it
/// the whole face's work is at most [`PARTS_WORK`] times this.
const FIRST_SUMS_WORK: u64 = 16 * BUDGET;
/// What the projection of a boundary point counts for in a face's work,
/// in evaluations of the integrand.
const PROJECTION: u64 = 8;
/// What an evaluation on a Bézier or NURBS surface counts for: measured,
/// it takes fifteen times as long as one on a cone.
const FREEFORM: u64 = 8;

/// The work one face has taken: evaluations of [`integrand`], and
/// projections of boundary points at [`PROJECTION`] of them each, all
/// [`FREEFORM`] times over on a freeform surface. A count, not a clock:
/// the same face is given up at the same place on every machine.
#[derive(Clone)]
struct Work {
    spent: u64,
    /// What one evaluation on the face's surface counts for.
    each: u64,
    /// The point positions are measured from in the volume and the
    /// moments ([`integrand`]).
    about: Vec3,
    /// Whether the second moments are summed too ([`inertia`]).
    second: bool,
    /// In u and in v: the domain of a spline surface that closes on
    /// itself, when its boundary is unrolled ([`Plan::Unrolled`]) and an
    /// inner integral may run beyond the domain, by whole turns.
    wrap: [Option<(f64, f64)>; 2],
    /// Past this much work the face is given up: [`BUDGET`], unless
    /// [`part_by_part`] sets it otherwise.
    limit: u64,
    /// What the area and the volume of the sums settled so far were held
    /// to: per sum taken, the difference its last two sums were allowed
    /// ([`Total::allowance`]). [`mass_bound`] reports their sum.
    held: [f64; 2],
}

impl Work {
    #[cfg(test)]
    fn new(surface: &Surface) -> Self {
        Self::about(surface, Vec3::ZERO, false)
    }

    fn about(surface: &Surface, about: Vec3, second: bool) -> Self {
        let freeform = matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_));
        Work {
            spent: 0,
            each: if freeform { FREEFORM } else { 1 },
            about,
            second,
            wrap: [None, None],
            limit: BUDGET,
            held: [0.0; 2],
        }
    }

    /// Counts what `total` was held to, with `noise` its share of
    /// rounding, among what the face's sums were held to.
    fn hold(&mut self, total: &Total, noise: f64) {
        let [area, volume] = total.allowance(noise);
        self.held[0] += area;
        self.held[1] += volume;
    }

    fn add(&mut self, evaluations: u64) {
        self.spent = self
            .spent
            .saturating_add(evaluations.saturating_mul(self.each));
    }

    /// Whether the face has taken more than it may ([`Work::limit`]).
    fn spent_up(&self) -> bool {
        self.spent > self.limit
    }
}

/// The error of a face whose sums do not agree within the doublings, or
/// within the work, it is given.
fn unsettled(face: usize) -> MassError {
    MassError::Unsupported {
        face,
        what: "a boundary integral that does not settle: the boundary is too intricate, or too close to where the surface's parametrisation collapses",
    }
}

/// Projects a run of nearby points onto a surface, each from where the
/// last one landed, taking the short way only where it is shown to give
/// the search's answer (see the module documentation).
struct Follow<'a> {
    surface: &'a Surface,
    /// Within this distance of the surface a point is on it; zero when
    /// nothing is to be taken from the last point (an analytic surface,
    /// whose projection is closed-form anyway).
    touch: f64,
    last: Option<(f64, f64)>,
}

impl<'a> Follow<'a> {
    fn new(surface: &'a Surface, touch: f64) -> Self {
        let freeform = matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_));
        Follow {
            surface,
            touch: if freeform { touch } else { 0.0 },
            last: None,
        }
    }

    /// The parameters of the nearest point of `p`, and the surface and
    /// its tangents there.
    #[allow(clippy::type_complexity)]
    fn project(&mut self, p: Vec3) -> Result<((f64, f64), (Vec3, Vec3, Vec3)), GeomError> {
        if self.touch > 0.0
            && let Some(start) = self.last
            && let Ok((u, v)) = self.surface.project_from(p, start)
        {
            let there = self.surface.derivatives(u, v);
            if self.is_foot(p, there) {
                self.last = Some((u, v));
                return Ok(((u, v), there));
            }
        }
        let (u, v) = self.surface.project(p)?;
        let there = self.surface.derivatives(u, v);
        self.last = Some((u, v));
        Ok(((u, v), there))
    }

    /// Whether the surface point `s` with tangents `su`, `sv` is the foot
    /// of `p`: within `touch` of it, with `p − s` square to both tangents
    /// to rounding. The domain is finite: the surface is freeform.
    fn is_foot(&self, p: Vec3, (s, su, sv): (Vec3, Vec3, Vec3)) -> bool {
        let r = p - s;
        let ((u0, u1), (v0, v1)) = self.surface.domain();
        let reach = su.length() * (u1 - u0) + sv.length() * (v1 - v0);
        let slack = SQUARE * (p.length() + reach);
        let square = |tangent: Vec3| r.dot(tangent).abs() <= slack * tangent.length();
        // Not `>`: a NaN is no foot.
        r.length() <= self.touch && square(su) && square(sv)
    }
}

/// The direction an inner integral runs in.
#[derive(Clone, Copy, Debug)]
enum Along {
    U,
    V,
}

impl Along {
    fn same(self, other: Along) -> bool {
        matches!((self, other), (Along::U, Along::U) | (Along::V, Along::V))
    }
}

/// A sum, and the sum of the sizes of its terms (see [`integrand`]): what
/// a difference between two sums is measured against.
#[derive(Clone, Copy, Debug, Default)]
struct Total {
    sum: Sums,
    size: Sums,
}

impl Total {
    fn add(&mut self, term: Total, weight: f64) {
        self.sum.add_scaled(term.sum, weight);
        self.size.add_scaled(term.size, weight.abs());
    }

    /// Whether `finer`, the same sum with twice the panels, says the same,
    /// given the share `noise` of every term that is rounding.
    fn agrees_with(&self, finer: &Total, noise: f64) -> bool {
        self.agrees_in(finer, noise, SUMS)
    }

    /// [`Self::agrees_with`] in the area, the volume and the moments only:
    /// what it says of sums without the second moments, which are zero
    /// there and agree.
    fn first_agrees_with(&self, finer: &Total, noise: f64) -> bool {
        self.agrees_in(finer, noise, FIRST_SUMS)
    }

    /// How far the area and the volume of two sums in a row may differ
    /// and agree, given the share `noise` of every term that is rounding:
    /// what the finer of them is held to.
    fn allowance(&self, noise: f64) -> [f64; 2] {
        let share = CONVERGED + noise;
        [share * self.size.area, share * self.size.volume]
    }

    /// Whether the first `parts` of the sums agree.
    fn agrees_in(&self, finer: &Total, noise: f64, parts: usize) -> bool {
        let size = finer.size.parts();
        self.sum
            .parts()
            .into_iter()
            .zip(finer.sum.parts())
            .zip(size)
            .take(parts)
            // Not `>`: a NaN agrees with nothing.
            .all(|((a, b), size)| (a - b).abs() <= (CONVERGED + noise) * size)
    }
}

/// `∫ f dt` from `from` to `to` along one parameter, the other one fixed.
///
/// On an analytic surface the integrand is a polynomial or trigonometric
/// and the panels are what makes the rule exact to rounding. On a spline
/// surface it is smooth only between the surface's knots in the direction
/// of `t`, so the panels end at the knots in between, and nothing says how
/// many a stretch needs: half of [`INNER_PANELS`] doubled `doublings`
/// times, along with the panels of the boundary, so that the sums of the
/// boundary only agree once these have settled too.
fn inner(
    surface: &Surface,
    rule: &Rule,
    along: Along,
    fixed: f64,
    (from, to): (f64, f64),
    doublings: u32,
    work: &mut Work,
) -> Total {
    let work_wrap = work.wrap;
    // The fixed parameter too may be unrolled beyond the domain.
    let across = match along {
        Along::U => work_wrap[1],
        Along::V => work_wrap[0],
    };
    let fixed = match across {
        Some((low, high)) if !(fixed >= low && fixed <= high) => {
            let back = low + (fixed - low).rem_euclid(high - low);
            if back.is_finite() { back } else { fixed }
        }
        _ => fixed,
    };
    let mut sum = Total::default();
    // `map`: the panel is one of the Möbius parameter `s` of a stretch,
    // `(start, width, β)` (see [`moebius`]); without it, of `t` itself.
    let mut panel = |start: f64, end: f64, sign: f64, map: Option<(f64, f64, f64)>| {
        work.add(NODES as u64);
        for (s, w) in rule.points(start, end) {
            let (t, w) = match map {
                Some(map) => moebius_node(s, w, map),
                None => (s, w),
            };
            let (u, v) = match along {
                Along::U => (t, fixed),
                Along::V => (fixed, t),
            };
            sum.add(integrand(surface, u, v, work.about, work.second), w * sign);
        }
    };
    let knots: &[f64] = match (surface, along) {
        (Surface::Nurbs(s), Along::U) => s.knots_u(),
        (Surface::Nurbs(s), Along::V) => s.knots_v(),
        _ => &[],
    };
    if matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
        let (low, high, sign) = if from <= to {
            (from, to, 1.0)
        } else {
            (to, from, -1.0)
        };
        // Every doubling refines the inner integrals as well, up to the
        // last: two sums with the same inner panels would agree in what
        // those panels miss.
        let count = ((INNER_PANELS << doublings.min(8)) / 2).clamp(1, MAX_SPLINE_INNER_PANELS);
        let mut stretch = |start: f64, end: f64| {
            if let Some(beta) = moebius(surface, along, fixed, (start, end)) {
                let width = 1.0 / count as f64;
                for k in 0..count {
                    let at = width * k as f64;
                    panel(at, at + width, sign, Some((start, end - start, beta)));
                }
                return;
            }
            let width = (end - start) / count as f64;
            for k in 0..count {
                let at = start + width * k as f64;
                panel(at, at + width, sign, None);
            }
        };
        let mut between = |low: f64, high: f64| {
            let mut start = low;
            for &knot in knots {
                if knot > start && knot < high {
                    stretch(start, knot);
                    start = knot;
                }
            }
            stretch(start, high);
        };
        let wrap = match along {
            Along::U => work_wrap[0],
            Along::V => work_wrap[1],
        };
        match wrap {
            // An unrolled parameter beyond the domain of a surface that
            // closes on itself: each turn is summed in the domain.
            Some((d0, d1)) if low < d0 || high > d1 => {
                let period = d1 - d0;
                let first = ((low - d0) / period).floor();
                let last = ((high - d0) / period).ceil();
                if !(first.is_finite() && last.is_finite() && last - first <= MAX_TURNS) {
                    // Nothing summed is a sum that never settles.
                    return Total {
                        sum: Sums {
                            area: f64::NAN,
                            ..Sums::default()
                        },
                        size: Sums::default(),
                    };
                }
                let mut turn = first;
                while turn < last {
                    let shift = period * turn;
                    let (a, b) = (low.max(d0 + shift), high.min(d1 + shift));
                    if b > a {
                        between((a - shift).max(d0), (b - shift).min(d1));
                    }
                    turn += 1.0;
                }
            }
            _ => between(low, high),
        }
        return sum;
    }
    let periodic = match along {
        Along::U => surface.period_u().is_some(),
        Along::V => surface.period_v().is_some(),
    };
    // Around a closed direction the integrand is trigonometric and the
    // span may be several turns: at most a quarter turn per panel.
    let count = if periodic {
        (((to - from).abs() / FRAC_PI_2).ceil() as usize).clamp(INNER_PANELS, MAX_INNER_PANELS)
    } else {
        INNER_PANELS
    };
    let width = (to - from) / count as f64;
    for k in 0..count {
        let start = from + width * k as f64;
        panel(start, start + width, 1.0, None);
    }
    sum
}

/// The Möbius reparametrisation that makes the end weights of a stretch
/// of an inner integral equal, on a rational surface: `β`, with the
/// stretch `[start, end]` reached from `s ∈ [0, 1]` by
/// `t = start + (end − start) τ`, `τ = β s / ((1 − s) + β s)`, and
/// `dt = (end − start) β / ((1 − s) + β s)² ds`. `None` where that is the
/// identity (`β = 1`: a polynomial surface, or end weights that are
/// equal) or cannot be had.
///
/// Why. Within a knot span the surface along the stretch is
/// `A(t) / w(t)`, with `w` a polynomial of the degree `q` in that
/// direction whose Bernstein coefficients on the stretch, `w₀ … w_q`, are
/// positive, and its ends are its values at the ends. Where they differ
/// by a large factor, `w` has a zero close to the light end, outside the
/// stretch (for a linear `w` at the stretch over the factor), the
/// integrand varies on that scale, and Gauss–Legendre sums over panels
/// wider than that converge slowly or, at the cap of the panels, not at
/// all. In `τ` the Bernstein coefficients of the denominator are
/// `w_i β^i` up to a common factor (`B_i^q(τ) = β^i B_i^q(s) / D^q`,
/// `D = (1 − s) + β s`), so with `β = (w₀ / w_q)^{1/q}` its ends are
/// equal: the surface along the stretch is drawn as evenly as its weights
/// allow. The integral is the same (a change of variable, exact); only
/// where the nodes fall changes. For weights of a product form
/// (`w_ij = a_i b_j`, every surface of revolution and every analytic
/// twin made with weights skewed by `c^i`) it is the reparametrisation
/// that takes the skew out again, so the sums are those of the evenly
/// drawn surface; for weights of no such form it makes each line's own
/// ends equal, which a linear direction makes polynomial outright.
fn moebius(surface: &Surface, along: Along, fixed: f64, (start, end): (f64, f64)) -> Option<f64> {
    let Surface::Nurbs(s) = surface else {
        return None;
    };
    let (du, dv) = s.degrees();
    let (degree, w0, w1) = match along {
        Along::U => (du, s.weight_at(start, fixed), s.weight_at(end, fixed)),
        Along::V => (dv, s.weight_at(fixed, start), s.weight_at(fixed, end)),
    };
    // Equal ends (every polynomial surface) take the plain nodes. (Their
    // sums still change in the last digits by the inner panels doubling
    // to the last and by [`cell_by_cell`].)
    if w0 == w1 || degree == 0 {
        return None;
    }
    let ratio = w0 / w1;
    let beta = match degree {
        1 => ratio,
        2 => ratio.sqrt(),
        _ => ratio.powf(1.0 / degree as f64),
    };
    // Not `<=`: a NaN gives no map. Far outside `[1e-100, 1e100]` the map
    // squeezes every node into rounding of an end; the plain sums are
    // then the better ones (and settle or are refused).
    (beta > MIN_BETA && beta < 1.0 / MIN_BETA && beta != 1.0).then_some(beta)
}

/// The factors of a Möbius map that are not taken ([`moebius`]).
const MIN_BETA: f64 = 1e-100;

/// A node `s` of the Möbius parameter and its weight `w`, as the place
/// `t` on the stretch `(from, width)` and the weight in `t` (see
/// [`moebius`]): `dt/ds = width β / D²`, as `(β / D) / D` so that it
/// neither overflows nor vanishes for any `β` [`moebius`] gives.
fn moebius_node(s: f64, w: f64, (from, width, beta): (f64, f64, f64)) -> (f64, f64) {
    let d = (1.0 - s) + beta * s;
    (from + width * (beta * s / d), w * width * ((beta / d) / d))
}

/// A quadrature panel along an edge: a parameter range, and whether it
/// takes the coarse rule (half the nodes) rather than the full one.
#[derive(Clone, Copy, Debug)]
struct Panel {
    start: f64,
    end: f64,
    coarse: bool,
}

/// The quadrature panels along a stretch of an edge, `low` to `high`:
/// half of what the edge needs, doubled `doublings` times.
///
/// What an edge needs: panels of an eighth of a turn at most along a
/// circle or an ellipse, two along a line, and two to every stretch of a
/// spline. A panel ends wherever the integrand has a kink: at the knots
/// of a spline edge, which is smooth only between them, and at `crossings`,
/// where the edge crosses a knot line of a spline surface. A sum over
/// panels that straddle a kink converges too slowly to ever settle.
///
/// A spline edge of [`MANY_STRETCHES`] stretches or more starts one step
/// earlier: one panel a stretch with the coarse rule, then with the full
/// one, then doubled (the module documentation says why).
fn panels(
    curve: &Curve3,
    (low, high): (f64, f64),
    crossings: &[f64],
    doublings: u32,
) -> Vec<Panel> {
    let mut stops = vec![low];
    if let Curve3::Nurbs(c) = curve {
        stops.extend(c.knots().iter().filter(|&&k| k > low && k < high));
    }
    stops.extend(crossings.iter().filter(|&&t| t > low && t < high));
    stops.push(high);
    stops.sort_by(f64::total_cmp);
    stops.dedup();
    let stretches = stops.len().saturating_sub(1).max(1);
    let many = matches!(curve, Curve3::Nurbs(_)) && stretches >= MANY_STRETCHES;
    let (double, coarse) = if many {
        (
            1_usize << doublings.saturating_sub(1).min(16),
            doublings == 0,
        )
    } else {
        (1_usize << doublings.min(16), false)
    };
    let mut panels = Vec::new();
    for pair in stops.windows(2) {
        let &[from, to] = pair else { continue };
        let needed = match curve {
            Curve3::Line(_) | Curve3::Nurbs(_) => 1,
            Curve3::Circle(_) | Curve3::Ellipse(_) => {
                (((to - from) / FRAC_PI_2).ceil() as usize).clamp(1, 32)
            }
        };
        let count = needed
            .saturating_mul(double)
            .min(MAX_PANELS / stretches)
            .max(1);
        let width = (to - from) / count as f64;
        for k in 0..count {
            let start = from + width * k as f64;
            panels.push(Panel {
                start,
                end: start + width,
                coarse,
            });
        }
    }
    panels
}

/// The distinct knots of a spline surface strictly inside its domain, in
/// u and in v: its knot lines, across which it is not smooth.
fn knot_lines(surface: &Surface) -> (Vec<f64>, Vec<f64>) {
    let Surface::Nurbs(s) = surface else {
        return (Vec::new(), Vec::new());
    };
    let ((u0, u1), (v0, v1)) = s.domain();
    let inside = |knots: &[f64], (start, end): (f64, f64)| {
        let mut lines: Vec<f64> = knots
            .iter()
            .copied()
            .filter(|&k| k > start && k < end)
            .collect();
        lines.dedup();
        lines
    };
    (inside(s.knots_u(), (u0, u1)), inside(s.knots_v(), (v0, v1)))
}

/// Looks along an edge at this many places per stretch between its knots
/// for crossings of the surface's knot lines.
const CROSSING_STEPS: u32 = 8;

/// Where the stretch `low` to `high` of an edge crosses a knot line of
/// the spline surface it lies on (or the seam of one that closes on
/// itself): parameters of the edge, in no particular order.
///
/// Found by a change of side between neighbouring places along the edge
/// and narrowed by bisection. An edge that runs along a knot line does
/// not cross it; one that touches a line and turns back between two of
/// the places looked at is missed, and its sums then do not settle.
fn knot_crossings(
    curve: &Curve3,
    surface: &Surface,
    (low, high): (f64, f64),
    touch: f64,
    (split, seams): (Option<(Along, f64)>, bool),
) -> Result<Vec<f64>, GeomError> {
    let (mut lines_u, mut lines_v) = knot_lines(surface);
    // The line a face through two poles is divided along is one more
    // that its boundary's panels end at.
    match split {
        Some((Along::V, v)) => lines_v.push(v),
        Some((Along::U, u)) => lines_u.push(u),
        None => {}
    }
    let mut crossings = Vec::new();
    // `seams`: the surface closes on itself, and the boundary's panels
    // end where it goes over the seam as well. On a freeform surface they
    // end where the edge leaves the domain too (see below).
    let freeform = matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_));
    let lined = !(lines_u.is_empty() && lines_v.is_empty() && !seams);
    if !(lined || freeform) || high <= low || (high - low).is_nan() {
        return Ok(crossings);
    }
    let ((u0, u1), (v0, v1)) = surface.domain();
    // Each place from the one looked at before it: the next place along,
    // or the middle of a stretch that is being narrowed.
    let mut follow = Follow::new(surface, touch);
    let mut at = |t: f64| follow.project(curve.eval(t)).map(|(uv, _)| uv);
    // Looks of its own for where the edge runs off the domain, so that
    // the places above are projected as they are without it.
    let mut aside = Follow::new(surface, touch);
    // A border that is one point (a pole) has no outside to run off to:
    // the parameter along it says nothing there.
    let poles: Vec<(bool, f64)> = if freeform {
        craft3d_geom::pole::poles(surface)
            .iter()
            .map(|pole| (pole.across_v(), pole.at(surface)))
            .collect()
    } else {
        Vec::new()
    };
    // The places to look at: evenly between the edge's own knots.
    let mut places = vec![low];
    let mut ends = vec![low];
    if let Curve3::Nurbs(c) = curve {
        ends.extend(c.knots().iter().filter(|&&k| k > low && k < high));
    }
    ends.push(high);
    ends.dedup();
    for pair in ends.windows(2) {
        let &[from, to] = pair else { continue };
        for k in 1..=CROSSING_STEPS {
            places.push(from + (to - from) * f64::from(k) / f64::from(CROSSING_STEPS));
        }
    }
    let mut before: Option<(f64, (f64, f64))> = None;
    for t in places {
        let here = at(t)?;
        if let Some((s, there)) = before {
            for (lines, from, to, (start, end), pick_u) in [
                (&lines_u, there.0, here.0, (u0, u1), true),
                (&lines_v, there.1, here.1, (v0, v1), false),
            ] {
                let width = end - start;
                let pick = |uv: (f64, f64)| if pick_u { uv.0 } else { uv.1 };
                if freeform {
                    for border in [start, end] {
                        if poles.contains(&(!pick_u, border)) {
                            continue;
                        }
                        let run: (_, _, &dyn Fn((f64, f64)) -> f64) = (curve, &mut aside, &pick);
                        if let Some(kink) = run_off(run, (s, t), (from, to), (border, width))? {
                            crossings.push(kink);
                        }
                    }
                }
                if !lined {
                    continue;
                }
                // The side of `x` the stretch started on, for bisection.
                let mut narrow = |starts: &dyn Fn(f64) -> bool| -> Result<(), GeomError> {
                    let (mut a, mut b) = (s, t);
                    for _ in 0..48 {
                        let middle = 0.5 * (a + b);
                        if !(middle > a && middle < b) {
                            break;
                        }
                        if starts(pick(at(middle)?)) {
                            a = middle;
                        } else {
                            b = middle;
                        }
                    }
                    crossings.push(0.5 * (a + b));
                    Ok(())
                };
                if (to - from).abs() > 0.5 * width {
                    // From one side of a seam to the other.
                    narrow(&|x| (x - from).abs() < (x - to).abs())?;
                    continue;
                }
                // On a line (within rounding) is on neither side of it.
                let clear = 1e-9 * width;
                for &line in lines.iter() {
                    let (d0, d1) = (from - line, to - line);
                    if (d0 > clear && d1 < -clear) || (d0 < -clear && d1 > clear) {
                        narrow(&|x| (x - line > 0.0) == (d0 > 0.0))?;
                    }
                }
            }
        }
        before = Some((t, here));
    }
    Ok(crossings)
}

/// Where an edge that lies within the tolerance of a freeform surface,
/// not on it, runs off the surface's domain between the places `s` and
/// `t` along it (its parameter `from` and `to` there, in the direction
/// `pick` takes). Past the border its points are projected onto the
/// border, so the parameter stops there with a kink, as it has where the
/// edge crosses a knot line: a fitted edge that ends on the border,
/// tangent to it, does that over its last thousandths, and the sums of
/// its panels then settle only by chance. Between a place clear of the
/// border (by more than `1e-9` of the domain's `width`) and one at it,
/// the kink is narrowed down by bisection. (`1e-9` is the clearance the
/// knot lines are held to: a chosen bound, not a derived one. Where the
/// parameter comes to the border quadratically the bisection ends up to
/// about `√1e-9` of its approach early, inside the stretch that is still
/// smooth; the panel end is then not at the kink, and a sum that does
/// not settle for it is refused as any other.) It is one only where the
/// point beyond it is held at the border: its parameter is the border's
/// to rounding and it is off the surface. An edge that only reaches the
/// border (at its end, or tangent to it), or comes to it where the
/// parameters say little (next to a pole), has none and is left as it
/// was. Nothing is projected unless one place is at the border and the
/// other clear of it.
#[allow(clippy::type_complexity)]
fn run_off(
    (curve, follow, pick): (&Curve3, &mut Follow<'_>, &dyn Fn((f64, f64)) -> f64),
    (s, t): (f64, f64),
    (from, to): (f64, f64),
    (border, width): (f64, f64),
) -> Result<Option<f64>, GeomError> {
    let clear = 1e-9 * width;
    let near = |x: f64| (x - border).abs() <= clear;
    if near(from) == near(to) {
        return Ok(None);
    }
    let mut at = |t: f64| follow.project(curve.eval(t)).map(|(uv, _)| pick(uv));
    // `a` clear of the border, `b` at it.
    let (mut a, mut b) = if near(to) { (s, t) } else { (t, s) };
    for _ in 0..48 {
        let middle = 0.5 * (a + b);
        if middle == a || middle == b {
            break;
        }
        if near(at(middle)?) {
            b = middle;
        } else {
            a = middle;
        }
    }
    let kink = 0.5 * (a + b);
    // Halfway on from it to the place at the border, the point is held
    // there: its parameter is the border's to rounding (a projection
    // stops on it exactly), and it is off the surface, beyond the border.
    // (A point of the surface at its border, such as one next to a pole
    // whose parameters say little, is not held.)
    let beyond = 0.5 * (kink + if near(to) { t } else { s });
    let p = curve.eval(beyond);
    let (uv, (foot, _, _)) = follow.project(p)?;
    let rounding = 4.0 * f64::EPSILON * border.abs().max(width);
    let held = (pick(uv) - border).abs() <= rounding
        && (p - foot).length() > SQUARE * (p.length() + foot.length());
    Ok(held.then_some(kink))
}

/// The frame a sphere face is measured in (see the module documentation):
/// the same sphere with its south pole as far from the face's boundary as
/// the candidates allow. `None` when the boundary has no finite point.
fn own_sphere(body: &Body, face: &Face, sphere: &Sphere) -> Option<Surface> {
    let (old, radius) = (*sphere.frame(), sphere.radius());
    let centre = old.origin();
    let mut points = Vec::new();
    let mut candidates: Vec<Vec3> = Vec::new();
    let mut circles = Vec::new();
    for loop_id in &face.loops {
        for coedge in &body.loop_(*loop_id)?.coedges {
            let edge = body.edge(coedge.edge)?;
            let (t0, t1) = edge.range;
            for k in 0..=FRAME_SAMPLES {
                let point = edge
                    .curve
                    .eval(t0 + (t1 - t0) * f64::from(k) / f64::from(FRAME_SAMPLES));
                if point.is_finite() {
                    points.push(point);
                }
            }
            if let Curve3::Circle(circle) = &edge.curve {
                circles.push(circle.frame().z());
            }
        }
    }
    // The middle of the boundary, when it has one.
    let mut mean = Vec3::ZERO;
    for point in &points {
        mean += (*point - centre) * (1.0 / points.len() as f64);
    }
    if mean.length() > 1e-3 * radius
        && let Some(direction) = mean.normalized()
    {
        candidates.push(direction);
    }
    candidates.extend(circles);
    candidates.extend([old.z(), old.x(), old.y()]);
    let diagonal = 1.0 / 3.0_f64.sqrt();
    for (x, y) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        candidates.push(old.vector(x, y, 1.0) * diagonal);
    }
    let mut best: Option<(f64, Vec3)> = None;
    for candidate in candidates {
        for north in [candidate, -candidate] {
            let south = centre - north * radius;
            let clear = points
                .iter()
                .map(|point| point.distance(south))
                .fold(f64::INFINITY, f64::min);
            // The first of equals: the order above is part of the result.
            if clear.is_finite() && best.is_none_or(|(most, _)| clear > most) {
                best = Some((clear, north));
            }
        }
    }
    let frame = Frame::from_z(centre, best?.1)?;
    Some(Surface::Sphere(Sphere::new(frame, radius).ok()?))
}

/// How near, as a share of the size of the control net, two rows of
/// control points must be to count as the same row.
const SAME_ROW: f64 = 1e-9;

/// Where a Bézier or NURBS surface closes on itself, in u and in v: the
/// parameter range across which it does, its domain in that direction.
/// It closes in a direction when its control net ends there on the row
/// it starts with (the same points to [`SAME_ROW`] of the net's size, and
/// weights in one proportion, so that the two rows are the same curve)
/// on clamped knots, and the rows in between are not all that one row
/// (which is a pole, not a seam). `[None, None]` for the analytic kinds,
/// which say how they close themselves (`Surface::period_u`).
fn spline_closure(surface: &Surface) -> [Option<(f64, f64)>; 2] {
    let (points, weights, (count_u, count_v), clamped): (&[Vec3], Option<&[f64]>, _, [bool; 2]) =
        match surface {
            Surface::Bezier(patch) => (
                patch.control_points(),
                None,
                (patch.degree_u() + 1, patch.degree_v() + 1),
                [true, true],
            ),
            Surface::Nurbs(s) => {
                let (degree_u, degree_v) = s.degrees();
                (
                    s.control_points(),
                    Some(s.weights()),
                    s.counts(),
                    [
                        clamped(s.knots_u(), degree_u),
                        clamped(s.knots_v(), degree_v),
                    ],
                )
            }
            _ => return [None, None],
        };
    if count_u < 2 || count_v < 2 || count_u.checked_mul(count_v) != Some(points.len()) {
        return [None, None];
    }
    let near = SAME_ROW * Bbox3::from_points(points.iter().copied()).diagonal();
    if !(near.is_finite() && near > 0.0) {
        return [None, None];
    }
    let at = |i: usize, j: usize| {
        let index = i.checked_mul(count_v)?.checked_add(j)?;
        let weight = match weights {
            Some(weights) => *weights.get(index)?,
            None => 1.0,
        };
        Some((*points.get(index)?, weight))
    };
    // Along `rows` rows of `across` points each, the k-th of row r being
    // `get(r, k)`.
    let closes = |rows: usize, across: usize, get: &dyn Fn(usize, usize) -> Option<(Vec3, f64)>| {
        let (Some((_, first)), Some((_, last))) = (get(0, 0), get(rows - 1, 0)) else {
            return false;
        };
        let mut moves = false;
        for k in 0..across {
            let (Some((p, w)), Some((q, x))) = (get(0, k), get(rows - 1, k)) else {
                return false;
            };
            // `w / first == x / last`, without dividing. Not `>`: a NaN
            // closes nothing.
            if !(p.distance(q) <= near
                && (w * last - x * first).abs() <= SAME_ROW * (w * last).abs())
            {
                return false;
            }
            for row in 1..rows - 1 {
                moves |= get(row, k).is_some_and(|(r, _)| r.distance(p) > near);
            }
        }
        moves
    };
    let ((u0, u1), (v0, v1)) = surface.domain();
    let range = |closes: bool, low: f64, high: f64| {
        (closes && low.is_finite() && high.is_finite() && high > low).then_some((low, high))
    };
    let [clamped_u, clamped_v] = clamped;
    [
        range(clamped_u && closes(count_u, count_v, &at), u0, u1),
        range(
            clamped_v && closes(count_v, count_u, &|j, i| at(i, j)),
            v0,
            v1,
        ),
    ]
}

/// Whether a knot vector is clamped: its first and its last `degree + 1`
/// knots are one value each, so that the spline starts on its first row
/// of control points and ends on its last.
fn clamped(knots: &[f64], degree: usize) -> bool {
    let (Some(first), Some(last)) = (knots.first(), knots.last()) else {
        return false;
    };
    knots.len() > 2 * degree + 1
        && knots.iter().take(degree + 1).all(|k| k == first)
        && knots.iter().rev().take(degree + 1).all(|k| k == last)
}

/// Whether the boundary samples jump, between two in a row of one loop
/// (around its end too), by more than half of `width` in the parameter
/// `pick` gives: the boundary goes over the seam of a surface that closes
/// on itself in that parameter, or around it.
fn jumps(samples: &[Sample], pick: impl Fn(&Sample) -> f64, width: f64) -> bool {
    let mut start = 0;
    while let Some(first) = samples.get(start) {
        let end = samples
            .iter()
            .skip(start)
            .position(|s| s.loop_index != first.loop_index)
            .map_or(samples.len(), |offset| start + offset);
        let Some(block) = samples.get(start..end) else {
            break;
        };
        let around = block.last().zip(block.first());
        if block
            .windows(2)
            .filter_map(|pair| match pair {
                [a, b] => Some((a, b)),
                _ => None,
            })
            .chain(around)
            // Not `>`: a NaN is no jump.
            .any(|(a, b)| (pick(a) - pick(b)).abs() > 0.5 * width)
        {
            return true;
        }
        start = end;
    }
    false
}

/// Whether a face whose every edge its loops use twice, once each way (a
/// seam walked there and back: nothing else bounds it), is its whole
/// surface: the surface closes on itself or ends in a pole on every side
/// ([`closed_all_around`]).
fn seams_only(body: &Body, face: &Face) -> bool {
    let mut uses: Vec<(u32, bool)> = Vec::new();
    for loop_id in &face.loops {
        let Some(loop_) = body.loop_(*loop_id) else {
            return false;
        };
        uses.extend(loop_.coedges.iter().map(|c| (c.edge.0, c.reversed)));
    }
    if uses.is_empty() {
        return false;
    }
    uses.sort_unstable();
    // Sorted, each edge is then once forwards and once backwards, in a row.
    let paired = uses
        .chunks(2)
        .all(|pair| matches!(pair, [(a, false), (b, true)] if a == b));
    paired && closed_all_around(&face.surface)
}

/// Whether a surface closes on itself or ends in a pole on every side, so
/// that it bounds a region of space on its own: a sphere and a torus do; a
/// Bézier or NURBS surface when each of its directions closes
/// ([`spline_closure`]) or ends in a pole at both of its borders.
fn closed_all_around(surface: &Surface) -> bool {
    match surface {
        Surface::Sphere(_) | Surface::Torus(_) => true,
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            let [closes_u, closes_v] = spline_closure(surface);
            let poles = craft3d_geom::pole::poles(surface);
            // Both borders of constant v (`across_v`) are poles, or both
            // of constant u.
            let both_ends =
                |across_v: bool| poles.iter().filter(|p| p.across_v() == across_v).count() >= 2;
            (closes_u.is_some() || both_ends(false)) && (closes_v.is_some() || both_ends(true))
        }
        _ => false,
    }
}

/// A border of a freeform surface's parameter range that is one point in
/// space: a pole.
struct Pole {
    /// The direction across the border: `V` for a border of constant v
    /// (the surface does not move with u there).
    along: Along,
    /// The parameter the border is at.
    at: f64,
    point: Vec3,
}

/// The poles of a Bézier or NURBS surface (`craft3d_geom::pole`: the
/// borders whose control points are all one point), as the integrals
/// take them: borders of constant v first.
fn poles(surface: &Surface) -> Vec<Pole> {
    let mut found: Vec<Pole> = craft3d_geom::pole::poles(surface)
        .into_iter()
        .map(|pole| Pole {
            along: if pole.across_v() { Along::V } else { Along::U },
            at: pole.at(surface),
            point: pole.point,
        })
        .collect();
    // Stable: the start of a parameter before its end.
    found.sort_by_key(|pole| matches!(pole.along, Along::U));
    found
}

/// Where the line lies along which a face through two poles is divided,
/// as a share of the way from one pole to the other: not a share at which
/// a knot line or the edge of a face is likely to lie, since an edge along
/// the line is on neither side of it.
const DIVIDE: f64 = 0.381_966_011_250_105;

/// How a face is measured whose boundary passes through poles of its
/// surface (see the module documentation).
#[derive(Clone, Copy, Debug)]
struct Poled {
    /// The direction the inner integral runs in: across the poles'
    /// borders.
    along: Along,
    /// The pole it runs from: the parameter of its border.
    from: f64,
    /// When the boundary passes through the pole at the other end too:
    /// the parameter of the line the face is divided along, and that
    /// pole's, which the inner integral runs from beyond the line.
    far: Option<(f64, f64)>,
}

impl Poled {
    /// For a boundary through the `touched` ones of `poles`: across the
    /// borders of constant v when it passes through one of those, across
    /// those of constant u otherwise. A boundary through poles of both
    /// kinds is closed in parameter space by the first kind alone: along
    /// a border of constant u the other parameter does not change.
    fn choose(poles: &[Pole], touched: &[bool]) -> Option<Poled> {
        let touched = |along: Along| {
            poles
                .iter()
                .zip(touched)
                .filter(move |(pole, touched)| **touched && pole.along.same(along))
                .map(|(pole, _)| pole.at)
        };
        let along = if touched(Along::V).next().is_some() {
            Along::V
        } else {
            Along::U
        };
        let mut at = touched(along);
        let from = at.next()?;
        let far = at.next().map(|far| (from + (far - from) * DIVIDE, far));
        Some(Poled { along, from, far })
    }

    /// Whether the parameter `r` across the borders is on the side of the
    /// dividing line that the pole `from` is on.
    fn near(&self, r: f64) -> bool {
        self.far
            .is_none_or(|(split, _)| (r - split) * (self.from - split) > 0.0)
    }

    /// Where the inner integral to `r` runs from.
    fn reference(&self, r: f64) -> f64 {
        match self.far {
            Some((_, far)) if !self.near(r) => far,
            _ => self.from,
        }
    }

    /// Of a boundary sample: its parameter along the borders, the one
    /// across them, and how fast the first changes.
    fn pick(&self, s: &Sample) -> (f64, f64, f64) {
        match self.along {
            Along::V => (s.u, s.v, s.du),
            Along::U => (s.v, s.u, s.dv),
        }
    }
}

/// A stretch of an edge as a loop runs along it, from parameter `start` to
/// `end` (either may be the larger).
struct Piece<'a> {
    curve: &'a Curve3,
    start: f64,
    end: f64,
    /// Where it crosses a knot line of its spline surface.
    crossings: Vec<f64>,
}

/// The boundary of a face: per loop its pieces in the order the loop runs.
struct Outline<'a> {
    loops: Vec<Vec<Piece<'a>>>,
    /// How the face is measured when its boundary passes through a pole
    /// of its freeform surface (a border collapsed into a point).
    poled: Option<Poled>,
    /// Whether every edge is a line, a circle or an ellipse.
    conic: bool,
    /// The share of the boundary's extent that is the rounding of its
    /// coordinates: four units in the last place of the largest of them,
    /// over how far the boundary reaches across. A face a million times
    /// smaller than it is far from the origin is known to ten digits, and
    /// its sums cannot agree to more. Of a thin face it is not the whole
    /// share: [`Outline::noise`].
    noise: f64,
    /// The rounding of the boundary's coordinates: four units in the last
    /// place of the largest of them.
    rounding: f64,
    /// How far the boundary reaches across.
    across: f64,
    /// How far from the surface a boundary point may be found to count
    /// as on it, for the warm start of the projections ([`ON_SURFACE`] of
    /// the body's size).
    touch: f64,
}

fn outline<'a>(
    body: &'a Body,
    face: &Face,
    surface: &Surface,
    index: usize,
    size: f64,
) -> Result<Outline<'a>, MassError> {
    let invalid = MassError::Invalid { face: index };
    let geometry = |error| MassError::Geometry { face: index, error };
    let touch = if size.is_finite() && size > 0.0 {
        ON_SURFACE * size
    } else {
        0.0
    };
    let apex = match surface {
        Surface::Cone(c) => {
            let v = c.apex_v();
            v.is_finite().then(|| c.frame().point(0.0, 0.0, v))
        }
        _ => None,
    };
    let poles = poles(surface);

    // How far the boundary reaches from the apex and from every pole.
    let mut reach = 0.0_f64;
    let mut reaches = vec![0.0_f64; poles.len()];
    let mut extent = Bbox3::EMPTY;
    let mut furthest = 0.0_f64;
    let mut conic = true;
    let mut loops = Vec::with_capacity(face.loops.len());
    for loop_id in &face.loops {
        let loop_ = body.loop_(*loop_id).ok_or(invalid.clone())?;
        for coedge in &loop_.coedges {
            let edge = body.edge(coedge.edge).ok_or(invalid.clone())?;
            if body.vertex(edge.start).is_none() || body.vertex(edge.end).is_none() {
                return Err(invalid);
            }
            conic &= !matches!(edge.curve, Curve3::Nurbs(_));
            let (t0, t1) = edge.range;
            for t in [t0, 0.5 * (t0 + t1), t1] {
                let point = edge.curve.eval(t);
                if point.is_finite() {
                    extent.add(point);
                    furthest = furthest.max(point.length());
                }
                if let Some(apex) = apex {
                    let distance = point.distance(apex);
                    if distance.is_finite() {
                        reach = reach.max(distance);
                    }
                }
                for (pole, reach) in poles.iter().zip(reaches.iter_mut()) {
                    let distance = point.distance(pole.point);
                    if distance.is_finite() {
                        *reach = reach.max(distance);
                    }
                }
            }
        }
        loops.push(&loop_.coedges);
    }

    // Where an edge passes through a point (an apex, a pole) on its way.
    let passes = |curve: &Curve3, (t0, t1): (f64, f64), point: Vec3, reach: f64| {
        if t1.is_nan() || t0.is_nan() || t1 <= t0 {
            return Ok(None);
        }
        let mut t = curve.project(point)?;
        if let Some(period) = curve.period() {
            t = t0 + (t - t0).rem_euclid(period);
        }
        let margin = 1e-9 * (t1 - t0);
        Ok::<_, GeomError>(
            (t > t0 + margin
                && t < t1 - margin
                && curve.eval(t).distance(point) <= AT_APEX * reach)
                .then_some(t),
        )
    };
    // The poles the boundary passes through, at the end of an edge or on
    // its way, and where: an edge that cannot say how near it comes
    // counts as one that stays clear.
    let mut touched = vec![false; poles.len()];
    let mut through: Vec<Vec<f64>> = Vec::new();
    if !poles.is_empty() {
        for coedges in &loops {
            for coedge in coedges.iter() {
                let edge = body.edge(coedge.edge).ok_or(invalid.clone())?;
                let (t0, t1) = edge.range;
                let mut cuts = Vec::new();
                for ((pole, &reach), touched) in poles.iter().zip(&reaches).zip(touched.iter_mut())
                {
                    for end in [t0, t1] {
                        // Not `>`: a NaN touches nothing.
                        if edge.curve.eval(end).distance(pole.point) <= AT_APEX * reach {
                            *touched = true;
                        }
                    }
                    if let Ok(Some(t)) = passes(&edge.curve, (t0, t1), pole.point, reach) {
                        *touched = true;
                        cuts.push(t);
                    }
                }
                through.push(cuts);
            }
        }
    }
    let poled = Poled::choose(&poles, &touched);
    let seams = spline_closure(surface).iter().any(Option::is_some);
    let split = poled.and_then(|p| p.far.map(|(split, _)| (p.along, split)));

    let mut pieces = Vec::with_capacity(loops.len());
    let mut through = through.into_iter();
    for coedges in loops {
        let mut run = Vec::with_capacity(coedges.len());
        for &Coedge { edge, reversed } in coedges {
            let edge = body.edge(edge).ok_or(invalid.clone())?;
            let (t0, t1) = edge.range;
            // Where the edge passes through the apex or a pole on its way.
            let mut cuts = through.next().unwrap_or_default();
            if let Some(apex) = apex
                && let Some(t) = passes(&edge.curve, (t0, t1), apex, reach).map_err(geometry)?
            {
                cuts.push(t);
            }
            cuts.sort_by(f64::total_cmp);
            cuts.dedup();
            let curve = &edge.curve;
            let crossings = knot_crossings(curve, surface, (t0, t1), touch, (split, seams))
                .map_err(geometry)?;
            let mut stops = Vec::with_capacity(cuts.len() + 2);
            stops.push(t0);
            stops.extend(cuts);
            stops.push(t1);
            let mut stretches: Vec<(f64, f64)> = stops
                .windows(2)
                .filter_map(|pair| match pair {
                    &[start, end] => Some((start, end)),
                    _ => None,
                })
                .collect();
            if reversed {
                stretches.reverse();
            }
            for (start, end) in stretches {
                let crossings = crossings.clone();
                run.push(if reversed {
                    Piece {
                        curve,
                        start: end,
                        end: start,
                        crossings,
                    }
                } else {
                    Piece {
                        curve,
                        start,
                        end,
                        crossings,
                    }
                });
            }
        }
        pieces.push(run);
    }
    let across = extent.diagonal();
    let noise = if across > 0.0 {
        (4.0 * f64::EPSILON * furthest / across).min(1.0)
    } else {
        // Smaller than its coordinates resolve: any sum is as good.
        1.0
    };
    Ok(Outline {
        loops: pieces,
        poled,
        conic,
        noise,
        rounding: 4.0 * f64::EPSILON * furthest,
        across,
        touch,
    })
}

impl Outline<'_> {
    /// The share of a face's sums that is the rounding of its boundary's
    /// coordinates, given what was summed for its area.
    ///
    /// Moving the boundary by its rounding changes the area by that times
    /// the boundary's length: a share of the area that is the rounding
    /// over how wide the face is, not over how far it reaches across. The
    /// two are the same for a face about as wide as it is long, and
    /// `noise` is that share. A strip is as wide as its area over its
    /// length: one 1e-5 wide and 1.5 long, a few units from the origin,
    /// is known to five digits less than `noise` says, and its sums
    /// differ from one doubling to the next by that rounding however
    /// fine the panels. Measured on such strips, as shares of the sums:
    /// between 1e-13 and 4e-12 on a cone; on a spline surface 2e-12 with
    /// two inner panels, and twice that with every doubling of them
    /// (3.5e-10 with thirty-two: the ends of every inner panel are
    /// rounded as well); this gives 3e-10 for both, and `noise` 2e-15.
    /// So where the share by the width is more than what sums must agree
    /// to anyway ([`CONVERGED`]) it is what counts; below that it is lost
    /// in the criterion, and `noise` stands as the smaller allowance of
    /// the two.
    ///
    /// Of a part of the boundary refined on its own ([`part_by_part`]) it
    /// is given what was summed for that part's area: the part's sum is
    /// taken and rounded on its own, and is made of inner integrals that
    /// run from the reference across the face to the part, so it carries
    /// the rounding of the face's reach, not of the part's (held to how far
    /// the part itself reaches, a fillet's face of six parts did not
    /// settle). The face's sum is the sum of its parts' and carries the
    /// rounding of every one of them: the parts' allowances add up as
    /// their roundings do. That is looser than one allowance for the
    /// face by as many times as it has parts, where the rounding is what
    /// counts at all: only for parts whose sums are small against the
    /// rounding of the furthest coordinate times the face's reach (the
    /// thin strips above); on the rest `CONVERGED` of their own size is
    /// the allowance, as before. (One allowance for the whole face, shared out
    /// by size, is tighter than the parts' sums can be: a torus at scale
    /// 1e-3 and a solid pinched at a point did not settle.)
    ///
    /// A boundary is known on its surface only as well as its edges lie on
    /// it: `off`, the furthest a sample of what was summed was found from
    /// its foot on the surface, counts as the boundary's rounding where it
    /// is more, up to `touch` ([`ON_SURFACE`] of the body's size, as far as
    /// a boundary point may be found from the surface and count as on it):
    /// a sample further off (a foot found on another sheet of the surface,
    /// an edge that does not lie on its face) earns no more allowance than
    /// that. (An edge approximated within the tolerance passes a pole of a
    /// spline surface within that of it, where the parameters of its feet
    /// turn around the pole: its sums then scatter by what the boundary
    /// moves, however fine the panels.)
    fn noise(&self, area: f64, off: f64) -> f64 {
        let by_width = self.rounding.max(off.min(self.touch)) * self.across / area;
        // Not `<=`: a NaN (nothing summed, nothing across) changes nothing.
        if by_width > CONVERGED {
            by_width.max(self.noise).min(1.0)
        } else {
            self.noise
        }
    }
}

/// One boundary sample: surface parameters, and how fast they change along
/// the edge, times the quadrature weight and the traversal direction.
struct Sample {
    /// How far the boundary point is from its foot on the surface.
    off: f64,
    /// Which loop of the face it belongs to.
    loop_index: usize,
    /// Which piece of that loop, and the parameter of its curve there.
    piece: usize,
    t: f64,
    u: f64,
    v: f64,
    du: f64,
    dv: f64,
}

/// The boundary as quadrature samples, in the order the loops run, with
/// half the panels every edge needs doubled as many times as `level`
/// says for the piece (by its loop and its place in the loop; a piece it
/// says `None` of is left out). `rules` is the full rule and the coarse
/// one of half its nodes.
fn samples(
    outline: &Outline,
    surface: &Surface,
    index: usize,
    rules: (&Rule, &Rule),
    level: &dyn Fn(usize, usize) -> Option<u32>,
    work: &mut Work,
) -> Result<Vec<Sample>, MassError> {
    let which = |loop_index: usize, piece_index: usize, piece: &Piece| {
        let doublings = level(loop_index, piece_index)?;
        let (low, high) = (piece.start.min(piece.end), piece.start.max(piece.end));
        Some(panels(
            piece.curve,
            (low, high),
            &piece.crossings,
            doublings,
        ))
    };
    Ok(sampled(outline, surface, index, rules, &which, work)?.0)
}

/// The panels of a piece a sum takes, by its loop and its place in the
/// loop; `None` leaves the piece out.
type PanelsOf<'a> = dyn Fn(usize, usize, &Piece) -> Option<Vec<Panel>> + 'a;

/// Where the samples of one panel are among those [`sampled`] gives: its
/// loop and piece, the panel, and the range of its samples.
struct Sampled {
    loop_index: usize,
    piece: usize,
    panel: Panel,
    range: std::ops::Range<usize>,
}

/// The boundary as quadrature samples, in the order the loops run, over
/// the panels `which` gives for each piece (by its loop and its place in
/// the loop, in increasing order of the curve's parameter; a piece it
/// says `None` of is left out), and where each panel's samples are.
fn sampled(
    outline: &Outline,
    surface: &Surface,
    index: usize,
    rules: (&Rule, &Rule),
    which: &PanelsOf,
    work: &mut Work,
) -> Result<(Vec<Sample>, Vec<Sampled>), MassError> {
    let mut samples = Vec::new();
    let mut where_ = Vec::new();
    for (loop_index, pieces) in outline.loops.iter().enumerate() {
        for (piece_index, piece) in pieces.iter().enumerate() {
            let Some(panels) = which(loop_index, piece_index, piece) else {
                continue;
            };
            let direction = if piece.end < piece.start { -1.0 } else { 1.0 };
            let first = samples.len();
            let first_panel = where_.len();
            // Each node from the one before it along the piece.
            let mut follow = Follow::new(surface, outline.touch);
            for panel in panels {
                let rule = if panel.coarse { rules.1 } else { rules.0 };
                work.add(PROJECTION * NODES as u64);
                if work.spent_up() {
                    return Err(unsettled(index));
                }
                let from = samples.len();
                for (t, w) in rule.points(panel.start, panel.end) {
                    let (point, tangent) = piece.curve.derivatives(t);
                    let ((u, v), (foot, su, sv)) = follow
                        .project(point)
                        .map_err(|error| MassError::Geometry { face: index, error })?;
                    // Solve [Sᵤ Sᵥ] (du, dv) = tangent in the least-squares
                    // sense: the edge lies on the surface, so this is exact
                    // up to the modelling tolerance.
                    let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
                    let det = a * c - b * b;
                    let (ru, rv) = (tangent.dot(su), tangent.dot(sv));
                    // At a pole or an apex Sᵤ vanishes and u has no rate of
                    // change; `G` is zero there or the boundary is not. (At
                    // a pole of a freeform surface it may be Sᵥ and v.)
                    let (du, dv) = if det > 1e-24 * a.max(c) * a.max(c) && det.is_finite() {
                        ((c * ru - b * rv) / det, (a * rv - b * ru) / det)
                    } else if a > c {
                        (ru / a, 0.0)
                    } else if c > 0.0 {
                        (0.0, rv / c)
                    } else {
                        (0.0, 0.0)
                    };
                    samples.push(Sample {
                        off: point.distance(foot),
                        loop_index,
                        piece: piece_index,
                        t,
                        u,
                        v,
                        du: du * w * direction,
                        dv: dv * w * direction,
                    });
                }
                where_.push(Sampled {
                    loop_index,
                    piece: piece_index,
                    panel,
                    range: from..samples.len(),
                });
            }
            if direction < 0.0 {
                // Keep the samples in the order the loop runs.
                let end = samples.len();
                if let Some(block) = samples.get_mut(first..) {
                    block.reverse();
                }
                for at in where_.iter_mut().skip(first_panel) {
                    at.range = (first + end - at.range.end)..(first + end - at.range.start);
                }
            }
        }
    }
    Ok((samples, where_))
}

/// `∬ f du dv` over a whole closed surface.
///
/// On a spline surface the panels in u end at its knots, like those in v
/// ([`inner`]), and are doubled until two sums agree.
fn whole_surface(
    surface: &Surface,
    index: usize,
    rule: &Rule,
    work: &mut Work,
) -> Result<Sums, MassError> {
    let ((u0, u1), (v0, v1)) = surface.domain();
    if ![u0, u1, v0, v1].iter().all(|x| x.is_finite()) {
        return Err(MassError::UnboundedFace { face: index });
    }
    let mut across = |from: f64, to: f64, count: usize, doublings: u32, sum: &mut Total| {
        let width = (to - from) / count as f64;
        for k in 0..count {
            if work.spent_up() {
                return Err(unsettled(index));
            }
            let start = from + width * k as f64;
            for (u, w) in rule.points(start, start + width) {
                sum.add(
                    inner(surface, rule, Along::V, u, (v0, v1), doublings, work),
                    w,
                );
            }
        }
        Ok(())
    };
    if !matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
        const U_PANELS: usize = 8;
        let mut sum = Total::default();
        across(u0, u1, U_PANELS, 0, &mut sum)?;
        // Exact to rounding (a trigonometric polynomial on panels of an
        // eighth of a turn); held to what any sum is.
        work.hold(&sum, 0.0);
        return Ok(sum.sum);
    }
    let (mut lines, _) = knot_lines(surface);
    lines.push(u1);
    let mut coarser: Option<Total> = None;
    for doublings in 0..=MAX_DOUBLINGS {
        let mut sum = Total::default();
        let mut start = u0;
        for &line in &lines {
            across(start, line, 1 << doublings, doublings, &mut sum)?;
            start = line;
        }
        if coarser.is_some_and(|coarser| coarser.agrees_with(&sum, 0.0)) {
            work.hold(&sum, 0.0);
            return Ok(sum.sum);
        }
        coarser = Some(sum);
    }
    Err(MassError::Unsupported {
        face: index,
        what: "an integral over a whole spline surface that does not settle",
    })
}

/// `value` moved by whole periods to be as near to `target` as it gets.
fn near(value: f64, target: f64, period: f64) -> f64 {
    value + ((target - value) / period).round() * period
}

/// How the boundary integral of a face is set up. Decided once, from the
/// coarsest samples, so that the finer sums are sums of the same thing.
enum Plan {
    /// The inner integral runs along `along` from `at`: `G` along v from
    /// a value of v (`-∮ G du`), or, on a spline surface whose boundary
    /// goes over the seam where it closes in v, `H` along u from a value
    /// of u (`∮ H dv`).
    From { along: Along, at: f64 },
    /// A boundary through poles of a freeform surface.
    Poles(Poled),
    /// A surface closed both ways: the parameters each loop's unrolling
    /// starts near, the direction the inner integral runs in, and the
    /// periods in u and v.
    Unrolled {
        starts: Vec<(f64, f64)>,
        along: Along,
        periods: (f64, f64),
    },
}

impl Plan {
    /// Whether the boundary integral by this plan is right only up to a
    /// whole number of times the entire surface (see the module
    /// documentation).
    fn up_to_whole_surfaces(&self) -> bool {
        matches!(self, Plan::Unrolled { .. })
    }
}

/// Unrolls u and v along each loop of a surface closed both ways, each
/// loop starting near its entry in `starts` (or wherever its first sample
/// is). Gives where each loop started and how often it goes around the
/// axis and around the tube before it is back at its first sample.
fn unroll(
    samples: &mut [Sample],
    loops: usize,
    starts: Option<&[(f64, f64)]>,
    (period_u, period_v): (f64, f64),
) -> (Vec<(f64, f64)>, Vec<f64>, Vec<f64>) {
    let mut began = vec![(0.0, 0.0); loops];
    let mut around_axis = vec![0.0_f64; loops];
    let mut around_tube = vec![0.0_f64; loops];
    let mut previous: Option<(usize, f64, f64)> = None;
    let mut first = (0.0, 0.0);
    let mut close = |loop_index: usize, last: (f64, f64), first: (f64, f64)| {
        if let (Some(axis), Some(tube)) = (
            around_axis.get_mut(loop_index),
            around_tube.get_mut(loop_index),
        ) {
            *axis = ((near(first.0, last.0, period_u) - first.0) / period_u).round();
            *tube = ((near(first.1, last.1, period_v) - first.1) / period_v).round();
        }
    };
    for sample in samples.iter_mut() {
        match previous {
            Some((loop_index, u, v)) if loop_index == sample.loop_index => {
                sample.u = near(sample.u, u, period_u);
                sample.v = near(sample.v, v, period_v);
            }
            other => {
                if let Some((loop_index, u, v)) = other {
                    close(loop_index, (u, v), first);
                }
                if let Some(&(u, v)) = starts.and_then(|s| s.get(sample.loop_index)) {
                    sample.u = near(sample.u, u, period_u);
                    sample.v = near(sample.v, v, period_v);
                }
                first = (sample.u, sample.v);
                if let Some(began) = began.get_mut(sample.loop_index) {
                    *began = first;
                }
            }
        }
        previous = Some((sample.loop_index, sample.u, sample.v));
    }
    if let Some((loop_index, u, v)) = previous {
        close(loop_index, (u, v), first);
    }
    (began, around_axis, around_tube)
}

/// The plan for a face from its coarsest samples (see the module
/// documentation), which are unrolled on the way for a surface closed
/// both ways.
fn plan(
    face: &Face,
    surface: &Surface,
    index: usize,
    samples: &mut [Sample],
    sense: f64,
) -> Result<Plan, MassError> {
    if let (Some(period_u), Some(period_v)) = (surface.period_u(), surface.period_v()) {
        return unrolled(face, index, samples, (period_u, period_v));
    }
    if surface.period_v().is_some() {
        return Err(MassError::Unsupported {
            face: index,
            what: "a surface closed in v only",
        });
    }
    // A spline surface may close on itself without saying so. A boundary
    // that goes over the seam where it closes in u needs nothing (`G` is
    // integrated against `du`, which is zero along the seam); one that
    // goes over the seam where it closes in v is integrated the other way
    // round, against `dv`; one that goes over both is unrolled as on a
    // torus.
    let [closes_u, closes_v] = spline_closure(surface);
    let over = |closes: Option<(f64, f64)>, pick: fn(&Sample) -> f64| {
        closes.filter(|&(low, high)| jumps(samples, pick, high - low))
    };
    match (over(closes_u, |s| s.u), over(closes_v, |s| s.v)) {
        (Some((u0, u1)), Some((v0, v1))) => {
            return unrolled(face, index, samples, (u1 - u0, v1 - v0));
        }
        (None, Some(_)) => return across_v_seam(surface, index, samples, sense),
        _ => {}
    }
    let ((u_min, u_max), (v_min, v_max)) = surface.domain();
    let first_v = samples.first().map_or(0.0, |s| s.v);
    let v_ref = match (surface, surface.period_u()) {
        // The north pole of the frame chosen for the face.
        (Surface::Sphere(_), _) => v_max,
        (Surface::Plane(_), _) => first_v,
        (_, None) => {
            // A spline surface may close on itself around u without
            // saying so, and a boundary that goes around it once in total
            // (the rim of a cap on a sphere of revolution) is not closed
            // in its parameters: the face reaches to the end of v on one
            // side. That is a face only if the surface is a single point
            // there, a pole, which is then where `G` runs from, as from
            // an apex.
            let swept = samples.iter().map(|s| s.du).sum::<f64>();
            let turns = (sense * swept / (u_max - u_min)).round();
            let risen = samples.iter().map(|s| s.dv).sum::<f64>();
            if (risen / (v_max - v_min)).round() != 0.0 {
                return Err(MassError::Unsupported {
                    face: index,
                    what: "a boundary that goes around a spline surface in v",
                });
            }
            if turns == 0.0 {
                first_v
            } else if turns.abs() == 1.0 {
                let end = if turns > 0.0 { v_max } else { v_min };
                let pole = (0..=8).all(|k| {
                    let u = u_min + (u_max - u_min) * f64::from(k) / 8.0;
                    let (_, su, sv) = surface.derivatives(u, end);
                    su.length() <= 1e-9 * (su.length() + sv.length())
                });
                if !pole {
                    return Err(MassError::UnboundedFace { face: index });
                }
                end
            } else {
                return Err(MassError::Unsupported {
                    face: index,
                    what: "a boundary that winds around the axis more than once",
                });
            }
        }
        (_, Some(period)) => {
            let apex = match surface {
                Surface::Cone(c) => Some(c.apex_v()).filter(|v| v.is_finite()),
                _ => None,
            };
            let near_apex = apex.filter(|&apex| {
                let (mut nearest, mut furthest) = (f64::INFINITY, 0.0_f64);
                for s in samples.iter() {
                    nearest = nearest.min((s.v - apex).abs());
                    furthest = furthest.max((s.v - apex).abs());
                }
                nearest < 0.5 * furthest
            });
            if let Some(apex) = near_apex {
                apex
            } else {
                // Net number of times the boundary goes around the axis in
                // the surface's counter-clockwise sense. It stays well
                // away from an apex here, so the sum is a good one.
                let swept = samples.iter().map(|s| s.du).sum::<f64>();
                let turns = (sense * swept / period).round();
                if turns == 0.0 {
                    first_v
                } else if turns == 1.0 {
                    // The face is on the +v side of a loop running in +u.
                    v_max
                } else if turns == -1.0 {
                    v_min
                } else {
                    return Err(MassError::Unsupported {
                        face: index,
                        what: "a boundary that winds around the axis more than once",
                    });
                }
            }
        }
    };
    if !v_ref.is_finite() {
        return Err(MassError::UnboundedFace { face: index });
    }
    Ok(Plan::From {
        along: Along::V,
        at: v_ref,
    })
}

/// The plan for a face of a surface closed both ways, with these periods:
/// its loops unrolled.
fn unrolled(
    face: &Face,
    index: usize,
    samples: &mut [Sample],
    (period_u, period_v): (f64, f64),
) -> Result<Plan, MassError> {
    let (starts, around_axis, around_tube) =
        unroll(samples, face.loops.len(), None, (period_u, period_v));
    if around_axis.iter().sum::<f64>() != 0.0 || around_tube.iter().sum::<f64>() != 0.0 {
        // Loops that go around without others going back do not enclose
        // a region.
        return Err(MassError::UnboundedFace { face: index });
    }
    let along = if around_tube.iter().all(|&turns| turns == 0.0) {
        Along::V
    } else if around_axis.iter().all(|&turns| turns == 0.0) {
        Along::U
    } else {
        return Err(MassError::Unsupported {
            face: index,
            what: "loops that wind around the axis and around the tube of a torus at once",
        });
    };
    Ok(Plan::Unrolled {
        starts,
        along,
        periods: (period_u, period_v),
    })
}

/// The plan for a face of a spline surface that closes on itself in v,
/// whose boundary goes over the seam there (and not over one in u): the
/// inner integral runs along u, from a value of u, and the boundary
/// integral is `∮ H dv`, to which the seam (a line of constant v)
/// contributes nothing. It mirrors the case of a surface closed in u: a
/// boundary that goes around in v once in total is the rim of a face that
/// reaches to an end of u, a face only if the surface is one point there.
fn across_v_seam(
    surface: &Surface,
    index: usize,
    samples: &[Sample],
    sense: f64,
) -> Result<Plan, MassError> {
    let ((u_min, u_max), (v_min, v_max)) = surface.domain();
    let swept = samples.iter().map(|s| s.dv).sum::<f64>();
    let turns = (sense * swept / (v_max - v_min)).round();
    let risen = samples.iter().map(|s| s.du).sum::<f64>();
    if (risen / (u_max - u_min)).round() != 0.0 {
        return Err(MassError::Unsupported {
            face: index,
            what: "a boundary that goes around a spline surface in u",
        });
    }
    let u_ref = if turns == 0.0 {
        samples.first().map_or(0.0, |s| s.u)
    } else if turns.abs() == 1.0 {
        // The face is on the −u side of a loop running in +v.
        let end = if turns > 0.0 { u_min } else { u_max };
        let pole = (0..=8).all(|k| {
            let v = v_min + (v_max - v_min) * f64::from(k) / 8.0;
            let (_, su, sv) = surface.derivatives(end, v);
            sv.length() <= 1e-9 * (su.length() + sv.length())
        });
        if !pole {
            return Err(MassError::UnboundedFace { face: index });
        }
        end
    } else {
        return Err(MassError::Unsupported {
            face: index,
            what: "a boundary that winds around a spline surface more than once",
        });
    };
    if !u_ref.is_finite() {
        return Err(MassError::UnboundedFace { face: index });
    }
    Ok(Plan::From {
        along: Along::U,
        at: u_ref,
    })
}

/// Whether a boundary that is measured from two poles jumps from one
/// side of its surface's parameter range to the other between two samples
/// of one piece: the surface closes on itself there and the face lies
/// across the seam, where the parameter along the poles' borders is not
/// one number.
fn crosses_seam(samples: &[Sample], poled: &Poled, surface: &Surface) -> bool {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let width = match poled.along {
        Along::V => u1 - u0,
        Along::U => v1 - v0,
    };
    let Some((_, far)) = poled.far else {
        return false;
    };
    // At a pole itself the parameter along its border is any number.
    let clear = |s: &Sample| {
        let r = poled.pick(s).1;
        let margin = 1e-6 * (far - poled.from).abs();
        (r - poled.from).abs() > margin && (r - far).abs() > margin
    };
    samples.windows(2).any(|pair| match pair {
        [a, b] => {
            a.loop_index == b.loop_index
                && a.piece == b.piece
                && clear(a)
                && clear(b)
                && (poled.pick(a).0 - poled.pick(b).0).abs() > 0.5 * width
        }
        _ => false,
    })
}

/// `∬ f` over the rectangle between two values of the parameter along a
/// pair of poles' borders and between the two poles, by the rule both
/// ways: the panels end at the surface's knot lines and are doubled with
/// `doublings`, like the inner integral's ([`inner`]). `None` when the
/// face has taken its budget on the way.
fn between_poles(
    surface: &Surface,
    rule: &Rule,
    poled: &Poled,
    (from, to): (f64, f64),
    doublings: u32,
    work: &mut Work,
) -> Option<Total> {
    let mut sum = Total::default();
    let Some((_, far)) = poled.far else {
        return Some(sum);
    };
    let (lines_u, lines_v) = knot_lines(surface);
    let lines = match poled.along {
        Along::V => lines_u,
        Along::U => lines_v,
    };
    let (low, high, sign) = if from <= to {
        (from, to, 1.0)
    } else {
        (to, from, -1.0)
    };
    let count = 1_usize << doublings.min(6);
    let mut stops = vec![low];
    stops.extend(lines.into_iter().filter(|&line| line > low && line < high));
    stops.push(high);
    for pair in stops.windows(2) {
        let &[start, end] = pair else { continue };
        let width = (end - start) / count as f64;
        for k in 0..count {
            if work.spent_up() {
                return None;
            }
            let at = start + width * k as f64;
            for (c, w) in rule.points(at, at + width) {
                sum.add(
                    inner(
                        surface,
                        rule,
                        poled.along,
                        c,
                        (poled.from, far),
                        doublings,
                        work,
                    ),
                    w * sign,
                );
            }
        }
    }
    Some(sum)
}

/// Where the boundary of a face through two poles crosses the line the
/// face is divided along (see the module documentation): the parameter
/// along the line of every crossing, and whether the boundary goes over
/// to the far pole's side there (`true`) or comes back from it.
///
/// A crossing lies between two samples in a row that are on different
/// sides: on one piece, or on two with the end of the one and the start
/// of the other between them. It is narrowed by bisection along the
/// piece.
fn divide_crossings(
    outline: &Outline,
    surface: &Surface,
    samples: &[Sample],
    poled: &Poled,
) -> Result<Vec<(f64, bool)>, GeomError> {
    let mut crossings = Vec::new();
    let mut follow = Follow::new(surface, outline.touch);
    let mut across = |curve: &Curve3, t: f64| -> Result<(f64, f64), GeomError> {
        let ((u, v), _) = follow.project(curve.eval(t))?;
        Ok(match poled.along {
            Along::V => (u, v),
            Along::U => (v, u),
        })
    };
    let mut start = 0;
    while start < samples.len() {
        let Some(first) = samples.get(start) else {
            break;
        };
        let end = samples
            .iter()
            .skip(start)
            .position(|s| s.loop_index != first.loop_index)
            .map_or(samples.len(), |offset| start + offset);
        let Some(block) = samples.get(start..end) else {
            break;
        };
        let pieces = outline.loops.get(first.loop_index);
        for (k, here) in block.iter().enumerate() {
            // The sample before this one as the loop runs, around its end.
            let before = if k == 0 {
                block.last()
            } else {
                block.get(k - 1)
            };
            let Some(before) = before else { continue };
            let (was_near, is_near) = (
                poled.near(poled.pick(before).1),
                poled.near(poled.pick(here).1),
            );
            if was_near == is_near {
                continue;
            }
            let piece = |index: usize| pieces.and_then(|pieces| pieces.get(index));
            let (Some(left), Some(right)) = (piece(before.piece), piece(here.piece)) else {
                continue;
            };
            // The stretch of one piece that the crossing is on.
            let (curve, mut a, mut b) = if k > 0 && before.piece == here.piece {
                (left.curve, before.t, here.t)
            } else if poled.near(across(right.curve, right.start)?.1) == was_near {
                (right.curve, right.start, here.t)
            } else {
                (left.curve, before.t, left.end)
            };
            for _ in 0..64 {
                let middle = 0.5 * (a + b);
                if !((middle > a && middle < b) || (middle < a && middle > b)) {
                    break;
                }
                if poled.near(across(curve, middle)?.1) == was_near {
                    a = middle;
                } else {
                    b = middle;
                }
            }
            crossings.push((across(curve, 0.5 * (a + b))?.0, was_near));
        }
        start = end;
    }
    Ok(crossings)
}

/// The boundary integral `-∮ G du` (or `∮ H dv`) of a face by `plan`;
/// `None` when the face has taken its budget on the way.
#[allow(clippy::too_many_arguments)]
fn boundary_integral(
    (face, outline): (&Face, &Outline),
    surface: &Surface,
    rule: &Rule,
    samples: &mut [Sample],
    sense: f64,
    plan: &Plan,
    (unrolled, doublings): (bool, u32),
    work: &mut Work,
) -> Result<Option<Total>, GeomError> {
    let mut total = Total::default();
    match plan {
        Plan::Poles(poled) => {
            // `-∮ G du` across borders of constant v, `∮ H dv` across
            // those of constant u.
            let weight = match poled.along {
                Along::V => -sense,
                Along::U => sense,
            };
            for s in samples.iter() {
                if work.spent_up() {
                    return Ok(None);
                }
                let (c, r, dc) = poled.pick(s);
                if dc != 0.0 {
                    total.add(
                        inner(
                            surface,
                            rule,
                            poled.along,
                            c,
                            (poled.reference(r), r),
                            doublings,
                            work,
                        ),
                        weight * dc,
                    );
                }
            }
            if poled.far.is_some() {
                // Beyond the dividing line the inner integral runs from
                // the far pole, and so falls short of the one from the
                // near pole by the whole way between the two: that, along
                // every stretch of the boundary beyond the line, is the
                // integral over the rectangle between where the stretch
                // goes over the line and where it comes back.
                let crossings = divide_crossings(outline, surface, samples, poled)?;
                if let Some(&(first, _)) = crossings.first() {
                    for &(c, leaves) in &crossings {
                        let Some(rectangle) =
                            between_poles(surface, rule, poled, (first, c), doublings, work)
                        else {
                            return Ok(None);
                        };
                        total.add(rectangle, if leaves { -weight } else { weight });
                    }
                }
            }
        }
        Plan::From {
            along: Along::V,
            at: v_ref,
        } => {
            for s in samples.iter() {
                if work.spent_up() {
                    return Ok(None);
                }
                if s.du != 0.0 {
                    total.add(
                        inner(surface, rule, Along::V, s.u, (*v_ref, s.v), doublings, work),
                        -sense * s.du,
                    );
                }
            }
        }
        Plan::From {
            along: Along::U,
            at: u_ref,
        } => {
            for s in samples.iter() {
                if work.spent_up() {
                    return Ok(None);
                }
                if s.dv != 0.0 {
                    total.add(
                        inner(surface, rule, Along::U, s.v, (*u_ref, s.u), doublings, work),
                        sense * s.dv,
                    );
                }
            }
        }
        Plan::Unrolled {
            starts,
            along,
            periods,
        } => {
            if !unrolled {
                unroll(samples, face.loops.len(), Some(starts), *periods);
            }
            let (u_ref, v_ref) = starts.first().copied().unwrap_or((0.0, 0.0));
            for s in samples.iter() {
                if work.spent_up() {
                    return Ok(None);
                }
                match along {
                    Along::V if s.du != 0.0 => total.add(
                        inner(surface, rule, Along::V, s.u, (v_ref, s.v), doublings, work),
                        -sense * s.du,
                    ),
                    Along::U if s.dv != 0.0 => total.add(
                        inner(surface, rule, Along::U, s.v, (u_ref, s.u), doublings, work),
                        sense * s.dv,
                    ),
                    _ => {}
                }
            }
        }
    }
    Ok(Some(total))
}

/// What a converged boundary integral says about the face.
fn settle(
    surface: &Surface,
    index: usize,
    rule: &Rule,
    (total, up_to_whole_surfaces): (Total, bool),
    work: &mut Work,
) -> Result<Sums, MassError> {
    let mut sum = total.sum;
    if up_to_whole_surfaces {
        // Right up to a whole number of times the entire surface; the area
        // says which.
        let before = work.held;
        let whole = whole_surface(surface, index, rule, work)?;
        let share = sum.area / whole.area;
        let times = share.floor();
        if times != 0.0 {
            let rest = share - times;
            if !(rest > 1e-9 && rest < 1.0 - 1e-9) {
                return Err(MassError::Unsupported {
                    face: index,
                    what: "a face too thin, or too nearly its whole surface, to tell which side of its loops it is on",
                });
            }
            sum.add_scaled(whole, -times);
        }
        // The whole surface counts as often as it is taken away.
        for (held, before) in work.held.iter_mut().zip(before) {
            *held = before + (*held - before) * times.abs();
        }
    } else if matches!(surface, Surface::Sphere(_)) {
        // The face, less the whole sphere when the face holds the south
        // pole; the sign of the area says which, unless the sum is zero
        // to rounding.
        if sum.area.abs() <= TOO_THIN * total.size.area {
            return Err(MassError::Unsupported {
                face: index,
                what: "a boundary that doubles back on itself, or a face too thin, or too nearly its whole sphere, to tell which side of its loops it is on",
            });
        }
        if sum.area < 0.0 {
            sum.add_scaled(whole_surface(surface, index, rule, work)?, 1.0);
        }
    } else if sum.area < 0.0 {
        // The loops run the wrong way around what they enclose: the face
        // is everything outside them. On a spline surface that closes on
        // itself all around (a NURBS torus, an egg) that is the whole
        // surface less what they enclose, unless the sum is zero to
        // rounding, as on a sphere.
        if !closed_all_around(surface) {
            return Err(MassError::UnboundedFace { face: index });
        }
        if sum.area.abs() <= TOO_THIN * total.size.area {
            return Err(MassError::Unsupported {
                face: index,
                what: "a face too thin, or too nearly its whole surface, to tell which side of its loops it is on",
            });
        }
        sum.add_scaled(whole_surface(surface, index, rule, work)?, 1.0);
    }
    Ok(sum)
}

/// `∬_D f du dv` over one face, with `D` oriented by the surface (not by
/// the face's `reversed` flag).
///
/// With `sums` [`Wanted::Both`], the second moments too, and beside them
/// the sums without them as [`Wanted::First`] gives them, bit for bit:
/// the integrals of the area, the volume and the moments do not depend on
/// whether the second moments are summed beside them, so the sums are
/// those of [`Wanted::First`] after as many doublings, which are taken as
/// they stand once those parts agree, and the work they took is the same
/// (see [`integrals_in`]).
fn face_integrals(
    body: &Body,
    face: &Face,
    index: usize,
    rule: &Rule,
    (size, middle): (f64, Vec3),
    sums: Wanted,
) -> FaceSums {
    let second = sums != Wanted::First;
    if let Some(ratio) = uneven_weights(&face.surface).filter(|&r| r > MAX_UNEVEN || r.is_nan()) {
        let error = MassError::UnevenWeights { face: index, ratio };
        return FaceSums {
            first: (sums == Wanted::Both).then(|| Err(error.clone())),
            all: Err(error),
            held: [0.0; 2],
        };
    }
    if face.loops.is_empty() || seams_only(body, face) {
        let mut work = Work::about(&face.surface, middle, second);
        let all = whole_surface(&face.surface, index, rule, &mut work);
        counted(&work);
        let held = work.held;
        let first = (sums == Wanted::Both).then(|| {
            let mut work = Work::about(&face.surface, middle, false);
            let first = whole_surface(&face.surface, index, rule, &mut work);
            counted(&work);
            first
        });
        return FaceSums { first, all, held };
    }
    let own = match &face.surface {
        Surface::Sphere(sphere) => own_sphere(body, face, sphere),
        _ => None,
    };
    let surface = own.as_ref().unwrap_or(&face.surface);
    let mut work = Work::about(surface, middle, second);
    let mut first = None;
    let all = integrals_in(
        (body, face, surface, index),
        rule,
        size,
        &mut work,
        (sums == Wanted::Both).then_some(&mut first),
    );
    counted(&work);
    // Without first-order sums that agreed, the sums without the second
    // moments ended where these did, with the same error.
    let first = (sums == Wanted::Both).then(|| first.unwrap_or_else(|| all.clone()));
    FaceSums {
        first,
        all,
        held: work.held,
    }
}

/// How far apart the weights within one span of a rational surface may be
/// once a reparametrisation has evened them out as far as it can
/// ([`uneven_weights`]), for a face to be measured. Measured to 1.6e-13
/// up to a thousand (the plate with the weights 1, h, h, 1, whose
/// unevenness no reparametrisation takes out) and refused, by sums that
/// do not settle, at ten thousand; further apart the edges no longer lie
/// on such a surface to the modelling tolerance (from 1e5) and sums came
/// out settled and wrong (from 1e7: 22 for a volume of 24). So the
/// measure stops where it is held, not where it is seen to fail.
const MAX_UNEVEN: f64 = 1e4;

/// The largest ratio of two weights within one Bézier span of a rational
/// surface after the Möbius maps along u and v that even out its corner
/// weights (`α^m = √(w₀₀ w₀ₙ / (w_m0 w_mn))`, `β^n = √(w₀₀ w_m0 / (w₀ₙ
/// w_mn))`, weights `w_ij α^i β^j`, the same surface): what is left is the
/// unevenness no reparametrisation removes (one for weights of a product
/// form, `h` for the corners 1, h, h, 1). In logarithms, so that weights
/// of any size neither overflow nor vanish. `None` for a surface that is
/// not a rational NURBS surface, or whose spans cannot be had.
fn uneven_weights(surface: &Surface) -> Option<f64> {
    let Surface::Nurbs(s) = surface else {
        return None;
    };
    let first = s.weights().first()?;
    if s.weights().iter().all(|w| w == first) {
        return None;
    }
    let (m, n) = s.degrees();
    let mut worst = 0.0_f64;
    for span in s.bezier_spans().ok()? {
        let logs: Vec<f64> = span.patch.weights().iter().map(|w| w.ln()).collect();
        let at = |i: usize, j: usize| logs.get(i * (n + 1) + j).copied();
        let (w00, w0n, wm0, wmn) = (at(0, 0)?, at(0, n)?, at(m, 0)?, at(m, n)?);
        let alpha = if m > 0 {
            (w00 + w0n - wm0 - wmn) / (2.0 * m as f64)
        } else {
            0.0
        };
        let beta = if n > 0 {
            (w00 + wm0 - w0n - wmn) / (2.0 * n as f64)
        } else {
            0.0
        };
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for (k, &w) in logs.iter().enumerate() {
            let (i, j) = (k / (n + 1), k % (n + 1));
            let even = w + alpha * i as f64 + beta * j as f64;
            low = low.min(even);
            high = high.max(even);
        }
        // Not `max`: a NaN (a weight that is not positive) is as uneven
        // as it gets.
        let spread = high - low;
        worst = if spread.is_nan() {
            f64::INFINITY
        } else {
            worst.max(spread)
        };
    }
    Some(worst.exp())
}

/// Faces integrated, and the work they took ([`Work::spent`]), since the
/// process started: for the performance benchmark of the gate
/// (`crates/engine/tests/benchmark.rs`). Added to once per face, and
/// summed the same whatever the threads.
static FACES_INTEGRATED: AtomicU64 = AtomicU64::new(0);
static WORK_SPENT: AtomicU64 = AtomicU64::new(0);

/// How many faces [`mass_properties`], [`inertia`] and
/// [`mass_and_inertia`] have integrated since the process started, and
/// the work that took in evaluations of the integrand (a count, not a
/// clock: the same on every machine and under any load).
pub fn mass_work() -> (u64, u64) {
    (
        FACES_INTEGRATED.load(Ordering::Relaxed),
        WORK_SPENT.load(Ordering::Relaxed),
    )
}

fn counted(work: &Work) {
    FACES_INTEGRATED.fetch_add(1, Ordering::Relaxed);
    WORK_SPENT.fetch_add(work.spent, Ordering::Relaxed);
}

/// Which sums [`integrate`] makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wanted {
    /// The area, the volume and the moments ([`mass_properties`]).
    First,
    /// Those and the second moments ([`inertia`]).
    Second,
    /// Both of the above at once ([`mass_and_inertia`]).
    Both,
}

/// What [`face_integrals`] gives: the sums asked for, and with
/// [`Wanted::Both`] the sums [`Wanted::First`] gives.
struct FaceSums {
    first: Option<Result<Sums, MassError>>,
    all: Result<Sums, MassError>,
    /// What the area and the volume of `all` were held to
    /// ([`Work::held`]).
    held: [f64; 2],
}

/// [`face_integrals`] of a face with loops, with `surface` standing in for
/// the face's own: the same surface, parametrised to suit the face.
/// `size` is how big the body is ([`body_size`]), `work` what the face
/// has taken so far: past [`BUDGET`] it is given up.
///
/// With `first` (the second moments being summed), it is also given the
/// sums as they would be without the second moments, once their area,
/// volume and moments agree: settled with a copy of the work as it stands
/// then and without the second moments, which is the work and the sums
/// the face takes without them up to there.
fn integrals_in(
    (body, face, surface, index): (&Body, &Face, &Surface, usize),
    rule: &Rule,
    size: f64,
    work: &mut Work,
    first: Option<&mut Option<Result<Sums, MassError>>>,
) -> Result<Sums, MassError> {
    let outline = outline(body, face, surface, index, size)?;
    // Loops run counter-clockwise around the face seen from outside, which
    // is clockwise in parameter space when the face is reversed.
    let sense = if face.reversed { -1.0 } else { 1.0 };
    // Exact with the panels an edge needs (see the module documentation).
    let exact = outline.conic && matches!(surface, Surface::Plane(_));

    let coarse = Rule::new(NODES / 2);
    // Through two poles the integral has a term of the whole boundary
    // (where it crosses the dividing line): the boundary goes together.
    let separable = outline.poled.is_none_or(|poled| poled.far.is_none());
    if !exact && separable {
        return part_by_part(
            face,
            (surface, &outline),
            index,
            (rule, &coarse),
            sense,
            work,
            first,
        );
    }
    together(
        face,
        (surface, &outline),
        index,
        (rule, &coarse),
        (sense, exact),
        work,
        first,
    )
}

/// [`integrals_in`] with the whole boundary refined together, every piece
/// doubled with every sum until two sums of the whole agree: for a plane
/// face bounded by lines and circles (exact at once) and a boundary
/// through two poles, where a term belongs to the whole boundary.
fn together(
    face: &Face,
    (surface, outline): (&Surface, &Outline),
    index: usize,
    (rule, coarse): (&Rule, &Rule),
    (sense, exact): (f64, bool),
    work: &mut Work,
    mut first: Option<&mut Option<Result<Sums, MassError>>>,
) -> Result<Sums, MassError> {
    let mut set_up: Option<Plan> = None;
    let mut coarser: Option<Total> = None;
    for doublings in u32::from(exact)..=MAX_DOUBLINGS {
        let mut samples = samples(
            outline,
            surface,
            index,
            (rule, coarse),
            &|_, _| Some(doublings),
            work,
        )?;
        let (plan, unrolled) = match set_up.take() {
            Some(plan) => (plan, false),
            None => {
                let plan = match outline.poled {
                    Some(poled) => {
                        if poled.far.is_some() && crosses_seam(&samples, &poled, surface) {
                            return Err(MassError::Unsupported {
                                face: index,
                                what: "a boundary through two poles of a spline surface that also crosses the seam where the surface closes on itself",
                            });
                        }
                        Plan::Poles(poled)
                    }
                    None => plan(face, surface, index, &mut samples, sense)?,
                };
                (plan, true)
            }
        };
        if unrolled && plan.up_to_whole_surfaces() {
            // Inner integrals may then run beyond the domain of a spline
            // surface, by whole turns.
            work.wrap = spline_closure(surface);
        }
        let Some(total) = boundary_integral(
            (face, outline),
            surface,
            rule,
            &mut samples,
            sense,
            &plan,
            (unrolled, doublings),
            work,
        )
        .map_err(|error| MassError::Geometry { face: index, error })?
        else {
            break;
        };
        let off = samples.iter().map(|s| s.off).fold(0.0, f64::max);
        let noise = outline.noise(total.size.area, off);
        if let Some(slot) = first.as_deref_mut()
            && slot.is_none()
            && (exact || coarser.is_some_and(|coarser| coarser.first_agrees_with(&total, noise)))
        {
            let mut alone = work.clone();
            alone.second = false;
            *slot = Some(settle(
                surface,
                index,
                rule,
                (total, plan.up_to_whole_surfaces()),
                &mut alone,
            ));
        }
        if exact || coarser.is_some_and(|coarser| coarser.agrees_with(&total, noise)) {
            work.hold(&total, noise);
            return settle(
                surface,
                index,
                rule,
                (total, plan.up_to_whole_surfaces()),
                work,
            );
        }
        coarser = Some(total);
        set_up = Some(plan);
    }
    Err(unsettled(index))
}

/// What is known of one part of a boundary integral refined on its own
/// ([`part_by_part`]).
#[derive(Default)]
struct Part {
    /// How often its panels are doubled next; `None` once it has settled.
    level: Option<u32>,
    coarser: Option<Total>,
    settled: Option<Total>,
    /// With the second moments summed: the sums once its area, volume
    /// and moments agreed, which are those without the second moments.
    first: Option<Total>,
    /// The share of rounding its settled sums were held with.
    noise: f64,
}

/// [`integrals_in`] for a face whose boundary integral is a sum of one
/// term per sample (not through two poles), refined part by part: each
/// piece of the boundary on its own, or each loop where the loops are
/// unrolled on a surface closed both ways (a loop is unrolled as a
/// whole). A part is refined until two of its sums in a row agree, and is
/// not summed again once it has; the face is settled when every part is.
/// Each part is held to the allowance of its own sums (rounding included,
/// [`Outline::noise`]), and together to the sum of those.
///
/// The work is bounded for the face as a whole. Its first sums (every
/// part at its coarsest) are as much work as its boundary is long, and
/// may take up to [`FIRST_SUMS_WORK`]; in all the face may take [`PARTS_WORK`]
/// times what they took, or its budget where that is more. A face with
/// tens of holes takes more than one budget only to sum each of them
/// twice.
fn part_by_part(
    face: &Face,
    (surface, outline): (&Surface, &Outline),
    index: usize,
    (rule, coarse): (&Rule, &Rule),
    sense: f64,
    work: &mut Work,
    mut first_sums: Option<&mut Option<Result<Sums, MassError>>>,
) -> Result<Sums, MassError> {
    let start = work.spent;
    let budget = work.limit;
    // The first sums are as much work as the boundary at its coarsest
    // (see above), up to a ceiling of their own.
    work.limit = budget.max(FIRST_SUMS_WORK);
    let coarsest = |_: usize, _: usize, piece: &Piece| {
        let (low, high) = (piece.start.min(piece.end), piece.start.max(piece.end));
        Some(panels(piece.curve, (low, high), &piece.crossings, 0))
    };
    let (mut first, panels_at) = sampled(outline, surface, index, (rule, coarse), &coarsest, work)?;
    let plan = match outline.poled {
        Some(poled) => Plan::Poles(poled),
        None => plan(face, surface, index, &mut first, sense)?,
    };
    let by_loop = plan.up_to_whole_surfaces();
    if !by_loop {
        return cell_by_cell(
            (face, surface, outline, index),
            (rule, coarse),
            (sense, &plan),
            (&mut first, &panels_at),
            (start, budget),
            work,
            first_sums,
        );
    }
    if by_loop {
        // Inner integrals may then run beyond the domain of a spline
        // surface, by whole turns.
        work.wrap = spline_closure(surface);
    }
    let key = |loop_index: usize, piece: usize| (loop_index, if by_loop { 0 } else { piece });
    // The samples of each part; a part without any adds nothing.
    let mut round: BTreeMap<(usize, usize), Vec<Sample>> = BTreeMap::new();
    for sample in first {
        round
            .entry(key(sample.loop_index, sample.piece))
            .or_default()
            .push(sample);
    }
    let mut parts: BTreeMap<(usize, usize), Part> = round
        .keys()
        .map(|&k| {
            (
                k,
                Part {
                    level: Some(0),
                    ..Part::default()
                },
            )
        })
        .collect();
    // The first samples were unrolled by the plan; later ones are not.
    let mut unrolled = true;
    // The budget, from the face's first sums: set after them.
    let mut capped = false;
    loop {
        let mut sums = Vec::with_capacity(round.len());
        for (at, mut own) in std::mem::take(&mut round) {
            let Some(doublings) = parts.get(&at).and_then(|part| part.level) else {
                continue;
            };
            let Some(total) = boundary_integral(
                (face, outline),
                surface,
                rule,
                &mut own,
                sense,
                &plan,
                (unrolled, doublings),
                work,
            )
            .map_err(|error| MassError::Geometry { face: index, error })?
            else {
                return Err(unsettled(index));
            };
            let off = own.iter().map(|s| s.off).fold(0.0, f64::max);
            sums.push((at, doublings, total, outline.noise(total.size.area, off)));
        }
        if !capped {
            capped = true;
            let first = work.spent.saturating_sub(start);
            work.limit = budget.max(
                PARTS_WORK
                    .saturating_mul(first)
                    .min(PARTS_WORK.saturating_mul(FIRST_SUMS_WORK)),
            );
        }
        for (at, doublings, total, noise) in sums {
            let Some(part) = parts.get_mut(&at) else {
                continue;
            };
            if part.first.is_none()
                && part
                    .coarser
                    .is_some_and(|before| before.first_agrees_with(&total, noise))
            {
                part.first = Some(total);
            }
            if part
                .coarser
                .is_some_and(|before| before.agrees_with(&total, noise))
            {
                part.settled = Some(total);
                part.noise = noise;
                part.level = None;
            } else if doublings >= MAX_DOUBLINGS {
                return Err(unsettled(index));
            } else {
                part.coarser = Some(total);
                part.level = Some(doublings + 1);
            }
        }
        // Once every part's sums without the second moments agree, those
        // are the face's as they would be without them (see
        // [`integrals_in`]), with the work as it stands.
        if let Some(slot) = first_sums.as_deref_mut()
            && slot.is_none()
            && parts.values().all(|part| part.first.is_some())
        {
            // (The work the whole surface may add there is counted on the
            // copy only, as it is for the whole boundary.)
            let mut alone = work.clone();
            alone.second = false;
            let mut total = Total::default();
            for part in parts.values() {
                if let Some(first) = part.first {
                    total.add(first, 1.0);
                }
            }
            *slot = Some(settle(
                surface,
                index,
                rule,
                (total, plan.up_to_whole_surfaces()),
                &mut alone,
            ));
        }
        if parts.values().all(|part| part.level.is_none()) {
            break;
        }
        if work.spent_up() {
            return Err(unsettled(index));
        }
        for (&at, part) in &parts {
            let Some(level) = part.level else {
                continue;
            };
            let own = samples(
                outline,
                surface,
                index,
                (rule, coarse),
                &|loop_index, piece| (key(loop_index, piece) == at).then_some(level),
                work,
            )?;
            round.insert(at, own);
        }
        unrolled = false;
    }
    let mut total = Total::default();
    for part in parts.into_values() {
        if let Some(settled) = part.settled {
            work.hold(&settled, part.noise);
            total.add(settled, 1.0);
        }
    }
    settle(
        surface,
        index,
        rule,
        (total, plan.up_to_whole_surfaces()),
        work,
    )
}

/// A panel of a piece of the boundary refined on its own
/// ([`cell_by_cell`]), and its sum.
#[derive(Clone, Copy)]
struct Cell {
    /// Its loop, and its piece in the loop.
    key: (usize, usize),
    panel: Panel,
    /// How often the panels the piece needs were halved for it.
    level: u32,
    /// How often the panels of its inner integrals were doubled
    /// ([`inner`]): `level`, up to [`MAX_DOUBLINGS`] (see [`cell_by_cell`]).
    inner: u32,
    total: Total,
    /// Past [`MAX_DOUBLINGS`]: the same sum with half the inner panels.
    inner_check: Option<Total>,
    /// How far its samples were found from their feet, at most.
    off: f64,
}

/// An open cell and its finer cells, in one round of [`cell_by_cell`].
type Opened = (Cell, Vec<Cell>);

/// A sum that was taken: the coarser sum of some cells, and the finer
/// cells that agreed with it.
struct Group {
    key: (usize, usize),
    coarser: Total,
    finer: Vec<Cell>,
    /// The share of rounding it was last held with.
    share: f64,
}

/// How often a panel may be halved, from the panels its piece needs: in
/// a panel 2⁻⁴⁰ of the piece wide the sixteen nodes are some 2⁻⁴⁷ of it
/// apart, a few dozen units in the last place of a parameter of the
/// piece's size; much finer and the nodes, and with them the sums, are
/// rounding.
const MAX_LEVEL: u32 = 40;

/// The cells finer than `cell`: its two halves, or, for a panel with the
/// coarse rule, the same panel with the full one (see [`panels`]); none
/// for a panel of no width, and itself for one too narrow to halve.
/// `None` past [`MAX_LEVEL`], or for a panel that is not one.
fn finer(cell: &Cell) -> Option<Vec<Panel>> {
    let Panel { start, end, coarse } = cell.panel;
    // A panel of no width (a piece cut where it ends) sums to nothing,
    // and so does nothing finer.
    if start == end {
        return Some(Vec::new());
    }
    if coarse {
        return Some(vec![Panel {
            start,
            end,
            coarse: false,
        }]);
    }
    if cell.level >= MAX_LEVEL {
        return None;
    }
    let middle = start + 0.5 * (end - start);
    if !(middle > start && middle < end) {
        // As narrow as its parameter allows (a piece cut a unit in the
        // last place before its end): the same panel, with only its inner
        // integrals refined.
        return (start < end).then_some(vec![Panel {
            start,
            end,
            coarse: false,
        }]);
    }
    Some(vec![
        Panel {
            start,
            end: middle,
            coarse: false,
        },
        Panel {
            start: middle,
            end,
            coarse: false,
        },
    ])
}

/// The sums of the panels `wanted` (each with its key and level), as
/// cells, in the order given.
#[allow(clippy::too_many_arguments)]
fn cells_of(
    (face, surface, outline, index): (&Face, &Surface, &Outline, usize),
    (rule, coarse): (&Rule, &Rule),
    (sense, plan): (f64, &Plan),
    wanted: &[((usize, usize), Panel, u32, u32)],
    work: &mut Work,
) -> Result<Vec<Cell>, MassError> {
    // Per piece in increasing order of its parameter, as `sampled` takes
    // them; each remembers where it was asked for.
    let mut by_piece: BTreeMap<(usize, usize), Vec<(usize, Panel)>> = BTreeMap::new();
    for (at, &(key, panel, _, _)) in wanted.iter().enumerate() {
        by_piece.entry(key).or_default().push((at, panel));
    }
    for panels in by_piece.values_mut() {
        panels.sort_by(|a, b| a.1.start.total_cmp(&b.1.start));
    }
    let which = |loop_index: usize, piece: usize, _: &Piece| {
        by_piece
            .get(&(loop_index, piece))
            .map(|panels| panels.iter().map(|&(_, panel)| panel).collect())
    };
    let (mut samples, panels_at) = sampled(outline, surface, index, (rule, coarse), &which, work)?;
    let mut seen: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut found: Vec<Option<Cell>> = vec![None; wanted.len()];
    for at in panels_at {
        let key = (at.loop_index, at.piece);
        let count = seen.entry(key).or_insert(0);
        let asked = by_piece.get(&key).and_then(|p| p.get(*count)).map(|p| p.0);
        *count += 1;
        let (Some(asked), Some(&(_, _, level, doublings))) =
            (asked, asked.and_then(|a| wanted.get(a)))
        else {
            return Err(MassError::Invalid { face: index });
        };
        let Some(own) = samples.get_mut(at.range) else {
            return Err(MassError::Invalid { face: index });
        };
        let mut sum = |doublings: u32, own: &mut [Sample]| {
            boundary_integral(
                (face, outline),
                surface,
                rule,
                own,
                sense,
                plan,
                (true, doublings),
                work,
            )
            .map_err(|error| MassError::Geometry { face: index, error })?
            .ok_or(unsettled(index))
        };
        let total = sum(doublings, own)?;
        let inner_check = if level > MAX_DOUBLINGS {
            Some(sum(doublings.saturating_sub(1), own)?)
        } else {
            None
        };
        let off = own.iter().map(|s| s.off).fold(0.0, f64::max);
        if let Some(slot) = found.get_mut(asked) {
            *slot = Some(Cell {
                key,
                panel: at.panel,
                level,
                inner: doublings,
                total,
                inner_check,
                off,
            });
        }
    }
    found
        .into_iter()
        .map(|cell| cell.ok_or(MassError::Invalid { face: index }))
        .collect()
}

/// [`part_by_part`] for a boundary integral that is a sum of one term
/// per sample along each piece (every plan but the unrolled one, whose
/// loops are unrolled as wholes): refined panel by panel, adaptively.
///
/// Each panel of a piece at its coarsest is a cell. A round sums every
/// open cell's finer cells (its halves, [`finer`]) and holds them to it:
/// first all of a piece's open cells together, as the piece's sums were
/// held before (2026-10-17), and where those do not agree each cell on
/// its own. The cells that agree are taken, the finer ones in place of
/// the coarser, and are not summed again; the halves of the others are
/// open in the next round. So the panels end up as fine as the integrand
/// needs where it needs it, and nowhere else: a boundary integrand that
/// gathers on a stretch a millionth of its edge long (the strips of a
/// rational patch with weights a thousand apart, next to its light
/// corner) takes some twenty halvings there, where doubling every panel
/// would take twenty doublings of the whole edge. Every comparison is of
/// sums that differ in everything that is refined: up to
/// [`MAX_DOUBLINGS`] the inner integrals double with each halving; past
/// it, where they do not, each finer cell is also summed with half its
/// inner panels and must agree with that too. A cell's allowance is the
/// share of its size the piece's is ([`CONVERGED`] and
/// [`Outline::noise`] of the piece's sums), so the allowances of the cells
/// a piece is taken in add up to the piece's.
///
/// With the second moments summed, the cells are first refined until the
/// area, the volume and the moments agree, exactly as without them (the
/// same cells, the same sums, the same work), which gives those sums;
/// then every taken sum whose second moments do not agree is opened
/// again and refined until they do.
#[allow(clippy::too_many_arguments)]
fn cell_by_cell(
    (face, surface, outline, index): (&Face, &Surface, &Outline, usize),
    (rule, coarse): (&Rule, &Rule),
    (sense, plan): (f64, &Plan),
    (first, panels_at): (&mut [Sample], &[Sampled]),
    (start, budget): (u64, u64),
    work: &mut Work,
    mut first_sums: Option<&mut Option<Result<Sums, MassError>>>,
) -> Result<Sums, MassError> {
    let at = (face, surface, outline, index);
    let mut open = Vec::with_capacity(panels_at.len());
    for at in panels_at {
        let Some(own) = first.get_mut(at.range.clone()) else {
            return Err(MassError::Invalid { face: index });
        };
        let total = boundary_integral(
            (face, outline),
            surface,
            rule,
            own,
            sense,
            plan,
            (true, 0),
            work,
        )
        .map_err(|error| MassError::Geometry { face: index, error })?
        .ok_or(unsettled(index))?;
        open.push(Cell {
            key: (at.loop_index, at.piece),
            panel: at.panel,
            level: 0,
            inner: 0,
            total,
            inner_check: None,
            off: own.iter().map(|s| s.off).fold(0.0, f64::max),
        });
    }
    // The budget, from the face's first sums (see `part_by_part`).
    let spent = work.spent.saturating_sub(start);
    work.limit = budget.max(
        PARTS_WORK
            .saturating_mul(spent)
            .min(PARTS_WORK.saturating_mul(FIRST_SUMS_WORK)),
    );
    // How wide each piece may be refined at once: as many open cells as
    // panels its sums had at the last doubling when every panel was
    // doubled (2^MAX_DOUBLINGS times its first panels).
    let mut widest: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for cell in &open {
        *widest.entry(cell.key).or_insert(0) += 1 << MAX_DOUBLINGS;
    }
    let mut taken: Vec<Group> = Vec::new();
    for parts in [FIRST_SUMS, SUMS] {
        if parts == SUMS {
            // Opened again where the second moments do not agree.
            let shares: Vec<f64> = taken
                .iter()
                .map(|group| piece_share(outline, &taken, &[], group.key))
                .collect();
            let mut kept = Vec::with_capacity(taken.len());
            for (mut group, share) in taken.into_iter().zip(shares) {
                if holds(&group.coarser, &group.finer, share, parts) {
                    group.share = share;
                    kept.push(group);
                } else {
                    open.extend(group.finer);
                }
            }
            taken = kept;
        }
        while !open.is_empty() {
            if work.spent_up() {
                return Err(unsettled(index));
            }
            // A piece with more open cells than that is not closing in on
            // a place where its integrand gathers: its sums scatter
            // (rounding, or a boundary through where the parametrisation
            // collapses) however fine the panels.
            let mut wide: BTreeMap<(usize, usize), usize> = BTreeMap::new();
            for cell in &open {
                let count = wide.entry(cell.key).or_insert(0);
                *count += 1;
                if *count > widest.get(&cell.key).copied().unwrap_or(0) {
                    return Err(unsettled(index));
                }
            }
            let mut wanted = Vec::new();
            let mut spans = Vec::with_capacity(open.len());
            for cell in &open {
                let Some(halves) = finer(cell) else {
                    return Err(unsettled(index));
                };
                let level = cell.level + 1;
                let doublings = if level <= MAX_DOUBLINGS {
                    level
                } else {
                    // Past the doublings, the inner panels go where the
                    // cell's own two sums of them say: half as many once
                    // they agreed, twice as many where they did not (up
                    // to the most an inner integral takes), as many where
                    // there was nothing to say. Never below two a
                    // stretch, so that half of them is one.
                    let share = piece_share(outline, &taken, &open, cell.key);
                    match cell.inner_check {
                        Some(half) if half.agrees_in(&cell.total, share, parts) => {
                            cell.inner.saturating_sub(1).max(1)
                        }
                        Some(_) => (cell.inner + 1).min(MAX_DOUBLINGS),
                        None => cell.inner,
                    }
                };
                let from = wanted.len();
                wanted.extend(halves.into_iter().map(|p| (cell.key, p, level, doublings)));
                spans.push(from..wanted.len());
            }
            let finer_cells = cells_of(at, (rule, coarse), (sense, plan), &wanted, work)?;
            // Per piece: its open cells and their finer ones.
            let mut pieces: BTreeMap<(usize, usize), Vec<Opened>> = BTreeMap::new();
            for (cell, span) in open.iter().zip(spans) {
                let Some(halves) = finer_cells.get(span) else {
                    return Err(MassError::Invalid { face: index });
                };
                pieces
                    .entry(cell.key)
                    .or_default()
                    .push((*cell, halves.to_vec()));
            }
            let mut next = Vec::new();
            for (key, cells) in pieces {
                let all_finer: Vec<Cell> =
                    cells.iter().flat_map(|(_, f)| f.iter().copied()).collect();
                let share = piece_share(outline, &taken, &all_finer, key);
                let mut coarser = Total::default();
                for (cell, _) in &cells {
                    coarser.add(cell.total, 1.0);
                }
                if holds(&coarser, &all_finer, share, parts) {
                    taken.push(Group {
                        key,
                        coarser,
                        finer: all_finer,
                        share,
                    });
                    continue;
                }
                for (cell, halves) in cells {
                    if holds(&cell.total, &halves, share, parts) {
                        taken.push(Group {
                            key,
                            coarser: cell.total,
                            finer: halves,
                            share,
                        });
                    } else {
                        next.extend(halves);
                    }
                }
            }
            open = next;
        }
        if parts == FIRST_SUMS
            && let Some(slot) = first_sums.as_deref_mut()
        {
            // The sums without the second moments, with the work as it
            // stands (see [`integrals_in`]).
            let mut alone = work.clone();
            alone.second = false;
            *slot = Some(settle(
                surface,
                index,
                rule,
                (sum_of(&taken), plan.up_to_whole_surfaces()),
                &mut alone,
            ));
        }
    }
    for group in &taken {
        let mut finer = Total::default();
        for cell in &group.finer {
            finer.add(cell.total, 1.0);
        }
        work.hold(&finer, group.share);
    }
    settle(
        surface,
        index,
        rule,
        (sum_of(&taken), plan.up_to_whole_surfaces()),
        work,
    )
}

/// The sum of the finer cells of every taken group.
fn sum_of(taken: &[Group]) -> Total {
    let mut total = Total::default();
    for group in taken {
        for cell in &group.finer {
            total.add(cell.total, 1.0);
        }
    }
    total
}

/// The share of their size that cells of the piece `key` are allowed to
/// differ by beyond [`CONVERGED`]: [`Outline::noise`] of what is summed
/// for the piece so far (its taken cells and `more`).
fn piece_share(outline: &Outline, taken: &[Group], more: &[Cell], key: (usize, usize)) -> f64 {
    let (mut area, mut off) = (0.0, 0.0_f64);
    let cells = taken
        .iter()
        .filter(|g| g.key == key)
        .flat_map(|g| g.finer.iter())
        .chain(more.iter().filter(|c| c.key == key));
    for cell in cells {
        area += cell.total.size.area;
        off = off.max(cell.off);
    }
    outline.noise(area, off)
}

/// Whether the cells `finer` agree with the coarser sum `coarser` in the
/// first `parts` sums, and each of them past [`MAX_DOUBLINGS`] with its
/// own sum of half the inner panels.
fn holds(coarser: &Total, finer: &[Cell], share: f64, parts: usize) -> bool {
    let mut total = Total::default();
    for cell in finer {
        total.add(cell.total, 1.0);
    }
    coarser.agrees_in(&total, share, parts)
        && finer.iter().all(|cell| {
            cell.inner_check
                .is_none_or(|half| half.agrees_in(&cell.total, share, parts))
        })
}

/// How big the body is: the extent of its vertices and of the ends and
/// middles of its edges, across. Zero for a body without any (a closed
/// surface without loops), or whose points are not finite.
#[cfg(test)]
fn body_size(body: &Body) -> f64 {
    body_extent(body).0
}

/// [`body_size`], and the middle of that extent: the point the volume and
/// the moments are taken about. The origin for a body without any.
fn body_extent(body: &Body) -> (f64, Vec3) {
    let mut extent = Bbox3::EMPTY;
    for vertex in &body.vertices {
        if vertex.point.is_finite() {
            extent.add(vertex.point);
        }
    }
    for edge in &body.edges {
        let (t0, t1) = edge.range;
        for t in [t0, 0.5 * (t0 + t1), t1] {
            let point = edge.curve.eval(t);
            if point.is_finite() {
                extent.add(point);
            }
        }
    }
    let size = extent.diagonal();
    // A face without loops is a whole closed surface (a sphere, a torus)
    // and has neither vertices nor edges to say where it is: six of its
    // points do.
    let mut around = extent;
    for face in body.faces.iter().filter(|face| face.loops.is_empty()) {
        for (u, v) in [0.0, 0.5 * PI, PI, 1.5 * PI]
            .map(|u| (u, 0.0))
            .into_iter()
            .chain([(0.0, 0.5 * PI), (0.0, -0.5 * PI)])
        {
            let point = face.surface.eval(u, v);
            if point.is_finite() {
                around.add(point);
            }
        }
    }
    let middle = around.center().filter(|middle| middle.is_finite());
    (
        if size.is_finite() { size } else { 0.0 },
        middle.unwrap_or(Vec3::ZERO),
    )
}

/// Fewest faces worth a second thread: below this, starting threads costs
/// more than it saves.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const PARALLEL_FACES: usize = 8;

/// `work(0..count)`, in that order. Faces are handed out one at a time to
/// as many threads as the machine has, so one slow face does not hold up
/// a share of the others; each result goes to its own slot. Where no
/// thread can be started (wasm, a machine out of threads) the faces are
/// done one after the other on this thread, with the same results.
fn each_face<T: Send>(count: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Mutex, PoisonError};
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(count / PARALLEL_FACES.max(1) + 1)
            .min(count);
        if threads > 1 {
            let next = AtomicUsize::new(0);
            let slots: Vec<Mutex<Option<T>>> = (0..count).map(|_| Mutex::new(None)).collect();
            let run = || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(slot) = slots.get(index) else {
                        break;
                    };
                    let result = work(index);
                    *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
                }
            };
            std::thread::scope(|scope| {
                // This thread works too; helpers that cannot be started
                // leave more for it.
                let helpers: Vec<_> = (1..threads)
                    .filter_map(|_| std::thread::Builder::new().spawn_scoped(scope, run).ok())
                    .collect();
                run();
                for helper in helpers {
                    // A panic on a helper leaves its face's slot empty;
                    // that face is done again below, where the panic
                    // surfaces on this thread as it would have.
                    let _ = helper.join();
                }
            });
            return slots
                .into_iter()
                .enumerate()
                .map(|(index, slot)| {
                    slot.into_inner()
                        .unwrap_or_else(PoisonError::into_inner)
                        .unwrap_or_else(|| work(index))
                })
                .collect();
        }
    }
    (0..count).map(work).collect()
}

/// Area, volume and centroid of `body`. The body should pass
/// [`crate::check`]; on an invalid one the result is meaningless or an
/// error, never a panic.
///
/// Refused with [`MassError::Unsupported`], never guessed: a boundary
/// through two poles of a spline surface that also crosses the seam where
/// the surface closes on itself, a boundary that winds around an axis more
/// than once, torus loops that wind around the
/// axis and the tube at once, a face of a sphere or a torus too thin (or
/// too nearly the whole surface) to tell which side of its loops it is
/// on, and a face whose boundary integral does not settle, or has not
/// within the work a face may take (see the module documentation: the
/// time is bounded). A face that is outside its own loops is
/// [`MassError::UnboundedFace`].
pub fn mass_properties(body: &Body) -> Result<MassProperties, MassError> {
    let (all, _, middle, _) = integrate(body, Wanted::First);
    Ok(mass_from(body, all?, middle))
}

/// How far [`MassProperties::area`] and [`MassProperties::volume`] can be
/// off by the integrator's own account ([`mass_bound`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MassBound {
    pub area: f64,
    pub volume: f64,
}

/// [`mass_properties`] of `body` (the very same value or error) and how
/// far its area and volume can be off by the integrator's own account:
/// every sum it takes is refined until two in a row agree to an allowance
/// ([`CONVERGED`] of the size of what was summed, and the share of it that
/// is rounding), and the bound is the sum of those allowances over every
/// sum taken (per face, per part of a face's boundary, per panel group
/// and per whole surface added or taken away), plus the rounding of
/// adding up the faces. The sums converge geometrically, so the finer of
/// two that agree is off by far less than they differ by; the bound is
/// what they were allowed to differ by. Of a sheet the volume is zero
/// and so is its bound.
pub fn mass_bound(body: &Body) -> Result<(MassProperties, MassBound), MassError> {
    let (all, _, middle, held) = integrate(body, Wanted::First);
    let mass = mass_from(body, all?, middle);
    let solid = body.kind == BodyKind::Solid;
    Ok((
        mass,
        MassBound {
            area: held[0],
            volume: if solid { held[1] } else { 0.0 },
        },
    ))
}

/// [`MassProperties`] from the sums of the faces.
fn mass_from(body: &Body, total: Sums, middle: Vec3) -> MassProperties {
    let solid = body.kind == BodyKind::Solid;
    MassProperties {
        area: total.area,
        volume: if solid { total.volume } else { 0.0 },
        centroid: (solid && total.volume != 0.0)
            .then(|| middle + total.moment * (1.0 / total.volume)),
    }
}

/// [`mass_properties`] and [`inertia`] of `body` at once, each the very
/// value (or error) the function on its own gives, in about the time of
/// [`inertia`] alone: the faces are integrated once, with the second
/// moments, and the sums without them are taken where they settle (see
/// [`face_integrals`]). Of a body that is not a solid only the first is
/// measured.
#[allow(clippy::type_complexity)]
pub fn mass_and_inertia(
    body: &Body,
) -> (
    Result<MassProperties, MassError>,
    Result<Inertia, MassError>,
) {
    if body.kind != BodyKind::Solid {
        return (mass_properties(body), Err(NOT_A_SOLID));
    }
    let (all, first, middle, _) = integrate(body, Wanted::Both);
    let mass = first
        .unwrap_or_else(|| all.clone())
        .map(|total| mass_from(body, total, middle));
    (mass, all.and_then(|total| inertia_from(total, middle)))
}

/// The sums over every face of `body`, outward, and the point the
/// moments are taken about: those asked for, and with [`Wanted::Both`]
/// beside them those of [`Wanted::First`].
#[allow(clippy::type_complexity)]
fn integrate(
    body: &Body,
    sums: Wanted,
) -> (
    Result<Sums, MassError>,
    Option<Result<Sums, MassError>>,
    Vec3,
    [f64; 2],
) {
    let rule = Rule::new(NODES);
    let (size, middle) = body_extent(body);
    // Every face's integrals on their own, on several threads where there
    // are any; summed below in the faces' order, so the result is the same
    // bits whatever the number of threads (AGENTS.md rule 5), and the
    // error is the first face's that has one, as one face after another
    // would give.
    let per_face = each_face(body.faces.len(), |index| match body.faces.get(index) {
        Some(face) => face_integrals(body, face, index, &rule, (size, middle), sums),
        None => {
            let lost = MassError::Unsupported {
                face: index,
                what: "a face that is not in the body",
            };
            FaceSums {
                first: (sums == Wanted::Both).then(|| Err(lost.clone())),
                all: Err(lost),
                held: [0.0; 2],
            }
        }
    });
    let mut all: Vec<Result<Sums, MassError>> = Vec::with_capacity(per_face.len());
    let mut first: Vec<Result<Sums, MassError>> = Vec::new();
    // What the faces' sums were held to, and how large they are: adding
    // them up rounds each partial sum, by at most a unit in the last place
    // of the largest it reaches, once per face.
    let mut held = [0.0_f64; 2];
    let mut largest = [0.0_f64; 2];
    for face in per_face {
        held[0] += face.held[0];
        held[1] += face.held[1];
        if let Ok(sums) = &face.all {
            largest[0] += sums.area.abs();
            largest[1] += sums.volume.abs();
        }
        all.push(face.all);
        if let Some(sums) = face.first {
            first.push(sums);
        }
    }
    let faces = body.faces.len() as f64;
    let held = [
        held[0] + faces * f64::EPSILON * largest[0],
        held[1] + faces * f64::EPSILON * largest[1],
    ];
    let first = (sums == Wanted::Both).then(|| total_of(body, first));
    (total_of(body, all), first, middle, held)
}

/// The sums of the faces of `body` (one per face, in order), outward: the
/// first face's error if one has one.
fn total_of(body: &Body, per_face: Vec<Result<Sums, MassError>>) -> Result<Sums, MassError> {
    let mut total = Sums::default();
    for (face, sums) in body.faces.iter().zip(per_face) {
        let sums = sums?;
        // Area counts whichever way the face points; volume and moments
        // follow the outward normal.
        let outward = if face.reversed { -1.0 } else { 1.0 };
        let area = total.area + sums.area;
        total.add_scaled(sums, outward);
        total.area = area;
    }
    if !total.parts().iter().all(|x| x.is_finite()) {
        return Err(MassError::Unsupported {
            face: 0,
            what: "geometry whose integrals are not finite",
        });
    }
    Ok(total)
}

/// The volume, the centroid and the inertia tensor of a solid of density
/// 1, from the exact B-rep like [`mass_properties`]. Times a density it
/// is the body's mass and inertia tensor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inertia {
    pub volume: f64,
    pub centroid: Vec3,
    /// About the centroid, along the model's axes: `I[i][i] = ∫(r² - x_i²)`,
    /// `I[i][j] = -∫x_i x_j` (in length to the fifth power).
    pub tensor: [[f64; 3]; 3],
}

/// [`Inertia`] of a solid. Refused like [`mass_properties`], and for a
/// sheet or a body of no volume, which have no centroid. The second
/// moments are summed about the middle of the body and moved to the
/// centroid by the parallel-axis rule, so a small body far from the
/// origin loses nothing to cancellation.
pub fn inertia(body: &Body) -> Result<Inertia, MassError> {
    if body.kind != BodyKind::Solid {
        return Err(NOT_A_SOLID);
    }
    let (all, _, middle, _) = integrate(body, Wanted::Second);
    inertia_from(all?, middle)
}

const NOT_A_SOLID: MassError = MassError::Unsupported {
    face: 0,
    what: "a body that is not a solid, which has no inertia",
};

/// [`Inertia`] from the sums of the faces of a solid, with the second
/// moments.
fn inertia_from(total: Sums, middle: Vec3) -> Result<Inertia, MassError> {
    let volume = total.volume;
    if volume == 0.0 {
        return Err(MassError::Unsupported {
            face: 0,
            what: "a solid of no volume, which has no centroid",
        });
    }
    // The centroid's offset from the middle.
    let c = total.moment * (1.0 / volume);
    let [sxx, syy, szz, sxy, syz, szx] = total.second;
    // About the centroid: ∫(x - cx)² = ∫x² - V cx², ∫(x - cx)(y - cy) =
    // ∫xy - V cx cy.
    let xx = sxx - volume * c.x * c.x;
    let yy = syy - volume * c.y * c.y;
    let zz = szz - volume * c.z * c.z;
    let xy = sxy - volume * c.x * c.y;
    let yz = syz - volume * c.y * c.z;
    let zx = szx - volume * c.z * c.x;
    let tensor = [
        [yy + zz, -xy, -zx],
        [-xy, xx + zz, -yz],
        [-zx, -yz, xx + yy],
    ];
    if !tensor.iter().flatten().all(|x| x.is_finite()) {
        return Err(MassError::Unsupported {
            face: 0,
            what: "geometry whose integrals are not finite",
        });
    }
    Ok(Inertia {
        volume,
        centroid: middle + c,
        tensor,
    })
}

#[cfg(test)]
mod tests {
    //! The parts of the integrator that no body reaches from outside,
    //! because it chooses a sphere face's frame itself: what a frame does
    //! that it would not choose.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::unreachable
    )]

    use super::*;
    use crate::{Body, BodyKind, Coedge};
    use craft3d_geom::{Circle, Cylinder, Ellipse};
    use std::f64::consts::{PI, TAU};

    const RADIUS: f64 = 2.0;
    const CENTRE: Vec3 = Vec3::new(0.3, -1.1, 0.7);

    /// Integrating a face is counted, with the evaluations it took.
    /// (Other tests add to the counts at the same time: only growth is
    /// certain.)
    #[test]
    fn integrating_is_counted() {
        let (faces, spent) = mass_work();
        let body = cap(0.5);
        mass_properties(&body).ok();
        let (faces_after, spent_after) = mass_work();
        assert!(faces_after > faces);
        assert!(spent_after > spent);
    }

    /// A cap of the sphere about `CENTRE` beyond the plane `level` along
    /// z, as a sheet of one face; its area is 2πr (r − level).
    fn cap(level: f64) -> Body {
        let mut body = Body::new(BodyKind::Sheet);
        let rim = (RADIUS * RADIUS - level * level).sqrt();
        let frame = Frame::WORLD
            .at(CENTRE + Vec3::new(0.0, 0.0, level))
            .unwrap();
        let vertex = body.add_vertex(frame.point(rim, 0.0, 0.0));
        let edge = body.add_edge(
            Curve3::Circle(Circle::new(frame, rim).unwrap()),
            (0.0, TAU),
            vertex,
            vertex,
        );
        let loop_ = body.add_loop(vec![Coedge {
            edge,
            reversed: false,
        }]);
        let sphere = Sphere::new(Frame::WORLD.at(CENTRE).unwrap(), RADIUS).unwrap();
        let face = body.add_face(Surface::Sphere(sphere), false, vec![loop_]);
        body.add_shell(vec![face]);
        body
    }

    /// The cap with its rim moved `out` off the sphere, outward in its
    /// plane.
    fn cap_off(level: f64, out: f64) -> Body {
        let mut body = cap(level);
        let rim = (RADIUS * RADIUS - level * level).sqrt() + out;
        let frame = Frame::WORLD
            .at(CENTRE + Vec3::new(0.0, 0.0, level))
            .unwrap();
        body.edges[0].curve = Curve3::Circle(Circle::new(frame, rim).unwrap());
        body.vertices[0].point = frame.point(rim, 0.0, 0.0);
        body
    }

    /// The allowance a boundary earns by lying off its surface is bounded
    /// by how far a boundary point may be found from the surface and count
    /// as on it (`touch`): a rim a millionth of the radius off the sphere is
    /// measured as the cap it nearly bounds, and no distance, however far
    /// (a wrong foot, an edge off its face), earns more than `touch` does.
    #[test]
    fn the_allowance_for_a_boundary_off_its_surface_is_bounded() {
        let expected = TAU * RADIUS * (RADIUS - 0.5);
        let near = mass_properties(&cap_off(0.5, 1e-6 * RADIUS)).unwrap();
        assert!((near.area - expected).abs() <= 1e-5 * expected, "{near:?}");
        let body = cap(0.5);
        let face = body.faces.first().unwrap();
        let outline = outline(&body, face, &face.surface, 0, body_size(&body)).unwrap();
        let area = 1e-3;
        let bound = outline.noise(area, outline.touch);
        assert!(bound > 0.0 && bound < 1.0, "{bound}");
        for off in [10.0 * outline.touch, 1.0, 1e9, f64::INFINITY] {
            assert_eq!(outline.noise(area, off), bound, "{off}");
        }
    }

    /// The cap's sphere with its north pole `tilt` away from the cap's
    /// middle.
    fn turned(tilt: f64) -> Surface {
        let pole = Vec3::new(tilt.sin() * 0.6, tilt.sin() * 0.8, tilt.cos());
        let frame = Frame::new(CENTRE, pole, Vec3::new(0.8, -0.6, 0.0)).unwrap();
        Surface::Sphere(Sphere::new(frame, RADIUS).unwrap())
    }

    fn area_in(body: &Body, surface: &Surface) -> Result<f64, MassError> {
        let rule = Rule::new(NODES);
        let face = body.faces.first().unwrap();
        let mut work = Work::new(surface);
        integrals_in(
            (body, face, surface, 0),
            &rule,
            body_size(body),
            &mut work,
            None,
        )
        .map(|sums| sums.area)
    }

    /// The pole `G` runs from may be anywhere: in the face, outside it,
    /// on its boundary or a hair from it. `G du` is smooth there.
    #[test]
    fn the_north_pole_may_be_on_the_boundary() {
        for level in [-1.5_f64, 0.0, 1.2] {
            let body = cap(level);
            let expected = TAU * RADIUS * (RADIUS - level);
            let rim = (level / RADIUS).acos();
            let mut tilts = vec![0.0, rim / 2.0, rim, (rim + PI) / 2.0];
            for off in [1e-13, 1e-10, 1e-7, 1e-5, 1e-3, 0.1] {
                tilts.extend([rim - off, rim + off]);
            }
            for tilt in tilts {
                // The other pole stays clear of the boundary by 0.2.
                if (PI - tilt - rim).abs() < 0.2 {
                    continue;
                }
                let area = area_in(&body, &turned(tilt)).unwrap();
                assert!(
                    (area - expected).abs() <= 1e-11 * expected,
                    "level {level}, pole {tilt}: {area} vs {expected}"
                );
            }
        }
    }

    /// The other pole may not: no `G` is smooth at both. A boundary that
    /// passes close to it takes more panels, and one that passes through
    /// it, or closer than the panels can be made fine, is refused. What
    /// is never returned is a number that is not the area.
    #[test]
    fn the_south_pole_on_the_boundary_is_refused_not_guessed() {
        let level = 0.8_f64;
        let body = cap(level);
        let expected = TAU * RADIUS * (RADIUS - level);
        let rim = (level / RADIUS).acos();
        let mut refused = 0;
        let mut measured = 0;
        for off in [
            0.0, 1e-13, -1e-13, 1e-9, -1e-9, 1e-6, -1e-6, 1e-4, -1e-4, 1e-3, -1e-3, 1e-2, -1e-2,
            0.1, -0.1, 0.5, -0.5,
        ] {
            // The south pole `off` outside the rim (inside, if negative).
            match area_in(&body, &turned(PI - rim - off)) {
                Ok(area) => {
                    measured += 1;
                    assert!(
                        (area - expected).abs() <= 1e-11 * expected,
                        "south pole {off} from the rim: {area} vs {expected}"
                    );
                }
                Err(MassError::Unsupported { what, .. }) => {
                    refused += 1;
                    assert!(what.contains("does not settle"), "{what}");
                    // The finest two sums have panels 0.025 and 0.012 wide
                    // and must agree: that takes more room than 0.01.
                    assert!(off.abs() <= 1e-2, "refused {off} from the rim");
                }
                Err(other) => panic!("south pole {off} from the rim: {other}"),
            }
        }
        // Through the pole it cannot be measured; a tenth away it can.
        assert!(
            refused >= 1 && measured >= 4,
            "{refused} refused, {measured} measured"
        );
    }

    /// The frame chosen for a face keeps its south pole away from the
    /// boundary: opposite the middle of a cap, whatever frame the sphere
    /// came with.
    #[test]
    fn the_chosen_frame_has_its_south_pole_clear_of_the_boundary() {
        for level in [-1.9_f64, -0.5, 0.0, 0.5, 1.9] {
            for tilt in [0.0, 0.4, (level / RADIUS).acos(), 2.0, PI] {
                let mut body = cap(level);
                body.faces[0].surface = turned(tilt);
                let Surface::Sphere(given) = &body.faces[0].surface else {
                    unreachable!()
                };
                let Some(Surface::Sphere(own)) = own_sphere(&body, &body.faces[0], given) else {
                    panic!("no frame for level {level}");
                };
                // North is the pole nearer the rim: +z above the equator.
                let north = own.frame().z();
                let expected = if level >= 0.0 { 1.0 } else { -1.0 };
                assert!(
                    (north.z - expected).abs() < 1e-12,
                    "level {level}, tilt {tilt}: north {north:?}"
                );
                assert_eq!(own.frame().origin(), CENTRE);
                assert_eq!(own.radius(), RADIUS);
            }
        }
    }

    /// A band of the cylinder of radius `RADIUS` about the z axis through
    /// `CENTRE` between two parallel planes, which cut the axis at `level`
    /// and `width` further up, as a sheet of one face. Its area is 2πr
    /// times `width`, however the planes are tilted: by a quarter of
    /// `width` (in radians), so that the rims wander across the band by
    /// about its width and every boundary point has a height of its own.
    fn band(level: f64, width: f64) -> Body {
        let tilt = 0.25 * width;
        let mut body = Body::new(BodyKind::Sheet);
        let mut loops = Vec::new();
        for (z, reversed) in [(level, false), (level + width, true)] {
            let frame = Frame::new(
                CENTRE + Vec3::new(0.0, 0.0, z),
                Vec3::new(0.0, -tilt.sin(), tilt.cos()),
                Vec3::X,
            )
            .unwrap();
            let vertex = body.add_vertex(frame.point(RADIUS, 0.0, 0.0));
            let edge = body.add_edge(
                Curve3::Ellipse(Ellipse::new(frame, RADIUS, RADIUS / tilt.cos()).unwrap()),
                (0.0, TAU),
                vertex,
                vertex,
            );
            loops.push(body.add_loop(vec![Coedge { edge, reversed }]));
        }
        let cylinder = Cylinder::new(Frame::WORLD.at(CENTRE).unwrap(), RADIUS).unwrap();
        let face = body.add_face(Surface::Cylinder(cylinder), false, loops);
        body.add_shell(vec![face]);
        body
    }

    /// A strip is known to as many digits as its width is: its sums
    /// agree to the rounding of its coordinates over the width, not over
    /// its length, and it is measured to that instead of being doubled
    /// until it is given up.
    #[test]
    fn a_strip_settles_to_the_rounding_over_its_width() {
        for level in [0.0, 1.3, 40.0] {
            for width in [1.0, 1e-3, 1e-5, 1e-7, 1e-9] {
                let body = band(level, width);
                let surface = body.faces[0].surface.clone();
                let area = area_in(&body, &surface)
                    .unwrap_or_else(|error| panic!("level {level}, width {width}: {error}"));
                // The top of the band is `level + width` rounded.
                let top = CENTRE.z + level + width;
                let expected = TAU * RADIUS * (top - (CENTRE.z + level));
                let known = 1e-11 + 8.0 * f64::EPSILON * top.abs().max(RADIUS) / width;
                assert!(
                    (area - expected).abs() <= known * expected,
                    "level {level}, width {width}: {area} vs {expected}"
                );
            }
        }
    }

    /// A face that has taken its budget is given up with the error of a
    /// boundary integral that does not settle, wherever it is at.
    #[test]
    fn a_face_past_its_budget_is_refused() {
        let body = cap(0.8);
        let surface = turned(0.3);
        let face = body.faces.first().unwrap();
        let rule = Rule::new(NODES);
        let run = |spent: u64| {
            let mut work = Work {
                spent,
                each: 1,
                about: Vec3::ZERO,
                second: false,
                wrap: [None, None],
                limit: BUDGET,
                held: [0.0; 2],
            };
            let sums = integrals_in(
                (&body, face, &surface, 0),
                &rule,
                body_size(&body),
                &mut work,
                None,
            );
            (sums.map(|sums| sums.area), work.spent)
        };
        let (area, taken) = run(0);
        let expected = TAU * RADIUS * (RADIUS - 0.8);
        assert!((area.unwrap() - expected).abs() <= 1e-11 * expected);
        assert!(taken > 0 && taken < BUDGET / 1000, "{taken}");
        // With all but half of that left, it stops on the way.
        let (refused, spent) = run(BUDGET - taken / 2);
        match refused {
            Err(MassError::Unsupported { what, .. }) => {
                assert!(what.contains("does not settle"), "{what}")
            }
            other => panic!("{other:?}"),
        }
        assert!(spent > BUDGET && spent <= BUDGET + taken, "{spent}");
        // And a whole surface likewise.
        let mut work = Work {
            spent: BUDGET + 1,
            each: 1,
            about: Vec3::ZERO,
            second: false,
            wrap: [None, None],
            limit: BUDGET,
            held: [0.0; 2],
        };
        assert!(matches!(
            whole_surface(&surface, 0, &rule, &mut work),
            Err(MassError::Unsupported { .. })
        ));
    }

    /// The cap's rim in `pieces` arcs.
    fn cap_in_pieces(level: f64, pieces: u32) -> Body {
        let mut body = Body::new(BodyKind::Sheet);
        let rim = (RADIUS * RADIUS - level * level).sqrt();
        let frame = Frame::WORLD
            .at(CENTRE + Vec3::new(0.0, 0.0, level))
            .unwrap();
        let circle = Circle::new(frame, rim).unwrap();
        let at = |k: u32| TAU * f64::from(k) / f64::from(pieces);
        let vertices: Vec<_> = (0..pieces)
            .map(|k| body.add_vertex(frame.point(rim * at(k).cos(), rim * at(k).sin(), 0.0)))
            .collect();
        let mut coedges = Vec::new();
        for k in 0..pieces {
            let (from, to) = (vertices[k as usize], vertices[((k + 1) % pieces) as usize]);
            let edge = body.add_edge(Curve3::Circle(circle), (at(k), at(k + 1)), from, to);
            coedges.push(Coedge {
                edge,
                reversed: false,
            });
        }
        let loop_ = body.add_loop(coedges);
        let sphere = Sphere::new(Frame::WORLD.at(CENTRE).unwrap(), RADIUS).unwrap();
        let face = body.add_face(Surface::Sphere(sphere), false, vec![loop_]);
        body.add_shell(vec![face]);
        body
    }

    /// The work of a face refined part by part is bounded for the face:
    /// the rim of the cap in three pieces, with the sphere's other pole
    /// on it, where `G` is not smooth and the part through it never
    /// settles. The others settle; the face stops when it has taken
    /// [`PARTS_WORK`] times its first sums (or its budget, where that is
    /// more), with no more than one further round on top.
    #[test]
    fn a_face_refined_part_by_part_stops_at_its_cap() {
        let level = 0.8_f64;
        let body = cap_in_pieces(level, 3);
        let rim = (level / RADIUS).acos();
        let surface = turned(PI - rim);
        let face = body.faces.first().unwrap();
        let rule = Rule::new(NODES);
        let run = |limit: u64| {
            let mut work = Work::new(&surface);
            work.limit = limit;
            let result = integrals_in(
                (&body, face, &surface, 0),
                &rule,
                body_size(&body),
                &mut work,
                None,
            );
            match result {
                Err(MassError::Unsupported { what, .. }) => {
                    assert!(what.contains("does not settle"), "{what}")
                }
                other => panic!("{other:?}"),
            }
            work
        };
        // A sphere is cheap: with the whole budget the part through the
        // pole is refused first, when its open cells grow wider than its
        // panels at the last doubling (`cell_by_cell`; until 2026-10-17,
        // when every panel was doubled, it ran out of doublings at under
        // a hundredth of the budget; the cells take 1.15 million, a
        // fifty-eighth).
        let whole = run(BUDGET);
        assert!(whole.spent < BUDGET / 50, "{}", whole.spent);
        // With a budget of one evaluation the face has PARTS_WORK times
        // its first sums, and stops there, with at most one further round
        // on top.
        let capped = run(1);
        let (spent, limit) = (capped.spent, capped.limit);
        assert!(limit > 1 && limit < whole.spent, "{limit}");
        assert!(spent > limit && spent <= 2 * limit, "{spent} of {limit}");
        // (Without the cap the same face goes on: the cap stopped it.)
        assert!(whole.spent > spent, "{spent} against {}", whole.spent);
    }

    /// A face whose whole boundary, refined together, needs more work than
    /// its budget is measured part by part (the lamp's finding M1, where a
    /// shade with eight holes needed 1.04 budgets): the face's budget
    /// grows with its first sums, which are as much work as its boundary
    /// is long. Here the rim of a cap in 24 pieces, with half of what the
    /// whole boundary takes together as the budget.
    #[test]
    fn a_face_past_a_budget_for_its_whole_boundary_is_measured_part_by_part() {
        let level = 0.8_f64;
        let body = cap_in_pieces(level, 24);
        let surface = turned(0.3);
        let face = body.faces.first().unwrap();
        let rule = Rule::new(NODES);
        let coarse = Rule::new(NODES / 2);
        let expected = TAU * RADIUS * (RADIUS - level);
        let whole = |limit: u64| {
            let outline = outline(&body, face, &surface, 0, body_size(&body)).unwrap();
            let mut work = Work::new(&surface);
            work.limit = limit;
            let sums = together(
                face,
                (&surface, &outline),
                0,
                (&rule, &coarse),
                (1.0, false),
                &mut work,
                None,
            );
            (sums.map(|sums| sums.area), work.spent)
        };
        let by_parts = |limit: u64| {
            let mut work = Work::new(&surface);
            work.limit = limit;
            let sums = integrals_in(
                (&body, face, &surface, 0),
                &rule,
                body_size(&body),
                &mut work,
                None,
            );
            (sums.map(|sums| sums.area), work.spent)
        };
        let (area, needed) = whole(BUDGET);
        assert!((area.unwrap() - expected).abs() <= 1e-11 * expected);
        // With half of that, the whole boundary is given up ...
        let (refused, _) = whole(needed / 2);
        match refused {
            Err(MassError::Unsupported { what, .. }) => {
                assert!(what.contains("does not settle"), "{what}")
            }
            other => panic!("{other:?}"),
        }
        // ... and part by part it is measured, as it is with the budget.
        let (area, spent) = by_parts(needed / 2);
        let area = area.unwrap();
        assert!((area - expected).abs() <= 1e-11 * expected);
        assert!(spent > needed / 2, "{spent} of {needed}");
        assert_eq!(by_parts(BUDGET).0, Ok(area));
    }

    /// The first sums of a face refined part by part have a ceiling of
    /// their own ([`FIRST_SUMS_WORK`]): a face whose first sums would pass it
    /// is given up there, as any face whose sums do not settle. Here the
    /// cap's rim in three pieces, with all but a little of the ceiling
    /// taken before.
    #[test]
    fn first_sums_past_their_ceiling_are_refused() {
        let level = 0.8_f64;
        let body = cap_in_pieces(level, 3);
        let surface = turned(0.3);
        let face = body.faces.first().unwrap();
        let rule = Rule::new(NODES);
        let run = |spent: u64| {
            let mut work = Work::new(&surface);
            work.spent = spent;
            let sums = integrals_in(
                (&body, face, &surface, 0),
                &rule,
                body_size(&body),
                &mut work,
                None,
            );
            (sums.map(|sums| sums.area), work.spent)
        };
        let (area, taken) = run(0);
        let expected = TAU * RADIUS * (RADIUS - level);
        assert!((area.unwrap() - expected).abs() <= 1e-11 * expected);
        let (refused, spent) = run(FIRST_SUMS_WORK - 1);
        match refused {
            Err(MassError::Unsupported { what, .. }) => {
                assert!(what.contains("does not settle"), "{what}")
            }
            other => panic!("{other:?}"),
        }
        assert!(
            spent > FIRST_SUMS_WORK && spent <= FIRST_SUMS_WORK + taken,
            "{spent}"
        );
    }

    /// A bilinear patch over the unit square with these corner weights
    /// (`w₀₀, w₀₁, w₁₀, w₁₁`).
    fn weighted_square(weights: [f64; 4]) -> Surface {
        let ctrl = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ];
        let knots = vec![0.0, 0.0, 1.0, 1.0];
        Surface::Nurbs(
            craft3d_geom::NurbsSurface::new(
                1,
                1,
                knots.clone(),
                knots,
                2,
                2,
                ctrl,
                weights.to_vec(),
            )
            .unwrap(),
        )
    }

    /// Hostile factors of the Möbius map: every node stays finite and
    /// inside its stretch, its weight finite and not negative, and a
    /// factor the map would squeeze into rounding is not taken.
    #[test]
    fn the_moebius_map_neither_overflows_nor_vanishes() {
        let rule = Rule::new(NODES);
        for beta in [
            1e-300, 1e-160, 1e-100, 1e-3, 0.5, 2.0, 1e3, 1e100, 1e160, 1e300,
        ] {
            let mut total = 0.0;
            for (s, w) in rule.points(0.0, 1.0) {
                let (t, weight) = moebius_node(s, w, (2.0, 3.0, beta));
                assert!((2.0..=5.0).contains(&t), "{beta}: {t}");
                assert!(weight.is_finite() && weight >= 0.0, "{beta}: {weight}");
                total += weight;
            }
            // The weights in t sum to the stretch's width where the map
            // is gentle enough for sixteen nodes to integrate it.
            if (0.5..=2.0).contains(&beta) {
                assert!((total - 3.0).abs() < 1e-9, "{beta}: {total}");
            }
        }
        let steep = weighted_square([1.0, 1e300, 1.0, 1e300]);
        assert_eq!(moebius(&steep, Along::V, 0.5, (0.0, 1.0)), None);
        let fine = weighted_square([1.0, 100.0, 1.0, 100.0]);
        assert_eq!(moebius(&fine, Along::V, 0.5, (0.0, 1.0)), Some(0.01));
        let even = weighted_square([3.0, 3.0, 3.0, 3.0]);
        assert_eq!(moebius(&even, Along::V, 0.5, (0.0, 1.0)), None);
    }

    /// What no reparametrisation evens out is what counts: weights of a
    /// product form are even, the corners 1, h, h, 1 are h apart, and
    /// weights of any size are compared without overflow.
    #[test]
    fn uneven_weights_are_those_a_reparametrisation_leaves() {
        let product = uneven_weights(&weighted_square([1.0, 1e6, 1e3, 1e9])).unwrap();
        assert!((product - 1.0).abs() < 1e-9, "{product}");
        for h in [10.0, 1e5, 1e12, 1e200] {
            let ratio = uneven_weights(&weighted_square([1.0, h, h, 1.0])).unwrap();
            assert!((ratio / h - 1.0).abs() < 1e-9, "{h}: {ratio}");
        }
        assert_eq!(uneven_weights(&weighted_square([2.0; 4])), None);
    }
}
