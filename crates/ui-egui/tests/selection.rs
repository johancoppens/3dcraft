//! The selection in the window (decisions of 2026-10-21, B1): a click
//! selects what is under it, Shift+click toggles, the same spot again or
//! Alt+click goes through to the face behind, a right click lists what
//! is under the cursor, Ctrl and a drag selects with a box, Esc clears;
//! agents read and set the same selection (`ui.get` › `selection`,
//! `ui.select`, `ui.pick` with `all`).
//!
//! The oracle: three blocks seen from the front, two of them one behind
//! the other, whose faces facing -y are known by `select.query`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell, StatusSlot};
use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// Front (y 0 to 10) hides Back (y 20 to 30) seen from the front; Side
/// stands beside them.
fn blocks() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Front", "size": [10, 10, 10]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Back", "size": [10, 10, 10], "position": [0, 20, 0]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Side", "size": [10, 10, 10], "position": [20, 0, 0]}),
    );
    s
}

/// The faces facing -y, as (Front's, Back's, Side's).
fn front_faces(session: &mut Session) -> Vec<String> {
    let found = run(
        session,
        "select.query",
        json!({"kind": "face", "where": {"normal": {"along": "-y"}}}),
    );
    found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_string())
        .collect()
}

fn selected(shell: &mut Shell, session: &mut Session) -> Vec<(String, String)> {
    let got = call(shell, session, "ui.get", json!({}));
    got["selection"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["kind"].as_str().unwrap().to_string(),
                i["name"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn framed(session: &mut Session) -> Shell {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    call(&mut shell, session, "ui.fit", json!({"view": "front"}));
    call(
        &mut shell,
        session,
        "ui.set",
        json!({"camera": {"projection": "orthographic"}}),
    );
    call(&mut shell, session, "ui.fit", json!({"view": "front"}));
    shell
}

/// Where the middle of a body's face facing the camera is, in the
/// viewport's pixels: found by asking the picture.
fn spot_of(shell: &mut Shell, session: &mut Session, body: &str) -> (f64, f64) {
    let [w, h] = shell.viewport_size();
    for j in (0..h).step_by(8) {
        for i in (0..w).step_by(8) {
            let (x, y) = (f64::from(i) + 0.5, f64::from(j) + 0.5);
            let at = call(
                shell,
                session,
                "ui.pick",
                json!({"x": x, "y": y, "radius": 0}),
            );
            if at["name"] == body {
                // The middle of the run of pixels of that body on this row.
                let mut k = i;
                while k < w
                    && call(
                        shell,
                        session,
                        "ui.pick",
                        json!({"x": f64::from(k) + 0.5, "y": y, "radius": 0}),
                    )["name"]
                        == body
                {
                    k += 4;
                }
                return ((f64::from(i) + f64::from(k)) / 2.0, y + 24.0);
            }
        }
    }
    panic!("{body} is not in the picture");
}

#[test]
fn clicks_select_toggle_and_go_through_to_the_face_behind() {
    let mut session = blocks();
    let mut shell = framed(&mut session);
    let faces = front_faces(&mut session);
    let (x, y) = spot_of(&mut shell, &mut session, "Front");
    let click = |shell: &mut Shell, session: &mut Session, x: f64, y: f64, keys: Value| {
        let mut params = json!({"x": x, "y": y, "select": true, "radius": 0});
        for (key, on) in keys.as_object().unwrap() {
            params[key] = on.clone();
        }
        call(shell, session, "ui.pick", params)
    };
    // Every face along the ray: Front's two, then Back's two.
    let through = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x, "y": y, "all": true}),
    );
    let hits = through["hits"].as_array().unwrap().clone();
    let bodies: Vec<&str> = hits.iter().map(|h| h["name"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["Front", "Front", "Back", "Back"], "{through}");
    // A click selects the face in front.
    let first = click(&mut shell, &mut session, x, y, json!({}));
    assert_eq!(first["selection"]["summary"], "1 face");
    assert_eq!(
        selected(&mut shell, &mut session),
        [("face".to_string(), faces[0].clone())]
    );
    // The same spot again: the next face behind, each time, and round.
    for k in [1, 2, 3, 0, 1] {
        let again = click(&mut shell, &mut session, x + 1.0, y, json!({}));
        assert_eq!(
            again["selection"]["items"][0]["name"], hits[k]["face"],
            "click {k}: {again}"
        );
        assert_eq!(again["highlight"]["face"], hits[k]["face"]);
    }
    // Elsewhere, then Alt: the face behind the front one at once.
    let (sx, sy) = spot_of(&mut shell, &mut session, "Side");
    click(&mut shell, &mut session, sx, sy, json!({}));
    let alt = click(&mut shell, &mut session, x, y + 6.0, json!({"alt": true}));
    assert_eq!(alt["selection"]["items"][0]["name"], hits[1]["face"]);
    // Shift adds and takes out.
    click(&mut shell, &mut session, x, y - 6.0, json!({}));
    let both = click(&mut shell, &mut session, sx, sy, json!({"shift": true}));
    assert_eq!(both["selection"]["summary"], "2 faces", "{both}");
    assert_eq!(
        selected(&mut shell, &mut session),
        [
            ("face".to_string(), faces[0].clone()),
            ("face".to_string(), faces[2].clone())
        ]
    );
    let one = click(
        &mut shell,
        &mut session,
        sx,
        sy + 4.0,
        json!({"shift": true}),
    );
    assert_eq!(one["selection"]["summary"], "1 face");
    // A click on nothing clears it.
    let none = click(&mut shell, &mut session, 1.5, 1.5, json!({}));
    assert_eq!(none["selection"]["count"], 0);
    // shift without select means nothing.
    assert!(
        shell
            .handle_method(
                &mut session,
                "ui.pick",
                Some(&json!({"x": x, "y": y, "shift": true}))
            )
            .unwrap()
            .is_err()
    );
}

#[test]
fn an_agent_sets_the_selection_and_the_window_lights_it() {
    let mut session = blocks();
    let mut shell = framed(&mut session);
    let set = call(
        &mut shell,
        &mut session,
        "ui.select",
        json!({"names": ["Back", "Side"]}),
    );
    assert_eq!(set["summary"], "2 bodies");
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["selection"]["summary"], "2 bodies");
    // The tree's row follows the first selected body.
    assert_eq!(got["state"]["selectedFeature"], "F2");
    // Every face of both bodies is lit, none of Front's.
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let lit = shell.face_highlight(Some(&scene));
    for body in scene.bodies.iter() {
        let first = body.first_face_id;
        for id in first..first + 6 {
            assert_eq!(lit.contains(id), body.name != "Front", "{} {id}", body.name);
        }
    }
    // selectedFeature selects a feature; null takes it out.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"selectedFeature": "F3"}),
    );
    assert_eq!(
        selected(&mut shell, &mut session),
        [("feature".to_string(), "Side".to_string())]
    );
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"selectedFeature": null}),
    );
    assert!(selected(&mut shell, &mut session).is_empty());
    // A face stays selected while the model changes around it, and drops
    // out, said, when it is gone.
    let faces = front_faces(&mut session);
    call(
        &mut shell,
        &mut session,
        "ui.select",
        json!({"items": [{"kind": "face", "name": faces[2], "feature": "Side"}]}),
    );
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Side", "size": [12, 10, 10]}),
    );
    assert_eq!(
        selected(&mut shell, &mut session),
        [("face".to_string(), faces[2].clone())]
    );
    run(&mut session, "feature.delete", json!({"feature": "Side"}));
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["selection"]["count"], 0);
    assert_eq!(got["selection"]["dropped"].as_array().unwrap().len(), 1);
    // Refused: a name that means nothing, and a wrong field.
    for params in [
        json!({"names": ["Nowhere"]}),
        json!({"what": 1}),
        json!({"mode": "swap"}),
    ] {
        let e = shell
            .handle_method(&mut session, "ui.select", Some(&params))
            .unwrap()
            .unwrap_err();
        assert!(e.starts_with("ui.select"), "{e}");
    }
}

/// A window of a fixed size, fed one frame at a time with the keys held.
struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            size: vec2(900.0, 640.0),
            time: 0.0,
        }
    }

    fn frame(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        events: Vec<Event>,
        keys: Modifiers,
    ) {
        self.time += 1.0 / 60.0;
        let mut events = events;
        events.insert(0, Event::ModifiersChanged(keys));
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    fn button(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        at: Pos2,
        button: PointerButton,
        pressed: bool,
        keys: Modifiers,
    ) {
        self.frame(
            shell,
            session,
            vec![Event::PointerButton {
                pos: at,
                button,
                pressed,
                modifiers: keys,
            }],
            keys,
        );
    }

    fn click(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        at: Pos2,
        button: PointerButton,
        keys: Modifiers,
    ) {
        self.frame(shell, session, vec![Event::PointerMoved(at)], keys);
        self.button(shell, session, at, button, true, keys);
        self.button(shell, session, at, button, false, keys);
        for _ in 0..2 {
            self.frame(shell, session, Vec::new(), keys);
        }
        // The double click's clock: the next click is a new one.
        self.time += 1.0;
    }
}

#[test]
fn the_mouse_and_the_keys_select_in_the_window() {
    let mut session = blocks();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new();
    window.frame(&mut shell, &mut session, Vec::new(), Modifiers::NONE);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"projection": "orthographic"}}),
    );
    call(&mut shell, &mut session, "ui.fit", json!({"view": "front"}));
    window.frame(&mut shell, &mut session, Vec::new(), Modifiers::NONE);
    let rect = shell.viewport_rect().unwrap();
    let ppp = f64::from(shell.pixels_per_point());
    let (x, y) = spot_of(&mut shell, &mut session, "Front");
    let (sx, sy) = spot_of(&mut shell, &mut session, "Side");
    let point = |x: f64, y: f64| {
        pos2(
            rect.left() + (x / ppp) as f32,
            rect.top() + (y / ppp) as f32,
        )
    };
    let faces = front_faces(&mut session);
    let none = Modifiers::NONE;
    let shift = Modifiers::SHIFT;
    // A click selects the face; Shift+click adds Side's.
    window.click(
        &mut shell,
        &mut session,
        point(x, y),
        PointerButton::Primary,
        none,
    );
    assert_eq!(
        selected(&mut shell, &mut session),
        [("face".to_string(), faces[0].clone())]
    );
    window.click(
        &mut shell,
        &mut session,
        point(sx, sy),
        PointerButton::Primary,
        shift,
    );
    assert_eq!(selected(&mut shell, &mut session).len(), 2);
    // The status bar says so.
    window.frame(&mut shell, &mut session, Vec::new(), none);
    assert!(
        shell
            .status_hits()
            .iter()
            .any(|(slot, _)| *slot == StatusSlot::Selection),
        "{:?}",
        shell.status_hits()
    );
    // Esc over the viewport clears it.
    window.frame(
        &mut shell,
        &mut session,
        vec![
            Event::PointerMoved(point(x, y)),
            Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: none,
            },
        ],
        none,
    );
    window.frame(&mut shell, &mut session, Vec::new(), none);
    assert!(selected(&mut shell, &mut session).is_empty());
    // A right click lists the four faces under the cursor.
    window.click(
        &mut shell,
        &mut session,
        point(x, y),
        PointerButton::Secondary,
        none,
    );
    let listed = shell.other_menu().expect("the list is open");
    assert_eq!(listed.len(), 4, "{listed:?}");
    assert_eq!(listed[0].1.as_deref(), Some(faces[0].as_str()));
    // Ctrl and a drag around everything: the three bodies.
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::PointerMoved(rect.min + vec2(4.0, 4.0))],
        none,
    );
    let ctrl = Modifiers::COMMAND;
    let from = rect.min + vec2(4.0, 4.0);
    let to = rect.max - vec2(4.0, 4.0);
    window.button(
        &mut shell,
        &mut session,
        from,
        PointerButton::Primary,
        true,
        ctrl,
    );
    for k in 1..=8 {
        let at = from + (to - from) * (k as f32 / 8.0);
        window.frame(
            &mut shell,
            &mut session,
            vec![Event::PointerMoved(at)],
            ctrl,
        );
    }
    let camera = shell.state.camera;
    window.button(
        &mut shell,
        &mut session,
        to,
        PointerButton::Primary,
        false,
        ctrl,
    );
    window.frame(&mut shell, &mut session, Vec::new(), none);
    assert_eq!(shell.state.camera, camera, "a box does not turn the camera");
    let boxed = selected(&mut shell, &mut session);
    let kinds: Vec<&str> = boxed.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(kinds, ["body", "body", "body"], "{boxed:?}");
}
