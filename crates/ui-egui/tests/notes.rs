//! The notes pinned to the model in the window (`shell/notes.rs`,
//! `shell/sidepanel/notes.rs`): nothing of them for a document without
//! notes; the Notes tab with the open ones counted, an agent's question
//! first; a marker per note in the view, a click on it choosing its note;
//! **N** on a selected face writing a note as the person; the tab's row
//! closing a note through `note.resolve`; the agent pill counting an
//! agent's open questions; a thousand notes thinned to the markers' cap.
//! The pictures (ignored, for a person to look at): light and dark, a
//! chosen note, the field, hundreds of notes, a phone; written to
//! `CRAFT3D_PICS` (else `plan/viewer-tools/pins-pics` of the workshop, or
//! `target/evidence/notes`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::{RendererChoice, Shell, SideTab};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_wgpu::wgpu;
use serde_json::{Value, json};

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

fn get(shell: &mut Shell, session: &mut Session) -> Value {
    shell
        .handle_method(session, "ui.get", None)
        .unwrap()
        .unwrap()
}

struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            size: vec2(width, height),
            time: 0.0,
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.time += 1.0 / 30.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
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

    fn click(&mut self, shell: &mut Shell, session: &mut Session, at: Pos2) {
        self.frame(shell, session, vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
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
        self.frames(shell, session, 2);
    }

    fn key(&mut self, shell: &mut Shell, session: &mut Session, key: Key) {
        for pressed in [true, false] {
            self.frame(
                shell,
                session,
                vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
            );
        }
    }
}

/// A plate 60 × 40 × 10 (F1) rounded along one top edge (F2, `Round`), a
/// lug beside it (F3) deleted after a note was put on it.
fn plate() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [60, 40, 10]}),
    );
    run(
        &mut s,
        "feature.fillet",
        json!({"edges": ["F1:edge(9)"], "radius": 4, "name": "Round"}),
    );
    s
}

/// The plate with its notes: one open (the person's), an agent's
/// question, one done by an agent, one detached, one by place on the
/// fillet.
fn noted() -> Session {
    let mut s = plate();
    s.by_person(|s| {
        s.execute(
            "note.add",
            &json!({"text": "2 mm thicker here", "on": "Plate:face(1)", "point": [40, 25, 10]}),
        )
    })
    .unwrap();
    s.acting(Some("Claude Code"), None, |s| {
        s.execute(
            "note.add",
            &json!({"text": "Should this side be drafted?", "on": "Plate:face(2)"}),
        )
    })
    .unwrap();
    s.by_person(|s| {
        s.execute(
            "note.add",
            &json!({"text": "round this edge", "on": "Plate:face(4)"}),
        )
    })
    .unwrap();
    s.acting(Some("Claude Code"), None, |s| {
        s.execute("note.resolve", &json!({"id": 3, "reply": "Rounded, 4 mm."}))
    })
    .unwrap();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Lug", "size": [10, 10, 10], "position": [60, 15, 0]}),
    );
    s.by_person(|s| {
        s.execute(
            "note.add",
            &json!({"text": "this lug can go", "on": "Lug:face(5)"}),
        )
    })
    .unwrap();
    run(&mut s, "feature.delete", json!({"feature": "Lug"}));
    s.by_person(|s| {
        s.execute(
            "note.add",
            &json!({"text": "a smaller radius here", "on": "F2:fillet(0)"}),
        )
    })
    .unwrap();
    s
}

fn open(window: &mut Window, session: &mut Session) -> Shell {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    window.frames(&mut shell, session, 6);
    shell
}

fn tabs(shell: &Shell) -> Vec<SideTab> {
    shell.side_hits().tabs.iter().map(|(t, _)| *t).collect()
}

#[test]
fn a_document_without_notes_shows_nothing_of_them() {
    let mut session = plate();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    assert!(
        !tabs(&shell).contains(&SideTab::Notes),
        "{:?}",
        tabs(&shell)
    );
    assert!(shell.note_markers().0.is_empty());
    let state = get(&mut shell, &mut session);
    assert!(state["state"].get("notes").is_none(), "{state}");
    // N with nothing selected opens no field.
    w.key(&mut shell, &mut session, Key::N);
    assert!(shell.note_writing().is_none());
}

#[test]
fn notes_show_in_the_tab_and_the_view_and_a_marker_chooses_its_note() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "notes"}}),
    );
    w.frames(&mut shell, &mut session, 3);
    assert!(tabs(&shell).contains(&SideTab::Notes), "{:?}", tabs(&shell));
    assert_eq!(shell.note_rows().len(), 5);
    let place = |id: u64| {
        shell
            .note_rows()
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .place
            .clone()
    };
    assert_eq!(place(4), "detached");
    assert_eq!(place(5), "by place, near Round");
    let (markers, stood_for) = shell.note_markers();
    assert!(
        !markers.is_empty() && stood_for >= markers.len(),
        "{markers:?}"
    );
    let (id, at) = markers[0];
    w.click(&mut shell, &mut session, at);
    assert_eq!(shell.state.notes.selected, Some(id));
    // The eye hides them; ui.set as well.
    set(
        &mut shell,
        &mut session,
        json!({"notes": {"hideMarkers": true}}),
    );
    w.frames(&mut shell, &mut session, 2);
    assert!(shell.note_markers().0.is_empty());
    let state = get(&mut shell, &mut session);
    assert_eq!(state["state"]["notes"]["hideMarkers"], true, "{state}");
}

#[test]
fn n_on_a_selected_face_writes_a_note_as_the_person() {
    let mut session = plate();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    // The plate's top as the rounded body names it.
    run(
        &mut session,
        "select.set",
        json!({"names": ["F2:copy(0)/F1:face(1)"]}),
    );
    w.frames(&mut shell, &mut session, 2);
    w.key(&mut shell, &mut session, Key::N);
    assert!(shell.note_writing().is_some(), "N opened the field");
    // The Notes tab is offered while a note is written (it may sit under
    // the tab bar's ⋯ here): choosing it holds.
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "notes"}}),
    );
    w.frames(&mut shell, &mut session, 2);
    assert!(shell.note_writing().is_some(), "still writing");
    assert_eq!(
        shell.state.panels.side_tab,
        SideTab::Notes,
        "the tab while writing"
    );
    shell.type_note("2 mm thicker here", true);
    w.frames(&mut shell, &mut session, 3);
    let listed = run(&mut session, "note.list", json!({}));
    assert_eq!(listed["count"], 1, "{listed}");
    assert_eq!(listed["notes"][0]["author"], "person");
    assert_eq!(listed["notes"][0]["text"], "2 mm thicker here");
    // One undo step takes it back.
    run(&mut session, "edit.undo", json!({}));
    assert_eq!(run(&mut session, "note.list", json!({}))["count"], 0);
}

/// *Note here…* in the list under the right mouse button writes a note on
/// the face selected, at the point the right click was on it.
#[test]
fn note_here_writes_a_note_where_the_right_click_was() {
    let mut session = plate();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    run(
        &mut session,
        "select.set",
        json!({"names": ["F2:copy(0)/F1:face(1)"]}),
    );
    w.frames(&mut shell, &mut session, 2);
    // The middle of the view, on the plate's top.
    let panel = shell.side_hits().panel.unwrap();
    let at = Pos2::new((panel.right() + 1280.0) / 2.0, 400.0);
    w.frame(&mut shell, &mut session, vec![Event::PointerMoved(at)]);
    for pressed in [true, false] {
        w.frame(
            &mut shell,
            &mut session,
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            }],
        );
    }
    w.frames(&mut shell, &mut session, 2);
    let entry = shell.note_here_entry().expect("Note here… offered");
    w.click(&mut shell, &mut session, entry.center());
    let writing = shell.note_writing().expect("the field opened");
    let point = writing.point.expect("the right click's point");
    assert!((point[2] - 10.0).abs() < 1e-6, "on the top: {point:?}");
    shell.type_note("a boss here", true);
    w.frames(&mut shell, &mut session, 3);
    let listed = run(&mut session, "note.list", json!({}));
    let note = &listed["notes"][0];
    assert_eq!(note["author"], "person", "{listed}");
    assert_eq!(note["on"]["kind"], "face", "{listed}");
    for k in 0..3 {
        let got = note["point"][k].as_f64().unwrap();
        assert!((got - point[k]).abs() < 1e-9, "{listed}");
    }
}

#[test]
fn the_tab_closes_a_note_and_the_pill_counts_an_agents_questions() {
    let mut session = noted();
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    assert!(
        shell.agents_pill_view().label.ends_with("1 question"),
        "{}",
        shell.agents_pill_view().label
    );
    set(
        &mut shell,
        &mut session,
        json!({"panels": {"sideTab": "notes"}}),
    );
    w.frames(&mut shell, &mut session, 3);
    // The question comes first among the open notes.
    let rows: Vec<String> = shell
        .side_hits()
        .targets
        .iter()
        .filter(|(k, _)| k.starts_with("note:"))
        .map(|(k, _)| k.clone())
        .collect();
    assert_eq!(rows.first().map(String::as_str), Some("note:2"), "{rows:?}");
    let target = |shell: &Shell, key: &str| {
        shell
            .side_hits()
            .targets
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| panic!("no {key}"))
    };
    let row = target(&shell, "note:2");
    w.click(&mut shell, &mut session, row);
    assert_eq!(shell.state.notes.selected, Some(2));
    let done = target(&shell, "note resolve:2");
    w.click(&mut shell, &mut session, done);
    w.frames(&mut shell, &mut session, 2);
    let listed = run(&mut session, "note.list", json!({"state": "done"}));
    assert!(
        listed["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == 2 && n["done"]["author"] == "person"),
        "{listed}"
    );
    assert_eq!(shell.notes_questions(), 0);
}

#[test]
fn a_thousand_notes_draw_within_the_cap() {
    let mut session = plate();
    for k in 0..1000 {
        let x = 1.0 + f64::from(k % 40) * 1.4;
        let y = 1.0 + f64::from(k / 40) * 1.4;
        run(
            &mut session,
            "note.add",
            json!({"text": format!("note {k}"), "on": "F1:face(0)", "point": [x, y, 0]}),
        );
    }
    let mut w = Window::new(1280.0, 800.0);
    let mut shell = open(&mut w, &mut session);
    set(
        &mut shell,
        &mut session,
        json!({"camera": {"pitch": -60.0}}),
    );
    w.frames(&mut shell, &mut session, 3);
    let (markers, stood_for) = shell.note_markers();
    println!(
        "1000 notes: {} markers drawn for {stood_for}",
        markers.len()
    );
    assert!(markers.len() <= craft3d_ui_egui::shell::notes::MAX_MARKERS);
    // A frame with every marker looked up again (a new camera) and one at
    // rest, against the same window without notes.
    let time = |shell: &mut Shell, session: &mut Session, w: &mut Window, turn: bool| {
        let started = std::time::Instant::now();
        for k in 0..5 {
            if turn {
                let yaw = 30.0 + f64::from(k);
                shell
                    .handle_method(session, "ui.set", Some(&json!({"camera": {"yaw": yaw}})))
                    .unwrap()
                    .unwrap();
            }
            w.frame(shell, session, Vec::new());
        }
        started.elapsed().as_secs_f64() / 5.0
    };
    let turning = time(&mut shell, &mut session, &mut w, true);
    let resting = time(&mut shell, &mut session, &mut w, false);
    set(
        &mut shell,
        &mut session,
        json!({"notes": {"hideMarkers": true}}),
    );
    w.frames(&mut shell, &mut session, 2);
    let without = time(&mut shell, &mut session, &mut w, true);
    println!(
        "frame with 1000 notes: {:.1} ms turning, {:.1} ms at rest; without markers {:.1} ms",
        turning * 1e3,
        resting * 1e3,
        without * 1e3
    );
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

/// A window drawn by egui's own wgpu renderer, for pictures.
struct Shot<'a> {
    state: &'a egui_wgpu::RenderState,
    window: Window,
    jobs: Vec<egui::ClippedPrimitive>,
}

impl Shot<'_> {
    fn frame(&mut self, shell: &mut Shell, session: &mut Session, events: Vec<Event>) {
        self.window.time += 1.0 / 30.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.window.size)),
            time: Some(self.window.time),
            events,
            ..Default::default()
        };
        let mut output = self.window.ctx.run_ui(input, |ui| shell.ui(ui, session));
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
        self.jobs = self
            .window
            .ctx
            .tessellate(output.shapes, output.pixels_per_point);
    }

    /// The picture as PNG, and its rows of RGB pixels.
    fn png(&self) -> (Vec<u8>, Vec<u8>) {
        let size = [self.window.size.x as u32, self.window.size.y as u32];
        let rgb = picture(self.state, &self.jobs, size);
        (
            craft3d_render::encode_png(size[0], size[1], &rgb).unwrap(),
            rgb,
        )
    }
}

type Prepare<'p> = &'p dyn Fn(&mut Shell, &mut Session, &mut Shot<'_>);

#[test]
#[ignore = "writes pictures for a person to look at; run with --ignored"]
fn pictures_of_the_notes() {
    use craft3d_ui_egui::direct::headless_render_state;
    let Ok(state) = headless_render_state() else {
        println!("no graphics adapter, no pictures");
        return;
    };
    let dir = std::env::var("CRAFT3D_PICS").map_or_else(
        |_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/notes")
        },
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let shoot =
        |name: &str, session: &mut Session, size: [f32; 2], ui_set: Value, prepare: Prepare<'_>| {
            let mut shot = Shot {
                state: &state,
                window: Window::new(size[0], size[1]),
                jobs: Vec::new(),
            };
            let mut shell = Shell::with_renderer(RendererChoice::Cpu);
            for _ in 0..4 {
                shot.frame(&mut shell, session, Vec::new());
            }
            set(&mut shell, session, ui_set);
            for _ in 0..6 {
                shot.frame(&mut shell, session, Vec::new());
            }
            prepare(&mut shell, session, &mut shot);
            for _ in 0..3 {
                shot.frame(&mut shell, session, Vec::new());
            }
            let (png, rgb) = shot.png();
            std::fs::write(dir.join(format!("{name}.png")), &png).unwrap();
            rgb
        };
    let nothing = |_: &mut Shell, _: &mut Session, _: &mut Shot<'_>| {};
    for theme in ["light", "dark"] {
        let mut session = noted();
        shoot(
            &format!("notes-{theme}"),
            &mut session,
            [1400.0, 900.0],
            json!({"appearance": theme, "panels": {"sideTab": "notes"}, "notes": {"selected": 1}}),
            &nothing,
        );
    }
    let mut session = noted();
    shoot(
        "notes-done-open",
        &mut session,
        [1400.0, 900.0],
        json!({"panels": {"sideTab": "notes"}, "notes": {"selected": 2, "doneOpen": true}}),
        &nothing,
    );
    let mut session = plate();
    run(
        &mut session,
        "select.set",
        json!({"names": ["F2:copy(0)/F1:face(1)"]}),
    );
    shoot(
        "notes-writing",
        &mut session,
        [1400.0, 900.0],
        json!({}),
        &|shell, session, shot| {
            for pressed in [true, false] {
                shot.frame(
                    shell,
                    session,
                    vec![Event::Key {
                        key: Key::N,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    }],
                );
            }
            shell.type_note("2 mm thicker here", false);
        },
    );
    let mut session = plate();
    for k in 0..300 {
        let x = 2.0 + f64::from(k % 20) * 2.8;
        let y = 2.0 + f64::from(k / 20) * 1.8;
        run(
            &mut session,
            "note.add",
            json!({"text": format!("note {k}"), "on": "F1:face(1)", "point": [x, y, 10]}),
        );
    }
    shoot(
        "notes-hundreds",
        &mut session,
        [1400.0, 900.0],
        json!({"panels": {"sideTab": "notes"}}),
        &nothing,
    );
    let mut session = noted();
    shoot(
        "notes-phone",
        &mut session,
        [390.0, 844.0],
        json!({"panels": {"drawer": true, "sideTab": "notes"}}),
        &nothing,
    );
    // A document without notes draws as it did: the same picture before
    // a note was added and after it was taken away again.
    let mut session = plate();
    let before = shoot(
        "plain-before",
        &mut session,
        [1400.0, 900.0],
        json!({}),
        &nothing,
    );
    run(
        &mut session,
        "note.add",
        json!({"text": "x", "on": "F1:face(1)"}),
    );
    run(&mut session, "note.delete", json!({"state": "all"}));
    let after = shoot(
        "plain-after",
        &mut session,
        [1400.0, 900.0],
        json!({}),
        &nothing,
    );
    // Pixel for pixel, but for the status bar's bottom 32 rows, where the
    // milliseconds the last frame's mesh took differ from run to run.
    let above = (900 - 32) * 1400 * 3;
    assert!(
        before.get(..above) == after.get(..above),
        "a document without notes draws as before"
    );
}
