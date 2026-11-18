//! The window without a section, byte for byte (the review of the UX round
//! of 2026-11-18, by `review-ux-render`): a drilled plate in a window of
//! 1000 × 700, plain, with a face selected, and (where `ui.pick` takes
//! `hover`) with the pointer placed on the background and on a face: the
//! `ui.screenshot` picture, the lines and dots over the CPU picture
//! (`Shell::overlay`) and the GPU path's segments (`segments_marked`),
//! hashed. The same file runs on main and on the round's tree; the hashes
//! it prints (`BYTES` lines) must be the same on both where both draw the
//! same thing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_math::Vec3;
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, Pos2};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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

struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn frames(&mut self, shell: &mut Shell, session: &mut Session, count: usize) {
        for _ in 0..count {
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size)),
                time: Some(self.time),
                events: Vec::<Event>::new(),
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
            output.textures_delta.clear();
        }
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

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

fn report(shell: &mut Shell, session: &mut Session, state: &str) {
    let shot = call(shell, session, "ui.screenshot", json!({"scale": 1.0}));
    let picture = fnv(shot["base64"].as_str().unwrap().as_bytes());
    let (overlay, size) = shell.overlay();
    let lines = fnv(format!("{:?}", overlay.lines).as_bytes());
    let points = fnv(format!("{:?}", overlay.points).as_bytes());
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for line in &overlay.lines {
        *kinds.entry(format!("{:?}", line.kind)).or_insert(0) += 1;
    }
    let (n_lines, n_points) = (overlay.lines.len(), overlay.points.len());
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let marks = shell.marks(Some(scene.as_ref()));
    let segments = craft3d_ui_egui::direct::segments_marked(
        Some(scene.as_ref()),
        shell.state.selected_feature.as_deref(),
        &shell.tokens,
        1.0,
        &marks,
        false,
    );
    let segment_hash = fnv(format!("{segments:?}").as_bytes());
    println!(
        "BYTES ui {state} screenshot {picture:016x} overlay {size:?} lines {lines:016x} ({n_lines}) \
         points {points:016x} ({n_points}) segments {segment_hash:016x} ({})",
        segments.len()
    );
    println!("BYTES ui {state} kinds {kinds:?}");
}

#[test]
fn the_window_without_a_section_byte_for_byte() {
    let mut session = Session::new(Workspace::none());
    run(&mut session, "file.new", json!({"units": "mm"}));
    run(
        &mut session,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 8]}),
    );
    run(
        &mut session,
        "feature.cylinder",
        json!({"radius": 6, "height": 12, "position": [20, 20, -2], "name": "Bore"}),
    );
    run(
        &mut session,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Bore", "name": "Drilled"}),
    );
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window {
        ctx: egui::Context::default(),
        size: egui::vec2(1000.0, 700.0),
        time: 0.0,
    };
    window.frames(&mut shell, &mut session, 3);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    window.frames(&mut shell, &mut session, 4);
    report(&mut shell, &mut session, "plain");

    let [bx, by] = [1.5, 1.5];
    match answer(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": bx, "y": by, "hover": true}),
    ) {
        Ok(_) => {
            window.frames(&mut shell, &mut session, 4);
            report(&mut shell, &mut session, "hover-on-nothing");
        }
        Err(e) => println!("BYTES ui hover-on-nothing none here: {e}"),
    }

    let [x, y] = pixel_of(&shell, Vec3::new(45.0, 20.0, 8.0));
    call(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x, "y": y, "select": true}),
    );
    window.frames(&mut shell, &mut session, 4);
    report(&mut shell, &mut session, "selected");

    match answer(
        &mut shell,
        &mut session,
        "ui.pick",
        json!({"x": x, "y": y, "hover": true}),
    ) {
        Ok(_) => {
            window.frames(&mut shell, &mut session, 4);
            report(&mut shell, &mut session, "selected-and-hovered");
            call(
                &mut shell,
                &mut session,
                "ui.pick",
                json!({"x": bx, "y": by, "hover": true}),
            );
            window.frames(&mut shell, &mut session, 4);
            report(&mut shell, &mut session, "selected-hover-gone");
        }
        Err(e) => println!("BYTES ui selected-and-hovered none here: {e}"),
    }
}
