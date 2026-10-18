//! Thicken in regeneration: faces of the body of an earlier feature given
//! a thickness, as a body of its own; the source keeps its body. How a
//! face is referred to is in [`crate::face_ops`]; the layer is named as a
//! shell is: a face that lies on or along a face of the source is its
//! copy in instance 0 (the far side of the layer, and its rim along the
//! faces round it), a face that lies under one is its copy in instance 1
//! (the near side).

use crate::face_ops::{self, explain};
use crate::{Contributors, FeatureResult, Kept, Names, length};
use craft3d_doc::{Document, FeatureId, Thicken, ThickenSide};
use craft3d_expr::Evaluation;
use craft3d_kernel::shell::Skin;
use craft3d_kernel::thicken::{Side, thicken_with};
use craft3d_topo::Body;

const NOUN: &str = "a layer";

fn instance(skin: Skin) -> u64 {
    match skin {
        Skin::Outer => 0,
        Skin::Inner => 1,
    }
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    layer: &Thicken,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let Some(source_id) = layer.source() else {
        return Err("faces: a layer needs at least one face".to_string());
    };
    let source = face_ops::source(
        doc,
        earlier,
        feature,
        source_id,
        layer.faces.first(),
        "faces",
        NOUN,
    )?;
    let thickness = length(doc, variables, "thickness", &layer.thickness)?;
    let unit = doc.units.length.symbol();
    if !(thickness.is_finite() && thickness > source.tolerance.linear) {
        return Err(format!(
            "thickness: has to be more than this document's tolerance, got {thickness} {unit}"
        ));
    }
    let picked = face_ops::pick(&layer.faces, "faces", NOUN, source_id, &source)?;
    let side = match layer.side {
        ThickenSide::Outside => Side::Outside,
        ThickenSide::Inside => Side::Inside,
        ThickenSide::Both => Side::Both,
    };
    let blends = face_ops::blends(doc, variables, &source);
    let made = thicken_with(
        source.body,
        &picked,
        thickness,
        side,
        &blends,
        &source.tolerance,
    )
    .map_err(|error| explain(error, "thickness", 1.0, unit, &source))?;
    let contributors: Contributors = made
        .faces
        .iter()
        .map(|(skin, face)| vec![(instance(*skin), face.0 as usize)])
        .collect();
    // The source names both sides: each has its entities.
    let Some(mut names) = Names::copied_faces(feature, &contributors, |_| Some(source.names))?
    else {
        return Ok((made.body, None));
    };
    let kept = |list: &[Option<(Skin, craft3d_topo::EdgeId)>]| -> Kept {
        list.iter()
            .map(|entry| entry.map(|(skin, id)| (instance(skin), id.0 as usize)))
            .collect()
    };
    let vertices: Kept = made
        .vertices
        .iter()
        .map(|entry| entry.map(|(skin, id)| (instance(skin), id.0 as usize)))
        .collect();
    names.name_entities(
        feature,
        &made.body,
        source.tolerance.linear,
        &kept(&made.edges),
        &vertices,
        |_| Some(source.names),
    );
    face_ops::not_through_itself(
        &made.body,
        &names.faces,
        "thickness",
        "layer",
        "too far: ",
        &source.tolerance,
    )?;
    Ok((made.body, Some(names)))
}
