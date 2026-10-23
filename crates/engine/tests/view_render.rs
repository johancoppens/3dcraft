//! `view.render`: orthographic pictures are to scale, `zoom` magnifies
//! about the centre, and `box` frames a part of the model from outside
//! the whole of it. Checked by measuring the pictures.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{
    Access, DARK_BACKGROUND, DISPLAY_RELATIVE_TOLERANCE, Session, VIEWPORT_BACKGROUND, Workspace,
};
use serde_json::{Value, json};
use std::path::PathBuf;

/// The background of a picture by default: the window's viewport.
const BACKGROUND: [u8; 3] = VIEWPORT_BACKGROUND;

fn refused(session: &mut Session, params: Value) -> String {
    match session.execute("view.render", &params) {
        Ok(result) => panic!("view.render {params} should fail, gave {result}"),
        Err(e) => e.message().to_string(),
    }
}

fn unbase64(text: &str) -> Vec<u8> {
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
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let bits = chunk
            .iter()
            .filter(|&&c| c != b'=')
            .fold(0_u32, |acc, &c| (acc << 6) | value(c))
            << (6 * pad);
        let bytes = [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    out
}

struct Shot {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
    result: Value,
    png: Vec<u8>,
}

impl Shot {
    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }

    fn drawn(&self, x: usize, y: usize) -> bool {
        self.pixel(x, y) != BACKGROUND
    }

    /// The box around everything drawn, in pixels: (width, height).
    fn extent(&self) -> (usize, usize) {
        let columns = (0..self.width)
            .filter(|&x| (0..self.height).any(|y| self.drawn(x, y)))
            .count();
        let rows = (0..self.height)
            .filter(|&y| (0..self.width).any(|x| self.drawn(x, y)))
            .count();
        (columns, rows)
    }

    fn coverage(&self) -> f64 {
        self.result["coverage"].as_f64().unwrap()
    }
}

fn shoot(session: &mut Session, params: Value) -> Shot {
    let result = run(session, "view.render", params);
    let png = unbase64(result["base64"].as_str().unwrap());
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
        .read_info()
        .unwrap();
    let mut rgb = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgb).unwrap();
    rgb.truncate(info.buffer_size());
    assert_eq!(result["width"], info.width);
    assert_eq!(result["height"], info.height);
    Shot {
        width: info.width as usize,
        height: info.height as usize,
        rgb,
        result,
        png,
    }
}

fn evidence(name: &str, shot: &Shot) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/m4");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), &shot.png).unwrap();
}

/// How much of a square picture the near face of a cube of half size
/// `half` fills when a perspective camera of 30° frames the cube: the
/// eye is as near as it can be while the near face is in view, at
/// `d = half + half / tan 15°` from the centre, and then 5 % further;
/// the face is at `1.05 · d − half` and would fill the picture at
/// `d − half`.
fn near_face_share(half: f64) -> f64 {
    let d = half + half / 15.0_f64.to_radians().tan();
    (d - half) / (1.05 * d - half)
}

/// A plate of 60 × 40 × 8 with a corner at the origin, times `scale`.
fn plate(scale: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"tolerance": 1e-6 * scale}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [60.0 * scale, 40.0 * scale, 8.0 * scale]}),
    );
    s
}

/// A length of `expected` pixels touches that many pixels, give or take
/// the one at each end that it covers only in part (the picture is
/// supersampled, so a pixel covered in part is not the background).
fn close(pixels: usize, expected: f64) -> bool {
    (pixels as f64 - expected).abs() <= 1.5
}

#[test]
fn an_orthographic_picture_is_to_scale() {
    for scale in [1e-3, 1.0, 1e6] {
        let mut s = plate(scale);
        // From the top, 3:2 like the plate: it fills the picture but for
        // the 5 % margin, in both directions at once.
        let shot = shoot(
            &mut s,
            json!({"view": "top", "projection": "orthographic", "width": 300, "height": 200}),
        );
        let (w, h) = shot.extent();
        assert!(
            close(w, 300.0 / 1.05) && close(h, 200.0 / 1.05),
            "scale {scale}: {w} x {h}"
        );
        assert!((shot.coverage() - 1.0 / (1.05 * 1.05)).abs() < 0.01);
        assert_eq!(shot.result["projection"], "orthographic");
        assert_eq!(shot.result["zoom"], 1.0);
        if scale == 1.0 {
            evidence("view-render-ortho-top.png", &shot);
        }
        // From the front the plate is 60 wide and 8 high: the width
        // decides, and the height is to the same scale.
        let shot = shoot(
            &mut s,
            json!({"view": "front", "projection": "orthographic", "width": 300, "height": 200}),
        );
        let (w, h) = shot.extent();
        let per_unit = 300.0 / 1.05 / 60.0;
        assert!(
            close(w, 60.0 * per_unit) && close(h, 8.0 * per_unit),
            "scale {scale}: {w} x {h}"
        );
        // From the side it is 40 wide: the same 8 is now larger, because
        // the picture is filled by 40 instead of 60, and still to scale.
        let shot = shoot(
            &mut s,
            json!({"view": "right", "projection": "orthographic", "width": 300, "height": 200}),
        );
        let (w, h) = shot.extent();
        let per_unit = 300.0 / 1.05 / 40.0;
        assert!(
            close(w, 40.0 * per_unit) && close(h, 8.0 * per_unit),
            "scale {scale}: {w} x {h}"
        );
    }
}

#[test]
fn in_perspective_what_is_nearer_is_larger_and_orthographic_it_is_not() {
    // Two posts of the same size, one far behind the other and beside it.
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 40], "position": [-30, 0, 0]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 40], "position": [20, 200, 0]}),
    );
    let heights = |shot: &Shot| -> (usize, usize) {
        let column = |x: usize| (0..shot.height).filter(|&y| shot.drawn(x, y)).count();
        let half = shot.width / 2;
        (
            (0..half).map(column).max().unwrap(),
            (half..shot.width).map(column).max().unwrap(),
        )
    };
    let ortho = shoot(
        &mut s,
        json!({"view": "front", "projection": "orthographic", "width": 400, "height": 300}),
    );
    let (near, far) = heights(&ortho);
    assert_eq!(near, far, "orthographic: the same height at any depth");
    let persp = shoot(
        &mut s,
        json!({"view": "front", "width": 400, "height": 300}),
    );
    assert_eq!(persp.result["projection"], "perspective");
    let (near, far) = heights(&persp);
    assert!(
        near as f64 > far as f64 * 1.3,
        "perspective: {near} near, {far} far"
    );
    evidence("view-render-ortho-front-two-posts.png", &ortho);
    evidence("view-render-perspective-front-two-posts.png", &persp);
}

#[test]
fn zoom_magnifies_about_the_centre() {
    let mut s = plate(1.0);
    let base = json!({"view": "top", "projection": "orthographic", "width": 300, "height": 200});
    let with = |zoom: f64| {
        let mut params = base.clone();
        params["zoom"] = json!(zoom);
        params
    };
    let full = 300.0 / 1.05;
    // Half: the plate is half as wide and stays in the middle.
    let shot = shoot(&mut s, with(0.5));
    let (w, h) = shot.extent();
    assert!(
        close(w, full * 0.5) && close(h, full * 0.5 / 1.5),
        "{w} x {h}"
    );
    assert!(shot.drawn(150, 100) && !shot.drawn(40, 100) && !shot.drawn(260, 100));
    assert_eq!(shot.result["zoom"], 0.5);
    evidence("view-render-ortho-zoom-0.5.png", &shot);
    let shot = shoot(&mut s, with(0.25));
    let (w, _) = shot.extent();
    assert!(close(w, full * 0.25), "{w}");
    // Twice: the plate is larger than the picture.
    let shot = shoot(&mut s, with(2.0));
    assert_eq!(shot.extent(), (300, 200));
    assert_eq!(shot.coverage(), 1.0);
    // 1 is no zoom at all: the same bytes as without.
    assert_eq!(
        shoot(&mut s, with(1.0)).png,
        shoot(&mut s, base.clone()).png
    );

    // In perspective too: the top face, the nearest thing, is half as
    // wide at half the zoom.
    let top = |s: &mut Session, zoom: f64| {
        shoot(
            s,
            json!({"view": "top", "width": 300, "height": 200, "zoom": zoom}),
        )
        .extent()
        .0 as f64
    };
    let (one, half, third) = (top(&mut s, 1.0), top(&mut s, 0.5), top(&mut s, 1.0 / 3.0));
    assert!((one - full).abs() <= 1.0, "{one}");
    assert!((half - one * 0.5).abs() <= 1.0, "{half} of {one}");
    assert!((third - one / 3.0).abs() <= 1.0, "{third} of {one}");
    // And the default projection without zoom is what it always was.
    let plain = shoot(&mut s, json!({"view": "top", "width": 300, "height": 200}));
    let named = shoot(
        &mut s,
        json!({"view": "top", "width": 300, "height": 200, "projection": "perspective", "zoom": 1}),
    );
    assert_eq!(plain.png, named.png);
}

/// A plate with a small post on it, far from the middle, and a tall wall
/// between the post and where the front view looks from.
fn plate_post_and_wall() -> Session {
    let mut s = plate(1.0);
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Post", "position": [50, 30, 8], "radius": 3, "height": 6}),
    );
    s
}

#[test]
fn a_box_frames_a_part_of_the_model() {
    let mut s = plate_post_and_wall();
    // From the top, the square around the post: a disc of diameter 6 in a
    // picture 6 wide, so 1 / 1.05 of it across, on the plate (which is
    // everywhere under it). Tessellated finely, so the disc is round to
    // well within a pixel.
    let around_post = json!([[47, 27, 8], [53, 33, 14]]);
    let shot = shoot(
        &mut s,
        json!({"view": "top", "projection": "orthographic", "box": around_post,
               "width": 240, "height": 240, "mode": "ids", "tolerance": 0.001}),
    );
    evidence("view-render-box-post-top-ids.png", &shot);
    assert_eq!(shot.coverage(), 1.0);
    assert_eq!(
        shot.result["box"],
        json!([[47.0, 27.0, 8.0], [53.0, 33.0, 14.0]])
    );
    // In the false colours the post's top is one colour, the plate's top
    // another: the disc is the colour at the centre.
    let disc = shot.pixel(120, 120);
    assert_ne!(disc, shot.pixel(2, 2));
    let across = (0..240).filter(|&x| shot.pixel(x, 120) == disc).count();
    let down = (0..240).filter(|&y| shot.pixel(120, y) == disc).count();
    let expected = 240.0 / 1.05;
    assert!(
        close(across, expected) && close(down, expected),
        "{across} x {down}, expected {expected}"
    );
    // The corners in the other order are the same box.
    let swapped = shoot(
        &mut s,
        json!({"view": "top", "projection": "orthographic", "box": [[53, 33, 14], [47, 27, 8]],
               "width": 240, "height": 240, "mode": "ids", "tolerance": 0.001}),
    );
    assert_eq!(swapped.png, shot.png);

    // In perspective, from the top: the eye is above the whole model, and
    // the disc's top is the near face of the box, 3 from its centre.
    let expected = 240.0 * near_face_share(3.0);
    assert!(expected < 240.0 / 1.05 - 2.0, "less than orthographic");
    let shot = shoot(
        &mut s,
        json!({"view": "top", "box": around_post, "width": 240, "height": 240, "mode": "ids", "tolerance": 0.001}),
    );
    evidence("view-render-box-post-top-perspective-ids.png", &shot);
    let disc = shot.pixel(120, 120);
    let across = (0..240).filter(|&x| shot.pixel(x, 120) == disc).count();
    assert!(close(across, expected), "{across}, expected {expected}");
    // Zoomed out four times the disc is a quarter of that.
    let shot = shoot(
        &mut s,
        json!({"view": "top", "box": around_post, "zoom": 0.25,
               "width": 240, "height": 240, "mode": "ids", "tolerance": 0.001}),
    );
    let disc = shot.pixel(120, 120);
    let across = (0..240).filter(|&x| shot.pixel(x, 120) == disc).count();
    assert!(close(across, expected * 0.25), "{across}");
}

#[test]
fn a_close_up_in_perspective_is_taken_from_outside_the_model() {
    // A small cube at the origin, and a tall thin wall that runs from
    // just in front of it to far towards the front view's eye. A camera
    // placed to frame the cube alone would stand inside the wall, which
    // the renderer cannot draw; from outside the model the wall's end is
    // in front of the cube.
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    run(
        &mut s,
        "feature.block",
        json!({"name": "Cube", "size": [10, 10, 10], "position": [-5, -5, -5]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Wall", "size": [2, 990, 40], "position": [-1, -1000, -20]}),
    );
    let cube = json!([[-5, -5, -5], [5, 5, 5]]);
    let shot = shoot(
        &mut s,
        json!({"view": "front", "box": cube, "width": 200, "height": 200}),
    );
    evidence("view-render-box-behind-wall.png", &shot);
    // The eye is 5 % beyond the wall's end, which is therefore some
    // twenty times nearer than the cube and as many times larger: it
    // fills the picture. (From inside the wall the picture would be the
    // cube alone, with the margin around it.)
    assert_eq!(shot.coverage(), 1.0);
    for (x, y) in [(0, 0), (199, 0), (0, 199), (199, 199), (100, 100)] {
        assert!(shot.drawn(x, y), "the wall is missing at ({x}, {y})");
    }

    // Orthographic there is no eye to be inside anything, and no depth
    // to magnify by: the wall is a stripe 2 wide down the middle, from
    // the very top to the very bottom of the picture, and the cube's
    // front shows beside it, to scale.
    let shot = shoot(
        &mut s,
        json!({"view": "front", "projection": "orthographic", "box": cube,
               "width": 200, "height": 200}),
    );
    evidence("view-render-box-behind-wall-ortho.png", &shot);
    for y in [0, 1, 100, 198, 199] {
        assert!(shot.drawn(100, y), "the wall is missing at row {y}");
    }
    assert!(shot.drawn(40, 100) && shot.drawn(160, 100));
    assert!(!shot.drawn(40, 1) && !shot.drawn(160, 198));
    let (w, _) = shot.extent();
    assert!(close(w, 200.0 / 1.05), "{w}");
    let stripe = (0..200).filter(|&x| shot.drawn(x, 1)).count();
    assert!(close(stripe, 2.0 * 200.0 / 10.5), "{stripe}");

    // Without the wall the close-up in perspective is the cube: its front
    // face, the nearest thing, 5 from its centre.
    run(&mut s, "feature.suppress", json!({"feature": "Wall"}));
    let shot = shoot(
        &mut s,
        json!({"view": "front", "box": cube, "width": 200, "height": 200}),
    );
    let (w, h) = shot.extent();
    assert!(
        close(w, 200.0 * near_face_share(5.0)) && close(h, 200.0 * near_face_share(5.0)),
        "{w} x {h}"
    );
}

#[test]
fn bad_params_are_errors_that_say_what_to_give() {
    let mut s = plate_post_and_wall();
    let error = refused(&mut s, json!({"projection": "isometric"}));
    assert!(
        error.contains("isometric") && error.contains("orthographic"),
        "{error}"
    );
    assert!(refused(&mut s, json!({"projection": 3})).contains("projection"));
    for zoom in [
        json!(0),
        json!(-1),
        json!(0.009),
        json!(1000.5),
        json!(1e300),
        json!("2"),
        json!([2]),
        json!(true),
    ] {
        let error = refused(&mut s, json!({"zoom": zoom}));
        assert!(error.contains("zoom"), "{zoom}: {error}");
    }
    for bad in [
        json!(5),
        json!("all"),
        json!({}),
        json!([]),
        json!([[0, 0, 0]]),
        json!([[0, 0, 0], [1, 1, 1], [2, 2, 2]]),
        json!([[0, 0], [1, 1]]),
        json!([[0, 0, 0, 0], [1, 1, 1, 1]]),
        json!([[0, 0, 0], [1, 1, "1"]]),
        json!([[0, 0, 0], [1, 1, null]]),
        json!([[0, 0, 0], {"x": 1}]),
        json!([0, 0, 0, 1, 1, 1]),
        // Not a box: one point.
        json!([[4, 5, 6], [4, 5, 6]]),
        // Out of range.
        json!([[0, 0, 0], [1e13, 1, 1]]),
        json!([[-1e300, 0, 0], [1, 1, 1]]),
    ] {
        let error = refused(&mut s, json!({"box": bad}));
        assert!(error.contains("`box`"), "{bad}: {error}");
    }
    // A box with no size seen from the view asked for: orthographic
    // there is no scale to show it at.
    for (lo, hi) in [([5, 5, 0], [5, 5, 30]), ([5, 5, 0], [5, 5, 3])] {
        let error = refused(
            &mut s,
            json!({"view": "top", "projection": "orthographic", "box": [lo, hi]}),
        );
        assert!(
            error.contains("`box`") && error.contains("no size"),
            "{error}"
        );
        // In perspective it is looked along, from in front of it and of
        // the model, whichever is nearer the eye.
        let shot = shoot(
            &mut s,
            json!({"view": "top", "box": [lo, hi], "width": 64, "height": 64}),
        );
        assert!(shot.coverage() > 0.0);
    }
    // Seen from the side the same box is a line with a length: fine.
    let shot = shoot(
        &mut s,
        json!({"view": "front", "projection": "orthographic",
               "box": [[5, 5, 0], [5, 5, 30]], "width": 64, "height": 64}),
    );
    assert_eq!(shot.width, 64);
    // A box far from the model is an empty picture, not an error.
    let shot = shoot(
        &mut s,
        json!({"box": [[1e6, 1e6, 1e6], [1e6 + 1.0, 1e6 + 1.0, 1e6 + 1.0]],
               "width": 64, "height": 64}),
    );
    assert_eq!(shot.coverage(), 0.0);
    // The extremes of zoom work, in both projections.
    for projection in ["orthographic", "perspective"] {
        for zoom in [0.01, 1000.0] {
            let shot = shoot(
                &mut s,
                json!({"projection": projection, "zoom": zoom, "width": 64, "height": 64}),
            );
            assert_eq!(shot.result["zoom"], zoom);
        }
    }
    // An unknown parameter is still refused, and nothing is written for
    // a refused request.
    assert!(refused(&mut s, json!({"zoomm": 2})).contains("zoomm"));
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/view-render-params");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    s.set_workspace(Workspace {
        read: Access::None,
        write: Access::Root(dir.clone()),
    });
    assert!(refused(&mut s, json!({"path": "a.png", "zoom": 0})).contains("zoom"));
    assert!(!dir.join("a.png").exists());
    let result = run(
        &mut s,
        "view.render",
        json!({"path": "a.png", "projection": "orthographic", "zoom": 1.5, "width": 64, "height": 64}),
    );
    assert_eq!(result["path"], "a.png");
    assert!(dir.join("a.png").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A picture is drawn on the window's light viewport background unless
/// it asks for the dark one; another background is refused.
#[test]
fn the_background_is_the_viewports_unless_dark_is_asked() {
    let mut s = plate_post_and_wall();
    let params = |background: Option<&str>| {
        let mut p = json!({"width": 64, "height": 48});
        if let Some(b) = background {
            p["background"] = json!(b);
        }
        p
    };
    let light = shoot(&mut s, params(None));
    assert_eq!(VIEWPORT_BACKGROUND, [235, 233, 229]);
    assert_eq!(light.pixel(0, 0), VIEWPORT_BACKGROUND);
    assert_eq!(shoot(&mut s, params(Some("light"))).png, light.png);
    let dark = shoot(&mut s, params(Some("dark")));
    assert_eq!(dark.pixel(0, 0), DARK_BACKGROUND);
    assert_eq!(dark.pixel(63, 47), DARK_BACKGROUND);
    // The model is drawn the same on both: only the background differs.
    assert_eq!(light.coverage(), dark.coverage());
    for bad in [json!("pink"), json!(3), json!([0, 0, 0])] {
        let error = refused(&mut s, json!({"background": bad}));
        assert!(error.contains("background"), "{bad}: {error}");
    }
}

#[test]
fn the_same_request_gives_the_same_picture() {
    let mut s = plate_post_and_wall();
    let params = json!({"view": "iso", "projection": "orthographic", "zoom": 1.7,
                        "box": [[40, 20, 0], [60, 40, 14]], "width": 200, "height": 150});
    let first = shoot(&mut s, params.clone());
    let second = shoot(&mut s, params.clone());
    assert_eq!(first.png, second.png);
    let mut other = plate_post_and_wall();
    assert_eq!(shoot(&mut other, params).png, first.png);
    evidence("view-render-iso-ortho-box-zoom.png", &first);
}

/// Without a tolerance a picture is meshed at between a quarter and half
/// a pixel where it shows the model, never finer than the display rule's
/// floor (2e-4 of the diagonal of the bodies' box), with the display's
/// angle (lamp T2 and impeller T1, 2026-10-15): a larger picture, a zoom
/// or a close-up box is finer, and a whole view costs no more triangles
/// than the window's scene. With a tolerance the tessellator cannot meet
/// on a body, the body is drawn coarser and the result says so, rather
/// than the picture failing.
#[test]
fn a_picture_is_as_fine_as_its_pixels_and_never_too_fine() {
    let mut s = plate_post_and_wall();
    // The box around the bodies: the plate 60 x 40 x 8, the post to 14.
    let floor = DISPLAY_RELATIVE_TOLERANCE * (60.0_f64 * 60.0 + 40.0 * 40.0 + 14.0 * 14.0).sqrt();
    let tolerance = |s: &mut Session, params: Value| -> (f64, u64) {
        let drawn = run(s, "view.render", params);
        assert_eq!(drawn["coarsened"], false, "{drawn}");
        assert!((drawn["maxAngle"].as_f64().unwrap() - 12.0).abs() < 1e-9);
        (
            drawn["tolerance"].as_f64().unwrap(),
            drawn["triangles"].as_u64().unwrap(),
        )
    };
    let (small, small_triangles) = tolerance(&mut s, json!({"width": 200, "height": 150}));
    let (large, large_triangles) = tolerance(&mut s, json!({"width": 400, "height": 300}));
    // Twice the pixels: half the tolerance, while above the floor.
    assert!(large > 1.5 * floor, "{large} against the floor {floor}");
    assert!((large / small - 0.5).abs() < 1e-12, "{small} {large}");
    assert!(large_triangles >= small_triangles);
    // A zoom and a close-up box are finer, down to the floor.
    let (zoomed, _) = tolerance(&mut s, json!({"width": 200, "height": 150, "zoom": 2}));
    assert!(zoomed < small, "{small} {zoomed}");
    let (close, _) = tolerance(
        &mut s,
        json!({"width": 200, "height": 150, "box": [[45, 25, 8], [55, 35, 14]]}),
    );
    assert!(close < small, "{close} {small}");
    let (deep, _) = tolerance(&mut s, json!({"width": 200, "height": 150, "zoom": 1000}));
    assert!((deep / floor - 1.0).abs() < 1e-6, "{deep} against {floor}");
    // A whole view costs no more than the window's scene.
    let window = s
        .scene_with(craft3d_engine::SceneQuality::display())
        .unwrap()
        .unwrap();
    assert!(large_triangles <= window.triangles() as u64);
    // The same request gives the same tolerance and mesh whatever the
    // window has built meanwhile: a picture is a function of the document
    // and its parameters.
    let (again, again_triangles) = tolerance(&mut s, json!({"width": 400, "height": 300}));
    assert_eq!((again, again_triangles), (large, large_triangles));

    let fine = json!({"width": 64, "height": 48, "tolerance": 1e-9});
    let drawn = run(&mut s, "view.render", fine);
    assert_eq!(drawn["coarsened"], true, "{drawn}");
    assert_eq!(drawn["tolerance"], 1e-9);
    assert_eq!(drawn["maxAngle"], Value::Null);
    assert_eq!(drawn["bodies"].as_array().unwrap().len(), 2, "{drawn}");
}

/// The picture's tolerance against a pixel the test works out itself: an
/// orthographic camera `height` model units high on a picture of 300
/// pixels has pixels of `height / 300` units, and the tolerance lies in
/// (a quarter, a half] of that. `view.render` and `view.pick` with the
/// same parameters choose the same tolerance, and so does a pick with the
/// camera a pick passed back.
#[test]
fn render_and_pick_agree_on_the_tolerance_of_a_picture() {
    let mut s = plate_post_and_wall();
    let floor = DISPLAY_RELATIVE_TOLERANCE * (60.0_f64 * 60.0 + 40.0 * 40.0 + 14.0 * 14.0).sqrt();
    for height in [60.0, 30.0, 12.0] {
        let camera = json!({"eye": [30, -200, 7], "target": [30, 20, 7],
                            "projection": "orthographic", "height": height});
        let picked = run(
            &mut s,
            "view.pick",
            json!({"x": 200, "y": 150, "width": 400, "height": 300, "camera": camera}),
        );
        let tolerance = picked["tolerance"].as_f64().unwrap();
        let pixel = height / 300.0;
        assert!(
            tolerance > pixel / 4.0 && tolerance <= pixel / 2.0,
            "{tolerance} for pixels of {pixel}"
        );
        // Half a pixel is 6.8 and 3.4 floors at heights 60 and 30: a rung
        // above the floor. At 12 it is 1.36 floors: the floor itself, which
        // is still within the band.
        if height > 12.0 {
            assert!(tolerance >= 2.0 * floor, "{tolerance} {floor}");
        } else {
            assert!(
                (tolerance / floor - 1.0).abs() < 1e-9,
                "{tolerance} {floor}"
            );
        }
    }
    for params in [
        json!({"width": 400, "height": 300}),
        json!({"width": 640, "height": 200, "view": "top", "projection": "orthographic"}),
        json!({"width": 300, "height": 300, "view": "front", "zoom": 3}),
        json!({"width": 320, "height": 240, "box": [[45, 25, 8], [55, 35, 14]]}),
    ] {
        let drawn = run(&mut s, "view.render", params.clone());
        let mut pick = params.clone();
        pick["x"] = 10.into();
        pick["y"] = 10.into();
        let picked = run(&mut s, "view.pick", pick);
        assert_eq!(picked["tolerance"], drawn["tolerance"], "{params}");
        let again = run(
            &mut s,
            "view.pick",
            json!({"x": 10, "y": 10, "width": params["width"], "height": params["height"],
                   "camera": picked["camera"]}),
        );
        assert_eq!(again["tolerance"], drawn["tolerance"], "{params}");
    }
}

/// The work of a small picture of a whole model is what its pixels can
/// show, not what the window's display scene costs (lamp T2, 2026-10-15:
/// the display scene of the lamp took minutes for one picture). Counted
/// in triangles, at three scales. Where the chordal tolerance and not the
/// angle decides, a sphere's mesh has triangles in proportion to one over
/// the tolerance. The picture's tolerance is here some sixteen times the
/// window's (between a quarter and half a pixel of a 120-pixel picture of
/// the ball, against 2e-4 of the side of a square with the ball's area,
/// on the display rule's ladder: up to 4.4 % coarser, 2026-10-19), held
/// to at least twelve times; so its mesh has at least seven times fewer
/// triangles (eight before the ladder; 7.9 at the smallest scale now),
/// the rest of the factor left for the angle and the rounding of the
/// counts.
#[test]
fn a_small_picture_of_a_whole_model_costs_what_its_pixels_show() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"tolerance": 1e-6 * k}));
        run(&mut s, "feature.sphere", json!({"radius": 100.0 * k}));
        let drawn = run(&mut s, "view.render", json!({"width": 160, "height": 120}));
        let window = s
            .scene_with(craft3d_engine::SceneQuality::display())
            .unwrap()
            .unwrap();
        let tolerance = drawn["tolerance"].as_f64().unwrap();
        assert!(
            tolerance >= 12.0 * window.tolerance,
            "size {k}: {tolerance} against the window's {}",
            window.tolerance
        );
        let triangles = drawn["triangles"].as_u64().unwrap();
        assert!(
            triangles * 7 <= window.triangles() as u64,
            "size {k}: {triangles} triangles for the picture, {} for the window",
            window.triangles()
        );
        // And it is still round: the angle holds whatever the tolerance.
        assert!(triangles >= 2 * 30 * 15, "size {k}: {triangles}");
    }
}

/// A picture's scene has a budget for the whole scene, as the window's
/// has: a close-up of many bodies at the floor does not mesh them all at
/// the floor, it climbs the ladder until the scene is within the budget
/// and says it was coarsened. Shown with a budget made small by the
/// number of bodies: 900 balls, each of which has some 1700 triangles at
/// any tolerance as fine as the floor (the angle alone asks for that), so
/// 1.5 million together.
#[test]
fn a_close_up_of_a_large_model_stays_within_the_budget() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    for i in 0..30 {
        for j in 0..30 {
            run(
                &mut s,
                "feature.sphere",
                json!({"radius": 1, "position": [3 * i, 3 * j, 0]}),
            );
        }
    }
    let drawn = run(
        &mut s,
        "view.render",
        json!({"width": 64, "height": 48, "zoom": 1000}),
    );
    let triangles = drawn["triangles"].as_u64().unwrap();
    assert!(triangles <= 1_000_000, "{triangles}");
    assert_eq!(drawn["coarsened"], true, "{drawn}");
    let floor = DISPLAY_RELATIVE_TOLERANCE * (89.0_f64 * 89.0 * 2.0 + 2.0 * 2.0).sqrt();
    assert!(drawn["tolerance"].as_f64().unwrap() > floor, "{drawn}");
}

/// The default framing is the box of what the faces hold, not of the
/// whole spheres and tori they lie on: the bike's chain stay, a tube of 7
/// swept along an arc of a circle of 1332 (finding B4, 2026-10-15: framed
/// as 2671 for a stay of 455), and part turns of profiles with an arc
/// (torus patches bounded both ways round) and of a half disc (a lune of
/// a sphere). Each is compared with the box of the body's mesh, whose
/// vertices lie on the faces: the frame holds it, and is no larger than
/// it by more than the mesh's tolerance.
#[test]
fn the_default_frame_is_the_box_of_what_the_faces_hold() {
    let stay = |s: &mut Session| {
        run(s, "sketch.create", json!({"plane": "zx", "name": "Path"}));
        run(
            s,
            "sketch.path",
            json!({"sketch": "Path", "start": [0, 0], "segments": [
                {"arc": [455.57083091, 80.329429113], "center": [0, 1332.0]}
            ]}),
        );
        run(s, "sketch.create", json!({"plane": "xy", "name": "Prof"}));
        run(
            s,
            "sketch.circle",
            json!({"sketch": "Prof", "center": [0, 0], "radius": 7}),
        );
        run(
            s,
            "feature.sweep",
            json!({"sketch": "Prof", "path": "Path", "name": "Body"}),
        );
    };
    let ring_part = |s: &mut Session| {
        run(s, "sketch.create", json!({"name": "Profile"}));
        run(
            s,
            "sketch.path",
            json!({"sketch": "Profile", "start": [10, 0], "segments": [
                {"line": [20, 0]},
                {"arc": [20, 5], "center": [16, 2.5]},
                {"line": [10, 5]},
                {"line": "start"}
            ]}),
        );
        run(
            s,
            "feature.revolve",
            json!({"sketch": "Profile", "axis": "y", "angle": "70 deg", "name": "Body"}),
        );
    };
    let lune = |s: &mut Session| {
        run(s, "sketch.create", json!({"name": "Profile"}));
        run(
            s,
            "sketch.path",
            json!({"sketch": "Profile", "start": [0, -3], "segments": [
                {"arc": [0, 7], "center": [0, 2]},
                {"line": "start"}
            ]}),
        );
        run(
            s,
            "feature.revolve",
            json!({"sketch": "Profile", "axis": "y", "angle": "50 deg", "name": "Body"}),
        );
    };
    type Build = fn(&mut Session);
    let models: [(&str, Build); 3] = [("stay", stay), ("ring part", ring_part), ("lune", lune)];
    for (name, build) in models {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({}));
        build(&mut s);
        let drawn = run(&mut s, "view.render", json!({"width": 64, "height": 48}));
        let framed = &drawn["box"];
        let inspected = run(&mut s, "document.inspect", json!({}));
        let body = inspected["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == "Body")
            .unwrap_or_else(|| panic!("{name}: {}", inspected["features"]));
        let meshed = &body["bbox"];
        let slack = body["bboxTolerance"].as_f64().unwrap();
        for axis in 0..3 {
            let (lo, hi) = (
                framed[0][axis].as_f64().unwrap(),
                framed[1][axis].as_f64().unwrap(),
            );
            let (mlo, mhi) = (
                meshed[0][axis].as_f64().unwrap(),
                meshed[1][axis].as_f64().unwrap(),
            );
            let rounding = 1e-9 * (1.0 + mlo.abs().max(mhi.abs()));
            assert!(
                lo <= mlo + rounding && hi >= mhi - rounding,
                "{name}: the frame {framed} misses the mesh's box {meshed}"
            );
            assert!(
                lo >= mlo - slack - rounding && hi <= mhi + slack + rounding,
                "{name}: the frame {framed} is looser than the mesh's box {meshed} by more than {slack}"
            );
        }
    }
}
