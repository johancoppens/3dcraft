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
//! they had (docs/design/names.md).

use crate::datum::{listed, named_body_of};
use crate::derive::source_body;
use crate::query::{faces_named, faces_of_edges};
use crate::sketch::same_plane;
use crate::{FeatureResult, Names};
use craft3d_doc::{Document, FeatureId, PersistentName};
use craft3d_kernel::KernelError;
use craft3d_math::Tolerance;
use craft3d_topo::{Body, EdgeId, FaceId};
use std::sync::Arc;

/// The body a face operation works on, with what its entities are called.
pub(crate) struct Source<'a> {
    pub body: &'a Body,
    pub names: &'a Arc<Names>,
    /// The name of the feature that made it.
    pub made_by: String,
    pub tolerance: Tolerance,
    /// The document's cap on any entity's own tolerance.
    pub loosest: f64,
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
        loosest: doc.tolerance.loosest_linear(),
    })
}

/// What the names of the faces of `source` say of how they were made:
/// per face whose name goes back to a fillet's face (`F3:fillet(1)`, or a
/// copy of one), the radius that fillet was made with, so that a face
/// operation that moves the face rebuilds the fillet between the faces
/// it rolls along (`kernel::offset_face::Blends`). The kernel measures
/// the radius on the face too and takes this one only when the two
/// agree; a radius that cannot be had is left out.
pub(crate) fn blends(
    doc: &Document,
    variables: &craft3d_expr::Evaluation,
    source: &Source<'_>,
) -> craft3d_kernel::offset_face::Blends {
    use craft3d_doc::{FeatureKind, NameRole};
    let mut radii = std::collections::BTreeMap::new();
    for (index, name) in source.names.faces.iter().enumerate() {
        let root = name.root();
        if root.role != NameRole::Fillet {
            continue;
        }
        let Some(FeatureKind::Fillet(made)) = doc.feature(root.feature).map(|f| &f.kind) else {
            continue;
        };
        let Ok(radius) = crate::length(doc, variables, "radius", &made.radius) else {
            continue;
        };
        if let (true, Ok(face)) = (radius.is_finite() && radius > 0.0, u32::try_from(index)) {
            radii.insert(FaceId(face), radius);
        }
    }
    craft3d_kernel::offset_face::Blends { radii }
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
        if let Some(why) = crate::query::ambiguous_cap(source.names, name) {
            return Err(format!("{role}: {why}"));
        }
        let mut found = faces_named(source.names, name);
        if found.is_empty() {
            // A reference to a cap survives a change of its outline: the
            // caps of one role of one feature lie in one plane
            // (docs/design/names.md).
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

/// The largest value that fits, for reading: six significant digits,
/// cut towards zero so what is shown still fits (4.99999 rather than
/// 4.999996662139893).
pub(crate) fn fits_shown(largest: f64) -> f64 {
    if !largest.is_finite() || largest == 0.0 {
        return largest;
    }
    let size = largest.abs();
    let mut scale = 1.0_f64;
    // Bounded: a finite f64 is within 10^±308.
    for _ in 0..700 {
        if size * scale < 1e5 {
            scale *= 10.0;
        } else if size * scale >= 1e6 {
            scale /= 10.0;
        } else {
            break;
        }
    }
    let shown = (largest * scale).trunc() / scale;
    if shown.is_finite() { shown } else { largest }
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
            let (value, largest) = (value * sign, fits_shown(largest) * sign);
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
    use crate::selfcheck::crossing;
    said(crossing(body, faces, tolerance.linear), param, noun, why)
}

/// [`not_through_itself`] near what an operation that keeps every entity
/// changed (move face, draft): `made` has the faces, edges and vertices
/// of `before` under the same numbers, and a face whose surface or one
/// of whose vertices moved is new; every other face is unchanged, a
/// piece of `before`, which does not pass through itself
/// (`selfcheck::crossing_near`).
pub(crate) fn not_through_itself_where_changed(
    before: &Body,
    made: &Body,
    faces: &[PersistentName],
    param: &str,
    noun: &str,
    why: &str,
    tolerance: &Tolerance,
) -> Result<(), String> {
    use crate::selfcheck::crossing_near;
    use craft3d_tess::FaceRole;
    let moved: Vec<bool> = made
        .vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| {
            before
                .vertices
                .get(index)
                .is_none_or(|was| was.point != vertex.point)
        })
        .collect();
    let roles: Vec<FaceRole> = made
        .faces
        .iter()
        .enumerate()
        .map(|(index, face)| {
            let same_surface = before
                .faces
                .get(index)
                .is_some_and(|was| was.surface == face.surface);
            let vertex_moved = face
                .loops
                .iter()
                .filter_map(|l| made.loop_(*l))
                .flat_map(|l| &l.coedges)
                .filter_map(|c| made.edge(c.edge))
                .flat_map(|e| [e.start, e.end])
                .any(|v| moved.get(v.0 as usize).copied().unwrap_or(true));
            if same_surface && !vertex_moved {
                FaceRole::Known(0)
            } else {
                FaceRole::New
            }
        })
        .collect();
    said(
        crossing_near(made, faces, tolerance.linear, &roles),
        param,
        noun,
        why,
    )
}

/// What a look for a crossing found, in the words of
/// [`not_through_itself`].
fn said(
    found: crate::selfcheck::Crossing,
    param: &str,
    noun: &str,
    why: &str,
) -> Result<(), String> {
    use crate::selfcheck::Crossing;
    match found {
        Crossing::None => Ok(()),
        Crossing::Spent => Err(Crossing::spent(param, noun)),
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

#[cfg(test)]
mod tests {
    use super::fits_shown;

    #[test]
    fn what_fits_is_shown_in_six_digits_and_still_fits() {
        assert_eq!(fits_shown(4.999_996_662_139_893).to_string(), "4.99999");
        assert_eq!(fits_shown(10.0).to_string(), "10");
        assert_eq!(fits_shown(0.833_333_333_3).to_string(), "0.833333");
        assert_eq!(fits_shown(44.999_999_87).to_string(), "44.9999");
        assert!(fits_shown(123_456_789.9) <= 123_456_789.9);
        assert!(fits_shown(1.234_567_89e-7) <= 1.234_567_89e-7);
        for odd in [0.0, f64::NAN, f64::INFINITY, 5e-324, f64::MAX] {
            let shown = fits_shown(odd);
            assert!(shown.is_nan() == odd.is_nan());
            assert!(shown.is_nan() || shown.abs() <= odd.abs());
        }
    }
}
