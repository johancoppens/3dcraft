//! The section's handle in the view (`shell/section_handle.rs`): drawn
//! only while the section is on, hit before the model, a drag on its
//! arrow moving the plane by the arithmetic of the pointer's ray and
//! snapping to the model's places and round values (Alt: none), a double
//! click flipping the side, the dots turning the plane, the number typed
//! into; the narrow layout without it, touch sizes with a finger's
//! targets; hostile documents and cameras; what a drag costs on a model
//! with thousands of faces. Everything it does is `state.section`, read
//! back through `ui.get`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, SnapKind, Workspace};
use craft3d_ui_egui::shell::section_handle::{
    ALONG_AXIS, HIDDEN, HandlePart, NEITHER, NO_PLANE, NUMBER_FIELD, ON_FACE_NOTE, Snapped,
    TAKES_FACES,
};
use craft3d_ui_egui::state::SectionMade;
use craft3d_ui_egui::{RendererChoice, Shell};
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

/// A block of 60 × 40 × 20 with a hole of radius 4 along y through
/// (20, ·, 10) and one of radius 3 along z through (45, 20, ·).
fn drilled() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60, 40, 20], "name": "Block"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 4, "height": 50, "position": [20, -5, 10], "axis": [0, 1, 0], "name": "Across"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 3, "height": 30, "position": [45, 20, -5], "name": "Down"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Across", "name": "Once"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Once", "tool": "Down", "name": "Drilled"}),
    );
    s
}

struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
    /// The keys held, and those egui was last told of.
    modifiers: Modifiers,
    sent: Modifiers,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: vec2(width, height),
            time: 0.0,
            modifiers: Modifiers::NONE,
            sent: Modifiers::NONE,
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, mut events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        if self.modifiers != self.sent {
            events.insert(0, Event::ModifiersChanged(self.modifiers));
            self.sent = self.modifiers;
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
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

    fn button(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2, pressed: bool) {
        let modifiers = self.modifiers;
        self.frame(
            shell,
            session,
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            }],
        );
    }

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        self.button(shell, session, at, true);
        self.button(shell, session, at, false);
    }

    /// A press at `from`, the pointer moved to `to` in `steps`, and let go
    /// there; `during` sees the shell after each step.
    fn drag(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
        from: Pos2,
        to: Pos2,
        steps: usize,
        mut during: impl FnMut(&Shell),
    ) {
        self.frame(shell, session, vec![Event::PointerMoved(from)]);
        self.button(shell, session, from, true);
        for k in 1..=steps {
            let at = from + (to - from) * (k as f32 / steps as f32);
            self.frame(shell, session, vec![Event::PointerMoved(at)]);
            during(shell);
        }
        self.button(shell, session, to, false);
        self.frame(shell, session, vec![Event::PointerGone]);
    }
}

/// The document's window: CPU renderer, `size`, the section across `plane`
/// at `offset`, seen from `view` orthographically.
fn window_on(
    session: &mut Session,
    size: [f32; 2],
    plane: &str,
    offset: f64,
    view: &str,
) -> (Shell, Window) {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(size[0], size[1]);
    window.frames(&mut shell, session, 3);
    call(
        &mut shell,
        session,
        "ui.set",
        json!({"camera": {"projection": "orthographic"},
               "section": {"on": true, "plane": plane, "offset": offset}}),
    );
    // An empty document has nothing to frame: then the view as it is.
    let _ = shell.handle_method(session, "ui.fit", Some(&json!({"view": view})));
    window.frames(&mut shell, session, 4);
    (shell, window)
}

#[test]
fn the_handle_is_there_only_while_the_section_is_on_and_ui_get_lists_its_places() {
    let mut session = drilled();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1000.0, 700.0);
    window.frames(&mut shell, &mut session, 4);
    assert!(shell.section_handle().is_none());
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": true, "plane": "yz", "offset": 30}}),
    );
    window.frames(&mut shell, &mut session, 3);
    let place = shell
        .section_handle()
        .expect("the handle, while the section is on");
    let viewport = shell.viewport_rect().unwrap();
    assert!(viewport.contains(place.frame.at));
    assert!(viewport.contains_rect(place.number));
    // What the plane snaps to, for an agent too: the faces at x = 0 and
    // 60 and both holes' axes.
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    let snaps: Vec<(f64, String)> = got["section"]["snaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["offset"].as_f64().unwrap(),
                s["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let want = [
        (0.0, "face"),
        (20.0, "axis"),
        (45.0, "axis"),
        (60.0, "face"),
    ];
    assert_eq!(snaps.len(), want.len(), "{snaps:?}");
    for ((g, gk), (w, wk)) in snaps.iter().zip(want) {
        assert!((g - w).abs() < 1e-9 && gk == wk, "{snaps:?}");
    }
    // Off: gone, and the state is the one ui.set left.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": false}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert!(shell.section_handle().is_none());
    assert_eq!(shell.state.section.offset, 30.0);
}

#[test]
fn a_drag_on_the_arrow_moves_the_plane_by_the_pointers_ray_and_snaps_lightly() {
    let mut session = drilled();
    // From the front, across z: the arrow points up the screen.
    let (mut shell, mut window) = window_on(&mut session, [1000.0, 700.0], "xy", 4.0, "front");
    let camera = shell.state.camera;
    let selection = call(&mut shell, &mut session, "ui.get", json!({}))["selection"].clone();
    let place = shell.section_handle().unwrap().clone();
    let [a, b] = place.frame.arrow.expect("seen side on");
    assert!(b.y < a.y - 40.0 && (b.x - a.x).abs() < 0.5, "{a:?} {b:?}");
    let per_point = place.frame.per_point;
    assert!(per_point > 0.0);

    // Alt held: no snapping. Up by 37 points from the middle of the
    // shaft: the plane rises 37 points' worth, to a number that is not
    // round.
    window.modifiers = Modifiers::ALT;
    let grip = a.lerp(b, 0.5);
    let mut seen = Vec::new();
    window.drag(
        &mut shell,
        &mut session,
        grip,
        grip - vec2(0.0, 37.0),
        8,
        |shell| seen.push(shell.state.section.offset),
    );
    window.modifiers = Modifiers::NONE;
    let moved = shell.state.section.offset - 4.0;
    assert!(
        (moved - 37.0 * per_point).abs() < 0.02 * 37.0 * per_point,
        "{moved} against {}",
        37.0 * per_point
    );
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    // The camera did not turn and nothing was picked or selected.
    assert_eq!(shell.state.camera, camera);
    assert_eq!(shell.state.picked, None);
    assert_eq!(
        call(&mut shell, &mut session, "ui.get", json!({}))["selection"],
        selection
    );

    // Without Alt, toward the hole along y at z = 10: within the reach it
    // snaps onto its axis exactly, and says so while it is dragged.
    window.frames(&mut shell, &mut session, 2);
    let place = shell.section_handle().unwrap().clone();
    let [a, b] = place.frame.arrow.unwrap();
    let grip = a.lerp(b, 0.5);
    let near_axis = (10.0 - shell.state.section.offset) / place.frame.per_point + 2.0;
    let mut snapped = Vec::new();
    window.drag(
        &mut shell,
        &mut session,
        grip,
        grip - vec2(0.0, near_axis as f32),
        6,
        |shell| snapped.push(shell.section_snapped()),
    );
    assert_eq!(shell.state.section.offset, 10.0, "{snapped:?}");
    assert_eq!(
        snapped.last().copied().flatten(),
        Some(Snapped::Place(SnapKind::Axis))
    );
    assert_eq!(shell.section_snapped(), None, "the drag is over");
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["state"]["section"]["offset"], json!(10.0));
    assert_eq!(got["section"]["point"], json!([0.0, 0.0, 10.0]));

    // A drag beside the handle turns the camera, as ever.
    let away = shell.viewport_rect().unwrap().left_top() + vec2(60.0, 200.0);
    window.drag(
        &mut shell,
        &mut session,
        away,
        away + vec2(40.0, 0.0),
        4,
        |_| {},
    );
    assert_ne!(shell.state.camera, camera);
    assert_eq!(shell.state.section.offset, 10.0);
}

#[test]
fn a_double_click_on_the_arrow_flips_a_dot_turns_the_plane_and_the_number_is_typed() {
    let mut session = drilled();
    let (mut shell, mut window) = window_on(&mut session, [1000.0, 700.0], "xy", 4.0, "front");
    let place = shell.section_handle().unwrap().clone();
    let [a, b] = place.frame.arrow.unwrap();
    let grip = a.lerp(b, 0.5);
    assert_eq!(place.part_at(grip), Some(HandlePart::Arrow));
    // One click on the arrow does nothing, and selects nothing.
    window.click(&mut shell, &mut session, grip);
    window.frames(&mut shell, &mut session, 1);
    assert!(!shell.state.section.flip);
    assert_eq!(shell.state.picked, None);
    // Twice quickly: the other half, and the arrow turns round.
    window.time += 1.0;
    window.click(&mut shell, &mut session, grip);
    window.click(&mut shell, &mut session, grip);
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.state.section.flip);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["normal"], json!([0.0, 0.0, -1.0]));
    let [a2, b2] = shell.section_handle().unwrap().frame.arrow.unwrap();
    assert!(b2.y > a2.y + 40.0, "it points down now: {a2:?} {b2:?}");
    assert_eq!(shell.state.picked, None);

    // The dot of X: across x through the model's middle; the display
    // bar's choice follows.
    let dot = shell.section_handle().unwrap().dots[0];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(shell.state.section.plane.as_deref(), Some("yz"));
    assert_eq!(shell.state.section.offset, 30.0);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["point"], json!([30.0, 0.0, 0.0]));

    // The number: a click types into it, Enter sets what was typed (an
    // expression with units), Escape leaves it.
    let number = shell.section_handle().unwrap().number.center();
    window.click(&mut shell, &mut session, number);
    window.frames(&mut shell, &mut session, 1);
    assert_eq!(shell.typing().map(|t| t.field), Some(NUMBER_FIELD));
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("1.5 cm".to_string())],
    );
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.typing(), None);
    assert_eq!(shell.state.section.offset, 15.0);
    let number = shell.section_handle().unwrap().number.center();
    window.click(&mut shell, &mut session, number);
    window.frames(&mut shell, &mut session, 1);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("99".to_string())],
    );
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.typing(), None);
    assert_eq!(shell.state.section.offset, 15.0);
    // Text that is no number keeps the field open and changes nothing.
    window.click(&mut shell, &mut session, number);
    window.frames(&mut shell, &mut session, 1);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("wide".to_string())],
    );
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    window.frames(&mut shell, &mut session, 2);
    assert!(shell.typing().is_some_and(|t| t.error.is_some()));
    assert_eq!(shell.state.section.offset, 15.0);
    // Switched off while typing: the typing goes with the handle.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": false}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.typing(), None);
    // Nothing of it changed the document.
    assert_eq!(session.document_revision(), drilled().document_revision());
}

#[test]
fn seen_end_on_the_arrow_is_a_ring_that_does_not_drag() {
    let mut session = drilled();
    // From the top, across z: the normal points at the viewer.
    let (mut shell, mut window) = window_on(&mut session, [1000.0, 700.0], "xy", 4.0, "top");
    let camera = shell.state.camera;
    let place = shell.section_handle().unwrap().clone();
    assert_eq!(place.frame.arrow, None);
    assert!(place.frame.toward);
    let at = place.frame.at;
    window.drag(
        &mut shell,
        &mut session,
        at,
        at + vec2(0.0, -40.0),
        5,
        |_| {},
    );
    // A drag there turns the view; the plane stays.
    assert_eq!(shell.state.section.offset, 4.0);
    assert_ne!(shell.state.camera, camera);
}

#[test]
fn the_narrow_layout_leaves_the_handle_out_and_a_finger_gets_larger_targets() {
    let mut session = drilled();
    let (shell, _) = window_on(&mut session, [390.0, 844.0], "yz", 30.0, "front");
    assert!(shell.narrow());
    assert!(shell.state.section.on);
    assert!(shell.section_handle().is_none(), "no handle on a phone");
    // The slider is still there.
    assert!(shell.section_hits().iter().any(|(n, _)| *n == "offset"));

    // A desk-wide window touched by a finger: the handle, at touch sizes.
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "yz", 30.0, "front");
    let mouse = shell.section_handle().unwrap().clone();
    let finger = shell.viewport_rect().unwrap().left_top() + vec2(10.0, 10.0);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase: egui::TouchPhase::Start,
            pos: finger,
            force: None,
        }],
    );
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase: egui::TouchPhase::End,
            pos: finger,
            force: None,
        }],
    );
    window.frames(&mut shell, &mut session, 3);
    let touch = shell.section_handle().unwrap().clone();
    assert!(
        touch.hit >= 20.0 && touch.hit > mouse.hit,
        "{} {}",
        touch.hit,
        mouse.hit
    );
    assert!(touch.dot_radius > mouse.dot_radius);
    let [a, b] = touch.frame.arrow.unwrap();
    assert!(a.distance(b) >= 70.0, "a finger's arrow: {}", a.distance(b));
}

#[test]
fn hostile_documents_and_cameras_draw_no_handle_and_nothing_panics() {
    // An empty document: the section is on, there is nothing to cut.
    let mut empty = Session::new(Workspace::none());
    run(&mut empty, "file.new", json!({"units": "mm"}));
    let (mut shell, mut window) = window_on(&mut empty, [800.0, 600.0], "xy", 0.0, "iso");
    assert!(shell.section_handle().is_none());
    let got = call(&mut shell, &mut empty, "ui.get", json!({}));
    assert_eq!(got["section"]["snaps"], json!([]));
    window.click(&mut shell, &mut empty, pos2(400.0, 300.0));

    // No document at all.
    let mut none = Session::new(Workspace::none());
    let (shell, _) = window_on(&mut none, [800.0, 600.0], "xy", 0.0, "iso");
    assert!(shell.section_handle().is_none());

    // A failed feature after the block: the block is still cut and handled.
    let mut failed = drilled();
    run(
        &mut failed,
        "feature.sphere",
        json!({"name": "Broken", "radius": "missing * 2"}),
    );
    let (mut shell, mut window) = window_on(&mut failed, [800.0, 600.0], "zx", 20.0, "iso");
    window.frames(&mut shell, &mut failed, 2);
    assert!(shell.section_handle().is_some());

    // A camera that is not a number: no handle, no panic, and the next
    // camera draws it again.
    let good = shell.state.camera;
    shell.state.camera.distance = f64::NAN;
    window.frames(&mut shell, &mut failed, 2);
    assert!(shell.section_handle().is_none());
    window.click(&mut shell, &mut failed, pos2(400.0, 300.0));
    shell.state.camera = good;
    window.frames(&mut shell, &mut failed, 2);
    assert!(shell.section_handle().is_some());

    // An offset far beyond the model: the handle where the plane is,
    // a drag held to the model's range.
    call(
        &mut shell,
        &mut failed,
        "ui.set",
        json!({"section": {"offset": 1e9}}),
    );
    window.frames(&mut shell, &mut failed, 3);
    if let Some([a, b]) = shell.section_handle().and_then(|p| p.frame.arrow) {
        window.drag(&mut shell, &mut failed, a.lerp(b, 0.5), a, 4, |_| {});
        let offset = shell.state.section.offset;
        assert!((0.0..=40.0).contains(&offset), "{offset}");
    }
}

#[test]
fn on_thousands_of_faces_the_places_are_capped_and_a_drag_costs_little() {
    // 20 × 20 drilled blocks: 400 bodies' worth of faces in one body,
    // 2800 faces, 400 hole axes along y.
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [8, 8, 8], "name": "Cube"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 2, "height": 12, "position": [4, -2, 4], "axis": [0, 1, 0], "name": "Bore"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Cube", "tool": "Bore", "name": "Bored"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Bored", "count": 20, "step": [10, 0, 0], "name": "Row"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Row", "count": 20, "step": [0, 0, 10], "name": "Wall"}),
    );
    let (mut shell, mut window) = window_on(&mut s, [1200.0, 800.0], "xy", 3.0, "front");
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    let faces = run(&mut s, "document.inspect", json!({}))["bodies"][0]["faceNames"]
        .as_array()
        .map_or(0, Vec::len);
    assert!(faces >= 2800, "{faces} faces");
    let triangles = got["scene"]["triangles"].as_u64().unwrap();
    assert!(triangles > 40_000, "{}", got["scene"]);
    // Across z: the cubes' tops and bottoms and the bores' axes, 60
    // offsets, every one of them listed (fewer than the cap).
    let snaps = got["section"]["snaps"].as_array().unwrap().len();
    assert_eq!(snaps, 60, "{}", got["section"]["snaps"]);
    // A drag across the wall: each frame moves the plane; the time per
    // frame of the shell (resolving, cutting, the CPU's quick picture)
    // is printed for the record.
    let place = shell.section_handle().unwrap().clone();
    let [a, b] = place.frame.arrow.unwrap();
    let grip = a.lerp(b, 0.5);
    let started = std::time::Instant::now();
    let mut frames = 0;
    window.drag(
        &mut shell,
        &mut s,
        grip,
        grip - vec2(0.0, 300.0),
        30,
        |_| frames += 1,
    );
    let per_frame = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames + 3);
    eprintln!(
        "COST drag of the section's arrow: {} triangles, {per_frame:.1} ms a frame (CPU renderer, test profile)",
        shell.render_stats().map_or(0, |s| s.triangles)
    );
    assert!(shell.state.section.offset > 3.0);
    sync_cost(&mut shell, &mut s, "the wall of 400 drilled cubes");
}

/// What the shell's own work costs for a frame of a drag (`Shell::sync`
/// after the offset moved: the plane moved, cut, its outline found),
/// against a frame without a change and one that resolves another plane.
fn sync_cost(shell: &mut Shell, session: &mut Session, what: &str) {
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let timed = |shell: &mut Shell, session: &mut Session, change: &dyn Fn(&mut Shell, usize)| {
        (0..15)
            .map(|k| {
                change(shell, k);
                let started = std::time::Instant::now();
                shell.sync(session);
                started.elapsed().as_secs_f64() * 1000.0
            })
            .collect::<Vec<f64>>()
    };
    let base = shell.state.section.offset;
    let still = median(timed(shell, session, &|_, _| {}));
    let moved = median(timed(shell, session, &|shell, k| {
        shell.state.section.offset = base + 0.01 * k as f64;
    }));
    let turned = median(timed(shell, session, &|shell, k| {
        shell.state.section.plane = Some(["yz", "zx", "xy"][k % 3].to_string());
    }));
    let scene = shell.sync(session).unwrap();
    let triangles = scene.mesh.triangles.len();
    // The direct path's lines, made again whenever the cut moves.
    let lines = median(
        (0..9)
            .map(|_| {
                let marks = shell.marks(Some(&scene));
                let started = std::time::Instant::now();
                let segments = craft3d_ui_egui::direct::segments_marked(
                    Some(&scene),
                    None,
                    &shell.tokens,
                    1.0,
                    &marks,
                    false,
                );
                assert!(!segments.is_empty());
                started.elapsed().as_secs_f64() * 1000.0
            })
            .collect(),
    );
    // What another plane costs, part by part.
    let plane = session
        .section_plane(
            &scene,
            &craft3d_engine::SectionSpec {
                plane: Some("zx".to_string()),
                ..craft3d_engine::SectionSpec::default()
            },
        )
        .unwrap();
    let time = |work: &dyn Fn()| {
        let started = std::time::Instant::now();
        work();
        started.elapsed().as_secs_f64() * 1000.0
    };
    let places = time(&|| {
        plane.snaps(&scene);
    });
    let slabs = time(&|| {
        craft3d_render::Slabs::new(&scene.mesh, plane.base, plane.along).unwrap();
    });
    let bounds = time(&|| {
        craft3d_render::section_bounds(&scene.mesh);
    });
    eprintln!(
        "COST {what}, {triangles} triangles: a sync {still:.2} ms unchanged, {moved:.2} ms with the plane moved (a frame of a drag), {turned:.2} ms with another plane (its places {places:.2} ms, slabs {slabs:.2} ms, box {bounds:.2} ms); the direct path's lines made again {lines:.2} ms"
    );
}

/// The turbofan of the benchmark (130 thousand triangles at the display
/// quality in this window): what the shell's work for a frame of a drag
/// costs on a heavy model. On demand: `-- --ignored`.
#[test]
#[ignore = "regenerates the turbofan; run with --ignored for the numbers"]
fn a_drag_on_the_turbofan_costs_this_much() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../engine/tests/bench_models/turbofan.steps.json");
    let script: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for step in script.as_array().unwrap() {
        let params = step.get(1).cloned().unwrap_or_else(|| json!({}));
        run(&mut s, step[0].as_str().unwrap(), params);
    }
    let (mut shell, _window) = window_on(&mut s, [1400.0, 900.0], "zx", 0.0, "iso");
    sync_cost(&mut shell, &mut s, "the turbofan");
}

/// The drilled block with its top's edge along y at x = 60 chamfered by
/// 8: a flat face at a slant, its outward normal (1, 0, 1) / √2.
fn chamfered() -> Session {
    let mut s = drilled();
    run(
        &mut s,
        "feature.chamfer",
        json!({"edges": [["F1:face(1)", "F1:face(5)"]], "distance": 8, "name": "Bevel"}),
    );
    s
}

/// Selects the one face `filter` finds, as a click would.
fn select_face(session: &mut Session, filter: Value) -> Value {
    let found = run(
        session,
        "select.query",
        json!({"kind": "face", "where": filter, "then": "replace"}),
    );
    assert_eq!(found["count"], 1, "{found}");
    found["items"][0].clone()
}

#[test]
fn the_plane_goes_parallel_to_a_selected_flat_face_and_stays_when_the_selection_changes() {
    let mut session = chamfered();
    let slant = select_face(&mut session, json!({"normal": {"along": [1, 0, 1]}}));
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "xy", 4.0, "iso");
    window.frames(&mut shell, &mut session, 2);
    // One flat face selected: the fourth button lays the plane along it.
    let choice = shell.section_face_choice().cloned().expect("the face");
    let (feature, face) = choice.faces.first().cloned().unwrap();
    assert_eq!(face, slant["name"].as_str().unwrap());
    assert_eq!(choice.refused, None);
    assert_eq!(choice.made, Some(SectionMade::Face));
    assert_eq!(shell.section_handle().unwrap().dots.len(), 4);
    // On a window this wide the bar has the Section button alone: the
    // group in the view has the rest (the developer's decision of
    // 2026-11-17).
    let names: Vec<&str> = shell.section_hits().iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["toggle"]);
    // The group's fourth button: the plane through the face, along its
    // outward normal, nothing cut yet.
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(shell.state.section.plane, Some(face.clone()));
    assert_eq!(shell.state.section.feature, Some(feature.clone()));
    assert_eq!(
        (shell.state.section.offset, shell.state.section.flip),
        (0.0, false)
    );
    assert!(shell.section_on_face());
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["direction"], "face");
    assert_eq!(got["section"]["made"], "face");
    assert_eq!(got["state"]["section"]["made"], "face");
    let n: Vec<f64> = got["section"]["normal"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let h = std::f64::consts::FRAC_1_SQRT_2;
    assert!(
        (n[0] - h).abs() < 1e-9 && n[1].abs() < 1e-9 && (n[2] - h).abs() < 1e-9,
        "{n:?}"
    );
    // The face itself is a place the plane snaps to, at 0.
    assert!(
        got["section"]["snaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["offset"].as_f64().unwrap().abs() < 1e-9 && s["kind"] == "face"),
        "{}",
        got["section"]["snaps"]
    );
    // The arrow points out of the face; dragged back into the body (Alt:
    // as it is), the plane goes in by as much.
    let place = shell.section_handle().unwrap().clone();
    let [a, b] = place.frame.arrow.expect("seen side on");
    let way = (b - a).normalized();
    let grip = a.lerp(b, 0.5);
    window.modifiers = Modifiers::ALT;
    window.drag(&mut shell, &mut session, grip, grip - way * 30.0, 6, |_| {});
    window.modifiers = Modifiers::NONE;
    window.frames(&mut shell, &mut session, 2);
    let offset = shell.state.section.offset;
    let expected = -30.0 * place.frame.per_point;
    assert!(
        (offset - expected).abs() < 0.05 * expected.abs(),
        "{offset} against {expected}"
    );
    // The outline the shell keeps for the face's plane, moved by the drag
    // and found among the slabs, is the one a fresh cut finds over the
    // whole mesh, at several offsets along the face's normal.
    for offset in [offset, -0.37, -3.0, -11.3] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"section": {"offset": offset}}),
        );
        let scene = shell.sync(&mut session).unwrap();
        let kept = shell.marks(Some(&scene)).cut.unwrap().outline;
        let fresh =
            craft3d_ui_egui::viewport::SectionCut::new(&scene, shell.section().unwrap()).unwrap();
        assert!(!kept.is_empty(), "{offset}");
        assert_eq!(*kept, *fresh.outline, "{offset}");
    }
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"offset": offset}}),
    );
    window.frames(&mut shell, &mut session, 2);
    // The selection goes: the plane stays on the face, its button
    // chosen; what a click would make says it takes faces.
    run(&mut session, "select.set", json!({"mode": "clear"}));
    window.frames(&mut shell, &mut session, 3);
    let none = shell.section_face_choice().cloned().unwrap();
    assert_eq!(
        (none.made, none.refused.as_deref()),
        (None, Some(TAKES_FACES))
    );
    assert_eq!(shell.state.section.plane, Some(face.clone()));
    assert_eq!(shell.state.section.offset, offset);
    assert_eq!(shell.section_handle().unwrap().dots.len(), 4);
    // X takes it back across x.
    let x = shell.section_handle().unwrap().dots[0];
    window.click(&mut shell, &mut session, x);
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(shell.state.section.plane.as_deref(), Some("yz"));
    assert_eq!(shell.state.section.made, None);
    assert!(!shell.section_on_face());
    // The fourth button stays, greyed: nothing selected gives no plane.
    assert_eq!(shell.section_handle().unwrap().dots.len(), 4);
    let before = shell.state.section.clone();
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.state.section, before);

    // A hole's wall is not flat: the fourth button gives the plane through
    // its axis instead (the developer, 2026-11-18;
    // `a_hole_gives_the_plane_through_its_axis_facing_the_viewer`), and
    // ui.set still refuses to lay a plane on it, in words.
    let wall = select_face(&mut session, json!({"hole": true, "radius": 3}));
    window.frames(&mut shell, &mut session, 3);
    let round = shell.section_face_choice().cloned().unwrap();
    assert_eq!(round.made, Some(SectionMade::Axis), "{round:?}");
    let before = shell.state.section.clone();
    let error = shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"section": {"plane": wall["name"], "feature": wall["feature"]}})),
        )
        .unwrap()
        .unwrap_err();
    assert!(error.contains("not flat"), "{error}");
    assert_eq!(shell.state.section, before);
    // The fourth dot over the handle: the slanted face again.
    select_face(&mut session, json!({"normal": {"along": [1, 0, 1]}}));
    window.frames(&mut shell, &mut session, 3);
    let dot = shell.section_handle().unwrap().dots[3];
    assert_eq!(
        shell.section_handle().unwrap().part_at(dot),
        Some(HandlePart::Dot(3))
    );
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.section_on_face());
    assert_eq!(shell.state.section.offset, 0.0);
    // An agent lays a plane in any direction the same way.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"point": [52, 20, 20], "normal": [1, 0, 1], "offset": -2}}),
    );
    window.frames(&mut shell, &mut session, 2);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["direction"], "free");
    assert!(!shell.section_on_face());
    // The face still selected: the fourth dot stays, not chosen.
    assert_eq!(shell.section_handle().unwrap().dots.len(), 4);
    assert_eq!(got["section"]["made"], Value::Null);
}

/// Selects the faces `filters` find, one each, as clicks with Shift
/// would: their items.
fn select_faces(session: &mut Session, filters: &[Value]) -> Vec<Value> {
    let mut items = Vec::new();
    for filter in filters {
        let found = run(
            session,
            "select.query",
            json!({"kind": "face", "where": filter}),
        );
        assert_eq!(found["count"], 1, "{filter}: {found}");
        items.push(found["items"][0].clone());
    }
    let picked: Vec<Value> = items
        .iter()
        .map(|item| json!({"kind": "face", "feature": item["feature"], "name": item["name"]}))
        .collect();
    run(session, "select.set", json!({"items": picked}));
    items
}

/// The section's plane as `ui.get` says it, and how it was made.
fn plane_of(shell: &mut Shell, session: &mut Session) -> ([f64; 3], [f64; 3], Value) {
    let got = call(shell, session, "ui.get", json!({}));
    let triple = |v: &Value| -> [f64; 3] {
        let a: Vec<f64> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        [a[0], a[1], a[2]]
    };
    (
        triple(&got["section"]["point"]),
        triple(&got["section"]["normal"]),
        got["section"]["made"].clone(),
    )
}

/// The developer's decision of 2026-11-17: the fourth button takes its
/// plane from what is selected. Two parallel flat faces: the plane midway
/// between them; two that are not: the bisector of their angle through the
/// material, and at a second click the other one; anything else: greyed,
/// saying it takes one or two flat faces. The plate says what the plane
/// is; the plane stays when the selection changes; `ui.get` and `ui.set`
/// carry its point, normal and how it was made.
#[test]
fn two_faces_give_the_plane_midway_or_their_bisector_and_the_plate_says_which() {
    let mut session = chamfered();
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "xy", 4.0, "iso");
    // The block's two ends, at x = 0 and x = 60, facing apart: midway.
    select_faces(
        &mut session,
        &[
            json!({"normal": {"along": [-1, 0, 0]}}),
            json!({"normal": {"along": [1, 0, 0]}}),
        ],
    );
    window.frames(&mut shell, &mut session, 3);
    let choice = shell.section_face_choice().cloned().unwrap();
    assert_eq!(choice.made, Some(SectionMade::Mid), "{choice:?}");
    assert_eq!(choice.faces.len(), 2);
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let (point, normal, made) = plane_of(&mut shell, &mut session);
    assert_eq!(made, "mid");
    assert!((point[0] - 30.0).abs() < 1e-9, "{point:?}");
    assert!(
        normal[0].abs() > 1.0 - 1e-9 && normal[1].abs() < 1e-9,
        "{normal:?}"
    );
    assert_eq!(shell.state.section.made, Some(SectionMade::Mid));
    assert_eq!(shell.state.section.plane, None);
    // The cut is the block's middle: the range runs from end to end.
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    let range = got["section"]["range"].as_array().unwrap();
    let span = (range[1].as_f64().unwrap() - range[0].as_f64().unwrap()).abs();
    assert!((span - 60.0).abs() < 1e-6, "{range:?}");

    // The top (z = 20) and the chamfer's slant: their bisector through
    // the material; the other, square to it, only touches the block along
    // the edge, so it is not offered (the review of 2026-11-17, round
    // three) and a second click keeps the plane.
    select_faces(
        &mut session,
        &[
            json!({"normal": {"along": [0, 0, 1]}}),
            json!({"normal": {"along": [1, 0, 1]}}),
        ],
    );
    window.frames(&mut shell, &mut session, 3);
    let choice = shell.section_face_choice().cloned().unwrap();
    assert_eq!(choice.made, Some(SectionMade::Bisector), "{choice:?}");
    assert_eq!(choice.planes.len(), 1, "{choice:?}");
    // The plane stayed midway while the selection changed.
    assert_eq!(plane_of(&mut shell, &mut session).2, "mid");
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let (point, first, made) = plane_of(&mut shell, &mut session);
    assert_eq!(made, "bisector");
    // Halving 45 degrees between +z and (1, 0, 1): the normal lies in
    // the xz plane, 22.5 degrees from each face's line, and the plane
    // passes through the edge where the two faces meet.
    let n = craft3d_math::Vec3::new(first[0], first[1], first[2]);
    let up = craft3d_math::Vec3::Z;
    let slant = craft3d_math::Vec3::new(1.0, 0.0, 1.0).normalized().unwrap();
    assert!(first[1].abs() < 1e-9, "{first:?}");
    assert!(
        (n.dot(up).abs() - n.dot(slant).abs()).abs() < 1e-9,
        "{first:?}"
    );
    let _ = point;
    // The plate says what the plane is.
    assert!(shell.section_handle().unwrap().number.width() > 0.0);
    // The group moves with the plane: the button where it is now.
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let (_, second, made) = plane_of(&mut shell, &mut session);
    assert_eq!(made, "bisector");
    assert_eq!(second, first);
    // Through the material: the steep one, 67.5 degrees from each face
    // (the outer one would lie 22.5 from them).
    assert!(n.x.abs() > n.z.abs(), "steep, into the block: {first:?}");

    // Anything else: greyed, saying it takes one or two flat faces, and
    // a click changes nothing: three faces, an edge, a face and an edge.
    let before = shell.state.section.clone();
    select_faces(
        &mut session,
        &[
            json!({"normal": {"along": [0, 0, 1]}}),
            json!({"normal": {"along": [1, 0, 1]}}),
            json!({"normal": {"along": [-1, 0, 0]}}),
        ],
    );
    window.frames(&mut shell, &mut session, 3);
    let choice = shell.section_face_choice().cloned().unwrap();
    assert_eq!(
        (choice.made, choice.refused.as_deref()),
        (None, Some(TAKES_FACES))
    );
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.state.section, before);
    // Two faces of which one is a hole's wall: not flat, says so.
    select_faces(
        &mut session,
        &[
            json!({"normal": {"along": [0, 0, 1]}}),
            json!({"hole": true, "radius": 3}),
        ],
    );
    window.frames(&mut shell, &mut session, 3);
    // The word button's words (the UX round of 2026-11-18, V2): the two
    // faces give no plane.
    let choice = shell.section_face_choice().cloned().unwrap();
    assert_eq!(choice.refused.as_deref(), Some(NO_PLANE), "{choice:?}");

    // An agent: ui.set with point, normal and made, read back by ui.get;
    // `made` that does not fit the plane is refused (the review of
    // 2026-11-17: it was dropped without a word), and a plane by name
    // takes it away.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"point": [30, 0, 0], "normal": [1, 0, 0], "made": "mid"}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(plane_of(&mut shell, &mut session).2, "mid");
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"plane": "xy"}}),
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(plane_of(&mut shell, &mut session).2, Value::Null);
    let before = shell.state.clone();
    for odd in [
        json!({"section": {"plane": "zx", "made": "bisector"}}),
        json!({"section": {"plane": "zx", "made": "face"}}),
        json!({"section": {"point": [1, 2, 3], "normal": [0, 0, 1], "made": "face"}}),
        json!({"section": {"made": "mid"}}),
    ] {
        let error = shell
            .handle_method(&mut session, "ui.set", Some(&odd))
            .unwrap()
            .unwrap_err();
        assert!(error.contains("does not fit the plane"), "{odd}: {error}");
        assert_eq!(shell.state, before, "{odd}");
    }
    let error = shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({"section": {"made": "halfway"}})),
        )
        .unwrap()
        .unwrap_err();
    assert!(
        error.contains("made") || error.contains("variant"),
        "{error}"
    );
}

/// Turns the camera, perspective, so that the section's normal across x
/// is `off` degrees from the line of sight, looking at what it looked at
/// (the review's reproducer, 2026-11-17).
fn look_along_x_off_by(shell: &mut Shell, session: &mut Session, window: &mut Window, off: f64) {
    call(
        shell,
        session,
        "ui.set",
        json!({"camera": {"projection": "perspective"}}),
    );
    let _ = shell.handle_method(session, "ui.fit", Some(&json!({"view": "left"})));
    window.frames(shell, session, 2);
    let mut camera = shell.state.camera;
    camera.auto_fit = false;
    camera.yaw += off;
    shell.state.camera = camera;
    window.frames(shell, session, 3);
}

/// A press on the arrow's middle, the pointer moved along the arrow on
/// the screen in `steps` of `step` points, `between` before each, then
/// let go; the offset after each step.
fn drag_along(
    shell: &mut Shell,
    session: &mut Session,
    window: &mut Window,
    steps: usize,
    step: f32,
    mut between: impl FnMut(usize, &mut Shell, &mut Session),
) -> Vec<f64> {
    let place = shell.section_handle().expect("a handle").clone();
    let [a, b] = place.frame.arrow.expect("seen side on");
    let way = (b - a).normalized();
    let grip = a.lerp(b, 0.5);
    window.frame(shell, session, vec![Event::PointerMoved(grip)]);
    window.button(shell, session, grip, true);
    let mut seen = Vec::new();
    for k in 1..=steps {
        between(k, shell, session);
        window.frame(
            shell,
            session,
            vec![Event::PointerMoved(grip + way * (step * k as f32))],
        );
        seen.push(shell.state.section.offset);
    }
    window.button(shell, session, grip + way * (step * steps as f32), false);
    window.frame(shell, session, vec![Event::PointerGone]);
    seen
}

/// Dragged along the arrow toward and past the point its line runs to
/// on the screen, in perspective, the plane goes to the far end and stays
/// there: the pointer's ray meets the arrow's line behind the eye from
/// then on, and that is no place (review of 2026-11-17: it jumped back to
/// the near end, 550 to 760 points on, at 18.5 to 25 degrees).
#[test]
fn dragged_past_where_the_arrow_runs_to_in_perspective_the_plane_does_not_jump_back() {
    for off in [18.5_f64, 20.0, 22.0, 25.0, 35.0] {
        let mut session = drilled();
        let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "yz", 10.0, "iso");
        look_along_x_off_by(&mut shell, &mut session, &mut window, off);
        if shell
            .section_handle()
            .and_then(|place| place.frame.arrow)
            .is_none()
        {
            continue;
        }
        let seen = drag_along(
            &mut shell,
            &mut session,
            &mut window,
            80,
            10.0,
            |_, _, _| {},
        );
        assert!(seen.iter().all(|v| v.is_finite()), "{off}: {seen:?}");
        assert!(
            seen.windows(2).all(|w| w[1] >= w[0] - 1e-9),
            "{off}: the plane went back: {seen:?}"
        );
        assert_eq!(seen.last().copied(), Some(60.0), "{off}: {seen:?}");
    }
}

/// An agent sets another plane during a drag, or another tab is shown:
/// the drag is over, holds the pointer until it lets go, and writes
/// nothing more (review of 2026-11-17: the agent's Y plane at 5 ended at
/// the X drag's 24).
#[test]
fn a_drag_ends_when_another_sets_the_plane_or_another_tab_is_shown() {
    let mut session = drilled();
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "yz", 20.0, "iso");
    let camera = shell.state.camera;
    drag_along(
        &mut shell,
        &mut session,
        &mut window,
        10,
        4.0,
        |k, shell, session| {
            if k == 5 {
                call(
                    shell,
                    session,
                    "ui.set",
                    json!({"section": {"plane": "zx", "offset": 5.0}}),
                );
            }
        },
    );
    assert_eq!(shell.state.section.plane.as_deref(), Some("zx"));
    assert_eq!(shell.state.section.offset, 5.0);
    assert_eq!(
        shell.state.camera, camera,
        "the rest of the drag turned the view"
    );
    // Only the offset, set by an agent mid-drag: that holds too.
    drag_along(
        &mut shell,
        &mut session,
        &mut window,
        10,
        4.0,
        |k, shell, session| {
            if k == 3 {
                call(
                    shell,
                    session,
                    "ui.set",
                    json!({"section": {"offset": 7.0}}),
                );
            }
        },
    );
    assert_eq!(shell.state.section.offset, 7.0);
    // Another tab mid-drag: the new document's view is left as it is,
    // and back on the first the plane is where the drag left it.
    let before = shell.state.section.offset;
    let mut at_switch = None;
    drag_along(
        &mut shell,
        &mut session,
        &mut window,
        10,
        4.0,
        |k, shell, session| {
            if k == 4 {
                at_switch = Some(shell.state.section.offset);
                run(session, "file.new", json!({"units": "mm"}));
            }
        },
    );
    window.frames(&mut shell, &mut session, 2);
    assert!(!shell.state.section.on, "the new tab has no section");
    let at_switch = at_switch.unwrap();
    assert!(at_switch > before, "{before} {at_switch}");
    run(&mut session, "file.activate", json!({"document": 0}));
    window.frames(&mut shell, &mut session, 3);
    assert!(shell.state.section.on);
    assert_eq!(shell.state.section.offset, at_switch);
}

/// More places than a drag looked at before (1200 faces parallel to the
/// plane: the 1024 lowest were kept): every one is kept, and `ui.get`
/// lists those nearest the plane, with how many there are (review of
/// 2026-11-17).
#[test]
fn on_hundreds_of_plates_every_place_is_kept_and_ui_get_lists_those_near_the_plane() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 0.4], "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Plate", "count": 600, "step": [0, 0, 1], "name": "Stack"}),
    );
    let (mut shell, mut window) = window_on(&mut s, [1200.0, 800.0], "xy", 590.0, "front");
    window.frames(&mut shell, &mut s, 2);
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    assert_eq!(got["section"]["snapsTotal"], 1200, "{}", got["section"]);
    let listed: Vec<f64> = got["section"]["snaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["offset"].as_f64().unwrap())
        .collect();
    assert_eq!(listed.len(), 64);
    assert!(
        listed.first().unwrap() < &590.0 && listed.last().unwrap() > &590.0,
        "{listed:?}"
    );
    // The top plate's bottom, 599, is a place a drag snaps to.
    let place = shell.section_handle().unwrap().clone();
    let [a, b] = place.frame.arrow.unwrap();
    let grip = a.lerp(b, 0.5);
    let points = ((599.0 - 590.0) / place.frame.per_point) as f32;
    let way = (b - a).normalized();
    let mut snapped = None;
    window.drag(&mut shell, &mut s, grip, grip + way * points, 6, |shell| {
        snapped = shell.section_snapped();
    });
    assert_eq!(snapped, Some(Snapped::Place(SnapKind::Face)));
    assert_eq!(shell.state.section.offset, 599.0);
}

/// A `ui.set` refused for its section changes nothing of the state, the
/// section's or any other field's, now or in the frames after; the error
/// says both the window's X, Y, Z and the planes' yz, zx, xy (seen by the
/// coordinator on 2026-11-17: `{"on": true, "plane": "y"}`).
#[test]
fn a_refused_section_changes_nothing_and_says_x_y_z_and_the_planes() {
    let mut session = drilled();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1000.0, 700.0);
    window.frames(&mut shell, &mut session, 3);
    let before = shell.state.clone();
    for patch in [
        json!({"section": {"on": true, "plane": "y"}}),
        json!({"section": {"on": true, "plane": "Y"}}),
        json!({"camera": {"yaw": 10}, "transparency": 0.5, "section": {"on": true, "plane": "z"}}),
    ] {
        let error = shell
            .handle_method(&mut session, "ui.set", Some(&patch))
            .unwrap()
            .unwrap_err();
        assert!(
            error.contains("across X lies on the plane yz, across Y on zx, across Z on xy"),
            "{patch}: {error}"
        );
        assert_eq!(shell.state, before, "{patch}");
        window.frames(&mut shell, &mut session, 3);
        assert!(!shell.state.section.on, "{patch}");
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        assert_eq!(got["section"], Value::Null, "{patch}");
        assert!(shell.section_handle().is_none());
    }
    // Before the window has drawn anything at all.
    let mut fresh = Shell::with_renderer(RendererChoice::Cpu);
    let untouched = fresh.state.clone();
    assert!(
        fresh
            .handle_method(
                &mut session,
                "ui.set",
                Some(&json!({"section": {"on": true, "plane": "x"}}))
            )
            .unwrap()
            .is_err()
    );
    assert_eq!(fresh.state.section, untouched.section);
}

/// On a window as wide as a computer's the bar has the Section button
/// alone, the group in the view the rest; narrower than the narrow layout
/// the bar keeps them, and the handle is left out (the developer's
/// decision of 2026-11-17).
#[test]
fn the_bar_keeps_the_section_options_on_a_phone_only() {
    for (width, wide) in [
        (1280.0, true),
        (1000.0, true),
        (790.0, false),
        (390.0, false),
    ] {
        let mut session = drilled();
        let (shell, _) = window_on(&mut session, [width, 800.0], "yz", 20.0, "iso");
        let names: Vec<&str> = shell.section_hits().iter().map(|(n, _)| *n).collect();
        if wide {
            assert_eq!(names, ["toggle"], "{width}");
            let place = shell.section_handle().expect("the group");
            assert_eq!(place.dots.len(), 4, "{width}");
        } else {
            assert_eq!(
                names,
                ["toggle", "x", "y", "z", "face", "offset", "flip"],
                "{width}"
            );
            assert!(shell.section_handle().is_none(), "{width}");
        }
    }
}

/// The review's fixture: the chamfered block, and a second block 20 wide
/// at x = 100 turned about z by `turn`.
fn two_bodies(turn: &str) -> Session {
    let mut s = chamfered();
    run(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 20], "position": [100, 0, 0], "angle": turn, "name": "Second"}),
    );
    s
}

/// The review of 2026-11-17: faces all but parallel (two blocks' +x ends,
/// one turned by a ten-millionth of a degree) gave a "bisector" 3.4e10
/// away that cut everything or nothing. The first plane is now the better
/// conditioned bisector, the plane between the faces, and it cuts the
/// model.
#[test]
fn faces_all_but_parallel_give_the_plane_midway_that_cuts_the_model() {
    for turn in [
        "0.00000005 deg",
        "0.0000001 deg",
        "0.0000003 deg",
        "0.001 deg",
    ] {
        let mut session = two_bodies(turn);
        let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "xy", 4.0, "iso");
        for (what, faces, middle) in [
            (
                "same way",
                [
                    json!({"in": "Bevel", "normal": {"along": [1, 0, 0]}}),
                    json!({"in": "Second", "normal": {"along": [1, 0, 0], "tol": 1}}),
                ],
                90.0,
            ),
            (
                "apart",
                [
                    json!({"in": "Bevel", "normal": {"along": [-1, 0, 0]}}),
                    json!({"in": "Second", "normal": {"along": [1, 0, 0], "tol": 1}}),
                ],
                60.0,
            ),
        ] {
            select_faces(&mut session, &faces);
            window.frames(&mut shell, &mut session, 3);
            // Called midway only within the document's angular tolerance
            // (1e-9 rad: 5e-8 degrees is, 1e-7 is not); the plane between
            // the faces either way, the other bisector far out and left
            // out.
            let parallel = turn == "0.00000005 deg";
            let (made, word) = if parallel {
                (SectionMade::Mid, "mid")
            } else {
                (SectionMade::Bisector, "bisector")
            };
            let choice = shell.section_face_choice().cloned().unwrap();
            assert_eq!(choice.made, Some(made), "{turn} {what}: {choice:?}");
            assert_eq!(choice.planes.len(), 1, "{turn} {what}: {choice:?}");
            let dot = shell.section_handle().unwrap().dots[3];
            window.click(&mut shell, &mut session, dot);
            window.frames(&mut shell, &mut session, 3);
            let got = call(&mut shell, &mut session, "ui.get", json!({}));
            assert_eq!(got["section"]["made"], word, "{turn} {what}");
            let point = got["section"]["point"][0].as_f64().unwrap();
            assert!(
                (point - middle).abs() < 0.5,
                "{turn} {what}: {}",
                got["section"]
            );
            for k in [1, 2] {
                let c = got["section"]["point"][k].as_f64().unwrap();
                assert!(c.abs() < 200.0, "{turn} {what}: {}", got["section"]);
            }
            // It cuts the model: there is a range, the section is on.
            assert!(shell.state.section.on, "{turn} {what}");
            let range = got["section"]["range"].as_array().unwrap();
            let (lo, hi) = (range[0].as_f64().unwrap(), range[1].as_f64().unwrap());
            assert!(lo < 0.0 && hi > 0.0, "{turn} {what}: {range:?}");
        }
    }
}

/// The review of 2026-11-17: after an edit the plane midway stays where
/// it was (a section is a view, like the camera: it does not move under
/// the person), and once its two faces no longer make it, it is no longer
/// called midway. A hidden body's face is refused in the window's words.
#[test]
fn after_an_edit_the_plane_stays_and_its_name_goes_once_it_no_longer_holds() {
    let mut session = chamfered();
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "xy", 4.0, "iso");
    select_faces(
        &mut session,
        &[
            json!({"normal": {"along": [-1, 0, 0]}}),
            json!({"normal": {"along": [1, 0, 0]}}),
        ],
    );
    window.frames(&mut shell, &mut session, 3);
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let made = shell.state.section.clone();
    assert_eq!(made.made, Some(SectionMade::Mid));
    // An edit that leaves the two ends where they are: still midway.
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Down", "radius": 2}),
    );
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(shell.state.section, made);
    // Another tab shown and this one again (the review, round three: the
    // record was the window's, and the other tab's section forgot it).
    run(&mut session, "file.new", json!({"units": "mm"}));
    window.frames(&mut shell, &mut session, 3);
    run(&mut session, "file.activate", json!({"document": 0}));
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(shell.state.section, made);
    // The block made longer: its +x end moves to 80; the plane stays at
    // 30, and is no longer called midway.
    run(
        &mut session,
        "feature.edit",
        json!({"feature": "Block", "size": [80, 40, 20]}),
    );
    window.frames(&mut shell, &mut session, 4);
    assert_eq!(shell.state.section.point, made.point);
    assert_eq!(shell.state.section.normal, made.normal);
    assert_eq!(shell.state.section.made, None);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["made"], Value::Null);
    assert_eq!(got["section"]["direction"], "free");

    // A hidden body's face: refused in the window's words.
    select_faces(&mut session, &[json!({"normal": {"along": [0, 0, 1]}})]);
    let feature = run(&mut session, "select.get", json!({}))["items"][0]["feature"].clone();
    run(
        &mut session,
        "feature.visible",
        json!({"feature": feature, "visible": false}),
    );
    window.frames(&mut shell, &mut session, 4);
    if let Some(choice) = shell.section_face_choice().cloned()
        && let Some(why) = choice.refused
    {
        assert!(
            why == TAKES_FACES || why == HIDDEN,
            "in the window's words: {why}"
        );
    }
}

/// The review of 2026-11-17: while the view turned, the group (the dark
/// plate) hopped sides hundreds of times, sometimes under the display bar
/// and the triad. Turned all round in steps of 3 degrees, at three
/// heights, across X, Y and Z: it keeps its side while that stays clear
/// and is never under a control.
#[test]
fn while_the_view_turns_the_group_keeps_its_side_and_clear_of_the_controls() {
    let mut session = drilled();
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "yz", 30.0, "iso");
    let (mut frames, mut hops, mut worst) = (0_usize, 0_usize, 0.0_f32);
    for plane in ["yz", "zx", "xy"] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"section": {"plane": plane}}),
        );
        let mut last: Option<egui::Vec2> = None;
        for pitch in [35.0, 10.0, -25.0] {
            for k in 0..120 {
                let yaw = f64::from(k) * 3.0;
                call(
                    &mut shell,
                    &mut session,
                    "ui.set",
                    json!({"camera": {"yaw": yaw, "pitch": pitch}}),
                );
                window.frames(&mut shell, &mut session, 1);
                let Some(place) = shell.section_handle().cloned() else {
                    last = None;
                    continue;
                };
                frames += 1;
                let from_foot = place.group.center() - place.frame.at;
                if let Some(before) = last {
                    let jump = (from_foot - before).length();
                    if jump > 60.0 {
                        hops += 1;
                        worst = worst.max(jump);
                    }
                }
                last = Some(from_foot);
                let layout = shell.controls_layout().unwrap();
                for (name, r) in layout.parts() {
                    assert!(
                        !r.intersects(place.group),
                        "{plane} pitch {pitch} yaw {yaw}: under the {name}: {:?} {r:?}",
                        place.group
                    );
                }
            }
        }
    }
    eprintln!("TURNED {frames} frames, {hops} hops, the worst {worst} pt");
    assert!(frames > 600, "{frames}");
    // Round one: 283 hops in 4356 frames (6.5 %). Now one in forty at most.
    assert!(hops * 40 <= frames, "{hops} hops in {frames} frames");
}

/// The review of 2026-11-17, round three: two ruled lofts, a flat tray
/// (100 to 90 over 20: opposite sides 28 degrees apart, meeting far above
/// it) and a steeper hopper (40 to 20 over 30). Their opposite sides
/// selected, the fourth button gives the plane halving them, x = 0,
/// through the model (a rule by where the planes meet gave the tray a
/// plane tilted 14 degrees along one side).
#[test]
fn the_opposite_sides_of_a_tray_and_a_hopper_are_halved_by_x_0() {
    for (name, base, top, height) in [
        ("tray", 100.0_f64, 90.0_f64, 20.0_f64),
        ("hopper", 40.0, 20.0, 30.0),
    ] {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "sketch.create",
            json!({"name": "Bottom", "plane": "xy"}),
        );
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Bottom", "from": [-base / 2.0, -base / 2.0], "to": [base / 2.0, base / 2.0]}),
        );
        run(
            &mut s,
            "sketch.create",
            json!({"name": "Top", "plane": "xy", "offset": format!("{height} mm")}),
        );
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Top", "from": [-top / 2.0, -top / 2.0], "to": [top / 2.0, top / 2.0]}),
        );
        run(
            &mut s,
            "feature.loft",
            json!({"sections": ["Bottom", "Top"], "ruled": true, "name": "Tray"}),
        );
        let (mut shell, mut window) = window_on(&mut s, [1200.0, 800.0], "xy", 4.0, "iso");
        let t = (base - top) / 2.0 / height;
        select_faces(
            &mut s,
            &[
                json!({"normal": {"along": [1.0, 0.0, t]}}),
                json!({"normal": {"along": [-1.0, 0.0, t]}}),
            ],
        );
        window.frames(&mut shell, &mut s, 3);
        let dot = shell.section_handle().unwrap().dots[3];
        window.click(&mut shell, &mut s, dot);
        window.frames(&mut shell, &mut s, 3);
        let got = call(&mut shell, &mut s, "ui.get", json!({}));
        let section = &got["section"];
        assert_eq!(section["made"], "bisector", "{name}: {section}");
        let normal = section["normal"][0].as_f64().unwrap();
        let x = section["point"][0].as_f64().unwrap();
        assert!(normal.abs() > 1.0 - 1e-9, "{name}: {section}");
        assert!(x.abs() < 1e-6, "{name}: {section}");
        let scene = shell.sync(&mut s).unwrap();
        let outline = shell
            .marks(Some(&scene))
            .cut
            .map_or(0, |cut| cut.outline.len());
        assert!(outline > 0, "{name}: the plane cuts the model");
    }
}

/// The review of 2026-11-17, round four: whether a bisector cuts was
/// tested against every body in the view, so a second block beyond the
/// chamfered one made the outer bisector of the top and its chamfer
/// (normal (0.383, 0, 0.924), touching the chamfered block along the edge
/// only) come first. Tested against the selected faces' own bodies, the
/// plane through the material comes first, alone.
#[test]
fn another_body_beyond_does_not_make_a_convex_edges_outer_bisector_cut() {
    let mut session = two_bodies("0 deg");
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "xy", 4.0, "iso");
    select_faces(
        &mut session,
        &[
            json!({"in": "Bevel", "normal": {"along": [0, 0, 1]}}),
            json!({"in": "Bevel", "normal": {"along": [1, 0, 1]}}),
        ],
    );
    window.frames(&mut shell, &mut session, 3);
    let choice = shell.section_face_choice().cloned().unwrap();
    assert_eq!(choice.made, Some(SectionMade::Bisector), "{choice:?}");
    assert_eq!(choice.planes.len(), 1, "{choice:?}");
    let [nx, _, nz] = choice.planes[0].1;
    assert!(nx.abs() > nz.abs(), "through the material: {choice:?}");
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    let n = got["section"]["normal"][0].as_f64().unwrap();
    assert!(n.abs() > 0.9, "{}", got["section"]);
}

/// The developer's decision of 2026-11-18 ("Ja, na de spelden"): one
/// round face selected (a hole) makes the fourth button lay the plane
/// through its axis, turned to face the viewer (the view's direction
/// toward the eye less its part along the axis), so the half toward the
/// viewer is cut away; a second click turns it a quarter about the axis;
/// looking along the axis it is refused in words and nothing changes. The
/// plate says `axis`; the plane holds the axis; a drag moves it along its
/// normal and the offset field prints the offset by the one rule.
#[test]
fn a_hole_gives_the_plane_through_its_axis_facing_the_viewer() {
    let mut session = chamfered();
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "xy", 4.0, "iso");
    select_face(&mut session, json!({"hole": true, "radius": 3}));
    window.frames(&mut shell, &mut session, 3);
    let choice = shell.section_face_choice().cloned().unwrap();
    assert_eq!(choice.made, Some(SectionMade::Axis), "{choice:?}");
    let eye = shell.state.camera.toward_eye();
    assert_eq!(
        choice.words(&shell.state.section, eye),
        "Cut through the middle of this hole, facing you"
    );
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let (point, first, made) = plane_of(&mut shell, &mut session);
    assert_eq!(made, "axis");
    assert_eq!(shell.state.section.made, Some(SectionMade::Axis));
    // The hole drilled down at (45, 20): the plane holds its axis.
    assert!(
        (point[0] - 45.0).abs() < 1e-9 && (point[1] - 20.0).abs() < 1e-9,
        "{point:?}"
    );
    assert!(first[2].abs() < 1e-9, "square to the axis: {first:?}");
    // Facing the viewer: the normal is the eye's direction across the axis.
    let across = craft3d_math::Vec3::new(eye.x, eye.y, 0.0)
        .normalized()
        .unwrap();
    assert!(
        (first[0] - across.x).abs() < 1e-9 && (first[1] - across.y).abs() < 1e-9,
        "{first:?} {across:?}"
    );
    // The plate says what the plane is, and the offset is a length.
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["direction"], "free", "{}", got["section"]);
    assert!(shell.section_handle().unwrap().number.width() > 0.0);
    // A second click: a quarter about the axis.
    assert_eq!(
        choice.words(&shell.state.section, eye),
        "Turn the cut a quarter around the axis"
    );
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let (again, second, made) = plane_of(&mut shell, &mut session);
    assert_eq!(made, "axis");
    assert_eq!(again, point);
    let dot_product = first[0] * second[0] + first[1] * second[1] + first[2] * second[2];
    assert!(
        dot_product.abs() < 1e-9 && second[2].abs() < 1e-9,
        "{second:?}"
    );
    // Another plane, then looking down the hole: refused in words,
    // nothing changes. (On the axis's plane already, a click still turns
    // it a quarter: no view is needed for that.)
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"plane": "yz", "offset": 30}}),
    );
    let _ = shell.handle_method(&mut session, "ui.fit", Some(&json!({"view": "top"})));
    window.frames(&mut shell, &mut session, 3);
    let eye = shell.state.camera.toward_eye();
    assert_eq!(choice.words(&shell.state.section, eye), ALONG_AXIS);
    let before = shell.state.section.clone();
    let dot = shell.section_handle().map(|h| h.dots[3]);
    if let Some(dot) = dot {
        window.click(&mut shell, &mut session, dot);
        window.frames(&mut shell, &mut session, 2);
    }
    assert_eq!(shell.state.section, before);
    // Taken by an agent: ui.set with made axis, a point and a normal.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"point": [45, 20, 0], "normal": [1, 0, 0], "made": "axis"}}),
    );
    assert_eq!(shell.state.section.made, Some(SectionMade::Axis));
    let _ = TAKES_FACES;
}

/// The word button's label and hover as `ui.get` says them.
fn word_of(shell: &mut Shell, session: &mut Session) -> (String, String, bool, bool) {
    let got = call(shell, session, "ui.get", json!({}));
    let word = &got["section"]["fromSelection"];
    (
        word["label"].as_str().unwrap_or_default().to_string(),
        word["hover"].as_str().unwrap_or_default().to_string(),
        word["lit"] == true,
        word["greyed"] == true,
    )
}

/// The UX round of 2026-11-18, V2: the plate's fourth button is a word
/// that says what a click will do now, lit where the plane is that
/// already (a click then changes nothing, not a dragged distance), greyed
/// where the selection gives none (a click then says why under the plate
/// and changes no field of the section).
#[test]
fn the_word_button_says_what_a_click_will_do() {
    let mut session = chamfered();
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "xy", 4.0, "iso");
    run(&mut session, "select.set", json!({"mode": "clear"}));
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(
        word_of(&mut shell, &mut session),
        (
            "From selection".to_string(),
            TAKES_FACES.to_string(),
            false,
            true
        )
    );
    // A click on it greyed: its words under the plate, nothing changed.
    let before = shell.state.section.clone();
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.state.section, before);
    assert_eq!(shell.section_note(), Some(TAKES_FACES));
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["section"]["note"], TAKES_FACES);
    // One flat face: On face; a click lays the plane and says how to cut.
    select_face(&mut session, json!({"normal": {"along": [0, 0, 1]}}));
    window.frames(&mut shell, &mut session, 3);
    let (label, hover, lit, greyed) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), hover.as_str(), lit, greyed),
        ("On face", "Lay the cut on this face", false, false)
    );
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let (label, hover, lit, _) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), hover.as_str(), lit),
        ("On face", "The cut lies on this face", true)
    );
    assert_eq!(shell.section_note(), Some(ON_FACE_NOTE));
    // Lit: a click keeps a distance dragged since, and the side.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"offset": -3}}),
    );
    window.frames(&mut shell, &mut session, 2);
    let before = shell.state.section.clone();
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.state.section, before);
    // Two parallel faces: Midway, lit once laid.
    select_faces(
        &mut session,
        &[
            json!({"normal": {"along": [-1, 0, 0]}}),
            json!({"normal": {"along": [1, 0, 0]}}),
        ],
    );
    window.frames(&mut shell, &mut session, 3);
    let (label, hover, ..) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), hover.as_str()),
        ("Midway", "Lay the cut midway between the two faces")
    );
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let (label, hover, lit, _) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), hover.as_str(), lit),
        ("Midway", "The cut lies midway between the two faces", true)
    );
    // Two faces at an angle: Bisect; the only bisector that cuts, laid:
    // lit.
    select_faces(
        &mut session,
        &[
            json!({"normal": {"along": [0, 0, 1]}}),
            json!({"normal": {"along": [1, 0, 1]}}),
        ],
    );
    window.frames(&mut shell, &mut session, 3);
    let (label, hover, ..) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), hover.as_str()),
        (
            "Bisect",
            "Lay the cut through their corner, halving the angle"
        )
    );
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let (label, _, lit, _) = word_of(&mut shell, &mut session);
    assert_eq!((label.as_str(), lit), ("Bisect", true));
    // A hole: Through axis, then Turn 90°.
    select_face(&mut session, json!({"hole": true, "radius": 3}));
    window.frames(&mut shell, &mut session, 3);
    let (label, hover, ..) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), hover.as_str()),
        (
            "Through axis",
            "Cut through the middle of this hole, facing you"
        )
    );
    let dot = shell.section_handle().unwrap().dots[3];
    window.click(&mut shell, &mut session, dot);
    window.frames(&mut shell, &mut session, 3);
    let (label, hover, ..) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), hover.as_str()),
        ("Turn 90°", "Turn the cut a quarter around the axis")
    );
    // Two faces of which one is a hole's wall: no plane.
    select_faces(
        &mut session,
        &[
            json!({"normal": {"along": [0, 0, 1]}}),
            json!({"hole": true, "radius": 3}),
        ],
    );
    window.frames(&mut shell, &mut session, 3);
    let (label, hover, _, greyed) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), hover.as_str(), greyed),
        ("From selection", NO_PLANE, true)
    );
    // The plate says what the plane is in the button's own word.
    assert_eq!(craft3d_ui_egui::state::SectionMade::Mid.label(), "Midway");
}

/// V2: one face neither flat nor round (a ball): greyed, saying so.
#[test]
fn a_ball_gives_no_cut_and_the_word_button_says_why() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.sphere",
        json!({"name": "Ball", "radius": 10}),
    );
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "xy", 0.0, "iso");
    select_face(&mut session, json!({"in": "Ball"}));
    window.frames(&mut shell, &mut session, 3);
    let (label, hover, _, greyed) = word_of(&mut shell, &mut session);
    assert_eq!(
        (label.as_str(), hover.as_str(), greyed),
        ("From selection", NEITHER, true)
    );
}

/// The UX round of 2026-11-18, V3: a cut faces you and cuts something.
/// From the six standard views and the iso view, the first switch-on and
/// each of X, Y and Z cut away the half on the eye's side (the normal
/// into it points to the eye; a plane seen edge on keeps a side); the
/// first plane is across the axis the eye looks along most, Z from above,
/// and from the iso view the axis along which the model is shorter.
#[test]
fn a_cut_faces_the_eye_from_every_view() {
    for view in ["front", "back", "right", "left", "top", "bottom", "iso"] {
        let mut session = drilled();
        let mut shell = Shell::with_renderer(RendererChoice::Cpu);
        let mut window = Window::new(1200.0, 800.0);
        window.frames(&mut shell, &mut session, 3);
        call(&mut shell, &mut session, "ui.fit", json!({"view": view}));
        window.frames(&mut shell, &mut session, 3);
        let toggle = shell
            .section_hits()
            .iter()
            .find(|(n, _)| *n == "toggle")
            .unwrap()
            .1
            .center();
        window.click(&mut shell, &mut session, toggle);
        window.frames(&mut shell, &mut session, 3);
        let eye = shell.state.camera.toward_eye();
        let (_, normal, _) = plane_of(&mut shell, &mut session);
        let n = craft3d_math::Vec3::new(normal[0], normal[1], normal[2]);
        assert!(n.dot(eye) > 0.5, "{view}: the first cut {n:?} {eye:?}");
        match view {
            "top" | "bottom" => assert_eq!(shell.state.section.plane.as_deref(), Some("xy")),
            "front" | "back" => assert_eq!(shell.state.section.plane.as_deref(), Some("zx")),
            "right" | "left" => assert_eq!(shell.state.section.plane.as_deref(), Some("yz")),
            // 60 along X, 40 along Y: the shorter.
            _ => assert_eq!(shell.state.section.plane.as_deref(), Some("zx")),
        }
        for k in 0..3 {
            let Some(dot) = shell.section_handle().map(|h| h.dots[k]) else {
                continue;
            };
            window.click(&mut shell, &mut session, dot);
            window.frames(&mut shell, &mut session, 3);
            let (_, normal, _) = plane_of(&mut shell, &mut session);
            let n = craft3d_math::Vec3::new(normal[0], normal[1], normal[2]);
            let d = n.dot(eye);
            assert!(d > -0.02, "{view}, axis {k}: {n:?} {eye:?}");
            if d.abs() > 0.02 {
                assert!(d > 0.0, "{view}, axis {k}");
            }
        }
    }
    // A part longer along Y from the iso view: across X.
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [40, 60, 20], "name": "Long"}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1200.0, 800.0);
    window.frames(&mut shell, &mut session, 3);
    let toggle = shell
        .section_hits()
        .iter()
        .find(|(n, _)| *n == "toggle")
        .unwrap()
        .1
        .center();
    window.click(&mut shell, &mut session, toggle);
    window.frames(&mut shell, &mut session, 3);
    assert_eq!(shell.state.section.plane.as_deref(), Some("yz"));
}

/// V3: Turn 90° keeps the cut toward the eye: the eye lies on the side
/// cut away (the plane turned a quarter about the axis holds the line of
/// sight, so the eye's own place decides), and a typed distance outside
/// where the plane meets the bodies is brought to the nearer end, and
/// the plate says so.
#[test]
fn a_turned_cut_and_a_typed_distance_cut_something() {
    let mut session = chamfered();
    let (mut shell, mut window) = window_on(&mut session, [1200.0, 800.0], "xy", 4.0, "iso");
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"projection": "perspective"}}),
    );
    select_face(&mut session, json!({"hole": true, "radius": 3}));
    window.frames(&mut shell, &mut session, 3);
    for _ in 0..2 {
        let dot = shell.section_handle().unwrap().dots[3];
        window.click(&mut shell, &mut session, dot);
        window.frames(&mut shell, &mut session, 3);
        let (point, normal, made) = plane_of(&mut shell, &mut session);
        assert_eq!(made, "axis");
        let eye = shell.state.camera.eye();
        let n = craft3d_math::Vec3::new(normal[0], normal[1], normal[2]);
        let p = craft3d_math::Vec3::new(point[0], point[1], point[2]);
        assert!(
            (eye - p).dot(n) > 0.0,
            "the eye on the side cut away: {n:?}"
        );
    }
    // The number typed: 500 on a block 60 long across X.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"plane": "yz", "offset": 30}}),
    );
    window.frames(&mut shell, &mut session, 3);
    let number = shell.section_handle().unwrap().number.center();
    window.click(&mut shell, &mut session, number);
    window.frames(&mut shell, &mut session, 1);
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Text("500".to_string())],
    );
    window.frame(
        &mut shell,
        &mut session,
        vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    window.frames(&mut shell, &mut session, 2);
    assert_eq!(shell.typing(), None);
    assert_eq!(shell.state.section.offset, 60.0);
    assert_eq!(shell.section_note(), Some("clamped to 0 – 60 mm"));
}

/// V2 on a phone: the word button after X, Y and Z is as wide as its
/// widest label where the row has room (480 points), and at least a
/// finger's (40 points) where it has not (390 points), the Offset slider
/// keeping a rail a finger can drag; every part inside the window, none
/// over another.
#[test]
fn on_a_phone_the_word_button_has_its_labels_width_where_there_is_room() {
    for width in [480.0_f32, 390.0] {
        let mut session = drilled();
        let (shell, window) = window_on(&mut session, [width, 844.0], "yz", 20.0, "iso");
        let hit = |name: &str| {
            shell
                .section_hits()
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap_or_else(|| panic!("{width}: no {name} in {:?}", shell.section_hits()))
                .1
        };
        let face = hit("face");
        assert!(face.width() >= 40.0 - 0.5, "{width}: {face:?}");
        if width >= 480.0 {
            let label = window.ctx.fonts_mut(|f| {
                f.layout_no_wrap(
                    "From selection".to_string(),
                    egui::FontId::proportional(shell.tokens.control_font),
                    egui::Color32::WHITE,
                )
                .size()
                .x
            });
            assert!(face.width() >= label + 8.0, "{width}: {face:?} for {label}");
        }
        let (_, rail, _) = shell
            .slider_hits()
            .into_iter()
            .find(|(n, _, _)| *n == "offset")
            .expect("the Offset slider");
        assert!(rail.width() >= 30.0, "{width}: the rail {rail:?}");
        let parts: Vec<egui::Rect> = ["x", "y", "z", "face", "offset", "flip"]
            .into_iter()
            .map(hit)
            .collect();
        for (k, a) in parts.iter().enumerate() {
            assert!(
                a.left() >= -0.5 && a.right() <= width + 0.5,
                "{width}: {a:?}"
            );
            for b in parts.iter().skip(k + 1) {
                assert!(!a.shrink(0.5).intersects(*b), "{width}: {a:?} {b:?}");
            }
        }
    }
}
