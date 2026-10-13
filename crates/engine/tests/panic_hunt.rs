//! Rule 13 of AGENTS.md: no command may panic, whatever its parameters and
//! whatever the document. Every command of the registry is run with
//! adversarial values for each of its parameters, alone and combined, with
//! and without a document. A panic inside a command is caught by
//! `Session::execute` and reported as "internal error", which fails here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_testkit::parallel::map_parallel;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn adversarial() -> Vec<Value> {
    let mut deep = json!(1);
    for _ in 0..500 {
        deep = json!([deep]);
    }
    vec![
        Value::Null,
        json!(true),
        json!(false),
        json!(0),
        json!(-1),
        json!(1),
        json!(0.5),
        json!(-0.0),
        json!(1e308),
        json!(-1e308),
        json!(5e-324),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(""),
        json!(" "),
        json!("a"),
        json!("Plate"),
        json!("F1"),
        json!("F0"),
        json!("Profile"),
        json!("e1"),
        json!("e7"),
        json!("e0"),
        json!("c1"),
        json!("c18446744073709551615"),
        json!("F7:side(e5)"),
        json!("F7:endCap(e5,e6,e7,e8)"),
        json!("F8:side(e10)"),
        json!("F1:side(e1)"),
        json!("F0:lateralEdge()"),
        json!("F8:side(e18446744073709551615)"),
        json!("F8:side(".to_string() + &"e1,".repeat(5_000) + "e1)"),
        json!("distance"),
        json!("tangent"),
        json!("zx"),
        json!(["e1", "e2"]),
        json!(["e7", "e8"]),
        json!(["e7"]),
        json!(["e1", "e1"]),
        json!([[0, 0], [1, 0], [1, 1]]),
        json!([[0, 0], [0, 0], [0, 0]]),
        json!([[1e308, -1e308], "e1", [0, 5e-324]]),
        json!("F18446744073709551615"),
        json!("F99999999999999999999999"),
        json!("width"),
        json!("width / 0"),
        json!("1 mm"),
        json!("0 mm"),
        json!("-1 mm"),
        json!("1e999"),
        json!("1e308 m * 1e308 m"),
        json!("30 deg"),
        json!("nope + 1"),
        json!("sin"),
        json!("mm"),
        json!("💥"),
        json!("a\0b"),
        json!("a\nb"),
        json!("../../etc/passwd"),
        json!("/etc/passwd"),
        json!("x.stl"),
        json!("x.3dcraft"),
        json!("."),
        json!("stl"),
        json!("ids"),
        json!("(".repeat(10_000)),
        json!("9".repeat(10_000)),
        json!("ab ".repeat(5_000)),
        json!([]),
        json!([1]),
        json!([1, 2, 3]),
        json!([0, 0, 0]),
        json!([-1, -1, -1]),
        json!([1e308, 1e308, 1e308]),
        json!(["a", "b", "c"]),
        json!(["width", "width / 0", "1 +"]),
        json!([null, null, null]),
        json!([[], [], []]),
        json!([1, 2, 3, 4]),
        json!({}),
        json!({"a": 1}),
        deep,
    ]
}

/// The parameter names a command documents: the quoted words before a
/// colon in its params string.
fn keys(params: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = params;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else { break };
        let (word, tail) = (&after[..end], &after[end + 1..]);
        if tail.trim_start().starts_with(':') {
            keys.push(word.to_string());
        }
        rest = tail;
    }
    keys
}

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("panic-hunt-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A session in one of the states commands must survive.
fn workspace(dir: &std::path::Path) -> Workspace {
    Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    }
}

fn session(dir: &std::path::Path, state: usize) -> Session {
    let mut s = Session::new(workspace(dir));
    let mut run = |id: &str, params: Value| {
        s.execute(id, &params)
            .unwrap_or_else(|e| panic!("setup {id}: {e}"));
    };
    match state {
        // No document at all.
        0 => {}
        // An empty document.
        1 => run("file.new", json!({})),
        // A healthy part.
        2 => {
            run("file.new", json!({}));
            run("variable.set", json!({"name": "width", "expr": "40 mm"}));
            run(
                "feature.block",
                json!({"name": "Plate", "size": ["width", 20, 5]}),
            );
            run("feature.cylinder", json!({"radius": 2, "height": 10}));
            run("feature.sphere", json!({"radius": "width / 4"}));
            run(
                "feature.cone",
                json!({"radius": 3, "height": 9, "axis": [1, 1, 0]}),
            );
            run(
                "feature.torus",
                json!({"major": 10, "minor": 2, "angle": "30 deg"}),
            );
            // A constrained rectangle with a circle in it.
            run("sketch.create", json!({"name": "Profile"}));
            run(
                "sketch.polyline",
                json!({"sketch": "Profile", "closed": true, "points": [[0, 0], [9, 1], [10, 6], [1, 5]]}),
            );
            run(
                "sketch.circle",
                json!({"sketch": "Profile", "center": [5, 3], "radius": 1}),
            );
            for (kind, entities) in [
                ("fixed", json!(["e1"])),
                ("horizontal", json!(["e5"])),
                ("vertical", json!(["e6"])),
                ("parallel", json!(["e5", "e7"])),
            ] {
                run(
                    "sketch.constrain",
                    json!({"sketch": "Profile", "type": kind, "entities": entities}),
                );
            }
            run(
                "sketch.constrain",
                json!({"sketch": "Profile", "type": "distance", "entities": ["e5"], "value": "width / 4"}),
            );
            // A plate with a hole, made from it, and a boss on its side.
            run(
                "feature.extrude",
                json!({"sketch": "Profile", "distance": "width / 8", "name": "Body"}),
            );
            run(
                "sketch.create",
                json!({"name": "Seat", "face": "F7:side(e5)", "offset": 1}),
            );
            run(
                "sketch.circle",
                json!({"sketch": "Seat", "center": [3, 2], "radius": 1}),
            );
            run("feature.extrude", json!({"sketch": "Seat", "distance": 2}));
        }
        // A document full of things that fail: bad formulas, a cycle, a
        // suppressed and a rolled-back feature, undo and redo stacks.
        _ => {
            run("file.new", json!({"units": "in", "tolerance": 1e-9}));
            run("variable.set", json!({"name": "a", "expr": "b + 1"}));
            run("variable.set", json!({"name": "b", "expr": "a * 2"}));
            run("variable.set", json!({"name": "broken", "expr": "1 +"}));
            run("variable.set", json!({"name": "zero", "expr": "1 mm / 0"}));
            run(
                "feature.block",
                json!({"name": "Plate", "size": ["a", "broken", "zero"]}),
            );
            run("feature.block", json!({"size": [1e-300, 1e300, 1]}));
            run("feature.sphere", json!({"radius": "30 deg"}));
            run("feature.cylinder", json!({"radius": 1, "height": 1}));
            // A sketch that cannot be solved: a dimension that does not
            // evaluate, two that contradict each other, a degenerate line.
            run(
                "sketch.create",
                json!({"name": "Profile", "offset": "zero"}),
            );
            run(
                "sketch.polyline",
                json!({"sketch": "Profile", "points": [[0, 0], [1, 0], [1, 0], [1e-300, 1e300]]}),
            );
            for value in ["broken", "1 in", "2 in"] {
                run(
                    "sketch.constrain",
                    json!({"sketch": "Profile", "type": "distance", "entities": ["e5"], "value": value}),
                );
            }
            run(
                "sketch.constrain",
                json!({"sketch": "Profile", "type": "angle", "entities": ["e6", "e7"], "value": "a"}),
            );
            // An extrusion of that sketch, and one whose sketch is gone.
            run(
                "feature.extrude",
                json!({"sketch": "Profile", "distance": "zero"}),
            );
            run(
                "sketch.create",
                json!({"name": "Nowhere", "face": "F99:endCap(e1)", "offset": "broken"}),
            );
            run("sketch.create", json!({"name": "Gone"}));
            run("feature.extrude", json!({"sketch": "Gone", "distance": 1}));
            run("feature.delete", json!({"feature": "Gone"}));
            run("feature.suppress", json!({"feature": "Cylinder1"}));
            run("history.rollback", json!({"feature": "Plate"}));
            run("edit.undo", json!({}));
            run("edit.undo", json!({}));
        }
    }
    s
}

#[test]
fn no_command_panics_on_adversarial_parameters() {
    let dir = workdir("hunt");
    let values = adversarial();
    let commands = Session::commands();
    let templates: Vec<Session> = (0..4).map(|state| session(&dir, state)).collect();
    // One case per command, state and adversarial value: the value as the
    // params itself, under each parameter, under all of them at once, and
    // with one parameter left valid in turn. The cases are independent of
    // each other, so they run on every core, each in its own directory for
    // the files the commands write.
    let mut cases = Vec::new();
    for c in 0..commands.len() {
        for state in 0..4 {
            for v in 0..values.len() {
                cases.push((c, state, v));
            }
        }
    }
    let timed = map_parallel(cases, |(c, state, v)| {
        let clock = std::time::Instant::now();
        let spec = &commands[c];
        let value = &values[v];
        let keys = keys(spec.params);
        let dir = dir.join(format!("{c}-{state}-{v}"));
        std::fs::create_dir_all(&dir).unwrap();
        // The first adversarial value runs in a session built from scratch
        // (a cold regeneration cache, as after `file.open`); the others in
        // a clone of a session built once per state, whose cache is warm,
        // as in a session that has been working for a while. Both must
        // hold, and the clone saves rebuilding the document a hundred
        // thousand times.
        let fresh = |dir: &std::path::Path| -> Session {
            if v == 0 {
                session(dir, state)
            } else {
                let mut s = templates[state].clone();
                s.set_workspace(workspace(dir));
                s
            }
        };
        let mut runs = 0_usize;
        let mut check = |params: &Value| {
            runs += 1;
            run_case(fresh(&dir), state, spec.id, params);
        };
        // The params value itself, in every shape.
        check(value);
        // Each parameter in turn, the others absent.
        for key in &keys {
            check(&json!({ key.as_str(): value }));
        }
        // Every parameter at once with the same value, so the checks
        // behind the first failing parameter are reached as well: also
        // with one parameter left valid in turn.
        let all: Map<String, Value> = keys.iter().map(|k| (k.clone(), value.clone())).collect();
        check(&Value::Object(all.clone()));
        for (key, valid) in [
            ("feature", json!("Plate")),
            ("sketch", json!("Profile")),
            ("distance", json!(5)),
            ("face", json!("F7:side(e5)")),
            ("type", json!("distance")),
            ("entities", json!(["e1", "e2"])),
            ("entity", json!("e1")),
            ("constraint", json!("c1")),
            ("name", json!("width")),
            ("path", json!("hunt.stl")),
            ("size", json!([1, 2, 3])),
            ("radius", json!(1)),
            ("major", json!(5)),
            ("axis", json!([0, 0, 1])),
        ] {
            if all.contains_key(key) {
                let mut mixed = all.clone();
                mixed.insert(key.to_string(), valid);
                check(&Value::Object(mixed));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        (c, state, runs, clock.elapsed().as_secs_f64())
    });
    let runs: usize = timed.iter().map(|t| t.2).sum();
    // Where the time goes, per command and state: `--nocapture` shows it.
    let mut by_command: BTreeMap<(&str, usize), f64> = BTreeMap::new();
    for (c, state, _, secs) in &timed {
        *by_command.entry((commands[*c].id, *state)).or_default() += secs;
    }
    let mut slowest: Vec<_> = by_command.into_iter().collect();
    slowest.sort_by(|a, b| b.1.total_cmp(&a.1));
    let total: f64 = timed.iter().map(|t| t.3).sum();
    println!(
        "panic_hunt: {runs} command runs over {} commands, {total:.0} s of thread time; the slowest:",
        commands.len()
    );
    for ((id, state), secs) in slowest.iter().take(12) {
        println!("    {secs:7.1} s  {id} in state {state}");
    }
    assert!(runs > 20_000);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs one command in a fresh session in the given state: it must not
/// have panicked, and the session must still answer afterwards.
fn run_case(mut s: Session, state: usize, id: &str, params: &Value) {
    let before = (s.document_revision(), s.document_status());
    let outcome = s.execute(id, params);
    let shown = || params.to_string().chars().take(200).collect::<String>();
    match &outcome {
        Err(e) => {
            assert!(
                !e.message().contains("internal error"),
                "{id} panicked in state {state} on {}: {e}",
                shown()
            );
            assert!(!e.message().is_empty(), "{id}: empty error message");
            // A command that fails leaves the documents as they were
            // (`Session::execute`): nothing changed, so there is nothing
            // to look at again. (Looking anyway, with document.inspect,
            // was 97% of this test's time.)
            assert_eq!(
                (s.document_revision(), s.document_status()),
                before,
                "{id} failed in state {state} on {} but changed the document: {e}",
                shown()
            );
        }
        Ok(_) => {
            // The command did something: the session still answers, and
            // the document still regenerates and reports.
            if s.has_document() {
                let inspected = s.execute("document.inspect", &Value::Null);
                assert!(
                    inspected.is_ok(),
                    "document.inspect broke after {id} in state {state} on {}: {inspected:?}",
                    shown()
                );
            }
        }
    }
}

#[test]
fn the_parameter_names_are_read_from_the_registry() {
    assert_eq!(
        keys(r#"{"units": "um|mm" = "mm", "tolerance": number = 1e-6}"#),
        ["units", "tolerance"]
    );
    assert_eq!(keys("{}"), Vec::<String>::new());
    // Every documented parameter is one the command accepts: none is
    // answered with "unknown parameter".
    let dir = workdir("names");
    for spec in Session::commands() {
        for key in keys(spec.params) {
            let mut s = session(&dir, 2);
            if let Err(e) = s.execute(spec.id, &json!({ key.as_str(): Value::Null })) {
                assert!(
                    !e.message().contains("unknown parameter"),
                    "{}: `{key}` is documented but refused: {e}",
                    spec.id
                );
            }
        }
    }
}
