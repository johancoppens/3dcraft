//! Finding the edges of a body by the faces they lie between.
//!
//! An edge of a solid is where two faces meet (or one face meets itself,
//! along a seam). Faces have persistent names through every feature kind,
//! booleans included, so "the edges between face A and face B" refers to
//! edges in a way that survives upstream edits. Several edges can lie
//! between two faces, and after a boolean several faces can share one
//! name: the answer is a list. (One edge of that list is told from the
//! others by its own name, [`crate::Names::edge_name`], which after a boolean
//! is made of the same two faces: `crate::entity`.)

use crate::Names;
use craft3d_doc::PersistentName;
use craft3d_topo::{Body, EdgeId, FaceId};

/// The faces of `body` called `name`, by their own name or an alias (the
/// name of another face a boolean merged into it): none, one, or several
/// (the pieces a boolean cut a face into).
pub fn faces_named(names: &Names, name: &PersistentName) -> Vec<FaceId> {
    (0..names.faces.len())
        .filter(|&face| names.face_answers_to(face, name))
        .filter_map(|index| u32::try_from(index).ok().map(FaceId))
        .collect()
}

/// For every edge of `body`, the faces that use it, in ascending order
/// without repeats: two for an edge of a solid, one for a seam or for the
/// border of a sheet.
pub fn faces_of_edges(body: &Body) -> Vec<Vec<FaceId>> {
    let mut users = craft3d_topo::edge_faces(body);
    // Each list is in face order, so a face's repeats are next to each
    // other.
    for list in &mut users {
        list.dedup();
    }
    users
}

/// The edges of `body` that lie between a face called `a` and a face
/// called `b`, in ascending order. When both names name the same faces
/// (`a == b`, or an alias and the face's own name): the edges both of
/// whose sides are faces of that name (a seam, or the cut between two
/// pieces of one face).
pub fn edges_between(
    body: &Body,
    names: &Names,
    a: &PersistentName,
    b: &PersistentName,
) -> Vec<EdgeId> {
    let (first, second) = (faces_named(names, a), faces_named(names, b));
    let same = first == second;
    faces_of_edges(body)
        .iter()
        .enumerate()
        .filter(|(_, users)| {
            let on = |set: &[FaceId]| users.iter().any(|face| set.contains(face));
            let only = |set: &[FaceId]| users.iter().all(|face| set.contains(face));
            if same {
                !users.is_empty() && only(&first)
            } else {
                on(&first) && on(&second)
            }
        })
        .filter_map(|(index, _)| u32::try_from(index).ok().map(EdgeId))
        .collect()
}
