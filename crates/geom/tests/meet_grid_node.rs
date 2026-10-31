//! A zero exactly on a node of the subdivision (2026-10-31, the planetary
//! gearbox's planet cut at its mid-plane, docs/design/booleans.md › *A
//! plane through a node of the marcher's grid*).
//!
//! Where the signed distance is linear across a cell (a wall extruded
//! along z against a plane square to it, a straight curve running at a
//! plane), the distance at the cell's centre equals the cell's reach when
//! the zero lies on one of its ends, and a rounding of the centre a hair
//! further off proved the cell clear; the cell beyond counted the end's
//! zero with its own sign and showed no change, and the curve or the hit
//! was lost. A cell is now clear only by more than rounding. Both tests
//! sweep a bottom a hair below z = 0 (rounding-sized, as a pose leaves it)
//! at three scales, so that the zero lands on a node. They are guards: a
//! sweep of 9000 such placements lost nothing before the fix either, since
//! whether a centre rounds past its reach is a matter of the last bit. The
//! rule itself is pinned by `march.rs` › `a_cell_whose_zero_is_on_a_node_
//! is_not_clear_of_zero` and `curves.rs` › `an_interval_with_a_crossing_
//! on_its_end_is_not_clear`, and the planet's case by the kernel's
//! `boolean_grid_node.rs`, which failed before.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::intersect::{
    CurveIntersection, SurfaceMeeting, intersect_curve_surface, meet_surfaces,
};
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// Bottoms a hair below z = 0, as multiples of 1e-16 of the height of 10
/// (times the scale): 1 to 64.
fn hairs(k: f64) -> impl Iterator<Item = f64> {
    (1..=64).map(move |j| f64::from(j) * 1e-16 * k)
}

fn plane_at(z: f64) -> Surface {
    Surface::Plane(Plane::new(
        Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap(),
    ))
}

/// A narrow wall: a cubic in x and y (1 wide, dipping 0.12, like a tooth
/// gap's root) extruded along z from `-hair` to 10 (times `k`), degree one
/// in v so that z is linear in v.
fn wall(k: f64, hair: f64) -> Surface {
    let profile = [
        (-0.5, 0.0),
        (-0.3, -0.12),
        (-0.1, -0.15),
        (0.1, -0.15),
        (0.3, -0.12),
        (0.5, 0.0),
    ];
    let mut ctrl = Vec::new();
    for (x, y) in profile {
        ctrl.push(Vec3::new(x * k, y * k, -hair));
        ctrl.push(Vec3::new(x * k, y * k, 10.0 * k));
    }
    Surface::Nurbs(
        NurbsSurface::new(
            3,
            1,
            vec![0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            6,
            2,
            ctrl,
            vec![1.0; 12],
        )
        .unwrap(),
    )
}

#[test]
fn a_plane_through_a_node_of_the_grid_meets_a_wall_linear_across_it() {
    for k in SCALES {
        let tol = tolerance(k);
        // The box a boolean asks in: the wall's own, grown a little, and a
        // slab about the plane.
        let within = Bbox3::from_points([
            Vec3::new(-0.6 * k, -0.3 * k, 4.0 * k),
            Vec3::new(0.6 * k, 0.1 * k, 6.0 * k),
        ]);
        let mut lost = Vec::new();
        for hair in hairs(k) {
            let met = meet_surfaces(&wall(k, hair), &plane_at(5.0 * k), &within, &tol);
            let Ok(SurfaceMeeting::Meets(meeting)) = met else {
                lost.push(format!("k {k}, bottom at {:e}: {met:?}", -hair));
                continue;
            };
            // One curve across the wall, from its one end to the other,
            // in the plane.
            assert_eq!(meeting.curves.len(), 1, "k {k}, hair {hair:e}");
            let c = &meeting.curves[0];
            let (a, b) = (c.curve.eval(c.range.0), c.curve.eval(c.range.1));
            let span = (a.x - b.x).abs();
            assert!(
                (span - k).abs() <= 4.0 * tol.linear,
                "k {k}, hair {hair:e}: spans {span}"
            );
            for s in 0..=16 {
                let p = c
                    .curve
                    .eval(c.range.0 + (c.range.1 - c.range.0) * f64::from(s) / 16.0);
                assert!((p.z - 5.0 * k).abs() <= tol.linear, "k {k}: {p:?}");
            }
        }
        assert!(lost.is_empty(), "{} lost:\n{}", lost.len(), lost.join("\n"));
    }
}

#[test]
fn a_straight_spline_meets_a_plane_on_a_node_of_its_intervals() {
    for k in SCALES {
        let tol = tolerance(k);
        let mut lost = Vec::new();
        for hair in hairs(k) {
            // A cubic with its control points evenly along z, so z is
            // linear in t, from -hair to 10: the plane z = 5 crosses it at
            // t = 1/2, a node of the sixteen pieces it starts from.
            let step = (10.0 * k + hair) / 3.0;
            let ctrl: Vec<Vec3> = (0..4)
                .map(|i| Vec3::new(0.25 * k, -0.5 * k, -hair + step * f64::from(i)))
                .collect();
            let curve = Curve3::Nurbs(
                NurbsCurve::new(
                    3,
                    vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                    ctrl,
                    vec![1.0; 4],
                )
                .unwrap(),
            );
            match intersect_curve_surface(&curve, &plane_at(5.0 * k), &tol) {
                Ok(CurveIntersection::Hits(hits)) if hits.len() == 1 => {
                    let h = hits[0];
                    assert!((h.point.z - 5.0 * k).abs() <= tol.linear, "{h:?}");
                    assert!((h.t - 0.5).abs() <= 1e-9, "k {k}: {h:?}");
                }
                other => lost.push(format!("k {k}, bottom at {:e}: {other:?}", -hair)),
            }
        }
        assert!(lost.is_empty(), "{} lost:\n{}", lost.len(), lost.join("\n"));
    }
}
