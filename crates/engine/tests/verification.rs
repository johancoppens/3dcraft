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
        let want = ((10.0_f64).powi(2) + (15.0_f64).powi(2)).sqrt() * k - 3.0 * k;
        close(&ac["distance"], want, 1e-8 * k, &format!("A to C at {k}"));
        // Faces by query: A's top to B's bottom are 3k apart in z but
        // 7.25k in x: the least distance of the two faces.
        let faces = run(
            &mut s,
            "measure.distance",
            json!({"from": {"in": "A", "normal": {"along": "z"}}, "to": {"in": "B", "normal": {"along": "-z"}}}),
        );
        let gap: f64 = (7.25_f64.powi(2) + 7.0_f64.powi(2)).sqrt() * k;
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
    let want = (8.5_f64.powi(2) + 6.0_f64.powi(2)).sqrt().min(6.5);
    close(&least["clearance"], want, 1e-9, "least");
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
    let blocked = &face["blocked"][0];
    assert_eq!(blocked["toward"], "+axis");
    close(
        &blocked["point"][2],
        15.0,
        1e-9,
        "blocked at the hole's end",
    );
    close(&blocked["beyondEnd"], 0.0, 1e-9, "right at the end");
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
    let broken: Vec<&str> = edit["assertions"]["broken"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert_eq!(broken, ["bolt holes go through", "volume"], "{edit}");
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], false);
    let problem = &check["problems"][0];
    assert_eq!(problem["assertion"], "bolt holes go through");
    assert_eq!(problem["measured"], false);
    assert_eq!(states(&mut s)[0].1, AssertionStatus::Failed);
    let wall_now = run(&mut s, "assert.list", json!({"details": true}))["assertions"][1].clone();
    close(&wall_now["measured"], 16.0, 1e-9, "the wall follows");
    assert_eq!(wall_now["status"], "passed");
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
        json!({"name": "thick", "measure": "thickness", "of": "Drilled", "min": "t - 1 mm", "max": "t"}),
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
    let broken = &thicker["assertions"]["broken"][0];
    assert_eq!(broken["name"], "the bolt holes go through", "{thicker}");
    assert_eq!(broken["measured"], false);
    let blind = run(&mut s, "measure.through", json!({"of": bolts}));
    assert_eq!(blind["throughCount"], 0, "{blind}");
    let stop = &blind["faces"][0]["blocked"][0];
    close(
        &stop["point"][2],
        14.0,
        1e-9,
        "blocked where the hole ends, 2 below the top",
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
