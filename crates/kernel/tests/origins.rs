//! The origin map of an extrusion, from which persistent names are made:
//! unique, worked out by hand for small cases, and unchanged by curves that
//! have nothing to do with the entity.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{Rng, catalogue, circle, frames, polygon, rectangle};
use craft3d_geom::Surface;
use craft3d_kernel::extrude::{OpResult, Origin, Role, extrude};
use craft3d_kernel::profile::{SketchCurve, detect_profiles};
use craft3d_math::{Frame, Tolerance, Vec3};
use std::collections::BTreeSet;

fn build(curves: &[SketchCurve], frame: &Frame, from: f64, to: f64) -> OpResult {
    let tol = Tolerance::default();
    let profiles = detect_profiles(curves, &tol).unwrap();
    extrude(&profiles, frame, from, to, &tol).unwrap()
}

fn origin(role: Role, sources: &[u64]) -> Origin {
    Origin::new(role, sources)
}

fn vertex_at(result: &OpResult, role: Role, sources: &[u64]) -> Vec3 {
    let wanted = origin(role, sources);
    let found: Vec<usize> = (0..result.origins.vertices.len())
        .filter(|&i| result.origins.vertices[i] == wanted)
        .collect();
    assert_eq!(found.len(), 1, "{wanted:?}");
    result.body.vertices[found[0]].point
}

fn edge_index(result: &OpResult, role: Role, sources: &[u64]) -> usize {
    let wanted = origin(role, sources);
    let found: Vec<usize> = (0..result.origins.edges.len())
        .filter(|&i| result.origins.edges[i] == wanted)
        .collect();
    assert_eq!(found.len(), 1, "{wanted:?}");
    found[0]
}

fn face_index(result: &OpResult, role: Role, sources: &[u64]) -> usize {
    let wanted = origin(role, sources);
    let found: Vec<usize> = (0..result.origins.faces.len())
        .filter(|&i| result.origins.faces[i] == wanted)
        .collect();
    assert_eq!(found.len(), 1, "{wanted:?}");
    found[0]
}

fn outward_normal(result: &OpResult, face: usize) -> Vec3 {
    let f = &result.body.faces[face];
    let Surface::Plane(_) = f.surface else {
        panic!("face {face} is not planar");
    };
    let n = f.surface.normal(0.0, 0.0).unwrap();
    if f.reversed { -n } else { n }
}

#[test]
fn origins_are_one_per_entity_and_unique() {
    for case in catalogue(1.0) {
        for frame in frames(1.0) {
            let result = build(&case.curves, &frame, -1.0, 2.0);
            let (body, origins) = (&result.body, &result.origins);
            assert_eq!(origins.faces.len(), body.faces.len(), "{}", case.name);
            assert_eq!(origins.edges.len(), body.edges.len(), "{}", case.name);
            assert_eq!(origins.vertices.len(), body.vertices.len(), "{}", case.name);
            let all: Vec<&Origin> = origins
                .faces
                .iter()
                .chain(&origins.edges)
                .chain(&origins.vertices)
                .collect();
            let distinct: BTreeSet<&Origin> = all.iter().copied().collect();
            assert_eq!(distinct.len(), all.len(), "{}: {all:?}", case.name);
            // Roles sit on the kind of entity they are for.
            use Role::*;
            assert!(
                origins
                    .faces
                    .iter()
                    .all(|o| matches!(o.role, StartCap | EndCap | Side))
            );
            assert!(
                origins
                    .edges
                    .iter()
                    .all(|o| matches!(o.role, StartEdge | EndEdge | LateralEdge))
            );
            assert!(
                origins
                    .vertices
                    .iter()
                    .all(|o| matches!(o.role, StartVertex | EndVertex))
            );
        }
    }
}

/// The rectangle of the catalogue: lines 10 (bottom, left to right), 20
/// (right, upwards), 30 (top) and 40 (left) around x 1..5, y 2..5,
/// extruded from −1 to 2 in the world frame.
#[test]
fn a_box_is_named_as_worked_out_by_hand() {
    let curves = rectangle(10, 10, (1.0, 5.0), (2.0, 5.0));
    let result = build(&curves, &Frame::WORLD, -1.0, 2.0);
    // The loop runs 10, 20, 30, 40: it arrives at a corner by one line and
    // leaves by the next.
    for (sources, x, y) in [
        ([40, 10], 1.0, 2.0),
        ([10, 20], 5.0, 2.0),
        ([20, 30], 5.0, 5.0),
        ([30, 40], 1.0, 5.0),
    ] {
        assert_eq!(
            vertex_at(&result, Role::StartVertex, &sources),
            Vec3::new(x, y, -1.0)
        );
        assert_eq!(
            vertex_at(&result, Role::EndVertex, &sources),
            Vec3::new(x, y, 2.0)
        );
        let lateral = &result.body.edges[edge_index(&result, Role::LateralEdge, &sources)];
        let ends = (
            result.body.vertices[lateral.start.0 as usize].point,
            result.body.vertices[lateral.end.0 as usize].point,
        );
        assert_eq!(ends, (Vec3::new(x, y, -1.0), Vec3::new(x, y, 2.0)));
    }
    // The edges made from line 10 lie along y = 2 at either end.
    for (role, z) in [(Role::StartEdge, -1.0), (Role::EndEdge, 2.0)] {
        let edge = &result.body.edges[edge_index(&result, role, &[10])];
        let middle = edge.curve.eval((edge.range.0 + edge.range.1) / 2.0);
        assert_eq!(middle, Vec3::new(3.0, 2.0, z));
    }
    // Sides face away from the rectangle; the caps down and up.
    for (id, normal) in [
        (10, Vec3::new(0.0, -1.0, 0.0)),
        (20, Vec3::new(1.0, 0.0, 0.0)),
        (30, Vec3::new(0.0, 1.0, 0.0)),
        (40, Vec3::new(-1.0, 0.0, 0.0)),
    ] {
        let face = face_index(&result, Role::Side, &[id]);
        assert_eq!(outward_normal(&result, face), normal, "side {id}");
    }
    let all = [10, 20, 30, 40];
    let start = face_index(&result, Role::StartCap, &all);
    let end = face_index(&result, Role::EndCap, &all);
    assert_eq!(outward_normal(&result, start), Vec3::new(0.0, 0.0, -1.0));
    assert_eq!(outward_normal(&result, end), Vec3::new(0.0, 0.0, 1.0));
    assert_eq!(result.origins.edges.len(), 12);
}

/// A hole runs clockwise, so its vertices are named in that direction.
/// Plate 10 × 6 (lines 1–4) with the square hole 2..5 by 1..4 drawn
/// counter-clockwise: 5 bottom, 6 right, 7 top, 8 left. Clockwise from its
/// smallest id the hole runs 5 (right to left), 8, 7, 6: it arrives at
/// (5, 1) by line 6 and leaves by line 5.
#[test]
fn the_vertices_of_a_hole_are_named_in_its_direction() {
    let mut curves = rectangle(1, 1, (0.0, 10.0), (0.0, 6.0));
    curves.extend(rectangle(5, 1, (2.0, 5.0), (1.0, 4.0)));
    let result = build(&curves, &Frame::WORLD, 0.0, 3.0);
    for (sources, x, y) in [
        ([6, 5], 5.0, 1.0),
        ([5, 8], 2.0, 1.0),
        ([8, 7], 2.0, 4.0),
        ([7, 6], 5.0, 4.0),
    ] {
        assert_eq!(
            vertex_at(&result, Role::StartVertex, &sources),
            Vec3::new(x, y, 0.0)
        );
        edge_index(&result, Role::LateralEdge, &sources);
    }
    // The caps are named after the outer loop only, and the hole's walls
    // face into the hole: the wall of line 5 (the hole's bottom, y = 1)
    // faces up, +y.
    face_index(&result, Role::StartCap, &[1, 2, 3, 4]);
    face_index(&result, Role::EndCap, &[1, 2, 3, 4]);
    let wall = face_index(&result, Role::Side, &[5]);
    assert_eq!(outward_normal(&result, wall), Vec3::new(0.0, 1.0, 0.0));
}

/// Two arcs that meet twice: the lens. Arc 2 (about (3, 0)) runs from
/// (0, 4) to (0, −4), arc 5 (about (−3, 0)) back up. The two lateral edges
/// join the same two faces and are told apart by the order of their
/// sources: at (0, −4) the loop arrives by arc 2 and leaves by arc 5.
#[test]
fn the_two_lateral_edges_of_a_lens_have_different_names() {
    let lens = catalogue(1.0)
        .into_iter()
        .find(|c| c.name == "lens")
        .unwrap();
    let result = build(&lens.curves, &Frame::WORLD, 0.0, 1.0);
    let laterals: Vec<&Origin> = result
        .origins
        .edges
        .iter()
        .filter(|o| o.role == Role::LateralEdge)
        .collect();
    assert_eq!(laterals.len(), 2);
    assert_ne!(laterals[0], laterals[1]);
    assert_eq!(
        vertex_at(&result, Role::StartVertex, &[2, 5]),
        Vec3::new(0.0, -4.0, 0.0)
    );
    assert_eq!(
        vertex_at(&result, Role::StartVertex, &[5, 2]),
        Vec3::new(0.0, 4.0, 0.0)
    );
    let edge = &result.body.edges[edge_index(&result, Role::LateralEdge, &[2, 5])];
    assert_eq!(
        result.body.vertices[edge.start.0 as usize].point,
        Vec3::new(0.0, -4.0, 0.0)
    );
}

/// A full circle: one vertex at each end, named by the circle twice, at the
/// circle's angle 0; no lateral edge.
#[test]
fn a_circle_names_its_one_vertex_twice_over() {
    let result = build(&[circle(42, [2.0, -1.0], 3.0)], &Frame::WORLD, 0.0, 1.0);
    assert_eq!(
        vertex_at(&result, Role::StartVertex, &[42, 42]),
        Vec3::new(5.0, -1.0, 0.0)
    );
    assert_eq!(
        vertex_at(&result, Role::EndVertex, &[42, 42]),
        Vec3::new(5.0, -1.0, 1.0)
    );
    edge_index(&result, Role::StartEdge, &[42]);
    edge_index(&result, Role::EndEdge, &[42]);
    face_index(&result, Role::Side, &[42]);
    face_index(&result, Role::StartCap, &[42]);
    assert!(
        result
            .origins
            .edges
            .iter()
            .all(|o| o.role != Role::LateralEdge)
    );
}

/// Adding curves that have nothing to do with an entity leaves its origin,
/// and the geometry found under that origin, as they were: a separate
/// profile with smaller and larger ids, another hole in the same plate, in
/// any input order.
#[test]
fn origins_survive_unrelated_curves() {
    let frame = frames(1.0)[1];
    let mut base_curves = rectangle(20, 1, (0.0, 10.0), (0.0, 6.0));
    base_curves.push(circle(30, [4.0, 3.0], 2.0));
    let base = build(&base_curves, &frame, -1.0, 2.0);

    let mut rng = Rng(7);
    let mut more = base_curves.clone();
    // A triangle far away with ids below every existing one.
    more.extend(polygon(1, 1, &[[30.0, 0.0], [36.0, 0.0], [30.0, 8.0]]));
    // A disc far away with an id above every existing one.
    more.push(circle(99, [-20.0, 0.0], 1.0));
    // A second hole in the plate, with an id between the others.
    more.push(circle(25, [8.5, 3.0], 1.0));
    for _ in 0..3 {
        rng.shuffle(&mut more);
        let grown = build(&more, &frame, -1.0, 2.0);
        assert!(grown.body.faces.len() > base.body.faces.len());
        for (i, o) in base.origins.vertices.iter().enumerate() {
            let p = vertex_at(&grown, o.role, &o.sources);
            assert_eq!(p, base.body.vertices[i].point, "{o:?}");
        }
        for (i, o) in base.origins.edges.iter().enumerate() {
            let (a, b) = (
                &base.body.edges[i],
                &grown.body.edges[edge_index(&grown, o.role, &o.sources)],
            );
            assert_eq!((&a.curve, a.range), (&b.curve, b.range), "{o:?}");
            let point =
                |r: &OpResult, v: craft3d_topo::VertexId| r.body.vertices[v.0 as usize].point;
            assert_eq!(point(&base, a.start), point(&grown, b.start), "{o:?}");
            assert_eq!(point(&base, a.end), point(&grown, b.end), "{o:?}");
        }
        for (i, o) in base.origins.faces.iter().enumerate() {
            let (a, b) = (
                &base.body.faces[i],
                &grown.body.faces[face_index(&grown, o.role, &o.sources)],
            );
            assert_eq!((&a.surface, a.reversed), (&b.surface, b.reversed), "{o:?}");
            // Only the caps gained a loop (the new hole).
            let extra = usize::from(matches!(o.role, Role::StartCap | Role::EndCap));
            assert_eq!(b.loops.len(), a.loops.len() + extra, "{o:?}");
        }
    }
}

/// A known limit, pinned so that it cannot change unnoticed: a curve that
/// is *not* unrelated. Drawing a loop around an existing one turns it from
/// an outer boundary into a hole, which reverses the direction it runs in.
/// The names of its sides and cap edges stay; the names of its vertices and
/// lateral edges get their sources in the other order, and the old names
/// are gone rather than moved to another corner; its caps are gone too
/// (they are the outer loop's now).
#[test]
fn enclosing_a_loop_renames_its_vertices_but_never_moves_a_name() {
    let inner = rectangle(10, 10, (1.0, 5.0), (2.0, 5.0));
    let alone = build(&inner, &Frame::WORLD, 0.0, 1.0);
    let mut both = inner.clone();
    both.extend(rectangle(1, 1, (-10.0, 20.0), (-10.0, 20.0)));
    let enclosed = build(&both, &Frame::WORLD, 0.0, 1.0);

    // Alone, the corner (5, 2) is where the loop arrives by 10 and leaves
    // by 20; as a hole it runs the other way.
    let corner = Vec3::new(5.0, 2.0, 0.0);
    assert_eq!(vertex_at(&alone, Role::StartVertex, &[10, 20]), corner);
    assert_eq!(vertex_at(&enclosed, Role::StartVertex, &[20, 10]), corner);
    for o in alone.origins.vertices.iter().chain(&alone.origins.edges) {
        let kept = enclosed
            .origins
            .vertices
            .iter()
            .chain(&enclosed.origins.edges)
            .any(|x| x == o);
        let directed = matches!(
            o.role,
            Role::StartVertex | Role::EndVertex | Role::LateralEdge
        );
        assert_eq!(kept, !directed, "{o:?}");
    }
    for o in &alone.origins.faces {
        let kept = enclosed.origins.faces.contains(o);
        assert_eq!(kept, o.role == Role::Side, "{o:?}");
    }
}

/// The same box drawn clockwise: 1 left (upwards), 2 top, 3 right
/// (downwards), 4 bottom, around x 0..4, y 0..3. The loop still runs
/// counter-clockwise from its smallest id: 1 (downwards now), 4, 3, 2.
/// Caps list the outer loop's ids ascending, not in loop order.
#[test]
fn a_box_drawn_clockwise_is_named_counter_clockwise() {
    let curves = polygon(1, 1, &[[0.0, 0.0], [0.0, 3.0], [4.0, 3.0], [4.0, 0.0]]);
    let result = build(&curves, &Frame::WORLD, 0.0, 1.0);
    face_index(&result, Role::StartCap, &[1, 2, 3, 4]);
    face_index(&result, Role::EndCap, &[1, 2, 3, 4]);
    // Counter-clockwise the loop arrives at the origin by line 1 and
    // leaves by line 4; at (4, 0) it arrives by 4 and leaves by 3.
    for (sources, x, y) in [
        ([1, 4], 0.0, 0.0),
        ([4, 3], 4.0, 0.0),
        ([3, 2], 4.0, 3.0),
        ([2, 1], 0.0, 3.0),
    ] {
        assert_eq!(
            vertex_at(&result, Role::StartVertex, &sources),
            Vec3::new(x, y, 0.0)
        );
    }
    // Line 1 is the left side: its wall faces −x.
    let wall = face_index(&result, Role::Side, &[1]);
    assert_eq!(outward_normal(&result, wall), Vec3::new(-1.0, 0.0, 0.0));
}
