//! Blends in regeneration: what chamfers and fillets share. Both replace
//! edges of the body of an earlier feature by faces, refer to an edge by
//! the two faces it lies between, and name what they make in one way.
//!
//! **What a blend holds of an edge** is the names of the two faces it
//! lies between. The names are found again at every regeneration; such a
//! pair means every edge between the two faces, so the pieces a later cut
//! divides an edge into stay blended, and a pair whose faces no longer
//! meet fails the feature with the pair. With `chain` an edge stands for
//! the edges that continue it smoothly as well.
//!
//! **Names.** A face of the body that is still there is named as a copy,
//! instance 0, of the face it was (`F9:copy(0)/F2:side(e7)`), as in every
//! feature that makes its body from another. The face in the place of the
//! edge of reference `k` is `F9:chamfer(k)` or `F9:fillet(k)`; when that
//! reference gave several edges their faces are `F9:fillet(k,0)`,
//! `F9:fillet(k,1)`, in the order of where the edges lie in space (as
//! numbered edge names are, with the same limit: docs/architecture.md
//! §4.1). The face in a corner where three blends meet is
//! `F9:chamferCorner(i,j,k)` or `F9:filletCorner(i,j,k)`, by the
//! references of the three edges. Edges and vertices are named by the
//! faces they lie on, and one the body already had answers to a copy of
//! the name it had there too, as after a boolean.

use crate::datum::listed;
use crate::query::{edges_between, faces_named, faces_of_edges};
use crate::selfcheck;
use crate::{Contributors, Kept, Names, entity};
use craft3d_doc::{FeatureId, NameRole, PersistentName};
use craft3d_kernel::chamfer::tangent_chain;
use craft3d_math::Vec3;
use craft3d_tess::FaceRole;
use craft3d_topo::{Body, EdgeId, FaceId, VertexId};
use std::collections::BTreeMap;

/// One edge of the body to blend and the reference it came from.
pub(crate) struct Picked {
    pub edge: EdgeId,
    /// The face of the edge that answers to the first name of the pair.
    pub first: FaceId,
    /// The place of its reference in the feature's list.
    pub reference: usize,
}

/// The edges the references `edges` of a blend mean in `body`; with
/// `chain` each with the edges that continue it smoothly. `noun` is the
/// kind of blend, in the words of what is refused.
pub(crate) fn pick(
    edges: &[[PersistentName; 2]],
    chain: bool,
    noun: &str,
    source: FeatureId,
    body: &Body,
    names: &Names,
    made_by: &str,
) -> Result<Vec<Picked>, String> {
    let users = faces_of_edges(body);
    let mut picked: Vec<Picked> = Vec::new();
    for (reference, [a, b]) in edges.iter().enumerate() {
        let role = format!("edges.{reference}");
        for face in [a, b] {
            if !face.is_face() {
                return Err(format!(
                    "{role}: {face} is not a face; an edge is referred to by the two faces it lies between"
                ));
            }
            if face.feature != source {
                return Err(format!(
                    "{role}: {face} is a face of {}, and the first edge is of `{made_by}`; one {noun} takes edges of one body",
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
                "{role}: {a} and {b} are one face; an edge to {noun} lies between two"
            ));
        }
        let found = edges_between(body, names, a, b);
        if found.is_empty() {
            return Err(format!(
                "{role}: the faces {a} and {b} of `{made_by}` do not meet in an edge (any more)"
            ));
        }
        for edge in found {
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
            let members = if chain {
                tangent_chain(body, edge, first_face)
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
                    Some(earlier) if chain => {
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

/// Per picked edge, the numbers of its blend face's name: its
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

/// Where a face of a blended body came from, as the kernel says it.
pub(crate) enum Made<'a> {
    /// Faces of the body before.
    Kept(&'a [FaceId]),
    /// The blend face of these picked edges.
    Blend(&'a [usize]),
    /// The corner at this vertex of the body before.
    Corner(VertexId),
}

/// The roles and the word of one kind of blend.
#[derive(Clone, Copy)]
pub(crate) struct Roles {
    pub face: NameRole,
    pub corner: NameRole,
    pub noun: &'static str,
}

/// The names of the blended body `made`: `faces`, `edges` and `vertices`
/// say where each of its entities came from in `body`, which `names`
/// names. `None` when the body's faces could not be named as copies.
#[allow(clippy::too_many_arguments)]
pub(crate) fn name(
    feature: FeatureId,
    roles: Roles,
    body: &Body,
    names: &Names,
    picked: &[Picked],
    made: &Body,
    faces: &[Made<'_>],
    edges: &[Option<EdgeId>],
    vertices: &[Option<VertexId>],
    tolerance: f64,
) -> Result<Option<Names>, String> {
    // The faces that are still the body's, named as copies of what they
    // were; then the new faces among them.
    let kept: Contributors = faces
        .iter()
        .filter_map(|face| match face {
            Made::Kept(faces) => Some(faces.iter().map(|f| (0_u64, f.0 as usize)).collect()),
            Made::Blend(_) | Made::Corner(_) => None,
        })
        .collect();
    let Some(copies) = Names::copied_faces(feature, &kept, |_| Some(names))? else {
        return Ok(None);
    };
    let numbered = numbers(body, picked, tolerance);
    let blend_name = |index: usize| {
        numbered
            .get(index)
            .map(|sources| PersistentName::new(feature, roles.face, sources.clone()))
    };
    let mut named = Names::default();
    let mut next_copy = 0_usize;
    for (index, face) in faces.iter().enumerate() {
        match face {
            Made::Kept(_) => {
                let Some(name) = copies.faces.get(next_copy) else {
                    return Err(format!(
                        "the {} lost the name of a face; this is a bug",
                        roles.noun
                    ));
                };
                named.faces.push(name.clone());
                let aliases = copies.aliases_of(next_copy);
                if !aliases.is_empty() {
                    named.aliases.insert(index, aliases.to_vec());
                }
                next_copy += 1;
            }
            Made::Blend(edges) => {
                let mut all = edges.iter().filter_map(|i| blend_name(*i));
                let Some(own) = all.next() else {
                    return Err(format!(
                        "the {} lost the edge of a face; this is a bug",
                        roles.noun
                    ));
                };
                named.faces.push(own);
                let aliases: Vec<PersistentName> = all.collect();
                if !aliases.is_empty() {
                    named.aliases.insert(index, aliases);
                }
            }
            Made::Corner(vertex) => {
                // By the references of the blended edges that end there.
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
                named
                    .faces
                    .push(PersistentName::new(feature, roles.corner, references));
            }
        }
    }
    let kept_of = |list: &[Option<u32>]| -> Kept {
        list.iter()
            .map(|id| id.map(|id| (0_u64, id as usize)))
            .collect()
    };
    let edges: Vec<Option<u32>> = edges.iter().map(|e| e.map(|e| e.0)).collect();
    let vertices: Vec<Option<u32>> = vertices.iter().map(|v| v.map(|v| v.0)).collect();
    named.name_entities(
        feature,
        made,
        tolerance,
        &kept_of(&edges),
        &kept_of(&vertices),
        |_| Some(names),
    );
    Ok(Some(named))
}

/// Fails when the blended body `made` passes through itself. A blend is
/// made by booleans of the body with tools, so a face kept from the body
/// is a piece of a face of a body that does not pass through itself,
/// unless it was merged from several or lies next to a new face (it may
/// have taken a tool's face that lies in its plane: a wall the end of a
/// chamfer runs into). The new faces, those, and what is near them are
/// looked at (`selfcheck::crossing_near`). That includes a blend whose
/// face has no closed form (a spline surface along a freeform edge,
/// `kernel::blend`'s `rolling`), which the kernel holds to the faces next
/// to the edge only: whether it runs into a face further away (a radius
/// too large for what lies across the body) is found here.
pub(crate) fn not_through_itself(
    made: &Body,
    faces: &[Made<'_>],
    names: Option<&Names>,
    linear: f64,
    roles: Roles,
    param: &str,
) -> Result<(), String> {
    let mut kinds: Vec<FaceRole> = faces
        .iter()
        .map(|face| match face {
            Made::Kept([_]) => FaceRole::Known(0),
            Made::Kept(_) | Made::Blend(_) | Made::Corner(_) => FaceRole::New,
        })
        .collect();
    selfcheck::with_neighbours(made, &mut kinds);
    let named = names.map_or(&[][..], |names| names.faces.as_slice());
    selfcheck::crossing_near(made, named, linear, &kinds).refused(
        param,
        &format!("{}ed body", roles.noun),
        &format!(
            "a {} runs into a face it does not border; make it smaller",
            roles.noun
        ),
    )
}
