//! What is drawn over the image is tested against the renderer's depth
//! buffer: the edges of a body are cut where a body is in front of them,
//! and a sketch is marked hidden (drawn dimmed) there.
//!
//! The scenes are small enough to say by hand what is in front of what.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Scene, Session, Workspace};
use craft3d_math::Vec3;
use craft3d_ui_egui::Tokens;
use craft3d_ui_egui::camera::Projector;
use craft3d_ui_egui::state::{CameraState, ProjectionKind};
use craft3d_ui_egui::viewport::{self, LineKind, Overlay, OverlayLine, Quality};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

const PROJECTIONS: [ProjectionKind; 2] =
    [ProjectionKind::Perspective, ProjectionKind::Orthographic];

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A block from (-20, -10, -15) to (20, 10, 15), times `scale`, and a
/// sketch on the plane z = `height` · `scale` with a line along x from
/// -60 to 60 (times `scale`) through the middle and a point at its centre.
fn block_and_line(scale: f64, height: f64) -> Arc<Scene> {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"tolerance": 1e-6 * scale}));
    let at = |v: f64| v * scale;
    run(
        &mut s,
        "feature.block",
        json!({"size": [at(40.0), at(20.0), at(30.0)], "position": [at(-20.0), at(-10.0), at(-15.0)]}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Line", "offset": at(height)}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Line", "from": [at(-60.0), 0], "to": [at(60.0), 0]}),
    );
    run(
        &mut s,
        "sketch.point",
        json!({"sketch": "Line", "at": [0, 0]}),
    );
    s.scene(None).unwrap().unwrap()
}

fn camera(
    scene: &Scene,
    yaw: f64,
    pitch: f64,
    projection: ProjectionKind,
    size: [u32; 2],
) -> CameraState {
    let mut camera = CameraState {
        yaw,
        pitch,
        projection,
        ..CameraState::default()
    };
    camera.fit(&scene.bbox, f64::from(size[0]) / f64::from(size[1]));
    camera
}

/// From straight above: x to the right, y up the screen.
fn from_above(scene: &Scene, projection: ProjectionKind, size: [u32; 2]) -> CameraState {
    camera(scene, -90.0, 90.0, projection, size)
}

fn tested(scene: &Scene, camera: &CameraState, size: [u32; 2], ssaa: u32) -> Overlay {
    let quality = Quality {
        width: size[0],
        height: size[1],
        ssaa,
    };
    let frame = viewport::render_bodies(Some(scene), camera, quality, &Tokens::default()).unwrap();
    viewport::overlay_on(Some(scene), camera, &frame, None)
}

fn of_kind(overlay: &Overlay, kind: LineKind) -> Vec<OverlayLine> {
    overlay
        .lines
        .iter()
        .copied()
        .filter(|l| l.kind == kind)
        .collect()
}

fn length(line: &OverlayLine) -> f64 {
    (line.to[0] - line.from[0]).hypot(line.to[1] - line.from[1])
}

/// The overlay with no stretch of an edge marked as an outline: where the
/// lines are and what is hidden, not how they are inked.
fn unmarked(mut overlay: Overlay) -> Overlay {
    for line in &mut overlay.lines {
        if let Some(edge) = &mut line.edge {
            edge.silhouette = false;
        }
    }
    overlay
}

fn evidence(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/m4");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn a_sketch_line_behind_a_block_is_hidden_exactly_where_the_block_covers_it() {
    let size = [400, 300];
    for scale in [1e-3, 1.0, 1e6] {
        for projection in PROJECTIONS {
            for ssaa in [1, 2] {
                // 40 below the origin: under the block, seen from above.
                let scene = block_and_line(scale, -40.0);
                let camera = from_above(&scene, projection, size);
                let what = format!("scale {scale}, {projection:?}, ssaa {ssaa}");
                let overlay = tested(&scene, &camera, size, ssaa);
                let lines = of_kind(&overlay, LineKind::Sketch);
                let flags: Vec<bool> = lines.iter().map(|l| l.hidden).collect();
                assert_eq!(flags, [false, true, false], "{what}: {lines:?}");

                let projector =
                    Projector::new(&camera.to_render(), f64::from(size[0]), f64::from(size[1]))
                        .unwrap();
                let at = |x: f64, z: f64| {
                    projector
                        .project(Vec3::new(x * scale, 0.0, z * scale))
                        .unwrap()
                };
                // The line runs from its own ends, in one piece: where one
                // stretch stops the next begins.
                let (start, end) = (at(-60.0, -40.0), at(60.0, -40.0));
                assert_eq!(lines[0].from, [start.x, start.y], "{what}");
                assert_eq!(lines[2].to, [end.x, end.y], "{what}");
                assert_eq!(lines[0].to, lines[1].from, "{what}");
                assert_eq!(lines[1].to, lines[2].from, "{what}");
                // It disappears under the outline of the block's top face
                // (the part nearest the viewer) and comes out at the other
                // side; within a pixel and a half, which is the pixel the
                // test is made at plus the one next to it.
                let (enters, leaves) = (at(-20.0, 15.0), at(20.0, 15.0));
                assert!(
                    (lines[1].from[0] - enters.x).abs() <= 1.5,
                    "{what}: hidden from {} but the block begins at {}",
                    lines[1].from[0],
                    enters.x
                );
                assert!(
                    (lines[1].to[0] - leaves.x).abs() <= 1.5,
                    "{what}: hidden up to {} but the block ends at {}",
                    lines[1].to[0],
                    leaves.x
                );
                // Horizontal on the screen, through the middle.
                for line in &lines {
                    assert!((line.from[1] - 150.0).abs() < 1e-6, "{what}");
                    assert!((line.to[1] - 150.0).abs() < 1e-6, "{what}");
                }
                // The point at the centre is under the block; the ends of
                // the line are not.
                let hidden: Vec<bool> = overlay.points.iter().map(|p| p.hidden).collect();
                assert_eq!(hidden, [false, false, true], "{what}");

                // Without the depth test nothing is hidden: one line.
                let plain = viewport::overlay(Some(&scene), &camera, size[0], size[1], None);
                let lines = of_kind(&plain, LineKind::Sketch);
                assert_eq!(lines.len(), 1, "{what}");
                assert!(!lines[0].hidden && plain.points.iter().all(|p| !p.hidden));
            }
        }
    }
}

#[test]
fn a_sketch_in_front_of_a_block_or_on_its_face_is_not_hidden() {
    let size = [400, 300];
    for scale in [1e-3, 1.0, 1e6] {
        for projection in PROJECTIONS {
            // Above the block; exactly on its top face; a hair above it.
            for height in [40.0, 15.0, 15.0 + 1e-9] {
                let scene = block_and_line(scale, height);
                // From straight above, and from the directions a part is
                // usually looked at from above.
                for (yaw, pitch) in [(-90.0, 90.0), (-45.0, 29.5), (120.0, 55.0), (-135.0, 8.0)] {
                    for ssaa in [1, 2] {
                        let camera = camera(&scene, yaw, pitch, projection, size);
                        let overlay = tested(&scene, &camera, size, ssaa);
                        let lines = of_kind(&overlay, LineKind::Sketch);
                        let what = format!(
                            "scale {scale}, {projection:?}, height {height}, yaw {yaw}, pitch {pitch}, ssaa {ssaa}"
                        );
                        assert_eq!(lines.len(), 1, "{what}: {lines:?}");
                        assert!(!lines[0].hidden, "{what}");
                        assert!(overlay.points.iter().all(|p| !p.hidden), "{what}");
                    }
                }
            }
            // On the bottom face, seen from above: under the block again.
            let scene = block_and_line(scale, -15.0);
            let camera = from_above(&scene, projection, size);
            let lines = of_kind(&tested(&scene, &camera, size, 1), LineKind::Sketch);
            let flags: Vec<bool> = lines.iter().map(|l| l.hidden).collect();
            assert_eq!(flags, [false, true, false], "scale {scale}, {projection:?}");
            // And seen from below it is the top that is hidden, not this.
            let camera = self::camera(&scene, -90.0, -90.0, projection, size);
            let lines = of_kind(&tested(&scene, &camera, size, 1), LineKind::Sketch);
            assert_eq!(lines.len(), 1, "scale {scale}, {projection:?}");
            assert!(!lines[0].hidden);
        }
    }
}

/// The twelve edges of the block as pairs of corners.
fn block_edges(scale: f64) -> Vec<(Vec3, Vec3)> {
    let corner = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { -20.0 } else { 20.0 },
            if i & 2 == 0 { -10.0 } else { 10.0 },
            if i & 4 == 0 { -15.0 } else { 15.0 },
        ) * scale
    };
    let mut edges = Vec::new();
    for i in 0..8 {
        for bit in [1, 2, 4] {
            if i & bit == 0 {
                edges.push((corner(i), corner(i | bit)));
            }
        }
    }
    edges
}

#[test]
fn the_edges_of_a_block_show_where_they_face_the_viewer_and_not_behind_it() {
    let size = [480, 360];
    for scale in [1e-3, 1.0, 1e6] {
        // Far above the block, out of the way.
        let scene = block_and_line(scale, 500.0);
        let scene = Scene {
            sketches: Vec::new(),
            bbox: scene.mesh.bbox(),
            ..(*scene).clone()
        };
        assert_eq!(scene.bodies[0].edges.len(), 12);
        for projection in PROJECTIONS {
            for (yaw, pitch) in [(-45.0, 29.5), (30.0, 50.0), (140.0, -35.0), (-120.0, 15.0)] {
                for ssaa in [1, 2] {
                    let camera = camera(&scene, yaw, pitch, projection, size);
                    let what = format!(
                        "scale {scale}, {projection:?}, yaw {yaw}, pitch {pitch}, ssaa {ssaa}"
                    );
                    let projector =
                        Projector::new(&camera.to_render(), f64::from(size[0]), f64::from(size[1]))
                            .unwrap();
                    // Seen from a general direction a block shows three
                    // faces: nine edges, all but the three that meet in
                    // the corner furthest from the eye.
                    let edges = block_edges(scale);
                    let furthest = edges
                        .iter()
                        .flat_map(|(a, b)| [*a, *b])
                        .max_by(|a, b| projector.depth(*a).total_cmp(&projector.depth(*b)))
                        .unwrap();
                    let on_screen = |(a, b): &(Vec3, Vec3)| {
                        let (a, b) = (
                            projector.project(*a).unwrap(),
                            projector.project(*b).unwrap(),
                        );
                        (b.x - a.x).hypot(b.y - a.y)
                    };
                    let all: f64 = edges.iter().map(on_screen).sum();
                    let facing: f64 = edges
                        .iter()
                        .filter(|(a, b)| *a != furthest && *b != furthest)
                        .map(on_screen)
                        .sum();
                    assert!(facing < all * 0.9, "{what}: the hidden edges are short");

                    let overlay = tested(&scene, &camera, size, ssaa);
                    let drawn = of_kind(&overlay, LineKind::Edge);
                    assert!(drawn.iter().all(|l| !l.hidden), "{what}");
                    let total: f64 = drawn.iter().map(length).sum();
                    // Each hidden edge may show a pixel or two where it
                    // leaves the corner it shares with visible ones.
                    assert!(
                        total >= facing - 1e-6 && total <= facing + 3.0 * 3.0,
                        "{what}: {total} pixels of edge drawn, {facing} face the viewer, {all} in all"
                    );
                    // Every edge that faces the viewer is there in full:
                    // from corner to corner, in stretches that follow on
                    // from each other (an edge is cut where it stops or
                    // starts being the block's outline, 2026-10-21).
                    for edge in edges
                        .iter()
                        .filter(|(a, b)| *a != furthest && *b != furthest)
                    {
                        let (a, b) = (
                            projector.project(edge.0).unwrap(),
                            projector.project(edge.1).unwrap(),
                        );
                        let same =
                            |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-6;
                        // Walk from one corner along stretches that start
                        // where the last one ended until the other corner.
                        let whole = |from: [f64; 2], to: [f64; 2]| {
                            let mut at = from;
                            for _ in 0..drawn.len() {
                                if same(at, to) {
                                    return true;
                                }
                                let Some(next) = drawn.iter().find(|l| {
                                    same(l.from, at) && !same(l.to, at) && {
                                        // Along the edge, towards `to`.
                                        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                                        let cross =
                                            (l.to[0] - from[0]) * dy - (l.to[1] - from[1]) * dx;
                                        cross.abs() < 1e-6 * dx.hypot(dy).max(1.0)
                                    }
                                }) else {
                                    return false;
                                };
                                at = next.to;
                            }
                            same(at, to)
                        };
                        assert!(
                            whole([a.x, a.y], [b.x, b.y]) || whole([b.x, b.y], [a.x, a.y]),
                            "{what}: the edge {edge:?} is cut or missing"
                        );
                    }

                    // Without the depth test all twelve are drawn: a
                    // wireframe.
                    let plain = viewport::overlay(Some(&scene), &camera, size[0], size[1], None);
                    let lines = of_kind(&plain, LineKind::Edge);
                    assert_eq!(lines.len(), 12, "{what}");
                    let total: f64 = lines.iter().map(length).sum();
                    assert!((total - all).abs() < 1e-6, "{what}");
                }
            }
        }
    }
}

#[test]
fn half_of_the_circle_at_the_back_of_a_cylinder_is_behind_its_wall() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 10, "height": 30, "position": [0, 0, -15]}),
    );
    let scene = s.scene(Some(0.002)).unwrap().unwrap();
    let size = [480, 480];
    // Orthographic, looking down at 40°: the top circle is seen whole,
    // and of the bottom circle the half towards the viewer, which is
    // where the wall ends. Its other half is behind the wall.
    let camera = camera(&scene, -70.0, 40.0, ProjectionKind::Orthographic, size);
    let projector = Projector::new(&camera.to_render(), 480.0, 480.0).unwrap();
    let on_screen = |z: f64| -> f64 {
        let edge = scene.bodies[0].edges.iter().find(|e| e[0].z == z).unwrap();
        edge.windows(2)
            .map(|w| {
                let (a, b) = (
                    projector.project(w[0]).unwrap(),
                    projector.project(w[1]).unwrap(),
                );
                (b.x - a.x).hypot(b.y - a.y)
            })
            .sum()
    };
    let (top, bottom) = (on_screen(15.0), on_screen(-15.0));
    assert!(
        (top - bottom).abs() < 1e-6,
        "orthographic: the same ellipse"
    );
    for ssaa in [1, 2] {
        let overlay = tested(&scene, &camera, size, ssaa);
        let drawn: f64 = of_kind(&overlay, LineKind::Edge).iter().map(length).sum();
        let expected = top + bottom * 0.5;
        assert!(
            (drawn - expected).abs() < 0.02 * expected,
            "ssaa {ssaa}: {drawn} pixels drawn, {expected} expected ({top} per circle)"
        );
    }
}

#[test]
fn the_picture_shows_edges_dark_a_sketch_in_its_colour_and_a_hidden_sketch_dimmed() {
    let tokens = Tokens::default();
    let size = [400, 300];
    let scene = block_and_line(1.0, -40.0);
    let camera = from_above(&scene, ProjectionKind::Orthographic, size);
    let quality = Quality {
        width: size[0],
        height: size[1],
        ssaa: 2,
    };
    let bodies = viewport::render_bodies(Some(&scene), &camera, quality, &tokens).unwrap();
    // Lines three times as wide as in the window, so the pixel on a line
    // is covered whole.
    let picture = viewport::picture(Some(&scene), &camera, quality, &tokens, None, 3.0).unwrap();
    let png = craft3d_render::encode_png(picture.width, picture.height, &picture.rgb).unwrap();
    std::fs::write(evidence("test-sketch-behind-block.png"), png).unwrap();
    let pixel = |rgb: &[u8], x: usize, y: usize| -> [u8; 3] {
        let i = (y * size[0] as usize + x) * 3;
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    };
    let projector = Projector::new(&camera.to_render(), 400.0, 300.0).unwrap();
    let at = |p: Vec3| {
        let p = projector.project(p).unwrap();
        (p.x.floor() as usize, p.y.floor() as usize)
    };
    // On the line beside the block: the sketch colour.
    let (x, y) = at(Vec3::new(-40.0, 0.0, -40.0));
    assert_eq!(pixel(&picture.rgb, x, y), tokens.sketch);
    // On the line under the block, away from the point at its centre and
    // from the X axis, which is drawn from the origin to the right: the
    // body with the sketch colour mixed in at the theme's opacity.
    let (x, y) = at(Vec3::new(-10.0, 0.0, -40.0));
    let body = pixel(&bodies.rgb, x, y);
    assert_ne!(body, tokens.viewport_background, "the block is here");
    let share = f64::from(tokens.hidden_opacity) / 255.0;
    assert!(share > 0.1 && share < 0.5, "dimmed, but there");
    let seen = pixel(&picture.rgb, x, y);
    for k in 0..3 {
        let mixed = f64::from(body[k]) + (f64::from(tokens.sketch[k]) - f64::from(body[k])) * share;
        assert!(
            (f64::from(seen[k]) - mixed).abs() <= 1.0,
            "{seen:?} is not {body:?} with {share} of {:?}",
            tokens.sketch
        );
    }
    // On an edge of the top face, away from the corners: the colour of
    // the block's outline (the background lies beyond it), a shade of the
    // block's own colour (2026-10-21).
    let (x, y) = at(Vec3::new(20.0, 5.0, 15.0));
    assert_eq!(
        pixel(&picture.rgb, x, y),
        tokens.edge_colors(tokens.body).silhouette
    );
    // Beside the edge, on the face: untouched.
    let (x, y) = at(Vec3::new(12.0, 5.0, 15.0));
    assert_eq!(pixel(&picture.rgb, x, y), pixel(&bodies.rgb, x, y));
}

#[test]
fn a_depth_buffer_that_is_wrong_hides_nothing_and_breaks_nothing() {
    let size = [200, 150];
    let scene = block_and_line(1.0, -40.0);
    let camera = from_above(&scene, ProjectionKind::Perspective, size);
    let quality = Quality {
        width: size[0],
        height: size[1],
        ssaa: 1,
    };
    let frame =
        viewport::render_bodies(Some(&scene), &camera, quality, &Tokens::default()).unwrap();
    let plain = viewport::overlay(Some(&scene), &camera, size[0], size[1], None);

    // Too short, too long, empty: not the buffer of this image.
    for cut in [0, 1, frame.depth.len() - 1, frame.depth.len() + 7] {
        let mut broken = frame.clone();
        broken.depth.resize(cut, 1.0);
        let overlay = viewport::overlay_on(Some(&scene), &camera, &broken, None);
        assert_eq!(overlay, plain, "{cut} depths");
    }
    // Depths that are not numbers hide nothing; with nothing drawn beside
    // them every edge is an outline, which changes how it is inked only.
    for bad in [f64::NAN, f64::INFINITY] {
        let mut broken = frame.clone();
        broken.depth.fill(bad);
        let overlay = viewport::overlay_on(Some(&scene), &camera, &broken, None);
        assert!(
            overlay
                .lines
                .iter()
                .filter_map(|line| line.edge)
                .all(|edge| edge.silhouette),
            "{bad}"
        );
        assert_eq!(unmarked(overlay), plain, "{bad}");
    }
    // A surface right at the eye hides everything that can be hidden:
    // the sketch stays, marked; the edges go.
    let mut near = frame.clone();
    near.depth.fill(f64::NEG_INFINITY);
    let overlay = viewport::overlay_on(Some(&scene), &camera, &near, None);
    assert!(of_kind(&overlay, LineKind::Edge).is_empty());
    let sketch = of_kind(&overlay, LineKind::Sketch);
    assert!(!sketch.is_empty() && sketch.iter().all(|l| l.hidden));
    assert_eq!(
        of_kind(&overlay, LineKind::Axis(0)).len(),
        1,
        "the axes stay"
    );
    // Cameras that are no cameras: an empty overlay, no panic.
    for distance in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
        let odd = CameraState { distance, ..camera };
        let _ = viewport::overlay_on(Some(&scene), &odd, &frame, None);
    }
    // No scene at all: the axes.
    let overlay = viewport::overlay_on(None, &camera, &frame, None);
    assert_eq!(overlay.lines.len(), 3);
}
