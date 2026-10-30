//! What lies on one side of a plane (`measure.below`, `measure.section`,
//! `measure.levelFor`, and the assertion measures `volumeBelow`,
//! `centroidBelow`, `sectionArea`, `sectionCentroid`, `sectionInertia`,
//! `levelFor`) against closed forms of a block: the volume and centroid
//! below a level, the rectangle's moments, the level for a volume, a
//! turned plane; and, as a worked example of what a Jig builds on them,
//! the metacentric height of a floating box (BM = I/V, GM = KB + BM − KG)
//! changing sign at its known limit.

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
        Err(e) => {
            let message = e.message().to_string();
            assert!(
                message.starts_with(id),
                "the error names the command: {message}"
            );
            message
        }
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

/// A block `l × b × d` mm centred on the z axis, its bottom at z = 0.
fn block(l: f64, b: f64, d: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [l, b, d], "position": [-l / 2.0, -b / 2.0, 0], "name": "Tank"}),
    );
    s
}

#[test]
fn a_block_below_a_level_matches_its_closed_forms() {
    let (l, b, d, t) = (4000.0, 1300.0, 1000.0, 300.0);
    let mut s = block(l, b, d);
    let below = run(
        &mut s,
        "measure.below",
        json!({"of": "Tank", "offset": "300 mm"}),
    );
    close(&below["volume"], l * b * t, "volume below");
    close(&below["centroid"][2], t / 2.0, "centroid height");
    close(&below["centroid"][0], 0.0, "centroid x");
    close(
        &below["surface"],
        l * b + 2.0 * (l + b) * t,
        "surface below",
    );
    let section = &below["section"];
    close(&section["area"], l * b, "section area");
    close(&section["ix"], l * b.powi(3) / 12.0, "ix = l b³/12");
    close(&section["iy"], b * l.powi(3) / 12.0, "iy = b l³/12");
    // Zero to the rounding of sums of the size of iy (1e14 mm^4).
    let ixy = section["ixy"].as_f64().unwrap();
    assert!(ixy.abs() <= 1e-12 * b * l.powi(3) / 12.0, "ixy {ixy}");
    close(&section["major"], b * l.powi(3) / 12.0, "major");
    close(&section["minor"], l * b.powi(3) / 12.0, "minor");
    assert_eq!(below["lengthUnit"], "mm");
    // measure.section gives the same section; a plane given by its normal
    // the other way round keeps the other side.
    let alone = run(&mut s, "measure.section", json!({"offset": 300}));
    assert_eq!(alone["area"], section["area"], "{alone}");
    let above = run(
        &mut s,
        "measure.below",
        json!({"plane": {"point": [0, 0, 0], "normal": [0, 0, -1]}, "offset": "-300 mm"}),
    );
    close(&above["volume"], l * b * (d - t), "volume above");
    // A flat face by its name: the top (z = d, facing up), moved down.
    let from_top = run(
        &mut s,
        "measure.below",
        json!({"plane": "F1:face(1)", "offset": -(d - t)}),
    );
    close(
        &from_top["volume"],
        l * b * t,
        "below the top face moved down",
    );
    close(
        &from_top["offset"],
        -(d - t),
        "the offset from the plane given",
    );
    // The level for a volume: the fill level of the block as a tank.
    let level = run(&mut s, "measure.levelFor", json!({"volume": l * b * t}));
    close(&level["offset"], t, "level for a volume");
    assert!(level["steps"].as_u64().unwrap() <= 8, "{level}");
    // A volume with a unit, as an expression of the variables.
    run(
        &mut s,
        "variable.set",
        json!({"name": "fill", "expr": "1.56 * (1 m)^3"}),
    );
    let level = run(&mut s, "measure.levelFor", json!({"volume": "fill"}));
    close(&level["offset"], t, "level for a volume in m³");
}

/// Worked example, as a Jig for a floating body would do it with the
/// general measures: a box of relative density s floats with s of its
/// depth under (levelFor of its volume times s), KB is the centroid's
/// height below that level, BM = ix / V, and GM = KB + BM − KG changes
/// sign at B/D = √(6 s (1 − s)) (Biran & López-Pulido, *Ship
/// Hydrostatics and Stability*).
#[test]
fn a_jig_gets_the_metacentric_height_from_the_general_measures() {
    let s_rel = 0.5_f64;
    let limit = (6.0 * s_rel * (1.0 - s_rel)).sqrt() * 1000.0;
    for (b, stable) in [(limit * 1.02, true), (limit * 0.98, false)] {
        let (l, d) = (4000.0, 1000.0);
        let mut s = block(l, b, d);
        let level = run(
            &mut s,
            "measure.levelFor",
            json!({"volume": l * b * d * s_rel}),
        );
        let t = level["offset"].as_f64().unwrap();
        close(&level["offset"], s_rel * d, "draft");
        let kb = level["centroid"][2].as_f64().unwrap();
        let bm = level["section"]["ix"].as_f64().unwrap() / level["volume"].as_f64().unwrap();
        close(&json!(bm), b * b / (12.0 * t), "BM = B²/(12 T)");
        let gm = kb + bm - d / 2.0;
        assert_eq!(gm > 0.0, stable, "B {b}: GM {gm}");
    }
}

/// An assertion on a section's moment fails on a block too narrow and
/// passes once it is widened; the level for a volume as an assertion.
#[test]
fn assertions_on_a_section_break_and_pass_again() {
    let mut s = block(4000.0, 1000.0, 1000.0);
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "stiff", "measure": "sectionInertia", "of": "Tank", "offset": 500, "about": "x", "min": "4 * (1 m)^4 / 12 * 1.2"}),
    );
    assert_eq!(added["status"], "failed", "{added}");
    assert_eq!(added["unit"], "mm^4", "{added}");
    let fill = run(
        &mut s,
        "assert.add",
        json!({"name": "fill", "measure": "levelFor", "volume": "2 * (1 m)^3", "eq": "500 mm", "tol": 1e-6}),
    );
    assert_eq!(fill["status"], "passed", "{fill}");
    let volume = run(
        &mut s,
        "assert.add",
        json!({"name": "half", "measure": "volumeBelow", "offset": "500 mm", "eq": "2 * (1 m)^3", "tol": 1}),
    );
    assert_eq!(volume["status"], "passed", "{volume}");
    let height = run(
        &mut s,
        "assert.add",
        json!({"name": "centroid", "measure": "centroidBelow", "offset": 500, "axis": "z", "eq": 250}),
    );
    assert_eq!(height["status"], "passed", "{height}");
    let area = run(
        &mut s,
        "assert.add",
        json!({"name": "area", "measure": "sectionArea", "offset": 500, "eq": "4 * (1 m)^2"}),
    );
    assert_eq!(area["status"], "passed", "{area}");
    let centre = run(
        &mut s,
        "assert.add",
        json!({"name": "centre", "measure": "sectionCentroid", "offset": 500, "axis": "z", "eq": 500}),
    );
    assert_eq!(centre["status"], "passed", "{centre}");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Tank", "size": [4000, 1100, 1000], "position": [-2000, -550, 0]}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    let status = |name: &str| {
        listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .map(|a| a["status"].clone())
            .unwrap()
    };
    assert_eq!(status("stiff"), "passed", "{listed}");
    // The tank widened: the same volume fills it less high.
    assert_eq!(status("fill"), "failed", "{listed}");
}

/// The parameters are expressions of the variables, and renaming a
/// variable renames it in the assertion.
#[test]
fn expressions_follow_the_variables() {
    let mut s = block(4000.0, 1000.0, 1000.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "level", "expr": "250 mm"}),
    );
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "v", "measure": "volumeBelow", "offset": "level", "eq": "1 * (1 m)^3"}),
    );
    assert_eq!(added["status"], "passed", "{added}");
    run(
        &mut s,
        "variable.rename",
        json!({"name": "level", "to": "h"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(
        listed["assertions"][0]["measure"]["offset"], "h",
        "{listed}"
    );
}

#[test]
fn what_cannot_be_measured_is_refused() {
    let mut s = block(4000.0, 1000.0, 1000.0);
    for id in ["measure.below", "measure.section"] {
        assert!(
            refused(
                &mut s,
                id,
                json!({"plane": {"point": [0, 0, 0], "normal": [0, 0, 0]}})
            )
            .contains("normal")
        );
        assert!(refused(&mut s, id, json!({"plane": "NoSuchFace"})).contains("plane"));
        assert!(refused(&mut s, id, json!({"xAxis": "z"})).contains("square"));
        assert!(refused(&mut s, id, json!({"offset": "nope"})).contains("offset"));
        assert!(refused(&mut s, id, json!({"offset": "5 deg"})).contains("length"));
        assert!(refused(&mut s, id, json!({"bogus": 1})).contains("bogus"));
        assert!(refused(&mut s, id, json!({"of": "Nothing"})).contains("Nothing"));
    }
    let id = "measure.levelFor";
    assert!(refused(&mut s, id, json!({})).contains("volume"));
    assert!(refused(&mut s, id, json!({"volume": 0})).contains("outside"));
    assert!(refused(&mut s, id, json!({"volume": 5e9})).contains("outside"));
    assert!(refused(&mut s, id, json!({"volume": -1})).contains("outside"));
    assert!(refused(&mut s, id, json!({"volume": "5 mm"})).contains("volume"));
    let mut empty = Session::new(Workspace::none());
    run(&mut empty, "file.new", json!({"units": "mm"}));
    assert!(refused(&mut empty, "measure.below", json!({})).contains("no bodies"));
    // Nothing below the plane: no centroid, and an assertion on it says so.
    let none = run(&mut s, "measure.below", json!({"offset": -10}));
    assert_eq!(none["volume"], 0.0);
    assert!(none["centroid"].is_null(), "{none}");
    let wrong = run(
        &mut s,
        "assert.add",
        json!({"name": "w", "measure": "centroidBelow", "offset": -10, "axis": "z", "min": 0}),
    );
    assert_eq!(wrong["status"], "error", "{wrong}");
    let bad = run(
        &mut s,
        "assert.add",
        json!({"name": "b", "measure": "sectionInertia", "offset": 10, "about": "q", "min": 0}),
    );
    assert_eq!(bad["status"], "error", "{bad}");
}

/// `measure.floating` (docs/design/sections-and-levels.md › *A level with
/// a tilt*) on a block 1000 × 200 × 200 mm of 20 kg in fresh water: a
/// draft of 100 mm, BM = B² / (12 T) = 33.3 mm. With the centre of
/// gravity at the centre of buoyancy it floats upright, GM = BM; at 90 mm
/// it is unstable upright and lolls to tan² φ = −2 GM / BM each way.
#[test]
fn a_block_floats_upright_or_lolls_at_its_wall_sided_angle() {
    let mut s = block(1000.0, 200.0, 200.0);
    let upright = run(
        &mut s,
        "measure.floating",
        json!({"mass": 20, "density": 1000, "centerOfGravity": [0, 0, 50], "heel": "10 deg", "trim": 2}),
    );
    assert_eq!(upright["floats"], 0, "{upright}");
    assert_eq!(upright["neutral"], false);
    let e = &upright["equilibria"][0];
    close(&e["offset"], 100.0, "draft");
    close(&e["heel"], 0.0, "heel");
    close(&e["trim"], 0.0, "trim");
    close(&e["volume"], 2.0e7, "volume");
    assert_eq!(e["stable"], true);
    let gms: Vec<f64> = e["principal"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["gm"].as_f64().unwrap())
        .collect();
    close(&json!(gms[0]), 40000.0 / 1200.0, "GM across = BM");
    close(&json!(gms[1]), 1.0e6 / 1200.0, "GM along");
    assert_eq!(upright["lengthUnit"], "mm");

    let lolled = run(
        &mut s,
        "measure.floating",
        json!({"volume": "2e7", "centerOfGravity": ["0 mm", "0 mm", "90 mm"]}),
    );
    let all = lolled["equilibria"].as_array().unwrap();
    assert_eq!(all.len(), 3, "{lolled}");
    assert_eq!(all[0]["stable"], false);
    assert_eq!(lolled["floats"], 1);
    let gm: f64 = 50.0 + 40000.0 / 1200.0 - 90.0;
    let phi = (-2.0 * gm / (40000.0 / 1200.0)).sqrt().atan().to_degrees();
    let mut heels: Vec<f64> = all[1..]
        .iter()
        .map(|e| e["heel"].as_f64().unwrap())
        .collect();
    heels.sort_by(f64::total_cmp);
    close(&json!(heels[0]), -phi, "loll to one side");
    close(&json!(heels[1]), phi, "and to the other");
}

#[test]
fn what_cannot_float_is_refused() {
    let mut s = block(1000.0, 200.0, 200.0);
    let cog = json!([0, 0, 50]);
    for (params, words) in [
        (json!({"volume": "2e7"}), "centerOfGravity"),
        (json!({"centerOfGravity": cog}), "`volume` is required"),
        (
            json!({"volume": "2e7", "mass": 20, "density": 1000, "centerOfGravity": cog}),
            "not both",
        ),
        (json!({"mass": 20, "centerOfGravity": cog}), "`density`"),
        (
            json!({"mass": 20, "density": -1, "centerOfGravity": cog}),
            "positive",
        ),
        (
            json!({"volume": "5e7", "centerOfGravity": cog}),
            "outside (0,",
        ),
        (
            json!({"volume": "2e7", "centerOfGravity": [0, 0]}),
            "[x, y, z]",
        ),
        (
            json!({"volume": "2e7", "centerOfGravity": cog, "heel": "3 mm"}),
            "angle",
        ),
        (
            json!({"volume": "2e7", "centerOfGravity": cog, "heel": true}),
            "angle",
        ),
        (
            json!({"volume": "2e7", "centerOfGravity": cog, "offset": 3}),
            "offset",
        ),
        (
            json!({"volume": "2e7", "centerOfGravity": cog, "of": "Nothing"}),
            "Nothing",
        ),
    ] {
        let message = refused(&mut s, "measure.floating", params.clone());
        assert!(message.contains(words), "{params}: {message}");
        assert!(!message.contains("internal error"), "{params}: {message}");
    }
    let mut empty = Session::new(Workspace::none());
    assert!(
        empty
            .execute(
                "measure.floating",
                &json!({"volume": 1, "centerOfGravity": [0, 0, 0]})
            )
            .is_err()
    );
}

/// The welded beam of the repro (plan/repro/repro-welded-beam-2): a box
/// 100 × 150 with walls of 3 and 4 mm cut across. measure.section gives
/// its torsion constant near Bredt's thin-walled 4 A_m² / ∮ ds / t (within
/// the thin-wall error) and its shear centre at the centroid; the
/// assertion and output measures give the same numbers; torsion: false
/// leaves them out.
#[test]
fn a_welded_box_twists_near_bredt() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Outer", "size": ["500 mm", "100 mm", "150 mm"]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Inner", "size": ["502 mm", "94 mm", "142 mm"], "position": ["-1 mm", "3 mm", "4 mm"]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "Box", "op": "subtract", "target": "Outer", "tool": "Inner"}),
    );
    let started = std::time::Instant::now();
    let section = run(
        &mut s,
        "measure.section",
        json!({"of": "Box", "plane": "yz", "offset": "250 mm"}),
    );
    eprintln!(
        "measure.section of the welded box: {:.1} ms, {}",
        started.elapsed().as_secs_f64() * 1e3,
        section
    );
    let (w, h): (f64, f64) = (97.0, 146.0);
    let bredt = 4.0 * (w * h).powi(2) / (2.0 * w / 4.0 + 2.0 * h / 3.0);
    let j = section["torsion"].as_f64().unwrap();
    assert!((j - bredt).abs() <= 0.03 * bredt, "J {j}, Bredt {bredt}");
    assert_eq!(section["torsionSolve"]["converged"], true, "{section}");
    // At the centroid by symmetry, within the solve's tolerance of the
    // section's size (1e-4 of √A ≈ 4 µm).
    let centre = &section["shearCentre"];
    assert!(
        (centre[1].as_f64().unwrap() - 50.0).abs() < 4e-3,
        "{centre}"
    );
    assert!(
        (centre[2].as_f64().unwrap() - 75.0).abs() < 4e-3,
        "{centre}"
    );
    assert!(section["warping"].as_f64().unwrap() > 0.0);
    let plain = run(
        &mut s,
        "measure.section",
        json!({"of": "Box", "plane": "yz", "offset": "250 mm", "torsion": false}),
    );
    assert!(plain.get("torsion").is_none(), "{plain}");
    let asserted = run(
        &mut s,
        "assert.add",
        json!({"name": "stiff", "measure": "sectionTorsion", "of": "Box", "plane": "yz", "offset": "250 mm", "min": bredt * 0.97}),
    );
    assert_eq!(asserted["status"], "passed", "{asserted}");
    let output = run(
        &mut s,
        "variable.measure",
        json!({"name": "sc", "measure": "sectionShearCentre", "of": "Box", "plane": "yz", "offset": "250 mm", "axis": "z"}),
    );
    let value = output["output"]["value"]
        .as_f64()
        .or_else(|| output["value"].as_f64())
        .unwrap_or_else(|| panic!("{output}"));
    assert!((value - 75.0).abs() < 1e-3, "{output}");
}

/// The torsion's parameters are checked, and a plane that cuts nothing
/// is an error, never a panic.
#[test]
fn what_cannot_be_twisted_is_refused() {
    let mut s = block(400.0, 100.0, 100.0);
    for bad in [
        json!({"offset": 50, "torsionTolerance": 0}),
        json!({"offset": 50, "torsionTolerance": "x"}),
        json!({"offset": 50, "maxElements": 0}),
        json!({"offset": 50, "maxElements": 1e9}),
        json!({"offset": 50, "maxElements": -3}),
        json!({"offset": 50, "torsion": "yes"}),
    ] {
        refused(&mut s, "measure.section", bad);
    }
    // A plane that cuts nothing: no torsion, and the measure an error.
    let nothing = run(&mut s, "measure.section", json!({"offset": 500}));
    assert!(nothing["torsion"].is_null(), "{nothing}");
    let missing = run(
        &mut s,
        "assert.add",
        json!({"name": "none", "measure": "sectionTorsion", "offset": 500, "min": 1}),
    );
    assert_eq!(missing["status"], "error", "{missing}");
    // A cap the solve reaches: what it reached, said to be unconverged.
    let capped = run(
        &mut s,
        "measure.section",
        json!({"offset": 50, "maxElements": 3000, "torsionTolerance": 1e-9}),
    );
    assert_eq!(capped["torsionSolve"]["converged"], false, "{capped}");
    // A cap below the first mesh: the section is still given, its torsion
    // null with why; asked for by name, the command's error.
    let low = run(
        &mut s,
        "measure.section",
        json!({"offset": 50, "maxElements": 10}),
    );
    assert!(low["torsion"].is_null(), "{low}");
    assert!(
        (low["area"].as_f64().unwrap() - 40000.0).abs() < 1e-6,
        "{low}"
    );
    let why = low["torsionSolve"]["error"].as_str().unwrap();
    assert!(why.contains("no solution within 10 elements"), "{why}");
    let message = refused(
        &mut s,
        "measure.section",
        json!({"offset": 50, "maxElements": 10, "torsion": true}),
    );
    assert!(
        message.contains("no solution within 10 elements"),
        "{message}"
    );
    let twisted = run(
        &mut s,
        "assert.add",
        json!({"name": "J", "measure": "sectionTorsion", "offset": 50, "min": 1}),
    );
    assert_eq!(twisted["status"], "passed", "{twisted}");
}
