//! Hydrostatics (`measure.hydrostatics` and the assertion measure
//! `hydrostatic`) against the closed forms of a box barge (Biran &
//! López-Pulido ch. 2): waterline T = m / (ρ L B), KB = T/2,
//! BM = B²/(12 T), GM's sign at the stability limit, the wall-sided
//! formula for GZ (Tupper ch. 4), loads and given weights, and the errors
//! for what cannot float.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn refused(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should fail, returned {value}"),
        Err(e) => e.message().to_string(),
    }
}

#[track_caller]
fn close(got: &Value, want: f64, what: &str) {
    let got = got
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: not a number: {got}"));
    let allowed = 1e-7 * want.abs().max(1.0);
    assert!(
        (got - want).abs() <= allowed,
        "{what}: {got} is not {want} within {allowed}"
    );
}

/// A box barge `l × b × d` mm of a density (kg/m³), centred on the z
/// axis with its bottom at z = 0.
fn barge(l: f64, b: f64, d: f64, density: Option<f64>) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [l, b, d], "position": [-l / 2.0, -b / 2.0, 0], "name": "Barge"}),
    );
    if let Some(density) = density {
        run(
            &mut s,
            "body.material",
            json!({"body": "Barge", "name": "Wood", "density": density}),
        );
    }
    s
}

#[test]
fn a_box_barge_floats_at_its_closed_forms() {
    let (l, b, d) = (4000.0, 1300.0, 1000.0);
    let mut s = barge(l, b, d, Some(500.0));
    let h = run(&mut s, "measure.hydrostatics", json!({"density": 1000}));
    // Half as dense as the water: half under.
    let t = 500.0;
    close(&h["draft"], t, "draft T = m / (ρ L B)");
    close(
        &h["mass"],
        4.0 * 1.3 * 1.0 * 500.0,
        "mass from the material",
    );
    close(&h["displacement"], 2600.0, "displacement = mass");
    close(&h["volume"], l * b * t, "∇");
    close(&h["KB"], t / 2.0, "KB = T/2");
    let bm = b * b / (12.0 * t);
    close(&h["BM"], bm, "BM = B²/(12 T)");
    close(&h["BMl"], l * l / (12.0 * t), "BMl = L²/(12 T)");
    close(&h["KG"], d / 2.0, "KG at the middle");
    close(&h["GM"], t / 2.0 + bm - d / 2.0, "GM = KB + BM − KG");
    close(&h["waterplaneArea"], l * b, "A_w");
    close(&h["iTransverse"], l * b.powi(3) / 12.0, "I_T");
    close(&h["wettedArea"], l * b + 2.0 * (l + b) * t, "wetted");
    close(&h["GZ"], 0.0, "upright, no righting arm");
    assert!(h["steps"].as_u64().unwrap() <= 8, "{h}");
    assert_eq!(h["lengthUnit"], "mm");
    // Heeled 10°: wall-sided, GZ = sin φ (GM + BM tan²φ / 2).
    let gm = t / 2.0 + bm - d / 2.0;
    for heel in ["10 deg", "10"] {
        let heeled = run(
            &mut s,
            "measure.hydrostatics",
            json!({"density": "1 g/cm3", "heel": heel}),
        );
        let phi = 10.0_f64.to_radians();
        close(
            &heeled["GZ"],
            phi.sin() * (gm + 0.5 * bm * phi.tan().powi(2)),
            "GZ wall-sided",
        );
        close(&heeled["volume"], l * b * t, "the same displacement heeled");
    }
}

/// GM changes sign at B/D = √(6 s (1 − s)) = √1.5 for s = 1/2; an
/// assertion that GM ≥ 0 fails on the narrow barge and passes once it is
/// made wider.
#[test]
fn an_assertion_on_gm_breaks_and_passes_again() {
    let limit = 1.5_f64.sqrt() * 1000.0;
    let mut s = barge(4000.0, limit * 0.98, 1000.0, Some(500.0));
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "stable", "measure": "hydrostatic", "quantity": "GM", "density": 1000, "min": 0}),
    );
    assert_eq!(added["status"], "failed", "{added}");
    let draft = run(
        &mut s,
        "assert.add",
        json!({"name": "draft", "measure": "hydrostatic", "quantity": "draft", "density": 1000, "eq": "500 mm", "tol": 1e-6}),
    );
    assert_eq!(draft["status"], "passed", "{draft}");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Barge", "size": [4000, limit * 1.02, 1000], "position": [-2000, -limit * 0.51, 0]}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["ok"], true, "{listed}");
    // A second moment is in the document's unit to the fourth power.
    let i = run(
        &mut s,
        "assert.add",
        json!({"name": "I", "measure": "hydrostatic", "quantity": "iTransverse", "density": 1000, "min": "1 * (1 m)^4"}),
    );
    assert_eq!(i["status"], "failed", "{i}");
    assert_eq!(i["unit"], "mm^4", "{i}");
}

#[test]
fn levels_masses_loads_and_heights_given() {
    let (l, b) = (4000.0, 1300.0);
    let mut s = barge(l, b, 1000.0, None);
    // At a level: no weight needed, and without one no G.
    let at = run(&mut s, "measure.hydrostatics", json!({"level": "300 mm"}));
    close(&at["volume"], l * b * 300.0, "∇ at a level");
    close(&at["KB"], 150.0, "KB");
    assert!(at["GM"].is_null(), "{at}");
    assert!(at["why"].as_str().unwrap().contains("centre of gravity"));
    let gm = run(
        &mut s,
        "assert.add",
        json!({"name": "gm", "measure": "hydrostatic", "quantity": "GM", "level": 300, "min": 0}),
    );
    assert_eq!(gm["status"], "error", "{gm}");
    // A mass given, its height above the keel given.
    let given = run(
        &mut s,
        "measure.hydrostatics",
        json!({"mass": "1 t", "kg": "200 mm", "density": 1000}),
    );
    let t = 1000.0 / (1000.0 * 4.0 * 1.3) * 1000.0;
    close(&given["draft"], t, "draft for a tonne");
    close(&given["KG"], 200.0, "KG given");
    close(
        &given["GM"],
        t / 2.0 + b * b / (12.0 * t) - 200.0,
        "GM with KG given",
    );
    // A keel below the origin raises every height by as much.
    let keel = run(
        &mut s,
        "measure.hydrostatics",
        json!({"mass": 1000, "kg": "200 mm", "density": 1000, "keel": -100}),
    );
    close(&keel["draft"], t + 100.0, "draft from a lower keel");
    close(&keel["KB"], t / 2.0 + 100.0, "KB from a lower keel");
    // The barge's own mass and a load.
    run(
        &mut s,
        "body.material",
        json!({"body": "Barge", "name": "Wood", "density": 500}),
    );
    let loaded = run(
        &mut s,
        "measure.hydrostatics",
        json!({"density": 1000, "loads": [{"mass": "300 kg", "at": [0, 0, "800 mm"]}]}),
    );
    let mass = 2600.0 + 300.0;
    close(&loaded["mass"], mass, "with the load");
    let t = mass / (1000.0 * 4.0 * 1.3) * 1000.0;
    close(&loaded["draft"], t, "draft loaded");
    close(
        &loaded["KG"],
        (2600.0 * 500.0 + 300.0 * 800.0) / mass,
        "KG loaded",
    );
}

#[test]
fn what_cannot_float_is_refused() {
    let mut s = barge(4000.0, 1000.0, 1000.0, Some(2000.0));
    let id = "measure.hydrostatics";
    assert!(refused(&mut s, id, json!({"density": 1000})).contains("sink"));
    assert!(refused(&mut s, id, json!({"mass": 0})).contains("nothing to float"));
    assert!(refused(&mut s, id, json!({"mass": -1})).contains("less than nothing"));
    assert!(refused(&mut s, id, json!({"up": [0, 0, 0]})).contains("up"));
    assert!(refused(&mut s, id, json!({"along": "z"})).contains("square"));
    assert!(refused(&mut s, id, json!({"heel": "5 mm"})).contains("angle"));
    assert!(refused(&mut s, id, json!({"density": 0})).contains("density"));
    assert!(refused(&mut s, id, json!({"level": "nope"})).contains("level"));
    assert!(refused(&mut s, id, json!({"loads": [{"mass": 1}]})).contains("at"));
    assert!(refused(&mut s, id, json!({"loads": 3})).contains("loads"));
    assert!(refused(&mut s, id, json!({"bogus": 1})).contains("bogus"));
    assert!(refused(&mut s, id, json!({"of": "Nothing"})).contains("Nothing"));
    let mut bare = barge(4000.0, 1000.0, 1000.0, None);
    assert!(refused(&mut bare, id, json!({})).contains("no material"));
    let mut empty = Session::new(Workspace::none());
    run(&mut empty, "file.new", json!({"units": "mm"}));
    assert!(refused(&mut empty, id, json!({})).contains("no bodies"));
    // An assertion of a quantity that is not one: an error, said.
    let wrong = run(
        &mut s,
        "assert.add",
        json!({"name": "w", "measure": "hydrostatic", "quantity": "nope", "level": 10, "min": 0}),
    );
    assert_eq!(wrong["status"], "error", "{wrong}");
    assert!(wrong["message"].as_str().unwrap().contains("quantity"));
}

/// The parameters are expressions of the variables, and renaming a
/// variable renames it in the assertion.
#[test]
fn expressions_follow_the_variables() {
    let mut s = barge(4000.0, 1300.0, 1000.0, Some(500.0));
    run(
        &mut s,
        "variable.set",
        json!({"name": "heelAngle", "expr": "15 deg"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "water", "expr": "1000"}),
    );
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "rights", "measure": "hydrostatic", "quantity": "GZ", "heel": "heelAngle", "density": "water", "min": 0}),
    );
    assert_eq!(added["status"], "passed", "{added}");
    run(
        &mut s,
        "variable.rename",
        json!({"name": "heelAngle", "to": "phi"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(
        listed["assertions"][0]["measure"]["heel"], "phi",
        "{listed}"
    );
}
