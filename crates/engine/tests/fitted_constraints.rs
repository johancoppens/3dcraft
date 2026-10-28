//! `pointOn` and `tangent` on a curve given by formulas and on a live
//! spline (R1, 2026-10-28), through the commands, against closed forms at
//! four scales: a circle written as formulas, a parabola, a quadratic live
//! spline through three points; and what stays refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{error, run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};

const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

fn document(k: f64) -> Session {
    let mut s = session();
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6 * k}),
    );
    run(&mut s, "sketch.create", json!({"name": "S"}));
    s
}

fn at(s: &mut Session, id: &Value) -> [f64; 2] {
    let inspected = run(s, "sketch.inspect", json!({"sketch": "S"}));
    let entity = inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == *id)
        .unwrap_or_else(|| panic!("no {id}: {inspected}"))
        .clone();
    [entity["x"].as_f64().unwrap(), entity["y"].as_f64().unwrap()]
}

fn constrain(s: &mut Session, kind: &str, entities: Value, value: Option<String>) -> Value {
    let mut params = json!({"sketch": "S", "type": kind, "entities": entities});
    if let Some(value) = value {
        params["value"] = value.into();
    }
    run(s, "sketch.constrain", params)
}

#[track_caller]
fn near(got: [f64; 2], want: [f64; 2], k: f64, what: &str) {
    let gap = (got[0] - want[0]).hypot(got[1] - want[1]);
    // The fit is within half the tolerance of the formulas, the solve
    // within the tolerance.
    assert!(
        gap <= 3e-6 * k,
        "{what}: {got:?} against {want:?}, {gap:e} apart"
    );
}

fn point(s: &mut Session, p: [f64; 2], k: f64) -> Value {
    run(
        s,
        "sketch.point",
        json!({"sketch": "S", "at": [p[0] * k, p[1] * k]}),
    )["entity"]
        .clone()
}

/// A circle of radius 10 k written as formulas: a point on it, a line
/// resting on its top, a circle touching it from outside.
#[test]
fn a_circle_written_as_formulas_takes_points_lines_and_circles() {
    for k in SCALES {
        let mut s = document(k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": format!("{} mm", 10.0 * k)}),
        );
        let curve = run(
            &mut s,
            "sketch.curve",
            json!({"sketch": "S", "x": "r * cos(t)", "y": "r * sin(t)", "t": [0, "360 deg"]}),
        );
        assert_eq!(curve["closed"], true, "{curve}");
        let c = curve["entity"].clone();
        // A point at x = 6 k: y = 8 k.
        let p = point(&mut s, [3.0, 2.0], k);
        constrain(&mut s, "x", json!([p]), Some(format!("{} mm", 6.0 * k)));
        let made = constrain(&mut s, "pointOn", json!([p, c]), None);
        assert_eq!(made["status"], "ok", "{made}");
        near(
            at(&mut s, &p),
            [6.0 * k, 8.0 * k],
            k,
            &format!("x{k}: the point"),
        );
        // A horizontal line between x = -5 k and 5 k, on top: y = 10 k.
        let line = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "S", "from": [-5.0 * k, 12.0 * k], "to": [5.0 * k, 12.5 * k]}),
        );
        constrain(&mut s, "horizontal", json!([line["entity"]]), None);
        constrain(
            &mut s,
            "x",
            json!([line["start"]]),
            Some(format!("{} mm", -5.0 * k)),
        );
        constrain(
            &mut s,
            "x",
            json!([line["end"]]),
            Some(format!("{} mm", 5.0 * k)),
        );
        let made = constrain(&mut s, "tangent", json!([line["entity"], c]), None);
        assert_eq!(made["status"], "ok", "{made}");
        near(
            at(&mut s, &line["start"]),
            [-5.0 * k, 10.0 * k],
            k,
            &format!("x{k}: the line"),
        );
        // A circle about (25 k, 0) touching it from outside: radius 15 k.
        let circle = run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "S", "center": [25.0 * k, 1.0 * k], "radius": 5.0 * k}),
        );
        constrain(
            &mut s,
            "x",
            json!([circle["center"]]),
            Some(format!("{} mm", 25.0 * k)),
        );
        constrain(&mut s, "y", json!([circle["center"]]), Some("0 mm".into()));
        let made = constrain(&mut s, "tangent", json!([c, circle["entity"]]), None);
        assert_eq!(made["status"], "ok", "{made}");
        assert_eq!(made["dof"], 0, "{made}");
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "S"}));
        let radius = inspected["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == circle["entity"])
            .unwrap()["radius"]
            .as_f64()
            .unwrap();
        assert!(
            (radius - 15.0 * k).abs() <= 3e-6 * k,
            "x{k}: radius {radius}"
        );
        // The circle follows the formulas' variable: r = 12 k, radius 13 k.
        run(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": format!("{} mm", 12.0 * k)}),
        );
        near(
            at(&mut s, &p),
            [6.0 * k, (144.0f64 - 36.0).sqrt() * k],
            k,
            &format!("x{k}: the point after r changed"),
        );
    }
}

/// From (0, -c) the tangents to y = t^2 / c touch at t = c: a line of
/// length 3 c leaves along (1, 2).
#[test]
fn a_line_from_a_point_touches_a_parabola_given_by_formulas() {
    for k in SCALES {
        let mut s = document(k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "c", "expr": format!("{k} mm")}),
        );
        let curve = run(
            &mut s,
            "sketch.curve",
            json!({"sketch": "S", "x": "t", "y": "t^2 / c", "t": ["-2 * c", "2 * c"]}),
        );
        let line = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "S", "from": [0.0, -k], "to": [1.2 * k, 2.6 * k]}),
        );
        constrain(&mut s, "x", json!([line["start"]]), Some("0 mm".into()));
        constrain(&mut s, "y", json!([line["start"]]), Some("-c".into()));
        constrain(
            &mut s,
            "distance",
            json!([line["entity"]]),
            Some("3 * c".into()),
        );
        let made = constrain(
            &mut s,
            "tangent",
            json!([curve["entity"], line["entity"]]),
            None,
        );
        assert_eq!(made["status"], "ok", "{made}");
        assert_eq!(made["dof"], 0, "{made}");
        let unit = 3.0 * k / 5f64.sqrt();
        near(
            at(&mut s, &line["end"]),
            [unit, -k + 2.0 * unit],
            k,
            &format!("x{k}"),
        );
    }
}

/// A quadratic live spline through (0, 0), (10 k, h) and (20 k, 0) is the
/// Bezier with its middle control point at (10 k, 2 h): y = 4 h u (1 - u)
/// with x = 20 k u. A point at x = 5 k is at y = 3/4 h; a horizontal line
/// rests at y = h; a line from its start leaves along (10 k, 2 h); all
/// follow h.
#[test]
fn a_live_spline_takes_points_and_tangents_and_they_follow_it() {
    for k in SCALES {
        let mut s = document(k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "h", "expr": format!("{} mm", 10.0 * k)}),
        );
        let live = run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "S", "through": [[0, 0], [10.0 * k, 10.0 * k], [20.0 * k, 0]], "degree": 2, "live": true}),
        );
        let through: Vec<Value> = live["through"].as_array().unwrap().clone();
        for (i, q) in through.iter().enumerate() {
            let x = [0.0, 10.0, 20.0][i] * k;
            constrain(&mut s, "x", json!([q]), Some(format!("{x} mm")));
            let y = if i == 1 {
                "h".to_string()
            } else {
                "0 mm".into()
            };
            constrain(&mut s, "y", json!([q]), Some(y));
        }
        let spline = live["entity"].clone();
        let p = point(&mut s, [5.0, 1.0], k);
        constrain(&mut s, "x", json!([p]), Some(format!("{} mm", 5.0 * k)));
        let made = constrain(&mut s, "pointOn", json!([p, spline]), None);
        assert_eq!(made["status"], "ok", "{made}");
        let top = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "S", "from": [5.0 * k, 12.0 * k], "to": [15.0 * k, 12.0 * k]}),
        );
        constrain(&mut s, "horizontal", json!([top["entity"]]), None);
        constrain(
            &mut s,
            "x",
            json!([top["start"]]),
            Some(format!("{} mm", 5.0 * k)),
        );
        constrain(
            &mut s,
            "x",
            json!([top["end"]]),
            Some(format!("{} mm", 15.0 * k)),
        );
        let made = constrain(&mut s, "tangent", json!([spline, top["entity"]]), None);
        assert_eq!(made["status"], "ok", "{made}");
        let lead = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "S", "from": through[0], "to": [1.0 * k, 3.0 * k]}),
        );
        constrain(
            &mut s,
            "distance",
            json!([lead["entity"]]),
            Some(format!("{} mm", 5.0 * k)),
        );
        let made = constrain(&mut s, "tangent", json!([lead["entity"], spline]), None);
        assert_eq!(made["status"], "ok", "{made}");
        assert_eq!(made["dof"], 0, "{made}");
        for h in [10.0, 6.0] {
            run(
                &mut s,
                "variable.set",
                json!({"name": "h", "expr": format!("{} mm", h * k)}),
            );
            let what = format!("x{k} h {h}");
            near(
                at(&mut s, &p),
                [5.0 * k, 0.75 * h * k],
                k,
                &format!("{what}: point"),
            );
            near(
                at(&mut s, &top["start"]),
                [5.0 * k, h * k],
                k,
                &format!("{what}: top"),
            );
            // Along (10 k, 2 h k), 5 k long.
            let (dx, dy): (f64, f64) = (10.0, 2.0 * h);
            let n = 5.0 / dx.hypot(dy);
            near(
                at(&mut s, &lead["end"]),
                [dx * n * k, dy * n * k],
                k,
                &format!("{what}: the line from the start"),
            );
        }
    }
}

/// What stays refused, with the reason; and bad parameters are errors.
#[test]
fn what_fitted_curves_do_not_take_is_refused_with_the_reason() {
    let mut s = document(1.0);
    let a = run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "S", "x": "t", "y": "t^2", "t": [0, 1]}),
    );
    let b = run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "S", "x": "1 + t", "y": "1 - t", "t": [0, 1], "start": a["end"]}),
    );
    let live = run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "S", "through": [[5, 5], [6, 7], [8, 5]], "degree": 2, "live": true}),
    );
    let line = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "S", "from": [0, -3], "to": [3, -3]}),
    );
    let refused = |s: &mut Session, kind: &str, entities: Value| {
        error(
            s,
            "sketch.constrain",
            json!({"sketch": "S", "type": kind, "entities": entities}),
        )
    };
    let both = refused(&mut s, "tangent", json!([a["entity"], b["entity"]]));
    assert!(both.contains("both curves given by formulas"), "{both}");
    let apart = refused(&mut s, "tangent", json!([a["entity"], live["entity"]]));
    assert!(apart.contains("share none"), "{apart}");
    let parallel = refused(&mut s, "parallel", json!([line["entity"], a["entity"]]));
    assert!(parallel.contains("pointOn and tangent"), "{parallel}");
    let backwards = refused(&mut s, "pointOn", json!([a["entity"], a["start"]]));
    assert!(!backwards.is_empty());
}
