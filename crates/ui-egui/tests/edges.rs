//! The edges of the bodies in the picture, on the CPU (the oracle; the
//! GPU's viewport is held to it in `gpu_viewport.rs`): each in a shade of
//! its own body's colour that contrasts with that colour, darker on a
//! light or an ordinary body and lighter on a very dark one; the outline
//! of a body against what lies behind it wider and darker than the edges
//! between its faces; and edges faded where they are short on screen or
//! where they crowd, crisp again up close.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Scene, Session, Workspace};
use craft3d_render::NO_FACE;
use craft3d_ui_egui::Tokens;
use craft3d_ui_egui::state::{CameraState, ProjectionKind};
use craft3d_ui_egui::theme::{contrast, luminance};
use craft3d_ui_egui::viewport::{self, Composed, OverlayLine, Picture, Quality};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

const LIGHT: [u8; 3] = [233, 230, 223];
const DARK: [u8; 3] = [29, 29, 32];
const RED: [u8; 3] = [156, 47, 37];

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Three blocks side by side, light, very dark and red, the middle one
/// taller, and one in the theme's colour behind them.
fn blocks() -> Arc<Scene> {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for (name, x, height, color) in [
        ("Light", 0, 30, Some(LIGHT)),
        ("Dark", 40, 45, Some(DARK)),
        ("Red", 80, 30, Some(RED)),
        ("Plain", 40, 20, None),
    ] {
        let y = if color.is_some() { 0 } else { 60 };
        run(
            &mut s,
            "feature.block",
            json!({"name": name, "size": [30, 30, height], "position": [x, y, 0]}),
        );
        if let Some(color) = color {
            run(
                &mut s,
                "body.appearance",
                json!({"body": name, "color": color}),
            );
        }
    }
    s.scene(None).unwrap().unwrap()
}

fn evidence(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/edges");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn write_png(name: &str, picture: &Picture) {
    let path = evidence(name);
    let png = craft3d_render::encode_png(picture.width, picture.height, &picture.rgb).unwrap();
    std::fs::write(&path, png).unwrap();
    println!("evidence: {}", path.display());
}

fn camera(scene: &Scene, projection: ProjectionKind, size: [u32; 2]) -> CameraState {
    let mut camera = CameraState {
        projection,
        ..CameraState::default()
    };
    camera.fit(&scene.bbox, f64::from(size[0]) / f64::from(size[1]));
    camera
}

/// Wide lines, so the pixel nearest a line's centre is all ink (with
/// the fades off: one line this wide is a crowd for the density).
fn wide() -> Tokens {
    Tokens {
        edge_width: 3.0,
        edge_silhouette_width: 5.0,
        ..Tokens::default()
    }
}

/// Tokens with every fade off.
fn unfaded(mut tokens: Tokens) -> Tokens {
    tokens.edge_fade_short = 0.0;
    tokens.edge_fade_long = 0.0;
    tokens.edge_dense_cap = 1e9;
    tokens.edge_fade_floor = 1.0;
    tokens
}

fn compose(scene: &Scene, camera: &CameraState, size: [u32; 2], tokens: &Tokens) -> Composed {
    let quality = Quality {
        width: size[0],
        height: size[1],
        ssaa: 2,
    };
    viewport::compose(Some(scene), camera, quality, tokens, None).unwrap()
}

fn pixel(rgb: &[u8], width: u32, p: [f64; 2]) -> [u8; 3] {
    let i = (p[1].floor() as usize * width as usize + p[0].floor() as usize) * 3;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

fn levels(a: [u8; 3], b: [u8; 3]) -> u8 {
    (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap()
}

fn length(line: &OverlayLine) -> f64 {
    (line.to[0] - line.from[0]).hypot(line.to[1] - line.from[1])
}

fn middle(line: &OverlayLine) -> [f64; 2] {
    [
        0.5 * (line.from[0] + line.to[0]),
        0.5 * (line.from[1] + line.to[1]),
    ]
}

/// The unit normal of a line on the screen.
fn normal(line: &OverlayLine) -> [f64; 2] {
    let l = length(line);
    [
        -(line.to[1] - line.from[1]) / l,
        (line.to[0] - line.from[0]) / l,
    ]
}

/// The body a face id belongs to: the index of the last body whose first
/// face id is at or below it.
fn body_of(scene: &Scene, face: u32) -> Option<usize> {
    if face == NO_FACE {
        return None;
    }
    scene
        .bodies
        .iter()
        .rposition(|body| body.first_face_id <= face)
}

/// Every edge is drawn in its own body's shade, darker than a light or
/// red body, lighter than a very dark one, contrasting with it by the
/// theme's ratio or more; the picture has that colour at the middle of
/// the edges.
#[test]
fn an_edge_is_drawn_in_a_shade_of_its_own_body() {
    let scene = blocks();
    let tokens = unfaded(wide());
    let size = [900, 500];
    for projection in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
        let camera = camera(&scene, projection, size);
        let composed = compose(&scene, &camera, size, &tokens);
        let lines = composed.overlay.lines.clone();
        let picture = viewport::rasterise(composed, &tokens, 1.0);
        write_png(&format!("colours-{projection:?}.png"), &picture);
        let mut seen = Vec::new();
        let (mut sampled, mut agree) = (0, 0);
        for (k, line) in lines.iter().enumerate() {
            let Some(edge) = line.edge else {
                continue;
            };
            let body = edge.body.unwrap_or(tokens.body);
            let colors = tokens.edge_colors(body);
            assert!(
                contrast(colors.inner, body) >= f64::from(tokens.edge_min_contrast),
                "{body:?}"
            );
            if body == DARK {
                assert!(luminance(colors.inner) > luminance(body));
            } else {
                assert!(luminance(colors.inner) < luminance(body));
            }
            assert_eq!(line.color(&tokens), colors.pick(edge.silhouette));
            if !seen.contains(&(body, edge.silhouette)) {
                seen.push((body, edge.silhouette));
            }
            // The middle of a stretch in sight, away from every other line.
            let p = middle(line);
            let crowded = lines.iter().enumerate().any(|(j, other)| {
                j != k && {
                    let q = middle(other);
                    let d = |a: [f64; 2]| (a[0] - p[0]).hypot(a[1] - p[1]);
                    d(other.from).min(d(other.to)).min(d(q)) < 8.0
                }
            });
            if line.hidden || length(line) < 16.0 || crowded {
                continue;
            }
            sampled += 1;
            agree += usize::from(levels(pixel(&picture.rgb, size[0], p), line.color(&tokens)) <= 2);
        }
        println!("{projection:?}: {sampled} middles, {agree} in the edge's colour; {seen:?}");
        // Every body has edges between faces and an outline.
        for body in [LIGHT, DARK, RED, tokens.body] {
            for silhouette in [false, true] {
                assert!(
                    seen.contains(&(body, silhouette)),
                    "{body:?} silhouette {silhouette}"
                );
            }
        }
        assert!(sampled >= 20 && agree * 100 >= sampled * 95);
    }
}

/// The outline is told by the depth buffer alone; the face ids say the
/// same independently: across an outline one side shows the background
/// or another body. Outlines are drawn wider and darker.
#[test]
fn the_outline_of_a_body_is_wider_and_darker_than_its_inner_edges() {
    let scene = blocks();
    let tokens = unfaded(wide());
    let size = [900, 500];
    for projection in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
        let camera = camera(&scene, projection, size);
        let quality = Quality {
            width: size[0],
            height: size[1],
            ssaa: 2,
        };
        let frame = viewport::render_bodies(Some(&scene), &camera, quality, &tokens).unwrap();
        let composed = compose(&scene, &camera, size, &tokens);
        let under = composed.rgb.clone();
        let lines = composed.overlay.lines.clone();
        let picture = viewport::rasterise(composed, &tokens, 1.0);
        write_png(&format!("outline-{projection:?}.png"), &picture);
        let w = size[0] as usize;
        let id = |p: [f64; 2]| {
            let (x, y) = (p[0].floor(), p[1].floor());
            if x < 0.0 || y < 0.0 || x >= f64::from(size[0]) || y >= f64::from(size[1]) {
                return NO_FACE;
            }
            frame.ids[y as usize * w + x as usize]
        };
        // How many pixels across the line at `p` the overlay changed.
        let ink_across = |p: [f64; 2], n: [f64; 2]| {
            (-12..=12)
                .filter(|&k| {
                    let q = [p[0] + n[0] * f64::from(k), p[1] + n[1] * f64::from(k)];
                    let (x, y) = (q[0].floor(), q[1].floor());
                    x >= 0.0
                        && y >= 0.0
                        && x < f64::from(size[0])
                        && y < f64::from(size[1])
                        && pixel(&picture.rgb, size[0], q) != pixel(&under, size[0], q)
                })
                .count() as f64
        };
        let (mut outline, mut outline_agree, mut inner, mut inner_agree) = (0, 0, 0, 0);
        let (mut outline_ink, mut inner_ink) = (0.0, 0.0);
        for line in &lines {
            let Some(edge) = line.edge else {
                continue;
            };
            if line.hidden || length(line) < 20.0 {
                continue;
            }
            let p = middle(line);
            let n = normal(line);
            let side = |s: f64| id([p[0] + n[0] * s * 5.0, p[1] + n[1] * s * 5.0]);
            let own = [side(1.0), side(-1.0)];
            let bodies = own.map(|f| body_of(&scene, f));
            let by_ids = bodies[0].is_none() || bodies[1].is_none() || bodies[0] != bodies[1];
            let ink = ink_across(p, n);
            if by_ids {
                outline += 1;
                outline_agree += usize::from(edge.silhouette);
                outline_ink += ink;
            } else {
                inner += 1;
                inner_agree += usize::from(!edge.silhouette);
                inner_ink += ink;
            }
            if edge.silhouette {
                assert_eq!(line.width(&tokens), tokens.edge_silhouette_width);
                let colors = edge.colors(&tokens);
                assert!(luminance(colors.silhouette) <= luminance(colors.inner));
            } else {
                assert_eq!(line.width(&tokens), tokens.edge_width);
            }
        }
        let (outline_ink, inner_ink) = (outline_ink / outline as f64, inner_ink / inner as f64);
        println!(
            "{projection:?}: outlines {outline_agree} of {outline}, inner edges {inner_agree} of {inner}; ink across {outline_ink:.2} against {inner_ink:.2} pixels"
        );
        assert!(outline >= 10 && inner >= 6, "the test is hollow");
        assert!(outline_agree * 100 >= outline * 95);
        assert!(inner_agree * 100 >= inner * 95);
        assert!(outline_ink > inner_ink + 1.0);
    }
}

/// A comb of thin plates: from far away its edges crowd, and together
/// they cover no more of the picture than the theme's cap allows (they
/// fade); up close they are as crisp as without any fade. A tiny block
/// far away has edges a few pixels long: they fade too.
#[test]
fn crowded_and_short_edges_fade_and_are_crisp_up_close() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    for k in 0..48 {
        run(
            &mut s,
            "feature.block",
            json!({"size": [0.5, 20, 20], "position": [f64::from(k) * 1.5, 0, 0]}),
        );
    }
    run(
        &mut s,
        "feature.block",
        json!({"name": "Speck", "size": [0.3, 0.3, 0.3], "position": [36, 0, 40]}),
    );
    let scene = s.scene(None).unwrap().unwrap();
    let tokens = Tokens::default();
    let size = [480, 360];
    // The ink of the edges: how far the picture is from the bodies alone.
    let ink = |composed: &Composed, tokens: &Tokens| {
        let under = composed.rgb.clone();
        let picture = viewport::rasterise(composed.clone(), tokens, 1.0);
        let sum: f64 = picture
            .rgb
            .iter()
            .zip(&under)
            .map(|(a, b)| f64::from(a.abs_diff(*b)))
            .sum();
        (sum, picture)
    };
    let far = camera(&scene, ProjectionKind::Orthographic, size);
    let composed = compose(&scene, &far, size, &tokens);
    let (faded, picture) = ink(&composed, &tokens);
    write_png("comb-far.png", &picture);
    let (plain, picture) = ink(&composed, &unfaded(tokens));
    write_png("comb-far-unfaded.png", &picture);
    println!("far: ink {faded:.0} faded, {plain:.0} unfaded");
    assert!(faded < 0.5 * plain, "{faded} against {plain}");
    // The speck's edges are a few pixels long.
    let projector = craft3d_ui_egui::camera::Projector::new(
        &far.to_render(),
        f64::from(size[0]),
        f64::from(size[1]),
    )
    .unwrap();
    let c = projector
        .project(craft3d_math::Vec3::new(36.15, 0.15, 40.15))
        .unwrap();
    // The lines near the speck's place on the screen.
    let speck_lines: Vec<&OverlayLine> = composed
        .overlay
        .lines
        .iter()
        .filter(|line| {
            let p = middle(line);
            line.edge.is_some() && (p[0] - c.x).hypot(p[1] - c.y) < 4.0
        })
        .collect();
    assert!(!speck_lines.is_empty());
    for line in speck_lines {
        let extent = line.edge.unwrap().extent;
        assert!(extent < 3.0, "the speck's edge is {extent} pixels");
        assert_eq!(viewport::fade_short(extent, &tokens, 1.0), 0.0);
    }
    // Up close: a few plates fill the picture, nothing is crowded.
    let mut near = far;
    near.distance *= 0.08;
    let composed = compose(&scene, &near, size, &tokens);
    let (faded, picture) = ink(&composed, &tokens);
    write_png("comb-near.png", &picture);
    let (plain, _) = ink(&composed, &unfaded(tokens));
    println!("near: ink {faded:.0} faded, {plain:.0} unfaded");
    assert!(
        plain > 0.0 && faded > 0.98 * plain,
        "{faded} against {plain}"
    );
}
