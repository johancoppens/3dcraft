//! A face on a sphere whose loop passes twice through one vertex: the
//! zone below a circle of latitude with a pocket cut out of it that hangs
//! from the circle at one point (the fuzzer's seed 21635, 2026-10-18: a
//! mirrored body whose sphere face had such a loop could not be meshed).
//!
//! The face winds around the sphere's axis, so the tessellator cuts it
//! open with a seam from its loop to the pole. Where the seam starts at
//! the vertex the loop passes twice, only one of the two passes opens
//! towards the pole: the other lies between the pocket and the rest of
//! the loop. Starting from the wrong one tore the mesh along the pocket.
//!
//! The body is a sheet of two faces built by hand, the zone with the
//! pocket and the pocket itself, both on the sphere, so that their areas
//! add up to the zone's in closed form, `2π R (R + c)` below the height
//! `c` (oracle 4 of docs/testing.md), and each face's mesh closes along
//! every edge it has (which the tessellator checks).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Sphere, Surface};
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, BodyKind, Coedge};
use std::f64::consts::{PI, TAU};

/// The short arc from `p` to `q` of the circle in which the plane through
/// both with normal `n` meets the sphere of radius `r` about the origin
/// of `frame` (`n` in world terms): the circle's frame starts at `p`.
fn arc(frame: &Frame, r: f64, p: Vec3, q: Vec3, n: Vec3) -> (Curve3, f64) {
    let centre_of_sphere = frame.origin();
    let n = n.normalized().unwrap();
    let mut n = n;
    for _ in 0..2 {
        let offset = n.dot(p - centre_of_sphere);
        let centre = centre_of_sphere + n * offset;
        let radius = (r * r - offset * offset).sqrt();
        let circle_frame = Frame::new(centre, n, p - centre).unwrap();
        let circle = Circle::new(circle_frame, radius).unwrap();
        let local = q - centre;
        let angle = local
            .dot(circle_frame.y())
            .atan2(local.dot(circle_frame.x()));
        let angle = if angle < 0.0 { angle + TAU } else { angle };
        if angle < PI {
            return (Curve3::Circle(circle), angle);
        }
        n = -n;
    }
    panic!("no short arc");
}

/// The sheet: face 0 the zone below the rim at height `c` (in the
/// construction frame `at`, whose z is up) without the pocket, face 1 the
/// pocket. The pocket hangs from the rim at its point `p` (longitude 0 of
/// `at`) down to the point `q` at height `c_q` and longitude `west` to
/// the west, between two arcs `tilt` either side of the great circle
/// through both: hanging aslant, so that straight down from `p` is in the
/// face. `rim_first` puts the rim first in the zone's loop, else the
/// pocket's first arc. The sphere's own frame is `own`, about the same
/// centre.
#[allow(clippy::too_many_arguments)]
fn pinched(
    at: &Frame,
    own: &Frame,
    r: f64,
    c: f64,
    (c_q, west): (f64, f64),
    tilt: f64,
    rim_first: bool,
) -> Body {
    let at_longitude = |z: f64, longitude: f64| {
        let across = (r * r - z * z).sqrt();
        at.point(across * longitude.cos(), across * longitude.sin(), z)
    };
    let (p, q) = (at_longitude(c, 0.0), at_longitude(c_q, -west));
    let mut body = Body::new(BodyKind::Sheet);
    let vp = body.add_vertex(p);
    let vq = body.add_vertex(q);
    let rim_frame = Frame::new(at.point(0.0, 0.0, c), at.z(), p - at.point(0.0, 0.0, c)).unwrap();
    let rim = Curve3::Circle(Circle::new(rim_frame, (r * r - c * c).sqrt()).unwrap());
    let rim = body.add_edge(rim, (0.0, TAU), vp, vp);
    // The planes through p and q, turned either way about the line
    // between them from the plane of the great circle through them.
    let axis = (q - p).normalized().unwrap();
    let meridian = (p - at.origin())
        .cross(q - at.origin())
        .normalized()
        .unwrap();
    let turned = |angle: f64| {
        let side = axis.cross(meridian);
        meridian * angle.cos() + side * angle.sin()
    };
    let left = (p - at.origin()).cross(q - p);
    let mut arcs = Vec::new();
    for angle in [tilt, -tilt] {
        let (curve, end) = arc(at, r, p, q, turned(angle));
        let middle = curve.eval(0.5 * end);
        arcs.push((middle.dot(left), body.add_edge(curve, (0.0, end), vp, vq)));
    }
    // The arc on the left of the way from p to q (seen from outside)
    // first: the one round the pocket clockwise.
    arcs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (east, west) = (arcs[0].1, arcs[1].1);
    // Seen from outside, the face lies left of its loops: west along the
    // rim (the face is below it), down the left arc and up the other
    // (round the pocket clockwise, the face outside it).
    let rim_use = Coedge {
        edge: rim,
        reversed: true,
    };
    let pocket = [
        Coedge {
            edge: east,
            reversed: false,
        },
        Coedge {
            edge: west,
            reversed: true,
        },
    ];
    let zone = if rim_first {
        vec![rim_use, pocket[0], pocket[1]]
    } else {
        vec![pocket[0], pocket[1], rim_use]
    };
    let zone = body.add_loop(zone);
    let inside = body.add_loop(vec![
        Coedge {
            edge: west,
            reversed: false,
        },
        Coedge {
            edge: east,
            reversed: true,
        },
    ]);
    let sphere = Surface::Sphere(Sphere::new(*own, r).unwrap());
    let f0 = body.add_face(sphere.clone(), false, vec![zone]);
    let f1 = body.add_face(sphere, false, vec![inside]);
    body.add_shell(vec![f0, f1]);
    body
}

fn area(mesh: &craft3d_tess::Mesh, face: u32) -> f64 {
    mesh.triangles
        .iter()
        .zip(&mesh.face_ids)
        .filter(|(_, f)| **f == face)
        .map(|([a, b, c], _)| {
            let (a, b, c) = (
                mesh.positions[*a as usize],
                mesh.positions[*b as usize],
                mesh.positions[*c as usize],
            );
            0.5 * (b - a).cross(c - a).length()
        })
        .sum()
}

#[test]
fn a_zone_with_a_pocket_hanging_from_its_rim_meshes_whichever_way_it_is_laid_out() {
    for k in [1e-3, 1.0, 1e3] {
        let r = 5.0 * k;
        let centre = Vec3::new(0.3 * k, -0.2 * k, 0.1 * k);
        // The rim above the equator with the pole below in the face, and
        // the same upside down (the rim below, the face above it); either
        // way at a longitude away from the sphere's own seam, and the
        // sphere's own frame either way up.
        for (up, longitude) in [(Vec3::Z, 2.0_f64), (-Vec3::Z, 4.4)] {
            let turn = Vec3::new(longitude.cos(), longitude.sin(), 0.0);
            let at = Frame::new(centre, up, turn).unwrap();
            for own_up in [Vec3::Z, -Vec3::Z] {
                let own = Frame::new(centre, own_up, Vec3::X).unwrap();
                for rim_first in [true, false] {
                    let (c, c_q) = (0.4 * r, -0.1 * r);
                    let body = pinched(&at, &own, r, c, (c_q, 0.8), 0.25, rim_first);
                    let chord = 1e-3 * r;
                    let what =
                        format!("scale {k}, up {up:?}, own {own_up:?}, rim first {rim_first}");
                    let mesh = tessellate_body(
                        &body,
                        &TessParams {
                            chordal_tolerance: chord,
                            ..TessParams::default()
                        },
                    )
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
                    let (zone, pocket) = (area(&mesh, 0), area(&mesh, 1));
                    // The zone in closed form; a mesh within the chordal
                    // tolerance of a sphere is short of its area by about
                    // 2 chord / r of it (a chord's sag over a circle of
                    // radius r), within 1 % here.
                    let exact = TAU * r * (r + c);
                    assert!(pocket > 0.0, "{what}: {pocket}");
                    assert!(
                        (zone + pocket - exact).abs() <= 1e-2 * exact,
                        "{what}: {zone} + {pocket} against {exact}"
                    );
                }
            }
        }
    }
}
