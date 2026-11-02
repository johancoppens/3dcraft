//! How long projected trims take (docs/design/surfacing.md › *A projected
//! edge, proven*): the guide's window on the quarter cylinder (along a
//! direction), the same circle taken to its closest points, and a window
//! cut across a spline drum by a slanted rectangle. Ignored by default; run
//! in release with `--ignored --nocapture` to print the times.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::time::Instant;

fn quarter_cylinder(s: &mut Session) -> String {
    for (name, plane, offset) in [("Low", "xy", "0 mm"), ("High", "xy", "30 mm")] {
        built(
            s,
            "sketch.create",
            json!({"name": name, "plane": plane, "offset": offset}),
        );
        run(
            s,
            "sketch.arc",
            json!({"sketch": name, "center": [0, 0], "start": [20, 0], "end": [0, 20]}),
        );
    }
    built(s, "sketch.create", json!({"name": "Left", "plane": "zx"}));
    run(
        s,
        "sketch.line",
        json!({"sketch": "Left", "from": [0, 20], "to": [30, 20]}),
    );
    built(s, "sketch.create", json!({"name": "Right", "plane": "yz"}));
    run(
        s,
        "sketch.line",
        json!({"sketch": "Right", "from": [20, 0], "to": [20, 30]}),
    );
    built(
        s,
        "feature.surface.coons",
        json!({"curves": ["Low", "Right", "High", "Left"], "name": "Wall"}),
    );
    body(s, "Wall")["faceNames"][0]
        .as_str()
        .unwrap()
        .to_string()
}

/// A case: its name, the outline it draws, the trim's params.
type Case<'a> = (
    &'a str,
    &'a dyn Fn(&mut Session),
    Box<dyn Fn(&str) -> Value>,
);

/// The fastest of five builds of the trim, in milliseconds, and its
/// report.
fn timed(case: &str, outline: &dyn Fn(&mut Session), trim: &dyn Fn(&str) -> Value) -> (f64, Value) {
    let mut best = f64::INFINITY;
    let mut report = Value::Null;
    for round in 0..5 {
        let dir = workdir("projected-trim-timing", &format!("{case}-{round}"));
        let mut s = session_in(&dir);
        new_document(&mut s, 1.0);
        let wall = quarter_cylinder(&mut s);
        outline(&mut s);
        let start = Instant::now();
        let made = built(&mut s, "feature.surface.trim", trim(&wall));
        best = best.min(start.elapsed().as_secs_f64() * 1e3);
        report = made["trim"].clone();
    }
    (best, report)
}

#[test]
#[ignore = "a timing, run on demand in release"]
fn projected_trims_take() {
    let window = |s: &mut Session| {
        built(
            s,
            "sketch.create",
            json!({"plane": "yz", "offset": "40 mm", "name": "Window"}),
        );
        run(
            s,
            "sketch.circle",
            json!({"sketch": "Window", "center": [9, 15], "radius": "6 mm"}),
        );
    };
    let slanted = |s: &mut Session| {
        built(
            s,
            "sketch.create",
            json!({"plane": "yz", "offset": "40 mm", "name": "Window"}),
        );
        for (from, to) in [
            ([4.0, 5.0], [15.0, 9.0]),
            ([15.0, 9.0], [13.0, 24.0]),
            ([13.0, 24.0], [3.0, 20.0]),
            ([3.0, 20.0], [4.0, 5.0]),
        ] {
            run(
                s,
                "sketch.line",
                json!({"sketch": "Window", "from": from, "to": to}),
            );
        }
    };
    let cases: [Case; 3] = [
        (
            "guide window, along -x",
            &window,
            Box::new(
                |w: &str| json!({"face": w, "outer": "Window", "along": [-1, 0, 0], "name": "Patch"}),
            ),
        ),
        (
            "guide window, closest points",
            &window,
            Box::new(|w: &str| json!({"face": w, "outer": "Window", "name": "Patch"})),
        ),
        (
            "slanted quadrilateral, closest points",
            &slanted,
            Box::new(|w: &str| json!({"face": w, "outer": "Window", "name": "Patch"})),
        ),
    ];
    for (case, outline, trim) in &cases {
        let (ms, report) = timed(case, *outline, trim.as_ref());
        println!("{case}: {ms:.2} ms, {report}");
    }
}
