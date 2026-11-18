//! The section's outline seen from close by: the runaway of 2026-11-18.
//! With the section on, turning the view and zooming in on an assembly,
//! the window filled 13 GB in seconds and the kernel ended it: a side of
//! the plane's outline, cut only at the eye plane, ran some 1e9 points
//! past the view, and epaint dashed it in `f32` steps that stop adding up
//! past 2^27 points, so the dashing never ended. Now the outline is dashed
//! only where it shows (`section_handle::dashed_sides`), never in more
//! than `MAX_OUTLINE_DASHES` dashes, and an edge of the CPU path's overlay
//! that reaches far past the image is drawn only where it is over it.
//! Whatever the camera (the one that ran away; a millionth to a million
//! times the fitted distance; the eye in the plane, inside the body;
//! orthographic, zoomed far in) a frame paints a bounded number of shapes
//! in bounded time, counted here, never by real memory.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{SectionPlane, Session, Workspace};
use craft3d_math::{Bbox3, Portable, Vec3};
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::shell::section_handle::{
    DashedSide, MAX_OUTLINE_DASHES, PlaneFrame, dashed_sides, outline_dashes, plane_frame,
};
use craft3d_ui_egui::{CameraState, ProjectionKind, RendererChoice, Shell, Tokens};
use egui::{Pos2, Rect, pos2, vec2};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Most shapes one frame of the window may paint, whatever the camera:
/// the panels, the picture, the edges and the handle of an ordinary frame
/// take some hundreds to a few thousand; before the mend the outline seen
/// from a millimetre took hundreds of thousands of dashes, and from closer
/// it never ended.
const MOST_SHAPES: usize = 20_000;
/// Longest one frame may take (a debug build, the CPU renderer).
const MOST_TIME: Duration = Duration::from_secs(60);
/// Past this many points epaint's dashing in `f32` steps of a dash or a
/// gap (5 and 4 points) no longer moves on: a side this long was never
/// done being dashed.
const NEVER_ENDS: f64 = 134_217_728.0;

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

/// The box of the assembly that ran away (demo04's machine: 520 × 420 ×
/// 488 from (-260, -210, -18)), as one block.
fn machine_box() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [520, 420, 488], "position": [-260, -210, -18], "name": "Machine"}),
    );
    s
}

fn machine_bounds() -> Bbox3 {
    Bbox3::from_points([
        Vec3::new(-260.0, -210.0, -18.0),
        Vec3::new(260.0, 210.0, 470.0),
    ])
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

    /// One frame: how many shapes it painted and how long it took.
    fn frame(&mut self, shell: &mut Shell, session: &mut Session) -> (usize, Duration) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            ..Default::default()
        };
        let started = Instant::now();
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        let took = started.elapsed();
        output.textures_delta.clear();
        (count(&output.shapes), took)
    }

    /// One frame's shapes.
    fn shapes(
        &mut self,
        shell: &mut Shell,
        session: &mut Session,
    ) -> Vec<egui::epaint::ClippedShape> {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| shell.ui(ui, session));
        output.textures_delta.clear();
        output.shapes
    }

    /// `n` frames: the most shapes one of them painted, and the longest
    /// one took.
    fn frames(&mut self, shell: &mut Shell, session: &mut Session, n: usize) -> (usize, Duration) {
        let mut most = (0, Duration::ZERO);
        for _ in 0..n {
            let (shapes, took) = self.frame(shell, session);
            most = (most.0.max(shapes), most.1.max(took));
        }
        most
    }
}

/// The shapes a frame painted, nested ones counted.
fn count(shapes: &[egui::epaint::ClippedShape]) -> usize {
    fn walk(shape: &egui::Shape) -> usize {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().map(walk).sum(),
            _ => 1,
        }
    }
    shapes.iter().map(|clipped| walk(&clipped.shape)).sum()
}

/// The longest side of the outline as it is projected, before it is cut
/// to the view.
fn longest(frame: &PlaneFrame) -> f64 {
    frame
        .outline
        .iter()
        .map(|[a, b]| f64::from(a.distance(*b)))
        .fold(0.0, f64::max)
}

/// The machine's box in a window of 1000 × 700 with the section across zx
/// at offset 0, fitted iso: the shell, the window, the camera fitted.
fn sectioned(session: &mut Session) -> (Shell, Window, CameraState) {
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1000.0, 700.0);
    window.frames(&mut shell, session, 3);
    call(
        &mut shell,
        session,
        "ui.set",
        json!({"section": {"on": true, "plane": "zx", "offset": 0}}),
    );
    call(&mut shell, session, "ui.fit", json!({"view": "iso"}));
    window.frames(&mut shell, session, 3);
    let fitted = shell.state.camera;
    (shell, window, fitted)
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

/// Looking along +y at the section's plane (y = 0) from `distance` in
/// front of it, at the middle of the box: face on.
fn face_on(distance: f64, projection: ProjectionKind) -> CameraState {
    CameraState {
        target: [0.0, 0.0, 226.0],
        distance,
        yaw: -90.0,
        pitch: 0.0,
        projection,
        fov_y: 30.0,
        auto_fit: false,
    }
}

/// A frame at `camera` (the section at `offset`) paints at most
/// [`MOST_SHAPES`] in at most [`MOST_TIME`], and the outline's dashes are
/// few and in view; the longest projected side, when the handle is drawn.
fn bounded_at(
    shell: &mut Shell,
    session: &mut Session,
    window: &mut Window,
    camera: &CameraState,
    offset: f64,
    what: &str,
) -> Option<f64> {
    call(
        shell,
        session,
        "ui.set",
        json!({"camera": camera_json(camera), "section": {"offset": offset}}),
    );
    let (shapes, took) = window.frames(shell, session, 2);
    assert!(
        shapes < MOST_SHAPES,
        "{what}: a frame painted {shapes} shapes"
    );
    assert!(took < MOST_TIME, "{what}: a frame took {took:?}");
    let viewport = shell.viewport_rect().unwrap();
    let place = shell.section_handle()?;
    let dashes = outline_dashes(&place.frame.dashed, Tokens::default().section_dash);
    assert!(dashes.len() <= MAX_OUTLINE_DASHES, "{what}");
    let room = viewport.expand(9.0);
    for [a, b] in &dashes {
        assert!(
            room.contains(*a) && room.contains(*b),
            "{what}: {a:?} {b:?}"
        );
    }
    Some(longest(&place.frame))
}

#[test]
fn seen_from_a_millimetre_the_outline_is_dashed_only_where_it_shows() {
    let mut session = machine_box();
    let (mut shell, mut window, _) = sectioned(&mut session);
    // Face on from a millimetre: the outline's sides run hundreds of
    // thousands of points past the view; dashed whole they took that
    // many shapes over nine points each (fails here before the mend).
    let camera = face_on(1.0, ProjectionKind::Perspective);
    let longest = bounded_at(
        &mut shell,
        &mut session,
        &mut window,
        &camera,
        0.0,
        "face on from 1 mm",
    )
    .expect("the handle, seen face on from 1 mm");
    assert!(longest > 1e5, "{longest}");
    // What is dashed is what shows: here, a millimetre from the middle of
    // the plane, none of its sides, so no dashes at all.
    let place = shell.section_handle().unwrap();
    let dashes = outline_dashes(&place.frame.dashed, Tokens::default().section_dash);
    assert!(dashes.len() < 1000, "{}", dashes.len());
}

#[test]
fn the_camera_that_ran_away_draws_a_bounded_frame() {
    let mut session = machine_box();
    let (mut shell, mut window, fitted) = sectioned(&mut session);
    // The loop that ran away (the coordinator's, 2026-11-18): the yaw on
    // by 3 degrees a step, the distance swinging between 0.2 and 1.8 of
    // the fitted one, every fifth step a new offset. Its longest side is
    // looked for on the frame alone (cheap), then drawn.
    let viewport = shell.viewport_rect().unwrap();
    let tokens = Tokens::default();
    let report = shell.section_report().unwrap();
    let triple = |v: &Value| {
        Vec3::new(
            v[0].as_f64().unwrap(),
            v[1].as_f64().unwrap(),
            v[2].as_f64().unwrap(),
        )
    };
    let (point, normal) = (triple(&report["point"]), triple(&report["normal"]));
    let bounds = machine_bounds();
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
        if let Some(frame) = plane_frame(&camera, viewport, &[], &plane, &bounds, &tokens) {
            let side = longest(&frame);
            if side > worst.0 {
                worst = (side, camera, offset);
            }
        }
    }
    let (side, camera, offset) = worst;
    eprintln!(
        "the loop's longest side of the outline: {side:.3e} points (yaw {:.1}, pitch {:.1}, distance {:.1} of a fitted {:.1}, offset {offset})",
        camera.yaw, camera.pitch, camera.distance, fitted.distance
    );
    // Over a million dashes a frame before the mend; in the window that
    // ran away the loop reached past NEVER_ENDS, where they never end
    // (how far it reaches depends on the fitted distance; the face-on
    // camera of `whatever_the_camera_a_frame_stays_bounded` is past it
    // whatever the window).
    assert!(side > 1e7, "{side:e}");
    bounded_at(
        &mut shell,
        &mut session,
        &mut window,
        &camera,
        offset,
        "the camera that ran away",
    );
}

#[test]
fn whatever_the_camera_a_frame_stays_bounded() {
    let mut session = machine_box();
    let (mut shell, mut window, fitted) = sectioned(&mut session);
    let d = fitted.distance;
    let mut cameras: Vec<(String, CameraState)> = Vec::new();
    for k in [1e-6, 1e-4, 1e-2, 1.0, 1e2, 1e4, 1e6] {
        cameras.push((
            format!("face on at {k:e} of the fitted distance"),
            face_on(k * d, ProjectionKind::Perspective),
        ));
        let mut iso = fitted;
        iso.distance = k * d;
        cameras.push((format!("iso at {k:e} of the fitted distance"), iso));
    }
    // The eye in the plane (y = 0), near and far.
    for k in [1e-6, 1e-2, 0.5] {
        cameras.push((
            format!("the eye in the plane at {k:e}"),
            CameraState {
                target: [0.0, 0.0, 226.0],
                distance: k * d,
                yaw: 0.0,
                pitch: 0.0,
                ..fitted
            },
        ));
    }
    // The eye inside the body, looking about.
    for yaw in [0.0, 70.0, 160.0, 250.0] {
        cameras.push((
            format!("the eye inside the body, yaw {yaw}"),
            CameraState {
                target: [20.0, -100.0, 120.0],
                distance: 5.0,
                yaw,
                pitch: 12.0,
                ..fitted
            },
        ));
    }
    // Orthographic, zoomed far in on the plane and beside it.
    for k in [1e-9, 1e-6, 1e-3] {
        cameras.push((
            format!("orthographic face on at {k:e}"),
            face_on(k * d, ProjectionKind::Orthographic),
        ));
        let mut iso = fitted;
        iso.projection = ProjectionKind::Orthographic;
        iso.distance = k * d;
        cameras.push((format!("orthographic iso at {k:e}"), iso));
    }
    let mut sides = Vec::new();
    for (what, camera) in &cameras {
        let side = bounded_at(&mut shell, &mut session, &mut window, camera, 0.0, what);
        sides.push((what.clone(), side));
    }
    // Face on from a millionth of the fitted distance, a side runs past
    // NEVER_ENDS: there the dashing of old never ended.
    let (what, nearest) = &sides[0];
    let nearest = nearest.unwrap_or_else(|| panic!("{what}: no handle"));
    eprintln!("{what}: the outline's longest side {nearest:.3e} points");
    assert!(nearest > NEVER_ENDS, "{what}: {nearest:e}");
}

#[test]
fn the_dashes_keep_their_place_however_much_of_the_line_is_cut_off() {
    // The outline's rectangle on y = 0, seen face on from 20 mm with its
    // first corner (-281, -39) in view, 4 mm left of and below the middle:
    // its two sides from there run tens of thousands of points past the
    // view. Through a wider view (the same picture, more of it) the lines
    // are cut elsewhere; every dash the narrow view shows well inside it
    // is where the wide one has it.
    let corners = [
        Vec3::new(-281.0, 0.0, -39.0),
        Vec3::new(281.0, 0.0, -39.0),
        Vec3::new(281.0, 0.0, 491.0),
        Vec3::new(-281.0, 0.0, 491.0),
    ];
    let camera = CameraState {
        target: [-277.0, 0.0, -35.0],
        ..face_on(20.0, ProjectionKind::Perspective)
    }
    .to_render();
    let dash = Tokens::default().section_dash;
    let dashes_in = |viewport: Rect| {
        let projector = Projector::new(
            &camera,
            f64::from(viewport.width()),
            f64::from(viewport.height()),
        )
        .unwrap();
        let sides = dashed_sides(&projector, &corners, viewport, dash);
        for side in &sides {
            for end in side.ends {
                assert!(viewport.expand(9.0).contains(end), "{end:?}");
            }
        }
        outline_dashes(&sides, dash)
    };
    let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let wide = Rect::from_min_size(pos2(-200.0, 0.0), vec2(1200.0, 600.0));
    let (seen, more) = (dashes_in(narrow), dashes_in(wide));
    let inner = narrow.shrink(20.0);
    let shown: Vec<&[Pos2; 2]> = seen
        .iter()
        .filter(|[a, b]| inner.contains(*a) && inner.contains(*b))
        .collect();
    assert!(shown.len() > 20, "{}", shown.len());
    for [a, b] in shown {
        assert!(
            more.iter()
                .any(|[c, d]| a.distance(*c) < 1e-2 && b.distance(*d) < 1e-2),
            "{a:?} {b:?} moved"
        );
    }
}

#[test]
fn a_side_of_any_length_takes_at_most_the_cap() {
    // Handed a side however long (here 1e9 points), the dashing stops at
    // the cap, at once; a side that is not a number draws nothing.
    let side = DashedSide {
        ends: [pos2(0.0, 0.0), pos2(1e9, 0.0)],
        phase: 0.0,
    };
    let started = Instant::now();
    let dashes = outline_dashes(&[side; 3], [5.0, 4.0]);
    assert_eq!(dashes.len(), MAX_OUTLINE_DASHES);
    assert!(started.elapsed() < Duration::from_secs(1));
    // The pattern: 5 on, 4 off, from the phase.
    assert_eq!(dashes[0], [pos2(0.0, 0.0), pos2(5.0, 0.0)]);
    assert_eq!(dashes[1], [pos2(9.0, 0.0), pos2(14.0, 0.0)]);
    let late = DashedSide { phase: 7.0, ..side };
    assert_eq!(
        outline_dashes(&[late], [5.0, 4.0])[0],
        [pos2(2.0, 0.0), pos2(7.0, 0.0)]
    );
    let odd = DashedSide {
        ends: [pos2(f32::NAN, 0.0), pos2(f32::INFINITY, 1.0)],
        phase: f32::NAN,
    };
    assert!(outline_dashes(&[odd], [5.0, 4.0]).is_empty());
    // No gap, or a pattern that is none (the review of 2026-11-18: a solid
    // line is friendlier than no outline at all): each side whole.
    for pattern in [
        [0.0, 4.0],
        [f32::NAN, 4.0],
        [5.0, -4.0],
        [f32::INFINITY, 4.0],
    ] {
        assert_eq!(
            outline_dashes(&[side], pattern),
            vec![side.ends],
            "{pattern:?}"
        );
    }
    assert_eq!(outline_dashes(&[side], [5.0, 0.0]), vec![side.ends]);
}

#[test]
fn close_by_nothing_is_painted_off_the_view() {
    // The review of 2026-11-18: close by, the CPU path painted its edges
    // whole in pieces, nearly all of them off the view (77 168 pieces,
    // 76 909 off it, on a grid of blocks), and the cap's outline whole. A
    // piece that cannot show is now left out: one egui's tessellator
    // leaves out anyway (`shell.rs` ›
    // `a_piece_left_out_is_one_the_tessellator_leaves_out`), so the
    // picture is the same. With the section and without.
    let mut session = machine_box();
    let (mut shell, mut window, fitted) = sectioned(&mut session);
    let close = [
        face_on(8.0, ProjectionKind::Perspective),
        CameraState {
            target: [255.0, 205.0, 465.0],
            distance: 8.0,
            yaw: 70.0,
            pitch: 55.0,
            ..fitted
        },
        face_on(fitted.distance * 1e-2, ProjectionKind::Orthographic),
    ];
    for on in [true, false] {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"section": {"on": on}}),
        );
        for camera in &close {
            call(
                &mut shell,
                &mut session,
                "ui.set",
                json!({"camera": camera_json(camera)}),
            );
            window.frames(&mut shell, &mut session, 2);
            let shapes = window.shapes(&mut shell, &mut session);
            let viewport = shell.viewport_rect().unwrap();
            // Past the section's dashes (8 points) and a stroke.
            let room = viewport.expand(12.0);
            fn walk(shape: &egui::Shape, out: &mut Vec<([Pos2; 2], f32)>) {
                match shape {
                    egui::Shape::LineSegment { points, stroke } => {
                        out.push((*points, stroke.width))
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                    _ => {}
                }
            }
            let mut segments = Vec::new();
            for clipped in shapes
                .iter()
                .filter(|c| viewport.contains_rect(c.clip_rect))
            {
                walk(&clipped.shape, &mut segments);
            }
            let off: Vec<_> = segments
                .iter()
                .filter(|([a, b], width)| {
                    !room.intersects(Rect::from_two_pos(*a, *b).expand(*width))
                })
                .collect();
            assert!(
                off.is_empty(),
                "section {on}, {camera:?}: {} of {} segments off the view, as {:?}",
                off.len(),
                segments.len(),
                off.first()
            );
        }
    }
}

#[test]
fn without_a_section_the_edges_are_drawn_as_before() {
    // No section: no handle, and in an ordinary view no edge of the
    // overlay is as long as the cut the overlay's painting makes (four of
    // the image's diagonals), so every one is drawn whole, in the pieces
    // it always was: the frame is the one main drew.
    let mut session = machine_box();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let mut window = Window::new(1000.0, 700.0);
    window.frames(&mut shell, &mut session, 3);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    let (shapes, _) = window.frames(&mut shell, &mut session, 3);
    assert!(shell.section_handle().is_none());
    assert!(shapes < MOST_SHAPES, "{shapes}");
    let (overlay, [w, h]) = shell.overlay();
    assert!(!overlay.lines.is_empty());
    let diagonal = f64::from(w).phypot(f64::from(h));
    for line in &overlay.lines {
        let length = (line.to[0] - line.from[0]).phypot(line.to[1] - line.from[1]);
        assert!(length <= 4.0 * diagonal, "{length} of {diagonal}");
    }
}
