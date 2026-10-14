//! Profiles with splines in them, and their extrusion into solids with
//! B-spline walls.
//!
//! The oracle is independent of the kernel: the area and the centroid of a
//! profile by Green's theorem, with the spline evaluated by de Boor's
//! algorithm on its knots and integrated by Simpson's rule on a fine
//! subdivision of every knot span (the kernel uses Bézier pieces and
//! Gauss–Legendre). The volume of the extrusion is that area times the
//! depth, its surface the two caps plus perimeter times depth.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{circle, frames, line};
use craft3d_geom::{Curve3, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::{Origin, Role, extrude};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::Tolerance;
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::PI;

type P = [f64; 2];

fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

fn spline(id: u64, degree: usize, control: &[P]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree,
            knots: knots(degree, control.len()),
            control: control.to_vec(),
        }),
    }
}

/// De Boor's algorithm: the point at knot value `u`.
fn de_boor(degree: usize, knots: &[f64], control: &[P], u: f64) -> P {
    let n = control.len();
    let k = (degree..n).rev().find(|&k| knots[k] <= u).unwrap_or(degree);
    let mut d: Vec<P> = control[k - degree..=k].to_vec();
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = k - degree + j;
            let alpha = (u - knots[i]) / (knots[i + degree + 1 - r] - knots[i]);
            d[j] = [
                d[j - 1][0] + (d[j][0] - d[j - 1][0]) * alpha,
                d[j - 1][1] + (d[j][1] - d[j - 1][1]) * alpha,
            ];
        }
    }
    d[degree]
}

/// `∫ (x dy − y dx) / 2`, the two first moments `∫ x (x dy − y dx) / 3` and
/// `∫ y (…) / 3`, and the length, along a spline from its start to its
/// end: composite Simpson on 2000 steps per knot span, with the derivative
/// from the control polygon of the derivative spline (evaluated by de Boor
/// as well).
fn along_spline(degree: usize, control: &[P]) -> [f64; 4] {
    let knots = knots(degree, control.len());
    // The derivative is a spline of one degree less on the inner knots.
    let derivative: Vec<P> = (0..control.len() - 1)
        .map(|i| {
            let scale = degree as f64 / (knots[i + degree + 1] - knots[i + 1]);
            [
                (control[i + 1][0] - control[i][0]) * scale,
                (control[i + 1][1] - control[i][1]) * scale,
            ]
        })
        .collect();
    let inner = &knots[1..knots.len() - 1];
    let at = |u: f64| {
        let p = de_boor(degree, &knots, control, u);
        let d = if degree == 1 {
            let k = (1..control.len())
                .rev()
                .find(|&k| knots[k] <= u)
                .unwrap_or(1);
            derivative[(k - 1).min(derivative.len() - 1)]
        } else {
            de_boor(degree - 1, inner, &derivative, u)
        };
        let swept = p[0] * d[1] - p[1] * d[0];
        [
            swept / 2.0,
            p[0] * swept / 3.0,
            p[1] * swept / 3.0,
            d[0].hypot(d[1]),
        ]
    };
    let mut sum = [0.0; 4];
    let spans = control.len() - degree;
    let steps = 2000;
    for span in 0..spans {
        let (a, b) = (knots[degree + span], knots[degree + span + 1]);
        let h = (b - a) / steps as f64;
        for i in 0..=steps {
            // Just inside the span at its ends, so de Boor (and a
            // piecewise-constant derivative) stay on this span's side.
            let u = (a + h * i as f64).clamp(a + (b - a) * 1e-14, b - (b - a) * 1e-14);
            let weight = if i == 0 || i == steps {
                1.0
            } else if i % 2 == 1 {
                4.0
            } else {
                2.0
            };
            let value = at(u);
            for k in 0..4 {
                sum[k] += value[k] * weight * h / 3.0;
            }
        }
    }
    sum
}

/// The same four numbers along a straight line.
fn along_line(a: P, b: P) -> [f64; 4] {
    let w = a[0] * b[1] - a[1] * b[0];
    [
        w / 2.0,
        (a[0] + b[0]) * w / 6.0,
        (a[1] + b[1]) * w / 6.0,
        (b[0] - a[0]).hypot(b[1] - a[1]),
    ]
}

fn add(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

fn scaled(points: &[P], k: f64) -> Vec<P> {
    points.iter().map(|p| [p[0] * k, p[1] * k]).collect()
}

/// A profile for the tests: its curves, and by the oracle its area, the
/// centroid of the area and the length of its boundary.
struct Case {
    name: &'static str,
    curves: Vec<SketchCurve>,
    area: f64,
    centroid: P,
    perimeter: f64,
    /// Side faces on B-spline surfaces.
    spline_sides: usize,
}

/// The bulge of the "D": from (0,0) out to the right and back to (0,6).
const BULGE: [P; 5] = [[0.0, 0.0], [5.0, -1.0], [7.0, 3.0], [5.0, 7.0], [0.0, 6.0]];
/// A leaf of two splines between (0,0) and (10,0): the upper side, drawn
/// from left to right, and the lower side, also drawn from left to right
/// (so the loop runs against one of them).
const UPPER: [P; 6] = [
    [0.0, 0.0],
    [1.0, 3.0],
    [4.0, 4.0],
    [7.0, 2.5],
    [9.0, 2.0],
    [10.0, 0.0],
];
const LOWER: [P; 4] = [[0.0, 0.0], [3.0, -2.0], [8.0, -1.5], [10.0, 0.0]];

fn cases(k: f64) -> Vec<Case> {
    let mut cases = Vec::new();
    for degree in [2, 3, 4] {
        // The D: a line down the left side, drawn from the top.
        let bulge = scaled(&BULGE, k);
        let (top, bottom) = (bulge[4], bulge[0]);
        let sums = add(along_spline(degree, &bulge), along_line(top, bottom));
        cases.push(Case {
            name: match degree {
                2 => "D, degree 2",
                3 => "D, degree 3",
                _ => "D, degree 4",
            },
            curves: vec![spline(7, degree, &bulge), line(9, top, bottom)],
            area: sums[0],
            centroid: [sums[1] / sums[0], sums[2] / sums[0]],
            perimeter: sums[3],
            spline_sides: 1,
        });
    }
    // The leaf: counter-clockwise it runs along the lower side and back
    // against the upper one.
    let (upper, lower) = (scaled(&UPPER, k), scaled(&LOWER, k));
    let (up, low) = (along_spline(3, &upper), along_spline(2, &lower));
    let leaf = [
        low[0] - up[0],
        low[1] - up[1],
        low[2] - up[2],
        low[3] + up[3],
    ];
    cases.push(Case {
        name: "leaf of two splines",
        curves: vec![spline(3, 3, &upper), spline(4, 2, &lower)],
        area: leaf[0],
        centroid: [leaf[1] / leaf[0], leaf[2] / leaf[0]],
        perimeter: leaf[3],
        spline_sides: 2,
    });
    // The leaf with a round hole, and a triangular island in the hole's
    // place would be another profile: here just the hole.
    let (c, r) = ([5.0 * k, 1.0 * k], 1.2 * k);
    let hole = PI * r * r;
    let area = leaf[0] - hole;
    cases.push(Case {
        name: "leaf with a round hole",
        curves: vec![spline(3, 3, &upper), spline(4, 2, &lower), circle(8, c, r)],
        area,
        centroid: [
            (leaf[1] - hole * c[0]) / area,
            (leaf[2] - hole * c[1]) / area,
        ],
        perimeter: leaf[3] + 2.0 * PI * r,
        spline_sides: 2,
    });
    cases
}

fn close(a: f64, b: f64, relative: f64, what: &str) {
    assert!(
        (a - b).abs() <= relative * b.abs(),
        "{what}: {a} against {b} (off by {:e})",
        ((a - b) / b).abs()
    );
}

#[test]
fn the_area_and_centroid_of_a_spline_profile_are_exact() {
    for k in [1e-3, 1.0, 1e3] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for case in cases(k) {
            let name = format!("{} x{k}", case.name);
            let profiles = detect_profiles(&case.curves, &tol).unwrap();
            assert_eq!(profiles.len(), 1, "{name}");
            let profile = &profiles[0];
            close(profile.area(), case.area, 1e-11, &name);
            close(
                profile.outer.signed_area(),
                profile.outer.signed_area().abs(),
                0.0,
                &name,
            );
            let centroid = profile.centroid().unwrap();
            for axis in 0..2 {
                assert!(
                    (centroid[axis] - case.centroid[axis]).abs() <= 1e-10 * k,
                    "{name}: centroid {centroid:?} against {:?}",
                    case.centroid
                );
            }
            // The order the curves are given in does not matter.
            let mut backwards = case.curves.clone();
            backwards.reverse();
            assert_eq!(
                detect_profiles(&backwards, &tol).unwrap(),
                profiles,
                "{name}"
            );
        }
    }
}

#[test]
fn a_spline_profile_extrudes_into_a_checked_solid_with_spline_walls() {
    for k in [1e-3, 1.0, 1e3] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for case in cases(k) {
            for (f, frame) in frames(k).iter().enumerate() {
                let name = format!("{} x{k} frame {f}", case.name);
                let profiles = detect_profiles(&case.curves, &tol).unwrap();
                let (from, to) = (-0.5 * k, 2.0 * k);
                let result = extrude(&profiles, frame, from, to, &tol)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                let body = &result.body;
                assert_eq!(check(body, &tol), [], "{name}");
                let nurbs_faces = body
                    .faces
                    .iter()
                    .filter(|face| matches!(face.surface, Surface::Nurbs(_)))
                    .count();
                assert_eq!(nurbs_faces, case.spline_sides, "{name}");
                // Each spline gives an edge at the start and one at the end.
                let nurbs_edges = body
                    .edges
                    .iter()
                    .filter(|edge| matches!(edge.curve, Curve3::Nurbs(_)))
                    .count();
                assert_eq!(nurbs_edges, 2 * case.spline_sides, "{name}");

                let depth = to - from;
                let mass = mass_properties(body).unwrap_or_else(|e| panic!("{name}: {e}"));
                close(
                    mass.volume,
                    case.area * depth,
                    1e-9,
                    &format!("{name} volume"),
                );
                close(
                    mass.area,
                    2.0 * case.area + case.perimeter * depth,
                    1e-9,
                    &format!("{name} area"),
                );
                let centre = mass.centroid.unwrap();
                let expected = frame.point(case.centroid[0], case.centroid[1], (from + to) / 2.0);
                assert!(
                    centre.distance(expected) <= 1e-8 * k,
                    "{name}: centroid {centre:?} against {expected:?}"
                );

                // The mesh is closed and converges on the volume.
                let chord = 2e-3 * k;
                let params = TessParams {
                    chordal_tolerance: chord,
                    ..TessParams::default()
                };
                let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{name}: {e}"));
                let welded = mesh.weld().unwrap_or_else(|e| panic!("{name}: {e:?}"));
                assert!(welded.is_closed(), "{name}: the mesh is not watertight");
                // A mesh within `chord` of the surface encloses the volume
                // to within the area times the chord.
                assert!(
                    (welded.volume() - case.area * depth).abs() <= mass.area * chord,
                    "{name}: mesh volume {} against {}",
                    welded.volume(),
                    case.area * depth
                );
                for face in 0..body.faces.len() {
                    assert!(
                        mesh.face_ids.contains(&(face as u32)),
                        "{name}: face {face}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_wall_of_a_spline_is_named_after_it_and_points_outwards() {
    let tol = Tolerance::default();
    let case = cases(1.0).remove(4);
    let profiles = detect_profiles(&case.curves, &tol).unwrap();
    let frame = &frames(1.0)[1];
    let result = extrude(&profiles, frame, 0.0, 3.0, &tol).unwrap();
    let origin = |role: Role, sources: &[u64]| Origin::new(role, sources);
    let faces = &result.origins.faces;
    assert_eq!(
        faces,
        &[
            origin(Role::Side, &[3]),
            origin(Role::Side, &[4]),
            origin(Role::Side, &[8]),
            origin(Role::StartCap, &[3, 4]),
            origin(Role::EndCap, &[3, 4]),
        ]
    );
    let count = |wanted: &Origin| result.origins.edges.iter().filter(|o| *o == wanted).count();
    for id in [3, 4, 8] {
        assert_eq!(count(&origin(Role::StartEdge, &[id])), 1);
        assert_eq!(count(&origin(Role::EndEdge, &[id])), 1);
    }
    assert_eq!(count(&origin(Role::LateralEdge, &[3, 4])), 1);
    assert_eq!(count(&origin(Role::LateralEdge, &[4, 3])), 1);
    // The loop runs counter-clockwise: along the lower spline (4) and
    // against the upper one (3), whose wall is therefore reversed. Either
    // way the normal of the face points away from the inside of the leaf.
    let inside = frame.point(5.0, 2.5, 1.5);
    for (index, expect_reversed) in [(0, true), (1, false)] {
        let face = &result.body.faces[index];
        assert_eq!(face.reversed, expect_reversed, "face {index}");
        let ((u0, u1), (v0, v1)) = face.surface.domain();
        let (u, v) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
        let mut normal = face.surface.normal(u, v).unwrap();
        if face.reversed {
            normal = -normal;
        }
        let outwards = face.surface.eval(u, v) - inside;
        assert!(normal.dot(outwards) > 0.0, "face {index}");
        assert!(normal.dot(frame.z()).abs() < 1e-12, "face {index}");
    }
    // Drawing a spline the other way round turns its wall around, and the
    // solid is the same size.
    let mut reversed_upper: Vec<P> = UPPER.to_vec();
    reversed_upper.reverse();
    let mut curves = case.curves.clone();
    curves[0] = spline(3, 3, &reversed_upper);
    let other = extrude(
        &detect_profiles(&curves, &tol).unwrap(),
        frame,
        0.0,
        3.0,
        &tol,
    )
    .unwrap();
    assert_eq!(other.origins, result.origins);
    assert!(!other.body.faces[0].reversed);
    let (a, b) = (
        mass_properties(&result.body).unwrap(),
        mass_properties(&other.body).unwrap(),
    );
    close(a.volume, b.volume, 1e-12, "volume drawn either way");
}

#[test]
fn splines_that_cross_touch_or_fold_are_refused() {
    for k in [1e-3, 1.0, 1e3] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let at = |points: &[P]| scaled(points, k);
        let crossing = |curves: &[SketchCurve], a: u64, b: u64, what: &str| {
            assert_eq!(
                detect_profiles(curves, &tol),
                Err(KernelError::CrossingCurves { curves: [a, b] }),
                "{what} x{k}"
            );
        };
        let bulge = at(&BULGE);
        let (top, bottom) = (bulge[4], bulge[0]);
        // A spline that loops over itself on the way.
        let looped = at(&[[0.0, 0.0], [8.0, 6.0], [8.0, -2.0], [3.0, 3.0], [0.0, 6.0]]);
        crossing(
            &[spline(7, 3, &looped), line(9, top, bottom)],
            7,
            7,
            "a spline with a loop in it",
        );
        // A spline that crosses the line that closes it.
        let through = at(&[[0.0, 0.0], [5.0, 1.0], [-10.0, 2.0], [9.0, 5.0], [0.0, 6.0]]);
        crossing(
            &[spline(7, 3, &through), line(9, top, bottom)],
            7,
            9,
            "a spline through its closing line",
        );
        // A circle that cuts the bulge, one that touches it within
        // tolerance from inside, and one that lies wholly outside the D
        // (a second profile, not an error).
        let d = [spline(7, 3, &bulge), line(9, top, bottom)];
        let with = |c: P, r: f64| {
            let mut curves = d.to_vec();
            curves.push(circle(12, [c[0] * k, c[1] * k], r * k));
            curves
        };
        crossing(&with([5.0, 3.0], 1.5), 7, 12, "a circle across the spline");
        // The bulge of degree 3 is furthest right at y = 3 by symmetry.
        let rightmost = de_boor(3, &knots(3, 5), &BULGE, 0.5)[0];
        crossing(
            &with([rightmost - 1.0 + 4e-7, 3.0], 1.0),
            7,
            12,
            "a circle touching the spline",
        );
        assert_eq!(
            detect_profiles(&with([rightmost - 1.5, 3.0], 1.0), &tol).unwrap()[0]
                .holes
                .len(),
            1
        );
        assert_eq!(
            detect_profiles(&with([20.0, 3.0], 1.0), &tol)
                .unwrap()
                .len(),
            2
        );
        // Two splines that cross each other between their shared ends.
        let (upper, wavy) = (
            at(&UPPER),
            at(&[
                [0.0, 0.0],
                [3.0, -2.0],
                [5.0, 9.0],
                [8.0, -1.5],
                [10.0, 0.0],
            ]),
        );
        crossing(
            &[spline(3, 3, &upper), spline(4, 3, &wavy)],
            3,
            4,
            "two splines that cross",
        );
        // The same spline twice encloses nothing.
        assert_eq!(
            detect_profiles(&[spline(3, 3, &upper), spline(4, 3, &upper)], &tol),
            Err(KernelError::ZeroAreaLoop { curves: vec![3, 4] })
        );
        // A spline with a cusp.
        let cusp = at(&[[0.0, 0.0], [6.0, 6.0], [0.0, 6.0], [6.0, 0.0]]);
        crossing(
            &[spline(7, 3, &cusp), line(9, cusp[3], cusp[0])],
            7,
            7,
            "a spline with a cusp",
        );
    }
}

#[test]
fn a_spline_that_cannot_bound_a_profile_says_why() {
    let tol = Tolerance::default();
    let reason = |curves: &[SketchCurve]| match detect_profiles(curves, &tol) {
        Err(KernelError::DegenerateCurve { id: 7, reason }) => reason,
        other => panic!("{other:?}"),
    };
    let closing = line(9, BULGE[4], BULGE[0]);
    // (A spline that closes on itself is a loop since 2026-10-14:
    // `closed_splines.rs`.)
    // No length, a broken knot vector, a coordinate that is not a number.
    let dot = [[1.0, 1.0], [1.0, 1.0 + 1e-8], [1.0 + 1e-8, 1.0]];
    assert!(reason(&[spline(7, 2, &dot), closing.clone()]).contains("no length"));
    let mut broken = spline(7, 3, &BULGE);
    if let Curve2::Spline(s) = &mut broken.curve {
        s.knots.pop();
    }
    assert!(reason(&[broken, closing.clone()]).contains("knot"));
    for bad in [f64::NAN, f64::INFINITY, 1e200] {
        let mut points = BULGE.to_vec();
        points[2][1] = bad;
        assert!(reason(&[spline(7, 3, &points), closing.clone()]).contains("not finite"));
    }
    for degree in [0, 5, 40] {
        let fault = reason(&[spline_unchecked(7, degree, &BULGE), closing.clone()]);
        assert!(
            fault.contains("degree") || fault.contains("control points"),
            "{fault}"
        );
    }
    // An open spline is an open profile; a spline that ends near nothing.
    assert_eq!(
        detect_profiles(&[spline(7, 3, &BULGE)], &tol),
        Err(KernelError::OpenProfile { curves: vec![7] })
    );
    // Flat: the spline lies on the line that closes it.
    let flat = [[0.0, 0.0], [0.0, 2.0], [0.0, 4.0], [0.0, 6.0]];
    assert!(detect_profiles(&[spline(7, 3, &flat), closing], &tol).is_err());
}

/// A spline whose knots fit whatever degree it is given, valid or not.
fn spline_unchecked(id: u64, degree: usize, control: &[P]) -> SketchCurve {
    let count = control.len() + degree + 1;
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree,
            knots: (0..count)
                .map(|i| if i < count / 2 { 0.0 } else { 1.0 })
                .collect(),
            control: control.to_vec(),
        }),
    }
}

#[test]
fn ends_that_meet_within_tolerance_are_made_one_point() {
    let tol = Tolerance::default();
    // The line's ends are a fraction of the tolerance off the spline's.
    let closing = line(9, [3e-7, 6.0 - 2e-7], [-2e-7, 1e-7]);
    let profiles = detect_profiles(&[spline(7, 3, &BULGE), closing], &tol).unwrap();
    let segments = &profiles[0].outer.segments;
    assert_eq!(segments.len(), 2);
    for (segment, next) in segments.iter().zip(segments.iter().cycle().skip(1)) {
        assert_eq!(segment.end, next.start);
    }
    let result = extrude(&profiles, &frames(1.0)[0], 0.0, 1.0, &tol).unwrap();
    assert_eq!(check(&result.body, &tol), []);
    // An arc's end wins over a spline's: the spline's end control point is
    // moved onto it.
    let arc = SketchCurve {
        id: 2,
        curve: Curve2::Arc {
            center: [0.0, 3.0],
            start: [1e-7, 6.0 + 2e-7],
            end: [-1e-7, -2e-7],
        },
    };
    let profiles = detect_profiles(&[spline(7, 3, &BULGE), arc], &tol).unwrap();
    let segments = &profiles[0].outer.segments;
    let spline_segment = segments.iter().find(|s| s.source == 7).unwrap();
    assert_eq!(spline_segment.end, [1e-7, 6.0 + 2e-7]);
    assert_eq!(spline_segment.start, [-1e-7, -2e-7]);
    let result = extrude(&profiles, &frames(1.0)[0], 0.0, 1.0, &tol).unwrap();
    assert_eq!(check(&result.body, &tol), []);
}

/// Since 2026-10-14 a spline is revolved (`spline_revolve.rs` has the
/// tests); until then this test pinned the refusal.
#[test]
fn a_spline_profile_is_revolved() {
    let tol = Tolerance::default();
    let case = cases(1.0).remove(1);
    let profiles = detect_profiles(&case.curves, &tol).unwrap();
    // About the y axis, which the D's straight side lies on: by Pappus the
    // volume is the angle times the area times the distance of its
    // centroid from the axis.
    let result = revolve(
        &profiles,
        &frames(1.0)[0],
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        1.0,
        &tol,
    )
    .unwrap();
    assert_eq!(check(&result.body, &tol), []);
    let chord = 1e-3;
    let params = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let welded = tessellate_body(&result.body, &params)
        .unwrap()
        .weld()
        .unwrap();
    assert!(welded.is_closed());
    let volume = case.area * case.centroid[0];
    assert!(
        (welded.volume() - volume).abs() <= 400.0 * chord,
        "{} against {volume}",
        welded.volume()
    );
}

#[test]
fn a_spline_profile_is_the_same_every_time() {
    let tol = Tolerance::default();
    for case in cases(1.0) {
        let build = || {
            let profiles = detect_profiles(&case.curves, &tol).unwrap();
            extrude(&profiles, &frames(1.0)[1], -1.0, 2.0, &tol).unwrap()
        };
        let (a, b) = (build(), build());
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "{}", case.name);
    }
}

/// Random loops of lines and splines through random corners: whatever is
/// accepted is a solid that passes the checker, has the volume of its
/// profile and meshes watertight; whatever is not is an error. Nothing
/// panics, hostile numbers included.
#[test]
fn random_spline_loops_give_valid_solids_or_errors() {
    use common::Rng;
    let tol = Tolerance::default();
    let mut rng = Rng(0x0591_17E5);
    let hostile = [f64::NAN, f64::INFINITY, -1e300, 1e-320, 0.0, -0.0];
    let (mut accepted, mut meshed, mut refused, mut declined, mut too_fine) = (0, 0, 0, 0, 0);
    let mut spent = [std::time::Duration::ZERO; 3];
    let mut measured_count = 0;
    for round in 0..300 {
        let corners = 2 + rng.below(3) as usize;
        // Corners roughly on a circle, so that many loops are simple.
        let corner: Vec<P> = (0..corners)
            .map(|i| {
                let angle = (i as f64 + rng.unit() * 0.6) / corners as f64 * 2.0 * PI;
                let radius = 4.0 + rng.unit() * 4.0;
                [radius * angle.cos(), radius * angle.sin()]
            })
            .collect();
        let mut curves = Vec::new();
        for i in 0..corners {
            let (a, b) = (corner[i], corner[(i + 1) % corners]);
            let id = 10 + i as u64;
            if corners > 2 && rng.below(3) == 0 {
                curves.push(line(id, a, b));
                continue;
            }
            let degree = 2 + rng.below(4) as usize;
            let inner = degree - 1 + rng.below(4) as usize;
            let mut control = vec![a];
            for j in 0..inner {
                let t = (j + 1) as f64 / (inner + 1) as f64;
                // Around the chord, pushed outwards, sometimes wildly.
                let wild = if rng.below(8) == 0 { 12.0 } else { 3.0 };
                let mut p = [
                    a[0] + (b[0] - a[0]) * t + (rng.unit() - 0.3) * wild * a[0].signum(),
                    a[1] + (b[1] - a[1]) * t + (rng.unit() - 0.3) * wild * a[1].signum(),
                ];
                if rng.below(400) == 0 {
                    p[rng.below(2) as usize] = hostile[rng.below(hostile.len() as u64) as usize];
                }
                control.push(p);
            }
            control.push(b);
            // Drawn either way round.
            if rng.below(2) == 0 {
                control.reverse();
            }
            curves.push(spline(id, degree, &control));
        }
        let clock = std::time::Instant::now();
        let detected = detect_profiles(&curves, &tol);
        spent[0] += clock.elapsed();
        let Ok(profiles) = detected else {
            refused += 1;
            continue;
        };
        let area: f64 = profiles.iter().map(|p| p.area()).sum();
        assert!(area > 0.0 && area.is_finite(), "round {round}: {curves:?}");
        let result = match extrude(&profiles, &frames(1.0)[round % 2], 0.0, 1.5, &tol) {
            Ok(result) => result,
            Err(error) => panic!("round {round}: {error}\n{curves:?}"),
        };
        accepted += 1;
        assert_eq!(check(&result.body, &tol), [], "round {round}: {curves:?}");
        // The exact integrals may decline a wall whose spline all but
        // stops on the way (a hairpin just short of a cusp: the surface's
        // parameters nearly collapse there). That is an error that says
        // so, never a wrong number; it has to stay rare, and the mesh then
        // stands in for the volume.
        // (Every sixteenth solid: the exact integrals take seconds on a wall
        // that wiggles, the mesh below a fraction of one.)
        if accepted % 16 != 0 && accepted % 4 != 0 {
            continue;
        }
        let clock = std::time::Instant::now();
        let measured = if accepted % 16 == 0 {
            measured_count += 1;
            mass_properties(&result.body)
        } else {
            Ok(craft3d_topo::MassProperties {
                area: 0.0,
                volume: area * 1.5,
                centroid: None,
            })
        };
        spent[1] += clock.elapsed();
        let exact = match measured {
            Ok(mass) => {
                assert!(
                    (mass.volume - area * 1.5).abs() <= 1e-9 * area * 1.5,
                    "round {round}: volume {} against {}\n{curves:?}",
                    mass.volume,
                    area * 1.5
                );
                true
            }
            Err(craft3d_topo::MassError::Unsupported { .. }) => {
                declined += 1;
                false
            }
            Err(error) => panic!("round {round}: {error}\n{curves:?}"),
        };
        if accepted % 4 == 0 || !exact {
            let params = TessParams {
                chordal_tolerance: 0.01,
                ..TessParams::default()
            };
            // The tessellator, too, may decline a hairpin: it says how
            // many segments the tolerance would take, or that it lost
            // track of the boundary where the two legs run close.
            let clock = std::time::Instant::now();
            let tessellated = tessellate_body(&result.body, &params);
            spent[2] += clock.elapsed();
            let mesh = match tessellated {
                Ok(mesh) => mesh,
                Err(
                    craft3d_tess::TessError::TooFine { .. } | craft3d_tess::TessError::Face { .. },
                ) => {
                    too_fine += 1;
                    continue;
                }
                Err(error) => panic!("round {round}: {error}\n{curves:?}"),
            };
            let welded = mesh.weld().unwrap();
            assert!(welded.is_closed(), "round {round}: {curves:?}");
            // Within the chord of the surface: the volume is off by less
            // than the chord times the area of the walls and caps.
            let surface = 2.0 * area + 1.5 * 200.0;
            assert!(
                (welded.volume() - area * 1.5).abs() <= surface * 0.01,
                "round {round}: mesh volume {} against {}",
                welded.volume(),
                area * 1.5
            );
            meshed += 1;
        }
    }
    println!(
        "{accepted} accepted, {refused} refused, {meshed} meshed, {measured_count} measured, {declined} declined by the exact integrals, {too_fine} too fine to mesh; profiles {:?}, integrals {:?}, meshes {:?}",
        spent[0], spent[1], spent[2]
    );
    // Both outcomes are common, so both are tested.
    assert!(accepted > 80, "{accepted} accepted, {refused} refused");
    assert!(refused > 80, "{accepted} accepted, {refused} refused");
    assert!(meshed > 20, "{meshed} meshed");
    assert!(measured_count > 5, "{measured_count} measured");
    assert!(
        declined * 10 < measured_count,
        "{declined} of {measured_count} declined"
    );
    assert!(
        too_fine * 10 < meshed,
        "{too_fine} too fine, {meshed} meshed"
    );
}

/// A loop of splines inside a polygon is a hole in it: its walls face the
/// other way, and an island inside the hole is a profile of its own.
#[test]
fn a_loop_of_splines_is_a_hole_in_the_profile_around_it() {
    for k in [1e-3, 1.0, 1e3] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let (upper, lower) = (scaled(&UPPER, k), scaled(&LOWER, k));
        let leaf = along_spline(2, &lower)[0] - along_spline(3, &upper)[0];
        let corners = scaled(&[[-2.0, -4.0], [12.0, -4.0], [12.0, 6.0], [-2.0, 6.0]], k);
        let mut curves = vec![spline(3, 3, &upper), spline(4, 2, &lower)];
        for i in 0..4 {
            curves.push(line(20 + i as u64, corners[i], corners[(i + 1) % 4]));
        }
        // An island in the hole: a small circle inside the leaf.
        let radius = 0.8 * k;
        curves.push(circle(30, [5.0 * k, 1.0 * k], radius));
        let profiles = detect_profiles(&curves, &tol).unwrap();
        assert_eq!(profiles.len(), 2, "x{k}");
        let plate = profiles
            .iter()
            .find(|p| p.outer.segments.len() == 4)
            .unwrap();
        assert_eq!(plate.holes.len(), 1, "x{k}");
        let plate_area = 14.0 * 10.0 * k * k - leaf;
        close(plate.area(), plate_area, 1e-11, &format!("x{k} plate"));
        let island = PI * radius * radius;
        let result = extrude(&profiles, &frames(k)[1], 0.0, 2.0 * k, &tol).unwrap();
        assert_eq!(check(&result.body, &tol), [], "x{k}");
        assert_eq!(result.body.shells.len(), 2, "x{k}");
        let mass = mass_properties(&result.body).unwrap();
        close(
            mass.volume,
            (plate_area + island) * 2.0 * k,
            1e-9,
            &format!("x{k} volume"),
        );
        // The walls of the hole face into it: towards the middle of the
        // leaf, where the island stands.
        let inside = frames(k)[1].point(5.0 * k, 2.5 * k, k);
        for (face, origin) in result.body.faces.iter().zip(&result.origins.faces) {
            if origin.role != Role::Side || !matches!(face.surface, Surface::Nurbs(_)) {
                continue;
            }
            let ((u0, u1), (v0, v1)) = face.surface.domain();
            let (u, v) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
            let mut normal = face.surface.normal(u, v).unwrap();
            if face.reversed {
                normal = -normal;
            }
            let towards = inside - face.surface.eval(u, v);
            assert!(normal.dot(towards) > 0.0, "x{k}: {origin:?}");
        }
        let params = TessParams {
            chordal_tolerance: 2e-3 * k,
            ..TessParams::default()
        };
        let welded = tessellate_body(&result.body, &params)
            .unwrap()
            .weld()
            .unwrap();
        assert!(welded.is_closed(), "x{k}");
    }
}
