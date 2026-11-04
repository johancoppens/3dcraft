//! Command-level tests: everything goes through `Session::execute`, as it
//! does for the CLI, the control channel and MCP.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{error, run, session_in};
use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

/// A fresh directory under the test's temp dir, used as both roots.
fn workdir(name: &str) -> PathBuf {
    common::workdir("commands", name)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

/// A bracket-like part: a plate and a pin, driven by variables.
fn build(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    run(
        session,
        "variable.set",
        json!({"name": "width", "expr": "40 mm"}),
    );
    run(
        session,
        "variable.set",
        json!({"name": "thickness", "expr": 5}),
    );
    run(
        session,
        "feature.block",
        json!({"name": "Plate", "size": ["width", "width / 2", "thickness"]}),
    );
    run(
        session,
        "feature.cylinder",
        json!({"name": "Pin", "position": ["width / 2", 10, "thickness"], "radius": "width / 10", "height": "2 cm"}),
    );
}

fn plate_volume(width: f64) -> f64 {
    width * (width / 2.0) * 5.0
}

fn pin_volume(width: f64) -> f64 {
    PI * (width / 10.0).ppowi(2) * 20.0
}

#[test]
fn a_part_is_built_saved_reopened_changed_exported_and_rendered() {
    let dir = workdir("scenario");
    let mut session = session_in(&dir);
    build(&mut session);

    // Verify from JSON alone: statuses, counts, volumes by hand calculation.
    let state = run(&mut session, "document.inspect", json!({}));
    assert_eq!(state["ok"], true);
    assert_eq!(state["features"][0]["status"], "ok");
    assert_eq!(state["features"][1]["params"]["radius"], "width / 10");
    assert_eq!(
        state["variables"][0],
        json!({"name": "width", "expr": "40 mm", "value": 40.0, "dimension": "length", "lengthUnit": "mm"})
    );
    assert_eq!(
        (
            state["bodies"][0]["faces"].as_u64(),
            state["bodies"][0]["edges"].as_u64()
        ),
        (Some(6), Some(12))
    );
    assert!(close(
        state["bodies"][0]["volume"].as_f64().unwrap(),
        plate_volume(40.0)
    ));
    assert!(close(
        state["bodies"][1]["volume"].as_f64().unwrap(),
        pin_volume(40.0)
    ));
    assert_eq!(state["dirty"], true);

    // Save, close, reopen: the same model comes back.
    assert_eq!(
        run(
            &mut session,
            "file.save",
            json!({"path": "bracket.3dcraft"})
        )["path"],
        "bracket.3dcraft"
    );
    assert_eq!(
        run(&mut session, "document.inspect", json!({}))["dirty"],
        false
    );
    run(&mut session, "file.close", json!({}));
    assert!(error(&mut session, "document.inspect", json!({})).contains("no document is open"));
    assert_eq!(
        run(
            &mut session,
            "file.open",
            json!({"path": "bracket.3dcraft"})
        )["features"],
        2
    );
    let reopened = run(&mut session, "document.inspect", json!({}));
    for key in [
        "units",
        "tolerance",
        "variables",
        "features",
        "bodies",
        "rollback",
    ] {
        assert_eq!(
            reopened[key], state[key],
            "{key} changed across save and open"
        );
    }

    // Change one variable: both bodies follow.
    let width = run(
        &mut session,
        "variable.set",
        json!({"name": "width", "expr": "2 in"}),
    );
    assert!(close(width["value"].as_f64().unwrap(), 50.8));
    let measured = run(&mut session, "document.measure", json!({}));
    assert!(close(
        measured["volume"].as_f64().unwrap(),
        plate_volume(50.8) + pin_volume(50.8)
    ));
    assert_eq!(measured["volumeUnit"], "mm^3");
    let pin = run(&mut session, "document.measure", json!({"feature": "Pin"}));
    assert!(close(pin["volume"].as_f64().unwrap(), pin_volume(50.8)));
    let centroid = pin["centroid"].as_array().unwrap();
    assert!(
        close(centroid[0].as_f64().unwrap(), 25.4) && close(centroid[2].as_f64().unwrap(), 15.0)
    );

    // Undo brings the old size back; save with no path goes to the same file.
    run(&mut session, "edit.undo", json!({}));
    assert!(close(
        run(&mut session, "document.measure", json!({}))["volume"]
            .as_f64()
            .unwrap(),
        plate_volume(40.0) + pin_volume(40.0)
    ));
    run(&mut session, "edit.redo", json!({}));
    run(&mut session, "file.save", json!({}));

    // Export in every format; the files exist and say what they hold.
    for (file, magic) in [
        ("b.stl", &b"3DCraft binary STL"[..]),
        ("b.3mf", b"PK"),
        ("b.obj", b"# 3DCraft"),
        ("b.step", b"ISO-10303-21;"),
    ] {
        let result = run(&mut session, "file.export", json!({"path": file}));
        assert_eq!(result["bodies"], 2);
        let bytes = std::fs::read(dir.join(file)).unwrap();
        assert_eq!(result["bytes"].as_u64().unwrap() as usize, bytes.len());
        assert!(bytes.starts_with(magic), "{file}");
    }
    let ascii = run(
        &mut session,
        "file.export",
        json!({"path": "b.txt", "format": "stl-ascii", "tolerance": 0.5}),
    );
    assert_eq!(ascii["tolerance"], 0.5);
    assert!(
        std::fs::read_to_string(dir.join("b.txt"))
            .unwrap()
            .starts_with("solid Plate")
    );

    // Render to a file and inline.
    let shot = run(
        &mut session,
        "view.render",
        json!({"path": "shot.png", "width": 320, "height": 240}),
    );
    assert!(
        std::fs::read(dir.join("shot.png"))
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
    assert!(shot["coverage"].as_f64().unwrap() > 0.1 && shot.get("base64").is_none());
    let inline = run(
        &mut session,
        "view.render",
        json!({"view": "top", "mode": "ids", "width": 64, "height": 64}),
    );
    assert_eq!(inline["mime"], "image/png");
    assert!(
        inline["base64"]
            .as_str()
            .unwrap()
            .starts_with("iVBORw0KGgo"),
        "base64 of the PNG signature"
    );
    assert_eq!(
        inline["bodies"][1],
        json!({"body": "Pin", "firstFaceId": 6})
    );

    // No temporary files are left behind.
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n.to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn every_edit_is_one_undo_step_that_restores_the_document_exactly() {
    let edits = [
        (
            "variable.set",
            json!({"name": "depth", "expr": "width * 3"}),
        ),
        ("variable.set", json!({"name": "width", "expr": "1 m"})),
        ("variable.rename", json!({"name": "width", "to": "w"})),
        (
            "variable.delete",
            json!({"name": "thickness", "force": true}),
        ),
        ("feature.block", json!({"size": [1, 2, 3]})),
        ("feature.cylinder", json!({"radius": 1, "height": "width"})),
        (
            "feature.sphere",
            json!({"radius": "width / 4", "position": [0, 0, "width"]}),
        ),
        (
            "feature.edit",
            json!({"feature": "Pin", "radius": "3 mm", "position": [1, 2, 3]}),
        ),
        ("feature.edit", json!({"feature": "F1", "size": [9, 9, 9]})),
        (
            "feature.cone",
            json!({"radius": 3, "height": "width", "axis": [1, 0, 0]}),
        ),
        (
            "feature.torus",
            json!({"major": "width", "minor": 2, "angle": "30 deg"}),
        ),
        (
            "feature.edit",
            json!({"feature": "Pin", "axis": [0, 1, 0], "angle": 45}),
        ),
        ("feature.rename", json!({"feature": 1, "name": "Base"})),
        ("feature.suppress", json!({"feature": "Plate"})),
        ("feature.move", json!({"feature": "Pin", "position": 0})),
        ("feature.delete", json!({"feature": "Plate"})),
        ("history.rollback", json!({"feature": "Plate"})),
    ];
    for (id, params) in edits {
        let mut session = Session::new(Workspace::none());
        build(&mut session);
        let snapshot = |session: &mut Session| {
            let mut state = run(session, "document.inspect", json!({}));
            let undo = state["history"]["undo"].as_array().unwrap().len();
            state.as_object_mut().unwrap().remove("history");
            (state, undo)
        };
        let (before, steps) = snapshot(&mut session);
        run(&mut session, id, params.clone());
        let (after, steps_after) = snapshot(&mut session);
        assert_ne!(after, before, "{id} {params} changed nothing");
        assert_eq!(
            steps_after,
            steps + 1,
            "{id} must record exactly one undo step"
        );
        run(&mut session, "edit.undo", json!({}));
        assert_eq!(snapshot(&mut session).0, before, "undo of {id} {params}");
        run(&mut session, "edit.redo", json!({}));
        assert_eq!(snapshot(&mut session).0, after, "redo of {id} {params}");
    }
}

#[test]
fn commands_that_fail_change_nothing_and_say_why() {
    let mut session = Session::new(Workspace::none());
    build(&mut session);
    let before = run(&mut session, "document.inspect", json!({}));
    let cases = [
        ("feature.blok", json!({}), "did you mean `feature.block`"),
        ("nonsense", json!({}), "unknown command `nonsense`"),
        (
            "feature.block",
            json!({"size": [1, 2, 3], "colour": "red"}),
            "unknown parameter `colour`",
        ),
        ("feature.block", json!({}), "missing parameter `size`"),
        (
            "feature.block",
            json!({"size": [1, 2]}),
            "`size` must be an array of three expressions",
        ),
        (
            "feature.block",
            json!({"size": "big"}),
            "`size` must be an array of three expressions",
        ),
        (
            "feature.block",
            json!([1, 2, 3]),
            "params must be a JSON object, got an array",
        ),
        (
            "feature.block",
            json!({"size": [1, 2, 3], "name": "Pin"}),
            "a feature named `Pin` already exists",
        ),
        (
            "feature.cylinder",
            json!({"radius": true, "height": 1}),
            "`radius` must be an expression",
        ),
        (
            "feature.edit",
            json!({"feature": "Plate", "radius": 2}),
            "`radius` is not a parameter of a block; its parameters are: position, size",
        ),
        (
            "feature.edit",
            json!({"feature": "Plate"}),
            "give at least one parameter",
        ),
        (
            "feature.edit",
            json!({"feature": "Nope", "size": [1, 1, 1]}),
            "there is no feature named `Nope`; features: Plate, Pin",
        ),
        (
            "feature.delete",
            json!({"feature": 99}),
            "there is no feature F99",
        ),
        (
            "feature.delete",
            json!({"feature": "F99"}),
            "there is no feature F99",
        ),
        (
            "feature.move",
            json!({"feature": "Pin", "position": 5}),
            "position 5 is past the end of the 2 features",
        ),
        (
            "feature.move",
            json!({"feature": "Pin", "position": -1}),
            "`position` must be a whole number",
        ),
        ("variable.set", json!({"name": "sin", "expr": "1"}), "sin"),
        (
            "variable.set",
            json!({"name": "x"}),
            "missing parameter `expr`",
        ),
        ("variable.delete", json!({"name": "nope"}), "nope"),
        (
            "variable.rename",
            json!({"name": "width", "to": "thickness"}),
            "thickness",
        ),
        ("history.rollback", json!({}), "missing parameter `feature`"),
        ("edit.redo", json!({}), "nothing to redo"),
        (
            "edit.undo",
            json!({"steps": 2}),
            "this command takes no parameters",
        ),
        (
            "file.new",
            json!({"units": "furlong"}),
            "unknown length unit `furlong`",
        ),
        (
            "file.new",
            json!({"tolerance": 0}),
            "`tolerance` must be between 1e-12 and 1e3 mm",
        ),
        ("file.new", json!({"tolerance": 1e-300}), "got 1e-300"),
        (
            "file.new",
            json!({"units": "in", "tolerance": 1e4}),
            "`tolerance` must be between 1e-12 and 1e3 in",
        ),
        (
            "file.export",
            json!({"path": "part.dwg"}),
            "cannot tell the format from `part.dwg`",
        ),
        (
            "file.export",
            json!({"path": "part.stl", "format": "dwg"}),
            "unknown format `dwg`",
        ),
        (
            "view.render",
            json!({"view": "sideways"}),
            "unknown view `sideways`",
        ),
        (
            "view.render",
            json!({"width": 100000}),
            "`width` must be a whole number from 16 to 4096",
        ),
        (
            "view.render",
            json!({"tolerance": -1}),
            "`tolerance` must be positive",
        ),
        (
            "document.measure",
            json!({"feature": "Ghost"}),
            "there is no feature named `Ghost`",
        ),
    ];
    for (id, params, expected) in cases {
        let message = error(&mut session, id, params.clone());
        assert!(
            message.contains(expected),
            "{id} {params}: `{message}` should contain `{expected}`"
        );
    }
    assert_eq!(
        run(&mut session, "document.inspect", json!({})),
        before,
        "a failed command changed the document"
    );
}

#[test]
fn a_feature_that_cannot_be_built_is_reported_not_refused() {
    let mut session = Session::new(Workspace::none());
    build(&mut session);
    // The command succeeds: the recipe is stored. The result says it failed.
    let result = run(
        &mut session,
        "feature.block",
        json!({"name": "Bad", "size": ["width", "missing_variable", 1]}),
    );
    assert_eq!(result["status"], "failed");
    assert!(
        result["error"].as_str().unwrap().contains("size.y"),
        "{result}"
    );
    let state = run(&mut session, "document.inspect", json!({}));
    assert_eq!(state["ok"], false);
    assert_eq!(
        state["bodies"].as_array().unwrap().len(),
        2,
        "the other bodies are still there"
    );
    assert!(
        error(&mut session, "document.measure", json!({"feature": "Bad"}))
            .contains("has no body (its status is failed)")
    );
    // Fixing the parameter fixes the feature.
    assert_eq!(
        run(
            &mut session,
            "feature.edit",
            json!({"feature": "Bad", "size": [1, 1, 1]})
        )["status"],
        "ok"
    );
    // A suppressed or rolled-back feature has no body either.
    assert_eq!(
        run(&mut session, "feature.suppress", json!({"feature": "Bad"}))["status"],
        "suppressed"
    );
    assert_eq!(
        run(
            &mut session,
            "feature.suppress",
            json!({"feature": "Bad", "suppressed": false})
        )["status"],
        "ok"
    );
    run(
        &mut session,
        "history.rollback",
        json!({"feature": "Plate"}),
    );
    assert_eq!(
        run(&mut session, "document.inspect", json!({}))["features"][1]["status"],
        "rolledBack"
    );
    assert_eq!(
        run(&mut session, "history.rollback", json!({"feature": null}))["rollback"],
        Value::Null
    );
}

#[test]
fn files_stay_inside_the_roots() {
    let dir = workdir("roots");
    let mut session = session_in(&dir);
    build(&mut session);
    for path in [
        "../escape.3dcraft",
        "/tmp/escape.3dcraft",
        "a/../../b.3dcraft",
        "",
    ] {
        assert!(
            session
                .execute("file.save", &json!({"path": path}))
                .is_err(),
            "`{path}`"
        );
        // An escaping directory with a file name appended is refused too.
        // (Not for the empty path: "" + "x.stl" is a perfectly good name.)
        if !path.is_empty() {
            let escaped = format!("{path}/x.stl");
            assert!(
                session
                    .execute("file.export", &json!({"path": escaped}))
                    .is_err(),
                "`{path}`"
            );
        }
        assert!(
            session
                .execute("file.open", &json!({"path": path}))
                .is_err(),
            "`{path}`"
        );
    }
    assert!(
        error(
            &mut session,
            "file.open",
            json!({"path": "missing.3dcraft"})
        )
        .contains("cannot open `missing.3dcraft`")
    );
    // Said once, in plain words: not the path twice and the system's
    // `No such file or directory (os error 2)`.
    let missing = error(
        &mut session,
        "file.open",
        json!({"path": "missing.3dcraft"}),
    );
    assert!(missing.contains("there is no such file"), "{missing}");
    assert_eq!(missing.matches("missing.3dcraft").count(), 1, "{missing}");
    assert!(error(&mut session, "file.save", json!({})).contains("give a `path`"));
    // Saving into a directory that does not exist fails cleanly.
    assert!(
        error(
            &mut session,
            "file.save",
            json!({"path": "no/such/dir/a.3dcraft"})
        )
        .contains("cannot save")
    );
    let no_folder = error(
        &mut session,
        "file.save",
        json!({"path": "no/such/dir/a.3dcraft"}),
    );
    assert!(
        no_folder.contains("its folder does not exist"),
        "{no_folder}"
    );
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "nothing was written"
    );
    // Not a .3dcraft file.
    std::fs::write(dir.join("junk.3dcraft"), b"not a zip").unwrap();
    assert!(
        error(&mut session, "file.open", json!({"path": "junk.3dcraft"})).contains("cannot open")
    );

    // Without roots there is no file access at all, and the error says how to get it.
    let mut sealed = Session::new(Workspace::none());
    build(&mut sealed);
    assert!(
        error(&mut sealed, "file.save", json!({"path": "a.3dcraft"})).contains("no write access")
    );
    assert!(
        error(&mut sealed, "file.open", json!({"path": "a.3dcraft"})).contains("no read access")
    );
    assert!(
        error(&mut sealed, "view.render", json!({"path": "a.png"})).contains("no write access")
    );
    // Rendering inline needs no file access.
    assert!(
        sealed
            .execute("view.render", &json!({"width": 32, "height": 32}))
            .is_ok()
    );
}

#[test]
fn closing_protects_unsaved_work() {
    let dir = workdir("close");
    let mut session = session_in(&dir);
    run(&mut session, "file.new", json!({}));
    run(&mut session, "file.close", json!({}));
    build(&mut session);
    assert!(error(&mut session, "file.close", json!({})).contains("unsaved changes"));
    run(&mut session, "file.save", json!({"path": "a.3dcraft"}));
    run(
        &mut session,
        "variable.set",
        json!({"name": "width", "expr": "1 mm"}),
    );
    assert!(error(&mut session, "file.close", json!({})).contains("unsaved changes"));
    assert_eq!(
        run(&mut session, "file.close", json!({"discard": true}))["open"],
        0
    );
}

#[test]
fn the_registry_describes_every_command() {
    let commands = Session::commands();
    assert!(commands.len() >= 20);
    let mut ids: Vec<&str> = commands.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), commands.len(), "duplicate command id");
    for c in commands {
        assert!(c.id.contains('.') && c.id.is_ascii(), "{}", c.id);
        assert!(!c.label.is_empty() && !c.summary.is_empty(), "{}", c.id);
        assert!(
            c.params.starts_with('{') && c.params.ends_with('}'),
            "{}: {}",
            c.id,
            c.params
        );
    }
    let empty = Session::new(Workspace::none());
    let needs = commands
        .iter()
        .filter(|c| empty.disabled_reason(c).is_some())
        .count();
    assert_eq!(
        needs,
        commands.len() - 10,
        "all but file.new, assembly.new, file.open, file.list, the three recovery commands, session.report, material.presets and font.list need a document"
    );
}

#[test]
fn cones_tori_and_orientations_are_modelled_and_measured() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({}));
    assert_eq!(
        run(
            &mut session,
            "feature.cone",
            json!({"radius": 3, "height": 9})
        )["name"],
        "Cone1"
    );
    run(
        &mut session,
        "feature.torus",
        json!({"major": 10, "minor": 2, "position": [0, 0, 20]}),
    );
    // A cylinder lying along +x from the point (5, 0, 0).
    run(
        &mut session,
        "feature.cylinder",
        json!({"name": "Axle", "radius": 1, "height": 8, "position": [5, 0, 0], "axis": [2, 0, 0]}),
    );
    let measured = run(&mut session, "document.measure", json!({}));
    let volume = |i: usize| measured["bodies"][i]["volume"].as_f64().unwrap();
    assert!(close(volume(0), PI * 9.0 * 9.0 / 3.0));
    assert!(close(volume(1), 2.0 * PI * PI * 10.0 * 4.0));
    assert!(close(volume(2), PI * 8.0));
    let centroid = measured["bodies"][2]["centroid"].as_array().unwrap();
    assert!(
        close(centroid[0].as_f64().unwrap(), 9.0) && centroid[1].as_f64().unwrap().abs() < 1e-9
    );

    // Turning it about its own axis changes nothing measurable; pointing it
    // along +y moves the centroid there.
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Axle", "angle": "33 deg"}),
    );
    let turned = run(&mut session, "document.measure", json!({"feature": "Axle"}));
    assert!(close(turned["centroid"][0].as_f64().unwrap(), 9.0));
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Axle", "axis": [0, 1, 0]}),
    );
    let moved = run(&mut session, "document.measure", json!({"feature": "Axle"}));
    assert!(
        close(moved["centroid"][0].as_f64().unwrap(), 5.0)
            && close(moved["centroid"][1].as_f64().unwrap(), 4.0)
    );
    let state = run(&mut session, "document.inspect", json!({}));
    assert_eq!(
        state["features"][2]["params"]["orientation"]["axis"],
        json!(["0", "1", "0"])
    );

    // Parameters that do not belong to a kind are refused by name.
    let message = error(
        &mut session,
        "feature.edit",
        json!({"feature": "Cone1", "major": 3}),
    );
    assert!(
        message.contains("`major` is not a parameter of a cone"),
        "{message}"
    );
    let message = error(
        &mut session,
        "feature.edit",
        json!({"feature": "Torus1", "radius": 3}),
    );
    assert!(
        message.contains("its parameters are: position, major, minor, axis, angle"),
        "{message}"
    );
    let message = error(
        &mut session,
        "feature.sphere",
        json!({"radius": 1, "axis": [1, 0, 0]}),
    );
    assert!(message.contains("unknown parameter `axis`"), "{message}");
    // A failed orientation is a failed feature with the reason.
    let bad = run(
        &mut session,
        "feature.cone",
        json!({"radius": 1, "height": 1, "axis": [0, 0, 0]}),
    );
    assert_eq!(bad["status"], "failed");
    assert!(
        bad["error"]
            .as_str()
            .unwrap()
            .contains("axis must be a direction")
    );
}

#[test]
fn inspect_reports_a_bounding_box_per_body() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [4, 2, 1], "position": [10, 20, 30]}),
    );
    run(&mut session, "feature.sphere", json!({"radius": 5}));
    let state = run(&mut session, "document.inspect", json!({}));
    // Exact for a block.
    assert_eq!(
        state["bodies"][0]["bbox"],
        json!([[10.0, 20.0, 30.0], [14.0, 22.0, 31.0]])
    );
    // Within the stated tolerance, and never outside, for a sphere.
    let tolerance = state["bodies"][1]["bboxTolerance"].as_f64().unwrap();
    for (corner, sign) in [(0, -1.0), (1, 1.0)] {
        for axis in 0..3 {
            let value = state["bodies"][1]["bbox"][corner][axis].as_f64().unwrap() * sign;
            assert!(
                value <= 5.0 + 1e-12 && value >= 5.0 - tolerance,
                "{value} vs 5 +/- {tolerance}"
            );
        }
    }
}

#[test]
fn check_lists_every_problem_or_says_ok() {
    let mut session = Session::new(Workspace::none());
    build(&mut session);
    assert_eq!(
        run(&mut session, "document.check", json!({})),
        json!({"ok": true, "bodiesChecked": 2, "problems": []})
    );
    run(
        &mut session,
        "variable.set",
        json!({"name": "loop_a", "expr": "loop_b"}),
    );
    run(
        &mut session,
        "variable.set",
        json!({"name": "loop_b", "expr": "loop_a"}),
    );
    run(
        &mut session,
        "feature.sphere",
        json!({"name": "Ball", "radius": "loop_a"}),
    );
    let report = run(&mut session, "document.check", json!({}));
    assert_eq!(report["ok"], false);
    assert_eq!(report["bodiesChecked"], 2);
    let problems = report["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 3, "{report}");
    assert_eq!(problems[0]["variable"], "loop_a");
    assert_eq!(
        (
            problems[2]["feature"].as_str(),
            problems[2]["name"].as_str()
        ),
        (Some("F3"), Some("Ball"))
    );
    assert!(problems[2]["problem"].as_str().unwrap().contains("radius"));
    assert!(
        error(&mut session, "document.check", json!({"thorough": true}))
            .contains("unknown parameter `thorough`")
    );
}

/// A file saved before 2026-10-15 may hold a tolerance outside the range
/// a document may have: `file.open` reads it as the nearer end and says
/// so under `warnings`; a file within the range opens without any.
#[test]
fn file_open_says_when_it_brought_the_tolerance_into_range() {
    let dir = workdir("tolerance-range");
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "in"}));
    run(&mut s, "feature.block", json!({"size": [1, 2, 3]}));
    run(&mut s, "file.save", json!({"path": "inside.3dcraft"}));
    let opened = run(&mut s, "file.open", json!({"path": "inside.3dcraft"}));
    assert!(opened.get("warnings").is_none(), "{opened}");
    // The same document with the tolerance an old build let through.
    let bytes = std::fs::read(dir.join("inside.3dcraft")).unwrap();
    let entries = craft3d_zipstore::read_stored(&bytes).unwrap();
    let manifest = entries.iter().find(|e| e.name == "manifest.json").unwrap();
    let mut value: Value = serde_json::from_slice(manifest.data).unwrap();
    for (written, read) in [(1e-13, "1e-12"), (5e4, "1e3")] {
        value["document"]["tolerance"]["linear"] = written.into();
        let text = serde_json::to_vec(&value).unwrap();
        let zipped = craft3d_zipstore::write_stored(&[("manifest.json", text.as_slice())]).unwrap();
        std::fs::write(dir.join("outside.3dcraft"), zipped).unwrap();
        let opened = run(&mut s, "file.open", json!({"path": "outside.3dcraft"}));
        let warnings = opened["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1, "{opened}");
        let warning = warnings[0].as_str().unwrap();
        assert!(
            warning.contains(&format!("{written:e} in read as {read} in"))
                && warning.contains("outside 1e-12..1e3"),
            "{warning}"
        );
        let inspected = run(&mut s, "document.inspect", json!({}));
        assert_eq!(
            inspected["tolerance"]["linear"].as_f64().unwrap(),
            read.parse::<f64>().unwrap()
        );
    }
}

/// `document.measure` with `bound` gives the integrator's own bound on the
/// volume and the area, per body and summed; the closed form lies within
/// it, and a bound that is not a boolean is an error.
#[test]
fn measure_gives_its_bound_when_asked() {
    for s in [0.01, 1.0, 1000.0] {
        let mut session = Session::new(Workspace::unrestricted());
        session
            .execute("file.new", &json!({"units": "mm"}))
            .unwrap();
        let mm = |x: f64| format!("{} mm", x * s);
        session
            .execute(
                "feature.cylinder",
                &json!({"radius": mm(3.0), "height": mm(5.0), "name": "Pin"}),
            )
            .unwrap();
        session
            .execute(
                "feature.block",
                &json!({"size": [mm(2.0), mm(3.0), mm(4.0)], "position": [mm(10.0), mm(0.0), mm(0.0)], "name": "Box"}),
            )
            .unwrap();
        let plain = session.execute("document.measure", &json!({})).unwrap();
        assert!(plain.get("volumeBound").is_none());
        let m = session
            .execute("document.measure", &json!({"bound": true}))
            .unwrap();
        assert_eq!(m["volume"], plain["volume"], "the very same volume");
        let bodies = m["bodies"].as_array().unwrap();
        let mut sum = 0.0;
        for body in bodies {
            let bound = body["volumeBound"].as_f64().unwrap();
            assert!(bound.is_finite() && bound >= 0.0, "{body}");
            assert!(body["areaBound"].as_f64().unwrap() >= 0.0);
            sum += bound;
        }
        assert_eq!(m["volumeBound"].as_f64().unwrap(), sum);
        let pin = &bodies[0];
        let exact = PI * 9.0 * 5.0 * s.ppowi(3);
        let off = (pin["volume"].as_f64().unwrap() - exact).abs();
        let bound = pin["volumeBound"].as_f64().unwrap();
        // The bound is an estimate: the closed form within it, and the
        // bound itself small (1e-9 of the volume).
        assert!(
            off <= bound + 1e-15 * exact,
            "scale {s}: off {off}, bound {bound}"
        );
        assert!(bound <= 1e-9 * exact, "scale {s}: bound {bound}");
        for bad in [json!({"bound": 1}), json!({"bound": "yes"})] {
            assert!(session.execute("document.measure", &bad).is_err(), "{bad}");
        }
    }
}

#[test]
fn file_new_takes_the_cap_on_entity_tolerances() {
    let mut s = Session::new(Workspace {
        read: craft3d_engine::Access::None,
        write: craft3d_engine::Access::None,
    });
    run(&mut s, "file.new", json!({"tolerance": 1e-5}));
    let inspected = run(&mut s, "document.inspect", json!({}));
    // Absent when the document sets none (1000 times the tolerance).
    assert!(inspected["tolerance"].get("loosest").is_none());
    run(
        &mut s,
        "file.new",
        json!({"tolerance": 1e-5, "loosest": 3e-4}),
    );
    let inspected = run(&mut s, "document.inspect", json!({}));
    assert_eq!(inspected["tolerance"]["loosest"].as_f64(), Some(3e-4));
    for bad in [json!(1e-6), json!(-1.0), json!("wide")] {
        let message = error(
            &mut s,
            "file.new",
            json!({"tolerance": 1e-5, "loosest": bad}),
        );
        assert!(message.contains("loosest"), "{bad}: {message}");
    }
}
