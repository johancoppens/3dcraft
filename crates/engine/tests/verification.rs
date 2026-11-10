//! Design verification (V1): the queries `measure.*` against closed
//! forms, and assertions kept in the document (`assert.*`) that pass,
//! break when an edit breaks the intent, and pass again when it is put
//! right; saved, undone, and refusing hostile parameters.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{AssertionStatus, Session, Workspace};
use craft3d_math::Portable;
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

fn close(got: &Value, want: f64, tol: f64, what: &str) {
    let got = got
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: not a number: {got}"));
    assert!(
        (got - want).abs() <= tol,
        "{what}: {got} is not {want} within {tol}"
    );
}

/// A plate `w × d × t` (scaled by `k`) with four bolt holes of radius 3
/// (a pattern of a cylinder 20 high from z = -5) drilled by subtracting.
fn plate(k: f64) -> Session {
    plate_in(Workspace::none(), k)
}

fn plate_in(workspace: Workspace, k: f64) -> Session {
    let mut s = Session::new(workspace);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "t", "expr": format!("{} mm", 8.0 * k)}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [80.0 * k, 50.0 * k, "t"]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": 3.0 * k, "height": 20.0 * k, "position": [10.0 * k, 10.0 * k, -5.0 * k]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"name": "Row", "source": "Drill", "count": 2, "step": [60.0 * k, 0, 0]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"name": "Drills", "source": "Row", "count": 2, "step": [0, 30.0 * k, 0]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "Drilled", "op": "subtract", "target": "Plate", "tool": "Drills"}),
    );
    s
}

fn holes() -> Value {
    json!({"hole": true, "madeBy": "Drill"})
}

#[test]
fn the_distance_between_faces_is_exact_at_three_scales() {
    for k in [0.01, 1.0, 1000.0] {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "feature.block",
            json!({"name": "A", "size": [10.0 * k, 10.0 * k, 10.0 * k]}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"name": "B", "size": [5.0 * k, 5.0 * k, 5.0 * k], "position": [17.25 * k, 2.0 * k, 3.0 * k]}),
        );
        // A cylinder of radius 3 standing at x = 20: 7 from A's side.
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "C", "radius": 3.0 * k, "height": 4.0 * k, "position": [20.0 * k, 25.0 * k, 0]}),
        );
        let ab = run(&mut s, "measure.distance", json!({"from": "A", "to": "B"}));
        close(
            &ab["distance"],
            7.25 * k,
            1e-9 * k,
            &format!("A to B at {k}"),
        );
        assert_eq!(ab["from"]["body"], "A");
        close(&ab["from"]["point"][0], 10.0 * k, 1e-9 * k, "on A's side");
        // From A's corner edge (10, 10) to the cylinder at (20, 25), r = 3.
        let ac = run(&mut s, "measure.distance", json!({"from": "A", "to": "C"}));
        let want = ((10.0_f64).ppowi(2) + (15.0_f64).ppowi(2)).sqrt() * k - 3.0 * k;
        close(&ac["distance"], want, 1e-8 * k, &format!("A to C at {k}"));
        // Faces by query: A's top to B's bottom are 3k apart in z but
        // 7.25k in x: the least distance of the two faces.
        let faces = run(
            &mut s,
            "measure.distance",
            json!({"from": {"in": "A", "normal": {"along": "z"}}, "to": {"in": "B", "normal": {"along": "-z"}}}),
        );
        let gap: f64 = (7.25_f64.ppowi(2) + 7.0_f64.ppowi(2)).sqrt() * k;
        close(&faces["distance"], gap, 1e-8 * k, "top of A to bottom of B");
    }
}

#[test]
fn clearance_tells_clear_touch_and_overlap() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "A", "size": [10, 10, 10]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Apart", "size": [4, 4, 4], "position": [12.5, 0, 0]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Touch", "size": [4, 4, 4], "position": [0, 10, 0]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Inside", "size": [2, 2, 2], "position": [4, 4, 4]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Cross", "size": [4, 4, 4], "position": [-2, -2, 8]}),
    );
    let apart = run(
        &mut s,
        "measure.clearance",
        json!({"from": "A", "to": "Apart"}),
    );
    close(&apart["clearance"], 2.5, 1e-9, "apart");
    let touch = run(
        &mut s,
        "measure.clearance",
        json!({"from": "A", "to": "Touch"}),
    );
    close(&touch["clearance"], 0.0, 1e-12, "touch");
    assert_eq!(touch["pairs"][0]["status"], "touch");
    for other in ["Inside", "Cross"] {
        let overlap = run(
            &mut s,
            "measure.clearance",
            json!({"from": "A", "to": other}),
        );
        assert!(overlap["clearance"].is_null(), "{other}: {overlap}");
        assert_eq!(overlap["pairs"][0]["status"], "overlap", "{other}");
    }
    // The least over several bodies.
    let least = run(
        &mut s,
        "measure.clearance",
        json!({"from": ["Apart"], "to": ["Touch", "Inside"]}),
    );
    // Apart (12.5..16.5, 0..4) to Inside (4..6): 6.5; to Touch (0..4, 10..14): 6.
    let want = (8.5_f64.ppowi(2) + 6.0_f64.ppowi(2)).sqrt().min(6.5);
    close(&least["clearance"], want, 1e-9, "least");
}

/// A-042: a body without vertices (a torus, a sphere) wholly inside
/// another, and one lump of a body of several, overlap; asked either way
/// round. A torus beside the block is clear at its closed-form distance.
#[test]
fn clearance_finds_a_body_without_vertices_or_one_lump_inside_another() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    // The plate spans x and y from -20 to 20, z from 0 to 10.
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 40, 10], "position": [-20, -20, 0]}),
    );
    // A ring about z at z = 5: radii 12.65 to 16.15, z from 3.25 to 6.75.
    run(
        &mut s,
        "feature.torus",
        json!({"name": "Ring", "major": 14.4, "minor": 1.75, "position": [0, 0, 5]}),
    );
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Ball", "radius": 3, "position": [0, 0, 5]}),
    );
    // Two rings in one body (a revolve of two circles about z): the
    // first at z = 5 inside the plate, the second at z = 20 above it.
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Sections", "plane": "yz"}),
    );
    for z in ["5 mm", "20 mm"] {
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Sections", "center": ["14.4 mm", z], "radius": "1.75 mm"}),
        );
    }
    run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Sections", "axis": "y", "name": "Rings"}),
    );
    // Two blocks in one body, the first (whose vertices come first) above
    // the plate, the second inside it.
    run(
        &mut s,
        "feature.block",
        json!({"name": "Cube", "size": [2, 2, 2], "position": [0, 0, 20]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"name": "Cubes", "source": "Cube", "count": 2, "step": [0, 0, -16]}),
    );
    for inner in ["Ring", "Ball", "Rings", "Cubes"] {
        for (from, to) in [("Plate", inner), (inner, "Plate")] {
            let out = run(&mut s, "measure.clearance", json!({"from": from, "to": to}));
            assert!(out["clearance"].is_null(), "{from} to {to}: {out}");
            assert_eq!(out["pairs"][0]["status"], "overlap", "{from} to {to}");
        }
    }
    // A ring beside a block: its outer radius 16.15 against the block's
    // face at x = 20, 3.85 apart (the nearest point (16.15, 0, 5) faces
    // the block's side squarely).
    run(
        &mut s,
        "feature.block",
        json!({"name": "Side", "size": [10, 40, 10], "position": [20, -20, 0]}),
    );
    for (from, to) in [("Side", "Ring"), ("Ring", "Side")] {
        let out = run(&mut s, "measure.clearance", json!({"from": from, "to": to}));
        assert_eq!(out["pairs"][0]["status"], "clear", "{from} to {to}");
        close(&out["clearance"], 20.0 - 16.15, 1e-9, "ring beside a block");
    }
}

#[test]
fn the_thinnest_wall_is_found_exactly() {
    // The plate: 8 thick, the holes far from the edges.
    let mut s = plate(1.0);
    // The plate is 8 thick, but a hole's wall is 10 - 3 = 7 from the edge.
    let thick = run(&mut s, "measure.thickness", json!({"of": "Drilled"}));
    close(&thick["thickness"], 7.0, 1e-9, "plate");
    // Measured from the top only, the plate's own thickness.
    let top = run(
        &mut s,
        "measure.thickness",
        json!({"of": {"in": "Drilled", "normal": {"along": "z"}}}),
    );
    close(&top["thickness"], 8.0, 1e-9, "from the top");
    // A hole 1 from the edge: the ligament is the thinnest place.
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Near", "radius": 3, "height": 20, "position": [40, 4, -5]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "Nicked", "op": "subtract", "target": "Drilled", "tool": "Near"}),
    );
    let thin = run(&mut s, "measure.thickness", json!({"of": "Nicked"}));
    close(&thin["thickness"], 1.0, 1e-6, "ligament");
    // A tube: radius 10 outside, 8 inside, 2 thick.
    let mut t = Session::new(Workspace::none());
    run(&mut t, "file.new", json!({"units": "mm"}));
    run(
        &mut t,
        "feature.cylinder",
        json!({"name": "Out", "radius": 10, "height": 30}),
    );
    run(
        &mut t,
        "feature.cylinder",
        json!({"name": "In", "radius": 8, "height": 40, "position": [0, 0, -5]}),
    );
    run(
        &mut t,
        "feature.boolean",
        json!({"name": "Tube", "op": "subtract", "target": "Out", "tool": "In"}),
    );
    let tube = run(&mut t, "measure.thickness", json!({"of": "Tube"}));
    close(&tube["thickness"], 2.0, 1e-6, "tube");
}

#[test]
fn a_ray_meets_the_faces_in_order() {
    let mut s = plate(1.0);
    let ray = run(
        &mut s,
        "measure.ray",
        json!({"from": [40, 25, -10], "direction": "z"}),
    );
    assert_eq!(ray["count"], 2, "{ray}");
    close(&ray["hits"][0]["distance"], 10.0, 1e-12, "bottom");
    close(&ray["hits"][1]["distance"], 18.0, 1e-12, "top");
    assert_eq!(ray["hits"][0]["enters"], true);
    assert_eq!(ray["hits"][1]["enters"], false);
    assert_eq!(ray["hits"][0]["body"], "Drilled");
    // Down a hole's axis: nothing.
    let hole = run(
        &mut s,
        "measure.ray",
        json!({"from": [10, 10, -10], "direction": [0, 0, 1]}),
    );
    assert_eq!(hole["count"], 0, "{hole}");
    // Across a hole, sideways: in, out at the hole, in, out.
    let across = run(
        &mut s,
        "measure.ray",
        json!({"from": [-5, 10, 4], "direction": "x"}),
    );
    assert_eq!(across["count"], 6, "{across}");
    let at: Vec<f64> = across["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["distance"].as_f64().unwrap())
        .collect();
    for (got, want) in at.iter().zip([5.0, 12.0, 18.0, 72.0, 78.0, 85.0]) {
        assert!((got - want).abs() < 1e-9, "{at:?}");
    }
}

#[test]
fn holes_go_through_until_the_plate_is_thicker_than_they_are_long() {
    let mut s = plate(1.0);
    let through = run(&mut s, "measure.through", json!({"of": holes()}));
    assert_eq!(through["through"], true, "{through}");
    assert_eq!(through["throughCount"], 4);
    // The plate 16 thick: the holes (z -5 to 15) stop 1 short of the top,
    // at their own end.
    run(
        &mut s,
        "variable.set",
        json!({"name": "t", "expr": "16 mm"}),
    );
    let blind = run(&mut s, "measure.through", json!({"of": holes()}));
    assert_eq!(blind["through"], false, "{blind}");
    let face = &blind["faces"][0];
    assert_eq!(face["through"], false);
    // The material left above the hole's end, z 15 to 16.
    let blocked = &face["blocked"][0];
    close(
        &blocked["point"][2],
        15.5,
        1e-9,
        "the material above the hole",
    );
    assert!(
        !blocked["faces"].as_array().unwrap().is_empty(),
        "{blocked}"
    );
    // A boss is not a hole: its axis runs in the material.
    let mut b = Session::new(Workspace::none());
    run(&mut b, "file.new", json!({"units": "mm"}));
    run(
        &mut b,
        "feature.cylinder",
        json!({"name": "Boss", "radius": 3, "height": 5}),
    );
    let boss = run(
        &mut b,
        "measure.through",
        json!({"of": {"in": "Boss", "surface": "cylinder"}}),
    );
    assert_eq!(boss["through"], false);
    assert!(
        boss["faces"][0]["why"]
            .as_str()
            .unwrap()
            .contains("material"),
        "{boss}"
    );
    // A face that is not round is refused.
    let flat = b
        .execute(
            "measure.through",
            &json!({"of": {"in": "Boss", "surface": "plane"}}),
        )
        .unwrap_err();
    assert!(flat.message().contains("cylindrical or conical"), "{flat}");
}

#[test]
fn symmetry_about_a_plane_is_found_and_a_one_sided_hole_breaks_it() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Block", "size": [40, 20, 10], "position": [-20, 0, 0]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Pin", "radius": 4, "height": 6, "position": [0, 10, 10]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "Part", "op": "union", "target": "Block", "tool": "Pin"}),
    );
    let yes = run(
        &mut s,
        "measure.symmetry",
        json!({"of": "Part", "plane": "yz"}),
    );
    assert_eq!(yes["symmetric"], true, "{yes}");
    assert!(yes["deviation"].as_f64().unwrap() < 1e-9, "{yes}");
    // Not about xz: the part lies on one side of it.
    let no = run(
        &mut s,
        "measure.symmetry",
        json!({"of": "Part", "plane": "zx"}),
    );
    assert_eq!(no["symmetric"], false);
    // About the plane y = 10 given by a point and a normal.
    let mid = run(
        &mut s,
        "measure.symmetry",
        json!({"of": "Part", "plane": {"point": [0, "10 mm", 0], "normal": [0, 1, 0]}}),
    );
    assert_eq!(mid["symmetric"], true, "{mid}");
    // A hole on one side only.
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Bore", "radius": 2, "height": 30, "position": [12, 10, -5]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "Bored", "op": "subtract", "target": "Part", "tool": "Bore"}),
    );
    let one_side = run(
        &mut s,
        "measure.symmetry",
        json!({"of": "Bored", "plane": "yz"}),
    );
    assert_eq!(one_side["symmetric"], false, "{one_side}");
    assert!(one_side["deviation"].as_f64().unwrap() > 0.5, "{one_side}");
}

/// J-022: a named set of bodies is a subject of `area` as of `extent`:
/// the sum of the bodies' areas; and of `volume` and `cavity`. A set of
/// faces is still the faces' area.
#[test]
fn a_set_of_bodies_is_measured_by_area_as_by_the_other_measures() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Block A", "size": [20, 10, 5]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Block B", "size": [10, 10, 10], "position": [40, 0, 0]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Other", "size": [1, 1, 1], "position": [-10, 0, 0]}),
    );
    run(
        &mut s,
        "set.define",
        json!({"name": "Blocks", "kind": "body", "where": {"name": "Block*"}}),
    );
    run(
        &mut s,
        "set.define",
        json!({"name": "Tops", "kind": "face", "where": {"in": "Block A", "normal": {"along": "z"}}}),
    );
    let blocks = json!({"set": "Blocks"});
    // 2 (20·10 + 10·5 + 20·5) = 700 and 6 · 10² = 600.
    let area = run(
        &mut s,
        "variable.measure",
        json!({"name": "a", "measure": "area", "of": blocks}),
    );
    close(&area["value"], 1300.0, 1e-9, "area of the set");
    // 20·10·5 = 1000 and 10³ = 1000; no cavity.
    let volume = run(
        &mut s,
        "variable.measure",
        json!({"name": "v", "measure": "volume", "of": blocks}),
    );
    close(&volume["value"], 2000.0, 1e-9, "volume of the set");
    let cavity = run(
        &mut s,
        "variable.measure",
        json!({"name": "c", "measure": "cavity", "of": blocks}),
    );
    close(&cavity["value"], 0.0, 1e-9, "cavity of the set");
    let extent = run(
        &mut s,
        "variable.measure",
        json!({"name": "x", "measure": "extent", "of": blocks, "axis": "x"}),
    );
    close(&extent["value"], 50.0, 1e-9, "extent of the set");
    // A set of faces is the faces' area: Block A's top, 20·10.
    let top = run(
        &mut s,
        "variable.measure",
        json!({"name": "t", "measure": "area", "of": {"set": "Tops"}}),
    );
    close(&top["value"], 200.0, 1e-9, "area of a set of faces");
    // As an assertion, as the finding wrote it.
    let asserted = run(
        &mut s,
        "assert.add",
        json!({"name": "area of the set", "measure": "area", "of": blocks, "eq": "1300"}),
    );
    assert_eq!(asserted["status"], "passed", "{asserted}");
}

/// J-026: a body switched off (its condition is 0) is left out of a
/// subject that names it, so one assertion holds for every option; when
/// every body a subject names is off there is nothing to check and the
/// assertion is inactive, saying why, never passed on nothing. A name of
/// no feature stays an error.
#[test]
fn a_switched_off_body_is_left_out_of_a_subject() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "option", "expr": "0"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Base", "size": [100, 100, 10]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Option", "size": [100, 100, 10], "position": [0, 0, 20], "when": "option"}),
    );
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "apart", "measure": "overlap", "of": "Base", "to": "Option", "is": false}),
    );
    assert_eq!(added["status"], "inactive", "{added}");
    let message = added["message"].as_str().unwrap();
    assert!(
        message.contains("switched off") && message.contains("`Option` is inactive"),
        "{message}"
    );
    // A list keeps the bodies that are on: Base alone, 100 · 100 · 10.
    let volume = run(
        &mut s,
        "assert.add",
        json!({"name": "volume", "measure": "volume", "of": ["Base", "Option"], "eq": "100000"}),
    );
    assert_eq!(volume["status"], "passed", "{volume}");
    let checked = run(&mut s, "document.check", json!({}));
    assert_eq!(checked["ok"], true, "{checked}");
    assert_eq!(checked["assertionSummary"]["inactive"], 1, "{checked}");
    // Asked directly, the measure says it.
    let e = refused(
        &mut s,
        "measure.clearance",
        json!({"from": "Base", "to": "Option"}),
    );
    assert!(e.contains("switched off"), "{e}");
    // A name of no feature is still an error.
    let e = refused(
        &mut s,
        "measure.clearance",
        json!({"from": "Base", "to": "Nobody"}),
    );
    assert!(!e.contains("switched off"), "{e}");
    // A body left out of a list is said (review, S4).
    let thick = run(
        &mut s,
        "measure.thickness",
        json!({"of": ["Base", "Option"]}),
    );
    assert_eq!(
        thick["switchedOffLeftOut"],
        json!({"count": 1, "bodies": ["Option"]}),
        "{thick}"
    );
    // A sketch switched off never has a body: an error that says so, not
    // a body left out.
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Drawing", "plane": "xy"}),
    );
    run(
        &mut s,
        "feature.when",
        json!({"feature": "Drawing", "expr": "option"}),
    );
    let e = refused(
        &mut s,
        "measure.clearance",
        json!({"from": "Base", "to": ["Base", "Drawing"]}),
    );
    assert!(e.contains("Drawing") && !e.contains("switched off"), "{e}");
    // The option on: the two plates 10 apart do not overlap, and the
    // volume is both, 200000; the same assertions hold.
    run(
        &mut s,
        "variable.set",
        json!({"name": "option", "expr": "1"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    let status = |name: &str| {
        listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status("apart"), "passed", "{listed}");
    assert_eq!(status("volume"), "failed", "{listed}");
    run(
        &mut s,
        "assert.edit",
        json!({"name": "volume", "eq": "100000 + 100000 * option"}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "option", "expr": "0"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["ok"], true, "{listed}");
}

/// J-020: a bound finer than the document's tolerance is met within that
/// tolerance (the verdict does not change) and says so in a warning, in
/// the result of assert.add and assert.edit, in assert.list's row and in
/// document.check. A bound comfortably above it, and a bound of 0, do
/// not warn.
#[test]
fn a_bound_finer_than_the_tolerance_passes_with_a_warning() {
    let warnings = |row: &Value| -> Vec<String> {
        row.get("warnings")
            .map(|w| {
                w.as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t.as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    let residual = |tolerance: f64| {
        let mut s = Session::new(Workspace::none());
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": tolerance}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"name": "Plate", "size": ["10 mm", "10 mm", "10 mm"]}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "residual", "expr": "0.005 mm"}),
        );
        s
    };
    let fine =
        json!({"name": "residual", "measure": "value", "expr": "residual", "max": "0.000015 mm"});
    // At 0.01 mm, 0.005 mm is within the tolerance of 0.000015 mm: it
    // passes, and anything up to 0.000015 + 0.01 = 0.010015 mm would.
    let mut s = residual(0.01);
    let added = run(&mut s, "assert.add", fine.clone());
    assert_eq!(added["status"], "passed", "{added}");
    let want = "the bound 0.000015 mm is finer than the document's tolerance of 0.01 mm: anything up to 0.010015 mm passes";
    assert_eq!(warnings(&added), [want], "{added}");
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(warnings(&listed["assertions"][0]), [want], "{listed}");
    let checked = run(&mut s, "document.check", json!({}));
    assert_eq!(checked["ok"], true, "{checked}");
    assert_eq!(
        checked["warnings"],
        json!([{"kind": "assertion", "assertion": "residual", "warning": want}]),
        "{checked}"
    );
    // A bound comfortably above the tolerance does not warn (and passes);
    // nor does a min of 0, a natural bound.
    let edited = run(
        &mut s,
        "assert.edit",
        json!({"name": "residual", "max": "0.05 mm", "min": "0 mm"}),
    );
    assert_eq!(edited["status"], "passed", "{edited}");
    assert!(edited.get("warnings").is_none(), "{edited}");
    let checked = run(&mut s, "document.check", json!({}));
    assert!(checked.get("warnings").is_none(), "{checked}");
    // A min finer than the tolerance: anything down to 0.000015 - 0.01 =
    // -0.009985 mm passes.
    let edited = run(
        &mut s,
        "assert.edit",
        json!({"name": "residual", "min": "0.000015 mm", "max": null}),
    );
    assert_eq!(
        warnings(&edited),
        [
            "the bound 0.000015 mm is finer than the document's tolerance of 0.01 mm: anything down to -0.009985 mm passes"
        ],
        "{edited}"
    );
    // eq with a tol finer than the tolerance: 0.005 ± 0.001, met from
    // 0.004 - 0.01 = -0.006 to 0.006 + 0.01 = 0.016 mm.
    let edited = run(
        &mut s,
        "assert.edit",
        json!({"name": "residual", "eq": "0.005 mm", "tol": "0.001 mm"}),
    );
    assert_eq!(edited["status"], "passed", "{edited}");
    assert_eq!(
        warnings(&edited),
        [
            "the tolerance 0.001 mm is finer than the document's tolerance of 0.01 mm: anything from -0.006 mm to 0.016 mm passes"
        ],
        "{edited}"
    );
    // An area: the plate's 600 mm^2 to ± 0.00005, finer than the 0.0001
    // mm^2 the tolerance of 0.01 mm gives an area.
    let area = run(
        &mut s,
        "assert.add",
        json!({"name": "area", "measure": "area", "of": "Plate", "eq": 600, "tol": 0.00005}),
    );
    assert_eq!(area["status"], "passed", "{area}");
    assert_eq!(
        warnings(&area),
        [
            "the tolerance 0.00005 mm^2 is finer than the document's tolerance of 0.01 mm (0.0001 mm^2 for an area): anything from 599.99985 mm^2 to 600.00015 mm^2 passes"
        ],
        "{area}"
    );
    // At the default tolerance (1e-6 mm) the same assertion fails, and
    // its bound is not finer than the tolerance.
    let mut s = residual(1e-6);
    let added = run(&mut s, "assert.add", fine);
    assert_eq!(added["status"], "failed", "{added}");
    assert!(added.get("warnings").is_none(), "{added}");
}

/// An angle's bound finer than the document's angular tolerance warns as
/// a length's does: at 1e-6 rad (0.0000572957795 deg), a `max` of
/// 0.00001 deg is met by anything up to 0.00001 + 0.0000572957795 =
/// 0.0000672957795 deg, and a measured 0.00002 deg passes; a bound
/// comfortably above it does not warn.
#[test]
fn an_angle_bound_finer_than_the_angular_tolerance_warns() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "document.setTolerance", json!({"angular": 1e-6}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "tilt", "expr": "0.00002 deg"}),
    );
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "tilt", "measure": "value", "expr": "tilt", "max": "0.00001 deg"}),
    );
    assert_eq!(added["status"], "passed", "{added}");
    assert_eq!(
        added["warnings"],
        json!([
            "the bound 0.00001 deg is finer than the document's angular tolerance of 0.000001 rad (0.0000572957795 deg): anything up to 0.0000672957795 deg passes"
        ]),
        "{added}"
    );
    let edited = run(
        &mut s,
        "assert.edit",
        json!({"name": "tilt", "max": "0.5 deg"}),
    );
    assert_eq!(edited["status"], "passed", "{edited}");
    assert!(edited.get("warnings").is_none(), "{edited}");
}

fn states(s: &mut Session) -> Vec<(String, AssertionStatus)> {
    s.assertion_states()
        .into_iter()
        .map(|a| (a.name, a.status))
        .collect()
}

#[test]
fn assertions_pass_break_on_an_edit_and_pass_again() {
    let mut s = plate(1.0);
    let through = run(
        &mut s,
        "assert.add",
        json!({"name": "bolt holes go through", "measure": "through", "of": holes()}),
    );
    assert_eq!(through["status"], "passed", "{through}");
    let wall = run(
        &mut s,
        "assert.add",
        json!({"name": "wall", "measure": "thickness", "of": {"in": "Drilled", "normal": {"along": "z"}}, "eq": "t"}),
    );
    assert_eq!(wall["status"], "passed", "{wall}");
    let volume = 80.0 * 50.0 * 8.0 - 4.0 * std::f64::consts::PI * 9.0 * 8.0;
    run(
        &mut s,
        "assert.add",
        json!({"name": "volume", "measure": "volume", "of": "Drilled", "eq": volume, "tol": 1e-6}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "four holes", "measure": "count", "of": {"kind": "face", "where": holes()}, "eq": 4}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "width", "measure": "extent", "of": "Drilled", "axis": "x", "eq": "80 mm"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "mirror", "measure": "symmetric", "of": "Drilled", "plane": {"point": ["40 mm", 0, 0], "normal": [1, 0, 0]}}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["passed"], 6, "{listed}");
    assert_eq!(listed["ok"], true);
    // The plate made twice as thick: the holes no longer go through, the
    // volume changed; the edit's own result says so, and so does the
    // check. Regeneration is not blocked: the body is there.
    let edit = run(
        &mut s,
        "variable.set",
        json!({"name": "t", "expr": "16 mm"}),
    );
    let broken: Vec<&str> = edit["assertionSummary"]["broken"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    // The holes stop 1 below the top: the wall over them is 1 thick, found
    // from the top face (the plate's own thickness is 16 elsewhere).
    assert_eq!(
        broken,
        ["bolt holes go through", "wall", "volume"],
        "{edit}"
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], false);
    let problem = &check["problems"][0];
    assert_eq!(problem["assertion"], "bolt holes go through");
    assert_eq!(problem["measured"], false);
    assert_eq!(states(&mut s)[0].1, AssertionStatus::Failed);
    let wall_now = run(&mut s, "assert.list", json!({"details": true}))["assertions"][1].clone();
    close(&wall_now["measured"], 1.0, 1e-6, "over the blind holes");
    assert_eq!(wall_now["status"], "failed");
    // Longer holes: through again; the volume assertion now follows a
    // variable.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Drill", "height": "30 mm"}),
    );
    run(
        &mut s,
        "assert.edit",
        json!({"name": "volume", "eq": "(80 mm * 50 mm - 4 * pi * (3 mm)^2) * t", "tol": 1e-6}),
    );
    let all = run(&mut s, "assert.list", json!({}));
    assert_eq!(all["ok"], true, "{all}");
    // Undo the expected value's edit: the volume is broken again.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "assert.list", json!({}))["failed"], 1);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(run(&mut s, "assert.list", json!({}))["failed"], 0);
    // Delete, undo.
    run(&mut s, "assert.delete", json!({"name": "width"}));
    assert_eq!(run(&mut s, "assert.list", json!({}))["count"], 5);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "assert.list", json!({}))["count"], 6);
}

#[test]
fn an_assertion_follows_a_renamed_feature_and_may_be_written_first() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    // Written before there is anything to measure: an error, not a refusal.
    let early = run(
        &mut s,
        "assert.add",
        json!({"name": "size", "measure": "volume", "of": {"kind": "body", "where": {"name": "Box"}}, "min": 900}),
    );
    assert_eq!(early["status"], "error", "{early}");
    run(
        &mut s,
        "feature.block",
        json!({"name": "Box", "size": [10, 10, 10]}),
    );
    assert_eq!(
        states(&mut s),
        [("size".to_string(), AssertionStatus::Passed)]
    );
    // A body named by name is kept by id.
    run(
        &mut s,
        "assert.add",
        json!({"name": "box", "measure": "area", "of": "Box", "eq": "6 * (10 mm)^2"}),
    );
    run(
        &mut s,
        "feature.rename",
        json!({"feature": "Box", "name": "Cube"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["assertions"][1]["status"], "passed", "{listed}");
    assert_eq!(listed["assertions"][1]["measure"]["of"], "F1");
    // The query by name no longer finds it.
    assert_eq!(listed["assertions"][0]["status"], "error");
}

#[test]
fn assertions_are_saved_and_read_back() {
    let dir = std::env::temp_dir().join(format!("craft3d-assert-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plate.3dcraft");
    let path = path.to_str().unwrap();
    let mut w = plate_in(Workspace::unrestricted(), 1.0);
    run(
        &mut w,
        "assert.add",
        json!({"name": "through", "measure": "through", "of": holes()}),
    );
    run(
        &mut w,
        "assert.add",
        json!({"name": "thick", "measure": "thickness", "of": {"in": "Drilled", "normal": {"along": "z"}}, "min": "t - 1 mm", "max": "t"}),
    );
    run(
        &mut w,
        "assert.add",
        json!({"name": "alone", "measure": "interference"}),
    );
    run(
        &mut w,
        "assert.add",
        json!({"name": "four", "measure": "count", "of": {"kind": "face", "where": holes()}, "eq": 4, "tol": 0}),
    );
    run(&mut w, "file.save", json!({"path": path}));
    let mut r = Session::new(Workspace::unrestricted());
    run(&mut r, "file.open", json!({"path": path}));
    let reread = run(&mut r, "assert.list", json!({"details": true}));
    assert_eq!(reread, run(&mut w, "assert.list", json!({"details": true})));
    assert_eq!(reread["passed"], 4, "{reread}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hostile_parameters_are_refused() {
    let mut s = plate(1.0);
    for (id, params) in [
        ("measure.distance", json!({})),
        ("measure.distance", json!({"from": "Nope", "to": "Drilled"})),
        ("measure.distance", json!({"from": 3, "to": "Drilled"})),
        (
            "measure.distance",
            json!({"from": {"kind": "edge", "where": {}}, "to": "Drilled"}),
        ),
        (
            "measure.distance",
            json!({"from": "Drilled", "to": {"where": {"bogus": 1}}}),
        ),
        (
            "measure.clearance",
            json!({"from": "Drilled", "to": "Drilled"}),
        ),
        ("measure.thickness", json!({"of": []})),
        ("measure.thickness", json!({"of": "Drilled", "extra": 1})),
        ("measure.ray", json!({"from": [0, 0], "direction": "z"})),
        (
            "measure.ray",
            json!({"from": [0, 0, 0], "direction": [0, 0, 0]}),
        ),
        (
            "measure.ray",
            json!({"from": [0, "1e999", 0], "direction": "x"}),
        ),
        (
            "measure.ray",
            json!({"from": [0, 0, 0], "direction": "z", "limit": 0}),
        ),
        ("measure.through", json!({"of": "Drilled"})),
        (
            "measure.through",
            json!({"of": {"hole": true, "madeBy": "Nothing"}}),
        ),
        ("measure.symmetry", json!({"plane": "nope"})),
        ("measure.symmetry", json!({"plane": "yz", "tol": -1})),
        ("measure.symmetry", json!({"plane": {"point": [0, 0, 0]}})),
        ("assert.add", json!({"measure": "volume", "min": 1})),
        (
            "assert.add",
            json!({"name": "x", "measure": "nonsense", "min": 1}),
        ),
        ("assert.add", json!({"name": "x", "measure": "volume"})),
        (
            "assert.add",
            json!({"name": "x", "measure": "volume", "is": true}),
        ),
        (
            "assert.add",
            json!({"name": "x", "measure": "through", "of": holes(), "min": 1}),
        ),
        (
            "assert.add",
            json!({"name": "x", "measure": "volume", "eq": 1, "min": 2}),
        ),
        (
            "assert.add",
            json!({"name": "x", "measure": "volume", "plane": "yz", "min": 1}),
        ),
        (
            "assert.add",
            json!({"name": " x", "measure": "volume", "min": 1}),
        ),
        ("assert.edit", json!({"name": "nothing"})),
        ("assert.delete", json!({"name": "nothing"})),
        ("assert.list", json!({"details": "yes"})),
    ] {
        refused(&mut s, id, params);
    }
    run(
        &mut s,
        "assert.add",
        json!({"name": "x", "measure": "volume", "min": 1}),
    );
    refused(
        &mut s,
        "assert.add",
        json!({"name": "x", "measure": "volume", "min": 1}),
    );
    // Without a document every command says so.
    let mut empty = Session::new(Workspace::none());
    for id in [
        "measure.distance",
        "measure.clearance",
        "measure.thickness",
        "measure.ray",
        "measure.through",
        "measure.symmetry",
        "assert.add",
        "assert.edit",
        "assert.delete",
        "assert.list",
    ] {
        assert!(empty.execute(id, &json!({})).is_err(), "{id}");
    }
}

/// The developer's case (2026-10-21): the housing's plate made twice as
/// thick (8 to 16 mm) while the bolt holes stay 20 mm high from z = -6:
/// they end at z = 14, two short of the top, and the assertion that they
/// go through breaks; made 30 mm high, it holds again.
#[test]
fn the_housings_bolt_holes_stop_going_through_when_the_plate_is_made_thicker() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/showcase/housing.3dcraft"
    );
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.open", json!({"path": path}));
    let bolts = json!({"hole": true, "madeBy": "Hole"});
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "the bolt holes go through", "measure": "through", "of": bolts}),
    );
    assert_eq!(added["status"], "passed", "{added}");
    let six = run(&mut s, "measure.through", json!({"of": bolts}));
    assert_eq!(six["throughCount"], 6, "{six}");
    // Built as a half and its mirror: symmetric about the datum plane
    // Mid, spline sides included; front to back too (its outline, sleeve
    // and holes are), but not top to bottom.
    let mirror = run(
        &mut s,
        "measure.symmetry",
        json!({"of": "Housing", "plane": "Mid"}),
    );
    assert_eq!(mirror["symmetric"], true, "{mirror}");
    let across = run(
        &mut s,
        "measure.symmetry",
        json!({"of": "Housing", "plane": "zx"}),
    );
    assert_eq!(across["symmetric"], true, "{across}");
    let flat = run(
        &mut s,
        "measure.symmetry",
        json!({"of": "Housing", "plane": {"point": [0, 0, "4 mm"], "normal": [0, 0, 1]}}),
    );
    assert_eq!(flat["symmetric"], false, "{flat}");
    let thicker = run(
        &mut s,
        "feature.edit",
        json!({"feature": "HalfBase", "distance": "16 mm"}),
    );
    assert_eq!(
        thicker["status"], "ok",
        "regeneration is not blocked: {thicker}"
    );
    let broken = &thicker["assertionSummary"]["broken"][0];
    assert_eq!(broken["name"], "the bolt holes go through", "{thicker}");
    assert_eq!(broken["measured"], false);
    let blind = run(&mut s, "measure.through", json!({"of": bolts}));
    assert_eq!(blind["throughCount"], 0, "{blind}");
    // Material from the hole's end at z = 14 to the top at 16.
    let stop = &blind["faces"][0]["blocked"][0];
    close(
        &stop["point"][2],
        15.0,
        1e-6,
        "the material above the hole's end",
    );
    assert_eq!(run(&mut s, "document.check", json!({}))["ok"], false);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Hole", "height": "30 mm"}),
    );
    let list = run(&mut s, "assert.list", json!({}));
    assert_eq!(list["passed"], 1, "{list}");
}

/// The review's probe (2026-10-22): a plate 10 thick, pocketed from below
/// to 3, a 45° groove leaving 1.5 at x = 47.5 and ten pockets leaving 2.5.
/// The thinnest wall is 1.5; `thickness min 2 mm` must fail, not pass on
/// the least of a sample that missed the groove.
fn ridge_plate() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": ["100 mm", "100 mm", "10 mm"]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Pocket", "size": ["35 mm", "120 mm", "8 mm"], "position": ["30 mm", "-10 mm", "-1 mm"]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "Cut1", "op": "subtract", "target": "Plate", "tool": "Pocket"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Ridge", "size": ["3 mm", "3 mm", "200 mm"], "position": ["47.5 mm", "-50 mm", "8.5 mm"], "axis": [0, 1, 0], "angle": "45 deg"}),
    );
    let mut last = "Cut".to_string();
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "Cut", "op": "subtract", "target": "Cut1", "tool": "Ridge"}),
    );
    for (i, (x, y)) in [
        (72, 5),
        (84, 5),
        (72, 24),
        (84, 24),
        (72, 43),
        (84, 43),
        (72, 62),
        (84, 62),
        (72, 81),
        (84, 81),
    ]
    .into_iter()
    .enumerate()
    {
        run(
            &mut s,
            "feature.block",
            json!({"name": format!("P{i}"), "size": ["5 mm", "5 mm", "8.5 mm"], "position": [format!("{x} mm"), format!("{y} mm"), "-1 mm"]}),
        );
        let next = format!("C{i}");
        run(
            &mut s,
            "feature.boolean",
            json!({"name": next, "op": "subtract", "target": last, "tool": format!("P{i}")}),
        );
        last = next;
    }
    s
}

#[test]
fn a_wall_thinner_than_the_bound_fails_and_never_passes_on_a_sample() {
    let mut s = ridge_plate();
    // The ray down the groove's apex: 1.5 of material.
    let ray = run(
        &mut s,
        "measure.ray",
        json!({"from": [47.5, 50, 20], "direction": "-z"}),
    );
    close(
        &ray["hits"][1]["distance"],
        10.0 + 1.5,
        1e-9,
        "the ray through the groove",
    );
    let thin = run(&mut s, "measure.thickness", json!({"of": "C9"}));
    close(&thin["thickness"], 1.5, 1e-5, "the groove");
    let at_least = thin["atLeast"].as_f64().unwrap();
    assert!(at_least <= thin["thickness"].as_f64().unwrap(), "{thin}");
    let wall = run(
        &mut s,
        "assert.add",
        json!({"name": "wall", "measure": "thickness", "of": "C9", "min": "2 mm"}),
    );
    assert_eq!(wall["status"], "failed", "{wall}");
    // Below what is proven: passes.
    let low = run(
        &mut s,
        "assert.add",
        json!({"name": "low", "measure": "thickness", "of": "C9", "min": "1 mm"}),
    );
    assert_eq!(low["status"], "passed", "{low}");
}

#[test]
fn a_bound_within_what_the_search_cannot_pin_down_is_not_proven() {
    // A fat torus (R = 5, r = 4): every line into it along the inward
    // normal passes the tube's middle and is 2r = 8 long. The cheap bound
    // proves only 2 (R − r) = 2 (a line could in principle meet the tube
    // across the axis); the exact faces close in on 8 piece by piece
    // within the work allowed (since 2026-10-31).
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.torus",
        json!({"name": "Ring", "major": 5, "minor": 4}),
    );
    let thin = run(&mut s, "measure.thickness", json!({"of": "Ring"}));
    close(&thin["thickness"], 8.0, 1e-6, "the tube");
    let proven = thin["atLeast"].as_f64().unwrap();
    assert!((2.0..=8.0).contains(&proven), "what is proven: {proven}");
    // Not proven for certain: a ring turned from a spline (inside radius
    // 5, outside 10 at its ends bulging to 12), 5 thick at its ends,
    // proven near 0 (a rational surface has no normal cone).
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Profile", "plane": "zx"}),
    );
    run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "Profile", "x": "10 mm * t", "y": "10 mm + 8 mm * t * (1 - t)", "t": [0, 1]}),
    );
    run(
        &mut s,
        "sketch.path",
        json!({"sketch": "Profile", "start": "e2", "segments": [{"line": ["10 mm", "5 mm"]}, {"line": ["0 mm", "5 mm"]}, {"line": "e1"}]}),
    );
    run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Profile", "axis": "x", "name": "Ring"}),
    );
    let thin = run(&mut s, "measure.thickness", json!({"of": "Ring"}));
    close(&thin["thickness"], 5.0, 1e-3, "the ring's ends");
    let tight = run(
        &mut s,
        "assert.add",
        json!({"name": "tight", "measure": "thickness", "of": "Ring", "min": "3 mm"}),
    );
    assert_eq!(tight["status"], "notProven", "{tight}");
    // Not passed: the check warns, the list is not ok.
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["warnings"][0]["assertion"], "tight", "{check}");
    assert_eq!(check["ok"], true, "a warning is not a problem: {check}");
    assert_eq!(run(&mut s, "assert.list", json!({}))["ok"], false);
    assert_eq!(states(&mut s)[0].1, AssertionStatus::NotProven);
    // A bound above what was found fails; one below what is proven passes.
    let over = run(
        &mut s,
        "assert.add",
        json!({"name": "over", "measure": "thickness", "of": "Ring", "min": "6 mm"}),
    );
    assert_eq!(over["status"], "failed", "{over}");
    let under = run(
        &mut s,
        "assert.add",
        json!({"name": "under", "measure": "thickness", "of": "Ring", "min": "0 mm"}),
    );
    assert_eq!(under["status"], "passed", "{under}");
    // A tube: coaxial walls, proven exactly.
    let mut t = Session::new(Workspace::none());
    run(&mut t, "file.new", json!({"units": "mm"}));
    run(
        &mut t,
        "feature.cylinder",
        json!({"name": "Out", "radius": 10, "height": 30}),
    );
    run(
        &mut t,
        "feature.cylinder",
        json!({"name": "In", "radius": 8, "height": 40, "position": [0, 0, -5]}),
    );
    run(
        &mut t,
        "feature.boolean",
        json!({"name": "Tube", "op": "subtract", "target": "Out", "tool": "In"}),
    );
    let tube = run(
        &mut t,
        "assert.add",
        json!({"name": "tube", "measure": "thickness", "of": "Tube", "min": "2 mm"}),
    );
    assert_eq!(tube["status"], "passed", "{tube}");
}

/// The review's probe: a blind hole with a coaxial vent, and a through
/// hole a rib blocks a third of: neither goes through, though the axis of
/// each is clear.
#[test]
fn a_vent_or_a_rib_in_the_bore_is_not_through() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 40, 10]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Bolt", "radius": 3, "height": 7, "position": [10, 10, 4]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "C1", "op": "subtract", "target": "Plate", "tool": "Bolt"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Vent", "radius": 0.3, "height": 6, "position": [10, 10, -1]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "C2", "op": "subtract", "target": "C1", "tool": "Vent"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Bolt2", "radius": 3, "height": 12, "position": [30, 30, -1]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "C3", "op": "subtract", "target": "C2", "tool": "Bolt2"}),
    );
    // Before the rib, the second goes through.
    let clear = run(
        &mut s,
        "measure.through",
        json!({"of": {"madeBy": "Bolt2", "hole": true}}),
    );
    assert_eq!(clear["through"], true, "{clear}");
    run(
        &mut s,
        "feature.block",
        json!({"name": "Rib", "size": [1, 8, 2], "position": [31.5, 26, 4]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "C4", "op": "union", "target": "C3", "tool": "Rib"}),
    );
    for (made_by, z) in [("Bolt", 2.0), ("Bolt2", 5.0)] {
        let r = run(
            &mut s,
            "measure.through",
            json!({"of": {"in": "C4", "madeBy": made_by, "hole": true}}),
        );
        assert_eq!(r["through"], false, "{made_by}: {r}");
        let blocked = &r["faces"][0]["blocked"][0];
        close(&blocked["point"][2], z, 1e-6, made_by);
        // The axis itself is clear: one ray would have said through.
        let axis = &r["faces"][0]["axis"]["point"];
        let (x, y) = (axis[0].as_f64().unwrap(), axis[1].as_f64().unwrap());
        let ray = run(
            &mut s,
            "measure.ray",
            json!({"from": [x, y, 20], "direction": "-z", "of": "C4"}),
        );
        let along = ray["hits"].as_array().unwrap().len();
        assert!(made_by == "Bolt" || along == 0, "{ray}");
    }
}

#[test]
fn a_unit_with_a_power_after_a_number_is_refused_in_a_bound() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Cube", "size": [100, 100, 100]}),
    );
    for bad in ["50000 mm^3", "50 cm^3", "2 cm^2"] {
        let e = refused(
            &mut s,
            "assert.add",
            json!({"name": "v", "measure": "volume", "max": bad}),
        );
        assert!(e.contains("* (1 "), "{bad}: {e}");
    }
    let ok = run(
        &mut s,
        "assert.add",
        json!({"name": "v", "measure": "volume", "max": "2000 * (1 cm)^3"}),
    );
    assert_eq!(ok["status"], "passed", "{ok}");
    let small = run(
        &mut s,
        "assert.add",
        json!({"name": "w", "measure": "volume", "max": "50 * (1 cm)^3"}),
    );
    assert_eq!(small["status"], "failed", "{small}");
    // In a measure's own parameters too.
    let e = s
        .execute(
            "measure.symmetry",
            &json!({"of": "Cube", "plane": "yz", "tol": "3 mm^1"}),
        )
        .unwrap_err();
    assert!(e.message().contains("* (1 mm)"), "{e}");
}

/// `0 mm^3` is `(0 mm)^3`, which is the same: nothing to mistake, so a
/// zero tolerance (and a zero bound) is taken as written, as
/// variable.set takes it (an R1 tester found it refused).
#[test]
fn zero_tolerances_and_bounds_are_taken() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Cube", "size": [10, 10, 10]}),
    );
    for (name, params) in [
        (
            "exact volume",
            json!({"measure": "volume", "eq": "1000 * (1 mm)^3", "tol": "0 mm^3"}),
        ),
        (
            "exact area",
            json!({"measure": "area", "eq": "600 * (1 mm)^2", "tol": "0 mm^2"}),
        ),
        (
            "exact width",
            json!({"measure": "extent", "axis": "x", "eq": "10 mm", "tol": "0 mm"}),
        ),
        (
            "zero tol as a number",
            json!({"measure": "extent", "axis": "y", "eq": 10, "tol": 0}),
        ),
        (
            "at least nothing",
            json!({"measure": "volume", "min": "0 mm^3"}),
        ),
        (
            "a unit volume or more",
            json!({"measure": "volume", "min": "1 mm^3"}),
        ),
        (
            "one body",
            json!({"measure": "count", "of": {"kind": "body", "where": {"name": "Cube"}}, "eq": 1, "tol": 0}),
        ),
        (
            "no hole",
            json!({"measure": "count", "of": {"kind": "face", "where": {"hole": true}}, "eq": 0}),
        ),
    ] {
        let mut params = params;
        params["name"] = name.into();
        let added = run(&mut s, "assert.add", params);
        assert_eq!(added["status"], "passed", "{name}: {added}");
    }
    // Exactly: off by a cubic millimetre fails at a tol of 0.
    let off = run(
        &mut s,
        "assert.add",
        json!({"name": "off", "measure": "volume", "eq": "1001 * (1 mm)^3", "tol": "0 mm^3"}),
    );
    assert_eq!(off["status"], "failed", "{off}");
    // A number of its own only: 2e1 mm^3 is (20 mm)^3, which is refused.
    for bad in ["2e1 mm^3", "10 mm^3", "1.5 mm^3"] {
        let e = refused(
            &mut s,
            "assert.add",
            json!({"name": "v", "measure": "volume", "max": bad}),
        );
        assert!(e.contains("* (1 "), "{bad}: {e}");
    }
}

/// Two blocks of 10 mm (scaled by `k`), `A` at the origin and `B` from
/// `20 k` along x, with the densities given (none: no material).
fn two_blocks(k: f64, density_a: Option<f64>, density_b: Option<f64>) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    let size = [10.0 * k, 10.0 * k, 10.0 * k];
    run(&mut s, "feature.block", json!({"name": "A", "size": size}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": size, "position": [20.0 * k, 0, 0]}),
    );
    for (body, density) in [("A", density_a), ("B", density_b)] {
        if let Some(density) = density {
            run(
                &mut s,
                "body.material",
                json!({"body": body, "name": "stuff", "density": density}),
            );
        }
    }
    s
}

/// The centre of mass of bodies of different densities lies where the
/// masses balance (CubeSat CDS 2.2.11 asks it within ±20/±20/±70 mm of the
/// geometric centre): `A` of density 1000 centred at x = 5k and `B` of
/// 3000 at 25k balance at x = (1 · 5 + 3 · 25) / 4 k = 20k.
#[test]
fn the_centre_of_mass_is_asserted_per_axis() {
    for k in [1.0, 1000.0, 1e-3] {
        let mut s = two_blocks(k, Some(1000.0), Some(3000.0));
        run(
            &mut s,
            "variable.set",
            json!({"name": "cx", "expr": format!("{} mm", 15.0 * k)}),
        );
        let x = run(
            &mut s,
            "assert.add",
            json!({"name": "x", "measure": "centerOfMass", "axis": "x", "eq": format!("{} mm", 20.0 * k)}),
        );
        assert_eq!(x["status"], "passed", "{k}: {x}");
        close(&x["measured"], 20.0 * k, 1e-9 * k, "x");
        assert_eq!(x["unit"], "mm");
        // Around the geometric centre of the pair (15k), as a Jig would
        // write it: within ±6k passes, ±4k does not.
        let y = run(
            &mut s,
            "assert.add",
            json!({"name": "y", "measure": "centerOfMass", "axis": "y", "min": format!("{} mm", 4.0 * k), "max": format!("{} mm", 6.0 * k)}),
        );
        assert_eq!(y["status"], "passed", "{k}: {y}");
        let near = run(
            &mut s,
            "assert.add",
            json!({"name": "near", "measure": "centerOfMass", "axis": "x", "min": format!("cx - {} mm", 6.0 * k), "max": format!("cx + {} mm", 6.0 * k)}),
        );
        assert_eq!(near["status"], "passed", "{k}: {near}");
        let tight = run(
            &mut s,
            "assert.add",
            json!({"name": "tight", "measure": "centerOfMass", "axis": "x", "min": format!("cx - {} mm", 4.0 * k), "max": format!("cx + {} mm", 4.0 * k)}),
        );
        assert_eq!(tight["status"], "failed", "{k}: {tight}");
        // Of one body: its own centroid.
        let of_a = run(
            &mut s,
            "assert.add",
            json!({"name": "a", "measure": "centerOfMass", "of": "A", "axis": "z", "eq": format!("{} mm", 5.0 * k)}),
        );
        assert_eq!(of_a["status"], "passed", "{k}: {of_a}");
        // A change of material moves it without a body changing: B as
        // light as A balances at 15k.
        run(
            &mut s,
            "body.material",
            json!({"body": "B", "name": "light", "density": 1000}),
        );
        let listed = run(&mut s, "assert.list", json!({}));
        let x = &listed["assertions"][0];
        assert_eq!(x["status"], "failed", "{k}: {listed}");
        close(&x["measured"], 15.0 * k, 1e-9 * k, "x after the material");
        assert_eq!(listed["assertions"][3]["status"], "passed", "{listed}");
    }
}

/// Where the box begins and ends along an axis, each side on its own: a
/// CubeSat (CDS 2.2.3) lets nothing stick out more than 6.5 mm beyond its
/// rails on +x and on -x separately, which an extent (their sum) cannot
/// say. Rails of width `w` centred on the origin, a panel sticking out
/// `p` beyond the +x rail.
#[test]
fn each_side_of_the_box_is_asserted_on_its_own() {
    for k in [1.0, 1000.0, 1e-3] {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "variable.set",
            json!({"name": "w", "expr": format!("{} mm", 100.0 * k)}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "p", "expr": format!("{} mm", 5.0 * k)}),
        );
        // What may stick out: 6.5 mm at the scale 1.
        run(
            &mut s,
            "variable.set",
            json!({"name": "out", "expr": format!("{} mm", 6.5 * k)}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"name": "Rails", "size": ["w", "w", "w"], "position": ["-w / 2", "-w / 2", 0]}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"name": "Panel", "size": ["p + w / 100", "w / 10", "w / 10"], "position": ["w / 2 - w / 100", 0, 0]}),
        );
        let plus = run(
            &mut s,
            "assert.add",
            json!({"name": "+x", "measure": "boxMax", "axis": "x", "max": "w / 2 + out"}),
        );
        assert_eq!(plus["status"], "passed", "{k}: {plus}");
        close(&plus["measured"], 55.0 * k, 1e-9 * k, "+x");
        let minus = run(
            &mut s,
            "assert.add",
            json!({"name": "-x", "measure": "boxMin", "axis": "x", "min": "-w / 2 - out"}),
        );
        assert_eq!(minus["status"], "passed", "{k}: {minus}");
        close(&minus["measured"], -50.0 * k, 1e-9 * k, "-x");
        assert_eq!(minus["unit"], "mm");
        // Of one body, and along another axis.
        let panel = run(
            &mut s,
            "assert.add",
            json!({"name": "panel", "measure": "boxMin", "of": "Panel", "axis": "x", "eq": "w / 2 - w / 100"}),
        );
        assert_eq!(panel["status"], "passed", "{k}: {panel}");
        let top = run(
            &mut s,
            "assert.add",
            json!({"name": "top", "measure": "boxMax", "axis": "z", "eq": "w"}),
        );
        assert_eq!(top["status"], "passed", "{k}: {top}");
        // The panel out 8: +x fails, -x still passes, while the extent
        // (the sum, 58 + 50, is within w + 2 × 6.5) would not tell which side.
        let edit = run(
            &mut s,
            "variable.set",
            json!({"name": "p", "expr": format!("{} mm", 8.0 * k)}),
        );
        let broken: Vec<&str> = edit["assertionSummary"]["broken"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["name"].as_str().unwrap())
            .collect();
        assert_eq!(broken, ["+x"], "{k}: {edit}");
    }
    // Hostile: no axis, a wrong one, a parameter it does not take.
    let mut s = two_blocks(1.0, None, None);
    for measure in ["boxMin", "boxMax"] {
        let e = refused(
            &mut s,
            "assert.add",
            json!({"name": "q", "measure": measure, "axis": "x", "from": "A", "max": 1}),
        );
        assert!(e.contains("`from`"), "{e}");
        for axis in [json!(null), json!("w"), json!(3)] {
            let v = run(
                &mut s,
                "assert.add",
                json!({"name": "q", "measure": measure, "axis": axis, "max": 1}),
            );
            assert_eq!(v["status"], "error", "{v}");
            assert!(v["message"].as_str().unwrap().contains("`axis`"), "{v}");
            run(&mut s, "assert.delete", json!({"name": "q"}));
        }
    }
}

/// The box of a body with a spline face cut by a boolean is the box of
/// what is left, not of the spline's whole control net (an R1 tester's
/// gear measured 44.99 x 17 mm where it is 43.99 x 15). A parabolic
/// segment `y = t^2 / (4 f)` for `t` from `-a` to `a`, closed by its chord
/// and extruded `d`, with everything past `x = c` cut away: its box is
/// `x` from `-a` to `c`, `y` from 0 (the vertex) to `a^2 / (4 f)` (the
/// end at `-a`), `z` from 0 to `d`.
#[test]
fn the_box_of_a_cut_spline_face_is_the_box_of_what_is_left() {
    for k in [1.0, 1000.0, 1e-3] {
        let (f, a, c, d) = (10.0 * k, 20.0 * k, 8.0 * k, 5.0 * k);
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"units": "mm"}));
        for (name, value) in [("f", f), ("a", a), ("c", c)] {
            run(
                &mut s,
                "variable.set",
                json!({"name": name, "expr": format!("{value} mm")}),
            );
        }
        run(&mut s, "sketch.create", json!({"name": "Dish"}));
        run(
            &mut s,
            "sketch.curve",
            json!({"sketch": "Dish", "x": "t", "y": "t^2 / (4 * f)", "t": ["-a", "a"]}),
        );
        run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Dish", "from": "e2", "to": "e1"}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Dish", "distance": d, "name": "Segment"}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"name": "Cutter", "size": [2.0 * a, 2.0 * a, 3.0 * d], "position": [c, -a, -d]}),
        );
        let cut = run(
            &mut s,
            "feature.boolean",
            json!({"name": "Cut", "op": "subtract", "target": "Segment", "tool": "Cutter"}),
        );
        assert_eq!(cut["status"], "ok", "{k}: {cut}");
        let top = a * a / (4.0 * f);
        // The box is found on a mesh: its vertices lie on the body (here to
        // within the document's tolerance, 1e-6 mm, by which the box
        // reached is shrunk), and the body lies within twice the mesh's
        // tolerance of it. That tolerance starts at a hundred-thousandth
        // of the side of a square of the area (some 3e-4 k here) and a
        // bound left undecided is looked at again on meshes eight times
        // finer, down to ten linear tolerances (1e-5 mm). A tol of 1e-4 k
        // needs that at the scale 1; at the scale 1e-3 the floor makes it
        // 5e-5 mm.
        // (Plus the rounding of the measure: the extent is shrunk on both
        // sides, and came out 2.000000002e-6 short at the scale 1 when the
        // fit of the formula took its ends' directions, 2026-10-28.)
        let reached = f64::max(1e-6 * k, 2e-6) * (1.0 + 1e-6);
        let tol = f64::max(1e-4 * k, 5e-5);
        for (measure, axis, want) in [
            ("boxMax", "x", c),
            ("boxMin", "x", -a),
            ("extent", "x", a + c),
            ("boxMax", "y", top),
            ("extent", "z", d),
        ] {
            let name = format!("{measure} {axis}");
            let got = run(
                &mut s,
                "assert.add",
                json!({"name": name, "measure": measure, "of": "Cut", "axis": axis, "eq": format!("{want} mm"), "tol": format!("{tol} mm")}),
            );
            assert_eq!(got["status"], "passed", "{k} {name}: {got}");
            close(&got["measured"], want, reached, &name);
        }
        // At the scale 1 the first look (3e-4) cannot settle 1e-4: the
        // mesh it was settled on is finer.
        if k == 1.0 {
            let listed = run(&mut s, "assert.list", json!({"details": true}));
            let at = listed["assertions"][0]["detail"]["meshTolerance"]
                .as_f64()
                .unwrap();
            assert!(at < 3e-4 / 7.0, "{listed}");
        }
        // The vertex of the parabola lies inside a face: reached within the
        // mesh's tolerance and settled with that room (at least the floor
        // of ten linear tolerances, twice over, at the scale 1e-3).
        let low = run(
            &mut s,
            "assert.add",
            json!({"name": "low", "measure": "boxMin", "of": "Cut", "axis": "y", "min": format!("{} mm", -f64::max(0.01 * k, 5e-5)), "max": format!("{} mm", f64::max(0.01 * k, 5e-5))}),
        );
        assert_eq!(low["status"], "passed", "{k}: {low}");
        // Past the vertex by more than the mesh can be off: failed, not
        // passed and not left unproven.
        let above = run(
            &mut s,
            "assert.add",
            json!({"name": "above", "measure": "boxMin", "of": "Cut", "axis": "y", "min": format!("{} mm", 0.1 * k)}),
        );
        assert_eq!(above["status"], "failed", "{k}: {above}");
    }
}

#[test]
fn a_centre_of_mass_without_a_material_says_which_body_lacks_one() {
    let mut s = two_blocks(1.0, Some(1000.0), None);
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "x", "measure": "centerOfMass", "axis": "x", "eq": "20 mm"}),
    );
    assert_eq!(added["status"], "error", "{added}");
    let message = added["message"].as_str().unwrap();
    assert!(
        message.contains("`B` has no material, so no mass; give it one with body.material"),
        "{message}"
    );
    // The same words as the mass measure.
    let mass = run(
        &mut s,
        "assert.add",
        json!({"name": "m", "measure": "mass", "min": 0}),
    );
    assert!(
        mass["message"]
            .as_str()
            .unwrap()
            .contains("`B` has no material, so no mass"),
        "{mass}"
    );
    // Of A alone it is measured.
    let a = run(
        &mut s,
        "assert.add",
        json!({"name": "a", "measure": "centerOfMass", "of": "A", "axis": "x", "eq": "5 mm"}),
    );
    assert_eq!(a["status"], "passed", "{a}");
    for (params, says) in [
        (
            json!({"name": "q", "measure": "centerOfMass", "eq": 1}),
            "`axis`",
        ),
        (
            json!({"name": "q", "measure": "centerOfMass", "axis": "w", "eq": 1}),
            "`axis`",
        ),
        (
            json!({"name": "q", "measure": "centerOfMass", "axis": "x", "is": true}),
            "is a number",
        ),
        (
            json!({"name": "q", "measure": "centerOfMass", "axis": "x", "from": "A", "eq": 1}),
            "`from`",
        ),
    ] {
        match s.execute("assert.add", &params) {
            Err(e) => assert!(e.message().contains(says), "{params}: {e}"),
            // A missing or wrong axis is found when it is measured.
            Ok(v) => {
                assert_eq!(v["status"], "error", "{params}: {v}");
                assert!(v["message"].as_str().unwrap().contains(says), "{v}");
                run(&mut s, "assert.delete", json!({"name": "q"}));
            }
        }
    }
}

/// `overlap` is true when bodies overlap, so `is: false` asserts that
/// none do, and the message says in words which overlap by how much. The
/// old form `interference` (true when none overlap) means what it meant:
/// written as overlap with the opposite `is`, and read so from a file.
#[test]
fn overlap_reads_the_way_it_is_meant_and_the_old_form_keeps_its_meaning() {
    // A at 0..10 and B moved to 5..15 along x: 500 mm^3 in common.
    let mut s = two_blocks(1.0, None, None);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B", "position": [5, 0, 0]}),
    );
    let none = run(
        &mut s,
        "assert.add",
        json!({"name": "apart", "measure": "overlap", "of": "A", "to": "B", "is": false}),
    );
    assert_eq!(none["status"], "failed", "{none}");
    assert_eq!(
        none["message"], "A overlaps B by 500 mm^3, expected no overlap",
        "{none}"
    );
    assert_eq!(none["measured"], true);
    assert_eq!(none["expected"], "no overlap");
    let fit = run(
        &mut s,
        "assert.add",
        json!({"name": "press fit", "measure": "overlap", "is": true}),
    );
    assert_eq!(fit["status"], "passed", "{fit}");
    assert_eq!(fit["message"], "A overlaps B by 500 mm^3");
    // `is` is said, not defaulted.
    let e = refused(
        &mut s,
        "assert.add",
        json!({"name": "q", "measure": "overlap"}),
    );
    assert!(e.contains("overlap needs `is`"), "{e}");
    // The old form, as a Jig written before may still send it: is true
    // meant none overlap. It is written as overlap, is false.
    let old = run(
        &mut s,
        "assert.add",
        json!({"name": "old", "measure": "interference", "of": "A", "to": "B"}),
    );
    assert_eq!(old["status"], "failed", "{old}");
    assert_eq!(old["measure"]["measure"], "overlap");
    assert_eq!(old["expect"], json!({"is": false}));
    assert_eq!(old["message"], none["message"]);
    let old_false = run(
        &mut s,
        "assert.add",
        json!({"name": "old fit", "measure": "interference", "is": false}),
    );
    assert_eq!(old_false["status"], "passed", "{old_false}");
    assert_eq!(old_false["expect"], json!({"is": true}));
    // Apart again: no overlap.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "B", "position": [20, 0, 0]}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    let by_name = |name: &str| {
        listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(by_name("apart")["status"], "passed", "{listed}");
    assert_eq!(by_name("apart")["message"], "no overlap");
    assert_eq!(by_name("old")["status"], "passed");
    assert_eq!(by_name("press fit")["status"], "failed");
    assert_eq!(
        by_name("press fit")["message"],
        "no overlap, expected an overlap"
    );
}

/// A document saved with the old form (format 35) keeps meaning what it
/// meant: opened, its `interference` assertion with is true is migrated
/// to `overlap` with is false, so it passes while none overlap and shows
/// what was measured against what it must be the same way round.
#[test]
fn a_saved_old_interference_assertion_means_what_it_meant() {
    let dir = std::env::temp_dir().join(format!("craft3d-overlap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("old.3dcraft");
    let mut w = Session::new(Workspace::unrestricted());
    run(&mut w, "file.new", json!({"units": "mm"}));
    run(
        &mut w,
        "feature.block",
        json!({"name": "A", "size": [10, 10, 10]}),
    );
    run(
        &mut w,
        "feature.block",
        json!({"name": "B", "size": [10, 10, 10], "position": [20, 0, 0]}),
    );
    run(&mut w, "file.save", json!({"path": path.to_str().unwrap()}));
    // The same file as format 35 wrote it, with two assertions of the old
    // form.
    let bytes = std::fs::read(&path).unwrap();
    let entries = craft3d_zipstore::read_stored(&bytes).unwrap();
    let manifest = entries
        .iter()
        .find(|e| e.name == craft3d_format::MANIFEST_NAME)
        .unwrap();
    let mut manifest: Value = serde_json::from_slice(manifest.data).unwrap();
    manifest["format_version"] = json!(35);
    manifest["document"]["assertions"] = json!([
        {"name": "clear", "measure": {"measure": "interference"}, "expect": {"is": true}},
        {"name": "clash", "measure": {"measure": "interference"}, "expect": {"is": false}},
    ]);
    let manifest = serde_json::to_vec(&manifest).unwrap();
    let old =
        craft3d_zipstore::write_stored(&[(craft3d_format::MANIFEST_NAME, &manifest)]).unwrap();
    std::fs::write(&path, old).unwrap();
    let mut r = Session::new(Workspace::unrestricted());
    run(&mut r, "file.open", json!({"path": path.to_str().unwrap()}));
    let listed = run(&mut r, "assert.list", json!({}));
    let clear = &listed["assertions"][0];
    assert_eq!(clear["status"], "passed", "{listed}");
    assert_eq!(clear["measure"], json!({"measure": "overlap"}));
    assert_eq!(clear["expect"], json!({"is": false}));
    // What was measured agrees with what it must be: no overlap.
    assert_eq!(clear["measured"], false);
    assert_eq!(clear["message"], "no overlap");
    assert_eq!(clear["expected"], "no overlap");
    let clash = &listed["assertions"][1];
    assert_eq!(clash["status"], "failed", "{listed}");
    assert_eq!(clash["expect"], json!({"is": true}));
    assert_eq!(clash["measured"], false);
    assert_eq!(clash["message"], "no overlap, expected an overlap");
    // Edited, it stays as it was migrated.
    let edited = run(
        &mut r,
        "assert.edit",
        json!({"name": "clear", "rename": "apart"}),
    );
    assert_eq!(edited["measure"]["measure"], "overlap");
    assert_eq!(edited["expect"], json!({"is": false}));
    assert_eq!(edited["status"], "passed");
    std::fs::remove_dir_all(&dir).ok();
}

/// A version-35 file written by the build of main 681d5e72
/// (`crates/format/tests/fixtures/migrations/v35-interference.3dcraft`),
/// with `interference` assertions of `is` true and false and with `of`
/// and `to` subjects: opened under version 36 each is the `overlap` with
/// the opposite `is`, its subjects as they were, and stands as it stood
/// then (the statuses that build reported).
#[test]
fn a_version_35_file_with_interference_assertions_stands_as_it_stood() {
    let fixtures = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../format/tests/fixtures/migrations");
    let dir = std::env::temp_dir().join(format!("craft3d-v35-overlap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("v35-interference.3dcraft");
    std::fs::copy(fixtures.join("v35-interference.3dcraft"), &path).unwrap();
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures.join("v35-interference.expected.json")).unwrap(),
    )
    .unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.open", json!({"path": path.to_str().unwrap()}));
    let listed = run(&mut s, "assert.list", json!({}));
    let got = listed["assertions"].as_array().unwrap();
    let was = expected["assertions"].as_array().unwrap();
    assert_eq!(got.len(), was.len(), "{listed}");
    for (now, then) in got.iter().zip(was) {
        assert_eq!(now["name"], then["name"]);
        assert_eq!(now["status"], then["status"], "{now} / {then}");
        assert_eq!(now["measure"]["measure"], "overlap", "{now}");
        let old_is = then["expected"] == "true";
        assert_eq!(now["expect"], json!({"is": !old_is}), "{now}");
        // What was measured reads the same way round as what it must be.
        let measured_overlap = now["measured"].as_bool().unwrap();
        assert_eq!(
            measured_overlap == !old_is,
            now["status"] == "passed",
            "{now}"
        );
    }
    assert_eq!(
        got[0]["measure"],
        json!({"measure": "overlap", "of": "F3", "to": ["F1", "F2"]})
    );
    assert_eq!(got[0]["message"], "no overlap");
    assert_eq!(
        got[1]["measure"],
        json!({"measure": "overlap", "of": "F2", "to": "F1"})
    );
    assert_eq!(got[1]["message"], "Pin overlaps Base by 500 mm^3");
    assert_eq!(
        got[2]["message"],
        "Base overlaps Pin by 500 mm^3, expected no overlap"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_edit_measures_again_only_the_assertions_whose_subjects_it_changed() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "10 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "A", "size": ["w", 10, 10]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "B", "size": [5, 5, 5], "position": [20, 0, 0]}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "a", "measure": "thickness", "of": "A", "min": "w - 2 mm"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "b", "measure": "volume", "of": "B", "eq": 125}),
    );
    let before = s.assertions_measured().unwrap();
    // B changes: only b is measured again.
    let edit = run(
        &mut s,
        "feature.edit",
        json!({"feature": "B", "size": [6, 5, 5]}),
    );
    assert_eq!(edit["assertionSummary"]["broken"][0]["name"], "b", "{edit}");
    assert_eq!(s.assertions_measured().unwrap(), before + 1);
    // A body added elsewhere: nothing.
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "S", "radius": 1, "position": [50, 0, 0]}),
    );
    run(&mut s, "assert.list", json!({}));
    assert_eq!(s.assertions_measured().unwrap(), before + 1);
    // w changes A and the bound of a.
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "12 mm"}),
    );
    assert_eq!(run(&mut s, "assert.list", json!({}))["passed"], 1);
    assert_eq!(s.assertions_measured().unwrap(), before + 2);
    // Undo rebuilds A (bodies are known by identity, not by content), so
    // a is measured again; b is not.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "assert.list", json!({}));
    assert_eq!(s.assertions_measured().unwrap(), before + 3);
}

#[test]
fn renames_reach_into_what_assertions_measure() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "half", "expr": "20 mm"}),
    );
    // Written before its body exists, by name.
    let early = run(
        &mut s,
        "assert.add",
        json!({"name": "box", "measure": "volume", "of": "Box", "min": 1}),
    );
    assert_eq!(early["status"], "error", "{early}");
    run(
        &mut s,
        "feature.block",
        json!({"name": "Box", "size": [40, 10, 10]}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "mirror", "measure": "symmetric", "of": "Box", "plane": {"point": ["half", 0, 0], "normal": [1, 0, 0]}}),
    );
    assert_eq!(run(&mut s, "assert.list", json!({}))["passed"], 2);
    run(
        &mut s,
        "feature.rename",
        json!({"feature": "Box", "name": "Brick"}),
    );
    run(
        &mut s,
        "variable.rename",
        json!({"name": "half", "to": "middle"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["passed"], 2, "{listed}");
    assert_eq!(listed["assertions"][0]["measure"]["of"], "F1");
    assert_eq!(
        listed["assertions"][1]["measure"]["plane"]["point"][0],
        "middle"
    );
}
