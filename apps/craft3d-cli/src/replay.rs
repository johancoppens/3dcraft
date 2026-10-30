//! `craft3d-cli replay`: a showcase video of how a document is built.
//!
//! The engine's `view.replay` writes the frames (PNG) and a subtitle file;
//! the engine never starts a program. Turning the frames into an MP4, a
//! WebM or a GIF is done here (`video.rs`), by `ffmpeg` when it is
//! installed: found on `PATH`, or named by `CRAFT3D_FFMPEG`. Without it the
//! frames are kept and the error says where they are.

use crate::args::{self, Arg, Args};
use crate::video::{Output, find_ffmpeg, output_kind};
use crate::{CliError, Parsed, print};
use craft3d_engine::Workspace;
use serde_json::{Map, Value, json};
use std::path::Path;

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
            "--view" | "--from" | "--to" | "--renderer" => {
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
            "missing `--out FILE.mp4|FILE.webm|FILE.gif|DIR`".to_string(),
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

    let mut session = crate::session(Workspace::unrestricted());
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
        let captions_track =
            crate::video::make(&ffmpeg, kind, frames, count, fps, &out).map_err(|why| {
                CliError::Failed(format!(
                    "ffmpeg could not make `{out}` ({why}); the frames are in `{frames_dir}`"
                ))
            })?;
        if let Some(map) = result.as_object_mut()
            && matches!(kind, Output::Mp4 | Output::Webm)
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
