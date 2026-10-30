//! Pictures of the whole window for whoever looks (docs/design/engine.md
//! › *A calm work indicator and status bar*): a four-bar assembly playing
//! its motion on the GPU's direct path (its parts moved by poses), with
//! the status bar's steady `Playing  t = …`, and the same window while a
//! long job runs, with the small work indicator in the status bar.
//! Painted by egui's own wgpu renderer into a texture, as the example
//! `window_picture` does, and printed as `EVIDENCE <name> <k> <base64>`
//! pieces of a PNG.
//! Without a graphics adapter it says so and passes: it only shows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::{Shell, StatusSlot, Work};
use egui::{Pos2, vec2};
use egui_wgpu::wgpu;
use serde_json::{Value, json};
use std::path::PathBuf;

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

const SIZE: [u32; 2] = [1100, 700];

/// Paints `jobs` into a texture of the window's size and reads it back as
/// a PNG.
fn picture(state: &egui_wgpu::RenderState, jobs: &[egui::ClippedPrimitive]) -> Vec<u8> {
    let mut renderer = state.renderer.write();
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: SIZE,
        pixels_per_point: 1.0,
    };
    let target = state.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
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
    let row = SIZE[0] * 4;
    let padded = row.div_ceil(256) * 256;
    let buffer = state.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded * SIZE[1]),
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
                rows_per_image: Some(SIZE[1]),
            },
        },
        wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
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
    let mut rgb = Vec::with_capacity((SIZE[0] * SIZE[1] * 3) as usize);
    let bgra = matches!(
        state.target_format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    for y in 0..SIZE[1] {
        let start = (y * padded) as usize;
        for pixel in data[start..start + row as usize].chunks(4) {
            if bgra {
                rgb.extend([pixel[2], pixel[1], pixel[0]]);
            } else {
                rgb.extend(&pixel[..3]);
            }
        }
    }
    craft3d_render::encode_png(SIZE[0], SIZE[1], &rgb).unwrap()
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

#[test]
fn the_calm_status_bar_and_work_indicator_in_pictures() {
    let state = match headless_render_state() {
        Ok(state) => state,
        Err(e) => {
            println!("no graphics adapter, no pictures: {e}");
            return;
        }
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("calm-evidence");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    });
    four_bar(&mut session);
    let mut shell = Shell::new();
    shell.attach_gpu(&state).unwrap();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    let mut frame = |shell: &mut Shell, session: &mut Session, away: bool| {
        time += 1.0 / 30.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                vec2(SIZE[0] as f32, SIZE[1] as f32),
            )),
            time: Some(time),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            if away {
                shell.ui_away(ui);
            } else {
                shell.ui(ui, session);
            }
        });
        let mut renderer = state.renderer.write();
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(&state.device, &state.queue, *id, delta);
            }
        }
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        output.textures_delta.clear();
        ctx.tessellate(output.shapes, output.pixels_per_point)
    };
    for _ in 0..3 {
        frame(&mut shell, &mut session, false);
    }
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({ "timeline": { "motion": true, "playing": true, "loop": true } })),
        )
        .unwrap()
        .unwrap();
    let mut jobs = Vec::new();
    for _ in 0..40 {
        jobs = frame(&mut shell, &mut session, false);
    }
    assert_eq!(shell.status_shown(StatusSlot::Command), Some("Playing"));
    let playing = picture(&state, &jobs);
    show("calm-playing", &playing);
    shell
        .handle_method(
            &mut session,
            "ui.set",
            Some(&json!({ "timeline": { "playing": false } })),
        )
        .unwrap()
        .unwrap();
    frame(&mut shell, &mut session, false);
    shell.set_work(Some(Work {
        label: "file.open".to_string(),
        subject: Some("strandbeest-walker".to_string()),
        phase: "regenerating".to_string(),
        feature: Some("F17".to_string()),
        name: Some("Leg 3".to_string()),
        step: 17,
        steps: 20,
        seconds: 0.5,
    }));
    for _ in 0..15 {
        jobs = frame(&mut shell, &mut session, true);
    }
    assert!(shell.work_shown().is_some());
    let working = picture(&state, &jobs);
    show("calm-working", &working);
}
