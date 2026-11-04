//! Chamfers in regeneration: edges of the body of an earlier feature are
//! replaced by flat or conical faces. How an edge is referred to and how
//! the result is named is in [`crate::blend`], which fillets share.

use crate::blend::{self, Picked, Roles};
use crate::datum::named_body_of;
use crate::derive::source_body;
use crate::{FeatureResult, Names, angle, length};
use craft3d_doc::{Chamfer, ChamferSize, Document, FeatureId, NameRole};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::chamfer::{self as kernel, ChamferEdge, ChamferFace};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, EdgeId, FaceId};

const ROLES: Roles = Roles {
    face: NameRole::Chamfer,
    corner: NameRole::ChamferCorner,
    noun: "chamfer",
};

/// A kernel error in the words of the document: the reference, the names
/// of the faces, the parameter.
fn explain(
    error: KernelError,
    chamfer: &Chamfer,
    picked: &[Picked],
    names: &Names,
    unit: &str,
) -> String {
    let reference = |edge: EdgeId| picked.iter().find(|p| p.edge == edge);
    let role = |edge: EdgeId| match reference(edge) {
        Some(pick) => match chamfer.edges.get(pick.reference) {
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
        KernelError::ChamferTooLarge {
            edge,
            face,
            distance,
            largest,
        } => {
            let on_first = reference(edge).is_some_and(|p| p.first == face);
            let parameter = match (&chamfer.size, on_first) {
                (ChamferSize::TwoDistances { .. }, false) => "size.second",
                (ChamferSize::DistanceAngle { .. }, false) => "size.angle",
                _ => "size.distance",
            };
            format!(
                "{parameter}: too large for {}: the cut line, {} {unit} from the edge along the face {}, leaves that face; the largest distance that fits there is below {} {unit}",
                role(edge),
                craft3d_kernel::short_number(distance),
                face_name(face),
                craft3d_kernel::fits_number(largest)
            )
        }
        KernelError::Chamfer { edge, face, reason } => {
            let at = edge.map_or_else(|| "edges".to_string(), role);
            match face {
                Some(face) => format!("{at}: {reason} (the face {})", face_name(face)),
                None => format!("{at}: {reason}"),
            }
        }
        KernelError::ChamferBoolean {
            edge,
            error,
            smaller_fails,
        } => format!(
            "{}: the boolean that cuts this chamfer refused it: {error}{}",
            role(edge),
            if smaller_fails {
                "; smaller distances tried fail the same way"
            } else {
                ""
            }
        ),
        other => other.to_string(),
    }
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    chamfer: &Chamfer,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let Some(source_id) = chamfer.source() else {
        return Err("edges: a chamfer needs at least one edge".to_string());
    };
    if source_id == feature {
        return Err("edges: a chamfer cannot be of its own edges".to_string());
    }
    // The feature the faces are of has to be there, earlier, and built:
    // said with the names that are lost with it, as every reference by
    // name says it. Then its body has to be its own still.
    if let Some([first, second]) = chamfer.edges.first() {
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
    let size = match &chamfer.size {
        ChamferSize::Equal { distance } => {
            let d = length(doc, variables, "size.distance", distance)?;
            kernel::ChamferSize::Distances {
                first: d,
                second: d,
            }
        }
        ChamferSize::TwoDistances { distance, second } => kernel::ChamferSize::Distances {
            first: length(doc, variables, "size.distance", distance)?,
            second: length(doc, variables, "size.second", second)?,
        },
        ChamferSize::DistanceAngle {
            distance,
            angle: turn,
        } => kernel::ChamferSize::DistanceAngle {
            distance: length(doc, variables, "size.distance", distance)?,
            angle: angle(doc, variables, "size.angle", turn)?,
        },
    };
    let sizes = match size {
        kernel::ChamferSize::Distances { first, second } => {
            vec![("size.distance", first), ("size.second", second)]
        }
        kernel::ChamferSize::DistanceAngle { distance, angle } => {
            vec![("size.distance", distance), ("size.angle", angle)]
        }
    };
    for (name, value) in sizes {
        if !(value.is_finite() && value > 0.0) {
            return Err(format!("{name}: has to be more than zero, got {value}"));
        }
    }
    let picked = blend::pick(
        &chamfer.edges,
        chamfer.chain,
        ROLES.noun,
        source_id,
        body,
        names,
        &made_by,
    )?;
    let requests: Vec<ChamferEdge> = picked
        .iter()
        .map(|pick| ChamferEdge {
            edge: pick.edge,
            first: pick.first,
            size,
        })
        .collect();
    let unit = doc.units.length.symbol();
    let made = kernel::chamfer_capped(
        body,
        &requests,
        &tolerance,
        Some(doc.tolerance.loosest_linear()),
    ).map_err(|error| match error {
        KernelError::InvalidSize { value, .. } => format!(
            "size: a distance of {value} {unit} is too small to tell from the edge at this document's tolerance"
        ),
        other => explain(other, chamfer, &picked, names, unit),
    })?;

    let faces: Vec<blend::Made<'_>> = made
        .faces
        .iter()
        .map(|face| match face {
            ChamferFace::Kept(faces) => blend::Made::Kept(faces),
            ChamferFace::Chamfer(edges) => blend::Made::Blend(edges),
            ChamferFace::Corner(vertex) => blend::Made::Corner(*vertex),
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
        "size.distance",
    )?;
    Ok((made.body, named))
}
