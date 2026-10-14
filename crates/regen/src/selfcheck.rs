//! The check every body built from freely placed profiles gets before it
//! is handed on: a loft or a sweep can be valid in how it hangs together
//! and pass through itself all the same, which the kernel does not see.

use craft3d_doc::PersistentName;
use craft3d_tess::{SelfCheckParams, body_self_intersections};
use craft3d_topo::Body;

/// What looking for places where a body passes through itself gave.
pub(crate) enum Crossing {
    /// It does not.
    None,
    /// It does; the two faces that cross, by their names, when the search
    /// is certain of a pair.
    Found(Option<[String; 2]>),
    /// The body could not be meshed to look; the tessellator's reason.
    Unknown(String),
}

/// Looks for places where `body` passes through itself, at the linear
/// tolerance `linear`. `faces[i]` is the name of face `i`.
pub(crate) fn crossing(body: &Body, faces: &[PersistentName], linear: f64) -> Crossing {
    let found = match body_self_intersections(body, &SelfCheckParams::new(linear)) {
        Ok(found) => found,
        Err(error) => return Crossing::Unknown(error.to_string()),
    };
    if found.is_clean() {
        return Crossing::None;
    }
    let name = |face: u32| {
        faces
            .get(face as usize)
            .map_or_else(|| format!("face {face}"), ToString::to_string)
    };
    Crossing::Found(
        found
            .crossings
            .iter()
            .find(|c| c.certain)
            .map(|c| c.faces.map(name)),
    )
}
