//! Videos of a replay's frames, made by `ffmpeg` when it is installed:
//! found on `PATH`, or named by `CRAFT3D_FFMPEG`. The engine never starts
//! a program; `craft3d-cli replay` calls this directly, and the sessions
//! of the CLI and its `serve` get it as `view.replay`'s video encoder
//! ([`FfmpegEncoder`]). FreeCAD is used the same way, as an optional
//! external tool: nothing of it is linked.

use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What an output path asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Output {
    Mp4,
    Webm,
    Gif,
    /// A directory of frames.
    Frames,
}

pub(crate) fn output_kind(out: &str) -> Output {
    match Path::new(out)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp4") => Output::Mp4,
        Some("webm") => Output::Webm,
        Some("gif") => Output::Gif,
        _ => Output::Frames,
    }
}

/// `ffmpeg`: `CRAFT3D_FFMPEG`, or the first `ffmpeg` on `PATH`.
pub(crate) fn find_ffmpeg() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("CRAFT3D_FFMPEG") {
        let named = PathBuf::from(named);
        return named.is_file().then_some(named);
    }
    let names: &[&str] = if cfg!(windows) {
        &["ffmpeg.exe", "ffmpeg"]
    } else {
        &["ffmpeg"]
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

/// The arguments for `ffmpeg` that make `out` of the frames in `frames`.
pub(crate) fn ffmpeg_args(
    kind: Output,
    frames: &Path,
    count: u64,
    fps: u64,
    out: &str,
    captions_track: bool,
    codec: &[&str],
) -> Vec<String> {
    // `file:` so that a path is never read as an option (`-x.mp4`) or a
    // protocol (`tcp://...`); in the frames' pattern a `%` of the
    // directory's own is written `%%`.
    let pattern = format!(
        "file:{}",
        frames.join("frame-").to_string_lossy().replace('%', "%%") + "%05d.png"
    );
    let captions = format!("file:{}", frames.join("captions.srt").to_string_lossy());
    let mut a: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error", "-framerate"]
        .map(String::from)
        .to_vec();
    a.push(fps.to_string());
    a.push("-i".into());
    a.push(pattern);
    match kind {
        Output::Mp4 | Output::Webm => {
            if captions_track {
                // A subtitle track the player can switch on: no font is
                // needed to make the video.
                let subtitles = if kind == Output::Mp4 {
                    "mov_text"
                } else {
                    "webvtt"
                };
                a.extend(["-i".into(), captions]);
                a.extend(["-map", "0:v", "-map", "1:s", "-c:s", subtitles].map(String::from));
            }
            a.extend(codec.iter().map(|s| (*s).to_string()));
            // H.264 and VP9 want even sides and 4:2:0 for players to show
            // them.
            a.extend(
                [
                    "-vf",
                    "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                    "-pix_fmt",
                    "yuv420p",
                ]
                .map(String::from),
            );
            if kind == Output::Mp4 {
                a.extend(["-movflags", "+faststart"].map(String::from));
            }
        }
        Output::Gif => {
            a.extend(
                [
                    "-vf",
                    "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4",
                    "-loop",
                    "0",
                ]
                .map(String::from),
            );
        }
        Output::Frames => {}
    }
    a.extend(["-frames:v".into(), count.to_string(), format!("file:{out}")]);
    a
}

/// The video encoder for `kind`: for an MP4, H.264 by x264 or OpenH264
/// when this ffmpeg has one (what browsers and players expect), else
/// ffmpeg's own choice for the container (MPEG-4 part 2 in builds without
/// H.264); for a WebM, VP9 or else VP8 by libvpx.
pub(crate) fn video_codec(kind: Output, encoders: &str) -> &'static [&'static str] {
    let has = |name: &str| {
        encoders
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(name))
    };
    match kind {
        Output::Mp4 if has("libx264") => &["-c:v", "libx264", "-crf", "18", "-preset", "slow"],
        Output::Mp4 if has("libopenh264") => &["-c:v", "libopenh264", "-b:v", "16M"],
        Output::Mp4 => &["-q:v", "2"],
        Output::Webm if has("libvpx-vp9") => &[
            "-c:v",
            "libvpx-vp9",
            "-b:v",
            "0",
            "-crf",
            "30",
            "-row-mt",
            "1",
        ],
        Output::Webm if has("libvpx") => &["-c:v", "libvpx", "-b:v", "8M", "-crf", "10"],
        Output::Webm | Output::Gif | Output::Frames => &[],
    }
}

/// Makes `out` of `count` frames in `frames` at `fps` with `ffmpeg`.
/// Returns whether the video carries the captions as a subtitle track.
pub(crate) fn make(
    ffmpeg: &Path,
    kind: Output,
    frames: &Path,
    count: u64,
    fps: u64,
    out: &str,
) -> Result<bool, String> {
    let codec = match kind {
        Output::Mp4 | Output::Webm => {
            let mut list = Command::new(ffmpeg);
            list.args(["-hide_banner", "-encoders"]);
            bounded(list, Duration::from_secs(30))
                .map(|o| video_codec(kind, &String::from_utf8_lossy(&o.stdout)))
                .unwrap_or(&[])
        }
        _ => &[],
    };
    let limit = time_limit(count);
    let run = |captions_track: bool| {
        let mut command = Command::new(ffmpeg);
        command.args(ffmpeg_args(
            kind,
            frames,
            count,
            fps,
            out,
            captions_track,
            codec,
        ));
        bounded(command, limit).map_err(|e| format!("`{}`: {e}", ffmpeg.display()))
    };
    // A video gets the captions as a subtitle track, when this ffmpeg has
    // the encoder for it (some builds leave it out).
    let wanted = matches!(kind, Output::Mp4 | Output::Webm)
        && std::fs::metadata(frames.join("captions.srt")).is_ok_and(|m| m.len() > 0);
    let mut ran = run(wanted)?;
    let mut captions_track = wanted;
    if wanted && !ran.status.success() {
        ran = run(false)?;
        captions_track = false;
    }
    if !ran.status.success() {
        let said = String::from_utf8_lossy(&ran.stderr);
        return Err(format!("{}: {}", ran.status, said.trim()));
    }
    Ok(captions_track)
}

/// How long ffmpeg may take for `count` frames: a minute, and a second a
/// frame more (an encoder makes several frames a second at full HD).
fn time_limit(count: u64) -> Duration {
    Duration::from_secs(60u64.saturating_add(count))
}

/// Runs `command` to its end, its output collected, or stops it after
/// `limit` (an ffmpeg that waits on something never ends otherwise).
fn bounded(mut command: Command, limit: Duration) -> Result<std::process::Output, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run it: {e}"))?;
    // Read on threads of their own, so a full pipe never stalls it.
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "it did not finish within {} s and was stopped",
                    limit.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("cannot wait for it: {e}")),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// `view.replay`'s `video` in the CLI's sessions.
#[derive(Default)]
pub(crate) struct FfmpegEncoder {
    found: Option<PathBuf>,
}

impl craft3d_engine::pictures::VideoEncoder for FfmpegEncoder {
    fn ready(&mut self) -> Result<(), String> {
        self.found = find_ffmpeg();
        match &self.found {
            Some(_) => Ok(()),
            None => Err("ffmpeg was not found on PATH (or set CRAFT3D_FFMPEG to it)".to_string()),
        }
    }

    fn encode(&mut self, frames: &Path, count: u64, fps: u64, out: &Path) -> Result<Value, String> {
        let ffmpeg = match &self.found {
            Some(found) => found.clone(),
            None => {
                self.ready()?;
                self.found.clone().ok_or("ffmpeg was not found")?
            }
        };
        let out_text = out.to_string_lossy();
        let kind = output_kind(&out_text);
        if kind == Output::Frames {
            return Err(format!("`{out_text}` is not an .mp4, .webm or .gif"));
        }
        let captions_track = make(&ffmpeg, kind, frames, count, fps, &out_text)?;
        Ok(match kind {
            Output::Mp4 | Output::Webm => json!({ "captionsTrack": captions_track }),
            _ => json!({}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_engine::pictures::VideoEncoder;

    #[test]
    fn the_extension_picks_the_output() {
        assert_eq!(output_kind("a/b.mp4"), Output::Mp4);
        assert_eq!(output_kind("a/b.WebM"), Output::Webm);
        assert_eq!(output_kind("B.GIF"), Output::Gif);
        assert_eq!(output_kind("frames"), Output::Frames);
        assert_eq!(output_kind("x.png"), Output::Frames);
    }

    #[test]
    fn ffmpeg_reads_the_frames_at_the_frame_rate_and_stops_at_the_count() {
        let a = ffmpeg_args(Output::Gif, Path::new("d"), 12, 24, "o.gif", false, &[]);
        let joined = a.join(" ");
        assert!(
            joined.contains("-framerate 24 -i file:d/frame-%05d.png"),
            "{joined}"
        );
        assert!(joined.ends_with("-frames:v 12 file:o.gif"), "{joined}");
        // A `%` of the directory's own is not a pattern; a name that looks
        // like an option or a protocol is a file.
        let odd = ffmpeg_args(Output::Gif, Path::new("50%"), 2, 30, "-x.gif", false, &[]).join(" ");
        assert!(odd.contains("-i file:50%%/frame-%05d.png"), "{odd}");
        assert!(odd.ends_with("file:-x.gif"), "{odd}");
        assert!(joined.contains("palettegen"), "{joined}");
        let a = ffmpeg_args(Output::Mp4, Path::new("d"), 3, 30, "o.mp4", true, &[]).join(" ");
        assert!(a.contains("yuv420p"), "{a}");
        assert!(
            a.contains("-i file:d/captions.srt") && a.contains("mov_text"),
            "{a}"
        );
        let a = ffmpeg_args(
            Output::Mp4,
            Path::new("d"),
            3,
            30,
            "o.mp4",
            false,
            &["-c:v", "x"],
        )
        .join(" ");
        assert!(!a.contains("captions"), "{a}");
        assert!(a.contains("-c:v x -vf"), "{a}");
        let a = ffmpeg_args(Output::Webm, Path::new("d"), 3, 30, "o.webm", true, &[]).join(" ");
        assert!(a.contains("webvtt") && a.contains("yuv420p"), "{a}");
        assert!(!a.contains("faststart"), "{a}");
    }

    #[test]
    fn the_encoder_is_chosen_from_what_this_ffmpeg_has() {
        let list =
            " V....D libx264  x264\n V....D libopenh264  OpenH264\n V....D libvpx-vp9  VP9\n";
        assert_eq!(video_codec(Output::Mp4, list)[1], "libx264");
        assert_eq!(
            video_codec(Output::Mp4, " V....D libopenh264  OpenH264\n")[1],
            "libopenh264"
        );
        assert_eq!(
            video_codec(Output::Mp4, " V....D mpeg4  MPEG-4\n"),
            &["-q:v", "2"]
        );
        assert_eq!(video_codec(Output::Webm, list)[1], "libvpx-vp9");
        assert_eq!(
            video_codec(Output::Webm, " V....D libvpx  VP8\n")[1],
            "libvpx"
        );
        assert!(video_codec(Output::Webm, "").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_does_not_end_is_stopped() {
        let mut sleeper = Command::new("sleep");
        sleeper.arg("30");
        let clock = Instant::now();
        let said = bounded(sleeper, Duration::from_millis(300)).unwrap_err();
        assert!(said.contains("did not finish"), "{said}");
        assert!(clock.elapsed() < Duration::from_secs(10));
        let mut echo = Command::new("echo");
        echo.arg("hello");
        let out = bounded(echo, Duration::from_secs(10)).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }

    #[test]
    fn a_missing_ffmpeg_is_an_error_that_says_so() {
        // CRAFT3D_FFMPEG naming no file means no ffmpeg, whatever PATH has.
        let found = std::env::var_os("CRAFT3D_FFMPEG").map(PathBuf::from);
        if found.as_ref().is_some_and(|f| !f.is_file()) {
            let mut encoder = FfmpegEncoder::default();
            assert!(encoder.ready().unwrap_err().contains("CRAFT3D_FFMPEG"));
        }
        let mut encoder = FfmpegEncoder {
            found: Some(PathBuf::from("/no/such/ffmpeg")),
        };
        let said = encoder
            .encode(Path::new("d"), 2, 30, Path::new("x.mp4"))
            .unwrap_err();
        assert!(said.contains("cannot run"), "{said}");
        assert_eq!(time_limit(0), Duration::from_secs(60));
        let said = encoder
            .encode(Path::new("d"), 2, 30, Path::new("x.png"))
            .unwrap_err();
        assert!(said.contains(".mp4"), "{said}");
    }
}
