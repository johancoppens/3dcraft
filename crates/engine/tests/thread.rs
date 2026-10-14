//! Threads through the commands: `feature.thread` on the wall of a
//! cylinder, from its bottom.
//!
//! The closed form of what the groove removes (`crates/kernel/tests/
//! helix.rs` has the derivation and the same form against the kernel
//! alone): along the thread at full depth `A r̄ 2π L / P`, with `A` the
//! groove's area inside the major diameter and `r̄` the distance of its
//! centre from the axis; along the run-out, the integral over the quarter
//! turn of the moment of the part of the groove still inside, by
//! Simpson's rule here.

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
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("thread-{name}"));
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

fn body(s: &mut Session, name: &str) -> Value {
    let document = inspect(s);
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name}: {}", document["features"]))
        .clone()
}

fn volume(s: &mut Session, name: &str) -> f64 {
    body(s, name)["volume"].as_f64().unwrap()
}

fn failure(s: &mut Session, name: &str) -> String {
    let state = feature(s, name);
    assert_eq!(state["status"], "failed", "{state}");
    state["error"].as_str().unwrap().to_string()
}

fn deep(s: &mut Session) {
    let check = run(s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

/// What the groove of an ISO thread of diameter `d` and pitch `p`
/// removes from a shank of that diameter over `length` of thread and its
/// run-out (see the module documentation).
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

/// A shank of radius 5 and height 20 from z = 0: `F1:face(0)` is its
/// wall, `F1:face(1)` its bottom, `F1:face(2)` its top.
fn shank(s: &mut Session) {
    run(
        s,
        "feature.cylinder",
        json!({"radius": "5 mm", "height": "20 mm", "name": "Shank"}),
    );
}

fn thread(s: &mut Session, params: Value) -> Value {
    let mut all = json!({"face": "F1:face(0)", "start": "F1:face(1)", "size": "M10", "length": "12 mm", "name": "Thread"});
    for (key, value) in params.as_object().unwrap() {
        all[key] = value.clone();
    }
    run(s, "feature.thread", all)
}

/// The groove's faces are within half the tolerance of the true ones
/// over an area of about 10 per turn.
fn slack(p: f64, length: f64) -> f64 {
    0.5e-6 * 10.0 * (length / p + 2.0)
}

#[test]
fn an_m10_thread_removes_its_groove_and_follows_its_parameters() {
    let dir = workdir("m10");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({}));
    shank(&mut s);
    let blank = PI * 25.0 * 20.0;
    let made = thread(&mut s, json!({}));
    assert_eq!(made["status"], "ok", "{made}");
    let report = &made["thread"];
    assert_eq!(report["size"], "M10");
    assert_eq!(report["pitch"], 1.5);
    assert_eq!(report["major"], 10.0);
    assert!(
        (report["minor"].as_f64().unwrap() - (10.0 - 1.082_531_754_730_548 * 1.5)).abs() < 1e-12
    );
    assert!(report["deviation"].as_f64().unwrap() <= 0.5e-6);
    deep(&mut s);
    let found = blank - volume(&mut s, "Thread");
    assert!(
        (found - removed(10.0, 1.5, 12.0)).abs() < slack(1.5, 12.0),
        "{found} against {}",
        removed(10.0, 1.5, 12.0)
    );
    // Its faces: the groove's own and copies of the shank's. The groove
    // starts a pitch before the end of the shank, outside it: the faces
    // of its first half turns are not left (`F2:thread(0)`, the root's
    // first, lies wholly outside).
    let names: Vec<String> = body(&mut s, "Thread")["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    for wanted in [
        "F2:piece(2)/F2:thread(0)",
        "F2:piece(2)/F2:thread(1)",
        "F2:piece(1)/F2:thread(2)",
        "F2:piece(18)/F2:thread(1)",
        "F2:copy(0)/F1:face(0)",
        "F2:copy(0)/F1:face(1)",
        "F2:copy(0)/F1:face(2)",
    ] {
        assert!(names.iter().any(|n| n == wanted), "{wanted}: {names:?}");
    }
    assert!(!names.iter().any(|n| n.contains("copy(1)")), "{names:?}");
    assert!(!names.iter().any(|n| n == "F2:thread(0)"), "{names:?}");
    assert_eq!(feature(&mut s, "Thread")["thread"]["size"], "M10");

    // A fine pitch, a shorter thread.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Thread", "pitch": "1.25 mm", "length": "10 mm"}),
    );
    let found = blank - volume(&mut s, "Thread");
    assert!(
        (found - removed(10.0, 1.25, 10.0)).abs() < slack(1.25, 10.0),
        "{found} against {}",
        removed(10.0, 1.25, 10.0)
    );
    // Left-handed, the same volume.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Thread", "leftHanded": true}),
    );
    let left = blank - volume(&mut s, "Thread");
    assert!((left - found).abs() < slack(1.25, 10.0), "{left} {found}");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    let back = blank - volume(&mut s, "Thread");
    assert!((back - removed(10.0, 1.5, 12.0)).abs() < slack(1.5, 12.0));
    run(&mut s, "edit.redo", json!({}));
    let again = blank - volume(&mut s, "Thread");
    assert!((again - found).abs() < 1e-12, "{again} {found}");

    // Saved and opened again, the same body.
    let before = inspect(&mut s)["bodies"].clone();
    run(&mut s, "file.save", json!({"path": "bolt.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "bolt.3dcraft"}));
    assert_eq!(inspect(&mut s)["bodies"], before);

    // Cosmetic: the body is the shank's, the thread is said.
    let cosmetic = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Thread", "cosmetic": true}),
    );
    assert_eq!(cosmetic["thread"]["cosmetic"], true, "{cosmetic}");
    assert!((volume(&mut s, "Thread") - blank).abs() < 1e-9 * blank);
    deep(&mut s);
}

#[test]
fn a_thread_from_the_top_runs_down_the_shank() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    shank(&mut s);
    let made = thread(&mut s, json!({"start": "F1:face(2)", "length": "8 mm"}));
    assert_eq!(made["status"], "ok", "{made}");
    deep(&mut s);
    let found = PI * 25.0 * 20.0 - volume(&mut s, "Thread");
    assert!((found - removed(10.0, 1.5, 8.0)).abs() < slack(1.5, 8.0));
}

#[test]
fn what_cannot_be_threaded_fails_with_the_reason() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    shank(&mut s);
    // Refused by the command.
    for (params, says) in [
        (json!({"size": "X10"}), "ISO metric"),
        (json!({"size": 10}), "size"),
        (json!({"size": "M11"}), "coarse pitch"),
        (json!({"face": "F1:edge(0)"}), "not a face"),
        (json!({"length": null}), "length"),
    ] {
        let mut all =
            json!({"face": "F1:face(0)", "start": "F1:face(1)", "size": "M10", "length": "12 mm"});
        for (key, value) in params.as_object().unwrap() {
            if value.is_null() {
                all.as_object_mut().unwrap().remove(key);
            } else {
                all[key] = value.clone();
            }
        }
        let error = refused(&mut s, "feature.thread", all.clone());
        assert!(error.contains(says), "{all}: {error}");
    }
    // Failed by regeneration, with the parameter.
    for (name, params, says) in [
        ("Flat", json!({"face": "F1:face(1)"}), "cylinder"),
        ("Round", json!({"start": "F1:face(0)"}), "flat"),
        ("Large", json!({"size": "M12"}), "diameter"),
        ("Long", json!({"length": "19.5 mm"}), "shorter"),
        ("Zero", json!({"length": "0 mm"}), "length"),
        ("Steep", json!({"pitch": "12 mm"}), "pitch"),
        ("Missing", json!({"face": "F1:face(7)"}), "face"),
    ] {
        let mut all = json!({"face": "F1:face(0)", "start": "F1:face(1)", "size": "M10", "length": "12 mm", "name": name});
        for (key, value) in params.as_object().unwrap() {
            all[key] = value.clone();
        }
        // Each on a shank of its own: a thread takes the shank's body.
        run(&mut s, "file.new", json!({}));
        shank(&mut s);
        run(&mut s, "feature.thread", all);
        let error = failure(&mut s, name);
        assert!(error.contains(says), "{name}: {error}");
    }
}
