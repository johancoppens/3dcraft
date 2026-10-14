//! A picture of the whole window while work runs on its engine thread:
//! the shell's panels, the viewport drawn on the GPU (the direct path, as
//! in the window) and the work in progress over it, painted by egui's own
//! wgpu renderer into a texture that is read back. What a screenshot of
//! the window shows, for a machine where screenshots of other programs
//! are not allowed (GNOME on Wayland).
//!
//! ```sh
//! cargo run --release -p craft3d-ui-egui --example window_picture -- \
//!     model.3dcraft out.png
//! ```
//!
//! The document is opened and shown as the window shows it; then the
//! shell is drawn as it is while a command runs elsewhere
//! (`Shell::ui_away`), with a step of regeneration as the work.

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::{Shell, Work};
use egui::{Pos2, vec2};
use egui_wgpu::wgpu;
use serde_json::json;

const SIZE: [u32; 2] = [1280, 800];

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let file = args
        .next()
        .ok_or("usage: window_picture FILE.3dcraft OUT.png")?;
    let out = args
        .next()
        .ok_or("usage: window_picture FILE.3dcraft OUT.png")?;
    let state = headless_render_state()?;
    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute("file.open", &json!({ "path": file }))
        .map_err(|e| e.to_string())?;
    let mut shell = Shell::new();
    shell.attach_gpu(&state)?;
    let ctx = egui::Context::default();
    let input = |time: f64| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            vec2(SIZE[0] as f32, SIZE[1] as f32),
        )),
        time: Some(time),
        ..Default::default()
    };
    // The renderer is locked only to paint: the shell's direct path
    // registers its texture with it during a frame.
    let paint = |output: egui::FullOutput, renderer: &mut egui_wgpu::Renderer| {
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(&state.device, &state.queue, *id, delta);
            }
        }
        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        jobs
    };
    // The window as it shows the document, then as it is while work runs.
    let mut jobs = Vec::new();
    for (frame, time) in [0.0, 0.1, 0.2, 0.3].into_iter().enumerate() {
        if frame == 2 {
            shell.set_work(Some(Work {
                label: "file.open".to_string(),
                phase: "regenerating".to_string(),
                feature: Some("F17".to_string()),
                name: Some("J2".to_string()),
                step: 17,
                steps: 20,
                seconds: 3.4,
            }));
        }
        let output = ctx.run_ui(input(time), |ui| {
            if frame < 2 {
                shell.ui(ui, &mut session);
            } else {
                shell.ui_away(ui);
            }
        });
        jobs = paint(output, &mut state.renderer.write());
    }
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
        renderer.update_buffers(&state.device, &state.queue, &mut encoder, &jobs, &screen);
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
        renderer.render(&mut pass, &jobs, &screen);
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
        .map_err(|e| e.to_string())?;
    let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
    let mut rgb = Vec::with_capacity((SIZE[0] * SIZE[1] * 3) as usize);
    for y in 0..SIZE[1] {
        let start = (y * padded) as usize;
        let line = data
            .get(start..start + row as usize)
            .ok_or("the read-back picture is shorter than asked for")?;
        for pixel in line.chunks(4) {
            rgb.extend(pixel.iter().take(3));
        }
    }
    let png = craft3d_render::encode_png(SIZE[0], SIZE[1], &rgb).map_err(|e| e.to_string())?;
    std::fs::write(&out, png).map_err(|e| e.to_string())?;
    println!("wrote {out} ({:?})", state.target_format);
    Ok(())
}
