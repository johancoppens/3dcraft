//! Shells the kernel set's group H found refused (2026-11-14), each against
//! a volume worked out by hand from the design (the sharp hollow: every face
//! that keeps a wall moved in by the thickness and continued until it meets
//! the others), at three scales, upright and turned off every axis: valid,
//! meshed watertight with the volume it encloses, clear of itself, the
//! boolean identity where one holds, and refused in words where it should
//! be.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;
mod rational_common;

use boolean_common::{SCALES, assert_close, boxed, mass, tolerance};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_kernel::shell::{Shelled, Skin, shell};
use craft3d_math::{Frame, Portable, Transform, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, EdgeId, FaceId, VertexId, check};
use std::f64::consts::PI;

/// The face on a surface of `kind` nearest `point`, `point` in the body's
/// own frame at scale 1 (`place` takes it where the body is).
fn face_at(body: &Body, place: &dyn Fn([f64; 3]) -> Vec3, kind: &str, point: [f64; 3]) -> FaceId {
    let target = place(point);
    let (index, _) = body
        .faces
        .iter()
        .enumerate()
        .filter(|(_, face)| face.surface.kind() == kind)
        .map(|(i, face)| {
            let (u, v) = face.surface.project(target).unwrap();
            (i, face.surface.eval(u, v).distance(target))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    FaceId(index as u32)
}

fn edge_at(body: &Body, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = Vec3::new(middle[0] * k, middle[1] * k, middle[2] * k);
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(target),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(index as u32)
}

/// What a document asks of a body before it hands it on: valid, meshed
/// watertight with the volume it encloses, and clear of itself on the mesh
/// of the self-check.
fn sound(body: &Body, k: f64, what: &str) {
    let tol = tolerance(k);
    assert_eq!(check(body, &tol), [], "{what}");
    let exact = mass(body);
    let params = TessParams {
        chordal_tolerance: 0.01 * k,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
    let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert!(welded.is_closed(), "{what}: the mesh is not watertight");
    assert!(
        (welded.volume() - exact.volume).abs() <= exact.area * 0.01 * k,
        "{what}: mesh volume {} against {}",
        welded.volume(),
        exact.volume
    );
    let found = body_self_intersections(body, &SelfCheckParams::new(tol.linear))
        .unwrap_or_else(|e| panic!("{what}: the self-check cannot mesh it: {e}"));
    assert!(
        found.crossings.is_empty() && found.certain_crossings == 0,
        "{what}: {} crossings",
        found.crossings.len()
    );
}

/// The turn every case is also built in: off every axis.
fn turn() -> Transform {
    Transform::rotation(Vec3::new(0.4, -1.0, 0.7), 1.1).unwrap()
}

/// `got` against `want` (at scale 1, for scale `k`): exact faces exactly
/// placed to 1e-9 upright; turned, or with edges found again as fitted
/// curves, within the tolerance over the area.
#[track_caller]
fn assert_volume(got: &Body, want: f64, k: f64, what: &str) {
    let m = mass(got);
    let want = want * k * k * k;
    let slack = (1e-9 * want).max(tolerance(k).linear * m.area);
    assert!(
        (m.volume - want).abs() <= slack,
        "{what}: volume {} against {want} (off {:e}, slack {slack:e})",
        m.volume,
        m.volume - want
    );
}

// ------------------------------------------------------------------ H-02

/// The volume a box loses when the four edges round its base `lx` × `ly`
/// are rounded `r`, the rounds meeting in mitres at the upright corners:
/// per edge the spandrel's area `(1 − π/4) r²` swept along it, less at both
/// mitres the spandrel's moment about the corner edge (its centroid lies
/// `r (10 − 3π)/(12 − 3π)` from it).
fn mitred_rounds(lx: f64, ly: f64, r: f64) -> f64 {
    let area = (1.0 - PI / 4.0) * r * r;
    let centroid = r * (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI);
    area * (2.0 * lx + 2.0 * ly) - 8.0 * area * centroid
}

/// A box `w` × `d` × `h` whose four bottom edges are rounded `r` (mitred
/// where they meet at the upright corners).
fn bottom_rounded(k: f64, (w, d, h, r): (f64, f64, f64, f64)) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [w, d, h]);
    let edges: Vec<FilletEdge> = [
        [w / 2.0, 0.0, 0.0],
        [w, d / 2.0, 0.0],
        [w / 2.0, d, 0.0],
        [0.0, d / 2.0, 0.0],
    ]
    .into_iter()
    .map(|middle| FilletEdge {
        edge: edge_at(&block, k, middle),
        radius: r * k,
    })
    .collect();
    fillet(&block, &edges, &tolerance(k)).unwrap().body
}

/// The kernel set's H-02: a box 40 × 30 × 10, its bottom edges rounded 2.4
/// (mitred at the upright corners), shelled open at its bottom. The bottom
/// has the rounds along it, so it keeps a wall like every face and the
/// hollow is opened through it by the prism on the hollow's floor (the
/// lip). Refused before from 1.87 on: the mitre where two rounds meet, an
/// ellipse that shrinks from radius 2.4 to 0.4 while its ends move 2.8, was
/// guessed from its old points moved as its ends moved, which lie far off
/// the new curve, and taken as turned round. The volume: the box less the
/// mitred rounds, less the closed hollow (the box `2t` smaller, its bottom
/// edges rounded `r − t` about the same axes), less the prism's slab inside
/// the body, `(w − 2r)(d − 2r) t`.
#[test]
fn a_box_rounded_along_its_bottom_shells_open_beside_its_rounds() {
    let size @ (w, d, h, r) = (40.0, 30.0, 10.0, 2.4);
    let solid = w * d * h - mitred_rounds(w, d, r);
    for k in SCALES {
        let tol = tolerance(k);
        let upright = bottom_rounded(k, size);
        assert_volume(&upright, solid, k, "the rounded box");
        let turned = upright.transformed(&turn()).unwrap();
        for (body, what, turning) in [
            (&upright, "upright", None),
            (&turned, "turned", Some(turn())),
        ] {
            let place = |p: [f64; 3]| {
                let at = Vec3::new(p[0] * k, p[1] * k, p[2] * k);
                match &turning {
                    Some(t) => t.point(at),
                    None => at,
                }
            };
            let bottom = face_at(body, &place, "plane", [w / 2.0, d / 2.0, 0.0]);
            for t in [1.0, 2.0, 2.3] {
                let what = format!("{what} at {t}, scale {k}");
                let made =
                    shell(body, t * k, &[bottom], &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
                sound(&made.body, k, &what);
                assert_eq!(made.body.shells.len(), 1, "{what}: one skin, open");
                let hollow = (w - 2.0 * t) * (d - 2.0 * t) * (h - 2.0 * t)
                    - mitred_rounds(w - 2.0 * t, d - 2.0 * t, r - t);
                let slab = (w - 2.0 * r) * (d - 2.0 * r) * t;
                assert_volume(&made.body, solid - hollow - slab, k, &what);
                // The boolean identity: the shell is the solid less its
                // closed hollow (every face moved in) less the slab the
                // prism opens below the hollow's floor.
                if turning.is_none() {
                    let offsets: Vec<FaceOffset> = (0..body.faces.len())
                        .map(|i| FaceOffset {
                            face: FaceId(i as u32),
                            distance: -t * k,
                        })
                        .collect();
                    let inner = offset_faces(body, &offsets, &tol)
                        .unwrap_or_else(|e| panic!("{what}: the closed hollow: {e}"))
                        .body;
                    let less = boolean(body, &inner, BooleanOp::Subtract, &tol)
                        .unwrap()
                        .body;
                    let slab_box = boxed(k, [r, r, 0.0], [w - r, d - r, t]);
                    let identity = boolean(&less, &slab_box, BooleanOp::Subtract, &tol)
                        .unwrap()
                        .body;
                    assert_close(mass(&identity).volume, mass(&made.body).volume, &what);
                }
            }
        }
        // Where the walls meet, nothing is left: too far, with what fits.
        let bottom = face_at(
            &upright,
            &|p| Vec3::new(p[0] * k, p[1] * k, p[2] * k),
            "plane",
            [w / 2.0, d / 2.0, 0.0],
        );
        match shell(&upright, 5.0 * k, &[bottom], &tol) {
            Err(KernelError::FaceOpTooFar { largest, .. }) => {
                assert!(largest > 4.9 * k && largest < 5.0 * k, "{largest}");
            }
            other => panic!("a shell of 5 at scale {k}: {other:?}"),
        }
        for bad in [f64::NAN, -1.0, 0.0] {
            assert!(
                matches!(
                    shell(&upright, bad, &[bottom], &tol),
                    Err(KernelError::InvalidSize { .. })
                ),
                "thickness {bad}"
            );
        }
    }
}

// ------------------------------------------------------- vertices split

/// The prism of the polygon `points` (in the frame's x and y) from the
/// frame's plane `length` along its z, at scale `k` (the frame's origin
/// too).
fn prism(k: f64, frame: &Frame, points: &[[f64; 2]], length: f64) -> Body {
    let tol = tolerance(k);
    let n = points.len();
    let curves: Vec<SketchCurve> = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line {
                    start: [a[0] * k, a[1] * k],
                    end: [b[0] * k, b[1] * k],
                },
            }
        })
        .collect();
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let frame = frame.at(frame.origin() * k).unwrap();
    extrude(&profiles, &frame, 0.0, length * k, &tol)
        .unwrap()
        .body
}

/// The frame whose x and y are the world's x and z, its z along y: a
/// polygon in (x, z) extruded along y from `y`.
fn across_y(y: f64) -> Frame {
    Frame::new(Vec3::new(0.0, y, 0.0), Vec3::Y, Vec3::X).unwrap()
}

/// The frame whose x and y are the world's y and z, its z along x.
fn across_x(x: f64) -> Frame {
    Frame::new(Vec3::new(x, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap()
}

/// The depth of the drain below the rim: 30 tan 1°, as the kernel set's
/// scripts give it.
const DRAIN: f64 = 0.5236519478465276;

/// A pyramid over the rectangle `x0..x1` × `y0..y1` at height `z`, its
/// apex `DRAIN` below the middle: two triangular prisms crossed, as the
/// kernel set builds it.
fn drain(k: f64, (x0, x1, y0, y1, z): (f64, f64, f64, f64, f64)) -> Body {
    let tol = tolerance(k);
    // Across y, the frame's y is the world's −z (as the scripts' plane).
    let along_y = Frame::new(Vec3::new(0.0, y0, z), Vec3::Y, Vec3::X).unwrap();
    let a = prism(
        k,
        &along_y,
        &[[x0, 0.0], [x1, 0.0], [(x0 + x1) / 2.0, DRAIN]],
        y1 - y0,
    );
    let b = prism(
        k,
        &across_x(x0).at(Vec3::new(x0, 0.0, z)).unwrap(),
        &[[y0, 0.0], [y1, 0.0], [(y0 + y1) / 2.0, -DRAIN]],
        x1 - x0,
    );
    boolean(&a, &b, BooleanOp::Intersect, &tol).unwrap().body
}

/// A floor plane `z = a0 + ax x + ay y`.
type Plane = (f64, f64, f64);

/// The part of the convex polygon where `a x + b y + c ≥ 0`.
fn clip(poly: &[[f64; 2]], (a, b, c): (f64, f64, f64)) -> Vec<[f64; 2]> {
    let n = poly.len();
    let mut out = Vec::new();
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        let (fp, fq) = (a * p[0] + b * p[1] + c, a * q[0] + b * q[1] + c);
        if fp >= 0.0 {
            out.push(p);
        }
        if (fp >= 0.0) != (fq >= 0.0) {
            let s = fp / (fp - fq);
            out.push([p[0] + s * (q[0] - p[0]), p[1] + s * (q[1] - p[1])]);
        }
    }
    out
}

/// `∫ (a0 + ax x + ay y)` over a polygon: its area times the plane at its
/// centroid.
fn over_polygon(poly: &[[f64; 2]], (a0, ax, ay): Plane) -> f64 {
    let n = poly.len();
    let (mut twice, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        let cross = p[0] * q[1] - q[0] * p[1];
        twice += cross;
        sx += (p[0] + q[0]) * cross;
        sy += (p[1] + q[1]) * cross;
    }
    if twice == 0.0 {
        return 0.0;
    }
    let (cx, cy) = (sx / (3.0 * twice), sy / (3.0 * twice));
    0.5 * twice.abs() * (a0 + ax * cx + ay * cy)
}

/// `∫ max_i plane_i` over the rectangle `x0..x1` × `y0..y1`, exactly: per
/// plane, the convex part of the rectangle where it is the highest.
fn integral_of_max(planes: &[Plane], (x0, y0, x1, y1): (f64, f64, f64, f64)) -> f64 {
    integral_of_max_over(planes, &[[x0, y0], [x1, y0], [x1, y1], [x0, y1]])
}

/// [`integral_of_max`] over a convex polygon (counter-clockwise).
fn integral_of_max_over(planes: &[Plane], domain: &[[f64; 2]]) -> f64 {
    let mut total = 0.0;
    for (i, p) in planes.iter().enumerate() {
        let mut poly = domain.to_vec();
        for (j, q) in planes.iter().enumerate() {
            if i != j && poly.len() >= 3 {
                poly = clip(&poly, (p.1 - q.1, p.2 - q.2, p.0 - q.0));
            }
        }
        if poly.len() >= 3 {
            total += over_polygon(&poly, *p);
        }
    }
    total
}

/// The four facets of a floor that falls `DRAIN` from the rim of the
/// rectangle `x0..x1` × `y0..y1` at height `z` to its middle, each moved by
/// `d` along its own normal (up for positive): `z + DRAIN (|x − xm| / hx −
/// 1)` and the same across y, lifted by `d / cos` of its slope.
fn facets(d: f64, (x0, x1, y0, y1, z): (f64, f64, f64, f64, f64)) -> [Plane; 4] {
    let (xm, ym, hx, hy) = (
        (x0 + x1) / 2.0,
        (y0 + y1) / 2.0,
        (x1 - x0) / 2.0,
        (y1 - y0) / 2.0,
    );
    let (sx, sy) = (DRAIN / hx, DRAIN / hy);
    let (lx, ly) = (d * (1.0 + sx * sx).sqrt(), d * (1.0 + sy * sy).sqrt());
    let base = z - DRAIN;
    [
        (base - sx * xm + lx, sx, 0.0),
        (base + sx * xm + lx, -sx, 0.0),
        (base - sy * ym + ly, 0.0, sy),
        (base + sy * ym + ly, 0.0, -sy),
    ]
}

/// The region over the rectangle `x0..x1` × `y0..y1` above the highest of
/// `planes` (two of them sloping across x, two across y) and below `top`:
/// two prisms crossed, each with the two planes of its direction as its
/// floor. Built from the planes alone, not from any offset.
fn above(k: f64, planes: &[Plane; 4], (x0, x1, y0, y1): (f64, f64, f64, f64), top: f64) -> Body {
    let tol = tolerance(k);
    let z = |p: &Plane, x: f64, y: f64| p.0 + p.1 * x + p.2 * y;
    // Across y: the floor in (x, z), the two planes that slope along x.
    let (east, west) = (planes[0], planes[1]);
    let xm = (west.0 - east.0) / (east.1 - west.1);
    let a = prism(
        k,
        &across_y(y0),
        &[
            [x0, -z(&west, x0, 0.0)],
            [x0, -top],
            [x1, -top],
            [x1, -z(&east, x1, 0.0)],
            [xm, -z(&east, xm, 0.0)],
        ],
        y1 - y0,
    );
    let (north, south) = (planes[2], planes[3]);
    let ym = (south.0 - north.0) / (north.2 - south.2);
    let b = prism(
        k,
        &across_x(x0),
        &[
            [y0, z(&south, 0.0, y0)],
            [ym, z(&north, 0.0, ym)],
            [y1, z(&north, 0.0, y1)],
            [y1, top],
            [y0, top],
        ],
        x1 - x0,
    );
    boolean(&a, &b, BooleanOp::Intersect, &tol).unwrap().body
}

/// Per vertex of `body`, the faces round it.
fn faces_round(body: &Body) -> Vec<Vec<FaceId>> {
    let uses = craft3d_topo::edge_faces(body);
    let mut round: Vec<Vec<FaceId>> = vec![Vec::new(); body.vertices.len()];
    for (index, edge) in body.edges.iter().enumerate() {
        for v in [edge.start, edge.end] {
            for f in &uses[index] {
                if !round[v.0 as usize].contains(f) {
                    round[v.0 as usize].push(*f);
                }
            }
        }
    }
    round
}

/// What is held of every shell whose vertices of four faces were split:
/// every vertex of the hollow is a vertex of three faces, and none of
/// those vertices is named as one of the solid's vertices of four faces
/// (each piece is named by its faces).
fn split_cleanly(solid: &Body, made: &Shelled, what: &str) {
    let before = faces_round(solid);
    let four: Vec<VertexId> = (0..before.len())
        .filter(|v| before[*v].len() >= 4)
        .map(|v| VertexId(v as u32))
        .collect();
    assert!(!four.is_empty(), "{what}: no vertex of four faces to split");
    let after = faces_round(&made.body);
    for (v, faces) in after.iter().enumerate() {
        if faces
            .iter()
            .all(|f| made.faces[f.0 as usize].0 == Skin::Inner)
        {
            assert!(
                faces.len() <= 3,
                "{what}: vertex {v} of the hollow has {faces:?}"
            );
        }
        if let Some((Skin::Inner, of)) = made.vertices[v] {
            assert!(!four.contains(&of), "{what}: vertex {v} named after {of:?}");
        }
    }
}

/// Each case upright at every scale and turned off every axis at scale 1.
fn placements() -> Vec<(f64, Option<Transform>)> {
    let mut all: Vec<(f64, Option<Transform>)> = SCALES.iter().map(|k| (*k, None)).collect();
    all.push((1.0, Some(turn())));
    all
}

fn placed(body: Body, turning: &Option<Transform>) -> Body {
    match turning {
        Some(t) => body.transformed(t).unwrap(),
        None => body,
    }
}

fn place(k: f64, turning: &Option<Transform>) -> impl Fn([f64; 3]) -> Vec3 + '_ {
    move |p: [f64; 3]| {
        let at = Vec3::new(p[0] * k, p[1] * k, p[2] * k);
        match turning {
            Some(t) => t.point(at),
            None => at,
        }
    }
}

// ------------------------------------------------------------------ H-07

/// The kernel set's H-07: a trough 100 × 60 × 40 whose floor falls 1° from
/// its edges to a drain in the middle, a tab 20 × 20 × 20 on one end, its
/// top flush with the trough's.
fn trough(k: f64) -> Body {
    let tol = tolerance(k);
    let box_ = boxed(k, [0.0, 0.0, 0.0], [100.0, 60.0, 40.0]);
    let floor = drain(k, (0.0, 100.0, 0.0, 60.0, 0.0));
    let with_drain = boolean(&box_, &floor, BooleanOp::Union, &tol).unwrap().body;
    let tab = boxed(k, [100.0, 20.0, 20.0], [120.0, 40.0, 40.0]);
    boolean(&with_drain, &tab, BooleanOp::Union, &tol)
        .unwrap()
        .body
}

/// The kernel set's H-07, shelled open at its top. Refused before at any
/// thickness: the four bottom corners (two walls and two facets of the
/// floor) and the drain (four facets) are vertices of four faces, convex
/// all round, and moved in they no longer meet in one point. Each becomes
/// two vertices of three faces and an edge between the two faces the moved
/// ones meet in behind the others (the hollow there is behind every moved
/// face). The volume: the trough with its pyramid and tab, less the cavity
/// `∫ (40 − max of the four facets moved up) dA` over the walls moved in
/// (exactly, per facet over the part where it is the highest), less the
/// tab's hollow `20 (20 − 2t)(20 − t)`. The identity: the solid less the
/// hollow built from the moved planes alone (two prisms crossed) and the
/// tab's hollow, by booleans. Refused with what fits where the tab's walls
/// meet (t = 10).
#[test]
fn a_trough_whose_floor_falls_to_a_drain_shells_round_its_split_corners() {
    let rim = (0.0, 100.0, 0.0, 60.0, 0.0);
    let solid = 100.0 * 60.0 * 40.0 + 100.0 * 60.0 * DRAIN / 3.0 + 20.0_f64.ppowi(3);
    for (k, turning) in placements() {
        let tol = tolerance(k);
        let upright = trough(k);
        let body = placed(upright.clone(), &turning);
        assert_volume(&body, solid, k, "the trough");
        let top = face_at(&body, &place(k, &turning), "plane", [50.0, 30.0, 40.0]);
        for t in [1.0, 2.0, 4.0] {
            let what = format!("t {t}, scale {k}, turned {}", turning.is_some());
            let made = shell(&body, t * k, &[top], &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            assert_eq!(made.body.shells.len(), 1, "{what}");
            let walls = (t, t, 100.0 - t, 60.0 - t);
            let area = (100.0 - 2.0 * t) * (60.0 - 2.0 * t);
            let cavity = 40.0 * area - integral_of_max(&facets(t, rim), walls);
            let tab = 20.0 * (20.0 - 2.0 * t) * (20.0 - t);
            assert_volume(&made.body, solid - cavity - tab, k, &what);
            split_cleanly(&body, &made, &what);
            if turning.is_none() {
                let hollow = above(k, &facets(t, rim), (t, 100.0 - t, t, 60.0 - t), 42.0);
                let tab = boxed(
                    k,
                    [100.0 - t, 20.0 + t, 20.0 + t],
                    [120.0 - t, 40.0 - t, 42.0],
                );
                let hollow = boolean(&hollow, &tab, BooleanOp::Union, &tol).unwrap().body;
                let identity = boolean(&body, &hollow, BooleanOp::Subtract, &tol)
                    .unwrap()
                    .body;
                assert_close(mass(&identity).volume, mass(&made.body).volume, &what);
            }
        }
        // The tab is 20 wide: from 10 on its walls meet.
        match shell(&body, 12.0 * k, &[top], &tol) {
            Err(KernelError::FaceOpTooFar { largest, faces, .. }) => {
                assert!(largest > 9.9 * k && largest < 10.0 * k, "{largest}");
                assert!(!faces.is_empty());
            }
            other => panic!("a shell of 12 at scale {k}: {other:?}"),
        }
    }
    // Only the shell's hollow splits a vertex: an offset of the same faces
    // is refused as before, naming the four faces.
    let k = 1.0;
    let body = trough(k);
    let at = place(k, &None);
    let floor: Vec<FaceOffset> = [[10.0, 30.0], [90.0, 30.0], [50.0, 5.0], [50.0, 55.0]]
        .into_iter()
        .map(|[x, y]| FaceOffset {
            face: face_at(&body, &at, "plane", [x, y, -0.1]),
            distance: 1.0,
        })
        .collect();
    match offset_faces(&body, &floor, &tolerance(k)) {
        Err(KernelError::FaceOpTooFar { faces, .. } | KernelError::FaceOp { faces, .. }) => {
            assert!(faces.len() >= 4, "{faces:?}");
        }
        other => panic!("the floor offset: {other:?}"),
    }
}

// ------------------------------------------------------------------ H-06

/// The kernel set's H-06: a slab 140 × 90 × 50 with a pocket 100 × 60, 30
/// deep from its top, whose floor falls 1° to a drain in its middle.
fn pocketed(k: f64) -> Body {
    let tol = tolerance(k);
    let slab = boxed(k, [0.0, 0.0, 0.0], [140.0, 90.0, 50.0]);
    let pocket = boxed(k, [20.0, 15.0, 20.0], [120.0, 75.0, 60.0]);
    let floor = drain(k, (20.0, 120.0, 15.0, 75.0, 20.0));
    let tool = boolean(&pocket, &floor, BooleanOp::Union, &tol)
        .unwrap()
        .body;
    boolean(&slab, &tool, BooleanOp::Subtract, &tol)
        .unwrap()
        .body
}

/// The kernel set's H-06, shelled open at its top. Refused before at any
/// thickness: the pocket's four bottom corners and its drain are vertices
/// of four faces, concave all round (the pocket is empty and convex); each
/// becomes two vertices and an edge between the two faces the moved ones
/// meet in in front of the others. With the top open, the pocket's lining
/// (its walls and floor, `t` thick, hanging from where the top was) no
/// longer touches the slab's walls: one body of two lumps, as the shell's
/// definition gives it (whether to refuse that is the developer's to
/// decide; nothing refused it before either, for a boss on an open face).
/// The volume: the slab less the pocket, less the cavity, the box `t`
/// inside the slab less the pocket grown by `t` (its walls moved out, its
/// floor's facets moved down and continued until they meet), exactly. Each
/// lump on its own: the lining is the grown pocket less the pocket below
/// the top. The identity: the solid less the cavity built from the moved
/// planes alone.
#[test]
fn a_pocket_whose_floor_falls_to_a_drain_shells_into_two_lumps() {
    let pocket = (20.0, 120.0, 15.0, 75.0, 20.0);
    let hollowed = 100.0 * 60.0 * 30.0 + 100.0 * 60.0 * DRAIN / 3.0;
    let solid = 140.0 * 90.0 * 50.0 - hollowed;
    for (k, turning) in placements() {
        let tol = tolerance(k);
        let body = placed(pocketed(k), &turning);
        assert_volume(&body, solid, k, "the pocketed slab");
        let top = face_at(&body, &place(k, &turning), "plane", [5.0, 5.0, 50.0]);
        for t in [1.0, 3.0, 6.0] {
            let what = format!("t {t}, scale {k}, turned {}", turning.is_some());
            let made = shell(&body, t * k, &[top], &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            let grown = (20.0 - t, 120.0 + t, 15.0 - t, 75.0 + t);
            let floor = facets(-t, pocket);
            let rect = (grown.0, grown.2, grown.1, grown.3);
            let area = (grown.1 - grown.0) * (grown.3 - grown.2);
            let grown_pocket = 50.0 * area - integral_of_max(&floor, rect);
            let cavity = (140.0 - 2.0 * t) * (90.0 - 2.0 * t) * (50.0 - t) - grown_pocket;
            assert_volume(&made.body, solid - cavity, k, &what);
            split_cleanly(&body, &made, &what);
            // Two lumps: the slab's walls and floor, and the lining.
            assert_eq!(made.body.shells.len(), 2, "{what}");
            let mut lumps: Vec<f64> = (0..2)
                .map(|i| mass(&boolean_common::shell_alone(&made.body, i)).volume)
                .collect();
            lumps.sort_by(f64::total_cmp);
            let lining = (grown_pocket - hollowed) * k * k * k;
            let slack = tol.linear * mass(&made.body).area;
            assert!(
                (lumps[0] - lining).abs() <= slack.max(1e-9 * lining),
                "{what}: the lining {} against {lining}",
                lumps[0]
            );
            if turning.is_none() {
                let grown_tool = above(k, &floor, grown, 60.0);
                let inside = boxed(k, [t, t, t], [140.0 - t, 90.0 - t, 52.0]);
                let hollow = boolean(&inside, &grown_tool, BooleanOp::Subtract, &tol)
                    .unwrap()
                    .body;
                let identity = boolean(&body, &hollow, BooleanOp::Subtract, &tol)
                    .unwrap()
                    .body;
                assert_close(mass(&identity).volume, mass(&made.body).volume, &what);
            }
        }
    }
}

// ------------------------------------------------------------------ H-01

/// The kernel set's H-01: a box 47.5 × 45 × 30 and a cylinder of radius
/// 7.5, 30 long, joined on its side, the cylinder's axis in the plane of
/// the box's front face (y = 0) at height 10; the cylinder's seam (where
/// its rims start) on the side `seam` points to from its axis.
fn box_and_cylinder(k: f64, seam: Vec3) -> Body {
    let tol = tolerance(k);
    let box_ = boxed(k, [0.0, 0.0, 0.0], [47.5, 45.0, 30.0]);
    let frame = Frame::new(Vec3::new(47.5 * k, 0.0, 10.0 * k), Vec3::X, seam).unwrap();
    let rod = craft3d_kernel::primitives::cylinder(&frame, 7.5 * k, 30.0 * k).unwrap();
    boolean(&box_, &rod, BooleanOp::Union, &tol).unwrap().body
}

/// The kernel set's H-01, shelled open at the box's bottom. Refused before
/// at any thickness: where the cylinder's lowest and highest lines meet the
/// box's front edge, the box's front and right faces meet the cylinder's
/// wall and the half of its end disc that stands out in front of the box,
/// that disc in the plane of the right face, facing the other way. Moved in,
/// the four faces no longer meet in a point; of the two trees, the one that
/// joins the right face's and the disc's corners needs those two, now
/// parallel planes apart, to meet: the vertex becomes two corners joined by
/// a line where the front face's offset meets the cylinder's. The volume:
/// the box and the cylinder, less the box's hollow `(47.5 − 2t)(45 − 2t)(30
/// − t)` and the cylinder's of radius `r = 7.5 − t` from the box's moved
/// side, less over `2t` of its length the part in front of the front face's
/// offset, `r² (π/2 + asin(t/r)) + t √(r² − t²)` (half the disc and the
/// strip up to `t`).
#[test]
fn a_box_and_a_cylinder_on_its_front_plane_shell_round_their_split_corners() {
    let solid = 47.5 * 45.0 * 30.0 + PI * 7.5 * 7.5 * 30.0;
    // The seam where the command puts it (+y), and on the cylinder's lowest
    // line, through the vertices that split.
    for (k, turning, seam) in placements()
        .into_iter()
        .flat_map(|(k, turning)| [(k, turning, Vec3::Y), (k, turning, -Vec3::Z)])
    {
        let tol = tolerance(k);
        let body = placed(box_and_cylinder(k, seam), &turning);
        assert_volume(&body, solid, k, "the box and the cylinder");
        let bottom = face_at(&body, &place(k, &turning), "plane", [20.0, 20.0, 0.0]);
        for t in [0.5, 1.0, 2.0] {
            let what = format!(
                "t {t}, scale {k}, turned {}, seam {seam:?}",
                turning.is_some()
            );
            let made =
                shell(&body, t * k, &[bottom], &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            assert_eq!(made.body.shells.len(), 1, "{what}");
            let r: f64 = 7.5 - t;
            let front = r * r * (PI / 2.0 + (t / r).pasin()) + t * (r * r - t * t).sqrt();
            let cavity = (47.5 - 2.0 * t) * (45.0 - 2.0 * t) * (30.0 - t) + PI * r * r * 30.0
                - 2.0 * t * front;
            assert_volume(&made.body, solid - cavity, k, &what);
            split_cleanly(&body, &made, &what);
            if turning.is_none() {
                let inner_box = boxed(k, [t, t, -t], [47.5 - t, 45.0 - t, 30.0 - t]);
                let frame =
                    Frame::new(Vec3::new((47.5 - t) * k, 0.0, 10.0 * k), Vec3::X, Vec3::Y).unwrap();
                let inner_rod =
                    craft3d_kernel::primitives::cylinder(&frame, r * k, 30.0 * k).unwrap();
                let in_front = boxed(k, [47.5 - t, -8.0, 0.0], [47.5 + t, t, 20.0]);
                let rod = boolean(&inner_rod, &in_front, BooleanOp::Subtract, &tol)
                    .unwrap()
                    .body;
                let hollow = boolean(&inner_box, &rod, BooleanOp::Union, &tol)
                    .unwrap()
                    .body;
                let identity = boolean(&body, &hollow, BooleanOp::Subtract, &tol)
                    .unwrap()
                    .body;
                assert_close(mass(&identity).volume, mass(&made.body).volume, &what);
            }
        }
    }
}

// ------------------------------- more split corners (the review of 2026-11-14)

/// The most faces round any vertex of `body`.
fn most_faces_round(body: &Body) -> usize {
    faces_round(body).iter().map(Vec::len).max().unwrap_or(0)
}

/// The hexagonal drain's three directions: the angle of the floor's
/// fall across them, and how far the rim lies on either side of the drain
/// across each (not the same on both sides, so no two facets match).
const HEX: [(f64, f64, f64); 3] = [
    (0.0, -30.0, 26.0),
    (60.0, -24.0, 32.0),
    (120.0, -20.0, 22.0),
];
const HEX_DEPTH: f64 = 8.0;
const HEX_TOP: f64 = 20.0;

/// Along and across the direction at `angle` degrees.
fn hex_axes(angle: f64) -> (Vec3, Vec3) {
    let a = angle.to_radians();
    (
        Vec3::new(a.pcos(), a.psin(), 0.0),
        Vec3::new(-a.psin(), a.pcos(), 0.0),
    )
}

/// The rim moved in by `t`: the hexagon where every `lo + t ≤ n·p ≤ hi − t`.
fn hexagon(t: f64) -> Vec<[f64; 2]> {
    let mut poly = vec![
        [-500.0, -500.0],
        [500.0, -500.0],
        [500.0, 500.0],
        [-500.0, 500.0],
    ];
    for (angle, lo, hi) in HEX {
        let (_, n) = hex_axes(angle);
        poly = clip(&poly, (n.x, n.y, -(lo + t)));
        poly = clip(&poly, (-n.x, -n.y, hi - t));
    }
    poly
}

/// A hexagonal trough `HEX_TOP` high whose floor falls `HEX_DEPTH` to a
/// drain at the origin in six facets, each pair across one direction a
/// V prism of the floor's fall, the three crossed: the drain is a vertex of
/// six faces, convex all round, the six bottom corners of four.
fn hex_drain(k: f64) -> Body {
    let tol = tolerance(k);
    let mut pyramid: Option<Body> = None;
    for (angle, lo, hi) in HEX {
        let (d, n) = hex_axes(angle);
        let frame = Frame::new(d * -150.0, d, n).unwrap();
        let v = prism(k, &frame, &[[lo, 0.0], [hi, 0.0], [0.0, -HEX_DEPTH]], 300.0);
        pyramid = Some(match pyramid {
            None => v,
            Some(acc) => boolean(&acc, &v, BooleanOp::Intersect, &tol).unwrap().body,
        });
    }
    let up = prism(k, &Frame::WORLD, &hexagon(0.0), HEX_TOP);
    boolean(&up, &pyramid.unwrap(), BooleanOp::Union, &tol)
        .unwrap()
        .body
}

/// The area of a polygon (counter-clockwise).
fn polygon_area(poly: &[[f64; 2]]) -> f64 {
    over_polygon(poly, (1.0, 0.0, 0.0))
}

/// The hexagonal drain, shelled open at its top (the review of
/// 2026-11-14: a vertex of more than four faces was not tried). The drain
/// is a vertex of six faces; the split makes a tree of four corners, its
/// triangles fanning three to a face. The volume: the trough with its
/// pyramid (the hexagon's area times `HEX_TOP + HEX_DEPTH / 3`), less the
/// cavity `∫ (HEX_TOP − max of the six facets moved up) dA` over the rim
/// moved in, exactly per facet over where it is highest. At three scales
/// and turned.
#[test]
fn a_hexagonal_drain_splits_its_vertex_of_six_faces() {
    let area = polygon_area(&hexagon(0.0));
    let solid = area * (HEX_TOP + HEX_DEPTH / 3.0);
    for (k, turning) in placements() {
        let tol = tolerance(k);
        let body = placed(hex_drain(k), &turning);
        assert_volume(&body, solid, k, "the hexagonal drain");
        assert_eq!(most_faces_round(&body), 6, "the drain");
        let top = face_at(&body, &place(k, &turning), "plane", [0.0, 0.0, HEX_TOP]);
        for t in [1.0, 2.0] {
            let what = format!("t {t}, scale {k}, turned {}", turning.is_some());
            let made = shell(&body, t * k, &[top], &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            assert_eq!(made.body.shells.len(), 1, "{what}");
            let mut planes = Vec::new();
            for (angle, lo, hi) in HEX {
                let (_, n) = hex_axes(angle);
                for w in [lo, hi] {
                    let s = HEX_DEPTH / w;
                    let lift = t * (1.0 + s * s).sqrt();
                    planes.push((-HEX_DEPTH + lift, s * n.x, s * n.y));
                }
            }
            let rim = hexagon(t);
            let cavity = HEX_TOP * polygon_area(&rim) - integral_of_max_over(&planes, &rim);
            assert_volume(&made.body, solid - cavity, k, &what);
            split_cleanly(&body, &made, &what);
        }
    }
}

/// The circle in a plane through (0, 0) and (`end`, −`h`) about the point
/// `s = sc` of the first axis: its centre's height and radius.
fn arc(sc: f64, end: f64, h: f64) -> (f64, f64) {
    let zc = (sc * sc - (end - sc) * (end - sc) - h * h) / (2.0 * h);
    (zc, (sc * sc + zc * zc).sqrt())
}

/// A trough over `x 0..50`, `y 0..30` (from `at`), from 9 below its rim to
/// `top` above it, whose floor falls 8 from the rim to a drain in the
/// middle in four cylinders: across x the arcs through the rim and the
/// drain about `x = sx` (and the mirror), across y about `y = sy`. Bowls
/// (the solid inside the tubes) or humps (outside).
fn curved(k: f64, bowl: bool, (sx, sy): (f64, f64), at: [f64; 3], top: f64) -> Body {
    let tol = tolerance(k);
    let (a, b, h) = (25.0, 15.0, 8.0);
    let mut body = boxed(
        k,
        [at[0], at[1], at[2] - 9.0],
        [at[0] + 50.0, at[1] + 30.0, at[2] + top],
    );
    let (zx, rx) = arc(sx, a, h);
    let (zy, ry) = arc(sy, b, h);
    let tubes = [
        (Vec3::new(sx, -20.0, zx), Vec3::Y, Vec3::X, rx, 70.0),
        (
            Vec3::new(2.0 * a - sx, -20.0, zx),
            Vec3::Y,
            Vec3::X,
            rx,
            70.0,
        ),
        (Vec3::new(-20.0, sy, zy), Vec3::X, Vec3::Y, ry, 90.0),
        (
            Vec3::new(-20.0, 2.0 * b - sy, zy),
            Vec3::X,
            Vec3::Y,
            ry,
            90.0,
        ),
    ];
    let shift = Vec3::new(at[0], at[1], at[2]);
    for (origin, axis, x, r, length) in tubes {
        let frame = Frame::new((origin + shift) * k, axis, x).unwrap();
        let tube = craft3d_kernel::primitives::cylinder(&frame, r * k, length * k).unwrap();
        let op = if bowl {
            BooleanOp::Intersect
        } else {
            BooleanOp::Subtract
        };
        body = boolean(&body, &tube, op, &tol).unwrap().body;
    }
    body
}

/// A drain whose floor is four bowls (the review of 2026-11-14: curved
/// faces were reasoned, not measured), shelled open at its top. The drain
/// and the four bottom corners are vertices of four convex faces, here
/// cylinders. The volumes by `∫ Lx(z) Ly(z) dz`: below a level z the floor
/// `max(E(x), N(y))` is under it on a product of two intervals, their
/// lengths in closed form from the arcs (moved in: radius less t), the
/// integral by Gauss–Legendre between the levels where an interval meets
/// a wall or the middle (`plan/shell-limits/tools/curved.py`); the
/// review's quadrature over x with the integral across y in closed form
/// gives the same to 1e-14. At three scales and turned; and once with the
/// four cylinders as rational spline twins, so that splines meet at every
/// vertex split (each within the tolerance over the area of the same).
#[test]
fn a_drain_of_four_bowls_splits_its_curved_corners() {
    let solid = 35912.661205509234;
    let shelled = [(1.0, 4740.201480742871), (5.0, 19780.46881274651)];
    for (k, turning) in placements() {
        let tol = tolerance(k);
        let body = placed(curved(k, true, (28.0, 17.0), [0.0; 3], 20.0), &turning);
        assert_volume(&body, solid, k, "the bowls");
        let top = face_at(&body, &place(k, &turning), "plane", [25.0, 15.0, 20.0]);
        for (t, want) in shelled {
            let what = format!("t {t}, scale {k}, turned {}", turning.is_some());
            let made = shell(&body, t * k, &[top], &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            assert_volume(&made.body, want, k, &what);
            split_cleanly(&body, &made, &what);
        }
    }
    let tol = tolerance(1.0);
    let body = curved(1.0, true, (28.0, 17.0), [0.0; 3], 20.0);
    let (twin, replaced) = rational_common::rationalized(&body, 0.25, false);
    assert_eq!(replaced, 4);
    let top = face_at(&twin, &place(1.0, &None), "plane", [25.0, 15.0, 20.0]);
    let made = shell(&twin, 1.0, &[top], &tol).unwrap_or_else(|e| panic!("splines: {e}"));
    sound(&made.body, 1.0, "splines");
    assert_volume(&made.body, shelled[0].1, 1.0, "splines");
    split_cleanly(&twin, &made, "splines");
}

/// A slab 140 × 90 × 50 with a pocket whose floor is four humps (the
/// solid outside the tubes), shelled open at its top: the pocket's corners
/// and drain are vertices of four concave faces, cylinders; the lining is a
/// lump of its own. The volumes and the lining's by the same integral as
/// the bowls', the floor moved down (the humps' tubes less t, the walls
/// out by t).
#[test]
fn a_pocket_of_four_humps_splits_its_curved_corners() {
    let k = 1.0;
    let tol = tolerance(k);
    let solid = 582557.3334889764;
    let slab = boxed(k, [0.0, 0.0, 0.0], [140.0, 90.0, 50.0]);
    let tool = curved(k, false, (-5.0, -4.0), [20.0, 15.0, 20.0], 35.0);
    let body = boolean(&slab, &tool, BooleanOp::Subtract, &tol)
        .unwrap()
        .body;
    assert_volume(&body, solid, k, "the pocketed slab");
    let top = face_at(&body, &place(k, &None), "plane", [5.0, 5.0, 50.0]);
    for (t, want, lining) in [
        (1.0, 41612.260852116626, 6668.260852116589),
        (3.0, 122721.62723377306, 21753.627233773077),
    ] {
        let what = format!("t {t}");
        let made = shell(&body, t, &[top], &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
        sound(&made.body, k, &what);
        assert_volume(&made.body, want, k, &what);
        split_cleanly(&body, &made, &what);
        assert_eq!(made.body.shells.len(), 2, "{what}");
        let mut lumps: Vec<f64> = (0..2)
            .map(|i| mass(&boolean_common::shell_alone(&made.body, i)).volume)
            .collect();
        lumps.sort_by(f64::total_cmp);
        let slack = tol.linear * mass(&made.body).area;
        assert!(
            (lumps[0] - lining).abs() <= slack,
            "{what}: the lining {} against {lining}",
            lumps[0]
        );
    }
}

/// A trough 40 × 20 whose floor falls 6 to a drain, its two facets across
/// x humps (arcs about x = −1 and 41), the two across y planes. Moved in,
/// the humps rise at the drain faster than the planes at first and slower
/// later: the drain's new edge runs between the humps below the flip `t*`
/// (where all four moved faces meet in one point again: the humps' height
/// at the drain equals the planes', 2.319594155090215 by bisection on the
/// closed forms) and between the planes above it.
fn flip(k: f64) -> Body {
    let tol = tolerance(k);
    let (zc, r) = arc(-1.0, 20.0, 6.0);
    let mut body = boxed(k, [0.0, 0.0, -7.0], [40.0, 20.0, 20.0]);
    for cx in [-1.0, 41.0] {
        let frame = Frame::new(Vec3::new(cx, -20.0, zc) * k, Vec3::Y, Vec3::X).unwrap();
        let tube = craft3d_kernel::primitives::cylinder(&frame, r * k, 60.0 * k).unwrap();
        body = boolean(&body, &tube, BooleanOp::Subtract, &tol)
            .unwrap()
            .body;
    }
    let across = Frame::new(Vec3::new(-5.0, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
    let v = prism(
        k,
        &across,
        &[
            [0.0, 25.0],
            [0.0, 0.0],
            [10.0, -6.0],
            [20.0, 0.0],
            [20.0, 25.0],
        ],
        50.0,
    );
    boolean(&body, &v, BooleanOp::Intersect, &tol).unwrap().body
}

/// The flip (the review of 2026-11-14): below `t*` the drain's new edge
/// lies between the humps' hollow faces, above it between the planes', and
/// at `t*` the four meet in one point and there is neither. Volumes by the
/// same integral as the bowls'.
#[test]
fn the_drains_new_edge_flips_where_its_four_faces_meet_again() {
    let k = 1.0;
    let tol = tolerance(k);
    let flip_at = 2.319594155090215;
    let body = flip(k);
    assert_volume(&body, 17139.49263785922, k, "the flip's trough");
    let at = place(k, &None);
    let humps = [
        face_at(&body, &at, "cylinder", [10.0, 10.0, -3.0]),
        face_at(&body, &at, "cylinder", [30.0, 10.0, -3.0]),
    ];
    let planes = [
        face_at(&body, &at, "plane", [20.0, 5.0, -3.0]),
        face_at(&body, &at, "plane", [20.0, 15.0, -3.0]),
    ];
    let top = face_at(&body, &at, "plane", [20.0, 10.0, 20.0]);
    // Below the flip (-1), at it (0), above it (1).
    for (t, want, side) in [
        (1.0, 3077.122255358154, -1),
        (2.3, 6544.695073963119, -1),
        (flip_at - 1e-3, f64::NAN, -1),
        (flip_at, 6592.559627037635, 0),
        (flip_at + 1e-3, f64::NAN, 1),
        (2.35, 6666.583395952524, 1),
        (4.0, 10241.273948338241, 1),
    ] {
        let what = format!("t {t}");
        let made = shell(&body, t, &[top], &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
        sound(&made.body, k, &what);
        if want.is_finite() {
            assert_volume(&made.body, want, k, &what);
        }
        let uses = craft3d_topo::edge_faces(&made.body);
        let behind = |src: FaceId| -> Vec<FaceId> {
            made.faces
                .iter()
                .enumerate()
                .filter(|(_, f)| **f == (Skin::Inner, src))
                .map(|(i, _)| FaceId(i as u32))
                .collect()
        };
        let between = |a: FaceId, b: FaceId| {
            let (a, b) = (behind(a), behind(b));
            uses.iter()
                .filter(|faces| {
                    faces.len() == 2
                        && ((a.contains(&faces[0]) && b.contains(&faces[1]))
                            || (a.contains(&faces[1]) && b.contains(&faces[0])))
                })
                .count()
        };
        let (across_x, across_y) = (between(humps[0], humps[1]), between(planes[0], planes[1]));
        let expected = match side {
            -1 => (1, 0),
            0 => (0, 0),
            _ => (0, 1),
        };
        assert_eq!((across_x, across_y), expected, "{what}");
    }
}

/// A corner of both kinds where both trees can be made (the review of
/// 2026-11-14): a box and a block whose end is tilted off every axis, the
/// block's floor on the box's front edge, so that the box's front and side
/// and the block's floor and end meet in one point, convex and concave in
/// turn, no two of them parallel. Moved in, both trees exist; the faces do
/// not decide between them, so it is refused as before, naming the four.
#[test]
fn a_corner_of_both_kinds_with_two_trees_is_refused() {
    let k = 1.0;
    let tol = tolerance(k);
    let box_ = boxed(k, [-20.0, 0.0, -20.0], [0.0, 20.0, 20.0]);
    let block = boxed(k, [-30.0, -20.0, 0.0], [30.0, 20.0, 10.0]);
    let normal = Vec3::new(1.0, 0.5, 0.3);
    let side = Frame::new(Vec3::ZERO, normal, Vec3::Y).unwrap();
    let half = prism(
        k,
        &side,
        &[
            [-100.0, -100.0],
            [100.0, -100.0],
            [100.0, 100.0],
            [-100.0, 100.0],
        ],
        100.0,
    );
    let block = boolean(&block, &half, BooleanOp::Intersect, &tol)
        .unwrap()
        .body;
    let body = boolean(&box_, &block, BooleanOp::Union, &tol).unwrap().body;
    let left = face_at(&body, &place(k, &None), "plane", [-20.0, 10.0, 0.0]);
    match shell(&body, 1.0, &[left], &tol) {
        Err(
            KernelError::FaceOp { faces, reason, .. }
            | KernelError::FaceOpTooFar { faces, reason, .. },
        ) => {
            assert!(reason.contains("no longer meet in one point"), "{reason}");
            assert!(faces.len() >= 4, "{faces:?}");
        }
        other => panic!("the corner of both kinds: {other:?}"),
    }
}
