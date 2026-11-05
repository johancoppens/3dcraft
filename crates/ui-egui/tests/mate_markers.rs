//! An assembly's mates in the window (docs/design/assemblies.md › *Mate
//! markers*): a marker per mate at its frame, the same size on screen at
//! any zoom, hidden with `show.mates`; pointing at a mate's row or marker
//! highlights it and tints its parts, a click chooses it; under each row a
//! joint that can be set by hand has its slider, one drag one undo step.
//! Driven through real frames of the shell. The GPU's picture with the
//! markers is compared with the oracle in `gpu_viewport.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, MateFrame, OutlineMate, Session, Workspace};
use craft3d_math::{Portable, Vec3};
use craft3d_render::Highlight;
use craft3d_ui_egui::viewport::{self, LineKind, Marks};
use craft3d_ui_egui::{Shell, Tokens, mates};
use egui::{Pos2, vec2};
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
        .unwrap()
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
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

    fn events(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session) {
        self.events(shell, session, Vec::new());
    }

    fn frames(&mut self, shell: &mut Shell, session: &mut Session, n: usize) {
        for _ in 0..n {
            self.frame(shell, session);
        }
    }

    fn hover(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.events(shell, session, vec![egui::Event::PointerMoved(at)]);
        self.frames(shell, session, 2);
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.events(shell, session, vec![egui::Event::PointerMoved(at)]);
        for pressed in [true, false] {
            self.events(
                shell,
                session,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        self.frames(shell, session, 3);
    }

    fn drag(&mut self, shell: &mut Shell, session: &mut Session, from: Pos2, to: Pos2) {
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.events(shell, session, vec![egui::Event::PointerMoved(from)]);
        self.events(shell, session, vec![button(from, true)]);
        for k in 1..=8 {
            let at = from + (to - from) * (k as f32 / 8.0);
            self.events(shell, session, vec![egui::Event::PointerMoved(at)]);
        }
        self.events(shell, session, vec![button(to, false)]);
        self.frames(shell, session, 3);
    }
}

fn window_session(name: &str) -> Session {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ui-mates-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("demo/asm")).unwrap();
    Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    })
}

/// A bar part and an assembly of four of it closed into a four-bar: a
/// fixed mate `Base` to the world's z axis and four revolutes, the crank's
/// `Drive` set to 60°.
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
    run(
        s,
        "joint.set",
        json!({ "mate": "Drive", "value": "60 deg" }),
    );
}

fn opened(name: &str) -> (Session, Shell, Window) {
    let mut s = window_session(name);
    four_bar(&mut s);
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frames(&mut shell, &mut s, 4);
    (s, shell, window)
}

fn v(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn frame_of(shell: &Shell, mate: &str) -> MateFrame {
    let outline = shell.assembly_outline().expect("an assembly");
    outline
        .mates
        .iter()
        .find(|m| m.name == mate)
        .and_then(|m| m.frame)
        .unwrap_or_else(|| panic!("{mate} has no frame: {outline:?}"))
}

/// Each mate's marker is at its frame: the crank's `Drive` on the world's
/// z axis where `Base` holds the ground, its axis along z; `Elbow` at the
/// crank's far end, 20 mm out at 60°. Its parts are the two instances.
#[test]
fn a_mate_s_marker_is_at_its_frame() {
    let (_s, shell, _w) = opened("frame");
    let outline = shell.assembly_outline().unwrap().clone();
    let drive = outline.mates.iter().find(|m| m.name == "Drive").unwrap();
    assert_eq!(drive.parts, ["Ground".to_string(), "Crank".to_string()]);
    let base = outline.mates.iter().find(|m| m.name == "Base").unwrap();
    assert_eq!(base.parts, ["world".to_string(), "Ground".to_string()]);
    for name in ["Base", "Drive"] {
        let f = frame_of(&shell, name);
        assert!(
            f.origin[0].abs() < 1e-6 && f.origin[1].abs() < 1e-6,
            "{name}: {f:?}"
        );
        assert!(
            (v(f.z).dot(Vec3::Z).abs() - 1.0).abs() < 1e-9,
            "{name}: {f:?}"
        );
        assert!(v(f.x).dot(v(f.z)).abs() < 1e-9);
    }
    let elbow = frame_of(&shell, "Elbow");
    let at = 60.0_f64.to_radians();
    assert!(
        (elbow.origin[0] - 20.0 * at.pcos()).abs() < 1e-3
            && (elbow.origin[1] - 20.0 * at.psin()).abs() < 1e-3,
        "{elbow:?}"
    );
    // The marker drawn: around each frame's origin, within its length.
    let marks = shell.mate_marks();
    assert!(!marks.mates.is_empty() && marks.mates_hot.is_empty());
    let origins: Vec<Vec3> = outline
        .mates
        .iter()
        .map(|m| v(m.frame.unwrap().origin))
        .collect();
    let longest = marks
        .mates
        .iter()
        .map(|[a, b]| (*b - *a).length())
        .fold(0.0, f64::max);
    for [a, b] in &marks.mates {
        for p in [a, b] {
            let near = origins
                .iter()
                .map(|o| (*p - *o).length())
                .fold(f64::INFINITY, f64::min);
            assert!(near <= longest + 1e-9, "{p:?} is by no frame");
        }
    }
    // The revolute's axis is drawn from its origin along its z.
    let o = v(frame_of(&shell, "Drive").origin);
    let z = v(frame_of(&shell, "Drive").z);
    assert!(marks.mates.iter().any(|[a, b]| (*a - o).length() < 1e-9
        && ((*b - *a).normalized().unwrap().dot(z) - 1.0).abs() < 1e-9));
}

/// A marker keeps its size on screen: twice the camera's distance, twice
/// its length in the world; with `tokens.mate_size` points at the target.
#[test]
fn a_marker_keeps_its_size_on_screen() {
    let (mut s, mut shell, mut window) = opened("size");
    let axis = |marks: &Marks, o: Vec3| {
        marks
            .mates
            .iter()
            .filter(|[a, _]| (*a - o).length() < 1e-9)
            .map(|[a, b]| (*b - *a).length())
            .fold(0.0, f64::max)
    };
    let o = v(frame_of(&shell, "Drive").origin);
    let near = axis(&shell.mate_marks(), o);
    let camera = shell.state.camera;
    let mut far = camera;
    far.distance *= 2.0;
    far.auto_fit = false;
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({ "camera": { "distance": far.distance, "autoFit": false } }),
    );
    window.frames(&mut shell, &mut s, 2);
    let distant = axis(&shell.mate_marks(), o);
    assert!((distant / near - 2.0).abs() < 1e-9, "{near} then {distant}");
    let length = mates::world_size(
        &shell.state.camera,
        shell.viewport_size()[0],
        shell.viewport_size()[1],
        1.0,
        Tokens::default().mate_size,
    )
    .unwrap();
    assert!(
        (distant - length).abs() < 1e-9 * length,
        "{distant} against {length}"
    );
}

/// `ui.set {"show": {"mates": false}}` hides them, `ui.get` says so, a
/// screenshot loses them; the display bar's first toggle is Mates.
#[test]
fn ui_set_hides_and_shows_the_mates() {
    let (mut s, mut shell, mut window) = opened("toggle");
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    assert_eq!(got["state"]["show"]["mates"], true);
    assert!(
        shell.show_hits().iter().any(|(n, _)| *n == "mates"),
        "{:?}",
        shell.show_hits()
    );
    assert!(!shell.show_hits().iter().any(|(n, _)| *n == "sketches"));
    let shot = |shell: &mut Shell, s: &mut Session| {
        call(
            shell,
            s,
            "ui.screenshot",
            json!({"width": 480, "height": 360, "scale": 1.0}),
        )["overlayCoverage"]
            .as_f64()
            .unwrap()
    };
    let shown = shot(&mut shell, &mut s);
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"show": {"mates": false}}),
    );
    window.frames(&mut shell, &mut s, 2);
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    assert_eq!(got["state"]["show"]["mates"], false);
    assert!(shell.mate_marks().mates.is_empty());
    let hidden = shot(&mut shell, &mut s);
    assert!(shown > hidden, "{shown} against {hidden}");
    // The toggle turns them on again.
    let chip = shell
        .show_hits()
        .iter()
        .find(|(n, _)| *n == "mates")
        .map(|(_, r)| r.center())
        .unwrap();
    window.click(&mut shell, &mut s, chip);
    assert!(shell.state.show.mates);
    assert!(!shell.mate_marks().mates.is_empty());
}

/// Pointing at a mate's row highlights its marker and tints its two
/// parts; a click chooses it; `ui.set` points at one as the pointer does.
#[test]
fn a_mate_s_row_highlights_its_marker_and_parts() {
    let (mut s, mut shell, mut window) = opened("row");
    let row = |shell: &Shell, name: &str| {
        shell
            .mate_row_hits()
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, r)| *r)
            .unwrap_or_else(|| panic!("no row of {name}: {:?}", shell.mate_row_hits()))
    };
    let scene = shell.shown_scene_arc().unwrap();
    assert_eq!(shell.face_highlight(Some(&scene)), Highlight::NONE);
    let at_ = row(&shell, "Elbow").center();
    window.hover(&mut shell, &mut s, at_);
    assert_eq!(shell.state.side_panel.mate_hover.as_deref(), Some("Elbow"));
    let marks = shell.mate_marks();
    assert!(!marks.mates_hot.is_empty());
    let elbow = v(frame_of(&shell, "Elbow").origin);
    assert!(
        marks
            .mates_hot
            .iter()
            .any(|[a, _]| (*a - elbow).length() < 1e-9)
    );
    // The crank and the coupler are tinted, the ground is not.
    let scene = shell.shown_scene_arc().unwrap();
    let lit = shell.face_highlight(Some(&scene));
    assert_eq!(lit.color, Tokens::default().mate_part);
    let faces_of = |name: &str| {
        let b = scene.bodies.iter().find(|b| b.name == name).unwrap();
        b.first_face_id
    };
    assert!(lit.contains(faces_of("Crank")) && lit.contains(faces_of("Coupler")));
    assert!(!lit.contains(faces_of("Ground")));
    // Off the rows the highlight goes.
    window.hover(&mut shell, &mut s, egui::pos2(990.0, 10.0));
    assert_eq!(shell.state.side_panel.mate_hover, None);
    // A click chooses it, a second lets it go.
    let at_ = row(&shell, "Knee").center();
    window.click(&mut shell, &mut s, at_);
    assert_eq!(shell.state.side_panel.mate.as_deref(), Some("Knee"));
    assert!(!shell.mate_marks().mates_hot.is_empty());
    let at_ = row(&shell, "Knee").center();
    window.click(&mut shell, &mut s, at_);
    assert_eq!(shell.state.side_panel.mate, None);
    // From outside, the pointer off the rows.
    window.hover(&mut shell, &mut s, egui::pos2(990.0, 10.0));
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"sidePanel": {"mateHover": "Hip"}}),
    );
    window.frames(&mut shell, &mut s, 2);
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    assert_eq!(got["state"]["sidePanel"]["mateHover"], "Hip");
    assert!(!shell.mate_marks().mates_hot.is_empty());
}

/// Pointing at a marker in the viewport names its mate and highlights
/// it; a click on it chooses the mate's row, on the Assembly tab.
#[test]
fn a_marker_is_pointed_at_and_chosen() {
    let (mut s, mut shell, mut window) = opened("marker");
    let at = |shell: &Shell, name: &str| {
        shell
            .mate_marker_hits()
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, p)| *p)
            .unwrap_or_else(|| panic!("no marker of {name}: {:?}", shell.mate_marker_hits()))
    };
    let knee = at(&shell, "Knee");
    window.hover(&mut shell, &mut s, knee);
    assert_eq!(shell.state.side_panel.mate_hover.as_deref(), Some("Knee"));
    let at_ = at(&shell, "Knee");
    window.click(&mut shell, &mut s, at_);
    assert_eq!(shell.state.side_panel.mate.as_deref(), Some("Knee"));
    assert_eq!(
        shell.state.panels.side_tab,
        craft3d_ui_egui::state::SideTab::Assembly
    );
    // The click picked nothing: the selection is as it was.
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    assert_eq!(got["selection"]["count"], 0, "{}", got["selection"]);
}

/// Under the crank's row is its joint's slider: dragging it sets the
/// joint, the assembly stays solved, and the drag is one undo step.
#[test]
fn a_joint_s_slider_under_its_mate_row_sets_it_in_one_undo_step() {
    let (mut s, mut shell, mut window) = opened("slider");
    let value = |s: &mut Session| {
        let inspected = run(s, "assembly.inspect", json!({}));
        inspected["mates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == "Drive")
            .and_then(|m| m["value"].as_f64())
            .unwrap()
    };
    let hits = shell.joint_slider_hits().to_vec();
    let names: Vec<&str> = hits.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["Drive"], "the Assembly tab's sliders");
    let row = shell
        .mate_row_hits()
        .iter()
        .find(|(n, _)| n == "Drive")
        .map(|(_, r)| *r)
        .unwrap();
    let rect = hits[0].1;
    assert!(
        rect.top() >= row.bottom() - 0.5,
        "under its row: {rect:?} {row:?}"
    );
    let before = value(&mut s);
    let from = egui::pos2(rect.right() - rect.width() * 0.45, rect.center().y);
    let to = egui::pos2(rect.right() - rect.width() * 0.38, rect.center().y);
    window.drag(&mut shell, &mut s, from, to);
    let after = value(&mut s);
    assert!((after - before).abs() > 1e-3, "{} deg", after.to_degrees());
    let inspected = run(&mut s, "assembly.inspect", json!({}));
    assert_eq!(inspected["solve"]["ok"], true, "{inspected}");
    run(&mut s, "edit.undo", json!({}));
    let undone = value(&mut s);
    assert!(
        (undone.to_degrees() - 60.0).abs() < 1e-6,
        "{}",
        undone.to_degrees()
    );
}

/// Both paths of the viewport draw the markers from the same list: the
/// GPU's segments and the CPU's overlay lines of the mates are the marks'
/// lines, in the theme's colours and widths.
#[test]
fn both_paths_draw_the_markers_from_one_list() {
    let (mut s, mut shell, mut window) = opened("paths");
    let drive = shell
        .mate_row_hits()
        .iter()
        .find(|(n, _)| n == "Drive")
        .unwrap()
        .1
        .center();
    window.hover(&mut shell, &mut s, drive);
    let tokens = Tokens::default();
    let scene = shell.shown_scene_arc().unwrap();
    let marks = shell.mate_marks();
    assert!(!marks.mates.is_empty() && !marks.mates_hot.is_empty());
    let segments =
        craft3d_ui_egui::direct::segments_marked(Some(&scene), None, &tokens, 1.0, &marks, false);
    for (lines, ink, width) in [
        (&marks.mates, tokens.mate, tokens.mate_width),
        (
            &marks.mates_hot,
            tokens.face_highlight,
            tokens.mate_hot_width,
        ),
    ] {
        let drawn: Vec<[Vec3; 2]> = segments
            .iter()
            .filter(|g| {
                g.color == ink && g.width == width && g.edge.is_none() && g.tested && g.dimmed
            })
            .map(|g| [g.from, g.to])
            .collect();
        for line in lines {
            assert!(drawn.contains(line), "{line:?} not drawn on the GPU");
        }
    }
    let [w, h] = shell.viewport_size();
    let overlay = viewport::overlay_untested(Some(&scene), &shell.state.camera, w, h, None, &marks);
    let count = |kind: LineKind| overlay.lines.iter().filter(|l| l.kind == kind).count();
    assert!(count(LineKind::Mate) > 0 && count(LineKind::MateHot) > 0);
    assert_eq!(LineKind::Mate.color(&tokens), tokens.mate);
    assert_eq!(LineKind::MateHot.color(&tokens), tokens.face_highlight);
}

/// Hostile mates never panic and draw nothing guessed: a frame with a
/// zero axis, axes not square, numbers that are not finite; a mate whose
/// face is gone fails with no frame and no marker, the others stay.
#[test]
fn hostile_mates_draw_no_marker_and_never_panic() {
    let mate = |frame: Option<MateFrame>| OutlineMate {
        name: "M".into(),
        kind: "revolute",
        status: "ok",
        error: None,
        value: None,
        driven: false,
        variables: Vec::new(),
        settable: false,
        limits: None,
        parts: ["Nowhere".into(), "world".into()],
        frame,
    };
    let f = |origin: [f64; 3], x: [f64; 3], z: [f64; 3]| Some(MateFrame { origin, x, z });
    for hostile in [
        None,
        f([0.0; 3], [1.0, 0.0, 0.0], [0.0; 3]),
        f([0.0; 3], [0.0; 3], [0.0, 0.0, 1.0]),
        f([0.0; 3], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        f([f64::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        f([0.0; 3], [f64::INFINITY, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ] {
        let m = mate(hostile);
        assert!(mates::markers([&m], 5.0).is_empty(), "{hostile:?}");
        for length in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e300] {
            let _ = mates::markers([&m], length);
        }
    }
    // The bar loses its datum End2: the mates on it (Elbow, Knee, Hip)
    // fail, with no frame and no marker; Base and Drive keep theirs. A
    // failed mate pointed at still tints its two parts (they are shown
    // where they are, which says what it joins); a mate that is not
    // there tints nothing.
    let (mut s, mut shell, mut window) = opened("hostile");
    let before = shell.mate_marks().mates.len();
    run(&mut s, "file.activate", json!({"document": 0}));
    run(&mut s, "feature.delete", json!({"feature": "End2"}));
    run(&mut s, "file.save", json!({"path": "demo/asm/bar.3dcraft"}));
    run(&mut s, "file.activate", json!({"document": 1}));
    run(&mut s, "assembly.reload", json!({}));
    window.frames(&mut shell, &mut s, 4);
    let outline = shell.assembly_outline().unwrap().clone();
    for name in ["Elbow", "Knee", "Hip"] {
        let m = outline.mates.iter().find(|m| m.name == name).unwrap();
        assert_eq!(m.status, "failed", "{m:?}");
        assert!(m.frame.is_none(), "{m:?}");
    }
    for name in ["Base", "Drive"] {
        let m = outline.mates.iter().find(|m| m.name == name).unwrap();
        assert!(m.frame.is_some(), "{m:?}");
    }
    let marks = shell.mate_marks();
    assert!(!marks.mates.is_empty() && marks.mates.len() < before);
    let names: Vec<&str> = shell
        .mate_marker_hits()
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(names, ["Base", "Drive"]);
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"sidePanel": {"mateHover": "Elbow"}}),
    );
    window.frames(&mut shell, &mut s, 2);
    assert!(shell.mate_marks().mates_hot.is_empty());
    let scene = shell.shown_scene_arc().unwrap();
    assert_eq!(
        shell.face_highlight(Some(&scene)).color,
        Tokens::default().mate_part
    );
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"sidePanel": {"mateHover": "Nobody", "mate": "Nobody"}}),
    );
    window.frames(&mut shell, &mut s, 2);
    assert!(shell.mate_marks().mates_hot.is_empty());
    let scene = shell.shown_scene_arc().unwrap();
    assert_eq!(shell.face_highlight(Some(&scene)), Highlight::NONE);
    // And on a part nothing is drawn.
    run(&mut s, "file.new", json!({}));
    run(&mut s, "feature.block", json!({"size": [10, 10, 10]}));
    window.frames(&mut shell, &mut s, 3);
    assert_eq!(shell.mate_marks(), Marks::default());
}

/// The viewport's size in pixels and the place of a point of the world
/// in the window, in points (the tests' pixels per point are 1).
fn on_screen(shell: &Shell, p: Vec3) -> Option<Pos2> {
    let [w, h] = shell.viewport_size();
    let projector = craft3d_ui_egui::camera::Projector::new(
        &shell.state.camera.to_render(),
        f64::from(w),
        f64::from(h),
    )?;
    let q = projector.project(p)?;
    let rect = shell.viewport_rect()?;
    Some(rect.min + vec2(q.x as f32, q.y as f32))
}

/// Where a click lands in the window for the pixel `(x, y)` of the
/// viewport.
fn at_pixel(shell: &Shell, x: f64, y: f64) -> Pos2 {
    shell.viewport_rect().unwrap().min + vec2(x as f32, y as f32)
}

/// A marker behind a body leaves the pointer to the face in front: a
/// click there picks the face and chooses no mate, and no marker is
/// pointed at. Found along the markers' lines: a point that a body
/// hides, with nothing of any marker in sight within reach.
#[test]
fn a_click_on_a_face_in_front_of_a_marker_picks_the_face() {
    let (mut s, mut shell, mut window) = opened("behind");
    let tokens = Tokens::default();
    let scene = shell.shown_scene_arc().unwrap();
    let camera = shell.state.camera;
    let [w, h] = shell.viewport_size();
    let projector =
        craft3d_ui_egui::camera::Projector::new(&camera.to_render(), f64::from(w), f64::from(h))
            .unwrap();
    let marks = shell.mate_marks();
    let samples: Vec<Vec3> = marks
        .mates
        .iter()
        .flat_map(|&[a, b]| (0..=8).map(move |k| a + (b - a) * (f64::from(k) / 8.0)))
        .collect();
    let in_front = |x: f64, y: f64| {
        scene
            .pick_cut(&camera.to_render(), w, h, x, y, None)
            .ok()
            .and_then(|(face, _)| face)
            .map(|f| projector.depth(f.point))
    };
    let hidden = |p: Vec3| {
        let Some(q) = projector.project(p) else {
            return false;
        };
        in_front(q.x, q.y).is_some_and(|d| d < q.depth - 0.5)
    };
    let reach = f64::from(tokens.mate_hit) + 2.0;
    let spot = samples
        .iter()
        .filter(|p| hidden(**p))
        .filter_map(|p| projector.project(*p))
        .find(|q| {
            samples.iter().all(|o| {
                projector
                    .project(*o)
                    .is_none_or(|r| (r.x - q.x).phypot(r.y - q.y) > reach || hidden(*o))
            })
        })
        .expect("a stretch of a marker behind a body, alone");
    let at = at_pixel(&shell, spot.x, spot.y);
    window.hover(&mut shell, &mut s, at);
    assert_eq!(shell.state.side_panel.mate_hover, None);
    assert!(shell.mate_tip().is_empty());
    window.click(&mut shell, &mut s, at);
    assert_eq!(shell.state.side_panel.mate, None);
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    assert_eq!(got["selection"]["count"], 1, "{}", got["selection"]);
}

/// Two markers at one place (Base and Drive both on the ground's End1,
/// along the same axis): the pointer on their axis names both, in the
/// mates' order, and points at the first.
#[test]
fn coaxial_markers_are_named_together_in_order() {
    let (mut s, mut shell, mut window) = opened("coaxial");
    let base = frame_of(&shell, "Base");
    let drive = frame_of(&shell, "Drive");
    assert!((v(base.origin) - v(drive.origin)).length() < 1e-9);
    let [w, h] = shell.viewport_size();
    let length =
        mates::world_size(&shell.state.camera, w, h, 1.0, Tokens::default().mate_size).unwrap();
    // Along the axis, three quarters up: both glyphs draw it.
    let up = v(drive.origin) + v(drive.z) * (0.75 * length);
    let at = on_screen(&shell, up).unwrap();
    window.hover(&mut shell, &mut s, at);
    assert_eq!(shell.mate_tip(), ["Base · fixed", "Drive · revolute"]);
    assert_eq!(shell.state.side_panel.mate_hover.as_deref(), Some("Base"));
    // The same again: the choice does not flicker.
    window.hover(&mut shell, &mut s, at + vec2(0.0, 0.0));
    assert_eq!(shell.state.side_panel.mate_hover.as_deref(), Some("Base"));
}

/// While the motion plays the markers go with the parts: at each moment
/// the crank's far end (Elbow) is where the crank's angle puts it, and
/// its marker is drawn there.
#[test]
fn the_markers_follow_the_motion() {
    let (mut s, mut shell, mut window) = opened("motion");
    run(&mut s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "Drive_angle", "expr": "360 deg * t" }),
    );
    let mut seen = Vec::new();
    for t in [0.05, 0.2, 0.35] {
        run(&mut s, "view.motion", json!({ "t": t }));
        window.frames(&mut shell, &mut s, 3);
        let a = 360.0_f64.to_radians() * t;
        let elbow = v(frame_of(&shell, "Elbow").origin);
        assert!(
            (elbow.x - 20.0 * a.pcos()).abs() < 1e-3 && (elbow.y - 20.0 * a.psin()).abs() < 1e-3,
            "t = {t}: Elbow at {elbow:?}"
        );
        let marks = shell.mate_marks();
        assert!(
            marks
                .mates
                .iter()
                .any(|[p, _]| (*p - elbow).length() < 1e-9),
            "t = {t}: no marker at {elbow:?}"
        );
        seen.push(elbow);
    }
    assert!((seen[0] - seen[1]).length() > 1.0 && (seen[1] - seen[2]).length() > 1.0);
}

/// A mate with a flexible subassembly on one side tints the
/// subassembly's instances (it has no bodies of its own).
#[test]
fn a_flexible_subassembly_s_instances_are_tinted() {
    let mut s = window_session("flexible");
    four_bar(&mut s);
    run(
        &mut s,
        "file.save",
        json!({ "path": "demo/asm/linkage.3dcraftasm" }),
    );
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "demo/asm/linkage.3dcraftasm", "name": "Linkage", "mode": "flexible" }),
    );
    run(
        &mut s,
        "instance.add",
        json!({ "source": "demo/asm/bar.3dcraft", "name": "Arm", "body": "Bar" }),
    );
    run(
        &mut s,
        "mate.add",
        json!({ "kind": "revolute", "a": "Linkage/Rocker/End1", "b": "Arm/End1", "name": "Pivot", "flip": true }),
    );
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frames(&mut shell, &mut s, 4);
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"sidePanel": {"mateHover": "Pivot"}}),
    );
    window.frames(&mut shell, &mut s, 2);
    let outline = shell.assembly_outline().unwrap().clone();
    let scene = shell.shown_scene_arc().unwrap();
    let lit = shell.face_highlight(Some(&scene));
    assert_eq!(lit.color, Tokens::default().mate_part);
    let first_face = |instance: &str| {
        let i = outline
            .instances
            .iter()
            .find(|i| i.name == instance)
            .unwrap();
        let feature = i.features.first().unwrap_or_else(|| panic!("{i:?}"));
        scene
            .bodies
            .iter()
            .find(|b| b.feature == *feature)
            .unwrap()
            .first_face_id
    };
    for name in [
        "Linkage/Ground",
        "Linkage/Crank",
        "Linkage/Coupler",
        "Linkage/Rocker",
        "Arm",
    ] {
        assert!(lit.contains(first_face(name)), "{name} is not tinted");
    }
}

/// Under the studio look's lens with something selected the parts of a
/// mate pointed at are not tinted (the selection keeps its own colours,
/// its outline says it); without a selection they are.
#[test]
fn no_tint_under_the_lens_with_a_selection() {
    let (mut s, mut shell, mut window) = opened("lens");
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"look": {"mode": "studio", "ground": false, "shadows": false,
                        "background": "plain", "edges": "off",
                        "dof": {"fstop": 1.4, "focus": "centre"}},
               "sidePanel": {"mateHover": "Elbow"}}),
    );
    shell.state.look.spec().unwrap().prepared().unwrap();
    window.frames(&mut shell, &mut s, 2);
    assert!(shell.lens_on());
    let scene = shell.shown_scene_arc().unwrap();
    assert_eq!(
        shell.face_highlight(Some(&scene)).color,
        Tokens::default().mate_part
    );
    let crank = shell
        .assembly_outline()
        .unwrap()
        .instances
        .iter()
        .find(|i| i.name == "Crank")
        .unwrap()
        .features[0]
        .clone();
    run(
        &mut s,
        "select.set",
        json!({"items": [{"kind": "body", "feature": crank}]}),
    );
    window.frames(&mut shell, &mut s, 2);
    let scene = shell.shown_scene_arc().unwrap();
    assert_eq!(shell.face_highlight(Some(&scene)), Highlight::NONE);
}

/// A mate chosen or pointed at belongs to its document: switching to
/// another tab lets both go.
#[test]
fn a_chosen_mate_goes_with_its_document() {
    let (mut s, mut shell, mut window) = opened("switch");
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"sidePanel": {"mate": "Knee", "mateHover": "Hip"}}),
    );
    window.frames(&mut shell, &mut s, 2);
    assert_eq!(shell.state.side_panel.mate.as_deref(), Some("Knee"));
    run(&mut s, "file.activate", json!({"document": 0}));
    window.frames(&mut shell, &mut s, 3);
    assert_eq!(shell.state.side_panel.mate, None);
    assert_eq!(shell.state.side_panel.mate_hover, None);
}

/// A mate pointed at from outside (`ui.set`) while the pointer had
/// pointed at a row stays when the pointer leaves the rows: only the
/// hover the pointer made goes with it.
#[test]
fn a_hover_set_from_outside_outlives_the_pointer_leaving() {
    let (mut s, mut shell, mut window) = opened("outside");
    let row = shell
        .mate_row_hits()
        .iter()
        .find(|(n, _)| n == "Knee")
        .map(|(_, r)| r.center())
        .unwrap();
    window.hover(&mut shell, &mut s, row);
    assert_eq!(shell.state.side_panel.mate_hover.as_deref(), Some("Knee"));
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"sidePanel": {"mateHover": "Hip"}}),
    );
    window.hover(&mut shell, &mut s, egui::pos2(990.0, 10.0));
    assert_eq!(shell.state.side_panel.mate_hover.as_deref(), Some("Hip"));
}
