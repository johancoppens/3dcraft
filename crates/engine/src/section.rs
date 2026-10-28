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
    let (Some(&origin), Some(normal)) = (corners_of_face.first(), normal.normalized()) else {
        return Err(format!(
            "face {name} has no area in the picture's mesh, so it gives no plane"
        ));
    };
    // Flat: every corner in the plane, within rounding of the face's size.
    let extent = Bbox3::from_points(corners_of_face.iter().copied()).diagonal();
    let off = corners_of_face
        .iter()
        .map(|&p| (p - origin).dot(normal).abs())
        .fold(0.0, f64::max);
    if off.is_nan() || off > extent * 1e-6 {
        return Err(format!(
            "face {name} is not flat; a section lies on a base plane (xy, yz, zx), a datum plane or a flat face"
        ));
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
        let origin = base + along * spec.offset;
        Ok(SectionPlane {
            origin,
            // Subtracted from zero, so that no component is a negative zero.
            normal: if spec.flip { Vec3::ZERO - along } else { along },
            base,
            along,
        })
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
        let open = self.document_mut().map_err(|e| e.message().to_string())?;
        let from_scene = from_scene && open.scrubbed_to().is_none();
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
