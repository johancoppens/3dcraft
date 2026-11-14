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
use craft3d_math::{Frame, Vec3};
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
