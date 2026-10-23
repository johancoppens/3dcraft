//! A view per document tab, flying to a feature or the selection, and the
//! feature list following the selection (2026-10-23, R1): the shell
//! without a window, `egui::Context::run_ui` with synthetic input, on a
//! session driven by commands.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Bbox3;
use craft3d_ui_egui::state::{CameraState, ProjectionKind, VIEWS};
use craft3d_ui_egui::{FLY_SECONDS, Shell};
use egui::{Event, PointerButton, Pos2, vec2};
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

/// A plate with a hole through it (`F1` the plate, `F2` the tool, `F3`
/// the cut) and a sketch high above it (`F4`).
fn plate() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [100, 60, 10]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Tool", "position": [20, 30, -5], "radius": 5, "height": 20}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "Hole", "op": "subtract", "target": "Plate", "tool": "Tool"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Above", "offset": 80}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Above", "closed": true, "points": [[60, 40], [90, 40], [90, 55]]}),
    );
    s
}

struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: vec2(width, height),
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

    fn idle(&mut self, shell: &mut Shell, session: &mut Session, frames: usize) {
        for _ in 0..frames {
            self.frame(shell, session, Vec::new());
        }
    }

    /// `clicks` clicks at `at`, one frame apart, after a pause long
    /// enough that the first is not taken with an earlier one.
    fn clicks(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2, clicks: usize) {
        self.time += 1.0;
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        for _ in 0..clicks {
            for pressed in [true, false] {
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
    }

    fn key(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2, key: egui::Key) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        self.frame(
            shell,
            session,
            vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
}

/// Where the list drew the row of `id`.
fn row(shell: &Shell, id: &str) -> egui::Rect {
    shell
        .tree_row_hits()
        .iter()
        .find(|(row, _)| row == id)
        .map(|(_, rect)| *rect)
        .unwrap_or_else(|| panic!("no row {id}"))
}

/// The camera `camera` becomes when it frames `bbox` in the shell's
/// viewport.
fn framed(shell: &Shell, camera: CameraState, bbox: &Bbox3) -> CameraState {
    let [w, h] = shell.viewport_size();
    let mut camera = camera;
    camera.fit(bbox, f64::from(w) / f64::from(h));
    camera.auto_fit = false;
    camera
}

fn iso() -> (f64, f64) {
    let (_, yaw, pitch) = VIEWS[0];
    (yaw, pitch)
}

#[test]
fn every_tab_keeps_its_own_view_and_a_new_one_starts_framed() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [100, 60, 10]}),
    );
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 3);
    let first = shell.state.camera;
    assert!(first.auto_fit);
    assert_eq!(first.target, [50.0, 30.0, 5.0], "framed on the block");
    // Turned, zoomed, orthographic, cut open and see-through.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"yaw": 30, "pitch": 10, "distance": 400, "projection": "orthographic"},
               "section": {"on": true}, "transparency": 0.5, "notice": "both tabs"}),
    );
    let a = shell.state.camera;
    let a_section = shell.state.section.clone();

    // A second document starts framed on its own model, iso,
    // perspective, uncut and opaque.
    run(&mut session, "file.new", json!({}));
    run(
        &mut session,
        "feature.cylinder",
        json!({"radius": 5, "height": 50}),
    );
    window.idle(&mut shell, &mut session, 3);
    let b = shell.state.camera;
    assert_eq!((b.yaw, b.pitch), iso());
    assert_eq!(b.projection, ProjectionKind::Perspective);
    assert!(b.auto_fit);
    assert_eq!(b.target, [0.0, 0.0, 25.0]);
    assert!(!shell.state.section.on);
    assert_eq!(shell.state.transparency, 0.0);
    // The window's own part is the window's.
    assert_eq!(shell.state.notice, "both tabs");
    // ui.get is the active tab's, in the shape it always had.
    let got = call(&mut shell, &mut session, "ui.get", Value::Null);
    assert_eq!(got["state"]["camera"]["target"], json!([0.0, 0.0, 25.0]));
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"yaw": 100}}),
    );
    let b = shell.state.camera;

    // Back and forth: each comes back as it was left.
    run(&mut session, "file.activate", json!({"document": 0}));
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(shell.state.camera, a);
    assert_eq!(shell.state.section, a_section);
    assert!(shell.state.section.on);
    assert_eq!(shell.state.transparency, 0.5);
    let got = call(&mut shell, &mut session, "ui.get", Value::Null);
    assert_eq!(got["state"]["camera"]["yaw"], json!(30.0));
    assert_eq!(got["state"]["camera"]["projection"], json!("orthographic"));
    run(&mut session, "file.activate", json!({"document": 1}));
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(shell.state.camera, b);
    assert!(!shell.state.section.on);
    // Also when the switch is read by the control channel first, with no
    // frame between.
    run(&mut session, "file.activate", json!({"document": 0}));
    let got = call(&mut shell, &mut session, "ui.get", Value::Null);
    assert_eq!(got["state"]["camera"]["yaw"], json!(30.0));
    run(&mut session, "file.activate", json!({"document": 1}));
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(shell.state.camera, b);

    // Closing the first moves the second to place 0: its view stays.
    run(
        &mut session,
        "file.close",
        json!({"document": 0, "discard": true}),
    );
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(shell.state.camera, b);
    // A third starts fresh, not with the closed one's view.
    run(&mut session, "file.new", json!({}));
    run(&mut session, "feature.sphere", json!({"radius": 3}));
    window.idle(&mut shell, &mut session, 2);
    let c = shell.state.camera;
    assert_eq!((c.yaw, c.pitch), iso());
    assert_eq!(c.projection, ProjectionKind::Perspective);
    assert!(c.auto_fit && !shell.state.section.on);
    // Closing all and starting again: fresh too.
    for _ in 0..2 {
        run(&mut session, "file.close", json!({"discard": true}));
    }
    window.idle(&mut shell, &mut session, 2);
    run(&mut session, "file.new", json!({}));
    run(&mut session, "feature.sphere", json!({"radius": 3}));
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(shell.state.camera.yaw, iso().0);
    assert_eq!(shell.state.transparency, 0.0);
}

#[test]
fn a_double_click_on_a_row_flies_to_what_the_feature_made() {
    let mut session = plate();
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 3);
    let start = shell.state.camera;
    let revision = session.document_revision();
    let tool = row(&shell, "F2").center();

    // One click selects and leaves the camera alone.
    window.clicks(&mut shell, &mut session, tool, 1);
    window.idle(&mut shell, &mut session, 20);
    assert_eq!(shell.state.selected_feature.as_deref(), Some("F2"));
    assert_eq!(shell.state.camera, start);

    // A double click selects it (its first click alone would have taken
    // it out again) and flies to the wall the tool left in the plate.
    window.clicks(&mut shell, &mut session, tool, 2);
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let wall = scene.feature_box("F2");
    let (lo, hi) = wall.corners().unwrap();
    assert!(hi.x - lo.x < 11.0 && hi.z - lo.z < 11.0, "{lo:?} {hi:?}");
    let goal = framed(&shell, start, &wall);
    window.idle(&mut shell, &mut session, 3);
    let midway = shell.state.camera;
    assert_ne!(midway, start, "under way");
    assert_ne!(midway, goal, "not there yet");
    let seconds = FLY_SECONDS;
    window.idle(&mut shell, &mut session, (seconds * 60.0) as usize + 5);
    assert_eq!(shell.state.camera, goal);
    assert!(!shell.state.camera.auto_fit);
    assert!(goal.distance < start.distance * 0.5);
    assert_eq!((goal.yaw, goal.pitch), (start.yaw, start.pitch));
    assert_eq!(shell.state.selected_feature.as_deref(), Some("F2"));
    assert_eq!(shell.selection().items.len(), 1);
    assert_eq!(shell.selection().items[0].feature, "F2");
    // Nothing in the document changed.
    assert_eq!(session.document_revision(), revision);
}

#[test]
fn f_flies_to_the_selection_and_the_list_follows_it() {
    let mut session = plate();
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 3);
    let start = shell.state.camera;
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let body = scene.bodies.iter().find(|b| b.feature == "F3").unwrap();
    let wall = body
        .face_names
        .iter()
        .find(|name| scene.made_by(name).as_deref() == Some("F2"))
        .unwrap()
        .clone();
    // A face of the cut's body that the tool made: the list marks the
    // tool's row, the selected feature stays none.
    call(
        &mut shell,
        &mut session,
        "ui.select",
        json!({"items": [{"kind": "face", "name": wall, "feature": "F3"}]}),
    );
    window.idle(&mut shell, &mut session, 2);
    assert_eq!(shell.made_selection(), Some("F2"));
    let marked: Vec<bool> = shell.tree_rows().iter().map(|r| r.made_selection).collect();
    assert_eq!(marked, [false, true, false, false]);
    assert_eq!(shell.state.selected_feature, None);
    let got = call(&mut shell, &mut session, "ui.get", Value::Null);
    assert_eq!(got["features"][1]["madeSelection"], true);
    assert_eq!(shell.state.camera, start, "selecting moves nothing");

    // F over the viewport flies to it.
    let middle = shell.viewport_rect().unwrap().center();
    window.key(&mut shell, &mut session, middle, egui::Key::F);
    window.idle(&mut shell, &mut session, 30);
    let items = shell.selection().items.clone();
    let goal = framed(&shell, start, &scene.items_box(&items));
    assert_eq!(shell.state.camera, goal);

    // ui.fit frames the same at once, and a feature by its name.
    call(&mut shell, &mut session, "ui.fit", json!({}));
    assert!(shell.state.camera.auto_fit);
    let fitted = call(
        &mut shell,
        &mut session,
        "ui.fit",
        json!({"selection": true}),
    );
    assert_eq!(shell.state.camera, goal);
    assert_eq!(fitted["camera"]["autoFit"], false);
    call(
        &mut shell,
        &mut session,
        "ui.fit",
        json!({"feature": "Tool"}),
    );
    assert_eq!(
        shell.state.camera,
        framed(&shell, start, &scene.feature_box("F2"))
    );

    // Nothing selected: F frames everything, at once, as before.
    call(
        &mut shell,
        &mut session,
        "ui.select",
        json!({"mode": "clear"}),
    );
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(shell.made_selection(), None);
    window.key(&mut shell, &mut session, middle, egui::Key::F);
    assert!(shell.state.camera.auto_fit);
    assert_eq!(shell.state.camera.target, start.target);
}

#[test]
fn ui_fit_refuses_what_it_cannot_frame_and_changes_nothing() {
    let mut session = plate();
    let mut shell = Shell::new();
    let mut window = Window::new(800.0, 600.0);
    window.idle(&mut shell, &mut session, 2);
    let before = shell.state.clone();
    for (params, says) in [
        (json!({"feature": "Nope"}), "Nope"),
        (json!({"feature": 3}), "feature"),
        (json!({"feature": "F1", "selection": true}), "not both"),
        (json!({"selection": true}), "nothing is selected"),
        (json!({"selection": "yes"}), "true or false"),
        (json!({"feature": "F1", "zoom": 2}), "zoom"),
    ] {
        let error = refused(&mut shell, &mut session, "ui.fit", params.clone());
        assert!(error.contains(says), "{params}: {error}");
        assert_eq!(shell.state, before, "{params}");
    }
    // Without a document.
    let mut empty = Session::new(Workspace::none());
    let mut other = Shell::new();
    let error = refused(&mut other, &mut empty, "ui.fit", json!({"feature": "F1"}));
    assert!(error.contains("no document"), "{error}");
}

#[test]
fn flying_to_what_is_hidden_shows_it() {
    let mut session = plate();
    let mut shell = Shell::new();
    let mut window = Window::new(1000.0, 700.0);
    window.idle(&mut shell, &mut session, 2);
    // The sketch hidden and cut away (everything above z = 20).
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"sketches": false}, "section": {"on": true, "plane": "xy", "offset": 20}}),
    );
    assert!(shell.section().is_some());
    call(
        &mut shell,
        &mut session,
        "ui.fit",
        json!({"feature": "Above"}),
    );
    assert!(shell.state.show.sketches);
    assert!(!shell.state.section.on, "the section cut all of it away");
    let target = shell.state.camera.target;
    assert!((target[2] - 80.0).abs() < 1e-9, "{target:?}");
    assert!((target[0] - 75.0).abs() < 1e-9, "{target:?}");

    // A section that leaves some of it in view stays: the tool's wall,
    // 0 to 10 high, cut at 5.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": true, "plane": "xy", "offset": 5}}),
    );
    call(
        &mut shell,
        &mut session,
        "ui.fit",
        json!({"feature": "Tool"}),
    );
    assert!(shell.state.section.on);
}

/// Frames for the list to scroll: egui animates it, for at most a
/// second.
const SCROLLED: usize = 70;

/// Forty blocks in a window whose list shows a dozen rows.
#[test]
fn selecting_in_the_viewport_scrolls_the_list_to_its_feature() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({}));
    for k in 0..40 {
        run(
            &mut session,
            "feature.block",
            json!({"size": [5, 5, 5], "position": [k * 10, 0, 0]}),
        );
    }
    let mut shell = Shell::new();
    let mut window = Window::new(900.0, 360.0);
    window.idle(&mut shell, &mut session, 3);
    let view = shell.tree_view().unwrap();
    assert!(row(&shell, "F1").top() >= view.top() - 1.0);
    assert!(
        row(&shell, "F40").top() > view.bottom(),
        "out of view at first"
    );

    // The bar lies against the panel's edge, not inside the rows.
    let viewport = shell.viewport_rect().unwrap();
    assert!(
        (viewport.left() - view.right()).abs() <= 2.0,
        "{view:?} {viewport:?}"
    );

    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let face_of = |feature: &str| {
        let body = scene.bodies.iter().find(|b| b.feature == feature).unwrap();
        json!({"items": [{"kind": "face", "name": body.face_names[0], "feature": feature}]})
    };
    call(&mut shell, &mut session, "ui.select", face_of("F40"));
    window.idle(&mut shell, &mut session, SCROLLED);
    let last = row(&shell, "F40");
    assert!(
        last.top() >= view.top() - 1.0 && last.bottom() <= view.bottom() + 1.0,
        "{last:?} in {view:?}"
    );
    assert_eq!(shell.made_selection(), Some("F40"));
    // A row already in view: the list stays where it is.
    call(&mut shell, &mut session, "ui.select", face_of("F38"));
    window.idle(&mut shell, &mut session, SCROLLED);
    assert_eq!(row(&shell, "F40"), last);
    // Back to the top.
    call(&mut shell, &mut session, "ui.select", face_of("F1"));
    window.idle(&mut shell, &mut session, SCROLLED);
    let first = row(&shell, "F1");
    assert!(first.top() >= view.top() - 1.0 && first.bottom() <= view.bottom() + 1.0);
    // A body selected (a double click in the viewport) is its feature.
    call(
        &mut shell,
        &mut session,
        "ui.select",
        json!({"items": [{"kind": "body", "feature": "F35"}]}),
    );
    window.idle(&mut shell, &mut session, SCROLLED);
    let body = row(&shell, "F35");
    assert!(body.bottom() <= view.bottom() + 1.0, "{body:?}");
    assert_eq!(shell.state.selected_feature.as_deref(), Some("F35"));
}
