//! The review of the UX round's view tools (`review-ux-view`, 2026-11-18,
//! on `check/ux-round` 5d8452a7): the plate's word button, the first cut,
//! the card's number, the one way to write a number, the status bar's
//! hint and Escape's order, measured by the reviewer's own tests: numbers
//! at several tolerances, scales and units against hand-worked texts, a
//! placed body's box against the closed form of its turn, the word
//! button's one degree on both sides, the agent's first cut against the
//! person's, and Escape through the real key path for things open
//! together.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_math::{Portable, Vec3};
use craft3d_ui_egui::shell::section_handle::{
    ALONG_AXIS, LOST_FACE, NUMBER_FIELD, PlaneChoice, choose_plane,
};
use craft3d_ui_egui::shell::shown;
use craft3d_ui_egui::state::{SectionMade, SectionState};
use craft3d_ui_egui::{RendererChoice, Shell, StatusSlot};
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
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

    /// `n` frames, then more while a measure runs.
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
                    modifiers: Modifiers::NONE,
                }],
            );
        }
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
                modifiers: Modifiers::NONE,
            }],
        );
        self.frames(shell, session, 2);
    }
}

fn select(session: &mut Session, kind: &str, filter: Value, limit: u64) {
    run(
        session,
        "select.query",
        json!({"kind": kind, "where": filter, "limit": limit, "then": "replace"}),
    );
}

/// A block of 60 × 40 × 20 with a hole of radius 4 along y through
/// (20, ·, 10) and one of radius 3 along z through (45, 20, ·).
fn drilled() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 40, 20], "name": "Block"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 4, "height": 50, "position": [20, -5, 10], "axis": [0, 1, 0], "name": "Across"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 3, "height": 30, "position": [45, 20, -5], "name": "Down"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Across", "name": "Once"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Once", "tool": "Down", "name": "Drilled"}),
    );
    s
}

/// A plate of 60 × 40 × 20 with two holes of radius 3 through it, at
/// (15, 20) and (40, 33): their centres √794 = 28.178005607… apart.
fn plate() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 40, 20], "name": "Block"}),
    );
    for (x, y, name) in [(15, 20, "Left"), (40, 33, "Right")] {
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": 3, "height": 30, "position": [x, y, -5], "name": name}),
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
    s
}

/// The document's window: CPU renderer, `size`, the section across
/// `plane` at `offset`, seen from `view` orthographically.
fn window_on(
    session: &mut Session,
    size: [f32; 2],
    plane: &str,
    offset: f64,
    view: &str,
) -> (Shell, Window) {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(size[0], size[1]);
    window.frames(&mut shell, session, 3);
    call(
        &mut shell,
        session,
        "ui.set",
        json!({"camera": {"projection": "orthographic"},
               "section": {"on": true, "plane": plane, "offset": offset}}),
    );
    let _ = shell.handle_method(session, "ui.fit", Some(&json!({"view": view})));
    window.frames(&mut shell, session, 4);
    (shell, window)
}

/// The word button as `ui.get` says it: label, hover, lit, greyed.
fn word_of(shell: &mut Shell, session: &mut Session) -> (String, String, bool, bool) {
    let got = call(shell, session, "ui.get", json!({}));
    let word = &got["section"]["fromSelection"];
    (
        word["label"].as_str().unwrap_or_default().to_string(),
        word["hover"].as_str().unwrap_or_default().to_string(),
        word["lit"] == true,
        word["greyed"] == true,
    )
}

/// The first cut of a fresh tab: the person's (one click on Section) and
/// the agent's (`ui.set {section: {on: true}}`), each in a window of its
/// own on `session`; and `ui.get` › `section` after the person's.
fn first_cuts(session: &mut Session) -> (SectionState, SectionState, Value) {
    let mut person = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1200.0, 800.0);
    window.frames(&mut person, session, 4);
    let toggle = person
        .section_hits()
        .iter()
        .find(|(n, _)| *n == "toggle")
        .expect("the Section chip")
        .1
        .center();
    window.click(&mut person, session, toggle);
    window.frames(&mut person, session, 3);
    let clicked = person.state.section.clone();
    let report = call(&mut person, session, "ui.get", json!({}))["section"].clone();
    let mut agent = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1200.0, 800.0);
    window.frames(&mut agent, session, 4);
    call(
        &mut agent,
        session,
        "ui.set",
        json!({"section": {"on": true}}),
    );
    window.frames(&mut agent, session, 2);
    (clicked, agent.state.section.clone(), report)
}

/// V5, by hand: the last place rounds, a negative that rounds to nothing
/// is `0` (never `-0`), a gap of 0.0004 mm is not `0 mm` at the default
/// tolerance (and is at 0.001, where it is nothing to the document), a
/// coarse tolerance shows no digit past it, the scales 1e-3 and 1e6, inches
/// and metres by their own places, not a number as `?`, a tolerance of
/// 10^-k holds k places for every k, and the hover's whole value lies
/// within half the tolerance of the value (2000 values at four scales).
#[test]
fn review_every_number_a_person_reads_is_right() {
    let mm = |v: f64, tolerance: f64| shown::length(v, tolerance, "mm").text;
    assert_eq!(mm(0.0005, 1e-6), "0.0005 mm");
    assert_eq!(mm(999.9995, 1e-6), "1000 mm");
    assert_eq!(mm(-0.0004, 1e-6), "-0.0004 mm");
    assert_eq!(mm(-0.0004, 1e-3), "0 mm");
    assert_eq!(mm(-0.000_000_4, 1e-6), "0 mm");
    assert_eq!(mm(0.0004, 1e-6), "0.0004 mm");
    assert_eq!(mm(25.980_762, 0.01), "25.98 mm");
    assert_eq!(shown::length(25.980_762, 0.01, "mm").whole, "25.98 mm");
    assert_eq!(mm(0.0004, 0.01), "0 mm");
    let big = shown::length(1_234_567.891_234, 1e-6, "mm");
    assert_eq!(
        (big.text.as_str(), big.whole.as_str()),
        ("1234567.891 mm", "1234567.891234 mm")
    );
    let small = shown::length(0.001_234_5, 1e-9, "mm");
    assert_eq!(
        (small.text.as_str(), small.whole.as_str()),
        ("0.0012 mm", "0.0012345 mm")
    );
    let inch = shown::length(0.039_370_078_7, 1e-6, "in");
    assert_eq!(
        (inch.text.as_str(), inch.whole.as_str()),
        ("0.0394 in", "0.03937 in")
    );
    assert_eq!(shown::length(1e-5, 1e-7, "in").text, "0.00001 in");
    assert_eq!(shown::length(1.234_567_89, 1e-9, "m").text, "1.234568 m");
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(mm(v, 1e-6), "? mm");
        assert_eq!(shown::length(v, 1e-6, "mm").whole, "? mm");
        assert_eq!(shown::area(v, "mm"), "? mm²");
    }
    assert_eq!(shown::area(-0.0, "mm"), "0 mm²");
    assert_eq!(shown::area(0.0004, "mm"), "0.0004 mm²");
    assert_eq!(shown::angle(-0.001, 1e-9).text, "0°");
    // A tolerance of 10^-k: k places in the whole value, for every k.
    let tolerances = [
        1e-1, 1e-2, 1e-3, 1e-4, 1e-5, 1e-6, 1e-7, 1e-8, 1e-9, 1e-10, 1e-11, 1e-12,
    ];
    for (k, tolerance) in tolerances.iter().enumerate() {
        let whole = shown::length_whole(1.0 / 3.0, *tolerance);
        let places = whole.split_once('.').map_or(0, |(_, f)| f.len());
        assert_eq!(places, k + 1, "tolerance {tolerance}: {whole}");
    }
    // The hover's whole value and the text, read back.
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for k in 0..2000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let unit = (x >> 11) as f64 / (1u64 << 53) as f64;
        let scale = [1e-3, 1.0, 1e3, 1e6][k % 4];
        let v = (unit - 0.5) * 2.0 * scale;
        let said = shown::length(v, 1e-6, "mm");
        let whole: f64 = said.whole.trim_end_matches(" mm").parse().unwrap();
        assert!(
            (whole - v).abs() <= 5e-7 + 1e-12 * v.abs(),
            "{v}: {}",
            said.whole
        );
        let text: f64 = said.text.trim_end_matches(" mm").parse().unwrap();
        assert!(
            (text - v).abs() <= 5e-4 + 1e-12 * v.abs(),
            "{v}: {}",
            said.text
        );
        assert!(!said.text.starts_with("-0 "), "{v}: {}", said.text);
    }
}

/// V2, fix 1: the word button on a round face is lit only while the cut
/// through its axis faces the eye within 1 degree: 0.9 degrees round the
/// axis lit, 1.1 not, either way; a turn of the eye up or down (the same
/// facing direction) lit; the other side not; within 2 degrees of looking
/// along the axis greyed with its words, and a click there changes
/// nothing.
#[test]
fn review_through_axis_is_lit_within_one_degree_and_greyed_along_the_axis() {
    let toward = |yaw: f64, pitch: f64| {
        let (y, p) = (yaw.to_radians(), pitch.to_radians());
        Vec3::new(p.pcos() * y.pcos(), p.pcos() * y.psin(), p.psin())
    };
    let round = PlaneChoice {
        made: Some(SectionMade::Axis),
        faces: vec![("F4".to_string(), "F4:face(0)".to_string())],
        planes: vec![([45.0, 20.0, 0.0], [0.0, 0.0, 1.0])],
        refused: None,
    };
    let mut section = SectionState {
        on: true,
        plane: Some("yz".to_string()),
        offset: 30.0,
        ..SectionState::default()
    };
    choose_plane(&mut section, &round, toward(-45.0, 30.0));
    assert_eq!(section.made, Some(SectionMade::Axis));
    for (yaw, pitch, lit) in [
        (-45.0, 30.0, true),
        (-44.1, 30.0, true),
        (-45.9, 30.0, true),
        (-43.9, 30.0, false),
        (-46.1, 30.0, false),
        (-45.0, 75.0, true),
        (-45.0, -20.0, true),
        (135.0, 30.0, false),
    ] {
        let word = round.button(&section, toward(yaw, pitch));
        assert_eq!(
            (word.label, word.lit, word.greyed),
            ("Through axis", lit, false),
            "yaw {yaw}, pitch {pitch}"
        );
    }
    for (pitch, greyed) in [(88.5, true), (87.5, false), (-88.5, true)] {
        let word = round.button(&section, toward(-45.0, pitch));
        assert_eq!(word.greyed, greyed, "pitch {pitch}");
        if greyed {
            assert_eq!(word.hover, ALONG_AXIS);
        }
    }
    let kept = section.clone();
    choose_plane(&mut section, &round, toward(-45.0, 88.5));
    assert_eq!(section, kept);
}

/// The same through the window, orthographic and perspective: laid by a
/// click, then the view turned 0.9 and 1.1 degrees about the hole either
/// way; another hole selected (offered, not lit) and the first again
/// (lit); looking down the hole a click on the greyed button changes no
/// field of the section, says why under the plate, and the words go after
/// their 4 seconds.
#[test]
fn review_through_axis_in_the_window() {
    let mut session = drilled();
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "yz", 30.0, "iso");
    select(&mut session, "face", json!({"hole": true, "radius": 3}), 1);
    window.frames(&mut shell, &mut session, 3);
    let dot = shell.section_handle().expect("the plate").dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let (label, _, lit, greyed) = word_of(&mut shell, &mut session);
    assert_eq!((label.as_str(), lit, greyed), ("Through axis", true, false));
    let yaw0 = shell.state.camera.yaw;
    for projection in ["orthographic", "perspective"] {
        for (turn, lit) in [
            (0.0, true),
            (0.9, true),
            (-0.9, true),
            (1.1, false),
            (-1.1, false),
        ] {
            call(
                &mut shell,
                &mut session,
                "ui.set",
                json!({"camera": {"projection": projection, "yaw": yaw0 + turn}}),
            );
            window.frames(&mut shell, &mut session, 2);
            let (label, _, got, greyed) = word_of(&mut shell, &mut session);
            assert_eq!(
                (label.as_str(), got, greyed),
                ("Through axis", lit, false),
                "{projection}, turned {turn} degrees"
            );
        }
    }
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"projection": "orthographic", "yaw": yaw0}}),
    );
    select(&mut session, "face", json!({"hole": true, "radius": 4}), 1);
    window.frames(&mut shell, &mut session, 3);
    let (label, _, lit, _) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), lit),
        ("Through axis", false),
        "another hole"
    );
    select(&mut session, "face", json!({"hole": true, "radius": 3}), 1);
    window.frames(&mut shell, &mut session, 3);
    let (label, _, lit, _) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), lit),
        ("Through axis", true),
        "the first again"
    );
    // Looking down the hole, the section across Z (seen face on).
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"plane": "xy", "offset": 10}, "camera": {"pitch": 88.5}}),
    );
    window.frames(&mut shell, &mut session, 3);
    let (label, hover, lit, greyed) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), hover.as_str(), lit, greyed),
        ("Through axis", ALONG_AXIS, false, true)
    );
    let before = shell.state.section.clone();
    let dot = shell
        .section_handle()
        .expect("the plate, seen from above")
        .dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.state.section, before);
    assert_eq!(shell.section_note(), Some(ALONG_AXIS));
    window.time += 5.0;
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.section_note(), None, "the words go after 4 s");
}

/// A block of 60 × 40 × 20 at (1000, 2000, 3000), far from the origin,
/// with a hole of radius 3 along z through (1015, `hole_y`).
fn far_block(hole_y: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 40, 20], "position": [1000, 2000, 3000], "name": "Block"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 3, "height": 30, "position": [1015, hole_y, 2995], "name": "Hole"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Hole", "name": "Far"}),
    );
    s
}

/// V3, fixes 3 and 9: the first cut's middle goes to a place of the model
/// within 1 % of the range (a hole's axis 0.3 from the middle of 40) and
/// stays the middle where there is none (0.5 from it), inside the range
/// where the plane meets the body, far from the origin and at the scales
/// 1e-3 and 1e6; the agent's first cut is the person's, field by field.
#[test]
fn review_the_first_cut_lies_in_the_model_and_the_agents_is_the_persons() {
    for (hole_y, want) in [(2020.3, 2020.3), (2020.5, 2020.0)] {
        let mut session = far_block(hole_y);
        let (clicked, agent, report) = first_cuts(&mut session);
        assert_eq!(clicked.plane.as_deref(), Some("zx"), "{clicked:?}");
        assert!(
            (clicked.offset - want).abs() < 1e-6,
            "hole at {hole_y}: {clicked:?}"
        );
        let range = &report["range"];
        let (lo, hi) = (range[0].as_f64().unwrap(), range[1].as_f64().unwrap());
        assert!(
            lo <= clicked.offset && clicked.offset <= hi,
            "{lo} {hi} {clicked:?}"
        );
        assert_eq!(agent, clicked, "hole at {hole_y}");
    }
    for scale in [1e-3, 1e6] {
        let mut session = Session::new(Workspace::none());
        run(&mut session, "file.new", json!({"units": "mm"}));
        run(
            &mut session,
            "feature.block",
            json!({"size": [6.0 * scale, 4.0 * scale, 2.0 * scale], "name": "Block"}),
        );
        let (clicked, agent, _) = first_cuts(&mut session);
        assert_eq!(clicked.plane.as_deref(), Some("zx"), "{scale}: {clicked:?}");
        assert!(
            (clicked.offset - 2.0 * scale).abs() <= 1e-12 * scale,
            "{scale}: {clicked:?}"
        );
        assert_eq!(agent, clicked, "{scale}");
    }
}

/// Fix 9 where nothing is shown yet: an empty document. The person's first
/// click lays the plane of the first-plane rule (from the default view,
/// X and Y faced alike and no box: X); the agent's `ui.set` lays Z.
#[test]
fn review_an_agents_first_cut_in_an_empty_document_is_the_persons() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    let (clicked, agent, _) = first_cuts(&mut session);
    assert_eq!(agent, clicked, "the agent's first cut, then the person's");
}

fn window_session(name: &str) -> (Session, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("review-ux-view-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("demo/asm")).unwrap();
    let session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    (session, dir)
}

/// `assembly.rs`'s four-bar: a bar part saved, then an assembly of four
/// instances of it (two lengths) closed by a fixed mate and four
/// revolutes, its crank `Drive`.
fn four_bar(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({ "name": "len", "expr": "24 mm" }));
    run(
        s,
        "variable.publish",
        json!({ "name": "len", "min": "10 mm", "max": "500 mm" }),
    );
    run(
        s,
        "feature.block",
        json!({ "name": "Bar", "size": ["len", 4, 2] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End1", "through": [[2, 2, 0], [2, 2, 1]] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]] }),
    );
    run(s, "file.save", json!({ "path": "demo/asm/bar.3dcraft" }));
    run(s, "assembly.new", json!({}));
    for (name, len) in [
        ("Ground", "64 mm"),
        ("Crank", "24 mm"),
        ("Coupler", "64 mm"),
        ("Rocker", "24 mm"),
    ] {
        run(
            s,
            "instance.add",
            json!({ "source": "demo/asm/bar.3dcraft", "name": name, "body": "Bar",
                    "params": { "len": len } }),
        );
    }
    for (name, a, b) in [
        ("Base", "world/z", "Ground/End1"),
        ("Drive", "Ground/End1", "Crank/End1"),
        ("Elbow", "Crank/End2", "Coupler/End1"),
        ("Knee", "Coupler/End2", "Rocker/End2"),
        ("Hip", "Ground/End2", "Rocker/End1"),
    ] {
        let kind = if name == "Base" { "fixed" } else { "revolute" };
        run(
            s,
            "mate.add",
            json!({ "kind": kind, "a": a, "b": b, "name": name, "flip": true }),
        );
    }
}

/// The box a body's card says, at its head or in a Box row.
fn boxes(shell: &Shell) -> Vec<String> {
    let props = shell.side_props().cloned().expect("the body's card");
    props
        .head
        .iter()
        .filter(|h| h.measure == "box")
        .map(|h| h.text.clone())
        .chain(
            props
                .rows
                .iter()
                .filter(|r| r.label == "Box")
                .map(|r| r.value.clone()),
        )
        .collect()
}

/// V4: *Wall thickness* is "not shown where it cannot run". A body of an
/// instance in the posed four-bar: where the card offers it, a click
/// answers with a wall (the crank is 2 thick), not a refusal.
#[test]
fn review_wall_thickness_is_offered_only_where_it_runs() {
    let (mut s, _dir) = window_session("wall");
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    four_bar(&mut s);
    window.frames(&mut shell, &mut s, 3);
    let crank = shell
        .assembly_outline()
        .expect("an assembly")
        .instances
        .iter()
        .find(|i| i.name == "Crank")
        .expect("the crank")
        .features[0]
        .clone();
    run(
        &mut s,
        "select.set",
        json!({"items": [{"kind": "body", "feature": crank}]}),
    );
    window.frames(&mut shell, &mut s, 3);
    let offered = shell
        .side_hits()
        .targets
        .iter()
        .find(|(n, _)| n == "measure")
        .map(|(_, r)| r.center());
    eprintln!("review: Wall thickness offered on an instance's body: {offered:?}");
    if let Some(at) = offered {
        window.click(&mut shell, &mut s, at);
        window.frames(&mut shell, &mut s, 3);
        let wall = shell.side_wall().cloned();
        eprintln!("review: its answer: {wall:?}");
        assert!(
            wall.as_ref()
                .is_some_and(|w| w.refused.is_none() && w.value.is_some()),
            "offered where it cannot run: {wall:?}"
        );
    }
}

/// Fix 8 against the closed form: the crank (24 × 4 × 2) turned 60 and 37
/// degrees about Z in the posed four-bar reads 24|cos| + 4|sin| ×
/// 24|sin| + 4|cos| × 2, once; a block turned 30 degrees in a part reads
/// 60 cos 30 + 40 sin 30 × 60 sin 30 + 40 cos 30 × 20. And the agent's
/// first cut of the assembly is the person's.
#[test]
fn review_a_placed_bodys_box_is_its_own_by_the_closed_form() {
    for angle in [60.0_f64, 37.0] {
        let (mut s, _dir) = window_session(&format!("box-{angle}"));
        let mut shell = Shell::with_renderer(RendererChoice::Cpu);
        let mut window = Window::new(1000.0, 700.0);
        four_bar(&mut s);
        run(
            &mut s,
            "joint.set",
            json!({ "mate": "Drive", "value": format!("{angle} deg") }),
        );
        window.frames(&mut shell, &mut s, 3);
        let crank = shell
            .assembly_outline()
            .expect("an assembly")
            .instances
            .iter()
            .find(|i| i.name == "Crank")
            .expect("the crank")
            .features[0]
            .clone();
        run(
            &mut s,
            "select.set",
            json!({"items": [{"kind": "body", "feature": crank}]}),
        );
        window.frames(&mut shell, &mut s, 3);
        let (sin, cos) = angle.to_radians().psin_cos();
        let tolerance = s.linear_tolerance().unwrap_or(1e-6);
        let side = |v: f64| shown::length_number(v, tolerance, "mm");
        let want = format!(
            "{} × {} × {} mm",
            side(24.0 * cos.abs() + 4.0 * sin.abs()),
            side(24.0 * sin.abs() + 4.0 * cos.abs()),
            side(2.0)
        );
        let scene = shell.shown_scene_arc().expect("a scene");
        let placed = scene
            .bodies
            .iter()
            .find(|b| b.name == "Crank")
            .and_then(|b| b.mesh.bbox().corners());
        assert_eq!(
            boxes(&shell),
            [want],
            "{angle} degrees, the mesh {placed:?}"
        );
        if angle == 60.0 {
            let (clicked, agent, _) = first_cuts(&mut s);
            assert_eq!(agent, clicked, "the assembly's first cut");
        }
    }
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 40, 20], "angle": "30 deg", "name": "Turned"}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1200.0, 800.0);
    window.frames(&mut shell, &mut s, 3);
    select(&mut s, "body", json!({"in": "Turned"}), 1);
    window.frames(&mut shell, &mut s, 3);
    let (sin, cos) = 30.0_f64.to_radians().psin_cos();
    let side = |v: f64| shown::length_number(v, 1e-6, "mm");
    let want = format!(
        "{} × {} × {} mm",
        side(60.0 * cos + 40.0 * sin),
        side(60.0 * sin + 40.0 * cos),
        side(20.0)
    );
    assert_eq!(boxes(&shell), [want]);
}

/// V4 and V5: a whole hole's number is `Ø 6 mm`; nothing of it was
/// shortened, so its whole value (the hover's, in the card and in the
/// view) is the same text. Before: `6 mm`, without the Ø, so the hover
/// says `6 mm · a click copies Ø 6 mm` for every diameter and radius.
#[test]
fn review_a_holes_number_says_its_whole_value_in_its_own_words() {
    let mut session = plate();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    select(
        &mut session,
        "face",
        json!({"in": "Plate", "hole": true}),
        1,
    );
    window.frames(&mut shell, &mut session, 3);
    let head = shell
        .side_props()
        .and_then(|p| p.head.clone())
        .expect("the hole's number");
    assert_eq!((head.measure, head.text.as_str()), ("diameter", "Ø 6 mm"));
    assert_eq!(head.whole, head.text, "nothing was shortened");
}

/// Two holes selected and their relation kept as a check with a click on
/// Keep as a check, through the window.
fn keep_two_holes(session: &mut Session, shell: &mut Shell, window: &mut Window) {
    window.frames(shell, session, 3);
    call(shell, session, "ui.fit", json!({"view": "iso"}));
    select(session, "face", json!({"in": "Plate", "hole": true}), 2);
    window.frames(shell, session, 3);
    let keep = shell
        .side_hits()
        .targets
        .iter()
        .find(|(n, _)| n == "keep")
        .expect("Keep as a check")
        .1
        .center();
    window.click(shell, session, keep);
    window.frames(shell, session, 4);
}

/// V4, V5: the centre distance √794 reads `28.178 mm`, its hover
/// `28.178006 mm` (within half the tolerance of the value measured), and
/// Keep as a check keeps the value measured, not the text shown.
#[test]
fn review_keep_as_a_check_keeps_the_measured_value_not_its_text() {
    let mut session = plate();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    select(
        &mut session,
        "face",
        json!({"in": "Plate", "hole": true}),
        2,
    );
    window.frames(&mut shell, &mut session, 3);
    let m = shell.measurement().cloned().expect("their relation");
    let value = m.value.expect("a value");
    assert!((value - 28.178_005_607_210_743).abs() < 1e-9, "{m:?}");
    assert_eq!(
        (m.text.as_str(), m.whole.as_str()),
        ("28.178 mm", "28.178006 mm")
    );
    let whole: f64 = m.whole.trim_end_matches(" mm").parse().unwrap();
    assert!((whole - value).abs() <= 5e-7);
    let params = m.check("k").expect("keepable");
    assert_eq!(params["eq"].as_f64(), Some(value), "{params}");
    let mut session2 = plate();
    let mut shell2 = Shell::with_renderer(RendererChoice::Cpu);
    let mut window2 = Window::new(1280.0, 800.0);
    keep_two_holes(&mut session2, &mut shell2, &mut window2);
    let listed = run(&mut session2, "assert.list", json!({}));
    let eq = &listed["assertions"][0]["expect"]["eq"];
    let kept: f64 = match eq {
        Value::String(s) => s
            .split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(f64::NAN),
        other => other.as_f64().unwrap_or(f64::NAN),
    };
    assert!((kept - value).abs() <= 1e-12 * value, "{listed}");
}

/// V4 and V6: *Kept as the check … · Ctrl+Z takes it back* is said in the
/// status bar until the next command; the person's next click in the view
/// selects something else (`select.set`, a command), and the bar then says
/// the hint of what is selected now. (The window's own clicks never set
/// `Shell::last_command`, which the hint waits for: it stays until an
/// agent's command, a file's or an undo.)
#[test]
fn review_the_kept_check_gives_way_to_the_persons_next_click() {
    let mut session = plate();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    keep_two_holes(&mut session, &mut shell, &mut window);
    assert!(
        shell
            .status_hint()
            .is_some_and(|h| h.starts_with("Kept as the check")),
        "{:?}",
        shell.status_hint()
    );
    let middle = shell.viewport_rect().expect("the view").center();
    window.click(&mut shell, &mut session, middle);
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(shell.selection().items.len(), 1, "the click selected");
    assert!(
        !shell
            .status_hint()
            .is_some_and(|h| h.starts_with("Kept as the check")),
        "after another selection the bar still says {:?}",
        shell.status_hint()
    );
}

/// V6: narrower than 800 points the bar shows the selection's words
/// alone; `ui.get` › `hint` is "the hint the status bar showed", so none.
#[test]
fn review_ui_get_says_the_hint_the_bar_shows() {
    let mut session = plate();
    select(
        &mut session,
        "face",
        json!({"in": "Plate", "hole": true}),
        2,
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(780.0, 860.0);
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(
        shell.status_shown(StatusSlot::Selection),
        Some("2 faces selected")
    );
    assert_eq!(shell.status_shown(StatusSlot::Hint), None);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["hint"], Value::Null, "the bar shows no hint at 780");
}

/// What the status bar says of the renderer for an empty document (the
/// count of 0 is never shown): printed for the review.
#[test]
fn review_the_bar_of_an_empty_document() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 4);
    eprintln!(
        "review: an empty document's renderer item: {:?}",
        shell.status_shown(StatusSlot::Render)
    );
}

/// The grammar's Escape for things open together, through the real key
/// path: the section's number typed into and a selection (the field gives
/// up, its offset kept; then the selection); an empty note field and a
/// selection (one Escape closes the field); the Studio popover, the
/// right-click list and a selection (one step each, in that order); the
/// section stays on throughout.
#[test]
fn review_escape_takes_one_step_for_each_pair_open_together() {
    let mut session = drilled();
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "yz", 30.0, "iso");
    select(&mut session, "face", json!({"hole": true, "radius": 3}), 1);
    window.frames(&mut shell, &mut session, 3);
    let number = shell.section_handle().expect("the plate").number.center();
    window.click(&mut shell, &mut session, number);
    window.frames(&mut shell, &mut session, 1);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("12".to_string())],
    );
    window.frames(&mut shell, &mut session, 1);
    assert_eq!(shell.typing().map(|t| t.field), Some(NUMBER_FIELD));
    window.key(&mut shell, &mut session, egui::Key::Escape);
    assert!(shell.typing().is_none(), "the field gave up");
    assert_eq!(shell.state.section.offset, 30.0);
    assert_eq!(shell.selection().items.len(), 1, "the selection kept");
    window.key(&mut shell, &mut session, egui::Key::Escape);
    assert_eq!(shell.selection().items.len(), 0);
    assert!(shell.state.section.on);
    // An empty note field and a selection.
    select(&mut session, "face", json!({"hole": true, "radius": 3}), 1);
    window.frames(&mut shell, &mut session, 3);
    window.key(&mut shell, &mut session, egui::Key::N);
    assert_eq!(
        shell.note_writing().map(|w| w.text.as_str()),
        Some(""),
        "N opened an empty field"
    );
    window.key(&mut shell, &mut session, egui::Key::Escape);
    assert!(shell.note_writing().is_none(), "the empty field closed");
    assert_eq!(shell.selection().items.len(), 1, "the selection kept");
    // The Studio popover, the right-click list and the selection.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"look": {"mode": "studio"}, "panels": {"lookOptions": true}}),
    );
    window.frames(&mut shell, &mut session, 2);
    let [vw, vh] = shell.viewport_size();
    call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": f64::from(vw) / 2.0, "y": f64::from(vh) / 2.0, "secondary": true}),
    );
    window.frames(&mut shell, &mut session, 2);
    let state = |shell: &Shell| {
        (
            shell.look_panel_open(),
            shell.other_menu().is_some(),
            shell.selection().items.len(),
            shell.state.section.on,
        )
    };
    assert_eq!(state(&shell), (true, true, 1, true), "all three open");
    for (k, expected) in [
        (false, true, 1, true),
        (false, false, 1, true),
        (false, false, 0, true),
        (false, false, 0, true),
    ]
    .into_iter()
    .enumerate()
    {
        window.key(&mut shell, &mut session, egui::Key::Escape);
        assert_eq!(state(&shell), expected, "Escape {}", k + 1);
    }
}

/// What `ui.get` only reports (the hint, the measurement, the word button,
/// the words under the plate) cannot be set, a megabyte of it neither, and
/// a refused patch changes nothing.
#[test]
fn review_ui_set_refuses_what_ui_get_only_reports() {
    let mut session = drilled();
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "yz", 30.0, "iso");
    select(&mut session, "face", json!({"hole": true, "radius": 3}), 1);
    window.frames(&mut shell, &mut session, 3);
    let patches = [
        json!({"hint": "Esc clears"}),
        json!({"hint": "x".repeat(1 << 20)}),
        json!({"measurement": {"text": "1 mm"}}),
        json!({"section": {"note": "clamped to 0 – 60 mm"}}),
        json!({"section": {"fromSelection": {"label": "On face", "lit": true}}}),
        json!({"section": {"made": "Midway"}}),
        json!({"section": {"offset": "NaN"}}),
    ];
    for patch in patches {
        let label: String = patch.to_string().chars().take(80).collect();
        let before = serde_json::to_value(&shell.state).unwrap();
        let got = shell
            .handle_method(&mut session, "ui.set", Some(&patch))
            .expect("ui.set");
        assert!(got.is_err(), "{label} was taken");
        assert_eq!(
            serde_json::to_value(&shell.state).unwrap(),
            before,
            "{label}"
        );
    }
}

/// V2: a section on a face that an edit takes away switches off, and the
/// status bar says so in the window's words.
#[test]
fn review_a_lost_face_is_said_in_the_bar() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [60, 40, 20], "name": "Base"}),
    );
    run(
        &mut session,
        "feature.block",
        json!({"size": [20, 20, 10], "position": [80, 0, 0], "name": "Side"}),
    );
    let inspect = run(&mut session, "document.inspect", json!({}));
    let face = inspect["bodies"]
        .as_array()
        .and_then(|b| b.last())
        .and_then(|b| b["faceNames"].get(1))
        .and_then(Value::as_str)
        .expect("the side's top")
        .to_string();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1200.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": true, "plane": face}}),
    );
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.state.section.on, "on {face}");
    run(&mut session, "feature.delete", json!({"feature": "Side"}));
    window.frames(&mut shell, &mut session, 3);
    assert!(!shell.state.section.on);
    assert_eq!(shell.status_hint(), Some(LOST_FACE));
}
