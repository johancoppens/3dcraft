//! The presentations (Embed, 2026-11-14; `UiState::presentation`): how
//! much of the window the shell draws, set as state by the page that
//! embeds the web build. `full` is the window as it was; `view` is the 3D
//! view alone, over the whole window, with its view cube and controls;
//! `form` puts a Jig's form beside the view (a half sheet under it in a
//! narrow window), without the menu, the tabs and the status bar. Drawn
//! in real frames, read back from where the shell laid things out.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::state::Presentation;
use egui::{Pos2, Rect, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A plate with a public width, published and opened as a Jig in use.
fn jig() -> Session {
    let mut s = Session::new(Workspace::memory());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "width", "expr": "60 mm"}),
    );
    run(
        &mut s,
        "variable.publish",
        json!({"name": "width", "min": "20 mm", "max": "200 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": ["width", 40, 30]}),
    );
    run(&mut s, "jig.export", json!({"path": "plate.3dcraftjig"}));
    run(&mut s, "file.close", json!({"discard": true}));
    run(&mut s, "file.open", json!({"path": "plate.3dcraftjig"}));
    assert!(s.document_status().unwrap().jig_in_use);
    s
}

/// Draws `frames` frames of a window of `width` × `height` points.
fn draw(shell: &mut Shell, session: &mut Session, width: f32, height: f32, frames: usize) {
    let ctx = egui::Context::default();
    for frame in 0..frames {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, height))),
            time: Some(frame as f64 / 60.0),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }
}

fn shown(shell: &Shell) -> Vec<String> {
    shell
        .parameter_hits()
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

#[test]
fn view_is_the_3d_view_alone_and_form_the_form_beside_it() {
    let (width, height) = (1280.0, 800.0);
    // The whole window: menu, tabs, status bar, the side panel with the
    // Jig's form and Open for editing.
    let mut session = jig();
    let mut shell = Shell::new();
    draw(&mut shell, &mut session, width, height, 4);
    let full = shell.viewport_rect().unwrap();
    assert!(full.top() > 0.0 && full.bottom() < height, "{full:?}");
    assert!(full.left() > 0.0, "the side panel: {full:?}");
    assert!(!shell.status_hits().is_empty());
    assert!(shown(&shell).iter().any(|h| h == "Open for editing"));

    // The view alone, over the whole window: no menu, no tabs, no side
    // panel, no status bar; the view cube and the controls stay. The web
    // build lets a page choose it; the sketches' lines go off with it.
    assert!(shell.state.show.sketches);
    shell.allow_presentations(true);
    assert!(shell.set_presentation(Presentation::View));
    assert!(!shell.state.show.sketches, "a visitor sees the model");
    draw(&mut shell, &mut session, width, height, 4);
    let view = shell.viewport_rect().unwrap();
    assert_eq!(view, Rect::from_min_size(Pos2::ZERO, vec2(width, height)));
    assert!(shell.status_hits().is_empty());
    assert!(shell.tab_hits().is_empty());
    assert!(shell.tree_panel_rect().is_none());
    assert!(shown(&shell).is_empty(), "no form: {:?}", shown(&shell));
    let got = shell
        .handle_method(&mut session, "ui.get", None)
        .unwrap()
        .unwrap();
    assert_eq!(got["state"]["presentation"], "view");
    assert!(got["scene"]["bodies"].as_u64().unwrap() >= 1);

    // The form beside the view, the whole height: the Jig's fields, not
    // Open for editing (the page decides what is opened).
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"presentation": "form"})),
        )
        .unwrap()
        .unwrap();
    draw(&mut shell, &mut session, width, height, 4);
    let form = shell.viewport_rect().unwrap();
    let panel = shell.tree_panel_rect().unwrap();
    assert_eq!(form.top(), 0.0, "no menu, no tabs: {form:?}");
    assert_eq!(form.bottom(), height, "no status bar: {form:?}");
    assert!(
        (form.left() - panel.right()).abs() < 1.0,
        "{form:?} {panel:?}"
    );
    assert!(shell.status_hits().is_empty());
    assert!(shell.tab_hits().is_empty());
    let fields = shown(&shell);
    assert!(fields.iter().any(|h| h.contains("width")), "{fields:?}");
    assert!(
        !fields.iter().any(|h| h == "Open for editing"),
        "{fields:?}"
    );
    assert!(!shell.state.show.sketches);
    // Still the page's to switch on, in the same patch or after.
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"show": {"sketches": true}})),
        )
        .unwrap()
        .unwrap();
    assert!(shell.state.show.sketches);
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"presentation": "view", "show": {"sketches": true}})),
        )
        .unwrap()
        .unwrap();
    assert!(shell.state.show.sketches);
    assert!(shell.set_presentation(Presentation::Form));

    // In a narrow window the form is a sheet under the view.
    draw(&mut shell, &mut session, 390.0, 844.0, 4);
    assert!(shell.parameters_sheet_rect().is_some());
    assert!(shell.drawer_button_rect().is_none(), "no menu bar");
    shell.state.presentation = Presentation::View;
    draw(&mut shell, &mut session, 390.0, 844.0, 4);
    assert!(shell.parameters_sheet_rect().is_none());

    // And back: the window as before, its sketches' lines on.
    assert!(shell.set_presentation(Presentation::Full));
    assert!(shell.state.show.sketches);
    draw(&mut shell, &mut session, width, height, 4);
    assert_eq!(shell.viewport_rect().unwrap(), full);
    assert!(!shell.status_hits().is_empty());
}

/// The developer's decision of 2026-11-14 (4): on the desktop (a shell
/// that does not allow presentations, as `Shell::new` makes it) `ui.set`
/// of a presentation other than `full` is refused, saying why, and
/// nothing of the patch is taken; the menu, the status bar and its agents
/// pill stay. `full` is taken.
#[test]
fn the_desktop_refuses_a_presentation_other_than_full() {
    let mut session = jig();
    let mut shell = Shell::new();
    assert!(!shell.presentations_allowed());
    draw(&mut shell, &mut session, 1280.0, 800.0, 3);
    for name in ["view", "form"] {
        let error = shell
            .handle_method(
                &mut session,
                "ui.set",
                Some(&json!({"presentation": name, "notice": "taken?"})),
            )
            .unwrap()
            .unwrap_err();
        assert!(
            error.contains("presentation") && error.contains("person's"),
            "{error}"
        );
        assert_eq!(shell.state.presentation, Presentation::Full);
        assert!(shell.state.notice.is_empty(), "nothing of the patch");
        assert!(shell.state.show.sketches);
        let presentation = Presentation::from_name(name).unwrap();
        assert!(!shell.set_presentation(presentation));
        assert_eq!(shell.state.presentation, Presentation::Full);
    }
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"presentation": "full"})),
        )
        .unwrap()
        .unwrap();
    draw(&mut shell, &mut session, 1280.0, 800.0, 3);
    assert!(!shell.status_hits().is_empty());
}

#[test]
fn without_the_whole_window_there_is_no_start_screen() {
    let mut session = Session::new(Workspace::none());
    let mut shell = Shell::new();
    shell.allow_presentations(true);
    draw(&mut shell, &mut session, 1280.0, 800.0, 3);
    assert!(shell.start_shown());
    for presentation in [Presentation::View, Presentation::Form] {
        assert!(shell.set_presentation(presentation));
        draw(&mut shell, &mut session, 1280.0, 800.0, 3);
        assert!(!shell.start_shown(), "{presentation:?}");
        let got = shell
            .handle_method(&mut session, "ui.get", None)
            .unwrap()
            .unwrap();
        assert_eq!(got["startScreen"], false);
    }
}

/// The developer's choice of 2026-11-14 (round 3 of Embed): back in the
/// whole window the sketches' lines are as they were there, on or off,
/// whether the presentation is set by the host or by `ui.set`; a patch
/// that says them itself keeps what it says.
#[test]
fn back_in_full_the_sketches_lines_are_as_they_were() {
    let mut session = jig();
    let mut shell = Shell::new();
    shell.allow_presentations(true);
    let set = |shell: &mut Shell, session: &mut Session, patch: Value| {
        shell
            .handle_method(session, "ui.set", Some(&patch))
            .unwrap()
            .unwrap();
    };
    for (sketches, away) in [
        (false, Presentation::View),
        (true, Presentation::Form),
        (false, Presentation::Form),
        (true, Presentation::View),
    ] {
        shell.state.show.sketches = sketches;
        assert!(shell.set_presentation(away));
        assert!(!shell.state.show.sketches, "{away:?}");
        assert!(shell.set_presentation(Presentation::Full));
        assert_eq!(shell.state.show.sketches, sketches, "{away:?}");
        // The same through `ui.set`.
        set(
            &mut shell,
            &mut session,
            json!({"presentation": away.name()}),
        );
        assert!(!shell.state.show.sketches, "{away:?}");
        set(&mut shell, &mut session, json!({"presentation": "full"}));
        assert_eq!(shell.state.show.sketches, sketches, "{away:?}");
    }
    // A patch that says the lines keeps them; what they were in the whole
    // window comes back after.
    shell.state.show.sketches = false;
    set(
        &mut shell,
        &mut session,
        json!({"presentation": "view", "show": {"sketches": true}}),
    );
    assert!(shell.state.show.sketches);
    assert!(shell.set_presentation(Presentation::Full));
    assert!(!shell.state.show.sketches);
}

/// A window of 1280 × 800 points drawn frame by frame, with the events
/// of each frame, in one egui context (what is open stays open).
struct Window {
    ctx: egui::Context,
    time: f64,
    focused: bool,
}

impl Window {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
            focused: true,
        }
    }

    /// One frame; `over` is drawn after the shell, as a page's own UI.
    fn frame_with(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        events: Vec<egui::Event>,
        over: &mut dyn FnMut(&egui::Context),
    ) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(self.time),
            focused: self.focused,
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| {
            shell.ui(ui, session);
            over(ui.ctx());
        });
        output.textures_delta.clear();
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<egui::Event>) {
        self.frame_with(shell, session, events, &mut |_| {});
    }

    fn frames(&mut self, shell: &mut Shell, session: &mut Session, n: usize) {
        for _ in 0..n {
            self.frame(shell, session, Vec::new());
        }
    }

    fn key(&mut self, shell: &mut Shell, session: &mut Session, key: egui::Key) {
        for pressed in [true, false] {
            self.frame(
                shell,
                session,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        self.frames(shell, session, 2);
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![egui::Event::PointerMoved(at)]);
        for pressed in [true, false] {
            self.frame(
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
        // The pointer leaves the window: what it is over does not count.
        self.frame(shell, session, vec![egui::Event::PointerGone]);
        self.frames(shell, session, 2);
    }

    fn busy(&self, shell: &Shell) -> bool {
        shell.escape_has_work(&self.ctx, &[])
    }
}

/// R2-1 of the review of Embed's round 2 (2026-11-14): in a page, Escape
/// left the canvas while a menu, the export dialog or a choice list stayed
/// open. `Shell::escape_has_work`, which the page's listener reads, is
/// true while something of the window's is open or has the keys, and
/// false once Escape has closed it: one Escape closes it, the next one
/// (nothing to do) is the page's to leave with.
#[test]
fn escape_has_work_while_something_is_open_and_none_once_it_closed() {
    let mut session = Session::new(Workspace::memory());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frames(&mut shell, &mut session, 4);
    assert!(!window.busy(&shell), "nothing open");

    // The export dialog.
    shell.open_export(&mut session).unwrap();
    window.frames(&mut shell, &mut session, 2);
    assert!(window.busy(&shell), "the export dialog");
    window.key(&mut shell, &mut session, egui::Key::Escape);
    assert!(!shell.state.export.open);
    assert!(!window.busy(&shell), "the export dialog closed");

    // A feature's menu in the side panel.
    let feature = shell.tree_rows().first().unwrap().id.clone();
    shell.state.side_panel.menu = Some(feature);
    window.frames(&mut shell, &mut session, 2);
    assert!(window.busy(&shell), "the feature's menu");
    window.key(&mut shell, &mut session, egui::Key::Escape);
    assert!(shell.state.side_panel.menu.is_none());
    assert!(!window.busy(&shell), "the menu closed");

    // The Studio popover.
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"look": {"mode": "studio"}, "panels": {"lookOptions": true}})),
        )
        .unwrap()
        .unwrap();
    window.frames(&mut shell, &mut session, 2);
    assert!(window.busy(&shell), "the Studio popover");
    window.key(&mut shell, &mut session, egui::Key::Escape);
    assert!(!shell.state.panels.look_options);
    assert!(!window.busy(&shell), "the popover closed");

    // A field that has the keys: the See-through value, typed into.
    let (_, _, value) = shell
        .slider_hits()
        .into_iter()
        .find(|(name, _, _)| *name == "see-through")
        .unwrap();
    window.click(&mut shell, &mut session, value.unwrap().center());
    assert!(shell.typing().is_some(), "typing into the value");
    assert!(window.busy(&shell), "a field has the keys");
    window.key(&mut shell, &mut session, egui::Key::Escape);
    assert!(shell.typing().is_none());
    assert!(!window.busy(&shell), "the field gave the keys back");

    // Any area drawn over the panels (a list, a menu of the kit's), but
    // those that stay: the page's own plate named in `staying`.
    let list = egui::Id::new("a list over the panels");
    let mut area = |ctx: &egui::Context| {
        egui::Area::new(list)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| ui.label("an item"));
    };
    window.frame_with(&mut shell, &mut session, Vec::new(), &mut area);
    window.frame_with(&mut shell, &mut session, Vec::new(), &mut area);
    assert!(window.busy(&shell), "an area over the panels");
    assert!(
        !shell.escape_has_work(&window.ctx, &[list]),
        "one that stays"
    );
    window.frames(&mut shell, &mut session, 2);
    assert!(!window.busy(&shell), "gone with its frame");

    // What the state says open but is not drawn takes no Escape, so an
    // Escape always leaves in the end: a feature's menu set (by a page's
    // `ui.set`) while the view alone is shown, without the list.
    shell.allow_presentations(true);
    assert!(shell.set_presentation(Presentation::View));
    let feature = shell.tree_rows().first().unwrap().id.clone();
    shell.state.side_panel.menu = Some(feature);
    window.frames(&mut shell, &mut session, 2);
    assert!(!window.busy(&shell), "a menu that is not drawn");
}

/// The cue the developer asked for (2026-11-14, round 3 of Embed): the
/// web build embedded in a page draws a ring round the window while it
/// has the keyboard, and none without it; a shell that was not asked to
/// (the desktop, the plain page) never does.
#[test]
fn the_focus_ring_is_drawn_while_the_window_has_the_keys() {
    let mut session = jig();
    let mut shell = Shell::new();
    let mut window = Window::new();
    window.frames(&mut shell, &mut session, 2);
    assert!(!shell.focus_ring_shown(), "not asked for");
    shell.set_focus_ring(true);
    window.frames(&mut shell, &mut session, 1);
    assert!(shell.focus_ring_shown(), "asked for, with the keys");
    window.focused = false;
    window.frames(&mut shell, &mut session, 1);
    assert!(!shell.focus_ring_shown(), "without the keys");
}
