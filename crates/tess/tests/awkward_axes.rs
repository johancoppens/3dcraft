//! Solids of revolution on axes that are not the world's: the circles at
//! the two ends of a wall then start a rounding error apart in angle (the
//! kernel re-orthonormalises the frame of the second one), so a point on
//! the seam of the surface projects to u = 0 on one loop and to just
//! below 2π on the other. The tessellator must not care: it follows u
//! along each loop by continuity and never trusts one projection alone.
//!
//! Regression: the kernel's cylinder (`craft3d_kernel::primitives`, here
//! through `testkit::solids`) on the axis (1, 0, 0.4) failed with
//! "a cylindrical face whose two boundary circles do not line up".

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_testkit::solids;
use craft3d_topo::{Body, check, mass_properties};
use std::f64::consts::PI;

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    }
}

/// Closed, outward, on the surface, within tolerance, and of the volume
/// `expected` to within area × tolerance.
fn assert_solid(body: &Body, mesh: &Mesh, tolerance: f64, expected: f64, area: f64, what: &str) {
    let welded = mesh.weld().unwrap();
    assert!(welded.is_closed(), "{what}: not a closed manifold");
    assert_eq!(welded.triangles.len(), mesh.triangles.len(), "{what}");
    for (t, &face_id) in mesh.face_ids.iter().enumerate() {
        let face = &body.faces[face_id as usize];
        let [a, b, c] = mesh.corners(t).unwrap();
        let measure = |p: Vec3| {
            let (u, v) = face.surface.project(p).unwrap();
            (
                face.surface.eval(u, v).distance(p),
                face.surface.normal(u, v),
            )
        };
        for p in [a, b, c] {
            assert!(
                measure(p).0 <= 1e-9 * expected.cbrt(),
                "{what}: vertex off its surface"
            );
        }
        let centroid = (a + b + c) * (1.0 / 3.0);
        for p in [a.lerp(b, 0.5), b.lerp(c, 0.5), c.lerp(a, 0.5), centroid] {
            let off = measure(p).0;
            assert!(
                off <= tolerance * (1.0 + 1e-9),
                "{what}: triangle {t} is {off:e} from the surface"
            );
        }
        let facing = (b - a).cross(c - a);
        assert!(facing.length() > 0.0, "{what}: triangle {t} has no area");
        if let Some(normal) = measure(centroid).1 {
            let outward = normal * if face.reversed { -1.0 } else { 1.0 };
            assert!(
                facing.dot(outward) > 0.0,
                "{what}: triangle {t} faces inward"
            );
        }
    }
    let volume = welded.volume();
    assert!(
        (volume - expected).abs() <= area * tolerance && volume > 0.0,
        "{what}: volume {volume} vs {expected}"
    );
}

/// The exact case that was reported.
#[test]
fn a_cylinder_on_a_tilted_axis_tessellates() {
    let frame = Frame::from_z(Vec3::new(0.0, 0.0, 20.0), Vec3::new(1.0, 0.0, 0.4)).unwrap();
    let body = solids::cylinder(&frame, 3.0, 40.0).unwrap().body;
    assert_eq!(check(&body, &Tolerance::default()), vec![]);
    let exact = mass_properties(&body).unwrap();
    assert!((exact.volume - PI * 9.0 * 40.0).abs() < 1e-9 * exact.volume);
    let tolerance = 0.07;
    let mesh = tessellate_body(&body, &params(tolerance)).unwrap();
    assert_solid(
        &body,
        &mesh,
        tolerance,
        PI * 9.0 * 40.0,
        2.0 * PI * 3.0 * (3.0 + 40.0),
        "the reported cylinder",
    );
    // Its two circles have the same number of points, so the wall needs
    // nothing but them: two triangles per segment, as on a straight axis.
    let straight = solids::cylinder(&Frame::WORLD, 3.0, 40.0).unwrap().body;
    let reference = tessellate_body(&straight, &params(tolerance)).unwrap();
    assert_eq!(mesh.triangles.len(), reference.triangles.len());
    assert_eq!(mesh.positions.len(), reference.positions.len());
}

#[test]
fn solids_of_revolution_tessellate_on_awkward_axes() {
    let axes = [
        Vec3::new(1.0, 0.0, 0.4),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(0.0, 1e-9, 1.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(-0.3, 0.7, -0.2),
        Vec3::new(1e-7, 1.0, 1e-7),
        Vec3::new(3.0, -4.0, 12.0),
    ];
    let origins = [Vec3::new(0.0, 0.0, 20.0), Vec3::new(-7.3, 11.1, 0.37)];
    for axis in axes {
        for origin in origins {
            let frame = Frame::from_z(origin, axis).unwrap();
            let (r, h) = (3.0, 40.0);
            let all = [
                solids::cylinder(&frame, r, h).unwrap(),
                solids::cone(&frame, r, h).unwrap(),
                solids::torus(&frame, 7.0, 2.0).unwrap(),
                solids::sphere(&frame, r).unwrap(),
                solids::hemisphere(&frame, r).unwrap(),
            ];
            // The closed forms, written out: independent of testkit's.
            let expected = [
                PI * r * r * h,
                PI * r * r * h / 3.0,
                2.0 * PI * PI * 7.0 * 4.0,
                4.0 / 3.0 * PI * r * r * r,
                2.0 / 3.0 * PI * r * r * r,
            ];
            for (solid, volume) in all.into_iter().zip(expected) {
                assert!((solid.volume - volume).abs() < 1e-12 * volume);
                let (name, body, area) = (solid.name, solid.body, solid.area);
                for tolerance in [0.07, 0.0031] {
                    let what =
                        format!("{name} on axis {axis:?} at {origin:?}, tolerance {tolerance}");
                    assert_eq!(check(&body, &Tolerance::default()), vec![], "{what}");
                    let mesh = tessellate_body(&body, &params(tolerance))
                        .unwrap_or_else(|e| panic!("{what}: {e}"));
                    assert_solid(&body, &mesh, tolerance, volume, area, &what);
                }
            }
        }
    }
}

/// The same hazard on trimmed solids: everything in `trimmed::all` on the
/// awkward axes.
#[test]
fn trimmed_solids_tessellate_on_awkward_axes() {
    use craft3d_testkit::trimmed;
    for axis in [
        Vec3::new(1.0, 0.0, 0.4),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(0.0, 1e-9, 1.0),
        Vec3::new(-1.0, 0.0, 0.0),
    ] {
        let frame = Frame::from_z(Vec3::new(0.0, 0.0, 20.0), axis).unwrap();
        let mut all = trimmed::all(&frame, 1.0).unwrap();
        all.push(trimmed::pipe_elbow(&frame, 3.0, 1.0, 4.0).unwrap());
        for solid in all {
            let tolerance = 0.007;
            let what = format!("{} on axis {axis:?}", solid.name);
            let mesh = tessellate_body(&solid.body, &params(tolerance))
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_solid(
                &solid.body,
                &mesh,
                tolerance,
                solid.volume,
                solid.area,
                &what,
            );
        }
    }
}
