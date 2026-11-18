//! The review of check/runaway (2026-11-18), through real frames of the
//! shell on the CPU path: the same file runs on main fd13129d (`ON_MAIN`
//! true, where a frame whose section outline main would dash in more than
//! `MAIN_SAFE_DASHES` dashes is skipped, so nothing runs away on the build
//! server) and on the branch, so their numbers compare line by line:
//! - without a section, the viewport's tessellated shapes and textures
//!   hashed, at ordinary cameras (the same on both, byte for byte);
//! - edges seen from close by, longer than four of the image's diagonals:
//!   covered where they cross the image, how many pieces, the fade along;
//! - with a section, frames at the cameras that ran away, timed;
//! - a 500-step path through the plane and into the body and out, with the
//!   plain and the studio look, perspective and orthographic: every frame
//!   counted and timed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SectionPlane, Session, Workspace};
use craft3d_math::{Bbox3, Portable, Vec3};
use craft3d_ui_egui::shell::section_handle::plane_frame;
use craft3d_ui_egui::{CameraState, ProjectionKind, RendererChoice, Shell, Tokens};
use egui::epaint::{ClippedShape, ImageData, Primitive};
use egui::{Pos2, Rect, Shape, vec2};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// True in the copy of this file run on main.
const ON_MAIN: bool = false;
/// Most dashes main is let draw in a frame here.
const MAIN_SAFE_DASHES: f64 = 6e5;
/// Bounds held on the branch, every frame.
const MOST_SHAPES: usize = 20_000;
const MOST_VERTICES: usize = 1_000_000;
const MOST_TIME: Duration = Duration::from_secs(30);
/// Longest a variant of the path may run before it stops (and says so).
const PATH_BUDGET: Duration = Duration::from_secs(240);

/// What broke a bound: said at once, failed at the end (so one failure
/// does not hide the rest of the numbers).
static BROKEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn hold(ok: bool, what: impl FnOnce() -> String) {
    if !ok {
        let what = what();
        println!("BROKEN {what}");
        BROKEN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(what);
    }
}

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

/// FNV-1a over 64-bit words.
struct Hash(u64);

impl Hash {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn add(&mut self, word: u64) {
        for byte in word.to_le_bytes() {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn f32(&mut self, v: f32) {
        self.add(u64::from(v.to_bits()));
    }
}

fn rss() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .map(|status| {
            status
                .lines()
                .filter(|l| l.starts_with("VmRSS") || l.starts_with("VmHWM"))
                .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// What one frame painted.
struct Shot {
    shapes: usize,
    vertices: usize,
    took: Duration,
    /// The viewport's shapes, tessellated, and the textures set (but the
    /// font atlas), hashed.
    view: u64,
    textures: u64,
    /// The viewport's vertices without their texture places (positions
    /// and colours), their texture places alone, and the line segments
    /// painted (ends, width, colour), hashed apart.
    places: u64,
    uvs: u64,
    lines: u64,
    /// The line segments painted, with their alpha.
    segments: Vec<([Pos2; 2], u8)>,
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

    fn frame(&mut self, shell: &mut Shell, session: &mut Session) -> Shot {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            ..Default::default()
        };
        let started = Instant::now();
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        let ppp = output.pixels_per_point;
        let shapes = count(&output.shapes);
        let all = self.ctx.tessellate(output.shapes.clone(), ppp);
        let took = started.elapsed();
        let vertices = all
            .iter()
            .map(|p| match &p.primitive {
                Primitive::Mesh(mesh) => mesh.vertices.len(),
                Primitive::Callback(_) => 0,
            })
            .sum();
        let viewport = shell.viewport_rect().unwrap_or(Rect::NOTHING);
        let room = viewport.expand(0.5);
        let in_view: Vec<ClippedShape> = output
            .shapes
            .iter()
            .filter(|c| room.contains_rect(c.clip_rect))
            .cloned()
            .collect();
        let mut view = Hash::new();
        let (mut places, mut uvs) = (Hash::new(), Hash::new());
        for primitive in self.ctx.tessellate(in_view, ppp) {
            for c in [
                primitive.clip_rect.min.x,
                primitive.clip_rect.min.y,
                primitive.clip_rect.max.x,
                primitive.clip_rect.max.y,
            ] {
                view.f32(c);
            }
            if let Primitive::Mesh(mesh) = primitive.primitive {
                view.add(match mesh.texture_id {
                    egui::TextureId::Managed(k) => k,
                    egui::TextureId::User(k) => k | (1 << 63),
                });
                for v in &mesh.vertices {
                    places.f32(v.pos.x);
                    places.f32(v.pos.y);
                    places.add(u64::from(u32::from_le_bytes(v.color.to_array())));
                    uvs.f32(v.uv.x);
                    uvs.f32(v.uv.y);
                    view.f32(v.pos.x);
                    view.f32(v.pos.y);
                    view.f32(v.uv.x);
                    view.f32(v.uv.y);
                    view.add(u64::from(u32::from_le_bytes(v.color.to_array())));
                }
                for &i in &mesh.indices {
                    view.add(u64::from(i));
                }
            }
        }
        let mut textures = Hash::new();
        let mut set: Vec<_> = output
            .textures_delta
            .set
            .iter()
            .filter(|(id, _)| **id != egui::TextureId::default())
            .collect();
        set.sort_by_key(|(id, _)| **id);
        for (_, deltas) in set {
            for delta in deltas {
                let ImageData::Color(image) = &delta.image;
                textures.add(image.size[0] as u64);
                textures.add(image.size[1] as u64);
                for pixel in &image.pixels {
                    textures.add(u64::from(u32::from_le_bytes(pixel.to_array())));
                }
            }
        }
        let mut segments = Vec::new();
        let mut lines = Hash::new();
        for clipped in &output.shapes {
            collect_segments(&clipped.shape, &mut segments);
            hash_lines(&clipped.shape, &mut lines);
        }
        // Handled: egui asserts that a delta is not dropped unread.
        output.textures_delta.clear();
        Shot {
            shapes,
            vertices,
            took,
            view: view.0,
            textures: textures.0,
            places: places.0,
            uvs: uvs.0,
            lines: lines.0,
            segments,
        }
    }
}

fn count(shapes: &[ClippedShape]) -> usize {
    fn walk(shape: &Shape) -> usize {
        match shape {
            Shape::Vec(shapes) => shapes.iter().map(walk).sum(),
            _ => 1,
        }
    }
    shapes.iter().map(|c| walk(&c.shape)).sum()
}

fn hash_lines(shape: &Shape, hash: &mut Hash) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| hash_lines(s, hash)),
        Shape::LineSegment { points, stroke } => {
            for p in points {
                hash.f32(p.x);
                hash.f32(p.y);
            }
            hash.f32(stroke.width);
            hash.add(u64::from(u32::from_le_bytes(stroke.color.to_array())));
        }
        _ => {}
    }
}

fn collect_segments(shape: &Shape, out: &mut Vec<([Pos2; 2], u8)>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| collect_segments(s, out)),
        Shape::LineSegment { points, stroke } => out.push((*points, stroke.color.a())),
        _ => {}
    }
}

fn camera_json(camera: &CameraState) -> Value {
    json!({
        "target": camera.target,
        "distance": camera.distance,
        "yaw": camera.yaw,
        "pitch": camera.pitch,
        "projection": match camera.projection {
            ProjectionKind::Perspective => "perspective",
            ProjectionKind::Orthographic => "orthographic",
        },
        "fovY": camera.fov_y,
        "autoFit": false,
    })
}

/// A document, its box, and the shell and window showing it fitted iso.
struct Scene {
    session: Session,
    bounds: Bbox3,
    shell: Shell,
    window: Window,
    fitted: CameraState,
    /// The section's plane at offset 0: its point and normal.
    plane: Option<(Vec3, Vec3)>,
}

impl Scene {
    fn new(session: Session, bounds: Bbox3, width: f32, height: f32) -> Self {
        let mut shell = Shell::with_renderer(RendererChoice::Cpu);
        let mut window = Window::new(width, height);
        let mut session = session;
        for _ in 0..3 {
            window.frame(&mut shell, &mut session);
        }
        call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
        for _ in 0..3 {
            window.frame(&mut shell, &mut session);
        }
        let fitted = shell.state.camera;
        Self {
            session,
            bounds,
            shell,
            window,
            fitted,
            plane: None,
        }
    }

    fn set(&mut self, params: Value) {
        call(&mut self.shell, &mut self.session, "ui.set", params);
    }

    fn section(&mut self, on: bool) {
        self.set(json!({"section": {"on": on, "plane": "zx", "offset": 0}}));
        self.window.frame(&mut self.shell, &mut self.session);
        self.plane = on.then(|| {
            let report = self.shell.section_report().unwrap();
            let v = |v: &Value| {
                Vec3::new(
                    v[0].as_f64().unwrap(),
                    v[1].as_f64().unwrap(),
                    v[2].as_f64().unwrap(),
                )
            };
            (v(&report["point"]), v(&report["normal"]))
        });
    }

    /// How many dashes main would draw for the outline at `camera` with
    /// the plane at `offset`, and its longest side (points).
    fn main_dashes(&self, camera: &CameraState, offset: f64) -> (f64, f64) {
        let (Some((point, normal)), Some(viewport)) = (self.plane, self.shell.viewport_rect())
        else {
            return (0.0, 0.0);
        };
        let tokens = Tokens::default();
        let plane = SectionPlane {
            origin: point + normal * offset,
            normal,
            base: point,
            along: normal,
        };
        let period = f64::from(tokens.section_dash[0] + tokens.section_dash[1]);
        plane_frame(camera, viewport, &[], &plane, &self.bounds, &tokens).map_or(
            (0.0, 0.0),
            |frame| {
                let lengths: Vec<f64> = frame
                    .outline
                    .iter()
                    .map(|[a, b]| f64::from(a.distance(*b)))
                    .collect();
                (
                    lengths.iter().sum::<f64>() / period,
                    lengths.iter().copied().fold(0.0, f64::max),
                )
            },
        )
    }

    /// A frame at `camera` (the plane at `offset` when the section is on),
    /// after one to put the picture in place; `None` on main where main
    /// would dash too much.
    fn shot(&mut self, camera: &CameraState, offset: f64, what: &str) -> Option<Shot> {
        let (dashes, longest) = self.main_dashes(camera, offset);
        if ON_MAIN && (dashes > MAIN_SAFE_DASHES || longest > 1e7) {
            println!(
                "{what}: skipped on main (it would dash {dashes:.3e} dashes, a side of {longest:.3e} points)"
            );
            return None;
        }
        let mut params = json!({"camera": camera_json(camera)});
        if self.plane.is_some() {
            params["section"] = json!({"offset": offset});
        }
        self.set(params);
        // The picture is made (and its texture set) in the first frame;
        // the second shows it as it stays.
        let first = self.window.frame(&mut self.shell, &mut self.session);
        let mut shot = self.window.frame(&mut self.shell, &mut self.session);
        let mut both = Hash::new();
        both.add(first.textures);
        both.add(shot.textures);
        shot.textures = both.0;
        shot.took = shot.took.max(first.took);
        println!(
            "{what}: {} shapes, {} vertices, {:.1} ms; view {:016x} textures {:016x}; main would dash {dashes:.3e}",
            shot.shapes,
            shot.vertices,
            shot.took.as_secs_f64() * 1e3,
            shot.view,
            shot.textures
        );
        if !ON_MAIN {
            hold(shot.shapes < MOST_SHAPES, || {
                format!("{what}: {} shapes", shot.shapes)
            });
            hold(shot.vertices < MOST_VERTICES, || {
                format!("{what}: {} vertices", shot.vertices)
            });
            hold(shot.took < MOST_TIME, || format!("{what}: {:?}", shot.took));
        }
        Some(shot)
    }
}

fn machine() -> (Session, Bbox3) {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [520, 420, 488], "position": [-260, -210, -18], "name": "Machine"}),
    );
    let bounds = Bbox3::from_points([
        Vec3::new(-260.0, -210.0, -18.0),
        Vec3::new(260.0, 210.0, 470.0),
    ]);
    (s, bounds)
}

/// 162 blocks of 30 in a grid of 18 by 9, 40 apart, round the origin.
fn grid() -> (Session, Bbox3) {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [30, 30, 30], "position": [-355, -175, 0], "name": "Cell"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Cell", "count": 18, "step": [40, 0, 0], "name": "Row"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Row", "count": 9, "step": [0, 40, 0], "name": "Grid"}),
    );
    let bounds = Bbox3::from_points([
        Vec3::new(-355.0, -175.0, 0.0),
        Vec3::new(355.0, 175.0, 30.0),
    ]);
    (s, bounds)
}

/// Ordinary cameras: fitted iso, turned round, nearer and further, and
/// orthographic.
fn ordinary(fitted: &CameraState) -> Vec<(String, CameraState)> {
    let mut out = vec![("fitted".to_string(), *fitted)];
    for k in 0..8 {
        out.push((
            format!("yaw {}", 45 * k),
            CameraState {
                yaw: 45.0 * f64::from(k),
                pitch: 25.0,
                ..*fitted
            },
        ));
    }
    for scale in [0.6, 2.0] {
        out.push((
            format!("{scale} × fitted"),
            CameraState {
                distance: fitted.distance * scale,
                ..*fitted
            },
        ));
    }
    out.push((
        "orthographic".to_string(),
        CameraState {
            projection: ProjectionKind::Orthographic,
            ..*fitted
        },
    ));
    out
}

/// Liang-Barsky in `f64`: where the segment `a`-`b` is inside the box.
fn clip(a: [f64; 2], b: [f64; 2], lo: [f64; 2], hi: [f64; 2]) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for axis in 0..2 {
        let (start, delta) = (a[axis], b[axis] - a[axis]);
        if delta == 0.0 {
            if start < lo[axis] || start > hi[axis] {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = ((lo[axis] - start) / delta, (hi[axis] - start) / delta);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

/// The overlay's edges longer than four of the image's diagonals in the
/// last frame: how many, the largest stretch of one over the image that no
/// segment painted covers (points), how many segments were painted along
/// them, and the alphas at eight places along the first.
fn long_edges(shell: &Shell, shot: &Shot) -> (usize, f64, usize, Vec<u8>) {
    let Some(rect) = shell.viewport_rect() else {
        return (0, 0.0, 0, Vec::new());
    };
    let (overlay, [w, h]) = shell.overlay();
    let (wf, hf) = (f64::from(w), f64::from(h));
    let diagonal = wf.phypot(hf);
    let (sx, sy) = (
        f64::from(rect.width()) / wf.max(1.0),
        f64::from(rect.height()) / hf.max(1.0),
    );
    let place = |p: [f64; 2]| {
        [
            f64::from(rect.left()) + p[0] * sx,
            f64::from(rect.top()) + p[1] * sy,
        ]
    };
    let (mut lines, mut worst_gap, mut pieces) = (0, 0.0_f64, 0);
    let mut alphas = Vec::new();
    for line in overlay.lines.iter().filter(|l| l.edge.is_some()) {
        let length = (line.to[0] - line.from[0]).phypot(line.to[1] - line.from[1]);
        if !(length > 4.0 * diagonal && length.is_finite()) {
            continue;
        }
        let Some((v0, v1)) = clip(line.from, line.to, [0.0, 0.0], [wf, hf]) else {
            continue;
        };
        lines += 1;
        let (a, b) = (place(line.from), place(line.to));
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let long2 = dx * dx + dy * dy;
        let long = long2.sqrt();
        let along = |p: Pos2| {
            let (px, py) = (f64::from(p.x) - a[0], f64::from(p.y) - a[1]);
            let t = (px * dx + py * dy) / long2;
            let off = (px * dy - py * dx).abs() / long;
            (t, off)
        };
        let mut covered: Vec<(f64, f64, u8)> = shot
            .segments
            .iter()
            .filter_map(|([p, q], alpha)| {
                let ((tp, op), (tq, oq)) = (along(*p), along(*q));
                (op < 0.25 && oq < 0.25).then_some((tp.min(tq), tp.max(tq), *alpha))
            })
            .collect();
        pieces += covered.len();
        covered.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut reached = v0;
        for (t0, t1, _) in &covered {
            if *t0 > reached {
                worst_gap = worst_gap.max((t0.min(v1) - reached) * long);
            }
            reached = reached.max(*t1);
            if reached >= v1 {
                break;
            }
        }
        if reached < v1 {
            worst_gap = worst_gap.max((v1 - reached) * long);
        }
        if alphas.is_empty() {
            for k in 0..8 {
                let t = v0 + (v1 - v0) * (f64::from(k) + 0.5) / 8.0;
                alphas.push(
                    covered
                        .iter()
                        .find(|(t0, t1, _)| *t0 <= t && t <= *t1)
                        .map_or(0, |c| c.2),
                );
            }
        }
    }
    (lines, worst_gap, pieces, alphas)
}

/// The camera of the loop that ran away (the coordinator's, 2026-11-18)
/// with the longest projected side of the outline, found on the frame
/// alone, and its offset.
fn runaway_camera(scene: &Scene) -> (CameraState, f64, f64) {
    let (point, normal) = scene.plane.unwrap();
    let viewport = scene.shell.viewport_rect().unwrap();
    let tokens = Tokens::default();
    let fitted = scene.fitted;
    let mut worst = (0.0, fitted, 0.0);
    let mut offset = 0.0;
    for n in 0..3000_u32 {
        if n % 5 == 0 {
            offset = -300.0 + f64::from((n * 11) % 600);
        }
        let mut camera = fitted;
        camera.yaw = (fitted.yaw + 3.0 * f64::from(n)).rem_euclid(360.0);
        camera.distance = fitted.distance * (1.0 + 0.8 * (f64::from(n) / 9.0).psin());
        let plane = SectionPlane {
            origin: point + normal * offset,
            normal,
            base: point,
            along: normal,
        };
        if let Some(frame) = plane_frame(&camera, viewport, &[], &plane, &scene.bounds, &tokens) {
            let side = frame
                .outline
                .iter()
                .map(|[a, b]| f64::from(a.distance(*b)))
                .fold(0.0, f64::max);
            if side > worst.0 {
                worst = (side, camera, offset);
            }
        }
    }
    (worst.1, worst.2, worst.0)
}

/// Looking along +y at the plane y = 0 from `distance` in front of it.
fn face_on(target: [f64; 3], distance: f64, projection: ProjectionKind) -> CameraState {
    CameraState {
        target,
        distance,
        yaw: -90.0,
        pitch: 0.0,
        projection,
        fov_y: 30.0,
        auto_fit: false,
    }
}

/// Without a section, at ordinary cameras: the hashes, which main and the
/// branch must share; and whether an edge is long enough for the new rule
/// (it should not be).
fn no_section(scene: &mut Scene, name: &str) {
    let fitted = scene.fitted;
    for (what, camera) in ordinary(&fitted) {
        let what = format!("{name}, no section, {what}");
        if let Some(shot) = scene.shot(&camera, 0.0, &what) {
            let (long, _, _, _) = long_edges(&scene.shell, &shot);
            println!(
                "HASH {what}: view {:016x} textures {:016x} long-edges {long}",
                shot.view, shot.textures
            );
        }
    }
}

/// Without a section, from close by a corner of `corner`: the long edges
/// are covered where they cross the image, in how many pieces.
fn close_edges(scene: &mut Scene, name: &str, corner: [f64; 3]) {
    for distance in [400.0, 120.0, 40.0, 8.0] {
        for (yaw, pitch) in [(45.0, 35.0), (20.0, 10.0), (70.0, 55.0)] {
            let camera = CameraState {
                target: corner,
                distance,
                yaw,
                pitch,
                projection: ProjectionKind::Perspective,
                fov_y: 30.0,
                auto_fit: false,
            };
            let what = format!("{name}, edges from {distance} mm, yaw {yaw}, pitch {pitch}");
            if let Some(shot) = scene.shot(&camera, 0.0, &what) {
                let (lines, gap, pieces, alphas) = long_edges(&scene.shell, &shot);
                println!(
                    "LONG {what}: {lines} edges past 4 diagonals, {pieces} segments along them, \
                     largest gap {gap:.3} points; alphas along the first {alphas:?}; view {:016x}",
                    shot.view
                );
                hold(gap < 0.5, || {
                    format!("{what}: a gap of {gap} points in a long edge")
                });
            }
        }
    }
}

/// With a section: ordinary cameras, then the ones that ran away.
fn with_section(scene: &mut Scene, name: &str, middle: [f64; 3]) {
    scene.section(true);
    let fitted = scene.fitted;
    for (what, camera) in ordinary(&fitted) {
        scene.shot(&camera, 0.0, &format!("{name}, section, {what}"));
    }
    let d = fitted.distance;
    for k in [1e-6, 1e-4, 1e-3, 1e-2, 1e-1] {
        for projection in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
            scene.shot(
                &face_on(middle, k * d, projection),
                0.0,
                &format!("{name}, section, face on at {k:e} of {d:.1}, {projection:?}"),
            );
        }
    }
    // Face on from a millimetre: the author's "about 2.8e5 dashes before".
    scene.shot(
        &face_on(middle, 1.0, ProjectionKind::Perspective),
        0.0,
        &format!("{name}, section, face on from 1 mm"),
    );
    let (camera, offset, side) = runaway_camera(scene);
    scene.shot(
        &camera,
        offset,
        &format!(
            "{name}, section, the loop's worst camera (side {side:.3e} points, yaw {:.1}, distance {:.1}, offset {offset})",
            camera.yaw, camera.distance
        ),
    );
    println!("{name}: {}", rss());
}

/// The developer's use: the section on, the view turned twice round while
/// it zooms from three times the fitted distance to a millionth of it and
/// back, the target swinging through the plane and into the body, the
/// plane moving; every frame bounded in shapes, vertices and time.
fn path(scene: &mut Scene, name: &str, studio: bool, projection: ProjectionKind) {
    scene.set(json!({"look": {"mode": if studio { "studio" } else { "plain" }}}));
    if studio {
        scene.shell.prepare_look().unwrap();
    }
    let fitted = scene.fitted;
    let centre = fitted.target;
    let steps = 500_u32;
    let started = Instant::now();
    let (mut done, mut skipped) = (0, 0);
    let (mut times, mut first): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    let (mut most_shapes, mut most_vertices) = (0, 0);
    let mut worst_at = String::new();
    for n in 0..steps {
        if started.elapsed() > PATH_BUDGET {
            println!("{name}: stopped after {n} of {steps} steps, {PATH_BUDGET:?} spent");
            break;
        }
        let s = f64::from(n) / f64::from(steps - 1);
        let tau = std::f64::consts::TAU;
        // log10 of the distance over the fitted one: 0.5 down to -6 and back.
        let level = 0.5 - 13.0 * (0.5 - (s - 0.5).abs());
        let camera = CameraState {
            target: [
                centre[0] + 120.0 * (tau * 3.0 * s).psin(),
                centre[1] + 300.0 * (tau * s).psin(),
                centre[2] + 100.0 * (tau * 2.0 * s).psin(),
            ],
            distance: fitted.distance * 10.0_f64.ppowf(level),
            yaw: fitted.yaw + 720.0 * s,
            pitch: (70.0 * (tau * 1.5 * s).psin()).clamp(-85.0, 85.0),
            projection,
            fov_y: 30.0,
            auto_fit: false,
        };
        let offset = 150.0 * (tau * 2.5 * s).psin();
        let (dashes, longest) = scene.main_dashes(&camera, offset);
        if ON_MAIN && (dashes > MAIN_SAFE_DASHES || longest > 1e7) {
            skipped += 1;
            continue;
        }
        scene.set(json!({"camera": camera_json(&camera), "section": {"offset": offset}}));
        let shot = scene.window.frame(&mut scene.shell, &mut scene.session);
        done += 1;
        // The first frames of a look prepare it: timed apart.
        if done <= 3 {
            first.push(shot.took.as_secs_f64() * 1e3);
        } else {
            times.push(shot.took.as_secs_f64() * 1e3);
        }
        if shot.shapes > most_shapes {
            most_shapes = shot.shapes;
            worst_at = format!(
                "step {n}: distance {:.3e}, yaw {:.1}, pitch {:.1}, offset {offset:.1}, main would dash {dashes:.3e}",
                camera.distance, camera.yaw, camera.pitch
            );
        }
        most_vertices = most_vertices.max(shot.vertices);
        if !ON_MAIN {
            hold(shot.shapes < MOST_SHAPES, || {
                format!("{name} step {n}: {} shapes", shot.shapes)
            });
            hold(shot.vertices < MOST_VERTICES, || {
                format!("{name} step {n}: {} vertices", shot.vertices)
            });
            hold(done <= 3 || shot.took < MOST_TIME, || {
                format!("{name} step {n}: {:?}", shot.took)
            });
        }
    }
    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| {
        sorted
            .get(((sorted.len() as f64 - 1.0) * q).round() as usize)
            .copied()
            .unwrap_or(0.0)
    };
    println!(
        "PATH {name}: {done} frames ({skipped} skipped) in {:.1} s; ms per frame median {:.1}, \
         p95 {:.1}, max {:.1} (the first three {first:.1?}); most shapes {most_shapes} ({worst_at}), \
         most vertices {most_vertices}; {}",
        started.elapsed().as_secs_f64(),
        at(0.5),
        at(0.95),
        at(1.0),
        rss()
    );
    if !ON_MAIN {
        // No frame far slower than the rest.
        hold(at(1.0) < 10.0 * at(0.5) + 1000.0, || {
            format!(
                "{name}: a frame of {:.1} ms against a median of {:.1}",
                at(1.0),
                at(0.5)
            )
        });
    }
}

#[test]
fn the_frames_of_the_review() {
    println!("ON_MAIN {ON_MAIN}");
    let (session, bounds) = machine();
    let mut scene = Scene::new(session, bounds, 1000.0, 700.0);
    println!("machine: fitted {:?}", scene.fitted);
    no_section(&mut scene, "machine");
    close_edges(&mut scene, "machine", [260.0, 210.0, 470.0]);
    println!("machine, no section: {}", rss());
    with_section(&mut scene, "machine", [0.0, 0.0, 226.0]);
    for (studio, projection) in [
        (false, ProjectionKind::Perspective),
        (false, ProjectionKind::Orthographic),
        (true, ProjectionKind::Perspective),
        (true, ProjectionKind::Orthographic),
    ] {
        let name = format!(
            "machine, {} look, {projection:?}",
            if studio { "studio" } else { "plain" }
        );
        path(&mut scene, &name, studio, projection);
    }
    drop(scene);

    let built = Instant::now();
    let (session, bounds) = grid();
    let mut scene = Scene::new(session, bounds, 1000.0, 700.0);
    println!(
        "grid of 162 blocks built and fitted in {:.1} s: fitted {:?}",
        built.elapsed().as_secs_f64(),
        scene.fitted
    );
    no_section(&mut scene, "grid");
    close_edges(&mut scene, "grid", [355.0, 175.0, 30.0]);
    with_section(&mut scene, "grid", [0.0, 0.0, 15.0]);
    path(
        &mut scene,
        "grid, plain look, Perspective",
        false,
        ProjectionKind::Perspective,
    );
    let broken = BROKEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        broken.is_empty(),
        "{} bounds broken: {broken:#?}",
        broken.len()
    );
}

/// Without a section, at the ordinary cameras, twice over in two fresh
/// windows: the hashes apart (the overlay's segments, the viewport's
/// vertex places, their texture places, the CPU picture), so main and the
/// branch compare part by part and a hash that is not the same from one
/// window to the next shows itself. Then close by the grid: how many of
/// the segments painted could show at all.
#[test]
fn no_section_hashes() {
    for round in 0..2 {
        for (name, (session, bounds)) in [("machine", machine()), ("grid", grid())] {
            let mut scene = Scene::new(session, bounds, 1000.0, 700.0);
            let fitted = scene.fitted;
            for (what, camera) in ordinary(&fitted) {
                let what = format!("{name}, no section, {what}");
                if let Some(shot) = scene.shot(&camera, 0.0, &what) {
                    println!(
                        "PARTS {round} {what}: lines {:016x} places {:016x} uvs {:016x} textures {:016x}",
                        shot.lines, shot.places, shot.uvs, shot.textures
                    );
                }
            }
        }
    }
    let (session, bounds) = grid();
    let mut scene = Scene::new(session, bounds, 1000.0, 700.0);
    let close = [
        (
            "8 mm, yaw 70, pitch 55",
            CameraState {
                target: [355.0, 175.0, 30.0],
                distance: 8.0,
                yaw: 70.0,
                pitch: 55.0,
                projection: ProjectionKind::Perspective,
                fov_y: 30.0,
                auto_fit: false,
            },
        ),
        (
            "orthographic face on at 1e-2 of the fitted distance",
            face_on(
                [0.0, 0.0, 15.0],
                scene.fitted.distance * 1e-2,
                ProjectionKind::Orthographic,
            ),
        ),
    ];
    for (what, camera) in close {
        let Some(shot) = scene.shot(&camera, 0.0, &format!("grid close, {what}")) else {
            continue;
        };
        let room = scene.shell.viewport_rect().unwrap().expand(2.0);
        let (lo, hi) = (
            [f64::from(room.left()), f64::from(room.top())],
            [f64::from(room.right()), f64::from(room.bottom())],
        );
        let unseen = shot
            .segments
            .iter()
            .filter(|([a, b], _)| {
                clip(
                    [f64::from(a.x), f64::from(a.y)],
                    [f64::from(b.x), f64::from(b.y)],
                    lo,
                    hi,
                )
                .is_none()
            })
            .count();
        println!(
            "UNSEEN grid close, {what}: {} segments painted, {unseen} of them wholly outside the viewport",
            shot.segments.len()
        );
    }
}
