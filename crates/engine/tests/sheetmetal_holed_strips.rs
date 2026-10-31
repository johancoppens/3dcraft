//! The press-brake check of a part with holes across its bends
//! (docs/design/sheetmetal.md › *Bend sequence* › *Holes across a bend*):
//! a strip not yet bent is its slab whole, holes and all, so a collision
//! found with it may lie in a hole and is never certain, while a clearance
//! proven to it is proven. The safety property pinned here: such a part
//! is never said to collide (`infeasibleProven`, the measure `false`) nor
//! to be bendable when the tool meets the holed strip; it is notProven.
//! And the refusals of `formed` and `order`.
//!
//! The part: a base 20 × 60 with flanges 10 long on its left and right
//! edges, bent 20 degrees up (R = T = 1, K = ½, so each strip is
//! `20° × 1.5 ≈ 0.52` wide), a slot across each bend. The punch carries a
//! thin bar at the height of the sheet reaching 20.3 to either side of its
//! tip: inside the base (a flat of the bend, not checked) and into the
//! other flange's strip (20 to 20.52 from the bend), never into that
//! flange's flat, which the 20-degree stroke turns away from the bar.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};

/// The bar punch: its tip at the origin, a body up `+y`, and a bar 0.3
/// thick at `y` from −1.5 to −1.2 (the sheet's top skin lies at `y = −R`
/// = −1 in the bend's frame), from −20.3 to 20.3. Its own mirror image.
fn bar() -> Value {
    json!([
        [-0.3, 30],
        [-0.3, -1.2],
        [-20.3, -1.2],
        [-20.3, -1.5],
        [20.3, -1.5],
        [20.3, -1.2],
        [0.3, -1.2],
        [0.3, 30]
    ])
}

fn plain_punch() -> Value {
    json!([[-0.3, 0], [0.3, 0], [0.3, 30], [-0.3, 30]])
}

fn die() -> Value {
    json!([[-3, 0], [3, 0], [3, -30], [-3, -30]])
}

/// The part, with or without the slots across its bends.
fn part(holes: bool) -> Session {
    let dir = std::env::temp_dir().join(format!(
        "craft3d-holed-strips-{}-{holes}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Base"}),
    );
    let made = run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0, 0], "to": [20, 60]}),
    );
    let lines = made["lines"].as_array().unwrap().clone();
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Slots"}),
    );
    // Across each bend's strip (0.52 wide, outside the base's edge).
    run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Slots", "from": [-2, 25], "to": [2, 35]}),
    );
    run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Slots", "from": [18, 25], "to": [22, 35]}),
    );
    let flanges: Vec<Value> = [&lines[3], &lines[1]]
        .iter()
        .map(|e| json!({"edge": e, "length": "10 mm", "angle": "20 deg"}))
        .collect();
    let made = run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Part", "thickness": "1 mm", "radius": "1 mm",
               "kFactor": 0.5, "flanges": flanges,
               "cutouts": if holes { json!(["Slots"]) } else { json!([]) }}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    s
}

fn sequence(s: &mut Session, punch: Value, extra: Value) -> Value {
    let mut p = json!({"feature": "Part", "punch": punch, "die": die(), "maxWork": 200000});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    run(s, "sheetMetal.bendSequence", p)
}

fn assertion(s: &mut Session, name: &str, params: Value) -> Value {
    let mut p = params;
    p["name"] = json!(name);
    p["measure"] = json!("bendable");
    run(s, "assert.add", p);
    let listed = run(s, "assert.list", json!({"details": true}));
    listed["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .cloned()
        .unwrap()
}

#[test]
fn a_tool_that_meets_a_holed_strip_is_never_a_certain_collision() {
    // Without the slots the bar lies in the other flange's strip for
    // certain: no order exists, proven.
    let mut solid = part(false);
    let got = sequence(&mut solid, bar(), json!({}));
    assert_eq!(got["feasible"], false, "{got}");
    assert_eq!(got["infeasibleProven"], true, "{got}");
    // With them the same strips are slabs that hold the slots' material
    // too: the collisions are not certain, so no order is found and none
    // is proven not to exist.
    let mut holed = part(true);
    let got = sequence(&mut holed, bar(), json!({}));
    assert_eq!(got["feasible"], false, "{got}");
    assert_eq!(got["infeasibleProven"], false, "{got}");
    let collisions = got["collisions"].as_array().unwrap();
    assert!(!collisions.is_empty(), "{got}");
    for c in collisions {
        assert_eq!(c["certain"], false, "{c}");
        assert_eq!(c["stepCertain"], false, "{c}");
    }
    // As an assertion: notProven, never false (collides) nor true.
    let a = assertion(
        &mut holed,
        "bar",
        json!({"of": "Part", "punch": bar(), "die": die(), "is": true}),
    );
    assert_eq!(a["status"], "notProven", "{a}");
    let a = assertion(
        &mut holed,
        "bar not",
        json!({"of": "Part", "punch": bar(), "die": die(), "is": false}),
    );
    assert_eq!(a["status"], "notProven", "{a}");
    // A punch clear of the strips is proven clear of the whole slabs: the
    // part with its slots is bendable.
    let got = sequence(&mut holed, plain_punch(), json!({}));
    assert_eq!(got["feasible"], true, "{got}");
    let a = assertion(
        &mut holed,
        "plain",
        json!({"of": "Part", "punch": plain_punch(), "die": die(), "is": true}),
    );
    assert_eq!(a["status"], "passed", "{a}");
}

#[test]
fn formed_and_order_are_refused_where_they_name_nothing_they_can() {
    let mut s = part(true);
    // An index past the end.
    let err = s
        .execute(
            "sheetMetal.bendSequence",
            &json!({"feature": "Part", "punch": plain_punch(), "die": die(), "formed": [2]}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("`formed`"), "{err}");
    // Not a list of indices.
    let err = s
        .execute(
            "sheetMetal.bendSequence",
            &json!({"feature": "Part", "punch": plain_punch(), "die": die(), "formed": "0"}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("`formed`"), "{err}");
    // A formed flange listed in the order.
    let err = s
        .execute(
            "sheetMetal.bendSequence",
            &json!({"feature": "Part", "punch": plain_punch(), "die": die(),
                    "formed": [0], "order": [0, 1]}),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("formed"), "{err}");
    // The order without the formed flange: checked, and it clears.
    let got = sequence(&mut s, plain_punch(), json!({"formed": [0], "order": [1]}));
    assert_eq!(got["feasible"], true, "{got}");
    assert_eq!(got["formed"], json!([0]), "{got}");
    assert!(
        got["unchecked"].as_str().unwrap().contains("formed"),
        "{got}"
    );
    // The measure: `formed` and `order` name flanges of one part.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "offset": 50, "name": "Other"}),
    );
    let made = run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Other", "from": [0, 0], "to": [20, 20]}),
    );
    let edge = made["lines"][0].clone();
    run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Other", "name": "Second", "thickness": "1 mm", "radius": "1 mm",
               "kFactor": 0.5,
               "flanges": [{"edge": edge, "length": "10 mm", "angle": "20 deg"}]}),
    );
    for extra in [json!({"formed": [0]}), json!({"order": [0]})] {
        let mut p = json!({"of": ["Part", "Second"], "punch": plain_punch(), "die": die(),
                           "is": true});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        let a = assertion(&mut s, &format!("two {extra}"), p);
        assert_eq!(a["status"], "error", "{a}");
        assert!(
            a["message"]
                .as_str()
                .unwrap()
                .contains("one sheet-metal part"),
            "{a}"
        );
    }
    // The measure's work cap: 1e6 with an order, 20000 searching.
    for (extra, ok) in [
        (json!({"order": [[0], [1]], "maxWork": 1_000_000}), true),
        (json!({"order": [[0], [1]], "maxWork": 1_000_001}), false),
        (json!({"maxWork": 20_001}), false),
    ] {
        let mut p = json!({"of": "Part", "punch": plain_punch(), "die": die(), "is": true});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        let a = assertion(&mut s, &format!("cap {extra}"), p);
        assert_eq!(a["status"] != "error", ok, "{extra}: {a}");
    }
}

/// A tool point in a hole across a bend that is already made (review
/// 2026-10-31): the made bend's ring sector knows no hole, so a point of
/// the tool found inside it by that test may lie in the hole and is not
/// a certain collision. The part: a base 20 × 60, a flange on its left
/// edge bent 20 degrees and formed before the press brake, a tab 10 wide
/// (inset 25 at both ends) on its right edge; a slot from y = 20 to 40
/// across the left bend. The punch, segmented (as long as the tab's
/// bend), carries a rod at the sheet's mid-thickness reaching 20.2 to
/// either side of its tip: to one side through the base (a flat of the
/// bend, not checked) into the left bend's ring at about 8 degrees of its
/// 20, where the slot is; to the other through the tab's own strip and
/// flat (not checked). Without the slot that point lies in the bend's
/// material for certain.
fn made_bend_part(holes: bool) -> Session {
    let dir =
        std::env::temp_dir().join(format!("craft3d-holed-bend-{}-{holes}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Base"}),
    );
    let made = run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0, 0], "to": [20, 60]}),
    );
    let lines = made["lines"].as_array().unwrap().clone();
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Slot"}),
    );
    run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Slot", "from": [-2, 20], "to": [2, 40]}),
    );
    let made = run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Part", "thickness": "1 mm", "radius": "1 mm",
               "kFactor": 0.5,
               "flanges": [{"edge": lines[3], "length": "10 mm", "angle": "20 deg"},
                           {"edge": lines[1], "length": "10 mm", "angle": "20 deg",
                            "insetStart": "25 mm", "insetEnd": "25 mm"}],
               "cutouts": if holes { json!(["Slot"]) } else { json!([]) }}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    s
}

/// The rod punch: its first point the rod's end at (20.2, −1.45), in the
/// made bend's ring (the bend frame's origin on the tab's axis, the
/// sheet's top skin at y = −1); its own mirror image, so it is checked
/// once.
fn rod() -> Value {
    json!([
        [20.2, -1.45],
        [0.3, -1.45],
        [0.3, 30],
        [-0.3, 30],
        [-0.3, -1.45],
        [-20.2, -1.45],
        [-20.2, -1.55],
        [20.2, -1.55]
    ])
}

#[test]
fn a_tool_point_in_a_hole_across_a_made_bend_is_never_a_certain_collision() {
    let params = json!({"formed": [0], "order": [1], "segmented": true});
    // Without the slot: the rod's end lies in the made bend, for certain.
    let mut solid = made_bend_part(false);
    let got = sequence(&mut solid, rod(), params.clone());
    assert_eq!(got["feasible"], false, "{got}");
    assert_eq!(got["infeasibleProven"], true, "{got}");
    // With it: the made bend's ring sector holds the slot too, so the
    // same point is no certain collision.
    let mut holed = made_bend_part(true);
    let got = sequence(&mut holed, rod(), params);
    assert_eq!(got["feasible"], false, "{got}");
    assert_eq!(got["infeasibleProven"], false, "{got}");
    for c in got["collisions"].as_array().unwrap() {
        assert_eq!(c["certain"], false, "{c}");
        assert_eq!(c["piece"], "the strip of flange 0, bent", "{c}");
    }
    let a = assertion(
        &mut holed,
        "rod",
        json!({"of": "Part", "punch": rod(), "die": die(), "segmented": true,
               "formed": [0], "order": [1], "is": true}),
    );
    assert_eq!(a["status"], "notProven", "{a}");
}
