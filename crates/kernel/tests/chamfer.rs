//! Chamfers against closed forms (docs/testing.md, oracle 1): every result
//! passes the checker and has the volume, area and counts that follow from
//! the sizes, at three scales.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::{SCALES, assert_close, boxed, mass, tolerance};
use craft3d_kernel::KernelError;
use craft3d_kernel::chamfer::{ChamferEdge, ChamferFace, ChamferResult, ChamferSize, chamfer};
use craft3d_math::Vec3;
use craft3d_topo::{Body, EdgeId, FaceId, check, genus};

/// The edge whose middle is nearest `middle` (given for scale 1).
fn edge_at(body: &Body, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = Vec3::new(middle[0] * k, middle[1] * k, middle[2] * k);
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

/// The face of `edge` whose outward normal is `normal`.
fn face_facing(body: &Body, edge: EdgeId, normal: [f64; 3]) -> FaceId {
    let want = Vec3::new(normal[0], normal[1], normal[2]);
    faces_of(body, edge)
        .into_iter()
        .find(|id| {
            let face = body.face(*id).unwrap();
            let n = face.surface.normal(0.0, 0.0).unwrap();
            let n = if face.reversed { -n } else { n };
            n.dot(want) > 0.999
        })
        .unwrap()
}

fn equal(body: &Body, k: f64, middle: [f64; 3], d: f64) -> ChamferEdge {
    let edge = edge_at(body, k, middle);
    ChamferEdge {
        edge,
        first: faces_of(body, edge)[0],
        size: ChamferSize::Distances {
            first: d * k,
            second: d * k,
        },
    }
}

fn solve(body: &Body, edges: &[ChamferEdge], k: f64) -> ChamferResult {
    let tol = tolerance(k);
    let result = match chamfer(body, edges, &tol) {
        Ok(result) => result,
        Err(e) => panic!("chamfer at scale {k}: {e}"),
    };
    assert_eq!(check(&result.body, &tol), [], "at scale {k}");
    assert_eq!(result.faces.len(), result.body.faces.len());
    assert_eq!(result.edges.len(), result.body.edges.len());
    assert_eq!(result.vertices.len(), result.body.vertices.len());
    result
}

fn counts(result: &ChamferResult) -> (usize, usize, usize) {
    let b = &result.body;
    (b.vertices.len(), b.edges.len(), b.faces.len())
}

#[track_caller]
fn assert_mass(result: &ChamferResult, volume: f64, area: f64, k: f64) {
    let m = mass(&result.body);
    assert_close(m.volume, volume * k * k * k, "volume");
    assert_close(m.area, area * k * k, "area");
}

/// The volume of the convex solid that the half-spaces `n · p <= c` leave,
/// worked out here from nothing but the planes: its corners are the points
/// three planes share that are inside all the others, and each face is a
/// fan of triangles about one of its corners.
fn convex_volume(planes: &[([f64; 3], f64)]) -> f64 {
    let planes: Vec<(Vec3, f64)> = planes
        .iter()
        .map(|(n, c)| {
            let n = Vec3::new(n[0], n[1], n[2]);
            (n * (1.0 / n.length()), c / n.length())
        })
        .collect();
    let size = planes.iter().map(|p| p.1.abs()).fold(0.0, f64::max);
    let eps = 1e-9 * size;
    let mut corners: Vec<Vec3> = Vec::new();
    for i in 0..planes.len() {
        for j in i + 1..planes.len() {
            for l in j + 1..planes.len() {
                let (a, b, c) = (planes[i], planes[j], planes[l]);
                let det = a.0.dot(b.0.cross(c.0));
                if det.abs() < 1e-9 {
                    continue;
                }
                let p = (b.0.cross(c.0) * a.1 + c.0.cross(a.0) * b.1 + a.0.cross(b.0) * c.1)
                    * (1.0 / det);
                let inside = planes.iter().all(|(n, c)| n.dot(p) <= c + eps);
                if inside && corners.iter().all(|q| q.distance(p) > eps) {
                    corners.push(p);
                }
            }
        }
    }
    let centre = corners.iter().fold(Vec3::ZERO, |s, p| s + *p) * (1.0 / corners.len() as f64);
    let mut volume = 0.0;
    for (n, c) in &planes {
        let mut on: Vec<Vec3> = corners
            .iter()
            .copied()
            .filter(|p| (n.dot(*p) - c).abs() <= eps)
            .collect();
        if on.len() < 3 {
            continue;
        }
        let middle = on.iter().fold(Vec3::ZERO, |s, p| s + *p) * (1.0 / on.len() as f64);
        let x = (on[0] - middle).normalized().unwrap();
        let y = n.cross(x);
        on.sort_by(|p, q| {
            let angle = |v: Vec3| (v - middle).dot(y).atan2((v - middle).dot(x));
            angle(*p).total_cmp(&angle(*q))
        });
        for pair in 0..on.len() {
            let (p, q) = (on[pair], on[(pair + 1) % on.len()]);
            volume += (middle - centre).dot((p - centre).cross(q - centre)) / 6.0;
        }
    }
    volume.abs()
}

/// The six faces of the box from the origin to `size`.
fn box_planes(size: [f64; 3]) -> Vec<([f64; 3], f64)> {
    vec![
        ([-1.0, 0.0, 0.0], 0.0),
        ([0.0, -1.0, 0.0], 0.0),
        ([0.0, 0.0, -1.0], 0.0),
        ([1.0, 0.0, 0.0], size[0]),
        ([0.0, 1.0, 0.0], size[1]),
        ([0.0, 0.0, 1.0], size[2]),
    ]
}

#[test]
fn the_oracle_for_convex_solids_knows_a_box_and_a_cut_corner() {
    assert_close(convex_volume(&box_planes([4.0, 3.0, 2.0])), 24.0, "box");
    let mut planes = box_planes([4.0, 3.0, 2.0]);
    planes.push(([-1.0, -1.0, -1.0], -1.0));
    assert_close(convex_volume(&planes), 24.0 - 1.0 / 6.0, "corner");
}

#[test]
fn one_edge_of_a_block() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let d = 0.5;
        let made = solve(&block, &[equal(&block, k, [0.0, 0.0, 1.0], d)], k);
        let area = 52.0 - 2.0 * d * 2.0 - d * d + 2.0_f64.sqrt() * d * 2.0;
        assert_mass(&made, 24.0 - d * d / 2.0 * 2.0, area, k);
        assert_eq!(counts(&made), (10, 15, 7));
        let chamfers = made
            .faces
            .iter()
            .filter(|f| matches!(f, ChamferFace::Chamfer(list) if list == &[0]))
            .count();
        assert_eq!(chamfers, 1);
        let kept = made
            .faces
            .iter()
            .filter(|f| matches!(f, ChamferFace::Kept(list) if list.len() == 1))
            .count();
        assert_eq!(kept, 6);
        // The four far corners and nine edges are the block's own.
        assert_eq!(made.vertices.iter().flatten().count(), 6);
    }
}

#[test]
fn two_edges_that_meet_are_mitred() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        for (da, db) in [(0.5, 0.5), (0.3, 0.7), (0.7, 0.3)] {
            let made = solve(
                &block,
                &[
                    equal(&block, k, [0.0, 0.0, 1.0], da),
                    equal(&block, k, [2.0, 0.0, 0.0], db),
                ],
                k,
            );
            let mut planes = box_planes([4.0, 3.0, 2.0]);
            planes.push(([-1.0, -1.0, 0.0], -da));
            planes.push(([0.0, -1.0, -1.0], -db));
            let m = mass(&made.body);
            assert_close(m.volume, convex_volume(&planes) * k * k * k, "volume");
            // The closed form: two prisms less what they share.
            let low = da.min(db);
            let shared = da * db * low - (da + db) * low * low / 2.0 + low * low * low / 3.0;
            let volume = 24.0 - da * da / 2.0 * 2.0 - db * db / 2.0 * 4.0 + shared;
            assert_close(m.volume, volume * k * k * k, "volume, closed form");
            assert_eq!(made.body.faces.len(), 8);
        }
    }
}

#[test]
fn three_edges_meet_in_a_flat_corner() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let d = 0.5;
        let made = solve(
            &block,
            &[
                equal(&block, k, [0.0, 0.0, 1.0], d),
                equal(&block, k, [2.0, 0.0, 0.0], d),
                equal(&block, k, [0.0, 1.5, 0.0], d),
            ],
            k,
        );
        // Three prisms, less what each two share, plus what all three
        // share, and the tip the corner face takes.
        let volume =
            24.0 - d * d / 2.0 * 9.0 + 3.0 * d * d * d / 3.0 - d * d * d / 4.0 - d * d * d / 12.0;
        let m = mass(&made.body);
        assert_close(m.volume, volume * k * k * k, "volume");
        let mut planes = box_planes([4.0, 3.0, 2.0]);
        planes.push(([-1.0, -1.0, 0.0], -d));
        planes.push(([0.0, -1.0, -1.0], -d));
        planes.push(([-1.0, 0.0, -1.0], -d));
        planes.push(([-1.0, -1.0, -1.0], -2.0 * d));
        assert_close(m.volume, convex_volume(&planes) * k * k * k, "volume");
        assert_eq!(made.body.faces.len(), 10);
        let corners = made
            .faces
            .iter()
            .filter(|f| matches!(f, ChamferFace::Corner(_)))
            .count();
        assert_eq!(corners, 1);
    }
}

#[test]
fn all_twelve_edges_of_a_block() {
    for k in SCALES {
        let (l, w, h) = (4.0, 3.0, 2.0);
        let block = boxed(k, [0.0, 0.0, 0.0], [l, w, h]);
        let d = 0.25;
        let edges: Vec<ChamferEdge> = (0..12)
            .map(|i| {
                let edge = EdgeId(i);
                ChamferEdge {
                    edge,
                    first: faces_of(&block, edge)[0],
                    size: ChamferSize::Distances {
                        first: d * k,
                        second: d * k,
                    },
                }
            })
            .collect();
        let made = solve(&block, &edges, k);
        let volume = l * w * h - 2.0 * d * d * (l + w + h) + 16.0 * d * d * d / 3.0;
        let (a, b, c) = (l - 2.0 * d, w - 2.0 * d, h - 2.0 * d);
        let area = 2.0 * (a * b + a * c + b * c)
            + 4.0 * 2.0_f64.sqrt() * d * (a + b + c)
            + 8.0 * 3.0_f64.sqrt() / 2.0 * d * d;
        assert_mass(&made, volume, area, k);
        assert_eq!(counts(&made), (24, 48, 26));
    }
}

#[test]
fn a_distance_too_large_names_the_edge_the_face_and_what_fits() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let edge = edge_at(&block, k, [0.0, 0.0, 1.0]);
        let narrow = face_facing(&block, edge, [-1.0, 0.0, 0.0]);
        let request = equal(&block, k, [0.0, 0.0, 1.0], 3.5);
        match chamfer(&block, &[request], &tolerance(k)) {
            Err(KernelError::ChamferTooLarge {
                edge: e,
                face,
                distance,
                largest,
            }) => {
                assert_eq!((e, face), (edge, narrow));
                assert_close(distance, 3.5 * k, "distance");
                assert_close(largest, 3.0 * k, "largest");
            }
            other => panic!("expected too large, got {other:?}"),
        }
        // Two chamfers that meet in the middle of a face.
        let pair = [
            equal(&block, k, [0.0, 0.0, 1.0], 1.6),
            equal(&block, k, [0.0, 3.0, 1.0], 1.6),
        ];
        match chamfer(&block, &pair, &tolerance(k)) {
            Err(KernelError::ChamferTooLarge { face, largest, .. }) => {
                assert_eq!(face, narrow);
                assert_close(largest, 1.4 * k, "largest");
            }
            other => panic!("expected too large, got {other:?}"),
        }
    }
}

mod common;

use common::{frames, polygon, scale_curves};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::Frame;
use std::f64::consts::PI;

/// The polygon through `points`, extruded from 0 to `height` in `frame`.
fn prism(frame: &Frame, k: f64, points: &[[f64; 2]], height: f64) -> Body {
    let curves = scale_curves(&polygon(1, 1, points), k);
    let profiles = detect_profiles(&curves, &tolerance(k)).unwrap();
    extrude(&profiles, frame, 0.0, height * k, &tolerance(k))
        .unwrap()
        .body
}

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

fn equal_in(body: &Body, frame: &Frame, k: f64, middle: [f64; 3], d: f64) -> ChamferEdge {
    let edge = edge_in(body, frame, k, middle);
    ChamferEdge {
        edge,
        first: faces_of(body, edge)[0],
        size: ChamferSize::Distances {
            first: d * k,
            second: d * k,
        },
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
            let d = 0.4;
            let made = solve(&ell, &[equal_in(&ell, &frame, k, [1.5, 1.5, 1.0], d)], k);
            // The two walls lose a strip each, the caps gain a triangle
            // each, and the chamfer face is new.
            let area = 2.0 * 8.25 + 14.0 * 2.0 - 2.0 * d * 2.0
                + 2.0 * d * d / 2.0
                + 2.0_f64.sqrt() * d * 2.0;
            assert_mass(&made, 16.5 + d * d / 2.0 * 2.0, area, k);
            assert_eq!(counts(&made), (14, 21, 9));
        }
    }
}

#[test]
fn an_edge_that_ends_against_a_wall_stops_there() {
    for k in SCALES {
        for frame in frames(k) {
            let ell = prism(&frame, k, &ELL, 2.0);
            let d = 0.4;
            // The top edge of the wall x = 1.5: one end is a corner of the
            // part, the other the inside corner, where the other wall
            // stands across it.
            let made = solve(&ell, &[equal_in(&ell, &frame, k, [1.5, 2.25, 2.0], d)], k);
            let m = mass(&made.body);
            assert_close(m.volume, (16.5 - d * d / 2.0 * 1.5) * k * k * k, "volume");
            // The wall it runs into has grown by a triangle and is still
            // one face.
            assert_eq!(made.body.faces.len(), 9);
        }
    }
}

#[test]
fn an_edge_that_ends_against_a_slanted_wall_is_refused() {
    let k = 1.0;
    let points = [
        [0.0, 0.0],
        [4.0, 0.0],
        [4.0, 1.5],
        [2.0, 1.5],
        [1.5, 3.0],
        [0.0, 3.0],
    ];
    let part = prism(&Frame::WORLD, k, &points, 2.0);
    let request = equal(&part, k, [1.75, 2.25, 2.0], 0.3);
    match chamfer(&part, &[request], &tolerance(k)) {
        Err(KernelError::Chamfer { edge, reason, .. }) => {
            assert_eq!(edge, Some(request.edge));
            assert!(reason.contains("does not end cleanly"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn two_distances_and_a_distance_with_an_angle() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let edge = edge_at(&block, k, [0.0, 0.0, 1.0]);
        // The first distance is along the face x = 0, so it is a length
        // in y; the second along y = 0, a length in x.
        let left = face_facing(&block, edge, [-1.0, 0.0, 0.0]);
        let front = face_facing(&block, edge, [0.0, -1.0, 0.0]);
        let (d1, d2) = (0.9, 0.3);
        let cut = |first: FaceId, size: ChamferSize| {
            solve(&block, &[ChamferEdge { edge, first, size }], k)
        };
        let two = ChamferSize::Distances {
            first: d1 * k,
            second: d2 * k,
        };
        for (first, along_y, along_x) in [(left, d1, d2), (front, d2, d1)] {
            let made = cut(first, two);
            let mut planes = box_planes([4.0, 3.0, 2.0]);
            planes.push(([-1.0 / along_x, -1.0 / along_y, 0.0], -1.0));
            let m = mass(&made.body);
            assert_close(m.volume, convex_volume(&planes) * k * k * k, "volume");
            assert_close(m.volume, (24.0 - d1 * d2 / 2.0 * 2.0) * k * k * k, "volume");
            let area = 52.0 - (d1 + d2) * 2.0 - d1 * d2 + d1.hypot(d2) * 2.0;
            assert_close(m.area, area * k * k, "area");
        }
        // 30 degrees between the first face and the chamfer face.
        let angle = 30.0_f64.to_radians();
        let made = cut(
            left,
            ChamferSize::DistanceAngle {
                distance: d1 * k,
                angle,
            },
        );
        let second = d1 * angle.tan();
        let mut planes = box_planes([4.0, 3.0, 2.0]);
        planes.push(([-1.0 / second, -1.0 / d1, 0.0], -1.0));
        let m = mass(&made.body);
        assert_close(m.volume, convex_volume(&planes) * k * k * k, "volume");
        assert_close(m.volume, (24.0 - d1 * second) * k * k * k, "volume");
    }
}

fn hexagon() -> Vec<[f64; 2]> {
    (0..6)
        .map(|i| {
            let a = f64::from(i) * PI / 3.0;
            [2.0 * a.cos(), 2.0 * a.sin()]
        })
        .collect()
}

/// The faces of the hexagonal prism of [`hexagon`] with height `height`.
fn hexagon_planes(height: f64) -> Vec<([f64; 3], f64)> {
    let mut planes: Vec<([f64; 3], f64)> = (0..6)
        .map(|i| {
            let a = (f64::from(i) + 0.5) * PI / 3.0;
            ([a.cos(), a.sin(), 0.0], 3.0_f64.sqrt())
        })
        .collect();
    planes.push(([0.0, 0.0, -1.0], 0.0));
    planes.push(([0.0, 0.0, 1.0], height));
    planes
}

#[test]
fn edges_of_a_hexagonal_prism_where_the_faces_are_not_square() {
    for k in SCALES {
        for frame in frames(k) {
            let part = prism(&frame, k, &hexagon(), 3.0);
            let whole = convex_volume(&hexagon_planes(3.0));
            // An upright edge, between two walls at 120 degrees.
            let d = 0.5;
            let upright = equal_in(&part, &frame, k, [2.0, 0.0, 1.5], d);
            let made = solve(&part, &[upright], k);
            let removed = d * d / 2.0 * (2.0 * PI / 3.0).sin() * 3.0;
            let m = mass(&made.body);
            assert_close(m.volume, (whole - removed) * k * k * k, "volume");
            // The same with a distance and an angle of 20 degrees.
            let angle = 20.0_f64.to_radians();
            let sized = ChamferEdge {
                size: ChamferSize::DistanceAngle {
                    distance: d * k,
                    angle,
                },
                ..upright
            };
            let second = d * angle.sin() / (angle + 2.0 * PI / 3.0).sin();
            let made = solve(&part, &[sized], k);
            let removed = d * second / 2.0 * (2.0 * PI / 3.0).sin() * 3.0;
            assert_close(
                mass(&made.body).volume,
                (whole - removed) * k * k * k,
                "volume",
            );
            // An edge of the top: at both ends the next wall leans away
            // and the chamfer face runs on until it leaves it.
            let a = 0.5 * PI / 3.0;
            let rim = [3.0_f64.sqrt() * a.cos(), 3.0_f64.sqrt() * a.sin(), 3.0];
            let made = solve(&part, &[equal_in(&part, &frame, k, rim, d)], k);
            let mut planes = hexagon_planes(3.0);
            planes.push(([a.cos(), a.sin(), 1.0], 3.0_f64.sqrt() + 3.0 - d));
            assert_close(
                mass(&made.body).volume,
                convex_volume(&planes) * k * k * k,
                "volume",
            );
            assert_eq!(counts(&made), (14, 21, 9));
        }
    }
}

#[test]
fn three_unequal_chamfers_meet_in_a_flat_corner_through_three_points() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let (da, db, dc) = (0.3, 0.5, 0.4);
        let made = solve(
            &block,
            &[
                equal(&block, k, [0.0, 0.0, 1.0], da),
                equal(&block, k, [2.0, 0.0, 0.0], db),
                equal(&block, k, [0.0, 1.5, 0.0], dc),
            ],
            k,
        );
        // The cut lines cross at (da, 0, db), (0, db, dc)... per face:
        // on y = 0 at (da, 0, db), on z = 0 at (dc, db, 0), on x = 0 at
        // (0, da, dc).
        let (p, q, r) = (
            Vec3::new(da, 0.0, db),
            Vec3::new(dc, db, 0.0),
            Vec3::new(0.0, da, dc),
        );
        let n = (q - p).cross(r - p);
        let n = if n.dot(p) > 0.0 { n } else { -n };
        let mut planes = box_planes([4.0, 3.0, 2.0]);
        planes.push(([-1.0, -1.0, 0.0], -da));
        planes.push(([0.0, -1.0, -1.0], -db));
        planes.push(([-1.0, 0.0, -1.0], -dc));
        planes.push(([-n.x, -n.y, -n.z], -n.dot(p)));
        assert_close(
            mass(&made.body).volume,
            convex_volume(&planes) * k * k * k,
            "volume",
        );
        assert_eq!(made.body.faces.len(), 10);
    }
}

fn rod(frame: &Frame, k: f64, radius: f64, height: f64) -> Body {
    primitives::cylinder(frame, radius * k, height * k).unwrap()
}

/// The volume a triangle in the half-plane (r, z) sweeps in a full turn
/// about the axis (Pappus).
fn ring(corners: [[f64; 2]; 3]) -> f64 {
    let [a, b, c] = corners;
    let area = 0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs();
    2.0 * PI * (a[0] + b[0] + c[0]) / 3.0 * area
}

#[test]
fn the_rim_of_a_rod_becomes_a_cone() {
    for k in SCALES {
        for frame in frames(k) {
            let (r, h, d) = (1.0, 3.0, 0.25);
            let part = rod(&frame, k, r, h);
            let made = solve(&part, &[equal_in(&part, &frame, k, [-r, 0.0, h], d)], k);
            let volume = PI * r * r * h - PI * d * d * (r - d / 3.0);
            let area = PI * r * r
                + 2.0 * PI * r * (h - d)
                + PI * (r - d) * (r - d)
                + PI * (2.0 * r - d) * d * 2.0_f64.sqrt();
            assert_mass(&made, volume, area, k);
            assert_eq!(made.body.faces.len(), 4);
            let cone = made
                .faces
                .iter()
                .position(|f| matches!(f, ChamferFace::Chamfer(_)))
                .unwrap();
            assert_eq!(made.body.faces[cone].surface.kind(), "cone");
            // Two distances: the first along the top.
            let edge = edge_in(&part, &frame, k, [-r, 0.0, h]);
            let top = faces_of(&part, edge)
                .into_iter()
                .find(|f| part.face(*f).unwrap().surface.kind() == "plane")
                .unwrap();
            let (d1, d2) = (0.6, 0.2);
            let sized = ChamferEdge {
                edge,
                first: top,
                size: ChamferSize::Distances {
                    first: d1 * k,
                    second: d2 * k,
                },
            };
            let made = solve(&part, &[sized], k);
            let removed = ring([[r, h], [r - d1, h], [r, h - d2]]);
            assert_close(
                mass(&made.body).volume,
                (PI * r * r * h - removed) * k * k * k,
                "volume",
            );
        }
    }
}

#[test]
fn the_mouth_of_a_hole_loses_material_and_the_foot_of_a_boss_gains_it() {
    for k in SCALES {
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let at = |z: f64| Frame::WORLD.at(Vec3::new(2.0 * k, 1.5 * k, z * k)).unwrap();
        let (r, d) = (0.5, 0.2);
        // A hole right through: its mouth in the top.
        let drill = rod(&at(-1.0), k, r, 4.0);
        let drilled = boolean(&plate, &drill, BooleanOp::Subtract, &tolerance(k))
            .unwrap()
            .body;
        let made = solve(&drilled, &[equal(&drilled, k, [2.0 - r, 1.5, 2.0], d)], k);
        let removed = ring([[r, 2.0], [r + d, 2.0], [r, 2.0 - d]]);
        assert_close(
            mass(&made.body).volume,
            (24.0 - PI * r * r * 2.0 - removed) * k * k * k,
            "volume",
        );
        assert_eq!(made.body.faces.len(), 8);
        assert_eq!(genus(&made.body), Some(1));
        // A blind hole: the rim at its bottom is an inside corner.
        let drill = rod(&at(1.0), k, r, 2.0);
        let blind = boolean(&plate, &drill, BooleanOp::Subtract, &tolerance(k))
            .unwrap()
            .body;
        let made = solve(&blind, &[equal(&blind, k, [2.0 - r, 1.5, 1.0], d)], k);
        let added = ring([[r, 1.0], [r - d, 1.0], [r, 1.0 + d]]);
        assert_close(
            mass(&made.body).volume,
            (24.0 - PI * r * r * 1.0 + added) * k * k * k,
            "volume",
        );
        // A boss on the plate: its foot is an inside corner too.
        let boss = rod(&at(2.0), k, r, 1.0);
        let raised = boolean(&plate, &boss, BooleanOp::Union, &tolerance(k))
            .unwrap()
            .body;
        let made = solve(&raised, &[equal(&raised, k, [2.0 - r, 1.5, 2.0], d)], k);
        let added = ring([[r, 2.0], [r + d, 2.0], [r, 2.0 + d]]);
        assert_close(
            mass(&made.body).volume,
            (24.0 + PI * r * r * 1.0 + added) * k * k * k,
            "volume",
        );
    }
}

#[test]
fn the_base_of_a_cone_and_the_end_of_a_frustum() {
    for k in SCALES {
        let (r, h) = (1.0, 2.0);
        let part = primitives::cone(&Frame::WORLD, r * k, h * k).unwrap();
        let edge = edge_at(&part, k, [-r, 0.0, 0.0]);
        let base = faces_of(&part, edge)
            .into_iter()
            .find(|f| part.face(*f).unwrap().surface.kind() == "plane")
            .unwrap();
        let (d1, d2) = (0.3, 0.4);
        let sized = ChamferEdge {
            edge,
            first: base,
            size: ChamferSize::Distances {
                first: d1 * k,
                second: d2 * k,
            },
        };
        let made = solve(&part, &[sized], k);
        // Up the slope from the rim: towards the apex.
        let slant = r.hypot(h);
        let up = [r - d2 * r / slant, d2 * h / slant];
        let removed = ring([[r, 0.0], [r - d1, 0.0], up]);
        assert_close(
            mass(&made.body).volume,
            (PI * r * r * h / 3.0 - removed) * k * k * k,
            "volume",
        );
    }
}

#[test]
fn a_rim_chamfer_too_large_for_the_end_of_a_rod_says_what_fits() {
    for k in SCALES {
        let part = rod(&Frame::WORLD, k, 1.0, 3.0);
        let edge = edge_at(&part, k, [-1.0, 0.0, 3.0]);
        let faces = faces_of(&part, edge);
        let kind = |f: &FaceId| part.face(*f).unwrap().surface.kind();
        let top = *faces.iter().find(|f| kind(f) == "plane").unwrap();
        let wall = *faces.iter().find(|f| kind(f) == "cylinder").unwrap();
        let sized = |first: f64, second: f64| ChamferEdge {
            edge,
            first: top,
            size: ChamferSize::Distances {
                first: first * k,
                second: second * k,
            },
        };
        for (request, face, fits) in [(sized(1.2, 0.5), top, 1.0), (sized(0.5, 3.5), wall, 3.0)] {
            match chamfer(&part, &[request], &tolerance(k)) {
                Err(KernelError::ChamferTooLarge {
                    edge: e,
                    face: f,
                    largest,
                    ..
                }) => {
                    assert_eq!((e, f), (edge, face));
                    assert_close(largest, fits * k, "largest");
                }
                other => panic!("expected too large, got {other:?}"),
            }
        }
    }
}

#[test]
fn what_is_not_built_is_refused_with_its_kind() {
    let k = 1.0;
    let tol = tolerance(k);
    // The straight edges of a rod cut in half lengthwise lie between a
    // plane and a cylinder.
    let part = rod(&Frame::WORLD, k, 1.0, 3.0);
    let half = boxed(k, [-2.0, 0.0, -1.0], [2.0, 2.0, 4.0]);
    let halved = boolean(&part, &half, BooleanOp::Intersect, &tol)
        .unwrap()
        .body;
    let side = equal(&halved, k, [1.0, 0.0, 1.5], 0.2);
    match chamfer(&halved, &[side], &tol) {
        Err(KernelError::Chamfer { reason, .. }) => {
            assert!(reason.contains("not both planes"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    // Bad requests.
    let block = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let good = equal(&block, k, [0.0, 0.0, 1.0], 0.5);
    assert!(chamfer(&block, &[good, good], &tol).is_err());
    let missing = ChamferEdge {
        edge: EdgeId(99),
        ..good
    };
    assert!(chamfer(&block, &[missing], &tol).is_err());
    let wrong = ChamferEdge {
        first: FaceId(99),
        ..good
    };
    assert!(chamfer(&block, &[wrong], &tol).is_err());
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-9] {
        let sized = ChamferEdge {
            size: ChamferSize::Distances {
                first: bad,
                second: 0.5,
            },
            ..good
        };
        assert!(chamfer(&block, &[sized], &tol).is_err(), "{bad}");
        let angled = ChamferEdge {
            size: ChamferSize::DistanceAngle {
                distance: 0.5,
                angle: bad,
            },
            ..good
        };
        if bad != 1e-9 {
            assert!(chamfer(&block, &[angled], &tol).is_err(), "angle {bad}");
        }
    }
    // An angle that leaves no triangle: 90 degrees against a square
    // corner.
    let angled = ChamferEdge {
        size: ChamferSize::DistanceAngle {
            distance: 0.5,
            angle: PI / 2.0,
        },
        ..good
    };
    assert!(chamfer(&block, &[angled], &tol).is_err());
    assert_eq!(chamfer(&block, &[], &tol).unwrap().body, block);
}

use common::{arc, line};
use craft3d_kernel::chamfer::tangent_chain;

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
fn the_top_rim_of_a_slot_is_a_chain_of_lines_and_arcs() {
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
            assert!(chain.iter().all(|(_, first)| *first == top));
            let (d1, d2) = (0.3, 0.2);
            let edges: Vec<ChamferEdge> = chain
                .iter()
                .map(|(edge, first)| ChamferEdge {
                    edge: *edge,
                    first: *first,
                    size: ChamferSize::Distances {
                        first: d1 * k,
                        second: d2 * k,
                    },
                })
                .collect();
            let made = solve(&part, &edges, k);
            // Two straight prisms and one ring in two halves.
            let section = d1 * d2 / 2.0;
            let removed =
                2.0 * 3.0 * section + ring([[1.0, 2.0], [1.0 - d1, 2.0], [1.0, 2.0 - d2]]);
            let whole = (6.0 + PI) * 2.0;
            let m = mass(&made.body);
            assert_close(m.volume, (whole - removed) * k * k * k, "volume");
            assert_eq!(made.body.faces.len(), 6 + 4);
            // One arc alone ends in the middle of the chain: refused.
            let alone: Vec<ChamferEdge> = edges
                .iter()
                .copied()
                .filter(|e| part.edge(e.edge).unwrap().curve.kind() == "circle")
                .take(1)
                .collect();
            match chamfer(&part, &alone, &tolerance(k)) {
                Err(KernelError::Chamfer { edge, reason, .. }) => {
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
        let d = 0.2;
        let made = solve(&halved, &[equal(&halved, k, [0.0, 1.0, 3.0], d)], k);
        let removed = ring([[1.0, 3.0], [1.0 - d, 3.0], [1.0, 3.0 - d]]) / 2.0;
        assert_close(
            mass(&made.body).volume,
            (PI * 3.0 / 2.0 - removed) * k * k * k,
            "volume",
        );
        assert_eq!(made.body.faces.len(), 5);
    }
}

/// The outward normal of a flat face and its offset: `n . p = c` on it.
fn plane_of(body: &Body, face: FaceId) -> (Vec3, f64) {
    let data = body.face(face).unwrap();
    let n = data.surface.normal(0.0, 0.0).unwrap();
    let n = if data.reversed { -n } else { n };
    (n, n.dot(data.surface.eval(0.0, 0.0)))
}

/// Blocks of seeded random sizes with seeded random sets of their edges
/// chamfered, each by a distance of its own: every one is built (nothing
/// refused, by the boolean or by the chamfer), passes the checker, and has
/// the volume of the convex solid its planes describe: the block, one
/// plane per chamfer, and the plane through three points at every corner
/// where three chamfers meet.
#[test]
fn seeded_random_sets_of_edges_of_random_blocks() {
    use boolean_common::Rng;
    let mut rng = Rng(0x00C4_A3FE);
    let mut corners_made = 0;
    for round in 0..120 {
        let k = SCALES[round % 3];
        let size = [
            1.0 + 4.0 * rng.unit(),
            1.0 + 4.0 * rng.unit(),
            1.0 + 4.0 * rng.unit(),
        ];
        let block = boxed(k, [0.0, 0.0, 0.0], size);
        let least = size.iter().fold(f64::INFINITY, |a, b| a.min(*b));
        let keep = 0.2 + 0.8 * rng.unit();
        let mut requests = Vec::new();
        let mut planes = box_planes(size);
        // Per edge that is chamfered: its two faces and its distance.
        let mut chosen: Vec<(EdgeId, [FaceId; 2], f64)> = Vec::new();
        for index in 0..12 {
            if rng.unit() > keep {
                continue;
            }
            let edge = EdgeId(index);
            let faces = faces_of(&block, edge);
            let d = least * (0.03 + 0.2 * rng.unit());
            requests.push(ChamferEdge {
                edge,
                first: faces[(rng.below(2)) as usize],
                size: ChamferSize::Distances {
                    first: d * k,
                    second: d * k,
                },
            });
            let (n1, c1) = plane_of(&block, faces[0]);
            let (n2, c2) = plane_of(&block, faces[1]);
            let n = n1 + n2;
            planes.push(([n.x, n.y, n.z], (c1 + c2) / k - d));
            chosen.push((edge, [faces[0], faces[1]], d));
        }
        // The corners: a vertex all three of whose edges are chamfered.
        for (index, vertex) in block.vertices.iter().enumerate() {
            let at: Vec<&(EdgeId, [FaceId; 2], f64)> = chosen
                .iter()
                .filter(|(edge, _, _)| {
                    let e = block.edge(*edge).unwrap();
                    e.start.0 as usize == index || e.end.0 as usize == index
                })
                .collect();
            if at.len() != 3 {
                continue;
            }
            // On each of the three faces the two cut lines cross: from
            // the vertex, back along each of the other two faces' normals
            // by the distance of the chamfer shared with that face.
            let mut faces: Vec<FaceId> = at.iter().flat_map(|(_, f, _)| *f).collect();
            faces.sort();
            faces.dedup();
            assert_eq!(faces.len(), 3);
            let distance = |a: FaceId, b: FaceId| {
                at.iter()
                    .find(|(_, f, _)| f.contains(&a) && f.contains(&b))
                    .unwrap()
                    .2
            };
            let corner = vertex.point * (1.0 / k);
            let on = |face: FaceId| {
                let mut p = corner;
                for other in faces.iter().filter(|f| **f != face) {
                    p = p - plane_of(&block, *other).0 * distance(face, *other);
                }
                p
            };
            // No corner face where one chamfer is larger than the other
            // two together: it takes the corner whole.
            let (ab, bc, ca) = (
                distance(faces[0], faces[1]),
                distance(faces[1], faces[2]),
                distance(faces[2], faces[0]),
            );
            if ab >= bc + ca || bc >= ca + ab || ca >= ab + bc {
                continue;
            }
            let (p, q, r) = (on(faces[0]), on(faces[1]), on(faces[2]));
            let n = (q - p).cross(r - p);
            let n = if n.dot(corner - p) > 0.0 { n } else { -n };
            planes.push(([n.x, n.y, n.z], n.dot(p)));
            corners_made += 1;
        }
        let made = match chamfer(&block, &requests, &tolerance(k)) {
            Ok(made) => made,
            Err(e) => panic!("round {round} at scale {k}, {} edges: {e}", requests.len()),
        };
        assert_eq!(check(&made.body, &tolerance(k)), [], "round {round}");
        assert_close(
            mass(&made.body).volume,
            convex_volume(&planes) * k * k * k,
            &format!("round {round}: volume"),
        );
    }
    assert!(corners_made > 50, "{corners_made} corners");
}
