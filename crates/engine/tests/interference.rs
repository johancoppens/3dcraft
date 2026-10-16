//! `document.interference`: clear, touching and overlapping bodies by
//! closed forms at three scales, the laser box of the showcase at kerf 0
//! and 0.15 mm, the box filter's work, and the document left unchanged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::path::PathBuf;

const ID: &str = "document.interference";

fn run(session: &mut Session, id: &str, params: &Value) -> Value {
    session
        .execute(id, params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn new_session() -> Session {
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", &json!({"units": "mm"}));
    s
}

fn block(s: &mut Session, name: &str, at: [f64; 3], size: [f64; 3]) {
    let mm = |v: [f64; 3]| json!(v.map(|x| format!("{x} mm")));
    run(
        s,
        "feature.block",
        &json!({"name": name, "position": mm(at), "size": mm(size)}),
    );
}

/// The pair of `a` and `b` (by name) in an answer.
fn pair<'a>(answer: &'a Value, a: &str, b: &str) -> &'a Value {
    answer["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| (p["aName"] == a && p["bName"] == b) || (p["aName"] == b && p["bName"] == a))
        .unwrap_or_else(|| panic!("no pair {a} x {b} in {answer}"))
}

fn close(got: f64, want: f64, relative: f64) -> bool {
    (got - want).abs() <= relative * want.abs().max(1e-300)
}

/// Two boxes overlapping by a known volume, touching on a face, an edge
/// and a corner, apart, and a cylinder through a plate, at three scales.
#[test]
fn closed_forms_at_three_scales() {
    for k in [0.01, 1.0, 1000.0] {
        let mut s = new_session();
        let v = |x: f64, y: f64, z: f64| [x * k, y * k, z * k];
        block(&mut s, "A", v(0.0, 0.0, 0.0), v(2.0, 2.0, 2.0));
        // Overlaps A in [1,2] x [0.5,2] x [0,2]: 3 k^3.
        block(&mut s, "Over", v(1.0, 0.5, 0.0), v(2.0, 2.0, 2.0));
        block(&mut s, "Face", v(-1.0, 0.5, 0.5), v(1.0, 1.0, 1.0));
        block(&mut s, "Edge", v(-1.0, -1.0, 0.0), v(1.0, 1.0, 2.0));
        block(&mut s, "Corner", v(-1.0, 2.0, 2.0), v(1.0, 1.0, 1.0));
        block(&mut s, "Apart", v(10.0, 10.0, 10.0), v(1.0, 1.0, 1.0));
        block(&mut s, "Plate", v(20.0, 0.0, 0.0), v(4.0, 4.0, 0.5));
        run(
            &mut s,
            "feature.cylinder",
            &json!({"name": "Pin", "radius": format!("{} mm", 0.5 * k), "height": format!("{} mm", 3.0 * k),
                    "position": [format!("{} mm", 22.0 * k), format!("{} mm", 2.0 * k), format!("{} mm", -k)]}),
        );
        let answer = run(&mut s, ID, &json!({}));
        let at = |a: &str, b: &str| pair(&answer, a, b).clone();
        let over = at("A", "Over");
        assert_eq!(over["status"], "overlap", "{over}");
        let volume = over["volume"].as_f64().unwrap();
        assert!(close(volume, 3.0 * k * k * k, 1e-9), "scale {k}: {volume}");
        let bbox = &over["bbox"];
        assert!(close(bbox["min"][0].as_f64().unwrap(), k, 1e-9), "{bbox}");
        assert!(
            close(bbox["max"][1].as_f64().unwrap(), 2.0 * k, 1e-9),
            "{bbox}"
        );
        let faces = over["faces"]["a"].as_array().unwrap();
        assert!(
            !faces.is_empty() && faces.iter().all(Value::is_string),
            "{over}"
        );
        for (other, kind) in [("Face", "face"), ("Edge", "edge"), ("Corner", "point")] {
            let p = at("A", other);
            assert_eq!(p["status"], "touch", "scale {k}: {p}");
            assert_eq!(p["contact"], kind, "scale {k}: {p}");
            assert!(!p["at"].as_array().unwrap().is_empty(), "{p}");
            assert!(!p["faces"]["a"].as_array().unwrap().is_empty(), "{p}");
        }
        assert_eq!(at("A", "Apart")["status"], "clear");
        assert_eq!(at("A", "Plate")["status"], "clear");
        let pin = at("Plate", "Pin");
        assert_eq!(pin["status"], "overlap", "{pin}");
        let want = std::f64::consts::PI * 0.25 * 0.5 * k * k * k;
        assert!(
            close(pin["volume"].as_f64().unwrap(), want, 1e-7),
            "scale {k}: {pin}"
        );
        assert_eq!(answer["summary"]["unknown"], 0, "{answer}");
    }
}

/// The pairs come in the order of the bodies (history order) or as given,
/// and `details: false` leaves the details out.
#[test]
fn order_pairs_and_details() {
    let mut s = new_session();
    block(&mut s, "A", [0.0; 3], [2.0; 3]);
    block(&mut s, "B", [1.0, 0.0, 0.0], [2.0; 3]);
    block(&mut s, "C", [5.0, 0.0, 0.0], [1.0; 3]);
    let all = run(&mut s, ID, &json!({}));
    let names: Vec<(String, String)> = all["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["aName"].as_str().unwrap().to_string(),
                p["bName"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let want = [("A", "B"), ("A", "C"), ("B", "C")].map(|(a, b)| (a.to_string(), b.to_string()));
    assert_eq!(names, want);
    assert_eq!(all["summary"]["overlap"], 1);
    assert_eq!(all["summary"]["clear"], 2);
    assert_eq!(all, run(&mut s, ID, &json!({})), "deterministic");
    let given = run(
        &mut s,
        ID,
        &json!({"pairs": [["C", "A"], ["B", "A"]], "details": false}),
    );
    let pairs = given["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0]["aName"], "C");
    assert_eq!(pairs[1]["status"], "overlap");
    assert!(pairs[1].get("faces").is_none() && pairs[1].get("bbox").is_none());
    assert!(close(pairs[1]["volume"].as_f64().unwrap(), 4.0, 1e-9));
    let some = run(&mut s, ID, &json!({"bodies": ["F3", 1]}));
    assert_eq!(some["count"], 1);
    assert_eq!(some["pairs"][0]["aName"], "C");
}

/// Twenty bodies far apart: 190 pairs, none past the box test, no boolean.
#[test]
fn bodies_far_apart_cost_no_boolean() {
    let mut s = new_session();
    for n in 0..20 {
        block(
            &mut s,
            &format!("B{n}"),
            [10.0 * n as f64, 0.0, 0.0],
            [1.0; 3],
        );
    }
    let answer = run(&mut s, ID, &json!({}));
    assert_eq!(answer["count"], 190);
    assert_eq!(answer["summary"]["clear"], 190);
    assert_eq!(answer["work"]["checked"], 0, "{}", answer["work"]);
    // A row of touching neighbours: only the 19 neighbour pairs are
    // checked.
    let mut s = new_session();
    for n in 0..20 {
        block(&mut s, &format!("B{n}"), [n as f64, 0.0, 0.0], [1.0; 3]);
    }
    let answer = run(&mut s, ID, &json!({}));
    assert_eq!(answer["work"]["checked"], 19, "{}", answer["work"]);
    assert_eq!(answer["summary"]["touch"], 19, "{}", answer["summary"]);
    assert_eq!(answer["summary"]["clear"], 171);
}

/// The question changes nothing: not the history, not the undo steps,
/// not the dirty flag, not the saved bytes.
#[test]
fn the_document_is_unchanged() {
    let dir = std::env::temp_dir().join(format!("craft3d-interference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = new_session();
    block(&mut s, "A", [0.0; 3], [2.0; 3]);
    block(&mut s, "B", [1.0, 0.0, 0.0], [2.0; 3]);
    let first = dir.join("first.3dcraft");
    run(&mut s, "file.save", &json!({"path": first}));
    let before = run(&mut s, "document.inspect", &json!({}));
    run(&mut s, ID, &json!({}));
    run(&mut s, ID, &json!({"pairs": [["A", "B"]]}));
    assert!(s.execute(ID, &json!({"bodies": ["nothing"]})).is_err());
    let after = run(&mut s, "document.inspect", &json!({}));
    assert_eq!(before, after);
    assert_eq!(after["dirty"], false);
    let second = dir.join("second.3dcraft");
    run(&mut s, "file.save", &json!({"path": second}));
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bad parameters are errors, not crashes.
#[test]
fn bad_parameters_fail_gracefully() {
    let mut s = Session::new(Workspace::unrestricted());
    assert!(s.execute(ID, &json!({})).is_err(), "no document");
    let mut s = new_session();
    block(&mut s, "A", [0.0; 3], [2.0; 3]);
    let empty = run(&mut s, ID, &json!({}));
    assert_eq!(empty["count"], 0);
    for bad in [
        json!([]),
        json!({"bodies": "A"}),
        json!({"bodies": [true]}),
        json!({"bodies": ["F99"]}),
        json!({"pairs": [["A"]]}),
        json!({"pairs": [["A", "A"]]}),
        json!({"pairs": "A"}),
        json!({"pairs": [[1, 2, 3]]}),
        json!({"bodies": ["A"], "pairs": []}),
        json!({"details": 3}),
        json!({"other": 1}),
    ] {
        assert!(s.execute(ID, &bad).is_err(), "{bad}");
    }
}

/// The laser box of the showcase (plan/laserbox.md): at kerf 0 the panels
/// only touch; at kerf 0.15 mm they overlap by the volumes an independent
/// cell grid gave (`overlap_oracle.py`, to 4 digits).
#[test]
fn laser_box_at_kerf_zero_and_kerf_015() {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/interference/laserbox.steps.json"),
    )
    .unwrap();
    let steps: Vec<(String, Value)> = serde_json::from_str(&text).unwrap();
    let mut s = new_session();
    for (id, params) in &steps {
        run(&mut s, id, params);
    }
    let answer = run(&mut s, ID, &json!({}));
    assert_eq!(answer["count"], 15, "{answer}");
    assert_eq!(answer["summary"]["unknown"], 0, "{answer}");
    for (a, b, want) in [
        ("Front", "Left", 24.5812),
        ("Front", "Right", 24.5812),
        ("Front", "Bottom", 54.4613),
        ("Front", "Lid", 31.1175),
        ("Back", "Left", 24.5812),
        ("Back", "Right", 24.5812),
        ("Back", "Bottom", 54.4613),
        ("Back", "Lid", 36.0338),
        ("Left", "Bottom", 33.9638),
        ("Left", "Lid", 23.4338),
        ("Right", "Bottom", 33.9637),
        ("Right", "Lid", 23.4338),
    ] {
        let p = pair(&answer, a, b);
        assert_eq!(p["status"], "overlap", "{p}");
        let got = p["volume"].as_f64().unwrap();
        assert!(
            (got - want).abs() <= 1.5e-4,
            "{a} x {b}: {got}, want {want}"
        );
    }
    for (a, b) in [("Front", "Back"), ("Left", "Right"), ("Bottom", "Lid")] {
        assert_eq!(pair(&answer, a, b)["status"], "clear");
    }
    run(
        &mut s,
        "variable.set",
        &json!({"name": "kerf", "expr": "0 mm"}),
    );
    let answer = run(&mut s, ID, &json!({}));
    assert_eq!(answer["summary"]["overlap"], 0, "{answer}");
    assert_eq!(answer["summary"]["unknown"], 0, "{answer}");
    assert_eq!(answer["summary"]["clear"], 3, "{answer}");
    assert_eq!(answer["summary"]["touch"], 12, "{answer}");
    for p in answer["pairs"].as_array().unwrap() {
        if p["status"] == "touch" {
            assert_eq!(p["contact"], "face", "{p}");
        }
    }
}

/// A pin that fits a drilled hole exactly shares the hole's wall: a face
/// contact (the wall is a reversed face), with the area 2 pi r t.
#[test]
fn a_pin_in_its_hole_touches_on_a_face() {
    let mut s = new_session();
    block(&mut s, "Plate", [0.0; 3], [10.0, 10.0, 1.0]);
    run(
        &mut s,
        "feature.cylinder",
        &json!({"name": "Drill", "radius": "2 mm", "height": "3 mm", "position": ["5 mm", "5 mm", "-1 mm"]}),
    );
    run(
        &mut s,
        "feature.boolean",
        &json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Holed"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        &json!({"name": "Pin", "radius": "2 mm", "height": "5 mm", "position": ["5 mm", "5 mm", "-2 mm"]}),
    );
    let answer = run(&mut s, ID, &json!({}));
    let p = pair(&answer, "Holed", "Pin");
    assert_eq!(p["status"], "touch", "{p}");
    assert_eq!(p["contact"], "face", "{p}");
    let want = 2.0 * std::f64::consts::PI * 2.0;
    assert!(close(p["area"].as_f64().unwrap(), want, 1e-6), "{p}");
}

/// More pairs past the box test than one question checks is an error that
/// names the count, before any boolean.
#[test]
fn too_many_pairs_to_check_is_an_error() {
    let mut s = new_session();
    for n in 0..65 {
        block(
            &mut s,
            &format!("B{n}"),
            [0.01 * f64::from(n), 0.0, 0.0],
            [1.0; 3],
        );
    }
    let error = s.execute(ID, &json!({})).unwrap_err().to_string();
    assert!(error.contains("2080 pairs"), "{error}");
    let some = run(
        &mut s,
        ID,
        &json!({"bodies": ["B0", "B1", "B2"], "details": false}),
    );
    assert_eq!(some["summary"]["overlap"], 3, "{some}");
}
