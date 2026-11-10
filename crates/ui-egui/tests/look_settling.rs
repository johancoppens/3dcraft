//! What a picture of the window waits for besides the viewport's
//! averaging (`Shell::look_settling`): the studio look's environment
//! prepared (`Shell::prepare_look`; the plain look is drawn meanwhile) and
//! a popover done fading in (`Shell::popover_fading`: egui fades an area in
//! over its animation time once it becomes visible). Driven by frames at
//! chosen times, so it does not depend on the clock.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use egui::{Pos2, vec2};
use serde_json::{Value, json};

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap()
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

fn frame(ctx: &egui::Context, shell: &mut Shell, session: &mut Session, time: f64) {
    frame_sized(ctx, shell, session, time, vec2(1280.0, 800.0));
}

fn frame_sized(
    ctx: &egui::Context,
    shell: &mut Shell,
    session: &mut Session,
    time: f64,
    size: egui::Vec2,
) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, size)),
        time: Some(time),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
    output.textures_delta.clear();
}

#[test]
fn a_picture_waits_for_the_look_and_the_popover_to_settle() {
    let mut session = Session::new(Workspace::none());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    session
        .execute("feature.block", &json!({"size": [20, 20, 10]}))
        .unwrap();
    let mut shell = Shell::new();
    let ctx = egui::Context::default();
    frame(&ctx, &mut shell, &mut session, 1.0);
    // The plain look: nothing on its way, no popover.
    assert!(!shell.look_settling(&ctx, 1.0));
    assert!(!shell.popover_fading(&ctx, 1.0));
    // The studio look switched on opens its popover, which becomes
    // visible in the next frame and fades in from there.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"look": {"mode": "studio"}}),
    );
    shell.prepare_look().unwrap();
    // A new area's first pass may only size it (drawn invisibly): two
    // passes at the same moment, as a picture of the window runs.
    frame(&ctx, &mut shell, &mut session, 2.0);
    frame(&ctx, &mut shell, &mut session, 2.0);
    assert!(!shell.look_hits().is_empty(), "the popover is drawn");
    let fade = f64::from(ctx.global_style().animation_time);
    assert!(fade > 0.0);
    assert!(shell.popover_fading(&ctx, 2.0));
    assert!(shell.look_settling(&ctx, 2.0 + fade * 0.5));
    // Faded in: settled, the environment prepared.
    assert!(!shell.popover_fading(&ctx, 2.0 + fade));
    assert!(!shell.look_settling(&ctx, 2.0 + fade));
    // Frames later it stays settled; closed and opened again it fades in
    // again from that frame.
    frame(&ctx, &mut shell, &mut session, 3.0);
    assert!(!shell.look_settling(&ctx, 3.0));
    let caret = shell.look_caret().unwrap().center();
    for (t, events) in [
        (3.1, vec![egui::Event::PointerMoved(caret)]),
        (
            3.2,
            vec![egui::Event::PointerButton {
                pos: caret,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        ),
        (
            3.3,
            vec![egui::Event::PointerButton {
                pos: caret,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ),
        (3.4, vec![egui::Event::PointerGone]),
    ] {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(t),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| shell.ui(ui, &mut session));
        output.textures_delta.clear();
    }
    assert!(!shell.look_panel_open(), "the caret closed it");
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"panels": {"lookOptions": true}}),
    );
    frame(&ctx, &mut shell, &mut session, 5.0);
    frame(&ctx, &mut shell, &mut session, 5.0);
    assert!(shell.popover_fading(&ctx, 5.0));
    assert!(!shell.popover_fading(&ctx, 5.0 + fade));
}

/// The narrow layout's drawer slides in and out: a picture waits until it
/// is in place.
#[test]
fn a_picture_waits_for_the_drawer_to_slide() {
    let mut session = Session::new(Workspace::none());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    session
        .execute("feature.block", &json!({"size": [20, 20, 10]}))
        .unwrap();
    let mut shell = Shell::new();
    let ctx = egui::Context::default();
    let phone = vec2(390.0, 844.0);
    frame_sized(&ctx, &mut shell, &mut session, 1.0, phone);
    frame_sized(&ctx, &mut shell, &mut session, 1.1, phone);
    assert!(!shell.animating(&ctx, 1.1));
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"panels": {"drawer": true}}),
    );
    frame_sized(&ctx, &mut shell, &mut session, 1.2, phone);
    frame_sized(&ctx, &mut shell, &mut session, 1.21, phone);
    assert!(shell.animating(&ctx, 1.21), "sliding in");
    frame_sized(&ctx, &mut shell, &mut session, 2.0, phone);
    frame_sized(&ctx, &mut shell, &mut session, 2.1, phone);
    assert!(!shell.animating(&ctx, 2.1), "in place");
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"panels": {"drawer": false}}),
    );
    frame_sized(&ctx, &mut shell, &mut session, 3.0, phone);
    frame_sized(&ctx, &mut shell, &mut session, 3.01, phone);
    assert!(shell.animating(&ctx, 3.01), "sliding out");
    frame_sized(&ctx, &mut shell, &mut session, 4.0, phone);
    frame_sized(&ctx, &mut shell, &mut session, 4.1, phone);
    assert!(!shell.animating(&ctx, 4.1), "gone");
}

/// The agents popover over the pill and the dialogs fade in as the
/// popovers do: a picture waits for them (they were taken half faded,
/// 2026-11-04).
#[test]
fn a_picture_waits_for_the_agents_popover_to_fade_in() {
    let mut session = Session::new(Workspace::none());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    let mut shell = Shell::new();
    let ctx = egui::Context::default();
    frame(&ctx, &mut shell, &mut session, 1.0);
    assert!(!shell.popover_fading(&ctx, 1.0));
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"dialogs": {"agents": true}}),
    );
    frame(&ctx, &mut shell, &mut session, 2.0);
    frame(&ctx, &mut shell, &mut session, 2.0);
    let fade = f64::from(ctx.global_style().animation_time);
    assert!(shell.popover_fading(&ctx, 2.0), "fading in");
    assert!(!shell.popover_fading(&ctx, 2.0 + fade), "faded in");
    // A dialog (About) likewise.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"dialogs": {"agents": false, "about": true}}),
    );
    frame(&ctx, &mut shell, &mut session, 4.0);
    frame(&ctx, &mut shell, &mut session, 4.0);
    assert!(shell.popover_fading(&ctx, 4.0), "the dialog fading in");
    assert!(!shell.popover_fading(&ctx, 4.0 + fade));
}

/// Any area that fades in is waited for, one no list knew included (the
/// review of e6c34e45): a dialog added later cannot be forgotten.
#[test]
fn a_picture_waits_for_any_area_fading_in() {
    let mut session = Session::new(Workspace::none());
    session
        .execute("file.new", &json!({"units": "mm"}))
        .unwrap();
    let mut shell = Shell::new();
    let ctx = egui::Context::default();
    let mut show_new = false;
    let pass = |shell: &mut Shell, session: &mut Session, time: f64, show_new: bool| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(time),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            shell.ui(ui, session);
            if show_new {
                egui::Area::new(egui::Id::new("a-dialog-nobody-listed"))
                    .show(ui.ctx(), |ui| ui.label("new"));
            }
        });
        output.textures_delta.clear();
    };
    pass(&mut shell, &mut session, 1.0, show_new);
    pass(&mut shell, &mut session, 1.5, show_new);
    assert!(!shell.popover_fading(&ctx, 1.5));
    show_new = true;
    pass(&mut shell, &mut session, 2.0, show_new);
    pass(&mut shell, &mut session, 2.0, show_new);
    let fade = f64::from(ctx.global_style().animation_time);
    assert!(shell.popover_fading(&ctx, 2.0), "the new area fades in");
    assert!(!shell.popover_fading(&ctx, 2.0 + fade));
}
