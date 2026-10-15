//! Pointing: which face is under a position of a picture and which edge or
//! vertex is near it, and which faces, edges and vertices of the scene a
//! name means.
//!
//! The rule for what a position points at ([`Scene::point_at`]), the same
//! for `view.pick`, `ui.pick` and a click in the viewport:
//!
//! 1. The face is the one the picture has under the position (the
//!    renderer's own arithmetic), whatever else is near.
//! 2. An edge is near when a point of it is within the pick radius of the
//!    position in the picture (pixels of the picture) and in sight: no
//!    face is in front of that point. An edge is looked at every half
//!    pixel along the part of it within the radius, starting from its
//!    nearest point. The nearest point in sight of all edges wins; at the
//!    same distance the one nearer to the eye, then the earlier body, then
//!    the earlier edge.
//! 3. A vertex is near when it is within the pick radius of the position
//!    in the picture and in sight; the nearest wins, with the same
//!    tie-breaks.
//! 4. A vertex that is near is what the position points at, however near
//!    an edge is (an edge ends in its vertices, so near a vertex an edge
//!    is always nearer or as near); otherwise an edge that is near;
//!    otherwise the face under it; otherwise nothing.
//!
//! In a picture that a section cuts open ([`Scene::point_at_cut`]) the
//! answer is of what is drawn: a face in the half that is kept, never one
//! that is cut away, and where the cap of the section is in front, the
//! section itself, with the body that is cut there and the point of the
//! plane. Edges and vertices in the half that is cut away are not near
//! anything, and the cap hides what is behind it like a face.
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
use craft3d_render::{Camera, Neighbourhood, Projection, SECTION_FACE, Section, Viewpoint};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("view.pick", "Pick", r#"{"x": number, "y": number, "width": integer = 800, "height": integer = 600, "radius": number = 4, "view": "iso|front|back|left|right|top|bottom" = "iso", "projection": "perspective|orthographic" = "perspective", "zoom": number = 1, "box": [[x, y, z], [x, y, z]] = the whole model, "camera": {eye, target, up, projection, fovY, height}, "section": {plane, feature, point, normal, offset, flip, cap} = none, "tolerance": number = the picture's, as view.render}"#,
        "Says what is at a position of a picture. kind says what the position points at: a vertex when one is near, otherwise an edge when one is near, otherwise the face under it. The face under the position, when there is one: the body (feature, name), the face's persistent name as document.inspect lists it under faceNames (with faceIndex, its place in that list, and aliases), and the point in space (point, depth). An edge is near when its point nearest to the position in the picture is within radius pixels (0 to 64; 0 looks at faces only) and not behind a face; the nearest one wins. It is given as edge: its body (feature, body), name (the edge's own persistent name as document.edges gives it, null when it has none, as after a boolean), faces (the names of the faces it lies between, which is how document.edges refers to an edge), index (its place in document.edges), point (the point of the edge nearest to the position), depth and distance (pixels from the position). A vertex is near when it is within radius pixels and not behind a face; it wins over an edge and is given as vertex, with the same fields: name is its own persistent name or null, faces the names of the faces that meet in it, index its index under vertices of document.edges. x and y are in pixels from the top-left corner of a picture of width by height, x to the right and y down; the centre of pixel (i, j) is (i + 0.5, j + 0.5). The picture is the one view.render makes with the same view, projection, zoom, box and tolerance, or the one seen by camera when that is given: an object with eye and target ([x, y, z]), up ([x, y, z], default [0, 0, 1]), projection (perspective, the default, or orthographic) and, for a perspective camera, fovY (the vertical field of view in degrees, default 30) or, for an orthographic one, height (the model units from the top of the picture to its bottom). hit is false when nothing is there, with outside true when the position is not in the picture: an answer, not an error. The result repeats the camera, so a second pick can pass it back, and gives the chordal tolerance of the mesh it looked at (tolerance). section cuts the picture open as in view.render (the same object); the answer is then of what that picture shows: never a face that is cut away, and where the cap of the section is in front, kind is section, with section: {feature, body, point, depth}, the body that is cut there and the point of the plane.", doc: true, view_pick),
];

/// Most face names a vertex is referred to by.
pub const MAX_CORNER_FACES: usize = 16;
/// The pick radius when none is given, in pixels: an edge this near to a
/// position is what the position points at.
pub const DEFAULT_PICK_RADIUS: f64 = 4.0;
/// The largest pick radius, in pixels. A larger one is clamped.
pub const MAX_PICK_RADIUS: f64 = 64.0;
/// An edge is looked at every this many pixels along it, where it is
/// within the pick radius.
const SAMPLE_STEP: f64 = 0.5;
/// Most points of edges near a position that are looked at. A picture
/// with more of them within the radius (thousands of edges through one
/// spot) is answered from the first ones, in the order of the scene.
const MAX_CANDIDATES: usize = 1 << 16;
/// How much nearer than a point the surface in front of it may be while
/// the point still counts as lying on it, as a fraction of the point's
/// distance plus the size of the scene. An edge's points are points of the
/// mesh, so this is rounding only.
const SIGHT_SLACK: f64 = 1e-7;

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

/// The cap of a section under a position of a picture: where the plane
/// passes through the inside of a body.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneCut {
    /// The feature that made the body that is cut there (`F3`) and its
    /// name; `None` when the faces cut away in front of the cap do not say
    /// which body it is (bodies that overlap in space).
    pub feature: Option<String>,
    pub body: Option<String>,
    /// The point of the plane under the position, in model coordinates.
    pub point: Vec3,
    /// Its distance from the eye along the view direction.
    pub depth: f64,
}

/// What a position of a picture points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind {
    Face,
    Edge,
    Vertex,
    /// The cap of a section.
    Section,
}

impl PickKind {
    /// The word the commands use.
    pub fn name(self) -> &'static str {
        match self {
            PickKind::Face => "face",
            PickKind::Edge => "edge",
            PickKind::Vertex => "vertex",
            PickKind::Section => "section",
        }
    }
}

/// An edge or a vertex near a position of a picture of a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneNear {
    /// The feature that made the body (`F3`) and its name.
    pub feature: String,
    pub body: String,
    /// The entity's own persistent name, as `document.edges` gives it;
    /// `None` when the body's edges and vertices have none (after a
    /// boolean).
    pub name: Option<String>,
    /// Its place among the edges, or the vertices, of its body: the order
    /// of `document.edges`, and `index` of its `vertices`.
    pub index: usize,
    /// The names of the faces an edge lies between, or that meet in a
    /// vertex: how it is referred to when it has no name of its own.
    pub faces: Vec<String>,
    /// The point of the entity nearest to the position, in model
    /// coordinates.
    pub point: Vec3,
    /// Its distance from the eye along the view direction.
    pub depth: f64,
    /// How far from the position it is in the picture, in pixels.
    pub distance: f64,
}

impl SceneNear {
    /// As `view.pick` and `ui.pick` return it.
    pub fn json(&self) -> Value {
        json!({
            "feature": self.feature,
            "body": self.body,
            "name": self.name,
            "index": self.index,
            "faces": self.faces,
            "point": vec3(self.point),
            "depth": self.depth,
            "distance": self.distance,
        })
    }
}

/// What a position of a picture of a scene points at
/// ([`Scene::point_at`]).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ScenePointed {
    /// The face under the position.
    pub face: Option<ScenePick>,
    /// The edge near it, which is then what the position points at,
    /// unless a vertex is near too.
    pub edge: Option<SceneNear>,
    /// The vertex near it, which is then what the position points at.
    pub vertex: Option<SceneNear>,
    /// The cap of a section under the position, when the picture is cut
    /// open and the cap is what it shows there; there is then no face.
    pub section: Option<SceneCut>,
}

impl ScenePointed {
    /// What the position points at: `None` for nothing.
    pub fn kind(&self) -> Option<PickKind> {
        if self.vertex.is_some() {
            Some(PickKind::Vertex)
        } else if self.edge.is_some() {
            Some(PickKind::Edge)
        } else if self.face.is_some() {
            Some(PickKind::Face)
        } else if self.section.is_some() {
            Some(PickKind::Section)
        } else {
            None
        }
    }

    /// The fields `view.pick` and `ui.pick` have in common, put into
    /// `map`: `hit`, and for a hit `kind`, the face under the position
    /// and the vertex or, without one, the edge near it.
    pub fn describe(&self, map: &mut serde_json::Map<String, Value>) {
        let kind = self.kind();
        map.insert("hit".into(), kind.is_some().into());
        if let Some(kind) = kind {
            map.insert("kind".into(), kind.name().into());
        }
        if let Some(pick) = &self.face {
            map.insert("feature".into(), pick.feature.clone().into());
            map.insert("name".into(), pick.body.clone().into());
            map.insert(
                "face".into(),
                pick.face.clone().map_or(Value::Null, Value::from),
            );
            map.insert("faceIndex".into(), pick.face_index.into());
            if !pick.aliases.is_empty() {
                map.insert("aliases".into(), json!(pick.aliases));
            }
            map.insert("point".into(), vec3(pick.point));
            map.insert("depth".into(), pick.depth.into());
        }
        if let Some(cut) = &self.section {
            map.insert(
                "section".into(),
                json!({
                    "feature": cut.feature,
                    "body": cut.body,
                    "point": vec3(cut.point),
                    "depth": cut.depth,
                }),
            );
        }
        match (&self.vertex, &self.edge) {
            (Some(vertex), _) => {
                map.insert("vertex".into(), vertex.json());
            }
            (None, Some(edge)) => {
                map.insert("edge".into(), edge.json());
            }
            (None, None) => {}
        }
    }
}

/// The edges of a scene a name, or the faces they lie between, mean.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneEdges {
    /// The feature that made the body (`F3`) and its name.
    pub feature: String,
    pub body: String,
    /// The body's place in [`Scene::bodies`].
    pub body_index: usize,
    /// The edge's own name, when that is what was asked for, written the
    /// way names are written.
    pub edge: Option<String>,
    /// The two face names, when those were asked for.
    pub between: Vec<String>,
    /// The places of the edges among the edges of the body, ascending.
    pub edges: Vec<usize>,
}

/// The vertices of a scene a name, or the faces that meet in them, mean.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneVertices {
    /// The feature that made the body (`F3`) and its name.
    pub feature: String,
    pub body: String,
    /// The body's place in [`Scene::bodies`].
    pub body_index: usize,
    /// The vertex's own name, when that is what was asked for.
    pub vertex: Option<String>,
    /// The face names, when those were asked for.
    pub corner: Vec<String>,
    /// The places of the vertices among the vertices of the body,
    /// ascending.
    pub vertices: Vec<usize>,
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
        if face_id == craft3d_render::NO_FACE || face_id == SECTION_FACE {
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
        Ok(self.pick_cut(camera, width, height, x, y, None)?.0)
    }

    /// [`Self::pick`] in a picture cut open by `section`: the face that
    /// picture shows under the position, or, where it shows the cap of the
    /// section, the cap ([`SceneCut`]). Never a face that is cut away.
    pub fn pick_cut(
        &self,
        camera: &Camera,
        width: u32,
        height: u32,
        x: f64,
        y: f64,
        section: Option<&Section>,
    ) -> Result<(Option<ScenePick>, Option<SceneCut>), String> {
        let hit = craft3d_render::pick_cut(&self.mesh, camera, width, height, x, y, section)
            .map_err(|e| e.to_string())?;
        let Some(hit) = hit else {
            return Ok((None, None));
        };
        // The point's own distance, not the picture's depth value.
        let depth = camera
            .axes()
            .map_or(hit.depth, |axes| (hit.point - axes.eye).dot(axes.forward));
        if hit.face == SECTION_FACE {
            // The body that is cut here: the one an odd number of whose
            // surfaces were cut away in front of the cap.
            let left_out =
                craft3d_render::cut_faces(&self.mesh, camera, width, height, x, y, section)
                    .map_err(|e| e.to_string())?;
            let mut odd: Vec<&SceneBody> = Vec::new();
            for face in left_out {
                if let Some((body, _)) = self.face_of(face) {
                    match odd.iter().position(|b| std::ptr::eq(*b, body)) {
                        Some(at) => {
                            odd.remove(at);
                        }
                        None => odd.push(body),
                    }
                }
            }
            let body = match odd.as_slice() {
                [body] => Some(*body),
                _ => None,
            };
            return Ok((
                None,
                Some(SceneCut {
                    feature: body.map(|body| body.feature.clone()),
                    body: body.map(|body| body.name.clone()),
                    point: hit.point,
                    depth,
                }),
            ));
        }
        let Some((body, index)) = self.face_of(hit.face) else {
            return Ok((None, None));
        };
        let pick = ScenePick {
            feature: body.feature.clone(),
            body: body.name.clone(),
            face: body.face_names.get(index).cloned(),
            aliases: body.face_aliases.get(index).cloned().unwrap_or_default(),
            face_index: index,
            face_id: hit.face,
            point: hit.point,
            depth,
        };
        Ok((Some(pick), None))
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

    /// What a position of a picture of this scene points at, by the rule
    /// at the top of this module: the face under (`x`, `y`) as
    /// [`Self::pick`] gives it, and the edge near it within `radius`
    /// pixels, if any. A radius that is not a positive number looks at
    /// faces only; one above [`MAX_PICK_RADIUS`] is that. Errors as
    /// [`Self::pick`].
    pub fn point_at(
        &self,
        camera: &Camera,
        width: u32,
        height: u32,
        x: f64,
        y: f64,
        radius: f64,
    ) -> Result<ScenePointed, String> {
        self.point_at_cut(camera, width, height, x, y, radius, None)
    }

    /// [`Self::point_at`] in a picture cut open by `section` (see the top
    /// of this module).
    #[allow(clippy::too_many_arguments)]
    pub fn point_at_cut(
        &self,
        camera: &Camera,
        width: u32,
        height: u32,
        x: f64,
        y: f64,
        radius: f64,
        section: Option<&Section>,
    ) -> Result<ScenePointed, String> {
        let (face, cap) = self.pick_cut(camera, width, height, x, y, section)?;
        let inside = x >= 0.0 && y >= 0.0 && x < f64::from(width) && y < f64::from(height);
        let radius = if radius.is_finite() {
            radius.min(MAX_PICK_RADIUS)
        } else {
            0.0
        };
        if !inside || radius <= 0.0 {
            return Ok(ScenePointed {
                face,
                section: cap,
                ..ScenePointed::default()
            });
        }
        let view = Viewpoint::new(camera, width, height).map_err(|e| e.to_string())?;
        // A point within the radius lands within the radius, give or take
        // rounding: one pixel more.
        let near = Neighbourhood::new_cut(
            &self.mesh,
            camera,
            width,
            height,
            x,
            y,
            radius + 1.0,
            section,
        )
        .map_err(|e| e.to_string())?;
        let sight = Sight {
            scene: self,
            view: &view,
            near: &near,
            size: [f64::from(width), f64::from(height)],
        };
        let vertex = self.vertex_near(&sight, x, y, radius);
        let edge = self.edge_near(&sight, x, y, radius);
        Ok(ScenePointed {
            face,
            edge,
            vertex,
            section: cap,
        })
    }

    /// The vertex in sight within `radius` pixels of (`x`, `y`): the
    /// nearest.
    fn vertex_near(&self, sight: &Sight<'_>, x: f64, y: f64, radius: f64) -> Option<SceneNear> {
        let mut candidates: Vec<Candidate> = Vec::new();
        for (body_index, body) in self.bodies.iter().enumerate() {
            for (index, &point) in body.vertices.iter().enumerate() {
                if sight.cut_away(point) {
                    continue;
                }
                let Some([sx, sy, depth]) = sight.view.project(point) else {
                    continue;
                };
                let distance = (sx - x).hypot(sy - y);
                if distance <= radius {
                    candidates.push(Candidate {
                        distance,
                        depth,
                        body: body_index,
                        index,
                        point,
                    });
                }
            }
        }
        candidates.sort_by(Candidate::order);
        let nearest = candidates.iter().find(|c| sight.sees(c.point))?;
        let body = self.bodies.get(nearest.body)?;
        let faces = body
            .vertex_faces
            .get(nearest.index)
            .into_iter()
            .flatten()
            .filter_map(|&face| body.face_names.get(face).cloned())
            .collect();
        Some(SceneNear {
            feature: body.feature.clone(),
            body: body.name.clone(),
            name: body.vertex_names.get(nearest.index).cloned(),
            index: nearest.index,
            faces,
            point: nearest.point,
            depth: nearest.depth,
            distance: nearest.distance,
        })
    }

    /// The edge with a point in sight within `radius` pixels of (`x`,
    /// `y`), and that point: the nearest such point of all edges.
    fn edge_near(&self, sight: &Sight<'_>, x: f64, y: f64, radius: f64) -> Option<SceneNear> {
        let perspective = sight.view.is_perspective();
        let mut candidates: Vec<Candidate> = Vec::new();
        'bodies: for (body_index, body) in self.bodies.iter().enumerate() {
            for (index, polyline) in body.edges.iter().enumerate() {
                for pair in polyline.windows(2) {
                    let &[q, p] = pair else {
                        continue;
                    };
                    if q == p {
                        continue;
                    }
                    // In a picture that is cut open: the part that is kept.
                    let (q, p) = match sight.near.cut() {
                        Some(cut) => match cut.clip_segment(q, p) {
                            Some([q, p]) => (q, p),
                            None => continue,
                        },
                        None => (q, p),
                    };
                    let (Some(a), Some(b)) = (sight.view.project(q), sight.view.project(p)) else {
                        continue;
                    };
                    for (distance, s) in samples_near(a, b, x, y, radius, perspective) {
                        let point = q + (p - q) * s;
                        if !point.is_finite() {
                            continue;
                        }
                        if candidates.len() >= MAX_CANDIDATES {
                            break 'bodies;
                        }
                        candidates.push(Candidate {
                            distance,
                            depth: sight.view.distance(point),
                            body: body_index,
                            index,
                            point,
                        });
                    }
                }
            }
        }
        candidates.sort_by(Candidate::order);
        let nearest = candidates.iter().find(|c| sight.sees(c.point))?;
        let body = self.bodies.get(nearest.body)?;
        let faces = body
            .edge_faces
            .get(nearest.index)
            .into_iter()
            .flatten()
            .filter_map(|&face| body.face_names.get(face).cloned())
            .collect();
        Some(SceneNear {
            feature: body.feature.clone(),
            body: body.name.clone(),
            name: body.edge_names.get(nearest.index).cloned(),
            index: nearest.index,
            faces,
            point: nearest.point,
            depth: nearest.depth,
            distance: nearest.distance,
        })
    }

    /// The places, among the faces of `body`, of the faces called
    /// `wanted`: by their own name or by an alias.
    fn faces_answering(body: &SceneBody, wanted: &str) -> Vec<usize> {
        (0..body.face_names.len())
            .filter(|&k| {
                body.face_names.get(k).is_some_and(|name| name == wanted)
                    || body
                        .face_aliases
                        .get(k)
                        .is_some_and(|aliases| aliases.iter().any(|alias| alias == wanted))
            })
            .collect()
    }

    /// The one body that has what `find` looks for: the body `feature`
    /// says (by the id of its feature, `F3` or `3`, or by its name), or
    /// the only body `find` gives anything for. `what` names the thing in
    /// an error ("edge F1:edge(2)", after "no" or "an") and `hint` says
    /// where its names are listed.
    fn body_having(
        &self,
        feature: Option<&str>,
        what: &str,
        hint: &str,
        find: impl Fn(&SceneBody) -> Vec<usize>,
    ) -> Result<(usize, Vec<usize>), String> {
        if let Some(feature) = feature {
            let shown: String = feature.chars().take(60).collect();
            let by_id = || {
                let digits = feature.strip_prefix('F').unwrap_or(feature);
                let id = digits.parse::<u64>().ok()?;
                self.bodies
                    .iter()
                    .position(|body| body.feature == format!("F{id}"))
            };
            let index = self
                .bodies
                .iter()
                .position(|body| body.name == feature)
                .or_else(by_id)
                .ok_or_else(|| {
                    format!(
                        "there is no body of a feature `{shown}`; the bodies: {}",
                        self.body_list()
                    )
                })?;
            let (found, name) = self
                .bodies
                .get(index)
                .map(|body| (find(body), body.name.as_str()))
                .unwrap_or_default();
            if found.is_empty() {
                return Err(format!("`{name}` has no {what} ({hint})"));
            }
            return Ok((index, found));
        }
        let mut having = self
            .bodies
            .iter()
            .enumerate()
            .map(|(index, body)| (index, body, find(body)))
            .filter(|(_, _, found)| !found.is_empty());
        let Some((index, body, found)) = having.next() else {
            return Err(format!("no body has {} {what} ({hint})", article(what)));
        };
        if let Some((_, other, _)) = having.next() {
            return Err(format!(
                "several bodies have {} {what} (`{}` and `{}` among them); say which with `feature`",
                article(what),
                body.name,
                other.name
            ));
        }
        Ok((index, found))
    }

    /// The edges that `edge` names (an edge's own persistent name, `name`
    /// of `document.edges`) or that lie `between` two faces (two face
    /// names from `document.inspect`, as `document.edges` takes them; the
    /// same name twice means the seams of that face and the cuts between
    /// its pieces): one or the other. `feature` says which body, as for
    /// [`Self::faces_named`]. The error is a sentence for whoever asked.
    pub fn edges_named(
        &self,
        feature: Option<&str>,
        edge: Option<&str>,
        between: &[String],
    ) -> Result<SceneEdges, String> {
        const HINT: &str = "document.edges lists the edges of a body";
        let found = |(body_index, edges): (usize, Vec<usize>),
                     edge: Option<String>,
                     between: Vec<String>| {
            let body = self.bodies.get(body_index);
            SceneEdges {
                feature: body.map(|b| b.feature.clone()).unwrap_or_default(),
                body: body.map(|b| b.name.clone()).unwrap_or_default(),
                body_index,
                edge,
                between,
                edges,
            }
        };
        match (edge, between) {
            (Some(_), [_, ..]) => Err(
                "give the edge's own name or the two faces it lies between, not both".to_string(),
            ),
            (Some(edge), []) => {
                let name: PersistentName = edge.parse()?;
                if name.is_face() {
                    return Err(format!(
                        "{name} is a face; an edge is named by `edge` (its own name) or by `between` (two faces)"
                    ));
                }
                let wanted = name.to_string();
                let at = self.body_having(feature, &format!("edge {wanted}"), HINT, |body| {
                    (0..body.edge_names.len())
                        .filter(|&k| body.edge_names.get(k) == Some(&wanted))
                        .collect()
                })?;
                Ok(found(at, Some(wanted), Vec::new()))
            }
            (None, [a, b]) => {
                let mut wanted = Vec::with_capacity(2);
                for face in [a, b] {
                    let name: PersistentName = face.parse()?;
                    if !name.is_face() {
                        return Err(format!("{name} is not a face"));
                    }
                    wanted.push(name.to_string());
                }
                let (first, second) = match wanted.as_slice() {
                    [first, second] => (first.clone(), second.clone()),
                    _ => return Err("`between` takes two face names".to_string()),
                };
                let what = format!("edge between {first} and {second}");
                let at = self.body_having(feature, &what, HINT, |body| {
                    let on_a = Self::faces_answering(body, &first);
                    let on_b = Self::faces_answering(body, &second);
                    let same = on_a == on_b;
                    body.edge_faces
                        .iter()
                        .enumerate()
                        .filter(|(_, users)| {
                            if same {
                                !users.is_empty() && users.iter().all(|face| on_a.contains(face))
                            } else {
                                users.iter().any(|face| on_a.contains(face))
                                    && users.iter().any(|face| on_b.contains(face))
                            }
                        })
                        .map(|(index, _)| index)
                        .collect()
                })?;
                Ok(found(at, None, wanted))
            }
            (None, []) => Err(
                "an edge is named by `edge` (its own name) or by `between` (two faces)".to_string(),
            ),
            (None, _) => Err("`between` takes two face names".to_string()),
        }
    }

    /// The vertices that `vertex` names (a vertex's own persistent name,
    /// `name` under `vertices` of `document.edges`) or in which the faces
    /// of `corner` all meet (one to [`MAX_CORNER_FACES`] face names): one
    /// or the other. `feature` says which body, as for
    /// [`Self::faces_named`]. The error is a sentence for whoever asked.
    pub fn vertices_named(
        &self,
        feature: Option<&str>,
        vertex: Option<&str>,
        corner: &[String],
    ) -> Result<SceneVertices, String> {
        const HINT: &str = "document.edges lists the vertices of a body";
        let found = |(body_index, vertices): (usize, Vec<usize>),
                     vertex: Option<String>,
                     corner: Vec<String>| {
            let body = self.bodies.get(body_index);
            SceneVertices {
                feature: body.map(|b| b.feature.clone()).unwrap_or_default(),
                body: body.map(|b| b.name.clone()).unwrap_or_default(),
                body_index,
                vertex,
                corner,
                vertices,
            }
        };
        match (vertex, corner) {
            (Some(_), [_, ..]) => {
                Err("give the vertex's own name or the faces that meet in it, not both"
                    .to_string())
            }
            (Some(vertex), []) => {
                let name: PersistentName = vertex.parse()?;
                if name.is_face() {
                    return Err(format!(
                        "{name} is a face; a vertex is named by `vertex` (its own name) or by `corner` (the faces that meet in it)"
                    ));
                }
                let wanted = name.to_string();
                let at = self
                    .body_having(feature, &format!("vertex {wanted}"), HINT, |body| {
                        (0..body.vertex_names.len())
                            .filter(|&k| body.vertex_names.get(k) == Some(&wanted))
                            .collect()
                    })?;
                Ok(found(at, Some(wanted), Vec::new()))
            }
            (None, []) => Err(
                "a vertex is named by `vertex` (its own name) or by `corner` (the faces that meet in it)"
                    .to_string(),
            ),
            (None, faces) => {
                if faces.len() > MAX_CORNER_FACES {
                    return Err(format!(
                        "`corner` takes at most {MAX_CORNER_FACES} face names"
                    ));
                }
                let mut wanted = Vec::with_capacity(faces.len());
                for face in faces {
                    let name: PersistentName = face.parse()?;
                    if !name.is_face() {
                        return Err(format!("{name} is not a face"));
                    }
                    wanted.push(name.to_string());
                }
                let shown: Vec<&str> = wanted.iter().take(3).map(String::as_str).collect();
                let more = if wanted.len() > 3 { " and more" } else { "" };
                let what = format!("vertex where {}{more} meet", shown.join(", "));
                let at = self
                    .body_having(feature, &what, HINT, |body| {
                        let sets: Vec<Vec<usize>> = wanted
                            .iter()
                            .map(|name| Self::faces_answering(body, name))
                            .collect();
                        body.vertex_faces
                            .iter()
                            .enumerate()
                            .filter(|(_, meeting)| {
                                sets.iter()
                                    .all(|set| meeting.iter().any(|face| set.contains(face)))
                            })
                            .map(|(index, _)| index)
                            .collect()
                    })?;
                Ok(found(at, None, wanted))
            }
        }
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

/// "a" or "an" before `what`.
fn article(what: &str) -> &'static str {
    if what.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

/// A point of an edge near a position, before it is known to be in sight.
struct Candidate {
    distance: f64,
    depth: f64,
    body: usize,
    index: usize,
    point: Vec3,
}

impl Candidate {
    /// Nearest in the picture first, then nearest to the eye, then the
    /// order of the scene: a total order, so the winner does not depend on
    /// anything else.
    fn order(a: &Candidate, b: &Candidate) -> std::cmp::Ordering {
        a.distance
            .total_cmp(&b.distance)
            .then(a.depth.total_cmp(&b.depth))
            .then(a.body.cmp(&b.body))
            .then(a.index.cmp(&b.index))
    }
}

/// The points of the straight line in the picture from `a` to `b` (the
/// projections, as x, y and distance, of the ends of a segment in space)
/// that are within `radius` pixels of (`x`, `y`): the nearest one, and
/// from it every [`SAMPLE_STEP`] pixels along the line both ways. Each as
/// its distance from the position, in pixels, and the fraction of the
/// segment in space it is at. Nothing when the line is further away.
fn samples_near(
    a: [f64; 3],
    b: [f64; 3],
    x: f64,
    y: f64,
    radius: f64,
    perspective: bool,
) -> Vec<(f64, f64)> {
    let [ax, ay, da] = a;
    let [bx, by, db] = b;
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx.hypot(dy);
    let at = |t: f64| -> Option<(f64, f64)> {
        let distance = (x - (ax + dx * t)).hypot(y - (ay + dy * t));
        // Along a line in the picture of a perspective camera it is the
        // reciprocal of the distance that changes evenly: the fraction in
        // space follows from it.
        let along = db - t * (db - da);
        let s = if perspective && along > 0.0 {
            t * da / along
        } else {
            t
        };
        (distance <= radius && s.is_finite()).then_some((distance, s.clamp(0.0, 1.0)))
    };
    // Negated, so that a length that is not a number is no length.
    if !(length > 0.0 && length.is_finite()) {
        return at(0.0).into_iter().collect();
    }
    let nearest = (((x - ax) * dx + (y - ay) * dy) / (length * length)).clamp(0.0, 1.0);
    let Some(first) = at(nearest) else {
        return Vec::new();
    };
    let mut out = vec![first];
    let step = SAMPLE_STEP / length;
    // The part of a line within a circle is at most its diameter long.
    let most = (2.0 * radius / SAMPLE_STEP).ceil().clamp(0.0, 1024.0) as usize;
    for direction in [-1.0, 1.0] {
        for k in 1..=most {
            let t = nearest + direction * step * k as f64;
            if !(0.0..=1.0).contains(&t) {
                break;
            }
            match at(t) {
                Some(sample) => out.push(sample),
                None => break,
            }
        }
    }
    out
}

/// The picture a position is looked up in.
struct Sight<'a> {
    scene: &'a Scene,
    view: &'a Viewpoint,
    /// The triangles near the position.
    near: &'a Neighbourhood,
    size: [f64; 2],
}

impl Sight<'_> {
    /// True when the picture is cut open and `point` is in the half that
    /// is cut away: a line is cut where it passes through the plane.
    fn cut_away(&self, point: Vec3) -> bool {
        self.near.cut().is_some_and(|cut| cut.side(point) > 0.0)
    }

    /// True when `point` is in the picture and no face is in front of it:
    /// the surface the picture shows at the place where the point lands is
    /// not nearer than the point is.
    fn sees(&self, point: Vec3) -> bool {
        let Some([x, y, depth]) = self.view.project(point) else {
            return false;
        };
        let [width, height] = self.size;
        if !(x >= 0.0 && y >= 0.0 && x < width && y < height) {
            return false;
        }
        let Some(hit) = self.near.pick(x, y) else {
            return true;
        };
        let slack = SIGHT_SLACK * (depth.abs() + self.scene.bbox.diagonal());
        let slack = if slack.is_finite() { slack } else { 0.0 };
        self.view.distance(hit.point) + slack >= depth
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
            "radius",
            "view",
            "projection",
            "zoom",
            "box",
            "camera",
            "section",
            "tolerance",
        ],
    )?;
    let x = p.number("x")?.ok_or_else(|| p.missing("x"))?;
    let y = p.number("y")?.ok_or_else(|| p.missing("y"))?;
    let width = p.integer("width", 16, 4096)?.unwrap_or(800);
    let height = p.integer("height", 16, 4096)?.unwrap_or(600);
    let radius = match p.number("radius")? {
        None => DEFAULT_PICK_RADIUS,
        Some(radius) if (0.0..=MAX_PICK_RADIUS).contains(&radius) => radius,
        Some(_) => {
            return fail(format!(
                "{ID}: `radius` must be a number of pixels from 0 to {MAX_PICK_RADIUS}"
            ));
        }
    };
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
    // Both are within 16..=4096.
    let (w, h) = (width as u32, height as u32);
    // The scene view.render draws for the same picture.
    let pixel = |model: &craft3d_math::Bbox3| {
        let camera = match given {
            Some(camera) => camera,
            None => framing.camera(model, width as f64 / height as f64).ok()?,
        };
        crate::scene::pixel_footprint(&camera, h, model)
    };
    let Some(scene) = session.picture_scene(tolerance, pixel)? else {
        return fail(format!(
            "{ID}: no document is open; run file.new or file.open first"
        ));
    };
    let camera = match given {
        Some(camera) => camera,
        // The picture of view.render: framed on the bodies' B-rep.
        None => framing.camera(&scene.bodies_box, width as f64 / height as f64)?,
    };
    let section = match p.get("section") {
        None | Some(Value::Null) => None,
        Some(value) => Some(crate::section::resolve_param(session, &scene, ID, value)?),
    };
    let outside = !(x >= 0.0 && y >= 0.0 && x < width as f64 && y < height as f64);
    let pointed = scene
        .point_at_cut(&camera, w, h, x, y, radius, section.as_ref())
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let mut result = json!({
        "x": x,
        "y": y,
        "width": width,
        "height": height,
        "radius": radius,
        "camera": camera_json(&camera),
        "tolerance": scene.tolerance,
    });
    let Some(map) = result.as_object_mut() else {
        return Ok(result);
    };
    pointed.describe(map);
    if pointed.kind().is_none() {
        map.insert("outside".into(), outside.into());
    }
    Ok(result)
}
