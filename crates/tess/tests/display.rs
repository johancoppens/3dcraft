//! Meshes for display: the angle criterion (`TessParams::max_angle`) and a
//! body kept within a triangle budget by coarsening
//! (`tessellate_within_budget`).
//!
//! Oracles: the mesh of a solid is closed (welded), its volume is within
//! what the chordal tolerance allows of the B-rep's (the mesh lies within
//! the tolerance of the surface, so the volume between them is at most
//! the tolerance times the area), the angle holds on every chord and every
//! triangle not below its floor, the same mesh comes back on any number
//! of threads and at a power-of-two scale, and without the criterion the
//! mesh is the one made before it existed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{
    ANGLE_FLOOR, Mesh, TessError, TessParams, tessellate_body, tessellate_body_and_edges,
    tessellate_body_with_threads, tessellate_edges, tessellate_within_budget,
};
use craft3d_testkit::solids::Reference;
use craft3d_testkit::{solids, trimmed};
use std::f64::consts::PI;

fn params(tolerance: f64, max_angle: Option<f64>, max_triangles: Option<usize>) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        max_angle,
        max_triangles,
        ..TessParams::default()
    }
}

/// Analytic and freeform bodies, with one and with many faces.
fn fixtures(scale: f64) -> Vec<Reference> {
    let mut all = solids::all(&Frame::WORLD, scale).unwrap();
    all.extend(trimmed::all(&Frame::WORLD, scale).unwrap());
    all.push(trimmed::pipe_elbow(&Frame::WORLD, 3.0 * scale, 1.0 * scale, 1.9).unwrap());
    all
}

fn turn(a: Vec3, b: Vec3) -> f64 {
    a.cross(b).length().patan2(a.dot(b))
}

fn closed(mesh: &Mesh) -> bool {
    mesh.weld().is_ok_and(|welded| welded.is_closed())
}

fn bits(mesh: &Mesh) -> Vec<u64> {
    mesh.positions
        .iter()
        .flat_map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
        .collect()
}

const DEGREES: f64 = PI / 180.0;

/// A tolerance of a tenth of the size makes a circle a triangle; with the
/// angle it is a polygon whose chords turn by no more than the angle.
#[test]
fn the_angle_holds_on_every_chord_of_every_edge() {
    let angle = 15.0 * DEGREES;
    let mut held = 0;
    for solid in fixtures(1.0) {
        for tolerance in [0.1, 0.02] {
            let coarse = tessellate_edges(&solid.body, &params(tolerance, None, None)).unwrap();
            let edges =
                tessellate_edges(&solid.body, &params(tolerance, Some(angle), None)).unwrap();
            for (index, (points, without)) in edges.iter().zip(&coarse).enumerate() {
                assert!(
                    points.len() >= without.len(),
                    "{}: edge {index}",
                    solid.name
                );
                let curve = &solid.body.edges[index].curve;
                for pair in points.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    if a.distance(b) < ANGLE_FLOOR * tolerance {
                        continue;
                    }
                    let (ta, tb) = (curve.project(a).unwrap(), curve.project(b).unwrap());
                    let (da, db) = (curve.derivatives(ta).1, curve.derivatives(tb).1);
                    // The chord's own turn, from its two ends.
                    let t = turn(da, db);
                    assert!(
                        t <= angle * (1.0 + 1e-6),
                        "{} at {tolerance}: edge {index} turns {:.2} degrees over a chord",
                        solid.name,
                        t / DEGREES
                    );
                    held += 1;
                }
            }
        }
    }
    assert!(held > 500, "{held}");
}

/// Every triangle of a curved face whose sides are all above the floor has
/// corner normals within the angle of each other; the mesh is closed and
/// its volume within the tolerance times the area of the B-rep's.
#[test]
fn the_angle_holds_on_the_triangles_and_the_mesh_stays_closed_and_true() {
    for angle in [10.0 * DEGREES, 25.0 * DEGREES] {
        for solid in fixtures(1.0) {
            for tolerance in [0.05, 0.005] {
                let without = tessellate_body(&solid.body, &params(tolerance, None, None)).unwrap();
                let name = format!(
                    "{} at {tolerance}, {:.0} degrees",
                    solid.name,
                    angle / DEGREES
                );
                let mesh = tessellate_body(&solid.body, &params(tolerance, Some(angle), None))
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                assert!(closed(&mesh), "{name}: not closed");
                assert!(
                    mesh.triangles.len() >= without.triangles.len(),
                    "{name}: coarser with the angle"
                );
                let welded = mesh.weld().unwrap();
                let off = (welded.volume() - solid.volume).abs();
                assert!(
                    off <= tolerance * solid.area,
                    "{name}: volume {} against {}",
                    welded.volume(),
                    solid.volume
                );
                // The area of a mesh within the tolerance of the surface
                // and within the angle of its normals.
                let area_off = (mesh.area() - solid.area).abs() / solid.area;
                assert!(area_off < 0.05, "{name}: area off by {area_off}");
                let floor = ANGLE_FLOOR * tolerance;
                let (mut checked, mut beyond) = (0, 0);
                for t in 0..mesh.triangles.len() {
                    let [a, b, c] = mesh.corners(t).unwrap();
                    if a.distance(b).min(b.distance(c)).min(c.distance(a)) < floor {
                        continue;
                    }
                    let n = mesh.triangles[t].map(|k| mesh.normals[k as usize]);
                    if n.iter().any(|n| n.length() == 0.0) {
                        continue;
                    }
                    let worst = turn(n[0], n[1]).max(turn(n[1], n[2])).max(turn(n[2], n[0]));
                    checked += 1;
                    if worst > angle * (1.0 + 1e-9) {
                        beyond += 1;
                    }
                }
                // A side on an edge is the edge's chord, which a face
                // cannot cut: where the surface's normal twists along an
                // edge more than its tangent turns, a triangle on it may
                // keep a wider turn. None on these bodies.
                assert_eq!(beyond, 0, "{name}: {beyond} of {checked} triangles");
            }
        }
    }
}

/// A small hole in a large plate: at a tolerance that makes it a
/// triangle, the angle keeps it round.
#[test]
fn a_small_hole_stays_round() {
    let plate = trimmed::plate_with_round_hole(&Frame::WORLD, 100.0, 100.0, 1.0, 5.0).unwrap();
    let tolerance = 0.5;
    let chords = |angle: Option<f64>| -> usize {
        let edges = tessellate_edges(&plate.body, &params(tolerance, angle, None)).unwrap();
        // The chords around the hole (top and bottom), in however many
        // edges each circle is.
        edges
            .iter()
            .zip(&plate.body.edges)
            .filter(|(_, edge)| matches!(edge.curve, craft3d_geom::Curve3::Circle(_)))
            .map(|(points, _)| points.len() - 1)
            .sum()
    };
    // Seven chords of 51 degrees each without the angle; 36 of 10 with it.
    assert_eq!(chords(None), 2 * 7);
    assert_eq!(chords(Some(10.0 * DEGREES)), 2 * 36);
    let mesh =
        tessellate_body(&plate.body, &params(tolerance, Some(10.0 * DEGREES), None)).unwrap();
    assert!(closed(&mesh));
}

#[test]
fn the_angle_gives_the_same_mesh_on_any_number_of_threads_and_at_twice_the_size() {
    let angle = 12.0 * DEGREES;
    let small = fixtures(1.0);
    let large = fixtures(2.0);
    for (solid, twice) in small.iter().zip(&large) {
        let p = params(0.01, Some(angle), None);
        let one = tessellate_body_with_threads(&solid.body, &p, 1).unwrap();
        for threads in [2, 8] {
            let again = tessellate_body_with_threads(&solid.body, &p, threads).unwrap();
            assert!(again == one, "{}: {threads} threads", solid.name);
        }
        let big = tessellate_body(&twice.body, &params(0.02, Some(angle), None)).unwrap();
        assert_eq!(big.triangles, one.triangles, "{}", solid.name);
        let doubled: Vec<u64> = one
            .positions
            .iter()
            .flat_map(|p| {
                [
                    (p.x * 2.0).to_bits(),
                    (p.y * 2.0).to_bits(),
                    (p.z * 2.0).to_bits(),
                ]
            })
            .collect();
        assert_eq!(bits(&big), doubled, "{}", solid.name);
    }
}

#[test]
fn an_angle_that_is_no_angle_is_refused() {
    let solid = solids::sphere(&Frame::WORLD, 1.0).unwrap();
    for angle in [0.0, -1.0, f64::NAN, f64::INFINITY, 7.0] {
        let error = tessellate_body(&solid.body, &params(0.01, Some(angle), None)).unwrap_err();
        assert!(matches!(error, TessError::InvalidAngle(_)), "{error:?}");
        assert!(
            tessellate_within_budget(&solid.body, &params(0.01, Some(angle), Some(10))).is_err()
        );
    }
    // A whole turn is an angle, and asks for nothing beyond the tolerance.
    let free = tessellate_body(&solid.body, &params(0.01, None, None)).unwrap();
    let turn = tessellate_body(&solid.body, &params(0.01, Some(2.0 * PI), None)).unwrap();
    assert!(turn == free);
}

/// One face that alone is over the budget stops at a floor of its count,
/// not after it was made in full.
#[test]
fn one_large_face_stops_as_soon_as_it_is_over_the_budget() {
    let sphere = solids::sphere(&Frame::WORLD, 1.0).unwrap();
    let p = params(1e-5, None, None);
    let full = tessellate_body(&sphere.body, &p).unwrap().triangles.len();
    assert!(full > 100_000, "{full}");
    for budget in [0, 1000, full / 4, full / 2] {
        for threads in [1, 8] {
            let error = tessellate_body_with_threads(
                &sphere.body,
                &params(1e-5, None, Some(budget)),
                threads,
            )
            .unwrap_err();
            let TessError::OverBudget {
                triangles, faces, ..
            } = error
            else {
                panic!("{error:?}");
            };
            assert_eq!(faces, 1);
            assert!(triangles > budget, "{triangles} for {budget}");
            // Stopped near the budget: a few triangles over it, far below
            // the whole face.
            assert!(
                triangles <= budget + budget / 10 + 16 && triangles < full,
                "{triangles} for {budget} of {full}"
            );
        }
    }
}

/// Within the budget: the mesh at the tolerance asked for, the very one
/// made without a budget. Over it: a coarser tolerance, closed, within
/// the budget, the same on every call, and true to the volume within the
/// tolerance it settled on.
#[test]
fn a_body_is_coarsened_to_its_budget() {
    let mut coarsened = 0;
    for solid in fixtures(1.0) {
        for angle in [None, Some(15.0 * DEGREES)] {
            let tolerance = 1e-4;
            let free =
                tessellate_body_and_edges(&solid.body, &params(tolerance, angle, None)).unwrap();
            let free_mesh = free.mesh.unwrap();
            let count = free_mesh.triangles.len();
            let within =
                tessellate_within_budget(&solid.body, &params(tolerance, angle, Some(count)))
                    .unwrap();
            assert!(within.mesh == free_mesh, "{}", solid.name);
            assert_eq!(within.edges, free.edges, "{}", solid.name);
            assert_eq!(within.tolerance, tolerance);
            assert_eq!(within.attempts, 1);
            for budget in [count / 3, count / 20, 500] {
                if budget >= count || budget < solid.body.faces.len() * 2 {
                    continue;
                }
                let p = params(tolerance, angle, Some(budget));
                let result = tessellate_within_budget(&solid.body, &p).unwrap();
                let name = format!("{} with a budget of {budget}", solid.name);
                assert!(result.mesh.triangles.len() <= budget, "{name}");
                assert!(result.tolerance > tolerance, "{name}");
                assert!(closed(&result.mesh), "{name}: not closed");
                // The mesh at the tolerance it settled on, as made without
                // a budget.
                let plain =
                    tessellate_body(&solid.body, &params(result.tolerance, angle, None)).unwrap();
                assert!(plain == result.mesh, "{name}");
                let welded = result.mesh.weld().unwrap();
                assert!(
                    (welded.volume() - solid.volume).abs() <= result.tolerance * solid.area,
                    "{name}: volume {} against {} at {}",
                    welded.volume(),
                    solid.volume,
                    result.tolerance
                );
                for threads_again in 0..2 {
                    let again = tessellate_within_budget(&solid.body, &p).unwrap();
                    assert!(again == result, "{name}: run {threads_again}");
                }
                coarsened += 1;
            }
        }
    }
    assert!(coarsened >= 40, "{coarsened}");
}

/// A budget smaller than the body can be at any tolerance is refused, not
/// a hang.
#[test]
fn a_budget_no_tolerance_can_meet_is_refused() {
    for solid in fixtures(1.0) {
        let error =
            tessellate_within_budget(&solid.body, &params(1e-3, None, Some(1))).unwrap_err();
        assert!(
            matches!(error, TessError::OverBudget { .. }),
            "{}: {error:?}",
            solid.name
        );
    }
}
