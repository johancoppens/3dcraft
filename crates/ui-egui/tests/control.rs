//! The `ui.*` methods, called the way the control channel calls them:
//! method name and JSON params in, JSON or an error sentence out.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_math::Vec3;
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::shell::{DEFAULT_VIEWPORT, image_quality};
use craft3d_ui_egui::{Shell, Tokens, UI_METHODS, UiState};
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
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

fn refused(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> String {
    match shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
    {
        Ok(value) => panic!("{method} {params} should fail, returned {value}"),
        Err(e) => {
            assert!(e.starts_with(method), "the error names the method: {e}");
            e
        }
    }
}

/// The demo part of docs/development.md: a plate with a post on it.
fn plate_and_post(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    run(
        session,
        "variable.set",
        json!({"name": "width", "expr": "60 mm"}),
    );
    run(
        session,
        "feature.block",
        json!({"name": "Plate", "size": ["width", "width * 2 / 3", "8 mm"]}),
    );
    run(
        session,
        "feature.cylinder",
        json!({"name": "Post", "position": ["width / 4", "width / 3", "8 mm"],
               "radius": "width / 10", "height": "30 mm"}),
    );
}

/// The L profile of `crates/engine/tests/sketch.rs`: 40 wide, 30 high, 5
/// thick, drawn by eye and constrained.
fn l_profile(session: &mut Session) {
    run(session, "file.new", json!({"units": "mm"}));
    run(session, "sketch.create", json!({"name": "Profile"}));
    run(
        session,
        "sketch.polyline",
        json!({"sketch": "Profile", "closed": true,
               "points": [[0, 0], [38, 1.5], [41, 6], [4, 4.5], [6.5, 28], [-1, 31]]}),
    );
    let mut constrain = |kind: &str, entity: &str, value: Option<&str>| {
        let mut params = json!({"sketch": "Profile", "type": kind, "entities": [entity]});
        if let Some(value) = value {
            params["value"] = value.into();
        }
        run(session, "sketch.constrain", params);
    };
    constrain("fixed", "e1", None);
    for (line, kind) in [
        ("e7", "horizontal"),
        ("e8", "vertical"),
        ("e9", "horizontal"),
        ("e10", "vertical"),
        ("e11", "horizontal"),
        ("e12", "vertical"),
    ] {
        constrain(kind, line, None);
    }
    constrain("distance", "e7", Some("40 mm"));
    constrain("distance", "e12", Some("30 mm"));
    constrain("distance", "e8", Some("5 mm"));
    constrain("distance", "e11", Some("5 mm"));
}

/// Where the developer looks (docs/testing.md, evidence).
fn evidence(name: &str) -> PathBuf {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui-egui/control");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn unbase64(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => u32::from(c - b'A'),
        b'a'..=b'z' => u32::from(c - b'a') + 26,
        b'0'..=b'9' => u32::from(c - b'0') + 52,
        b'+' => 62,
        b'/' => 63,
        other => panic!("not base64: {other}"),
    };
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        assert_eq!(chunk.len(), 4, "padded");
        let data: Vec<u8> = chunk.iter().copied().filter(|&c| c != b'=').collect();
        let mut group = 0_u32;
        for (i, &c) in data.iter().enumerate() {
            group |= value(c) << (18 - 6 * i);
        }
        out.extend(group.to_be_bytes()[1..].iter().take(data.len() - 1));
    }
    out
}

/// Decodes a PNG to (width, height, RGB bytes).
fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(png_bytes).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

/// The fraction of pixels that are not the viewport's background.
fn drawn(rgb: &[u8]) -> f64 {
    let background = Tokens::default().viewport_background;
    let pixels = rgb.as_chunks::<3>().0;
    pixels.iter().filter(|p| **p != background).count() as f64 / pixels.len() as f64
}

#[test]
fn only_ui_methods_are_handled_and_each_listed_one_exists() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    for other in [
        "engine.execute",
        "batch",
        "methods",
        "ui",
        "ui.",
        "ui.nope",
        "UI.GET",
        "",
    ] {
        assert!(
            shell.handle_method(&mut session, other, None).is_none(),
            "{other}"
        );
    }
    let names: Vec<&str> = UI_METHODS.iter().map(|m| m.name).collect();
    assert_eq!(
        names,
        [
            "ui.get",
            "ui.set",
            "ui.pick",
            "ui.highlight",
            "ui.select",
            "ui.fit",
            "ui.screenshot",
            "ui.param",
            "ui.notice",
            "ui.quit"
        ]
    );
    for method in UI_METHODS {
        assert!(
            shell
                .handle_method(&mut session, method.name, None)
                .is_some(),
            "{} is listed but not handled",
            method.name
        );
        assert!(method.params.starts_with('{') && !method.summary.is_empty());
    }
}

#[test]
fn ui_get_reports_the_state_the_scene_and_the_tree() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let empty = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        empty["state"],
        serde_json::to_value(UiState::default()).unwrap()
    );
    assert_eq!(
        empty["viewport"],
        json!({"width": DEFAULT_VIEWPORT[0], "height": DEFAULT_VIEWPORT[1], "pixelsPerPoint": 1.0})
    );
    assert_eq!(empty["document"], Value::Null);
    assert_eq!(empty["scene"], Value::Null);
    assert_eq!(empty["features"], json!([]));
    assert_eq!(empty["render"], Value::Null, "no window has rendered");
    assert_eq!(empty["lastCommand"], Value::Null);

    plate_and_post(&mut session);
    run(
        &mut session,
        "feature.sphere",
        json!({"name": "Broken", "radius": "nope"}),
    );
    let got = call(&mut shell, &mut session, "ui.get", Value::Null);
    assert_eq!(
        got["document"]["units"],
        json!({"length": "mm", "angle": "deg"})
    );
    assert_eq!(got["document"]["dirty"], true);
    assert_eq!(got["document"]["path"], Value::Null);
    assert_eq!(got["scene"]["bodies"], 2);
    assert_eq!(got["scene"]["sketches"], 0);
    assert_eq!(
        got["scene"]["bbox"],
        json!([[0.0, 0.0, 0.0], [60.0, 40.0, 38.0]])
    );
    assert_eq!(got["scene"]["revision"], session.scene_revision());
    let features = got["features"].as_array().unwrap();
    assert_eq!(features.len(), 3);
    assert_eq!(
        features[0],
        json!({"id": "F1", "name": "Plate", "kind": "block", "state": "ok",
               "status": "ok", "error": null, "causedBy": null, "selected": false,
               "hidden": false,
               // Added with the list following the selection (2026-10-23).
               "madeSelection": false,
               // Added with the row's zoom icon (2026-10-23).
               "zoomable": true})
    );
    assert_eq!(features[2]["state"], "failed");
    assert_eq!(
        features[2]["zoomable"], false,
        "a failed feature cannot be flown to"
    );
    assert!(features[2]["error"].as_str().unwrap().contains("nope"));
    // The camera followed the model: it looks at the middle of its box.
    assert_eq!(got["state"]["camera"]["target"], json!([30.0, 20.0, 19.0]));
    assert_eq!(got["state"]["camera"]["autoFit"], true);

    let error = refused(&mut shell, &mut session, "ui.get", json!({"verbose": true}));
    assert!(error.contains("verbose"), "{error}");
}

#[test]
fn ui_set_changes_the_state_or_nothing() {
    let mut session = Session::new(Workspace::none());
    plate_and_post(&mut session);
    let mut shell = Shell::new();
    let set = call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"view": "top", "panels": {"tree": false}, "notice": "Step 1",
               "selectedFeature": "F2", "camera": {"projection": "orthographic"}}),
    );
    assert_eq!(set["state"]["camera"]["yaw"], -90.0);
    assert_eq!(set["state"]["camera"]["pitch"], 90.0);
    assert_eq!(set["state"]["camera"]["projection"], "orthographic");
    assert_eq!(
        set["state"]["panels"],
        json!({"tree": false, "status": true, "recovery": true})
    );
    assert_eq!(set["state"]["notice"], "Step 1");
    assert_eq!(set["state"]["selectedFeature"], "F2");
    assert_eq!(set["state"], serde_json::to_value(&shell.state).unwrap());
    // Turning the camera keeps it on the model.
    assert_eq!(set["state"]["camera"]["target"], json!([30.0, 20.0, 19.0]));

    // A selection that names no feature does not stick.
    let set = call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"selectedFeature": "F99"}),
    );
    assert_eq!(set["state"]["selectedFeature"], Value::Null);

    let before = shell.state.clone();
    for bad in [
        Value::Null,
        json!(3),
        json!("top"),
        json!([]),
        json!({"view": "sideways"}),
        json!({"camera": {"yaw": "a lot"}}),
        json!({"camera": {"roll": 3}}),
        json!({"theme": "light"}),
        json!({"notice": "ok", "panels": {"tree": "no"}}),
    ] {
        let error = refused(&mut shell, &mut session, "ui.set", bad.clone());
        assert!(error.len() > "ui.set: ".len(), "{bad}: {error}");
        assert_eq!(shell.state, before, "{bad} changed the state");
    }
    // Out of range is clamped, not refused.
    let set = call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"distance": 1e300, "pitch": 1000}}),
    );
    assert_eq!(set["state"]["camera"]["distance"], 1e12);
    assert_eq!(set["state"]["camera"]["pitch"], 90.0);
    assert_eq!(set["state"]["camera"]["autoFit"], false);
    // No command ran: the document is as it was.
    assert_eq!(
        run(&mut session, "document.inspect", json!({}))["history"]["undo"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn ui_fit_frames_the_model_and_refuses_when_there_is_none() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let start = shell.state.clone();
    let error = refused(&mut shell, &mut session, "ui.fit", json!({}));
    assert!(error.contains("nothing to frame"), "{error}");
    refused(&mut shell, &mut session, "ui.fit", json!({"view": "top"}));
    assert_eq!(shell.state, start, "a refused fit does not turn the camera");

    plate_and_post(&mut session);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"target": [500, 500, 500], "distance": 3}}),
    );
    assert!(!shell.state.camera.auto_fit);
    let fitted = call(&mut shell, &mut session, "ui.fit", json!({"view": "front"}));
    assert_eq!(fitted["camera"]["target"], json!([30.0, 20.0, 19.0]));
    assert_eq!(fitted["camera"]["yaw"], -90.0);
    assert_eq!(fitted["camera"]["pitch"], 0.0);
    assert_eq!(fitted["camera"]["autoFit"], true);
    // From the front the box is 60 wide and 38 high, and its nearest face
    // is 20 in front of the centre. In a 960 × 640 viewport (3:2) the half
    // width of 30 needs 30 / (1.5 · tan 15°) = 74.6 of depth and the half
    // height of 19 needs 19 / tan 15° = 70.9: the width decides.
    let expected = (20.0 + 30.0 / (1.5 * 15.0_f64.to_radians().tan())) * 1.05;
    let distance = fitted["camera"]["distance"].as_f64().unwrap();
    assert!(
        (distance - expected).abs() < 1e-9,
        "{distance} vs {expected}"
    );

    // It follows the model from now on.
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Post", "height": "92 mm"}),
    );
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["camera"]["target"], json!([30.0, 20.0, 50.0]));

    let before = shell.state.clone();
    for bad in [
        json!({"view": "inside"}),
        json!({"view": 3}),
        json!({"margin": 2}),
        json!(7),
    ] {
        refused(&mut shell, &mut session, "ui.fit", bad);
        assert_eq!(shell.state, before);
    }
}

#[test]
fn ui_notice_sets_and_clears_the_caption() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    let set = call(
        &mut shell,
        &mut session,
        "ui.notice",
        json!({"text": "Adding the post"}),
    );
    assert_eq!(set, json!({"notice": "Adding the post"}));
    assert_eq!(shell.state.notice, "Adding the post");
    let long = call(
        &mut shell,
        &mut session,
        "ui.notice",
        json!({"text": "x".repeat(10_000)}),
    );
    assert_eq!(long["notice"].as_str().unwrap().len(), 200);
    for bad in [
        json!({}),
        json!({"text": 5}),
        json!({"text": null}),
        json!({"txt": "a"}),
        json!("hi"),
    ] {
        refused(&mut shell, &mut session, "ui.notice", bad);
        assert_eq!(shell.state.notice.len(), 200);
    }
    call(&mut shell, &mut session, "ui.notice", json!({"text": ""}));
    assert_eq!(shell.state.notice, "");
}

#[test]
fn ui_quit_asks_the_application_to_close() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    refused(&mut shell, &mut session, "ui.quit", json!({"force": true}));
    assert!(!shell.quit_requested());
    assert_eq!(
        call(&mut shell, &mut session, "ui.quit", json!({})),
        json!({"quitting": true})
    );
    assert!(shell.quit_requested());
}

#[test]
fn ui_screenshot_returns_a_png_of_the_requested_size_that_shows_the_model() {
    let mut session = Session::new(Workspace::none());
    plate_and_post(&mut session);
    let mut shell = Shell::new();
    let shot = call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        json!({"width": 320, "height": 200}),
    );
    assert_eq!(shot["mime"], "image/png");
    assert_eq!(
        (&shot["width"], &shot["height"]),
        (&json!(320), &json!(200))
    );
    let png = unbase64(shot["base64"].as_str().unwrap());
    assert_eq!(shot["bytes"], png.len());
    let (width, height, rgb) = decode(&png);
    assert_eq!((width, height), (320, 200));
    // Not blank. The plate and post, framed for the viewport's 3:2, fill
    // well over a tenth of an image of nearly that shape; the floor is 10 %.
    let coverage = shot["coverage"].as_f64().unwrap();
    assert!(coverage > 0.10 && coverage < 0.9, "coverage {coverage}");
    let drawn_fraction = drawn(&rgb);
    assert!(
        drawn_fraction >= coverage - 0.01 && drawn_fraction < coverage + 0.05,
        "{drawn_fraction} of the decoded pixels are drawn, coverage {coverage}"
    );
    assert_eq!(
        shot["triangles"],
        session
            .scene_with(shell.scene_quality())
            .unwrap()
            .unwrap()
            .triangles()
    );

    // Without a size it is the viewport's.
    let shot = call(&mut shell, &mut session, "ui.screenshot", json!({}));
    let (width, height, _) = decode(&unbase64(shot["base64"].as_str().unwrap()));
    assert_eq!([width, height], DEFAULT_VIEWPORT);

    // Right way up and not mirrored: the top of the post is the highest
    // thing in the picture, and it is where the camera says it is, left
    // of the middle.
    let (w, h) = (480_u32, 320_u32);
    let shot = call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        json!({"width": w, "height": h}),
    );
    let (_, _, rgb) = decode(&unbase64(shot["base64"].as_str().unwrap()));
    let tokens = Tokens::default();
    let is_body = |x: u32, y: u32| {
        let i = ((y * w + x) * 3) as usize;
        // Shaded body colour: grey, lighter than the background.
        let (r, g, b) = (rgb[i], rgb[i + 1], rgb[i + 2]);
        [r, g, b] != tokens.viewport_background && r.abs_diff(b) < 40 && r > 60
    };
    let top_row = (0..h).find(|&y| (0..w).any(|x| is_body(x, y))).unwrap();
    let xs: Vec<u32> = (0..w).filter(|&x| is_body(x, top_row)).collect();
    let top_x = f64::from(xs[xs.len() / 2]);
    let projector =
        Projector::new(&shell.state.camera.to_render(), f64::from(w), f64::from(h)).unwrap();
    // The post: axis at (15, 20), radius 6, from z = 8 to z = 38.
    let post_top = projector.project(Vec3::new(15.0, 20.0, 38.0)).unwrap();
    let plate_middle = projector.project(Vec3::new(30.0, 20.0, 8.0)).unwrap();
    let radius_pixels = (projector
        .project(Vec3::new(15.0, 20.0, 38.0) + shell.state.camera.right() * 6.0)
        .unwrap()
        .x
        - post_top.x)
        .abs();
    assert!(
        (top_x - post_top.x).abs() <= radius_pixels + 1.0,
        "the highest body pixel is at x {top_x}, the post's top at {}",
        post_top.x
    );
    assert!((f64::from(top_row) - post_top.y).abs() <= radius_pixels + 1.0);
    assert!(post_top.x < plate_middle.x && post_top.y < plate_middle.y);
    assert!(
        top_x < f64::from(w) / 2.0,
        "the post stands on the left half of the plate"
    );
}

#[test]
fn ui_screenshot_shows_a_sketch_without_any_body() {
    let mut session = Session::new(Workspace::none());
    l_profile(&mut session);
    let mut shell = Shell::new();
    call(&mut shell, &mut session, "ui.fit", json!({"view": "top"}));
    let (w, h) = (600_u32, 400_u32);
    let shot = call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        json!({"width": w, "height": h}),
    );
    assert_eq!(shot["coverage"], 0.0, "no body");
    let overlay = shot["overlayCoverage"].as_f64().unwrap();
    assert!(
        overlay > 0.005 && overlay < 0.1,
        "overlay coverage {overlay}"
    );
    let (_, _, rgb) = decode(&unbase64(shot["base64"].as_str().unwrap()));
    let tokens = Tokens::default();
    let pixel = |x: f64, y: f64| {
        let i = (y as usize * w as usize + x as usize) * 3;
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    };
    // The pixel a point of a line falls in has its centre at most half a
    // pixel from the line; a line 1.75 wide covers such a pixel for at
    // least 87.5 %, so its colour is within an eighth of the way to the
    // background: 28 of 255 at most.
    let close = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= 29);
    let projector =
        Projector::new(&shell.state.camera.to_render(), f64::from(w), f64::from(h)).unwrap();
    let at = |x: f64, y: f64| {
        let p = projector.project(Vec3::new(x, y, 0.0)).unwrap();
        (p.x, p.y)
    };
    // The middles of the six edges of the solved L (40 × 30, 5 thick) are
    // drawn in the sketch colour; the inside of the L and the notch are
    // not drawn at all.
    for (x, y) in [
        (20.0, 0.0),
        (40.0, 2.5),
        (22.5, 5.0),
        (5.0, 17.5),
        (2.5, 30.0),
        (0.0, 15.0),
    ] {
        let (px, py) = at(x, y);
        assert!(
            close(pixel(px, py), tokens.sketch),
            "edge through ({x}, {y}) at pixel ({px:.1}, {py:.1}): {:?}",
            pixel(px, py)
        );
    }
    for (x, y) in [(20.0, 2.5), (2.5, 20.0), (25.0, 20.0), (-6.0, 10.0)] {
        let (px, py) = at(x, y);
        assert_eq!(pixel(px, py), tokens.viewport_background, "({x}, {y})");
    }
    // The long leg runs to the right and the short one up the screen.
    assert!(at(40.0, 0.0).0 > at(0.0, 0.0).0 && at(0.0, 30.0).1 < at(0.0, 0.0).1);

    let path = evidence("l-profile.png");
    std::fs::write(&path, unbase64(shot["base64"].as_str().unwrap())).unwrap();
}

#[test]
fn ui_screenshot_writes_under_the_write_root_only() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ui-screenshot");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = Session::new(Workspace {
        read: Access::None,
        write: Access::Root(dir.clone()),
    });
    plate_and_post(&mut session);
    let mut shell = Shell::new();
    let shot = call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        json!({"path": "part.png", "width": 300, "height": 200}),
    );
    assert_eq!(shot["path"], "part.png");
    assert!(shot.get("base64").is_none() && shot.get("mime").is_none());
    let written = std::fs::read(dir.join("part.png")).unwrap();
    assert_eq!(shot["bytes"], written.len());
    let (width, height, rgb) = decode(&written);
    assert_eq!((width, height), (300, 200));
    assert!(drawn(&rgb) > 0.10);

    // Outside the root, or into a directory that is not there: refused,
    // and nothing appears anywhere.
    let outside = dir.parent().unwrap().join("ui-screenshot-escape.png");
    let _ = std::fs::remove_file(&outside);
    for bad in [
        "../ui-screenshot-escape.png".to_string(),
        outside.to_str().unwrap().to_string(),
        "missing-directory/part.png".to_string(),
        String::new(),
    ] {
        let error = refused(
            &mut shell,
            &mut session,
            "ui.screenshot",
            json!({"path": bad, "width": 64, "height": 64}),
        );
        assert!(!error.contains("internal error"), "{error}");
    }
    assert!(!outside.exists());
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(entries, ["part.png"], "no stray or temporary files");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ui_screenshot_refuses_bad_params_before_rendering() {
    let mut session = Session::new(Workspace::none());
    plate_and_post(&mut session);
    let mut shell = Shell::new();
    for bad in [
        json!({"width": 5}),
        json!({"width": 0}),
        json!({"height": 99_999}),
        json!({"width": "wide"}),
        json!({"width": 320.5}),
        json!({"width": -320}),
        json!({"height": [200]}),
        json!({"scale": 0}),
        json!({"scale": 100}),
        json!({"scale": "2x"}),
        json!({"path": 7}),
        json!({"path": ""}),
        json!({"format": "jpeg"}),
        json!([640, 480]),
        json!("shot.png"),
    ] {
        let error = refused(&mut shell, &mut session, "ui.screenshot", bad.clone());
        assert!(error.len() > "ui.screenshot: ".len(), "{bad}");
    }
    // No write root: a path is refused, and says how to get the picture.
    let error = refused(
        &mut shell,
        &mut session,
        "ui.screenshot",
        json!({"path": "a.png"}),
    );
    assert!(error.contains("write"), "{error}");
}

/// Every method with every kind of nonsense: an answer, never a panic.
#[test]
fn no_params_panic_a_ui_method() {
    let nonsense = [
        Value::Null,
        json!(true),
        json!(-1),
        json!(1e308),
        json!(""),
        json!("\u{0}\u{202e}"),
        json!([]),
        json!([[[[]]]]),
        json!({}),
        json!({"": ""}),
        json!({"text": {"text": {"text": 1}}}),
        json!({"view": "top", "camera": {"target": [1e308, -1e308, 0], "distance": 0}}),
        json!({"camera": {"fovY": 179.999, "pitch": 90, "distance": 1e-300}}),
        json!({"width": 4096, "height": 4096}),
        json!({"width": 16, "height": 16, "scale": 8}),
        json!({"path": "../../../etc/passwd"}),
        json!({"path": "a\u{0}b"}),
        json!({"selectedFeature": "F1", "notice": "\n\n\n"}),
    ];
    for document in [false, true] {
        let mut session = Session::new(Workspace::none());
        if document {
            plate_and_post(&mut session);
        }
        let mut shell = Shell::new();
        for method in UI_METHODS {
            for params in &nonsense {
                let outcome = shell.handle_method(&mut session, method.name, Some(params));
                assert!(outcome.is_some(), "{} {params}", method.name);
                // Whatever was set, the state is still a valid one.
                assert_eq!(
                    shell.state.clone().validated().unwrap(),
                    shell.state,
                    "{} {params}",
                    method.name
                );
            }
        }
        // And a screenshot of whatever camera that left behind still works
        // or says why not.
        let _ = shell.handle_method(
            &mut session,
            "ui.screenshot",
            Some(&json!({"width": 64, "height": 64})),
        );
    }
}

#[test]
fn the_image_never_exceeds_the_largest_texture() {
    // At rest: the viewport's own size, supersampled.
    let rest = image_quality([1200, 800], false, 8192);
    assert_eq!((rest.width, rest.height, rest.ssaa), (1200, 800, 2));
    // Moving: half per side, one sample per pixel.
    let moving = image_quality([1200, 800], true, 8192);
    assert_eq!((moving.width, moving.height, moving.ssaa), (600, 400, 1));
    // A viewport larger than the largest texture: the same shape, smaller.
    let capped = image_quality([4000, 1000], false, 2048);
    assert_eq!((capped.width, capped.height), (2048, 512));
    let capped = image_quality([16, 2977], false, 2048);
    assert_eq!((capped.width, capped.height), (16, 2048));

    // The sample budget. At rest 4 million: 1652 × 993 is 1.64 million
    // pixels, which fits once but not four times, so it is not supersampled.
    let large = image_quality([1652, 993], false, 8192);
    assert_eq!((large.width, large.height, large.ssaa), (1652, 993, 1));
    // 3840 × 2160 is 8.29 million pixels: both sides by √(4 / 8.2944) =
    // 0.6944, which is 2666.7 × 1500.0.
    let huge = image_quality([3840, 2160], false, 8192);
    assert_eq!((huge.width, huge.ssaa), (2666, 1));
    assert!((1499..=1500).contains(&huge.height), "{huge:?}");
    // Moving 600 thousand: half of 2560 × 1400 is 896 thousand pixels, so
    // both sides by √(600 / 896) = 0.8183, which is 1047.4 × 572.8.
    let moving = image_quality([2560, 1400], true, 8192);
    assert_eq!((moving.width, moving.height, moving.ssaa), (1047, 572, 1));
    for (size, moving) in [
        ([1200, 800], false),
        ([1652, 993], false),
        ([3840, 2160], false),
        ([4096, 4096], false),
        ([2560, 1400], true),
        ([4096, 4096], true),
    ] {
        let q = image_quality(size, moving, 8192);
        let samples = u64::from(q.width) * u64::from(q.height) * u64::from(q.ssaa * q.ssaa);
        let budget = if moving { 600_000 } else { 4_000_000 };
        assert!(samples <= budget, "{size:?}: {q:?} is {samples} samples");
        // Not needlessly small either: over 90 % of the budget is used
        // when the viewport had to be shrunk.
        if q.width < size[0] / if moving { 2 } else { 1 } {
            assert!(samples * 10 > budget * 9, "{size:?}: {q:?}");
        }
    }
    for size in [
        [0, 0],
        [1, 1],
        [16, 16],
        [4096, 4096],
        [u32::MAX, u32::MAX],
        [u32::MAX, 1],
    ] {
        for largest in [0, 1, 16, 2048, 16_384, usize::MAX] {
            for moving in [false, true] {
                let q = image_quality(size, moving, largest);
                let limit = (largest.clamp(16, 4096)) as u32;
                assert!((16..=limit).contains(&q.width), "{size:?} {largest}: {q:?}");
                assert!(
                    (16..=limit).contains(&q.height),
                    "{size:?} {largest}: {q:?}"
                );
            }
        }
    }
}

#[test]
fn evidence_of_the_demo_part() {
    let mut session = Session::new(Workspace::none());
    plate_and_post(&mut session);
    let mut shell = Shell::new();
    call(
        &mut shell,
        &mut session,
        "ui.notice",
        json!({"text": "Plate and post"}),
    );
    let shot = call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        json!({"width": 900, "height": 600}),
    );
    let png = unbase64(shot["base64"].as_str().unwrap());
    std::fs::write(evidence("plate-and-post.png"), &png).unwrap();
    // The axes are in the picture: some pixels in each axis colour.
    let (_, _, rgb) = decode(&png);
    let tokens = Tokens::default();
    for axis in tokens.axes {
        let count = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|p| **p == axis)
            .count();
        assert!(count > 3, "axis colour {axis:?}: {count} pixels");
    }
}
