//! Blends in regeneration: what chamfers and fillets share. Both replace
//! edges of the body of an earlier feature by faces, refer to an edge by
//! the two faces it lies between or by its own name, and name what they
//! make in one way.
//!
//! **What a blend holds of an edge** ([`EdgeRef`]) is the names of the two
//! faces it lies between, or the edge's own name. The names are found
//! again at every regeneration. A pair means every edge between the two
//! faces, so the pieces a later cut divides an edge into stay blended,
//! and a pair whose faces no longer meet fails the feature with the pair.
//! An edge's own name means the one edge that has it (or the alias), and
//! one the body no longer has fails the feature with the name; the face
//! given with it says which side a chamfer's first size is measured on.
//! With `chain` an edge stands for the edges that continue it smoothly as
//! well.
//!
//! **Names.** A face of the body that is still there is named as a copy,
//! instance 0, of the face it was (`F9:copy(0)/F2:side(e7)`), as in every
//! feature that makes its body from another. The face in the place of the
//! edge of reference `k` is `F9:chamfer(k)` or `F9:fillet(k)`; when that
//! reference gave several edges their faces are `F9:fillet(k,0)`,
//! `F9:fillet(k,1)`, in the order of where the edges lie in space (as
//! numbered edge names are, with the same limit: docs/design/names.md). The face in a corner where three blends meet is
//! `F9:chamferCorner(i,j,k)` or `F9:filletCorner(i,j,k)`, by the
//! references of the three edges. Edges and vertices are named by the
//! faces they lie on, and one the body already had answers to a copy of
//! the name it had there too, as after a boolean.

use crate::datum::listed;
use crate::query::{edges_between, faces_named, faces_of_edges};
use crate::selfcheck;
use crate::{Contributors, Kept, Names, entity};
use craft3d_doc::{EdgeRef, FeatureId, NameRole, PersistentName};
use craft3d_kernel::chamfer::tangent_chain;
use craft3d_math::BUG;
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

/// What a blend asks of the face an edge reference gives first.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum First {
    /// Nothing: both sides of the edge are blended alike (a fillet, a
    /// chamfer of one distance).
    Either,
    /// It is the side the first size is measured on (a chamfer of two
    /// distances, or of a distance and an angle): an edge given by its own
    /// name has to say which side that is.
    Measured,
}

/// The faces `name` names in `body`, refused with the reason when they
/// are none, several caps a short name cannot choose between, or not of
/// the body the blend takes.
fn checked_face(
    name: &PersistentName,
    role: &str,
    noun: &str,
    source: FeatureId,
    names: &Names,
    made_by: &str,
) -> Result<Vec<FaceId>, String> {
    if name.feature != source {
        return Err(format!(
            "{role}: {name} is a face of {}, and the first edge is of `{made_by}`; one {noun} takes edges of one body",
            name.feature
        ));
    }
    if let Some(why) = crate::query::ambiguous_cap(names, name) {
        return Err(format!("{role}: {why}"));
    }
    Ok(faces_named(names, name))
}

/// The one edge of `body` the reference by its own name `edge` means,
/// with the face of it that answers to `first` (or, when none is given
/// and `measured` does not ask for one, the first of its faces).
#[allow(clippy::too_many_arguments)]
fn named_edge(
    edge: &PersistentName,
    first: Option<&PersistentName>,
    measured: First,
    role: &str,
    noun: &str,
    source: FeatureId,
    names: &Names,
    users: &[Vec<FaceId>],
    made_by: &str,
) -> Result<(EdgeId, FaceId), String> {
    if !edge.is_edge() {
        return Err(format!(
            "{role}: {edge} is not an edge; an edge is referred to by its own name (document.edges lists it under name) or by the two faces it lies between"
        ));
    }
    if edge.feature != source {
        return Err(format!(
            "{role}: {edge} is an edge of {}, and the first edge is of `{made_by}`; one {noun} takes edges of one body",
            edge.feature
        ));
    }
    let Some(index) = names.edge_named(edge) else {
        return Err(format!(
            "{role}: `{made_by}` has no edge {edge} (any more); document.edges lists its edges"
        ));
    };
    let (Ok(id), Some(sides)) = (u32::try_from(index), users.get(index)) else {
        return Err(format!(
            "{BUG}: {role}: the edge {edge} of `{made_by}` is not in its body"
        ));
    };
    let face_list = || {
        sides
            .iter()
            .filter_map(|face| names.faces.get(face.0 as usize))
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let first_face = match first {
        Some(face) => {
            if !face.is_face() {
                return Err(format!(
                    "{role}: {face} is not a face; after an edge's name comes the face of it the first distance is measured along"
                ));
            }
            let named = checked_face(face, role, noun, source, names, made_by)?;
            let Some(side) = sides.iter().copied().find(|side| named.contains(side)) else {
                return Err(format!(
                    "{role}: {face} is not a face of the edge {edge}; its faces are: {}",
                    face_list()
                ));
            };
            side
        }
        None if measured == First::Measured => {
            return Err(format!(
                "{role}: a {noun} whose two sides differ measures its first size along one face of the edge: give the edge with that face, [\"{edge}\", face] (its faces: {})",
                face_list()
            ));
        }
        None => {
            let Some(side) = sides.first().copied() else {
                return Err(format!(
                    "{role}: the edge {edge} of `{made_by}` lies on no face"
                ));
            };
            side
        }
    };
    Ok((EdgeId(id), first_face))
}

/// The edges the pair of faces `[a, b]` means in `body` (every edge
/// between them), each with its face that answers to `a`.
#[allow(clippy::too_many_arguments)]
fn edges_of_pair(
    a: &PersistentName,
    b: &PersistentName,
    role: &str,
    noun: &str,
    source: FeatureId,
    body: &Body,
    names: &Names,
    users: &[Vec<FaceId>],
    made_by: &str,
) -> Result<Vec<(EdgeId, FaceId)>, String> {
    for face in [a, b] {
        if !face.is_face() {
            return Err(format!(
                "{role}: {face} is not a face; an edge is referred to by the two faces it lies between, or by its own name alone (or followed by the face its first distance is measured along)"
            ));
        }
    }
    let first = checked_face(a, role, noun, source, names, made_by)?;
    let second = checked_face(b, role, noun, source, names, made_by)?;
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
    Ok(found
        .into_iter()
        .filter_map(|edge| {
            // The face of the edge that answers to the first name.
            let first_face = users
                .get(edge.0 as usize)
                .into_iter()
                .flatten()
                .copied()
                .find(|face| first.contains(face))?;
            Some((edge, first_face))
        })
        .collect())
}

/// The edges the references `edges` of a blend mean in `body`; with
/// `chain` each with the edges that continue it smoothly. `noun` is the
/// kind of blend, in the words of what is refused; `measured` says
/// whether the face a reference gives first is the side of a size.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pick(
    edges: &[EdgeRef],
    chain: bool,
    noun: &str,
    measured: First,
    source: FeatureId,
    body: &Body,
    names: &Names,
    made_by: &str,
) -> Result<Vec<Picked>, String> {
    let users = faces_of_edges(body);
    let mut picked: Vec<Picked> = Vec::new();
    for (reference, edge_ref) in edges.iter().enumerate() {
        let role = format!("edges.{reference}");
        let found = match edge_ref {
            EdgeRef::Faces([a, b]) => {
                edges_of_pair(a, b, &role, noun, source, body, names, &users, made_by)?
            }
            EdgeRef::Edge { edge, first } => vec![named_edge(
                edge,
                first.as_ref(),
                measured,
                &role,
                noun,
                source,
                names,
                &users,
                made_by,
            )?],
        };
        for (edge, first_face) in found {
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
                            "{role}: {} is the edge of edges.{} too",
                            edge_ref.describe(),
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

/// The reference at `index` of `edges` as reasons name it:
/// `edges.0 (between a and b)`, `edges.1 (the edge e)`.
pub(crate) fn role_of(edges: &[EdgeRef], index: usize) -> String {
    match edges.get(index) {
        Some(EdgeRef::Faces([a, b])) => format!("edges.{index} (between {a} and {b})"),
        Some(EdgeRef::Edge { edge, .. }) => format!("edges.{index} (the edge {edge})"),
        None => format!("edges.{index}"),
    }
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
                    return Err(format!("{BUG}: the {} lost the name of a face", roles.noun));
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
                    return Err(format!("{BUG}: the {} lost the edge of a face", roles.noun));
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
    )?;
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
