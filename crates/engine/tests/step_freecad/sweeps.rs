//! Sweeps along a path, read by FreeCAD: the solids of `tests/sweep.rs`
//! built by `feature.sweep`, exported as STEP (and as STL and 3MF), at
//! the scales 1e-3, 1 and 1e6. Where a sweep has a closed form ours is
//! held to it here as well; FreeCAD has to find a valid solid with our
//! counts and our measures, body by body.
//!
//! ```sh
//! CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo test --release -p craft3d-engine --test step_freecad sweeps -- --ignored --nocapture
//! ```
//!
//! The report is left in `target/tmp/step-sweep-files/report.md`.

// A ratio that is not a number is a failure too: `!(x <= bound)`.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::features::check_meshes;
use super::{APPROX, EXACT, Expect, FLAT, RING, SPLINE, SPLINE_QUADRATURE, Set, harness};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::fmt::Write as _;
use std::path::PathBuf;

/// A body's volume and area for lengths of 1, by a formula that owes
/// nothing to the kernel.
struct Closed {
    body: &'static str,
    volume: f64,
    area: Option<f64>,
}

type Forms = Result<Vec<Closed>, String>;

fn step(s: &mut Session, id: &str, params: Value) -> Result<Value, String> {
    let result = s
        .execute(id, &params)
        .map_err(|e| format!("{id} {params}: {e}"))?;
    if result["status"] == "failed" {
        return Err(format!(
            "{id} {params}: {}",
            result["error"].as_str().unwrap_or("failed")
        ));
    }
    Ok(result)
}

/// A profile on the zx plane: `u` is above the path's plane, `v` to the
/// right of a path that leaves the origin along +y.
fn round(s: &mut Session, name: &str, center: [f64; 2], r: f64) -> Result<(), String> {
    if s.execute("sketch.inspect", &json!({"sketch": name}))
        .is_err()
    {
        step(s, "sketch.create", json!({"plane": "zx", "name": name}))?;
    }
    step(
        s,
        "sketch.circle",
        json!({"sketch": name, "center": center, "radius": r}),
    )?;
    Ok(())
}

fn rectangle(s: &mut Session, name: &str, u: [f64; 2], v: [f64; 2]) -> Result<(), String> {
    step(s, "sketch.create", json!({"plane": "zx", "name": name}))?;
    step(
        s,
        "sketch.polyline",
        json!({"sketch": name, "closed": true,
               "points": [[u[0], v[0]], [u[0], v[1]], [u[1], v[1]], [u[1], v[0]]]}),
    )?;
    Ok(())
}

fn leaf(s: &mut Session, name: &str, k: f64) -> Result<(), String> {
    step(s, "sketch.create", json!({"plane": "zx", "name": name}))?;
    step(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": [[-2.0 * k, 0.0], [-k, -k], [0.0, -1.3 * k], [k, -k], [2.0 * k, 0.0]]}),
    )?;
    step(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": ["e5", [k, k], [0.0, 1.3 * k], [-k, k], "e1"]}),
    )?;
    Ok(())
}

fn path(s: &mut Session, name: &str, start: [f64; 2], segments: Value) -> Result<(), String> {
    step(s, "sketch.create", json!({"plane": "xy", "name": name}))?;
    step(
        s,
        "sketch.path",
        json!({"sketch": name, "start": start, "segments": segments}),
    )?;
    Ok(())
}

/// 10 along +y, a quarter bend of radius 5 to the left, 12 along −x.
fn bent(s: &mut Session, k: f64) -> Result<(), String> {
    path(
        s,
        "Route",
        [0.0, 0.0],
        json!([
            {"line": [0.0, 10.0 * k]},
            {"arc": [-5.0 * k, 15.0 * k], "center": [-5.0 * k, 10.0 * k]},
            {"line": [-17.0 * k, 15.0 * k]},
        ]),
    )
}

fn bent_length(right: f64) -> f64 {
    10.0 + (5.0 + right) * FRAC_PI_2 + 12.0
}

fn cornered(s: &mut Session, k: f64, degrees: f64) -> Result<(), String> {
    let turn = degrees.to_radians();
    path(
        s,
        "Route",
        [0.0, 0.0],
        json!([
            {"line": [0.0, 10.0 * k]},
            {"line": [-8.0 * k * turn.sin(), (10.0 + 8.0 * turn.cos()) * k]},
        ]),
    )
}

fn rounded_rectangle(s: &mut Session, k: f64) -> Result<(), String> {
    let at = |x: f64, y: f64| json!([x * k, y * k]);
    path(
        s,
        "Route",
        [0.0, -7.0 * k],
        json!([
            {"line": at(0.0, 7.0)},
            {"arc": at(-3.0, 10.0), "center": at(-3.0, 7.0)},
            {"line": at(-9.0, 10.0)},
            {"arc": at(-12.0, 7.0), "center": at(-9.0, 7.0)},
            {"line": at(-12.0, -7.0)},
            {"arc": at(-9.0, -10.0), "center": at(-9.0, -7.0)},
            {"line": at(-3.0, -10.0)},
            {"arc": "start", "center": at(-3.0, -7.0)},
        ]),
    )
}

fn wave(s: &mut Session, k: f64) -> Result<(), String> {
    step(s, "sketch.create", json!({"plane": "xy", "name": "Route"}))?;
    let control: Vec<[f64; 2]> = [
        [0.0, 0.0],
        [0.0, 4.0],
        [-3.0, 7.0],
        [-3.0, 11.0],
        [1.0, 14.0],
        [1.0, 18.0],
    ]
    .iter()
    .map(|p: &[f64; 2]| [p[0] * k, p[1] * k])
    .collect();
    step(
        s,
        "sketch.spline",
        json!({"sketch": "Route", "control": control}),
    )?;
    Ok(())
}

fn sweep(s: &mut Session, name: &str) -> Result<(), String> {
    step(
        s,
        "feature.sweep",
        json!({"sketch": "Section", "path": "Route", "name": name}),
    )?;
    Ok(())
}

fn pipe(s: &mut Session, k: f64) -> Forms {
    round(s, "Section", [0.0, 0.0], k)?;
    bent(s, k)?;
    sweep(s, "Pipe")?;
    Ok(vec![Closed {
        body: "Pipe",
        volume: PI * bent_length(0.0),
        area: Some(TAU * bent_length(0.0) + TAU),
    }])
}

/// A pipe with a bore, off the path.
fn tube(s: &mut Session, k: f64) -> Forms {
    round(s, "Section", [0.0, 0.5 * k], k)?;
    round(s, "Section", [0.0, 0.5 * k], 0.6 * k)?;
    bent(s, k)?;
    sweep(s, "Tube")?;
    let length = bent_length(0.5);
    Ok(vec![Closed {
        body: "Tube",
        volume: PI * (1.0 - 0.36) * length,
        area: Some(TAU * 1.6 * length + 2.0 * PI * (1.0 - 0.36)),
    }])
}

fn bar(s: &mut Session, k: f64) -> Forms {
    rectangle(s, "Section", [-k, k], [k, 4.0 * k])?;
    bent(s, k)?;
    sweep(s, "Bar")?;
    Ok(vec![Closed {
        body: "Bar",
        volume: 6.0 * bent_length(2.5),
        area: Some(2.0 * (bent_length(1.0) + bent_length(4.0)) + 6.0 * bent_length(2.5) + 12.0),
    }])
}

fn blade(s: &mut Session, k: f64) -> Forms {
    leaf(s, "Section", k)?;
    bent(s, k)?;
    sweep(s, "Blade")?;
    Ok(Vec::new())
}

fn mitred_rod(degrees: f64) -> impl Fn(&mut Session, f64) -> Forms {
    move |s, k| {
        round(s, "Section", [0.0, 0.25 * k], 0.5 * k)?;
        cornered(s, k, degrees)?;
        sweep(s, "Rod")?;
        let travel = 18.0 + 0.5 * (degrees.to_radians() / 2.0).tan();
        Ok(vec![Closed {
            body: "Rod",
            volume: PI * 0.25 * travel,
            area: Some(PI * travel + PI * 0.5),
        }])
    }
}

fn mitred_bar(degrees: f64) -> impl Fn(&mut Session, f64) -> Forms {
    move |s, k| {
        rectangle(s, "Section", [-0.5 * k, 0.5 * k], [-0.5 * k, k])?;
        cornered(s, k, degrees)?;
        sweep(s, "Bar")?;
        let travel = |right: f64| 18.0 + 2.0 * right * (degrees.to_radians() / 2.0).tan();
        Ok(vec![Closed {
            body: "Bar",
            volume: 1.5 * travel(0.25),
            area: Some(3.0 * travel(0.25) + travel(-0.5) + travel(1.0) + 3.0),
        }])
    }
}

fn gasket(s: &mut Session, k: f64) -> Forms {
    round(s, "Section", [0.0, 0.0], 0.8 * k)?;
    rounded_rectangle(s, k)?;
    sweep(s, "Gasket")?;
    let length = 40.0 + TAU * 3.0;
    Ok(vec![Closed {
        body: "Gasket",
        volume: PI * 0.64 * length,
        area: Some(TAU * 0.8 * length),
    }])
}

fn frame(s: &mut Session, k: f64) -> Forms {
    rectangle(s, "Section", [0.0, k], [-2.0 * k, 0.0])?;
    step(s, "sketch.create", json!({"plane": "xy", "name": "Route"}))?;
    step(
        s,
        "sketch.polyline",
        json!({"sketch": "Route", "closed": true, "points":
            [[0.0, -5.0 * k], [0.0, 5.0 * k], [-16.0 * k, 5.0 * k], [-16.0 * k, -5.0 * k]]}),
    )?;
    sweep(s, "Frame")?;
    Ok(vec![Closed {
        body: "Frame",
        volume: 88.0,
        area: Some(2.0 * 88.0 + 52.0 + 36.0),
    }])
}

/// A square bar round a circle.
fn ring(s: &mut Session, k: f64) -> Forms {
    rectangle(s, "Section", [-k, k], [-k, k])?;
    step(s, "sketch.create", json!({"plane": "xy", "name": "Route"}))?;
    step(
        s,
        "sketch.circle",
        json!({"sketch": "Route", "center": [-6.0 * k, 0.0], "radius": 6.0 * k}),
    )?;
    sweep(s, "Ring")?;
    Ok(vec![Closed {
        body: "Ring",
        volume: 4.0 * TAU * 6.0,
        area: Some(8.0 * TAU * 6.0),
    }])
}

fn hose(s: &mut Session, k: f64) -> Forms {
    round(s, "Section", [0.0, 0.0], 0.5 * k)?;
    wave(s, k)?;
    sweep(s, "Hose")?;
    Ok(Vec::new())
}

/// A rectangle along a spline: its flat sides are planes bounded by the
/// curves the corners sweep.
fn rail(s: &mut Session, k: f64) -> Forms {
    rectangle(s, "Section", [-0.4 * k, 0.4 * k], [-0.6 * k, 0.6 * k])?;
    wave(s, k)?;
    sweep(s, "Rail")?;
    Ok(Vec::new())
}

/// A bar with a hole drilled across the bend and a block on its first
/// leg: sweeps in booleans.
fn bar_drilled(s: &mut Session, k: f64) -> Forms {
    rectangle(s, "Section", [-k, k], [-k, k])?;
    bent(s, k)?;
    sweep(s, "Bar")?;
    step(
        s,
        "feature.cylinder",
        json!({"position": [-2.0 * k, 12.5 * k, -3.0 * k], "radius": 0.5 * k, "height": 6.0 * k, "name": "Drill"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Bar", "tool": "Drill", "name": "Drilled"}),
    )?;
    step(
        s,
        "feature.block",
        json!({"position": [-3.0 * k, 3.0 * k, -0.5 * k], "size": [6.0 * k, 4.0 * k, 3.0 * k], "name": "Block"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "union", "target": "Drilled", "tool": "Block", "name": "Joined"}),
    )?;
    Ok(Vec::new())
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_sweeps() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-sweep");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut set = Set {
        dir: dir.clone(),
        cases: Vec::new(),
        refused: Vec::new(),
        unmeasured: Vec::new(),
        per_body: true,
        meshes: Some(Vec::new()),
    };
    let mut forms_text = String::from(
        "\n## Ours against the closed forms\n\n| file | body | volume - 1 | area - 1 |\n|---|---|---|---|\n",
    );
    let mut forms_bad = Vec::new();

    type Build<'a> = Box<dyn Fn(&mut Session, f64) -> Forms + 'a>;
    let cases: Vec<(&str, Expect, Build)> = vec![
        // A face on a torus: FreeCAD's box is not compared (`RING`).
        ("sweep-pipe", RING, Box::new(pipe)),
        ("sweep-tube", RING, Box::new(tube)),
        ("sweep-bar", EXACT, Box::new(bar)),
        // Caps bounded by splines, and faces of revolution of splines:
        // see `SPLINE`.
        ("sweep-spline-blade", SPLINE, Box::new(blade)),
        // Two cylinders that meet in an ellipse, which is not a line in
        // the parameter plane of a cylinder: FreeCAD approximates such
        // an edge on import (`APPROXIMATED` in the harness has the
        // story), and its volume and area come back 4e-7 (60°) and
        // 1.2e-6 (120°) larger than ours, alike at every scale. Ours is
        // the closed form to 1e-9 (the table below).
        ("sweep-mitre-rod-60", APPROX, Box::new(mitred_rod(60.0))),
        ("sweep-mitre-rod-120", APPROX, Box::new(mitred_rod(120.0))),
        ("sweep-mitre-bar-90", FLAT, Box::new(mitred_bar(90.0))),
        ("sweep-mitre-bar-120", FLAT, Box::new(mitred_bar(120.0))),
        ("sweep-gasket", RING, Box::new(gasket)),
        ("sweep-frame", FLAT, Box::new(frame)),
        ("sweep-ring", EXACT, Box::new(ring)),
        // Faces of degree 7 along a spline path: FreeCAD's own
        // integration of them is 7e-5 off in volume, alike at every
        // scale; what it imported is our solid, as the quadrature over
        // its geometry shows (the file has `spline` in its name for
        // that).
        ("sweep-spline-hose", SPLINE, Box::new(hose)),
        // Plane faces bounded by splines (the curves the corners sweep).
        ("sweep-spline-rail", SPLINE, Box::new(rail)),
        ("sweep-bar-drilled", APPROX, Box::new(bar_drilled)),
    ];
    for (k, tag) in [(1e-3, "milli"), (1.0, "one"), (1e6, "mega")] {
        for (name, expect, build) in &cases {
            let file = format!("{tag}-{name}");
            let mut forms = Vec::new();
            let before = set.cases.len();
            set.add(&file, "mm", k, *expect, |s, k| {
                forms = build(s, k)?;
                Ok(())
            });
            if set.cases.len() == before {
                continue;
            }
            let case = set.cases.last().unwrap();
            for form in &forms {
                let Some(part) = case.parts.iter().find(|p| p.name == form.body) else {
                    forms_bad.push(format!("{file}: no body {}", form.body));
                    continue;
                };
                let volume = part.volume / (form.volume * k.powi(3)) - 1.0;
                let area = form.area.map_or(0.0, |a| part.area / (a * k * k) - 1.0);
                writeln!(
                    forms_text,
                    "| {file} | {} | {volume:+.1e} | {} |",
                    form.body,
                    form.area
                        .map_or("-".to_string(), |_| format!("{area:+.1e}")),
                )
                .unwrap();
                if !(volume.abs() <= 1e-9 && area.abs() <= 1e-9) {
                    forms_bad.push(format!(
                        "{file}: {} is off its closed form: volume {volume:+e}, area {area:+e}",
                        form.body
                    ));
                }
            }
        }
    }
    let meshes = set.meshes.take();

    let rows = harness::check(
        "step-sweep-files",
        "STEP: sweeps along a path, read by FreeCAD",
        &set.cases,
    );
    let files = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-sweep-files");
    let mut report = std::fs::read_to_string(files.join("report.md")).unwrap();
    report.push_str(
        "\n## Body by body: FreeCAD's solid over ours\n\n| file | body | volume - 1 | area - 1 | centre off |\n|---|---|---|---|---|\n",
    );
    for row in &rows {
        for part in &row.parts {
            writeln!(
                report,
                "| {} | {} | {:+.1e} | {:+.1e} | {:.1e} |",
                row.file,
                part.name,
                part.volume_ratio - 1.0,
                part.area_ratio - 1.0,
                part.centre_off
            )
            .unwrap();
        }
    }
    report.push_str(&forms_text);

    let mut bad: Vec<String> = Vec::new();
    report.push_str("\n## By quadrature over FreeCAD's geometry\n\n");
    for row in rows.iter().filter(|r| r.file.contains("spline")) {
        match row.quadrature_volume_ratio {
            Some(ratio) => {
                writeln!(
                    report,
                    "- {}: the volume of FreeCAD's geometry over ours - 1 = {:+.2e}",
                    row.file,
                    ratio - 1.0
                )
                .unwrap();
                if !((ratio - 1.0).abs() <= SPLINE_QUADRATURE) {
                    bad.push(format!(
                        "{}: FreeCAD's geometry has {ratio} times our volume",
                        row.file
                    ));
                }
            }
            None => bad.push(format!("{}: no volume by quadrature", row.file)),
        }
    }
    let (mesh_text, mesh_bad) = check_meshes(&files, meshes.as_deref().unwrap());
    report.push_str(&mesh_text);
    for row in rows.iter().filter(|r| !r.problems.is_empty()) {
        bad.push(format!("{}: {}", row.file, row.problems.join("; ")));
    }
    bad.extend(mesh_bad);
    bad.extend(forms_bad);
    for (title, lines) in [
        ("refused by a command (not exported)", &set.refused),
        ("where our own measurement is missing", &set.unmeasured),
        ("PROBLEMS", &bad),
    ] {
        if !lines.is_empty() {
            writeln!(report, "\n## {} {title}\n", lines.len()).unwrap();
            for line in lines {
                writeln!(report, "- {line}").unwrap();
            }
        }
    }
    std::fs::write(files.join("report.md"), &report).unwrap();
    println!("{report}\nreport: {}", files.join("report.md").display());
    assert!(
        set.refused.is_empty(),
        "{} documents were refused:\n{}",
        set.refused.len(),
        set.refused.join("\n")
    );
    assert!(set.unmeasured.is_empty(), "{}", set.unmeasured.join("\n"));
    assert!(
        bad.is_empty(),
        "{} problems:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
