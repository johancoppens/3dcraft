//! Pictures of the section's handle in the whole window, for a person to
//! look at (AGENTS.md rule 10): at rest, under the pointer, dragged onto
//! a hole's axis, seen side on and end on, typed into, in the dark
//! appearance, on a phone and with touch sizes, on several documents. The
//! viewport by the CPU renderer, the window painted by egui's own wgpu
//! renderer into a texture and read back; written as PNG files to
//! `CRAFT3D_PICS` (else `target/evidence/section-handle`). Without a
//! graphics adapter it says so and passes: it only shows.
//!
//! `cargo test -p craft3d-ui-egui --test section_handle_evidence -- --ignored`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::{RendererChoice, Shell};
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_wgpu::wgpu;
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn set(shell: &mut Shell, session: &mut Session, params: Value) {
    shell
        .handle_method(session, "ui.set", Some(&params))
        .unwrap()
        .unwrap_or_else(|e| panic!("ui.set {params}: {e}"));
}

/// A new document in millimetres built by the script `name` of
/// `docs/examples`.
fn example(name: &str) -> Session {
    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples");
    let text = std::fs::read_to_string(docs.join(format!("{name}.json"))).unwrap();
    let script: Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for step in script.as_array().unwrap() {
        let params = step.get(1).cloned().unwrap_or_else(|| json!({}));
        run(&mut s, step[0].as_str().unwrap(), params);
    }
    s
}

/// The block of the handle's tests: 60 × 40 × 20, a hole along y through
/// (20, ·, 10), one along z through (45, 20, ·).
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

/// Paints `jobs` into a texture of `size` pixels and reads it back as RGB.
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
    rgb
}

struct Window<'a> {
    state: &'a egui_wgpu::RenderState,
    ctx: egui::Context,
    time: f64,
    size: [u32; 2],
    jobs: Vec<egui::ClippedPrimitive>,
    dir: PathBuf,
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

    fn press(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2, pressed: bool) {
        self.frame(
            shell,
            session,
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }],
        );
    }

    fn save(&self, name: &str) {
        let rgb = picture(self.state, &self.jobs, self.size);
        let png = craft3d_render::encode_png(self.size[0], self.size[1], &rgb).unwrap();
        std::fs::write(self.dir.join(format!("{name}.png")), png).unwrap();
    }
}

#[test]
#[ignore = "writes pictures for a person to look at; run with --ignored"]
fn pictures_of_the_section_handle() {
    let state = match headless_render_state() {
        Ok(state) => state,
        Err(e) => {
            println!("no graphics adapter, no pictures: {e}");
            return;
        }
    };
    let dir = std::env::var("CRAFT3D_PICS").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/section-handle"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let window = |size: [u32; 2]| Window {
        state: &state,
        ctx: egui::Context::default(),
        time: 0.0,
        size,
        jobs: Vec::new(),
        dir: dir.clone(),
    };
    let open = |session: &mut Session, w: &mut Window, ui_set: Value| -> Shell {
        let mut shell = Shell::with_renderer(RendererChoice::Cpu);
        w.frames(&mut shell, session, 3);
        set(&mut shell, session, ui_set);
        w.frames(&mut shell, session, 8);
        shell
    };
    let pointer_on = |shell: &Shell, part: &str| -> Pos2 {
        let place = shell.section_handle().expect("the handle");
        match part {
            "arrow" => place
                .frame
                .arrow
                .map_or(place.frame.at, |[a, b]| a.lerp(b, 0.55)),
            "number" => place.number.center(),
            _ => place.frame.at + vec2(-300.0, 240.0),
        }
    };

    // The drilled block from the iso corner, across X through the hole
    // along y: at rest, under the pointer, dragged onto the other hole.
    for (name, part) in [("drilled-iso-rest", "away"), ("drilled-iso-near", "arrow")] {
        let mut session = drilled();
        let mut w = window([1400, 900]);
        let mut shell = open(
            &mut session,
            &mut w,
            json!({"section": {"on": true, "plane": "yz", "offset": 20}}),
        );
        let at = pointer_on(&shell, part);
        w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        w.frames(&mut shell, &mut session, 2);
        w.save(name);
    }
    {
        let mut session = drilled();
        let mut w = window([1400, 900]);
        let mut shell = open(
            &mut session,
            &mut w,
            json!({"section": {"on": true, "plane": "yz", "offset": 20}}),
        );
        let from = pointer_on(&shell, "arrow");
        let place = shell.section_handle().unwrap().clone();
        let [a, b] = place.frame.arrow.unwrap();
        let way = (b - a).normalized();
        // Toward x = 45, two points at a time, until the snap takes it
        // onto the axis of the hole along z.
        w.frame(&mut shell, &mut session, vec![Event::PointerMoved(from)]);
        w.press(&mut shell, &mut session, from, true);
        let mut points = 0.0;
        while points < 600.0 && shell.state.section.offset != 45.0 {
            points += 2.0;
            let at = from + way * points;
            w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        }
        w.frames(&mut shell, &mut session, 1);
        println!(
            "dragged to {} (snapped {:?})",
            shell.state.section.offset,
            shell.section_snapped()
        );
        w.save("drilled-iso-drag-snapped");
        let to = from + way * points;
        w.press(&mut shell, &mut session, to, false);
        w.frame(&mut shell, &mut session, vec![Event::PointerGone]);
        w.frames(&mut shell, &mut session, 2);
        w.save("drilled-iso-after-drag");
    }
    // From the front, orthographic, across Z; from the top, end on; the
    // number typed into; the dark appearance.
    for (name, view, plane, offset, typing, dark) in [
        ("drilled-front-z", "front", "xy", 7.0, false, false),
        ("drilled-top-ring", "top", "xy", 7.0, false, false),
        ("drilled-right-y", "right", "zx", 20.0, false, false),
        ("drilled-typing", "iso", "yz", 20.0, true, false),
        ("drilled-dark", "iso", "zx", 20.0, false, true),
    ] {
        let mut session = drilled();
        let mut w = window([1400, 900]);
        let projection = if view == "iso" {
            "perspective"
        } else {
            "orthographic"
        };
        let mut shell = open(
            &mut session,
            &mut w,
            json!({"view": view, "camera": {"projection": projection},
                   "appearance": if dark { "dark" } else { "light" },
                   "section": {"on": true, "plane": plane, "offset": offset}}),
        );
        if typing {
            let at = pointer_on(&shell, "number");
            w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
            w.press(&mut shell, &mut session, at, true);
            w.press(&mut shell, &mut session, at, false);
            w.frame(
                &mut shell,
                &mut session,
                vec![Event::Text("width / 3".into())],
            );
            w.frames(&mut shell, &mut session, 2);
        } else {
            let at = pointer_on(&shell, "arrow");
            w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
            w.frames(&mut shell, &mut session, 2);
        }
        w.save(name);
    }
    // Other documents, the pointer near the handle.
    // The guide's picture: the flange of the lessons cut across X through
    // the axes of two bolt holes (x = 30, a place the plane snaps to).
    for (name, script, plane, offset, view) in [
        ("flange-x", "flange", "yz", Some(30.0), "iso"),
        ("gear-z", "gear", "xy", None, "iso"),
        ("case-y", "case", "zx", None, "iso"),
        ("bracket-x", "bracket", "yz", None, "iso"),
    ] {
        let mut session = example(script);
        let mut w = window([1400, 900]);
        let mut section = json!({"on": true, "plane": plane});
        if let Some(offset) = offset {
            section["offset"] = json!(offset);
        }
        let mut shell = open(
            &mut session,
            &mut w,
            json!({"view": view, "section": section}),
        );
        let at = pointer_on(&shell, "arrow");
        w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        w.frames(&mut shell, &mut session, 2);
        w.save(name);
    }
    // A slanted face (the drilled block's top edge chamfered by 8)
    // selected: the fourth dot; laid on it; dragged into the body.
    for (view, name) in [("iso", "face"), ("front", "face-front")] {
        let mut session = drilled();
        run(
            &mut session,
            "feature.chamfer",
            json!({"edges": [["F1:face(1)", "F1:face(5)"]], "distance": 8, "name": "Bevel"}),
        );
        run(
            &mut session,
            "select.query",
            json!({"kind": "face", "where": {"normal": {"along": [1, 0, 1]}}, "then": "replace"}),
        );
        let mut w = window([1400, 900]);
        let mut shell = open(
            &mut session,
            &mut w,
            json!({"view": view, "section": {"on": true, "plane": "yz", "offset": 20}}),
        );
        let dot = shell.section_handle().unwrap().dots[3];
        w.frame(&mut shell, &mut session, vec![Event::PointerMoved(dot)]);
        w.frames(&mut shell, &mut session, 2);
        w.save(&format!("{name}-selected"));
        w.press(&mut shell, &mut session, dot, true);
        w.press(&mut shell, &mut session, dot, false);
        w.frames(&mut shell, &mut session, 4);
        let at = pointer_on(&shell, "arrow");
        w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        w.frames(&mut shell, &mut session, 2);
        w.save(&format!("{name}-laid"));
        // Into the body by 6 mm: the arrow's way back, in steps.
        let place = shell.section_handle().unwrap().clone();
        if let Some([a, b]) = place.frame.arrow {
            let way = (b - a).normalized();
            let points = (6.0 / place.frame.per_point) as f32;
            w.press(&mut shell, &mut session, at, true);
            for k in 1..=10 {
                let to = at - way * (points * k as f32 / 10.0);
                w.frame(&mut shell, &mut session, vec![Event::PointerMoved(to)]);
            }
            w.press(&mut shell, &mut session, at - way * points, false);
            w.frames(&mut shell, &mut session, 3);
        }
        println!("{name}: offset {}", shell.state.section.offset);
        w.save(&format!("{name}-cut"));
    }
    // A phone: no handle; a desk window touched: a finger's handle.
    {
        let mut session = drilled();
        let mut w = window([390, 844]);
        let mut shell = open(
            &mut session,
            &mut w,
            json!({"section": {"on": true, "plane": "yz", "offset": 20}}),
        );
        w.frames(&mut shell, &mut session, 2);
        w.save("phone");
    }
    {
        let mut session = drilled();
        let mut w = window([1400, 900]);
        let mut shell = open(
            &mut session,
            &mut w,
            json!({"section": {"on": true, "plane": "yz", "offset": 20}}),
        );
        let finger = Pos2::new(700.0, 120.0);
        for phase in [egui::TouchPhase::Start, egui::TouchPhase::End] {
            w.frame(
                &mut shell,
                &mut session,
                vec![Event::Touch {
                    device_id: egui::TouchDeviceId(1),
                    id: egui::TouchId(1),
                    phase,
                    pos: finger,
                    force: None,
                }],
            );
        }
        w.frames(&mut shell, &mut session, 3);
        let at = pointer_on(&shell, "arrow");
        w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
        w.frames(&mut shell, &mut session, 2);
        w.save("touch-sizes");
    }
    println!("pictures in {}", dir.display());
}
