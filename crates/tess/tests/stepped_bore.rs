//! The wall of a bore with a stepped bottom: a cylinder face whose lower
//! loop winds once around the axis as half a circle at one depth, a line
//! along the cylinder, half a circle deeper and a line back, the second
//! line at the angle `±π` where the surface's parameter wraps. The
//! straight seam tried first runs from that line's lower end up the same
//! meridian; computed a period on, it leaned a last bit away from the
//! line's other end, the exact test saw neither a crossing nor a touch,
//! and the seam swallowed the line: "its mesh is not closed along edge
//! 1068" on the engine block's bored mounts at every tolerance finer than
//! about 1 mm (2026-10-15). The geometry is the engine block's, to the
//! bit where it matters (a cylinder about x of radius 29 at x = -220).
//!
//! Oracle: the mesh is made, its border (after welding) is exactly the
//! chords of the face's edges, and its area is the closed form within
//! the tolerance times the perimeter.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Cylinder, Line, Surface};
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

const R: f64 = 29.0;

fn coedge(edge: EdgeId, reversed: bool) -> Coedge {
    Coedge { edge, reversed }
}

fn circle_at(x: f64) -> Curve3 {
    let frame = Frame::new(
        Vec3::new(x, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    )
    .unwrap();
    Curve3::Circle(Circle::new(frame, R).unwrap())
}

/// The sheet: the wall from x = 165 (one half) or 188 (the other) up to
/// x = 196.
fn stepped_wall() -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let axis = Frame::new(
        Vec3::new(-220.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    )
    .unwrap();
    let surface = Surface::Cylinder(Cylinder::new(axis, R).unwrap());

    let top = circle_at(196.0);
    let t0 = body.add_vertex(top.eval(0.0));
    let rim = body.add_edge(top, (0.0, TAU), t0, t0);

    let shallow = circle_at(165.0);
    let deep = circle_at(188.0);
    let a = body.add_vertex(Vec3::new(165.0, R, 0.0));
    let b = body.add_vertex(shallow.eval(PI));
    let c = body.add_vertex(Vec3::new(188.0, -R, 0.0));
    let d = body.add_vertex(Vec3::new(188.0, R, 0.0));
    let half_shallow = body.add_edge(shallow, (0.0, PI), a, b);
    let half_deep = body.add_edge(deep, (PI, TAU), c, d);
    let x = Vec3::new(1.0, 0.0, 0.0);
    let line_top = Line::new(Vec3::new(-220.0, R, 0.0), x).unwrap();
    let line_bottom = Line::new(Vec3::new(-220.0, -R, 0.0), x).unwrap();
    let up = body.add_edge(Curve3::Line(line_top), (385.0, 408.0), a, d);
    let down = body.add_edge(Curve3::Line(line_bottom), (385.0, 408.0), b, c);

    let upper = body.add_loop(vec![coedge(rim, false)]);
    let lower = body.add_loop(vec![
        coedge(half_shallow, true),
        coedge(up, false),
        coedge(half_deep, true),
        coedge(down, true),
    ]);
    let face = body.add_face(surface, true, vec![upper, lower]);
    body.add_shell(vec![face]);
    body
}

#[test]
fn a_bore_with_a_stepped_bottom_meshes_along_its_line_at_the_wrap() {
    let body = stepped_wall();
    let exact = R * PI * (2.0 * 31.0 - 23.0);
    let perimeter = 2.0 * TAU * R + 2.0 * 23.0;
    for tolerance in [1.0, 0.6, 0.3, 0.24, 0.1, 0.03, 0.01] {
        let params = TessParams {
            chordal_tolerance: tolerance,
            ..TessParams::default()
        };
        let mesh =
            tessellate_body(&body, &params).unwrap_or_else(|e| panic!("at {tolerance}: {e}"));
        let area = mesh.area();
        assert!(
            (area - exact).abs() <= tolerance * perimeter,
            "at {tolerance}: area {area} against {exact}"
        );
        // The border is the face's edges: every side used once is on the
        // loops (y = ±29 lines, or a circle at x = 165, 188 or 196).
        let welded = mesh.weld().unwrap();
        let mut sides: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for t in &welded.triangles {
            for k in 0..3 {
                let (p, q) = (t[k], t[(k + 1) % 3]);
                *sides.entry((p.min(q), p.max(q))).or_insert(0) += 1;
            }
        }
        for (&(p, q), &n) in &sides {
            assert!(n <= 2, "at {tolerance}: a side of {n} triangles");
            if n == 1 {
                let (p, q) = (welded.positions[p as usize], welded.positions[q as usize]);
                let on_rim = |x: f64| [165.0, 188.0, 196.0].contains(&x);
                let on_line = p[1] == q[1] && p[1].abs() == R && p[2].abs() < 1e-9;
                assert!(
                    (p[0] == q[0] && on_rim(p[0])) || on_line,
                    "at {tolerance}: a border {p:?} -> {q:?} that is no edge"
                );
            }
        }
        assert_eq!(
            tessellate_body_with_threads(&body, &params, 1).unwrap(),
            mesh,
            "at {tolerance}: one thread"
        );
    }
}
