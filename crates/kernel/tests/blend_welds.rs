//! Welds: fillets along the closed edge where a tube meets a tube (the
//! intersection of two cylinders, a single closed edge that is a fitted
//! spline), held point by point against the definition of a rolling-ball
//! fillet, at three scales in a tilted frame.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::{SCALES, mass, tolerance};
use craft3d_kernel::boolean::{BooleanOp, boolean, contains_points};
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, EdgeId, check};

/// The post: radius 17.5 about the z axis; the stay: radius 6 from the
/// origin along `d` (at `alpha` from the post's axis, in the xz plane).
const POST: f64 = 17.5;
const STAY: f64 = 6.0;

fn stay_axis(alpha: f64) -> Vec3 {
    Vec3::new(alpha.sin(), 0.0, alpha.cos())
}

/// The tilted frame everything is built in.
fn tilted(k: f64) -> Frame {
    Frame::new(
        Vec3::new(3.0, -2.0, 1.0) * k,
        Vec3::new(0.3, 0.2, 1.0).normalized().unwrap(),
        Vec3::X,
    )
    .unwrap()
}

/// The post and the stay, at scale `k` in the tilted frame, joined or
/// the stay taken out of the post (a blind cross hole).
fn tee(k: f64, alpha: f64, op: BooleanOp) -> Body {
    let t = tilted(k);
    let post_frame = Frame::new(t.point(0.0, 0.0, -100.0 * k), t.z(), t.x()).unwrap();
    let d = stay_axis(alpha);
    let stay_frame = Frame::new(t.origin(), t.vector(d.x, d.y, d.z), t.y()).unwrap();
    let post = primitives::cylinder(&post_frame, POST * k, 200.0 * k).unwrap();
    let stay = primitives::cylinder(&stay_frame, STAY * k, 150.0 * k).unwrap();
    boolean(&post, &stay, op, &tolerance(k)).unwrap().body
}

/// The one closed edge that is not a circle: where the two cylinders meet.
fn weld_edge(body: &Body) -> EdgeId {
    let found: Vec<usize> = (0..body.edges.len())
        .filter(|i| body.edges[*i].start == body.edges[*i].end)
        .filter(|i| body.edges[*i].curve.kind() != "circle")
        .collect();
    assert_eq!(found.len(), 1, "{found:?}");
    EdgeId(found[0] as u32)
}

/// Distance from the post's axis (local coordinates).
fn rho_post(c: Vec3) -> f64 {
    c.x.hypot(c.y)
}

/// Distance from the stay's axis.
fn rho_stay(c: Vec3, d: Vec3) -> f64 {
    (c - d * c.dot(d)).length()
}

/// The set of points `s1 (rho_post - a) >= 0` and `s2 (rho_stay - b) >= 0`
/// on the stay's side of the post's axis (c · d > 0): where the centres
/// of the balls may lie. The distance from `p` to it: zero inside it;
/// otherwise the nearest of the radial projections onto either cylinder
/// that lie in the set and the nearest point of the curve where the two
/// cylinders cross (a cylinder has no other nearest point than the
/// radial one, so a nearest point of the set off that curve is one).
fn distance_to_centres(p: Vec3, alpha: f64, (a, s1): (f64, f64), (b, s2): (f64, f64)) -> f64 {
    let d = stay_axis(alpha);
    let inside = |c: Vec3| s1 * (rho_post(c) - a) >= -1e-12 && s2 * (rho_stay(c, d) - b) >= -1e-12;
    if inside(p) {
        return 0.0;
    }
    let mut best = f64::INFINITY;
    let radial = Vec3::new(p.x, p.y, 0.0).normalized().unwrap();
    let on_post = Vec3::new(radial.x * a, radial.y * a, p.z);
    if inside(on_post) {
        best = best.min(p.distance(on_post));
    }
    let axis_point = d * p.dot(d);
    let on_stay = axis_point + (p - axis_point).normalized().unwrap() * b;
    if inside(on_stay) {
        best = best.min(p.distance(on_stay));
    }
    // The curve: c = (a cos t, a sin t, z) with rho_stay(c) = b, a
    // quadratic in z; both roots, on the stay's side.
    let (sa, ca) = (alpha.sin(), alpha.cos());
    let points = |t: f64| -> [Option<Vec3>; 2] {
        let qa = sa * sa;
        let qb = -2.0 * a * t.cos() * sa * ca;
        let qc = a * a * (1.0 - t.cos() * t.cos() * sa * sa) - b * b;
        let disc = qb * qb - 4.0 * qa * qc;
        if disc < 0.0 {
            return [None, None];
        }
        let root = |sign: f64| {
            let z = (-qb + sign * disc.sqrt()) / (2.0 * qa);
            let c = Vec3::new(a * t.cos(), a * t.sin(), z);
            (c.dot(d) > 0.0).then_some(c)
        };
        [root(-1.0), root(1.0)]
    };
    let count = 720;
    let step = std::f64::consts::TAU / f64::from(count);
    for branch in 0..2 {
        let at = |t: f64| points(t)[branch].map_or(f64::INFINITY, |c| p.distance(c));
        let (mut t_best, mut d_best) = (0.0, f64::INFINITY);
        for i in 0..count {
            let t = f64::from(i) * step;
            let dist = at(t);
            if dist < d_best {
                (t_best, d_best) = (t, dist);
            }
        }
        if !d_best.is_finite() {
            continue;
        }
        // Golden-section refinement round the best sample.
        let (mut lo, mut hi) = (t_best - step, t_best + step);
        let g = 0.5 * (5.0_f64.sqrt() - 1.0);
        for _ in 0..80 {
            let (m1, m2) = (hi - g * (hi - lo), lo + g * (hi - lo));
            if at(m1) < at(m2) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        best = best.min(at(0.5 * (lo + hi)).min(d_best));
    }
    best
}

/// The grid round the weld (local coordinates, unit scale): a box about
/// the curve where the two cylinders meet, grown by two radii.
fn grid(alpha: f64, r: f64) -> Vec<Vec3> {
    let d = stay_axis(alpha);
    let (mut lo, mut hi) = (
        Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for i in 0..720 {
        let t = std::f64::consts::TAU * f64::from(i) / 720.0;
        // Round the stay: its surface points, followed in to the post.
        let e1 = d.cross(Vec3::Y).normalized().unwrap();
        let e2 = d.cross(e1);
        let off = (e1 * t.cos() + e2 * t.sin()) * STAY;
        // Solve rho_post(off + d s) = POST for s > 0.
        let (qa, qb, qc) = (
            d.x * d.x + d.y * d.y,
            2.0 * (off.x * d.x + off.y * d.y),
            off.x * off.x + off.y * off.y - POST * POST,
        );
        let s = (-qb + (qb * qb - 4.0 * qa * qc).sqrt()) / (2.0 * qa);
        let c = off + d * s;
        lo = Vec3::new(lo.x.min(c.x), lo.y.min(c.y), lo.z.min(c.z));
        hi = Vec3::new(hi.x.max(c.x), hi.y.max(c.y), hi.z.max(c.z));
    }
    let grow = 2.0 * r;
    let n = 9;
    let mut points = Vec::new();
    for i in 0..n {
        for j in 0..n {
            for l in 0..n {
                let f = |m: usize, a: f64, b: f64| {
                    (a - grow) + (b - a + 2.0 * grow) * (m as f64 + 0.37) / n as f64
                };
                points.push(Vec3::new(
                    f(i, lo.x, hi.x),
                    f(j, lo.y, hi.y),
                    f(l, lo.z, hi.z),
                ));
            }
        }
    }
    points
}

/// The weld of the tee (`op` union: concave, material added; subtract:
/// the mouth of a cross hole, convex) by `r`, held against the
/// definition: a point is in the welded union exactly when it is in the
/// union or no ball of the radius outside both cylinders reaches it; a
/// point is in the rounded hole's body exactly when a ball of the radius
/// inside the post and outside the hole holds it.
fn weld_by_definition(alpha: f64, r: f64, op: BooleanOp) {
    let concave = op == BooleanOp::Union;
    let d = stay_axis(alpha);
    for k in SCALES {
        let tol = tolerance(k);
        let body = tee(k, alpha, op);
        let edge = weld_edge(&body);
        let made = fillet(
            &body,
            &[FilletEdge {
                edge,
                radius: r * k,
            }],
            &tol,
        )
        .unwrap_or_else(|e| panic!("alpha {alpha}, r {r}, scale {k}: {e}"));
        assert_eq!(check(&made.body, &tol), []);
        assert_eq!(made.body.faces.len(), body.faces.len() + 1);
        let (before, after) = (mass(&body).volume, mass(&made.body).volume);
        assert!(if concave {
            after > before
        } else {
            after < before
        });
        let t = tilted(k);
        let margin = 1e-3 * r;
        let mut points = Vec::new();
        let mut expected = Vec::new();
        for p in grid(alpha, r) {
            let along = p.dot(d);
            let (in_post, in_stay) = (rho_post(p) < POST, rho_stay(p, d) < STAY && along > 0.0);
            let near_face = (rho_post(p) - POST).abs() < margin
                || ((rho_stay(p, d) - STAY).abs() < margin && along > 0.0);
            let (want, dist) = if concave {
                let dist = distance_to_centres(p, alpha, (POST + r, 1.0), (STAY + r, 1.0));
                (in_post || in_stay || dist >= r, dist)
            } else {
                let dist = distance_to_centres(p, alpha, (POST - r, -1.0), (STAY + r, 1.0));
                (in_post && !in_stay && dist <= r, dist)
            };
            if near_face || (dist - r).abs() < margin {
                continue;
            }
            points.push(t.point(p.x * k, p.y * k, p.z * k));
            expected.push(want);
        }
        let found = contains_points(&made.body, &points, &tol).unwrap();
        let mut undecided = 0;
        let mut differ = 0;
        for ((p, want), got) in points.iter().zip(&expected).zip(&found) {
            match got {
                Some(got) if got != want => {
                    differ += 1;
                    eprintln!("at {p:?}: want {want}, got {got}");
                }
                Some(_) => {}
                None => undecided += 1,
            }
        }
        assert_eq!(differ, 0, "alpha {alpha}, r {r}, scale {k}");
        assert!(undecided * 100 < points.len(), "{undecided} undecided");
    }
}

/// Bike finding B7: a tube welded square into a tube.
#[test]
fn a_tube_welded_square_into_a_tube() {
    weld_by_definition(std::f64::consts::FRAC_PI_2, 2.0, BooleanOp::Union);
}

/// The same at 54° and 35° to the post, a small and a large weld.
#[test]
fn a_tube_welded_at_a_slant_into_a_tube() {
    weld_by_definition(54_f64.to_radians(), 0.5, BooleanOp::Union);
    weld_by_definition(35_f64.to_radians(), 4.0, BooleanOp::Union);
}

/// The mouth of a blind cross hole rounded: the same edge, convex.
#[test]
fn the_mouth_of_a_cross_hole_rounded() {
    weld_by_definition(std::f64::consts::FRAC_PI_2, 1.5, BooleanOp::Subtract);
    weld_by_definition(60_f64.to_radians(), 1.0, BooleanOp::Subtract);
}
