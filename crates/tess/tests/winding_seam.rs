//! Faces that go around the axis of their surface with holes that leave
//! no room for a straight seam: two rows of windows, the second row
//! offset by half a pitch, each window wider than half a pitch. Every
//! line of constant angle from the lower boundary to the upper one runs
//! into a window, so the face has to be cut open along a seam in pieces
//! that passes *through* windows (a turbine hub with seven blade roots
//! that together cover the whole circumference was the model that asked
//! for this).
//!
//! Sheets with one face, on a cylinder, a cone (also from its apex), a
//! sphere (a zone, a cap up to a pole, and the whole sphere but for the
//! windows) and a torus (around the axis and around the tube), each as
//! the surface has it and reversed, in two frames and at three sizes.
//!
//! The oracle for each is the closed-form area, the tolerance measured on
//! every triangle, the way every triangle faces, and the mesh's own
//! boundary: after welding, the only edges used by one triangle are the
//! chords of the face's edges, so a seam whose two sides do not share
//! their points, or a window put on the wrong side of the cut, shows up
//! as open edges or as area that is not the face's.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Cone, Curve3, Cylinder, Line, Sphere, Surface, Torus};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessError, TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    }
}

/// The world frame, and one that is square to nothing. Its origin grows
/// with the model; its directions do not.
fn frames(size: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * size,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap(),
    ]
}

fn bits(p: &Vec3) -> [u64; 3] {
    [p.x, p.y, p.z].map(f64::to_bits)
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

#[derive(Clone, Copy, Debug)]
enum Kind {
    Cylinder,
    /// With the radius at `v = 0` over the model's size.
    Cone(f64),
    Sphere,
    Torus,
}

const HALF_ANGLE: f64 = 0.4;

/// A surface of revolution about the z axis of `frame`, `size` large:
/// longitude `u`, and `v` as the surface has it (a length along the axis
/// on a cylinder and a cone, an angle on a sphere and a torus).
#[derive(Clone, Copy)]
struct Wall {
    frame: Frame,
    kind: Kind,
    size: f64,
}

impl Wall {
    fn surface(&self) -> Surface {
        let s = self.size;
        match self.kind {
            Kind::Cylinder => Surface::Cylinder(Cylinder::new(self.frame, 2.0 * s).unwrap()),
            Kind::Cone(r) => Surface::Cone(Cone::new(self.frame, r * s, HALF_ANGLE).unwrap()),
            Kind::Sphere => Surface::Sphere(Sphere::new(self.frame, 2.0 * s).unwrap()),
            Kind::Torus => Surface::Torus(Torus::new(self.frame, 3.0 * s, s).unwrap()),
        }
    }

    /// The radius and the height of the circle of latitude at `v`.
    fn level(&self, v: f64) -> (f64, f64) {
        let s = self.size;
        match self.kind {
            Kind::Cylinder => (2.0 * s, v),
            Kind::Cone(r) => (r * s + v * HALF_ANGLE.ptan(), v),
            Kind::Sphere => (2.0 * s * v.pcos(), 2.0 * s * v.psin()),
            Kind::Torus => (3.0 * s + s * v.pcos(), s * v.psin()),
        }
    }

    /// The edge along the circle of latitude at `v` from `u.0` to `u.1`.
    fn parallel(&self, body: &mut Body, v: f64, u: (f64, f64)) -> EdgeId {
        let (radius, z) = self.level(v);
        let frame = &self.frame;
        let level = Frame::new(frame.point(0.0, 0.0, z), frame.z(), frame.x()).unwrap();
        let curve = Curve3::Circle(Circle::new(level, radius).unwrap());
        let start = body.add_vertex(curve.eval(u.0));
        let end = if u.1 - u.0 >= TAU - 1e-12 {
            start
        } else {
            body.add_vertex(curve.eval(u.1))
        };
        body.add_edge(curve, (u.0, u.1), start, end)
    }

    /// The curve of constant longitude `u`, and the parameters on it of
    /// the latitudes `v.0` and `v.1`.
    fn meridian_curve(&self, u: f64, v: (f64, f64)) -> (Curve3, (f64, f64)) {
        let frame = &self.frame;
        let outward = frame.vector(u.pcos(), u.psin(), 0.0);
        let ahead = frame.vector(-u.psin(), u.pcos(), 0.0);
        let round = |centre: Vec3, radius: f64| {
            let plane = Frame::new(centre, -ahead, outward).unwrap();
            (Curve3::Circle(Circle::new(plane, radius).unwrap()), v)
        };
        match self.kind {
            Kind::Sphere => round(frame.origin(), 2.0 * self.size),
            Kind::Torus => {
                let major = 3.0 * self.size;
                round(
                    frame.point(major * u.pcos(), major * u.psin(), 0.0),
                    self.size,
                )
            }
            Kind::Cylinder | Kind::Cone(_) => {
                let at = |v: f64| {
                    let (r, z) = self.level(v);
                    frame.point(r * u.pcos(), r * u.psin(), z)
                };
                let (p, q) = (at(v.0), at(v.1));
                (
                    Curve3::Line(Line::through(p, q).unwrap()),
                    (0.0, p.distance(q)),
                )
            }
        }
    }

    /// The edge along the meridian at `u` between the start (or end) of
    /// one parallel and the start (or end) of another.
    fn meridian(
        &self,
        body: &mut Body,
        u: f64,
        v: (f64, f64),
        from: (EdgeId, bool),
        to: (EdgeId, bool),
    ) -> EdgeId {
        let vertex = |body: &Body, (edge, at_end): (EdgeId, bool)| {
            let e = body.edge(edge).unwrap();
            if at_end { e.end } else { e.start }
        };
        let (a, b) = (vertex(body, from), vertex(body, to));
        let (curve, range) = self.meridian_curve(u, v);
        body.add_edge(curve, range, a, b)
    }

    /// A whole circle around the tube of a torus at longitude `u`.
    fn section(&self, body: &mut Body, u: f64, start: f64) -> EdgeId {
        let (curve, range) = self.meridian_curve(u, (start, start + TAU));
        let vertex = body.add_vertex(curve.eval(start));
        body.add_edge(curve, range, vertex, vertex)
    }

    /// A window between the longitudes `u` and the latitudes `v`, as a
    /// hole: clockwise seen from outside.
    fn window(&self, body: &mut Body, u: (f64, f64), v: (f64, f64)) -> Vec<Coedge> {
        let low = self.parallel(body, v.0, u);
        let high = self.parallel(body, v.1, u);
        let right = self.meridian(body, u.1, v, (low, true), (high, true));
        let left = self.meridian(body, u.0, v, (low, false), (high, false));
        vec![forward(left), forward(high), backward(right), backward(low)]
    }

    /// The area between two longitudes `du` apart and the latitudes `v`.
    fn area(&self, du: f64, v: (f64, f64)) -> f64 {
        let s = self.size;
        let (v0, v1) = v;
        match self.kind {
            Kind::Cylinder => 2.0 * s * du * (v1 - v0),
            Kind::Cone(r) => {
                du * (r * s * (v1 - v0) + HALF_ANGLE.ptan() * (v1 * v1 - v0 * v0) / 2.0)
                    / HALF_ANGLE.pcos()
            }
            Kind::Sphere => 4.0 * s * s * du * (v1.psin() - v0.psin()),
            Kind::Torus => du * s * (3.0 * s * (v1 - v0) + s * (v1.psin() - v0.psin())),
        }
    }
}

/// `x` of the way from `range.0` to `range.1`.
fn part(range: (f64, f64), x: f64) -> f64 {
    range.0 + (range.1 - range.0) * x
}

/// Two rows of `count` windows around the axis between the latitudes
/// `v`, the second row half a pitch further, every window 0.6 of a pitch
/// wide: no longitude is free of them. Returns their loops and their
/// area.
fn staggered(wall: &Wall, body: &mut Body, v: (f64, f64), count: usize) -> (Vec<Vec<Coedge>>, f64) {
    let pitch = TAU / count as f64;
    let mut loops = Vec::new();
    let mut area = 0.0;
    for (row, rows_v) in [(0.15, 0.4), (0.6, 0.85)].into_iter().enumerate() {
        let rows_v = (part(v, rows_v.0), part(v, rows_v.1));
        for k in 0..count {
            let middle = 0.37 + pitch * (k as f64 + 0.5 * row as f64);
            let u = (middle - 0.3 * pitch, middle + 0.3 * pitch);
            loops.push(wall.window(body, u, rows_v));
            area += wall.area(u.1 - u.0, rows_v);
        }
    }
    (loops, area)
}

/// The same loops for the face seen from the other side.
fn turned(loops: Vec<Vec<Coedge>>) -> Vec<Vec<Coedge>> {
    loops
        .into_iter()
        .map(|l| {
            l.into_iter()
                .rev()
                .map(|c| Coedge {
                    edge: c.edge,
                    reversed: !c.reversed,
                })
                .collect()
        })
        .collect()
}

fn sheet(surface: Surface, mut body: Body, loops: Vec<Vec<Coedge>>, reversed: bool) -> Body {
    let loops = if reversed { turned(loops) } else { loops };
    let loops = loops.into_iter().map(|l| body.add_loop(l)).collect();
    let face = body.add_face(surface, reversed, loops);
    body.add_shell(vec![face]);
    body
}

/// What a face of these tests is, to build it in every variant.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// Between two circles of latitude.
    Band(Kind, (f64, f64)),
    /// From a circle of latitude to the upper end of the surface.
    UpToTheEnd(Kind, (f64, f64)),
    /// From the lower end of the surface to a circle of latitude.
    FromTheEnd(Kind, (f64, f64)),
    /// A whole sphere but for the windows.
    WholeSphere,
    /// A torus between two cross-sections of its tube, with rows of
    /// windows around the tube.
    AroundTheTube,
}

/// The sheet, its area in closed form and a name for it.
fn build(shape: Shape, frame: Frame, size: f64, count: usize, reversed: bool) -> (Body, f64) {
    let mut body = Body::new(BodyKind::Sheet);
    // Latitudes that are lengths grow with the model.
    let lengths = |kind: Kind, v: (f64, f64)| match kind {
        Kind::Cylinder | Kind::Cone(_) => (v.0 * size, v.1 * size),
        Kind::Sphere | Kind::Torus => v,
    };
    let (wall, loops, area) = match shape {
        Shape::Band(kind, v) => {
            let wall = Wall { frame, kind, size };
            let v = lengths(kind, v);
            let low = wall.parallel(&mut body, v.0, (0.0, TAU));
            let high = wall.parallel(&mut body, v.1, (3.0, 3.0 + TAU));
            let (mut loops, windows) = staggered(&wall, &mut body, v, count);
            loops.push(vec![forward(low)]);
            loops.push(vec![backward(high)]);
            (wall, loops, wall.area(TAU, v) - windows)
        }
        Shape::UpToTheEnd(kind, v) => {
            let wall = Wall { frame, kind, size };
            let v = lengths(kind, v);
            let low = wall.parallel(&mut body, v.0, (1.0, 1.0 + TAU));
            let (mut loops, windows) = staggered(&wall, &mut body, v, count);
            loops.insert(0, vec![forward(low)]);
            (wall, loops, wall.area(TAU, v) - windows)
        }
        Shape::FromTheEnd(kind, v) => {
            let wall = Wall { frame, kind, size };
            let v = lengths(kind, v);
            let high = wall.parallel(&mut body, v.1, (2.0, 2.0 + TAU));
            let (mut loops, windows) = staggered(&wall, &mut body, v, count);
            loops.push(vec![backward(high)]);
            (wall, loops, wall.area(TAU, v) - windows)
        }
        Shape::WholeSphere => {
            let wall = Wall {
                frame,
                kind: Kind::Sphere,
                size,
            };
            let (loops, windows) = staggered(&wall, &mut body, (-1.2, 1.2), count);
            (
                wall,
                loops,
                wall.area(TAU, (-FRAC_PI_2, FRAC_PI_2)) - windows,
            )
        }
        Shape::AroundTheTube => {
            let wall = Wall {
                frame,
                kind: Kind::Torus,
                size,
            };
            let u = (0.5, 2.9);
            let first = wall.section(&mut body, u.0, 0.0);
            let last = wall.section(&mut body, u.1, 2.5);
            let pitch = TAU / count as f64;
            let mut loops = vec![vec![backward(first)], vec![forward(last)]];
            let mut area = wall.area(u.1 - u.0, (0.0, TAU));
            for (row, row_u) in [(0.15, 0.4), (0.6, 0.85)].into_iter().enumerate() {
                let row_u = (part(u, row_u.0), part(u, row_u.1));
                for k in 0..count {
                    let middle = 0.37 + pitch * (k as f64 + 0.5 * row as f64);
                    let v = (middle - 0.3 * pitch, middle + 0.3 * pitch);
                    loops.push(wall.window(&mut body, row_u, v));
                    area -= wall.area(row_u.1 - row_u.0, v);
                }
            }
            (wall, loops, area)
        }
    };
    (sheet(wall.surface(), body, loops, reversed), area)
}

/// Meshes the sheet at three tolerances and checks: every vertex on the
/// surface, every triangle within tolerance, with area, facing the way
/// the face does; the mesh's open edges are exactly the chords of the
/// body's edges; the area is the closed form within what the tolerance
/// explains and converges to it; and the mesh is the same bit for bit on
/// a second run, on one thread and on four. Returns the area errors.
fn assert_sheet(body: &Body, expected: f64, size: f64, what: &str) -> Vec<f64> {
    assert_eq!(
        craft3d_topo::check(body, &craft3d_math::Tolerance::default()),
        vec![],
        "{what}"
    );
    let face = &body.faces[0];
    let near = 1e-9 * size;
    let mut errors = Vec::new();
    for relative in [3e-2, 3e-3, 3e-4] {
        let tolerance = relative * size;
        let mesh = tessellate_body(body, &params(tolerance))
            .unwrap_or_else(|e| panic!("{what} at {tolerance}: {e}"));
        for threads in [0, 1, 4] {
            let again = tessellate_body_with_threads(body, &params(tolerance), threads).unwrap();
            let same = mesh.triangles == again.triangles
                && mesh.face_ids == again.face_ids
                && mesh.positions.len() == again.positions.len()
                && mesh
                    .positions
                    .iter()
                    .zip(&again.positions)
                    .all(|(p, q)| bits(p) == bits(q));
            assert!(
                same,
                "{what} at {tolerance}: not the same on {threads} threads"
            );
        }
        for t in 0..mesh.triangles.len() {
            let [a, b, c] = mesh.corners(t).unwrap();
            let centroid = (a + b + c) * (1.0 / 3.0);
            for p in [a, b, c] {
                let (u, v) = face.surface.project(p).unwrap();
                assert!(
                    face.surface.eval(u, v).distance(p) < near,
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
            "{what} at {tolerance}: an edge is used twice the same way"
        );
        let open: Vec<(u32, u32)> = uses
            .keys()
            .filter(|(a, b)| !uses.contains_key(&(*b, *a)))
            .copied()
            .collect();
        let boundary_points: BTreeSet<[u64; 3]> = open
            .iter()
            .flat_map(|&(a, b)| [a, b])
            .map(|k| welded.positions[k as usize].map(f64::to_bits))
            .collect();
        assert_eq!(
            open.len(),
            boundary_points.len(),
            "{what} at {tolerance}: the open edges do not form simple closed loops"
        );
        // The open edges are the chords of the body's edges, no more and
        // no fewer: every point of every edge is on the mesh's boundary,
        // and every open edge lies on one of the edges.
        let edge_points: BTreeSet<[u64; 3]> =
            craft3d_tess::tessellate_edges(body, &params(tolerance))
                .unwrap()
                .iter()
                .flatten()
                .map(bits)
                .collect();
        assert_eq!(
            boundary_points, edge_points,
            "{what} at {tolerance}: the mesh's boundary is not the points of the face's edges \
             (a seam did not close, or a window is on the wrong side of it)"
        );
        for (a, b) in &open {
            for k in [a, b] {
                let [x, y, z] = welded.positions[*k as usize];
                let p = Vec3::new(x, y, z);
                let on_an_edge = body.edges.iter().any(|e| {
                    let t = e.curve.project(p).unwrap();
                    e.curve.eval(t).distance(p) < near
                });
                assert!(
                    on_an_edge,
                    "{what} at {tolerance}: an open edge off the boundary"
                );
            }
        }
        let error = (mesh.area() - expected).abs() / expected;
        println!(
            "{what} at {relative:e}: {} triangles, area error {error:e}",
            mesh.triangles.len()
        );
        // A chord that sags by the tolerance over a radius of the model's
        // size shortens its arc by about two thirds of their ratio.
        assert!(
            error <= 2.0 * relative,
            "{what} at {tolerance}: area error {error:e}"
        );
        assert!(
            errors.last().is_none_or(|previous| error <= *previous),
            "{what}: area error grew to {error:e}"
        );
        errors.push(error);
    }
    assert!(
        errors.last().is_some_and(|e| *e < 1e-3),
        "{what}: {errors:?}"
    );
    errors
}

fn shapes() -> Vec<(&'static str, Shape)> {
    vec![
        ("cylinder wall", Shape::Band(Kind::Cylinder, (0.0, 3.0))),
        ("cone frustum", Shape::Band(Kind::Cone(2.0), (0.0, 3.0))),
        ("sphere zone", Shape::Band(Kind::Sphere, (-1.0, 1.0))),
        ("torus outer band", Shape::Band(Kind::Torus, (-1.2, 1.2))),
        // Across the seam v = 0 of the tube and beyond a whole turn.
        ("torus inner band", Shape::Band(Kind::Torus, (2.0, 4.4))),
        (
            "torus band above a turn",
            Shape::Band(Kind::Torus, (5.5, 7.5)),
        ),
        (
            "sphere cap up to its pole",
            Shape::UpToTheEnd(Kind::Sphere, (-0.2, FRAC_PI_2)),
        ),
        (
            "sphere from its pole",
            Shape::FromTheEnd(Kind::Sphere, (-FRAC_PI_2, 0.3)),
        ),
        (
            "cone from its apex",
            Shape::FromTheEnd(Kind::Cone(0.0), (0.0, 3.0)),
        ),
        ("whole sphere", Shape::WholeSphere),
        ("torus around its tube", Shape::AroundTheTube),
    ]
}

fn every_shape(size: f64) {
    // Every variant is tried, so that a failure says which ones fail.
    let mut failed = Vec::new();
    for (name, shape) in shapes() {
        for (f, frame) in frames(size).into_iter().enumerate() {
            for reversed in [false, true] {
                for count in [3, 7] {
                    let (body, area) = build(shape, frame, size, count, reversed);
                    let what = format!(
                        "{name}, 2 x {count} windows, frame {f}, size {size:e}{}",
                        if reversed { ", reversed" } else { "" }
                    );
                    let run = std::panic::catch_unwind(|| assert_sheet(&body, area, size, &what));
                    if run.is_err() {
                        failed.push(what);
                    }
                }
            }
        }
    }
    assert!(failed.is_empty(), "failed: {failed:#?}");
}

#[test]
fn staggered_windows_at_unit_size() {
    every_shape(1.0);
}

#[test]
fn staggered_windows_at_a_small_size() {
    every_shape(1e-3);
}

#[test]
fn staggered_windows_at_a_large_size() {
    every_shape(1e3);
}

/// The windows really are in the way of every straight seam: the area
/// they cover, summed over the longitudes, leaves no gap. (A guard for
/// the fixture: with narrower windows these tests would pass without a
/// seam in pieces.)
#[test]
fn the_windows_cover_every_longitude() {
    for count in [3_usize, 7] {
        let pitch = TAU / count as f64;
        for step in 0..3600 {
            let u = TAU * step as f64 / 3600.0;
            let covered = (0..2).any(|row| {
                (0..count).any(|k| {
                    let middle = 0.37 + pitch * (k as f64 + 0.5 * row as f64);
                    let away = (u - middle + PI).rem_euclid(TAU) - PI;
                    away.abs() < 0.3 * pitch
                })
            });
            assert!(covered, "longitude {u} is free with {count} windows a row");
        }
    }
}

/// Many small windows: the search for the seam is not what the face's
/// time goes to.
#[test]
fn many_windows() {
    let (body, area) = build(
        Shape::Band(Kind::Cylinder, (0.0, 3.0)),
        Frame::WORLD,
        1.0,
        60,
        false,
    );
    let started = std::time::Instant::now();
    let mesh = tessellate_body(&body, &params(1e-3)).unwrap();
    let error = (mesh.area() - area).abs() / area;
    println!(
        "120 windows: {} triangles in {:?}, area error {error:e}",
        mesh.triangles.len(),
        started.elapsed()
    );
    assert!(error < 1e-3, "{error:e}");
}

/// A face that cannot be cut open is still an error, not a crash: two
/// loops that wind the same way.
#[test]
fn loops_that_bound_nothing_are_refused() {
    let wall = Wall {
        frame: Frame::WORLD,
        kind: Kind::Cylinder,
        size: 1.0,
    };
    let mut body = Body::new(BodyKind::Sheet);
    let low = wall.parallel(&mut body, 0.0, (0.0, TAU));
    let high = wall.parallel(&mut body, 3.0, (0.0, TAU));
    let (mut loops, _) = staggered(&wall, &mut body, (0.0, 3.0), 5);
    loops.push(vec![forward(low)]);
    loops.push(vec![forward(high)]);
    let body = sheet(wall.surface(), body, loops, false);
    assert!(matches!(
        tessellate_body(&body, &params(1e-2)),
        Err(TessError::Face { .. })
    ));
}
