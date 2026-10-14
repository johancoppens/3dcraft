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

use super::freeform::is_freeform;
use super::outline::Sheet;
use craft3d_geom::intersect::{CurveHit, CurveIntersection, intersect_curve_surface};
use craft3d_geom::{Curve3, GeomError, Surface};
use craft3d_math::{Bbox3, Tolerance, Vec3};

const FREEFORM: &str = "a freeform (Bézier or NURBS) surface that was not indexed for booleans (a bug in 3DCraft, please report it)";
const ALONG: &str = "a curve that runs along a surface within the tolerance without lying on it (at the edge of tolerance)";

/// The most evaluations one curve and surface may cost.
const MAX_EVALUATIONS: usize = 200_000;

/// A signed distance from `p` to the surface: zero on it, changing by at
/// most the distance moved, and of one sign on each side. For a cone it is
/// the distance to the cone continued past its apex as a line in the
/// half-plane through the axis, which is positive there and so has no zero
/// that the surface does not have. For a freeform surface the distance to
/// its nearest point through its index, and `None` without one.
pub(super) fn signed_distance(sheet: Sheet, p: Vec3) -> Option<f64> {
    let d = match sheet.surface {
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
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            return sheet.tree.and_then(|tree| tree.signed_distance(p));
        }
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
    let speed = speed_bound(curve);
    let turn = turn_bound(curve)?;
    let bound = speed * speed * curvature + turn;
    (bound.is_finite() && zone > 0.0).then_some((bound, zone))
}

/// How much the curve bends, at most: its second derivative per unit of
/// parameter squared. `None` where no finite bound comes out.
fn turn_bound(curve: &Curve3) -> Option<f64> {
    match curve {
        Curve3::Line(_) => Some(0.0),
        Curve3::Circle(c) => Some(c.radius()),
        Curve3::Ellipse(e) => Some(e.semi_axes().0.max(e.semi_axes().1)),
        Curve3::Nurbs(c) => {
            // The second derivative of a polynomial spline is a spline
            // over the second differences of its control points, each
            // scaled by the knot spans it straddles (*The NURBS Book*
            // 3.3), and lies in their hull.
            // A rational spline's second derivative is no spline, but it
            // is bounded through the nets of its Bézier pieces all the
            // same (`NurbsCurve::derivative_bounds`).
            if c.weights().iter().any(|&w| (w - 1.0).abs() > 1e-12) {
                return c.derivative_bounds().map(|(_, second)| second);
            }
            let (degree, knots, points) = (c.degree(), c.knots(), c.control_points());
            if degree < 2 {
                return Some(0.0);
            }
            let first: Vec<Vec3> = points
                .windows(2)
                .enumerate()
                .map(|(i, pair)| {
                    let (Some(&lo), Some(&hi), &[a, b]) =
                        (knots.get(i + 1), knots.get(i + degree + 1), pair)
                    else {
                        return Vec3::ZERO;
                    };
                    if hi > lo {
                        (b - a) * (degree as f64 / (hi - lo))
                    } else {
                        Vec3::ZERO
                    }
                })
                .collect();
            let mut bound = 0.0_f64;
            for (i, pair) in first.windows(2).enumerate() {
                let (Some(&lo), Some(&hi), &[a, b]) =
                    (knots.get(i + 2), knots.get(i + degree + 1), pair)
                else {
                    continue;
                };
                if hi > lo {
                    bound = bound.max((b - a).length() * ((degree - 1) as f64) / (hi - lo));
                }
            }
            bound.is_finite().then_some(bound)
        }
    }
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
    least(&|t| f(t).abs(), lo, hi)
}

/// Where a curve that touches the surface from the side `positive` comes
/// nearest to it or, where it dips through it by less than half the
/// tolerance on the way (which counts as touching, as for faces), goes
/// deepest: one place for the touch, between the two crossings it stands
/// for, where the nearest point to the surface would be either crossing,
/// whichever the search happens on. A dip deeper than that is left at a
/// crossing, on the surface, as before. Where the curve stays on its side
/// it is [`closest`].
fn deepest(f: &dyn Fn(f64) -> f64, lo: f64, hi: f64, positive: bool, tol: f64) -> f64 {
    let sign = if positive { 1.0 } else { -1.0 };
    let t = least(&|t| sign * f(t), lo, hi);
    if f(t).abs() <= 0.5 * tol {
        t
    } else {
        closest(f, lo, hi)
    }
}

/// Where `g` is least over `[lo, hi]`: the least of seventeen samples,
/// refined by a ternary search about it.
fn least(g: &dyn Fn(f64) -> f64, lo: f64, hi: f64) -> f64 {
    let f = g;
    const SAMPLES: u32 = 16;
    let at = |k: u32| lo + (hi - lo) * f64::from(k) / f64::from(SAMPLES);
    let mut best = (f64::INFINITY, lo);
    for k in 0..=SAMPLES {
        let value = f(at(k));
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
        if f(m1) <= f(m2) {
            b = m2;
        } else {
            a = m1;
        }
    }
    let t = 0.5 * (a + b);
    if f(t) <= best.0 { t } else { best.1 }
}

/// The hits of the piece of `curve` over `range` with the zero set of
/// `distance`.
/// `lower`, when given, is a bound below the distance that never
/// overestimates it (an index of a freeform surface has one, and the
/// distance it reports is only as near as its search got): a stretch is
/// called clear of the surface by the bound, never by the distance alone.
/// What the root isolation can ask of a surface, beyond the signed
/// distance to it: bounds that let it judge a whole stretch of the curve
/// at once. Each is optional; without them a stretch is judged by its
/// middle and how far the curve can travel from it.
struct Probe<'a> {
    /// The signed distance from a point to the surface; `None` where the
    /// surface has none (then the root isolation fails as unsupported).
    distance: &'a dyn Fn(Vec3) -> Option<f64>,
    /// A bound below the distance from a point, capped at a threshold
    /// (the second argument) that is all the asker needs it to exceed.
    lower: Option<&'a dyn Fn(Vec3, f64) -> f64>,
    /// A bound below the distance from every point of a stretch of the
    /// curve between two parameters, capped likewise.
    lower_stretch: Option<&'a dyn Fn(f64, f64, f64) -> f64>,
    /// Near the surface, how much the signed distance along the curve can
    /// bend per unit of parameter squared, and how near "near" is.
    bend: Option<(f64, f64)>,
    /// A proof that a stretch between two parameters is within the
    /// tolerance of the surface throughout.
    within: Option<&'a dyn Fn(f64, f64) -> bool>,
}

fn numeric(
    curve: &Curve3,
    range: (f64, f64),
    probe: Probe<'_>,
    tol: f64,
) -> Result<CurveIntersection, GeomError> {
    let Probe {
        distance,
        lower,
        lower_stretch,
        bend,
        within,
    } = probe;
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
        // The bound below the distance is asked for only where it can
        // prove the stretch clear, which the value in the middle must
        // already allow: near the surface it would be refined to the
        // finest resolution for nothing.
        let certain = match lower {
            Some(bound) if value.abs() - stray > tol => {
                bound(curve.eval(mid), 1.001 * (tol + stray)).min(value.abs())
            }
            _ => value.abs(),
        };
        // Clear: by its middle and how far the curve can travel from
        // there, or, off the surface in the middle, by a bound below the
        // whole stretch at once (a stretch that runs beside the surface
        // is then proven clear in one piece rather than in pieces the
        // size of its distance). Only a stretch clear at its ends and
        // quarters can be clear throughout, and looking there is far
        // cheaper than bounding a stretch that touches the surface
        // somewhere.
        // The look at the ends and quarters is by the bound below the
        // distance where there is one (cheap: a few boxes), else by the
        // distance itself.
        let off = |t: f64| match lower {
            Some(bound) => bound(curve.eval(t), 1.001 * tol) > tol,
            None => f(t).abs() > tol,
        };
        let clear = certain - stray > tol
            || (value.abs() > tol
                && lower_stretch.is_some_and(|bound| {
                    [a, 0.5 * (a + mid), 0.5 * (mid + b), b]
                        .into_iter()
                        .all(off)
                        && bound(a, b, tol) > tol
                }));
        // Near: within the tolerance by its middle and the travel, too
        // short to divide, or, on the surface in the middle, proven to
        // stay within the tolerance over the whole stretch (a stretch
        // that lies on the surface is then found in one piece rather
        // than in pieces the size of the tolerance).
        let near = value.abs() + stray <= tol
            || b - a <= finest
            || !(mid > a && mid < b)
            || (value.abs() <= tol && within.is_some_and(|proven| proven(a, b)));
        let kind = if clear {
            Kind::Clear {
                positive: value > 0.0,
            }
        } else if near {
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
            (Some(positive), Some(_)) => (deepest(&f, lo, hi, positive, tol), true),
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
    sheet: Sheet,
    tolerance: &Tolerance,
) -> Result<CurveIntersection, GeomError> {
    let surface = sheet.surface;
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
    if let Some(found) = coaxial(curve, sheet, tolerance.linear) {
        return Ok(found);
    }
    if let Some(found) = ruling(curve, range, sheet, tolerance) {
        return Ok(found);
    }
    if is_freeform(surface) {
        // The index gives the distance to the nearest point, and a bound
        // below it that a stretch is called clear by.
        let Some(tree) = sheet.tree else {
            return Err(GeomError::Unsupported(FREEFORM));
        };
        return curve_hits_freeform(curve, range, tree, tolerance);
    }
    numeric(
        curve,
        range,
        Probe {
            distance: &|p| signed_distance(sheet, p),
            lower: None,
            lower_stretch: None,
            bend: bend_bound(curve, surface),
            within: None,
        },
        tolerance.linear,
    )
}

/// A circle about the axis of a cone, a sphere or a torus is at one
/// distance from it all round: on it or clear of it, decided by one point.
/// The numeric path cannot settle such a circle when it runs a few
/// tolerances from a cone (whose distance has no bound on its bend, so
/// that the stretches it is cut into must each be shorter than that
/// distance; two cones whose rims are ten tolerances apart). A circle that
/// is only nearly about the axis is within `δ`, its centre's distance from
/// the axis plus its radius times its tilt, of one that is (each point
/// moves by no more), and the distance to the surface by no more than
/// that: decided here only when `δ` leaves no doubt, and left to the
/// numeric path otherwise. `None` for every other curve and surface.
fn coaxial(curve: &Curve3, sheet: Sheet, tol: f64) -> Option<CurveIntersection> {
    let Curve3::Circle(circle) = curve else {
        return None;
    };
    let frame = circle.frame();
    let (origin, axis) = match sheet.surface {
        Surface::Cone(s) => (s.frame().origin(), s.frame().z()),
        Surface::Torus(s) => (s.frame().origin(), s.frame().z()),
        // Every line through the centre is an axis of a sphere.
        Surface::Sphere(s) => (s.frame().origin(), frame.z()),
        _ => return None,
    };
    let (n, radius) = (frame.z(), circle.radius());
    let tilt = n.cross(axis).length().atan2(n.dot(axis).abs());
    let offset = frame.origin() - origin;
    let along = axis * offset.dot(axis);
    let off_axis = (offset - along).length();
    let delta = off_axis + radius * tilt;
    if !(delta.is_finite() && delta <= 0.25 * tol) {
        return None;
    }
    // A point of the circle about the axis at the same height and radius.
    let across = frame.x() - axis * frame.x().dot(axis);
    let point = origin + along + across.normalized()? * radius;
    let distance = signed_distance(sheet, point)?.abs();
    if !distance.is_finite() {
        return None;
    }
    if distance - delta > tol {
        Some(CurveIntersection::Hits(Vec::new()))
    } else if distance + delta <= tol {
        Some(CurveIntersection::OnSurface)
    } else {
        None
    }
}

/// A line along a ruling of a cone, through its apex: on the cone on the
/// side of the apex where the cone is, and leaving it at the apex. Its
/// distance there is within the tolerance over the whole stretch, which
/// the numeric path can only cut into pieces the size of the tolerance
/// (a cone's distance has no bound on its bend), and gives up on: a
/// straight edge of one solid that lies along a cone's ruling (the corpus
/// case contact/cone-dome/002). The piece is on the surface when it lies
/// wholly on the cone's side, and meets it at the apex, a touch, when the
/// apex is inside it; on the other side it is left to the numeric path,
/// which settles a line that moves away from the surface. Decided only
/// when the ruling and the line are within a quarter of the tolerance of
/// each other over the piece. `None` otherwise.
fn ruling(
    curve: &Curve3,
    range: (f64, f64),
    sheet: Sheet,
    tolerance: &Tolerance,
) -> Option<CurveIntersection> {
    let (Curve3::Line(line), Surface::Cone(cone)) = (curve, sheet.surface) else {
        return None;
    };
    let tol = tolerance.linear;
    let (frame, alpha) = (cone.frame(), cone.half_angle());
    let tangent = alpha.tan();
    if !(tangent.is_finite() && tangent != 0.0) {
        return None;
    }
    let apex = frame.point(0.0, 0.0, -cone.radius() / tangent);
    // From the apex into the cone.
    let into = frame.z() * alpha.signum();
    let (o, d) = (line.origin(), line.direction());
    let at = (apex - o).dot(d);
    let (t0, t1) = range;
    if !(at.is_finite() && t0.is_finite() && t1.is_finite() && t1 > t0) {
        return None;
    }
    // The way along the line that goes into the cone from the apex.
    let way = if d.dot(into) >= 0.0 { 1.0 } else { -1.0 };
    let ray = d * way;
    // How far the ruling with the line's direction from the apex is from
    // the line over the piece: the apex's distance from the line, and the
    // angle between the line and the cone (from the axis) times the reach.
    let miss = (o + d * at).distance(apex);
    let angle = (ray.dot(into).clamp(-1.0, 1.0).acos() - alpha.abs()).abs();
    let reach = (t0 - at).abs().max((t1 - at).abs());
    // (False for NaN as well.)
    let close = miss + angle * reach <= 0.25 * tol;
    if !close {
        return None;
    }
    // The parameters on the cone's side of the apex.
    let on = |t: f64| (t - at) * way >= 0.0;
    if on(t0) && on(t1) {
        return Some(CurveIntersection::OnSurface);
    }
    if !on(t0) && !on(t1) {
        return None;
    }
    let point = apex;
    Some(CurveIntersection::Hits(vec![CurveHit {
        t: at,
        point,
        tangent: true,
    }]))
}

/// The root isolation against an indexed freeform surface.
fn curve_hits_freeform(
    curve: &Curve3,
    range: (f64, f64),
    tree: &std::sync::Arc<super::freeform::PatchTree>,
    tolerance: &Tolerance,
) -> Result<CurveIntersection, GeomError> {
    // Where the curve's bend is bounded, a whole stretch is judged at
    // once: one beside the surface by the distance from its chord to
    // the surface less how far the curve can leave the chord, and
    // one on the surface by its ends and how much it and the surface
    // can bend between them.
    let turn = turn_bound(curve);
    let lower_stretch = turn.map(|turn| {
        move |a: f64, b: f64, needed: f64| {
            let fat = 0.125 * turn * (b - a) * (b - a);
            tree.segment_lower_bound(
                curve.eval(a),
                curve.eval(b),
                0.25 * tolerance.linear,
                1.001 * (needed + fat),
            ) - fat
        }
    });
    let within = turn.map(|turn| {
        move |a: f64, b: f64| {
            tree.stretch_within(
                curve.eval(a),
                curve.eval(b),
                turn * (b - a) * (b - a),
                tolerance.linear,
            )
        }
    });
    numeric(
        curve,
        range,
        Probe {
            distance: &|p| tree.signed_distance(p),
            lower: Some(&|p, threshold| tree.lower_bound(p, 0.25 * tolerance.linear, threshold)),
            lower_stretch: lower_stretch
                .as_ref()
                .map(|l| l as &dyn Fn(f64, f64, f64) -> f64),
            bend: None,
            within: within.as_ref().map(|w| w as &dyn Fn(f64, f64) -> bool),
        },
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
                    let d = signed_distance(Sheet::bare(&surface), on).unwrap();
                    assert!(d.abs() < 1e-12 * k, "{}: {d}", surface.kind());
                    // Moving off along the normal changes it by the step.
                    let n = surface.normal(u, v).unwrap();
                    let step = 0.05 * k;
                    let up = signed_distance(Sheet::bare(&surface), on + n * step).unwrap();
                    let down = signed_distance(Sheet::bare(&surface), on - n * step).unwrap();
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
                        signed_distance(Sheet::bare(&surface), p).unwrap(),
                        signed_distance(Sheet::bare(&surface), q).unwrap(),
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
        let CurveIntersection::Hits(hits) =
            curve_hits(curve, range, Sheet::bare(surface), &tol).unwrap()
        else {
            panic!("{} on {}", curve.kind(), surface.kind());
        };
        for hit in &hits {
            assert!(hit.point.distance(curve.eval(hit.t)) < 1e-9 * k);
            let d = signed_distance(Sheet::bare(surface), hit.point).unwrap();
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
                signed_distance(Sheet::bare(surface), curve.eval(a)).unwrap(),
                signed_distance(Sheet::bare(surface), curve.eval(b)).unwrap(),
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
        let CurveIntersection::Hits(hits) =
            curve_hits(&graze, (0.0, 18.0), Sheet::bare(&torus), &tol).unwrap()
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
            curve_hits(&equator, (0.0, TAU), Sheet::bare(&torus), &tol).unwrap(),
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
        let CurveIntersection::Hits(hits) =
            curve_hits(&spoke, (0.0, 1.0), Sheet::bare(&ball), &tol).unwrap()
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
        let CurveIntersection::Hits(hits) =
            curve_hits(&past, (0.0, 6.0), Sheet::bare(&cone), &tol).unwrap()
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
            curve_hits(&graze, (0.0, 18.0), Sheet::bare(&patch), &tol),
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

    #[test]
    fn a_line_that_runs_on_a_flat_patch_and_beyond_it_is_found_in_one_piece() {
        use super::super::freeform::PatchTree;
        use craft3d_geom::BezierPatch;
        use std::sync::Arc;
        // z = 0 over [0, 1]²; beyond its border the surface ends.
        let patch = BezierPatch::new(
            1,
            1,
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
            ],
        )
        .unwrap();
        let surface = Surface::Bezier(patch);
        let tree = Arc::new(PatchTree::new(&surface).unwrap());
        let sheet = Sheet {
            surface: &surface,
            tree: Some(&tree),
        };
        let line = Curve3::Line(Line::new(Vec3::new(-1.0, 0.5, 0.0), Vec3::X).unwrap());
        let tol = Tolerance::new(1e-6, 1e-9).unwrap();
        // On the surface for a third of its range and clear of it on both
        // sides of that: a touch. Taken a tolerance at a time the stretch
        // on the surface would be a million pieces, and used to be refused
        // as running along the surface.
        let result = curve_hits(&line, (0.0, 3.0), sheet, &tol).unwrap();
        let CurveIntersection::Hits(hits) = result else {
            panic!("expected one touch, got {result:?}");
        };
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].tangent);
        assert!((0.99..=2.01).contains(&hits[0].t), "{}", hits[0].t);
    }

    #[test]
    fn a_line_grazing_a_flat_patch_is_proven_clear_in_one_piece() {
        use super::super::freeform::PatchTree;
        use craft3d_geom::BezierPatch;
        use std::sync::Arc;
        let patch = BezierPatch::new(
            1,
            1,
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
            ],
        )
        .unwrap();
        let surface = Surface::Bezier(patch);
        let tree = Arc::new(PatchTree::new(&surface).unwrap());
        let sheet = Sheet {
            surface: &surface,
            tree: Some(&tree),
        };
        let tol = Tolerance::new(1e-6, 1e-9).unwrap();
        // Three tolerances above the patch, across all of it: clear, and
        // found to be in a few stretches rather than a million.
        let graze = Curve3::Line(Line::new(Vec3::new(-1.0, 0.5, 3e-6), Vec3::X).unwrap());
        assert_eq!(
            curve_hits(&graze, (0.0, 3.0), sheet, &tol).unwrap(),
            CurveIntersection::Hits(Vec::new())
        );
        // And a crossing beside the grazing stretch is still found.
        let dip = Curve3::Line(
            Line::new(Vec3::new(-1.0, 0.5, 3e-6), Vec3::new(1.0, 0.0, -2e-6)).unwrap(),
        );
        let CurveIntersection::Hits(hits) = curve_hits(&dip, (0.0, 3.0), sheet, &tol).unwrap()
        else {
            panic!("expected a crossing");
        };
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(!hits[0].tangent);
        assert!((hits[0].t - 1.5).abs() < 1e-3, "{}", hits[0].t);
    }

    #[test]
    fn the_turn_bound_of_a_spline_holds_at_samples() {
        let curve = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.3, 1.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::ZERO,
                Vec3::new(3.0, 1.0, 0.0),
                Vec3::new(1.0, 4.0, 2.0),
                Vec3::new(5.0, 5.0, -1.0),
                Vec3::new(6.0, 0.0, 3.0),
                Vec3::new(8.0, 2.0, 2.0),
            ],
            vec![1.0; 6],
        )
        .unwrap();
        let bound = turn_bound(&Curve3::Nurbs(curve.clone())).unwrap();
        assert!(bound > 0.0);
        let h = 1e-5;
        let mut steepest = 0.0_f64;
        for k in 1..1000 {
            let t = f64::from(k) / 1000.0;
            let second = (curve.derivatives(t + h).1 - curve.derivatives(t - h).1) * (0.5 / h);
            steepest = steepest.max(second.length());
        }
        assert!(steepest <= bound * (1.0 + 1e-6), "{steepest} > {bound}");
        assert!(
            steepest > 0.2 * bound,
            "a bound {bound} far above {steepest}"
        );
        // A rational spline has one too, from the nets of its Bézier
        // pieces (it had none until rational faces came in).
        let rational = NurbsCurve::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::ZERO,
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
            ],
            vec![1.0, 0.5, 1.0],
        )
        .unwrap();
        let bound = turn_bound(&Curve3::Nurbs(rational.clone())).unwrap();
        let mut steepest = 0.0_f64;
        for k in 1..1000 {
            let t = f64::from(k) / 1000.0;
            let second =
                (rational.derivatives(t + h).1 - rational.derivatives(t - h).1) * (0.5 / h);
            steepest = steepest.max(second.length());
        }
        assert!(steepest <= bound * (1.0 + 1e-6), "{steepest} > {bound}");
        assert!(
            steepest > 0.5 * bound,
            "a bound {bound} far above {steepest}"
        );
        // And the speed bound of a rational spline holds as well.
        let speed = speed_bound(&Curve3::Nurbs(rational.clone()));
        for k in 0..=1000 {
            let fast = rational.derivatives(f64::from(k) / 1000.0).1.length();
            assert!(fast <= speed, "{fast} > {speed}");
        }
    }
}
