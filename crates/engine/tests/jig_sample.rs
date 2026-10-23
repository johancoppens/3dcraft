//! Proof over the range (`document.sample`, Jigs phase 4;
//! docs/design/verification.md › *Proof over the range*). The oracles:
//! the corners of a box of n ranges are its 2^n corners, each value at
//! its min or max; a range that uses another parameter holds at every
//! point; a point known to break the model (a block of height
//! `8 mm - t` for `t` up to 10 mm) is reported, with values that break it
//! again through param.set; the same seed gives the same points; and the
//! open document, its revision and its undo history do not change.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{refused, session};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// A plate `w` × `d` × `h` with `w` at most twice `d`; a pin of height
/// `8 mm - t` that fails for `t` of 8 mm and more; an integer, a boolean
/// and a choice that drive nothing.
fn jig(s: &mut Session, t_max: &str) {
    run(s, "file.new", json!({"units": "mm"}));
    for (name, expr) in [
        ("w", "40 mm"),
        ("d", "20 mm"),
        ("h", "5 mm"),
        ("t", "2 mm"),
        ("n", "3"),
        ("lid", "1"),
        ("style", "2"),
    ] {
        run(s, "variable.set", json!({"name": name, "expr": expr}));
    }
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": ["w", "d", "h"]}),
    );
    run(
        s,
        "feature.block",
        json!({"name": "Pin", "size": ["4 mm", "4 mm", "8 mm - t"], "position": [100, 0, 0]}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "w", "min": "10 mm", "max": "2 * d"}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "d", "min": "5 mm", "max": "30 mm"}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "t", "min": "1 mm", "max": t_max}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "n", "kind": "integer", "min": 1, "max": 6}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "lid", "kind": "boolean"}),
    );
    run(
        s,
        "variable.publish",
        json!({"name": "style", "kind": "choice", "choices": ["Plain", "Slotted", "Round"]}),
    );
}

/// A number of a value `param.set` takes: `12.5 mm` or `3`.
fn number(text: &Value) -> f64 {
    text.as_str()
        .unwrap()
        .trim_end_matches(" mm")
        .parse()
        .unwrap_or_else(|_| panic!("not a value in mm: {text}"))
}

fn sample(s: &mut Session, params: Value) -> Value {
    run(s, "document.sample", params)
}

#[test]
fn a_jig_that_holds_over_its_range_passes_every_point() {
    let mut s = session();
    jig(&mut s, "6 mm");
    let result = sample(&mut s, json!({"samples": 20, "seed": 5, "values": true}));
    println!(
        "{}",
        serde_json::to_string_pretty(&result["counts"]).unwrap()
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["failed"], 0);
    // Six parameters move: 64 corners, all of them.
    assert_eq!(result["counts"]["corner"], 64);
    assert_eq!(result["counts"]["allCorners"], true);
    // The two other choices of `style`.
    assert_eq!(result["counts"]["choice"], 2);
    assert_eq!(result["counts"]["random"], 20);
    assert_eq!(result["counts"]["points"], 1 + 64 + 2 + 20);
    assert_eq!(result["passed"], 87);
    let points = result["points"].as_array().unwrap();
    assert_eq!(points.len(), 87);
    assert!(points[0]["ms"].as_f64().unwrap() >= 0.0);
    assert!(result["timing"]["maxMs"].as_f64().unwrap() >= 0.0);
    // The defaults as published.
    let defaults = &points[0]["values"];
    assert_eq!(points[0]["kind"], "default");
    assert_eq!(defaults["w"], "40 mm");
    assert_eq!(defaults["style"], "2");
    for point in points {
        let v = &point["values"];
        let (w, d, t) = (number(&v["w"]), number(&v["d"]), number(&v["t"]));
        // The range of `w` uses `d`: it holds at every point.
        assert!((10.0..=2.0 * d * (1.0 + 1e-12)).contains(&w), "{point}");
        assert!((5.0..=30.0).contains(&d), "{point}");
        assert!((1.0..=6.0).contains(&t), "{point}");
        let n: i64 = v["n"].as_str().unwrap().parse().unwrap();
        assert!((1..=6).contains(&n), "{point}");
        assert!(["0", "1"].contains(&v["lid"].as_str().unwrap()), "{point}");
        assert!(
            ["1", "2", "3"].contains(&v["style"].as_str().unwrap()),
            "{point}"
        );
    }
    // Every corner has each value at an end of its range at that point.
    for corner in points.iter().filter(|p| p["kind"] == "corner") {
        let v = &corner["values"];
        let (w, d) = (number(&v["w"]), number(&v["d"]));
        assert!(d == 5.0 || d == 30.0, "{corner}");
        assert!(w == 10.0 || w == 2.0 * d, "{corner}");
        assert!(["1 mm", "6 mm"].contains(&v["t"].as_str().unwrap()));
        assert!(["1", "6"].contains(&v["n"].as_str().unwrap()));
        assert!(["1", "3"].contains(&v["style"].as_str().unwrap()));
    }
    // 64 corners, 48 of them distinct: where `d` is at its min, 5 mm,
    // the range of `w` is 10 mm to 2 * d = 10 mm, both ends one value.
    let mut seen: Vec<String> = points
        .iter()
        .filter(|p| p["kind"] == "corner")
        .map(|p| p["values"].to_string())
        .collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 48);
}

#[test]
fn the_same_seed_gives_the_same_points_and_another_seed_others() {
    let mut s = session();
    jig(&mut s, "10 mm");
    let values = |r: &Value| -> Vec<Value> {
        r["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| json!([p["kind"], p["ok"], p["values"]]))
            .collect()
    };
    let params = json!({"samples": 12, "seed": 42, "corners": false, "values": true});
    let a = sample(&mut s, params.clone());
    let b = sample(&mut s, params);
    assert_eq!(values(&a), values(&b));
    let failures = |r: &Value| -> Vec<Value> {
        r["failures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| json!([f["index"], f["values"], f["problems"]]))
            .collect()
    };
    assert_eq!(failures(&a), failures(&b));
    let c = sample(
        &mut s,
        json!({"samples": 12, "seed": 43, "corners": false, "values": true}),
    );
    assert_ne!(values(&a), values(&c));
    // Random values are spread over the range, not stuck at its ends.
    let ts: Vec<f64> = a["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"] == "random")
        .map(|p| number(&p["values"]["t"]))
        .collect();
    assert!(ts.iter().any(|t| *t > 1.0 && *t < 10.0), "{ts:?}");
}

#[test]
fn a_point_that_breaks_the_model_is_reported_and_breaks_it_again() {
    let mut s = session();
    jig(&mut s, "10 mm");
    let result = sample(&mut s, json!({"samples": 16, "seed": 1}));
    assert_eq!(result["ok"], false);
    let failed = result["failed"].as_u64().unwrap();
    // Half the corners have t at 10 mm: a pin of height -2 mm.
    assert!(failed >= 32, "{failed}");
    let failures = result["failures"].as_array().unwrap();
    assert_eq!(failures.len() as u64, failed.min(100));
    for failure in failures {
        let t = number(&failure["values"]["t"]);
        assert!(t >= 8.0, "{failure}");
        let problems = failure["problems"].to_string();
        assert!(problems.contains("Pin"), "{problems}");
    }
    // The defaults pass, and no passing point has t of 8 mm or more.
    assert_eq!(result["points"][0]["ok"], true);
    // A failing point set on the document fails the pin there too.
    let values = failures[0]["values"].clone();
    let set = run(&mut s, "param.set", json!({"values": values}));
    assert!(set["failed"].to_string().contains("Pin"), "{set}");
    // At most maxFailures are listed; the count is all of them.
    let few = sample(&mut s, json!({"samples": 16, "seed": 1, "maxFailures": 3}));
    assert_eq!(few["failures"].as_array().unwrap().len(), 3);
    assert_eq!(few["truncated"], true);
}

#[test]
fn the_document_and_its_undo_history_do_not_change() {
    let mut s = session();
    jig(&mut s, "10 mm");
    run(&mut s, "param.set", json!({"values": {"w": "30 mm"}}));
    let before = run(&mut s, "document.inspect", json!({"measures": false}));
    let revision = s.document_revision();
    sample(&mut s, json!({"samples": 8, "deep": true}));
    assert_eq!(s.document_revision(), revision);
    let after = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(before["undo"], after["undo"]);
    assert_eq!(before["redo"], after["redo"]);
    assert_eq!(before["variables"], after["variables"]);
    // The last undo step is still the param.set.
    let undone = run(&mut s, "edit.undo", json!({}));
    assert!(undone.to_string().contains('w'), "{undone}");
}

#[test]
fn a_budget_fails_points_that_take_longer() {
    let mut s = session();
    jig(&mut s, "6 mm");
    let result = sample(
        &mut s,
        json!({"samples": 2, "corners": false, "choices": false, "budgetMs": 1e-9}),
    );
    assert_eq!(result["ok"], false);
    assert_eq!(result["failed"], 3);
    assert!(result["failures"].to_string().contains("budget"));
}

#[test]
fn many_parameters_take_a_seeded_subset_of_their_corners() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    for k in 0..10 {
        let name = format!("p{k}");
        run(
            &mut s,
            "variable.set",
            json!({"name": name, "expr": "5 mm"}),
        );
        run(
            &mut s,
            "variable.publish",
            json!({"name": name, "min": "1 mm", "max": "9 mm"}),
        );
    }
    run(
        &mut s,
        "feature.block",
        json!({"size": ["p0 + p1 + p2", "p3 + p4 + p5", "p6 + p7 + p8 + p9"]}),
    );
    let result = sample(
        &mut s,
        json!({"samples": 0, "maxCorners": 16, "values": true}),
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["counts"]["corner"], 16);
    assert_eq!(result["counts"]["allCorners"], false);
    let corners: Vec<&Value> = result["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"] == "corner")
        .collect();
    // All low and all high first.
    for k in 0..10 {
        assert_eq!(corners[0]["values"][format!("p{k}")], "1 mm");
        assert_eq!(corners[1]["values"][format!("p{k}")], "9 mm");
    }
    // Times differ from run to run; the points do not.
    let without_times = |r: &Value| -> Vec<Value> {
        r["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| json!([p["kind"], p["ok"], p["values"]]))
            .collect()
    };
    let again = sample(
        &mut s,
        json!({"samples": 0, "maxCorners": 16, "values": true}),
    );
    assert_eq!(without_times(&result), without_times(&again));
}

#[test]
fn bad_params_and_a_document_without_parameters_are_refused() {
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    let error = refused(&mut s, "document.sample", json!({}));
    assert!(error.contains("no public parameters"), "{error}");
    jig(&mut s, "6 mm");
    for (params, word) in [
        (json!({"samples": 10_001}), "samples"),
        (json!({"samples": -1}), "samples"),
        (json!({"maxCorners": 5000}), "maxCorners"),
        (json!({"budgetMs": 0}), "budgetMs"),
        (json!({"budgetMs": "fast"}), "budgetMs"),
        (json!({"seed": 1.5}), "seed"),
        (json!({"timeLimitMs": 0}), "timeLimitMs"),
        (json!({"timeLimitMs": "long"}), "timeLimitMs"),
        (json!({"deep": "yes"}), "deep"),
        (json!({"nonsense": 1}), "nonsense"),
    ] {
        let error = refused(&mut s, "document.sample", params.clone());
        assert!(error.contains(word), "{params}: {error}");
    }
}

/// The kinds and values of a run's points of `kind`.
fn values_of(result: &Value, kind: &str) -> Vec<Value> {
    result["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"] == kind)
        .map(|p| p["values"].clone())
        .collect()
}

#[test]
fn the_random_points_do_not_depend_on_the_corners_asked_for() {
    let mut s = session();
    jig(&mut s, "6 mm");
    let with = sample(&mut s, json!({"samples": 6, "seed": 9, "values": true}));
    let without = sample(
        &mut s,
        json!({"samples": 6, "seed": 9, "corners": false, "values": true}),
    );
    let fewer = sample(
        &mut s,
        json!({"samples": 6, "seed": 9, "maxCorners": 8, "values": true}),
    );
    assert_eq!(values_of(&with, "random").len(), 6);
    assert_eq!(values_of(&with, "random"), values_of(&without, "random"));
    assert_eq!(values_of(&with, "random"), values_of(&fewer, "random"));
    assert_eq!(fewer["counts"]["corner"], 8);
    assert_eq!(fewer["counts"]["allCorners"], false);
}

#[test]
fn a_range_that_ends_at_zero_is_stepped_to_zero_not_past_it() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "off", "expr": "-0.1 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": ["5 mm + off", "5 mm", "5 mm"]}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "off", "min": "-0.3 mm", "max": "0 mm", "step": "0.1 mm"}),
    );
    let result = sample(&mut s, json!({"samples": 40, "seed": 3, "values": true}));
    assert_eq!(result["ok"], true, "{result}");
    let offs: Vec<f64> = result["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| number(&p["values"]["off"]))
        .collect();
    assert!(offs.iter().all(|v| (-0.3..=0.0).contains(v)), "{offs:?}");
    // The high corner is exactly the end of the range.
    assert!(offs.contains(&0.0) && offs.contains(&-0.3), "{offs:?}");
}

#[test]
fn ranges_that_push_each_other_on_fail_the_point_with_the_last_values() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "variable.set", json!({"name": "a", "expr": "5 mm"}));
    run(&mut s, "variable.set", json!({"name": "b", "expr": "5 mm"}));
    run(&mut s, "feature.block", json!({"size": ["a", "b", "1 mm"]}));
    run(
        &mut s,
        "variable.publish",
        json!({"name": "a", "min": "1 mm", "max": "b"}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "b", "min": "1 mm", "max": "a + 1 mm"}),
    );
    let result = sample(&mut s, json!({"samples": 0}));
    assert_eq!(result["ok"], false, "{result}");
    let unsettled: Vec<&Value> = result["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| {
            f["problems"]
                .to_string()
                .contains("did not settle after 3 rounds")
        })
        .collect();
    // Both at their high end: a = b, b = a + 1, on and on.
    assert!(!unsettled.is_empty(), "{result}");
    assert!(unsettled[0]["values"]["a"].is_string(), "{result}");
}

#[test]
fn a_time_limit_stops_the_run_without_calling_it_ok() {
    let mut s = session();
    jig(&mut s, "10 mm");
    let stopped = sample(&mut s, json!({"samples": 50, "timeLimitMs": 1e-9}));
    assert_eq!(stopped["complete"], false);
    assert_eq!(stopped["stoppedBy"], "timeLimit");
    assert_eq!(stopped["ok"], Value::Null);
    // The first point is always checked.
    assert_eq!(stopped["counts"]["points"], 1);
    // A failure found before the limit makes it false, not null: the
    // defaults themselves fail (a pin of height -1 mm).
    run(
        &mut s,
        "variable.publish",
        json!({"name": "t", "default": "9 mm"}),
    );
    let failing = sample(&mut s, json!({"samples": 50, "timeLimitMs": 1e-9}));
    assert_eq!(failing["ok"], false, "{failing}");
    assert_eq!(failing["complete"], false);
    // null is no limit; the default is a minute.
    run(
        &mut s,
        "variable.publish",
        json!({"name": "t", "default": "2 mm"}),
    );
    let unlimited = sample(&mut s, json!({"samples": 2, "timeLimitMs": null}));
    assert_eq!(unlimited["complete"], true);
    assert_eq!(unlimited["timeLimitMs"], Value::Null);
    let default = sample(&mut s, json!({"samples": 2}));
    assert_eq!(default["timeLimitMs"], 60000.0);
    assert_eq!(default["complete"], true);
}
