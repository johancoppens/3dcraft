//! Offset face in regeneration: faces of the body of an earlier feature
//! are moved along their normals. How a face is referred to and how the
//! result is named is in [`crate::face_ops`].

use crate::derive::{copied, copied_aliases};
use crate::face_ops::{self, explain};
use crate::{FeatureResult, Names, length};
use craft3d_doc::{Document, FeatureId, OffsetFace};
use craft3d_expr::Evaluation;
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_topo::Body;

const NOUN: &str = "an offset";

/// The names of a body that has the entities of the body `names` names,
/// each under its own number: every one is the copy of what it was.
pub(crate) fn same_entities(feature: FeatureId, names: &Names) -> Result<Names, String> {
    let mut made = Names::default();
    copied(feature, 0, &names.faces, &mut made.faces)?;
    copied(feature, 0, &names.edges, &mut made.edges)?;
    copied(feature, 0, &names.vertices, &mut made.vertices)?;
    copied_aliases(feature, 0, &names.aliases, 0, &mut made.aliases);
    copied_aliases(feature, 0, &names.edge_aliases, 0, &mut made.edge_aliases);
    copied_aliases(
        feature,
        0,
        &names.vertex_aliases,
        0,
        &mut made.vertex_aliases,
    );
    Ok(made)
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    offset: &OffsetFace,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let Some(source_id) = offset.source() else {
        return Err("faces: an offset needs at least one face".to_string());
    };
    let source = face_ops::source(
        doc,
        earlier,
        feature,
        source_id,
        offset.faces.first(),
        "faces",
        NOUN,
    )?;
    let distance = length(doc, variables, "distance", &offset.distance)?;
    if !distance.is_finite() {
        return Err(format!("distance: has to be a length, got {distance}"));
    }
    let unit = doc.units.length.symbol();
    if distance.abs() <= source.tolerance.linear {
        return Err(format!(
            "distance: {distance} {unit} cannot be told from no move at this document's tolerance; give a positive distance to move the faces outward, a negative one to move them into the material"
        ));
    }
    let picked = face_ops::pick(&offset.faces, "faces", NOUN, source_id, &source)?;
    let offsets: Vec<FaceOffset> = picked
        .iter()
        .map(|face| FaceOffset {
            face: *face,
            distance,
        })
        .collect();
    let made = offset_faces(source.body, &offsets, &source.tolerance)
        .map_err(|error| explain(error, "distance", distance.signum(), unit, &source))?;
    let names = same_entities(feature, source.names)?;
    face_ops::not_through_itself(
        &made.body,
        &names.faces,
        "distance",
        "body with the faces moved",
        &source.tolerance,
    )?;
    Ok((made.body, Some(names)))
}
