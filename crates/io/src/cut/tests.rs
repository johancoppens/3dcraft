//! The 2D work on parts, in closed form: areas, bounding rectangles,
//! identical parts and kerf offsets.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use craft3d_math::Tolerance;
use std::f64::consts::PI;

fn rect(x: f64, y: f64, w: f64, h: f64) -> Path {
    Path::Chain {
        start: [x, y],
        segments: vec![
            Segment::Line { to: [x + w, y] },
            Segment::Line { to: [x + w, y + h] },
            Segment::Line { to: [x, y + h] },
            Segment::Line { to: [x, y] },
        ],
        closed: true,
    }
}

/// A slot `l` between its centres, of radius `r`, along x from `(x, y)`,
/// running clockwise (a hole).
fn slot(x: f64, y: f64, l: f64, r: f64) -> Path {
    Path::Chain {
        start: [x, y + r],
        segments: vec![
            Segment::Line { to: [x + l, y + r] },
            Segment::Arc {
                to: [x + l, y - r],
                center: [x + l, y],
                radius: r,
                ccw: false,
            },
            Segment::Line { to: [x, y - r] },
            Segment::Arc {
                to: [x, y + r],
                center: [x, y],
                radius: r,
                ccw: false,
            },
        ],
        closed: true,
    }
}

fn plate(k: f64) -> Shape {
    Shape::from_loops(
        vec![
            Path::Circle {
                center: [20.0 * k, 25.0 * k],
                radius: 5.0 * k,
            },
            rect(0.0, 0.0, 100.0 * k, 50.0 * k),
            slot(50.0 * k, 25.0 * k, 30.0 * k, 4.0 * k),
        ],
        Vec::new(),
    )
    .unwrap()
}

fn tolerance(k: f64) -> Tolerance {
    Tolerance {
        linear: 1e-6 * k,
        angular: 1e-9,
    }
}

fn close(a: f64, b: f64, k: f64) {
    assert!((a - b).abs() <= 1e-9 * k.max(k * k), "{a} != {b}");
}

#[test]
fn the_outline_is_the_largest_loop_and_the_area_is_in_closed_form() {
    for k in [1e-3, 1.0, 1e3] {
        let s = plate(k);
        assert!(matches!(s.outline, Path::Chain { .. }));
        assert_eq!(s.holes.len(), 2);
        let area = 5000.0 - PI * 25.0 - (30.0 * 8.0 + PI * 16.0);
        close(s.area(), area * k * k, k);
        let cut = 300.0 + 10.0 * PI + 60.0 + 8.0 * PI;
        close(s.cut_length(), cut * k, k);
    }
}

#[test]
fn a_turned_rectangle_has_its_own_sides_as_its_rectangle() {
    for k in [1e-3, 1.0, 1e3] {
        let turn = Motion {
            cos: 0.6,
            sin: 0.8,
            mirror: false,
            shift: [7.0 * k, -3.0 * k],
        };
        let s = plate(k).moved(&turn).unwrap();
        let (own, _, r) = own_frame(&s).unwrap();
        close(r.length, 100.0 * k, k);
        close(r.width, 50.0 * k, k);
        let b = own.bbox().unwrap();
        close(b[0], 0.0, k);
        close(b[1], 0.0, k);
        close(b[2], 100.0 * k, k);
        close(b[3], 50.0 * k, k);
        close(own.area(), plate(k).area(), k);
    }
}

#[test]
fn an_l_has_its_box_as_its_rectangle() {
    // An L 100 × 60 with arms 20 wide, standing on its short side.
    let l = Path::Chain {
        start: [0.0, 0.0],
        segments: vec![
            Segment::Line { to: [60.0, 0.0] },
            Segment::Line { to: [60.0, 20.0] },
            Segment::Line { to: [20.0, 20.0] },
            Segment::Line { to: [20.0, 100.0] },
            Segment::Line { to: [0.0, 100.0] },
            Segment::Line { to: [0.0, 0.0] },
        ],
        closed: true,
    };
    let s = Shape::from_loops(vec![l], Vec::new()).unwrap();
    let r = bounding_rect(&s).unwrap();
    close(r.length, 100.0, 1.0);
    close(r.width, 60.0, 1.0);
    close(s.area(), 60.0 * 20.0 + 20.0 * 80.0, 1.0);
}

#[test]
fn a_disc_is_as_long_as_it_is_wide() {
    let s = Shape::from_loops(
        vec![Path::Circle {
            center: [3.0, 4.0],
            radius: 10.0,
        }],
        Vec::new(),
    )
    .unwrap();
    let r = bounding_rect(&s).unwrap();
    close(r.length, 20.0, 1.0);
    close(r.width, 20.0, 1.0);
}

#[test]
fn moved_and_turned_over_parts_are_the_same_part() {
    for k in [1e-3, 1.0, 1e3] {
        let a = plate(k);
        for mirror in [false, true] {
            let m = Motion {
                cos: (1.1_f64).pcos(),
                sin: (1.1_f64).psin(),
                mirror,
                shift: [300.0 * k, 12.0 * k],
            };
            let b = a.moved(&m).unwrap();
            let mut work = 1_000_000;
            assert_eq!(
                same_part(&prepare(&a), &prepare(&b), 1e-5 * k, &mut work),
                Some(true)
            );
            assert_eq!(
                same_part(&prepare(&b), &prepare(&a), 1e-5 * k, &mut work),
                Some(true)
            );
        }
        // The slot a little longer: another part.
        let other = Shape::from_loops(
            vec![
                Path::Circle {
                    center: [20.0 * k, 25.0 * k],
                    radius: 5.0 * k,
                },
                rect(0.0, 0.0, 100.0 * k, 50.0 * k),
                slot(50.0 * k, 25.0 * k, 31.0 * k, 4.0 * k),
            ],
            Vec::new(),
        )
        .unwrap();
        let mut work = 1_000_000;
        assert_eq!(
            same_part(&prepare(&a), &prepare(&other), 1e-5 * k, &mut work),
            Some(false)
        );
        // The hole moved: the same counts, area and length, another part.
        let moved_hole = Shape::from_loops(
            vec![
                Path::Circle {
                    center: [20.0 * k, 20.0 * k],
                    radius: 5.0 * k,
                },
                rect(0.0, 0.0, 100.0 * k, 50.0 * k),
                slot(50.0 * k, 25.0 * k, 30.0 * k, 4.0 * k),
            ],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            same_part(&prepare(&a), &prepare(&moved_hole), 1e-5 * k, &mut work),
            Some(false)
        );
        // Out of work: not known.
        let mut none = 0;
        assert_eq!(
            same_part(&prepare(&a), &prepare(&a), 1e-5 * k, &mut none),
            None
        );
    }
}

#[test]
fn a_bend_up_turned_over_is_a_bend_down() {
    let mark = |kind| Mark {
        kind,
        path: Path::Chain {
            start: [0.0, 20.0],
            segments: vec![Segment::Line { to: [100.0, 20.0] }],
            closed: false,
        },
    };
    let up = Shape::from_loops(
        vec![rect(0.0, 0.0, 100.0, 50.0)],
        vec![mark(MarkKind::BendUp)],
    )
    .unwrap();
    let down = Shape::from_loops(
        vec![rect(0.0, 0.0, 100.0, 50.0)],
        vec![mark(MarkKind::BendDown)],
    )
    .unwrap();
    let mut work = 1_000_000;
    // Turned over about the line x = 50 the bend stays at y = 20 and is
    // a bend down.
    assert_eq!(
        same_part(&prepare(&up), &prepare(&down), 1e-6, &mut work),
        Some(true)
    );
    let shifted = Shape::from_loops(
        vec![rect(0.0, 0.0, 100.0, 50.0)],
        vec![Mark {
            kind: MarkKind::BendUp,
            path: Path::Chain {
                start: [0.0, 30.0],
                segments: vec![Segment::Line { to: [100.0, 30.0] }],
                closed: false,
            },
        }],
    )
    .unwrap();
    // Turned end for end (a half turn) the bend at 20 is at 30, still up.
    assert_eq!(
        same_part(&prepare(&up), &prepare(&shifted), 1e-6, &mut work),
        Some(true)
    );
}

#[test]
fn a_kerf_grows_a_rectangle_by_two_kerfs_and_shrinks_a_hole_by_one() {
    for k in [1e-3, 1.0, 1e3] {
        let kerf_k = 0.15 * k;
        let s = plate(k);
        let (out, notes) = kerf(&s, kerf_k, &tolerance(k)).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let b = out.bbox().unwrap();
        close(b[2] - b[0], 100.0 * k + 2.0 * kerf_k, k);
        close(b[3] - b[1], 50.0 * k + 2.0 * kerf_k, k);
        // Steiner: a + p d + π d² for the convex outline.
        let outer = out.outline.area().abs();
        close(
            outer,
            5000.0 * k * k + 300.0 * k * kerf_k + PI * kerf_k * kerf_k,
            k,
        );
        let circle = out
            .holes
            .iter()
            .find_map(|h| match h {
                Path::Circle { radius, .. } => Some(*radius),
                _ => None,
            })
            .unwrap();
        close(circle, 5.0 * k - kerf_k, k);
        // The slot keeps its length and loses a kerf of radius.
        let slot_area = out
            .holes
            .iter()
            .filter(|h| matches!(h, Path::Chain { .. }))
            .map(|h| h.area().abs())
            .sum::<f64>();
        let r = 4.0 * k - kerf_k;
        close(slot_area, 30.0 * k * 2.0 * r + PI * r * r, k);
        // Arcs stay arcs.
        if let Path::Chain { segments, .. } = &out.outline {
            assert_eq!(
                segments
                    .iter()
                    .filter(|s| matches!(s, Segment::Arc { .. }))
                    .count(),
                4
            );
        }
    }
}

#[test]
fn a_kerf_that_closes_a_hole_or_a_slot_is_refused() {
    let s = plate(1.0);
    let e = kerf(&s, 4.5, &tolerance(1.0)).unwrap_err();
    assert!(e.contains("largest"), "{e}");
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(kerf(&s, bad, &tolerance(1.0)).is_err());
    }
    // A notch 2 wide in the outline: a kerf of 1.5 closes it.
    let notched = Path::Chain {
        start: [0.0, 0.0],
        segments: vec![
            Segment::Line { to: [49.0, 0.0] },
            Segment::Line { to: [49.0, 30.0] },
            Segment::Line { to: [51.0, 30.0] },
            Segment::Line { to: [51.0, 0.0] },
            Segment::Line { to: [100.0, 0.0] },
            Segment::Line { to: [100.0, 50.0] },
            Segment::Line { to: [0.0, 50.0] },
            Segment::Line { to: [0.0, 0.0] },
        ],
        closed: true,
    };
    let n = Shape::from_loops(vec![notched], Vec::new()).unwrap();
    assert!(kerf(&n, 0.5, &tolerance(1.0)).is_ok());
    // The notch is 2 wide: a kerf of 1.5 would fill it, so the beam would
    // not cut it; up to 1 it does.
    let e = kerf(&n, 1.5, &tolerance(1.0)).unwrap_err();
    assert!(e.contains("cannot cut"), "{e}");
    let largest: f64 = e.rsplit(' ').next().unwrap().parse().unwrap();
    assert!((largest - 1.0).abs() < 1.5 / 1000.0, "{e}");
    // An ellipse is not offset.
    let el = Shape::from_loops(
        vec![Path::Ellipse {
            center: [0.0, 0.0],
            major: [10.0, 0.0],
            ratio: 0.5,
        }],
        Vec::new(),
    )
    .unwrap();
    assert!(
        kerf(&el, 0.1, &tolerance(1.0))
            .unwrap_err()
            .contains("ellipse")
    );
    assert_eq!(kerf(&el, 0.0, &tolerance(1.0)).unwrap().0, el);
}

/// A 100 × 50 plate with two D holes, flats at x = 20 and x = 40, radius
/// 5 about y = 25: the first bulging right, the second left, or the other
/// way round. A half circle and its opposite have the same ends, centre
/// and length; the point half way along tells them apart.
fn d_holes(first_right: bool) -> Shape {
    let d = |x: f64, right: bool| Path::Chain {
        start: [x, 20.0],
        segments: vec![
            Segment::Arc {
                to: [x, 30.0],
                center: [x, 25.0],
                radius: 5.0,
                ccw: right,
            },
            Segment::Line { to: [x, 20.0] },
        ],
        closed: true,
    };
    Shape::from_loops(
        vec![
            rect(0.0, 0.0, 100.0, 50.0),
            d(20.0, first_right),
            d(40.0, !first_right),
        ],
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn a_half_circle_is_not_its_opposite() {
    let (a, b) = (d_holes(true), d_holes(false));
    assert!((a.area() - b.area()).abs() < 1e-9);
    let mut work = 1_000_000;
    assert_eq!(
        same_part(&prepare(&a), &prepare(&b), 1e-6, &mut work),
        Some(false)
    );
    assert_eq!(
        same_part(&prepare(&a), &prepare(&a), 1e-6, &mut work),
        Some(true)
    );
    // Turned a half turn about the plate's middle, b is a moved: the holes
    // at x = 60 and 80. Not b itself.
    let turned = a
        .moved(&Motion {
            cos: -1.0,
            sin: 0.0,
            mirror: false,
            shift: [100.0, 50.0],
        })
        .unwrap();
    assert_eq!(
        same_part(&prepare(&a), &prepare(&turned), 1e-6, &mut work),
        Some(true)
    );
}

#[test]
fn a_spline_with_other_knots_or_weights_is_another_piece() {
    let spline = |knots: Vec<f64>, weights: Option<Vec<f64>>| {
        let s = Spline::new(
            2,
            knots,
            vec![[100.0, 0.0], [100.0, 40.0], [50.0, 60.0], [0.0, 0.0]],
            weights,
        )
        .unwrap();
        Shape::from_loops(
            vec![Path::Chain {
                start: [0.0, 0.0],
                segments: vec![Segment::Line { to: [100.0, 0.0] }, Segment::Spline(s)],
                closed: true,
            }],
            Vec::new(),
        )
        .unwrap()
    };
    let a = spline(vec![0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0], None);
    let knots = spline(vec![0.0, 0.0, 0.0, 0.3, 1.0, 1.0, 1.0], None);
    let weighed = spline(
        vec![0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0],
        Some(vec![1.0, 2.0, 1.0, 1.0]),
    );
    let mut work = 1_000_000;
    let tol = 1e-6;
    assert_eq!(
        same_part(&prepare(&a), &prepare(&a), tol, &mut work),
        Some(true)
    );
    // Reversed with its knots: the same piece.
    let reversed = a
        .moved(&Motion {
            cos: 1.0,
            sin: 0.0,
            mirror: true,
            shift: [0.0, 0.0],
        })
        .unwrap();
    assert_eq!(
        same_part(&prepare(&a), &prepare(&reversed), tol, &mut work),
        Some(true)
    );
    // The same control points with other knots or weights are other
    // curves; the cheap measures may already tell, the pieces must.
    assert_eq!(
        same_part(&prepare(&a), &prepare(&knots), tol, &mut work),
        Some(false)
    );
    assert_eq!(
        same_part(&prepare(&a), &prepare(&weighed), tol, &mut work),
        Some(false)
    );
}

#[test]
fn an_inner_round_smaller_than_the_kerf_comes_out_at_the_kerf() {
    // An L whose inner corner is rounded with radius 1: a kerf of 2 cannot
    // follow it, as it cannot follow a sharp one; both leave radius 2.
    let l = Path::Chain {
        start: [0.0, 0.0],
        segments: vec![
            Segment::Line { to: [60.0, 0.0] },
            Segment::Line { to: [60.0, 20.0] },
            Segment::Line { to: [21.0, 20.0] },
            Segment::Arc {
                to: [20.0, 21.0],
                center: [21.0, 21.0],
                radius: 1.0,
                ccw: false,
            },
            Segment::Line { to: [20.0, 100.0] },
            Segment::Line { to: [0.0, 100.0] },
            Segment::Line { to: [0.0, 0.0] },
        ],
        closed: true,
    };
    let s = Shape::from_loops(vec![l], Vec::new()).unwrap();
    let (out, notes) = kerf(&s, 2.0, &tolerance(1.0)).unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("radius 1") && notes[0].contains("segment 3"),
        "{notes:?}"
    );
    // The path's inner corner is where the two lines' offsets meet.
    if let Path::Chain { segments, .. } = &out.outline {
        assert!(segments.iter().any(|s| matches!(s, Segment::Line { to } if (to[0] - 22.0).abs() < 1e-9 && (to[1] - 22.0).abs() < 1e-9)), "{segments:?}");
    }
    // A kerf of 0.5 follows the round.
    assert!(kerf(&s, 0.5, &tolerance(1.0)).unwrap().1.is_empty());
}

#[test]
fn a_bite_smaller_than_the_kerf_is_refused_not_rounded() {
    // A bend relief: a half circle of radius 0.5 bitten straight into the
    // top edge of a 100 × 50 plate. A kerf of 0.8 would join the lines'
    // offsets across it, and the bite would vanish from the part.
    let bitten = Path::Chain {
        start: [0.0, 0.0],
        segments: vec![
            Segment::Line { to: [100.0, 0.0] },
            Segment::Line { to: [100.0, 50.0] },
            Segment::Line { to: [50.5, 50.0] },
            Segment::Arc {
                to: [49.5, 50.0],
                center: [50.0, 50.0],
                radius: 0.5,
                ccw: false,
            },
            Segment::Line { to: [0.0, 50.0] },
            Segment::Line { to: [0.0, 0.0] },
        ],
        closed: true,
    };
    let s = Shape::from_loops(vec![bitten], Vec::new()).unwrap();
    let e = kerf(&s, 0.8, &tolerance(1.0)).unwrap_err();
    assert!(
        e.contains("segment 3 of the outline") && e.contains("largest"),
        "{e}"
    );
    let largest: f64 = e.rsplit(' ').next().unwrap().parse().unwrap();
    assert!(largest > 0.0 && largest <= 0.5, "{e}");
    // A kerf below it follows the bite.
    let (_, notes) = kerf(&s, 0.4, &tolerance(1.0)).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
}
