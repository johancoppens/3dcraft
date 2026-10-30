//! `view.replay`: a frame per step of the history that changes a body,
//! the first one empty, the last one the picture `view.render` gives, and
//! the document left exactly as it was.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{Access, Session, VIEWPORT_BACKGROUND, Workspace};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The background of a picture: the window's viewport.
const BACKGROUND: [u8; 3] = VIEWPORT_BACKGROUND;

fn workdir(name: &str) -> PathBuf {
    common::workdir("replay", name)
}

fn session(dir: &Path) -> Session {
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    });
    run(&mut s, "file.new", json!({}));
    s
}

/// A plate, a sketch, a hole cut through it (a sweep and a boolean), and
/// a sphere beside it.
fn part(s: &mut Session) {
    run(s, "variable.set", json!({"name": "w", "expr": "20 mm"}));
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": ["w", "w", 5]}),
    );
    run(s, "sketch.create", json!({"name": "Hole"}));
    run(
        s,
        "sketch.circle",
        json!({"sketch": "Hole", "center": [10, 10], "radius": 4}),
    );
    run(
        s,
        "feature.extrude",
        json!({"sketch": "Hole", "distance": 10, "cut": "Plate", "name": "Drilled"}),
    );
    run(
        s,
        "feature.sphere",
        json!({"radius": 4, "position": [30, 10, 4], "name": "Ball"}),
    );
}

/// What a replay must not change: the whole state document.inspect
/// reports (rollback marker, dirty flag, undo and redo history) and the
/// revision.
fn state(s: &mut Session) -> (Option<u64>, Value) {
    (s.document_revision(), run(s, "document.inspect", json!({})))
}

fn decode(path: &Path) -> (usize, usize, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width as usize, info.height as usize, buf)
}

fn frames_in(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| {
            let name = e.as_ref().unwrap().file_name();
            let name = name.to_str().unwrap();
            name.starts_with("frame-") && name.ends_with(".png")
        })
        .count()
}

#[test]
fn a_frame_per_step_that_changes_a_body_from_empty_to_the_render() {
    let dir = workdir("steps");
    let mut s = session(&dir);
    part(&mut s);
    let before = state(&mut s);
    let size = json!({"width": 160, "height": 120});
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "out", "width": 160, "height": 120}),
    );
    assert_eq!(state(&mut s), before, "the replay changed the document");

    // Plate, the hole's sweep, the boolean and the ball change a body; the
    // sketch does not.
    let steps = result["steps"].as_array().unwrap();
    let names: Vec<&str> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(steps.len(), 5, "{names:?}");
    let shown: Vec<&str> = steps
        .iter()
        .filter(|s| !s["frames"].is_null())
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(shown.len(), 4, "{shown:?}");
    assert!(!shown.contains(&"Hole"), "{shown:?}");
    assert_eq!(result["shownSteps"], 4);
    assert_eq!(result["frames"], 5);
    let out = dir.join("out");
    assert_eq!(frames_in(&out), 5);

    // The first frame is empty: background only.
    let (w, h, first) = decode(&out.join("frame-00000.png"));
    assert_eq!((w, h), (160, 120));
    assert!(first.chunks(3).all(|p| p == BACKGROUND));
    // The last frame is the picture of the document, to the pixel.
    let mut render = size.clone();
    render["path"] = json!("render.png");
    run(&mut s, "view.render", render);
    assert_eq!(
        std::fs::read(out.join("frame-00004.png")).unwrap(),
        std::fs::read(dir.join("render.png")).unwrap()
    );
    // Every step that is shown looks different from the one before.
    let pictures: Vec<Vec<u8>> = (0..5)
        .map(|i| decode(&out.join(format!("frame-{i:05}.png"))).2)
        .collect();
    for pair in pictures.windows(2) {
        assert_ne!(pair[0], pair[1]);
    }

    // A caption per shown step, in a subtitle file.
    let srt = std::fs::read_to_string(out.join("captions.srt")).unwrap();
    assert_eq!(srt.matches(" --> ").count(), 4, "{srt}");
    assert!(srt.contains("Step 1/5 · Plate · block"), "{srt}");
    assert!(srt.contains("00:00:00,033 --> 00:00:00,066"), "{srt}");
    assert!(steps[1]["caption"].as_str().unwrap().contains("Hole"));
}

#[test]
fn every_step_frames_per_step_hold_and_a_range() {
    let dir = workdir("options");
    let mut s = session(&dir);
    part(&mut s);
    let before = state(&mut s);
    // every: the sketch gets its frames too.
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "all", "width": 64, "height": 48, "every": true, "framesPerStep": 2, "hold": 3}),
    );
    assert_eq!(result["frames"], (5 + 1) * 2 + 3);
    assert_eq!(frames_in(&dir.join("all")), 15);
    let last = &result["steps"][4]["frames"];
    assert_eq!(last, &json!([10, 14]));

    // A range: from the sweep to the boolean. The first frames show the
    // model before the sweep, the plate.
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "range", "width": 64, "height": 48, "from": "Hole", "to": "Drilled"}),
    );
    let names: Vec<&str> = result["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.first(), Some(&"Hole"));
    assert_eq!(names.last(), Some(&"Drilled"));
    let first = decode(&dir.join("range/frame-00000.png")).2;
    assert!(first.chunks(3).any(|p| p != BACKGROUND));

    // A shorter replay into the same directory leaves no frames of the
    // longer one behind.
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "all", "width": 64, "height": 48}),
    );
    assert_eq!(result["frames"], 5);
    assert_eq!(frames_in(&dir.join("all")), 5);
    assert_eq!(state(&mut s), before);
}

#[test]
fn a_rolled_back_document_replays_to_its_marker_and_keeps_it() {
    let dir = workdir("marker");
    let mut s = session(&dir);
    part(&mut s);
    run(&mut s, "history.rollback", json!({"feature": "Drilled"}));
    let before = state(&mut s);
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "out", "width": 96, "height": 72}),
    );
    assert_eq!(state(&mut s), before);
    assert_eq!(result["steps"].as_array().unwrap().len(), 4);
    run(
        &mut s,
        "view.render",
        json!({"path": "render.png", "width": 96, "height": 72}),
    );
    let last = result["frames"].as_u64().unwrap() - 1;
    assert_eq!(
        std::fs::read(dir.join(format!("out/frame-{last:05}.png"))).unwrap(),
        std::fs::read(dir.join("render.png")).unwrap()
    );
    // Past the marker on request; the marker stays.
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "out", "width": 96, "height": 72, "to": "Ball"}),
    );
    assert_eq!(result["steps"].as_array().unwrap().len(), 5);
    assert_eq!(state(&mut s), before);
}

#[test]
fn an_orbit_turns_the_camera_and_ends_where_it_says() {
    let dir = workdir("orbit");
    let mut s = session(&dir);
    part(&mut s);
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "out", "width": 64, "height": 48, "orbit": 360, "hold": 4}),
    );
    assert_eq!(result["frames"], 9);
    let out = dir.join("out");
    // The hold frames differ: the camera keeps turning.
    let a = decode(&out.join("frame-00006.png")).2;
    let b = decode(&out.join("frame-00007.png")).2;
    assert_ne!(a, b);
    // Every frame has the model in it and nothing of it on the border:
    // the framing holds from every angle.
    for i in 1..9 {
        let (w, h, rgb) = decode(&out.join(format!("frame-{i:05}.png")));
        assert!(rgb.chunks(3).any(|p| p != BACKGROUND), "frame {i} is empty");
        let at = |x: usize, y: usize| &rgb[(y * w + x) * 3..(y * w + x) * 3 + 3];
        for x in 0..w {
            assert_eq!(at(x, 0), BACKGROUND, "frame {i}");
            assert_eq!(at(x, h - 1), BACKGROUND, "frame {i}");
        }
        for y in 0..h {
            assert_eq!(at(0, y), BACKGROUND, "frame {i}");
            assert_eq!(at(w - 1, y), BACKGROUND, "frame {i}");
        }
    }
}

#[test]
fn bad_requests_are_refused_and_change_nothing() {
    let dir = workdir("errors");
    let mut s = session(&dir);
    let refused = |s: &mut Session, params: Value| -> String {
        let before = state(s);
        let message = match s.execute("view.replay", &params) {
            Ok(result) => panic!("view.replay {params} should fail, gave {result}"),
            Err(e) => e.message().to_string(),
        };
        assert!(!message.contains("internal error"), "{message}");
        assert_eq!(state(s), before, "{params}");
        message
    };
    // An empty document.
    assert!(refused(&mut s, json!({"dir": "out"})).contains("no features"));
    // Nothing but a sketch: no body at the end.
    run(&mut s, "sketch.create", json!({"name": "Lone"}));
    assert!(refused(&mut s, json!({"dir": "out"})).contains("no bodies"));
    part(&mut s);
    assert!(refused(&mut s, json!({})).contains("dir"));
    assert!(refused(&mut s, json!({"dir": "../out"})).contains("beneath"));
    assert!(
        refused(&mut s, json!({"dir": "out", "from": "Ball", "to": "Plate"})).contains("before")
    );
    assert!(refused(&mut s, json!({"dir": "out", "from": "Nope"})).contains("Nope"));
    assert!(refused(&mut s, json!({"dir": "out", "framesPerStep": 0})).contains("framesPerStep"));
    assert!(
        refused(
            &mut s,
            json!({"dir": "out", "framesPerStep": 600, "hold": 3600})
        )
        .contains("limit")
    );
    assert!(
        refused(
            &mut s,
            json!({"dir": "out", "width": 4096, "height": 4096, "framesPerStep": 600})
        )
        .contains("pixels")
    );
    assert!(refused(&mut s, json!({"dir": "out", "orbit": 1e9})).contains("orbit"));
    assert!(refused(&mut s, json!({"dir": "out", "view": "sideways"})).contains("view"));
    assert!(refused(&mut s, json!({"dir": "out", "fps": 0})).contains("fps"));
    assert!(refused(&mut s, json!({"dir": "out", "colour": 1})).contains("colour"));
    // A directory that is a file.
    std::fs::write(dir.join("file"), b"x").unwrap();
    assert!(
        refused(&mut s, json!({"dir": "file", "width": 32, "height": 32})).contains("directory")
    );
    // No write access at all.
    s.set_workspace(Workspace::none());
    assert!(refused(&mut s, json!({"dir": "out"})).contains("write"));
}

/// The pixels of a picture that are not the background.
fn covered(rgb: &[u8]) -> usize {
    rgb.chunks(3).filter(|p| *p != BACKGROUND).count()
}

#[test]
fn an_animation_sets_its_variables_and_frames_like_the_render() {
    let dir = workdir("animate");
    let mut s = session(&dir);
    part(&mut s);
    let before = state(&mut s);
    // A track that holds the document's own value: each frame is the
    // picture view.render gives of the document, to the pixel.
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "still", "width": 160, "height": 120, "frames": 2,
               "animate": [{"variable": "w", "from": 20, "to": 20, "unit": "mm"}]}),
    );
    assert_eq!(state(&mut s), before, "the animation changed the document");
    assert_eq!(result["frames"], 2);
    assert_eq!(result["renderer"], "cpu");
    assert_eq!(result["animate"]["tracks"][0]["from"], "20 mm");
    run(
        &mut s,
        "view.render",
        json!({"width": 160, "height": 120, "path": "render.png"}),
    );
    let render = std::fs::read(dir.join("render.png")).unwrap();
    assert_eq!(
        std::fs::read(dir.join("still/frame-00000.png")).unwrap(),
        render
    );
    assert_eq!(
        std::fs::read(dir.join("still/frame-00001.png")).unwrap(),
        render
    );
    // The camera alone: an empty list of tracks.
    let camera_only = run(
        &mut s,
        "view.replay",
        json!({"dir": "camera", "width": 160, "height": 120, "frames": 3, "animate": []}),
    );
    assert_eq!(camera_only["frames"], 3);
    assert_eq!(
        std::fs::read(dir.join("camera/frame-00002.png")).unwrap(),
        render
    );

    // A sweep of the plate's width: the plate grows from frame to frame,
    // each frame captioned with its value, and the hold repeats the last.
    let swept = run(
        &mut s,
        "view.replay",
        json!({"dir": "sweep", "width": 160, "height": 120, "duration": 0.2, "fps": 20, "hold": 2,
               "animate": [{"variable": "w", "keys": [{"t": 0, "value": 12}, {"t": 1, "value": 28}],
                            "unit": "mm", "easing": "inOut"}]}),
    );
    assert_eq!(state(&mut s), before);
    assert_eq!(swept["animate"]["states"], 4);
    assert_eq!(swept["frames"], 6);
    assert_eq!(frames_in(&dir.join("sweep")), 6);
    let pictures: Vec<Vec<u8>> = (0..6)
        .map(|i| decode(&dir.join(format!("sweep/frame-{i:05}.png"))).2)
        .collect();
    for pair in pictures.windows(2).take(3) {
        assert_ne!(pair[0], pair[1]);
    }
    assert_eq!(pictures[3], pictures[5], "the hold repeats the last state");
    let srt = std::fs::read_to_string(dir.join("sweep/captions.srt")).unwrap();
    assert!(srt.contains("w = 12 mm"), "{srt}");
    assert!(srt.contains("w = 28 mm"), "{srt}");
}

#[test]
fn a_camera_path_dollies_orbits_and_passes_through_its_keys() {
    let dir = workdir("camera");
    let mut s = session(&dir);
    part(&mut s);
    // A dolly to half the distance: the model fills more of the last
    // frame than of the first.
    run(
        &mut s,
        "view.replay",
        json!({"dir": "dolly", "width": 96, "height": 72, "frames": 3, "animate": [],
               "camera": {"dolly": 0.5, "easing": "in"}}),
    );
    let first = covered(&decode(&dir.join("dolly/frame-00000.png")).2);
    let last = covered(&decode(&dir.join("dolly/frame-00002.png")).2);
    assert!(last > first * 2, "{first} -> {last}");
    // The history with a camera path: each frame its own picture.
    let orbit = run(
        &mut s,
        "view.replay",
        json!({"dir": "orbit", "width": 64, "height": 48, "camera": {"orbit": 180}, "hold": 2}),
    );
    assert_eq!(orbit["orbit"], 180.0);
    let a = decode(&dir.join("orbit/frame-00005.png")).2;
    let b = decode(&dir.join("orbit/frame-00006.png")).2;
    assert_ne!(a, b, "the camera keeps turning in the hold");
    // Keys: from the top at the start to the front at the end, the
    // target held on the plate.
    run(
        &mut s,
        "view.replay",
        json!({"dir": "keys", "width": 64, "height": 48, "frames": 2, "animate": [],
               "camera": {"keys": [
                   {"t": 0, "eye": [10, 10, 120], "target": [10, 10, 0], "up": [0, 1, 0]},
                   {"t": 1, "eye": [10, -120, 2], "target": [10, 10, 2], "fov": 40}]}}),
    );
    let top = decode(&dir.join("keys/frame-00000.png")).2;
    let front = decode(&dir.join("keys/frame-00001.png")).2;
    assert_ne!(top, front);
    assert!(covered(&top) > 0 && covered(&front) > 0);
}

#[test]
fn the_timeline_is_a_track_of_its_own() {
    let dir = workdir("timeline");
    let mut s = session(&dir);
    run(&mut s, "variable.set", json!({"name": "t", "expr": "0"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "gap", "expr": "4 mm"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Base", "size": [10, 10, 4]}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Lid", "size": [10, 10, 2], "position": [0, 0, "4 mm + 30 mm * t"]}),
    );
    let before = state(&mut s);
    let result = run(
        &mut s,
        "view.replay",
        json!({"dir": "explode", "width": 64, "height": 96, "frames": 3,
               "animate": [{"variable": "t", "from": 0, "to": 1, "easing": "out"}]}),
    );
    assert_eq!(state(&mut s), before);
    let tracks = result["animate"]["tracks"].as_array().unwrap();
    assert_eq!(tracks[0]["timeline"], true);
    let a = decode(&dir.join("explode/frame-00000.png")).2;
    let c = decode(&dir.join("explode/frame-00002.png")).2;
    assert_ne!(a, c, "the lid moved");
    // The timeline with another variable at once.
    let both = run(
        &mut s,
        "view.replay",
        json!({"dir": "both", "width": 64, "height": 96, "frames": 2,
               "animate": [{"variable": "t", "from": 0, "to": 1},
                           {"variable": "gap", "from": 4, "to": 8, "unit": "mm"}]}),
    );
    assert_eq!(both["animate"]["tracks"].as_array().unwrap().len(), 2);
    let srt = std::fs::read_to_string(dir.join("both/captions.srt")).unwrap();
    assert!(srt.contains("t = 1, gap = 8 mm"), "{srt}");
}

#[test]
fn the_studio_look_and_the_background_go_to_every_frame() {
    let dir = workdir("look");
    let mut s = session(&dir);
    part(&mut s);
    let plain = run(
        &mut s,
        "view.replay",
        json!({"dir": "plain", "width": 64, "height": 48, "background": "dark"}),
    );
    assert!(plain.get("look").is_none());
    let (w, h, first) = decode(&dir.join("plain/frame-00000.png"));
    assert_eq!((w, h), (64, 48));
    assert!(
        first
            .chunks(3)
            .all(|p| p == craft3d_engine::DARK_BACKGROUND)
    );
    let studio = run(
        &mut s,
        "view.replay",
        json!({"dir": "studio", "width": 64, "height": 48, "look": {"accumulate": 2}, "renderer": "cpu"}),
    );
    assert_eq!(studio["look"]["accumulate"], 2);
    assert_eq!(studio["renderer"], "cpu");
    let lit = decode(&dir.join("studio/frame-00004.png")).2;
    let flat = decode(&dir.join("plain/frame-00004.png")).2;
    assert_ne!(lit, flat);
    // Without accumulate the window's 16 frames, within the CPU's limit.
    let settled = run(
        &mut s,
        "view.replay",
        json!({"dir": "settled", "width": 32, "height": 24, "look": {}, "from": "Ball"}),
    );
    assert_eq!(settled["look"]["accumulate"], 16);
}

#[test]
fn bad_animations_are_refused_and_change_nothing() {
    let dir = workdir("animate-errors");
    let mut s = session(&dir);
    part(&mut s);
    let refused = |s: &mut Session, params: Value| -> String {
        let before = state(s);
        let message = match s.execute("view.replay", &params) {
            Ok(result) => panic!("view.replay {params} should fail, gave {result}"),
            Err(e) => e.message().to_string(),
        };
        assert!(!message.contains("internal error"), "{message}");
        assert_eq!(state(s), before, "{params}");
        message
    };
    let with = |extra: Value| -> Value {
        let mut p = json!({"dir": "out", "width": 32, "height": 24, "frames": 2, "animate": []});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    let cases = [
        (
            json!({"animate": [{"variable": "nope", "from": 1, "to": 2}]}),
            "nope",
        ),
        (
            json!({"animate": [{"variable": "w", "from": 1, "to": 2}]}),
            "unit",
        ),
        (
            json!({"animate": [{"variable": "w", "from": 1, "to": 2, "unit": "deg"}]}),
            "unit",
        ),
        (json!({"animate": [{"variable": "w"}]}), "from and to"),
        (json!({"animate": 7}), "list of tracks"),
        (json!({"animate": [], "duration": 1}), "not both"),
        (json!({"animate": [], "from": "Plate"}), "does not go with"),
        (json!({"animate": [], "motion": {}}), "does not go with"),
        (json!({"animate": [], "frames": 1}), "frames"),
        (json!({"animate": [], "frames": 3601}), "frames"),
        (json!({"animate": [], "camera": {"dolly": 0}}), "dolly"),
        (
            json!({"animate": [], "camera": {"keys": [{"t": 0, "eye": [10, 10, 2.5], "target": [10, 10, 2.5]}]}}),
            "direction",
        ),
        (
            json!({"animate": [], "camera": {"orbit": 10}, "orbit": 10}),
            "not both",
        ),
        (
            json!({"animate": [], "projection": "orthographic", "camera": {"keys": [{"t": 0, "fov": 20}]}}),
            "perspective",
        ),
        (
            json!({"width": 800, "height": 600, "look": {"accumulate": 256}}),
            "accumulate",
        ),
        (json!({"look": {"mode": "chrome"}}), "chrome"),
        (json!({"background": "pink"}), "pink"),
        (json!({"renderer": "vulkan"}), "vulkan"),
        (json!({"renderer": "gpu"}), "no GPU renderer"),
        (json!({"video": "out.avi"}), ".mp4"),
        (json!({"video": "out.mp4"}), "PNG sequence"),
        (json!({"video": "../out.mp4"}), "beneath"),
    ];
    for (extra, said) in cases {
        let params = with(extra);
        let message = refused(&mut s, params.clone());
        assert!(message.contains(said), "{params}: {message}");
    }
    // duration and frames are the animation's.
    let message = refused(&mut s, json!({"dir": "out", "duration": 2}));
    assert!(message.contains("animate"), "{message}");
    let message = refused(&mut s, json!({"dir": "out", "animate": [], "duration": -1}));
    assert!(message.contains("duration"), "{message}");
    // A timeline track takes 0 to 1 and no unit.
    run(&mut s, "variable.set", json!({"name": "t", "expr": "0"}));
    let message = refused(
        &mut s,
        json!({"dir": "out", "animate": [{"variable": "t", "from": 0, "to": 2}]}),
    );
    assert!(message.contains("0 to 1"), "{message}");
}
