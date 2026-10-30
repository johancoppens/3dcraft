//! An assembly in the window (docs/design/assemblies.md): the picture the
//! shell draws follows `joint.set`, and `ui.fit` frames the placed parts.
//! Driven through real frames of the shell, as the window draws them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::Shell;
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

    fn frame(&mut self, shell: &mut Shell, session: &mut Session) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            time: Some(self.time),
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    fn events(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<egui::Event>) {
        self.events_with(shell, session, events, egui::Modifiers::NONE);
    }

    fn events_with(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        self.time += 1.0 / 60.0;
        let mut events = events;
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
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
        for _ in 0..3 {
            self.frame(shell, session);
        }
    }

    /// A drag with the primary button from `from` to `to` in steps,
    /// holding `modifiers`.
    fn drag(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        from: Pos2,
        to: Pos2,
        modifiers: egui::Modifiers,
    ) {
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        self.events_with(
            shell,
            session,
            vec![egui::Event::PointerMoved(from)],
            modifiers,
        );
        self.events_with(shell, session, vec![button(from, true)], modifiers);
        for k in 1..=8 {
            let f = k as f32 / 8.0;
            let at = from + (to - from) * f;
            self.events_with(
                shell,
                session,
                vec![egui::Event::PointerMoved(at)],
                modifiers,
            );
        }
        self.events_with(shell, session, vec![button(to, false)], modifiers);
        for _ in 0..3 {
            self.frame(shell, session);
        }
    }
}

/// A session as the window has one: reading and writing under `dir`
/// (`--automation-read-root`), the display scene refined elsewhere.
pub fn window_session(name: &str) -> (Session, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ui-assembly-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("demo/asm")).unwrap();
    let session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    (session, dir)
}

/// The window's scenario: a bar part saved and left open, then a new
/// assembly of four instances of it (two lengths) closed into a four-bar
/// by a fixed mate and four revolutes, its crank `Drive`.
pub fn four_bar(s: &mut Session) {
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

/// The crank's box in the scene the shell shows now.
fn crank_box(shell: &Shell) -> ([f64; 3], [f64; 3]) {
    let scene = shell.shown_scene_arc().expect("the shell shows a scene");
    let crank = scene
        .bodies
        .iter()
        .find(|b| b.name == "Crank")
        .unwrap_or_else(|| {
            panic!(
                "no Crank among {:?}",
                scene.bodies.iter().map(|b| &b.name).collect::<Vec<_>>()
            )
        });
    let (lo, hi) = crank.mesh.bbox().corners().expect("a box");
    ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
}

#[test]
fn the_window_follows_joint_set_on_an_assembly_and_fits_it() {
    for refine_elsewhere in [false, true] {
        let (mut s, _dir) = window_session(&format!("follow-{refine_elsewhere}"));
        s.set_refine_elsewhere(refine_elsewhere);
        let mut shell = Shell::new();
        let mut window = Window::new();
        window.frame(&mut shell, &mut s);
        four_bar(&mut s);
        window.frame(&mut shell, &mut s);
        let mut boxes = Vec::new();
        for angle in [60.0_f64, 150.0, 240.0] {
            run(
                &mut s,
                "joint.set",
                json!({ "mate": "Drive", "value": format!("{angle} deg") }),
            );
            for _ in 0..3 {
                window.frame(&mut shell, &mut s);
            }
            let (lo, hi) = crank_box(&shell);
            // The crank turns about End1 at (2, 2) of the ground: its far
            // end is near 20 mm out at the angle.
            let (c, n) = (angle.to_radians().cos(), angle.to_radians().sin());
            let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
            assert!(
                (mid[0] - 10.0 * c).abs() < 3.0 && (mid[1] - 10.0 * n).abs() < 3.0,
                "refine elsewhere {refine_elsewhere}, {angle} deg: crank at {mid:?}"
            );
            boxes.push((lo, hi));
        }
        assert_ne!(boxes[0], boxes[1]);
        assert_ne!(boxes[1], boxes[2]);
        let fitted = call(&mut shell, &mut s, "ui.fit", json!({}));
        assert!(fitted.is_object(), "{fitted}");
    }
}

/// The left panel of an assembly lists its instances (marks, a click
/// selects, an eye hides, Open opens the source in a tab), then its mates
/// with status and value and the degrees of freedom left.
#[test]
fn the_tree_of_an_assembly_lists_instances_then_mates() {
    let (mut s, _dir) = window_session("tree");
    let mut shell = Shell::new();
    let mut window = Window::new();
    four_bar(&mut s);
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Drive", "value": "60 deg" }),
    );
    for _ in 0..3 {
        window.frame(&mut shell, &mut s);
    }
    let rows = shell.tree_rows().to_vec();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Ground", "Crank", "Coupler", "Rocker"]);
    assert!(
        rows.iter()
            .all(|r| r.kind == "instance" && r.depth == 0 && !r.id.is_empty())
    );
    let outline = shell.assembly_outline().expect("an outline").clone();
    let mates: Vec<&str> = outline.mates.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(mates, ["Base", "Drive", "Elbow", "Knee", "Hip"]);
    assert!(
        outline.mates.iter().all(|m| m.status == "ok"),
        "{outline:?}"
    );
    assert_eq!(outline.dof, Some(0), "{outline:?}");
    let drive = &outline.mates[1];
    assert!(drive.driven);
    let (value, angle) = drive.value.unwrap();
    assert!(
        angle && (value.to_degrees() - 60.0).abs() < 1e-6,
        "{drive:?}"
    );

    // A click on Crank's row selects it.
    let crank = rows[1].id.clone();
    let rect = shell
        .tree_row_hits()
        .iter()
        .find(|(id, _)| *id == crank)
        .map(|(_, r)| *r)
        .expect("Crank's row drawn");
    window.click(&mut shell, &mut s, rect.center());
    assert!(shell.tree_rows()[1].selected, "{:?}", shell.tree_rows()[1]);

    // Its eye hides the instance.
    let hit = |shell: &Shell, what: &str| {
        shell
            .instance_hits()
            .iter()
            .find(|(n, k, _)| n == "Crank" && *k == what)
            .map(|(_, _, r)| r.center())
            .unwrap_or_else(|| panic!("no {what} drawn"))
    };
    let eye = hit(&shell, "eye");
    window.click(&mut shell, &mut s, eye);
    let inspected = run(&mut s, "assembly.inspect", json!({}));
    let instance = inspected["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "Crank")
        .unwrap()
        .clone();
    assert_eq!(instance["visible"], false, "{instance}");
    assert!(shell.tree_rows()[1].hidden);

    // Open opens the bar part in a tab of its own.
    let before = s.document_statuses().len();
    let open = hit(&shell, "open");
    window.click(&mut shell, &mut s, open);
    assert_eq!(s.document_statuses().len(), before + 1);
}

/// The value of mate `name` as `assembly.inspect` gives it (radians).
fn mate_value(s: &mut Session, name: &str) -> f64 {
    let inspected = run(s, "assembly.inspect", json!({}));
    inspected["mates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .and_then(|m| m["value"].as_f64())
        .unwrap_or_else(|| panic!("no value of {name}: {inspected}"))
}

/// A joint that can be set by hand is a slider on the Parameters tab:
/// dragging it sets the joint (`joint.set`).
#[test]
fn a_free_joint_is_a_slider_on_the_parameters_tab() {
    let (mut s, _dir) = window_session("slider");
    let mut shell = Shell::new();
    let mut window = Window::new();
    four_bar(&mut s);
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Drive", "value": "60 deg" }),
    );
    // The assembly opens on its tree; the tab is opened once it is shown.
    window.frame(&mut shell, &mut s);
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({ "parameters": { "open": true } }),
    );
    for _ in 0..3 {
        window.frame(&mut shell, &mut s);
    }
    let hits = shell.joint_slider_hits().to_vec();
    let names: Vec<&str> = hits.iter().map(|(n, _)| n.as_str()).collect();
    // Every revolute's angle can be set; the fixed Base has none.
    assert_eq!(names, ["Drive", "Elbow", "Knee", "Hip"]);
    let rect = hits[0].1;
    let before = mate_value(&mut s, "Drive");
    assert!((before.to_degrees() - 60.0).abs() < 1e-6);
    // From the slider's middle part to its right end.
    let from = egui::pos2(rect.right() - rect.width() * 0.45, rect.center().y);
    let to = egui::pos2(rect.right() - 4.0, rect.center().y);
    window.drag(&mut shell, &mut s, from, to, egui::Modifiers::NONE);
    let after = mate_value(&mut s, "Drive");
    assert!(
        (after - before).abs() > 1e-3,
        "the slider did not set Drive: {} deg",
        after.to_degrees()
    );
    let inspected = run(&mut s, "assembly.inspect", json!({}));
    assert_eq!(inspected["solve"]["ok"], true, "{inspected}");
}

/// Alt and a drag on a part of an assembly turn it about its joint toward
/// the pointer (the crank turns, the linkage follows), as `joint.set`.
#[test]
fn alt_drag_turns_a_part_about_its_joint() {
    use craft3d_math::Vec3;
    use craft3d_ui_egui::camera::Projector;
    let (mut s, _dir) = window_session("drag");
    let mut shell = Shell::new();
    let mut window = Window::new();
    four_bar(&mut s);
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Drive", "value": "60 deg" }),
    );
    // Seen from above, so the crank's plane is the screen's.
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({ "camera": { "yaw": -90.0, "pitch": 89.0 } }),
    );
    for _ in 0..4 {
        window.frame(&mut shell, &mut s);
    }
    let rect = shell.viewport_rect().expect("a viewport");
    let [w, h] = shell.viewport_size();
    let projector =
        Projector::new(&shell.state.camera.to_render(), f64::from(w), f64::from(h)).unwrap();
    let screen = |angle: f64| {
        let a = angle.to_radians();
        let p = projector
            .project(Vec3::new(14.0 * a.cos(), 14.0 * a.sin(), 2.0))
            .unwrap();
        egui::pos2(
            rect.left() + (p.x as f32) * rect.width() / w as f32,
            rect.top() + (p.y as f32) * rect.height() / h as f32,
        )
    };
    let camera = shell.state.camera;
    window.drag(
        &mut shell,
        &mut s,
        screen(60.0),
        screen(100.0),
        egui::Modifiers::ALT,
    );
    let after = mate_value(&mut s, "Drive").to_degrees();
    assert!(
        (after - 100.0).abs() < 3.0,
        "the crank was dragged to 100 deg, it is at {after} deg"
    );
    // The camera did not turn meanwhile.
    assert_eq!(shell.state.camera.yaw, camera.yaw);
    assert_eq!(shell.state.camera.pitch, camera.pitch);
}

/// The four-bar moving over `t` (its crank driven by `360 deg * t`) in
/// three pictures of the viewport, printed as EVIDENCE for whoever looks.
#[test]
fn the_four_bar_moves_over_t_in_the_viewport() {
    let (mut s, _dir) = window_session("evidence");
    let mut shell = Shell::new();
    let mut window = Window::new();
    four_bar(&mut s);
    run(&mut s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "Drive_angle", "expr": "360 deg * t" }),
    );
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({ "camera": { "yaw": -90.0, "pitch": 89.0 } }),
    );
    let mut pictures = Vec::new();
    for t in [0.05, 0.2, 0.35] {
        run(&mut s, "view.motion", json!({ "t": t }));
        for _ in 0..3 {
            window.frame(&mut shell, &mut s);
        }
        let (lo, hi) = crank_box(&shell);
        let a = 360.0_f64.to_radians() * t;
        let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        assert!(
            (mid[0] - 10.0 * a.cos()).abs() < 1.0 && (mid[1] - 10.0 * a.sin()).abs() < 1.0,
            "t = {t}: crank at {mid:?}"
        );
        let shot = call(
            &mut shell,
            &mut s,
            "ui.screenshot",
            json!({ "width": 480, "height": 320, "scale": 1.0 }),
        );
        let text = shot["base64"].as_str().unwrap().to_string();
        println!("EVIDENCE four-bar-t{t} {text}");
        pictures.push(text);
    }
    assert_ne!(pictures[0], pictures[1]);
    assert_ne!(pictures[1], pictures[2]);
}

/// Two faces of two instances selected: a right click offers the mates
/// that join them first ("Revolute between these", …), and one issues
/// `mate.add` with the two faces named through their instances.
#[test]
fn two_selected_faces_offer_the_mates_that_join_them() {
    use craft3d_math::Vec3;
    use craft3d_ui_egui::camera::Projector;
    let (mut s, _dir) = window_session("mate-menu");
    let mut shell = Shell::new();
    let mut window = Window::new();
    four_bar(&mut s);
    run(
        &mut s,
        "joint.set",
        json!({ "mate": "Drive", "value": "60 deg" }),
    );
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({ "camera": { "yaw": -90.0, "pitch": 89.0 } }),
    );
    for _ in 0..4 {
        window.frame(&mut shell, &mut s);
    }
    let rect = shell.viewport_rect().expect("a viewport");
    let [w, h] = shell.viewport_size();
    let projector =
        Projector::new(&shell.state.camera.to_render(), f64::from(w), f64::from(h)).unwrap();
    let a = 60.0_f64.to_radians();
    let crank = projector
        .project(Vec3::new(10.0 * a.cos(), 10.0 * a.sin(), 2.0))
        .unwrap();
    let ground = projector.project(Vec3::new(30.0, 0.0, 2.0)).unwrap();
    call(
        &mut shell,
        &mut s,
        "ui.pick",
        json!({ "x": crank.x, "y": crank.y, "select": true }),
    );
    call(
        &mut shell,
        &mut s,
        "ui.pick",
        json!({ "x": ground.x, "y": ground.y, "select": true, "shift": true }),
    );
    for _ in 0..2 {
        window.frame(&mut shell, &mut s);
    }
    assert_eq!(
        shell.selection().items.len(),
        2,
        "{:?}",
        shell.selection().items
    );
    let at = egui::pos2(
        rect.left() + (crank.x as f32) * rect.width() / w as f32,
        rect.top() + (crank.y as f32) * rect.height() / h as f32,
    );
    window.events(&mut shell, &mut s, vec![egui::Event::PointerMoved(at)]);
    for pressed in [true, false] {
        window.events(
            &mut shell,
            &mut s,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    window.frame(&mut shell, &mut s);
    let kinds: Vec<&str> = shell.mate_entry_hits().iter().map(|(k, _)| *k).collect();
    assert_eq!(
        kinds,
        [
            "revolute",
            "fixed",
            "slider",
            "cylindrical",
            "planar",
            "ball"
        ]
    );
    let revolute = shell.mate_entry_hits()[0].1.center();
    window.click(&mut shell, &mut s, revolute);
    let inspected = run(&mut s, "assembly.inspect", json!({}));
    let mates = inspected["mates"].as_array().unwrap();
    assert_eq!(mates.len(), 6, "{inspected}");
    let added = &mates[5];
    assert_eq!(added["kind"], "revolute", "{added}");
    assert!(
        added["a"].as_str().unwrap().starts_with("Crank/"),
        "{added}"
    );
    assert!(
        added["b"].as_str().unwrap().starts_with("Ground/"),
        "{added}"
    );
}
