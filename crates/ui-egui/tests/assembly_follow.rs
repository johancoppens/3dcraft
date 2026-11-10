//! An assembly in the window follows its sources (docs/design/assemblies.md
//! › *Following the sources*): the source opened from the Instances panel,
//! changed and saved in its own tab, shows in the assembly's tab; a source
//! changed by another program is followed when the window gets the focus
//! back; a pinned instance's row is marked, with the reason in its tooltip.
//! Driven through real frames of the shell.

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

struct Window {
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, focused: bool) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            time: Some(self.time),
            focused,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        let moved = egui::Event::PointerMoved(at);
        self.events(shell, session, vec![moved.clone()]);
        for pressed in [true, false] {
            self.events(
                shell,
                session,
                vec![
                    moved.clone(),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    fn events(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<egui::Event>) {
        {
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
    }

    fn frames(&mut self, shell: &mut Shell, session: &mut Session) {
        for _ in 0..3 {
            self.frame(shell, session, true);
        }
    }
}

fn knob(s: &mut Session, w: &str) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({ "name": "w", "expr": w }));
    run(
        s,
        "feature.block",
        json!({ "name": "Base", "size": ["w", 10, 10] }),
    );
    run(s, "file.save", json!({ "path": "knob.3dcraft" }));
    run(s, "file.close", json!({}));
}

/// The x extent of instance `K`'s body in the scene the shell shows.
fn width(shell: &Shell) -> f64 {
    let scene = shell.shown_scene_arc().expect("the shell shows a scene");
    let body = scene
        .bodies
        .iter()
        .find(|b| b.name == "K")
        .unwrap_or_else(|| {
            panic!(
                "no K among {:?}",
                scene.bodies.iter().map(|b| &b.name).collect::<Vec<_>>()
            )
        });
    let (lo, hi) = body.mesh.bbox().corners().expect("a box");
    hi.x - lo.x
}

fn setup(name: &str) -> (Session, PathBuf, Shell, Window) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ui-asm-follow-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    knob(&mut s, "40 mm");
    run(&mut s, "assembly.new", json!({}));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "knob.3dcraft", "name": "K" }),
    );
    run(&mut s, "file.save", json!({ "path": "pi.3dcraftasm" }));
    let mut shell = Shell::new();
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    window.frames(&mut shell, &mut s);
    (s, dir, shell, window)
}

/// The developer's flow (2026-11-06): Open on the instance's row, the
/// variable changed and saved in the source's tab, back to the
/// assembly's tab: it shows the new part.
#[test]
fn a_source_saved_in_its_tab_shows_in_the_assembly_tab() {
    let (mut s, _dir, mut shell, mut window) = setup("saved");
    assert!((width(&shell) - 40.0).abs() < 1e-6);
    // Hovered, the row shows its eye and Open beside it.
    let row = shell.tree_row_hits()[0].1.center();
    window.events(&mut shell, &mut s, vec![egui::Event::PointerMoved(row)]);
    window.frames(&mut shell, &mut s);
    let open = shell
        .instance_hits()
        .iter()
        .find(|(n, k, _)| n == "K" && *k == "open")
        .map(|(_, _, r)| r.center())
        .expect("K's Open drawn");
    window.click(&mut shell, &mut s, open);
    window.frames(&mut shell, &mut s);
    assert_eq!(s.document_statuses().len(), 2, "Open made a tab");
    run(
        &mut s,
        "variable.set",
        json!({ "name": "w", "expr": "50 mm" }),
    );
    let saved = run(&mut s, "file.save", json!({}));
    assert_eq!(saved["followed"][0]["document"], 0, "{saved}");
    window.frames(&mut shell, &mut s);
    // The tab of the assembly, clicked: file.activate.
    run(&mut s, "file.activate", json!({ "document": 0 }));
    window.frames(&mut shell, &mut s);
    assert!((width(&shell) - 50.0).abs() < 1e-6, "{}", width(&shell));
    assert!(shell.tree_rows()[0].held.is_none());
}

/// Another program writes the source while the window is in the
/// background: when it gets the focus back the assembly follows, and the
/// status bar says so.
#[test]
fn a_source_changed_outside_is_followed_when_the_window_gets_the_focus_back() {
    let (mut s, dir, mut shell, mut window) = setup("focus");
    window.frame(&mut shell, &mut s, false);
    let mut other = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    knob(&mut other, "60 mm");
    window.frame(&mut shell, &mut s, false);
    assert!((width(&shell) - 40.0).abs() < 1e-6);
    window.frames(&mut shell, &mut s);
    assert!((width(&shell) - 60.0).abs() < 1e-6, "{}", width(&shell));
    let said = shell.last_command.as_ref().expect("the status bar says it");
    assert_eq!(said.label, "assembly.follow");
    assert!(said.ok && said.text.contains("knob.3dcraft"), "{said:?}");
}

/// A pinned instance whose source changed: its row is marked, with why.
#[test]
fn a_pinned_instance_whose_source_changed_is_marked_in_the_instances_panel() {
    let (mut s, _dir, mut shell, mut window) = setup("pinned");
    run(
        &mut s,
        "instance.edit",
        json!({ "name": "K", "pinned": true }),
    );
    knob(&mut s, "50 mm");
    run(&mut s, "file.activate", json!({ "document": 0 }));
    window.frames(&mut shell, &mut s);
    assert!((width(&shell) - 40.0).abs() < 1e-6);
    let row = &shell.tree_rows()[0];
    assert!(
        row.status.contains(craft3d_ui_egui::shell::HELD_MARK),
        "{row:?}"
    );
    assert!(row.held.as_deref().unwrap().contains("pinned"), "{row:?}");
}

/// A-023: an instance whose part has a feature that fails with the
/// instance's values is marked in the Instances panel, with the feature
/// and the part's reason in its tooltip, and still shows its body.
#[test]
fn an_instance_whose_part_has_a_failed_feature_is_marked_in_the_instances_panel() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ui-asm-failed-feature");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "variable.set",
        json!({ "name": "corner", "expr": "1 mm" }),
    );
    run(
        &mut s,
        "variable.publish",
        json!({ "name": "corner", "min": "0.5 mm", "max": "30 mm" }),
    );
    run(
        &mut s,
        "feature.block",
        json!({ "name": "Blank", "size": ["30 mm", "20 mm", "4 mm"] }),
    );
    run(
        &mut s,
        "feature.fillet",
        json!({ "name": "Round", "edges": [["F1:face(0)", "F1:face(2)"]], "radius": "corner" }),
    );
    run(&mut s, "file.save", json!({ "path": "plate.3dcraft" }));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "assembly.new", json!({ "units": "mm" }));
    run(
        &mut s,
        "instance.add",
        json!({ "source": "plate.3dcraft", "name": "K", "params": { "corner": "12 mm" } }),
    );
    let mut shell = Shell::new();
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    window.frames(&mut shell, &mut s);
    // The blank, 30 long, from before the fillet.
    assert!((width(&shell) - 30.0).abs() < 1e-6, "{}", width(&shell));
    let row = &shell.tree_rows()[0];
    assert_eq!(row.state, craft3d_engine::FeatureState::Ok, "{row:?}");
    assert!(
        row.status.contains(craft3d_ui_egui::shell::HELD_MARK),
        "{row:?}"
    );
    let failed = row.failed.as_deref().unwrap_or_else(|| panic!("{row:?}"));
    assert!(
        failed.contains("`Round` (F2)") && failed.contains("radius"),
        "{failed}"
    );
    assert!(row.held.is_none(), "{row:?}");
}
