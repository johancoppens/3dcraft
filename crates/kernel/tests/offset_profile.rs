//! The offset of closed profiles against closed forms: Steiner's formula
//! for round offsets of convex polygons, the mitre formula for sharp ones,
//! inward offsets of rectangles, slots and rounded rectangles, holes, a
//! C-shape that splits and one that vanishes, at three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::offset_profile::{
    Corners, OffsetOutline, OffsetSource, offset_profiles, steiner_area,
};
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_math::Tolerance;
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

fn tol(scale: f64) -> Tolerance {
    Tolerance::new(1e-7 * scale.max(1e-2), 1e-9).unwrap()
}

fn line(id: u64, a: [f64; 2], b: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start: a, end: b },
    }
}

fn arc(id: u64, c: [f64; 2], a: [f64; 2], b: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Arc {
            center: c,
            start: a,
            end: b,
        },
    }
}

fn polygon(points: &[[f64; 2]], first_id: u64) -> Vec<SketchCurve> {
    (0..points.len())
        .map(|i| {
            line(
                first_id + i as u64,
                points[i],
                points[(i + 1) % points.len()],
            )
        })
        .collect()
}

fn profiles(curves: &[SketchCurve], t: &Tolerance) -> Vec<Profile> {
    detect_profiles(curves, t).unwrap()
}

fn area(o: &OffsetOutline) -> f64 {
    o.profiles.iter().map(Profile::area).sum()
}

fn close(a: f64, b: f64, rel: f64) {
    assert!(
        (a - b).abs() <= rel * b.abs().max(1e-300),
        "{a} is not {b} (relative {})",
        (a - b) / b
    );
}

/// A convex pentagon, its area and perimeter, and the turning angles.
fn pentagon(s: f64) -> (Vec<[f64; 2]>, f64, f64, Vec<f64>) {
    let p: Vec<[f64; 2]> = [[0.0, 0.0], [4.0, 0.0], [5.0, 2.0], [2.0, 4.0], [-1.0, 2.5]]
        .iter()
        .map(|q| [q[0] * s, q[1] * s])
        .collect();
    let n = p.len();
    let mut area = 0.0;
    let mut perimeter = 0.0;
    let mut turns = Vec::new();
    for i in 0..n {
        let (a, b, c) = (p[i], p[(i + 1) % n], p[(i + 2) % n]);
        area += (a[0] * b[1] - a[1] * b[0]) / 2.0;
        perimeter += (b[0] - a[0]).hypot(b[1] - a[1]);
        let u = [b[0] - a[0], b[1] - a[1]];
        let v = [c[0] - b[0], c[1] - b[1]];
        turns.push((u[0] * v[1] - u[1] * v[0]).atan2(u[0] * v[0] + u[1] * v[1]));
    }
    (p, area, perimeter, turns)
}

#[test]
fn round_offset_of_a_convex_polygon_is_steiner() {
    for s in SCALES {
        let t = tol(s);
        let (p, a, per, _) = pentagon(s);
        let input = profiles(&polygon(&p, 1), &t);
        for d in [0.01 * s, 0.3 * s, 2.0 * s] {
            let o = offset_profiles(&input, d, Corners::Round, &t).unwrap();
            close(area(&o), steiner_area(a, per, d), 1e-12);
            assert_eq!(o.curves.len(), 10, "five lines and five corners");
            assert_eq!((o.loops_before, o.loops_after), (1, 1));
            let corners = o
                .origins
                .iter()
                .filter(|o| matches!(o.source, OffsetSource::Corner(..)))
                .count();
            assert_eq!(corners, 5);
        }
    }
}

#[test]
fn sharp_offset_of_a_convex_polygon_is_the_mitre_form() {
    for s in SCALES {
        let t = tol(s);
        let (p, a, per, turns) = pentagon(s);
        let input = profiles(&polygon(&p, 1), &t);
        for d in [0.01 * s, 0.3 * s, 2.0 * s] {
            let o = offset_profiles(&input, d, Corners::Sharp, &t).unwrap();
            let mitre: f64 = turns.iter().map(|k| (k / 2.0).tan()).sum();
            close(area(&o), a + per * d + d * d * mitre, 1e-12);
            // Lines are lengthened: no corner pieces.
            assert_eq!(o.curves.len(), 5);
        }
    }
}

#[test]
fn inward_offset_of_a_rectangle_and_of_a_slot() {
    for s in SCALES {
        let t = tol(s);
        let (w, h) = (10.0 * s, 4.0 * s);
        let rect = profiles(&polygon(&[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]], 1), &t);
        for corners in [Corners::Round, Corners::Sharp] {
            let d = 0.5 * s;
            let o = offset_profiles(&rect, -d, corners, &t).unwrap();
            close(area(&o), (w - 2.0 * d) * (h - 2.0 * d), 1e-12);
            assert_eq!(o.curves.len(), 4);
        }
        // A slot: two half circles of radius r joined by lines.
        let (l, r) = (6.0 * s, 1.5 * s);
        let slot = vec![
            line(1, [0.0, -r], [l, -r]),
            arc(2, [l, 0.0], [l, -r], [l, r]),
            line(3, [l, r], [0.0, r]),
            arc(4, [0.0, 0.0], [0.0, r], [0.0, -r]),
        ];
        let slot = profiles(&slot, &t);
        for d in [-0.5 * s, 0.7 * s] {
            let o = offset_profiles(&slot, d, Corners::Round, &t).unwrap();
            let rr = r + d;
            close(area(&o), 2.0 * rr * l + PI * rr * rr, 1e-12);
            assert_eq!(o.curves.len(), 4, "smooth joints add nothing");
        }
        // Inward past the radius: nothing is left.
        let err = offset_profiles(&slot, -1.6 * s, Corners::Round, &t).unwrap_err();
        let KernelError::OffsetProfile { largest, .. } = err else {
            panic!("{err:?}");
        };
        // An arc whose radius is no longer above the tolerance is left
        // out, so the largest is the radius to within the tolerance.
        let largest = largest.unwrap();
        assert!(
            largest < 0.0 && (largest + r).abs() <= 2.0 * t.linear,
            "{largest}"
        );
    }
}

#[test]
fn rounded_corners_whose_radius_reaches_zero_are_removed() {
    for s in SCALES {
        let t = tol(s);
        let (w, h, r) = (10.0 * s, 6.0 * s, 1.0 * s);
        let curves = vec![
            line(1, [r, 0.0], [w - r, 0.0]),
            arc(2, [w - r, r], [w - r, 0.0], [w, r]),
            line(3, [w, r], [w, h - r]),
            arc(4, [w - r, h - r], [w, h - r], [w - r, h]),
            line(5, [w - r, h], [r, h]),
            arc(6, [r, h - r], [r, h], [0.0, h - r]),
            line(7, [0.0, h - r], [0.0, r]),
            arc(8, [r, r], [0.0, r], [r, 0.0]),
        ];
        let input = profiles(&curves, &t);
        // Within the radius: the arcs shrink, the area is closed form.
        let d = 0.4 * s;
        let o = offset_profiles(&input, -d, Corners::Round, &t).unwrap();
        let rr = r - d;
        close(
            area(&o),
            (w - 2.0 * d) * (h - 2.0 * d) - (4.0 - PI) * rr * rr,
            1e-12,
        );
        // Past it: the arcs go, the lines meet in sharp corners.
        let d = 1.5 * s;
        for corners in [Corners::Round, Corners::Sharp] {
            let o = offset_profiles(&input, -d, corners, &t).unwrap();
            close(area(&o), (w - 2.0 * d) * (h - 2.0 * d), 1e-12);
            assert_eq!(o.curves.len(), 4);
        }
    }
}

#[test]
fn holes_go_the_other_way() {
    for s in SCALES {
        let t = tol(s);
        let w = 10.0 * s;
        let r = 2.0 * s;
        let mut curves = polygon(&[[0.0, 0.0], [w, 0.0], [w, w], [0.0, w]], 1);
        curves.push(SketchCurve {
            id: 9,
            curve: Curve2::Circle {
                center: [w / 2.0, w / 2.0],
                radius: r,
            },
        });
        let input = profiles(&curves, &t);
        let d = 0.5 * s;
        let o = offset_profiles(&input, d, Corners::Round, &t).unwrap();
        close(
            area(&o),
            steiner_area(w * w, 4.0 * w, d) - PI * (r - d) * (r - d),
            1e-12,
        );
        let o = offset_profiles(&input, -d, Corners::Round, &t).unwrap();
        close(
            area(&o),
            (w - 2.0 * d) * (w - 2.0 * d) - PI * (r + d) * (r + d),
            1e-12,
        );
        // The hole closes: refused, with the radius as the largest.
        let err = offset_profiles(&input, 2.5 * s, Corners::Round, &t).unwrap_err();
        let KernelError::OffsetProfile {
            largest, curves, ..
        } = err
        else {
            panic!("{err:?}");
        };
        assert_eq!(curves, vec![9]);
        assert!((largest.unwrap() - r).abs() <= 2.0 * t.linear);
    }
}

/// A C: a rectangle with a slot cut into it from the right, leaving two
/// arms of width `arm` joined by a back of width `back`.
fn c_shape(s: f64, arm: f64, back: f64) -> Vec<SketchCurve> {
    let (w, gap) = (10.0 * s, 2.0 * s);
    let h = 2.0 * arm + gap;
    polygon(
        &[
            [0.0, 0.0],
            [w, 0.0],
            [w, arm],
            [back, arm],
            [back, arm + gap],
            [w, arm + gap],
            [w, h],
            [0.0, h],
        ],
        1,
    )
}

#[test]
fn a_c_whose_inward_offset_splits_is_reported() {
    for s in SCALES {
        let t = tol(s);
        // Arms 3 wide, back 1 wide: at d = 0.6 the back is gone and the
        // arms stay, two regions.
        let input = profiles(&c_shape(s, 3.0 * s, 1.0 * s), &t);
        let o = offset_profiles(&input, -0.6 * s, Corners::Round, &t).unwrap();
        assert_eq!((o.loops_before, o.loops_after), (1, 2));
        let whole = o.whole_up_to.unwrap();
        assert!((whole + 0.5 * s).abs() <= 1e-6 * s, "{whole}");
        // Each arm: (10 - 2d) by (3 - 2d), and where the back was, the
        // strip from x = d to the back's side under the arc of radius d
        // about the inner corner.
        let (d, b) = (0.6 * s, 1.0 * s);
        let a = b - d;
        let strip = d * a - (a * (d * d - a * a).sqrt() + d * d * (a / d).asin()) / 2.0;
        close(
            area(&o),
            2.0 * ((10.0 * s - 2.0 * d) * (3.0 * s - 2.0 * d) + strip),
            1e-12,
        );
        // The same C by a little: one region still.
        let o = offset_profiles(&input, -0.2 * s, Corners::Round, &t).unwrap();
        assert_eq!(o.loops_after, 1);
        assert_eq!(o.whole_up_to, None);
    }
}

#[test]
fn a_c_whose_inward_offset_vanishes_is_refused() {
    for s in SCALES {
        let t = tol(s);
        let input = profiles(&c_shape(s, 1.0 * s, 1.0 * s), &t);
        let err = offset_profiles(&input, -0.6 * s, Corners::Round, &t).unwrap_err();
        let KernelError::OffsetProfile { largest, .. } = &err else {
            panic!("{err:?}");
        };
        let largest = largest.unwrap();
        assert!((largest + 0.5 * s).abs() <= 1e-6 * s, "{largest}");
        assert!(err.to_string().contains("largest distance"));
    }
}

#[test]
fn an_outward_offset_that_closes_a_channel_is_reported() {
    for s in SCALES {
        let t = tol(s);
        // A square with a 6 by 6 cavity reached through a channel 1 wide:
        // growing by 0.6 closes the channel and leaves the cavity a hole.
        let p: Vec<[f64; 2]> = [
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 4.5],
            [8.0, 4.5],
            [8.0, 2.0],
            [2.0, 2.0],
            [2.0, 8.0],
            [8.0, 8.0],
            [8.0, 5.5],
            [10.0, 5.5],
            [10.0, 10.0],
            [0.0, 10.0],
        ]
        .iter()
        .map(|q| [q[0] * s, q[1] * s])
        .collect();
        let input = profiles(&polygon(&p, 1), &t);
        let o = offset_profiles(&input, 0.6 * s, Corners::Round, &t).unwrap();
        assert_eq!((o.loops_before, o.loops_after), (1, 2));
        assert!((o.whole_up_to.unwrap() - 0.5 * s).abs() <= 1e-6 * s);
        // The hole: the cavity shrunk by 0.6, a 4.8 square, and the notch
        // at the channel's mouth between the arcs about its two corners:
        // twice the 0.6 by 0.5 rectangle less the disk of radius 0.6 in it.
        let (d, h) = (0.6 * s, 0.5 * s);
        let disk = (h * (d * d - h * h).sqrt() + d * d * (h / d).asin()) / 2.0;
        let hole = o.profiles[0].holes[0].signed_area();
        close(-hole, (4.8 * s) * (4.8 * s) + 2.0 * (d * h - disk), 1e-12);
    }
}

#[test]
fn a_tiny_distance_leaves_the_outline_as_it_is() {
    let t = tol(1.0);
    let (p, a, _, _) = pentagon(1.0);
    let input = profiles(&polygon(&p, 1), &t);
    for d in [0.0, 1e-8, -1e-8] {
        let o = offset_profiles(&input, d, Corners::Round, &t).unwrap();
        close(area(&o), a, 1e-15);
        assert!(
            o.origins
                .iter()
                .all(|o| matches!(o.source, OffsetSource::Curve(_)))
        );
    }
}

#[test]
fn bad_input_is_an_error() {
    let t = tol(1.0);
    let (p, _, _, _) = pentagon(1.0);
    let input = profiles(&polygon(&p, 1), &t);
    for d in [f64::NAN, f64::INFINITY, 1e200] {
        assert!(offset_profiles(&input, d, Corners::Round, &t).is_err());
    }
    assert!(offset_profiles(&[], 1.0, Corners::Round, &t).is_err());
    // A spline: not supported yet.
    let spline = vec![
        line(1, [0.0, 0.0], [4.0, 0.0]),
        SketchCurve {
            id: 2,
            curve: Curve2::Spline(craft3d_kernel::profile::Spline2 {
                degree: 2,
                knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                control: vec![[4.0, 0.0], [2.0, 4.0], [0.0, 0.0]],
            }),
        },
    ];
    let input = profiles(&spline, &t);
    let err = offset_profiles(&input, 0.5, Corners::Round, &t).unwrap_err();
    assert!(err.to_string().contains("not supported yet"), "{err}");
}
