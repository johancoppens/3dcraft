//! Pointing in the viewport (roadmap M8.5): a click gives the persistent
//! name of the face under it as UI state an agent reads, an agent asks
//! what is at a pixel (`ui.pick`) and has a face highlighted
//! (`ui.highlight`), and the picture shows the highlight.
//!
//! The oracle for "the face under it" is the id buffer of the CPU picture
//! of the viewport, with the names `document.inspect` lists.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Scene, Session, Workspace};
use craft3d_render::{Frame, NO_FACE};
use craft3d_ui_egui::state::{FaceRef, Picked};
use craft3d_ui_egui::viewport::{self, Quality};
use craft3d_ui_egui::{RendererChoice, Shell, Tokens};
use egui::{Event, PointerButton, Pos2, pos2, vec2};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

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

/// A plate with a hole drilled through it by a boolean, a post beside it
/// and a row of three studs: bodies of a primitive, a boolean and a
/// pattern.
fn part() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "position": [42, 20, -4], "radius": 9, "height": 20}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Drilled"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Post", "position": [12, 20, 8], "radius": 6, "height": 30}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Stud", "size": [6, 6, 10], "position": [0, -20, 0]}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Stud", "count": 3, "step": [14, 0, 0], "name": "Studs"}),
    );
    s
}

/// A window of a fixed size, fed one frame at a time.
struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
    /// The images the last frame uploaded as whole textures.
    uploads: Vec<(usize, usize, Vec<u8>)>,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: vec2(width, height),
            time: 0.0,
            uploads: Vec::new(),
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        let uploads: Vec<(usize, usize, Vec<u8>)> = output
            .textures_delta
            .set
            .values()
            .flat_map(|deltas| deltas.iter())
            .filter(|delta| delta.pos.is_none())
            .map(|delta| match &delta.image {
                egui::ImageData::Color(image) => (
                    image.width(),
                    image.height(),
                    image
                        .pixels
                        .iter()
                        .flat_map(|p| [p.r(), p.g(), p.b()])
                        .collect(),
                ),
            })
            .collect();
        if !uploads.is_empty() {
            self.uploads = uploads;
        }
        output.textures_delta.clear();
    }

    fn idle(&mut self, shell: &mut Shell, session: &mut Session, frames: usize) {
        for _ in 0..frames {
            self.frame(shell, session, Vec::new());
        }
    }

    /// The image of `width` x `height` last put on screen.
    fn uploaded(&self, width: u32, height: u32) -> Option<&[u8]> {
        self.uploads
            .iter()
            .rev()
            .find(|(w, h, _)| (*w, *h) == (width as usize, height as usize))
            .map(|(_, _, rgb)| rgb.as_slice())
    }

    fn button(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2, pressed: bool) {
        self.frame(
            shell,
            session,
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }

    /// Moves to `at`, presses and lets go, and lets the picture settle.
    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        self.button(shell, session, at, true);
        self.button(shell, session, at, false);
        self.idle(shell, session, 3);
    }
}

/// The id buffer of the viewport as the shell shows it now: the CPU
/// picture of its scene with its camera, one sample per pixel.
fn id_buffer(shell: &mut Shell, session: &mut Session) -> (Arc<Scene>, Frame) {
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let [width, height] = shell.viewport_size();
    let frame = viewport::render_bodies(
        Some(&scene),
        &shell.state.camera,
        Quality {
            width,
            height,
            ssaa: 1,
        },
        &shell.tokens,
    )
    .unwrap();
    (scene, frame)
}

/// Per visible face, the pixels whose 5 × 5 neighbourhood is all that
/// face: well inside it, away from every edge line drawn over it.
fn interiors(frame: &Frame) -> BTreeMap<u32, Vec<(u32, u32)>> {
    let (w, h) = (frame.width as i64, frame.height as i64);
    let at = |x: i64, y: i64| frame.ids[(y * w + x) as usize];
    let mut out: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
    for y in 2..h - 2 {
        for x in 2..w - 2 {
            let id = at(x, y);
            if id != NO_FACE && (-2..=2).all(|dy| (-2..=2).all(|dx| at(x + dx, y + dy) == id)) {
                out.entry(id).or_default().push((x as u32, y as u32));
            }
        }
    }
    out
}

/// The pixels of `pixels` (one face's, from [`interiors`]) with nothing
/// but that face within 6 pixels: further from every edge than the pick
/// radius of a click (4 pixels at one pixel per point), so that a click
/// there is a click on the face. Added 2026-10-13 with edges under the
/// pointer: a click within the radius of an edge in sight is a click on
/// that edge, whatever face is under it.
fn clear_of_edges(frame: &Frame, pixels: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let (w, h) = (i64::from(frame.width), i64::from(frame.height));
    pixels
        .iter()
        .copied()
        .filter(|&(x, y)| {
            let id = frame.ids[(y * frame.width + x) as usize];
            (-6..=6_i64).all(|dy| {
                (-6..=6_i64).all(|dx| {
                    let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                    nx >= 0
                        && ny >= 0
                        && nx < w
                        && ny < h
                        && frame.ids[(ny * w + nx) as usize] == id
                })
            })
        })
        .collect()
}

/// What `document.inspect` calls the face with id `id` of the scene.
fn inspect_name(session: &mut Session, scene: &Scene, id: u32) -> (String, String, usize) {
    let (body, index) = scene.face_of(id).unwrap();
    let inspect = run(session, "document.inspect", json!({}));
    let entry = inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["feature"] == body.feature.as_str())
        .unwrap()
        .clone();
    (
        body.feature.clone(),
        entry["faceNames"][index].as_str().unwrap().to_string(),
        index,
    )
}

fn pixel(rgb: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * width + x) * 3) as usize;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

/// True for a colour shaded from the highlight token and not from the
/// body's or a line's, in the light or in the shade: shading scales the
/// token, so its hue stays. The token (Mission's copper, 2026-10-21; it
/// was cyan) has 1.6 times as much red as green and 3 times as much green
/// as blue; the body, the edges and the background are grey, the X axis
/// has far more red than green, and the gold of a sketch has far less blue.
fn is_highlight(p: [u8; 3], tokens: &Tokens) -> bool {
    let hue = |[r, g, b]: [i32; 3]| {
        b >= 8 && 4 * r >= 5 * g && 10 * r <= 21 * g && 5 * g >= 9 * b && g <= 5 * b
    };
    assert!(hue(tokens.face_highlight.map(i32::from)), "the token");
    for other in [tokens.body, tokens.edge, tokens.viewport_background]
        .into_iter()
        .chain(tokens.axes)
        .chain([tokens.sketch, tokens.sketch_construction])
    {
        assert!(
            !hue(other.map(i32::from)),
            "{other:?} looks like the highlight"
        );
    }
    hue(p.map(i32::from))
}

/// The fraction of `pixels` that are of the highlight's colour in `rgb`.
fn share(rgb: &[u8], width: u32, pixels: &[(u32, u32)], tokens: &Tokens) -> f64 {
    let marked = pixels
        .iter()
        .filter(|&&(x, y)| is_highlight(pixel(rgb, width, x, y), tokens))
        .count();
    marked as f64 / pixels.len() as f64
}

fn evidence(name: &str, width: u32, height: u32, rgb: &[u8]) {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
    std::fs::create_dir_all(&dir).unwrap();
    let png = craft3d_render::encode_png(width, height, rgb).unwrap();
    std::fs::write(dir.join(name), png).unwrap();
}

#[test]
fn a_click_in_the_viewport_picks_the_face_under_it_and_shows_it_highlighted() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let tokens = shell.tokens;
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 4);
    assert_eq!(shell.state.picked, None);
    assert_eq!(shell.state.highlight, None);
    let rect = shell.viewport_rect().unwrap();
    let [width, height] = shell.viewport_size();
    let (scene, frame) = id_buffer(&mut shell, &mut session);
    let faces = interiors(&frame);
    assert!(faces.len() >= 8, "{} faces are seen", faces.len());
    // Keep the camera where it is, so the id buffer stays the picture.
    let camera = shell.state.camera;

    let mut serial = 0;
    let mut bodies = std::collections::BTreeSet::new();
    let mut clicked_faces = 0;
    for (&id, pixels) in &faces {
        // Changed 2026-10-13 (edges under the pointer): the click was on
        // the middle pixel of the face's inside, which for a narrow face
        // is within the pick radius of one of its edges, and a click
        // there is now a click on that edge. It is on the middle one of
        // the pixels further from every edge than the radius; a face too
        // narrow to have one cannot be clicked without clicking an edge,
        // and is left out (how many are left is counted below).
        let clear = clear_of_edges(&frame, pixels);
        // Changed 2026-10-16 (the larger view cube and the glass bar): and
        // not under the controls over the viewport, whose click is theirs.
        let layout = shell.controls_layout().unwrap();
        let clear: Vec<_> = clear
            .into_iter()
            .filter(|&(x, y)| {
                let at = pos2(rect.left() + x as f32 + 0.5, rect.top() + y as f32 + 0.5);
                !layout
                    .parts()
                    .iter()
                    .any(|(_, part)| part.expand(2.0).contains(at))
            })
            .collect();
        let Some(&(x, y)) = clear.get(clear.len() / 2) else {
            continue;
        };
        clicked_faces += 1;
        let (feature, name, index) = inspect_name(&mut session, &scene, id);
        // The middle of the pixel, in points (one pixel per point here).
        let at = pos2(rect.left() + x as f32 + 0.5, rect.top() + y as f32 + 0.5);
        window.click(&mut shell, &mut session, at);
        assert_eq!(shell.state.camera, camera, "a click is not a drag");
        serial += 1;
        let picked = shell.state.picked.clone().unwrap();
        assert_eq!(picked.serial, serial);
        assert_eq!(
            picked.face.as_deref(),
            Some(name.as_str()),
            "pixel ({x}, {y})"
        );
        assert_eq!(picked.feature.as_deref(), Some(feature.as_str()));
        assert_eq!(picked.face_index, Some(index as u64));
        assert!((picked.x - (f64::from(x) + 0.5)).abs() < 1e-3, "{picked:?}");
        assert!((picked.y - (f64::from(y) + 0.5)).abs() < 1e-3, "{picked:?}");
        assert!(picked.point.is_some());
        assert_eq!(
            shell.state.highlight,
            Some(FaceRef {
                feature: Some(feature.clone()),
                face: name.clone()
            })
        );
        bodies.insert(feature);
        // The picture on screen shows that face, and only that face, in
        // the highlight's colour.
        // (Well inside a face there is still the odd line over it: the
        // seam of a cylinder, an axis.)
        let shown = window.uploaded(width, height).unwrap();
        for (&other, pixels) in &faces {
            let same_name = inspect_name(&mut session, &scene, other).1 == name
                && scene.face_of(other).unwrap().0.feature == scene.face_of(id).unwrap().0.feature;
            let marked = share(shown, width, pixels, &tokens);
            if same_name {
                assert!(marked > 0.9, "face {other}, picked: {marked}");
            } else {
                assert!(marked < 0.01, "face {other} while {id} is picked: {marked}");
            }
        }
        // An agent reads the same through ui.get.
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        assert_eq!(got["state"]["picked"]["face"], name.as_str());
        assert_eq!(got["state"]["picked"]["serial"], serial);
        assert_eq!(got["state"]["highlight"]["face"], name.as_str());
    }
    assert_eq!(
        bodies.len(),
        3,
        "the boolean, the post and the pattern: {bodies:?}"
    );
    assert!(clicked_faces >= 8, "{clicked_faces} faces were clicked");
    let last = window.uploaded(width, height).unwrap().to_vec();
    evidence("pointing-click-cpu.png", width, height, &last);

    // A click on the background: a pick of nothing, and no highlight.
    let empty = frame.ids.iter().position(|&id| id == NO_FACE).unwrap() as u32;
    let (x, y) = (empty % width, empty / width);
    // The top-left corner of the picture is background, away from the
    // buttons at the bottom right.
    assert!(x < 50 && y < 50, "({x}, {y})");
    let at = pos2(rect.left() + x as f32 + 0.5, rect.top() + y as f32 + 0.5);
    window.click(&mut shell, &mut session, at);
    serial += 1;
    assert_eq!(
        shell.state.picked,
        Some(Picked {
            serial,
            x: f64::from(x) + 0.5,
            y: f64::from(y) + 0.5,
            ..Picked::default()
        })
    );
    assert_eq!(shell.state.highlight, None);
    let shown = window.uploaded(width, height).unwrap();
    let marked = shown
        .chunks(3)
        .filter(|p| is_highlight([p[0], p[1], p[2]], &tokens))
        .count();
    assert_eq!(marked, 0, "nothing is highlighted any more");

    // A drag is not a click: it orbits and picks nothing.
    let from = rect.center();
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(from)]);
    window.button(&mut shell, &mut session, from, true);
    for step in 1..=5 {
        let at = from + vec2(12.0 * step as f32, 0.0);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    }
    window.button(&mut shell, &mut session, from + vec2(60.0, 0.0), false);
    window.idle(&mut shell, &mut session, 2);
    assert_ne!(shell.state.camera, camera);
    assert_eq!(shell.state.picked.as_ref().unwrap().serial, serial);

    // A click on a view button turns the camera and picks nothing.
    let before = shell.state.camera;
    // (Changed 2026-10-16: the button is found where the shell says it
    // drew it; the controls' plates moved.)
    let iso = shell
        .camera_hits()
        .iter()
        .find(|(name, _)| *name == "iso")
        .expect("the iso button")
        .1
        .center();
    window.click(&mut shell, &mut session, iso);
    assert_ne!(
        shell.state.camera, before,
        "the iso button turned the camera"
    );
    assert_eq!(shell.state.picked.as_ref().unwrap().serial, serial);

    // A click outside the viewport (the tree) picks nothing either.
    window.click(&mut shell, &mut session, pos2(rect.left() - 30.0, 400.0));
    assert_eq!(shell.state.picked.as_ref().unwrap().serial, serial);
}

#[test]
fn ui_pick_says_what_the_viewport_shows_at_a_pixel() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    for (view, projection) in [
        ("iso", "perspective"),
        ("top", "orthographic"),
        ("front", "perspective"),
        ("back", "orthographic"),
    ] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"view": view, "camera": {"projection": projection}}),
        );
        call(&mut shell, &mut session, "ui.fit", json!({}));
        let [width, height] = shell.viewport_size();
        let (scene, frame) = id_buffer(&mut shell, &mut session);
        let faces = interiors(&frame);
        assert!(faces.len() >= 4, "{view}: {} faces", faces.len());
        for (&id, pixels) in &faces {
            let (feature, name, index) = inspect_name(&mut session, &scene, id);
            for (x, y) in [
                pixels[0],
                pixels[pixels.len() / 2],
                pixels[pixels.len() - 1],
            ] {
                let picked = call(
                    &mut shell,
                    &mut session,
                    "ui.pick",
                    json!({"x": f64::from(x) + 0.5, "y": f64::from(y) + 0.5}),
                );
                let context = format!("{view}: pixel ({x}, {y}): {picked}");
                assert_eq!(picked["hit"], true, "{context}");
                assert_eq!(picked["face"], name.as_str(), "{context}");
                assert_eq!(picked["feature"], feature.as_str(), "{context}");
                assert_eq!(picked["faceIndex"], index, "{context}");
                assert_eq!(
                    (&picked["width"], &picked["height"]),
                    (&json!(width), &json!(height))
                );
                // The same pixel of a screenshot twice as large.
                let doubled = call(
                    &mut shell,
                    &mut session,
                    "ui.pick",
                    json!({"x": f64::from(x) * 2.0 + 1.0, "y": f64::from(y) * 2.0 + 1.0,
                           "width": width * 2, "height": height * 2}),
                );
                assert_eq!(doubled["face"], name.as_str(), "{context}");
            }
        }
        // Asking is not clicking.
        assert_eq!(shell.state.picked, None);
        assert_eq!(shell.state.highlight, None);
    }

    // Nothing there, and not in the viewport: answers.
    let [width, height] = shell.viewport_size();
    let (_, frame) = id_buffer(&mut shell, &mut session);
    let empty = frame.ids.iter().position(|&id| id == NO_FACE).unwrap() as u32;
    let nothing = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": f64::from(empty % width) + 0.5, "y": f64::from(empty / width) + 0.5}),
    );
    assert_eq!(
        (&nothing["hit"], &nothing["outside"]),
        (&json!(false), &json!(false))
    );
    assert!(nothing.get("face").is_none());
    for (x, y) in [
        (-1.0, 5.0),
        (5.0, -0.5),
        (f64::from(width), 5.0),
        (5.0, f64::from(height)),
        (1e300, -1e300),
        (f64::MAX, f64::MAX),
    ] {
        let out = call(&mut shell, &mut session, "ui.pick", json!({"x": x, "y": y}));
        assert_eq!(
            (&out["hit"], &out["outside"]),
            (&json!(false), &json!(true)),
            "({x}, {y})"
        );
        // As a click it is a pick of nothing.
        let out = call(
            &mut shell,
            &mut session,
            "ui.pick",
            json!({"x": x, "y": y, "select": true}),
        );
        assert_eq!(out["hit"], false);
        assert_eq!(out["picked"]["face"], json!(null));
        assert_eq!(out["highlight"], json!(null));
    }
    // Every pixel on an edge of a face or on the silhouette: an answer.
    let inner: std::collections::BTreeSet<(u32, u32)> =
        interiors(&frame).values().flatten().copied().collect();
    let mut border = 0;
    for y in (0..height).step_by(3) {
        for x in (0..width).step_by(3) {
            if frame.ids[(y * width + x) as usize] == NO_FACE || inner.contains(&(x, y)) {
                continue;
            }
            border += 1;
            let picked = call(
                &mut shell,
                &mut session,
                "ui.pick",
                json!({"x": f64::from(x) + 0.5, "y": f64::from(y) + 0.5}),
            );
            assert_eq!(picked["hit"], true, "({x}, {y})");
        }
    }
    assert!(border > 100, "{border}");

    // With select it is a click.
    let (scene, frame) = id_buffer(&mut shell, &mut session);
    let faces = interiors(&frame);
    // Changed 2026-10-13 (edges under the pointer): it was the middle
    // pixel of the first face's inside, which is within the pick radius of
    // an edge of that face, and a click there is now a click on the edge.
    // It is the first face that has pixels further from every edge than
    // the radius, at the middle one of those.
    let (id, (x, y)) = faces
        .iter()
        .find_map(|(&id, pixels)| {
            let clear = clear_of_edges(&frame, pixels);
            clear.get(clear.len() / 2).map(|&at| (id, at))
        })
        .unwrap();
    let (feature, name, _) = inspect_name(&mut session, &scene, id);
    let before = shell.state.picked.clone().unwrap().serial;
    let clicked = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": f64::from(x) + 0.5, "y": f64::from(y) + 0.5, "select": true}),
    );
    assert_eq!(clicked["picked"]["serial"], before + 1);
    assert_eq!(clicked["picked"]["face"], name.as_str());
    assert_eq!(
        clicked["highlight"],
        json!({"feature": feature, "face": name})
    );
    assert_eq!(shell.state.highlight.as_ref().unwrap().face, name);

    // What is wrong is said, and changes nothing.
    let state = shell.state.clone();
    for (params, part) in [
        (json!({}), "missing parameter `x`"),
        (json!({"x": 1}), "missing parameter `y`"),
        (json!({"x": null, "y": 1}), "missing parameter `x`"),
        (json!({"x": "1", "y": 1}), "`x` must be a finite number"),
        (json!({"x": 1, "y": [1]}), "`y` must be a finite number"),
        (json!({"x": 1, "y": 1, "width": 0}), "`width`"),
        (json!({"x": 1, "y": 1, "height": 1e9}), "`height`"),
        (json!({"x": 1, "y": 1, "select": "yes"}), "`select`"),
        (
            json!({"x": 1, "y": 1, "face": "x"}),
            "unknown parameter `face`",
        ),
        (json!([1, 2]), "must be a JSON object"),
    ] {
        let error = refused(&mut shell, &mut session, "ui.pick", params.clone());
        assert!(error.contains(part), "{params}: {error}");
        assert_eq!(shell.state, state);
    }

    // No document: an error that says so, for a pick and for a highlight.
    run(&mut session, "file.close", json!({"discard": true}));
    let error = refused(&mut shell, &mut session, "ui.pick", json!({"x": 1, "y": 1}));
    assert!(error.contains("no document is open"), "{error}");
    let error = refused(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": name}),
    );
    assert!(error.contains("no document is open"), "{error}");
    // And what was pointed at went with the document.
    assert_eq!(shell.state.picked, None);
    assert_eq!(shell.state.highlight, None);
    // Removing a highlight that is not there is fine.
    let cleared = call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": null}),
    );
    assert_eq!(cleared["highlight"], json!(null));
}

#[test]
fn ui_highlight_shows_a_face_by_its_name_and_clears_it() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let tokens = shell.tokens;
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    let (width, height) = (480_u32, 360_u32);
    let shot = |shell: &mut Shell, session: &mut Session| -> Vec<u8> {
        let result = call(
            shell,
            session,
            "ui.screenshot",
            json!({"width": width, "height": height, "scale": 1.0}),
        );
        let png = base64_decode(result["base64"].as_str().unwrap());
        let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
            .read_info()
            .unwrap();
        let mut rgb = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut rgb).unwrap();
        rgb
    };
    let plain = shot(&mut shell, &mut session);
    assert_eq!(
        plain
            .chunks(3)
            .filter(|p| is_highlight([p[0], p[1], p[2]], &tokens))
            .count(),
        0
    );
    // The id buffer of that screenshot.
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let frame = viewport::render_bodies(
        Some(&scene),
        &shell.state.camera,
        Quality {
            width,
            height,
            ssaa: 1,
        },
        &tokens,
    )
    .unwrap();
    let faces = interiors(&frame);
    let inspect = run(&mut session, "document.inspect", json!({}));
    let mut shown = 0;
    for body in inspect["bodies"].as_array().unwrap() {
        let feature = body["feature"].as_str().unwrap();
        let scene_body = scene.bodies.iter().find(|b| b.feature == feature).unwrap();
        for (index, name) in body["faceNames"].as_array().unwrap().iter().enumerate() {
            let name = name.as_str().unwrap();
            let result = call(
                &mut shell,
                &mut session,
                "ui.highlight",
                json!({"face": name, "feature": feature}),
            );
            assert_eq!(
                result["highlight"],
                json!({"feature": feature, "face": name})
            );
            assert_eq!(result["name"], body["name"]);
            assert!(result["faces"].as_u64().unwrap() >= 1);
            assert!(result.get("facesNotShown").is_none(), "{result}");
            assert_eq!(
                shell.state.highlight,
                Some(FaceRef {
                    feature: Some(feature.to_string()),
                    face: name.to_string()
                })
            );
            let marked = shot(&mut shell, &mut session);
            // Well inside every face: the highlight's colour exactly on
            // the faces of that name, the plain picture everywhere else.
            let id = scene_body.first_face_id + index as u32;
            let named: Vec<u32> = scene.faces_named(Some(feature), name).unwrap().ids;
            assert!(named.contains(&id));
            for (&other, pixels) in &faces {
                if named.contains(&other) {
                    let marked = share(&marked, width, pixels, &tokens);
                    assert!(marked > 0.9, "{name}: face {other}: {marked}");
                } else {
                    for &(x, y) in pixels {
                        assert_eq!(
                            pixel(&marked, width, x, y),
                            pixel(&plain, width, x, y),
                            "{name}: face {other} at ({x}, {y})"
                        );
                    }
                }
            }
            if faces.contains_key(&id) {
                shown += 1;
                if shown == 3 {
                    evidence("pointing-highlight-cpu.png", width, height, &marked);
                }
            }
        }
    }
    assert!(shown >= 8, "{shown} highlighted faces were in view");

    // By the body's name, by its id as a number, and without a feature
    // while one body has the name.
    let post = scene.bodies.iter().find(|b| b.name == "Post").unwrap();
    let name = post.face_names[0].clone();
    for feature in [json!("Post"), json!(post.feature), json!(null)] {
        let result = call(
            &mut shell,
            &mut session,
            "ui.highlight",
            json!({"face": name, "feature": feature}),
        );
        assert_eq!(result["highlight"]["feature"], post.feature.as_str());
    }
    let digits: u64 = post.feature.trim_start_matches('F').parse().unwrap();
    let result = call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": name, "feature": digits}),
    );
    assert_eq!(result["highlight"]["feature"], post.feature.as_str());
    // ui.set does the same without the checks, and null clears.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"highlight": null}),
    );
    assert_eq!(shell.state.highlight, None);
    assert_eq!(shot(&mut shell, &mut session), plain);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"highlight": {"face": name}}),
    );
    assert_eq!(shell.state.highlight.as_ref().unwrap().face, name);
    assert_ne!(shot(&mut shell, &mut session), plain);
    // A name nobody has set through ui.set is dropped, not kept.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"highlight": {"face": "F99:face(0)"}}),
    );
    assert_eq!(shell.state.highlight, None);

    // What is wrong is said and leaves the highlight as it was.
    call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": name}),
    );
    let state = shell.state.clone();
    for (params, part) in [
        (json!({}), "missing parameter `face`"),
        (json!({"face": 3}), "`face` must be a face name or null"),
        (json!({"face": "top"}), "is not a name"),
        (json!({"face": ""}), "is not a name"),
        (json!({"face": "F99:face(0)"}), "no body has a face"),
        (
            json!({"face": name, "feature": "Nobody"}),
            "no body of a feature `Nobody`",
        ),
        (
            json!({"face": name, "feature": "Drilled"}),
            "`Drilled` has no face",
        ),
        (json!({"face": name, "feature": [1]}), "`feature` must be"),
        (json!({"face": name, "feature": -1}), "`feature` must be"),
        (
            json!({"face": null, "feature": "Post"}),
            "nothing to highlight",
        ),
        (
            json!({"face": name, "color": "red"}),
            "unknown parameter `color`",
        ),
        (json!("F1:face(0)"), "must be a JSON object"),
        (json!({"face": "F1:face(".repeat(20_000)}), "is not a name"),
    ] {
        let error = refused(&mut shell, &mut session, "ui.highlight", params.clone());
        assert!(error.contains(part), "{:.80}: {error}", params.to_string());
        assert!(error.len() < 600, "{}", error.len());
        assert_eq!(shell.state, state);
    }

    // The highlight follows the face through an edit that keeps it, and
    // goes when the face goes.
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Post", "height": 45}),
    );
    shell.sync(&mut session);
    assert_eq!(shell.state.highlight.as_ref().unwrap().face, name);
    let taller = shot(&mut shell, &mut session);
    assert!(
        taller
            .chunks(3)
            .filter(|p| is_highlight([p[0], p[1], p[2]], &tokens))
            .count()
            > 50
    );
    run(&mut session, "feature.delete", json!({"feature": "Post"}));
    shell.sync(&mut session);
    assert_eq!(shell.state.highlight, None);
    run(&mut session, "edit.undo", json!({}));
    let cleared = call(
        &mut shell,
        &mut session,
        "ui.highlight",
        json!({"face": null}),
    );
    assert_eq!(cleared, json!({"highlight": null, "faces": 0}));
}

fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => panic!("not base64"),
    };
    let mut out = Vec::new();
    let bytes: Vec<u8> = text.bytes().filter(|&c| c != b'=').collect();
    for chunk in bytes.chunks(4) {
        let mut acc = 0_u32;
        for &c in chunk {
            acc = (acc << 6) | u32::from(value(c));
        }
        acc <<= 6 * (4 - chunk.len());
        let all = acc.to_be_bytes();
        out.extend(&all[1..chunk.len()]);
    }
    out
}
