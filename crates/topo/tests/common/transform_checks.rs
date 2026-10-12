//! What must hold for the image of a body under an isometry, checked
//! without the code under test's own reasoning: the checker, the mass
//! properties, and geometry compared point by point with the transform
//! applied to the original.
//!
//! Shared by `crates/topo/tests/transform.rs` and (through `#[path]`)
//! `crates/kernel/tests/transform.rs`, which can build the extrusions that
//! `craft3d-topo`'s own tests cannot reach.

#![allow(dead_code)]

use craft3d_geom::Surface;
use craft3d_math::{Tolerance, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, Face, check, mass_properties};

/// The document-style tolerance at a model scale.
pub fn tolerance(scale: f64) -> Tolerance {
    Tolerance::new(1e-6 * scale, 1e-9).unwrap()
}

/// Named isometries at the size of the geometry: rigid motions and
/// mirrors, plain and composed.
pub fn isometries(scale: f64) -> Vec<(&'static str, Transform)> {
    let spin = Transform::rotation_about(
        Vec3::new(3.0, 1.0, -2.0) * scale,
        Vec3::new(1.0, 1.0, 1.0),
        2.1,
    )
    .unwrap();
    let slide = Transform::translation(Vec3::new(-7.0, 0.25, 40.0) * scale);
    let flip =
        Transform::mirror(Vec3::new(0.0, 5.0, 0.0) * scale, Vec3::new(1.0, -3.0, 2.0)).unwrap();
    vec![
        ("identity", Transform::IDENTITY),
        ("translation", slide),
        ("rotation", spin),
        ("rotation then translation", spin.then(&slide)),
        (
            "mirror in the xy plane",
            Transform::mirror(Vec3::ZERO, Vec3::Z).unwrap(),
        ),
        (
            "mirror in the yz plane",
            Transform::mirror(Vec3::ZERO, Vec3::X).unwrap(),
        ),
        ("oblique mirror", flip),
        (
            "rotation, mirror, translation",
            spin.then(&flip).then(&slide),
        ),
        ("point mirror", Transform::scaling(-1.0)),
    ]
}

/// The outward unit normal of `face` at the surface point closest to `p`.
pub fn outward(face: &Face, p: Vec3) -> Option<Vec3> {
    let (u, v) = face.surface.project(p).ok()?;
    let n = face.surface.normal(u, v)?;
    Some(if face.reversed { -n } else { n })
}

fn on_surface(surface: &Surface, p: Vec3) -> Vec3 {
    let (u, v) = surface.project(p).unwrap();
    surface.eval(u, v)
}

/// The point of `face` a distance `step` to the left of `coedge` (seen
/// from outside) at fraction `k/4` of its edge: by the loop convention a
/// point inside the face, next to its boundary. `None` where the surface
/// has no normal (an apex).
pub fn left_of(body: &Body, face: &Face, coedge: Coedge, k: u32, step: f64) -> Option<Vec3> {
    let edge = body.edge(coedge.edge).unwrap();
    let (t0, t1) = edge.range;
    let (p, d) = edge.curve.derivatives(t0 + (t1 - t0) * f64::from(k) / 4.0);
    let along = if coedge.reversed { -d } else { d }.normalized()?;
    let n = outward(face, p)?;
    Some(on_surface(&face.surface, p + n.cross(along) * step))
}

/// Points of every face of `body`, with the index of their face: next to
/// each boundary edge on the inside, and for a face without loops a grid
/// over the whole surface.
pub fn face_points(body: &Body, step: f64) -> Vec<(usize, Vec3)> {
    let mut out = Vec::new();
    for (i, face) in body.faces.iter().enumerate() {
        if face.loops.is_empty() {
            let ((u0, u1), (v0, v1)) = face.surface.domain();
            for a in 0..12 {
                for b in 1..12 {
                    let u = u0 + (u1 - u0) * f64::from(a) / 12.0;
                    let v = v0 + (v1 - v0) * f64::from(b) / 12.0;
                    out.push((i, face.surface.eval(u, v)));
                }
            }
        }
        for loop_id in &face.loops {
            for &coedge in &body.loop_(*loop_id).unwrap().coedges {
                for k in 1..4 {
                    out.extend(left_of(body, face, coedge, k, step).map(|p| (i, p)));
                }
            }
        }
    }
    out
}

/// The largest coordinate around: what rounding errors are relative to.
fn reach(body: &Body, image: &Body, scale: f64) -> f64 {
    let mut most = scale;
    for b in [body, image] {
        if let Some((lo, hi)) = b.bbox().unwrap().corners() {
            for c in [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z] {
                most = most.max(c.abs());
            }
        }
    }
    most
}

/// Transforms `body` (which must be valid) by `t` and checks everything
/// that can be said about the result. Returns it.
pub fn check_image(what: &str, body: &Body, t: &Transform, scale: f64) -> Body {
    let mirror = t.determinant() < 0.0;
    let image = body
        .transformed(t)
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    let reach = reach(body, &image, scale);
    let tight = 1e-12 * reach;

    // The same entities, in the same order.
    assert_eq!(image.kind, body.kind, "{what}");
    assert_eq!(image.vertices.len(), body.vertices.len(), "{what}");
    assert_eq!(image.edges.len(), body.edges.len(), "{what}");
    assert_eq!(image.loops.len(), body.loops.len(), "{what}");
    assert_eq!(image.faces.len(), body.faces.len(), "{what}");
    assert_eq!(image.shells, body.shells, "{what}");

    // Valid by the checker.
    assert_eq!(check(body, &tolerance(scale)), vec![], "{what}: original");
    assert_eq!(check(&image, &tolerance(scale)), vec![], "{what}: image");

    // Every vertex is the image of the vertex with the same id.
    for (i, (v, w)) in body.vertices.iter().zip(&image.vertices).enumerate() {
        assert!(
            w.point.distance(t.point(v.point)) <= tight,
            "{what}: vertex {i}"
        );
    }

    // Every edge is the image of the edge with the same id, point for
    // point, and its ends are at its vertices.
    for (i, (e, f)) in body.edges.iter().zip(&image.edges).enumerate() {
        assert_eq!((f.start, f.end), (e.start, e.end), "{what}: edge {i}");
        let (t0, t1) = f.range;
        assert!(t0 < t1, "{what}: edge {i}");
        assert_eq!(f.range, e.range, "{what}: edge {i} keeps its range");
        assert_eq!(f.curve.kind(), e.curve.kind(), "{what}: edge {i}");
        for k in 0..=8 {
            let s = t0 + (t1 - t0) * f64::from(k) / 8.0;
            assert!(
                f.curve.eval(s).distance(t.point(e.curve.eval(s))) <= tight,
                "{what}: edge {i} at {s}"
            );
        }
        for (vertex, s) in [(f.start, t0), (f.end, t1)] {
            let p = image.vertex(vertex).unwrap().point;
            // The original may itself be off by its modelling tolerance;
            // the transform must not add to that.
            let before = e.curve.eval(s).distance(body.vertex(vertex).unwrap().point);
            assert!(
                f.curve.eval(s).distance(p) <= before + tight,
                "{what}: edge {i} misses vertex {}",
                vertex.0
            );
        }
    }

    // Faces: the same loops, each walked the other way under a mirror.
    let step = 1e-3 * scale;
    for (i, (face, image_face)) in body.faces.iter().zip(&image.faces).enumerate() {
        assert_eq!(image_face.loops, face.loops, "{what}: face {i}");
        assert_eq!(image_face.surface.kind(), face.surface.kind(), "{what}");
        for loop_id in &face.loops {
            let coedges = &body.loop_(*loop_id).unwrap().coedges;
            let image_coedges = &image.loop_(*loop_id).unwrap().coedges;
            assert_eq!(image_coedges.len(), coedges.len(), "{what}: face {i}");
            assert_eq!(
                image_coedges[0].edge, coedges[0].edge,
                "{what}: loop {} starts with the same edge",
                loop_id.0
            );
            for &coedge in coedges {
                // The use of this edge by the image loop: the same sense
                // for a rigid motion, the opposite for a mirror. (A seam is
                // used twice by one loop, once each way.)
                let twin = Coedge {
                    edge: coedge.edge,
                    reversed: coedge.reversed != mirror,
                };
                assert!(
                    image_coedges.contains(&twin),
                    "{what}: face {i}, edge {}",
                    coedge.edge.0
                );
                for k in 1..4 {
                    // Left of the coedge, seen from outside, is inside the
                    // face. That point of the image, worked out from the
                    // image alone, is the image of that point.
                    let Some(inside) = left_of(body, face, coedge, k, step) else {
                        continue;
                    };
                    let image_inside = left_of(&image, image_face, twin, k, step)
                        .unwrap_or_else(|| panic!("{what}: face {i} lost a normal"));
                    assert!(
                        image_inside.distance(t.point(inside)) <= 1e-4 * step + 1e-9 * reach,
                        "{what}: face {i}, edge {}: the face is on the wrong side of its \
                         boundary ({:e} from where it should be, step {step:e})",
                        coedge.edge.0,
                        image_inside.distance(t.point(inside))
                    );
                }
            }
        }
    }

    // Points of each face map onto the image face's surface, and the
    // outward normal there is the image of the outward normal (for a
    // mirror: the reflected one).
    for (i, p) in face_points(body, step) {
        let (face, image_face) = (&body.faces[i], &image.faces[i]);
        let q = t.point(p);
        let distance = on_surface(&image_face.surface, q).distance(q);
        assert!(
            distance <= 1e-9 * reach,
            "{what}: face {i}: a mapped point is {distance:e} off the image surface"
        );
        let (Some(n), Some(m)) = (outward(face, p), outward(image_face, q)) else {
            continue;
        };
        assert!(
            m.distance(t.vector(n)) <= 1e-6,
            "{what}: face {i}: outward normal {m:?}, expected {:?}",
            t.vector(n)
        );
    }

    // The box contains everything sampled.
    let bbox = image.bbox().unwrap_or_else(|e| panic!("{what}: {e}"));
    for (i, p) in face_points(&image, step) {
        assert!(
            bbox.contains(p, 0.0),
            "{what}: face {i} sticks out at {p:?}"
        );
    }
    for (i, edge) in image.edges.iter().enumerate() {
        for k in 0..=256 {
            let s = edge.range.0 + (edge.range.1 - edge.range.0) * f64::from(k) / 256.0;
            assert!(
                bbox.contains(edge.curve.eval(s), 0.0),
                "{what}: edge {i} sticks out at {s}"
            );
        }
    }
    for (i, v) in image.vertices.iter().enumerate() {
        assert!(bbox.contains(v.point, 0.0), "{what}: vertex {i}");
    }

    // Deterministic to the bit.
    let again = body.transformed(t).unwrap();
    assert_eq!(format!("{again:?}"), format!("{image:?}"), "{what}");
    assert_eq!(
        format!("{:?}", again.bbox().unwrap()),
        format!("{bbox:?}"),
        "{what}"
    );
    image
}

/// The mass properties of the image are those of the original, moved:
/// the same area, the same *positive* volume, the centroid mapped.
pub fn check_mass(what: &str, body: &Body, image: &Body, t: &Transform, scale: f64) {
    let before = mass_properties(body).unwrap_or_else(|e| panic!("{what}: {e}"));
    let after = mass_properties(image).unwrap_or_else(|e| panic!("{what}: image: {e}"));
    assert!(
        (after.area - before.area).abs() <= 1e-12 * before.area,
        "{what}: area {:e} became {:e} (off by {:e} relative)",
        before.area,
        after.area,
        (after.area - before.area).abs() / before.area
    );
    if body.kind == BodyKind::Sheet {
        return;
    }
    assert!(before.volume > 0.0, "{what}: the original is inside out");
    assert!(
        after.volume > 0.0,
        "{what}: the image is inside out (volume {:e})",
        after.volume
    );
    assert!(
        (after.volume - before.volume).abs() <= 1e-12 * before.volume,
        "{what}: volume {:e} became {:e} (off by {:e} relative)",
        before.volume,
        after.volume,
        (after.volume - before.volume).abs() / before.volume
    );
    let reach = reach(body, image, scale);
    let (c, d) = (before.centroid.unwrap(), after.centroid.unwrap());
    assert!(
        d.distance(t.point(c)) <= 1e-12 * reach,
        "{what}: centroid {d:?}, expected {:?} ({:e} of the reach)",
        t.point(c),
        d.distance(t.point(c)) / reach
    );
}

/// There and back again is the original: the same topology exactly, the
/// same geometry up to rounding.
pub fn check_round_trip(what: &str, body: &Body, image: &Body, t: &Transform, scale: f64) {
    let back = image.transformed(&t.inverse().unwrap()).unwrap();
    let tight = 1e-12 * reach(body, image, scale);
    assert_eq!(back.loops, body.loops, "{what}");
    assert_eq!(back.shells, body.shells, "{what}");
    for (i, (v, w)) in body.vertices.iter().zip(&back.vertices).enumerate() {
        assert!(w.point.distance(v.point) <= tight, "{what}: vertex {i}");
    }
    for (i, (e, f)) in body.edges.iter().zip(&back.edges).enumerate() {
        assert_eq!((f.range, f.start, f.end), (e.range, e.start, e.end));
        for k in 0..=4 {
            let s = e.range.0 + (e.range.1 - e.range.0) * f64::from(k) / 4.0;
            assert!(
                f.curve.eval(s).distance(e.curve.eval(s)) <= tight,
                "{what}: edge {i}"
            );
        }
    }
    for (i, (f, g)) in body.faces.iter().zip(&back.faces).enumerate() {
        assert_eq!((g.reversed, &g.loops), (f.reversed, &f.loops), "{what}");
        // Mirrored twice or not at all: the parametrisation is back.
        let ((u0, u1), (v0, v1)) = f.surface.domain();
        let span = |a: f64, b: f64| match (a.is_finite(), b.is_finite()) {
            (true, true) => (a, b),
            (true, false) => (a, a + 4.0 * scale),
            (false, true) => (b - 4.0 * scale, b),
            (false, false) => (-3.0 * scale, 3.0 * scale),
        };
        let ((u0, u1), (v0, v1)) = (span(u0, u1), span(v0, v1));
        for a in 0..=4 {
            for b in 0..=4 {
                let u = u0 + (u1 - u0) * f64::from(a) / 4.0;
                let v = v0 + (v1 - v0) * f64::from(b) / 4.0;
                assert!(
                    g.surface.eval(u, v).distance(f.surface.eval(u, v)) <= tight,
                    "{what}: face {i} at ({u}, {v})"
                );
            }
        }
    }
}
