//! Sweep along two rails: the solid a closed profile sweeps out as it is
//! carried so that two of its points follow two curves in space.
//!
//! **The rails** are two chains of lines, arcs and splines, each drawn
//! in a sketch of its own (so the two may lie in different planes), both
//! open or both closed (one circle is a closed rail: two half turns).
//! An open rail starts on the profile: the end of a chain that touches
//! the profile's outline (within the tolerance) is where it starts. A
//! closed rail passes through the profile's plane on its outline (where
//! it does so nearest the outline: along a line and an arc in closed
//! form, along a spline by sign changes and bisection) and is travelled
//! the way the profile's normal points. Those two points of the outline,
//! `a` and `b`, are the points that follow the rails.
//!
//! **Rings.** Round two closed rails the stations are fractions of each
//! ring's length counted from `a` and `b`, as for open rails, and the
//! placement at `σ = 1` is the one at `σ = 0`. The body starts at the
//! first joint of either ring after the profile (so the curve the profile
//! is on is not cut there); the other ring's curve there is cut in two,
//! each part keeping the curve's parameter and its share of `σ`, and runs
//! first and last, beside different curves of the first ring, so no two
//! faces have one name. The last leg ends on the first leg's ring of
//! edges (its last station is the start's, whose gap from the exact one
//! is in the proof like any other); a shell per loop of the profile, no
//! caps.
//!
//! **Stations.** A station is a value `σ` from 0 to 1 and a point on each
//! rail: `A(σ)` at the fraction `σ` of rail A's length, `B(σ)` at the same
//! fraction of rail B's. Along a line or an arc that is by arc length,
//! exactly; along a spline piece by the spline's own parameter, the piece
//! taking the share of `σ` its length has (by Gauss–Legendre). The chord
//! `c(σ) = B(σ) − A(σ)` is never zero: rails that meet or cross anywhere,
//! at any pair of places, are refused before anything is built (below).
//!
//! **How the section is placed.** At each station the profile is put down
//! by a similarity that takes `a` to `A(σ)` and `b` to `B(σ)`. Its x axis
//! (from `a` to `b`) is the chord; its y axis is the *up* direction made
//! square to the chord: `w = (up − (up·e) e) / |…|`, `e = c / |c|`. The up
//! direction is the one given ([`sweep_two_rails_up`]) or the normal of
//! rail A's sketch plane, turned round when the rails leave the profile
//! against it: for rails drawn in one plane the sections stand square to
//! that plane by default. A constant `up` keeps every bound below as it
//! is; an up direction along the rails lays the section into their course
//! and is refused as the fold it makes. A profile is drawn in the plane of
//! the chord and the up direction at the start (a profile that leans off
//! it by less than the linear tolerance moves is laid into it). The
//! profile is scaled by `s = |c(σ)| / |b − a|`: **uniformly** by default,
//! the standard behaviour (a circle stays a circle, a square a square), or
//! only **along the chord** ([`Scaling::Width`]: the height stays as
//! drawn, a circle becomes an ellipse). The point `(x, y)` of the profile,
//! in axes with `a` at the origin and `b` on the x axis, is then at
//!
//! ```text
//! P(x, y, σ) = O(σ) + U(σ) x + B(σ) y,   O = A(σ),   U = c / |b − a|,
//! B = w s (uniform) or w (width only).
//! ```
//!
//! **The surface.** Each curve of the profile sweeps one face per *leg*:
//! a stretch of `σ` over which each rail runs along one piece. Where the
//! legs meet every face shares the profile's edges at an exact station.
//! A leg along which both rails are parallel lines of equal length, the
//! profile keeps its size and moves square to its plane is a translation,
//! and is swept exactly (planes and cylinders, as an extrusion; at a slant
//! a circle would sweep an elliptic cylinder, so that leg is followed). Every other leg is a B-spline surface
//! through the profile placed at stations along it, in parts of degree 5,
//! 7 or 9 through the Chebyshev points ([`crate::sweep::part_band`]), as
//! along a spline path. **The bound is proven, not sampled:** the surface
//! through stations whose placements are `(o_i, u_i, b_i)` is exactly
//! `O*(v) + U*(v) x(u) + B*(v) y(u)`, where `O*`, `U*` and `B*` are the
//! band's polynomials through `o_i`, `u_i` and `b_i` and `(x(u), y(u))` is
//! the profile's curve (affine maps commute with the B-spline sums, and
//! the weights of an arc are the same at every station). So its distance
//! from the true sweep at `(u, v)` is at most
//! `|ΔO| + X |ΔU| + Y |ΔB|`, with `X` and `Y` the largest `|x|` and `|y|`
//! of the profile's control points. Each `Δ` is a function of `σ` alone
//! whose second derivative is bounded by the polynomial's (from its
//! second differences) and the true placement's (from the rails' speeds
//! and bends, through the chord's length and the angle between the chord
//! and the up direction, each bounded from below first). On an interval of
//! width `h` a vector function with `|Δ''| ≤ M` is within `M h² / 8` of
//! the chord of its end values, so the deviation is at most the largest
//! sampled value plus `M h² / 8` plus rounding, with `h` chosen so that
//! the last term is at most an eighth of the tolerance. Degrees and parts
//! are raised until that bound is at most **half the linear tolerance**;
//! what was reached is [`Swept::deviation`].
//!
//! **What is refused, before anything is built:** a rail that is not one
//! chain, an open rail with a closed one, a rail that does not start on
//! (or a ring that does not pass through) the profile's outline, an up
//! direction of no length, rails
//! that meet or cross (a branch and bound over pairs of places, each pair
//! of boxes judged by the distance at its centre less what the rails can
//! move within it: proven apart, or refused with the place), rails that
//! start at one point, a profile whose plane does not hold the chord and
//! the up direction, a chord that turns into the up direction, joints of
//! the two rails closer along them than four tolerances (but not at one
//! fraction), and a sweep that folds: the speed of every point of the
//! profile across its station's plane, proven positive the same way as
//! the deviation. What passes through itself without folding is for the
//! caller's self-intersection check, as for every sweep.
//!
//! **Names.** A side is named after the curve of the profile and the
//! pieces of rail A and rail B it runs between (`side(e3,e12,e7)`); faces
//! with the same three are numbered in the order they are made (a whole
//! circle of the profile is four quarters). The caps are `startCap(…)` and
//! `endCap(…)` after the curves of the profile's outer loop.

use crate::extrude::{Origin, Role, cap_sources, each_curve_once};
use crate::loft::Band;
use crate::profile::{Curve2, Profile, Segment, Shape, SketchCurve, chain};
use crate::spline2::{Spline2, distance_to};
use crate::sweep::{
    Builder, Form, Map2, Motion, P2, Station, Swept, coedge, part_band, point_text, profile_loops,
    quarters, refuse, stations_of,
};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::NurbsCurve;
use craft3d_math::quadrature::Rule;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, EdgeId, FaceId};
use std::collections::BTreeMap;

/// How the profile is scaled between the rails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scaling {
    /// By the ratio of the chord's length to the profile's, both ways.
    #[default]
    Uniform,
    /// Only along the chord; the height stays as drawn.
    Width,
}

/// How a leg is followed, tried in this order: in how many parts, and of
/// what degree the surface is along each.
const PLANS: [(usize, usize); 10] = [
    (1, 5),
    (1, 7),
    (1, 9),
    (2, 7),
    (2, 9),
    (4, 7),
    (4, 9),
    (8, 9),
    (16, 9),
    (32, 9),
];

/// Most samples the proof of one plan of one leg takes.
const MAX_SAMPLES: usize = 1 << 20;

/// Most samples one sweep takes in all, over every leg and plan and the
/// grids that bound the chord and the fold.
const MAX_TOTAL_SAMPLES: usize = 1 << 23;

/// The finest tolerance a sweep takes, in units in the last place of its
/// size (how far from the origin its points reach, the profile's reach
/// included): below it the rounding of the band's sums and of the
/// surface's evaluation, which the proof bounds by `256 ε` times the
/// size, would not be at most a sixteenth of the tolerance.
const FINEST: f64 = 4096.0;

/// Most boxes the search for where the rails come nearest takes.
const MAX_BOXES: usize = 1 << 20;

/// Most legs (pieces of the two rails together).
const MAX_LEGS: usize = 4096;

/// Most points of the grid that bounds the chord and the fold of one leg.
const MAX_GRID: usize = 1 << 18;

/// Fractions of the rails closer than this are one joint.
const SAME_JOINT: f64 = 1e-12;

fn internal() -> KernelError {
    refuse(
        &[],
        "the sweep along two rails lost track of a curve; this is a bug",
    )
}

/// A direction as the caller gave it, short whatever its size: three
/// significant digits each.
fn direction_text(d: Vec3) -> String {
    format!("({:.3e}, {:.3e}, {:.3e})", d.x, d.y, d.z)
}

/// What a piece of a rail runs along, in the direction of travel.
#[derive(Clone, Debug)]
enum Course {
    Line {
        from: Vec3,
        to: Vec3,
    },
    /// `center + radius (cos φ x + sin φ y)` for `φ` from 0 to `sweep`.
    Arc {
        center: Vec3,
        x: Vec3,
        y: Vec3,
        radius: f64,
        sweep: f64,
    },
    /// The curve from `t0` to `t1`; `speed` and `bend` bound the norms of
    /// its first and second derivatives in its own parameter.
    Spline {
        curve: NurbsCurve,
        derivative: NurbsCurve,
        t0: f64,
        t1: f64,
        speed: f64,
        bend: f64,
    },
}

impl Course {
    /// The point at the share `q` of the course (its own parameter).
    fn at(&self, q: f64) -> Vec3 {
        match self {
            Course::Line { from, to } => *from + (*to - *from) * q,
            Course::Arc {
                center,
                x,
                y,
                radius,
                sweep,
            } => {
                let (s, c) = (sweep * q).psin_cos();
                *center + (*x * c + *y * s) * *radius
            }
            Course::Spline { curve, t0, t1, .. } => curve.eval(t0 + (t1 - t0) * q),
        }
    }

    /// The derivative by the share at `q`.
    fn tangent(&self, q: f64) -> Vec3 {
        match self {
            Course::Line { from, to } => *to - *from,
            Course::Arc {
                x,
                y,
                radius,
                sweep,
                ..
            } => {
                let (s, c) = (sweep * q).psin_cos();
                (*y * c - *x * s) * (radius * sweep)
            }
            Course::Spline {
                derivative, t0, t1, ..
            } => derivative.eval(t0 + (t1 - t0) * q) * (t1 - t0),
        }
    }

    /// The stretch of the course from the share `q0` to `q1`, run by the
    /// same parameter (a spline's bounds on its derivatives hold for any
    /// stretch of it).
    fn between(&self, q0: f64, q1: f64) -> Course {
        match self {
            Course::Line { .. } => Course::Line {
                from: self.at(q0),
                to: self.at(q1),
            },
            Course::Arc {
                center,
                x,
                y,
                radius,
                sweep,
            } => {
                let (s, c) = (sweep * q0).psin_cos();
                Course::Arc {
                    center: *center,
                    x: *x * c + *y * s,
                    y: *y * c - *x * s,
                    radius: *radius,
                    sweep: sweep * (q1 - q0),
                }
            }
            Course::Spline {
                curve,
                derivative,
                t0,
                t1,
                speed,
                bend,
            } => Course::Spline {
                curve: curve.clone(),
                derivative: derivative.clone(),
                t0: t0 + (t1 - t0) * q0,
                t1: t0 + (t1 - t0) * q1,
                speed: *speed,
                bend: *bend,
            },
        }
    }

    /// The shares at which the course passes through the level set
    /// `height = 0` of an affine function: in closed form along a line
    /// and an arc, by sign changes at 32 places per span of a spline and
    /// bisection.
    fn crossings(&self, height: &dyn Fn(Vec3) -> f64, tol: f64) -> Vec<f64> {
        let mut out = Vec::new();
        match self {
            Course::Line { from, to } => {
                let (a, b) = (height(*from), height(*to));
                let f = a / (a - b);
                let slack = tol / from.distance(*to);
                if f.is_finite() && f >= -slack && f <= 1.0 + slack {
                    out.push(f.clamp(0.0, 1.0));
                } else if a.abs() <= tol {
                    out.push(0.0);
                }
            }
            Course::Arc {
                center,
                x,
                y,
                radius,
                sweep,
            } => {
                // h(φ) = h(c) + r (h_x cos φ + h_y sin φ), h affine.
                let base = height(*center);
                let hx = height(*center + *x) - base;
                let hy = height(*center + *y) - base;
                let reach = hx.phypot(hy) * radius;
                let level = -base / reach;
                if level.is_finite() && level.abs() <= 1.0 {
                    let middle = hy.patan2(hx);
                    let half = level.pacos();
                    let slack = tol / radius;
                    for angle in [middle - half, middle + half] {
                        let turned = angle.rem_euclid(std::f64::consts::TAU);
                        let turned = if turned >= std::f64::consts::TAU - slack {
                            0.0
                        } else {
                            turned
                        };
                        if turned <= sweep + slack {
                            out.push((turned / sweep).clamp(0.0, 1.0));
                        }
                    }
                }
            }
            Course::Spline { curve, t0, t1, .. } => {
                let mut spans: Vec<f64> = vec![*t0];
                spans.extend(
                    curve
                        .knots()
                        .iter()
                        .copied()
                        .filter(|k| *k > *t0 && *k < *t1),
                );
                spans.push(*t1);
                spans.dedup();
                let share = |t: f64| (t - t0) / (t1 - t0);
                let at = |t: f64| height(curve.eval(t));
                if at(*t0).abs() <= tol {
                    out.push(0.0);
                }
                for (&low, &high) in spans.iter().zip(spans.iter().skip(1)) {
                    const STEPS: u32 = 32;
                    for step in 0..STEPS {
                        let mut a = low + (high - low) * f64::from(step) / f64::from(STEPS);
                        let mut b = low + (high - low) * f64::from(step + 1) / f64::from(STEPS);
                        let (ha, hb) = (at(a), at(b));
                        if !(ha.is_finite() && hb.is_finite()) || (ha > 0.0) == (hb > 0.0) {
                            continue;
                        }
                        for _ in 0..60 {
                            let middle = 0.5 * (a + b);
                            if (at(middle) > 0.0) == (ha > 0.0) {
                                a = middle;
                            } else {
                                b = middle;
                            }
                        }
                        out.push(share(0.5 * (a + b)).clamp(0.0, 1.0));
                    }
                }
            }
        }
        out
    }
}

/// One piece of a rail, over `[from, to]` of `σ`.
#[derive(Clone, Debug)]
struct Piece {
    source: u64,
    /// Which piece of its sketch curve ([`Segment::piece`]): a spline
    /// with corners is several.
    part: u32,
    course: Course,
    length: f64,
    from: f64,
    to: f64,
}

impl Piece {
    fn share(&self, sigma: f64) -> f64 {
        (sigma - self.from) / (self.to - self.from)
    }

    /// The point at `σ` (continued past the piece's ends as the curve
    /// goes on, for a joint within [`SAME_JOINT`]).
    fn at(&self, sigma: f64) -> Vec3 {
        let q = self.share(sigma);
        match &self.course {
            Course::Line { from, to } => *from + (*to - *from) * q,
            Course::Arc {
                center,
                x,
                y,
                radius,
                sweep,
            } => {
                let (s, c) = (sweep * q).psin_cos();
                *center + (*x * c + *y * s) * *radius
            }
            Course::Spline { curve, t0, t1, .. } => curve.eval(t0 + (t1 - t0) * q),
        }
    }

    /// `dA/dσ` at `σ`.
    fn velocity(&self, sigma: f64) -> Vec3 {
        let q = self.share(sigma);
        let rate = 1.0 / (self.to - self.from);
        match &self.course {
            Course::Line { from, to } => (*to - *from) * rate,
            Course::Arc {
                x,
                y,
                radius,
                sweep,
                ..
            } => {
                let (s, c) = (sweep * q).psin_cos();
                (*y * c - *x * s) * (radius * sweep * rate)
            }
            Course::Spline {
                derivative, t0, t1, ..
            } => derivative.eval(t0 + (t1 - t0) * q) * ((t1 - t0) * rate),
        }
    }

    /// A bound on `|dA/dσ|` over the piece.
    fn speed(&self) -> f64 {
        let rate = 1.0 / (self.to - self.from);
        match &self.course {
            Course::Line { from, to } => from.distance(*to) * rate,
            Course::Arc { radius, sweep, .. } => radius * sweep * rate,
            Course::Spline { t0, t1, speed, .. } => speed * (t1 - t0) * rate,
        }
    }

    /// A bound on `|d²A/dσ²|` over the piece.
    fn bend(&self) -> f64 {
        let rate = 1.0 / (self.to - self.from);
        match &self.course {
            Course::Line { .. } => 0.0,
            Course::Arc { radius, sweep, .. } => radius * (sweep * rate) * (sweep * rate),
            Course::Spline { t0, t1, bend, .. } => {
                let k = (t1 - t0) * rate;
                bend * k * k
            }
        }
    }
}

/// A rail as pieces over `σ` from 0 to 1.
#[derive(Clone, Debug)]
struct Rail {
    pieces: Vec<Piece>,
    length: f64,
    /// The most any piece moves per unit of `σ`: `|A(σ) − A(τ)|` is at
    /// most this times `|σ − τ|`.
    rate: f64,
}

impl Rail {
    fn piece_at(&self, sigma: f64) -> Option<&Piece> {
        self.pieces
            .iter()
            .find(|p| sigma <= p.to)
            .or(self.pieces.last())
    }

    fn at(&self, sigma: f64) -> Option<Vec3> {
        self.piece_at(sigma).map(|p| p.at(sigma))
    }

    fn first(&self) -> Option<u64> {
        self.pieces.first().map(|p| p.source)
    }
}

/// The derivative of a polynomial B-spline (degree, knots, control
/// points) as one a degree down, for degree 1 and up.
fn derivative_of(
    degree: usize,
    knots: &[f64],
    points: &[Vec3],
) -> Option<(usize, Vec<f64>, Vec<Vec3>)> {
    let p = degree;
    if p == 0 {
        return None;
    }
    let mut control = Vec::with_capacity(points.len().saturating_sub(1));
    for (i, (a, b)) in points.iter().zip(points.iter().skip(1)).enumerate() {
        let low = *knots.get(i + 1)?;
        let high = *knots.get(i + p + 1)?;
        let width = high - low;
        control.push(if width > 0.0 {
            (*b - *a) * (p as f64 / width)
        } else {
            Vec3::ZERO
        });
    }
    let inner = knots.get(1..knots.len().saturating_sub(1))?.to_vec();
    Some((p - 1, inner, control))
}

fn largest(points: &[Vec3]) -> f64 {
    points.iter().map(|p| p.length()).fold(0.0, f64::max)
}

/// A piece of a rail's chain lifted into space, travelled forwards or
/// backwards.
fn lift(segment: &Segment, plane: &Frame, backwards: bool) -> Result<Course, KernelError> {
    let at = |p: P2| plane.point(p[0], p[1], 0.0);
    let (start, end) = if backwards {
        (segment.end, segment.start)
    } else {
        (segment.start, segment.end)
    };
    let degenerate = |reason: &'static str| KernelError::DegenerateCurve {
        id: segment.source,
        reason,
    };
    Ok(match &segment.shape {
        Shape::Line => Course::Line {
            from: at(start),
            to: at(end),
        },
        &Shape::Arc { center, radius } => {
            let (_, sweep) = segment.angles().ok_or_else(internal)?;
            let center = at(center);
            let x = (at(start) - center)
                .normalized()
                .ok_or_else(|| degenerate("the arc has no radius"))?;
            // Counter-clockwise in the sketch when the chain runs with
            // the arc's own direction.
            let ccw = segment.reversed == backwards;
            let y = if ccw {
                plane.z().cross(x)
            } else {
                x.cross(plane.z())
            };
            if !(radius.is_finite() && radius > 0.0 && sweep.is_finite() && sweep > 0.0) {
                return Err(degenerate("the arc has no radius or no angle"));
            }
            Course::Arc {
                center,
                x,
                y,
                radius,
                sweep,
            }
        }
        Shape::Circle { .. } => {
            return Err(refuse(
                &[segment.source],
                format!(
                    "the circle e{} cannot be a rail: a rail is an open chain",
                    segment.source
                ),
            ));
        }
        Shape::Spline(spline) => {
            let forwards = segment.reversed == backwards;
            let own: Spline2 = if forwards {
                spline.clone()
            } else {
                let low = spline.knots.first().copied().unwrap_or(0.0);
                let high = spline.knots.last().copied().unwrap_or(1.0);
                Spline2 {
                    degree: spline.degree,
                    knots: spline.knots.iter().rev().map(|k| low + high - k).collect(),
                    control: spline.control.iter().rev().copied().collect(),
                }
            };
            let control: Vec<Vec3> = own.control.iter().map(|p| at(*p)).collect();
            let curve = NurbsCurve::polynomial(own.degree, own.knots.clone(), control.clone())?;
            let (d1, k1, c1) = derivative_of(own.degree, &own.knots, &control)
                .ok_or_else(|| degenerate("the spline has no derivative"))?;
            let speed = largest(&c1);
            let bend = match derivative_of(d1, &k1, &c1) {
                Some((_, _, c2)) => largest(&c2),
                None => 0.0,
            };
            let derivative = NurbsCurve::polynomial(d1, k1, c1)?;
            let (t0, t1) = curve.domain();
            Course::Spline {
                curve,
                derivative,
                t0,
                t1,
                speed,
                bend,
            }
        }
    })
}

fn course_length(course: &Course) -> f64 {
    match course {
        Course::Line { from, to } => from.distance(*to),
        Course::Arc { radius, sweep, .. } => radius * sweep,
        Course::Spline {
            derivative, t0, t1, ..
        } => {
            let rule = Rule::new(8);
            let mut knots: Vec<f64> = derivative
                .knots()
                .iter()
                .copied()
                .filter(|k| *k > *t0 && *k < *t1)
                .collect();
            knots.dedup();
            let mut total = 0.0;
            let mut low = *t0;
            for high in knots.into_iter().chain(std::iter::once(*t1)) {
                total += rule.integrate(low, high, 4, |t| derivative.eval(t).length());
                low = high;
            }
            total
        }
    }
}

/// The segments of a rail's sketch in the order they are joined, and
/// whether they close: one chain, or one circle as two half turns
/// (pieces 0 and 1, from its point at angle 0 of the sketch).
fn segments_of(
    curves: &[SketchCurve],
    tolerance: &Tolerance,
) -> Result<(Vec<Segment>, bool), KernelError> {
    if let [only] = curves
        && let Curve2::Circle { center, radius } = only.curve
    {
        if !(radius.is_finite() && radius > tolerance.linear) {
            return Err(KernelError::DegenerateCurve {
                id: only.id,
                reason: "the circle has no radius",
            });
        }
        let east = [center[0] + radius, center[1]];
        let west = [center[0] - radius, center[1]];
        let half = |piece: u32, start: [f64; 2], end: [f64; 2]| Segment {
            source: only.id,
            piece,
            reversed: false,
            start,
            end,
            shape: Shape::Arc { center, radius },
        };
        return Ok((vec![half(0, east, west), half(1, west, east)], true));
    }
    let (chain, closed) = chain(curves, tolerance)?;
    Ok((chain.segments, closed))
}

/// Pieces of one length each laid end to end over `σ` from 0 to 1, and
/// the rail they make.
fn laid(pieces: Vec<(u64, u32, Course, f64)>) -> Result<Rail, KernelError> {
    let total: f64 = pieces.iter().map(|p| p.3).sum();
    let count = pieces.len();
    let mut run = 0.0;
    let mut out: Vec<Piece> = Vec::with_capacity(count);
    for (k, (source, part, course, length)) in pieces.into_iter().enumerate() {
        let from = run / total;
        run += length;
        let to = if k + 1 == count { 1.0 } else { run / total };
        if to <= from {
            return Err(internal());
        }
        out.push(Piece {
            source,
            part,
            course,
            length,
            from,
            to,
        });
    }
    let rate = out.iter().map(Piece::speed).fold(0.0, f64::max);
    if !(total.is_finite() && rate.is_finite()) {
        return Err(internal());
    }
    Ok(Rail {
        pieces: out,
        length: total,
        rate,
    })
}

/// A rail as drawn: an open one laid on `σ` from where it starts on the
/// profile, or a closed one, laid on `σ` once it is known where the body
/// starts round it.
enum Drawn {
    Open(Rail),
    Closed(Round),
}

/// A closed rail: its pieces in the direction it is travelled, as
/// fractions of its length from the start of its first piece, and where
/// it passes through the profile's outline.
struct Round {
    rail: Rail,
    /// The fraction at which it passes through the outline.
    at: f64,
}

impl Round {
    /// The fractions of its joints, counted from where it passes through
    /// the outline, in `[0, 1)`.
    fn joints_after(&self) -> impl Iterator<Item = f64> + '_ {
        self.rail
            .pieces
            .iter()
            .map(|p| (p.from - self.at).rem_euclid(1.0))
            .map(|f| if f >= 1.0 - SAME_JOINT { 0.0 } else { f })
    }

    /// The rail laid on `σ` from the fraction `g` of the ring: the piece
    /// `g` falls in is cut in two there (each part keeping the piece's
    /// parameter, and the share of `σ` it had), unless `g` is at one of
    /// its ends within [`SAME_JOINT`].
    fn opened_at(&self, g: f64) -> Result<Rail, KernelError> {
        let pieces = &self.rail.pieces;
        let count = pieces.len();
        let k = pieces
            .iter()
            .position(|p| g <= p.to)
            .unwrap_or(count.saturating_sub(1));
        let here = pieces.get(k).ok_or_else(internal)?;
        let whole = |p: &Piece| (p.source, p.part, p.course.clone(), p.length);
        let mut out = Vec::with_capacity(count + 1);
        let (first, split) = if g - here.from <= SAME_JOINT {
            (k, None)
        } else if here.to - g <= SAME_JOINT {
            ((k + 1) % count, None)
        } else {
            (k + 1, Some(here.share(g)))
        };
        if let Some(q) = split {
            out.push((
                here.source,
                here.part,
                here.course.between(q, 1.0),
                here.length * (1.0 - q),
            ));
        }
        for i in 0..count {
            let index = (first + i) % count;
            if split.is_some() && index == k {
                break;
            }
            out.push(whole(pieces.get(index).ok_or_else(internal)?));
        }
        if let Some(q) = split {
            out.push((
                here.source,
                here.part,
                here.course.between(0.0, q),
                here.length * q,
            ));
        }
        laid(out)
    }
}

/// The rail through `curves` in `plane`. An open chain starts at the end
/// nearer to the profile (`near`: how far a point is off its outline). A
/// closed one (a ring, or one circle) is found where it passes through
/// the profile's plane (`height`, signed) on its outline, and travelled
/// the way the profile's normal `normal` points.
fn rail_of(
    curves: &[SketchCurve],
    plane: &Frame,
    tolerance: &Tolerance,
    near: &dyn Fn(Vec3) -> f64,
    height: &dyn Fn(Vec3) -> f64,
    normal: Vec3,
) -> Result<Drawn, KernelError> {
    let (segments, closed) = segments_of(curves, tolerance)?;
    let first = segments.first().ok_or(KernelError::NoProfile)?;
    let last = segments.last().ok_or(KernelError::NoProfile)?;
    let point = |p: P2| plane.point(p[0], p[1], 0.0);
    let lifted = |segment: &Segment, backwards: bool| -> Result<_, KernelError> {
        let course = lift(segment, plane, backwards)?;
        let length = course_length(&course);
        if !(length.is_finite() && length > tolerance.linear) {
            return Err(KernelError::DegenerateCurve {
                id: segment.source,
                reason: "the curve of a rail is shorter than the tolerance",
            });
        }
        Ok((segment.source, segment.piece, course, length))
    };
    if !closed {
        let backwards = near(point(last.end)) < near(point(first.start));
        let ordered: Vec<&Segment> = if backwards {
            segments.iter().rev().collect()
        } else {
            segments.iter().collect()
        };
        let pieces = ordered
            .into_iter()
            .map(|segment| lifted(segment, backwards))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Drawn::Open(laid(pieces)?));
    }

    // A ring: where it passes through the outline, nearest to it.
    let tol = tolerance.linear;
    let forwards = segments
        .iter()
        .map(|segment| lifted(segment, false))
        .collect::<Result<Vec<_>, _>>()?;
    let mut best: Option<(usize, f64, Vec3, f64)> = None;
    for (index, (_, _, course, _)) in forwards.iter().enumerate() {
        for q in course.crossings(height, tol) {
            let at = course.at(q);
            let off = near(at);
            if off.is_finite() && best.is_none_or(|(_, _, _, least)| off < least) {
                best = Some((index, q, at, off));
            }
        }
    }
    let sources: Vec<u64> = segments
        .iter()
        .map(|s| s.source)
        .take(MAX_REPORTED)
        .collect();
    let Some((index, q, at, off)) = best else {
        return Err(refuse(
            &sources,
            format!(
                "the closed rail through e{} does not pass through the section's plane; a closed rail has to pass through the outline of the section",
                first.source
            ),
        ));
    };
    if off > tol * 7.0 / 8.0 {
        return Err(refuse(
            &sources,
            format!(
                "the closed rail through e{} passes through the section's plane at {}, which is {off} from the section's outline; a closed rail has to pass through the outline of the section",
                first.source,
                point_text(at)
            ),
        ));
    }
    let (_, _, course, _) = forwards.get(index).ok_or_else(internal)?;
    let tangent = course.tangent(q);
    let rise = tangent.dot(normal);
    let along = tangent.length();
    if !(rise.is_finite() && along.is_finite() && rise.abs() > 1e-9 * along) {
        return Err(refuse(
            &sources,
            format!(
                "the closed rail through e{} runs along the section's plane where it passes through it at {}; a rail has to cross the section's plane",
                first.source,
                point_text(at)
            ),
        ));
    }
    // Travelled the way the profile's normal points.
    let backwards = rise < 0.0;
    let (pieces, index, q) = if backwards {
        let pieces = segments
            .iter()
            .rev()
            .map(|segment| lifted(segment, true))
            .collect::<Result<Vec<_>, _>>()?;
        (pieces, segments.len() - 1 - index, 1.0 - q)
    } else {
        (forwards, index, q)
    };
    let rail = laid(pieces)?;
    let piece = rail.pieces.get(index).ok_or_else(internal)?;
    let fraction = piece.from + (piece.to - piece.from) * q;
    Ok(Drawn::Closed(Round { at: fraction, rail }))
}

/// Where the profile is at a station: the chord's length, its direction
/// and the up direction made square to it, with the sine of the angle
/// between the chord and the up direction.
#[derive(Clone, Copy, Debug)]
struct Placed {
    a: Vec3,
    chord: Vec3,
    r: f64,
    e: Vec3,
    w: Vec3,
    g: f64,
}

fn placed(a: Vec3, b: Vec3, up: Vec3) -> Option<Placed> {
    let chord = b - a;
    let r = chord.length();
    let e = chord.normalized()?;
    let across = up - e * up.dot(e);
    let g = across.length();
    let w = across.normalized()?;
    (r.is_finite() && g.is_finite() && r > 0.0 && g > 0.0).then_some(Placed {
        a,
        chord,
        r,
        e,
        w,
        g,
    })
}

/// What every station needs to know.
struct Setting {
    up: Vec3,
    /// What the up direction is, for messages.
    up_text: String,
    /// The up direction was given (not the first rail's sketch normal).
    up_given: bool,
    /// A loop of the profile has an arc or a circle: a slide at a slant
    /// would carry it into an elliptic cylinder.
    round: bool,
    /// The distance from `a` to `b` in the profile, which the chord's
    /// length is measured against.
    d_ref: f64,
    scaling: Scaling,
    /// The largest `|x|` and `|y|` of the profile's control points.
    x: f64,
    y: f64,
    /// The profile's box: the least and largest `x` and `y` of its
    /// control points (the crossing speed, affine in the point, is least
    /// at a corner of it).
    xs: (f64, f64),
    ys: (f64, f64),
    tol: f64,
    /// The rails are at least this far apart anywhere.
    apart: f64,
}

impl Setting {
    fn station(&self, p: &Placed) -> Station {
        let s = p.r / self.d_ref;
        Station {
            origin: p.a,
            u: p.chord * (1.0 / self.d_ref),
            b: match self.scaling {
                Scaling::Uniform => p.w * s,
                Scaling::Width => p.w,
            },
            square: self.scaling == Scaling::Uniform,
        }
    }
}

/// A stretch of `σ` along which each rail runs along one piece.
struct Leg<'a> {
    from: f64,
    to: f64,
    a: &'a Piece,
    b: &'a Piece,
}

impl Leg<'_> {
    fn placed(&self, sigma: f64, up: Vec3) -> Option<Placed> {
        placed(self.a.at(sigma), self.b.at(sigma), up)
    }
}

/// Bounds that hold all along a leg.
struct Bounds {
    /// Second derivatives of the true `O`, `U`, `B`.
    o2: f64,
    u2: f64,
    b2: f64,
    /// Lower bounds of the chord's length and of the sine between the
    /// chord and up, an upper bound of the scale.
    r_lo: f64,
    g_lo: f64,
}

/// `d/dσ` of the placement at `σ`: of `O`, `U`, `B` and of the forward
/// direction `e × w`... returned as (`O'`, `U'`, `B'`).
fn rates(leg: &Leg, sigma: f64, p: &Placed, set: &Setting) -> (Vec3, Vec3, Vec3) {
    let va = leg.a.velocity(sigma);
    let dc = leg.b.velocity(sigma) - va;
    let de = (dc - p.e * p.e.dot(dc)) * (1.0 / p.r);
    let up = set.up;
    let dg = -(p.e * up.dot(de)) - de * up.dot(p.e);
    let dw = (dg - p.w * p.w.dot(dg)) * (1.0 / p.g);
    let du = dc * (1.0 / set.d_ref);
    let db = match set.scaling {
        Scaling::Uniform => {
            let s = p.r / set.d_ref;
            let ds = p.e.dot(dc) / set.d_ref;
            dw * s + p.w * ds
        }
        Scaling::Width => dw,
    };
    (va, du, db)
}

/// The least speed across its station's plane of the corners of a box
/// round the profile at `σ`: the profile's own box when `sharp`, else the
/// box of its reach both ways, `±x` by `±y` (the cruder first proof).
/// The speed is affine in the point, so it is least at a corner.
fn crossing_speed(leg: &Leg, sigma: f64, p: &Placed, set: &Setting, sharp: bool) -> f64 {
    let (vo, vu, vb) = rates(leg, sigma, p, set);
    let forward = p.e.cross(p.w);
    let (xs, ys) = if sharp {
        (set.xs, set.ys)
    } else {
        ((-set.x, set.x), (-set.y, set.y))
    };
    let mut least = f64::INFINITY;
    for (x, y) in [(xs.1, ys.1), (xs.1, ys.0), (xs.0, ys.1), (xs.0, ys.0)] {
        least = least.min((vo + vu * x + vb * y).dot(forward));
    }
    least
}

/// Proves along the leg that the chord keeps a length, stays off the up
/// direction and the profile crosses its station's plane, and returns the
/// bounds the proof of the deviation needs.
///
/// Two ways, tried in this order by the caller. The cruder (`sharp`
/// false) bounds the chord's speed by the sum of the rails' and the
/// crossing speed at the corners of the box of the profile's reach both
/// ways; it is what format 66 built with, so what it proves is built as
/// it was, and where it proves nothing (`None`: a corner of that box that
/// does not cross, or no grid fine enough) the sharper one is tried. The
/// sharper bounds the chord's speed by its largest value on the grid
/// plus `|c''| h / 2` and takes the profile's own box; what it cannot
/// prove is refused.
fn leg_bounds(
    leg: &Leg,
    set: &Setting,
    spent: &mut usize,
    sharp: bool,
) -> Result<Option<Bounds>, KernelError> {
    let (ra, rb) = (leg.a.speed(), leg.b.speed());
    let (ka, kb) = (leg.a.bend(), leg.b.bend());
    let kc = ka + kb;
    let width = leg.to - leg.from;
    let sources = [leg.a.source, leg.b.source];
    let mut n = 64_usize;
    loop {
        spend(spent, n + 1, leg)?;
        let h = width / n as f64;
        let mut r_min = f64::INFINITY;
        let mut r_max: f64 = 0.0;
        let mut g_min = f64::INFINITY;
        let mut cross_min = f64::INFINITY;
        let mut worst_at = leg.from;
        let mut chord_speed: f64 = 0.0;
        for j in 0..=n {
            let sigma = if j == n {
                leg.to
            } else {
                leg.from + h * j as f64
            };
            let Some(p) = leg.placed(sigma, set.up) else {
                let (a, b) = (leg.a.at(sigma), leg.b.at(sigma));
                return Err(refuse(
                    &sources,
                    format!(
                        "the chord between the rails at {} and {} runs along {}; the section cannot be stood up there",
                        point_text(a),
                        point_text(b),
                        set.up_text
                    ),
                ));
            };
            r_min = r_min.min(p.r);
            r_max = r_max.max(p.r);
            if p.g < g_min {
                g_min = p.g;
                worst_at = sigma;
            }
            if sharp {
                let dc = leg.b.velocity(sigma) - leg.a.velocity(sigma);
                chord_speed = chord_speed.max(dc.length());
            }
            let speed = crossing_speed(leg, sigma, &p, set, sharp);
            if !sharp && (speed.is_nan() || speed <= 0.0) {
                return Ok(None);
            }
            if speed.is_nan() || speed <= 0.0 {
                return Err(refuse(
                    &sources,
                    format!(
                        "the rails carry the section along its own plane near {} (on e{} and e{}): the sweep would fold over itself{}",
                        point_text(p.a),
                        leg.a.source,
                        leg.b.source,
                        if set.up_given {
                            format!(
                                " (the section stands in the plane of the chord and {}: an up direction along the rails lays it into their course)",
                                set.up_text
                            )
                        } else {
                            String::new()
                        }
                    ),
                ));
            }
            cross_min = cross_min.min(speed);
        }
        // |c'| at most: the rails' speeds added, or (sharp) its largest
        // value on the grid and what |c''| allows between.
        let lc = if sharp {
            (chord_speed + kc * h / 2.0).min(ra + rb)
        } else {
            ra + rb
        };
        let r_lo = (r_min - lc * h / 2.0).max(set.apart);
        let e1 = lc / r_lo;
        let g_lo = g_min - 2.0 * e1 * h / 2.0;
        let s_hi = (r_max + lc * h / 2.0) / set.d_ref;
        let bounds = if g_lo > 0.0 {
            let e2 = kc / r_lo + 3.0 * lc * lc / (r_lo * r_lo);
            let g1 = 2.0 * e1;
            let g2 = 2.0 * e2 + 2.0 * e1 * e1;
            let w1 = g1 / g_lo;
            let w2 = g2 / g_lo + 3.0 * g1 * g1 / (g_lo * g_lo);
            let s1 = lc / set.d_ref;
            let s2 = (kc + lc * lc / r_lo) / set.d_ref;
            let (b1, b2) = match set.scaling {
                Scaling::Uniform => (w1 * s_hi + s1, w2 * s_hi + 2.0 * w1 * s1 + s2),
                Scaling::Width => (w1, w2),
            };
            // The crossing speed (V·f) is Lipschitz in σ with at most
            // |V'| + |V| |f'|.
            let v1 = ra + set.x * lc / set.d_ref + set.y * b1;
            let v2 = ka + set.x * kc / set.d_ref + set.y * b2;
            let lip = v2 + v1 * (e1 + w1);
            let proven = cross_min - lip * h / 2.0 > 0.0;
            proven.then_some(Bounds {
                o2: ka,
                u2: kc / set.d_ref,
                b2,
                r_lo,
                g_lo,
            })
        } else {
            None
        };
        if let Some(bounds) = bounds
            && [bounds.o2, bounds.u2, bounds.b2, bounds.r_lo, bounds.g_lo]
                .iter()
                .all(|v| v.is_finite())
        {
            return Ok(Some(bounds));
        }
        if n >= MAX_GRID && !sharp {
            return Ok(None);
        }
        if n >= MAX_GRID {
            let at = leg.a.at(worst_at);
            return Err(refuse(
                &sources,
                format!(
                    "near {} (on e{} and e{}) the chord between the rails comes so near the up direction, or the section so near to running along its own plane, that the sweep cannot be shown not to fold; move the rails apart or change their course there",
                    point_text(at),
                    leg.a.source,
                    leg.b.source
                ),
            ));
        }
        n = n.saturating_mul(4);
    }
}

/// The point at `t` in `[0, 1]` of the Bézier curve on `points`.
fn bezier(points: &[Vec3], t: f64) -> Vec3 {
    let mut d: Vec<Vec3> = points.to_vec();
    let n = d.len();
    for r in 1..n {
        for i in 0..n - r {
            if let (Some(&a), Some(&b)) = (d.get(i), d.get(i + 1))
                && let Some(slot) = d.get_mut(i)
            {
                *slot = a + (b - a) * t;
            }
        }
    }
    d.first().copied().unwrap_or(Vec3::ZERO)
}

/// The most the second derivative of the Bézier curve on `points` (over
/// the unit interval) can be.
fn second_bound(points: &[Vec3]) -> f64 {
    let p = points.len().saturating_sub(1);
    if p < 2 {
        return 0.0;
    }
    let factor = (p * (p - 1)) as f64;
    points
        .windows(3)
        .map(|w| match w {
            [a, b, c] => (*a - *b * 2.0 + *c).length() * factor,
            _ => 0.0,
        })
        .fold(0.0, f64::max)
}

/// The stations of a leg at a plan, the band through them and the proven
/// bound on how far the faces lie from the true sweep.
struct Followed {
    stations: Vec<Station>,
    band: Band,
    deviation: f64,
}

/// What following a leg at one plan gave.
enum Outcome {
    /// Plainly too coarse: a quick look found the faces too far off.
    Coarse,
    /// Its proof would take more than [`MAX_SAMPLES`] samples.
    Costly,
    Made(Followed),
}

/// Takes `count` samples from what the sweep may spend in all, or
/// refuses with the place.
fn spend(spent: &mut usize, count: usize, leg: &Leg) -> Result<(), KernelError> {
    *spent = spent.saturating_add(count);
    if *spent > MAX_TOTAL_SAMPLES {
        return Err(refuse(
            &[leg.a.source, leg.b.source],
            format!(
                "the sweep cannot be shown to lie within the tolerance in {MAX_TOTAL_SAMPLES} samples in all; it ran out along e{} and e{} near {}; give the document a coarser tolerance or the rails fewer curves",
                leg.a.source,
                leg.b.source,
                point_text(leg.a.at(leg.from))
            ),
        ));
    }
    Ok(())
}

/// `first` is the station the leg starts from (the one the leg before
/// ended with, to the bit); `last`, when given, the one it has to end
/// with (round closed rails, the start's), whose gap from the exact one
/// is in the samples like any other.
#[allow(clippy::too_many_arguments)]
fn follow(
    leg: &Leg,
    first: &Station,
    last: Option<&Station>,
    set: &Setting,
    bounds: &Bounds,
    (parts, degree): (usize, usize),
    spent: &mut usize,
) -> Result<Outcome, KernelError> {
    let within = stations_of(degree);
    let width = leg.to - leg.from;
    let part_width = width / parts as f64;
    let sigma_at = |v: f64| leg.from + width * v / parts as f64;
    let exact = |sigma: f64| -> Result<Station, KernelError> {
        let p = leg.placed(sigma, set.up).ok_or_else(internal)?;
        Ok(set.station(&p))
    };
    let mut stations = Vec::with_capacity(parts * degree + 1);
    for part in 0..parts {
        for (step, share) in within.iter().enumerate() {
            if step == 0 && part > 0 {
                continue;
            }
            stations.push(if part == 0 && step == 0 {
                *first
            } else if part + 1 == parts && step == degree {
                match last {
                    Some(last) => *last,
                    None => exact(leg.to)?,
                }
            } else {
                exact(sigma_at(part as f64 + share))?
            });
        }
    }
    let band = part_band(parts, degree, &within)?;
    let o_ctrl = band.points(&stations.iter().map(|s| s.origin).collect::<Vec<_>>());
    let u_ctrl = band.points(&stations.iter().map(|s| s.u).collect::<Vec<_>>());
    let b_ctrl = band.points(&stations.iter().map(|s| s.b).collect::<Vec<_>>());
    let gap = |interp: (Vec3, Vec3, Vec3), truth: &Station| {
        (
            interp.0.distance(truth.origin),
            interp.1.distance(truth.u),
            interp.2.distance(truth.b),
        )
    };
    let span = |ctrl: &[Vec3], part: usize| -> Result<Vec<Vec3>, KernelError> {
        ctrl.get(part * degree..=part * degree + degree)
            .map(<[Vec3]>::to_vec)
            .ok_or_else(internal)
    };
    // A quick look first: a plan that is plainly too coarse is not worth
    // a proof.
    for part in 0..parts {
        let (o, u, b) = (
            span(&o_ctrl, part)?,
            span(&u_ctrl, part)?,
            span(&b_ctrl, part)?,
        );
        for step in 0..degree {
            let (Some(&s0), Some(&s1)) = (within.get(step), within.get(step + 1)) else {
                return Err(internal());
            };
            for share in [0.25, 0.5, 0.75] {
                let t = s0 + (s1 - s0) * share;
                let truth = exact(sigma_at(part as f64 + t))?;
                let (go, gu, gb) = gap((bezier(&o, t), bezier(&u, t), bezier(&b, t)), &truth);
                if go + set.x * gu + set.y * gb > set.tol / 2.0 {
                    return Ok(Outcome::Coarse);
                }
            }
        }
    }
    // The proof.
    let mut deviation: f64 = 0.0;
    let mut samples = 0_usize;
    let mut size: f64 = 0.0;
    for part in 0..parts {
        let (o, u, b) = (
            span(&o_ctrl, part)?,
            span(&u_ctrl, part)?,
            span(&b_ctrl, part)?,
        );
        let squared = part_width * part_width;
        let m = (second_bound(&o) / squared + bounds.o2)
            + set.x * (second_bound(&u) / squared + bounds.u2)
            + set.y * (second_bound(&b) / squared + bounds.b2);
        if !m.is_finite() {
            return Err(internal());
        }
        // M h² / 8 at most an eighth of the tolerance.
        let h_most = (set.tol / m).sqrt();
        let count = if h_most.is_finite() && h_most > 0.0 {
            let wanted = (part_width / h_most).ceil();
            if !(wanted.is_finite() && wanted < MAX_SAMPLES as f64) {
                return Ok(Outcome::Costly);
            }
            (wanted as usize).max(1)
        } else {
            1
        };
        samples = samples.saturating_add(count);
        if samples > MAX_SAMPLES {
            return Ok(Outcome::Costly);
        }
        spend(spent, count, leg)?;
        let h = part_width / count as f64;
        let mut before: Option<(f64, f64, f64)> = None;
        for j in 0..=count {
            let t = j as f64 / count as f64;
            let truth = exact(sigma_at(part as f64 + t))?;
            let interp = (bezier(&o, t), bezier(&u, t), bezier(&b, t));
            size =
                size.max(interp.0.length() + set.x * interp.1.length() + set.y * interp.2.length());
            let now = gap(interp, &truth);
            if let Some(was) = before {
                let here = was.0.max(now.0) + set.x * was.1.max(now.1) + set.y * was.2.max(now.2);
                deviation = deviation.max(here + m * h * h / 8.0);
            }
            before = Some(now);
        }
    }
    // The first station is the one the leg before ended with, to the bit;
    // the gap from the exact one was in the samples at the start. The
    // rounding of the sums of the band and of the surface's own
    // evaluation (a few dozen operations, each off by half a unit in the
    // last place of the size): 256 ε times the size, which [`FINEST`]
    // keeps at most a sixteenth of the tolerance.
    deviation += 256.0 * f64::EPSILON * size;
    if !deviation.is_finite() {
        return Err(internal());
    }
    Ok(Outcome::Made(Followed {
        stations,
        band,
        deviation,
    }))
}

/// The leg as a translation, when it is one: both rails parallel lines
/// whose ends differ by the same vector, within a tiny part of the
/// tolerance, and the profile of its own size. The deviation bound of
/// the translation from the true sweep is returned with it.
fn translation(leg: &Leg, start: &Station, set: &Setting, bounds: &Bounds) -> Option<(Vec3, f64)> {
    let (Course::Line { .. }, Course::Line { .. }) = (&leg.a.course, &leg.b.course) else {
        return None;
    };
    if !start.square || (start.b.length() - 1.0).abs() > 1e-12 {
        return None;
    }
    let da = leg.a.at(leg.to) - leg.a.at(leg.from);
    let db = leg.b.at(leg.to) - leg.b.at(leg.from);
    // A profile with an arc or a circle slides only square to its
    // plane: at a slant a circle sweeps an elliptic cylinder, not the
    // round one a slide makes, so that leg is followed instead. Lines
    // (planes) and splines (ruled between the two stations) slide
    // exactly at any slant.
    if set.round {
        let square_on = da.normalized()?.dot(start.forward().ok()?).abs();
        if !(square_on.is_finite() && 1.0 - square_on <= 1e-12) {
            return None;
        }
    }
    let off = (db - da).length();
    // The chord moves by at most `off`; the placement follows it.
    let shift = off / set.d_ref;
    let turn = 2.0 * off / (bounds.r_lo * bounds.g_lo);
    // The station the leg starts from is the one the leg before ended
    // with, which may be off the exact one by a little.
    let exact = set.station(&leg.placed(leg.from, set.up)?);
    let inherited = start.origin.distance(exact.origin)
        + set.x * start.u.distance(exact.u)
        + set.y * start.b.distance(exact.b);
    let bound = inherited + set.x * shift + set.y * (turn * (1.0 + shift) + shift);
    (bound.is_finite() && bound <= set.tol / 64.0).then_some((da, bound))
}

/// Sweeps `profile`, which lies in `profile_plane`, along two rails: the
/// curves `rail_a` of a sketch in `plane_a` and `rail_b` of one in
/// `plane_b`.
///
/// See the module documentation for how the profile is placed, what the
/// faces are, what is proven and what is refused. The body is one solid
/// shell that has passed `craft3d_topo::check` with `tolerance`.
#[allow(clippy::too_many_arguments)]
pub fn sweep_two_rails(
    profile: &Profile,
    profile_plane: &Frame,
    rail_a: &[SketchCurve],
    plane_a: &Frame,
    rail_b: &[SketchCurve],
    plane_b: &Frame,
    scaling: Scaling,
    tolerance: &Tolerance,
) -> Result<Swept, KernelError> {
    sweep_two_rails_up(
        profile,
        profile_plane,
        rail_a,
        plane_a,
        rail_b,
        plane_b,
        scaling,
        None,
        tolerance,
    )
}

/// [`sweep_two_rails`] with the up direction given: the section stands in
/// the plane of the chord and `up` at every station (any length, either
/// way: it is turned round when the rails leave the profile against it).
/// `None` is the normal of rail A's sketch.
#[allow(clippy::too_many_arguments)]
pub fn sweep_two_rails_up(
    profile: &Profile,
    profile_plane: &Frame,
    rail_a: &[SketchCurve],
    plane_a: &Frame,
    rail_b: &[SketchCurve],
    plane_b: &Frame,
    scaling: Scaling,
    up: Option<Vec3>,
    tolerance: &Tolerance,
) -> Result<Swept, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if profile.outer.segments.is_empty() {
        return Err(KernelError::NoProfile);
    }
    each_curve_once(std::slice::from_ref(profile))?;

    // The rails start where they touch the profile's outline.
    let outline: Vec<_> = profile
        .outer
        .segments
        .iter()
        .flat_map(Segment::parts)
        .collect();
    let off_outline = |p: Vec3| -> f64 {
        let (x, y, z) = profile_plane.local(p);
        z.abs().max(distance_to(&outline, [x, y], tol))
    };
    let height = |p: Vec3| profile_plane.local(p).2;
    let normal = profile_plane.z();
    let drawn_a = rail_of(rail_a, plane_a, tolerance, &off_outline, &height, normal)?;
    let drawn_b = rail_of(rail_b, plane_b, tolerance, &off_outline, &height, normal)?;
    // An open rail starts on the profile; round two closed ones the body
    // starts at the first joint of either after the profile (so that the
    // piece the profile is on is not cut there), and the profile is at
    // `σ_p`.
    let (rail_a, rail_b, closed, sigma_p) = match (drawn_a, drawn_b) {
        (Drawn::Open(a), Drawn::Open(b)) => (a, b, false, 0.0),
        (Drawn::Closed(a), Drawn::Closed(b)) => {
            let c = a
                .joints_after()
                .chain(b.joints_after())
                .fold(1.0_f64, f64::min);
            if !(c.is_finite() && (0.0..1.0).contains(&c)) {
                return Err(internal());
            }
            let open = |round: &Round| round.opened_at((round.at + c).rem_euclid(1.0));
            let sigma_p = if c > 0.0 { 1.0 - c } else { 0.0 };
            (open(&a)?, open(&b)?, true, sigma_p)
        }
        (Drawn::Open(open), Drawn::Closed(_)) | (Drawn::Closed(_), Drawn::Open(open)) => {
            let ids: Vec<u64> = open.pieces.iter().map(|p| p.source).take(2).collect();
            return Err(refuse(
                &ids,
                "one rail closes into a ring and the other does not; both rails are open chains, or both close",
            ));
        }
    };
    let (Some(a0), Some(b0)) = (rail_a.at(sigma_p), rail_b.at(sigma_p)) else {
        return Err(internal());
    };
    let (Some(first_a), Some(first_b)) = (rail_a.first(), rail_b.first()) else {
        return Err(internal());
    };
    for (start, source) in [(a0, first_a), (b0, first_b)] {
        let off = off_outline(start);
        if off.is_nan() || off > tol * 7.0 / 8.0 {
            return Err(refuse(
                &[source],
                format!(
                    "the rail through e{source} starts at {}, which is {off} from the section's outline; each rail has to start on the outline of the section",
                    point_text(start)
                ),
            ));
        }
    }
    let local = |p: Vec3| {
        let (x, y, _) = profile_plane.local(p);
        [x, y]
    };
    let (pa, pb) = (local(a0), local(b0));
    let along = [pb[0] - pa[0], pb[1] - pa[1]];
    let d_ref = along[0].phypot(along[1]);
    if !(d_ref.is_finite() && d_ref > tol) {
        return Err(refuse(
            &[first_a, first_b],
            format!(
                "the rails start at one point ({}): the section between them would have no width",
                point_text(a0)
            ),
        ));
    }

    // The rails never meet: the distance between any two of their places
    // is bounded from below by a branch and bound.
    let apart = rails_apart(&rail_a, &rail_b, tol)?;

    // The up direction: the one given, or the normal of rail A's sketch;
    // turned round when the rails leave the profile against it.
    let up_given = up.is_some();
    let (mut up, up_text) = match up {
        None => (
            plane_a.z(),
            "the up direction (the normal of the first rail's sketch)".to_string(),
        ),
        Some(given) => {
            let unit = given
                .normalized()
                .filter(|_| given.x.is_finite() && given.y.is_finite() && given.z.is_finite());
            let Some(unit) = unit else {
                return Err(refuse(
                    &[first_a, first_b],
                    format!(
                        "the up direction {} has no direction; give a direction that is not zero",
                        direction_text(given)
                    ),
                ));
            };
            (
                unit,
                format!(
                    "the up direction ({:.6}, {:.6}, {:.6}) given",
                    unit.x, unit.y, unit.z
                ),
            )
        }
    };
    let e0 = (b0 - a0).normalized().ok_or_else(internal)?;
    let leaving = |rail: &Rail| {
        rail.piece_at(sigma_p)
            .map(|p| p.velocity(sigma_p))
            .and_then(Vec3::normalized)
            .unwrap_or(Vec3::ZERO)
    };
    let heading = leaving(&rail_a) + leaving(&rail_b);
    let first = placed(a0, b0, up).ok_or_else(|| {
        refuse(
            &[first_a, first_b],
            format!(
                "the rails start at {} and {}, along {up_text}; the section cannot be stood up between them",
                point_text(a0),
                point_text(b0)
            ),
        )
    })?;
    if heading.dot(e0.cross(first.w)) < 0.0 {
        up = -up;
    }
    let first = placed(a0, b0, up).ok_or_else(internal)?;

    // The profile's axes: `a` at the origin, `b` on the x axis, y the way
    // the up direction goes in its plane.
    let ex = [along[0] / d_ref, along[1] / d_ref];
    let ny = [-ex[1], ex[0]];
    let lifted_y = profile_plane.vector(ny[0], ny[1], 0.0);
    let facing = lifted_y.dot(first.w);
    if facing.is_nan() || facing.abs() < 0.5 {
        return Err(refuse(
            &[first_a, first_b],
            format!(
                "the section's plane does not hold {up_text} where the rails start on it; draw the section on a plane through both rails' starts that holds that direction"
            ),
        ));
    }
    let mirrored = facing < 0.0;
    let sy = if mirrored { -1.0 } else { 1.0 };
    let map = Map2 {
        origin: [
            -(ex[0] * pa[0] + ex[1] * pa[1]),
            -sy * (ny[0] * pa[0] + ny[1] * pa[1]),
        ],
        x: [ex[0], sy * ny[0]],
        y: [ex[1], sy * ny[1]],
        mirrored,
    };
    let mut loops = profile_loops(profile, &map, tol)?;
    for elements in &mut loops {
        // No face goes all the way round.
        *elements = elements.iter().flat_map(quarters).collect();
        for element in elements.iter_mut() {
            element.flat = false;
        }
    }
    let (mut xs, mut ys) = (
        (f64::INFINITY, f64::NEG_INFINITY),
        (f64::INFINITY, f64::NEG_INFINITY),
    );
    let mut grow = |p: P2| {
        xs = (xs.0.min(p[0]), xs.1.max(p[0]));
        ys = (ys.0.min(p[1]), ys.1.max(p[1]));
    };
    for element in loops.iter().flatten() {
        grow(element.start);
        grow(element.end);
        match &element.form {
            Form::Line => {}
            &Form::Arc { center, radius, .. } | &Form::Circle { center, radius, .. } => {
                grow([center[0] - radius, center[1] - radius]);
                grow([center[0] + radius, center[1] + radius]);
            }
            Form::Spline { own, .. } => own.control.iter().for_each(|p| grow(*p)),
        }
    }
    let x_reach = xs.0.abs().max(xs.1.abs());
    let y_reach = ys.0.abs().max(ys.1.abs());
    if !(x_reach.is_finite() && y_reach.is_finite()) {
        return Err(internal());
    }
    let set = Setting {
        up,
        up_text: up_text.clone(),
        up_given,
        round: loops
            .iter()
            .flatten()
            .any(|e| matches!(e.form, Form::Arc { .. } | Form::Circle { .. })),
        d_ref,
        scaling,
        x: x_reach,
        y: y_reach,
        xs,
        ys,
        tol,
        apart,
    };
    let home = set.station(&first);

    // How far the sweep reaches from the origin, at most: every point of
    // a rail lies in the hull of its pieces' ends, centres and radii and
    // control points; the chord is at most twice that, so `U` and a
    // uniform `B` are at most that over `|b − a|`. Below [`FINEST`] units
    // in the last place of it the rounding is not bounded by the proof.
    let reach = rail_a
        .pieces
        .iter()
        .chain(&rail_b.pieces)
        .map(|p| match &p.course {
            Course::Line { from, to } => from.length().max(to.length()),
            Course::Arc { center, radius, .. } => center.length() + radius,
            Course::Spline { curve, .. } => largest(curve.control_points()),
        })
        .fold(0.0, f64::max);
    let across = 2.0 * reach / d_ref;
    let size = reach + x_reach * across + y_reach * across.max(1.0);
    if !(size.is_finite() && tol >= FINEST * f64::EPSILON * size) {
        return Err(refuse(
            &[first_a, first_b],
            format!(
                "the tolerance {tol} is too fine for a sweep that reaches {size} from the origin: below {} (4096 units in the last place) its rounding is not bounded; give the document a coarser tolerance or model nearer the origin",
                FINEST * f64::EPSILON * size
            ),
        ));
    }

    // A profile that leans off the plane of the chord and up is laid into
    // it when that moves it by no more than the tolerance (measured at
    // the corners of its box, which bound an affine movement).
    let mut moved: f64 = 0.0;
    for (x, y) in [(xs.0, ys.0), (xs.0, ys.1), (xs.1, ys.0), (xs.1, ys.1)] {
        let sketch = [
            pa[0] + x * ex[0] + y * sy * ny[0],
            pa[1] + x * ex[1] + y * sy * ny[1],
        ];
        let drawn = profile_plane.point(sketch[0], sketch[1], 0.0);
        moved = moved.max(drawn.distance(home.at([x, y])));
    }
    if moved.is_nan() || moved > tol {
        return Err(refuse(
            &[first_a, first_b],
            format!(
                "the section is not in the plane through both rails' starts that holds {up_text}: laying it there would move it by {moved}; draw it on that plane"
            ),
        ));
    }

    // The legs: where either rail goes from one piece to the next.
    let mut joints: Vec<f64> = rail_a
        .pieces
        .iter()
        .chain(&rail_b.pieces)
        .map(|p| p.to)
        .filter(|t| *t < 1.0)
        .collect();
    joints.sort_by(f64::total_cmp);
    let mut cuts: Vec<f64> = vec![0.0];
    for t in joints {
        let last = cuts.last().copied().unwrap_or(0.0);
        if t - last <= SAME_JOINT {
            continue;
        }
        cuts.push(t);
    }
    if 1.0 - cuts.last().copied().unwrap_or(0.0) <= SAME_JOINT && cuts.len() > 1 {
        cuts.pop();
    }
    cuts.push(1.0);
    if cuts.len() > MAX_LEGS {
        return Err(KernelError::TooManyCurves {
            count: cuts.len(),
            limit: MAX_LEGS,
        });
    }
    let longer = rail_a.length.max(rail_b.length);
    let mut legs: Vec<Leg> = Vec::with_capacity(cuts.len());
    for pair in cuts.windows(2) {
        let [from, to] = pair else {
            return Err(internal());
        };
        let middle = 0.5 * (from + to);
        let (Some(a), Some(b)) = (rail_a.piece_at(middle), rail_b.piece_at(middle)) else {
            return Err(internal());
        };
        if (to - from) * longer < 4.0 * tol {
            return Err(refuse(
                &[a.source, b.source],
                format!(
                    "the rails change curves at places only {} apart along them (on e{} and e{}); make the joints of the two rails fall at the same fraction of their lengths or further apart",
                    (to - from) * longer,
                    a.source,
                    b.source
                ),
            ));
        }
        legs.push(Leg {
            from: *from,
            to: *to,
            a,
            b,
        });
    }
    let sides: usize = loops.iter().map(Vec::len).sum();
    if sides.saturating_mul(legs.len()) > crate::sweep::MAX_SWEEP_FACES {
        return Err(KernelError::TooManyCurves {
            count: sides.saturating_mul(legs.len()),
            limit: crate::sweep::MAX_SWEEP_FACES,
        });
    }

    let mut b = Builder {
        body: Body::new(BodyKind::Solid),
        faces: Vec::new(),
        tol,
    };
    // The profile's circles are scaled across with the profile, said
    // explicitly; scaled only along the chord they keep their height.
    let across = |scale: f64| match scaling {
        Scaling::Uniform => Some(scale),
        Scaling::Width => None,
    };
    // Where the body starts: on the profile for open rails; round closed
    // ones at `σ = 0`, a joint of one of them, where the profile is
    // placed as at every station.
    let (start, start_scale) = if closed {
        let (Some(a), Some(b)) = (rail_a.at(0.0), rail_b.at(0.0)) else {
            return Err(internal());
        };
        let there = placed(a, b, set.up).ok_or_else(internal)?;
        (set.station(&there), there.r / d_ref)
    } else {
        (home, first.r / d_ref)
    };
    let mut scale = start_scale;
    let first_ring = b.ring_scaled(&loops, &start, true, across(scale))?;
    let mut ring = first_ring.clone();
    let mut station = start;
    let mut shell: Vec<FaceId> = Vec::new();
    // Round closed rails, a shell per loop of the profile (a hole sweeps
    // a ring of its own).
    let mut groups: Vec<Vec<FaceId>> = vec![Vec::new(); loops.len()];
    let leg_count = legs.len();
    // Per side, the pieces of the rails' curves it runs between.
    let mut parts: Vec<(u32, u32)> = Vec::new();
    let (mut deviation, mut stations) = (0.0_f64, 1_usize);
    let mut spent = 0_usize;
    for (index, leg) in legs.iter().enumerate() {
        // Round closed rails the last leg ends where the first began.
        let closing = closed && index + 1 == leg_count;
        let end = closing.then_some(&start);
        // The cruder proof first (what format 66 built with), the sharper
        // where it proves nothing.
        let crude = leg_bounds(leg, &set, &mut spent, false)?;
        let mut sharp: Option<Bounds> = None;
        if crude.is_none() {
            sharp = leg_bounds(leg, &set, &mut spent, true)?;
        }
        let Some(bounds) = crude.as_ref().or(sharp.as_ref()) else {
            return Err(internal());
        };
        let (motion, next, reached) = match translation(leg, &station, &set, bounds) {
            Some((shift, bound)) => {
                let along = shift.normalized().ok_or_else(internal)?;
                let next = Station {
                    origin: station.origin + shift,
                    ..station
                };
                (Motion::Slide { along }, next, bound)
            }
            None => {
                let mut followed = None;
                let mut costly = false;
                let mut tried_sharp = sharp.is_some();
                let mut bounds = bounds;
                loop {
                    for plan in PLANS {
                        match follow(leg, &station, end, &set, bounds, plan, &mut spent)? {
                            Outcome::Made(made) if made.deviation <= tol / 2.0 => {
                                followed = Some(made);
                                break;
                            }
                            Outcome::Costly => costly = true,
                            Outcome::Made(_) | Outcome::Coarse => {}
                        }
                    }
                    if followed.is_some() || tried_sharp {
                        break;
                    }
                    tried_sharp = true;
                    // The sharper round says for itself whether it ran
                    // out of samples.
                    costly = false;
                    sharp = leg_bounds(leg, &set, &mut spent, true)?;
                    bounds = sharp.as_ref().ok_or_else(internal)?;
                }
                if followed.is_none() && costly {
                    return Err(refuse(
                        &[leg.a.source, leg.b.source],
                        format!(
                            "the sweep along e{} and e{} near {} cannot be shown to lie within the tolerance in {MAX_SAMPLES} samples at any plan; give the document a coarser tolerance",
                            leg.a.source,
                            leg.b.source,
                            point_text(leg.a.at(leg.from))
                        ),
                    ));
                }
                let Some(followed) = followed else {
                    return Err(refuse(
                        &[leg.a.source, leg.b.source],
                        format!(
                            "the sweep along e{} and e{} cannot be brought within half the tolerance ({}) of the true sweep; give the document a coarser tolerance",
                            leg.a.source,
                            leg.b.source,
                            tol / 2.0
                        ),
                    ));
                };
                stations += followed.stations.len().saturating_sub(1);
                let next = *followed.stations.last().ok_or_else(internal)?;
                scale = leg.placed(leg.to, set.up).ok_or_else(internal)?.r / d_ref;
                (
                    Motion::Follow {
                        stations: followed.stations,
                        band: followed.band,
                    },
                    next,
                    followed.deviation,
                )
            }
        };
        let (next, next_ring) = if closing {
            // Round the rails the profile is back where it started, to
            // the bit when the last leg was followed (its last station
            // is the start's), within rounding when it slid.
            let gap = loops
                .iter()
                .flatten()
                .map(|e| next.at(e.start).distance(start.at(e.start)))
                .fold(0.0, f64::max);
            if gap.is_nan() || gap > tol / 4.0 {
                return Err(refuse(
                    &[leg.a.source, leg.b.source],
                    format!(
                        "followed round the closed rails the section comes back {gap} from where it started; this is a bug"
                    ),
                ));
            }
            deviation = deviation.max(reached + gap);
            (start, first_ring.clone())
        } else {
            deviation = deviation.max(reached);
            (next, b.ring_scaled(&loops, &next, true, across(scale))?)
        };
        for (l, elements) in loops.iter().enumerate() {
            let (Some(from), Some(to), Some(low_edges), Some(high_edges)) = (
                ring.vertices.get(l),
                next_ring.vertices.get(l),
                ring.edges.get(l),
                next_ring.edges.get(l),
            ) else {
                return Err(internal());
            };
            let mut swept: Vec<EdgeId> = Vec::with_capacity(elements.len());
            for (element, (&here, &there)) in elements.iter().zip(from.iter().zip(to)) {
                swept.push(b.rail(&motion, element.start, here, there)?.0);
            }
            for (j, element) in elements.iter().enumerate() {
                let (
                    Some(&Some((bottom, bottom_against))),
                    Some(&Some((top, top_against))),
                    Some(&here),
                    Some(&there),
                ) = (
                    low_edges.get(j),
                    high_edges.get(j),
                    swept.get(j),
                    swept.get((j + 1) % elements.len()),
                )
                else {
                    return Err(internal());
                };
                let around = vec![vec![
                    coedge(bottom, bottom_against),
                    coedge(there, false),
                    coedge(top, !top_against),
                    coedge(here, true),
                ]];
                let (surface, reversed) =
                    b.surface(&motion, element, &station, &next, leg.a.source)?;
                let face = b.face(
                    surface,
                    reversed,
                    around,
                    Role::Side,
                    vec![element.source, leg.a.source, leg.b.source],
                );
                shell.push(face);
                groups.get_mut(l).ok_or_else(internal)?.push(face);
                parts.push((leg.a.part, leg.b.part));
            }
        }
        station = next;
        ring = next_ring;
    }

    if closed {
        for group in groups {
            b.body.add_shell(group);
        }
    } else {
        let sources = cap_sources(&profile.outer);
        for (role, at, ring) in [
            (Role::StartCap, &home, &first_ring),
            (Role::EndCap, &station, &ring),
        ] {
            let forward = at.forward()?;
            let start = role == Role::StartCap;
            let normal = if start { -forward } else { forward };
            let frame = Frame::new(at.origin, normal, at.u).ok_or_else(internal)?;
            let mut around = Vec::with_capacity(ring.edges.len());
            for edges in &ring.edges {
                let edges = edges
                    .iter()
                    .map(|e| e.ok_or_else(internal))
                    .collect::<Result<Vec<_>, _>>()?;
                around.push(if start {
                    edges
                        .iter()
                        .rev()
                        .map(|&(edge, against)| coedge(edge, !against))
                        .collect()
                } else {
                    edges
                        .iter()
                        .map(|&(edge, against)| coedge(edge, against))
                        .collect()
                });
            }
            shell.push(b.face(
                craft3d_geom::Surface::Plane(craft3d_geom::Plane::new(frame)),
                false,
                around,
                role,
                sources.clone(),
            ));
        }
        b.body.add_shell(shell);
    }

    // A side's number says which pieces of the rails' curves it runs
    // between (a spline with corners is several), and then counts the
    // faces those give (the quarters of a circle): `k + 4 (b + 1024 a)`,
    // so a corner added to one rail renumbers only the legs on the
    // pieces after it of that rail. Caps are numbered in order.
    let mut seen: BTreeMap<(Role, Vec<u64>, u32, u32), u32> = BTreeMap::new();
    let mut sides = parts.iter();
    for origin in &mut b.faces {
        let (a, bp) = if origin.role == Role::Side {
            sides.next().copied().ok_or_else(internal)?
        } else {
            (0, 0)
        };
        let number = seen
            .entry((origin.role, origin.sources.clone(), a, bp))
            .or_insert(0);
        origin.piece = if origin.role == Role::Side {
            let packed = u64::from(*number) + 4 * (u64::from(bp) + 1024 * u64::from(a));
            u32::try_from(packed).map_err(|_| internal())?
        } else {
            *number
        };
        *number = number.saturating_add(1);
    }
    let faces: Vec<Origin> = b.faces;
    let mut defects = craft3d_topo::check_remembered(&b.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "sweep along two rails",
            defects,
        });
    }
    Ok(Swept {
        body: b.body,
        faces,
        closed,
        length: rail_a.length,
        deviation,
        stations,
    })
}

/// A lower bound on the distance between any place of rail A and any
/// place of rail B, proven by cutting the square of pairs of places into
/// boxes until each is shown apart; rails that come within the tolerance
/// of each other are refused with the place.
fn rails_apart(a: &Rail, b: &Rail, tol: f64) -> Result<f64, KernelError> {
    let mut stack: Vec<(f64, f64, f64, f64)> = vec![(0.0, 1.0, 0.0, 1.0)];
    let mut boxes = 0_usize;
    let mut least = f64::INFINITY;
    while let Some((s0, s1, t0, t1)) = stack.pop() {
        boxes = boxes.saturating_add(1);
        let (sc, tc) = (0.5 * (s0 + s1), 0.5 * (t0 + t1));
        let (Some(p), Some(q)) = (a.at(sc), b.at(tc)) else {
            return Err(internal());
        };
        let d = p.distance(q);
        if d.is_nan() || d <= 4.0 * tol {
            let source = [
                a.piece_at(sc).map_or(0, |x| x.source),
                b.piece_at(tc).map_or(0, |x| x.source),
            ];
            return Err(refuse(
                &source,
                format!(
                    "the rails meet or cross near {} (e{} and e{}), or come within four tolerances of each other; two rails have to stay apart",
                    point_text(p),
                    source[0],
                    source[1]
                ),
            ));
        }
        if boxes > MAX_BOXES {
            return Err(refuse(
                &[],
                format!(
                    "the rails come so near each other (within {d} near {}) that it cannot be told in {MAX_BOXES} steps whether they meet; move them apart",
                    point_text(p)
                ),
            ));
        }
        let (ws, wt) = (a.rate * (s1 - s0), b.rate * (t1 - t0));
        let low = d - 0.5 * (ws + wt);
        if low >= 0.5 * d {
            least = least.min(low);
            continue;
        }
        if ws >= wt {
            stack.push((s0, sc, t0, t1));
            stack.push((sc, s1, t0, t1));
        } else {
            stack.push((s0, s1, t0, tc));
            stack.push((s0, s1, tc, t1));
        }
    }
    // Ends of curves within the tolerance of each other are one joint
    // and are not moved onto each other: a rail may be that much off its
    // curves as drawn.
    let least = least - tol;
    if !(least.is_finite() && least > 0.0) {
        return Err(internal());
    }
    Ok(least)
}
