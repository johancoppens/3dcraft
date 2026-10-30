//! The popovers of the Studio and Analysis chips (`shell/popover.rs`):
//! at a large, a middling and a small window, each is inside the
//! viewport and clear of the view cube; opened and closed twice from its
//! chip it comes back at the very same rect; folded and unfolded (by its
//! header, and by Escape) it keeps its size. The view cube is flush with
//! the viewport's right margin. Driven by clicks in real frames.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, PointerButton, Pos2, Rect, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn part() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    s
}

struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
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
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![Event::PointerMoved(at)],
            vec![press(true)],
            vec![press(false)],
            vec![Event::PointerGone],
        ] {
            self.frame(shell, session, events);
        }
        self.frames(shell, session, 3);
    }

    fn escape(&mut self, shell: &mut Shell, session: &mut Session) {
        let key = |pressed| Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(shell, session, vec![key(true)]);
        self.frame(shell, session, vec![key(false)]);
        self.frames(shell, session, 3);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Which {
    Studio,
    Analysis,
}

fn hits(shell: &Shell, which: Which) -> &[(String, Rect)] {
    match which {
        Which::Studio => shell.look_hits(),
        Which::Analysis => shell.analysis_hits(),
    }
}

fn hit(shell: &Shell, which: Which, name: &str) -> Option<Rect> {
    hits(shell, which)
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| *r)
}

fn open(shell: &Shell, which: Which) -> bool {
    match which {
        Which::Studio => shell.look_panel_open(),
        Which::Analysis => shell.analysis_panel_open(),
    }
}

fn chip(shell: &Shell, which: Which) -> Pos2 {
    let layout = shell.controls_layout().unwrap();
    match which {
        Which::Studio => layout.studio.unwrap().center(),
        Which::Analysis => layout.analysis.unwrap().center(),
    }
}

/// The panel is inside the viewport and clear of the cube and the
/// plates.
fn placed_well(shell: &Shell, which: Which, size: egui::Vec2) -> Rect {
    let panel = hit(shell, which, "panel")
        .unwrap_or_else(|| panic!("{which:?} at {size:?}: no panel: {:?}", hits(shell, which)));
    let viewport = shell.viewport_rect().unwrap();
    assert!(
        viewport.contains_rect(panel),
        "{which:?} at {size:?}: {panel:?} in {viewport:?}"
    );
    let layout = shell.controls_layout().unwrap();
    for (name, part) in layout.parts() {
        assert!(
            !part.intersects(panel),
            "{which:?} at {size:?}: over the {name}: {panel:?} {part:?}"
        );
    }
    panel
}

#[test]
fn the_popovers_sit_clear_of_the_cube_and_keep_their_size() {
    for size in [
        vec2(1600.0, 1000.0),
        vec2(1024.0, 700.0),
        vec2(640.0, 420.0),
    ] {
        let mut session = part();
        let mut shell = Shell::with_renderer(RendererChoice::Cpu);
        let mut window = Window {
            ctx: egui::Context::default(),
            size,
            time: 0.0,
        };
        window.frames(&mut shell, &mut session, 4);
        // The cube flush with the viewport's right margin, under the
        // camera plate.
        let viewport = shell.viewport_rect().unwrap();
        let tokens = shell.tokens;
        if let Some(cube) = shell.view_cube_rect() {
            assert!(
                (viewport.right() - tokens.nav_margin - cube.right()).abs() < 0.5,
                "{size:?}: {cube:?} in {viewport:?}"
            );
            let plate = shell.controls_layout().unwrap().camera_plate.unwrap();
            assert!(cube.top() > plate.bottom(), "{size:?}");
        }
        for which in [Which::Studio, Which::Analysis] {
            // Off: no popover.
            assert!(hits(&shell, which).is_empty(), "{which:?} at {size:?}");
            // The chip opens it, unfolded.
            let at = chip(&shell, which);
            window.click(&mut shell, &mut session, at);
            assert!(open(&shell, which), "{which:?} at {size:?}");
            let first = placed_well(&shell, which, size);
            // Closed and opened from the chip, twice: the same rect.
            for cycle in 0..2 {
                let at = chip(&shell, which);
                window.click(&mut shell, &mut session, at);
                assert!(
                    hits(&shell, which).is_empty(),
                    "{which:?} at {size:?}, cycle {cycle}: still shown"
                );
                let at = chip(&shell, which);
                window.click(&mut shell, &mut session, at);
                let again = placed_well(&shell, which, size);
                assert_eq!(again, first, "{which:?} at {size:?}, cycle {cycle}");
            }
            // Folded by its header and unfolded: the same rect each time.
            for cycle in 0..2 {
                let header = hit(&shell, which, "collapse").unwrap();
                window.click(&mut shell, &mut session, header.center());
                assert!(!open(&shell, which), "{which:?} at {size:?}");
                let folded = placed_well(&shell, which, size);
                assert_eq!(folded.min, first.min, "{which:?} at {size:?}");
                assert_eq!(folded.width(), first.width(), "{which:?} at {size:?}");
                assert!(
                    folded.height() < first.height(),
                    "{which:?} at {size:?}: {folded:?} against {first:?}"
                );
                let header = hit(&shell, which, "collapse").unwrap();
                window.click(&mut shell, &mut session, header.center());
                assert!(open(&shell, which), "{which:?} at {size:?}");
                let unfolded = placed_well(&shell, which, size);
                assert_eq!(unfolded, first, "{which:?} at {size:?}, cycle {cycle}");
            }
            // Escape folds it; the header unfolds it to the same rect.
            window.escape(&mut shell, &mut session);
            assert!(!open(&shell, which), "{which:?} at {size:?}: Escape");
            let header = hit(&shell, which, "collapse").unwrap();
            window.click(&mut shell, &mut session, header.center());
            assert_eq!(placed_well(&shell, which, size), first);
            // Off again from the chip.
            let at = chip(&shell, which);
            window.click(&mut shell, &mut session, at);
            assert!(hits(&shell, which).is_empty(), "{which:?} at {size:?}");
        }
    }
}

/// Escape with no popover open still clears the selection over the
/// viewport; with one open it folds that first and keeps the selection.
#[test]
fn escape_folds_an_open_popover_before_it_clears_the_selection() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window {
        ctx: egui::Context::default(),
        size: vec2(1024.0, 700.0),
        time: 0.0,
    };
    window.frames(&mut shell, &mut session, 4);
    let set = |shell: &mut Shell, session: &mut Session, patch: Value| {
        shell
            .handle_method(session, "ui.set", Some(&patch))
            .unwrap()
            .unwrap();
    };
    set(
        &mut shell,
        &mut session,
        json!({"look": {"mode": "studio"}, "selectedFeature": "F1"}),
    );
    window.frames(&mut shell, &mut session, 4);
    let middle = shell.viewport_rect().unwrap().center();
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(middle)]);
    assert!(shell.look_panel_open());
    window.escape(&mut shell, &mut session);
    assert!(!shell.look_panel_open());
    assert_eq!(shell.state.selected_feature.as_deref(), Some("F1"));
    window.frame(&mut shell, &mut session, vec![Event::PointerMoved(middle)]);
    window.escape(&mut shell, &mut session);
    assert_eq!(shell.state.selected_feature, None);
}
