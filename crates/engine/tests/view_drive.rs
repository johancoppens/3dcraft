//! `view.drive`: a mate's joint held by hand for the view (docs/design/
//! assemblies.md › *Driving a joint by hand*). The document never changes;
//! couplings and closures follow; a dead point stops the view short.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{refused, session_in, workdir};
use craft3d_engine::{SceneQuality, Session, Workspace, scene_begin, scene_bytes, scene_next};
use serde_json::{Value, json};

fn planetary() -> Session {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/examples/published/planetary-gearbox.3dcraftasm"
    ))
    .unwrap();
    let mut s = Session::new(Workspace::memory());
    s.open_bytes("planetary-gearbox.3dcraftasm", &bytes)
        .unwrap();
    s
}

/// The value shown for a mate (the outline the window lists), in
/// degrees for an angle.
fn shown(s: &mut Session, mate: &str) -> f64 {
    let outline = s.assembly_outline().unwrap();
    let m = outline.mates.iter().find(|m| m.name == mate).unwrap();
    let (v, angle) = m.value.unwrap();
    if angle { v.to_degrees() } else { v }
}

/// What says the document changed: dirty, and the undo and redo lists.
fn document_state(s: &mut Session) -> Value {
    let inspect = run(s, "document.inspect", json!({}));
    json!([inspect["dirty"], inspect["history"], inspect["variables"]])
}

/// A crank-rocker four-bar (links 60, 20, 60, 40 between their ends):
/// `Drive` turns the crank as a formula of `t`, the rocker (`Hip`)
/// rocks within a range, so it cannot be turned past its dead points.
fn four_bar(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({ "name": "len", "expr": "24 mm" }));
    run(
        s,
        "variable.publish",
        json!({ "name": "len", "min": "10 mm", "max": "500 mm" }),
    );
    run(
        s,
        "feature.block",
        json!({ "name": "Bar", "size": ["len", 4, 2] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End1", "through": [[2, 2, 0], [2, 2, 1]] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]] }),
    );
    run(s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(s, "assembly.new", json!({}));
    for (name, len) in [
        ("Ground", "64 mm"),
        ("Crank", "24 mm"),
        ("Coupler", "64 mm"),
        ("Rocker", "44 mm"),
    ] {
        run(
            s,
            "instance.add",
            json!({ "source": "bar.3dcraft", "name": name, "body": "Bar",
                    "params": { "len": len } }),
        );
    }
    for (name, a, b) in [
        ("Base", "world/z", "Ground/End1"),
        ("Drive", "Ground/End1", "Crank/End1"),
        ("Elbow", "Crank/End2", "Coupler/End1"),
        ("Knee", "Coupler/End2", "Rocker/End2"),
        ("Hip", "Ground/End2", "Rocker/End1"),
    ] {
        let kind = if name == "Base" { "fixed" } else { "revolute" };
        run(
            s,
            "mate.add",
            json!({ "kind": kind, "a": a, "b": b, "name": name, "flip": true }),
        );
    }
    run(s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        s,
        "variable.set",
        json!({ "name": "Drive_angle", "expr": "90 deg + 360 deg * t" }),
    );
    let solve = run(s, "assembly.solve", json!({}));
    assert_eq!(solve["ok"], true, "{solve}");
}

/// The planetary's carrier (`Output`) is not driven: the sun is, by `t`
/// (`SunOnCarrier`). Held by hand, the carrier turns and the sun follows
/// it by the gearbox's ratio (Zr/Zs = 48/12 relative to the carrier),
/// the sun's driver let go of; the document does not change.
#[test]
fn a_coupled_mate_held_by_hand_turns_the_gears_it_drives() {
    let mut s = planetary();
    let before = document_state(&mut s);
    assert!(shown(&mut s, "Output").abs() < 1e-6);
    let answer = run(&mut s, "view.drive", json!({"mate": "Output", "value": 30}));
    assert_eq!(answer["mate"], "Output", "{answer}");
    assert_eq!(answer["released"], json!(["SunOnCarrier"]), "{answer}");
    assert_eq!(answer["stopped"], Value::Null, "{answer}");
    assert!((answer["reached"].as_f64().unwrap() - 30.0).abs() < 1e-9);
    assert!((shown(&mut s, "Output").abs() - 30.0).abs() < 1e-6);
    let sun = shown(&mut s, "SunOnCarrier");
    assert!(
        (sun.abs() - 4.0 * 30.0).abs() < 1e-4,
        "sun on carrier {sun}"
    );
    assert_eq!(document_state(&mut s), before);
    assert_eq!(s.driven_mate().as_deref(), Some("Output"));
    // A further value, as a slider dragged on: from the document's pose
    // again, the same pose whichever way it came.
    run(
        &mut s,
        "view.drive",
        json!({"mate": "Output", "value": "-45 deg"}),
    );
    assert!((shown(&mut s, "Output").abs() - 45.0).abs() < 1e-6);
    let sun = shown(&mut s, "SunOnCarrier");
    assert!((sun.abs() - 4.0 * 45.0).abs() < 1e-4, "{sun}");
    // Let go: the document's pose again.
    let answer = run(&mut s, "view.drive", json!({"mate": null}));
    assert_eq!(answer["mate"], Value::Null);
    assert!(shown(&mut s, "Output").abs() < 1e-6);
    assert!(shown(&mut s, "SunOnCarrier").abs() < 1e-6);
    assert_eq!(s.driven_mate(), None);
    assert_eq!(document_state(&mut s), before);
}

/// A mate whose joint a formula of `t` drives is held by hand at a
/// value: its own driver gives way, nothing else is let go of, and the
/// document (its dirty state, its undo history, its variables) does not
/// change. A moment of the motion lets go of it.
#[test]
fn a_motion_driven_mate_held_by_hand_leaves_the_document_as_it_is() {
    let mut s = planetary();
    let before = document_state(&mut s);
    let answer = run(
        &mut s,
        "view.drive",
        json!({"mate": "SunOnCarrier", "value": 90}),
    );
    assert_eq!(answer["released"], json!([]), "{answer}");
    assert!((shown(&mut s, "SunOnCarrier") - 90.0).abs() < 1e-6);
    // The carrier follows by the ratio: a quarter of the sun's turn on it.
    let carrier = shown(&mut s, "Output");
    assert!((carrier.abs() - 90.0 / 4.0).abs() < 1e-4, "{carrier}");
    assert_eq!(document_state(&mut s), before);
    run(&mut s, "view.motion", json!({"t": 0.5}));
    assert_eq!(s.driven_mate(), None);
    run(&mut s, "view.motion", json!({"t": null}));
    assert_eq!(document_state(&mut s), before);
}

/// An edit lets go of the joint held by hand, as of a moment of the
/// motion: the view shows the document as it now is.
#[test]
fn an_edit_lets_go_of_the_joint_held_by_hand() {
    let mut s = planetary();
    run(&mut s, "view.drive", json!({"mate": "Output", "value": 10}));
    assert_eq!(s.driven_mate().as_deref(), Some("Output"));
    run(&mut s, "variable.set", json!({"name": "t", "expr": "0.25"}));
    assert_eq!(s.driven_mate(), None);
    let answer = run(&mut s, "view.drive", json!({}));
    assert_eq!(answer["mate"], Value::Null);
}

/// A four-bar's rocker turned by hand turns the crank (the crank's
/// driver let go of) until the rocker's dead point: the view stops at
/// the last value it reaches and says why, and the document does not
/// change.
#[test]
fn a_dead_point_stops_the_joint_held_by_hand_and_says_so() {
    let dir = workdir("view-drive", "four-bar");
    let mut s = session_in(&dir);
    four_bar(&mut s);
    let before = document_state(&mut s);
    let start = shown(&mut s, "Hip");
    let small = run(
        &mut s,
        "view.drive",
        json!({"mate": "Hip", "value": start + 5.0}),
    );
    assert_eq!(small["released"], json!(["Drive"]), "{small}");
    assert_eq!(small["stopped"], Value::Null, "{small}");
    assert!((shown(&mut s, "Hip") - (start + 5.0)).abs() < 1e-6);
    // The crank followed: no longer where its driver has it.
    assert!((shown(&mut s, "Drive") - 90.0).abs() > 1.0);
    // A rocker of a crank-rocker cannot turn a half turn either way: it
    // stops where the crank and the coupler line up, at the angles the
    // law of cosines gives (cos θ = 1/4 and −3/4 for these links).
    let limits = [138.590_377_890_729_1, 75.522_487_814_070_04];
    for (far, limit) in [(start + 180.0, limits[0]), (start - 180.0, limits[1])] {
        let answer = run(&mut s, "view.drive", json!({"mate": "Hip", "value": far}));
        let reached = answer["reached"].as_f64().unwrap();
        let why = answer["stopped"].as_str().unwrap_or_default();
        assert!(why.contains("stopped at"), "{answer}");
        assert!((reached - limit).abs() < 0.01, "{answer}");
        assert!((shown(&mut s, "Hip") - reached).abs() < 1e-6);
    }
    // The pose is a function of the value alone, whatever came before.
    run(
        &mut s,
        "view.drive",
        json!({"mate": "Hip", "value": start + 30.0}),
    );
    let after_far = shown(&mut s, "Drive");
    run(&mut s, "view.drive", json!({"mate": null}));
    run(
        &mut s,
        "view.drive",
        json!({"mate": "Hip", "value": start + 30.0}),
    );
    assert_eq!(shown(&mut s, "Drive").to_bits(), after_far.to_bits());
    assert_eq!(document_state(&mut s), before);
}

/// A mate held by nothing but its mates (fixed) cannot be held, and
/// hostile parameters are refused with a message, never a panic: the
/// view stays as it was.
#[test]
fn view_drive_refuses_what_it_cannot_hold() {
    let mut s = planetary();
    for params in [
        json!({"mate": "RingHeld", "value": 10}),
        json!({"mate": "Nope", "value": 10}),
        json!({"mate": "Output"}),
        json!({"mate": "Output", "value": "5 mm"}),
        json!({"mate": "Output", "value": "nonsense +"}),
        json!({"mate": "Output", "value": 1e300}),
        json!({"mate": "Output", "value": [1, 2]}),
        json!({"mate": "Output", "value": true}),
        json!({"mate": "Output", "value": 10, "which": "distance"}),
        json!({"mate": 7, "value": 10}),
        json!({"value": 10}),
        json!({"mate": "Output", "value": 10, "extra": 1}),
    ] {
        refused(&mut s, "view.drive", params);
    }
    assert_eq!(s.driven_mate(), None);
    // A part is not an assembly.
    let dir = workdir("view-drive", "part");
    let mut part = session_in(&dir);
    run(&mut part, "file.new", json!({}));
    refused(&mut part, "view.drive", json!({"mate": "M", "value": 1}));
}

/// The page (the browser's host) never places an assembly: a joint held
/// by hand there is held where the view is made (the worker), pending
/// until its facts came; then the scene, the outline and the answer are
/// those of a session that holds it itself, bit for bit, and the page
/// placed nothing. Letting go is made there too.
#[test]
fn a_joint_held_by_hand_on_the_page_is_held_beside_it_bit_for_bit() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/examples/published/planetary-gearbox.3dcraftasm"
    ))
    .unwrap();
    let mut page = Session::new(Workspace::memory());
    page.set_stepwise(true);
    page.set_checks_elsewhere(true);
    page.set_regen_elsewhere(true);
    page.set_commands_elsewhere(true);
    page.open_bytes("planetary-gearbox.3dcraftasm", &bytes)
        .unwrap();
    let mut here = planetary();
    let quality = SceneQuality::Preview;
    let answer_scenes = |page: &mut Session| {
        let mut jobs = 0;
        while let Some(token) = page.scene_job(quality) {
            jobs += 1;
            assert!(jobs < 100);
            let request = page.scene_request(token).unwrap();
            scene_begin(&request.document, &request.work);
            while let Some(part) = scene_next() {
                page.scene_answered(token, Ok(&part));
            }
            page.scene_done(token);
        }
    };
    answer_scenes(&mut page);
    let before = page.work_here();
    let asked = run(
        &mut page,
        "view.drive",
        json!({"mate": "Output", "value": 30}),
    );
    assert_eq!(asked["pending"], true, "{asked}");
    assert_eq!(page.driven_mate().as_deref(), Some("Output"));
    answer_scenes(&mut page);
    let came = run(&mut page, "view.drive", json!({}));
    let mine = run(
        &mut here,
        "view.drive",
        json!({"mate": "Output", "value": 30}),
    );
    let strip = |mut v: Value| {
        v.as_object_mut().unwrap().remove("elsewhere");
        v
    };
    assert_eq!(strip(came), mine);
    assert_eq!(
        scene_bytes(&page.scene_with(quality).unwrap().unwrap()),
        scene_bytes(&here.scene_with(quality).unwrap().unwrap())
    );
    assert_eq!(page.assembly_outline(), here.assembly_outline());
    run(&mut page, "view.drive", json!({"mate": null}));
    run(&mut here, "view.drive", json!({"mate": null}));
    answer_scenes(&mut page);
    assert_eq!(page.driven_mate(), None);
    assert_eq!(page.assembly_outline(), here.assembly_outline());
    assert_eq!(page.work_here(), before);
}
