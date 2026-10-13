//! Faces that are a whole sphere but for holes whose loops do not go
//! around the sphere's axis: the zone left of a ball with a hole drilled
//! through it across its axis, and a ball with a flat on its side. In the
//! sphere's parameters such a face is everything *outside* its loops.
//!
//! The bodies are built by hand, as a boolean makes them (a ball minus a
//! rod: `crates/testkit/tests/boolean_tess.rs`), and held to oracle 4 of
//! docs/testing.md with closed forms: the napkin ring's volume
//! `4/3 π (R² − a²)^{3/2}` and its zone's area `2π R h`, `h = 2 √(R² − a²)`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Cylinder, Plane, Sphere, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_topo::{Body, BodyKind, Coedge, check, mass_properties};
use std::f64::consts::{PI, TAU};

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    }
}

/// A solid with what is known about it in closed form. `face_areas` are
/// the areas of its faces, in face order.
struct Solid {
    name: String,
    body: Body,
    volume: f64,
    face_areas: Vec<f64>,
}

/// A direction `angle` away from `z`, turned by `turn` about it.
fn off_axis(frame: &Frame, angle: f64, turn: f64) -> Vec3 {
    frame.vector(
        angle.sin() * turn.cos(),
        angle.sin() * turn.sin(),
        angle.cos(),
    )
}

/// A ball of radius `r` about the origin of `frame` with a hole of radius
/// `a` through it along the frame's z. The sphere's own axis is `axis`
/// and its seam starts towards `towards`.
fn napkin_ring(frame: &Frame, r: f64, a: f64, axis: Vec3, towards: Vec3, name: &str) -> Solid {
    let half = (r * r - a * a).sqrt();
    let mut body = Body::new(BodyKind::Solid);
    let circle = |z: f64| {
        Curve3::Circle(Circle::new(frame.at(frame.point(0.0, 0.0, z)).unwrap(), a).unwrap())
    };
    let (low, high) = (circle(-half), circle(half));
    let v_low = body.add_vertex(low.eval(0.0));
    let v_high = body.add_vertex(high.eval(0.0));
    let e_low = body.add_edge(low, (0.0, TAU), v_low, v_low);
    let e_high = body.add_edge(high, (0.0, TAU), v_high, v_high);
    let use_of = |edge, reversed| vec![Coedge { edge, reversed }];
    let zone = [
        body.add_loop(use_of(e_low, false)),
        body.add_loop(use_of(e_high, true)),
    ];
    let wall = [
        body.add_loop(use_of(e_high, false)),
        body.add_loop(use_of(e_low, true)),
    ];
    let sphere = Sphere::new(Frame::new(frame.origin(), axis, towards).unwrap(), r).unwrap();
    let f_zone = body.add_face(Surface::Sphere(sphere), false, zone.to_vec());
    let cylinder = Cylinder::new(*frame, a).unwrap();
    let f_wall = body.add_face(Surface::Cylinder(cylinder), true, wall.to_vec());
    body.add_shell(vec![f_zone, f_wall]);
    Solid {
        name: format!("a ball with a hole, {name}"),
        body,
        volume: 4.0 / 3.0 * PI * half.powi(3),
        face_areas: vec![TAU * r * 2.0 * half, TAU * a * 2.0 * half],
    }
}

/// A ball of radius `r` about the origin of `frame`, cut off by the plane
/// at height `c` along the frame's z. The sphere's own axis is `axis`.
fn ball_with_a_flat(frame: &Frame, r: f64, c: f64, axis: Vec3, towards: Vec3, name: &str) -> Solid {
    let rim_radius = (r * r - c * c).sqrt();
    let height = r - c;
    let mut body = Body::new(BodyKind::Solid);
    let rim_frame = frame.at(frame.point(0.0, 0.0, c)).unwrap();
    let rim = Curve3::Circle(Circle::new(rim_frame, rim_radius).unwrap());
    let vertex = body.add_vertex(rim.eval(0.0));
    let edge = body.add_edge(rim, (0.0, TAU), vertex, vertex);
    let round = body.add_loop(vec![Coedge {
        edge,
        reversed: true,
    }]);
    let flat = body.add_loop(vec![Coedge {
        edge,
        reversed: false,
    }]);
    let sphere = Sphere::new(Frame::new(frame.origin(), axis, towards).unwrap(), r).unwrap();
    let f_round = body.add_face(Surface::Sphere(sphere), false, vec![round]);
    let f_flat = body.add_face(Surface::Plane(Plane::new(rim_frame)), false, vec![flat]);
    body.add_shell(vec![f_round, f_flat]);
    let cap = PI * height * height * (3.0 * r - height) / 3.0;
    Solid {
        name: format!("a ball with a flat, {name}"),
        body,
        volume: 4.0 / 3.0 * PI * r.powi(3) - cap,
        face_areas: vec![
            4.0 * PI * r * r - TAU * r * height,
            PI * rim_radius * rim_radius,
        ],
    }
}

fn frames(scale: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap(),
    ]
}

/// The bodies, with the sphere's axis in several places relative to the
/// holes. The hole of the ring is 30° wide seen from the centre, so an
/// axis more than that away from the hole's leaves both loops clear of
/// the poles: neither goes around the axis.
fn solids(frame: &Frame, scale: f64) -> Vec<Solid> {
    let (r, a) = (2.0 * scale, 1.0 * scale);
    let (x, y, z) = (frame.x(), frame.y(), frame.z());
    let mut all = vec![
        // As `kernel::primitives::sphere` and a boolean make it: the holes
        // at a quarter and three quarters of the way around.
        napkin_ring(frame, r, a, x, y, "its axis across the hole"),
        // The holes on the seam of the sphere and opposite it.
        napkin_ring(frame, r, a, x, z, "the hole on the seam"),
        napkin_ring(frame, r, a, x, -z, "the hole on the seam, the other way"),
        // The axis anywhere else, 40° to 75° from the hole's.
        napkin_ring(
            frame,
            r,
            a,
            off_axis(frame, 40.0_f64.to_radians(), 0.3),
            y,
            "its axis 40° off",
        ),
        napkin_ring(
            frame,
            r,
            a,
            off_axis(frame, 75.0_f64.to_radians(), 2.1),
            x + z,
            "its axis 75° off",
        ),
        // For comparison, what worked before: the hole along the axis,
        // both loops going around it.
        napkin_ring(frame, r, a, z, x, "along its axis"),
        napkin_ring(frame, r, a, -z, y, "along its axis, upside down"),
    ];
    for (c, name) in [(0.75, "a small one"), (-1.0, "more than half gone")] {
        let c = c * scale;
        all.push(ball_with_a_flat(
            frame,
            r,
            c,
            x,
            y,
            &format!("{name}, axis across"),
        ));
        all.push(ball_with_a_flat(
            frame,
            r,
            c,
            x,
            z,
            &format!("{name}, on the seam"),
        ));
        all.push(ball_with_a_flat(
            frame,
            r,
            c,
            -z,
            x,
            &format!("{name}, along the axis"),
        ));
    }
    all
}

fn bits(mesh: &Mesh) -> Vec<u64> {
    mesh.positions
        .iter()
        .chain(&mesh.normals)
        .flat_map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
        .collect()
}

fn face_area(mesh: &Mesh, face: u32) -> f64 {
    (0..mesh.triangles.len())
        .filter(|&t| mesh.face_ids[t] == face)
        .map(|t| {
            let [a, b, c] = mesh.corners(t).unwrap();
            (b - a).cross(c - a).length() * 0.5
        })
        .sum()
}

#[test]
fn the_bodies_are_valid_and_their_closed_forms_are_theirs() {
    for scale in [1e-3, 1.0, 1e6] {
        let tolerance = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
        for frame in frames(scale) {
            for solid in solids(&frame, scale) {
                let what = format!("{} at scale {scale}", solid.name);
                assert_eq!(check(&solid.body, &tolerance), vec![], "{what}");
                // By another route than the tessellator: the integrals of
                // `craft3d_topo` over the exact faces.
                let mass = mass_properties(&solid.body).unwrap();
                let area: f64 = solid.face_areas.iter().sum();
                assert!(
                    (mass.volume - solid.volume).abs() <= 1e-9 * solid.volume,
                    "{what}: volume {} vs {}",
                    mass.volume,
                    solid.volume
                );
                assert!(
                    (mass.area - area).abs() <= 1e-9 * area,
                    "{what}: area {} vs {area}",
                    mass.area
                );
            }
        }
    }
}

#[test]
fn meshes_are_closed_outward_and_within_tolerance() {
    for scale in [1e-3, 1.0, 1e6] {
        for frame in frames(scale) {
            for solid in solids(&frame, scale) {
                for rel in [1e-1, 1e-2, 1e-3] {
                    let tolerance = rel * scale;
                    let what = format!("{} at scale {scale}, tolerance {rel}", solid.name);
                    let mesh = match tessellate_body(&solid.body, &params(tolerance)) {
                        Ok(mesh) => mesh,
                        Err(e) => panic!("{what}: {e}"),
                    };
                    assert_eq!(mesh.normals.len(), mesh.positions.len(), "{what}");
                    assert_eq!(mesh.face_ids.len(), mesh.triangles.len(), "{what}");
                    let welded = mesh.weld().unwrap();
                    assert!(welded.is_closed(), "{what}: not a closed manifold");
                    for (t, &face) in mesh.face_ids.iter().enumerate() {
                        let face = &solid.body.faces[face as usize];
                        let [a, b, c] = mesh.corners(t).unwrap();
                        let distance = |p: Vec3| {
                            let (u, v) = face.surface.project(p).unwrap();
                            (
                                face.surface.eval(u, v).distance(p),
                                face.surface.normal(u, v),
                            )
                        };
                        for p in [a, b, c] {
                            assert!(distance(p).0 <= 1e-9 * scale, "{what}: vertex off its face");
                        }
                        let centroid = (a + b + c) * (1.0 / 3.0);
                        let (off, normal) = distance(centroid);
                        assert!(
                            off <= tolerance * (1.0 + 1e-9),
                            "{what}: centroid {off:e} from the surface"
                        );
                        let outward = normal.unwrap() * if face.reversed { -1.0 } else { 1.0 };
                        assert!(
                            (b - a).cross(c - a).dot(outward) > 0.0,
                            "{what}: triangle {t} faces inward"
                        );
                    }
                    // Within `tolerance` of the surface everywhere, so the
                    // volume is off by at most area × tolerance.
                    let area: f64 = solid.face_areas.iter().sum();
                    let volume = welded.volume();
                    assert!(
                        (volume - solid.volume).abs() <= area * tolerance,
                        "{what}: volume {volume} vs {}",
                        solid.volume
                    );
                }
            }
        }
    }
}

#[test]
fn volume_and_face_areas_converge_to_the_closed_forms() {
    for solid in solids(&Frame::WORLD, 1.0) {
        let mut previous = f64::INFINITY;
        let mut last = Vec::new();
        for tolerance in [1e-1, 1e-2, 1e-3, 1e-4] {
            let mesh = tessellate_body(&solid.body, &params(tolerance)).unwrap();
            let volume_error = (mesh.weld().unwrap().volume() - solid.volume).abs() / solid.volume;
            last = solid
                .face_areas
                .iter()
                .enumerate()
                .map(|(f, exact)| (face_area(&mesh, f as u32) - exact).abs() / exact)
                .collect();
            println!(
                "{:55} tol {tolerance:e}: {:7} triangles, volume error {volume_error:.2e}, face area errors {last:?}",
                solid.name,
                mesh.triangles.len()
            );
            assert!(volume_error <= previous, "{}", solid.name);
            previous = volume_error;
        }
        // A chord error of 1e-4 on a ball of radius 2: errors of that
        // order, relative to the whole.
        assert!(previous < 1e-3, "{}: {previous}", solid.name);
        assert!(last.iter().all(|e| *e < 1e-3), "{}: {last:?}", solid.name);
    }
}

#[test]
fn output_does_not_depend_on_the_number_of_threads() {
    for solid in solids(&frames(1.0)[1], 1.0) {
        let reference = tessellate_body_with_threads(&solid.body, &params(1e-3), 1).unwrap();
        for threads in [1, 2, 8, 0] {
            let mesh = tessellate_body_with_threads(&solid.body, &params(1e-3), threads).unwrap();
            assert_eq!(bits(&mesh), bits(&reference), "{}: {threads}", solid.name);
            assert_eq!(mesh, reference, "{}: {threads} threads", solid.name);
        }
    }
}

/// When the holes leave no meridian free for the seam, the face is
/// refused as before, with an error that names it; it does not come out
/// as a mesh of something else.
#[test]
fn a_sphere_with_holes_on_every_meridian_is_refused() {
    use craft3d_tess::TessError;
    // Twenty holes around the equator, each wider than the gap between
    // the sixteen places a seam is tried at. A sheet: only the sphere.
    let r = 2.0;
    let mut body = Body::new(BodyKind::Sheet);
    let mut loops = Vec::new();
    for k in 0..20 {
        let angle = TAU * f64::from(k) / 20.0 + 0.05;
        let out = Vec3::new(angle.cos(), angle.sin(), 0.0);
        let rim = (r * (TAU / 40.0).sin()) * 0.95;
        let centre = out * (r * r - rim * rim).sqrt();
        let circle =
            Curve3::Circle(Circle::new(Frame::new(centre, out, Vec3::Z).unwrap(), rim).unwrap());
        let vertex = body.add_vertex(circle.eval(0.0));
        let edge = body.add_edge(circle, (0.0, TAU), vertex, vertex);
        loops.push(body.add_loop(vec![Coedge {
            edge,
            reversed: true,
        }]));
    }
    let sphere = Sphere::new(Frame::WORLD, r).unwrap();
    let face = body.add_face(Surface::Sphere(sphere), false, loops);
    body.add_shell(vec![face]);
    let error = tessellate_body(&body, &params(1e-2)).unwrap_err();
    assert!(
        matches!(error, TessError::Unsupported { face: 0, .. }),
        "{error}"
    );
    assert!(error.to_string().contains("seam"), "{error}");
}

/// A loop that runs counter-clockwise with a hole in it is still the
/// part inside the loop, also on a sphere: only a face *all* of whose
/// loops run clockwise is everything outside them.
#[test]
fn a_ring_on_the_side_of_a_sphere_is_the_ring_not_the_rest() {
    let r = 2.0;
    let rim = |x: f64| {
        let plane = Frame::new(Vec3::new(x, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
        Curve3::Circle(Circle::new(plane, (r * r - x * x).sqrt()).unwrap())
    };
    let mut body = Body::new(BodyKind::Sheet);
    let mut loops = Vec::new();
    for (x, reversed) in [(1.0, false), (1.6, true)] {
        let circle = rim(x);
        let vertex = body.add_vertex(circle.eval(0.0));
        let edge = body.add_edge(circle, (0.0, TAU), vertex, vertex);
        loops.push(body.add_loop(vec![Coedge { edge, reversed }]));
    }
    let face = body.add_face(
        Surface::Sphere(Sphere::new(Frame::WORLD, r).unwrap()),
        false,
        loops,
    );
    body.add_shell(vec![face]);
    // The zone between the planes x = 1 and x = 1.6: 2π r times its width.
    let expected = TAU * r * 0.6;
    for tolerance in [1e-2, 1e-3] {
        let mesh = tessellate_body(&body, &params(tolerance)).unwrap();
        // Inscribed, so smaller; by the order of the tolerance over the
        // width of the ring.
        let short = expected - mesh.area();
        assert!(
            short > 0.0 && short < 10.0 * tolerance * expected,
            "at {tolerance}: {} vs {expected}",
            mesh.area()
        );
    }
}
