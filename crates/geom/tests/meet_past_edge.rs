//! A meeting curve that leaves the other surface over its edge and comes
//! back (2026-10-28, the sea kayak's inner hull, repro 20).
//!
//! A march takes steps and looks at their ends; the field of a freeform
//! surface goes on past the surface's edge, so a step can pass over a
//! stretch where `f = 0` runs past the edge and back. The refinement then
//! puts points there that no span through them can bring within the
//! tolerance of the bounded surface, and the intersection was refused
//! ("an intersection curve could not be fitted within the tolerance").
//! Now the branch is divided where it leaves the surface, as a march
//! that met the edge ends there.
//!
//! The case: a flat patch `z = 0` and a vertical wall in the plane
//! `y = 0` whose lower border dips below `z = 0` everywhere but over a
//! few narrow bumps, where it rises above it. The meeting is the line
//! `y = z = 0` where the wall reaches below `z = 0`: pieces between the
//! bumps, whose lengths are known from where the border crosses `z = 0`.
//! At 1e-3, 1 and 1e6, with the tolerance scaled.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::intersect::{MeetingCurve, SurfaceMeeting, meet_surfaces};
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_math::{Bbox3, Tolerance, Vec3};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
/// Control points along the wall's lower border, 0.05 apart.
const COUNT: usize = 201;
/// The bumps' middles (indices of the control points lifted), placed
/// unevenly so that steps of any one length pass over some of them.
const BUMPS: [usize; 5] = [41, 73, 104, 131, 167];

fn floor(k: f64) -> Surface {
    let ctrl = vec![
        Vec3::new(-1.0, -1.0, 0.0) * k,
        Vec3::new(-1.0, 1.0, 0.0) * k,
        Vec3::new(11.0, -1.0, 0.0) * k,
        Vec3::new(11.0, 1.0, 0.0) * k,
    ];
    Surface::Nurbs(
        NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            2,
            ctrl,
            vec![1.0; 4],
        )
        .unwrap(),
    )
}

/// The wall: degree 3 along x over 0..10, ruled up to `z = 1`; its lower
/// border at `z = -0.3` but for three control points at each bump lifted
/// to `z = 0.4`, `0.6`, `0.4` (a bump about 0.2 wide above `z = 0`).
fn wall(k: f64) -> Surface {
    let mut ctrl = Vec::new();
    for i in 0..COUNT {
        let x = 10.0 * i as f64 / (COUNT - 1) as f64;
        let lift = BUMPS
            .iter()
            .map(|&b| match i.abs_diff(b) {
                0 => 0.9,
                1 => 0.7,
                _ => 0.0,
            })
            .fold(0.0, f64::max);
        ctrl.push(Vec3::new(x, 0.0, -0.3 + lift) * k);
        ctrl.push(Vec3::new(x, 0.0, 1.0) * k);
    }
    let mut knots = vec![0.0; 4];
    for j in 1..COUNT - 3 {
        knots.push(j as f64 / (COUNT - 3) as f64);
    }
    knots.extend([1.0; 4]);
    Surface::Nurbs(
        NurbsSurface::new(
            3,
            1,
            knots,
            vec![0.0, 0.0, 1.0, 1.0],
            COUNT,
            2,
            ctrl,
            vec![1.0; 2 * COUNT],
        )
        .unwrap(),
    )
}

/// The stretches of `x` over which the wall's lower border is below
/// `z = 0` (bisection on its sign along the border, sampled finely).
fn below(wall: &Surface, k: f64) -> Vec<(f64, f64)> {
    let n = 20_000;
    let z = |u: f64| wall.eval(u, 0.0).z;
    let crossing = |mut lo: f64, mut hi: f64| {
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if (z(mid) < 0.0) == (z(lo) < 0.0) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    };
    let mut stretches = Vec::new();
    let mut start = Some(0.0);
    for i in 1..=n {
        let (a, b) = ((i - 1) as f64 / n as f64, i as f64 / n as f64);
        match ((z(a) < 0.0), (z(b) < 0.0)) {
            (true, false) => {
                let u = crossing(a, b);
                stretches.push((start.take().unwrap(), wall.eval(u, 0.0).x / k));
            }
            (false, true) => start = Some(wall.eval(crossing(a, b), 0.0).x / k),
            _ => {}
        }
    }
    if let Some(s) = start {
        stretches.push((s, 10.0));
    }
    stretches
}

fn everywhere(k: f64) -> Bbox3 {
    Bbox3::from_points([
        Vec3::new(-2.0, -2.0, -2.0) * k,
        Vec3::new(12.0, 2.0, 2.0) * k,
    ])
}

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// Distance from `p` to the bounded surface (its parameters clamped).
fn off(s: &Surface, p: Vec3) -> f64 {
    let (u, v) = s.project(p).unwrap();
    let ((u0, u1), (v0, v1)) = s.domain();
    s.eval(u.clamp(u0, u1), v.clamp(v0, v1)).distance(p)
}

#[test]
fn a_meeting_that_leaves_the_other_surface_over_its_edge_and_comes_back_is_divided_there() {
    for k in SCALES {
        let (a, b) = (floor(k), wall(k));
        let tol = tolerance(k).linear;
        let expected = below(&b, k);
        assert_eq!(expected.len(), BUMPS.len() + 1, "{k}: {expected:?}");
        let curves: Vec<MeetingCurve> = match meet_surfaces(&a, &b, &everywhere(k), &tolerance(k)) {
            Ok(SurfaceMeeting::Meets(m)) => m.curves,
            other => panic!("{k}: {other:?}"),
        };
        // Each curve on both bounded surfaces, sampled.
        let mut spans = Vec::new();
        for c in &curves {
            let (t0, t1) = c.range;
            for i in 0..=64 {
                let p = c.curve.eval(t0 + (t1 - t0) * f64::from(i) / 64.0);
                assert!(off(&a, p) <= tol, "{k}: off the floor at {p:?}");
                assert!(off(&b, p) <= tol, "{k}: off the wall at {p:?}");
                assert!(p.y.abs() <= tol && p.z.abs() <= tol, "{k}: {p:?}");
            }
            let (x0, x1) = (c.curve.eval(t0).x / k, c.curve.eval(t1).x / k);
            spans.push((x0.min(x1), x0.max(x1)));
        }
        spans.sort_by(|p, q| p.0.total_cmp(&q.0));
        // One curve per stretch, ending where the border crosses `z = 0`
        // (to the tolerance over the border's slope there, at least one
        // in a thousand).
        assert_eq!(
            spans.len(),
            expected.len(),
            "{k}: {spans:?} against {expected:?}"
        );
        for (got, want) in spans.iter().zip(&expected) {
            let slack = 1e3 * tol / k;
            assert!(
                (got.0 - want.0).abs() <= slack && (got.1 - want.1).abs() <= slack,
                "{k}: {got:?} against {want:?}"
            );
        }
    }
}
