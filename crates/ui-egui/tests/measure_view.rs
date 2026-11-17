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
use craft3d_math::Portable;
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

    /// `n` frames, then more while the relation is measured on its own
    /// thread (the window does not wait for it; a test does, ten seconds
    /// at most).
    fn frames(&mut self, shell: &mut Shell, session: &mut Session, n: usize) {
        for _ in 0..n {
            self.frame(shell, session, Vec::new());
        }
        for _ in 0..5000 {
            if !shell.measuring() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
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
    // One face: its own properties, no relation; a hole's wall shows its
    // diameter in the view (the developer, 2026-11-17: "ik zou maten in
    // de view willen zien"; before, one face drew nothing there).
    select(
        &mut session,
        "face",
        json!({"in": "Bevel", "hole": true, "inside": [[0, 0, -10], [30, 40, 30]]}),
        1,
    );
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.measurement().is_none());
    let single = shell
        .single_measure()
        .cloned()
        .expect("the hole's diameter");
    assert_eq!(
        (single.measure, single.text.as_str()),
        ("diameter", "Ø 6 mm")
    );
    // Across its rim in view, the top's (z = 20), not at mid-depth (the
    // review of 2026-11-17).
    let (centre, _, _) = single.round.expect("drawn across its rim");
    assert!((centre[2] - 20.0).abs() < 1e-9, "{centre:?}");
    assert_eq!(
        shell.dimension_place().map(|p| p.text.as_str()),
        Some("Ø 6 mm")
    );
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
    assert_eq!(m.text, "30 mm");
    assert_eq!(m.words, "Centre distance · axes parallel");
    assert_eq!(m.title, "2 faces");
    assert_eq!(m.parts.as_deref(), Some("X 30 · Y 0 · Z 0 mm"));
    let agent = run(
        &mut session,
        "measure.between",
        json!({"from": m.items[0], "to": m.items[1]}),
    );
    assert_eq!(agent["value"].as_f64(), m.value);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["measurement"]["text"], "30 mm");
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
    assert_eq!(window.copied.as_deref(), Some("30 mm"));
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["selection"], selection);
    // The panel's large number copies too.
    window.copied = None;
    let value = target(&shell, "copy value").center();
    window.click(&mut shell, &mut session, value);
    assert_eq!(window.copied.as_deref(), Some("30 mm"));
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

/// The developer's words of 2026-11-17: "ik zou maten in de view willen
/// zien … als ik bijvoorbeeld de lengte van onderste pad selecteer moet ik
/// eerst naar features gaan om de lengte te zien." One thing selected
/// shows its measure in the view: a straight edge its length along a line
/// beside it, a hole's rim its diameter, a flat face nothing; and what is
/// selected shows under whichever tab of the side panel is open.
#[test]
fn one_thing_shows_its_measure_in_the_view_and_its_card_under_every_tab() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    // Another tab open, as the developer had.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"panels": {"sideTab": "variables"}}),
    );
    // A straight edge 60 long along the foot of the block.
    run(
        &mut session,
        "select.query",
        json!({"kind": "edge", "where": {"in": "Bevel", "length": 60}, "limit": 1, "then": "replace"}),
    );
    window.frames(&mut shell, &mut session, 3);
    let single = shell.single_measure().cloned().expect("its length");
    assert_eq!((single.measure, single.text.as_str()), ("length", "60 mm"));
    assert_eq!(single.points.len(), 2, "{single:?}");
    let place = shell.dimension_place().cloned().expect("a line beside it");
    assert!(place.from.distance(place.to) > 20.0, "{place:?}");
    let viewport = shell.viewport_rect().unwrap();
    assert!(viewport.contains_rect(place.number), "{place:?}");
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["panels"]["sideTab"], "variables");
    assert_eq!(got["measurement"]["measure"], "length");
    assert_eq!(got["measurement"]["text"], "60 mm");
    // The card is under the Variables tab too.
    let name = target(&shell, "copy name");
    assert!(name.width() > 0.0);
    // A click on the number copies it and selects nothing else.
    let selection = got["selection"].clone();
    window.click(&mut shell, &mut session, place.number.center());
    assert_eq!(window.copied.as_deref(), Some("60 mm"));
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["selection"], selection);
    // A hole's rim, a whole circle: its diameter.
    run(
        &mut session,
        "select.query",
        json!({"kind": "edge", "where": {"in": "Bevel", "radius": 3}, "limit": 1, "then": "replace"}),
    );
    window.frames(&mut shell, &mut session, 3);
    let single = shell.single_measure().cloned().expect("the rim's diameter");
    assert_eq!(
        (single.measure, single.text.as_str()),
        ("diameter", "Ø 6 mm")
    );
    assert!(shell.dimension_place().is_some());
    // A flat face: nothing in the view, its card all the same, under
    // every tab.
    select(
        &mut session,
        "face",
        json!({"in": "Bevel", "normal": {"along": "-z"}}),
        1,
    );
    for tab in ["bodies", "features", "variables"] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"panels": {"sideTab": tab}}),
        );
        window.frames(&mut shell, &mut session, 3);
        assert!(shell.single_measure().is_none(), "{tab}");
        assert!(shell.dimension_place().is_none(), "{tab}");
        assert!(target(&shell, "copy name").width() > 0.0, "{tab}");
    }
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["measurement"], Value::Null);
    // Escape over the view: the selection, the card and the line go.
    run(
        &mut session,
        "select.query",
        json!({"kind": "edge", "where": {"in": "Bevel", "length": 60}, "limit": 1, "then": "replace"}),
    );
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.dimension_place().is_some());
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(viewport.center())],
    );
    window.key(&mut shell, &mut session, egui::Key::Escape);
    assert!(shell.single_measure().is_none());
    assert!(shell.dimension_place().is_none());
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
    assert_eq!(m.text, "135°");
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
        ("distance", "20 mm")
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

/// The review of 2026-11-17: the diameter sign ⌀ (U+2300) is in neither
/// font the window ships and drew as a box. Every character the measuring
/// can print, numbers, units, signs and words, is in the fonts the window
/// has once it has drawn.
#[test]
fn every_character_the_measuring_prints_is_in_the_windows_fonts() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    let printed = [
        "0123456789.-? Ø R ° · – … mm cm m in ft µm um",
        "X Y Z Overlap Measuring… Not measured",
        "Centre distance · axes parallel",
        "Distance between parallel faces",
        "Distance between the parallel edges",
        "Distance from the axis to the face",
        "Distance between the points",
        "Angle between the faces Angle between the edges",
        "Angle between the axes Angle between the axis and the face",
        "Least distance Least distance between the edges Clearance about",
        "Keep as a check Kept as a check Copy",
    ];
    for text in printed {
        let font = egui::FontId::proportional(14.0);
        let missing: Vec<char> = text
            .chars()
            .filter(|c| !window.ctx.fonts_mut(|f| f.has_glyph(&font, *c)))
            .collect();
        assert!(missing.is_empty(), "{text}: no glyph for {missing:?}");
    }
    // The sign that drew as a box is not printed.
    let one = run(
        &mut session,
        "select.query",
        json!({"kind": "face", "where": {"in": "Bevel", "hole": true}, "limit": 1, "then": "replace"}),
    );
    assert_eq!(one["count"].as_u64().map(|n| n >= 1), Some(true));
    window.frames(&mut shell, &mut session, 3);
    let single = shell
        .single_measure()
        .cloned()
        .expect("the hole's diameter");
    assert!(
        single.text.starts_with('Ø') && !single.text.contains('⌀'),
        "{}",
        single.text
    );
}

/// The review of 2026-11-17: in a short window (1024 × 420) the Features
/// tab showed no card, so the relation and Keep were out of reach; and
/// after the kept check was undone the button still said it was kept. Now
/// the card is a line there, with Keep, and a check undone can be kept
/// again.
#[test]
fn in_a_short_window_the_relation_and_keep_stay_in_reach_and_undone_can_be_kept_again() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1024.0, 420.0);
    window.frames(&mut shell, &mut session, 3);
    select(
        &mut session,
        "face",
        json!({"in": "Bevel", "hole": true}),
        2,
    );
    window.frames(&mut shell, &mut session, 3);
    let m = shell.measurement().cloned().expect("their relation");
    assert_eq!(m.text, "30 mm");
    let count = |s: &mut Session| {
        run(s, "assert.list", json!({}))["assertions"]
            .as_array()
            .unwrap()
            .len()
    };
    let keep = target(&shell, "keep").center();
    window.click(&mut shell, &mut session, keep);
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(count(&mut session), 1);
    run(&mut session, "edit.undo", json!({}));
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(count(&mut session), 0);
    // The same selection: it can be kept again.
    let keep = target(&shell, "keep").center();
    window.click(&mut shell, &mut session, keep);
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(count(&mut session), 1);
}

/// The review of 2026-11-17: the card's Measure button failed on every
/// face ("there is no feature named `F6:copy(0)/F1:face(1)`": it passed
/// the face's bare name). It asks for the face by its body and name, and
/// the measure answers; the button, clicked, says the wall.
#[test]
fn the_cards_measure_button_measures_a_faces_wall() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    let found = run(
        &mut session,
        "select.query",
        json!({"kind": "face", "where": {"in": "Bevel", "normal": {"along": "z"}}, "then": "replace"}),
    );
    let it = &found["items"][0];
    let answer = run(
        &mut session,
        "measure.thickness",
        json!({"of": {"kind": "face", "where": {"in": it["feature"], "name": it["name"]}}}),
    );
    assert!(answer.is_object(), "{answer}");
    window.frames(&mut shell, &mut session, 3);
    let measure = target(&shell, "measure").center();
    window.click(&mut shell, &mut session, measure);
    window.frames(&mut shell, &mut session, 3);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    let text = got.to_string();
    assert!(!text.contains("there is no feature named"), "{text}");
}

/// Frames until the relation is measured: every text the card showed
/// settled on the way.
fn shown_until_settled(
    window: &mut Window,
    shell: &mut Shell,
    session: &mut Session,
) -> Vec<String> {
    let mut seen = Vec::new();
    for _ in 0..5000 {
        window.frame(shell, session, Vec::new());
        if let Some(m) = shell.measurement().filter(|m| !m.measuring) {
            seen.push(m.text.clone());
        }
        if !shell.measuring() && !seen.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    seen
}

/// The review of 2026-11-17, round two: one measure runs at a time and an
/// answer for an older selection or revision is never shown. Two holes,
/// then the right one moved before the answer came: only its new centre
/// distance; two holes, then two parallel faces at once: only the faces'
/// distance; then a quick run of selections: only the last one's.
#[test]
fn an_answer_for_an_older_selection_or_revision_is_never_shown() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    let holes = json!({"in": "Bevel", "hole": true});
    select(&mut session, "face", holes.clone(), 2);
    window.frame(&mut shell, &mut session, Vec::new());
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Right", "position": [47, 20, -5]}),
    );
    let seen = shown_until_settled(&mut window, &mut shell, &mut session);
    assert!(
        !seen.is_empty() && seen.iter().all(|t| t == "32 mm"),
        "{seen:?}"
    );
    let faces =
        json!({"in": "Bevel", "any": [{"normal": {"along": "z"}}, {"normal": {"along": "-z"}}]});
    select(&mut session, "face", holes.clone(), 2);
    window.frame(&mut shell, &mut session, Vec::new());
    select(&mut session, "face", faces.clone(), 2);
    let seen = shown_until_settled(&mut window, &mut shell, &mut session);
    assert!(
        !seen.is_empty() && seen.iter().all(|t| t == "20 mm"),
        "{seen:?}"
    );
    for _ in 0..3 {
        select(&mut session, "face", holes.clone(), 2);
        window.frame(&mut shell, &mut session, Vec::new());
        select(&mut session, "face", faces.clone(), 2);
        window.frame(&mut shell, &mut session, Vec::new());
    }
    let seen = shown_until_settled(&mut window, &mut shell, &mut session);
    assert!(
        !seen.is_empty() && seen.iter().all(|t| t == "20 mm"),
        "{seen:?}"
    );
}

/// The review of 2026-11-17, round three: the faces of a profile of 2000
/// segments have names of over 8192 bytes; selected, the card measures
/// them (it showed "—"), and Keep keeps the check, which passes.
#[test]
fn faces_named_in_over_8192_bytes_are_measured_and_kept() {
    let n = 2000_usize;
    let point = |i: usize| {
        let a = std::f64::consts::TAU * (i % n) as f64 / n as f64;
        json!([50.0 * a.pcos(), 50.0 * a.psin()])
    };
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "sketch.create",
        json!({"plane": "xy", "name": "Many"}),
    );
    // A polyline takes at most 1000 points: in pieces of 500 segments.
    for start in (0..n).step_by(500) {
        let points: Vec<Value> = (start..=(start + 500).min(n)).map(point).collect();
        run(
            &mut session,
            "sketch.polyline",
            json!({"sketch": "Many", "points": points}),
        );
    }
    run(
        &mut session,
        "feature.extrude",
        json!({"sketch": "Many", "distance": 5, "name": "Poly"}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    let found = run(
        &mut session,
        "select.query",
        json!({"kind": "face", "where": {"in": "Poly", "any": [{"normal": {"along": "z"}}, {"normal": {"along": "-z"}}]}, "then": "replace"}),
    );
    assert_eq!(found["count"], 2, "{found}");
    let longest = found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["name"].as_str().map_or(0, str::len))
        .max()
        .unwrap();
    assert!(longest > 8192, "{longest} bytes");
    window.frames(&mut shell, &mut session, 3);
    let m = shell.measurement().cloned().unwrap();
    assert_eq!(
        (m.relation.as_str(), m.text.as_str()),
        ("distance", "5 mm"),
        "{:?}",
        m.refused
    );
    let keep = target(&shell, "keep").center();
    window.click(&mut shell, &mut session, keep);
    window.frames(&mut shell, &mut session, 3);
    let listed = run(&mut session, "assert.list", json!({}));
    let rows = listed["assertions"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{listed}");
    assert_eq!(rows[0]["status"], "passed", "{listed}");
}

/// The review of 2026-11-17: a range's large number fits the card at the
/// panel's least width (it was cut at the edge): broken after its dash,
/// or drawn smaller. An edge in an inner corner and a ball near both of
/// its faces: the faces come nearer than the edge, so the least is a range.
#[test]
fn a_range_fits_the_card_at_the_panels_least_width() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [20, 10, 2], "name": "Base"}),
    );
    run(
        &mut session,
        "feature.block",
        json!({"size": [2, 10, 20], "name": "Wall"}),
    );
    run(
        &mut session,
        "feature.boolean",
        json!({"op": "union", "target": "Base", "tool": "Wall", "name": "L"}),
    );
    run(
        &mut session,
        "feature.sphere",
        json!({"radius": 0.5, "position": [3, 5, 3], "name": "Ball"}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"panels": {"treeWidth": 190}}),
    );
    select(
        &mut session,
        "edge",
        json!({"in": "L", "inside": [[1.9, -1, 1.9], [2.1, 11, 2.1]]}),
        1,
    );
    run(
        &mut session,
        "select.query",
        json!({"kind": "body", "where": {"in": "Ball"}, "then": "add"}),
    );
    window.frames(&mut shell, &mut session, 3);
    let m = shell.measurement().cloned().unwrap();
    assert!(m.text.contains(" – "), "{}", m.text);
    let number = target(&shell, "copy value");
    assert!(
        number.right() <= 190.0 && number.width() > 0.0,
        "{} drawn at {number:?}",
        m.text
    );
}
