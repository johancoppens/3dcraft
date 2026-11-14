//! The nets of a tip: the faces that close a body in one point over the
//! loop of an end face, each running on from the side face behind its rim
//! curve without a crease and all of them reaching the point with one
//! tangent plane.
//!
//! What goes in, per curve of the loop (the *rim*, a simple closed curve
//! in one plane): the curve as a rational B-spline, and the side face's
//! *cross row*, its step across the rim, pointing onward, written in the
//! rim's own basis (one four-vector per control point; for a spline face
//! whose border is the rim, the difference of its last two rows of
//! homogeneous control points: [`TipRim::of_border`]). And the tip point
//! with the normal of its plane, and the fullness.
//!
//! What comes out: one rational B-spline surface per rim curve with the
//! rim as its border `v = 0` (the same control points, weights and knots)
//! and the tip as its border `v = 1`; the rails from every rim vertex to
//! the tip; and what was proven. `docs/design/sweeps.md` › *The nets of a
//! tip* has the method and its proofs.
//!
//! References: Piegl & Tiller, *The NURBS Book*, 2nd ed., ch. 5 (knot
//! insertion, degree elevation) and ch. 7 (conics as rational quadratics);
//! Farin, *Curves and Surfaces for CAGD*, 5th ed., ch. 3 (blossoms),
//! ch. 5–6 (Bernstein forms) and ch. 14 (degenerate patches); Liming,
//! *Practical Analytic Geometry with Applications to Aircraft*, 1944 (a
//! conic from two points, two tangents and a fullness).

mod prove;
mod simple;
#[cfg(test)]
mod tests;

use crate::closure::clamped;
use crate::pole::Border;
use crate::{GeomError, NurbsCurve, NurbsSurface};
use craft3d_math::quadrature::Rule;
use craft3d_math::{Bbox3, Frame, Vec3};
use prove::{Grid, Verdict, Work, certain, last_across, seen_from_tip, shoulder_sine};
use simple::{Piece, closed_curve_is_simple};
use std::fmt;

/// The most rim curves of one loop.
pub const MAX_TIP_RIMS: usize = 1 << 10;
/// The most control points of a loop's rim curves together.
pub const MAX_TIP_POINTS: usize = 1 << 13;
/// The least and the largest fullness: the conic's `ρ = ω / (1 + ω)`.
/// Toward 0 the tip is a straight cone with a crease at the rim, toward 1
/// a flat lid with a crease at the tip; both ends are left out.
pub const FULLNESS_RANGE: (f64, f64) = (1e-3, 0.999);
/// The least sine of the angle at which a side must leave the plane of
/// its rim toward the tip's plane, at every control point: below it the
/// side runs along the end and never reaches the tip's plane.
pub const ONWARD_MARGIN: f64 = 1e-6;
/// The share of the tolerance by which a side's row, carried by one
/// factor, may miss the tip's plane and still be level.
const LEVEL_SHARE: f64 = 0.25;
/// The share of the tolerance by which a side's weights across the rim
/// may move its row and still be in step with the rim's.
const IN_STEP_SHARE: f64 = 0.25;
/// The share of the angular tolerance within which two faces' rows must
/// leave a rail at one ratio for the rail to be called smooth.
const SMOOTH_SHARE: f64 = 0.25;
/// The places along a rail at which the angle between its two faces is
/// measured, the rim's end the first.
const CREASE_PLACES: u32 = 64;
/// The least area a rim encloses, as a share of the square of its size:
/// below it the rim is a line there and back, and runs neither way round.
const LEAST_AREA: f64 = 1e-12;
/// How near the longest length that fits is bracketed, as a share of it.
pub const LENGTH_RESOLUTION: f64 = 1e-9;
/// How often the length asked for is halved in the search for lengths
/// that fit.
const LENGTH_HALVINGS: u32 = 32;
/// The work the proofs of one call may do, in coefficients written (a
/// fraction of a second): a tip that needs more is told as not proven.
const MAX_WORK: u64 = 1 << 29;

/// Why a tip cannot be built. Every reason names what is wrong and where.
#[derive(Clone, Debug, PartialEq)]
pub enum TipError {
    /// Input that describes no tip; the text says which rule was broken.
    Invalid(&'static str),
    /// Valid input that these nets do not carry yet.
    Unsupported(&'static str),
    /// A curve or surface could not be made from the numbers.
    Geometry(GeomError),
    /// The end of rim curve `rim` is `gap` away from the start of the
    /// next (of the first, after the last): the loop is not closed.
    RimOpen { rim: usize, gap: f64 },
    /// The rim is not shown to be a simple closed curve: rim curves `rim`
    /// and `other` (one curve twice: itself) cross, touch or come within
    /// the tolerance of one another away from where they join, or could
    /// not be told apart within the work allowed.
    RimCrosses { rim: usize, other: usize },
    /// At the start of rim curve `rim` the two neighbouring sides do not
    /// go the same way: a step back along each, of the shorter's length,
    /// ends `gap` apart, which is more than the tolerance.
    SidesDisagree { rim: usize, gap: f64 },
    /// Control point `control` of rim curve `rim` is not before the
    /// tip's plane: `room` is how far before it is (negative: beyond).
    RimReachesPlane {
        rim: usize,
        control: usize,
        room: f64,
    },
    /// The side behind rim curve `rim` does not go onward toward the
    /// tip's plane at control point `control`: the sine of its angle with
    /// the plane is `sine`.
    NotOnward {
        rim: usize,
        control: usize,
        sine: f64,
    },
    /// What the sides are carried on to in the tip's plane is not seen
    /// from the tip from inside all round: at parameter `at` of rim curve
    /// `rim` the tip stands outside it, or it has turned inside out.
    NotStarShaped { rim: usize, at: f64 },
    /// The sides close in before the tip: at parameter `at` of rim curve
    /// `rim`, the share `across` of the way to the tip, the face turns
    /// its other side to the tip.
    ClosesIn { rim: usize, at: f64, across: f64 },
    /// Neither proven nor refuted: near parameter `at` of rim curve `rim`
    /// the faces are seen from the tip so nearly edge-on that rounding
    /// could decide it either way, or the work allowed ran out.
    NotProven { rim: usize, at: f64 },
    /// No length up to the one asked fits.
    NoLengthFits,
}

impl fmt::Display for TipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TipError::Invalid(rule) => write!(f, "no tip: {rule}"),
            TipError::Unsupported(what) => write!(f, "not supported yet: {what}"),
            TipError::Geometry(error) => write!(f, "{error}"),
            TipError::RimOpen { rim, gap } => write!(
                f,
                "the rim is not a closed loop: curve {rim} ends {gap:e} away from where the next starts"
            ),
            TipError::RimCrosses { rim, other } => write!(
                f,
                "the rim is not shown to be a curve that does not cross or touch itself: rim curves {rim} and {other} meet, come within the tolerance of one another, or could not be told apart"
            ),
            TipError::SidesDisagree { rim, gap } => write!(
                f,
                "the two sides that meet where rim curve {rim} starts do not go on the same way there: a step back along each ends {gap:e} apart"
            ),
            TipError::RimReachesPlane { rim, control, room } => write!(
                f,
                "the rim reaches the tip's plane: control point {control} of rim curve {rim} is {room:e} before it"
            ),
            TipError::NotOnward { rim, control, sine } => write!(
                f,
                "the side behind rim curve {rim} does not go onward toward the tip at control point {control}: it leaves the end at a sine of {sine:e}"
            ),
            TipError::NotStarShaped { rim, at } => write!(
                f,
                "the tip does not stand inside what the sides are carried on to, or the sides have closed in and turned inside out before it (rim curve {rim} at parameter {at})"
            ),
            TipError::ClosesIn { rim, at, across } => write!(
                f,
                "the sides close in before the tip (rim curve {rim} at parameter {at}, {across} of the way to the tip)"
            ),
            TipError::NotProven { rim, at } => write!(
                f,
                "the tip's faces could not be proven to face the tip one way: near parameter {at} of rim curve {rim} they are seen from it edge-on within rounding, or the proof ran out of the work allowed"
            ),
            TipError::NoLengthFits => write!(f, "no length fits: the sides close in at once"),
        }
    }
}

impl std::error::Error for TipError {}

impl From<GeomError> for TipError {
    fn from(error: GeomError) -> Self {
        TipError::Geometry(error)
    }
}

/// `x <= limit`, and false for a NaN: what is not known to be within a
/// limit is not within it.
fn within(x: f64, limit: f64) -> bool {
    x <= limit
}

/// A homogeneous point or vector: `[x w, y w, z w, w]`.
type H = [f64; 4];

fn h_of(p: Vec3, w: f64) -> H {
    [p.x * w, p.y * w, p.z * w, w]
}

fn h_xyz(h: H) -> Vec3 {
    let [x, y, z, _] = h;
    Vec3::new(x, y, z)
}

fn h_weight(h: H) -> f64 {
    let [_, _, _, w] = h;
    w
}

/// `a fa + b fb`.
fn h_sum(a: H, fa: f64, b: H, fb: f64) -> H {
    let ([a0, a1, a2, a3], [b0, b1, b2, b3]) = (a, b);
    [
        a0 * fa + b0 * fb,
        a1 * fa + b1 * fb,
        a2 * fa + b2 * fb,
        a3 * fa + b3 * fb,
    ]
}

/// `a (1 − t) + b t`, exact at both ends.
fn h_lerp(a: H, b: H, t: f64) -> H {
    h_sum(a, 1.0 - t, b, t)
}

fn h_finite(h: &H) -> bool {
    h.iter().all(|x| x.is_finite())
}

/// One curve of the loop of an end face, with the side face behind it.
#[derive(Clone, Debug, PartialEq)]
pub struct TipRim {
    /// The rim curve, running the way the loop runs: it ends where the
    /// next rim curve starts. Its knot vector is clamped and has no inner
    /// knot more often than its degree.
    pub curve: NurbsCurve,
    /// The step of the side across the rim, onward (out of the side, over
    /// the end), in homogeneous terms and in the rim's basis: `[x, y, z,
    /// w]` per control point of `curve`. For a spline face that ends on
    /// the rim it is the rim's row of homogeneous control points less the
    /// row before it; for any other side, a vector field along the rim in
    /// the side's tangent planes, times the rim's weights, with `w` 0.
    ///
    /// **The tip is made along these directions**: two rows that give
    /// the same tangent planes and other directions in them give other
    /// tips. What does not matter is how the step is weighted: its `w`
    /// must be one multiple of the rim's weights along the whole curve
    /// (the side's weights across the rim in step with the rim's, as a
    /// surface of revolution's are), and is then taken out. Nor its
    /// length, along a side whose row reaches the tip's plane as one.
    /// Its length is also the lever by which two neighbours are held to
    /// agree where they meet: a step back along each must end within the
    /// tolerance of the other's.
    pub cross: Vec<[f64; 4]>,
}

impl TipRim {
    /// The rim and the cross row of a spline face that ends on one of its
    /// borders: the border's row of control points as the rim curve
    /// (running the way the face's parameter along it runs), and that row
    /// less the row before it. Both knot vectors must be clamped.
    pub fn of_border(surface: &NurbsSurface, border: Border) -> Result<TipRim, TipError> {
        let (count_u, count_v) = surface.counts();
        let (degree_u, degree_v) = surface.degrees();
        let along_v = matches!(border, Border::UStart | Border::UEnd);
        let at_end = matches!(border, Border::UEnd | Border::VEnd);
        let (degree, knots, count, across_degree, across_knots, across_count) = if along_v {
            (
                degree_v,
                surface.knots_v(),
                count_v,
                degree_u,
                surface.knots_u(),
                count_u,
            )
        } else {
            (
                degree_u,
                surface.knots_u(),
                count_u,
                degree_v,
                surface.knots_v(),
                count_v,
            )
        };
        if !clamped(knots, degree) || !clamped(across_knots, across_degree) || across_count < 2 {
            return Err(TipError::Unsupported(
                "a side face whose knot vectors are not clamped: its border is not its row of control points",
            ));
        }
        let (row, before) = if at_end {
            (across_count - 1, across_count - 2)
        } else {
            (0, 1)
        };
        let index = |along: usize, across: usize| {
            if along_v {
                across * count_v + along
            } else {
                along * count_v + across
            }
        };
        let (points, weights) = (surface.control_points(), surface.weights());
        let at = |along: usize, across: usize| -> Option<(Vec3, f64)> {
            let i = index(along, across);
            Some((*points.get(i)?, *weights.get(i)?))
        };
        let mut rim = Vec::with_capacity(count);
        let mut rim_weights = Vec::with_capacity(count);
        let mut cross = Vec::with_capacity(count);
        for along in 0..count {
            let (Some((c, wc)), Some((b, wb))) = (at(along, row), at(along, before)) else {
                return Err(TipError::Invalid("a side face with a short control net"));
            };
            rim.push(c);
            rim_weights.push(wc);
            // (About the rim's own point, so that a face far from the
            // origin loses nothing: w_b (C − B) and w_c − w_b, which is
            // the difference of the homogeneous points moved by C.)
            let step = (c - b) * wb;
            cross.push(h_sum(
                h_of(c, wc - wb),
                1.0,
                [step.x, step.y, step.z, 0.0],
                1.0,
            ));
        }
        let curve = NurbsCurve::new(degree, knots.to_vec(), rim, rim_weights)?;
        Ok(TipRim { curve, cross })
    }

    /// The same rim and side, the rim curve running the other way.
    pub fn reversed(&self) -> Result<TipRim, TipError> {
        let knots = self.curve.knots();
        let (Some(&first), Some(&last)) = (knots.first(), knots.last()) else {
            return Err(TipError::Invalid("a rim curve without knots"));
        };
        let knots = knots.iter().rev().map(|k| first + last - k).collect();
        let points = self.curve.control_points().iter().rev().copied().collect();
        let weights = self.curve.weights().iter().rev().copied().collect();
        Ok(TipRim {
            curve: NurbsCurve::new(self.curve.degree(), knots, points, weights)?,
            cross: self.cross.iter().rev().copied().collect(),
        })
    }
}

/// Where the tip is and how full.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TipSpec {
    /// The tip point. The tip's plane is the plane through it square to
    /// `normal`; every rim curve lies before it.
    pub tip: Vec3,
    /// The outward direction: from the end toward the tip's plane. Any
    /// length.
    pub normal: Vec3,
    /// The conic's `ρ = ω / (1 + ω)`, within [`FULLNESS_RANGE`]: `√2 − 1`
    /// gives quarter ellipses (`ω = √½`: the half ball on a rod as long
    /// as its radius), `0.5` parabolas, more is blunter.
    pub fullness: f64,
    /// The length below which two points are one. The rim must lie in
    /// one plane within it, its curves join within it and stay further
    /// than it from one another elsewhere, and stay further than it
    /// before the tip's plane; two neighbouring sides agree where a step
    /// back along each ends within it; a side is level where its row,
    /// carried by one factor, misses the tip's plane by less than a
    /// quarter of it.
    pub tolerance: f64,
    /// The angle below which two directions are one: a rail is called
    /// smooth where its two faces' rows leave it at one ratio within a
    /// quarter of it.
    pub angular: f64,
}

/// Which of the two nets a tip has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipForm {
    /// Every side's cross row reaches the tip's plane as one row: three
    /// rows, a conic across.
    Level,
    /// It does not: four rows, degree 3 across.
    General,
}

/// The angle between the two faces of a rail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TipCrease {
    /// At the rim's end of the rail: the angle between the two sides'
    /// tangent planes there, which the faces take over.
    pub at_rim: f64,
    /// The largest at 64 places along the rail, its rim end the first: a
    /// measurement, not a bound. The angle falls to nothing at the tip.
    pub largest: f64,
}

/// The faces of a tip, and what was proven of them.
///
/// **Proven of every tip returned**, on convex hulls: the rim is a
/// simple closed curve in one plane (within the tolerance); every side
/// leaves the end toward the tip's plane; and every ray from the tip
/// crosses the faces from the side the rim shows, by more than rounding
/// could undo. From these: the faces are regular, cross neither
/// themselves nor one another, and have one tangent plane at the tip.
/// Each face is tangent to its side along the rim by construction.
/// Whether two faces are tangent along their rail is told per rail
/// (`smooth`, `creases`).
#[derive(Clone, Debug, PartialEq)]
pub struct Tip {
    pub form: TipForm,
    /// The rim curves as the faces have them: the curves given, or the
    /// two halves of a single closed one.
    pub rims: Vec<NurbsCurve>,
    /// One face per rim curve: `u` along the rim (its degree and knots),
    /// `v` from the rim (`v = 0`, the rim curve's own control points and
    /// weights) to the tip (`v = 1`, a pole).
    pub faces: Vec<NurbsSurface>,
    /// One rail per rim curve: from its start to the tip. It is the
    /// border `u = start` of its own face, with that column's control
    /// points and its weights as shares of the first (1, ω, 1, or 1,
    /// (1 + 2ω)/3 twice, 1); the face before ends on the same control
    /// points (bit for bit beyond the rim, where two rim curves may meet
    /// within the tolerance).
    pub rails: Vec<NurbsCurve>,
    pub tip: Vec3,
    /// Whether the rim runs counter-clockwise seen against the normal
    /// (from beyond the tip): the faces' `∂u × ∂v` then points out of the
    /// body, and into it otherwise.
    pub counter_clockwise: bool,
    /// The least sine of the angle at which the sides leave the end
    /// toward the tip's plane, over the rim's control points and so all
    /// along it: at least [`ONWARD_MARGIN`].
    pub onward: f64,
    /// A lower bound on the sine of the angle at which the rays from the
    /// tip cross the shoulder (the curve of the row before the tip, in
    /// the tip's plane), all round: a number of the shoulder's shape and
    /// the tip's place alone, not of the fullness, the weights or the
    /// size. The faces' normals near the tip differ from the plane's in
    /// proportion to the distance from the tip over this sine. 0 where
    /// no bound was found (the proof that it is positive does not rest
    /// on it); the bound can be several times below the least sine.
    pub star: f64,
    /// Per rail: whether its two faces are proven tangent all along it:
    /// their rows leave it at one ratio of speeds, row for row, within a
    /// quarter of the angular tolerance. They do where the rim is smooth
    /// at the vertex and the sides' own rows leave their common edge at
    /// one ratio (as a loft's are made to).
    pub smooth: Vec<bool>,
    /// Per rail: the angle between its two faces, at the rim and at its
    /// largest. A rail that is not smooth is an edge whose angle falls to
    /// nothing at the tip: where the rim has a corner (`at_rim` is the
    /// corner's angle between the sides), and where the sides are not
    /// tangent across their common edge behind the rim though the rim is
    /// smooth (`at_rim` nothing, `largest` not).
    pub creases: Vec<TipCrease>,
}

/// One rim curve taken apart, with its cross row.
#[derive(Clone, Debug)]
struct Side {
    degree: usize,
    knots: Vec<f64>,
    points: Vec<Vec3>,
    weights: Vec<f64>,
    cross: Vec<H>,
}

impl Side {
    fn homogeneous(&self) -> impl Iterator<Item = H> + '_ {
        self.points
            .iter()
            .zip(&self.weights)
            .map(|(&p, &w)| h_of(p, w))
    }
}

/// Inserts the knot `u` once into `knots` and into every row (Boehm's
/// algorithm, *The NURBS Book* A5.1, on homogeneous points). `u` lies
/// inside the domain with a multiplicity below the degree.
fn insert_knot(knots: &mut Vec<f64>, degree: usize, rows: &mut [Vec<H>], u: f64) {
    let k = knots.partition_point(|&x| x <= u).saturating_sub(1);
    let alpha = |i: usize| -> f64 {
        if i + degree <= k {
            1.0
        } else if i > k {
            0.0
        } else {
            let (Some(&lo), Some(&hi)) = (knots.get(i), knots.get(i + degree)) else {
                return 0.0;
            };
            if hi > lo { (u - lo) / (hi - lo) } else { 0.0 }
        }
    };
    for row in rows.iter_mut() {
        let old = std::mem::take(row);
        let count = old.len() + 1;
        let mut new = Vec::with_capacity(count);
        for i in 0..count {
            let here = old.get(i).copied();
            let before = i.checked_sub(1).and_then(|i| old.get(i)).copied();
            new.push(match (here, before) {
                (Some(h), Some(b)) => h_lerp(b, h, alpha(i)),
                (Some(h), None) => h,
                (None, Some(b)) => b,
                (None, None) => [0.0; 4],
            });
        }
        *row = new;
    }
    knots.insert((k + 1).min(knots.len()), u);
}

/// A single closed rim curve cut in two at the middle of its parameter
/// range, as a revolution's full turn is: a face that closed on itself
/// would meet itself along a seam.
fn cut_in_two(side: &Side) -> Result<[Side; 2], TipError> {
    let invalid = TipError::Invalid("a rim curve too short to cut in two");
    let (Some(&start), Some(&end)) = (side.knots.first(), side.knots.last()) else {
        return Err(invalid);
    };
    let middle = 0.5 * (start + end);
    if !(start < middle && middle < end) {
        return Err(invalid);
    }
    let mut knots = side.knots.clone();
    let mut rows = vec![side.homogeneous().collect::<Vec<H>>(), side.cross.clone()];
    for _ in 0..=side.degree {
        if knots.iter().filter(|&&x| x == middle).count() >= side.degree {
            break;
        }
        insert_knot(&mut knots, side.degree, &mut rows, middle);
    }
    // The curve passes through the control point before the first of the
    // middle knots: the last of the first half, the first of the second.
    let first = knots.partition_point(|&x| x < middle);
    let [rim, cross] = rows.as_slice() else {
        return Err(invalid);
    };
    let half = |points: std::ops::Range<usize>, knots: Vec<f64>| -> Result<Side, TipError> {
        let (Some(rim), Some(cross)) = (rim.get(points.clone()), cross.get(points)) else {
            return Err(TipError::Invalid("a rim curve too short to cut in two"));
        };
        let mut side = Side {
            degree: side.degree,
            knots,
            points: Vec::with_capacity(rim.len()),
            weights: Vec::with_capacity(rim.len()),
            cross: cross.to_vec(),
        };
        for &h in rim {
            let w = h_weight(h);
            if !(w.is_finite() && w > 0.0) {
                return Err(TipError::Invalid("a rim curve's weight is not positive"));
            }
            side.points.push(h_xyz(h) * (1.0 / w));
            side.weights.push(w);
        }
        Ok(side)
    };
    let count = rim.len();
    if first == 0 || first > count {
        return Err(invalid);
    }
    let mut low: Vec<f64> = knots.iter().take(first + side.degree).copied().collect();
    low.push(middle);
    let mut high = vec![middle];
    high.extend(knots.iter().skip(first).copied());
    Ok([half(0..first, low)?, half(first - 1..count, high)?])
}

/// The rim curves checked and taken apart: at least two sides, joined
/// end to end into a loop that lies in one plane and does not cross
/// itself.
fn prepare(rims: &[TipRim], tolerance: f64) -> Result<Vec<Side>, TipError> {
    if !(tolerance.is_finite() && tolerance > 0.0) {
        return Err(TipError::Invalid(
            "the tolerance must be positive and finite",
        ));
    }
    if rims.is_empty() {
        return Err(TipError::Invalid("a tip needs a rim"));
    }
    if rims.len() > MAX_TIP_RIMS {
        return Err(TipError::Invalid(
            "more rim curves than a tip is built over",
        ));
    }
    let mut total = 0_usize;
    let mut sides = Vec::with_capacity(rims.len().max(2));
    for rim in rims {
        let curve = &rim.curve;
        let count = curve.control_points().len();
        total = total.saturating_add(count);
        if total > MAX_TIP_POINTS {
            return Err(TipError::Invalid(
                "more control points round the rim than a tip is built over",
            ));
        }
        if rim.cross.len() != count {
            return Err(TipError::Invalid(
                "a cross row needs one vector per control point of its rim curve",
            ));
        }
        if !rim.cross.iter().all(h_finite) {
            return Err(TipError::Invalid(
                "a cross row is not finite (NaN or infinity)",
            ));
        }
        if !clamped(curve.knots(), curve.degree()) {
            return Err(TipError::Unsupported(
                "a rim curve whose knot vector is not clamped",
            ));
        }
        // An inner knot more often than the degree tears the curve.
        let (mut run, mut previous) = (0_usize, f64::NAN);
        let (first, last) = curve.domain();
        for &knot in curve.knots() {
            run = if knot == previous { run + 1 } else { 1 };
            previous = knot;
            if run > curve.degree() && knot > first && knot < last {
                return Err(TipError::Invalid(
                    "a rim curve with an inner knot more often than its degree: the curve may jump there",
                ));
            }
        }
        sides.push(Side {
            degree: curve.degree(),
            knots: curve.knots().to_vec(),
            points: curve.control_points().to_vec(),
            weights: curve.weights().to_vec(),
            cross: rim.cross.clone(),
        });
    }
    if let [only] = sides.as_slice() {
        let halves = cut_in_two(only)?;
        sides = halves.to_vec();
    }
    for (k, side) in sides.iter().enumerate() {
        let next = sides.get((k + 1) % sides.len());
        let (Some(end), Some(start)) = (side.points.last(), next.and_then(|n| n.points.first()))
        else {
            return Err(TipError::Invalid("a rim curve without control points"));
        };
        let gap = end.distance(*start);
        // (A NaN is not within the tolerance.)
        if !within(gap, tolerance) {
            return Err(TipError::RimOpen { rim: k, gap });
        }
    }
    in_one_plane_and_simple(&sides, tolerance)?;
    Ok(sides)
}

/// The unit normal and the conic's weight of a specification.
fn checked(normal: Vec3, fullness: f64, tip: Vec3) -> Result<(Vec3, f64), TipError> {
    if !tip.is_finite() {
        return Err(TipError::Invalid("the tip point is not finite"));
    }
    let normal = normal
        .is_finite()
        .then(|| normal.normalized())
        .flatten()
        .ok_or(TipError::Invalid(
            "the tip's normal has no direction (zero, NaN or infinity)",
        ))?;
    if !(fullness >= FULLNESS_RANGE.0 && fullness <= FULLNESS_RANGE.1) {
        return Err(TipError::Invalid(
            "the fullness must lie between 0.001 and 0.999",
        ));
    }
    Ok((normal, fullness / (1.0 - fullness)))
}

/// The Bézier control points of the span `[knot k, knot k + 1]` of one
/// row: point `m` is the row's blossom at `degree − m` times the span's
/// start and `m` times its end (Farin ch. 3; Ramshaw 1987), by de Boor's
/// steps with one argument per level.
fn bezier_of_span(degree: usize, knots: &[f64], k: usize, row: &[H]) -> Option<Vec<H>> {
    let (&lo, &hi) = (knots.get(k)?, knots.get(k + 1)?);
    let first = k.checked_sub(degree)?;
    let local = row.get(first..=k)?;
    let mut out = Vec::with_capacity(degree + 1);
    for m in 0..=degree {
        let mut d = local.to_vec();
        for level in 1..=degree {
            // The argument of this level: the end's first, then the
            // start's, so the copies of a point stay exact.
            let argument = if level <= m { hi } else { lo };
            for i in (level..=degree).rev() {
                let (a, b) = (
                    *knots.get(first + i)?,
                    *knots.get(first + i + degree + 1 - level)?,
                );
                let t = if b > a { (argument - a) / (b - a) } else { 0.0 };
                let (before, here) = (*d.get(i - 1)?, *d.get(i)?);
                *d.get_mut(i)? = h_lerp(before, here, t);
            }
        }
        out.push(*d.get(degree)?);
    }
    Some(out)
}

/// The knot spans of a side that have a length: the index of each span's
/// first knot, and its two knots.
fn spans(side: &Side) -> impl Iterator<Item = (usize, f64, f64)> + '_ {
    (side.degree..side.points.len()).filter_map(|k| {
        let (&lo, &hi) = (side.knots.get(k)?, side.knots.get(k + 1)?);
        (hi > lo).then_some((k, lo, hi))
    })
}

/// The rim shown to lie in one plane and to be a simple closed curve
/// there: what the proof that the faces do not cross rests on.
///
/// The plane is the one through the middle of the control points square
/// to the area vector of their polygon; every control point lies within
/// the tolerance of it (and so does the rim: positive weights). In that
/// plane the rim's Bézier pieces are shown not to cross or touch, and to
/// stay further than the tolerance from one another where they are not
/// one run (`tip/simple.rs`).
fn in_one_plane_and_simple(sides: &[Side], tolerance: f64) -> Result<(), TipError> {
    let points: Vec<Vec3> = sides
        .iter()
        .flat_map(|side| side.points.iter().copied())
        .collect();
    let mut centre = Vec3::ZERO;
    for p in &points {
        centre += *p * (1.0 / points.len() as f64);
    }
    let mut area = Vec3::ZERO;
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)) {
        area += (*a - centre).cross(*b - centre);
    }
    let size = Bbox3::from_points(points.iter().copied()).diagonal();
    let frame = if within(area.length(), LEAST_AREA * size * size) || !area.is_finite() {
        None
    } else {
        Frame::from_z(centre, area)
    };
    let Some(frame) = frame else {
        return Err(TipError::Invalid("the rim encloses no area"));
    };
    let off = points
        .iter()
        .fold(0.0_f64, |m, p| m.max(frame.z().dot(*p - centre).abs()));
    if !within(off, tolerance) {
        return Err(TipError::Unsupported(
            "a rim that does not lie in one plane: the faces of a tip are proven not to cross only over a rim in a plane",
        ));
    }
    let mut pieces = Vec::new();
    for (rim, side) in sides.iter().enumerate() {
        // (About the middle, so that a rim far from the origin loses
        // nothing.)
        let row: Vec<H> = side
            .points
            .iter()
            .zip(&side.weights)
            .map(|(&p, &w)| h_of(p - centre, w))
            .collect();
        for (k, _, _) in spans(side) {
            let piece = bezier_of_span(side.degree, &side.knots, k, &row).and_then(|span| {
                let flat = span
                    .iter()
                    .map(|&h| {
                        let at = h_xyz(h);
                        [at.dot(frame.x()), at.dot(frame.y()), h_weight(h)]
                    })
                    .collect();
                Piece::new(rim, flat)
            });
            let Some(piece) = piece else {
                return Err(TipError::Invalid(
                    "a rim curve that cannot be cut into pieces",
                ));
            };
            pieces.push(piece);
        }
    }
    closed_curve_is_simple(&pieces, tolerance)
        .map_err(|(rim, other)| TipError::RimCrosses { rim, other })
}

/// What the sides are carried on to in the tip's plane: the shoulder.
struct Shoulder {
    form: TipForm,
    /// Per side and control point: the rim's point carried on along the
    /// side by the side's own factor (level), or by the least factor of
    /// the whole loop (general).
    carried: Vec<Vec<Vec3>>,
    /// Per side and control point: the shoulder's point in the tip's
    /// plane: `carried` itself where the sides are level, `carried`
    /// dropped onto the plane along the normal otherwise.
    rows: Vec<Vec<Vec3>>,
    /// The least sine at which a side leaves the end.
    onward: f64,
}

/// The step of every side across the rim at every control point, as a
/// vector: the cross row without its weights.
///
/// A cross vector `[d, δ]` at the rim's point `C` of weight `w` stands
/// for the derivative `(d − δ C) / w` of the side's points; `δ` only
/// says how the side's weight changes with it. Where `δ / w` is one
/// number along the whole curve, the weights across the rim are the
/// rim's own times a function of the parameter across, and the side is
/// the same surface with any other such function: the tip must not
/// depend on it, and takes the vectors alone. Where `δ / w` varies, the
/// directions `(d − δ C) / w` are those of one way of writing the side
/// among others with other directions, and the side is refused: its
/// shape does not say which way the tip is to go.
fn steps(sides: &[Side], tolerance: f64) -> Result<Vec<Vec<Vec3>>, TipError> {
    let mut all = Vec::with_capacity(sides.len());
    for side in sides {
        let (mut least, mut most) = (f64::INFINITY, f64::NEG_INFINITY);
        for (d, &w) in side.cross.iter().zip(&side.weights) {
            let ratio = h_weight(*d) / w;
            least = least.min(ratio);
            most = most.max(ratio);
        }
        let share = 0.5 * (least + most);
        if !share.is_finite() {
            return Err(TipError::Invalid(
                "a cross row's weight is out of range against its rim's",
            ));
        }
        let mut row = Vec::with_capacity(side.cross.len());
        for ((d, &c), &w) in side.cross.iter().zip(&side.points).zip(&side.weights) {
            let step = (h_xyz(*d) - c * h_weight(*d)) * (1.0 / w);
            if !step.is_finite() {
                return Err(TipError::Invalid("a cross row is out of range"));
            }
            // How far the row before the rim would move if its weight
            // were the common one.
            let moved = (h_weight(*d) / w - share).abs() * step.length();
            if !within(moved, IN_STEP_SHARE * tolerance) {
                return Err(TipError::Unsupported(
                    "a side whose weights across the rim are out of step with the rim's own: written so, its shape does not say which way across the rim the tip is to go",
                ));
            }
            row.push(step);
        }
        all.push(row);
    }
    Ok(all)
}

/// Carries every side on to the tip's plane (the plane through `on`
/// square to the unit `normal`), after the checks that it can be.
///
/// A control point `C` with the step `s` reaches the plane at `C + τ s`,
/// `τ` its *carry factor*: its room before the plane over the step's
/// climb. A side is level when its factors are one number: `C + τ s` is
/// then one row of the side's own kind, which is what keeps the tip
/// tangent to the side along the rim and puts the row in the plane.
/// Where the factors differ the row is carried by the least of them
/// (nothing passes the plane) and dropped onto the plane along the
/// normal: one map for every control point, so what holds between two
/// neighbours' rows across their common edge holds between the tip's.
///
/// Where two sides meet, their steps must agree within the tolerance: a
/// step back along each, of the shorter's length, ends within it of the
/// other. The tip is then built on one row: the earlier side takes the
/// later side's step there (its direction with the earlier side's own
/// climb where the sides are level, which are free in their steps'
/// lengths; the step itself otherwise, where the lengths must agree
/// too).
fn shoulder(
    sides: &[Side],
    on: Vec3,
    normal: Vec3,
    tolerance: f64,
    form: Option<TipForm>,
) -> Result<Shoulder, TipError> {
    let level_of_plane = normal.dot(on);
    let mut steps = steps(sides, tolerance)?;
    let mut rooms: Vec<Vec<f64>> = Vec::with_capacity(sides.len());
    let mut onward = f64::INFINITY;
    for (rim, (side, row)) in sides.iter().zip(&steps).enumerate() {
        let mut room_row = Vec::with_capacity(row.len());
        for (control, (p, step)) in side.points.iter().zip(row).enumerate() {
            let room = level_of_plane - normal.dot(*p);
            if within(room, tolerance) || room.is_nan() {
                return Err(TipError::RimReachesPlane { rim, control, room });
            }
            // The side's derivative across is Σ N w s / Σ N w along the
            // rim: its climb at least the least of the control points',
            // as a share of its length (the triangle inequality).
            let sine = normal.dot(*step) / step.length();
            if !(sine >= ONWARD_MARGIN && sine.is_finite()) {
                return Err(TipError::NotOnward {
                    rim,
                    control,
                    sine: if sine.is_finite() { sine } else { 0.0 },
                });
            }
            onward = onward.min(sine);
            room_row.push(room);
        }
        rooms.push(room_row);
    }
    let factor = |room: f64, step: Vec3| room / normal.dot(step);
    // Level: carried by the middle of its factors, every control point of
    // a side lands within a quarter of the tolerance of the plane.
    let mut middles = Vec::with_capacity(sides.len());
    let mut all_level = true;
    for (row, room_row) in steps.iter().zip(&rooms) {
        let (mut lo, mut hi) = (f64::INFINITY, 0.0_f64);
        for (step, &room) in row.iter().zip(room_row) {
            let tau = factor(room, *step);
            lo = lo.min(tau);
            hi = hi.max(tau);
        }
        let middle = 0.5 * (lo + hi);
        let miss = row.iter().zip(room_row).fold(0.0_f64, |m, (step, &room)| {
            m.max((middle * normal.dot(*step) - room).abs())
        });
        all_level &= within(miss, LEVEL_SHARE * tolerance);
        middles.push(middle);
    }
    let form = form.unwrap_or(if all_level {
        TipForm::Level
    } else {
        TipForm::General
    });
    // One row where two sides meet.
    let count = sides.len();
    for rim in 0..count {
        let before = (rim + count - 1) % count;
        let (Some(&later), Some(&earlier)) = (
            steps.get(rim).and_then(|row| row.first()),
            steps.get(before).and_then(|row| row.last()),
        ) else {
            continue;
        };
        let (long, short) = (earlier.length(), later.length());
        let lever = long.min(short);
        let gap = (earlier * (1.0 / long) - later * (1.0 / short)).length() * lever;
        if !within(gap, tolerance) {
            return Err(TipError::SidesDisagree { rim, gap });
        }
        let agreed = match form {
            TipForm::Level => later * (normal.dot(earlier) / normal.dot(later)),
            TipForm::General => {
                if !within((earlier - later).length(), tolerance) {
                    return Err(TipError::Unsupported(
                        "sides that are not level and whose steps across the rim differ in length where they meet: the row beside the rim is carried by one factor for the whole loop",
                    ));
                }
                later
            }
        };
        if let Some(last) = steps.get_mut(before).and_then(|row| row.last_mut()) {
            *last = agreed;
        }
    }
    let mut least = f64::INFINITY;
    for (row, room_row) in steps.iter().zip(&rooms) {
        for (step, &room) in row.iter().zip(room_row) {
            least = least.min(factor(room, *step));
        }
    }
    let mut carried = Vec::with_capacity(count);
    let mut rows = Vec::with_capacity(count);
    for ((side, row), &middle) in sides.iter().zip(&steps).zip(&middles) {
        let tau = match form {
            TipForm::Level => middle,
            TipForm::General => least,
        };
        let mut on_the_way = Vec::with_capacity(row.len());
        let mut in_the_plane = Vec::with_capacity(row.len());
        for (p, step) in side.points.iter().zip(row) {
            let point = *p + *step * tau;
            if !point.is_finite() {
                return Err(TipError::Invalid("a cross row is out of range"));
            }
            on_the_way.push(point);
            in_the_plane.push(match form {
                TipForm::Level => point,
                TipForm::General => point + normal * (level_of_plane - normal.dot(point)),
            });
        }
        carried.push(on_the_way);
        rows.push(in_the_plane);
    }
    Ok(Shoulder {
        form,
        carried,
        rows,
        onward,
    })
}

/// The control net of one face: `rows[j][i]`, row `j` from the rim
/// (`j = 0`) to the tip, each control point with its weight.
type Net = Vec<Vec<(Vec3, f64)>>;

/// The nets of all faces from the shoulder (`C` a rim point of weight
/// `w`, `M` its shoulder point, `P` the tip, `ω` the conic's weight).
///
/// Level, a conic across: `C`, `M`, `P` with the weights `w`, `ω w`,
/// `w`.
/// General, the same conic raised to degree 3 with its second row short
/// of the shoulder where the side is slower (`S̄` the point carried by
/// the least factor, `M` that point dropped onto the plane, `λ = 2ω /
/// (1 + 2ω)`): `C`, `C + λ (S̄ − C)`, `P + λ (M − P)`, `P` with the
/// weights `w`, `w (1 + 2ω) / 3` twice, `w`.
fn nets(sides: &[Side], shoulder: &Shoulder, tip: Vec3, omega: f64) -> Vec<Net> {
    let raised = (1.0 + 2.0 * omega) / 3.0;
    let toward = 2.0 * omega / (1.0 + 2.0 * omega);
    let mut nets: Vec<Net> = Vec::with_capacity(sides.len());
    for ((side, carried), row) in sides.iter().zip(&shoulder.carried).zip(&shoulder.rows) {
        let count = side.points.len();
        let rows = match shoulder.form {
            TipForm::Level => 3,
            TipForm::General => 4,
        };
        let mut net: Net = (0..rows).map(|_| Vec::with_capacity(count)).collect();
        for (((&c, &w), &on_the_way), &point) in
            side.points.iter().zip(&side.weights).zip(carried).zip(row)
        {
            let column: Vec<(Vec3, f64)> = match shoulder.form {
                TipForm::Level => vec![(c, w), (point, omega * w), (tip, w)],
                TipForm::General => vec![
                    (c, w),
                    (c + (on_the_way - c) * toward, raised * w),
                    (tip + (point - tip) * toward, raised * w),
                    (tip, w),
                ],
            };
            for (row, entry) in net.iter_mut().zip(column) {
                row.push(entry);
            }
        }
        nets.push(net);
    }
    nets
}

/// Where two faces meet on a rail, their columns are one curve: the
/// earlier face's column takes the later face's points between the rim
/// and the tip, which the agreed steps have put within rounding of its
/// own.
fn join_rails(nets: &mut [Net], tolerance: f64) -> Result<(), TipError> {
    let count = nets.len();
    let starts: Vec<Vec<Vec3>> = nets
        .iter()
        .map(|net| {
            net.iter()
                .filter_map(|row| row.first())
                .map(|&(p, _)| p)
                .collect()
        })
        .collect();
    for (rim, start) in starts.iter().enumerate() {
        let Some(before) = nets.get_mut((rim + count - 1) % count) else {
            continue;
        };
        let rows = before.len();
        for (j, (row, &point)) in before.iter_mut().zip(start).enumerate() {
            if j == 0 || j + 1 == rows {
                continue;
            }
            let Some(last) = row.last_mut() else {
                continue;
            };
            let gap = last.0.distance(point);
            if !within(gap, tolerance) {
                return Err(TipError::SidesDisagree { rim, gap });
            }
            last.0 = point;
        }
    }
    Ok(())
}

/// Whether the rim runs counter-clockwise seen against the normal: the
/// sign of the area it encloses (Green's theorem about its first point,
/// by Gauss–Legendre on every knot span). A rim that encloses nothing
/// seen along the normal runs neither way.
fn runs_counter_clockwise(sides: &[Side], normal: Vec3) -> Result<bool, TipError> {
    let rule = Rule::new(8);
    let Some(&about) = sides.first().and_then(|side| side.points.first()) else {
        return Err(TipError::Invalid("a rim curve without control points"));
    };
    let mut area = 0.0_f64;
    let mut bbox = Bbox3::EMPTY;
    for side in sides {
        let curve = NurbsCurve::new(
            side.degree,
            side.knots.clone(),
            side.points.clone(),
            side.weights.clone(),
        )?;
        for &p in &side.points {
            bbox.add(p);
        }
        for (_, lo, hi) in spans(side) {
            for (t, weight) in rule.points(lo, hi) {
                let (point, along) = curve.derivatives(t);
                area += 0.5 * weight * normal.dot((point - about).cross(along));
            }
        }
    }
    let size = bbox.diagonal();
    if within(area.abs(), LEAST_AREA * size * size) || area.is_nan() {
        return Err(TipError::Invalid(
            "the rim encloses no area seen along the tip's normal",
        ));
    }
    Ok(area > 0.0)
}

/// Proves that every face is seen from the tip from the side its rim
/// shows it, everywhere, by more than rounding could undo; and gives a
/// lower bound on the sine at which the rays from the tip cross the
/// shoulder.
///
/// What it means. A ray from the tip crosses the faces transversally and
/// always the same way round, so the faces are a graph over the
/// directions seen from the tip: regular (no fold, no second point where
/// a face shrinks to nothing). Along `v = 1` it is the condition for one
/// tangent plane at the tip: the shoulder is star-shaped about the tip
/// and runs the way the rim runs. Along `v = 0` it says the rim is seen
/// from the tip on the outside of what it is carried on to: the sides
/// have not closed in and crossed over before the tip's plane. And with
/// the rim a simple closed curve in a plane ([`in_one_plane_and_simple`])
/// every ray meets the faces once: before the tip's plane the directions
/// lie in the open half below the tip and reach its edge only along the
/// shoulder; the rim's own directions are a simple closed curve there (a
/// plane seen from a point outside it) with the faces on its outer side
/// by the determinant's sign; closed by the disc inside that curve, the
/// map from the faces to the half is a covering of a disc by a disc, so
/// one to one.
fn prove(
    sides: &[Side],
    nets: &[Net],
    tip: Vec3,
    normal: Vec3,
    counter_clockwise: bool,
    work: &mut Work,
) -> Result<f64, TipError> {
    /// One Bézier span of a face: the determinant and its terms' size.
    struct Span {
        rim: usize,
        range: (f64, f64),
        seen: Grid<f64>,
        size: Grid<f64>,
    }
    let mut found = Vec::new();
    let mut star = f64::INFINITY;
    let seen_from = if counter_clockwise { normal } else { -normal };
    let tip_reach = tip.length();
    for (rim, (side, net)) in sides.iter().zip(nets).enumerate() {
        let rows_about = net.iter().take(net.len().saturating_sub(1));
        let about: Vec<Vec<H>> = rows_about
            .clone()
            .map(|row| row.iter().map(|&(p, w)| h_of(p - tip, w)).collect())
            .collect();
        // How far each control point and the tip lie from the origin,
        // times the weight: what the last place of their difference is
        // worth.
        let reach: Vec<Vec<H>> = rows_about
            .map(|row| {
                row.iter()
                    .map(|&(p, w)| [(p.length() + tip_reach) * w, 0.0, 0.0, 0.0])
                    .collect()
            })
            .collect();
        for (k, lo, hi) in spans(side) {
            let unproven = TipError::NotProven {
                rim,
                at: 0.5 * (lo + hi),
            };
            // (What the spans are cut out with and multiplied counts
            // against the work allowed too.)
            let points = side.degree + 1;
            if !work.spend(8 * about.len() * points * points * points) {
                return Err(unproven);
            }
            let mut rows = Vec::with_capacity(about.len());
            let mut far = Vec::with_capacity(about.len());
            let mut shoulder_weights = Vec::new();
            for (row, reach) in about.iter().zip(&reach) {
                let (Some(span), Some(reach)) = (
                    bezier_of_span(side.degree, &side.knots, k, row),
                    bezier_of_span(side.degree, &side.knots, k, reach),
                ) else {
                    return Err(unproven);
                };
                shoulder_weights = span.iter().map(|&h| h_weight(h)).collect();
                rows.push(span.into_iter().map(h_xyz).collect::<Vec<Vec3>>());
                far.push(reach.into_iter().map(|h| h_xyz(h).x).collect::<Vec<f64>>());
            }
            if let Some(shoulder) = rows.last() {
                star = star.min(shoulder_sine(shoulder, &shoulder_weights, seen_from));
            }
            let Some((mut seen, size)) = seen_from_tip(&rows, &far) else {
                return Err(unproven);
            };
            // A rim that runs counter-clockwise against the normal is
            // seen from the tip with a positive determinant, the other
            // with a negative.
            if !counter_clockwise {
                for x in &mut seen.c {
                    *x = -*x;
                }
            }
            found.push(Span {
                rim,
                range: (lo, hi),
                seen,
                size,
            });
        }
    }
    if found.is_empty() {
        return Err(TipError::Invalid(
            "a rim without a span to build a face over",
        ));
    }
    // First the row before the tip alone, all round, then all of every
    // face: so a tip that stands outside the shoulder is told as that.
    for whole in [false, true] {
        for span in &found {
            let (rim, (lo, hi)) = (span.rim, span.range);
            let verdict = if whole {
                certain(&span.seen, &span.size, work)
            } else {
                certain(&last_across(&span.seen), &last_across(&span.size), work)
            };
            let along = |u: f64| lo + u * (hi - lo);
            match verdict {
                Verdict::Proven => {}
                Verdict::Refuted { u, v } => {
                    return Err(if whole {
                        TipError::ClosesIn {
                            rim,
                            at: along(u),
                            across: v,
                        }
                    } else {
                        TipError::NotStarShaped { rim, at: along(u) }
                    });
                }
                Verdict::Undecided { u, .. } => {
                    return Err(TipError::NotProven { rim, at: along(u) });
                }
            }
        }
    }
    Ok(if star.is_finite() { star } else { 0.0 })
}

/// The step from the first control point of a row to the second, or (for
/// the face before) from the last but one to the last: its points' part
/// about the row's own end of the rail, and its weights' part, the
/// weights divided by `scale`.
fn step_at(row: &[(Vec3, f64)], last: bool, scale: f64) -> Option<(Vec3, f64)> {
    if last {
        let (&(q0, v0), &(q1, v1)) = (row.last()?, row.get(row.len().checked_sub(2)?)?);
        let (v0, v1) = (v0 / scale, v1 / scale);
        Some(((q0 - q1) * v1, v0 - v1))
    } else {
        let (&(p0, w0), &(p1, w1)) = (row.first()?, row.get(1)?);
        Some(((p1 - p0) * w1, w1 - w0))
    }
}

/// Per rail, whether its two faces are proven tangent all along it.
///
/// Two faces that share a column of control points are tangent along it
/// where their rows leave it at one ratio of speeds: with `Δ` the step
/// from the column into the later face and `∇` the step into it out of
/// the earlier one, `Δⱼ = μ ∇ⱼ + ν Rⱼ` for every row `j` with one `μ > 0`
/// and one `ν` (`Rⱼ` the column's homogeneous point; a multiple of it
/// moves no point). The derivatives along u on either side are then
/// `Σ Bⱼ(v) Δⱼ` and `Σ Bⱼ(v) ∇ⱼ`, the first `μ` times the second plus `ν`
/// times the rail's own point: one tangent plane. `μ` and `ν` come from
/// the rim's row, where the faces are the rim curves; every row must
/// agree within a quarter of the angular tolerance, as a share of its
/// own length.
fn smooth_rails(nets: &[Net], angular: f64) -> Vec<bool> {
    let count = nets.len();
    let slack = SMOOTH_SHARE * angular;
    let mut smooth = Vec::with_capacity(count);
    for (rail, here) in nets.iter().enumerate() {
        let tangent = || -> Option<bool> {
            let before = nets.get((rail + count - 1) % count)?;
            // The earlier face's column in the later one's weights.
            let scale = before.first()?.last()?.1 / here.first()?.first()?.1;
            let mut ratio: Option<(f64, f64)> = None;
            for (row, other) in here.iter().zip(before) {
                let w0 = row.first()?.1;
                let (step, step_weight) = step_at(row, false, 1.0)?;
                let (back, back_weight) = step_at(other, true, scale)?;
                let (mu, nu) = match ratio {
                    Some(found) => found,
                    None => {
                        let length = back.length_squared();
                        if !(length > 0.0 && step.length_squared() > 0.0) {
                            return Some(false);
                        }
                        if !within(back.angle_to(step), slack) {
                            return Some(false);
                        }
                        let mu = step.dot(back) / length;
                        *ratio.insert((mu, (step_weight - mu * back_weight) / w0))
                    }
                };
                let off = (step - back * mu).length();
                let size = step.length() + mu * back.length();
                let off_weight = (step_weight - mu * back_weight - nu * w0).abs();
                let size_weight = step_weight.abs() + mu * back_weight.abs() + (nu * w0).abs();
                if !(within(off, slack * size) && within(off_weight, slack * size_weight)) {
                    return Some(false);
                }
            }
            Some(ratio.is_some())
        };
        smooth.push(tangent().unwrap_or(false));
    }
    smooth
}

/// Per rail, the angle between its two faces: at the rim, and the
/// largest at [`CREASE_PLACES`] places along it. Each face's normal at a
/// place of the rail is the rail's tangent crossed with the face's
/// derivative along u there, both from the control net.
fn creases(nets: &[Net]) -> Vec<TipCrease> {
    let count = nets.len();
    let mut out = Vec::with_capacity(count);
    for (rail, here) in nets.iter().enumerate() {
        let mut crease = TipCrease {
            at_rim: 0.0,
            largest: 0.0,
        };
        let measured = || -> Option<Vec<f64>> {
            let before = nets.get((rail + count - 1) % count)?;
            let scale = before.first()?.last()?.1 / here.first()?.first()?.1;
            let degree = here.len().checked_sub(1)?;
            let column: Vec<H> = here
                .iter()
                .filter_map(|row| row.first().map(|&(p, w)| h_of(p, w)))
                .collect();
            let mut into: Vec<H> = Vec::with_capacity(here.len());
            let mut out_of: Vec<H> = Vec::with_capacity(here.len());
            for (row, other) in here.iter().zip(before) {
                let (&(p0, w0), &(p1, w1)) = (row.first()?, row.get(1)?);
                let (&(q0, v0), &(q1, v1)) =
                    (other.last()?, other.get(other.len().checked_sub(2)?)?);
                into.push(h_sum(h_of(p1, w1), 1.0, h_of(p0, w0), -1.0));
                out_of.push(h_sum(h_of(q0, v0 / scale), 1.0, h_of(q1, v1 / scale), -1.0));
            }
            let mut angles = Vec::with_capacity(CREASE_PLACES as usize);
            for place in 0..CREASE_PLACES {
                let v = f64::from(place) / f64::from(CREASE_PLACES);
                let basis = crate::bernstein(degree, v);
                let slope = crate::bernstein(degree.saturating_sub(1), v);
                let blend = |rows: &[H], basis: &[f64]| {
                    rows.iter()
                        .zip(basis)
                        .fold([0.0; 4], |sum, (&h, &b)| h_sum(sum, 1.0, h, b))
                };
                let at = blend(&column, &basis);
                let steps: Vec<H> = column
                    .iter()
                    .zip(column.iter().skip(1))
                    .map(|(&a, &b)| h_sum(b, 1.0, a, -1.0))
                    .collect();
                let along_rail = blend(&steps, &slope);
                let point = h_xyz(at) * (1.0 / h_weight(at));
                let flat = |h: H| h_xyz(h) - point * h_weight(h);
                let tangent = flat(along_rail);
                let (later, earlier) = (
                    flat(blend(&into, &basis)).cross(tangent),
                    flat(blend(&out_of, &basis)).cross(tangent),
                );
                let angle = later.angle_to(earlier);
                angles.push(if angle.is_finite() { angle } else { 0.0 });
            }
            Some(angles)
        };
        if let Some(angles) = measured() {
            crease.at_rim = angles.first().copied().unwrap_or(0.0);
            crease.largest = angles.iter().copied().fold(0.0, f64::max);
        }
        out.push(crease);
    }
    out
}

/// Everything about a tip but its curves and surfaces.
struct Solved {
    form: TipForm,
    nets: Vec<Net>,
    onward: f64,
    star: f64,
    counter_clockwise: bool,
    smooth: Vec<bool>,
    creases: Vec<TipCrease>,
}

/// The unit normal, the conic's weight and the tolerances of one tip.
#[derive(Clone, Copy)]
struct Asked {
    tip: Vec3,
    normal: Vec3,
    omega: f64,
    tolerance: f64,
    angular: f64,
}

fn solve(
    sides: &[Side],
    asked: &Asked,
    form: Option<TipForm>,
    work: &mut Work,
) -> Result<Solved, TipError> {
    let shoulder = shoulder(sides, asked.tip, asked.normal, asked.tolerance, form)?;
    let mut nets = nets(sides, &shoulder, asked.tip, asked.omega);
    join_rails(&mut nets, asked.tolerance)?;
    let counter_clockwise = runs_counter_clockwise(sides, asked.normal)?;
    let star = prove(
        sides,
        &nets,
        asked.tip,
        asked.normal,
        counter_clockwise,
        work,
    )?;
    let smooth = smooth_rails(&nets, asked.angular);
    let creases = creases(&nets);
    Ok(Solved {
        form: shoulder.form,
        nets,
        onward: shoulder.onward,
        star,
        counter_clockwise,
        smooth,
        creases,
    })
}

/// A clamped knot vector of one span.
fn one_span(degree: usize) -> Vec<f64> {
    let mut knots = vec![0.0; degree + 1];
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    knots
}

fn curves_and_surfaces(sides: &[Side], solved: Solved, tip: Vec3) -> Result<Tip, TipError> {
    let mut rims = Vec::with_capacity(sides.len());
    let mut faces = Vec::with_capacity(sides.len());
    let mut rails = Vec::with_capacity(sides.len());
    for (side, net) in sides.iter().zip(&solved.nets) {
        let across = net.len().saturating_sub(1);
        let count = side.points.len();
        rims.push(NurbsCurve::new(
            side.degree,
            side.knots.clone(),
            side.points.clone(),
            side.weights.clone(),
        )?);
        let mut points = Vec::with_capacity(count * net.len());
        let mut weights = Vec::with_capacity(count * net.len());
        for i in 0..count {
            for row in net {
                let Some(&(p, w)) = row.get(i) else {
                    return Err(TipError::Invalid("a rim curve without control points"));
                };
                points.push(p);
                weights.push(w);
            }
        }
        faces.push(NurbsSurface::new(
            side.degree,
            across,
            side.knots.clone(),
            one_span(across),
            count,
            net.len(),
            points,
            weights,
        )?);
        let column: Vec<(Vec3, f64)> = net.iter().filter_map(|row| row.first().copied()).collect();
        let rim_weight = column.first().map_or(f64::NAN, |&(_, w)| w);
        rails.push(NurbsCurve::new(
            across,
            one_span(across),
            column.iter().map(|&(p, _)| p).collect(),
            column.iter().map(|&(_, w)| w / rim_weight).collect(),
        )?);
    }
    Ok(Tip {
        form: solved.form,
        rims,
        faces,
        rails,
        tip,
        counter_clockwise: solved.counter_clockwise,
        onward: solved.onward,
        star: solved.star,
        smooth: solved.smooth,
        creases: solved.creases,
    })
}

/// The faces of the tip over the loop of `rims`, or the reason there is
/// none. Nothing is fitted or sampled into being: the nets are closed
/// forms of the rim's and the sides' control points, and what [`Tip`]
/// says is proven of them is proven before they are returned.
///
/// The same input gives the same bits, and the same faces bit for bit
/// whichever rim curve the loop is started from. The loop run the other
/// way round gives the same surfaces to rounding, not to the bit: a rail
/// takes the points of the face that comes after it.
pub fn tip_nets(rims: &[TipRim], spec: &TipSpec) -> Result<Tip, TipError> {
    let (normal, omega) = checked(spec.normal, spec.fullness, spec.tip)?;
    if !(spec.angular.is_finite() && spec.angular > 0.0) {
        return Err(TipError::Invalid(
            "the angular tolerance must be positive and finite",
        ));
    }
    let sides = prepare(rims, spec.tolerance)?;
    let asked = Asked {
        tip: spec.tip,
        normal,
        omega,
        tolerance: spec.tolerance,
        angular: spec.angular,
    };
    let solved = solve(&sides, &asked, None, &mut Work(MAX_WORK))?;
    curves_and_surfaces(&sides, solved, spec.tip)
}

/// What the sides are carried on to in the plane through `on` square to
/// `normal`: one rational curve per rim curve (per half of a single
/// closed one), in the rim's basis and with its weights. It does not
/// depend on where in the plane the tip stands, nor on the fullness, so
/// a place for the tip can be chosen from it: a tip has one tangent
/// plane where the shoulder is star-shaped about it.
pub fn tip_shoulder(
    rims: &[TipRim],
    on: Vec3,
    normal: Vec3,
    tolerance: f64,
) -> Result<(TipForm, Vec<NurbsCurve>), TipError> {
    let (normal, _) = checked(normal, 0.5, on)?;
    let sides = prepare(rims, tolerance)?;
    let shoulder = shoulder(&sides, on, normal, tolerance, None)?;
    let mut curves = Vec::with_capacity(sides.len());
    for (side, row) in sides.iter().zip(&shoulder.rows) {
        curves.push(NurbsCurve::new(
            side.degree,
            side.knots.clone(),
            row.clone(),
            side.weights.clone(),
        )?);
    }
    Ok((shoulder.form, curves))
}

/// The longest tip that fits over one foot, as far as it was found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TipLength {
    /// A length at which every condition is proven: the longest of the
    /// first stretch of lengths that fit, met from short to long among
    /// the lengths tried, within [`LENGTH_RESOLUTION`].
    pub fits: f64,
    /// The shortest length tried beyond `fits` at which the tip is
    /// refused: within [`LENGTH_RESOLUTION`] of `fits`. `None` when
    /// every length tried from `fits` up to the length asked for fits.
    pub fails: Option<f64>,
    /// The shortest length tried beyond `fails` that fits again: the
    /// lengths that fit are then more than one stretch, and `fits` is
    /// the end of the first. `None` when none of those tried does.
    pub again: Option<f64>,
}

/// The longest tip that fits over `foot`: tips stand at `foot + length ×
/// normal`, and lengths up to `upto` are tried.
///
/// The lengths tried first are `upto` halved again and again (32 times
/// at most, down to four tolerances). From the shortest of them that
/// fits the search goes up while they fit: to `upto` itself, or to the
/// first that is refused, and then bisects between the two. So `fits` is
/// proven to fit and `fails` is refused, within [`LENGTH_RESOLUTION`] of
/// one another, and where a longer length among those tried fits again
/// `again` says so. Lengths between those tried are not looked at: a
/// stretch of fitting lengths narrower than a factor of two can be
/// missed, and so can a gap that narrow.
pub fn longest_tip_length(
    rims: &[TipRim],
    foot: Vec3,
    normal: Vec3,
    fullness: f64,
    tolerance: f64,
    upto: f64,
) -> Result<TipLength, TipError> {
    let (normal, omega) = checked(normal, fullness, foot)?;
    if !(upto.is_finite() && upto > 0.0) {
        return Err(TipError::Invalid("the length must be positive and finite"));
    }
    let sides = prepare(rims, tolerance)?;
    // One allowance of work for the whole search.
    let mut work = Work(MAX_WORK);
    let mut attempt = |length: f64| -> Result<bool, TipError> {
        let asked = Asked {
            tip: foot + normal * length,
            normal,
            omega,
            tolerance,
            // (Whether a rail is smooth takes no part in what fits.)
            angular: 1.0,
        };
        match solve(&sides, &asked, None, &mut work) {
            Ok(_) => Ok(true),
            // What no other length mends is told as it is, and so is
            // the end of the work allowed.
            Err(
                error @ (TipError::Invalid(_)
                | TipError::Unsupported(_)
                | TipError::Geometry(_)
                | TipError::RimOpen { .. }
                | TipError::RimCrosses { .. }),
            ) => Err(error),
            Err(error) if work.spent() => Err(error),
            Err(_) => Ok(false),
        }
    };
    // From the length asked for down: the lengths and whether each fits.
    let mut tried = Vec::new();
    let mut length = upto;
    for _ in 0..=LENGTH_HALVINGS {
        tried.push((length, attempt(length)?));
        length *= 0.5;
        if within(length, 4.0 * tolerance) || length.is_nan() {
            break;
        }
    }
    // From short to long: the first that fits, then on while they fit.
    let from_short = || tried.iter().rev().copied();
    let Some(first) = from_short().position(|(_, fits)| fits) else {
        return Err(TipError::NoLengthFits);
    };
    let mut stretch = from_short().skip(first).take_while(|(_, fits)| *fits);
    let Some((mut fits, _)) = stretch.by_ref().last() else {
        return Err(TipError::NoLengthFits);
    };
    let beyond = from_short().skip(first).skip_while(|(_, fits)| *fits);
    let Some((mut fails, _)) = beyond.clone().next() else {
        return Ok(TipLength {
            fits,
            fails: None,
            again: None,
        });
    };
    let again = beyond
        .skip(1)
        .find(|(_, fits)| *fits)
        .map(|(length, _)| length);
    for _ in 0..64 {
        let middle = 0.5 * (fits + fails);
        if within(fails - fits, LENGTH_RESOLUTION * fails) || !(middle > fits && middle < fails) {
            break;
        }
        if attempt(middle)? {
            fits = middle;
        } else {
            fails = middle;
        }
    }
    Ok(TipLength {
        fits,
        fails: Some(fails),
        again,
    })
}
