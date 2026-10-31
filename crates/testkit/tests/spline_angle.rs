//! The angle criterion on spline faces: a twisted blade lofted through
//! three cambered aerofoils (the developer's rotor, `crates/engine/tests/loft.rs`),
//! whose sides turn half a turn around a nose a few millimetres round
//! and hardly at all along the span.
//!
//! With the display's angle the turbofan's spline faces had 1.8 times
//! the triangles of the chordal tolerance alone (19,760 and 35,302 on its
//! fan blades, 2026-10-15). Measured against a grid that meets both
//! criteria with nothing refined (below), part of it was the curvature's
//! and part refinement in a plane that fitted the face nowhere near the
//! nose, where its parameter runs forty times slower than at the root of
//! the side: this blade had 1.9 times the triangles of that grid, now
//! within 1.5 (the plane and the grid of `angle_warp` in
//! `crates/tess/src/face.rs`); the turbofan's blades 30,720.
//!
//! Oracles: every side not below the floor turns the normal by no more
//! than the angle (measured on the mesh's own vertex normals, which are
//! the surface's); the mesh is closed; the same mesh on one and eight
//! threads; the triangles of the spline faces stay within a small factor
//! of the unrefined grid; and the body at a thousand times the size,
//! with its tolerance, has the same number of triangles.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Surface;
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{ANGLE_FLOOR, Mesh, TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_topo::Body;
use std::f64::consts::PI;

type P = [f64; 2];

const DEGREES: f64 = PI / 180.0;

fn turn(a: Vec3, b: Vec3) -> f64 {
    a.cross(b).length().patan2(a.dot(b))
}

fn spline(id: u64, control: &[P], k: f64) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control: control.iter().map(|p| [p[0] * k, p[1] * k]).collect(),
        }),
    }
}

/// A cambered aerofoil of two cubic splines at height `z`.
fn aerofoil(z: f64, upper: [P; 4], lower: [P; 2], k: f64) -> Section {
    let curves = [
        spline(1, &upper, k),
        spline(2, &[upper[3], lower[0], lower[1], upper[0]], k),
    ];
    let profiles = detect_profiles(&curves, &Tolerance::default()).unwrap();
    Section {
        plane: Frame::WORLD.at(Vec3::new(0.0, 0.0, z * k)).unwrap(),
        outline: profiles[0].outer.clone(),
        start: None,
    }
}

/// The rotor blade from 8 m to its tip at 21.5 m, `k` times as large.
fn blade(k: f64) -> Body {
    let sections = [
        aerofoil(
            8.0,
            [
                [-0.765669, -0.148831],
                [-0.825698, 0.159989],
                [-0.109143, 0.561491],
                [1.786561, 0.347272],
            ],
            [[0.442863, -0.234404], [-0.716555, -0.401502]],
            k,
        ),
        aerofoil(
            18.0,
            [
                [-0.329887, -0.008638],
                [-0.332104, 0.076033],
                [-0.004031, 0.153947],
                [0.769736, 0.020156],
            ],
            [[0.167161, -0.080352], [-0.328073, -0.077915]],
            k,
        ),
        aerofoil(
            21.5,
            [[-0.165, 0.0], [-0.165, 0.0363], [0.0, 0.066], [0.385, 0.0]],
            [[0.0825, -0.0363], [-0.165, -0.0297]],
            k,
        ),
    ];
    let tolerance = Tolerance {
        linear: 1e-6 * k,
        ..Tolerance::default()
    };
    loft(&sections, &Options::default(), &tolerance)
        .unwrap()
        .body
}

/// The largest turn of the normal over a side not below `floor`.
fn worst_turn(mesh: &Mesh, floor: f64) -> f64 {
    let mut worst = 0.0_f64;
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
            worst = worst.max(turn(n, m));
        }
    }
    worst
}

/// Triangles of a grid over the whole surface whose cells, each cut in
/// two along a diagonal, meet the chordal tolerance and the angle, with
/// nothing refined: whole columns and rows are cut in two where a cell
/// of theirs fails, until none fails. Measured as the tessellator
/// measures (the surface at the middle of each side and at the centroid
/// of each triangle against the flat triangles; the normals at the ends
/// of each side not below the floor), but by this test alone.
fn grid(surface: &Surface, tolerance: f64, angle: f64) -> usize {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let (mut us, mut vs) = (vec![u0, u1], vec![v0, v1]);
    let floor = ANGLE_FLOOR * tolerance;
    for _ in 0..40 {
        let (mut columns, mut rows) = (vec![false; us.len() - 1], vec![false; vs.len() - 1]);
        for i in 0..us.len() - 1 {
            for j in 0..vs.len() - 1 {
                let corner = |a: usize, b: usize| {
                    let (u, v) = (us[i + a], vs[j + b]);
                    (
                        [u, v],
                        surface.eval(u, v),
                        surface.normal(u, v).unwrap_or(Vec3::new(0.0, 0.0, 0.0)),
                    )
                };
                let [c00, c10, c01, c11] = [corner(0, 0), corner(1, 0), corner(0, 1), corner(1, 1)];
                let off = |points: &[&([f64; 2], Vec3, Vec3)]| {
                    let n = points.len() as f64;
                    let u = points.iter().map(|p| p.0[0]).sum::<f64>() / n;
                    let v = points.iter().map(|p| p.0[1]).sum::<f64>() / n;
                    let flat =
                        points.iter().fold(Vec3::new(0.0, 0.0, 0.0), |s, p| s + p.1) * (1.0 / n);
                    surface.eval(u, v).distance(flat) > tolerance
                };
                let turns = |a: &([f64; 2], Vec3, Vec3), b: &([f64; 2], Vec3, Vec3)| {
                    a.1.distance(b.1) >= floor && turn(a.2, b.2) > angle * (1.0 + 1e-6)
                };
                let along_u = off(&[&c00, &c10])
                    || off(&[&c01, &c11])
                    || turns(&c00, &c10)
                    || turns(&c01, &c11);
                let along_v = off(&[&c00, &c01])
                    || off(&[&c10, &c11])
                    || turns(&c00, &c01)
                    || turns(&c10, &c11);
                let inside = off(&[&c00, &c11])
                    || off(&[&c00, &c10, &c11])
                    || off(&[&c00, &c11, &c01])
                    || turns(&c00, &c11);
                columns[i] |= along_u;
                rows[j] |= along_v;
                if inside && !along_u && !along_v {
                    // The diagonal: cut the way that turns more.
                    if turn(c00.2, c10.2) >= turn(c00.2, c01.2) {
                        columns[i] = true;
                    } else {
                        rows[j] = true;
                    }
                }
            }
        }
        if !columns.contains(&true) && !rows.contains(&true) {
            return 2 * (us.len() - 1) * (vs.len() - 1);
        }
        let cut = |lines: &[f64], marked: &[bool]| {
            let mut out = vec![lines[0]];
            for (n, &m) in marked.iter().enumerate() {
                if m {
                    out.push(0.5 * (lines[n] + lines[n + 1]));
                }
                out.push(lines[n + 1]);
            }
            out
        };
        us = cut(&us, &columns);
        vs = cut(&vs, &rows);
    }
    panic!("the grid did not settle");
}

#[test]
fn a_twisted_blade_meets_the_angle_with_the_triangles_its_curvature_asks_for() {
    let angle = 12.0 * DEGREES;
    let body = blade(1.0);
    for tolerance in [0.003, 0.001] {
        let p = TessParams {
            chordal_tolerance: tolerance,
            max_angle: Some(angle),
            ..TessParams::default()
        };
        let plain = tessellate_body(
            &body,
            &TessParams {
                max_angle: None,
                ..p
            },
        )
        .unwrap();
        let mesh = tessellate_body(&body, &p).unwrap();
        let welded = mesh.weld().unwrap();
        assert!(welded.is_closed(), "at {tolerance}: not closed");
        let worst = worst_turn(&mesh, ANGLE_FLOOR * tolerance);
        assert!(
            worst <= angle * (1.0 + 1e-6),
            "at {tolerance}: a side turns {} degrees",
            worst / DEGREES
        );
        let (mut triangles, mut without, mut ideal) = (0, 0, 0);
        for (index, face) in body.faces.iter().enumerate() {
            if !matches!(face.surface, Surface::Nurbs(_)) {
                continue;
            }
            let count = |m: &Mesh| m.face_ids.iter().filter(|&&f| f as usize == index).count();
            triangles += count(&mesh);
            without += count(&plain);
            ideal += grid(&face.surface, tolerance, angle);
        }
        println!(
            "at {tolerance}: spline faces {triangles} triangles ({without} without the angle, an \
             unrefined grid {ideal}), worst turn {:.3} degrees",
            worst / DEGREES
        );
        assert!(ideal > 0 && without > 0);
        assert!(
            (triangles as f64) <= 1.5 * ideal as f64,
            "at {tolerance}: {triangles} triangles, a grid has {ideal}"
        );
        for threads in [1, 8] {
            let again = tessellate_body_with_threads(&body, &p, threads).unwrap();
            assert_eq!(again, mesh, "at {tolerance}: {threads} threads");
        }
        // No length of its own: a thousand times as large, with its
        // tolerance, the same triangles.
        let large = tessellate_body(
            &blade(1024.0),
            &TessParams {
                chordal_tolerance: 1024.0 * tolerance,
                ..p
            },
        )
        .unwrap();
        assert_eq!(
            large.triangles.len(),
            mesh.triangles.len(),
            "at {tolerance}"
        );
    }
}
