//! Surfaces that cross at a shallow angle in a hairpin a few tolerances
//! across (2026-11-20, the RC wing Jig's spars beside its ribs,
//! docs/design/booleans.md › *A spar's face beside a rib's skin*).
//!
//! A spar's flat bottom all but holds a ruling of the lofted skin it lies
//! in: the plane crosses the skin in a curve that runs down one side of
//! the ruling, turns round within a few hundredths of a millimetre and
//! runs back up the other side, and at the turn the surfaces cross at a
//! sine of a few 1e-5. A marched point was brought within a ten-thousandth
//! of the tolerance of the other surface, which at that sine is anywhere
//! across a band three tolerances wide about the curve: wider than the
//! turn. No step, however short, then landed where the march held it to,
//! and the march stalled ("marching an intersection curve stalled").
//!
//! The pair here is that crossing in closed form. A trough along y,
//! `z = a (y − y0) − x² (1 + twist y / length) a / (2 r)`, against the
//! plane `z = 0`: they cross in the parabola
//! `x² (1 + twist y / length) = 2 r (y − y0)`, whose apex is `y0` from the
//! trough's end, where it turns with the radius `r` and the surfaces cross
//! at the sine `a`. The trough is a Bézier patch of degree two by one (a
//! ruled surface, as a loft between two sections is), and the same surface
//! as a spline of two spans. Every point of the curve that comes back is
//! held to the parabola itself, along the surface, and the parabola to the
//! curve from one side of the trough to the other: not only to both
//! surfaces, which lie within the tolerance of each other all over the
//! turn.
//!
//! Lengths are in tolerances, so the pair is the same at every tolerance;
//! the tolerances run from 1e-8 (a trough a few millimetres long in
//! metres) to 10 (one of a few thousand kilometres).

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// `!(x <= y)` is deliberate: unlike `x > y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

mod common;

use common::*;
use craft3d_geom::intersect::{MeetingCurve, SurfaceMeeting, meet_surfaces};
use craft3d_geom::{BezierPatch, Curve3, NurbsSurface, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};

/// The linear tolerances the pairs are made at.
const TOLERANCES: [f64; 5] = [1e-8, 1e-6, 1e-5, 1e-2, 10.0];
/// The sine at which the surfaces cross at the turn (the Jig's: 3.3e-5).
const SINES: [f64; 2] = [1e-4, 3.3e-5];
/// The radius of the turn, in tolerances (the Jig's: 1.6).
const RADII: [f64; 3] = [1.6, 0.4, 0.1];
/// How far the turn is from the trough's end, in tolerances (the Jig's:
/// 110).
const INSETS: [f64; 3] = [30.0, 110.0, 500.0];

fn tolerance(linear: f64) -> Tolerance {
    Tolerance::new(linear, 1e-9).unwrap()
}

/// The trough, in the axes of `frame`: over `x` in `±half` and `y` in
/// `0..length`.
#[derive(Clone, Copy, Debug)]
struct Trough {
    frame: Frame,
    tol: f64,
    /// The slope along the trough: the sine of the crossing at the turn.
    a: f64,
    /// The radius of the turn.
    r: f64,
    /// The turn's distance from the end `y = 0`.
    y0: f64,
    twist: f64,
    half: f64,
    length: f64,
}

impl Trough {
    /// Wide enough for its sides to be 60 tolerances off the plane, and
    /// long enough for its far end to be 150 off: the surfaces are two,
    /// not one within the tolerance.
    fn new(tol: f64, a: f64, radius: f64, inset: f64, twist: f64, frame: Frame) -> Self {
        let r = radius * tol;
        let y0 = inset * tol;
        Self {
            frame,
            tol,
            a,
            r,
            y0,
            twist,
            half: (120.0 * r * tol / a).sqrt(),
            length: y0 + 150.0 * tol / a,
        }
    }

    /// The trough's height at `(x, y)`.
    fn height(&self, x: f64, y: f64) -> f64 {
        self.a * (y - self.y0)
            - x * x * (1.0 + self.twist * y / self.length) * self.a / (2.0 * self.r)
    }

    /// The control points of the patch of degree two across and one
    /// along, across first.
    fn control(&self) -> Vec<Vec3> {
        let depth = self.half * self.half * self.a / (2.0 * self.r);
        let mut ctrl = Vec::new();
        for (i, x) in [-self.half, 0.0, self.half].into_iter().enumerate() {
            for (j, y) in [0.0, self.length].into_iter().enumerate() {
                let middle = self.a * (y - self.y0);
                let depth = depth * (1.0 + self.twist * j as f64);
                // A parabola `middle − depth (2u − 1)²` as a Bézier curve.
                let z = if i == 1 {
                    middle + depth
                } else {
                    middle - depth
                };
                ctrl.push(self.frame.point(x, y, z));
            }
        }
        ctrl
    }

    fn bezier(&self) -> Surface {
        Surface::Bezier(BezierPatch::new(2, 1, self.control()).unwrap())
    }

    /// The same surface as a spline of two spans across, a knot at 0.4.
    fn nurbs(&self) -> Surface {
        let c = self.control();
        let knot = 0.4;
        let row = |i: usize| [c[2 * i], c[2 * i + 1]];
        let between = |a: [Vec3; 2], b: [Vec3; 2]| [a[0].lerp(b[0], knot), a[1].lerp(b[1], knot)];
        let rows = [
            row(0),
            between(row(0), row(1)),
            between(row(1), row(2)),
            row(2),
        ];
        Surface::Nurbs(
            NurbsSurface::new(
                2,
                1,
                vec![0.0, 0.0, 0.0, knot, 1.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                4,
                2,
                rows.concat(),
                vec![1.0; 8],
            )
            .unwrap(),
        )
    }

    /// The box a boolean asks in: the trough's own, with room.
    fn within(&self) -> Bbox3 {
        let hull = Bbox3::from_points(self.control());
        let (lo, hi) = hull.corners().unwrap();
        let room = 16.0 * self.tol + 0.013 * hull.diagonal();
        let pad = Vec3::new(room, room, room);
        Bbox3::from_points([lo - pad, hi + pad])
    }
}

/// What a trough is met with, and the curve they cross in: `y` at `x`
/// (rising with `|x|`), and the height of the other surface at `x`.
struct Pair {
    trough: Trough,
    surface: Surface,
    other: Surface,
    what: String,
    y_at: Box<dyn Fn(f64) -> f64>,
    z_at: Box<dyn Fn(f64) -> f64>,
}

impl Pair {
    /// `|x|` of the curve at `y`, by halving (`y_at` rises with `|x|`).
    fn x_at(&self, y: f64) -> Option<f64> {
        let (mut lo, mut hi) = (0.0, self.trough.half);
        if y < (self.y_at)(lo) || y > (self.y_at)(hi) {
            return None;
        }
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if (self.y_at)(mid) <= y {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some(0.5 * (lo + hi))
    }

    /// No less than how far the point at `(x, y)` is from the curve: the
    /// lesser of the way to it along y and along x.
    fn off_curve(&self, x: f64, y: f64) -> f64 {
        let along = (y - (self.y_at)(x)).abs();
        match self.x_at(y) {
            Some(at) => along.min((x.abs() - at).abs()),
            None => along,
        }
    }
}

/// The trough against the plane `z = 0` of its frame.
fn against_plane(trough: Trough, spline: bool) -> Pair {
    let t = trough;
    Pair {
        trough,
        surface: if spline { t.nurbs() } else { t.bezier() },
        other: plane(t.frame),
        what: format!(
            "{} against a plane",
            if spline { "a spline" } else { "a patch" }
        ),
        y_at: Box::new(move |x| {
            (2.0 * t.r * t.y0 + x * x) / (2.0 * t.r - x * x * t.twist / t.length)
        }),
        z_at: Box::new(|_| 0.0),
    }
}

/// The trough against a cylinder along it that touches the plane `z = 0`
/// from below along `x = 0`, of radius `2 r / a`: it takes half the
/// trough's bend, so the turn is twice as wide.
fn against_cylinder(trough: Trough) -> Pair {
    let t = trough;
    let radius = 2.0 * t.r / t.a;
    let axis = Frame::new(
        t.frame.point(0.0, 0.0, -radius),
        t.frame.vector(0.0, 1.0, 0.0),
        t.frame.vector(1.0, 0.0, 0.0),
    )
    .unwrap();
    // How far the cylinder is below the plane at `x`.
    let dip = move |x: f64| x * x / (radius + (radius * radius - x * x).sqrt());
    Pair {
        trough,
        surface: t.bezier(),
        other: cylinder(axis, radius),
        what: "a patch against a cylinder".to_string(),
        y_at: Box::new(move |x| t.y0 + (x * x * t.a / (2.0 * t.r) - dip(x)) / t.a),
        z_at: Box::new(move |x| -dip(x)),
    }
}

/// The trough against a flat patch in the plane `z = 0` of its frame (a
/// parallelogram of degree one by one, askew to the trough), one of the
/// two a spline: `walked` says which, so that each is the one the marcher
/// walks once.
fn against_flat(trough: Trough, walked: bool) -> Pair {
    let t = trough;
    let (x, y) = (3.0 * t.half, 0.1 * t.length);
    let flat = BezierPatch::new(
        1,
        1,
        vec![
            t.frame.point(-x, -y, 0.0),
            t.frame.point(-x, t.length + y, 0.0),
            t.frame.point(x, -y + 2.0 * t.half, 0.0),
            t.frame.point(x, t.length + y + 2.0 * t.half, 0.0),
        ],
    )
    .unwrap();
    let (surface, other) = if walked {
        (
            t.bezier(),
            Surface::Nurbs(NurbsSurface::from_bezier(&flat).unwrap()),
        )
    } else {
        (t.nurbs(), Surface::Bezier(flat))
    };
    Pair {
        trough,
        surface,
        other,
        what: format!(
            "{} against a flat {}",
            if walked { "a patch" } else { "a spline" },
            if walked { "spline" } else { "patch" }
        ),
        y_at: Box::new(move |x| {
            (2.0 * t.r * t.y0 + x * x) / (2.0 * t.r - x * x * t.twist / t.length)
        }),
        z_at: Box::new(|_| 0.0),
    }
}

/// Points of a marched curve, four to each of its own spans (it has more
/// spans where it bends: evenly spread points would step over the turn).
fn along(curve: &MeetingCurve) -> Vec<Vec3> {
    let Curve3::Nurbs(spline) = &curve.curve else {
        panic!("a marched curve is a spline: {curve:?}");
    };
    let (t0, t1) = curve.range;
    let mut at = vec![t0];
    at.extend(
        spline
            .knots()
            .iter()
            .copied()
            .filter(|k| *k > t0 && *k < t1),
    );
    at.push(t1);
    at.dedup();
    let mut points = Vec::new();
    for w in at.windows(2) {
        for s in 0..4 {
            points.push(spline.eval(w[0] + (w[1] - w[0]) * f64::from(s) / 4.0));
        }
    }
    points.push(spline.eval(t1));
    points
}

/// How far `p` is from a polyline.
fn off_polyline(points: &[Vec3], p: Vec3) -> f64 {
    let mut best = f64::INFINITY;
    for w in points.windows(2) {
        let d = w[1] - w[0];
        let l2 = d.length_squared();
        let s = if l2 > 0.0 {
            ((p - w[0]).dot(d) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        best = best.min((w[0] + d * s).distance(p));
    }
    best
}

/// The pair's curve as it should come back: one open curve, on both
/// surfaces, every point of it on the parabola and the parabola on it
/// from one side of the trough to the other. `Ok` with how far off the
/// parabola it is at most (in tolerances) and how many spans it has.
fn check(pair: &Pair) -> Result<(f64, usize), String> {
    let t = &pair.trough;
    let tol = tolerance(t.tol);
    let within = t.within();
    let meeting = match meet_surfaces(&pair.surface, &pair.other, &within, &tol) {
        Ok(SurfaceMeeting::Meets(m)) => m,
        Ok(other) => return Err(format!("no curve: {other:?}")),
        Err(e) => return Err(format!("refused: {e}")),
    };
    if meeting.curves.len() != 1 || !meeting.points.is_empty() {
        return Err(format!(
            "{} curves and {} points",
            meeting.curves.len(),
            meeting.points.len()
        ));
    }
    let curve = &meeting.curves[0];
    if curve.closed || curve.tangent || !(curve.deviation <= t.tol) {
        return Err(format!(
            "closed {}, tangent {}, deviation {}",
            curve.closed, curve.tangent, curve.deviation
        ));
    }
    let points = along(curve);
    let mut worst = 0.0_f64;
    for (k, p) in points.iter().enumerate() {
        let (x, y, z) = t.frame.local(*p);
        // On both surfaces: the other's height at x, and the trough's.
        let off_other = (z - (pair.z_at)(x)).abs();
        let off_trough = (z - t.height(x, y)).abs();
        if !(off_other <= t.tol && off_trough <= 1.1 * t.tol) {
            return Err(format!(
                "point {k} of {} is {:.3} and {:.3} tolerances off the surfaces",
                points.len(),
                off_other / t.tol,
                off_trough / t.tol
            ));
        }
        // On the surfaces by their own projections as well, here and
        // there (the heights above are closed forms, these are not).
        if k % 16 == 3 {
            for surface in [&pair.surface, &pair.other] {
                let off = distance(surface, *p);
                if !(off <= t.tol) {
                    return Err(format!(
                        "point {k} is {:.3} tolerances off a surface by its projection",
                        off / t.tol
                    ));
                }
            }
        }
        let off = pair.off_curve(x, y);
        if !(off <= t.tol) {
            return Err(format!(
                "point {k} of {} at ({:.3}, {:.3}) tolerances is {:.3} tolerances off the parabola",
                points.len(),
                x / t.tol,
                y / t.tol,
                off / t.tol
            ));
        }
        worst = worst.max(off / t.tol);
    }
    // The parabola from side to side: closer together about the turn.
    for k in -100..=100 {
        let s = f64::from(k) / 100.0;
        let x = t.half * s * s * s;
        let p = t.frame.point(x, (pair.y_at)(x), (pair.z_at)(x));
        let off = off_polyline(&points, p);
        if !(off <= t.tol) {
            return Err(format!(
                "the parabola at x = {:.3} tolerances is {:.3} tolerances from the curve",
                x / t.tol,
                off / t.tol
            ));
        }
        worst = worst.max(off / t.tol);
    }
    Ok((worst, points.len() / 4))
}

/// Every trough of the family at one tolerance: the sines, radii and
/// insets, every other one twisted (deeper towards its far end, as a
/// tapered loft is) and two in three set askew to the world's axes.
fn troughs(tol: f64) -> Vec<Trough> {
    let mut all = Vec::new();
    for a in SINES {
        for radius in RADII {
            for inset in INSETS {
                let n = all.len();
                let twist = if n % 2 == 1 { 0.5 } else { 0.0 };
                let frame = if n % 3 == 0 {
                    Frame::WORLD
                } else {
                    tilted(1e4 * tol)
                };
                all.push(Trough::new(tol, a, radius, inset, twist, frame));
            }
        }
    }
    all
}

/// Checks every pair and fails with the list of those that did not come
/// back as they should.
fn all_come_back(pairs: impl Iterator<Item = Pair>) {
    let (mut failed, mut count, mut worst, mut spans) = (Vec::new(), 0, 0.0_f64, 0);
    for pair in pairs {
        count += 1;
        let t = &pair.trough;
        match check(&pair) {
            Ok((off, n)) => {
                worst = worst.max(off);
                spans += n;
            }
            Err(why) => failed.push(format!(
                "{} at tolerance {:e}, sine {:e}, radius {} and inset {} tolerances, twist {}: {why}",
                pair.what,
                t.tol,
                t.a,
                t.r / t.tol,
                t.y0 / t.tol,
                t.twist
            )),
        }
    }
    eprintln!(
        "{count} pairs, {} failed; at most {worst:.3} tolerances off the parabola, {spans} spans",
        failed.len()
    );
    assert!(
        failed.is_empty(),
        "{} of {count} pairs:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// A plane that crosses a ruled patch, or the same surface as a spline, in
/// a hairpin at a shallow angle: one curve from side to side, on the
/// parabola. (Before points were held to the curve, the march stalled at
/// the turn of some of these and went round it by luck at the others.)
#[test]
fn a_plane_crosses_a_ruled_surface_in_a_hairpin_at_a_shallow_angle() {
    all_come_back(TOLERANCES.into_iter().flat_map(|tol| {
        troughs(tol)
            .into_iter()
            .flat_map(|trough| [against_plane(trough, false), against_plane(trough, true)])
    }));
}

/// The same against a cylinder that lies along the trough: an analytic
/// surface that is not flat.
#[test]
fn a_cylinder_crosses_a_ruled_patch_in_a_hairpin_at_a_shallow_angle() {
    all_come_back(TOLERANCES.into_iter().flat_map(|tol| {
        troughs(tol)
            .into_iter()
            .filter(|t| t.twist == 0.0)
            .map(against_cylinder)
    }));
}

/// The same between two freeform surfaces, the trough walked and
/// measured against in turn.
#[test]
fn two_freeform_surfaces_cross_in_a_hairpin_at_a_shallow_angle() {
    all_come_back(TOLERANCES.into_iter().flat_map(|tol| {
        troughs(tol)
            .into_iter()
            .flat_map(|trough| [against_flat(trough, true), against_flat(trough, false)])
    }));
}
