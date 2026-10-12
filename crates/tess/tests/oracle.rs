//! Oracle 4 (docs/testing.md) for every reference solid, the primitives
//! and the trimmed ones: the mesh is closed, outward, on the surfaces,
//! within the chordal tolerance, free of zero-area triangles, converges
//! to the closed forms, and does not depend on the number of threads.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::{Frame, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_testkit::solids::{self, Reference};
use craft3d_testkit::trimmed;

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
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

/// Every reference solid: the primitives, the trimmed solids, and the
/// two whose closed forms `mass_properties` cannot confirm (the pipe
/// elbow, by Pappus, and the sphere octant, an eighth of a sphere).
fn everything(frame: &Frame, scale: f64) -> Vec<Reference> {
    let mut all = solids::all(frame, scale).unwrap();
    all.extend(trimmed::all(frame, scale).unwrap());
    all.push(trimmed::pipe_elbow(frame, 3.0 * scale, 1.0 * scale, 1.9).unwrap());
    all.push(trimmed::sphere_octant(frame, 2.0 * scale).unwrap());
    all
}

/// Everything that must hold for the mesh of a solid at one tolerance.
fn assert_mesh_is_right(solid: &Reference, mesh: &Mesh, tolerance: f64, scale: f64, what: &str) {
    assert_eq!(mesh.normals.len(), mesh.positions.len(), "{what}");
    assert_eq!(mesh.face_ids.len(), mesh.triangles.len(), "{what}");
    assert!(
        mesh.face_ids
            .iter()
            .all(|&f| (f as usize) < solid.body.faces.len()),
        "{what}"
    );
    // Every face has triangles, in face order.
    assert!(mesh.face_ids.windows(2).all(|w| w[0] <= w[1]), "{what}");
    for face in 0..solid.body.faces.len() {
        assert!(
            mesh.face_ids.contains(&(face as u32)),
            "{what}: face {face} is empty"
        );
    }

    let welded = mesh.weld().unwrap();
    assert!(
        welded.is_closed(),
        "{what}: the welded mesh is not a closed manifold"
    );
    // Welding drops nothing: no triangle has two corners in one place.
    assert_eq!(welded.triangles.len(), mesh.triangles.len(), "{what}");

    for (t, &face_id) in mesh.face_ids.iter().enumerate() {
        let face = &solid.body.faces[face_id as usize];
        let [a, b, c] = mesh.corners(t).unwrap();
        let measure = |p: Vec3| {
            let (u, v) = face.surface.project(p).unwrap();
            (
                face.surface.eval(u, v).distance(p),
                face.surface.normal(u, v),
            )
        };
        // Every vertex is on its face's surface, to rounding.
        for p in [a, b, c] {
            let off = measure(p).0;
            assert!(
                off <= 1e-9 * scale,
                "{what}: a vertex of face {face_id} is {off:e} off its surface"
            );
        }
        // The middle of every side and of the triangle is within the
        // chordal tolerance of the surface.
        for (name, p) in [
            ("side ab", a.lerp(b, 0.5)),
            ("side bc", b.lerp(c, 0.5)),
            ("side ca", c.lerp(a, 0.5)),
        ] {
            let off = measure(p).0;
            assert!(
                off <= tolerance * (1.0 + 1e-9),
                "{what}: {name} of triangle {t} (face {face_id}) is {off:e} from the surface"
            );
        }
        let centroid = (a + b + c) * (1.0 / 3.0);
        let (off, normal) = measure(centroid);
        assert!(
            off <= tolerance * (1.0 + 1e-9),
            "{what}: the centroid of triangle {t} (face {face_id}) is {off:e} from the surface"
        );
        // It has area and faces the way the face does.
        let facing = (b - a).cross(c - a);
        assert!(
            facing.length() > 0.0,
            "{what}: triangle {t} (face {face_id}) has no area"
        );
        let outward = normal.unwrap() * if face.reversed { -1.0 } else { 1.0 };
        assert!(
            facing.dot(outward) > 0.0,
            "{what}: triangle {t} (face {face_id}) faces inward"
        );
    }
    // Per-vertex normals: unit length and along the triangle's facing,
    // or zero where the surface has none (a cone's apex).
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let [a, b, c] = mesh.corners(t).unwrap();
        let facing = (b - a).cross(c - a);
        for &k in tri {
            let n = mesh.normals[k as usize];
            if n != Vec3::ZERO {
                assert!((n.length() - 1.0).abs() < 1e-12, "{what}");
                assert!(
                    n.dot(facing) > 0.0,
                    "{what}: a vertex normal of triangle {t} points inward"
                );
            }
        }
    }

    // The mesh lies within `tolerance` of the surface, so its volume
    // differs from the exact one by at most area × tolerance.
    let volume = welded.volume();
    assert!(
        volume > 0.0 && (volume - solid.volume).abs() <= solid.area * tolerance,
        "{what}: volume {volume} vs {}",
        solid.volume
    );
}

/// Every solid at one scale in one frame, at a tolerance of a thousandth
/// of the scale. (One test per scale and frame, so that they run side by
/// side: measuring every triangle against a spline surface takes a while.)
fn every_solid_meshes_right(scale: f64, which: usize) {
    let frame = frames(scale)[which];
    for solid in everything(&frame, scale) {
        let tolerance = 1e-3 * scale;
        let what = format!("{} at scale {scale}, frame {which}", solid.name);
        let mesh = tessellate_body(&solid.body, &params(tolerance))
            .unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_mesh_is_right(&solid, &mesh, tolerance, scale, &what);
    }
}

#[test]
fn every_solid_meshes_right_at_a_small_scale() {
    every_solid_meshes_right(1e-3, 0);
}

#[test]
fn every_solid_meshes_right_at_a_small_scale_tilted() {
    every_solid_meshes_right(1e-3, 1);
}

#[test]
fn every_solid_meshes_right_at_unit_scale() {
    every_solid_meshes_right(1.0, 0);
}

#[test]
fn every_solid_meshes_right_at_unit_scale_tilted() {
    every_solid_meshes_right(1.0, 1);
}

#[test]
fn every_solid_meshes_right_at_a_large_scale() {
    every_solid_meshes_right(1e6, 0);
}

#[test]
fn every_solid_meshes_right_at_a_large_scale_tilted() {
    every_solid_meshes_right(1e6, 1);
}

/// Coarse and odd tolerances are where a general method breaks first:
/// few points, large triangles, seams and poles next to each other.
#[test]
fn every_solid_meshes_at_coarse_tolerances_too() {
    for (which, frame) in frames(1.0).iter().enumerate() {
        for solid in everything(frame, 1.0) {
            for tolerance in [0.3, 0.1, 0.037, 0.01] {
                let what = format!("{} at tolerance {tolerance}, frame {which}", solid.name);
                let mesh = tessellate_body(&solid.body, &params(tolerance))
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
                assert_mesh_is_right(&solid, &mesh, tolerance, 1.0, &what);
            }
        }
    }
}

#[test]
fn volume_and_area_converge_for_every_solid() {
    for solid in everything(&Frame::WORLD, 1.0) {
        let mut previous = (f64::INFINITY, f64::INFINITY);
        let mut first = None;
        // Three decades.
        for tolerance in [1e-1, 1e-2, 1e-3, 1e-4] {
            let mesh = tessellate_body(&solid.body, &params(tolerance)).unwrap();
            let volume_error = (mesh.weld().unwrap().volume() - solid.volume).abs() / solid.volume;
            let area_error = (mesh.area() - solid.area).abs() / solid.area;
            println!(
                "{:28} tol {tolerance:e}: {:7} triangles, volume error {volume_error:.2e}, area error {area_error:.2e}",
                solid.name,
                mesh.triangles.len()
            );
            // The bound that holds whatever the shape: a mesh within
            // `tolerance` of the surface encloses the exact volume to
            // within area × tolerance.
            assert!(
                volume_error * solid.volume <= solid.area * tolerance,
                "{}: volume error {volume_error:e} at tolerance {tolerance}",
                solid.name
            );
            // Planar solids are exact from the start, so "shrinks" allows
            // "stays at rounding".
            assert!(
                area_error <= previous.1.max(1e-13),
                "{}: the area error grew to {area_error:e} at tolerance {tolerance}",
                solid.name
            );
            // The volume error shrinks step by step where the surface
            // bulges one way only, so that every triangle errs to the
            // same side. The roof of the domed block is convex in places
            // and concave in others: there the errors of the two kinds
            // partly cancel, more at one tolerance than at another, and
            // only the bound above and the overall trend below hold.
            let one_sided = !solid.name.starts_with("domed block");
            assert!(
                !one_sided || volume_error <= previous.0.max(1e-13),
                "{}: the volume error grew to {volume_error:e} at tolerance {tolerance}",
                solid.name
            );
            previous = (volume_error, area_error);
            first.get_or_insert(previous);
        }
        // Over three decades of tolerance both errors, which are linear
        // in it, drop by far more than a factor of a hundred.
        let first = first.unwrap();
        assert!(
            previous.0 <= (first.0 / 100.0).max(1e-13)
                && previous.1 <= (first.1 / 100.0).max(1e-13),
            "{}: from {first:?} to {previous:?}",
            solid.name
        );
        assert!(
            previous.0 < 1e-3 && previous.1 < 1e-3,
            "{}: {previous:?}",
            solid.name
        );
    }
}

#[test]
fn output_does_not_depend_on_the_number_of_threads() {
    for (which, frame) in frames(1.0).iter().enumerate() {
        for solid in everything(frame, 1.0) {
            let what = format!("{}, frame {which}", solid.name);
            let reference = tessellate_body_with_threads(&solid.body, &params(1e-3), 1).unwrap();
            assert!(!reference.triangles.is_empty(), "{what}");
            for threads in [1, 2, 8, 0] {
                let mesh =
                    tessellate_body_with_threads(&solid.body, &params(1e-3), threads).unwrap();
                // Byte-equal, not merely close.
                let bits = |m: &Mesh| -> Vec<u64> {
                    m.positions
                        .iter()
                        .chain(&m.normals)
                        .flat_map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
                        .collect()
                };
                assert_eq!(bits(&mesh), bits(&reference), "{what}: {threads} threads");
                assert_eq!(mesh, reference, "{what}: {threads} threads");
            }
            assert_eq!(
                tessellate_body(&solid.body, &params(1e-3)).unwrap(),
                reference,
                "{what}: default threads"
            );
        }
    }
}

/// Many faces, so that the threads really share work: errors come back
/// as the error of the first failing face whatever the thread count.
#[test]
fn the_first_failing_face_is_reported_for_any_number_of_threads() {
    let mut body = trimmed::block_with_rectangular_hole(&Frame::WORLD, 5.0, 4.0, 2.0, 2.0, 1.0)
        .unwrap()
        .body;
    // Faces 3 and 7 lose their loops: a plane without loops is unbounded.
    body.faces[3].loops.clear();
    body.faces[7].loops.clear();
    for threads in [1, 2, 8] {
        let error = tessellate_body_with_threads(&body, &params(1e-2), threads).unwrap_err();
        assert!(
            matches!(error, craft3d_tess::TessError::Face { face: 3, .. }),
            "{threads} threads: {error}"
        );
    }
}

/// Planar faces get no interior points: a polygon with n boundary points
/// and h holes gives n + 2 (h − 1) triangles, whatever the tolerance.
#[test]
fn planar_faces_stay_minimal() {
    let solid =
        trimmed::block_with_rectangular_hole(&Frame::WORLD, 5.0, 4.0, 2.0, 2.0, 1.0).unwrap();
    for tolerance in [10.0, 1e-9] {
        let mesh = tessellate_body(&solid.body, &params(tolerance)).unwrap();
        // 8 walls of 2 triangles, floor and lid of 4 + 4 + 2 (1 − 1) = 8.
        assert_eq!(mesh.triangles.len(), 8 * 2 + 2 * 8);
        assert_eq!(mesh.weld().unwrap().positions.len(), 16);
        assert!((mesh.area() - solid.area).abs() < 1e-12 * solid.area);
        assert!((mesh.weld().unwrap().volume() - solid.volume).abs() < 1e-12 * solid.volume);
    }
    let solid = trimmed::plate_with_round_hole(&Frame::WORLD, 5.0, 4.0, 1.5, 1.2).unwrap();
    let mesh = tessellate_body(&solid.body, &params(1e-2)).unwrap();
    let circle = mesh.face_ids.iter().filter(|&&f| f == 4).count();
    // Face 4 is the hole's wall: two triangles per circle segment.
    let segments = circle / 2;
    for face in [5, 6] {
        let count = mesh.face_ids.iter().filter(|&&f| f == face).count();
        assert_eq!(
            count,
            4 + segments,
            "face {face}: n + 2 (h − 1) with one hole"
        );
    }
}

/// A wall that is straight along one parameter (a cylinder, a cone) is
/// within tolerance as soon as its triangles join boundary points that
/// are neighbours around the axis, along the rulings. Between two equal,
/// parallel ends that takes no interior point at all: one triangle per
/// boundary chord. Where the ends have different numbers of points (a
/// frustum) or are not parallel (an oblique cut) a chord from end to end
/// can be a little too slanted and get a point; those stay few.
#[test]
fn ruled_walls_need_next_to_no_interior_points() {
    let frame = frames(1.0)[1];
    for (solid, exact) in [
        (solids::cylinder(&frame, 1.5, 4.0).unwrap(), true),
        (trimmed::half_cylinder(&frame, 1.5, 3.0).unwrap(), true),
        (
            trimmed::plate_with_round_hole(&frame, 5.0, 4.0, 1.5, 1.2).unwrap(),
            true,
        ),
        (trimmed::cone_frustum(&frame, 2.0, 0.8, 2.5).unwrap(), false),
        (
            trimmed::oblique_cylinder(&frame, 1.5, 1.0, 3.0).unwrap(),
            false,
        ),
    ] {
        for tolerance in [1e-2, 1e-3, 1e-4] {
            let mesh = tessellate_body(&solid.body, &params(tolerance)).unwrap();
            for (face_id, face) in solid.body.faces.iter().enumerate() {
                use craft3d_geom::Surface;
                if !matches!(face.surface, Surface::Cylinder(_) | Surface::Cone(_)) {
                    continue;
                }
                let triangles: Vec<&[u32; 3]> = mesh
                    .triangles
                    .iter()
                    .zip(&mesh.face_ids)
                    .filter(|(_, f)| **f as usize == face_id)
                    .map(|(t, _)| t)
                    .collect();
                let points: std::collections::BTreeSet<[u64; 3]> = triangles
                    .iter()
                    .flat_map(|t| t.iter())
                    .map(|&k| {
                        let p = mesh.positions[k as usize];
                        [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
                    })
                    .collect();
                // With b boundary points and i interior ones a closed band
                // has b + 2 i triangles on b + i points, and a patch two
                // fewer: triangles − points is i, or i − 2.
                let interior = triangles.len().saturating_sub(points.len());
                let allowed = if exact { 0 } else { points.len() / 10 };
                assert!(
                    interior <= allowed,
                    "{} at {tolerance}: face {face_id} has {} triangles on {} points",
                    solid.name,
                    triangles.len(),
                    points.len()
                );
            }
        }
    }
}

/// Every edge is turned into points once: along an edge the two faces
/// that share it have the same vertices, bit for bit.
#[test]
fn faces_share_the_points_of_their_common_edges() {
    let solid = trimmed::oblique_cylinder(&frames(1.0)[1], 1.5, 1.0, 3.0).unwrap();
    let mesh = tessellate_body(&solid.body, &params(1e-3)).unwrap();
    let bits = |p: &Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    let on_face = |face: u32| -> std::collections::BTreeSet<[u64; 3]> {
        mesh.triangles
            .iter()
            .zip(&mesh.face_ids)
            .filter(|(_, f)| **f == face)
            .flat_map(|(t, _)| t.iter().map(|&k| bits(&mesh.positions[k as usize])))
            .collect()
    };
    // Wall (0) and oblique lid (2) share the ellipse and nothing else;
    // the lid has no other vertices.
    let (wall, lid) = (on_face(0), on_face(2));
    assert!(lid.len() > 50);
    assert!(
        lid.is_subset(&wall),
        "the lid has points the wall does not have"
    );
}
