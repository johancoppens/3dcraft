//! A host with one thread (the page) gets a document ready a piece at a
//! time (`Session::prepare_some`): the regeneration a feature at a time,
//! the first picture and the display mesh a body at a time, the
//! assertions one at a time; and it ends with exactly what a session that
//! does it all at once shows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SceneQuality, Session, Workspace};
use serde_json::{Value, json};

/// Plates at scale `s` (in mm), joined, with a hole cut from the first,
/// and assertions on them.
fn build(session: &mut Session, s: f64) {
    let mm = |v: f64| format!("{} mm", v * s);
    let steps = [
        ("file.new", json!({"units": "mm"})),
        ("variable.set", json!({"name": "w", "expr": mm(40.0)})),
        (
            "feature.block",
            json!({"size": ["w", mm(20.0), mm(5.0)], "name": "A"}),
        ),
        (
            "feature.block",
            json!({"size": [mm(10.0), mm(10.0), mm(10.0)], "position": [mm(60.0), "0 mm", "0 mm"], "name": "B"}),
        ),
        (
            "feature.block",
            json!({"size": [mm(10.0), mm(10.0), mm(10.0)], "position": [mm(80.0), "0 mm", "0 mm"], "name": "C"}),
        ),
        (
            "feature.cylinder",
            json!({"radius": mm(2.0), "height": mm(20.0), "position": [mm(10.0), mm(10.0), mm(-5.0)], "name": "Pin"}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "A", "tool": "Pin", "name": "Holed"}),
        ),
        (
            "assert.add",
            json!({"name": "two blocks", "measure": "volume", "of": ["B", "C"], "eq": format!("2 * ({})^3", mm(10.0)), "tol": "1e-6 * (1 mm)^3"}),
        ),
        (
            "assert.add",
            json!({"name": "apart", "measure": "overlap", "is": false}),
        ),
        (
            "assert.add",
            json!({"name": "plate", "measure": "volume", "of": "Holed", "min": format!("{} * (1 mm)^3", s * s * s)}),
        ),
    ];
    for (id, params) in steps {
        session
            .execute(id, &params)
            .unwrap_or_else(|e| panic!("{id}: {}", e.message()));
    }
}

/// The scene a viewer shows, as numbers that must be the same bits.
fn picture(session: &mut Session) -> Vec<u64> {
    let scene = session
        .scene_with(SceneQuality::display())
        .unwrap()
        .unwrap();
    assert!(!scene.preview, "the display scene");
    let mut bits: Vec<u64> = scene
        .mesh
        .positions
        .iter()
        .flat_map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
        .collect();
    bits.extend(scene.mesh.triangles.iter().flatten().map(|&i| u64::from(i)));
    bits.push(scene.tolerance.to_bits());
    bits
}

fn states(session: &mut Session) -> Value {
    session
        .execute("assert.list", &json!({ "wait": true }))
        .unwrap()["assertions"]
        .clone()
}

#[test]
fn a_piece_at_a_time_ends_as_all_at_once() {
    for s in [1e-2, 1.0, 1e3] {
        let mut whole = Session::new(Workspace::none());
        build(&mut whole, s);
        let want_picture = picture(&mut whole);
        let want_states = states(&mut whole);
        assert!(
            want_states
                .as_array()
                .unwrap()
                .iter()
                .all(|a| a["status"] == "passed"),
            "{want_states}"
        );

        let mut session = Session::new(Workspace::none());
        session.set_stepwise(true);
        build(&mut session, s);
        // An edit and back, so that everything is to be done again from
        // what is cached: here from a new parameter value.
        session
            .execute(
                "variable.set",
                &json!({"name": "w", "expr": format!("{} mm", 41.0 * s)}),
            )
            .unwrap();
        session
            .execute(
                "variable.set",
                &json!({"name": "w", "expr": format!("{} mm", 40.0 * s)}),
            )
            .unwrap();
        let mut seen: Vec<(String, usize, usize)> = Vec::new();
        let mut calls = 0;
        // Stop as soon as allowed: one feature, body or assertion a call.
        while let Some(p) = session.prepare_some(SceneQuality::display(), || true) {
            calls += 1;
            assert!(calls < 100, "no progress: {seen:?}");
            seen.push((p.phase.to_string(), p.step, p.steps));
        }
        let phases: Vec<&str> = seen.iter().map(|(p, ..)| p.as_str()).collect();
        assert!(phases.contains(&"checking"), "{seen:?} at scale {s}");
        // The assertions one at a time, of 3 (those whose bodies the edit
        // left as they were are not measured again).
        let checking: Vec<(usize, usize)> = seen
            .iter()
            .filter(|(p, ..)| p == "checking")
            .map(|(_, a, b)| (*a, *b))
            .collect();
        assert!(
            checking
                .windows(2)
                .all(|w| w[1].0 == w[0].0 + 1 && w[1].1 == 3)
                && checking.iter().all(|&(done, of)| of == 3 && done < 3),
            "{seen:?}"
        );
        // Phases in order, never back.
        let order = ["regenerating", "meshing", "refining", "checking"];
        let ranks: Vec<usize> = phases
            .iter()
            .map(|p| order.iter().position(|o| o == p).unwrap())
            .collect();
        assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert_eq!(picture(&mut session), want_picture, "scale {s}");
        assert_eq!(states(&mut session), want_states, "scale {s}");
    }
}

/// Opening a file regenerates it a feature at a time: each call builds
/// one more (counted from 1) and says which one is next.
#[test]
fn a_file_opened_is_regenerated_a_feature_at_a_time() {
    let dir = std::env::temp_dir().join(format!("craft3d-stepwise-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plates.3dcraft").to_string_lossy().to_string();
    let mut maker = Session::new(Workspace::unrestricted());
    build(&mut maker, 1.0);
    maker
        .execute("file.save", &json!({ "path": path }))
        .unwrap();
    let mut session = Session::new(Workspace::unrestricted());
    session.set_stepwise(true);
    session
        .execute("file.open", &json!({ "path": path }))
        .unwrap();
    let mut regenerating = Vec::new();
    while let Some(p) = session.prepare_some(SceneQuality::display(), || true) {
        if p.phase == "regenerating" {
            regenerating.push((p.step, p.steps, p.name.clone().unwrap_or_default()));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        regenerating,
        [
            (1, 5, "B".to_string()),
            (2, 5, "C".to_string()),
            (3, 5, "Pin".to_string()),
            (4, 5, "Holed".to_string()),
        ]
    );
    // Nothing left, and a session that does it all at once agrees.
    assert!(
        session
            .prepare_some(SceneQuality::display(), || true)
            .is_none()
    );
    assert_eq!(picture(&mut session), picture(&mut maker));
}

/// Without a stop that says so, it is all done in one call.
#[test]
fn a_stop_that_never_stops_does_it_all_in_one_call() {
    let mut session = Session::new(Workspace::none());
    session.set_stepwise(true);
    build(&mut session, 1.0);
    session
        .execute("variable.set", &json!({"name": "w", "expr": "50 mm"}))
        .unwrap();
    assert!(
        session
            .prepare_some(SceneQuality::display(), || false)
            .is_none()
    );
    assert!(
        session
            .assertion_states()
            .iter()
            .all(|a| format!("{:?}", a.status) == "Passed")
    );
}

/// Commands between the pieces of a regeneration: a parameter set and
/// undone, the document saved and exported, then the rest a piece at a
/// time. The features (names, statuses, bodies) and the picture are what
/// a session that opens the same file and does it all at once has; also
/// with the rollback marker on a feature in the middle.
#[test]
fn commands_between_the_pieces_change_nothing_that_is_built() {
    let dir = std::env::temp_dir().join(format!("craft3d-stepwise-cmds-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (variant, marker) in [("plain", None), ("rolled", Some("Pin"))] {
        let path = dir
            .join(format!("{variant}.3dcraft"))
            .to_string_lossy()
            .to_string();
        let again = dir
            .join(format!("{variant}-again.3dcraft"))
            .to_string_lossy()
            .to_string();
        let stl = dir
            .join(format!("{variant}.stl"))
            .to_string_lossy()
            .to_string();
        let mut maker = Session::new(Workspace::unrestricted());
        build(&mut maker, 1.0);
        maker
            .execute(
                "variable.publish",
                &json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
            )
            .unwrap();
        if let Some(feature) = marker {
            maker
                .execute("history.rollback", &json!({ "feature": feature }))
                .unwrap();
        }
        maker
            .execute("file.save", &json!({ "path": path }))
            .unwrap();

        let mut session = Session::new(Workspace::unrestricted());
        session.set_stepwise(true);
        session
            .execute("file.open", &json!({ "path": path }))
            .unwrap();
        let piece = |session: &mut Session| {
            session
                .prepare_some(SceneQuality::display(), || true)
                .map(|p| p.phase)
        };
        assert_eq!(piece(&mut session), Some("regenerating"), "{variant}");
        session
            .execute("param.set", &json!({"values": {"w": 60}}))
            .unwrap();
        let _ = piece(&mut session);
        session.execute("edit.undo", &json!({})).unwrap();
        let _ = piece(&mut session);
        session
            .execute("file.save", &json!({ "path": again }))
            .unwrap();
        let _ = piece(&mut session);
        session
            .execute("file.export", &json!({ "path": stl }))
            .unwrap();
        let mut calls = 0;
        while piece(&mut session).is_some() {
            calls += 1;
            assert!(calls < 100, "{variant}: no progress");
        }

        let mut fresh = Session::new(Workspace::unrestricted());
        fresh
            .execute("file.open", &json!({ "path": path }))
            .unwrap();
        let features = |s: &mut Session| {
            s.execute("document.inspect", &json!({})).unwrap()["features"].clone()
        };
        let got = features(&mut session);
        assert_eq!(got, features(&mut fresh), "{variant}");
        if marker.is_some() {
            assert!(got.to_string().contains("rolledBack"), "{variant}: {got}");
        }
        assert_eq!(picture(&mut session), picture(&mut fresh), "{variant}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// An edit on a host that works a piece at a time leaves the
/// regeneration to the pieces: `param.set` regenerates nothing itself
/// (it held the page's one thread for the whole regeneration before), its
/// result says `deferred` instead of what fails and the assertions'
/// summary, and the pieces then regenerate a feature at a time and end
/// as a session that does it all at once (features with their statuses,
/// the picture, the assertions). At three scales, for values that move
/// the plate's end past the hole and back.
#[test]
fn an_edit_leaves_its_regeneration_to_the_pieces() {
    for s in [1e-2, 1.0, 1e3] {
        let mm = |v: f64| format!("{} mm", v * s);
        let publish = json!({"name": "w", "min": mm(1.0), "max": mm(90.0)});
        let mut whole = Session::new(Workspace::none());
        build(&mut whole, s);
        whole.execute("variable.publish", &publish).unwrap();
        let mut session = Session::new(Workspace::none());
        session.set_stepwise(true);
        build(&mut session, s);
        session.execute("variable.publish", &publish).unwrap();
        while session
            .prepare_some(SceneQuality::display(), || false)
            .is_some()
        {}
        for value in [60.0, 30.0, 5.0] {
            let set = json!({"values": {"w": value * s}, "clamp": true});
            let all_at_once = whole.execute("param.set", &set).unwrap();
            assert!(all_at_once.get("deferred").is_none(), "{all_at_once}");
            let (done, built) = rebuilt(|| session.execute("param.set", &set).unwrap());
            assert_eq!(built, 0, "scale {s}, w {value}: the command regenerated");
            assert_eq!(done["deferred"], true, "{done}");
            assert!(done.get("failed").is_none(), "{done}");
            assert!(done.get("assertions").is_none(), "{done}");
            assert_eq!(done["set"], all_at_once["set"], "{done}");
            assert_eq!(done["changed"], true, "{done}");
            let mut phases = Vec::new();
            while let Some(p) = session.prepare_some(SceneQuality::display(), || true) {
                phases.push(p.phase);
                assert!(phases.len() < 100, "{phases:?}");
            }
            assert_eq!(phases.first(), Some(&"regenerating"), "{phases:?}");
            let features = |s: &mut Session| {
                s.execute("document.inspect", &json!({})).unwrap()["features"].clone()
            };
            assert_eq!(features(&mut session), features(&mut whole), "scale {s}");
            assert_eq!(picture(&mut session), picture(&mut whole), "scale {s}");
            assert_eq!(states(&mut session), states(&mut whole), "scale {s}");
            // Regenerated now: the same value again regenerates nothing.
            let (_, built) = rebuilt(|| session.execute("param.set", &set).unwrap());
            assert_eq!(built, 0, "scale {s}, w {value}: the same value again");
        }
    }
}

/// An edit given `wait: true` is final on every host: on one that works
/// a piece at a time it regenerates all the same and reports what fails
/// (`failed`, against the state before it, regenerated first when it was
/// not yet) and the assertions, never `deferred`, exactly as a session
/// that does it all at once. A block whose width is `w - 35 mm` fails
/// below 35 mm. At three scales.
#[test]
fn an_edit_that_waits_reports_what_fails_on_a_host_that_works_a_piece_at_a_time() {
    for s in [1e-2, 1.0, 1e3] {
        let mm = |v: f64| format!("{} mm", v * s);
        let publish = json!({"name": "w", "min": mm(1.0), "max": mm(90.0)});
        let narrow = json!({
            "size": [format!("w - {}", mm(35.0)), mm(10.0), mm(10.0)],
            "position": [mm(200.0), "0 mm", "0 mm"],
            "name": "D",
        });
        let mut whole = Session::new(Workspace::none());
        let mut session = Session::new(Workspace::none());
        session.set_stepwise(true);
        for one in [&mut whole, &mut session] {
            build(one, s);
            one.execute("feature.block", &narrow).unwrap();
            one.execute("variable.publish", &publish).unwrap();
        }
        while session
            .prepare_some(SceneQuality::display(), || false)
            .is_some()
        {}
        // Not waited for: deferred, and the state after it not regenerated
        // when the next edit comes.
        let first = json!({"values": {"w": 60.0 * s}});
        whole.execute("param.set", &first).unwrap();
        assert_eq!(
            session.execute("param.set", &first).unwrap()["deferred"],
            true
        );
        for (value, fails) in [(30.0, true), (20.0, false), (50.0, false)] {
            let set = json!({"values": {"w": value * s}, "wait": true});
            let all_at_once = whole.execute("param.set", &set).unwrap();
            let done = session.execute("param.set", &set).unwrap();
            assert!(
                done.get("deferred").is_none(),
                "scale {s}, w {value}: {done}"
            );
            assert_eq!(done, all_at_once, "scale {s}, w {value}");
            // D fails from 30 on; at 20 it failed before already.
            assert_eq!(
                done.get("failed")
                    .is_some_and(|f| f.to_string().contains("\"D\"")),
                fails,
                "scale {s}, w {value}: {done}"
            );
            assert_eq!(done["assertions"]["pending"], 0, "{done}");
            // The same through variable.set, which takes wait too.
            let var = json!({"name": "w", "expr": mm(value), "wait": true});
            let all_at_once = whole.execute("variable.set", &var).unwrap();
            let done = session.execute("variable.set", &var).unwrap();
            assert!(done.get("deferred").is_none(), "{done}");
            assert_eq!(done, all_at_once, "scale {s}, w {value}");
        }
    }
}

/// A clearance over many pairs of bodies and an overlap over all of
/// them, on a host that works a piece at a time: taken a pair at a time
/// (with a stop that always says stop, a pair per piece, not an
/// assertion per piece: the gear reducer's held a frame for seconds),
/// and the answers, details included, bit for bit those of a session
/// that measures them in one go. At three scales.
#[test]
fn a_measure_over_many_pairs_of_bodies_is_taken_a_pair_at_a_time() {
    for s in [1e-2, 1.0, 1e3] {
        let mm = |v: f64| format!("{} mm", v * s);
        let mut steps = vec![("file.new", json!({"units": "mm"}))];
        let (mut from, mut to) = (Vec::new(), Vec::new());
        for i in 0..4 {
            let name = format!("A{i}");
            steps.push((
                "feature.block",
                json!({"size": [mm(10.0), mm(10.0), mm(10.0)], "position": [mm(20.0 * f64::from(i)), "0 mm", "0 mm"], "name": name}),
            ));
            from.push(name);
        }
        for j in 0..3 {
            let name = format!("B{j}");
            steps.push((
                "feature.cylinder",
                json!({"radius": mm(4.0), "height": mm(10.0), "position": [mm(20.0 * f64::from(j) + 5.0), mm(30.0), "0 mm"], "name": name}),
            ));
            to.push(name);
        }
        steps.push((
            "assert.add",
            json!({"name": "gap", "measure": "clearance", "from": from, "to": to, "min": mm(1.0)}),
        ));
        steps.push((
            "assert.add",
            json!({"name": "apart", "measure": "overlap", "is": false}),
        ));
        let mut whole = Session::new(Workspace::none());
        let mut session = Session::new(Workspace::none());
        session.set_stepwise(true);
        for one in [&mut whole, &mut session] {
            for (id, params) in &steps {
                one.execute(id, params)
                    .unwrap_or_else(|e| panic!("{id}: {}", e.message()));
            }
        }
        let mut checking = 0;
        let mut calls = 0;
        while let Some(p) = session.prepare_some(SceneQuality::display(), || true) {
            calls += 1;
            assert!(calls < 1000, "scale {s}");
            if p.phase == "checking" {
                checking += 1;
            }
        }
        // 12 pairs, then 21: a piece each (the last piece of all says
        // nothing is left).
        assert!(checking >= 12 + 21 - 1, "scale {s}: {checking} pieces");
        let listed = |one: &mut Session| {
            one.execute("assert.list", &json!({"wait": true, "details": true}))
                .unwrap()
        };
        let (got, want) = (listed(&mut session), listed(&mut whole));
        assert_eq!(got, want, "scale {s}");
        assert_eq!(want["ok"], true, "scale {s}: {want}");
        let pairs = &want["assertions"][0]["detail"]["pairs"];
        assert_eq!(pairs.as_array().map(Vec::len), Some(12), "{want}");
    }
}

/// What `work` returns, and how many features the regenerations on this
/// thread built afresh meanwhile (each tells the thread's progress sink,
/// so tests running at once on other threads do not count).
fn rebuilt<R>(work: impl FnOnce() -> R) -> (R, usize) {
    let count = std::rc::Rc::new(std::cell::Cell::new(0_usize));
    let seen = std::rc::Rc::clone(&count);
    let out = craft3d_regen::progress::with_sink(
        move |p| {
            if p.phase == "regenerating" {
                seen.set(seen.get() + 1);
            }
        },
        work,
    );
    (out, count.get())
}
