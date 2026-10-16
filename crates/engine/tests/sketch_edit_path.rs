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
    let mut fresh = session(dir);
    run(&mut fresh, "file.open", json!({"path": file}));
    let there = state(&mut fresh);
    assert!(
        here == there,
        "{file}: the edited session and a fresh open differ"
    );
    here
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
}
