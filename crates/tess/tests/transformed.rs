//! Bodies moved and mirrored by `Body::transformed`, seen through the
//! tessellator: an oracle that shares nothing with the transform, the
//! checker or the mass properties. The mesh of the image is a closed
//! manifold facing outward, with a positive volume that converges to the
//! closed form, and it lies inside `Body::bbox`.
//!
//! This is also the only volume oracle for the mirrored pipe elbow and
//! sphere octant, which `mass_properties` cannot integrate.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_testkit::solids::{self, Reference};
use craft3d_testkit::trimmed;
use craft3d_topo::{Body, merge};

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    }
}

fn tilted(scale: f64) -> Frame {
    Frame::new(
        Vec3::new(1.0, -2.0, 0.5) * scale,
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

fn fixtures(frame: &Frame, scale: f64) -> Vec<Reference> {
    let mut all = solids::all(frame, scale).unwrap();
    all.extend(trimmed::all(frame, scale).unwrap());
    all.push(trimmed::pipe_elbow(frame, 3.0 * scale, 1.0 * scale, 1.9).unwrap());
    all.push(trimmed::sphere_octant(frame, 2.0 * scale).unwrap());
    all
}

fn isometries(scale: f64) -> Vec<(&'static str, Transform)> {
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
        ("rotation then translation", spin.then(&slide)),
        (
            "mirror in the xy plane",
            Transform::mirror(Vec3::ZERO, Vec3::Z).unwrap(),
        ),
        ("oblique mirror", flip),
        (
            "rotation, mirror, translation",
            spin.then(&flip).then(&slide),
        ),
    ]
}

/// Checks that `mesh` is a closed manifold whose triangles face the way
/// the faces of `body` do, and returns its volume.
fn closed_outward_volume(what: &str, body: &Body, mesh: &Mesh) -> f64 {
    let welded = mesh.weld().unwrap();
    assert!(
        welded.is_closed(),
        "{what}: the mesh is not a closed manifold"
    );
    let (mut agree, mut total) = (0.0, 0.0);
    for (t, &face) in mesh.face_ids.iter().enumerate() {
        let face = &body.faces[face as usize];
        let [a, b, c] = mesh.corners(t).unwrap();
        let cross = (b - a).cross(c - a);
        let centroid = (a + b + c) * (1.0 / 3.0);
        let (u, v) = face.surface.project(centroid).unwrap();
        let Some(n) = face.surface.normal(u, v) else {
            continue;
        };
        let outward = if face.reversed { -n } else { n };
        // By area: slivers next to a pole or an apex have no say.
        total += cross.length();
        if cross.dot(outward) > 0.0 {
            agree += cross.length();
        }
    }
    assert!(
        agree >= (1.0 - 1e-9) * total,
        "{what}: only {:.6} of the mesh area faces outward",
        agree / total
    );
    welded.volume()
}

#[test]
fn meshes_of_moved_and_mirrored_solids_are_closed_outward_and_inside_the_box() {
    for scale in [1e-3, 1.0, 1e6] {
        let frame = tilted(scale);
        for solid in fixtures(&frame, scale) {
            let tolerance = 1e-3 * scale;
            let original = tessellate_body(&solid.body, &params(tolerance)).unwrap();
            let original_volume = original.weld().unwrap().volume();
            for (name, t) in isometries(scale) {
                let what = format!("{} at scale {scale} under {name}", solid.name);
                let image = solid.body.transformed(&t).unwrap();
                let mesh = tessellate_body(&image, &params(tolerance))
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
                let volume = closed_outward_volume(&what, &image, &mesh);
                assert!(volume > 0.0, "{what}: the mesh is inside out ({volume:e})");
                // The same body at the same tolerance, somewhere else: the
                // volume is the closed form within what the original's
                // own mesh is off by, and a little for a different grid.
                let off = (original_volume - solid.volume).abs() / solid.volume;
                let error = (volume - solid.volume).abs() / solid.volume;
                assert!(
                    error <= 3.0 * off + 1e-9,
                    "{what}: volume error {error:e}, the original's is {off:e}"
                );
                assert!(error < 5e-3, "{what}: volume error {error:e}");

                // Every mesh vertex is a point of the body: inside its box.
                let bbox = image.bbox().unwrap();
                for (i, p) in mesh.positions.iter().enumerate() {
                    assert!(
                        bbox.contains(*p, 0.0),
                        "{what}: mesh vertex {i} at {p:?} is outside {bbox:?}"
                    );
                }
                // And where the box is promised to be tight it is: no
                // further out than the mesh, which is within the chordal
                // tolerance of the body.
                let exact = image.faces.iter().all(|f| match f.surface {
                    Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_) => true,
                    Surface::Sphere(_) | Surface::Torus(_) => f.loops.is_empty(),
                    Surface::Bezier(_) | Surface::Nurbs(_) => false,
                }) && image
                    .edges
                    .iter()
                    .all(|e| !matches!(e.curve, Curve3::Nurbs(_)));
                if exact {
                    let (lo, hi) = bbox.corners().unwrap();
                    let of_mesh = mesh.bbox();
                    assert!(
                        of_mesh.contains(lo, 2.0 * tolerance)
                            && of_mesh.contains(hi, 2.0 * tolerance),
                        "{what}: box {bbox:?} is looser than the mesh's {of_mesh:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn volumes_of_mirrored_solids_converge_to_the_closed_forms() {
    let frame = tilted(1.0);
    let mirror = Transform::mirror(Vec3::new(0.0, 5.0, 0.0), Vec3::new(1.0, -3.0, 2.0)).unwrap();
    for solid in fixtures(&frame, 1.0) {
        let image = solid.body.transformed(&mirror).unwrap();
        let mut errors = Vec::new();
        for tolerance in [1e-1, 1e-2, 1e-3] {
            let mesh = tessellate_body(&image, &params(tolerance)).unwrap();
            let what = format!("mirrored {} at tolerance {tolerance:e}", solid.name);
            let volume = closed_outward_volume(&what, &image, &mesh);
            assert!(volume > 0.0, "{what}");
            let area_error = (mesh.area() - solid.area).abs() / solid.area;
            errors.push(((volume - solid.volume).abs() / solid.volume, area_error));
        }
        println!("mirrored {:28}: {errors:?}", solid.name);
        let (first, last) = (errors[0], errors[2]);
        // Planar solids are exact from the start; the others get better.
        assert!(
            last.0 <= first.0.max(1e-12) && last.1 <= first.1.max(1e-12),
            "{}: {errors:?}",
            solid.name
        );
        // At a chordal tolerance of 1e-3 on solids a few units across.
        assert!(last.0 < 2e-3 && last.1 < 2e-3, "{}: {errors:?}", solid.name);
    }
}

#[test]
fn a_merged_ring_of_copies_tessellates_as_one_closed_body() {
    let frame = tilted(1.0);
    for solid in fixtures(&frame, 1.0) {
        // Three copies well apart, the middle one mirrored.
        let step = Vec3::new(60.0, 0.0, 0.0);
        let copies = [
            solid
                .body
                .transformed(&Transform::translation(step * -1.0))
                .unwrap(),
            solid
                .body
                .transformed(&Transform::mirror(Vec3::ZERO, Vec3::Y).unwrap())
                .unwrap(),
            solid
                .body
                .transformed(&Transform::rotation(Vec3::Z, 1.0).unwrap())
                .unwrap()
                .transformed(&Transform::translation(step))
                .unwrap(),
        ];
        let refs: Vec<&Body> = copies.iter().collect();
        let (merged, offsets) = merge(&refs).unwrap();
        let mesh = tessellate_body(&merged, &params(1e-3)).unwrap();
        let what = format!("three of {}", solid.name);
        let volume = closed_outward_volume(&what, &merged, &mesh);
        let error = (volume - 3.0 * solid.volume).abs() / (3.0 * solid.volume);
        assert!(error < 5e-3, "{what}: volume error {error:e}");
        // Triangles of copy k carry face ids at its offset.
        let faces = solid.body.faces.len() as u32;
        for (k, at) in offsets.iter().enumerate() {
            assert_eq!(at.faces, k as u32 * faces, "{what}");
            let part = tessellate_body(&copies[k], &params(1e-3)).unwrap();
            let in_merged = mesh
                .face_ids
                .iter()
                .filter(|&&f| f >= at.faces && f < at.faces + faces)
                .count();
            assert_eq!(in_merged, part.triangles.len(), "{what}: copy {k}");
        }
        let bbox = merged.bbox().unwrap();
        assert!(
            mesh.positions.iter().all(|p| bbox.contains(*p, 0.0)),
            "{what}"
        );
    }
}
