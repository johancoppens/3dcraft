//! The popovers of the Studio and Analysis split buttons
//! (`shell/popover.rs`): at a large, a middling, a small and a phone's
//! window, each hangs from the camera plate, right edges aligned, inside
//! the viewport and clear of every control, the view cube moved down
//! under it (or out) while it is open and back where it was when it
//! closes; switched off and on twice from its chip, and closed and opened
//! twice from its caret (and by Escape), it comes back at the very same
//! rect, the look or the analysis staying on while it is closed. A drag
//! on a slider never closes it. Driven by clicks in real frames.

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
    let rect = match which {
        Which::Studio => layout.studio.unwrap(),
        Which::Analysis => layout.analysis.unwrap(),
    };
    craft3d_ui_egui::shell::split_chip(rect, &shell.tokens)
        .0
        .center()
}

fn caret(shell: &Shell, which: Which) -> Pos2 {
    match which {
        Which::Studio => shell.look_caret(),
        Which::Analysis => shell.analysis_caret(),
    }
    .unwrap()
    .center()
}

fn on(shell: &Shell, which: Which) -> bool {
    match which {
        Which::Studio => shell.state.look.is_studio(),
        Which::Analysis => shell.state.analysis.is_on(),
    }
}

/// The panel is inside the viewport, hangs from the camera plate with
/// its right edge on the plate's, and is clear of the cube (where it
/// went) and the plates.
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
    let plate = layout.camera_plate.unwrap();
    assert!(
        (panel.right() - plate.right()).abs() < 0.5,
        "{which:?} at {size:?}: {panel:?} under {plate:?}"
    );
    assert!(
        panel.top() > plate.bottom() && panel.top() - plate.bottom() <= shell.tokens.nav_gap + 0.5,
        "{which:?} at {size:?}: {panel:?} under {plate:?}"
    );
    assert_eq!(layout.cube, shell.view_cube_rect(), "{which:?} at {size:?}");
    panel
}

#[test]
fn the_popovers_hang_from_the_plate_and_keep_their_size() {
    for size in [
        vec2(1600.0, 1000.0),
        vec2(1024.0, 700.0),
        vec2(640.0, 420.0),
        vec2(390.0, 844.0),
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
        let home = shell.view_cube_rect();
        if let Some(cube) = home {
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
            // The chip switches it on, its popover open.
            let at = chip(&shell, which);
            window.click(&mut shell, &mut session, at);
            assert!(
                on(&shell, which) && open(&shell, which),
                "{which:?} at {size:?}"
            );
            let first = placed_well(&shell, which, size);
            // At the desktop's sizes the cube has a place under it.
            if size.x >= 1000.0 {
                let cube = shell.view_cube_rect().unwrap();
                assert!(cube.top() >= first.bottom(), "{which:?} at {size:?}");
                assert_eq!(cube.right(), home.unwrap().right());
            }
            // Off and on from the chip, twice: the same rect.
            for cycle in 0..2 {
                let at = chip(&shell, which);
                window.click(&mut shell, &mut session, at);
                assert!(!on(&shell, which), "{which:?} at {size:?}, cycle {cycle}");
                assert!(
                    hits(&shell, which).is_empty(),
                    "{which:?} at {size:?}, cycle {cycle}: still shown"
                );
                assert_eq!(shell.view_cube_rect(), home, "{which:?} at {size:?}");
                let at = chip(&shell, which);
                window.click(&mut shell, &mut session, at);
                let again = placed_well(&shell, which, size);
                assert_eq!(again, first, "{which:?} at {size:?}, cycle {cycle}");
            }
            // Closed from the caret and opened again: gone while closed,
            // the look or the analysis still on, the cube back; the same
            // rect each time.
            for cycle in 0..2 {
                let at = caret(&shell, which);
                window.click(&mut shell, &mut session, at);
                assert!(!open(&shell, which), "{which:?} at {size:?}");
                assert!(
                    on(&shell, which),
                    "{which:?} at {size:?}: closing switched it off"
                );
                assert!(hits(&shell, which).is_empty(), "{which:?} at {size:?}");
                assert_eq!(shell.view_cube_rect(), home, "{which:?} at {size:?}");
                let at = caret(&shell, which);
                window.click(&mut shell, &mut session, at);
                assert!(open(&shell, which), "{which:?} at {size:?}");
                let reopened = placed_well(&shell, which, size);
                assert_eq!(reopened, first, "{which:?} at {size:?}, cycle {cycle}");
            }
            // Escape closes it, the look stays; the caret opens it to the
            // same rect.
            window.escape(&mut shell, &mut session);
            assert!(!open(&shell, which), "{which:?} at {size:?}: Escape");
            assert!(on(&shell, which), "{which:?} at {size:?}: Escape");
            assert!(hits(&shell, which).is_empty(), "{which:?} at {size:?}");
            let at = caret(&shell, which);
            window.click(&mut shell, &mut session, at);
            assert_eq!(placed_well(&shell, which, size), first);
            // Off again from the chip; the cube is back.
            let at = chip(&shell, which);
            window.click(&mut shell, &mut session, at);
            assert!(hits(&shell, which).is_empty(), "{which:?} at {size:?}");
            assert_eq!(shell.view_cube_rect(), home, "{which:?} at {size:?}");
        }
        // The caret of a chip that is off switches it on, open.
        let at = caret(&shell, Which::Studio);
        window.click(&mut shell, &mut session, at);
        assert!(on(&shell, Which::Studio) && open(&shell, Which::Studio));
        placed_well(&shell, Which::Studio, size);
    }
}

/// A drag along a slider of the studio popover changes its value and
/// never closes the popover, also when it ends outside it.
#[test]
fn dragging_a_slider_keeps_the_popover_open() {
    let mut session = part();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window {
        ctx: egui::Context::default(),
        size: vec2(1024.0, 700.0),
        time: 0.0,
    };
    window.frames(&mut shell, &mut session, 4);
    let at = chip(&shell, Which::Studio);
    window.click(&mut shell, &mut session, at);
    let track = hit(&shell, Which::Studio, "exposure").unwrap();
    let before = shell.state.look.exposure;
    let from = track.center();
    let to = from - vec2(track.width() * 0.4 + 300.0, -200.0);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let mut steps = vec![vec![Event::PointerMoved(from)], vec![button(from, true)]];
    for k in 1..=8 {
        let p = from + (to - from) * (k as f32 / 8.0);
        steps.push(vec![Event::PointerMoved(p)]);
    }
    steps.push(vec![button(to, false)]);
    for events in steps {
        window.frame(&mut shell, &mut session, events);
    }
    window.frames(&mut shell, &mut session, 3);
    assert!(
        shell.state.look.exposure < before,
        "{}",
        shell.state.look.exposure
    );
    assert!(shell.look_panel_open());
    assert!(hit(&shell, Which::Studio, "panel").is_some());
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
