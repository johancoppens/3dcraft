//! Outputs (`variable.measure`, `variable.output`, `output.list`, and the
//! assertion measure `value`): read-only variables computed after
//! regeneration from measures and expressions (docs/design/verification.md
//! › *Outputs*; decided by the developer 2026-10-27). Against closed
//! forms: the metacentric height of a box hull, GM = KB + BM − KG, and
//! the draft of a floating box, m / (ρ L B), at three scales; and the
//! rules: an input or a feature that uses an output is refused by name, a
//! cycle is refused with its names, a measure that fails makes its output
//! unknown and an assertion on it notProven, outputs survive a save and
//! follow a rename.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
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
fn close(got: f64, want: f64, rel: f64) {
    assert!(
        (got - want).abs() <= rel * want.abs().max(1e-300),
        "{got} is not {want} within {rel}"
    );
}

/// The output named `name` as `output.list` gives it.
fn output(session: &mut Session, name: &str) -> Value {
    let listed = run(session, "output.list", json!({}));
    listed["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no output {name}: {listed}"))
}

fn value_of(session: &mut Session, name: &str) -> f64 {
    let o = output(session, name);
    assert_eq!(o["status"], "known", "{o}");
    o["value"].as_f64().unwrap()
}

fn assertion(session: &mut Session, name: &str) -> Value {
    let listed = run(session, "assert.list", json!({}));
    listed["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no assertion {name}: {listed}"))
}

/// The kayak's question on a box hull 2000 × 600 × 400 mm displacing
/// 0.3 m³: the waterline T = V / (L B) = 250 mm, the centre of buoyancy
/// KB = T / 2 = 125 mm, the waterplane's second moment about the roll
/// axis I = L B³ / 12 = 3.6e10 mm⁴, BM = I / V = 120 mm, and with KG =
/// 180 mm, GM = KB + BM − KG = 65 mm.
fn kayak() -> Session {
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for step in [
        json!(["variable.set", {"name": "KG", "expr": "180 mm"}]),
        json!(["variable.set", {"name": "V", "expr": "0.3 * (1 m)^3"}]),
        json!(["feature.block", {"size": ["2000 mm", "600 mm", "400 mm"], "position": ["-1000 mm", "-300 mm", "0 mm"], "name": "Hull"}]),
        json!(["variable.measure", {"name": "T", "measure": "levelFor", "of": "Hull", "volume": "V"}]),
        json!(["variable.measure", {"name": "KB", "measure": "centroidBelow", "of": "Hull", "offset": "T", "axis": "z"}]),
        json!(["variable.measure", {"name": "I", "measure": "sectionInertia", "of": "Hull", "offset": "T", "about": "x", "xAxis": "x"}]),
        json!(["variable.output", {"name": "BM", "expr": "I / V"}]),
        json!(["variable.output", {"name": "GM", "expr": "KB + BM - KG"}]),
        json!(["assert.add", {"name": "stable", "measure": "value", "expr": "GM", "min": "0 mm"}]),
    ] {
        run(&mut s, step[0].as_str().unwrap(), step[1].clone());
    }
    s
}

#[test]
fn the_metacentric_height_of_a_box_hull_is_its_closed_form() {
    let mut s = kayak();
    close(value_of(&mut s, "T"), 250.0, 1e-9);
    close(value_of(&mut s, "KB"), 125.0, 1e-9);
    close(value_of(&mut s, "I"), 2000.0 * 600f64.powi(3) / 12.0, 1e-9);
    close(value_of(&mut s, "BM"), 120.0, 1e-9);
    close(value_of(&mut s, "GM"), 65.0, 1e-9);
    let gm = output(&mut s, "GM");
    assert_eq!(gm["unit"], "mm");
    assert_eq!(gm["expr"], "KB + BM - KG");
    assert_eq!(output(&mut s, "I")["unit"], "mm^4");
    let stable = assertion(&mut s, "stable");
    assert_eq!(stable["status"], "passed", "{stable}");
    close(stable["measured"].as_f64().unwrap(), 65.0, 1e-9);
    // Shown in document.inspect and param.list as outputs.
    let inspected = run(&mut s, "document.inspect", json!({"measures": false}));
    let names: Vec<&str> = inspected["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["T", "KB", "I", "BM", "GM"]);
    let states = s.output_states();
    assert_eq!(states.len(), 5);
    // A centre of gravity too high: GM = 125 + 120 - 300 < 0, and only the
    // assertion is measured again (the measures do not depend on KG).
    let measured = s.assertions_measured().unwrap();
    run(
        &mut s,
        "variable.set",
        json!({"name": "KG", "expr": "300 mm"}),
    );
    close(value_of(&mut s, "GM"), -55.0, 1e-9);
    let stable = assertion(&mut s, "stable");
    assert_eq!(stable["status"], "failed", "{stable}");
    assert_eq!(s.assertions_measured().unwrap(), measured + 1);
    // Undo brings it back, from what was measured before.
    run(&mut s, "edit.undo", json!({}));
    close(value_of(&mut s, "GM"), 65.0, 1e-9);
    assert_eq!(s.assertions_measured().unwrap(), measured + 1);
}

#[test]
fn the_same_document_gives_the_same_outputs() {
    let mut a = kayak();
    let mut b = kayak();
    for name in ["T", "KB", "I", "BM", "GM"] {
        assert_eq!(
            value_of(&mut a, name).to_bits(),
            value_of(&mut b, name).to_bits(),
            "{name}"
        );
    }
}

/// A box L × B × H of mass m floats in a liquid of density ρ at the draft
/// m / (ρ L B): here half its height, at 1e-3, 1 and 1e6 mm.
#[test]
fn a_floating_box_has_its_draft_at_three_scales() {
    for scale in [1e-3_f64, 1.0, 1e6] {
        let mut s = Session::new(Workspace::unrestricted());
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 1e-6 * scale}),
        );
        let (l, b, h) = (4.0 * scale, 1.3 * scale, scale);
        for (name, expr) in [
            ("L", format!("{l:e} mm")),
            ("B", format!("{b:e} mm")),
            ("H", format!("{h:e} mm")),
            ("rho", "1000 / (1 m)^3".to_string()),
            ("mass", "rho * L * B * H * 0.4".to_string()),
        ] {
            run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
        }
        run(
            &mut s,
            "feature.block",
            json!({"size": ["L", "B", "H"], "position": ["-L / 2", "-B / 2", "0 mm"], "name": "Box"}),
        );
        run(
            &mut s,
            "variable.measure",
            json!({"name": "draft", "measure": "levelFor", "of": "Box", "volume": "mass / rho"}),
        );
        run(
            &mut s,
            "assert.add",
            json!({"name": "draft", "measure": "value", "expr": "draft", "eq": "mass / (rho * L * B)"}),
        );
        close(value_of(&mut s, "draft"), 0.4 * h, 1e-9);
        let checked = assertion(&mut s, "draft");
        assert_eq!(checked["status"], "passed", "scale {scale}: {checked}");
    }
}

#[test]
fn an_input_or_a_feature_that_uses_an_output_is_refused_by_name() {
    let mut s = kayak();
    let message = refused(
        &mut s,
        "variable.set",
        json!({"name": "beam", "expr": "2 * GM"}),
    );
    assert!(
        message.contains("the variable `beam` uses `GM`") && message.contains("one way"),
        "{message}"
    );
    let message = refused(
        &mut s,
        "feature.block",
        json!({"size": ["T", "10 mm", "10 mm"], "name": "Ballast"}),
    );
    assert!(
        message.contains("the feature `Ballast`") && message.contains("uses `T`"),
        "{message}"
    );
    // An input defined first and the output after: the output is refused.
    run(
        &mut s,
        "variable.set",
        json!({"name": "later", "expr": "Q + 1 mm"}),
    );
    let message = refused(
        &mut s,
        "variable.output",
        json!({"name": "Q", "expr": "GM"}),
    );
    assert!(
        message.contains("the variable `later` uses `Q`"),
        "{message}"
    );
    // An output is read-only: variable.set, param.set and variable.publish
    // refuse it by name.
    let message = refused(
        &mut s,
        "variable.set",
        json!({"name": "GM", "expr": "1 mm"}),
    );
    assert!(message.contains("`GM` is an output"), "{message}");
    run(&mut s, "variable.publish", json!({"name": "KG"}));
    let message = refused(&mut s, "param.set", json!({"values": {"GM": 3}}));
    assert!(message.contains("`GM` is an output"), "{message}");
    let message = refused(&mut s, "variable.publish", json!({"name": "GM"}));
    assert!(message.contains("`GM` is an output"), "{message}");
    // An output does not take a variable's name.
    let message = refused(
        &mut s,
        "variable.output",
        json!({"name": "KG", "expr": "1 mm"}),
    );
    assert!(message.contains("`KG` is a variable"), "{message}");
    // The form shows the outputs.
    let listed = run(&mut s, "param.list", json!({}));
    assert_eq!(listed["outputs"].as_array().unwrap().len(), 5, "{listed}");
}

#[test]
fn a_cycle_among_outputs_is_refused_with_its_names() {
    let mut s = kayak();
    run(
        &mut s,
        "variable.output",
        json!({"name": "A", "expr": "B + 1 mm"}),
    );
    assert_eq!(output(&mut s, "A")["status"], "unknown");
    let message = refused(&mut s, "variable.output", json!({"name": "B", "expr": "A"}));
    assert!(message.contains("B → A → B"), "{message}");
    let message = refused(
        &mut s,
        "variable.output",
        json!({"name": "C", "expr": "C * 2"}),
    );
    assert!(message.contains("C → C"), "{message}");
    // Through a measure's expression as well.
    let message = refused(
        &mut s,
        "variable.measure",
        json!({"name": "T", "measure": "levelFor", "of": "Hull", "volume": "KB * (1 m)^2"}),
    );
    assert!(message.contains("T → KB → T"), "{message}");
    // Nothing of it was kept.
    close(value_of(&mut s, "GM"), 65.0, 1e-9);
}

#[test]
fn a_measure_that_fails_leaves_its_output_unknown_and_an_assertion_not_proven() {
    let mut s = kayak();
    // More than the hull holds: no level leaves that volume below.
    run(
        &mut s,
        "variable.measure",
        json!({"name": "Tover", "measure": "levelFor", "of": "Hull", "volume": "2 * (1 m)^3"}),
    );
    run(
        &mut s,
        "variable.output",
        json!({"name": "freeboard", "expr": "400 mm - Tover"}),
    );
    let over = output(&mut s, "Tover");
    assert_eq!(over["status"], "unknown", "{over}");
    assert!(over["value"].is_null());
    let freeboard = output(&mut s, "freeboard");
    assert_eq!(freeboard["status"], "unknown");
    assert!(
        freeboard["message"]
            .as_str()
            .unwrap()
            .contains("it uses `Tover`, which is unknown"),
        "{freeboard}"
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "dry", "measure": "value", "expr": "freeboard", "min": "50 mm"}),
    );
    let dry = assertion(&mut s, "dry");
    assert_eq!(dry["status"], "notProven", "{dry}");
    assert!(
        dry["message"].as_str().unwrap().contains("`freeboard`"),
        "{dry}"
    );
    // In a bound of another measure, too.
    run(
        &mut s,
        "assert.add",
        json!({"name": "deep", "measure": "levelFor", "of": "Hull", "volume": "V", "max": "Tover"}),
    );
    assert_eq!(assertion(&mut s, "deep")["status"], "notProven");
    // A wall found but proven only above a bound short of it is not a
    // value either.
    run(
        &mut s,
        "feature.sphere",
        json!({"radius": "10 mm", "position": [0, 0, 1000], "name": "Ball"}),
    );
    run(
        &mut s,
        "variable.measure",
        json!({"name": "wall", "measure": "thickness", "of": "Ball"}),
    );
    let wall = output(&mut s, "wall");
    assert_ne!(wall["status"], "pending", "{wall}");
    if wall["status"] == "unknown" {
        assert!(
            wall["message"].as_str().unwrap().contains("not proven"),
            "{wall}"
        );
    }
}

#[test]
fn outputs_survive_a_save_and_follow_a_rename() {
    let dir = std::env::temp_dir().join(format!("craft3d-outputs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = kayak();
    run(
        &mut s,
        "variable.rename",
        json!({"name": "V", "to": "displacement"}),
    );
    run(
        &mut s,
        "variable.rename",
        json!({"name": "T", "to": "waterline"}),
    );
    run(
        &mut s,
        "feature.rename",
        json!({"feature": "Hull", "name": "Kayak"}),
    );
    close(value_of(&mut s, "GM"), 65.0, 1e-9);
    let kb = output(&mut s, "KB");
    assert_eq!(kb["measure"]["offset"], "waterline", "{kb}");
    assert_eq!(output(&mut s, "BM")["expr"], "I / displacement");
    let path = dir.join("hull.3dcraft");
    let path = path.to_str().unwrap();
    run(
        &mut s,
        "file.save",
        json!({"path": path, "overwrite": true}),
    );
    let mut t = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut t, "file.open", json!({"path": "hull.3dcraft"}));
    for name in ["waterline", "KB", "I", "BM", "GM"] {
        assert_eq!(
            value_of(&mut t, name).to_bits(),
            value_of(&mut s, name).to_bits(),
            "{name}"
        );
    }
    assert_eq!(assertion(&mut t, "stable")["status"], "passed");
    // An output still used is not deleted, unless forced.
    let message = refused(&mut t, "variable.delete", json!({"name": "BM"}));
    assert!(message.contains("the output `GM`"), "{message}");
    run(
        &mut t,
        "variable.delete",
        json!({"name": "GM", "force": true}),
    );
    let stable = assertion(&mut t, "stable");
    assert_eq!(stable["status"], "error", "{stable}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_params_are_errors() {
    let mut s = kayak();
    for (id, params) in [
        ("variable.measure", json!({})),
        ("variable.measure", json!({"name": "x"})),
        (
            "variable.measure",
            json!({"name": "x", "measure": "nonsense"}),
        ),
        (
            "variable.measure",
            json!({"name": "x", "measure": "value", "expr": "1"}),
        ),
        (
            "variable.measure",
            json!({"name": "x", "measure": "volume", "about": "x"}),
        ),
        (
            "variable.measure",
            json!({"name": "pi", "measure": "volume"}),
        ),
        ("variable.output", json!({"name": "x"})),
        ("variable.output", json!({"expr": "1"})),
        ("variable.output", json!({"name": 3, "expr": "1"})),
        ("output.list", json!({"wait": "no"})),
    ] {
        refused(&mut s, id, params);
    }
    // An assertion on a value whose expression does not evaluate is in
    // error, with the reason.
    run(
        &mut s,
        "assert.add",
        json!({"name": "typo", "measure": "value", "expr": "GMM", "min": "0 mm"}),
    );
    let typo = assertion(&mut s, "typo");
    assert_eq!(typo["status"], "error", "{typo}");
}

/// The curvature of a sphere of radius r is 1 / r²: a max of 0.5 / r²
/// fails and one of 2 / r² passes, at three scales. Before 2026-10-27 a
/// bound on a negative power of a length had the tolerance to that power
/// as slack (1e12 / r² here), so the first passed too.
#[test]
fn a_spheres_curvature_fails_a_max_below_it_at_three_scales() {
    for scale in [1e-3_f64, 1.0, 1e6] {
        let mut s = Session::new(Workspace::unrestricted());
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 1e-6 * scale}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": format!("{scale:e} mm")}),
        );
        run(
            &mut s,
            "feature.sphere",
            json!({"radius": "r", "name": "Ball"}),
        );
        for (name, max) in [("flat enough", "0.5 / r^2"), ("round", "2 / r^2")] {
            run(
                &mut s,
                "assert.add",
                json!({"name": name, "measure": "curvature", "of": "Ball", "max": max}),
            );
        }
        let flat = assertion(&mut s, "flat enough");
        assert_eq!(flat["status"], "failed", "scale {scale}: {flat}");
        close(flat["measured"].as_f64().unwrap(), scale.powi(-2), 1e-6);
        let round = assertion(&mut s, "round");
        assert_eq!(round["status"], "passed", "scale {scale}: {round}");
    }
}

/// A value of one over a length (1 / GM = 1 / 65 mm) and a plain number
/// (KB / KG = 125 / 180) held to bounds they miss: they fail, with no
/// slack of the tolerance to the power -1 (1e6) or 0 (1).
#[test]
fn a_value_of_a_power_that_is_not_positive_fails_a_bound_it_misses() {
    let mut s = kayak();
    run(
        &mut s,
        "assert.add",
        json!({"name": "inverse", "measure": "value", "expr": "1 / GM", "max": "0.01 / (1 mm)"}),
    );
    let inverse = assertion(&mut s, "inverse");
    assert_eq!(inverse["status"], "failed", "{inverse}");
    close(inverse["measured"].as_f64().unwrap(), 1.0 / 65.0, 1e-9);
    assert_eq!(inverse["unit"], "mm^-1");
    run(
        &mut s,
        "assert.add",
        json!({"name": "ratio", "measure": "value", "expr": "KB / KG", "min": "1.2"}),
    );
    let ratio = assertion(&mut s, "ratio");
    assert_eq!(ratio["status"], "failed", "{ratio}");
    close(ratio["measured"].as_f64().unwrap(), 125.0 / 180.0, 1e-9);
    run(
        &mut s,
        "assert.add",
        json!({"name": "ratio eq", "measure": "value", "expr": "KB / KG", "eq": "0.5"}),
    );
    assert_eq!(assertion(&mut s, "ratio eq")["status"], "failed");
}

/// A measured output follows a table its measure looks a value up in: a
/// table edit changes no variable, and the output is measured again.
#[test]
fn a_measured_output_follows_a_table_it_looks_up() {
    let mut s = kayak();
    run(
        &mut s,
        "table.set",
        json!({"name": "loads", "columns": ["case", "litres"], "rows": [["light", 240], ["full", 360]]}),
    );
    run(
        &mut s,
        "variable.measure",
        json!({"name": "Tload", "measure": "levelFor", "of": "Hull", "volume": "loads[1].litres * 1000 * (1 cm)^3"}),
    );
    // 240 litres over 2000 x 600 mm: 200 mm.
    close(value_of(&mut s, "Tload"), 200.0, 1e-9);
    run(
        &mut s,
        "table.set",
        json!({"name": "loads", "columns": ["case", "litres"], "rows": [["light", 300], ["full", 360]]}),
    );
    close(value_of(&mut s, "Tload"), 250.0, 1e-9);
}

/// The waterline's length (repro sea-kayak 15): the extent along an axis
/// of a body's section by a plane at the level found for its
/// displacement. A box 4000 × 500 × 300 mm displacing 0.2 m³ floats at
/// T = 100 mm with a waterline 4000 mm long and 500 mm wide; a cylinder
/// lying along x, radius 200 mm, cut at z = h spans 2 √(r² − h²) across;
/// a sphere cut at a height spans its circle along a slanted axis.
#[test]
fn the_extent_of_a_section_along_an_axis_is_its_closed_form() {
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for step in [
        json!(["feature.block", {"size": ["4000 mm", "500 mm", "300 mm"], "position": ["-2000 mm", "-250 mm", "0 mm"], "name": "Hull"}]),
        json!(["variable.set", {"name": "V", "expr": "0.2 * (1 m)^3"}]),
        json!(["variable.measure", {"name": "T", "measure": "levelFor", "of": "Hull", "volume": "V"}]),
        json!(["variable.measure", {"name": "LWL", "measure": "sectionExtent", "of": "Hull", "plane": "xy", "offset": "T", "axis": "x"}]),
        json!(["variable.measure", {"name": "BWL", "measure": "sectionExtent", "of": "Hull", "offset": "T", "axis": "-y"}]),
        json!(["feature.cylinder", {"radius": "200 mm", "height": "1000 mm", "position": ["0 mm", "2000 mm", "0 mm"], "axis": [1, 0, 0], "name": "Log"}]),
        json!(["variable.measure", {"name": "LogB", "measure": "sectionExtent", "of": "Log", "offset": "120 mm", "axis": "y"}]),
        json!(["variable.measure", {"name": "LogL", "measure": "sectionExtent", "of": "Log", "offset": "-50 mm", "axis": "x"}]),
        json!(["feature.sphere", {"radius": "300 mm", "position": ["0 mm", "-2000 mm", "0 mm"], "name": "Ball"}]),
        json!(["variable.measure", {"name": "BallD", "measure": "sectionExtent", "of": "Ball", "offset": "180 mm", "axis": [3, 4, 0]}]),
        json!(["assert.add", {"name": "waterline", "measure": "sectionExtent", "of": "Hull", "offset": "T", "axis": "x", "eq": "4000 mm"}]),
    ] {
        run(&mut s, step[0].as_str().unwrap(), step[1].clone());
    }
    close(value_of(&mut s, "T"), 100.0, 1e-9);
    close(value_of(&mut s, "LWL"), 4000.0, 1e-12);
    close(value_of(&mut s, "BWL"), 500.0, 1e-12);
    close(
        value_of(&mut s, "LogB"),
        2.0 * (200f64.powi(2) - 120f64.powi(2)).sqrt(),
        1e-9,
    );
    close(value_of(&mut s, "LogL"), 1000.0, 1e-12);
    close(
        value_of(&mut s, "BallD"),
        2.0 * (300f64.powi(2) - 180f64.powi(2)).sqrt(),
        1e-9,
    );
    assert_eq!(output(&mut s, "LWL")["unit"], "mm");
    let waterline = assertion(&mut s, "waterline");
    assert_eq!(waterline["status"], "passed", "{waterline}");
    // Where it begins and ends along the axis, in the details.
    let detailed = run(&mut s, "assert.list", json!({"details": true}));
    let found = detailed["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "waterline")
        .unwrap()
        .clone();
    let details = found.to_string();
    assert!(details.contains("\"from\":-2000"), "{details}");
    assert!(details.contains("\"to\":2000"), "{details}");
}

#[test]
fn a_section_extent_that_cannot_be_measured_is_said() {
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [100, 50, 30], "name": "B"}),
    );
    // Above the body: no section, an error and not a length of zero.
    let above = run(
        &mut s,
        "assert.add",
        json!({"name": "above", "measure": "sectionExtent", "of": "B", "offset": 40, "axis": "x", "eq": 0}),
    );
    assert_eq!(above["status"], "error", "{above}");
    assert!(above.to_string().contains("does not cut"), "{above}");
    for bad in [
        json!({"name": "no axis", "measure": "sectionExtent", "of": "B", "offset": 10, "eq": 1}),
        json!({"name": "zero axis", "measure": "sectionExtent", "of": "B", "offset": 10, "axis": [0, 0, 0], "eq": 1}),
        json!({"name": "word", "measure": "sectionExtent", "of": "B", "offset": 10, "axis": "w", "eq": 1}),
        json!({"name": "key", "measure": "sectionExtent", "of": "B", "offset": 10, "axis": "x", "about": "x", "eq": 1}),
    ] {
        if let Ok(v) = s.execute("assert.add", &bad) {
            assert_eq!(v["status"], "error", "{bad}: {v}");
        }
    }
}
