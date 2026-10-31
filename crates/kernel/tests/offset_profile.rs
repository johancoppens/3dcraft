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
use craft3d_math::Portable;
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
        perimeter += (b[0] - a[0]).phypot(b[1] - a[1]);
        let u = [b[0] - a[0], b[1] - a[1]];
        let v = [c[0] - b[0], c[1] - b[1]];
        turns.push((u[0] * v[1] - u[1] * v[0]).patan2(u[0] * v[0] + u[1] * v[1]));
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
            let mitre: f64 = turns.iter().map(|k| (k / 2.0).ptan()).sum();
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
        let strip = d * a - (a * (d * d - a * a).sqrt() + d * d * (a / d).pasin()) / 2.0;
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
        let disk = (h * (d * d - h * h).sqrt() + d * d * (h / d).pasin()) / 2.0;
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

fn circle(id: u64, c: [f64; 2], r: f64) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Circle {
            center: c,
            radius: r,
        },
    }
}

/// A square of side `s` turned 45°, its near corner... its near edge at
/// `at` along the diagonal (1, 1)/√2 from the origin.
fn diamond_edge_at(at: f64, s: f64, first_id: u64) -> Vec<SketchCurve> {
    let u = [std::f64::consts::FRAC_1_SQRT_2; 2];
    let v = [-u[0], u[1]];
    let p = |a: f64, b: f64| [u[0] * a + v[0] * b, u[1] * a + v[1] * b];
    polygon(
        &[
            p(at, -s / 2.0),
            p(at + s, -s / 2.0),
            p(at + s, s / 2.0),
            p(at, s / 2.0),
        ],
        first_id,
    )
}

/// A line that misses a circle by more than rounding is not cut at the
/// foot of the perpendicular (review of L2): a D of radius 5 or a disc of
/// radius 5 beside a square whose near edge is 6.5 out along the diagonal,
/// grown by 0.5, are two separate offsets with their closed-form areas.
#[test]
fn a_line_that_misses_an_offset_circle_does_not_cut_it() {
    let t = tol(1.0);
    let (r, s, d) = (5.0, 2.0, 0.5);
    let square = steiner_area(s * s, 4.0 * s, d);
    // A: a D, a half circle closed by its diameter.
    let mut curves = vec![
        arc(1, [0.0, 0.0], [0.0, -r], [0.0, r]),
        line(2, [0.0, r], [0.0, -r]),
    ];
    curves.extend(diamond_edge_at(6.5, s, 10));
    let o = offset_profiles(&profiles(&curves, &t), d, Corners::Round, &t).unwrap();
    close(
        area(&o),
        steiner_area(PI * r * r / 2.0, PI * r + 2.0 * r, d) + square,
        1e-12,
    );
    assert!(
        o.origins.iter().all(|o| o.within.is_none()),
        "nothing split"
    );
    // B: a whole circle.
    let mut curves = vec![circle(1, [0.0, 0.0], r)];
    curves.extend(diamond_edge_at(6.5, s, 10));
    let o = offset_profiles(&profiles(&curves, &t), d, Corners::Round, &t).unwrap();
    close(area(&o), PI * (r + d) * (r + d) + square, 1e-12);
    assert_eq!(o.curves.len(), 1 + 8, "the circle and the square, not cut");
}

/// The pieces of one curve of a region that split are told apart by the
/// loop they close, never by an order: across distances the same name
/// stays in the same region (here by the side of its region's centroid),
/// and a name whose region went fails to be there rather than moving.
#[test]
fn pieces_are_named_by_their_loop_not_by_an_order() {
    let t = tol(1.0);
    // A plate 10 by 6 with a hole of radius 1.5 in the middle: from 1.5 to
    // 1.75 in it is a left and a right strip, each closed by pieces of the
    // top, the bottom and the hole, and its own side.
    let mut curves = polygon(&[[-5.0, -3.0], [5.0, -3.0], [5.0, 3.0], [-5.0, 3.0]], 1);
    curves.push(circle(9, [0.0, 0.0], 1.5));
    let input = profiles(&curves, &t);
    // A name in one region at one distance is in that region at every
    // distance where it is in one region; one in several regions at once
    // (regions of the same curves) is not told apart there, and resolves
    // to all of them.
    let mut seen: std::collections::BTreeMap<String, bool> = Default::default();
    let mut single = 0;
    for d in [-1.55, -1.6, -1.65, -1.7] {
        let o = offset_profiles(&input, d, Corners::Round, &t).unwrap();
        assert_eq!(o.regions_after, 2);
        let mut here: std::collections::BTreeMap<String, std::collections::BTreeSet<bool>> =
            Default::default();
        for profile in &o.profiles {
            let c = profile.centroid().unwrap();
            for s in std::iter::once(&profile.outer)
                .chain(&profile.holes)
                .flat_map(|l| &l.segments)
            {
                let origin = &o.origins[s.source as usize];
                let key = format!("{:?} in {:?}", origin.source, origin.within);
                // The side of the region (its centroid is on the x axis).
                here.entry(key).or_default().insert(c[0] > 0.0);
            }
        }
        for (key, sides) in here {
            if let [side] = sides.into_iter().collect::<Vec<_>>()[..] {
                single += 1;
                if let Some(before) = seen.insert(key.clone(), side) {
                    assert_eq!(before, side, "{key} moved at d = {d}");
                }
            }
        }
    }
    assert!(single > 10);
    // Two regions of the same curves (a disc with two holes, split in two
    // by them) are not told apart: their pieces have the same origin, as
    // their caps have one name.
    let curves = vec![
        circle(1, [0.0, 0.0], 10.0),
        circle(2, [-2.0, 5.5], 3.0),
        circle(3, [-2.0, -5.5], 3.0),
    ];
    let o = offset_profiles(&profiles(&curves, &t), -3.0, Corners::Round, &t).unwrap();
    assert_eq!(o.regions_after, 2);
    let outer: Vec<_> = o
        .origins
        .iter()
        .filter(|o| o.source == OffsetSource::Curve(1))
        .collect();
    assert_eq!(outer.len(), 2);
    assert_eq!(outer[0], outer[1]);
    assert_eq!(outer[0].within.as_deref(), Some(&[1, 2, 3][..]));
}

/// A polygon of 2100 sides offset with round corners has 4200 pieces:
/// more than the offset took before 2026-10-17 (4096, refused then with
/// `TooManyOffsetCurves`), within what it takes now. Its area is Steiner's
/// `A + P d + π d²`. The cap itself (`MAX_PIECES`) is met by
/// `offset_scale.rs` › `too_many_pieces_are_passed_on`, and a bound on the
/// work that ends the search for a distance by
/// `a_bound_ends_the_search_for_a_distance` there.
#[test]
fn a_polygon_of_2100_round_corners_fits() {
    let t = tol(1.0);
    let n = 2100;
    let r = 1000.0;
    let points: Vec<[f64; 2]> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            [r * a.pcos(), r * a.psin()]
        })
        .collect();
    let input = profiles(&polygon(&points, 1), &t);
    // On the work of this thread, which a busy machine does not stretch.
    let started = craft3d_testkit::cost::WorkClock::start();
    let o = offset_profiles(&input, 0.5, Corners::Round, &t).unwrap();
    assert!(started.elapsed().as_secs_f64() < 5.0);
    assert_eq!(o.curves.len(), 2 * n);
    let k = n as f64;
    let a = k / 2.0 * r * r * (2.0 * PI / k).psin();
    let p = 2.0 * k * r * (PI / k).psin();
    close(area(&o), steiner_area(a, p, 0.5), 1e-9);
    // Sharp corners add no curve where lines meet.
    let o = offset_profiles(&input, 0.5, Corners::Sharp, &t).unwrap();
    assert_eq!(o.curves.len(), n);
}

/// A huge distance: inward past half the outline's diagonal nothing is
/// left; outward past what the tolerance can hold it is refused; both
/// with the largest distance that fits.
#[test]
fn huge_distances_are_bounded_by_the_outline() {
    let t = tol(1.0);
    let input = profiles(
        &polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 6.0], [0.0, 6.0]], 1),
        &t,
    );
    for d in [-1e99, -1e10, -50.0] {
        let err = offset_profiles(&input, d, Corners::Round, &t).unwrap_err();
        let KernelError::OffsetProfile {
            reason, largest, ..
        } = &err
        else {
            panic!("{err:?}");
        };
        assert!(reason.contains("leaves nothing"), "{err}");
        assert!((largest.unwrap() + 3.0).abs() <= 2.0 * t.linear, "{err}");
    }
    for d in [1e99, 1e40] {
        let err = offset_profiles(&input, d, Corners::Round, &t).unwrap_err();
        let KernelError::OffsetProfile { largest, .. } = &err else {
            panic!("{err:?}");
        };
        let largest = largest.unwrap();
        assert!(largest > 1e3 && largest < 1e12, "{err}");
        // That distance itself is made.
        assert!(offset_profiles(&input, largest, Corners::Round, &t).is_ok());
    }
}
