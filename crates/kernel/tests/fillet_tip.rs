//! A fillet that ends in a point: its radius runs to nothing at an end of
//! the edge, or short of it (`Fade::tip`), and the fillet face closes in a
//! vertex on the edge there, tangent to both faces all the way.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::{SCALES, mass, tolerance};
use craft3d_geom::Surface;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::fillet::{
    Fade, FilletFace, FilletLawEdge, FilletResult, RadiusLaw, fillet_by_law,
};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

/// The box from `lo` with `size`, in `frame`, at scale `k`.
fn block(frame: &Frame, k: f64, lo: [f64; 3], size: [f64; 3]) -> Body {
    let corner = frame.point(lo[0] * k, lo[1] * k, lo[2] * k);
    let at = Frame::new(corner, frame.z(), frame.x()).unwrap();
    primitives::block(&at, size[0] * k, size[1] * k, size[2] * k).unwrap()
}

/// The edge whose middle is nearest `middle` (in `frame`, at scale 1).
fn edge_at(body: &Body, frame: &Frame, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = frame.point(middle[0] * k, middle[1] * k, middle[2] * k);
    let (index, off) = body
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
    assert!(off < 1e-3 * k, "no edge at {middle:?}: {off}");
    EdgeId(index as u32)
}

/// The law of `radius` with `fade` at the end of `edge` further along
/// `up`, nothing at the other.
fn law_towards(body: &Body, edge: EdgeId, up: Vec3, radius: f64, fade: Fade) -> RadiusLaw {
    let data = body.edge(edge).unwrap();
    let along = data.curve.eval(data.range.1) - data.curve.eval(data.range.0);
    RadiusLaw {
        radius,
        fades: if along.dot(up) > 0.0 {
            [None, Some(fade)]
        } else {
            [Some(fade), None]
        },
    }
}

/// The one fillet face of a result.
fn fillet_face(made: &FilletResult) -> FaceId {
    let found: Vec<usize> = (0..made.faces.len())
        .filter(|i| matches!(made.faces[*i], FilletFace::Fillet(_)))
        .collect();
    assert_eq!(found.len(), 1, "one fillet face");
    FaceId(found[0] as u32)
}

/// What a document asks of a body before it hands it on: valid, meshed
/// watertight at two chords with the volume it encloses, and clear of
/// itself on the mesh of the self-check.
fn sound(body: &Body, k: f64, what: &str) {
    let tol = tolerance(k);
    assert_eq!(check(body, &tol), [], "{what}");
    let exact = mass(body);
    for chord in [0.05, 0.005] {
        let params = TessParams {
            chordal_tolerance: chord * k,
            ..TessParams::default()
        };
        let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
        let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert!(welded.is_closed(), "{what}: the mesh is not watertight");
        assert!(
            (welded.volume() - exact.volume).abs() <= exact.area * chord * k,
            "{what}: mesh volume {} against {}",
            welded.volume(),
            exact.volume
        );
    }
    let found = body_self_intersections(body, &SelfCheckParams::new(tol.linear))
        .unwrap_or_else(|e| panic!("{what}: the self-check cannot mesh it: {e}"));
    assert!(
        found.crossings.is_empty() && found.certain_crossings == 0,
        "{what}: {} crossings",
        found.crossings.len()
    );
}

/// The block 40 × 30 × 40 whose upright edge at the origin is rounded 3
/// from the bottom, the radius running to nothing over 15 at `back` short
/// of the top: with its closed form. Between two planes at a right angle
/// the section at each height is the quarter circle of that height's
/// radius, so the fillet takes `(1 − π/4) ∫ r² dz`, and over a fade to
/// nothing `∫ r² = 9/14 R² L`.
fn tipped(frame: &Frame, k: f64, back: f64) -> (Body, FilletResult, f64) {
    let body = block(frame, k, [0.0, 0.0, 0.0], [40.0, 30.0, 40.0]);
    let edge = edge_at(&body, frame, k, [0.0, 0.0, 20.0]);
    let law = law_towards(
        &body,
        edge,
        frame.z(),
        3.0 * k,
        Fade::tip(15.0 * k, back * k),
    );
    let made = fillet_by_law(&body, &[FilletLawEdge { edge, law }], &tolerance(k))
        .unwrap_or_else(|e| panic!("back {back} at {k}: {e}"));
    let taken = (1.0 - 0.25 * PI) * 9.0 * ((40.0 - back - 15.0) + 15.0 * 9.0 / 14.0);
    (body, made, taken)
}

/// The first question of a tip (the fade's design, its first risk): is a
/// fitted rational face whose last row is exactly a point taken by what
/// comes after a fillet? Here: the validity check, the measure against
/// the closed form, the tessellator (watertight at two chords, the mesh's
/// volume the body's), the self-check on a mesh, and booleans through the
/// tip, beside it, across the face and along it. (STEP out and back:
/// `crates/io/tests/step_tip.rs`.)
#[test]
fn a_face_that_closes_in_a_point_is_taken_by_what_comes_after_it() {
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in common::frames(k).iter().enumerate() {
            for back in [0.0, 5.0] {
                let what = format!("back {back}, scale {k}, frame {index}");
                let (body, made, taken) = tipped(frame, k, back);
                // The face: three sides, a fitted rational surface whose
                // last row is one point, the tip.
                let face = made.body.face(fillet_face(&made)).unwrap();
                let Surface::Nurbs(surface) = &face.surface else {
                    panic!("{what}: not a fitted face");
                };
                let coedges: usize = face
                    .loops
                    .iter()
                    .map(|l| made.body.loop_(*l).unwrap().coedges.len())
                    .sum();
                assert_eq!(coedges, 3, "{what}");
                let (rows, across) = surface.counts();
                let tip = frame.point(0.0, 0.0, (40.0 - back) * k);
                let points = surface.control_points();
                let first_row = &points[..across];
                let last_row = &points[(rows - 1) * across..];
                let pole = if first_row[0].distance(tip) < last_row[0].distance(tip) {
                    first_row
                } else {
                    last_row
                };
                assert!(pole.iter().all(|p| *p == pole[0]), "{what}: {pole:?}");
                assert!(pole[0].distance(tip) <= 1e-12 * 40.0 * k, "{what}");
                assert!(surface.weights().iter().any(|w| *w != 1.0), "{what}");
                // Valid, measured, meshed, clear of itself.
                sound(&made.body, k, &what);
                let (before, after) = (mass(&body), mass(&made.body));
                let slack = 0.5 * PI * 3.0 * 40.0 * k * k * tol.linear;
                let off = (before.volume - after.volume - taken * k * k * k).abs();
                assert!(off <= 0.1 * slack, "{what}: the volume is {off:e} off");
                // Cut and kept by boxes: through the tip, beside it,
                // across the fade, along the face, and one whose face
                // lies in the tip itself.
                let top = 40.0 - back;
                let mut worst = 0.0_f64;
                for (name, lo, size) in [
                    (
                        "through the tip",
                        [-5.0, -5.0, top - 2.0],
                        [10.0, 10.0, 4.0],
                    ),
                    ("beside the tip", [0.5, -5.0, top - 2.0], [10.0, 10.0, 4.0]),
                    (
                        "across the fade",
                        [-5.0, -5.0, top - 8.0],
                        [10.0, 10.0, 20.0],
                    ),
                    ("along the face", [-5.0, -5.0, 10.0], [5.3, 60.0, 60.0]),
                    ("at the tip's height", [-5.0, -5.0, top], [10.0, 10.0, 20.0]),
                    (
                        "a corner in the face",
                        [0.2, 0.3, top - 6.0],
                        [10.0, 10.0, 3.0],
                    ),
                ] {
                    let tool = block(frame, k, lo, size);
                    let mut parts = 0.0;
                    for op in [BooleanOp::Subtract, BooleanOp::Intersect] {
                        let out = match boolean(&made.body, &tool, op, &tol) {
                            Ok(out) => out,
                            // (Above a tip at the top there is no body.)
                            Err(e) if e.to_string().contains("leaves nothing") => continue,
                            Err(e) => panic!("{what}, {name}, {op:?}: {e}"),
                        };
                        assert_eq!(check(&out.body, &tol), [], "{what}, {name}, {op:?}");
                        parts += mass(&out.body).volume;
                    }
                    let off = (parts - after.volume).abs() / (after.area * tol.linear);
                    worst = worst.max(off);
                    assert!(
                        off <= 1.0,
                        "{what}, {name}: the parts are {off} tolerances of area off the whole"
                    );
                }
                println!(
                    "{what}: {rows} rows, volume off {:.3} of the slack, the parts of a cut within {worst:.2e} of the area times the tolerance",
                    off / slack
                );
            }
        }
    }
}

/// The radius of the block's fillet at the height `z` (scale 1): 3 from
/// the bottom, running to nothing over 15 at `back` short of the top.
fn radius_at(z: f64, back: f64) -> f64 {
    let tau = (40.0 - back - z) / 15.0;
    if tau <= 0.0 {
        0.0
    } else if tau < 1.0 {
        let m = 1.0 - tau;
        3.0 * (1.0 - m * m * m)
    } else {
        3.0
    }
}

/// The two borders of the fillet face that end in the vertex at `tip`:
/// each as its edge and whether the tip is its end (not its start).
fn borders_into(made: &FilletResult, tip: Vec3, k: f64) -> Vec<(EdgeId, bool)> {
    let body = &made.body;
    let face = body.face(fillet_face(made)).unwrap();
    let mut found = Vec::new();
    for l in &face.loops {
        for coedge in &body.loop_(*l).unwrap().coedges {
            let edge = body.edge(coedge.edge).unwrap();
            for (vertex, at_end) in [(edge.start, false), (edge.end, true)] {
                if body.vertex(vertex).unwrap().point.distance(tip) <= 1e-9 * k {
                    found.push((coedge.edge, at_end));
                }
            }
        }
    }
    found
}

/// At the tip both borders end in the one vertex, on the edge, and each
/// leaves it at the angle the law gives: a foot lies `r cot(θ/2)` from the
/// edge and the radius falls by `3R/L` per length at the tip, so between
/// planes at a right angle the border makes the angle whose tangent is
/// `3R/L` with the edge, 30.96° here. Along the borders the face is
/// tangent to the planes as closely as the tolerance makes over the
/// radius of each place, down to a hundredth of the fade's length from
/// the tip.
#[test]
fn the_borders_end_in_the_tip_at_the_angle_of_the_law_and_tangent_all_the_way() {
    let expected = (3.0 * 3.0 / 15.0_f64).patan();
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in common::frames(k).iter().enumerate() {
            for back in [0.0, 5.0] {
                let what = format!("back {back}, scale {k}, frame {index}");
                let (_, made, _) = tipped(frame, k, back);
                let body = &made.body;
                let tip = frame.point(0.0, 0.0, (40.0 - back) * k);
                let borders = borders_into(&made, tip, k);
                assert_eq!(borders.len(), 2, "{what}");
                let own = fillet_face(&made);
                let (mut widest, mut leaning) = (0.0_f64, 0.0_f64);
                for (edge, at_end) in borders {
                    let data = body.edge(edge).unwrap();
                    // The angle at the tip.
                    let t = if at_end { data.range.1 } else { data.range.0 };
                    let (point, d) = data.curve.derivatives(t);
                    assert!(point.distance(tip) <= 1e-9 * k, "{what}");
                    let cosine = d.normalized().unwrap().dot(frame.z()).abs();
                    let angle = cosine.min(1.0).pacos();
                    widest = widest.max((angle - expected).abs());
                    assert!(
                        (angle - expected).abs() <= 1e-6,
                        "{what}: a border leaves the tip at {angle}, not {expected}"
                    );
                    // The plane the border lies on: the other face of it.
                    let beside = (0..body.faces.len())
                        .map(|i| FaceId(i as u32))
                        .find(|f| {
                            *f != own
                                && body.face(*f).unwrap().loops.iter().any(|l| {
                                    body.loop_(*l)
                                        .unwrap()
                                        .coedges
                                        .iter()
                                        .any(|c| c.edge == edge)
                                })
                        })
                        .unwrap();
                    // Tangent, at places ever nearer the tip.
                    for from_tip in [7.5, 3.0, 1.0, 0.3, 0.15] {
                        let level = (40.0 - back - from_tip) * k;
                        let (mut lo, mut hi) = data.range;
                        let height = |t: f64| frame.local(data.curve.eval(t)).2;
                        if height(lo) > height(hi) {
                            (lo, hi) = (hi, lo);
                        }
                        for _ in 0..100 {
                            let middle = 0.5 * (lo + hi);
                            if height(middle) < level {
                                lo = middle;
                            } else {
                                hi = middle;
                            }
                        }
                        let p = data.curve.eval(0.5 * (lo + hi));
                        let normal = |f: FaceId| {
                            let face = body.face(f).unwrap();
                            let (u, v) = face.surface.project(p).unwrap();
                            let n = face.surface.normal(u, v).unwrap();
                            if face.reversed { -n } else { n }
                        };
                        let sine = normal(own).cross(normal(beside)).length();
                        let radius = radius_at(40.0 - back - from_tip, back) * k;
                        let bound = (tol.linear / radius).max(1e-9);
                        leaning = leaning.max(sine / bound);
                        assert!(
                            sine <= bound,
                            "{what}: {from_tip} from the tip the face leans {sine:e} off its neighbour, of {bound:e}"
                        );
                    }
                }
                println!(
                    "{what}: the borders' angle at the tip within {widest:.1e} rad, their lean at most {leaning:.2e} of the tolerance over the radius"
                );
            }
        }
    }
}

/// Beyond the tip nothing is touched. Short of the top, what is left of
/// the edge is an edge between the same two faces on the edge's own curve,
/// from the tip to the edge's own end; at the top, the vertex and every
/// other edge there are what they were. The two faces keep their
/// surfaces, to the last bit.
#[test]
fn the_edge_and_the_faces_beyond_a_tip_are_what_they_were() {
    for k in SCALES {
        for (index, frame) in common::frames(k).iter().enumerate() {
            for back in [0.0, 5.0] {
                let what = format!("back {back}, scale {k}, frame {index}");
                let (body, made, _) = tipped(frame, k, back);
                let edge = edge_at(&body, frame, k, [0.0, 0.0, 20.0]);
                let old = body.edge(edge).unwrap();
                let corner = frame.point(0.0, 0.0, 40.0 * k);
                let tip = frame.point(0.0, 0.0, (40.0 - back) * k);
                // Every face of the body is there on its own surface.
                for (now, source) in made.faces.iter().enumerate() {
                    if let FilletFace::Kept(from) = source {
                        assert_eq!(from.len(), 1, "{what}");
                        let (was, is) = (body.face(from[0]).unwrap(), &made.body.faces[now]);
                        assert!(
                            was.surface == is.surface && was.reversed == is.reversed,
                            "{what}"
                        );
                    }
                }
                // Every edge the result calls the body's own is on its own
                // curve, and one that ends at the top corner has its range.
                let mut at_corner = 0;
                for (now, source) in made.edges.iter().enumerate() {
                    let Some(from) = source else { continue };
                    let (was, is) = (body.edge(*from).unwrap(), &made.body.edges[now]);
                    assert!(was.curve == is.curve, "{what}: edge {now}");
                    let ends = |b: &Body, e: &craft3d_topo::Edge| {
                        [
                            b.vertex(e.start).unwrap().point,
                            b.vertex(e.end).unwrap().point,
                        ]
                    };
                    if ends(&body, was).contains(&corner) {
                        at_corner += 1;
                        assert_eq!(was.range, is.range, "{what}: edge {now}");
                        assert_eq!(ends(&body, was), ends(&made.body, is), "{what}: edge {now}");
                    }
                }
                assert_eq!(at_corner, 2, "{what}: the two edges of the top there");
                // The corner is a vertex still, where it was, and the
                // result says it is the body's own: also where the fillet
                // closes in it.
                let own = made
                    .body
                    .vertices
                    .iter()
                    .position(|v| v.point == corner)
                    .unwrap_or_else(|| panic!("{what}: the corner is gone"));
                let was = made.vertices[own].unwrap_or_else(|| panic!("{what}: a new vertex"));
                assert_eq!(body.vertex(was).unwrap().point, corner, "{what}");
                // What is left of the edge.
                let rest: Vec<&craft3d_topo::Edge> = made
                    .body
                    .edges
                    .iter()
                    .filter(|e| {
                        let (a, b) = (
                            made.body.vertex(e.start).unwrap().point,
                            made.body.vertex(e.end).unwrap().point,
                        );
                        (a == corner && b.distance(tip) <= 1e-9 * k)
                            || (b == corner && a.distance(tip) <= 1e-9 * k)
                    })
                    .collect();
                if back == 0.0 {
                    assert!(rest.is_empty(), "{what}");
                    continue;
                }
                let [rest] = rest[..] else {
                    panic!("{what}: {} edges from the tip to the corner", rest.len());
                };
                assert!(rest.curve == old.curve, "{what}");
                // From the tip to the edge's own end, whichever end the
                // corner is.
                let (t0, t1) = old.range;
                let kept_end = if body.vertex(old.end).unwrap().point == corner {
                    assert_eq!(rest.range.1, t1, "{what}");
                    rest.range.0
                } else {
                    assert_eq!(rest.range.0, t0, "{what}");
                    rest.range.1
                };
                assert!(old.curve.eval(kept_end).distance(tip) <= 1e-9 * k, "{what}");
            }
        }
    }
}

/// Every point of a grid round the edge is in the body or out of it as
/// the law says: in the block, and not in what the arc of its height cuts
/// from the corner (nothing from the tip on).
#[test]
fn points_round_the_edge_are_in_and_out_as_the_law_says() {
    use craft3d_kernel::boolean::contains_points;
    let margin = 1e-4;
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in common::frames(k).iter().enumerate() {
            for back in [0.0, 5.0] {
                let what = format!("back {back}, scale {k}, frame {index}");
                let (_, made, _) = tipped(frame, k, back);
                let (mut points, mut expected) = (Vec::new(), Vec::new());
                let mut taken = 0;
                let n = 10;
                for i in 0..n {
                    for j in 0..n {
                        for l in 0..3 * n {
                            let at = |m: usize, lo: f64, hi: f64, count: usize| {
                                lo + (hi - lo) * (m as f64 + 0.37) / count as f64
                            };
                            let (x, y, z) = (
                                at(i, -0.4, 3.6, n),
                                at(j, -0.4, 3.6, n),
                                at(l, -1.0, 41.0, 3 * n),
                            );
                            let r = radius_at(z, back);
                            let from_arc = (x - r).phypot(y - r) - r;
                            let in_corner = x < r && y < r && from_arc > 0.0;
                            let near = [x, y, z, 40.0 - z].iter().any(|d| d.abs() < margin)
                                || (r > 0.0
                                    && x < r + margin
                                    && y < r + margin
                                    && from_arc.abs() < margin);
                            if near {
                                continue;
                            }
                            let in_block = x > 0.0 && y > 0.0 && z > 0.0 && z < 40.0;
                            points.push(frame.point(x * k, y * k, z * k));
                            expected.push(in_block && !in_corner);
                            if in_block && in_corner {
                                taken += 1;
                            }
                        }
                    }
                }
                let found = contains_points(&made.body, &points, &tol).unwrap();
                let mut undecided = 0;
                for ((p, want), got) in points.iter().zip(&expected).zip(&found) {
                    match got {
                        Some(got) => assert_eq!(got, want, "{what}: at {:?}", frame.local(*p)),
                        None => undecided += 1,
                    }
                }
                assert!(
                    undecided * 100 < points.len(),
                    "{what}: {undecided} undecided"
                );
                assert!(
                    taken > 100,
                    "{what}: only {taken} points in what the fillet takes"
                );
            }
        }
    }
}

/// Cuts the edge `edge` of `body` in two at its parameter `t`: a vertex
/// there, the edge up to it and a new edge from it on, both faces running
/// the two where they ran the one. The new edge.
fn split(body: &mut Body, edge: EdgeId, t: f64) -> EdgeId {
    let data = body.edge(edge).unwrap().clone();
    let vertex = craft3d_topo::VertexId(body.vertices.len() as u32);
    body.vertices.push(craft3d_topo::Vertex {
        point: data.curve.eval(t),
    });
    let second = EdgeId(body.edges.len() as u32);
    body.edges.push(craft3d_topo::Edge {
        curve: data.curve.clone(),
        range: (t, data.range.1),
        start: vertex,
        end: data.end,
    });
    let first = &mut body.edges[edge.0 as usize];
    first.range.1 = t;
    first.end = vertex;
    for ring in &mut body.loops {
        if let Some(at) = ring.coedges.iter().position(|c| c.edge == edge) {
            let reversed = ring.coedges[at].reversed;
            ring.coedges.insert(
                if reversed { at } else { at + 1 },
                craft3d_topo::Coedge {
                    edge: second,
                    reversed,
                },
            );
        }
    }
    second
}

/// A tip at a vertex of two faces: the edge runs on there into a next
/// edge between the same two faces (a stem into a keel). The block's
/// upright edge cut in two at 25: the lower piece rounded 3 with a fade
/// to nothing over 15 at the cut ends in a point there, and the upper
/// piece is what it was. Without the fade the lower piece cannot end
/// there at all.
#[test]
fn a_tip_where_the_edge_runs_on_between_the_same_two_faces() {
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in common::frames(k).iter().enumerate() {
            let what = format!("scale {k}, frame {index}");
            let mut body = block(frame, k, [0.0, 0.0, 0.0], [40.0, 30.0, 40.0]);
            let whole = edge_at(&body, frame, k, [0.0, 0.0, 20.0]);
            let data = body.edge(whole).unwrap().clone();
            let at = |z: f64| {
                let (t0, t1) = data.range;
                let up = frame.local(data.curve.eval(t1)).2 > frame.local(data.curve.eval(t0)).2;
                if up {
                    t0 + (t1 - t0) * z / 40.0
                } else {
                    t1 - (t1 - t0) * z / 40.0
                }
            };
            split(&mut body, whole, at(25.0));
            assert_eq!(
                check(&body, &tol),
                [],
                "{what}: the block with its edge in two"
            );
            let lower = edge_at(&body, frame, k, [0.0, 0.0, 12.5]);
            let upper = edge_at(&body, frame, k, [0.0, 0.0, 32.5]);
            // With one radius it has no end there.
            let plain = RadiusLaw::constant(3.0 * k);
            assert!(
                fillet_by_law(
                    &body,
                    &[FilletLawEdge {
                        edge: lower,
                        law: plain
                    }],
                    &tol
                )
                .is_err(),
                "{what}"
            );
            let law = law_towards(&body, lower, frame.z(), 3.0 * k, Fade::tip(15.0 * k, 0.0));
            let made = fillet_by_law(&body, &[FilletLawEdge { edge: lower, law }], &tol)
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            sound(&made.body, k, &what);
            assert_eq!(made.body.faces.len(), 7, "{what}");
            let taken = (1.0 - 0.25 * PI) * 9.0 * (10.0 + 15.0 * 9.0 / 14.0);
            let off = (mass(&body).volume - mass(&made.body).volume - taken * k * k * k).abs();
            let slack = 0.5 * PI * 3.0 * 25.0 * k * k * tol.linear;
            assert!(off <= 0.1 * slack, "{what}: the volume is {off:e} off");
            // The upper piece: the same edge.
            let was = body.edge(upper).unwrap();
            let kept = made
                .edges
                .iter()
                .position(|e| *e == Some(upper))
                .unwrap_or_else(|| panic!("{what}: the upper piece is gone"));
            let is = &made.body.edges[kept];
            assert!(was.curve == is.curve && was.range == is.range, "{what}");
            // The tip: a vertex of three faces and three edges now.
            let tip = frame.point(0.0, 0.0, 25.0 * k);
            let there = made
                .body
                .edges
                .iter()
                .filter(|e| {
                    [e.start, e.end]
                        .iter()
                        .any(|v| made.body.vertex(*v).unwrap().point.distance(tip) <= 1e-9 * k)
                })
                .count();
            assert_eq!(there, 3, "{what}");
        }
    }
}

/// A tip at a vertex of four faces, and one at a corner of three: a hip
/// of a square pyramid (two roofs across each other), rounded 1 with a
/// fade to nothing at the apex over 6 and at the base corner over 5. The
/// two faces meet at the angle whose normals turn by `φ`,
/// `cos φ = a² / (a² + h²)`, and the fillet takes
/// `(tan(φ/2) − φ/2) ∫ r²`, with `∫ r² = R² (ℓ − L₁ − L₂) + 9/14 R² (L₁ + L₂)`.
#[test]
fn a_tip_at_the_apex_of_a_pyramid_and_at_its_base() {
    use craft3d_kernel::extrude::extrude;
    use craft3d_kernel::profile::detect_profiles;
    let (a, h) = (10.0_f64, 12.0_f64);
    let turn = (a * a / (a * a + h * h)).pacos();
    let section = (0.5 * turn).ptan() - 0.5 * turn;
    let length = (2.0 * a * a + h * h).sqrt();
    let taken = section * ((length - 11.0) + 9.0 / 14.0 * 11.0);
    for k in SCALES {
        let tol = tolerance(k);
        let roof =
            common::scale_curves(&common::polygon(1, 1, &[[-a, 0.0], [a, 0.0], [0.0, h]]), k);
        let profiles = detect_profiles(&roof, &tol).unwrap();
        let along_x = Frame::new(Vec3::new(-20.0 * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
        let along_y = Frame::new(Vec3::new(0.0, -20.0 * k, 0.0), Vec3::Y, -Vec3::X).unwrap();
        let one = extrude(&profiles, &along_x, 0.0, 40.0 * k, &tol)
            .unwrap()
            .body;
        let other = extrude(&profiles, &along_y, 0.0, 40.0 * k, &tol)
            .unwrap()
            .body;
        let pyramid = boolean(&one, &other, BooleanOp::Intersect, &tol)
            .unwrap_or_else(|e| panic!("scale {k}: the pyramid: {e}"))
            .body;
        assert_eq!(check(&pyramid, &tol), [], "scale {k}");
        assert_eq!(
            (
                pyramid.faces.len(),
                pyramid.edges.len(),
                pyramid.vertices.len()
            ),
            (5, 8, 5),
            "scale {k}"
        );
        let hip = edge_at(&pyramid, &Frame::WORLD, k, [5.0, 5.0, 6.0]);
        let data = pyramid.edge(hip).unwrap();
        let up = data.curve.eval(data.range.1).z > data.curve.eval(data.range.0).z;
        let (apex, base) = (Some(Fade::tip(6.0 * k, 0.0)), Some(Fade::tip(5.0 * k, 0.0)));
        let law = RadiusLaw {
            radius: k,
            fades: if up { [base, apex] } else { [apex, base] },
        };
        let what = format!("scale {k}");
        let made = fillet_by_law(&pyramid, &[FilletLawEdge { edge: hip, law }], &tol)
            .unwrap_or_else(|e| panic!("{what}: {e}"));
        sound(&made.body, k, &what);
        // One more face, with two sides; one more edge; no more vertices.
        assert_eq!(
            (
                made.body.faces.len(),
                made.body.edges.len(),
                made.body.vertices.len()
            ),
            (6, 9, 5),
            "{what}"
        );
        let face = made.body.face(fillet_face(&made)).unwrap();
        let sides: usize = face
            .loops
            .iter()
            .map(|l| made.body.loop_(*l).unwrap().coedges.len())
            .sum();
        assert_eq!(sides, 2, "{what}");
        let off = (mass(&pyramid).volume - mass(&made.body).volume - taken * k * k * k).abs();
        let slack = PI * length * k * k * tol.linear;
        println!("{what}: the volume is {:.3} of the slack off", off / slack);
        assert!(off <= 0.1 * slack, "{what}: the volume is {off:e} off");
        // Four faces and four edges at the apex before; five and five now.
        let apex = Vec3::new(0.0, 0.0, h * k);
        let edges_at = |body: &Body| {
            body.edges
                .iter()
                .filter(|e| {
                    [e.start, e.end]
                        .iter()
                        .any(|v| body.vertex(*v).unwrap().point.distance(apex) <= 1e-9 * k)
                })
                .count()
        };
        assert_eq!((edges_at(&pyramid), edges_at(&made.body)), (4, 5), "{what}");
    }
}

/// What is not a tip, or not one that can be built, is an error and never
/// a body.
#[test]
fn a_tip_that_cannot_be_is_refused() {
    use craft3d_kernel::KernelError;
    let k = 1.0;
    let tol = tolerance(k);
    let frame = Frame::WORLD;
    let body = block(&frame, k, [0.0, 0.0, 0.0], [40.0, 30.0, 40.0]);
    let edge = edge_at(&body, &frame, k, [0.0, 0.0, 20.0]);
    let try_fade = |fade: Fade| {
        let law = law_towards(&body, edge, frame.z(), 3.0, fade);
        fillet_by_law(&body, &[FilletLawEdge { edge, law }], &tol)
    };
    // Short of the end only a fade to nothing ends; by a length.
    let not_a_tip = Fade {
        to: 1.0,
        over: 15.0,
        back: 2.0,
    };
    let error = try_fade(not_a_tip).unwrap_err();
    assert!(matches!(
        error,
        KernelError::FilletFade {
            fault: craft3d_kernel::fillet::FadeFault::BackNotATip { .. },
            ..
        }
    ));
    assert!(
        error
            .to_string()
            .contains("2 short of the end of the edge, but it runs to a radius of 1"),
        "{error}"
    );
    // The smallest back there is: said as it is, not as nothing.
    let error = try_fade(Fade::tip(15.0, 5e-324)).unwrap_err();
    assert!(error.to_string().contains("5e-324 short"), "{error}");
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            try_fade(Fade::tip(15.0, bad)),
            Err(KernelError::InvalidSize { .. })
        ));
        assert!(matches!(
            try_fade(Fade::tip(bad, 0.0)),
            Err(KernelError::InvalidSize { .. })
        ));
    }
    // The fade and what it stops short by, longer than the edge.
    for (over, back) in [(40.001, 0.0), (15.0, 25.001), (30.0, 10.001)] {
        match try_fade(Fade::tip(over, back)) {
            Err(KernelError::Fillet { reason, .. }) => {
                assert!(reason.contains("longer than the edge"), "{reason}");
            }
            other => panic!("{over} {back}: {other:?}"),
        }
    }
    // The whole edge: from a point at the top to nothing cut at the
    // bottom but the radius it has come to there.
    assert!(try_fade(Fade::tip(40.0, 0.0)).is_ok());
    assert!(try_fade(Fade::tip(15.0, 25.0)).is_ok());
    // Short of the end by less than the borders can be told from the
    // edges at the end: refused; by nothing, or by more: built.
    assert!(try_fade(Fade::tip(15.0, 5.0 * tol.linear)).is_err());
    assert!(try_fade(Fade::tip(15.0, 0.0)).is_ok());
    assert!(try_fade(Fade::tip(15.0, 20.0 * tol.linear)).is_ok());
}

/// The nearest edge of `body` to `target`.
fn edge_through(body: &Body, target: Vec3) -> EdgeId {
    let mut best = (EdgeId(0), f64::INFINITY);
    for (i, e) in body.edges.iter().enumerate() {
        for j in 0..=256 {
            let t = e.range.0 + (e.range.1 - e.range.0) * f64::from(j) / 256.0;
            let d = e.curve.eval(t).distance(target);
            if d < best.1 {
                best = (EdgeId(i as u32), d);
            }
        }
    }
    best.0
}

/// The review's third finding: on the curved edge where a plane cuts a
/// cone (radius 10, height 30, cut at z = 15 and by the plane y = 2.5)
/// every tip was refused as not followed, though one radius and fades to
/// a radius build there. The face was followed too far past the edge's
/// other end (steps doubling without a cap); with the cap, tips at either
/// end, at the vertex and short of it, are built.
#[test]
fn a_tip_on_the_cones_curved_edge_is_built() {
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in common::frames(k).iter().enumerate() {
            let cone = primitives::cone(frame, 10.0 * k, 30.0 * k).unwrap();
            let cut = primitives::block(
                &frame.at(frame.point(-20.0 * k, 2.5 * k, 0.0)).unwrap(),
                40.0 * k,
                30.0 * k,
                15.0 * k,
            )
            .unwrap();
            let body = boolean(&cone, &cut, BooleanOp::Intersect, &tol)
                .unwrap()
                .body;
            let x = (56.25_f64 - 6.25).sqrt();
            let edge = edge_through(&body, frame.point(x * k, 2.5 * k, 7.5 * k));
            for (end, over, back) in [(1, 6.0, 0.0), (1, 6.0, 3.0), (0, 6.0, 0.0), (0, 6.0, 3.0)] {
                let what = format!("end {end}, over {over}, back {back}, scale {k}, frame {index}");
                let mut fades = [None, None];
                fades[end] = Some(Fade::tip(over * k, back * k));
                let law = RadiusLaw {
                    radius: 1.2 * k,
                    fades,
                };
                let made = fillet_by_law(&body, &[FilletLawEdge { edge, law }], &tol)
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
                sound(&made.body, k, &what);
            }
        }
    }
}

/// A prism whose one wall is a spline that turns a corner tightly: its
/// rim runs along x, turns, and runs down y.
fn bent_rim(frame: &Frame, k: f64) -> (Body, EdgeId) {
    use craft3d_kernel::extrude::extrude;
    use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    let tol = tolerance(k);
    let scaled = |p: [f64; 2]| [p[0] * k, p[1] * k];
    let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line {
            start: scaled(a),
            end: scaled(b),
        },
    };
    let control = [
        [40.0, 10.0],
        [28.0, 10.0],
        [14.0, 10.0],
        [10.0, 10.0],
        [10.0, 6.0],
        [10.0, -10.0],
    ];
    let curves = [
        SketchCurve {
            id: 1,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0],
                control: control.iter().map(|p| scaled(*p)).collect(),
            }),
        },
        line(2, [10.0, -10.0], [40.0, -10.0]),
        line(3, [40.0, -10.0], [40.0, 10.0]),
    ];
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let body = extrude(&profiles, frame, 0.0, 8.0 * k, &tol).unwrap().body;
    let rim = edge_through(&body, frame.point(11.5 * k, 8.5 * k, 8.0 * k));
    (body, rim)
}

/// The review's fourth finding, on a small body: where the edge turns
/// more tightly than the ball's radius there (the rim's least radius of
/// curvature is 1.57), the face turns back on itself: its sections run
/// into each other. On a hull's stem such fades came out folded and were
/// refused by the post-check in the wording of a bug, or were built
/// folded; here the face was "not approximated". Refused now before the
/// face is put in, with the place and the radius there, and what the
/// refusal says would build is built.
#[test]
fn a_radius_too_large_where_the_edge_turns_is_refused_with_the_place() {
    use craft3d_kernel::KernelError;
    use craft3d_kernel::fillet::FadeFault;
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in common::frames(k).iter().enumerate() {
            let what = format!("scale {k}, frame {index}");
            let (body, rim) = bent_rim(frame, k);
            let data = body.edge(rim).unwrap();
            // The rim's far end is the one down y.
            let far_is_end = frame.local(data.curve.eval(data.range.1)).1
                < frame.local(data.curve.eval(data.range.0)).1;
            let try_tip = |radius: f64, over: f64, back: f64| {
                let fade = Some(Fade::tip(over * k, back * k));
                let law = RadiusLaw {
                    radius: radius * k,
                    fades: if far_is_end {
                        [None, fade]
                    } else {
                        [fade, None]
                    },
                };
                fillet_by_law(&body, &[FilletLawEdge { edge: rim, law }], &tol)
            };
            // 4 all the way to the turn, to nothing over 5 just past it.
            let corner = frame.point(10.0 * k, 10.0 * k, 8.0 * k);
            match try_tip(4.0, 5.0, 12.0) {
                Err(KernelError::FilletFade {
                    edge,
                    fault: FadeFault::TurnsBack { at, radius },
                }) => {
                    assert_eq!(edge, rim, "{what}");
                    assert!(at.distance(corner) <= 3.0 * k, "{what}: at {at:?}");
                    assert!(radius > 1.57 * k && radius <= 4.001 * k, "{what}: {radius}");
                }
                Err(other) => panic!("{what}: refused otherwise: {other}"),
                Ok(_) => panic!("{what}: built"),
            }
            let text = try_tip(4.0, 5.0, 12.0).unwrap_err().to_string();
            assert!(text.contains("turn back on itself at ("), "{text}");
            // What the refusal says would build: a smaller radius (the
            // rim's least radius of curvature is 1.57) with the same tip,
            // and the same radius ending before the turn.
            for (radius, back) in [(1.5, 12.0), (4.0, 28.0)] {
                let made = try_tip(radius, 5.0, back)
                    .unwrap_or_else(|e| panic!("{what}, {radius} back {back}: {e}"));
                sound(&made.body, k, &what);
            }
        }
    }
}
