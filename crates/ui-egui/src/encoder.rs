//! Which video encoder `ui.record` asks `ffmpeg` for: the best one this
//! ffmpeg has and can use, per container, chosen once per ffmpeg and
//! remembered.
//!
//! An ffmpeg build carries the encoders its packager could ship: Fedora's
//! (`ffmpeg-free`) has no x264, only OpenH264, VP9, AV1 and hardware
//! encoders, and a hardware encoder it lists may have no device to run on
//! (VAAPI without an H.264 profile on an AMD laptop). So the choice is made
//! in two steps: what `ffmpeg -hide_banner -encoders` lists, then a trial of
//! each candidate in order, two small frames piped in as the recording pipes
//! them and encoded to nothing. The first that works is used; when none
//! does, `ui.record` itself fails, naming what it tried, before a frame is
//! drawn.
//!
//! The order, best first (`quality: screen` | `quality: web`):
//!
//! | Container | Encoder | screen | web |
//! |---|---|---|---|
//! | `.mp4` `.m4v` `.mov` `.mkv` | `libx264` | 4:4:4, CRF 12 | 4:2:0, CRF 18 |
//! | | `libopenh264` | 4:2:0, high profile, 0.3 bit a pixel, a keyframe a second | 4:2:0, 0.1 bit a pixel, a keyframe a second |
//! | | `h264_vaapi` (Linux), `h264_qsv`, `h264_nvenc` | 4:2:0, QP 16 | 4:2:0, QP 22 |
//! | | `mpeg4` (MPEG-4 part 2, in every ffmpeg) | 4:2:0, q 2 | 4:2:0, q 4 |
//! | `.webm` | `libvpx-vp9` | 4:4:4, CRF 16 | 4:2:0, CRF 30 |
//! | | `libsvtav1` | 4:2:0, CRF 24 | 4:2:0, CRF 35 |
//! | | `libaom-av1` | 4:4:4, CRF 20 | 4:2:0, CRF 32 |
//! | | `libvpx` (VP8) | 4:2:0, 0.3 bit a pixel | 4:2:0, 0.1 bit a pixel |
//! | `.gif` | `gif`, a palette made from the frames | | |
//! | `.avi` | `mpeg4` | q 2 | q 4 |
//!
//! Only `libx264`, VP9 and AOM's AV1 keep colour at full resolution
//! (4:4:4), what keeps thin coloured lines and text crisp; the others
//! subsample it (4:2:0), which halves the colour's resolution and needs
//! even sides. A frame of odd width or height is then padded to even by
//! repeating its last column or row (never cut: no pixel of the window is
//! lost, and the extra line looks like the edge rather than black that
//! would bleed into the colours beside it).
//!
//! OpenH264 is the weak one, and ffmpeg gives no handle on why: it
//! exposes neither OpenH264's screen-content mode nor its background
//! detection, and its rate control spends about 7 Mbit/s on a 1920 × 1014
//! window whatever bitrate it is asked for. Blocks it takes for background
//! are skipped, so after a change (the studio ground giving way to the
//! analysis colours) faint remnants of the old picture stay, and a smooth
//! environment gradient turns into flat blocks while the camera moves;
//! the frames themselves are clean (2026-10-31, the tank demo). A keyframe
//! every second bounds how long they last. It reaches about 40 dB on a
//! window recording where x264 at CRF 12 is near lossless; on an ffmpeg
//! without x264, `.webm` (VP9 4:4:4) is the crisp video. `mpeg4` plays in
//! desktop players but not in browsers. The PNG frames remain the lossless
//! way.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::record::VideoQuality;

/// How long listing the encoders may take.
const PROBE_PATIENCE: Duration = Duration::from_secs(10);
/// How long the trial of one encoder may take.
const TRIAL_PATIENCE: Duration = Duration::from_secs(15);
/// The trial's frames: two of this many pixels a side, mid grey.
const TRIAL_SIDE: u32 = 64;
/// The most of what ffmpeg says that an error repeats: its last lines.
const SAID_LINES: usize = 6;

/// An encoder and the arguments it is used with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Encoder {
    /// ffmpeg's name for it (`libx264`, `libopenh264`, ...).
    pub name: &'static str,
    /// Arguments before the input (a hardware device).
    pub before_input: Vec<String>,
    /// Arguments after the input: the codec, its quality, its pixels.
    pub args: Vec<String>,
    /// Whether it subsamples colour (4:2:0), so the frames need even sides.
    pub even: bool,
}

/// The containers `ui.record` writes, by extension.
fn family(ext: &str) -> Option<&'static str> {
    match ext {
        "mp4" | "m4v" | "mov" | "mkv" => Some("h264"),
        "webm" => Some("webm"),
        "gif" => Some("gif"),
        "avi" => Some("avi"),
        _ => None,
    }
}

/// Bits a second for `bits_per_pixel` at `size` and `fps`, as ffmpeg's
/// `-b:v` (at least 1 Mbit/s).
fn bitrate(size: [u32; 2], fps: f64, bits_per_pixel: f64) -> String {
    let pixels = f64::from(size[0]) * f64::from(size[1]);
    let bits = (pixels * fps.max(1.0) * bits_per_pixel).clamp(1e6, 2e9);
    format!("{}k", (bits / 1000.0).round() as u64)
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}

/// The encoders for a file with extension `ext`, best first, with their
/// arguments for frames of `size` at `fps`.
pub fn candidates(ext: &str, quality: VideoQuality, size: [u32; 2], fps: f64) -> Vec<Encoder> {
    let screen = quality == VideoQuality::Screen;
    let pick = |a: &'static str, b: &'static str| if screen { a } else { b };
    let rate = bitrate(size, fps, if screen { 0.3 } else { 0.1 });
    let keyframes = format!("{}", fps.round().clamp(1.0, 1000.0) as u32);
    let encoder = |name: &'static str, args: Vec<String>, even: bool| Encoder {
        name,
        before_input: Vec::new(),
        args,
        even,
    };
    let h264 = || {
        let mut list = vec![
            encoder(
                "libx264",
                strings(&[
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    pick("12", "18"),
                    "-pix_fmt",
                    pick("yuv444p", "yuv420p"),
                ]),
                !screen,
            ),
            encoder(
                "libopenh264",
                [
                    strings(&["-c:v", "libopenh264", "-profile:v", "high"]),
                    strings(&["-rc_mode", "quality", "-b:v"]),
                    vec![rate.clone()],
                    // A keyframe a second: what its background detection
                    // left stale is redrawn then (see the module's note).
                    vec!["-g".to_string(), keyframes.clone()],
                    strings(&["-pix_fmt", "yuv420p"]),
                ]
                .concat(),
                true,
            ),
        ];
        if cfg!(target_os = "linux") {
            list.push(Encoder {
                name: "h264_vaapi",
                before_input: strings(&["-vaapi_device", "/dev/dri/renderD128"]),
                args: strings(&[
                    "-vf",
                    "format=nv12,hwupload",
                    "-c:v",
                    "h264_vaapi",
                    "-qp",
                    pick("16", "22"),
                ]),
                even: true,
            });
        }
        list.push(encoder(
            "h264_qsv",
            strings(&[
                "-c:v",
                "h264_qsv",
                "-global_quality",
                pick("16", "22"),
                "-pix_fmt",
                "nv12",
            ]),
            true,
        ));
        list.push(encoder(
            "h264_nvenc",
            strings(&[
                "-c:v",
                "h264_nvenc",
                "-rc",
                "constqp",
                "-qp",
                pick("16", "22"),
                "-pix_fmt",
                "yuv420p",
            ]),
            true,
        ));
        list
    };
    let mpeg4 = || {
        encoder(
            "mpeg4",
            strings(&[
                "-c:v",
                "mpeg4",
                "-q:v",
                pick("2", "4"),
                "-pix_fmt",
                "yuv420p",
            ]),
            true,
        )
    };
    match family(ext) {
        Some("h264") => {
            let mut list = h264();
            list.push(mpeg4());
            if ext != "mkv" {
                // The index at the front: a player starts before the end.
                for e in &mut list {
                    e.args.extend(strings(&["-movflags", "+faststart"]));
                }
            }
            list
        }
        Some("webm") => vec![
            encoder(
                "libvpx-vp9",
                strings(&[
                    "-c:v",
                    "libvpx-vp9",
                    "-b:v",
                    "0",
                    "-row-mt",
                    "1",
                    "-crf",
                    pick("16", "30"),
                    "-pix_fmt",
                    pick("yuv444p", "yuv420p"),
                ]),
                !screen,
            ),
            encoder(
                "libsvtav1",
                strings(&[
                    "-c:v",
                    "libsvtav1",
                    "-preset",
                    "8",
                    "-crf",
                    pick("24", "35"),
                    "-pix_fmt",
                    "yuv420p",
                ]),
                true,
            ),
            encoder(
                "libaom-av1",
                strings(&[
                    "-c:v",
                    "libaom-av1",
                    "-cpu-used",
                    "8",
                    "-row-mt",
                    "1",
                    "-crf",
                    pick("20", "32"),
                    "-pix_fmt",
                    pick("yuv444p", "yuv420p"),
                ]),
                !screen,
            ),
            encoder(
                "libvpx",
                [
                    strings(&["-c:v", "libvpx", "-crf", pick("4", "10"), "-b:v"]),
                    vec![rate],
                    strings(&["-pix_fmt", "yuv420p"]),
                ]
                .concat(),
                true,
            ),
        ],
        // A palette made from the frames themselves.
        Some("gif") => vec![encoder(
            "gif",
            strings(&[
                "-filter_complex",
                "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a",
            ]),
            false,
        )],
        Some("avi") => vec![mpeg4()],
        _ => Vec::new(),
    }
}

/// Whether `listing` (`ffmpeg -encoders`) has the video encoder `name`.
pub fn lists(listing: &str, name: &str) -> bool {
    listing.lines().any(|line| {
        let mut words = line.split_whitespace();
        let flags = words.next().unwrap_or_default();
        flags.starts_with('V') && words.next() == Some(name)
    })
}

/// What a program said and how it ended, or why it did not.
struct Ran {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Runs `command` with `input` on its standard input, for at most
/// `patience`: a program that does not end is killed, never waited for.
fn run_bounded(
    mut command: std::process::Command,
    input: Vec<u8>,
    patience: Duration,
) -> Result<Ran, String> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::Builder::new()
            .name("craft3d-record-probe".to_string())
            .spawn(move || {
                let mut text = Vec::new();
                if let Some(mut pipe) = pipe {
                    // Bounded: what a listing says is a few tens of kB.
                    let _ = pipe.by_ref().take(1 << 20).read_to_end(&mut text);
                }
                String::from_utf8_lossy(&text).into_owned()
            })
            .ok()
    };
    let stdout = reader(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = reader(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    if let Some(mut stdin) = child.stdin.take() {
        // On a thread: a program that reads nothing must not block us.
        let _ = std::thread::Builder::new()
            .name("craft3d-record-probe".to_string())
            .spawn(move || {
                let _ = stdin.write_all(&input);
            });
    }
    let began = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if began.elapsed() < patience => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("it did not answer within {} s", patience.as_secs()));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    // Threads that read pipes a grandchild may hold open are not waited for.
    let join = |handle: Option<std::thread::JoinHandle<String>>| {
        handle
            .filter(|h| {
                let end = Instant::now() + Duration::from_secs(1);
                while !h.is_finished() && Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(2));
                }
                h.is_finished()
            })
            .and_then(|h| h.join().ok())
            .unwrap_or_default()
    };
    Ok(Ran {
        success: status.success(),
        stdout: join(stdout),
        stderr: join(stderr),
    })
}

/// The last lines of what ffmpeg said, on one line, for an error.
pub fn last_lines(said: &str) -> String {
    let lines: Vec<&str> = said
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let from = lines.len().saturating_sub(SAID_LINES);
    lines.get(from..).unwrap_or_default().join(" | ")
}

/// What has been learned about each ffmpeg: its listing, and per encoder
/// and quality whether the trial worked (`Err` says why not).
#[derive(Default)]
struct Known {
    listings: BTreeMap<PathBuf, String>,
    trials: BTreeMap<(PathBuf, &'static str, bool), Result<(), String>>,
}

fn known() -> &'static Mutex<Known> {
    static KNOWN: OnceLock<Mutex<Known>> = OnceLock::new();
    KNOWN.get_or_init(Mutex::default)
}

/// What `ffmpeg -hide_banner -encoders` lists, asked once per ffmpeg.
fn listing(ffmpeg: &Path) -> Result<String, String> {
    if let Some(listing) = known()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .listings
        .get(ffmpeg)
    {
        return Ok(listing.clone());
    }
    let mut command = std::process::Command::new(ffmpeg);
    command.args(["-hide_banner", "-encoders"]);
    let ran = run_bounded(command, Vec::new(), PROBE_PATIENCE)
        .map_err(|e| format!("cannot start `{}`: {e}", ffmpeg.display()))?;
    if !ran.success {
        return Err(format!(
            "`{} -encoders` failed: {}",
            ffmpeg.display(),
            last_lines(&ran.stderr)
        ));
    }
    known()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .listings
        .insert(ffmpeg.to_path_buf(), ran.stdout.clone());
    Ok(ran.stdout)
}

/// Encodes two small frames with `encoder` to nothing: whether it works on
/// this machine (a listed hardware encoder may have no device).
fn trial(ffmpeg: &Path, encoder: &Encoder, quality: VideoQuality) -> Result<(), String> {
    let key = (
        ffmpeg.to_path_buf(),
        encoder.name,
        quality == VideoQuality::Screen,
    );
    if let Some(known) = known()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .trials
        .get(&key)
    {
        return known.clone();
    }
    let side = TRIAL_SIDE;
    let mut command = std::process::Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error"])
        .args(&encoder.before_input)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-s"])
        .arg(format!("{side}x{side}"))
        .args(["-framerate", "30", "-i", "-"])
        .args(
            // `-movflags` means nothing to the null muxer.
            encoder
                .args
                .iter()
                .filter(|a| a.as_str() != "-movflags" && a.as_str() != "+faststart"),
        )
        .args(["-f", "null", "-"]);
    let frames = vec![128u8; (side * side * 3 * 2) as usize];
    let result = match run_bounded(command, frames, TRIAL_PATIENCE) {
        Ok(ran) if ran.success => Ok(()),
        Ok(ran) => Err(last_lines(&ran.stderr)),
        Err(e) => Err(e),
    };
    known()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .trials
        .insert(key, result.clone());
    result
}

/// The encoder `ffmpeg` writes a file with extension `ext` with: the first
/// of [`candidates`] it lists and whose trial works. An error says what was
/// tried and why each failed.
pub fn choose(
    ffmpeg: &Path,
    ext: &str,
    quality: VideoQuality,
    size: [u32; 2],
    fps: f64,
) -> Result<Encoder, String> {
    let all = candidates(ext, quality, size, fps);
    if all.is_empty() {
        return Err(format!("no video encoder is known for `.{ext}`"));
    }
    let listing = listing(ffmpeg)?;
    let mut tried = Vec::new();
    for encoder in all.iter() {
        if !lists(&listing, encoder.name) {
            continue;
        }
        match trial(ffmpeg, encoder, quality) {
            Ok(()) => return Ok(encoder.clone()),
            Err(why) => tried.push(format!("{} ({why})", encoder.name)),
        }
    }
    let wanted: Vec<&str> = all.iter().map(|e| e.name).collect();
    Err(format!(
        "`{}` has no encoder for `.{ext}` that works: it looks for {}; {}. Install an ffmpeg with one of them, record another container (.webm, .mp4), or PNG frames into a directory",
        ffmpeg.display(),
        wanted.join(", "),
        if tried.is_empty() {
            "this ffmpeg lists none of them".to_string()
        } else {
            format!("tried {}", tried.join("; "))
        }
    ))
}

/// `rgb` (`width × height`) padded to even sides by repeating its last
/// column and row; as it is when they are even.
pub fn pad_even(rgb: Vec<u8>, width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let (ew, eh) = (w + (w & 1), h + (h & 1));
    if (ew, eh) == (w, h) || w == 0 || h == 0 {
        return rgb;
    }
    let mut out = Vec::with_capacity(ew.saturating_mul(eh).saturating_mul(3));
    for y in 0..h {
        let row = rgb.get(y * w * 3..(y + 1) * w * 3).unwrap_or_default();
        out.extend_from_slice(row);
        if ew > w {
            let last = row.get(row.len().saturating_sub(3)..).unwrap_or_default();
            out.extend_from_slice(last);
        }
    }
    if eh > h {
        let from = (h - 1) * ew * 3;
        let last: Vec<u8> = out.get(from..from + ew * 3).unwrap_or_default().to_vec();
        out.extend_from_slice(&last);
    }
    out
}

/// The size a frame of `size` is encoded at by `encoder`.
pub fn encoded_size(encoder: &Encoder, size: [u32; 2]) -> [u32; 2] {
    if encoder.even {
        [size[0] + (size[0] & 1), size[1] + (size[1] & 1)]
    } else {
        size
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing, clippy::unwrap_used)]
    use super::*;

    #[test]
    fn odd_sides_are_padded_by_repeating_the_last_column_and_row() {
        // 3 × 1: red, green, blue.
        let rgb = vec![255, 0, 0, 0, 255, 0, 0, 0, 255];
        let padded = pad_even(rgb, 3, 1);
        assert_eq!(padded.len(), 4 * 2 * 3);
        let row = [255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 255];
        assert_eq!(&padded[..12], &row);
        assert_eq!(&padded[12..], &row);
        // Even sides are left alone.
        let even = vec![7u8; 2 * 2 * 3];
        assert_eq!(pad_even(even.clone(), 2, 2), even);
    }

    #[test]
    fn the_order_puts_x264_first_and_mpeg4_last_for_an_mp4() {
        let names = |ext, q| {
            candidates(ext, q, [1920, 1013], 30.0)
                .iter()
                .map(|e| e.name)
                .collect::<Vec<_>>()
        };
        let mp4 = names("mp4", VideoQuality::Screen);
        assert_eq!(mp4.first(), Some(&"libx264"));
        assert_eq!(mp4.get(1), Some(&"libopenh264"));
        assert_eq!(mp4.last(), Some(&"mpeg4"));
        assert_eq!(names("webm", VideoQuality::Web)[0], "libvpx-vp9");
        assert!(names("png", VideoQuality::Web).is_empty());
        // x264 at screen quality keeps colour whole, so odd sides are fine;
        // OpenH264 subsamples it, so they are padded.
        let x264 = &candidates("mp4", VideoQuality::Screen, [1920, 1013], 30.0)[0];
        assert!(!x264.even);
        assert_eq!(encoded_size(x264, [1920, 1013]), [1920, 1013]);
        let openh264 = &candidates("mp4", VideoQuality::Screen, [1920, 1013], 30.0)[1];
        assert_eq!(encoded_size(openh264, [1920, 1013]), [1920, 1014]);
    }

    #[test]
    fn a_listing_is_read_by_its_video_lines() {
        let listing = " V....D libopenh264   OpenH264\n A....D aac   AAC\n";
        assert!(lists(listing, "libopenh264"));
        assert!(!lists(listing, "libx264"));
        assert!(!lists(listing, "aac"));
    }
}
