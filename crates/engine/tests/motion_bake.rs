//! A motion made ahead (`motion.bake`, docs/design/motion.md ›
//! *Configurations over a motion*): an assembly whose instance takes a
//! parameter driven by `t` (a configuration per moment) and a part whose
//! block grows with `t`. Baked, a play shows what an unbaked play shows,
//! bit for bit, whatever the budget of configurations; a bake in pieces
//! (`limit`) builds what one bake builds; the document does not change.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{error, session, session_in, workdir};
use craft3d_engine::Session;
use serde_json::json;

/// A bar part with a published length, its body `Bar` 4 wide and 2
/// thick, and datum axes `End1` and `End2` 2 in from its ends.
fn bar_part(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({ "name": "len", "expr": "24 mm" }));
    run(
        s,
        "variable.publish",
        json!({ "name": "len", "min": "10 mm", "max": "500 mm" }),
    );
    run(
        s,
        "feature.block",
        json!({ "name": "Bar", "size": ["len", 4, 2] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End1", "through": [[2, 2, 0], [2, 2, 1]] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]] }),
    );
    run(s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(s, "file.close", json!({}));
}

/// A crank turned by `360 deg * t` about a fixed ground bar, and a bar
/// held to the world whose length is `30 mm + 20 mm * t`: a configuration
/// of its own at every moment.
fn stretching(s: &mut Session) {
    run(s, "assembly.new", json!({}));
    let bar = |name: &str, len: &str, y: f64| {
        json!({ "source": "bar.3dcraft", "name": name, "body": "Bar",
                "params": { "len": len }, "position": [0, y, 0] })
    };
    run(s, "instance.add", bar("Ground", "64 mm", 0.0));
    run(s, "instance.add", bar("Crank", "24 mm", 0.0));
    run(s, "instance.add", bar("Belt", "30 mm + 20 mm * t", 20.0));
    run(s, "variable.set", json!({ "name": "t", "expr": "0" }));
    for (name, kind, a, b) in [
        ("Base", "fixed", "world/z", "Ground/End1"),
        ("Drive", "revolute", "Ground/End1", "Crank/End1"),
        ("Hold", "fixed", "world/z", "Belt/End1"),
    ] {
        run(
            s,
            "mate.add",
            json!({ "kind": kind, "a": a, "b": b, "name": name, "flip": true }),
        );
    }
    run(
        s,
        "variable.set",
        json!({ "name": "Drive_angle", "expr": "360 deg * t" }),
    );
}

/// What a play at `frames` frames shows: per frame and per body, its name,
/// its triangles and every bit of its mesh's points.
fn played(s: &mut Session, frames: u32) -> Vec<Vec<(String, usize, u64)>> {
    let mut out = Vec::new();
    for k in 0..=frames {
        let t = f64::from(k) / f64::from(frames);
        let shown = run(s, "view.motion", json!({ "t": t }));
        assert_eq!(shown["t"], t, "{shown}");
        let scene = s.scene(None).unwrap().unwrap();
        out.push(
            scene
                .bodies
                .iter()
                .map(|body| {
                    let mut digest = 0xcbf2_9ce4_8422_2325u64;
                    for p in &body.mesh.positions {
                        for v in [p.x, p.y, p.z] {
                            digest = (digest ^ v.to_bits()).wrapping_mul(0x100_0000_01b3);
                        }
                    }
                    (body.name.clone(), body.mesh.positions.len(), digest)
                })
                .collect(),
        );
    }
    out
}

fn assembly_session(name: &str) -> Session {
    let dir = workdir("motion-bake", name);
    let mut s = session_in(&dir);
    bar_part(&mut s);
    stretching(&mut s);
    s
}

#[test]
fn a_baked_assembly_motion_plays_what_an_unbaked_one_plays() {
    const FRAMES: u32 = 16;
    let mut s = assembly_session("plain");
    let unbaked = played(&mut s, FRAMES);
    // The belt is longer at the end than at the start.
    assert_ne!(unbaked[0], unbaked[FRAMES as usize]);

    let mut s = assembly_session("baked");
    let before = run(&mut s, "document.inspect", json!({}));
    let baked = run(&mut s, "motion.bake", json!({ "frames": FRAMES }));
    assert_eq!(baked["done"], true, "{baked}");
    assert_eq!(baked["remaining"], 0, "{baked}");
    let configs = &baked["configurations"];
    // The frames ask for Ground's and Crank's and the belt's at 17 frames;
    // built: the belt at the grid points the frames are carried through
    // too (t = k/128 up to 1, the frames among them: k/16 = 8k/128), less
    // what the assembly at rest built already.
    assert_eq!(configs["wanted"], 2 + 17, "{baked}");
    assert!(configs["built"].as_u64().unwrap() >= 128, "{baked}");
    assert_eq!(configs["failed"], 0, "{baked}");
    assert_eq!(baked["overBudget"], 0, "{baked}");
    assert!(baked["megabytes"].as_f64().unwrap() > 0.0, "{baked}");
    // A second bake finds everything kept.
    let again = run(&mut s, "motion.bake", json!({ "frames": FRAMES }));
    assert_eq!(again["configurations"]["built"], 0, "{again}");
    assert_eq!(played(&mut s, FRAMES), unbaked);
    // View state only.
    run(&mut s, "view.motion", json!({ "t": null }));
    assert_eq!(run(&mut s, "document.inspect", json!({})), before);

    // With room for almost nothing the play is the same: what was let go
    // of is built again.
    let mut s = assembly_session("tight");
    s.set_config_budget(1);
    let baked = run(&mut s, "motion.bake", json!({ "frames": FRAMES }));
    assert_eq!(baked["done"], true, "{baked}");
    assert!(baked["overBudget"].as_u64().unwrap() > 0, "{baked}");
    assert_eq!(played(&mut s, FRAMES), unbaked);
}

#[test]
fn a_bake_in_pieces_builds_what_one_bake_builds() {
    let mut s = assembly_session("pieces");
    let mut built = 0;
    let mut calls = 0;
    loop {
        let baked = run(&mut s, "motion.bake", json!({ "frames": 8, "limit": 40 }));
        built += baked["configurations"]["built"].as_u64().unwrap();
        calls += 1;
        assert!(baked["configurations"]["built"].as_u64().unwrap() <= 40);
        if baked["done"] == true {
            break;
        }
        assert!(calls < 20, "{baked}");
    }
    // The belt at 129 grid points (8 frames all on the grid), less what
    // the placement at rest built already.
    assert!(built >= 129 - 1, "{built}");
    assert!(calls >= 3, "{calls}");
    let mut whole = assembly_session("whole");
    let baked = run(&mut whole, "motion.bake", json!({ "frames": 8 }));
    assert_eq!(baked["configurations"]["wanted"], 2 + 9, "{baked}");
    assert_eq!(baked["configurations"]["built"], built, "{baked}");
    assert_eq!(played(&mut s, 8), played(&mut whole, 8));
}

/// A bake in pieces whose batch holds more than the budget keeps (each
/// call builds a few configurations, the budget would let the earlier
/// ones go): the batch being built is held until it is done, so the bake
/// gets there, and plays what an unbaked motion plays.
#[test]
fn a_bake_in_pieces_over_a_tiny_budget_gets_done() {
    let mut plain = assembly_session("tiny-plain");
    let unbaked = played(&mut plain, 8);
    let mut s = assembly_session("tiny");
    s.set_config_budget(1);
    let mut calls = 0;
    loop {
        let baked = run(&mut s, "motion.bake", json!({ "frames": 8, "limit": 5 }));
        calls += 1;
        if baked["done"] == true {
            break;
        }
        assert!(calls < 100, "never done: {baked}");
    }
    assert!(calls >= 20, "{calls}");
    assert_eq!(played(&mut s, 8), unbaked);
}

/// A part whose block grows with `t` into a fixed one.
fn growing(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        s,
        "feature.block",
        json!({ "name": "Grow", "size": ["10 mm + 20 mm * t", 5, 5] }),
    );
    run(
        s,
        "feature.block",
        json!({ "name": "Wall", "size": [5, 5, 5], "position": [20, 0, 0] }),
    );
}

#[test]
fn a_baked_part_motion_shows_its_moments_and_collisions_as_unbaked() {
    const FRAMES: u32 = 8;
    let mut plain = session();
    growing(&mut plain);
    run(&mut plain, "view.motion", json!({ "collisions": true }));
    let mut unbaked_collisions = Vec::new();
    let mut unbaked = Vec::new();
    for k in 0..=FRAMES {
        let t = f64::from(k) / f64::from(FRAMES);
        let shown = run(&mut plain, "view.motion", json!({ "t": t }));
        unbaked_collisions.push(shown["collisions"].clone());
        unbaked.push(shown["rebuilt"].clone());
    }
    let unbaked_scenes = played(&mut plain, FRAMES);
    // Past t = 0.5 the block reaches into the wall.
    assert_eq!(unbaked_collisions[0], json!([]));
    assert_eq!(
        unbaked_collisions[FRAMES as usize]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let mut s = session();
    growing(&mut s);
    let baked = run(
        &mut s,
        "motion.bake",
        json!({ "frames": FRAMES, "collisions": true }),
    );
    assert_eq!(baked["done"], true, "{baked}");
    assert_eq!(baked["moments"], FRAMES + 1, "{baked}");
    assert_eq!(baked["collisions"], FRAMES + 1, "{baked}");
    assert!(baked["configurations"].is_null(), "{baked}");
    run(&mut s, "view.motion", json!({ "collisions": true }));
    for k in 0..=FRAMES {
        let t = f64::from(k) / f64::from(FRAMES);
        let shown = run(&mut s, "view.motion", json!({ "t": t }));
        assert_eq!(shown["collisions"], unbaked_collisions[k as usize], "{t}");
        assert_eq!(shown["rebuilt"], unbaked[k as usize], "{t}");
    }
    assert_eq!(played(&mut s, FRAMES), unbaked_scenes);
}

#[test]
fn a_bake_without_a_timeline_or_with_bad_parameters_is_refused() {
    let mut s = session();
    assert!(error(&mut s, "motion.bake", json!({})).contains("no document"));
    run(&mut s, "file.new", json!({}));
    let why = error(&mut s, "motion.bake", json!({}));
    assert!(why.contains("timeline"), "{why}");
    run(
        &mut s,
        "variable.set",
        json!({ "name": "t", "expr": "4 mm" }),
    );
    let why = error(&mut s, "motion.bake", json!({}));
    assert!(why.contains("not the timeline"), "{why}");
    run(&mut s, "variable.set", json!({ "name": "t", "expr": "0" }));
    for params in [
        json!({ "frames": 0 }),
        json!({ "frames": 1001 }),
        json!({ "frames": "many" }),
        json!({ "limit": 0 }),
        json!({ "collisions": 3 }),
        json!({ "speed": 2 }),
    ] {
        let why = error(&mut s, "motion.bake", params.clone());
        assert!(why.starts_with("motion.bake"), "{params}: {why}");
    }
    // Nothing to build in a document without features: done at once.
    let baked = run(&mut s, "motion.bake", json!({ "frames": 4 }));
    assert_eq!(baked["done"], true, "{baked}");
    assert_eq!(baked["moments"], 5, "{baked}");
}
