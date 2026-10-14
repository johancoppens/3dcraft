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

use crate::commands::{Framing, Run, fail, feature_ref, spec, tolerance_param};
use crate::params::Params;
use crate::{CommandSpec, Session};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "view.replay",
    "Replay",
    r#"{"dir": string, "width": integer = 800, "height": integer = 600, "view": "iso|front|back|left|right|top|bottom" = "iso", "projection": "perspective|orthographic" = "perspective", "zoom": number = 1, "box": [[x, y, z], [x, y, z]] = the finished model, "tolerance": number = automatic, "from": id or name = the first feature, "to": id or name = the rollback marker or the last feature, "every": boolean = false, "framesPerStep": integer = 1, "hold": integer = 0, "orbit": number = 0, "fps": integer = 30}"#,
    "Renders how the document is built, for a showcase video: the history is rolled back to nothing and forward one feature at a time, from from to to, and each step is drawn with the CPU renderer as view.render would, from one camera framed on the model as it is at to, so the picture stays put. Writes the frames as dir/frame-00000.png, frame-00001.png, ... under the write root (frames of an earlier, longer replay there are removed) and dir/captions.srt, a subtitle per step with the feature's place, name and type, timed at fps. The first framesPerStep frames show the model before from (nothing, from the first feature), then framesPerStep frames per step, then hold more of the end; the last frame is the picture view.render gives of the document. A step that changes no body (a sketch, a datum, a failed or suppressed feature) gets no frames unless every is true. orbit turns the camera that many degrees about the vertical over the whole replay (the model is then framed so that it stays in the picture all round). The document does not change: not its rollback marker, not its undo history. At most 3600 frames, and 8e9 pixels in all. Returns the frame count, the steps (with the frames each got, or none) and the captions. craft3d-cli replay makes an MP4 or a GIF of it.",
    doc: true,
    view_replay
)];

/// Most frames one replay writes.
pub const MAX_REPLAY_FRAMES: u64 = 3600;
/// Most pixels one replay renders, over all its frames.
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

/// The caption of a step: its place in the history, name and type.
fn caption(place: usize, count: usize, name: &str, kind: &str) -> String {
    format!("{place}/{count}  {name} ({kind})")
}

/// `seconds` as an SRT time stamp, `00:01:02,500`.
fn srt_time(frame: u64, fps: u64) -> String {
    let millis = frame.saturating_mul(1000) / fps.max(1);
    let (h, rest) = (millis / 3_600_000, millis % 3_600_000);
    let (m, rest) = (rest / 60_000, rest % 60_000);
    let (s, ms) = (rest / 1000, rest % 1000);
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

/// The name of frame `index`.
fn frame_name(index: u64) -> String {
    format!("frame-{index:05}.png")
}

/// The index of a file named as [`frame_name`] names one.
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
    use crate::commands::vec3;
    use craft3d_doc::FeatureId;
    use craft3d_math::{Bbox3, Vec3};
    use craft3d_regen::Status;
    use craft3d_render::{Camera, RenderParams};
    use craft3d_tess::{Mesh, TessParams, tessellate_body};
    use craft3d_topo::Body;
    use std::sync::Arc;

    /// The bodies of one state of the history, in the order of the
    /// regeneration (the order `view.render` draws them in).
    type Bodies = Vec<(FeatureId, Arc<Body>)>;

    fn same(a: &Bodies, b: &Bodies) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|((ia, ba), (ib, bb))| ia == ib && Arc::ptr_eq(ba, bb))
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
        fn of(
            &mut self,
            body: &Arc<Body>,
            name: &str,
            tess: &TessParams,
        ) -> Result<Arc<Mesh>, CommandError> {
            if let Some((_, mesh)) = self.held.iter().find(|(b, _)| Arc::ptr_eq(b, body)) {
                return Ok(Arc::clone(mesh));
            }
            let mesh = Arc::new(
                tessellate_body(body, tess)
                    .map_err(|e| CommandError::new(format!("view.replay: {name}: {e}")))?,
            );
            self.held.push((Arc::clone(body), Arc::clone(&mesh)));
            Ok(mesh)
        }

        /// One mesh of all `bodies`, face ids counting on across bodies,
        /// exactly as `view.render` joins them.
        fn joined(
            &mut self,
            bodies: &Bodies,
            names: &dyn Fn(FeatureId) -> String,
            tess: &TessParams,
        ) -> Result<Mesh, CommandError> {
            let mut all = Mesh::default();
            for (id, body) in bodies {
                let mesh = self.of(body, &names(*id), tess)?;
                let vertex_base = u32::try_from(all.positions.len())
                    .map_err(|_| CommandError::new("view.replay: mesh too large"))?;
                let face_base = all.face_ids.iter().copied().max().map_or(0, |m| m + 1);
                all.positions.extend(&mesh.positions);
                all.normals.extend(&mesh.normals);
                all.triangles
                    .extend(mesh.triangles.iter().map(|t| t.map(|k| k + vertex_base)));
                all.face_ids
                    .extend(mesh.face_ids.iter().map(|f| f + face_base));
            }
            Ok(all)
        }
    }

    /// The box the camera frames when it orbits: wide enough in x and y
    /// that the model turned about the vertical through its centre stays
    /// inside it.
    fn orbit_box(model: &Bbox3) -> Option<Bbox3> {
        let (lo, hi) = model.corners()?;
        let c = model.center()?;
        let r = (hi.x - c.x).hypot(hi.y - c.y);
        Some(Bbox3::from_points([
            Vec3::new(c.x - r, c.y - r, lo.z),
            Vec3::new(c.x + r, c.y + r, hi.z),
        ]))
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
        let dir = session.workspace.resolve_write(dir_param)?;

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
        let bodies_of = |regen: &craft3d_regen::Regen| -> Bodies {
            regen.bodies().map(|(id, b)| (id, Arc::clone(b))).collect()
        };
        let start: Bodies = match from.checked_sub(1) {
            None => Vec::new(),
            Some(before) => bodies_of(&at(&mut doc, before)?),
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
            let bodies = bodies_of(&regen);
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
        // The tolerance view.render picks for these bodies: when the
        // replay ends where the document does, the document's.
        let tess = tolerance_param(&p, "view.replay", &final_regen)?;

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
        let mut meshes = Meshes::default();
        let end = meshes.joined(&last, &names, &tess)?;
        let model = end.bbox();
        if model.is_empty() {
            return fail(
                "view.replay: there are no bodies at the end of the replay; check document.inspect for failed features",
            );
        }
        let aspect = width as f64 / height as f64;
        let camera = if orbit == 0.0 {
            framing.camera(&model, aspect)?
        } else {
            let wide = orbit_box(&model)
                .ok_or_else(|| CommandError::new("view.replay: cannot frame the model"))?;
            framing.camera(&wide, aspect)?
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
        let mut frame = 0_u64;
        let mut write = |mesh: &Mesh, count: u64| -> Result<(u64, u64), CommandError> {
            let first = frame;
            let mut png: Option<Vec<u8>> = None;
            for _ in 0..count {
                if orbit != 0.0 || png.is_none() {
                    let angle = orbit * frame as f64 / frames.saturating_sub(1).max(1) as f64;
                    let picture =
                        craft3d_render::render(mesh, &turned(&camera, angle), &render_params)
                            .and_then(|f| f.to_png())
                            .map_err(|e| CommandError::new(format!("view.replay: {e}")))?;
                    png = Some(picture);
                }
                if let Some(png) = &png {
                    crate::workspace::write_atomic(&dir.join(frame_name(frame)), png)?;
                }
                frame += 1;
            }
            Ok((first, frame.saturating_sub(1)))
        };

        write(&meshes.joined(&start, &names, &tess)?, per_step)?;
        let mut steps_json = Vec::new();
        let mut srt = String::new();
        let mut cue = 0_u64;
        let last_shown = steps.iter().rposition(|s| s.shown);
        for (k, step) in steps.iter().enumerate() {
            let text = caption(step.place, count, &step.name, step.kind);
            let range = if step.shown {
                let extra = if Some(k) == last_shown { hold } else { 0 };
                let mesh = meshes.joined(&step.bodies, &names, &tess)?;
                let (first, last) = write(&mesh, per_step.saturating_add(extra))?;
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
        // Nothing shown at all: the hold is of the start.
        if last_shown.is_none() && hold > 0 {
            write(&meshes.joined(&start, &names, &tess)?, hold)?;
        }
        crate::workspace::write_atomic(&dir.join("captions.srt"), srt.as_bytes())?;
        // Frames an earlier, longer replay left here would run on after
        // this one's in a video made from the directory.
        if let Ok(entries) = std::fs::read_dir(&dir) {
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
        assert_eq!(caption(3, 10, "Plate", "block"), "3/10  Plate (block)");
    }
}
