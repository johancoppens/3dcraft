//! Slivers a few tolerances wide: the corner a face of one body cuts off a
//! face of the other just past its vertex, a face's edge beside a convex
//! or a concave edge of the other body, and the joint of a riveted
//! truss, where three angle bars turned about a common point have edges
//! in one plane that nearly meet in one point (the Garabit pier's finding
//! of 2026-10-18). A region of a face that holds no point at the margin
//! from its boundary is classified at its boundary, and the result is
//! what the exact geometry says: the oracles are closed forms (the volume
//! of a union of prisms by inclusion–exclusion, the corner tetrahedron),
//! the face counts, both orders, the full check of the body and the
//! boolean's own check, at scales from 1e-3 to 1e6.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::BooleanOp::{self, Intersect, Subtract, Union};
use craft3d_kernel::boolean::boolean;
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_topo::{Body, check, mass_properties};

/// The scales these tests run at (the tolerance scales along).
const AT: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

#[track_caller]
fn assert_volume(body: &Body, expected: f64, k: f64, what: &str) {
    let m = mass_properties(body).unwrap();
    // The volume is exact up to the integrator's bound and rounding.
    let allowance = 1e-9 * expected.abs() + 2.0 * tolerance(k).linear * m.area;
    assert!(
        (m.volume - expected).abs() <= allowance,
        "{what}: volume {} but expected {expected} (allowance {allowance})",
        m.volume
    );
}

/// The boolean, which must succeed; its result checked in full.
#[track_caller]
fn op(a: &Body, b: &Body, op: BooleanOp, k: f64, what: &str) -> Body {
    let tol = tolerance(k);
    let body = match boolean(a, b, op, &tol) {
        Ok(r) => r.body,
        Err(e) => panic!("{what}: {op:?} at scale {k}: {e}"),
    };
    assert_eq!(check(&body, &tol), [], "{what}: {op:?} at scale {k}");
    verified(a, b, op, &body, &tol, what);
    body
}

// --- A corner cut off a few tolerances from the vertex -------------------

/// The block beyond the plane that cuts the corner `(1, 1, 1)` off the
/// unit cube at `delta` (times `k`) from the vertex along each edge.
fn corner_cutter(k: f64, delta: f64) -> Body {
    let n = Vec3::new(1.0, 1.0, 1.0).normalized().unwrap();
    let corner = Vec3::new(k, k, k);
    let on_plane = corner - n * (delta * k / 3.0_f64.sqrt());
    let frame = Frame::from_z(on_plane, n).unwrap();
    let origin = on_plane - frame.x() * k - frame.y() * k;
    primitives::block(&frame.at(origin).unwrap(), 2.0 * k, 2.0 * k, 2.0 * k).unwrap()
}

/// A plane through a block a few tolerances from one of its vertices cuts
/// a sliver triangle off each of the three faces there, too thin to hold
/// a point at the margin, and a curve across each that is inside the face
/// at its middle and within the tolerance of an edge at a quarter point.
/// Union, difference and intersection are exact: the face counts say the
/// slivers went where they belong, and the intersection is the corner
/// tetrahedron.
#[test]
fn corner_cut_a_few_tolerances_from_the_vertex() {
    for k in AT {
        let tol = tolerance(k).linear;
        for tolerances in [3.0, 6.0] {
            let delta = tolerances * tol / k;
            let a = boxed(k, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
            let b = corner_cutter(k, delta);
            let what = format!("corner cut at {tolerances} tolerances, scale {k}");
            let tetrahedron = (delta * k).powi(3) / 6.0;
            let cube = k.powi(3);
            let block = 8.0 * cube;
            for (first, second) in [(&a, &b), (&b, &a)] {
                let union = op(first, second, Union, k, &what);
                assert_eq!(union.shells.len(), 1, "{what}");
                assert_eq!(union.faces.len(), 12, "{what}: union faces");
                assert_volume(&union, cube + block - tetrahedron, k, &what);
            }
            let difference = op(&a, &b, Subtract, k, &what);
            assert_eq!(difference.shells.len(), 1, "{what}");
            assert_eq!(difference.faces.len(), 7, "{what}: difference faces");
            assert_volume(&difference, cube - tetrahedron, k, &what);
            let common = op(&a, &b, Intersect, k, &what);
            assert_eq!(common.shells.len(), 1, "{what}");
            assert_eq!(common.faces.len(), 4, "{what}: intersection faces");
            assert_eq!(common.vertices.len(), 4, "{what}: intersection vertices");
            assert_volume(&common, tetrahedron, k, &what);
        }
    }
}

// --- A sliver beside an edge of the other body ---------------------------

/// A block turned 45° about y whose edge lies in the plane `z = k` at
/// `x = k − delta`, its 90° wedge opening towards +x, straddling the
/// plane: the sliver of the cube's top face beside it reads the two
/// walls of a convex edge.
fn convex_wedge(k: f64, delta: f64) -> Body {
    let origin = Vec3::new(k - delta, -k, k);
    let frame = Frame::new(origin, Vec3::new(0.0, 1.0, 0.0), Vec3::new(1.0, 0.0, 1.0)).unwrap();
    primitives::block(&frame, 0.5 * k, 0.5 * k, 3.0 * k).unwrap()
}

/// A pentagon prism along y with a reflex edge at `(k − delta, *, k)`:
/// the notch's faces rise at ±45° from the plane `z = k`, the material
/// fills 270° around the edge: the two walls of a concave edge.
fn concave_wedge(k: f64, delta: f64) -> Body {
    let x0 = k - delta;
    let pts: [[f64; 2]; 5] = [
        [x0 - 0.5 * k, 0.5 * k],
        [x0 + 0.5 * k, 0.5 * k],
        [x0 + 0.5 * k, 1.5 * k],
        [x0, k],
        [x0 - 0.5 * k, 1.5 * k],
    ];
    let curves: Vec<SketchCurve> = (0..5)
        .map(|i| SketchCurve {
            id: i as u64 + 1,
            curve: Curve2::Line {
                start: pts[i],
                end: pts[(i + 1) % 5],
            },
        })
        .collect();
    let tol = tolerance(k);
    let profiles = detect_profiles(&curves, &tol).unwrap();
    // The profile's (u, v) are world (x, z); the extrusion runs along −y
    // from y = 2k to y = −k.
    let frame = Frame::new(
        Vec3::new(0.0, 2.0 * k, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .unwrap();
    extrude(&profiles, &frame, 0.0, 3.0 * k, &tol).unwrap().body
}

/// The sliver of the cube's top face between its edge and an edge of the
/// other body a few tolerances away, convex (a turned block) and concave
/// (a notched prism): union in both orders, difference and intersection
/// to closed-form volumes (the review's probe of 2026-10-18).
#[test]
fn sliver_beside_an_edge_of_the_other_body() {
    for k in AT {
        let tol = tolerance(k).linear;
        let cube = k.powi(3);
        for tolerances in [3.0, 5.0] {
            let delta = tolerances * tol;
            let a = boxed(k, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
            for (shape, b, volume, common) in [
                (
                    "convex",
                    convex_wedge(k, delta),
                    0.75 * cube,
                    0.5 * delta * delta * k,
                ),
                (
                    "concave",
                    concave_wedge(k, delta),
                    2.25 * cube,
                    (0.5 * k + delta) * k * 0.5 * k,
                ),
            ] {
                let what = format!("{shape} wedge at {tolerances} tolerances, scale {k}");
                let mb = mass_properties(&b).unwrap();
                assert!(
                    (mb.volume - volume).abs() <= 1e-9 * cube,
                    "{what}: B {}",
                    mb.volume
                );
                for (first, second) in [(&a, &b), (&b, &a)] {
                    let union = op(first, second, Union, k, &what);
                    assert_eq!(union.shells.len(), 1, "{what}");
                    assert_volume(&union, cube + volume - common, k, &what);
                }
                let difference = op(&a, &b, Subtract, k, &what);
                assert_eq!(difference.shells.len(), 1, "{what}");
                assert_volume(&difference, cube - common, k, &what);
                let meet = op(&a, &b, Intersect, k, &what);
                assert_eq!(meet.shells.len(), 1, "{what}");
                assert_volume(&meet, common, k, &what);
            }
        }
    }
}

// --- The joint of a riveted truss ----------------------------------------

/// The L 70 × 70 × 8 angle of the pier, in metres: `(x, y)` corners of
/// the profile, counter-clockwise.
const ANGLE: [[f64; 2]; 6] = [
    [-0.2, -0.172],
    [-0.13, -0.172],
    [-0.13, -0.164],
    [-0.192, -0.164],
    [-0.192, -0.102],
    [-0.2, -0.102],
];
const LENGTH: f64 = 10.0;
/// The pivot every bar is turned about (the middle of its length).
const PIVOT: [f64; 3] = [0.0, 0.0, 5.0];
/// The bars of the joint: turned about the y axis by these degrees. In
/// the plane `y = -0.164` the inner edge of the first bar's vertical leg,
/// the outer face of the second and the end of the third's horizontal
/// leg nearly meet in one point: 3 to 6 tolerances apart at every scale.
const JOINT: [f64; 3] = [0.0, 44.0, 77.0];

/// The angle bar extruded along z, turned by `degrees` about the y axis
/// through the pivot, times `k`.
fn angle_bar(k: f64, degrees: f64) -> Body {
    let tol = tolerance(k);
    let curves: Vec<SketchCurve> = (0..ANGLE.len())
        .map(|i| SketchCurve {
            id: i as u64 + 1,
            curve: Curve2::Line {
                start: [ANGLE[i][0] * k, ANGLE[i][1] * k],
                end: [ANGLE[(i + 1) % 6][0] * k, ANGLE[(i + 1) % 6][1] * k],
            },
        })
        .collect();
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let body = extrude(&profiles, &Frame::WORLD, 0.0, LENGTH * k, &tol)
        .unwrap()
        .body;
    let pivot = Vec3::new(PIVOT[0] * k, PIVOT[1] * k, PIVOT[2] * k);
    let turn =
        Transform::rotation_about(pivot, Vec3::new(0.0, 1.0, 0.0), degrees.to_radians()).unwrap();
    body.transformed(&turn).unwrap()
}

/// The cross-section of a bar in the plane `y = const`, as a rectangle in
/// `(x, z)`: the strip of its leg between `x0` and `x1` of the profile,
/// along the bar's length, turned about the pivot.
fn strip(k: f64, degrees: f64, x0: f64, x1: f64) -> Vec<[f64; 2]> {
    let (s, c) = degrees.to_radians().sin_cos();
    // A rotation about y by θ maps (x, z) to (x cos θ + z sin θ, −x sin θ + z cos θ)
    // about the pivot.
    let turn = |x: f64, z: f64| {
        let (dx, dz) = (x * k, (z - PIVOT[2]) * k);
        [dx * c + dz * s, -dx * s + dz * c + PIVOT[2] * k]
    };
    vec![
        turn(x0, 0.0),
        turn(x1, 0.0),
        turn(x1, LENGTH),
        turn(x0, LENGTH),
    ]
}

fn polygon_area(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        .abs()
        * 0.5
}

/// `subject` clipped to the convex polygon `clip` (Sutherland–Hodgman;
/// the side is read off the clip polygon's own orientation).
fn clip_convex(subject: &[[f64; 2]], clip: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let orient = {
        let n = clip.len();
        (0..n)
            .map(|i| clip[i][0] * clip[(i + 1) % n][1] - clip[(i + 1) % n][0] * clip[i][1])
            .sum::<f64>()
            .signum()
    };
    let mut output = subject.to_vec();
    for i in 0..clip.len() {
        let (a, b) = (clip[i], clip[(i + 1) % clip.len()]);
        let side =
            |p: [f64; 2]| ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])) * orient;
        let input = std::mem::take(&mut output);
        if input.is_empty() {
            break;
        }
        for j in 0..input.len() {
            let (p, q) = (input[j], input[(j + 1) % input.len()]);
            let (sp, sq) = (side(p), side(q));
            let cross = || {
                let t = sp / (sp - sq);
                [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]
            };
            if sp >= 0.0 {
                output.push(p);
                if sq < 0.0 {
                    output.push(cross());
                }
            } else if sq >= 0.0 {
                output.push(cross());
            }
        }
    }
    output
}

/// The area common to all of the convex polygons.
fn common_area(polygons: &[Vec<[f64; 2]>]) -> f64 {
    let mut common: Option<Vec<[f64; 2]>> = None;
    for polygon in polygons {
        common = Some(match common {
            None => polygon.clone(),
            Some(c) => clip_convex(&c, polygon),
        });
    }
    common.map(|c| polygon_area(&c)).unwrap_or(0.0)
}

/// The area of the union of convex polygons by inclusion–exclusion.
fn union_area(polygons: &[Vec<[f64; 2]>]) -> f64 {
    let n = polygons.len();
    let mut total = 0.0;
    for mask in 1u32..(1 << n) {
        let chosen: Vec<Vec<[f64; 2]>> = polygons
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, p)| p.clone())
            .collect();
        let sign = if mask.count_ones() % 2 == 1 {
            1.0
        } else {
            -1.0
        };
        total += sign * common_area(&chosen);
    }
    total
}

/// The layers of the profile in y (the horizontal leg, the vertical leg):
/// `((y0, y1), (x0, x1))`.
const LAYERS: [((f64, f64), (f64, f64)); 2] = [
    ((-0.172, -0.164), (-0.2, -0.13)),
    ((-0.164, -0.102), (-0.2, -0.192)),
];

/// A volume of the bars at `degrees` summed over the layers: `area` of
/// the strips of a layer times its height.
fn layered(k: f64, degrees: &[f64], area: impl Fn(&[Vec<[f64; 2]>]) -> f64) -> f64 {
    LAYERS
        .iter()
        .map(|&((y0, y1), (x0, x1))| {
            let strips: Vec<_> = degrees.iter().map(|&d| strip(k, d, x0, x1)).collect();
            area(&strips) * (y1 - y0) * k
        })
        .sum()
}

/// The volume of the union of the bars at `degrees`.
fn union_volume(k: f64, degrees: &[f64]) -> f64 {
    layered(k, degrees, union_area)
}

/// The volume of `(bars[0] ∪ bars[1]) ∩ bars[2]`.
fn pair_meets_third_volume(k: f64, degrees: [f64; 3]) -> f64 {
    layered(k, &degrees, |s| {
        common_area(&[s[0].clone(), s[2].clone()]) + common_area(&[s[1].clone(), s[2].clone()])
            - common_area(s)
    })
}

/// The bars united one by one in the given order, each result checked.
fn unite(bars: &[Body], order: &[usize], k: f64) -> Body {
    let mut result = bars[order[0]].clone();
    for &i in &order[1..] {
        result = op(
            &result,
            &bars[i],
            Union,
            k,
            &format!("bar {i} onto {order:?}"),
        );
    }
    result
}

/// Two bars that cross: every pair in every order, the volume by
/// inclusion–exclusion.
#[test]
fn two_crossing_angle_bars() {
    for k in AT {
        for degrees in [[0.0, 44.0], [0.0, 77.0], [44.0, 77.0]] {
            let bars = [angle_bar(k, degrees[0]), angle_bar(k, degrees[1])];
            let expected = union_volume(k, &degrees);
            for order in [[0, 1], [1, 0]] {
                let body = unite(&bars, &order, k);
                assert_eq!(body.shells.len(), 1);
                assert_volume(
                    &body,
                    expected,
                    k,
                    &format!("{degrees:?} order {order:?} at {k}"),
                );
            }
        }
    }
}

/// The joint: three bars at 0°, 44° and 77°, united in every order; the
/// same volume every time.
#[test]
fn three_crossing_angle_bars_in_every_order() {
    for k in AT {
        let bars: Vec<Body> = JOINT.iter().map(|&d| angle_bar(k, d)).collect();
        let expected = union_volume(k, &JOINT);
        for order in [
            [0, 1, 2],
            [1, 2, 0],
            [2, 0, 1],
            [0, 2, 1],
            [1, 0, 2],
            [2, 1, 0],
        ] {
            let body = unite(&bars, &order, k);
            assert_eq!(body.shells.len(), 1, "order {order:?} at {k}");
            assert_volume(&body, expected, k, &format!("order {order:?} at {k}"));
        }
    }
}

/// The third bar taken out of, and kept in common with, the union of
/// the other two: the slivers of the joint classified for a difference
/// and an intersection, held to inclusion–exclusion.
#[test]
fn third_bar_cut_from_and_common_with_the_pair() {
    for k in AT {
        let bars: Vec<Body> = JOINT.iter().map(|&d| angle_bar(k, d)).collect();
        for (p, q, r) in [(0, 1, 2), (1, 2, 0), (2, 0, 1)] {
            let degrees = [JOINT[p], JOINT[q], JOINT[r]];
            let pair = op(&bars[p], &bars[q], Union, k, "the pair");
            let common = pair_meets_third_volume(k, degrees);
            let what = format!("bars {p}, {q} with {r} at {k}");
            let meet = op(&pair, &bars[r], Intersect, k, &what);
            assert_volume(&meet, common, k, &what);
            let less = op(&pair, &bars[r], Subtract, k, &what);
            assert_volume(&less, union_volume(k, &degrees[..2]) - common, k, &what);
        }
    }
}
