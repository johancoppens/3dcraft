//! The studio look's lens focused on the selection (`look.dof.focus`
//! `selection`): it focuses on the centre of the box of what is selected,
//! in the viewport or the feature list, follows the selection, falls back
//! to the model's centre (and says so) with nothing selected; `ui.get`'s
//! `lens` says where it focuses; and the picture is sharp where it
//! focuses and blurred elsewhere. Bad lens values are refused or clamped,
//! never a panic.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::state::LookState;
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
        .unwrap()
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

fn refused(shell: &mut Shell, session: &mut Session, params: Value) -> String {
    shell
        .handle_method(session, "ui.set", Some(&params))
        .unwrap()
        .unwrap_err()
}

/// One frame of the shell, as the window draws it.
fn frame(ctx: &egui::Context, shell: &mut Shell, session: &mut Session) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
    output.textures_delta.clear();
}

/// Two cubes of side 10 seen along -x from (150, 0, 0): Near, centred at
/// (45, -20, 0), 105 ahead of the eye on the left of the picture; Far,
/// centred at (-55, 20, 0), 205 ahead on the right. The model's box is
/// x -60..50, y -25..25, z -5..5: its centre (-5, 0, 0) is 155 ahead.
fn two_cubes() -> (Session, Shell, egui::Context) {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Near", "size": [10, 10, 10], "position": [40, -25, -5]}),
    );
    run(
        &mut session,
        "feature.block",
        json!({"name": "Far", "size": [10, 10, 10], "position": [-60, 15, -5]}),
    );
    let mut shell = Shell::new();
    let ctx = egui::Context::default();
    frame(&ctx, &mut shell, &mut session);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({
            "camera": {"target": [0, 0, 0], "distance": 150, "yaw": 0, "pitch": 0,
                       "projection": "perspective", "fovY": 30, "autoFit": false},
            "look": {"mode": "studio", "ground": false, "shadows": false,
                     "background": "plain", "edges": "off",
                     "dof": {"fstop": 1.4, "focus": "selection"}},
        }),
    );
    frame(&ctx, &mut shell, &mut session);
    (session, shell, ctx)
}

fn select(session: &mut Session, feature: &str) {
    run(
        session,
        "select.set",
        json!({"items": [{"kind": "body", "feature": feature}]}),
    );
}

fn lens(shell: &mut Shell, session: &mut Session) -> Value {
    call(shell, session, "ui.get", json!({}))["lens"].clone()
}

fn near(a: &Value, b: f64) -> bool {
    a.as_f64().is_some_and(|a| (a - b).abs() < 1e-9)
}

fn at(lens: &Value, point: [f64; 3], distance: f64) {
    for (k, want) in point.iter().enumerate() {
        assert!(near(&lens["point"][k], *want), "{lens}: {point:?}");
    }
    assert!(near(&lens["distance"], distance), "{lens}: {distance}");
}

#[test]
fn the_lens_focuses_on_the_selected_body_and_follows_the_selection() {
    let (mut session, mut shell, ctx) = two_cubes();
    // Nothing selected: the model's centre, and it says so.
    let got = lens(&mut shell, &mut session);
    assert_eq!(got["focus"], "selection");
    at(&got, [-5.0, 0.0, 0.0], 155.0);
    assert!(
        got["fallback"]
            .as_str()
            .unwrap()
            .contains("nothing selected"),
        "{got}"
    );
    // The near cube, selected as the viewport selects it.
    select(&mut session, "Near");
    frame(&ctx, &mut shell, &mut session);
    let got = lens(&mut shell, &mut session);
    at(&got, [45.0, -20.0, 0.0], 105.0);
    assert_eq!(got["fallback"], Value::Null);
    // The far one: the focus follows.
    select(&mut session, "Far");
    frame(&ctx, &mut shell, &mut session);
    at(&lens(&mut shell, &mut session), [-55.0, 20.0, 0.0], 205.0);
    // Chosen in the feature list: the feature's faces, the same box.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"selectedFeature": "F1"}),
    );
    frame(&ctx, &mut shell, &mut session);
    at(&lens(&mut shell, &mut session), [45.0, -20.0, 0.0], 105.0);
    // A face: the centre of the near cube's front face (+x, face 5).
    run(
        &mut session,
        "select.set",
        json!({"items": [{"kind": "face", "feature": "Near", "name": "F1:face(5)"}]}),
    );
    frame(&ctx, &mut shell, &mut session);
    at(&lens(&mut shell, &mut session), [50.0, -20.0, 0.0], 100.0);
    // Cleared: back to the centre.
    run(&mut session, "select.set", json!({"mode": "clear"}));
    frame(&ctx, &mut shell, &mut session);
    let got = lens(&mut shell, &mut session);
    at(&got, [-5.0, 0.0, 0.0], 155.0);
    assert!(got["fallback"].is_string());
    // Without a lens, or in the plain look, there is none to report.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"look": {"dof": null}}),
    );
    assert_eq!(lens(&mut shell, &mut session), Value::Null);
}

fn screenshot(shell: &mut Shell, session: &mut Session) -> (usize, usize, Vec<u8>) {
    let result = call(
        shell,
        session,
        "ui.screenshot",
        json!({"width": 320, "height": 240, "scale": 1.0}),
    );
    let png = base64_decode(result["base64"].as_str().unwrap());
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
        .read_info()
        .unwrap();
    let mut rgb = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgb).unwrap();
    let channels = info.color_type.samples();
    let gray = rgb
        .chunks(channels)
        .map(|p| {
            let sum: u32 = p.iter().take(3).map(|&c| u32::from(c)).sum();
            (sum / 3) as u8
        })
        .collect();
    (info.width as usize, info.height as usize, gray)
}

fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        }
    };
    let bytes: Vec<u8> = text.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut word = 0_u32;
        for (k, &c) in chunk.iter().enumerate() {
            word |= value(c) << (18 - 6 * k);
        }
        for k in 0..chunk.len().saturating_sub(1) {
            out.push((word >> (16 - 8 * k)) as u8);
        }
    }
    out
}

/// How far a picture is from the sharp one, in its left and its right
/// half: the sum of the differences of the grey levels. Blur moves the
/// light of a part's outline into its surroundings.
fn off_sharp(picture: &(usize, usize, Vec<u8>), sharp: &(usize, usize, Vec<u8>)) -> [u64; 2] {
    let (w, _, gray) = picture;
    let mut halves = [0; 2];
    for (k, (a, b)) in gray.iter().zip(&sharp.2).enumerate() {
        halves[usize::from(k % w >= w / 2)] += u64::from(a.abs_diff(*b));
    }
    halves
}

#[test]
fn the_part_in_focus_is_sharp_and_the_other_blurred() {
    let (mut session, mut shell, _ctx) = two_cubes();
    // Sixteen frames, so the lens's disc is filled and the picture is
    // not grainy (`look.accumulate`).
    let mut shot = |shell: &mut Shell, dof: Value| {
        call(
            shell,
            &mut session,
            "ui.set",
            json!({"look": {"dof": dof, "accumulate": 16}}),
        );
        screenshot(shell, &mut session)
    };
    // Nothing selected, so no outline is drawn over the picture: the
    // lens focused on each cube's centre in turn, at f/4 (at f/1.4 the
    // near cube's front, 5 nearer than its centre, is out of focus too).
    let sharp = shot(&mut shell, Value::Null);
    let on_near = off_sharp(
        &shot(&mut shell, json!({"fstop": 4, "focus": [45.0, -20.0, 0.0]})),
        &sharp,
    );
    let on_far = off_sharp(
        &shot(&mut shell, json!({"fstop": 4, "focus": [-55.0, 20.0, 0.0]})),
        &sharp,
    );
    eprintln!("off the sharp picture, focused near {on_near:?}, far {on_far:?}");
    // Near is on the left, Far on the right: each half is close to the
    // sharp picture when its cube is in focus, far from it when not.
    assert!(4 * on_near[0] < on_far[0], "{on_near:?} {on_far:?}");
    assert!(4 * on_far[1] < on_near[1], "{on_near:?} {on_far:?}");
    // Stopped down to f/22 the far cube, out of focus, is much nearer
    // the sharp picture than at f/4.
    let stopped = off_sharp(
        &shot(
            &mut shell,
            json!({"fstop": 22, "focus": [45.0, -20.0, 0.0]}),
        ),
        &sharp,
    );
    eprintln!("f/22: {stopped:?}");
    assert!(2 * stopped[1] < on_near[1], "{stopped:?} {on_near:?}");
}

/// The lens on the selection draws the very picture of the lens on the
/// selection's centre given as a point: the selection only says where.
#[test]
fn the_lens_on_the_selection_is_the_lens_on_its_centre() {
    let (mut session, mut shell, ctx) = two_cubes();
    for (feature, centre) in [("Near", [45.0, -20.0, 0.0]), ("Far", [-55.0, 20.0, 0.0])] {
        select(&mut session, feature);
        frame(&ctx, &mut shell, &mut session);
        let mut shot = |shell: &mut Shell, focus: Value| {
            call(
                shell,
                &mut session,
                "ui.set",
                json!({"look": {"dof": {"fstop": 1.4, "focus": focus}}}),
            );
            screenshot(shell, &mut session).2
        };
        let selected = shot(&mut shell, json!("selection"));
        let point = shot(&mut shell, json!(centre));
        let centred = shot(&mut shell, json!("centre"));
        assert!(selected == point, "{feature}: the selection's centre");
        assert!(selected != centred, "{feature}: not the model's centre");
    }
}

#[test]
fn bad_lens_values_are_refused_or_clamped_and_the_state_round_trips() {
    let (mut session, mut shell, _ctx) = two_cubes();
    let before = shell.state.look.clone();
    for bad in [
        json!({"look": {"dof": {"focus": "nowhere"}}}),
        json!({"look": {"dof": {"focus": "Selection"}}}),
        json!({"look": {"dof": {"focus": 0}}}),
        json!({"look": {"dof": {"focus": -2}}}),
        json!({"look": {"dof": {"focus": [1, 2]}}}),
        json!({"look": {"dof": {"focus": [1, 2, "x"]}}}),
        json!({"look": {"dof": {"fstop": "wide"}}}),
        json!({"look": {"dof": {"fstop": null}}}),
        json!({"look": {"dof": {"fstop": 0}}}),
        json!({"look": {"dof": {"fstop": -1}}}),
        json!({"look": {"dof": {"aperture": 2}}}),
        json!({"look": {"dof": 3}}),
    ] {
        let error = refused(&mut shell, &mut session, bad.clone());
        assert!(error.contains("dof"), "{bad}: {error}");
        if bad["look"]["dof"]["fstop"].is_number() {
            assert!(error.contains("positive f-number"), "{bad}: {error}");
        }
        assert_eq!(shell.state.look, before, "{bad}: a refusal changes nothing");
    }
    // A positive f-number out of range is clamped to the lens's 0.5 to
    // 64 (one that is not positive was refused above).
    for (fstop, want) in [(1e-6, 0.5), (0.2, 0.5), (1e12, 64.0)] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"look": {"dof": {"fstop": fstop}}}),
        );
        let look = call(&mut shell, &mut session, "ui.get", json!({}))["state"]["look"].clone();
        assert_eq!(look["dof"]["fstop"], want, "{fstop}");
        assert_eq!(look["dof"]["focus"], "selection");
    }
    // The state with every focus survives serde unchanged.
    for focus in [
        json!("selection"),
        json!("picked"),
        json!("centre"),
        json!(120.5),
        json!([1.0, 2.0, 3.0]),
    ] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"look": {"dof": {"fstop": 4.0, "focus": focus}}}),
        );
        let state = shell.state.look.clone();
        let text = serde_json::to_string(&state).unwrap();
        let back: LookState = serde_json::from_str(&text).unwrap();
        assert_eq!(back, state, "{text}");
        assert_eq!(back.checked().unwrap(), state, "{text}");
    }
}

/// The focus on the selection never goes stale: it follows an edit that
/// moves the selected body, comes back with undo, and falls back (saying
/// why) when the body is hidden or deleted.
#[test]
fn the_focus_on_the_selection_follows_edits_and_falls_back() {
    let (mut session, mut shell, ctx) = two_cubes();
    select(&mut session, "Near");
    frame(&ctx, &mut shell, &mut session);
    at(&lens(&mut shell, &mut session), [45.0, -20.0, 0.0], 105.0);
    // Moved 20 towards the eye: its centre is at x 65, 85 ahead.
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Near", "position": [60, -25, -5]}),
    );
    frame(&ctx, &mut shell, &mut session);
    at(&lens(&mut shell, &mut session), [65.0, -20.0, 0.0], 85.0);
    run(&mut session, "edit.undo", json!({}));
    frame(&ctx, &mut shell, &mut session);
    at(&lens(&mut shell, &mut session), [45.0, -20.0, 0.0], 105.0);
    // Hidden: still selected, not shown.
    run(
        &mut session,
        "feature.visible",
        json!({"feature": "Near", "visible": false}),
    );
    frame(&ctx, &mut shell, &mut session);
    let got = lens(&mut shell, &mut session);
    assert!(
        got["fallback"].as_str().unwrap().contains("not shown"),
        "{got}"
    );
    run(&mut session, "edit.undo", json!({}));
    frame(&ctx, &mut shell, &mut session);
    at(&lens(&mut shell, &mut session), [45.0, -20.0, 0.0], 105.0);
    // Deleted: what was selected is gone. Only Far is left, so the
    // model's centre is its centre.
    run(&mut session, "feature.delete", json!({"feature": "Near"}));
    frame(&ctx, &mut shell, &mut session);
    let got = lens(&mut shell, &mut session);
    at(&got, [-55.0, 20.0, 0.0], 205.0);
    assert!(got["fallback"].as_str().unwrap().contains("gone"), "{got}");
}

/// An orthographic view has no lens: `ui.get` says so, and so does the
/// popover's note.
#[test]
fn an_orthographic_view_says_the_lens_blurs_nothing() {
    let (mut session, mut shell, ctx) = two_cubes();
    assert_eq!(lens(&mut shell, &mut session)["noBlur"], Value::Null);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"projection": "orthographic"}}),
    );
    frame(&ctx, &mut shell, &mut session);
    let got = lens(&mut shell, &mut session);
    assert!(
        got["noBlur"].as_str().unwrap().contains("orthographic"),
        "{got}"
    );
    // A focus behind the eye blurs nothing either.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"projection": "perspective"},
               "look": {"dof": {"focus": [400.0, 0.0, 0.0]}}}),
    );
    frame(&ctx, &mut shell, &mut session);
    let got = lens(&mut shell, &mut session);
    assert!(got["noBlur"].as_str().unwrap().contains("behind"), "{got}");
}

/// While the lens is on the selected faces keep their own colour (the
/// part in focus shows its sharpness; the outline says what is
/// selected), and without a lens they are lit in the selection's colour.
#[test]
fn the_selection_is_not_tinted_under_the_lens() {
    let (mut session, mut shell, ctx) = two_cubes();
    select(&mut session, "Near");
    frame(&ctx, &mut shell, &mut session);
    let scene = shell.shown_scene_arc();
    assert!(shell.lens_on());
    assert_eq!(
        shell.face_highlight(scene.as_deref()),
        craft3d_render::Highlight::NONE
    );
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"look": {"dof": null}}),
    );
    assert!(!shell.lens_on());
    assert_ne!(
        shell.face_highlight(scene.as_deref()),
        craft3d_render::Highlight::NONE
    );
}

/// A face picked by a click in the viewport is selected and pointed at;
/// under the lens neither tints it (the outline says it), without the
/// lens it is lit as ever.
#[test]
fn a_clicked_face_is_not_tinted_under_the_lens() {
    let (mut session, mut shell, ctx) = two_cubes();
    // The near cube's front face, seen at (70, 120) of a 320 x 240
    // picture: its centre (50, -20, 0) is 100 ahead and 20 to the left,
    // 20 / (100 tan 15° · 4/3) = 0.56 of the half width.
    call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": 70, "y": 120, "width": 320, "height": 240, "select": true}),
    );
    frame(&ctx, &mut shell, &mut session);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["selection"]["count"], 1, "{}", got["selection"]);
    assert!(
        got["state"]["highlight"].is_object(),
        "the click points at it"
    );
    let scene = shell.shown_scene_arc();
    assert!(shell.lens_on());
    assert_eq!(
        shell.face_highlight(scene.as_deref()),
        craft3d_render::Highlight::NONE,
        "under the lens"
    );
    // The face is the lens's focus: the centre of the front face.
    at(&lens(&mut shell, &mut session), [50.0, -20.0, 0.0], 100.0);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"look": {"dof": null}}),
    );
    assert_ne!(
        shell.face_highlight(scene.as_deref()),
        craft3d_render::Highlight::NONE,
        "without the lens"
    );
}

/// A selected face that gets no outline (a sphere's, bounded by its seam
/// alone) is still lit under the lens, or nothing would say it is
/// selected.
#[test]
fn a_face_without_an_outline_is_still_lit_under_the_lens() {
    let (mut session, mut shell, ctx) = two_cubes();
    run(
        &mut session,
        "feature.sphere",
        json!({"name": "Ball", "radius": 5, "position": [0, 0, 0]}),
    );
    select(&mut session, "Ball");
    frame(&ctx, &mut shell, &mut session);
    let scene = shell.shown_scene_arc();
    assert!(shell.lens_on());
    assert_ne!(
        shell.face_highlight(scene.as_deref()),
        craft3d_render::Highlight::NONE
    );
    // A cube's faces have their outline: not lit.
    select(&mut session, "Near");
    frame(&ctx, &mut shell, &mut session);
    assert_eq!(
        shell.face_highlight(scene.as_deref()),
        craft3d_render::Highlight::NONE
    );
}

/// No lens drawn, no lens to say the selection by its outline alone: a
/// focus behind the camera blurs nothing, so the selection is lit.
#[test]
fn the_selection_is_lit_when_the_lens_blurs_nothing() {
    let (mut session, mut shell, ctx) = two_cubes();
    select(&mut session, "Near");
    frame(&ctx, &mut shell, &mut session);
    assert!(shell.lens_on());
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"look": {"dof": {"focus": [400.0, 0.0, 0.0]}}}),
    );
    frame(&ctx, &mut shell, &mut session);
    assert!(!shell.lens_on(), "the focus is behind the eye");
    let scene = shell.shown_scene_arc();
    assert_ne!(
        shell.face_highlight(scene.as_deref()),
        craft3d_render::Highlight::NONE
    );
}
