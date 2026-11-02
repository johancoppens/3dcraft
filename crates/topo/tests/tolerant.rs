//! Tolerant entities: the table of tolerances per vertex and edge, the
//! check that holds each entity to its own (edge-on-surface proven), and
//! measuring and recording a gap (`docs/design/topology.md`, Method ›
//! *Tolerant entities*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, NurbsCurve};
use craft3d_math::{Bits, Frame, Portable, Tolerance, Transform, Vec3, bits_of};
use craft3d_testkit::solids;
use craft3d_topo::{
    Body, Defect, EdgeId, FaceId, RECORD_MARGIN, ToleranceError, VertexId, check, check_capped,
    check_remembered, check_remembered_capped, measure_edge, merge, record_edge, record_vertex,
};

fn tolerance(scale: f64) -> Tolerance {
    Tolerance {
        linear: 1e-6 * scale,
        angular: 1e-9,
    }
}

fn block(scale: f64) -> Body {
    solids::block(&Frame::WORLD, 4.0 * scale, 2.0 * scale, 1.0 * scale)
        .unwrap()
        .body
}

/// The edges of face `face`, in loop order.
fn edges_of(body: &Body, face: FaceId) -> Vec<EdgeId> {
    let face = body.face(face).unwrap();
    face.loops
        .iter()
        .flat_map(|l| body.loop_(*l).unwrap().coedges.iter().map(|c| c.edge))
        .collect()
}

/// The face of `body` on a plane whose normal is +z at the top.
fn top_face(body: &Body) -> FaceId {
    let top = body
        .faces
        .iter()
        .enumerate()
        .max_by(|a, b| {
            let z = |f: &craft3d_topo::Face| {
                let e = body
                    .edge(body.loop_(f.loops[0]).unwrap().coedges[0].edge)
                    .unwrap();
                e.curve.eval(e.range.0).z + e.curve.eval(e.range.1).z
            };
            z(a.1).total_cmp(&z(b.1))
        })
        .unwrap()
        .0;
    FaceId(top as u32)
}

/// The body with the surface of `face` moved by `by`: its edges then lie
/// off it by up to the length of `by`, as in a STEP file whose face was
/// written somewhat elsewhere than its edges.
fn face_moved(body: &Body, face: FaceId, by: Vec3) -> Body {
    let mut out = body.clone();
    let f = &mut out.faces[face.0 as usize];
    f.surface = f.surface.transformed(&Transform::translation(by)).unwrap();
    out
}

#[test]
fn an_empty_table_writes_out_as_nothing() {
    let body = block(1.0);
    let mut loose = body.clone();
    loose.set_edge_tolerance(EdgeId(0), 1e-5).unwrap();
    assert_ne!(bits_of(&body), bits_of(&loose));
    loose.clear_edge_tolerance(EdgeId(0));
    for v in 0..loose.vertices.len() {
        loose.clear_vertex_tolerance(VertexId(v as u32));
    }
    assert!(!loose.is_tolerant());
    assert_eq!(bits_of(&body), bits_of(&loose));
    // The table enters the words only when it holds something.
    let mut words = Vec::new();
    body.tolerances.push_bits(&mut words);
    assert!(words.is_empty());
    // And the text of an exact body is what it was (the corpus's digest is
    // of it): no table in it.
    assert!(!format!("{body:?}").contains("tolerances"));
    let mut loose = body.clone();
    loose.set_edge_tolerance(EdgeId(0), 1e-5).unwrap();
    assert!(format!("{loose:?}").contains("tolerances"));
}

#[test]
fn a_face_moved_off_its_edges_passes_with_the_measured_tolerances_and_fails_without() {
    for scale in [1e-3, 1.0, 1e6] {
        let tol = tolerance(scale);
        let cap = 1000.0 * tol.linear;
        for k in [3.0, 30.0, 300.0] {
            let gap = k * tol.linear;
            let body = block(scale);
            let top = top_face(&body);
            let mut moved = face_moved(&body, top, Vec3::new(0.0, 0.0, gap));
            assert!(
                check(&moved, &tol)
                    .iter()
                    .any(|d| matches!(d, Defect::EdgeOffSurface { .. })),
                "scale {scale}, {k} tolerances: the gap is seen"
            );
            for edge in edges_of(&body, top) {
                let recorded = record_edge(&mut moved, edge, &tol, cap).unwrap().unwrap();
                // What was measured is the gap, proven: at least it, and
                // not much more.
                assert!(
                    recorded >= gap && recorded <= RECORD_MARGIN * gap * (1.0 + 1e-6),
                    "scale {scale}, {k}: recorded {recorded:e} for a gap of {gap:e}"
                );
                for v in [
                    moved.edges[edge.0 as usize].start,
                    moved.edges[edge.0 as usize].end,
                ] {
                    assert!(moved.vertex_tolerance(v).unwrap() >= recorded);
                }
            }
            assert_eq!(check(&moved, &tol), Vec::new(), "scale {scale}, {k}");
            // Every other edge is exact still.
            assert_eq!(moved.tolerances.edges().count(), 4);
            // A tenth of what was recorded is not enough.
            let mut tight = moved.clone();
            for (e, t) in moved.tolerances.edges() {
                tight.set_edge_tolerance(e, t / 10.0).unwrap();
            }
            for (v, t) in moved.tolerances.vertices() {
                tight.set_vertex_tolerance(v, t / 10.0).unwrap();
            }
            assert!(!check(&tight, &tol).is_empty(), "scale {scale}, {k}");
            // The table moves with the body.
            let turned = Transform::rotation(Vec3::new(1.0, 2.0, 3.0).normalized().unwrap(), 0.7)
                .unwrap()
                .then(&Transform::translation(Vec3::new(5.0, -3.0, 2.0) * scale));
            let there = moved.transformed(&turned).unwrap();
            assert_eq!(there.tolerances, moved.tolerances);
            assert_eq!(check(&there, &tol), Vec::new(), "scale {scale}, {k} moved");
            // And into a merge, with its ids shifted.
            let other = block(scale)
                .transformed(&Transform::translation(Vec3::new(10.0 * scale, 0.0, 0.0)))
                .unwrap();
            let (merged, offsets) = merge(&[&other, &moved]).unwrap();
            assert_eq!(merged.tolerances.edges().count(), 4);
            for (e, t) in moved.tolerances.edges() {
                assert_eq!(
                    merged.edge_tolerance(EdgeId(e.0 + offsets[1].edges)),
                    Some(t)
                );
            }
            assert_eq!(check(&merged, &tol), Vec::new());
        }
    }
}

#[test]
fn a_nudged_vertex_passes_with_its_measured_tolerance_and_fails_without() {
    for scale in [1e-3, 1.0, 1e6] {
        let tol = tolerance(scale);
        let cap = 1000.0 * tol.linear;
        let mut body = block(scale);
        let nudge = Vec3::new(1.0, -2.0, 2.0) * (10.0 * tol.linear / 3.0);
        body.vertices[0].point += nudge;
        assert!(
            check(&body, &tol)
                .iter()
                .any(|d| matches!(d, Defect::VertexOffEdge { .. }))
        );
        let recorded = record_vertex(&mut body, VertexId(0), &tol, cap)
            .unwrap()
            .unwrap();
        assert!(recorded >= 10.0 * tol.linear * (1.0 - 1e-9));
        assert_eq!(check(&body, &tol), Vec::new(), "scale {scale}");
        body.set_vertex_tolerance(VertexId(0), recorded / 10.0)
            .unwrap();
        assert!(!check(&body, &tol).is_empty());
    }
}

/// A line along the block's bottom front edge, written as a polyline that
/// rises `bump` off the bottom face at an eighth of its length and is back
/// on it at a quarter: the five places the checker samples all lie on the
/// face.
fn bumped(scale: f64, bump: f64) -> (Body, EdgeId) {
    let mut body = block(scale);
    let (index, edge) = body
        .edges
        .iter()
        .enumerate()
        .find(|(_, e)| {
            let (a, b) = (e.curve.eval(e.range.0), e.curve.eval(e.range.1));
            a.y.abs() < 1e-12 && b.y.abs() < 1e-12 && a.z.abs() < 1e-12 && b.z.abs() < 1e-12
        })
        .unwrap();
    let (a, b) = (edge.curve.eval(edge.range.0), edge.curve.eval(edge.range.1));
    let at = |s: f64, z: f64| a + (b - a) * s + Vec3::new(0.0, 0.0, z);
    let points = vec![at(0.0, 0.0), at(0.125, bump), at(0.25, 0.0), at(1.0, 0.0)];
    let curve = NurbsCurve::polynomial(1, vec![0.0, 0.0, 0.125, 0.25, 1.0, 1.0], points).unwrap();
    let id = EdgeId(index as u32);
    body.edges[index].curve = Curve3::Nurbs(curve);
    body.edges[index].range = (0.0, 1.0);
    (body, id)
}

#[test]
fn a_loose_edge_is_proven_on_its_faces_not_sampled() {
    for scale in [1e-3, 1.0, 1e6] {
        let tol = tolerance(scale);
        let bump = 50.0 * tol.linear;
        let (mut body, edge) = bumped(scale, bump);
        // Exact edges keep the five places: this bump is between them.
        assert_eq!(check(&body, &tol), Vec::new(), "scale {scale}");
        // A loose edge is proven over its whole length.
        body.set_edge_tolerance(edge, 10.0 * tol.linear).unwrap();
        let found = check(&body, &tol);
        assert!(
            found
                .iter()
                .any(|d| matches!(d, Defect::EdgeNotProven { edge: e, .. } if *e == edge)),
            "scale {scale}: {found:?}"
        );
        assert!(found.iter().all(|d| !d.to_string().is_empty()));
        // Measured, it is the bump.
        let measured = measure_edge(&body, edge, &tol, 1000.0 * tol.linear)
            .unwrap()
            .unwrap();
        assert!(
            measured >= bump * (1.0 - 1e-9) && measured <= 2.0 * bump,
            "scale {scale}: {measured:e} for {bump:e}"
        );
        record_edge(&mut body, edge, &tol, 1000.0 * tol.linear).unwrap();
        assert_eq!(check(&body, &tol), Vec::new(), "scale {scale}");
    }
}

#[test]
fn the_table_is_held_to_the_cap_and_the_invariant() {
    let tol = tolerance(1.0);
    let mut body = block(1.0);
    body.set_edge_tolerance(EdgeId(0), 2e-3).unwrap();
    let found = check(&body, &tol);
    assert!(
        found
            .iter()
            .any(|d| matches!(d, Defect::ToleranceAboveCap { .. })),
        "{found:?}"
    );
    assert_eq!(check_capped(&body, &tol, 1e-2), Vec::new());
    // A vertex tighter than an edge that ends at it.
    let start = body.edges[0].start;
    body.set_vertex_tolerance(start, 1e-5).unwrap();
    assert!(
        check_capped(&body, &tol, 1e-2)
            .iter()
            .any(|d| matches!(d, Defect::VertexTighterThanEdge { vertex, .. } if *vertex == start))
    );
    // Measuring refuses above the cap.
    let mut moved = face_moved(
        &block(1.0),
        top_face(&block(1.0)),
        Vec3::new(0.0, 0.0, 5e-3),
    );
    let edge = edges_of(&moved, top_face(&moved))[0];
    assert!(record_edge(&mut moved, edge, &tol, 1e-3).is_err());
    assert!(!moved.is_tolerant());
}

#[test]
fn the_remembered_check_holds_the_cap_it_is_given() {
    let tol = tolerance(1.0);
    let mut body = block(1.0);
    body.set_edge_tolerance(EdgeId(0), 5e-4).unwrap();
    // Passes at the default cap and is remembered...
    assert_eq!(check_remembered(&body, &tol), Vec::new());
    // ... which does not let it pass a smaller cap, nor a larger one fail.
    assert!(
        check_remembered_capped(&body, &tol, 1e-4)
            .iter()
            .any(|d| matches!(d, Defect::ToleranceAboveCap { .. }))
    );
    assert_eq!(check_remembered_capped(&body, &tol, 1e-2), Vec::new());
}

#[test]
fn bad_tolerances_and_entities_are_refused_not_panicked_on() {
    let mut body = block(1.0);
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            body.set_edge_tolerance(EdgeId(0), bad),
            Err(ToleranceError::NotPositive { .. })
        ));
        assert!(body.set_vertex_tolerance(VertexId(0), bad).is_err());
    }
    assert!(body.set_edge_tolerance(EdgeId(999), 1e-5).is_err());
    assert!(body.set_vertex_tolerance(VertexId(999), 1e-5).is_err());
    assert!(measure_edge(&body, EdgeId(999), &tolerance(1.0), 1.0).is_err());
    assert!(record_vertex(&mut body, VertexId(999), &tolerance(1.0), 1.0).is_err());
    assert!(!body.is_tolerant());
    // A table that refers to an entity no longer there is a defect.
    body.set_edge_tolerance(EdgeId(11), 1e-5).unwrap();
    body.edges.truncate(11);
    assert!(
        check(&body, &tolerance(1.0))
            .iter()
            .any(|d| matches!(d, Defect::DanglingReference { .. }))
    );
}

/// A gap within the cap is measured within the cap: the first proof, at
/// twice what five samples see, may bound it by anything up to that
/// target, so a gap of 0.6 to 0.95 of the cap came out above it and was
/// refused (ABC models, 2026-11-01); a last proof at the cap holds it.
/// Above the cap it is still refused.
#[test]
fn a_gap_within_the_cap_is_measured_within_it() {
    use craft3d_geom::{Circle, Cylinder, Surface};
    for scale in [1e-3, 1.0, 1e3] {
        let t = tolerance(scale);
        let cap = 1000.0 * t.linear;
        let frame = Frame::new(
            Vec3::new(0.3, -0.2, 0.1) * scale,
            Vec3::new(0.2, 0.3, 1.0),
            Vec3::new(1.0, -0.1, 0.0),
        )
        .unwrap();
        let radius = 10.0 * scale;
        let cylinder = Surface::Cylinder(Cylinder::new(frame, radius).unwrap());
        for share in [0.01, 0.3, 0.6, 0.8, 0.95, 1.2] {
            let gap = share * cap;
            let circle = Curve3::Circle(Circle::new(frame, radius + gap).unwrap());
            let measured = craft3d_topo::measure_curve(&circle, (0.3, 2.9), &cylinder, &t, cap);
            match measured {
                Ok(d) => {
                    assert!(
                        d >= gap * (1.0 - 1e-9),
                        "{scale} {share}: {d:e} below {gap:e}"
                    );
                    if share < 1.0 {
                        assert!(d <= cap, "{scale} {share}: {d:e} above the cap {cap:e}");
                        // Recorded near the gap, not anywhere up to the
                        // first target (twice the gap): review of
                        // 2026-11-02.
                        assert!(
                            d <= 1.25 * gap,
                            "{scale} {share}: {d:e} for a gap of {gap:e}"
                        );
                    } else {
                        assert!(d > cap, "{scale} {share}: {d:e}");
                    }
                }
                // Above the cap the ladder may not try at all.
                Err(_) if share > 1.0 => {}
                Err(at) => panic!("{scale} {share}: not measured (at {at})"),
            }
        }
    }
}

/// A ribbon wound `turns` times round a helix, `scale` across, and the
/// line along its middle (exact: the surface is of degree one across).
fn ribbon(turns: usize, scale: f64) -> (craft3d_geom::Surface, Curve3) {
    let (surface, mut lines) = ribbon_lines(turns, scale, &[10.0]);
    (surface, lines.remove(0))
}

/// [`ribbon`] and its lines at the given radii (9 to 11 across it).
fn ribbon_lines(turns: usize, scale: f64, radii: &[f64]) -> (craft3d_geom::Surface, Vec<Curve3>) {
    use craft3d_geom::{NurbsSurface, Surface};
    let per = 16;
    let n = turns * per + 3;
    let row = |r: f64| -> Vec<Vec3> {
        (0..n)
            .map(|j| {
                let a = j as f64 / per as f64 * std::f64::consts::TAU;
                Vec3::new(r * a.pcos(), r * a.psin(), 0.5 * a) * scale
            })
            .collect()
    };
    let mut knots = vec![0.0; 4];
    knots.extend((1..n - 3).map(|i| i as f64));
    knots.extend([(n - 3) as f64; 4]);
    let mut points = row(9.0);
    points.extend(row(11.0));
    let weights = vec![1.0; points.len()];
    let surface = Surface::Nurbs(
        NurbsSurface::new(
            1,
            3,
            vec![0.0, 0.0, 1.0, 1.0],
            knots.clone(),
            2,
            n,
            points,
            weights,
        )
        .unwrap(),
    );
    let lines = radii
        .iter()
        .map(|r| Curve3::Nurbs(NurbsCurve::polynomial(3, knots.clone(), row(*r)).unwrap()))
        .collect();
    (surface, lines)
}

/// An edge on a spline surface of many spans whose own projection lands
/// in another dip of the distance (a ribbon wound round a helix: a point
/// on one turn projects onto the next, three units away) is measured
/// from feet walked along it, not left unmeasured (2026-11-01: one far
/// foot among the five made the first target larger than the cap, and
/// the edge was never tried; ABC models on rational surfaces).
#[test]
fn an_edge_whose_projection_lands_on_another_turn_is_measured() {
    for scale in [1.0, 1e3, 1e-3] {
        let t = tolerance(scale);
        let cap = 1000.0 * t.linear;
        let (surface, middle) = ribbon(16, scale);
        // At one of the five places the measure starts from (its end)
        // the projection is three units off.
        let (lo, hi) = middle.domain();
        let range = (lo + 0.37, hi - 0.61);
        let wrong = (0..=4).any(|k| {
            let p = middle.eval(range.0 + (range.1 - range.0) * f64::from(k) / 4.0);
            let (u, v) = surface.project(p).unwrap();
            surface.eval(u, v).distance(p) > scale
        });
        assert!(
            wrong,
            "{scale}: the probe needs a projection that goes wrong"
        );
        let measured = craft3d_topo::measure_curve(&middle, range, &surface, &t, cap);
        assert!(
            measured.is_ok_and(|d| d <= t.linear),
            "{scale}: {measured:?}"
        );
    }
}

/// A short edge whose every place projects onto another turn of the
/// ribbon (all five feet wrong, so walking from them stays wrong) is
/// measured from feet found by the surface's spans
/// (`topo::nearest_foot`): ABC models on rational surfaces of 96 spans,
/// 2026-11-01.
#[test]
fn a_short_edge_whose_every_projection_is_wrong_is_measured() {
    for scale in [1.0, 1e3, 1e-3] {
        let t = tolerance(scale);
        let cap = 1000.0 * t.linear;
        let (surface, middle) = ribbon(16, scale);
        let (lo, hi) = middle.domain();
        let wrong = |c: f64| {
            let p = middle.eval(c);
            let (u, v) = surface.project(p).unwrap();
            surface.eval(u, v).distance(p) > scale
        };
        // A piece a fiftieth of a span long all of whose places project
        // wrong.
        let centre = (0..2000)
            .map(|k| lo + (hi - lo) * (f64::from(k) + 0.5) / 2000.0)
            .find(|&c| (0..=4).all(|k| wrong(c - 0.01 + 0.005 * f64::from(k))))
            .expect("a piece whose projections all go wrong");
        let range = (centre - 0.01, centre + 0.01);
        let foot = craft3d_topo::nearest_foot(&surface, middle.eval(centre)).unwrap();
        assert!(surface.eval(foot.0, foot.1).distance(middle.eval(centre)) <= t.linear);
        let measured = craft3d_topo::measure_curve(&middle, range, &surface, &t, cap);
        assert!(
            measured.is_ok_and(|d| d <= t.linear),
            "{scale}: {measured:?}"
        );
    }
}

/// A gap is recorded near its size, not anywhere up to the first
/// proof's target (twice the gap five samples see): a cubic that rises
/// and falls off a plane between its control points, at gaps of a
/// hundredth to a third of the cap, is measured within a quarter above
/// its largest rise (review of 2026-11-02: tighter bounds per piece let
/// proofs stop earlier, and a recorded tolerance rose towards twice the
/// gap, which an ABC model's boolean then refused).
#[test]
fn a_gap_is_recorded_near_its_size() {
    use craft3d_geom::{Plane, Surface};
    for scale in [1e-3, 1.0, 1e3] {
        let t = tolerance(scale);
        let cap = 1000.0 * t.linear;
        let plane = Surface::Plane(Plane::new(Frame::WORLD));
        for share in [0.0003, 0.0008, 0.01, 0.1, 0.3] {
            let lift = share * cap;
            let points: Vec<Vec3> = (0..9)
                .map(|k| {
                    let z = if k % 2 == 1 { 3.0 * lift } else { 0.0 };
                    Vec3::new(f64::from(k) * 10.0 * scale, 0.0, z)
                })
                .collect();
            let knots = vec![
                0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 6.0, 6.0, 6.0,
            ];
            let curve = Curve3::Nurbs(NurbsCurve::polynomial(3, knots, points).unwrap());
            let gap = (0..=6000)
                .map(|k| curve.eval(6.0 * f64::from(k) / 6000.0).z.abs())
                .fold(0.0, f64::max);
            let d = craft3d_topo::measure_curve(&curve, (0.0, 6.0), &plane, &t, cap).unwrap();
            assert!(
                d >= gap * (1.0 - 1e-9),
                "{scale} {share}: {d:e} below {gap:e}"
            );
            if gap * 1.1 <= t.linear {
                // A gap within the document's tolerance is measured
                // within it: no loose edge where there is no gap to hold
                // (review of 2026-11-02).
                assert!(d <= t.linear, "{scale} {share}: {d:e} above {:e}", t.linear);
            } else {
                assert!(
                    d <= 1.25 * gap,
                    "{scale} {share}: {d:e} for a gap of {gap:e}"
                );
            }
        }
    }
}
