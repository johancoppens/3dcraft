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

/// A stand-in for ffmpeg: a shell script that lists x264 and OpenH264
/// and passes their trials, then does `body`.
#[cfg(unix)]
fn fake_ffmpeg(dir: &Path, name: &str, body: &str) -> PathBuf {
    fake_ffmpeg_with(
        dir,
        name,
        " V....D libx264  x264\n V....D libopenh264  OpenH264\n",
        "cat > /dev/null; exit 0",
        body,
    )
}

/// A stand-in for ffmpeg that answers `-encoders` with `listing`, a
/// trial (`-f null -`) with `trial`, and otherwise does `body`; it writes
/// the arguments of each call to `<name>.args`.
#[cfg(unix)]
fn fake_ffmpeg_with(dir: &Path, name: &str, listing: &str, trial: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> \"$0.args\"\ncase \"$*\" in\n-version) exit 0;;\n*-encoders*) printf '{listing}'; exit 0;;\n*'-f null -'*) {trial};;\nesac\n{body}\n"
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Another test's fork may hold the file open for writing a moment
    // ("Text file busy"): wait until it runs.
    for _ in 0..500 {
        match std::process::Command::new(&path).arg("-version").output() {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            _ => break,
        }
    }
    let _ = std::fs::remove_file(dir.join(format!("{name}.args")));
    let _ = std::fs::remove_file(dir.join(format!("{name}.bytes")));
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
    // Either way the error carries what ffmpeg said.
    let error = stopped.err().or(pushed.err()).unwrap();
    assert!(
        error.contains("ffmpeg") && error.contains("no codec here"),
        "{error}"
    );
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

#[cfg(unix)]
fn video_options(ffmpeg: PathBuf, path: &str, quality: VideoQuality) -> RecordOptions {
    RecordOptions {
        path: path.to_string(),
        fps: 30.0,
        step: true,
        include_ui: true,
        ffmpeg: Some(ffmpeg),
        accumulate: 1,
        quality,
    }
}

/// Fedora's ffmpeg has no x264: the next H.264 encoder that works is
/// used, and a window of odd height is padded to even for its 4:2:0.
#[cfg(unix)]
#[test]
fn without_x264_the_next_encoder_is_used_and_odd_sides_are_padded() {
    use craft3d_ui_egui::record::{Frame, Recording};
    let root = dir("record-encoder-fallback");
    let s = session(&root);
    // Lists x264 and VAAPI but neither works here (no x264 library, no
    // H.264 profile on the device); OpenH264 does.
    let ffmpeg = fake_ffmpeg_with(
        &root,
        "fedora",
        " V....D libopenh264  OpenH264\n V....D h264_vaapi  VAAPI\n V....D libx264  x264\n",
        "case \"$*\" in *libopenh264*) cat > /dev/null; exit 0;; *) echo 'No usable encoding profile found.' >&2; exit 1;; esac",
        "cat > \"$0.bytes\"",
    );
    let (w, h) = (33u32, 31u32);
    let mut recording = Recording::start(
        &s,
        video_options(ffmpeg.clone(), "videos/odd.mp4", VideoQuality::Screen),
        [w, h],
        None,
        0.0,
    )
    .unwrap();
    let status = recording.status();
    assert_eq!(status["encoder"], "libopenh264", "{status}");
    assert_eq!(status["encoded"], json!([34, 32]), "{status}");
    assert_eq!(status["width"], 33);
    // ffmpeg makes no directory; the recording makes the file's.
    assert!(root.join("videos").is_dir());
    // A frame whose last column and row are a colour of their own.
    let mut rgb = vec![10u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            if x == w - 1 || y == h - 1 {
                let i = ((y * w + x) * 3) as usize;
                rgb[i..i + 3].copy_from_slice(&[200, 100, 50]);
            }
        }
    }
    let frame = Frame {
        width: w,
        height: h,
        rgb,
    };
    for _ in 0..3 {
        recording.push(&frame).unwrap();
    }
    assert_eq!(recording.stop().unwrap()["frames"], 3);
    let bytes = std::fs::read(root.join("fedora.bytes")).unwrap();
    assert_eq!(bytes.len(), 34 * 32 * 3 * 3);
    // Padded by repeating the last column and row, never black.
    let px = |x: usize, y: usize| &bytes[(y * 34 + x) * 3..(y * 34 + x) * 3 + 3];
    assert_eq!(px(33, 0), &[200, 100, 50]);
    assert_eq!(px(0, 31), &[200, 100, 50]);
    assert_eq!(px(5, 5), &[10, 10, 10]);
    let args = std::fs::read_to_string(root.join("fedora.args")).unwrap();
    let record = args.lines().last().unwrap();
    assert!(
        record.contains("-s 34x32")
            && record.contains("libopenh264")
            && !record.contains("libx264"),
        "{args}"
    );
    // The listing and each trial are asked once per ffmpeg, not per
    // recording.
    let calls = args.lines().count();
    let again = Recording::start(
        &s,
        video_options(ffmpeg, "again.mp4", VideoQuality::Screen),
        [w, h],
        None,
        0.0,
    )
    .unwrap();
    drop(again);
    let args = std::fs::read_to_string(root.join("fedora.args")).unwrap();
    assert_eq!(args.lines().count(), calls + 1, "{args}");
}

/// An ffmpeg with no encoder for the container is refused by `ui.record`
/// itself, before a frame, saying what it looked for.
#[cfg(unix)]
#[test]
fn an_ffmpeg_without_an_encoder_for_the_container_is_refused_at_the_start() {
    use craft3d_ui_egui::record::Recording;
    let root = dir("record-no-encoder");
    let s = session(&root);
    let ffmpeg = fake_ffmpeg_with(
        &root,
        "bare",
        " V....D libvpx-vp9  VP9\n A....D aac  AAC\n",
        "echo 'Unknown encoder' >&2; exit 1",
        "cat > /dev/null",
    );
    let error = Recording::start(
        &s,
        video_options(ffmpeg.clone(), "x.mp4", VideoQuality::Web),
        [32, 32],
        None,
        0.0,
    )
    .err()
    .unwrap();
    assert!(
        error.starts_with("ui.record:") && error.contains("no encoder for `.mp4`"),
        "{error}"
    );
    assert!(
        error.contains("libx264") && error.contains("libopenh264"),
        "{error}"
    );
    // Listed but failing its trial: said, with what ffmpeg said.
    let error = Recording::start(
        &s,
        video_options(ffmpeg, "x.webm", VideoQuality::Web),
        [32, 32],
        None,
        0.0,
    )
    .err()
    .unwrap();
    assert!(error.contains("libvpx-vp9 (Unknown encoder)"), "{error}");
    // Nothing was recorded.
    let args = std::fs::read_to_string(root.join("bare.args")).unwrap();
    assert!(
        args.lines()
            .all(|l| l.contains("-encoders") || l.contains("-f null -")),
        "{args}"
    );
}

/// When ffmpeg stops taking frames (the broken pipe of an encoder that
/// cannot open), the error carries the last lines it said.
#[cfg(unix)]
#[test]
fn a_broken_pipe_says_what_ffmpeg_said() {
    use craft3d_ui_egui::record::{Frame, Recording};
    let root = dir("record-broken-pipe");
    let s = session(&root);
    let ffmpeg = fake_ffmpeg(
        &root,
        "breaks",
        "echo 'line one' >&2; echo '[libx264] height not divisible by 2 (64x31)' >&2; exit 1",
    );
    let mut recording = Recording::start(
        &s,
        video_options(ffmpeg, "x.mp4", VideoQuality::Screen),
        [64, 64],
        None,
        0.0,
    )
    .unwrap();
    // Frames big enough that the pipe fills and breaks.
    let frame = Frame {
        width: 64,
        height: 64,
        rgb: vec![1; 64 * 64 * 3],
    };
    let mut error = None;
    for _ in 0..200 {
        if let Err(e) = recording.push(&frame) {
            error = Some(e);
            break;
        }
    }
    let error = error.or(recording.stop().err()).unwrap();
    assert!(error.contains("height not divisible by 2"), "{error}");
}
