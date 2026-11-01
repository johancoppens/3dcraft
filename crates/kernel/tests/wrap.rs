//! A profile of short curved pieces wrapped deep onto a cylinder: the rims
//! of the plate face outward (2026-10-30: a rim's side was probed at a
//! station far past its short edge, and the rims of the r and t of 3DCraft
//! faced inward). The plate is valid, every face's loops bound a region,
//! and it meshes; on the face turned over (a bore) too.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Cylinder, Surface};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::wrap::wrap;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::check;

/// Three arcs of a circle of radius `r` about `(x, y)`, a third of a turn
/// each.
fn three_arcs(x: f64, y: f64, r: f64) -> Vec<SketchCurve> {
    let at = |k: f64| {
        let a = k * std::f64::consts::TAU / 3.0;
        [x + r * a.pcos(), y + r * a.psin()]
    };
    (0..3)
        .map(|k| SketchCurve {
            id: k + 1,
            curve: Curve2::Arc {
                center: [x, y],
                start: at(k as f64),
                end: at(k as f64 + 1.0),
            },
        })
        .collect()
}

#[test]
fn short_curved_pieces_wrapped_deep_have_rims_facing_out() {
    for k in [1e-2, 1.0, 1e2] {
        let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let profiles = detect_profiles(&three_arcs(0.0, 5.0 * k, 0.15 * k), &tolerance).unwrap();
        let surface = Surface::Cylinder(Cylinder::new(Frame::WORLD, 20.0 * k).unwrap());
        for reversed in [false, true] {
            for depth in [k, -k] {
                let made = wrap(&profiles, &surface, reversed, depth, &tolerance)
                    .unwrap_or_else(|e| panic!("k {k}, reversed {reversed}, depth {depth}: {e}"));
                assert_eq!(check(&made.body, &tolerance), [], "k {k}");
                let params = craft3d_tess::TessParams {
                    chordal_tolerance: 1e-3 * k,
                    ..craft3d_tess::TessParams::default()
                };
                craft3d_tess::tessellate_body(&made.body, &params)
                    .unwrap_or_else(|e| panic!("k {k}, reversed {reversed}, depth {depth}: {e}"));
            }
        }
    }
}

/// A quadratic span of a glyph's outline from `a` through the control
/// point `c` to `b`.
fn quad(id: u64, a: [f64; 2], c: [f64; 2], b: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control: vec![a, c, b],
        }),
    }
}

/// An oval of four quadratic spans about `(x, y)`, half-axes `a` and `b`,
/// tangent where they join and of different curvature there (as the
/// spans of a TrueType glyph are).
fn oval(x: f64, y: f64, a: f64, b: f64) -> Vec<SketchCurve> {
    vec![
        quad(1, [x + a, y], [x + a, y + b], [x, y + b]),
        quad(2, [x, y + b], [x - 0.6 * a, y + b], [x - a, y]),
        quad(3, [x - a, y], [x - a, y - b], [x, y - b]),
        quad(4, [x, y - b], [x + 0.6 * a, y - b], [x + a, y]),
    ]
}

/// The spans of a glyph wrapped onto a cylinder, raised and sunk: the
/// corners where two curved rims are tangent stay on their seam, and the
/// edge across each corner, its points found on the seam, is traced to
/// them (2026-11-01: `tweak::on_seam` took such a corner 1e-3 mm along
/// the front skin, past the end of a rim, and the edge across it could
/// not be traced in 4096 points, still 2.8e-4 mm off; 434 of 752 glyphs
/// of the text font failed so).
#[test]
fn an_oval_of_quadratic_spans_wraps_raised_and_sunk() {
    for k in [1e-2, 1.0, 1e2] {
        let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let profiles = detect_profiles(&oval(0.0, 12.0 * k, 2.0 * k, 3.0 * k), &tolerance).unwrap();
        let surface = Surface::Cylinder(Cylinder::new(Frame::WORLD, 20.0 * k).unwrap());
        for depth in [k, -k] {
            let made = wrap(&profiles, &surface, false, depth, &tolerance)
                .unwrap_or_else(|e| panic!("k {k}, depth {depth}: {e}"));
            assert_eq!(check(&made.body, &tolerance), [], "k {k}, depth {depth}");
        }
    }
}
