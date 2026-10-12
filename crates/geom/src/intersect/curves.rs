//! Curve/surface intersections beyond lines and conics with planes and
//! cylinders: closed forms for a line with a sphere or a cone, a circle
//! with a sphere and a circle about the axis of a surface of revolution;
//! everything else by root finding on the signed distance along the curve.
//!
//! **The numeric method.** With `g(t) = F(C(t))` the signed distance from
//! the curve to the surface, an interval of half width `h` about `c`
//!
//! - holds no root when `|g(c)| > L h` (`L` bounds the curve's speed; a
//!   distance is 1-Lipschitz);
//! - holds at most one, found by its change of sign, when
//!   `|g'(c)| > (K L² + |C''|) h` (`K` the surface's curvature): `g` is
//!   monotonic there;
//! - is split otherwise, down to intervals the curve crosses in a quarter
//!   of the linear tolerance; what is left there and within the tolerance
//!   of the surface is a tangent hit.
//!
//! Two crossings with the curve never more than the tolerance past the
//! surface between them are one tangent hit, as in the closed forms. The
//! bounds are proofs for lines, circles and ellipses against the analytic
//! kinds and estimates from samples for NURBS curves and freeform
//! surfaces.

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::field::{Field, cone_apex_axis};
use super::{CurveHit, CurveIntersection, wrap};
use crate::{Circle, Cone, Curve3, GeomError, Line, Sphere, Surface};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use std::f64::consts::TAU;

const MAX_INTERVALS: usize = 200_000;
const MAX_FLOOR: usize = 4_000;

fn hits(mut found: Vec<CurveHit>) -> CurveIntersection {
    found.retain(|h| h.point.is_finite() && h.t.is_finite());
    found.sort_by(|a, b| a.t.total_cmp(&b.t));
    CurveIntersection::Hits(found)
}

/// The roots of `a t² + b t + c`, in the form that does not cancel.
fn quadratic(a: f64, b: f64, c: f64) -> Vec<f64> {
    let disc = b * b - 4.0 * a * c;
    if a == 0.0 || disc < 0.0 {
        return Vec::new();
    }
    let q = -0.5 * (b + if b >= 0.0 { 1.0 } else { -1.0 } * disc.sqrt());
    let mut roots = vec![q / a];
    if q != 0.0 {
        roots.push(c / q);
    }
    roots
}

pub(super) fn line_sphere(line: &Line, sphere: &Sphere, tol: &Tolerance) -> CurveIntersection {
    let (o, d, r) = (line.origin(), line.direction(), sphere.radius());
    let w = sphere.frame().origin() - o;
    let nearest = w.dot(d);
    let reach = (w - d * nearest).length();
    let hit = |t: f64, tangent: bool| CurveHit {
        t,
        point: o + d * t,
        tangent,
    };
    hits(if reach > r + tol.linear {
        Vec::new()
    } else if (reach - r).abs() <= tol.linear {
        vec![hit(nearest, true)]
    } else {
        let half = ((r - reach) * (r + reach)).max(0.0).sqrt();
        vec![hit(nearest - half, false), hit(nearest + half, false)]
    })
}

/// A line and a cone: `cos²α |q|² = (q·a)²` along `q = o + d t` is a
/// quadratic; the roots with `q·a ≥ 0` are on the cone's own nappe. A line
/// along a ruling is on the surface (the cone ends at its apex; the line
/// does not).
pub(super) fn line_cone(
    line: &Line,
    cone: &Cone,
    tol: &Tolerance,
) -> Result<CurveIntersection, GeomError> {
    let (apex, axis) = cone_apex_axis(cone);
    let cos = cone.half_angle().cos();
    let (o, d) = (line.origin() - apex, line.direction());
    let (da, oa) = (d.dot(axis), o.dot(axis));
    let cos2 = cos * cos;
    let (a, b, c) = (
        cos2 - da * da,
        2.0 * (cos2 * o.dot(d) - oa * da),
        cos2 * o.length_squared() - oa * oa,
    );
    let field = Field::new(&Surface::Cone(*cone))?;
    let distance = |t: f64| {
        field
            .sample(line.origin() + d * t)
            .map_or(f64::INFINITY, |s| s.dist)
    };
    let hit = |t: f64, tangent: bool| CurveHit {
        t,
        point: line.origin() + d * t,
        tangent,
    };
    let parallel = (da.abs() - cos).abs() <= tol.angular; // to a ruling
    if parallel {
        // Through the apex it is a ruling; otherwise it meets the cone once.
        let past = -o.dot(d);
        if (o + d * past).length() <= tol.linear {
            return Ok(CurveIntersection::OnSurface);
        }
        if b == 0.0 {
            return Ok(hits(Vec::new()));
        }
        let t = -c / b;
        return Ok(hits(if distance(t) <= tol.linear {
            vec![hit(t, false)]
        } else {
            Vec::new()
        }));
    }
    // Where the line comes closest to crossing: the quadratic's vertex.
    let vertex = -b / (2.0 * a);
    let mut roots: Vec<f64> = quadratic(a, b, c)
        .into_iter()
        .filter(|t| (o + d * *t).dot(axis) >= 0.0 && distance(*t) <= tol.linear)
        .collect();
    roots.sort_by(f64::total_cmp);
    roots.dedup();
    let touching = vertex.is_finite() && distance(vertex) <= tol.linear;
    Ok(hits(match roots.as_slice() {
        [] if touching => vec![hit(vertex, true)],
        [first, second] if touching && *first <= vertex && vertex <= *second => {
            vec![hit(vertex, true)]
        }
        // Through the apex: flatter than the rulings the line stays
        // outside on both sides and only touches there.
        [only] if (o + d * *only).length() <= tol.linear => vec![hit(*only, da.abs() < cos)],
        _ => roots.iter().map(|t| hit(*t, false)).collect(),
    }))
}

/// A circle and a sphere: the squared distance to the sphere's centre is
/// `h + a cos t + b sin t` along the circle.
pub(super) fn circle_sphere(
    circle: &Circle,
    sphere: &Sphere,
    tol: &Tolerance,
) -> CurveIntersection {
    let (f, r) = (circle.frame(), circle.radius());
    let big = sphere.radius();
    let w = f.origin() - sphere.frame().origin();
    let (a, b) = (2.0 * r * w.dot(f.x()), 2.0 * r * w.dot(f.y()));
    let h = w.length_squared() + r * r - big * big;
    // A change of the squared distance by 2 R tol is a move by tol.
    let slack = 2.0 * big * tol.linear;
    let amplitude = a.hypot(b);
    let at = |angle: f64, tangent: bool| {
        let t = wrap(angle);
        let (s, c) = t.sin_cos();
        CurveHit {
            t,
            point: f.point(r * c, r * s, 0.0),
            tangent,
        }
    };
    if amplitude <= slack {
        return if h.abs() <= slack {
            CurveIntersection::OnSurface
        } else {
            hits(Vec::new())
        };
    }
    if h.abs() > amplitude + slack {
        return hits(Vec::new());
    }
    let phase = b.atan2(a);
    if (h.abs() - amplitude).abs() <= slack {
        let angle = if h > 0.0 {
            phase + std::f64::consts::PI
        } else {
            phase
        };
        return hits(vec![at(angle, true)]);
    }
    let spread = (-h / amplitude).clamp(-1.0, 1.0).acos();
    hits(vec![at(phase - spread, false), at(phase + spread, false)])
}

/// A circle about the axis of a surface of revolution is on it or misses
/// it. `None` when the circle is not about the surface's axis.
fn coaxial_circle(
    circle: &Circle,
    surface: &Surface,
    field: &Field,
    tol: &Tolerance,
) -> Option<CurveIntersection> {
    let (origin, axis) = match surface {
        Surface::Cone(c) => (c.frame().origin(), c.frame().z()),
        Surface::Torus(t) => (t.frame().origin(), t.frame().z()),
        Surface::Cylinder(c) => (c.frame().origin(), c.frame().z()),
        Surface::Sphere(s) => (s.frame().origin(), circle.frame().z()),
        _ => return None,
    };
    let f = circle.frame();
    let offset = f.origin() - origin;
    if f.z().cross(axis).length() > tol.angular
        || (offset - axis * offset.dot(axis)).length() > tol.linear
    {
        return None;
    }
    let on = field.sample(f.point(circle.radius(), 0.0, 0.0))?.dist <= tol.linear;
    Some(if on {
        CurveIntersection::OnSurface
    } else {
        hits(Vec::new())
    })
}

/// Bounds on the first and second derivative of a curve over its range.
fn curve_bounds(curve: &Curve3, range: (f64, f64)) -> (f64, f64) {
    match curve {
        Curve3::Line(_) => (1.0, 0.0),
        Curve3::Circle(c) => (c.radius(), c.radius()),
        Curve3::Ellipse(e) => {
            let big = e.semi_axes().0.max(e.semi_axes().1);
            (big, big)
        }
        Curve3::Nurbs(n) => {
            // Estimated from samples, sixteen per control point.
            let count = n.control_points().len().saturating_mul(16).clamp(64, 4096);
            let step = (range.1 - range.0) / count as f64;
            let (mut speed, mut bend) = (0.0_f64, 0.0_f64);
            let mut previous: Option<Vec3> = None;
            for k in 0..=count {
                let (_, d) = n.derivatives(range.0 + step * k as f64);
                speed = speed.max(d.length());
                if let Some(p) = previous {
                    bend = bend.max(d.distance(p) / step);
                }
                previous = Some(d);
            }
            (1.5 * speed, 2.0 * bend)
        }
    }
}

/// The part of a line that can meet a bounded surface.
fn line_range(line: &Line, surface: &Surface, tol: &Tolerance) -> Option<(f64, f64)> {
    let bbox = match surface {
        Surface::Torus(t) => {
            let reach = t.radii().0 + t.radii().1;
            let c = t.frame().origin();
            let d = Vec3::new(reach, reach, reach);
            Bbox3::from_points([c - d, c + d])
        }
        Surface::Sphere(s) => {
            let c = s.frame().origin();
            let d = Vec3::new(s.radius(), s.radius(), s.radius());
            Bbox3::from_points([c - d, c + d])
        }
        Surface::Bezier(b) => b.control_bbox(),
        Surface::Nurbs(n) => Bbox3::from_points(n.control_points().iter().copied()),
        Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_) => return None,
    };
    let (lo, hi) = bbox.corners()?;
    let (o, d) = (line.origin(), line.direction());
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for (oc, dc, low, high) in [
        (o.x, d.x, lo.x, hi.x),
        (o.y, d.y, lo.y, hi.y),
        (o.z, d.z, lo.z, hi.z),
    ] {
        let (low, high) = (low - 2.0 * tol.linear, high + 2.0 * tol.linear);
        if dc == 0.0 {
            if oc < low || oc > high {
                return None;
            }
            continue;
        }
        let (a, b) = ((low - oc) / dc, (high - oc) / dc);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    (t0 < t1 && t0.is_finite() && t1.is_finite()).then_some((t0, t1))
}

/// Every root of the signed distance along the curve, by subdivision.
pub(super) fn numeric(
    curve: &Curve3,
    surface: &Surface,
    tol: &Tolerance,
) -> Result<CurveIntersection, GeomError> {
    if !(tol.linear.is_finite() && tol.linear > 0.0 && tol.angular.is_finite() && tol.angular > 0.0)
    {
        return Err(GeomError::InvalidParameter(
            "a tolerance must be positive and finite",
        ));
    }
    let field = Field::new(surface)?;
    if let Curve3::Circle(c) = curve
        && let Some(found) = coaxial_circle(c, surface, &field, tol)
    {
        return Ok(found);
    }
    let range = match curve {
        Curve3::Line(l) => match line_range(l, surface, tol) {
            Some(range) => range,
            None if matches!(
                surface,
                Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_)
            ) =>
            {
                return Err(GeomError::Unsupported(
                    "a line against an unbounded surface without a closed form",
                ));
            }
            None => return Ok(hits(Vec::new())),
        },
        _ => curve.domain(),
    };
    let (t0, t1) = range;
    if !(t0.is_finite() && t1.is_finite() && t0 < t1) {
        return Err(GeomError::InvalidParameter("a curve with no finite range"));
    }
    let periodic = curve.period().is_some();
    let sample = |t: f64| {
        let (p, d) = curve.derivatives(t);
        field.sample(p).map(|s| (p, s, s.normal.dot(d)))
    };
    // On the surface all along?
    if !matches!(curve, Curve3::Line(_))
        && (0..=64).all(|k| {
            sample(t0 + (t1 - t0) * f64::from(k) / 64.0)
                .is_some_and(|(_, s, _)| s.dist <= tol.linear)
        })
    {
        return Ok(CurveIntersection::OnSurface);
    }
    let (speed, bend) = curve_bounds(curve, range);
    if !(speed.is_finite() && bend.is_finite()) {
        return Err(GeomError::InvalidParameter("a curve that is not finite"));
    }
    let floor = 0.25 * tol.linear;
    // Sixteen pieces to start with: a closed curve is not one interval.
    let mut stack = Vec::new();
    for k in (0..16).rev() {
        let a = t0 + (t1 - t0) * f64::from(k) / 16.0;
        let b = if k == 15 {
            t1
        } else {
            t0 + (t1 - t0) * f64::from(k + 1) / 16.0
        };
        stack.push((a, b));
    }
    let mut found: Vec<CurveHit> = Vec::new();
    let mut flats: Vec<(f64, f64)> = Vec::new();
    let mut seen = 0_usize;
    while let Some((lo, hi)) = stack.pop() {
        seen += 1;
        if seen > MAX_INTERVALS || flats.len() > MAX_FLOOR {
            return Err(GeomError::NoConvergence(
                "the curve runs along the surface, or nearly, over too long a stretch to intersect",
            ));
        }
        let (mid, half) = (0.5 * (lo + hi), 0.5 * (hi - lo));
        let reach = speed * half;
        let monotonic = match sample(mid) {
            Some((_, s, _)) if s.dist > reach => continue,
            Some((p, _, slope)) => field
                .bend(p, reach)
                .is_some_and(|k| slope.abs() > 1.01 * (k * speed * speed + bend) * half),
            None => false,
        };
        if monotonic {
            let (Some((_, a, _)), Some((_, b, _))) = (sample(lo), sample(hi)) else {
                continue;
            };
            if (a.signed >= 0.0) == (b.signed >= 0.0) {
                continue;
            }
            // Bisection with regula falsi steps, to rounding.
            let (mut x0, mut x1, mut f0, mut f1) = (lo, hi, a.signed, b.signed);
            for k in 0..200 {
                let guess = (x0 * f1 - x1 * f0) / (f1 - f0);
                let x = if k % 3 != 2 && guess > x0 && guess < x1 {
                    guess
                } else {
                    0.5 * (x0 + x1)
                };
                if !(x > x0 && x < x1) {
                    break;
                }
                let Some((_, s, _)) = sample(x) else { break };
                if (s.signed >= 0.0) == (f0 >= 0.0) {
                    (x0, f0) = (x, s.signed);
                } else {
                    (x1, f1) = (x, s.signed);
                }
            }
            let t = if f0.abs() <= f1.abs() { x0 } else { x1 };
            // Past the edge of a patch the sign changes without a hit.
            if let Some((point, s, _)) = sample(t)
                && s.dist <= tol.linear
            {
                found.push(CurveHit {
                    t,
                    point,
                    tangent: false,
                });
            }
        } else if reach <= floor || !(half > 1e-15 * (t1 - t0)) {
            flats.push((lo, hi));
        } else {
            stack.push((mid, hi));
            stack.push((lo, mid));
        }
    }
    // Runs of flat intervals: one tangent hit each, where the curve is
    // within the tolerance.
    flats.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut run: Option<(f64, f64)> = None;
    let mut runs = Vec::new();
    for (lo, hi) in flats {
        run = match run {
            Some((a, b)) if lo <= b + 1e-12 * (t1 - t0) => Some((a, hi.max(b))),
            Some(done) => {
                runs.push(done);
                Some((lo, hi))
            }
            None => Some((lo, hi)),
        };
    }
    runs.extend(run);
    for (a, b) in runs {
        let mut best: Option<(f64, Vec3, f64)> = None;
        for k in 0..=16 {
            let t = a + (b - a) * f64::from(k) / 16.0;
            if let Some((p, s, _)) = sample(t)
                && best.is_none_or(|(_, _, d)| s.dist < d)
            {
                best = Some((t, p, s.dist));
            }
        }
        if let Some((t, point, dist)) = best
            && dist <= tol.linear
        {
            let crosses = match (sample(a), sample(b)) {
                (Some((_, x, _)), Some((_, y, _))) => {
                    (x.signed >= 0.0) != (y.signed >= 0.0)
                        && x.dist > tol.linear
                        && y.dist > tol.linear
                }
                _ => false,
            };
            found.push(CurveHit {
                t,
                point,
                tangent: !crosses,
            });
        }
    }
    found.sort_by(|a, b| a.t.total_cmp(&b.t));
    // Two hits with the curve within the tolerance of the surface all the
    // way between them are one touch.
    let grazes = |a: f64, b: f64| {
        (1..16).all(|k| {
            sample(a + (b - a) * f64::from(k) / 16.0).is_some_and(|(_, s, _)| s.dist <= tol.linear)
        })
    };
    let mut merged: Vec<CurveHit> = Vec::new();
    for hit in found {
        match merged.last_mut() {
            Some(last) if grazes(last.t, hit.t) => {
                let t = 0.5 * (last.t + hit.t);
                *last = CurveHit {
                    t,
                    point: curve.eval(t),
                    tangent: true,
                };
            }
            _ => merged.push(hit),
        }
    }
    // The same across the seam of a closed curve.
    if periodic
        && merged.len() > 1
        && let (Some(first), Some(last)) = (merged.first().copied(), merged.last().copied())
        && grazes(last.t, first.t + (t1 - t0))
    {
        merged.pop();
        if let Some(head) = merged.first_mut() {
            head.tangent = true;
        }
    }
    if periodic {
        for hit in &mut merged {
            hit.t = if (t1 - t0 - TAU).abs() < 1e-12 {
                wrap(hit.t)
            } else {
                hit.t
            };
        }
    }
    Ok(hits(merged))
}
