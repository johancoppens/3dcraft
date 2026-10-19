//! Delete face in regeneration: faces of the body of an earlier feature
//! taken out, the body closed again by the faces round them. How a face
//! is referred to is in [`crate::face_ops`].
//!
//! **Names.** Every face of the result is a face of the body it was made
//! from, on the surface it had, so it is named as a copy of that face
//! (`F9:copy(0)/F2:side(e7)`), as in every feature that makes its body
//! from another. Edges and vertices are named by the faces they lie on
//! (docs/design/names.md), with the copy of the name they had as an
//! alias when they were there before: the edge a strip collapsed into and
//! the vertex its two ends became are new and have only the first.

use crate::face_ops::{self, explain};
use crate::{Contributors, FeatureResult, Kept, Names};
use craft3d_doc::{DeleteFace, Document, FeatureId, NameRole};
use craft3d_kernel::delete_face::delete_faces;
use craft3d_topo::Body;

const NOUN: &str = "a deletion";

pub(crate) fn build(
    doc: &Document,
    feature: FeatureId,
    deletion: &DeleteFace,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let Some(source_id) = deletion.source() else {
        return Err("faces: a deletion needs at least one face".to_string());
    };
    let source = face_ops::source(
        doc,
        earlier,
        feature,
        source_id,
        deletion.faces.first(),
        "faces",
        NOUN,
    )?;
    // The corner where blends meet is no face that can go on its own:
    // said by its name before the kernel says it in numbers.
    for (index, name) in deletion.faces.iter().enumerate() {
        if matches!(
            name.root().role,
            NameRole::ChamferCorner | NameRole::FilletCorner
        ) {
            return Err(format!(
                "faces.{index}: {name} is the corner where three blends meet; deleting a corner is not built (delete the blends with feature.suppress, or edit them)"
            ));
        }
    }
    let picked = face_ops::pick(&deletion.faces, "faces", NOUN, source_id, &source)?;
    let unit = doc.units.length.symbol();
    let made = delete_faces(source.body, &picked, &source.tolerance)
        .map_err(|error| explain(error, "faces", 1.0, unit, &source))?;
    let contributors: Contributors = made
        .faces
        .iter()
        .map(|face| vec![(0, face.0 as usize)])
        .collect();
    let Some(mut names) = Names::copied_faces(feature, &contributors, |_| Some(source.names))?
    else {
        return Ok((made.body, None));
    };
    let kept_edges: Kept = made
        .edges
        .iter()
        .map(|edge| edge.map(|edge| (0, edge.0 as usize)))
        .collect();
    let kept_vertices: Kept = made
        .vertices
        .iter()
        .map(|vertex| vertex.map(|vertex| (0, vertex.0 as usize)))
        .collect();
    names.name_entities(
        feature,
        &made.body,
        source.tolerance.linear,
        &kept_edges,
        &kept_vertices,
        |_| Some(source.names),
    )?;
    face_ops::not_through_itself(
        &made.body,
        &names.faces,
        "faces",
        "body with the faces deleted",
        "the faces round them, continued, run into the body elsewhere: ",
        &source.tolerance,
    )?;
    Ok((made.body, Some(names)))
}
