//! A chamfer whose line across the edge would run through a face beside
//! the edge, through the commands (the kernel's side, and the property
//! over random blends: crates/kernel/tests/blend_through.rs and
//! blend_monotone.rs). A chamfer is the face that joins two cut lines,
//! each its distance from the edge along its face; past the size at which
//! the line between them comes to lie along a face it has no such face,
//! and once the lens the line cuts from that face is deeper than the
//! tolerance the feature fails as too large, with the largest that fits,
//! which then builds. What was built there until 2026-11-13 passed
//! the deep check and was not the chamfer.
//!
//! The oracles are worked out by hand in the plane across the edge: the
//! plate 60 x 40 x 10 and the pipe of radius 10 along x whose axis is at
//! (y, z) = (20, 16) leave the top at y = 12 and y = 28 (8 either side of
//! the axis, 6 below it); a chamfer `a` along the top and `b` of arc along
//! the pipe adds (to the pipe on the plate) or takes (from the groove the
//! pipe leaves in it) the triangle of the edge and its two feet less the
//! pipe's segment between the edge and the foot on it, along the length
//! 60, at each of the two edges the pair of faces names. At the scales
//! 1e-3, 1 and 1e6, away from the origin.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{assert_solid, built, failure, new_document, run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// Where the plate's corner lies, at scale 1.
const AT: [f64; 3] = [120.0, -75.0, 40.0];

/// The top of the plate and the wall of the pipe, as their union and the
/// plate less the pipe name them.
const TOP: &str = "F3:copy(0)/F1:face(1)";
const WALL: &str = "F3:copy(1)/F2:face(0)";

fn at(k: f64, p: [f64; 3]) -> Value {
    json!([(AT[0] + p[0]) * k, (AT[1] + p[1]) * k, (AT[2] + p[2]) * k])
}

/// The plate and the pipe, united (`union`) or the pipe taken out, as
/// `Joined`.
fn plate_and_pipe(s: &mut Session, k: f64, union: bool) {
    new_document(s, k);
    built(
        s,
        "feature.block",
        json!({"size": [60.0 * k, 40.0 * k, 10.0 * k], "position": at(k, [0.0, 0.0, 0.0]),
               "name": "Plate"}),
    );
    built(
        s,
        "feature.cylinder",
        json!({"radius": 10.0 * k, "height": 60.0 * k, "position": at(k, [0.0, 20.0, 16.0]),
               "axis": [1, 0, 0], "name": "Pipe"}),
    );
    built(
        s,
        "feature.boolean",
        json!({"op": if union { "union" } else { "subtract" }, "target": "Plate",
               "tool": "Pipe", "name": "Joined"}),
    );
}

/// The pipe's segment below the top: the area the pipe and the plate
/// share, and the groove's.
fn dip() -> f64 {
    100.0 * 0.6_f64.pacos() - 6.0 * 8.0
}

/// The foot on the pipe `b` of arc from the edge at (28, 10): up the
/// pipe's wall, or down the groove's.
fn on_pipe(b: f64, up: bool) -> [f64; 2] {
    let start = (-6.0_f64).patan2(8.0);
    let t = start + if up { b / 10.0 } else { -b / 10.0 };
    [20.0 + 10.0 * t.pcos(), 16.0 + 10.0 * t.psin()]
}

/// What the chamfer adds or takes at one edge, per unit of length.
fn section(a: f64, b: f64, up: bool) -> f64 {
    let (e, g1, g2) = ([28.0, 10.0], [28.0 + a, 10.0], on_pipe(b, up));
    let triangle = 0.5 * ((g1[0] - e[0]) * (g2[1] - e[1]) - (g2[0] - e[0]) * (g1[1] - e[1])).abs();
    let phi = b / 10.0;
    triangle - 50.0 * (phi - phi.psin())
}

/// How deep the lens is that the line from the foot on the pipe to the
/// one on the top cuts from the pipe (scale 1): `R (1 - cos α)`, α the
/// angle between the line and the pipe's tangent at its foot (the sine of
/// it the clearance `(G1 - G2) . (G2 - C)` over the line's length and the
/// radius); none where the line leaves the pipe outward.
fn lens(a: f64, b: f64, up: bool) -> f64 {
    let (g1, g2) = ([28.0 + a, 10.0], on_pipe(b, up));
    let clearance = (g1[0] - g2[0]) * (g2[0] - 20.0) + (g1[1] - g2[1]) * (g2[1] - 16.0);
    let sine = clearance / ((g1[0] - g2[0]).phypot(g1[1] - g2[1]) * 10.0);
    if sine >= 0.0 {
        0.0
    } else {
        10.0 * (1.0 - (1.0 - sine * sine).sqrt())
    }
}

/// The tolerance at scale 1 (the documents' 1e-6).
const DEEP: f64 = 1e-6;

/// The multiple of the size `(a, b)` at which its line enters the pipe by
/// a lens deeper than [`DEEP`], by bisection.
fn touching(a: f64, b: f64, up: bool) -> f64 {
    let deep = |s: f64| lens(s * a, s * b, up) > DEEP;
    let (mut lo, mut hi) = (1e-6, 1.0);
    assert!(!deep(lo) && deep(hi));
    for _ in 0..200 {
        let middle = 0.5 * (lo + hi);
        if deep(middle) {
            hi = middle;
        } else {
            lo = middle;
        }
    }
    0.5 * (lo + hi)
}

/// The size said to be the largest that fits, in model units.
fn said_largest(reason: &str) -> f64 {
    reason
        .rsplit("below ")
        .next()
        .unwrap()
        .trim_end_matches(" mm")
        .parse()
        .unwrap_or_else(|_| panic!("{reason}"))
}

/// The plate and the pipe (or the groove), both edges between the top and
/// the pipe chamfered `b` along the pipe and 1 along the top: the closed
/// form while the line stays clear of the pipe or enters it by a lens
/// within the tolerance, too large past that, with the multiple of the
/// size at which the lens grows past the tolerance; that
/// size, a hundredth under, builds and is the closed form.
fn the_pair_along_the_pipe(union: bool, fits: &[f64], past: &[f64]) {
    let a = 1.0;
    let v0 = if union {
        24000.0 + 60.0 * (PI * 100.0 - dip())
    } else {
        24000.0 - 60.0 * dip()
    };
    let sign = if union { 1.0 } else { -1.0 };
    for k in SCALES {
        let mut s = session();
        plate_and_pipe(&mut s, k, union);
        assert_solid(&mut s, "Joined", v0, None, k);
        for &b in fits {
            assert!(lens(a, b, union) <= DEEP, "{b}");
            let _ = s.execute("feature.delete", &json!({"feature": "Bevel"}));
            built(
                &mut s,
                "feature.chamfer",
                json!({"edges": [[WALL, TOP]], "distance": b * k, "second": a * k,
                       "name": "Bevel"}),
            );
            let volume = v0 + sign * 2.0 * 60.0 * section(a, b, union);
            assert_solid(&mut s, "Bevel", volume, None, k);
        }
        for &b in past {
            assert!(lens(a, b, union) > DEEP, "{b}");
            run(
                &mut s,
                "feature.edit",
                json!({"feature": "Bevel", "distance": b * k, "second": a * k}),
            );
            let reason = failure(&mut s, "Bevel");
            assert!(
                reason.starts_with(&format!(
                    "size.distance: too large for edges.0 (between {WALL} and {TOP})"
                )),
                "{b} at {k}: {reason}"
            );
            let largest = said_largest(&reason);
            let expected = touching(a, b, union) * b * k;
            assert!(
                (largest - expected).abs() <= 1e-4 * expected,
                "{b} at {k}: {largest} for {expected}"
            );
            // The size as a whole, a hundredth under that.
            let scale = 0.99 * largest / (b * k);
            built(
                &mut s,
                "feature.edit",
                json!({"feature": "Bevel", "distance": scale * b * k,
                       "second": scale * a * k}),
            );
            let volume = v0 + sign * 2.0 * 60.0 * section(scale * a, scale * b, union);
            assert_solid(&mut s, "Bevel", volume, None, k);
        }
    }
}

#[test]
fn a_chamfer_along_a_pipe_sunk_in_a_plate_is_too_large_where_its_line_enters_the_pipe() {
    // Up to 4.4657 of arc along the pipe (with 1 along the top) the line
    // stays clear of it, and up to 4.4702 it enters it by a lens within the
    // tolerance (4.469 builds, as 0.1.0 built it). Past that
    // what was built took material from the pipe: 8 gave 40123.44 and 10
    // gave 39784.01 against the union's 40165.78 (0.1.0).
    the_pair_along_the_pipe(true, &[3.0, 4.4, 4.469], &[4.6, 8.0, 10.0]);
}

#[test]
fn a_chamfer_along_a_groove_is_too_large_where_its_line_leaves_the_material() {
    // The groove's rims are convex: up to 3.35575 of arc down the groove
    // the line runs through the material, and up to 3.36023 it leaves it
    // by a lens within the tolerance. Past that what was built
    // added material to the groove: 8 gave 21576.95 against the grooved
    // plate's 21316.23 (0.1.0).
    the_pair_along_the_pipe(false, &[2.0, 3.3, 3.359], &[3.4, 8.0]);
}

/// What the chamfer `d` of the rim of the hole that touches the bottom
/// takes from the block: the chamfer's ring where it lies in the block,
/// at `ρ` from the axis `d - (ρ - 5)` deep over the angle `2π - 2 acos(5 /
/// ρ)` of its circle above the bottom; `ρ = 5 + t²`, Simpson's rule.
fn rim_ring(d: f64) -> f64 {
    let n = 4000;
    let h = d.sqrt() / f64::from(n);
    let mut sum = 0.0;
    for i in 0..=n {
        let t = h * f64::from(i);
        let rho = 5.0 + t * t;
        let angle = 2.0 * PI - 2.0 * (5.0 / rho).min(1.0).pacos();
        let f = (d - (rho - 5.0)) * rho * angle * 2.0 * t;
        let w = if i == 0 || i == n {
            1.0
        } else if i % 2 == 1 {
            4.0
        } else {
            2.0
        };
        sum += w * f;
    }
    sum * h / 3.0
}

#[test]
fn a_chamfer_on_the_rim_of_a_hole_that_touches_the_bottom_is_the_ring_in_the_block() {
    // A block 20 x 20 x 20 less a hole of radius 5 along y whose axis is
    // 5 above the bottom: the hole touches the bottom along a line, and
    // the rim on the front is two arcs that go on into each other there,
    // where the front face has no width under the rim. The chamfer of the
    // pair of faces is the block less the chamfer's ring where it lies in
    // the block (6289.508178 at 3), its face pinched to a point at the
    // bottom: the material is the chamfer's.
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"size": [20.0 * k, 20.0 * k, 20.0 * k], "position": at(k, [0.0, 0.0, 0.0])}),
        );
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": 5.0 * k, "height": 20.0 * k, "position": at(k, [10.0, 20.0, 5.0]),
                   "axis": [0, -1, 0]}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "F1", "tool": "F2"}),
        );
        assert_solid(&mut s, "Boolean1", 8000.0 - 500.0 * PI, None, k);
        for d in [3.0, 0.5] {
            built(
                &mut s,
                "feature.chamfer",
                json!({"edges": [["F3:copy(0)/F1:face(2)", "F3:copy(1)/F2:face(0)"]],
                       "distance": d * k, "name": format!("Rim{d}")}),
            );
            let volume = 8000.0 - 500.0 * PI - rim_ring(d);
            assert_solid(&mut s, &format!("Rim{d}"), volume, None, k);
            run(
                &mut s,
                "feature.delete",
                json!({"feature": format!("Rim{d}")}),
            );
        }
    }
}
