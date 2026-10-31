//! A rolling-ball fillet along an edge of a face that an offset made,
//! against closed forms.
//!
//! The face is a quarter of a cylinder given as a rational *cubic* NURBS
//! surface that is that cylinder exactly (the rational quadratic quarter
//! of `rational_common`, raised to degree three and with two simple knots
//! inserted): the kernel sees a spline with knots at which a cubic's
//! offset is only once continuously differentiable, offsets it to a
//! fitted spline (which is the cylinder with its radius moved, to a share
//! of the tolerance), and fillets the edge between it and a flat cap by
//! rolling a ball along the fitted face. The fillet of a circular edge
//! between a plane and a cylinder about the circle's axis is a piece of a
//! torus, so the result's volume has a closed form: Pappus's rule over the
//! corner the ball cuts away (a convex rim) or fills (a concave foot).
//! Three scales; the face moved out and in.
//!
//! The case this pins (`docs/design/face-operations.md`, open item 4 of
//! 2026-10-16): the bike's saddle shelled first and filleted after, whose
//! inner rims were refused, the ball not settling on, or the fillet not
//! approximable along, the found-again edges of fitted offset surfaces.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_math::Portable;
mod boolean_common;
mod rational_common;

use boolean_common::{SCALES, boxed, mass, rod, solve, tolerance};
use craft3d_geom::refine::{elevated, with_knots};
use craft3d_geom::{NurbsCurve, NurbsSurface, Surface};
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract};
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_math::Vec3;
use craft3d_topo::{Body, EdgeId, FaceId, check};
use rational_common::rationalized;
use std::f64::consts::PI;

/// The quarter of a rod of radius 2 and height 1 standing on the origin
/// (x and y at least 0): its rim at the top is convex.
fn wedge(k: f64) -> Body {
    let rod = rod(k, [0.0, 0.0, 0.0], 2.0, 1.0);
    let quadrant = boxed(k, [0.0, 0.0, -1.0], [3.0, 3.0, 2.0]);
    solve(&rod, &quadrant, Intersect, k).body
}

/// A block 3 × 3 × 1.5 with the quarter of a pocket of radius 2 and depth
/// 1 at its corner on the z axis: the foot of the pocket's wall on its
/// floor (z = 0.5) is concave.
fn pocket(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [3.0, 3.0, 1.5]);
    let rod = rod(k, [0.0, 0.0, 0.5], 2.0, 2.0);
    solve(&block, &rod, Subtract, k).body
}

/// The same rational surface with its first parameter raised to degree
/// three and the knots 1 and 2 inserted (its domain in u is [0, 3]): the
/// same points at the same parameters.
fn cubic(surface: &NurbsSurface) -> NurbsSurface {
    let (count_u, count_v) = surface.counts();
    assert_eq!(surface.degrees().0, 2);
    let lines: Vec<NurbsCurve> = (0..count_v)
        .map(|j| {
            let ctrl = (0..count_u)
                .map(|i| surface.control_points()[i * count_v + j])
                .collect();
            let weights = (0..count_u)
                .map(|i| surface.weights()[i * count_v + j])
                .collect();
            let line = NurbsCurve::new(2, surface.knots_u().to_vec(), ctrl, weights).unwrap();
            with_knots(&elevated(&line, 3).unwrap(), &[1.0, 2.0]).unwrap()
        })
        .collect();
    let new_u = lines[0].control_points().len();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..new_u {
        for line in &lines {
            ctrl.push(line.control_points()[i]);
            weights.push(line.weights()[i]);
        }
    }
    NurbsSurface::new(
        3,
        surface.degrees().1,
        lines[0].knots().to_vec(),
        surface.knots_v().to_vec(),
        new_u,
        count_v,
        ctrl,
        weights,
    )
    .unwrap()
}

/// `body` with its cylinder face on the rational cubic that is that piece
/// of the cylinder, and the face.
fn twin(body: &Body) -> (Body, FaceId) {
    let (mut twin, replaced) = rationalized(body, 0.0, false);
    assert_eq!(replaced, 1);
    let index = twin
        .faces
        .iter()
        .position(|f| matches!(f.surface, Surface::Nurbs(_)))
        .unwrap();
    let Surface::Nurbs(quadratic) = &twin.faces[index].surface else {
        panic!("not the rational patch")
    };
    let raised = cubic(quadratic);
    // The same surface, point for point.
    let ((u0, u1), (v0, v1)) = raised.domain();
    for i in 0..=12 {
        for j in 0..=4 {
            let (u, v) = (
                u0 + (u1 - u0) * f64::from(i) / 12.0,
                v0 + (v1 - v0) * f64::from(j) / 4.0,
            );
            let (a, b) = (raised.eval(u, v), quadratic.eval(u, v));
            assert!(a.distance(b) <= 1e-12 * (1.0 + a.length()), "{a:?} {b:?}");
        }
    }
    twin.faces[index].surface = Surface::Nurbs(raised);
    assert_eq!(check(&twin, &tolerance(1.0)), []);
    (twin, FaceId(index as u32))
}

/// The edge between `face` and the flat face at the height `z`.
fn rim(body: &Body, face: FaceId, z: f64) -> EdgeId {
    let users = |e: usize| -> Vec<FaceId> {
        (0..body.faces.len())
            .filter(|f| {
                body.faces[*f]
                    .loops
                    .iter()
                    .flat_map(|l| &body.loop_(*l).unwrap().coedges)
                    .any(|c| c.edge == EdgeId(e as u32))
            })
            .map(|f| FaceId(f as u32))
            .collect()
    };
    let found: Vec<usize> = (0..body.edges.len())
        .filter(|e| {
            let faces = users(*e);
            faces.contains(&face)
                && faces.iter().any(|f| {
                    *f != face && matches!(body.face(*f).unwrap().surface, Surface::Plane(_)) && {
                        let edge = &body.edges[*e];
                        let p = edge.curve.eval(0.5 * (edge.range.0 + edge.range.1));
                        (p.z - z).abs() <= 1e-6 * (1.0 + z.abs())
                    }
                })
        })
        .collect();
    assert_eq!(found.len(), 1, "one edge at the height {z}: {found:?}");
    EdgeId(found[0] as u32)
}

/// Where the centroid of the corner a ball of radius ρ rounds off lies,
/// from the corner, along either face: ρ (10 − 3π) / (12 − 3π).
fn centroid_in(rho: f64) -> f64 {
    rho * (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI)
}

/// The corner a ball of radius ρ rounds off: ρ² (1 − π / 4).
fn corner(rho: f64) -> f64 {
    rho * rho * (1.0 - PI / 4.0)
}

/// The distances tried at scale `k`: out at the smallest scale, in at the
/// largest, both at 1 (each a whole offset and fillet).
fn moves(k: f64, [out, inward]: [f64; 2]) -> Vec<f64> {
    if k < 1.0 {
        vec![out]
    } else if k > 1.0 {
        vec![inward]
    } else {
        vec![out, inward]
    }
}

/// The face of `body` moved by `d` and the edge between it and the flat
/// face at the height `z` filleted with radius `rho` (all at scale 1, the
/// body at `k`): the volume and the area.
fn offset_then_fillet(body: &Body, face: FaceId, d: f64, z: f64, rho: f64, k: f64) -> (f64, f64) {
    let tol = tolerance(k);
    let moved = offset_faces(
        body,
        &[FaceOffset {
            face,
            distance: d * k,
        }],
        &tol,
    )
    .unwrap_or_else(|e| panic!("offset {d} at {k}: {e}"));
    assert_eq!(check(&moved.body, &tol), [], "offset {d} at {k}");
    let edge = rim(&moved.body, face, z * k);
    let made = fillet(
        &moved.body,
        &[FilletEdge {
            edge,
            radius: rho * k,
        }],
        &tol,
    )
    .unwrap_or_else(|e| panic!("fillet after the offset {d} at {k}: {e}"));
    assert_eq!(check(&made.body, &tol), [], "fillet after {d} at {k}");
    let m = mass(&made.body);
    (m.volume, m.area)
}

#[test]
fn the_convex_rim_of_an_offset_freeform_face_is_filleted_as_the_torus_it_is() {
    let rho = 0.25;
    for k in SCALES {
        let tol = tolerance(k);
        let (body, face) = twin(&wedge(k));
        for d in moves(k, [0.3, -0.4]) {
            let (volume, area) = offset_then_fillet(&body, face, d, 1.0, rho, k);
            // The rod's radius moved by d; the ring the ball cuts off the
            // rim, a quarter of it.
            let r = 2.0 + d;
            let expected = PI / 4.0 * r * r - PI / 2.0 * (r - centroid_in(rho)) * corner(rho);
            // Every face within a quarter of the tolerance of the exact one.
            let slack = 0.5 * tol.linear * area;
            let off = volume - expected * k.ppowi(3);
            println!(
                "rim, k {k}, d {d}: volume {volume}, closed form {}, off {off:e}, slack {slack:e}",
                expected * k.ppowi(3)
            );
            assert!(
                off.abs() <= slack,
                "k {k}, d {d}: off by {off:e}, slack {slack:e}"
            );
        }
    }
}

#[test]
fn the_concave_foot_of_an_offset_freeform_face_is_filleted_as_the_torus_it_is() {
    let rho = 0.25;
    for k in SCALES {
        let tol = tolerance(k);
        let (body, face) = twin(&pocket(k));
        for d in moves(k, [0.3, -0.4]) {
            let (volume, area) = offset_then_fillet(&body, face, d, 0.5, rho, k);
            // The wall moves away from the material: the pocket's radius
            // shrinks by d. The ring the ball fills at its foot, a quarter.
            let r = 2.0 - d;
            let expected =
                13.5 - PI / 4.0 * r * r + PI / 2.0 * (r - centroid_in(rho)) * corner(rho);
            let slack = 0.5 * tol.linear * area;
            let off = volume - expected * k.ppowi(3);
            println!(
                "foot, k {k}, d {d}: volume {volume}, closed form {}, off {off:e}, slack {slack:e}",
                expected * k.ppowi(3)
            );
            assert!(
                off.abs() <= slack,
                "k {k}, d {d}: off by {off:e}, slack {slack:e}"
            );
        }
    }
}

/// The offset of the cubic twin keeps its normal across the knots at
/// which a cubic's offset is only once continuously differentiable, and
/// at its borders: within what the fit of positions allows, so that a
/// ball rolling along it sees no crease. Held against the cylinder's own
/// normal, which the offset of a cylinder shares.
#[test]
fn the_offset_of_a_cubic_twin_has_the_normal_of_the_cylinder_across_its_knots_and_borders() {
    let k = 1.0;
    let tol = tolerance(k);
    let (body, face) = twin(&wedge(k));
    let moved = offset_faces(
        &body,
        &[FaceOffset {
            face,
            distance: 0.3,
        }],
        &tol,
    )
    .unwrap();
    let surface = &moved.body.face(face).unwrap().surface;
    let ((u0, u1), (v0, v1)) = surface.domain();
    // The largest sine between the fitted normal and the cylinder's, at
    // the borders and at the knots.
    let (mut border, mut knot, mut jump) = (0.0_f64, 0.0_f64, 0.0_f64);
    for u in [u0, 1.0, 2.0, u1] {
        for j in 0..=8 {
            let v = v0 + (v1 - v0) * f64::from(j) / 8.0;
            let p = surface.eval(u, v);
            let n = surface.normal(u, v).unwrap();
            let radial = Vec3::new(p.x, p.y, 0.0).normalized().unwrap();
            let sine = n.cross(radial).length();
            if u == u0 || u == u1 {
                border = border.max(sine);
            } else {
                knot = knot.max(sine);
            }
            let r = p.x.phypot(p.y);
            assert!(
                (r - 2.3).abs() <= 0.25 * tol.linear,
                "radius {r} at ({u}, {v})"
            );
            // Either side of the knot (or just inside a border): the
            // normal there, to rounding and what the curvature turns it by.
            for side in [u - 1e-12, u + 1e-12] {
                if side > u0 && side < u1 {
                    let m = surface.normal(side, v).unwrap();
                    jump = jump.max(m.cross(n).length());
                }
            }
        }
    }
    println!(
        "sine between the fitted normal and the cylinder's: at the borders {border:e}, at the knots {knot:e}; across a knot {jump:e}"
    );
    // Across a knot the normal goes on (two fits joined at a point only
    // left it turning by what each fit's slope is off, about 1e-6 on the
    // saddle).
    assert!(jump <= 1e-12, "{jump:e}");
    // Within what holding the points to a quarter of the tolerance on
    // steps of about a fifth of a knot span (0.24 long here) leaves of the
    // slope of a quintic: 5 × 2.5e-7 / 0.24, about 5e-6.
    assert!(knot <= 5e-6, "{knot:e}");
    // At the borders, which lie inside the fit, better than at a joint
    // (at the end of an interpolation they were as bad as 9e-7 on the
    // saddle).
    assert!(border <= 1e-7, "{border:e}");
}
