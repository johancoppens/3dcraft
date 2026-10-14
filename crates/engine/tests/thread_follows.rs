//! A thread that follows its shank: the size found from the face's
//! diameter (or given as a length), the pitch the coarse one of that size
//! unless given, both looked up at every regeneration.
//!
//! The parametric M10 socket head cap screw of the morning (its head,
//! socket, rounds and lead chamfer made from a variable `d`) is driven
//! through 6, 8, 10 and 12 mm with a cosmetic thread; the cut thread,
//! which is slow to regenerate, is cut twice on a plain shank driven by
//! `d`, against the closed form of `thread.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::path::PathBuf;

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("thread-follows-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    let result = session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    assert_ne!(
        session.regeneration_matches_a_full_one(),
        Some(false),
        "after {id} {params}"
    );
    result
}

fn refused(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(result) => panic!("{id} {params} should have been refused: {result}"),
        Err(error) => error.to_string(),
    }
}

fn inspect(s: &mut Session) -> Value {
    run(s, "document.inspect", json!({}))
}

fn feature(s: &mut Session, name: &str) -> Value {
    inspect(s)["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
        .clone()
}

fn volume(s: &mut Session, name: &str) -> f64 {
    let document = inspect(s);
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name}: {}", document["features"]))["volume"]
        .as_f64()
        .unwrap()
}

/// The thread as it was built; fails the test when it was not.
fn report(s: &mut Session) -> Value {
    let state = feature(s, "Thread");
    assert_eq!(state["status"], "ok", "{state}");
    state["thread"].clone()
}

fn failure(s: &mut Session, name: &str) -> String {
    let state = feature(s, name);
    assert_eq!(state["status"], "failed", "{state}");
    state["error"].as_str().unwrap().to_string()
}

fn set_d(s: &mut Session, mm: f64) {
    run(
        s,
        "variable.set",
        json!({"name": "d", "expr": format!("{mm} mm")}),
    );
}

/// The basic minor diameter of ISO 68-1: `D − 5H/4`, `H = √3/2 · P`.
fn minor(d: f64, p: f64) -> f64 {
    d - 5.0 / 4.0 * 3.0_f64.sqrt() / 2.0 * p
}

/// The coarse pitches of ISO 261 for the sizes the screw goes through.
const SIZES: [(f64, f64); 4] = [(6.0, 1.0), (8.0, 1.25), (10.0, 1.5), (12.0, 1.75)];

/// The parametric socket head cap screw (`d` the nominal diameter, `L`
/// the length under the head), its thread given without a size: the
/// size of the shank. Every command and parameter as the morning's
/// screw had them, but the thread's size; the thread is cosmetic, the
/// default.
fn screw(s: &mut Session) {
    let steps = json!([
        ["variable.set", {"name": "d", "expr": "10 mm"}],
        ["variable.set", {"name": "L", "expr": "40 mm"}],
        ["variable.set", {"name": "k", "expr": "d"}],
        ["variable.set", {"name": "dk", "expr": "1.5 * d"}],
        ["variable.set", {"name": "s", "expr": "0.8 * d"}],
        ["variable.set", {"name": "t", "expr": "0.5 * d"}],
        ["feature.cylinder", {"radius": "dk / 2", "height": "k", "name": "Head"}],
        ["feature.cylinder", {"radius": "d / 2", "height": "L",
            "position": ["0 mm", "0 mm", "-L"], "name": "Shank"}],
        ["feature.boolean", {"op": "union", "target": "Head", "tool": "Shank", "name": "Blank"}],
        ["feature.block", {"size": ["s", "3 * s", "t + 1 mm"],
            "position": ["-s / 2", "-1.5 * s", "k - t"], "name": "Flat0"}],
        ["feature.transform", {"source": "Flat0", "angle": "60 deg", "axis": [0, 0, 1],
            "keep": true, "name": "Flat1"}],
        ["feature.transform", {"source": "Flat0", "angle": "120 deg", "axis": [0, 0, 1],
            "keep": true, "name": "Flat2"}],
        ["feature.boolean", {"op": "intersect", "target": "Flat0", "tool": "Flat1", "name": "Hex01"}],
        ["feature.boolean", {"op": "intersect", "target": "Hex01", "tool": "Flat2", "name": "Hex"}],
        ["feature.boolean", {"op": "subtract", "target": "Blank", "tool": "Hex", "name": "Socket"}],
        ["feature.fillet", {"edges": [["F1:face(1)", "F2:face(0)"]], "radius": "0.04 * d",
            "name": "UnderHead"}],
        ["feature.fillet", {"edges": [["F1:face(0)", "F1:face(2)"]], "radius": "0.08 * d",
            "name": "HeadTop"}],
        ["feature.chamfer", {"edges": [["F2:face(0)", "F2:face(1)"]], "distance": "0.1 * d",
            "name": "Lead"}],
        ["feature.thread", {"face": "F2:face(0)", "start": "F2:face(1)", "length": "0.6 * L",
            "name": "Thread"}],
    ]);
    for step in steps.as_array().unwrap() {
        run(s, step[0].as_str().unwrap(), step[1].clone());
    }
}

/// Every feature of the document built.
fn all_ok(s: &mut Session) {
    for f in inspect(s)["features"].as_array().unwrap() {
        assert_eq!(f["status"], "ok", "{f}");
    }
}

#[test]
fn the_screw_s_thread_follows_d() {
    let dir = workdir("screw");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({}));
    screw(&mut s);
    all_ok(&mut s);
    // No size and no pitch in the feature: both are found.
    let params = feature(&mut s, "Thread")["params"].clone();
    assert!(params.get("size").is_none_or(Value::is_null), "{params}");
    assert!(params.get("pitch").is_none_or(Value::is_null), "{params}");

    // d through the sizes: the thread follows, size and coarse pitch.
    for (d, p) in SIZES {
        set_d(&mut s, d);
        all_ok(&mut s);
        let thread = report(&mut s);
        assert_eq!(thread["size"], format!("M{d}"), "{thread}");
        assert_eq!(thread["pitch"].as_f64().unwrap(), p, "{thread}");
        assert_eq!(thread["pitchGiven"], false, "{thread}");
        assert_eq!(thread["major"].as_f64().unwrap(), d, "{thread}");
        let found = thread["minor"].as_f64().unwrap();
        assert!(
            (found - minor(d, p)).abs() < 1e-12,
            "{found} {}",
            minor(d, p)
        );
        assert_eq!(thread["length"].as_f64().unwrap(), 24.0, "{thread}");
    }
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");

    // A diameter no size of ISO 261 has: the thread fails, naming the
    // diameter and the sizes on either side; the rest of the screw is
    // built. Back to a size, it is built again.
    set_d(&mut s, 9.0);
    let error = failure(&mut s, "Thread");
    for says in ["9 mm across", "M8 (8 mm)", "M10 (10 mm)", "size"] {
        assert!(error.contains(says), "{says}: {error}");
    }
    assert_eq!(feature(&mut s, "Lead")["status"], "ok");
    set_d(&mut s, 10.0);
    assert_eq!(report(&mut s)["size"], "M10");

    // A pitch given stays across a change of the size; taken away, the
    // pitch follows the size again.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Thread", "pitch": "1 mm"}),
    );
    set_d(&mut s, 8.0);
    let thread = report(&mut s);
    assert_eq!(
        (thread["size"].clone(), thread["pitch"].as_f64().unwrap()),
        (json!("M8"), 1.0)
    );
    assert_eq!(thread["pitchGiven"], true);
    assert!((thread["minor"].as_f64().unwrap() - minor(8.0, 1.0)).abs() < 1e-12);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Thread", "pitch": null}),
    );
    let thread = report(&mut s);
    assert_eq!(thread["pitch"].as_f64().unwrap(), 1.25, "{thread}");
    assert_eq!(thread["pitchGiven"], false);

    // The size as a designation: M8 has its own coarse pitch, not the
    // one of the size the thread had before (the defect: 1.5 stayed).
    set_d(&mut s, 10.0);
    assert_eq!(report(&mut s)["pitch"].as_f64().unwrap(), 1.5);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Thread", "size": "M8"}),
    );
    // Not the shank's size: fails with the diameter, until d follows.
    let error = failure(&mut s, "Thread");
    assert!(
        error.contains("10 mm across") && error.contains("M8"),
        "{error}"
    );
    set_d(&mut s, 8.0);
    let thread = report(&mut s);
    assert_eq!(thread["size"], "M8");
    assert_eq!(thread["pitch"].as_f64().unwrap(), 1.25, "{thread}");
    // A designation stays when d moves on: M8 on a shank of 12 fails.
    set_d(&mut s, 12.0);
    assert!(failure(&mut s, "Thread").contains("12 mm across"));
    // Undo goes back through d and the size.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(report(&mut s)["size"], "M8");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(report(&mut s)["size"], "M10");
    assert_eq!(
        feature(&mut s, "Thread")["params"]["size"],
        Value::Null,
        "the size of the face again"
    );
    run(&mut s, "edit.redo", json!({}));
    assert!(failure(&mut s, "Thread").contains("M8"));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(report(&mut s)["pitch"].as_f64().unwrap(), 1.25);

    // The size as a length: `d` follows like the face's size, and a
    // diameter of no size of the table is threaded with a pitch given.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Thread", "size": "d"}),
    );
    for (d, p) in SIZES {
        set_d(&mut s, d);
        let thread = report(&mut s);
        assert_eq!(thread["size"], format!("M{d}"), "{thread}");
        assert_eq!(thread["pitch"].as_f64().unwrap(), p, "{thread}");
    }
    set_d(&mut s, 9.0);
    let error = failure(&mut s, "Thread");
    assert!(error.contains("M9 has no coarse pitch"), "{error}");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Thread", "pitch": "1.25 mm"}),
    );
    let thread = report(&mut s);
    assert_eq!(thread["size"], "M9");
    assert_eq!(thread["pitch"].as_f64().unwrap(), 1.25);
    // Back to the size of the face, with the pitch still given.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Thread", "size": null, "pitch": null}),
    );
    assert!(failure(&mut s, "Thread").contains("9 mm across"));
    set_d(&mut s, 12.0);
    all_ok(&mut s);

    // Saved and opened again: the same parameters and the same thread,
    // and it still follows.
    let before = feature(&mut s, "Thread");
    run(&mut s, "file.save", json!({"path": "screw.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "screw.3dcraft"}));
    assert_eq!(feature(&mut s, "Thread"), before);
    set_d(&mut s, 6.0);
    let thread = report(&mut s);
    assert_eq!(thread["size"], "M6");
    assert_eq!(thread["pitch"].as_f64().unwrap(), 1.0);
}

#[test]
fn what_a_thread_s_size_and_pitch_cannot_be() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    set_d(&mut s, 10.0);
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "d / 2", "height": "20 mm", "name": "Shank"}),
    );
    run(
        &mut s,
        "feature.thread",
        json!({"face": "F1:face(0)", "start": "F1:face(1)", "length": "12 mm",
            "cosmetic": true, "name": "Thread"}),
    );
    assert_eq!(report(&mut s)["size"], "M10");
    // Refused by feature.edit, and nothing changes.
    for (params, says) in [
        (json!({"size": "M10x1.25"}), "ISO metric"),
        (json!({"size": 10}), "size"),
        (json!({"size": true}), "size"),
        (json!({"pitch": true}), "pitch"),
        (json!({"pitch": [1]}), "pitch"),
    ] {
        let mut all = json!({"feature": "Thread"});
        for (key, value) in params.as_object().unwrap() {
            all[key] = value.clone();
        }
        let error = refused(&mut s, "feature.edit", all.clone());
        assert!(error.contains(says), "{all}: {error}");
    }
    assert_eq!(report(&mut s)["size"], "M10");
    // Null is for a thread's size and pitch only.
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "1 mm", "height": "2 mm", "name": "Other"}),
    );
    let error = refused(
        &mut s,
        "feature.edit",
        json!({"feature": "Other", "pitch": null}),
    );
    assert!(error.contains("give at least one parameter"), "{error}");
    // Failed by regeneration, with the parameter.
    for (params, says) in [
        (json!({"size": "0 mm"}), "above 0"),
        (json!({"size": "2 * d"}), "20 mm"),
        (json!({"size": "12 mm"}), "10 mm across"),
        (json!({"pitch": "0 mm"}), "pitch"),
        (json!({"pitch": "d"}), "pitch"),
    ] {
        let mut all = json!({"feature": "Thread", "size": null, "pitch": null});
        for (key, value) in params.as_object().unwrap() {
            all[key] = value.clone();
        }
        run(&mut s, "feature.edit", all.clone());
        let error = failure(&mut s, "Thread");
        assert!(error.contains(says), "{all}: {error}");
    }
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Thread", "size": null, "pitch": null}),
    );
    assert_eq!(report(&mut s)["pitch"].as_f64().unwrap(), 1.5);
}

/// What the groove of an ISO thread of diameter `d` and pitch `p`
/// removes from a shank of that diameter over `length` of thread and its
/// run-out: the closed form of `thread.rs`.
fn removed(d: f64, p: f64, length: f64) -> f64 {
    let h = 3.0_f64.sqrt() / 2.0 * p;
    let depth = 5.0 * h / 8.0;
    let (big, root) = (d / 2.0, d / 2.0 - depth);
    let (a, b) = (p / 4.0, 7.0 * p / 8.0);
    let area = (a + b) / 2.0 * depth;
    let centre = root + depth * (a + 2.0 * b) / (3.0 * (a + b));
    let main = area * centre * TAU * length / p;
    let slope = 2.0 / 3.0_f64.sqrt();
    let moment = |rho: f64, to: f64| -> f64 {
        if to <= root {
            return 0.0;
        }
        let f = |r: f64| {
            let s = r - root;
            let c = root + rho;
            c * p / 4.0 * s + (c * slope + p / 4.0) * s * s / 2.0 + slope * s * s * s / 3.0
        };
        f(to) - f(root)
    };
    let rise = depth + h / 8.0;
    let n = 2000;
    let step = FRAC_PI_2 / f64::from(n);
    let mut run = 0.0;
    for i in 0..=n {
        let rho = rise * (f64::from(i) / f64::from(n)).powi(2);
        let w = if i == 0 || i == n {
            1.0
        } else if i % 2 == 1 {
            4.0
        } else {
            2.0
        };
        run += w * moment(rho, big - rho);
    }
    main + run * step / 3.0
}

#[test]
fn a_cut_thread_follows_its_shank_s_diameter() {
    // Two cut regenerations: the thread is slow to cut.
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    set_d(&mut s, 8.0);
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "d / 2", "height": "20 mm", "name": "Shank"}),
    );
    run(
        &mut s,
        "feature.thread",
        json!({"face": "F1:face(0)", "start": "F1:face(1)", "length": "12 mm",
            "modeled": true, "name": "Thread"}),
    );
    for (d, p) in [(8.0, 1.25), (6.0, 1.0)] {
        if d != 8.0 {
            set_d(&mut s, d);
        }
        let thread = report(&mut s);
        assert_eq!(thread["size"], format!("M{d}"), "{thread}");
        assert_eq!(thread["pitch"].as_f64().unwrap(), p, "{thread}");
        assert!(thread["deviation"].as_f64().unwrap() <= 0.5e-6, "{thread}");
        let found = PI * d * d / 4.0 * 20.0 - volume(&mut s, "Thread");
        // The groove's faces are within half the tolerance over an area
        // of about d per turn (`thread.rs` takes 10 for M10).
        let slack = 0.5e-6 * d * (12.0 / p + 2.0);
        let closed = removed(d, p, 12.0);
        assert!(
            (found - closed).abs() < slack,
            "M{d}: {found} against {closed}, {} off",
            found - closed
        );
    }
}

#[test]
fn a_thread_is_cosmetic_unless_modeled() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    set_d(&mut s, 10.0);
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "d / 2", "height": "20 mm", "name": "Shank"}),
    );
    let blank = PI * 25.0 * 20.0;
    let base = json!({"face": "F1:face(0)", "start": "F1:face(1)", "length": "12 mm"});
    let with = |extra: Value| {
        let mut all = base.clone();
        for (key, value) in extra.as_object().unwrap() {
            all[key] = value.clone();
        }
        all
    };
    // Neither flag disagreeing with the other.
    for both in [
        json!({"modeled": true, "cosmetic": true}),
        json!({"modeled": false, "cosmetic": false}),
    ] {
        let error = refused(&mut s, "feature.thread", with(both.clone()));
        assert!(error.contains("opposite"), "{both}: {error}");
    }
    // By default: cosmetic, the body unchanged.
    let made = run(&mut s, "feature.thread", with(json!({"name": "Thread"})));
    assert_eq!(made["thread"]["cosmetic"], true, "{made}");
    assert_eq!(made["thread"]["modeled"], false, "{made}");
    assert_eq!(feature(&mut s, "Thread")["params"]["modeled"], false);
    assert!((volume(&mut s, "Thread") - blank).abs() < 1e-9 * blank);
    // Agreeing flags are taken; `cosmetic: false` is `modeled: true`.
    // (Each on a thread too long for the shank, which fails before it is
    // cut: only what is stored is looked at here.)
    for (flags, modeled) in [
        (json!({"modeled": true, "cosmetic": false}), true),
        (json!({"cosmetic": false}), true),
        (json!({"cosmetic": true}), false),
        (json!({"modeled": false}), false),
    ] {
        run(&mut s, "file.new", json!({}));
        set_d(&mut s, 10.0);
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": "d / 2", "height": "20 mm", "name": "Shank"}),
        );
        let mut all = with(flags.clone());
        all["length"] = json!("19.5 mm");
        all["name"] = json!("Long");
        run(&mut s, "feature.thread", all);
        assert_eq!(
            feature(&mut s, "Long")["params"]["modeled"],
            modeled,
            "{flags}"
        );
        assert!(failure(&mut s, "Long").contains("shorter"));
        // feature.edit takes either name too.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Long", "cosmetic": modeled}),
        );
        assert_eq!(feature(&mut s, "Long")["params"]["modeled"], !modeled);
        let error = refused(
            &mut s,
            "feature.edit",
            json!({"feature": "Long", "modeled": true, "cosmetic": true}),
        );
        assert!(error.contains("opposite"), "{error}");
    }
}
