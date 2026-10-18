//! The checker detects each kind of defect when it is seeded into an
//! otherwise valid body (DoD of milestone M1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, Line, NurbsCurve, Plane, Surface};
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

/// A plate 4 × 3 × 1 with a hole of radius 0.5 through its middle, the
/// hole (its wall, its two circles and their vertices) moved by `by`
/// within the plate's planes: every other check still holds.
fn plate_with_hole_moved(by: Vec3) -> Body {
    let mut body =
        craft3d_testkit::trimmed::plate_with_round_hole(&Frame::WORLD, 4.0, 3.0, 1.0, 0.5)
            .unwrap()
            .body;
    let wall = body
        .faces
        .iter()
        .position(|f| matches!(f.surface, Surface::Cylinder(_)))
        .unwrap();
    let Surface::Cylinder(c) = body.faces[wall].surface else {
        panic!("a cylinder")
    };
    body.faces[wall].surface = Surface::Cylinder(
        craft3d_geom::Cylinder::new(c.frame().at(c.frame().origin() + by).unwrap(), c.radius())
            .unwrap(),
    );
    let mut moved = std::collections::BTreeSet::new();
    for l in body.faces[wall].loops.clone() {
        for coedge in body.loops[l.0 as usize].coedges.clone() {
            let edge = &mut body.edges[coedge.edge.0 as usize];
            if let craft3d_geom::Curve3::Circle(circle) = edge.curve {
                edge.curve = craft3d_geom::Curve3::Circle(
                    craft3d_geom::Circle::new(
                        circle.frame().at(circle.frame().origin() + by).unwrap(),
                        circle.radius(),
                    )
                    .unwrap(),
                );
                moved.insert(edge.start);
                moved.insert(edge.end);
            }
        }
    }
    for v in moved {
        body.vertices[v.0 as usize].point += by;
    }
    body
}

#[test]
fn a_hole_left_outside_its_face_or_across_its_border_is_found() {
    // Moved within the plate: clean.
    assert_eq!(
        defects(&plate_with_hole_moved(Vec3::new(1.0, 0.5, 0.0))),
        vec![]
    );
    // Wholly outside the plate: the hole's loops on the top and on the
    // bottom are stranded, and nothing else is wrong (the hole is a valid
    // cylinder, its circles lie on the planes).
    let found = defects(&plate_with_hole_moved(Vec3::new(3.0, 0.0, 0.0)));
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found
            .iter()
            .all(|d| matches!(d, Defect::StrandedLoop { .. }))
    );
    // Across the plate's border: the loops cross.
    let across = plate_with_hole_moved(Vec3::new(2.0, 0.0, 0.0));
    assert!(has(&across, |d| matches!(d, Defect::StrandedLoop { .. })));
}

/// Two blocks apart, one shell each (`merge` keeps them so).
fn two_blocks() -> Body {
    let far = solids::block(
        &Frame::WORLD.at(Vec3::new(10.0, 0.0, 0.0)).unwrap(),
        1.0,
        1.0,
        1.0,
    )
    .unwrap()
    .body;
    craft3d_topo::merge(&[&block(), &far]).unwrap().0
}

#[test]
fn shells_that_hang_together_and_apart_are_told_from_how_faces_share_edges() {
    let body = two_blocks();
    assert_eq!(body.shells.len(), 2);
    assert_eq!(defects(&body), vec![]);
    assert_eq!(craft3d_topo::shell_genera(&body), Some(vec![0, 0]));
    assert_eq!(craft3d_topo::face_groups(&body).len(), 2);
    // The same faces, three of each block in each shell: the counts over
    // the whole body still fit (V − E + F = 16 − 24 + 12 = 2 · 2), the
    // shells do not.
    let mut mixed = body.clone();
    let first: Vec<FaceId> = mixed.shells[0].faces.clone();
    let second: Vec<FaceId> = mixed.shells[1].faces.clone();
    mixed.shells[0].faces = first[..3].iter().chain(&second[..3]).copied().collect();
    mixed.shells[1].faces = first[3..].iter().chain(&second[3..]).copied().collect();
    assert!(genus(&mixed).is_some(), "the whole body's counts fit");
    assert!(has(&mixed, |d| matches!(
        d,
        Defect::ShellApart { parts: 2, .. }
    )));
    assert!(has(&mixed, |d| matches!(d, Defect::EdgeInTwoShells { .. })));
    // Both blocks in one shell: the whole body's counts already fail,
    // and the shell is in two pieces.
    let mut one = body.clone();
    let all: Vec<FaceId> = first.iter().chain(&second).copied().collect();
    one.shells = vec![craft3d_topo::Shell { faces: all }];
    assert!(has(&one, |d| matches!(
        d,
        Defect::ShellApart { parts: 2, .. }
    )));
    assert!(has(&one, |d| matches!(d, Defect::Euler { .. })));
}

#[test]
fn a_shell_alone_is_a_body_of_its_faces_and_only_what_they_use() {
    let body = two_blocks();
    let second = craft3d_topo::shell_alone(&body, craft3d_topo::ShellId(1)).unwrap();
    assert_eq!(
        (
            second.vertices.len(),
            second.edges.len(),
            second.faces.len(),
            second.shells.len()
        ),
        (8, 12, 6, 1)
    );
    assert_eq!(defects(&second), vec![]);
    let volume = craft3d_topo::mass_properties(&second).unwrap().volume;
    assert!((volume - 1.0).abs() < 1e-9, "{volume}");
    assert!(craft3d_topo::shell_alone(&body, craft3d_topo::ShellId(2)).is_none());
}

/// A square sheet in the plane `z = 0` whose edge from `(3, 0, 0)` to
/// `(1, 0, 0)` is `along`, over `range`.
fn square_sheet(along: Curve3, range: (f64, f64)) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let corners = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(3.0, 0.0, 0.0),
        Vec3::new(3.0, 1.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
    ];
    let v: Vec<VertexId> = corners.iter().map(|p| body.add_vertex(*p)).collect();
    // The given edge runs from the second corner to the first: the loop
    // uses it against its direction.
    let first = body.add_edge(along, range, v[1], v[0]);
    let mut coedges = vec![Coedge {
        edge: first,
        reversed: true,
    }];
    for k in 1..4 {
        let (a, b) = (corners[k], corners[(k + 1) % 4]);
        let edge = body.add_edge(
            Curve3::Line(Line::through(a, b).unwrap()),
            (0.0, a.distance(b)),
            v[k],
            v[(k + 1) % 4],
        );
        coedges.push(Coedge {
            edge,
            reversed: false,
        });
    }
    let loop_ = body.add_loop(coedges);
    let face = body.add_face(Surface::Plane(Plane::new(Frame::WORLD)), false, vec![loop_]);
    body.add_shell(vec![face]);
    body
}

/// A point of an edge whose distance to the surface cannot be had is not
/// known to be on it, and the edge is off its surface: here a rational
/// line of degree one, used past its domain to where its weight vanishes,
/// so that the point in the middle of the edge's range is not finite,
/// while every other place the checker measures lies in the plane. (It
/// used to leave such a point out of its measure, and the edge passed.)
#[test]
fn a_point_without_a_distance_puts_the_edge_off_its_surface() {
    let straight =
        Curve3::Line(Line::through(Vec3::new(3.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap());
    assert_eq!(defects(&square_sheet(straight, (0.0, 2.0))), vec![]);
    // Weight `(1 - t) + 2t = 1 + t`: zero at `t = -1`, the middle of the
    // range `(-3, 1)`; the ends are `(3, 0, 0)` and `(1, 0, 0)`.
    let rational = NurbsCurve::new(
        1,
        vec![0.0, 0.0, 1.0, 1.0],
        vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)],
        vec![1.0, 2.0],
    )
    .unwrap();
    assert!(rational.eval(-3.0).distance(Vec3::new(3.0, 0.0, 0.0)) < 1e-12);
    assert!(rational.eval(1.0).distance(Vec3::new(1.0, 0.0, 0.0)) < 1e-12);
    assert!(!rational.eval(-1.0).is_finite());
    for t in [-2.0, 0.0] {
        assert_eq!(rational.eval(t).z, 0.0);
    }
    let body = square_sheet(Curve3::Nurbs(rational), (-3.0, 1.0));
    assert!(has(&body, |d| matches!(
        d,
        Defect::EdgeOffSurface { face: FaceId(0), edge: EdgeId(0), distance } if distance.is_infinite()
    )));
    // It says the distance could not be had, not that it is infinite.
    let text = Defect::EdgeOffSurface {
        face: FaceId(0),
        edge: EdgeId(0),
        distance: f64::INFINITY,
    }
    .to_string();
    assert!(text.contains("cannot be measured"), "{text}");
    assert!(!text.contains("inf"), "{text}");
}
