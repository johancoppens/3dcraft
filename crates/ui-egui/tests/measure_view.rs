//! Measuring by selecting (the developer's design of 2026-11-17, steps B
//! and C): with two things selected the window shows their relation by
//! itself, by the engine's `measure.between` (the same numbers an agent
//! gets): one large number at the foot of the side panel with what it is
//! in small, one dimension line in the view, a click on the number copies
//! it with its unit, Escape or another selection takes it away, and
//! "Keep as a check" makes it an assertion of the document, one step to
//! undo. What is shown is in `ui.get` › `measurement`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell};
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

/// A block of 60 × 40 × 20 with holes of radius 3 along z through
/// (15, 20) and (45, 20), its top's edge along y at x = 60 chamfered by 8.
fn part() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 40, 20], "name": "Block"}),
    );
    for (x, name) in [(15, "Left"), (45, "Right")] {
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": 3, "height": 30, "position": [x, 20, -5], "name": name}),
        );
    }
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Left", "name": "Once"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Once", "tool": "Right", "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:face(1)", "F1:face(5)"]], "distance": 8, "name": "Bevel"}),
    );
    s
}

struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
    /// What the window copied last.
    copied: Option<String>,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: vec2(width, height),
            time: 0.0,
            copied: None,
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
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                self.copied = Some(text.clone());
            }
        }
        output.textures_delta.clear();
    }

    fn frames(&mut self, shell: &mut Shell, session: &mut Session, n: usize) {
        for _ in 0..n {
            self.frame(shell, session, Vec::new());
        }
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
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
        self.frames(shell, session, 2);
    }

    fn key(&mut self, shell: &mut Shell, session: &mut Session, key: egui::Key) {
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
        self.frames(shell, session, 2);
    }
}

fn target(shell: &Shell, name: &str) -> egui::Rect {
    shell
        .side_hits()
        .targets
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no {name}: {:?}", shell.side_hits().targets))
        .1
}

fn select(session: &mut Session, kind: &str, filter: Value, count: u64) {
    let found = run(
        session,
        "select.query",
        json!({"kind": kind, "where": filter, "then": "replace"}),
    );
    assert_eq!(found["count"], count, "{found}");
}

#[test]
fn two_holes_show_their_centre_distance_by_themselves_and_escape_takes_it_away() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    // One face: its own properties, no relation, no line.
    select(
        &mut session,
        "face",
        json!({"in": "Bevel", "hole": true, "inside": [[0, 0, -10], [30, 40, 30]]}),
        1,
    );
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.measurement().is_none());
    assert!(shell.dimension_place().is_none());
    // Both holes: the centre distance, the same as measure.between's.
    select(
        &mut session,
        "face",
        json!({"in": "Bevel", "hole": true}),
        2,
    );
    window.frames(&mut shell, &mut session, 3);
    let m = shell.measurement().cloned().expect("their relation");
    assert_eq!(m.relation, "centerDistance");
    assert_eq!(m.text, "30.00 mm");
    assert_eq!(m.words, "Centre distance · axes parallel");
    assert_eq!(m.title, "2 faces");
    assert_eq!(m.parts.as_deref(), Some("X 30.00 · Y 0.00 · Z 0.00 mm"));
    let agent = run(
        &mut session,
        "measure.between",
        json!({"from": m.items[0], "to": m.items[1]}),
    );
    assert_eq!(agent["value"].as_f64(), m.value);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["measurement"]["text"], "30.00 mm");
    assert_eq!(got["measurement"]["relation"], "centerDistance");
    // The line in the view, its number on it; a click on the number copies
    // it and selects nothing else.
    let place = shell
        .dimension_place()
        .cloned()
        .expect("the dimension line");
    assert!(place.from.distance(place.to) > 20.0, "{place:?}");
    let viewport = shell.viewport_rect().unwrap();
    assert!(viewport.contains_rect(place.number));
    let selection = got["selection"].clone();
    window.click(&mut shell, &mut session, place.number.center());
    assert_eq!(window.copied.as_deref(), Some("30.00 mm"));
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["selection"], selection);
    // The panel's large number copies too.
    window.copied = None;
    let value = target(&shell, "copy value").center();
    window.click(&mut shell, &mut session, value);
    assert_eq!(window.copied.as_deref(), Some("30.00 mm"));
    // Escape over the view: the selection, the number and the line go.
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(viewport.center())],
    );
    window.key(&mut shell, &mut session, egui::Key::Escape);
    assert!(shell.measurement().is_none());
    assert!(shell.dimension_place().is_none());
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["measurement"], Value::Null);
}

#[test]
fn faces_at_an_angle_show_it_on_an_arc_and_parallel_faces_their_distance() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    select(
        &mut session,
        "face",
        json!({"in": "Bevel", "any": [{"normal": {"along": "z"}}, {"normal": {"along": [1, 0, 1]}}]}),
        2,
    );
    window.frames(&mut shell, &mut session, 3);
    let m = shell.measurement().cloned().unwrap();
    assert_eq!(m.relation, "angle");
    assert_eq!(m.text, "135.00°");
    let place = shell.dimension_place().cloned().unwrap();
    assert!(place.at.is_some(), "an arc about its corner");
    select(
        &mut session,
        "face",
        json!({"in": "Bevel", "any": [{"normal": {"along": "z"}}, {"normal": {"along": "-z"}}]}),
        2,
    );
    window.frames(&mut shell, &mut session, 3);
    let m = shell.measurement().cloned().unwrap();
    assert_eq!(
        (m.relation.as_str(), m.text.as_str()),
        ("distance", "20.00 mm")
    );
    assert!(shell.dimension_place().unwrap().at.is_none());
}

#[test]
fn keep_as_a_check_adds_the_relation_now_as_an_assertion_one_step_to_undo() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    select(
        &mut session,
        "face",
        json!({"in": "Bevel", "hole": true}),
        2,
    );
    window.frames(&mut shell, &mut session, 3);
    let keep = target(&shell, "keep").center();
    window.click(&mut shell, &mut session, keep);
    window.frames(&mut shell, &mut session, 3);
    let listed = run(&mut session, "assert.list", json!({}));
    let rows = listed["assertions"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{listed}");
    let row = &rows[0];
    assert_eq!(
        row["name"], "Centre distance Left face – Right face",
        "{row}"
    );
    assert_eq!(row["status"], "passed", "{row}");
    assert_eq!(row["measure"]["measure"], "between");
    assert_eq!(row["measure"]["relation"], "centerDistance");
    // Kept once: the button says so until the selection changes.
    let keep = target(&shell, "keep").center();
    window.click(&mut shell, &mut session, keep);
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(
        run(&mut session, "assert.list", json!({}))["assertions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // One step to undo, the check's own (the revision counts on through
    // an undo, so it is not compared).
    let undone = run(&mut session, "edit.undo", json!({}));
    assert_eq!(
        undone["undone"], "Add assertion Centre distance Left face – Right face",
        "{undone}"
    );
    assert!(
        run(&mut session, "assert.list", json!({}))["assertions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // An angle is kept as angleBetween, in degrees; a second check of the
    // same two gets a name of its own.
    select(
        &mut session,
        "face",
        json!({"in": "Bevel", "any": [{"normal": {"along": "z"}}, {"normal": {"along": [1, 0, 1]}}]}),
        2,
    );
    window.frames(&mut shell, &mut session, 3);
    for _ in 0..2 {
        let keep = target(&shell, "keep").center();
        window.click(&mut shell, &mut session, keep);
        window.frames(&mut shell, &mut session, 2);
        // Selected again, so the button can keep it again.
        run(&mut session, "select.set", json!({"mode": "clear"}));
        window.frames(&mut shell, &mut session, 2);
        select(
            &mut session,
            "face",
            json!({"in": "Bevel", "any": [{"normal": {"along": "z"}}, {"normal": {"along": [1, 0, 1]}}]}),
            2,
        );
        window.frames(&mut shell, &mut session, 3);
    }
    let listed = run(&mut session, "assert.list", json!({}));
    let names: Vec<&str> = listed["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "{listed}");
    assert!(names[1].ends_with(" 2"), "{names:?}");
    for a in listed["assertions"].as_array().unwrap() {
        assert_eq!(a["status"], "passed", "{a}");
        assert_eq!(a["measure"]["measure"], "angleBetween");
    }
}

#[test]
fn what_has_no_relation_shows_none_and_nothing_panics() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    // Two features: no relation.
    run(
        &mut session,
        "select.set",
        json!({"items": [{"kind": "feature", "feature": "Left"}, {"kind": "feature", "feature": "Right"}]}),
    );
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.measurement().is_none());
    // A straight edge and a body: refused in words, no line, nothing to
    // keep.
    let edge = run(
        &mut session,
        "select.query",
        json!({"kind": "edge", "where": {"in": "Bevel", "length": 60}, "limit": 1}),
    );
    let it = &edge["items"][0];
    run(
        &mut session,
        "select.set",
        json!({"items": [
            {"kind": "edge", "name": it["name"], "feature": it["feature"]},
            {"kind": "body", "feature": "Bevel"}
        ]}),
    );
    window.frames(&mut shell, &mut session, 3);
    if let Some(m) = shell.measurement() {
        assert!(m.refused.is_some() || m.value.is_some(), "{m:?}");
        if m.refused.is_some() {
            assert!(shell.dimension_place().is_none());
            assert_eq!(m.check("x"), None);
        }
    }
    // A camera that is not a number: no line, no panic.
    select(
        &mut session,
        "face",
        json!({"in": "Bevel", "hole": true}),
        2,
    );
    window.frames(&mut shell, &mut session, 3);
    shell.state.camera.distance = f64::NAN;
    window.frames(&mut shell, &mut session, 2);
    assert!(shell.dimension_place().is_none());
    // The document emptied: nothing measured.
    let mut empty = Session::new(Workspace::none());
    run(&mut empty, "file.new", json!({"units": "mm"}));
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    window.frames(&mut shell, &mut empty, 3);
    assert!(shell.measurement().is_none());
}
