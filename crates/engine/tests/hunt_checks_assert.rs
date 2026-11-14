//! A seeded hunt of the verdicts a document gives on its checks: the
//! assertions `min`, `max` and `eq` on a clearance, a distance, a wall
//! thickness and a volume, set at and around the true value (from far off
//! down to within the tolerance, on either side); the outputs
//! (`variable.measure`) of those measures; `document.measure {bound}`
//! against closed forms; and `document.sample` (what `craft3d-cli
//! check-range` runs) over a published gap that runs from overlapping to
//! clear, each point's verdict held against the gap it was given. Every
//! case is turned about a random axis and moved far from the origin, at
//! scales 1e-3 to 1e6.
//!
//! The oracles (docs/design/verification.md › *Assertions*: "A bound holds
//! when the whole of that range is within it, fails when none of it is,
//! and is otherwise not proven"; a bound is met within the document's
//! tolerance): an assertion that passes holds of the true value within
//! twice the tolerance (its cube for a volume, plus a millionth of a
//! millionth of the bound); one that fails is false of it by more than
//! that; `notProven` and errors are allowed. An output that is known is
//! within twice the tolerance of the truth. `document.measure`'s volume
//! and area are within `volumeBound` and `areaBound`. A sampled point
//! whose gap is more than twice the tolerance clear does not fail a
//! `min 0` clearance, one more than twice the tolerance into overlap does
//! not pass it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod hunt_checks_common;

use craft3d_math::Portable;
use hunt_checks_common::*;
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 5] = [1e-3, 1.0, 1.0, 1e3, 1e6];

/// What is measured, and its true value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Measure {
    Clearance,
    Distance,
    Thickness,
    Volume,
}

/// A ball of radius r over a block's top face, its centre `r + g` above
/// a point of the face well inside it; a plate of thickness t; all moved.
struct Setup {
    k: f64,
    steps: Vec<Step>,
    gap: f64,
    plate: f64,
    volume: f64,
    motion: Motion,
    what: String,
}

fn setup(rng: &mut Rng) -> Setup {
    let k = rng.pick(&SCALES);
    let motion = Motion::random(rng, k);
    let size = V::new(
        rng.between(4.0, 12.0) * k,
        rng.between(4.0, 12.0) * k,
        rng.between(0.2, 3.0) * k,
    );
    let r = rng.between(0.5, 2.0) * k;
    let g = rng.pick(&[0.5, 0.01, 1e-4, 1e-5]) * k;
    let at = V::new(
        rng.between(0.3, 0.7) * size.x,
        rng.between(0.3, 0.7) * size.y,
        size.z + r + g,
    );
    let steps = vec![
        (
            "feature.block",
            json!({"name": "A0", "size": mm3(size), "position": mm3(V::new(0.0, 0.0, 0.0))}),
        ),
        (
            "feature.sphere",
            json!({"name": "B0", "radius": mm(r), "position": mm3(at)}),
        ),
        motion.step("A0", "A"),
        motion.step("B0", "B"),
    ];
    Setup {
        k,
        steps,
        gap: g,
        plate: size.z,
        volume: size.x * size.y * size.z,
        motion,
        what: format!(
            "ball {r:.6e} {g:.3e} over a block {:.4e} x {:.4e} x {:.4e} at scale {k:e}, {}",
            size.x,
            size.y,
            size.z,
            motion.describe()
        ),
    }
}

fn measure_json(m: Measure) -> Value {
    match m {
        Measure::Clearance => json!({"measure": "clearance", "from": "A", "to": "B"}),
        Measure::Distance => json!({"measure": "distance", "from": "A", "to": "B"}),
        Measure::Thickness => json!({"measure": "thickness", "of": "A"}),
        Measure::Volume => json!({"measure": "volume", "of": "A"}),
    }
}

/// The bound's text: a length or a volume in mm.
fn value_text(m: Measure, v: f64) -> String {
    match m {
        Measure::Volume => format!("{v:?}"),
        _ => mm(v),
    }
}

/// One assertion: its verdict against the truth, or what was wrong. An
/// `eq` is given with a `tol` of `eq_tol`.
fn assertion(
    s: &mut craft3d_engine::Session,
    m: Measure,
    truth: f64,
    bound: (&str, f64),
    eq_tol: f64,
    slack: f64,
) -> Result<String, String> {
    let mut p = measure_json(m);
    p["name"] = json!("hunted");
    p[bound.0] = json!(value_text(m, bound.1));
    if bound.0 == "eq" {
        p["tol"] = json!(value_text(m, eq_tol));
    }
    let out = match run(s, "assert.add", &p) {
        Ran::Ok(v) => v,
        Ran::Err(e) => return Ok(format!("error: {e}")),
        Ran::Panic(e) => return Err(format!("PANIC {e}")),
    };
    let _ = run(s, "assert.delete", &json!({"name": "hunted"}));
    let status = out["status"].as_str().unwrap_or("?").to_string();
    // Passed only where the bound holds of the truth (to the slack),
    // failed only where it is false of it by more than the slack.
    let (holds, false_of_it) = match bound.0 {
        "min" => (truth >= bound.1 - slack, truth < bound.1 + slack),
        "max" => (truth <= bound.1 + slack, truth > bound.1 - slack),
        _ => {
            let off = (truth - bound.1).abs();
            (off <= eq_tol + slack, off > eq_tol - slack)
        }
    };
    match status.as_str() {
        "passed" if !holds => Err(format!(
            "{m:?} {} {:.17e} passed, the true value {truth:.17e} ({out})",
            bound.0, bound.1
        )),
        "failed" if !false_of_it => Err(format!(
            "{m:?} {} {:.17e} failed, the true value {truth:.17e} ({out})",
            bound.0, bound.1
        )),
        _ => Ok(status),
    }
}

/// Assertions at and around the true value of each measure.
fn assertions_case(rng: &mut Rng, index: usize) -> (Verdict, String) {
    let su = setup(rng);
    let mut s = document(su.k);
    if let Err(e) = build(&mut s, &su.steps) {
        return (Verdict::Declined, format!("not built: {e:?}"));
    }
    let tol = 1e-6 * su.k;
    let eps = 1e-13 * (su.motion.reach() + 50.0 * su.k);
    let mut wrong = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    for m in [
        Measure::Clearance,
        Measure::Distance,
        Measure::Thickness,
        Measure::Volume,
    ] {
        let truth = match m {
            Measure::Clearance | Measure::Distance => su.gap,
            Measure::Thickness => su.plate,
            Measure::Volume => su.volume,
        };
        // The slack: the tolerance (cubed for a volume) twice, the
        // rounding of a far frame, a millionth of a millionth.
        let slack = match m {
            Measure::Volume => 2.0 * tol * tol * tol + 1e-12 * truth + 1e-15 * truth,
            _ => 2.0 * tol + eps + 1e-12 * truth,
        };
        let unit = match m {
            Measure::Volume => tol * tol * tol + 1e-12 * truth,
            _ => tol,
        };
        let offsets = [
            -1e-3 * truth,
            -10.0,
            -2.5,
            -0.5,
            0.0,
            0.5,
            2.5,
            10.0,
            1e-3 * truth,
        ];
        for (i, &o) in offsets.iter().enumerate() {
            let off = if i == 0 || i == offsets.len() - 1 {
                o
            } else {
                o * unit
            };
            for key in ["min", "max", "eq"] {
                match assertion(&mut s, m, truth, (key, truth + off), 3.0 * unit, slack) {
                    Ok(status) => {
                        *counts.entry(format!("{m:?} {key} {status}")).or_insert(0) += 1;
                    }
                    Err(e) => wrong.push(e),
                }
            }
        }
    }
    let line = format!(
        "case {index}: {}: {}",
        su.what,
        counts
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if wrong.is_empty() {
        (Verdict::Right, line)
    } else {
        (Verdict::Wrong, format!("{line}; {}", wrong.join("; ")))
    }
}

/// The outputs of the same measures: known ones within twice the
/// tolerance of the truth.
fn outputs_case(rng: &mut Rng, index: usize) -> (Verdict, String) {
    let su = setup(rng);
    let mut s = document(su.k);
    if let Err(e) = build(&mut s, &su.steps) {
        return (Verdict::Declined, format!("not built: {e:?}"));
    }
    let tol = 1e-6 * su.k;
    let eps = 1e-13 * (su.motion.reach() + 50.0 * su.k);
    let mut wrong = Vec::new();
    let mut seen = Vec::new();
    for (name, m, truth) in [
        ("oc", Measure::Clearance, su.gap),
        ("od", Measure::Distance, su.gap),
        ("ot", Measure::Thickness, su.plate),
        ("ov", Measure::Volume, su.volume),
    ] {
        let mut p = measure_json(m);
        p["name"] = json!(name);
        if let Ran::Panic(e) = run(&mut s, "variable.measure", &p) {
            wrong.push(format!("PANIC {e}"));
        }
        let _ = truth;
    }
    let listed = match run(&mut s, "output.list", &json!({})) {
        Ran::Ok(v) => v,
        other => return (Verdict::Declined, format!("output.list: {other:?}")),
    };
    let outputs = listed["outputs"].as_array().cloned().unwrap_or_default();
    for (name, truth, slack) in [
        ("oc", su.gap, 2.0 * tol + eps),
        ("od", su.gap, 2.0 * tol + eps),
        ("ot", su.plate, 2.0 * tol + eps),
        ("ov", su.volume, 2.0 * tol * tol * tol + 1e-11 * su.volume),
    ] {
        let Some(o) = outputs.iter().find(|o| o["name"] == name) else {
            wrong.push(format!("{name} not listed: {listed}"));
            continue;
        };
        let status = o["status"].as_str().unwrap_or("?");
        seen.push(format!("{name} {status}"));
        if status == "known" {
            let v = o["value"].as_f64().unwrap_or(f64::NAN);
            if (v - truth).abs() > slack || v.is_nan() {
                wrong.push(format!(
                    "output {name} {v:.17e} for the true {truth:.17e} ({o})"
                ));
            }
        }
    }
    let line = format!("case {index}: {}: {}", su.what, seen.join(", "));
    if wrong.is_empty() {
        (Verdict::Right, line)
    } else {
        (Verdict::Wrong, format!("{line}; {}", wrong.join("; ")))
    }
}

/// `document.measure {bound}` of primitives against their closed forms.
fn measure_case(rng: &mut Rng, index: usize) -> (Verdict, String) {
    let k = rng.pick(&SCALES);
    let motion = Motion::random(rng, k);
    let (step, volume, area, what) = match rng.below(5) {
        0 => {
            let s = V::new(
                rng.between(1.0, 20.0) * k,
                rng.between(1.0, 20.0) * k,
                rng.between(1.0, 20.0) * k,
            );
            (
                (
                    "feature.block",
                    json!({"name": "A0", "size": mm3(s), "position": mm3(V::new(0.0, 0.0, 0.0))}),
                ),
                s.x * s.y * s.z,
                2.0 * (s.x * s.y + s.y * s.z + s.z * s.x),
                format!("block {:.6e} x {:.6e} x {:.6e}", s.x, s.y, s.z),
            )
        }
        1 => {
            let (r, h) = (rng.between(0.5, 10.0) * k, rng.between(0.5, 20.0) * k);
            (
                (
                    "feature.cylinder",
                    json!({"name": "A0", "radius": mm(r), "height": mm(h)}),
                ),
                PI * r * r * h,
                2.0 * PI * r * (r + h),
                format!("cylinder {r:.6e}, {h:.6e}"),
            )
        }
        2 => {
            let (r, h) = (rng.between(0.5, 10.0) * k, rng.between(0.5, 20.0) * k);
            (
                (
                    "feature.cone",
                    json!({"name": "A0", "radius": mm(r), "height": mm(h)}),
                ),
                PI * r * r * h / 3.0,
                PI * r * (r + r.phypot(h)),
                format!("cone {r:.6e}, {h:.6e}"),
            )
        }
        3 => {
            let r = rng.between(0.5, 10.0) * k;
            (
                ("feature.sphere", json!({"name": "A0", "radius": mm(r)})),
                4.0 * PI * r * r * r / 3.0,
                4.0 * PI * r * r,
                format!("ball {r:.6e}"),
            )
        }
        _ => {
            let big = rng.between(1.0, 10.0) * k;
            let small = rng.between(0.05, 0.95) * big;
            (
                (
                    "feature.torus",
                    json!({"name": "A0", "major": mm(big), "minor": mm(small)}),
                ),
                2.0 * PI * PI * big * small * small,
                4.0 * PI * PI * big * small,
                format!("ring {big:.6e}, {small:.6e}"),
            )
        }
    };
    let mut s = document(k);
    if let Err(e) = build(&mut s, &[step, motion.step("A0", "A")]) {
        return (Verdict::Declined, format!("not built: {e:?}"));
    }
    let out = match run(&mut s, "document.measure", &json!({"bound": true})) {
        Ran::Ok(v) => v,
        Ran::Err(e) => return (Verdict::Declined, format!("measure: {e}")),
        Ran::Panic(e) => return (Verdict::Panic, format!("measure: {e}")),
    };
    let (v, a, bv, ba) = (
        out["volume"].as_f64().unwrap_or(f64::NAN),
        out["area"].as_f64().unwrap_or(f64::NAN),
        out["volumeBound"].as_f64().unwrap_or(f64::NAN),
        out["areaBound"].as_f64().unwrap_or(f64::NAN),
    );
    let reach = motion.reach();
    let size = (volume / area) * 6.0;
    let slack_v = 4e-16 * area * (reach + size) + 1e-15 * volume;
    let slack_a = 4e-16 * area * (reach + size) / size + 1e-15 * area;
    let mut wrong = Vec::new();
    if (v - volume).abs() > bv + slack_v || v.is_nan() || bv.is_nan() {
        wrong.push(format!(
            "volume {v:.17e} for {volume:.17e}: off by {:.3e}, bound {bv:.3e}",
            (v - volume).abs()
        ));
    }
    if (a - area).abs() > ba + slack_a || a.is_nan() || ba.is_nan() {
        wrong.push(format!(
            "area {a:.17e} for {area:.17e}: off by {:.3e}, bound {ba:.3e}",
            (a - area).abs()
        ));
    }
    let line = format!("case {index}: {what} at scale {k:e}, {}", motion.describe());
    if wrong.is_empty() {
        (Verdict::Right, line)
    } else {
        (Verdict::Wrong, format!("{line}: {}", wrong.join("; ")))
    }
}

/// `document.sample` over a published gap from overlapping to clear: each
/// point's verdict on `min 0` of the clearance, and on `max` of the
/// distance at a tenth of the scale, against the gap it was given.
fn sample_case(rng: &mut Rng, index: usize) -> (Verdict, String) {
    let k = rng.pick(&SCALES);
    let motion = Motion::random(rng, k);
    let tol = 1e-6 * k;
    let size = V::new(
        rng.between(4.0, 12.0) * k,
        rng.between(4.0, 12.0) * k,
        rng.between(0.5, 3.0) * k,
    );
    let r = rng.between(0.5, 2.0) * k;
    let at = V::new(0.5 * size.x, 0.5 * size.y, size.z + r);
    let (lo, hi) = (-0.2 * k, 0.2 * k);
    let limit = 0.1 * k;
    let mut steps: Vec<Step> = vec![
        ("variable.set", json!({"name": "gap", "expr": mm(0.0)})),
        (
            "variable.publish",
            json!({"name": "gap", "min": mm(lo), "max": mm(hi), "unit": "mm"}),
        ),
        (
            "feature.block",
            json!({"name": "A0", "size": mm3(size), "position": mm3(V::new(0.0, 0.0, 0.0))}),
        ),
        (
            "feature.sphere",
            json!({"name": "B0", "radius": mm(r), "position": [mm(at.x), mm(at.y), format!("{} + gap", mm(at.z))]}),
        ),
    ];
    steps.push(motion.step("A0", "A"));
    steps.push(motion.step("B0", "B"));
    steps.push((
        "assert.add",
        json!({"name": "apart", "measure": "clearance", "from": "A", "to": "B", "min": "0 mm"}),
    ));
    steps.push((
        "assert.add",
        json!({"name": "close", "measure": "distance", "from": "A", "to": "B", "max": mm(limit)}),
    ));
    let mut s = document(k);
    if let Err(e) = build(&mut s, &steps) {
        return (Verdict::Declined, format!("not built: {e:?}"));
    }
    let told = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let into = std::rc::Rc::clone(&told);
    let samples = 24;
    let result = craft3d_engine::with_point_sink(
        move |point| {
            into.borrow_mut().push(point.clone());
            true
        },
        || {
            run(
                &mut s,
                "document.sample",
                &json!({"samples": samples, "seed": index as u64 + 1, "timeLimitMs": null}),
            )
        },
    );
    if let Ran::Panic(e) = &result {
        return (Verdict::Panic, format!("sample: {e}"));
    }
    if let Ran::Err(e) = &result {
        return (Verdict::Declined, format!("sample: {e}"));
    }
    let mut wrong = Vec::new();
    let mut seen = (0, 0, 0);
    for point in told.borrow().iter() {
        let text = point["values"]["gap"].as_str().unwrap_or("");
        let Some(g) = text.trim_end_matches("mm").trim().parse::<f64>().ok() else {
            wrong.push(format!("no gap in {point}"));
            continue;
        };
        let names = |key: &str| -> Vec<String> {
            point[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x["assertion"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let failed = names("problems");
        let unproven = names("unproven");
        let passed = |n: &str| !failed.iter().any(|f| f == n) && !unproven.iter().any(|f| f == n);
        // The distance between the faces: the gap when clear, 0 when
        // the ball sinks into the top (it never sinks past it).
        let faces = g.max(0.0);
        if g > 2.0 * tol && failed.iter().any(|f| f == "apart") {
            wrong.push(format!(
                "gap {g:e}: `apart` (clearance min 0) failed ({point})"
            ));
        }
        if g < -2.0 * tol && passed("apart") {
            wrong.push(format!(
                "gap {g:e}: `apart` (clearance min 0) passed ({point})"
            ));
        }
        if faces > limit + 2.0 * tol && passed("close") {
            wrong.push(format!(
                "gap {g:e}: `close` (distance max {limit:e}) passed ({point})"
            ));
        }
        if faces < limit - 2.0 * tol && failed.iter().any(|f| f == "close") {
            wrong.push(format!(
                "gap {g:e}: `close` (distance max {limit:e}) failed ({point})"
            ));
        }
        if point["ok"] == true {
            seen.0 += 1;
        } else {
            seen.1 += 1;
        }
        seen.2 += unproven.len();
    }
    let line = format!(
        "case {index}: ball {r:.4e} over a block, gap {lo:e} to {hi:e} at scale {k:e}, {}: {} points ok, {} failed, {} unproven assertions",
        motion.describe(),
        seen.0,
        seen.1,
        seen.2
    );
    if wrong.is_empty() {
        (Verdict::Right, line)
    } else {
        (Verdict::Wrong, format!("{line}; {}", wrong.join("; ")))
    }
}

type CaseFn = fn(&mut Rng, usize) -> (Verdict, String);

fn sweep(name: &str, f: CaseFn, seed: u64, count: usize) -> Tally {
    let mut rng = Rng(seed);
    let mut tally = Tally::default();
    for index in 0..count {
        let (verdict, why) = f(&mut rng, index);
        if verdict != Verdict::Right || std::env::var("HUNT_VERBOSE").is_ok() {
            println!("{} {name} {index}: {why}", verdict.letter());
        }
        tally.add(&format!("{name} {index}"), verdict, why);
    }
    tally
}

/// Wrong cases of the gate's sweep, by part and index, with what is wrong.
const KNOWN: [(&str, usize, &str); 0] = [];

const GATE_SEED: u64 = 0x5EED_A55E_0001;

fn hold(name: &str, f: CaseFn, count: usize) {
    let tally = sweep(name, f, GATE_SEED, count);
    tally.print(name);
    for (id, letter, what) in &tally.notes {
        let index: usize = id.rsplit(' ').next().unwrap().parse().unwrap();
        let known = KNOWN.iter().any(|(n, i, _)| *n == name && *i == index);
        assert!(
            *letter == 'D' || (known && *letter == 'W'),
            "{id} {letter}: {what}"
        );
    }
    for (n, i, _) in KNOWN.iter().filter(|(n, ..)| *n == name) {
        let id = format!("{n} {i}");
        assert!(
            tally.notes.iter().any(|(m, l, _)| *m == id && *l == 'W'),
            "{id} is listed as known wrong but is not: take it off KNOWN"
        );
    }
}

#[test]
fn assertions_around_the_true_value() {
    hold("assertions", assertions_case, 4);
}

#[test]
fn outputs_of_measures() {
    hold("outputs", outputs_case, 6);
}

#[test]
fn the_measure_within_its_bound() {
    hold("measure", measure_case, 20);
}

#[test]
fn sampling_over_a_gap() {
    hold("sample", sample_case, 2);
}

fn wide(name: &str, f: CaseFn, default: usize) {
    let count: usize = std::env::var("HUNT_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    let seed: u64 = std::env::var("HUNT_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5EED_A55E_1000);
    println!("{}", sweep(name, f, seed, count).summary(name));
}

#[test]
#[ignore]
fn wide_assertions() {
    wide("assertions", assertions_case, 40);
}

#[test]
#[ignore]
fn wide_outputs() {
    wide("outputs", outputs_case, 60);
}

#[test]
#[ignore]
fn wide_measure() {
    wide("measure", measure_case, 300);
}

#[test]
#[ignore]
fn wide_sampling() {
    wide("sample", sample_case, 16);
}
