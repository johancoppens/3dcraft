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

fn example(name: &str) -> Session {
    let path = format!(
        "{}/../../docs/examples/published/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(path).unwrap();
    let mut s = Session::new(Workspace::memory());
    s.open_bytes(name, &bytes).unwrap();
    s
}

/// Seconds `f` takes.
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = std::time::Instant::now();
    let out = f();
    (out, t.elapsed().as_secs_f64())
}

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

/// A chain of couplings held many turns round: the reducer's output
/// (`Shaft3`) at 720° turns the input some ten times as far, and the
/// solve keeps the turns each joint had come (carried with the poses):
/// it once failed past 560° ("did not converge", residual ±16) and the
/// view fell back to the document's pose. The gears follow by the ratio
/// at every whole turn, and the planetary's carrier holds at 3000°.
#[test]
fn a_coupling_chain_holds_many_turns_round() {
    let mut s = example("gear-reducer.3dcraftasm");
    let (answer, first) = timed(|| {
        run(
            &mut s,
            "view.drive",
            json!({"mate": "Shaft3", "value": 720}),
        )
    });
    assert_eq!(answer["stopped"], Value::Null, "{answer}");
    assert!((answer["reached"].as_f64().unwrap() - 720.0).abs() < 1e-9);
    assert!((shown(&mut s, "Shaft3") - 720.0).abs() < 1e-6);
    let ratio = (63.0 / 18.0) * (49.0 / 17.0);
    let input = shown(&mut s, "Shaft1");
    assert!((input.abs() - 720.0 * ratio).abs() < 1e-3, "{input}");
    let (_, warm) = timed(|| {
        run(
            &mut s,
            "view.drive",
            json!({"mate": "Shaft3", "value": 722}),
        )
    });
    eprintln!("reducer Shaft3 720°: {first:.3} s cold, 722° then {warm:.3} s");
    let mut p = planetary();
    let answer = run(
        &mut p,
        "view.drive",
        json!({"mate": "Output", "value": 3000}),
    );
    assert_eq!(answer["stopped"], Value::Null, "{answer}");
    let sun = shown(&mut p, "SunOnCarrier");
    assert!((sun.abs() - 4.0 * 3000.0).abs() < 1e-3, "{sun}");
}

/// Past a dead point the limit found is kept: a later value beyond it is
/// shown there without a solve (the first one searched for it).
#[test]
fn a_value_past_a_known_dead_point_costs_no_solve() {
    let dir = workdir("view-drive", "four-bar-limit");
    let mut s = session_in(&dir);
    four_bar(&mut s);
    let start = shown(&mut s, "Hip");
    let (first, cold) = timed(|| {
        run(
            &mut s,
            "view.drive",
            json!({"mate": "Hip", "value": start + 100.0}),
        )
    });
    assert!(first["stopped"].is_string(), "{first}");
    let mut later = Vec::new();
    for k in 1..=5 {
        let (answer, t) = timed(|| {
            run(
                &mut s,
                "view.drive",
                json!({"mate": "Hip", "value": start + 100.0 + f64::from(k)}),
            )
        });
        assert_eq!(answer["reached"], first["reached"], "{answer}");
        assert!(answer["stopped"].is_string());
        later.push(t);
    }
    let worst = later.iter().copied().fold(0.0, f64::max);
    eprintln!(
        "four-bar past its dead point: {cold:.3} s the first time, at most {worst:.4} s after"
    );
    assert!(worst < cold / 5.0, "{cold} {later:?}");
}

/// A value more than ten turns from the joint's is refused at once, and
/// the view stays as it was.
#[test]
fn a_value_too_far_round_is_refused_at_once() {
    let mut s = planetary();
    run(&mut s, "view.drive", json!({"mate": "Output", "value": 10}));
    let (why, t) = timed(|| {
        refused(
            &mut s,
            "view.drive",
            json!({"mate": "Output", "value": 4000}),
        )
    });
    assert!(why.contains("at most 720 steps"), "{why}");
    assert!(t < 1.0, "{t}");
    assert!((shown(&mut s, "Output") - 10.0).abs() < 1e-6);
}

/// An undo and a redo bring back the document state the joint was held
/// on, and still let go of it.
#[test]
fn an_undo_and_a_redo_let_go_of_the_joint_held_by_hand() {
    let mut s = planetary();
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": "11 mm"}),
    );
    run(&mut s, "view.drive", json!({"mate": "Output", "value": 20}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.driven_mate(), None);
    assert!(shown(&mut s, "Output").abs() < 1e-6);
}

fn page() -> Session {
    let path = format!(
        "{}/../../docs/examples/published/planetary-gearbox.3dcraftasm",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(path).unwrap();
    let mut page = Session::new(Workspace::memory());
    page.set_stepwise(true);
    page.set_checks_elsewhere(true);
    page.set_regen_elsewhere(true);
    page.set_commands_elsewhere(true);
    page.open_bytes("planetary-gearbox.3dcraftasm", &bytes)
        .unwrap();
    page
}

fn answer_page_scenes(page: &mut Session) {
    let quality = SceneQuality::Preview;
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
}

/// A joint the worker cannot hold (too far round) is not left pending on
/// the page: the facts say why, the page lets go of it and says so once.
#[test]
fn a_joint_the_worker_cannot_hold_is_let_go_of_on_the_page() {
    let mut page = page();
    answer_page_scenes(&mut page);
    let asked = run(
        &mut page,
        "view.drive",
        json!({"mate": "Output", "value": 5000}),
    );
    assert_eq!(asked["pending"], true, "{asked}");
    answer_page_scenes(&mut page);
    assert_eq!(page.driven_mate(), None);
    let said = run(&mut page, "view.drive", json!({}));
    assert_eq!(said["mate"], Value::Null, "{said}");
    assert!(
        said["error"].as_str().unwrap().contains("too far"),
        "{said}"
    );
    let again = run(&mut page, "view.drive", json!({}));
    assert!(again.get("error").is_none(), "{again}");
}

/// In the worker the next value of the same joint goes on from the
/// solves it keeps, not from the document's pose each time.
#[test]
fn the_worker_keeps_its_solves_from_value_to_value() {
    let mut page = page();
    answer_page_scenes(&mut page);
    run(
        &mut page,
        "view.drive",
        json!({"mate": "Output", "value": 720}),
    );
    let (_, cold) = timed(|| answer_page_scenes(&mut page));
    run(
        &mut page,
        "view.drive",
        json!({"mate": "Output", "value": 725}),
    );
    let (_, warm) = timed(|| answer_page_scenes(&mut page));
    let came = run(&mut page, "view.drive", json!({}));
    assert!(
        (came["reached"].as_f64().unwrap() - 725.0).abs() < 1e-9,
        "{came}"
    );
    eprintln!("worker, planetary Output 720°: {cold:.3} s, then 725°: {warm:.3} s");
    assert!(warm < cold * 0.6, "{cold} {warm}");
}

/// What the view shows of a held joint, whole: `view.drive`'s answer,
/// the outline and the scene's bytes.
fn held_view(s: &mut Session) -> (Value, String, Vec<u8>) {
    let answer = run(s, "view.drive", json!({}));
    let outline = format!("{:?}", s.assembly_outline());
    let scene = scene_bytes(&s.scene_with(SceneQuality::Preview).unwrap().unwrap());
    (answer, outline, scene)
}

/// A pose is a function of the value alone, whatever was asked before:
/// the reducer's Shaft3 at 400° asked at once, after 720°, and after a
/// detour (−100°, 50°, 610°, 133°) shows the same answer, outline and
/// scene, bit for bit.
#[test]
fn a_held_joint_s_pose_does_not_depend_on_the_way_there() {
    let routes: [&[f64]; 3] = [
        &[400.0],
        &[720.0, 400.0],
        &[-100.0, 50.0, 610.0, 133.0, 400.0],
    ];
    let mut views = Vec::new();
    for route in routes {
        let mut s = example("gear-reducer.3dcraftasm");
        for v in route {
            run(&mut s, "view.drive", json!({"mate": "Shaft3", "value": v}));
        }
        views.push(held_view(&mut s));
    }
    assert!((views[0].0["reached"].as_f64().unwrap() - 400.0).abs() < 1e-9);
    for view in &views[1..] {
        assert_eq!(view.0, views[0].0);
        assert_eq!(view.1, views[0].1);
        assert!(view.2 == views[0].2, "the scenes differ");
    }
}

/// Where a joint stops does not depend on the first value that found it:
/// the four-bar's rocker asked at once far past its dead point, or first
/// just past it and then far past it, or from the other side of it first,
/// stops at the same value with the same pose and reason, bit for bit.
#[test]
fn a_dead_point_does_not_depend_on_the_value_that_found_it() {
    let dir = workdir("view-drive", "four-bar-paths");
    let mut views = Vec::new();
    for route in [
        &[100.0][..],
        &[57.5, 100.0],
        &[20.0, 56.0, 100.0],
        &[-30.0, 100.0],
    ] {
        let mut s = session_in(&dir);
        four_bar(&mut s);
        let start = shown(&mut s, "Hip");
        for d in route {
            run(
                &mut s,
                "view.drive",
                json!({"mate": "Hip", "value": start + d}),
            );
        }
        views.push(held_view(&mut s));
    }
    assert!(views[0].0["stopped"].is_string(), "{}", views[0].0);
    for view in &views[1..] {
        assert_eq!(
            view.0["reached"].as_f64().map(f64::to_bits),
            views[0].0["reached"].as_f64().map(f64::to_bits),
            "{} {}",
            view.0,
            views[0].0
        );
        assert_eq!(view.0, views[0].0);
        assert_eq!(view.1, views[0].1);
        assert!(view.2 == views[0].2, "the scenes differ");
    }
}

/// The same mate asked again, at a value the worker can hold, before the
/// page took in the refusal of the one before: held, not refused with
/// the older reason. A refusal is not said after an edit.
#[test]
fn a_refusal_on_the_page_is_of_its_own_request_only() {
    let mut page = page();
    answer_page_scenes(&mut page);
    run(
        &mut page,
        "view.drive",
        json!({"mate": "Output", "value": 5000}),
    );
    answer_page_scenes(&mut page);
    let asked = run(
        &mut page,
        "view.drive",
        json!({"mate": "Output", "value": 10}),
    );
    assert_eq!(asked["pending"], true, "{asked}");
    assert_eq!(page.driven_mate().as_deref(), Some("Output"));
    answer_page_scenes(&mut page);
    let came = run(&mut page, "view.drive", json!({}));
    assert!(
        (came["reached"].as_f64().unwrap() - 10.0).abs() < 1e-9,
        "{came}"
    );
    // Refused, then an edit: nothing is said of it any more.
    run(
        &mut page,
        "view.drive",
        json!({"mate": "Output", "value": 5000}),
    );
    answer_page_scenes(&mut page);
    assert_eq!(page.driven_mate(), None);
    assert!(
        page.take_drive_refusal().is_some(),
        "said to the person once"
    );
    assert!(page.take_drive_refusal().is_none());
    run(
        &mut page,
        "variable.set",
        json!({"name": "w", "expr": "11 mm"}),
    );
    let said = run(&mut page, "view.drive", json!({}));
    assert!(said.get("error").is_none(), "{said}");
}

/// A part is held to the world by a fixed mate that holds: one whose
/// face is gone (it failed) holds nothing.
#[test]
fn a_failed_fixed_mate_grounds_nothing() {
    let dir = workdir("view-drive", "grounded");
    let mut s = session_in(&dir);
    four_bar(&mut s);
    run(&mut s, "file.activate", json!({"document": 0}));
    run(
        &mut s,
        "datum.axis",
        json!({ "name": "Mid", "through": [[8, 2, 0], [8, 2, 1]] }),
    );
    run(&mut s, "file.save", json!({"path": "bar.3dcraft"}));
    run(&mut s, "file.activate", json!({"document": 1}));
    run(&mut s, "assembly.reload", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "bar.3dcraft", "name": "Tag", "body": "Bar" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "fixed", "a": "Ground/End1", "b": "Tag/Mid", "name": "Glue", "flip": true }),
    );
    let grounded = |s: &mut Session, name: &str| {
        let outline = s.assembly_outline().unwrap();
        outline
            .instances
            .iter()
            .find(|i| i.name == name)
            .unwrap()
            .grounded
    };
    assert!(grounded(&mut s, "Ground") && grounded(&mut s, "Tag"));
    assert!(!grounded(&mut s, "Crank"));
    run(&mut s, "file.activate", json!({"document": 0}));
    run(&mut s, "feature.delete", json!({"feature": "Mid"}));
    run(&mut s, "file.save", json!({"path": "bar.3dcraft"}));
    run(&mut s, "file.activate", json!({"document": 1}));
    run(&mut s, "assembly.reload", json!({}));
    let outline = s.assembly_outline().unwrap();
    let glue = outline.mates.iter().find(|m| m.name == "Glue").unwrap();
    assert_eq!(glue.status, "failed", "{glue:?}");
    assert!(grounded(&mut s, "Ground"));
    assert!(!grounded(&mut s, "Tag"), "held by a mate that failed");
}
