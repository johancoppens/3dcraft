//! Pointing: which face is under a position of a picture, and which faces
//! of the scene a persistent name means.
//!
//! Both are questions about a [`Scene`], so that a viewer asks them of the
//! very mesh it draws: the window of the scene in its viewport, with the
//! viewport's camera, and `view.pick` of the scene `view.render` draws,
//! with the camera `view.render` frames. The answer is the renderer's own
//! (`craft3d_render::pick`): at the centre of a pixel it is the face the
//! picture's id buffer holds for that pixel.

use crate::commands::{Framing, Run, fail, spec, vec3};
use crate::params::Params;
use crate::scene::{Scene, SceneBody};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::PersistentName;
use craft3d_math::Vec3;
use craft3d_render::{Camera, Projection};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("view.pick", "Pick", r#"{"x": number, "y": number, "width": integer = 800, "height": integer = 600, "view": "iso|front|back|left|right|top|bottom" = "iso", "projection": "perspective|orthographic" = "perspective", "zoom": number = 1, "box": [[x, y, z], [x, y, z]] = the whole model, "camera": {eye, target, up, projection, fovY, height}, "tolerance": number = automatic}"#,
        "Says what is at a position of a picture: the body (feature, name), the face's persistent name as document.inspect lists it under faceNames (with faceIndex, its place in that list, and aliases), and the point in space. x and y are in pixels from the top-left corner of a picture of width by height, x to the right and y down; the centre of pixel (i, j) is (i + 0.5, j + 0.5). The picture is the one view.render makes with the same view, projection, zoom, box and tolerance, or the one seen by camera when that is given: an object with eye and target ([x, y, z]), up ([x, y, z], default [0, 0, 1]), projection (perspective, the default, or orthographic) and, for a perspective camera, fovY (the vertical field of view in degrees, default 30) or, for an orthographic one, height (the model units from the top of the picture to its bottom). hit is false when nothing is there, with outside true when the position is not in the picture: an answer, not an error. The result repeats the camera, so a second pick can pass it back.", doc: true, view_pick),
];

/// What is under a position of a picture of a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenePick {
    /// The feature that made the body, as `document.inspect` writes it
    /// (`F3`), and its name.
    pub feature: String,
    pub body: String,
    /// The face's persistent name, as `document.inspect` lists it under
    /// `faceNames`; `None` for a body whose faces have no names.
    pub face: Option<String>,
    /// The other names the face answers to.
    pub aliases: Vec<String>,
    /// The face's place among the faces of its body.
    pub face_index: usize,
    /// The face's id in [`Scene::mesh`].
    pub face_id: u32,
    /// The point under the position, in model coordinates.
    pub point: Vec3,
    /// Its distance from the eye along the view direction.
    pub depth: f64,
}

/// The faces of a scene a persistent name means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneFaces {
    /// The feature that made the body (`F3`) and its name.
    pub feature: String,
    pub body: String,
    /// The name as it was asked for, written the way names are written.
    pub face: String,
    /// The ids of the faces in [`Scene::mesh`], ascending. More than one
    /// when a boolean cut the face into pieces that kept its name.
    pub ids: Vec<u32>,
}

impl Scene {
    /// The body that has the face with id `face_id` of [`Scene::mesh`],
    /// and the face's place among the faces of that body. `None` for an
    /// id no body has.
    pub fn face_of(&self, face_id: u32) -> Option<(&SceneBody, usize)> {
        if face_id == craft3d_render::NO_FACE {
            return None;
        }
        // Ids keep counting across bodies: the last body that starts at or
        // before the id.
        let body = self
            .bodies
            .iter()
            .rev()
            .find(|body| body.first_face_id <= face_id)?;
        let index = usize::try_from(face_id.checked_sub(body.first_face_id)?).ok()?;
        // A body whose faces have names has as many names as faces.
        (body.face_names.is_empty() || index < body.face_names.len()).then_some((body, index))
    }

    /// What `craft3d_render::render` draws at the position (`x`, `y`) of a
    /// picture of `width` × `height` pixels of this scene seen from
    /// `camera`: x to the right and y down from the top-left corner, the
    /// centre of pixel (i, j) at (i + 0.5, j + 0.5). `Ok(None)` when no
    /// body is there, also for a position outside the picture or one that
    /// is not a number. An error for a picture that cannot be made (its
    /// size, a camera that looks nowhere).
    pub fn pick(
        &self,
        camera: &Camera,
        width: u32,
        height: u32,
        x: f64,
        y: f64,
    ) -> Result<Option<ScenePick>, String> {
        let hit = craft3d_render::pick(&self.mesh, camera, width, height, x, y)
            .map_err(|e| e.to_string())?;
        let Some(hit) = hit else {
            return Ok(None);
        };
        let Some((body, index)) = self.face_of(hit.face) else {
            return Ok(None);
        };
        Ok(Some(ScenePick {
            feature: body.feature.clone(),
            body: body.name.clone(),
            face: body.face_names.get(index).cloned(),
            aliases: body.face_aliases.get(index).cloned().unwrap_or_default(),
            face_index: index,
            face_id: hit.face,
            point: hit.point,
            // The point's own distance, not the picture's depth value.
            depth: camera
                .axes()
                .map_or(hit.depth, |axes| (hit.point - axes.eye).dot(axes.forward)),
        }))
    }

    /// The faces `face` names: a persistent name from `document.inspect`
    /// (`faceNames`, or an alias from `faceAliases`). `feature` says which
    /// body, by the id of its feature (`F3` or `3`) or by its name; it is
    /// needed only when several bodies have a face of that name. The error
    /// is a sentence for whoever asked.
    pub fn faces_named(&self, feature: Option<&str>, face: &str) -> Result<SceneFaces, String> {
        let name: PersistentName = face.parse()?;
        if !name.is_face() {
            return Err(format!("{name} is not a face"));
        }
        let wanted = name.to_string();
        let ids_in = |body: &SceneBody| -> Vec<u32> {
            (0..body.face_names.len())
                .filter(|&k| {
                    body.face_names.get(k) == Some(&wanted)
                        || body
                            .face_aliases
                            .get(k)
                            .is_some_and(|aliases| aliases.contains(&wanted))
                })
                .filter_map(|k| u32::try_from(k).ok()?.checked_add(body.first_face_id))
                .collect()
        };
        let found = |body: &SceneBody, ids: Vec<u32>| SceneFaces {
            feature: body.feature.clone(),
            body: body.name.clone(),
            face: wanted.clone(),
            ids,
        };
        if let Some(feature) = feature {
            let shown: String = feature.chars().take(60).collect();
            // A name first, as the commands do, then an id with or
            // without its F.
            let body = self
                .bodies
                .iter()
                .find(|body| body.name == feature)
                .or_else(|| {
                    let digits = feature.strip_prefix('F').unwrap_or(feature);
                    let id = digits.parse::<u64>().ok()?;
                    self.bodies
                        .iter()
                        .find(|body| body.feature == format!("F{id}"))
                })
                .ok_or_else(|| {
                    format!(
                        "there is no body of a feature `{shown}`; the bodies: {}",
                        self.body_list()
                    )
                })?;
            let ids = ids_in(body);
            if ids.is_empty() {
                return Err(format!(
                    "`{}` has no face {wanted} (document.inspect lists its faceNames)",
                    body.name
                ));
            }
            return Ok(found(body, ids));
        }
        let mut having = self
            .bodies
            .iter()
            .map(|body| (body, ids_in(body)))
            .filter(|(_, ids)| !ids.is_empty());
        let Some((body, ids)) = having.next() else {
            return Err(format!(
                "no body has a face {wanted} (document.inspect lists the faceNames of every body)"
            ));
        };
        if let Some((other, _)) = having.next() {
            return Err(format!(
                "several bodies have a face {wanted} (`{}` and `{}` among them); say which with `feature`",
                body.name, other.name
            ));
        }
        Ok(found(body, ids))
    }

    /// The bodies as an error message lists them.
    fn body_list(&self) -> String {
        if self.bodies.is_empty() {
            return "none".to_string();
        }
        let listed: Vec<String> = self
            .bodies
            .iter()
            .take(20)
            .map(|body| format!("{} ({})", body.name, body.feature))
            .collect();
        listed.join(", ")
    }
}

/// A camera as `view.pick` returns it and takes it back.
fn camera_json(camera: &Camera) -> Value {
    let mut out = json!({
        "eye": vec3(camera.eye),
        "target": vec3(camera.target),
        "up": vec3(camera.up),
    });
    if let Some(map) = out.as_object_mut() {
        match camera.projection {
            Projection::Perspective { fov_y_deg } => {
                map.insert("projection".into(), "perspective".into());
                map.insert("fovY".into(), fov_y_deg.into());
            }
            Projection::Orthographic { half_height } => {
                map.insert("projection".into(), "orthographic".into());
                map.insert("height".into(), (2.0 * half_height).into());
            }
        }
    }
    out
}

/// The `camera` of `view.pick`.
fn camera_param(value: &Value) -> Result<Camera, CommandError> {
    const ID: &str = "view.pick";
    let p = Params::new(
        ID,
        value,
        &["eye", "target", "up", "projection", "fovY", "height"],
    )
    .map_err(|e| CommandError::new(format!("{e} (of `camera`)")))?;
    let point = |key: &str| -> Result<Option<Vec3>, CommandError> {
        let Some(value) = p.get(key) else {
            return Ok(None);
        };
        let number = |v: &Value| v.as_f64().filter(|n| n.is_finite() && n.abs() <= 1e12);
        let parsed = match value.as_array().map(Vec::as_slice) {
            Some([x, y, z]) => (|| Some(Vec3::new(number(x)?, number(y)?, number(z)?)))(),
            _ => None,
        };
        parsed.map(Some).ok_or_else(|| {
            CommandError::new(format!(
                "{ID}: `camera.{key}` must be [x, y, z], numbers within ±1e12"
            ))
        })
    };
    let eye = point("eye")?
        .ok_or_else(|| CommandError::new(format!("{ID}: `camera` needs `eye`, [x, y, z]")))?;
    let target = point("target")?
        .ok_or_else(|| CommandError::new(format!("{ID}: `camera` needs `target`, [x, y, z]")))?;
    let up = point("up")?.unwrap_or(Vec3::Z);
    let projection = match p.str("projection")?.unwrap_or("perspective") {
        "perspective" => {
            if p.get("height").is_some() {
                return fail(format!(
                    "{ID}: `camera.height` belongs to an orthographic camera; a perspective one takes `fovY`"
                ));
            }
            let fov_y_deg = match p.number("fovY")? {
                None => 30.0,
                Some(fov) if fov > 0.0 && fov < 180.0 => fov,
                Some(_) => {
                    return fail(format!(
                        "{ID}: `camera.fovY` must be an angle between 0 and 180 degrees"
                    ));
                }
            };
            Projection::Perspective { fov_y_deg }
        }
        "orthographic" => {
            if p.get("fovY").is_some() {
                return fail(format!(
                    "{ID}: `camera.fovY` belongs to a perspective camera; an orthographic one takes `height`"
                ));
            }
            match p.number("height")? {
                Some(height) if height > 0.0 && height <= 1e12 => Projection::Orthographic {
                    half_height: 0.5 * height,
                },
                Some(_) => {
                    return fail(format!(
                        "{ID}: `camera.height` must be a positive length, at most 1e12"
                    ));
                }
                None => {
                    return fail(format!(
                        "{ID}: an orthographic `camera` needs `height`, the model units from the top of the picture to its bottom"
                    ));
                }
            }
        }
        other => {
            let other: String = other.chars().take(40).collect();
            return fail(format!(
                "{ID}: unknown `camera.projection` `{other}`; use perspective or orthographic"
            ));
        }
    };
    let camera = Camera {
        eye,
        target,
        up,
        projection,
    };
    // A camera that looks nowhere (the eye on the target, up along the
    // view) is refused here, with its reason.
    camera
        .axes()
        .map_err(|e| CommandError::new(format!("{ID}: `camera`: {e}")))?;
    Ok(camera)
}

fn view_pick(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "view.pick";
    let p = Params::new(
        ID,
        params,
        &[
            "x",
            "y",
            "width",
            "height",
            "view",
            "projection",
            "zoom",
            "box",
            "camera",
            "tolerance",
        ],
    )?;
    let x = p.number("x")?.ok_or_else(|| p.missing("x"))?;
    let y = p.number("y")?.ok_or_else(|| p.missing("y"))?;
    let width = p.integer("width", 16, 4096)?.unwrap_or(800);
    let height = p.integer("height", 16, 4096)?.unwrap_or(600);
    let given = p.get("camera").map(camera_param).transpose()?;
    if given.is_some()
        && let Some(key) = Framing::KEYS.iter().find(|key| p.get(key).is_some())
    {
        return fail(format!(
            "{ID}: `{key}` frames the model as view.render does; with `camera` the picture is the one that camera sees, so give one or the other"
        ));
    }
    let framing = Framing::from_params(&p, ID)?;
    let tolerance = match p.number("tolerance")? {
        Some(t) if t > 0.0 => Some(t),
        Some(_) => return fail(format!("{ID}: `tolerance` must be positive")),
        None => None,
    };
    let Some(scene) = session.scene(tolerance)? else {
        return fail(format!(
            "{ID}: no document is open; run file.new or file.open first"
        ));
    };
    // Both are within 16..=4096.
    let (w, h) = (width as u32, height as u32);
    let camera = match given {
        Some(camera) => camera,
        // The picture of view.render: the model is the box of its mesh.
        None => framing.camera(&scene.mesh.bbox(), width as f64 / height as f64)?,
    };
    let outside = !(x >= 0.0 && y >= 0.0 && x < width as f64 && y < height as f64);
    let picked = scene
        .pick(&camera, w, h, x, y)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let mut result = json!({
        "hit": picked.is_some(),
        "x": x,
        "y": y,
        "width": width,
        "height": height,
        "camera": camera_json(&camera),
    });
    let Some(map) = result.as_object_mut() else {
        return Ok(result);
    };
    match picked {
        None => {
            map.insert("outside".into(), outside.into());
        }
        Some(pick) => {
            map.insert("feature".into(), pick.feature.into());
            map.insert("name".into(), pick.body.into());
            map.insert("face".into(), pick.face.map_or(Value::Null, Value::from));
            map.insert("faceIndex".into(), pick.face_index.into());
            if !pick.aliases.is_empty() {
                map.insert("aliases".into(), json!(pick.aliases));
            }
            map.insert("point".into(), vec3(pick.point));
            map.insert("depth".into(), pick.depth.into());
        }
    }
    Ok(result)
}
