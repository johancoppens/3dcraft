//! A one-body document in the window, cut open and not, byte for byte
//! (the review of `check/hatch-body`, by `review-ux-render`): a drilled
//! plate in a window of 1000 × 700, without a section, cut across z and
//! across x, opaque and see-through: the `ui.screenshot` picture, the
//! lines over the CPU picture and the GPU path's segments, hashed. The
//! same file runs on `check/ux-round-2` and on the hatch's trial tree; the
//! hashes it prints (`BYTES` lines) must be the same on both.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, Pos2};
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

fn report(shell: &mut Shell, session: &mut Session, state: &str) {
    let shot = call(shell, session, "ui.screenshot", json!({"scale": 1.0}));
    let picture = fnv(shot["base64"].as_str().unwrap().as_bytes());
    let (overlay, size) = shell.overlay();
    let lines = fnv(format!("{:?}", overlay.lines).as_bytes());
    let n_lines = overlay.lines.len();
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
    println!(
        "BYTES ui {state} screenshot {picture:016x} overlay {size:?} lines {lines:016x} \
         ({n_lines}) segments {:016x} ({})",
        fnv(format!("{segments:?}").as_bytes()),
        segments.len()
    );
}

#[test]
fn a_one_body_window_cut_open_byte_for_byte() {
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
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"camera": {"autoFit": false}}),
    );
    window.frames(&mut shell, &mut session, 4);
    report(&mut shell, &mut session, "none");
    for (state, section, transparency) in [
        ("z", json!({"on": true, "plane": "xy", "offset": 4}), 0.0),
        ("x", json!({"on": true, "plane": "yz", "offset": 20}), 0.0),
        (
            "z-seethrough",
            json!({"on": true, "plane": "xy", "offset": 4}),
            0.5,
        ),
    ] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"section": section, "transparency": transparency}),
        );
        window.frames(&mut shell, &mut session, 4);
        report(&mut shell, &mut session, state);
    }
}
