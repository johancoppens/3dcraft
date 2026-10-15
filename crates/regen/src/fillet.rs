//! Fillets in regeneration: edges of the body of an earlier feature are
//! replaced by faces tangent to both their neighbours, with one radius.
//! How an edge is referred to and how the result is named is in
//! [`crate::blend`], which chamfers share.

use crate::blend::{self, Picked, Roles};
use crate::datum::named_body_of;
use crate::derive::source_body;
use crate::{FeatureResult, Names, length};
use craft3d_doc::{Document, FeatureId, Fillet, NameRole};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::fillet::{self as kernel, FilletEdge, FilletFace};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, EdgeId, FaceId};

const ROLES: Roles = Roles {
    face: NameRole::Fillet,
    corner: NameRole::FilletCorner,
    noun: "fillet",
};

/// A kernel error in the words of the document: the reference, the names
/// of the faces, the parameter.
fn explain(
    error: KernelError,
    fillet: &Fillet,
    picked: &[Picked],
    names: &Names,
    unit: &str,
) -> String {
    let reference = |edge: EdgeId| picked.iter().find(|p| p.edge == edge);
    let role = |edge: EdgeId| match reference(edge) {
        Some(pick) => match fillet.edges.get(pick.reference) {
            Some([a, b]) => format!("edges.{} (between {a} and {b})", pick.reference),
            None => format!("edges.{}", pick.reference),
        },
        None => "edges".to_string(),
    };
    let face_name = |face: FaceId| {
        names
            .faces
            .get(face.0 as usize)
            .map_or_else(|| format!("face {}", face.0), ToString::to_string)
    };
    match error {
        KernelError::FilletTooLarge {
            edge,
            face,
            radius,
            largest,
        } => format!(
            "radius: too large for {}: with a radius of {radius} {unit} the line along which the fillet touches the face {} leaves that face; the largest radius that fits there is below {largest} {unit}",
            role(edge),
            face_name(face)
        ),
        KernelError::Fillet { edge, face, reason } => {
            let at = edge.map_or_else(|| "edges".to_string(), role);
            match face {
                Some(face) => format!("{at}: {reason} (the face {})", face_name(face)),
                None => format!("{at}: {reason}"),
            }
        }
        KernelError::FilletBoolean { edge, error } => format!(
            "{}: the boolean that cuts this fillet refused it: {error}",
            role(edge)
        ),
        other => other.to_string(),
    }
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    fillet: &Fillet,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let Some(source_id) = fillet.source() else {
        return Err("edges: a fillet needs at least one edge".to_string());
    };
    if source_id == feature {
        return Err("edges: a fillet cannot be of its own edges".to_string());
    }
    // The feature the faces are of has to be there, earlier, and built:
    // said with the names that are lost with it, as every reference by
    // name says it. Then its body has to be its own still.
    if let Some([first, second]) = fillet.edges.first() {
        named_body_of(doc, earlier, first, Some(second), "edges.0")?;
    }
    let source = source_body(doc, earlier, source_id, "edges")?;
    let made_by = doc
        .feature(source_id)
        .map_or_else(|| source_id.to_string(), |f| f.name.clone());
    let (Some(body), Some(names)) = (&source.body, &source.names) else {
        return Err(format!(
            "edges: `{made_by}` does not name its faces, so its edges cannot be referred to"
        ));
    };
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let radius = length(doc, variables, "radius", &fillet.radius)?;
    if !(radius.is_finite() && radius > 0.0) {
        return Err(format!("radius: has to be more than zero, got {radius}"));
    }
    let picked = blend::pick(
        &fillet.edges,
        fillet.chain,
        ROLES.noun,
        source_id,
        body,
        names,
        &made_by,
    )?;
    let requests: Vec<FilletEdge> = picked
        .iter()
        .map(|pick| FilletEdge {
            edge: pick.edge,
            radius,
        })
        .collect();
    let unit = doc.units.length.symbol();
    let made = kernel::fillet(body, &requests, &tolerance).map_err(|error| match error {
        KernelError::InvalidSize { value, .. } => format!(
            "radius: a radius of {value} {unit} is too small to tell from the edge at this document's tolerance"
        ),
        other => explain(other, fillet, &picked, names, unit),
    })?;
    let faces: Vec<blend::Made<'_>> = made
        .faces
        .iter()
        .map(|face| match face {
            FilletFace::Kept(faces) => blend::Made::Kept(faces),
            FilletFace::Fillet(edges) => blend::Made::Blend(edges),
            FilletFace::Corner(vertex) => blend::Made::Corner(*vertex),
        })
        .collect();
    let named = blend::name(
        feature,
        ROLES,
        body,
        names,
        &picked,
        &made.body,
        &faces,
        &made.edges,
        &made.vertices,
        tolerance.linear,
    )?;
    blend::not_through_itself(
        &made.body,
        &faces,
        named.as_ref(),
        tolerance.linear,
        ROLES,
        "radius",
    )?;
    Ok((made.body, named))
}
