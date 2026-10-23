//! Curves from formulas in a sketch (`sketch.curve`) and live splines
//! (`sketch.spline {live: true}`): fitted again at every regeneration from
//! the variables, so the solids made from them follow the variables.
//! Checked against closed-form areas and volumes (a parabola segment, an
//! Archimedean spiral, an involute gear, a paraboloid and a sine vase
//! revolved), at several scales, across a save and reopen, for determinism
//! and against bad parameters.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, error, failed, run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

fn set(s: &mut Session, name: &str, expr: &str) {
    run(s, "variable.set", json!({ "name": name, "expr": expr }));
}

fn new_document(s: &mut Session, tolerance: f64) {
    run(
        s,
        "file.new",
        json!({"units": "mm", "tolerance": tolerance}),
    );
}

/// The volume of body `name`, valid and checked deeply, within `slack`
/// of `expected`.
#[track_caller]
fn assert_volume(s: &mut Session, name: &str, expected: f64, slack: f64) -> Value {
    let found = body(s, name);
    assert_eq!(found["valid"], true, "{found}");
    let volume = found["volume"].as_f64().unwrap();
    assert!(
        (volume - expected).abs() <= slack,
        "{name}: volume {volume}, expected {expected}, off by {:e} (allowed {slack:e})",
        volume - expected
    );
    let check = run(s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    found
}

/// The fit of entity `entity` of sketch `sketch`, as `sketch.inspect`
/// gives it.
fn fit_of(s: &mut Session, sketch: &str, entity: &str) -> Value {
    let inspected = run(s, "sketch.inspect", json!({ "sketch": sketch }));
    inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == entity)
        .unwrap_or_else(|| panic!("no {entity}: {inspected}"))["fit"]
        .clone()
}

/// A parabola `y = x² / (4 f)` from `-a` to `a`, closed by the chord at the
/// top: the segment's area is two thirds of its bounding box, `4/3 a h`
/// with `h = a² / (4 f)`. At three scales; and it follows `a`.
#[test]
fn a_parabola_segment_has_two_thirds_of_its_box() {
    for k in [1e-3, 1.0, 1e3, 1e6] {
        let mut s = session();
        new_document(&mut s, 1e-6 * k);
        set(&mut s, "f", &format!("{} mm", 10.0 * k));
        set(&mut s, "a", &format!("{} mm", 20.0 * k));
        set(&mut s, "d", &format!("{} mm", 5.0 * k));
        run(&mut s, "sketch.create", json!({"name": "P"}));
        let curve = run(
            &mut s,
            "sketch.curve",
            json!({"sketch": "P", "x": "t", "y": "t^2 / (4 * f)", "t": ["-a", "a"]}),
        );
        assert_eq!(curve["closed"], false, "{curve}");
        assert_eq!(curve["parameter"], "t");
        assert!(curve["fit"]["deviation"].as_f64().unwrap() <= 0.5e-6 * k);
        let (start, end) = (curve["start"].clone(), curve["end"].clone());
        run(
            &mut s,
            "sketch.line",
            json!({"sketch": "P", "from": end, "to": start}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "P", "distance": "d", "name": "Segment"}),
        );
        let area = |a: f64, f: f64| 4.0 / 3.0 * a * a * a / (4.0 * f);
        let length = 2.0 * 50.0 * k;
        let slack = length * 1e-6 * k * 5.0 * k;
        assert_volume(&mut s, "Segment", area(20.0 * k, 10.0 * k) * 5.0 * k, slack);
        // The curve follows its variables.
        set(&mut s, "a", &format!("{} mm", 15.0 * k));
        assert_volume(&mut s, "Segment", area(15.0 * k, 10.0 * k) * 5.0 * k, slack);
    }
}

/// An Archimedean spiral `r = a + b θ` over one turn, closed by the piece of
/// the x axis between its ends: the area its radius sweeps is
/// `((a + 2π b)³ − a³) / (6 b)`.
#[test]
fn an_archimedean_spiral_sweeps_its_closed_form_area() {
    let mut s = session();
    new_document(&mut s, 1e-6);
    set(&mut s, "a", "10 mm");
    set(&mut s, "b", "3 mm");
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let curve = run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "S", "x": "(a + b * t) * cos(t)", "y": "(a + b * t) * sin(t)", "t": [0, "2 * pi"]}),
    );
    let (start, end) = (curve["start"].clone(), curve["end"].clone());
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "S", "from": end, "to": start}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "S", "distance": 2, "name": "Spiral"}),
    );
    let (a, b) = (10.0, 3.0);
    let area = ((a + 2.0 * PI * b).powi(3) - a.powi(3)) / (6.0 * b);
    assert_volume(&mut s, "Spiral", area * 2.0, 1e-6 * 200.0 * 2.0);
    let fit = fit_of(&mut s, "S", curve["entity"].as_str().unwrap());
    assert!(fit["controlPoints"].as_u64().unwrap() < 400, "{fit}");
}

/// A full circle written as formulas closes on one point, and is a
/// profile on its own: a disc of `π r²`.
#[test]
fn a_closed_formula_curve_is_a_profile_on_its_own() {
    let mut s = session();
    new_document(&mut s, 1e-6);
    set(&mut s, "r", "7 mm");
    run(&mut s, "sketch.create", json!({"name": "C"}));
    let curve = run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "C", "x": "r * cos(t)", "y": "r * sin(t)", "t": ["0 deg", "360 deg"]}),
    );
    assert_eq!(curve["closed"], true, "{curve}");
    assert_eq!(curve["start"], curve["end"]);
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "C", "distance": 3, "name": "Disc"}),
    );
    assert_volume(&mut s, "Disc", PI * 49.0 * 3.0, 1e-6 * 50.0 * 3.0);
}

/// The variables of an involute spur gear of module `m`, `z` teeth and
/// pressure angle `alpha`; `g` is the angle (plain radians) at which the
/// lower flank of the tooth centred on the x axis leaves the base circle,
/// `ta` the involute's parameter at the tip.
fn gear_variables(s: &mut Session, m: f64, z: u32) {
    set(s, "module", &format!("{m} mm"));
    set(s, "z", &z.to_string());
    set(s, "alpha", "20 deg");
    set(s, "rp", "module * z / 2");
    set(s, "rb", "rp * cos(alpha)");
    set(s, "ra", "rp + module");
    set(s, "rf", "rp - 1.25 * module");
    set(s, "rc", "rf - module / 2");
    set(s, "g", "-pi / (2 * z) - (tan(alpha) - alpha / 1 rad)");
    set(s, "ta", "sqrt((ra / rb)^2 - 1)");
    set(s, "width", &format!("{} mm", 4.0 * m));
}

/// One tooth in sketch `name`: the two involute flanks (formula curves, the
/// upper one the mirror of the lower), the tip arc, radial lines down to
/// `rc` inside the root circle and the arc at `rc` that closes it.
fn tooth_sketch(s: &mut Session, name: &str) -> (String, String) {
    run(s, "sketch.create", json!({"name": name}));
    let lower = run(
        s,
        "sketch.curve",
        json!({"sketch": name,
               "x": "rb * (cos(g + t) + t * sin(g + t))",
               "y": "rb * (sin(g + t) - t * cos(g + t))",
               "t": [0, "ta"]}),
    );
    let upper = run(
        s,
        "sketch.curve",
        json!({"sketch": name,
               "x": "rb * (cos(g + t) + t * sin(g + t))",
               "y": "-rb * (sin(g + t) - t * cos(g + t))",
               "t": [0, "ta"]}),
    );
    let center = run(
        s,
        "sketch.point",
        json!({"sketch": name, "at": ["0 mm", "0 mm"]}),
    );
    let center = center["entity"].clone();
    run(
        s,
        "sketch.arc",
        json!({"sketch": name, "center": center, "start": lower["end"], "end": upper["end"]}),
    );
    let low = run(
        s,
        "sketch.point",
        json!({"sketch": name, "at": ["rc * cos(g * 1 rad)", "rc * sin(g * 1 rad)"]}),
    );
    let high = run(
        s,
        "sketch.point",
        json!({"sketch": name, "at": ["rc * cos(g * 1 rad)", "-rc * sin(g * 1 rad)"]}),
    );
    run(
        s,
        "sketch.line",
        json!({"sketch": name, "from": low["entity"], "to": lower["start"]}),
    );
    run(
        s,
        "sketch.line",
        json!({"sketch": name, "from": high["entity"], "to": upper["start"]}),
    );
    run(
        s,
        "sketch.arc",
        json!({"sketch": name, "center": center, "start": low["entity"], "end": high["entity"]}),
    );
    (
        lower["entity"].as_str().unwrap().to_string(),
        upper["entity"].as_str().unwrap().to_string(),
    )
}

/// The area of the gear: the root disc and, per tooth, what the involutes
/// and the tip arc add outside it. The area an involute sweeps from the
/// centre, `rb² t³ / 6`, makes it exact.
fn gear_area(m: f64, z: u32) -> f64 {
    let alpha = 20f64.to_radians();
    let zf = f64::from(z);
    let rp = m * zf / 2.0;
    let rb = rp * alpha.cos();
    let ra = rp + m;
    let rf = rp - 1.25 * m;
    let g = -PI / (2.0 * zf) - (alpha.tan() - alpha);
    let ta = ((ra / rb).powi(2) - 1.0).sqrt();
    let tooth = rb * rb * ta.powi(3) / 3.0 - ra * ra * (g + ta - ta.atan()) + rf * rf * g;
    PI * rf * rf + zf * tooth
}

/// An involute spur gear: one tooth from two formula flanks, extruded,
/// patterned round the axis `z` times and joined to the root disc. Its
/// volume is the closed form's, and it follows the module and the count.
#[test]
fn an_involute_gear_has_its_closed_form_volume_and_follows_its_variables() {
    for (m, k) in [(2.0, 1.0), (2e-3, 1e-3), (2e3, 1e3)] {
        let mut s = session();
        new_document(&mut s, 1e-6 * k);
        gear_variables(&mut s, m, 12);
        let (lower, _) = tooth_sketch(&mut s, "Tooth");
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Tooth", "distance": "width", "name": "T"}),
        );
        built(
            &mut s,
            "feature.pattern",
            json!({"source": "T", "count": "z", "angle": "360 deg / z", "axis": [0, 0, 1], "name": "Teeth"}),
        );
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": "rf", "height": "width", "name": "Root"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Root", "tool": "Teeth", "name": "Gear"}),
        );
        let width = 4.0 * m;
        let perimeter = 2.0 * PI * (m * 7.0) * 2.0;
        let slack = perimeter * 1e-6 * k * width;
        assert_volume(&mut s, "Gear", gear_area(m, 12) * width, slack);
        let fit = fit_of(&mut s, "Tooth", &lower);
        assert!(fit["deviation"].as_f64().unwrap() <= 0.5e-6 * k, "{fit}");
        if k == 1.0 {
            // Fewer, larger teeth: the flanks are fitted again.
            set(&mut s, "z", "10");
            set(&mut s, "module", "2.5 mm");
            // The width is its own variable, still 8 mm.
            assert_volume(
                &mut s,
                "Gear",
                gear_area(2.5, 10) * width,
                2.0 * PI * 17.5 * 2.0 * 1e-6 * width,
            );
        }
    }
}

/// A paraboloid dish: the region under `y = x² / (4 f)` up to `h`, turned
/// about the sketch's y axis, has half the volume of its cylinder.
#[test]
fn a_paraboloid_revolved_has_half_its_cylinder() {
    let mut s = session();
    new_document(&mut s, 1e-6);
    set(&mut s, "f", "15 mm");
    set(&mut s, "a", "30 mm");
    run(&mut s, "sketch.create", json!({"name": "R"}));
    let curve = run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "R", "x": "t", "y": "t^2 / (4 * f)", "t": ["0 mm", "a"]}),
    );
    let top = run(
        &mut s,
        "sketch.point",
        json!({"sketch": "R", "at": ["0 mm", "a^2 / (4 * f)"]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "R", "from": curve["end"], "to": top["entity"]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "R", "from": top["entity"], "to": curve["start"]}),
    );
    built(
        &mut s,
        "feature.revolve",
        json!({"sketch": "R", "axis": "y", "name": "Dish"}),
    );
    let (a, h) = (30.0, 900.0 / 60.0);
    assert_volume(
        &mut s,
        "Dish",
        PI * a * a * h / 2.0,
        2.0 * PI * a * 60.0 * 1e-6,
    );
}

/// A vase whose wall is a sine wave, `r = R + A sin(2π n z / H)`, turned
/// about its axis: `π (R² + A² / 2) H` for whole waves.
#[test]
fn a_sine_vase_revolved_has_its_closed_form_volume() {
    let mut s = session();
    new_document(&mut s, 1e-6);
    set(&mut s, "R", "20 mm");
    set(&mut s, "A", "3 mm");
    set(&mut s, "H", "80 mm");
    set(&mut s, "n", "3");
    run(&mut s, "sketch.create", json!({"name": "V"}));
    let wall = run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "V", "x": "R + A * sin(360 deg * n * t / H)", "y": "t", "t": ["0 mm", "H"]}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "V", "points": [wall["end"], ["0 mm", "H"], ["0 mm", "0 mm"], wall["start"]]}),
    );
    built(
        &mut s,
        "feature.revolve",
        json!({"sketch": "V", "axis": "y", "name": "Vase"}),
    );
    let (r, a, h) = (20.0, 3.0, 80.0);
    let expected = PI * (r * r + a * a / 2.0) * h;
    assert_volume(&mut s, "Vase", expected, 2.0 * PI * 23.0 * 100.0 * 1e-6);
    let fit = fit_of(&mut s, "V", wall["entity"].as_str().unwrap());
    println!("vase wall fit: {fit}");
}

/// A live spline through points held by formulas follows them, and is the
/// spline `through` fits once through the same places.
#[test]
fn a_live_spline_follows_its_points_and_matches_a_spline_fitted_once() {
    let mut s = session();
    new_document(&mut s, 1e-6);
    set(&mut s, "h", "10 mm");
    run(&mut s, "sketch.create", json!({"name": "L"}));
    let live = run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "L", "through": [["0 mm", "0 mm"], ["10 mm", "h"], ["20 mm", "h / 2"], ["30 mm", "0 mm"]], "live": true}),
    );
    assert_eq!(live["live"], true, "{live}");
    let through = live["through"].as_array().unwrap().clone();
    assert_eq!(through.len(), 4);
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "L", "from": through[3], "to": through[0]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "L", "distance": 1, "name": "Live"}),
    );
    let volume = |s: &mut Session| body(s, "Live")["volume"].as_f64().unwrap();
    let first = volume(&mut s);
    // The same places, fitted once: the same area.
    let mut once = session();
    new_document(&mut once, 1e-6);
    run(&mut once, "sketch.create", json!({"name": "L"}));
    let fixed = run(
        &mut once,
        "sketch.spline",
        json!({"sketch": "L", "through": [[0, 0], [10, 10], [20, 5], [30, 0]]}),
    );
    run(
        &mut once,
        "sketch.line",
        json!({"sketch": "L", "from": fixed["end"], "to": fixed["start"]}),
    );
    built(
        &mut once,
        "feature.extrude",
        json!({"sketch": "L", "distance": 1, "name": "Live"}),
    );
    let fixed_volume = body(&mut once, "Live")["volume"].as_f64().unwrap();
    assert!(
        (first - fixed_volume).abs() <= 1e-9 * first,
        "{first} {fixed_volume}"
    );
    // Scaling the held heights scales the area (the fit is linear in the
    // points).
    set(&mut s, "h", "20 mm");
    let doubled = volume(&mut s);
    assert!(
        (doubled - 2.0 * first).abs() <= 1e-9 * doubled,
        "{first} {doubled}"
    );
}

/// Saved and opened again, a document with formula curves and a live
/// spline regenerates to the same bodies, bit for bit; and regenerating it
/// twice gives the same numbers.
#[test]
fn formula_curves_survive_a_save_and_are_deterministic() {
    let dir = common::workdir("formula-curves", "save");
    let mut s = common::session_in(&dir);
    new_document(&mut s, 1e-6);
    gear_variables(&mut s, 2.0, 12);
    tooth_sketch(&mut s, "Tooth");
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Tooth", "distance": "width", "name": "T"}),
    );
    run(&mut s, "sketch.create", json!({"name": "L", "offset": 20}));
    let live = run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "L", "through": [["0 mm", "0 mm"], ["10 mm", "module * 3"], ["30 mm", "0 mm"]], "degree": 2, "live": true}),
    );
    let through = live["through"].as_array().unwrap().clone();
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "L", "from": through[2], "to": through[0]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "L", "distance": 1, "name": "Arch"}),
    );
    let before = (body(&mut s, "T"), body(&mut s, "Arch"));
    run(&mut s, "file.save", json!({"path": "gear.3dcraft"}));
    let mut again = common::session_in(&dir);
    run(&mut again, "file.open", json!({"path": "gear.3dcraft"}));
    let after = (body(&mut again, "T"), body(&mut again, "Arch"));
    assert_eq!(before.0["volume"], after.0["volume"]);
    assert_eq!(before.0["area"], after.0["area"]);
    assert_eq!(before.1["volume"], after.1["volume"]);
    let inspected = run(&mut again, "sketch.inspect", json!({"sketch": "Tooth"}));
    let formulas: Vec<&Value> = inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "formula")
        .collect();
    assert_eq!(formulas.len(), 2, "{inspected}");
    assert_eq!(formulas[0]["x"], "rb * (cos(g + t) + t * sin(g + t))");
    assert_eq!(formulas[0]["to"], "ta");
}

/// Renaming and deleting variables: the parameter `t` shadows a variable
/// of that name inside the formulas, a rename follows into them, and a
/// variable a formula uses is in use.
#[test]
fn variables_in_formulas_are_renamed_and_kept() {
    let mut s = session();
    new_document(&mut s, 1e-6);
    set(&mut s, "t", "2 mm");
    set(&mut s, "r", "5 mm");
    run(&mut s, "sketch.create", json!({"name": "C"}));
    let curve = run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "C", "x": "r * cos(t)", "y": "r * sin(t)", "t": [0, "pi"]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "C", "from": curve["end"], "to": curve["start"]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "C", "distance": "t", "name": "Half"}),
    );
    // The thickness is the variable t, the curve's t its parameter.
    assert_volume(&mut s, "Half", PI * 25.0 / 2.0 * 2.0, 1e-6 * 20.0 * 2.0);
    run(
        &mut s,
        "variable.rename",
        json!({"name": "r", "to": "radius"}),
    );
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "C"}));
    let entity = curve["entity"].as_str().unwrap();
    let entry = inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == entity)
        .unwrap()
        .clone();
    assert_eq!(entry["x"], "radius * cos(t)", "{entry}");
    // Renaming the variable t leaves the curve's t alone.
    run(
        &mut s,
        "variable.rename",
        json!({"name": "t", "to": "thick"}),
    );
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "C"}));
    let entry = inspected["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == entity)
        .unwrap()
        .clone();
    assert_eq!(entry["y"], "radius * sin(t)", "{entry}");
    assert_volume(&mut s, "Half", PI * 25.0 / 2.0 * 2.0, 1e-6 * 20.0 * 2.0);
    // A variable the formulas use cannot take the parameter's name.
    let refused = error(
        &mut s,
        "variable.rename",
        json!({"name": "radius", "to": "t"}),
    );
    assert!(refused.contains("parameter"), "{refused}");
    // And is in use.
    let refused = error(&mut s, "variable.delete", json!({"name": "radius"}));
    assert!(refused.contains(&format!("{entity}.x")), "{refused}");
}

/// Bad formulas and ranges are refused with a reason, never a crash; and a
/// variable that later breaks a formula fails the sketch with the reason.
#[test]
fn bad_formula_curves_are_refused_with_a_reason() {
    let mut s = session();
    new_document(&mut s, 1e-6);
    set(&mut s, "r", "5 mm");
    run(&mut s, "sketch.create", json!({"name": "C"}));
    let cases = [
        (json!({"sketch": "C", "x": "t", "y": "t"}), "t"),
        (
            json!({"sketch": "C", "x": "t", "y": "t", "t": [0]}),
            "[from, to]",
        ),
        (
            json!({"sketch": "C", "x": "t", "y": "t", "t": [1, 1]}),
            "empty",
        ),
        (
            json!({"sketch": "C", "x": "t", "y": "t", "t": ["0 deg", "1 mm"]}),
            "dimension",
        ),
        (
            json!({"sketch": "C", "x": "t", "y": "nope * t", "t": [0, 1]}),
            "nope",
        ),
        (
            json!({"sketch": "C", "x": "t * 1 deg", "y": "t", "t": [0, 1]}),
            "length",
        ),
        (
            json!({"sketch": "C", "x": "1 / (t - 0.5)", "y": "t", "t": [0, 1]}),
            "t =",
        ),
        (
            json!({"sketch": "C", "x": "r", "y": "r", "t": [0, 1]}),
            "one point",
        ),
        (
            json!({"sketch": "C", "x": "t", "y": "if(t < 0.5, 0, 1)", "t": [0, 1]}),
            "split the curve",
        ),
        (
            json!({"sketch": "C", "x": "t", "y": "t", "t": [0, 1], "start": "e99"}),
            "e99",
        ),
        (
            json!({"sketch": "C", "x": "t", "y": "t", "t": [0, 1], "closed": true}),
            "does not close",
        ),
        (json!({"sketch": "C", "x": 5, "y": [], "t": [0, 1]}), "y"),
    ];
    for (params, wanted) in cases {
        let refused = error(&mut s, "sketch.curve", params.clone());
        assert!(refused.contains(wanted), "{params}: {refused}");
    }
    // Made now, broken later by a variable: the sketch fails and says why.
    let curve = run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "C", "x": "r * cos(t)", "y": "r * sin(t)", "t": [0, "pi"]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "C", "from": curve["end"], "to": curve["start"]}),
    );
    set(&mut s, "r", "0 mm");
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "C"}));
    assert_eq!(inspected["status"], "failed", "{inspected}");
    set(&mut s, "r", "5 mm");
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "C"}));
    assert_eq!(inspected["status"], "ok", "{inspected}");
    // A constraint that moves a curve's end fails the sketch.
    let reason = failed(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "C", "type": "x", "entities": [curve["start"]], "value": "1 mm"}),
    );
    assert!(reason.contains("formulas put it"), "{reason}");
}

/// The area of one tooth of [`tooth_sketch`], down to `rc`.
fn tooth_area(m: f64, z: u32) -> f64 {
    let alpha = 20f64.to_radians();
    let zf = f64::from(z);
    let rp = m * zf / 2.0;
    let rb = rp * alpha.cos();
    let ra = rp + m;
    let rc = rp - 1.75 * m;
    let g = -PI / (2.0 * zf) - (alpha.tan() - alpha);
    let ta = ((ra / rb).powi(2) - 1.0).sqrt();
    rb * rb * ta.powi(3) / 3.0 - ra * ra * (g + ta - ta.atan()) + rc * rc * g
}

/// A tooth repeated round the centre by a sketch pattern: every instance is
/// the seed's fit turned, and one extrusion makes them all. The outline
/// goes to DXF with its flanks as splines, and the solid to a watertight
/// STL.
#[test]
fn a_ring_of_formula_teeth_extrudes_and_exports() {
    let dir = common::workdir("formula-curves", "ring");
    let mut s = common::session_in(&dir);
    new_document(&mut s, 1e-6);
    gear_variables(&mut s, 2.0, 12);
    tooth_sketch(&mut s, "Tooth");
    let seed: Vec<Value> = run(&mut s, "sketch.inspect", json!({"sketch": "Tooth"}))["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] != "point")
        .map(|e| e["id"].clone())
        .collect();
    assert_eq!(seed.len(), 6, "{seed:?}");
    run(
        &mut s,
        "sketch.pattern",
        json!({"sketch": "Tooth", "entities": seed, "count": "z", "angle": "360 deg / z", "center": ["0 mm", "0 mm"]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Tooth", "distance": "width", "name": "Teeth"}),
    );
    let found = body(&mut s, "Teeth");
    let volume = found["volume"].as_f64().unwrap();
    let expected = 12.0 * tooth_area(2.0, 12) * 8.0;
    assert!(
        (volume - expected).abs() <= 12.0 * 20.0 * 1e-6 * 8.0,
        "{volume} {expected}"
    );
    let dxf = run(
        &mut s,
        "file.export",
        json!({"sketch": "Tooth", "path": "tooth.dxf"}),
    );
    assert_eq!(dxf["splines"], json!(24), "{dxf}");
    assert_eq!(dxf["closed"], json!(12), "{dxf}");
    let stl = run(&mut s, "file.export", json!({"path": "teeth.stl"}));
    assert_eq!(stl["closed"], stl["bodies"], "{stl}");
}

/// A curve may not be held by other constraints at its ends, and takes no
/// tangent or pointOn constraint yet: refused with the reason.
#[test]
fn constraints_on_formula_curves_are_refused_for_now() {
    let mut s = session();
    new_document(&mut s, 1e-6);
    run(&mut s, "sketch.create", json!({"name": "C"}));
    let curve = run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "C", "x": "t", "y": "t^2", "t": [0, 1]}),
    );
    let line = run(
        &mut s,
        "sketch.line",
        json!({"sketch": "C", "from": curve["end"], "to": [3, 0]}),
    );
    for kind in ["tangent", "pointOn"] {
        let entities = if kind == "tangent" {
            json!([curve["entity"], line["entity"]])
        } else {
            json!([curve["start"], curve["entity"]])
        };
        let refused = error(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "C", "type": kind, "entities": entities}),
        );
        assert!(refused.contains("formula curve"), "{kind}: {refused}");
    }
}

/// The fits of the examples, for the dev log: control points, deviation
/// and the time a regeneration that refits them takes.
#[test]
fn fits_of_the_examples() {
    let mut s = session();
    new_document(&mut s, 1e-6);
    gear_variables(&mut s, 2.0, 12);
    let (lower, _) = tooth_sketch(&mut s, "Tooth");
    set(&mut s, "a", "10 mm");
    set(&mut s, "b", "3 mm");
    run(&mut s, "sketch.create", json!({"name": "S"}));
    let spiral = run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "S", "x": "(a + b * t) * cos(t)", "y": "(a + b * t) * sin(t)", "t": [0, "6 * pi"]}),
    );
    for (sketch, entity) in [
        ("Tooth", lower.as_str()),
        ("S", spiral["entity"].as_str().unwrap()),
    ] {
        let fit = fit_of(&mut s, sketch, entity);
        assert!(fit["deviation"].as_f64().unwrap() <= 0.5e-6, "{fit}");
        println!("{sketch} {entity}: {fit}");
    }
    let started = std::time::Instant::now();
    for k in 0..10 {
        set(&mut s, "b", &format!("{} mm", 3.0 + f64::from(k) * 0.1));
    }
    println!(
        "spiral of three turns refitted: {:.2} ms per edit",
        started.elapsed().as_secs_f64() * 100.0
    );
}

/// The window draws a formula curve and a live spline like other sketch
/// curves: polylines whose every point lies on the curve, ending on its
/// end points, and within the scene's tolerance between them.
#[test]
fn the_scene_draws_formula_curves_and_live_splines() {
    let mut s = craft3d_engine::Session::new(craft3d_engine::Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "C"}));
    run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "C", "x": "10 mm * cos(t)", "y": "10 mm * sin(t)", "t": [0, "pi"]}),
    );
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "C", "through": [[20, 0], [25, 5], [30, 0]], "degree": 2, "live": true}),
    );
    let scene = s.scene(None).unwrap().expect("a document");
    let lines = &scene.sketches[0].polylines;
    let arc = lines.iter().find(|p| p.entity == "e3").expect("the curve");
    assert_eq!(arc.curve, craft3d_engine::SketchCurve::Spline);
    assert!(arc.points.len() > 8, "{}", arc.points.len());
    for p in &arc.points {
        assert!((p.length() - 10.0).abs() <= 1e-6, "{p:?}");
    }
    for pair in arc.points.windows(2) {
        let sag = 10.0 - pair[0].lerp(pair[1], 0.5).length();
        assert!(sag <= scene.tolerance * 1.01, "{sag} {}", scene.tolerance);
    }
    let first = arc.points[0];
    let last = arc.points[arc.points.len() - 1];
    assert!(
        (first.x - 10.0).abs() < 1e-12 && first.y.abs() < 1e-12,
        "{first:?}"
    );
    assert!(
        (last.x + 10.0).abs() < 1e-9 && last.y.abs() < 1e-9,
        "{last:?}"
    );
    let live = lines
        .iter()
        .find(|p| p.entity == "e7")
        .unwrap_or_else(|| panic!("the live spline: {lines:?}"));
    assert_eq!(live.points[0].x, 20.0);
    assert_eq!(live.points[live.points.len() - 1].x, 30.0);
}
