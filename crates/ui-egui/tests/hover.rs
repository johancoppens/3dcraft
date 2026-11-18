//! What a click would select lights up under the pointer (the UX round of
//! 2026-11-18, V7): `ui.pick {x, y, hover: true}` places the pointer
//! without a click, and `ui.get` › `hovered` says what lies there, shaped
//! as `state.picked`: the same face, edge or vertex `ui.pick` gives at
//! that position. The mouse resting over the model does the same, and
//! the window outlines what a click would select; what that costs on a
//! model of thousands of faces is printed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Vec3;
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::viewport::LineKind;
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, PointerButton, Pos2};
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

fn refused(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> String {
    match shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
    {
        Ok(value) => panic!("{method} {params} should fail, returned {value}"),
        Err(e) => e,
    }
}

/// A plate 60 × 40 × 8 seen from the iso view.
fn plate() -> (Session, Shell) {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    (session, shell)
}

/// The pixel of the viewport `p` is seen at.
fn pixel_of(shell: &Shell, p: Vec3) -> [f64; 2] {
    let [width, height] = shell.viewport_size();
    let projector = Projector::new(
        &shell.state.camera.to_render(),
        f64::from(width),
        f64::from(height),
    )
    .unwrap();
    let at = projector.project(p).unwrap();
    [at.x, at.y]
}

/// The first of `points` where `ui.pick` gives `kind`, and its answer.
fn where_kind(
    shell: &mut Shell,
    session: &mut Session,
    points: &[Vec3],
    kind: &str,
) -> ([f64; 2], Value) {
    for &p in points {
        let [x, y] = pixel_of(shell, p);
        let picked = call(shell, session, "ui.pick", json!({"x": x, "y": y}));
        if picked["kind"] == kind {
            return ([x, y], picked);
        }
    }
    panic!("no {kind} found at {points:?}");
}

/// `hovered` names what a click on `picked` (a `ui.pick` answer) selects:
/// its vertex, else its edge, else its face. (The pointer of a window is
/// a position in points, an `f32`: beside a silhouette the face under it
/// may be another than under the exact position, the vertex or the edge
/// it points at the same.)
fn same_target(hovered: &Value, picked: &Value, context: &str) {
    assert_eq!(hovered["kind"], picked["kind"], "{context}: kind");
    match picked["kind"].as_str() {
        Some("face") => {
            for key in ["feature", "name", "face", "faceIndex"] {
                assert_eq!(hovered[key], picked[key], "{context}: {key}");
            }
        }
        Some(entity) => {
            for key in ["feature", "body", "name", "index", "faces"] {
                assert_eq!(
                    hovered[entity][key], picked[entity][key],
                    "{context}: {entity}.{key}"
                );
            }
        }
        None => panic!("{context}: nothing picked"),
    }
}

/// `hovered` names what `picked` (a `ui.pick` answer) names.
fn same_thing(hovered: &Value, picked: &Value, context: &str) {
    for key in ["kind", "feature", "name", "face", "faceIndex"] {
        assert_eq!(hovered[key], picked[key], "{context}: {key}");
    }
    for entity in ["edge", "vertex"] {
        for key in ["feature", "body", "name", "index", "faces"] {
            assert_eq!(
                hovered[entity][key], picked[entity][key],
                "{context}: {entity}.{key}"
            );
        }
    }
}

#[test]
fn hovered_is_what_ui_pick_gives_at_the_same_position() {
    let (mut session, mut shell) = plate();
    let get = |shell: &mut Shell, session: &mut Session| {
        call(shell, session, "ui.get", json!({}))["hovered"].clone()
    };
    assert_eq!(get(&mut shell, &mut session), Value::Null, "nothing yet");

    let v = |x, y, z| Vec3::new(x, y, z);
    let corners = [
        v(0.0, 0.0, 8.0),
        v(60.0, 0.0, 8.0),
        v(60.0, 40.0, 8.0),
        v(0.0, 40.0, 8.0),
    ];
    let middles = [
        v(30.0, 0.0, 8.0),
        v(60.0, 20.0, 8.0),
        v(30.0, 40.0, 8.0),
        v(0.0, 20.0, 8.0),
    ];
    let face = where_kind(&mut shell, &mut session, &[v(30.0, 20.0, 8.0)], "face");
    let edge = where_kind(&mut shell, &mut session, &middles, "edge");
    let vertex = where_kind(&mut shell, &mut session, &corners, "vertex");
    let mut serial = 0;
    for (what, ([x, y], picked)) in [("face", face), ("edge", edge), ("vertex", vertex)] {
        let placed = call(
            &mut shell,
            &mut session,
            "ui.pick",
            json!({"x": x, "y": y, "hover": true}),
        );
        let context = format!("the {what} at ({x}, {y}): {placed}");
        // The answer is the pick's, with `hovered` beside it.
        assert_eq!(placed["kind"], what, "{context}");
        same_thing(&placed["hovered"], &picked, &context);
        let hovered = get(&mut shell, &mut session);
        assert_eq!(hovered, placed["hovered"], "{context}");
        serial += 1;
        assert_eq!(hovered["serial"], serial, "{context}");
        assert_eq!(hovered["x"], x, "{context}");
        assert_eq!(hovered["y"], y, "{context}");
        // Placing the pointer is not a click: nothing is picked,
        // highlighted or selected.
        assert_eq!(shell.state.picked, None, "{context}");
        assert_eq!(shell.state.highlight, None, "{context}");
        assert_eq!(shell.state.highlight_edge, None, "{context}");
        assert_eq!(shell.state.highlight_vertex, None, "{context}");
        let selection = call(&mut shell, &mut session, "ui.get", json!({}))["selection"].clone();
        assert_eq!(selection["count"], 0, "{context}: {selection}");
    }

    // A change of the view state keeps it; it is not in the state's JSON
    // and cannot be set.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"notice": "kept"}),
    );
    assert_eq!(get(&mut shell, &mut session)["kind"], "vertex");
    let state = call(&mut shell, &mut session, "ui.get", json!({}))["state"].clone();
    assert!(state.get("hovered").is_none(), "{state}");
    let e = refused(&mut shell, &mut session, "ui.set", json!({"hovered": null}));
    assert!(e.contains("hovered"), "{e}");

    // Outside the model: nothing.
    let placed = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": 1.5, "y": 1.5, "hover": true}),
    );
    assert_eq!(placed["hit"], false, "{placed}");
    assert_eq!(placed["hovered"], Value::Null, "{placed}");
    assert_eq!(get(&mut shell, &mut session), Value::Null);
}

#[test]
fn hover_on_a_picture_of_another_size_is_the_viewports_place() {
    let (mut session, mut shell) = plate();
    let [width, height] = shell.viewport_size();
    let [x, y] = pixel_of(&shell, Vec3::new(30.0, 20.0, 8.0));
    let placed = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x * 2.0, "y": y * 2.0, "width": width * 2, "height": height * 2,
               "hover": true}),
    );
    assert_eq!(placed["kind"], "face", "{placed}");
    let hovered = &placed["hovered"];
    // In the viewport's own pixels, as a click is.
    assert!(
        (hovered["x"].as_f64().unwrap() - x).abs() < 1e-9,
        "{hovered}"
    );
    assert!(
        (hovered["y"].as_f64().unwrap() - y).abs() < 1e-9,
        "{hovered}"
    );
}

#[test]
fn hover_and_select_are_not_given_together() {
    let (mut session, mut shell) = plate();
    for params in [
        json!({"x": 10, "y": 10, "hover": true, "select": true}),
        json!({"x": 10, "y": 10, "hover": 1}),
        json!({"x": 10, "y": 10, "hover": true, "shift": true}),
    ] {
        let e = refused(&mut shell, &mut session, "ui.pick", params.clone());
        assert!(e.starts_with("ui.pick"), "{params}: {e}");
    }
    assert_eq!(shell.state.hovered, None);
}

/// A window of a fixed size, fed one frame at a time, a sixtieth of a
/// second apart.
struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: egui::vec2(width, height),
            time: 0.0,
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
        output.textures_delta.clear();
    }

    fn frames(&mut self, shell: &mut Shell, session: &mut Session, count: usize) {
        for _ in 0..count {
            self.frame(shell, session, Vec::new());
        }
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
}

/// The plate in a window of 1000 × 700, framed from the iso view.
fn plate_in_a_window() -> (Session, Shell, Window) {
    let (mut session, _) = plate();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1000.0, 700.0);
    window.frames(&mut shell, &mut session, 3);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    window.frames(&mut shell, &mut session, 3);
    (session, shell, window)
}

/// The face, an edge and a vertex of the plate, each where `ui.pick` gives
/// it, in viewport pixels (here points), with its answer.
fn three_things(shell: &mut Shell, session: &mut Session) -> Vec<(&'static str, [f64; 2], Value)> {
    let v = |x, y, z| Vec3::new(x, y, z);
    let corners = [
        v(0.0, 0.0, 8.0),
        v(60.0, 0.0, 8.0),
        v(60.0, 40.0, 8.0),
        v(0.0, 40.0, 8.0),
    ];
    let middles = [
        v(30.0, 0.0, 8.0),
        v(60.0, 20.0, 8.0),
        v(30.0, 40.0, 8.0),
        v(0.0, 20.0, 8.0),
    ];
    let (face_at, face) = where_kind(shell, session, &[v(30.0, 20.0, 8.0)], "face");
    let (edge_at, edge) = where_kind(shell, session, &middles, "edge");
    let (vertex_at, vertex) = where_kind(shell, session, &corners, "vertex");
    vec![
        ("face", face_at, face),
        ("edge", edge_at, edge),
        ("vertex", vertex_at, vertex),
    ]
}

/// The mouse resting on the face, an edge and a vertex: after a frame or
/// two `hovered` is what `ui.pick` gives there, it is outlined in the
/// picture (a face by its boundary, an edge along itself, a vertex as a
/// dot, in the kinds of line of the hover), and a click there selects it.
#[test]
fn the_resting_mouse_outlines_what_a_click_selects() {
    let (mut session, mut shell, mut window) = plate_in_a_window();
    let rect = shell.viewport_rect().unwrap();
    for (what, [x, y], picked) in three_things(&mut shell, &mut session) {
        let at = egui::pos2(rect.left() + x as f32, rect.top() + y as f32);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        window.frames(&mut shell, &mut session, 4);
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        let hovered = &got["hovered"];
        let context = format!("the {what} at ({x}, {y}): {hovered}");
        assert_eq!(hovered["kind"], what, "{context}");
        same_target(hovered, &picked, &context);
        // Within a hundredth of a pixel: the pointer's place in points
        // and back.
        assert!(
            (hovered["x"].as_f64().unwrap() - x).abs() < 0.01,
            "{context}"
        );
        let (overlay, _) = shell.overlay();
        match what {
            "face" => {
                let outline = overlay
                    .lines
                    .iter()
                    .filter(|l| l.kind == LineKind::Hover)
                    .count();
                // The top of the plate: its four sides, in sight.
                assert!(outline >= 4, "{context}: {outline} lines");
            }
            "edge" => assert!(
                overlay.lines.iter().any(|l| l.kind == LineKind::HoverEdge),
                "{context}"
            ),
            _ => assert!(
                overlay.points.iter().any(|p| p.kind == LineKind::HoverEdge),
                "{context}"
            ),
        }
        // Nothing is selected or lit by resting on it.
        assert_eq!(got["selection"]["count"], 0, "{context}");
        // A click selects what was outlined.
        window.button(&mut shell, &mut session, at, true);
        window.button(&mut shell, &mut session, at, false);
        window.frames(&mut shell, &mut session, 2);
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        assert_eq!(
            got["selection"]["count"], 1,
            "{context}: {}",
            got["selection"]
        );
        assert_eq!(got["selection"]["items"][0]["kind"], what, "{context}");
        call(
            &mut shell,
            &mut session,
            "ui.select",
            json!({"mode": "clear"}),
        );
    }
}

/// Nothing is outlined while a button is down, once the mouse has left the
/// viewport, or on a part of the view a click does not select in; a
/// pointer an agent placed stays until the mouse moves over the viewport.
#[test]
fn the_outline_goes_with_a_button_down_the_mouse_gone_and_the_mouse_moving_in() {
    let (mut session, mut shell, mut window) = plate_in_a_window();
    let rect = shell.viewport_rect().unwrap();
    let things = three_things(&mut shell, &mut session);
    let (_, [x, y], _) = &things[0];
    let at = egui::pos2(rect.left() + *x as f32, rect.top() + *y as f32);
    let hovered = |shell: &mut Shell, session: &mut Session| {
        call(shell, session, "ui.get", json!({}))["hovered"].clone()
    };
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(hovered(&mut shell, &mut session)["kind"], "face");
    // A button down (a drag about to turn the view): no outline.
    window.button(&mut shell, &mut session, at, true);
    assert_eq!(hovered(&mut shell, &mut session), Value::Null);
    assert!(
        !shell
            .overlay()
            .0
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Hover)
    );
    window.button(&mut shell, &mut session, at, false);
    call(
        &mut shell,
        &mut session,
        "ui.select",
        json!({"mode": "clear"}),
    );
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(hovered(&mut shell, &mut session)["kind"], "face");
    // The mouse gone: nothing.
    window.frame(&mut shell, &mut session, vec![Event::PointerGone]);
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(hovered(&mut shell, &mut session), Value::Null);
    // Placed by an agent on the edge: there frame after frame while the
    // mouse is away, also in the window's picture.
    let (_, [ex, ey], _) = &things[1];
    call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": ex, "y": ey, "hover": true}),
    );
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(hovered(&mut shell, &mut session)["kind"], "edge");
    assert!(
        shell
            .overlay()
            .0
            .lines
            .iter()
            .any(|l| l.kind == LineKind::HoverEdge)
    );
    // The mouse moving in takes over.
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(hovered(&mut shell, &mut session)["kind"], "face");
    // Off the model: nothing.
    let off = egui::pos2(rect.left() + 5.0, rect.top() + 5.0);
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(off)]);
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(hovered(&mut shell, &mut session), Value::Null);
}

/// The picture `ui.screenshot` makes shows the outline where an agent
/// placed the pointer: in the selection's colour, along the edge; without
/// it the picture is the one before, bit for bit.
#[test]
fn the_screenshot_shows_the_outline_of_the_placed_pointer() {
    let (mut session, mut shell) = plate();
    let things = three_things(&mut shell, &mut session);
    let shot = |shell: &mut Shell, session: &mut Session| {
        let result = call(shell, session, "ui.screenshot", json!({"scale": 1.0}));
        result["base64"].as_str().unwrap().to_string()
    };
    let before = shot(&mut shell, &mut session);
    let (_, [x, y], _) = &things[1];
    call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x, "y": y, "hover": true}),
    );
    let outlined = shot(&mut shell, &mut session);
    assert_ne!(outlined, before, "the edge is outlined");
    // Off the model again: the picture before.
    call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": 1.5, "y": 1.5, "hover": true}),
    );
    assert_eq!(shot(&mut shell, &mut session), before);
}

/// What a pick under the pointer costs on a model of thousands of faces
/// (20 × 20 drilled cubes: 2800 faces, over 40 000 triangles), at a grid
/// of places over the model, as the window's pointer makes it; printed
/// for the record (the UX round's budget: 4 ms to follow the pointer,
/// otherwise once it rests, `shell/hover.rs`).
#[test]
fn hover_cost_on_thousands_of_faces() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [8, 8, 8], "name": "Cube"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 12, "position": [4, -2, 4], "axis": [0, 1, 0], "name": "Bore"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Cube", "tool": "Bore", "name": "Bored"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bored", "count": 20, "step": [10, 0, 0], "name": "Row"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Row", "count": 20, "step": [0, 0, 10], "name": "Wall"}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    call(&mut shell, &mut s, "ui.fit", json!({"view": "iso"}));
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    let triangles = got["scene"]["triangles"].as_u64().unwrap();
    assert!(triangles > 40_000, "{}", got["scene"]);
    let scene = s.scene_with(shell.scene_quality()).unwrap().unwrap();
    let [width, height] = shell.viewport_size();
    let mut costs = Vec::new();
    let mut kinds = std::collections::BTreeMap::new();
    for round in 0..2 {
        for j in 1..12 {
            for i in 1..12 {
                let x = f64::from(width) * f64::from(i) / 12.0 + 0.5;
                let y = f64::from(height) * f64::from(j) / 12.0 + 0.5;
                let started = std::time::Instant::now();
                let found = shell
                    .pick(Some(scene.as_ref()), x, y, None, None, false)
                    .unwrap();
                costs.push(started.elapsed().as_secs_f64() * 1000.0);
                if round == 0 {
                    *kinds.entry(format!("{:?}", found.kind())).or_insert(0) += 1;
                }
            }
        }
    }
    costs.sort_by(f64::total_cmp);
    let at = |q: f64| costs[((costs.len() - 1) as f64 * q) as usize];
    // The outline of what lies under the middle, and the lines the
    // window's device is given again when it changes.
    let (w, h) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    let started = std::time::Instant::now();
    call(
        &mut shell,
        &mut s,
        "ui.pick",
        json!({"x": w, "y": h, "hover": true}),
    );
    let placed = started.elapsed().as_secs_f64() * 1000.0;
    let marks = shell.marks(Some(scene.as_ref()));
    let started = std::time::Instant::now();
    let segments = craft3d_ui_egui::direct::segments_marked(
        Some(scene.as_ref()),
        None,
        &shell.tokens,
        1.0,
        &marks,
        false,
    );
    let lines = started.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "COST hover pick: {triangles} triangles, {} picks over the model ({kinds:?}): \
         median {:.2} ms, 90 % {:.2} ms, most {:.2} ms; placed with its outline {placed:.2} ms; \
         the direct path's {} lines made again {lines:.2} ms (test profile)",
        costs.len(),
        at(0.5),
        at(0.9),
        at(1.0),
        segments.len()
    );
    assert!(at(0.5) < 1_000.0);
}

/// On a note's pin a click is the pin's: nothing of the model is outlined
/// there, with the mouse on it or the pointer an agent placed on it, and
/// `hovered` names the note (the verdict's fix 10; the pin's ring, which
/// reads `Shell::hover_point`, is the notes' to test).
#[test]
fn on_a_notes_pin_nothing_of_the_model_is_outlined() {
    let (mut session, mut shell, mut window) = plate_in_a_window();
    let note = run(
        &mut session,
        "note.add",
        json!({"text": "2 mm thicker here", "on": "Plate", "point": [30, 20, 8]}),
    );
    window.frames(&mut shell, &mut session, 4);
    let pin = shell.note_markers().0.first().expect("the pin is drawn").1;
    let rect = shell.viewport_rect().unwrap();
    let (x, y) = (
        f64::from(pin.x - rect.left()),
        f64::from(pin.y - rect.top()),
    );
    // Under the pin lies the plate's top: placed there, the note.
    let named = |hovered: &Value| hovered["kind"] == "note" && hovered["id"] == note["id"];
    let picked = call(&mut shell, &mut session, "ui.pick", json!({"x": x, "y": y}));
    assert_eq!(picked["hit"], true, "{picked}");
    let placed = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x, "y": y, "hover": true}),
    );
    assert!(named(&placed["hovered"]), "{placed}");
    window.frames(&mut shell, &mut session, 3);
    let hovered = |shell: &mut Shell, session: &mut Session| {
        call(shell, session, "ui.get", json!({}))["hovered"].clone()
    };
    assert!(named(&hovered(&mut shell, &mut session)));
    assert_eq!(shell.state.hovered, None, "nothing of the model");
    assert!(
        !shell
            .overlay()
            .0
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Hover)
    );
    // The mouse on it: the note too; beside it, the plate's top.
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(pin)]);
    window.frames(&mut shell, &mut session, 4);
    assert!(named(&hovered(&mut shell, &mut session)));
    assert_eq!(shell.state.hovered, None, "nothing of the model");
    let beside = egui::pos2(pin.x + 40.0, pin.y + 40.0);
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(beside)]);
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(hovered(&mut shell, &mut session)["kind"], "face");
}

fn pixels(result: &Value) -> Vec<u8> {
    let text = result["base64"].as_str().unwrap();
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
    let mut png = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let bits = chunk.iter().fold(0_u32, |acc, &c| {
            (acc << 6) | if c == b'=' { 0 } else { value(c) }
        });
        let bits = bits << (6 * (4 - chunk.len()));
        png.extend_from_slice(&bits.to_be_bytes()[1..4 - pad]);
    }
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut rgb = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgb).unwrap();
    rgb.truncate(info.buffer_size());
    rgb
}

/// The hover's amber wears a dark casing, as the dimension line does (the
/// verdict's fix 7), so it reads on a body of any colour: copper brown,
/// near white, near black, the amber itself and slate blue. On each, the
/// pixels the outline of an edge changes hold both the amber and the
/// casing's ink.
#[test]
fn the_hover_outline_reads_on_any_body_colour() {
    let apart = |a: &[u8], b: [u8; 3]| -> u32 {
        a.iter()
            .zip(b)
            .map(|(&x, y)| u32::from(x.abs_diff(y)))
            .sum()
    };
    for color in ["#8a5a3c", "#f0f0f0", "#202020", "#d9862c", "#5b7299"] {
        let (mut session, mut shell) = plate();
        run(
            &mut session,
            "body.appearance",
            json!({"body": "Plate", "color": color}),
        );
        let tokens = shell.tokens;
        let things = three_things(&mut shell, &mut session);
        let (_, [x, y], _) = &things[1];
        let shot = |shell: &mut Shell, session: &mut Session| {
            pixels(&call(
                shell,
                session,
                "ui.screenshot",
                json!({"scale": 1.0}),
            ))
        };
        let before = shot(&mut shell, &mut session);
        call(
            &mut shell,
            &mut session,
            "ui.pick",
            json!({"x": x, "y": y, "hover": true}),
        );
        let after = shot(&mut shell, &mut session);
        let changed: Vec<&[u8]> = before
            .chunks(3)
            .zip(after.chunks(3))
            .filter(|(a, b)| a != b)
            .map(|(_, b)| b)
            .collect();
        let amber = changed
            .iter()
            .filter(|px| apart(px, tokens.face_highlight) <= 90)
            .count();
        let casing = changed
            .iter()
            .filter(|px| apart(px, tokens.hover_casing) <= 60)
            .count();
        assert!(
            amber >= 10 && casing >= 10,
            "{color}: {} pixels changed, {amber} amber, {casing} of the casing",
            changed.len()
        );
    }
}

/// A thing already selected is not outlined as what a click would select
/// (the review of 2026-11-18): the pointer resting on an edge outlines it;
/// clicked, it is selected and lit, and the hover draws nothing over the
/// selection's light; `hovered` still names it.
#[test]
fn what_is_selected_already_is_not_outlined_again() {
    let (mut session, mut shell, mut window) = plate_in_a_window();
    let rect = shell.viewport_rect().unwrap();
    let things = three_things(&mut shell, &mut session);
    let (_, [x, y], _) = &things[1];
    let at = egui::pos2(rect.left() + *x as f32, rect.top() + *y as f32);
    let hover_lines = |shell: &Shell| {
        shell
            .overlay()
            .0
            .lines
            .iter()
            .filter(|l| matches!(l.kind, LineKind::HoverEdge | LineKind::HoverEdgeCasing))
            .count()
    };
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    window.frames(&mut shell, &mut session, 4);
    assert!(hover_lines(&shell) > 0, "the edge is outlined");
    window.button(&mut shell, &mut session, at, true);
    window.button(&mut shell, &mut session, at, false);
    window.frames(&mut shell, &mut session, 4);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        got["selection"]["items"][0]["kind"], "edge",
        "{}",
        got["selection"]
    );
    assert_eq!(got["hovered"]["kind"], "edge", "{}", got["hovered"]);
    assert_eq!(hover_lines(&shell), 0, "no outline over the selection");
    assert!(
        shell
            .overlay()
            .0
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Highlight)
    );
    // Taken out of the selection, it is outlined again.
    call(
        &mut shell,
        &mut session,
        "ui.select",
        json!({"mode": "clear"}),
    );
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"highlight": null, "highlightEdge": null, "highlightVertex": null}),
    );
    window.frames(&mut shell, &mut session, 4);
    assert!(hover_lines(&shell) > 0, "outlined again");
}

/// Where a pick under the pointer has cost more than 16 ms on the model
/// shown, the frames make no more of them (the window would stall at each
/// rest); `ui.pick` with `hover` still places one; a model of another size
/// is measured again. Counted, not timed: the costs are given.
#[test]
fn a_pick_too_dear_for_the_model_stops_the_hover() {
    let (mut session, mut shell, mut window) = plate_in_a_window();
    let rect = shell.viewport_rect().unwrap();
    let [x, y] = pixel_of(&shell, Vec3::new(30.0, 20.0, 8.0));
    let on = egui::pos2(rect.left() + x as f32, rect.top() + y as f32);
    let off = egui::pos2(rect.left() + 5.0, rect.top() + 5.0);
    let hovered = |shell: &mut Shell, session: &mut Session| {
        call(shell, session, "ui.get", json!({}))["hovered"].clone()
    };
    // Cheap: the mouse resting on the face picks it.
    shell.note_hover_costs(&[1.0; 15]);
    let picks = shell.hover_picks();
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(on)]);
    window.frames(&mut shell, &mut session, 8);
    assert!(shell.hover_picks() > picks);
    assert_eq!(hovered(&mut shell, &mut session)["kind"], "face");
    // Dear: no pick, nothing outlined.
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(off)]);
    window.frames(&mut shell, &mut session, 4);
    shell.note_hover_costs(&[20.0; 15]);
    let picks = shell.hover_picks();
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(on)]);
    window.frames(&mut shell, &mut session, 8);
    assert_eq!(shell.hover_picks(), picks, "no pick on a model too dear");
    assert_eq!(hovered(&mut shell, &mut session), Value::Null);
    // An agent's pointer still is placed.
    let placed = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x, "y": y, "hover": true}),
    );
    assert_eq!(placed["hovered"]["kind"], "face", "{placed}");
    // A model of another size is measured again: the plate patterned 4
    // times over.
    run(
        &mut session,
        "feature.pattern",
        json!({"source": "Plate", "count": 4, "step": [70, 0, 0], "name": "Plates"}),
    );
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(off)]);
    window.frames(&mut shell, &mut session, 2);
    let picks = shell.hover_picks();
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(on)]);
    window.frames(&mut shell, &mut session, 8);
    assert!(shell.hover_picks() > picks, "measured again");
}
