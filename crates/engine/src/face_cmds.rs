//! The face operations: features that make a body from the body of an
//! earlier feature by moving faces of it (`feature.offsetFace`) or
//! hollowing it (`feature.shell`).
//!
//! A face is given by its persistent name from `document.inspect`.
//! Whether the faces exist and leave room for the distance is found out
//! by regeneration, like every other parameter: the feature is stored
//! and reports `failed` with the reason.

use crate::commands::{Run, add_feature, fail, feature_ref, spec};
use crate::datum_cmds::face_value;
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Expr, FeatureKind, MAX_FACE_OP_FACES, OffsetFace, PersistentName, Shell};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.offsetFace", "Offset Face", r#"{"faces": [face name, ...], "distance": expr, "name": string = automatic}"#,
        "Adds a feature that moves faces of a body along their normals by distance and lets the faces next to them follow, so the body stays closed: positive is outward, away from the material (a block gets taller, a boss thicker, a hole narrower), negative is into the material (a pocket gets deeper when its floor moves, a hole wider when its wall does: a hole of radius 10 with distance -1 has radius 11). A flat face moves to the parallel plane; a cylinder, a sphere and a cone grow or shrink about the same axis. The neighbours are continued or cut back to where they meet the moved face: straight and slanted flat neighbours, and round ones about an axis the moved face is square to (a hole through a face that moves gets longer). The faces are given by their names (faceNames in document.inspect, or view.pick); all are faces of one body, which goes into this feature. Nothing is added or taken away: the body has the faces, edges and vertices it had, under their names with a prefix, \"F9:copy(0)/...\", and a sketch on a face that moved lies where the face lies now. A distance at which an edge shrinks to nothing, a hole leaves its face or the body would pass through itself fails with the edge or the face and the largest distance that fits. Not built, and refused with the faces: a neighbour that is tangent to the moved face (a fillet next to it), a vertex where four or more faces meet, an elliptic or freeform edge next to a moved face, a freeform face. Change it with feature.edit: faces, distance.", doc: true, feature_offset_face),
    spec!("feature.shell", "Shell", r#"{"source": id or name, "thickness": expr, "open": [face name, ...] = [], "name": string = automatic}"#,
        "Adds a feature that makes the body of source hollow: walls of thickness are left behind every face, and the faces of open are taken away, so the hollow is open there and as wide as it is inside (a block open at its top is a box, a cylinder open at both ends a tube). Without open faces the hollow is a closed cavity inside, a second shell of the body. The walls follow the faces: flat faces, cylinders, cones and spheres, so a rounded box has inner rounds of the radius less the thickness, a hole through the body gets a tube round it and a boss a bore. The source's body goes into this feature. Faces of the outside keep their names with a prefix, \"F9:copy(0)/...\"; a face of the hollow is named after the face it lies behind, in instance 1: \"F9:copy(1)/...\" (a sketch on it looks into the hollow). A thickness at which a wall of the hollow would vanish (half the width of the body, the radius of a rounded edge) fails with the face or the edge and the largest thickness that fits; at the radius of a rounded edge or more the hollow would need a sharp edge there, which is not built. Not built, and refused with the faces: a face next to a tangent one whose offsets no longer meet as they did, a vertex where four or more faces meet, an elliptic or freeform edge, a freeform face. Change it with feature.edit: source, thickness, open.", doc: true, feature_shell),
];

/// A list of face names of one body, `most` at most; `noun` is the
/// feature, in the words of what is refused.
pub(crate) fn faces_value(
    command: &str,
    key: &str,
    value: &Value,
    noun: &str,
    may_be_empty: bool,
) -> Result<Vec<PersistentName>, CommandError> {
    let Some(items) = value.as_array() else {
        return fail(format!(
            "{command}: `{key}` must be a list of face names (faceNames in document.inspect lists them)"
        ));
    };
    if items.is_empty() && !may_be_empty {
        return fail(format!(
            "{command}: `{key}` is empty; give at least one face"
        ));
    }
    if items.len() > MAX_FACE_OP_FACES {
        return fail(format!(
            "{command}: `{key}` has {} faces; one {noun} takes {MAX_FACE_OP_FACES} at most",
            items.len()
        ));
    }
    let mut faces: Vec<PersistentName> = Vec::with_capacity(items.len());
    for item in items {
        let face = face_value(command, key, item)?;
        if faces.contains(&face) {
            return fail(format!("{command}: `{key}`: {face} is given twice"));
        }
        if let Some(first) = faces.first()
            && first.feature != face.feature
        {
            return fail(format!(
                "{command}: `{key}`: {face} is not a face of {}, the feature of the first face; one {noun} takes faces of one body",
                first.feature
            ));
        }
        faces.push(face);
    }
    Ok(faces)
}

/// `feature.edit` for an offset: `faces` (`distance` is a plain
/// expression, set where the others are).
pub(crate) fn edit_offset(p: &Params, offset: &mut OffsetFace) -> Result<(), CommandError> {
    if let Some(value) = p.get("faces") {
        offset.faces = faces_value("feature.edit", "faces", value, "offset", false)?;
    }
    Ok(())
}

fn feature_offset_face(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.offsetFace";
    let p = Params::new(ID, params, &["faces", "distance", "name"])?;
    // The document has to be there before anything is said of the
    // parameters.
    session.document()?;
    let Some(value) = p.get("faces") else {
        return Err(p.missing("faces"));
    };
    let faces = faces_value(ID, "faces", value, "offset", false)?;
    let Some(distance) = p.expr("distance")? else {
        return Err(p.missing("distance"));
    };
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::OffsetFace(OffsetFace {
            faces,
            distance: Expr::new(distance),
        }),
    )
}

/// `feature.edit` for a shell: `open` (`source` and `thickness` are set
/// where the others are).
pub(crate) fn edit_shell(p: &Params, shell: &mut Shell) -> Result<(), CommandError> {
    if let Some(value) = p.get("open") {
        shell.open = faces_value("feature.edit", "open", value, "shell", true)?;
    }
    Ok(())
}

fn feature_shell(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.shell";
    let p = Params::new(ID, params, &["source", "thickness", "open", "name"])?;
    let source = feature_ref(session.document()?.history.current(), &p, ID, "source")?;
    let Some(thickness) = p.expr("thickness")? else {
        return Err(p.missing("thickness"));
    };
    let open = match p.get("open") {
        Some(value) => faces_value(ID, "open", value, "shell", true)?,
        None => Vec::new(),
    };
    add_feature(
        session,
        ID,
        p.str("name")?,
        FeatureKind::Shell(Shell {
            source,
            thickness: Expr::new(thickness),
            open,
        }),
    )
}
