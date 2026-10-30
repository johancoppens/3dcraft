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
