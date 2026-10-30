//! Which renderer draws the pictures of `view.render` and `view.replay`.
//!
//! The CPU renderer (`craft3d_render`) is the oracle and is always there.
//! A host may give the session another one ([`Session::set_picture_renderer`]):
//! `craft3d-cli` and its `serve` give it the wgpu renderer on a device of
//! its own, without a window (docs/design/rendering.md › *Headless GPU*).
//! The engine does not depend on the GPU crate: it only knows this trait,
//! as the window's viewport is drawn by the shell and not by the engine.
//!
//! A command's `renderer` param chooses: `auto` (the default) draws with
//! the host's renderer when there is one and falls back to the CPU when
//! that fails (no adapter, a lost device), `gpu` insists on it, `cpu`
//! draws with the oracle. The result says which drew the picture.

use crate::commands::fail;
use crate::{CommandError, Session};
use craft3d_render::studio::{Look, Prepared};
use craft3d_render::{Camera, FaceColors, Frame, RenderParams};
use craft3d_tess::Mesh;
use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, PoisonError};

/// A host's renderer or encoder in the session. A clone of the session
/// has none (as it has no recovery): the device stays with the session
/// the host gave it to. Shared with the [`Painter`] that has it out for a
/// command, which puts it back when it is dropped, also when the command
/// panicked and the host's last-resort guard caught it.
pub(crate) struct Hosted<T: ?Sized>(Arc<Mutex<Option<Box<T>>>>);

type Slot<T> = Arc<Mutex<Option<Box<T>>>>;

impl<T: ?Sized> Hosted<T> {
    /// Takes it out (for one command; [`Hosted::put`] gives it back).
    pub(crate) fn take(&mut self) -> Option<Box<T>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    pub(crate) fn put(&mut self, value: Box<T>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
    }

    /// Where it goes back to.
    fn slot(&self) -> Slot<T> {
        Arc::clone(&self.0)
    }
}

impl<T: ?Sized> Default for Hosted<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }
}

impl<T: ?Sized> Clone for Hosted<T> {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// How a picture is shaded, as the CPU renderer's three entry points.
#[derive(Clone, Copy)]
pub enum Shading<'a> {
    /// `craft3d_render::render_colored`.
    Plain,
    /// `craft3d_render::render_analysed`, with a value per vertex.
    Analysed(&'a [f32]),
    /// `craft3d_render::studio::render_studio`.
    Studio(&'a Look, &'a Arc<Prepared>),
}

/// A renderer a host gives the session besides the CPU's: the same
/// pictures as the CPU renderer's three entry points, within the parity
/// the GPU renderer states (docs/design/rendering.md › *Tolerance*).
pub trait PictureRenderer: Send {
    /// One line naming what draws (the adapter), once it is ready; why it
    /// cannot draw otherwise. Making the device may happen here, the first
    /// time.
    fn describe(&mut self) -> Result<String, String>;

    /// The picture of `mesh` with `colors` from `camera`, shaded as
    /// `shading` says. `key`, when given, names the mesh and its colours:
    /// a renderer may keep what it uploaded for the next call with the same
    /// key (the caller gives the same key only for the same mesh and
    /// colours).
    fn render(
        &mut self,
        mesh: &Mesh,
        colors: &FaceColors,
        camera: &Camera,
        params: &RenderParams,
        shading: Shading<'_>,
        key: Option<u64>,
    ) -> Result<Frame, String>;
}

/// The `renderer` param.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Auto,
    Cpu,
    Gpu,
}

impl Choice {
    /// The param `renderer` of `command`: `auto` when absent.
    pub fn from_param(command: &str, value: Option<&Value>) -> Result<Choice, CommandError> {
        match value {
            None | Some(Value::Null) => Ok(Choice::Auto),
            Some(Value::String(s)) => match s.as_str() {
                "auto" => Ok(Choice::Auto),
                "cpu" => Ok(Choice::Cpu),
                "gpu" => Ok(Choice::Gpu),
                other => {
                    let other: String = other.chars().take(40).collect();
                    fail(format!(
                        "{command}: unknown renderer `{other}`; use auto, cpu or gpu"
                    ))
                }
            },
            Some(other) => fail(format!(
                "{command}: `renderer` must be \"auto\", \"cpu\" or \"gpu\", got {}",
                other.to_string().chars().take(40).collect::<String>()
            )),
        }
    }
}

/// What drew a command's pictures, for its result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Used {
    /// The host's renderer drew (at least the last picture).
    pub gpu: bool,
    /// Its adapter, when it drew.
    pub adapter: Option<String>,
    /// Why `auto` fell back to the CPU, when it did.
    pub fallback: Option<String>,
    /// Pictures the CPU drew and the host's renderer drew.
    pub cpu_pictures: u64,
    pub gpu_pictures: u64,
    /// The frames per studio still the CPU averaged after a fallback in
    /// the middle of the command, when that is fewer than the look's.
    pub cpu_accumulate: Option<u32>,
}

impl Used {
    /// The fields the result gets: `renderer` (`gpu`, `cpu`, or `mixed`
    /// when the GPU drew some pictures and the CPU the rest), `adapter`
    /// when the GPU drew, `rendererFallback` when `auto` fell back, and
    /// with `mixed` the count of each (`pictures`) and the CPU's frames per
    /// studio still when it averaged fewer (`cpuAccumulate`).
    pub fn insert_into(&self, result: &mut Value) {
        let Some(map) = result.as_object_mut() else {
            return;
        };
        let renderer = match (self.gpu_pictures > 0, self.cpu_pictures > 0) {
            (true, true) => "mixed",
            (true, false) => "gpu",
            _ => "cpu",
        };
        map.insert("renderer".into(), json!(renderer));
        if let Some(adapter) = &self.adapter
            && self.gpu_pictures > 0
        {
            map.insert("adapter".into(), json!(adapter));
        }
        if let Some(why) = &self.fallback {
            map.insert("rendererFallback".into(), json!(why));
        }
        if renderer == "mixed" {
            map.insert(
                "pictures".into(),
                json!({"gpu": self.gpu_pictures, "cpu": self.cpu_pictures}),
            );
        }
        if let Some(cap) = self.cpu_accumulate {
            map.insert("cpuAccumulate".into(), json!(cap));
        }
    }
}

/// Draws pictures for one command: with the host's renderer (taken out of
/// the session for the command, and put back when the painter is done or
/// dropped) or with the CPU's, as the choice says.
pub(crate) struct Painter {
    command: &'static str,
    choice: Choice,
    host: Option<Box<dyn PictureRenderer>>,
    /// Where the host's renderer goes back to.
    home: Option<Slot<dyn PictureRenderer>>,
    /// The host's renderer cannot draw (or failed) under `auto`: the CPU
    /// draws the rest.
    given_up: bool,
    /// It failed while the command was drawing: a studio still the CPU
    /// cannot average as many frames of is averaged over as many as it
    /// can, rather than the command failing half-way.
    cap_on_cpu: bool,
    pub used: Used,
}

impl Painter {
    /// A painter for `command` with the session's renderer. Under `auto`
    /// the renderer is asked at once whether it can draw (its device is
    /// made), so a command that sizes its pictures for the renderer
    /// (`view.replay`'s frames per still) knows before it starts; `gpu`
    /// fails at once when there is none or it cannot draw.
    pub fn new(
        session: &mut Session,
        command: &'static str,
        choice: Choice,
    ) -> Result<Painter, CommandError> {
        let host = match choice {
            Choice::Cpu => None,
            Choice::Auto | Choice::Gpu => session.pictures.take(),
        };
        let mut painter = Painter {
            command,
            choice,
            host,
            home: Some(session.pictures.slot()),
            given_up: false,
            cap_on_cpu: false,
            used: Used::default(),
        };
        if choice == Choice::Cpu {
            return Ok(painter);
        }
        let ready = match painter.host.as_mut() {
            None => Err(
                "this session has no GPU renderer (craft3d-cli and craft3d-cli serve have one; the window and the browser draw these pictures on the CPU)"
                    .to_string(),
            ),
            Some(host) => painter_call(|| host.describe()),
        };
        match ready {
            Ok(adapter) => painter.used.adapter = Some(adapter),
            // No renderer at all is not a fallback: the CPU is the only one.
            Err(_) if choice == Choice::Auto && painter.host.is_none() => painter.given_up = true,
            Err(why) if choice == Choice::Auto => {
                painter.given_up = true;
                painter.used.fallback = Some(why);
            }
            Err(why) => return fail(format!("{command}: renderer gpu: {why}")),
        }
        Ok(painter)
    }

    /// A painter that draws on the CPU only (thumbnails: the same bytes
    /// on every machine).
    pub fn cpu(command: &'static str) -> Painter {
        Painter {
            command,
            choice: Choice::Cpu,
            host: None,
            home: None,
            given_up: false,
            cap_on_cpu: false,
            used: Used::default(),
        }
    }

    /// Puts the host's renderer back into the session (dropping the
    /// painter does the same).
    pub fn finish(&mut self) {
        if let (Some(host), Some(home)) = (self.host.take(), self.home.as_ref()) {
            *home.lock().unwrap_or_else(PoisonError::into_inner) = Some(host);
        }
    }

    /// True when the next picture goes to the host's renderer (so the
    /// caller draws them one at a time, on this thread).
    pub fn on_host(&self) -> bool {
        self.host.is_some() && !self.given_up && self.choice != Choice::Cpu
    }

    /// One picture.
    pub fn draw(
        &mut self,
        mesh: &Mesh,
        colors: &FaceColors,
        camera: &Camera,
        params: &RenderParams,
        shading: Shading<'_>,
        key: Option<u64>,
    ) -> Result<Frame, CommandError> {
        if self.on_host()
            && let Some(host) = self.host.as_mut()
        {
            let drawn = catch_unwind(AssertUnwindSafe(|| {
                host.render(mesh, colors, camera, params, shading, key)
            }));
            let why = match drawn {
                Ok(Ok(frame)) => {
                    self.used.gpu = true;
                    self.used.gpu_pictures = self.used.gpu_pictures.saturating_add(1);
                    return Ok(frame);
                }
                Ok(Err(why)) => why,
                Err(_) => {
                    // A renderer that panicked is not trusted again: it
                    // is let go of, not put back into the session.
                    self.host = None;
                    "the GPU renderer panicked; this session draws on the CPU from now on"
                        .to_string()
                }
            };
            if self.choice != Choice::Auto {
                return fail(format!("{}: renderer gpu: {why}", self.command));
            }
            self.given_up = true;
            self.cap_on_cpu = true;
            self.used.fallback = Some(why);
        }
        // A fallback in the middle of a command: as many frames per still
        // as the CPU averages at this size.
        let capped;
        let shading = match shading {
            Shading::Studio(look, env) => {
                capped = self.cpu_look(look, params);
                Shading::Studio(&capped, env)
            }
            other => other,
        };
        let frame = cpu_draw(mesh, colors, camera, params, shading)
            .map_err(|e| CommandError::new(format!("{}: {e}", self.command)))?;
        self.used.cpu_pictures = self.used.cpu_pictures.saturating_add(1);
        Ok(frame)
    }

    /// The look a studio still is averaged with on the CPU: the look as
    /// it is, or after a fallback in the middle of the command as many
    /// frames as the CPU averages at `params`'s size.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn cpu_look(&mut self, look: &Look, params: &RenderParams) -> Look {
        let cap = cpu_frame_cap(params);
        if self.cap_on_cpu && look.accumulate > cap {
            self.used.cpu_accumulate = Some(cap);
            return Look {
                accumulate: cap,
                ..*look
            };
        }
        *look
    }

    /// Counts `n` pictures the caller drew on the CPU itself (on threads
    /// of its own, with [`cpu_draw`]).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn count_cpu(&mut self, n: u64) {
        self.used.cpu_pictures = self.used.cpu_pictures.saturating_add(n);
    }
}

impl Drop for Painter {
    fn drop(&mut self) {
        self.finish();
    }
}

/// A call into the host's renderer, a panic in it an error.
fn painter_call<T>(call: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(call))
        .unwrap_or_else(|_| Err("the GPU renderer panicked".to_string()))
}

/// A picture with the CPU renderer.
pub(crate) fn cpu_draw(
    mesh: &Mesh,
    colors: &FaceColors,
    camera: &Camera,
    params: &RenderParams,
    shading: Shading<'_>,
) -> Result<Frame, craft3d_render::RenderError> {
    match shading {
        Shading::Plain => craft3d_render::render_colored(mesh, colors, camera, params),
        Shading::Analysed(values) => {
            craft3d_render::render_analysed(mesh, colors, values, camera, params)
        }
        Shading::Studio(look, env) => {
            craft3d_render::studio::render_studio(mesh, colors, camera, params, look, env)
        }
    }
}

/// The most frames a studio still on the CPU averages at `params`'s size:
/// the CPU renderer refuses more than `MAX_SAMPLES` samples over all of
/// them (at least one frame).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn cpu_frame_cap(params: &RenderParams) -> u32 {
    let samples = u64::from(params.width)
        .saturating_mul(u64::from(params.height))
        .saturating_mul(u64::from(params.ssaa).saturating_pow(2))
        .max(1);
    let cap = craft3d_render::MAX_SAMPLES / samples;
    u32::try_from(cap).unwrap_or(u32::MAX).max(1)
}

impl Session {
    /// Gives the session a renderer besides the CPU's for `view.render`
    /// and `view.replay` (`renderer: "auto"` or `"gpu"`); the host's
    /// choice, as the window draws its own viewport. Replaces one given
    /// before.
    pub fn set_picture_renderer(&mut self, renderer: Box<dyn PictureRenderer>) {
        self.pictures.put(renderer);
    }

    /// Gives the session a way to make a video of a replay's frames
    /// (`view.replay`'s `video`): the engine starts no program itself.
    pub fn set_video_encoder(&mut self, encoder: Box<dyn VideoEncoder>) {
        self.video.put(encoder);
    }
}

/// Makes a video file of the PNG frames a replay wrote, for a host that
/// can start a program (`craft3d-cli`: ffmpeg).
pub trait VideoEncoder: Send {
    /// Whether a video can be made at all, before the frames are drawn:
    /// why not, otherwise (ffmpeg missing).
    fn ready(&mut self) -> Result<(), String>;

    /// Makes `out` (its extension says the container: mp4, webm or gif)
    /// of `count` frames `frame-%05d.png` in `frames`, at `fps`, with the
    /// subtitles `captions.srt` there when the container takes them.
    /// Returns what the result says about it (a JSON object).
    fn encode(
        &mut self,
        frames: &std::path::Path,
        count: u64,
        fps: u64,
        out: &std::path::Path,
    ) -> Result<Value, String>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_renderer_param() {
        assert_eq!(Choice::from_param("x", None).unwrap(), Choice::Auto);
        assert_eq!(
            Choice::from_param("x", Some(&json!("cpu"))).unwrap(),
            Choice::Cpu
        );
        assert_eq!(
            Choice::from_param("x", Some(&json!("gpu"))).unwrap(),
            Choice::Gpu
        );
        for bad in [json!("vulkan"), json!(1), json!([]), json!({})] {
            assert!(Choice::from_param("x", Some(&bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_cpu_frame_cap_follows_the_samples() {
        let p = RenderParams {
            width: 800,
            height: 600,
            ..RenderParams::default()
        };
        // 800 × 600 × 4 samples a frame: 34 frames within 2^26.
        assert_eq!(cpu_frame_cap(&p), 34);
        let big = RenderParams {
            width: 4096,
            height: 4096,
            ssaa: 4,
            ..RenderParams::default()
        };
        assert_eq!(cpu_frame_cap(&big), 1);
    }

    #[test]
    fn the_result_says_what_drew() {
        let mut r = json!({});
        Used::default().insert_into(&mut r);
        assert_eq!(r["renderer"], "cpu");
        let mut r = json!({});
        Used {
            gpu: true,
            adapter: Some("lavapipe".into()),
            gpu_pictures: 3,
            ..Used::default()
        }
        .insert_into(&mut r);
        assert_eq!(r["renderer"], "gpu");
        assert_eq!(r["adapter"], "lavapipe");
    }
}
