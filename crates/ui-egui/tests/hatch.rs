//! The cut in the window is paper with hatching, never the selection's
//! colour (the UX round of 2026-11-18, V1): the window hands the theme's
//! paper, ink and hatching to the renderers, for the viewport's height in
//! points; its pictures (`ui.screenshot`) and `view.render` hatch the cap
//! as the renderers define it (`craft3d_render::Hatching`, checked in
//! `crates/render/tests/hatch.rs` and against the GPU in
//! `crates/gpu/tests/section.rs`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use craft3d_render::{RenderParams, SECTION_FACE};
use craft3d_ui_egui::{RendererChoice, Shell};
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
        .unwrap_or_else(|| panic!("{method} is not a ui method"))
        .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
}

/// A flange: a plate of 80 × 60 × 10 with a bore of radius 8 and a bolt
/// hole of radius 4 on the line y = 30.
fn flange() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [80, 60, 10]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Bore", "position": [28, 30, -5], "radius": 8, "height": 20}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Bore", "name": "Bored"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Bolt", "position": [62, 30, -5], "radius": 4, "height": 20}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Bored", "tool": "Bolt", "name": "Flange"}),
    );
    s
}

fn decode(result: &Value) -> (u32, u32, Vec<u8>) {
    let text = result["base64"].as_str().unwrap();
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
        let bits = chunk.iter().fold(0_u32, |acc, &c| {
            (acc << 6) | if c == b'=' { 0 } else { value(c) }
        });
        let bits = bits << (6 * (4 - chunk.len()));
        png.extend_from_slice(&bits.to_be_bytes()[1..4 - pad]);
    }
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut rgb = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgb).unwrap();
    rgb.truncate(info.buffer_size());
    (info.width, info.height, rgb)
}

fn evidence(name: &str, width: u32, height: u32, rgb: &[u8]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ui-egui/hatch");
    std::fs::create_dir_all(&dir).unwrap();
    let png = craft3d_render::encode_png(width, height, rgb).unwrap();
    std::fs::write(dir.join(name), png).unwrap();
}

/// The sum of the channels' differences.
fn apart(a: &[u8], b: [u8; 3]) -> u32 {
    a.iter()
        .zip(b)
        .map(|(&x, y)| u32::from(x.abs_diff(y)))
        .sum()
}

/// Whether `px` is the paper mixed with the ink, each channel within two
/// levels of one mix: a pixel of the hatched cap, however much of a line
/// it covers.
fn on_cap(px: &[u8]) -> bool {
    let (paper, ink) = (craft3d_engine::SECTION_PAPER, craft3d_engine::SECTION_INK);
    let level = |c: [u8; 3], i: usize| f64::from(c[i]);
    let share = (level(paper, 0) - f64::from(px[0])) / (level(paper, 0) - level(ink, 0));
    (-0.01..=1.01).contains(&share)
        && (0..3).all(|i| {
            let mix = level(paper, i) + (level(ink, i) - level(paper, i)) * share;
            (mix - f64::from(px[i])).abs() <= 2.0
        })
}

#[test]
fn the_cut_is_hatched_paper_and_never_the_selections_colour() {
    let mut session = flange();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    let tokens = shell.tokens;
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    // Across y = 30, through both holes; the half towards the eye cut
    // away, so that the cut faces it.
    call(
        &mut shell,
        &mut session,
        "ui.set",
        json!({"section": {"on": true, "plane": "zx", "offset": 30}}),
    );
    let eye = shell.state.camera.to_render().eye;
    let plane = shell.section().unwrap();
    if (eye - plane.origin).dot(plane.normal) < 0.0 {
        call(
            &mut shell,
            &mut session,
            "ui.set",
            json!({"section": {"flip": true}}),
        );
    }
    // The window's picture, before anything is selected (a selection's
    // outline is drawn over the picture, dimmed where hidden). Below: it
    // shows the cap hatched where the renderer has it, away from its
    // outline (drawn over it as an edge); its samples may lie elsewhere
    // in the pixel, so a pixel is the paper mixed with the ink, by as much
    // or a little more or less. (The world axes are drawn over everything,
    // the cap too: three quarters of it, at least, is the cap.)
    let shot = call(
        &mut shell,
        &mut session,
        "ui.screenshot",
        json!({"scale": 1.0}),
    );
    let (w, h, rgb) = decode(&shot);
    assert_eq!([w, h], shell.viewport_size());
    evidence("flange-cut-screenshot.png", w, h, &rgb);
    // The flange selected: its faces are lit in the selection's colour,
    // the bore's wall among them.
    call(
        &mut shell,
        &mut session,
        "ui.select",
        json!({"items": [{"kind": "feature", "feature": "Flange"}]}),
    );
    let section = shell.section().unwrap();
    assert_eq!(section.color, tokens.section_paper);
    let hatch = section.hatch.expect("the window's cap is hatched");
    assert_eq!(hatch.ink, tokens.section_ink);
    assert_eq!(
        hatch.spacing_points,
        f64::from(tokens.section_hatch_spacing)
    );
    assert_eq!(hatch.width_points, f64::from(tokens.section_hatch_width));
    // In points: here one pixel is one point.
    let [width, height] = shell.viewport_size();
    assert_eq!(hatch.height_points, Some(f64::from(height)));

    // The picture as the renderers make it, with what the window lights.
    let scene = session.scene_with(shell.scene_quality()).unwrap().unwrap();
    let highlight = shell.face_highlight(Some(scene.as_ref()));
    let frame = craft3d_render::render_colored(
        &scene.mesh,
        &scene.face_colors,
        &shell.state.camera.to_render(),
        &RenderParams {
            width,
            height,
            ssaa: 1,
            background: tokens.viewport_background,
            color: tokens.body,
            highlight,
            section: Some(section),
            ..RenderParams::default()
        },
    )
    .unwrap();
    evidence("flange-cut-bore-selected.png", width, height, &frame.rgb);
    let (mut cap, mut paper, mut inked, mut lit) = (0, 0, 0, 0);
    for (i, &id) in frame.ids.iter().enumerate() {
        let px = &frame.rgb[i * 3..i * 3 + 3];
        if id == SECTION_FACE {
            cap += 1;
            paper += usize::from(px == tokens.section_paper);
            inked += usize::from(px[0] < 160);
            // Never near the selection's colour.
            assert!(
                apart(px, tokens.face_highlight) > 40,
                "the cap at pixel {i} is {px:?}, near the selection's {:?}",
                tokens.face_highlight
            );
        } else if highlight.contains(id) {
            lit += 1;
        }
    }
    assert!(cap > 2_000, "{cap} pixels of the cap");
    assert!(
        paper > cap / 4 && inked > cap / 40,
        "{cap}: {paper} paper, {inked} inked"
    );
    assert!(lit > 2_000, "{lit} pixels of the flange lit");

    let (w, h) = (w as usize, h as usize);
    let inside = |i: usize| {
        let (x, y) = (i % w, i / w);
        x > 0
            && y > 0
            && x + 1 < w
            && y + 1 < h
            && (y - 1..=y + 1)
                .all(|ny| (x - 1..=x + 1).all(|nx| frame.ids[ny * w + nx] == SECTION_FACE))
    };
    let inner: Vec<usize> = (0..frame.ids.len()).filter(|&i| inside(i)).collect();
    let hatched = inner
        .iter()
        .filter(|&&i| on_cap(&rgb[i * 3..i * 3 + 3]))
        .count();
    assert!(inner.len() > 1_000, "{} pixels inside the cap", inner.len());
    assert!(
        hatched * 4 >= inner.len() * 3,
        "{hatched} of {} inside the cap hatched",
        inner.len()
    );

    // `view.render` hatches its cap too, a pixel taken for a point: seen
    // from the side that is cut away, paper and ink.
    let counts: Vec<(usize, usize)> = [false, true]
        .into_iter()
        .map(|flip| {
            let rendered = run(
                &mut session,
                "view.render",
                json!({"width": 480, "height": 360, "view": "iso",
                       "section": {"plane": "zx", "offset": 30, "flip": flip}}),
            );
            let (w, h, rgb) = decode(&rendered);
            evidence(&format!("flange-cut-view-render-{flip}.png"), w, h, &rgb);
            let paper = rgb
                .chunks(3)
                .filter(|px| *px == craft3d_engine::SECTION_PAPER)
                .count();
            let inked = rgb.chunks(3).filter(|px| on_cap(px) && px[0] < 160).count();
            (paper, inked)
        })
        .collect();
    let (paper, inked) = counts.iter().copied().max().unwrap();
    assert!(
        paper > 1_000 && inked > 100,
        "{counts:?}: paper and inked pixels"
    );
}

#[test]
fn without_a_section_the_window_draws_no_cap() {
    let mut session = flange();
    let mut shell = Shell::with_renderer(RendererChoice::Cpu);
    call(&mut shell, &mut session, "ui.fit", json!({"view": "iso"}));
    assert!(shell.section().is_none());
    let shot = call(&mut shell, &mut session, "ui.screenshot", json!({}));
    let (_, _, rgb) = decode(&shot);
    let tokens = shell.tokens;
    assert_eq!(
        rgb.chunks(3)
            .filter(|px| *px == tokens.section_paper)
            .count(),
        0
    );
}
