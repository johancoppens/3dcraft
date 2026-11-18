//! The grid of a grid surface in the window: drawn with the sketches and
//! switched with them (`ui.set {"show": {"sketches": false}}`, the
//! Sketches toggle), said by `ui.get` (`scene.grids`), and tested against
//! the depth of the bodies a lift nearer than it is
//! (`SceneSketch::lift`), by the CPU overlay and in the GPU's segments
//! alike: its lines and points lie on the sheet's exact surface, which
//! the mesh follows only within the scene's tolerance, so without the
//! lift a grid on a hollow sheet would be drawn dimmed where it should
//! show.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SURFACE_LIFT, Scene, SceneQuality, Session, Workspace};
use craft3d_math::{Portable, Vec3};
use craft3d_ui_egui::state::CameraState;
use craft3d_ui_egui::viewport::{self, LineKind, Marks, Quality};
use craft3d_ui_egui::{Shell, Tokens};
use serde_json::{Value, json};
use std::sync::Arc;

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

/// A bowl of 7 × 6 heights from a table, 10 mm apart: a sheet hollow
/// toward the eye above it, where the mesh's chords lie above the exact
/// surface and a line on that surface below them.
fn bowl() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    let rows: Vec<Value> = (0..6)
        .map(|r| {
            let mut row = vec![json!(r + 1)];
            for c in 0..7 {
                let (x, y) = (c as f64 - 3.0, r as f64 - 2.5);
                row.push(json!(0.9 * (x * x + y * y)));
            }
            Value::Array(row)
        })
        .collect();
    run(
        &mut s,
        "table.set",
        json!({"name": "bowl", "columns": ["row", "a", "b", "c", "d", "e", "f", "g"], "rows": rows}),
    );
    run(
        &mut s,
        "feature.surface.grid",
        json!({"table": "bowl", "step": ["10 mm", "10 mm"], "factor": "1 mm", "name": "Bowl"}),
    );
    s
}

fn scene(session: &mut Session) -> Arc<Scene> {
    session
        .scene_with(SceneQuality::display())
        .unwrap()
        .unwrap()
}

fn screenshot(shell: &mut Shell, session: &mut Session) -> Value {
    call(
        shell,
        session,
        "ui.screenshot",
        json!({"width": 320, "height": 240, "scale": 1.0}),
    )
}

#[test]
fn the_grid_is_switched_with_the_sketches_and_ui_get_says_it() {
    let mut session = bowl();
    let mut shell = Shell::new();
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(
        got["scene"]["grids"],
        json!([{"feature": "F1", "name": "Bowl", "rows": 6, "columns": 7, "every": [1, 1], "thinned": false}]),
        "{}",
        got["scene"]
    );
    assert_eq!(got["scene"]["sketches"], 1);
    let shown = screenshot(&mut shell, &mut session);
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"sketches": false}}),
    );
    let hidden = screenshot(&mut shell, &mut session);
    let coverage = |shot: &Value| shot["overlayCoverage"].as_f64().unwrap();
    assert!(
        coverage(&shown) > coverage(&hidden),
        "{shown} against {hidden}"
    );
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"show": {"sketches": true}}),
    );
    let again = screenshot(&mut shell, &mut session);
    assert_eq!(shown["base64"], again["base64"], "bit for bit");
    // Hidden with its feature, which the document keeps.
    run(
        &mut session,
        "feature.visible",
        json!({"feature": "Bowl", "visible": false}),
    );
    let got = call(&mut shell, &mut session, "ui.get", json!({}));
    assert_eq!(got["scene"]["grids"], json!([]));
}

/// From above at a slant, at the display's quality.
fn above(scene: &Scene, size: [u32; 2]) -> CameraState {
    let mut camera = CameraState {
        yaw: 30.0,
        pitch: 55.0,
        ..CameraState::default()
    };
    camera.fit(&scene.bbox, f64::from(size[0]) / f64::from(size[1]));
    camera
}

#[test]
fn the_grids_lines_and_points_lie_on_the_sheet_for_both_paths() {
    let mut session = bowl();
    let scene = scene(&mut session);
    let size = [480, 360];
    let camera = above(&scene, size);
    let quality = Quality {
        width: size[0],
        height: size[1],
        ssaa: 1,
    };
    let frame =
        viewport::render_bodies(Some(&scene), &camera, quality, &Tokens::default()).unwrap();
    // The CPU overlay: every point of the grid in sight, and its lines too
    // (but where a line runs off the sheet's border, past the picture's
    // last sample of it); asked with what is hidden kept (see-through),
    // so that nothing hidden goes unseen here.
    let slack = viewport::depth_slack(
        craft3d_ui_egui::backend::Backend::Cpu,
        Some(&scene),
        &camera,
    );
    let overlay = viewport::overlay_see_through(
        Some(&scene),
        &camera,
        &frame,
        None,
        slack,
        &Marks::default(),
        true,
    );
    let points: Vec<_> = overlay
        .points
        .iter()
        .filter(|p| p.kind == LineKind::Sketch)
        .collect();
    assert_eq!(points.len(), 42);
    assert!(points.iter().all(|p| !p.hidden), "{points:?}");
    // The lines are drawn as construction geometry (a guide, not a
    // profile), the points as a sketch's points.
    assert!(!overlay.lines.iter().any(|l| l.kind == LineKind::Sketch));
    let length = |hidden: bool| -> f64 {
        overlay
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Construction && l.hidden == hidden)
            .map(|l| (l.to[0] - l.from[0]).phypot(l.to[1] - l.from[1]))
            .sum()
    };
    let (seen, behind) = (length(false), length(true));
    assert!(seen > 1000.0, "{seen} pixels of the grid's lines");
    assert!(behind <= 1e-3 * seen, "{behind} of {seen} pixels dimmed");
    // Without the lift (the same lines as a sketch's), the hollow sheet's
    // mesh lies above them: much of the grid would be dimmed.
    let mut unlifted = (*scene).clone();
    for sketch in &mut unlifted.sketches {
        sketch.grid = None;
    }
    let overlay = viewport::overlay_on(Some(&unlifted), &camera, &frame, None);
    let dimmed = overlay
        .points
        .iter()
        .filter(|p| p.kind == LineKind::Sketch && p.hidden)
        .count();
    assert!(dimmed > 0, "without the lift no point was dimmed");
    // The GPU's segments carry the same lift, the bodies' lines none.
    let tokens = Tokens::default();
    let segments = craft3d_ui_egui::direct::segments_marked(
        Some(&scene),
        None,
        &tokens,
        1.0,
        &Marks::default(),
        false,
    );
    let lift = (SURFACE_LIFT * scene.tolerance) as f32;
    let grid_segments = |segments: &[craft3d_gpu::Segment]| -> Vec<craft3d_gpu::Segment> {
        segments
            .iter()
            .filter(|s| {
                s.color == LineKind::Sketch.color(&tokens)
                    || s.color == LineKind::Construction.color(&tokens)
            })
            .copied()
            .collect()
    };
    let sketchy = grid_segments(&segments);
    // The rows and columns (from the knots: one chord a span at least)
    // and the 42 points.
    assert!(sketchy.len() > 42 + 6 * 4 + 7 * 3, "{}", sketchy.len());
    assert!(sketchy.iter().all(|s| s.lift == lift), "{lift}");
    // Left out where hidden, unless see-through: then drawn dimmed.
    assert!(sketchy.iter().all(|s| s.tested && !s.dimmed));
    let through = craft3d_ui_egui::direct::segments_marked(
        Some(&scene),
        None,
        &tokens,
        1.0,
        &Marks::default(),
        true,
    );
    assert!(grid_segments(&through).iter().all(|s| s.tested && s.dimmed));
    assert!(
        segments
            .iter()
            .filter(|s| s.edge.is_some())
            .all(|s| s.lift == 0.0)
    );
}

/// The grid's lines of an overlay as chains, each a line of the grid as
/// it runs over the picture: stretches that follow on from each other (the
/// end of one the start of the next), in order, with whether each is
/// hidden and its length in pixels.
fn chains(overlay: &viewport::Overlay) -> Vec<Vec<(bool, f64)>> {
    let mut out: Vec<Vec<(bool, f64)>> = Vec::new();
    let mut last: Option<[f64; 2]> = None;
    for line in overlay
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Construction)
    {
        let length = (line.to[0] - line.from[0]).phypot(line.to[1] - line.from[1]);
        match (out.last_mut(), last) {
            (Some(chain), Some(end)) if end == line.from => chain.push((line.hidden, length)),
            _ => out.push(vec![(line.hidden, length)]),
        }
        last = Some(line.to);
    }
    out
}

/// How often a line of the grid goes in and out of sight within `short`
/// pixels: runs (of one state, the stretches joined) shorter than that
/// with the other state on both sides.
fn flickers(chains: &[Vec<(bool, f64)>], short: f64) -> usize {
    let mut count = 0;
    for chain in chains {
        let mut runs: Vec<(bool, f64)> = Vec::new();
        for &(hidden, length) in chain {
            match runs.last_mut() {
                Some(run) if run.0 == hidden => run.1 += length,
                _ => runs.push((hidden, length)),
            }
        }
        count += runs
            .windows(3)
            .filter(|w| w[1].1 < short && w[0].0 == w[2].0)
            .count();
    }
    count
}

/// A ridge along x from a table, seen low from the side: the rows run
/// along the crest and the columns over it.
fn ridge() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    let rows: Vec<Value> = (0..9)
        .map(|r| {
            let mut row = vec![json!(r + 1)];
            for c in 0..12 {
                let (x, y) = (c as f64 * 5.0, r as f64 * 5.0);
                let crest = 12.0 * (-(y - 20.0) * (y - 20.0) / 60.0).pexp();
                row.push(json!(crest + 0.8 * (x / 7.0).psin()));
            }
            Value::Array(row)
        })
        .collect();
    let columns: Vec<String> = std::iter::once("row".to_string())
        .chain((0..12).map(|c| format!("c{c}")))
        .collect();
    run(
        &mut s,
        "table.set",
        json!({"name": "ridge", "columns": columns, "rows": rows}),
    );
    run(
        &mut s,
        "feature.surface.grid",
        json!({"table": "ridge", "step": ["5 mm", "5 mm"], "factor": "1 mm", "name": "Ridge"}),
    );
    s
}

/// The overlay of `scene` seen with `camera` (`size` pixels, CPU), its
/// hidden stretches drawn dimmed (see-through) or left out.
fn overlay_of(
    scene: &Scene,
    camera: &CameraState,
    size: [u32; 2],
    see_through: bool,
) -> viewport::Overlay {
    let quality = Quality {
        width: size[0],
        height: size[1],
        ssaa: 1,
    };
    let frame = viewport::render_bodies(Some(scene), camera, quality, &Tokens::default()).unwrap();
    let slack = viewport::depth_slack(craft3d_ui_egui::backend::Backend::Cpu, Some(scene), camera);
    viewport::overlay_see_through(
        Some(scene),
        camera,
        &frame,
        None,
        slack,
        &Marks::default(),
        see_through,
    )
}

/// A ridge seen low from either side and at a slant: the lines of the
/// grid that pass behind the crest end there once, they do not go in and
/// out of sight pixel by pixel (the lift grows with the slant of the
/// surface under the line, and a lifted line is hidden where most of five
/// places a pixel apart are: `viewport::lifted_hides`), and what lies
/// behind is left out, unless see-through.
#[test]
fn a_grid_over_a_ridge_seen_low_does_not_flicker_and_its_back_is_left_out() {
    let mut session = ridge();
    let scene = scene(&mut session);
    let size = [800, 500];
    for (yaw, pitch) in [
        (-90.0, 3.0),
        (-90.0, 8.0),
        (-70.0, 5.0),
        (-110.0, 6.0),
        (90.0, 5.0),
        (-90.0, 15.0),
    ] {
        let mut camera = CameraState {
            yaw,
            pitch,
            ..CameraState::default()
        };
        camera.fit(&scene.bbox, f64::from(size[0]) / f64::from(size[1]));
        // Every decision, the hidden stretches drawn dimmed.
        let all = chains(&overlay_of(&scene, &camera, size, true));
        let hidden: f64 = all.iter().flatten().filter(|s| s.0).map(|s| s.1).sum();
        let seen: f64 = all.iter().flatten().filter(|s| !s.0).map(|s| s.1).sum();
        assert!(
            hidden > 1000.0 && seen > 3000.0,
            "yaw {yaw}, pitch {pitch}: {seen} px seen, {hidden} px hidden"
        );
        assert_eq!(
            flickers(&all, 4.0),
            0,
            "yaw {yaw}, pitch {pitch}: in and out of sight within 4 px"
        );
        // Left out without see-through: what is drawn is what was seen.
        let opaque = overlay_of(&scene, &camera, size, false);
        let drawn = chains(&opaque);
        assert!(drawn.iter().flatten().all(|s| !s.0));
        let drawn_length: f64 = drawn.iter().flatten().map(|s| s.1).sum();
        assert!(
            (drawn_length - seen).abs() <= 1e-6 * seen,
            "{drawn_length} against {seen}"
        );
        assert!(
            !opaque
                .points
                .iter()
                .any(|p| p.kind == LineKind::Sketch && p.hidden)
        );
    }
}

/// The ridge's sheet cut into a block, with a sketch line across the
/// plane of the sections below (a sketch is drawn whole under a section).
fn ridge_cut_into_a_block() -> Session {
    let mut s = ridge();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Base", "size": [55, 40, 30], "position": [0, 0, -10]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "intersect", "target": "Base", "tool": "Ridge", "name": "Terrain"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "offset": "25 mm", "name": "Across"}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Across", "from": [10, 2], "to": [10, 38]}),
    );
    s
}

/// The section planes the grid is cut by below: across y through the
/// middle, the other half (flipped), and slanted.
fn planes() -> [(&'static str, Vec3, Vec3); 3] {
    [
        (
            "across y",
            Vec3::new(27.0, 21.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ),
        (
            "flipped",
            Vec3::new(27.0, 21.0, 0.0),
            Vec3::new(0.0, -1.0, 0.0),
        ),
        (
            "slanted",
            Vec3::new(30.0, 18.0, 5.0),
            Vec3::new(1.0, 1.0, 0.5),
        ),
    ]
}

/// A section cuts the grid where it cuts the bodies: nothing of the grid
/// on the side cut away, every line that crosses the plane ending on it
/// exactly, the points beyond it left out; the GPU's segments and the
/// CPU overlay's lines the same pieces; a sketch's line across the plane
/// drawn whole, as ever.
#[test]
fn a_section_cuts_the_grid_and_its_lines_end_on_the_plane() {
    let mut session = ridge_cut_into_a_block();
    let scene = scene(&mut session);
    let grid = scene.sketches.iter().find(|s| s.grid.is_some()).unwrap();
    let across = scene.sketches.iter().find(|s| s.name == "Across").unwrap();
    let tokens = Tokens::default();
    let mut camera = CameraState {
        yaw: -60.0,
        pitch: 30.0,
        ..CameraState::default()
    };
    camera.fit(&scene.bbox, 800.0 / 500.0);
    let projector =
        craft3d_ui_egui::camera::Projector::new(&camera.to_render(), 800.0, 500.0).unwrap();
    let diagonal = scene.bbox.diagonal();
    let construction = LineKind::Construction.color(&tokens);
    let sketch_color = LineKind::Sketch.color(&tokens);
    for (name, origin, normal) in planes() {
        let section = craft3d_render::Section {
            origin,
            normal,
            cap: true,
            color: [255, 128, 0],
            hatch: None,
        };
        let cut = viewport::SectionCut::new(&scene, section).unwrap();
        let side = |p: Vec3| cut.cut.side(p);
        let marks = Marks {
            cut: Some(cut.clone()),
            ..Marks::default()
        };
        // What the grid would be cut into: its lines that cross the plane.
        let crossing = grid
            .polylines
            .iter()
            .filter(|l| {
                l.points.iter().any(|p| side(*p) > 0.0) && l.points.iter().any(|p| side(*p) < 0.0)
            })
            .count();
        assert!(crossing > 3, "{name}: {crossing} lines cross the plane");
        let kept_points = grid.points.iter().filter(|p| side(**p) <= 0.0).count();
        assert!(kept_points > 0 && kept_points < grid.points.len(), "{name}");
        // The GPU's segments: the grid's on the kept side only, ending on
        // the plane; its points beyond the plane left out.
        let segments = craft3d_ui_egui::direct::segments_marked(
            Some(&scene),
            None,
            &tokens,
            1.0,
            &marks,
            false,
        );
        let lines: Vec<_> = segments
            .iter()
            .filter(|s| s.color == construction)
            .collect();
        let beyond = |p: Vec3| side(p) > 1e-12 * diagonal;
        assert!(
            lines.iter().all(|s| !beyond(s.from) && !beyond(s.to)),
            "{name}: a piece of the grid beyond the plane"
        );
        let on_plane = lines
            .iter()
            .flat_map(|s| [s.from, s.to])
            .filter(|p| side(*p).abs() <= 1e-12 * diagonal)
            .count();
        assert!(
            on_plane >= crossing,
            "{name}: {on_plane} ends on the plane for {crossing} lines that cross it"
        );
        let dots = segments
            .iter()
            .filter(|s| s.color == sketch_color && s.from == s.to && s.lift > 0.0)
            .count();
        assert_eq!(dots, kept_points, "{name}");
        // The CPU overlay draws the same pieces (untested: every one).
        let overlay = viewport::overlay_untested(Some(&scene), &camera, 800, 500, None, &marks);
        let drawn: Vec<_> = overlay
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Construction)
            .collect();
        assert_eq!(drawn.len(), lines.len(), "{name}");
        for (line, segment) in drawn.iter().zip(&lines) {
            let (from, to) = projector.segment(segment.from, segment.to).unwrap();
            assert_eq!(line.from, [from.x, from.y], "{name}");
            assert_eq!(line.to, [to.x, to.y], "{name}");
        }
        let points = overlay
            .points
            .iter()
            .filter(|p| p.kind == LineKind::Sketch)
            .count();
        // The grid's kept points, and the sketch line's two ends, which
        // a section does not cut.
        assert_eq!(points, kept_points + 2, "{name}");
        // The sketch's line across the plane is drawn whole.
        let whole: Vec<_> = segments
            .iter()
            .filter(|s| s.color == sketch_color && s.from != s.to)
            .collect();
        let pieces: usize = across
            .polylines
            .iter()
            .map(|l| l.points.len().saturating_sub(1))
            .sum();
        assert_eq!(whole.len(), pieces, "{name}");
        assert!(
            whole.iter().any(|s| beyond(s.from) || beyond(s.to)),
            "{name}: the sketch reaches beyond the plane"
        );
    }
}

/// A window of `size` points, driven frame by frame.
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

    /// Three frames: what the window shows has settled.
    fn settle(&mut self, shell: &mut Shell, session: &mut Session) {
        for _ in 0..3 {
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                time: Some(self.time),
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
            output.textures_delta.clear();
        }
    }
}

/// A PNG of `ui.screenshot` as its pixels.
fn screenshot_rgb(shell: &mut Shell, session: &mut Session, width: u32, height: u32) -> Vec<u8> {
    let shot = call(
        shell,
        session,
        "ui.screenshot",
        json!({"width": width, "height": height, "scale": 1.0}),
    );
    let text = shot["base64"].as_str().unwrap();
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
    let mut png = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let bits = chunk
            .iter()
            .filter(|&&c| c != b'=')
            .fold(0_u32, |acc, &c| (acc << 6) | value(c))
            << (6 * pad);
        let bytes = [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8];
        png.extend_from_slice(&bytes[..3 - pad]);
    }
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
        .read_info()
        .unwrap();
    let mut rgb = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgb).unwrap();
    rgb.truncate(info.buffer_size());
    rgb
}

/// Where the grid's ink is: the pixels of `with` that differ from
/// `without` by more than `levels` in a channel.
fn ink(with: &[u8], without: &[u8], levels: u8) -> Vec<bool> {
    with.chunks(3)
        .zip(without.chunks(3))
        .map(|(a, b)| a.iter().zip(b).any(|(x, y)| x.abs_diff(*y) > levels))
        .collect()
}

/// How many of the pixels marked in `of` have no pixel marked in `near`
/// within `reach` pixels.
fn alone(of: &[bool], near: &[bool], width: usize, reach: i64) -> usize {
    let height = of.len() / width;
    (0..of.len())
        .filter(|&i| of[i])
        .filter(|&i| {
            let (x, y) = ((i % width) as i64, (i / width) as i64);
            !(-reach..=reach).any(|dy| {
                (-reach..=reach).any(|dx| {
                    let (nx, ny) = (x + dx, y + dy);
                    nx >= 0
                        && ny >= 0
                        && (nx as usize) < width
                        && (ny as usize) < height
                        && near[ny as usize * width + nx as usize]
                })
            })
        })
        .count()
}

/// The window's own device cuts the grid as the oracle does: with a
/// section across y, the other half and a slanted one, opaque and
/// see-through, the grid's ink of the direct path (what its picture with
/// the sketches has that the one without has not) lies where the
/// oracle's (`ui.screenshot`, the CPU) does, within two pixels, and the
/// other way round: so the GPU draws nothing of the grid on the side cut
/// away, where the oracle has none (the test above).
#[test]
fn the_direct_path_cuts_the_grid_as_the_oracle_does() {
    let state = craft3d_ui_egui::direct::headless_render_state().unwrap_or_else(|e| {
        panic!(
            "the direct path cannot be tested: {e}; {}",
            craft3d_gpu::ADAPTER_HINT
        )
    });
    for (name, origin, normal) in planes() {
        for transparency in [0.0, 0.5] {
            let mut session = ridge_cut_into_a_block();
            let mut shell = Shell::with_renderer(craft3d_ui_egui::RendererChoice::Cpu);
            shell.attach_gpu(&state).unwrap();
            let mut window = Window::new(1000.0, 729.0);
            window.settle(&mut shell, &mut session);
            call(
                &mut shell,
                &mut session,
                "ui.set",
                json!({"view": "iso", "transparency": transparency,
                       "section": {"on": true, "point": [origin.x, origin.y, origin.z],
                                   "normal": [normal.x, normal.y, normal.z]}}),
            );
            let mut pictures = Vec::new();
            for sketches in [true, false] {
                call(
                    &mut shell,
                    &mut session,
                    "ui.set",
                    json!({"show": {"sketches": sketches}}),
                );
                window.settle(&mut shell, &mut session);
                assert!(shell.draws_directly());
                let (w, h, gpu) = shell.direct_picture().unwrap().unwrap();
                let cpu = screenshot_rgb(&mut shell, &mut session, w, h);
                pictures.push((w, gpu, cpu));
            }
            let (w, gpu_with, cpu_with) = &pictures[0];
            let (_, gpu_without, cpu_without) = &pictures[1];
            let width = *w as usize;
            let gpu = ink(gpu_with, gpu_without, 24);
            let cpu = ink(cpu_with, cpu_without, 24);
            let (gpu_ink, cpu_ink) = (
                gpu.iter().filter(|&&p| p).count(),
                cpu.iter().filter(|&&p| p).count(),
            );
            let label = format!("{name}, see-through {transparency}");
            assert!(cpu_ink > 1000, "{label}: {cpu_ink} pixels of the grid");
            let stray = alone(&gpu, &cpu, width, 2);
            let missing = alone(&cpu, &gpu, width, 2);
            println!(
                "{label}: grid ink {gpu_ink} GPU, {cpu_ink} CPU; {stray} GPU pixels with no CPU ink near, {missing} CPU pixels with no GPU ink near"
            );
            assert!(stray * 200 <= gpu_ink, "{label}: {stray} of {gpu_ink}");
            assert!(missing * 50 <= cpu_ink, "{label}: {missing} of {cpu_ink}");
        }
    }
}
