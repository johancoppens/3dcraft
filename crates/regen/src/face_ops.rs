//! The face operations in regeneration: what offset face and shell
//! share. Each makes its body from the body of an earlier
//! feature and refers to faces of it by name.
//!
//! **What an operation holds of a face** is its name. The name is found
//! again at every regeneration; the pieces a cut divided a face into
//! share its name, and a name means all of them. A cap's name survives a
//! change of the outline it was named after, as everywhere: it means the
//! caps of that role of that feature. A name that no face
//! answers to any more fails the feature with the parameter, the feature
//! and the name.
//!
//! **Names.** A face of the result is named as a copy of the face it was
//! (`F9:copy(0)/F2:side(e7)`), moved or not, as in every feature that
//! makes its body from another: a face that moved is still that face.
//! The faces of the hollow of a shell are copies too, of the faces they
//! lie behind, in instance 1 (`F9:copy(1)/F2:side(e7)`): instance 0 is
//! the outer skin, instance 1 the inner. Edges and vertices follow the
//! rules of the feature kinds that are like them: after an offset, which
//! changes nothing about how the body hangs together, each is the copy
//! of what it was, as after a move; in a shell they
//! are named by the faces they lie on and answer to a copy of the name
//! they had (docs/architecture.md §4.1).

use crate::datum::{listed, named_body_of};
use crate::derive::source_body;
use crate::query::{faces_named, faces_of_edges};
use crate::sketch::same_plane;
use crate::{FeatureResult, Names};
use craft3d_doc::{Document, FeatureId, PersistentName};
use craft3d_kernel::KernelError;
use craft3d_math::Tolerance;
use craft3d_topo::{Body, EdgeId, FaceId};

/// The body a face operation works on, with what its entities are called.
pub(crate) struct Source<'a> {
    pub body: &'a Body,
    pub names: &'a Names,
    /// The name of the feature that made it.
    pub made_by: String,
    pub tolerance: Tolerance,
}

/// The body of `source`, which the faces of `param` are faces of: there,
/// built, earlier, with named faces, and its own still. `first` is the
/// first face referred to, when there is one: a feature that is gone is
/// said with the name that is lost with it.
pub(crate) fn source<'a>(
    doc: &Document,
    earlier: &'a [FeatureResult],
    feature: FeatureId,
    source_id: FeatureId,
    first: Option<&PersistentName>,
    param: &str,
    noun: &str,
) -> Result<Source<'a>, String> {
    if source_id == feature {
        return Err(format!("{param}: {noun} cannot be of its own body"));
    }
    if let Some(first) = first {
        named_body_of(doc, earlier, first, None, &format!("{param}.0"))?;
    }
    let outcome = source_body(doc, earlier, source_id, param)?;
    let made_by = doc
        .feature(source_id)
        .map_or_else(|| source_id.to_string(), |f| f.name.clone());
    let (Some(body), Some(names)) = (&outcome.body, &outcome.names) else {
        return Err(format!(
            "{param}: `{made_by}` does not name its faces, so they cannot be referred to"
        ));
    };
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    Ok(Source {
        body,
        names,
        made_by,
        tolerance,
    })
}

/// The faces the names `faces` of parameter `param` mean in the body of
/// `source_id`, each once, in the order of the names.
pub(crate) fn pick(
    faces: &[PersistentName],
    param: &str,
    noun: &str,
    source_id: FeatureId,
    source: &Source<'_>,
) -> Result<Vec<FaceId>, String> {
    let made_by = &source.made_by;
    let mut picked: Vec<FaceId> = Vec::new();
    for (reference, name) in faces.iter().enumerate() {
        let role = format!("{param}.{reference}");
        if !name.is_face() {
            return Err(format!("{role}: {name} is not a face"));
        }
        if name.feature != source_id {
            return Err(format!(
                "{role}: {name} is a face of {}, not of `{made_by}`; {noun} takes faces of one body",
                name.feature
            ));
        }
        let mut found = faces_named(source.names, name);
        if found.is_empty() {
            // A reference to a cap survives a change of its outline: the
            // caps of one role of one feature lie in one plane
            // (docs/architecture.md §1.1).
            found = (0..source.names.faces.len())
                .filter(|&face| source.names.names_of(face).any(|own| same_plane(own, name)))
                .filter_map(|face| u32::try_from(face).ok().map(FaceId))
                .collect();
        }
        if found.is_empty() {
            return Err(format!(
                "{role}: `{made_by}` no longer has a face {name}; its faces are: {}",
                listed(&source.names.faces)
            ));
        }
        for face in found {
            if !picked.contains(&face) {
                picked.push(face);
            }
        }
    }
    Ok(picked)
}

/// What a face of the source body is called.
pub(crate) fn face_name(names: &Names, face: FaceId) -> String {
    names
        .faces
        .get(face.0 as usize)
        .map_or_else(|| format!("face {}", face.0), ToString::to_string)
}

/// The entities a kernel error is about, in the names of the document:
/// "the edge between X and Y", "the faces X, Y and Z".
pub(crate) fn about(
    body: &Body,
    names: &Names,
    faces: &[FaceId],
    edge: Option<EdgeId>,
) -> Option<String> {
    let join = |list: &[FaceId]| {
        let named: Vec<String> = list.iter().map(|face| face_name(names, *face)).collect();
        match named.as_slice() {
            [] => None,
            [one] => Some(one.clone()),
            [most @ .., last] => Some(format!("{} and {last}", most.join(", "))),
        }
    };
    if let Some(edge) = edge {
        let users = faces_of_edges(body);
        let between = users.get(edge.0 as usize).map_or(&[][..], Vec::as_slice);
        return Some(match join(between) {
            Some(faces) => format!("the edge between {faces}"),
            None => format!("edge {}", edge.0),
        });
    }
    join(faces).map(|named| {
        if faces.len() == 1 {
            format!("the face {named}")
        } else {
            format!("the faces {named}")
        }
    })
}

/// A kernel error of a face operation in the words of the document:
/// `param` is the parameter that says how far, `unit` its unit, and
/// `sign` whether it was given negative (-1) or positive (1).
pub(crate) fn explain(
    error: KernelError,
    param: &str,
    sign: f64,
    unit: &str,
    source: &Source<'_>,
) -> String {
    match error {
        KernelError::FaceOpTooFar {
            faces,
            edge,
            reason,
            value,
            largest,
            ..
        } => {
            let at = about(source.body, source.names, &faces, edge)
                .map(|at| format!(" ({at})"))
                .unwrap_or_default();
            let (value, largest) = (value * sign, largest * sign);
            let bound = if sign < 0.0 { "above" } else { "below" };
            format!(
                "{param}: {value} {unit} is too far: {reason}{at}; what fits is {bound} {largest} {unit}"
            )
        }
        KernelError::FaceOp {
            faces,
            edge,
            reason,
            ..
        } => match about(source.body, source.names, &faces, edge) {
            Some(at) => format!("faces: {reason} ({at})"),
            None => format!("faces: {reason}"),
        },
        KernelError::FaceOpBoolean { error, .. } => {
            format!("the boolean that makes this refused it: {error}")
        }
        other => other.to_string(),
    }
}

/// Fails when `body` passes through itself: the local operations hold
/// their result against what a body is next to the faces that moved, and
/// two faces far apart that ran into each other are found here, on the
/// mesh. `param` is the parameter to blame, `faces` what the faces of
/// `body` are called, `why` what is said first ("too far: " for an
/// operation that went too far, "" otherwise).
pub(crate) fn not_through_itself(
    body: &Body,
    faces: &[PersistentName],
    param: &str,
    noun: &str,
    why: &str,
    tolerance: &Tolerance,
) -> Result<(), String> {
    use crate::selfcheck::{Crossing, crossing};
    match crossing(body, faces, tolerance.linear) {
        Crossing::None => Ok(()),
        Crossing::Unknown(error) => Err(format!(
            "{param}: the {noun} cannot be meshed to see whether it passes through itself ({error})"
        )),
        touching @ Crossing::Touching(..) => {
            touching.refused(param, noun, "make it smaller, or move what it runs into")
        }
        Crossing::Found(pair) => {
            let between = pair
                .map(|[a, b]| format!(": {a} and {b} cross"))
                .unwrap_or_default();
            Err(format!(
                "{param}: {why}the {noun} passes through itself{between}"
            ))
        }
    }
}
