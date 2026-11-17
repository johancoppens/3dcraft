//! The section plane of a view: where a way of saying it (a base plane, a
//! datum plane, a flat face by its name, or a point and a normal, moved by
//! an offset) lies in space.
//!
//! A section is a view, not a feature: nothing here changes a document.
//! `view.render` and `view.pick` take one as a parameter and the window
//! keeps one as UI state; both resolve it here, against the scene they
//! draw, so that a name means the same plane in both. The renderers
//! define what the plane does to a picture (`craft3d_render::render`,
//! *Section*).

use crate::commands::fail;
use crate::datum_cmds::plane_value;
use crate::params::Params;
use crate::scene::{Scene, SceneDatumShape};
use crate::{CommandError, Session};
use craft3d_doc::{PlaneRef, WorldPlane};
use craft3d_math::{Bbox3, Vec3};
use craft3d_regen::Datum;
use craft3d_render::Section;
use serde_json::Value;
use std::sync::Arc;

/// The colour of a section's cap in the pictures the engine makes
/// (`view.render`); a shell has its own in its theme.
pub const SECTION_CAP_COLOR: [u8; 3] = [214, 132, 74];
/// The largest coordinate of a section's point and the largest offset, in
/// model units.
pub const MAX_SECTION_EXTENT: f64 = 1e12;

/// How a section plane is given.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SectionSpec {
    /// A base plane (`xy`, `yz`, `zx`), a datum plane by its feature's id
    /// (`F5`) or name, or a flat face by its persistent name. A base
    /// plane's name wins over a feature of the same name.
    pub plane: Option<String>,
    /// Which body has the face, by the id of its feature or its name;
    /// needed only when several bodies have a face of that name.
    pub feature: Option<String>,
    /// A plane given directly, in place of `plane`: a point of it and its
    /// normal.
    pub point: Option<[f64; 3]>,
    pub normal: Option<[f64; 3]>,
    /// How far the plane is moved along its normal, in model units.
    pub offset: f64,
    /// Keep the other half: what lies on the side the normal points to is
    /// cut away, and with `flip` what lies on the other side.
    pub flip: bool,
}

/// A section plane in space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionPlane {
    /// A point of the plane, moved by the offset.
    pub origin: Vec3,
    /// Unit length, into the half that is cut away.
    pub normal: Vec3,
    /// The point the offset is measured from, and the unit direction it is
    /// measured along (the normal as it was given, whatever `flip` says).
    pub base: Vec3,
    pub along: Vec3,
}

impl SectionPlane {
    /// The section for a renderer, with a cap in `color`. The bodies of a
    /// document are closed solids, so the plane through one has a cap.
    pub fn section(&self, color: [u8; 3]) -> Section {
        Section {
            origin: self.origin,
            normal: self.normal,
            cap: true,
            color,
        }
    }

    /// The offsets between which the plane passes through `bbox`: least
    /// and greatest. `None` for an empty box.
    pub fn range(&self, bbox: &Bbox3) -> Option<[f64; 2]> {
        let (lo, hi) = bbox.corners()?;
        let (mut least, mut greatest) = (f64::INFINITY, f64::NEG_INFINITY);
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    let at = (Vec3::new(x, y, z) - self.base).dot(self.along);
                    least = least.min(at);
                    greatest = greatest.max(at);
                }
            }
        }
        (least.is_finite() && greatest.is_finite()).then_some([least, greatest])
    }

    /// The same plane moved to `offset` and keeping the half `flip` says:
    /// what [`Session::section_plane`] gives for a spec that differs from
    /// this plane's only there, without resolving its name again. An
    /// offset that is not a number within ±[`MAX_SECTION_EXTENT`] is an
    /// error, as there.
    pub fn at(&self, offset: f64, flip: bool) -> Result<SectionPlane, String> {
        if !(offset.is_finite() && offset.abs() <= MAX_SECTION_EXTENT) {
            return Err(format!(
                "the section's `offset` must be a number within ±{MAX_SECTION_EXTENT:e}"
            ));
        }
        Ok(placed(self.base, self.along, offset, flip))
    }

    /// The offsets at which the plane passes through a place the model
    /// gives in `scene`: a flat face parallel to it, or the axis of a
    /// circular edge (a hole, a boss, a round) that lies parallel to it.
    /// Found on the scene's mesh and edge lines, whose points lie on the
    /// exact faces and curves, in one pass over each; sorted by offset,
    /// one per offset (an axis before a face there), at most
    /// [`MAX_SECTION_SNAPS`]. What a drag of the section's handle snaps to.
    pub fn snaps(&self, scene: &Scene) -> Vec<SectionSnap> {
        let size = scene.bodies_box.diagonal().max(scene.bbox.diagonal());
        if !(size.is_finite() && size > 0.0 && self.along.is_finite() && self.base.is_finite()) {
            return Vec::new();
        }
        // Rounding at the model's size: a face this flat is flat, a circle
        // this round is round.
        let flat = size * SNAP_ROUNDING;
        let mut out = face_snaps(self, scene, flat);
        for body in scene.bodies.iter() {
            for (e, line) in body.edges.iter().enumerate() {
                let Some((center, axis)) = circle_of(line, flat) else {
                    continue;
                };
                if axis.dot(self.along).abs() > SNAP_SQUARE {
                    continue;
                }
                let offset = (center - self.base).dot(self.along);
                if offset.is_finite() {
                    out.push(SectionSnap {
                        offset,
                        kind: SnapKind::Axis,
                        feature: body.feature.clone(),
                        name: body.edge_names.get(e).cloned(),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.offset.total_cmp(&b.offset).then(a.kind.cmp(&b.kind)));
        let mut kept: Vec<SectionSnap> = Vec::with_capacity(out.len().min(MAX_SECTION_SNAPS));
        for snap in out {
            if kept
                .last()
                .is_some_and(|last| (snap.offset - last.offset).abs() <= flat)
            {
                continue;
            }
            if kept.len() == MAX_SECTION_SNAPS {
                break;
            }
            kept.push(snap);
        }
        kept
    }
}

/// Most places a section snaps to ([`SectionPlane::snaps`]).
pub const MAX_SECTION_SNAPS: usize = 1024;
/// How flat a face and how round a circle must be to be a place a section
/// snaps to, as a share of the model's size: rounding, nothing more.
const SNAP_ROUNDING: f64 = 1e-7;
/// How nearly square to a section's normal a circle's axis must be to lie
/// parallel to the plane (the cosine of the angle between them), and how
/// nearly along it a flat face's normal (one less the cosine).
const SNAP_SQUARE: f64 = 1e-6;
const SNAP_ALONG: f64 = 1e-9;
/// Most faces a mesh may number for [`SectionPlane::snaps`] to look at
/// them: more is no mesh the scene makes.
const MAX_SNAP_FACES: usize = 1 << 24;

/// What a place a section snaps to is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapKind {
    /// The axis of a circular edge parallel to the plane: the middle of a
    /// hole, a boss or a round.
    Axis,
    /// A flat face parallel to the plane.
    Face,
}

impl SnapKind {
    /// As `ui.get` says it.
    pub fn name(self) -> &'static str {
        match self {
            SnapKind::Axis => "axis",
            SnapKind::Face => "face",
        }
    }
}

/// A place a section snaps to ([`SectionPlane::snaps`]).
#[derive(Clone, Debug, PartialEq)]
pub struct SectionSnap {
    /// The offset at which the plane passes through it.
    pub offset: f64,
    pub kind: SnapKind,
    /// The feature of the body that has it (`F3`), and the face's or the
    /// edge's persistent name where it has one.
    pub feature: String,
    pub name: Option<String>,
}

/// The plane through `base` with unit normal `along`, moved by `offset`
/// and keeping the half `flip` says.
fn placed(base: Vec3, along: Vec3, offset: f64, flip: bool) -> SectionPlane {
    SectionPlane {
        origin: base + along * offset,
        // Subtracted from zero, so that no component is a negative zero.
        normal: if flip { Vec3::ZERO - along } else { along },
        base,
        along,
    }
}

/// The flat faces of `scene` parallel to `plane`, as places it snaps to:
/// a face whose corners all lie at one offset within `flat`.
fn face_snaps(plane: &SectionPlane, scene: &Scene, flat: f64) -> Vec<SectionSnap> {
    let mesh = &scene.mesh;
    let faces = mesh
        .face_ids
        .iter()
        .max()
        .map_or(0, |&f| f as usize + 1)
        .min(MAX_SNAP_FACES);
    // Per face: its normal times twice its area, and its corners' least
    // and greatest offset.
    let mut seen: Vec<Option<(Vec3, f64, f64)>> = vec![None; faces];
    for (t, &face) in mesh.face_ids.iter().enumerate() {
        let (Some(slot), Some([a, b, c])) = (seen.get_mut(face as usize), mesh.corners(t)) else {
            continue;
        };
        let [sa, sb, sc] = [a, b, c].map(|p| (p - plane.base).dot(plane.along));
        let (lo, hi) = (sa.min(sb).min(sc), sa.max(sb).max(sc));
        let normal = (b - a).cross(c - a);
        *slot = Some(match *slot {
            None => (normal, lo, hi),
            Some((n, l, h)) => (n + normal, l.min(lo), h.max(hi)),
        });
    }
    let mut out = Vec::new();
    for (face, found) in seen.into_iter().enumerate() {
        let Some((normal, lo, hi)) = found else {
            continue;
        };
        let Some(normal) = normal.normalized() else {
            continue;
        };
        if !(hi - lo <= flat && 1.0 - normal.dot(plane.along).abs() <= SNAP_ALONG) {
            continue;
        }
        // The body whose faces start at or before this one.
        let Ok(face) = u32::try_from(face) else {
            continue;
        };
        let at = scene
            .bodies
            .partition_point(|body| body.first_face_id <= face);
        let Some(body) = at.checked_sub(1).and_then(|k| scene.bodies.get(k)) else {
            continue;
        };
        let name = face
            .checked_sub(body.first_face_id)
            .and_then(|k| body.face_names.get(k as usize))
            .cloned();
        out.push(SectionSnap {
            offset: (lo + hi) * 0.5,
            kind: SnapKind::Face,
            feature: body.feature.clone(),
            name,
        });
    }
    out
}

/// The centre and the unit axis of the circle the points of `line` lie
/// on, each within `tolerance`: `None` for fewer than three points, points
/// on a straight line, or points that are not on one circle. The circle
/// is the one through the first point and those a third and two thirds
/// of the way along the line.
fn circle_of(line: &[Vec3], tolerance: f64) -> Option<(Vec3, Vec3)> {
    let n = line.len();
    if n < 3 {
        return None;
    }
    let (p0, p1, p2) = (*line.first()?, *line.get(n / 3)?, *line.get(2 * n / 3)?);
    let (a, b) = (p1 - p0, p2 - p0);
    let axb = a.cross(b);
    let area = axb.length_squared();
    // Three points on a line (or two of them the same) make no circle.
    if !(area > 1e-24 * a.length_squared() * b.length_squared() && area.is_finite()) {
        return None;
    }
    let center =
        p0 + (b.cross(axb) * a.length_squared() + axb.cross(a) * b.length_squared()) * (0.5 / area);
    let axis = axb.normalized()?;
    let radius = center.distance(p0);
    if !(center.is_finite() && radius.is_finite()) {
        return None;
    }
    let on_it = line.iter().all(|&p| {
        (p.distance(center) - radius).abs() <= tolerance
            && (p - center).dot(axis).abs() <= tolerance
    });
    on_it.then_some((center, axis))
}

fn finite3(what: &str, v: [f64; 3]) -> Result<Vec3, String> {
    if v.iter()
        .all(|c| c.is_finite() && c.abs() <= MAX_SECTION_EXTENT)
    {
        Ok(Vec3::new(v[0], v[1], v[2]))
    } else {
        Err(format!(
            "the section's `{what}` must be three numbers within ±{MAX_SECTION_EXTENT:e}"
        ))
    }
}

/// The plane of the faces `ids` of `scene`'s mesh, when they are flat: a
/// point of it and its unit normal, which points out of the body (the
/// triangles of a body are wound that way).
fn face_plane(scene: &Scene, ids: &[u32], name: &str) -> Result<(Vec3, Vec3), String> {
    let mesh = &scene.mesh;
    let mut normal = Vec3::ZERO;
    let mut corners_of_face: Vec<Vec3> = Vec::new();
    for (t, face) in mesh.face_ids.iter().enumerate() {
        if !ids.contains(face) {
            continue;
        }
        let Some([a, b, c]) = mesh.corners(t) else {
            continue;
        };
        // Twice the triangle's area, along its normal.
        normal += (b - a).cross(c - a);
        corners_of_face.extend([a, b, c]);
    }
    let not_flat = || {
        format!(
            "face {name} is not flat; a section lies on a base plane (xy, yz, zx), a datum plane or a flat face"
        )
    };
    let Some(&origin) = corners_of_face.first() else {
        return Err(format!(
            "face {name} has no area in the picture's mesh, so it gives no plane"
        ));
    };
    // Normals that cancel (the whole wall of a hole) are no plane's.
    let Some(normal) = normal.normalized() else {
        return Err(not_flat());
    };
    // Flat: every corner in the plane, within rounding of the face's size.
    let extent = Bbox3::from_points(corners_of_face.iter().copied()).diagonal();
    let off = corners_of_face
        .iter()
        .map(|&p| (p - origin).dot(normal).abs())
        .fold(0.0, f64::max);
    if off.is_nan() || off > extent * 1e-6 {
        return Err(not_flat());
    }
    Ok((origin, normal))
}

impl Session {
    /// Where the section `spec` lies, for a picture of `scene` (the scene
    /// of the active document). The error is a sentence for whoever asked:
    /// a name that means no plane, a face that is not flat, a normal
    /// without direction, a number that is not finite.
    pub fn section_plane(
        &mut self,
        scene: &Scene,
        spec: &SectionSpec,
    ) -> Result<SectionPlane, String> {
        if !(spec.offset.is_finite() && spec.offset.abs() <= MAX_SECTION_EXTENT) {
            return Err(format!(
                "the section's `offset` must be a number within ±{MAX_SECTION_EXTENT:e}"
            ));
        }
        let (base, along) = match (&spec.plane, spec.point, spec.normal) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err(
                    "a section is given by `plane` or by `point` and `normal`, not both"
                        .to_string(),
                );
            }
            (None, Some(point), Some(normal)) => {
                let normal = finite3("normal", normal)?.normalized().ok_or_else(|| {
                    "the section's `normal` has no direction; give a vector that is not zero"
                        .to_string()
                })?;
                (finite3("point", point)?, normal)
            }
            (None, _, _) => {
                return Err(
                    "a section needs `plane` (xy, yz, zx, a datum plane or a flat face) or `point` and `normal`"
                        .to_string(),
                );
            }
            (Some(plane), None, None) => self.named_plane(scene, plane, spec.feature.as_deref())?,
        };
        if !(base.is_finite() && along.is_finite()) {
            return Err("the section's plane is not finite".to_string());
        }
        Ok(placed(base, along, spec.offset, spec.flip))
    }

    /// The plane `text` names: a point of it and its unit normal.
    fn named_plane(
        &mut self,
        scene: &Scene,
        text: &str,
        feature: Option<&str>,
    ) -> Result<(Vec3, Vec3), String> {
        // Regenerated elsewhere and not here: a datum plane is taken
        // from the scene, which carries it as regeneration placed it
        // (bit for bit), so that a frame never regenerates for it. Not
        // while the view is scrubbed: the scene is then of another state,
        // and the section lies on the plane of the document as it is.
        let from_scene = self.regen_elsewhere() && !self.regenerated_here();
        // While the view is scrubbed, the scene drawn is of another
        // state, and the section lies on the plane of the document as it
        // is: taken from a scene of that state, which is kept while the
        // view is scrubbed (or built beside the page first when it is
        // not, `Session::scene_job`).
        let open = self.document_mut().map_err(|e| e.message().to_string())?;
        let current = Arc::clone(open.history.current());
        let of_current = if from_scene && open.scrubbed_to().is_some() {
            Some(self.kept_scene_of(&current))
        } else {
            None
        };
        let open = self.document_mut().map_err(|e| e.message().to_string())?;
        let reference = plane_value(
            open.history.current(),
            "section",
            "plane",
            &Value::String(text.to_string()),
        )
        .map_err(|e| {
            let message = e.message();
            message
                .strip_prefix("section: ")
                .unwrap_or(message)
                .to_string()
        })?;
        match reference {
            PlaneRef::World { plane } => Ok((
                Vec3::ZERO,
                match plane {
                    WorldPlane::Xy => Vec3::Z,
                    WorldPlane::Yz => Vec3::X,
                    WorldPlane::Zx => Vec3::Y,
                },
            )),
            PlaneRef::Datum { feature: id } => {
                let name = open
                    .history
                    .current()
                    .feature(id)
                    .map_or_else(|| id.to_string(), |f| f.name.clone());
                let not_placed = || {
                    format!(
                        "the datum plane `{name}` is not placed (its feature failed or is suppressed), so it gives no section"
                    )
                };
                if from_scene {
                    let id = id.to_string();
                    let current_scene;
                    let scene = match of_current {
                        None => scene,
                        Some(None) => {
                            return Err(format!(
                                "the datum plane `{name}` is being placed beside the page; the section follows"
                            ));
                        }
                        Some(Some(Err(why))) => return Err(why),
                        Some(Some(Ok(kept))) => {
                            current_scene = kept;
                            &*current_scene
                        }
                    };
                    return scene
                        .datums
                        .iter()
                        .find_map(|datum| match datum.shape {
                            SceneDatumShape::Plane { origin, normal, .. }
                                if datum.feature == id =>
                            {
                                Some((origin, normal))
                            }
                            _ => None,
                        })
                        .ok_or_else(not_placed);
                }
                let regen = open.regen();
                match regen.feature(id).and_then(|result| result.datum.as_ref()) {
                    Some(Datum::Plane(frame)) => Ok((frame.origin(), frame.z())),
                    _ => Err(not_placed()),
                }
            }
            PlaneRef::Face { face } => {
                let name = face.to_string();
                let found = scene.faces_named(feature, &name)?;
                face_plane(scene, &found.ids, &name)
            }
        }
    }
}

/// The `section` of `view.render` and `view.pick`: an object as
/// [`SectionSpec`], with `cap` besides.
pub(crate) fn section_param(
    command: &'static str,
    value: &Value,
) -> Result<(SectionSpec, bool), CommandError> {
    let p = Params::new(
        command,
        value,
        &[
            "plane", "feature", "point", "normal", "offset", "flip", "cap",
        ],
    )
    .map_err(|e| CommandError::new(format!("{} (of `section`)", e.message())))?;
    let triple = |key: &str| -> Result<Option<[f64; 3]>, CommandError> {
        let Some(value) = p.get(key) else {
            return Ok(None);
        };
        let number = |v: &Value| v.as_f64().filter(|n| n.is_finite());
        let parsed = match value.as_array().map(Vec::as_slice) {
            Some([x, y, z]) => (|| Some([number(x)?, number(y)?, number(z)?]))(),
            _ => None,
        };
        parsed.map(Some).ok_or_else(|| {
            CommandError::new(format!(
                "{command}: `section.{key}` must be [x, y, z], three finite numbers"
            ))
        })
    };
    let plane = match p.get("plane") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        // A datum plane's id may be given as a number, as the commands
        // take it.
        Some(Value::Number(id)) if id.is_u64() => Some(format!("F{id}")),
        Some(_) => {
            return fail(format!(
                "{command}: `section.plane` must be xy, yz or zx, a datum plane or a face name"
            ));
        }
    };
    let feature = match p.get("feature") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(id)) if id.is_u64() => Some(id.to_string()),
        Some(_) => {
            return fail(format!(
                "{command}: `section.feature` must be a feature id or name"
            ));
        }
    };
    Ok((
        SectionSpec {
            plane,
            feature,
            point: triple("point")?,
            normal: triple("normal")?,
            offset: p.number("offset")?.unwrap_or(0.0),
            flip: p.bool("flip")?.unwrap_or(false),
        },
        p.bool("cap")?.unwrap_or(true),
    ))
}

/// The section a command was given, resolved against `scene`.
pub(crate) fn resolve_param(
    session: &mut Session,
    scene: &Scene,
    command: &'static str,
    value: &Value,
) -> Result<Section, CommandError> {
    let (spec, cap) = section_param(command, value)?;
    let plane = session
        .section_plane(scene, &spec)
        .map_err(|e| CommandError::new(format!("{command}: {e}")))?;
    Ok(Section {
        cap,
        ..plane.section(SECTION_CAP_COLOR)
    })
}
