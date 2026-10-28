//! A loop is inside another when its first point is (winding number). A
//! circle whose first point (its rightmost) lies on the chord of an arc of
//! the loop around it, between the arc's ends, was found outside: the
//! arc's turn about the point is ±π by the sign of a zero cross product,
//! and a `-0.0` took the wrong one (found by sheet metal's arch outline,
//! 2026-10-28). At three scales and both ways round.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::Tolerance;
use std::f64::consts::PI;

#[test]
fn a_circle_level_with_an_arcs_ends_is_a_hole() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        // An arch: a 20 × 10 rectangle closed below by a half circle of
        // radius 10, with circles of radius 3 at the half circle's centre
        // and 4 to its left (their rightmost points on its chord).
        for dx in [0.0, -4.0, 4.0] {
            let s = |x: f64, y: f64| [x * k, y * k];
            let curves = vec![
                SketchCurve {
                    id: 1,
                    curve: Curve2::Line {
                        start: s(20.0, 0.0),
                        end: s(0.0, 0.0),
                    },
                },
                SketchCurve {
                    id: 2,
                    curve: Curve2::Line {
                        start: s(0.0, 0.0),
                        end: s(0.0, -10.0),
                    },
                },
                SketchCurve {
                    id: 3,
                    curve: Curve2::Arc {
                        center: s(10.0, -10.0),
                        start: s(0.0, -10.0),
                        end: s(20.0, -10.0),
                    },
                },
                SketchCurve {
                    id: 4,
                    curve: Curve2::Line {
                        start: s(20.0, -10.0),
                        end: s(20.0, 0.0),
                    },
                },
                SketchCurve {
                    id: 5,
                    curve: Curve2::Circle {
                        center: s(10.0 + dx, -10.0),
                        radius: 3.0 * k,
                    },
                },
            ];
            let found = detect_profiles(&curves, &tol).unwrap();
            assert_eq!(found.len(), 1, "dx {dx}, k {k}");
            assert_eq!(found[0].holes.len(), 1, "dx {dx}, k {k}");
            let area = (200.0 + 50.0 * PI - 9.0 * PI) * k * k;
            assert!(
                (found[0].area() - area).abs() <= 1e-9 * area,
                "dx {dx}, k {k}"
            );
        }
    }
}
