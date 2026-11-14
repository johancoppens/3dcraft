//! A chamfer is the face that joins two cut lines, each its distance from
//! the edge along its face: it cuts a convex corner off and fills a
//! concave one. Where the line across the edge from one cut line to the
//! other would pass through a face beside the edge (a pipe lying in a
//! plate, chamfered far along the pipe: the line runs through the pipe),
//! there is no chamfer of that size, and it is refused as too large. What
//! was built there until 2026-11-13 was a solid that passed every check
//! and was not the chamfer: material taken from the pipe at a concave
//! edge, material added to a groove at a convex one.
//!
//! The oracles are the sections across the edge worked out by hand in 2D
//! (docs/design/blends.md › *A chamfer through a face beside it*): what a
//! chamfer adds or takes is the triangle of the edge and its two feet,
//! less the circular segment of the pipe between the edge and the foot on
//! it (a Bézier hump's by Gauss-Legendre, exact for its polynomial); the
//! size past which the line enters the pipe is where the lens it cuts
//! from the pipe, `R (1 - cos α)` (α the angle between the line and the
//! pipe's tangent at the foot on it), grows past the tolerance: short of
//! that the line is within the tolerance of one that stays clear, and the
//! chamfer builds.
//! Found by bisection on that depth. At the scales 1e-3, 1 and 1e6, in the
//! world and in a tilted frame away from the origin.
//!
//! The plate is 60 x 40 x 10 (x along the pipe, y across, z up, in the
//! frame of the test); the pipe has radius 10 and its axis at
//! (y, z) = (20, 16). United with the plate it rises 16 above the top
//! between y = 12 and y = 28, at concave edges; taken out of the plate it
//! leaves a groove 4 deep, whose rims are convex.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod boolean_common;
mod common;

use boolean_common::{SCALES, assert_close, mass, tolerance};
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::probe::{OnFace, Probe};
use craft3d_kernel::boolean::{BooleanOp, Place, boolean, classify_points};
use craft3d_kernel::chamfer::{ChamferEdge, ChamferFace, ChamferSize, chamfer};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

type P = [f64; 2];

/// The pipe's radius and axis, and the high edge where it leaves the
/// top, in the section (y, z).
const R: f64 = 10.0;
const C: P = [20.0, 16.0];
const E: P = [28.0, 10.0];

/// The world and two frames away from the origin, one of them tilted.
fn frames(k: f64) -> Vec<Frame> {
    let mut all = common::frames(k);
    all.push(
        Frame::new(
            Vec3::new(400.0 * k, -300.0 * k, 250.0 * k),
            Vec3::new(-2.0, 1.0, 2.0),
            Vec3::new(0.0, 1.0, -0.5),
        )
        .unwrap(),
    );
    all
}

fn triangle(p: P, q: P, r: P) -> f64 {
    0.5 * ((q[0] - p[0]) * (r[1] - p[1]) - (r[0] - p[0]) * (q[1] - p[1])).abs()
}

/// The foot on the pipe `b` of arc from the edge: up its wall (the pipe
/// on the plate) or down it (the groove).
fn on_pipe(b: f64, up: bool) -> P {
    let start = (E[1] - C[1]).patan2(E[0] - C[0]);
    let t = start + if up { b / R } else { -b / R };
    [C[0] + R * t.pcos(), C[1] + R * t.psin()]
}

/// What the chamfer `a` along the top and `b` along the pipe adds (the
/// pipe) or takes (the groove) per unit of length: the triangle of the
/// edge and the two feet less the segment of the pipe's circle between
/// the edge and the foot on it, which lies inside the triangle.
fn section_area(a: f64, b: f64, up: bool) -> f64 {
    let phi = b / R;
    triangle(E, [E[0] + a, E[1]], on_pipe(b, up)) - 0.5 * R * R * (phi - phi.psin())
}

/// `(G1 - G2) . (G2 - C)`, G1 the foot on the top and G2 the one on the
/// pipe: positive while the line from G2 to G1 leaves the pipe's circle
/// outward, so it meets the circle nowhere else between them; zero where
/// it is tangent there.
fn clearance(a: f64, b: f64, up: bool) -> f64 {
    let (g1, g2) = ([E[0] + a, E[1]], on_pipe(b, up));
    (g1[0] - g2[0]) * (g2[0] - C[0]) + (g1[1] - g2[1]) * (g2[1] - C[1])
}

/// How deep the lens is that the line from G2 to G1 cuts from the pipe's
/// circle where it enters it at G2 (scale 1): `R (1 - cos α)`, the sine of
/// α the clearance over the line's length and the radius; none where it
/// leaves the circle outward.
fn lens(a: f64, b: f64, up: bool) -> f64 {
    let (g1, g2) = ([E[0] + a, E[1]], on_pipe(b, up));
    let sine = clearance(a, b, up) / ((g1[0] - g2[0]).phypot(g1[1] - g2[1]) * R);
    if sine >= 0.0 {
        0.0
    } else {
        R * (1.0 - (1.0 - sine * sine).sqrt())
    }
}

/// The tolerance at scale 1: a lens no deeper is within the tolerance of
/// none.
const DEEP: f64 = 1e-6;

/// The scale of the size `(a, b)` at which its line enters the pipe by a
/// lens deeper than [`DEEP`]: bisection between a scale where it does
/// not and one where it does.
fn touching(a: f64, b: f64, up: bool) -> f64 {
    let (mut lo, mut hi) = (1e-6, 1.0);
    assert!(lens(lo * a, lo * b, up) <= DEEP && lens(a, b, up) > DEEP);
    for _ in 0..200 {
        let middle = 0.5 * (lo + hi);
        if lens(middle * a, middle * b, up) <= DEEP {
            lo = middle;
        } else {
            hi = middle;
        }
    }
    0.5 * (lo + hi)
}

/// The pipe's segment below the top (the union) or the groove's area.
fn dip() -> f64 {
    let t = C[1] - 10.0;
    R * R * (t / R).pacos() - t * (R * R - t * t).sqrt()
}

/// The plate and the pipe, united or the pipe taken out, built from a
/// block and a cylinder in `frame` at scale `k`.
fn plate_and_pipe(frame: &Frame, k: f64, op: BooleanOp) -> Body {
    let plate = primitives::block(frame, 60.0 * k, 40.0 * k, 10.0 * k).unwrap();
    let axis = Frame::new(frame.point(0.0, C[0] * k, C[1] * k), frame.x(), frame.y()).unwrap();
    let pipe = primitives::cylinder(&axis, R * k, 60.0 * k).unwrap();
    boolean(&plate, &pipe, op, &tolerance(k)).unwrap().body
}

/// The edge whose middle is nearest `point`.
fn edge_near(body: &Body, point: Vec3) -> EdgeId {
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(point),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(index as u32)
}

fn faces_of(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    (0..body.faces.len())
        .filter(|i| {
            body.faces[*i]
                .loops
                .iter()
                .flat_map(|l| &body.loop_(*l).unwrap().coedges)
                .any(|c| c.edge == edge)
        })
        .map(|i| FaceId(i as u32))
        .collect()
}

/// The edge where the pipe leaves the top at `y` (12 or 28, in the
/// section), with the face beside it that is not a plane first: the size
/// is `(b, a)`, `b` along the pipe and `a` along the top.
fn pipe_edge(body: &Body, frame: &Frame, k: f64, y: f64, a: f64, b: f64) -> (ChamferEdge, FaceId) {
    let edge = edge_near(body, frame.point(30.0 * k, y * k, 10.0 * k));
    let pipe = faces_of(body, edge)
        .into_iter()
        .find(|f| !matches!(body.faces[f.0 as usize].surface, Surface::Plane(_)))
        .unwrap();
    (
        ChamferEdge {
            edge,
            first: pipe,
            size: ChamferSize::Distances {
                first: b * k,
                second: a * k,
            },
        },
        pipe,
    )
}

/// The chamfer of the pipe's edges at the scale of the size `s`: both,
/// or the high one alone.
fn bevel(body: &Body, frame: &Frame, k: f64, a: f64, b: f64, both: bool) -> Vec<ChamferEdge> {
    let mut edges = vec![pipe_edge(body, frame, k, E[0], a, b).0];
    if both {
        edges.push(pipe_edge(body, frame, k, 40.0 - E[0], a, b).0);
    }
    edges
}

#[test]
fn a_chamfer_beside_a_pipe_is_the_section_until_its_line_enters_the_pipe() {
    let a = 1.0;
    let v0 = 24000.0 + 60.0 * (PI * R * R - dip());
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let body = plate_and_pipe(&frame, k, BooleanOp::Union);
            assert_close(mass(&body).volume, v0 * k * k * k, "the union");
            // Up to 4.4657 of arc along the pipe its line stays outside
            // it, and up to 4.4702 it enters it by a lens no deeper than the
            // tolerance (4.467: 8.2e-8, 4.469: 5.4e-7): the chamfer adds the
            // section, on one edge or both, as 0.1.0 built it.
            for (b, both) in [(2.0, false), (4.0, true), (4.467, false), (4.469, false)] {
                assert!(lens(a, b, true) <= DEEP);
                let made = chamfer(&body, &bevel(&body, &frame, k, a, b, both), &tol)
                    .unwrap_or_else(|e| panic!("{b} at {k}: {e}"));
                assert_eq!(check(&made.body, &tol), [], "{b} at {k}");
                let edges = if both { 2.0 } else { 1.0 };
                let expected = (v0 + edges * 60.0 * section_area(a, b, true)) * k * k * k;
                let got = mass(&made.body).volume;
                assert_close(got, expected, "volume");
                assert!(got > v0 * k * k * k, "a concave chamfer adds");
            }
            // Past it no chamfer of the size exists: too large for the
            // pipe, and what fits is the size whose lens is the tolerance
            // deep, as a whole (both distances scaled together).
            for b in [4.6, 8.0, 30.0] {
                assert!(lens(a, b, true) > DEEP);
                let (edge, pipe) = pipe_edge(&body, &frame, k, E[0], a, b);
                let error = chamfer(&body, &[edge], &tol).unwrap_err();
                let KernelError::ChamferTooLarge {
                    face,
                    distance,
                    largest,
                    ..
                } = error
                else {
                    panic!("{b} at {k}: {error}");
                };
                assert_eq!(face, pipe, "{b} at {k}");
                assert_close(distance, b * k, "the distance said");
                let at = touching(a, b, true);
                assert!(
                    (largest - at * b * k).abs() <= 1e-4 * at * b * k,
                    "{b} at {k}: {largest} for {}",
                    at * b * k
                );
                // Just below it the chamfer is built and is the section;
                // just past it, refused.
                let s = 0.99 * largest / (b * k);
                let made = chamfer(&body, &bevel(&body, &frame, k, a * s, b * s, false), &tol)
                    .unwrap_or_else(|e| panic!("{b} scaled by {s} at {k}: {e}"));
                assert_eq!(check(&made.body, &tol), []);
                let expected = (v0 + 60.0 * section_area(a * s, b * s, true)) * k * k * k;
                assert_close(mass(&made.body).volume, expected, "volume at what fits");
                let s = 1.01 * largest / (b * k);
                let past = chamfer(&body, &bevel(&body, &frame, k, a * s, b * s, false), &tol);
                assert!(
                    matches!(past, Err(KernelError::ChamferTooLarge { .. })),
                    "{past:?}"
                );
            }
        }
    }
}

#[test]
fn a_chamfer_along_a_groove_is_the_section_until_its_line_leaves_the_material() {
    // The rims of the groove are convex: the chamfer takes the section,
    // until its line runs out of the material into the groove (3.35575 of
    // arc down the groove with 1 along the top) by a lens deeper than the
    // tolerance (3.36023). Past it what was built added
    // material to the groove: 8 gave 21576.95 against the grooved plate's
    // 21316.23 (2026-11-13).
    let a = 1.0;
    let v0 = 24000.0 - 60.0 * dip();
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let body = plate_and_pipe(&frame, k, BooleanOp::Subtract);
            assert_close(mass(&body).volume, v0 * k * k * k, "the groove");
            for (b, both) in [(1.0, false), (2.0, true), (3.357, false), (3.359, false)] {
                assert!(lens(a, b, false) <= DEEP);
                let made = chamfer(&body, &bevel(&body, &frame, k, a, b, both), &tol)
                    .unwrap_or_else(|e| panic!("{b} at {k}: {e}"));
                assert_eq!(check(&made.body, &tol), [], "{b} at {k}");
                let edges = if both { 2.0 } else { 1.0 };
                let expected = (v0 - edges * 60.0 * section_area(a, b, false)) * k * k * k;
                let got = mass(&made.body).volume;
                assert_close(got, expected, "volume");
                assert!(got < v0 * k * k * k, "a convex chamfer takes");
            }
            for b in [3.4, 5.0, 8.0] {
                assert!(lens(a, b, false) > DEEP);
                let (edge, pipe) = pipe_edge(&body, &frame, k, E[0], a, b);
                let error = chamfer(&body, &[edge], &tol).unwrap_err();
                let KernelError::ChamferTooLarge { face, largest, .. } = error else {
                    panic!("{b} at {k}: {error}");
                };
                assert_eq!(face, pipe, "{b} at {k}");
                let at = touching(a, b, false);
                assert!(
                    (largest - at * b * k).abs() <= 1e-4 * at * b * k,
                    "{b} at {k}: {largest} for {}",
                    at * b * k
                );
                let s = 0.99 * largest / (b * k);
                let made = chamfer(&body, &bevel(&body, &frame, k, a * s, b * s, false), &tol)
                    .unwrap_or_else(|e| panic!("{b} scaled by {s} at {k}: {e}"));
                let expected = (v0 - 60.0 * section_area(a * s, b * s, false)) * k * k * k;
                assert_close(mass(&made.body).volume, expected, "volume at what fits");
            }
        }
    }
}

/// A cubic Bézier in the section.
struct Bez([P; 4]);

impl Bez {
    fn at(&self, u: f64) -> P {
        let w = [
            (1.0 - u) * (1.0 - u) * (1.0 - u),
            3.0 * u * (1.0 - u) * (1.0 - u),
            3.0 * u * u * (1.0 - u),
            u * u * u,
        ];
        let p = &self.0;
        [
            w[0] * p[0][0] + w[1] * p[1][0] + w[2] * p[2][0] + w[3] * p[3][0],
            w[0] * p[0][1] + w[1] * p[1][1] + w[2] * p[2][1] + w[3] * p[3][1],
        ]
    }

    fn d(&self, u: f64) -> P {
        let w = [
            -3.0 * (1.0 - u) * (1.0 - u),
            3.0 * (1.0 - u) * (1.0 - u) - 6.0 * u * (1.0 - u),
            6.0 * u * (1.0 - u) - 3.0 * u * u,
            3.0 * u * u,
        ];
        let p = &self.0;
        [
            w[0] * p[0][0] + w[1] * p[1][0] + w[2] * p[2][0] + w[3] * p[3][0],
            w[0] * p[0][1] + w[1] * p[1][1] + w[2] * p[2][1] + w[3] * p[3][1],
        ]
    }

    /// Half the integral of y z' - z y' from `u0` to `u1`: Gauss-Legendre
    /// with five points, exact for the polynomial of degree five.
    fn green(&self, u0: f64, u1: f64) -> f64 {
        const X: [f64; 5] = [
            -0.906_179_845_938_664,
            -0.538_469_310_105_683_1,
            0.0,
            0.538_469_310_105_683_1,
            0.906_179_845_938_664,
        ];
        const W: [f64; 5] = [
            0.236_926_885_056_189_1,
            0.478_628_670_499_366_5,
            0.568_888_888_888_888_9,
            0.478_628_670_499_366_5,
            0.236_926_885_056_189_1,
        ];
        let (m, h) = (0.5 * (u0 + u1), 0.5 * (u1 - u0));
        X.iter()
            .zip(W)
            .map(|(x, w)| {
                let u = m + h * x;
                let (p, q) = (self.at(u), self.d(u));
                w * 0.5 * (p[0] * q[1] - p[1] * q[0])
            })
            .sum::<f64>()
            * h
    }

    /// The parameter of the point `d` from the start, near the start.
    fn at_distance(&self, d: f64) -> f64 {
        let start = self.at(0.0);
        let (mut lo, mut hi) = (0.0, 0.5);
        for _ in 0..200 {
            let middle = 0.5 * (lo + hi);
            let q = self.at(middle);
            if (q[0] - start[0]).phypot(q[1] - start[1]) < d {
                lo = middle;
            } else {
                hi = middle;
            }
        }
        0.5 * (lo + hi)
    }
}

/// A hump on the plate in place of the pipe: a cubic Bézier from the edge
/// at (28, 10) up and over to (12, 10), bulging out over the top like the
/// pipe. A straight edge between a plane and a spline face, which the
/// construction in place chamfers, measuring each distance straight in
/// the plane across the edge.
const HUMP: Bez = Bez([[28.0, 10.0], [34.0, 22.0], [6.0, 22.0], [12.0, 10.0]]);

fn hump_profile(k: f64) -> Vec<SketchCurve> {
    let line = |id: u64, s: P, e: P| SketchCurve {
        id,
        curve: Curve2::Line {
            start: [s[0] * k, s[1] * k],
            end: [e[0] * k, e[1] * k],
        },
    };
    vec![
        line(1, [0.0, 0.0], [40.0, 0.0]),
        line(2, [40.0, 0.0], [40.0, 10.0]),
        line(3, [40.0, 10.0], E),
        SketchCurve {
            id: 4,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: HUMP.0.iter().map(|p| [p[0] * k, p[1] * k]).collect(),
            }),
        },
        line(5, [12.0, 10.0], [0.0, 10.0]),
        line(6, [0.0, 10.0], [0.0, 0.0]),
    ]
}

/// The foot on the hump `b` from the edge (straight), and what the
/// chamfer `a` along the top and `b` adds per unit of length: the region
/// bounded by the top from the edge to its foot, the line to the foot on
/// the hump and the hump back to the edge (Green's theorem: the triangle
/// of the two lines from the edge less the hump's piece).
fn hump_section(a: f64, b: f64) -> (P, f64, f64) {
    let u = HUMP.at_distance(b);
    let (g1, g2) = ([E[0] + a, E[1]], HUMP.at(u));
    let lines = 0.5 * ((E[0] * g1[1] - g1[0] * E[1]) + (g1[0] * g2[1] - g2[0] * g1[1]));
    let added = lines - HUMP.green(0.0, u);
    // The hump's outward normal at G2: the profile runs round it
    // counter-clockwise, so out is to the right of the way it runs.
    let t = HUMP.d(u);
    let n = [t[1], -t[0]];
    let clearance = (g1[0] - g2[0]) * n[0] + (g1[1] - g2[1]) * n[1];
    (g2, added, clearance)
}

#[test]
fn a_chamfer_beside_a_spline_hump_is_refused_where_its_line_enters_the_hump() {
    // The same, with a spline face: its line enters the hump from 3.5098
    // along it (straight, with 1 along the top). Past it what was built in
    // place filled the corner and cut into the hump.
    let a = 1.0;
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let profiles = detect_profiles(&hump_profile(k), &tol).unwrap();
            // The profile's (u, v) are the frame's (y, z); the extrusion
            // runs along the frame's x.
            let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
            let body = extrude(&profiles, &along, 0.0, 60.0 * k, &tol)
                .unwrap()
                .body;
            let v0 = mass(&body).volume;
            for b in [2.0, 3.0] {
                let (_, added, clearance) = hump_section(a, b);
                assert!(clearance > 0.0);
                let made = chamfer(&body, &bevel(&body, &frame, k, a, b, false), &tol)
                    .unwrap_or_else(|e| panic!("{b} at {k}: {e}"));
                assert_eq!(check(&made.body, &tol), [], "{b} at {k}");
                let got = mass(&made.body).volume;
                let expected = v0 + 60.0 * added * k * k * k;
                assert!(
                    (got - expected).abs() <= 1e-7 * expected,
                    "{b} at {k}: {got} for {expected}"
                );
            }
            for b in [4.0, 6.0] {
                let (_, _, clearance) = hump_section(a, b);
                assert!(clearance < 0.0);
                let error =
                    chamfer(&body, &bevel(&body, &frame, k, a, b, false), &tol).unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("pass through a face beside the edge"),
                    "{b} at {k}: {error}"
                );
            }
        }
    }
}

/// An ogee on the plate in place of the pipe: a clamped cubic B-spline
/// from the edge at (28, 10) out over the top and back (its nose overhangs
/// the top between z = 12 and 17), up to z = 24 and down to (12, 10); it
/// bends one way and then the other.
const OGEE: [P; 7] = [
    [28.0, 10.0],
    [31.0, 12.0],
    [31.0, 15.0],
    [26.0, 17.0],
    [27.0, 24.0],
    [12.0, 24.0],
    [12.0, 10.0],
];
const OGEE_KNOTS: [f64; 11] = [0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0];

/// The ogee's point and derivative at `u`, by de Boor's algorithm (and
/// the derivative's, the B-spline of the differences).
fn ogee_at(u: f64) -> (P, P) {
    let span = (3..7)
        .rev()
        .find(|&i| OGEE_KNOTS[i] <= u)
        .unwrap_or(3)
        .min(6);
    let mut d: Vec<P> = (0..4).map(|j| OGEE[span - 3 + j]).collect();
    // The derivative's control points, degree 2.
    let mut q: Vec<P> = (0..3)
        .map(|j| {
            let i = span - 3 + j + 1;
            let h = 3.0 / (OGEE_KNOTS[i + 3] - OGEE_KNOTS[i]);
            [
                h * (OGEE[i][0] - OGEE[i - 1][0]),
                h * (OGEE[i][1] - OGEE[i - 1][1]),
            ]
        })
        .collect();
    for r in 1..=3 {
        for j in (r..=3).rev() {
            let i = span - 3 + j;
            let a = (u - OGEE_KNOTS[i]) / (OGEE_KNOTS[i + 4 - r] - OGEE_KNOTS[i]);
            d[j] = [
                (1.0 - a) * d[j - 1][0] + a * d[j][0],
                (1.0 - a) * d[j - 1][1] + a * d[j][1],
            ];
        }
    }
    for r in 1..=2 {
        for j in (r..=2).rev() {
            let i = span - 3 + j + 1;
            let a = (u - OGEE_KNOTS[i]) / (OGEE_KNOTS[i + 3 - r] - OGEE_KNOTS[i]);
            q[j] = [
                (1.0 - a) * q[j - 1][0] + a * q[j][0],
                (1.0 - a) * q[j - 1][1] + a * q[j][1],
            ];
        }
    }
    (d[3], q[2])
}

fn ogee_profile(k: f64) -> Vec<SketchCurve> {
    let line = |id: u64, s: P, e: P| SketchCurve {
        id,
        curve: Curve2::Line {
            start: [s[0] * k, s[1] * k],
            end: [e[0] * k, e[1] * k],
        },
    };
    vec![
        line(1, [0.0, 0.0], [40.0, 0.0]),
        line(2, [40.0, 0.0], [40.0, 10.0]),
        line(3, [40.0, 10.0], E),
        SketchCurve {
            id: 4,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: OGEE_KNOTS.to_vec(),
                control: OGEE.iter().map(|p| [p[0] * k, p[1] * k]).collect(),
            }),
        },
        line(5, [12.0, 10.0], [0.0, 10.0]),
        line(6, [0.0, 10.0], [0.0, 0.0]),
    ]
}

#[test]
fn a_chamfer_beside_an_ogee_is_refused_where_the_ogee_bends_back_across_its_line() {
    // 8 along the ogee (straight, as the construction in place measures)
    // and 3 along the top: the feet at (31, 10) and (26.958, 17.933), and
    // the chamfer face meets both faces turning as the concave edge does
    // (sines 0.891 and 0.084). Yet the line between the feet cuts the
    // ogee's nose off: (30, 13.5) is inside the nose and beyond the line,
    // and the region the chamfer would add crosses itself. Until
    // 2026-11-13 (after the check at the borders) it was built and lost
    // about 147 mm³; so did 9 and 2, and 8 and 4.
    let line_side = |a: f64, b: f64, p: P| {
        // Which side of the line between the feet `p` is on, the edge's
        // side positive: the foot on the ogee where it first comes the
        // straight distance `b` from the edge (found in steps of 1/1000,
        // then by bisection).
        let away = |u: f64| {
            let q = ogee_at(u).0;
            (q[0] - E[0]).phypot(q[1] - E[1])
        };
        let first = (1..=1000)
            .map(|i| f64::from(i) / 1000.0)
            .find(|u| away(*u) >= b)
            .unwrap();
        let (mut lo, mut hi) = (first - 1e-3, first);
        for _ in 0..200 {
            let m = 0.5 * (lo + hi);
            let q = ogee_at(m).0;
            if (q[0] - E[0]).phypot(q[1] - E[1]) < b {
                lo = m;
            } else {
                hi = m;
            }
        }
        let (g1, g2) = ([E[0] + a, E[1]], ogee_at(0.5 * (lo + hi)).0);
        let side = |q: P| (g2[0] - g1[0]) * (q[1] - g1[1]) - (g2[1] - g1[1]) * (q[0] - g1[0]);
        side(p) * side(E) > 0.0
    };
    for (a, b) in [(3.0, 8.0), (2.0, 9.0), (4.0, 8.0)] {
        // The nose's point lies inside the ogee (left of it as it runs
        // from the edge) and beyond the line.
        assert!(!line_side(a, b, [30.0, 13.5]), "{a} {b}");
    }
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let profiles = detect_profiles(&ogee_profile(k), &tol).unwrap();
            let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
            let body = extrude(&profiles, &along, 0.0, 60.0 * k, &tol)
                .unwrap()
                .body;
            for (a, b) in [(3.0, 8.0), (2.0, 9.0), (4.0, 8.0)] {
                match chamfer(&body, &bevel(&body, &frame, k, a, b, false), &tol) {
                    Err(error) => assert!(
                        error
                            .to_string()
                            .contains("pass through a face beside the edge"),
                        "{a} {b} at {k}: {error}"
                    ),
                    Ok(made) => panic!(
                        "{a} {b} at {k}: built, {} to {}",
                        mass(&body).volume,
                        mass(&made.body).volume
                    ),
                }
            }
            // A small one, whose trace on the ogee runs before it bends
            // back, is built and fills the corner.
            let made = chamfer(&body, &bevel(&body, &frame, k, 0.5, 1.0, false), &tol)
                .unwrap_or_else(|e| panic!("0.5 and 1 at {k}: {e}"));
            assert_eq!(check(&made.body, &tol), []);
            assert!(mass(&made.body).volume > mass(&body).volume);
        }
    }
}

#[test]
fn a_chamfer_beside_a_cone_is_refused_where_its_line_enters_the_cone_near_one_end() {
    // The plate and a cone along x in place of the pipe: its axis at
    // (y, z) = (20, 16), radius 10 at x = 0 narrowing to 7 at the plate's
    // end (its apex at x = 200). Its sections across the edge where it
    // leaves the top change along the edge, and the line from 1 along the
    // top to b along the cone first enters the cone near the narrow end.
    // Worked out in each plane across the edge (the cone's trace there a
    // conic, the feet straight from the edge, x of the edge in steps of
    // 1/60): up to 3.4907 it enters the cone only where the plane has
    // left the face (past x = 60, the plate's end); at 3.5 on the face by
    // a lens up to 5.9e-6 deep at x 59.77 to 59.99, in the planes through
    // the edge at x 59.53 to 59.75; at 3.55 in those from x 57.37, at 3.6
    // from 55.07. The planes looked at, a sixty-fourth of the edge's
    // parameter apart, are at x 58.78, 59.39 and 60 near that end.
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let plate = primitives::block(&frame, 60.0 * k, 40.0 * k, 10.0 * k).unwrap();
            let axis =
                Frame::new(frame.point(0.0, C[0] * k, C[1] * k), frame.x(), frame.y()).unwrap();
            let cone = primitives::cone(&axis, R * k, 200.0 * k).unwrap();
            let joined = boolean(&plate, &cone, BooleanOp::Union, &tol).unwrap().body;
            let beyond = primitives::block(
                &Frame::new(
                    frame.point(60.0 * k, -50.0 * k, -50.0 * k),
                    frame.z(),
                    frame.x(),
                )
                .unwrap(),
                300.0 * k,
                150.0 * k,
                150.0 * k,
            )
            .unwrap();
            let body = boolean(&joined, &beyond, BooleanOp::Subtract, &tol)
                .unwrap()
                .body;
            // The high edge where the cone leaves the top, at x = 30
            // (radius 8.5: 6.02 off the axis's plane).
            let edge = edge_near(&body, frame.point(30.0 * k, 26.02 * k, 10.0 * k));
            let curved = faces_of(&body, edge)
                .into_iter()
                .find(|f| !matches!(body.faces[f.0 as usize].surface, Surface::Plane(_)))
                .unwrap();
            let request = |b: f64| ChamferEdge {
                edge,
                first: curved,
                size: ChamferSize::Distances {
                    first: b * k,
                    second: k,
                },
            };
            // Clear on the face: built, and it fills the corner; points
            // just off the chamfer face near the narrow end, on the side
            // it leaves empty, are outside.
            for b in [1.0, 3.49] {
                let made = chamfer(&body, &[request(b)], &tol)
                    .unwrap_or_else(|e| panic!("{b} at {k}: {e}"));
                assert_eq!(check(&made.body, &tol), []);
                assert!(mass(&made.body).volume > mass(&body).volume);
                let points = off_the_chamfer(&made.body, &made.faces, &frame, k, &tol);
                assert!(points.len() >= 10, "{b} at {k}: {}", points.len());
                let places = classify_points(&made.body, &points, &tol).unwrap();
                assert!(
                    places.iter().all(|p| *p == Place::Outside),
                    "{b} at {k}: {places:?}"
                );
            }
            // 3.5 enters the cone on the face only between the last two
            // planes looked at: not seen, and built as 0.1.0 built it,
            // wrong by that lens. What the search can miss (a crossing
            // shorter than a sixty-fourth of the edge), pinned.
            assert!(
                chamfer(&body, &[request(3.5)], &tol).is_ok(),
                "3.5 at {k}: the plane at x 59.39 now sees it"
            );
            // From 3.52 the stretch reaches the plane at x 59.39: refused.
            for b in [3.52, 3.55, 3.6] {
                let error = chamfer(
                    &body,
                    &[ChamferEdge {
                        edge,
                        first: curved,
                        size: ChamferSize::Distances {
                            first: b * k,
                            second: k,
                        },
                    }],
                    &tol,
                )
                .unwrap_err();
                assert!(
                    error.to_string().contains("pass through"),
                    "{b} at {k}: {error}"
                );
            }
        }
    }
}

/// Points just off the chamfer face of `body` (`faces` saying which face
/// is what) on the side it leaves empty, where the plate's frame puts x
/// between 59.8 and 59.99 (of 60), inside the face and away from its
/// borders.
fn off_the_chamfer(
    body: &Body,
    faces: &[ChamferFace],
    frame: &Frame,
    k: f64,
    tol: &craft3d_math::Tolerance,
) -> Vec<Vec3> {
    let index = faces
        .iter()
        .position(|f| matches!(f, ChamferFace::Chamfer(_)))
        .unwrap();
    let face = &body.faces[index];
    let probe = Probe::new(body, tol).unwrap();
    let ((u0, u1), (v0, v1)) = face.surface.domain();
    let mut points = Vec::new();
    for i in 0..=400 {
        for j in 2..19 {
            let u = u0 + (u1 - u0) * f64::from(i) / 400.0;
            let v = v0 + (v1 - v0) * f64::from(j) / 20.0;
            let p = face.surface.eval(u, v);
            let x = (p - frame.origin()).dot(frame.x()) / k;
            if !(59.8..59.99).contains(&x) || probe.on_face(index, p).unwrap() != OnFace::In {
                continue;
            }
            let n = face.surface.normal(u, v).unwrap();
            let out = if face.reversed { -n } else { n };
            points.push(p + out * (1e-5 * k));
        }
    }
    points
}

// ----------------------------------------------------------------- a rim

/// A block 20 x 20 x 20 in `frame` less a hole of radius 5 through it
/// along y whose axis is at (x, z) = (10, `zc`).
fn holed(frame: &Frame, k: f64, zc: f64) -> Body {
    let block = primitives::block(frame, 20.0 * k, 20.0 * k, 20.0 * k).unwrap();
    let axis = Frame::new(frame.point(10.0 * k, 0.0, zc * k), frame.y(), frame.x()).unwrap();
    let hole = primitives::cylinder(&axis, 5.0 * k, 20.0 * k).unwrap();
    boolean(&block, &hole, BooleanOp::Subtract, &tolerance(k))
        .unwrap()
        .body
}

/// Every edge of the rim at y = 0 (one circle, or two arcs where the rim
/// touches the bottom), the front face first.
fn rim(body: &Body, frame: &Frame, k: f64, d: f64) -> Vec<ChamferEdge> {
    let mut found = Vec::new();
    for (index, data) in body.edges.iter().enumerate() {
        let middle = data.curve.eval(0.5 * (data.range.0 + data.range.1));
        let local = middle - frame.origin();
        let edge = EdgeId(index as u32);
        let faces = faces_of(body, edge);
        let front = faces
            .iter()
            .copied()
            .find(|f| matches!(body.faces[f.0 as usize].surface, Surface::Plane(_)));
        let hole = faces
            .iter()
            .any(|f| matches!(body.faces[f.0 as usize].surface, Surface::Cylinder(_)));
        if local.dot(frame.y()).abs() <= 1e-9 * k
            && let (Some(front), true) = (front, hole)
        {
            found.push(ChamferEdge {
                edge,
                first: front,
                size: ChamferSize::Distances {
                    first: d * k,
                    second: d * k,
                },
            });
        }
    }
    assert!(!found.is_empty());
    found
}

/// What the chamfer `d` of the rim of the hole that touches the bottom
/// takes from the block: the chamfer's ring where it lies in the block.
/// At `ρ` from the axis (5 to 5 + d) its section is `d - (ρ - 5)` deep, and
/// its circle lies in the block (above the bottom, 5 below the axis) for
/// the angle `2π - 2 acos(5 / ρ)`; with `ρ = 5 + t²`, which takes away the
/// root at `ρ = 5`, Simpson's rule.
fn rim_ring(d: f64) -> f64 {
    let n = 4000;
    let h = d.sqrt() / f64::from(n);
    let mut sum = 0.0;
    for i in 0..=n {
        let t = h * f64::from(i);
        let rho = 5.0 + t * t;
        let angle = 2.0 * PI - 2.0 * (5.0 / rho).min(1.0).pacos();
        let f = (d - (rho - 5.0)) * rho * angle * 2.0 * t;
        let w = if i == 0 || i == n {
            1.0
        } else if i % 2 == 1 {
            4.0
        } else {
            2.0
        };
        sum += w * f;
    }
    sum * h / 3.0
}

#[test]
fn a_chamfer_on_the_rim_of_a_hole_that_touches_the_bottom_is_the_ring_in_the_block() {
    // The hole touches the bottom along a line; the rim is two arcs that
    // go on into each other where they meet the bottom, where the front
    // face has no width under the rim. The chamfer of both is the block
    // less the chamfer's ring where it lies in the block (6289.508178 at
    // 3), its face pinched to a point where the hole touches the bottom:
    // the material is the chamfer's.
    let v0 = 8000.0 - 25.0 * PI * 20.0;
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let body = holed(&frame, k, 5.0);
            for d in [3.0, 1.0, 0.25] {
                let edges = rim(&body, &frame, k, d);
                assert_eq!(edges.len(), 2, "two arcs");
                let made =
                    chamfer(&body, &edges, &tol).unwrap_or_else(|e| panic!("{d} at {k}: {e}"));
                assert_eq!(check(&made.body, &tol), [], "{d} at {k}");
                let expected = (v0 - rim_ring(d)) * k * k * k;
                assert_close(mass(&made.body).volume, expected, "volume");
            }
            // With a wall of 1 under the hole the rim is one circle, and
            // a chamfer up to that wall is the block less the ring the
            // chamfer's triangle sweeps (Pappus: its centroid is d / 3
            // out from the rim).
            let body = holed(&frame, k, 6.0);
            let d = 0.5;
            let made = chamfer(&body, &rim(&body, &frame, k, d), &tol).unwrap();
            assert_eq!(check(&made.body, &tol), []);
            let ring = 2.0 * PI * (5.0 + d / 3.0) * 0.5 * d * d;
            assert_close(mass(&made.body).volume, (v0 - ring) * k * k * k, "volume");
            match chamfer(&body, &rim(&body, &frame, k, 1.5), &tol) {
                Err(KernelError::ChamferTooLarge { largest, .. }) => {
                    assert!((largest - k).abs() <= 1e-3 * k, "1.5 at {k}: {largest}");
                }
                other => panic!("1.5 at {k}: {other:?}"),
            }
        }
    }
}

// ------------------------------------------------------------ a bead

/// A clamped cubic B-spline in the section (uniform interior knots).
struct Spline {
    control: Vec<P>,
    knots: Vec<f64>,
}

impl Spline {
    fn new(control: Vec<P>) -> Spline {
        let spans = control.len() - 3;
        let mut knots = vec![0.0; 4];
        knots.extend((1..spans).map(|i| i as f64 / spans as f64));
        knots.extend([1.0; 4]);
        Spline { control, knots }
    }

    /// Its point at `t` (de Boor).
    fn at(&self, t: f64) -> P {
        let p = 3;
        let n = self.control.len();
        let mut span = p;
        while span + 1 < n && self.knots[span + 1] <= t {
            span += 1;
        }
        let mut d: Vec<P> = (0..=p).map(|j| self.control[span - p + j]).collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = span - p + j;
                let den = self.knots[i + p + 1 - r] - self.knots[i];
                let a = if den > 0.0 {
                    (t - self.knots[i]) / den
                } else {
                    0.0
                };
                d[j] = [
                    (1.0 - a) * d[j - 1][0] + a * d[j][0],
                    (1.0 - a) * d[j - 1][1] + a * d[j][1],
                ];
            }
        }
        d[p]
    }

    /// Half the integral of y z' - z y' from 0 to `t1`: five-point
    /// Gauss-Legendre on each knot span (in each a polynomial of degree
    /// five), with the derivative exact ([`Spline::derivative`]).
    fn green(&self, t1: f64) -> f64 {
        const X: [f64; 5] = [
            -0.906_179_845_938_664,
            -0.538_469_310_105_683_1,
            0.0,
            0.538_469_310_105_683_1,
            0.906_179_845_938_664,
        ];
        const W: [f64; 5] = [
            0.236_926_885_056_189_1,
            0.478_628_670_499_366_5,
            0.568_888_888_888_888_9,
            0.478_628_670_499_366_5,
            0.236_926_885_056_189_1,
        ];
        let mut breaks: Vec<f64> = self.knots.iter().copied().filter(|k| *k < t1).collect();
        breaks.dedup();
        breaks.push(t1);
        let mut sum = 0.0;
        for pair in breaks.windows(2) {
            let (m, h) = (0.5 * (pair[0] + pair[1]), 0.5 * (pair[1] - pair[0]));
            for (x, w) in X.iter().zip(W) {
                let u = m + h * x;
                let (p, q) = (self.at(u), self.derivative(u));
                sum += w * h * 0.5 * (p[0] * q[1] - p[1] * q[0]);
            }
        }
        sum
    }

    /// The derivative exactly: the spline of degree two of the control
    /// points' differences.
    fn derivative(&self, t: f64) -> P {
        let p = 3;
        let n = self.control.len();
        let mut span = p;
        while span + 1 < n && self.knots[span + 1] <= t {
            span += 1;
        }
        // Basis of degree two on the knots without their ends.
        let q: Vec<P> = (0..n - 1)
            .map(|i| {
                let den = self.knots[i + p + 1] - self.knots[i + 1];
                let s = if den > 0.0 { p as f64 / den } else { 0.0 };
                [
                    s * (self.control[i + 1][0] - self.control[i][0]),
                    s * (self.control[i + 1][1] - self.control[i][1]),
                ]
            })
            .collect();
        let knots = &self.knots[1..self.knots.len() - 1];
        let p = 2;
        let span = span - 1;
        let mut d: Vec<P> = (0..=p).map(|j| q[span - p + j]).collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = span - p + j;
                let den = knots[i + p + 1 - r] - knots[i];
                let a = if den > 0.0 { (t - knots[i]) / den } else { 0.0 };
                d[j] = [
                    (1.0 - a) * d[j - 1][0] + a * d[j][0],
                    (1.0 - a) * d[j - 1][1] + a * d[j][1],
                ];
            }
        }
        d[p]
    }
}

/// The knife edge with a bead: a wedge of 15° between the bottom (y from
/// 0 to 12) and a clamped cubic spline face from the edge at the origin,
/// its control points `s (cos 15°, sin 15°) + n (-sin 15°, cos 15°)`, `n`
/// lifting the third and fourth into a bead near the edge; closed by
/// lines over to the spline's end.
fn bead() -> Spline {
    let (c, s) = (15.0_f64.to_radians().pcos(), 15.0_f64.to_radians().psin());
    let along = [0.0, 0.15, 0.35, 0.55, 0.75, 0.95, 1.3, 2.5, 4.0, 6.0];
    let lift = [0.0, 0.0, 0.45, 0.45, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    Spline::new(
        along
            .iter()
            .zip(lift)
            .map(|(a, n)| [a * c - n * s, a * s + n * c])
            .collect(),
    )
}

fn bead_profile(spline: &Spline, k: f64) -> Vec<SketchCurve> {
    let end = *spline.control.last().unwrap();
    let line = |id: u64, s: P, e: P| SketchCurve {
        id,
        curve: Curve2::Line {
            start: [s[0] * k, s[1] * k],
            end: [e[0] * k, e[1] * k],
        },
    };
    vec![
        line(1, [0.0, 0.0], [12.0, 0.0]),
        line(2, [12.0, 0.0], [12.0, end[1]]),
        line(3, [12.0, end[1]], end),
        SketchCurve {
            id: 4,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: spline.knots.clone(),
                control: spline
                    .control
                    .iter()
                    .map(|p| [p[0] * k, p[1] * k])
                    .collect(),
            }),
        },
    ]
}

/// What the chamfer `d1` along the spline and `d2` along the bottom takes
/// per unit of length (scale 1): the region bounded by the spline from
/// the edge to its foot (`d1` straight from the edge), the line to the
/// foot on the bottom and the bottom back (Green's theorem); and how far
/// past the line between the feet the spline runs between the edge and its
/// foot, with where along the line (from the foot on the spline towards
/// the other) that is.
fn bead_section(spline: &Spline, d1: f64, d2: f64) -> (f64, f64, f64) {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..200 {
        let middle = 0.5 * (lo + hi);
        let p = spline.at(middle);
        if p[0].phypot(p[1]) < d1 {
            lo = middle;
        } else {
            hi = middle;
        }
    }
    let t1 = 0.5 * (lo + hi);
    let (g1, g2) = (spline.at(t1), [d2, 0.0]);
    // Counter-clockwise: the bottom out to G2, the line to G1, the
    // spline back to the edge.
    let lines = 0.5 * (g2[0] * g1[1] - g1[0] * g2[1]);
    let taken = lines - spline.green(t1);
    let length = (g2[0] - g1[0]).phypot(g2[1] - g1[1]);
    let along = [(g2[0] - g1[0]) / length, (g2[1] - g1[1]) / length];
    // Towards the edge (the origin), square to the line.
    let towards = {
        let off = [-g1[0], -g1[1]];
        let a = off[0] * along[0] + off[1] * along[1];
        let t = [off[0] - a * along[0], off[1] - a * along[1]];
        let n = t[0].phypot(t[1]);
        [t[0] / n, t[1] / n]
    };
    let (mut worst, mut at) = (f64::INFINITY, 0.0);
    for i in 0..=4000 {
        let p = spline.at(t1 * f64::from(i) / 4000.0);
        let c = (p[0] - g1[0]) * towards[0] + (p[1] - g1[1]) * towards[1];
        if c < worst {
            worst = c;
            at = (p[0] - g1[0]) * along[0] + (p[1] - g1[1]) * along[1];
        }
    }
    (taken, worst, at)
}

#[test]
fn a_chamfer_beside_a_bead_that_lies_past_its_line_beyond_the_foot_is_built() {
    // The bead lies past the line between the feet only beyond the foot on
    // the spline (along the line's extension, away from the other foot):
    // the line between the feet meets neither face, the spline comes to its
    // foot from the edge's side, and the chamfer is the region between
    // them (0.6943880902 per unit of length for 1 and 4, 0.4355690451 for 1
    // and 2), as 0.1.0 built it. Held against the line rather than the
    // segment between the feet it was refused (2026-11-14).
    let spline = bead();
    let (taken, worst, at) = bead_section(&spline, 1.0, 4.0);
    assert!((taken - 0.694_388_090_2).abs() < 1e-9, "{taken}");
    assert!(
        (worst + 0.2226).abs() < 1e-4 && (at + 0.626).abs() < 1e-3,
        "{worst} at {at}"
    );
    let (taken_2, _, _) = bead_section(&spline, 1.0, 2.0);
    assert!((taken_2 - 0.435_569_045_1).abs() < 1e-9, "{taken_2}");
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let profiles = detect_profiles(&bead_profile(&spline, k), &tol).unwrap();
            let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
            let body = extrude(&profiles, &along, 0.0, 40.0 * k, &tol)
                .unwrap()
                .body;
            let v0 = mass(&body).volume;
            let edge = edge_near(&body, frame.point(20.0 * k, 0.0, 0.0));
            let curved = faces_of(&body, edge)
                .into_iter()
                .find(|f| !matches!(body.faces[f.0 as usize].surface, Surface::Plane(_)))
                .unwrap();
            for d2 in [4.0, 2.0] {
                let (taken, _, _) = bead_section(&spline, 1.0, d2);
                let request = ChamferEdge {
                    edge,
                    first: curved,
                    size: ChamferSize::Distances {
                        first: k,
                        second: d2 * k,
                    },
                };
                let made = chamfer(&body, &[request], &tol)
                    .unwrap_or_else(|e| panic!("1 and {d2} at {k}: {e}"));
                assert_eq!(check(&made.body, &tol), [], "1 and {d2} at {k}");
                let expected = v0 - 40.0 * taken * k * k * k;
                let got = mass(&made.body).volume;
                assert!(
                    (got - expected).abs() <= 1e-7 * v0,
                    "1 and {d2} at {k}: {got} for {expected}"
                );
            }
        }
    }
}

/// Down the bead's far slope: for the chamfer `d1` along the spline and
/// `d2` along the bottom (scale 1), how far the spline beyond its foot runs
/// into the region between the feet (on the edge's side of the line,
/// between the feet), and how far the stretch before the foot lies past
/// the line next to the foot.
fn bead_beyond(spline: &Spline, d1: f64, d2: f64) -> (f64, f64) {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..200 {
        let middle = 0.5 * (lo + hi);
        let p = spline.at(middle);
        if p[0].phypot(p[1]) < d1 {
            lo = middle;
        } else {
            hi = middle;
        }
    }
    let t1 = 0.5 * (lo + hi);
    let (g1, g2) = (spline.at(t1), [d2, 0.0]);
    let length = (g2[0] - g1[0]).phypot(g2[1] - g1[1]);
    let along = [(g2[0] - g1[0]) / length, (g2[1] - g1[1]) / length];
    let towards = {
        let off = [-g1[0], -g1[1]];
        let a = off[0] * along[0] + off[1] * along[1];
        let t = [off[0] - a * along[0], off[1] - a * along[1]];
        let n = t[0].phypot(t[1]);
        [t[0] / n, t[1] / n]
    };
    let clearance = |p: P| (p[0] - g1[0]) * towards[0] + (p[1] - g1[1]) * towards[1];
    let at = |p: P| (p[0] - g1[0]) * along[0] + (p[1] - g1[1]) * along[1];
    let mut beyond: f64 = 0.0;
    for i in 1..=200_000 {
        let p = spline.at((t1 + 0.1 * f64::from(i) / 200_000.0).min(1.0));
        let (c, a) = (clearance(p), at(p));
        if !(c > 0.0 && a > 0.0 && a < length) {
            break;
        }
        beyond = beyond.max(c);
    }
    let mut before: f64 = 0.0;
    for i in 1..=200_000 {
        let c = clearance(spline.at(t1 * (1.0 - f64::from(i) / 200_000.0)));
        if c >= 0.0 {
            break;
        }
        before = before.max(-c);
    }
    (beyond, before)
}

#[test]
fn a_chamfer_that_runs_out_tangent_down_a_beads_far_slope_is_built_within_the_tolerance() {
    // The knife edge with the bead, 4 along the bottom and `d1` along the
    // spline, its foot down the bead's far slope: at 0.8373004 the line
    // between the feet is tangent to the spline at its foot. Just short of
    // that the spline comes to its foot from past the line (the bead lies
    // 0.27 past it, along its extension beyond the foot), and the face
    // beyond the foot runs back into the region between the feet: by
    // 2.65e-7 at 0.8370 (within the tolerance: built, the region by
    // Green's theorem, 0.6310934807 per unit of length, and 0.6310930555
    // at 0.8373), by 3.0e-4 at 0.8273 and more further down (refused). Held by the
    // depth of the stretch before the foot, 0.8370 and 0.8373 were refused
    // (2026-11-14); 0.1.0 built them, the same region.
    let spline = bead();
    for (d1, area) in [(0.8370, 0.631_093_480_7), (0.8373, 0.631_093_055_5)] {
        let (taken, _, _) = bead_section(&spline, d1, 4.0);
        assert!((taken - area).abs() < 1e-9, "{d1}: {taken}");
        let (beyond, before) = bead_beyond(&spline, d1, 4.0);
        assert!(beyond <= 1e-6 && before > 0.2, "{d1}: {beyond} {before}");
    }
    for d1 in [0.80, 0.83] {
        let (beyond, _) = bead_beyond(&spline, d1, 4.0);
        assert!(beyond > 1e-4, "{d1}: {beyond}");
    }
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let profiles = detect_profiles(&bead_profile(&spline, k), &tol).unwrap();
            let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
            let body = extrude(&profiles, &along, 0.0, 40.0 * k, &tol)
                .unwrap()
                .body;
            let v0 = mass(&body).volume;
            let edge = edge_near(&body, frame.point(20.0 * k, 0.0, 0.0));
            let curved = faces_of(&body, edge)
                .into_iter()
                .find(|f| !matches!(body.faces[f.0 as usize].surface, Surface::Plane(_)))
                .unwrap();
            let request = |d1: f64| ChamferEdge {
                edge,
                first: curved,
                size: ChamferSize::Distances {
                    first: d1 * k,
                    second: 4.0 * k,
                },
            };
            for d1 in [0.8370, 0.8373] {
                let (taken, _, _) = bead_section(&spline, d1, 4.0);
                let made = chamfer(&body, &[request(d1)], &tol)
                    .unwrap_or_else(|e| panic!("{d1} at {k}: {e}"));
                assert_eq!(check(&made.body, &tol), [], "{d1} at {k}");
                let expected = v0 - 40.0 * taken * k * k * k;
                let got = mass(&made.body).volume;
                assert!(
                    (got - expected).abs() <= 1e-7 * v0,
                    "{d1} at {k}: {got} for {expected}"
                );
            }
            for d1 in [0.80, 0.83] {
                let error = chamfer(&body, &[request(d1)], &tol).unwrap_err();
                assert!(
                    error.to_string().contains("pass through"),
                    "{d1} at {k}: {error}"
                );
            }
        }
    }
}

// A chamfer whose cut line runs past a smooth edge of its face onto the
// face beyond (2026-11-14): a boss whose wall leaves the top as an arc and
// goes on as its tangent plane.

/// How far the boss's wall runs on as its tangent plane.
const RUN: f64 = 10.0;

/// The angle about C of the edge E, where the wall leaves the top.
fn at_edge() -> f64 {
    (E[1] - C[1]).patan2(E[0] - C[0])
}

/// Where the wall's arc ends (`phi` degrees about C) and the unit
/// direction in which its tangent plane goes on from there.
fn tangent_point(phi: f64) -> (P, P) {
    let t = phi.to_radians();
    (
        [C[0] + R * t.pcos(), C[1] + R * t.psin()],
        [-t.psin(), t.pcos()],
    )
}

/// The section (y, z) of the plate (y from 0 to 40, z from 0 to 10) with
/// the boss on its top: its wall leaves the top at E as the arc of the
/// circle about C (the pipe's) counter-clockwise up to `phi` degrees, goes
/// on as the arc's tangent plane for [`RUN`], over to y = 10 and down to
/// the top. Its outline, closed, counter-clockwise; the arc from E (index
/// 3) to the tangent point.
fn boss_outline(phi: f64) -> Vec<P> {
    let (t, dir) = tangent_point(phi);
    let q = [t[0] + RUN * dir[0], t[1] + RUN * dir[1]];
    vec![
        [0.0, 0.0],
        [40.0, 0.0],
        [40.0, 10.0],
        E,
        t,
        q,
        [10.0, q[1]],
        [10.0, 10.0],
        [0.0, 10.0],
    ]
}

/// The area of the polygon through `points` (counter-clockwise: positive).
fn polygon_area(points: &[P]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (p, q) = (points[i], points[(i + 1) % n]);
            0.5 * (p[0] * q[1] - q[0] * p[1])
        })
        .sum()
}

/// The boss's section: its area and the length round it (scale 1). The
/// arc bulges out of the outline's polygon by its circular segment.
fn boss_section(phi: f64) -> (f64, f64) {
    let outline = boss_outline(phi);
    let theta = phi.to_radians() - at_edge();
    let n = outline.len();
    let mut round = R * theta;
    for i in (0..n).filter(|i| *i != 3) {
        let (p, q) = (outline[i], outline[(i + 1) % n]);
        round += (q[0] - p[0]).phypot(q[1] - p[1]);
    }
    (
        polygon_area(&outline) + 0.5 * R * R * (theta - theta.psin()),
        round,
    )
}

/// The boss along x of `frame`, 40 long, at scale `k`.
fn boss(frame: &Frame, k: f64, phi: f64, tol: &Tolerance) -> Body {
    let outline = boss_outline(phi);
    let n = outline.len();
    let curves: Vec<SketchCurve> = (0..n)
        .map(|i| {
            let (p, q) = (outline[i], outline[(i + 1) % n]);
            let (start, end) = ([p[0] * k, p[1] * k], [q[0] * k, q[1] * k]);
            SketchCurve {
                id: i as u64 + 1,
                curve: if i == 3 {
                    Curve2::Arc {
                        center: [C[0] * k, C[1] * k],
                        start,
                        end,
                    }
                } else {
                    Curve2::Line { start, end }
                },
            }
        })
        .collect();
    let profiles = detect_profiles(&curves, tol).unwrap();
    let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
    extrude(&profiles, &along, 0.0, 40.0 * k, tol).unwrap().body
}

/// A chamfer of the boss's edge E, `b` of arc along the wall and `a`
/// along the top, worked out in the section (scale 1). The construction
/// in place measures each cut line as the point of its face at the
/// straight distance from the edge that the distance gives on that face
/// (on the wall's circle the chord `2 R sin(b / 2R)`); where the arc ends
/// short of that, the cut line is the point of the tangent plane at that
/// straight distance, the arc goes whole, and the chamfer's line runs
/// from there (docs/design/blends.md › *A cut line carried onto the face
/// beyond*).
struct Cut {
    /// The foot on the wall, and how far along the tangent plane it lies
    /// (zero on the arc).
    foot: P,
    run: f64,
    /// What the chamfer adds per unit of length: the region between the
    /// wall from E to the foot, the line to the top's foot and the top.
    added: f64,
    /// The length of the wall from E to the foot, and of the line.
    wall: f64,
    line: f64,
    /// How far the arc reaches past the line, between the feet, found
    /// among 40001 points of it (negative where it keeps clear); on the
    /// tangent plane the wall runs on from the arc's end to the foot on
    /// the line, a segment no further past it than the arc's end. And the
    /// point of the arc where it reaches furthest.
    past: f64,
    deepest: P,
}

fn cut(phi: f64, b: f64, a: f64) -> Cut {
    let theta = phi.to_radians() - at_edge();
    let (t, dir) = tangent_point(phi);
    let chord = 2.0 * R * (b / (2.0 * R)).psin();
    let g = [E[0] + a, E[1]];
    let (foot, run, arc) = if b <= R * theta {
        (on_pipe(b, true), 0.0, b / R)
    } else {
        // |T - E + s dir| = chord.
        let w = [t[0] - E[0], t[1] - E[1]];
        let half = w[0] * dir[0] + w[1] * dir[1];
        let s = -half + (half * half - (w[0] * w[0] + w[1] * w[1]) + chord * chord).sqrt();
        ([t[0] + s * dir[0], t[1] + s * dir[1]], s, theta)
    };
    let mut outline = vec![E];
    if run > 0.0 {
        outline.push(t);
    }
    outline.push(foot);
    outline.push(g);
    // Clockwise (E up the wall, over to G and back): the arc bulges into
    // the polygon by its segment.
    let added = -polygon_area(&outline) - 0.5 * R * R * (arc - arc.psin());
    let line = (g[0] - foot[0]).phypot(g[1] - foot[1]);
    let w = [(g[0] - foot[0]) / line, (g[1] - foot[1]) / line];
    let mut n = [-w[1], w[0]];
    if (E[0] - foot[0]) * n[0] + (E[1] - foot[1]) * n[1] > 0.0 {
        n = [w[1], -w[0]];
    }
    let (mut past, mut deepest) = (f64::NEG_INFINITY, E);
    let start = at_edge();
    let steps = 40_000;
    for i in 0..=steps {
        let angle = start + arc * f64::from(i) / f64::from(steps);
        let p = [C[0] + R * angle.pcos(), C[1] + R * angle.psin()];
        let along = (p[0] - foot[0]) * w[0] + (p[1] - foot[1]) * w[1];
        let height = (p[0] - foot[0]) * n[0] + (p[1] - foot[1]) * n[1];
        if (0.0..=line).contains(&along) && height > past {
            (past, deepest) = (height, p);
        }
    }
    Cut {
        foot,
        run,
        added,
        wall: R * arc + run,
        line,
        past,
        deepest,
    }
}

/// The edge E of the boss, its wall (the cylinder face) and its top.
fn boss_edge(body: &Body, frame: &Frame, k: f64) -> (EdgeId, FaceId, FaceId) {
    let edge = edge_near(body, frame.point(20.0 * k, E[0] * k, E[1] * k));
    let faces = faces_of(body, edge);
    let wall = *faces
        .iter()
        .find(|f| matches!(body.faces[f.0 as usize].surface, Surface::Cylinder(_)))
        .unwrap();
    let top = *faces.iter().find(|f| **f != wall).unwrap();
    (edge, wall, top)
}

/// The chamfer of the boss's edge, `b` along the wall and `a` along the
/// top (scale 1), with the wall or the top as the first face.
#[allow(clippy::too_many_arguments)]
fn boss_chamfer(
    body: &Body,
    frame: &Frame,
    k: f64,
    b: f64,
    a: f64,
    wall_first: bool,
    tol: &Tolerance,
) -> Result<craft3d_kernel::chamfer::ChamferResult, KernelError> {
    let (edge, wall, top) = boss_edge(body, frame, k);
    let (first, size) = if wall_first {
        (
            wall,
            ChamferSize::Distances {
                first: b * k,
                second: a * k,
            },
        )
    } else {
        (
            top,
            ChamferSize::Distances {
                first: a * k,
                second: b * k,
            },
        )
    };
    chamfer(body, &[ChamferEdge { edge, first, size }], tol)
}

/// The chamfer built of the boss's edge, held to the section: valid, its
/// volume and area, and its face the plane through both cut lines with its
/// origin where the cut line on the first face starts.
#[allow(clippy::too_many_arguments)]
fn assert_boss_chamfer(
    body: &Body,
    frame: &Frame,
    k: f64,
    phi: f64,
    b: f64,
    a: f64,
    wall_first: bool,
    tol: &Tolerance,
) {
    let (section, round) = boss_section(phi);
    let (v0, a0) = (40.0 * section, 40.0 * round + 2.0 * section);
    let c = cut(phi, b, a);
    let what = format!("{phi}° {b} and {a} at {k}, the wall first {wall_first}");
    let made = boss_chamfer(body, frame, k, b, a, wall_first, tol)
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!(check(&made.body, tol), [], "{what}");
    let m = mass(&made.body);
    assert_close(m.volume, (v0 + 40.0 * c.added) * k * k * k, &what);
    let area = a0 + 40.0 * (c.line - a - c.wall) + 2.0 * c.added;
    assert_close(m.area, area * k * k, &what);
    let faces: Vec<usize> = made
        .faces
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f, ChamferFace::Chamfer(_)))
        .map(|(i, _)| i)
        .collect();
    let [face] = faces[..] else {
        panic!("{what}: {faces:?}")
    };
    let Surface::Plane(plane) = &made.body.faces[face].surface else {
        panic!("{what}: not a plane")
    };
    let g = [E[0] + a, E[1]];
    let point = |x: f64, p: P| frame.point(x * k, p[0] * k, p[1] * k);
    let (origin, normal) = (plane.frame().origin(), plane.frame().z());
    for x in [0.0, 40.0] {
        for p in [c.foot, g] {
            let off = (point(x, p) - origin).dot(normal);
            assert!(off.abs() <= 1e-9 * k, "{what}: {off}");
        }
    }
    let first = if wall_first { c.foot } else { g };
    let from = point(0.0, first)
        .distance(origin)
        .min(point(40.0, first).distance(origin));
    assert!(from <= 1e-9 * k, "{what}: the frame's origin {from} off");
}

#[test]
fn a_chamfer_whose_cut_line_runs_past_the_end_of_an_arc_onto_its_tangent_plane_is_built() {
    // Until 2026-11-14 the chamfer face was moved to pass through the
    // plan's foot on the wall's circle, which lies off the tangent plane:
    // with the tangent point at -15°, 4.2 along the wall and 1 along the
    // top left the chamfer face's borders 7.24e-3 off it, an invalid body
    // (in the released 0.1.0 and 0.1.1, where regeneration failed the
    // feature as "not a valid body").
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            for (phi, sizes) in [(-15.0, [4.2, 4.6, 5.0, 5.5]), (-20.0, [3.5, 4.2, 6.0, 9.0])] {
                let body = boss(&frame, k, phi, &tol);
                let (section, _) = boss_section(phi);
                assert_close(mass(&body).volume, 40.0 * section * k * k * k, "the boss");
                for b in sizes {
                    // On the tangent plane, the line between the feet clear
                    // of the arc.
                    let c = cut(phi, b, 1.0);
                    assert!(c.run > 0.0 && c.run < RUN - 1.0, "{phi} {b}: {}", c.run);
                    assert!(c.past < -1e-3, "{phi} {b}: {}", c.past);
                    for wall_first in [true, false] {
                        assert_boss_chamfer(&body, &frame, k, phi, b, 1.0, wall_first, &tol);
                    }
                }
            }
        }
    }
}

/// The size `a` along the top at which the line of the chamfer 5 along the
/// wall (the tangent point at -15°, the foot on the tangent plane) reaches
/// `target` past the arc (by bisection: the less along the top, the
/// steeper the line and the further the arc reaches past it).
fn along_top_at(target: f64) -> f64 {
    let (mut lo, mut hi) = (0.5, 1.0);
    assert!(cut(-15.0, 5.0, lo).past > target && cut(-15.0, 5.0, hi).past < target);
    for _ in 0..60 {
        let middle = 0.5 * (lo + hi);
        if cut(-15.0, 5.0, middle).past > target {
            lo = middle;
        } else {
            hi = middle;
        }
    }
    0.5 * (lo + hi)
}

#[test]
fn a_chamfer_carried_onto_the_tangent_plane_is_refused_where_its_line_cuts_the_arc() {
    // With the tangent point at -10° the line from the foot on the tangent
    // plane to the top's foot runs through the arc (5 along the wall: by
    // 8.4e-3; 5.5: 1.65e-2): no chamfer of the size exists. Built invalid
    // before 2026-11-14, as above; now too large for the wall, saying the
    // size (both distances together) at which the line enters the wall's
    // circle by the tolerance, the foot then on the arc, which builds.
    // 4.6 (the foot on the arc, its line into the circle) was refused so
    // before.
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let body = boss(&frame, k, -10.0, &tol);
            let (_, wall, _) = boss_edge(&body, &frame, k);
            for b in [4.6, 5.0, 5.5] {
                let c = cut(-10.0, b, 1.0);
                assert!(
                    c.past > 1e-3 || lens(1.0, b, true) > 1e-4,
                    "{b}: {}",
                    c.past
                );
                let what = format!("{b} at {k}");
                let error = boss_chamfer(&body, &frame, k, b, 1.0, true, &tol).unwrap_err();
                let KernelError::ChamferTooLarge {
                    face,
                    distance,
                    largest,
                    ..
                } = error
                else {
                    panic!("{what}: {error}");
                };
                assert_eq!(face, wall, "{what}");
                assert_close(distance, b * k, "the distance said");
                let at = touching(1.0, b, true);
                assert!(
                    (largest - at * b * k).abs() <= 1e-4 * at * b * k,
                    "{what}: {largest} for {}",
                    at * b * k
                );
                let s = 0.99 * largest / (b * k);
                assert_boss_chamfer(&body, &frame, k, -10.0, b * s, s, true, &tol);
            }
        }
    }
}

#[test]
fn a_chamfer_carried_onto_the_tangent_plane_with_the_top_first_is_refused_where_its_line_cuts_the_arc()
 {
    // The same chamfers with the top as the first face. Until 2026-11-14
    // the frame kept the plan's foot on the top, which lies on the plane
    // built, so nothing moved and the body was valid, and wrong: the
    // solid bounded by the top, the line and the tangent plane, whose
    // volume grows by the length times the signed area of the region that
    // crosses itself (measured on release/0.1: 1 along the top and 5 along
    // the wall +50.679729, 40 x 1.2669932; 0.5 and 6 -6.535122, a chamfer
    // of a concave edge that takes material away). Now too large for the
    // wall, as with the wall first, with the size at which the line enters
    // the wall's circle by the tolerance, which builds.
    for (b, a) in [(5.0, 1.0), (6.0, 0.5)] {
        let c = cut(-10.0, b, a);
        assert!(c.run > 0.0 && c.past > 1e-3, "{b} {a}: {}", c.past);
    }
    assert!(cut(-10.0, 6.0, 0.5).added < 0.0);
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let body = boss(&frame, k, -10.0, &tol);
            let (_, wall, _) = boss_edge(&body, &frame, k);
            for (b, a) in [(5.0, 1.0), (6.0, 0.5)] {
                let what = format!("{a} along the top and {b} along the wall at {k}");
                let error = boss_chamfer(&body, &frame, k, b, a, false, &tol).unwrap_err();
                let KernelError::ChamferTooLarge {
                    face,
                    distance,
                    largest,
                    ..
                } = error
                else {
                    panic!("{what}: {error}");
                };
                assert_eq!(face, wall, "{what}");
                assert_close(distance, b * k, "the distance said");
                let at = touching(a, b, true);
                assert!(
                    (largest - at * b * k).abs() <= 1e-4 * at * b * k,
                    "{what}: {largest} for {}",
                    at * b * k
                );
                let s = 0.99 * largest / (b * k);
                assert_boss_chamfer(&body, &frame, k, -10.0, b * s, a * s, false, &tol);
            }
        }
    }
}

#[test]
fn a_chamfer_carried_onto_the_tangent_plane_is_built_up_to_where_its_line_cuts_the_arc_by_the_tolerance()
 {
    // The tangent point at -15°, 5 along the wall: the line from the foot
    // on the tangent plane clears the arc for `a` along the top from about
    // 0.75 on. Just past that it is clear (by 5e-6) and within the
    // tolerance (the arc 5e-7 past it): built, the section; 2e-6 past it,
    // refused as too large for the wall, with a size that builds.
    let (clear, within, over) = (along_top_at(-5e-6), along_top_at(5e-7), along_top_at(2e-6));
    assert!(
        clear > within && within > over && clear - over < 1e-4,
        "{clear} {within} {over}"
    );
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let body = boss(&frame, k, -15.0, &tol);
            let (_, wall, _) = boss_edge(&body, &frame, k);
            for a in [clear, within] {
                for wall_first in [true, false] {
                    assert_boss_chamfer(&body, &frame, k, -15.0, 5.0, a, wall_first, &tol);
                }
            }
            for wall_first in [true, false] {
                let what = format!("{over} at {k}, the wall first {wall_first}");
                let error =
                    boss_chamfer(&body, &frame, k, 5.0, over, wall_first, &tol).unwrap_err();
                let KernelError::ChamferTooLarge {
                    face,
                    distance,
                    largest,
                    ..
                } = error
                else {
                    panic!("{what}: {error}");
                };
                // The distance along the wall, and the largest of it.
                assert_eq!(face, wall, "{what}");
                assert_close(distance, 5.0 * k, "the distance said");
                let s = largest / distance;
                assert!(s < 1.0, "{what}: {largest}");
                assert_boss_chamfer(&body, &frame, k, -15.0, 5.0 * s, over * s, wall_first, &tol);
            }
        }
    }
}

#[test]
fn chamfers_of_bosses_whose_wall_goes_on_as_its_tangent_plane_are_built_where_their_line_is_clear()
{
    // A sweep for refusals of right chamfers and chamfers built wrong: the
    // tangent point from -25° to -5°, from 2.5 to 16 along the wall (on the
    // arc, on the tangent plane and past it) and 0.5 to 2 along the top. On
    // the arc the chamfer is refused exactly where its line enters the
    // wall's circle by a lens deeper than the tolerance; on the tangent
    // plane where the arc reaches past its line by more than the tolerance;
    // past the plane's sharp far edge always; every other one is built and
    // is the section. Sizes within a factor of two of the tolerance from the
    // limit, a foot within 0.05 of the end of the arc or 0.5 of the end of
    // the tangent plane are left out. (2026-11-14: 232 built, 118 refused.)
    let tol = tolerance(1.0);
    let (mut built, mut refused) = (0, 0);
    for frame in [Frame::WORLD, frames(1.0)[2]] {
        for phi in [-25.0, -20.0, -15.0, -12.0, -11.0, -10.0, -5.0] {
            let body = boss(&frame, 1.0, phi, &tol);
            let theta = phi.to_radians() - at_edge();
            let sizes = [2.5, 3.5, 4.5, 5.5, 6.5, 8.0, 10.0, 12.0, 14.0, 16.0];
            for b in sizes {
                let tops: &[f64] = if b > 12.0 { &[1.0] } else { &[0.5, 1.0, 2.0] };
                for &a in tops {
                    let c = cut(phi, b, a);
                    if c.run > RUN + 0.5 {
                        // Past the tangent plane's sharp far edge: no chamfer
                        // that carries its cut line once.
                        let made = boss_chamfer(&body, &frame, 1.0, b, a, true, &tol);
                        assert!(made.is_err(), "{phi}° {b} and {a}: built past the plane");
                        refused += 1;
                        continue;
                    }
                    if (b - R * theta).abs() < 0.05 || c.run > RUN - 0.5 {
                        continue;
                    }
                    let deep = if c.run == 0.0 {
                        lens(a, b, true)
                    } else {
                        c.past
                    };
                    if deep > 0.5 * DEEP && deep < 2.0 * DEEP {
                        continue;
                    }
                    let what = format!("{phi}° {b} and {a}");
                    if deep > DEEP {
                        let made = boss_chamfer(&body, &frame, 1.0, b, a, true, &tol);
                        assert!(
                            made.is_err(),
                            "{what}: built, its line {deep} into the wall"
                        );
                        refused += 1;
                    } else {
                        assert_boss_chamfer(&body, &frame, 1.0, phi, b, a, true, &tol);
                        built += 1;
                    }
                }
            }
        }
    }
    println!("{built} built, {refused} refused");
    assert!(
        built > 100 && refused > 40,
        "{built} built, {refused} refused"
    );
}

/// A plate whose top goes on into a round: the section (y, z) from y = 0 to
/// 40 and z = 0 to 20 with a step down to the top z = 10 at y = 28 (the
/// wall, from z = 10 up), the top running to y = 36 and on into a round of
/// radius 4 about (36, 6) down to the side y = 40. Its outline, and the
/// round's centre and radius.
const ROUND: (P, f64) = ([36.0, 6.0], 4.0);

fn rounded_outline() -> Vec<P> {
    vec![
        [0.0, 0.0],
        [40.0, 0.0],
        [40.0, 6.0],
        [36.0, 10.0],
        E,
        [28.0, 20.0],
        [0.0, 20.0],
    ]
}

/// The rounded plate along x of `frame`, 40 long, at scale `k`; the round
/// is the curve from index 2.
fn rounded(frame: &Frame, k: f64, tol: &Tolerance) -> Body {
    let outline = rounded_outline();
    let n = outline.len();
    let curves: Vec<SketchCurve> = (0..n)
        .map(|i| {
            let (p, q) = (outline[i], outline[(i + 1) % n]);
            let (start, end) = ([p[0] * k, p[1] * k], [q[0] * k, q[1] * k]);
            SketchCurve {
                id: i as u64 + 1,
                curve: if i == 2 {
                    Curve2::Arc {
                        center: [ROUND.0[0] * k, ROUND.0[1] * k],
                        start,
                        end,
                    }
                } else {
                    Curve2::Line { start, end }
                },
            }
        })
        .collect();
    let profiles = detect_profiles(&curves, tol).unwrap();
    let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
    extrude(&profiles, &along, 0.0, 40.0 * k, tol).unwrap().body
}

/// The chamfer of the rounded plate's edge E, `d` along the top and `e` up
/// the wall (scale 1), past the round's tangent line at y = 36: the foot
/// on the round `d` (straight) from E, the region it fills (between the
/// top, the round, the line to the wall's foot and the wall), the length
/// of the top and round it covers and of its line, and how far the top and
/// the round from E to the foot reach past the line between the feet
/// (among 40001 points; negative where they keep clear).
fn rounded_cut(d: f64, e: f64) -> (P, f64, f64, f64, f64) {
    let ([kx, ky], r) = ROUND;
    let s = [36.0, 10.0];
    // |K + r u(φ) - E|² = d²: 8 cos φ - 4 sin φ = (d² - 96) / 8.
    let amplitude = 80.0_f64.sqrt();
    let phi = ((d * d - 96.0) / (8.0 * amplitude)).pacos() - 4.0_f64.patan2(8.0);
    let foot = [kx + r * phi.pcos(), ky + r * phi.psin()];
    let theta = PI / 2.0 - phi;
    let g = [E[0], E[1] + e];
    let added = polygon_area(&[E, s, foot, g]) - 0.5 * r * r * (theta - theta.psin());
    let line = (g[0] - foot[0]).phypot(g[1] - foot[1]);
    let w = [(g[0] - foot[0]) / line, (g[1] - foot[1]) / line];
    let mut n = [-w[1], w[0]];
    if (E[0] - foot[0]) * n[0] + (E[1] - foot[1]) * n[1] > 0.0 {
        n = [w[1], -w[0]];
    }
    let mut past = f64::NEG_INFINITY;
    for i in 0..=40_000 {
        let u = f64::from(i) / 40_000.0;
        let p = if u <= 0.5 {
            [E[0] + 16.0 * u, E[1]]
        } else {
            let angle = PI / 2.0 - theta * (2.0 * u - 1.0);
            [kx + r * angle.pcos(), ky + r * angle.psin()]
        };
        let along = (p[0] - foot[0]) * w[0] + (p[1] - foot[1]) * w[1];
        if (1e-9..=line).contains(&along) {
            past = past.max((p[0] - foot[0]) * n[0] + (p[1] - foot[1]) * n[1]);
        }
    }
    (foot, added, 8.0 + r * theta, line, past)
}

#[test]
fn a_chamfer_whose_cut_line_runs_past_a_tops_edge_onto_a_round_is_built_or_refused_by_its_line() {
    // A straight edge between two planes, chamfered in place: past y = 36
    // the cut line on the top is carried onto the round, at the straight
    // distance from the edge. Built invalid before 2026-11-14 (the face
    // moved to pass through the top's plane continued); now built, the
    // section, where its line clears the top and the round, and refused as
    // too large where it runs through the top (9 along the top and 1 up the
    // wall: the line from the round to the wall's foot dips under the top)
    // or through the round (11 and 9: it leaves the round inward at its
    // foot and comes out 8.7e-3 above it further up).
    let profile = polygon_area(&rounded_outline()) + 8.0 * (PI / 2.0 - 1.0);
    let perimeter = 40.0 + 6.0 + 2.0 * PI + 8.0 + 10.0 + 28.0 + 20.0;
    let (v0, a0) = (40.0 * profile, 40.0 * perimeter + 2.0 * profile);
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let body = rounded(&frame, k, &tol);
            assert_close(mass(&body).volume, v0 * k * k * k, "the rounded plate");
            let edge = edge_near(&body, frame.point(20.0 * k, E[0] * k, E[1] * k));
            let faces = faces_of(&body, edge);
            let top = *faces
                .iter()
                .find(|f| {
                    let Surface::Plane(p) = &body.faces[f.0 as usize].surface else {
                        return false;
                    };
                    p.frame().z().dot(frame.z()).abs() > 0.5
                })
                .unwrap();
            let wall = *faces.iter().find(|f| **f != top).unwrap();
            let request = |d: f64, e: f64, top_first: bool| ChamferEdge {
                edge,
                first: if top_first { top } else { wall },
                size: if top_first {
                    ChamferSize::Distances {
                        first: d * k,
                        second: e * k,
                    }
                } else {
                    ChamferSize::Distances {
                        first: e * k,
                        second: d * k,
                    }
                },
            };
            for (d, e) in [(9.0, 9.0), (10.0, 9.0), (9.5, 6.0)] {
                let (_, added, covered, line, past) = rounded_cut(d, e);
                assert!(past <= 0.0, "{d} {e}: {past}");
                for top_first in [true, false] {
                    let what = format!("{d} and {e} at {k}, the top first {top_first}");
                    let made = chamfer(&body, &[request(d, e, top_first)], &tol)
                        .unwrap_or_else(|error| panic!("{what}: {error}"));
                    assert_eq!(check(&made.body, &tol), [], "{what}");
                    let m = craft3d_topo::mass_properties(&made.body)
                        .unwrap_or_else(|error| panic!("{what}: {error}"));
                    assert_close(m.volume, (v0 + 40.0 * added) * k * k * k, &what);
                    let area = a0 + 40.0 * (line - covered - e) + 2.0 * added;
                    assert_close(m.area, area * k * k, &what);
                }
            }
            for (d, e) in [(9.0, 1.0), (10.0, 3.0), (11.0, 9.0)] {
                let (_, _, _, _, past) = rounded_cut(d, e);
                assert!(past > 1e-3, "{d} {e}: {past}");
                let what = format!("{d} and {e} at {k}");
                let error = chamfer(&body, &[request(d, e, true)], &tol).unwrap_err();
                assert!(
                    matches!(error, KernelError::ChamferTooLarge { .. }),
                    "{what}: {error}"
                );
            }
        }
    }
}

/// The plate and the pipe as one extruded section whose arc above the top
/// is two arcs of the pipe's circle that meet at `split` degrees (a smooth
/// edge between two faces of one cylinder), 40 long along x of `frame`.
fn split_pipe(frame: &Frame, k: f64, split: f64, tol: &Tolerance) -> Body {
    let t = split.to_radians();
    let p1 = [C[0] + R * t.pcos(), C[1] + R * t.psin()];
    let s = |p: P| [p[0] * k, p[1] * k];
    let line = |id: u64, a: P, b: P| SketchCurve {
        id,
        curve: Curve2::Line {
            start: s(a),
            end: s(b),
        },
    };
    let arc = |id: u64, a: P, b: P| SketchCurve {
        id,
        curve: Curve2::Arc {
            center: s(C),
            start: s(a),
            end: s(b),
        },
    };
    let curves = vec![
        line(1, [0.0, 0.0], [40.0, 0.0]),
        line(2, [40.0, 0.0], [40.0, 10.0]),
        line(3, [40.0, 10.0], E),
        arc(4, E, p1),
        arc(5, p1, [12.0, 10.0]),
        line(6, [12.0, 10.0], [0.0, 10.0]),
        line(7, [0.0, 10.0], [0.0, 0.0]),
    ];
    let profiles = detect_profiles(&curves, tol).unwrap();
    let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
    extrude(&profiles, &along, 0.0, 40.0 * k, tol).unwrap().body
}

#[test]
fn a_chamfer_whose_cut_line_runs_onto_the_next_face_of_one_cylinder_is_the_pipes() {
    // The pipe on the plate with its wall made of two faces of one
    // cylinder, split 2.07 of arc above the edge: the cut line along the
    // wall is carried onto the second face, on the same circle, and the
    // chamfer is the pipe's (built as 0.1.0 built it up to where its line
    // enters the pipe by the tolerance). Past that 0.1.0 built it wrong,
    // as the pipe in one face was until 2026-11-13 (the plan's section,
    // whose foot lies off the first face, stood down): now too large, with
    // the size the pipe in one face is given.
    let v0 = 40.0 * (400.0 + PI * R * R - dip());
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let body = split_pipe(&frame, k, -25.0, &tol);
            assert_close(mass(&body).volume, v0 * k * k * k, "the split pipe");
            let edge = edge_near(&body, frame.point(20.0 * k, E[0] * k, E[1] * k));
            let wall = faces_of(&body, edge)
                .into_iter()
                .find(|f| matches!(body.faces[f.0 as usize].surface, Surface::Cylinder(_)))
                .unwrap();
            let request = |b: f64, a: f64| ChamferEdge {
                edge,
                first: wall,
                size: ChamferSize::Distances {
                    first: b * k,
                    second: a * k,
                },
            };
            for b in [1.5, 3.0, 4.0, 4.467, 4.469] {
                let what = format!("{b} at {k}");
                let made = chamfer(&body, &[request(b, 1.0)], &tol)
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
                assert_eq!(check(&made.body, &tol), [], "{what}");
                let expected = (v0 + 40.0 * section_area(1.0, b, true)) * k * k * k;
                assert_close(mass(&made.body).volume, expected, &what);
            }
            for b in [4.6, 8.0] {
                let what = format!("{b} at {k}");
                let error = chamfer(&body, &[request(b, 1.0)], &tol).unwrap_err();
                let KernelError::ChamferTooLarge { largest, .. } = error else {
                    panic!("{what}: {error}");
                };
                let at = touching(1.0, b, true);
                assert!(
                    (largest - at * b * k).abs() <= 1e-4 * at * b * k,
                    "{what}: {largest} for {}",
                    at * b * k
                );
            }
        }
    }
}

/// The boss whose wall leaves the top at E as the arc about C up to `phi`
/// degrees and goes on tangentially as a cubic Bézier spline [`RUN`] long
/// whose end lies `bend` to the left of the tangent (0: the tangent plane
/// itself, as a spline face), 40 long along x of `frame`; and its control
/// points.
fn spline_boss(frame: &Frame, k: f64, phi: f64, bend: f64, tol: &Tolerance) -> (Body, [P; 4]) {
    let (t, dir) = tangent_point(phi);
    let side = [-dir[1], dir[0]];
    let at = |s: f64, off: f64| {
        [
            t[0] + s * dir[0] + off * side[0],
            t[1] + s * dir[1] + off * side[1],
        ]
    };
    let control = [
        t,
        at(RUN / 3.0, 0.0),
        at(2.0 * RUN / 3.0, bend / 3.0),
        at(RUN, bend),
    ];
    let q = control[3];
    let s = |p: P| [p[0] * k, p[1] * k];
    let line = |id: u64, a: P, b: P| SketchCurve {
        id,
        curve: Curve2::Line {
            start: s(a),
            end: s(b),
        },
    };
    let curves = vec![
        line(1, [0.0, 0.0], [40.0, 0.0]),
        line(2, [40.0, 0.0], [40.0, 10.0]),
        line(3, [40.0, 10.0], E),
        SketchCurve {
            id: 4,
            curve: Curve2::Arc {
                center: s(C),
                start: s(E),
                end: s(t),
            },
        },
        SketchCurve {
            id: 5,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: control.iter().map(|p| s(*p)).collect(),
            }),
        },
        line(6, q, [10.0, q[1]]),
        line(7, [10.0, q[1]], [10.0, 10.0]),
        line(8, [10.0, 10.0], [0.0, 10.0]),
        line(9, [0.0, 10.0], [0.0, 0.0]),
    ];
    let profiles = detect_profiles(&curves, tol).unwrap();
    let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
    (
        extrude(&profiles, &along, 0.0, 40.0 * k, tol).unwrap().body,
        control,
    )
}

/// The plate and the pipe as one extruded section, 40 long along x of
/// `frame`, with the top beside E made of two faces of one plane that meet
/// at y = `split` (or of one face).
fn split_top(frame: &Frame, k: f64, split: Option<f64>, tol: &Tolerance) -> Body {
    let s = |p: P| [p[0] * k, p[1] * k];
    let line = |id: u64, a: P, b: P| SketchCurve {
        id,
        curve: Curve2::Line {
            start: s(a),
            end: s(b),
        },
    };
    let mut curves = vec![
        line(1, [0.0, 0.0], [40.0, 0.0]),
        line(2, [40.0, 0.0], [40.0, 10.0]),
    ];
    match split {
        Some(y) => {
            curves.push(line(3, [40.0, 10.0], [y, 10.0]));
            curves.push(line(4, [y, 10.0], E));
        }
        None => curves.push(line(3, [40.0, 10.0], E)),
    }
    curves.push(SketchCurve {
        id: 5,
        curve: Curve2::Arc {
            center: s(C),
            start: s(E),
            end: s([12.0, 10.0]),
        },
    });
    curves.push(line(6, [12.0, 10.0], [0.0, 10.0]));
    curves.push(line(7, [0.0, 10.0], [0.0, 0.0]));
    let profiles = detect_profiles(&curves, tol).unwrap();
    let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
    extrude(&profiles, &along, 0.0, 40.0 * k, tol).unwrap().body
}

#[test]
fn a_chamfer_beside_a_pipe_whose_top_is_two_faces_of_one_plane_is_the_whole_tops() {
    // The top beside the pipe made of two faces of one plane, 1.5 from the
    // edge: a cut line on the top past 1.5 is carried onto the second face.
    // Until 2026-11-14 the plan's section stood down there (its foot off
    // the first face) and the chamfer was built where its line runs
    // through the pipe (8 along the pipe and 2 or 3 along the top: built,
    // 26914.246 for 2 at scale 1, where the top of one face is refused with
    // 5.069 as what fits). Now every size is what the top of one face
    // gives: the same volume, or the same refusal.
    let mut refused = 0;
    for (k, frame) in [(1.0, Frame::WORLD), (1e6, frames(1e6)[2])] {
        let tol = tolerance(k);
        let whole = split_top(&frame, k, None, &tol);
        let split = split_top(&frame, k, Some(29.5), &tol);
        for b in [2.0, 3.0, 4.0, 4.4, 4.5, 4.6, 5.0, 6.0, 8.0] {
            for a in [1.0, 2.0, 3.0, 5.0] {
                let what = format!("{b} along the pipe and {a} along the top at {k}");
                let outcome = |body: &Body| {
                    let edge = edge_near(body, frame.point(20.0 * k, E[0] * k, E[1] * k));
                    let pipe = faces_of(body, edge)
                        .into_iter()
                        .find(|f| matches!(body.faces[f.0 as usize].surface, Surface::Cylinder(_)))
                        .unwrap();
                    let request = ChamferEdge {
                        edge,
                        first: pipe,
                        size: ChamferSize::Distances {
                            first: b * k,
                            second: a * k,
                        },
                    };
                    chamfer(body, &[request], &tol).map(|m| {
                        assert_eq!(check(&m.body, &tol), [], "{what}");
                        mass(&m.body).volume
                    })
                };
                match (outcome(&whole), outcome(&split)) {
                    (Ok(one), Ok(two)) => assert_close(two, one, &what),
                    (
                        Err(KernelError::ChamferTooLarge { largest: one, .. }),
                        Err(KernelError::ChamferTooLarge { largest: two, .. }),
                    ) => {
                        assert!((two - one).abs() <= 1e-6 * one, "{what}: {two} for {one}");
                        refused += 1;
                    }
                    (one, two) => panic!("{what}: one face {one:?}, two faces {two:?}"),
                }
            }
        }
    }
    assert!(refused > 10, "{refused}");
}

/// Chamfers built wrong with 0.1.0 and still, owed: where the boss's wall
/// goes on as a spline face tangent to the arc (even the tangent plane
/// itself, made a spline), the cut line carried onto it is not held
/// (`carried` holds a plane or a cylinder along the edge beyond; the
/// in-place search stands down where a foot leaves its face:
/// docs/design/blends.md › *A cut line carried onto the face beyond*,
/// Open). Each is the tangent point (degrees), how far the spline's end
/// bends off the tangent, the distance along the wall and along the top,
/// and what the wrong body adds per unit of length (measured on
/// release/0.1 and with 2026-11-14's fix alike). When a change refuses one
/// of them or builds it right, this test fails: take it off the list.
const KNOWN_WRONG: [(f64, f64, f64, f64, f64); 3] = [
    (-10.0, 0.0, 5.0, 1.0, 1.266_993_236_2),
    (-10.0, 0.0, 6.0, 0.5, -0.163_378_046_6),
    (-10.0, 0.2, 6.0, 0.5, -0.171_677_234_3),
];

#[test]
fn chamfers_carried_onto_a_spline_beyond_the_arc_are_built_wrong_still_known() {
    // Why each is wrong: along the straight spline the section is the
    // tangent plane's, whose line runs through the arc (8.4e-3 at 5 and
    // 1, 0.184 at 6 and 0.5; on the tangent plane as a plane refused), and
    // a point of the boss just inside the arc where it lies past the line
    // is left outside the body; at 6 and 0.5 a chamfer of a concave edge
    // takes material away.
    for (k, frame) in [(1.0, Frame::WORLD), (1e6, frames(1e6)[2])] {
        let tol = tolerance(k);
        for (phi, bend, b, a, added) in KNOWN_WRONG {
            let what = format!("{phi}° bend {bend}, {b} and {a} at {k}");
            let (body, _) = spline_boss(&frame, k, phi, bend, &tol);
            let v0 = mass(&body).volume;
            let edge = edge_near(&body, frame.point(20.0 * k, E[0] * k, E[1] * k));
            let wall = faces_of(&body, edge)
                .into_iter()
                .find(|f| matches!(body.faces[f.0 as usize].surface, Surface::Cylinder(_)))
                .unwrap();
            let request = ChamferEdge {
                edge,
                first: wall,
                size: ChamferSize::Distances {
                    first: b * k,
                    second: a * k,
                },
            };
            let made = chamfer(&body, &[request], &tol)
                .unwrap_or_else(|e| panic!("{what}: refused now ({e}): take it off the list"));
            assert_eq!(check(&made.body, &tol), [], "{what}");
            let got = (mass(&made.body).volume - v0) / (40.0 * k * k * k);
            assert!(
                (got - added).abs() <= 1e-8 * added.abs(),
                "{what}: adds {got} per unit of length, not the known wrong {added}: \
                 take it off the list if it is right now"
            );
            if bend == 0.0 {
                let c = cut(phi, b, a);
                assert!(c.run > 0.0 && c.past > 1e-3, "{what}: {}", c.past);
                let inward = [C[0] - c.deepest[0], C[1] - c.deepest[1]];
                let reach = inward[0].phypot(inward[1]);
                let q = [
                    c.deepest[0] + inward[0] / reach * 0.5 * c.past,
                    c.deepest[1] + inward[1] / reach * 0.5 * c.past,
                ];
                let points: Vec<Vec3> = [5.0, 20.0, 35.0]
                    .iter()
                    .map(|x| frame.point(x * k, q[0] * k, q[1] * k))
                    .collect();
                let before = classify_points(&body, &points, &tol).unwrap();
                let after = classify_points(&made.body, &points, &tol).unwrap();
                assert!(
                    before.iter().all(|p| *p == Place::Inside),
                    "{what}: {before:?}"
                );
                assert!(
                    after.iter().all(|p| *p == Place::Outside),
                    "{what}: {after:?}"
                );
            }
            if added < 0.0 {
                assert!(got < 0.0, "{what}");
            }
        }
    }
}
