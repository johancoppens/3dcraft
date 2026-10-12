//! The checker detects each kind of defect when it is seeded into an
//! otherwise valid body (DoD of milestone M1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Plane, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_testkit::solids;
use craft3d_topo::{
    Body, BodyKind, Coedge, Defect, EdgeId, FaceId, LoopId, VertexId, check, genus,
};

fn block() -> Body {
    solids::block(&Frame::WORLD, 4.0, 2.0, 1.0).unwrap().body
}

fn cylinder() -> Body {
    solids::cylinder(&Frame::WORLD, 1.5, 4.0).unwrap().body
}

fn defects(body: &Body) -> Vec<Defect> {
    check(body, &Tolerance::default())
}

/// True when some defect of `body` matches.
fn has(body: &Body, matches: impl Fn(&Defect) -> bool) -> bool {
    let found = defects(body);
    assert!(
        !found.is_empty(),
        "expected a defect, the body checked clean"
    );
    // Every defect can say what is wrong.
    assert!(found.iter().all(|d| !d.to_string().is_empty()));
    found.iter().any(matches)
}

#[test]
fn the_unmodified_bodies_are_clean() {
    assert_eq!(defects(&block()), vec![]);
    assert_eq!(defects(&cylinder()), vec![]);
    assert_eq!(defects(&Body::new(BodyKind::Sheet)), vec![]);
}

#[test]
fn a_moved_vertex_is_off_its_edges_and_faces() {
    let mut body = block();
    body.vertices[3].point += Vec3::new(0.0, 0.0, 1e-3);
    assert!(has(
        &body,
        |d| matches!(d, Defect::VertexOffEdge { vertex: VertexId(3), distance, .. } if (*distance - 1e-3).abs() < 1e-12)
    ));
    // Within tolerance it is fine.
    let mut body = block();
    body.vertices[3].point += Vec3::new(0.0, 0.0, 1e-7);
    assert_eq!(defects(&body), vec![]);
}

#[test]
fn an_edge_that_leaves_its_surface_is_found() {
    let mut body = block();
    // Tilt the bottom face's plane: its boundary no longer lies in it.
    let tilted = Frame::new(Vec3::ZERO, Vec3::new(0.01, 0.0, -1.0), Vec3::Y).unwrap();
    body.faces[0].surface = Surface::Plane(Plane::new(tilted));
    assert!(has(
        &body,
        |d| matches!(d, Defect::EdgeOffSurface { face: FaceId(0), distance, .. } if *distance > 1e-3)
    ));
}

#[test]
fn a_missing_face_leaves_edges_used_once() {
    let mut body = block();
    body.shells[0].faces.pop();
    let removed = body.faces.pop().unwrap();
    assert!(has(&body, |d| matches!(
        d,
        Defect::EdgeUses { uses: 1, .. }
    )));
    // Its loop is now orphaned, and Euler no longer adds up.
    assert!(has(
        &body,
        |d| matches!(d, Defect::LoopUses { loop_, uses: 0 } if *loop_ == removed.loops[0])
    ));
    assert!(has(&body, |d| matches!(d, Defect::Euler { faces: 5, .. })));
    assert_eq!(genus(&body), None);
    // As a sheet, an open box is legitimate apart from the orphaned loop.
    body.kind = BodyKind::Sheet;
    assert!(
        defects(&body)
            .iter()
            .all(|d| matches!(d, Defect::LoopUses { .. }))
    );
}

#[test]
fn a_flipped_face_uses_its_edges_the_wrong_way() {
    let mut body = block();
    let loop_ = body.faces[2].loops[0].0 as usize;
    body.loops[loop_].coedges.reverse();
    for coedge in &mut body.loops[loop_].coedges {
        coedge.reversed = !coedge.reversed;
    }
    assert!(has(&body, |d| matches!(
        d,
        Defect::EdgeUsedTwiceSameWay { .. }
    )));
    let same_way = defects(&body)
        .iter()
        .filter(|d| matches!(d, Defect::EdgeUsedTwiceSameWay { .. }))
        .count();
    assert_eq!(same_way, 4, "all four edges of the flipped face");
}

#[test]
fn a_broken_loop_is_not_closed() {
    let mut body = block();
    body.loops[0].coedges.swap(0, 1);
    assert!(has(&body, |d| matches!(
        d,
        Defect::LoopNotClosed {
            loop_: LoopId(0),
            ..
        }
    )));
    let mut body = block();
    body.loops[0].coedges.pop();
    assert!(has(&body, |d| matches!(
        d,
        Defect::LoopNotClosed {
            loop_: LoopId(0),
            ..
        }
    )));
    let mut body = block();
    body.loops[4].coedges.clear();
    assert!(has(&body, |d| matches!(
        d,
        Defect::LoopEmpty { loop_: LoopId(4) }
    )));
}

#[test]
fn dangling_references_are_reported_not_followed() {
    let mut body = block();
    body.edges[0].start = VertexId(99);
    assert!(has(
        &body,
        |d| matches!(d, Defect::DanglingReference { to, .. } if to == "vertex 99")
    ));
    let mut body = block();
    body.loops[1].coedges[0] = Coedge {
        edge: EdgeId(77),
        reversed: false,
    };
    assert!(has(
        &body,
        |d| matches!(d, Defect::DanglingReference { to, .. } if to == "edge 77")
    ));
    let mut body = block();
    body.faces[5].loops[0] = LoopId(66);
    assert!(has(
        &body,
        |d| matches!(d, Defect::DanglingReference { to, .. } if to == "loop 66")
    ));
    let mut body = block();
    body.shells[0].faces.push(FaceId(55));
    assert!(has(
        &body,
        |d| matches!(d, Defect::DanglingReference { to, .. } if to == "face 55")
    ));
}

#[test]
fn shared_and_orphaned_entities_are_found() {
    let mut body = block();
    body.shells[0].faces.push(FaceId(2));
    assert!(has(&body, |d| matches!(
        d,
        Defect::FaceUses {
            face: FaceId(2),
            uses: 2
        }
    )));
    let mut body = block();
    body.shells[0].faces.remove(0);
    assert!(has(&body, |d| matches!(
        d,
        Defect::FaceUses {
            face: FaceId(0),
            uses: 0
        }
    )));
    let mut body = block();
    let shared = body.faces[0].loops[0];
    body.faces[1].loops.push(shared);
    assert!(has(&body, |d| matches!(
        d,
        Defect::LoopUses { uses: 2, .. }
    )));
}

#[test]
fn bad_numbers_are_reported() {
    let mut body = cylinder();
    body.edges[1].range = (1.0, 1.0);
    assert!(has(&body, |d| matches!(
        d,
        Defect::EdgeRange { edge: EdgeId(1) }
    )));
    let mut body = cylinder();
    body.edges[1].range = (0.0, f64::INFINITY);
    assert!(has(&body, |d| matches!(d, Defect::NonFinite { .. })));
    let mut body = cylinder();
    body.vertices[0].point.y = f64::NAN;
    assert!(has(
        &body,
        |d| matches!(d, Defect::NonFinite { what } if what == "vertex 0")
    ));
    // A full circle cut short no longer ends at its vertex.
    let mut body = cylinder();
    body.edges[0].range = (0.0, 3.0);
    assert!(has(&body, |d| matches!(
        d,
        Defect::VertexOffEdge {
            edge: EdgeId(0),
            ..
        }
    )));
}

#[test]
fn a_loop_moved_to_the_wrong_face_is_found() {
    // Move the wall's top loop to the lid. Every count still adds up (each
    // edge used once each way, each loop in one face, Euler unchanged), but
    // the lid now has the same circle as two loops, and the wall no top.
    let mut body = cylinder();
    let extra = body.faces[0].loops.pop().unwrap();
    body.faces[2].loops.push(extra);
    assert_eq!(
        defects(&body),
        vec![Defect::EdgeInTwoLoopsOfFace {
            face: FaceId(2),
            edge: EdgeId(1)
        }]
    );
    assert!(
        craft3d_topo::mass_properties(&body).is_err(),
        "the wall is unbounded"
    );
}

#[test]
fn euler_counts_catch_a_stray_vertex() {
    // A stray vertex alone breaks the count.
    let mut body = block();
    body.add_vertex(Vec3::new(9.0, 9.0, 9.0));
    assert!(has(&body, |d| matches!(
        d,
        Defect::Euler { vertices: 9, .. }
    )));
}
