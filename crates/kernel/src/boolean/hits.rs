//! Where a curve meets a surface, for every curve kind and every analytic
//! surface kind.
//!
//! `craft3d_geom::intersect::intersect_curve_surface` is asked first: where
//! it has a closed form the answer is exact. Where it has none (it says
//! `Unsupported`), the roots of the signed distance from the curve to the
//! surface are isolated numerically. The distance changes by at most the
//! length travelled, so a stretch whose middle is further from the surface
//! than half its length cannot meet it: stretches are halved until each is
//! either clear of the surface, or within the tolerance of it over its
//! whole length, or too short to matter. Nothing is found by sampling and
//! hoping.
//!
//! When `intersect_curve_surface` covers every pair, the numeric path is
//! simply never taken.

use craft3d_geom::intersect::{CurveHit, CurveIntersection, intersect_curve_surface};
use craft3d_geom::{Curve3, GeomError, Surface};
use craft3d_math::{Bbox3, Tolerance, Vec3};

const FREEFORM: &str = "a freeform (Bézier or NURBS) surface; booleans handle planes, cylinders, cones, spheres and tori";
const ALONG: &str = "a curve that runs along a surface within the tolerance without lying on it (at the edge of tolerance)";

/// The most evaluations one curve and surface may cost.
const MAX_EVALUATIONS: usize = 200_000;

/// A signed distance from `p` to the surface: zero on it, changing by at
/// most the distance moved, and of one sign on each side. For a cone it is
/// the distance to the cone continued past its apex as a line in the
/// half-plane through the axis, which is positive there and so has no zero
/// that the surface does not have. `None` for freeform surfaces.
pub(super) fn signed_distance(surface: &Surface, p: Vec3) -> Option<f64> {
    let d = match surface {
        Surface::Plane(s) => s.frame().local(p).2,
        Surface::Cylinder(s) => {
            let (x, y, _) = s.frame().local(p);
            x.hypot(y) - s.radius()
        }
        Surface::Cone(s) => {
            let (x, y, z) = s.frame().local(p);
            let (sine, cosine) = s.half_angle().sin_cos();
            (x.hypot(y) - s.radius()) * cosine - z * sine
        }
        Surface::Sphere(s) => p.distance(s.frame().origin()) - s.radius(),
        Surface::Torus(s) => {
            let (x, y, z) = s.frame().local(p);
            let (major, minor) = s.radii();
            (x.hypot(y) - major).hypot(z) - minor
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    };
    Some(d)
}

/// An upper bound on the speed of the curve: the length travelled per unit
/// of parameter.
pub(super) fn speed_bound(curve: &Curve3) -> f64 {
    match curve {
        Curve3::Line(_) => 1.0,
        Curve3::Circle(c) => c.radius(),
        Curve3::Ellipse(e) => e.semi_axes().0.max(e.semi_axes().1),
        Curve3::Nurbs(c) => {
            // The derivative of a B-spline is a B-spline over the scaled
            // differences of its control points; for a rational curve the
            // differences of the weighted points, seen from a point of the
            // curve, are bounded through the size of the control polygon.
            let degree = c.degree();
            let (knots, points, weights) = (c.knots(), c.control_points(), c.weights());
            let size = Bbox3::from_points(points.iter().copied()).diagonal();
            let (mut lightest, mut heaviest) = (f64::INFINITY, 0.0_f64);
            for &w in weights {
                lightest = lightest.min(w);
                heaviest = heaviest.max(w);
            }
            let mut bound = 0.0_f64;
            for (i, (pair, weight)) in points.windows(2).zip(weights.windows(2)).enumerate() {
                let (Some(&lo), Some(&hi)) = (knots.get(i + 1), knots.get(i + degree + 1)) else {
                    continue;
                };
                let (&[a, b], &[wa, wb]) = (pair, weight) else {
                    continue;
                };
                if hi > lo {
                    let reach = heaviest * a.distance(b) + (wb - wa).abs() * size;
                    bound = bound.max(degree as f64 * reach / (hi - lo));
                }
            }
            bound / lightest
        }
    }
}

/// Near the surface the signed distance along the curve bends by no more
/// than the first number, per unit of parameter squared: from the
/// curvature of the surface and of the curve. The second number is how
/// near "near" is: within that distance of the surface its level sets are
/// at most twice as curved as it is. `None` where there is no such bound
/// (a cone, whose curvature has no limit towards the apex; a spline).
fn bend_bound(curve: &Curve3, surface: &Surface) -> Option<(f64, f64)> {
    let (curvature, zone) = match surface {
        Surface::Plane(_) => (0.0, f64::INFINITY),
        Surface::Cylinder(s) => (2.0 / s.radius(), 0.25 * s.radius()),
        Surface::Sphere(s) => (2.0 / s.radius(), 0.25 * s.radius()),
        Surface::Torus(s) => {
            let least = s.radii().1.min(s.radii().0 - s.radii().1);
            (2.0 / least, 0.25 * least)
        }
        Surface::Cone(_) | Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    };
    let (speed, turn) = match curve {
        Curve3::Line(_) => (1.0, 0.0),
        Curve3::Circle(c) => (c.radius(), c.radius()),
        Curve3::Ellipse(e) => {
            let reach = e.semi_axes().0.max(e.semi_axes().1);
            (reach, reach)
        }
        Curve3::Nurbs(_) => return None,
    };
    let bound = speed * speed * curvature + turn;
    (bound.is_finite() && zone > 0.0).then_some((bound, zone))
}

/// What a stretch of the curve is with respect to the surface.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Further than the tolerance from it everywhere, on this side.
    Clear { positive: bool },
    /// Within the tolerance of it, or too short to tell.
    Near,
}

/// The parameter in `[lo, hi]` where `f` is zero, given that it changes
/// sign between them.
fn root(f: &dyn Fn(f64) -> f64, mut lo: f64, mut hi: f64) -> f64 {
    let rising = f(hi) > 0.0;
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if !(mid > lo && mid < hi) {
            break;
        }
        if (f(mid) > 0.0) == rising {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The parameter in `[lo, hi]` where `|f|` is least, as far as sampling and
/// then narrowing in on the best sample finds it.
fn closest(f: &dyn Fn(f64) -> f64, lo: f64, hi: f64) -> f64 {
    const SAMPLES: u32 = 16;
    let at = |k: u32| lo + (hi - lo) * f64::from(k) / f64::from(SAMPLES);
    let mut best = (f64::INFINITY, lo);
    for k in 0..=SAMPLES {
        let value = f(at(k)).abs();
        if value < best.0 {
            best = (value, at(k));
        }
    }
    let step = (hi - lo) / f64::from(SAMPLES);
    let (mut a, mut b) = ((best.1 - step).max(lo), (best.1 + step).min(hi));
    for _ in 0..60 {
        let (m1, m2) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
        if !(m1 > a && m2 < b && m1 < m2) {
            break;
        }
        if f(m1).abs() <= f(m2).abs() {
            b = m2;
        } else {
            a = m1;
        }
    }
    let t = 0.5 * (a + b);
    if f(t).abs() <= best.0 { t } else { best.1 }
}

/// The hits of the piece of `curve` over `range` with the zero set of
/// `distance`.
fn numeric(
    curve: &Curve3,
    range: (f64, f64),
    distance: &dyn Fn(Vec3) -> Option<f64>,
    bend: Option<(f64, f64)>,
    tol: f64,
) -> Result<CurveIntersection, GeomError> {
    let (start, end) = range;
    let speed = speed_bound(curve);
    if !(start.is_finite() && end.is_finite() && speed.is_finite() && speed > 0.0) {
        return Err(GeomError::InvalidParameter(
            "a curve piece without a finite extent cannot be intersected numerically",
        ));
    }
    if end <= start {
        return Ok(CurveIntersection::Hits(Vec::new()));
    }
    let failed = std::cell::Cell::new(false);
    let f = |t: f64| match distance(curve.eval(t)) {
        Some(d) if d.is_finite() => d,
        _ => {
            failed.set(true);
            0.0
        }
    };
    // Stretches shorter than a quarter of the tolerance are not divided.
    let finest = (0.25 * tol / speed).max((end - start) * 1e-13);
    // A curve on the surface, as far as a look at it can tell (which is
    // all the caller's own test does): found here rather than by dividing
    // it into ten thousand stretches that are each on it.
    const LOOKS: u32 = 32;
    if (0..=LOOKS).all(|k| f(start + (end - start) * f64::from(k) / f64::from(LOOKS)).abs() <= tol)
        && !failed.get()
    {
        return Ok(CurveIntersection::OnSurface);
    }
    let mut runs: Vec<(f64, f64, Kind)> = Vec::new();
    let mut stack = vec![(start, end)];
    let mut evaluations = 0_usize;
    while let Some((a, b)) = stack.pop() {
        evaluations += 1;
        if evaluations > MAX_EVALUATIONS {
            return Err(GeomError::Unsupported(ALONG));
        }
        let mid = 0.5 * (a + b);
        let value = f(mid);
        // How far the distance can stray from its value in the middle:
        // by the length travelled, or, near the surface, by the slope
        // across the stretch and how much that slope can change.
        let half = 0.5 * (b - a);
        let mut stray = speed * half;
        if let Some((bend, zone)) = bend
            && value.abs() <= zone
            && stray <= zone
        {
            let slope = (f(b) - f(a)).abs() / (b - a) + 2.0 * bend * half;
            stray = stray.min(slope * half);
        }
        let kind = if value.abs() - stray > tol {
            Kind::Clear {
                positive: value > 0.0,
            }
        } else if value.abs() + stray <= tol || b - a <= finest || !(mid > a && mid < b) {
            Kind::Near
        } else {
            stack.push((mid, b));
            stack.push((a, mid));
            continue;
        };
        match runs.last_mut() {
            Some(last) if last.2 == kind => last.1 = b,
            _ => runs.push((a, b, kind)),
        }
    }
    if failed.get() {
        return Err(GeomError::Unsupported(FREEFORM));
    }
    if let [(_, _, Kind::Near)] = runs.as_slice() {
        return Ok(CurveIntersection::OnSurface);
    }

    let side = |run: Option<&(f64, f64, Kind)>| match run {
        Some(&(_, _, Kind::Clear { positive })) => Some(positive),
        _ => None,
    };
    let mut hits = Vec::new();
    for (index, &(lo, hi, kind)) in runs.iter().enumerate() {
        if kind != Kind::Near {
            continue;
        }
        let before = index.checked_sub(1).and_then(|i| side(runs.get(i)));
        let after = side(runs.get(index + 1));
        let (t, tangent) = match (before, after) {
            (Some(x), Some(y)) if x != y => (root(&f, lo, hi), false),
            (Some(_), Some(_)) => (closest(&f, lo, hi), true),
            // The piece starts or ends within the tolerance of the
            // surface: on it there, unless it crosses on the way.
            (None, Some(positive)) if f(lo) != 0.0 && (f(lo) > 0.0) != positive => {
                (root(&f, lo, hi), false)
            }
            (Some(positive), None) if f(hi) != 0.0 && (f(hi) > 0.0) != positive => {
                (root(&f, lo, hi), false)
            }
            // No crossing seen: as far as this piece shows, a touch.
            _ => (closest(&f, lo, hi), true),
        };
        if f(t).abs() > tol {
            continue;
        }
        let point = curve.eval(t);
        if point.is_finite() {
            hits.push(CurveHit { t, point, tangent });
        }
    }
    if failed.get() {
        return Err(GeomError::Unsupported(FREEFORM));
    }
    Ok(CurveIntersection::Hits(hits))
}

/// Where the piece of `curve` over `range` meets `surface`.
///
/// A closed form gives the hits of the whole curve (a line is infinite, a
/// circle closed), the numeric path only those within `range`; the caller
/// keeps the ones it cares about either way. `range` must be finite.
pub(super) fn curve_hits(
    curve: &Curve3,
    range: (f64, f64),
    surface: &Surface,
    tolerance: &Tolerance,
) -> Result<CurveIntersection, GeomError> {
    // Geometry's closed forms are exact, and are asked for where they
    // exist: lines, circles and ellipses against planes and cylinders.
    // For every other pair with an analytic surface the roots are isolated
    // here rather than by geometry's general routine, because what the
    // booleans need differs from it in two places that decide inside from
    // outside: the piece is taken as given (a hit at an end of it is a
    // touch, since the piece does not show a crossing there), and a
    // stretch near the surface is bounded, never sampled.
    let exact = matches!(surface, Surface::Plane(_) | Surface::Cylinder(_))
        && !matches!(curve, Curve3::Nurbs(_));
    if exact {
        match intersect_curve_surface(curve, surface, tolerance) {
            Err(GeomError::Unsupported(_) | GeomError::NoConvergence(_)) => {}
            other => return other,
        }
    }
    if signed_distance(surface, Vec3::ZERO).is_none() {
        return Err(GeomError::Unsupported(FREEFORM));
    }
    numeric(
        curve,
        range,
        &|p| signed_distance(surface, p),
        bend_bound(curve, surface),
        tolerance.linear,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_geom::{Circle, Cone, Cylinder, Line, NurbsCurve, Plane, Sphere, Torus};
    use craft3d_math::Frame;
    use std::f64::consts::TAU;

    fn tilted(scale: f64) -> Frame {
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap()
    }

    fn surfaces(f: Frame, k: f64) -> Vec<Surface> {
        vec![
            Surface::Plane(Plane::new(f)),
            Surface::Cylinder(Cylinder::new(f, 2.0 * k).unwrap()),
            Surface::Cone(Cone::new(f, 1.5 * k, 0.4).unwrap()),
            Surface::Cone(Cone::new(f, 1.5 * k, -0.7).unwrap()),
            Surface::Sphere(Sphere::new(f, 3.0 * k).unwrap()),
            Surface::Torus(Torus::new(f, 4.0 * k, k).unwrap()),
        ]
    }

    #[test]
    fn the_signed_distance_is_zero_on_the_surface_and_no_steeper_than_one() {
        for k in [1e-3, 1.0, 1e6] {
            let f = tilted(k);
            for surface in surfaces(f, k) {
                for (u, v) in [(0.3, 0.2), (2.9, -0.4), (5.1, 0.9)] {
                    // Lengths scale with the model, angles do not.
                    let (u, v) = match &surface {
                        Surface::Plane(_) => (u * k, v * k),
                        Surface::Cylinder(_) | Surface::Cone(_) => (u, v * k),
                        _ => (u, v),
                    };
                    let on = surface.eval(u, v);
                    let d = signed_distance(&surface, on).unwrap();
                    assert!(d.abs() < 1e-12 * k, "{}: {d}", surface.kind());
                    // Moving off along the normal changes it by the step.
                    let n = surface.normal(u, v).unwrap();
                    let step = 0.05 * k;
                    let up = signed_distance(&surface, on + n * step).unwrap();
                    let down = signed_distance(&surface, on - n * step).unwrap();
                    assert!((up - step).abs() < 1e-9 * k, "{}: {up}", surface.kind());
                    assert!((down + step).abs() < 1e-9 * k, "{}", surface.kind());
                }
                // Never steeper than one, anywhere.
                let mut seed = 5_u64;
                let mut unit = || {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
                };
                for _ in 0..200 {
                    let p = f.point(unit() * 12.0 * k, unit() * 12.0 * k, unit() * 12.0 * k);
                    let q = p + Vec3::new(unit(), unit(), unit()) * k;
                    let (dp, dq) = (
                        signed_distance(&surface, p).unwrap(),
                        signed_distance(&surface, q).unwrap(),
                    );
                    assert!((dp - dq).abs() <= p.distance(q) * (1.0 + 1e-12));
                }
            }
        }
    }

    /// Every hit lies on both; crossings found by dense sampling are all
    /// among the hits.
    fn assert_hits(curve: &Curve3, range: (f64, f64), surface: &Surface, k: f64) -> usize {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let CurveIntersection::Hits(hits) = curve_hits(curve, range, surface, &tol).unwrap() else {
            panic!("{} on {}", curve.kind(), surface.kind());
        };
        for hit in &hits {
            assert!(hit.point.distance(curve.eval(hit.t)) < 1e-9 * k);
            let d = signed_distance(surface, hit.point).unwrap();
            assert!(d.abs() <= tol.linear, "{}: {d}", surface.kind());
        }
        let in_range: Vec<&CurveHit> = hits
            .iter()
            .filter(|h| h.t >= range.0 - 1e-9 && h.t <= range.1 + 1e-9)
            .collect();
        const N: u32 = 20_000;
        let at = |i: u32| range.0 + (range.1 - range.0) * f64::from(i) / f64::from(N);
        let mut crossings = 0;
        for i in 0..N {
            let (a, b) = (at(i), at(i + 1));
            let (fa, fb) = (
                signed_distance(surface, curve.eval(a)).unwrap(),
                signed_distance(surface, curve.eval(b)).unwrap(),
            );
            if (fa > 0.0) != (fb > 0.0) {
                crossings += 1;
                let width = b - a;
                assert!(
                    in_range
                        .iter()
                        .any(|h| h.t >= a - width && h.t <= b + width),
                    "{} on {}: the crossing at {a} was missed",
                    curve.kind(),
                    surface.kind()
                );
            }
        }
        let crossing_hits = in_range.iter().filter(|h| !h.tangent).count();
        assert_eq!(
            crossing_hits,
            crossings,
            "{} on {}",
            curve.kind(),
            surface.kind()
        );
        crossings
    }

    #[test]
    fn lines_circles_and_splines_meet_every_analytic_surface() {
        let mut total = 0;
        for k in [1e-3, 1.0, 1e6] {
            let f = tilted(k);
            let other = Frame::new(
                f.point(0.7 * k, 0.4 * k, 0.3 * k),
                Vec3::new(0.9, 0.3, 0.4),
                Vec3::new(0.0, 1.0, 0.1),
            )
            .unwrap();
            let line = Curve3::Line(
                Line::new(
                    f.point(-6.0 * k, 0.3 * k, 0.2 * k),
                    other.x() + other.z() * 0.2,
                )
                .unwrap(),
            );
            let circle = Curve3::Circle(Circle::new(other, 3.5 * k).unwrap());
            let spline = Curve3::Nurbs(NurbsCurve::circle(&other, 2.5 * k).unwrap());
            for surface in surfaces(f, k) {
                total += assert_hits(&line, (0.0, 14.0 * k), &surface, k);
                total += assert_hits(&circle, (0.0, TAU), &surface, k);
                total += assert_hits(&circle, (1.0, 4.0), &surface, k);
                total += assert_hits(&spline, (0.0, 1.0), &surface, k);
            }
        }
        // The cases are placed to cross: two per closed surface, mostly.
        assert!(total > 60, "{total}");
    }

    #[test]
    fn a_touch_is_tangent_and_a_curve_on_the_surface_is_on_it() {
        let tol = Tolerance::default();
        let torus = Surface::Torus(Torus::new(Frame::WORLD, 4.0, 1.0).unwrap());
        // Along the top of the torus: touches the circle of radius 4 at
        // height 1 in one point.
        let graze = Curve3::Line(Line::new(Vec3::new(-9.0, 4.0, 1.0), Vec3::X).unwrap());
        let CurveIntersection::Hits(hits) = curve_hits(&graze, (0.0, 18.0), &torus, &tol).unwrap()
        else {
            panic!("expected a touch");
        };
        assert_eq!(hits.len(), 1);
        assert!(hits[0].tangent);
        // (To the fourth order: where exactly is not well determined.)
        assert!(hits[0].point.distance(Vec3::new(0.0, 4.0, 1.0)) < 0.1);
        // Its outer equator lies on it.
        let equator = Curve3::Circle(Circle::new(Frame::WORLD, 5.0).unwrap());
        assert_eq!(
            curve_hits(&equator, (0.0, TAU), &torus, &tol).unwrap(),
            CurveIntersection::OnSurface
        );
        // A line that ends on a sphere: a hit at that end; whether it
        // would cross there the piece does not show, so it counts as a
        // touch.
        let ball = Surface::Sphere(Sphere::new(Frame::WORLD, 2.0).unwrap());
        let spoke = Curve3::Nurbs(
            NurbsCurve::polynomial(
                1,
                vec![0.0, 0.0, 1.0, 1.0],
                vec![Vec3::new(0.5, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)],
            )
            .unwrap(),
        );
        let CurveIntersection::Hits(hits) = curve_hits(&spoke, (0.0, 1.0), &ball, &tol).unwrap()
        else {
            panic!("expected a hit");
        };
        assert_eq!(hits.len(), 1);
        assert!(hits[0].tangent);
        assert!((hits[0].t - 1.0).abs() < 1e-6);
        // Through the apex of a cone, from outside to outside: a touch.
        let cone = Surface::Cone(Cone::new(Frame::WORLD, 1.0, -0.5).unwrap());
        let apex = Vec3::new(0.0, 0.0, 1.0 / 0.5_f64.tan());
        let past = Curve3::Line(Line::new(apex - Vec3::X * 3.0, Vec3::X).unwrap());
        let CurveIntersection::Hits(hits) = curve_hits(&past, (0.0, 6.0), &cone, &tol).unwrap()
        else {
            panic!("expected a touch");
        };
        assert_eq!(hits.len(), 1);
        assert!(hits[0].tangent);
        assert!(hits[0].point.distance(apex) < 1e-5);
        // A freeform surface is refused, not guessed.
        let patch = Surface::Bezier(
            craft3d_geom::BezierPatch::new(
                1,
                1,
                vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.5)],
            )
            .unwrap(),
        );
        assert!(matches!(
            curve_hits(&graze, (0.0, 18.0), &patch, &tol),
            Err(GeomError::Unsupported(_))
        ));
    }

    #[test]
    fn the_speed_bound_is_a_bound() {
        let f = tilted(1.0);
        let curves = [
            Curve3::Circle(Circle::new(f, 2.5).unwrap()),
            Curve3::Ellipse(craft3d_geom::Ellipse::new(f, 4.0, 0.5).unwrap()),
            Curve3::Nurbs(NurbsCurve::circle(&f, 2.5).unwrap()),
            Curve3::Nurbs(
                NurbsCurve::new(
                    2,
                    vec![0.0, 0.0, 0.0, 0.2, 1.0, 1.0, 1.0],
                    vec![
                        Vec3::ZERO,
                        Vec3::new(3.0, 1.0, 0.0),
                        Vec3::new(1.0, 4.0, 2.0),
                        Vec3::new(-2.0, 0.0, 1.0),
                    ],
                    vec![1.0, 3.0, 0.5, 1.0],
                )
                .unwrap(),
            ),
        ];
        for curve in &curves {
            let bound = speed_bound(curve);
            let (a, b) = curve.domain();
            let mut fastest = 0.0_f64;
            for i in 0..=2000 {
                let t = a + (b - a) * f64::from(i) / 2000.0;
                fastest = fastest.max(curve.derivatives(t).1.length());
            }
            assert!(
                fastest <= bound * (1.0 + 1e-12),
                "{}: {fastest} > {bound}",
                curve.kind()
            );
            // Not uselessly loose either.
            assert!(bound <= 40.0 * fastest, "{}: {bound}", curve.kind());
        }
    }
}
