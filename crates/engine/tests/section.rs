//! The section plane of a view (roadmap M8.5): `view.render` and
//! `view.pick` cut the picture open with a plane given as a base plane, a
//! datum plane, a flat face by its name or a point and a normal, and a
//! pick on a cut-open model answers what that picture shows.
//!
//! The oracles: the area of the cap in an orthographic picture against the
//! closed form of the cross-section, the id buffer of the CPU renderer for
//! the picks, and the document before and after (a section is a view).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{run, session};
use craft3d_engine::{PickKind, SECTION_INK, SECTION_PAPER, SectionSpec, Session};
use craft3d_math::Vec3;
use craft3d_render::{Camera, Projection, SECTION_FACE};
use serde_json::{Value, json};

/// A block of 40 × 30 × 20 with a pocket of 20 × 10 sunk 12 deep into its
/// top, made by a boolean, and a datum plane 5 above the base plane xy.
fn pocketed() -> Session {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 30, 20], "name": "Block"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [20, 10, 20], "position": [10, 10, 8], "name": "Tool"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Tool", "name": "Pocketed"}),
    );
    run(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "distance": 5, "name": "Low"}),
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    s
}

fn png_pixels(result: &Value) -> (u32, u32, Vec<u8>) {
    let bytes = base64(result["base64"].as_str().unwrap());
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    (info.width, info.height, buffer)
}

fn base64(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => u32::from(c - b'A'),
        b'a'..=b'z' => u32::from(c - b'a') + 26,
        b'0'..=b'9' => u32::from(c - b'0') + 52,
        b'+' => 62,
        _ => 63,
    };
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let bits = chunk.iter().fold(0_u32, |acc, &c| {
            (acc << 6) | if c == b'=' { 0 } else { value(c) }
        });
        let bits = bits << (6 * (4 - chunk.len()));
        let bytes = bits.to_be_bytes();
        out.extend_from_slice(&bytes[1..4 - pad]);
    }
    out
}

/// How many pixels of a picture have a colour of the cap: its paper
/// mixed with the ink of its hatching (2026-11-18: the cut is hatched
/// paper; before, one flat colour, counted by equality), each channel
/// within two levels of one mix. The bodies' blue grey is never such a
/// colour.
fn cap_pixels(result: &Value) -> usize {
    let (_, _, rgb) = png_pixels(result);
    rgb.chunks(3).filter(|px| on_cap(px)).count()
}

fn on_cap(px: &[u8]) -> bool {
    let level = |c: [u8; 3], i: usize| f64::from(c[i]);
    let share = (level(SECTION_PAPER, 0) - f64::from(px[0]))
        / (level(SECTION_PAPER, 0) - level(SECTION_INK, 0));
    (-0.01..=1.01).contains(&share)
        && (0..3).all(|i| {
            let mix =
                level(SECTION_PAPER, i) + (level(SECTION_INK, i) - level(SECTION_PAPER, i)) * share;
            (mix - f64::from(px[i])).abs() <= 2.0
        })
}

#[test]
fn view_render_cuts_the_picture_open_and_the_document_stays_as_it_is() {
    let mut s = pocketed();
    let before = run(&mut s, "document.inspect", json!({}));
    let revision = s.document_revision();
    let top = json!({"view": "top", "projection": "orthographic", "width": 400, "height": 400});
    let with = |section: Value| {
        let mut params = top.clone();
        params["section"] = section;
        params
    };
    let whole = run(&mut s, "view.render", top.clone());
    assert_eq!(cap_pixels(&whole), 0);
    assert!(whole.get("section").is_none());

    // Across z at 14, through the pocket: the cap is the block's 40 × 30
    // less the pocket's 20 × 10. The picture shows the model's box with a
    // margin of 5 %: 40 model units over 400 / 1.05 pixels.
    let cut = run(
        &mut s,
        "view.render",
        with(json!({"plane": "xy", "offset": 14})),
    );
    let per_unit = 400.0 / 1.05 / 40.0;
    let expected = (40.0 * 30.0 - 20.0 * 10.0) * per_unit * per_unit;
    let measured = cap_pixels(&cut) as f64;
    // Off by at most the pixels the outline passes through: its length
    // (140 + 60 units) in pixels, times √2.
    let allowed = 200.0 * per_unit * 2.0_f64.sqrt();
    assert!(
        (measured - expected).abs() <= allowed,
        "{measured} cap pixels, {expected} expected (± {allowed})"
    );
    assert_eq!(cut["section"]["point"], json!([0.0, 0.0, 14.0]));
    assert_eq!(cut["section"]["normal"], json!([0.0, 0.0, 1.0]));
    assert_eq!(cut["section"]["range"], json!([0.0, 20.0]));

    // Below the pocket (at 5, on the datum plane; and at 5 by a point and
    // a normal): the whole rectangle, the same picture either way.
    let full = (40.0 * 30.0) * per_unit * per_unit;
    let on_datum = run(&mut s, "view.render", with(json!({"plane": "Low"})));
    let by_point = run(
        &mut s,
        "view.render",
        with(json!({"point": [3, 4, 5], "normal": [0, 0, 2]})),
    );
    assert_eq!(on_datum["base64"], by_point["base64"]);
    let measured = cap_pixels(&on_datum) as f64;
    assert!((measured - full).abs() <= 140.0 * per_unit * 2.0_f64.sqrt());

    // On the top face, moved 6 into the body: the plane of the first cut.
    let names = before["bodies"][0]["faceNames"].as_array().unwrap().clone();
    let mut on_top = None;
    for name in &names {
        let section = json!({"plane": name, "offset": -6});
        if let Ok(result) = s.execute("view.render", &with(section))
            && result["section"]["normal"] == json!([0.0, 0.0, 1.0])
            && result["section"]["point"][2] == json!(14.0)
        {
            on_top = Some(result);
        }
    }
    let on_top = on_top.expect("no face of the body is its top");
    assert_eq!(on_top["base64"], cut["base64"]);

    // Flipped, the top is kept and the cap is hidden under it; without a
    // cap there is none.
    let flipped = run(
        &mut s,
        "view.render",
        with(json!({"plane": "xy", "offset": 14, "flip": true})),
    );
    assert_eq!(cap_pixels(&flipped), 0);
    let open = run(
        &mut s,
        "view.render",
        with(json!({"plane": "xy", "offset": 14, "cap": false})),
    );
    assert_eq!(cap_pixels(&open), 0);
    // Seen at an angle the open cut shows the inside of the walls.
    let iso_open = run(
        &mut s,
        "view.render",
        json!({"section": {"plane": "xy", "offset": 14, "cap": false}}),
    );
    let iso_whole = run(&mut s, "view.render", json!({}));
    assert_eq!(cap_pixels(&iso_open), 0);
    assert_ne!(iso_open["base64"], iso_whole["base64"]);

    // Beyond the model: everything, which is the whole picture.
    let beyond = run(
        &mut s,
        "view.render",
        with(json!({"plane": "xy", "offset": 500})),
    );
    assert_eq!(beyond["base64"], whole["base64"]);

    // A view: the document did not change.
    assert_eq!(s.document_revision(), revision);
    assert_eq!(run(&mut s, "document.inspect", json!({})), before);
}

#[test]
fn a_section_that_means_no_plane_is_an_error_that_says_why() {
    let mut s = pocketed();
    let cylinder = run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 4, "height": 9, "position": [80, 0, 0], "name": "Post"}),
    );
    let _ = cylinder;
    let inspect = run(&mut s, "document.inspect", json!({}));
    let post = inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|body| body["name"] == "Post")
        .unwrap();
    let round = post["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .find(|name| {
            let section = json!({"plane": name});
            s.execute("view.render", &json!({"section": section}))
                .is_err_and(|e| e.message().contains("not flat"))
        })
        .cloned();
    assert!(round.is_some(), "the wall of the cylinder is not flat");
    for (section, says) in [
        (json!({}), "needs `plane`"),
        (json!({"plane": "nowhere"}), "there is no plane"),
        // The window's X, Y, Z are not the planes' names: the error says
        // both.
        (
            json!({"plane": "Y"}),
            "across X lies on the plane yz, across Y on zx, across Z on xy",
        ),
        (json!({"plane": "x"}), "across X lies on the plane yz"),
        (json!({"plane": "Block"}), "not a datumPlane"),
        (json!({"plane": "F1:face(99)"}), "no body has a face"),
        (
            json!({"plane": "xy", "point": [0, 0, 0], "normal": [0, 0, 1]}),
            "not both",
        ),
        (json!({"point": [0, 0, 0]}), "needs `plane`"),
        (
            json!({"point": [0, 0, 0], "normal": [0, 0, 0]}),
            "no direction",
        ),
        (
            json!({"point": [0, 0, 0], "normal": [0, 1e300, 0]}),
            "within",
        ),
        (
            json!({"point": [0, "a", 0], "normal": [0, 0, 1]}),
            "three finite numbers",
        ),
        (json!({"plane": "xy", "offset": 1e300}), "within"),
        (json!({"plane": "xy", "offset": "far"}), "offset"),
        (json!({"plane": 3.5}), "section.plane"),
        (json!({"plane": "xy", "side": "up"}), "unknown parameter"),
        (json!("xy"), "JSON object"),
    ] {
        for command in ["view.render", "view.pick"] {
            let mut params = json!({"section": section.clone()});
            if command == "view.pick" {
                params["x"] = json!(10);
                params["y"] = json!(10);
            }
            let error = s.execute(command, &params).unwrap_err();
            assert!(
                error.message().contains(says),
                "{command} {section}: {}",
                error.message()
            );
        }
    }
    // Without a document: an error, not a panic.
    let mut empty = session();
    let error = empty
        .execute("view.render", &json!({"section": {"plane": "xy"}}))
        .unwrap_err();
    assert!(
        error.message().contains("no document"),
        "{}",
        error.message()
    );
}

#[test]
fn a_pick_on_a_cut_open_model_is_what_is_seen() {
    let mut s = pocketed();
    let scene = s.scene(None).unwrap().unwrap();
    let plane = s
        .section_plane(
            &scene,
            &SectionSpec {
                plane: Some("zx".to_string()),
                offset: 15.0,
                flip: true,
                ..SectionSpec::default()
            },
        )
        .unwrap();
    // Across y at 15, through the middle of the pocket, the half towards
    // the eye (y below 15) cut away; seen from the front, from -y.
    assert_eq!(plane.normal, -Vec3::Y);
    assert_eq!(plane.range(&scene.mesh.bbox()), Some([0.0, 30.0]));
    let section = plane.section(SECTION_PAPER, None);
    let camera = Camera {
        eye: Vec3::new(20.0, -200.0, 10.0),
        target: Vec3::new(20.0, 0.0, 10.0),
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 15.0 },
    };
    let (width, height) = (400, 300);
    // 10 pixels per unit: x from 0 to 400, z = 20 at the top row 50.
    let at = |x: f64, z: f64| (x * 10.0, 250.0 - z * 10.0);
    let frame = craft3d_render::render(
        &scene.mesh,
        &camera,
        &craft3d_render::RenderParams {
            width,
            height,
            ssaa: 1,
            section: Some(section),
            ..craft3d_render::RenderParams::default()
        },
    )
    .unwrap();
    let id_at = |x: f64, y: f64| frame.ids[y as usize * width as usize + x as usize];

    // In the wall beside the pocket: the section, of the one body, on the
    // plane.
    let (x, y) = at(5.0, 10.0);
    assert_eq!(id_at(x, y), SECTION_FACE);
    let pointed = scene
        .point_at_cut(&camera, width, height, x, y, 0.0, Some(&section))
        .unwrap();
    assert_eq!(pointed.kind(), Some(PickKind::Section));
    assert!(pointed.face.is_none());
    let cut = pointed.section.clone().unwrap();
    assert_eq!(cut.body.as_deref(), Some("Pocketed"));
    assert!(
        (cut.point - Vec3::new(5.05, 15.0, 9.95)).length() < 0.11,
        "{:?}",
        cut.point
    );
    assert!((cut.depth - 215.0).abs() < 1e-9);

    // In the pocket: the wall of the pocket behind it (at y = 20), a face
    // of the body, not the face in front that was cut away.
    let (x, y) = at(20.0, 14.0);
    let pointed = scene
        .point_at_cut(&camera, width, height, x, y, 0.0, Some(&section))
        .unwrap();
    assert_eq!(pointed.kind(), Some(PickKind::Face));
    let face = pointed.face.unwrap();
    assert_eq!(face.face_id, id_at(x, y));
    assert!((face.point.y - 20.0).abs() < 1e-9, "{:?}", face.point);
    // Without the section the same position is the front of the block.
    let whole = scene.point_at(&camera, width, height, x, y, 0.0).unwrap();
    assert!((whole.face.unwrap().point.y).abs() < 1e-9);

    // Every pixel: the pick is the id buffer's, and a face that is picked
    // lies in the half that is kept.
    for j in (0..height).step_by(7) {
        for i in (0..width).step_by(7) {
            let (x, y) = (f64::from(i) + 0.5, f64::from(j) + 0.5);
            let pointed = scene
                .point_at_cut(&camera, width, height, x, y, 0.0, Some(&section))
                .unwrap();
            let id = id_at(x, y);
            match (&pointed.face, &pointed.section) {
                (Some(face), None) => {
                    assert_eq!(face.face_id, id);
                    assert!(face.point.y >= 15.0 - 1e-9, "a face that was cut away");
                }
                (None, Some(cut)) => {
                    assert_eq!(id, SECTION_FACE);
                    assert!((cut.point.y - 15.0).abs() < 1e-9);
                }
                (None, None) => assert_eq!(id, craft3d_render::NO_FACE),
                (Some(_), Some(_)) => panic!("a face and the section at once"),
            }
        }
    }

    // Edges and vertices: the front corner of the block at (0, 0, 20) is
    // cut away, so near its place nothing is near; the corner of the cut
    // at (0, 15, 20) is no vertex. An edge that crosses the plane is near
    // only where it is kept: the top edge along y at x = 0 is seen end-on.
    let (x, y) = at(0.0, 20.0);
    let pointed = scene
        .point_at_cut(
            &camera,
            width,
            height,
            x + 2.0,
            y + 2.0,
            6.0,
            Some(&section),
        )
        .unwrap();
    assert!(pointed.vertex.is_none(), "{:?}", pointed.vertex);
    let edge = pointed.edge.expect("the kept part of the edge along y");
    assert!(edge.point.y >= 15.0 - 1e-9, "{:?}", edge.point);
    let whole = scene
        .point_at(&camera, width, height, x + 2.0, y + 2.0, 6.0)
        .unwrap();
    assert!(whole.vertex.is_some(), "the corner, in the whole picture");

    // view.pick answers the same, by the same plane given as a parameter.
    let (x, y) = at(5.0, 10.0);
    let picked = run(
        &mut s,
        "view.pick",
        json!({
            "x": x, "y": y, "width": width, "height": height, "radius": 0,
            "camera": {"eye": [20, -200, 10], "target": [20, 0, 10], "projection": "orthographic", "height": 30},
            "section": {"plane": "zx", "offset": 15, "flip": true},
        }),
    );
    assert_eq!(picked["hit"], true, "{picked}");
    assert_eq!(picked["kind"], "section");
    assert_eq!(picked["section"]["body"], "Pocketed");
    assert_eq!(picked["section"]["point"][1], json!(15.0));
    assert!(picked.get("face").is_none());
}

/// A block of 60 × 40 × 20 times `k` with a hole of radius 4 along y
/// through (20, ·, 10) and one of radius 3 along z through (45, 20, ·).
fn drilled(k: f64) -> Session {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [60.0 * k, 40.0 * k, 20.0 * k], "name": "Block"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 4.0 * k, "height": 50.0 * k, "position": [20.0 * k, -5.0 * k, 10.0 * k],
               "axis": [0, 1, 0], "name": "Across"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 3.0 * k, "height": 30.0 * k, "position": [45.0 * k, 20.0 * k, -5.0 * k],
               "name": "Down"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Block", "tool": "Across", "name": "Once"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Once", "tool": "Down", "name": "Drilled"}),
    );
    s
}

#[test]
fn a_section_snaps_to_the_flat_faces_parallel_to_it_and_to_the_axes_of_holes_across_it() {
    // At three scales: the places are found within rounding of each.
    for k in [1e-3, 1.0, 1e3] {
        let mut s = drilled(k);
        let scene = s.scene(None).unwrap().unwrap();
        let mut snaps = |spec: SectionSpec| -> Vec<(f64, &'static str)> {
            let plane = s.section_plane(&scene, &spec).unwrap();
            plane
                .snaps(&scene)
                .iter()
                .map(|snap| {
                    assert_eq!(snap.feature, "F5", "{snap:?}");
                    (snap.offset / k, snap.kind.name())
                })
                .collect()
        };
        let base = |plane: &str| SectionSpec {
            plane: Some(plane.to_string()),
            ..SectionSpec::default()
        };
        let same = |got: Vec<(f64, &str)>, want: &[(f64, &str)]| {
            assert_eq!(got.len(), want.len(), "{k}: {got:?} against {want:?}");
            for ((g, gk), (w, wk)) in got.iter().zip(want) {
                assert!((g - w).abs() < 1e-9, "{k}: {got:?} against {want:?}");
                assert_eq!(gk, wk, "{k}: {got:?} against {want:?}");
            }
        };
        // Across z: the bottom and the top, and the hole along y at its
        // middle; the hole along z runs square to the plane.
        same(
            snaps(base("xy")),
            &[(0.0, "face"), (10.0, "axis"), (20.0, "face")],
        );
        // Across x: both holes' axes lie parallel to the plane.
        same(
            snaps(base("yz")),
            &[
                (0.0, "face"),
                (20.0, "axis"),
                (45.0, "axis"),
                (60.0, "face"),
            ],
        );
        same(
            snaps(base("zx")),
            &[(0.0, "face"), (20.0, "axis"), (40.0, "face")],
        );
        // Flipped and moved, the same places: they are offsets along the
        // plane's normal as given.
        same(
            snaps(SectionSpec {
                offset: 7.0 * k,
                flip: true,
                ..base("zx")
            }),
            &[(0.0, "face"), (20.0, "axis"), (40.0, "face")],
        );
        // At a slant: no face is parallel, the hole along z is.
        same(
            snaps(SectionSpec {
                point: Some([0.0, 0.0, 0.0]),
                normal: Some([1.0, 1.0, 0.0]),
                ..SectionSpec::default()
            }),
            &[(65.0 / std::f64::consts::SQRT_2, "axis")],
        );
    }
    // A document without bodies: no places, and nothing panics.
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let xy = SectionSpec {
        plane: Some("xy".to_string()),
        ..SectionSpec::default()
    };
    let scene = s.scene(None).unwrap().unwrap();
    let plane = s.section_plane(&scene, &xy).unwrap();
    assert!(plane.snaps(&scene).is_empty());
    // Moved by `at`, the plane is the one the spec gives.
    let mut s = drilled(1.0);
    let scene = s.scene(None).unwrap().unwrap();
    let yz = SectionSpec {
        plane: Some("yz".to_string()),
        ..SectionSpec::default()
    };
    let start = s.section_plane(&scene, &yz).unwrap();
    let moved = SectionSpec {
        offset: 12.5,
        flip: true,
        ..yz
    };
    assert_eq!(
        start.at(12.5, true).unwrap(),
        s.section_plane(&scene, &moved).unwrap()
    );
    assert!(start.at(f64::NAN, false).is_err());
    assert!(start.at(1e300, false).is_err());
}
