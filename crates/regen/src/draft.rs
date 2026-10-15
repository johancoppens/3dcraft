//! Draft in regeneration: faces of the body of an earlier feature tilted
//! about a neutral plane. How a face is referred to and how the result is
//! named is in [`crate::face_ops`]: as after an offset, every face, edge
//! and vertex is the copy of what it was.
//!
//! **The pull direction** is the normal of the neutral plane: of a world
//! or a datum plane its own, of a face the one into the body the face
//! bounds (a block drafted from its bottom face narrows toward its top,
//! from its top face toward its bottom); `reversed` turns it round.

use crate::face_ops::{self, about};
use crate::offset_face::same_entities;
use crate::{FeatureResult, Names, angle};
use craft3d_doc::{Document, Draft, FeatureId, PlaneRef};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::draft::{Draft as Tilt, draft};
use craft3d_topo::Body;

const NOUN: &str = "a draft";

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    tilted: &Draft,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let Some(source_id) = tilted.source() else {
        return Err("faces: a draft needs at least one face".to_string());
    };
    let source = face_ops::source(
        doc,
        earlier,
        feature,
        source_id,
        tilted.faces.first(),
        "faces",
        NOUN,
    )?;
    let turn = angle(doc, variables, "angle", &tilted.angle)?;
    if !(turn.is_finite() && turn.abs() < std::f64::consts::FRAC_PI_2) {
        return Err(format!(
            "angle: has to be between -90 and 90 degrees, got {} deg",
            (turn.to_degrees() * 1e9).round() / 1e9
        ));
    }
    if turn.abs() <= source.tolerance.angular {
        return Err(
            "angle: cannot be told from none at this document's tolerance; give the angle the faces lean in by"
                .to_string(),
        );
    }
    let neutral = crate::datum::plane_ref(doc, earlier, &tilted.neutral, "neutral")?;
    // Into the body for a face, whose frame looks out of it.
    let into = matches!(tilted.neutral, PlaneRef::Face { .. });
    let pull = if into != tilted.reversed {
        -neutral.z()
    } else {
        neutral.z()
    };
    let picked = face_ops::pick(&tilted.faces, "faces", NOUN, source_id, &source)?;
    let how = Tilt {
        faces: picked,
        point: neutral.origin(),
        pull,
        angle: turn,
    };
    let made = draft(source.body, &how, &source.tolerance).map_err(|error| match error {
        KernelError::FaceOpTooFar {
            faces,
            edge,
            reason,
            value,
            largest,
            ..
        } => {
            let part = if value > 0.0 { largest / value } else { 0.0 };
            let at = about(source.body, source.names, &faces, edge)
                .map(|at| format!(" ({at})"))
                .unwrap_or_default();
            // In degrees, without the last bits of the conversion.
            let degrees = |radians: f64| (radians.to_degrees() * 1e9).round() / 1e9;
            format!(
                "angle: {} deg is too far: {reason}{at}; what fits is {} {} deg",
                degrees(turn),
                if turn < 0.0 { "above" } else { "below" },
                degrees(turn * part)
            )
        }
        other => face_ops::explain(other, "angle", 1.0, "deg", &source),
    })?;
    let names = same_entities(feature, source.names)?;
    face_ops::not_through_itself_where_changed(
        source.body,
        &made.body,
        &names.faces,
        "angle",
        "body with the faces drafted",
        "too far: ",
        &source.tolerance,
    )?;
    Ok((made.body, Some(names)))
}
