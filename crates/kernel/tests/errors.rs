//! What profile detection and extrusion refuse, and that they say why:
//! every case returns its own error naming the curves, never a panic and
//! never a region.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{arc, circle, line, polygon, rectangle};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::profile::{
    Loop2, MAX_CURVES, Profile, Segment, Shape, SketchCurve, detect_profiles,
};
use craft3d_kernel::{KernelError, MAX_REPORTED};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::check;

fn detect(curves: &[SketchCurve]) -> Result<Vec<Profile>, KernelError> {
    detect_profiles(curves, &Tolerance::default())
}

fn crossing(curves: &[SketchCurve], a: u64, b: u64) {
    assert_eq!(
        detect(curves),
        Err(KernelError::CrossingCurves { curves: [a, b] })
    );
    // Whatever the order of the input.
    let mut reversed = curves.to_vec();
    reversed.reverse();
    assert_eq!(
        detect(&reversed),
        Err(KernelError::CrossingCurves { curves: [a, b] })
    );
}

fn degenerate(curve: SketchCurve) {
    let id = curve.id;
    // Alone, and as part of an otherwise good sketch.
    for curves in [vec![curve.clone()], {
        let mut v = rectangle(100, 1, (50.0, 60.0), (50.0, 60.0));
        v.push(curve);
        v
    }] {
        match detect(&curves) {
            Err(KernelError::DegenerateCurve { id: reported, .. }) => assert_eq!(reported, id),
            other => panic!("expected a degenerate curve {id}, got {other:?}"),
        }
    }
}

#[test]
fn no_curves_is_no_profile() {
    assert_eq!(detect(&[]), Err(KernelError::NoProfile));
    let message = KernelError::NoProfile.to_string();
    assert!(message.contains("no closed profile"), "{message}");
}

#[test]
fn an_open_chain_names_the_curves_with_a_free_end() {
    // Three sides of a square: lines 1 and 3 each have an end in the air.
    let mut curves = rectangle(1, 1, (0.0, 4.0), (0.0, 4.0));
    curves.pop();
    let error = detect(&curves).unwrap_err();
    assert_eq!(error, KernelError::OpenProfile { curves: vec![1, 3] });
    assert!(error.to_string().contains("1, 3"), "{error}");
    // A single line, and a single arc.
    assert_eq!(
        detect(&[line(7, [0.0, 0.0], [1.0, 0.0])]),
        Err(KernelError::OpenProfile { curves: vec![7] })
    );
    assert_eq!(
        detect(&[arc(9, [0.0, 0.0], [1.0, 0.0], [0.0, 1.0])]),
        Err(KernelError::OpenProfile { curves: vec![9] })
    );
    // A closed square next to it does not hide the open one.
    curves.extend(rectangle(10, 1, (10.0, 14.0), (0.0, 4.0)));
    assert_eq!(
        detect(&curves),
        Err(KernelError::OpenProfile { curves: vec![1, 3] })
    );
}

#[test]
fn the_list_of_curves_in_an_error_is_capped() {
    let curves: Vec<SketchCurve> = (0..50)
        .map(|i| line(i, [i as f64 * 3.0, 0.0], [i as f64 * 3.0 + 1.0, 0.0]))
        .collect();
    let Err(KernelError::OpenProfile { curves }) = detect(&curves) else {
        panic!("expected an open profile");
    };
    assert_eq!(curves, (0..MAX_REPORTED as u64).collect::<Vec<_>>());
}

#[test]
fn a_branch_names_the_curves_that_meet() {
    // A square with a diagonal: three ends meet at two of its corners.
    let mut curves = rectangle(1, 1, (0.0, 4.0), (0.0, 4.0));
    curves.push(line(9, [0.0, 0.0], [4.0, 4.0]));
    assert_eq!(
        detect(&curves),
        Err(KernelError::BranchingProfile {
            curves: vec![1, 2, 3, 4, 9]
        })
    );
    // A square with a tail from one corner: the tail's far end is open as
    // well, but the branch is what is reported. Lines 1 and 4 meet at the
    // origin, where the tail 9 starts.
    let mut curves = rectangle(1, 1, (0.0, 4.0), (0.0, 4.0));
    curves.push(line(9, [0.0, 0.0], [-3.0, -3.0]));
    let error = detect(&curves).unwrap_err();
    assert_eq!(
        error,
        KernelError::BranchingProfile {
            curves: vec![1, 4, 9]
        }
    );
    assert!(error.to_string().contains("1, 4, 9"), "{error}");
    // Two squares sharing a side: four ends at each of its corners.
    let mut curves = rectangle(1, 1, (0.0, 2.0), (0.0, 2.0));
    curves.extend(rectangle(11, 1, (2.0, 4.0), (0.0, 2.0)));
    assert!(matches!(
        detect(&curves),
        Err(KernelError::BranchingProfile { .. })
    ));
}

#[test]
fn crossings_of_every_pair_of_kinds_are_refused() {
    // Line and line: two rectangles in a cross. Line 1 (y = 0, x 0..6)
    // meets line 14 (x = 2, y from 3 down to −1).
    let mut curves = rectangle(1, 1, (0.0, 6.0), (0.0, 2.0));
    curves.extend(rectangle(11, 1, (2.0, 4.0), (-1.0, 3.0)));
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves { .. })
    ));
    // Line and circle: the circle about a corner of the square crosses the
    // two sides that meet there.
    let mut curves = rectangle(1, 1, (0.0, 6.0), (0.0, 6.0));
    curves.push(circle(9, [6.0, 0.0], 2.0));
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves { curves: [1 | 2, 9] })
    ));
    // Line and arc: a D (the upper half disc of radius 5) and a triangle
    // that sticks out through its arc.
    let d = [
        arc(1, [0.0, 0.0], [5.0, 0.0], [-5.0, 0.0]),
        line(2, [-5.0, 0.0], [5.0, 0.0]),
    ];
    let mut curves = d.to_vec();
    curves.extend(polygon(11, 1, &[[-1.0, 4.0], [1.0, 4.0], [0.0, 9.0]]));
    // Lines 12 and 13 run from y = 4 inside the D to y = 9 outside: both
    // cross the arc; neither comes near line 2.
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves {
            curves: [1, 12 | 13]
        })
    ));
    // Arc and arc: two D shapes, the second shifted right by 5.
    let mut curves = d.to_vec();
    curves.extend([
        arc(11, [5.0, 1.0], [10.0, 1.0], [0.0, 1.0]),
        line(12, [0.0, 1.0], [10.0, 1.0]),
    ]);
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves { .. })
    ));
    // The two arcs alone, each closed by a far-away chain that crosses
    // nothing, so that the arc pair is the only crossing: arcs of radius 5
    // about (0, 0) and (5, 0), upper halves, meet at (2.5, 4.33).
    let curves = [
        arc(1, [0.0, 0.0], [5.0, 0.0], [-5.0, 0.0]),
        line(2, [-5.0, 0.0], [-5.0, -20.0]),
        line(3, [-5.0, -20.0], [5.0, -20.0]),
        line(4, [5.0, -20.0], [5.0, 0.0]),
        arc(11, [5.0, 0.0], [10.0, 0.0], [0.0, 0.0]),
        line(12, [0.0, 0.0], [0.0, 30.0]),
        line(13, [0.0, 30.0], [10.0, 30.0]),
        line(14, [10.0, 30.0], [10.0, 0.0]),
    ];
    // (Line 12 also crosses arc 1 at (0, 5); arcs 1 and 11 cross too.)
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves {
            curves: [1, 11 | 12]
        })
    ));
    // Circle and circle.
    crossing(
        &[circle(3, [0.0, 0.0], 2.0), circle(8, [3.0, 0.0], 2.0)],
        3,
        8,
    );
    // Circle and arc: a circle about the top of the D.
    let mut curves = d.to_vec();
    curves.push(circle(7, [0.0, 5.0], 1.0));
    crossing(&curves, 1, 7);
    // Arc and arc, nothing else near: the lens with one arc moved right by
    // 0.5 is open, so cross two lenses instead. The lens of the catalogue
    // (ids 2, 5) and the same lens turned a quarter (ids 12, 15).
    let curves = [
        arc(5, [-3.0, 0.0], [0.0, -4.0], [0.0, 4.0]),
        arc(2, [3.0, 0.0], [0.0, 4.0], [0.0, -4.0]),
        arc(15, [0.0, -3.0], [4.0, 0.0], [-4.0, 0.0]),
        arc(12, [0.0, 3.0], [-4.0, 0.0], [4.0, 0.0]),
    ];
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves {
            curves: [2 | 5, 12 | 15]
        })
    ));
}

#[test]
fn a_loop_that_crosses_itself_is_refused() {
    // A figure of eight with unequal lobes: (0,0) → (4,4) → (4,0) → (0,2).
    // Its first side (y = x) and third side (y = 2 − x/2) cross at x = 4/3.
    // The lobes' areas do not cancel (shoelace: −4), so it is the crossing
    // that is found.
    let curves = polygon(1, 1, &[[0.0, 0.0], [4.0, 4.0], [4.0, 0.0], [0.0, 2.0]]);
    crossing(&curves, 1, 3);
    // With equal lobes the signed area is exactly zero (shoelace:
    // 0 − 16 + 16 + 0), which is reported first, as a loop without area.
    let curves = polygon(1, 1, &[[0.0, 0.0], [4.0, 4.0], [4.0, 0.0], [0.0, 4.0]]);
    assert_eq!(
        detect(&curves),
        Err(KernelError::ZeroAreaLoop {
            curves: vec![1, 2, 3, 4]
        })
    );
    // A bow tie made with arcs: two arcs that cross between their shared
    // ends cannot exist (two circles meet at most twice), so the arc case
    // is a line through a D: the flat side replaced by a detour that
    // crosses the arc. (5,0) arc to (−5,0), then to (0, 8), then back to
    // (5, 0): the side from (−5, 0) to (0, 8) crosses the arc, and so does
    // the side from (0, 8) to (5, 0).
    let curves = [
        arc(1, [0.0, 0.0], [5.0, 0.0], [-5.0, 0.0]),
        line(2, [-5.0, 0.0], [0.0, 8.0]),
        line(3, [0.0, 8.0], [5.0, 0.0]),
    ];
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves { curves: [1, 2 | 3] })
    ));
}

#[test]
fn touching_without_crossing_is_refused() {
    // A circle of radius 2 inside a square, touching its left side.
    let mut curves = rectangle(1, 1, (0.0, 10.0), (0.0, 10.0));
    curves.push(circle(9, [2.0, 5.0], 2.0));
    crossing(&curves, 4, 9);
    // The corner of one square on the side of another: line 1 (y = 0) is
    // touched at (5, 0) by the corner where lines 11 and 14 meet.
    let mut curves = rectangle(1, 1, (0.0, 10.0), (0.0, 10.0));
    curves.extend(polygon(11, 1, &[[5.0, 0.0], [7.0, -3.0], [3.0, -3.0]]));
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves {
            curves: [1, 11 | 13]
        })
    ));
    // A side of one square lying on a side of another (partly).
    let mut curves = rectangle(1, 1, (0.0, 2.0), (0.0, 2.0));
    curves.extend(rectangle(11, 1, (2.0, 4.0), (0.5, 1.5)));
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves {
            curves: [2, 11 | 13 | 14]
        })
    ));
    // Two circles touching from outside, and from inside; the same circle
    // twice.
    crossing(
        &[circle(1, [0.0, 0.0], 2.0), circle(2, [5.0, 0.0], 3.0)],
        1,
        2,
    );
    crossing(
        &[circle(1, [0.0, 0.0], 2.0), circle(2, [1.0, 0.0], 3.0)],
        1,
        2,
    );
    crossing(
        &[circle(1, [0.0, 0.0], 2.0), circle(2, [0.0, 0.0], 2.0)],
        1,
        2,
    );
    // A line that comes back along its neighbour: (0,0) → (10,0) → (4,0)
    // → (4,5) → (0,5). Sides 1 and 2 overlap.
    let curves = polygon(
        1,
        1,
        &[[0.0, 0.0], [10.0, 0.0], [4.0, 0.0], [4.0, 5.0], [0.0, 5.0]],
    );
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves { curves: [1, 2 | 3] })
    ));
    // An arc that comes back along its neighbour: a quarter of the unit
    // circle (0° → 90°) followed by an arc on the same circle from 45° to
    // 90°, entered at its end.
    let h = 0.5_f64.sqrt();
    let curves = [
        arc(1, [0.0, 0.0], [1.0, 0.0], [0.0, 1.0]),
        arc(2, [0.0, 0.0], [h, h], [0.0, 1.0]),
        line(3, [h, h], [1.0, 0.0]),
    ];
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves { curves: [1, 2 | 3] })
    ));
}

#[test]
fn a_loop_without_area_is_refused() {
    // There and back along the same line.
    let curves = [
        line(4, [0.0, 0.0], [3.0, 1.0]),
        line(6, [3.0, 1.0], [0.0, 0.0]),
    ];
    assert_eq!(
        detect(&curves),
        Err(KernelError::ZeroAreaLoop { curves: vec![4, 6] })
    );
    // Drawn the same way twice.
    let curves = [
        line(4, [0.0, 0.0], [3.0, 1.0]),
        line(6, [0.0, 0.0], [3.0, 1.0]),
    ];
    assert_eq!(
        detect(&curves),
        Err(KernelError::ZeroAreaLoop { curves: vec![4, 6] })
    );
    // A sliver thinner than the tolerance has an area, but its long sides
    // lie on each other: refused as touching.
    let curves = polygon(1, 1, &[[0.0, 0.0], [10.0, 0.0], [5.0, 1e-8]]);
    assert!(matches!(
        detect(&curves),
        Err(KernelError::CrossingCurves { .. })
    ));
}

#[test]
fn degenerate_curves_are_refused_by_id() {
    degenerate(line(5, [1.0, 1.0], [1.0, 1.0]));
    degenerate(line(5, [1.0, 1.0], [1.0, 1.0 + 1e-7]));
    degenerate(circle(5, [0.0, 0.0], 0.0));
    degenerate(circle(5, [0.0, 0.0], -2.0));
    degenerate(circle(5, [0.0, 0.0], f64::NAN));
    degenerate(circle(5, [0.0, f64::INFINITY], 1.0));
    degenerate(circle(5, [0.0, 0.0], 1e200));
    degenerate(line(5, [f64::NAN, 0.0], [1.0, 0.0]));
    degenerate(line(5, [0.0, 0.0], [1.0, f64::NEG_INFINITY]));
    degenerate(line(5, [0.0, 0.0], [1e300, 0.0]));
    degenerate(arc(5, [f64::NAN, 0.0], [1.0, 0.0], [0.0, 1.0]));
    // An arc whose ends are at distances 1 and 1.5 from its centre.
    degenerate(arc(5, [0.0, 0.0], [1.0, 0.0], [0.0, 1.5]));
    // An arc of no radius, and one that starts where it ends.
    degenerate(arc(5, [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]));
    degenerate(arc(5, [0.0, 0.0], [1.0, 0.0], [1.0, 0.0]));
    let message = detect(&[circle(5, [0.0, 0.0], -2.0)])
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("curve 5") && message.contains("radius"),
        "{message}"
    );
}

#[test]
fn duplicate_ids_and_too_many_curves_are_refused() {
    let curves = [circle(3, [0.0, 0.0], 1.0), circle(3, [5.0, 0.0], 1.0)];
    assert_eq!(
        detect(&curves),
        Err(KernelError::DuplicateCurveId { id: 3 })
    );
    let many: Vec<SketchCurve> = (0..=MAX_CURVES as u64)
        .map(|i| circle(i, [i as f64 * 3.0, 0.0], 1.0))
        .collect();
    assert_eq!(
        detect(&many),
        Err(KernelError::TooManyCurves {
            count: MAX_CURVES + 1,
            limit: MAX_CURVES
        })
    );
    // Exactly the cap is accepted: that many separate discs.
    assert_eq!(detect(&many[..MAX_CURVES]).unwrap().len(), MAX_CURVES);
}

#[test]
fn a_bad_tolerance_is_refused() {
    for linear in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let tol = Tolerance {
            linear,
            angular: 1e-9,
        };
        let curves = rectangle(1, 1, (0.0, 1.0), (0.0, 1.0));
        assert!(matches!(
            detect_profiles(&curves, &tol),
            Err(KernelError::InvalidSize {
                name: "tolerance",
                ..
            })
        ));
        let good = detect(&curves).unwrap();
        assert!(matches!(
            extrude(&good, &Frame::WORLD, 0.0, 1.0, &tol),
            Err(KernelError::InvalidSize {
                name: "tolerance",
                ..
            })
        ));
    }
}

/// Ends join within the tolerance and not beyond it. The gap is at the
/// corner (4, 3) of a 4 × 3 rectangle: line 2 starts `gap` to the right of
/// where line 1 ends.
#[test]
fn ends_join_within_the_tolerance_only() {
    let tol = Tolerance::default();
    let gapped = |gap: f64| {
        vec![
            line(1, [0.0, 0.0], [4.0, 0.0]),
            line(2, [4.0 + gap, 0.0], [4.0, 3.0]),
            line(3, [4.0, 3.0], [0.0, 3.0]),
            line(4, [0.0, 3.0], [0.0, 0.0]),
        ]
    };
    for gap in [0.0, 1e-9, 0.5e-6, 0.99e-6] {
        let profiles = detect(&gapped(gap)).unwrap_or_else(|e| panic!("gap {gap}: {e}"));
        assert_eq!(profiles.len(), 1);
        // The representative is the end of the curve with the smaller id:
        // line 2 now starts at the end of line 1 exactly.
        let segments = &profiles[0].outer.segments;
        assert_eq!(segments[0].end, [4.0, 0.0]);
        assert_eq!(segments[1].start, [4.0, 0.0]);
        // 4 × 3 exactly, whatever the gap was.
        assert_eq!(profiles[0].area(), 12.0);
        let result = extrude(&profiles, &Frame::WORLD, 0.0, 1.0, &tol).unwrap();
        assert_eq!(check(&result.body, &tol), vec![]);
        assert_eq!(result.body.vertices.len(), 8);
    }
    for gap in [1.01e-6, 2e-6, 1e-3] {
        assert_eq!(
            detect(&gapped(gap)),
            Err(KernelError::OpenProfile { curves: vec![1, 2] }),
            "gap {gap}"
        );
    }
    // The distance counts, not the difference in x: a gap of 1.5e-6 straight
    // up, and one of 1.3e-6 at 45° (0.92e-6 in x and in y), stay open.
    for (dx, dy) in [(0.0, 1.5e-6), (0.92e-6, 0.92e-6), (0.0, -1.5e-6)] {
        let mut curves = gapped(0.0);
        curves[1] = line(2, [4.0 + dx, dy], [4.0, 3.0]);
        assert_eq!(
            detect(&curves),
            Err(KernelError::OpenProfile { curves: vec![1, 2] }),
            "gap ({dx}, {dy})"
        );
    }
    // And 0.9e-6 at 45° (0.64e-6 each way) closes.
    let mut curves = gapped(0.0);
    curves[1] = line(2, [4.0 + 0.64e-6, 0.64e-6], [4.0, 3.0]);
    assert_eq!(detect(&curves).unwrap().len(), 1);
    // The tolerance is data: with a coarser one the same gap closes.
    let coarse = Tolerance::new(1e-2, 1e-9).unwrap();
    assert_eq!(detect_profiles(&gapped(1e-3), &coarse).unwrap().len(), 1);
}

/// Where an arc meets a line the arc's end is the representative, whatever
/// the ids: a line through two given points is exact, an arc about a given
/// centre is not.
#[test]
fn an_arc_end_represents_a_junction_with_a_line() {
    let tol = Tolerance::default();
    for (arc_id, line_id) in [(1, 2), (2, 1)] {
        // A D whose flat side stops 4e-7 short of the arc at both ends.
        let curves = [
            arc(arc_id, [0.0, 0.0], [5.0, 0.0], [-5.0, 0.0]),
            line(line_id, [-5.0 + 4e-7, 0.0], [5.0 - 4e-7, 0.0]),
        ];
        let profiles = detect(&curves).unwrap();
        for s in &profiles[0].outer.segments {
            assert_eq!(s.start[0].abs(), 5.0, "{s:?}");
            assert_eq!(s.end[0].abs(), 5.0, "{s:?}");
        }
        let result = extrude(&profiles, &Frame::WORLD, 0.0, 1.0, &tol).unwrap();
        assert_eq!(check(&result.body, &tol), vec![]);
    }
}

/// Curves keep clear of each other by more than the tolerance, or touch.
#[test]
fn touching_is_decided_at_the_tolerance() {
    // A circle of radius 2 inside a square, `gap` from its left side.
    let with_gap = |gap: f64| {
        let mut curves = rectangle(1, 1, (0.0, 10.0), (0.0, 10.0));
        curves.push(circle(9, [2.0 + gap, 5.0], 2.0));
        curves
    };
    for gap in [-0.5e-6, 0.0, 0.5e-6, 0.9e-6] {
        assert_eq!(
            detect(&with_gap(gap)),
            Err(KernelError::CrossingCurves { curves: [4, 9] }),
            "gap {gap}"
        );
    }
    for gap in [1.1e-6, 2e-6, 1e-3] {
        let profiles = detect(&with_gap(gap)).unwrap_or_else(|e| panic!("gap {gap}: {e}"));
        assert_eq!(
            (profiles.len(), profiles[0].holes.len()),
            (1, 1),
            "gap {gap}"
        );
    }
    // A circle of radius 3 passing a corner of the square on the outside,
    // along the diagonal: its centre is 3 + gap from the corner (10, 10).
    // It comes within tolerance of nothing but the corner point itself.
    let past_corner = |gap: f64| {
        let d = (3.0 + gap) * 0.5_f64.sqrt();
        let mut curves = rectangle(1, 1, (0.0, 10.0), (0.0, 10.0));
        curves.push(circle(9, [10.0 + d, 10.0 + d], 3.0));
        curves
    };
    for gap in [0.0, 0.5e-6, 0.9e-6] {
        assert!(
            matches!(
                detect(&past_corner(gap)),
                Err(KernelError::CrossingCurves { curves: [2 | 3, 9] })
            ),
            "gap {gap}"
        );
    }
    for gap in [1.1e-6, 1e-3] {
        assert_eq!(detect(&past_corner(gap)).unwrap().len(), 2, "gap {gap}");
    }
    // An arc's ends within the tolerance of the same distance to its
    // centre, and beyond it.
    let d = |extra: f64| {
        [
            arc(1, [0.0, 0.0], [5.0, 0.0], [-5.0 - extra, 0.0]),
            line(2, [-5.0 - extra, 0.0], [5.0, 0.0]),
        ]
    };
    assert_eq!(detect(&d(0.5e-6)).unwrap().len(), 1);
    assert!(matches!(
        detect(&d(2e-6)),
        Err(KernelError::DegenerateCurve { id: 1, .. })
    ));
}

/// Tangent joins are the hard case for touch detection: the curves stay
/// within tolerance of each other over a length of about √(2·r·tol), far
/// more than the tolerance. They must still be accepted, also when the
/// ends are a little apart.
#[test]
fn tangent_joins_are_not_mistaken_for_touching() {
    let tol = Tolerance::default();
    for k in [1e-3, 1.0, 1e3, 1e6] {
        for gap in [0.0, 3e-7] {
            // The slot of the catalogue with its lines moved by `gap`.
            let curves = [
                line(1, [gap, -2.0 * k], [6.0 * k, -2.0 * k - gap]),
                arc(2, [6.0 * k, 0.0], [6.0 * k, -2.0 * k], [6.0 * k, 2.0 * k]),
                line(3, [6.0 * k + gap, 2.0 * k], [0.0, 2.0 * k + gap]),
                arc(4, [0.0, 0.0], [0.0, 2.0 * k], [0.0, -2.0 * k]),
            ];
            let profiles = detect(&curves).unwrap_or_else(|e| panic!("scale {k} gap {gap}: {e}"));
            assert_eq!(profiles.len(), 1);
            extrude(&profiles, &Frame::WORLD, 0.0, k, &tol)
                .unwrap_or_else(|e| panic!("scale {k} gap {gap}: {e}"));
        }
    }
    // A full circle drawn as two half arcs, and as three arcs.
    let curves = [
        arc(1, [0.0, 0.0], [5.0, 0.0], [-5.0, 0.0]),
        arc(2, [0.0, 0.0], [-5.0, 0.0], [5.0, 0.0]),
    ];
    let profiles = detect(&curves).unwrap();
    // π r² up to the rounding of two half turns.
    assert!((profiles[0].area() - 25.0 * std::f64::consts::PI).abs() < 1e-13);
    let curves = [
        arc(1, [0.0, 0.0], [5.0, 0.0], [0.0, 5.0]),
        arc(2, [0.0, 0.0], [0.0, 5.0], [0.0, -5.0]),
        arc(3, [0.0, 0.0], [0.0, -5.0], [5.0, 0.0]),
    ];
    assert_eq!(detect(&curves).unwrap().len(), 1);
    // Collinear lines end to end: a rectangle with a split side.
    let curves = polygon(
        1,
        1,
        &[[0.0, 0.0], [2.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]],
    );
    assert_eq!(detect(&curves).unwrap()[0].area(), 12.0);
}

fn square_profile() -> Vec<Profile> {
    detect(&rectangle(1, 1, (0.0, 2.0), (0.0, 2.0))).unwrap()
}

#[test]
fn extrusion_refuses_bad_offsets_and_no_profiles() {
    let tol = Tolerance::default();
    let profiles = square_profile();
    let w = Frame::WORLD;
    for (from, to) in [
        (0.0, 0.0),
        (1.0, 0.0),
        (-1.0, -2.0),
        (f64::NAN, 1.0),
        (0.0, f64::NAN),
        (0.0, f64::INFINITY),
        (f64::NEG_INFINITY, 0.0),
        // Thinner than the tolerance.
        (0.0, 0.5e-6),
    ] {
        match extrude(&profiles, &w, from, to, &tol) {
            Err(KernelError::InvalidOffsets { .. }) => {}
            other => panic!("from {from} to {to}: {other:?}"),
        }
    }
    assert_eq!(
        extrude(&[], &w, 0.0, 1.0, &tol),
        Err(KernelError::NoProfile)
    );
    assert!(extrude(&profiles, &w, 0.0, 2e-6, &tol).is_ok());
    // Offsets so large that points overflow.
    assert!(extrude(&profiles, &w, -1e308, 1e308, &tol).is_err());
}

/// `Profile` has public fields; one made by hand that is not a closed
/// boundary gives an error, not a broken body.
#[test]
fn extrusion_refuses_profiles_that_are_not_closed_boundaries() {
    let tol = Tolerance::default();
    let w = Frame::WORLD;
    let good = square_profile();
    let with = |edit: &dyn Fn(&mut Profile)| {
        let mut p = good.clone();
        edit(&mut p[0]);
        extrude(&p, &w, 0.0, 1.0, &tol)
    };
    // No segments.
    assert!(matches!(
        with(&|p| p.outer.segments.clear()),
        Err(KernelError::InvalidProfile { .. })
    ));
    // One segment that is a line.
    assert!(matches!(
        with(&|p| p.outer.segments.truncate(1)),
        Err(KernelError::InvalidProfile { .. })
    ));
    // The same curve id twice.
    assert!(matches!(
        with(&|p| p.outer.segments[1].source = 1),
        Err(KernelError::InvalidProfile { .. })
    ));
    // A circle among lines.
    assert!(matches!(
        with(&|p| p.outer.segments[1].shape = Shape::Circle {
            center: [0.0, 0.0],
            radius: 1.0
        }),
        Err(KernelError::InvalidProfile { .. })
    ));
    // A chain that does not close: the checker finds the vertex off its
    // edge.
    assert!(matches!(
        with(&|p| p.outer.segments[2].start = [7.0, 7.0]),
        Err(KernelError::InvalidBody { .. })
    ));
    // An arc whose ends coincide, a zero-length line, NaN.
    let segment = |shape: Shape, start: [f64; 2], end: [f64; 2]| Segment {
        source: 9,
        piece: 0,
        reversed: false,
        start,
        end,
        shape,
    };
    let arc_shape = Shape::Arc {
        center: [0.0, 0.0],
        radius: 1.0,
    };
    for bad in [
        segment(arc_shape, [1.0, 0.0], [1.0, 0.0]),
        segment(Shape::Line, [1.0, 0.0], [1.0, 0.0]),
        segment(Shape::Line, [f64::NAN, 0.0], [1.0, 0.0]),
        segment(
            Shape::Arc {
                center: [0.0, 0.0],
                radius: -1.0,
            },
            [1.0, 0.0],
            [-1.0, 0.0],
        ),
    ] {
        let profile = Profile {
            outer: Loop2 {
                segments: vec![bad.clone(), segment(Shape::Line, bad.end, bad.start)],
            },
            holes: vec![],
        };
        let mut profile = profile;
        profile.outer.segments[1].source = 10;
        assert!(extrude(&[profile], &w, 0.0, 1.0, &tol).is_err(), "{bad:?}");
    }
    // A zero area gives no centroid rather than NaN.
    let flat = Profile {
        outer: Loop2 {
            segments: vec![
                segment(Shape::Line, [0.0, 0.0], [1.0, 0.0]),
                segment(Shape::Line, [1.0, 0.0], [0.0, 0.0]),
            ],
        },
        holes: vec![],
    };
    assert_eq!(flat.area(), 0.0);
    assert_eq!(flat.centroid(), None);
}

#[test]
fn every_error_says_what_to_look_at() {
    let cases: Vec<(KernelError, &[&str])> = vec![
        (
            KernelError::OpenProfile { curves: vec![3, 4] },
            &["not closed", "3, 4"],
        ),
        (
            KernelError::BranchingProfile {
                curves: vec![1, 2, 9],
            },
            &["more than two", "1, 2, 9"],
        ),
        (
            KernelError::CrossingCurves { curves: [2, 7] },
            &["2, 7", "cross"],
        ),
        (
            KernelError::ZeroAreaLoop { curves: vec![4, 6] },
            &["4, 6", "no area"],
        ),
        (KernelError::DuplicateCurveId { id: 12 }, &["12", "same id"]),
        (
            KernelError::TooManyCurves {
                count: 5000,
                limit: 4096,
            },
            &["5000", "4096"],
        ),
        (
            KernelError::InvalidOffsets { from: 2.0, to: 1.0 },
            &["from 2 to 1"],
        ),
    ];
    for (error, needles) in cases {
        let message = error.to_string();
        for needle in needles {
            assert!(message.contains(needle), "{message:?} lacks {needle:?}");
        }
    }
}
