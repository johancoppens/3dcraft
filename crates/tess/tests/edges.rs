//! The edges of a body as polylines (`tessellate_edges`), for a viewer:
//! they follow the edge's curve within the tolerance, end on its vertices,
//! and are made of the very points the mesh has along that edge.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Curve3;
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessError, TessParams, tessellate_body, tessellate_edges};
use craft3d_testkit::solids::{self, Reference};
use craft3d_testkit::trimmed;
use std::collections::BTreeSet;
use std::f64::consts::TAU;

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

/// Analytic solids and trimmed ones: lines, circles, ellipses and NURBS
/// curves as edges.
fn every_solid(frame: &Frame, scale: f64) -> Vec<Reference> {
    let mut all = solids::all(frame, scale).unwrap();
    all.extend(trimmed::all(frame, scale).unwrap());
    all
}

fn bits(p: Vec3) -> [u64; 3] {
    [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
}

fn length(points: &[Vec3]) -> f64 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

#[test]
fn every_edge_is_a_polyline_between_its_vertices_within_tolerance_of_its_curve() {
    let mut kinds = BTreeSet::new();
    for scale in [1e-3, 1.0, 1e6] {
        for frame in frames(scale) {
            for solid in every_solid(&frame, scale) {
                let tolerance = 1e-3 * scale;
                let edges = tessellate_edges(&solid.body, &params(tolerance)).unwrap();
                let what = format!("{} at scale {scale}", solid.name);
                assert_eq!(edges.len(), solid.body.edges.len(), "{what}");
                for (edge, points) in solid.body.edges.iter().zip(&edges) {
                    kinds.insert(edge.curve.kind());
                    assert!(points.len() >= 2, "{what}");
                    // From the start vertex to the end vertex, exactly.
                    let start = solid.body.vertex(edge.start).unwrap().point;
                    let end = solid.body.vertex(edge.end).unwrap().point;
                    assert_eq!(bits(points[0]), bits(start), "{what}");
                    assert_eq!(bits(*points.last().unwrap()), bits(end), "{what}");
                    // Every point is on the curve, within the edge's range.
                    let (t0, t1) = edge.range;
                    let slack = 1e-9 * (t1 - t0).abs().max(1.0);
                    for p in points {
                        let t = edge.curve.project(*p).unwrap();
                        let on = edge.curve.eval(t);
                        assert!(
                            on.distance(*p) <= 1e-9 * scale,
                            "{what}: a point is {:e} off its curve",
                            on.distance(*p)
                        );
                        // A closed curve's parameter wraps; only an open
                        // one pins the range.
                        if edge.curve.period().is_none() {
                            assert!(t >= t0 - slack && t <= t1 + slack, "{what}: t = {t}");
                        }
                    }
                    // No chord strays further from the curve than the
                    // tolerance: checked at each chord's middle and
                    // quarter points.
                    for pair in points.windows(2) {
                        for s in [0.25, 0.5, 0.75] {
                            let on_chord = pair[0].lerp(pair[1], s);
                            let t = edge.curve.project(on_chord).unwrap();
                            let off = edge.curve.eval(t).distance(on_chord);
                            assert!(
                                off <= tolerance * (1.0 + 1e-6),
                                "{what}: a chord is {off:e} from its curve"
                            );
                        }
                    }
                }
            }
        }
    }
    // The test met straight and curved edges, analytic and freeform.
    for kind in ["line", "circle", "ellipse", "nurbs curve"] {
        assert!(kinds.contains(kind), "no {kind} edge was tested: {kinds:?}");
    }
}

#[test]
fn the_edge_points_are_the_ones_the_mesh_has_along_the_edge() {
    for scale in [1e-3, 1.0, 1e6] {
        for solid in every_solid(&frames(scale)[1], scale) {
            let p = params(2e-3 * scale);
            let edges = tessellate_edges(&solid.body, &p).unwrap();
            let Ok(mesh) = tessellate_body(&solid.body, &p) else {
                // What cannot be meshed yet is covered elsewhere.
                continue;
            };
            let vertices: BTreeSet<[u64; 3]> = mesh.positions.iter().map(|p| bits(*p)).collect();
            for (index, points) in edges.iter().enumerate() {
                for point in points {
                    assert!(
                        vertices.contains(&bits(*point)),
                        "{} at scale {scale}: a point of edge {index} is not a mesh vertex",
                        solid.name
                    );
                }
            }
        }
    }
}

#[test]
fn a_block_has_twelve_straight_edges_and_a_cylinder_two_circles() {
    let block = solids::block(&frames(1.0)[1], 3.0, 4.0, 5.0).unwrap();
    for tolerance in [1.0, 1e-3, 1e-6] {
        let edges = tessellate_edges(&block.body, &params(tolerance)).unwrap();
        assert_eq!(edges.len(), 12);
        assert!(edges.iter().all(|e| e.len() == 2));
        let total: f64 = edges.iter().map(|e| length(e)).sum();
        assert!((total - 4.0 * (3.0 + 4.0 + 5.0)).abs() < 1e-12, "{total}");
    }

    let (radius, height) = (2.5, 7.0);
    let cylinder = solids::cylinder(&frames(1.0)[1], radius, height).unwrap();
    let mut previous = 0.0;
    for tolerance in [1e-1, 1e-2, 1e-3, 1e-4] {
        let edges = tessellate_edges(&cylinder.body, &params(tolerance)).unwrap();
        assert_eq!(edges.len(), 2);
        for circle in &edges {
            // Closed: it ends where it starts.
            assert_eq!(bits(circle[0]), bits(*circle.last().unwrap()));
            // An inscribed polygon: shorter than the circle, by no more
            // than a polygon with this sag can be, and longer as the
            // tolerance shrinks.
            let (exact, got) = (TAU * radius, length(circle));
            let segments = (circle.len() - 1) as f64;
            let inscribed = 2.0 * segments * radius * (TAU / (2.0 * segments)).sin();
            assert!((got - inscribed).abs() < 1e-9, "{got} vs {inscribed}");
            assert!(got < exact && got > exact * (1.0 - 2.0 * tolerance / radius));
            assert!(got >= previous - 1e-9);
        }
        previous = length(&edges[0]);
    }
}

#[test]
fn the_edges_are_reproducible_and_bad_input_is_an_error() {
    let scale = 1.0;
    for solid in every_solid(&frames(scale)[1], scale) {
        let p = params(1e-3);
        let first = tessellate_edges(&solid.body, &p).unwrap();
        let second = tessellate_edges(&solid.body, &p).unwrap();
        let flat = |edges: &[Vec<Vec3>]| -> Vec<Vec<[u64; 3]>> {
            edges
                .iter()
                .map(|e| e.iter().map(|p| bits(*p)).collect())
                .collect()
        };
        assert_eq!(flat(&first), flat(&second), "{}", solid.name);
    }

    let mut cylinder = solids::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap().body;
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            tessellate_edges(&cylinder, &params(bad)),
            Err(TessError::InvalidTolerance(_))
        ));
    }
    // A tolerance finer than the segment limit allows is refused, with
    // the edge it was refused for.
    let too_fine = TessParams {
        chordal_tolerance: 1e-15,
        max_segments: 16,
    };
    assert!(matches!(
        tessellate_edges(&cylinder, &too_fine),
        Err(TessError::TooFine { patch: 0, .. })
    ));
    // An edge that names a vertex the body does not have, or whose range
    // is not a range, is an invalid body, not a panic.
    let mut broken = cylinder.clone();
    broken.vertices.clear();
    assert!(matches!(
        tessellate_edges(&broken, &params(1e-3)),
        Err(TessError::InvalidBody(_))
    ));
    cylinder.edges[1].range = (1.0, f64::NAN);
    assert!(matches!(
        tessellate_edges(&cylinder, &params(1e-3)),
        Err(TessError::InvalidBody(_))
    ));
    // A curve the tests name, so an added kind is noticed here.
    assert!(matches!(cylinder.edges[0].curve, Curve3::Circle(_)));
}
