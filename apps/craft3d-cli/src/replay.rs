//! `craft3d-cli replay`: a showcase video of how a document is built.
//!
//! The engine's `view.replay` writes the frames (PNG) and a subtitle file;
//! the engine never starts a program. Turning the frames into an MP4 or a
//! GIF is done here, by `ffmpeg` when it is installed: found on `PATH`, or
//! named by `CRAFT3D_FFMPEG`. Without it the frames are kept and the error
//! says where they are.

use crate::args::{self, Arg, Args};
use crate::{CliError, Parsed, print};
use craft3d_engine::{Session, Workspace};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

/// What the output path asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Output {
    Mp4,
    Gif,
    /// A directory of frames.
    Frames,
}

fn output_kind(out: &str) -> Output {
    match Path::new(out)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp4") => Output::Mp4,
        Some("gif") => Output::Gif,
        _ => Output::Frames,
    }
}

fn integer(flag: &str, text: &str) -> Result<u64, CliError> {
    text.parse()
        .map_err(|_| CliError::Usage(format!("`{flag}` must be a whole number, got `{text}`")))
}

fn number(flag: &str, text: &str) -> Result<f64, CliError> {
    text.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| CliError::Usage(format!("`{flag}` must be a number, got `{text}`")))
}

/// `ffmpeg`: `CRAFT3D_FFMPEG`, or the first `ffmpeg` on `PATH`.
fn find_ffmpeg() -> Option<PathBuf> {
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
fn ffmpeg_args(
    kind: Output,
    frames: &Path,
    count: u64,
    fps: u64,
    out: &str,
    captions_track: bool,
    codec: &[&str],
) -> Vec<String> {
    let pattern = frames.join("frame-%05d.png");
    let captions = frames.join("captions.srt");
    let mut a: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error", "-framerate"]
        .map(String::from)
        .to_vec();
    a.push(fps.to_string());
    a.push("-i".into());
    a.push(pattern.to_string_lossy().into_owned());
    match kind {
        Output::Mp4 => {
            if captions_track {
                // A subtitle track the player can switch on: no font is
                // needed to make the video.
                a.extend(["-i".into(), captions.to_string_lossy().into_owned()]);
                a.extend(["-map", "0:v", "-map", "1:s", "-c:s", "mov_text"].map(String::from));
            }
            a.extend(codec.iter().map(|s| (*s).to_string()));
            // H.264 wants even sides and 4:2:0 for players to show it.
            a.extend(
                [
                    "-vf",
                    "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                    "-pix_fmt",
                    "yuv420p",
                    "-movflags",
                    "+faststart",
                ]
                .map(String::from),
            );
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
    a.extend(["-frames:v".into(), count.to_string(), out.to_string()]);
    a
}

/// The video encoder for an MP4: H.264 by x264 or OpenH264 when this
/// ffmpeg has one (what browsers and players expect), else ffmpeg's own
/// choice for the container (MPEG-4 part 2 in builds without H.264).
fn video_codec(encoders: &str) -> &'static [&'static str] {
    let has = |name: &str| {
        encoders
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(name))
    };
    if has("libx264") {
        &["-c:v", "libx264", "-crf", "18", "-preset", "slow"]
    } else if has("libopenh264") {
        &["-c:v", "libopenh264", "-b:v", "16M"]
    } else {
        &["-q:v", "2"]
    }
}

/// Removes what `view.replay` wrote into `frames`, and the directory when
/// that leaves it empty. Nothing else in it is touched.
fn remove_frames(frames: &Path) {
    if let Ok(entries) = std::fs::read_dir(frames) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let ours = name.to_str().is_some_and(|n| {
                n == "captions.srt" || (n.starts_with("frame-") && n.ends_with(".png"))
            });
            if ours {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir(frames);
}

pub(crate) fn replay(mut args: Args) -> Result<Parsed<()>, CliError> {
    let mut params = Map::new();
    let mut extra: Option<Map<String, Value>> = None;
    let mut out: Option<String> = None;
    let mut keep_frames = false;
    let mut positionals = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        let (name, inline) = match arg {
            Arg::Positional(value) => {
                positionals.push(value);
                continue;
            }
            Arg::Flag { name, inline } => (name, inline),
        };
        if seen.contains(&name) {
            return Err(CliError::Usage(format!("`{name}` was given twice")));
        }
        seen.push(name.clone());
        match name.as_str() {
            "--help" | "-h" => {
                args::no_value(&name, inline.as_ref())?;
                return Ok(Parsed::Help);
            }
            "--every" => {
                args::no_value(&name, inline.as_ref())?;
                params.insert("every".into(), true.into());
            }
            "--keep-frames" => {
                args::no_value(&name, inline.as_ref())?;
                keep_frames = true;
            }
            "--out" => out = Some(args.value(&name, inline)?),
            "--size" => {
                let value = args.value(&name, inline)?;
                let (width, height) = crate::local::parse_size(&value)?;
                params.insert("width".into(), width.into());
                params.insert("height".into(), height.into());
            }
            "--view" | "--from" | "--to" => {
                let value = args.value(&name, inline)?;
                params.insert(name.trim_start_matches('-').into(), value.into());
            }
            "--orbit" => {
                let value = args.value(&name, inline)?;
                params.insert("orbit".into(), number(&name, &value)?.into());
            }
            "--fps" | "--frames-per-step" | "--hold" => {
                let value = args.value(&name, inline)?;
                let key = match name.as_str() {
                    "--fps" => "fps",
                    "--hold" => "hold",
                    _ => "framesPerStep",
                };
                params.insert(key.into(), integer(&name, &value)?.into());
            }
            "--params" => {
                let value = args.value(&name, inline)?;
                match serde_json::from_str::<Value>(&value) {
                    Ok(Value::Object(map)) => extra = Some(map),
                    _ => {
                        return Err(CliError::Usage(format!(
                            "`--params` must be a JSON object of view.replay params, got `{value}`"
                        )));
                    }
                }
            }
            _ => return Err(CliError::Usage(format!("unknown flag `{name}`"))),
        }
    }
    let [file] = positionals.as_slice() else {
        return Err(match positionals.get(1) {
            Some(extra) => CliError::Usage(format!("unexpected argument `{extra}`")),
            None => CliError::Usage("missing FILE".to_string()),
        });
    };
    let Some(out) = out else {
        return Err(CliError::Usage(
            "missing `--out FILE.mp4|FILE.gif|DIR`".to_string(),
        ));
    };
    // The flags win over the same keys in --params.
    if let Some(extra) = extra {
        for (key, value) in extra {
            params.entry(key).or_insert(value);
        }
    }
    let kind = output_kind(&out);
    let ffmpeg = match kind {
        Output::Frames => None,
        // Found before the frames are rendered, so a missing ffmpeg costs
        // nothing; the frames are made anyway, for whoever has it later.
        _ => Some(find_ffmpeg()),
    };
    let frames_dir = match kind {
        Output::Frames => out.clone(),
        _ => format!("{out}.frames"),
    };
    params.insert("dir".into(), frames_dir.clone().into());

    let mut session = Session::new(Workspace::unrestricted());
    session
        .execute("file.open", &json!({ "path": file }))
        .map_err(|e| CliError::Failed(e.message().to_string()))?;
    let mut result = session
        .execute("view.replay", &Value::Object(params))
        .map_err(|e| CliError::Failed(e.message().to_string()))?;
    let count = result.get("frames").and_then(Value::as_u64).unwrap_or(0);
    let fps = result.get("fps").and_then(Value::as_u64).unwrap_or(30);

    if let Some(ffmpeg) = ffmpeg {
        let Some(ffmpeg) = ffmpeg else {
            return Err(CliError::Failed(format!(
                "ffmpeg was not found on PATH (or set CRAFT3D_FFMPEG to it), so `{out}` was not made; the {count} frames are in `{frames_dir}` (frame-%05d.png at {fps} fps, with captions.srt)"
            )));
        };
        let frames = Path::new(&frames_dir);
        let codec = match kind {
            Output::Mp4 => Command::new(&ffmpeg)
                .args(["-hide_banner", "-encoders"])
                .output()
                .map(|o| video_codec(&String::from_utf8_lossy(&o.stdout)))
                .unwrap_or(&[]),
            _ => &[],
        };
        let make = |captions_track: bool| {
            Command::new(&ffmpeg)
                .args(ffmpeg_args(
                    kind,
                    frames,
                    count,
                    fps,
                    &out,
                    captions_track,
                    codec,
                ))
                .output()
                .map_err(|e| {
                    CliError::Failed(format!(
                        "cannot run `{}`: {e}; the frames are in `{frames_dir}`",
                        ffmpeg.display()
                    ))
                })
        };
        // An MP4 gets the captions as a subtitle track, when this ffmpeg
        // has the encoder for it (mov_text; some builds leave it out).
        let wanted = kind == Output::Mp4
            && std::fs::metadata(frames.join("captions.srt")).is_ok_and(|m| m.len() > 0);
        let mut ran = make(wanted)?;
        let mut captions_track = wanted;
        if wanted && !ran.status.success() {
            ran = make(false)?;
            captions_track = false;
        }
        if !ran.status.success() {
            let said = String::from_utf8_lossy(&ran.stderr);
            return Err(CliError::Failed(format!(
                "ffmpeg could not make `{out}` ({}): {}; the frames are in `{frames_dir}`",
                ran.status,
                said.trim()
            )));
        }
        if let Some(map) = result.as_object_mut()
            && kind == Output::Mp4
        {
            map.insert("captionsTrack".into(), captions_track.into());
        }
        if let Some(map) = result.as_object_mut() {
            map.insert("video".into(), out.clone().into());
            if keep_frames {
                map.insert("dir".into(), frames_dir.clone().into());
            } else {
                map.remove("dir");
                remove_frames(frames);
            }
        }
    }
    let line = serde_json::to_string(&result)
        .map_err(|e| CliError::Failed(format!("cannot write the result as JSON: {e}")))?;
    print(&line).map(Parsed::Run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_picks_the_output() {
        assert_eq!(output_kind("a/b.mp4"), Output::Mp4);
        assert_eq!(output_kind("B.GIF"), Output::Gif);
        assert_eq!(output_kind("frames"), Output::Frames);
        assert_eq!(output_kind("x.png"), Output::Frames);
    }

    #[test]
    fn ffmpeg_reads_the_frames_at_the_frame_rate_and_stops_at_the_count() {
        let a = ffmpeg_args(Output::Gif, Path::new("d"), 12, 24, "o.gif", false, &[]);
        let joined = a.join(" ");
        assert!(
            joined.contains("-framerate 24 -i d/frame-%05d.png"),
            "{joined}"
        );
        assert!(joined.ends_with("-frames:v 12 o.gif"), "{joined}");
        assert!(joined.contains("palettegen"), "{joined}");
        let a = ffmpeg_args(Output::Mp4, Path::new("d"), 3, 30, "o.mp4", true, &[]).join(" ");
        assert!(a.contains("yuv420p"), "{a}");
        assert!(
            a.contains("-i d/captions.srt") && a.contains("mov_text"),
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
    }

    #[test]
    fn h264_is_preferred_when_this_ffmpeg_has_it() {
        let list = " V....D libx264  x264\n V....D libopenh264  OpenH264\n";
        assert_eq!(video_codec(list)[1], "libx264");
        assert_eq!(
            video_codec(" V....D libopenh264  OpenH264\n")[1],
            "libopenh264"
        );
        assert_eq!(video_codec(" V....D mpeg4  MPEG-4\n"), &["-q:v", "2"]);
    }
}
