//! A sketch's solution does not depend on the path of edits (the welding
//! table's finding W1, docs/design/document.md › *Carrying a sketch along
//! when its images jump*): geometry after a chained run follows the run's
//! end when the count jumps, a model returns to where it was when its
//! variables do, and every edited session equals a fresh open of its saved
//! file, bit for bit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("craft3d-edit-path-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn session(dir: &Path) -> Session {
    Session::new(Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    })
}

/// A session with the steps of a fixture run.
fn replay(dir: &Path, steps: &str) -> Session {
    let steps: Vec<(String, Value)> = serde_json::from_str(steps).unwrap();
    let mut s = session(dir);
    for (id, params) in steps {
        run(&mut s, &id, params);
    }
    s
}

fn set(s: &mut Session, values: &[(&str, &str)]) {
    for (name, expr) in values {
        run(s, "variable.set", json!({"name": name, "expr": expr}));
    }
}

/// Everything a document shows: every feature's status and error, every
/// body's volume and area, and every sketch as `sketch.inspect` gives it
/// (the coordinates of every point, image points included).
fn state(s: &mut Session) -> Value {
    let doc = run(s, "document.inspect", json!({}));
    let mut sketches = Vec::new();
    for feature in doc["features"].as_array().unwrap() {
        let name = feature["name"].as_str().unwrap();
        if let Ok(sketch) = s.execute("sketch.inspect", &json!({"sketch": name})) {
            sketches.push(sketch);
        }
    }
    let features: Vec<Value> = doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| json!([f["name"], f["status"], f.get("error")]))
        .collect();
    let bodies: Vec<Value> = doc["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| json!([b["name"], b["volume"], b["area"]]))
        .collect();
    json!({"features": features, "bodies": bodies, "sketches": sketches})
}

/// Saves the session's document, opens it in a new session and compares:
/// the same state, bit for bit.
fn equals_a_fresh_open(s: &mut Session, dir: &Path, file: &str) -> Value {
    let here = state(s);
    run(s, "file.save", json!({"path": file, "overwrite": true}));
    // What the session stores is in the form a file must have: no image
    // place had to be dropped on reading it.
    let (_, notes) = craft3d_format::load_noted(&dir.join(file)).unwrap();
    assert!(notes.is_empty(), "{file}: {notes:?}");
    // Nor was any sketch carried only part of the way.
    for sketch in here["sketches"].as_array().unwrap() {
        assert!(
            sketch.get("warnings").is_none(),
            "{file}: {}",
            sketch["warnings"]
        );
    }
    let mut fresh = session(dir);
    run(&mut fresh, "file.open", json!({"path": file}));
    let there = state(&mut fresh);
    assert!(
        here == there,
        "{file}: the edited session and a fresh open differ"
    );
    // The bodies themselves, exactly: STEP writes every point, curve and
    // surface of the B-rep with all its digits (the same file name, so the
    // header is the same too).
    assert!(
        step(s, dir) == step(&mut fresh, dir),
        "{file}: the STEP of the edited session and of a fresh open differ"
    );
    here
}

/// The STEP file of every body, or the error when there are none.
fn step(s: &mut Session, dir: &Path) -> Result<Vec<u8>, String> {
    s.execute("file.export", &json!({"path": "bodies.step"}))
        .map_err(|e| e.to_string())?;
    Ok(std::fs::read(dir.join("bodies.step")).unwrap())
}

/// The largest difference between the coordinates of the same point in
/// two states, relative to the size of the sketches (the largest
/// coordinate, at least 1).
fn largest_move(a: &Value, b: &Value) -> f64 {
    let points = |s: &Value| -> Vec<(String, f64, f64)> {
        s["sketches"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|sk| {
                let name = sk["name"].as_str().unwrap().to_string();
                sk["entities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(move |e| {
                        let (x, y) = (e["x"].as_f64()?, e["y"].as_f64()?);
                        Some((format!("{name}/{}", e["id"]), x, y))
                    })
            })
            .collect()
    };
    let (pa, pb) = (points(a), points(b));
    assert_eq!(
        pa.iter().map(|p| &p.0).collect::<Vec<_>>(),
        pb.iter().map(|p| &p.0).collect::<Vec<_>>(),
        "the same points"
    );
    let size = pa
        .iter()
        .fold(1.0f64, |m, p| m.max(p.1.abs()).max(p.2.abs()));
    pa.iter().zip(&pb).fold(0.0f64, |m, (p, q)| {
        m.max((p.1 - q.1).abs()).max((p.2 - q.2).abs())
    }) / size
}

/// A width, a depth and the volumes of the table's bodies there.
type Size = (&'static str, &'static str, [(&'static str, f64); 3]);

fn failed(state: &Value) -> Vec<Value> {
    state["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f[1] != "ok")
        .cloned()
        .collect()
}

fn volumes(state: &Value) -> Vec<(String, f64)> {
    state["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| (b[0].as_str().unwrap().to_string(), b[1].as_f64().unwrap()))
        .collect()
}

fn close<S: AsRef<str>>(got: &[(String, f64)], want: &[(S, f64)], rel: f64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?}");
    for ((name, v), (wname, w)) in got.iter().zip(want) {
        assert_eq!(name, wname.as_ref(), "{what}");
        assert!((v - w).abs() <= rel * w.abs(), "{what}: {name} {v} vs {w}");
    }
}

fn point(state: &Value, sketch: &str, id: &str) -> [f64; 2] {
    let sketch = state["sketches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == sketch)
        .unwrap();
    let e = sketch["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .unwrap();
    [e["x"].as_f64().unwrap(), e["y"].as_f64().unwrap()]
}

/// W1: a square wave of `n` periods of 10, a line one unit long after the
/// run's last instance, then down 5 and back to the start. The count goes
/// 3 -> 10 -> 3: the point after the run is at `10 n + 1` every time
/// (it was 99 for n = 10, the profile folded back), the body is a
/// closed-form area, and each state is what a fresh open of it gives.
#[test]
fn geometry_after_a_run_follows_its_end_through_count_changes() {
    let dir = dir("w1");
    let mut s = replay(&dir, include_str!("edit_path/w1-run-end.steps.json"));
    let start = equals_a_fresh_open(&mut s, &dir, "w1-3.3dcraft");
    for n in [10u32, 3, 400, 1, 3] {
        set(&mut s, &[("n", &n.to_string())]);
        let now = equals_a_fresh_open(&mut s, &dir, &format!("w1-{n}.3dcraft"));
        assert!(failed(&now).is_empty(), "n = {n}: {:?}", failed(&now));
        let x = f64::from(10 * n + 1);
        for id in ["e11", "e13"] {
            let [px, _] = point(&now, "Edge", id);
            assert!(
                (px - x).abs() <= 1e-9 * x,
                "n = {n}: {id} at {px}, want {x}"
            );
        }
        // 1 mm thick: the rectangle (10 n + 1) x 5 and n periods of
        // 5 x 2 above it.
        let want = 5.0 * x + f64::from(n) * 10.0;
        close(
            &volumes(&now),
            &[("Body", want)],
            1e-12,
            &format!("n = {n}"),
        );
    }
    let back = state(&mut s);
    close(
        &volumes(&back),
        &[("Body", volumes(&start)[0].1)],
        1e-12,
        "back at 3",
    );
    let moved = largest_move(&start, &back);
    eprintln!("w1: back at 3, the points moved by {moved:e} of the sketch's size");
    assert!(moved <= 1e-12, "back at 3: the points moved by {moved:e}");
}

/// The welding table's first model: ribs whose runs end in a line from
/// the run's last instance. 48 x 36 -> 30 x 30 -> 96 x 48 -> 48 x 36:
/// every state builds, its volumes are the closed forms of the table's own
/// generator (`target/work/weldtable/findings/W1-model-v1.py`), and each
/// equals a fresh open. (96 x 48 folded the x-rib, and the return to
/// 48 x 36 left the y-rib's sketch failing, "c60 conflicts".)
#[test]
fn the_welding_table_goes_through_sizes_and_comes_back() {
    let dir = dir("weld");
    let mut s = replay(&dir, include_str!("edit_path/weldtable-v1.steps.json"));
    let start = equals_a_fresh_open(&mut s, &dir, "weld-48x36.3dcraft");
    assert!(failed(&start).is_empty(), "{:?}", failed(&start));
    // The closed forms (`oracle()` of the generator), the plate's to the
    // 1e-9 of its offset arcs' approximation in the oracle.
    let sizes: [Size; 3] = [
        (
            "30 in",
            "30 in",
            [
                ("Plate", 203.634538803),
                ("XRibs", 171.858793709),
                ("YRibs", 171.858793709),
            ],
        ),
        (
            "96 in",
            "48 in",
            [
                ("Plate", 1031.910010471),
                ("XRibs", 1027.061860794),
                ("YRibs", 960.177843974),
            ],
        ),
        (
            "48 in",
            "36 in",
            [
                ("Plate", 388.943000203),
                ("XRibs", 360.066691490),
                ("YRibs", 343.345687285),
            ],
        ),
    ];
    for (w, d, want) in sizes {
        set(&mut s, &[("W", w), ("D", d)]);
        let now = equals_a_fresh_open(&mut s, &dir, &format!("weld-{w}x{d}.3dcraft"));
        assert!(failed(&now).is_empty(), "{w} x {d}: {:?}", failed(&now));
        close(&volumes(&now), &want, 1e-8, &format!("{w} x {d}"));
    }
    let back = state(&mut s);
    close(&volumes(&back), &volumes(&start), 1e-12, "back at 48 x 36");
    let moved = largest_move(&start, &back);
    eprintln!("weldtable: back at 48 x 36, the points moved by {moved:e} of the sketches' size");
    assert!(
        moved <= 1e-12,
        "back at 48 x 36: the points moved by {moved:e}"
    );
}

/// The laser box with kerf as an offset of each panel's nominal outline
/// (`target/work/laserbox/kerf2`): its finger counts are formulas, so a
/// new size changes every run's count. 160 x 110 x 80 -> 240 x 200 x 60
/// -> back, one variable at a time: every state equals a fresh open, the
/// big box equals the box built at that size from the start, and the way
/// back returns the volumes it started with.
#[test]
fn the_laser_box_goes_through_count_changes() {
    let dir = dir("laserbox");
    let steps = include_str!("edit_path/laserbox-kerf.steps.json");
    let mut s = replay(&dir, steps);
    let start = equals_a_fresh_open(&mut s, &dir, "box-1.3dcraft");
    assert!(failed(&start).is_empty(), "{:?}", failed(&start));
    let big = [
        ("width", "240 mm"),
        ("depth", "200 mm"),
        ("height", "60 mm"),
        ("thick", "6 mm"),
        ("fingerW", "7 mm"),
    ];
    let small = [
        ("width", "160 mm"),
        ("depth", "110 mm"),
        ("height", "80 mm"),
        ("thick", "3 mm"),
        ("fingerW", "12 mm"),
    ];
    set(&mut s, &big);
    let edited = equals_a_fresh_open(&mut s, &dir, "box-2.3dcraft");
    assert!(failed(&edited).is_empty(), "{:?}", failed(&edited));
    // The box built at that size from the start (`box.py`, drawn at its
    // own coordinates; kerf2.json, where it matches the block-built box
    // and the closed forms of its round corners).
    let built = [
        ("Front", 71304.31911627609),
        ("Back", 72439.06636589413),
        ("Left", 59198.64032570596),
        ("Right", 59198.64032570596),
        ("Bottom", 273367.8308307064),
        ("Lid", 286894.6328708496),
    ];
    close(&volumes(&edited), &built, 1e-9, "edited to the big box");

    set(&mut s, &small);
    let back = equals_a_fresh_open(&mut s, &dir, "box-1-again.3dcraft");
    close(
        &volumes(&back),
        &volumes(&start),
        1e-12,
        "back at the small box",
    );
    let moved = largest_move(&start, &back);
    eprintln!(
        "laser box: back at the small box, the points moved by {moved:e} of the sketches' size"
    );
    assert!(
        moved <= 1e-12,
        "back at the small box: the points moved by {moved:e}"
    );
}

/// A file from before image places were stored (format 19: no
/// `image_places`) is given them from its solution before its first edit,
/// so the count's first jump after the open follows the run's end as well.
#[test]
fn a_file_without_image_places_is_carried_from_its_first_edit() {
    let dir = dir("w1-old");
    let mut s = replay(&dir, include_str!("edit_path/w1-run-end.steps.json"));
    run(&mut s, "file.save", json!({"path": "w1-new.3dcraft"}));
    // As a version-19 file has it: the same JSON without the places.
    let doc = craft3d_format::load(&dir.join("w1-new.3dcraft")).unwrap();
    let json = craft3d_format::to_manifest_json(&doc).unwrap();
    assert!(json.contains("image_places"), "the session stores them");
    let mut value: Value = serde_json::from_str(&json).unwrap();
    value["format_version"] = json!(19);
    for feature in value["document"]["features"].as_array_mut().unwrap() {
        if let Some(kind) = feature["kind"].as_object_mut() {
            kind.remove("image_places");
        }
    }
    let old = craft3d_format::from_manifest_json(value.to_string().as_bytes()).unwrap();
    craft3d_format::save(&dir.join("w1-old.3dcraft"), &old).unwrap();

    let mut s = session(&dir);
    run(&mut s, "file.open", json!({"path": "w1-old.3dcraft"}));
    for n in [10u32, 3] {
        set(&mut s, &[("n", &n.to_string())]);
        let now = equals_a_fresh_open(&mut s, &dir, &format!("w1-old-{n}.3dcraft"));
        assert!(failed(&now).is_empty(), "n = {n}: {:?}", failed(&now));
        let x = f64::from(10 * n + 1);
        let [px, _] = point(&now, "Edge", "e11");
        assert!((px - x).abs() <= 1e-9 * x, "n = {n}: e11 at {px}, want {x}");
    }
}

/// Undo and redo of a count that jumped, and an edit of the sketch while
/// image places are stored: every state equals a fresh open of its file,
/// and the point after the run is where the count says.
#[test]
fn undo_redo_and_deletions_keep_the_session_equal_to_its_file() {
    let dir = dir("w1-undo");
    let mut s = replay(&dir, include_str!("edit_path/w1-run-end.steps.json"));
    let at = |state: &Value| point(state, "Edge", "e11")[0];
    set(&mut s, &[("n", "10")]);
    let up = equals_a_fresh_open(&mut s, &dir, "undo-10.3dcraft");
    assert_eq!(at(&up), 101.0);
    run(&mut s, "edit.undo", json!({}));
    let undone = equals_a_fresh_open(&mut s, &dir, "undo-3.3dcraft");
    assert_eq!(at(&undone), 31.0);
    run(&mut s, "edit.redo", json!({}));
    let redone = equals_a_fresh_open(&mut s, &dir, "redo-10.3dcraft");
    assert!(up == redone, "redo gives the state the edit gave");
    // From the undone state the same edit lands where it did.
    run(&mut s, "edit.undo", json!({}));
    set(&mut s, &[("n", "10")]);
    let again = equals_a_fresh_open(&mut s, &dir, "again-10.3dcraft");
    assert_eq!(at(&again), 101.0);

    // The line on the run's end deleted (its image point stays, with its
    // place), then a line of the seed: whatever that leaves of the run and
    // its image points, the places in the session are those a file holds.
    // Then the count changed, and all of it undone.
    for (entity, file) in [("e12", "no-e12"), ("e3", "seed-less")] {
        run(
            &mut s,
            "sketch.delete",
            json!({"sketch": "Edge", "entity": entity}),
        );
        equals_a_fresh_open(&mut s, &dir, &format!("{file}.3dcraft"));
    }
    set(&mut s, &[("n", "4")]);
    equals_a_fresh_open(&mut s, &dir, "seed-less-4.3dcraft");
    for _ in 0..3 {
        run(&mut s, "edit.undo", json!({}));
    }
    let back = equals_a_fresh_open(&mut s, &dir, "seed-back.3dcraft");
    assert!(
        back == again,
        "three undos give the state before the deletions"
    );

    // The pattern deleted with its image points: their places go with
    // them.
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Edge", "entity": "e12"}),
    );
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Edge", "pattern": "p1"}),
    );
    equals_a_fresh_open(&mut s, &dir, "no-pattern.3dcraft");
    set(&mut s, &[("n", "3")]);
    equals_a_fresh_open(&mut s, &dir, "no-pattern-3.3dcraft");
}

/// A version-19 file's first edit that changes nothing stays one that
/// changes nothing: no undo step, the document unchanged.
#[test]
fn a_no_op_edit_of_an_old_file_changes_nothing() {
    let dir = dir("w1-noop");
    let mut s = replay(&dir, include_str!("edit_path/w1-run-end.steps.json"));
    run(&mut s, "file.save", json!({"path": "noop.3dcraft"}));
    let mut doc = craft3d_format::load(&dir.join("noop.3dcraft")).unwrap();
    for feature in &mut doc.features {
        if let craft3d_doc::FeatureKind::Sketch(sketch) = &mut feature.kind {
            sketch.image_places.clear();
        }
    }
    craft3d_format::save(&dir.join("noop-old.3dcraft"), &doc).unwrap();
    let mut s = session(&dir);
    run(&mut s, "file.open", json!({"path": "noop-old.3dcraft"}));
    let before = run(&mut s, "document.inspect", json!({}));
    // The same value again: nothing changes.
    set(&mut s, &[("n", "3")]);
    assert!(
        s.execute("edit.undo", &json!({})).is_err(),
        "a no-op edit made an undo step"
    );
    assert_eq!(run(&mut s, "document.inspect", json!({})), before);
    run(&mut s, "file.save", json!({"path": "noop-again.3dcraft"}));
    let again = craft3d_format::load(&dir.join("noop-again.3dcraft")).unwrap();
    assert_eq!(again, doc, "the no-op edit changed the document");
}

/// A sketch that can be carried only part of the way says so: the point
/// after the run pinned where it is cannot follow the run's end from 3 to
/// 10 periods. `sketch.inspect` warns with how far it got, the sketch keeps
/// its stored coordinates and places (so a fresh open of its file carries
/// and warns the same), and back at 3 it is as it was.
#[test]
fn a_sketch_carried_part_of_the_way_warns() {
    let dir = dir("w1-stop");
    let mut s = replay(&dir, include_str!("edit_path/w1-run-end.steps.json"));
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Edge", "type": "fixed", "entities": ["e11"]}),
    );
    let start = equals_a_fresh_open(&mut s, &dir, "stop-3.3dcraft");
    set(&mut s, &[("n", "10")]);
    let here = state(&mut s);
    let warned = |state: &Value| -> Vec<Value> {
        state["sketches"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|sk| sk.get("warnings").cloned())
            .collect()
    };
    let warnings = warned(&here);
    assert_eq!(warnings.len(), 1, "{here}");
    assert!(
        warnings[0][0]
            .as_str()
            .unwrap()
            .contains("of the way from where its image points were"),
        "{warnings:?}"
    );
    run(&mut s, "file.save", json!({"path": "stop-10.3dcraft"}));
    let mut fresh = session(&dir);
    run(&mut fresh, "file.open", json!({"path": "stop-10.3dcraft"}));
    assert!(
        here == state(&mut fresh),
        "the session and a fresh open differ"
    );
    set(&mut s, &[("n", "3")]);
    let back = equals_a_fresh_open(&mut s, &dir, "stop-back.3dcraft");
    assert!(back == start, "back at 3 it is as it was");
}
