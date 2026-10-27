//! Sheet metal in regeneration: the base flat (the one closed profile of
//! a sketch), its flanges and reliefs, folded and plated by the kernel
//! (`craft3d_kernel::sheetmetal`, docs/design/sheetmetal.md).
//!
//! **Names.** The folded bottom skin's faces are `F9:face(0)` (the base),
//! `F9:face(2i + 1)` (the bend of flange `i`) and `F9:face(2i + 2)` (flange
//! `i`'s flat); the part's faces are a plate's of that skin: the top skin
//! of a face `F9:copy(0)/F9:face(k)`, the bottom skin `F9:copy(1)/…`, the
//! rim along an edge `F9:rim(h)/F9:face(k)` (`regen::plate`).

use crate::extrude::{drawn, profile_error};
use crate::{FeatureResult, Names, angle, face_ops, length, number};
use craft3d_doc::{
    BendDirection, Document, FeatureId, FeatureKind, FlangeFrom, FlangeOn, NameRole,
    PersistentName, ReliefShape, SheetMetal,
};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::profile::detect_profiles;
use craft3d_kernel::sheetmetal::{
    self, Direction, FlangeSpec, LengthFrom, On, Part, Relief, ReliefKind, SheetFace, Spec,
};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, FaceId};
use std::sync::Arc;

/// A sheet-metal part as regeneration built it, for the commands that
/// read it (the bend table, the measures, the flat pattern's export).
#[derive(Clone, Debug, PartialEq)]
pub struct SheetMetalPart {
    pub part: Part,
    pub tolerance: Tolerance,
    /// The curve ids of each hole of the base, in the kernel's order.
    pub holes: Vec<Vec<u64>>,
    /// The base's sketch, for messages.
    pub sketch: String,
}

/// The name of a face of the folded bottom skin.
pub fn sheet_face_name(feature: FeatureId, face: SheetFace) -> PersistentName {
    let number = match face {
        SheetFace::Flat(0) => 0,
        SheetFace::Flat(i) => 2 * i as u64,
        SheetFace::Bend(i) => 2 * i as u64 + 1,
    };
    PersistentName::new(feature, NameRole::Face, vec![number])
}

fn explain(error: KernelError) -> String {
    match error {
        KernelError::SheetMetal {
            flange: Some(i),
            reason,
        } => format!("flanges.{i}: {reason}"),
        KernelError::SheetMetal {
            flange: None,
            reason,
        } => reason,
        KernelError::InvalidSize { name, value } => format!("{name}: {value} is not a size"),
        other => other.to_string(),
    }
}

/// Reads and builds the part.
pub(crate) fn analyse(
    doc: &Document,
    variables: &Evaluation,
    made: &SheetMetal,
    earlier: &[FeatureResult],
) -> Result<SheetMetalPart, String> {
    let drawn = drawn(doc, earlier, made.sketch)?;
    let name = drawn.name.to_string();
    let tol = drawn.tolerance;
    if drawn.curves.is_empty() {
        return Err(format!(
            "sketch `{name}` has nothing to bend: draw the base as one closed profile"
        ));
    }
    let mut profiles = detect_profiles(&drawn.curves, &tol).map_err(|e| {
        format!(
            "sketch `{name}`: {}",
            profile_error(&e, &drawn.solved.curve_ids)
        )
    })?;
    if profiles.len() != 1 {
        return Err(format!(
            "sketch `{name}` has {} closed profiles; the base of a sheet-metal part is one (holes inside it allowed)",
            profiles.len()
        ));
    }
    let base = profiles.remove(0);
    let holes = base
        .holes
        .iter()
        .map(|h| h.segments.iter().map(|s| s.source).collect())
        .collect();
    let thickness = length(doc, variables, "thickness", &made.thickness)?;
    let radius = length(doc, variables, "radius", &made.radius)?;
    let k = number(variables, "kFactor", &made.k_factor)?;
    let mut flanges = Vec::with_capacity(made.flanges.len());
    for (i, f) in made.flanges.iter().enumerate() {
        let param = |what: &str| format!("flanges.{i}.{what}");
        let on = match f.on {
            FlangeOn::Base(line) => {
                if drawn.sketch.entity(line).is_err() {
                    return Err(format!("{}: sketch `{name}` has no {line}", param("on")));
                }
                On::Base(line.0)
            }
            FlangeOn::Tip(j) => On::Tip(usize::try_from(j).unwrap_or(usize::MAX)),
        };
        flanges.push(FlangeSpec {
            on,
            length: length(doc, variables, &param("length"), &f.length)?,
            angle: angle(doc, variables, &param("angle"), &f.angle)?,
            direction: match f.direction {
                BendDirection::Up => Direction::Up,
                BendDirection::Down => Direction::Down,
            },
            radius: match &f.radius {
                Some(expr) => length(doc, variables, &param("radius"), expr)?,
                None => radius,
            },
            k: match &f.k_factor {
                Some(expr) => number(variables, &param("kFactor"), expr)?,
                None => k,
            },
            from: match f.from {
                FlangeFrom::Tangent => LengthFrom::Tangent,
                FlangeFrom::Mould => LengthFrom::Mould,
            },
        });
    }
    let relief = match &made.relief {
        None => None,
        Some(r) => Some(Relief {
            kind: match r.shape {
                ReliefShape::Rect => ReliefKind::Rect,
                ReliefShape::Round => ReliefKind::Round,
                ReliefShape::Tear => ReliefKind::Tear,
            },
            size: length(doc, variables, "relief.size", &r.size)?,
        }),
    };
    let spec = Spec {
        base,
        plane: drawn.plane,
        thickness,
        flanges,
        relief,
    };
    let part = sheetmetal::build(&spec, &tol).map_err(explain)?;
    Ok(SheetMetalPart {
        part,
        tolerance: tol,
        holes,
        sketch: name,
    })
}

/// Builds a sheet-metal part: the solid and its names.
pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    id: FeatureId,
    made: &SheetMetal,
    earlier: &[FeatureResult],
) -> Result<(Body, Names, Arc<SheetMetalPart>), String> {
    let built = analyse(doc, variables, made, earlier)?;
    let (part, tolerance) = (&built.part, built.tolerance);
    let mut sheet_names = Names::default();
    for face in &part.sheet_faces {
        sheet_names.faces.push(sheet_face_name(id, *face));
    }
    let none = vec![None; part.sheet.edges.len()];
    let no_vertices = vec![None; part.sheet.vertices.len()];
    sheet_names.name_entities(
        id,
        &part.sheet,
        tolerance.linear,
        &none,
        &no_vertices,
        |_| None,
    )?;
    let picked: Vec<FaceId> = (0..part.sheet.faces.len())
        .filter_map(|f| u32::try_from(f).ok().map(FaceId))
        .collect();
    let made_by = doc
        .feature(id)
        .map_or_else(|| id.to_string(), |f| f.name.clone());
    let names = crate::plate::plate_names(
        id,
        &part.solid,
        &part.sheet,
        &sheet_names,
        &picked,
        &made_by,
        tolerance.linear,
    )?;
    face_ops::not_through_itself(
        &part.solid.body,
        &names.faces,
        "flanges",
        "sheet-metal part",
        "flanges that run into each other or into the base when bent: ",
        &tolerance,
    )?;
    Ok((part.solid.body.clone(), names, Arc::new(built)))
}

fn part_of(doc: &Document, id: FeatureId) -> Result<&SheetMetal, String> {
    let feature = doc
        .feature(id)
        .ok_or_else(|| format!("there is no feature {id}"))?;
    match &feature.kind {
        FeatureKind::SheetMetal(made) => Ok(made),
        other => Err(format!(
            "`{}` is a {}, not a sheet-metal part",
            feature.name,
            other.type_name()
        )),
    }
}

/// The sheet-metal part `id` of `doc` as regeneration (`regen`) built it:
/// what the bend table, the manufacturability measures and the flat
/// pattern's export read. Refused, with the reason, when the feature did
/// not build (failed, switched off, rolled back) or is no sheet-metal part.
pub fn sheet_metal_part(
    doc: &Document,
    regen: &crate::Regen,
    id: FeatureId,
) -> Result<Arc<SheetMetalPart>, String> {
    let feature = doc
        .feature(id)
        .ok_or_else(|| format!("there is no feature {id}"))?;
    part_of(doc, id)?;
    let result = regen
        .features
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("`{}` has not been regenerated", feature.name))?;
    match &result.status {
        crate::Status::Ok => {}
        crate::Status::Failed(reason) => {
            return Err(format!("`{}` failed: {reason}", feature.name));
        }
        crate::Status::Suppressed | crate::Status::Inactive => {
            return Err(format!("`{}` is switched off", feature.name));
        }
        crate::Status::RolledBack => {
            return Err(format!("`{}` is after the rollback marker", feature.name));
        }
    }
    match &result.developed {
        Some(crate::Developed::SheetMetal(part)) => Ok(part.clone()),
        _ => Err(format!("`{}` keeps no part; this is a bug", feature.name)),
    }
}
