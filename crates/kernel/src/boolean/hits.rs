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
use super::poly::Poly;
use craft3d_geom::intersect::{CurveHit, CurveIntersection, intersect_curve_surface};
use craft3d_geom::{Curve3, GeomError, Surface};
use craft3d_math::Portable;
use craft3d_math::{Bbox3, Tolerance, Vec3};

const FREEFORM: &str = "a freeform (Bézier or NURBS) surface that was not indexed for booleans (a bug in 3DCraft, please report it)";
const ALONG: &str = "a curve that runs along a surface within the tolerance without lying on it (at the edge of tolerance)";

/// The most evaluations one curve and surface may cost.
const MAX_EVALUATIONS: usize = 200_000;
/// How much narrower than the distance allows a window cleared about a
/// stretch's middle is kept, for the rounding of the distance and of the
/// parameters (relative).
const WINDOW_MARGIN: f64 = 1e-9;
/// A stretch of a curve about a middle, clear of the surface on the side
/// given (positive: the side its normal points to): the pieces halved
/// into it need no look ([`numeric`]).
type Window = (f64, f64, bool);
/// What part of the distance in the middle a bound below it is asked to
/// reach for a window (the bound is refined no further than that).
const WINDOW_ASKED: f64 = 0.9;
/// A stretch whose window would take more than this part of half of it
/// is not bounded as a whole first (see `numeric`).
const STRETCH_BEFORE_WINDOW: f64 = 0.5;

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
            x.phypot(y) - s.radius()
        }
        Surface::Cone(s) => {
            let (x, y, z) = s.frame().local(p);
            let (sine, cosine) = s.half_angle().psin_cos();
            (x.phypot(y) - s.radius()) * cosine - z * sine
        }
        Surface::Sphere(s) => p.distance(s.frame().origin()) - s.radius(),
        Surface::Torus(s) => {
            let (x, y, z) = s.frame().local(p);
            let (major, minor) = s.radii();
            (x.phypot(y) - major).phypot(z) - minor
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            return sheet.tree.and_then(|tree| tree.signed_distance(p));
        }
    };
    Some(d)
}

/// An upper bound on the speed of the curve: the length travelled per unit
/// of parameter.
pub(crate) fn speed_bound(curve: &Curve3) -> f64 {
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
pub(super) fn turn_bound(curve: &Curve3) -> Option<f64> {
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

/// The parameter in `[lo, hi]` where `f` is zero, given that it changes
/// sign between them and is monotone there, by the Illinois variant of
/// false position (a secant kept within the bracket the signs leave, the
/// end that stays put halved in weight so that both ends move), halving
/// where a step would leave the bracket: the place with the least `|f|`
/// it met. A few steps where bisection takes sixty; the caller holds the
/// answer to the distance and bisects where it is not near enough.
fn false_position(f: &dyn Fn(f64) -> f64, lo: f64, hi: f64) -> Option<f64> {
    let (mut a, mut b) = (lo, hi);
    let (mut fa, mut fb) = (f(a), f(b));
    if !(fa.is_finite() && fb.is_finite()) || fa.signum() == fb.signum() {
        return None;
    }
    let mut best = if fa.abs() <= fb.abs() {
        (fa.abs(), a)
    } else {
        (fb.abs(), b)
    };
    let mut kept = 0_i8;
    for _ in 0..60 {
        let secant = (a * fb - b * fa) / (fb - fa);
        let mid = 0.5 * (a + b);
        if !(mid > a && mid < b) {
            break;
        }
        let x = if secant > a && secant < b {
            secant
        } else {
            mid
        };
        let fx = f(x);
        if !fx.is_finite() {
            return None;
        }
        if fx.abs() < best.0 {
            best = (fx.abs(), x);
        }
        if fx == 0.0 {
            break;
        }
        if fx.signum() == fb.signum() {
            (b, fb) = (x, fx);
            if kept == 1 {
                fa *= 0.5;
            }
            kept = 1;
        } else {
            (a, fa) = (x, fx);
            if kept == -1 {
                fb *= 0.5;
            }
            kept = -1;
        }
    }
    Some(best.1)
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
    /// The surface ends (a freeform patch): a curve may lie on it for a
    /// while and leave it where it ends.
    bounded: bool,
}

/// How long, in tolerances, a stretch within the tolerance of a surface
/// that ends must be (and on it at its middle and quarters) to be taken
/// for the curve lying on the surface, and cut where it leaves it rather
/// than touched somewhere along it.
const LIES_ALONG: f64 = 64.0;

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
        bounded,
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
    // Stretches still to judge, and stretches already judged clear on the
    // way (see the window below), taken from the left.
    let mut stack: Vec<(f64, f64, Vec<Window>)> = vec![(start, end, Vec::new())];
    let mut evaluations = 0_usize;
    while let Some((a, b, windows)) = stack.pop() {
        // Within a window cleared about the middle of a stretch it was
        // halved from: clear, on that middle's side, unevaluated.
        if let Some(&(_, _, positive)) = windows.iter().find(|w| w.0 <= a && b <= w.1) {
            let kind = Kind::Clear { positive };
            match runs.last_mut() {
                Some(last) if last.2 == kind => last.1 = b,
                _ => runs.push((a, b, kind)),
            }
            continue;
        }
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
        // Where the curve cannot come within the tolerance of the surface
        // while it stays nearer to `mid` than the distance there less the
        // tolerance (over its speed): that window is clear, on the side of
        // the middle, by the same reasoning that clears a whole stretch.
        // A bound below the distance is asked where the distance itself
        // may be an overestimate. Asked once, where it is needed.
        let window = std::cell::OnceCell::new();
        let reach = || {
            *window.get_or_init(|| {
                let below = if value.abs() > tol {
                    match lower {
                        Some(bound) => {
                            bound(curve.eval(mid), WINDOW_ASKED * value.abs()).min(value.abs())
                        }
                        None => value.abs(),
                    }
                } else {
                    0.0
                };
                // Less what rounding can move the window's ends by.
                (below - tol) / speed * (1.0 - WINDOW_MARGIN) - 4.0 * f64::EPSILON * mid.abs()
            })
        };
        // The bound below a whole stretch is asked only where such a
        // window would leave most of it: where it would take most of it,
        // the curve is far from the surface for its length, and a
        // crossing beside it (where the bound below the stretch fails
        // after all its work) is the likelier case.
        let clear = certain - stray > tol
            || (value.abs() > tol
                && (reach().is_nan() || reach() <= STRETCH_BEFORE_WINDOW * half)
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
            // Not clear as a whole: halved. Against a surface with an
            // index (a freeform one, whose distance is a search and dear)
            // the window about the middle is kept with the halves, so
            // that the pieces they are halved into that lie within it are
            // clear without a look: where the curve crosses the surface,
            // the stretches beside the crossing cost nothing, where each
            // took a distance and often a bound below the whole stretch
            // that failed (docs/design/booleans.md › *A crossing closed
            // in on*). The halving itself is as before, so the runs near
            // the surface are the ones it always made: a first version
            // cut the window out, which moved their ends and turned
            // near-tangent passages at the edge of tolerance (a corpus
            // case, a fillet's tool) into refusals.
            let reach = reach();
            // The halves keep the windows of the stretches they come from
            // that reach into them, and this one's.
            let mut kept: Vec<Window> =
                windows.into_iter().filter(|w| w.0 < b && a < w.1).collect();
            if lower.is_some() && reach.is_finite() && reach > 0.0 {
                kept.push((mid - reach, mid + reach, value > 0.0));
            }
            stack.push((mid, b, kept.clone()));
            stack.push((a, mid, kept));
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
    let hits = hits_of_runs(curve, &runs, &f, bounded, tol, None);
    if failed.get() {
        return Err(GeomError::Unsupported(FREEFORM));
    }
    Ok(CurveIntersection::Hits(hits))
}

/// The hits a division of a piece of `curve` into runs gives (each run
/// clear of the surface on one side, or near it), with `f` the signed
/// distance along the curve: a near run between clear runs on opposite
/// sides is a crossing, on one side a touch, and one at an end of the
/// piece a crossing only when it crosses on the way. `bounded`: the
/// surface ends, and a long near run is the curve lying on it and cut
/// where it leaves (see [`Probe::bounded`]).
fn hits_of_runs(
    curve: &Curve3,
    runs: &[(f64, f64, Kind)],
    f: &dyn Fn(f64) -> f64,
    bounded: bool,
    tol: f64,
    level: Option<&Level<'_>>,
) -> Vec<CurveHit> {
    // Where the distance is zero on a stretch it crosses zero on: in
    // closed form where there is one, else by bisection.
    let root = |f: &dyn Fn(f64) -> f64, lo: f64, hi: f64| {
        level
            .and_then(|solve| solve(0.0, lo, hi))
            .unwrap_or_else(|| root(f, lo, hi))
    };
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
        // A curve that lies on a surface that ends (an edge of one body
        // along a patch of the other, past where the patch ends) is cut
        // where it leaves it: at each end of the stretch that is not an
        // end of the piece, the last place within a quarter of the
        // tolerance before it is clear. (Somewhere along the stretch,
        // as a touch, would be a place nothing else is.)
        let long = curve.eval(lo).distance(curve.eval(hi)) > LIES_ALONG * tol;
        if bounded
            && long
            && [0.25, 0.5, 0.75]
                .iter()
                .all(|&k| f(lo + (hi - lo) * k).abs() <= 0.25 * tol)
        {
            let leave = |inside: f64, outside: f64| {
                let (mut a, mut b) = (inside, outside);
                for _ in 0..80 {
                    let mid = 0.5 * (a + b);
                    if !(mid != a && mid != b) {
                        break;
                    }
                    if f(mid).abs() <= 0.25 * tol {
                        a = mid;
                    } else {
                        b = mid;
                    }
                }
                a
            };
            let middle = 0.5 * (lo + hi);
            let mut ends = Vec::new();
            if before.is_some()
                && let Some(&(clear_lo, _, _)) = index.checked_sub(1).and_then(|i| runs.get(i))
            {
                ends.push(leave(middle, clear_lo));
            }
            if after.is_some()
                && let Some(&(_, clear_hi, _)) = runs.get(index + 1)
            {
                ends.push(leave(middle, clear_hi));
            }
            // (Touches: the curve does not pass through the surface there,
            // and a ray that runs along a face is no ray to count by.)
            for t in ends {
                let point = curve.eval(t);
                if point.is_finite() && f(t).abs() <= tol {
                    hits.push(CurveHit {
                        t,
                        point,
                        tangent: true,
                    });
                }
            }
            continue;
        }
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
    hits
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
    if let Some(turns) = turning_points(curve, surface) {
        let level = levels(curve, surface);
        if let Some(found) = monotone(
            curve,
            range,
            sheet,
            &turns,
            level.as_ref().map(|l| l.as_ref()),
            tolerance.linear,
            false,
        ) {
            return Ok(found);
        }
    }
    if let Some(found) = by_quartic(curve, range, sheet, tolerance.linear) {
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
            bounded: false,
        },
        tolerance.linear,
    )
}

/// Where the signed distance from `surface` along `curve` may turn
/// (have a zero derivative or none), in closed form, for the pairs that
/// have one: along a line or a circle the squared distance to a sphere's
/// centre is a quadratic, or `h + a cos t + b sin t`, whose turning points
/// are its vertex, or `atan2(b, a)` and the angle opposite; along a line
/// the distance from a cone is `cos α ρ(t) − sin α z(t)` (less a
/// constant), `ρ` the root of a quadratic and `z` linear, which turns
/// where `cos² α (A t + B)² = sin² α z'² (A t² + 2 B t + C)` (a quadratic;
/// a root of the squared equation that is no turning point only adds a
/// place to divide at) and has no derivative where the line meets the
/// axis (the vertex of `ρ²`). Parameters on the curve as it is (a circle's
/// angles taken once; the caller lifts them into the piece). `None` for
/// every other pair, whose roots are isolated numerically.
fn turning_points(curve: &Curve3, surface: &Surface) -> Option<Vec<f64>> {
    match (curve, surface) {
        (Curve3::Line(line), Surface::Sphere(sphere)) => {
            let w = line.origin() - sphere.frame().origin();
            Some(vec![-w.dot(line.direction())])
        }
        (Curve3::Circle(circle), Surface::Sphere(sphere)) => {
            let f = circle.frame();
            let w = f.origin() - sphere.frame().origin();
            let (a, b) = (w.dot(f.x()), w.dot(f.y()));
            if a == 0.0 && b == 0.0 {
                // At one distance all round: nowhere to divide.
                return Some(Vec::new());
            }
            let phase = b.patan2(a);
            Some(vec![phase, phase + std::f64::consts::PI])
        }
        (Curve3::Line(line), Surface::Cone(cone)) => {
            let frame = cone.frame();
            let (x0, y0, _) = frame.local(line.origin());
            let d = line.direction();
            let (dx, dy, dz) = (d.dot(frame.x()), d.dot(frame.y()), d.dot(frame.z()));
            let a = dx * dx + dy * dy;
            if a.is_nan() || a <= 0.0 {
                // Along the axis: the distance is linear.
                return Some(Vec::new());
            }
            // Measured from the line's point nearest the axis (`near`),
            // where `ρ² = A s² + C` with `C` the squared distance there,
            // worked out from that point (not as a difference of large
            // squares when the line's origin is far away).
            let near = -(x0 * dx + y0 * dy) / a;
            let (px, py) = (x0 + near * dx, y0 + near * dy);
            let c = px * px + py * py;
            let (sine, cosine) = cone.half_angle().psin_cos();
            let (k, m) = (cosine * cosine, sine * sine * dz * dz);
            // `cos² α (A s)² = sin² α z'² (A s² + C)`: `s² A (k A − m) = m C`.
            let mut turns = vec![near];
            let (top, bottom) = (m * c, a * (k * a - m));
            if bottom > 0.0 && top >= 0.0 {
                let s = (top / bottom).sqrt();
                if s.is_finite() {
                    turns.extend([near - s, near + s]);
                }
            }
            Some(turns)
        }
        _ => None,
    }
}

/// How many stretches [`by_quartic`] halves, at most, to prove a piece
/// clear of the surface before it looks for the turning points.
const CLEAR_LOOKS: usize = 8;

/// The hits of the piece of an ellipse with a ball, or of a line with a
/// torus, from the turning points of the distance along it
/// ([`quartic_turns`]) and [`monotone`] runs, the places where it
/// passes a level found by false position ([`false_position`], held to
/// the distance as closed forms are, bisection where it is not near
/// enough). A piece whose stretches each have their middle further from
/// the surface than half their length (by the curve's speed bound) and
/// two tolerances meets nothing, and is answered at once. `None` for every other pair, and
/// where a stretch between the turning points is not monotone (the root
/// isolation then answers).
fn by_quartic(
    curve: &Curve3,
    range: (f64, f64),
    sheet: Sheet,
    tol: f64,
) -> Option<CurveIntersection> {
    if !matches!(
        (curve, sheet.surface),
        (Curve3::Ellipse(_), Surface::Sphere(_)) | (Curve3::Line(_), Surface::Torus(_))
    ) {
        return None;
    }
    let f = |t: f64| signed_distance(sheet, curve.eval(t)).unwrap_or(f64::NAN);
    // Clear, stretch by stretch, halving at most [`CLEAR_LOOKS`] times.
    // (Beyond two tolerances and what rounding could be, as `monotone`
    // reads the distance.)
    let speed = speed_bound(curve);
    let size = [range.0, range.1]
        .into_iter()
        .map(|t| curve.eval(t).length())
        .fold(surface_size(sheet.surface), f64::max);
    let margin = 2.0 * tol + ROUNDING * f64::EPSILON * size;
    let mut stretches = vec![range];
    let (mut clear, mut looks) = (true, 0);
    while let Some((a, b)) = stretches.pop() {
        let half = 0.5 * (b - a) * speed;
        if f(0.5 * (a + b)).abs() > half + margin && half.is_finite() {
            continue;
        }
        looks += 1;
        if looks > CLEAR_LOOKS {
            clear = false;
            break;
        }
        let mid = 0.5 * (a + b);
        stretches.extend([(a, mid), (mid, b)]);
    }
    if clear {
        return Some(CurveIntersection::Hits(Vec::new()));
    }
    let turns = quartic_turns(curve, sheet.surface, range)?;
    let solve = |level: f64, lo: f64, hi: f64| false_position(&|t| f(t) - level, lo, hi);
    monotone(curve, range, sheet, &turns, Some(&solve), tol, true)
}

/// Where the signed distance may turn along an ellipse from a ball, or
/// along a line from a torus, whose turning points are the roots of a
/// quartic: places where the quartic may change sign ([`Poly::changes`],
/// from the roots of its derivatives down, no root formula), with the
/// places where the distance has no derivative. Extra places only divide
/// the curve where nothing turns. [`monotone`] holds every stretch
/// between them to the distance itself and leaves the pair to the root
/// isolation where it is not monotone. `None` for every other pair.
///
/// - An ellipse `c + a cos t x + b sin t y` from a ball about `o`: with
///   `w = c − o`, `p = w·x`, `q = w·y`, half the derivative of the
///   squared distance is `(b² − a²) sin t cos t − a p sin t + b q cos t`,
///   which times `(1 + u²)²`, `u = tan(t/2)`, is the quartic
///   `−C u⁴ + 2(B − A) u³ + 2(A + B) u + C` (`A = b² − a²`, `B = −a p`,
///   `C = b q`), taken for `|t| ≤ π/2` (`|u| ≤ 1`) and, with `B` and `C`
///   turned over, for the half turn from `π/2` to `3π/2`.
/// - A line from a torus of radii `R`, `r`: the distance is
///   `sqrt(D) − r`, `D = (ρ − R)² + z²`, monotone where `D` is. Measured
///   from a base point of the line (`s` from there, `ρ² = A s² + 2 B s +
///   C`, `z = z₁ + z' s`), `D'/2 = k s + n − R (A s + B) / ρ` (`k = A +
///   z'²`, `n = B + z' z₁`), with no derivative where the line meets the
///   axis (`ρ = 0`, at its point nearest the axis, given). Its roots are
///   roots of `(k s + n)² (A s² + 2 B s + C) − R² (A s + B)²`, a quartic
///   whose other roots (of `(k s + n) ρ + R (A s + B)`) only add places;
///   the two factors vanish together only where `A s + B = 0` (the point
///   nearest the axis) and `k s + n = 0` or `ρ = 0`, and as the line
///   turns towards the axis their roots close in on `k s + n = 0` (where
///   `z = 0` along the axis): both points are given. The base point is the
///   point nearest the axis, or the end of the piece nearest to it (a
///   line nearly along the axis has its nearest point far away), so that
///   the coefficients are worked out from small numbers.
fn quartic_turns(curve: &Curve3, surface: &Surface, range: (f64, f64)) -> Option<Vec<f64>> {
    match (curve, surface) {
        (Curve3::Ellipse(ellipse), Surface::Sphere(sphere)) => {
            let f = ellipse.frame();
            let w = f.origin() - sphere.frame().origin();
            let (a, b) = ellipse.semi_axes();
            let (p, q) = (w.dot(f.x()), w.dot(f.y()));
            // (The products with `p` and `q` are as uncertain as the
            // numbers `w` was worked out from make them, whatever their
            // size: the centres far from the origin.)
            let reach = w.length() + f.origin().length() + sphere.frame().origin().length();
            let (big_a, size_a) = (b * b - a * a, b * b + a * a);
            let (size_b, size_c) = (a * reach, b * reach);
            let mut turns = Vec::new();
            for (shift, sign) in [(0.0, 1.0), (std::f64::consts::PI, -1.0)] {
                let (big_b, big_c) = (-sign * a * p, sign * b * q);
                let poly = Poly {
                    c: [
                        big_c,
                        2.0 * (big_a + big_b),
                        0.0,
                        2.0 * (big_b - big_a),
                        -big_c,
                    ],
                    size: [
                        size_c,
                        2.0 * (size_a + size_b),
                        0.0,
                        2.0 * (size_a + size_b),
                        size_c,
                    ],
                };
                turns.extend(
                    poly.changes(-1.0, 1.0)
                        .into_iter()
                        .map(|u| shift + 2.0 * u.patan()),
                );
            }
            Some(turns)
        }
        (Curve3::Line(line), Surface::Torus(torus)) => {
            let frame = torus.frame();
            let (x0, y0, z0) = frame.local(line.origin());
            let d = line.direction();
            let (dx, dy, dz) = (d.dot(frame.x()), d.dot(frame.y()), d.dot(frame.z()));
            let (major, _) = torus.radii();
            let a = dx * dx + dy * dy;
            let k = a + dz * dz;
            // Measured from the line's point nearest the axis, as for a
            // cone ([`turning_points`]), or from the end of the piece
            // nearest to it where it lies beyond (a line nearly along the
            // axis has its nearest point far away, or none).
            let (start, end) = range;
            let near = -(x0 * dx + y0 * dy) / a;
            let base = if near.is_finite() {
                near.clamp(start, end)
            } else {
                0.5 * (start + end)
            };
            if !base.is_finite() {
                return None;
            }
            let (px, py) = (x0 + base * dx, y0 + base * dy);
            let z1 = z0 + base * dz;
            let (b, c) = (px * dx + py * dy, px * px + py * py);
            let n = b + dz * z1;
            // The sizes: the point's coordinates are as uncertain as the
            // numbers they were worked out from (the line's origin may be
            // far away), so they are taken that large.
            let far = x0.abs().max(y0.abs()).max(z0.abs()).max(base.abs());
            let reach = c.sqrt() + far;
            let (b_size, c_size) = (reach, reach * reach);
            let n_size = reach + dz.abs() * (z1.abs() + far);
            let r2 = major * major;
            let poly = Poly {
                c: [
                    n * n * c - r2 * b * b,
                    2.0 * k * n * c + 2.0 * n * n * b - 2.0 * r2 * a * b,
                    k * k * c + 4.0 * k * n * b + n * n * a - r2 * a * a,
                    2.0 * k * k * b + 2.0 * k * n * a,
                    k * k * a,
                ],
                size: [
                    n_size * n_size * c_size + r2 * b_size * b_size,
                    2.0 * k * n_size * c_size
                        + 2.0 * n_size * n_size * b_size
                        + 2.0 * r2 * a * b_size,
                    k * k * c_size + 4.0 * k * n_size * b_size + n_size * n_size * a + r2 * a * a,
                    2.0 * k * k * b_size + 2.0 * k * n_size * a,
                    k * k * a,
                ],
            };
            let mut turns: Vec<f64> = [near, base - n / k]
                .into_iter()
                .filter(|t| t.is_finite())
                .collect();
            turns.extend(
                poly.changes(start - base, end - base)
                    .into_iter()
                    .map(|s| base + s),
            );
            Some(turns)
        }
        _ => None,
    }
}

/// Where on a stretch `(lo, hi)` of the curve over which the signed
/// distance is monotone it equals a level: `solve(level, lo, hi)`, or
/// `None` where no closed form settles it (bisection then does).
type Level<'a> = dyn Fn(f64, f64, f64) -> Option<f64> + 'a;

/// The closed form of [`Level`] for a line or a circle against a sphere:
/// the distance from the centre is `R + level` where the squared
/// distance, a quadratic or `h + a cos t + b sin t`, equals `(R + level)²`;
/// of its roots the one in the stretch. A root that rounding puts just
/// outside it, or none, leaves the stretch to bisection.
fn levels<'a>(curve: &'a Curve3, surface: &'a Surface) -> Option<Box<Level<'a>>> {
    let inside = |t: f64, lo: f64, hi: f64| (t >= lo && t <= hi).then_some(t);
    match (curve, surface) {
        (Curve3::Line(line), Surface::Sphere(sphere)) => {
            // Measured from the line's point nearest the centre, at `near`:
            // the squared distance is `s² + C`, `C` worked out from that
            // point (not as a difference of large squares when the line's
            // origin is far from the ball).
            let w = line.origin() - sphere.frame().origin();
            let near = -w.dot(line.direction());
            let c = (w + line.direction() * near).length_squared();
            let radius = sphere.radius();
            Some(Box::new(move |level: f64, lo: f64, hi: f64| {
                let reach = radius + level;
                let room = reach * reach - c;
                if room.is_nan() || room < 0.0 {
                    return None;
                }
                let s = room.sqrt();
                [near - s, near + s]
                    .into_iter()
                    .find_map(|t| inside(t, lo, hi))
            }))
        }
        (Curve3::Circle(circle), Surface::Sphere(sphere)) => {
            let f = circle.frame();
            let w = f.origin() - sphere.frame().origin();
            let r = circle.radius();
            let (a, b) = (2.0 * r * w.dot(f.x()), 2.0 * r * w.dot(f.y()));
            let (h, amplitude) = (w.length_squared() + r * r, a.phypot(b));
            let (phase, radius) = (b.patan2(a), sphere.radius());
            if amplitude.is_nan() || amplitude <= 0.0 {
                return None;
            }
            Some(Box::new(move |level: f64, lo: f64, hi: f64| {
                let reach = radius + level;
                let cosine = (reach * reach - h) / amplitude;
                if !(-1.0..=1.0).contains(&cosine) {
                    return None;
                }
                let spread = cosine.pacos();
                let tau = std::f64::consts::TAU;
                [phase - spread, phase + spread].into_iter().find_map(|t| {
                    // Moved by whole turns into the stretch.
                    let turns = ((lo - t) / tau).ceil();
                    turns
                        .is_finite()
                        .then_some(t + turns * tau)
                        .and_then(|t| inside(t, lo, hi))
                })
            }))
        }
        _ => None,
    }
}

/// How many units in the last place of the largest number involved the
/// signed distance from a ball or a cone along a line or a circle can be
/// off by rounding. Derived from its evaluation: the point of the curve
/// (`o + t d`, or `c + r (x cos t + y sin t)`: two roundings of numbers up
/// to the size per coordinate), its offset from the centre (one), its
/// length or, for a cone, its coordinates in the cone's frame (three
/// products and two sums per coordinate) and `hypot` (two), then the
/// radius or the cone's `cos α ρ − sin α z − r cos α` (three): about 13
/// for a ball and 20 for a cone, each relative to the size, and 64 leaves
/// room for the size being the larger of several numbers. The root
/// isolation of the other pairs has no such rule: it calls a stretch clear
/// only beyond the tolerance plus a bound on how far the distance strays
/// over the stretch (never a bare comparison of one value with the
/// tolerance), and halves no stretch below a quarter of the tolerance;
/// that this always covers rounding near a turning point is not proven.
pub(super) const ROUNDING: f64 = 64.0;

/// More whole turns of a circle than this in one piece are left to the
/// root isolation by [`monotone`] (pieces of booleans are at most a turn).
const MOST_TURNS: f64 = 64.0;

/// How large the numbers of `surface` are: its frame's distance from the
/// origin and its size (radius), for the rounding of a distance from it.
pub(super) fn surface_size(surface: &Surface) -> f64 {
    match surface {
        Surface::Sphere(s) => s.frame().origin().length() + s.radius().abs(),
        Surface::Cone(c) => c.frame().origin().length() + c.radius().abs(),
        Surface::Cylinder(c) => c.frame().origin().length() + c.radius().abs(),
        Surface::Torus(t) => t.frame().origin().length() + t.radii().0.abs() + t.radii().1.abs(),
        Surface::Plane(p) => p.frame().origin().length(),
        Surface::Bezier(_) | Surface::Nurbs(_) => 0.0,
    }
}

/// The hits of the piece of `curve` over `range` with the surface, when
/// the signed distance along it turns only at `turns` (moved by whole
/// periods into the piece): between them it is monotone, so each stretch
/// is clear of the surface on one side, then near it, then clear on the
/// other side, at most, divided where the distance is the tolerance on
/// either side (bisection, which a monotone function needs no bound
/// for). The runs are then read as the numeric root isolation reads its
/// own ([`hits_of_runs`]): the same rules, from exact runs rather than
/// halved stretches, and no sampling.
fn monotone(
    curve: &Curve3,
    range: (f64, f64),
    sheet: Sheet,
    turns: &[f64],
    solve: Option<&Level<'_>>,
    tol: f64,
    verify: bool,
) -> Option<CurveIntersection> {
    let (start, end) = range;
    let f = |t: f64| signed_distance(sheet, curve.eval(t)).unwrap_or(f64::NAN);
    let mut cuts: Vec<f64> = Vec::new();
    for &t in turns {
        match curve.period() {
            Some(period) if period > 0.0 => {
                // Every turn of the curve the piece reaches; a piece of
                // more turns than [`MOST_TURNS`] is left to the root
                // isolation.
                let first = ((start - t) / period).ceil();
                let last = ((end - t) / period).floor();
                if !(first.is_finite() && last.is_finite()) || last - first > MOST_TURNS {
                    return None;
                }
                let mut k = first;
                while k <= last {
                    let at = t + k * period;
                    if at > start && at < end {
                        cuts.push(at);
                    }
                    k += 1.0;
                }
            }
            _ => {
                if t > start && t < end {
                    cuts.push(t);
                }
            }
        }
    }
    cuts.sort_by(f64::total_cmp);
    let mut ends = vec![start];
    ends.extend(cuts);
    ends.push(end);
    // No run is called clear by a margin over the tolerance that could be
    // rounding ([`ROUNDING`]): a ball and a circle built to dip one
    // tolerance into it are within it there.
    let size = ends
        .iter()
        .map(|&t| curve.eval(t).length())
        .fold(surface_size(sheet.surface), f64::max);
    let rounding = ROUNDING * f64::EPSILON * size;
    let reach = tol + rounding;
    // A root in closed form is taken only where the distance there is the
    // level to within that rounding (twice it: the root's own and the
    // distance's); else, or without a closed form, bisection finds it.
    // So a closed form that cancels (a line whose origin is far from the
    // ball) costs time, never a crossing.
    let checked = |level: f64, lo: f64, hi: f64| {
        solve
            .and_then(|solve| solve(level, lo, hi))
            .filter(|&t| (f(t) - level).abs() <= 2.0 * rounding)
    };
    let checked_ref: &Level<'_> = &checked;
    // Turning points found as places where a quartic may change sign
    // ([`quartic_turns`]) are held to the distance itself: along each
    // stretch between them it must be monotone at its quarters, to its
    // rounding, or the root isolation is asked instead. (A check that
    // can only send a pair to the slower path.)
    if verify {
        for pair in ends.windows(2) {
            let &[a, b] = pair else {
                continue;
            };
            if a.is_nan() || b.is_nan() || b <= a {
                continue;
            }
            let values: Vec<f64> = (0..=4)
                .map(|i| f(a + (b - a) * f64::from(i) / 4.0))
                .collect();
            if values.iter().any(|v| !v.is_finite()) {
                return None;
            }
            let slack = 2.0 * rounding;
            let rising = values.windows(2).all(|w| match w {
                [x, y] => *y >= *x - slack,
                _ => true,
            });
            let falling = values.windows(2).all(|w| match w {
                [x, y] => *y <= *x + slack,
                _ => true,
            });
            if !(rising || falling) {
                return None;
            }
        }
    }
    let kind = |v: f64| {
        if v > reach {
            Kind::Clear { positive: true }
        } else if v < -reach {
            Kind::Clear { positive: false }
        } else {
            Kind::Near
        }
    };
    let mut runs: Vec<(f64, f64, Kind)> = Vec::new();
    let mut push = |a: f64, b: f64, k: Kind| match runs.last_mut() {
        Some(last) if last.2 == k => last.1 = b,
        _ => runs.push((a, b, k)),
    };
    for pair in ends.windows(2) {
        let &[a, b] = pair else {
            continue;
        };
        if a.is_nan() || b.is_nan() || b <= a {
            continue;
        }
        let (fa, fb) = (f(a), f(b));
        if !(fa.is_finite() && fb.is_finite()) {
            // (Not expected for an analytic surface: no division.)
            push(a, b, Kind::Near);
            continue;
        }
        // The levels the distance passes on the way from `fa` to `fb`,
        // in the order it passes them.
        let mut levels: Vec<f64> = [-reach, reach]
            .into_iter()
            .filter(|&level| (fa - level) * (fb - level) < 0.0)
            .collect();
        if fb < fa {
            levels.reverse();
        }
        let mut from = a;
        for level in levels {
            let at =
                checked_ref(level, from, b).unwrap_or_else(|| root(&|t| f(t) - level, from, b));
            push(from, at, kind(f(0.5 * (from + at))));
            from = at;
        }
        push(from, b, kind(f(0.5 * (from + b))));
    }
    if let [(_, _, Kind::Near)] = runs.as_slice() {
        return Some(CurveIntersection::OnSurface);
    }
    // A run near the surface that is no plain crossing (clear on the same
    // side before and after it, or at an end of the piece) and in which
    // the distance changes sign is a touch whose place is either of its
    // zeros, whichever a search happens on (`deepest`): at the edge of
    // tolerance (a dip of half to one tolerance) what is built next
    // depends on which. The quartic path leaves such pieces to the root
    // isolation, so that they are placed as they were before it.
    if verify
        && runs.iter().enumerate().any(|(index, &(lo, hi, run))| {
            let side = |i: Option<usize>| match i.and_then(|i| runs.get(i)) {
                Some(&(_, _, Kind::Clear { positive })) => Some(positive),
                _ => None,
            };
            let crossing = matches!(
                (side(index.checked_sub(1)), side(Some(index + 1))),
                (Some(x), Some(y)) if x != y
            );
            if run != Kind::Near || crossing {
                return false;
            }
            let values: Vec<f64> = ends
                .iter()
                .copied()
                .filter(|&t| t > lo && t < hi)
                .chain([lo, hi, 0.5 * (lo + hi)])
                .map(f)
                .collect();
            values.iter().any(|&v| v > 0.0) && values.iter().any(|&v| v < 0.0)
        })
    {
        return None;
    }
    Some(CurveIntersection::Hits(hits_of_runs(
        curve,
        &runs,
        &f,
        false,
        tol,
        Some(checked_ref),
    )))
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
    let tilt = n.cross(axis).length().patan2(n.dot(axis).abs());
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
    let tangent = alpha.ptan();
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
    let angle = (ray.dot(into).clamp(-1.0, 1.0).pacos() - alpha.abs()).abs();
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
            bounded: true,
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
        let apex = Vec3::new(0.0, 0.0, 1.0 / 0.5_f64.ptan());
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
        // sides of that: touches where it comes onto the patch and where it
        // leaves it, at the patch's borders (x = 0 and 1, t = 1 and 2).
        // Taken a tolerance at a time the stretch on the surface would be a
        // million pieces, and used to be refused as running along the
        // surface. (Until 2026-10-15 one touch somewhere along the stretch,
        // found by narrowing in on a distance that is zero all along: as a
        // cut of an edge it was a vertex where nothing is.)
        let result = curve_hits(&line, (0.0, 3.0), sheet, &tol).unwrap();
        let CurveIntersection::Hits(hits) = result else {
            panic!("expected two touches, got {result:?}");
        };
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().all(|h| h.tangent));
        assert!(
            (hits[0].t - 1.0).abs() <= 0.25 * tol.linear,
            "{}",
            hits[0].t
        );
        assert!(
            (hits[1].t - 2.0).abs() <= 0.25 * tol.linear,
            "{}",
            hits[1].t
        );
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

    /// The root isolation as `curve_hits` asked it for these pairs before
    /// their closed forms.
    fn isolated(
        curve: &Curve3,
        range: (f64, f64),
        surface: &Surface,
        tol: f64,
    ) -> CurveIntersection {
        let sheet = Sheet::bare(surface);
        numeric(
            curve,
            range,
            Probe {
                distance: &|p| signed_distance(sheet, p),
                lower: None,
                lower_stretch: None,
                bend: bend_bound(curve, surface),
                within: None,
                bounded: false,
            },
            tol,
        )
        .unwrap()
    }

    /// A line through a ball at a slant, by the root isolation alone at
    /// three scales, with a bound below the distance as a freeform
    /// surface's index gives one (here the distance itself): both
    /// crossings at their closed-form places, in fewer looks. Before the
    /// windows (docs/design/booleans.md › *A crossing closed in on*) it
    /// took 198 distances and 51 bounds; the pieces halved into a window
    /// need none, 153 to 155 and 62 now.
    #[test]
    fn a_line_through_a_ball_is_settled_in_fewer_evaluations() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            let ball = Surface::Sphere(Sphere::new(tilted(k), 3.0 * k).unwrap());
            let sheet = Sheet::bare(&ball);
            let centre = tilted(k).origin();
            let way = Vec3::new(0.6, -0.3, 0.74).normalized().unwrap();
            // 1.2 from the centre, so the chord is 2·sqrt(9 − 1.44).
            let off = way.cross(Vec3::new(0.0, 0.0, 1.0)).normalized().unwrap();
            let line = Curve3::Line(Line::new(centre + off * (1.2 * k), way).unwrap());
            let (count, bounds) = (std::cell::Cell::new(0_usize), std::cell::Cell::new(0_usize));
            let found = numeric(
                &line,
                (-10.0 * k, 12.0 * k),
                Probe {
                    distance: &|p| {
                        count.set(count.get() + 1);
                        signed_distance(sheet, p)
                    },
                    // As an index gives one: here the exact distance.
                    lower: Some(&|p, _| {
                        bounds.set(bounds.get() + 1);
                        signed_distance(sheet, p).map_or(0.0, f64::abs)
                    }),
                    lower_stretch: None,
                    bend: None,
                    within: None,
                    bounded: false,
                },
                tol,
            )
            .unwrap();
            let CurveIntersection::Hits(hits) = found else {
                panic!("on the surface")
            };
            let half = (9.0_f64 - 1.44).sqrt() * k;
            assert_eq!(hits.len(), 2, "{k}: {hits:?}");
            for (hit, t) in hits.iter().zip([-half, half]) {
                assert!((hit.t - t).abs() <= 1e-9 * k, "{k}: {} for {t}", hit.t);
                assert!(!hit.tangent);
            }
            // 198 distances and 51 bounds before.
            let (count, bounds) = (count.get(), bounds.get());
            assert!(count <= 165 && bounds <= 70, "{k}: {count} and {bounds}");
        }
    }

    /// The closed form, as `curve_hits` asks it now.
    fn closed(curve: &Curve3, range: (f64, f64), surface: &Surface, tol: f64) -> CurveIntersection {
        let turns = turning_points(curve, surface).unwrap();
        let level = levels(curve, surface);
        monotone(
            curve,
            range,
            Sheet::bare(surface),
            &turns,
            level.as_deref(),
            tol,
            false,
        )
        .unwrap()
    }

    /// SplitMix64, as a number in `[-1, 1)`.
    fn next(state: &mut u64) -> f64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        2.0 * ((z >> 11) as f64 / (1u64 << 53) as f64) - 1.0
    }

    /// Lines and circles against balls, lines against cones of both
    /// nappes, placed at random about a turned frame at three scales:
    /// the closed forms find what the root isolation finds, the same
    /// crossings and touches at the same places (to the tolerance), and
    /// on the surface where it does. Pieces that pass within eight
    /// tolerances of the surface without crossing it are left out (where
    /// a touch is put along a dip within the tolerance is the root
    /// isolation's search, not a fact); the next test holds those.
    #[test]
    fn closed_forms_find_what_the_root_isolation_finds() {
        let mut state = 17_u64;
        let mut compared = 0;
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            let f = tilted(k);
            let surfaces = [
                Surface::Sphere(Sphere::new(f, 3.0 * k).unwrap()),
                Surface::Cone(Cone::new(f, 1.5 * k, 0.4).unwrap()),
                Surface::Cone(Cone::new(f, 1.5 * k, -0.7).unwrap()),
            ];
            for surface in &surfaces {
                for _ in 0..400 {
                    let mut r = || next(&mut state);
                    let at = f.point(4.0 * k * r(), 4.0 * k * r(), 4.0 * k * r());
                    let way = Vec3::new(r(), r(), r());
                    let curve = if matches!(surface, Surface::Sphere(_)) && r() > 0.0 {
                        let Some(frame) = Frame::new(at, way, Vec3::new(r(), r(), r())) else {
                            continue;
                        };
                        Curve3::Circle(Circle::new(frame, (2.5 + 2.0 * r()) * k).unwrap())
                    } else {
                        let Ok(line) = Line::new(at, way) else {
                            continue;
                        };
                        Curve3::Line(line)
                    };
                    let range = match curve {
                        Curve3::Circle(_) => {
                            let from = 3.0 * r();
                            (from, from + 2.0 + 2.0 * r().abs())
                        }
                        _ => (-10.0 * k, (10.0 + 5.0 * r()) * k),
                    };
                    let sheet = Sheet::bare(surface);
                    let g = |t: f64| signed_distance(sheet, curve.eval(t)).unwrap();
                    // Clear of the band where a touch's place is the
                    // search's: at the ends, and at every turning point.
                    let turns = turning_points(&curve, surface).unwrap();
                    let lifted: Vec<f64> = turns
                        .iter()
                        .flat_map(|&t| (-3..=3).map(move |n| t + f64::from(n) * TAU))
                        .chain(turns.iter().copied())
                        .filter(|&t| t > range.0 && t < range.1)
                        .collect();
                    if [range.0, range.1]
                        .into_iter()
                        .chain(lifted)
                        .any(|t| g(t).abs() < 8.0 * tol)
                    {
                        continue;
                    }
                    let (want, got) = (
                        isolated(&curve, range, surface, tol),
                        closed(&curve, range, surface, tol),
                    );
                    match (&want, &got) {
                        (CurveIntersection::Hits(w), CurveIntersection::Hits(g)) => {
                            assert_eq!(
                                w.len(),
                                g.len(),
                                "{} on {}: {w:?} against {g:?}",
                                curve.kind(),
                                surface.kind()
                            );
                            for (x, y) in w.iter().zip(g) {
                                assert_eq!(x.tangent, y.tangent);
                                assert!(x.point.distance(y.point) <= tol, "{x:?} against {y:?}");
                            }
                            compared += w.len();
                        }
                        _ => assert_eq!(want, got),
                    }
                }
            }
        }
        assert!(compared > 1000, "{compared}");
    }

    /// A line and a circle that pass a ball at its radius plus `δ`: three
    /// tolerances outside miss it, within the tolerance either way touch
    /// it once (tangent), three tolerances inside cross it twice; a
    /// piece that ends on the ball meets it there, at three scales and
    /// turned.
    #[test]
    fn a_line_or_a_circle_grazing_a_ball_touches_it_once() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            let f = tilted(k);
            let radius = 3.0 * k;
            let ball = Surface::Sphere(Sphere::new(f, radius).unwrap());
            for (delta, count, tangent) in [
                (3.0, 0, false),
                (0.6, 1, true),
                (0.0, 1, true),
                (-0.6, 1, true),
                (-3.0, 2, false),
            ] {
                let reach = radius + delta * tol;
                // A line along the frame's x, `reach` above the centre.
                let line = Curve3::Line(Line::new(f.point(-5.0 * k, 0.0, reach), f.x()).unwrap());
                // A circle of radius 2 in the frame's xy plane whose
                // nearest point to the centre is `reach` from it.
                let centre = Frame::new(f.point(reach + 2.0 * k, 0.0, 0.0), f.z(), f.x()).unwrap();
                let circle = Curve3::Circle(Circle::new(centre, 2.0 * k).unwrap());
                for (curve, range) in [(&line, (0.0, 10.0 * k)), (&circle, (0.5, 5.5))] {
                    for found in [
                        closed(curve, range, &ball, tol),
                        curve_hits(
                            curve,
                            range,
                            Sheet::bare(&ball),
                            &Tolerance::new(tol, 1e-9).unwrap(),
                        )
                        .unwrap(),
                    ] {
                        let CurveIntersection::Hits(hits) = found else {
                            panic!("{delta}: on the ball");
                        };
                        assert_eq!(hits.len(), count, "{} at {delta}: {hits:?}", curve.kind());
                        for hit in &hits {
                            assert_eq!(hit.tangent, tangent, "{} at {delta}", curve.kind());
                            let d = signed_distance(Sheet::bare(&ball), hit.point).unwrap();
                            assert!(d.abs() <= tol, "{d}");
                        }
                    }
                }
            }
            // A line that ends on the ball, from inside: one hit, at the
            // end, as the root isolation finds it.
            let spoke = Curve3::Line(Line::new(f.point(0.0, 0.5 * k, 0.0), f.x()).unwrap());
            let end = (radius * radius - 0.25 * k * k).sqrt();
            let want = isolated(&spoke, (0.0, end), &ball, tol);
            let got = closed(&spoke, (0.0, end), &ball, tol);
            let (CurveIntersection::Hits(w), CurveIntersection::Hits(g)) = (&want, &got) else {
                panic!("{want:?} {got:?}");
            };
            assert_eq!((w.len(), g.len()), (1, 1));
            assert_eq!(w[0].tangent, g[0].tangent);
            assert!(w[0].point.distance(g[0].point) <= tol);
        }
    }

    /// A line that dips three tolerances into a ball (or a cone) and whose
    /// origin is far from it, up to ten million times the ball's radius
    /// away along the line: the squared distance from the line's origin
    /// would cancel against the radius (review of 2026-10-22: the root of
    /// "a tolerance inside" was found where the line is outside, and the
    /// two crossings were lost). Both crossings are found, as the numeric
    /// isolation finds them, at three scales and turned.
    #[test]
    fn a_line_from_far_away_crosses_a_ball_or_a_cone_it_dips_into() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            let f = tilted(k);
            let radius = 3.0 * k;
            let ball = Surface::Sphere(Sphere::new(f, radius).unwrap());
            let cone = Surface::Cone(Cone::new(f, radius, 0.4).unwrap());
            for far in [1e3, 1e5, 1e7] {
                // Along the frame's x, `radius - 3 tol` from the centre:
                // through the ball, and through the cone's wall where it is
                // `radius` wide.
                let depth = radius - 3.0 * tol;
                let start = f.point(-far * k, depth, 0.0);
                let line = Curve3::Line(Line::new(start, f.x()).unwrap());
                let range = (0.0, 2.0 * far * k);
                for surface in [&ball, &cone] {
                    let found = curve_hits(
                        &line,
                        range,
                        Sheet::bare(surface),
                        &Tolerance::new(tol, 1e-9).unwrap(),
                    )
                    .unwrap();
                    let CurveIntersection::Hits(hits) = &found else {
                        panic!("{} from {far}: {found:?}", surface.kind());
                    };
                    let crossings = hits.iter().filter(|h| !h.tangent).count();
                    assert_eq!(
                        crossings,
                        2,
                        "{} from {far} at {k}: {hits:?}",
                        surface.kind()
                    );
                    for hit in hits {
                        let d = signed_distance(Sheet::bare(surface), hit.point).unwrap();
                        // (Within the tolerance, and the rounding of a
                        // point that far out.)
                        assert!(d.abs() <= tol + 1e-13 * far * k, "{d}");
                    }
                    let CurveIntersection::Hits(want) = isolated(&line, range, surface, tol) else {
                        panic!("the root isolation finds hits");
                    };
                    assert_eq!(want.iter().filter(|h| !h.tangent).count(), 2);
                }
            }
        }
    }

    /// The quartic path, as `curve_hits` asks it: `None` where its check
    /// sent the pair to the root isolation.
    fn quartic(
        curve: &Curve3,
        range: (f64, f64),
        surface: &Surface,
        tol: f64,
    ) -> Option<CurveIntersection> {
        by_quartic(curve, range, Sheet::bare(surface), tol)
    }

    /// Ellipses against balls and lines against tori, placed at random
    /// about a turned frame at three scales: the quartic path finds what
    /// the root isolation finds (count, kind, place within the
    /// tolerance), and its check against the distance never sends a pair
    /// back. Pieces that pass within eight tolerances of the surface at
    /// an end or a turning point are left out, as above.
    #[test]
    fn quartic_turns_find_what_the_root_isolation_finds() {
        let mut state = 23_u64;
        let (mut compared, mut pairs, mut needed) = (0, 0, 0);
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            let f = tilted(k);
            let surfaces = [
                Surface::Sphere(Sphere::new(f, 3.0 * k).unwrap()),
                Surface::Torus(Torus::new(f, 4.0 * k, k).unwrap()),
                Surface::Torus(Torus::new(f, 2.0 * k, 1.5 * k).unwrap()),
            ];
            for surface in &surfaces {
                for _ in 0..500 {
                    let mut r = || next(&mut state);
                    let at = f.point(5.0 * k * r(), 5.0 * k * r(), 3.0 * k * r());
                    let way = Vec3::new(r(), r(), r());
                    let curve = if matches!(surface, Surface::Sphere(_)) {
                        let Some(frame) = Frame::new(at, way, Vec3::new(r(), r(), r())) else {
                            continue;
                        };
                        let (a, b) = ((2.5 + 2.0 * r()) * k, (1.0 + 0.9 * r()) * k);
                        Curve3::Ellipse(craft3d_geom::Ellipse::new(frame, a, b).unwrap())
                    } else {
                        let Ok(line) = Line::new(at, way) else {
                            continue;
                        };
                        Curve3::Line(line)
                    };
                    let range = match curve {
                        Curve3::Ellipse(_) => {
                            let from = 3.0 * r();
                            (from, from + 2.0 + 2.0 * r().abs())
                        }
                        _ => (-10.0 * k, (10.0 + 5.0 * r()) * k),
                    };
                    let sheet = Sheet::bare(surface);
                    let g = |t: f64| signed_distance(sheet, curve.eval(t)).unwrap();
                    let turns = quartic_turns(&curve, surface, range).unwrap();
                    let lifted: Vec<f64> = turns
                        .iter()
                        .flat_map(|&t| (-3..=3).map(move |n| t + f64::from(n) * TAU))
                        .filter(|&t| t > range.0 && t < range.1)
                        .collect();
                    if [range.0, range.1]
                        .into_iter()
                        .chain(lifted)
                        .any(|t| g(t).abs() < 8.0 * tol)
                    {
                        continue;
                    }
                    let want = isolated(&curve, range, surface, tol);
                    let got = quartic(&curve, range, surface, tol)
                        .unwrap_or_else(|| panic!("{} sent back", curve.kind()));
                    pairs += 1;
                    // Without its turning points the same path is wrong
                    // or sent back for some pairs: they are what it needs.
                    let bare = monotone(&curve, range, Sheet::bare(surface), &[], None, tol, true);
                    if bare.as_ref() != Some(&got) {
                        needed += 1;
                    }
                    match (&want, &got) {
                        (CurveIntersection::Hits(w), CurveIntersection::Hits(g)) => {
                            assert_eq!(
                                w.len(),
                                g.len(),
                                "{} on {}: {w:?} against {g:?}",
                                curve.kind(),
                                surface.kind()
                            );
                            for (x, y) in w.iter().zip(g) {
                                assert_eq!(x.tangent, y.tangent);
                                assert!(x.point.distance(y.point) <= tol, "{x:?} against {y:?}");
                            }
                            compared += w.len();
                        }
                        _ => assert_eq!(want, got),
                    }
                }
            }
        }
        assert!(
            compared > 1000 && pairs > 2000,
            "{compared} hits, {pairs} pairs"
        );
        assert!(needed > 100, "{needed}");
    }

    /// An ellipse that passes a ball at its radius plus `δ` (in a plane
    /// over the ball's top, its vertex over the top), a line that passes
    /// the top of a torus's tube likewise, and a line along the axis in
    /// the hole that passes the tube's inner equator: three tolerances
    /// outside miss, within the tolerance either way touch once, three
    /// tolerances inside cross twice; at three scales and turned, from
    /// the quartic path and from `curve_hits`.
    #[test]
    fn an_ellipse_grazing_a_ball_or_a_line_grazing_a_torus_touches_once() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            let f = tilted(k);
            let radius = 3.0 * k;
            let ball = Surface::Sphere(Sphere::new(f, radius).unwrap());
            let (major, minor) = (4.0 * k, k);
            let torus = Surface::Torus(Torus::new(f, major, minor).unwrap());
            for (delta, count, tangent) in [
                (3.0, 0, false),
                (0.6, 1, true),
                (0.0, 1, true),
                (-0.6, 1, true),
                (-3.0, 2, false),
            ] {
                let off = delta * tol;
                let plane = Frame::new(f.point(2.0 * k, 0.0, radius + off), f.z(), f.x()).unwrap();
                let ellipse =
                    Curve3::Ellipse(craft3d_geom::Ellipse::new(plane, 2.0 * k, 0.7 * k).unwrap());
                let top =
                    Curve3::Line(Line::new(f.point(-5.0 * k, major, minor + off), f.x()).unwrap());
                let hole = Curve3::Line(
                    Line::new(f.point(major - minor - off, 0.0, -5.0 * k), f.z()).unwrap(),
                );
                for (curve, range, surface) in [
                    (&ellipse, (1.5, 5.0), &ball),
                    (&top, (0.0, 10.0 * k), &torus),
                    (&hole, (0.0, 10.0 * k), &torus),
                ] {
                    // A dip through the surface within the tolerance is a
                    // touch at either zero: sent back to the root isolation.
                    let quartic = quartic(curve, range, surface, tol);
                    // (Exactly tangent, rounding decides.)
                    let through = delta < 0.0 && delta > -1.0;
                    if delta != 0.0 {
                        assert_eq!(
                            quartic.is_none(),
                            through,
                            "{} at {delta}, {k}",
                            curve.kind()
                        );
                    }
                    for found in quartic.into_iter().chain([curve_hits(
                        curve,
                        range,
                        Sheet::bare(surface),
                        &Tolerance::new(tol, 1e-9).unwrap(),
                    )
                    .unwrap()])
                    {
                        let CurveIntersection::Hits(hits) = found else {
                            panic!("{delta}: on the surface");
                        };
                        assert_eq!(hits.len(), count, "{} at {delta}: {hits:?}", curve.kind());
                        for hit in &hits {
                            assert_eq!(hit.tangent, tangent, "{} at {delta}", curve.kind());
                            let d = signed_distance(Sheet::bare(surface), hit.point).unwrap();
                            assert!(d.abs() <= tol, "{d}");
                        }
                    }
                }
            }
        }
    }

    /// A line from far away (up to ten million radii along it) that dips
    /// three tolerances into a torus's tube, and an ellipse whose centre
    /// is that far from a ball and whose vertex dips three tolerances
    /// into it: both crossings are found, at three scales and turned.
    #[test]
    fn a_line_or_an_ellipse_from_far_away_crosses_what_it_dips_into() {
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            let f = tilted(k);
            let (major, minor) = (4.0 * k, k);
            let torus = Surface::Torus(Torus::new(f, major, minor).unwrap());
            let radius = 3.0 * k;
            let ball = Surface::Sphere(Sphere::new(f, radius).unwrap());
            for far in [1e3, 1e5, 1e7] {
                let line = Curve3::Line(
                    Line::new(f.point(-far * k, major, minor - 3.0 * tol), f.x()).unwrap(),
                );
                // An ellipse in the plane over the ball's top whose vertex
                // at `t = π` is over the top.
                let plane =
                    Frame::new(f.point(far * k, 0.0, radius - 3.0 * tol), f.z(), f.x()).unwrap();
                let ellipse = Curve3::Ellipse(
                    craft3d_geom::Ellipse::new(plane, far * k, 0.5 * far * k).unwrap(),
                );
                let pi = std::f64::consts::PI;
                for (curve, range, surface) in [
                    (&line, (0.0, 2.0 * far * k), &torus),
                    (&ellipse, (pi - 0.5, pi + 0.5), &ball),
                ] {
                    let found = curve_hits(
                        curve,
                        range,
                        Sheet::bare(surface),
                        &Tolerance::new(tol, 1e-9).unwrap(),
                    )
                    .unwrap();
                    // (By the quartic path, not sent back.)
                    assert_eq!(quartic(curve, range, surface, tol).as_ref(), Some(&found));
                    let CurveIntersection::Hits(hits) = &found else {
                        panic!("{} from {far}: {found:?}", curve.kind());
                    };
                    let crossings = hits.iter().filter(|h| !h.tangent).count();
                    assert_eq!(crossings, 2, "{} from {far} at {k}: {hits:?}", curve.kind());
                    for hit in hits {
                        let d = signed_distance(Sheet::bare(surface), hit.point).unwrap();
                        assert!(d.abs() <= tol + 1e-13 * far * k, "{d}");
                    }
                }
            }
        }
    }

    /// Pieces that graze: lines tangent to a torus (at random points of
    /// its tube, outside and in the hole) and ellipses tangent to a ball,
    /// moved off the surface by −3 to 3 tolerances, some ending within a
    /// few tolerances of the place they graze, at three scales and turned.
    /// What `curve_hits` answers (the quartic path, or the root isolation
    /// where it sends a piece back) is what the root isolation answers:
    /// the same count and kinds, at the same places to the tolerance.
    #[test]
    fn grazing_pieces_are_found_as_the_root_isolation_finds_them() {
        let mut state = 99_u64;
        let (mut compared, mut by_quartic_path) = (0, 0);
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            let f = tilted(k);
            let (major, minor) = (4.0 * k, k);
            let torus = Surface::Torus(Torus::new(f, major, minor).unwrap());
            let radius = 3.0 * k;
            let ball = Surface::Sphere(Sphere::new(f, radius).unwrap());
            for case in 0..1200 {
                let mut r = || next(&mut state);
                let off = 3.0 * tol * r();
                let (curve, range, surface) = if case % 2 == 0 {
                    // A point of the tube, its outward normal, and a
                    // direction square to it.
                    let (u, v) = (TAU * r(), TAU * r());
                    let ring = f.x() * u.pcos() + f.y() * u.psin();
                    let normal = ring * v.pcos() + f.z() * v.psin();
                    let at = f.origin() + ring * major + normal * (minor + off);
                    let across = normal.cross(Vec3::new(r(), r(), r())).normalized().unwrap();
                    let start = if r() > 0.0 { -5.0 * k } else { 8.0 * tol * r() };
                    (
                        Curve3::Line(Line::new(at, across).unwrap()),
                        (start, 5.0 * k * (1.0 + r().abs())),
                        &torus,
                    )
                } else {
                    // An ellipse in a plane tangent to the ball, its vertex
                    // at `t = π` over the point of tangency.
                    let normal = Vec3::new(r(), r(), r()).normalized().unwrap();
                    let side = normal.cross(Vec3::new(r(), r(), r())).normalized().unwrap();
                    let touch = f.origin() + normal * (radius + off);
                    let a = (1.0 + r().abs()) * k;
                    let plane = Frame::new(touch + side * a, normal, side).unwrap();
                    let ellipse = craft3d_geom::Ellipse::new(plane, a, (0.5 + 0.4 * r()) * k);
                    let pi = std::f64::consts::PI;
                    let from = if r() > 0.0 { pi - 1.0 } else { pi + 8e-6 * r() };
                    (Curve3::Ellipse(ellipse.unwrap()), (from, pi + 1.0), &ball)
                };
                let want = isolated(&curve, range, surface, tol);
                let got = curve_hits(
                    &curve,
                    range,
                    Sheet::bare(surface),
                    &Tolerance::new(tol, 1e-9).unwrap(),
                )
                .unwrap();
                if quartic(&curve, range, surface, tol).is_some() {
                    by_quartic_path += 1;
                }
                match (&want, &got) {
                    (CurveIntersection::Hits(w), CurveIntersection::Hits(g)) => {
                        assert_eq!(
                            w.len(),
                            g.len(),
                            "{} at {k}: {w:?} against {g:?}",
                            curve.kind()
                        );
                        for (x, y) in w.iter().zip(g) {
                            assert_eq!(x.tangent, y.tangent, "{} at {k}", curve.kind());
                            assert!(x.point.distance(y.point) <= tol, "{x:?} against {y:?}");
                        }
                        compared += w.len();
                    }
                    _ => assert_eq!(want, got),
                }
            }
        }
        assert!(
            compared > 2000 && by_quartic_path > 1000,
            "{compared}, {by_quartic_path}"
        );
    }
}
