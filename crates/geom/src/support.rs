//! Proven extremes of a linear function over a piece of a surface or of a
//! curve, and cones that hold the piece's normals or tangents: what a
//! certified distance between faces is built from
//! (docs/design/verification.md › *The proven distance*).
//!
//! A **piece** is a surface over a box of its parameters, a curve over an
//! interval of its parameter, or a point. [`Piece::least`] is a number no
//! larger than the least of `d · x` over the piece's points `x` (its
//! *support* in the direction `-d`):
//!
//! - **In closed form** for every analytic kind. `d · x` over a plane is
//!   linear in `(u, v)`, so least at a corner. Over a cylinder, a cone, a
//!   sphere or a torus it is `d · c + ρ(v) g(u) + h(v)` with
//!   `g(u) = A cos u + B sin u` and `ρ ≥ 0` (the cone kept on one nappe,
//!   the sphere's latitude within ±π/2, the torus a ring torus), so the
//!   least over `u` is `ρ(v) min g`, and what is left over `v` is linear
//!   (cylinder, cone) or again `A' cos v + B' sin v` (sphere, torus):
//!   `min A cos t + B sin t` over an interval is at its ends or at
//!   `atan2(B, A) + π` when that lies within. A line's segment is least at
//!   an end, a circle's or an ellipse's arc as `g`.
//! - **From the convex hull** of the Bézier net for a freeform piece: the
//!   spans of the B-spline the piece covers, each cut to the piece by
//!   blossoming (*The NURBS Book* §5.3, de Casteljau at the new ends), and
//!   with every weight positive the piece lies in the hull of its net's
//!   points, so `min d · P` over the net bounds it. As a piece is halved
//!   the net closes in on the piece quadratically.
//!
//! Every value has an allowance of rounding taken off: 2e-15 of the
//! magnitudes that enter it (the frame's origin, the radii, the
//! parameters' size, the net's reach), some nine units in the last place.
//!
//! **Cones.** [`Piece::cone`] is a unit axis and a half-angle below 90°
//! such that every unit normal (a surface piece) or unit tangent (a curve
//! piece) is within that angle of the axis or of its opposite: a plane's
//! normal (0°); a cylinder's or a cone's turns with `u` alone, by no more
//! than the turn (half the width of `u` from its middle); a sphere's or a
//! torus's by at most half the widths of `u` and `v` together; a circle's
//! tangent by half the width of `t`, an ellipse's by that times the ratio
//! of its axes (the tangent's angle turns at most `a/b` as fast as `t`); a
//! freeform curve's derivative is a positive combination of the
//! differences of its net (adjacent ones for a polynomial; for a rational
//! piece `C' = Σ_{i<j} w_i w_j (P_j − P_i) |K_ij| / w²`), so it lies in the
//! cone of those differences, which also proves the curve regular there;
//! a polynomial surface's `∂u` and `∂v` lie in the cones of the u- and
//! v-differences of its net (axes `a`, `b`, half-angles `α`, `β`), so its
//! normal is within `asin((2 sin(α/2) + 2 sin(β/2)) / |a × b|)` of `a × b`.
//! No cone (`None`) where none is known: a rational freeform surface, a
//! cone whose piece reaches the apex, a piece wider than half a turn.

#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::{Curve3, GeomError, Surface};
use craft3d_math::{Bbox3, Vec3};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// The share of the magnitudes entering a value that is taken off it for
/// rounding.
const ROUNDING: f64 = 2e-15;
/// Most Bézier spans one freeform piece may cover (each cut separately).
const MAX_SPANS: usize = 4096;

/// A homogeneous control point `(x w, y w, z w, w)`.
type Hom = [f64; 4];

/// One Bézier span of a freeform surface: its share of the parameters and
/// its homogeneous net, `(degree_u + 1) × (degree_v + 1)`, row-major.
#[derive(Clone, Debug)]
struct SpanNet {
    u: (f64, f64),
    v: (f64, f64),
    m: usize,
    n: usize,
    c: Vec<Hom>,
}

/// The Bézier spans of a freeform surface, made once per surface.
#[derive(Clone, Debug, Default)]
pub struct SurfaceNets {
    spans: Vec<SpanNet>,
}

/// One Bézier piece of a freeform curve: its span and homogeneous net.
#[derive(Clone, Debug)]
struct PieceNet {
    t: (f64, f64),
    c: Vec<Hom>,
}

/// The Bézier pieces of a freeform curve, made once per curve.
#[derive(Clone, Debug, Default)]
pub struct CurveNets {
    pieces: Vec<PieceNet>,
}

impl SurfaceNets {
    /// The spans of a Bézier or NURBS surface; empty for an analytic one.
    pub fn of(surface: &Surface) -> Result<SurfaceNets, GeomError> {
        let spans = match surface {
            Surface::Bezier(b) => vec![crate::BezierSpan {
                u: (0.0, 1.0),
                v: (0.0, 1.0),
                patch: crate::RationalBezierPatch::from_bezier(b),
            }],
            Surface::Nurbs(s) => s.bezier_spans()?,
            _ => Vec::new(),
        };
        let spans = spans
            .into_iter()
            .map(|span| {
                let (m, n) = span.patch.degrees();
                let c = span
                    .patch
                    .control_points()
                    .into_iter()
                    .zip(span.patch.weights())
                    .map(|(p, w)| [p.x * w, p.y * w, p.z * w, w])
                    .collect();
                SpanNet {
                    u: span.u,
                    v: span.v,
                    m,
                    n,
                    c,
                }
            })
            .collect();
        Ok(SurfaceNets { spans })
    }
}

impl CurveNets {
    /// The pieces of a NURBS curve; empty for an analytic one.
    pub fn of(curve: &Curve3) -> CurveNets {
        let Curve3::Nurbs(c) = curve else {
            return CurveNets::default();
        };
        let pieces = c
            .bezier_pieces()
            .into_iter()
            .map(|(t, points)| PieceNet {
                t,
                c: points
                    .into_iter()
                    .map(|(p, w)| [p.x * w, p.y * w, p.z * w, w])
                    .collect(),
            })
            .collect();
        CurveNets { pieces }
    }
}

/// A unit axis and a half-angle (radians, below π/2): every normal or
/// tangent of the piece is within the angle of the axis or its opposite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cone {
    pub axis: Vec3,
    pub half_angle: f64,
}

/// What a cone holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConeKind {
    /// The normals of a surface piece.
    Normal,
    /// The tangents of a curve piece.
    Tangent,
}

/// A piece of a surface, of a curve, or a point, ready for its support
/// in any direction.
#[derive(Clone, Debug)]
pub enum Piece {
    Plane {
        o: Vec3,
        x: Vec3,
        y: Vec3,
        z: Vec3,
        u: (f64, f64),
        v: (f64, f64),
    },
    /// A cylinder (`slope` 0, `r` its radius) or a cone (`r + v slope`
    /// the radius at `v`, not negative over the piece).
    Revolved {
        o: Vec3,
        x: Vec3,
        y: Vec3,
        z: Vec3,
        r: f64,
        slope: f64,
        u: (f64, f64),
        v: (f64, f64),
        /// The piece reaches a cone's apex.
        apex: bool,
    },
    /// A sphere (`major` 0, `minor` its radius) or a ring torus.
    Round {
        o: Vec3,
        x: Vec3,
        y: Vec3,
        z: Vec3,
        major: f64,
        minor: f64,
        u: (f64, f64),
        v: (f64, f64),
    },
    /// The nets of a freeform surface cut to the piece, as points.
    SurfaceHull {
        points: Vec<Vec3>,
        reach: f64,
        /// Around the normals, for a polynomial piece (see [`Piece::cone`]).
        cone: Option<Cone>,
    },
    Segment {
        a: Vec3,
        b: Vec3,
    },
    /// A circle (`a == b`) or an ellipse's arc.
    Arc {
        o: Vec3,
        x: Vec3,
        y: Vec3,
        a: f64,
        b: f64,
        t: (f64, f64),
    },
    /// The nets of a freeform curve cut to the piece, one per Bézier
    /// piece it covers.
    CurveHull {
        nets: Vec<Vec<Vec3>>,
        rational: bool,
        reach: f64,
    },
    Point(Vec3),
}

/// The least of `a cos t + b sin t` over `[t0, t1]`.
pub fn least_cos(a: f64, b: f64, (t0, t1): (f64, f64)) -> f64 {
    let m = a.hypot(b);
    if !(t1 - t0 < TAU) {
        return -m;
    }
    let f = |t: f64| {
        let (s, c) = t.sin_cos();
        a * c + b * s
    };
    let mut least = f(t0).min(f(t1));
    if m > 0.0 {
        // The least is at atan2(b, a) + π, shifted by turns into the range.
        let at = b.atan2(a) + PI;
        let k = ((t0 - at) / TAU).ceil();
        let t = at + k * TAU;
        if (t0..=t1).contains(&t) {
            least = -m;
        }
    }
    least
}

fn least_linear(k: f64, (a, b): (f64, f64)) -> f64 {
    (k * a).min(k * b)
}

fn mag(r: (f64, f64)) -> f64 {
    r.0.abs().max(r.1.abs())
}

/// A Bernstein row reparametrised to `[a, b]` by blossoming: coefficient
/// `k` of the new row is the blossom at `a` taken `p − k` times and `b`
/// taken `k` times.
fn reparametrise(row: &[Hom], a: f64, b: f64) -> Vec<Hom> {
    let p = row.len().saturating_sub(1);
    (0..=p)
        .map(|k| {
            let mut work = row.to_vec();
            for level in 0..p {
                let t = if level < k { b } else { a };
                for i in 0..p - level {
                    let (Some(&x), Some(&y)) = (work.get(i), work.get(i + 1)) else {
                        continue;
                    };
                    if let Some(slot) = work.get_mut(i) {
                        for (s, (u, v)) in slot.iter_mut().zip(x.iter().zip(y.iter())) {
                            *s = (1.0 - t) * u + t * v;
                        }
                    }
                }
            }
            work.first().copied().unwrap_or([0.0; 4])
        })
        .collect()
}

/// The points of a homogeneous net, `None` unless every weight is
/// positive and finite.
fn points(net: &[Hom]) -> Option<Vec<Vec3>> {
    net.iter()
        .map(|h| {
            let w = h[3];
            (w.is_finite() && w > 0.0).then(|| Vec3::new(h[0] / w, h[1] / w, h[2] / w))
        })
        .collect()
}

fn local(span: (f64, f64), (a, b): (f64, f64)) -> Option<(f64, f64)> {
    let width = span.1 - span.0;
    (width > 0.0).then(|| ((a - span.0) / width, (b - span.0) / width))
}

/// The piece of `surface` over `u × v`. `nets` must be
/// [`SurfaceNets::of`] the same surface for a freeform one. `None` where
/// the box is not a finite, ordered box or a net has a weight that is not
/// positive.
pub fn surface_piece(
    surface: &Surface,
    nets: &SurfaceNets,
    u: (f64, f64),
    v: (f64, f64),
) -> Option<Piece> {
    let ordered = |r: (f64, f64)| r.0.is_finite() && r.1.is_finite() && r.0 <= r.1;
    if !(ordered(u) && ordered(v)) {
        return None;
    }
    let clamp = |r: (f64, f64), lo: f64, hi: f64| (r.0.max(lo), r.1.min(hi));
    Some(match surface {
        Surface::Plane(p) => {
            let f = p.frame();
            Piece::Plane {
                o: f.origin(),
                x: f.x(),
                y: f.y(),
                z: f.z(),
                u,
                v,
            }
        }
        Surface::Cylinder(c) => {
            let f = c.frame();
            Piece::Revolved {
                o: f.origin(),
                x: f.x(),
                y: f.y(),
                z: f.z(),
                r: c.radius(),
                slope: 0.0,
                u,
                v,
                apex: false,
            }
        }
        Surface::Cone(c) => {
            let f = c.frame();
            let (_, (lo, hi)) = surface.domain();
            let v = clamp(v, lo, hi);
            if !(v.0 <= v.1) {
                return None;
            }
            let apex = c.apex_v();
            Piece::Revolved {
                o: f.origin(),
                x: f.x(),
                y: f.y(),
                z: f.z(),
                r: c.radius(),
                slope: c.half_angle().tan(),
                u,
                v,
                apex: (v.0..=v.1).contains(&apex),
            }
        }
        Surface::Sphere(s) => {
            let f = s.frame();
            let v = clamp(v, -FRAC_PI_2, FRAC_PI_2);
            if !(v.0 <= v.1) {
                return None;
            }
            Piece::Round {
                o: f.origin(),
                x: f.x(),
                y: f.y(),
                z: f.z(),
                major: 0.0,
                minor: s.radius(),
                u,
                v,
            }
        }
        Surface::Torus(t) => {
            let f = t.frame();
            let (major, minor) = t.radii();
            Piece::Round {
                o: f.origin(),
                x: f.x(),
                y: f.y(),
                z: f.z(),
                major,
                minor,
                u,
                v,
            }
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            let mut all = Vec::new();
            let mut cut = 0_usize;
            // The differences of the cut nets along u and along v.
            let (mut along_u, mut along_v) = (Vec::new(), Vec::new());
            let mut polynomial = true;
            for span in &nets.spans {
                let (a, b) = (u.0.max(span.u.0), u.1.min(span.u.1));
                let (c, d) = (v.0.max(span.v.0), v.1.min(span.v.1));
                if !(a <= b && c <= d) {
                    continue;
                }
                cut += 1;
                if cut > MAX_SPANS {
                    return None;
                }
                let lu = local(span.u, (a, b))?;
                let lv = local(span.v, (c, d))?;
                let net = cut_net(span, lu, lv);
                polynomial &= net.iter().all(|h| (h[3] - 1.0).abs() <= 1e-12);
                let pts = points(&net)?;
                let width = span.n + 1;
                let at = |i: usize, j: usize| pts.get(i * width + j).copied();
                for i in 0..=span.m {
                    for j in 0..=span.n {
                        if i < span.m
                            && let (Some(p), Some(q)) = (at(i, j), at(i + 1, j))
                        {
                            along_u.extend((q - p).normalized());
                        }
                        if j < span.n
                            && let (Some(p), Some(q)) = (at(i, j), at(i, j + 1))
                        {
                            along_v.extend((q - p).normalized());
                        }
                    }
                }
                all.extend(pts);
            }
            if all.is_empty() {
                return None;
            }
            let reach = all.iter().map(|p| p.length()).fold(0.0, f64::max);
            let cone = if polynomial {
                normal_cone(&along_u, &along_v)
            } else {
                None
            };
            Piece::SurfaceHull {
                points: all,
                reach,
                cone,
            }
        }
    })
}

/// The net of a span over `[u0, u1] × [v0, v1]` of its local `[0, 1]²`.
fn cut_net(span: &SpanNet, (u0, u1): (f64, f64), (v0, v1): (f64, f64)) -> Vec<Hom> {
    let width = span.n + 1;
    let mut c = span.c.clone();
    for j in 0..width {
        let column: Vec<Hom> = (0..=span.m)
            .filter_map(|i| span.c.get(i * width + j).copied())
            .collect();
        for (i, h) in reparametrise(&column, u0, u1).into_iter().enumerate() {
            if let Some(slot) = c.get_mut(i * width + j) {
                *slot = h;
            }
        }
    }
    for i in 0..=span.m {
        let Some(row) = c.get(i * width..(i + 1) * width) else {
            continue;
        };
        let new = reparametrise(row, v0, v1);
        for (j, h) in new.into_iter().enumerate() {
            if let Some(slot) = c.get_mut(i * width + j) {
                *slot = h;
            }
        }
    }
    c
}

/// The narrowest cone found around unit directions: their mean and the
/// largest angle from it; `None` when they do not lie within a right
/// angle of it (or there are none).
fn cone_of(directions: &[Vec3]) -> Option<(Vec3, f64)> {
    let sum = directions.iter().fold(Vec3::ZERO, |s, d| s + *d);
    let axis = sum.normalized()?;
    let half = directions
        .iter()
        .map(|d| d.dot(axis).clamp(-1.0, 1.0).acos())
        .fold(0.0, f64::max)
        + 1e-12;
    (half < FRAC_PI_2).then_some((axis, half))
}

/// A cone around the normals of a polynomial patch from the differences
/// of its net along u and v: `∂u` is a positive combination of the
/// u-differences and `∂v` of the v-differences, so each lies in the cone
/// of those (axis `a`, half-angle `α`; `b`, `β`). Unit vectors within `α`
/// of `a` and `β` of `b` have `|t_u × t_v − a × b| ≤ c_α + c_β` (the
/// chords `2 sin(α/2)`, `2 sin(β/2)`), so the normal is within
/// `asin((c_α + c_β) / |a × b|)` of `a × b` when that is below 1.
fn normal_cone(along_u: &[Vec3], along_v: &[Vec3]) -> Option<Cone> {
    let (a, alpha) = cone_of(along_u)?;
    let (b, beta) = cone_of(along_v)?;
    let n = a.cross(b);
    let s = n.length();
    let e = 2.0 * (0.5 * alpha).sin() + 2.0 * (0.5 * beta).sin();
    if !(e < s) {
        return None;
    }
    let half = (e / s).asin() + 1e-12;
    let axis = n.normalized()?;
    (half < FRAC_PI_2 - 1e-9).then_some(Cone {
        axis,
        half_angle: half,
    })
}

/// The piece of `curve` over `[t0, t1]`. `nets` must be
/// [`CurveNets::of`] the same curve for a NURBS one.
pub fn curve_piece(curve: &Curve3, nets: &CurveNets, t: (f64, f64)) -> Option<Piece> {
    if !(t.0.is_finite() && t.1.is_finite() && t.0 <= t.1) {
        return None;
    }
    Some(match curve {
        Curve3::Line(l) => Piece::Segment {
            a: l.origin() + l.direction() * t.0,
            b: l.origin() + l.direction() * t.1,
        },
        Curve3::Circle(c) => {
            let f = c.frame();
            Piece::Arc {
                o: f.origin(),
                x: f.x(),
                y: f.y(),
                a: c.radius(),
                b: c.radius(),
                t,
            }
        }
        Curve3::Ellipse(e) => {
            let f = e.frame();
            let (a, b) = e.semi_axes();
            Piece::Arc {
                o: f.origin(),
                x: f.x(),
                y: f.y(),
                a,
                b,
                t,
            }
        }
        Curve3::Nurbs(c) => {
            let mut all = Vec::new();
            let rational = c
                .weights()
                .iter()
                .any(|w| (w - 1.0).abs() > 1e-12 * w.abs().max(1.0));
            for piece in &nets.pieces {
                let (a, b) = (t.0.max(piece.t.0), t.1.min(piece.t.1));
                if !(a <= b) {
                    continue;
                }
                if all.len() >= MAX_SPANS {
                    return None;
                }
                let (la, lb) = local(piece.t, (a, b))?;
                all.push(points(&reparametrise(&piece.c, la, lb))?);
            }
            if all.is_empty() {
                return None;
            }
            let reach = all.iter().flatten().map(|p| p.length()).fold(0.0, f64::max);
            Piece::CurveHull {
                nets: all,
                rational,
                reach,
            }
        }
    })
}

impl Piece {
    /// No more than the least of `d · x` over the piece (see the module
    /// documentation), the allowance of rounding taken off.
    pub fn least(&self, d: Vec3) -> f64 {
        let (value, scale) = match self {
            Piece::Plane { o, x, y, u, v, .. } => (
                d.dot(*o) + least_linear(d.dot(*x), *u) + least_linear(d.dot(*y), *v),
                o.length() + mag(*u) + mag(*v),
            ),
            Piece::Revolved {
                o,
                x,
                y,
                z,
                r,
                slope,
                u,
                v,
                ..
            } => {
                let g = least_cos(d.dot(*x), d.dot(*y), *u);
                let dz = d.dot(*z);
                let at = |w: f64| (r + w * slope).max(0.0) * g + w * dz;
                (
                    d.dot(*o) + at(v.0).min(at(v.1)),
                    o.length() + r.abs() + mag(*v) * (1.0 + slope.abs()),
                )
            }
            Piece::Round {
                o,
                x,
                y,
                z,
                major,
                minor,
                u,
                v,
            } => {
                let g = least_cos(d.dot(*x), d.dot(*y), *u);
                let value = d.dot(*o) + major * g + minor * least_cos(g, d.dot(*z), *v);
                (value, o.length() + major + minor)
            }
            Piece::SurfaceHull { points, reach, .. } => (
                points
                    .iter()
                    .map(|p| d.dot(*p))
                    .fold(f64::INFINITY, f64::min),
                *reach,
            ),
            Piece::Segment { a, b } => (d.dot(*a).min(d.dot(*b)), a.length().max(b.length())),
            Piece::Arc { o, x, y, a, b, t } => (
                d.dot(*o) + least_cos(a * d.dot(*x), b * d.dot(*y), *t),
                o.length() + a.max(*b),
            ),
            Piece::CurveHull { nets, reach, .. } => (
                nets.iter()
                    .flatten()
                    .map(|p| d.dot(*p))
                    .fold(f64::INFINITY, f64::min),
                *reach,
            ),
            Piece::Point(p) => (d.dot(*p), p.length()),
        };
        value - ROUNDING * scale * d.length().max(1.0)
    }

    /// No less than the most of `d · x` over the piece.
    pub fn most(&self, d: Vec3) -> f64 {
        -self.least(-d)
    }

    /// A box holding the piece.
    pub fn bbox(&self) -> Bbox3 {
        let lo = Vec3::new(
            self.least(Vec3::X),
            self.least(Vec3::Y),
            self.least(Vec3::Z),
        );
        let hi = Vec3::new(self.most(Vec3::X), self.most(Vec3::Y), self.most(Vec3::Z));
        Bbox3::from_points([lo, hi])
    }

    /// What the piece's cone holds.
    pub fn cone_kind(&self) -> Option<ConeKind> {
        match self {
            Piece::Plane { .. }
            | Piece::Revolved { .. }
            | Piece::Round { .. }
            | Piece::SurfaceHull { .. } => Some(ConeKind::Normal),
            Piece::Segment { .. } | Piece::Arc { .. } | Piece::CurveHull { .. } => {
                Some(ConeKind::Tangent)
            }
            Piece::Point(_) => None,
        }
    }

    /// A cone holding the piece's normals or tangents (see the module
    /// documentation); `None` where none is known.
    pub fn cone(&self) -> Option<Cone> {
        let width = |r: (f64, f64)| r.1 - r.0;
        let mid = |r: (f64, f64)| 0.5 * (r.0 + r.1);
        let within = |axis: Option<Vec3>, half: f64| {
            let axis = axis?;
            (0.0..FRAC_PI_2 - 1e-9).contains(&half).then_some(Cone {
                axis,
                half_angle: half,
            })
        };
        match self {
            Piece::Plane { z, .. } => within(Some(*z), 0.0),
            Piece::Revolved {
                x,
                y,
                z,
                slope,
                u,
                apex,
                ..
            } => {
                if *apex {
                    return None;
                }
                let (s, c) = mid(*u).sin_cos();
                let n = (*x * c + *y * s) - *z * *slope;
                within(n.normalized(), 0.5 * width(*u))
            }
            Piece::Round { x, y, z, u, v, .. } => {
                let (su, cu) = mid(*u).sin_cos();
                let (sv, cv) = mid(*v).sin_cos();
                let n = (*x * cu + *y * su) * cv + *z * sv;
                within(n.normalized(), 0.5 * (width(*u) + width(*v)))
            }
            Piece::SurfaceHull { cone, .. } => *cone,
            Piece::Point(_) => None,
            Piece::Segment { a, b } => within((*b - *a).normalized(), 0.0),
            Piece::Arc { x, y, a, b, t, .. } => {
                let (s, c) = mid(*t).sin_cos();
                let tangent = *x * (-a * s) + *y * (b * c);
                let (big, small) = (a.max(*b), a.min(*b));
                if !(small > 0.0) {
                    return None;
                }
                within(tangent.normalized(), 0.5 * width(*t) * big / small)
            }
            Piece::CurveHull { nets, rational, .. } => {
                let mut directions = Vec::new();
                for net in nets {
                    if *rational {
                        for (i, p) in net.iter().enumerate() {
                            for q in net.iter().skip(i + 1) {
                                directions.extend((*q - *p).normalized());
                            }
                        }
                    } else {
                        for pair in net.windows(2) {
                            if let [p, q] = pair {
                                directions.extend((*q - *p).normalized());
                            }
                        }
                    }
                }
                let sum = directions.iter().fold(Vec3::ZERO, |s, d| s + *d);
                let axis = sum.normalized()?;
                let half = directions
                    .iter()
                    .map(|d| d.dot(axis).clamp(-1.0, 1.0).acos())
                    .fold(0.0, f64::max);
                // The derivative is a positive combination of these: in
                // the cone, and never zero, when it is narrower than a
                // right angle. Rounding of the acos: a little more.
                within(Some(axis), half + 1e-12)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Circle, Cone as ConeSurface, Cylinder, Ellipse, Line, NurbsCurve, Plane, Sphere, Torus,
    };
    use craft3d_math::Frame;

    fn frame() -> Frame {
        Frame::new(
            Vec3::new(0.3, -1.2, 2.5),
            Vec3::new(0.2, 0.3, 1.0),
            Vec3::new(1.0, 0.1, 0.0),
        )
        .unwrap()
    }

    fn directions() -> Vec<Vec3> {
        let mut out = Vec::new();
        for i in 0..7 {
            for j in 0..5 {
                let (a, b) = (i as f64 * 0.9 + 0.1, j as f64 * 0.7 - 1.3);
                out.push(Vec3::new(a.cos() * b.cos(), a.sin() * b.cos(), b.sin()));
            }
        }
        out.extend([Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, -Vec3::Y, -Vec3::Z]);
        out
    }

    /// The least over a dense grid is no less than the proven least, and
    /// not far above it (the closed forms are exact).
    fn check_surface(s: &Surface, u: (f64, f64), v: (f64, f64), exact: bool) {
        let nets = SurfaceNets::of(s).unwrap();
        let piece = surface_piece(s, &nets, u, v).unwrap();
        let n = 120;
        for d in directions() {
            let mut sampled = f64::INFINITY;
            for i in 0..=n {
                for j in 0..=n {
                    let a = u.0 + (u.1 - u.0) * i as f64 / n as f64;
                    let b = v.0 + (v.1 - v.0) * j as f64 / n as f64;
                    sampled = sampled.min(d.dot(s.eval(a, b)));
                }
            }
            let proven = piece.least(d);
            assert!(
                proven <= sampled + 1e-12,
                "{s:?} {d:?}: {proven} > {sampled}"
            );
            if exact {
                assert!(
                    sampled - proven < 2e-3,
                    "{d:?}: {proven} far below {sampled}"
                );
            }
        }
        // Normals within the cone.
        if let Some(cone) = piece.cone() {
            for i in 0..=20 {
                for j in 0..=20 {
                    let a = u.0 + (u.1 - u.0) * i as f64 / 20.0;
                    let b = v.0 + (v.1 - v.0) * j as f64 / 20.0;
                    let Some(n) = s.normal(a, b) else { continue };
                    let angle = n.dot(cone.axis).abs().min(1.0).acos();
                    assert!(angle <= cone.half_angle + 1e-9, "normal outside the cone");
                }
            }
        }
    }

    #[test]
    fn analytic_supports_are_exact_and_hold() {
        let f = frame();
        check_surface(
            &Surface::Plane(Plane::new(f)),
            (-2.0, 3.0),
            (0.5, 1.5),
            true,
        );
        let cyl = Surface::Cylinder(Cylinder::new(f, 2.0).unwrap());
        check_surface(&cyl, (0.3, 2.0), (-1.0, 4.0), true);
        check_surface(&cyl, (0.0, TAU), (-1.0, 4.0), true);
        check_surface(&cyl, (5.0, 7.5), (0.0, 1.0), true);
        let cone = Surface::Cone(ConeSurface::new(f, 1.0, 0.4).unwrap());
        check_surface(&cone, (0.3, 2.0), (-1.0, 4.0), true);
        let sphere = Surface::Sphere(Sphere::new(f, 1.5).unwrap());
        check_surface(&sphere, (0.3, 2.5), (-1.2, 0.4), true);
        check_surface(&sphere, (0.0, TAU), (-FRAC_PI_2, FRAC_PI_2), true);
        let torus = Surface::Torus(Torus::new(f, 3.0, 1.0).unwrap());
        check_surface(&torus, (0.3, 2.5), (-1.2, 2.4), true);
        check_surface(&torus, (0.0, TAU), (0.0, TAU), true);
    }

    #[test]
    fn freeform_supports_hold_and_close_in_when_halved() {
        let patch = crate::BezierPatch::new(
            2,
            2,
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.5),
                Vec3::new(0.0, 2.0, 0.0),
                Vec3::new(1.0, 0.0, 0.7),
                Vec3::new(1.0, 1.0, 2.0),
                Vec3::new(1.0, 2.0, 0.3),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 1.0, 0.4),
                Vec3::new(2.0, 2.0, 0.0),
            ],
        )
        .unwrap();
        let s = Surface::Bezier(patch);
        check_surface(&s, (0.0, 1.0), (0.0, 1.0), false);
        check_surface(&s, (0.2, 0.3), (0.6, 0.62), false);
        // A small piece of a polynomial patch has a normal cone.
        let nets = SurfaceNets::of(&s).unwrap();
        let small = surface_piece(&s, &nets, (0.2, 0.3), (0.6, 0.62)).unwrap();
        let cone = small.cone().expect("a small polynomial piece has a cone");
        assert!(cone.half_angle < 0.5, "{cone:?}");
        // The gap to the true least falls with the square of the piece.
        let nets = SurfaceNets::of(&s).unwrap();
        let d = Vec3::new(0.1, 0.2, -1.0).normalized().unwrap();
        let gap = |w: f64| {
            let piece = surface_piece(&s, &nets, (0.5 - w, 0.5 + w), (0.5 - w, 0.5 + w)).unwrap();
            let mut sampled = f64::INFINITY;
            for i in 0..=40 {
                for j in 0..=40 {
                    let a = 0.5 - w + 2.0 * w * i as f64 / 40.0;
                    let b = 0.5 - w + 2.0 * w * j as f64 / 40.0;
                    sampled = sampled.min(d.dot(s.eval(a, b)));
                }
            }
            sampled - piece.least(d)
        };
        let (g1, g2) = (gap(0.1), gap(0.05));
        assert!(g2 < 0.3 * g1, "{g1} then {g2}");
    }

    #[test]
    fn curve_supports_and_tangent_cones_hold() {
        let f = frame();
        let curves = [
            Curve3::Line(Line::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.3, -0.2, 0.9)).unwrap()),
            Curve3::Circle(Circle::new(f, 2.0).unwrap()),
            Curve3::Ellipse(Ellipse::new(f, 3.0, 1.0).unwrap()),
            Curve3::Nurbs(
                NurbsCurve::new(
                    2,
                    vec![0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0],
                    vec![
                        Vec3::new(0.0, 0.0, 0.0),
                        Vec3::new(1.0, 2.0, 0.0),
                        Vec3::new(2.0, 2.0, 1.0),
                        Vec3::new(3.0, 0.0, 1.0),
                    ],
                    vec![1.0, 2.0, 0.5, 1.0],
                )
                .unwrap(),
            ),
        ];
        for c in &curves {
            let nets = CurveNets::of(c);
            for t in [(0.1, 0.4), (0.2, 0.9), (0.45, 0.55)] {
                let piece = curve_piece(c, &nets, t).unwrap();
                let samples: Vec<(Vec3, Vec3)> = (0..=200)
                    .map(|i| c.derivatives(t.0 + (t.1 - t.0) * i as f64 / 200.0))
                    .collect();
                for d in directions() {
                    let sampled = samples
                        .iter()
                        .map(|(p, _)| d.dot(*p))
                        .fold(f64::INFINITY, f64::min);
                    assert!(piece.least(d) <= sampled + 1e-12, "{c:?}");
                }
                if let Some(cone) = piece.cone() {
                    for (_, tangent) in &samples {
                        let angle = tangent
                            .normalized()
                            .unwrap()
                            .dot(cone.axis)
                            .abs()
                            .min(1.0)
                            .acos();
                        assert!(angle <= cone.half_angle + 1e-9, "{c:?}: tangent outside");
                    }
                }
            }
        }
    }

    #[test]
    fn the_least_of_a_turning_line_is_found_inside_the_range() {
        assert!((least_cos(1.0, 0.0, (2.0, 4.0)) + 1.0).abs() < 1e-15);
        assert!((least_cos(1.0, 0.0, (0.0, 1.0)) - 1.0_f64.cos()).abs() < 1e-15);
        assert!((least_cos(0.0, 1.0, (-8.0, -7.0)) + 1.0).abs() < 1e-15);
        assert_eq!(least_cos(0.0, 0.0, (0.0, 1.0)), 0.0);
    }
}
