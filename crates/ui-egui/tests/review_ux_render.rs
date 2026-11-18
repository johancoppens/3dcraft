//! The review of the hover (the UX round of 2026-11-18, V7) and of the
//! window's cut, by `review-ux-render`: hostile `ui.pick {hover}`, what
//! `hovered` says after the document changed under a resting pointer and
//! after a tab switch, what a pick under the pointer costs as the model
//! grows and whether the window stalls on it, how many pictures a hover
//! change makes on the CPU path, and the outline of a face whose boundary
//! has tens of thousands of segments, near and zoomed far in. Findings are
//! printed as `REVIEW` lines.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Vec3;
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::viewport::LineKind;
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, Pos2};
use serde_json::{Value, json};
use std::time::Instant;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn answer(
    shell: &mut Shell,
    session: &mut Session,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    answer(shell, session, method, params.clone())
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

fn hovered(shell: &mut Shell, session: &mut Session) -> Value {
    call(shell, session, "ui.get", json!({}))["hovered"].clone()
}

fn shot(shell: &mut Shell, session: &mut Session) -> String {
    let result = call(shell, session, "ui.screenshot", json!({"scale": 1.0}));
    result["base64"].as_str().unwrap().to_string()
}

/// The pixels of a `ui.screenshot` picture (base64 of a PNG, RGB) within 40
/// (the sum of the channels' differences) of the hover's amber, the
/// selection's colour.
fn amber_pixels(base64: &str) -> usize {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        }
    };
    let mut bytes = Vec::new();
    for chunk in base64.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let bits = chunk.iter().fold(0_u32, |acc, &c| {
            (acc << 6) | if c == b'=' { 0 } else { value(c) }
        });
        let bits = bits << (6 * (4 - chunk.len()));
        bytes.extend_from_slice(&bits.to_be_bytes()[1..4 - pad]);
    }
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut rgb = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgb).unwrap();
    rgb.truncate(info.buffer_size());
    rgb.chunks(3)
        .filter(|px| {
            px.iter()
                .zip([217_u8, 134, 44])
                .map(|(a, b)| u32::from(a.abs_diff(b)))
                .sum::<u32>()
                <= 40
        })
        .count()
}

/// A window of a fixed size, fed one frame at a time, a sixtieth of a
/// second apart.
struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: egui::vec2(width, height),
            time: 0.0,
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
    }

    fn frames(&mut self, shell: &mut Shell, session: &mut Session, count: usize) {
        for _ in 0..count {
            self.frame(shell, session, Vec::new());
        }
    }
}

/// The pixel of the viewport `p` is seen at.
fn pixel_of(shell: &Shell, p: Vec3) -> [f64; 2] {
    let [width, height] = shell.viewport_size();
    let projector = Projector::new(
        &shell.state.camera.to_render(),
        f64::from(width),
        f64::from(height),
    )
    .unwrap();
    let at = projector.project(p).unwrap();
    [at.x, at.y]
}

/// A plate 60 × 40 × 8.
fn plate() -> Session {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    session
}

/// `session` in a window of 1000 × 700, framed from the iso view.
fn in_a_window(session: &mut Session) -> (Shell, Window) {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1000.0, 700.0);
    window.frames(&mut shell, session, 3);
    call(&mut shell, session, "ui.fit", json!({"view": "iso"}));
    window.frames(&mut shell, session, 3);
    (shell, window)
}

/// `ui.pick {hover}` with numbers out of every range, and with every other
/// flag beside it: refused, or taken, and then the frames, `ui.get` and
/// `ui.screenshot` go on; `hovered` is null or a pick.
#[test]
fn a_hostile_hover_is_refused_or_harmless() {
    let mut session = plate();
    let (mut shell, mut window) = in_a_window(&mut session);
    let cases = [
        json!({"x": 1e308, "y": 10, "hover": true}),
        json!({"x": -1e308, "y": -1e308, "hover": true}),
        json!({"x": 1e308, "y": 1e308, "width": 1, "height": 1, "hover": true}),
        json!({"x": 0.5, "y": 0.5, "width": 1, "height": 1, "hover": true}),
        json!({"x": -40, "y": 20, "hover": true}),
        json!({"x": 400, "y": 300, "hover": true, "radius": 0}),
        json!({"x": 400, "y": 300, "hover": true, "radius": 1e308}),
        json!({"x": 400, "y": 300, "hover": true, "all": true}),
        json!({"x": 400, "y": 300, "hover": true, "secondary": true}),
        json!({"x": 400, "y": 300, "hover": true, "select": true}),
        json!({"x": 400, "y": 300, "hover": true, "width": 0}),
        json!({"x": 400, "y": 300, "hover": true, "width": 4_294_967_295_u64, "height": 4_294_967_295_u64}),
        json!({"x": 400, "y": 300, "hover": true, "width": 1e300}),
        json!({"x": null, "y": 300, "hover": true}),
        json!({"x": "400", "y": 300, "hover": true}),
        json!({"x": 400, "y": 300, "hover": "yes"}),
        json!({"x": 400, "y": 300, "hover": false}),
        json!({"x": 400, "y": 300, "hover": true}),
    ];
    for params in cases {
        let result = answer(&mut shell, &mut session, "ui.pick", params.clone());
        window.frames(&mut shell, &mut session, 3);
        let now = hovered(&mut shell, &mut session);
        assert!(now.is_null() || now.is_object(), "{params}: {now}");
        let picture = shot(&mut shell, &mut session);
        assert!(!picture.is_empty(), "{params}");
        eprintln!(
            "REVIEW hostile hover: {params}: {}; hovered {now}",
            match &result {
                Ok(value) => format!("taken, kind {}", value["kind"]),
                Err(e) => format!("refused: {e}"),
            }
        );
    }
}

/// The pointer placed on the top face, then the document changes under it
/// (`edit.undo` takes the plate away, `edit.redo` brings it back, a new
/// document becomes the active tab, the first one again): what `hovered`
/// says before a frame, after one and after four, and whether the
/// screenshot taken before a frame outlines what is gone.
#[test]
fn hovered_after_the_document_changed_under_a_resting_pointer() {
    let mut session = plate();
    let (mut shell, mut window) = in_a_window(&mut session);
    let [x, y] = pixel_of(&shell, Vec3::new(30.0, 20.0, 8.0));
    call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x, "y": y, "hover": true}),
    );
    window.frames(&mut shell, &mut session, 2);
    let first = hovered(&mut shell, &mut session);
    assert_eq!(first["kind"], "face", "{first}");
    // The camera held, so the pictures differ only by what is drawn over
    // the model.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"autoFit": false}}),
    );
    let mut report = Vec::new();
    for (what, id, params) in [
        ("edit.undo", "edit.undo", json!({})),
        ("edit.redo", "edit.redo", json!({})),
        ("a new tab", "file.new", json!({"units": "mm"})),
        (
            "the first tab again",
            "file.activate",
            json!({"document": 0}),
        ),
    ] {
        run(&mut session, id, params);
        let at_once = hovered(&mut shell, &mut session);
        let picture_at_once = shot(&mut shell, &mut session);
        window.frames(&mut shell, &mut session, 1);
        let after_one = hovered(&mut shell, &mut session);
        window.frames(&mut shell, &mut session, 3);
        let after_four = hovered(&mut shell, &mut session);
        let picture_after = shot(&mut shell, &mut session);
        let name = |v: &Value| {
            if v.is_null() {
                "null".to_string()
            } else {
                format!("{} {} {}", v["kind"], v["feature"], v["face"])
            }
        };
        report.push(format!(
            "after {what}: before a frame {}, after one {}, after four {}; the screenshot \
             before a frame {} the one after ({} amber pixels before a frame, {} after)",
            name(&at_once),
            name(&after_one),
            name(&after_four),
            if picture_at_once == picture_after {
                "is"
            } else {
                "is not"
            },
            amber_pixels(&picture_at_once),
            amber_pixels(&picture_after)
        ));
        if what == "edit.undo" || what == "a new tab" {
            // Nothing is there: nothing lies under the pointer.
            assert!(after_four.is_null(), "{what}: {after_four}");
        } else {
            assert_eq!(after_four["kind"], "face", "{what}: {after_four}");
        }
    }
    for line in report {
        eprintln!("REVIEW stale: {line}");
    }
}

/// Cubes 8 on a side with a bore, patterned `n` × `m`.
fn bored_cubes(n: u32, m: u32) -> Session {
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
        json!({"source": "Bored", "count": n, "step": [10, 0, 0], "name": "Row"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Row", "count": m, "step": [0, 0, 10], "name": "Wall"}),
    );
    s
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

/// What a pick under the pointer costs as the model grows (10 × 10, 20 ×
/// 20 and 40 × 20 bored cubes), and, on the largest, what the window's
/// frames cost while the mouse moves over the model resting 100 ms on each
/// of 12 places, against the same frames with the mouse away; the pictures
/// made meanwhile. The pick runs in the frame (the UI's thread): a frame
/// that picks costs the pick.
#[test]
fn the_cost_of_a_pick_as_the_model_grows_and_the_frames_it_stalls() {
    let mut last = None;
    for (n, m) in [(10, 10), (20, 20), (40, 20)] {
        let mut session = bored_cubes(n, m);
        let (mut shell, window) = in_a_window(&mut session);
        let got = call(&mut shell, &mut session, "ui.get", json!({}));
        let triangles = got["scene"]["triangles"].as_u64().unwrap_or(0);
        let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
        let [width, height] = shell.viewport_size();
        let mut costs = Vec::new();
        for j in 1..9 {
            for i in 1..9 {
                let x = f64::from(width) * f64::from(i) / 9.0 + 0.5;
                let y = f64::from(height) * f64::from(j) / 9.0 + 0.5;
                let started = Instant::now();
                let _ = shell
                    .pick(Some(scene.as_ref()), x, y, None, None, false)
                    .unwrap();
                costs.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        let faces = n * m * 7;
        eprintln!(
            "REVIEW pick cost: {faces} faces, {triangles} triangles: median {:.2} ms, most {:.2} ms",
            median(&mut costs.clone()),
            costs.iter().copied().fold(0.0, f64::max)
        );
        last = Some((session, shell, window));
    }
    let (mut session, mut shell, mut window) = last.unwrap();
    let rect = shell.viewport_rect().unwrap();
    let places: Vec<Pos2> = (0..12)
        .map(|k| {
            let t = (f64::from(k) + 0.5) / 12.0;
            egui::pos2(
                rect.left() + rect.width() * (0.2 + 0.6 * t as f32),
                rect.top() + rect.height() * (0.35 + 0.3 * ((k % 3) as f32) / 2.0),
            )
        })
        .collect();
    let walk = |shell: &mut Shell, session: &mut Session, window: &mut Window, mouse: bool| {
        let renders = shell.render_count();
        let mut times = Vec::new();
        let mut changes = 0;
        let mut was = Value::Null;
        for &at in &places {
            let events = if mouse {
                vec![Event::PointerMoved(at)]
            } else {
                vec![Event::PointerGone]
            };
            let started = Instant::now();
            window.frame(shell, session, events);
            times.push(started.elapsed().as_secs_f64() * 1000.0);
            for _ in 0..6 {
                let started = Instant::now();
                window.frame(shell, session, Vec::new());
                times.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            let now = hovered(shell, session);
            if now["serial"] != was["serial"] {
                changes += 1;
            }
            was = now;
        }
        let most = times.iter().copied().fold(0.0, f64::max);
        let total: f64 = times.iter().sum();
        (
            median(&mut times),
            most,
            total,
            shell.render_count() - renders,
            changes,
        )
    };
    let away = walk(&mut shell, &mut session, &mut window, false);
    let over = walk(&mut shell, &mut session, &mut window, true);
    let away_again = walk(&mut shell, &mut session, &mut window, false);
    for (what, (median, most, total, renders, changes)) in [
        ("the mouse away", away),
        ("the mouse over the model", over),
        ("the mouse away again", away_again),
    ] {
        eprintln!(
            "REVIEW stall: 5600 faces, 84 frames with {what}: median {median:.1} ms, most \
             {most:.1} ms, all {total:.0} ms; {renders} pictures made; hovered changed {changes} times"
        );
    }
}

/// The pick under the pointer on a model of about a million triangles
/// (20 × 20 bored cubes at the finest display quality, rule 12's realistic
/// size): what one costs. The hover runs it in the frame, on the UI's
/// thread, each time the pointer has rested 80 ms, whatever draws the
/// picture (the GPU path too); the frames of the CPU path also make the
/// picture again.
#[test]
fn a_pick_under_the_pointer_on_a_million_triangles() {
    let mut session = bored_cubes(20, 20);
    let (mut shell, mut window) = in_a_window(&mut session);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"quality": {"chordal": 1e-7, "maxTriangles": 4_000_000}}),
    );
    window.frames(&mut shell, &mut session, 2);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    let triangles = got["scene"]["triangles"].as_u64().unwrap_or(0);
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let [width, height] = shell.viewport_size();
    let mut costs = Vec::new();
    for j in 1..5 {
        for i in 1..5 {
            let x = f64::from(width) * f64::from(i) / 5.0 + 0.5;
            let y = f64::from(height) * f64::from(j) / 5.0 + 0.5;
            let started = Instant::now();
            let _ = shell
                .pick(Some(scene.as_ref()), x, y, None, None, false)
                .unwrap();
            costs.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let started = Instant::now();
    call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": f64::from(width) / 2.0, "y": f64::from(height) / 2.0, "hover": true}),
    );
    let placed = started.elapsed().as_secs_f64() * 1000.0;
    let rect = shell.viewport_rect().unwrap();
    let (mut slowest, mut picks) = (0.0_f64, 0);
    for k in 0..6 {
        let at = egui::pos2(
            rect.left() + rect.width() * (0.3 + 0.08 * k as f32),
            rect.center().y,
        );
        let before = hovered(&mut shell, &mut session)["serial"].clone();
        window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        for _ in 0..8 {
            let started = Instant::now();
            window.frame(&mut shell, &mut session, Vec::new());
            slowest = slowest.max(started.elapsed().as_secs_f64() * 1000.0);
        }
        if hovered(&mut shell, &mut session)["serial"] != before {
            picks += 1;
        }
    }
    eprintln!(
        "REVIEW million: {triangles} triangles: a pick costs {:.1} ms (median of 16, most \
         {:.1}); ui.pick with hover {placed:.1} ms; the mouse resting on 6 places: {picks} \
         picks, the slowest frame {slowest:.0} ms (the CPU path's picture made again with it)",
        median(&mut costs.clone()),
        costs.iter().copied().fold(0.0, f64::max)
    );
}

/// The mouse moved between the plate's top face and the background, six
/// times each way, resting 100 ms: how many pictures the CPU path makes
/// for a change of what lies under the pointer.
#[test]
fn pictures_made_per_hover_change_on_the_cpu_path() {
    let mut session = plate();
    let (mut shell, mut window) = in_a_window(&mut session);
    let rect = shell.viewport_rect().unwrap();
    let [x, y] = pixel_of(&shell, Vec3::new(30.0, 20.0, 8.0));
    let on = egui::pos2(rect.left() + x as f32, rect.top() + y as f32);
    let off = egui::pos2(rect.left() + 5.0, rect.top() + 5.0);
    window.frames(&mut shell, &mut session, 4);
    let renders = shell.render_count();
    let mut changes = 0;
    for _ in 0..6 {
        for at in [on, off] {
            window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
            window.frames(&mut shell, &mut session, 6);
            changes += 1;
        }
    }
    let made = shell.render_count() - renders;
    // The same walk off the model only: what moving the mouse costs
    // without a change under it.
    let quiet = shell.render_count();
    for _ in 0..6 {
        for at in [off, off + egui::vec2(3.0, 2.0)] {
            window.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
            window.frames(&mut shell, &mut session, 6);
        }
    }
    let quiet = shell.render_count() - quiet;
    eprintln!(
        "REVIEW pictures: {changes} changes of what lies under the pointer made {made} \
         pictures on the CPU path; the mouse moving off the model only made {quiet}"
    );
    assert_eq!(quiet, 0, "moving the mouse over nothing makes no picture");
}

/// A plate of 100 × 100 × 3 with 6 × 6 holes, meshed at the finest
/// display quality: its top face is bounded by tens of thousands of
/// segments. The pointer on it: the outline holds at most 20 000; what
/// placing it costs; the frame's lines of the hover and its cost, framed
/// and zoomed far in on the face's edge (nothing may grow with the zoom).
#[test]
fn the_outline_of_a_face_with_a_long_boundary() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"size": [100, 100, 3], "name": "Plate"}),
    );
    run(
        &mut session,
        "feature.cylinder",
        json!({"radius": 3, "height": 5, "position": [10, 10, -1], "name": "Hole"}),
    );
    run(
        &mut session,
        "feature.pattern",
        json!({"source": "Hole", "count": 6, "step": [16, 0, 0], "name": "Row"}),
    );
    run(
        &mut session,
        "feature.pattern",
        json!({"source": "Row", "count": 6, "step": [0, 16, 0], "name": "Holes"}),
    );
    run(
        &mut session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Holes", "name": "Perforated"}),
    );
    let (mut shell, mut window) = in_a_window(&mut session);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"quality": {"chordal": 1e-7, "maxTriangles": 8_000_000}}),
    );
    window.frames(&mut shell, &mut session, 4);
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    eprintln!("REVIEW boundary: the scene {}", got["scene"]);
    // Between the holes on the top.
    let [x, y] = pixel_of(&shell, Vec3::new(18.0, 18.0, 3.0));
    let started = Instant::now();
    let placed = call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x, "y": y, "hover": true}),
    );
    let place_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(placed["kind"], "face", "{placed}");
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let segments = shell.marks(Some(scene.as_ref())).hover_lines.len();
    let count = |shell: &Shell, kind: LineKind| {
        shell
            .overlay()
            .0
            .lines
            .iter()
            .filter(|l| l.kind == kind)
            .count()
    };
    let mut rows = Vec::new();
    for (what, distance) in [("framed", None), ("zoomed far in", Some(0.05))] {
        if let Some(distance) = distance {
            // On the edge of the first hole, from close by.
            call(
                &mut shell,
                &mut session,
                "ui.set",
                json!({"camera": {"target": [13.0, 10.0, 3.0], "distance": distance, "autoFit": false}}),
            );
            let [x, y] = pixel_of(&shell, Vec3::new(13.004, 10.0, 3.0));
            call(
                &mut shell,
                &mut session,
                "ui.pick",
                json!({"x": x, "y": y, "hover": true}),
            );
        }
        let started = Instant::now();
        window.frames(&mut shell, &mut session, 1);
        let first = started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        window.frames(&mut shell, &mut session, 3);
        let rest = started.elapsed().as_secs_f64() * 1000.0 / 3.0;
        let now = hovered(&mut shell, &mut session);
        rows.push(format!(
            "{what}: hovered {} {}; the overlay has {} hover lines and {} casings, {} edges; \
             the first frame {first:.0} ms, the next {rest:.0} ms each; {} segments in the marks",
            now["kind"],
            now["face"],
            count(&shell, LineKind::Hover),
            count(&shell, LineKind::HoverCasing),
            count(&shell, LineKind::Edge),
            shell.marks(Some(scene.as_ref())).hover_lines.len()
        ));
    }
    eprintln!(
        "REVIEW boundary: {segments} segments outlined, placed in {place_ms:.0} ms; {}",
        rows.join("; ")
    );
    assert!(segments <= 20_000, "{segments}");
}
