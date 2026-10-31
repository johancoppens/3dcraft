//! Recording the window: its own frames, panels and all, painted by
//! egui's wgpu renderer into a texture of the window's size and read back
//! (`ui.record`, `ui.advance`, and `ui.screenshot {window: true}`).
//!
//! A picture taken from the compositor needs the window on screen: a
//! Wayland compositor sends a window it does not show (covered,
//! minimised, on another workspace) no frame callbacks, and then nothing
//! is drawn at all. So the host runs the frame itself, outside the
//! toolkit's loop: one pass of the shell on the window's own
//! [`egui::Context`], tessellated and painted by the window's own
//! [`egui_wgpu::Renderer`] (the textures the shell registered with it,
//! the GPU viewport's among them, are there) into an [`Offscreen`]
//! texture, then read back. Nothing here depends on the window being
//! shown.
//!
//! Two clocks:
//!
//! - real time (`ui.record {path, fps}`): the host runs a pass every
//!   `1/fps` seconds of the wall clock, with the input the window got;
//! - frame-exact (`ui.record {path, step: true}`): nothing is drawn until
//!   `ui.advance {frames, dt}`, and then each frame is a pass at a time
//!   `dt` after the last one, run once what the window shows has settled
//!   (no command running, the scene built, the viewport's picture done).
//!   The history player, flights and the orbit follow egui's input time,
//!   so an agent gets the same frames for the same requests.
//!
//! The frames go to a sequence of PNGs in a directory, or through a pipe
//! to an external `ffmpeg` (`CRAFT3D_FFMPEG`, else the one on `PATH`) for
//! a video file: no codec is linked in. They are written on a thread of
//! their own, a few frames behind at most, so a slow disk holds up the
//! recording rather than filling memory.

use craft3d_engine::Session;
use egui_wgpu::wgpu;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

/// Frames a second a recording takes at most, and by default.
pub const MAX_FPS: f64 = 240.0;
pub const DEFAULT_FPS: f64 = 30.0;
/// Most frames one `ui.advance` draws.
pub const MAX_ADVANCE: u64 = 100_000;
/// The longest step of a frame-exact clock, in seconds.
pub const MAX_DT: f64 = 60.0;
/// Most pixels a side of a recorded frame has: a texture wgpu can make on
/// every device.
pub const MAX_SIDE: u32 = 8192;
/// Frames waiting for the writer before a new one waits for it.
const QUEUED_FRAMES: usize = 4;
/// Extensions written through `ffmpeg`.
const VIDEO: [&str; 7] = ["mp4", "mkv", "mov", "webm", "gif", "avi", "m4v"];

/// A frame read back: `width × height` pixels of RGB, row by row from the
/// top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Frame {
    /// The part of the frame within `rect` (pixels: x, y, width, height),
    /// clamped to the frame. `None` when nothing of it is in the frame.
    pub fn crop(&self, [x, y, w, h]: [u32; 4]) -> Option<Frame> {
        let x1 = x.saturating_add(w).min(self.width);
        let y1 = y.saturating_add(h).min(self.height);
        if x >= x1 || y >= y1 {
            return None;
        }
        let row = usize::try_from(self.width).ok()?.checked_mul(3)?;
        let (from, to) = (
            usize::try_from(x).ok()?.checked_mul(3)?,
            usize::try_from(x1).ok()?.checked_mul(3)?,
        );
        let mut rgb = Vec::with_capacity(to.saturating_sub(from).saturating_mul((y1 - y) as usize));
        for line in y..y1 {
            let start = usize::try_from(line).ok()?.checked_mul(row)?;
            rgb.extend_from_slice(self.rgb.get(start + from..start + to)?);
        }
        Some(Frame {
            width: x1 - x,
            height: y1 - y,
            rgb,
        })
    }

    /// The frame as egui shows an image.
    pub fn color_image(&self) -> egui::ColorImage {
        let pixels = self
            .rgb
            .as_chunks::<3>()
            .0
            .iter()
            .map(|[r, g, b]| egui::Color32::from_rgb(*r, *g, *b))
            .collect();
        egui::ColorImage::new([self.width as usize, self.height as usize], pixels)
    }
}

/// The texture a window's frames are painted into, on the window's own
/// device, and read back from.
pub struct Offscreen {
    state: egui_wgpu::RenderState,
    target: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
}

impl Offscreen {
    /// On the device, with the renderer, of `state`: the window's, so the
    /// textures the shell registered with it are there.
    pub fn new(state: egui_wgpu::RenderState) -> Self {
        Self {
            state,
            target: None,
        }
    }

    /// Paints `output`, a pass of `ctx`, into a texture of `size` pixels,
    /// and reads it back when `read`. The textures the pass set or freed
    /// are applied to the renderer either way: the window's own frames
    /// will never see those changes again.
    pub fn paint(
        &mut self,
        ctx: &egui::Context,
        mut output: egui::FullOutput,
        size: [u32; 2],
        read: bool,
    ) -> Result<Option<Frame>, String> {
        let [width, height] = size;
        let state = self.state.clone();
        let mut renderer = state.renderer.write();
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(&state.device, &state.queue, *id, delta);
            }
        }
        let shapes = std::mem::take(&mut output.shapes);
        let jobs = ctx.tessellate(shapes, output.pixels_per_point);
        let freed = std::mem::take(&mut output.textures_delta.free);
        // Applied above: epaint insists that a delta is not dropped unseen.
        output.textures_delta.clear();
        let painted = (|| {
            if !read {
                return Ok(None);
            }
            if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
                return Err(format!(
                    "a frame of {width} × {height} pixels cannot be painted (1 to {MAX_SIDE} a side)"
                ));
            }
            let view = self.target(size);
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: size,
                pixels_per_point: output.pixels_per_point,
            };
            let mut encoder = state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            let mut commands =
                renderer.update_buffers(&state.device, &state.queue, &mut encoder, &jobs, &screen);
            {
                let mut pass = encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("craft3d-offscreen-window"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    })
                    .forget_lifetime();
                renderer.render(&mut pass, &jobs, &screen);
            }
            let texture = self
                .target
                .as_ref()
                .map(|(texture, _, _)| texture.clone())
                .ok_or("the offscreen texture is gone")?;
            let frame = read_back(&state, &texture, size, encoder, &mut commands)?;
            Ok(Some(frame))
        })();
        for id in &freed {
            renderer.free_texture(id);
        }
        painted
    }

    /// The view of a texture of `size`, made again only when the size
    /// changes.
    fn target(&mut self, size: [u32; 2]) -> wgpu::TextureView {
        if let Some((_, view, at)) = &self.target
            && *at == size
        {
            return view.clone();
        }
        let texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("craft3d-offscreen-window"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.state.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.target = Some((texture, view.clone(), size));
        view
    }
}

/// Copies `texture` into a buffer, submits `encoder` after `commands`,
/// waits for the device and returns the pixels as RGB.
fn read_back(
    state: &egui_wgpu::RenderState,
    texture: &wgpu::Texture,
    [width, height]: [u32; 2],
    mut encoder: wgpu::CommandEncoder,
    commands: &mut Vec<wgpu::CommandBuffer>,
) -> Result<Frame, String> {
    let bgra = match state.target_format {
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => true,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => false,
        other => {
            return Err(format!(
                "frames of the format {other:?} cannot be read back"
            ));
        }
    };
    let row = width.checked_mul(4).ok_or("the frame is too wide")?;
    let padded = row
        .div_ceil(256)
        .checked_mul(256)
        .ok_or("the frame is too wide")?;
    let buffer = state.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("craft3d-offscreen-read-back"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    commands.push(encoder.finish());
    state.queue.submit(std::mem::take(commands));
    let slice = buffer.slice(..);
    let mapped: Arc<Mutex<Option<Result<(), String>>>> = Arc::new(Mutex::new(None));
    let told = Arc::clone(&mapped);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        *told.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(result.map_err(|e| e.to_string()));
    });
    state
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(READ_BACK_PATIENCE),
        })
        .map_err(|e| format!("the device did not finish the frame: {e}"))?;
    match mapped.lock().unwrap_or_else(PoisonError::into_inner).take() {
        Some(Ok(())) => {}
        Some(Err(e)) => return Err(format!("the frame could not be read back: {e}")),
        None => return Err("the frame was not read back".to_string()),
    }
    let data = slice
        .get_mapped_range()
        .map_err(|e| format!("the frame could not be read back: {e}"))?;
    let pixels = (width as usize).saturating_mul(height as usize);
    let mut rgb = Vec::with_capacity(pixels.saturating_mul(3));
    for y in 0..height as usize {
        let start = y.saturating_mul(padded as usize);
        let line = data
            .get(start..start.saturating_add(row as usize))
            .ok_or("the frame read back is shorter than asked for")?;
        for [first, g, third, _] in line.as_chunks::<4>().0 {
            if bgra {
                rgb.extend_from_slice(&[*third, *g, *first]);
            } else {
                rgb.extend_from_slice(&[*first, *g, *third]);
            }
        }
    }
    drop(data);
    buffer.unmap();
    Ok(Frame { width, height, rgb })
}

/// The size in pixels of a window whose input is `input`, at
/// `pixels_per_point`.
pub fn input_pixels(input: &egui::RawInput, pixels_per_point: f32) -> Option<[u32; 2]> {
    let rect = input.screen_rect?;
    let side = |points: f32| {
        let pixels = (points * pixels_per_point).round();
        (pixels.is_finite() && pixels >= 1.0 && pixels <= MAX_SIDE as f32).then_some(pixels as u32)
    };
    Some([side(rect.width())?, side(rect.height())?])
}

/// What `ui.record` asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum RecordAsk {
    /// No parameters: how the recording stands.
    Status,
    Start(RecordOptions),
    Stop,
}

/// How a recording is made.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordOptions {
    /// Under the write root: a directory for PNGs, or a video file.
    pub path: String,
    pub fps: f64,
    /// Frame-exact: frames only on `ui.advance`.
    pub step: bool,
    /// The whole window, or only the viewport.
    pub include_ui: bool,
    /// The ffmpeg to use instead of the one found ([`find_ffmpeg`]): for
    /// tests. Never from a request.
    pub ffmpeg: Option<PathBuf>,
    /// How many frames of its still a frame-exact frame waits for the
    /// viewport to average (the studio look's `accumulate`): 1, none, by
    /// default, since a model that moves starts its average again every
    /// frame anyway.
    pub accumulate: u32,
    /// How a video is encoded.
    pub quality: VideoQuality,
}

/// How `ui.record` encodes a video (the encoder and its settings per
/// container: [`crate::encoder`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoQuality {
    /// For a screen recording: text and edges crisp, colours not
    /// subsampled where the encoder allows it (x264 4:4:4 at CRF 12);
    /// some players (browsers, QuickTime) do not play 4:4:4.
    Screen,
    /// For the web: 4:2:0, playable everywhere.
    Web,
}

fn object<'a>(
    method: &str,
    params: Option<&'a Value>,
    allowed: &[&str],
) -> Result<Option<&'a Map<String, Value>>, String> {
    let map = match params {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(map)) => map,
        Some(_) => return Err(format!("{method}: parameters must be a JSON object")),
    };
    if let Some(unknown) = map.keys().find(|key| !allowed.contains(&key.as_str())) {
        let unknown: String = unknown.chars().take(40).collect();
        return Err(format!(
            "{method}: unknown parameter `{unknown}`; its parameters are: {}",
            allowed.join(", ")
        ));
    }
    Ok(Some(map))
}

fn flag(method: &str, map: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("{method}: `{key}` must be true or false")),
    }
}

fn number(
    method: &str,
    map: &Map<String, Value>,
    key: &str,
    range: (f64, f64),
) -> Result<Option<f64>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|v| v.is_finite() && *v > range.0 && *v <= range.1)
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "{method}: `{key}` must be a number above {} and at most {}",
                    range.0, range.1
                )
            }),
    }
}

/// Reads the parameters of `ui.record`.
pub fn record_params(params: Option<&Value>) -> Result<RecordAsk, String> {
    let method = "ui.record";
    let Some(map) = object(
        method,
        params,
        &[
            "path",
            "fps",
            "start",
            "stop",
            "step",
            "includeUi",
            "accumulate",
            "quality",
        ],
    )?
    else {
        return Ok(RecordAsk::Status);
    };
    if map.is_empty() {
        return Ok(RecordAsk::Status);
    }
    let start = flag(method, map, "start")?;
    let stop = flag(method, map, "stop")?;
    if start == Some(true) && stop == Some(true) {
        return Err(format!("{method}: give `start` or `stop`, not both"));
    }
    if stop == Some(true) {
        if let Some(other) = map.keys().find(|k| *k != "stop" && *k != "start") {
            return Err(format!(
                "{method}: `stop` takes no other parameter (`{other}`)"
            ));
        }
        return Ok(RecordAsk::Stop);
    }
    if start == Some(false) || stop == Some(false) {
        return Err(format!(
            "{method}: start with {{path, ...}} (or start: true), stop with {{stop: true}}"
        ));
    }
    let path = match map.get("path") {
        Some(Value::String(path)) if !path.trim().is_empty() => path.clone(),
        None | Some(Value::Null) => {
            return Err(format!(
                "{method}: `path` is missing: a directory for PNG frames, or a video file (.mp4, .webm, ...) written by ffmpeg"
            ));
        }
        Some(_) => return Err(format!("{method}: `path` must be a non-empty string")),
    };
    let fps = number(method, map, "fps", (0.0, MAX_FPS))?.unwrap_or(DEFAULT_FPS);
    if fps < 1.0 {
        return Err(format!("{method}: `fps` must be at least 1"));
    }
    Ok(RecordAsk::Start(RecordOptions {
        path,
        fps,
        step: flag(method, map, "step")?.unwrap_or(false),
        include_ui: flag(method, map, "includeUi")?.unwrap_or(true),
        ffmpeg: None,
        accumulate: match map.get("accumulate") {
            None | Some(Value::Null) => 1,
            Some(value) => value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| (1..=MAX_ACCUMULATE).contains(n))
                .ok_or_else(|| {
                    format!(
                        "{method}: `accumulate` must be a whole number from 1 to {MAX_ACCUMULATE}"
                    )
                })?,
        },
        quality: match map.get("quality") {
            None | Some(Value::Null) => VideoQuality::Screen,
            Some(Value::String(q)) if q == "screen" => VideoQuality::Screen,
            Some(Value::String(q)) if q == "web" => VideoQuality::Web,
            Some(_) => return Err(format!("{method}: `quality` is \"screen\" or \"web\"")),
        },
    }))
}

/// Reads the parameters of `ui.advance`: how many frames, and the time
/// between them in seconds (`1/fps` by default).
pub fn advance_params(params: Option<&Value>, fps: f64) -> Result<(u64, f64, f64), String> {
    let method = "ui.advance";
    let map = object(method, params, &["frames", "dt", "timeout"])?;
    let frames = match map.and_then(|m| m.get("frames")) {
        None | Some(Value::Null) => 1,
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=MAX_ADVANCE).contains(n))
            .ok_or_else(|| {
                format!("{method}: `frames` must be a whole number from 1 to {MAX_ADVANCE}")
            })?,
    };
    let dt = match map {
        Some(map) => number(method, map, "dt", (0.0, MAX_DT))?,
        None => None,
    };
    let dt = dt.unwrap_or(1.0 / fps.max(1.0));
    let timeout = match map {
        Some(map) => number(method, map, "timeout", (0.0, MAX_ADVANCE_SECONDS))?,
        None => None,
    }
    .unwrap_or(DEFAULT_ADVANCE_SECONDS);
    Ok((frames, dt, timeout))
}

/// How long one `ui.advance` draws before it answers with the frames it
/// drew (the client asks again for the rest), by default and at most:
/// well within a control client's 120 s.
pub const DEFAULT_ADVANCE_SECONDS: f64 = 30.0;
pub const MAX_ADVANCE_SECONDS: f64 = 110.0;
/// Most frames a frame-exact frame waits for the viewport to average.
pub const MAX_ACCUMULATE: u32 = 256;

/// Where the frames go.
enum Sink {
    Png(PathBuf),
    /// ffmpeg's input; `even`: frames padded to even sides first.
    Ffmpeg {
        stdin: std::process::ChildStdin,
        even: bool,
    },
}

/// How long a frame waits for room in the writer's queue, and how long
/// stopping waits for the writer and for ffmpeg, before it is an error
/// (a disk that hangs, an ffmpeg that stalls): never a window that hangs.
pub const WRITER_PATIENCE: std::time::Duration = std::time::Duration::from_secs(20);
/// The most of what ffmpeg says that is kept, its end: for the error.
const STDERR_TAIL: usize = 4096;
/// How long the device may take to finish a frame before reading it back
/// is an error.
const READ_BACK_PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

/// The `ffmpeg` to pipe a video through: `CRAFT3D_FFMPEG`, else the first
/// `ffmpeg` on `PATH`.
pub fn find_ffmpeg() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CRAFT3D_FFMPEG").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    })
}

/// Whether `path` names a video, written through `ffmpeg`.
fn is_video(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO.contains(&e.to_ascii_lowercase().as_str()))
}

/// A recording in progress.
pub struct Recording {
    pub options: RecordOptions,
    /// The size of every frame, fixed when it starts.
    pub size: [u32; 2],
    /// The part of the window recorded (pixels), when not all of it.
    pub crop: Option<[u32; 4]>,
    /// Frames handed to the writer.
    pub frames: u64,
    /// The frame-exact clock: egui's input time of the last frame.
    pub time: f64,
    /// Frames real time skipped (a pass that took longer than a frame).
    pub skipped: u64,
    send: Option<SyncSender<Vec<u8>>>,
    writer: Option<JoinHandle<()>>,
    failed: Arc<Mutex<Option<String>>>,
    /// ffmpeg, while it runs, and the end of what it said (drained on a
    /// thread of its own, so a talkative ffmpeg never blocks on its pipe).
    child: Option<std::process::Child>,
    said: Arc<Said>,
    format: &'static str,
    /// The encoder of a video and the size it encodes (even sides for
    /// one that subsamples colour: [`crate::encoder`]).
    encoder: Option<(&'static str, [u32; 2])>,
}

/// What ffmpeg said (its end), and whether it has stopped saying it.
#[derive(Default)]
struct Said {
    text: Mutex<String>,
    done: std::sync::atomic::AtomicBool,
}

impl Said {
    /// The last lines, once ffmpeg has ended (waiting at most `patience`
    /// for that).
    fn last_lines(&self, patience: std::time::Duration) -> String {
        let end = std::time::Instant::now() + patience;
        while !self.done.load(std::sync::atomic::Ordering::Acquire)
            && std::time::Instant::now() < end
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        crate::encoder::last_lines(&self.text.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Recording {
    /// Starts writing frames of `size` pixels (the window's, or `crop`'s)
    /// where `options` say, through `session`'s write root. `time` is the
    /// clock's start.
    pub fn start(
        session: &Session,
        options: RecordOptions,
        window: [u32; 2],
        crop: Option<[u32; 4]>,
        time: f64,
    ) -> Result<Self, String> {
        let method = "ui.record";
        let size = match crop {
            Some([_, _, w, h]) => [w, h],
            None => window,
        };
        if size[0] == 0 || size[1] == 0 {
            return Err(format!(
                "{method}: there is nothing to record (the viewport has no size yet)"
            ));
        }
        let resolved = session
            .workspace()
            .resolve_write(&options.path)
            .map_err(|e| format!("{method}: {e}"))?;
        let sink = if is_video(&options.path) {
            let ffmpeg = options.ffmpeg.clone().or_else(find_ffmpeg).ok_or_else(|| {
                format!(
                    "{method}: a video needs ffmpeg, which is not on PATH (set CRAFT3D_FFMPEG to it), or record PNG frames into a directory"
                )
            })?;
            let ext = Path::new(&options.path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            // Before a frame is drawn: an ffmpeg without a working encoder
            // for the container is refused here, saying what it lacks.
            let encoder = crate::encoder::choose(&ffmpeg, &ext, options.quality, size, options.fps)
                .map_err(|e| format!("{method}: {e}"))?;
            let encoded = crate::encoder::encoded_size(&encoder, size);
            // ffmpeg makes no directory: the file's own is made here.
            if let Some(parent) = resolved.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "{method}: cannot make the directory of `{}`: {e}",
                        options.path
                    )
                })?;
            }
            let mut command = std::process::Command::new(&ffmpeg);
            command
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(&encoder.before_input)
                .args(["-f", "rawvideo", "-pix_fmt", "rgb24"])
                .arg("-s")
                .arg(format!("{}x{}", encoded[0], encoded[1]))
                .arg("-framerate")
                .arg(format!("{}", options.fps))
                .args(["-i", "-"])
                .args(&encoder.args);
            command
                .arg(&resolved)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped());
            let mut child = command
                .spawn()
                .map_err(|e| format!("{method}: cannot start `{}`: {e}", ffmpeg.display()))?;
            let Some(stdin) = child.stdin.take() else {
                end_child(&mut child);
                return Err(format!("{method}: ffmpeg took no input"));
            };
            (
                Sink::Ffmpeg {
                    stdin,
                    even: encoder.even,
                },
                "ffmpeg",
                Some(child),
                Some((encoder.name, encoded)),
            )
        } else {
            std::fs::create_dir_all(&resolved).map_err(|e| {
                format!(
                    "{method}: cannot make the directory `{}`: {e}",
                    options.path
                )
            })?;
            (Sink::Png(resolved), "png", None, None)
        };
        let (sink, format, mut child, encoder) = sink;
        let said = Arc::new(Said::default());
        if let Some(stderr) = child.as_mut().and_then(|c| c.stderr.take()) {
            let keep = Arc::clone(&said);
            let drained = std::thread::Builder::new()
                .name("craft3d-record-ffmpeg".to_string())
                .spawn(move || drain(stderr, &keep));
            if let Err(e) = drained {
                if let Some(child) = child.as_mut() {
                    end_child(child);
                }
                return Err(format!("{method}: cannot read what ffmpeg says: {e}"));
            }
        }
        let failed = Arc::new(Mutex::new(None));
        let (send, receive) = sync_channel::<Vec<u8>>(QUEUED_FRAMES);
        let writer = match spawn_writer(sink, receive, size, Arc::clone(&failed), Arc::clone(&said))
        {
            Ok(writer) => writer,
            Err(e) => {
                if let Some(child) = child.as_mut() {
                    end_child(child);
                }
                return Err(format!("{method}: cannot start the writer: {e}"));
            }
        };
        Ok(Self {
            options,
            size,
            crop,
            frames: 0,
            time,
            skipped: 0,
            send: Some(send),
            writer: Some(writer),
            failed,
            child,
            said,
            format,
            encoder,
        })
    }

    /// Hands one frame of the window to the writer (cropped to the part
    /// recorded); waits while the writer is [`QUEUED_FRAMES`] behind.
    pub fn push(&mut self, frame: &Frame) -> Result<(), String> {
        if let Some(error) = self.failed() {
            return Err(error);
        }
        let frame = match self.crop {
            Some(rect) => frame
                .crop(rect)
                .ok_or("ui.record: the part recorded is outside the window")?,
            None => frame.clone(),
        };
        // A window resized while it records: the frames keep their size,
        // the picture is cut or padded with black at its right and bottom.
        let rgb = if [frame.width, frame.height] == self.size {
            frame.rgb
        } else {
            fit(&frame, self.size)
        };
        let send = self
            .send
            .as_ref()
            .ok_or("ui.record: the recording has ended")?;
        // Never waits long: a writer that takes no frame within its
        // patience (a disk or an ffmpeg that hangs) is an error.
        let mut rgb = rgb;
        let began = std::time::Instant::now();
        loop {
            match send.try_send(rgb) {
                Ok(()) => break,
                Err(std::sync::mpsc::TrySendError::Full(back)) => {
                    if began.elapsed() >= WRITER_PATIENCE {
                        return Err(format!(
                            "ui.record: the writer took no frame for {} s (is the disk or ffmpeg stuck?)",
                            WRITER_PATIENCE.as_secs()
                        ));
                    }
                    rgb = back;
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                    return Err(self
                        .failed()
                        .unwrap_or_else(|| "ui.record: the writer stopped".to_string()));
                }
            }
        }
        self.frames = self.frames.saturating_add(1);
        Ok(())
    }

    fn failed(&self) -> Option<String> {
        self.failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// How it stands, as `ui.record` replies.
    pub fn status(&self) -> Value {
        json!({
            "recording": true,
            "path": self.options.path,
            "format": self.format,
            "fps": self.options.fps,
            "step": self.options.step,
            "includeUi": self.options.include_ui,
            "accumulate": self.options.accumulate,
            "quality": match self.options.quality {
                VideoQuality::Screen => "screen",
                VideoQuality::Web => "web",
            },
            "width": self.size[0],
            "height": self.size[1],
            "encoder": self.encoder.map(|(name, _)| name),
            "encoded": self.encoder.map(|(_, [w, h])| json!([w, h])),
            "frames": self.frames,
            "skipped": self.skipped,
            "time": self.time,
        })
    }

    /// Ends it: waits for the frames still queued to be written (and for
    /// ffmpeg to finish the file), and says what was written.
    pub fn stop(mut self) -> Result<Value, String> {
        let mut status = self.status();
        self.finish();
        if let Some(map) = status.as_object_mut() {
            map.insert("recording".into(), false.into());
        }
        match self.failed() {
            Some(error) => Err(error),
            None => Ok(status),
        }
    }

    /// Closes the queue, waits for the writer and for ffmpeg, each
    /// within [`WRITER_PATIENCE`]: an ffmpeg that does not end then is
    /// killed (which also frees a writer stuck on its pipe).
    fn finish(&mut self) {
        drop(self.send.take());
        let deadline = std::time::Instant::now() + WRITER_PATIENCE;
        if let Some(writer) = self.writer.take() {
            while !writer.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            if writer.is_finished() {
                if writer.join().is_err() {
                    self.fail("ui.record: the writer failed".to_string());
                }
            } else {
                // Stuck on a pipe nobody reads (a process ffmpeg started
                // may hold it open after ffmpeg is killed): left to end
                // when the pipe closes, never waited for.
                self.fail("ui.record: the writer did not finish in time".to_string());
                if let Some(child) = self.child.as_mut() {
                    let _ = child.kill();
                }
                drop(writer);
            }
        }
        let Some(mut child) = self.child.take() else {
            return;
        };
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                _ => {
                    end_child(&mut child);
                    break None;
                }
            }
        };
        match status {
            Some(status) if status.success() => {}
            Some(status) => {
                let said = self.said.last_lines(std::time::Duration::from_secs(1));
                self.fail(format!("ui.record: ffmpeg failed ({status}): {said}"));
            }
            None => {
                self.fail("ui.record: ffmpeg did not finish in time and was stopped".to_string())
            }
        }
    }

    fn fail(&self, error: String) {
        let mut failed = self.failed.lock().unwrap_or_else(PoisonError::into_inner);
        if failed.is_none() {
            *failed = Some(error);
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.finish();
    }
}

/// `frame` in a picture of `size`: cut at its right and bottom, or padded
/// with black there.
fn fit(frame: &Frame, [width, height]: [u32; 2]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut rgb = vec![0; w.saturating_mul(h).saturating_mul(3)];
    let copy = (frame.width as usize).min(w).saturating_mul(3);
    for y in 0..(frame.height as usize).min(h) {
        let from = y.saturating_mul(frame.width as usize).saturating_mul(3);
        let to = y.saturating_mul(w).saturating_mul(3);
        if let (Some(source), Some(target)) = (
            frame.rgb.get(from..from.saturating_add(copy)),
            rgb.get_mut(to..to.saturating_add(copy)),
        ) {
            target.copy_from_slice(source);
        }
    }
    rgb
}

/// The writer's thread: each frame as a PNG `frame-000001.png`, … in the
/// directory, or into ffmpeg's input.
fn spawn_writer(
    sink: Sink,
    frames: Receiver<Vec<u8>>,
    [width, height]: [u32; 2],
    failed: Arc<Mutex<Option<String>>>,
    said: Arc<Said>,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("craft3d-record".to_string())
        .spawn(move || {
            let fail = |error: String| {
                let mut failed = failed.lock().unwrap_or_else(PoisonError::into_inner);
                if failed.is_none() {
                    *failed = Some(error);
                }
            };
            match sink {
                Sink::Png(dir) => {
                    let mut n: u64 = 0;
                    while let Ok(rgb) = frames.recv() {
                        n = n.saturating_add(1);
                        let written = craft3d_render::encode_png(width, height, &rgb)
                            .map_err(|e| e.to_string())
                            .and_then(|png| {
                                let path = dir.join(format!("frame-{n:06}.png"));
                                std::fs::write(&path, png)
                                    .map_err(|e| format!("cannot write `{}`: {e}", path.display()))
                            });
                        if let Err(error) = written {
                            fail(format!("ui.record: {error}"));
                            return;
                        }
                    }
                }
                Sink::Ffmpeg { mut stdin, even } => {
                    use std::io::Write;
                    while let Ok(rgb) = frames.recv() {
                        let rgb = if even {
                            crate::encoder::pad_even(rgb, width, height)
                        } else {
                            rgb
                        };
                        if let Err(e) = stdin.write_all(&rgb) {
                            // Why it stopped is in what it said as it ended.
                            let said = said.last_lines(std::time::Duration::from_secs(2));
                            fail(format!(
                                "ui.record: ffmpeg stopped taking frames: {e}{}",
                                if said.is_empty() {
                                    String::new()
                                } else {
                                    format!("; ffmpeg said: {said}")
                                }
                            ));
                            return;
                        }
                    }
                    // Closing its input ends the video.
                    drop(stdin);
                }
            }
        })
}

/// Kills `child` and waits for it.
fn end_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Reads what ffmpeg says until it ends, keeping the last
/// [`STDERR_TAIL`] bytes.
fn drain(mut stderr: std::process::ChildStderr, keep: &Said) {
    use std::io::Read;
    let mut buffer = [0u8; 4096];
    while let Ok(n) = stderr.read(&mut buffer) {
        if n == 0 {
            break;
        }
        let mut said = keep.text.lock().unwrap_or_else(PoisonError::into_inner);
        said.push_str(&String::from_utf8_lossy(
            buffer.get(..n).unwrap_or_default(),
        ));
        if said.len() > STDERR_TAIL {
            let mut cut = said.len() - STDERR_TAIL;
            while !said.is_char_boundary(cut) {
                cut += 1;
            }
            said.drain(..cut);
        }
    }
    keep.done.store(true, std::sync::atomic::Ordering::Release);
}

/// What records a window: its [`Offscreen`] texture, and the recording
/// when one runs.
pub struct Recorder {
    offscreen: Offscreen,
    pub recording: Option<Recording>,
}

impl Recorder {
    pub fn new(state: egui_wgpu::RenderState) -> Self {
        Self {
            offscreen: Offscreen::new(state),
            recording: None,
        }
    }

    /// One pass of `ctx` with `input`, the shell drawn by `run`, painted
    /// offscreen; read back when `read`.
    pub fn pass(
        &mut self,
        ctx: &egui::Context,
        input: egui::RawInput,
        read: bool,
        run: impl FnMut(&mut egui::Ui),
    ) -> Result<Option<Frame>, String> {
        let output = ctx.run_ui(input.clone(), run);
        let pixels_per_point = output.pixels_per_point;
        let size = input_pixels(&input, pixels_per_point)
            .ok_or("the window has no size to paint a frame of")?;
        self.offscreen.paint(ctx, output, size, read)
    }

    /// Starts a recording of the window whose input is `input` (and whose
    /// viewport lies at `viewport`, in points, for `includeUi: false`),
    /// unless one runs.
    pub fn start(
        &mut self,
        session: &Session,
        options: RecordOptions,
        input: &egui::RawInput,
        pixels_per_point: f32,
        viewport: Option<egui::Rect>,
    ) -> Result<Value, String> {
        if self.recording.is_some() {
            return Err(
                "ui.record: a recording runs already; stop it first ({\"stop\": true})".to_string(),
            );
        }
        let window = input_pixels(input, pixels_per_point)
            .ok_or("ui.record: the window has no size to record yet")?;
        let crop = if options.include_ui {
            None
        } else {
            let rect = viewport.ok_or("ui.record: the viewport has not been laid out yet")?;
            let px = |v: f32| (v * pixels_per_point).round().max(0.0) as u32;
            let (x, y) = (px(rect.min.x), px(rect.min.y));
            let (w, h) = (
                px(rect.width()).min(window[0].saturating_sub(x)),
                px(rect.height()).min(window[1].saturating_sub(y)),
            );
            Some([x, y, w, h])
        };
        let time = input.time.unwrap_or(0.0);
        let recording = Recording::start(session, options, window, crop, time)?;
        let status = recording.status();
        self.recording = Some(recording);
        Ok(status)
    }

    /// Stops the recording, and says what was written.
    pub fn stop(&mut self) -> Result<Value, String> {
        self.recording
            .take()
            .ok_or_else(|| "ui.record: nothing is being recorded".to_string())?
            .stop()
    }

    /// How the recording stands.
    pub fn status(&self) -> Value {
        self.recording
            .as_ref()
            .map_or_else(|| json!({"recording": false}), Recording::status)
    }

    /// Whether a frame-exact recording runs.
    pub fn stepping(&self) -> bool {
        self.recording.as_ref().is_some_and(|r| r.options.step)
    }

    /// Hands `frame` to the recording, when one runs.
    pub fn push(&mut self, frame: &Frame) -> Result<(), String> {
        match self.recording.as_mut() {
            Some(recording) => recording.push(frame),
            None => Ok(()),
        }
    }
}
