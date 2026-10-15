//! Kinks in a sweep's path, measured against the exact sweep.
//!
//! The sweep takes two pieces of a path that meet a little off tangent as
//! tangent when the profile can be turned through the kink within half
//! the tolerance (`reach (1 - cos a)`, a second-order measure), and then
//! keeps the profile square to the piece before the kink until a line
//! lays it square again. These tests check that claim from outside the
//! code: they build bodies at the largest kink the rule admits (found by
//! bisection on the sweep itself, next to the kink the rule's formula
//! gives), and measure how far the body is from the exact sweep of the
//! profile along the path as drawn, both ways:
//!
//! * **body to exact**: points of the B-rep faces (the mesh says where the
//!   faces are; each point is then projected onto the face's own surface
//!   and evaluated there) and of its edges (evaluated on their curves),
//!   with their distance to the boundary of the exact sweep;
//! * **exact to body**: points of the boundary of the exact sweep near the
//!   kinks and along the pieces after them, with an upper bound of their
//!   distance to the body's boundary: along the exact surface's normal, the
//!   farthest crossing with a face's surface that lies on that face.
//!
//! **The exact sweep** is computed here, in closed form, from the pieces as
//! drawn. A profile carried square along a path, and turned about the
//! path's binormal through each kink, sweeps the union of what it sweeps
//! along each piece and what it sweeps turning at the vertex; for a round
//! profile of radius `r` centred on the path that is the set of points
//! within `r` of the path (each piece's capsule: the ball at the vertex is
//! what the turning disc sweeps on the outside of the kink and is inside
//! the pieces' tubes elsewhere); for a rectangle `2w` wide in the path's
//! plane and `2h` high, centred on it, the points within `w` of the path in
//! plan and within `h` of its plane. The distance of a point outside to
//! that boundary is then exact (`d - r`, or the distance to a slab-box
//! corner in the `(d - w, |z| - h)` plane); inside, the depth below the
//! deepest piece is a lower bound that the crease where two pieces' tubes
//! meet at an angle `a` raises by at most `1 / cos(a / 2)`, which the
//! measure applies. The end caps are left out (the exact sweep has the
//! square caps of the path's ends where the capsules are round; the kinks
//! are far from them).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{arc, circle, line, rectangle};
use craft3d_kernel::extrude::Role;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::sweep::{Swept, sweep};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_topo::check;
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

type P = [f64; 2];

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The plane the profiles are drawn on: facing −y, its x along the
/// world's x and its y along the world's z. The paths leave the origin
/// along +y in the world's xy plane.
fn front() -> Frame {
    Frame::new(Vec3::ZERO, -Vec3::Y, Vec3::X).unwrap()
}

fn profile(curves: &[SketchCurve]) -> Profile {
    let profiles = detect_profiles(curves, &Tolerance::default()).unwrap();
    assert_eq!(profiles.len(), 1);
    profiles[0].clone()
}

fn turned(v: P, angle: f64) -> P {
    let (s, c) = angle.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}

fn left(v: P) -> P {
    [-v[1], v[0]]
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

fn norm(v: P) -> f64 {
    v[0].hypot(v[1])
}

/// The angle in `[0, 2π)`.
fn wrapped(a: f64) -> f64 {
    let a = a.rem_euclid(TAU);
    if a >= TAU { 0.0 } else { a }
}

/// A piece of the path as drawn: a line of a length, an arc of a radius
/// through an angle (counter-clockwise when positive), or a straight
/// cubic spline of a length.
#[derive(Clone, Copy, Debug)]
enum Go {
    Line(f64),
    Arc(f64, f64),
    Spline(f64),
}

/// A piece of the path in the plane, for the exact sweep.
#[derive(Clone, Copy, Debug)]
enum Piece {
    Segment {
        a: P,
        b: P,
    },
    Arc {
        centre: P,
        radius: f64,
        from: f64,
        sweep: f64,
    },
}

impl Piece {
    fn length(&self) -> f64 {
        match *self {
            Piece::Segment { a, b } => norm(sub(b, a)),
            Piece::Arc { radius, sweep, .. } => radius * sweep.abs(),
        }
    }

    /// The point `s` along the piece and the unit tangent there.
    fn at(&self, s: f64) -> (P, P) {
        match *self {
            Piece::Segment { a, b } => {
                let l = norm(sub(b, a));
                let t = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
                ([a[0] + t[0] * s, a[1] + t[1] * s], t)
            }
            Piece::Arc {
                centre,
                radius,
                from,
                sweep,
            } => {
                let side = sweep.signum();
                let angle = from + side * s / radius;
                let (sin, cos) = angle.sin_cos();
                (
                    [centre[0] + radius * cos, centre[1] + radius * sin],
                    [-side * sin, side * cos],
                )
            }
        }
    }

    /// The distance from `p` to the piece, ends included.
    fn distance(&self, p: P) -> f64 {
        match *self {
            Piece::Segment { a, b } => {
                let d = sub(b, a);
                let t = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]);
                let t = t.clamp(0.0, 1.0);
                norm(sub(p, [a[0] + d[0] * t, a[1] + d[1] * t]))
            }
            Piece::Arc {
                centre,
                radius,
                from,
                sweep,
            } => {
                let off = sub(p, centre);
                let rho = norm(off);
                let within = if rho == 0.0 {
                    true
                } else {
                    let angle = off[1].atan2(off[0]);
                    let along = if sweep > 0.0 {
                        wrapped(angle - from)
                    } else {
                        wrapped(from - angle)
                    };
                    along <= sweep.abs()
                };
                if within {
                    (rho - radius).abs()
                } else {
                    let (start, _) = self.at(0.0);
                    let (end, _) = self.at(self.length());
                    norm(sub(p, start)).min(norm(sub(p, end)))
                }
            }
        }
    }
}

/// The path from the origin along +y, piece by piece, each leaving
/// turned by its kink from the direction the one before arrives in: the
/// curves to sweep along (ids 101, 102, …) and the pieces as drawn. Every
/// length is times `k`.
fn path(pieces: &[(f64, Go)], k: f64) -> (Vec<SketchCurve>, Vec<Piece>) {
    let (mut at, mut heading) = ([0.0, 0.0], [0.0, 1.0]);
    let (mut curves, mut drawn) = (Vec::new(), Vec::new());
    for (i, &(kink, go)) in pieces.iter().enumerate() {
        let id = 101 + i as u64;
        heading = turned(heading, kink);
        match go {
            Go::Line(run) | Go::Spline(run) => {
                let end = [at[0] + heading[0] * run * k, at[1] + heading[1] * run * k];
                curves.push(if matches!(go, Go::Line(_)) {
                    line(id, at, end)
                } else {
                    let control = (0..4)
                        .map(|j| {
                            let t = run * k * f64::from(j) / 3.0;
                            [at[0] + heading[0] * t, at[1] + heading[1] * t]
                        })
                        .collect();
                    SketchCurve {
                        id,
                        curve: Curve2::Spline(Spline2 {
                            degree: 3,
                            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                            control,
                        }),
                    }
                });
                drawn.push(Piece::Segment { a: at, b: end });
                at = end;
            }
            Go::Arc(radius, angle) => {
                let r = radius * k;
                let side = angle.signum();
                let normal = left(heading);
                let centre = [at[0] + normal[0] * r * side, at[1] + normal[1] * r * side];
                let out = sub(at, centre);
                let end_out = turned(out, angle);
                let end = [centre[0] + end_out[0], centre[1] + end_out[1]];
                curves.push(if angle > 0.0 {
                    arc(id, centre, at, end)
                } else {
                    arc(id, centre, end, at)
                });
                drawn.push(Piece::Arc {
                    centre,
                    radius: r,
                    from: out[1].atan2(out[0]),
                    sweep: angle,
                });
                heading = turned(heading, angle);
                at = end;
            }
        }
    }
    (curves, drawn)
}

/// The profile: a circle of radius `r`, or a rectangle `2w` across the
/// path and `2h` high, centred on the path.
#[derive(Clone, Copy, Debug)]
enum Shape {
    Round(f64),
    Box(f64, f64),
}

impl Shape {
    fn curves(&self) -> Vec<SketchCurve> {
        match *self {
            Shape::Round(r) => vec![circle(1, [0.0, 0.0], r)],
            Shape::Box(w, h) => rectangle(1, 1, (-w, w), (-h, h)),
        }
    }

    /// How far the profile reaches to the side of the path.
    fn reach(&self) -> f64 {
        match *self {
            Shape::Round(r) => r,
            Shape::Box(w, _) => w,
        }
    }
}

/// The exact sweep of a profile along drawn pieces.
struct Exact {
    pieces: Vec<Piece>,
    shape: Shape,
    /// What a crease between two pieces' tubes adds to a depth below the
    /// deepest of them: `1 / cos(a / 2)` for the largest kink `a`.
    crease: f64,
}

impl Exact {
    fn plan_distance(&self, p: P) -> f64 {
        self.pieces
            .iter()
            .map(|piece| piece.distance(p))
            .fold(f64::INFINITY, f64::min)
    }

    /// How far `x` is from the boundary of the exact sweep (inside: at
    /// most this far).
    fn off(&self, x: Vec3) -> f64 {
        let d2 = self.plan_distance([x.x, x.y]);
        match self.shape {
            Shape::Round(r) => {
                let d = d2.hypot(x.z);
                if d >= r { d - r } else { (r - d) * self.crease }
            }
            Shape::Box(w, h) => {
                let (across, up) = (d2 - w, x.z.abs() - h);
                if across > 0.0 || up > 0.0 {
                    across.max(0.0).hypot(up.max(0.0))
                } else {
                    (-across * self.crease).min(-up)
                }
            }
        }
    }

    /// Where the samples go along piece `j`: evenly, and closer and
    /// closer to its ends; none within `margin` of the ends of the path.
    fn along(&self, j: usize, margin: f64) -> Vec<f64> {
        let piece = &self.pieces[j];
        let length = piece.length();
        let reach = self.shape.reach();
        let mut s: Vec<f64> = (0..=48).map(|i| length * f64::from(i) / 48.0).collect();
        for e in 0..14 {
            let near = reach * 10f64.powf(-f64::from(e) / 2.0);
            s.push(near);
            s.push(length - near);
        }
        s.retain(|&s| {
            (0.0..=length).contains(&s)
                && !(j == 0 && s < margin)
                && !(j + 1 == self.pieces.len() && s > length - margin)
        });
        s.sort_by(f64::total_cmp);
        s.dedup();
        s
    }

    /// Points of the boundary of the exact sweep, with its outward normal
    /// there: round the profile placed square along every piece, and
    /// turning through every kink (from square to the piece before to
    /// square to the piece after, about the vertex); none within `margin`
    /// of the ends of the path, nor inside the sweep.
    fn samples(&self, margin: f64) -> Vec<(Vec3, Vec3)> {
        // Where the profile is placed: its centre and the unit normal to
        // the path in the plane.
        let mut stations: Vec<(P, P)> = Vec::new();
        for (j, piece) in self.pieces.iter().enumerate() {
            for s in self.along(j, margin) {
                let (c, t) = piece.at(s);
                stations.push((c, left(t)));
            }
            if let Some(next) = self.pieces.get(j + 1) {
                let (v, arriving) = piece.at(piece.length());
                let (_, leaving) = next.at(0.0);
                let cross = arriving[0] * leaving[1] - arriving[1] * leaving[0];
                let dot = arriving[0] * leaving[0] + arriving[1] * leaving[1];
                let kink = cross.atan2(dot);
                for i in 0..=16 {
                    let beta = kink * f64::from(i) / 16.0;
                    stations.push((v, left(turned(arriving, beta))));
                }
            }
        }
        let around = |n: u32| (0..n).map(move |i| TAU * (f64::from(i) + 0.25) / f64::from(n));
        let chebyshev =
            |n: u32| (0..n).map(move |i| (PI * (f64::from(i) + 0.5) / f64::from(n)).cos());
        let mut out = Vec::new();
        for (c, n) in stations {
            let centre = Vec3::new(c[0], c[1], 0.0);
            let across = Vec3::new(n[0], n[1], 0.0);
            match self.shape {
                Shape::Round(r) => {
                    for theta in around(64) {
                        let (sin, cos) = theta.sin_cos();
                        let normal = across * cos + Vec3::Z * sin;
                        let x = centre + normal * r;
                        if self.plan_distance([x.x, x.y]).hypot(x.z) >= r * (1.0 - 1e-9) {
                            out.push((x, normal));
                        }
                    }
                }
                Shape::Box(w, h) => {
                    for side in [-1.0, 1.0] {
                        for z in chebyshev(16) {
                            let x = centre + across * (w * side) + Vec3::Z * (h * z);
                            if self.plan_distance([x.x, x.y]) >= w * (1.0 - 1e-9) {
                                out.push((x, across * side));
                            }
                        }
                    }
                    for lambda in chebyshev(16) {
                        for up in [-1.0, 1.0] {
                            let x = centre + across * (w * lambda) + Vec3::Z * (h * up);
                            out.push((x, Vec3::Z * up));
                        }
                    }
                }
            }
        }
        out
    }
}

/// The point of triangle `abc` nearest to `p`, by its Voronoi regions.
fn nearest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// The side faces of a swept body, their mesh, and the triangles by cell
/// of a grid, to find which face a point near the body is on.
struct Built<'a> {
    swept: &'a Swept,
    mesh: Mesh,
    chord: f64,
    cell: f64,
    cells: BTreeMap<[i64; 3], Vec<usize>>,
    /// Per face, the parameters it was last projected to.
    last: Vec<Option<(f64, f64)>>,
    /// Per face, the box round its triangles.
    boxes: Vec<Option<(Vec3, Vec3)>>,
    /// Per face, its edges.
    edges: Vec<Vec<usize>>,
    /// Per edge, the box round it.
    edge_boxes: Vec<(Vec3, Vec3)>,
}

impl<'a> Built<'a> {
    fn new(swept: &'a Swept, reach: f64) -> Built<'a> {
        let chord = 1e-3 * reach;
        let mesh = tessellate_body(
            &swept.body,
            &TessParams {
                chordal_tolerance: chord,
                max_segments: 4096,
                ..TessParams::default()
            },
        )
        .unwrap();
        let cell = reach / 4.0;
        let key = |p: Vec3| {
            [
                (p.x / cell).floor() as i64,
                (p.y / cell).floor() as i64,
                (p.z / cell).floor() as i64,
            ]
        };
        let mut cells: BTreeMap<[i64; 3], Vec<usize>> = BTreeMap::new();
        let mut boxes: Vec<Option<(Vec3, Vec3)>> = vec![None; swept.body.faces.len()];
        for t in 0..mesh.triangles.len() {
            let [a, b, c] = mesh.corners(t).unwrap();
            let (low, high) = (a.min(b).min(c), a.max(b).max(c));
            let face = mesh.face_ids[t] as usize;
            let slot = &mut boxes[face];
            *slot = Some(slot.map_or((low, high), |(l, h)| (l.min(low), h.max(high))));
            let (k0, k1) = (key(low), key(high));
            for i in k0[0]..=k1[0] {
                for j in k0[1]..=k1[1] {
                    for k in k0[2]..=k1[2] {
                        cells.entry([i, j, k]).or_default().push(t);
                    }
                }
            }
        }
        let body = &swept.body;
        let edges = body
            .faces
            .iter()
            .map(|face| {
                face.loops
                    .iter()
                    .flat_map(|&l| body.loop_(l).unwrap().coedges.iter())
                    .map(|coedge| coedge.edge.0 as usize)
                    .collect()
            })
            .collect();
        let edge_boxes = body
            .edges
            .iter()
            .map(|edge| {
                let (t0, t1) = edge.range;
                let points: Vec<Vec3> = (0..=256)
                    .map(|i| edge.curve.eval(t0 + (t1 - t0) * f64::from(i) / 256.0))
                    .collect();
                let low = points.iter().fold(points[0], |a, &b| a.min(b));
                let high = points.iter().fold(points[0], |a, &b| a.max(b));
                (low, high)
            })
            .collect();
        Built {
            swept,
            mesh,
            chord,
            cell,
            cells,
            last: vec![None; swept.body.faces.len()],
            boxes,
            edges,
            edge_boxes,
        }
    }

    fn side(&self, face: usize) -> bool {
        self.swept.faces[face].role == Role::Side
    }

    /// Whether `x` is within `within` of the mesh of `face`.
    fn on_face(&self, x: Vec3, face: usize, within: f64) -> bool {
        let k = [
            (x.x / self.cell).floor() as i64,
            (x.y / self.cell).floor() as i64,
            (x.z / self.cell).floor() as i64,
        ];
        for i in -1..=1 {
            for j in -1..=1 {
                for l in -1..=1 {
                    let Some(list) = self.cells.get(&[k[0] + i, k[1] + j, k[2] + l]) else {
                        continue;
                    };
                    for &t in list {
                        if self.mesh.face_ids[t] as usize != face {
                            continue;
                        }
                        let [a, b, c] = self.mesh.corners(t).unwrap();
                        if nearest_on_triangle(x, a, b, c).distance(x) <= within {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// The centre of the triangle of `face` nearest to `x`.
    fn nearest_centre(&self, x: Vec3, face: usize) -> Option<Vec3> {
        let k = [
            (x.x / self.cell).floor() as i64,
            (x.y / self.cell).floor() as i64,
            (x.z / self.cell).floor() as i64,
        ];
        let mut best: Option<(f64, Vec3)> = None;
        for i in -1..=1 {
            for j in -1..=1 {
                for l in -1..=1 {
                    let Some(list) = self.cells.get(&[k[0] + i, k[1] + j, k[2] + l]) else {
                        continue;
                    };
                    for &t in list {
                        if self.mesh.face_ids[t] as usize != face {
                            continue;
                        }
                        let [a, b, c] = self.mesh.corners(t).unwrap();
                        let d = nearest_on_triangle(x, a, b, c).distance(x);
                        if best.is_none_or(|(e, _)| d < e) {
                            best = Some((d, (a + b + c) * (1.0 / 3.0)));
                        }
                    }
                }
            }
        }
        best.map(|(_, centre)| centre)
    }

    /// Whether `x`, a point of the surface of `face`, is on the face: near
    /// its mesh, and on the face's side of each of its edges that it is
    /// near (the side is measured exactly, square to the edge in the
    /// surface; which side is the face's, the mesh says).
    fn inside(&mut self, x: Vec3, face: usize, within: f64) -> bool {
        self.on_face(x, face, within) && self.within_edges(x, face, within)
    }

    /// Whether `x`, a point of the surface of `face` near its mesh, is on
    /// the face's side of each edge of it that it is near.
    fn within_edges(&mut self, x: Vec3, face: usize, within: f64) -> bool {
        let guard = 20.0 * within;
        for e in self.edges[face].clone() {
            let (low, high) = self.edge_boxes[e];
            let outside = |q: f64, l: f64, h: f64| q < l - guard || q > h + guard;
            if outside(x.x, low.x, high.x)
                || outside(x.y, low.y, high.y)
                || outside(x.z, low.z, high.z)
            {
                continue;
            }
            let edge = &self.swept.body.edges[e];
            let Ok(t) = edge.curve.project(x) else {
                continue;
            };
            let (t0, t1) = edge.range;
            let on = match edge.curve.period() {
                Some(period) if (t1 - t0).abs() >= period - 1e-12 => true,
                Some(period) => (t - t0.min(t1)).rem_euclid(period) <= (t1 - t0).abs() + 1e-12,
                None => (t0.min(t1)..=t0.max(t1)).contains(&t),
            };
            if !on {
                continue;
            }
            let (point, tangent) = edge.curve.derivatives(t);
            if point.distance(x) > guard {
                continue;
            }
            let Some(uv) = self.project(face, point, guard) else {
                continue;
            };
            let surface = &self.swept.body.faces[face].surface;
            let Some(across) = surface
                .normal(uv.0, uv.1)
                .and_then(|n| n.cross(tangent).normalized())
            else {
                continue;
            };
            let Some(centre) = self.nearest_centre(point, face) else {
                continue;
            };
            let inward = (centre - point).dot(across).signum();
            // On the edge itself, to rounding, is on both faces.
            if (x - point).dot(across) * inward < -1e-6 * within {
                return false;
            }
        }
        true
    }

    /// The parameters of the point of `face`'s surface nearest to `x`,
    /// which is known to be within `near` of it: a walk from where the
    /// face was last projected to, or a search when that lands farther.
    fn project(&mut self, face: usize, x: Vec3, near: f64) -> Option<(f64, f64)> {
        let surface = &self.swept.body.faces[face].surface;
        if let Some(start) = self.last[face]
            && let Ok(uv) = surface.project_from(x, start)
            && surface.eval(uv.0, uv.1).distance(x) <= near
        {
            self.last[face] = Some(uv);
            return Some(uv);
        }
        let uv = surface.project(x).ok()?;
        self.last[face] = Some(uv);
        Some(uv)
    }

    /// Points of the side faces: inside each triangle of the mesh, moved
    /// onto the face's surface; and points of every edge, on its curve.
    /// The largest `measure` of them, and where.
    fn body_to_exact(&mut self, measure: impl Fn(Vec3) -> f64) -> (f64, Vec3, usize) {
        let (mut worst, mut at, mut count) = (0.0_f64, Vec3::ZERO, 0);
        const N: u32 = 4;
        for t in 0..self.mesh.triangles.len() {
            let face = self.mesh.face_ids[t] as usize;
            if !self.side(face) {
                continue;
            }
            let [a, b, c] = self.mesh.corners(t).unwrap();
            for i in 0..=N {
                for j in 0..=(N - i) {
                    let (u, v) = (f64::from(i) / f64::from(N), f64::from(j) / f64::from(N));
                    let m = a + (b - a) * u + (c - a) * v;
                    let near = 4.0 * self.chord;
                    let uv = self.project(face, m, near).unwrap();
                    let p = self.swept.body.faces[face].surface.eval(uv.0, uv.1);
                    assert!(
                        p.distance(m) <= near,
                        "the mesh of face {face} is {} from it",
                        p.distance(m)
                    );
                    // A triangle along a curved edge in the surface
                    // reaches past it by its chord.
                    if !self.within_edges(p, face, 2.0 * self.chord) {
                        continue;
                    }
                    let off = measure(p);
                    count += 1;
                    if off.is_nan() || off > worst {
                        (worst, at) = (off, p);
                    }
                }
            }
        }
        for edge in &self.swept.body.edges {
            let (t0, t1) = edge.range;
            for i in 0..=256 {
                let p = edge.curve.eval(t0 + (t1 - t0) * f64::from(i) / 256.0);
                let off = measure(p);
                count += 1;
                if off.is_nan() || off > worst {
                    (worst, at) = (off, p);
                }
            }
        }
        (worst, at, count)
    }

    /// Where the line from `q` along `n` crosses the surface of `face`
    /// within `far` of `q`: the signed distance along it.
    fn crossing(&mut self, face: usize, q: Vec3, n: Vec3, far: f64, exact: f64) -> Option<f64> {
        let mut t = 0.0;
        for _ in 0..60 {
            let x = q + n * t;
            let uv = self.project(face, x, far)?;
            let p = self.swept.body.faces[face].surface.eval(uv.0, uv.1);
            let gap = p - x;
            if gap.length() <= exact {
                return Some(t);
            }
            t += gap.dot(n);
            if t.is_nan() || t.abs() > far {
                return None;
            }
        }
        None
    }

    /// For each point of the exact boundary, the farthest crossing of the
    /// line along its normal with a side face's surface that lies on
    /// that face (the crossing with the body's boundary is one of them,
    /// so this is no less than how far the point is from the body). The
    /// largest of them, and where.
    fn exact_to_body(&mut self, samples: &[(Vec3, Vec3)], reach: f64) -> (f64, Vec3) {
        let (far, exact) = (1e-3 * reach, 1e-12 * reach);
        let within = 2.0 * self.chord + 1e-9 * reach;
        let (mut worst, mut at) = (0.0_f64, Vec3::ZERO);
        for &(q, n) in samples {
            let mut found: Option<f64> = None;
            for face in 0..self.swept.body.faces.len() {
                if !self.side(face) {
                    continue;
                }
                let Some((low, high)) = self.boxes[face] else {
                    continue;
                };
                let outside = |q: f64, low: f64, high: f64| q < low - far || q > high + far;
                if outside(q.x, low.x, high.x)
                    || outside(q.y, low.y, high.y)
                    || outside(q.z, low.z, high.z)
                {
                    continue;
                }
                let Some(t) = self.crossing(face, q, n, far, exact) else {
                    continue;
                };
                if self.inside(q + n * t, face, within) {
                    found = Some(found.map_or(t.abs(), |f: f64| f.max(t.abs())));
                }
            }
            let off = found.unwrap_or(f64::INFINITY);
            if off.is_nan() || off > worst {
                (worst, at) = (off, q);
            }
        }
        (worst, at)
    }
}

/// The kink that turns a profile reaching `reach` to the side through
/// `moved`: `reach (1 - cos a) = moved`.
fn kink_for(moved: f64, reach: f64) -> f64 {
    2.0 * (0.5 * moved / reach).sqrt().asin()
}

/// The paths: line, arc, line and so on, with the kink `a` where the
/// name says.
fn cases(a: f64) -> Vec<(&'static str, Vec<(f64, Go)>)> {
    let quarter = FRAC_PI_2;
    vec![
        (
            "line→arc",
            vec![
                (0.0, Go::Line(10.0)),
                (a, Go::Arc(5.0, quarter)),
                (0.0, Go::Line(12.0)),
            ],
        ),
        (
            "arc→line",
            vec![
                (0.0, Go::Line(10.0)),
                (0.0, Go::Arc(5.0, quarter)),
                (-a, Go::Line(12.0)),
            ],
        ),
        (
            "arc→arc",
            vec![
                (0.0, Go::Line(10.0)),
                (0.0, Go::Arc(5.0, quarter)),
                (a, Go::Arc(5.0, -quarter)),
                (0.0, Go::Line(12.0)),
            ],
        ),
        (
            "arc→spline",
            vec![
                (0.0, Go::Line(10.0)),
                (0.0, Go::Arc(5.0, quarter)),
                (-a, Go::Spline(9.0)),
            ],
        ),
    ]
}

/// Measures one body against the exact sweep both ways.
fn measured(swept: &Swept, pieces: Vec<Piece>, shape: Shape, kink: f64) -> (f64, f64, usize) {
    let reach = shape.reach();
    let exact = Exact {
        pieces,
        shape,
        crease: 1.0 / (kink / 2.0).cos(),
    };
    let mut built = Built::new(swept, reach);
    let (there, where_there, count) = built.body_to_exact(|p| exact.off(p));
    let samples = exact.samples(1.5 * reach);
    let (back, where_back) = built.exact_to_body(&samples, reach);
    eprintln!("worst body→exact at {where_there:?}, exact→body at {where_back:?}");
    (there, back, count + samples.len())
}

/// The developer's condition for the second-order kink rule
/// (2026-10-15): measured, not argued. For line→arc, arc→line, arc→arc
/// and arc→spline kinks, round and rectangular profiles, at three scales
/// with the tolerance scaled along: the largest kink the sweep admits
/// (by bisection on the sweep itself; at most the kink of the rule,
/// `reach (1 - cos a) = tol / 2`, where carrying the profile round an arc
/// tilted asks for less) gives a valid body whose faces and edges are
/// within the tolerance of the exact sweep, and every point of the exact
/// sweep's boundary near the kinks and along the pieces after them is
/// within the tolerance of the body. A kink 1 % larger is refused.
///
/// Measured (2026-10-15), at every scale alike: half the tolerance both
/// ways, to four digits, at the largest kink (the rule's own for
/// line→arc, arc→line and arc→spline, 0.894 of it for arc→arc, where an
/// arc of radius 5 carries a profile reaching 1 turned).
fn largest_kinks_measured(k: f64) {
    let mut table = Vec::new();
    {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for shape in [Shape::Round(k), Shape::Box(k, 0.5 * k)] {
            let reach = shape.reach();
            let section = profile(&shape.curves());
            let rule = kink_for(0.5 * tol.linear, reach);
            for (i, (what, _)) in cases(0.0).iter().enumerate() {
                let build = |a: f64| {
                    let (curves, pieces) = path(&cases(a)[i].1, k);
                    (
                        sweep(&section, &front(), &curves, &Frame::WORLD, &tol),
                        pieces,
                    )
                };
                assert!(build(0.0).0.is_ok(), "{k} {shape:?} {what}: tangent");
                assert!(build(2.0 * rule).0.is_err(), "{k} {shape:?} {what}: twice");
                let (mut low, mut high) = (0.0, 2.0 * rule);
                for _ in 0..16 {
                    let middle = 0.5 * (low + high);
                    if build(middle).0.is_ok() {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                let largest = low;
                assert!(
                    largest <= rule * (1.0 + 1e-9) && largest >= 0.5 * rule,
                    "{k} {shape:?} {what}: the largest kink admitted is {largest}, the rule's {rule}"
                );
                assert!(
                    build(1.01 * largest).0.is_err(),
                    "{k} {shape:?} {what}: 1 % above the largest is swept"
                );
                let (swept, pieces) = build(largest);
                let swept = swept.unwrap();
                assert_eq!(check(&swept.body, &tol), vec![]);
                let (there, back, count) = measured(&swept, pieces, shape, largest);
                table.push(format!(
                    "{k:e} {shape:?} {what}: kink {largest:.4e} ({:.4} of the rule's), body→exact {:.4} tol, exact→body {:.4} tol ({count} points)",
                    largest / rule,
                    there / tol.linear,
                    back / tol.linear
                ));
                assert!(
                    there <= tol.linear && back <= tol.linear,
                    "{}",
                    table.last().unwrap()
                );
                // Not blind to the kink: at the largest one the body is
                // that far off.
                assert!(there >= 0.25 * tol.linear, "{}", table.last().unwrap());
            }
        }
    }
    eprintln!("{}", table.join("\n"));
}

#[test]
fn the_largest_kink_admitted_is_within_the_tolerance_of_the_exact_sweep_small() {
    largest_kinks_measured(SCALES[0]);
}

#[test]
fn the_largest_kink_admitted_is_within_the_tolerance_of_the_exact_sweep() {
    largest_kinks_measured(SCALES[1]);
}

#[test]
fn the_largest_kink_admitted_is_within_the_tolerance_of_the_exact_sweep_large() {
    largest_kinks_measured(SCALES[2]);
}

/// The measure sees a corner: two lines that meet at a kink of 0.02
/// radians are mitred, and the mitre's outer corner reaches
/// `reach (1 / cos(a / 2) - 1)` beyond the round of the exact sweep (the
/// profile turned through the corner), both ways.
#[test]
fn the_measure_sees_a_mitre() {
    let a = 0.02;
    for k in SCALES {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for shape in [Shape::Round(k), Shape::Box(k, 0.5 * k)] {
            let (curves, pieces) = path(&[(0.0, Go::Line(10.0)), (a, Go::Line(12.0))], k);
            let swept = sweep(
                &profile(&shape.curves()),
                &front(),
                &curves,
                &Frame::WORLD,
                &tol,
            )
            .unwrap();
            let (there, back, _) = measured(&swept, pieces, shape, a);
            let expected = shape.reach() * (1.0 / (a / 2.0).cos() - 1.0);
            assert!(expected > 40.0 * tol.linear);
            for (found, way) in [(there, "body→exact"), (back, "exact→body")] {
                assert!(
                    (found - expected).abs() <= 0.02 * expected,
                    "{k} {shape:?} {way}: {found}, expected {expected}"
                );
            }
        }
    }
}
