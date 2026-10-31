//! Lofts, read by FreeCAD: the solids of `tests/loft.rs` built by
//! `feature.loft`, exported as STEP (and as STL and 3MF), at the scales
//! 1e-3, 1 and 1e6. Where a loft has a closed form ours is held to it
//! here as well; FreeCAD has to find a valid solid with our counts and
//! our measures, body by body.
//!
//! ```sh
//! CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo test --release -p craft3d-engine --test step_freecad lofts -- --ignored --nocapture
//! ```
//!
//! The report is left in `target/tmp/step-loft-files/report.md`.

// A ratio that is not a number is a failure too: `!(x <= bound)`.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::features::check_meshes;
use super::step;
use super::{APPROX, EXACT, Expect, FLAT, SPLINE, SPLINE_QUADRATURE, Set, harness};
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::{PI, TAU};
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

fn polygon(s: &mut Session, name: &str, z: f64, points: &[[f64; 2]]) -> Result<(), String> {
    step(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": z, "name": name}),
    )?;
    step(
        s,
        "sketch.polyline",
        json!({"sketch": name, "closed": true, "points": points}),
    )?;
    Ok(())
}

fn square(half: f64) -> Vec<[f64; 2]> {
    vec![[-half, -half], [half, -half], [half, half], [-half, half]]
}

fn circle(s: &mut Session, name: &str, z: f64, center: [f64; 2], r: f64) -> Result<(), String> {
    step(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": z, "name": name}),
    )?;
    step(
        s,
        "sketch.circle",
        json!({"sketch": name, "center": center, "radius": r}),
    )?;
    Ok(())
}

fn leaf(s: &mut Session, name: &str, z: f64, k: f64, fat: f64) -> Result<(), String> {
    step(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": z, "name": name}),
    )?;
    step(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": [[-2.0 * k, 0.0], [-k, -fat * k], [0.5 * k, -1.2 * fat * k], [2.0 * k, 0.0]]}),
    )?;
    step(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": ["e4", [k, fat * k], [-k, 0.8 * fat * k], "e1"]}),
    )?;
    Ok(())
}

fn loft(s: &mut Session, sections: &[&str], more: Value, name: &str) -> Result<(), String> {
    let mut params = more;
    params["sections"] = json!(sections);
    params["name"] = name.into();
    step(s, "feature.loft", params)?;
    Ok(())
}

fn the_box(s: &mut Session, k: f64) -> Forms {
    polygon(s, "Low", 0.0, &square(k))?;
    polygon(s, "High", 3.0 * k, &square(k))?;
    loft(s, &["Low", "High"], json!({}), "Box")?;
    Ok(vec![Closed {
        body: "Box",
        volume: 12.0,
        area: Some(32.0),
    }])
}

fn frustum(s: &mut Session, k: f64) -> Forms {
    let (r0, r1, h): (f64, f64, f64) = (2.0, 0.75, 3.0);
    circle(s, "Low", 0.0, [k, -k], r0 * k)?;
    circle(s, "High", h * k, [k, -k], r1 * k)?;
    loft(s, &["Low", "High"], json!({}), "Frustum")?;
    let slant = (h * h + (r0 - r1) * (r0 - r1)).sqrt();
    Ok(vec![Closed {
        body: "Frustum",
        volume: PI * h / 3.0 * (r0 * r0 + r0 * r1 + r1 * r1),
        area: Some(PI * (r0 + r1) * slant + PI * (r0 * r0 + r1 * r1)),
    }])
}

fn pyramid(s: &mut Session, k: f64) -> Forms {
    let (a, b, h): (f64, f64, f64) = (2.0, 0.5, 5.0);
    polygon(s, "Low", 0.0, &square(a * k))?;
    polygon(s, "High", h * k, &square(b * k))?;
    loft(s, &["Low", "High"], json!({}), "Pyramid")?;
    let (a1, a2) = (4.0 * a * a, 4.0 * b * b);
    let slant = (h * h + (a - b) * (a - b)).sqrt();
    Ok(vec![Closed {
        body: "Pyramid",
        volume: h / 3.0 * (a1 + a2 + (a1 * a2).sqrt()),
        area: Some(a1 + a2 + 4.0 * (a + b) * slant),
    }])
}

/// A rectangle to itself turned by 30 degrees: see `twisted_volume` in
/// `tests/loft.rs`.
fn twist(s: &mut Session, k: f64) -> Forms {
    let corners = [[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]];
    let (sin, cos) = 30f64.to_radians().psin_cos();
    let above: Vec<[f64; 2]> = corners
        .iter()
        .map(|p| [p[0] * cos - p[1] * sin, p[0] * sin + p[1] * cos])
        .collect();
    let scaled = |points: &[[f64; 2]]| -> Vec<[f64; 2]> {
        points.iter().map(|p| [p[0] * k, p[1] * k]).collect()
    };
    polygon(s, "Low", 0.0, &scaled(&corners))?;
    polygon(s, "High", 4.0 * k, &scaled(&above))?;
    loft(s, &["Low", "High"], json!({}), "Twist")?;
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    let (mut a0, mut a1, mut mixed) = (0.0, 0.0, 0.0);
    for i in 0..4 {
        let j = (i + 1) % 4;
        a0 += 0.5 * cross(corners[i], corners[j]);
        a1 += 0.5 * cross(above[i], above[j]);
        mixed += 0.5 * (cross(corners[i], above[j]) + cross(above[i], corners[j]));
    }
    Ok(vec![Closed {
        body: "Twist",
        volume: 4.0 * (a0 / 3.0 + mixed / 6.0 + a1 / 3.0),
        area: None,
    }])
}

/// Circles whose radii grow evenly: smooth, the cone as a B-spline
/// surface in four faces.
fn smooth_cone(s: &mut Session, k: f64) -> Forms {
    for i in 0..4 {
        let z = f64::from(i) * 2.0;
        circle(s, &format!("S{i}"), z * k, [0.0, 0.0], (1.0 + 0.25 * z) * k)?;
    }
    loft(s, &["S0", "S1", "S2", "S3"], json!({}), "Cone")?;
    let (r0, r1, h): (f64, f64, f64) = (1.0, 2.5, 6.0);
    let slant = (h * h + (r1 - r0) * (r1 - r0)).sqrt();
    Ok(vec![Closed {
        body: "Cone",
        volume: PI * h / 3.0 * (r0 * r0 + r0 * r1 + r1 * r1),
        area: Some(PI * (r0 + r1) * slant + PI * (r0 * r0 + r1 * r1)),
    }])
}

/// A vase: circles that widen, narrow and move off the axis.
fn vase(s: &mut Session, k: f64) -> Forms {
    let rings = [
        ([0.0, 0.0], 2.0, 0.0),
        ([0.3, 0.0], 3.0, 2.0),
        ([0.0, 0.4], 1.2, 5.0),
        ([0.0, 0.0], 1.6, 6.5),
        ([-0.2, 0.0], 0.8, 9.0),
    ];
    let mut names = Vec::new();
    for (i, (c, r, z)) in rings.iter().enumerate() {
        let name = format!("S{i}");
        circle(s, &name, z * k, [c[0] * k, c[1] * k], r * k)?;
        names.push(name);
    }
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    loft(s, &names, json!({}), "Vase")?;
    Ok(Vec::new())
}

/// A square, a circle and a square: the circle cut in quarters.
fn square_to_circle(s: &mut Session, k: f64) -> Forms {
    polygon(s, "Foot", 0.0, &square(2.0 * k))?;
    circle(s, "Waist", 3.0 * k, [0.0, 0.0], k)?;
    polygon(s, "Head", 6.0 * k, &square(1.5 * k))?;
    loft(s, &["Foot", "Waist", "Head"], json!({}), "Transition")?;
    Ok(Vec::new())
}

fn leaves(s: &mut Session, k: f64) -> Forms {
    leaf(s, "Small", 0.0, k, 1.0)?;
    leaf(s, "Large", 4.0 * k, 1.5 * k, 1.4)?;
    leaf(s, "Tip", 7.0 * k, 0.8 * k, 0.7)?;
    loft(s, &["Small", "Large", "Tip"], json!({}), "Leaf")?;
    Ok(Vec::new())
}

/// The rectangle 3..5 from the z axis, 2 high, on planes through that
/// axis at `angles` (degrees): its area times the distance of its centre
/// is 16.
fn spokes(s: &mut Session, k: f64, angles: &[f64]) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for (i, angle) in angles.iter().enumerate() {
        let plane = format!("P{i}");
        let made = step(
            s,
            "datum.plane",
            json!({"from": "zx", "about": "z", "angle": format!("{angle} deg"), "name": plane}),
        )?;
        // Along the axis the plane's x or y is z: away from the axis is
        // the other one.
        let radial_is_x = made["datum"]["x"][2].as_f64().unwrap_or(0.0).abs() < 0.5;
        let (x, y) = if radial_is_x {
            ([3.0 * k, 5.0 * k], [-k, k])
        } else {
            ([-k, k], [3.0 * k, 5.0 * k])
        };
        let name = format!("On{plane}");
        step(s, "sketch.create", json!({"plane": plane, "name": name}))?;
        step(
            s,
            "sketch.polyline",
            json!({"sketch": name, "closed": true,
                   "points": [[x[0], y[0]], [x[1], y[0]], [x[1], y[1]], [x[0], y[1]]]}),
        )?;
        names.push(name);
    }
    Ok(names)
}

fn bend(ruled: bool) -> impl Fn(&mut Session, f64) -> Forms {
    move |s, k| {
        let angles = [0.0, 30.0, 65.0, 90.0];
        let names = spokes(s, k, &angles)?;
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        loft(s, &names, json!({"ruled": ruled}), "Bend")?;
        if !ruled {
            return Ok(Vec::new());
        }
        let volume: f64 = angles
            .windows(2)
            .map(|w| (w[1] - w[0]).to_radians().psin() * 16.0)
            .sum();
        Ok(vec![Closed {
            body: "Bend",
            volume,
            area: None,
        }])
    }
}

fn ring(ruled: bool) -> impl Fn(&mut Session, f64) -> Forms {
    move |s, k| {
        let angles: Vec<f64> = (0..6).map(|i| 60.0 * f64::from(i)).collect();
        let names = spokes(s, k, &angles)?;
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        loft(s, &names, json!({"ruled": ruled, "closed": true}), "Ring")?;
        if !ruled {
            return Ok(Vec::new());
        }
        Ok(vec![Closed {
            body: "Ring",
            volume: 6.0 * (TAU / 6.0).psin() * 16.0,
            area: None,
        }])
    }
}

/// The leaf with a round hole through it and a block on it: lofts in
/// booleans.
fn leaf_drilled(s: &mut Session, k: f64) -> Forms {
    leaves(s, k)?;
    step(
        s,
        "feature.cylinder",
        json!({"position": [0.3 * k, 0.1 * k, -k], "radius": 0.6 * k, "height": 10.0 * k, "name": "Drill"}),
    )?;
    step(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Leaf", "tool": "Drill", "name": "Drilled"}),
    )?;
    step(
        s,
        "feature.block",
        json!({"position": [-0.5 * k, -3.0 * k, 2.0 * k], "size": [4.0 * k, 6.0 * k, 3.0 * k], "name": "Block"}),
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
fn freecad_reads_lofts() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-loft");
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
        ("loft-box", FLAT, Box::new(the_box)),
        ("loft-frustum", EXACT, Box::new(frustum)),
        ("loft-pyramid", FLAT, Box::new(pyramid)),
        ("loft-twist", EXACT, Box::new(twist)),
        ("loft-smooth-cone", EXACT, Box::new(smooth_cone)),
        // FreeCAD's own integration of these faces (rational, cubic
        // along the loft with a knot inside) is 6.0e-6 off in volume and
        // 3.3e-5 in area, alike at every scale; what it imported is our
        // solid, as the quadrature over its geometry shows (the file has
        // `spline` in its name for that), and ours agrees to 1.1e-8 with
        // Simpson's rule over the surfaces in the kernel's loft tests.
        ("loft-spline-surface-vase", SPLINE, Box::new(vase)),
        ("loft-square-circle", EXACT, Box::new(square_to_circle)),
        // Caps bounded by splines: see `SPLINE`.
        ("loft-spline-leaf", SPLINE, Box::new(leaves)),
        ("loft-bend-ruled", FLAT, Box::new(bend(true))),
        ("loft-bend-smooth", EXACT, Box::new(bend(false))),
        ("loft-ring-ruled", FLAT, Box::new(ring(true))),
        ("loft-ring-smooth", EXACT, Box::new(ring(false))),
        ("loft-leaf-drilled", APPROX, Box::new(leaf_drilled)),
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
                let volume = part.volume / (form.volume * k.ppowi(3)) - 1.0;
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
        "step-loft-files",
        "STEP: lofts, read by FreeCAD",
        &set.cases,
    );
    let files = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-loft-files");
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
