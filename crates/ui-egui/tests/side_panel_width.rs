//! The side panel's width is the person's (`state.panels.treeWidth`):
//! long rows (a long file name in a tab, instances and mates with long
//! names, a long assertion message), the Parameters tab and the player
//! playing never change it; only a drag on the split does, held to the
//! tokens' range.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, PointerButton, Pos2, Rect, pos2, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Long words for names that must be identifiers (instances, mates).
const LONG: &str = "OfTheFourBarDemonstrationWithAReallyRatherLongName";

/// A bar part with published parameters, saved under a long name, and a
/// four-bar of it with long instance names, a driven crank and a failing
/// assertion with a long message.
fn documents(s: &mut Session) {
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
        json!({ "name": format!("Bar{LONG}"), "size": ["len", 4, 2] }),
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
    run(
        s,
        "assert.add",
        json!({ "name": format!("Heavy enough {LONG}"), "measure": "volume", "min": 1e12 }),
    );
    run(
        s,
        "file.save",
        json!({ "path": "the-bar-of-a-four-bar-linkage-with-a-long-file-name.3dcraft" }),
    );
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
            json!({ "source": "the-bar-of-a-four-bar-linkage-with-a-long-file-name.3dcraft",
                    "name": format!("{name}{LONG}"), "body": format!("Bar{LONG}"),
                    "params": { "len": len } }),
        );
    }
    let g = format!("Ground{LONG}");
    let c = format!("Crank{LONG}");
    let k = format!("Coupler{LONG}");
    let r = format!("Rocker{LONG}");
    for (name, a, b) in [
        ("Base", "world/z".to_string(), format!("{g}/End1")),
        ("Drive", format!("{g}/End1"), format!("{c}/End1")),
        ("Elbow", format!("{c}/End2"), format!("{k}/End1")),
        ("Knee", format!("{k}/End2"), format!("{r}/End2")),
        ("Hip", format!("{g}/End2"), format!("{r}/End1")),
    ] {
        let kind = if name == "Base" { "fixed" } else { "revolute" };
        run(
            s,
            "mate.add",
            json!({ "kind": kind, "a": a, "b": b, "name": format!("{name}{LONG}"), "flip": true }),
        );
    }
    run(s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        s,
        "variable.set",
        json!({ "name": format!("Drive{LONG}_angle"), "expr": "360 deg * t" }),
    );
}

struct Window {
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 30.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    fn button(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2, pressed: bool) {
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

fn set(shell: &mut Shell, session: &mut Session, patch: Value) {
    shell
        .handle_method(session, "ui.set", Some(&patch))
        .unwrap()
        .unwrap_or_else(|e| panic!("ui.set {patch}: {e}"));
}

fn width(shell: &Shell) -> f32 {
    shell.tree_panel_rect().unwrap().width()
}

#[test]
fn the_side_panel_keeps_its_width_whatever_it_shows() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("side-panel-width");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    documents(&mut session);
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    window.frame(&mut shell, &mut session, Vec::new());
    let tokens = shell.tokens;
    let wanted = width(&shell);
    assert_eq!(wanted, tokens.tree_width);
    assert_eq!(shell.state.panels.tree_width, tokens.tree_width);
    let mut check = |shell: &mut Shell, session: &mut Session, what: &str, frames: usize| {
        for k in 0..frames {
            window.frame(shell, session, Vec::new());
            assert_eq!(width(shell), wanted, "{what}, frame {k}");
        }
    };
    // The assembly: long instance, mate and assertion rows.
    check(&mut shell, &mut session, "the assembly", 20);
    // Playing its motion.
    set(
        &mut shell,
        &mut session,
        json!({ "timeline": { "motion": true, "playing": true, "loop": true } }),
    );
    check(&mut shell, &mut session, "playing", 40);
    set(
        &mut shell,
        &mut session,
        json!({ "timeline": { "playing": false } }),
    );
    // The Parameters tab (the assembly's joints) and back.
    set(
        &mut shell,
        &mut session,
        json!({"parameters": {"open": true}}),
    );
    check(&mut shell, &mut session, "the Parameters tab", 10);
    set(
        &mut shell,
        &mut session,
        json!({"parameters": {"open": false}}),
    );
    check(&mut shell, &mut session, "the tree again", 10);
    // The part, in its own tab with its long file name: its features,
    // its failing assertion and its Parameters tab.
    run(&mut session, "file.activate", json!({"document": 0}));
    check(&mut shell, &mut session, "the part", 10);
    set(
        &mut shell,
        &mut session,
        json!({"parameters": {"open": true}}),
    );
    check(&mut shell, &mut session, "the part's parameters", 10);
    set(
        &mut shell,
        &mut session,
        json!({"parameters": {"open": false}}),
    );
    check(&mut shell, &mut session, "the part's features", 10);

    // A drag on the split sets the width, held to the tokens' range.
    let panel = shell.tree_panel_rect().unwrap();
    let split = pos2(panel.right(), panel.center().y);
    for (to, expected) in [
        (panel.left() + 330.0, 330.0),
        (panel.left() + 20.0, tokens.tree_min_width),
        (
            panel.left() + 900.0,
            tokens.tree_max_width.min(1280.0 * tokens.tree_max_share),
        ),
    ] {
        let from = pos2(shell.tree_panel_rect().unwrap().right(), split.y);
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(from)]);
        window.button(&mut shell, &mut session, from, true);
        for step in 1..=6 {
            let at = from + (pos2(to, split.y) - from) * (step as f32 / 6.0);
            window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        }
        window.button(&mut shell, &mut session, pos2(to, split.y), false);
        window.frame(&mut shell, &mut session, vec![Event::PointerGone]);
        window.frame(&mut shell, &mut session, Vec::new());
        assert!(
            (shell.state.panels.tree_width - expected).abs() < 1.0,
            "dragged to {to}: {} against {expected}",
            shell.state.panels.tree_width
        );
        assert!((width(&shell) - expected).abs() < 1.0, "{}", width(&shell));
    }
    // `ui.set` sets it too.
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"treeWidth": 300.0}}),
    );
    window.frame(&mut shell, &mut session, Vec::new());
    window.frame(&mut shell, &mut session, Vec::new());
    assert!((width(&shell) - 300.0).abs() < 1.0, "{}", width(&shell));
}
