//! Chamfers in regeneration: edges of the body of an earlier feature are
//! replaced by faces.
//!
//! **What a chamfer holds of an edge** is the names of the two faces it
//! lies between, in the order that says which face the first distance is
//! measured along. The names are found again at every regeneration; such
//! a pair means every edge between the two faces, so the pieces a later
//! cut divides an edge into stay chamfered, and a pair whose faces no
//! longer meet fails the feature with the pair. With `chain` an edge
//! stands for the edges that continue it smoothly as well.
//!
//! **Names.** A face of the body that is still there is named as a copy,
//! instance 0, of the face it was (`F9:copy(0)/F2:side(e7)`), as in every
//! feature that makes its body from another. The face in the place of the
//! edge of reference `k` is `F9:chamfer(k)`; when that reference gave
//! several edges their faces are `F9:chamfer(k,0)`, `F9:chamfer(k,1)`, in
//! the order of where the edges lie in space (as numbered edge names are,
//! with the same limit: docs/architecture.md §4.1). The flat face where
//! three chamfers meet is `F9:chamferCorner(i,j,k)`, by the references of
//! the three edges. Edges and vertices are named by the faces they lie
//! on, and one the body already had answers to a copy of the name it had
//! there too, as after a boolean.

use crate::datum::{listed, named_body_of};
use crate::derive::source_body;
use crate::query::{edges_between, faces_named, faces_of_edges};
use crate::{Contributors, FeatureResult, Kept, Names, angle, entity, length};
use craft3d_doc::{Chamfer, ChamferSize, Document, FeatureId, NameRole, PersistentName};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::chamfer::{self as kernel, ChamferEdge, ChamferFace};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId};
use std::collections::BTreeMap;

/// One edge of the body to chamfer and the reference it came from.
struct Picked {
    edge: EdgeId,
    first: FaceId,
    /// The place of its reference in the chamfer's list.
    reference: usize,
}

/// The edges the references of `chamfer` mean in `body`.
fn pick(
    chamfer: &Chamfer,
    source: FeatureId,
    body: &Body,
    names: &Names,
    made_by: &str,
) -> Result<Vec<Picked>, String> {
    let users = faces_of_edges(body);
    let mut picked: Vec<Picked> = Vec::new();
    for (reference, [a, b]) in chamfer.edges.iter().enumerate() {
        let role = format!("edges.{reference}");
        for face in [a, b] {
            if !face.is_face() {
                return Err(format!(
                    "{role}: {face} is not a face; an edge is referred to by the two faces it lies between"
                ));
            }
            if face.feature != source {
                return Err(format!(
                    "{role}: {face} is a face of {}, and the first edge is of `{made_by}`; one chamfer takes edges of one body",
                    face.feature
                ));
            }
        }
        let (first, second) = (faces_named(names, a), faces_named(names, b));
        // A reference of two names says every one of them that is gone.
        let gone: Vec<String> = [(a, &first), (b, &second)]
            .into_iter()
            .filter(|(_, found)| found.is_empty())
            .map(|(face, _)| format!("a face {face}"))
            .collect();
        if !gone.is_empty() {
            return Err(format!(
                "{role}: `{made_by}` no longer has {}; its faces are: {}",
                gone.join(", nor "),
                listed(&names.faces)
            ));
        }
        if first == second {
            return Err(format!(
                "{role}: {a} and {b} are one face; an edge to chamfer lies between two"
            ));
        }
        let edges = edges_between(body, names, a, b);
        if edges.is_empty() {
            return Err(format!(
                "{role}: the faces {a} and {b} of `{made_by}` do not meet in an edge (any more)"
            ));
        }
        for edge in edges {
            // The face of the edge that answers to the first name.
            let Some(first_face) = users
                .get(edge.0 as usize)
                .into_iter()
                .flatten()
                .copied()
                .find(|face| first.contains(face))
            else {
                continue;
            };
            let members = if chamfer.chain {
                kernel::tangent_chain(body, edge, first_face)
                    .map_err(|e| format!("{role}: {e}"))?
                    .unwrap_or_else(|| vec![(edge, first_face)])
            } else {
                vec![(edge, first_face)]
            };
            for (member, member_first) in members {
                match picked.iter().find(|p| p.edge == member) {
                    // Reached again along its own chain, or between
                    // pieces of the same two faces.
                    Some(earlier) if earlier.reference == reference => {}
                    Some(earlier) if chamfer.chain => {
                        // Two references on one chain: the first has it.
                        let _ = earlier;
                    }
                    Some(earlier) => {
                        return Err(format!(
                            "{role}: the edge between {a} and {b} is the edge of edges.{} too",
                            earlier.reference
                        ));
                    }
                    None => picked.push(Picked {
                        edge: member,
                        first: member_first,
                        reference,
                    }),
                }
            }
        }
    }
    Ok(picked)
}

/// Per picked edge, the numbers of its chamfer face's name: its
/// reference, and which of that reference's edges it is when there are
/// several, in the order of where they lie in space.
fn numbers(body: &Body, picked: &[Picked], tolerance: f64) -> Vec<Vec<u64>> {
    let mut by_reference: BTreeMap<usize, Vec<(usize, Vec3)>> = BTreeMap::new();
    for (index, pick) in picked.iter().enumerate() {
        let middle = body
            .edge(pick.edge)
            .map(|e| e.curve.eval(0.5 * (e.range.0 + e.range.1)))
            .unwrap_or(Vec3::ZERO);
        by_reference
            .entry(pick.reference)
            .or_default()
            .push((index, middle));
    }
    let mut out = vec![Vec::new(); picked.len()];
    for (reference, mut edges) in by_reference {
        let several = edges.len() > 1;
        entity::sort_in_space(&mut edges, tolerance, 0);
        for (place, (index, _)) in edges.into_iter().enumerate() {
            if let Some(slot) = out.get_mut(index) {
                *slot = if several {
                    vec![reference as u64, place as u64]
                } else {
                    vec![reference as u64]
                };
            }
        }
    }
    out
}

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
                "{parameter}: too large for {}: the cut line, {distance} {unit} from the edge along the face {}, leaves that face; the largest distance that fits there is below {largest} {unit}",
                role(edge),
                face_name(face)
            )
        }
        KernelError::Chamfer { edge, face, reason } => {
            let at = edge.map_or_else(|| "edges".to_string(), role);
            match face {
                Some(face) => format!("{at}: {reason} (the face {})", face_name(face)),
                None => format!("{at}: {reason}"),
            }
        }
        KernelError::ChamferBoolean { edge, error } => format!(
            "{}: the boolean that cuts this chamfer refused it: {error}",
            role(edge)
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
    let picked = pick(chamfer, source_id, body, names, &made_by)?;
    let requests: Vec<ChamferEdge> = picked
        .iter()
        .map(|pick| ChamferEdge {
            edge: pick.edge,
            first: pick.first,
            size,
        })
        .collect();
    let unit = doc.units.length.symbol();
    let made = kernel::chamfer(body, &requests, &tolerance).map_err(|error| match error {
        KernelError::InvalidSize { value, .. } => format!(
            "size: a distance of {value} {unit} is too small to tell from the edge at this document's tolerance"
        ),
        other => explain(other, chamfer, &picked, names, unit),
    })?;

    // The faces that are still the body's, named as copies of what they
    // were; then the new faces among them.
    let kept: Contributors = made
        .faces
        .iter()
        .filter_map(|face| match face {
            ChamferFace::Kept(faces) => Some(faces.iter().map(|f| (0_u64, f.0 as usize)).collect()),
            ChamferFace::Chamfer(_) | ChamferFace::Corner(_) => None,
        })
        .collect();
    let Some(copies) = Names::copied_faces(feature, &kept, |_| Some(names.as_ref()))? else {
        return Ok((made.body, None));
    };
    let numbered = numbers(body, &picked, tolerance.linear);
    let chamfer_name = |index: usize| {
        numbered
            .get(index)
            .map(|sources| PersistentName::new(feature, NameRole::Chamfer, sources.clone()))
    };
    let mut named = Names::default();
    let mut next_copy = 0_usize;
    for (index, face) in made.faces.iter().enumerate() {
        match face {
            ChamferFace::Kept(_) => {
                let Some(name) = copies.faces.get(next_copy) else {
                    return Err("the chamfer lost the name of a face; this is a bug".to_string());
                };
                named.faces.push(name.clone());
                let aliases = copies.aliases_of(next_copy);
                if !aliases.is_empty() {
                    named.aliases.insert(index, aliases.to_vec());
                }
                next_copy += 1;
            }
            ChamferFace::Chamfer(edges) => {
                let mut all = edges.iter().filter_map(|i| chamfer_name(*i));
                let Some(own) = all.next() else {
                    return Err("the chamfer lost the edge of a face; this is a bug".to_string());
                };
                named.faces.push(own);
                let aliases: Vec<PersistentName> = all.collect();
                if !aliases.is_empty() {
                    named.aliases.insert(index, aliases);
                }
            }
            ChamferFace::Corner(vertex) => {
                // By the references of the chamfered edges that end there.
                let mut references: Vec<u64> = picked
                    .iter()
                    .filter(|pick| {
                        body.edge(pick.edge)
                            .is_some_and(|e| e.start == *vertex || e.end == *vertex)
                    })
                    .map(|pick| pick.reference as u64)
                    .collect();
                references.sort_unstable();
                references.dedup();
                named.faces.push(PersistentName::new(
                    feature,
                    NameRole::ChamferCorner,
                    references,
                ));
            }
        }
    }
    let kept_of = |list: &[Option<u32>]| -> Kept {
        list.iter()
            .map(|id| id.map(|id| (0_u64, id as usize)))
            .collect()
    };
    let edges: Vec<Option<u32>> = made.edges.iter().map(|e| e.map(|e| e.0)).collect();
    let vertices: Vec<Option<u32>> = made.vertices.iter().map(|v| v.map(|v| v.0)).collect();
    named.name_entities(
        feature,
        &made.body,
        tolerance.linear,
        &kept_of(&edges),
        &kept_of(&vertices),
        |_| Some(names.as_ref()),
    );
    Ok((made.body, Some(named)))
}
