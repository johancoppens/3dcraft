//! Fillets against closed forms (docs/testing.md, oracle 1): every result
//! passes the checker, has the volume, area and counts that follow from
//! the sizes, at three scales, and every fillet face is tangent to its
//! neighbours along the seams, measured here from the surfaces.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::{Rng, SCALES, assert_close, boxed, mass, tolerance};
use common::{arc, frames, line, polygon, rounded_polygon, scale_curves};
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean, contains_points};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, FilletFace, FilletResult, fillet, tangent_chain};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check, genus};
use std::f64::consts::PI;

/// What a square corner loses to a round of radius 1: the square less the
/// quarter disc.
const SPANDREL: f64 = 1.0 - PI / 4.0;

/// The edge whose middle is nearest `middle`, given in `frame` at scale 1.
fn edge_in(body: &Body, frame: &Frame, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = frame.point(middle[0] * k, middle[1] * k, middle[2] * k);
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let p = e.curve.eval(0.5 * (e.range.0 + e.range.1));
            (i, p.distance(target))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(index as u32)
}

/// The faces that use `edge`, in ascending order.
fn faces_of(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    let mut found = Vec::new();
    for (index, face) in body.faces.iter().enumerate() {
        let uses = face
            .loops
            .iter()
            .flat_map(|l| &body.loop_(*l).unwrap().coedges)
            .any(|c| c.edge == edge);
        if uses {
            found.push(FaceId(index as u32));
        }
    }
    found
}

fn round_in(body: &Body, frame: &Frame, k: f64, middle: [f64; 3], r: f64) -> FilletEdge {
    FilletEdge {
        edge: edge_in(body, frame, k, middle),
        radius: r * k,
    }
}

fn round(body: &Body, k: f64, middle: [f64; 3], r: f64) -> FilletEdge {
    round_in(body, &Frame::WORLD, k, middle, r)
}

/// The outward normal of `face` of `body` at the point of it nearest
/// `point`, from the surface alone.
fn normal_at(body: &Body, face: FaceId, point: Vec3) -> Vec3 {
    let face = body.face(face).unwrap();
    let (u, v) = face.surface.project(point).unwrap();
    let n = face.surface.normal(u, v).unwrap();
    if face.reversed { -n } else { n }
}

/// The largest sine of the angle between the normals of two faces at
/// points along the edges they share (nine per edge); `None` when they
/// share none.
fn crease(body: &Body, a: FaceId, b: FaceId) -> Option<f64> {
    let mut worst: Option<f64> = None;
    for (index, edge) in body.edges.iter().enumerate() {
        let users = faces_of(body, EdgeId(index as u32));
        if !(users.contains(&a) && users.contains(&b)) {
            continue;
        }
        for i in 0..9 {
            let s = (f64::from(i) + 0.5) / 9.0;
            let p = edge
                .curve
                .eval(edge.range.0 + s * (edge.range.1 - edge.range.0));
            let (n, m) = (normal_at(body, a, p), normal_at(body, b, p));
            assert!(n.dot(m) > 0.0, "the faces fold back on each other");
            let sine = n.cross(m).length();
            worst = Some(worst.map_or(sine, |w| w.max(sine)));
        }
    }
    worst
}

/// Every fillet face runs into both faces of its edge, and into the ball
/// of a corner, without a crease. Returns the largest sine found.
fn assert_tangent(body: &Body, edges: &[FilletEdge], made: &FilletResult) -> f64 {
    let limit = 1e-9;
    let mut worst = 0.0_f64;
    let mut seams = 0;
    for (index, face) in made.faces.iter().enumerate() {
        let FilletFace::Fillet(requests) = face else {
            continue;
        };
        for request in requests {
            for neighbour in faces_of(body, edges[*request].edge) {
                let now = made
                    .faces
                    .iter()
                    .position(|f| matches!(f, FilletFace::Kept(list) if list.contains(&neighbour)))
                    .expect("the neighbour of a filleted edge is still there");
                let sine = crease(&made.body, FaceId(index as u32), FaceId(now as u32))
                    .expect("a fillet face borders both faces of its edge");
                assert!(sine <= limit, "a crease of {sine} along a fillet's seam");
                worst = worst.max(sine);
                seams += 1;
            }
        }
        for (other, corner) in made.faces.iter().enumerate() {
            if !matches!(corner, FilletFace::Corner(_)) {
                continue;
            }
            if let Some(sine) = crease(&made.body, FaceId(index as u32), FaceId(other as u32)) {
                assert!(
                    sine <= limit,
                    "a crease of {sine} between fillet and corner"
                );
                worst = worst.max(sine);
            }
        }
    }
    assert!(seams >= 2 * edges.len().min(1));
    // For the record: `FILLET_SEAMS=1 cargo test ... -- --nocapture`.
    if std::env::var_os("FILLET_SEAMS").is_some() {
        println!("SEAMS {seams} worst {worst:e}");
    }
    worst
}

fn solve(body: &Body, edges: &[FilletEdge], k: f64) -> FilletResult {
    let tol = tolerance(k);
    let result = match fillet(body, edges, &tol) {
        Ok(result) => result,
        Err(e) => panic!("fillet at scale {k}: {e}"),
    };
    assert_eq!(check(&result.body, &tol), [], "at scale {k}");
    assert_eq!(result.faces.len(), result.body.faces.len());
    assert_eq!(result.edges.len(), result.body.edges.len());
    assert_eq!(result.vertices.len(), result.body.vertices.len());
    assert_tangent(body, edges, &result);
    result
}

fn counts(result: &FilletResult) -> (usize, usize, usize) {
    let b = &result.body;
    (b.vertices.len(), b.edges.len(), b.faces.len())
}

#[track_caller]
fn assert_mass(result: &FilletResult, volume: f64, area: f64, k: f64) {
    let m = mass(&result.body);
    assert_close(m.volume, volume * k * k * k, "volume");
    assert_close(m.area, area * k * k, "area");
}

/// The faces of a result that are fillet faces.
fn fillet_faces(result: &FilletResult) -> Vec<usize> {
    (0..result.faces.len())
        .filter(|i| matches!(result.faces[*i], FilletFace::Fillet(_)))
        .collect()
}

#[test]
fn one_edge_of_a_block_becomes_a_quarter_cylinder() {
    let mut worst = 0.0_f64;
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let r = 0.5;
        let edges = [round(&block, k, [0.0, 0.0, 1.0], r)];
        let made = solve(&block, &edges, k);
        worst = worst.max(assert_tangent(&block, &edges, &made));
        // Two faces lose a strip, the two ends a spandrel each, and the
        // quarter cylinder is new.
        let area = 52.0 - 2.0 * r * 2.0 - 2.0 * SPANDREL * r * r + PI / 2.0 * r * 2.0;
        assert_mass(&made, 24.0 - SPANDREL * r * r * 2.0, area, k);
        assert_eq!(counts(&made), (10, 15, 7));
        let [face] = fillet_faces(&made)[..] else {
            panic!("one fillet face");
        };
        assert_eq!(made.faces[face], FilletFace::Fillet(vec![0]));
        // A cylinder of the radius whose axis is the radius from both
        // faces, along the edge.
        let Surface::Cylinder(cylinder) = &made.body.faces[face].surface else {
            panic!("the fillet face of a straight edge is a cylinder");
        };
        assert_close(cylinder.radius(), r * k, "radius");
        let axis = cylinder.frame().z();
        assert!(axis.cross(Vec3::Z).length() < 1e-12);
        let origin = cylinder.frame().origin();
        assert!((origin.x - r * k).abs() < 1e-9 * k && (origin.y - r * k).abs() < 1e-9 * k);
        let kept = made
            .faces
            .iter()
            .filter(|f| matches!(f, FilletFace::Kept(list) if list.len() == 1))
            .count();
        assert_eq!(kept, 6);
        // The four far corners and nine edges are the block's own.
        assert_eq!(made.vertices.iter().flatten().count(), 6);
    }
    println!("largest crease along a seam: {worst:e}");
}

/// The polygon through `points`, extruded from 0 to `height` in `frame`.
fn prism(frame: &Frame, k: f64, points: &[[f64; 2]], height: f64) -> Body {
    let curves = scale_curves(&polygon(1, 1, points), k);
    let profiles = detect_profiles(&curves, &tolerance(k)).unwrap();
    extrude(&profiles, frame, 0.0, height * k, &tolerance(k))
        .unwrap()
        .body
}

/// The section a round of radius `r` takes out of a corner of `theta`
/// between the faces: the kite between the two tangent lengths less the
/// sector of the ball.
fn section(theta: f64, r: f64) -> f64 {
    r * r * (1.0 / (theta / 2.0).tan() - (PI - theta) / 2.0)
}

fn regular(sides: u32, circumradius: f64) -> Vec<[f64; 2]> {
    (0..sides)
        .map(|i| {
            let a = f64::from(i) * 2.0 * PI / f64::from(sides);
            [circumradius * a.cos(), circumradius * a.sin()]
        })
        .collect()
}

#[test]
fn edges_between_faces_at_sixty_and_at_a_hundred_and_twenty_degrees() {
    assert_close(section(PI / 2.0, 1.0), SPANDREL, "the square corner");
    for k in SCALES {
        for frame in frames(k) {
            // (sides, the angle between two side faces, the area of the
            // polygon, the length of a side)
            for (sides, theta, base, side) in [
                (6, 2.0 * PI / 3.0, 6.0 * 3.0_f64.sqrt(), 2.0),
                (3, PI / 3.0, 3.0 * 3.0_f64.sqrt(), 2.0 * 3.0_f64.sqrt()),
            ] {
                let (h, r) = (2.5, 0.3);
                let part = prism(&frame, k, &regular(sides, 2.0), h);
                // The upright edge through the first corner.
                let edges = [round_in(&part, &frame, k, [2.0, 0.0, h / 2.0], r)];
                let made = solve(&part, &edges, k);
                let d = r / (theta / 2.0).tan();
                let volume = base * h - section(theta, r) * h;
                let area = 2.0 * base + f64::from(sides) * side * h
                    - 2.0 * d * h
                    - 2.0 * section(theta, r)
                    + (PI - theta) * r * h;
                assert_mass(&made, volume, area, k);
                let [face] = fillet_faces(&made)[..] else {
                    panic!("one fillet face");
                };
                let Surface::Cylinder(cylinder) = &made.body.faces[face].surface else {
                    panic!("a cylinder");
                };
                assert_close(cylinder.radius(), r * k, "radius");
                // The axis is the radius from both faces: on the bisector,
                // r / sin(theta / 2) from the edge.
                let on_axis = frame.point((2.0 - r / (theta / 2.0).sin()) * k, 0.0, 0.0);
                let offset = on_axis - cylinder.frame().origin();
                let axis = cylinder.frame().z();
                assert!((offset - axis * offset.dot(axis)).length() < 1e-9 * k);
            }
        }
    }
}

const ELL: [[f64; 2]; 6] = [
    [0.0, 0.0],
    [4.0, 0.0],
    [4.0, 1.5],
    [1.5, 1.5],
    [1.5, 3.0],
    [0.0, 3.0],
];

#[test]
fn the_inside_corner_of_an_l_gains_material() {
    for k in SCALES {
        for frame in frames(k) {
            let ell = prism(&frame, k, &ELL, 2.0);
            let r = 0.4;
            let made = solve(&ell, &[round_in(&ell, &frame, k, [1.5, 1.5, 1.0], r)], k);
            // The two walls lose a strip each, the caps gain a spandrel
            // each, and the fillet face is new.
            let area = 2.0 * 8.25 + 14.0 * 2.0 - 2.0 * r * 2.0
                + 2.0 * SPANDREL * r * r
                + PI / 2.0 * r * 2.0;
            assert_mass(&made, 16.5 + SPANDREL * r * r * 2.0, area, k);
            assert_eq!(counts(&made), (14, 21, 9));
        }
    }
}

#[test]
fn an_edge_that_ends_against_a_wall_stops_there() {
    for k in SCALES {
        for frame in frames(k) {
            let ell = prism(&frame, k, &ELL, 2.0);
            let r = 0.4;
            let made = solve(&ell, &[round_in(&ell, &frame, k, [1.5, 2.25, 2.0], r)], k);
            let m = mass(&made.body);
            assert_close(
                m.volume,
                (16.5 - SPANDREL * r * r * 1.5) * k * k * k,
                "volume",
            );
            // The wall it runs into has grown by a spandrel and is still
            // one face.
            assert_eq!(made.body.faces.len(), 9);
        }
    }
}

/// A ledge's edge that runs into a wall at a slant: the fillet ends in
/// the plane of that wall (refused as "does not end cleanly" before
/// 2026-10-15; engine-block findings E3, E4). The removed prism lies
/// between two planes oblique to the edge, so its volume is the section
/// times the length of the line through its centroid between them.
#[test]
fn an_edge_that_ends_against_a_slanted_wall_ends_in_its_plane() {
    let points = [
        [0.0, 0.0],
        [4.0, 0.0],
        [4.0, 1.5],
        [2.0, 1.5],
        [1.5, 3.0],
        [0.0, 3.0],
    ];
    let whole = (4.0 * 1.5 + 0.5 * (2.0 + 1.5) * 1.5) * 2.0;
    let r = 0.3;
    for k in SCALES {
        for frame in frames(k) {
            let part = prism(&frame, k, &points, 2.0);
            let request = round_in(&part, &frame, k, [1.75, 2.25, 2.0], r);
            let made = solve(&part, &[request], k);
            let removed = SPANDREL * r * r * 2.5_f64.sqrt();
            assert_close(
                mass(&made.body).volume,
                (whole - removed) * k * k * k,
                "volume",
            );
            assert_eq!(made.body.faces.len(), part.faces.len() + 1);
        }
    }
}

/// Two rounds of radii `a` and `b` on edges that meet at a square corner
/// take the corner's cube twice: the volume both take, by the midpoint
/// rule along the edge both faces share (written out here, with nothing
/// of the kernel in it).
fn shared_by_two(a: f64, b: f64) -> f64 {
    let width = |r: f64, y: f64| {
        if y < r {
            r - (r * r - (r - y) * (r - y)).sqrt()
        } else {
            0.0
        }
    };
    let n = 4_000_000;
    let top = a.min(b);
    let h = top / f64::from(n);
    (0..n)
        .map(|i| {
            let y = (f64::from(i) + 0.5) * h;
            width(a, y) * width(b, y) * h
        })
        .sum()
}

#[test]
fn two_edges_that_meet_run_out_into_each_other() {
    // The closed form for equal radii, and the rule it is held to.
    let closed = 5.0 / 3.0 - PI / 2.0;
    assert!((shared_by_two(1.0, 1.0) - closed).abs() < 1e-9);
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        for (ra, rb) in [(0.5, 0.5), (0.3, 0.7), (0.7, 0.3)] {
            let made = solve(
                &block,
                &[
                    round(&block, k, [0.0, 0.0, 1.0], ra),
                    round(&block, k, [2.0, 0.0, 0.0], rb),
                ],
                k,
            );
            let shared = if ra == rb {
                closed * ra * ra * ra
            } else {
                shared_by_two(ra, rb)
            };
            let volume = 24.0 - SPANDREL * (ra * ra * 2.0 + rb * rb * 4.0) + shared;
            let m = mass(&made.body);
            let slack = if ra == rb { 1e-9 } else { 1e-7 };
            assert!(
                (m.volume / (k * k * k) - volume).abs() < slack * volume,
                "{} is not {volume}",
                m.volume / (k * k * k)
            );
            assert_eq!(made.body.faces.len(), 8);
            // The two cylinders meet in one edge: an arc of an ellipse
            // for equal radii.
            let [a, b] = fillet_faces(&made)[..] else {
                panic!("two fillet faces");
            };
            let between: Vec<&str> = (0..made.body.edges.len())
                .filter(|e| {
                    let users = faces_of(&made.body, EdgeId(*e as u32));
                    users.contains(&FaceId(a as u32)) && users.contains(&FaceId(b as u32))
                })
                .map(|e| made.body.edges[e].curve.kind())
                .collect();
            assert_eq!(between.len(), 1);
            if ra == rb {
                assert_eq!(between, ["ellipse"]);
            }
        }
    }
}

/// The distance from `p` to the convex region `n . q <= c` of `planes`
/// (unit normals): the nearest of the points that are nearest on one,
/// two or three of the planes and inside the others.
fn distance_to(planes: &[(Vec3, f64)], p: Vec3) -> f64 {
    let inside = |q: Vec3| {
        planes
            .iter()
            .all(|(n, c)| n.dot(q) <= c + 1e-9 * (1.0 + c.abs()))
    };
    if inside(p) {
        return 0.0;
    }
    let mut best = f64::INFINITY;
    let count = planes.len();
    let mut consider = |active: &[usize]| {
        // q = p - sum(l_i n_i) with n_j . q = c_j for the active planes.
        let m = active.len();
        let mut g = [[0.0_f64; 4]; 3];
        for (row, &i) in active.iter().enumerate() {
            for (col, &j) in active.iter().enumerate() {
                g[row][col] = planes[i].0.dot(planes[j].0);
            }
            g[row][3] = planes[i].0.dot(p) - planes[i].1;
        }
        for col in 0..m {
            let pivot = (col..m)
                .max_by(|a, b| g[*a][col].abs().total_cmp(&g[*b][col].abs()))
                .unwrap();
            g.swap(col, pivot);
            if g[col][col].abs() < 1e-12 {
                return;
            }
            for row in 0..m {
                if row != col {
                    let f = g[row][col] / g[col][col];
                    let pivot_row = g[col];
                    for (entry, above) in g[row].iter_mut().zip(pivot_row) {
                        *entry -= f * above;
                    }
                }
            }
        }
        let mut q = p;
        for (row, &i) in active.iter().enumerate() {
            q = q - planes[i].0 * (g[row][3] / g[row][row]);
        }
        if inside(q) {
            best = best.min(q.distance(p));
        }
    };
    for i in 0..count {
        consider(&[i]);
        for j in i + 1..count {
            consider(&[i, j]);
            for l in j + 1..count {
                consider(&[i, j, l]);
            }
        }
    }
    best
}

/// `planes` moved inwards by `r`.
fn shrunk(planes: &[(Vec3, f64)], r: f64) -> Vec<(Vec3, f64)> {
    planes.iter().map(|(n, c)| (*n, c - r)).collect()
}

fn box_planes(size: [f64; 3]) -> Vec<(Vec3, f64)> {
    vec![
        (-Vec3::X, 0.0),
        (-Vec3::Y, 0.0),
        (-Vec3::Z, 0.0),
        (Vec3::X, size[0]),
        (Vec3::Y, size[1]),
        (Vec3::Z, size[2]),
    ]
}

/// Holds a filleted body against its definition at seeded points near
/// `about`: a point is in it when it is in the solid of `planes` and no
/// further than `r` from the region `rounded` leaves when it is moved
/// inwards by `r` (the union of the balls that fit behind those faces).
/// Everything is given at scale 1. Returns how many points decided.
fn assert_matches_definition(
    body: &Body,
    k: f64,
    planes: &[(Vec3, f64)],
    rounded: &[(Vec3, f64)],
    r: f64,
    about: Vec3,
    seed: u64,
) -> usize {
    let inner = shrunk(rounded, r);
    let mut rng = Rng(seed);
    let mut points = Vec::new();
    let mut expected = Vec::new();
    while points.len() < 600 {
        let p = about
            + Vec3::new(
                rng.between(-2.0 * r, 2.0 * r),
                rng.between(-2.0 * r, 2.0 * r),
                rng.between(-2.0 * r, 2.0 * r),
            );
        let in_solid = planes.iter().map(|(n, c)| c - n.dot(p)).fold(1.0, f64::min);
        let from_inner = distance_to(&inner, p);
        // Leave out what is too near a face to call.
        if in_solid.abs() < 1e-3 * r || (from_inner - r).abs() < 1e-3 * r {
            continue;
        }
        points.push(p * k);
        expected.push(in_solid > 0.0 && from_inner < r);
    }
    let found = contains_points(body, &points, &tolerance(k)).unwrap();
    let mut decided = 0;
    for ((point, want), got) in points.iter().zip(&expected).zip(found) {
        if let Some(got) = got {
            assert_eq!(got, *want, "the point {point:?} at scale {k}");
            decided += 1;
        }
    }
    assert!(decided > 500, "{decided} points decided");
    decided
}

/// The part of the cube `[0, r]^3` in the corner of a block that is left
/// when its three edges are rounded: counted on a grid from the
/// definition alone (a point stays when it is no further than `r` from
/// the corner moved inwards by `r`).
fn corner_left_by_counting(r: f64) -> f64 {
    let corner = [(-Vec3::X, 0.0), (-Vec3::Y, 0.0), (-Vec3::Z, 0.0)];
    let inner = shrunk(&corner, r);
    let n = 96;
    let h = r / f64::from(n);
    let mut inside = 0_u64;
    for i in 0..n {
        for j in 0..n {
            for l in 0..n {
                let p = Vec3::new(
                    (f64::from(i) + 0.5) * h,
                    (f64::from(j) + 0.5) * h,
                    (f64::from(l) + 0.5) * h,
                );
                if distance_to(&inner, p) <= r {
                    inside += 1;
                }
            }
        }
    }
    inside as f64 * h * h * h
}

#[test]
fn three_edges_meet_in_an_eighth_of_a_sphere() {
    let r = 0.5;
    // Within the cube of side r in the corner what is left is an eighth
    // of the ball; outside it the three strips do not meet. So:
    //   V = V0 - (1 - pi/4) r^2 (L + W + H - 3 r) - (r^3 - pi r^3 / 6)
    let left = PI / 6.0 * r * r * r;
    let counted = corner_left_by_counting(r);
    assert!(
        (counted - left).abs() < 2e-3 * r * r * r,
        "counting gives {counted}, the closed form {left}"
    );
    let volume = 24.0 - SPANDREL * r * r * (9.0 - 3.0 * r) - (r * r * r - left);
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let edges = [
            round(&block, k, [0.0, 0.0, 1.0], r),
            round(&block, k, [2.0, 0.0, 0.0], r),
            round(&block, k, [0.0, 1.5, 0.0], r),
        ];
        let made = solve(&block, &edges, k);
        // Each of the three faces at the corner loses two strips that
        // share a square; each of the three far faces a spandrel; three
        // quarter cylinders that stop the radius short of the corner; an
        // eighth of a sphere.
        let area = 52.0 - 2.0 * r * 9.0 + 3.0 * r * r - 3.0 * SPANDREL * r * r
            + PI / 2.0 * r * (9.0 - 3.0 * r)
            + PI / 2.0 * r * r;
        let m = mass(&made.body);
        assert_close(m.volume, volume * k * k * k, "volume");
        assert_close(m.area, area * k * k, "area");
        assert_eq!(counts(&made), (13, 21, 10));
        let corner: Vec<usize> = (0..made.faces.len())
            .filter(|i| matches!(made.faces[*i], FilletFace::Corner(_)))
            .collect();
        let [corner] = corner[..] else {
            panic!("one corner face");
        };
        let Surface::Sphere(sphere) = &made.body.faces[corner].surface else {
            panic!("the corner is a piece of a sphere");
        };
        assert_close(sphere.radius(), r * k, "radius");
        assert!(sphere.frame().origin().distance(Vec3::new(r, r, r) * k) < 1e-9 * k);
        let planes = box_planes([4.0, 3.0, 2.0]);
        assert_matches_definition(&made.body, k, &planes, &planes[..3], r, Vec3::ZERO, 7);
    }
}

#[test]
fn all_twelve_edges_of_a_block() {
    for k in SCALES {
        let (l, w, h) = (4.0, 3.0, 2.0);
        let block = boxed(k, [0.0, 0.0, 0.0], [l, w, h]);
        let r = 0.25;
        let edges: Vec<FilletEdge> = (0..12)
            .map(|i| FilletEdge {
                edge: EdgeId(i),
                radius: r * k,
            })
            .collect();
        let made = solve(&block, &edges, k);
        // The block with every edge and corner rounded is the smaller
        // block grown by a ball (Steiner): volume, area, the edges'
        // quarter turns and the ball.
        let (a, b, c) = (l - 2.0 * r, w - 2.0 * r, h - 2.0 * r);
        let face = 2.0 * (a * b + a * c + b * c);
        let volume = a * b * c + face * r + PI * r * r * (a + b + c) + 4.0 / 3.0 * PI * r * r * r;
        let area = face + 2.0 * PI * r * (a + b + c) + 4.0 * PI * r * r;
        assert_mass(&made, volume, area, k);
        // The same by strips and corners.
        let strips = l * w * h - SPANDREL * r * r * 4.0 * (l + w + h)
            + 8.0 * (2.0 - 7.0 * PI / 12.0) * r * r * r;
        assert_close(volume, strips, "the two closed forms");
        assert_eq!(counts(&made), (24, 48, 26));
        let planes = box_planes([l, w, h]);
        assert_matches_definition(&made.body, k, &planes, &planes, r, Vec3::new(l, w, h), 11);
    }
}

fn hexagon_planes(height: f64) -> Vec<(Vec3, f64)> {
    let mut planes: Vec<(Vec3, f64)> = (0..6)
        .map(|i| {
            let a = (f64::from(i) + 0.5) * PI / 3.0;
            (Vec3::new(a.cos(), a.sin(), 0.0), 3.0_f64.sqrt())
        })
        .collect();
    planes.push((-Vec3::Z, 0.0));
    planes.push((Vec3::Z, height));
    planes
}

#[test]
fn corners_that_are_not_square_take_a_piece_of_the_ball_too() {
    for k in SCALES {
        let (h, r) = (2.5, 0.3);
        let part = prism(&Frame::WORLD, k, &regular(6, 2.0), h);
        // Every edge: the smaller prism grown by a ball. With the mean
        // width term from the edges' lengths and outer angles.
        let edges: Vec<FilletEdge> = (0..part.edges.len())
            .map(|i| FilletEdge {
                edge: EdgeId(i as u32),
                radius: r * k,
            })
            .collect();
        assert_eq!(edges.len(), 18);
        let made = solve(&part, &edges, k);
        let apothem = 3.0_f64.sqrt() - r;
        let side = 2.0 * apothem / 3.0_f64.sqrt();
        let tall = h - 2.0 * r;
        let base = 2.0 * 3.0_f64.sqrt() * apothem * apothem;
        let face = 2.0 * base + 6.0 * side * tall;
        let turn = 0.5 * (12.0 * side * PI / 2.0 + 6.0 * tall * PI / 3.0);
        let volume = base * tall + face * r + turn * r * r + 4.0 / 3.0 * PI * r * r * r;
        let area = face + 2.0 * turn * r + 4.0 * PI * r * r;
        assert_mass(&made, volume, area, k);
        assert_eq!(counts(&made), (36, 72, 38));
        let spheres = made
            .faces
            .iter()
            .zip(&made.body.faces)
            .filter(|(from, face)| {
                matches!(from, FilletFace::Corner(_)) && face.surface.kind() == "sphere"
            })
            .count();
        assert_eq!(spheres, 12);
        let planes = hexagon_planes(h);
        assert_matches_definition(
            &made.body,
            k,
            &planes,
            &planes,
            r,
            Vec3::new(2.0, 0.0, h),
            13,
        );
        // Three edges at one corner only.
        let edges = [
            round(&part, k, [2.0, 0.0, h / 2.0], r),
            round(&part, k, [1.5, 0.75_f64.sqrt(), h], r),
            round(&part, k, [1.5, -(0.75_f64.sqrt()), h], r),
        ];
        let made = solve(&part, &edges, k);
        assert_eq!(made.body.faces.len(), 8 + 3 + 1);
        let at_corner = [planes[0], planes[5], planes[7]];
        assert_matches_definition(
            &made.body,
            k,
            &planes,
            &at_corner,
            r,
            Vec3::new(2.0, 0.0, h),
            17,
        );
    }
}

#[test]
fn an_inside_corner_of_three_edges_is_filled_by_a_piece_of_the_ball() {
    for k in SCALES {
        // A block with a smaller block taken out of one corner: the three
        // edges at the bottom of the pocket are concave.
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let pocket = boxed(k, [2.0, 1.5, 1.0], [5.0, 4.0, 3.0]);
        let part = boolean(&block, &pocket, BooleanOp::Subtract, &tolerance(k))
            .unwrap()
            .body;
        let r = 0.3;
        let edges = [
            round(&part, k, [3.0, 1.5, 1.0], r),
            round(&part, k, [2.0, 2.25, 1.0], r),
            round(&part, k, [2.0, 1.5, 1.5], r),
        ];
        let made = solve(&part, &edges, k);
        // The mirror image of the outside corner: the strips are added.
        let lengths = 2.0 + 1.5 + 1.0;
        let added = SPANDREL * r * r * (lengths - 3.0 * r) + (r * r * r - PI / 6.0 * r * r * r);
        assert_close(
            mass(&made.body).volume,
            (24.0 - 2.0 * 1.5 * 1.0 + added) * k * k * k,
            "volume",
        );
        let corners = made
            .faces
            .iter()
            .filter(|f| matches!(f, FilletFace::Corner(_)))
            .count();
        assert_eq!(corners, 1);
    }
}

#[test]
fn seeded_random_sets_of_edges_of_random_blocks() {
    let mut rng = Rng(0x00F1_11E7);
    let (mut corners, mut pairs) = (0, 0);
    for round in 0..60 {
        let k = SCALES[round % 3];
        let size = [
            1.0 + 4.0 * rng.unit(),
            1.0 + 4.0 * rng.unit(),
            1.0 + 4.0 * rng.unit(),
        ];
        let block = boxed(k, [0.0, 0.0, 0.0], size);
        let least = size.iter().fold(f64::INFINITY, |a, b| a.min(*b));
        let keep = 0.2 + 0.8 * rng.unit();
        let r = least * (0.03 + 0.3 * rng.unit());
        let chosen: Vec<EdgeId> = (0..12).filter(|_| rng.unit() <= keep).map(EdgeId).collect();
        let requests: Vec<FilletEdge> = chosen
            .iter()
            .map(|edge| FilletEdge {
                edge: *edge,
                radius: r * k,
            })
            .collect();
        // Strips, and per corner what two or three of them share.
        let mut volume = size[0] * size[1] * size[2];
        for edge in &chosen {
            let e = block.edge(*edge).unwrap();
            volume -= SPANDREL * r * r * (e.range.1 - e.range.0) / k;
        }
        for index in 0..block.vertices.len() {
            let here = chosen
                .iter()
                .filter(|edge| {
                    let e = block.edge(**edge).unwrap();
                    e.start.0 as usize == index || e.end.0 as usize == index
                })
                .count();
            match here {
                2 => {
                    volume += (5.0 / 3.0 - PI / 2.0) * r * r * r;
                    pairs += 1;
                }
                3 => {
                    volume += (2.0 - 7.0 * PI / 12.0) * r * r * r;
                    corners += 1;
                }
                _ => {}
            }
        }
        let made = match fillet(&block, &requests, &tolerance(k)) {
            Ok(made) => made,
            Err(e) => panic!("round {round} at scale {k}, {} edges: {e}", requests.len()),
        };
        assert_eq!(check(&made.body, &tolerance(k)), [], "round {round}");
        assert_tangent(&block, &requests, &made);
        assert_close(
            mass(&made.body).volume,
            volume * k * k * k,
            &format!("round {round}: volume"),
        );
    }
    println!("{corners} corners with a ball, {pairs} pairs that run out into each other");
    assert!(
        corners > 30 && pairs > 30,
        "{corners} corners, {pairs} pairs"
    );
}

/// What the region between a corner at `rim` of a section in the
/// half-plane (distance from the axis, height) and the arc of a ball of
/// radius `r` that touches both its sides sweeps in a full turn about the
/// axis, and the area the arc sweeps (Pappus, twice). The sides leave the
/// corner along the unit directions `b1` and `b2`.
fn ring(rim: [f64; 2], b1: [f64; 2], b2: [f64; 2], r: f64) -> (f64, f64) {
    let theta = (b1[0] * b2[0] + b1[1] * b2[1]).clamp(-1.0, 1.0).acos();
    let d = r / (theta / 2.0).tan();
    let t1 = [rim[0] + d * b1[0], rim[1] + d * b1[1]];
    let t2 = [rim[0] + d * b2[0], rim[1] + d * b2[1]];
    let far = d / (1.0 + theta.cos());
    let centre = [
        rim[0] + far * (b1[0] + b2[0]),
        rim[1] + far * (b1[1] + b2[1]),
    ];
    // The kite as two triangles of area d r / 2 each.
    let kite = 0.5 * d * r * ((rim[0] + t1[0] + centre[0]) + (rim[0] + t2[0] + centre[0])) / 3.0;
    // The sector of the ball, half of whose angle is alpha: its centroid
    // and that of its arc lie towards the corner.
    let alpha = (PI - theta) / 2.0;
    let to_rim = (rim[0] - centre[0]) / (rim[0] - centre[0]).hypot(rim[1] - centre[1]);
    let sector_at = centre[0] + to_rim * 2.0 * r * alpha.sin() / (3.0 * alpha);
    let sector = r * r * alpha * sector_at;
    let arc_at = centre[0] + to_rim * r * alpha.sin() / alpha;
    (
        2.0 * PI * (kite - sector),
        2.0 * PI * arc_at * 2.0 * alpha * r,
    )
}

fn rod(frame: &Frame, k: f64, radius: f64, height: f64) -> Body {
    primitives::cylinder(frame, radius * k, height * k).unwrap()
}

#[test]
fn the_rim_of_a_rod_becomes_a_quarter_of_a_torus() {
    // The square corner by hand: the square less the quarter disc, each
    // at its own centroid.
    let (rr, r) = (1.0_f64, 0.25_f64);
    let by_hand =
        2.0 * PI * (r * r * (rr - r / 2.0) - PI / 4.0 * r * r * (rr - r) - r * r * r / 3.0);
    assert_close(
        ring([rr, 3.0], [-1.0, 0.0], [0.0, -1.0], r).0,
        by_hand,
        "Pappus",
    );
    for k in SCALES {
        for frame in frames(k) {
            let (rr, h, r) = (1.0, 3.0, 0.25);
            let part = rod(&frame, k, rr, h);
            let edges = [round_in(&part, &frame, k, [-rr, 0.0, h], r)];
            let made = solve(&part, &edges, k);
            let (removed, torus_area) = ring([rr, h], [-1.0, 0.0], [0.0, -1.0], r);
            let area =
                PI * rr * rr + 2.0 * PI * rr * (h - r) + PI * (rr - r) * (rr - r) + torus_area;
            assert_mass(&made, PI * rr * rr * h - removed, area, k);
            assert_eq!(made.body.faces.len(), 4);
            let [face] = fillet_faces(&made)[..] else {
                panic!("one fillet face");
            };
            let Surface::Torus(torus) = &made.body.faces[face].surface else {
                panic!("the fillet face of a rim is a torus");
            };
            assert_close(torus.radii().1, r * k, "the tube");
            assert_close(torus.radii().0, (rr - r) * k, "the ring");
            let centre = frame.point(0.0, 0.0, (h - r) * k);
            assert!(torus.frame().origin().distance(centre) < 1e-9 * k.max(1.0) * 10.0);
        }
    }
}

#[test]
fn the_mouth_of_a_hole_loses_material_and_the_foot_of_a_boss_gains_it() {
    for k in SCALES {
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let at = |z: f64| Frame::WORLD.at(Vec3::new(2.0 * k, 1.5 * k, z * k)).unwrap();
        let (rh, r) = (0.5, 0.2);
        // A hole right through: its mouth in the top.
        let drill = rod(&at(-1.0), k, rh, 4.0);
        let drilled = boolean(&plate, &drill, BooleanOp::Subtract, &tolerance(k))
            .unwrap()
            .body;
        let made = solve(&drilled, &[round(&drilled, k, [2.0 - rh, 1.5, 2.0], r)], k);
        let (removed, _) = ring([rh, 2.0], [1.0, 0.0], [0.0, -1.0], r);
        assert_close(
            mass(&made.body).volume,
            (24.0 - PI * rh * rh * 2.0 - removed) * k * k * k,
            "volume",
        );
        assert_eq!(made.body.faces.len(), 8);
        assert_eq!(genus(&made.body), Some(1));
        // A blind hole: the rim at its bottom is an inside corner.
        let drill = rod(&at(1.0), k, rh, 2.0);
        let blind = boolean(&plate, &drill, BooleanOp::Subtract, &tolerance(k))
            .unwrap()
            .body;
        let made = solve(&blind, &[round(&blind, k, [2.0 - rh, 1.5, 1.0], r)], k);
        let (added, _) = ring([rh, 1.0], [-1.0, 0.0], [0.0, 1.0], r);
        assert_close(
            mass(&made.body).volume,
            (24.0 - PI * rh * rh * 1.0 + added) * k * k * k,
            "volume",
        );
        // A ball that does not fit in the hole: the bottom's tangent
        // circle would run into the axis. What fits is less than that:
        // from half the hole's radius on, the ball's centre (rh - r from
        // the axis) comes as near to the axis as its radius, and the
        // torus would reach its own axis, which is refused (until
        // 2026-10-15 the estimate said rh, while every radius from rh / 2
        // up to it was refused that way).
        match fillet(
            &blind,
            &[round(&blind, k, [2.0 - rh, 1.5, 1.0], 0.6)],
            &tolerance(k),
        ) {
            Err(KernelError::FilletTooLarge {
                radius, largest, ..
            }) => {
                assert_close(radius, 0.6 * k, "radius");
                assert_close(largest, rh / 2.0 * k, "largest");
            }
            other => panic!("expected too large, got {other:?}"),
        }
        match fillet(
            &blind,
            &[round(&blind, k, [2.0 - rh, 1.5, 1.0], 0.26)],
            &tolerance(k),
        ) {
            Err(KernelError::Fillet { reason, .. }) => {
                assert!(reason.contains("half the rim's"), "{reason}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        let r = 0.24;
        let made = solve(&blind, &[round(&blind, k, [2.0 - rh, 1.5, 1.0], r)], k);
        let (added, _) = ring([rh, 1.0], [-1.0, 0.0], [0.0, 1.0], r);
        assert_close(
            mass(&made.body).volume,
            (24.0 - PI * rh * rh * 1.0 + added) * k * k * k,
            "volume just below half the hole's radius",
        );
        // A boss on the plate: its foot is an inside corner too.
        let boss = rod(&at(2.0), k, rh, 1.0);
        let raised = boolean(&plate, &boss, BooleanOp::Union, &tolerance(k))
            .unwrap()
            .body;
        let made = solve(&raised, &[round(&raised, k, [2.0 - rh, 1.5, 2.0], r)], k);
        let (added, _) = ring([rh, 2.0], [1.0, 0.0], [0.0, 1.0], r);
        assert_close(
            mass(&made.body).volume,
            (24.0 + PI * rh * rh * 1.0 + added) * k * k * k,
            "volume",
        );
    }
}

#[test]
fn the_base_of_a_cone() {
    for k in SCALES {
        let (rr, h, r) = (1.0, 2.0, 0.2);
        let part = primitives::cone(&Frame::WORLD, rr * k, h * k).unwrap();
        let made = solve(&part, &[round(&part, k, [-rr, 0.0, 0.0], r)], k);
        // Up the slope from the rim: towards the apex.
        let slant = rr.hypot(h);
        let (removed, _) = ring([rr, 0.0], [-1.0, 0.0], [-rr / slant, h / slant], r);
        assert_close(
            mass(&made.body).volume,
            (PI * rr * rr * h / 3.0 - removed) * k * k * k,
            "volume",
        );
        let [face] = fillet_faces(&made)[..] else {
            panic!("one fillet face");
        };
        assert_eq!(made.body.faces[face].surface.kind(), "torus");
    }
}

/// A slot: two half circles of radius 1 about (0, 0) and (3, 0) joined
/// by two lines, extruded to `height`.
fn slot(frame: &Frame, k: f64, height: f64) -> Body {
    let curves = scale_curves(
        &[
            line(1, [0.0, -1.0], [3.0, -1.0]),
            arc(2, [3.0, 0.0], [3.0, -1.0], [3.0, 1.0]),
            line(3, [3.0, 1.0], [0.0, 1.0]),
            arc(4, [0.0, 0.0], [0.0, 1.0], [0.0, -1.0]),
        ],
        k,
    );
    let profiles = detect_profiles(&curves, &tolerance(k)).unwrap();
    extrude(&profiles, frame, 0.0, height * k, &tolerance(k))
        .unwrap()
        .body
}

#[test]
fn the_top_rim_of_a_slot_is_a_chain_of_cylinders_and_tori() {
    for k in SCALES {
        for frame in frames(k) {
            let part = slot(&frame, k, 2.0);
            let seed = edge_in(&part, &frame, k, [1.5, -1.0, 2.0]);
            let top = faces_of(&part, seed)
                .into_iter()
                .find(|f| faces_of(&part, edge_in(&part, &frame, k, [1.5, 1.0, 2.0])).contains(f))
                .unwrap();
            let chain = tangent_chain(&part, seed, top).unwrap().unwrap();
            assert_eq!(chain.len(), 4);
            let r = 0.3;
            let edges: Vec<FilletEdge> = chain
                .iter()
                .map(|(edge, _)| FilletEdge {
                    edge: *edge,
                    radius: r * k,
                })
                .collect();
            let made = solve(&part, &edges, k);
            // Two straight strips and one ring in two halves.
            let (ring_removed, ring_area) = ring([1.0, 2.0], [-1.0, 0.0], [0.0, -1.0], r);
            let removed = 2.0 * 3.0 * SPANDREL * r * r + ring_removed;
            let whole = (6.0 + PI) * 2.0;
            let top_area = (3.0 * 2.0 * (1.0 - r)) + PI * (1.0 - r) * (1.0 - r);
            let area = (6.0 + PI)
                + top_area
                + (6.0 + 2.0 * PI) * (2.0 - r)
                + 2.0 * 3.0 * PI / 2.0 * r
                + ring_area;
            assert_mass(&made, whole - removed, area, k);
            assert_eq!(made.body.faces.len(), 6 + 4);
            // Along the chain the cylinders and the tori continue each
            // other without a crease.
            let blends = fillet_faces(&made);
            let kinds: Vec<&str> = blends
                .iter()
                .map(|i| made.body.faces[*i].surface.kind())
                .collect();
            assert_eq!(kinds.iter().filter(|kind| **kind == "cylinder").count(), 2);
            assert_eq!(kinds.iter().filter(|kind| **kind == "torus").count(), 2);
            let mut joints = 0;
            for a in &blends {
                for b in &blends {
                    if a < b
                        && let Some(sine) = crease(&made.body, FaceId(*a as u32), FaceId(*b as u32))
                    {
                        assert!(sine < 1e-9, "a crease of {sine} along the chain");
                        joints += 1;
                    }
                }
            }
            assert_eq!(joints, 4);
            // One arc alone ends in the middle of the chain: refused.
            let alone: Vec<FilletEdge> = edges
                .iter()
                .copied()
                .filter(|e| part.edge(e.edge).unwrap().curve.kind() == "circle")
                .take(1)
                .collect();
            match fillet(&part, &alone, &tolerance(k)) {
                Err(KernelError::Fillet { edge, reason, .. }) => {
                    assert_eq!(edge, Some(alone[0].edge));
                    assert!(reason.contains("does not end cleanly"), "{reason}");
                }
                other => panic!("expected a refusal, got {other:?}"),
            }
        }
    }
}

#[test]
fn half_a_rim_ends_in_the_flat_side_of_a_halved_rod() {
    for k in SCALES {
        let part = rod(&Frame::WORLD, k, 1.0, 3.0);
        let half = boxed(k, [-2.0, 0.0, -1.0], [2.0, 2.0, 4.0]);
        let halved = boolean(&part, &half, BooleanOp::Intersect, &tolerance(k))
            .unwrap()
            .body;
        let r = 0.2;
        let made = solve(&halved, &[round(&halved, k, [0.0, 1.0, 3.0], r)], k);
        let (removed, _) = ring([1.0, 3.0], [-1.0, 0.0], [0.0, -1.0], r);
        assert_close(
            mass(&made.body).volume,
            (PI * 3.0 / 2.0 - removed / 2.0) * k * k * k,
            "volume",
        );
        assert_eq!(made.body.faces.len(), 5);
    }
}

#[test]
fn a_radius_too_large_names_the_edge_the_face_and_what_fits() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        // The edge along z between the faces x = 0 (3 wide) and y = 0 (4
        // wide): the tangent line leaves the narrower one first.
        let request = round(&block, k, [0.0, 0.0, 1.0], 3.5);
        match fillet(&block, &[request], &tolerance(k)) {
            Err(KernelError::FilletTooLarge {
                edge,
                face,
                radius,
                largest,
            }) => {
                assert_eq!(edge, request.edge);
                assert!(normal_at(&block, face, Vec3::ZERO).dot(-Vec3::X) > 0.999);
                assert_close(radius, 3.5 * k, "radius");
                assert_close(largest, 3.0 * k, "largest");
            }
            other => panic!("expected too large, got {other:?}"),
        }
        // Just below it fits.
        solve(&block, &[round(&block, k, [0.0, 0.0, 1.0], 2.9)], k);
        // Two fillets that would meet in the middle of the face y = 0.
        let pair = [
            round(&block, k, [0.0, 0.0, 1.0], 2.1),
            round(&block, k, [4.0, 0.0, 1.0], 2.1),
        ];
        // Of one radius they fit up to half the face's width each (until
        // 2026-10-15 the hint held the other at 2.1 and said 1.9).
        match fillet(&block, &pair, &tolerance(k)) {
            Err(KernelError::FilletTooLarge { face, largest, .. }) => {
                assert!(normal_at(&block, face, Vec3::ZERO).dot(-Vec3::Y) > 0.999);
                assert_close(largest, 2.0 * k, "largest");
            }
            other => panic!("expected too large, got {other:?}"),
        }
        // Just below it both build: two corners of (1 - pi / 4) r^2 by 2.
        let r = 1.99;
        let pair = [
            round(&block, k, [0.0, 0.0, 1.0], r),
            round(&block, k, [4.0, 0.0, 1.0], r),
        ];
        let made = solve(&block, &pair, k);
        assert_close(
            mass(&made.body).volume,
            (24.0 - 2.0 * (1.0 - PI / 4.0) * r * r * 2.0) * k * k * k,
            "volume just below half the face",
        );
        // Where the faces are not square the radius and the distance
        // along the face differ: at sixty degrees a side of 2 sqrt(3)
        // takes a radius of 2 (the inscribed circle is of radius 1, and
        // every radius above it is refused by the other edges... not by
        // this one, which stands alone).
        let part = prism(&Frame::WORLD, k, &regular(3, 2.0), 2.5);
        let request = round(&part, k, [2.0, 0.0, 1.25], 2.5);
        match fillet(&part, &[request], &tolerance(k)) {
            Err(KernelError::FilletTooLarge { largest, .. }) => {
                assert_close(largest, 2.0 * k, "largest at sixty degrees");
            }
            other => panic!("expected too large, got {other:?}"),
        }
        // The rim of a rod: at half its radius the torus would reach its
        // own axis.
        let part = rod(&Frame::WORLD, k, 1.0, 3.0);
        match fillet(
            &part,
            &[round(&part, k, [-1.0, 0.0, 3.0], 0.6)],
            &tolerance(k),
        ) {
            Err(KernelError::Fillet { reason, .. }) => {
                assert!(reason.contains("half the rim's"), "{reason}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        // Far too large: what fits is below half the rod's radius, where
        // the torus would reach its axis (until 2026-10-15 the estimate
        // said the whole radius, where the end's tangent circle runs into
        // the axis, while 0.6 above is refused).
        match fillet(
            &part,
            &[round(&part, k, [-1.0, 0.0, 3.0], 1.5)],
            &tolerance(k),
        ) {
            Err(KernelError::FilletTooLarge { largest, .. }) => {
                assert_close(largest, 0.5 * k, "largest on the end of a rod");
            }
            other => panic!("expected too large, got {other:?}"),
        }
        let r = 0.49;
        let made = solve(&part, &[round(&part, k, [-1.0, 0.0, 3.0], r)], k);
        let (removed, _) = ring([1.0, 3.0], [-1.0, 0.0], [0.0, -1.0], r);
        assert_close(
            mass(&made.body).volume,
            (PI * 3.0 - removed) * k * k * k,
            "volume just below half the rod's radius",
        );
    }
}

#[test]
fn corners_that_are_not_built_are_refused_and_say_why() {
    let k = 1.0;
    let tol = tolerance(k);
    let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    // Three radii that differ.
    let edges = [
        round(&block, k, [0.0, 0.0, 1.0], 0.5),
        round(&block, k, [2.0, 0.0, 0.0], 0.5),
        round(&block, k, [0.0, 1.5, 0.0], 0.3),
    ];
    match fillet(&block, &edges, &tol) {
        Err(KernelError::Fillet { reason, .. }) => {
            assert!(reason.contains("different radii"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    // A corner with a circular edge: half a rod, its rim and the two
    // straight edges of the flat side that end where the rim does.
    let part = rod(&Frame::WORLD, k, 1.0, 3.0);
    let half = boxed(k, [-2.0, 0.0, -1.0], [2.0, 2.0, 4.0]);
    let halved = boolean(&part, &half, BooleanOp::Intersect, &tol)
        .unwrap()
        .body;
    // The upright edge of the flat side lies between a plane and a
    // cylinder (built on its own: `blend_cylinders.rs`); with the rim and
    // the top edge of the flat side it meets in a corner with a circular
    // edge.
    let edges = [
        round(&halved, k, [1.0, 0.0, 1.5], 0.2),
        round(&halved, k, [0.0, 1.0, 3.0], 0.2),
        round(&halved, k, [0.0, 0.0, 3.0], 0.2),
    ];
    match fillet(&halved, &edges, &tol) {
        Err(KernelError::Fillet { reason, .. }) => {
            assert!(reason.contains("corner that is not built"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    // A straight edge next to a cone that runs into its apex, where only
    // the cone and the cut meet: half a cone, cut through its axis.
    let part = primitives::cone(&Frame::WORLD, k, 3.0 * k).unwrap();
    let halved = boolean(&part, &half, BooleanOp::Intersect, &tol)
        .unwrap()
        .body;
    match fillet(&halved, &[round(&halved, k, [0.5, 0.0, 1.5], 0.2)], &tol) {
        Err(KernelError::Fillet { reason, .. }) => {
            assert!(reason.contains("does not end cleanly"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    // An outside and an inside fillet that meet: the inside corner of an
    // L together with the top edge that ends in it.
    let ell = prism(&Frame::WORLD, k, &ELL, 2.0);
    let edges = [
        round(&ell, k, [1.5, 1.5, 1.0], 0.3),
        round(&ell, k, [1.5, 2.25, 2.0], 0.3),
    ];
    match fillet(&ell, &edges, &tol) {
        Err(KernelError::Fillet { reason, .. }) => {
            assert!(reason.contains("does not end cleanly"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn bad_requests_are_errors() {
    let k = 1.0;
    let tol = tolerance(k);
    let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let good = round(&block, k, [0.0, 0.0, 1.0], 0.5);
    assert!(fillet(&block, &[good, good], &tol).is_err());
    let missing = FilletEdge {
        edge: EdgeId(99),
        ..good
    };
    assert!(fillet(&block, &[missing], &tol).is_err());
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-9] {
        let sized = FilletEdge {
            radius: bad,
            ..good
        };
        assert!(fillet(&block, &[sized], &tol).is_err(), "{bad}");
    }
    assert_eq!(fillet(&block, &[], &tol).unwrap().body, block);
    // A whole ball has no edge between two faces; its seam is refused.
    let ball = boolean_common::ball(k, [0.0, 0.0, 0.0], 1.0);
    for index in 0..ball.edges.len() {
        let request = FilletEdge {
            edge: EdgeId(index as u32),
            radius: 0.1,
        };
        assert!(fillet(&ball, &[request], &tol).is_err());
    }
    // An elliptic edge: a rod cut by a slanted plane.
    let part = rod(&Frame::WORLD, k, 1.0, 3.0);
    let tilted = Frame::new(
        Vec3::new(-3.0, -3.0, 1.5),
        Vec3::new(0.3, 0.0, 1.0),
        Vec3::X,
    )
    .unwrap();
    let cutter = primitives::block(&tilted, 6.0, 6.0, 6.0).unwrap();
    let cut = boolean(&part, &cutter, BooleanOp::Subtract, &tol)
        .unwrap()
        .body;
    let ellipse = (0..cut.edges.len())
        .find(|i| cut.edges[*i].curve.kind() == "ellipse")
        .expect("a slanted cut of a rod has an elliptic edge");
    let request = FilletEdge {
        edge: EdgeId(ellipse as u32),
        radius: 0.1,
    };
    // Built as a ring of spline surface since 2026-10-14 (its oracle is
    // in `blend_freeform.rs`); a radius the rod cannot hold is refused.
    let made = fillet(&cut, &[request], &tol).unwrap();
    assert_eq!(check(&made.body, &tol), []);
    let too_large = FilletEdge {
        radius: 1.5,
        ..request
    };
    assert!(fillet(&cut, &[too_large], &tol).is_err());
}

/// The contact fillets make where one goes on in the next, kept as the
/// smallest case: a quarter cylinder that continues in a quarter torus
/// across the border the two faces share, as the fillets of a straight
/// edge and of the arc it continues in do along the rim of a slot. The two
/// surfaces are tangent along that border (a circle), and beyond it they
/// cross in a second curve that leaves the border at its end
/// (`geom::intersect` has both in closed form since 2026-10-14; the
/// boolean refused the pair before). `fillet` does not ask this of the
/// boolean yet: it joins the tools of such edges into one tool first
/// (`blend::join`), a step that can go now.
///
/// The bar with its rounded edge and the tool of the arc's fillet share
/// that quarter circle where the tool starts, and half a turn on the tool
/// ends flat against the bar's end, so the union is both in one piece
/// (less that end, twice) and the difference is the bar; the volumes are
/// closed forms (the tool's by Pappus: half a turn of the corner a round
/// of radius r leaves).
#[test]
fn a_cylinder_that_goes_on_in_a_torus_touches_it_along_their_border() {
    use craft3d_kernel::revolve::revolve;
    for k in SCALES {
        let tol = tolerance(k);
        let r = 0.3;
        // A bar along x from -3 to 0 with one edge rounded: the edge y = 0,
        // z = 2.
        let bar = scale_curves(
            &[
                line(1, [0.0, 0.0], [2.0, 0.0]),
                line(2, [2.0, 0.0], [2.0, 2.0]),
                line(3, [2.0, 2.0], [r, 2.0]),
                arc(4, [r, 2.0 - r], [r, 2.0], [0.0, 2.0 - r]),
                line(5, [0.0, 2.0 - r], [0.0, 0.0]),
            ],
            k,
        );
        let frame = Frame::new(Vec3::new(-3.0 * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
        let profiles = detect_profiles(&bar, &tol).unwrap();
        let bar = extrude(&profiles, &frame, 0.0, 3.0 * k, &tol).unwrap().body;
        // The tool of the arc that edge continues in, about the upright axis
        // through (0, 1): the corner at distance 1 from the axis, turned half
        // way round from where the bar ends.
        let section = scale_curves(
            &[
                line(1, [1.0, 2.0], [1.0 - r, 2.0]),
                arc(2, [1.0 - r, 2.0 - r], [1.0, 2.0 - r], [1.0 - r, 2.0]),
                line(3, [1.0, 2.0 - r], [1.0, 2.0]),
            ],
            k,
        );
        let from = -Vec3::Y;
        let frame = Frame::new(Vec3::new(0.0, k, 0.0), from.cross(Vec3::Z), from).unwrap();
        let profiles = detect_profiles(&section, &tol).unwrap();
        let tool = revolve(&profiles, &frame, [0.0, 0.0], [0.0, 1.0], 0.0, PI, &tol)
            .unwrap()
            .body;
        let bar_volume = 3.0 * (4.0 - SPANDREL * r * r) * k * k * k;
        // ∫ρ dA over the square corner less the quarter disc, times π.
        let tool_volume = PI
            * (r * r * (1.0 - r / 2.0) - ((1.0 - r) * PI * r * r / 4.0 + r * r * r / 3.0))
            * k
            * k
            * k;
        assert_close(mass(&bar).volume, bar_volume, "the bar");
        assert_close(mass(&tool).volume, tool_volume, "the tool");
        // They lie against each other: the union is both, the difference
        // the bar, and they have nothing in common.
        let union = boolean(&bar, &tool, BooleanOp::Union, &tol).unwrap();
        assert_eq!(check(&union.body, &tol), []);
        assert_eq!(union.body.shells.len(), 1);
        let both = mass(&union.body);
        assert_close(both.volume, bar_volume + tool_volume, "volume");
        let end = SPANDREL * r * r * k * k;
        assert_close(
            both.area,
            mass(&bar).area + mass(&tool).area - 2.0 * end,
            "area",
        );
        // The tool's far end and its top are gone into the bar's.
        assert_eq!(
            union.body.faces.len(),
            bar.faces.len() + tool.faces.len() - 2
        );
        let difference = boolean(&bar, &tool, BooleanOp::Subtract, &tol).unwrap();
        assert_eq!(check(&difference.body, &tol), []);
        assert_close(mass(&difference.body).volume, bar_volume, "volume");
        assert_close(mass(&difference.body).area, mass(&bar).area, "area");
        assert_eq!(difference.body.faces.len(), bar.faces.len());
        let other = boolean(&tool, &bar, BooleanOp::Subtract, &tol).unwrap();
        assert_close(mass(&other.body).volume, tool_volume, "volume");
        assert!(matches!(
            boolean(&bar, &tool, BooleanOp::Intersect, &tol),
            Err(KernelError::EmptyResult { .. })
        ));
    }
}

/// The rim of a cap whose outline turns round tangent arcs by less than
/// a right angle (engine-block finding E5): the line where the fillet
/// touches the cap runs on round each arc, so the arc next to an edge
/// does not bound the fillet by how far it bends into the cap.
#[test]
fn a_rim_chain_round_shallow_tangent_arcs() {
    // The crankcase bay of the finding at a tenth: corners that turn by
    // 90, 90, 38.7, 51.3, 51.3 and 38.7 degrees, each rounded by 1.2.
    let points = [
        [-8.8, -7.1],
        [8.8, -7.1],
        [8.8, 4.0],
        [4.4, 9.5],
        [-4.4, 9.5],
        [-8.8, 4.0],
    ];
    let (big, r, height) = (1.2, 0.5, 2.0);
    let (curves, area, lines, turns) = rounded_polygon(&points, &[big; 6]);
    // Before 2026-10-15 the fit estimate took the far end of the shallow
    // arc as the end of the cap: below 0.27.
    assert!(turns.iter().any(|t| big * (1.0 - t.cos()) < r));
    // The spandrel's centroid lies this far in from both faces.
    let inset = r * (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI);
    let spandrel = SPANDREL * r * r;
    let removed = spandrel * (lines + turns.iter().map(|t| t * (big - inset)).sum::<f64>());
    for k in SCALES {
        for frame in frames(k) {
            let profiles = detect_profiles(&scale_curves(&curves, k), &tolerance(k)).unwrap();
            let part = extrude(&profiles, &frame, 0.0, height * k, &tolerance(k))
                .unwrap()
                .body;
            let seed = edge_in(&part, &frame, k, [0.0, -7.1, height]);
            let top = faces_of(&part, seed)
                .into_iter()
                .find(|f| {
                    part.face(*f).unwrap().surface.kind() == "plane"
                        && faces_of(&part, edge_in(&part, &frame, k, [0.0, 9.5, height]))
                            .contains(f)
                })
                .unwrap();
            let chain = tangent_chain(&part, seed, top).unwrap().unwrap();
            assert_eq!(chain.len(), 12);
            let edges: Vec<FilletEdge> = chain
                .iter()
                .map(|(edge, _)| FilletEdge {
                    edge: *edge,
                    radius: r * k,
                })
                .collect();
            let made = solve(&part, &edges, k);
            let got = mass(&made.body).volume;
            assert_close(got, (area * height - removed) * k * k * k, "volume");
        }
    }
}

/// The plate of engine-block finding E1 at a tenth (six of its eight side
/// edges rounded by 1.4, the outline drawn with its arcs or made by
/// filleting the sharp plate), its cap rim rounded as one chain that ends
/// in the two sharp corners, at the finding's tolerance. Before
/// 2026-10-15 the estimate took the far end of the arc next to the edge
/// as the end of the cap ("below 1.13" at the finding's size).
#[test]
fn a_rim_chain_that_ends_in_two_sharp_corners() {
    let (curves, area, lines, turns, sharp) = common::plate_e1();
    let (r, height) = (0.15, 1.4);
    let inset = r * (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI);
    // The spandrel's centroid runs along the outline moved in by `inset`,
    // round the arcs, and past each sharp corner to the plane of the
    // side that is not rounded (`inset / tan(turn)` further).
    let path = lines - 24.0
        + (0..8)
            .filter(|i| !sharp.contains(i))
            .map(|i| turns[i] * (1.4 - inset))
            .sum::<f64>()
        + sharp.iter().map(|i| inset / turns[*i].tan()).sum::<f64>();
    let removed = SPANDREL * r * r * path;
    for (k, by_fillet) in SCALES.into_iter().flat_map(|k| [(k, false), (k, true)]) {
        let fine = craft3d_math::Tolerance::new(1e-7 * k, 1e-9).unwrap();
        for frame in frames(k) {
            let part = if by_fillet {
                let sharp_plate = polygon(1, 1, &common::PLATE_E1);
                let profiles = detect_profiles(&scale_curves(&sharp_plate, k), &fine).unwrap();
                let plate = extrude(&profiles, &frame, 0.0, height * k, &fine)
                    .unwrap()
                    .body;
                let rounds: Vec<FilletEdge> = common::PLATE_E1[2..]
                    .iter()
                    .map(|p| round_in(&plate, &frame, k, [p[0], p[1], 0.5 * height], 1.4))
                    .collect();
                fillet(&plate, &rounds, &fine).unwrap().body
            } else {
                let profiles = detect_profiles(&scale_curves(&curves, k), &fine).unwrap();
                extrude(&profiles, &frame, 0.0, height * k, &fine)
                    .unwrap()
                    .body
            };
            let seed = edge_in(&part, &frame, k, [15.0, 4.5, height]);
            let top = faces_of(&part, seed)
                .into_iter()
                .find(|f| {
                    part.face(*f).unwrap().surface.kind() == "plane"
                        && faces_of(&part, edge_in(&part, &frame, k, [0.0, 17.5, height]))
                            .contains(f)
                })
                .unwrap();
            let chain = tangent_chain(&part, seed, top).unwrap().unwrap();
            assert_eq!(chain.len(), 13);
            let edges: Vec<FilletEdge> = chain
                .iter()
                .map(|(edge, _)| FilletEdge {
                    edge: *edge,
                    radius: r * k,
                })
                .collect();
            let made = match fillet(&part, &edges, &fine) {
                Ok(made) => made,
                Err(e) => panic!("scale {k}, by fillet {by_fillet}: {e}"),
            };
            assert_eq!(check(&made.body, &fine), []);
            assert_tangent(&part, &edges, &made);
            assert_close(
                mass(&made.body).volume,
                (area * height - removed) * k * k * k,
                "volume",
            );
        }
    }
}

/// An upright on a plate whose end leans (one way, then the other), its
/// foot along the plate rounded; at the leaning end the round meets a
/// convex edge and a concave one, and ends in the plane of the end face
/// (engine-block findings E3, E4: a flange root that ends on a chamfer or
/// at a corner of the outline). The added prism lies between the square
/// end x = 2 and the leaning plane: its volume is the section times the
/// line through its centroid between them.
#[test]
fn a_root_that_ends_on_a_leaning_face() {
    let r = 0.6;
    let inset = r * (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI);
    // The trapezoid in (x, z), its area, and the centroid line's length.
    let leans: [([[f64; 2]; 4], f64, f64); 2] = [
        (
            [[2.0, 0.0], [8.0, 0.0], [6.0, 5.0], [2.0, 5.0]],
            25.0,
            6.0 - 0.4 * inset,
        ),
        (
            [[2.0, 0.0], [7.0, 0.0], [8.0, 5.0], [2.0, 5.0]],
            27.5,
            5.0 + 0.2 * inset,
        ),
    ];
    for (outline, area, length) in leans {
        for k in SCALES {
            for frame in frames(k) {
                let tol = tolerance(k);
                let plate_frame = frame.at(frame.point(0.0, 0.0, -k)).unwrap();
                let plate = primitives::block(&plate_frame, 10.0 * k, 10.0 * k, k).unwrap();
                let up_frame =
                    Frame::new(frame.point(0.0, 4.0 * k, 0.0), -frame.y(), frame.x()).unwrap();
                let curves = scale_curves(&polygon(1, 1, &outline), k);
                let profiles = detect_profiles(&curves, &tol).unwrap();
                let upright = extrude(&profiles, &up_frame, 0.0, 4.0 * k, &tol)
                    .unwrap()
                    .body;
                let part = boolean(&plate, &upright, BooleanOp::Union, &tol)
                    .unwrap()
                    .body;
                let middle = 0.5 * (outline[0][0] + outline[1][0]);
                let request = round_in(&part, &frame, k, [middle, 4.0, 0.0], r);
                let made = solve(&part, &[request], k);
                let volume = 100.0 + 4.0 * area + SPANDREL * r * r * length;
                assert_close(mass(&made.body).volume, volume * k * k * k, "volume");
            }
        }
    }
}

/// An L-shaped upright on a plate, its inside corner rounded by 0.8 and
/// the ledge beyond the arc only 0.4 long before the outline turns away:
/// the root of the upright rounded as one chain (engine-block finding E3,
/// the bellhousing's rail root). The estimate measured the short ledge as
/// what bounds the plate beside the arc ("below 0.094"); it lies beyond
/// the arc's sector and bounds nothing of it. The bead's centroid runs
/// round the arc at `0.8 - inset` and along both lines to the square ends.
#[test]
fn a_root_chain_round_an_inside_arc_with_a_short_ledge() {
    let outline = [
        [0.0, 0.0],
        [2.2, 0.0],
        [2.2, 1.0],
        [1.0, 1.0],
        [1.0, 4.0],
        [0.0, 4.0],
    ];
    let (big, r, height) = (0.8, 0.3, 2.0);
    let (curves, area, _, turns) =
        common::rounded_polygon(&outline, &[0.0, 0.0, 0.0, big, 0.0, 0.0]);
    let inset = r * (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI);
    let path = 0.4 + 2.2 + turns[3].abs() * (big - inset);
    let added = SPANDREL * r * r * path;
    for k in SCALES {
        for frame in frames(k) {
            let tol = tolerance(k);
            let plate_frame = frame.at(frame.point(-k, -k, -k)).unwrap();
            let plate = primitives::block(&plate_frame, 5.0 * k, 6.0 * k, k).unwrap();
            let profiles = detect_profiles(&scale_curves(&curves, k), &tol).unwrap();
            let upright = extrude(&profiles, &frame, 0.0, height * k, &tol)
                .unwrap()
                .body;
            let part = boolean(&plate, &upright, BooleanOp::Union, &tol)
                .unwrap()
                .body;
            let seed = edge_in(&part, &frame, k, [1.0, 2.9, 0.0]);
            let floor = faces_of(&part, seed)
                .into_iter()
                .find(|f| faces_of(&part, edge_in(&part, &frame, k, [2.0, 1.0, 0.0])).contains(f))
                .unwrap();
            let chain = tangent_chain(&part, seed, floor).unwrap().unwrap();
            assert_eq!(chain.len(), 3);
            let edges: Vec<FilletEdge> = chain
                .iter()
                .map(|(edge, _)| FilletEdge {
                    edge: *edge,
                    radius: r * k,
                })
                .collect();
            let made = solve(&part, &edges, k);
            let volume = 5.0 * 6.0 + area * height + added;
            assert_close(mass(&made.body).volume, volume * k * k * k, "volume");
        }
    }
}

/// Engine-block finding E5b: a corner of the crankcase bay (a hexagon
/// whose corner at (8.8, 4) turns by 51.3 degrees) with its long edge and
/// both rims there rounded by one radius: a piece of the ball. At the
/// finding's tolerance the boolean refused the second rim's tool ("two
/// points closer together than the tolerance can tell apart"): the end
/// of an ellipse and of a circle that both run onto the cap there were
/// found as hits on the cap a little apart, where they share a vertex
/// that lies on it exactly. Held against the definition of a rolling-ball
/// fillet near the corner.
#[test]
fn a_ball_corner_where_the_outline_turns_by_less_than_a_right_angle() {
    let points = [
        [-8.8, -7.1],
        [8.8, -7.1],
        [8.8, 4.0],
        [4.4, 9.5],
        [-4.4, 9.5],
        [-8.8, 4.0],
    ];
    let (h, r) = (6.6, 0.8);
    // The prism's planes, outward.
    let mut planes: Vec<(Vec3, f64)> = (0..6)
        .map(|i| {
            let (p, q) = (points[i], points[(i + 1) % 6]);
            let n = Vec3::new(q[1] - p[1], p[0] - q[0], 0.0)
                .normalized()
                .unwrap();
            (n, n.dot(Vec3::new(p[0], p[1], 0.0)))
        })
        .collect();
    planes.push((-Vec3::Z, 0.0));
    planes.push((Vec3::Z, h));
    let at_corner = [planes[1], planes[2], planes[7]];
    for k in SCALES {
        let fine = craft3d_math::Tolerance::new(1e-7 * k, 1e-9).unwrap();
        let part = prism(&Frame::WORLD, k, &points, h);
        let edges = [
            round(&part, k, [8.8, 4.0, h / 2.0], r),
            round(&part, k, [8.8, -1.55, h], r),
            round(&part, k, [6.6, 6.75, h], r),
        ];
        let made = match fillet(&part, &edges, &fine) {
            Ok(made) => made,
            Err(e) => panic!("scale {k}: {e}"),
        };
        assert_eq!(check(&made.body, &fine), []);
        assert_tangent(&part, &edges, &made);
        assert_eq!(made.body.faces.len(), 8 + 3 + 1);
        assert_matches_definition(
            &made.body,
            k,
            &planes,
            &at_corner,
            r,
            Vec3::new(8.8, 4.0, h),
            23,
        );
    }
}

/// Engine-block finding E8: a slot-shaped rib on a wall whose lower end
/// touches exactly the line where the wall goes over into the round of a
/// rail below it. The root of the rib, rounded as one chain, would have
/// to run onto that round; and the wall pinches to nothing between the
/// rib's foot and the round where they touch. The boolean refuses the
/// tool ("an imprinted curve that does not fit"), and a quarter of the
/// radius fails the same way: it is refused with the boolean's reason,
/// saying that smaller radii fail too (until 2026-10-15 it was called too
/// large with nothing that fits, which nothing in the result showed).
/// Moved clear by more than the radius, the same chain is built.
#[test]
fn a_rib_that_touches_the_round_below_it() {
    let r = 0.3;
    for k in SCALES {
        for frame in frames(k) {
            let tol = tolerance(k);
            let block = |lo: [f64; 3], size: [f64; 3]| {
                let at = frame
                    .at(frame.point(lo[0] * k, lo[1] * k, lo[2] * k))
                    .unwrap();
                primitives::block(&at, size[0] * k, size[1] * k, size[2] * k).unwrap()
            };
            let wall = block([-10.0, -9.8, -6.0], [20.0, 4.0, 12.0]);
            let ledge = block([-10.0, -11.0, -6.0], [20.0, 1.2, 2.0]);
            let joined = boolean(&wall, &ledge, BooleanOp::Union, &tol).unwrap().body;
            // The rail's round: the wall goes over into it at z = -3.2.
            let rail = round_in(&joined, &frame, k, [0.0, -9.8, -4.0], 0.8);
            let railed = fillet(&joined, &[rail], &tol).unwrap().body;
            for (lift, builds) in [(0.0, false), (0.5, true)] {
                // The slot from u = -3.2 + lift to 4.5, u along z, standing
                // 0.7 out of the wall.
                let (low, high) = (-2.7 + lift, 4.0);
                let curves = scale_curves(
                    &[
                        line(1, [low, -0.5], [high, -0.5]),
                        arc(2, [high, 0.0], [high, -0.5], [high, 0.5]),
                        line(3, [high, 0.5], [low, 0.5]),
                        arc(4, [low, 0.0], [low, 0.5], [low, -0.5]),
                    ],
                    k,
                );
                let rib_frame =
                    Frame::new(frame.point(0.0, -9.6 * k, 0.0), -frame.y(), frame.z()).unwrap();
                let profiles = detect_profiles(&curves, &tol).unwrap();
                let rib = extrude(&profiles, &rib_frame, 0.0, 0.9 * k, &tol)
                    .unwrap()
                    .body;
                let part = boolean(&railed, &rib, BooleanOp::Union, &tol).unwrap().body;
                let seed = edge_in(&part, &frame, k, [0.5, -9.8, 1.0]);
                let wall_face = faces_of(&part, seed)
                    .into_iter()
                    .find(|f| {
                        part.face(*f).unwrap().surface.kind() == "plane" && {
                            let n = normal_at(&part, *f, frame.point(0.5 * k, -9.8 * k, k));
                            n.dot(-frame.y()) > 0.99
                        }
                    })
                    .unwrap();
                let chain = tangent_chain(&part, seed, wall_face).unwrap().unwrap();
                let edges: Vec<FilletEdge> = chain
                    .iter()
                    .map(|(edge, _)| FilletEdge {
                        edge: *edge,
                        radius: r * k,
                    })
                    .collect();
                match fillet(&part, &edges, &tol) {
                    Ok(made) if builds => {
                        assert_eq!(check(&made.body, &tol), []);
                        assert_tangent(&part, &edges, &made);
                        // Wall, ledge, the rail's bead, the rib's 0.7 out
                        // of the wall, and the root's bead round the slot
                        // (its centroid `inset` out from the rib).
                        let inset = r * (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI);
                        let long = high - low;
                        let volume = 20.0 * 4.0 * 12.0
                            + 20.0 * 1.2 * 2.0
                            + SPANDREL * 0.8 * 0.8 * 20.0
                            + (long + PI * 0.25) * 0.7
                            + SPANDREL * r * r * (2.0 * long + 2.0 * PI * (0.5 + inset));
                        assert_close(mass(&made.body).volume, volume * k * k * k, "volume");
                    }
                    Err(KernelError::FilletBoolean {
                        edge,
                        smaller_fails,
                        ..
                    }) if !builds => {
                        assert!(edges.iter().any(|e| e.edge == edge), "{edge:?}");
                        assert!(smaller_fails, "lift {lift} at scale {k}");
                    }
                    other => panic!("lift {lift} at scale {k}: {other:?}"),
                }
            }
        }
    }
}

/// A document tolerance as fine as a float allows (1e-300): a radius too
/// large for a block is refused, and the search for what fits ends (its
/// bisection once went on for ever when the middle rounded to an end).
#[test]
fn a_radius_too_large_at_the_finest_tolerance_is_refused() {
    let block = boxed(1.0, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let tol = Tolerance::new(1e-300, 1e-9).unwrap();
    let request = round(&block, 1.0, [0.0, 0.0, 1.0], 3.5);
    assert!(fillet(&block, &[request], &tol).is_err());
}

/// A straight edge between two planes, alone at both its ends, is built
/// in place (one blend construction, `docs/design/blends.md`): its face
/// is the exact cylinder the translation along the edge sweeps the ball's
/// arc into, and every edge of that face is the line or circle in which
/// its two surfaces meet, not a fitted curve.
#[test]
fn an_edge_built_in_place_has_an_exact_face_and_exact_edges() {
    for k in SCALES {
        for frame in frames(k) {
            let part = prism(
                &frame,
                k,
                &[[0.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]],
                2.0,
            );
            let r = 0.5;
            let made = solve(&part, &[round_in(&part, &frame, k, [2.0, 0.0, 2.0], r)], k);
            let area = 52.0 - 2.0 * r * 4.0 - 2.0 * SPANDREL * r * r + PI / 2.0 * r * 4.0;
            assert_mass(&made, 24.0 - SPANDREL * r * r * 4.0, area, k);
            let [face] = fillet_faces(&made)[..] else {
                panic!("one fillet face");
            };
            let data = &made.body.faces[face];
            assert!(
                matches!(data.surface, Surface::Cylinder(_)),
                "{:?}",
                data.surface
            );
            for l in &data.loops {
                for coedge in &made.body.loop_(*l).unwrap().coedges {
                    let curve = &made.body.edge(coedge.edge).unwrap().curve;
                    assert!(
                        matches!(
                            curve,
                            craft3d_geom::Curve3::Line(_) | craft3d_geom::Curve3::Circle(_)
                        ),
                        "an edge of the fillet face is a {}",
                        curve.kind()
                    );
                }
            }
        }
    }
}

/// A hole that lies wholly in the strip a fillet would take of the face
/// beside its edge is not crossed by the fillet's border, and is refused
/// all the same: the fillet would have to run through it. What fits is
/// the radius at which the border reaches the hole (its nearest point is
/// 0.3 from the edge, and the tangent line of a square corner lies r from
/// the edge); just below it builds, against the closed form (the block
/// less the hole less the corner).
#[test]
fn a_hole_wholly_inside_the_strip_a_fillet_takes_is_refused() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let (rh, at) = (0.3, 0.6);
        let drill = rod(
            &Frame::WORLD.at(Vec3::new(2.0 * k, at * k, -k)).unwrap(),
            k,
            rh,
            4.0,
        );
        let holed = boolean(&block, &drill, BooleanOp::Subtract, &tolerance(k))
            .unwrap()
            .body;
        // The top front edge (its middle is nearer this point than the
        // middle of the hole's mouth is).
        match fillet(
            &holed,
            &[round(&holed, k, [2.0, -1.0, 2.5], 1.0)],
            &tolerance(k),
        ) {
            Err(KernelError::FilletTooLarge { largest, .. }) => {
                assert!(
                    (largest - (at - rh) * k).abs() <= 1e-3 * k,
                    "largest {largest}"
                );
            }
            other => panic!("expected too large, got {other:?}"),
        }
        let r = 0.25;
        let made = solve(&holed, &[round(&holed, k, [2.0, -1.0, 2.5], r)], k);
        assert_close(
            mass(&made.body).volume,
            (24.0 - PI * rh * rh * 2.0 - SPANDREL * r * r * 4.0) * k * k * k,
            "volume",
        );
    }
}

/// The fillet of `chamfer.rs`'s E7 feature: on a block whose two long top
/// edges an earlier chamfer took, the top edge across the end (built in
/// place, it takes the chamfer's end edges whole, so the body it leaves
/// numbers its edges anew) with the vertical edge at the far corner and
/// the chamfer's end edge above it (built by the tools on that body).
/// Each request is built where it was asked for: every fillet face is a
/// cylinder of its request's radius along its request's edge, and the
/// solid is the one the in-place fillet and then the other two, found
/// again by where they are, make in two features.
#[test]
fn edges_after_one_a_fillet_takes_whole_keep_their_requests() {
    use craft3d_kernel::chamfer::{ChamferEdge, ChamferSize, chamfer};
    for k in SCALES {
        let tol = tolerance(k);
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let cut = |middle: [f64; 3]| {
            let edge = edge_in(&block, &Frame::WORLD, k, middle);
            ChamferEdge {
                edge,
                first: faces_of(&block, edge)[0],
                size: ChamferSize::Distances {
                    first: 0.5 * k,
                    second: 0.5 * k,
                },
            }
        };
        let first = chamfer(&block, &[cut([2.0, 0.0, 2.0]), cut([2.0, 3.0, 2.0])], &tol)
            .unwrap()
            .body;
        let middles = [[0.0, 1.5, 2.0], [4.0, 2.75, 1.75], [4.0, 3.0, 0.75]];
        let radii = [0.5, 0.2, 0.2];
        let requests: Vec<FilletEdge> = middles
            .iter()
            .zip(radii)
            .map(|(m, r)| round(&first, k, *m, r))
            .collect();
        let taken = edge_in(&first, &Frame::WORLD, k, [0.0, 2.75, 1.75]);
        assert!(requests[1].edge > taken);
        let made = solve(&first, &requests, k);
        let along = [Vec3::Y, Vec3::new(0.0, -1.0, 1.0), Vec3::Z];
        for (index, axis) in along.iter().enumerate() {
            let faces: Vec<usize> = fillet_faces(&made)
                .into_iter()
                .filter(
                    |f| matches!(&made.faces[*f], FilletFace::Fillet(of) if of.contains(&index)),
                )
                .collect();
            assert!(!faces.is_empty(), "scale {k}: no face for request {index}");
            for face in faces {
                let Surface::Cylinder(cylinder) = &made.body.faces[face].surface else {
                    panic!("scale {k}: the fillet face of request {index} is not a cylinder");
                };
                assert_close(cylinder.radius(), radii[index] * k, "radius");
                let z = cylinder.frame().z();
                assert!(
                    z.cross(axis.normalized().unwrap()).length() < 1e-9,
                    "scale {k}: the fillet face of request {index} is not along its edge"
                );
            }
        }
        let once = solve(&first, &requests[..1], k);
        let rest: Vec<FilletEdge> = middles[1..]
            .iter()
            .zip(&radii[1..])
            .map(|(m, r)| round(&once.body, k, *m, *r))
            .collect();
        let twice = solve(&once.body, &rest, k);
        let (m, t) = (mass(&made.body), mass(&twice.body));
        assert_close(m.volume, t.volume, "volume against two fillets");
        assert_close(m.area, t.area, "area against two fillets");
        assert_eq!(counts(&made), counts(&twice));
    }
}
