//! The face operations: features that make a body from the body of an
//! earlier feature by moving faces of it (`feature.offsetFace`), taking
//! faces out of it (`feature.deleteFace`), moving faces by a rigid motion
//! (`feature.moveFace`), tilting faces (`feature.draft`) or hollowing it
//! (`feature.shell`), and the
//! feature that makes a body of its own from faces of one
//! (`feature.thicken`).
//!
//! A face is given by its persistent name from `document.inspect`.
//! Whether the faces exist and leave room for the distance is found out
//! by regeneration, like every other parameter: the feature is stored
//! and reports `failed` with the reason.

use crate::commands::{Run, add_feature, exprs, fail, feature_ref, origin, spec};
use crate::datum_cmds::face_value;
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    DeleteFace, Draft, Expr, FeatureKind, MAX_FACE_OP_FACES, MoveFace, OffsetFace, PersistentName,
    Shell, Thicken, ThickenSide,
};
use serde_json::Value;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.offsetFace", "Offset Face", r#"{"faces": [face name, ...], "distance": expr, "name": string = automatic, "when": expr = always on}"#,
        "Adds a feature that moves faces of a body along their normals by distance and lets the faces next to them follow, so the body stays closed: positive is outward, away from the material (a block gets taller, a boss thicker, a hole narrower), negative is into the material (a pocket gets deeper when its floor moves, a hole wider when its wall does: a hole of radius 10 with distance -1 has radius 11). A flat face moves to the parallel plane; a cylinder, a sphere and a cone grow or shrink about the same axis; a freeform (Bezier or NURBS) face becomes a spline surface within a quarter of the tolerance of its exact offset, refused where that would fold over (the distance reaches a radius of curvature of the face on the side it moves to: fails with the largest distance that fits) or where its normal is not defined (a pole). The neighbours are continued or cut back to where they meet the moved face: straight and slanted flat neighbours, round ones about an axis the moved face is square to (a hole through a face that moves gets longer), and freeform ones, continued past their borders where they have to be; an edge that is no longer a line or a circle is found again where its faces meet. The faces are given by their names (faceNames in document.inspect, or view.pick); all are faces of one body, which goes into this feature (a name of an earlier body of the same chain, such as \"F1:face(0)\" after that body went into later features, is followed to the newest body before this feature). Nothing is added or taken away: the body has the faces, edges and vertices it had, under their names with a prefix, \"F9:copy(0)/...\", and a sketch on a face that moved lies where the face lies now. A distance at which an edge shrinks to nothing, a hole leaves its face or the body would pass through itself fails with the edge or the face and the largest distance that fits. Not built, and refused with the faces: a neighbour that is tangent to the moved face and does not move with it (a fillet next to it), a vertex where four or more faces meet that would have to become an edge, a freeform face with a pole. Change it with feature.edit: faces, distance.", doc: true, feature_offset_face),
    spec!("feature.moveFace", "Move Face", r#"{"faces": [face name, ...], "translate": [expr, expr, expr] = [0, 0, 0], "angle": expr = "0 deg", "axis": [expr, expr, expr] = [0, 0, 1], "pivot": [expr, expr, expr] = [0, 0, 0], "name": string = automatic, "when": expr = always on}"#,
        "Adds a feature that moves faces of a body together by one rigid motion, as feature.transform moves a body: turned by angle about the line through pivot along axis (counter-clockwise seen from the tip of axis), then shifted by translate. The faces next to them follow, so the body stays closed: they are continued or cut back to where they meet the moved faces. The top of a block shifted up makes it taller, tilted about a line across it makes a wedge; the wall of a hole, or the wall and top of a boss, shifted sideways put the hole or the boss somewhere else on its face. What lies between two moved faces moves with them as it is, whatever its kind. The faces are given by their names (faceNames in document.inspect, or view.pick); all are faces of one body, which goes into this feature (a name of an earlier body of the same chain is followed to the newest body before this feature). Nothing is added or taken away: the body has the faces, edges and vertices it had, under their names with a prefix, \"F9:copy(0)/...\", and a sketch on a face that moved lies where the face lies now, turned with it. A motion at which an edge shrinks to nothing, a hole leaves its face or the body would pass through itself fails with the edge or the face and the part of the motion that fits. Refused, with the faces, where the faces round a moved one would have to change: an edge between a moved face and one that stays that would no longer be a line or a circle (a cylinder tilted against a flat face meets it in an ellipse), a neighbour tangent to a moved face, a vertex where four or more faces meet that would have to become an edge, a freeform face next to a moved one. Change it with feature.edit: faces, translate, pivot, axis, angle.", doc: true, feature_move_face),
    spec!("feature.thicken", "Thicken", r#"{"faces": [face name, ...], "thickness": expr, "side": "outside|inside|both" = "outside", "name": string = automatic, "when": expr = always on}"#,
        "Adds a feature whose body is a layer of thickness on faces of a body: outside puts it on the faces, away from the material, inside under them, in the material, both half on each side. At its rim the layer is bounded by the faces round the ones given, continued as feature.offsetFace continues them: the top of a block thickened outside is a plate on it as wide as the block, the wall of a rod a tube as long as the rod, the wall of a hole a sleeve in it (outside) or round it (inside), the top and a side of a block an angle. The faces are given by their names (faceNames in document.inspect, or view.pick); all are faces of one body, which keeps its own body: the layer is a body of its own (combine it with feature.boolean). A face of the layer that lies on or along a face of the body is named as its copy in instance 0, \"F9:copy(0)/...\" (the far side of the layer and its rim), one that lies under a face given as its copy in instance 1, \"F9:copy(1)/...\" (the near side; a sketch on it looks into the body). A thickness at which a face of the layer would vanish fails with the face or the edge and the largest thickness that fits. Not built, and refused with the faces, as for feature.offsetFace: a face next to a tangent one, a vertex where four or more faces meet, a freeform face with a pole or whose offset would fold over. Change it with feature.edit: faces, thickness, side.", doc: true, feature_thicken),
    spec!("feature.draft", "Draft", r#"{"faces": [face name, ...], "neutral": plane, "angle": expr, "reversed": boolean = false, "name": string = automatic, "when": expr = always on}"#,
        "Adds a feature that tilts faces of a body by angle so that the part comes out of a mould: each face keeps the line or circle in which it meets the neutral plane and leans in by the angle as it goes along the pull direction, the normal of neutral (xy, yz or zx, a datum plane, or a flat face by its name: then the pull goes into the body the face bounds; reversed turns it round). A block drafted with its bottom face as neutral narrows toward its top (a frustum), with its top face toward its bottom; a hole drafted from its bottom widens toward its top; a negative angle leans the other way. Flat faces that run along the pull direction stay flat, turned about the line they share with the neutral plane; a cylinder whose axis runs along it becomes a cone with the cylinder's radius at the neutral plane; a flat face tangent to a drafted cylinder stays tangent to its cone (a rounded box drafted is a rounded frustum). The faces next to them follow, as with feature.offsetFace. The faces are given by their names (faceNames in document.inspect, or view.pick); all are faces of one body, which goes into this feature. Nothing is added or taken away: the faces, edges and vertices keep their names with a prefix, \"F9:copy(0)/...\", and a sketch on a drafted face lies on it, turned with it. An angle at which a face would vanish (the top drawn to a point) fails with the face or the edge and the largest angle that fits. Refused, with the face: a face that does not run along the pull direction (its normal not square to it, a cylinder's axis not along it), a cone, a sphere, a torus, a freeform face, and what feature.offsetFace refuses round it (a neighbour tangent to a drafted face that is not drafted itself, a vertex where four or more faces meet). Change it with feature.edit: faces, neutral, angle, reversed.", doc: true, feature_draft),
    spec!("feature.deleteFace", "Delete Face", r#"{"faces": [face name, ...], "name": string = automatic, "when": expr = always on}"#,
        "Adds a feature that takes faces out of a body and closes it again with the faces round them, as if those faces had never been made. Faces that together are bounded only by whole holes of the faces round them are filled from those faces: the wall of a hole through a plate (the plate is whole again), the wall and the floor of a blind hole, the wall and the top of a boss, the walls and the floor of a pocket, the faces of a closed cavity; give all of them. A single chamfer or fillet face along one edge (between two faces, ending against a face at each end, or running all the way round a rim) is collapsed into the sharp edge between the two faces beside it, which are continued until they meet. The faces are given by their names (faceNames in document.inspect, or view.pick); all are faces of one body, which goes into this feature (a name of an earlier body of the same chain, such as \"F1:face(0)\" after that body went into later features, is followed to the newest body before this feature). The faces that stay keep their names with a prefix, \"F9:copy(0)/...\"; edges and vertices are named by their faces. Refused, with the faces and the reason: a face whose loops are not holes of its neighbours and that is no strip (the top of a block, a boss's wall alone), the corner where three blends meet, a blend that ends against another blend (a chain, a mitre) or where more than three faces meet, faces beside a strip that do not meet in a line or a circle when continued, and a deletion after which the body would pass through itself or enclose nothing. Change it with feature.edit: faces.", doc: true, feature_delete_face),
    spec!("feature.shell", "Shell", r#"{"source": id or name, "thickness": expr, "open": [face name, ...] = [], "name": string = automatic, "when": expr = always on}"#,
        "Adds a feature that makes the body of source hollow: walls of thickness are left behind every face, and the faces of open are taken away, so the hollow is open there and as wide as it is inside (a block open at its top is a box, a cylinder open at both ends a tube). Without open faces the hollow is a closed cavity inside, a second shell of the body. The walls follow the faces: flat faces, cylinders, cones and spheres, so a rounded box has inner rounds of the radius less the thickness, a hole through the body gets a tube round it and a boss a bore; a freeform face (a loft, a rolling-ball fillet) has a spline wall within a quarter of the tolerance of its exact offset. The source's body goes into this feature. Faces of the outside keep their names with a prefix, \"F9:copy(0)/...\"; a face of the hollow is named after the face it lies behind, in instance 1: \"F9:copy(1)/...\" (a sketch on it looks into the hollow). A thickness at which a wall of the hollow would vanish (half the width of the body, the radius of a rounded edge) fails with the face or the edge and the largest thickness that fits; at the radius of a rounded edge or more the hollow would need a sharp edge there, which is not built. Not built, and refused with the faces: a face next to a tangent one whose offsets no longer meet as they did, a vertex where four or more faces meet, a freeform face with a pole, or one whose offset would fold over (a thickness at or past a radius of curvature on the inside: fails with the largest thickness that fits). Change it with feature.edit: source, thickness, open.", doc: true, feature_shell),
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
        // Faces named after different features can be faces of one body
        // (regen `follow`); regeneration finds out whether they are.
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
    let kind = FeatureKind::OffsetFace(OffsetFace {
        faces,
        distance: Expr::new(distance),
    });
    crate::blend_cmds::of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
}

/// `feature.edit` for a move: `faces` (the motion's expressions are set
/// where the others are).
pub(crate) fn edit_move(p: &Params, moving: &mut MoveFace) -> Result<(), CommandError> {
    if let Some(value) = p.get("faces") {
        moving.faces = faces_value("feature.edit", "faces", value, "move", false)?;
    }
    Ok(())
}

fn feature_move_face(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.moveFace";
    let p = Params::new(
        ID,
        params,
        &["faces", "translate", "angle", "axis", "pivot", "name"],
    )?;
    // The document has to be there before anything is said of the
    // parameters.
    session.document()?;
    let Some(value) = p.get("faces") else {
        return Err(p.missing("faces"));
    };
    let faces = faces_value(ID, "faces", value, "move", false)?;
    if !p.has("translate") && !p.has("angle") {
        return fail(format!(
            "{ID}: give `translate`, `angle` (with `axis` and `pivot`) or both: there is nothing to move the faces by"
        ));
    }
    let z_axis = || ["0".to_string(), "0".to_string(), "1".to_string()];
    let kind = FeatureKind::MoveFace(MoveFace {
        faces,
        translate: exprs(p.expr3("translate")?.unwrap_or_else(origin)),
        pivot: exprs(p.expr3("pivot")?.unwrap_or_else(origin)),
        axis: exprs(p.expr3("axis")?.unwrap_or_else(z_axis)),
        angle: Expr::new(p.expr("angle")?.unwrap_or_else(|| "0 deg".to_string())),
    });
    crate::blend_cmds::of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
}

/// `feature.edit` for a draft: `faces` (the neutral plane is set by
/// `feature.edit`, which knows the document; `angle` and `reversed` where
/// the others are).
pub(crate) fn edit_draft(p: &Params, tilted: &mut Draft) -> Result<(), CommandError> {
    if let Some(value) = p.get("faces") {
        tilted.faces = faces_value("feature.edit", "faces", value, "draft", false)?;
    }
    Ok(())
}

fn feature_draft(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.draft";
    let p = Params::new(
        ID,
        params,
        &["faces", "neutral", "angle", "reversed", "name"],
    )?;
    // The document has to be there before anything is said of the
    // parameters.
    let doc = session.document()?.history.current();
    let Some(value) = p.get("faces") else {
        return Err(p.missing("faces"));
    };
    let faces = faces_value(ID, "faces", value, "draft", false)?;
    let Some(value) = p.get("neutral") else {
        return Err(p.missing("neutral"));
    };
    let neutral = crate::datum_cmds::plane_value(doc, ID, "neutral", value)?;
    let Some(angle) = p.expr("angle")? else {
        return Err(p.missing("angle"));
    };
    let kind = FeatureKind::Draft(Draft {
        faces,
        neutral,
        angle: Expr::new(angle),
        reversed: p.bool("reversed")?.unwrap_or(false),
    });
    crate::blend_cmds::of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
}

/// The side of a layer, as `feature.thicken` and `feature.edit` take it.
fn side_value(command: &str, p: &Params) -> Result<Option<ThickenSide>, CommandError> {
    let Some(side) = p.str("side")? else {
        return Ok(None);
    };
    Ok(Some(match side {
        "outside" => ThickenSide::Outside,
        "inside" => ThickenSide::Inside,
        "both" => ThickenSide::Both,
        other => {
            return fail(format!(
                "{command}: `side` is \"{other}\"; give \"outside\", \"inside\" or \"both\""
            ));
        }
    }))
}

/// `feature.edit` for a layer: `faces` and `side` (`thickness` is a
/// plain expression, set where the others are).
pub(crate) fn edit_thicken(p: &Params, layer: &mut Thicken) -> Result<(), CommandError> {
    if let Some(value) = p.get("faces") {
        layer.faces = faces_value("feature.edit", "faces", value, "layer", false)?;
    }
    if let Some(side) = side_value("feature.edit", p)? {
        layer.side = side;
    }
    Ok(())
}

fn feature_thicken(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.thicken";
    let p = Params::new(ID, params, &["faces", "thickness", "side", "name"])?;
    // The document has to be there before anything is said of the
    // parameters.
    session.document()?;
    let Some(value) = p.get("faces") else {
        return Err(p.missing("faces"));
    };
    let faces = faces_value(ID, "faces", value, "layer", false)?;
    let Some(thickness) = p.expr("thickness")? else {
        return Err(p.missing("thickness"));
    };
    let kind = FeatureKind::Thicken(Thicken {
        faces,
        thickness: Expr::new(thickness),
        side: side_value(ID, &p)?.unwrap_or_default(),
    });
    crate::blend_cmds::of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
}

/// `feature.edit` for a deletion: `faces`.
pub(crate) fn edit_delete(p: &Params, deletion: &mut DeleteFace) -> Result<(), CommandError> {
    if let Some(value) = p.get("faces") {
        deletion.faces = faces_value("feature.edit", "faces", value, "deletion", false)?;
    }
    Ok(())
}

fn feature_delete_face(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.deleteFace";
    let p = Params::new(ID, params, &["faces", "name"])?;
    // The document has to be there before anything is said of the
    // parameters.
    session.document()?;
    let Some(value) = p.get("faces") else {
        return Err(p.missing("faces"));
    };
    let faces = faces_value(ID, "faces", value, "deletion", false)?;
    let kind = FeatureKind::DeleteFace(DeleteFace { faces });
    crate::blend_cmds::of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
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
    let kind = FeatureKind::Shell(Shell {
        source,
        thickness: Expr::new(thickness),
        open,
    });
    crate::blend_cmds::of_one_body(ID, session, &kind)?;
    add_feature(session, ID, p.str("name")?, kind)
}
