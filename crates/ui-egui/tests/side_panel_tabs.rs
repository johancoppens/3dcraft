//! The side panel of the UI overhaul (docs/design/rendering.md › *The
//! side panel*): tabs by document kind, set and read through `ui.set` and
//! `ui.get`; the Features tab's lines (sketches nested, datums grouped in
//! place, history order kept), key values, dependencies, a failed feature
//! and the features regeneration skipped after it; the checks; a variable
//! edited in place as one undo step; the Bodies tab's totals and the
//! bodies used up; what is selected, measured; and everything inside the
//! panel at 1024 × 700 and in the phone's drawer at 390 points.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::shell::sidepanel::checks::CheckStatus;
use craft3d_ui_egui::{RendererChoice, Shell, SideTab};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Places by what is there.
type Named = Vec<(String, Rect)>;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
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
        self.time += 1.0 / 30.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
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
            let button = Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(shell, session, vec![button]);
        }
    }

    fn key(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        key: egui::Key,
        modifiers: Modifiers,
    ) {
        let event = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        self.frame(shell, session, vec![event(true), event(false)]);
    }
}

fn set(shell: &mut Shell, session: &mut Session, patch: Value) {
    shell
        .handle_method(session, "ui.set", Some(&patch))
        .unwrap()
        .unwrap_or_else(|e| panic!("ui.set {patch}: {e}"));
}

fn get(shell: &mut Shell, session: &mut Session) -> Value {
    shell
        .handle_method(session, "ui.get", None)
        .unwrap()
        .unwrap()
}

/// The housing showcase, with variables driving it, a material and the
/// seven checks of the plan's boards.
fn housing() -> Session {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/showcase/housing.3dcraft");
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.open", json!({"path": path}));
    for (name, expr) in [
        ("base_t", "8 mm"),
        ("sleeve_r", "18 mm"),
        ("bore_d", "22 mm"),
        ("wall", "sleeve_r - bore_d / 2"),
        ("hole_d", "7 mm"),
        ("hole_pitch", "22 mm"),
    ] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "HalfBase", "distance": "base_t"}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Sleeve", "radius": "sleeve_r"}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "HolesRight", "step": ["0", "hole_pitch", "0"]}),
    );
    run(
        &mut s,
        "material.preset",
        json!({"body": "Housing", "preset": "aluminium-brushed"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "Sleeve wall", "measure": "value", "expr": "wall", "min": "5 mm"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "Edge distance", "measure": "value", "expr": "35 mm - hole_pitch - hole_d / 2", "min": "6 mm"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "Height", "measure": "extent", "of": "Housing", "axis": "z", "max": "75 mm"}),
    );
    s
}

fn id_of(shell: &Shell, name: &str) -> String {
    shell
        .tree_rows()
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no row {name}"))
        .id
        .clone()
}

/// Every rectangle that must be whole lies inside `outer`.
fn all_inside(what: &str, outer: Rect, rects: &[(String, Rect)]) {
    for (name, r) in rects {
        assert!(
            r.min.x >= outer.min.x - 0.5
                && r.max.x <= outer.max.x + 0.5
                && r.min.y >= outer.min.y - 0.5
                && r.max.y <= outer.max.y + 0.5,
            "{what}: {name} {r:?} is not inside {outer:?}"
        );
    }
}

#[test]
fn tabs_follow_the_document_and_ui_set() {
    let mut session = housing();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    let tabs: Vec<SideTab> = shell.side_hits().tabs.iter().map(|(t, _)| *t).collect();
    assert_eq!(
        tabs,
        [SideTab::Features, SideTab::Variables, SideTab::Bodies]
    );
    assert_eq!(
        get(&mut shell, &mut session)["state"]["panels"]["sideTab"],
        "features"
    );
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "variables"}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.state.panels.side_tab, SideTab::Variables);
    assert_eq!(
        get(&mut shell, &mut session)["state"]["panels"]["sideTab"],
        "variables"
    );
    assert!(
        shell
            .side_hits()
            .rows
            .iter()
            .any(|(n, _)| n == "hole_pitch")
    );
    // A click on Bodies.
    let bodies = shell
        .side_hits()
        .tabs
        .iter()
        .find(|(t, _)| *t == SideTab::Bodies)
        .unwrap()
        .1;
    window.click(&mut shell, &mut session, bodies.center());
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(
        get(&mut shell, &mut session)["state"]["panels"]["sideTab"],
        "bodies"
    );
    // A tab the document does not offer shows the nearest one it does.
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "assembly"}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.state.panels.side_tab, SideTab::Features);
    // A document with public parameters offers Parameters first, and the
    // tab follows the Parameters memory both ways.
    run(
        &mut session,
        "variable.publish",
        json!({"name": "base_t", "min": "4 mm", "max": "20 mm"}),
    );
    window.frames(&mut shell, &mut session, 3);
    let tabs: Vec<SideTab> = shell.side_hits().tabs.iter().map(|(t, _)| *t).collect();
    assert_eq!(tabs.first(), Some(&SideTab::Parameters));
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "parameters"}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert!(shell.state.parameters.open);
    set(
        &mut shell,
        &mut session,
        json!({"parameters": {"open": false}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.state.panels.side_tab, SideTab::Features);
}

/// The housing's lines: sketches under the feature made from them, the
/// three datums in a row as one group in their place, key values from
/// the parameters; the order is the history's.
#[test]
fn the_feature_lines_keep_the_history_order() {
    let mut session = housing();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    let lines = shell.side_outline();
    let shown: Vec<(&str, &str)> = lines
        .iter()
        .map(|l| (l.name.as_str(), l.key.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            ("Mid", "YZ · 0 mm"),
            ("HalfBase", "Extrude 8 mm"),
            ("OtherHalf", "Mirror · Mid"),
            ("Base", "+ OtherHalf"),
            ("Sleeve", "R18 × 60 mm"),
            ("Pedestal", "Up to Sleeve"),
            ("Stand", "+ Pedestal"),
            ("Solid", "+ Sleeve"),
            ("Bore", "R11 × 70 mm"),
            ("Bored", "− Bore"),
            ("Datums", "axis · 40 mm · 30°"),
            ("Extrude1", "Extrude 24 mm"),
            ("Oiled", "− Extrude1"),
            ("Hole", "R3.5 × 20 mm"),
            ("HolesRight", "3 × 22 mm"),
            ("HolesLeft", "Mirror · Mid"),
            ("DrilledRight", "− HolesRight"),
            ("Housing", "− HolesLeft"),
        ]
    );
    let datums = lines.iter().find(|l| l.name == "Datums").unwrap();
    assert_eq!(datums.members.len(), 3);
    assert_eq!(lines[1].fold, Some(false));
    // Opened: the sketch under its feature, the datums in their place.
    let half_base = id_of(&shell, "HalfBase");
    set(
        &mut shell,
        &mut session,
        json!({"sidePanel": {"unfolded": [half_base, datums.id]}}),
    );
    window.frames(&mut shell, &mut session, 2);
    let names: Vec<(String, u8)> = shell
        .side_outline()
        .iter()
        .map(|l| (l.name.clone(), l.depth))
        .collect();
    let at = |n: &str| names.iter().position(|(name, _)| name == n).unwrap();
    assert_eq!(names[at("HalfOutline")].1, 1);
    assert_eq!(at("HalfOutline"), at("HalfBase") + 1);
    assert_eq!(at("SleeveAxis"), at("Datums") + 1);
    assert_eq!(at("Slope"), at("Datums") + 3);
    assert!(at("Slope") < at("Extrude1"));
    // The filter keeps what holds it, with what it is nested in.
    set(
        &mut shell,
        &mut session,
        json!({"sidePanel": {"filter": "outline"}}),
    );
    window.frames(&mut shell, &mut session, 2);
    let names: Vec<String> = shell
        .side_outline()
        .iter()
        .map(|l| l.name.clone())
        .collect();
    assert_eq!(names, ["HalfBase", "HalfOutline"]);
}

/// The pointer's hover on HalfBase: it uses its outline, OtherHalf and
/// Base use it, and the rest of the history through them is counted.
#[test]
fn the_dependencies_of_a_known_model() {
    let mut session = housing();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    let id = |shell: &Shell, n: &str| id_of(shell, n);
    let deps = shell.side_dependencies(&id(&shell, "HalfBase"));
    assert_eq!(deps.uses, [id(&shell, "HalfOutline")]);
    assert_eq!(deps.used_by, [id(&shell, "OtherHalf"), id(&shell, "Base")]);
    // Through Base: Stand, Solid, Bored, Oiled, DrilledRight, Housing
    // (OtherHalf's own user is Base again).
    assert_eq!(deps.later, 6, "{deps:?}");
    // Shown by ui.set as by the pointer: the strip names them.
    let half_base = id(&shell, "HalfBase");
    set(
        &mut shell,
        &mut session,
        json!({"sidePanel": {"hover": half_base}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.state.side_panel.hover, Some(id(&shell, "HalfBase")));
    // The pointer on another row moves it there.
    let sleeve = id(&shell, "Sleeve");
    let at = shell
        .side_hits()
        .rows
        .iter()
        .find(|(r, _)| *r == sleeve)
        .unwrap()
        .1;
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(at.center())],
    );
    window.frames(&mut shell, &mut session, 1);
    assert_eq!(shell.state.side_panel.hover, Some(sleeve));
}

/// A feature that fails for a reason of its own is the red row with its
/// reason and Show where; what is made from it is not rebuilt (dimmed),
/// the independent features after it are.
#[test]
fn a_failed_feature_and_what_regeneration_skips() {
    let mut session = housing();
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Extrude1", "distance": "0 mm"}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    let summary = shell.side_summary();
    assert_eq!((summary.failed, summary.not_rebuilt), (1, 3), "{summary:?}");
    let rows = shell.tree_rows();
    let row = |n: &str| rows.iter().find(|r| r.name == n).unwrap();
    use craft3d_ui_egui::shell::sidepanel::features::{failed_itself, not_rebuilt};
    assert!(failed_itself(row("Extrude1")));
    assert!(
        row("Extrude1")
            .error
            .as_deref()
            .unwrap()
            .contains("distance")
    );
    for n in ["Oiled", "DrilledRight", "Housing"] {
        assert!(not_rebuilt(row(n)), "{n}");
    }
    for n in ["Hole", "HolesRight", "HolesLeft"] {
        assert!(!not_rebuilt(row(n)) && !failed_itself(row(n)), "{n}");
    }
    // The cause stays in view: the strip of a dimmed row names it.
    let oiled = id_of(&shell, "Oiled");
    assert_eq!(
        shell.side_strip(&oiled).first().map(String::as_str),
        Some("Not rebuilt: Extrude1 failed")
    );
    let extrude = id_of(&shell, "Extrude1");
    let targets = &shell.side_hits().targets;
    assert!(
        targets
            .iter()
            .any(|(t, _)| *t == format!("show where:{extrude}"))
    );
    assert!(targets.iter().any(|(t, _)| *t == format!("edit:{extrude}")));
}

/// The checks: a summary and a row each, measured against the limit; a
/// change that breaks one makes it the failing row with its reason.
#[test]
fn checks_summary_and_rows() {
    let mut session = housing();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    for _ in 0..40 {
        window.frames(&mut shell, &mut session, 1);
        if shell
            .side_checks()
            .0
            .iter()
            .all(|c| c.status != CheckStatus::Pending)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (lines, note) = shell.side_checks();
    assert_eq!(note, "all pass");
    let wall = lines.iter().find(|l| l.name == "Sleeve wall").unwrap();
    assert_eq!(
        (wall.measured.as_str(), wall.limit.as_str()),
        ("7 mm", "≥ 5 mm")
    );
    assert_eq!(shell.side_summary().checks, 3);
    run(
        &mut session,
        "variable.set",
        json!({"name": "hole_pitch", "expr": "26 mm"}),
    );
    for _ in 0..40 {
        window.frames(&mut shell, &mut session, 1);
        if shell
            .side_checks()
            .0
            .iter()
            .all(|c| c.status != CheckStatus::Pending)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (lines, note) = shell.side_checks();
    assert_eq!(note, "1 failing");
    let edge = lines.iter().find(|l| l.name == "Edge distance").unwrap();
    assert_eq!(edge.status, CheckStatus::Failed);
    assert_eq!(edge.measured, "5.5 mm");
    assert!(edge.note.is_some());
    assert_eq!(shell.side_summary().checks_failing, 1);
    assert!(
        shell
            .side_hits()
            .checks
            .iter()
            .any(|(n, _)| n == "Edge distance")
    );
}

fn undo_steps(session: &mut Session) -> usize {
    run(session, "document.inspect", json!({"measures": false}))["history"]["undo"]
        .as_array()
        .unwrap()
        .len()
}

/// A click on a variable's value opens it in place; typed and Enter, it is
/// set by `variable.set` as one undo step. A refusal is said at the field.
#[test]
fn a_variable_edited_in_place_is_one_undo_step() {
    let mut session = housing();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "variables"}}),
    );
    window.frames(&mut shell, &mut session, 3);
    let before = undo_steps(&mut session);
    let edit = |shell: &mut Shell, session: &mut Session, window: &mut Window, text: &str| {
        let at = shell
            .side_hits()
            .values
            .iter()
            .find(|(n, _)| n == "hole_pitch")
            .unwrap()
            .1;
        window.click(shell, session, at.center());
        window.frames(shell, session, 2);
        window.key(shell, session, egui::Key::A, Modifiers::COMMAND);
        window.frame(shell, session, vec![Event::Text(text.to_string())]);
        window.key(shell, session, egui::Key::Enter, Modifiers::NONE);
        window.frames(shell, session, 2);
    };
    edit(&mut shell, &mut session, &mut window, "24 mm");
    let view = shell.side_view().unwrap();
    let pitch = view
        .variables
        .iter()
        .find(|v| v.name == "hole_pitch")
        .unwrap();
    assert_eq!(pitch.expr, "24 mm");
    assert_eq!(undo_steps(&mut session), before + 1);
    // An expression that does not evaluate is kept, as variable.set
    // keeps it, and its reason is said at the field.
    edit(&mut shell, &mut session, &mut window, "24 mm +");
    let pitch = shell
        .side_view()
        .unwrap()
        .variables
        .iter()
        .find(|v| v.name == "hole_pitch")
        .unwrap()
        .clone();
    assert!(pitch.error.is_some(), "{pitch:?}");
    assert_eq!(undo_steps(&mut session), before + 2);
}

/// The Bodies tab: the result body with its measures, the totals, and the
/// bodies booleans used up only with "show consumed".
#[test]
fn bodies_totals_and_the_consumed_filter() {
    let mut session = housing();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "bodies"}}),
    );
    window.frames(&mut shell, &mut session, 3);
    let (bodies, consumed, totals) = shell.side_bodies();
    assert_eq!(bodies.len(), 1);
    assert_eq!(totals.count, 1);
    // The housing's volume, 112.12 cm³, and aluminium's 2.7 g/cm³.
    assert!((totals.volume - 112_123.08).abs() < 1.0, "{totals:?}");
    let mass = totals.mass.unwrap();
    assert!((mass - totals.volume * 2.7e-6).abs() < 1e-6, "{mass}");
    assert!(consumed.len() > 5);
    let rows = |shell: &Shell| {
        shell
            .side_hits()
            .rows
            .iter()
            .map(|(n, _)| n.clone())
            .collect::<Vec<_>>()
    };
    let tool = consumed[0].feature.clone();
    assert!(!rows(&shell).contains(&tool));
    set(
        &mut shell,
        &mut session,
        json!({"sidePanel": {"showConsumed": true}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert!(rows(&shell).contains(&tool));
}

/// What is selected, measured: a block's top face (a plane, 40 × 30, its
/// normal +Z), a cylinder's side (R 5, 2π·5·10) and its rim (a circle,
/// 2π·5 long, at 90° to the cap).
#[test]
fn the_properties_of_a_face_and_an_edge() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [40, 30, 20]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Pin", "radius": 5, "height": 10, "position": [100, 0, 0]}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    let prop = |shell: &Shell, label: &str| {
        shell
            .side_props()
            .unwrap()
            .rows
            .iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("no {label}: {:?}", shell.side_props()))
            .value
            .clone()
    };
    run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"in": "Plate", "normal": {"along": "z"}}, "then": "replace"}),
    );
    window.frames(&mut shell, &mut s, 2);
    assert_eq!(shell.side_props().unwrap().kind, "Face");
    assert_eq!(prop(&shell, "Type"), "Plane");
    assert_eq!(prop(&shell, "Area"), "1200 mm²");
    assert_eq!(prop(&shell, "Normal"), "Z");
    assert_eq!(prop(&shell, "Made by"), "Plate (F1)");
    run(
        &mut s,
        "select.query",
        json!({"kind": "face", "where": {"in": "Pin", "surface": "cylinder"}, "then": "replace"}),
    );
    window.frames(&mut shell, &mut s, 2);
    assert_eq!(prop(&shell, "Type"), "Cylinder  R 5 mm");
    // 2π · 5 · 10 = 314.16.
    assert_eq!(prop(&shell, "Area"), "314 mm²");
    assert!(
        prop(&shell, "Axis").starts_with("Z  through (100, 0, 0)"),
        "{}",
        prop(&shell, "Axis")
    );
    run(
        &mut s,
        "select.query",
        json!({"kind": "edge", "where": {"in": "Pin", "above": {"z": 10}}, "then": "replace"}),
    );
    window.frames(&mut shell, &mut s, 2);
    assert_eq!(shell.side_props().unwrap().kind, "Edge");
    assert_eq!(prop(&shell, "Type"), "Circle  R 5 mm");
    // 2π · 5 = 31.42.
    assert_eq!(prop(&shell, "Length"), "31.42 mm");
    assert_eq!(prop(&shell, "Angle"), "90°");
    assert!(
        shell
            .side_hits()
            .targets
            .iter()
            .any(|(t, _)| t == "measure")
    );
    // The checks are not there: nothing to fold.
}

/// At 1024 × 700 and in the phone's drawer at 390 points, every tab's
/// targets and what is drawn lie inside the panel, and the panel inside
/// the window.
#[test]
fn everything_fits_at_1024_and_in_the_drawer_at_390() {
    for (w, h, drawer) in [(1024.0, 700.0, false), (390.0, 844.0, true)] {
        let mut session = housing();
        run(
            &mut session,
            "select.query",
            json!({"kind": "face", "where": {"madeBy": "Sleeve", "surface": "cylinder"}, "then": "replace", "limit": 1}),
        );
        let mut shell = Shell::with_renderer(RendererChoice::Cpu);
        let mut window = Window::new(w, h);
        if drawer {
            window.frames(&mut shell, &mut session, 2);
            set(
                &mut shell,
                &mut session,
                json!({"panels": {"drawer": true}}),
            );
        }
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(w, h));
        for tab in ["features", "variables", "bodies"] {
            set(
                &mut shell,
                &mut session,
                json!({"panels": {"sideTab": tab}, "sidePanel": {"body": "F23"}}),
            );
            window.frames(&mut shell, &mut session, 12);
            let hits = shell.side_hits().clone();
            let panel = hits.panel.unwrap_or_else(|| panic!("{w}: no panel"));
            let what = format!("{w}×{h} {tab}");
            assert!(
                screen.contains_rect(panel.shrink(0.5)),
                "{what}: panel {panel:?}"
            );
            let named = |list: &[(String, Rect)]| list.to_vec();
            let tabs: Vec<(String, Rect)> = hits
                .tabs
                .iter()
                .map(|(t, r)| (format!("{t:?}"), *r))
                .collect();
            all_inside(&what, panel, &tabs);
            // What lies in a list that scrolls may scroll out at its foot,
            // never past the sides; the rest is whole.
            let in_list = |name: &str| {
                [
                    "rollback",
                    "show where:",
                    "edit:",
                    "material",
                    "show consumed",
                    "add variable",
                    "grip:",
                ]
                .iter()
                .any(|p| name.starts_with(p))
            };
            let (scrolled, fixed): (Named, Named) = named(&hits.targets)
                .into_iter()
                .partition(|(n, _)| in_list(n));
            all_inside(&what, panel, &fixed);
            all_inside(&what, panel, &named(&hits.checks));
            let drawn: Vec<(String, Rect)> = hits
                .drawn
                .iter()
                .map(|r| ("drawn".to_string(), *r))
                .collect();
            for (n, r) in hits
                .rows
                .iter()
                .chain(&hits.values)
                .chain(&hits.eyes)
                .chain(&scrolled)
                .chain(&drawn)
            {
                assert!(
                    r.min.x >= panel.min.x - 0.5 && r.max.x <= panel.max.x + 0.5,
                    "{what}: {n} {r:?} sticks out of {panel:?}"
                );
            }
            assert!(!hits.rows.is_empty(), "{what}: no rows");
        }
    }
}

/// A click on a feature's key value opens it in place, with the
/// expression as written; Enter sets it by `feature.edit`, one undo step.
#[test]
fn a_key_value_edited_in_place_is_one_undo_step() {
    let mut session = housing();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    let before = undo_steps(&mut session);
    let sleeve = id_of(&shell, "Sleeve");
    let row = shell
        .side_hits()
        .rows
        .iter()
        .find(|(n, _)| *n == sleeve)
        .unwrap()
        .1;
    // A click selects the row (the first click, wherever it lands); the
    // pointer on it makes its value a field.
    window.click(
        &mut shell,
        &mut session,
        row.left_center() + vec2(40.0, 0.0),
    );
    window.frames(&mut shell, &mut session, 1);
    assert_eq!(
        shell.state.selected_feature.as_deref(),
        Some(sleeve.as_str())
    );
    let at = shell
        .side_hits()
        .values
        .iter()
        .find(|(n, _)| *n == sleeve)
        .unwrap()
        .1;
    window.click(&mut shell, &mut session, at.center());
    window.frames(&mut shell, &mut session, 2);
    window.key(&mut shell, &mut session, egui::Key::A, Modifiers::COMMAND);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("20 mm".to_string())],
    );
    window.key(&mut shell, &mut session, egui::Key::Enter, Modifiers::NONE);
    window.frames(&mut shell, &mut session, 2);
    let line = shell
        .side_outline()
        .into_iter()
        .find(|l| l.name == "Sleeve")
        .unwrap();
    assert_eq!(line.key, "R20 × 60 mm");
    assert_eq!(undo_steps(&mut session), before + 1);
}

fn entry(id: &str, uses: &[&str]) -> craft3d_engine::HistoryEntry {
    craft3d_engine::HistoryEntry {
        id: id.to_string(),
        name: id.to_string(),
        kind: "block",
        op: None,
        is_sketch: false,
        is_datum: false,
        key: String::new(),
        edit: None,
        uses: uses.iter().map(|u| u.to_string()).collect(),
        curves: String::new(),
    }
}

/// The hover's cost on a history of 2000 features is a walk over
/// what depends on the feature, not over the history per user: 200
/// hovers of the first feature of a chain stay well within a second.
#[test]
fn the_hover_of_a_long_history_is_bounded() {
    let features: Vec<craft3d_engine::HistoryEntry> = (0..2000)
        .map(|k| {
            let id = format!("F{}", k + 1);
            if k == 0 {
                entry(&id, &[])
            } else {
                let prev = format!("F{k}");
                entry(&id, &[prev.as_str()])
            }
        })
        .collect();
    let started = std::time::Instant::now();
    let graph = craft3d_ui_egui::shell::sidepanel::features::Graph::of(&features);
    let mut later = 0;
    for _ in 0..200 {
        later = graph.dependencies(&features, "F1").later;
    }
    assert_eq!(later, 1998);
    let deps = graph.dependencies(&features, "F1000");
    assert_eq!(
        (deps.uses, deps.used_by),
        (vec!["F999".to_string()], vec!["F1001".to_string()])
    );
    assert!(
        started.elapsed().as_secs_f64() < 2.0,
        "{:?}",
        started.elapsed()
    );
}

fn feature_order(session: &mut Session) -> Vec<String> {
    run(session, "document.inspect", json!({"measures": false}))["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_string())
        .collect()
}

fn target(shell: &Shell, name: &str) -> Rect {
    shell
        .side_hits()
        .targets
        .iter()
        .find(|(t, _)| t == name)
        .unwrap_or_else(|| panic!("no target {name}: {:?}", shell.side_hits().targets))
        .1
}

fn row_rect(shell: &Shell, name: &str) -> Rect {
    let id = id_of(shell, name);
    shell
        .side_hits()
        .rows
        .iter()
        .find(|(r, _)| *r == id)
        .unwrap_or_else(|| panic!("no row {name}"))
        .1
}

/// Presses at `from`, moves to `to` in steps, releases there.
fn drag(window: &mut Window, shell: &mut Shell, session: &mut Session, from: Pos2, to: Pos2) {
    window.frame(shell, session, vec![Event::PointerMoved(from)]);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    window.frame(shell, session, vec![button(from, true)]);
    for k in 1..=8 {
        let at = from + (to - from) * (k as f32 / 8.0);
        window.frame(shell, session, vec![Event::PointerMoved(at)]);
    }
    window.frame(shell, session, vec![button(to, false)]);
    window.frames(shell, session, 2);
}

/// A row dragged by its grip to another place: `feature.move` once, one
/// undo step; a drop outside the list does nothing.
#[test]
fn a_row_dragged_by_its_grip_moves_its_feature() {
    let mut session = housing();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 900.0);
    window.frames(&mut shell, &mut session, 3);
    let before = undo_steps(&mut session);
    let order = feature_order(&mut session);
    let hole = row_rect(&shell, "Hole");
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(hole.center())],
    );
    window.frames(&mut shell, &mut session, 1);
    let grip = target(&shell, &format!("grip:{}", id_of(&shell, "Hole"))).center();
    // Dropped over the viewport: nothing.
    drag(
        &mut window,
        &mut shell,
        &mut session,
        grip,
        pos2(700.0, 300.0),
    );
    assert_eq!(feature_order(&mut session), order);
    assert_eq!(undo_steps(&mut session), before);
    // Dropped on the top of the Datums group: Hole before the datums.
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(hole.center())],
    );
    window.frames(&mut shell, &mut session, 1);
    let datums = shell
        .side_outline()
        .iter()
        .position(|l| l.name == "Datums")
        .unwrap();
    let datums_rect = shell.side_hits().rows[datums].1;
    drag(
        &mut window,
        &mut shell,
        &mut session,
        grip,
        datums_rect.left_top() + vec2(60.0, 3.0),
    );
    let moved = feature_order(&mut session);
    assert_eq!(undo_steps(&mut session), before + 1, "one feature.move");
    let at = |n: &str| moved.iter().position(|m| m == n).unwrap();
    assert_eq!(at("Hole") + 1, at("SleeveAxis"), "{moved:?}");
    assert!(at("Hole") > at("Bored"));
}

/// The roll-back bar dragged up: `history.rollback` after the line above
/// the drop; the steps after it are rolled back.
#[test]
fn the_roll_back_bar_dragged_rolls_back() {
    let mut session = housing();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 900.0);
    window.frames(&mut shell, &mut session, 3);
    let bar = target(&shell, "rollback").center();
    let datums = shell
        .side_outline()
        .iter()
        .position(|l| l.name == "Datums")
        .unwrap();
    let datums_rect = shell.side_hits().rows[datums].1;
    drag(
        &mut window,
        &mut shell,
        &mut session,
        bar,
        datums_rect.left_top() + vec2(60.0, 3.0),
    );
    let got = run(&mut session, "document.inspect", json!({"measures": false}));
    assert_eq!(
        got["rollback"],
        id_of(&shell, "Bored"),
        "{}",
        got["rollback"]
    );
    assert!(shell.side_summary().rolled_back.is_some());
}

/// Each item of a row's menu runs its command.
#[test]
fn the_menu_items_run_their_commands() {
    let mut session = housing();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 900.0);
    window.frames(&mut shell, &mut session, 3);
    let feature = |session: &mut Session, name: &str| -> Option<Value> {
        run(session, "document.inspect", json!({"measures": false}))["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == name)
            .cloned()
    };
    let mut choose = |shell: &mut Shell, session: &mut Session, name: &str, item: &str| {
        let id = id_of(shell, name);
        set(shell, session, json!({"sidePanel": {"menu": id}}));
        window.frames(shell, session, 2);
        let at = target(shell, &format!("menu:{item}")).center();
        window.click(shell, session, at);
        window.frames(shell, session, 2);
        assert_eq!(shell.state.side_panel.menu, None, "{item} closes the menu");
    };
    choose(&mut shell, &mut session, "Oiled", "Suppress");
    assert_eq!(feature(&mut session, "Oiled").unwrap()["suppressed"], true);
    choose(&mut shell, &mut session, "Sleeve", "Hide");
    assert_eq!(feature(&mut session, "Sleeve").unwrap()["hidden"], true);
    choose(&mut shell, &mut session, "Bored", "Roll back to here");
    let got = run(&mut session, "document.inspect", json!({"measures": false}));
    assert_eq!(got["rollback"], id_of(&shell, "Bored"));
    // The last row offers the way forward instead: the marker goes.
    choose(&mut shell, &mut session, "Housing", "Roll forward to end");
    let got = run(&mut session, "document.inspect", json!({"measures": false}));
    assert_eq!(got["rollback"], serde_json::Value::Null);
    choose(&mut shell, &mut session, "Housing", "Delete");
    assert!(feature(&mut session, "Housing").is_none());
}

/// A host whose bodies are built elsewhere (the page's worker).
fn elsewhere() -> Session {
    let mut session = Session::new(Workspace::none());
    session.set_stepwise(true);
    session.set_checks_elsewhere(true);
    session.set_regen_elsewhere(true);
    session
}

/// Answers the scenes the window waits for, as the worker does.
fn answer(session: &mut Session, quality: craft3d_engine::SceneQuality) {
    for _ in 0..10 {
        let Some(token) = session.scene_job(quality) else {
            return;
        };
        let request = session.scene_request(token).unwrap();
        craft3d_engine::scene_begin(&request.document, &request.work);
        while let Some(part) = craft3d_engine::scene_next() {
            assert!(session.scene_answered(token, Ok(&part)));
        }
        assert!(session.scene_done(token));
    }
}

/// One frame of a window over a host that builds elsewhere; the delay
/// to the next frame it asks for.
fn web_frame(
    ctx: &egui::Context,
    shell: &mut Shell,
    session: &mut Session,
    time: f64,
) -> std::time::Duration {
    answer(session, shell.scene_quality());
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
        time: Some(time),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
    output.textures_delta.clear();
    output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map_or(std::time::Duration::MAX, |v| v.repaint_delay)
}

/// The web: the Bodies tab neither regenerates the page's state nor asks
/// for a frame after frame; Measure is off while the bodies are elsewhere.
#[test]
fn on_the_web_the_side_panel_neither_regenerates_nor_spins() {
    let ctx = egui::Context::default();
    let mut shell = Shell::new();
    let mut session = elsewhere();
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "variable.set",
        json!({"name": "d", "expr": "10 mm"}),
    );
    run(
        &mut session,
        "variable.publish",
        json!({"name": "d", "min": "1 mm", "max": "100 mm"}),
    );
    run(
        &mut session,
        "feature.block",
        json!({"size": ["d", 20, 5], "name": "A"}),
    );
    web_frame(&ctx, &mut shell, &mut session, 0.0);
    // A face selected (a command: it regenerates here), then an edit left
    // to the worker, as the Parameters form leaves it.
    run(&mut session, "select.set", json!({"names": ["F1:face(0)"]}));
    run(&mut session, "param.set", json!({"values": {"d": "12 mm"}}));
    assert!(!session.regenerated_here());
    let mut time = 1.0;
    for _ in 0..4 {
        web_frame(&ctx, &mut shell, &mut session, time);
        time += 0.5;
    }
    let props = shell.side_props().cloned().expect("the face's card");
    assert!(props.measure.is_none(), "Measure is off: {props:?}");
    assert_eq!(props.shape, None);
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "bodies"}}),
    );
    let mut delay = std::time::Duration::ZERO;
    for _ in 0..6 {
        delay = web_frame(&ctx, &mut shell, &mut session, time);
        time += 0.5;
        assert!(
            !session.regenerated_here(),
            "the Bodies tab regenerated the page's state"
        );
    }
    assert!(
        delay > std::time::Duration::from_millis(100),
        "{delay:?}: a frame after frame"
    );
    assert!(shell.side_bodies().0.is_empty());
}

/// The Bodies tab and the card of a selected body show volume, area and
/// mass, never the inertia: an edit and an undo with them open integrate
/// no inertia (the fuel tank, 2026-11-04: 23 s of inertia after every
/// undo, felt as a slow undo).
#[test]
fn an_undo_with_the_bodies_tab_open_integrates_no_inertia() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "variable.set",
        json!({"name": "r", "expr": "10 mm"}),
    );
    run(
        &mut session,
        "feature.sphere",
        json!({"name": "Ball", "radius": "r"}),
    );
    run(
        &mut session,
        "body.material",
        json!({"body": "Ball", "name": "Steel", "density": 7850}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1280.0, 800.0);
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"panels": {"sideTab": "bodies"}})),
        )
        .unwrap()
        .unwrap();
    run(
        &mut session,
        "select.set",
        json!({"items": [{"kind": "body", "feature": "Ball"}]}),
    );
    let before = craft3d_engine::fresh_inertias();
    window.frames(&mut shell, &mut session, 4);
    run(
        &mut session,
        "variable.set",
        json!({"name": "r", "expr": "12 mm"}),
    );
    window.frames(&mut shell, &mut session, 4);
    run(&mut session, "edit.undo", json!({}));
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(
        craft3d_engine::fresh_inertias(),
        before,
        "no inertia integrated"
    );
    // The tab still lists the body's mass.
    let shown = format!("{:?}", shell.side_bodies());
    assert!(shown.contains("mass: Some("), "{shown}");
}
