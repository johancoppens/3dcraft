//! The window over a session that regenerates elsewhere (the page's Web
//! Worker, `Session::set_regen_elsewhere`): a frame reads what the state
//! shown holds (the selection, a section on a datum plane) without
//! regenerating that state on the page's thread, whatever the window held
//! for the tab it showed before.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SceneQuality, Session, Workspace, scene_begin, scene_next};
use craft3d_ui_egui::Shell;
use egui::{Pos2, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// A host that regenerates elsewhere.
fn elsewhere() -> Session {
    let mut session = Session::new(Workspace::none());
    session.set_stepwise(true);
    session.set_checks_elsewhere(true);
    session.set_regen_elsewhere(true);
    assert!(session.regen_elsewhere());
    session
}

/// Answers the scenes the window waits for, as the worker does.
fn answer(session: &mut Session, quality: SceneQuality) {
    for _ in 0..10 {
        let Some(token) = session.scene_job(quality) else {
            assert!(session.scene_ready(quality));
            return;
        };
        let request = session.scene_request(token).unwrap();
        scene_begin(&request.document, &request.work);
        while let Some(part) = scene_next() {
            assert!(session.scene_answered(token, Ok(&part)));
        }
        assert!(session.scene_done(token));
    }
    panic!("jobs without end");
}

/// A frame of the window, the scene of the state shown there.
fn frame(ctx: &egui::Context, shell: &mut Shell, session: &mut Session, time: f64) {
    answer(session, shell.scene_quality());
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
        time: Some(time),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
    output.textures_delta.clear();
}

/// The review's case (2026-10-28): a face selected in one document, a
/// second opened beside it; the frame that shows the second read its
/// selection, which regenerated the whole document on the page's thread
/// (a heavy Jig: an unstoppable wait). And a section on a datum plane of
/// the second, which regenerated it to find the plane.
#[test]
fn a_frame_never_regenerates_the_state_it_shows() {
    let ctx = egui::Context::default();
    let mut shell = Shell::new();
    let mut session = elsewhere();
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [10, 20, 5], "name": "A"}),
    );
    frame(&ctx, &mut shell, &mut session, 0.0);
    // A click selects: a command, which regenerates here.
    run(&mut session, "select.set", json!({"names": ["A"]}));
    assert!(session.regenerated_here());
    frame(&ctx, &mut shell, &mut session, 0.1);
    assert_eq!(shell.selection().items.len(), 1);

    // A second document, opened beside it.
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [30, 30, 30], "name": "C"}),
    );
    run(
        &mut session,
        "variable.set",
        json!({"name": "d", "expr": "11 mm"}),
    );
    run(
        &mut session,
        "variable.publish",
        json!({"name": "d", "min": "1 mm", "max": "100 mm"}),
    );
    run(
        &mut session,
        "datum.plane",
        json!({"from": "xy", "distance": "d", "name": "Top"}),
    );
    // Its last edit left to the host, as the Parameters form leaves it
    // (a command that adds a feature may regenerate to answer).
    run(&mut session, "param.set", json!({"values": {"d": "12 mm"}}));
    assert!(!session.regenerated_here());
    for k in 0..3 {
        frame(&ctx, &mut shell, &mut session, 1.0 + f64::from(k));
        assert!(
            !session.regenerated_here(),
            "frame {k} regenerated the document it shows"
        );
    }
    assert!(shell.selection().items.is_empty());

    // A section on its datum plane.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": true, "plane": "Top", "offset": 0}}),
    );
    frame(&ctx, &mut shell, &mut session, 5.0);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["point"], json!([0.0, 0.0, 12.0]), "{got}");
    assert_eq!(got["section"]["normal"], json!([0.0, 0.0, 1.0]));
    // An edit, as the Parameters form makes it: a new state, the
    // section found on it again.
    let done = run(&mut session, "param.set", json!({"values": {"d": "14 mm"}}));
    assert_eq!(done["deferred"], true, "{done}");
    frame(&ctx, &mut shell, &mut session, 6.0);
    frame(&ctx, &mut shell, &mut session, 7.0);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["point"], json!([0.0, 0.0, 14.0]), "{got}");
    assert!(
        !session.regenerated_here(),
        "the section regenerated the document it cuts"
    );
}
