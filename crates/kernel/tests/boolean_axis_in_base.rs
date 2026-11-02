//! Union of two cylinders square to each other, where the axis of one lies
//! in the base plane of the other.
//!
//! A: axis x, radius 5, x from 17.5 to 27.5, about (y, z) = (30, 7.5).
//! B: axis y, radius 12.5, y from 30 to 35, about (x, z) = (17.5, 15).
//! A's axis lies in B's base plane y = 30, and its wall touches B's top
//! plane y = 35 along the line z = 7.5. B's wall meets A's end plane
//! x = 27.5 at z = 7.5 and is tangent to A's wall at (17.5, 30, 2.5). So
//! four faces meet at (27.5, 35, 7.5): A's wall, A's end disc, B's wall
//! and B's top. B's top circle runs onto A's wall there tangentially and
//! is placed along it only to a fraction of the tolerance, while A's end
//! circle meets B's wall square. Both are one cut of the curve in which
//! the walls meet. Before 2026-11-02 the edge on that curve began where
//! the first of the two lay along it, 0.09 tolerances past A's end plane:
//! it left the faces it bounds and came back, and the result could not be
//! meshed (B's wall: "the boundary crosses itself in parameter space"; at
//! the origin A's wall: "the surface turns away from the chords").
//!
//! Volume: `V = Va + Vb − Vi` with `Va = 250 π`, `Vb = 781.25 π`, and
//! `Vi = ∫ L(z) min(10, w(z)) dz` over z from 2.5 to 12.5, where
//! `L = √(25 − (z − 7.5)²)` is how far A reaches past y = 30 (B only has
//! y ≥ 30) and `w = √(156.25 − (z − 15)²)` how far B reaches past
//! x = 17.5 (A only has x ≥ 17.5, and 10 of it). For z ≥ 7.5, `w ≥ 10`
//! and the strip is a quarter disc times 10: `62.5 π`. Below, the
//! integral is taken here by Simpson's rule after `z = 7.5 − 5 cos s`,
//! which leaves a smooth integrand.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::BooleanOp::Union;
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Portable, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::Body;
use std::f64::consts::PI;

/// A cylinder placed as a document places one: its axis, and the x axis
/// `Frame::from_z` picks for it.
fn cylinder(origin: Vec3, axis: Vec3, radius: f64, height: f64) -> Body {
    let upright = Frame::from_z(origin, axis).unwrap();
    let frame = Frame::new(origin, axis, upright.x()).unwrap();
    primitives::cylinder(&frame, radius, height).unwrap()
}

/// The two operands at scale `k`, moved by `shift` (in model units of
/// scale 1).
fn operands(k: f64, shift: Vec3) -> (Body, Body) {
    (
        cylinder(
            (Vec3::new(17.5, 30.0, 7.5) + shift) * k,
            Vec3::X,
            5.0 * k,
            10.0 * k,
        ),
        cylinder(
            (Vec3::new(17.5, 30.0, 15.0) + shift) * k,
            Vec3::Y,
            12.5 * k,
            5.0 * k,
        ),
    )
}

/// The volume of the union at scale 1, from the closed form above.
fn expected_volume() -> f64 {
    let lower = |s: f64| {
        let z = 7.5 - 5.0 * s.pcos();
        let dz = 5.0 * s.psin();
        let l = (25.0 - (z - 7.5).ppowi(2)).max(0.0).sqrt();
        let w = (156.25 - (z - 15.0).ppowi(2)).max(0.0).sqrt();
        l * w * dz
    };
    let n = 4000;
    let h = 0.5 * PI / f64::from(n);
    let mut sum = lower(0.0) + lower(0.5 * PI);
    for i in 1..n {
        sum += if i % 2 == 1 { 4.0 } else { 2.0 } * lower(h * f64::from(i));
    }
    let common = 62.5 * PI + sum * h / 3.0;
    250.0 * PI + 781.25 * PI - common
}

const SHIFTS: [Vec3; 2] = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(1000.0, -700.0, 300.0)];

#[test]
fn the_closed_form_is_what_a_grid_of_points_finds() {
    // An independent look at the expected volume: membership counted at
    // the middles of a grid of 400 × 160 × 400 cells around the union.
    let expected = expected_volume();
    assert!((expected - 2_893.457_918).abs() < 1e-5, "{expected}");
    let (nx, ny, nz) = (400_u32, 160_u32, 400_u32);
    let (x0, x1, y0, y1, z0, z1) = (5.0, 30.0, 25.0, 35.0, 2.5, 27.5);
    let (dx, dy, dz) = (
        (x1 - x0) / f64::from(nx),
        (y1 - y0) / f64::from(ny),
        (z1 - z0) / f64::from(nz),
    );
    let mut inside = 0_u64;
    for i in 0..nx {
        let x = x0 + (f64::from(i) + 0.5) * dx;
        for k in 0..nz {
            let z = z0 + (f64::from(k) + 0.5) * dz;
            for j in 0..ny {
                let y = y0 + (f64::from(j) + 0.5) * dy;
                let in_a =
                    (17.5..=27.5).contains(&x) && (y - 30.0).ppowi(2) + (z - 7.5).ppowi(2) <= 25.0;
                let in_b = (30.0..=35.0).contains(&y)
                    && (x - 17.5).ppowi(2) + (z - 15.0).ppowi(2) <= 156.25;
                inside += u64::from(in_a || in_b);
            }
        }
    }
    let counted = inside as f64 * dx * dy * dz;
    assert!(
        (counted - expected).abs() <= 1e-3 * expected,
        "{counted} against {expected}"
    );
}

#[test]
fn the_union_is_valid_meshed_and_found_clear_of_itself() {
    let expected = expected_volume();
    for k in SCALES {
        for shift in SHIFTS {
            let what = format!("scale {k}, moved by {shift:?}");
            let (a, b) = operands(k, shift);
            // Valid, and held to the boolean's own check in full.
            let made = solve(&a, &b, Union, k);
            let body = &made.body;
            let exact = mass(body);
            let want = expected * k.ppowi(3);
            assert!(
                (exact.volume - want).abs() <= 1e-6 * want,
                "{what}: volume {} against {want}",
                exact.volume
            );
            assert_eq!(body.faces.len(), 6, "{what}");
            // Watertight at the chords of a display and of an export, with
            // the volume it encloses.
            for chord in [0.05, 0.01, 0.002] {
                let params = TessParams {
                    chordal_tolerance: chord * k,
                    ..TessParams::default()
                };
                let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
                let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
                assert!(welded.is_closed(), "{what}: the mesh is not watertight");
                assert!(
                    (welded.volume() - exact.volume).abs() <= exact.area * chord * k,
                    "{what}: mesh volume {} against {}",
                    welded.volume(),
                    exact.volume
                );
            }
            // What a document checks before it hands the body on.
            let check = body_self_intersections(body, &SelfCheckParams::new(tolerance(k).linear))
                .unwrap_or_else(|e| panic!("{what}: the self-check cannot mesh it: {e}"));
            assert!(
                check.crossings.is_empty() && check.certain_crossings == 0,
                "{what}: {} crossings",
                check.crossings.len()
            );
        }
    }
}

#[test]
fn every_edge_comes_nearest_its_vertices_at_its_ends() {
    // The cause itself: an edge that runs on past its vertex comes nearer
    // to it a little way in than at its end. Before 2026-11-02 the edge
    // where the walls meet began 0.09 tolerances from its vertex and passed
    // within 1e-4 tolerances of it a little way in.
    for k in SCALES {
        for shift in SHIFTS {
            let (a, b) = operands(k, shift);
            let body = solve(&a, &b, Union, k).body;
            let slack = 1e-3 * tolerance(k).linear;
            for (index, edge) in body.edges.iter().enumerate() {
                let (t0, t1) = edge.range;
                let ends = [
                    (t0, t1 - t0, body.vertices[edge.start.0 as usize].point),
                    (t1, t0 - t1, body.vertices[edge.end.0 as usize].point),
                ];
                for (end, inward, vertex) in ends {
                    let at_end = edge.curve.eval(end).distance(vertex);
                    for step in 1..=40 {
                        let t = end + inward * 0.5_f64.ppowi(step);
                        let d = edge.curve.eval(t).distance(vertex);
                        assert!(
                            d >= at_end - slack,
                            "scale {k}, moved by {shift:?}: edge {index} is {at_end:e} from its \
                             vertex at its end and {d:e} a little way in"
                        );
                    }
                }
            }
        }
    }
}
