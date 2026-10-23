//! A picture of the whole window: the shell's panels, the row of tabs
//! of the open documents, the viewport drawn on the GPU (the direct path,
//! as in the window) and, unless `--idle`, the work in progress over it,
//! painted by egui's own wgpu renderer into a texture that is read back.
//! What a screenshot of the window shows, for a machine where screenshots
//! of other programs are not allowed (GNOME on Wayland).
//!
//! ```sh
//! cargo run --release -p craft3d-ui-egui --example window_picture -- \
//!     [--idle] [--first] [--active N] [--edit N] [--ask N] [--set JSON] [--size WxH] [--scale S] [--hover X,Y,Z] [--point NAME] [--type NAME=TEXT] [--click X,Y] [--dialog export:NAME|error:TEXT] [--recent FILE] model.3dcraft [more.3dcraft | untitled | none ...] out.png
//! ```
//!
//! The documents are opened in order (`untitled` is a new document with a
//! block in it, never saved; `none` opens nothing, for the start screen)
//! and the last one, or document `N` of
//! `--active`, is shown as the window shows it; `--edit N` changes
//! document `N` after it was opened, so its tab is marked unsaved, and
//! `--ask N` closes document `N` from its tab, which shows the question
//! before closing when it has unsaved changes. `--set JSON` patches the
//! window's state after the first frame, as `ui.set` does (a camera, a
//! display quality: `{"quality": {"chordal": 1e-4}}`). `--first` shows
//! the quick first picture, as the window does while the display scene is
//! refined beside it. `--size` is the window in points (1280x800 by
//! default; with `--scale S` the picture has S pixels per point, so it is
//! S times as many pixels each way); `--hover X,Y,Z` puts the pointer on the middle of the view
//! cube's facet for that side (`1,-1,0`: the edge between front and
//! right), as it is drawn under the pointer; `--point NAME` puts it on the
//! middle of that control (`see-through`, `offset`, `fit`, ...: the names
//! of `ControlsLayout::controls`), and `--type NAME=TEXT` double-clicks
//! that number field and types TEXT into it, without Enter; `--click X,Y`
//! clicks the window at that point (in points), and leaves the pointer
//! there: `--click 20,10` opens the File menu. `--recent FILE` starts
//! with the recent files kept in FILE (`{"recent": [path, ...]}`).
//! `--dialog export:NAME` opens the export dialog for a file of that name
//! (with what is selected, after `--set {"selectedFeature": …}`), `--dialog
//! error:TEXT` the error dialog. Then,
//! unless `--idle`, the shell is drawn as it is while a command runs
//! elsewhere (`Shell::ui_away`), with a step of regeneration as the work.

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::{Shell, Work};
use egui::{Pos2, vec2};
use egui_wgpu::wgpu;
use serde_json::json;

const DEFAULT_SIZE: [u32; 2] = [1280, 800];

const USAGE: &str = "usage: window_picture [--idle] [--first] [--active N] [--edit N] [--ask N] [--set JSON] [--size WxH] [--scale S] [--hover X,Y,Z] [--point NAME] [--type NAME=TEXT] [--click X,Y] [--dialog export:NAME|error:TEXT] [--recent FILE] FILE.3dcraft [FILE.3dcraft | untitled | none ...] OUT.png";

fn main() -> Result<(), String> {
    let mut idle = false;
    let mut first = false;
    let mut active = None;
    let mut edit = None;
    let mut ask = None;
    let mut set: Option<serde_json::Value> = None;
    let mut size = DEFAULT_SIZE;
    let mut hover: Option<[i8; 3]> = None;
    let mut scale = 1.0_f32;
    let mut point: Option<String> = None;
    let mut typed: Option<(String, String)> = None;
    let mut click: Option<Pos2> = None;
    let mut dialog: Option<String> = None;
    let mut recent = None;
    let mut paths = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut number = |flag: &str| -> Result<usize, String> {
            args.next()
                .and_then(|n| n.parse().ok())
                .ok_or(format!("{flag} takes a number; {USAGE}"))
        };
        match arg.as_str() {
            "--idle" => idle = true,
            "--first" => first = true,
            "--active" => active = Some(number("--active")?),
            "--edit" => edit = Some(number("--edit")?),
            "--ask" => ask = Some(number("--ask")?),
            "--dialog" => {
                dialog = Some(
                    args.next()
                        .ok_or(format!("--dialog takes export:NAME or error:TEXT; {USAGE}"))?,
                );
            }
            "--set" => {
                let text = args.next().ok_or(format!("--set takes JSON; {USAGE}"))?;
                set = Some(serde_json::from_str(&text).map_err(|e| format!("--set: {e}"))?);
            }
            "--size" => {
                let text = args.next().ok_or(format!("--size takes WxH; {USAGE}"))?;
                let (w, h) = text
                    .split_once('x')
                    .ok_or(format!("--size takes WxH; {USAGE}"))?;
                size = [
                    w.parse().map_err(|_| format!("--size: {w}"))?,
                    h.parse().map_err(|_| format!("--size: {h}"))?,
                ];
                if size.iter().any(|&n| !(64..=8192).contains(&n)) {
                    return Err(format!("--size {text}: 64 to 8192 per side"));
                }
            }
            "--scale" => {
                let text = args
                    .next()
                    .ok_or(format!("--scale takes a number; {USAGE}"))?;
                scale = text.parse().map_err(|_| format!("--scale: {text}"))?;
                if !(1.0..=4.0).contains(&scale) {
                    return Err(format!("--scale {text}: 1 to 4"));
                }
            }
            "--point" => {
                point = Some(
                    args.next()
                        .ok_or(format!("--point takes a name; {USAGE}"))?,
                )
            }
            "--type" => {
                let text = args
                    .next()
                    .ok_or(format!("--type takes NAME=TEXT; {USAGE}"))?;
                let (name, value) = text
                    .split_once('=')
                    .ok_or(format!("--type takes NAME=TEXT; {USAGE}"))?;
                typed = Some((name.to_string(), value.to_string()));
            }
            "--click" => {
                let text = args.next().ok_or(format!("--click takes X,Y; {USAGE}"))?;
                let (x, y) = text
                    .split_once(',')
                    .ok_or(format!("--click takes X,Y; {USAGE}"))?;
                let x: f32 = x.trim().parse().map_err(|_| format!("--click: {x}"))?;
                let y: f32 = y.trim().parse().map_err(|_| format!("--click: {y}"))?;
                click = Some(Pos2::new(x, y));
            }
            "--recent" => {
                let file = args
                    .next()
                    .ok_or(format!("--recent takes a file; {USAGE}"))?;
                let text = std::fs::read_to_string(&file).map_err(|e| format!("--recent: {e}"))?;
                recent = Some(craft3d_ui_egui::shell::RecentFiles::from_json(&text));
            }
            "--hover" => {
                let text = args.next().ok_or(format!("--hover takes X,Y,Z; {USAGE}"))?;
                let parts: Vec<i8> = text
                    .split(',')
                    .map(|n| n.trim().parse::<i8>().map_err(|_| format!("--hover: {n}")))
                    .collect::<Result<_, _>>()?;
                match parts.as_slice() {
                    &[x, y, z] => hover = Some([x, y, z]),
                    _ => return Err(format!("--hover takes X,Y,Z; {USAGE}")),
                }
            }
            _ => paths.push(arg),
        }
    }
    let out = paths.pop().ok_or(USAGE)?;
    if paths.is_empty() {
        return Err(USAGE.to_string());
    }
    let state = headless_render_state()?;
    let mut session = Session::new(Workspace::unrestricted());
    let execute = |session: &mut Session, command: &str, params: serde_json::Value| {
        session
            .execute(command, &params)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    for (index, path) in paths.iter().enumerate() {
        if path == "none" {
            continue;
        }
        if path == "untitled" {
            execute(&mut session, "file.new", json!({}))?;
            execute(&mut session, "feature.block", json!({"size": [40, 30, 20]}))?;
        } else {
            execute(&mut session, "file.open", json!({ "path": path }))?;
        }
        if edit == Some(index) {
            execute(&mut session, "feature.sphere", json!({"radius": 5}))?;
        }
    }
    if let Some(document) = active {
        execute(
            &mut session,
            "file.activate",
            json!({ "document": document }),
        )?;
    }
    session.set_refine_elsewhere(first);
    let mut shell = Shell::new();
    shell.attach_gpu(&state)?;
    if let Some(recent) = recent {
        shell.set_recent_files(recent);
    }
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(scale);
    let pixels = size.map(|n| (n as f32 * scale).round() as u32);
    let input = |time: f64, events: Vec<egui::Event>| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            vec2(size[0] as f32, size[1] as f32),
        )),
        time: Some(time),
        events,
        ..Default::default()
    };
    // The renderer is locked only to paint: the shell's direct path
    // registers its texture with it during a frame.
    let paint = |mut output: egui::FullOutput, renderer: &mut egui_wgpu::Renderer| {
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(&state.device, &state.queue, *id, delta);
            }
        }
        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        // Applied above: epaint insists that a delta is not dropped unseen.
        output.textures_delta.clear();
        jobs
    };
    // The window as it shows the document, then as it is while work runs.
    let mut jobs = Vec::new();
    let frames: usize = if typed.is_some() || click.is_some() {
        9
    } else {
        4
    };
    let control = |shell: &Shell, name: &str| {
        shell.controls_layout().and_then(|layout| {
            layout
                .controls()
                .into_iter()
                .find(|(n, _)| *n == name)
                .map(|(_, rect)| rect.center())
        })
    };
    for frame in 0..frames {
        let time = frame as f64 * 0.1;
        if frame == 1
            && let Some(patch) = &set
        {
            shell
                .handle_method(&mut session, "ui.set", Some(patch))
                .unwrap_or(Ok(serde_json::Value::Null))?;
        }
        if frame == 1
            && let Some(document) = ask
        {
            shell.request_close(document);
        }
        // The export dialog for a file of that name, as the selection
        // makes it, or the error dialog with that text (`shell/dialogs.rs`).
        if frame == 1
            && let Some(what) = &dialog
        {
            match what.split_once(':') {
                Some(("export", name)) => {
                    let check = craft3d_ui_egui::files::export_check(&mut session, name);
                    shell.ask_export(name.to_string(), check);
                }
                Some(("error", text)) => shell.show_error("file.save", text),
                _ => return Err(format!("--dialog takes export:NAME or error:TEXT; {USAGE}")),
            }
        }
        if frame == frames - 2 && !idle {
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
        // From the second frame on, the pointer on the facet asked for,
        // where the last frame drew it.
        let events = match hover {
            Some(side) if frame >= 1 => craft3d_ui_egui::view_cube::facets(&shell.state.camera)
                .into_iter()
                .find(|f| f.side == side)
                .and_then(|f| shell.view_cube_point(f.center()))
                .map(|at| vec![egui::Event::PointerMoved(at)])
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut events = events;
        if frame >= 1
            && let Some(at) = point.as_deref().and_then(|name| control(&shell, name))
        {
            events.push(egui::Event::PointerMoved(at));
        }
        if let Some((name, text)) = &typed
            && let Some(at) = control(&shell, name)
        {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            match frame {
                1 => events.push(egui::Event::PointerMoved(at)),
                2 | 3 => events.extend([button(true), button(false)]),
                5 => events.push(egui::Event::Text(text.clone())),
                _ => {}
            }
        }
        if let Some(at) = click
            && frame >= 1
        {
            events.push(egui::Event::PointerMoved(at));
            if frame == 2 {
                events.extend([true, false].map(|pressed| egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }));
            }
        }
        let output = ctx.run_ui(input(time, events), |ui| {
            if frame < frames - 2 || idle {
                shell.ui(ui, &mut session);
            } else {
                shell.ui_away(ui);
            }
        });
        jobs = paint(output, &mut state.renderer.write());
    }
    let mut renderer = state.renderer.write();
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: pixels,
        pixels_per_point: scale,
    };
    let target = state.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: pixels[0],
            height: pixels[1],
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
    let row = pixels[0] * 4;
    let padded = row.div_ceil(256) * 256;
    let buffer = state.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded * pixels[1]),
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
                rows_per_image: Some(pixels[1]),
            },
        },
        wgpu::Extent3d {
            width: pixels[0],
            height: pixels[1],
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
    let mut rgb = Vec::with_capacity((pixels[0] * pixels[1] * 3) as usize);
    for y in 0..pixels[1] {
        let start = (y * padded) as usize;
        let line = data
            .get(start..start + row as usize)
            .ok_or("the read-back picture is shorter than asked for")?;
        for pixel in line.chunks(4) {
            rgb.extend(pixel.iter().take(3));
        }
    }
    let png = craft3d_render::encode_png(pixels[0], pixels[1], &rgb).map_err(|e| e.to_string())?;
    std::fs::write(&out, png).map_err(|e| e.to_string())?;
    println!("wrote {out} ({:?})", state.target_format);
    Ok(())
}
