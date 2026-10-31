//! The tessellator has no size of its own: a body and its chordal
//! tolerance made `k` times as large give the same triangulation, `k`
//! times as large. Nothing in it may be a length (an epsilon in model
//! units, a "small" parameter step), only ratios.
//!
//! Two kinds of `k`:
//!
//! - **Powers of two.** Multiplying by one is exact, so the fixture at
//!   size `k` is the fixture at size 1 with every length scaled without
//!   rounding, and the mesh must be too: the same triangles, and every
//!   position *bit for bit* `k` times the position at size 1. One
//!   constant anywhere that is a length breaks this.
//! - **Powers of ten** (1e-6 to 1e6). Here the fixture itself differs
//!   from the scaled one in the last places. The points of the edges do
//!   not depend on that: as many per edge, in the same places to
//!   rounding. The triangles between them do, where two triangulations
//!   are equally good: four points of a grid on a cylinder lie on one
//!   circle, and which diagonal joins them is decided by the last bit
//!   (and, after that, which triangles are measured and refined). So
//!   the meshes are compared as surfaces: area and volume within what
//!   the chordal tolerance allows, and about as many triangles.
//!
//! And one face built by hand for what broke this once: an edge on a
//! spline whose range starts a rounding error before a knot.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, Line, NurbsCurve, Plane, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body, tessellate_edges};
use craft3d_testkit::solids::{self, Reference};
use craft3d_testkit::trimmed;
use craft3d_topo::{Body, BodyKind, Coedge};

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    }
}

/// The world frame, and one that is square to nothing. Its origin grows
/// with the model; its directions do not.
fn frames(scale: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap(),
    ]
}

fn fixtures(frame: &Frame, scale: f64) -> Vec<Reference> {
    let mut all = solids::all(frame, scale).unwrap();
    all.extend(trimmed::all(frame, scale).unwrap());
    all.push(trimmed::pipe_elbow(frame, 3.0 * scale, 1.0 * scale, 1.9).unwrap());
    all.push(trimmed::sphere_octant(frame, 2.0 * scale).unwrap());
    all
}

/// One fixture in one frame at one chord.
struct Case {
    what: String,
    mesh: Mesh,
    edges: Vec<Vec<Vec3>>,
}

/// Every fixture in both frames at chords of 1/100 and 1/1000 of `scale`.
fn cases(scale: f64) -> Vec<Case> {
    let mut all = Vec::new();
    for (f, frame) in frames(scale).iter().enumerate() {
        for solid in fixtures(frame, scale) {
            for relative in [1e-2, 1e-3] {
                let what = format!("{} in frame {f} at chord {relative}", solid.name);
                let params = params(relative * scale);
                let mesh = tessellate_body(&solid.body, &params)
                    .unwrap_or_else(|e| panic!("{what}, scale {scale}: {e}"));
                let edges = tessellate_edges(&solid.body, &params).unwrap();
                all.push(Case { what, mesh, edges });
            }
        }
    }
    all
}

fn shrunk(p: Vec3, scale: f64) -> Vec3 {
    Vec3::new(p.x / scale, p.y / scale, p.z / scale)
}

/// The volume a closed mesh encloses.
fn volume(mesh: &Mesh) -> f64 {
    (0..mesh.triangles.len())
        .filter_map(|t| mesh.corners(t))
        .map(|[a, b, c]| a.dot(b.cross(c)) / 6.0)
        .sum()
}

#[test]
fn a_power_of_two_times_the_body_is_bit_for_bit_that_times_the_mesh() {
    let unit = cases(1.0);
    for exponent in [-40, -20, -10, 10, 20, 40] {
        let scale = 2.0_f64.ppowi(exponent);
        let scaled = cases(scale);
        assert_eq!(scaled.len(), unit.len());
        for (case, reference) in scaled.iter().zip(&unit) {
            let what = format!("{}, scale 2^{exponent}", case.what);
            let (mesh, unit) = (&case.mesh, &reference.mesh);
            assert_eq!(mesh.triangles, unit.triangles, "{what}");
            assert_eq!(mesh.face_ids, unit.face_ids, "{what}");
            assert_eq!(mesh.normals, unit.normals, "{what}");
            assert_eq!(mesh.positions.len(), unit.positions.len(), "{what}");
            for (n, (p, q)) in mesh.positions.iter().zip(&unit.positions).enumerate() {
                assert!(
                    *p == *q * scale,
                    "{what}: vertex {n} is {p:?}, not {:?}",
                    *q * scale
                );
            }
            let scaled_edges: Vec<Vec<Vec3>> = reference
                .edges
                .iter()
                .map(|edge| edge.iter().map(|p| *p * scale).collect())
                .collect();
            assert!(case.edges == scaled_edges, "{what}: the edges");
        }
    }
}

#[test]
fn a_power_of_ten_times_the_body_has_the_same_edges_and_the_same_surface() {
    let unit = cases(1.0);
    let (mut worst, mut same, mut total) = (0.0_f64, 0, 0);
    for scale in [1e-6, 1e-3, 1e3, 1e6] {
        let scaled = cases(scale);
        assert_eq!(scaled.len(), unit.len());
        for (case, reference) in scaled.iter().zip(&unit) {
            let what = format!("{}, scale {scale}", case.what);
            // The edges: point for point.
            assert_eq!(case.edges.len(), reference.edges.len(), "{what}");
            for (n, (edge, unit)) in case.edges.iter().zip(&reference.edges).enumerate() {
                assert_eq!(edge.len(), unit.len(), "{what}: points of edge {n}");
                for (p, q) in edge.iter().zip(unit) {
                    let apart = shrunk(*p, scale).distance(*q);
                    // Lengths of a few units, a few dozen operations each:
                    // rounding is some 1e-15.
                    assert!(
                        apart <= 1e-12,
                        "{what}: a point of edge {n} is {apart:e} off"
                    );
                    worst = worst.max(apart);
                }
            }
            // The mesh: the same surface. Two meshes within the chordal
            // tolerance of one surface differ in area and volume by no
            // more than that tolerance over the size of the body, here
            // the chord as a fraction of the scale.
            let (mesh, unit) = (&case.mesh, &reference.mesh);
            let chord = if case.what.ends_with("0.01") {
                1e-2
            } else {
                1e-3
            };
            let area = mesh.area() / (scale * scale);
            let inside = volume(mesh) / (scale * scale * scale);
            assert!(
                (area - unit.area()).abs() <= chord * unit.area(),
                "{what}: area {area} against {}",
                unit.area()
            );
            assert!(
                (inside - volume(unit)).abs() <= chord * volume(unit),
                "{what}: volume {inside} against {}",
                volume(unit)
            );
            let (count, expected) = (mesh.triangles.len(), unit.triangles.len());
            assert!(
                count.abs_diff(expected) * 10 <= expected,
                "{what}: {count} triangles against {expected}"
            );
            total += 1;
            same += usize::from(count == expected);
        }
    }
    println!(
        "edge points differ from those at scale 1 by at most {worst:e}; \
         {same} of {total} meshes have as many triangles as at scale 1"
    );
}

/// A flat face with three straight sides and one on a cubic spline with
/// several knot spans, bulging outwards. The spline edge's range is
/// `range` of the curve; the vertices are where that puts them.
fn sail(scale: f64, range: (f64, f64), knots: &[f64]) -> Body {
    // A cubic through evenly spread control points: a gentle arc from
    // (0, 0) to (4, 0) below the x axis, in the plane z = 0.
    let n = knots.len() - 4;
    let control: Vec<Vec3> = (0..n)
        .map(|i| {
            let s = i as f64 / (n - 1) as f64;
            Vec3::new(4.0 * s, -1.5 * (s * (1.0 - s)), 0.0) * scale
        })
        .collect();
    let weights = vec![1.0; n];
    let curve = NurbsCurve::new(3, knots.to_vec(), control, weights).unwrap();
    let curve = Curve3::Nurbs(curve);
    let (start, end) = (curve.eval(range.0), curve.eval(range.1));
    let corners = [
        start,
        end,
        Vec3::new(end.x, 2.0 * scale, 0.0),
        Vec3::new(start.x, 2.0 * scale, 0.0),
    ];
    let mut body = Body::new(BodyKind::Sheet);
    let ids: Vec<_> = corners.iter().map(|p| body.add_vertex(*p)).collect();
    let mut coedges = vec![Coedge {
        edge: body.add_edge(curve, range, ids[0], ids[1]),
        reversed: false,
    }];
    for k in 1..4 {
        let (from, to) = (corners[k], corners[(k + 1) % 4]);
        let line = Curve3::Line(Line::new(from, to - from).unwrap());
        coedges.push(Coedge {
            edge: body.add_edge(line, (0.0, from.distance(to)), ids[k], ids[(k + 1) % 4]),
            reversed: false,
        });
    }
    let outer = body.add_loop(coedges);
    let face = body.add_face(Surface::Plane(Plane::new(Frame::WORLD)), false, vec![outer]);
    body.add_shell(vec![face]);
    body
}

/// What the boolean of seed 20 (grid placement, scale 0.001) came to: an
/// edge cut from a spline at a knot, with the start of its range a few
/// units in the last place before that knot. The knot is then inside the
/// range, and was taken for the end of a piece of the edge: a piece some
/// 1e-19 long, two points of the boundary that the triangulation could
/// not tell apart ("its boundary passes twice through one place"). At
/// scale 1 rounding happened to put the start on the knot.
#[test]
fn a_knot_a_rounding_error_inside_an_edges_range_is_not_a_point_of_the_edge() {
    let knots = [0.0, 0.0, 0.0, 0.0, 0.2, 0.4, 0.6, 0.8, 1.0, 1.0, 1.0, 1.0];
    for scale in [1e-6, 1e-3, 1.0, 1e3, 1e6] {
        let knots: Vec<f64> = knots.iter().map(|k| k * scale).collect();
        let knot = |i: usize| knots[i];
        let before = |t: f64| f64::from_bits(t.to_bits() - 4);
        let after = |t: f64| f64::from_bits(t.to_bits() + 4);
        // The range from one knot to another, and the same with its ends
        // moved by four units in the last place: inwards (nothing
        // changes), and outwards (a knot just inside either end).
        let exact = (knot(4), knot(7));
        let ranges = [
            ("on the knots", exact),
            ("a hair inside", (after(exact.0), before(exact.1))),
            ("a hair outside", (before(exact.0), after(exact.1))),
            ("start a hair before", (before(exact.0), exact.1)),
            ("end a hair after", (exact.0, after(exact.1))),
        ];
        let tolerance = 1e-3 * scale;
        let reference = tessellate_body(&sail(scale, exact, &knots), &params(tolerance)).unwrap();
        for (name, range) in ranges {
            let what = format!("scale {scale}, range {name}");
            let body = sail(scale, range, &knots);
            let edges = tessellate_edges(&body, &params(tolerance)).unwrap();
            // No two points of the spline edge nearer than a millionth of
            // the chord the tolerance asks for.
            let spline = &edges[0];
            let shortest = spline
                .windows(2)
                .map(|w| w[0].distance(w[1]))
                .fold(f64::INFINITY, f64::min);
            assert!(
                shortest > 1e-3 * scale,
                "{what}: the spline edge has two points {shortest:e} apart"
            );
            let mesh = tessellate_body(&body, &params(tolerance))
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(mesh.triangles, reference.triangles, "{what}");
            let apart = mesh
                .positions
                .iter()
                .zip(&reference.positions)
                .map(|(p, q)| p.distance(*q))
                .fold(0.0, f64::max);
            assert!(apart <= 1e-12 * scale, "{what}: moved by {apart:e}");
        }
    }
}
