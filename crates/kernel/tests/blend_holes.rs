//! Holes and other loops of a face that lie wholly in the material a
//! blend takes away. A blend built in place cuts its faces back and knows
//! only the loops it changes: a shallow hole inside what it removes was
//! left outside its face, a body every other check passed and whose
//! volume was short by the hole's (merge review of 2026-10-16, cases A to
//! D). Such a blend is either built right (the hole taken whole, as the
//! boolean of a tool does) or refused; never a wrong solid. And
//! `topo::check` finds a loop left outside its face on a plane, whatever
//! made it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::{SCALES, boxed, mass, tolerance};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

/// The edge of `body` that passes nearest `p`.
fn edge_near(body: &Body, p: Vec3) -> EdgeId {
    let (i, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let d = (0..=32)
                .map(|j| {
                    let t = e.range.0 + (e.range.1 - e.range.0) * f64::from(j) / 32.0;
                    e.curve.eval(t).distance(p)
                })
                .fold(f64::INFINITY, f64::min);
            (i, d)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(i as u32)
}

fn faces_of(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    let mut found = Vec::new();
    for (index, face) in body.faces.iter().enumerate() {
        let uses = face
            .loops
            .iter()
            .flat_map(|l| &body.loops[l.0 as usize].coedges)
            .any(|c| c.edge == edge);
        if uses {
            found.push(FaceId(index as u32));
        }
    }
    found
}

/// A rod of radius `r` and length `h` along `axis` from `origin`, taken
/// out of `body` (all times `k`).
fn drill(body: &Body, k: f64, origin: Vec3, axis: Vec3, r: f64, h: f64) -> Body {
    let x = if axis.cross(Vec3::X).length() > 0.5 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let rod =
        primitives::cylinder(&Frame::new(origin * k, axis, x).unwrap(), r * k, h * k).unwrap();
    boolean(body, &rod, BooleanOp::Subtract, &tolerance(k))
        .unwrap()
        .body
}

/// A half of a cone frustum (radius 1, apex 3 up, cut at z = 1), the half
/// y >= 0: its cut edges are straight edges between a plane and a cone,
/// blended in place.
fn half_frustum(k: f64) -> Body {
    let cone = primitives::cone(&Frame::WORLD, k, 3.0 * k).unwrap();
    let half = boxed(k, [-2.0, 0.0, -1.0], [2.0, 2.0, 1.0]);
    boolean(&cone, &half, BooleanOp::Intersect, &tolerance(k))
        .unwrap()
        .body
}

/// The rod of radius 1 cut by a plane at a slant: its rim is a closed
/// elliptic edge, blended in place as a ring. With the cut's normal.
fn slanted_rod(k: f64) -> (Body, Vec3) {
    let rod = primitives::cylinder(&Frame::WORLD, k, 3.0 * k).unwrap();
    let n = Vec3::new(0.3, 0.0, 1.0).normalized().unwrap();
    let o = Vec3::new(-3.0, -3.0, 1.5) * k;
    let cutter = primitives::block(
        &Frame::new(o, n, Vec3::X).unwrap(),
        6.0 * k,
        6.0 * k,
        6.0 * k,
    )
    .unwrap();
    let body = boolean(&rod, &cutter, BooleanOp::Subtract, &tolerance(k))
        .unwrap()
        .body;
    (body, n)
}

fn rim(body: &Body) -> EdgeId {
    let i = (0..body.edges.len())
        .find(|i| body.edges[*i].curve.kind() == "ellipse")
        .unwrap();
    EdgeId(i as u32)
}

#[derive(Clone, Copy)]
enum Blend {
    Fillet(f64),
    Chamfer(f64),
}

fn blend(body: &Body, edge: EdgeId, how: Blend, k: f64) -> Result<Body, String> {
    let tol = tolerance(k);
    match how {
        Blend::Fillet(r) => fillet(
            body,
            &[FilletEdge {
                edge,
                radius: r * k,
            }],
            &tol,
        )
        .map(|m| m.body)
        .map_err(|e| e.to_string()),
        Blend::Chamfer(d) => chamfer(
            body,
            &[ChamferEdge {
                edge,
                first: faces_of(body, edge)[0],
                size: ChamferSize::Distances {
                    first: d * k,
                    second: d * k,
                },
            }],
            &tol,
        )
        .map(|m| m.body)
        .map_err(|e| e.to_string()),
    }
}

/// The blend of `holed` is the blend of the body without the hole (which
/// lies wholly in what the blend removes: `whole` is that body's volume
/// after the blend), or refused; never a body short by the hole. Built
/// when `must_build`.
#[track_caller]
fn right_or_refused(
    name: &str,
    holed: &Body,
    edge: EdgeId,
    how: Blend,
    whole: f64,
    k: f64,
    must_build: bool,
) {
    match blend(holed, edge, how, k) {
        Ok(made) => {
            assert_eq!(check(&made, &tolerance(k)), [], "{name} at {k}");
            let v = mass(&made).volume;
            assert!(
                (v - whole).abs() <= 1e-6 * whole,
                "{name} at {k}: volume {v}, the blend without the hole {whole}"
            );
        }
        Err(e) => {
            assert!(!must_build, "{name} at {k}: refused: {e}");
        }
    }
}

#[test]
fn a_hole_in_a_face_beside_a_freeform_edge_inside_the_strip() {
    // Case A: a shallow hole in the cut plane y = 0, 0.12 in from the
    // edge, 0.02 deep.
    for k in SCALES {
        let body = half_frustum(k);
        let on_edge = Vec3::new(5.0 / 6.0, 0.0, 0.5);
        let inward = Vec3::new(-3.0, 0.0, -1.0).normalized().unwrap();
        let c = on_edge + inward * 0.12;
        let holed = drill(&body, k, c - Vec3::Y * 0.5, Vec3::Y, 0.04, 0.52);
        for how in [Blend::Fillet(0.3), Blend::Chamfer(0.3)] {
            let whole = mass(&blend(&body, edge_near(&body, on_edge * k), how, k).unwrap()).volume;
            right_or_refused(
                "A",
                &holed,
                edge_near(&holed, on_edge * k),
                how,
                whole,
                k,
                false,
            );
        }
    }
}

#[test]
fn a_hole_in_the_end_face_of_a_freeform_edge_at_its_corner() {
    // Case B: a shallow hole in the end face z = 1 next to the corner.
    for k in SCALES {
        let body = half_frustum(k);
        let on_edge = Vec3::new(5.0 / 6.0, 0.0, 0.5);
        let rho: f64 = 2.0 / 3.0 - 0.05;
        let p = Vec3::new((rho * rho - 0.0025).sqrt(), 0.05, 1.0);
        let holed = drill(&body, k, p - Vec3::Z * 0.1, Vec3::Z, 0.015, 0.5);
        for how in [Blend::Fillet(0.3), Blend::Chamfer(0.3)] {
            let whole = mass(&blend(&body, edge_near(&body, on_edge * k), how, k).unwrap()).volume;
            right_or_refused(
                "B",
                &holed,
                edge_near(&holed, on_edge * k),
                how,
                whole,
                k,
                false,
            );
        }
    }
}

#[test]
fn a_hole_in_a_face_of_a_block_inside_what_its_edge_blend_takes() {
    // Case C: a hole in the end face x = 0 at the corner of the top front
    // edge; case C2: a shallow hole in the top face inside the strip.
    // Both lie wholly in what a blend of 1 removes, and the tools take
    // them whole: built, the block's blend exactly.
    let fillet_whole = 24.0 - (1.0 - PI / 4.0) * 4.0;
    let chamfer_whole = 24.0 - 0.5 * 4.0;
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let on_edge = Vec3::new(2.0, 0.0, 2.0) * k;
        let c = drill(&block, k, Vec3::new(-1.0, 0.12, 1.88), Vec3::X, 0.05, 1.3);
        let c2 = drill(&block, k, Vec3::new(2.0, 0.12, 1.98), Vec3::Z, 0.05, 1.0);
        for (how, whole) in [
            (Blend::Fillet(1.0), fillet_whole),
            (Blend::Chamfer(1.0), chamfer_whole),
        ] {
            let whole = whole * k * k * k;
            right_or_refused("C", &c, edge_near(&c, on_edge), how, whole, k, true);
            right_or_refused("C2", &c2, edge_near(&c2, on_edge), how, whole, k, true);
        }
    }
}

#[test]
fn a_hole_in_the_cap_inside_a_ring_blend() {
    // Case D: the rim of a slanted cut of a rod, a shallow hole in the cap
    // inside the strip.
    for k in SCALES {
        let (body, n) = slanted_rod(k);
        let q = Vec3::new(0.0, 0.94, 0.6);
        let holed = drill(&body, k, q - n * 0.02, n, 0.015, 0.5);
        for how in [Blend::Fillet(0.2), Blend::Chamfer(0.2)] {
            let whole = mass(&blend(&body, rim(&body), how, k).unwrap()).volume;
            right_or_refused("D", &holed, rim(&holed), how, whole, k, false);
        }
    }
}
