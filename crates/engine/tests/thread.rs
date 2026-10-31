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

use craft3d_math::Portable;
mod common;

use common::checked::{
    deep, failure, feature_brief as feature, inspect, refused_with as refused, run,
};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::path::PathBuf;

fn workdir(name: &str) -> PathBuf {
    common::workdir("thread", name)
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
        let rho = rise * (f64::from(i) / f64::from(n)).ppowi(2);
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
    let mut all = json!({"face": "F1:face(0)", "start": "F1:face(1)", "size": "M10", "length": "12 mm", "modeled": true, "name": "Thread"});
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
    assert_eq!(cosmetic["thread"]["modeled"], false, "{cosmetic}");
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
        (json!({"size": "M10x1.25"}), "ISO metric"),
        (json!({"size": "  "}), "ISO metric"),
        (json!({"size": 10}), "size"),
        (json!({"size": "M15"}), "fine pitches: 1.5, 1 mm"),
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
        // Any other text is a length, and `X10` is no variable.
        ("Unknown", json!({"size": "X10"}), "ISO metric"),
        ("Angle", json!({"size": "10 deg"}), "length"),
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

#[test]
fn a_thread_given_by_the_names_of_an_earlier_body_follows_it() {
    // After a lead chamfer the shank's faces are the chamfer's copies;
    // the thread names them as the cylinder had them, and is built as
    // with the full names.
    let mut volumes = Vec::new();
    for (face, start) in [
        ("F1:face(0)", "F1:face(1)"),
        ("F2:copy(0)/F1:face(0)", "F2:copy(0)/F1:face(1)"),
    ] {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({}));
        shank(&mut s);
        run(
            &mut s,
            "feature.chamfer",
            json!({"edges": [["F1:face(0)", "F1:face(1)"]], "distance": "1 mm", "name": "Lead"}),
        );
        let made = thread(&mut s, json!({"face": face, "start": start}));
        assert_eq!(made["status"], "ok", "{made}");
        volumes.push(volume(&mut s, "Thread"));
    }
    assert_eq!(volumes[0], volumes[1]);
}

/// What the groove of an internal ISO thread of diameter `d` and pitch
/// `p` removes from the wall round a hole of radius `hole` over `length`
/// of thread and its run-out into the hole: the groove is the nut's,
/// `P/8 + 2 s tan 30°` wide at `s` in from the major radius (the
/// derivation is the kernel's test, `crates/kernel/tests/helix.rs`).
fn removed_internal(d: f64, p: f64, hole: f64, length: f64) -> f64 {
    let h = 3.0_f64.sqrt() / 2.0 * p;
    let depth = 5.0 * h / 8.0;
    let big = d / 2.0;
    let slope = 2.0 / 3.0_f64.sqrt();
    let deepest = depth + h / 16.0;
    let moment = |rho: f64| -> f64 {
        let to = (big - rho - hole).clamp(0.0, deepest);
        let a = big - rho;
        a * p / 8.0 * to + (a * slope - p / 8.0) * to * to / 2.0 - slope * to * to * to / 3.0
    };
    let main = moment(0.0) * TAU * length / p;
    let rise = depth + h / 8.0;
    let n = 2000;
    let step = FRAC_PI_2 / f64::from(n);
    let mut run = 0.0;
    for i in 0..=n {
        let rho = rise * (f64::from(i) / f64::from(n)).ppowi(2);
        let w = if i == 0 || i == n {
            1.0
        } else if i % 2 == 1 {
            4.0
        } else {
            2.0
        };
        run += w * moment(rho);
    }
    main + run * step / 3.0
}

/// A tube from z = 0 to 20, radius 9, with a through hole of `hole`
/// diameter (mm): `Tube`, whose hole's wall is `F3:copy(1)/F2:face(0)`
/// and bottom `F3:copy(0)/F1:face(1)`.
fn tube(s: &mut Session, hole: f64) {
    run(
        s,
        "feature.cylinder",
        json!({"radius": "9 mm", "height": "20 mm", "name": "Outer"}),
    );
    run(
        s,
        "feature.cylinder",
        json!({"radius": format!("{} mm", hole / 2.0), "height": "30 mm", "position": ["0 mm", "0 mm", "-5 mm"], "name": "Bore"}),
    );
    run(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Outer", "tool": "Bore", "name": "Tube"}),
    );
}

/// Tapped holes: a thread on the wall of a hole is internal, found from
/// the face. Left without a size, a hole of M10's tap drill (8.5) is M10
/// and one of M6's minor diameter (4.917) M6, coarse; the thread reports
/// internal and the tap drill. Cosmetic, the body is as it was; modeled,
/// the nut's groove is cut into the wall, and the volume it removes is
/// the closed form; the body passes the deep check. A hole of no size's
/// tap drill and a size given that does not fit the hole fail with the
/// reason.
#[test]
fn a_thread_in_a_hole_is_internal_cosmetic_or_modeled() {
    let minor6 = 6.0 - 1.082_531_754_730_548;
    for (hole, size, pitch) in [(8.5, "M10", 1.5), (minor6, "M6", 1.0)] {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({}));
        tube(&mut s, hole);
        let before = volume(&mut s, "Tube");
        let made = run(
            &mut s,
            "feature.thread",
            json!({"face": "F3:copy(1)/F2:face(0)", "start": "F3:copy(0)/F1:face(1)", "length": "12 mm", "name": "Tapped"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        let thread = feature(&mut s, "Tapped")["thread"].clone();
        assert_eq!(thread["size"], size, "{thread}");
        assert_eq!(thread["pitch"].as_f64().unwrap(), pitch, "{thread}");
        assert_eq!(thread["internal"], true, "{thread}");
        assert_eq!(thread["cosmetic"], true, "{thread}");
        let major: f64 = size[1..].parse().unwrap();
        assert!((thread["tapDrill"].as_f64().unwrap() - (major - pitch)).abs() < 1e-12);
        assert!((volume(&mut s, "Tapped") - before).abs() < 1e-9 * before);

        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Tapped", "modeled": true}),
        );
        let thread = feature(&mut s, "Tapped")["thread"].clone();
        assert_eq!(thread["modeled"], true, "{thread}");
        let removed = before - volume(&mut s, "Tapped");
        let expected = removed_internal(major, pitch, hole / 2.0, 12.0);
        let slack = 0.5e-6 * 10.0 * (12.0 / pitch + 2.0);
        eprintln!("{size} internal: removed {removed} against {expected}");
        assert!(
            (removed - expected).abs() < slack,
            "{size}: removed {removed} against {expected}"
        );
        deep(&mut s);
    }

    // A hole of 9: between M10's tap drill and M11's minor diameter.
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    tube(&mut s, 9.0);
    run(
        &mut s,
        "feature.thread",
        json!({"face": "F3:copy(1)/F2:face(0)", "start": "F3:copy(0)/F1:face(1)", "length": "12 mm", "name": "Tapped"}),
    );
    let error = failure(&mut s, "Tapped");
    assert!(error.contains("tapped for no ISO metric size"), "{error}");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Tapped", "size": "M12"}),
    );
    let error = failure(&mut s, "Tapped");
    assert!(error.contains("tap drill's diameter, 10.25"), "{error}");
}
