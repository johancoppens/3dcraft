//! Plane cuts the random designs found refused as *"boolean produced an
//! invalid body"* (2026-11-20, fuzz seeds 118385 and 115098;
//! docs/design/booleans.md › *A plane cut the boolean's own check refused
//! as an invalid body*): the boolean had built something wrong inside and
//! its own check refused it, so nothing invalid was handed on, but a cut
//! that should build was refused as an internal failure. Here each seed's
//! own commands, and each reduced to the least that showed it, through the
//! commands at three scales and held to closed forms; and, after the
//! review of 2026-11-21, the second seed's cut where its touch is exact
//! only to rounding (decimal coordinates, the body turned out of the
//! axes, two holes that touch one another). The kernel's side is
//! `crates/kernel/tests/halfspace.rs` and the unit tests of
//! `boolean/outline.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{assert_solid, body, built, deep, failed, new_document, run, session};
use craft3d_engine::Session;
use craft3d_math::{Portable, Vec3};
use serde_json::{Value, json};
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

/// More scales than [`SCALES`] for the cuts below: the review of
/// 2026-11-21 found one of them refused at a thousandth only.
const WIDER: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

/// A plate of 1 × 10 × 10 less round holes of these radii, right through
/// it: volume and area, for scale 1.
fn plate_less(radii: &[f64]) -> (f64, f64) {
    let discs: f64 = radii.iter().map(|r| PI * r * r).sum();
    let walls: f64 = radii.iter().map(|r| 2.0 * PI * r).sum();
    (100.0 - discs, 2.0 * (100.0 - discs) + 40.0 + walls)
}

/// Both sides of a plane across the body `target`, each kept as a body of
/// its own: what is wrong, if anything (a side refused, a body that is
/// not valid or not to the closed form given for scale 1, a deep check
/// that fails).
fn sides_wrong(
    s: &mut Session,
    target: &str,
    point: Vec3,
    normal: Vec3,
    (volume, area): (f64, f64),
    k: f64,
) -> Vec<String> {
    let close = |value: &Value, expected: f64| {
        value
            .as_f64()
            .is_some_and(|found| (found - expected).abs() <= 1e-9 * expected)
    };
    let mut wrong = Vec::new();
    for (name, side) in [("Below", "below"), ("Above", "above")] {
        let result = run(
            s,
            CUT,
            json!({"target": target, "keep": true, "name": name, "planes": [
                {"point": [point.x, point.y, point.z], "normal": [normal.x, normal.y, normal.z],
                 "side": side},
            ]}),
        );
        if result["status"] != "ok" {
            let reason: String = result["error"]
                .as_str()
                .unwrap_or("no reason")
                .chars()
                .take(140)
                .collect();
            wrong.push(format!("{side}: {reason}"));
            continue;
        }
        let found = body(s, name);
        if found["valid"] != true
            || !close(&found["volume"], volume * k * k * k)
            || !close(&found["area"], area * k * k)
        {
            wrong.push(format!(
                "{side}: valid {}, volume {}, area {}",
                found["valid"], found["volume"], found["area"]
            ));
        }
    }
    if wrong.is_empty() {
        let check = run(s, "document.check", json!({"deep": true}));
        if check["ok"] != true || check["problems"] != json!([]) {
            wrong.push(format!("the deep check: {check}"));
        }
    }
    wrong
}

/// The reduction of seed 115098 with its block at `y0` along y: a block
/// of 2 × 10 × 10 less a hole of radius `r` along x whose wall is tangent
/// to the block's side at `y0`, the hole's centre given as the sum a
/// designer writes, `y0 + r`. The circles about the hole's axis start
/// away from that side, so their middle is where they touch it.
fn tangent_hole(s: &mut Session, k: f64, y0: f64, r: f64) {
    built(
        s,
        "feature.block",
        json!({"position": [0.0, y0 * k, 0.0], "size": [2.0 * k, 10.0 * k, 10.0 * k],
               "name": "Block"}),
    );
    built(
        s,
        "feature.cylinder",
        json!({"axis": [1.0, 0.0, 0.0], "position": [-2.0 * k, (y0 + r) * k, 5.0 * k],
               "radius": r * k, "height": 6.0 * k, "name": "Drill"}),
    );
    built(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Drill", "name": "Holed"}),
    );
}

/// The hole tangent to a side where the touch is exact only to rounding:
/// at coordinates that are no binary fractions, the circle the plane cuts
/// from the hole passes the outline by a unit in the last place, or stops
/// as far short of it. After the repair of seed 115098 the cut was still
/// refused as *"boolean produced an invalid body"* where it passes it
/// (the review of 2026-11-21, through these commands: the block at 1.1
/// with a hole of radius 1.7, one of 24 placements at scale 1 and 8 of 96
/// sides over four scales), the touch being told at steps a fifth of a
/// tolerance long to a billionth of their length. Both sides of every
/// cut, to the closed form, at four scales.
#[test]
fn a_hole_tangent_to_a_side_at_decimal_places_is_cut_across() {
    let mut wrong = Vec::new();
    let mut cuts = 0;
    for k in WIDER {
        for (y0, r) in [
            (1.1, 1.7),
            (1.1, 2.5),
            (0.1, 2.5),
            (0.3, 2.5),
            (0.7, 0.3),
            (2.3, 1.7),
            (17.9, 2.5),
            (123.456, 0.3),
        ] {
            let mut s = session();
            new_document(&mut s, k);
            tangent_hole(&mut s, k, y0, r);
            cuts += 1;
            let across = Vec3::new(k, 0.0, 0.0);
            for fault in sides_wrong(&mut s, "Holed", across, -Vec3::X, plate_less(&[r]), k) {
                wrong.push(format!(
                    "scale {k}, the block at y {y0}, radius {r}, {fault}"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} sides of {cuts} cuts:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// `v` turned by `degrees` about the direction `axis`, as
/// `feature.transform` turns a body (Rodrigues' formula).
fn turned(v: Vec3, axis: Vec3, degrees: f64) -> Vec3 {
    let axis = axis.normalized().unwrap();
    let (sin, cos) = degrees.to_radians().psin_cos();
    v * cos + axis.cross(v) * sin + axis * (axis.dot(v) * (1.0 - cos))
}

/// The reduction of seed 115098 turned out of the axes by
/// `feature.transform`, the plane with it: the touch of the circle and
/// the outline is then exact only to rounding, and after the repair of
/// the seed three of twelve turns were still refused as *"boolean
/// produced an invalid body"* (the review of 2026-11-21, at scale 1: the
/// first three below). Both sides of every cut, to the closed form, at
/// four scales.
#[test]
fn a_hole_tangent_to_a_side_turned_out_of_the_axes_is_cut_across() {
    let mut wrong = Vec::new();
    let mut cuts = 0;
    for k in WIDER {
        for (axis, degrees, pivot) in [
            ([0.3, -0.4, 1.0], 33.0, [1.0, -2.0, 0.5]),
            ([2.0, -1.0, 0.5], 143.0, [-3.0, 0.7, 11.0]),
            ([1.0, 0.0, 1.0], 60.0, [0.1, 0.2, 0.3]),
            ([0.0, 0.0, 1.0], 30.0, [0.0, 0.0, 0.0]),
            ([1.0, 2.0, 3.0], 71.0, [5.0, 5.0, 5.0]),
            ([0.0, 1.0, 0.0], 45.0, [0.0, 0.0, 0.0]),
        ] {
            let mut s = session();
            new_document(&mut s, k);
            tangent_hole(&mut s, k, 0.0, 2.5);
            let pivot = Vec3::new(pivot[0], pivot[1], pivot[2]) * k;
            built(
                &mut s,
                "feature.transform",
                json!({"source": "Holed", "angle": format!("{degrees} deg"), "axis": axis,
                       "pivot": [pivot.x, pivot.y, pivot.z], "name": "Turned"}),
            );
            cuts += 1;
            let axis = Vec3::new(axis[0], axis[1], axis[2]);
            let across = pivot + turned(Vec3::new(k, 0.0, 0.0) - pivot, axis, degrees);
            let normal = turned(-Vec3::X, axis, degrees);
            for fault in sides_wrong(&mut s, "Turned", across, normal, plate_less(&[2.5]), k) {
                wrong.push(format!(
                    "scale {k}, turned {degrees} degrees about {axis:?}, {fault}"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} sides of {cuts} cuts:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Two holes side by side through a block, their walls touching along a
/// line, cut across: the two circles the plane cuts touch one another in
/// a point that is a vertex of neither, each circle's middle there (the
/// first hole's circles start away from the second, the second's away
/// from the first). At scales where the radii and centres are no binary
/// fractions the touch is exact only to rounding: after the repair of
/// seed 115098 the cut was built at scales 1, 1e3 and 1e6 and still
/// refused as *"boolean produced an invalid body"* at 1e-3 (the review
/// of 2026-11-21). Both sides, to the closed form, at four scales.
#[test]
fn two_holes_that_touch_are_cut_across() {
    let mut wrong = Vec::new();
    for k in WIDER {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"size": [2.0 * k, 10.0 * k, 10.0 * k], "name": "Block"}),
        );
        let holes = [
            ("One", "Block", "First", 3.0, 1.5, "180 deg"),
            ("Two", "First", "Holed", 3.0 + 1.5 + 1.0, 1.0, "0 deg"),
        ];
        for (drill, from, name, y, r, angle) in holes {
            built(
                &mut s,
                "feature.cylinder",
                json!({"angle": angle, "axis": [1.0, 0.0, 0.0], "height": 6.0 * k, "name": drill,
                       "position": [-2.0 * k, y * k, 5.0 * k], "radius": r * k}),
            );
            built(
                &mut s,
                "feature.boolean",
                json!({"op": "subtract", "target": from, "tool": drill, "name": name}),
            );
        }
        let across = Vec3::new(k, 0.0, 0.0);
        let plate = plate_less(&[1.5, 1.0]);
        for fault in sides_wrong(&mut s, "Holed", across, -Vec3::X, plate, k) {
            wrong.push(format!("scale {k}, {fault}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} sides of 4 cuts:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
