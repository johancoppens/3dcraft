//! The face operations through the commands (M9): offset face, delete
//! face and shell. Every body is measured against a closed form worked
//! out by hand, at three scales, and checked deep; the references follow
//! upstream edits and fail by name; the faces of a result carry names
//! later features build on; a saved document reopens to the same bodies;
//! and what is not built, or does not fit, is refused with the face or
//! the edge and the reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{
    assert_close, assert_solid, body, built, failed, feature, inspect, names_of, new_document,
    refused, run, session,
};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::json;
use std::f64::consts::PI;
use std::path::PathBuf;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn workdir(name: &str) -> PathBuf {
    common::workdir("face-ops", name)
}

fn body_names(session: &mut Session) -> Vec<String> {
    inspect(session)["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect()
}

fn block_area(x: f64, y: f64, z: f64) -> f64 {
    2.0 * (x * y + y * z + x * z)
}

// Faces of a block: 0 bottom, 1 top, 2 front (-y), 3 back (+y), 4 left
// (-x), 5 right (+x). Of a cylinder: 0 the wall, 1 the bottom, 2 the top.
const TOP: &str = "F1:face(1)";

/// `Base`, a block of 10 x 10 x 5 (F1).
fn base(session: &mut Session, k: f64) {
    built(
        session,
        "feature.block",
        json!({"name": "Base", "size": [10.0 * k, 10.0 * k, 5.0 * k]}),
    );
}

/// `Base` with a pocket of 4 x 4 x 3 in its top: `Pocketed` (F3). The
/// floor of the pocket is the bottom of the cutter.
fn pocketed(session: &mut Session, k: f64) {
    base(session, k);
    built(
        session,
        "feature.block",
        json!({"name": "Cutter", "position": [3.0 * k, 3.0 * k, 2.0 * k],
               "size": [4.0 * k, 4.0 * k, 4.0 * k]}),
    );
    built(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Base", "tool": "Cutter", "name": "Pocketed"}),
    );
}

const FLOOR: &str = "F3:copy(1)/F2:face(0)";

/// `Base` with a hole of radius 2 through it: `Drilled` (F3).
fn drilled(session: &mut Session, k: f64) {
    base(session, k);
    built(
        session,
        "feature.cylinder",
        json!({"name": "Drill", "position": [5.0 * k, 5.0 * k, -k],
               "radius": 2.0 * k, "height": 7.0 * k}),
    );
    built(
        session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Base", "tool": "Drill", "name": "Drilled"}),
    );
}

/// `Base` with a boss of radius 2 and height 3 on it: `Bossed` (F3).
fn bossed(session: &mut Session, k: f64) {
    base(session, k);
    built(
        session,
        "feature.cylinder",
        json!({"name": "Stud", "position": [5.0 * k, 5.0 * k, 5.0 * k],
               "radius": 2.0 * k, "height": 3.0 * k}),
    );
    built(
        session,
        "feature.boolean",
        json!({"op": "union", "target": "Base", "tool": "Stud", "name": "Bossed"}),
    );
}

const WALL: &str = "F3:copy(1)/F2:face(0)";
const TOP_OF_3: &str = "F3:copy(0)/F1:face(1)";

fn drilled_mass(radius: f64, height: f64) -> (f64, f64) {
    let section = 100.0 - PI * radius * radius;
    (
        section * height,
        2.0 * section + 40.0 * height + 2.0 * PI * radius * height,
    )
}

// ------------------------------------------------------------- offset face

#[test]
fn the_top_of_a_block_moves_and_the_block_keeps_its_six_faces() {
    for k in SCALES {
        for d in [0.5, -1.5] {
            let mut s = session();
            new_document(&mut s, k);
            base(&mut s, k);
            let made = built(
                &mut s,
                "feature.offsetFace",
                json!({"faces": [TOP], "distance": d * k, "name": "Moved"}),
            );
            assert_eq!(made["id"], "F2");
            let h = 5.0 + d;
            let found = assert_solid(
                &mut s,
                "Moved",
                100.0 * h,
                Some(block_area(10.0, 10.0, h)),
                k,
            );
            assert_eq!(found["faces"], 6);
            // Every face is the face it was.
            let expected: Vec<String> =
                (0..6).map(|i| format!("F2:copy(0)/F1:face({i})")).collect();
            assert_eq!(names_of(&found), expected);
            // The block went into the offset.
            assert_eq!(body_names(&mut s), ["Moved"]);
        }
    }
}

#[test]
fn the_floor_of_a_pocket_and_a_face_with_a_hole_through_it() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        pocketed(&mut s, k);
        built(
            &mut s,
            "feature.offsetFace",
            json!({"faces": [FLOOR], "distance": k, "name": "Shallow"}),
        );
        assert_solid(
            &mut s,
            "Shallow",
            500.0 - 16.0 * 2.0,
            Some(block_area(10.0, 10.0, 5.0) + 16.0 * 2.0),
            k,
        );
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Shallow", "distance": -1.5 * k}),
        );
        assert_solid(&mut s, "Shallow", 500.0 - 16.0 * 4.5, None, k);

        let mut s = session();
        new_document(&mut s, k);
        drilled(&mut s, k);
        built(
            &mut s,
            "feature.offsetFace",
            json!({"faces": [TOP_OF_3], "distance": 2.0 * k, "name": "Thick"}),
        );
        let (volume, area) = drilled_mass(2.0, 7.0);
        let found = assert_solid(&mut s, "Thick", volume, Some(area), k);
        assert_eq!(found["faces"], 7);
    }
}

#[test]
fn the_wall_of_a_hole_of_a_boss_and_a_ball_in_a_block() {
    for k in SCALES {
        // A hole: outward is into the hole. -1 widens it by 1.
        for d in [-1.0, 0.5] {
            let mut s = session();
            new_document(&mut s, k);
            drilled(&mut s, k);
            built(
                &mut s,
                "feature.offsetFace",
                json!({"faces": [WALL], "distance": d * k, "name": "Bored"}),
            );
            let (volume, area) = drilled_mass(2.0 - d, 5.0);
            assert_solid(&mut s, "Bored", volume, Some(area), k);
        }
        // A boss: outward is away from its axis.
        for d in [1.0, -0.5] {
            let mut s = session();
            new_document(&mut s, k);
            bossed(&mut s, k);
            built(
                &mut s,
                "feature.offsetFace",
                json!({"faces": [WALL], "distance": d * k, "name": "Stout"}),
            );
            let r = 2.0 + d;
            assert_solid(
                &mut s,
                "Stout",
                500.0 + PI * r * r * 3.0,
                Some(block_area(10.0, 10.0, 5.0) + 2.0 * PI * r * 3.0),
                k,
            );
        }
        // A ball half sunk in the top, and its radius.
        let mut s = session();
        new_document(&mut s, k);
        base(&mut s, k);
        built(
            &mut s,
            "feature.sphere",
            json!({"name": "Ball", "position": [5.0 * k, 5.0 * k, 5.0 * k], "radius": 2.0 * k}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Base", "tool": "Ball", "name": "Domed"}),
        );
        built(
            &mut s,
            "feature.offsetFace",
            json!({"faces": ["F3:copy(1)/F2:face(0)"], "distance": k, "name": "Bigger"}),
        );
        assert_solid(
            &mut s,
            "Bigger",
            500.0 + 2.0 / 3.0 * PI * 27.0,
            Some(block_area(10.0, 10.0, 5.0) + PI * 9.0),
            k,
        );
    }
}

/// A frustum of a pyramid on a regular hexagon: circumradius 3 at the
/// bottom and 2 at the top, 2 high: `Frustum` (F3).
fn hexagonal_frustum(session: &mut Session, k: f64) {
    let hexagon = |r: f64| -> Vec<[f64; 2]> {
        (0..6)
            .map(|i| {
                let a = f64::from(i) * PI / 3.0;
                [r * k * a.pcos(), r * k * a.psin()]
            })
            .collect()
    };
    for (name, z, r) in [("Low", 0.0, 3.0), ("High", 2.0, 2.0)] {
        run(
            session,
            "sketch.create",
            json!({"plane": "xy", "offset": z * k, "name": name}),
        );
        run(
            session,
            "sketch.polyline",
            json!({"sketch": name, "closed": true, "points": hexagon(r)}),
        );
    }
    built(
        session,
        "feature.loft",
        json!({"sections": ["Low", "High"], "ruled": true, "name": "Frustum"}),
    );
}

/// Volume and area of a frustum on regular hexagons of circumradius `a`
/// and `b`, `h` apart.
fn frustum_mass(a: f64, b: f64, h: f64) -> (f64, f64) {
    let hexagon = |r: f64| 1.5 * 3.0_f64.sqrt() * r * r;
    let volume = h / 3.0 * (hexagon(a) + hexagon(b) + (hexagon(a) * hexagon(b)).sqrt());
    // Six trapezia between sides a and b (a regular hexagon's side is its
    // circumradius), whose height is the slant between the two sides:
    // the apothems differ by (a - b) sqrt(3) / 2.
    let slant = (h * h + 0.75 * (a - b) * (a - b)).sqrt();
    (
        volume,
        hexagon(a) + hexagon(b) + 6.0 * 0.5 * (a + b) * slant,
    )
}

#[test]
fn the_top_of_a_hexagonal_frustum_moves_and_its_sides_are_continued() {
    for k in SCALES {
        for d in [1.0, -1.0, 3.5] {
            let mut s = session();
            new_document(&mut s, k);
            hexagonal_frustum(&mut s, k);
            let top = names_of(&body(&mut s, "Frustum"))
                .into_iter()
                .find(|n| n.contains("endCap"))
                .unwrap();
            built(
                &mut s,
                "feature.offsetFace",
                json!({"faces": [top], "distance": d * k, "name": "Continued"}),
            );
            // The circumradius shrinks by half the height.
            let (volume, area) = frustum_mass(3.0, 2.0 - d / 2.0, 2.0 + d);
            let found = assert_solid(&mut s, "Continued", volume, Some(area), k);
            assert_eq!(found["faces"], 8);
        }
        // The sides meet in a point 4 above the top: there is no top
        // there, and the error says which edge goes and what fits.
        let mut s = session();
        new_document(&mut s, k);
        hexagonal_frustum(&mut s, k);
        let top = names_of(&body(&mut s, "Frustum"))
            .into_iter()
            .find(|n| n.contains("endCap"))
            .unwrap();
        let reason = failed(
            &mut s,
            "feature.offsetFace",
            json!({"faces": [top], "distance": 5.0 * k, "name": "Gone"}),
        );
        assert!(reason.starts_with("distance: "), "{reason}");
        assert!(reason.contains("is too far"), "{reason}");
        assert!(reason.contains("the edge between F3:"), "{reason}");
        let fits: f64 = reason
            .rsplit("what fits is below ")
            .next()
            .unwrap()
            .trim_end_matches(" mm")
            .parse()
            .unwrap_or_else(|_| panic!("{reason}"));
        assert!(fits < 4.0 * k && fits > 3.99 * k, "{reason}");
        assert_eq!(body_names(&mut s), ["Frustum"]);
    }
}

#[test]
fn a_distance_too_far_names_what_vanishes_and_what_fits() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    base(&mut s, k);
    // Into the material by the whole height.
    let reason = failed(
        &mut s,
        "feature.offsetFace",
        json!({"faces": [TOP], "distance": -6.0, "name": "Flat"}),
    );
    assert!(
        reason.starts_with("distance: -6 mm is too far: the edge shrinks to nothing"),
        "{reason}"
    );
    assert!(reason.contains("what fits is above -4.9999"), "{reason}");
    // A boss as wide as the block; a hole of no radius.
    let mut s = session();
    new_document(&mut s, k);
    bossed(&mut s, k);
    let reason = failed(
        &mut s,
        "feature.offsetFace",
        json!({"faces": [WALL], "distance": 4.0, "name": "Wide"}),
    );
    assert!(
        reason.contains("the outline of the face would cross itself"),
        "{reason}"
    );
    assert!(
        reason.contains("(the face F3:copy(0)/F1:face(1))"),
        "{reason}"
    );
    assert!(reason.contains("what fits is below 2.99999"), "{reason}");
    let mut s = session();
    new_document(&mut s, k);
    drilled(&mut s, k);
    let reason = failed(
        &mut s,
        "feature.offsetFace",
        json!({"faces": [WALL], "distance": 2.5, "name": "Closed"}),
    );
    assert!(reason.contains("is too far"), "{reason}");
    assert!(reason.contains("what fits is below 1.99999"), "{reason}");
    // The floor of a pocket moved through the bottom of the block.
    let mut s = session();
    new_document(&mut s, k);
    pocketed(&mut s, k);
    let reason = failed(
        &mut s,
        "feature.offsetFace",
        json!({"faces": [FLOOR], "distance": -3.0, "name": "Through"}),
    );
    assert!(
        reason.contains("would pass through another part of the body"),
        "{reason}"
    );
    assert!(reason.contains("what fits is above -1.9999"), "{reason}");
}

#[test]
fn what_is_not_built_is_refused_with_the_faces() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    base(&mut s, k);
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [[TOP, "F1:face(2)"]], "radius": 1.0, "name": "Round"}),
    );
    // The top, next to the fillet that is tangent to it.
    let reason = failed(
        &mut s,
        "feature.offsetFace",
        json!({"faces": ["F2:copy(0)/F1:face(1)"], "distance": 1.0, "name": "Up"}),
    );
    assert!(reason.starts_with("faces: "), "{reason}");
    assert!(reason.contains("F2:fillet(0)"), "{reason}");
    assert!(!reason.contains("too far"), "{reason}");
}

/// A blade: a leaf (a spline over a line) extruded 2 high.
fn blade(s: &mut Session) {
    run(s, "sketch.create", json!({"name": "Leaf"}));
    run(
        s,
        "sketch.spline",
        json!({"sketch": "Leaf", "through": [[0.0, 0.0], [2.0, 1.5], [4.0, 1.0], [5.0, 0.0]]}),
    );
    run(
        s,
        "sketch.line",
        json!({"sketch": "Leaf", "from": [5.0, 0.0], "to": [0.0, 0.0]}),
    );
    built(
        s,
        "feature.extrude",
        json!({"sketch": "Leaf", "distance": 2.0, "name": "Blade"}),
    );
}

#[test]
fn the_cap_of_a_freeform_wall_moves_out_and_in() {
    // A flat cap next to a freeform wall, its edge a spline: moved out,
    // the wall is continued past its border; moved in, the edge is found
    // again on the wall. Both were refused before the offset of freeform
    // faces was built (2026-10-16), as "a freeform face next to a face
    // that moves cannot be continued past its edge" and "an elliptic or
    // freeform edge next to a face that moves is not built".
    let k = 1.0;
    for (distance, height) in [(1.0, 3.0), (-1.0, 1.0)] {
        let mut s = session();
        new_document(&mut s, k);
        blade(&mut s);
        let before = body(&mut s, "Blade");
        let leaf = before["volume"].as_f64().unwrap() / 2.0;
        let cap = names_of(&before)
            .into_iter()
            .find(|n| n.contains("endCap"))
            .unwrap();
        built(
            &mut s,
            "feature.offsetFace",
            json!({"faces": [cap], "distance": distance, "name": "Moved"}),
        );
        let moved = body(&mut s, "Moved");
        assert_eq!(moved["faces"], before["faces"], "{moved}");
        // The wall continued and the edge found again within a quarter of
        // the tolerance: the volume within that over the area.
        let volume = moved["volume"].as_f64().unwrap();
        let slack = 0.25e-6 * moved["area"].as_f64().unwrap();
        assert!(
            (volume - leaf * height).abs() <= slack,
            "{distance}: {volume} is not {}",
            leaf * height
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
}

#[test]
fn bad_parameters_of_an_offset_are_errors() {
    let k = 1.0;
    let mut s = session();
    // No document.
    refused(
        &mut s,
        "feature.offsetFace",
        json!({"faces": [TOP], "distance": 1.0}),
    );
    new_document(&mut s, k);
    base(&mut s, k);
    built(
        &mut s,
        "feature.block",
        json!({"name": "Other", "position": [20.0, 0.0, 0.0], "size": [1.0, 1.0, 1.0]}),
    );
    for params in [
        json!({}),
        json!({"faces": [TOP]}),
        json!({"distance": 1.0}),
        json!({"faces": [], "distance": 1.0}),
        json!({"faces": TOP, "distance": 1.0}),
        json!({"faces": [7], "distance": 1.0}),
        json!({"faces": ["nonsense"], "distance": 1.0}),
        json!({"faces": [TOP, TOP], "distance": 1.0}),
        json!({"faces": [TOP, "F2:face(1)"], "distance": 1.0}),
        json!({"faces": ["F1:edge(1)"], "distance": 1.0}),
        json!({"faces": [TOP], "distance": [1.0]}),
        json!({"faces": [TOP], "distance": 1.0, "far": true}),
    ] {
        refused(&mut s, "feature.offsetFace", params);
    }
    // Accepted, and failed with the parameter.
    for (params, starts) in [
        (json!({"faces": [TOP], "distance": 0.0}), "distance: "),
        (json!({"faces": [TOP], "distance": "1 deg"}), "distance: "),
        (json!({"faces": [TOP], "distance": "nope"}), "distance: "),
        (
            json!({"faces": ["F1:face(9)"], "distance": 1.0}),
            "faces.0: `Base` no longer has a face F1:face(9)",
        ),
        (
            json!({"faces": ["F9:face(1)"], "distance": 1.0}),
            "faces.0: there is no feature F9",
        ),
    ] {
        let reason = failed(&mut s, "feature.offsetFace", params.clone());
        assert!(reason.starts_with(starts), "{params}: {reason}");
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(body_names(&mut s), ["Base", "Other"]);
}

#[test]
fn an_offset_follows_its_face_and_fails_by_name_when_the_face_goes() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        for (name, value) in [("depth", 3.0), ("lift", 1.0)] {
            run(
                &mut s,
                "variable.set",
                json!({"name": name, "expr": value * k}),
            );
        }
        base(&mut s, k);
        // The pocket, `depth` deep.
        built(
            &mut s,
            "feature.block",
            json!({"name": "Cutter", "position": [3.0 * k, 3.0 * k, format!("{} - depth", 5.0 * k)],
                   "size": [4.0 * k, 4.0 * k, format!("depth + {}", k)]}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Base", "tool": "Cutter", "name": "Pocketed"}),
        );
        built(
            &mut s,
            "feature.offsetFace",
            json!({"faces": [FLOOR], "distance": "lift", "name": "Raised"}),
        );
        // Something that does not depend on any of it.
        built(
            &mut s,
            "feature.block",
            json!({"name": "Other", "position": [20.0 * k, 0.0, 0.0], "size": [k, k, k]}),
        );
        // A sketch on the floor that moved, and a pin from it: the pin
        // stands where the floor is now.
        built(
            &mut s,
            "sketch.create",
            json!({"name": "Mark", "face": format!("F4:copy(0)/{FLOOR}")}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Mark", "center": [2.0 * k, 2.0 * k], "radius": 0.5 * k}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Mark", "distance": 0.5 * k, "reversed": true, "name": "Pin"}),
        );
        let pocket = |depth: f64, lift: f64| 500.0 - 16.0 * (depth - lift);
        assert_solid(&mut s, "Raised", pocket(3.0, 1.0), None, k);
        let floor_height = |s: &mut Session| {
            let pin = body(s, "Pin");
            let (low, high) = (
                pin["bbox"][0][2].as_f64().unwrap(),
                pin["bbox"][1][2].as_f64().unwrap(),
            );
            // The pin stands on the floor or hangs under it, by which way
            // the face the cutter left looks; either way one of its ends
            // is on the floor.
            (low / k, high / k)
        };
        let (low, high) = floor_height(&mut s);
        assert!(
            (low - 3.0).abs() < 1e-6 || (high - 3.0).abs() < 1e-6,
            "{low} {high}"
        );
        // Upstream edits that move the face, and the distance.
        run(
            &mut s,
            "variable.set",
            json!({"name": "depth", "expr": 4.0 * k}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "lift", "expr": 2.5 * k}),
        );
        assert_eq!(feature(&mut s, "Raised")["status"], "ok");
        assert_solid(&mut s, "Raised", pocket(4.0, 2.5), None, k);
        let (low, high) = floor_height(&mut s);
        assert!(
            (low - 3.5).abs() < 1e-6 || (high - 3.5).abs() < 1e-6,
            "{low} {high}"
        );
        // An edit that removes the face: the pocket goes through, and
        // there is no floor. The offset fails with the parameter, the
        // feature and the name; what is made from it names it; nothing
        // else fails.
        run(
            &mut s,
            "variable.set",
            json!({"name": "depth", "expr": 6.0 * k}),
        );
        let raised = feature(&mut s, "Raised");
        assert_eq!(raised["status"], "failed", "{raised}");
        let reason = raised["error"].as_str().unwrap();
        assert!(
            reason.starts_with(
                "faces.0: `Pocketed` no longer has a face F3:copy(1)/F2:face(0); its faces are: "
            ),
            "{reason}"
        );
        let mark = feature(&mut s, "Mark");
        assert_eq!(mark["status"], "failed", "{mark}");
        assert!(
            mark["error"].as_str().unwrap().contains("`Raised`"),
            "{mark}"
        );
        assert_eq!(mark["causedBy"], "F4", "{mark}");
        assert_eq!(feature(&mut s, "Pin")["causedBy"], "F4");
        assert_eq!(feature(&mut s, "Other")["status"], "ok");
        assert_eq!(feature(&mut s, "Pocketed")["status"], "ok");
        assert_eq!(body_names(&mut s), ["Pocketed", "Other"]);
        // Putting the floor back repairs all three.
        run(
            &mut s,
            "variable.set",
            json!({"name": "depth", "expr": 4.0 * k}),
        );
        for name in ["Raised", "Mark", "Pin"] {
            assert_eq!(feature(&mut s, name)["status"], "ok");
        }
        // Pointing the offset at another face repairs it too, with the
        // pocket through: the top of the block, and another distance.
        run(
            &mut s,
            "variable.set",
            json!({"name": "depth", "expr": 6.0 * k}),
        );
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Raised", "faces": [TOP_OF_3], "distance": 2.0 * k}),
        );
        let raised = body(&mut s, "Raised");
        assert_eq!(raised["valid"], true);
        assert_close(
            &raised["volume"],
            (100.0 - 16.0) * 7.0 * k * k * k,
            "volume",
        );
        // The sketch was on the floor, which is still gone.
        assert_eq!(feature(&mut s, "Mark")["status"], "failed");
        // The repaired offset is the one built that way from the start.
        let repaired = body(&mut s, "Raised");
        let mut fresh = session();
        new_document(&mut fresh, k);
        base(&mut fresh, k);
        built(
            &mut fresh,
            "feature.block",
            json!({"name": "Cutter", "position": [3.0 * k, 3.0 * k, -k],
                   "size": [4.0 * k, 4.0 * k, 7.0 * k]}),
        );
        built(
            &mut fresh,
            "feature.boolean",
            json!({"op": "subtract", "target": "Base", "tool": "Cutter", "name": "Pocketed"}),
        );
        built(
            &mut fresh,
            "feature.offsetFace",
            json!({"faces": [TOP_OF_3], "distance": 2.0 * k, "name": "Raised"}),
        );
        let again = body(&mut fresh, "Raised");
        assert_eq!(again["faceNames"], repaired["faceNames"]);
        assert_close(
            &again["volume"],
            repaired["volume"].as_f64().unwrap(),
            "volume",
        );
    }
}

#[test]
fn a_chamfer_on_an_edge_of_a_moved_face_and_names_of_edges_and_vertices() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    base(&mut s, k);
    built(
        &mut s,
        "feature.offsetFace",
        json!({"faces": [TOP], "distance": 1.0, "name": "Moved"}),
    );
    // Edges and vertices are the copies of what they were.
    let listed = run(&mut s, "document.edges", json!({"feature": "Moved"}));
    let edges = listed["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 12);
    for edge in edges {
        let name = edge["name"].as_str().unwrap();
        assert!(name.starts_with("F2:copy(0)/F1:edge("), "{name}");
    }
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F2:copy(0)/F1:face(1)", "F2:copy(0)/F1:face(2)"]],
               "distance": 1.0, "name": "Bevel"}),
    );
    assert_solid(&mut s, "Bevel", 600.0 - 10.0 * 0.5, None, k);
    // A datum on the moved face lies where it is now.
    built(
        &mut s,
        "datum.plane",
        json!({"from": "F3:copy(0)/F2:copy(0)/F1:face(1)", "distance": 0.0, "name": "Lid"}),
    );
    let lid = feature(&mut s, "Lid");
    assert_eq!(lid["status"], "ok", "{lid}");
}

#[test]
fn offsets_are_saved_reopened_undone_and_redone() {
    let dir = workdir("offset-files");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    let k = 1.0;
    new_document(&mut s, k);
    bossed(&mut s, k);
    built(
        &mut s,
        "feature.offsetFace",
        json!({"faces": [WALL, TOP_OF_3], "distance": "3 mm / 4", "name": "Grown"}),
    );
    // The boss is 0.75 thicker and stands on a block 0.75 taller, with
    // its lid where it was.
    assert_solid(
        &mut s,
        "Grown",
        100.0 * 5.75 + PI * 2.75 * 2.75 * 2.25,
        None,
        k,
    );
    let before = inspect(&mut s);
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "file.save", json!({"path": "offset.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "offset.3dcraft"}));
    let after = inspect(&mut s);
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

// ------------------------------------------------------------------- shell

#[test]
fn an_open_box_a_closed_hollow_block_a_cup_and_a_tube() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        base(&mut s, k);
        let made = built(
            &mut s,
            "feature.shell",
            json!({"source": "Base", "thickness": k, "open": [TOP], "name": "Box"}),
        );
        assert_eq!(made["id"], "F2");
        // The hollow is 8 x 8 x 4; the rim is what is left of the top.
        let found = assert_solid(
            &mut s,
            "Box",
            500.0 - 256.0,
            Some(block_area(10.0, 10.0, 5.0) - 64.0 + 64.0 + 32.0 * 4.0),
            k,
        );
        let names = names_of(&found);
        assert_eq!(names.len(), 11);
        for i in 0..6 {
            assert!(
                names.contains(&format!("F2:copy(0)/F1:face({i})")),
                "{names:?}"
            );
        }
        for i in [0, 2, 3, 4, 5] {
            assert!(
                names.contains(&format!("F2:copy(1)/F1:face({i})")),
                "{names:?}"
            );
        }
        assert!(!names.contains(&"F2:copy(1)/F1:face(1)".to_string()));
        // No open face: a closed cavity of 8 x 8 x 3.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Box", "open": []}),
        );
        assert_solid(
            &mut s,
            "Box",
            500.0 - 192.0,
            Some(block_area(10.0, 10.0, 5.0) + block_area(8.0, 8.0, 3.0)),
            k,
        );
        // A cup and a tube.
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.cylinder",
            json!({"name": "Rod", "radius": 3.0 * k, "height": 5.0 * k}),
        );
        built(
            &mut s,
            "feature.shell",
            json!({"source": "Rod", "thickness": 0.5 * k, "open": ["F1:face(2)"], "name": "Cup"}),
        );
        let (r, i) = (3.0, 2.5);
        assert_solid(&mut s, "Cup", PI * r * r * 5.0 - PI * i * i * 4.5, None, k);
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Cup", "open": ["F1:face(2)", "F1:face(1)"]}),
        );
        let found = assert_solid(&mut s, "Cup", PI * (r * r - i * i) * 5.0, None, k);
        assert_eq!(found["faces"], 4);
    }
}

/// An L-shaped prism, extruded 6 high from an L of arms 10 long and 4
/// wide: `Ell` (F2).
fn ell(session: &mut Session, k: f64) {
    run(session, "sketch.create", json!({"name": "Outline"}));
    let points: Vec<[f64; 2]> = [
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 4.0],
        [4.0, 4.0],
        [4.0, 10.0],
        [0.0, 10.0],
    ]
    .iter()
    .map(|[x, y]| [x * k, y * k])
    .collect();
    run(
        session,
        "sketch.polyline",
        json!({"sketch": "Outline", "closed": true, "points": points}),
    );
    built(
        session,
        "feature.extrude",
        json!({"sketch": "Outline", "distance": 6.0 * k, "name": "Ell"}),
    );
}

#[test]
fn an_l_shaped_prism_and_a_rounded_box() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        ell(&mut s, k);
        let end = names_of(&body(&mut s, "Ell"))
            .into_iter()
            .find(|n| n.contains("endCap"))
            .unwrap();
        built(
            &mut s,
            "feature.shell",
            json!({"source": "Ell", "thickness": k, "open": [end], "name": "Channel"}),
        );
        assert_solid(&mut s, "Channel", 64.0 * 6.0 - 28.0 * 5.0, None, k);

        // A block with its four upright edges rounded by 2, open at the
        // top: the inner rounds have the radius less the wall.
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"name": "Base", "size": [10.0 * k, 8.0 * k, 6.0 * k]}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F1:face(2)", "F1:face(4)"], ["F1:face(2)", "F1:face(5)"],
                             ["F1:face(3)", "F1:face(4)"], ["F1:face(3)", "F1:face(5)"]],
                   "radius": 2.0 * k, "name": "Round"}),
        );
        let rounded = |w: f64, h: f64, r: f64| w * h - (4.0 - PI) * r * r;
        for t in [0.5, 1.5] {
            let mut t_s = session();
            new_document(&mut t_s, k);
            let _ = &mut t_s;
            let reason = run(
                &mut s,
                "feature.shell",
                json!({"source": "Round", "thickness": t * k,
                       "open": ["F2:copy(0)/F1:face(1)"], "name": format!("Tray{t}")}),
            );
            assert_eq!(reason["status"], "ok", "{reason}");
            assert_solid(
                &mut s,
                &format!("Tray{t}"),
                rounded(10.0, 8.0, 2.0) * 6.0
                    - rounded(10.0 - 2.0 * t, 8.0 - 2.0 * t, 2.0 - t) * (6.0 - t),
                None,
                k,
            );
            run(&mut s, "edit.undo", json!({}));
        }
        // From the radius of the rounds on (2026-10-21) the hollow has
        // sharp upright edges: the rounds have a skin and nothing behind
        // it.
        built(
            &mut s,
            "feature.shell",
            json!({"source": "Round", "thickness": 2.0 * k,
                   "open": ["F2:copy(0)/F1:face(1)"], "name": "Thin"}),
        );
        let sharp =
            |t: f64| rounded(10.0, 8.0, 2.0) * 6.0 - (10.0 - 2.0 * t) * (8.0 - 2.0 * t) * (6.0 - t);
        assert_solid(&mut s, "Thin", sharp(2.0), None, k);
        let names = names_of(&body(&mut s, "Thin"));
        assert!(
            names.contains(&"F3:copy(0)/F2:fillet(0)".to_string()),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|n| n.starts_with("F3:copy(1)/F2:fillet")),
            "{names:?}"
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
        // The sharp inner edge where the front and the left wall of the
        // hollow meet, rounded, follows a thicker wall.
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F3:copy(1)/F1:face(2)", "F3:copy(1)/F1:face(4)"]],
                   "radius": 0.5 * k, "name": "Inside"}),
        );
        let inside = |t: f64| sharp(t) + (4.0 - PI) / 4.0 * 0.25 * (6.0 - t);
        assert_solid(&mut s, "Inside", inside(2.0), None, k);
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Thin", "thickness": 2.5 * k}),
        );
        assert_solid(&mut s, "Inside", inside(2.5), None, k);
        // Thinner than the rounds again, the inner edge is a round of its
        // own, and the fillet on it has nothing to round: it fails, it
        // does not move to another edge.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Thin", "thickness": 1.0 * k}),
        );
        assert_eq!(feature(&mut s, "Inside")["status"], "failed");
    }
}

/// The classic enclosure (an R1 blocker, 2026-10-21): a box whose upright
/// and bottom edges are rounded 2 mm, the balls in its four bottom
/// corners, shelled 2 mm (and 3 mm) open at the top. The hollow is the
/// box shrunk by the wall with sharp edges: what the rounds hold, less
/// `(40 − 2t)(30 − 2t)(20 − t)`; checked deep, at three scales.
#[test]
fn the_classic_enclosure_rounded_then_shelled_as_thick_as_its_rounds() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"name": "Box", "size": [40.0 * k, 30.0 * k, 20.0 * k]}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F1:face(2)", "F1:face(4)"], ["F1:face(2)", "F1:face(5)"],
                             ["F1:face(3)", "F1:face(4)"], ["F1:face(3)", "F1:face(5)"],
                             ["F1:face(0)", "F1:face(2)"], ["F1:face(0)", "F1:face(3)"],
                             ["F1:face(0)", "F1:face(4)"], ["F1:face(0)", "F1:face(5)"]],
                   "radius": 2.0 * k, "name": "Round"}),
        );
        let round = body(&mut s, "Round")["volume"].as_f64().unwrap() / (k * k * k);
        built(
            &mut s,
            "feature.shell",
            json!({"source": "Round", "thickness": 2.0 * k, "open": ["F1:face(1)"],
                   "name": "Enclosure"}),
        );
        for t in [2.0, 3.0] {
            if t != 2.0 {
                built(
                    &mut s,
                    "feature.edit",
                    json!({"feature": "Enclosure", "thickness": t * k}),
                );
            }
            let hollow = (40.0 - 2.0 * t) * (30.0 - 2.0 * t) * (20.0 - t);
            assert_solid(&mut s, "Enclosure", round - hollow, None, k);
            let check = run(&mut s, "document.check", json!({"deep": true}));
            assert_eq!(check["ok"], true, "at {t}, scale {k}: {check}");
            // The hollow: a floor and four walls.
            let names = names_of(&body(&mut s, "Enclosure"));
            let inner = names
                .iter()
                .filter(|n| n.starts_with("F3:copy(1)/"))
                .count();
            assert_eq!(inner, 5, "{names:?}");
        }
    }
}

/// A box rounded all round, shelled open at its top (2026-10-22): the
/// rounds of the top's rim keep their walls, a lip `t` thick all along
/// that ends square to the top (faces named after the hollow behind the
/// top), and from the rounds' radius on the walls run straight up with a
/// ring of the top left on the lip. Volume in closed form, checked deep,
/// followed through an edit of the thickness, at three scales.
#[test]
fn a_box_rounded_all_round_shells_open_at_its_top_with_a_lip() {
    let rounded = |w: f64, d: f64, h: f64, r: f64| {
        let (a, b, c) = (w - 2.0 * r, d - 2.0 * r, h - 2.0 * r);
        a * b * c
            + 2.0 * r * (a * b + a * c + b * c)
            + PI * r * r * (a + b + c)
            + 4.0 / 3.0 * PI * r.ppowi(3)
    };
    let (w, d, h, r) = (40.0, 30.0, 20.0, 2.0);
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"name": "Box", "size": [w * k, d * k, h * k]}),
        );
        let mut edges = Vec::new();
        for (a, b) in [(2, 4), (2, 5), (3, 4), (3, 5)] {
            edges.push(json!([format!("F1:face({a})"), format!("F1:face({b})")]));
        }
        for cap in [0, 1] {
            for side in 2..6 {
                edges.push(json!([
                    format!("F1:face({cap})"),
                    format!("F1:face({side})")
                ]));
            }
        }
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": edges, "radius": r * k, "name": "Round"}),
        );
        assert_solid(&mut s, "Round", rounded(w, d, h, r), None, k);
        built(
            &mut s,
            "feature.shell",
            json!({"source": "Round", "thickness": k, "open": ["F1:face(1)"],
                   "name": "Tub"}),
        );
        for t in [1.0, 2.0, 3.0] {
            if t != 1.0 {
                built(
                    &mut s,
                    "feature.edit",
                    json!({"feature": "Tub", "thickness": t * k}),
                );
            }
            let m = r.max(t);
            let hollow = rounded(w - 2.0 * t, d - 2.0 * t, h - 2.0 * t, (r - t).max(0.0))
                + (w - 2.0 * m) * (d - 2.0 * m) * t;
            assert_solid(&mut s, "Tub", rounded(w, d, h, r) - hollow, None, k);
            let check = run(&mut s, "document.check", json!({"deep": true}));
            assert_eq!(check["ok"], true, "at {t}, scale {k}: {check}");
            let names = names_of(&body(&mut s, "Tub"));
            let ends = names
                .iter()
                .filter(|n| n.as_str() == "F3:copy(1)/F1:face(1)")
                .count();
            assert_eq!(ends, if t < r { 4 } else { 0 }, "at {t}: {names:?}");
            let top = names
                .iter()
                .filter(|n| n.as_str() == "F3:copy(0)/F1:face(1)")
                .count();
            assert_eq!(top, usize::from(t > r), "at {t}: {names:?}");
        }
    }
}

/// A block whose top-front edge is rounded, shelled open at the top, the
/// round and the front (the review of 2026-10-22: built at b133a34,
/// refused by the first form of the lip): open faces tangent to each other
/// go outward together, as before. Closed form, checked deep, three scales.
#[test]
fn a_round_left_open_with_its_faces_shells_as_before() {
    let corner = 4.0 * (1.0 - PI / 4.0);
    let expected = 10.0 * (48.0 - corner) - 8.0 * (35.0 - corner);
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"name": "Box", "size": [10.0 * k, 8.0 * k, 6.0 * k]}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F1:face(1)", "F1:face(2)"]], "radius": 2.0 * k, "name": "Round"}),
        );
        built(
            &mut s,
            "feature.shell",
            json!({"source": "Round", "thickness": k,
                   "open": ["F1:face(1)", "F2:fillet(0)", "F1:face(2)"], "name": "Shelled"}),
        );
        assert_solid(&mut s, "Shelled", expected, None, k);
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "at scale {k}: {check}");
    }
}

#[test]
fn a_shell_follows_its_body_and_its_faces_carry_names_to_build_on() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "height", "expr": 5.0 * k}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"name": "Base", "size": [10.0 * k, 10.0 * k, "height"]}),
        );
        built(
            &mut s,
            "feature.shell",
            json!({"source": "Base", "thickness": k, "open": [TOP], "name": "Box"}),
        );
        // A post standing on the floor of the hollow, sketched on it.
        built(
            &mut s,
            "sketch.create",
            json!({"name": "Spot", "face": "F2:copy(1)/F1:face(0)"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Spot", "center": [5.0 * k, -5.0 * k], "radius": k}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Spot", "distance": 2.0 * k, "join": "Box", "name": "Post"}),
        );
        // It stands on the floor, 1 above the bottom, and rises 2 into the
        // hollow: a post that went the other way would be half in the
        // floor and half outside.
        let shelled = |h: f64| 100.0 * h - 64.0 * (h - 1.0);
        assert_solid(&mut s, "Post", shelled(5.0) + PI * 2.0, None, k);
        // A chamfer on an edge of the shell: the outer edge of the rim.
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["F5:copy(0)/F2:copy(0)/F1:face(1)", "F5:copy(0)/F2:copy(0)/F1:face(2)"]],
                   "distance": 0.5 * k, "name": "Bevel"}),
        );
        assert_solid(
            &mut s,
            "Bevel",
            shelled(5.0) + PI * 2.0 - 10.0 * 0.125,
            None,
            k,
        );
        // An upstream edit: the block gets taller, and everything follows.
        run(
            &mut s,
            "variable.set",
            json!({"name": "height", "expr": 7.0 * k}),
        );
        assert_solid(
            &mut s,
            "Bevel",
            shelled(7.0) + PI * 2.0 - 10.0 * 0.125,
            None,
            k,
        );
        // A thickness that changes the floor moves the post with it.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Box", "thickness": 2.0 * k}),
        );
        let post = body(&mut s, "Bevel");
        assert_eq!(post["valid"], true);
        let thick = 100.0 * 7.0 - 36.0 * 5.0;
        assert_close(
            &post["volume"],
            (thick + PI * 2.0 - 10.0 * 0.125) * k * k * k,
            "volume",
        );
    }
}

#[test]
fn a_shell_fails_by_name_when_its_open_face_goes_and_is_repaired() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    run(
        &mut s,
        "variable.set",
        json!({"name": "depth", "expr": "3 mm"}),
    );
    base(&mut s, k);
    built(
        &mut s,
        "feature.block",
        json!({"name": "Cutter", "position": [3.0, 3.0, "5 mm - depth"],
               "size": [4.0, 4.0, "depth + 1 mm"]}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Base", "tool": "Cutter", "name": "Pocketed"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"name": "Other", "position": [20.0, 0.0, 0.0], "size": [1.0, 1.0, 1.0]}),
    );
    // Open at the floor of the pocket: the hollow opens into the pocket.
    built(
        &mut s,
        "feature.shell",
        json!({"source": "Pocketed", "thickness": 0.5, "open": [FLOOR], "name": "Hollow"}),
    );
    let hollow = body(&mut s, "Hollow");
    assert_eq!(hollow["valid"], true, "{hollow}");
    // The hollow: the block less 0.5 all round (9 x 9 x 4), less the
    // pocket with its walls moved out by 0.5 (5 x 5 from 2.5 up), less
    // the slab of 4 x 4 x 0.5 between the floor and where the floor
    // moved out to, which is the pocket's and not the block's. Behind
    // the floor the hollow is wider than the opening; its ceiling there
    // is a ring of the floor moved out.
    let inner = 81.0 * 4.0 - 25.0 * 2.0 - 16.0 * 0.5;
    let solid = 500.0 - 16.0 * 3.0;
    assert_close(&hollow["volume"], solid - inner, "volume");
    assert!(names_of(&hollow).contains(&"F5:copy(1,1)/F2:face(0)".to_string()));
    // The pocket goes through: there is no floor.
    run(
        &mut s,
        "variable.set",
        json!({"name": "depth", "expr": "6 mm"}),
    );
    let failed_shell = feature(&mut s, "Hollow");
    assert_eq!(failed_shell["status"], "failed");
    let reason = failed_shell["error"].as_str().unwrap();
    assert!(
        reason.starts_with(
            "open.0: `Pocketed` no longer has a face F3:copy(1)/F2:face(0); its faces are: "
        ),
        "{reason}"
    );
    assert_eq!(feature(&mut s, "Other")["status"], "ok");
    assert_eq!(body_names(&mut s), ["Pocketed", "Other"]);
    // Pointed at the top instead, it is a box with a square hole.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Hollow", "open": [TOP_OF_3], "thickness": 1.0}),
    );
    // Outside 10 x 10 x 5 less the hole of 4 x 4; the hollow 8 x 8 x 4
    // less the hole's walls grown by 1, 6 x 6.
    assert_solid(
        &mut s,
        "Hollow",
        (100.0 - 16.0) * 5.0 - (64.0 - 36.0) * 4.0,
        None,
        k,
    );
}

#[test]
fn bad_parameters_of_a_shell_are_errors_and_shells_are_saved() {
    let dir = workdir("shell-files");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    refused(
        &mut s,
        "feature.shell",
        json!({"source": "Base", "thickness": 1.0}),
    );
    new_document(&mut s, 1.0);
    base(&mut s, 1.0);
    for params in [
        json!({}),
        json!({"source": "Base"}),
        json!({"thickness": 1.0}),
        json!({"source": "Nothing", "thickness": 1.0}),
        json!({"source": "Base", "thickness": [1.0]}),
        json!({"source": "Base", "thickness": 1.0, "open": TOP}),
        json!({"source": "Base", "thickness": 1.0, "open": [TOP, TOP]}),
        json!({"source": "Base", "thickness": 1.0, "open": [3]}),
        json!({"source": "Base", "thickness": 1.0, "wall": 1}),
    ] {
        refused(&mut s, "feature.shell", params);
    }
    for (params, starts) in [
        (json!({"source": "Base", "thickness": 0.0}), "thickness: "),
        (json!({"source": "Base", "thickness": -1.0}), "thickness: "),
        (
            json!({"source": "Base", "thickness": "2 deg"}),
            "thickness: ",
        ),
        (
            json!({"source": "Base", "thickness": 1.0, "open": ["F1:face(7)"]}),
            "open.0: `Base` no longer has a face F1:face(7)",
        ),
        (
            json!({"source": "Base", "thickness": 1.0, "open": ["F7:face(1)"]}),
            "open.0: F7:face(1) is a face of F7",
        ),
        (
            json!({"source": "Base", "thickness": 2.5}),
            "thickness: 2.5 mm is too far",
        ),
    ] {
        let reason = failed(&mut s, "feature.shell", params.clone());
        assert!(reason.starts_with(starts), "{params}: {reason}");
        run(&mut s, "edit.undo", json!({}));
    }
    built(
        &mut s,
        "feature.shell",
        json!({"source": "Base", "thickness": "1 mm / 2", "open": [TOP, "F1:face(2)"], "name": "Scoop"}),
    );
    let before = inspect(&mut s);
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "file.save", json!({"path": "shell.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "shell.3dcraft"}));
    let after = inspect(&mut s);
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

// ------------------------------------------------------------- delete face

const FRONT: &str = "F1:face(2)";

/// The faces of `Base` as copies through the features `prefixes` (the
/// last feature first).
fn base_faces(prefixes: &[&str]) -> Vec<String> {
    (0..6)
        .map(|i| {
            let mut name = format!("F1:face({i})");
            for prefix in prefixes.iter().rev() {
                name = format!("{prefix}/{name}");
            }
            name
        })
        .collect()
}

/// Every round and every ball of a block rounded all round, deleted
/// together (2026-10-21), give the block back; a ball alone is refused
/// by its name; at three scales.
#[test]
fn the_rounds_and_balls_of_a_block_deleted_together_give_the_block_back() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"name": "Base", "size": [10.0 * k, 8.0 * k, 6.0 * k]}),
        );
        let mut edges = Vec::new();
        for (a, b) in [(2, 4), (2, 5), (3, 4), (3, 5)] {
            edges.push(json!([format!("F1:face({a})"), format!("F1:face({b})")]));
        }
        for a in [0, 1] {
            for b in 2..6 {
                edges.push(json!([format!("F1:face({a})"), format!("F1:face({b})")]));
            }
        }
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": edges, "radius": 2.0 * k, "name": "Round"}),
        );
        let blends: Vec<String> = names_of(&body(&mut s, "Round"))
            .into_iter()
            .filter(|n| n.starts_with("F2:fillet"))
            .collect();
        assert_eq!(blends.len(), 20, "{blends:?}");
        let corner = blends
            .iter()
            .find(|n| n.starts_with("F2:filletCorner"))
            .unwrap()
            .clone();
        let reason = failed(
            &mut s,
            "feature.deleteFace",
            json!({"faces": [corner], "name": "Ball"}),
        );
        assert!(reason.contains("goes only together"), "{reason}");
        run(&mut s, "edit.undo", json!({}));
        built(
            &mut s,
            "feature.deleteFace",
            json!({"faces": blends, "name": "Sharp"}),
        );
        assert_solid(&mut s, "Sharp", 480.0, Some(block_area(10.0, 8.0, 6.0)), k);
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "at scale {k}: {check}");
    }
}

#[test]
fn a_fillet_or_a_chamfer_deleted_gives_the_block_back() {
    for k in SCALES {
        for (blend, params) in [
            ("feature.fillet", json!({"radius": k})),
            ("feature.chamfer", json!({"distance": 1.5 * k})),
        ] {
            let mut s = session();
            new_document(&mut s, k);
            base(&mut s, k);
            let mut params = params;
            params["edges"] = json!([[TOP, FRONT]]);
            params["name"] = json!("Blend");
            built(&mut s, blend, params);
            let made = body(&mut s, "Blend");
            assert_eq!(made["faces"], 7);
            let face = if blend == "feature.fillet" {
                "F2:fillet(0)"
            } else {
                "F2:chamfer(0)"
            };
            let deleted = built(
                &mut s,
                "feature.deleteFace",
                json!({"faces": [face], "name": "Plain"}),
            );
            assert_eq!(deleted["id"], "F3");
            let found = assert_solid(&mut s, "Plain", 500.0, Some(block_area(10.0, 10.0, 5.0)), k);
            assert_eq!(found["faces"], 6);
            // The faces that stay are copies of what they were; the
            // original names do not answer, the copies of them do.
            assert_eq!(names_of(&found), base_faces(&["F3:copy(0)"]));
            assert_eq!(body_names(&mut s), ["Plain"]);
            // The block's twelve edges are back, named by their faces;
            // the sharp edge is there to build on.
            let listed = run(&mut s, "document.edges", json!({"feature": "Plain"}));
            assert_eq!(listed["edges"].as_array().unwrap().len(), 12, "{listed}");
            built(
                &mut s,
                "feature.chamfer",
                json!({"edges": [["F3:copy(0)/F2:copy(0)/F1:face(1)",
                                  "F3:copy(0)/F2:copy(0)/F1:face(2)"]],
                       "distance": k, "name": "Again"}),
            );
            assert_solid(&mut s, "Again", 500.0 - 10.0 * 0.5, None, k);
        }
    }
}

/// Faces of the cutter of `pocketed` that are the pocket: its floor and
/// its four walls.
const POCKET: [&str; 5] = [
    "F3:copy(1)/F2:face(0)",
    "F3:copy(1)/F2:face(2)",
    "F3:copy(1)/F2:face(3)",
    "F3:copy(1)/F2:face(4)",
    "F3:copy(1)/F2:face(5)",
];
const STUD_TOP: &str = "F3:copy(1)/F2:face(2)";

#[test]
fn a_hole_a_boss_and_a_pocket_deleted_give_the_block_back() {
    for k in SCALES {
        for (make, faces) in [
            (drilled as fn(&mut Session, f64), vec![WALL]),
            (bossed, vec![WALL, STUD_TOP]),
            (pocketed, POCKET.to_vec()),
        ] {
            let mut s = session();
            new_document(&mut s, k);
            make(&mut s, k);
            built(
                &mut s,
                "feature.deleteFace",
                json!({"faces": faces, "name": "Plain"}),
            );
            let found = assert_solid(&mut s, "Plain", 500.0, Some(block_area(10.0, 10.0, 5.0)), k);
            assert_eq!(found["faces"], 6);
            assert_eq!(names_of(&found), base_faces(&["F4:copy(0)"]));
        }
    }
}

#[test]
fn a_deletion_follows_upstream_edits_and_fails_by_name_when_its_face_goes() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": 2.0 * k}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "x", "expr": 5.0 * k}),
        );
        base(&mut s, k);
        built(
            &mut s,
            "feature.cylinder",
            json!({"name": "Drill", "position": ["x", 5.0 * k, -k],
                   "radius": "r", "height": 7.0 * k}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Base", "tool": "Drill", "name": "Drilled"}),
        );
        built(
            &mut s,
            "feature.deleteFace",
            json!({"faces": [WALL], "name": "Plain"}),
        );
        // A pin on the top that was drilled: the top is whole again.
        built(
            &mut s,
            "sketch.create",
            json!({"name": "Mark", "face": "F4:copy(0)/F3:copy(0)/F1:face(1)"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Mark", "center": [5.0 * k, 5.0 * k], "radius": 0.5 * k}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Mark", "distance": k, "name": "Pin"}),
        );
        let pin = body(&mut s, "Pin");
        assert_close(&pin["bbox"][0][2], 5.0 * k, "the pin's foot");
        assert_close(&pin["bbox"][1][2], 6.0 * k, "the pin's top");
        assert_solid(&mut s, "Plain", 500.0, None, k);
        // A wider hole and one elsewhere: the deletion follows.
        run(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": 3.0 * k}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "x", "expr": 4.0 * k}),
        );
        assert_eq!(feature(&mut s, "Plain")["status"], "ok");
        assert_solid(&mut s, "Plain", 500.0, None, k);
        // The drill misses the block: there is no wall. The deletion
        // fails with the parameter, the feature and the name; what is
        // built on it says it is the cause.
        run(
            &mut s,
            "variable.set",
            json!({"name": "x", "expr": 30.0 * k}),
        );
        let plain = feature(&mut s, "Plain");
        assert_eq!(plain["status"], "failed", "{plain}");
        let reason = plain["error"].as_str().unwrap();
        assert!(
            reason.starts_with(
                "faces.0: `Drilled` no longer has a face F3:copy(1)/F2:face(0); its faces are: "
            ),
            "{reason}"
        );
        assert_eq!(feature(&mut s, "Mark")["causedBy"], "F4");
        assert_eq!(feature(&mut s, "Pin")["causedBy"], "F4");
        assert_eq!(feature(&mut s, "Drilled")["status"], "ok");
        // Back again repairs all three.
        run(
            &mut s,
            "variable.set",
            json!({"name": "x", "expr": 5.0 * k}),
        );
        for name in ["Plain", "Mark", "Pin"] {
            assert_eq!(feature(&mut s, name)["status"], "ok", "{name}");
        }
    }
}

#[test]
fn a_deletion_is_pointed_at_other_faces_with_feature_edit() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    bossed(&mut s, k);
    // The wall alone: the boss's top cannot reach the block.
    let reason = failed(
        &mut s,
        "feature.deleteFace",
        json!({"faces": [WALL], "name": "Plain"}),
    );
    assert!(reason.starts_with("faces: "), "{reason}");
    assert!(reason.contains(WALL), "{reason}");
    // The top too: the boss is gone.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Plain", "faces": [WALL, STUD_TOP]}),
    );
    let repaired = assert_solid(&mut s, "Plain", 500.0, None, k);
    // The one built that way from the start.
    let mut fresh = session();
    new_document(&mut fresh, k);
    bossed(&mut fresh, k);
    built(
        &mut fresh,
        "feature.deleteFace",
        json!({"faces": [WALL, STUD_TOP], "name": "Plain"}),
    );
    let again = body(&mut fresh, "Plain");
    assert_eq!(again["faceNames"], repaired["faceNames"]);
    assert_eq!(again["volume"], repaired["volume"]);
    // Bad faces in an edit are refused like in the command.
    refused(
        &mut s,
        "feature.edit",
        json!({"feature": "Plain", "faces": []}),
    );
    refused(
        &mut s,
        "feature.edit",
        json!({"feature": "Plain", "faces": [WALL, WALL]}),
    );
}

#[test]
fn what_delete_face_does_not_build_is_refused_with_the_faces() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    base(&mut s, k);
    // The top of a block: its sides never meet.
    let reason = failed(&mut s, "feature.deleteFace", json!({"faces": [TOP]}));
    assert!(reason.starts_with("faces: "), "{reason}");
    assert!(reason.contains("cannot be collapsed"), "{reason}");
    run(&mut s, "edit.undo", json!({}));
    // Three fillets in a corner: the ball is refused by its name, a
    // fillet next to it because a blend goes on at its end.
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [[TOP, FRONT], [TOP, "F1:face(4)"], [FRONT, "F1:face(4)"]],
               "radius": 1.0, "name": "Round"}),
    );
    let reason = failed(
        &mut s,
        "feature.deleteFace",
        json!({"faces": ["F2:filletCorner(0,1,2)"]}),
    );
    assert!(
        reason.starts_with("faces.0: F2:filletCorner(0,1,2) is the corner"),
        "{reason}"
    );
    run(&mut s, "edit.undo", json!({}));
    let reason = failed(
        &mut s,
        "feature.deleteFace",
        json!({"faces": ["F2:fillet(0)"]}),
    );
    assert!(reason.contains("tangent"), "{reason}");
    assert!(reason.contains("F2:fillet(0)"), "{reason}");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(body_names(&mut s), ["Round"]);
}

#[test]
fn bad_parameters_of_a_deletion_are_errors_and_deletions_are_saved() {
    let dir = workdir("delete-files");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    refused(&mut s, "feature.deleteFace", json!({"faces": [TOP]}));
    let k = 1.0;
    new_document(&mut s, k);
    base(&mut s, k);
    built(
        &mut s,
        "feature.block",
        json!({"name": "Other", "position": [20.0, 0.0, 0.0], "size": [1.0, 1.0, 1.0]}),
    );
    for params in [
        json!({}),
        json!({"faces": []}),
        json!({"faces": TOP}),
        json!({"faces": [7]}),
        json!({"faces": ["nonsense"]}),
        json!({"faces": [TOP, TOP]}),
        json!({"faces": [TOP, "F2:face(1)"]}),
        json!({"faces": ["F1:edge(1)"]}),
        json!({"faces": [TOP], "distance": 1.0}),
        json!({"faces": [TOP], "name": 3}),
    ] {
        refused(&mut s, "feature.deleteFace", params);
    }
    for (params, starts) in [
        (
            json!({"faces": ["F1:face(9)"]}),
            "faces.0: `Base` no longer has a face F1:face(9)",
        ),
        (
            json!({"faces": ["F9:face(1)"]}),
            "faces.0: there is no feature F9",
        ),
    ] {
        let reason = failed(&mut s, "feature.deleteFace", params.clone());
        assert!(reason.starts_with(starts), "{params}: {reason}");
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(body_names(&mut s), ["Base", "Other"]);
    // A round deleted again, saved, reopened, undone and redone.
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [[TOP, FRONT]], "radius": "3 mm / 2", "name": "Round"}),
    );
    built(
        &mut s,
        "feature.deleteFace",
        json!({"faces": ["F3:fillet(0)"], "name": "Plain"}),
    );
    assert_solid(&mut s, "Plain", 500.0, None, k);
    let before = inspect(&mut s);
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "file.save", json!({"path": "delete.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "delete.3dcraft"}));
    let after = inspect(&mut s);
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

#[test]
fn a_deletion_added_while_rolled_back_goes_in_at_the_marker() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    drilled(&mut s, k);
    built(
        &mut s,
        "feature.block",
        json!({"name": "Other", "position": [20.0, 0.0, 0.0], "size": [1.0, 1.0, 1.0]}),
    );
    run(&mut s, "history.rollback", json!({"feature": "Drilled"}));
    built(
        &mut s,
        "feature.deleteFace",
        json!({"faces": [WALL], "name": "Plain"}),
    );
    let names: Vec<String> = inspect(&mut s)["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_string())
        .collect();
    let plain = names.iter().position(|n| n == "Plain").unwrap();
    let other = names.iter().position(|n| n == "Other").unwrap();
    assert!(plain < other, "{names:?}");
}
