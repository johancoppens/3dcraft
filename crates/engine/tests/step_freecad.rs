//! Oracle 8 (docs/testing.md) for STEP, through the command layer: FreeCAD,
//! an independent kernel, imports what `file.export` writes and measures
//! it; per file it must find valid solids of our volume, area and centre
//! of mass, in millimetres whatever the document's unit, under our names.
//!
//! Opt-in, because it needs FreeCAD (`cargo xtask freecad` runs it):
//!
//! ```sh
//! CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo test --release -p craft3d-engine --test step_freecad -- --ignored --nocapture
//! ```
//!
//! The documents: every primitive, the bracket of `tests/acceptance.rs`,
//! extrusions with arcs and holes and of splines, revolved bodies (whole and part turns,
//! through apexes and poles), moved, mirrored and patterned bodies, and
//! the three booleans of pairs like those of the kernel's tests
//! (`crates/kernel/tests/boolean*.rs`), cavities included; all at the
//! scales 1e-3, 1 and 1e6. A boolean the kernel refuses is listed, not
//! failed: that is the kernel's work, not the export's. The tables are
//! printed and left in `target/tmp/step-engine-files/report.md`.
//!
//! A second test, in `step_freecad/features.rs`, has the documents of
//! extrusion ends, datums, spline profiles and slivers, compared body by
//! body, with closed forms and with the mesh exports
//! (`target/tmp/step-features-files/report.md`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "step_freecad/features.rs"]
mod features;
#[path = "../../io/tests/common/step_harness.rs"]
mod harness;

use craft3d_engine::{Access, Session, Workspace};
use harness::{APPROXIMATED, Case, Extent, RELATIVE};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// The documents built so far, and the booleans the kernel refused.
struct Set {
    dir: PathBuf,
    cases: Vec<Case>,
    refused: Vec<String>,
    /// Bodies our own measurements fail on: reported, since FreeCAD
    /// cannot be compared with nothing.
    unmeasured: Vec<String>,
    /// Hold FreeCAD's solids to our bodies one by one, not only in total
    /// (`harness::Part`).
    per_body: bool,
    /// When there: every document is also exported as STL and 3MF, for
    /// FreeCAD's mesh module to read (`features::check_meshes`).
    meshes: Option<Vec<features::MeshCase>>,
}

/// What a case expects of FreeCAD beyond the numbers.
#[derive(Clone, Copy)]
struct Expect {
    relative: f64,
    extent: Extent,
}

const EXACT: Expect = Expect {
    relative: RELATIVE,
    extent: Extent::Tight,
};
/// Planar faces only: the boxes must agree too.
const FLAT: Expect = Expect {
    relative: RELATIVE,
    extent: Extent::Exact,
};
/// A face on a torus: FreeCAD's box is not compared (see `Extent`).
const RING: Expect = Expect {
    relative: RELATIVE,
    extent: Extent::Unchecked,
};
/// An edge that is not a line in the parameter plane of its face.
const APPROX: Expect = Expect {
    relative: APPROXIMATED,
    extent: Extent::Tight,
};
/// A plane face bounded by a spline: a cap of an extruded spline profile.
/// FreeCAD imports it as a valid solid with our counts, but its own
/// integration of such a face is approximate, and not by the tolerance or
/// the size: the D with a cubic bulge comes back with its area 2.2e-7 off
/// ours and its volume 9.7e-5 off, at the scales 1e-3, 1 and 1e6 alike,
/// which no prism can have (its volume is the area of a cap times its
/// height); the leaf with a quartic side is 3.8e-4 off in area and 3.2e-4
/// in volume. So what is asked of those two numbers is only that they are
/// near, and the shape itself is checked by a measure that does not go
/// through that integration: the volume by Gauss-Legendre over the curves
/// and surfaces FreeCAD imported (`quadrature_volume` in
/// `step_report.py`), which must agree with ours to [`SPLINE_QUADRATURE`].
/// (Ours is pinned to 1e-9 against Green's theorem by de Boor and Simpson
/// in `tests/sketch_spline.rs`.)
///
/// Not FreeCAD's mesh of the solid, which was tried first: its volume
/// nears ours only as the deflection goes down (3e-6 off at 1e-4 of the
/// size), not alike at every scale (4e-3 off at 1e6), and the mesher's
/// memory explodes long before it is exact (1.6 GB at 1e-5 of the size,
/// past 3 GB at 1e-6, for some 6000 triangles).
const SPLINE: Expect = Expect {
    relative: 1e-3,
    extent: Extent::Tight,
};
/// Measured: 3.3e-14 at most, over both solids at the three scales (the D
/// is 139.2 mm³ and comes back as 139.20000000000002).
const SPLINE_QUADRATURE: f64 = 1e-9;

/// A zone of a sphere: a face between two circles about the sphere's
/// axis, which we write, as it is, with two bounds and no seam. FreeCAD
/// gives such a face a third wire on import (a whole meridian from pole to
/// pole and the two poles) and then measures its area 7.6e-6 short (the
/// faces around it are exact to 1e-16), where a cap (one circle) and a
/// cylinder between two circles come out exact. It still calls the solid
/// valid, and the volume is right to 1e-5: enough to know the bounds mean
/// what we think.
const ZONE: Expect = APPROX;

fn unit_in_mm(unit: &str) -> f64 {
    match unit {
        "um" => 1e-3,
        "mm" => 1.0,
        "cm" => 10.0,
        "m" => 1000.0,
        "in" => 25.4,
        "ft" => 304.8,
        other => panic!("unit {other}"),
    }
}

impl Set {
    /// Builds a document in `unit` with lengths times `k`, exports it and
    /// records what we say is in it. `build` returns why the document
    /// could not be built, for the booleans the kernel refuses.
    fn add(
        &mut self,
        name: &str,
        unit: &str,
        k: f64,
        expect: Expect,
        build: impl FnOnce(&mut Session, f64) -> Result<(), String>,
    ) {
        let mut s = Session::new(Workspace {
            read: Access::Root(self.dir.clone()),
            write: Access::Root(self.dir.clone()),
        });
        // The tolerance follows the size of what is modelled.
        run(
            &mut s,
            "file.new",
            json!({"units": unit, "tolerance": 1e-6 * k}),
        );
        if let Err(reason) = build(&mut s, k) {
            self.refused.push(format!("{name}: {reason}"));
            return;
        }
        let document = run(&mut s, "document.inspect", json!({}));
        for feature in document["features"].as_array().unwrap() {
            assert_ne!(feature["status"], "failed", "{name}: {feature}");
        }
        let check = run(&mut s, "document.check", json!({}));
        assert_eq!(check["ok"], true, "{name}: {check}");
        let file = format!("{name}.step");
        let exported = run(&mut s, "file.export", json!({"path": file}));
        let bytes = std::fs::read(self.dir.join(&file)).unwrap();
        // Pinned here as well: what was written parses back to the same
        // bytes, without cycles.
        let step = craft3d_step::read(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(step.reference_cycle(), None, "{name}");
        assert_eq!(craft3d_step::write(&step).unwrap().into_bytes(), bytes);

        let bodies = document["bodies"].as_array().unwrap();
        assert!(!bodies.is_empty(), "{name}: no bodies");
        let (mut volume, mut area) = (0.0, 0.0);
        let mut moment = [0.0; 3];
        let mut counts = (0, 0, 0);
        let mut bbox = Some([
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ]);
        let mut slack: f64 = 0.0;
        let mut names = Vec::new();
        let mut parts = Vec::new();
        for body in bodies {
            assert_eq!(body["kind"], "solid", "{name}: {body}");
            let Some(v) = body["volume"].as_f64() else {
                self.unmeasured
                    .push(format!("{name}: `mass_properties` gives no volume"));
                return;
            };
            volume += v;
            area += body["area"].as_f64().unwrap();
            for axis in 0..3 {
                moment[axis] += v * body["centroid"][axis].as_f64().unwrap();
            }
            counts.0 += body["faces"].as_u64().unwrap() as usize;
            counts.1 += body["edges"].as_u64().unwrap() as usize;
            counts.2 += body["vertices"].as_u64().unwrap() as usize;
            match (bbox.as_mut(), body["bbox"].as_array()) {
                (Some(bbox), Some(_)) => {
                    for axis in 0..3 {
                        bbox[axis] = bbox[axis].min(body["bbox"][0][axis].as_f64().unwrap());
                        bbox[axis + 3] =
                            bbox[axis + 3].max(body["bbox"][1][axis].as_f64().unwrap());
                    }
                    slack = slack.max(body["bboxTolerance"].as_f64().unwrap());
                }
                _ => {
                    if bbox.is_some() {
                        self.unmeasured.push(format!(
                            "{name}: `document.inspect` gives no bounding box (the body does not tessellate)"
                        ));
                    }
                    bbox = None;
                }
            }
            names.push(body["name"].as_str().unwrap().to_string());
            if self.per_body {
                parts.push(harness::Part {
                    name: body["name"].as_str().unwrap().to_string(),
                    volume: v,
                    area: body["area"].as_f64().unwrap(),
                    centroid: [0, 1, 2].map(|axis| body["centroid"][axis].as_f64().unwrap()),
                });
            }
        }
        if let Some(meshes) = self.meshes.as_mut() {
            for extension in ["stl", "3mf"] {
                let file = format!("{name}.{extension}");
                let exported = run(&mut s, "file.export", json!({"path": file}));
                meshes.push(features::MeshCase {
                    bytes: std::fs::read(self.dir.join(&file)).unwrap(),
                    file,
                    bodies: bodies.len(),
                    closed: exported["closed"].as_u64().unwrap() as usize,
                    triangles: exported["triangles"].as_u64().unwrap() as usize,
                    tolerance: exported["tolerance"].as_f64().unwrap(),
                    volume,
                    area,
                    bbox,
                });
            }
        }
        self.cases.push(Case {
            file,
            bytes,
            to_mm: unit_in_mm(unit),
            solid: true,
            solids: exported["solids"].as_u64().unwrap() as usize,
            volume,
            area,
            centroid: Some(moment.map(|m| m / volume)),
            bbox,
            // The box of `document.inspect` is that of a mesh: inside the
            // body by at most its chordal tolerance.
            bbox_slack: slack,
            extent: expect.extent,
            counts: (
                counts.0,
                counts.1,
                counts.2,
                exported["shells"].as_u64().unwrap() as usize,
            ),
            names,
            relative: expect.relative,
            parts,
        });
    }
}

type Built = Result<(), String>;

/// A sketch `Profile` on `plane` with a closed polygon through `points`.
fn polygon(s: &mut Session, plane: &str, k: f64, points: &[[f64; 2]]) {
    run(
        s,
        "sketch.create",
        json!({"name": "Profile", "plane": plane}),
    );
    let scaled: Vec<[f64; 2]> = points.iter().map(|p| [p[0] * k, p[1] * k]).collect();
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true, "points": scaled}),
    );
}

fn revolve(s: &mut Session, params: Value) {
    let mut params = params;
    params["sketch"] = "Profile".into();
    params["name"] = "Turned".into();
    run(s, "feature.revolve", params);
}

/// The bracket of `tests/acceptance.rs`: an L constrained by variables,
/// extruded, with two holes cut from a sketch on its top.
fn bracket(s: &mut Session, k: f64) -> Built {
    let mm = |x: f64| format!("{} mm", x * k);
    for (name, value) in [
        ("width", 40.0),
        ("height", 30.0),
        ("thickness", 5.0),
        ("hole_d", 3.0),
    ] {
        run(s, "variable.set", json!({"name": name, "expr": mm(value)}));
    }
    // Drawn in document units: a millimetre is `1 / unit` of them.
    let unit = run(s, "document.inspect", json!({}))["units"]["length"]
        .as_str()
        .unwrap()
        .to_string();
    let d = k / unit_in_mm(&unit);
    let constrain = |s: &mut Session, sketch: &str, kind: &str, entities: Value, extra: Value| {
        let mut params = json!({"sketch": sketch, "type": kind, "entities": entities});
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        let result = run(s, "sketch.constrain", params);
        assert_eq!(result["status"], "ok", "{result}");
    };
    run(s, "sketch.create", json!({"name": "Profile"}));
    let draft = [
        [0.0, 0.0],
        [38.0, 1.5],
        [41.0, 6.0],
        [4.0, 4.5],
        [6.5, 28.0],
        [-1.0, 31.0],
    ]
    .map(|[x, y]| [x * d, y * d]);
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true, "points": draft}),
    );
    constrain(s, "Profile", "fixed", json!(["e1"]), json!({}));
    for (line, kind) in [
        ("e7", "horizontal"),
        ("e8", "vertical"),
        ("e9", "horizontal"),
        ("e10", "vertical"),
        ("e11", "horizontal"),
        ("e12", "vertical"),
    ] {
        constrain(s, "Profile", kind, json!([line]), json!({}));
    }
    for (line, value) in [
        ("e7", "width"),
        ("e12", "height"),
        ("e8", "thickness"),
        ("e11", "thickness"),
    ] {
        constrain(
            s,
            "Profile",
            "distance",
            json!([line]),
            json!({"value": value}),
        );
    }
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": mm(8.0), "name": "Bracket"}),
    );
    run(
        s,
        "sketch.create",
        json!({"name": "Holes", "face": "F2:endCap()"}),
    );
    run(s, "sketch.point", json!({"sketch": "Holes", "at": [0, 0]}));
    constrain(s, "Holes", "fixed", json!(["e1"]), json!({}));
    for (at, across, up) in [
        ([18.0, 3.0], "width / 2", "thickness / 2"),
        ([3.0, 17.0], "thickness / 2", "height / 2"),
    ] {
        let circle = run(
            s,
            "sketch.circle",
            json!({"sketch": "Holes", "center": [at[0] * d, at[1] * d], "radius": d}),
        );
        let (center, id) = (circle["center"].clone(), circle["entity"].clone());
        constrain(
            s,
            "Holes",
            "diameter",
            json!([id]),
            json!({"value": "hole_d"}),
        );
        constrain(
            s,
            "Holes",
            "distance",
            json!(["e1", center]),
            json!({"value": across, "direction": "horizontal"}),
        );
        constrain(
            s,
            "Holes",
            "distance",
            json!(["e1", center]),
            json!({"value": up, "direction": "vertical"}),
        );
    }
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Holes", "distance": mm(-8.0), "cut": "Bracket", "name": "Part"}),
    );
    Ok(())
}

/// A stadium (two lines, two arcs) with a round hole and a square one.
fn slot_profile(s: &mut Session, plane: &str, k: f64) {
    run(
        s,
        "sketch.create",
        json!({"name": "Profile", "plane": plane}),
    );
    let p = |x: f64, y: f64| json!([x * k, y * k]);
    run(
        s,
        "sketch.line",
        json!({"sketch": "Profile", "from": p(0.0, 0.0), "to": p(20.0, 0.0)}),
    );
    run(
        s,
        "sketch.arc",
        json!({"sketch": "Profile", "center": p(20.0, 5.0), "start": p(20.0, 0.0), "end": p(20.0, 10.0)}),
    );
    run(
        s,
        "sketch.line",
        json!({"sketch": "Profile", "from": p(20.0, 10.0), "to": p(0.0, 10.0)}),
    );
    run(
        s,
        "sketch.arc",
        json!({"sketch": "Profile", "center": p(0.0, 5.0), "start": p(0.0, 10.0), "end": p(0.0, 0.0)}),
    );
    run(
        s,
        "sketch.circle",
        json!({"sketch": "Profile", "center": p(4.0, 5.0), "radius": 2.5 * k}),
    );
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true,
               "points": [p(12.0, 3.0), p(18.0, 3.0), p(18.0, 7.0), p(12.0, 7.0)]}),
    );
}

fn primitive(s: &mut Session, kind: &str, name: &str, k: f64, tilted: bool) {
    let mut params = match kind {
        "block" => json!({"size": [40.0 * k, 20.0 * k, 10.0 * k]}),
        "cylinder" => json!({"radius": 8.0 * k, "height": 30.0 * k}),
        "cone" => json!({"radius": 10.0 * k, "height": 25.0 * k}),
        "sphere" => json!({"radius": 12.0 * k}),
        "torus" => json!({"major": 15.0 * k, "minor": 4.0 * k}),
        other => panic!("{other}"),
    };
    params["name"] = name.into();
    if tilted {
        params["position"] = json!([7.0 * k, -3.0 * k, 5.0 * k]);
        if kind != "sphere" {
            params["axis"] = json!([0.3, -0.4, 1.0]);
            params["angle"] = "25 deg".into();
        }
    }
    run(s, &format!("feature.{kind}"), params);
}

/// A pair of primitives for the booleans: `(kind, params)` with lengths
/// to be scaled, each given by size and position.
#[derive(Clone)]
struct Solid {
    kind: &'static str,
    /// Lengths: the size parameters in the order the command names them.
    size: Vec<f64>,
    position: [f64; 3],
    axis: [f64; 3],
}

fn solid(kind: &'static str, size: &[f64], position: [f64; 3]) -> Solid {
    Solid {
        kind,
        size: size.to_vec(),
        position,
        axis: [0.0, 0.0, 1.0],
    }
}

fn make(s: &mut Session, solid: &Solid, name: &str, k: f64) {
    let z = &solid.size;
    let mut params = match solid.kind {
        "block" => json!({"size": [z[0] * k, z[1] * k, z[2] * k]}),
        "cylinder" | "cone" => json!({"radius": z[0] * k, "height": z[1] * k}),
        "sphere" => json!({"radius": z[0] * k}),
        "torus" => json!({"major": z[0] * k, "minor": z[1] * k}),
        other => panic!("{other}"),
    };
    params["name"] = name.into();
    params["position"] = json!(solid.position.map(|x| x * k));
    if solid.kind != "sphere" {
        params["axis"] = json!(solid.axis);
    }
    run(s, &format!("feature.{}", solid.kind), params);
}

/// `A op B` as the document's one body, or why the kernel refused.
fn boolean(s: &mut Session, a: &Solid, b: &Solid, op: &str, k: f64) -> Built {
    make(s, a, "A", k);
    make(s, b, "B", k);
    run(
        s,
        "feature.boolean",
        json!({"op": op, "target": "A", "tool": "B", "name": "Result"}),
    );
    let document = run(s, "document.inspect", json!({}));
    let feature = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Result")
        .unwrap();
    if feature["status"] == "failed" {
        return Err(feature["error"].as_str().unwrap_or("failed").to_string());
    }
    Ok(())
}

#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_what_the_commands_build() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-engine");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut set = Set {
        dir,
        cases: Vec::new(),
        refused: Vec::new(),
        unmeasured: Vec::new(),
        per_body: false,
        meshes: None,
    };

    for (k, tag) in [(1e-3, "milli"), (1.0, "one"), (1e6, "mega")] {
        // Every primitive, upright and tilted, and all five in one file.
        for (kind, expect) in [
            ("block", FLAT),
            ("cylinder", EXACT),
            ("cone", EXACT),
            ("sphere", EXACT),
            ("torus", RING),
        ] {
            for tilted in [false, true] {
                let name = format!("{tag}-{kind}{}", if tilted { "-tilted" } else { "" });
                set.add(&name, "mm", k, expect, |s, k| {
                    primitive(s, kind, "Body", k, tilted);
                    Ok(())
                });
            }
        }
        set.add(&format!("{tag}-five"), "mm", k, RING, |s, k| {
            for (i, kind) in ["block", "cylinder", "cone", "sphere", "torus"].iter().enumerate() {
                primitive(s, kind, &format!("The {kind}"), k, false);
                run(
                    s,
                    "feature.transform",
                    json!({"source": format!("The {kind}"), "translate": [60.0 * k * i as f64, 0, 0],
                           "name": format!("Moved {kind}")}),
                );
            }
            Ok(())
        });

        set.add(&format!("{tag}-bracket"), "mm", k, EXACT, bracket);

        // Extrusions: arcs and holes, on two planes, one way and both.
        set.add(&format!("{tag}-slot"), "mm", k, EXACT, |s, k| {
            slot_profile(s, "xy", k);
            run(
                s,
                "feature.extrude",
                json!({"sketch": "Profile", "distance": 6.0 * k, "name": "Slot"}),
            );
            Ok(())
        });
        set.add(&format!("{tag}-slot-yz-symmetric"), "mm", k, EXACT, |s, k| {
            slot_profile(s, "yz", k);
            run(
                s,
                "feature.extrude",
                json!({"sketch": "Profile", "distance": -9.0 * k, "symmetric": true, "name": "Slot"}),
            );
            Ok(())
        });

        // Extrusions of splines: the wall is a B-spline surface between
        // B-spline edges, the caps are planes bounded by them.
        set.add(&format!("{tag}-spline-d"), "mm", k, SPLINE, |s, k| {
            run(s, "sketch.create", json!({"name": "Profile", "plane": "zx"}));
            let p = |x: f64, y: f64| json!([x * k, y * k]);
            run(
                s,
                "sketch.spline",
                json!({"sketch": "Profile",
                       "control": [p(0.0, 0.0), p(5.0, -1.0), p(7.0, 3.0), p(5.0, 7.0), p(0.0, 6.0)]}),
            );
            run(
                s,
                "sketch.line",
                json!({"sketch": "Profile", "from": "e5", "to": "e1"}),
            );
            run(
                s,
                "feature.extrude",
                json!({"sketch": "Profile", "distance": 4.0 * k, "name": "Hull"}),
            );
            Ok(())
        });
        set.add(&format!("{tag}-spline-leaf"), "mm", k, SPLINE, |s, k| {
            run(s, "sketch.create", json!({"name": "Profile"}));
            let p = |x: f64, y: f64| json!([x * k, y * k]);
            run(
                s,
                "sketch.spline",
                json!({"sketch": "Profile", "through": [p(0.0, 0.0), p(2.0, 2.6), p(5.0, 3.4),
                       p(8.0, 2.2), p(10.0, 0.0)], "degree": 4}),
            );
            run(
                s,
                "sketch.spline",
                json!({"sketch": "Profile", "degree": 2,
                       "control": ["e1", p(3.0, -2.0), p(8.0, -1.5), "e5"]}),
            );
            run(
                s,
                "sketch.circle",
                json!({"sketch": "Profile", "center": p(5.0, 0.8), "radius": 1.2 * k}),
            );
            run(
                s,
                "feature.extrude",
                json!({"sketch": "Profile", "distance": -3.0 * k, "symmetric": true,
                       "name": "Leaf"}),
            );
            Ok(())
        });

        // Revolved: a ring, a cone, a sphere, a torus; part turns of
        // each, through apexes and poles; a profile with a hole.
        let rectangle = [[10.0, 0.0], [20.0, 0.0], [20.0, 5.0], [10.0, 5.0]];
        let triangle = [[0.0, 0.0], [6.0, 0.0], [0.0, 9.0]];
        let trapezium = [[3.0, 0.0], [8.0, 0.0], [5.0, 6.0], [3.0, 6.0]];
        for (name, angle, symmetric) in [
            ("ring", "360 deg", false),
            ("ring-90", "90 deg", false),
            ("ring-200-symmetric", "200 deg", true),
        ] {
            set.add(&format!("{tag}-{name}"), "mm", k, EXACT, |s, k| {
                polygon(s, "xy", k, &rectangle);
                revolve(
                    s,
                    json!({"axis": "y", "angle": angle, "symmetric": symmetric}),
                );
                Ok(())
            });
        }
        for (name, angle) in [("turned-cone", "360 deg"), ("turned-cone-270", "270 deg")] {
            set.add(&format!("{tag}-{name}"), "mm", k, EXACT, |s, k| {
                polygon(s, "zx", k, &triangle);
                revolve(s, json!({"axis": "y", "angle": angle}));
                Ok(())
            });
        }
        for (name, angle) in [("frustum", "360 deg"), ("frustum-45", "-45 deg")] {
            set.add(&format!("{tag}-{name}"), "mm", k, EXACT, |s, k| {
                polygon(s, "xy", k, &trapezium);
                revolve(s, json!({"axis": "y", "angle": angle}));
                Ok(())
            });
        }
        for (name, angle) in [
            ("turned-sphere", "360 deg"),
            ("turned-sphere-120", "120 deg"),
        ] {
            set.add(&format!("{tag}-{name}"), "mm", k, EXACT, |s, k| {
                run(s, "sketch.create", json!({"name": "Profile"}));
                run(
                    s,
                    "sketch.arc",
                    json!({"sketch": "Profile", "center": [0, 2.0 * k], "start": [0, -3.0 * k], "end": [0, 7.0 * k]}),
                );
                run(s, "sketch.line", json!({"sketch": "Profile", "from": "e3", "to": "e2"}));
                revolve(s, json!({"axis": "y", "angle": angle}));
                Ok(())
            });
        }
        for (name, angle) in [("turned-torus", "360 deg"), ("turned-torus-270", "270 deg")] {
            set.add(&format!("{tag}-{name}"), "mm", k, RING, |s, k| {
                run(
                    s,
                    "sketch.create",
                    json!({"name": "Profile", "plane": "yz"}),
                );
                run(
                    s,
                    "sketch.circle",
                    json!({"sketch": "Profile", "center": [12.0 * k, k], "radius": 3.0 * k}),
                );
                revolve(s, json!({"axis": "y", "angle": angle}));
                Ok(())
            });
        }
        set.add(
            &format!("{tag}-ring-with-bore-180"),
            "mm",
            k,
            RING,
            |s, k| {
                polygon(s, "xy", k, &rectangle);
                run(
                    s,
                    "sketch.circle",
                    json!({"sketch": "Profile", "center": [15.0 * k, 2.5 * k], "radius": k}),
                );
                revolve(s, json!({"axis": "y", "angle": "180 deg"}));
                Ok(())
            },
        );

        // Moved, mirrored, patterned.
        set.add(&format!("{tag}-moved"), "mm", k, EXACT, |s, k| {
            primitive(s, "cone", "Cone", k, true);
            run(
                s,
                "feature.transform",
                json!({"source": "Cone", "translate": [5.0 * k, 6.0 * k, -7.0 * k], "angle": "40 deg",
                       "axis": [1, 1, 0], "pivot": [k, 2.0 * k, 3.0 * k], "name": "Moved"}),
            );
            Ok(())
        });
        set.add(&format!("{tag}-mirrored"), "mm", k, EXACT, |s, k| {
            slot_profile(s, "xy", k);
            run(s, "feature.extrude", json!({"sketch": "Profile", "distance": 6.0 * k, "name": "Slot"}));
            run(
                s,
                "feature.mirror",
                json!({"source": "Slot", "normal": [1, 2, 3], "point": [0, 0, 20.0 * k], "name": "Mirrored"}),
            );
            Ok(())
        });
        set.add(&format!("{tag}-mirrored-cone-kept"), "mm", k, EXACT, |s, k| {
            primitive(s, "cone", "Cone", k, true);
            run(
                s,
                "feature.mirror",
                json!({"source": "Cone", "normal": [0, 0, 1], "point": [0, 0, -30.0 * k], "keep": true,
                       "name": "Mirrored"}),
            );
            Ok(())
        });
        set.add(&format!("{tag}-pattern-row"), "mm", k, EXACT, |s, k| {
            primitive(s, "cylinder", "Pin", k, false);
            run(
                s,
                "feature.pattern",
                json!({"source": "Pin", "count": 4, "step": [25.0 * k, 5.0 * k, 0], "name": "Pins"}),
            );
            Ok(())
        });
        set.add(&format!("{tag}-pattern-ring"), "mm", k, RING, |s, k| {
            run(
                s,
                "feature.torus",
                json!({"major": 6.0 * k, "minor": 2.0 * k, "position": [30.0 * k, 0, 0],
                       "axis": [1, 0, 0], "name": "Link"}),
            );
            run(
                s,
                "feature.pattern",
                json!({"source": "Link", "count": 5, "angle": "72 deg", "name": "Links"}),
            );
            Ok(())
        });

        // Booleans: pairs like those of the kernel's tests. `flat` and
        // the like say what FreeCAD can be held to for each operation's
        // result (union, subtract, intersect).
        let block = solid("block", &[40.0, 30.0, 20.0], [0.0, 0.0, 0.0]);
        let axis = |mut s: Solid, axis: [f64; 3]| {
            s.axis = axis;
            s
        };
        let pairs: Vec<(&str, Solid, Solid, [Expect; 3])> = vec![
            (
                "boxes-corner",
                block.clone(),
                solid("block", &[30.0, 20.0, 40.0], [20.0, 10.0, 5.0]),
                [FLAT; 3],
            ),
            (
                "boxes-crossing",
                block.clone(),
                solid("block", &[10.0, 50.0, 8.0], [15.0, -10.0, 6.0]),
                [FLAT; 3],
            ),
            (
                "box-tilted-box",
                block.clone(),
                axis(
                    solid("block", &[20.0, 20.0, 30.0], [20.0, 10.0, 5.0]),
                    [0.3, 0.2, 1.0],
                ),
                [FLAT; 3],
            ),
            (
                "box-cylinder-through",
                block.clone(),
                solid("cylinder", &[6.0, 40.0], [20.0, 15.0, -10.0]),
                [EXACT; 3],
            ),
            (
                "box-cylinder-blind",
                block.clone(),
                solid("cylinder", &[6.0, 20.0], [20.0, 15.0, 10.0]),
                [EXACT; 3],
            ),
            (
                "box-cylinder-oblique",
                block.clone(),
                axis(
                    solid("cylinder", &[5.0, 80.0], [10.0, 10.0, -20.0]),
                    [0.3, 0.2, 1.0],
                ),
                [APPROX; 3],
            ),
            (
                "box-cylinder-side",
                block.clone(),
                solid("cylinder", &[8.0, 30.0], [0.0, 15.0, -5.0]),
                [EXACT; 3],
            ),
            (
                "box-ball-inside",
                block.clone(),
                solid("sphere", &[6.0], [20.0, 15.0, 10.0]),
                [EXACT; 3],
            ),
            (
                "box-ball-top",
                block.clone(),
                solid("sphere", &[8.0], [20.0, 15.0, 20.0]),
                [EXACT; 3],
            ),
            (
                "box-ball-side",
                block.clone(),
                solid("sphere", &[8.0], [40.0, 15.0, 10.0]),
                [EXACT; 3],
            ),
            (
                "box-ball-corner",
                block.clone(),
                solid("sphere", &[9.0], [40.0, 30.0, 20.0]),
                [APPROX; 3],
            ),
            (
                "box-cone-through-top",
                block.clone(),
                solid("cone", &[8.0, 30.0], [20.0, 15.0, 5.0]),
                [EXACT; 3],
            ),
            (
                "box-cone-inside",
                block.clone(),
                solid("cone", &[6.0, 10.0], [20.0, 15.0, 5.0]),
                [EXACT; 3],
            ),
            (
                "box-cone-apex-in",
                block.clone(),
                solid("cone", &[10.0, 20.0], [20.0, 15.0, -10.0]),
                [EXACT; 3],
            ),
            (
                "box-cone-sideways",
                block.clone(),
                axis(
                    solid("cone", &[8.0, 30.0], [-10.0, 15.0, 10.0]),
                    [1.0, 0.0, 0.0],
                ),
                [EXACT; 3],
            ),
            (
                "box-ring-half-sunk",
                block.clone(),
                solid("torus", &[9.0, 3.0], [20.0, 15.0, 20.0]),
                [RING; 3],
            ),
            (
                "box-ring-inside",
                block.clone(),
                solid("torus", &[8.0, 3.0], [20.0, 15.0, 10.0]),
                [RING; 3],
            ),
            (
                "box-ring-upright",
                block.clone(),
                axis(
                    solid("torus", &[9.0, 3.0], [20.0, 15.0, 20.0]),
                    [0.0, 1.0, 0.0],
                ),
                [RING; 3],
            ),
            (
                "cylinders-coaxial",
                solid("cylinder", &[10.0, 30.0], [0.0, 0.0, 0.0]),
                solid("cylinder", &[6.0, 50.0], [0.0, 0.0, -10.0]),
                [EXACT; 3],
            ),
            (
                "cylinders-parallel",
                solid("cylinder", &[10.0, 30.0], [0.0, 0.0, 0.0]),
                solid("cylinder", &[8.0, 30.0], [9.0, 0.0, 10.0]),
                [EXACT; 3],
            ),
            (
                "cylinders-crossing",
                solid("cylinder", &[10.0, 40.0], [0.0, 0.0, -20.0]),
                axis(
                    solid("cylinder", &[6.0, 40.0], [-20.0, 0.0, 0.0]),
                    [1.0, 0.0, 0.0],
                ),
                [APPROX; 3],
            ),
            (
                "balls",
                solid("sphere", &[10.0], [0.0, 0.0, 0.0]),
                solid("sphere", &[8.0], [0.0, 0.0, 12.0]),
                [EXACT; 3],
            ),
            (
                "balls-askew",
                solid("sphere", &[10.0], [0.0, 0.0, 0.0]),
                solid("sphere", &[8.0], [7.0, 5.0, 6.0]),
                [EXACT; 3],
            ),
            (
                "ball-cylinder-coaxial",
                solid("sphere", &[10.0], [0.0, 0.0, 0.0]),
                solid("cylinder", &[6.0, 30.0], [0.0, 0.0, -15.0]),
                [ZONE, ZONE, EXACT],
            ),
            (
                "cone-cylinder-coaxial",
                solid("cone", &[10.0, 30.0], [0.0, 0.0, 0.0]),
                solid("cylinder", &[4.0, 40.0], [0.0, 0.0, -5.0]),
                [EXACT; 3],
            ),
            (
                "cone-ball-coaxial",
                solid("cone", &[10.0, 30.0], [0.0, 0.0, 0.0]),
                solid("sphere", &[8.0], [0.0, 0.0, 12.0]),
                [EXACT; 3],
            ),
            (
                "ring-cylinder-coaxial",
                solid("torus", &[12.0, 4.0], [0.0, 0.0, 0.0]),
                solid("cylinder", &[11.0, 20.0], [0.0, 0.0, -10.0]),
                [RING; 3],
            ),
            (
                "ring-block",
                solid("torus", &[12.0, 4.0], [0.0, 0.0, 0.0]),
                solid("block", &[40.0, 40.0, 10.0], [-20.0, -20.0, 0.0]),
                [RING; 3],
            ),
        ];
        for (name, a, b, expects) in &pairs {
            for (op, expect) in ["union", "subtract", "intersect"].iter().zip(expects) {
                set.add(&format!("{tag}-{name}-{op}"), "mm", k, *expect, |s, k| {
                    boolean(s, a, b, op, k)
                });
            }
        }
    }

    // Units: the 40 mm bracket from documents in every unit is 40 mm in
    // FreeCAD (the comparison is in millimetres: `Case::to_mm`).
    for unit in ["um", "mm", "cm", "m", "in", "ft"] {
        set.add(&format!("unit-{unit}-bracket"), unit, 1.0, EXACT, bracket);
        let case = set.cases.last().unwrap();
        let to_mm = case.to_mm;
        let bbox = case.bbox.expect("the bracket tessellates");
        for (ours, mm) in bbox.iter().zip([0.0, 0.0, 0.0, 40.0, 30.0, 8.0]) {
            assert!((ours * to_mm - mm).abs() < 1e-6, "{unit}: {bbox:?}");
        }
        set.add(
            &format!("unit-{unit}-cone"),
            unit,
            1.0 / to_mm,
            EXACT,
            |s, k| {
                primitive(s, "cone", "Cone", k, true);
                Ok(())
            },
        );
    }

    let rows = harness::check(
        "step-engine-files",
        "STEP: documents built by commands, read by FreeCAD",
        &set.cases,
    );
    // The spline solids, by a quadrature over what FreeCAD imported (see
    // `SPLINE`).
    let splines: Vec<&harness::Row> = rows.iter().filter(|r| r.file.contains("spline")).collect();
    assert_eq!(splines.len(), 6, "two spline solids at three scales");
    for row in splines {
        let ratio = row
            .quadrature_volume_ratio
            .unwrap_or_else(|| panic!("{}: no volume by quadrature", row.file));
        println!(
            "{}: the volume of FreeCAD's geometry over ours - 1 = {:+.2e}",
            row.file,
            ratio - 1.0
        );
        assert!(
            (ratio - 1.0).abs() <= SPLINE_QUADRATURE,
            "{}: FreeCAD's geometry has {ratio} times our volume",
            row.file
        );
    }
    let mut report = String::new();
    if !set.refused.is_empty() {
        report.push_str(&format!(
            "\n{} booleans the kernel refused (not exported):\n",
            set.refused.len()
        ));
        for line in &set.refused {
            report.push_str(&format!("- {line}\n"));
        }
        println!("{report}");
    }
    if !set.unmeasured.is_empty() {
        let mut text = format!(
            "\n{} documents where our own measurement is missing:\n",
            set.unmeasured.len()
        );
        for line in &set.unmeasured {
            text.push_str(&format!("- {line}\n"));
        }
        println!("{text}");
        report.push_str(&text);
    }
    std::fs::write(set.dir.join("refused.md"), &report).unwrap();
    let bad: Vec<String> = rows
        .iter()
        .filter(|r| !r.problems.is_empty())
        .map(|r| format!("{}: {}", r.file, r.problems.join("; ")))
        .collect();
    assert!(
        bad.is_empty(),
        "{} of {} files with problems:\n{}",
        bad.len(),
        rows.len(),
        bad.join("\n")
    );
}

/// What a spline sweeps beyond a wall between two caps (2026-10-14): a
/// profile with splines in it turned about an axis (faces on rational
/// B-spline surfaces of revolution: a full turn, in two halves, and a
/// third of one, with caps bounded by splines), a spline that closes on
/// itself (cut in two) extruded and turned, and a polyline. Counts,
/// validity, box, area and volume as for every case; the volume also by
/// the quadrature over what FreeCAD imported, as for the other spline
/// solids (see [`SPLINE`]).
#[test]
#[ignore = "needs FreeCAD: set CRAFT3D_FREECAD"]
fn freecad_reads_revolved_and_closed_splines() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("step-spline-sweeps");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut set = Set {
        dir,
        cases: Vec::new(),
        refused: Vec::new(),
        unmeasured: Vec::new(),
        per_body: false,
        meshes: None,
    };
    // The section of a hollow vase (the showcase's, a tenth of its size):
    // sketch x is the height, y the distance from the axis.
    let vase = |s: &mut Session, k: f64, turn: &str| {
        run(
            s,
            "sketch.create",
            json!({"name": "Section", "plane": "zx"}),
        );
        let p = |x: f64, y: f64| json!([x * k, y * k]);
        run(
            s,
            "sketch.line",
            json!({"sketch": "Section", "from": p(0.0, 0.0), "to": p(0.0, 3.0)}),
        );
        run(
            s,
            "sketch.spline",
            json!({"sketch": "Section", "control": ["e2", p(2.0, 5.8), p(7.5, 7.2),
                   p(12.0, 1.8), p(15.0, 2.0), p(17.8, 4.8), p(20.0, 3.6)]}),
        );
        run(
            s,
            "sketch.line",
            json!({"sketch": "Section", "from": "e9", "to": p(20.0, 3.2)}),
        );
        run(
            s,
            "sketch.spline",
            json!({"sketch": "Section", "control": ["e11", p(17.8, 4.4), p(15.0, 1.6),
                   p(12.0, 1.4), p(7.5, 6.8), p(2.0, 5.4), p(0.6, 2.6)]}),
        );
        run(
            s,
            "sketch.line",
            json!({"sketch": "Section", "from": "e18", "to": p(0.6, 0.0)}),
        );
        run(
            s,
            "sketch.line",
            json!({"sketch": "Section", "from": "e20", "to": "e1"}),
        );
        run(
            s,
            "feature.revolve",
            json!({"sketch": "Section", "axis": "x", "angle": turn, "name": "Vase"}),
        );
    };
    let egg = |s: &mut Session, k: f64, degree: u32| {
        run(s, "sketch.create", json!({"name": "Outline"}));
        let p = |x: f64, y: f64| json!([x * k, y * k]);
        run(
            s,
            "sketch.spline",
            json!({"sketch": "Outline", "closed": true, "degree": degree,
                   "control": [p(4.0, 2.0), p(8.0, 1.0), p(10.0, 5.0), p(7.0, 9.0),
                               p(3.0, 8.0), p(2.0, 4.0)]}),
        );
    };
    for (k, tag) in [(1e-3, "milli"), (1.0, "one"), (1e6, "mega")] {
        set.add(&format!("{tag}-spline-vase"), "mm", k, SPLINE, |s, k| {
            vase(s, k, "360 deg");
            Ok(())
        });
        set.add(
            &format!("{tag}-spline-vase-third"),
            "mm",
            k,
            SPLINE,
            |s, k| {
                vase(s, k, "120 deg");
                Ok(())
            },
        );
        set.add(&format!("{tag}-spline-egg"), "mm", k, SPLINE, |s, k| {
            egg(s, k, 3);
            run(
                s,
                "feature.extrude",
                json!({"sketch": "Outline", "distance": 2.5 * k, "name": "Egg"}),
            );
            Ok(())
        });
        set.add(
            &format!("{tag}-spline-egg-ring"),
            "mm",
            k,
            SPLINE,
            |s, k| {
                egg(s, k, 4);
                run(
                    s,
                    "feature.revolve",
                    json!({"sketch": "Outline", "axis": "y", "name": "Ring"}),
                );
                Ok(())
            },
        );
        set.add(&format!("{tag}-spline-polyline"), "mm", k, EXACT, |s, k| {
            run(s, "sketch.create", json!({"name": "Outline"}));
            let p = |x: f64, y: f64| json!([x * k, y * k]);
            run(
                s,
                "sketch.spline",
                json!({"sketch": "Outline", "closed": true, "degree": 1,
                       "control": [p(1.0, 0.0), p(7.0, 1.0), p(6.0, 5.0), p(3.0, 3.0), p(2.0, 6.0)]}),
            );
            run(
                s,
                "feature.revolve",
                json!({"sketch": "Outline", "axis": "y", "angle": "200 deg", "name": "Spun"}),
            );
            Ok(())
        });
    }
    assert!(set.refused.is_empty(), "{:?}", set.refused);
    assert!(set.unmeasured.is_empty(), "{:?}", set.unmeasured);
    let rows = harness::check(
        "step-spline-sweeps-files",
        "STEP: revolved splines, closed splines and polylines, read by FreeCAD",
        &set.cases,
    );
    assert_eq!(rows.len(), 15, "five solids at three scales");
    for row in rows.iter().filter(|r| !r.file.contains("polyline")) {
        let ratio = row
            .quadrature_volume_ratio
            .unwrap_or_else(|| panic!("{}: no volume by quadrature", row.file));
        println!(
            "{}: the volume of FreeCAD's geometry over ours - 1 = {:+.2e}",
            row.file,
            ratio - 1.0
        );
        assert!(
            (ratio - 1.0).abs() <= SPLINE_QUADRATURE,
            "{}: FreeCAD's geometry has {ratio} times our volume",
            row.file
        );
    }
    let bad: Vec<String> = rows
        .iter()
        .filter(|r| !r.problems.is_empty())
        .map(|r| format!("{}: {}", r.file, r.problems.join("; ")))
        .collect();
    assert!(
        bad.is_empty(),
        "{} of {} files with problems:\n{}",
        bad.len(),
        rows.len(),
        bad.join("\n")
    );
}
