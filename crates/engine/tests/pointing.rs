//! Pointing (roadmap M8.5): a position in a picture gives the persistent
//! name of the face under it, the one `document.inspect` reports.
//!
//! The oracle is the id buffer of the CPU renderer, read on generated
//! views: for every face a view shows, pixels inside it are picked and the
//! name that comes back is compared with `faceNames` of `document.inspect`
//! at the place the id buffer and `view.render`'s legend (`firstFaceId`
//! per body) give. Neither side of that comparison is the code under test:
//! the picture is rendered here, and the names are looked up here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_math::Vec3;
use craft3d_render::{Camera, Frame, NO_FACE, Projection, RenderParams};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn session() -> Session {
    Session::new(Workspace {
        read: Access::None,
        write: Access::None,
    })
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A box, a cylinder, a block with a hole drilled by a boolean, a row of
/// four separate copies and a row of three copies that overlap into one
/// piece: several bodies in one document.
fn workshop() -> Session {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Box"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 8, "height": 25, "position": [70, 10, 0], "name": "Post"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [30, 30, 12], "position": [0, 50, 0], "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": 6, "height": 30, "position": [15, 65, -5], "name": "Drill"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Drilled"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [6, 6, 6], "position": [0, -40, 0], "name": "Stud"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Stud", "count": 4, "step": [11, 0, 3], "name": "Studs"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [8, 8, 5], "position": [50, -40, 0], "name": "Tile"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Tile", "count": 3, "step": [6, 3, 2], "name": "Tiles"}),
    );
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["ok"], true, "{check}");
    s
}

fn vec3(value: &Value) -> Vec3 {
    let c: Vec<f64> = value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    Vec3::new(c[0], c[1], c[2])
}

/// The camera `view.pick` says it used.
fn camera_of(result: &Value) -> Camera {
    let camera = &result["camera"];
    Camera {
        eye: vec3(&camera["eye"]),
        target: vec3(&camera["target"]),
        up: vec3(&camera["up"]),
        projection: match camera["projection"].as_str().unwrap() {
            "perspective" => Projection::Perspective {
                fov_y_deg: camera["fovY"].as_f64().unwrap(),
            },
            "orthographic" => Projection::Orthographic {
                half_height: camera["height"].as_f64().unwrap() / 2.0,
            },
            other => panic!("{other}"),
        },
    }
}

/// With `extra` (the view's parameters) merged into a pick at (x, y).
fn pick_params(view: &Value, x: f64, y: f64) -> Value {
    let mut params = view.clone();
    params["x"] = x.into();
    params["y"] = y.into();
    params
}

/// The pixels of `frame` whose eight neighbours hold the same id as they
/// do, per face id: the inside of each visible face.
fn interiors(frame: &Frame) -> BTreeMap<u32, Vec<(u32, u32)>> {
    let (w, h) = (frame.width as i64, frame.height as i64);
    let at = |x: i64, y: i64| frame.ids[(y * w + x) as usize];
    let mut out: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let id = at(x, y);
            if id == NO_FACE {
                continue;
            }
            let inside = (-1..=1).all(|dy| (-1..=1).all(|dx| at(x + dx, y + dy) == id));
            if inside {
                out.entry(id).or_default().push((x as u32, y as u32));
            }
        }
    }
    out
}

/// What `document.inspect` and `view.render` say about face id `id`: the
/// feature of its body and its name.
fn expected_name(inspect: &Value, legend: &Value, id: u32) -> (String, String, usize) {
    let (body, first) = legend
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["body"].as_str().unwrap(),
                entry["firstFaceId"].as_u64().unwrap() as u32,
            )
        })
        .rfind(|(_, first)| *first <= id)
        .unwrap();
    let index = (id - first) as usize;
    let entry = inspect["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == body)
        .unwrap();
    (
        entry["feature"].as_str().unwrap().to_string(),
        entry["faceNames"][index].as_str().unwrap().to_string(),
        index,
    )
}

/// Where the developer looks (docs/testing.md, evidence).
fn evidence(name: &str, png: &[u8]) {
    let dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/pointing");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), png).unwrap();
}

/// Picks pixels inside every face the view shows and compares the names.
/// Returns how many faces were seen and how many picks were made.
fn check_view(s: &mut Session, label: &str, view: &Value) -> (usize, usize) {
    let (width, height) = (
        view["width"].as_u64().unwrap() as u32,
        view["height"].as_u64().unwrap() as u32,
    );
    let inspect = run(s, "document.inspect", json!({}));
    // The legend of view.render for the same view: which ids are whose.
    let mut render = view.clone();
    render["mode"] = "ids".into();
    render.as_object_mut().unwrap().remove("camera");
    let rendered = run(s, "view.render", render);
    let legend = &rendered["bodies"];
    // The camera the pick uses, and the picture it stands for, made here.
    let first = run(s, "view.pick", pick_params(view, 0.5, 0.5));
    let camera = camera_of(&first);
    let tolerance = view.get("tolerance").and_then(Value::as_f64);
    // The scene view.pick looks at.
    let scene = s.picture_scene(tolerance).unwrap().unwrap();
    let frame = craft3d_render::render(
        &scene.mesh,
        &camera,
        &RenderParams {
            width,
            height,
            ssaa: 1,
            ..RenderParams::default()
        },
    )
    .unwrap();
    evidence(
        &format!("{label}-ids.png"),
        &craft3d_render::encode_png(width, height, &frame.false_color()).unwrap(),
    );
    let faces = interiors(&frame);
    let mut picks = 0;
    for (&id, pixels) in &faces {
        let (feature, name, index) = expected_name(&inspect, legend, id);
        // The first, the middle and the last pixel of the face's inside,
        // in scan order: top, centre and bottom of what is seen of it.
        let chosen: BTreeSet<(u32, u32)> = [0, pixels.len() / 2, pixels.len() - 1]
            .into_iter()
            .map(|k| pixels[k])
            .collect();
        for (x, y) in chosen {
            let at = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let picked = run(s, "view.pick", pick_params(view, at.0, at.1));
            let context = format!("{label}: pixel ({x}, {y}), face id {id}: {picked}");
            assert_eq!(picked["hit"], true, "{context}");
            assert_eq!(picked["face"], name.as_str(), "{context}");
            assert_eq!(picked["feature"], feature.as_str(), "{context}");
            assert_eq!(picked["faceIndex"], index, "{context}");
            // The point is the one seen there: it projects onto the
            // position that was picked (`ViewAxes` says how), and its
            // distance is the one reported. (The picture's own depth
            // value is interpolated on screen and is not compared: in a
            // close-up it is a percent off on a large triangle.)
            let point = vec3(&picked["point"]);
            let axes = camera.axes().unwrap();
            let rel = point - axes.eye;
            let dist = rel.dot(axes.forward);
            let scale = match axes.projection {
                Projection::Perspective { .. } => 1.0 / (dist * axes.half),
                Projection::Orthographic { .. } => 1.0 / axes.half,
            };
            let aspect = f64::from(width) / f64::from(height);
            let sx = (rel.dot(axes.right) * scale / aspect * 0.5 + 0.5) * f64::from(width);
            let sy = (0.5 - rel.dot(axes.up) * scale * 0.5) * f64::from(height);
            assert!(
                (sx - at.0).abs() < 1e-6 && (sy - at.1).abs() < 1e-6,
                "{context}: projects to ({sx}, {sy})"
            );
            let depth = picked["depth"].as_f64().unwrap();
            assert!(
                (depth - dist).abs() <= 1e-9 * dist.abs().max(1.0),
                "{context}"
            );
            // And it is on the body: within its box.
            let body = inspect["bodies"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["feature"] == feature.as_str())
                .unwrap();
            let (lo, hi) = (vec3(&body["bbox"][0]), vec3(&body["bbox"][1]));
            let slack = 1e-6 * (hi - lo).length() + body["bboxTolerance"].as_f64().unwrap();
            for (c, (l, h)) in [point.x, point.y, point.z].into_iter().zip([
                (lo.x, hi.x),
                (lo.y, hi.y),
                (lo.z, hi.z),
            ]) {
                assert!(c >= l - slack && c <= h + slack, "{context}");
            }
            picks += 1;
        }
    }
    // A pixel of the background, when the view has one. Changed
    // 2026-10-13 (edges under the pointer): it was the first pixel without
    // a face; since a position within the pick radius of an edge in sight
    // points at that edge, also from just outside the silhouette, the
    // pixel is now one with no body within the radius (a square of 6
    // pixels about it is all background: the id buffer says so, not the
    // code under test), and such a pixel still has to answer `hit: false`.
    let (w, h) = (width as i64, height as i64);
    let clear = |i: usize| {
        let (x, y) = (i as i64 % w, i as i64 / w);
        (-6..=6_i64).all(|dy| {
            (-6..=6_i64).all(|dx| {
                let (nx, ny) = (x + dx, y + dy);
                nx < 0
                    || ny < 0
                    || nx >= w
                    || ny >= h
                    || frame.ids[(ny * w + nx) as usize] == NO_FACE
            })
        })
    };
    if let Some(i) = (0..frame.ids.len()).find(|&i| clear(i)) {
        let (x, y) = (i as u32 % width, i as u32 / width);
        let picked = run(
            s,
            "view.pick",
            pick_params(view, f64::from(x) + 0.5, f64::from(y) + 0.5),
        );
        assert_eq!(picked["hit"], false, "{label}: {picked}");
        assert_eq!(picked["outside"], false, "{label}: {picked}");
        assert!(picked.get("face").is_none(), "{label}: {picked}");
    }
    (faces.len(), picks)
}

#[test]
fn a_pick_inside_a_rendered_face_is_the_name_document_inspect_gives_it() {
    let mut s = workshop();
    let views = [
        ("iso", json!({"view": "iso", "width": 320, "height": 240})),
        (
            "iso-ortho",
            json!({"view": "iso", "projection": "orthographic", "width": 300, "height": 300}),
        ),
        (
            "front",
            json!({"view": "front", "width": 360, "height": 200}),
        ),
        (
            "back-ortho",
            json!({"view": "back", "projection": "orthographic", "width": 320, "height": 240}),
        ),
        ("left", json!({"view": "left", "width": 320, "height": 240})),
        (
            "right",
            json!({"view": "right", "width": 240, "height": 320}),
        ),
        (
            "top-ortho",
            json!({"view": "top", "projection": "orthographic", "width": 320, "height": 320}),
        ),
        (
            "bottom",
            json!({"view": "bottom", "width": 320, "height": 240}),
        ),
        (
            "drilled-close",
            json!({"view": "iso", "box": [[0, 50, 0], [30, 80, 12]], "width": 320, "height": 240}),
        ),
        (
            "tiles-zoom",
            json!({"view": "iso", "box": [[50, -40, 0], [70, -26, 9]], "zoom": 0.8,
                   "projection": "orthographic", "width": 320, "height": 240}),
        ),
        (
            "fine",
            json!({"view": "iso", "tolerance": 0.002, "zoom": 1.5, "width": 320, "height": 240}),
        ),
        // A camera of the caller's own, from below and behind.
        (
            "own-camera",
            json!({"camera": {"eye": [-120, 190, -90], "target": [35, 20, 5], "up": [0, 0, 1],
                              "fovY": 35}, "width": 320, "height": 240}),
        ),
        (
            "own-camera-ortho",
            json!({"camera": {"eye": [200, -150, 120], "target": [35, 20, 5],
                              "projection": "orthographic", "height": 150},
                   "width": 320, "height": 240}),
        ),
    ];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let (mut faces, mut picks) = (0, 0);
    for (label, view) in &views {
        let (f, p) = check_view(&mut s, label, view);
        assert!(f >= 3, "{label} shows {f} faces");
        faces += f;
        picks += p;
        seen.insert((*label).to_string());
    }
    println!("{} views, {faces} faces seen, {picks} picks", seen.len());
    assert!(faces > 150 && picks > 400, "{faces} faces, {picks} picks");
    // Every body was pointed at, and through the views together every
    // face of the simple ones.
    let inspect = run(&mut s, "document.inspect", json!({}));
    assert_eq!(inspect["bodies"].as_array().unwrap().len(), 5);
}

#[test]
fn every_face_of_every_body_is_picked_from_some_side() {
    let mut s = workshop();
    let inspect = run(&mut s, "document.inspect", json!({}));
    let mut wanted: BTreeSet<(String, String)> = BTreeSet::new();
    for body in inspect["bodies"].as_array().unwrap() {
        for name in body["faceNames"].as_array().unwrap() {
            wanted.insert((
                body["feature"].as_str().unwrap().to_string(),
                name.as_str().unwrap().to_string(),
            ));
        }
    }
    let total = wanted.len();
    // Six sides and four corners, a coarse grid of picks from each.
    let (width, height) = (200_u32, 160_u32);
    let mut found: BTreeSet<(String, String)> = BTreeSet::new();
    let directions = [
        [1.0, -1.0, 0.8],
        [-1.0, 1.0, 0.8],
        [-1.0, -1.0, -0.8],
        [1.0, 1.0, -0.8],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
        [0.3, -1.0, 0.1],
        [-0.3, 1.0, 0.1],
    ];
    for direction in directions {
        let target = Vec3::new(35.0, 20.0, 8.0);
        let eye = target + Vec3::new(direction[0], direction[1], direction[2]) * 400.0;
        let up = if direction[0] == 0.0 && direction[1] == 0.0 {
            json!([0, 1, 0])
        } else {
            json!([0, 0, 1])
        };
        let view = json!({"camera": {"eye": [eye.x, eye.y, eye.z],
                                      "target": [target.x, target.y, target.z], "up": up,
                                      "projection": "orthographic", "height": 150},
                          "width": width, "height": height});
        for y in (0..height).step_by(2) {
            for x in (0..width).step_by(2) {
                let picked = run(
                    &mut s,
                    "view.pick",
                    pick_params(&view, f64::from(x) + 0.5, f64::from(y) + 0.5),
                );
                // Changed 2026-10-13 (edges under the pointer): a hit used
                // to be a face; a position just outside the silhouette is
                // now a hit too, on the edge near it, with no face under
                // it. The faces are collected as before, wherever one is
                // under the position.
                if picked["hit"] == true && picked.get("face").is_some() {
                    found.insert((
                        picked["feature"].as_str().unwrap().to_string(),
                        picked["face"].as_str().unwrap().to_string(),
                    ));
                }
            }
        }
    }
    // Nothing was named that inspect does not list.
    assert!(found.is_subset(&wanted), "{:?}", found.difference(&wanted));
    let missing: Vec<_> = wanted.difference(&found).collect();
    println!(
        "{} of {total} faces picked; not seen: {missing:?}",
        found.len()
    );
    // What can stay unseen: faces between overlapping copies have none;
    // every face of this document is on the outside or in the hole.
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn the_point_of_a_pick_is_where_the_geometry_says() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Box"}),
    );
    // Straight down on the middle of the picture: the box is framed about
    // its centre, so the centre pixel is the middle of its top.
    let top = json!({"view": "top", "projection": "orthographic", "width": 200, "height": 200});
    let above = run(&mut s, "view.pick", pick_params(&top, 100.0, 100.0));
    assert_eq!(above["hit"], true, "{above}");
    let point = vec3(&above["point"]);
    assert!(
        (point - Vec3::new(20.0, 10.0, 10.0)).length() < 1e-9,
        "{point:?}"
    );
    let bottom =
        json!({"view": "bottom", "projection": "orthographic", "width": 200, "height": 200});
    let below = run(&mut s, "view.pick", pick_params(&bottom, 100.0, 100.0));
    let point = vec3(&below["point"]);
    assert!(
        (point - Vec3::new(20.0, 10.0, 0.0)).length() < 1e-9,
        "{point:?}"
    );
    // Two faces, two names, both of the box, both in inspect's list.
    assert_ne!(above["face"], below["face"]);
    let inspect = run(&mut s, "document.inspect", json!({}));
    let names = inspect["bodies"][0]["faceNames"].as_array().unwrap();
    for picked in [&above, &below] {
        assert_eq!(picked["feature"], "F1");
        assert_eq!(picked["name"], "Box");
        assert_eq!(
            names[picked["faceIndex"].as_u64().unwrap() as usize],
            picked["face"]
        );
    }
    // A sketch can be put on the face that was pointed at: the name is
    // one the document understands.
    let sketch = run(
        &mut s,
        "sketch.create",
        json!({"face": above["face"], "name": "OnTop"}),
    );
    assert_eq!(sketch["status"], "ok", "{sketch}");
    // Seen in perspective from the front, a pixel right of the middle is
    // a point right of the middle on the front face (y = 0).
    let front = json!({"view": "front", "width": 400, "height": 200});
    let hit = run(&mut s, "view.pick", pick_params(&front, 260.0, 100.0));
    let point = vec3(&hit["point"]);
    assert!(
        point.y.abs() < 1e-9 && point.x > 20.0 && (point.z - 5.0).abs() < 0.1,
        "{point:?}"
    );
    // The camera that comes back gives the same answer when passed in.
    let again = run(
        &mut s,
        "view.pick",
        json!({"camera": hit["camera"], "x": 260.0, "y": 100.0, "width": 400, "height": 200}),
    );
    assert_eq!(again, hit);
}

#[test]
fn a_pick_that_finds_nothing_or_makes_no_sense_is_an_answer_or_an_error() {
    let mut s = session();
    // No document.
    let error = s
        .execute("view.pick", &json!({"x": 1, "y": 1}))
        .unwrap_err();
    assert!(error.message().contains("no document is open"), "{error}");
    run(&mut s, "file.new", json!({"units": "mm"}));
    // An empty document cannot be framed, like view.render; with a camera
    // of the caller's there is simply nothing there.
    let error = s
        .execute("view.pick", &json!({"x": 1, "y": 1}))
        .unwrap_err();
    assert!(
        error.message().contains("cannot frame the model"),
        "{error}"
    );
    let own = json!({"eye": [100, 100, 100], "target": [0, 0, 0]});
    let nothing = run(
        &mut s,
        "view.pick",
        json!({"x": 400, "y": 300, "camera": own}),
    );
    assert_eq!(nothing["hit"], false);
    assert_eq!(nothing["outside"], false);

    run(
        &mut s,
        "feature.block",
        json!({"size": [40, 20, 10], "name": "Box"}),
    );
    let view = json!({"view": "iso", "width": 320, "height": 240});
    // The background, and positions that are not in the picture: answers.
    let corner = run(&mut s, "view.pick", pick_params(&view, 0.5, 0.5));
    assert_eq!(
        (&corner["hit"], &corner["outside"]),
        (&json!(false), &json!(false))
    );
    for (x, y) in [
        (-0.001, 10.0),
        (10.0, -1.0),
        (320.0, 10.0),
        (10.0, 240.0),
        (1e300, 1e300),
        (-1e300, 5.0),
        (f64::MAX, f64::MIN),
    ] {
        let out = run(&mut s, "view.pick", pick_params(&view, x, y));
        assert_eq!(
            (&out["hit"], &out["outside"]),
            (&json!(false), &json!(true)),
            "({x}, {y})"
        );
    }
    // Whole numbers are positions too: the top-left corner of a pixel.
    let middle = run(
        &mut s,
        "view.pick",
        json!({"x": 160, "y": 120, "width": 320, "height": 240}),
    );
    assert_eq!(middle["hit"], true, "{middle}");

    // On the silhouette: every pixel of the picture, the edge ones too,
    // gives the face of the id buffer or nothing, and never a panic.
    let camera = camera_of(&corner);
    let scene = s.picture_scene(None).unwrap().unwrap();
    let frame = craft3d_render::render(
        &scene.mesh,
        &camera,
        &RenderParams {
            width: 320,
            height: 240,
            ssaa: 1,
            ..RenderParams::default()
        },
    )
    .unwrap();
    let inside = interiors(&frame);
    let inner: BTreeSet<(u32, u32)> = inside.values().flatten().copied().collect();
    let mut border = 0;
    for y in 0..240_u32 {
        for x in 0..320_u32 {
            let id = frame.ids[(y * 320 + x) as usize];
            if id == NO_FACE || inner.contains(&(x, y)) {
                continue;
            }
            border += 1;
            let picked = run(
                &mut s,
                "view.pick",
                pick_params(&view, f64::from(x) + 0.5, f64::from(y) + 0.5),
            );
            assert_eq!(picked["hit"], true, "({x}, {y}): {picked}");
            let (body, index) = scene.face_of(id).unwrap();
            assert_eq!(
                picked["face"],
                body.face_names[index].as_str(),
                "({x}, {y})"
            );
            // Half a pixel further, across the silhouette or not: still
            // an answer.
            for (dx, dy) in [(0.0, 0.0), (0.999, 0.0), (0.0, 0.999), (0.999, 0.999)] {
                let near = run(
                    &mut s,
                    "view.pick",
                    pick_params(&view, f64::from(x) + dx, f64::from(y) + dy),
                );
                assert!(near["hit"].is_boolean());
            }
        }
    }
    assert!(border > 300, "{border} pixels on an edge");

    // Parameters that are wrong: errors that say what is.
    for (params, part) in [
        (json!({}), "missing parameter `x`"),
        (json!({"x": 1}), "missing parameter `y`"),
        (json!({"x": "left", "y": 1}), "`x` must be a finite number"),
        (json!({"x": null, "y": 1}), "missing parameter `x`"),
        (json!({"x": 1, "y": 1, "width": 0}), "`width`"),
        (json!({"x": 1, "y": 1, "height": 1e9}), "`height`"),
        (json!({"x": 1, "y": 1, "view": "under"}), "unknown view"),
        (json!({"x": 1, "y": 1, "zoom": 0}), "`zoom`"),
        (json!({"x": 1, "y": 1, "box": [1, 2]}), "`box`"),
        (
            json!({"x": 1, "y": 1, "tolerance": -1}),
            "`tolerance` must be positive",
        ),
        (
            json!({"x": 1, "y": 1, "pixel": 3}),
            "unknown parameter `pixel`",
        ),
        (json!({"x": 1, "y": 1, "camera": 5}), "camera"),
        (json!({"x": 1, "y": 1, "camera": {}}), "needs `eye`"),
        (
            json!({"x": 1, "y": 1, "camera": {"eye": [1, 2, 3]}}),
            "needs `target`",
        ),
        (
            json!({"x": 1, "y": 1, "camera": {"eye": [1, 2], "target": [0, 0, 0]}}),
            "`camera.eye`",
        ),
        (
            json!({"x": 1, "y": 1, "camera": {"eye": [1e300, 2, 3], "target": [0, 0, 0]}}),
            "`camera.eye`",
        ),
        (
            json!({"x": 1, "y": 1, "camera": {"eye": [1, 2, 3], "target": [1, 2, 3]}}),
            "`camera`",
        ),
        (
            json!({"x": 1, "y": 1, "camera": {"eye": [0, 0, 9], "target": [0, 0, 0]}}),
            "`camera`",
        ),
        (
            json!({"x": 1, "y": 1, "camera": {"eye": [9, 9, 9], "target": [0, 0, 0], "fovY": 180}}),
            "`camera.fovY`",
        ),
        (
            json!({"x": 1, "y": 1, "camera": {"eye": [9, 9, 9], "target": [0, 0, 0],
                                                "projection": "orthographic"}}),
            "needs `height`",
        ),
        (
            json!({"x": 1, "y": 1, "camera": {"eye": [9, 9, 9], "target": [0, 0, 0],
                                                "projection": "orthographic", "height": -2}}),
            "`camera.height`",
        ),
        (
            json!({"x": 1, "y": 1, "camera": {"eye": [9, 9, 9], "target": [0, 0, 0], "height": 4}}),
            "`camera.height` belongs",
        ),
        (
            json!({"x": 1, "y": 1, "camera": {"eye": [9, 9, 9], "target": [0, 0, 0], "lens": 4}}),
            "unknown parameter `lens`",
        ),
        (
            json!({"x": 1, "y": 1, "view": "top", "camera": {"eye": [9, 9, 9], "target": [0, 0, 0]}}),
            "one or the other",
        ),
    ] {
        let error = s.execute("view.pick", &params).unwrap_err();
        assert!(
            error.message().starts_with("view.pick: ") && error.message().contains(part),
            "{params}: {error}"
        );
    }
    // None of it changed the document.
    let inspect = run(&mut s, "document.inspect", json!({}));
    assert_eq!(inspect["dirty"], true);
    assert_eq!(inspect["history"]["undo"].as_array().unwrap().len(), 1);
}

#[test]
fn a_name_is_looked_up_in_the_scene_with_the_feature_when_it_is_ambiguous() {
    let mut s = workshop();
    let scene = s.scene(None).unwrap().unwrap();
    let inspect = run(&mut s, "document.inspect", json!({}));
    // Every name of every body means the faces at its places, and all the
    // ways of saying which body agree.
    let mut ambiguous = 0;
    for body in inspect["bodies"].as_array().unwrap() {
        let (feature, name) = (
            body["feature"].as_str().unwrap(),
            body["name"].as_str().unwrap(),
        );
        let scene_body = scene.bodies.iter().find(|b| b.feature == feature).unwrap();
        let names: Vec<&str> = body["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert_eq!(scene_body.face_names, names);
        for face in &names {
            let places: Vec<u32> = (0..names.len())
                .filter(|&k| names[k] == *face)
                .map(|k| scene_body.first_face_id + k as u32)
                .collect();
            let by_id = scene.faces_named(Some(feature), face).unwrap();
            assert_eq!(by_id.feature, feature);
            assert_eq!(by_id.body, name);
            assert_eq!(by_id.face, *face);
            // Its own places, and those of faces it is an alias of.
            assert!(places.iter().all(|id| by_id.ids.contains(id)), "{face}");
            for id in &by_id.ids {
                let (owner, index) = scene.face_of(*id).unwrap();
                assert_eq!(owner.feature, feature);
                assert!(
                    owner.face_names[index] == *face
                        || owner.face_aliases[index].iter().any(|a| a == face),
                    "{face} at {index}"
                );
            }
            assert_eq!(scene.faces_named(Some(name), face).unwrap(), by_id);
            let digits = feature.trim_start_matches('F');
            assert_eq!(scene.faces_named(Some(digits), face).unwrap(), by_id);
            match scene.faces_named(None, face) {
                Ok(alone) => assert_eq!(alone, by_id),
                Err(error) => {
                    assert!(error.contains("say which with `feature`"), "{error}");
                    ambiguous += 1;
                }
            }
        }
    }
    println!("{ambiguous} names are in more than one body");
    // Names nobody has, and things that are not names.
    let first = scene.bodies[0].face_names[0].clone();
    let error = scene.faces_named(Some("Post"), &first).unwrap_err();
    assert!(error.contains("`Post` has no face"), "{error}");
    let error = scene.faces_named(Some("Nobody"), &first).unwrap_err();
    assert!(
        error.contains("no body of a feature `Nobody`") && error.contains("Box (F1)"),
        "{error}"
    );
    let error = scene.faces_named(Some("F999"), &first).unwrap_err();
    assert!(error.contains("no body of a feature"), "{error}");
    // A feature whose body went into a boolean has none any more.
    let error = scene.faces_named(Some("Plate"), &first).unwrap_err();
    assert!(error.contains("no body of a feature `Plate`"), "{error}");
    let error = scene.faces_named(None, "F99:side(e1)").unwrap_err();
    assert!(error.contains("no body has a face"), "{error}");
    for junk in [
        "",
        " ",
        "top",
        "F1",
        "F1:",
        "\u{0}",
        &"F1:side(".repeat(5000),
    ] {
        assert!(scene.faces_named(None, junk).is_err(), "{junk:.40}");
        assert!(scene.faces_named(Some(junk), junk).is_err(), "{junk:.40}");
    }
    // Two bodies with a face of the same name (no command makes them
    // today: a copy's faces are named after the copy): the name alone is
    // refused, and the feature says which.
    let mut twins = (*scene).clone();
    let mut twin = twins.bodies[0].clone();
    twin.feature = "F99".to_string();
    twin.name = "Twin".to_string();
    twin.first_face_id = 1000;
    std::sync::Arc::make_mut(&mut twins.bodies).push(twin);
    let error = twins.faces_named(None, &first).unwrap_err();
    assert!(
        error.contains("several bodies have a face")
            && error.contains("`Box` and `Twin`")
            && error.contains("say which with `feature`"),
        "{error}"
    );
    assert_eq!(
        twins.faces_named(Some("Twin"), &first).unwrap().ids,
        vec![1000]
    );
    assert_eq!(
        twins.faces_named(Some("99"), &first).unwrap().feature,
        "F99"
    );
    assert_eq!(twins.faces_named(Some("F1"), &first).unwrap().ids, vec![0]);
    assert_eq!(twins.face_of(1003).unwrap().0.name, "Twin");
    // An id that is no face of the scene.
    assert!(scene.face_of(u32::MAX).is_none());
    assert!(scene.face_of(u32::MAX - 1).is_none());
    assert!(scene.face_of(0).is_some());
}
