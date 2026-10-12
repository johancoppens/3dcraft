//! Faces on surfaces that close on themselves, as sheets with one face:
//! holes in a wall, patches and holes that straddle the seam u = 0, bands
//! on a sphere and a torus, a face through both poles of a sphere.
//!
//! The oracle for each is a closed-form area, the tolerance measured on
//! every triangle, and the mesh's own boundary: after welding, the only
//! edges used by one triangle are the chords of the face's edges, so a
//! seam that did not close or a pole that left a gap shows up as extra
//! open edges.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Cylinder, Line, Sphere, Surface, Torus};
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    }
}

fn tilted() -> Frame {
    Frame::new(
        Vec3::new(1.0, -2.0, 0.5),
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap()
}

fn forward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: false,
    }
}

fn backward(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: true,
    }
}

/// An arc of the circle of `radius` about the z axis of `frame` at height
/// `z`, from angle `from` to angle `to` (`from < to`).
fn arc(body: &mut Body, frame: &Frame, radius: f64, z: f64, from: f64, to: f64) -> EdgeId {
    let level = Frame::new(frame.point(0.0, 0.0, z), frame.z(), frame.x()).unwrap();
    let curve = Curve3::Circle(Circle::new(level, radius).unwrap());
    let start = body.add_vertex(curve.eval(from));
    let end = if to - from >= TAU - 1e-12 {
        start
    } else {
        body.add_vertex(curve.eval(to))
    };
    body.add_edge(curve, (from, to), start, end)
}

/// A straight edge between the start (or end) of one edge and the start
/// (or end) of another.
fn join(body: &mut Body, from: (EdgeId, bool), to: (EdgeId, bool)) -> EdgeId {
    let vertex = |body: &Body, (edge, at_end): (EdgeId, bool)| {
        let e = body.edge(edge).unwrap();
        if at_end { e.end } else { e.start }
    };
    let (a, b) = (vertex(body, from), vertex(body, to));
    let (pa, pb) = (body.vertex(a).unwrap().point, body.vertex(b).unwrap().point);
    let line = Curve3::Line(Line::through(pa, pb).unwrap());
    body.add_edge(line, (0.0, pa.distance(pb)), a, b)
}

fn sheet(body: &mut Body, surface: Surface, reversed: bool, loops: Vec<Vec<Coedge>>) {
    let loops = loops.into_iter().map(|l| body.add_loop(l)).collect();
    let face = body.add_face(surface, reversed, loops);
    body.add_shell(vec![face]);
}

/// A window in a wall of `radius`: between the angles `u` and the heights
/// `v`, counter-clockwise seen from outside (or clockwise, as a hole).
fn window(
    body: &mut Body,
    frame: &Frame,
    radius: f64,
    u: (f64, f64),
    v: (f64, f64),
    hole: bool,
) -> Vec<Coedge> {
    let low = arc(body, frame, radius, v.0, u.0, u.1);
    let high = arc(body, frame, radius, v.1, u.0, u.1);
    let right = join(body, (low, true), (high, true));
    let left = join(body, (low, false), (high, false));
    if hole {
        vec![forward(left), forward(high), backward(right), backward(low)]
    } else {
        vec![forward(low), forward(right), backward(high), backward(left)]
    }
}

/// Meshes the sheet at two tolerances and checks: every triangle within
/// tolerance, with area, facing the way the face does; the mesh's open
/// edges are exactly the chords of the body's edges; the area converges
/// to `expected`; and the mesh is the same on one thread and on four.
fn assert_sheet(body: &Body, expected: f64, what: &str) {
    assert_eq!(
        craft3d_topo::check(body, &craft3d_math::Tolerance::default()),
        vec![],
        "{what}"
    );
    let face = &body.faces[0];
    let mut previous = f64::INFINITY;
    for tolerance in [3e-2, 3e-3, 3e-4] {
        let mesh = tessellate_body(body, &params(tolerance))
            .unwrap_or_else(|e| panic!("{what} at {tolerance}: {e}"));
        assert_eq!(
            mesh,
            tessellate_body_with_threads(body, &params(tolerance), 4).unwrap(),
            "{what}"
        );
        for t in 0..mesh.triangles.len() {
            let [a, b, c] = mesh.corners(t).unwrap();
            let centroid = (a + b + c) * (1.0 / 3.0);
            for p in [a, b, c] {
                let (u, v) = face.surface.project(p).unwrap();
                assert!(
                    face.surface.eval(u, v).distance(p) < 1e-9,
                    "{what}: vertex off"
                );
            }
            for p in [a.lerp(b, 0.5), b.lerp(c, 0.5), c.lerp(a, 0.5), centroid] {
                let (u, v) = face.surface.project(p).unwrap();
                let off = face.surface.eval(u, v).distance(p);
                assert!(
                    off <= tolerance * (1.0 + 1e-9),
                    "{what} at {tolerance}: triangle {t} is {off:e} from the surface"
                );
            }
            let facing = (b - a).cross(c - a);
            assert!(facing.length() > 0.0, "{what}: triangle {t} has no area");
            let (u, v) = face.surface.project(centroid).unwrap();
            let outward =
                face.surface.normal(u, v).unwrap() * if face.reversed { -1.0 } else { 1.0 };
            assert!(
                facing.dot(outward) > 0.0,
                "{what} at {tolerance}: triangle {t} faces the wrong way"
            );
        }
        let welded = mesh.weld().unwrap();
        assert_eq!(welded.triangles.len(), mesh.triangles.len(), "{what}");
        let mut uses: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for &[a, b, c] in &welded.triangles {
            for e in [(a, b), (b, c), (c, a)] {
                *uses.entry(e).or_insert(0) += 1;
            }
        }
        assert!(
            uses.values().all(|&n| n == 1),
            "{what}: an edge is used twice the same way"
        );
        let open = uses
            .keys()
            .filter(|(a, b)| !uses.contains_key(&(*b, *a)))
            .count();
        // The chords of the body's edges: each loop's welded points, as
        // many chords as points. Count them through a mesh of the same
        // body with every edge used: the number of distinct boundary
        // points equals the number of chords for closed loops.
        let boundary_points: std::collections::BTreeSet<[u64; 3]> = uses
            .keys()
            .filter(|(a, b)| !uses.contains_key(&(*b, *a)))
            .flat_map(|&(a, b)| [a, b])
            .map(|k| welded.positions[k as usize].map(f64::to_bits))
            .collect();
        assert_eq!(
            open,
            boundary_points.len(),
            "{what} at {tolerance}: the open edges do not form simple closed loops"
        );
        // Every open edge lies on one of the body's edge curves.
        for (a, b) in uses.keys().filter(|(a, b)| !uses.contains_key(&(*b, *a))) {
            let [x, y, z] = welded.positions[*a as usize];
            let [x2, y2, z2] = welded.positions[*b as usize];
            for p in [Vec3::new(x, y, z), Vec3::new(x2, y2, z2)] {
                let on_an_edge = body.edges.iter().any(|e| {
                    let t = e.curve.project(p).unwrap();
                    e.curve.eval(t).distance(p) < 1e-9
                });
                assert!(
                    on_an_edge,
                    "{what} at {tolerance}: an open edge of the mesh is not on the boundary \
                     (a seam or a pole did not close)"
                );
            }
        }
        let error = (mesh.area() - expected).abs() / expected;
        println!(
            "{what} at {tolerance:e}: {} triangles, area error {error:e}",
            mesh.triangles.len()
        );
        assert!(error <= previous, "{what}: area error grew to {error:e}");
        previous = error;
    }
    assert!(previous < 1e-3, "{what}: area error {previous:e}");
}

#[test]
fn walls_windows_and_holes_on_a_cylinder() {
    for frame in [Frame::WORLD, tilted()] {
        let (r, h) = (1.5, 2.0);
        let cylinder = || Surface::Cylinder(Cylinder::new(frame, r).unwrap());
        // Windows at several places around the axis, among them across
        // the seam u = 0 and with a corner exactly on it.
        for (u0, u1) in [
            (0.4, 1.7),
            (5.8, 7.1),
            (-0.5, 0.5),
            (0.0, 1.0),
            (5.0, TAU),
            (3.0, 8.5),
        ] {
            let what = format!("window from {u0} to {u1}");
            let mut body = Body::new(BodyKind::Sheet);
            let outline = window(&mut body, &frame, r, (u0, u1), (0.3, 1.6), false);
            sheet(&mut body, cylinder(), false, vec![outline]);
            assert_sheet(&body, r * (u1 - u0) * 1.3, &what);

            // The same window seen from inside the cylinder.
            let mut body = Body::new(BodyKind::Sheet);
            let outline = window(&mut body, &frame, r, (u0, u1), (0.3, 1.6), true);
            sheet(&mut body, cylinder(), true, vec![outline]);
            assert_sheet(&body, r * (u1 - u0) * 1.3, &format!("{what}, reversed"));

            // A window with a hole in it.
            let mut body = Body::new(BodyKind::Sheet);
            let outline = window(&mut body, &frame, r, (u0, u1), (0.3, 1.6), false);
            let middle = 0.5 * (u0 + u1);
            let hole = window(
                &mut body,
                &frame,
                r,
                (middle - 0.2, middle + 0.3),
                (0.7, 1.1),
                true,
            );
            sheet(&mut body, cylinder(), false, vec![outline, hole]);
            assert_sheet(
                &body,
                r * (u1 - u0) * 1.3 - r * 0.5 * 0.4,
                &format!("{what}, with a hole"),
            );

            // The whole wall with that window as a hole.
            if u1 - u0 < 5.0 {
                let mut body = Body::new(BodyKind::Sheet);
                let bottom = arc(&mut body, &frame, r, 0.0, 0.0, TAU);
                let top = arc(&mut body, &frame, r, h, 0.0, TAU);
                let hole = window(&mut body, &frame, r, (u0, u1), (0.3, 1.6), true);
                sheet(
                    &mut body,
                    cylinder(),
                    false,
                    vec![vec![forward(bottom)], vec![backward(top)], hole],
                );
                assert_sheet(
                    &body,
                    TAU * r * h - r * (u1 - u0) * 1.3,
                    &format!("wall with a {what}"),
                );
            }
        }
        // Two holes on opposite sides, one of them on the seam, and the
        // two end circles starting at different angles.
        let mut body = Body::new(BodyKind::Sheet);
        let bottom = arc(&mut body, &frame, r, 0.0, 1.0, 1.0 + TAU);
        let top = arc(&mut body, &frame, r, h, 4.0, 4.0 + TAU);
        let first = window(&mut body, &frame, r, (-0.6, 0.7), (0.3, 1.6), true);
        let second = window(&mut body, &frame, r, (2.5, 3.9), (0.5, 1.2), true);
        sheet(
            &mut body,
            cylinder(),
            false,
            vec![vec![forward(bottom)], vec![backward(top)], first, second],
        );
        assert_sheet(
            &body,
            TAU * r * h - r * 1.3 * 1.3 - r * 1.4 * 0.7,
            "wall with two holes",
        );
    }
}

#[test]
fn zones_and_lunes_of_a_sphere() {
    for frame in [Frame::WORLD, tilted()] {
        let r = 2.0;
        let sphere = || Surface::Sphere(Sphere::new(frame, r).unwrap());
        let latitude = |body: &mut Body, v: f64, start: f64| {
            arc(body, &frame, r * v.cos(), r * v.sin(), start, start + TAU)
        };
        // The zone between two latitudes, on either side of the equator
        // and close to a pole; the circles start at different angles.
        for (v0, v1) in [(-0.4, 0.9), (0.2, 1.3), (-1.5, -1.2), (1.3, 1.55)] {
            let mut body = Body::new(BodyKind::Sheet);
            let low = latitude(&mut body, v0, 0.0);
            let high = latitude(&mut body, v1, 2.0);
            sheet(
                &mut body,
                sphere(),
                false,
                vec![vec![forward(low)], vec![backward(high)]],
            );
            assert_sheet(
                &body,
                TAU * r * r * (v1.sin() - v0.sin()),
                &format!("zone from {v0} to {v1}"),
            );
        }
        // Everything below a latitude: the face reaches the south pole.
        let mut body = Body::new(BodyKind::Sheet);
        let rim = latitude(&mut body, 0.7, 1.0);
        sheet(&mut body, sphere(), false, vec![vec![backward(rim)]]);
        assert_sheet(&body, TAU * r * r * (0.7_f64.sin() + 1.0), "bowl");
        // A zone with a window cut out of it across the seam.
        let mut body = Body::new(BodyKind::Sheet);
        let low = latitude(&mut body, -0.8, 0.0);
        let high = latitude(&mut body, 0.8, 0.0);
        let (w0, w1) = (-0.3_f64, 0.4_f64);
        let south = arc(&mut body, &frame, r * w0.cos(), r * w0.sin(), -0.5, 0.6);
        let north = arc(&mut body, &frame, r * w1.cos(), r * w1.sin(), -0.5, 0.6);
        let meridian = |body: &mut Body, u: f64, from: EdgeId, to: EdgeId, at_end: bool| {
            // The circle through both poles at longitude u; its angle is
            // the latitude.
            let x = frame.vector(u.cos(), u.sin(), 0.0);
            let side = frame.vector(-u.sin(), u.cos(), 0.0);
            let plane = Frame::new(frame.origin(), -side, x).unwrap();
            let curve = Curve3::Circle(Circle::new(plane, r).unwrap());
            let pick = |e: EdgeId| {
                let e = body.edge(e).unwrap();
                if at_end { e.end } else { e.start }
            };
            let (a, b) = (pick(from), pick(to));
            body.add_edge(curve, (w0, w1), a, b)
        };
        let east = meridian(&mut body, 0.6, south, north, true);
        let west = meridian(&mut body, -0.5, south, north, false);
        sheet(
            &mut body,
            sphere(),
            false,
            vec![
                vec![forward(low)],
                vec![backward(high)],
                vec![
                    forward(west),
                    forward(north),
                    backward(east),
                    backward(south),
                ],
            ],
        );
        assert_sheet(
            &body,
            r * r * (TAU * 2.0 * 0.8_f64.sin() - 1.1 * (w1.sin() - w0.sin())),
            "zone with a window",
        );

        // Lunes: between two meridians, from pole to pole. The boundary
        // passes through both poles, where it turns by the lune's angle.
        for (u0, u1) in [
            (0.3, 1.4),
            (-0.7, 0.9),
            (5.5, 7.0),
            (1.0, 1.0 + PI),
            (2.0, 6.5),
        ] {
            let mut body = Body::new(BodyKind::Sheet);
            let south = body.add_vertex(frame.point(0.0, 0.0, -r));
            let north = body.add_vertex(frame.point(0.0, 0.0, r));
            let mut half = |u: f64| {
                let x = frame.vector(u.cos(), u.sin(), 0.0);
                let side = frame.vector(-u.sin(), u.cos(), 0.0);
                let plane = Frame::new(frame.origin(), -side, x).unwrap();
                let curve = Curve3::Circle(Circle::new(plane, r).unwrap());
                body.add_edge(curve, (-FRAC_PI_2, FRAC_PI_2), south, north)
            };
            // Up the eastern meridian, down the western: counter-clockwise
            // seen from outside.
            let (west, east) = (half(u0), half(u1));
            sheet(
                &mut body,
                sphere(),
                false,
                vec![vec![forward(east), backward(west)]],
            );
            assert_sheet(
                &body,
                2.0 * r * r * (u1 - u0),
                &format!("lune from {u0} to {u1}"),
            );
        }
    }
}

#[test]
fn bands_of_a_torus() {
    for frame in [Frame::WORLD, tilted()] {
        let (major, minor) = (3.0, 1.0);
        let torus = || Surface::Torus(Torus::new(frame, major, minor).unwrap());
        // Around the axis: between two circles of latitude of the tube.
        // The outer half crosses the seam v = 0, the inner half does not,
        // and a narrow band lies entirely "above" 2π.
        for (v0, v1) in [
            (-FRAC_PI_2, FRAC_PI_2),
            (FRAC_PI_2, 1.5 * PI),
            (5.0, 6.0),
            (6.0, 7.5),
        ] {
            let mut body = Body::new(BodyKind::Sheet);
            let ring = |body: &mut Body, v: f64, start: f64| {
                arc(
                    body,
                    &frame,
                    major + minor * v.cos(),
                    minor * v.sin(),
                    start,
                    start + TAU,
                )
            };
            let low = ring(&mut body, v0, 0.0);
            let high = ring(&mut body, v1, 3.0);
            sheet(
                &mut body,
                torus(),
                false,
                vec![vec![forward(low)], vec![backward(high)]],
            );
            assert_sheet(
                &body,
                TAU * minor * (major * (v1 - v0) + minor * (v1.sin() - v0.sin())),
                &format!("torus band from {v0} to {v1}"),
            );
        }
        // Around the tube: between two cross-sections, also across the
        // seam u = 0 and for more than half a turn.
        for (u0, u1) in [(0.5, 1.9), (-0.8, 0.6), (4.0, 9.0)] {
            let mut body = Body::new(BodyKind::Sheet);
            let mut section = |u: f64, start: f64| {
                let outward = frame.vector(u.cos(), u.sin(), 0.0);
                let ahead = frame.vector(-u.sin(), u.cos(), 0.0);
                let centre = frame.point(major * u.cos(), major * u.sin(), 0.0);
                let plane = Frame::new(centre, -ahead, outward).unwrap();
                let curve = Curve3::Circle(Circle::new(plane, minor).unwrap());
                let vertex = body.add_vertex(curve.eval(start));
                body.add_edge(curve, (start, start + TAU), vertex, vertex)
            };
            let (first, last) = (section(u0, 0.0), section(u1, 2.5));
            sheet(
                &mut body,
                torus(),
                false,
                vec![vec![backward(first)], vec![forward(last)]],
            );
            assert_sheet(
                &body,
                TAU * minor * major * (u1 - u0),
                &format!("torus elbow from {u0} to {u1}"),
            );
        }
    }
}
