//! Finding the edges of a body by the faces they lie between.
//!
//! An edge of a solid is where two faces meet (or one face meets itself,
//! along a seam). Faces have persistent names through every feature kind,
//! booleans included, so "the edges between face A and face B" refers to
//! edges in a way that survives upstream edits without the edges having
//! names of their own. Several edges can lie between two faces, and after
//! a boolean several faces can share one name: the answer is a list.

use crate::Names;
use craft3d_doc::PersistentName;
use craft3d_topo::{Body, EdgeId, FaceId};

/// The faces of `body` called `name`: none, one, or several (the pieces a
/// boolean cut a face into).
pub fn faces_named(names: &Names, name: &PersistentName) -> Vec<FaceId> {
    names
        .faces
        .iter()
        .enumerate()
        .filter(|(_, face)| *face == name)
        .filter_map(|(index, _)| u32::try_from(index).ok().map(FaceId))
        .collect()
}

/// For every edge of `body`, the faces that use it, in ascending order
/// without repeats: two for an edge of a solid, one for a seam or for the
/// border of a sheet.
pub fn faces_of_edges(body: &Body) -> Vec<Vec<FaceId>> {
    let mut users: Vec<Vec<FaceId>> = vec![Vec::new(); body.edges.len()];
    for (index, face) in body.faces.iter().enumerate() {
        let Ok(id) = u32::try_from(index).map(FaceId) else {
            continue;
        };
        for coedge in face
            .loops
            .iter()
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges)
        {
            if let Some(list) = users.get_mut(coedge.edge.0 as usize)
                && !list.contains(&id)
            {
                list.push(id);
            }
        }
    }
    users
}

/// The edges of `body` that lie between a face called `a` and a face
/// called `b`, in ascending order. With `a == b`: the edges both of whose
/// sides are faces of that name (a seam, or the cut between two pieces of
/// one face).
pub fn edges_between(
    body: &Body,
    names: &Names,
    a: &PersistentName,
    b: &PersistentName,
) -> Vec<EdgeId> {
    let (first, second) = (faces_named(names, a), faces_named(names, b));
    faces_of_edges(body)
        .iter()
        .enumerate()
        .filter(|(_, users)| {
            let on = |set: &[FaceId]| users.iter().any(|face| set.contains(face));
            let only = |set: &[FaceId]| users.iter().all(|face| set.contains(face));
            if a == b {
                !users.is_empty() && only(&first)
            } else {
                on(&first) && on(&second)
            }
        })
        .filter_map(|(index, _)| u32::try_from(index).ok().map(EdgeId))
        .collect()
}
