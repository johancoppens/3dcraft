//! The studio look's options panel (`shell/look_panel.rs`): shown under
//! the view cube while the Studio chip is on, each control drives a
//! field of `state.look`, which `ui.get` gives back. Driven by clicks in
//! real frames of the shell.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::Shell;
use egui::{Event, PointerButton, Pos2, pos2, vec2};
use serde_json::{Value, json};

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
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
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
            Vec::new(),
        ] {
            self.frame(shell, session, events);
        }
    }
}

fn hit(shell: &Shell, name: &str) -> egui::Rect {
    shell
        .look_hits()
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| *r)
        .unwrap_or_else(|| {
            let names: Vec<&str> = shell.look_hits().iter().map(|(n, _)| n.as_str()).collect();
            panic!("no `{name}` in the panel: {names:?}")
        })
}

/// A click at `fraction` of the way along a slider's track.
fn along(rect: egui::Rect, fraction: f32) -> Pos2 {
    pos2(rect.left() + rect.width() * fraction, rect.center().y)
}

fn look(shell: &mut Shell, session: &mut Session) -> Value {
    call(shell, session, "ui.get", json!({}))["state"]["look"].clone()
}

#[test]
fn every_control_of_the_studio_panel_drives_the_look_state() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(&mut session, "feature.block", json!({"size": [20, 20, 10]}));
    let mut shell = Shell::new();
    let mut window = Window {
        ctx: egui::Context::default(),
        time: 0.0,
    };
    for _ in 0..3 {
        window.frame(&mut shell, &mut session, Vec::new());
    }
    // Plain: no panel.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"look": {"mode": "plain"}}),
    );
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.look_hits().is_empty());
    // The Studio chip switches the look on, and the panel shows.
    let chip = shell.controls_layout().unwrap().studio.unwrap().center();
    window.click(&mut shell, &mut session, chip);
    assert_eq!(look(&mut shell, &mut session)["mode"], "studio");
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(!shell.look_hits().is_empty());

    // The choices.
    for (control, field, value) in [
        ("environment.outdoor", "environment", "outdoor"),
        ("environment.workshop", "environment", "workshop"),
        ("background.plain", "background", "plain"),
        ("background.environment", "background", "environment"),
        ("tonemap.aces", "tonemap", "aces"),
        ("tonemap.linear", "tonemap", "linear"),
        ("tonemap.agx", "tonemap", "agx"),
    ] {
        let at = hit(&shell, control).center();
        window.click(&mut shell, &mut session, at);
        assert_eq!(look(&mut shell, &mut session)[field], value, "{control}");
    }
    // The sliders: a click along the track sets the value there.
    for (control, field, fraction, check) in [
        (
            "rotation",
            "rotation",
            0.2,
            (|v: f64| v < -60.0) as fn(f64) -> bool,
        ),
        ("exposure", "exposure", 0.85, |v| v > 1.5),
        ("groundReflection", "groundReflection", 0.8, |v| v > 0.5),
        ("bloom", "bloom", 0.9, |v| v > 0.6),
    ] {
        let at = along(hit(&shell, control), fraction);
        window.click(&mut shell, &mut session, at);
        let value = look(&mut shell, &mut session)[field].as_f64().unwrap();
        assert!(check(value), "{control}: {value}");
    }
    // The toggles flip their field.
    for (control, field) in [
        ("shadows", "shadows"),
        ("ao", "ao"),
        ("reflections", "reflections"),
        ("ground", "ground"),
    ] {
        let before = look(&mut shell, &mut session)[field].as_bool().unwrap();
        let at = hit(&shell, control).center();
        window.click(&mut shell, &mut session, at);
        assert_eq!(
            look(&mut shell, &mut session)[field].as_bool(),
            Some(!before),
            "{control}"
        );
    }
    // The lens: on with the default f-number, off again.
    let at = hit(&shell, "lens").center();
    window.click(&mut shell, &mut session, at);
    assert_eq!(look(&mut shell, &mut session)["dof"]["fstop"], 2.8);
    let at = hit(&shell, "lens").center();
    window.click(&mut shell, &mut session, at);
    assert_eq!(look(&mut shell, &mut session)["dof"], Value::Null);

    // What `ui.set` sets is what the panel shows next.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"look": {"tonemap": "aces", "exposure": 7.0}}),
    );
    window.frame(&mut shell, &mut session, Vec::new());
    // A value beyond the slider's track is kept while it is not moved.
    assert_eq!(look(&mut shell, &mut session)["exposure"], 7.0);
    assert_eq!(look(&mut shell, &mut session)["tonemap"], "aces");
    // An analysis is drawn in the plain look: the panel steps aside.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"analysis": {"mode": "zebra"}}),
    );
    window.frame(&mut shell, &mut session, Vec::new());
    assert!(shell.look_hits().is_empty());
}
