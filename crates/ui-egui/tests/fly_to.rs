//! Flying to a feature on a large part (the developer's test of
//! 2026-10-23: a double click on a small fillet of a big body came far too
//! close, from the bottom view it was in, with nothing around it): the
//! flight turns to the iso view that sees the feature, keeps some of the
//! body around it, lights only the faces the feature made, and offers a
//! zoom icon on the rows that can be flown to.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Portable;
use craft3d_ui_egui::state::VIEWS;
use craft3d_ui_egui::{MIN_FLY_CONTEXT, Shell, ZOOM_TOOLTIP};
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

/// A plate 400 × 300 × 20 with a post of radius 10 on top, joined (`F3`)
/// and rounded where they meet (`Round`, `F4`, radius 3), then a feature
/// that fails (`F5`) and one that is suppressed (`F6`).
fn part() -> Session {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [400, 300, 20]}),
    );
    run(
        &mut session,
        "feature.cylinder",
        json!({"name": "Post", "position": [200, 150, 20], "radius": 10, "height": 30}),
    );
    run(
        &mut session,
        "feature.boolean",
        json!({"name": "Joined", "op": "union", "target": "Plate", "tool": "Post"}),
    );
    let scene = session.scene(None).unwrap().unwrap();
    let body = scene.bodies.iter().find(|b| b.feature == "F3").unwrap();
    let edge: Vec<String> = body
        .edge_faces
        .iter()
        .map(|faces| {
            faces
                .iter()
                .filter_map(|&f| body.face_names.get(f).cloned())
                .collect::<Vec<String>>()
        })
        .find(|faces| {
            let makers: Vec<Option<String>> = faces.iter().map(|f| scene.made_by(f)).collect();
            makers.contains(&Some("F1".to_string())) && makers.contains(&Some("F2".to_string()))
        })
        .unwrap();
    run(
        &mut session,
        "feature.fillet",
        json!({"name": "Round", "edges": [edge], "radius": 3}),
    );
    run(
        &mut session,
        "feature.sphere",
        json!({"name": "Broken", "radius": "missing * 2"}),
    );
    run(
        &mut session,
        "feature.sphere",
        json!({"name": "Ball", "radius": 5, "position": [0, 0, 100]}),
    );
    run(&mut session, "feature.suppress", json!({"feature": "Ball"}));
    session
}

struct Window {
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
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

    /// `clicks` clicks at `at`, after a pause.
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
}

fn row(shell: &Shell, id: &str) -> egui::Rect {
    shell
        .tree_row_hits()
        .iter()
        .find(|(row, _)| row == id)
        .map(|(_, rect)| *rect)
        .unwrap_or_else(|| panic!("no row {id}"))
}

/// Fits from the bottom view, as the developer had it.
fn from_below(shell: &mut Shell, session: &mut Session) {
    call(shell, session, "ui.set", json!({"view": "bottom"}));
    call(shell, session, "ui.fit", json!({}));
}

#[test]
fn a_small_fillet_is_flown_to_from_above_with_its_body_around_it() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.idle(&mut shell, &mut session, 3);
    from_below(&mut shell, &mut session);
    window.idle(&mut shell, &mut session, 2);
    let start = shell.state.camera;
    assert_eq!(start.pitch, -90.0);
    let at = row(&shell, "F4").center();
    window.clicks(&mut shell, &mut session, at, 2);
    window.idle(&mut shell, &mut session, 40);
    let camera = shell.state.camera;
    // An iso view from above: the plate hides the fillet from below.
    let (_, _, iso_pitch) = VIEWS[0];
    assert_eq!(camera.pitch, iso_pitch, "{camera:?}");
    assert!(
        [-45.0, 45.0, 135.0, -135.0].contains(&camera.yaw),
        "{camera:?}"
    );
    // Aimed at the fillet, a ring about the post's foot.
    assert!((camera.target[0] - 200.0).abs() < 1.0, "{camera:?}");
    assert!((camera.target[1] - 150.0).abs() < 1.0, "{camera:?}");
    // The eye is above the plate, and far enough away to show a fifth of
    // the plate's diagonal around it, not the fillet alone.
    let eye = camera.eye();
    assert!(eye.z > 20.0, "{eye:?}");
    let diagonal = (400.0_f64 * 400.0 + 300.0 * 300.0 + 50.0 * 50.0).sqrt();
    let half_view = camera.distance * (camera.fov_y.to_radians() / 2.0).ptan();
    assert!(
        half_view * 2.0 >= MIN_FLY_CONTEXT * diagonal / 3.0_f64.sqrt() * 0.99,
        "{} of {diagonal}",
        half_view * 2.0
    );
    assert!(
        camera.distance < start.distance * 0.5,
        "nearer than the whole"
    );

    // Only the fillet's faces are lit, not the whole part.
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let round = scene.feature_face_ids("F4");
    assert!(!round.is_empty());
    let mut lit = shell.selected_face_ids();
    lit.sort_unstable();
    assert_eq!(lit, round);
    // A feature that made no face of its own (the union) lights its body
    // once it has one; here the fillet's body took it, so nothing.
    call(
        &mut shell,
        &mut session,
        "ui.select",
        json!({"items": [{"kind": "body", "feature": "F4"}]}),
    );
    window.idle(&mut shell, &mut session, 1);
    assert_eq!(
        shell.selected_face_ids().len(),
        scene.bodies[0].face_names.len()
    );
}

#[test]
fn only_rows_that_can_be_shown_have_a_zoom_icon_and_fly() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.idle(&mut shell, &mut session, 3);
    let zoomable: Vec<(String, bool)> = shell
        .tree_rows()
        .iter()
        .map(|r| (r.id.clone(), r.zoomable))
        .collect();
    assert_eq!(
        zoomable,
        [
            ("F1".to_string(), true),
            ("F2".to_string(), true),
            // The union made no face and its body went into the fillet.
            ("F3".to_string(), false),
            ("F4".to_string(), true),
            ("F5".to_string(), false),
            ("F6".to_string(), false),
        ]
    );
    // The icon shows on the row under the pointer, where it can fly.
    for (id, can) in &zoomable {
        let at = row(&shell, id).center();
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        let icons: Vec<String> = shell
            .tree_zoom_hits()
            .iter()
            .map(|(row, _)| row.clone())
            .collect();
        if *can {
            assert_eq!(icons, std::slice::from_ref(id), "{id}");
        } else {
            assert!(icons.is_empty(), "{id}: {icons:?}");
        }
    }
    assert!(ZOOM_TOOLTIP.contains("Double-click"));

    // A double click on a row that cannot be shown does nothing.
    from_below(&mut shell, &mut session);
    window.idle(&mut shell, &mut session, 2);
    let before = shell.state.camera;
    for id in ["F3", "F5", "F6"] {
        let at = row(&shell, id).center();
        window.clicks(&mut shell, &mut session, at, 2);
        window.idle(&mut shell, &mut session, 30);
        assert_eq!(shell.state.camera, before, "{id}");
    }
    // The icon of one that can flies, as a double click does.
    let at = row(&shell, "F2").center();
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    let icon = shell
        .tree_zoom_hits()
        .iter()
        .find(|(row, _)| row == "F2")
        .map(|(_, rect)| rect.center())
        .unwrap();
    window.clicks(&mut shell, &mut session, icon, 1);
    window.idle(&mut shell, &mut session, 40);
    assert_ne!(shell.state.camera, before);
    assert_eq!(shell.state.selected_feature.as_deref(), Some("F2"));
    assert!(shell.state.camera.pitch > 0.0);
    // A hidden feature can be flown to: the flight shows it.
    call(
        &mut shell,
        &mut session,
        "ui.select",
        json!({"mode": "clear"}),
    );
    run(
        &mut session,
        "feature.visible",
        json!({"feature": "F4", "visible": false}),
    );
    window.idle(&mut shell, &mut session, 2);
    assert!(
        shell
            .tree_rows()
            .iter()
            .any(|r| r.id == "F4" && r.hidden && r.zoomable)
    );
}

/// The zoom icon's tooltip names the icon by its glyph, which the
/// default fonts must have (a missing one is drawn as a box).
#[test]
fn the_zoom_tooltip_is_in_the_fonts() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    output.textures_delta.clear();
    let body = egui::FontId::proportional(14.0);
    let glyph = |c: char| {
        ctx.fonts_mut(|f| {
            let galley = f.layout_no_wrap(c.to_string(), body.clone(), egui::Color32::WHITE);
            galley.rows[0].glyphs[0].uv_rect
        })
    };
    let missing = glyph('\u{10FFFD}');
    for c in ZOOM_TOOLTIP.chars().filter(|c| !c.is_ascii()) {
        assert_ne!(glyph(c), missing, "{c}");
    }
}

/// A hidden feature is offered only when showing it brings something to
/// frame (2026-10-23 review): the union, whose body the fillet took, has
/// none, hidden or not; the fillet, hidden, has its body.
#[test]
fn a_hidden_feature_is_zoomable_only_with_something_to_show() {
    let mut session = part();
    let mut shell = Shell::new();
    let mut window = Window::new();
    for id in ["F3", "F4"] {
        run(
            &mut session,
            "feature.visible",
            json!({"feature": id, "visible": false}),
        );
    }
    window.idle(&mut shell, &mut session, 3);
    let row = |shell: &Shell, id: &str| {
        shell
            .tree_rows()
            .iter()
            .find(|r| r.id == id)
            .map(|r| (r.hidden, r.zoomable))
            .unwrap()
    };
    assert_eq!(row(&shell, "F3"), (true, false));
    assert_eq!(row(&shell, "F4"), (true, true));
}

/// A small hole on a big part is framed as a hole: the context of a
/// fifth of the part is capped at eight times the hole's own size.
#[test]
fn a_small_hole_on_a_big_part_is_framed_as_a_hole() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Deck", "size": [2000, 1500, 20]}),
    );
    run(
        &mut session,
        "feature.cylinder",
        json!({"name": "Drill", "position": [1000, 750, -5], "radius": 1, "height": 30}),
    );
    run(
        &mut session,
        "feature.boolean",
        json!({"name": "Hole", "op": "subtract", "target": "Deck", "tool": "Drill"}),
    );
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.idle(&mut shell, &mut session, 2);
    call(
        &mut shell,
        &mut session,
        "ui.fit",
        json!({"feature": "Drill"}),
    );
    let camera = shell.state.camera;
    // The hole's wall: radius 1, 20 high, a diagonal of about 20.2.
    let wall = (2.0_f64 * 2.0 + 2.0 * 2.0 + 20.0 * 20.0).sqrt();
    let seen = 2.0 * camera.distance * (camera.fov_y.to_radians() / 2.0).ptan();
    assert!(seen > wall * 0.5, "{seen}");
    assert!(
        seen < 8.0 * wall * 1.5,
        "{seen}: not a fifth of the deck ({})",
        0.2 * 2500.0
    );
}
