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
use craft3d_ui_egui::shell::section_handle::{HandlePart, NUMBER_FIELD, Snapped};
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

/// The turbofan of the benchmark (665 thousand triangles at the display
/// quality): what the shell's work for a frame of a drag costs on a heavy
/// model. On demand: `-- --ignored`.
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
