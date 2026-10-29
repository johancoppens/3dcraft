//! `sheetMetal.bendSequence` with strokes, segmented tools and a given
//! order (docs/design/sheetmetal.md › *Bend sequence*), at 1e-3, 1 and
//! 1e6. A base 100 × 30 (R = T = 1, K = ½) with two tabs on its bottom
//! edge (x from 10 to 40 and from 60 to 90, 15 high) and a wall on its
//! right edge (10 high), bent with a straight punch 10 wide and a die 30
//! wide:
//!
//! - the two tabs lie on one line with the same angle and side, so they
//!   are made in one stroke. With segmented tooling (`segmented`) the
//!   order is the wall, then both tabs; the tools of that stroke are a
//!   segment per tab, as long as the tab, so
//!   the punch's least distance is the 10 from the second segment's end
//!   (x = 90) to the wall's bend (from x = 100), reached while the
//!   punch's corner passes the bend's thickness. With the default
//!   one-piece tools, reaching past the part, the wall cannot come first
//!   (the punch along the tabs' line meets it standing): the default
//!   fails closed, and `unchecked` says which tooling was assumed;
//! - an order given is checked as given (the tabs one by one too); one
//!   that groups bends not on one line, lists a flange twice or leaves
//!   one out is an error;
//! - a channel whose walls collide in either order fails the order given,
//!   naming the step;
//! - the work cap is the caller's (`maxWork`): too small an error that
//!   says so.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

const STRAIGHT: [[f64; 2]; 4] = [[-5.0, 0.0], [5.0, 0.0], [5.0, 100.0], [-5.0, 100.0]];
const DIE: [[f64; 2]; 4] = [[-15.0, -20.0], [15.0, -20.0], [15.0, 0.0], [-15.0, 0.0]];

fn mm(v: f64) -> String {
    format!("{v} mm")
}

fn scaled(points: &[[f64; 2]], k: f64) -> Value {
    json!(
        points
            .iter()
            .map(|p| [p[0] * k, p[1] * k])
            .collect::<Vec<_>>()
    )
}

/// The part `Tabs`: the base `w` × 30 and the flanges given (each its
/// edge of the rectangle from the bottom on, length, insets).
fn part(s: &mut Session, k: f64, w: f64, flanges: &[(usize, f64, [f64; 2])]) {
    run(s, "sketch.create", json!({"plane": "xy", "name": "Base"}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [w * k, 30.0 * k]}),
    );
    let lines: Vec<String> = made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    let flanges: Vec<Value> = flanges
        .iter()
        .map(|(edge, h, [a, b])| {
            json!({"edge": lines[*edge], "length": mm(h * k), "angle": "90 deg",
                   "insetStart": mm(a * k), "insetEnd": mm(b * k)})
        })
        .collect();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Tabs", "thickness": mm(k), "radius": mm(k),
               "kFactor": 0.5, "flanges": flanges}),
    );
}

fn tabs(s: &mut Session, k: f64) {
    part(
        s,
        k,
        100.0,
        &[
            (0, 15.0, [10.0, 60.0]),
            (0, 15.0, [60.0, 10.0]),
            (1, 10.0, [0.0, 0.0]),
        ],
    );
}

fn sequence(s: &mut Session, k: f64, extra: Value) -> Value {
    let mut params = json!({"feature": "Tabs", "punch": scaled(&STRAIGHT, k),
                            "die": scaled(&DIE, k)});
    for (key, v) in extra.as_object().unwrap() {
        params[key] = v.clone();
    }
    run(s, "sheetMetal.bendSequence", params)
}

#[test]
fn tabs_on_one_line_are_bent_in_one_stroke() {
    for k in SCALES {
        let dir = workdir("sheetmetal-strokes", &format!("tabs-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        tabs(&mut s, k);
        let out = sequence(&mut s, k, json!({}));
        assert_ne!(out["order"], json!([2, [0, 1]]), "scale {k}: {out}");
        // In either order the one-piece tools meet what stands: the
        // tabs' punch the wall, and the wall's punch the second tab, its
        // top swinging past the wall's axis as the base rises: no order
        // with these tools, for certain.
        assert_eq!(out["feasible"], false, "scale {k}: {out}");
        assert_eq!(out["infeasibleProven"], true, "scale {k}: {out}");
        let unchecked = out["unchecked"].as_str().unwrap();
        assert!(unchecked.contains("one-piece tools"), "{unchecked}");
        let steps = out["steps"].as_array().unwrap();
        assert!(steps.iter().all(|st| st.get("segments").is_none()), "{out}");
        let out = sequence(&mut s, k, json!({"segmented": true}));
        assert_eq!(out["feasible"], true, "scale {k}: {out}");
        assert_eq!(out["given"], false, "{out}");
        let unchecked = out["unchecked"].as_str().unwrap();
        assert!(unchecked.contains("segmented tooling"), "{unchecked}");
        // The wall (the shorter flange) first, then both tabs at once.
        assert_eq!(out["order"], json!([2, [0, 1]]), "scale {k}: {out}");
        let steps = out["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 2, "{out}");
        assert_eq!(steps[1]["flanges"], json!([0, 1]), "{out}");
        assert_eq!(steps[1]["done"], json!([2]), "{out}");
        // Each step lists its segments' lengths: the tabs 30 long, the
        // wall 30 (the base's height).
        for (step, n) in [(1, 2), (0, 1)] {
            let segments = steps[step]["segments"].as_array().unwrap();
            assert_eq!(segments.len(), n, "{out}");
            for l in segments {
                let l = l.as_f64().unwrap();
                assert!((l - 30.0 * k).abs() <= 1e-9 * k, "scale {k}: {l}: {out}");
            }
        }
        // The punch's segments end at x = 40 and x = 90; the wall's bend
        // starts at x = 100: 10 from the second segment's end face. What
        // is proven at an angle is the mesh's lower bound, within the
        // tolerance (1e-6 k) of the true distance below it.
        let least = steps[1]["least"].as_f64().unwrap();
        assert!(
            least <= 10.0 * k && 10.0 * k - least <= 2e-6 * k,
            "scale {k}: least {least} against {}: {out}",
            10.0 * k
        );
        let proven = steps[1]["proven"].as_f64().unwrap();
        assert!(
            proven <= least && proven >= out["clearance"].as_f64().unwrap(),
            "{out}"
        );

        // Given: the tabs one by one, or together.
        for (order, n) in [(json!([2, 0, 1]), 3), (json!([[0, 1], 2]), 2)] {
            let out = sequence(&mut s, k, json!({"order": order, "segmented": true}));
            assert_eq!(out["feasible"], true, "scale {k} {order}: {out}");
            assert_eq!(out["given"], true, "{out}");
            assert_eq!(out["order"], order, "{out}");
            assert_eq!(out["steps"].as_array().unwrap().len(), n, "{out}");
        }
        // Orders that are not orders of this part.
        for (order, why) in [
            (json!([[0, 2], 1]), "one stroke"),
            (json!([0, 0, 1, 2]), "twice"),
            (json!([0, 1]), "leaves out flanges [2]"),
            (json!([0, 1, 7]), "index"),
            (json!([[], 0, 1, 2]), "empty stroke"),
            (json!("0, 1, 2"), "list of flanges"),
            // One-piece tools bend both tabs at once: an order that
            // splits them is not an order of this brake.
            (json!([0, 2, 1]), "segmented"),
            (json!([2, 0, 1]), "segmented"),
        ] {
            let mut params = json!({"feature": "Tabs", "punch": scaled(&STRAIGHT, k),
                                    "die": scaled(&DIE, k)});
            params["order"] = order.clone();
            let got = error(&mut s, "sheetMetal.bendSequence", params);
            assert!(got.contains(why), "{order}: {got}");
        }
    }
}

#[test]
fn an_order_given_that_collides_names_its_step() {
    for k in SCALES {
        let dir = workdir("sheetmetal-strokes", &format!("given-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        // A U channel, walls 40 high on the left and right edges of a base
        // 40 wide: the second wall swings into the punch in either order.
        part(
            &mut s,
            k,
            40.0,
            &[(3, 40.0, [0.0, 0.0]), (1, 40.0, [0.0, 0.0])],
        );
        for (order, second, first) in [(json!([0, 1]), 1, 0), (json!([1, 0]), 0, 1)] {
            let out = sequence(&mut s, k, json!({"order": order}));
            assert_eq!(out["feasible"], false, "scale {k}: {out}");
            assert_eq!(out["order"], Value::Null, "{out}");
            let steps = out["steps"].as_array().unwrap();
            assert_eq!(steps.len(), 1, "{out}");
            assert_eq!(steps[0]["flange"], first, "{out}");
            let collisions = out["collisions"].as_array().unwrap();
            assert_eq!(collisions.len(), 1, "{out}");
            assert_eq!(collisions[0]["flange"], second, "{out}");
            assert_eq!(collisions[0]["done"], json!([first]), "{out}");
            assert_eq!(collisions[0]["tool"], "punch", "{out}");
            assert_eq!(out["given"], true, "{out}");
            assert_eq!(out["infeasibleProven"], true, "{out}");
        }
    }
}

#[test]
fn the_work_cap_is_the_callers() {
    let k = 1.0;
    let dir = workdir("sheetmetal-strokes", "work");
    let mut s = session_in(&dir);
    new_document(&mut s, k);
    tabs(&mut s, k);
    let base = json!({"feature": "Tabs", "punch": scaled(&STRAIGHT, k), "die": scaled(&DIE, k),
                      "segmented": true});
    let with = |key: &str, v: Value| {
        let mut p = base.clone();
        p[key] = v;
        p
    };
    let got = error(&mut s, "sheetMetal.bendSequence", with("maxWork", json!(5)));
    assert!(
        got.contains("more than 5") && got.contains("maxWork"),
        "{got}"
    );
    for bad in [json!(0), json!(-3), json!(1.5), json!("x"), json!(1e12)] {
        let got = error(
            &mut s,
            "sheetMetal.bendSequence",
            with("maxWork", bad.clone()),
        );
        assert!(got.contains("maxWork"), "{bad}: {got}");
    }
    let out = run(
        &mut s,
        "sheetMetal.bendSequence",
        with("maxWork", json!(1_000_000)),
    );
    assert_eq!(out["feasible"], true, "{out}");
    assert!(out["work"].as_u64().unwrap() <= 20_000, "{out}");
    // Tool lengths per flange: one for each, null for the bend's own.
    let out = run(
        &mut s,
        "sheetMetal.bendSequence",
        with("length", json!([null, 30.0, null])),
    );
    assert_eq!(out["feasible"], true, "{out}");
    for bad in [
        json!([1.0, 2.0]),
        json!([1.0, -2.0, null]),
        json!([1.0, "x", 3.0]),
        json!("long"),
    ] {
        let got = error(
            &mut s,
            "sheetMetal.bendSequence",
            with("length", bad.clone()),
        );
        assert!(got.contains("length"), "{bad}: {got}");
    }
    // A length per flange is segmented tooling: refused with segmented
    // false; `segmented` is a yes or no.
    let mut p = with("length", json!([null, 30.0, null]));
    p["segmented"] = json!(false);
    let got = error(&mut s, "sheetMetal.bendSequence", p);
    assert!(got.contains("segmented"), "{got}");
    let got = error(
        &mut s,
        "sheetMetal.bendSequence",
        with("segmented", json!("yes")),
    );
    assert!(got.contains("segmented"), "{got}");
}
