//! The Utah teapot's 32 Bézier patches against planes: every crossing of a
//! patch's iso-lines with the plane lies on a returned curve.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use craft3d_geom::Surface;
use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
use craft3d_math::{Frame, Vec3};
use craft3d_testkit::teapot::teapot;

/// Where iso-lines of a patch cross a plane, by fine sampling and
/// bisection. Lines a hair off the round fractions, so that none runs
/// along a patch edge.
fn iso_roots(surface: &Surface, origin: Vec3, normal: Vec3) -> Vec<Vec3> {
    let mut roots = Vec::new();
    for along_u in [true, false] {
        for line in 0..24 {
            let fixed = (f64::from(line) + 0.37) / 24.0;
            let at = |x: f64| if along_u { (x, fixed) } else { (fixed, x) };
            let value = |x: f64| (surface.eval(at(x).0, at(x).1) - origin).dot(normal);
            for k in 0..400 {
                let (mut lo, mut hi) = (f64::from(k) / 400.0, f64::from(k + 1) / 400.0);
                if (value(lo) >= 0.0) == (value(hi) >= 0.0) {
                    continue;
                }
                for _ in 0..60 {
                    let mid = 0.5 * (lo + hi);
                    if (value(mid) >= 0.0) == (value(lo) >= 0.0) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let (u, v) = at(0.5 * (lo + hi));
                roots.push(surface.eval(u, v));
            }
        }
    }
    roots
}

#[test]
fn planes_cut_every_patch_of_the_teapot() {
    let patches = teapot().unwrap();
    let t = tol(1.0);
    let within = cube(Vec3::new(0.0, 0.0, 1.5), 6.0);
    let planes = [
        (Vec3::new(0.0, 0.0, 1.2), Vec3::new(0.03, 0.02, 1.0)),
        (Vec3::new(0.0, 0.0, 2.3), Vec3::new(0.1, -0.05, 1.0)),
        (Vec3::new(0.35, 0.0, 0.0), Vec3::new(1.0, 0.07, 0.04)),
        (Vec3::new(0.0, 0.2, 0.0), Vec3::new(0.02, 1.0, 0.05)),
        (Vec3::new(0.0, 0.0, 2.9), Vec3::new(0.3, 0.2, 1.0)),
    ];
    let (mut crossing, mut apart, mut curves, mut refused) = (0, 0, 0, Vec::new());
    for (index, patch) in patches.iter().enumerate() {
        let surface = Surface::Bezier(patch.clone());
        for (which, (origin, normal)) in planes.iter().enumerate() {
            let normal = normal.normalized().unwrap();
            let cut = plane(Frame::from_z(*origin, normal).unwrap());
            let found = meet_surfaces(&cut, &surface, &within, &t);
            assert_eq!(found, meet_surfaces(&surface, &cut, &within, &t));
            let roots = iso_roots(&surface, *origin, normal);
            match found {
                Err(e) => refused.push(format!("patch {index}, plane {which}: {e}")),
                Ok(SurfaceMeeting::None) => {
                    assert!(roots.is_empty(), "patch {index}, plane {which}: missed");
                    apart += 1;
                }
                Ok(SurfaceMeeting::Coincident { .. }) => panic!("coincident"),
                Ok(SurfaceMeeting::Meets(m)) => {
                    check_meeting(&m, &cut, &surface, &within, &t);
                    let lines = Lines::of(&m, 3000);
                    for p in &roots {
                        assert!(
                            lines.distance(*p) <= 1e-5,
                            "patch {index}, plane {which}: the crossing at {p:?} is on no curve"
                        );
                    }
                    crossing += 1;
                    curves += m.curves.len();
                }
            }
        }
    }
    eprintln!(
        "teapot: {crossing} patch/plane pairs cross in {curves} curves, {apart} apart, {} refused",
        refused.len()
    );
    for line in &refused {
        eprintln!("  {line}");
    }
    assert!(refused.is_empty());
    assert!(crossing >= 40);
}
