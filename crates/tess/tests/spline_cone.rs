//! The cone a straight spline sweeps from the axis, uncut: its apex is a
//! pole of the first order of its spline surface and every ruling is
//! straight, so the apex and two points of one ruling are three points on
//! a line. Such triangles were kept as rounding and refused at output
//! ("folds three distinct points onto one line") at most tolerances until
//! 2026-10-17 (`docs/design/tessellator.md` › *Cuts along the axis of a
//! cusp and of a spline cone*).

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::solids::spline_cone;

#[test]
fn an_uncut_spline_cone_meshes_closed_within_its_volume() {
    for k in [1e-3, 1.0, 1e6] {
        let frame = Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * k,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::X,
        )
        .unwrap();
        let cone = spline_cone(&frame, 3.0 * k, 6.0 * k).unwrap();
        for relative in [1e-1, 3e-2, 1e-2, 3e-3, 1e-3, 3e-4, 1e-4] {
            let what = format!("spline cone at {k}, tolerance {relative} of its size");
            let chord = relative * 6.0 * k;
            let params = TessParams {
                chordal_tolerance: chord,
                ..TessParams::default()
            };
            let mesh =
                tessellate_body(&cone.body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
            let welded = mesh.weld().unwrap();
            assert!(welded.is_closed(), "{what}: not closed");
            let off = (welded.volume() - cone.volume).abs();
            assert!(off <= cone.area * chord, "{what}: volume off by {off:e}");
        }
    }
}
