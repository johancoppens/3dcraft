//! The format promise, regenerated: the file of every format version
//! (`crates/format/tests/fixtures/versions`, written by that version's own
//! build) opens with today's code and regenerates to the bodies that build
//! reported, saving it writes the current version, and the saved file
//! reopens to the same bodies, bit for bit. How the fixtures were written
//! and what keeps them frozen: `crates/format/tests/versions.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{session_in, workdir};
use craft3d_format::FORMAT_VERSION;
use serde_json::{Value, json};
use std::path::PathBuf;

/// Volumes, areas and masses are integrals over the exact B-rep, by
/// quadrature: equal to what the writing build reported to this relative
/// difference. Measured on 2026-10-21: every one agrees to 1e-9 but the
/// area of version 13's fine modelled thread (F6), 1.03e-9 apart (356.32502417
/// against 356.32502380 mm²); counts of faces, edges and vertices agree
/// exactly for every body.
const RELATIVE: f64 = 1e-8;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../format/tests/fixtures/versions")
}

fn expected(version: u32) -> Value {
    let path = fixtures().join(format!("v{version}.expected.json"));
    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
}

fn close(actual: &Value, expected: &Value) -> bool {
    match (actual.as_f64(), expected.as_f64()) {
        (Some(a), Some(e)) => (a - e).abs() <= RELATIVE * e.abs().max(1.0),
        _ => false,
    }
}

/// Every way `inspect` differs from what the writing build reported.
/// `entries` with each `face` written in the canonical form of its name.
fn canonical_faces(entries: &Value) -> Value {
    let mut entries = entries.clone();
    if let Some(list) = entries.as_array_mut() {
        for entry in list {
            let canonical = entry["face"]
                .as_str()
                .and_then(|text| text.parse::<craft3d_doc::PersistentName>().ok())
                .map(|name| name.to_string());
            if let Some(text) = canonical {
                entry["face"] = Value::String(text);
            }
        }
    }
    entries
}

fn differences(version: u32, inspect: &Value, want: &Value) -> Vec<String> {
    let mut found = Vec::new();
    let tag = format!("v{version}");
    if inspect["rollback"] != want["rollback"] {
        found.push(format!(
            "{tag}: rollback {} (was {})",
            inspect["rollback"], want["rollback"]
        ));
    }
    let features = inspect["features"].as_array().unwrap();
    let wanted = want["features"].as_array().unwrap();
    if features.len() != wanted.len() {
        found.push(format!(
            "{tag}: {} features (was {})",
            features.len(),
            wanted.len()
        ));
    }
    for (feature, wanted) in features.iter().zip(wanted) {
        // Who made and changed it (format 63): pinned where the fixture
        // says, and never made up where it does not.
        for key in ["madeBy", "lastChangedBy"] {
            if feature.get(key) != wanted.get(key) {
                found.push(format!(
                    "{tag} {}: {key} {:?} (was {:?})",
                    wanted["id"],
                    feature.get(key),
                    wanted.get(key)
                ));
            }
        }
        for key in ["id", "name", "status", "dof"] {
            if wanted.get(key).is_some() && feature[key] != wanted[key] {
                found.push(format!(
                    "{tag} {}: {key} {} (was {}){}",
                    wanted["id"],
                    feature[key],
                    wanted[key],
                    feature
                        .get("error")
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                ));
            }
        }
    }
    // The features hidden in the view (format 30), as listed.
    if let Some(hidden) = want.get("hidden") {
        let got: Vec<Value> = features
            .iter()
            .filter(|f| f["hidden"] == true)
            .map(|f| f["id"].clone())
            .collect();
        if Value::Array(got.clone()) != *hidden {
            found.push(format!("{tag}: hidden {got:?} (were {hidden})"));
        }
    }
    // The colours given to faces (format 26), as listed, none of them lost.
    if let Some(faces) = want.get("faceAppearances") {
        // A name read in the long form and the short one is the same name
        // (`PersistentName` is canonical once read, 2026-10-22): compare
        // the names as names, the face and its colour exactly as pinned.
        if inspect.get("faceAppearances").map(canonical_faces) != Some(canonical_faces(faces)) {
            found.push(format!(
                "{tag}: faceAppearances {} (were {faces})",
                inspect["faceAppearances"]
            ));
        }
        if let Some(lost) = inspect.get("lostFaces") {
            found.push(format!("{tag}: lostFaces {lost}"));
        }
    }
    // The assertions (format 28), each as it stood: its name, measure,
    // status and what it must be; what was measured within the measure's
    // own rounding (it is said in words).
    if let Some(assertions) = want.get("assertions") {
        let keys = |list: &Value| -> Vec<Value> {
            list.as_array()
                .map(|l| {
                    l.iter()
                        .map(|a| json!([a["name"], a["measure"], a["status"], a["expected"]]))
                        .collect()
                })
                .unwrap_or_default()
        };
        let got = inspect.get("assertions").cloned().unwrap_or(Value::Null);
        if keys(&got) != keys(assertions) {
            found.push(format!("{tag}: assertions {got} (were {assertions})"));
        }
    }
    // The outputs (format 43), each by name with its status, and its value
    // where it had one.
    if let Some(outputs) = want.get("outputs") {
        let got = inspect.get("outputs").cloned().unwrap_or(Value::Null);
        let keys = |list: &Value| -> Vec<Value> {
            list.as_array()
                .map(|l| {
                    l.iter()
                        .map(|o| json!([o["name"], o["status"], o["unit"]]))
                        .collect()
                })
                .unwrap_or_default()
        };
        let values_close = got
            .as_array()
            .zip(outputs.as_array())
            .is_some_and(|(g, w)| {
                g.iter()
                    .zip(w)
                    .all(|(g, w)| w["value"].is_null() || close(&g["value"], &w["value"]))
            });
        if keys(&got) != keys(outputs) || !values_close {
            found.push(format!("{tag}: outputs {got} (were {outputs})"));
        }
    }
    let bodies = inspect["bodies"].as_array().unwrap();
    let wanted = want["bodies"].as_array().unwrap();
    let ids =
        |list: &[Value]| -> Vec<Value> { list.iter().map(|b| b["feature"].clone()).collect() };
    if ids(bodies) != ids(wanted) {
        found.push(format!(
            "{tag}: bodies of {:?} (were {:?})",
            ids(bodies),
            ids(wanted)
        ));
        return found;
    }
    for (body, wanted) in bodies.iter().zip(wanted) {
        let Some(wanted) = wanted.as_object() else {
            continue;
        };
        for (key, value) in wanted {
            let same = match key.as_str() {
                "volume" | "area" | "mass" => close(&body[key], value),
                _ => &body[key] == value,
            };
            if !same {
                found.push(format!(
                    "{tag} body {}: {key} {} (was {value})",
                    wanted["feature"], body[key]
                ));
            }
        }
    }
    found
}

/// The notes as `note.list` lists them, without when each was written
/// and closed (the clock of the build that wrote the file).
fn noted(notes: &Value) -> Value {
    let notes = notes.as_array().cloned().unwrap_or_default();
    Value::Array(
        notes
            .iter()
            .map(|n| {
                json!({
                    "id": n["id"], "text": n["text"], "state": n["state"],
                    "author": n["author"], "agent": n["agent"],
                    "on": {"kind": n["on"]["kind"], "name": n["on"]["name"], "feature": n["on"]["feature"]},
                    "attachment": n["attachment"], "now": n.get("now").map(|now| now["name"].clone()),
                    "point": n["point"], "reply": n["done"]["reply"], "closedBy": n["done"]["agent"],
                })
            })
            .collect(),
    )
}

/// The bodies as `document.inspect` lists them, without what says where
/// the document is.
fn bodies(inspect: &Value) -> Value {
    json!({"features": inspect["features"], "bodies": inspect["bodies"], "rollback": inspect["rollback"]})
}

#[test]
fn the_file_of_every_format_version_regenerates_to_what_its_build_made() {
    let dir = workdir("format-versions", "all");
    let mut found = Vec::new();
    for version in 1..=FORMAT_VERSION {
        let want = expected(version);
        assert_eq!(want["format_version"], version, "v{version}.expected.json");
        // An assembly's fixture is a .3dcraftasm file (version 52).
        let name = if fixtures().join(format!("v{version}.3dcraftasm")).is_file() {
            format!("v{version}.3dcraftasm")
        } else {
            format!("v{version}.3dcraft")
        };
        std::fs::copy(fixtures().join(&name), dir.join(&name)).unwrap();
        let mut s = session_in(&dir);
        run(&mut s, "file.open", json!({"path": name}));
        let opened = run(&mut s, "document.inspect", json!({}));
        found.extend(differences(version, &opened, &want));
        // The named sets (format 27), each with what it holds now: the
        // query computed again against today's regeneration.
        if let Some(sets) = want.get("sets") {
            let listed = run(&mut s, "set.list", json!({}));
            if &listed["sets"] != sets {
                found.push(format!("v{version}: sets {} (were {sets})", listed["sets"]));
            }
        }
        // The notes pinned to the model (format 75), each with what it is
        // on and whether that is still there, found again against today's
        // regeneration; who wrote and closed it, and the reply, as kept.
        if let Some(notes) = want.get("notes") {
            let listed = run(&mut s, "note.list", json!({}));
            let got = noted(&listed["notes"]);
            if got != *notes {
                found.push(format!("v{version}: notes {got} (were {notes})"));
            }
        }
        // The public parameters and the document info (format 32): each
        // parameter's name, kind, expression, unit and validity, as
        // param.list gives them, and the info as kept.
        if let Some(parameters) = want.get("parameters") {
            let listed = run(&mut s, "param.list", json!({}));
            let got: Vec<Value> = listed["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    json!({"name": p["name"], "kind": p["kind"], "expr": p["expr"],
                           "unit": p["unit"], "valid": p["valid"]})
                })
                .collect();
            if Value::Array(got.clone()) != *parameters {
                found.push(format!(
                    "v{version}: parameters {got:?} (were {parameters})"
                ));
            }
            if want.get("info") != Some(&listed["info"]) {
                found.push(format!(
                    "v{version}: info {} (was {:?})",
                    listed["info"],
                    want.get("info")
                ));
            }
        }

        // Saved again: the current version, and the same bodies, exactly.
        let extension = name.rsplit('.').next().unwrap_or("3dcraft");
        let again = format!("v{version}-saved.{extension}");
        run(&mut s, "file.save", json!({"path": again}));
        let text = String::from_utf8_lossy(&std::fs::read(dir.join(&again)).unwrap()).to_string();
        assert!(
            text.contains(&format!("\"format_version\": {FORMAT_VERSION}")),
            "v{version} saved"
        );
        let mut t = session_in(&dir);
        run(&mut t, "file.open", json!({"path": again}));
        let reopened = run(&mut t, "document.inspect", json!({}));
        assert_eq!(
            bodies(&reopened),
            bodies(&opened),
            "v{version}: saved as version {FORMAT_VERSION} and reopened"
        );
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// What pins v61's sphere cap besides its fixture (whose `bbox` is a
/// tessellation's, inside the true box by at most `bboxTolerance`, so it
/// moves when the cap's edge is written differently: 2026-11-03, the
/// projected trim's proof fits a circle quarter by quarter). The cap is a
/// sphere of radius 20 about (150, 0, 0) cut by the circle of radius 8
/// about x = 150, z = 0 projected along −y: its true box is x in
/// [142, 158], y in [√336, 20], z in [−8, 8], its area `2π R h` with
/// `h = 20 − √336`, and its edge lies at y = √336 within the tolerance.
#[test]
fn the_v61_cap_is_the_sphere_cap_of_its_closed_form() {
    let dir = workdir("format-versions", "v61-cap");
    std::fs::copy(fixtures().join("v61.3dcraft"), dir.join("v61.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "v61.3dcraft"}));
    let inspect = run(&mut s, "document.inspect", json!({}));
    let cap = inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["feature"] == "F7")
        .unwrap_or_else(|| panic!("no F7 in {inspect}"));
    let num = |v: &Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));
    let rim = 336.0_f64.sqrt();
    let tol = 1e-6 * 20.0;
    let chord = num(&cap["bboxTolerance"]);
    let lo = [142.0, rim, -8.0];
    let hi = [158.0, 20.0, 8.0];
    for axis in 0..3 {
        let (got_lo, got_hi) = (num(&cap["bbox"][0][axis]), num(&cap["bbox"][1][axis]));
        // Inside the true box (the edge within the tolerance of it), and
        // short of it by at most the tessellation's tolerance.
        assert!(
            got_lo >= lo[axis] - tol && got_lo <= lo[axis] + chord,
            "axis {axis}: {cap}"
        );
        assert!(
            got_hi <= hi[axis] + tol && got_hi >= hi[axis] - chord,
            "axis {axis}: {cap}"
        );
    }
    let area = 2.0 * std::f64::consts::PI * 20.0 * (20.0 - rim);
    assert!(
        (num(&cap["area"]) - area).abs() <= 1e-6 * area,
        "area {} vs {area}",
        cap["area"]
    );
}

/// What pins v64's jogged plate besides its fixture (whose `bbox` is a
/// tessellation's, set by the body's vertices: its largest y went from 40
/// to 40 + 7.1e-15, one unit in the last place, on 2026-11-14, when the
/// corners where the jog's first bend runs on into the plate's sides were
/// taken from the rims' rows as every smooth corner is,
/// docs/design/plates.md › *Findings and history*; its volume and area kept
/// every bit). The part: a plate 80 × 40, 1 thick, a hole of radius 4 on
/// the far side, jogged up 8 at x = 35 by two bends of 60 degrees and
/// radius 2, and a flange 12 long bent 90 degrees at radius 1.5 on a side
/// 40 long. Its volume is `T` times its flats (the near side 35 × 40, the
/// web `w × 40` with `w = (h − (2R + T)(1 − cos α)) / sin α`, the far side
/// 45 × 40 less the hole, the flange 12 × 40) plus each bend's
/// `θ 40 T (R + T/2)`; across the plate it reaches from y = 0 to y = 40,
/// to the rounding of its coordinates.
#[test]
fn the_v64_jogged_plate_is_its_closed_form() {
    use craft3d_math::Portable;
    use std::f64::consts::PI;
    let dir = workdir("format-versions", "v64-jog");
    std::fs::copy(fixtures().join("v64.3dcraft"), dir.join("v64.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "v64.3dcraft"}));
    let inspect = run(&mut s, "document.inspect", json!({}));
    let plate = inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["feature"] == "F2")
        .unwrap_or_else(|| panic!("no F2 in {inspect}"));
    let num = |v: &Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));
    let (t, h, alpha, jog_r, flange_r) = (1.0, 8.0, PI / 3.0, 2.0, 1.5);
    let w = (h - (2.0 * jog_r + t) * (1.0 - alpha.pcos())) / alpha.psin();
    let flats = 35.0 * 40.0 + w * 40.0 + 45.0 * 40.0 - PI * 16.0 + 12.0 * 40.0;
    let volume = t * flats
        + 2.0 * alpha * 40.0 * t * (jog_r + t / 2.0)
        + PI / 2.0 * 40.0 * t * (flange_r + t / 2.0);
    assert!(
        (num(&plate["volume"]) - volume).abs() <= 1e-12 * volume,
        "volume {} vs {volume}",
        plate["volume"]
    );
    let rounding = 1e-12 * 100.0;
    let chord = num(&plate["bboxTolerance"]);
    let (lo, hi) = (num(&plate["bbox"][0][1]), num(&plate["bbox"][1][1]));
    assert!(lo >= -rounding && lo <= chord, "{plate}");
    assert!(hi <= 40.0 + rounding && hi >= 40.0 - chord, "{plate}");
}

/// What pins v67's ring besides its fixture, whose centroid is the
/// rounding of zero and was re-recorded on 2026-11-14 (the measure takes
/// the feet's rates for samples off their face by more than rounding, and
/// the ring's eight seam edges lie up to 1.1e-13 mm off their faces: its
/// centroid moved from [6.798989834637259e-15, 7.487275225627571e-15,
/// −3.599651550782486e-17] to [6.798989834636841e-15, 7.487275225627571e-15,
/// −5.399477326173729e-17]). The ring (F5) is a circle of radius 2 swept
/// round the z axis at radius 10 with the rail at 12: a torus, centred on
/// the origin by its symmetry, of volume `2π² R r²`. Its centroid is held
/// to zero within what the measure's own bound allows (the volume's bound
/// over the volume, times the body's diagonal), its volume to the torus's.
#[test]
fn the_v67_ring_is_a_torus_centred_by_its_symmetry() {
    let dir = workdir("format-versions", "v67-ring");
    std::fs::copy(fixtures().join("v67.3dcraft"), dir.join("v67.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "v67.3dcraft"}));
    let measured = run(
        &mut s,
        "document.measure",
        json!({"feature": "F5", "bound": true, "inertia": false}),
    );
    let num = |v: &Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));
    let (volume, bound) = (num(&measured["volume"]), num(&measured["volumeBound"]));
    let torus = 2.0 * std::f64::consts::PI * std::f64::consts::PI * 10.0 * 4.0;
    assert!(
        (volume - torus).abs() <= 1e-8 * torus,
        "volume {volume} vs {torus}"
    );
    let diagonal = (24.0_f64 * 24.0 + 24.0 * 24.0 + 4.0 * 4.0).sqrt();
    let allowed = bound / volume * diagonal;
    assert!(
        allowed > 0.0 && allowed < 1e-9,
        "the measure's bound {bound}"
    );
    for axis in 0..3 {
        let c = num(&measured["centroid"][axis]);
        assert!(
            c.abs() <= allowed,
            "centroid {}: {c} against {allowed}",
            measured["centroid"]
        );
    }
}

/// What pins v72's grids besides their fixture: the grid read from the
/// table lies on the plane `z = 7 + 0.2 x + 0.2 y` (heights `1 + r + c`
/// mm, 10 mm apart, times `lift` = 2, from 5 mm up), which a grid's sheet
/// gives exactly, so the 20 × 20 mm block cut by it is a wedge of
/// `∫∫ 7 + 0.2 x + 0.2 y = 4400` mm³, and 6800 mm³ at `lift` = 4; the grid
/// given point by point follows `lift` too, its middle point `lift * 1 mm`
/// high.
#[test]
fn the_v72_wedge_under_a_grid_is_its_closed_form() {
    let dir = workdir("format-versions", "v72-grid");
    std::fs::copy(fixtures().join("v72.3dcraft"), dir.join("v72.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "v72.3dcraft"}));
    let body = |s: &mut craft3d_engine::Session, name: &str| -> Value {
        let inspect = run(s, "document.inspect", json!({}));
        inspect["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == name)
            .unwrap_or_else(|| panic!("no {name} in {inspect}"))
            .clone()
    };
    let volume = body(&mut s, "Wedge")["volume"].as_f64().unwrap();
    assert!((volume - 4400.0).abs() <= 1e-9 * 4400.0, "{volume}");
    let low = body(&mut s, "Dome")["area"].as_f64().unwrap();
    run(&mut s, "variable.set", json!({"name": "lift", "expr": "4"}));
    let high = body(&mut s, "Dome")["area"].as_f64().unwrap();
    assert!(high > low + 1.0, "{low} {high}");
    let volume = body(&mut s, "Wedge")["volume"].as_f64().unwrap();
    assert!((volume - 6800.0).abs() <= 1e-9 * 6800.0, "{volume}");
}

/// What pins v74's tips besides their fixture: the rod of radius `r`
/// lofted 20 mm long through equal circles, its end tipped as long as `r`
/// at the point given in the model where the tip is to be,
/// `(0, 0, 20 mm + r)`, and its start at a point in the model on the end,
/// `(0, 0, 0)`: a half ball on each end, ⅔ π r³ each, that follows `r`;
/// the places stay three expressions.
#[test]
fn the_v74_rod_tipped_at_points_in_the_model_is_its_closed_form() {
    let dir = workdir("format-versions", "v74-tip");
    std::fs::copy(fixtures().join("v74.3dcraft"), dir.join("v74.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "v74.3dcraft"}));
    for r in [5.0_f64, 4.0] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": format!("{r} mm")}),
        );
        let inspect = run(&mut s, "document.inspect", json!({}));
        let got = inspect["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == "Tail")
            .unwrap_or_else(|| panic!("no Tail in {inspect}"))["volume"]
            .as_f64()
            .unwrap();
        let pi = std::f64::consts::PI;
        let want = pi * r * r * 20.0 + 4.0 / 3.0 * pi * r * r * r;
        assert!(
            (got - want).abs() <= 1e-11 * want,
            "r {r}: {got} against {want}"
        );
        let nose = inspect["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "Nose")
            .unwrap();
        assert_eq!(
            nose["params"]["at"],
            json!(["0 mm", "0 mm", "20 mm + r"]),
            "{nose}"
        );
        assert_eq!(nose["tip"]["placed"], "givenPoint", "{nose}");
    }
}

/// What pins v73's tips besides their fixture: the rod of radius `r`
/// lofted 20 mm long through equal circles, its end tipped as long as `r`
/// at the default fullness (a half ball, ⅔ π r³) and its start at the
/// length chosen (the radius of the circle of its area, `r`) with
/// parabolas (11/15 π r³), follows `r`.
#[test]
fn the_v73_rod_tipped_at_both_ends_is_its_closed_form() {
    let dir = workdir("format-versions", "v73-tip");
    std::fs::copy(fixtures().join("v73.3dcraft"), dir.join("v73.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "v73.3dcraft"}));
    let volume = |s: &mut craft3d_engine::Session| -> f64 {
        let inspect = run(s, "document.inspect", json!({}));
        inspect["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == "Tail")
            .unwrap_or_else(|| panic!("no Tail in {inspect}"))["volume"]
            .as_f64()
            .unwrap()
    };
    for r in [5.0_f64, 4.0] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": format!("{r} mm")}),
        );
        let pi = std::f64::consts::PI;
        let want = pi * r * r * 20.0 + (2.0 / 3.0 + 11.0 / 15.0) * pi * r * r * r;
        let got = volume(&mut s);
        assert!(
            (got - want).abs() <= 1e-11 * want,
            "r {r}: {got} against {want}"
        );
    }
}

/// What pins v75's notes besides their fixture: the plate they are pinned
/// to is its closed form (40 × 20 × t less a hole of radius 3, t = 7 mm),
/// and the notes follow an edit after opening: the top's note stays on
/// the top at t = 9 mm, the detached one stays detached at its point, and
/// one more note takes id 6 (5 was given out and deleted).
#[test]
fn the_v75_notes_follow_the_plate_they_are_pinned_to() {
    let dir = workdir("format-versions", "v75-notes");
    std::fs::copy(fixtures().join("v75.3dcraft"), dir.join("v75.3dcraft")).unwrap();
    let mut s = session_in(&dir);
    run(&mut s, "file.open", json!({"path": "v75.3dcraft"}));
    let pi = std::f64::consts::PI;
    for t in [7.0_f64, 9.0] {
        run(
            &mut s,
            "variable.set",
            json!({"name": "t", "expr": format!("{t} mm")}),
        );
        let inspect = run(&mut s, "document.inspect", json!({}));
        let got = inspect["bodies"][0]["volume"].as_f64().unwrap();
        let want = 40.0 * 20.0 * t - pi * 9.0 * t;
        assert!(
            (got - want).abs() <= 1e-9 * want,
            "t {t}: {got} against {want}"
        );
        let notes = run(&mut s, "note.list", json!({}));
        let states: Vec<(Value, Value)> = notes["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| (n["id"].clone(), n["attachment"].clone()))
            .collect();
        assert_eq!(
            states,
            [
                (json!(1), json!("attached")),
                (json!(2), json!("attached")),
                (json!(3), json!("detached")),
                (json!(4), json!("attached")),
            ],
            "t {t}"
        );
    }
    let added = run(
        &mut s,
        "note.add",
        json!({"text": "one more", "on": "Drilled"}),
    );
    assert_eq!(added["id"], 6);
}
