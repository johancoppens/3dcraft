//! `view.replay`: how a document was built, as a row of pictures.
//!
//! The rollback marker is stepped through the feature history, from
//! nothing to the last feature, and every step that changes a body is
//! drawn with the CPU renderer, from one camera framed on the finished
//! model, so the picture does not jump. The frames are PNG files in a
//! directory under the write root, with a subtitle file of what each step
//! added; `craft3d-cli replay` turns them into an MP4 or a GIF.
//!
//! The document is not touched: the steps are regenerated on a copy of
//! it, each from a copy of the document's own regeneration cache, so a
//! step rebuilds only what the document has not built already, and the
//! document's marker, revision, undo history and dirty state stay as they
//! were, also when the replay fails half-way.

use crate::commands::{Run, fail, spec};
use crate::params::Params;
use crate::{CommandSpec, Session};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "view.replay",
    "Replay",
    r#"{"dir": string, "width": integer = 800, "height": integer = 600, "view": "iso|front|back|left|right|top|bottom" = "iso", "projection": "perspective|orthographic" = "perspective", "zoom": number = 1, "box": [[x, y, z], [x, y, z]] = the finished model, "tolerance": number = the picture's at to, as view.render, "from": id or name = the first feature, "to": id or name = the rollback marker or the last feature, "every": boolean = false, "framesPerStep": integer = 1, "hold": integer = 0, "orbit": number = 0, "fps": integer = 30, "motion": {t, frames, collisions} = none, "animate": [{variable: name, from: number, to: number, keys: [{t: 0 to 1, value: number}], unit: string, easing: "linear|in|out|inOut" = "linear"}] = none, "frames": integer = 2 s at fps, "duration": seconds, "camera": {keys: [{t: 0 to 1, eye: [x, y, z], target: [x, y, z], up: [x, y, z], fov: degrees}], orbit: degrees = 0, dolly: factor = 1, easing: "linear|in|out|inOut" = "linear"} = the framed camera, standing still, "background": "light|dark" = "light", "look": {as view.render's; accumulate = 16} = the plain look, "renderer": "auto|cpu|gpu" = "auto", "video": string (.mp4, .webm or .gif) = none}"#,
    "Renders how the document is built, for a showcase video: the history is rolled back to nothing and forward one feature at a time, from from to to, and each step is drawn as view.render would, from one camera framed on the model as it is at to, so the picture stays put. Writes the frames as dir/frame-00000.png, frame-00001.png, ... under the write root (frames of an earlier, longer replay there are removed) and dir/captions.srt, a subtitle per step with the feature's place, name and type, timed at fps. The first framesPerStep frames show the model before from (nothing, from the first feature), then framesPerStep frames per step, then hold more of the end; without an orbit the last frame is the picture view.render gives of the document with the same view, projection, zoom, box and tolerance. Without tolerance every step is meshed at the tolerance view.render chooses for the model as it is at to, for the camera of the replay. A step that changes no body (a sketch, a datum, a failed or suppressed feature) gets no frames unless every is true. orbit turns the camera that many degrees about the vertical over the whole replay (the model is then framed so that it stays in the picture all round). The document does not change: not its rollback marker, not its undo history. At most 3600 frames, and 8e9 pixels in all. Returns the frame count, the steps (with the frames each got, or none) and the captions. craft3d-cli replay makes an MP4 or a GIF of it. With motion ({t: [from, to] = [0, 1], frames: integer = 61, collisions: boolean = false}) it replays the motion instead of the history: frames values of t evenly from t[0] to t[1] (both included), each state made as view.motion makes it (only the features that depend on t are rebuilt and meshed again), framed on the box around every state, one frame each (hold more of the last), captioned t = ...; collisions: true draws the bodies that overlap at each t red. from, to, every and framesPerStep do not go with motion, and the document needs a timeline (view.motion). The result then has motion.frames: per frame its t and how many pairs collide. The motion is drawn a few frames at a time, so what it holds does not grow with the number of frames; frames whose meshes need more than 20000000 triangles at once are refused (give a coarser tolerance). With animate it renders an animation instead (an exploded view, a stroke, a parameter sweep): frames states (or duration seconds at fps; 2 s by default), state k at s = k / (frames - 1) from 0 to 1, each the document with every track's variable set to its value at s (from to to, or through keys [{t, value}] with t from 0 to 1 rising; easing moves a value between two keys: linear, in, out, inOut, each per stretch between two keys, so in, out and inOut come to rest at every key, the ones between included; unit is written after each value, as \"mm\" or \"deg\", and must give the variable the dimension it has, a track of the timeline t takes 0 to 1 and no unit) and regenerated from the document's own regeneration, so only what the variables touch is rebuilt; a track of the document's timeline t is played as the motion is (view.motion: an assembly's parts are placed by its solve, the only track an assembly takes), an empty list animates the camera alone. Each frame is captioned with the values; the result has animate: {tracks, states, failed: the frames whose features failed}. frames and duration go with animate only; from, to, every, framesPerStep and motion do not. camera moves the camera over every replay: keys (t from 0, the first frame, to 1, the last, hold included) give eye, target, up and fov, the others the framed camera's, and the camera passes through them on a Catmull-Rom spline in time (a key's tangent the chord between its neighbours over the time between them, so uneven keys are not overshot; the up and the field of view between them); orbit (or the top-level orbit, not both) turns it about the vertical through its target, dolly moves it towards the target (0.5 ends at half the distance), easing paces all of it. look and background are view.render's: the studio look's frames average look.accumulate frames each (16 when not given, the window's settled picture; on the CPU at most what its 2^26 samples allow). renderer is view.render's: auto draws on the session's GPU renderer when it has one (craft3d-cli and craft3d-cli serve), else on the CPU on every core; auto asks it before the first frame whether it can draw and sizes the studio frames for the CPU when it cannot; when it fails part-way the CPU draws the rest (a studio frame then averages as many frames as the CPU can at the size); the result says renderer (gpu, cpu, or mixed with pictures {gpu, cpu} and cpuAccumulate), adapter and rendererFallback. video makes a video of the frames under the write root with ffmpeg (found on PATH or named by CRAFT3D_FFMPEG; an MP4 gets the captions as a subtitle track), in a session whose host has an encoder (craft3d-cli and its serve): refused before a frame is drawn when ffmpeg is missing, and the PNG sequence of dir alone is the alternative.",
    doc: true,
    view_replay
)];

/// Most frames one replay writes.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const MAX_REPLAY_FRAMES: u64 = 3600;
/// Most pixels one replay renders, over all its frames.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const MAX_REPLAY_PIXELS: u64 = 8_000_000_000;

/// Most triangles the meshes of a motion replay's chunk of frames hold at
/// once (about 2 GB of mesh).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const MAX_REPLAY_TRIANGLES: usize = 20_000_000;

#[cfg(target_arch = "wasm32")]
fn view_replay(_session: &mut Session, params: &Value) -> Run {
    Params::new("view.replay", params, &[])?;
    fail("view.replay: writing frames needs a file system, which this build does not have")
}

#[cfg(not(target_arch = "wasm32"))]
fn view_replay(session: &mut Session, params: &Value) -> Run {
    native::view_replay(session, params)
}

/// The caption of a step: the one the window's history player shows.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn caption(place: usize, count: usize, name: &str, kind: &str) -> String {
    crate::step_caption(place, count, name, kind)
}

/// The time of frame `frame` at `fps` as an SRT time stamp, `00:01:02,500`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn srt_time(frame: u64, fps: u64) -> String {
    let millis = frame.saturating_mul(1000) / fps.max(1);
    let (h, rest) = (millis / 3_600_000, millis % 3_600_000);
    let (m, rest) = (rest / 60_000, rest % 60_000);
    let (s, ms) = (rest / 1000, rest % 1000);
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

/// The name of frame `index`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn frame_name(index: u64) -> String {
    format!("frame-{index:05}.png")
}

/// The index of a file named as [`frame_name`] names one.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn frame_index(name: &str) -> Option<u64> {
    let digits = name.strip_prefix("frame-")?.strip_suffix(".png")?;
    (digits.len() == 5 && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use crate::CommandError;
    use crate::animation::{CameraPath, Track};
    use crate::commands::{Framing, feature_ref, vec3};
    use crate::pictures::{Choice, Painter, Shading};
    use craft3d_doc::FeatureId;
    use craft3d_math::Portable;
    use craft3d_math::{Bbox3, Vec3};
    use craft3d_regen::Status;
    use craft3d_render::{Camera, Projection, RenderParams};
    use craft3d_tess::{Mesh, TessParams, tessellate_within_budget};
    use craft3d_topo::Body;
    use serde_json::json;
    use std::sync::Arc;

    const ID: &str = "view.replay";

    /// The bodies of one state of the history, in the order of the
    /// regeneration (the order `view.render` draws them in).
    /// The bodies of one state, each with the colour of each of its faces
    /// (`appearance`), which a body keeps from one state to the next.
    type Bodies = Vec<(FeatureId, Arc<Body>, Arc<Vec<Option<Look>>>)>;
    /// A face's colour and finish.
    type Look = crate::appearance::FaceLook;

    fn same(a: &Bodies, b: &Bodies) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|((ia, ba, _), (ib, bb, _))| ia == ib && Arc::ptr_eq(ba, bb))
    }

    /// One feature of the range.
    struct Step {
        id: FeatureId,
        name: String,
        kind: &'static str,
        place: usize,
        status: &'static str,
        bodies: Bodies,
        /// Drawn: it changed a body, or `every` asked for it.
        shown: bool,
    }

    /// Tessellations by body, so a body that stays from one step to the
    /// next is meshed once. The `Arc` is held, so its address is not
    /// reused while it is a key.
    #[derive(Default)]
    struct Meshes {
        held: std::collections::BTreeMap<usize, (Arc<Body>, Arc<Mesh>)>,
    }

    fn address(body: &Arc<Body>) -> usize {
        Arc::as_ptr(body) as usize
    }

    impl Meshes {
        /// Tessellates every body of `bodies` not held yet.
        fn fill(
            &mut self,
            bodies: &Bodies,
            names: &dyn Fn(FeatureId) -> String,
            tess: &TessParams,
        ) -> Result<(), CommandError> {
            for (id, body, _) in bodies {
                if self.get(body).is_none() {
                    // As the picture's scene meshes a body: coarser where
                    // the tolerance is too fine for the tessellator, and
                    // without the angle where that cannot be met.
                    let mesh = |tess: &TessParams| {
                        tessellate_within_budget(body, tess).map(|within| within.mesh)
                    };
                    let mesh = match mesh(tess) {
                        Err(e)
                            if tess.max_angle.is_some()
                                && !matches!(e, craft3d_tess::TessError::OverBudget { .. }) =>
                        {
                            mesh(&TessParams {
                                max_angle: None,
                                ..*tess
                            })
                        }
                        other => other,
                    }
                    .map_err(|e| CommandError::new(format!("view.replay: {}: {e}", names(*id))))?;
                    self.held
                        .insert(address(body), (Arc::clone(body), Arc::new(mesh)));
                }
            }
            Ok(())
        }

        fn get(&self, body: &Arc<Body>) -> Option<&Mesh> {
            self.held
                .get(&address(body))
                .filter(|(b, _)| Arc::ptr_eq(b, body))
                .map(|(_, mesh)| mesh.as_ref())
        }

        /// Lets go of the meshes of bodies none of `states` has.
        fn keep_only<'a>(&mut self, states: impl Iterator<Item = &'a Bodies>) {
            let wanted: std::collections::BTreeSet<usize> = states
                .flat_map(|bodies| bodies.iter().map(|(_, body, _)| address(body)))
                .collect();
            self.held.retain(|key, _| wanted.contains(key));
        }

        /// The triangles of the meshes held.
        fn triangles(&self) -> usize {
            self.held
                .values()
                .map(|(_, mesh)| mesh.triangles.len())
                .fold(0, usize::saturating_add)
        }

        /// One mesh of all `bodies` (filled before), face ids counting on
        /// across bodies, exactly as `view.render` joins them.
        fn joined(
            &self,
            bodies: &Bodies,
        ) -> Result<(Mesh, craft3d_render::FaceColors), CommandError> {
            let mut all = Mesh::default();
            let mut colors = craft3d_render::FaceColors::new();
            for (_, body, faces) in bodies {
                let mesh = self
                    .get(body)
                    .ok_or_else(|| CommandError::new("view.replay: a body was not meshed"))?;
                let vertex_base = u32::try_from(all.positions.len())
                    .map_err(|_| CommandError::new("view.replay: mesh too large"))?;
                let face_base = all.face_ids.iter().copied().max().map_or(0, |m| m + 1);
                for (face, look) in faces.iter().enumerate() {
                    if let (Some((color, finish, optics)), Ok(face)) = (look, u32::try_from(face)) {
                        let id = face_base.saturating_add(face);
                        colors.set_look(id, Some((*color, *finish)));
                        colors.set_optics(id, *optics);
                    }
                }
                all.positions.extend(&mesh.positions);
                all.normals.extend(&mesh.normals);
                all.triangles
                    .extend(mesh.triangles.iter().map(|t| t.map(|k| k + vertex_base)));
                all.face_ids
                    .extend(mesh.face_ids.iter().map(|f| f + face_base));
            }
            Ok((all, colors))
        }
    }

    /// Runs `job(0)` .. `job(count - 1)` on up to `threads` threads; the
    /// error of the lowest job that failed, so the answer does not depend
    /// on which thread got there first.
    fn in_parallel(
        count: usize,
        threads: usize,
        job: impl Fn(usize) -> Result<(), CommandError> + Sync,
    ) -> Result<(), CommandError> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let next = AtomicUsize::new(0);
        let work = || -> Vec<(usize, CommandError)> {
            let mut failed = Vec::new();
            loop {
                let n = next.fetch_add(1, Ordering::Relaxed);
                if n >= count {
                    return failed;
                }
                if let Err(e) = job(n) {
                    failed.push((n, e));
                }
            }
        };
        let mut failed = Vec::new();
        let mut stopped = false;
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for _ in 1..threads.min(count) {
                // A thread that cannot be started is not an error: the
                // others (at least this one) do its share.
                if let Ok(worker) = std::thread::Builder::new().spawn_scoped(scope, work) {
                    workers.push(worker);
                }
            }
            failed.extend(work());
            for worker in workers {
                match worker.join() {
                    Ok(more) => failed.extend(more),
                    Err(_) => stopped = true,
                }
            }
        });
        if stopped {
            return fail("view.replay: a render thread stopped unexpectedly");
        }
        match failed.into_iter().min_by_key(|(n, _)| *n) {
            Some((_, e)) => Err(e),
            None => Ok(()),
        }
    }

    /// How many frames to render at once: the cores, as far as the
    /// renderer's buffers for frames of this size fit in about 1.5 GB.
    fn threads_for(width: u64, height: u64, ssaa: u32) -> usize {
        let cores = craft3d_math::available_threads();
        let samples = width
            .saturating_mul(height)
            .saturating_mul(u64::from(ssaa).pow(2))
            .max(1);
        let fit = 1_500_000_000_u64 / samples.saturating_mul(16);
        cores.min(16).min(usize::try_from(fit).unwrap_or(16)).max(1)
    }

    /// The camera of an orbit: framed so that the box `framed` stays in
    /// the picture, with the same 5 % margin as `view.render`, from every
    /// angle the camera turns through (sampled every 2 degrees; between
    /// samples the margin covers the difference). The angles are of the
    /// camera about the vertical through the box's centre; seen from the
    /// camera, the box turns the other way.
    fn orbit_camera(framing: &Framing, framed: &Bbox3, orbit: f64, aspect: f64) -> Option<Camera> {
        let center = framed.center()?;
        let (lo, hi) = framed.corners()?;
        let (toward_eye, up) = framing.direction();
        let dir = toward_eye.normalized()?;
        let right = up.cross(dir).normalized()?;
        let view_up = dir.cross(right);
        let sweep = orbit.abs().min(360.0);
        // Bounded: at most 180 samples.
        let samples = (sweep / 2.0).ceil().clamp(1.0, 180.0) as u32;
        let mut points = Vec::new();
        for k in 0..=samples {
            let angle = -orbit.signum() * sweep * f64::from(k) / f64::from(samples);
            let (s, c) = angle.to_radians().psin_cos();
            for x in [lo.x, hi.x] {
                for y in [lo.y, hi.y] {
                    for z in [lo.z, hi.z] {
                        let r = Vec3::new(x, y, z) - center;
                        points.push(Vec3::new(c * r.x - s * r.y, s * r.x + c * r.y, r.z));
                    }
                }
            }
        }
        let projection;
        let distance;
        if framing.orthographic {
            let (mut half_height, mut reach) = (0.0_f64, 0.0_f64);
            for r in &points {
                half_height = half_height
                    .max(r.dot(view_up).abs())
                    .max(r.dot(right).abs() / aspect);
                reach = reach.max(r.dot(dir));
            }
            let half_height = half_height * 1.05;
            distance = reach * 1.05 + half_height;
            projection = Projection::Orthographic { half_height };
            if !(half_height.is_finite() && half_height > 0.0) {
                return None;
            }
        } else {
            let fov_y_deg = 30.0_f64;
            let half_y = (fov_y_deg.to_radians() * 0.5).ptan();
            let half_x = half_y * aspect;
            let mut far = 0.0_f64;
            for r in &points {
                let along = r.dot(dir);
                far = far
                    .max(along + r.dot(right).abs() / half_x)
                    .max(along + r.dot(view_up).abs() / half_y);
            }
            distance = far * 1.05;
            projection = Projection::Perspective { fov_y_deg };
        }
        (distance.is_finite() && distance > 0.0).then_some(())?;
        Camera {
            eye: center + dir * distance,
            target: center,
            up,
            projection,
        }
        .zoomed(framing.zoom)
    }

    /// The bodies of `doc` as `regen` built them, each with its faces'
    /// colours, without those the view hides; those of `red` all in the
    /// colour of a collision ([`crate::motion::marked`]).
    fn bodies_of(
        doc: &craft3d_doc::Document,
        regen: &craft3d_regen::Regen,
        red: &[craft3d_doc::FeatureId],
    ) -> Bodies {
        let red_look = crate::motion::collision_look();
        regen
            .bodies()
            .filter(|(id, _)| !doc.is_hidden(*id))
            .map(|(id, b)| {
                let forced = red.contains(&id).then_some(&red_look);
                let colors = crate::appearance::face_looks(doc, regen, id, b, forced);
                (id, Arc::clone(b), Arc::new(colors))
            })
            .collect()
    }

    /// The parameters of `view.replay`.
    pub(super) const KEYS: &[&str] = &[
        "dir",
        "width",
        "height",
        "view",
        "projection",
        "zoom",
        "box",
        "tolerance",
        "from",
        "to",
        "every",
        "framesPerStep",
        "hold",
        "orbit",
        "fps",
        "motion",
        "background",
        "look",
        "renderer",
        "camera",
        "animate",
        "frames",
        "duration",
        "video",
    ];

    /// The keys of one kind of replay that the others do not take.
    const HISTORY_ONLY: [&str; 4] = ["from", "to", "every", "framesPerStep"];
    const ANIMATION_ONLY: [&str; 2] = ["frames", "duration"];

    /// What every kind of replay shares: where the frames go, their size,
    /// the framing, the camera's path, the look and the pacing.
    struct Options {
        dir_param: String,
        dir: std::path::PathBuf,
        width: u64,
        height: u64,
        framing: Framing,
        hold: u64,
        fps: u64,
        path: CameraPath,
        tolerance: Option<f64>,
        background: [u8; 3],
        look: crate::look::LookSpec,
    }

    impl Options {
        fn of(session: &Session, p: &Params) -> Result<Options, CommandError> {
            let dir_param = p.required_str("dir")?.to_string();
            let width = p.integer("width", 16, 4096)?.unwrap_or(800);
            let height = p.integer("height", 16, 4096)?.unwrap_or(600);
            let framing = Framing::from_params(p, ID)?;
            let hold = p.integer("hold", 0, MAX_REPLAY_FRAMES)?.unwrap_or(0);
            let fps = p.integer("fps", 1, 240)?.unwrap_or(30);
            let path = CameraPath::parse(ID, p.get("camera"), p.number("orbit")?)?;
            if framing.orthographic && path.has_fov() {
                return fail(format!(
                    "{ID}: a camera key's `fov` is a perspective camera's; this replay is orthographic"
                ));
            }
            let background = match p.str("background")?.unwrap_or("light") {
                "light" => crate::commands::VIEWPORT_BACKGROUND,
                "dark" => crate::commands::DARK_BACKGROUND,
                other => {
                    let other: String = other.chars().take(40).collect();
                    return fail(format!(
                        "{ID}: unknown background `{other}`; use light or dark"
                    ));
                }
            };
            let look = crate::look::look_param(ID, p.get("look"))?;
            if session.writes_to_memory() {
                return fail(format!(
                    "{ID}: the frames go to a folder on disk, and this session keeps its files in memory"
                ));
            }
            let dir = session.workspace.resolve_write(&dir_param)?;
            let tolerance = match p.number("tolerance")? {
                Some(t) if t > 0.0 => Some(t),
                Some(_) => return fail(format!("{ID}: `tolerance` must be positive")),
                None => None,
            };
            Ok(Options {
                dir_param,
                dir,
                width,
                height,
                framing,
                hold,
                fps,
                path,
                tolerance,
                background,
                look,
            })
        }

        fn aspect(&self) -> f64 {
            self.width as f64 / self.height as f64
        }

        /// The limits on frames and pixels.
        fn check_count(&self, frames: u64, what: &str) -> Result<(), CommandError> {
            if frames > MAX_REPLAY_FRAMES {
                return fail(format!(
                    "{ID}: {frames} frames ({what}) exceed the limit of {MAX_REPLAY_FRAMES}"
                ));
            }
            let pixels = frames
                .saturating_mul(self.width)
                .saturating_mul(self.height);
            if pixels > MAX_REPLAY_PIXELS {
                return fail(format!(
                    "{ID}: {frames} frames of {}x{} are {pixels} pixels, more than the limit of {MAX_REPLAY_PIXELS}; make them smaller or fewer",
                    self.width, self.height
                ));
            }
            Ok(())
        }

        /// The camera of the replay before its path moves it: framed on
        /// `model` as `view.render` frames it, or all round for an orbit.
        fn base_camera(&self, model: &Bbox3) -> Result<Camera, CommandError> {
            if self.path.orbit == 0.0 {
                self.framing.camera(model, self.aspect())
            } else {
                orbit_camera(
                    &self.framing,
                    &self.framing.look_at.unwrap_or(*model),
                    self.path.orbit,
                    self.aspect(),
                )
                .ok_or_else(|| CommandError::new(format!("{ID}: cannot frame the model")))
            }
        }

        fn render_params(&self) -> RenderParams {
            // Both are within 16..=4096.
            RenderParams {
                width: self.width as u32,
                height: self.height as u32,
                background: self.background,
                ..RenderParams::default()
            }
        }

        /// The studio look resolved for the model in `model` of a document
        /// in `mm_per_unit`, with the frames each picture averages: the
        /// look's `accumulate`, else [`crate::look::VIEWPORT_ACCUMULATE`]
        /// (the window's, so a replay's frames are as settled as its
        /// screenshots), on the CPU at most what its sample limit allows.
        /// `None` for the plain look.
        fn studio(
            &self,
            model: &Bbox3,
            mm_per_unit: f64,
            on_host: bool,
        ) -> Result<Option<(StudioLook, Arc<Prepared>)>, CommandError> {
            let Some(env) = self
                .look
                .prepared()
                .map_err(|e| CommandError::new(format!("{ID}: look: {e}")))?
            else {
                return Ok(None);
            };
            let params = self.render_params();
            let cap = crate::pictures::cpu_frame_cap(&params);
            let frames = match self.look.accumulate {
                Some(n) if !on_host && n > cap => {
                    return fail(format!(
                        "{ID}: look.accumulate {n} is more frames than the CPU renderer averages at {}x{} (at most {cap}); give fewer, or draw on the GPU",
                        self.width, self.height
                    ));
                }
                Some(n) => n,
                None if on_host => crate::look::VIEWPORT_ACCUMULATE,
                None => crate::look::VIEWPORT_ACCUMULATE.min(cap),
            };
            Ok(Some((
                self.look
                    .resolved(model, mm_per_unit, &crate::look::FocusAt::default(), frames),
                env,
            )))
        }

        /// The tessellation of every state: at the tolerance given, or at
        /// the picture's tolerance of the state `(doc, regen)` for the
        /// replay's camera, as `view.render` meshes it.
        fn tess(
            &self,
            doc: &craft3d_doc::Document,
            regen: &craft3d_regen::Regen,
            meshes: &crate::meshes::MeshStore,
            camera: Option<&Camera>,
        ) -> Result<TessParams, CommandError> {
            if let Some(chordal_tolerance) = self.tolerance {
                return Ok(TessParams {
                    chordal_tolerance,
                    ..TessParams::default()
                });
            }
            let rung = crate::scene::picture_tolerance(regen, doc, |model| {
                let camera = match camera {
                    Some(camera) => *camera,
                    None => self.base_camera(model).ok()?,
                };
                // Both are within 16..=4096.
                crate::scene::pixel_footprint(&camera, self.height as u32, model)
            });
            let settled = crate::scene::picture_scene_of((doc, regen, &[]), 0, rung, meshes);
            if let Some(problem) = settled.problems.first() {
                return fail(format!("{ID}: {problem}"));
            }
            Ok(TessParams {
                chordal_tolerance: settled.tolerance,
                max_angle: settled.max_angle,
                max_triangles: Some(crate::scene::DISPLAY_MAX_TRIANGLES),
                ..TessParams::default()
            })
        }

        fn create_dir(&self) -> Result<(), CommandError> {
            std::fs::create_dir_all(&self.dir).map_err(|e| {
                CommandError::new(format!(
                    "{ID}: cannot create the directory `{}`: {e}",
                    self.dir_param
                ))
            })
        }

        /// Writes the captions and removes frames an earlier, longer
        /// replay left in the directory (they would run on after this
        /// one's in a video made of it).
        fn finish(&self, frames: u64, srt: &str) -> Result<(), CommandError> {
            crate::workspace::write_atomic(&self.dir.join("captions.srt"), srt.as_bytes())?;
            if let Ok(entries) = std::fs::read_dir(&self.dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    if name
                        .to_str()
                        .and_then(frame_index)
                        .is_some_and(|index| index >= frames)
                    {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
            Ok(())
        }

        /// What every result says.
        fn result(&self, frames: u64, model: &Bbox3, studio: Option<&StudioLook>) -> Value {
            let mut result = json!({
                "dir": self.dir_param,
                "frames": frames,
                "framePattern": "frame-%05d.png",
                "fps": self.fps,
                "width": self.width,
                "height": self.height,
                "view": self.framing.view,
                "orbit": self.path.orbit,
                "box": model.corners().map(|(lo, hi)| json!([vec3(lo), vec3(hi)])),
                "captions": "captions.srt",
            });
            if let (Some(look), Some(map)) = (studio, result.as_object_mut()) {
                let mut said = self.look.json();
                if let Some(said) = said.as_object_mut() {
                    said.insert("accumulate".into(), look.accumulate.into());
                }
                map.insert("look".into(), said);
                // A replay has no selection and no click per frame: a
                // lens on either focuses on the model's centre, and says so.
                if let Some(why) = self
                    .look
                    .dof
                    .and_then(|d| d.focus.fallback(&crate::look::FocusAt::default()))
                {
                    map.insert("lens".into(), json!({ "fallback": why }));
                }
            }
            result
        }
    }

    /// The studio look as the renderers take it.
    type StudioLook = craft3d_render::studio::Look;
    type Prepared = craft3d_render::studio::Prepared;

    /// A picture to draw: the bodies of a state, the first frame it is
    /// and how many frames show it (more than one only while the camera
    /// stands still), and the key its mesh goes by on the GPU.
    struct Job<'a> {
        bodies: &'a Bodies,
        first: u64,
        n: u64,
        key: u64,
    }

    /// How the frames are drawn.
    struct Canvas<'a> {
        params: RenderParams,
        studio: Option<(StudioLook, Arc<Prepared>)>,
        base: Camera,
        path: &'a CameraPath,
        /// All frames, the hold included: the camera's path runs over them.
        frames: u64,
        dir: &'a std::path::Path,
    }

    impl Canvas<'_> {
        fn camera(&self, frame: u64) -> Result<Camera, CommandError> {
            let s = frame as f64 / self.frames.saturating_sub(1).max(1) as f64;
            self.path.at(&self.base, s).ok_or_else(|| {
                CommandError::new(format!(
                    "{ID}: the camera has no direction at frame {frame} (its eye on its target, or looking along its up)"
                ))
            })
        }

        fn shading(&self) -> Shading<'_> {
            match &self.studio {
                Some((look, env)) => Shading::Studio(look, env),
                None => Shading::Plain,
            }
        }

        fn threads(&self) -> usize {
            // The studio look holds more per sample: its geometry, its
            // light and its occlusion.
            let per_sample = if self.studio.is_some() { 4 } else { 1 };
            threads_for(
                self.params.width.into(),
                u64::from(self.params.height).saturating_mul(per_sample),
                self.params.ssaa,
            )
        }

        /// Draws `jobs` and writes their frames: on the host's renderer one
        /// at a time (the PNGs made a batch at a time on every core), on the
        /// CPU on every core.
        fn paint(
            &self,
            painter: &mut Painter,
            meshes: &Meshes,
            jobs: &[Job<'_>],
        ) -> Result<(), CommandError> {
            let threads = self.threads();
            let write = |png: &[u8], first: u64, n: u64| -> Result<(), CommandError> {
                for f in first..first.saturating_add(n) {
                    crate::workspace::write_atomic(&self.dir.join(frame_name(f)), png)?;
                }
                Ok(())
            };
            if painter.on_host() {
                for batch in jobs.chunks(threads.max(1)) {
                    let mut drawn = Vec::with_capacity(batch.len());
                    for job in batch {
                        let (mesh, colors) = meshes.joined(job.bodies)?;
                        let camera = self.camera(job.first)?;
                        let frame = painter.draw(
                            &mesh,
                            &colors,
                            &camera,
                            &self.params,
                            self.shading(),
                            Some(job.key),
                        )?;
                        drawn.push((frame, job.first, job.n));
                    }
                    in_parallel(drawn.len(), threads, |k| {
                        let Some((frame, first, n)) = drawn.get(k) else {
                            return Ok(());
                        };
                        let png = frame
                            .to_png()
                            .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
                        write(&png, *first, *n)
                    })?;
                }
                return Ok(());
            }
            // After a fallback in an earlier batch, the frames per still
            // the CPU can average.
            let look = self
                .studio
                .as_ref()
                .map(|(look, env)| (painter.cpu_look(look, &self.params), env));
            let shading = match &look {
                Some((look, env)) => Shading::Studio(look, env),
                None => Shading::Plain,
            };
            in_parallel(jobs.len(), threads, |k| {
                let Some(job) = jobs.get(k) else {
                    return Ok(());
                };
                let (mesh, colors) = meshes.joined(job.bodies)?;
                let camera = self.camera(job.first)?;
                let png = crate::pictures::cpu_draw(&mesh, &colors, &camera, &self.params, shading)
                    .and_then(|f| f.to_png())
                    .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
                write(&png, job.first, job.n)
            })?;
            painter.count_cpu(jobs.len() as u64);
            Ok(())
        }

        /// The jobs of states shown from frame `first` for `n` frames: one
        /// picture written `n` times while the camera stands still, else a
        /// picture per frame.
        fn jobs_of<'b>(&self, units: &[(&'b Bodies, u64, u64, u64)]) -> Vec<Job<'b>> {
            let mut jobs = Vec::new();
            for &(bodies, first, n, key) in units {
                if self.path.is_still() {
                    jobs.push(Job {
                        bodies,
                        first,
                        n,
                        key,
                    });
                } else {
                    for f in first..first.saturating_add(n) {
                        jobs.push(Job {
                            bodies,
                            first: f,
                            n: 1,
                            key,
                        });
                    }
                }
            }
            jobs
        }
    }

    /// The length unit of `doc` in millimetres (for the lens).
    fn mm_per_unit(doc: &craft3d_doc::Document) -> f64 {
        doc.units.length.as_expr_unit().factor()
    }

    /// A key for a state's mesh on the GPU, unique in the process.
    fn fresh_key() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    /// The video asked for: the path as given and where it goes.
    pub(super) fn video_param(
        session: &Session,
        p: &Params,
    ) -> Result<Option<(String, std::path::PathBuf)>, CommandError> {
        let Some(video) = p.str("video")? else {
            return Ok(None);
        };
        let extension = std::path::Path::new(video)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(extension.as_deref(), Some("mp4" | "webm" | "gif")) {
            return fail(format!(
                "{ID}: `video` must be a file name ending in .mp4, .webm or .gif; for a PNG sequence give dir alone"
            ));
        }
        let resolved = session.workspace.resolve_write(video)?;
        Ok(Some((video.to_string(), resolved)))
    }

    /// `view.replay`: the history, the motion, or an animation, drawn by
    /// the renderer the session and `renderer` choose, and made into a
    /// video when `video` asks.
    pub(super) fn view_replay(session: &mut Session, params: &Value) -> Run {
        let p = Params::new(ID, params, KEYS)?;
        let choice = Choice::from_param(ID, p.get("renderer"))?;
        let video = video_param(session, &p)?;
        // A video needs the host's encoder, ready before a frame is drawn.
        let mut encoder = None;
        if let Some((given, _)) = &video {
            let Some(mut found) = session.video.take() else {
                return fail(format!(
                    "{ID}: this session cannot start ffmpeg to make `{given}` (craft3d-cli and craft3d-cli serve can); give dir alone for the PNG sequence, frame-00000.png and on, and make the video of it yourself"
                ));
            };
            let ready = found.ready();
            if let Err(why) = ready {
                session.video.put(found);
                return fail(format!(
                    "{ID}: cannot make `{given}`: {why}; give dir alone for the PNG sequence (frame-00000.png and on, with captions.srt), or install ffmpeg"
                ));
            }
            encoder = Some(found);
        }
        let painter = Painter::new(session, ID, choice);
        let made = painter.and_then(|mut painter| {
            let made = if p.has("animate") {
                animate_replay(session, &p, &mut painter)
            } else if p.has("motion") {
                motion_replay(session, &p, &mut painter)
            } else {
                history_replay(session, &p, &mut painter)
            };
            painter.finish();
            made.map(|mut result| {
                painter.used.insert_into(&mut result);
                result
            })
        });
        let made = match (made, &video, encoder.as_mut()) {
            (Ok(mut result), Some((given, out)), Some(encoder)) => {
                let dir = session
                    .workspace
                    .resolve_write(result.get("dir").and_then(Value::as_str).unwrap_or("."));
                let count = result.get("frames").and_then(Value::as_u64).unwrap_or(0);
                let fps = result.get("fps").and_then(Value::as_u64).unwrap_or(30);
                dir.and_then(|dir| {
                    let said = encoder.encode(&dir, count, fps, out).map_err(|why| {
                        CommandError::new(format!(
                            "{ID}: ffmpeg could not make `{given}`: {why}; the frames are in `{}`",
                            result.get("dir").and_then(Value::as_str).unwrap_or(".")
                        ))
                    })?;
                    if let Some(map) = result.as_object_mut() {
                        map.insert("video".into(), json!(given));
                        if let Some(said) = said.as_object() {
                            for (key, value) in said {
                                map.insert(key.clone(), value.clone());
                            }
                        }
                    }
                    Ok(result)
                })
            }
            (made, ..) => made,
        };
        if let Some(found) = encoder {
            session.video.put(found);
        }
        made
    }

    /// `view.replay` over the motion (`motion`): a frame per value of `t`,
    /// each state made as `view.motion` makes it.
    fn motion_replay(session: &mut Session, p: &Params, painter: &mut Painter) -> Run {
        if let Some(key) = HISTORY_ONLY
            .into_iter()
            .chain(ANIMATION_ONLY)
            .find(|key| p.has(key))
        {
            return fail(format!(
                "{ID}: `{key}` does not go with `motion`: the frames are the moments of the motion"
            ));
        }
        let motion_value = p.get("motion").cloned().unwrap_or(Value::Null);
        if !motion_value.is_object() {
            return fail(format!(
                "{ID}: `motion` is {{\"t\": [from, to], \"frames\": integer, \"collisions\": boolean}}"
            ));
        }
        let m = Params::new(ID, &motion_value, &["t", "frames", "collisions"])?;
        let range = crate::motion::t_range(ID, &m)?;
        let count = m.integer("frames", 2, MAX_REPLAY_FRAMES)?.unwrap_or(61);
        let red = m.bool("collisions")?.unwrap_or(false);
        let o = Options::of(session, p)?;
        let frames = count.saturating_add(o.hold);
        o.check_count(frames, &format!("frames {count} plus hold {}", o.hold))?;
        let store = session.scene.meshes.clone();
        let open = session.document_mut()?;
        let base = Arc::clone(open.history.current());
        crate::motion::timeline(&base).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
        open.regen();
        // Each state is a function of its t alone (`motion::Sweep`), so
        // the states are made twice rather than kept: once for the box the
        // camera is framed on, once to draw them a chunk at a time. What is
        // held at once is one chunk's bodies and meshes, whatever the
        // number of frames.
        let mut sweep = crate::motion::Sweep::of(open, &base);
        let ts = crate::motion::samples_of(range, count);
        let mut model = Bbox3::EMPTY;
        let mut last = None;
        for &t in &ts {
            let (doc, regen) = sweep
                .at(t)
                .map_err(|e| CommandError::new(format!("{ID}: at t = {t}: {e}")))?;
            let framed = crate::scene::bodies_box(&doc, &regen);
            if let Some((lo, hi)) = framed.corners() {
                model.add(lo);
                model.add(hi);
            }
            last = Some((doc, regen));
        }
        let Some((last_doc, last_regen)) = last else {
            return fail(format!("{ID}: nothing to replay"));
        };
        if model.is_empty() {
            return fail(format!(
                "{ID}: there are no bodies over the motion; check document.inspect for failed features"
            ));
        }
        let framed = o.framing.look_at.unwrap_or(model);
        let camera = o.base_camera(&framed)?;
        let tess = o.tess(&last_doc, &last_regen, &store, Some(&camera))?;
        drop((last_doc, last_regen));
        let names = |id: FeatureId| {
            base.feature(id)
                .map_or_else(|| id.to_string(), |f| f.name.clone())
        };
        let studio = o.studio(&model, mm_per_unit(&base), painter.on_host())?;
        o.create_dir()?;
        let canvas = Canvas {
            params: o.render_params(),
            studio,
            base: camera,
            path: &o.path,
            frames,
            dir: &o.dir,
        };
        let chunk_size = canvas.threads().saturating_mul(2).max(1);
        let mut memo = crate::motion::PairMemo::default();
        let mut meshes = Meshes::default();
        let mut srt = String::new();
        let mut frames_json = Vec::new();
        let mut frame = 0_u64;
        let last_index = ts.len().saturating_sub(1);
        for (chunk_index, chunk) in ts.chunks(chunk_size).enumerate() {
            let first_k = chunk_index.saturating_mul(chunk_size);
            let mut states: Vec<(Bodies, u64, u64, u64)> = Vec::new();
            for (offset, &t) in chunk.iter().enumerate() {
                let k = first_k.saturating_add(offset);
                let (doc, regen) = sweep
                    .at(t)
                    .map_err(|e| CommandError::new(format!("{ID}: at t = {t}: {e}")))?;
                let (marked, collided) = if red {
                    let found = crate::motion::collisions(&doc, &regen, &mut memo)
                        .map_err(|e| CommandError::new(format!("{ID}: at t = {t}: {e}")))?;
                    let overlapping = found.iter().filter(|c| c.volume.is_some()).count();
                    (crate::motion::colliding(&found), overlapping)
                } else {
                    (Vec::new(), 0)
                };
                let n = if k == last_index { 1 + o.hold } else { 1 };
                let text = format!("t = {}", crate::assert_cmds::shown(t));
                srt.push_str(&format!(
                    "{}\n{} --> {}\n{text}\n\n",
                    k + 1,
                    srt_time(frame, o.fps),
                    srt_time(frame.saturating_add(n), o.fps)
                ));
                frames_json.push(json!({"t": t, "frame": frame, "collisions": collided}));
                states.push((bodies_of(&doc, &regen, &marked), frame, n, fresh_key()));
                frame = frame.saturating_add(n);
            }
            // The meshes of this chunk's bodies: those of the chunk before
            // that are still shown are kept, the others let go of.
            meshes.keep_only(states.iter().map(|(bodies, ..)| bodies));
            for (bodies, ..) in &states {
                meshes.fill(bodies, &names, &tess)?;
            }
            if meshes.triangles() > MAX_REPLAY_TRIANGLES {
                return fail(format!(
                    "{ID}: the frames from t = {} need {} triangles at once, more than the {MAX_REPLAY_TRIANGLES} a replay holds; give a coarser tolerance or a smaller model",
                    chunk.first().copied().unwrap_or(0.0),
                    meshes.triangles()
                ));
            }
            let units: Vec<(&Bodies, u64, u64, u64)> = states
                .iter()
                .map(|(bodies, first, n, key)| (bodies, *first, *n, *key))
                .collect();
            canvas.paint(painter, &meshes, &canvas.jobs_of(&units))?;
        }
        o.finish(frame, &srt)?;
        let mut result = o.result(frame, &model, canvas.studio.as_ref().map(|(l, _)| l));
        if let Some(map) = result.as_object_mut() {
            map.insert(
                "motion".into(),
                json!({"t": [range.0, range.1], "frames": frames_json, "collisions": red}),
            );
        }
        Ok(result)
    }

    /// One variable track and the variable's dimension as the document
    /// has it.
    struct Tracked {
        track: Track,
        dimension: craft3d_expr::Dimension,
    }

    /// The states of an animation, frame by frame: the document with its
    /// tracks' values set (each a function of its values alone), the
    /// timeline's through the motion's sweep.
    struct Animation {
        base: Arc<craft3d_doc::Document>,
        tracks: Vec<Tracked>,
        timeline: Option<Track>,
        sweep: Option<crate::motion::Sweep>,
        cache: craft3d_regen::RegenCache,
        count: u64,
    }

    impl Animation {
        /// The place `s` of state `k`.
        fn s(&self, k: u64) -> f64 {
            k as f64 / self.count.saturating_sub(1).max(1) as f64
        }

        /// The state of frame `k`, its regeneration and its caption.
        fn at(
            &mut self,
            k: u64,
        ) -> Result<
            (
                Arc<craft3d_doc::Document>,
                Arc<craft3d_regen::Regen>,
                String,
            ),
            CommandError,
        > {
            let s = self.s(k);
            let mut caption: Vec<String> = Vec::new();
            let t = self.timeline.as_ref().map(|track| track.at(s));
            if let Some(t) = t {
                caption.push(format!(
                    "{} = {}",
                    crate::motion::TIMELINE,
                    crate::assert_cmds::shown(t)
                ));
            }
            for tracked in &self.tracks {
                caption.push(format!(
                    "{} = {}",
                    tracked.track.variable,
                    tracked.track.expr_at(s)
                ));
            }
            let caption = caption.join(", ");
            let at_t = |e: String| CommandError::new(format!("{ID}: at frame {k}: {e}"));
            if let (Some(t), Some(sweep), true) = (t, self.sweep.as_mut(), self.tracks.is_empty()) {
                let (doc, regen) = sweep.at(t).map_err(at_t)?;
                return Ok((doc, regen, caption));
            }
            let mut doc = match (t, self.sweep.as_mut()) {
                (Some(t), Some(sweep)) => sweep.settled(t).map_err(at_t)?,
                _ => craft3d_doc::Document::clone(&self.base),
            };
            for tracked in &self.tracks {
                let expr = tracked.track.expr_at(s);
                doc.set_variable(&tracked.track.variable, &expr)
                    .map_err(|e| {
                        CommandError::new(format!(
                            "{ID}: at frame {k}: `{}` = `{expr}`: {e}",
                            tracked.track.variable
                        ))
                    })?;
            }
            if k == 0 && !self.tracks.is_empty() {
                let evaluated = doc
                    .variable_table()
                    .map_err(|e| CommandError::new(format!("{ID}: {e}")))?
                    .evaluate();
                for tracked in &self.tracks {
                    let name = &tracked.track.variable;
                    match evaluated.get(name) {
                        Some(Ok(q)) if q.dimension == tracked.dimension => {}
                        Some(Ok(q)) => {
                            return fail(format!(
                                "{ID}: `{name}` is {} in the document, and the track makes it {}; give the track the variable's unit (\"unit\": \"mm\", \"deg\", ...)",
                                tracked.dimension.described(),
                                q.dimension.described()
                            ));
                        }
                        Some(Err(e)) => {
                            return fail(format!(
                                "{ID}: `{name}` = `{}` does not evaluate: {e}",
                                tracked.track.expr_at(0.0)
                            ));
                        }
                        None => return fail(format!("{ID}: `{name}` does not evaluate")),
                    }
                }
            }
            let regen = craft3d_regen::regenerate_cached(&doc, &mut self.cache);
            Ok((Arc::new(doc), Arc::new(regen), caption))
        }
    }

    /// `view.replay` with `animate`: frames over which variables (and the
    /// timeline) follow their tracks and the camera its path.
    fn animate_replay(session: &mut Session, p: &Params, painter: &mut Painter) -> Run {
        if let Some(key) = HISTORY_ONLY
            .into_iter()
            .chain(["motion"])
            .find(|key| p.has(key))
        {
            return fail(format!(
                "{ID}: `{key}` does not go with `animate`: the frames are the animation's"
            ));
        }
        let tracks = Track::parse_all(ID, p.get("animate").unwrap_or(&Value::Null))?;
        let o = Options::of(session, p)?;
        let count = match (p.get("frames"), p.get("duration")) {
            (Some(_), Some(_)) => {
                return fail(format!("{ID}: give `frames` or `duration`, not both"));
            }
            (Some(_), None) => p.integer("frames", 2, MAX_REPLAY_FRAMES)?.unwrap_or(2),
            (None, Some(_)) => match p.number("duration")? {
                Some(d) if d > 0.0 && d <= MAX_REPLAY_FRAMES as f64 => {
                    // At least two frames; at most what the limit allows
                    // (checked below).
                    ((d * o.fps as f64).round() as u64).max(2)
                }
                _ => {
                    return fail(format!(
                        "{ID}: `duration` must be seconds, more than 0 and at most {MAX_REPLAY_FRAMES}"
                    ));
                }
            },
            (None, None) => o.fps.saturating_mul(2).max(2),
        };
        let frames = count.saturating_add(o.hold);
        o.check_count(frames, &format!("{count} plus hold {}", o.hold))?;
        let store = session.scene.meshes.clone();
        let open = session.document_mut()?;
        let base = Arc::clone(open.history.current());
        if base.features.is_empty() {
            return fail(format!("{ID}: the document has no features to animate"));
        }
        let has_timeline = crate::motion::timeline(&base).is_ok();
        let evaluated = base
            .variable_table()
            .map_err(|e| CommandError::new(format!("{ID}: {e}")))?
            .evaluate();
        let mut timeline = None;
        let mut tracked = Vec::new();
        for track in tracks {
            let name = track.variable.as_str();
            if name == crate::motion::TIMELINE && has_timeline {
                if track.unit.is_some() || track.values.iter().any(|v| !(0.0..=1.0).contains(v)) {
                    return fail(format!(
                        "{ID}: the timeline `{name}` runs from 0 to 1, a plain number: its track takes values from 0 to 1 and no unit"
                    ));
                }
                timeline = Some(track);
                continue;
            }
            if base.variable(name).is_none() {
                let name: String = name.chars().take(60).collect();
                return fail(format!(
                    "{ID}: the document has no variable `{name}` (variable.list names them)"
                ));
            }
            if base.is_assembly() {
                return fail(format!(
                    "{ID}: an assembly animates its timeline `{}` (its drivers are formulas of it); `{name}` cannot be animated here",
                    crate::motion::TIMELINE
                ));
            }
            let dimension = match evaluated.get(name) {
                Some(Ok(q)) => q.dimension,
                _ => {
                    return fail(format!(
                        "{ID}: `{name}` does not evaluate in the document; fix it before animating it"
                    ));
                }
            };
            tracked.push(Tracked { track, dimension });
        }
        if base.is_assembly() && timeline.is_none() {
            // An assembly's parts are placed by its solve at a moment of
            // its timeline: the document's own, when no track moves it.
            match crate::motion::timeline(&base) {
                Ok(t0) => {
                    timeline = Some(Track {
                        variable: crate::motion::TIMELINE.to_string(),
                        times: vec![0.0],
                        values: vec![t0],
                        unit: None,
                        easing: crate::animation::Easing::Linear,
                    });
                }
                Err(why) => {
                    return fail(format!(
                        "{ID}: an assembly is animated through its timeline: {why}"
                    ));
                }
            }
        }
        let sweep = if timeline.is_some() {
            Some(crate::motion::Sweep::of(open, &base))
        } else {
            None
        };
        if !base.is_assembly() {
            open.regen();
        }
        let mut animation = Animation {
            base: Arc::clone(&base),
            tracks: tracked,
            timeline,
            sweep,
            cache: open.cache.clone(),
            count,
        };
        // The box over every state, unless `box` frames the camera: the
        // states are made twice rather than kept, as for the motion.
        let mut model = Bbox3::EMPTY;
        let mut last = None;
        for k in 0..count {
            let (doc, regen, _) = animation.at(k)?;
            if (o.framing.look_at.is_none() || k + 1 == count)
                && let Some((lo, hi)) = crate::scene::bodies_box(&doc, &regen).corners()
            {
                model.add(lo);
                model.add(hi);
            }
            if k + 1 == count {
                last = Some((doc, regen));
            }
        }
        let Some((last_doc, last_regen)) = last else {
            return fail(format!("{ID}: nothing to animate"));
        };
        if model.is_empty() {
            return fail(format!(
                "{ID}: there are no bodies over the animation; check document.inspect for failed features"
            ));
        }
        let framed = o.framing.look_at.unwrap_or(model);
        let camera = o.base_camera(&framed)?;
        let tess = o.tess(&last_doc, &last_regen, &store, Some(&camera))?;
        drop((last_doc, last_regen));
        let names = |id: FeatureId| {
            base.feature(id)
                .map_or_else(|| id.to_string(), |f| f.name.clone())
        };
        let studio = o.studio(&model, mm_per_unit(&base), painter.on_host())?;
        o.create_dir()?;
        let canvas = Canvas {
            params: o.render_params(),
            studio,
            base: camera,
            path: &o.path,
            frames,
            dir: &o.dir,
        };
        let chunk_size = canvas.threads().saturating_mul(2).max(1) as u64;
        let mut meshes = Meshes::default();
        let mut srt = String::new();
        let mut failed: Vec<Value> = Vec::new();
        let mut frame = 0_u64;
        let mut k = 0_u64;
        while k < count {
            let end = k.saturating_add(chunk_size).min(count);
            let mut states: Vec<(Bodies, u64, u64, u64)> = Vec::new();
            for j in k..end {
                let (doc, regen, caption) = animation.at(j)?;
                let n = if j + 1 == count { 1 + o.hold } else { 1 };
                srt.push_str(&format!(
                    "{}\n{} --> {}\n{caption}\n\n",
                    j + 1,
                    srt_time(frame, o.fps),
                    srt_time(frame.saturating_add(n), o.fps)
                ));
                let broken: Vec<String> = doc
                    .features
                    .iter()
                    .filter(|f| {
                        matches!(
                            regen.feature(f.id).map(|r| &r.status),
                            Some(Status::Failed(_))
                        )
                    })
                    .map(|f| f.name.clone())
                    .collect();
                if !broken.is_empty() && failed.len() < 50 {
                    failed.push(json!({"frame": frame, "failed": broken}));
                }
                states.push((bodies_of(&doc, &regen, &[]), frame, n, fresh_key()));
                frame = frame.saturating_add(n);
            }
            meshes.keep_only(states.iter().map(|(bodies, ..)| bodies));
            for (bodies, ..) in &states {
                meshes.fill(bodies, &names, &tess)?;
            }
            if meshes.triangles() > MAX_REPLAY_TRIANGLES {
                return fail(format!(
                    "{ID}: the frames from {k} need {} triangles at once, more than the {MAX_REPLAY_TRIANGLES} a replay holds; give a coarser tolerance or a smaller model",
                    meshes.triangles()
                ));
            }
            let units: Vec<(&Bodies, u64, u64, u64)> = states
                .iter()
                .map(|(bodies, first, n, key)| (bodies, *first, *n, *key))
                .collect();
            canvas.paint(painter, &meshes, &canvas.jobs_of(&units))?;
            k = end;
        }
        o.finish(frame, &srt)?;
        let mut result = o.result(frame, &model, canvas.studio.as_ref().map(|(l, _)| l));
        let mut tracks_json: Vec<Value> = Vec::new();
        if let Some(track) = &animation.timeline {
            tracks_json.push(json!({
                "variable": crate::motion::TIMELINE,
                "from": track.at(0.0),
                "to": track.at(1.0),
                "timeline": true,
            }));
        }
        for tracked in &animation.tracks {
            tracks_json.push(json!({
                "variable": tracked.track.variable,
                "from": tracked.track.expr_at(0.0),
                "to": tracked.track.expr_at(1.0),
            }));
        }
        if let Some(map) = result.as_object_mut() {
            map.insert(
                "animate".into(),
                json!({"tracks": tracks_json, "states": count, "failed": failed}),
            );
        }
        Ok(result)
    }

    /// `view.replay` of the history.
    fn history_replay(session: &mut Session, p: &Params, painter: &mut Painter) -> Run {
        if let Some(key) = ANIMATION_ONLY.into_iter().find(|key| p.has(key)) {
            return fail(format!(
                "{ID}: `{key}` is for an animation (`animate`); the history's frames are framesPerStep and hold"
            ));
        }
        let every = p.bool("every")?.unwrap_or(false);
        let per_step = p.integer("framesPerStep", 1, 600)?.unwrap_or(1);
        let o = Options::of(session, p)?;
        let meshes = session.scene.meshes.clone();

        let open = session.document_mut()?;
        // The document's own regeneration, so its cache holds every
        // feature it builds: each step starts from a copy of that.
        open.regen();
        let pristine = open.cache.clone();
        let mut doc = craft3d_doc::Document::clone(open.history.current());
        let count = doc.features.len();
        if count == 0 {
            return fail(format!("{ID}: the document has no features to replay"));
        }
        let index_of = |id: FeatureId| doc.features.iter().position(|f| f.id == id);
        let from = match p.get("from") {
            None | Some(Value::Null) => 0,
            Some(_) => index_of(feature_ref(&doc, p, ID, "from")?).unwrap_or(0),
        };
        let to = match p.get("to") {
            None | Some(Value::Null) => doc.rollback.and_then(index_of).unwrap_or(count - 1),
            Some(_) => index_of(feature_ref(&doc, p, ID, "to")?).unwrap_or(count - 1),
        };
        if to < from {
            return fail(format!("{ID}: `to` comes before `from` in the history"));
        }

        // Every state of the range, regenerated on the copy.
        let at = |doc: &mut craft3d_doc::Document,
                  index: usize|
         -> Result<craft3d_regen::Regen, CommandError> {
            let id = doc
                .features
                .get(index)
                .map(|f| f.id)
                .ok_or_else(|| CommandError::new(format!("{ID}: lost a feature")))?;
            doc.set_rollback(Some(id))
                .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
            let mut cache = pristine.clone();
            Ok(craft3d_regen::regenerate_cached(doc, &mut cache))
        };
        let start: Bodies = match from.checked_sub(1) {
            None => Vec::new(),
            Some(before) => {
                let regen = at(&mut doc, before)?;
                bodies_of(&doc, &regen, &[])
            }
        };
        let mut steps: Vec<Step> = Vec::new();
        let mut last = start.clone();
        let mut final_regen = None;
        for index in from..=to {
            let regen = at(&mut doc, index)?;
            let Some(feature) = doc.features.get(index) else {
                return fail(format!("{ID}: lost a feature"));
            };
            let status = match regen.feature(feature.id).map(|o| &o.status) {
                Some(Status::Ok) => "ok",
                Some(Status::Suppressed) => "suppressed",
                Some(Status::Inactive) => "inactive",
                Some(Status::RolledBack) => "rolledBack",
                Some(Status::Failed(_)) => "failed",
                None => "unknown",
            };
            let bodies = bodies_of(&doc, &regen, &[]);
            let shown = every || !same(&bodies, &last);
            last.clone_from(&bodies);
            steps.push(Step {
                id: feature.id,
                name: feature.name.clone(),
                kind: feature.kind.type_name(),
                place: index + 1,
                status,
                bodies,
                shown,
            });
            if index == to {
                final_regen = Some(regen);
            }
        }
        let Some(final_regen) = final_regen else {
            return fail(format!("{ID}: nothing to replay"));
        };
        if steps.iter().all(|s| s.bodies.is_empty()) && !doc.hidden.is_empty() {
            let hidden: Vec<String> = doc.hidden.iter().map(ToString::to_string).collect();
            return fail(format!(
                "{ID}: every body of the range is hidden in the view ({}); show them with feature.visible",
                hidden.join(", ")
            ));
        }
        let shown = steps.iter().filter(|s| s.shown).count() as u64;
        let frames = (shown + 1).saturating_mul(per_step).saturating_add(o.hold);
        o.check_count(
            frames,
            &format!(
                "{shown} steps that change a body, plus the start, times framesPerStep {per_step}, plus hold {}; use fewer framesPerStep, a shorter hold or a range (from, to)",
                o.hold
            ),
        )?;

        let names = |id: FeatureId| {
            doc.feature(id)
                .map_or_else(|| id.to_string(), |f| f.name.clone())
        };
        // Framed, as view.render frames, on the box around the bodies at
        // `to` from their B-rep. Every step is sampled as view.render
        // samples the state at `to` (the document's own when the replay
        // ends where it does, so that the last frame is its picture): at
        // the tolerance given, or at the tolerance of the picture's scene
        // of that state for the replay's camera
        // (`crate::scene::picture_tolerance` and the ladder of
        // `crate::scene::picture_scene_of`), with the display's angle, each
        // body within the display's triangles.
        let model = crate::scene::bodies_box(&doc, &final_regen);
        let tess = o.tess(&doc, &final_regen, &meshes, None)?;
        let mut meshes = Meshes::default();
        meshes.fill(&last, &names, &tess)?;
        if model.is_empty() {
            return fail(format!(
                "{ID}: there are no bodies at the end of the replay; check document.inspect for failed features"
            ));
        }
        let camera = o.base_camera(&model)?;
        let studio = o.studio(&model, mm_per_unit(&doc), painter.on_host())?;
        o.create_dir()?;
        // What to draw where: the start, each step shown, the hold on the
        // last one shown (or on the start when none is).
        let mut units: Vec<(&Bodies, u64, u64, u64)> = Vec::new();
        let mut frame = 0_u64;
        let mut unit = |count: u64| -> (u64, u64) {
            let first = frame;
            frame = frame.saturating_add(count);
            (first, frame.saturating_sub(1))
        };
        let last_shown = steps.iter().rposition(|s| s.shown);
        let start_count = per_step.saturating_add(if last_shown.is_none() { o.hold } else { 0 });
        meshes.fill(&start, &names, &tess)?;
        let (first, _) = unit(start_count);
        units.push((&start, first, start_count, fresh_key()));
        let mut steps_json = Vec::new();
        let mut srt = String::new();
        let mut cue = 0_u64;
        for (k, step) in steps.iter().enumerate() {
            let text = caption(step.place, count, &step.name, step.kind);
            let range = if step.shown {
                let extra = if Some(k) == last_shown { o.hold } else { 0 };
                let n = per_step.saturating_add(extra);
                let (first, last) = unit(n);
                units.push((&step.bodies, first, n, fresh_key()));
                meshes.fill(&step.bodies, &names, &tess)?;
                cue += 1;
                srt.push_str(&format!(
                    "{cue}\n{} --> {}\n{text}\n\n",
                    srt_time(first, o.fps),
                    srt_time(last.saturating_add(1), o.fps)
                ));
                json!([first, last])
            } else {
                Value::Null
            };
            steps_json.push(json!({
                "feature": step.id.to_string(),
                "name": step.name,
                "type": step.kind,
                "status": step.status,
                "frames": range,
                "caption": text,
            }));
        }
        let canvas = Canvas {
            params: o.render_params(),
            studio,
            base: camera,
            path: &o.path,
            frames,
            dir: &o.dir,
        };
        canvas.paint(painter, &meshes, &canvas.jobs_of(&units))?;
        o.finish(frame, &srt)?;
        let mut result = o.result(frame, &model, canvas.studio.as_ref().map(|(l, _)| l));
        if let Some(map) = result.as_object_mut() {
            map.insert("shownSteps".into(), json!(shown));
            map.insert("steps".into(), json!(steps_json));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_stamps_and_frame_names() {
        assert_eq!(srt_time(0, 30), "00:00:00,000");
        assert_eq!(srt_time(45, 30), "00:00:01,500");
        assert_eq!(srt_time(30 * 3725, 30), "01:02:05,000");
        assert_eq!(frame_name(7), "frame-00007.png");
        assert_eq!(frame_index("frame-00007.png"), Some(7));
        assert_eq!(frame_index("frame-7.png"), None);
        assert_eq!(frame_index("frame-0000x.png"), None);
        assert_eq!(frame_index("captions.srt"), None);
        assert_eq!(
            caption(3, 10, "Plate", "block"),
            "Step 3/10 · Plate · block"
        );
    }
}
