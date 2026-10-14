//! The angle criterion on thin tubes: a torus and a pipe elbow whose
//! tube is a hundred times thinner than its ring (a fillet of 0.6 mm on a
//! ring of 50 mm, as the turbofan's flange roots have them). Around the
//! axis the surface's normal turns slowly over a long way, across the
//! tube fast over a short one, so the cells that meet the angle are a
//! hundred times longer than they are wide. Refined in a plane scaled to
//! lengths in space, every point put into such a sliver was joined to
//! points rows away whose sides turned too far again: 41,077 triangles
//! for a turbofan fillet that needs about a thousand (2026-10-15).
//!
//! Oracles: every side not below the floor turns the normal by no more
//! than the angle (measured on the mesh's own vertex normals, which are
//! the surface's); the mesh is closed; its volume is the closed form
//! within the tolerance times the area; the same mesh on one and eight
//! threads; and the triangles stay within a small factor of a grid that
//! meets both criteria with no refinement at all.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::{Frame, Vec3};
use craft3d_tess::{ANGLE_FLOOR, Mesh, TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_testkit::solids::{self, Reference};
use craft3d_testkit::trimmed;
use std::f64::consts::{PI, TAU};

const DEGREES: f64 = PI / 180.0;

fn turn(a: Vec3, b: Vec3) -> f64 {
    a.cross(b).length().atan2(a.dot(b))
}

/// The largest turn of the normal over a side not below `floor`, and how
/// many such sides there are.
fn worst_turn(mesh: &Mesh, floor: f64) -> (f64, usize) {
    let (mut worst, mut sides) = (0.0_f64, 0);
    for tri in &mesh.triangles {
        for k in 0..3 {
            let (p, q) = (tri[k] as usize, tri[(k + 1) % 3] as usize);
            if mesh.positions[p].distance(mesh.positions[q]) < floor {
                continue;
            }
            let (n, m) = (mesh.normals[p], mesh.normals[q]);
            if n.length() == 0.0 || m.length() == 0.0 {
                continue;
            }
            sides += 1;
            worst = worst.max(turn(n, m));
        }
    }
    (worst, sides)
}

/// Triangles of a grid over `sweep` of the ring and `across` of the tube
/// that meets the chordal tolerance and the angle each way, with nothing
/// refined: what the angle cannot do without.
fn grid(major: f64, minor: f64, sweep: f64, across: f64, tolerance: f64, angle: f64) -> f64 {
    let arc = |radius: f64, span: f64| {
        let chordal = 2.0 * (1.0 - (tolerance / radius).min(1.0)).acos();
        let floor = ANGLE_FLOOR * tolerance / radius;
        (span / chordal.min(angle.max(floor))).ceil()
    };
    2.0 * arc(major + minor, sweep) * arc(minor, across)
}

fn tubes() -> Vec<(Reference, f64, f64, f64)> {
    let (major, minor) = (50.0, 0.6);
    vec![
        (
            solids::torus(&Frame::WORLD, major, minor).unwrap(),
            major,
            minor,
            TAU,
        ),
        (
            trimmed::pipe_elbow(&Frame::WORLD, major, minor, 1.9).unwrap(),
            major,
            minor,
            1.9,
        ),
    ]
}

#[test]
fn a_thin_tube_meets_the_angle_with_few_triangles() {
    let angle = 12.0 * DEGREES;
    for (solid, major, minor, sweep) in tubes() {
        for tolerance in [0.17, 0.05, 0.01] {
            let name = format!("{} at {tolerance}", solid.name);
            let p = TessParams {
                chordal_tolerance: tolerance,
                max_angle: Some(angle),
                ..TessParams::default()
            };
            let plain = tessellate_body(
                &solid.body,
                &TessParams {
                    max_angle: None,
                    ..p
                },
            )
            .unwrap();
            let mesh = tessellate_body(&solid.body, &p).unwrap_or_else(|e| panic!("{name}: {e}"));
            let welded = mesh.weld().unwrap();
            assert!(welded.is_closed(), "{name}: not closed");
            let off = (welded.volume() - solid.volume).abs();
            assert!(off <= tolerance * solid.area, "{name}: volume off by {off}");
            let (worst, sides) = worst_turn(&mesh, ANGLE_FLOOR * tolerance);
            let ideal = grid(major, minor, sweep, TAU, tolerance, angle);
            println!(
                "{name}: {} triangles ({} without the angle, a grid {ideal}), {sides} sides \
                 held, worst turn {:.3} degrees",
                mesh.triangles.len(),
                plain.triangles.len(),
                worst / DEGREES
            );
            assert!(
                worst <= angle * (1.0 + 1e-6),
                "{name}: a side turns {} degrees",
                worst / DEGREES
            );
            // Before the plane of the grid: 165,050 triangles for the
            // torus at 0.17 (92 grids), 49,364 for the elbow; 3,960 and
            // 1,264 after (2.2 and 2.3 grids).
            assert!(
                (mesh.triangles.len() as f64) <= 3.0 * ideal,
                "{name}: {} triangles, a grid has {ideal}",
                mesh.triangles.len()
            );
            for threads in [1, 8] {
                let again = tessellate_body_with_threads(&solid.body, &p, threads).unwrap();
                assert_eq!(again, mesh, "{name}: {threads} threads");
            }
        }
    }
}
