//! A trace (`view.trace`) in the picture the window puts on screen, not
//! only in `ui.screenshot`'s: through real frames of the shell, on the
//! direct path (the GPU drawing on the window's device) and on the path
//! that uploads the renderer's image as a texture, for a part and for an
//! assembly whose parts move by poses. Until 2026-10-30 both window paths
//! built their marks without the traces, so the window showed none while
//! the screenshot did.
//!
//! The direct path's pictures land in `target/evidence/ui/` as
//! `trace-on-screen-*.png`.
//!
//! These tests need a graphics adapter and fail, with what to install,
//! when there is none (the GPU crate's policy).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_gpu::ADAPTER_HINT;
use craft3d_ui_egui::backend::RendererChoice;
use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::{Shell, Tokens};
use egui::{Pos2, vec2};
use serde_json::{Value, json};
use std::path::PathBuf;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn call(shell: &mut Shell, session: &mut Session, method: &str, params: Value) -> Value {
    shell
        .handle_method(session, method, Some(&params))
        .unwrap()
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// A window of a fixed size, fed one frame at a time, that keeps the last
/// image it was asked to upload as a texture of each size.
struct Window {
    ctx: egui::Context,
    time: f64,
    uploads: Vec<(usize, usize, Vec<u8>)>,
    /// How many line segments in the trace's colour the last frame
    /// painted over the picture (the overlay of a path that does not draw
    /// its lines into the texture).
    trace_segments: usize,
}

impl Window {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
            uploads: Vec::new(),
            trace_segments: 0,
        }
    }

    fn frame(&mut self, shell: &mut Shell, session: &mut Session) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            time: Some(self.time),
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        self.trace_segments = output
            .shapes
            .iter()
            .map(|clipped| trace_segments(&clipped.shape))
            .sum();
        for delta in output
            .textures_delta
            .set
            .values()
            .flat_map(|deltas| deltas.iter())
        {
            if delta.pos.is_none() {
                let egui::ImageData::Color(image) = &delta.image;
                self.uploads.push((
                    image.width(),
                    image.height(),
                    image
                        .pixels
                        .iter()
                        .flat_map(|p| [p.r(), p.g(), p.b()])
                        .collect(),
                ));
            }
        }
        output.textures_delta.clear();
    }

    fn frames(&mut self, shell: &mut Shell, session: &mut Session, n: usize) {
        for _ in 0..n {
            self.frame(shell, session);
        }
    }

    /// The last image of `width` x `height` put on screen.
    fn uploaded(&self, [width, height]: [u32; 2]) -> Option<&[u8]> {
        self.uploads
            .iter()
            .rev()
            .find(|(w, h, _)| (*w, *h) == (width as usize, height as usize))
            .map(|(_, _, rgb)| rgb.as_slice())
    }
}

/// The line segments of `shape` in the trace's colour, at any opacity.
fn trace_segments(shape: &egui::Shape) -> usize {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().map(trace_segments).sum(),
        egui::Shape::LineSegment { stroke, .. } => {
            let [r, g, b, a] = stroke.color.to_srgba_unmultiplied();
            let ink = trace_ink();
            let near = [r, g, b].iter().zip(ink).all(|(x, y)| x.abs_diff(y) <= 3);
            usize::from(a > 0 && near)
        }
        _ => 0,
    }
}

/// Pixels near `ink`.
fn count(rgb: &[u8], ink: [u8; 3], within: i32) -> usize {
    rgb.chunks(3)
        .filter(|p| (0..3).all(|k| (i32::from(p[k]) - i32::from(ink[k])).abs() <= within))
        .count()
}

fn trace_ink() -> [u8; 3] {
    Tokens::default().trace
}

fn evidence(name: &str, width: u32, height: u32, rgb: &[u8]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("trace-on-screen-{name}.png"));
    let png = craft3d_render::encode_png(width, height, rgb).unwrap();
    std::fs::write(&path, png).unwrap();
    println!("evidence: {}", path.display());
}

/// The trace's pixels in the picture the direct path shows now.
fn direct_ink(shell: &Shell, name: &str) -> usize {
    let (w, h, rgb) = shell.direct_picture().unwrap().unwrap();
    evidence(name, w, h, &rgb);
    count(&rgb, trace_ink(), 40)
}

/// A crank whose block goes round a circle of 15 mm about the z axis.
fn crank() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(&mut s, "variable.set", json!({"name": "t", "expr": "0"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Crank", "size": ["4 mm", "4 mm", "4 mm"],
               "position": ["15 mm * cos(360 deg * t) - 2 mm", "15 mm * sin(360 deg * t) - 2 mm", "0 mm"]}),
    );
    s
}

/// The name of a top corner of the crank.
fn crank_corner(s: &mut Session) -> String {
    let edges = run(s, "document.edges", json!({"feature": "Crank"}));
    edges["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["point"][2].as_f64() == Some(4.0))
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_string()
}

fn direct_shell(state: &egui_wgpu::RenderState) -> Shell {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    shell.attach_gpu(state).unwrap();
    assert!(shell.draws_directly());
    shell
}

fn look_down(shell: &mut Shell, s: &mut Session) {
    call(
        shell,
        s,
        "ui.set",
        json!({"camera": {"yaw": -90.0, "pitch": 89.0}}),
    );
    call(shell, s, "ui.fit", json!({}));
}

/// A part's trace on the direct path, plain and studio, and on the path
/// that uploads the renderer's image.
#[test]
fn a_parts_trace_is_in_the_picture_on_screen_on_both_window_paths() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut s = crank();
    let mut shell = direct_shell(&state);
    let mut window = Window::new();
    look_down(&mut shell, &mut s);
    window.frames(&mut shell, &mut s, 3);
    assert_eq!(direct_ink(&shell, "part-none"), 0);
    let corner = crank_corner(&mut s);
    run(
        &mut s,
        "view.trace",
        json!({"point": corner, "samples": 65}),
    );
    run(&mut s, "view.motion", json!({"t": 0.3}));
    window.frames(&mut shell, &mut s, 3);
    let plain = direct_ink(&shell, "part-plain");
    // A circle of 15 mm framed with a part some 34 mm across, three
    // points wide: well over a thousand pixels.
    assert!(plain > 500, "{plain} pixels of the trace, plain");
    call(
        &mut shell,
        &mut s,
        "ui.set",
        json!({"look": {"mode": "studio", "accumulate": 1}}),
    );
    window.frames(&mut shell, &mut s, 6);
    let studio = direct_ink(&shell, "part-studio");
    assert!(studio > 300, "{studio} pixels of the trace, studio");

    // The window without the direct path: the renderer's image as a
    // texture, its lines painted over it by egui (not into the texture):
    // the trace is among the segments the frame paints: its 64 segments,
    // each in one piece or more where the bodies hide part of it.
    let mut cpu = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new();
    look_down(&mut cpu, &mut s);
    window.frames(&mut cpu, &mut s, 4);
    assert!(!cpu.draws_directly());
    assert!(window.uploaded(cpu.viewport_size()).is_some());
    assert!(
        window.trace_segments >= 64,
        "{} segments of the trace painted",
        window.trace_segments
    );
    // Cleared: gone from the window.
    run(&mut s, "view.trace", json!({"clear": true}));
    window.frames(&mut cpu, &mut s, 4);
    assert_eq!(window.trace_segments, 0);
}

/// A session reading and writing under a directory of its own.
fn session_in(name: &str) -> Session {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("trace-on-screen-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("asm")).unwrap();
    Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir),
    })
}

/// A four-bar of one bar part (as `tests/assembly.rs` builds it), its
/// crank driven by `360 deg * t`.
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
    run(s, "file.save", json!({ "path": "asm/bar.3dcraft" }));
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
            json!({ "source": "asm/bar.3dcraft", "name": name, "body": "Bar",
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

/// An assembly's trace on the direct path while its parts move by poses
/// from one moment to the next (the parts are uploaded once, then only
/// moved), and a growing trace drawn as far as the moment shown.
#[test]
fn an_assemblys_trace_is_on_screen_while_its_parts_move_and_a_growing_one_grows() {
    let state = headless_render_state()
        .unwrap_or_else(|e| panic!("the direct path cannot be tested: {e}; {ADAPTER_HINT}"));
    let mut s = session_in("four-bar");
    four_bar(&mut s);
    let mut shell = direct_shell(&state);
    let mut window = Window::new();
    look_down(&mut shell, &mut s);
    run(&mut s, "view.motion", json!({ "t": 0.1 }));
    window.frames(&mut shell, &mut s, 3);
    assert_eq!(direct_ink(&shell, "assembly-none"), 0);
    run(
        &mut s,
        "view.trace",
        json!({ "point": "Coupler/End2", "samples": 65 }),
    );
    let mut seen = Vec::new();
    for t in [0.2, 0.3, 0.45] {
        run(&mut s, "view.motion", json!({ "t": t }));
        window.frames(&mut shell, &mut s, 3);
        seen.push(direct_ink(&shell, &format!("assembly-t{t}")));
    }
    assert!(
        seen.iter().all(|n| *n > 300),
        "{seen:?} pixels of the trace"
    );

    // Growing: drawn as far as the moment shown, so more of it later on.
    run(
        &mut s,
        "view.trace",
        json!({ "point": "Coupler/End2", "samples": 65, "grow": true, "name": "Trace 1" }),
    );
    let got = call(&mut shell, &mut s, "ui.get", json!({}));
    assert_eq!(got["timeline"]["traces"], 1);
    let mut grown = Vec::new();
    // 65 samples over [0, 1]: a sample every 1/64 of t, and at a `t`
    // between two samples the point where it lies.
    for (t, points) in [(0.25, 17), (0.5, 33), (0.95, 62)] {
        run(&mut s, "view.motion", json!({ "t": t }));
        window.frames(&mut shell, &mut s, 3);
        let drawn = shell.drawn_traces();
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].len(), points, "t = {t}");
        grown.push(direct_ink(&shell, &format!("assembly-grow-t{t}")));
    }
    // More of it is on screen as it grows. (How much at the first moment
    // depends on how much of that quarter the parts cover: there is some.)
    assert!(
        grown.windows(2).all(|w| w[1] > w[0]),
        "{grown:?} pixels of the growing trace"
    );
    assert!(grown[0] > 0, "{grown:?}");
    // At the end of the motion it is the whole trace.
    run(&mut s, "view.motion", json!({ "t": 1.0 }));
    window.frames(&mut shell, &mut s, 3);
    assert_eq!(shell.drawn_traces()[0].len(), 65);
    let whole = direct_ink(&shell, "assembly-grow-t1");
    assert!(whole >= grown[2], "{whole} against {grown:?}");
}
