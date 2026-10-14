//! The boolean on a solid whose wall is one face on a NURBS surface that
//! closes on itself around the axis (as a STEP file gives a turned part:
//! the kernel itself splits a full turn into two faces). The solid is cut
//! in half by a block, which by symmetry leaves half its volume; the
//! oracles are that closed form, the exact measure and the mesh of the
//! result.
//!
//! Written on 2026-10-14 while fixing `craft3d_topo::mass_properties`,
//! which took a face's region to be the part of its surface's parameter
//! rectangle that its loops bound; on a surface that closes on itself that
//! is not so where a loop goes over or around the seam. The boolean charts
//! a freeform face by the same parameters (`boolean::chart`, `Chart::Patch`)
//! and does not know that the surface closes: it refuses every one of these
//! cuts ("a piece of a face is not closed off by its boundary"), which is
//! work owed, not a wrong result. This test holds it to that: the half, or
//! an error, never another number.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_kernel::boolean::BooleanOp::Intersect;
use craft3d_kernel::boolean::boolean;
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, check, mass_properties};
use std::f64::consts::{FRAC_PI_4, PI, TAU};

/// The vase's profile `(ρ, z)` as a cubic Bézier.
const VASE: [(f64, f64); 4] = [(1.0, 0.0), (1.9, 0.7), (0.3, 1.4), (1.2, 2.0)];

/// `π ∫ ρ² dz` along the profile (Pappus), by Gauss–Legendre: the
/// integrand is a polynomial of degree 8, which five nodes on each of a
/// hundred stretches sum exactly.
fn vase_volume() -> f64 {
    const GAUSS: [(f64, f64); 5] = [
        (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
        (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
        (0.0, 0.568_888_888_888_888_9),
        (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
        (0.906_179_845_938_664, 0.236_926_885_056_189_1),
    ];
    let mut sum = 0.0;
    for i in 0..100 {
        for &(x, w) in &GAUSS {
            let t = (f64::from(i) + (x + 1.0) / 2.0) / 100.0;
            let s = 1.0 - t;
            let b = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
            let d = [s * s, 2.0 * s * t, t * t];
            let rho: f64 = (0..4).map(|k| b[k] * VASE[k].0).sum();
            let dz: f64 = (0..3).map(|k| 3.0 * d[k] * (VASE[k + 1].1 - VASE[k].1)).sum();
            sum += rho * rho * dz * w / 200.0;
        }
    }
    PI * sum
}

/// The vase turned about the z axis, the wall one face on a NURBS surface
/// that starts at `from` radians; with `around_v` the turn is the
/// surface's second parameter. The wall is bounded by its two rims and,
/// with `seam`, by the seam edge walked up and down.
fn vase(from: f64, around_v: bool, seam: bool) -> Body {
    let weight = FRAC_PI_4.cos();
    let ring: Vec<(f64, f64, f64)> = (0..9)
        .map(|k| {
            let (sin, cos) = (from + FRAC_PI_4 * f64::from(k)).sin_cos();
            if k % 2 == 0 {
                (cos, sin, 1.0)
            } else {
                (cos / weight, sin / weight, weight)
            }
        })
        .collect();
    let ring_knots = vec![
        0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
    ];
    let bezier = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
    let mut put = |(x, y, w): (f64, f64, f64), (rho, z): (f64, f64)| {
        ctrl.push(Vec3::new(rho * x, rho * y, z));
        weights.push(w);
    };
    if around_v {
        for &p in &VASE {
            for &r in &ring {
                put(r, p);
            }
        }
    } else {
        for &r in &ring {
            for &p in &VASE {
                put(r, p);
            }
        }
    }
    let surface = if around_v {
        NurbsSurface::new(3, 2, bezier.clone(), ring_knots, 4, 9, ctrl, weights)
    } else {
        NurbsSurface::new(2, 3, ring_knots, bezier.clone(), 9, 4, ctrl, weights)
    }
    .unwrap();

    let mut body = Body::new(BodyKind::Solid);
    let (sin, cos) = from.sin_cos();
    let x_axis = Vec3::new(cos, sin, 0.0);
    let rim = |body: &mut Body, (rho, z): (f64, f64)| -> EdgeId {
        let vertex = body.add_vertex(Vec3::new(rho * cos, rho * sin, z));
        let frame = Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, x_axis).unwrap();
        body.add_edge(
            Curve3::Circle(Circle::new(frame, rho).unwrap()),
            (0.0, TAU),
            vertex,
            vertex,
        )
    };
    let lower = rim(&mut body, VASE[0]);
    let upper = rim(&mut body, VASE[3]);
    let use_of = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
    let loops = if seam {
        let (start, end) = (body.edges[0].start, body.edges[1].start);
        let profile = VASE
            .iter()
            .map(|&(rho, z)| Vec3::new(rho * cos, rho * sin, z))
            .collect();
        let curve = Curve3::Nurbs(NurbsCurve::polynomial(3, bezier, profile).unwrap());
        let seam = body.add_edge(curve, (0.0, 1.0), start, end);
        vec![body.add_loop(vec![
            use_of(lower, false),
            use_of(seam, false),
            use_of(upper, true),
            use_of(seam, true),
        ])]
    } else {
        vec![
            body.add_loop(vec![use_of(lower, false)]),
            body.add_loop(vec![use_of(upper, true)]),
        ]
    };
    let side = body.add_face(Surface::Nurbs(surface), around_v, loops);
    let floor_loop = body.add_loop(vec![use_of(lower, true)]);
    let floor = Frame::new(Vec3::ZERO, -Vec3::Z, x_axis).unwrap();
    let floor = body.add_face(Surface::Plane(Plane::new(floor)), false, vec![floor_loop]);
    let lid_loop = body.add_loop(vec![use_of(upper, false)]);
    let lid = Frame::new(Vec3::new(0.0, 0.0, VASE[3].1), Vec3::Z, x_axis).unwrap();
    let lid = body.add_face(Surface::Plane(Plane::new(lid)), false, vec![lid_loop]);
    body.add_shell(vec![side, floor, lid]);
    body
}

/// The half of the vase with `x > 0`, where the seam of its surface lies
/// inside the half (from 0), on the cut (from π/2) or outside it (from
/// π): half the volume, by the exact measure and by the mesh, or an error.
/// What comes back as a number that is not half is a defect.
#[test]
fn half_of_a_vase_whose_wall_closes_on_itself() {
    let tolerance = Tolerance::new(1e-6, 1e-9).unwrap();
    let half = 0.5 * vase_volume();
    let corner = Frame::WORLD.at(Vec3::new(0.0, -5.0, -1.0)).unwrap();
    let block = primitives::block(&corner, 5.0, 10.0, 4.0).unwrap();
    let mut wrong = Vec::new();
    for from in [0.0, 0.5 * PI, PI, 0.4] {
        for around_v in [false, true] {
            for seam in [true, false] {
                let what = format!("from {from}, around v: {around_v}, seam edge: {seam}");
                let solid = vase(from, around_v, seam);
                assert_eq!(check(&solid, &tolerance), vec![], "{what}");
                let whole = mass_properties(&solid).unwrap().volume;
                assert!((whole - 2.0 * half).abs() < 1e-9 * whole, "{what}: {whole}");
                let result = match boolean(&solid, &block, Intersect, &tolerance) {
                    Ok(result) => result.body,
                    Err(error) => {
                        println!("{what}: refused: {error}");
                        continue;
                    }
                };
                let problems = check(&result, &tolerance);
                if !problems.is_empty() {
                    wrong.push(format!("{what}: an invalid body: {problems:?}"));
                    continue;
                }
                let params = TessParams {
                    chordal_tolerance: 1e-3,
                    ..TessParams::default()
                };
                let mesh = tessellate_body(&result, &params).map(|m| m.weld());
                let exact = mass_properties(&result);
                match (exact, mesh) {
                    (Ok(m), _) if (m.volume - half).abs() > 1e-9 * half => {
                        wrong.push(format!("{what}: the exact measure says {}", m.volume));
                    }
                    (_, Ok(Ok(mesh))) if (mesh.volume() - half).abs() > 2e-2 * half => {
                        wrong.push(format!("{what}: the mesh encloses {}", mesh.volume()));
                    }
                    (exact, mesh) => println!(
                        "{what}: {:?} / mesh {:?}",
                        exact.map(|m| m.volume),
                        mesh.map(|m| m.map(|m| m.volume()))
                    ),
                }
            }
        }
    }
    assert!(wrong.is_empty(), "half is {half}:\n{}", wrong.join("\n"));
}
