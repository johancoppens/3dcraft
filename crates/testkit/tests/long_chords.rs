//! Long straight chords of flat faces (`crates/tess/src/long.rs`): a
//! plate a hundred holes long, whose long edges were one chord each and
//! whose top and bottom were triangulated as fans of slivers from the
//! plate's corners across the whole plate (2026-10-15: on a plate of
//! 4010 mm with 400 holes the self-intersection check could prune
//! nothing, 140 ms of a boolean's 200).
//!
//! Oracles: no triangle of the plate is longer than a few hole spacings;
//! the mesh is closed and its volume the closed form's within the
//! tolerance times the area; the same mesh on one and eight threads; the
//! same triangles at every scale; and faces with no fan to undo (a long
//! block, a half disc over its diameter, a plate with one hole far from
//! its edges) are meshed as before.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "../../kernel/tests/boolean_common/mod.rs"]
mod boolean_common;

use boolean_common::{SCALES, tolerance};
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_topo::{Body, merge};
use std::f64::consts::PI;

const HOLES: usize = 100;

/// A plate `10 n + 10` by 30 by 5 with a row of `n` holes of radius 3, 10
/// apart, 10 from its long edge at y = 0; `k` times as large.
fn plate(k: f64, n: usize) -> Body {
    let tol = tolerance(k);
    let at = |x: f64, y: f64, z: f64| Frame::WORLD.at(Vec3::new(x * k, y * k, z * k)).unwrap();
    let cutters: Vec<Body> = (0..n)
        .map(|i| {
            primitives::cylinder(&at(10.0 + 10.0 * i as f64, 10.0, -1.0), 3.0 * k, 7.0 * k).unwrap()
        })
        .collect();
    let row = merge(&cutters.iter().collect::<Vec<_>>()).unwrap().0;
    let block = primitives::block(
        &Frame::WORLD,
        (10.0 * n as f64 + 10.0) * k,
        30.0 * k,
        5.0 * k,
    )
    .unwrap();
    boolean(&block, &row, BooleanOp::Subtract, &tol)
        .unwrap()
        .body
}

fn params(k: f64) -> TessParams {
    TessParams {
        chordal_tolerance: 0.05 * k,
        ..TessParams::default()
    }
}

fn longest_side(mesh: &Mesh) -> f64 {
    let mut longest = 0.0_f64;
    for tri in &mesh.triangles {
        for i in 0..3 {
            let (p, q) = (tri[i] as usize, tri[(i + 1) % 3] as usize);
            longest = longest.max(mesh.positions[p].distance(mesh.positions[q]));
        }
    }
    longest
}

#[test]
fn a_plate_with_a_row_of_holes_has_no_fans_across_it() {
    let mut counts = Vec::new();
    for k in SCALES {
        let body = plate(k, HOLES);
        let p = params(k);
        let mesh = tessellate_body(&body, &p).unwrap();
        let welded = mesh.weld().unwrap();
        assert!(welded.is_closed(), "at {k}: not closed");
        let length = 10.0 * HOLES as f64 + 10.0;
        let volume = (length * 30.0 * 5.0 - HOLES as f64 * PI * 9.0 * 5.0) * k * k * k;
        let area = (2.0 * (length * 30.0 - HOLES as f64 * PI * 9.0)
            + 2.0 * 5.0 * (length + 30.0)
            + HOLES as f64 * 2.0 * PI * 3.0 * 5.0)
            * k
            * k;
        let off = (welded.volume() - volume).abs();
        assert!(
            off <= p.chordal_tolerance * area,
            "at {k}: volume off by {off}"
        );
        // Before: fans from the corners across the plate, sides of the
        // whole length (1010). Now the long edges are cut where the
        // holes are near: into pieces no longer than eight times their
        // distance from the holes (7 from the near edge, 17 from the far
        // one: 1010 / 8 = 126 there).
        let longest = longest_side(&mesh) / k;
        assert!(longest < 8.0 * 17.0, "at {k}: a side {longest} long");
        for threads in [1, 8] {
            let again = tessellate_body_with_threads(&body, &p, threads).unwrap();
            assert_eq!(again, mesh, "at {k}: {threads} threads");
        }
        counts.push(mesh.triangles.len());
    }
    assert!(counts.windows(2).all(|w| w[0] == w[1]), "{counts:?}");
}

#[test]
fn faces_without_a_fan_are_left_as_they_were() {
    for k in SCALES {
        let p = params(k);
        // A long block: two triangles a face, however slender.
        let block = primitives::block(&Frame::WORLD, 4010.0 * k, 30.0 * k, 5.0 * k).unwrap();
        assert_eq!(
            tessellate_body(&block, &p).unwrap().triangles.len(),
            12,
            "at {k}"
        );

        // A half disc over its diameter: the arc meets the diameter at
        // both its ends, its points are not in front of it.
        let r = 20.0 * k;
        let curves = [
            SketchCurve {
                id: 1,
                curve: Curve2::Line {
                    start: [-r, 0.0],
                    end: [r, 0.0],
                },
            },
            SketchCurve {
                id: 2,
                curve: Curve2::Arc {
                    center: [0.0, 0.0],
                    start: [r, 0.0],
                    end: [-r, 0.0],
                },
            },
        ];
        let tol = tolerance(k);
        let profiles = detect_profiles(&curves, &tol).unwrap();
        let half = extrude(&profiles, &Frame::WORLD, 0.0, 5.0 * k, &tol)
            .unwrap()
            .body;
        let mesh = tessellate_body(&half, &p).unwrap();
        let diameter = mesh
            .positions
            .iter()
            .filter(|v| v.y.abs() <= 1e-9 * k && v.x.abs() < r * (1.0 - 1e-9))
            .count();
        assert_eq!(diameter, 0, "at {k}: the diameter was cut");

        // One hole far from every edge: no point in front of an edge
        // nearer than an eighth of its length.
        let one = plate(k, 1);
        let mesh = tessellate_body(&one, &p).unwrap();
        let on_edges = mesh
            .positions
            .iter()
            .filter(|v| {
                let (x, y) = (v.x / k, v.y / k);
                (y.abs() < 1e-6 || (y - 30.0).abs() < 1e-6) && x > 1e-6 && x < 20.0 - 1e-6
            })
            .count();
        assert_eq!(on_edges, 0, "at {k}: the plate's long edges were cut");
    }
}
