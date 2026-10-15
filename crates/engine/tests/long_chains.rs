//! Long rows of features that each make their body from the one before:
//! a plate with holes cut one after another, a row of unions that takes
//! the growing body in as the tool every time. Until 2026-10-14 a name
//! grew a link per feature and the 33rd such feature failed ("more than
//! 32 features in a row each make their body from the one before"); a
//! run of copies that each take instance 0, or that each take another
//! instance, is now one link (docs/design/names.md, *Copies of
//! copies*), so the names stay short however long the row is.
//!
//! The timing at a thousand holes is ignored by default:
//! `cargo test --release -p craft3d-engine --test long_chains -- --ignored --nocapture`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;
use std::time::Instant;

fn session() -> Session {
    Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    })
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn built(session: &mut Session, id: &str, params: Value) -> Value {
    let result = run(session, id, params.clone());
    assert_eq!(result["status"], "ok", "{id} {params}: {result}");
    result
}

fn body(session: &mut Session, name: &str) -> Value {
    let document = run(session, "document.inspect", json!({}));
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name} in {}", document["bodies"]))
        .clone()
}

fn names_of(body: &Value) -> Vec<String> {
    body["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

fn assert_close(found: f64, expected: f64, what: &str) {
    assert!(
        (found - expected).abs() <= 1e-9 * expected.abs(),
        "{what}: {found} vs {expected}"
    );
}

/// The plate: 10 mm per hole along x (at least 50), 40 mm deep, 5 mm
/// thick, its low corner at the origin. The block is F1; its top is
/// `face(1)`.
const THICK: f64 = 5.0;
const RADIUS: f64 = 2.0;

fn plate_size(holes: usize) -> (f64, f64) {
    let columns = holes.div_ceil(2).max(5);
    (10.0 * columns as f64, 40.0)
}

/// A plate with `holes` holes, each cut by its own extrusion with `cut`
/// (a sketch, an extrusion and a subtraction per hole), each from the body
/// the hole before it left. Returns the session and the name of the last
/// body.
fn plate_with_holes(holes: usize) -> (Session, String) {
    plate_with_holes_in(session(), holes)
}

fn plate_with_holes_in(mut s: Session, holes: usize) -> (Session, String) {
    run(&mut s, "file.new", json!({"units": "mm"}));
    let (length, width) = plate_size(holes);
    built(
        &mut s,
        "feature.block",
        json!({"size": [length, width, THICK], "name": "Plate"}),
    );
    let mut last = "Plate".to_string();
    for hole in 0..holes {
        let sketch = format!("S{hole}");
        run(&mut s, "sketch.create", json!({"name": sketch}));
        let (x, y) = (
            5.0 + 10.0 * (hole / 2) as f64,
            10.0 + 20.0 * (hole % 2) as f64,
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": sketch, "center": [x, y], "radius": RADIUS}),
        );
        let name = format!("H{hole}");
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": sketch, "distance": 3.0 * THICK, "symmetric": true,
                   "cut": last, "name": name}),
        );
        last = name;
    }
    (s, last)
}

fn plate_volume(holes: usize) -> f64 {
    let (length, width) = plate_size(holes);
    length * width * THICK - holes as f64 * PI * RADIUS * RADIUS * THICK
}

/// The id of a feature, by its name.
fn id_of(session: &mut Session, name: &str) -> String {
    let document = run(session, "document.inspect", json!({}));
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn seventy_holes_cut_one_after_another() {
    // The reproducer of the developer's report: 70 holes, one cut each.
    let holes = 70;
    let (mut s, last) = plate_with_holes(holes);
    let made = body(&mut s, &last);
    assert_close(
        made["volume"].as_f64().unwrap(),
        plate_volume(holes),
        "volume",
    );
    assert_eq!(made["faces"], 6 + holes);
    // The faces of the plate are copies of the block's faces, one link
    // each, whatever the number of cuts in between; the wall of the first
    // hole is the tool of the first cut, kept as instance 0 ever since.
    let head = id_of(&mut s, &last);
    let names = names_of(&made);
    for face in 0..6 {
        let name = format!("{head}:copy(0)/F1:face({face})");
        assert!(names.contains(&name), "{name}: {names:?}");
    }
    let first_cutter = id_of(&mut s, "H0");
    // The extrusion of the first hole is the feature before the cut.
    let first_tool: u64 = first_cutter[1..].parse::<u64>().unwrap() - 1;
    let wall = names
        .iter()
        .find(|n| n.ends_with(&format!("/F{first_tool}:side(e2)")))
        .unwrap_or_else(|| panic!("the first hole's wall: {names:?}"))
        .clone();
    assert_eq!(
        wall,
        format!("{head}:copy(0)/{first_cutter}:copy(1)/F{first_tool}:side(e2)")
    );
    for name in &names {
        assert!(name.len() < 60, "{name}");
    }

    // A sketch on the top of the last body, by its name: its frame is
    // carried through all seventy cuts.
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Top", "face": format!("{head}:copy(0)/F1:face(1)")}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Top", "center": [12, 20], "radius": 3}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Top", "distance": 4, "join": last, "name": "Boss"}),
    );
    let boss = body(&mut s, "Boss");
    assert_close(
        boss["volume"].as_f64().unwrap(),
        plate_volume(holes) + PI * 9.0 * 4.0,
        "volume with the boss",
    );
    // It stands on the top, not under the bottom: its material lifts the
    // centroid above the middle of the plate.
    let z = boss["centroid"][2].as_f64().unwrap();
    assert!(z > THICK / 2.0 + 1e-3, "{z}");

    // A fillet on the rim of the first hole, by the names the block and
    // the first extrusion gave: followed through seventy-one features.
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(1)", format!("F{first_tool}:side(e2)")]],
               "radius": 0.5, "name": "Eased"}),
    );
    let eased = body(&mut s, "Eased");
    let removed = plate_volume(holes) + PI * 9.0 * 4.0 - eased["volume"].as_f64().unwrap();
    // A rounded rim takes (1 - pi/4) r^2 of section round the hole, which
    // Pappus turns about the axis at the centroid of that section, a
    // little outside the hole: between the circumference of the hole and
    // that of the hole plus the radius.
    let section = (1.0 - PI / 4.0) * 0.25;
    assert!(
        removed > section * 2.0 * PI * RADIUS && removed < section * 2.0 * PI * (RADIUS + 0.5),
        "{removed}"
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");

    // The edges are listed with their short names too: the bottom rim
    // of the first hole by the names the block and the extrusion gave.
    let listed = run(&mut s, "document.edges", json!({"feature": "Eased"}));
    let edges = listed["edges"].as_array().unwrap();
    let rim = [
        json!("F1:face(0)"),
        json!(format!("F{first_tool}:side(e2)")),
    ];
    let found = edges.iter().any(|e| {
        let short = e["short"].as_array().unwrap();
        short.len() == 2 && rim.iter().all(|r| short.contains(r))
    });
    assert!(found, "{}", edges.len());

    // Built edit by edit, the document is what regenerating it from
    // nothing gives.
    assert_ne!(s.regeneration_matches_a_full_one(), Some(false));
}

#[test]
fn seventy_holes_save_reopen_and_follow_an_edit() {
    let holes = 70;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("long-chains");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rooted = || {
        Session::new(Workspace {
            read: Access::Root(dir.clone()),
            write: Access::Root(dir.clone()),
        })
    };
    let (mut s, last) = plate_with_holes_in(rooted(), holes);
    let before = body(&mut s, &last);
    run(&mut s, "file.save", json!({"path": "holes.3dcraft"}));
    let mut s2 = rooted();
    run(&mut s2, "file.open", json!({"path": "holes.3dcraft"}));
    let after = body(&mut s2, &last);
    for key in ["volume", "area", "faces", "edges", "vertices", "faceNames"] {
        assert_eq!(after[key], before[key], "{key}");
    }
    // The plate gets longer: every hole is cut again, every name stays.
    let (length, width) = plate_size(holes);
    run(
        &mut s2,
        "feature.edit",
        json!({"feature": "Plate", "size": [length + 10.0, width, THICK]}),
    );
    let longer = body(&mut s2, &last);
    assert_close(
        longer["volume"].as_f64().unwrap(),
        plate_volume(holes) + 10.0 * width * THICK,
        "longer",
    );
    assert_eq!(longer["faceNames"], before["faceNames"]);
}

/// Unions where the body grown so far goes in as the tool each time: a
/// face of the first block is instance 1 of every union, so its name
/// keeps one number per union (`copy(1,1,…)`), and still nothing caps
/// the row at 32.
#[test]
fn forty_unions_that_take_the_growing_body_as_the_tool() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "name": "B0"}),
    );
    let mut grown = "B0".to_string();
    let count = 40;
    for i in 1..=count {
        let block = format!("B{i}");
        built(
            &mut s,
            "feature.block",
            json!({"size": [10, 10, 10], "position": [8.0 * i as f64, 0, 0], "name": block}),
        );
        let union = format!("U{i}");
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": block, "tool": grown, "name": union}),
        );
        grown = union;
    }
    let made = body(&mut s, &grown);
    assert_close(
        made["volume"].as_f64().unwrap(),
        1000.0 + count as f64 * 800.0,
        "volume",
    );
    let head = id_of(&mut s, &grown);
    let ones = vec!["1"; count].join(",");
    let left = format!("{head}:copy({ones})/F1:face(4)");
    let names = names_of(&made);
    assert!(names.contains(&left), "{left}: {names:?}");
    // A sketch on the first block's left face, through forty tools.
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Side", "face": left}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Side", "center": [0, 0], "radius": 1}),
    );
    let side = run(&mut s, "document.inspect", json!({}));
    let sketch = side["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Side")
        .unwrap();
    assert_eq!(sketch["status"], "ok", "{sketch}");
    assert_ne!(s.regeneration_matches_a_full_one(), Some(false));
}

/// Time and size against the number of holes: the names stay as short at
/// a thousand holes as at ten, so what grows is the bodies, not the names.
#[test]
#[ignore = "a timing, not a check"]
fn time_of_a_thousand_holes_cut_one_after_another() {
    for holes in [70, 250, 500, 1000] {
        let started = Instant::now();
        let (mut s, last) = plate_with_holes(holes);
        let build = started.elapsed().as_secs_f64();
        let made = body(&mut s, &last);
        assert_close(
            made["volume"].as_f64().unwrap(),
            plate_volume(holes),
            "volume",
        );
        let longest = names_of(&made).iter().map(String::len).max().unwrap();
        // Everything again, from the block on.
        let (length, width) = plate_size(holes);
        let again = Instant::now();
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Plate", "size": [length + 10.0, width, THICK]}),
        );
        let full = again.elapsed().as_secs_f64();
        let longer = body(&mut s, &last);
        assert_close(
            longer["volume"].as_f64().unwrap(),
            plate_volume(holes) + 10.0 * width * THICK,
            "longer",
        );
        println!(
            "{holes} holes: built one by one in {build:.2} s, all regenerated in {full:.2} s ({:.2} ms per cut), longest face name {longest} chars",
            full * 1e3 / holes as f64
        );
    }
}

/// Where the time of adding one more hole goes, on a plate that has many.
#[test]
#[ignore = "a timing, not a check"]
fn time_of_one_more_hole_by_command() {
    for holes in [100, 200, 400] {
        let (mut s, mut last) = plate_with_holes(holes);
        let mut spent = [0.0f64; 3];
        let extra = 10;
        for hole in holes..holes + extra {
            let sketch = format!("S{hole}");
            let t = Instant::now();
            run(&mut s, "sketch.create", json!({"name": sketch}));
            spent[0] += t.elapsed().as_secs_f64();
            let (x, y) = (5.0 + 10.0 * (hole % 4) as f64, 15.0);
            let t = Instant::now();
            run(
                &mut s,
                "sketch.circle",
                json!({"sketch": sketch, "center": [x, y], "radius": 0.5}),
            );
            spent[1] += t.elapsed().as_secs_f64();
            let name = format!("H{hole}");
            let t = Instant::now();
            built(
                &mut s,
                "feature.extrude",
                json!({"sketch": sketch, "distance": 3.0 * THICK, "symmetric": true,
                       "cut": last, "name": name}),
            );
            spent[2] += t.elapsed().as_secs_f64();
            last = name;
        }
        let ms = |t: f64| t * 1e3 / extra as f64;
        println!(
            "{holes} holes, per hole more: sketch.create {:.1} ms, sketch.circle {:.1} ms, feature.extrude with cut {:.1} ms",
            ms(spent[0]),
            ms(spent[1]),
            ms(spent[2])
        );
    }
}

/// What regeneration does besides building, counted rather than timed so
/// that it cannot grow back unseen: adding a feature to a long row, or
/// changing one near its end, looks at what the features are made from in
/// proportion to the length of the history. Until 2026-10-15 every
/// feature's whole context was put together for every command, the square
/// of the length of the history (a thousand holes cut one by one took
/// 593 s, of which 18 s was building).
#[test]
fn the_work_of_adding_to_a_long_row_grows_with_its_length() {
    let mut per_feature = Vec::new();
    for holes in [30, 90] {
        let (mut s, last) = plate_with_holes(holes);
        let mut worst = 0usize;
        let mut step = |s: &mut Session, id: &str, params: Value| {
            run(s, id, params.clone());
            let features = run(s, "document.inspect", json!({}))["features"]
                .as_array()
                .unwrap()
                .len();
            let work = s.last_regeneration_work().unwrap();
            assert!(
                work <= 4 * features,
                "{holes} holes, {id} {params}: {work} entries looked at for {features} features"
            );
            worst = worst.max(work);
        };
        // A variable nothing uses yet, then one more hole whose radius is
        // that variable.
        step(&mut s, "variable.set", json!({"name": "r", "expr": "1 mm"}));
        step(&mut s, "sketch.create", json!({"name": "Last"}));
        step(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Last", "center": [5, 20], "radius": 1}),
        );
        step(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Last", "type": "diameter", "entities": ["e2"], "value": "2 * r"}),
        );
        step(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Last", "distance": 3.0 * THICK, "symmetric": true,
                   "cut": last, "name": "Final"}),
        );
        // Changing it rebuilds the sketch, the extrusion and the cut,
        // nothing else.
        step(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": "1.5 mm"}),
        );
        assert_eq!(
            s.last_regeneration().map(|(rebuilt, _)| rebuilt),
            Some(3),
            "{holes} holes"
        );
        let made = body(&mut s, "Final");
        assert_close(
            made["volume"].as_f64().unwrap(),
            plate_volume(holes) - PI * 1.5 * 1.5 * THICK,
            "volume",
        );
        step(&mut s, "edit.undo", json!({}));
        step(&mut s, "edit.undo", json!({}));
        assert_ne!(s.regeneration_matches_a_full_one(), Some(false));
        per_feature.push(worst as f64 / (1 + 3 * holes) as f64);
    }
    // Three times the features: about the same work per feature, not three
    // times as much.
    println!("most entries looked at per feature, at 30 and 90 holes: {per_feature:?}");
    assert!(
        per_feature[1] <= 1.5 * per_feature[0],
        "work per feature at 30 and 90 holes: {per_feature:?}"
    );
}
