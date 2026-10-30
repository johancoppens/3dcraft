//! Pictures of the whole window for a UX audit (docs/development.md ›
//! *The view cube and the controls over the viewport*): the start
//! screen, a part, an assembly playing, the studio look with its panel
//! (and again after the panel was folded and unfolded), an analysis, a
//! section, the drawer of the narrow layout, the Parameters tab and an
//! error in the status bar, each at 1600 × 1000 and 1024 × 700 points and
//! on a phone (390 × 844) and a tablet (820 × 1180). Painted by egui's own wgpu renderer
//! into a texture and printed as `EVIDENCE <name> <k> <base64>` pieces of
//! a PNG, as `calm_evidence` does. Without a graphics adapter it says so
//! and passes: it only shows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::CommandReport;
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::direct::headless_render_state;
use egui::{Event, PointerButton, Pos2, pos2, vec2};
use egui_wgpu::wgpu;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A four-bar of four instances of one bar part, its crank driven by `t`.
fn four_bar(s: &mut Session) {
    run(s, "file.new", json!({}));
    run(s, "variable.set", json!({ "name": "len", "expr": "24 mm" }));
    run(
        s,
        "variable.publish",
        json!({ "name": "len", "min": "10 mm", "max": "500 mm" }),
    );
    run(
        s,
        "feature.block",
        json!({ "name": "Bar", "size": ["len", 4, 2] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End1", "through": [[2, 2, 0], [2, 2, 1]] }),
    );
    run(
        s,
        "datum.axis",
        json!({ "name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]] }),
    );
    run(s, "file.save", json!({ "path": "bar.3dcraft" }));
    run(s, "assembly.new", json!({}));
    for (name, len) in [
        ("Ground", "64 mm"),
        ("Crank", "24 mm"),
        ("Coupler", "64 mm"),
        ("Rocker", "24 mm"),
    ] {
        run(
            s,
            "instance.add",
            json!({ "source": "bar.3dcraft", "name": name, "body": "Bar",
                    "params": { "len": len } }),
        );
    }
    for (name, a, b) in [
        ("Base", "world/z", "Ground/End1"),
        ("Drive", "Ground/End1", "Crank/End1"),
        ("Elbow", "Crank/End2", "Coupler/End1"),
        ("Knee", "Coupler/End2", "Rocker/End2"),
        ("Hip", "Ground/End2", "Rocker/End1"),
    ] {
        let kind = if name == "Base" { "fixed" } else { "revolute" };
        run(
            s,
            "mate.add",
            json!({ "kind": kind, "a": a, "b": b, "name": name, "flip": true }),
        );
    }
    run(s, "variable.set", json!({ "name": "t", "expr": "0" }));
    run(
        s,
        "variable.set",
        json!({ "name": "Drive_angle", "expr": "360 deg * t" }),
    );
}

/// Paints `jobs` into a texture of the window's size and reads it back as
/// a PNG.
fn picture(
    state: &egui_wgpu::RenderState,
    jobs: &[egui::ClippedPrimitive],
    size: [u32; 2],
) -> Vec<u8> {
    let mut renderer = state.renderer.write();
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: size,
        pixels_per_point: 1.0,
    };
    let target = state.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: state.target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = state
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let mut commands =
        renderer.update_buffers(&state.device, &state.queue, &mut encoder, jobs, &screen);
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        renderer.render(&mut pass, jobs, &screen);
    }
    let row = size[0] * 4;
    let padded = row.div_ceil(256) * 256;
    let buffer = state.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded * size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size[1]),
            },
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    commands.push(encoder.finish());
    state.queue.submit(commands);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    state
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let data = slice.get_mapped_range().unwrap();
    let mut rgb = Vec::with_capacity((size[0] * size[1] * 3) as usize);
    let bgra = matches!(
        state.target_format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    for y in 0..size[1] {
        let start = (y * padded) as usize;
        for pixel in data[start..start + row as usize].chunks(4) {
            if bgra {
                rgb.extend([pixel[2], pixel[1], pixel[0]]);
            } else {
                rgb.extend(&pixel[..3]);
            }
        }
    }
    craft3d_render::encode_png(size[0], size[1], &rgb).unwrap()
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * k)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Prints `png` as `EVIDENCE <name> <k> <base64 piece>` lines of at most
/// 2000 characters (a CI log cuts longer lines), `k` counting from 0.
fn show(name: &str, png: &[u8]) {
    let text = base64(png);
    for (k, piece) in text.as_bytes().chunks(2000).enumerate() {
        println!("EVIDENCE {name} {k} {}", String::from_utf8_lossy(piece));
    }
}

/// A part: a plate with a boss, two published parameters.
fn part(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(
        s,
        "variable.set",
        json!({ "name": "width", "expr": "60 mm" }),
    );
    run(
        s,
        "variable.publish",
        json!({ "name": "width", "min": "20 mm", "max": "200 mm" }),
    );
    run(
        s,
        "variable.set",
        json!({ "name": "height", "expr": "12 mm" }),
    );
    run(
        s,
        "variable.publish",
        json!({ "name": "height", "min": "4 mm", "max": "60 mm" }),
    );
    run(
        s,
        "feature.block",
        json!({ "name": "Plate", "size": ["width", 40, "height"] }),
    );
}

fn workspace(dir: &Path) -> Workspace {
    Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    }
}

/// One window of `size` points, painted on `state`'s device.
struct Window<'a> {
    state: &'a egui_wgpu::RenderState,
    ctx: egui::Context,
    time: f64,
    size: [u32; 2],
    jobs: Vec<egui::ClippedPrimitive>,
}

impl Window<'_> {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 30.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                vec2(self.size[0] as f32, self.size[1] as f32),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        let mut renderer = self.state.renderer.write();
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(&self.state.device, &self.state.queue, *id, delta);
            }
        }
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        output.textures_delta.clear();
        self.jobs = self.ctx.tessellate(output.shapes, output.pixels_per_point);
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
    }

    fn show(&self, name: &str) {
        let png = picture(self.state, &self.jobs, self.size);
        show(&format!("{name}-{}x{}", self.size[0], self.size[1]), &png);
    }
}

fn set(shell: &mut Shell, session: &mut Session, params: Value) {
    shell
        .handle_method(session, "ui.set", Some(&params))
        .unwrap()
        .unwrap_or_else(|e| panic!("ui.set {params}: {e}"));
}

fn hit(shell: &Shell, name: &str) -> Option<egui::Rect> {
    shell
        .look_hits()
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| *r)
}

// Pictures only, 48 PNGs in the log: run on demand with
// `cargo test -p craft3d-ui-egui --test ui_audit_evidence -- --ignored --nocapture`.
#[test]
#[ignore = "prints window pictures for a person to look at; run with --ignored"]
fn the_window_in_pictures_for_a_ux_audit() {
    let state = match headless_render_state() {
        Ok(state) => state,
        Err(e) => {
            println!("no graphics adapter, no pictures: {e}");
            return;
        }
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ui-audit-evidence");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for size in [[1600, 1000], [1024, 700], [390, 844], [820, 1180]] {
        let window = |state| Window {
            state,
            ctx: egui::Context::default(),
            time: 0.0,
            size,
            jobs: Vec::new(),
        };
        // The start screen: no document.
        {
            let mut session = Session::new(workspace(&dir));
            let mut shell = Shell::new();
            shell.attach_gpu(&state).unwrap();
            let mut w = window(&state);
            w.frames(&mut shell, &mut session, 6);
            w.show("start");
        }
        // A part, then the same part in each state.
        type Setup = fn(&mut Shell, &mut Session);
        let states: [(&str, Setup); 7] = [
            ("part", |_, _| {}),
            ("studio", |shell, session| {
                set(shell, session, json!({"look": {"mode": "studio"}}));
            }),
            ("analysis", |shell, session| {
                set(shell, session, json!({"analysis": {"mode": "zebra"}}));
            }),
            ("section", |shell, session| {
                set(shell, session, json!({"section": {"on": true}}));
            }),
            ("drawer", |shell, session| {
                set(shell, session, json!({"panels": {"drawer": true}}));
            }),
            ("parameters", |shell, session| {
                set(shell, session, json!({"parameters": {"open": true}}));
            }),
            ("error", |shell, _| {
                shell.last_command = Some(CommandReport {
                    label: "feature.extrude".to_string(),
                    ok: false,
                    text: "the profile is not closed: sketch Sketch1 has an open end at (12, 4) mm"
                        .to_string(),
                });
            }),
        ];
        for (name, setup) in states {
            let mut session = Session::new(workspace(&dir));
            part(&mut session);
            let mut shell = Shell::new();
            shell.attach_gpu(&state).unwrap();
            let mut w = window(&state);
            w.frames(&mut shell, &mut session, 3);
            setup(&mut shell, &mut session);
            w.frames(&mut shell, &mut session, 12);
            w.show(name);
            // The studio panel folded and unfolded again.
            if name == "studio"
                && let Some(fold) = hit(&shell, "collapse")
            {
                w.click(&mut shell, &mut session, fold.center());
                w.frames(&mut shell, &mut session, 3);
                w.show("studio-folded");
                if let Some(fold) = hit(&shell, "collapse") {
                    w.click(&mut shell, &mut session, fold.center());
                }
                w.frames(&mut shell, &mut session, 3);
                w.show("studio-unfolded");
            }
            // The pointer over the Studio chip.
            if name == "part"
                && let Some(chip) = shell.controls_layout().and_then(|l| l.studio)
            {
                w.frame(
                    &mut shell,
                    &mut session,
                    vec![Event::PointerMoved(pos2(chip.center().x, chip.center().y))],
                );
                w.frames(&mut shell, &mut session, 2);
                w.show("part-hover");
            }
        }
        // An assembly playing its motion.
        {
            let mut session = Session::new(workspace(&dir));
            four_bar(&mut session);
            let mut shell = Shell::new();
            shell.attach_gpu(&state).unwrap();
            let mut w = window(&state);
            w.frames(&mut shell, &mut session, 3);
            set(
                &mut shell,
                &mut session,
                json!({ "timeline": { "motion": true, "playing": true, "loop": true } }),
            );
            w.frames(&mut shell, &mut session, 40);
            w.show("assembly");
        }
    }
}
