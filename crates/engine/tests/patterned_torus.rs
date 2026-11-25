//! Random design 108069 through the commands: a cut torus patterned about
//! the middle of its extent (docs/design/booleans.md › *A curve of touch
//! kept past a face's border*).
//!
//! The pattern's last join, instances 0 to 3 with instances 4 and 5, was
//! refused: *"instances 0 to 3 and instances 4 to 5 touch or overlap and
//! cannot be joined: boolean produced an invalid body"*. The copies half a
//! turn apart touch along circles (each one's torus tangent to the other's
//! cut plane from within it), which makes no edge of the union: the
//! pattern is a body, held here to the deep check and to the volume
//! written without the kernel (`plan/patterned-torus/tools/quad_patterned.py`
//! in the workshop; `crates/kernel/tests/boolean_patterned_torus.rs` says
//! how, and holds the booleans themselves).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

/// The six instances of the reduction united: the torus about the x axis
/// (major 10, minor 5) kept below x = 2.5, turned by 45 degrees five times
/// about the z axis through (-1.25, 0.1, 0).
const REDUCED: f64 = 11_935.951_438_6;
/// The six instances of the seed's own numbers: the cut at 2.489335440737587
/// from the torus's centre, the pivot 0.011383997545657 off its axis.
const SEED: f64 = 11_921.760_261_7;

fn run(session: &mut Session, id: &str, params: &Value) -> Value {
    session
        .execute(id, params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// The body named `name`, built: its feature is ok, the document passes
/// the deep check, and its volume is `expected` to within the tolerance
/// times its area.
fn built(session: &mut Session, name: &str, expected: f64, tolerance: f64) {
    let document = run(session, "document.inspect", &json!({}));
    let feature = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}: {document}"));
    assert_eq!(feature["status"], "ok", "{feature}");
    let check = run(session, "document.check", &json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    let body = document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name}: {document}"));
    let (volume, area) = (
        body["volume"].as_f64().unwrap(),
        body["area"].as_f64().unwrap(),
    );
    println!("{name}: volume {volume:.9e} against {expected:.9e}, area {area:.6e}");
    assert!(
        (volume - expected).abs() <= tolerance * area,
        "{name}: volume {volume:e} against {expected:e}, allowed {:e}",
        tolerance * area
    );
}

/// The seed's own commands, as the fuzzer made them, up to the pattern
/// that was refused (without the sketch on the cut face and its extrusion
/// between, which take no part in the pattern).
#[test]
fn the_seeds_pattern_is_built() {
    let mut s = Session::new(Workspace::unrestricted());
    run(
        &mut s,
        "file.new",
        &json!({"tolerance": 1e-6, "units": "mm"}),
    );
    let steps = [
        (
            "feature.torus",
            json!({"angle": "0 deg", "axis": [1.0, 0.0, 0.0], "major": 10.0, "minor": 5.0,
                   "name": "N0", "position": [-2.5, 2.5, 7.5]}),
        ),
        ("feature.edit", json!({"feature": "N0", "minor": 5.0})),
        (
            "feature.planeCut",
            json!({"name": "N2", "target": "N0", "planes": [{
                "normal": [-1.0, 0.0, 0.0],
                "point": [-0.010664559262413498, -12.5, 14.98900060388343],
                "side": "above"}]}),
        ),
        (
            "feature.pattern",
            json!({"angle": "45 deg", "axis": [0.0, 0.0, 1.0], "count": 6, "name": "N4",
                   "point": [-3.7553322796312067, 2.511383997545657, 7.499999999999999],
                   "source": "N2"}),
        ),
    ];
    for (id, params) in &steps {
        let result = run(&mut s, id, params);
        assert_ne!(result["status"], "failed", "{id}: {result}");
    }
    built(&mut s, "N4", SEED, 1e-6);
}

/// The reduction, three commands, at three scales (the tolerance with the
/// lengths, as the random designs scale theirs).
#[test]
fn the_reduced_pattern_is_built_at_three_scales() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = Session::new(Workspace::unrestricted());
        run(
            &mut s,
            "file.new",
            &json!({"tolerance": 1e-6 * k, "units": "mm"}),
        );
        let steps = [
            (
                "feature.torus",
                json!({"name": "T", "axis": [1, 0, 0], "major": 10.0 * k, "minor": 5.0 * k}),
            ),
            (
                "feature.planeCut",
                json!({"name": "C", "target": "T", "planes": [{
                    "point": [2.5 * k, 0, 0], "normal": [-1, 0, 0], "side": "above"}]}),
            ),
            (
                "feature.pattern",
                json!({"name": "P", "source": "C", "count": 6, "angle": "45 deg",
                       "axis": [0, 0, 1], "point": [-1.25 * k, 0.1 * k, 0]}),
            ),
        ];
        for (id, params) in &steps {
            let result = run(&mut s, id, params);
            assert_ne!(result["status"], "failed", "{id} at {k}: {result}");
        }
        built(&mut s, "P", REDUCED * k * k * k, 1e-6 * k);
    }
}
