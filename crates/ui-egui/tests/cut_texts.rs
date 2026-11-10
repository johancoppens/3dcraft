//! A text cut with "…" to the room it has is said whole on hover, and a
//! label never runs into the control beside it (the guide's pictures,
//! 2026-11-10): the Parameters form's labels for every kind of control at
//! the side panel's default width, the rows of the side panel, the option
//! a dropdown shows and a document's tab.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell, TabPart};
use egui::{Event, Pos2, Rect, pos2, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

struct Window {
    ctx: egui::Context,
    time: f64,
    /// What the last frame painted.
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Window {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
            shapes: Vec::new(),
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
        self.shapes = output.shapes;
        // The host, between frames.
        if let Some(asked) = shell.take_command() {
            let result = session
                .execute(asked.command, &asked.params)
                .map_err(|e| e.message().to_string());
            shell.command_done(result);
        }
    }

    fn idle(&mut self, shell: &mut Shell, session: &mut Session, frames: usize) {
        for _ in 0..frames {
            self.frame(shell, session, Vec::new());
        }
    }

    /// The pointer resting on `at` until the tooltips show.
    fn hover(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        self.idle(shell, session, 60);
    }

    /// Whether the last frame painted `text` whole near `at` (within a
    /// tooltip's reach of it): in a text that was not cut (the row itself
    /// paints it cut), not elsewhere in the window (the status bar names
    /// the document).
    fn shows_whole(&self, text: &str, at: Pos2) -> bool {
        fn walk(shape: &egui::Shape, text: &str, near: Rect) -> bool {
            match shape {
                egui::Shape::Text(t) => {
                    !t.galley.elided && t.galley.text().contains(text) && near.contains(t.pos)
                }
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| walk(s, text, near)),
                _ => false,
            }
        }
        let near = Rect::from_center_size(at, vec2(700.0, 300.0));
        self.shapes.iter().any(|c| walk(&c.shape, text, near))
    }
}

fn hit(shell: &Shell, name: &str) -> Rect {
    shell
        .parameter_hits()
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| *r)
        .unwrap_or_else(|| panic!("no `{name}` in {:?}", shell.parameter_hits()))
}

const STEPPER: &str = "Compartments side by side along the width of the bin";
const CHOICE: &str = "Holes in the base under every compartment";
const OPTION: &str = "Magnets in every corner and in the middle";
const SWITCH: &str = "Stacking lip on top of the walls all around";
const SLIDER: &str = "Thickness of the outer walls of the bin";
const DRAG: &str = "Clearance between the bin and the bins beside it";

/// A Jig whose parameters have labels longer than the side panel has
/// room for beside their controls: one of each kind of control.
fn long_labels() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (name, expr) in [
        ("count", "2"),
        ("holes", "2"),
        ("lip", "1"),
        ("wall", "1.2 mm"),
        ("gap", "0.25 mm"),
    ] {
        run(&mut s, "variable.set", json!({"name": name, "expr": expr}));
    }
    run(
        &mut s,
        "feature.block",
        json!({"name": "Bin", "size": ["40 mm", "20 mm", "wall * 4"]}),
    );
    for params in [
        json!({"name": "count", "label": STEPPER, "kind": "integer", "min": "1", "max": "6", "order": 1}),
        json!({"name": "holes", "label": CHOICE, "kind": "choice", "choices": ["None", OPTION], "order": 2}),
        json!({"name": "lip", "label": SWITCH, "kind": "boolean", "order": 3}),
        json!({"name": "wall", "label": SLIDER, "min": "1.2 mm", "max": "1.8 mm", "order": 4}),
        json!({"name": "gap", "label": DRAG, "order": 5}),
    ] {
        run(&mut s, "variable.publish", params);
    }
    s
}

/// At the side panel's default width (260 points) a label longer than its
/// row leaves the control its room (the Gridfinity bin's "Compartments
/// side by side" ran into its − stepper) and is said whole on hover;
/// the slider's and the drag field's labels are cut before their fields
/// and said whole on hover too, as is a dropdown's option cut to its
/// field.
#[test]
fn a_long_label_never_runs_into_its_control_and_is_whole_on_hover() {
    let mut session = long_labels();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.set_parameters_single_thread(false);
    let mut window = Window::new();
    window.idle(&mut shell, &mut session, 4);
    let panel = shell.tree_panel_rect().unwrap();
    assert!(
        (panel.width() - 260.0).abs() < 2.0,
        "the default width: {panel:?}"
    );
    for (name, label) in [("count", STEPPER), ("holes", CHOICE), ("lip", SWITCH)] {
        let text = hit(&shell, &format!("{name}.label"));
        let control = hit(&shell, name);
        assert!(
            text.right() <= control.left() + 0.5,
            "{name}: the label {text:?} runs into its control {control:?}"
        );
        assert!(
            text.left() >= panel.left() && text.width() > 40.0,
            "{name}: {text:?} in {panel:?}"
        );
        assert!(
            !window.shows_whole(label, text.center()),
            "{name}: not cut at 260 points"
        );
        window.hover(&mut shell, &mut session, text.center());
        assert!(
            window.shows_whole(label, text.center()),
            "{name}: not whole on hover"
        );
        window.hover(
            &mut shell,
            &mut session,
            panel.right_bottom() + vec2(200.0, -200.0),
        );
    }
    // The slider and the drag field paint their labels at their left,
    // before the field.
    for (name, label) in [("wall", SLIDER), ("gap", DRAG)] {
        let field = hit(&shell, &format!("{name}.field"));
        let control = hit(&shell, name);
        let at = pos2(control.left() + 20.0, field.center().y);
        assert!(
            !window.shows_whole(label, at),
            "{name}: not cut at 260 points"
        );
        window.hover(&mut shell, &mut session, at);
        assert!(window.shows_whole(label, at), "{name}: not whole on hover");
        window.hover(
            &mut shell,
            &mut session,
            panel.right_bottom() + vec2(200.0, -200.0),
        );
    }
    // The option the dropdown shows, cut to its field.
    let at = hit(&shell, "holes").center();
    assert!(!window.shows_whole(OPTION, at), "the option is cut");
    window.hover(&mut shell, &mut session, at);
    assert!(window.shows_whole(OPTION, at), "the option whole on hover");
}

/// Names cut in the side panel are said whole on hover: a feature's row
/// (name and value), a variable's and a body's (the feature list cut
/// "Countersunk thread …" with no way to read it), and a document's tab.
#[test]
fn a_name_cut_in_the_side_panel_is_whole_on_hover() {
    const FEATURE: &str = "Countersunk thread for the screw at the back of the bracket";
    const VARIABLE: &str = "distance_between_the_mounting_holes_at_the_back";
    const TITLE: &str = "A title much longer than any tab has room for";
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "variable.set",
        json!({"name": VARIABLE, "expr": "30 mm"}),
    );
    run(
        &mut session,
        "feature.block",
        json!({"name": FEATURE, "size": [VARIABLE, "20 mm", "10 mm"]}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new();
    window.idle(&mut shell, &mut session, 4);
    let (id, tip) = shell
        .side_hits()
        .cut
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("no row cut: {:?}", shell.side_hits().rows));
    assert!(tip.starts_with(FEATURE), "{tip}");
    let row = shell
        .side_hits()
        .rows
        .iter()
        .find(|(r, _)| *r == id)
        .map(|(_, rect)| *rect)
        .unwrap();
    assert!(!window.shows_whole(FEATURE, row.center()), "the row is cut");
    window.hover(&mut shell, &mut session, row.center());
    assert!(
        window.shows_whole(FEATURE, row.center()),
        "the feature's name on hover"
    );
    // The Variables and the Bodies tabs.
    for (tab, name) in [("variables", VARIABLE), ("bodies", FEATURE)] {
        shell
            .handle_method(
                &mut session,
                "ui.set",
                Some(&json!({"panels": {"sideTab": tab}})),
            )
            .unwrap()
            .unwrap();
        window.idle(&mut shell, &mut session, 4);
        let cut = &shell.side_hits().cut;
        assert!(
            cut.iter().any(|(_, tip)| tip.starts_with(name)),
            "{tab}: {cut:?}"
        );
    }
    // A tab whose title is cut.
    let bytes = session.document_bytes().unwrap();
    session
        .open_bytes_titled("long.3dcraft", Some(TITLE), &bytes)
        .unwrap();
    window.idle(&mut shell, &mut session, 4);
    let tab = shell
        .tab_hits()
        .iter()
        .filter(|(part, _)| matches!(part, TabPart::Tab(_)))
        .map(|(_, rect)| *rect)
        .last()
        .unwrap();
    assert!(
        !window.shows_whole(TITLE, tab.center()),
        "the title is cut on its tab"
    );
    window.hover(&mut shell, &mut session, tab.center());
    assert!(
        window.shows_whole(TITLE, tab.center()),
        "the title whole on hover"
    );
}
