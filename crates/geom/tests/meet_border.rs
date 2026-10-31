//! A stretch of a border of a freeform surface that lies on the other
//! surface is a meeting curve (2026-10-27, the sea kayak's touching
//! plates, repro 3 and 12).
//!
//! Two thin ruled strips (degree 3 along, 1 across) share their border
//! over the middle of its parameters, `u` in [−1, 1], where they have the
//! same control points and the end ones' basis functions vanish; past
//! that, the end control points of the second are moved by `δ` and the
//! borders part. Whether the strips then also part, or go on crossing,
//! depends on the side the border is moved to. The same against a plane
//! that holds the middle of a strip's border. Every case at 1e-3, 1 and
//! 1e6, with the tolerance scaled.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::intersect::{MeetingCurve, SurfaceMeeting, meet_surfaces};
use craft3d_geom::{Curve3, NurbsSurface, Plane, Surface};
use craft3d_math::Portable;
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
const KNOTS_U: [f64; 10] = [-1.2, -1.2, -1.2, -1.2, -1.0, 1.0, 1.2, 1.2, 1.2, 1.2];

/// The shared border's control points (a curve that bends both ways).
fn border(k: f64) -> [Vec3; 6] {
    [
        Vec3::new(-1.2, 0.0, 0.05),
        Vec3::new(-1.0, 0.0, -0.1),
        Vec3::new(-0.3, 0.1, 0.15),
        Vec3::new(0.4, -0.1, 0.05),
        Vec3::new(1.0, 0.0, 0.2),
        Vec3::new(1.2, 0.0, 0.1),
    ]
    .map(|p| p * k)
}

/// A ruled strip from `row` across by `across` (one vector per control
/// point).
fn strip(row: &[Vec3; 6], across: &[Vec3; 6]) -> Surface {
    let mut ctrl = Vec::new();
    for i in 0..6 {
        ctrl.push(row[i]);
        ctrl.push(row[i] + across[i]);
    }
    Surface::Nurbs(
        NurbsSurface::new(
            3,
            1,
            KNOTS_U.to_vec(),
            vec![0.0, 0.0, 1.0, 1.0],
            6,
            2,
            ctrl,
            vec![1.0; 12],
        )
        .unwrap(),
    )
}

/// The first strip, about `0.01 k` wide and standing up along z with a
/// little twist.
fn across_a(k: f64) -> [Vec3; 6] {
    std::array::from_fn(|i| Vec3::new(0.0, -0.2 * (i as f64).psin(), 1.0) * (0.01 * k))
}

/// The second strip's across, the first's turned by 0.5 about x towards
/// +y.
fn across_b(k: f64) -> [Vec3; 6] {
    let (s, c) = (-0.5_f64).psin_cos();
    across_a(k).map(|a| Vec3::new(a.x, c * a.y - s * a.z, s * a.y + c * a.z))
}

fn everywhere(k: f64) -> Bbox3 {
    Bbox3::from_points([
        Vec3::new(-2.0, -1.0, -1.0) * k,
        Vec3::new(2.0, 1.0, 1.0) * k,
    ])
}

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn curves(a: &Surface, b: &Surface, k: f64) -> Vec<MeetingCurve> {
    match meet_surfaces(a, b, &everywhere(k), &tolerance(k)) {
        Ok(SurfaceMeeting::Meets(m)) => m.curves,
        other => panic!("{k}: {other:?}"),
    }
}

/// The curves in brief: kind, range, ends and deviation.
fn describe(found: &[MeetingCurve]) -> String {
    found
        .iter()
        .map(|c| {
            format!(
                "\n  {} {:?} from {:?} to {:?} dev {:e}",
                c.curve.kind(),
                c.range,
                c.curve.eval(c.range.0),
                c.curve.eval(c.range.1),
                c.deviation
            )
        })
        .collect()
}

fn off_surface(s: &Surface, p: Vec3) -> f64 {
    let (u, v) = s.project(p).unwrap();
    s.eval(u, v).distance(p)
}

fn off_curve(c: &MeetingCurve, p: Vec3) -> f64 {
    let t = c.curve.project(p).unwrap().clamp(c.range.0, c.range.1);
    c.curve.eval(t).distance(p)
}

/// Every curve lies within its deviation of both surfaces (sampled), the
/// border over `u` in [−0.99, 0.99] is covered by exactly one of them, and
/// the curves' points, past the stretch, keep to `u` within `reach`.
fn check(found: &[MeetingCurve], a: &Surface, b: &Surface, k: f64, what: &str) {
    sound(found, a, b, k, what);
    covered(found, a, k, what, &|_| true);
}

/// Every curve lies within its deviation of both surfaces (sampled), is
/// a curve (more than a few tolerances long: no two-point piece where a
/// branch meets a stretch), and no two run together but near the ends
/// they share.
fn sound(found: &[MeetingCurve], a: &Surface, b: &Surface, k: f64, what: &str) {
    let tol = tolerance(k).linear;
    for c in found {
        let length: f64 = (0..200)
            .map(|i| {
                let at = |j: i32| c.range.0 + (c.range.1 - c.range.0) * f64::from(j) / 200.0;
                c.curve.eval(at(i)).distance(c.curve.eval(at(i + 1)))
            })
            .sum();
        assert!(
            length > 4.0 * tol,
            "{what} at {k}: a curve {length} long: {}",
            describe(found)
        );
        assert!(
            c.deviation <= tol,
            "{what} at {k}: deviation {}",
            c.deviation
        );
        for i in 0..=200 {
            let t = c.range.0 + (c.range.1 - c.range.0) * f64::from(i) / 200.0;
            let p = c.curve.eval(t);
            assert!(off_surface(a, p) <= tol, "{what} at {k}: {p:?} off a");
            assert!(off_surface(b, p) <= tol, "{what} at {k}: {p:?} off b");
        }
    }
    // No two curves run together (a marched branch beside the border's
    // stretch), but near the ends they share.
    for (i, c) in found.iter().enumerate() {
        let ends = [c.curve.eval(c.range.0), c.curve.eval(c.range.1)];
        for j in 0..=400 {
            let t = c.range.0 + (c.range.1 - c.range.0) * f64::from(j) / 400.0;
            let p = c.curve.eval(t);
            for (other_index, other) in found.iter().enumerate() {
                if other_index == i {
                    continue;
                }
                let other_ends = [
                    other.curve.eval(other.range.0),
                    other.curve.eval(other.range.1),
                ];
                if ends
                    .iter()
                    .chain(&other_ends)
                    .any(|e| e.distance(p) <= 64.0 * tol)
                {
                    continue;
                }
                assert!(
                    off_curve(other, p) > 4.0 * tol,
                    "{what} at {k}: curves {i} and {other_index} together at {p:?}: {}",
                    describe(found)
                );
            }
        }
    }
}

/// The border of `a` over `u` in [−0.99, 0.99], where `at` holds, is
/// covered by exactly one curve.
fn covered(found: &[MeetingCurve], a: &Surface, k: f64, what: &str, at: &dyn Fn(f64) -> bool) {
    let tol = tolerance(k).linear;
    for i in 0..=100 {
        let u = -0.99 + 1.98 * f64::from(i) / 100.0;
        if !at(u) {
            continue;
        }
        let p = a.eval(u, 0.0);
        let near = found
            .iter()
            .filter(|c| off_curve(c, p) <= 4.0 * tol)
            .count();
        assert_eq!(
            near,
            1,
            "{what} at {k}: u {u} near {near} curves: {}",
            describe(found)
        );
    }
}

fn moved(row: &[Vec3; 6], by: Vec3) -> [Vec3; 6] {
    let mut row = *row;
    row[0] += by;
    row[5] += by;
    row
}

#[test]
fn strips_that_share_the_middle_of_their_border_and_part_meet_along_it() {
    for k in SCALES {
        let row = border(k);
        let a = strip(&row, &across_a(k));
        // The second strip leans to +y; its border moved to +y as well
        // keeps it off the first past the stretch.
        let b = strip(&moved(&row, Vec3::new(0.0, 0.05, 0.0) * k), &across_b(k));
        for (x, y, what) in [(&a, &b, "a b"), (&b, &a, "b a")] {
            let found = curves(x, y, k);
            check(&found, &a, &b, k, what);
            // Nothing past where the borders part, but for the few
            // tolerances over which they still agree.
            for c in &found {
                for i in 0..=200 {
                    let t = c.range.0 + (c.range.1 - c.range.0) * f64::from(i) / 200.0;
                    let p = c.curve.eval(t);
                    assert!(p.x.abs() <= 1.03 * k, "{what} at {k}: {p:?}");
                }
            }
        }
    }
}

#[test]
fn strips_that_share_the_middle_of_their_border_and_go_on_crossing_meet_past_it() {
    for k in SCALES {
        let row = border(k);
        let a = strip(&row, &across_a(k));
        // Moved to −y, the second strip's border goes under the first,
        // and the second strip, leaning to +y, crosses it.
        let b = strip(&moved(&row, Vec3::new(0.0, -0.002, 0.0) * k), &across_b(k));
        for (x, y, what) in [(&a, &b, "a b"), (&b, &a, "b a")] {
            let found = curves(x, y, k);
            check(&found, &a, &b, k, what);
            // Where they go on crossing, at u = ±1.1 on the first strip:
            // the point of its ruling there on the second.
            for u in [-1.1, 1.1] {
                let (lo, hi) = (a.eval(u, 0.0), a.eval(u, 1.0));
                let side = |p: Vec3| {
                    let (bu, bv) = b.project(p).unwrap();
                    let n = b.normal(bu, bv).unwrap();
                    (p - b.eval(bu, bv)).dot(n)
                };
                let (mut s0, mut s1) = (0.0, 1.0);
                let f0 = side(lo);
                assert!(f0 * side(hi) < 0.0, "{what} at {k}: no crossing at {u}");
                for _ in 0..80 {
                    let m = 0.5 * (s0 + s1);
                    if side(lo.lerp(hi, m)) * f0 > 0.0 {
                        s0 = m;
                    } else {
                        s1 = m;
                    }
                }
                let p = lo.lerp(hi, 0.5 * (s0 + s1));
                let tol = tolerance(k).linear;
                assert!(
                    found.iter().any(|c| off_curve(c, p) <= 4.0 * tol),
                    "{what} at {k}: {p:?} on no curve: {}",
                    describe(&found)
                );
            }
        }
    }
}

#[test]
fn borders_that_agree_only_within_the_tolerance_still_meet_along_them() {
    // The kayak's two plates trace their common edge each for itself, so
    // the borders agree to a quarter of the tolerance, either side.
    for k in SCALES {
        let row = border(k);
        let a = strip(&row, &across_a(k));
        let tol = tolerance(k).linear;
        let mut other = moved(&row, Vec3::new(0.0, 0.05, 0.0) * k);
        for (i, p) in other.iter_mut().enumerate().take(5).skip(1) {
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            *p += Vec3::new(0.0, sign * 0.12 * tol, 0.1 * tol);
        }
        let b = strip(&other, &across_b(k));
        for (x, y, what) in [(&a, &b, "a b"), (&b, &a, "b a")] {
            let found = curves(x, y, k);
            check(&found, &a, &b, k, what);
        }
    }
}

#[test]
fn a_plane_that_holds_the_middle_of_a_border_meets_it_there() {
    for k in SCALES {
        // The border's middle control points in z = 0, the ends lifted
        // (parting) or lowered (crossing on).
        for (lift, crossing) in [(0.05, false), (-0.002, true)] {
            let mut row = border(k).map(|p| Vec3::new(p.x, p.z, 0.0));
            row[0].z = lift * k;
            row[5].z = lift * k;
            let a = strip(
                &row,
                &std::array::from_fn(|_| Vec3::new(0.0, 0.3, 1.0) * (0.01 * k)),
            );
            let plane = Surface::Plane(Plane::new(Frame::WORLD));
            let what = if crossing { "crossing" } else { "parting" };
            let found = curves(&a, &plane, k);
            check(&found, &a, &plane, k, what);
            let tol = tolerance(k).linear;
            for u in [-1.1, 1.1] {
                // On the ruling at u, z = 0 at v = −z(u, 0) / dz.
                let (lo, hi) = (a.eval(u, 0.0), a.eval(u, 1.0));
                let v = -lo.z / (hi.z - lo.z);
                let on = found
                    .iter()
                    .any(|c| off_curve(c, lo.lerp(hi, v)) <= 4.0 * tol);
                assert_eq!(
                    on,
                    crossing,
                    "{what} at {k}: u {u}, v {v}: {}",
                    describe(&found)
                );
            }
        }
    }
}

/// The stretch is a curve of its own kind: the border's iso-curve, not a
/// marched one (a NURBS of the strip's degree along the border).
#[test]
fn the_stretch_is_the_borders_own_curve() {
    let k = 1.0;
    let row = border(k);
    let a = strip(&row, &across_a(k));
    let b = strip(&moved(&row, Vec3::new(0.0, 0.05, 0.0) * k), &across_b(k));
    let found = curves(&a, &b, k);
    let along = found
        .iter()
        .find(|c| off_curve(c, a.eval(0.0, 0.0)) <= 4e-6)
        .unwrap();
    let Curve3::Nurbs(n) = &along.curve else {
        panic!("{along:?}");
    };
    assert_eq!(n.degree(), 3, "{along:?}");
    assert!(
        along.range.0 <= -1.0 + 1e-9 && along.range.1 >= 1.0 - 1e-9,
        "{along:?}"
    );
}

/// The Greville abscissae of a clamped knot vector of `degree`.
fn greville(knots: &[f64], degree: usize) -> Vec<f64> {
    let count = knots.len() - degree - 1;
    (0..count)
        .map(|i| knots[i + 1..=i + degree].iter().sum::<f64>() / degree as f64)
        .collect()
}

/// A flat strip in the plane y = 0, from its border on z = 0 up to
/// `height` (down, when negative), over x in [−1.2, 1.2] with x its
/// parameter.
fn flat_strip(height: f64, k: f64) -> Surface {
    let xs = greville(&KNOTS_U, 3);
    let row: [Vec3; 6] = std::array::from_fn(|i| Vec3::new(xs[i], 0.0, 0.0) * k);
    strip(&row, &[Vec3::new(0.0, 0.0, height) * k; 6])
}

/// A surface at 45 degrees through the line y = z = 0 (z = y over y in
/// [−0.02, 0.02]), with a bump of `bump` (model units at scale 1) on
/// its middle rows near x = 0 that rises out of y = 0 over |x| < 0.6.
fn bumped_slope(bump: f64, k: f64) -> Surface {
    let knots_u = vec![
        -1.2, -1.2, -1.2, -1.2, -0.9, -0.6, -0.3, 0.0, 0.3, 0.6, 0.9, 1.2, 1.2, 1.2, 1.2,
    ];
    let xs = greville(&knots_u, 3);
    let mut ctrl = Vec::new();
    for (i, &x) in xs.iter().enumerate() {
        for j in 0..4 {
            let y = -0.02 + 0.04 * f64::from(j) / 3.0;
            let lift = if i == 5 && (j == 1 || j == 2) {
                bump
            } else {
                0.0
            };
            ctrl.push(Vec3::new(x, y, y + lift) * k);
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            3,
            3,
            knots_u,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            xs.len(),
            4,
            ctrl,
            vec![1.0; xs.len() * 4],
        )
        .unwrap(),
    )
}

/// A branch of its own a few tens of tolerances from a stretch is kept:
/// the slope through a strip's border, bumped up out of it by about 40
/// tolerances over |x| < 0.6, meets the strip along its border on either
/// side and in an arch of that height between, which runs within the
/// band's reach of the border near its ends. The arch's top is on a curve.
#[test]
fn a_branch_a_few_tens_of_tolerances_from_a_stretch_is_kept() {
    for k in SCALES {
        let tol = tolerance(k).linear;
        let a = flat_strip(0.01, k);
        let b = bumped_slope(80e-6, k);
        let top = b.eval(0.0, 0.5);
        assert!(top.y.abs() <= 1e-12 * k, "{top:?}");
        assert!(
            top.z > 30.0 * tol && top.z < 60.0 * tol,
            "{k}: the arch {}",
            top.z / tol
        );
        for (x, y, what) in [(&a, &b, "a b"), (&b, &a, "b a")] {
            let found = curves(x, y, k);
            sound(&found, &a, &b, k, what);
            covered(&found, &a, k, what, &|u| u.abs() > 0.62);
            for h in [0.0, -0.3, 0.3] {
                let p = b.eval(h, 0.5);
                assert!(
                    found.iter().any(|c| off_curve(c, p) <= 4.0 * tol),
                    "{what} at {k}: the arch at {h} ({:e} up) is on no curve: {}",
                    p.z / tol,
                    describe(&found)
                );
            }
        }
    }
}

/// A plane that crosses a strip's border at a tiny angle (5e-6) runs
/// within half the tolerance of the border for 0.1 either side of the
/// crossing, and on into the strip below past it: the stretch it leaves
/// on each strip is within the tolerance, and the curves of the two
/// strips together follow the plane's line all along.
#[test]
fn a_plane_that_crosses_a_border_at_a_tiny_angle_goes_on_into_the_next_strip() {
    let slope = 5e-6;
    for k in SCALES {
        let tol = tolerance(k).linear;
        let (up, down) = (flat_strip(0.01, k), flat_strip(-0.01, k));
        let normal = Vec3::new(-slope, 0.0, 1.0);
        let plane = Surface::Plane(Plane::new(
            Frame::new(Vec3::new(0.3 * k, 0.0, 0.0), normal, Vec3::Y).unwrap(),
        ));
        let on_up = curves(&up, &plane, k);
        sound(&on_up, &up, &plane, k, "up");
        let on_down = curves(&down, &plane, k);
        sound(&on_down, &down, &plane, k, "down");
        for i in 0..=88 {
            let x = -1.1 + 2.2 * f64::from(i) / 88.0;
            let p = Vec3::new(x, 0.0, slope * (x - 0.3)) * k;
            let near = |found: &[MeetingCurve]| found.iter().any(|c| off_curve(c, p) <= 4.0 * tol);
            assert!(
                near(&on_up) || near(&on_down),
                "{k}: x {x} on no curve: up {} down {}",
                describe(&on_up),
                describe(&on_down)
            );
        }
    }
}

/// A whole border that lies on the other surface only to the tolerance
/// (its control points off by 0.12 of the tolerance either way, all
/// along) is one curve, not that curve and the excursions of `f = 0`
/// across the border beside it: the band beside a whole border along
/// which the surfaces cross.
#[test]
fn a_whole_border_that_agrees_only_within_the_tolerance_meets_along_it_once() {
    for k in SCALES {
        let row = border(k);
        let a = strip(&row, &across_a(k));
        let tol = tolerance(k).linear;
        let mut other = row;
        for (i, p) in other.iter_mut().enumerate() {
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            *p += Vec3::new(0.0, sign * 0.12 * tol, 0.1 * tol);
        }
        let b = strip(&other, &across_b(k));
        for (x, y, what) in [(&a, &b, "a b"), (&b, &a, "b a")] {
            let found = curves(x, y, k);
            check(&found, &a, &b, k, what);
            assert_eq!(found.len(), 1, "{what} at {k}: {}", describe(&found));
        }
    }
}
