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
    r#"{"dir": string, "width": integer = 800, "height": integer = 600, "view": "iso|front|back|left|right|top|bottom" = "iso", "projection": "perspective|orthographic" = "perspective", "zoom": number = 1, "box": [[x, y, z], [x, y, z]] = the finished model, "tolerance": number = the picture's at to, as view.render, "from": id or name = the first feature, "to": id or name = the rollback marker or the last feature, "every": boolean = false, "framesPerStep": integer = 1, "hold": integer = 0, "orbit": number = 0, "fps": integer = 30}"#,
    "Renders how the document is built, for a showcase video: the history is rolled back to nothing and forward one feature at a time, from from to to, and each step is drawn with the CPU renderer as view.render would, from one camera framed on the model as it is at to, so the picture stays put. Writes the frames as dir/frame-00000.png, frame-00001.png, ... under the write root (frames of an earlier, longer replay there are removed) and dir/captions.srt, a subtitle per step with the feature's place, name and type, timed at fps. The first framesPerStep frames show the model before from (nothing, from the first feature), then framesPerStep frames per step, then hold more of the end; without an orbit the last frame is the picture view.render gives of the document with the same view, projection, zoom, box and tolerance. Without tolerance every step is meshed at the tolerance view.render chooses for the model as it is at to, for the camera of the replay. A step that changes no body (a sketch, a datum, a failed or suppressed feature) gets no frames unless every is true. orbit turns the camera that many degrees about the vertical over the whole replay (the model is then framed so that it stays in the picture all round). The document does not change: not its rollback marker, not its undo history. At most 3600 frames, and 8e9 pixels in all. Returns the frame count, the steps (with the frames each got, or none) and the captions. craft3d-cli replay makes an MP4 or a GIF of it.",
    doc: true,
    view_replay
)];

/// Most frames one replay writes.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const MAX_REPLAY_FRAMES: u64 = 3600;
/// Most pixels one replay renders, over all its frames.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const MAX_REPLAY_PIXELS: u64 = 8_000_000_000;

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
    use crate::commands::{Framing, feature_ref, vec3};
    use craft3d_doc::FeatureId;
    use craft3d_math::{Bbox3, Vec3};
    use craft3d_regen::Status;
    use craft3d_render::{Camera, Projection, RenderParams};
    use craft3d_tess::{Mesh, TessParams, tessellate_within_budget};
    use craft3d_topo::Body;
    use serde_json::json;
    use std::sync::Arc;

    /// The bodies of one state of the history, in the order of the
    /// regeneration (the order `view.render` draws them in).
    /// The bodies of one state, each with the colour of each of its faces
    /// (`appearance`), which a body keeps from one state to the next.
    type Bodies = Vec<(FeatureId, Arc<Body>, Arc<Vec<Option<Look>>>)>;
    /// A face's colour and finish.
    type Look = ([u8; 3], craft3d_render::Finish);

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
        held: Vec<(Arc<Body>, Arc<Mesh>)>,
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
                    self.held.push((Arc::clone(body), Arc::new(mesh)));
                }
            }
            Ok(())
        }

        fn get(&self, body: &Arc<Body>) -> Option<&Mesh> {
            self.held
                .iter()
                .find(|(b, _)| Arc::ptr_eq(b, body))
                .map(|(_, mesh)| mesh.as_ref())
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
                    if let (Some(look), Ok(face)) = (look, u32::try_from(face)) {
                        colors.set_look(face_base.saturating_add(face), Some(*look));
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
            let (s, c) = angle.to_radians().sin_cos();
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
            let half_y = (fov_y_deg.to_radians() * 0.5).tan();
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

    /// `camera` turned by `degrees` about the vertical through its target.
    fn turned(camera: &Camera, degrees: f64) -> Camera {
        let (s, c) = degrees.to_radians().sin_cos();
        let turn = |v: Vec3| Vec3::new(c * v.x - s * v.y, s * v.x + c * v.y, v.z);
        Camera {
            eye: camera.target + turn(camera.eye - camera.target),
            up: turn(camera.up),
            ..*camera
        }
    }

    pub(super) fn view_replay(session: &mut Session, params: &Value) -> Run {
        let p = Params::new(
            "view.replay",
            params,
            &[
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
            ],
        )?;
        let dir_param = p.required_str("dir")?;
        let width = p.integer("width", 16, 4096)?.unwrap_or(800);
        let height = p.integer("height", 16, 4096)?.unwrap_or(600);
        let framing = Framing::from_params(&p, "view.replay")?;
        let every = p.bool("every")?.unwrap_or(false);
        let per_step = p.integer("framesPerStep", 1, 600)?.unwrap_or(1);
        let hold = p.integer("hold", 0, MAX_REPLAY_FRAMES)?.unwrap_or(0);
        let fps = p.integer("fps", 1, 240)?.unwrap_or(30);
        let orbit = match p.number("orbit")? {
            None => 0.0,
            Some(o) if o.is_finite() && o.abs() <= 3600.0 => o,
            Some(_) => {
                return fail("view.replay: `orbit` must be a number of degrees within ±3600");
            }
        };
        if session.writes_to_memory() {
            return fail(
                "view.replay: the frames go to a folder on disk, and this session keeps its files in memory",
            );
        }
        let dir = session.workspace.resolve_write(dir_param)?;
        let given = match p.number("tolerance")? {
            Some(t) if t > 0.0 => Some(t),
            Some(_) => return fail("view.replay: `tolerance` must be positive"),
            None => None,
        };
        let meshes = session.scene.meshes.clone();

        let open = session.document_mut()?;
        // The document's own regeneration, so its cache holds every
        // feature it builds: each step starts from a copy of that.
        open.regen();
        let pristine = open.cache.clone();
        let mut doc = craft3d_doc::Document::clone(open.history.current());
        let count = doc.features.len();
        if count == 0 {
            return fail("view.replay: the document has no features to replay");
        }
        let index_of = |id: FeatureId| doc.features.iter().position(|f| f.id == id);
        let from = match p.get("from") {
            None | Some(Value::Null) => 0,
            Some(_) => index_of(feature_ref(&doc, &p, "view.replay", "from")?).unwrap_or(0),
        };
        let to = match p.get("to") {
            None | Some(Value::Null) => doc.rollback.and_then(index_of).unwrap_or(count - 1),
            Some(_) => index_of(feature_ref(&doc, &p, "view.replay", "to")?).unwrap_or(count - 1),
        };
        if to < from {
            return fail("view.replay: `to` comes before `from` in the history");
        }

        // Every state of the range, regenerated on the copy.
        let at = |doc: &mut craft3d_doc::Document,
                  index: usize|
         -> Result<craft3d_regen::Regen, CommandError> {
            let id = doc
                .features
                .get(index)
                .map(|f| f.id)
                .ok_or_else(|| CommandError::new("view.replay: lost a feature"))?;
            doc.set_rollback(Some(id))
                .map_err(|e| CommandError::new(format!("view.replay: {e}")))?;
            let mut cache = pristine.clone();
            Ok(craft3d_regen::regenerate_cached(doc, &mut cache))
        };
        let bodies_of = |doc: &craft3d_doc::Document, regen: &craft3d_regen::Regen| -> Bodies {
            // What the view hides is not drawn (`feature.visible`).
            regen
                .bodies()
                .filter(|(id, _)| !doc.is_hidden(*id))
                .map(|(id, b)| {
                    let colors = crate::appearance::face_appearances(doc, regen, id, b.faces.len())
                        .into_iter()
                        .map(|a| a.map(|a| (a.color, crate::appearance::finish(&a))))
                        .collect();
                    (id, Arc::clone(b), Arc::new(colors))
                })
                .collect()
        };
        let start: Bodies = match from.checked_sub(1) {
            None => Vec::new(),
            Some(before) => {
                let regen = at(&mut doc, before)?;
                bodies_of(&doc, &regen)
            }
        };
        let mut steps: Vec<Step> = Vec::new();
        let mut last = start.clone();
        let mut final_regen = None;
        for index in from..=to {
            let regen = at(&mut doc, index)?;
            let Some(feature) = doc.features.get(index) else {
                return fail("view.replay: lost a feature");
            };
            let status = match regen.feature(feature.id).map(|o| &o.status) {
                Some(Status::Ok) => "ok",
                Some(Status::Suppressed) => "suppressed",
                Some(Status::RolledBack) => "rolledBack",
                Some(Status::Failed(_)) => "failed",
                None => "unknown",
            };
            let bodies = bodies_of(&doc, &regen);
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
            return fail("view.replay: nothing to replay");
        };
        if steps.iter().all(|s| s.bodies.is_empty()) && !doc.hidden.is_empty() {
            let hidden: Vec<String> = doc.hidden.iter().map(ToString::to_string).collect();
            return fail(format!(
                "view.replay: every body of the range is hidden in the view ({}); show them with feature.visible",
                hidden.join(", ")
            ));
        }
        let shown = steps.iter().filter(|s| s.shown).count() as u64;
        let frames = (shown + 1).saturating_mul(per_step).saturating_add(hold);
        if frames > MAX_REPLAY_FRAMES {
            return fail(format!(
                "view.replay: {frames} frames ({shown} steps that change a body, plus the start, times framesPerStep {per_step}, plus hold {hold}) exceed the limit of {MAX_REPLAY_FRAMES}; use fewer framesPerStep, a shorter hold or a range (from, to)"
            ));
        }
        let pixels = frames.saturating_mul(width).saturating_mul(height);
        if pixels > MAX_REPLAY_PIXELS {
            return fail(format!(
                "view.replay: {frames} frames of {width}x{height} are {pixels} pixels, more than the limit of {MAX_REPLAY_PIXELS}; make them smaller or fewer"
            ));
        }

        // The camera, framed on the model as it is at the end.
        let names = |id: FeatureId| {
            doc.feature(id)
                .map_or_else(|| id.to_string(), |f| f.name.clone())
        };
        // Framed, as view.render frames, on the box around the bodies at
        // `to` from their B-rep.
        let model = crate::scene::bodies_box(&doc, &final_regen);
        let aspect = width as f64 / height as f64;
        // Every step is sampled as view.render samples the state at `to`
        // (the document's own when the replay ends where it does, so that
        // the last frame is its picture): at the tolerance given, or at the
        // tolerance of the picture's scene of that state for the replay's
        // camera (`crate::scene::picture_tolerance` and the ladder of
        // `crate::scene::picture_scene_of`), with the display's angle, each
        // body within the display's triangles.
        let tess = match given {
            Some(chordal_tolerance) => TessParams {
                chordal_tolerance,
                ..TessParams::default()
            },
            None => {
                let rung = crate::scene::picture_tolerance(&final_regen, &doc, |model| {
                    let camera = if orbit == 0.0 {
                        framing.camera(model, aspect).ok()?
                    } else {
                        orbit_camera(&framing, &framing.look_at.unwrap_or(*model), orbit, aspect)?
                    };
                    // Both are within 16..=4096.
                    crate::scene::pixel_footprint(&camera, height as u32, model)
                });
                let settled = crate::scene::picture_scene_of(&doc, &final_regen, 0, rung, &meshes);
                if let Some(problem) = settled.problems.first() {
                    return fail(format!("view.replay: {problem}"));
                }
                TessParams {
                    chordal_tolerance: settled.tolerance,
                    max_angle: settled.max_angle,
                    max_triangles: Some(crate::scene::DISPLAY_MAX_TRIANGLES),
                    ..TessParams::default()
                }
            }
        };
        let mut meshes = Meshes::default();
        meshes.fill(&last, &names, &tess)?;
        if model.is_empty() {
            return fail(
                "view.replay: there are no bodies at the end of the replay; check document.inspect for failed features",
            );
        }
        let camera = if orbit == 0.0 {
            framing.camera(&model, aspect)?
        } else {
            orbit_camera(&framing, &framing.look_at.unwrap_or(model), orbit, aspect)
                .ok_or_else(|| CommandError::new("view.replay: cannot frame the model"))?
        };
        // Both are within 16..=4096.
        let render_params = RenderParams {
            width: width as u32,
            height: height as u32,
            ..RenderParams::default()
        };

        std::fs::create_dir_all(&dir).map_err(|e| {
            CommandError::new(format!(
                "view.replay: cannot create the directory `{dir_param}`: {e}"
            ))
        })?;
        // What to draw where: the start, each step shown, the hold on the
        // last one shown (or on the start when none is).
        let mut units: Vec<(&Bodies, u64, u64)> = Vec::new();
        let mut frame = 0_u64;
        let mut unit = |count: u64| -> (u64, u64) {
            let first = frame;
            frame = frame.saturating_add(count);
            (first, frame.saturating_sub(1))
        };
        let last_shown = steps.iter().rposition(|s| s.shown);
        let start_count = per_step.saturating_add(if last_shown.is_none() { hold } else { 0 });
        meshes.fill(&start, &names, &tess)?;
        let (first, _) = unit(start_count);
        units.push((&start, first, start_count));
        let mut steps_json = Vec::new();
        let mut srt = String::new();
        let mut cue = 0_u64;
        for (k, step) in steps.iter().enumerate() {
            let text = caption(step.place, count, &step.name, step.kind);
            let range = if step.shown {
                let extra = if Some(k) == last_shown { hold } else { 0 };
                let n = per_step.saturating_add(extra);
                let (first, last) = unit(n);
                units.push((&step.bodies, first, n));
                meshes.fill(&step.bodies, &names, &tess)?;
                cue += 1;
                srt.push_str(&format!(
                    "{cue}\n{} --> {}\n{text}\n\n",
                    srt_time(first, fps),
                    srt_time(last.saturating_add(1), fps)
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
        // With an orbit every frame is a picture of its own; without, a
        // unit is drawn once and written as often as it is shown.
        let jobs: Vec<(&Bodies, u64, u64)> = if orbit == 0.0 {
            units
        } else {
            units
                .into_iter()
                .flat_map(|(bodies, first, n)| {
                    (first..first.saturating_add(n)).map(move |f| (bodies, f, 1))
                })
                .collect()
        };
        let meshes = &meshes;
        let (dir, camera, render_params) = (&dir, &camera, &render_params);
        let draw = |job: usize| -> Result<(), CommandError> {
            let Some(&(bodies, first, n)) = jobs.get(job) else {
                return Ok(());
            };
            let (mesh, colors) = meshes.joined(bodies)?;
            let angle = orbit * first as f64 / frames.saturating_sub(1).max(1) as f64;
            let png = craft3d_render::render_colored(
                &mesh,
                &colors,
                &turned(camera, angle),
                render_params,
            )
            .and_then(|f| f.to_png())
            .map_err(|e| CommandError::new(format!("view.replay: {e}")))?;
            for f in first..first.saturating_add(n) {
                crate::workspace::write_atomic(&dir.join(frame_name(f)), &png)?;
            }
            Ok(())
        };
        in_parallel(
            jobs.len(),
            threads_for(width, height, render_params.ssaa),
            draw,
        )?;
        crate::workspace::write_atomic(&dir.join("captions.srt"), srt.as_bytes())?;
        // Frames an earlier, longer replay left here would run on after
        // this one's in a video made from the directory.
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name
                    .to_str()
                    .and_then(frame_index)
                    .is_some_and(|index| index >= frame)
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }

        Ok(json!({
            "dir": dir_param,
            "frames": frame,
            "framePattern": "frame-%05d.png",
            "fps": fps,
            "width": width,
            "height": height,
            "view": framing.view,
            "orbit": orbit,
            "box": model.corners().map(|(lo, hi)| json!([vec3(lo), vec3(hi)])),
            "shownSteps": shown,
            "steps": steps_json,
            "captions": "captions.srt",
        }))
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
