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

/// Why a short cap (`F2:endCap()`, [`PersistentName::shortens`]) cannot
/// name a face of this body: it answers to the caps of more than one
/// loop (a sketch of several profiles extruded at once, or one that got a
/// second profile after the reference was made). `None` for any other
/// name, and for a short cap of one loop (its pieces after a cut are one
/// cap). A feature does not take every cap then: that would make a
/// reference silently take more faces after an upstream edit (rule 4).
pub fn ambiguous_cap(names: &Names, name: &PersistentName) -> Option<String> {
    if !name.is_short_cap() {
        return None;
    }
    let mut loops: Vec<&PersistentName> = Vec::new();
    for face in 0..names.faces.len() {
        if names.face_answers_to(face, name)
            && let Some(own) = names.faces.get(face)
            && !loops.contains(&own)
        {
            loops.push(own);
        }
    }
    if loops.len() < 2 {
        return None;
    }
    let listed: Vec<String> = loops.iter().take(12).map(ToString::to_string).collect();
    Some(format!(
        "{name} names the caps of {} loops ({}{}); give the full name of the one meant",
        loops.len(),
        listed.join(", "),
        if loops.len() > listed.len() {
            ", ..."
        } else {
            ""
        }
    ))
}

/// Why a chamfer or a fillet (`noun`) does not take `name`, an edge's name
/// numbered by place ([`PersistentName::is_numbered_by_place`]), and what
/// names that edge instead: another name of it that is not numbered (an
/// alias, which it kept from a body before), or the pair of its faces
/// where they meet in that edge only; when nothing does, it says so.
/// `at` is the body the name is of and its names, when they are there.
pub fn numbered_edge(name: &PersistentName, noun: &str, at: Option<(&Body, &Names)>) -> String {
    let why = format!(
        "{name} tells its edge from the other edges between the same faces only by where it lies in space (the number at its end), which an edit can move to another edge without any failure, so a {noun} does not take it"
    );
    let Some((body, names)) = at else {
        return format!(
            "{why}; document.edges lists the names of the body's edges and their aliases"
        );
    };
    let Some(index) = names.edge_named(name) else {
        return format!(
            "{why}; document.edges lists the names of the body's edges and their aliases"
        );
    };
    let other: Vec<String> = names
        .edge_names_of(index)
        .into_iter()
        .filter(|alias| alias != name && !alias.is_numbered_by_place())
        .map(|alias| format!("\"{alias}\""))
        .collect();
    if !other.is_empty() {
        return format!(
            "{why}; the same edge is also called {}: give that name",
            other.join(", ")
        );
    }
    let sides: Vec<&PersistentName> = faces_of_edges(body)
        .get(index)
        .into_iter()
        .flatten()
        .filter_map(|face| names.faces.get(face.0 as usize))
        .collect();
    let pair = match sides.as_slice() {
        [a, b] => Some((*a, *b)),
        [a] => Some((*a, *a)),
        _ => None,
    };
    match pair {
        Some((a, b)) if edges_between(body, names, a, b).len() == 1 => {
            format!("{why}; its faces meet in this edge only: give the pair [\"{a}\", \"{b}\"]")
        }
        Some((a, b)) => format!(
            "{why}; no other name says which edge it is: the pair [\"{a}\", \"{b}\"] takes every edge between those faces, and one of them told apart only by place cannot be taken alone yet"
        ),
        None => format!(
            "{why}; no other name says which edge it is, and one told apart only by place cannot be taken alone yet"
        ),
    }
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
