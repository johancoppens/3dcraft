//! The floating equilibrium as assertion and output measures
//! (`floatingOffset`, `floatingHeel`, `floatingTrim`, `floatingGM`,
//! `floatingGML`, `floatingStable`; docs/design/sections-and-levels.md ›
//! *A level with a tilt*), held to the closed forms of a box barge at
//! 1e-3, 1 and 1e6: L × B × D = 2000 × 600 × 400 displacing L B T with
//! T = 250, so KB = T / 2 = 125, BM = B² / (12 T) = 120, BML = L² / (12 T)
//! = 1333.3, and with KG = 180, GM = KB + BM − KG = 65 and GML = 1278.3.
//! Wall-sided, a centre of gravity forward by xg trims it by θ with
//! xg = tan θ (GML + BML tan² θ / 2). The centre of gravity comes from an
//! output; an output that is unknown leaves the assertion notProven.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn refused(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should fail, returned {value}"),
        Err(e) => e.message().to_string(),
    }
}

#[track_caller]
fn close(got: f64, want: f64, rel: f64, what: &str) {
    assert!(
        (got - want).abs() <= rel * want.abs().max(1e-300),
        "{what}: {got} is not {want} within {rel}"
    );
}

fn assertion(session: &mut Session, name: &str) -> Value {
    let listed = run(session, "assert.list", json!({"details": true}));
    listed["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no assertion {name}: {listed}"))
}

fn output(session: &mut Session, name: &str) -> Value {
    let listed = run(session, "output.list", json!({}));
    listed["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no output {name}: {listed}"))
}

fn mm(v: f64) -> String {
    format!("{v} mm")
}

/// The barge at scale `k`, with the variables `xg` and `KG` (an input)
/// and the output `zG` (the centre of gravity's height, from `KG`).
fn barge(k: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6 * k}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [mm(2000.0 * k), mm(600.0 * k), mm(400.0 * k)],
               "position": [mm(-1000.0 * k), mm(-300.0 * k), "0 mm"], "name": "Barge"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "sc", "expr": k.to_string()}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "KG", "expr": "180 mm * sc"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "xg", "expr": "0 mm"}),
    );
    run(
        &mut s,
        "variable.output",
        json!({"name": "zG", "expr": "KG"}),
    );
    s
}

/// The floating parameters of the barge: its displacement and the
/// centre of gravity `[xg, 0, zG]`, `zG` an output.
fn afloat(k: f64, measure: &str, more: Value) -> Value {
    let mut p = json!({
        "measure": measure,
        "of": "Barge",
        "volume": 2000.0 * 600.0 * 250.0 * k.powi(3),
        "centerOfGravity": ["xg", "0 mm", "zG"],
    });
    for (key, v) in more.as_object().unwrap() {
        p[key] = v.clone();
    }
    p
}

fn add(s: &mut Session, name: &str, k: f64, measure: &str, more: Value) -> Value {
    let mut p = afloat(k, measure, more);
    p["name"] = json!(name);
    run(s, "assert.add", p);
    assertion(s, name)
}

#[test]
fn a_box_barge_floats_at_its_closed_form_at_three_scales() {
    for k in SCALES {
        let mut s = barge(k);
        let draft = add(
            &mut s,
            "draft",
            k,
            "floatingOffset",
            json!({"eq": mm(250.0 * k)}),
        );
        assert_eq!(draft["status"], "passed", "scale {k}: {draft}");
        let gm = add(&mut s, "GM", k, "floatingGM", json!({"min": "0 mm"}));
        assert_eq!(gm["status"], "passed", "scale {k}: {gm}");
        assert_eq!(gm["unit"], "mm", "{gm}");
        close(gm["measured"].as_f64().unwrap(), 65.0 * k, 1e-7, "GM");
        let gml = add(&mut s, "GML", k, "floatingGML", json!({"min": "0 mm"}));
        let bml = 2000.0_f64.powi(2) / (12.0 * 250.0);
        close(
            gml["measured"].as_f64().unwrap(),
            (125.0 + bml - 180.0) * k,
            1e-7,
            "GML",
        );
        let heel = add(
            &mut s,
            "level",
            k,
            "floatingHeel",
            json!({"min": "-0.001 deg", "max": "0.001 deg"}),
        );
        assert_eq!(heel["status"], "passed", "scale {k}: {heel}");
        assert_eq!(heel["unit"], "rad", "{heel}");
        let stable = add(&mut s, "upright", k, "floatingStable", json!({"is": true}));
        assert_eq!(stable["status"], "passed", "scale {k}: {stable}");
        // The same as outputs, the centre of gravity's height an output
        // itself.
        run(
            &mut s,
            "variable.measure",
            afloat(k, "floatingGM", json!({"name": "GMf"})),
        );
        let out = output(&mut s, "GMf");
        assert_eq!(out["status"], "known", "{out}");
        close(out["value"].as_f64().unwrap(), 65.0 * k, 1e-7, "GM output");
        run(
            &mut s,
            "variable.measure",
            afloat(k, "floatingStable", json!({"name": "ok"})),
        );
        assert_eq!(output(&mut s, "ok")["value"], 1.0);

        // Too high a centre of gravity: GM = 125 + 120 − 300 < 0, upright
        // is unstable, and only what uses KG is measured again.
        run(
            &mut s,
            "variable.set",
            json!({"name": "KG", "expr": "300 mm * sc"}),
        );
        let gm = assertion(&mut s, "GM");
        assert_eq!(gm["status"], "failed", "scale {k}: {gm}");
        close(gm["measured"].as_f64().unwrap(), -55.0 * k, 1e-7, "GM high");
        assert_eq!(assertion(&mut s, "upright")["status"], "failed");
        assert_eq!(output(&mut s, "ok")["value"], 0.0);
        run(
            &mut s,
            "variable.set",
            json!({"name": "KG", "expr": "180 mm * sc"}),
        );

        // Trimmed by a centre of gravity forward: tan θ = 0.05.
        let t = 0.05_f64;
        let xg = t * (125.0 + bml - 180.0 + bml * t * t / 2.0) * k;
        run(
            &mut s,
            "variable.set",
            json!({"name": "xg", "expr": mm(xg)}),
        );
        let trim = add(
            &mut s,
            "trim",
            k,
            "floatingTrim",
            json!({"min": "-3 deg", "max": "3 deg"}),
        );
        assert_eq!(trim["status"], "passed", "scale {k}: {trim}");
        close(
            trim["measured"].as_f64().unwrap().abs(),
            t.atan(),
            1e-6,
            "trim",
        );
        let tight = add(
            &mut s,
            "tight trim",
            k,
            "floatingTrim",
            json!({"min": "-2 deg", "max": "2 deg"}),
        );
        assert_eq!(tight["status"], "failed", "scale {k}: {tight}");
        // Trimmed, it does not heel.
        assert_eq!(assertion(&mut s, "level")["status"], "passed");
    }
}

#[test]
fn mass_and_density_give_the_same_draft() {
    let mut s = barge(1.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "cargo", "expr": "300"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "draft", "measure": "floatingOffset", "of": "Barge", "mass": "cargo",
               "density": 1000, "centerOfGravity": [0, 0, "zG"], "eq": "250 mm"}),
    );
    assert_eq!(assertion(&mut s, "draft")["status"], "passed");
    // Renaming the variable renames it inside the measure.
    run(
        &mut s,
        "variable.rename",
        json!({"name": "cargo", "to": "load"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    let stored = listed["assertions"][0]["measure"].to_string();
    assert!(stored.contains("load"), "{stored}");
    assert_eq!(assertion(&mut s, "draft")["status"], "passed");
}

#[test]
fn an_unknown_centre_of_gravity_leaves_the_assertion_not_proven() {
    let mut s = barge(1.0);
    // More than the barge holds: no level, so `T` is unknown.
    run(
        &mut s,
        "variable.measure",
        json!({"name": "T", "measure": "levelFor", "of": "Barge", "volume": "2 * (1 m)^3"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "GM", "measure": "floatingGM", "of": "Barge", "volume": 3e8,
               "centerOfGravity": [0, 0, "T"], "min": "0 mm"}),
    );
    let gm = assertion(&mut s, "GM");
    assert_eq!(gm["status"], "notProven", "{gm}");
    assert!(gm["message"].as_str().unwrap().contains("`T`"), "{gm}");
    // A displacement the barge cannot hold is an error, not a pass.
    run(
        &mut s,
        "assert.add",
        json!({"name": "sunk", "measure": "floatingGM", "of": "Barge", "volume": 1e9,
               "centerOfGravity": [0, 0, 100], "min": "0 mm"}),
    );
    assert_eq!(assertion(&mut s, "sunk")["status"], "error");
}

/// A sphere with its centre of gravity at its centre floats neutral in
/// every direction (every metacentric height 0, within the floor): stable
/// is not proven either way, through the assertion and as an output
/// (review 2026-10-29: it was a certain no). Off the centre, below it,
/// it is stable.
#[test]
fn a_neutral_sphere_is_neither_stable_nor_unstable_for_certain() {
    for k in SCALES {
        let mut s = Session::new(Workspace::none());
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 1e-6 * k}),
        );
        run(
            &mut s,
            "feature.sphere",
            json!({"radius": mm(100.0 * k), "name": "Ball"}),
        );
        let whole = 4.0 / 3.0 * std::f64::consts::PI * (100.0 * k).powi(3);
        run(
            &mut s,
            "assert.add",
            json!({"name": "stable", "measure": "floatingStable", "of": "Ball", "volume": 0.3 * whole,
                   "centerOfGravity": [0, 0, 0], "is": true}),
        );
        let a = assertion(&mut s, "stable");
        assert_eq!(a["status"], "notProven", "scale {k}: {a}");
        assert!(
            a["message"]
                .as_str()
                .unwrap()
                .contains("neutral within measurement"),
            "{a}"
        );
        run(
            &mut s,
            "variable.measure",
            json!({"name": "upright", "measure": "floatingStable", "of": "Ball", "volume": 0.3 * whole,
                   "centerOfGravity": [0, 0, 0]}),
        );
        assert_eq!(output(&mut s, "upright")["status"], "unknown");
        run(
            &mut s,
            "assert.add",
            json!({"name": "low", "measure": "floatingStable", "of": "Ball", "volume": 0.3 * whole,
                   "centerOfGravity": [0, 0, mm(-30.0 * k)], "is": true}),
        );
        let a = assertion(&mut s, "low");
        assert_eq!(a["status"], "passed", "scale {k}: {a}");
    }
}

/// A table looked up only inside a measure's own parameters is used:
/// `table.delete` without force refuses, naming the assertion and the
/// output (review 2026-10-29: it saw only an assertion's bounds), and
/// `table.list` lists them.
#[test]
fn a_table_used_inside_a_measure_is_not_deleted_without_force() {
    let mut s = barge(1.0);
    run(
        &mut s,
        "table.set",
        json!({"name": "loads", "columns": ["load", "v"],
               "rows": [["light", 3e8]]}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "GM", "measure": "floatingGM", "of": "Barge", "volume": "loads[1].v",
               "centerOfGravity": [0, 0, "zG"], "min": "0 mm"}),
    );
    run(
        &mut s,
        "variable.measure",
        json!({"name": "draftOut", "measure": "floatingOffset", "of": "Barge", "volume": "loads[1].v",
               "centerOfGravity": [0, 0, "zG"]}),
    );
    let e = refused(&mut s, "table.delete", json!({"name": "loads"}));
    assert!(e.contains("assertion `GM`"), "{e}");
    assert!(e.contains("output `draftOut`"), "{e}");
    let got = run(&mut s, "table.list", json!({"name": "loads"}));
    assert_eq!(
        got["usedBy"],
        json!(["assertion `GM`", "output `draftOut`"]),
        "{got}"
    );
    run(
        &mut s,
        "table.delete",
        json!({"name": "loads", "force": true}),
    );
}

#[test]
fn bad_parameters_are_errors_not_crashes() {
    let mut s = barge(1.0);
    let cog = json!([0, 0, 100]);
    for (params, words) in [
        (
            json!({"measure": "floatingGM", "of": "Barge", "centerOfGravity": cog, "min": 0}),
            "volume",
        ),
        (
            json!({"measure": "floatingGM", "of": "Barge", "volume": 3e8, "min": 0}),
            "centerOfGravity",
        ),
        (
            json!({"measure": "floatingGM", "of": "Barge", "volume": 3e8, "centerOfGravity": [0, 0], "min": 0}),
            "centerOfGravity",
        ),
        (
            json!({"measure": "floatingHeel", "of": "Barge", "volume": 3e8, "centerOfGravity": cog, "heel": "5 mm", "min": 0}),
            "heel",
        ),
        (
            json!({"measure": "floatingTrim", "of": "Nowhere", "volume": 3e8, "centerOfGravity": cog, "max": "1 deg"}),
            "Nowhere",
        ),
        // A plain number bounding an angle: the heel's own 5 is degrees,
        // but a bound of 5 would be 5 radians (review 2026-10-29).
        (
            json!({"measure": "floatingHeel", "of": "Barge", "volume": 3e8, "centerOfGravity": cog, "heel": 5, "max": 5}),
            "plain number",
        ),
        (
            json!({"measure": "floatingTrim", "of": "Barge", "volume": 3e8, "centerOfGravity": cog, "min": "-2", "max": "2 deg"}),
            "plain number",
        ),
    ] {
        let mut p = params.clone();
        p["name"] = json!("bad");
        run(&mut s, "assert.add", p);
        let a = assertion(&mut s, "bad");
        assert_eq!(a["status"], "error", "{params}: {a}");
        let message = a["message"].as_str().unwrap();
        assert!(message.contains(words), "{params}: {message}");
        assert!(!message.contains("internal error"), "{params}: {message}");
        run(&mut s, "assert.delete", json!({"name": "bad"}));
    }
    // A key the measure does not take, a stable measure with a bound, a
    // number with `is`: refused at once.
    for (params, words) in [
        (
            json!({"name": "x", "measure": "floatingGM", "volume": 3e8, "centerOfGravity": cog, "punch": [], "min": 0}),
            "punch",
        ),
        (
            json!({"name": "x", "measure": "floatingStable", "volume": 3e8, "centerOfGravity": cog, "min": 0}),
            "is",
        ),
        (
            json!({"name": "x", "measure": "floatingGM", "volume": 3e8, "centerOfGravity": cog, "is": true}),
            "is",
        ),
    ] {
        let message = refused(&mut s, "assert.add", params.clone());
        assert!(message.contains(words), "{params}: {message}");
    }
}
