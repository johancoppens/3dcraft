//! Recording the window without a window (`craft3d_ui_egui::record`): the
//! shell's passes painted by egui's wgpu renderer into a texture of the
//! window's size and read back, as the desktop window does for
//! `ui.record`, `ui.advance` and `ui.screenshot {window: true}`. Nothing
//! here has a window or a compositor, which is the point: a covered or
//! minimised window records the same way.
//!
//! Without a graphics adapter the tests that paint say so and pass; the
//! parameters and failures are checked either way.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Access, Session, Workspace};
use craft3d_ui_egui::Shell;
use craft3d_ui_egui::direct::headless_render_state;
use craft3d_ui_egui::record::{
    RecordAsk, RecordOptions, Recorder, VideoQuality, advance_params, record_params,
};
use egui::{Pos2, vec2};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The window in points.
const SIZE: [f32; 2] = [640.0, 400.0];

fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir).unwrap()
}

/// A session writing beneath `dir`, with a block, a cylinder and their
/// union.
fn session(dir: &Path) -> Session {
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.to_path_buf()),
        write: Access::Root(dir.to_path_buf()),
    });
    for (id, params) in [
        ("file.new", json!({"units": "mm"})),
        (
            "feature.block",
            json!({"size": ["20 mm", "20 mm", "10 mm"], "name": "Plate"}),
        ),
        (
            "feature.cylinder",
            json!({"radius": "4 mm", "height": "20 mm", "name": "Pin", "position": ["10 mm", "10 mm", "0 mm"]}),
        ),
        (
            "feature.boolean",
            json!({"op": "union", "target": "Plate", "tool": "Pin", "name": "Join"}),
        ),
    ] {
        s.execute(id, &params)
            .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    }
    s
}

fn input(time: f64) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            vec2(SIZE[0], SIZE[1]),
        )),
        time: Some(time),
        ..Default::default()
    }
}

/// The size of a PNG file.
fn png_size(path: &Path) -> (u32, u32) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let reader = decoder.read_info().unwrap();
    let info = reader.info();
    (info.width, info.height)
}

/// The RGB pixels of a PNG file.
fn pixels(path: &Path) -> Vec<u8> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    buffer
}

fn frames_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    files.sort();
    files
}

/// Records `frames` frames frame-exact into `path` (under the session's
/// root), the player playing the history; gives the files and the
/// stop's reply.
fn record(
    state: &egui_wgpu::RenderState,
    root: &Path,
    path: &str,
    include_ui: bool,
    frames: u64,
) -> (Vec<PathBuf>, Value, egui::Rect) {
    let mut s = session(root);
    let mut shell = Shell::new();
    // The desktop layout in this small window (the narrow one's viewport
    // takes the whole width: `tests/mobile.rs`).
    shell.tokens.narrow_width = 0.0;
    shell.attach_gpu(state).unwrap();
    // As the window does for a frame-exact recording.
    shell.set_frame_exact(true);
    let ctx = egui::Context::default();
    let mut recorder = Recorder::new(state.clone());
    // Laid out once, as a window is before it records (its textures
    // applied to the renderer, as the window's frames apply theirs).
    let mut time = 0.0;
    for _ in 0..3 {
        recorder
            .pass(&ctx, input(time), false, |ui| shell.ui(ui, &mut s))
            .unwrap();
        time += 0.1;
    }
    shell
        .handle_method(
            &mut s,
            "ui.set",
            Some(&json!({"timeline": {"playing": true, "speed": 2}})),
        )
        .unwrap()
        .unwrap();
    let started = recorder
        .start(
            &s,
            RecordOptions {
                path: path.to_string(),
                fps: 10.0,
                step: true,
                include_ui,
                ffmpeg: None,
                accumulate: 1,
                quality: VideoQuality::Screen,
            },
            &input(time),
            1.0,
            shell.viewport_rect(),
        )
        .unwrap();
    assert_eq!(started["recording"], true);
    assert_eq!(started["format"], "png");
    let (_, dt, _) = advance_params(Some(&json!({"frames": frames})), 10.0).unwrap();
    for _ in 0..frames {
        time += dt;
        let frame = recorder
            .pass(&ctx, input(time), true, |ui| shell.ui(ui, &mut s))
            .unwrap()
            .unwrap();
        recorder.push(&frame).unwrap();
    }
    let stopped = recorder.stop().unwrap();
    let viewport = shell.viewport_rect().unwrap();
    (frames_in(&root.join(path)), stopped, viewport)
}

#[test]
fn a_frame_exact_recording_writes_one_png_per_frame_of_the_window_size() {
    let state = match headless_render_state() {
        Ok(state) => state,
        Err(e) => {
            println!("SKIPPED: no graphics adapter: {e}");
            return;
        }
    };
    let root = dir("record-window");
    let frames = 12;
    let (files, stopped, viewport) = record(&state, &root, "frames", true, frames);
    assert_eq!(files.len(), frames as usize, "{files:?}");
    assert_eq!(stopped["frames"], frames);
    assert_eq!(stopped["recording"], false);
    for (n, file) in files.iter().enumerate() {
        assert_eq!(
            file.file_name().unwrap().to_str().unwrap(),
            format!("frame-{:06}.png", n + 1)
        );
        assert_eq!(png_size(file), (SIZE[0] as u32, SIZE[1] as u32), "{file:?}");
    }
    // The player stepped through the history meanwhile: the frames are
    // not all the same picture.
    let first = std::fs::read(&files[0]).unwrap();
    assert!(
        files.iter().any(|f| std::fs::read(f).unwrap() != first),
        "every frame is the same picture"
    );
    // The same requests give the same frames: the clock is the
    // recording's, not the wall's.
    let again = dir("record-window-again");
    let (repeat, _, _) = record(&state, &again, "frames", true, frames);
    for (a, b) in files.iter().zip(&repeat) {
        let (pa, pb) = (pixels(a), pixels(b));
        // The whole window, the same pixels; where they differ, if
        // anywhere, is said.
        let mut differ: Option<[usize; 4]> = None;
        let width = SIZE[0] as usize;
        for (i, (x, y)) in pa.chunks(3).zip(pb.chunks(3)).enumerate() {
            if x != y {
                let (px, py) = (i % width, i / width);
                let b = differ.get_or_insert([px, py, px, py]);
                *b = [b[0].min(px), b[1].min(py), b[2].max(px), b[3].max(py)];
            }
        }
        let Some([x0, y0, x1, y1]) = differ else {
            continue;
        };
        panic!(
            "{a:?}: the runs differ in pixels x {x0}..={x1}, y {y0}..={y1} (the viewport is {viewport:?})"
        );
    }
}

#[test]
fn without_the_ui_a_recording_is_the_viewport_alone() {
    let state = match headless_render_state() {
        Ok(state) => state,
        Err(e) => {
            println!("SKIPPED: no graphics adapter: {e}");
            return;
        }
    };
    let root = dir("record-viewport");
    let (files, stopped, _) = record(&state, &root, "view", false, 3);
    assert_eq!(files.len(), 3);
    let (w, h) = png_size(&files[0]);
    assert_eq!(
        (stopped["width"].as_u64(), stopped["height"].as_u64()),
        (Some(u64::from(w)), Some(u64::from(h)))
    );
    assert!(w < SIZE[0] as u32 && h < SIZE[1] as u32, "{w} × {h}");
    assert!(w > 100 && h > 100, "{w} × {h}");
}

#[test]
fn recording_refuses_what_it_cannot_do() {
    // Parameters.
    assert_eq!(record_params(None).unwrap(), RecordAsk::Status);
    assert_eq!(record_params(Some(&json!({}))).unwrap(), RecordAsk::Status);
    assert_eq!(
        record_params(Some(&json!({"stop": true}))).unwrap(),
        RecordAsk::Stop
    );
    match record_params(Some(&json!({"path": "out"}))).unwrap() {
        RecordAsk::Start(options) => {
            assert_eq!(options.fps, 30.0);
            assert!(!options.step);
            assert!(options.include_ui);
        }
        other => panic!("{other:?}"),
    }
    for bad in [
        json!(7),
        json!({"path": 3}),
        json!({"path": ""}),
        json!({"fps": 30}),
        json!({"path": "x", "fps": 0}),
        json!({"path": "x", "fps": 0.5}),
        json!({"path": "x", "fps": 1e9}),
        json!({"path": "x", "fps": "fast"}),
        json!({"path": "x", "step": "yes"}),
        json!({"path": "x", "includeUi": 1}),
        json!({"start": true, "stop": true}),
        json!({"stop": true, "path": "x"}),
        json!({"stop": false}),
        json!({"path": "x", "colour": "red"}),
        json!({"path": "x", "accumulate": 0}),
        json!({"path": "x", "accumulate": 1000}),
        json!({"path": "x", "quality": "best"}),
    ] {
        assert!(record_params(Some(&bad)).is_err(), "{bad}");
    }
    assert_eq!(advance_params(None, 25.0).unwrap(), (1, 1.0 / 25.0, 30.0));
    assert_eq!(
        advance_params(Some(&json!({"frames": 4, "dt": 0.5, "timeout": 5})), 25.0).unwrap(),
        (4, 0.5, 5.0)
    );
    for bad in [
        json!({"frames": 0}),
        json!({"frames": -1}),
        json!({"frames": 1.5}),
        json!({"frames": 1e12}),
        json!({"dt": 0}),
        json!({"dt": -1}),
        json!({"dt": 1e9}),
        json!({"dt": "1/30"}),
        json!({"steps": 3}),
        json!({"timeout": 0}),
        json!({"timeout": 500}),
        json!([1]),
    ] {
        assert!(advance_params(Some(&bad), 30.0).is_err(), "{bad}");
    }
    // A window host is needed: the shell itself refuses.
    let root = dir("record-refused");
    let mut s = session(&root);
    let mut shell = Shell::new();
    // The desktop layout in this small window (the narrow one's viewport
    // takes the whole width: `tests/mobile.rs`).
    shell.tokens.narrow_width = 0.0;
    for method in ["ui.record", "ui.advance"] {
        let answer = shell
            .handle_method(&mut s, method, Some(&json!({})))
            .unwrap();
        assert!(answer.unwrap_err().contains("desktop window"), "{method}");
    }
    // Paths outside the write root (a video whose ffmpeg is no program:
    // `an_ffmpeg_that_fails_is_an_error_never_a_hang`).
    let state = match headless_render_state() {
        Ok(state) => state,
        Err(e) => {
            println!("SKIPPED the rest: no graphics adapter: {e}");
            return;
        }
    };
    let mut recorder = Recorder::new(state);
    let options = |path: &str| RecordOptions {
        path: path.to_string(),
        fps: 30.0,
        step: true,
        include_ui: true,
        ffmpeg: None,
        accumulate: 1,
        quality: VideoQuality::Screen,
    };
    for path in ["../outside", "/etc/frames"] {
        let error = recorder
            .start(&s, options(path), &input(0.0), 1.0, None)
            .unwrap_err();
        assert!(error.starts_with("ui.record:"), "{error}");
    }
    // The viewport alone before it was laid out.
    let error = recorder
        .start(
            &s,
            RecordOptions {
                include_ui: false,
                ..options("v")
            },
            &input(0.0),
            1.0,
            None,
        )
        .unwrap_err();
    assert!(error.contains("viewport"), "{error}");
    // Nothing to stop; one recording at a time.
    assert!(recorder.stop().is_err());
    recorder
        .start(&s, options("one"), &input(0.0), 1.0, None)
        .unwrap();
    assert!(
        recorder
            .start(&s, options("two"), &input(0.0), 1.0, None)
            .is_err()
    );
    assert_eq!(recorder.stop().unwrap()["frames"], 0);
    assert_eq!(recorder.status(), json!({"recording": false}));
}

/// A stand-in for ffmpeg: a shell script that does `body`.
#[cfg(unix)]
fn fake_ffmpeg(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn an_ffmpeg_that_fails_is_an_error_never_a_hang() {
    use craft3d_ui_egui::record::{Frame, Recording, WRITER_PATIENCE};
    let root = dir("record-ffmpeg");
    let s = session(&root);
    let options = |ffmpeg: PathBuf| RecordOptions {
        path: "out.mp4".to_string(),
        fps: 30.0,
        step: true,
        include_ui: true,
        ffmpeg: Some(ffmpeg),
        accumulate: 1,
        quality: VideoQuality::Screen,
    };
    let frame = Frame {
        width: 32,
        height: 32,
        rgb: vec![90; 32 * 32 * 3],
    };
    // No such program.
    let error = Recording::start(&s, options(root.join("none")), [32, 32], None, 0.0)
        .err()
        .unwrap();
    assert!(error.contains("cannot start"), "{error}");
    // One that dies at once: the frames or the stop say so.
    let dies = fake_ffmpeg(&root, "dies", "echo 'no codec here' >&2; exit 3");
    let mut recording = Recording::start(&s, options(dies), [32, 32], None, 0.0).unwrap();
    let mut pushed = Ok(());
    for _ in 0..50 {
        pushed = recording.push(&frame);
        if pushed.is_err() {
            break;
        }
    }
    let stopped = recording.stop();
    assert!(
        pushed.is_err() || stopped.is_err(),
        "{pushed:?} {stopped:?}"
    );
    if let Err(error) = stopped {
        assert!(error.contains("ffmpeg"), "{error}");
    }
    // One that says a great deal (more than a pipe holds) and then takes
    // the frames: all is well.
    let talks = fake_ffmpeg(
        &root,
        "talks",
        "head -c 300000 /dev/zero | tr '\\0' x >&2; cat > /dev/null",
    );
    let mut recording = Recording::start(&s, options(talks), [32, 32], None, 0.0).unwrap();
    for _ in 0..20 {
        recording.push(&frame).unwrap();
    }
    assert_eq!(recording.stop().unwrap()["frames"], 20);
    // One that never reads: a frame errs after the writer's patience, and
    // the stop comes back too, ffmpeg killed.
    // Its child holds the pipe open after the script itself is killed.
    let stuck = fake_ffmpeg(&root, "stuck", "sleep 600");
    let began = std::time::Instant::now();
    let mut recording = Recording::start(&s, options(stuck), [32, 32], None, 0.0).unwrap();
    let mut failed = None;
    for _ in 0..200 {
        if let Err(error) = recording.push(&frame) {
            failed = Some(error);
            break;
        }
    }
    assert!(failed.unwrap().contains("took no frame"));
    assert!(recording.stop().is_err());
    assert!(
        began.elapsed() < WRITER_PATIENCE * 4,
        "{:?}",
        began.elapsed()
    );
}
