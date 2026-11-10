//! The proof from meshes before the exact boolean of a collision query
//! (`apart.rs`): the answers of `document.interference`, the overlap
//! measure and a motion's collisions are the same with it as without it,
//! bit for bit, on synthetic pairs (touching, apart by half and twice the
//! tolerance and further, deep overlap, a body nested inside another, a
//! ball in a closed cavity, a pin in a hole with room), on the planetary
//! gearbox and on any number of threads; and the pairs it should prove
//! apart it does prove.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace, set_mesh_proof};
use serde_json::{Value, json};
use std::sync::{Mutex, PoisonError};

/// The tests of this file turn the proof on and off for the whole
/// process: one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn run(session: &mut Session, id: &str, params: &Value) -> Value {
    session
        .execute(id, params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn mm(v: [f64; 3]) -> Value {
    json!(v.map(|x| format!("{x} mm")))
}

fn block(s: &mut Session, name: &str, at: [f64; 3], size: [f64; 3]) {
    run(
        s,
        "feature.block",
        &json!({"name": name, "position": mm(at), "size": mm(size)}),
    );
}

/// `answer` without what only says how it was found (`work.apart`).
fn answer_alone(mut answer: Value) -> Value {
    if let Some(work) = answer.get_mut("work").and_then(Value::as_object_mut) {
        work.remove("apart");
    }
    answer
}

/// `id` with `params` asked with the proof and without it: the two
/// answers, the one with the proof first.
fn both(s: &mut Session, id: &str, params: &Value) -> (Value, Value) {
    set_mesh_proof(true);
    let with = run(s, id, params);
    set_mesh_proof(false);
    let without = run(s, id, params);
    set_mesh_proof(true);
    (with, without)
}

/// Bodies at gaps around the tolerance and far beyond it, overlapping,
/// nested and enclosed: every pair as the kernel alone answers it, and
/// the pairs far apart proven so.
/// The default linear tolerance of a new document (`file.new`).
const TOL: f64 = 1e-6;

/// Gaps along x from a block's face: touching, half the tolerance, twice
/// it, ten times it, a hundredth of the size, and well apart.
fn gaps(k: f64) -> [f64; 6] {
    [0.0, 0.5 * TOL, 2.0 * TOL, 10.0 * TOL, 0.1 * k, 3.0 * k]
}

/// A document at scale `k` with the bodies of the synthetic pairs, and the
/// pairs.
fn synthetic(k: f64) -> (Session, Vec<Value>) {
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", &json!({"units": "mm"}));
    let v = |x: f64, y: f64, z: f64| [x * k, y * k, z * k];
    block(&mut s, "A", v(0.0, 0.0, 0.0), v(10.0, 10.0, 10.0));
    let gaps = gaps(k);
    for (i, gap) in gaps.iter().enumerate() {
        let y = 20.0 * i as f64;
        block(
            &mut s,
            &format!("Base{i}"),
            v(0.0, y, 0.0),
            v(10.0, 10.0, 10.0),
        );
        block(
            &mut s,
            &format!("Gap{i}"),
            [10.0 * k + gap, y * k, 0.0],
            v(5.0, 5.0, 5.0),
        );
    }
    // Deep overlap, and a block nested inside another.
    block(&mut s, "Deep", v(5.0, 5.0, 5.0), v(10.0, 10.0, 10.0));
    block(&mut s, "Big", v(200.0, 0.0, 0.0), v(30.0, 30.0, 30.0));
    block(&mut s, "Nested", v(210.0, 10.0, 10.0), v(5.0, 5.0, 5.0));
    // A ball in a closed cavity: inside the other's box, apart from its
    // walls.
    block(&mut s, "Case", v(300.0, 0.0, 0.0), v(30.0, 30.0, 30.0));
    run(
        &mut s,
        "feature.shell",
        &json!({"source": "Case", "thickness": format!("{} mm", 3.0 * k), "name": "Hollow"}),
    );
    run(
        &mut s,
        "feature.sphere",
        &json!({"name": "Ball", "radius": format!("{} mm", 5.0 * k), "position": mm(v(315.0, 15.0, 15.0))}),
    );
    // A pin in a drilled hole with room around it, and one without.
    block(&mut s, "Plate", v(400.0, 0.0, 0.0), v(20.0, 20.0, 4.0));
    run(
        &mut s,
        "feature.cylinder",
        &json!({"name": "Drill", "radius": format!("{} mm", 3.0 * k), "height": format!("{} mm", 10.0 * k), "position": mm(v(410.0, 10.0, -3.0))}),
    );
    run(
        &mut s,
        "feature.boolean",
        &json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Holed"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        &json!({"name": "Loose", "radius": format!("{} mm", 2.0 * k), "height": format!("{} mm", 12.0 * k), "position": mm(v(410.0, 10.0, -4.0))}),
    );
    run(
        &mut s,
        "feature.cylinder",
        &json!({"name": "Tight", "radius": format!("{} mm", 3.0 * k), "height": format!("{} mm", 12.0 * k), "position": mm(v(410.0, 10.0, -4.0))}),
    );
    let mut pairs = vec![json!(["A", "Deep"]), json!(["Big", "Nested"])];
    for i in 0..gaps.len() {
        pairs.push(json!([format!("Base{i}"), format!("Gap{i}")]));
    }
    pairs.extend([
        json!(["Hollow", "Ball"]),
        json!(["Holed", "Loose"]),
        json!(["Holed", "Tight"]),
        json!(["Loose", "Tight"]),
    ]);
    (s, pairs)
}

/// Bodies at gaps around the tolerance and far beyond it, overlapping,
/// nested and enclosed: every pair as the kernel alone answers it, and
/// the pairs far apart proven so.
#[test]
fn synthetic_pairs_answer_as_the_kernel_alone() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    for k in [0.01, 1.0, 1000.0] {
        let (mut s, pairs) = synthetic(k);
        let params = json!({ "pairs": pairs });
        let (with, without) = both(&mut s, "document.interference", &params);
        assert_eq!(
            answer_alone(with.clone()),
            answer_alone(without.clone()),
            "scale {k}"
        );
        assert_eq!(without["work"]["apart"], 0, "{without}");
        let status = |a: &str, b: &str| {
            with["pairs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["aName"] == a && p["bName"] == b)
                .unwrap_or_else(|| panic!("no pair {a} {b}: {with}"))["status"]
                .clone()
        };
        assert_eq!(status("A", "Deep"), "overlap");
        assert_eq!(status("Big", "Nested"), "overlap");
        assert_eq!(status("Hollow", "Ball"), "clear");
        assert_eq!(status("Holed", "Loose"), "clear");
        assert_eq!(status("Base5", "Gap5"), "clear");
        assert_eq!(status("Base0", "Gap0"), "touch");
        // The ball in its cavity and the loose pin in its hole are inside
        // the other's box and proven apart by the meshes (the far pairs
        // are clear by their boxes already); the nested block is not.
        assert!(
            with["work"]["apart"].as_u64().unwrap() >= 2,
            "scale {k}: {with}"
        );
        // The overlap measure over the same bodies (each answer in a
        // document of its own, so no answer is remembered from the
        // other): the same.
        let measure = json!({"name": "None overlap", "measure": "overlap", "of": ["A", "Deep", "Big", "Nested", "Hollow", "Ball", "Holed", "Loose", "Base5", "Gap5", "Base4", "Gap4"], "is": false});
        let mut checked = Vec::new();
        for on in [true, false] {
            set_mesh_proof(on);
            let (mut s, _) = synthetic(k);
            run(&mut s, "assert.add", &measure);
            checked.push(run(&mut s, "document.check", &json!({})));
        }
        set_mesh_proof(true);
        assert_eq!(checked[0], checked[1], "scale {k}");
    }
}

/// The answers are the same on one thread as on many.
#[test]
fn the_answers_do_not_depend_on_the_threads() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    set_mesh_proof(true);
    let ask = || {
        let mut s = Session::new(Workspace::unrestricted());
        run(&mut s, "file.new", &json!({"units": "mm"}));
        for i in 0..6 {
            let x = 7.0 * f64::from(i);
            block(&mut s, &format!("B{i}"), [x, 0.0, 0.0], [8.0, 8.0, 8.0]);
            run(
                &mut s,
                "feature.sphere",
                &json!({"name": format!("S{i}"), "radius": "3 mm", "position": mm([x + 4.0, 4.0, 12.0 - 0.5 * f64::from(i)])}),
            );
        }
        run(&mut s, "document.interference", &json!({}))
    };
    let many = ask();
    let one = std::thread::spawn(move || {
        craft3d_math::pool::work_alone();
        ask()
    })
    .join()
    .unwrap();
    assert_eq!(many, one);
}

/// The planetary gearbox of the examples: its interference at rest, the
/// collisions of moments of its motion and the overlap assertion over the
/// motion, as without the proof; and its pairs past the box test proven
/// apart.
#[test]
fn the_planetary_gearbox_answers_as_the_kernel_alone() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/testkit/data/planetary-gearbox.3dcraftasm"
    );
    let bytes = std::fs::read(path).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    s.open_bytes("planetary-gearbox.3dcraftasm", &bytes)
        .unwrap();
    let (with, without) = both(&mut s, "document.interference", &json!({"details": false}));
    assert_eq!(answer_alone(with.clone()), answer_alone(without));
    assert_eq!(with["work"]["apart"], with["work"]["checked"], "{with}");
    for t in [0.13, 0.5] {
        set_mesh_proof(true);
        let with = run(&mut s, "view.motion", &json!({"t": t, "collisions": true}));
        set_mesh_proof(false);
        // Another moment between, so the memo of the moment before does
        // not answer.
        run(
            &mut s,
            "view.motion",
            &json!({"t": 0.9, "collisions": true}),
        );
        let without = run(&mut s, "view.motion", &json!({"t": t, "collisions": true}));
        set_mesh_proof(true);
        assert_eq!(with["collisions"], without["collisions"], "t = {t}");
    }
    run(&mut s, "view.motion", &json!({"t": null}));
    let check = json!({"assertions": ["No part overlaps another"], "samples": 3});
    let (with, without) = both(&mut s, "motion.check", &check);
    assert_eq!(with, without);
    assert_eq!(with["ok"], true, "{with}");
    // Its samples one after the other on one thread: the same.
    let alone = std::thread::spawn(move || {
        craft3d_math::pool::work_alone();
        let mut s = Session::new(Workspace::unrestricted());
        s.open_bytes("planetary-gearbox.3dcraftasm", &bytes)
            .unwrap();
        run(&mut s, "motion.check", &check)
    })
    .join()
    .unwrap();
    assert_eq!(with, alone);
}

/// A part whose block moves through another with `t` (apart, touching,
/// overlapping, touching, apart): the collisions a bake finds for its
/// frames, several frames in one pass, and those of each moment shown
/// alone without the proof are the same.
#[test]
fn a_bake_finds_the_collisions_each_moment_has_alone() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let make = || {
        let mut s = Session::new(Workspace::unrestricted());
        run(&mut s, "file.new", &json!({"units": "mm"}));
        run(&mut s, "variable.set", &json!({"name": "t", "expr": "0"}));
        block(&mut s, "Fixed", [0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        block(&mut s, "Far", [0.0, 30.0, 0.0], [10.0, 10.0, 10.0]);
        run(
            &mut s,
            "feature.block",
            &json!({"name": "Mover", "position": ["40 mm * t - 15 mm", "2 mm", "2 mm"], "size": ["5 mm", "5 mm", "5 mm"]}),
        );
        s
    };
    let frames = 8;
    set_mesh_proof(true);
    let mut baked = make();
    run(
        &mut baked,
        "motion.bake",
        &json!({"frames": frames, "collisions": true, "background": false}),
    );
    let mut with = Vec::new();
    for k in 0..=frames {
        let t = f64::from(k) / f64::from(frames);
        with.push(
            run(
                &mut baked,
                "view.motion",
                &json!({"t": t, "collisions": true}),
            )["collisions"]
                .clone(),
        );
    }
    set_mesh_proof(false);
    let mut alone = make();
    let mut without = Vec::new();
    for k in 0..=frames {
        let t = f64::from(k) / f64::from(frames);
        without.push(
            run(
                &mut alone,
                "view.motion",
                &json!({"t": t, "collisions": true}),
            )["collisions"]
                .clone(),
        );
    }
    set_mesh_proof(true);
    assert_eq!(with, without);
    let overlapping = with
        .iter()
        .filter(|c| c.as_array().is_some_and(|a| !a.is_empty()))
        .count();
    assert!(overlapping >= 2, "{with:?}");
}

/// `name` = the union of `a` and `b` (bodies that need not meet: one body
/// of two lumps).
fn union(s: &mut Session, a: &str, b: &str, name: &str) {
    run(
        s,
        "feature.boolean",
        &json!({"op": "union", "target": a, "tool": b, "name": name}),
    );
}

fn ball(s: &mut Session, name: &str, at: [f64; 3], r: f64) {
    run(
        s,
        "feature.sphere",
        &json!({"name": name, "radius": format!("{r} mm"), "position": mm(at)}),
    );
}

/// `name` = a ball of radius `outer` at `at` less one of radius `inner`
/// (a closed cavity, a second shell), the cavity made first or last.
fn hollow_ball(s: &mut Session, name: &str, at: [f64; 3], outer: f64, inner: f64) {
    ball(s, &format!("{name}Out"), at, outer);
    ball(s, &format!("{name}In"), at, inner);
    run(
        s,
        "feature.boolean",
        &json!({"op": "subtract", "target": format!("{name}Out"), "tool": format!("{name}In"), "name": name}),
    );
}

/// Bodies of several shells: two lumps of which one lies inside the
/// other body (in both orders, so either lump may be the first piece of
/// its mesh), hollow balls nested in each other's cavity (apart) and too
/// large for it (overlapping), and a body of two lumps one of which lies
/// in a block's cavity: every pair as the kernel alone answers it.
#[test]
fn bodies_of_several_shells_answer_as_the_kernel_alone() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let make = || {
        let mut s = Session::new(Workspace::unrestricted());
        run(&mut s, "file.new", &json!({"units": "mm"}));
        // A block with two balls as one body: one far outside, one in its
        // middle; once with the far ball first, once with the inner one.
        block(&mut s, "A", [0.0, 0.0, 0.0], [30.0, 30.0, 30.0]);
        ball(&mut s, "Far", [50.0, 15.0, 15.0], 3.0);
        ball(&mut s, "In", [15.0, 15.0, 15.0], 3.0);
        union(&mut s, "Far", "In", "FarFirst");
        block(&mut s, "A2", [0.0, 100.0, 0.0], [30.0, 30.0, 30.0]);
        ball(&mut s, "In2", [15.0, 115.0, 15.0], 3.0);
        ball(&mut s, "Far2", [50.0, 115.0, 15.0], 3.0);
        union(&mut s, "In2", "Far2", "InFirst");
        // Hollow balls: one in the other's cavity, and one too large for
        // it.
        hollow_ball(&mut s, "Shell", [0.0, 300.0, 0.0], 20.0, 15.0);
        hollow_ball(&mut s, "Small", [0.0, 300.0, 0.0], 10.0, 5.0);
        hollow_ball(&mut s, "Shell2", [0.0, 400.0, 0.0], 20.0, 15.0);
        hollow_ball(&mut s, "Big", [0.0, 400.0, 0.0], 18.0, 5.0);
        // A closed box with a ball in its cavity and another far off, as
        // one body.
        block(&mut s, "Box", [200.0, 0.0, 0.0], [30.0, 30.0, 30.0]);
        run(
            &mut s,
            "feature.shell",
            &json!({"source": "Box", "thickness": "3 mm", "name": "Closed"}),
        );
        ball(&mut s, "Inside", [215.0, 15.0, 15.0], 4.0);
        ball(&mut s, "Outside", [260.0, 15.0, 15.0], 4.0);
        union(&mut s, "Outside", "Inside", "Pair");
        s
    };
    let pairs = json!({"pairs": [["A", "FarFirst"], ["A2", "InFirst"], ["Shell", "Small"], ["Shell2", "Big"], ["Closed", "Pair"]]});
    let mut s = make();
    let (with, without) = both(&mut s, "document.interference", &pairs);
    assert_eq!(answer_alone(with.clone()), answer_alone(without.clone()));
    let status = |a: &str| {
        with["pairs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["aName"] == a)
            .unwrap_or_else(|| panic!("no pair of {a}: {with}"))["status"]
            .clone()
    };
    assert_eq!(status("A"), "overlap", "{with}");
    assert_eq!(status("A2"), "overlap", "{with}");
    assert_eq!(status("Shell"), "clear", "{with}");
    assert_eq!(status("Shell2"), "overlap", "{with}");
    assert_eq!(status("Closed"), "clear", "{with}");
    // The overlap measure and the collisions of the motion's machinery:
    // the same.
    for (a, b) in [("A", "FarFirst"), ("A2", "InFirst"), ("Shell", "Small")] {
        let mut checked = Vec::new();
        for on in [true, false] {
            set_mesh_proof(on);
            let mut s = make();
            run(
                &mut s,
                "assert.add",
                &json!({"name": "No overlap", "measure": "overlap", "of": [a, b], "is": false}),
            );
            checked.push(run(&mut s, "document.check", &json!({})));
        }
        set_mesh_proof(true);
        assert_eq!(checked[0], checked[1], "{a} {b}");
    }
}
