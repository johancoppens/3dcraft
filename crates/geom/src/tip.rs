//! The nets of a tip: the faces that close a body in one point over the
//! loop of an end face, each running on from the side face behind its rim
//! curve without a crease and all of them reaching the point with one
//! tangent plane.
//!
//! What goes in, per curve of the loop (the *rim*): the curve as a
//! rational B-spline, and the side face's *cross row*, its homogeneous
//! derivative across the rim, pointing onward, written in the rim's own
//! basis (one four-vector per control point; for a spline face whose
//! border is the rim, the difference of its last two rows of homogeneous
//! control points: [`TipRim::of_border`]). And the tip point with the
//! normal of its plane, and the fullness.
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
#[cfg(test)]
mod tests;

use crate::closure::clamped;
use crate::pole::Border;
use crate::{GeomError, NurbsCurve, NurbsSurface};
use craft3d_math::quadrature::Rule;
use craft3d_math::{Bbox3, Vec3};
use prove::{Grid, Verdict, above, at_least, last_across, seen_from_tip};
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
/// its rim toward the tip's plane, all along the rim: below it the side
/// runs along the end and never reaches the tip's plane.
pub const ONWARD_MARGIN: f64 = 1e-6;
/// The least value of the determinant by which the faces are seen from
/// the tip from one side, as a share of the product of its three
/// factors' largest lengths (along the rim, along the row before the tip,
/// and between them in proportion): far above what rounding makes of an
/// exact zero, far below any shape drawn on purpose (a blade ten thousand
/// times longer than thick stays above it).
pub const STAR_MARGIN: f64 = 1e-9;
/// Carry factors that differ by less than this share of the largest are
/// one: the side's cross row reaches the tip's plane as one row.
pub const LEVEL: f64 = 1e-9;
/// Weights of two neighbours' columns on their common rail, each as a
/// share of its rim weight there, that differ by less than this share are
/// the same.
const SAME_WEIGHT: f64 = 1e-9;
/// The least area a rim encloses, as a share of the square of its size:
/// below it the rim is a line there and back, and runs neither way round.
const LEAST_AREA: f64 = 1e-12;
/// How near the longest length that fits is bracketed, as a share of it.
pub const LENGTH_RESOLUTION: f64 = 1e-9;
/// The most lengths one search tries.
const MAX_LENGTH_TRIES: u32 = 160;

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
    /// At the start of rim curve `rim` the two neighbouring sides do not
    /// go the same way: what they are carried on to differs by `gap`.
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
    /// the plane is proven no better than `sine`.
    NotOnward {
        rim: usize,
        control: usize,
        sine: f64,
    },
    /// The side's weights run out before the tip at control point
    /// `control` of rim curve `rim`.
    WeightsRunOut { rim: usize, control: usize },
    /// What the sides are carried on to in the tip's plane is not seen
    /// from the tip from inside all round: at parameter `at` of rim curve
    /// `rim` the tip stands on it or outside it, or it has turned inside
    /// out.
    NotStarShaped { rim: usize, at: f64 },
    /// The sides close in before the tip: at parameter `at` of rim curve
    /// `rim`, the share `across` of the way to the tip, the face turns
    /// its other side to the tip.
    ClosesIn { rim: usize, at: f64, across: f64 },
    /// Neither proven nor refuted within the work allowed: near parameter
    /// `at` of rim curve `rim` the faces are seen from the tip edge-on.
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
            TipError::SidesDisagree { rim, gap } => write!(
                f,
                "the two sides that meet where rim curve {rim} starts do not go on the same way there: carried on, they are {gap:e} apart"
            ),
            TipError::RimReachesPlane { rim, control, room } => write!(
                f,
                "the rim reaches the tip's plane: control point {control} of rim curve {rim} is {room:e} before it"
            ),
            TipError::NotOnward { rim, control, sine } => write!(
                f,
                "the side behind rim curve {rim} does not go onward toward the tip at control point {control}: it leaves the end at a sine of {sine:e}"
            ),
            TipError::WeightsRunOut { rim, control } => write!(
                f,
                "the weights of the side behind rim curve {rim} fall to zero before the tip at control point {control}"
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
                "the tip's faces could not be proven to face the tip one way: near parameter {at} of rim curve {rim} they are seen from it edge-on"
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
    /// next rim curve starts. Its knot vector is clamped.
    pub curve: NurbsCurve,
    /// The side's homogeneous derivative across the rim, pointing onward
    /// (out of the side, over the end), in the rim's basis: `[x, y, z,
    /// w]` per control point of `curve`. Its length does not matter along
    /// a side whose row reaches the tip's plane as one; where it does
    /// not, neighbours must agree in it at their common vertex.
    pub cross: Vec<[f64; 4]>,
}

impl TipRim {
    /// The rim and the cross row of a spline face that ends on one of its
    /// borders: the border's row of control points as the rim curve
    /// (running the way the face's parameter along it runs), and the
    /// face's derivative across it, outward, from the border's row and
    /// the row before it. Both knot vectors must be clamped.
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
        // The border's row and the one before it, and the knots between
        // which the derivative across is their difference.
        let (row, before, lo, hi) = if at_end {
            (
                across_count - 1,
                across_count - 2,
                across_count - 1,
                across_count - 1 + across_degree,
            )
        } else {
            (0, 1, 1, across_degree + 1)
        };
        let (Some(&lo), Some(&hi)) = (across_knots.get(lo), across_knots.get(hi)) else {
            return Err(TipError::Invalid("a side face without knots across"));
        };
        let pace = across_degree as f64 / (hi - lo);
        if !(pace.is_finite() && pace > 0.0) {
            return Err(TipError::Invalid(
                "a side face whose last knot span across is empty",
            ));
        }
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
            cross.push(h_sum(h_of(c, wc), pace, h_of(b, wb), -pace));
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
    /// The length below which two points are one: how far the rim curves
    /// may be apart where they join, and how far before the tip's plane
    /// the rim must stay.
    pub tolerance: f64,
    /// The angle below which two directions are one: a rim vertex with
    /// less between its two curves is smooth.
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

/// The faces of a tip, and what was proven of them.
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
    /// points and its weights as shares of the first; the face before
    /// ends on the same control points (bit for bit beyond the rim, where
    /// two rim curves may meet within the tolerance).
    pub rails: Vec<NurbsCurve>,
    pub tip: Vec3,
    /// Whether the rim runs counter-clockwise seen against the normal
    /// (from beyond the tip): the faces' `∂u × ∂v` then points out of the
    /// body, and into it otherwise.
    pub counter_clockwise: bool,
    /// The least sine of the angle at which the sides leave the rim's
    /// plane toward the tip's, proven all along the rim: at least
    /// [`ONWARD_MARGIN`].
    pub onward: f64,
    /// The least determinant by which the row before the tip is seen from
    /// the tip, as a share of its factors' largest lengths there, proven
    /// all round: at least [`STAR_MARGIN`]. It is what the faces' normals
    /// near the tip are held to the plane's by: they differ from it in
    /// proportion to the distance from the tip over this number.
    pub star: f64,
    /// Per rail: whether its two faces are proven tangent all along it.
    /// They are where the rim is smooth at the vertex (within the angular
    /// tolerance) and the two faces' rows leave the rail at one ratio of
    /// speeds, row for row, which they do when the sides' own rows do. A
    /// rail that is not smooth is an edge whose angle falls to zero at
    /// the tip: where the rim has a corner, and also where the two sides
    /// are tangent but their rows are out of step (weights that change
    /// across the rim on one row and not on the other); a caller that
    /// knows the sides to be tangent along their common edge refuses that
    /// case, a crease the sides do not have.
    pub smooth: Vec<bool>,
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
/// end to end into a loop.
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

/// What the sides are carried on to in the tip's plane: the shoulder.
struct Shoulder {
    form: TipForm,
    /// Per side, the carry factor of its row: its own where every side
    /// is level, the least of all otherwise.
    carry: Vec<f64>,
    /// Per side, the cross row as it is used: with the share of the rim's
    /// own homogeneous points taken out that the weights of all sides
    /// have in common (it changes no derivative of the side).
    cross: Vec<Vec<H>>,
    /// Per side and control point: the shoulder's point in the tip's
    /// plane and its weight.
    rows: Vec<Vec<(Vec3, f64)>>,
    /// The least sine at which a side leaves the rim's plane.
    onward: f64,
}

/// Carries every side on to the tip's plane (the plane through `on`
/// square to the unit `normal`), after the checks that it can be.
///
/// With `π(X, w) = n · X − c w` the plane's form on homogeneous points
/// (`c` its level), a control point `C` of weight `w` with cross vector
/// `D` reaches the plane at `C + τ D`, `τ = −π(C) / π(D)`: its *carry
/// factor*. A side is level when its factors are one number: `C + τ D`
/// is then one row of the side's own kind, which is what keeps the tip
/// tangent to the side along the rim and puts the row in the plane.
/// Where the factors differ the row is carried by the least of them
/// (nothing passes the plane) and dropped onto the plane along the
/// normal: one map for every control point, so what holds between two
/// neighbours' rows across their common edge holds between the tip's.
fn shoulder(
    sides: &[Side],
    on: Vec3,
    normal: Vec3,
    tolerance: f64,
    form: Option<TipForm>,
) -> Result<Shoulder, TipError> {
    let level_of_plane = normal.dot(on);
    // The share of the rim's own points in the cross rows: half way
    // between the least and the largest ratio of the weights.
    let (mut least, mut most) = (f64::INFINITY, f64::NEG_INFINITY);
    for side in sides {
        for (d, &w) in side.cross.iter().zip(&side.weights) {
            let ratio = h_weight(*d) / w;
            least = least.min(ratio);
            most = most.max(ratio);
        }
    }
    let share = 0.5 * (least + most);
    if !share.is_finite() {
        return Err(TipError::Invalid("a rim curve without control points"));
    }
    let mut cross = Vec::with_capacity(sides.len());
    let mut factors: Vec<Vec<f64>> = Vec::with_capacity(sides.len());
    let mut onward = f64::INFINITY;
    for (rim, side) in sides.iter().enumerate() {
        let row: Vec<H> = side
            .cross
            .iter()
            .zip(side.homogeneous())
            .map(|(&d, c)| {
                if share == 0.0 {
                    d
                } else {
                    h_sum(d, 1.0, c, -share)
                }
            })
            .collect();
        let levels: Vec<f64> = side.points.iter().map(|p| normal.dot(*p)).collect();
        let lowest = levels.iter().copied().fold(f64::INFINITY, f64::min);
        let Some((lo, hi)) = Bbox3::from_points(side.points.iter().copied()).corners() else {
            return Err(TipError::Invalid("a rim curve without control points"));
        };
        let mut taus = Vec::with_capacity(row.len());
        for (control, ((d, &w), &level)) in row.iter().zip(&side.weights).zip(&levels).enumerate() {
            let room = level_of_plane - level;
            if within(room, tolerance) || room.is_nan() {
                return Err(TipError::RimReachesPlane { rim, control, room });
            }
            let (direction, weight) = (h_xyz(*d), h_weight(*d));
            // The derivative across, times the rim's weight, is
            // Σ N (D_xyz − D_w C(u)) with C(u) in the box of the control
            // points and between the lowest of them and the tip's plane:
            // its height is at least the least of the two ends, its
            // length at most the largest over the box's corners.
            let rise = normal.dot(direction);
            let up = (rise - weight * lowest).min(rise - weight * level_of_plane);
            let mut reach = 0.0_f64;
            for x in [lo.x, hi.x] {
                for y in [lo.y, hi.y] {
                    for z in [lo.z, hi.z] {
                        let at = if weight == 0.0 {
                            direction
                        } else {
                            direction - Vec3::new(x, y, z) * weight
                        };
                        reach = reach.max(at.length());
                    }
                }
            }
            let sine = up / reach;
            let tau = w * room / (rise - weight * level_of_plane);
            if !(sine >= ONWARD_MARGIN && tau.is_finite() && tau > 0.0) {
                return Err(TipError::NotOnward {
                    rim,
                    control,
                    sine: if sine.is_finite() { sine } else { 0.0 },
                });
            }
            onward = onward.min(sine);
            taus.push(tau);
        }
        cross.push(row);
        factors.push(taus);
    }
    let range = |taus: &[f64]| {
        taus.iter().fold((f64::INFINITY, 0.0_f64), |(lo, hi), &t| {
            (lo.min(t), hi.max(t))
        })
    };
    let all_level = factors.iter().all(|taus| {
        let (lo, hi) = range(taus);
        hi - lo <= LEVEL * hi
    });
    let form = form.unwrap_or(if all_level {
        TipForm::Level
    } else {
        TipForm::General
    });
    let least_of_all = factors
        .iter()
        .map(|taus| range(taus).0)
        .fold(f64::INFINITY, f64::min);
    let mut carry = Vec::with_capacity(sides.len());
    let mut rows = Vec::with_capacity(sides.len());
    for (rim, ((side, row), taus)) in sides.iter().zip(&cross).zip(&factors).enumerate() {
        let tau = match form {
            TipForm::Level => {
                let (lo, hi) = range(taus);
                0.5 * (lo + hi)
            }
            TipForm::General => least_of_all,
        };
        let mut points = Vec::with_capacity(row.len());
        for (control, (c, &d)) in side.homogeneous().zip(row).enumerate() {
            let carried = h_sum(c, 1.0, d, tau);
            let weight = h_weight(carried);
            if !(weight.is_finite() && weight > 0.0) {
                return Err(TipError::WeightsRunOut { rim, control });
            }
            let mut point = h_xyz(carried) * (1.0 / weight);
            if form == TipForm::General {
                point = point + normal * (level_of_plane - normal.dot(point));
            }
            points.push((point, weight));
        }
        carry.push(tau);
        rows.push(points);
    }
    Ok(Shoulder {
        form,
        carry,
        cross,
        rows,
        onward,
    })
}

/// The control net of one face: `rows[j][i]`, row `j` from the rim
/// (`j = 0`) to the tip, each control point with its weight.
type Net = Vec<Vec<(Vec3, f64)>>;

/// The nets of all faces from the shoulder.
///
/// Level, a conic across (`C` the rim's homogeneous points, `S` the
/// shoulder's, `P` the tip, `ω` the conic's weight):
/// `C`, `ω S`, `κ (P, 1)`.
/// General, the same conic raised to degree 3 with its second row short
/// of the shoulder where the side is slower (`S̄` the row carried by the
/// least factor, `S` that row dropped onto the plane):
/// `C`, `(C + 2 ω S̄) / 3`, `(2 ω S + κ (P, 1)) / 3`, `κ (P, 1)`.
/// `κ` is the weight of the rim's point carried twice as far: the rim's
/// own weight where the side's weights do not change across the rim, and
/// one row of the side's kind always.
fn nets(sides: &[Side], shoulder: &Shoulder, tip: Vec3, omega: f64) -> Result<Vec<Net>, TipError> {
    let mut nets: Vec<Net> = Vec::with_capacity(sides.len());
    for (rim, (((side, cross), row), &tau)) in sides
        .iter()
        .zip(&shoulder.cross)
        .zip(&shoulder.rows)
        .zip(&shoulder.carry)
        .enumerate()
    {
        let count = side.points.len();
        let rows = match shoulder.form {
            TipForm::Level => 3,
            TipForm::General => 4,
        };
        let mut net: Net = (0..rows).map(|_| Vec::with_capacity(count)).collect();
        for (control, (((&c, &w), &d), &(point, weight))) in side
            .points
            .iter()
            .zip(&side.weights)
            .zip(cross)
            .zip(row)
            .enumerate()
        {
            let pole = w + 2.0 * tau * h_weight(d);
            if !(pole.is_finite() && pole > 0.0) {
                return Err(TipError::WeightsRunOut { rim, control });
            }
            let rows: Vec<(Vec3, f64)> = match shoulder.form {
                TipForm::Level => vec![(c, w), (point, omega * weight), (tip, pole)],
                TipForm::General => {
                    let second = h_sum(
                        h_of(c, w),
                        (1.0 + 2.0 * omega) / 3.0,
                        d,
                        2.0 * omega * tau / 3.0,
                    );
                    let second_weight = h_weight(second);
                    if !(second_weight.is_finite() && second_weight > 0.0) {
                        return Err(TipError::WeightsRunOut { rim, control });
                    }
                    let third_weight = (2.0 * omega * weight + pole) / 3.0;
                    let toward = 2.0 * omega * weight / (2.0 * omega * weight + pole);
                    vec![
                        (c, w),
                        (h_xyz(second) * (1.0 / second_weight), second_weight),
                        (tip + (point - tip) * toward, third_weight),
                        (tip, pole),
                    ]
                }
            };
            for (row, entry) in net.iter_mut().zip(rows) {
                row.push(entry);
            }
        }
        nets.push(net);
    }
    Ok(nets)
}

/// Where two faces meet on a rail, their columns must be one curve: the
/// rows between the rim and the tip are compared (a side whose cross row
/// goes another way than its neighbour's at their common vertex leaves a
/// gap) and the earlier face's column takes the later face's points.
fn join_rails(nets: &mut [Net], tolerance: f64) -> Result<(), TipError> {
    let count = nets.len();
    let mut starts: Vec<Vec<(Vec3, f64)>> = Vec::with_capacity(count);
    for net in nets.iter() {
        let rim_weight = net
            .first()
            .and_then(|row| row.first())
            .map_or(f64::NAN, |&(_, w)| w);
        starts.push(
            net.iter()
                .filter_map(|row| row.first())
                .map(|&(p, w)| (p, w / rim_weight))
                .collect(),
        );
    }
    for (rim, start) in starts.iter().enumerate() {
        let Some(before) = nets.get_mut((rim + count - 1) % count) else {
            continue;
        };
        let rim_weight = before
            .first()
            .and_then(|row| row.last())
            .map_or(f64::NAN, |&(_, w)| w);
        let rows = before.len();
        for (j, (row, &(point, share))) in before.iter_mut().zip(start).enumerate() {
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
            let own = last.1 / rim_weight;
            if !within(
                (own - share).abs(),
                SAME_WEIGHT * own.abs().max(share.abs()),
            ) {
                return Err(TipError::Unsupported(
                    "sides whose weights change across the rim at different rates where they meet",
                ));
            }
            last.0 = point;
        }
        // The pole's weights must be one conic's too.
        if let (Some(&(_, pole)), Some(&(_, share))) =
            (before.last().and_then(|row| row.last()), start.last())
        {
            let own = pole / rim_weight;
            if !within(
                (own - share).abs(),
                SAME_WEIGHT * own.abs().max(share.abs()),
            ) {
                return Err(TipError::Unsupported(
                    "sides whose weights change across the rim at different rates where they meet",
                ));
            }
        }
    }
    Ok(())
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

/// Whether the rim runs counter-clockwise seen against the normal: the
/// sign of the area it encloses (Green's theorem about its first point,
/// by Gauss–Legendre on every knot span). A rim that encloses nothing
/// runs neither way.
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
        for (&lo, &hi) in side.knots.iter().zip(side.knots.iter().skip(1)) {
            if within(hi, lo) || hi.is_nan() {
                continue;
            }
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
/// shows it, everywhere: the least value of the determinant along the
/// row before the tip, as a share of its size there.
///
/// What it means. A ray from the tip crosses the faces transversally and
/// always the same way round, so the faces are a graph over the
/// directions seen from the tip: regular (no fold, no second point where
/// a face shrinks to nothing) and, over a rim that lies in one plane,
/// met once by every ray, so they neither cross themselves nor one
/// another. Along `v = 1` it is the condition for one tangent plane at
/// the tip: the shoulder is star-shaped about the tip and runs the way
/// the rim runs. Along `v = 0` it says the rim is seen from the tip on
/// the outside of what it is carried on to: the sides have not closed in
/// and crossed over before the tip's plane.
fn prove(
    sides: &[Side],
    nets: &[Net],
    tip: Vec3,
    counter_clockwise: bool,
) -> Result<f64, TipError> {
    /// One Bézier span of a face: the determinant and its sizes.
    struct Span {
        rim: usize,
        range: (f64, f64),
        seen: Grid<f64>,
        sizes: [f64; 2],
    }
    let mut spans = Vec::new();
    for (rim, (side, net)) in sides.iter().zip(nets).enumerate() {
        let about: Vec<Vec<H>> = net
            .iter()
            .take(net.len().saturating_sub(1))
            .map(|row| row.iter().map(|&(p, w)| h_of(p - tip, w)).collect())
            .collect();
        for k in side.degree..side.points.len() {
            let (Some(&lo), Some(&hi)) = (side.knots.get(k), side.knots.get(k + 1)) else {
                continue;
            };
            if within(hi, lo) || hi.is_nan() {
                continue;
            }
            let unproven = TipError::NotProven {
                rim,
                at: 0.5 * (lo + hi),
            };
            let mut rows = Vec::with_capacity(about.len());
            for row in &about {
                let Some(span) = bezier_of_span(side.degree, &side.knots, k, row) else {
                    return Err(unproven);
                };
                rows.push(span.into_iter().map(h_xyz).collect::<Vec<Vec3>>());
            }
            let Some((seen, sizes)) = seen_from_tip(&rows) else {
                return Err(unproven);
            };
            spans.push(Span {
                rim,
                range: (lo, hi),
                seen,
                sizes,
            });
        }
    }
    if spans.is_empty() {
        return Err(TipError::Invalid(
            "a rim without a span to build a face over",
        ));
    }
    // A rim that runs counter-clockwise against the normal is seen from
    // the tip with a positive determinant, the other with a negative.
    for span in &mut spans {
        if !counter_clockwise {
            for x in &mut span.seen.c {
                *x = -*x;
            }
        }
        span.seen = above(&span.seen, span.sizes.map(|size| STAR_MARGIN * size));
    }
    let mut star = f64::INFINITY;
    // First the row before the tip alone, all round, then all of every
    // face: so a tip that stands outside the shoulder is told as that.
    for whole in [false, true] {
        for span in &spans {
            let (rim, (lo, hi)) = (span.rim, span.range);
            let verdict = if whole {
                at_least(&span.seen, 0.0)
            } else {
                at_least(&last_across(&span.seen), 0.0)
            };
            let along = |u: f64| lo + u * (hi - lo);
            match verdict {
                Verdict::Proven { least } => {
                    if !whole {
                        let [_, size] = span.sizes;
                        star = star.min(STAR_MARGIN + least / size);
                    }
                }
                Verdict::Refuted { u, v, .. } => {
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
    Ok(star)
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
/// the rim's row, where the faces are the rim curves; the rows beyond
/// must agree within the angular tolerance, as shares of their own
/// lengths.
fn smooth_rails(nets: &[Net], angular: f64) -> Vec<bool> {
    let count = nets.len();
    let mut smooth = Vec::with_capacity(count);
    for (rail, here) in nets.iter().enumerate() {
        let tangent = || -> Option<bool> {
            let before = nets.get((rail + count - 1) % count)?;
            // The earlier face's column in the later one's weights.
            let scale = before.first()?.last()?.1 / here.first()?.first()?.1;
            let mut ratio: Option<(f64, f64, f64)> = None;
            for (row, other) in here.iter().zip(before) {
                let (&(p0, w0), &(p1, w1)) = (row.first()?, row.get(1)?);
                let (&(q0, v0), &(q1, v1)) =
                    (other.last()?, other.get(other.len().checked_sub(2)?)?);
                let (v0, v1) = (v0 / scale, v1 / scale);
                // The steps, each about its own end of the rail: their
                // points' parts and their weights' parts.
                let (step, step_weight) = ((p1 - p0) * w1, w1 - w0);
                let (back, back_weight) = ((q0 - q1) * v1, v0 - v1);
                let (mu, nu, slack) = match ratio {
                    Some(found) => found,
                    None => {
                        let length = back.length_squared();
                        if !(length > 0.0 && step.length_squared() > 0.0) {
                            return Some(false);
                        }
                        if !within(back.angle_to(step), angular) {
                            return Some(false);
                        }
                        let mu = step.dot(back) / length;
                        let nu = (step_weight - mu * back_weight) / w0;
                        let slack = (step - back * mu).length() / step.length();
                        *ratio.insert((mu, nu, angular + 4.0 * slack))
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

/// Everything about a tip but its curves and surfaces.
struct Solved {
    form: TipForm,
    nets: Vec<Net>,
    onward: f64,
    star: f64,
    counter_clockwise: bool,
    smooth: Vec<bool>,
}

fn solve(
    sides: &[Side],
    tip: Vec3,
    normal: Vec3,
    omega: f64,
    tolerance: f64,
    angular: f64,
    form: Option<TipForm>,
) -> Result<Solved, TipError> {
    let shoulder = shoulder(sides, tip, normal, tolerance, form)?;
    let mut nets = nets(sides, &shoulder, tip, omega)?;
    join_rails(&mut nets, tolerance)?;
    let counter_clockwise = runs_counter_clockwise(sides, normal)?;
    let star = prove(sides, &nets, tip, counter_clockwise)?;
    let smooth = smooth_rails(&nets, angular);
    Ok(Solved {
        form: shoulder.form,
        nets,
        onward: shoulder.onward,
        star,
        counter_clockwise,
        smooth,
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
    })
}

/// The faces of the tip over the loop of `rims`, or the reason there is
/// none. Nothing is fitted or sampled: the nets are closed forms of the
/// rim's and the sides' control points, and what is promised of them is
/// proven on convex hulls before they are returned.
pub fn tip_nets(rims: &[TipRim], spec: &TipSpec) -> Result<Tip, TipError> {
    let (normal, omega) = checked(spec.normal, spec.fullness, spec.tip)?;
    if !(spec.angular.is_finite() && spec.angular > 0.0) {
        return Err(TipError::Invalid(
            "the angular tolerance must be positive and finite",
        ));
    }
    let sides = prepare(rims, spec.tolerance)?;
    let solved = solve(
        &sides,
        spec.tip,
        normal,
        omega,
        spec.tolerance,
        spec.angular,
        None,
    )?;
    curves_and_surfaces(&sides, solved, spec.tip)
}

/// What the sides are carried on to in the plane through `on` square to
/// `normal`: one rational curve per rim curve (per half of a single
/// closed one), in the rim's basis. It does not depend on where in the
/// plane the tip stands, so a place for the tip can be chosen from it: a
/// tip has one tangent plane where the shoulder is star-shaped about it.
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
            row.iter().map(|&(p, _)| p).collect(),
            row.iter().map(|&(_, w)| w).collect(),
        )?);
    }
    Ok((shoulder.form, curves))
}

/// A length at which a tip fits, bracketed from above.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TipLength {
    /// A length at which every condition is proven.
    pub fits: f64,
    /// The shortest length tried beyond `fits` at which the tip is
    /// refused: within [`LENGTH_RESOLUTION`] of `fits`. `None` when the
    /// length asked for fits.
    pub fails: Option<f64>,
}

/// The longest tip that fits over `foot`: tips stand at `foot + length ×
/// normal`, and lengths up to `upto` are tried.
///
/// The search halves `upto` until a length fits and then bisects between
/// that length and the shortest refused one above it. So `fits` is
/// proven to fit and `fails` is refused, and they are within
/// [`LENGTH_RESOLUTION`] of one another; where the lengths that fit are
/// one stretch from nothing to a limit (sides that close in, as every
/// case with a closed form does), that limit lies between the two.
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
    let attempt = |length: f64| -> Result<(), TipError> {
        // (Whether a rail is smooth takes no part in what fits.)
        let tip = foot + normal * length;
        solve(&sides, tip, normal, omega, tolerance, 1.0, None).map(|_| ())
    };
    // What no shorter tip mends is told as it is.
    match attempt(upto) {
        Ok(()) => {
            return Ok(TipLength {
                fits: upto,
                fails: None,
            });
        }
        Err(
            error @ (TipError::Invalid(_)
            | TipError::Unsupported(_)
            | TipError::Geometry(_)
            | TipError::RimOpen { .. }),
        ) => return Err(error),
        Err(_) => {}
    }
    let mut tries = 1_u32;
    let (mut fits, mut fails) = (0.0_f64, upto);
    let mut length = upto;
    while tries < MAX_LENGTH_TRIES && fits == 0.0 {
        length *= 0.5;
        if within(length, tolerance) || length.is_nan() {
            break;
        }
        tries += 1;
        match attempt(length) {
            Ok(()) => fits = length,
            Err(_) => fails = length,
        }
    }
    if fits == 0.0 {
        return Err(TipError::NoLengthFits);
    }
    while tries < MAX_LENGTH_TRIES && fails - fits > LENGTH_RESOLUTION * fails {
        let middle = 0.5 * (fits + fails);
        if !(middle > fits && middle < fails) {
            break;
        }
        tries += 1;
        match attempt(middle) {
            Ok(()) => fits = middle,
            Err(_) => fails = middle,
        }
    }
    Ok(TipLength {
        fits,
        fails: Some(fails),
    })
}
