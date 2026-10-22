//! Move face in regeneration: faces of the body of an earlier feature
//! moved together by one rigid motion. How a face is referred to and how
//! the result is named is in [`crate::face_ops`]: as after an offset,
//! every face, edge and vertex is the copy of what it was.

use crate::derive::{direction, lengths};
use crate::face_ops::{self, about};
use crate::offset_face::same_entities;
use crate::{FeatureResult, Names, angle};
use craft3d_doc::{Document, FeatureId, MoveFace};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::move_face::{Motion, move_faces_capped};
use craft3d_math::Vec3;
use craft3d_topo::Body;

const NOUN: &str = "a move";

/// The motion of `moving`, in model units and radians.
pub(crate) fn motion(
    doc: &Document,
    variables: &Evaluation,
    moving: &MoveFace,
) -> Result<Motion, String> {
    let translate = lengths(doc, variables, "translate", &moving.translate)?;
    let turn = angle(doc, variables, "angle", &moving.angle)?;
    if !turn.is_finite() {
        return Err("angle: is not finite".to_string());
    }
    // No turn: the axis and the pivot are not looked at, so they need
    // not be valid.
    let (pivot, axis) = if turn == 0.0 {
        (Vec3::ZERO, Vec3::Z)
    } else {
        (
            lengths(doc, variables, "pivot", &moving.pivot)?,
            direction(variables, "axis", &moving.axis)?,
        )
    };
    Ok(Motion {
        translate,
        pivot,
        axis,
        angle: turn,
    })
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    moving: &MoveFace,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let Some(source_id) = moving.source() else {
        return Err("faces: a move needs at least one face".to_string());
    };
    let source = face_ops::source(
        doc,
        earlier,
        feature,
        source_id,
        moving.faces.first(),
        "faces",
        NOUN,
    )?;
    let motion = motion(doc, variables, moving)?;
    let picked = face_ops::pick(&moving.faces, "faces", NOUN, source_id, &source)?;
    let made = move_faces_capped(
        source.body,
        &picked,
        &motion,
        &source.tolerance,
        Some(source.loosest),
    ).map_err(|error| {
        match error {
            KernelError::FaceOpTooFar {
                faces,
                edge,
                reason,
                value,
                largest,
                ..
            } => {
                // The same part of the shift and of the turn fits.
                let part = if value > 0.0 { largest / value } else { 0.0 };
                let unit = doc.units.length.symbol();
                let at = about(source.body, source.names, &faces, edge)
                    .map(|at| format!(" ({at})"))
                    .unwrap_or_default();
                let shift = motion.translate * part;
                let mut fits = String::new();
                if motion.translate != Vec3::ZERO {
                    fits.push_str(&format!(
                        "translate ({}, {}, {}) {unit}",
                        shift.x, shift.y, shift.z
                    ));
                }
                if motion.angle != 0.0 {
                    if !fits.is_empty() {
                        fits.push_str(" with ");
                    }
                    fits.push_str(&format!(
                        "angle {} deg",
                        (motion.angle * part).to_degrees()
                    ));
                }
                format!(
                    "translate, angle: the move is too far: {reason}{at}; what fits is below {part:.6} of it, {fits}"
                )
            }
            other => face_ops::explain(other, "translate", 1.0, "", &source),
        }
    })?;
    let names = same_entities(feature, source.names)?;
    face_ops::not_through_itself_where_changed(
        source.body,
        &made.body,
        &names.faces,
        "translate",
        "body with the faces moved",
        "too far: ",
        &source.tolerance,
    )?;
    Ok((made.body, Some(names)))
}
