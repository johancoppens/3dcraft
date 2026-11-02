//! The window draws lighter while a motion plays (docs/design/motion.md ›
//! *The player*): the studio look without its accumulated frames and its
//! depth of field, only while the motion plays and `timeline.playQuality`
//! is auto, never for a picture of the window or a recording, and the full
//! look again, settling, once it stops.
//!
//! The direct path's pictures land in `target/evidence/ui/` as
//! `play-light-*.png`. The test of the direct path needs a graphics
//! adapter and fails, with what to install, when there is none (the GPU
//! crate's policy).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_gpu::ADAPTER_HINT;
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::backend::RendererChoice;
use craft3d_ui_egui::direct::headless_render_state;
use egui::{Pos2, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap()
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// A wall and a crank whose block goes round a circle of 15 mm.
fn crank() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(&mut s, "variable.set", json!({"name": "t", "expr": "0"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Wall", "size": ["4 mm", "30 mm", "6 mm"], "position": ["-24 mm", "-15 mm", "0 mm"]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Crank", "size": ["4 mm", "4 mm", "4 mm"],
               "position": ["15 mm * cos(360 deg * t) - 2 mm", "15 mm * sin(360 deg * t) - 2 mm", "0 mm"]}),
    );
    s
}

/// The studio look with a lens that blurs and frames averaged.
fn studio_with_lens(shell: &mut Shell, s: &mut Session) {
    call(
        shell,
        s,
        "ui.set",
        json!({"look": {"mode": "studio", "accumulate": 4,
                        "dof": {"fstop": 1.4, "focus": [-22.0, 0.0, 3.0]}}}),
    );
    shell.prepare_look().unwrap();
}

/// Frames of a window whose clock moves `dt` seconds a frame: with 0 a
/// playing motion stays at its moment.
fn frame(ctx: &egui::Context, time: &mut f64, dt: f64, shell: &mut Shell, s: &mut Session) {
    *time += dt;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 560.0))),
        time: Some(*time),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| shell.ui(ui, s));
    output.textures_delta.clear();
}

fn timeline(shell: &mut Shell, s: &mut Session) -> Value {
    call(shell, s, "ui.get", json!({}))["timeline"].clone()
}

/// Whether the look the viewport draws with is the light one: one frame,
/// no lens.
fn draws_light(shell: &Shell) -> bool {
    let studio = shell.studio().expect("the studio look, prepared");
    let look = studio.look;
    let light = look.accumulate == 1 && look.dof.is_none() && !look.ao && !look.ssr;
    // The key's shadows and the ground stay.
    assert!(look.shadows && look.ground, "{look:?}");
    light
}

#[test]
fn the_window_draws_lighter_only_while_a_motion_plays() {
    let mut s = crank();
    let mut shell = Shell::new();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    frame(&ctx, &mut time, 0.0, &mut shell, &mut s);
    studio_with_lens(&mut shell, &mut s);
    frame(&ctx, &mut time, 0.0, &mut shell, &mut s);
    // Standing still: the full look.
    assert_eq!(timeline(&mut shell, &mut s)["playingLight"], false);
    assert_eq!(timeline(&mut shell, &mut s)["playQuality"], "auto");
    assert!(!draws_light(&shell));
    assert_eq!(shell.studio().unwrap().look.accumulate, 4);

    // The motion playing: light.
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"timeline": {"motion": true, "playing": true, "loop": true}}),
    );
    for _ in 0..5 {
        frame(&ctx, &mut time, 0.05, &mut shell, &mut s);
    }
    assert_eq!(timeline(&mut shell, &mut s)["playingLight"], true);
    assert!(draws_light(&shell));

    // A picture of the window, a frame-exact recording, a recording in
    // real time: the full look while they last.
    shell.set_capturing(true);
    assert!(!shell.playing_light());
    assert!(!draws_light(&shell));
    shell.set_capturing(false);
    shell.set_frame_exact(true);
    assert!(!draws_light(&shell));
    shell.set_frame_exact(false);
    shell.set_recording(true);
    assert!(!draws_light(&shell));
    assert_eq!(timeline(&mut shell, &mut s)["playingLight"], false);
    shell.set_recording(false);
    assert!(draws_light(&shell));

    // playQuality full: the full look while it plays.
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"timeline": {"playQuality": "full"}}),
    );
    assert!(!draws_light(&shell));
    assert_eq!(timeline(&mut shell, &mut s)["playingLight"], false);
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"timeline": {"playQuality": "auto"}}),
    );
    assert!(draws_light(&shell));
    let refused = shell
        .handle_method(
            &mut s,
            "ui.set",
            Some(&json!({"timeline": {"playQuality": "fast"}})),
        )
        .unwrap();
    assert!(refused.is_err(), "{refused:?}");

    // ui.screenshot while it plays is the picture of that moment standing
    // still, pixel for pixel.
    let playing = call(
        &mut shell,
        &mut s,
        "ui.screenshot",
        json!({"width": 240, "height": 180, "scale": 1.0}),
    );
    let t = timeline(&mut shell, &mut s)["t"].as_f64().unwrap();
    assert!(t > 0.0, "the motion moved: t = {t}");
    let mut still = crank();
    let mut still_shell = Shell::new();
    let still_ctx = egui::Context::default();
    let mut still_time = 0.0;
    frame(
        &still_ctx,
        &mut still_time,
        0.0,
        &mut still_shell,
        &mut still,
    );
    studio_with_lens(&mut still_shell, &mut still);
    run(&mut still, "view.motion", json!({"t": t}));
    frame(
        &still_ctx,
        &mut still_time,
        0.0,
        &mut still_shell,
        &mut still,
    );
    let standing = call(
        &mut still_shell,
        &mut still,
        "ui.screenshot",
        json!({"width": 240, "height": 180, "scale": 1.0}),
    );
    assert!(
        playing["base64"] == standing["base64"],
        "the screenshot taken while it plays differs from the still one"
    );

    // Paused: the full look at once.
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"timeline": {"playing": false}}),
    );
    assert!(!draws_light(&shell));
    assert_eq!(timeline(&mut shell, &mut s)["playingLight"], false);
}

fn evidence(name: &str, width: u32, height: u32, rgb: &[u8]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("play-light-{name}.png"));
    let png = craft3d_render::encode_png(width, height, rgb).unwrap();
    std::fs::write(&path, png).unwrap();
    println!("evidence: {}", path.display());
}

/// The picture the direct path shows, once it has settled (its frames
/// averaged): at most `frames` frames.
fn settled_picture(
    ctx: &egui::Context,
    time: &mut f64,
    shell: &mut Shell,
    s: &mut Session,
    name: &str,
) -> (usize, Vec<u8>) {
    let mut drawn = 0;
    for _ in 0..64 {
        frame(ctx, time, 0.0, shell, s);
        drawn += 1;
        if !shell.converging() {
            break;
        }
    }
    assert!(!shell.converging(), "{name}: still averaging");
    let (w, h, rgb) = shell.direct_picture().unwrap().unwrap();
    evidence(name, w, h, &rgb);
    (drawn, rgb)
}

/// On the window's own device: while the motion plays one frame is drawn
/// per moment (nothing averaged, no lens); recorded frame by frame it is
/// the full picture, pixel for pixel the one of the motion standing still
/// at that moment; paused, the full look comes back and settles.
#[test]
fn on_the_direct_path_a_playing_motion_is_one_frame_and_a_recorded_one_full() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut s = crank();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.attach_gpu(&state).unwrap();
    assert!(shell.draws_directly());
    let ctx = egui::Context::default();
    let mut time = 0.0;
    frame(&ctx, &mut time, 0.0, &mut shell, &mut s);
    studio_with_lens(&mut shell, &mut s);
    run(&mut s, "view.motion", json!({"t": 0.25}));
    // The motion plays, its clock standing still (frames 0 s apart): it
    // stays at its moment.
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"timeline": {"motion": true, "playing": true, "loop": true}}),
    );
    frame(&ctx, &mut time, 0.0, &mut shell, &mut s);
    frame(&ctx, &mut time, 0.0, &mut shell, &mut s);
    assert!(shell.playing_light());
    let t = timeline(&mut shell, &mut s)["t"].as_f64().unwrap();
    // One frame and it is done: nothing is averaged while it plays.
    let (drawn, light) = settled_picture(&ctx, &mut time, &mut shell, &mut s, "playing");
    assert_eq!(drawn, 1, "a playing motion averages no frames");

    // Paused: the full look at once, settling over its frames.
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"timeline": {"playing": false}}),
    );
    frame(&ctx, &mut time, 0.0, &mut shell, &mut s);
    assert!(shell.converging(), "the settle starts at once");
    let (_, full) = settled_picture(&ctx, &mut time, &mut shell, &mut s, "paused");
    assert!(
        light != full,
        "the light picture is the full one: nothing was lighter"
    );

    // Playing again: light again, at once.
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"timeline": {"playing": true}}),
    );
    let (drawn, again) = settled_picture(&ctx, &mut time, &mut shell, &mut s, "playing-again");
    assert_eq!(drawn, 1);
    assert!(again == light, "light again");
    assert_eq!(timeline(&mut shell, &mut s)["t"].as_f64(), Some(t));

    // Recorded frame by frame while it plays: the full picture, pixel for
    // pixel the one of the motion standing still.
    shell.set_frame_exact(true);
    let (drawn, recorded) = settled_picture(&ctx, &mut time, &mut shell, &mut s, "recorded");
    assert!(
        drawn >= 4,
        "the recorded picture averages its frames: {drawn}"
    );
    shell.set_frame_exact(false);
    assert_eq!(timeline(&mut shell, &mut s)["t"].as_f64(), Some(t));
    assert!(
        recorded == full,
        "the recorded frame is not the full picture"
    );
}
