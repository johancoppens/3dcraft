//! Plane cuts the random designs found refused as *"boolean produced an
//! invalid body"* (2026-11-20, fuzz seeds 118385 and 115098;
//! docs/design/booleans.md › *A plane cut the boolean's own check refused
//! as an invalid body*): the boolean had built something wrong inside and
//! its own check refused it, so nothing invalid was handed on, but a cut
//! that should build was refused as an internal failure. Here each seed's
//! own commands, and each reduced to the least that showed it, through the
//! commands at three scales and held to closed forms. The kernel's side is
//! `crates/kernel/tests/halfspace.rs` and the unit tests of
//! `boolean/outline.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{assert_solid, body, built, deep, failed, new_document, session};
use craft3d_math::Portable;
use serde_json::json;
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

const CUT: &str = "feature.planeCut";

/// What the plane x + y = c leaves of a cone of radius and height 7.5 on
/// the origin, on the side away from the axis, between the levels where
/// the cone's radius is `top` and `bottom`: volume and area in closed form
/// (`crates/kernel/tests/halfspace.rs` › `HalfCone` has the derivation).
fn half_cone(c: f64, top: f64, bottom: f64) -> (f64, f64) {
    let d = c / 2f64.sqrt();
    let s = |r: f64| (r * r - d * d).max(0.0).sqrt();
    let section = |r: f64| r * r * (d / r).min(1.0).pacos() - d * s(r);
    let sections_to = |r: f64| {
        r * r * r * (d / r).min(1.0).pacos() / 3.0 - 2.0 * d * r * s(r) / 3.0
            + d * d * d * (r + s(r)).pln() / 3.0
    };
    let chords_to = |r: f64| r * s(r) - d * d * (r + s(r)).pln();
    let (over, under) = (section(top), section(bottom));
    (
        sections_to(bottom) - sections_to(top),
        under + over + 2f64.sqrt() * (under - over) + chords_to(bottom) - chords_to(top),
    )
}

/// Seed 118385 as the fuzzer made it (scale 1e3, tolerance 1e-3), group 1:
/// a cone along x cut by four planes in one feature. The second, level
/// across the half the first leaves, was refused as an invalid body. The
/// first three each leave a face; the fourth, beside what they leave,
/// changes nothing.
#[test]
fn seed_118385_as_found() {
    let mut s = session();
    new_document(&mut s, 1e3);
    built(
        &mut s,
        "feature.cone",
        json!({"angle": "0 deg", "axis": [1.0, 0.0, 0.0], "height": 7500.0, "name": "N0",
               "position": [-10000.0, 0.0, 0.0], "radius": 7500.0}),
    );
    let result = built(
        &mut s,
        CUT,
        json!({"name": "N1", "target": "N0", "planes": [
            {"normal": [0.0, 1.0, 1.0], "point": [-6250.0, 10.003936261680792, 0.0], "side": "above"},
            {"offset": -5000.0, "plane": "F1:face(1)", "side": "above"},
            {"offset": 0.0, "plane": "xy", "side": "below"},
            {"offset": -7479.992127476638, "plane": "zx", "side": "above"},
        ]}),
    );
    let cuts: Vec<bool> = result["planes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plane| plane["cuts"].as_bool().unwrap())
        .collect();
    assert_eq!(cuts, [true, true, true, false], "{result}");
    let found = body(&mut s, "N1");
    assert_eq!(found["valid"], true, "{found}");
    assert_eq!(found["shells"], 1, "{found}");
    deep(&mut s);
}

/// Seed 115098 as the fuzzer made it (scale 1), group 5: a block less a
/// cylinder with rounded ends whose wall is tangent to the block's side,
/// cut across the hole by one plane. What is left is a plate of
/// 0.625 × 20 × 17.5 less a hole of radius 2.5.
#[test]
fn seed_115098_as_found() {
    let mut s = session();
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.cylinder",
        json!({"angle": "0 deg", "axis": [1.0, 0.0, 0.0], "height": 20.0, "name": "N0",
               "position": [-12.5, -5.0, 10.0], "radius": 2.5}),
    );
    built(
        &mut s,
        "feature.fillet",
        json!({"chain": false, "name": "N2", "radius": 0.625,
               "edges": [["F1:face(0)", "F1:face(1)"], ["F1:face(0)", "F1:face(2)"]]}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"angle": "0 deg", "axis": [0.0, 0.0, 1.0], "name": "N3",
               "position": [-12.5, -7.5, 3.75367379315737], "size": [2.5, 20.0, 17.5]}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"keepTool": true, "name": "N4", "op": "subtract", "target": "N3", "tool": "N2"}),
    );
    built(
        &mut s,
        CUT,
        json!({"name": "N5", "target": "N4", "planes": [
            {"normal": [-1.0, 0.0, 0.0], "point": [-10.625, 12.5, 12.50367379315737]},
        ]}),
    );
    let found = assert_solid(
        &mut s,
        "N5",
        0.625 * (350.0 - 6.25 * PI),
        Some(746.875 - 9.375 * PI),
        1.0,
    );
    assert_eq!(found["faces"], 7, "{found}");
}

/// Seed 118385 reduced: a cone halved by a plane that passes its axis by
/// a thousandth of its size, then cut level, both sides kept, at a third
/// and at two thirds of its height (the second was refused).
#[test]
fn half_a_cone_cut_beside_its_apex_is_cut_level() {
    let c = 10.003936261680792e-3;
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.cone",
            json!({"radius": 7.5 * k, "height": 7.5 * k, "name": "Cone"}),
        );
        built(
            &mut s,
            CUT,
            json!({"target": "Cone", "name": "Half", "planes": [
                {"point": [c * k, 0.0, 3.75 * k], "normal": [1.0, 1.0, 0.0], "side": "above"},
            ]}),
        );
        let d = c / 2f64.sqrt();
        let (volume, area) = half_cone(c, d, 7.5);
        assert_solid(&mut s, "Half", volume, Some(area), k);
        for (index, h) in [2.5, 5.0].into_iter().enumerate() {
            let level = json!({"point": [0.0, 0.0, h * k], "normal": [0.0, 0.0, 1.0]});
            let mut over = level.clone();
            over["side"] = json!("above");
            let (low, high) = (format!("Low{index}"), format!("High{index}"));
            built(
                &mut s,
                CUT,
                json!({"target": "Half", "planes": [level], "keep": true, "name": low}),
            );
            built(
                &mut s,
                CUT,
                json!({"target": "Half", "planes": [over], "keep": true, "name": high}),
            );
            let (volume, area) = half_cone(c, 7.5 - h, 7.5);
            let found = assert_solid(&mut s, &low, volume, Some(area), k);
            assert_eq!(found["faces"], 4, "scale {k}, level {h}: {found}");
            let (volume, area) = half_cone(c, d, 7.5 - h);
            let found = assert_solid(&mut s, &high, volume, Some(area), k);
            assert_eq!(found["faces"], 3, "scale {k}, level {h}: {found}");
        }
    }
}

/// Seed 115098 reduced: a block of 2 × 10 × 10 with a hole of radius 2.5
/// along x whose wall is tangent to the side y = 0, cut across the hole at
/// x = 1, both sides kept: plates of 1 × 10 × 10 less the hole. The
/// cylinder is turned about its axis: at 0 degrees the circle the plane
/// cuts from its wall starts away from the side, and its middle is where
/// it touches the outline the plane cuts from the block (the case the
/// boolean built wrong and refused); at 37 neither is.
///
/// At 180 the circle starts at the touch, so the cut body has a vertex
/// there, on an edge of the outline it is no end of, and regeneration's
/// own check refuses the body as touching itself, naming the place. The
/// same body with the vertex elsewhere is accepted as a wall that thins to
/// nothing: which of the two it is to be is the developer's open question
/// (docs/design/booleans.md › *A tangency line on which another wall
/// ends*), and what is held here is what happens today, in words, at that
/// place.
#[test]
fn a_hole_tangent_to_a_side_is_cut_across() {
    for k in SCALES {
        for angle in ["0 deg", "37 deg", "180 deg"] {
            let mut s = session();
            new_document(&mut s, k);
            built(
                &mut s,
                "feature.block",
                json!({"size": [2.0 * k, 10.0 * k, 10.0 * k], "name": "Block"}),
            );
            built(
                &mut s,
                "feature.cylinder",
                json!({"angle": angle, "axis": [1.0, 0.0, 0.0], "height": 6.0 * k, "name": "Drill",
                       "position": [-2.0 * k, 2.5 * k, 5.0 * k], "radius": 2.5 * k}),
            );
            built(
                &mut s,
                "feature.boolean",
                json!({"op": "subtract", "target": "Block", "tool": "Drill", "name": "Holed"}),
            );
            for (name, side) in [("Right", "below"), ("Left", "above")] {
                let params = json!({"target": "Holed", "keep": true, "name": name, "planes": [
                    {"point": [k, 0.0, 0.0], "normal": [-1.0, 0.0, 0.0], "side": side},
                ]});
                if angle == "180 deg" {
                    let reason = failed(&mut s, CUT, params);
                    assert!(
                        reason.contains("the cut body touches itself at (")
                            && reason.contains("move what meets there apart"),
                        "scale {k}, {name}: {reason}"
                    );
                    continue;
                }
                built(&mut s, CUT, params);
                let found =
                    assert_solid(&mut s, name, 100.0 - 6.25 * PI, Some(240.0 - 7.5 * PI), k);
                assert_eq!(found["faces"], 7, "scale {k}, angle {angle}: {found}");
                assert_eq!(found["shells"], 1, "scale {k}, angle {angle}: {found}");
            }
        }
    }
}
