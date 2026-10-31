//! Oracle 4 (docs/testing.md) for B-rep bodies: the mesh of a solid is a
//! closed, outward-oriented manifold within the chordal tolerance of the
//! exact body, and its volume and area converge to the closed forms.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Surface, Torus};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessError, TessParams, tessellate_body};
use craft3d_testkit::solids::{self, Reference};
use craft3d_topo::{Body, BodyKind, Coedge};

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

/// The solids the tessellator supports: all of them, the hemisphere (a
/// trimmed sphere face) included.
fn supported(frame: &Frame, scale: f64) -> Vec<Reference> {
    solids::all(frame, scale).unwrap()
}

#[test]
fn meshes_of_solids_are_closed_outward_and_within_tolerance() {
    for scale in [1e-3, 1.0, 1e6] {
        for frame in frames(scale) {
            for solid in supported(&frame, scale) {
                let tolerance = 1e-3 * scale;
                let mesh = tessellate_body(&solid.body, &params(tolerance)).unwrap();
                let what = format!("{} at scale {scale}", solid.name);
                assert_eq!(mesh.normals.len(), mesh.positions.len(), "{what}");
                assert_eq!(mesh.face_ids.len(), mesh.triangles.len(), "{what}");
                assert!(
                    mesh.face_ids
                        .iter()
                        .all(|&f| (f as usize) < solid.body.faces.len()),
                    "{what}"
                );

                let welded = mesh.weld().unwrap();
                assert!(
                    welded.is_closed(),
                    "{what}: the welded mesh is not a closed manifold"
                );

                // Every vertex is on its face's surface; every triangle's
                // centroid is within the tolerance of it; every triangle
                // faces the way the face does.
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
                        assert!(
                            distance(p).0 <= 1e-9 * scale,
                            "{what}: vertex off its surface"
                        );
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

                // The mesh lies within `tolerance` of the surface, so its
                // volume differs by at most area × tolerance.
                let volume = welded.volume();
                assert!(
                    (volume - solid.volume).abs() <= solid.area * tolerance,
                    "{what}: volume {volume} vs {}",
                    solid.volume
                );
                assert!(
                    volume > 0.0 && mesh.area() <= solid.area * (1.0 + 1e-12),
                    "{what}"
                );
            }
        }
    }
}

#[test]
fn volume_and_area_converge_to_the_closed_forms() {
    for solid in supported(&Frame::WORLD, 1.0) {
        let mut previous = (f64::INFINITY, f64::INFINITY);
        for tolerance in [1e-1, 1e-2, 1e-3, 1e-4] {
            let mesh = tessellate_body(&solid.body, &params(tolerance)).unwrap();
            let volume_error = (mesh.weld().unwrap().volume() - solid.volume).abs() / solid.volume;
            let area_error = (mesh.area() - solid.area).abs() / solid.area;
            println!(
                "{:9} tol {tolerance:e}: {:7} triangles, volume error {volume_error:.2e}, area error {area_error:.2e}",
                solid.name,
                mesh.triangles.len()
            );
            assert!(
                volume_error <= previous.0 && area_error <= previous.1,
                "{}",
                solid.name
            );
            previous = (volume_error, area_error);
        }
        assert!(
            previous.0 < 1e-3 && previous.1 < 1e-3,
            "{}: {previous:?}",
            solid.name
        );
    }
}

#[test]
fn a_block_is_exact_at_any_tolerance() {
    let solid = solids::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap();
    for tolerance in [10.0, 1e-9] {
        let mesh = tessellate_body(&solid.body, &params(tolerance)).unwrap();
        assert_eq!(mesh.triangles.len(), 12);
        let welded = mesh.weld().unwrap();
        assert_eq!(welded.positions.len(), 8);
        assert!((welded.volume() - 8.0).abs() < 1e-12 && (mesh.area() - 28.0).abs() < 1e-12);
        // Planar faces carry their face normal on every vertex.
        for (t, &face) in mesh.face_ids.iter().enumerate() {
            let [a, b, c] = mesh.corners(t).unwrap();
            let geometric = (b - a).cross(c - a).normalized().unwrap();
            for k in mesh.triangles[t] {
                assert!(
                    mesh.normals[k as usize].distance(geometric) < 1e-12,
                    "face {face}"
                );
            }
        }
    }
}

#[test]
fn output_is_reproducible() {
    for solid in supported(&Frame::WORLD, 1.0) {
        let first = tessellate_body(&solid.body, &params(1e-3)).unwrap();
        assert_eq!(
            first,
            tessellate_body(&solid.body, &params(1e-3)).unwrap(),
            "{}",
            solid.name
        );
    }
}

#[test]
fn what_cannot_be_tessellated_yet_is_an_error_that_says_so() {
    // A loop that winds around both the axis and the tube of a torus: a
    // Villarceau circle, cut from the torus by a plane through its centre
    // that touches it twice (tilted by asin(r / R); the circle has radius
    // R and its centre is r off the axis).
    let (major, minor) = (3.0_f64, 1.0_f64);
    let tilt = (minor / major).pasin();
    let plane = Frame::new(
        Vec3::new(0.0, minor, 0.0),
        Vec3::new(-tilt.psin(), 0.0, tilt.pcos()),
        Vec3::new(tilt.pcos(), 0.0, tilt.psin()),
    )
    .unwrap();
    let circle = Curve3::Circle(Circle::new(plane, major).unwrap());
    let torus = Surface::Torus(Torus::new(Frame::WORLD, major, minor).unwrap());
    for k in 0..16 {
        // The circle does lie on the torus.
        let p = circle.eval(std::f64::consts::TAU * f64::from(k) / 16.0);
        let (u, v) = torus.project(p).unwrap();
        assert!(torus.eval(u, v).distance(p) < 1e-12);
    }
    let mut sheet = Body::new(BodyKind::Sheet);
    let vertex = sheet.add_vertex(circle.eval(0.0));
    let edge = sheet.add_edge(circle, (0.0, std::f64::consts::TAU), vertex, vertex);
    let loop_ = sheet.add_loop(vec![Coedge {
        edge,
        reversed: false,
    }]);
    let face = sheet.add_face(torus, false, vec![loop_]);
    sheet.add_shell(vec![face]);
    let error = tessellate_body(&sheet, &params(1e-2)).unwrap_err();
    assert!(
        matches!(error, TessError::Unsupported { face: 0, .. }),
        "{error}"
    );
    assert!(error.to_string().contains("torus"), "{error}");

    let cylinder = solids::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap().body;
    for tolerance in [0.0, -1.0, f64::NAN] {
        assert!(matches!(
            tessellate_body(&cylinder, &params(tolerance)),
            Err(TessError::InvalidTolerance(_))
        ));
    }
    let too_fine = TessParams {
        chordal_tolerance: 1e-14,
        max_segments: 8,
        ..TessParams::default()
    };
    assert!(matches!(
        tessellate_body(&cylinder, &too_fine),
        Err(TessError::TooFine { .. })
    ));

    // Broken references are errors, not panics.
    let mut broken = cylinder.clone();
    broken.edges[0].start = craft3d_topo::VertexId(99);
    assert!(matches!(
        tessellate_body(&broken, &params(1e-2)),
        Err(TessError::InvalidBody(_))
    ));
    let mut broken = cylinder.clone();
    broken.faces[1].loops[0] = craft3d_topo::LoopId(99);
    assert!(matches!(
        tessellate_body(&broken, &params(1e-2)),
        Err(TessError::InvalidBody(_))
    ));
    let mut broken = cylinder;
    broken.loops[0].coedges[0].edge = craft3d_topo::EdgeId(99);
    assert!(matches!(
        tessellate_body(&broken, &params(1e-2)),
        Err(TessError::InvalidBody(_))
    ));
}
