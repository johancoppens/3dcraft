//! The check that no body is handed on that passes through itself
//! (docs/design/regeneration.md › *Never through itself*).
//!
//! `craft3d_topo::check` sees how a body hangs together, not whether two
//! of its faces pass through each other; `tess::body_self_intersections`
//! sees that on the mesh. Every operation either cannot make such a body
//! (the reasoning is in §5.5, per operation) or is looked at here:
//! - [`crossing`], the whole body: a loft or a sweep, built from freely
//!   placed profiles, is new everywhere;
//! - [`crossing_near`], what an operation changed: the result of a
//!   boolean is pieces of the faces of two bodies that were looked at
//!   when they were made, so only where pieces of the two can meet is
//!   looked at, and a blend's new faces against what is near them.
//!
//! A body that cannot be meshed to look is not handed on either:
//! [`Crossing::Unknown`], which every caller refuses with the
//! tessellator's reason.

use crate::Contributors;
use craft3d_doc::PersistentName;
use craft3d_math::Vec3;
use craft3d_tess::{
    FaceRole, SelfCheck, SelfCheckParams, Touched, body_self_intersections,
    body_self_intersections_near,
};
use craft3d_topo::Body;

/// What looking for places where a body passes through itself gave.
pub(crate) enum Crossing {
    /// It does not.
    None,
    /// It does; the two faces that cross, by their names, when the search
    /// is certain of a pair.
    Found(Option<[String; 2]>),
    /// It touches itself (a vertex on another vertex or on an edge of
    /// its shell, or of another of its shells): the names of a face at
    /// each of the two, where, and whether two shells touch (zero
    /// thickness) rather than one shell itself.
    Touching([String; 2], Vec3, bool),
    /// The body could not be meshed to look; the tessellator's reason.
    Unknown(String),
}

impl Crossing {
    /// The verdict as a feature's error: `param` is the parameter to
    /// blame, `noun` what the feature made, `advice` what to do about a
    /// crossing.
    pub(crate) fn refused(self, param: &str, noun: &str, advice: &str) -> Result<(), String> {
        match self {
            Crossing::None => Ok(()),
            Crossing::Unknown(error) => Err(format!(
                "{param}: the {noun} cannot be meshed to see whether it passes through itself ({error})"
            )),
            Crossing::Found(pair) => {
                let between = pair
                    .map(|[a, b]| format!(": {a} and {b} cross"))
                    .unwrap_or_default();
                Err(format!(
                    "{param}: the {noun} passes through itself{between}; {advice}"
                ))
            }
            Crossing::Touching([a, b], at, true) => Err(format!(
                "{param}: the {noun} would have zero thickness at ({}, {}, {}): two of its parts touch there along an edge or in a point, at {a} and {b}, and no solid can be that thin; move them apart by more than the tolerance, or into each other",
                at.x, at.y, at.z
            )),
            Crossing::Touching([a, b], at, false) => Err(format!(
                "{param}: the {noun} touches itself at ({}, {}, {}), where {a} and {b} meet in a point or along a line that is no edge of both: no solid has a surface like that; move what meets there apart by more than the tolerance, or into each other",
                at.x, at.y, at.z
            )),
        }
    }
}

/// The faces that have edge `edge`, or vertex `vertex` (when `edge` is
/// `None`), in face order.
fn faces_at(body: &Body, edge: Option<u32>, vertex: u32) -> Vec<usize> {
    (0..body.faces.len())
        .filter(|&f| {
            body.faces.get(f).is_some_and(|face| {
                face.loops
                    .iter()
                    .filter_map(|l| body.loop_(*l))
                    .flat_map(|l| &l.coedges)
                    .any(|c| match edge {
                        Some(edge) => c.edge.0 == edge,
                        None => body
                            .edge(c.edge)
                            .is_some_and(|e| e.start.0 == vertex || e.end.0 == vertex),
                    })
            })
        })
        .collect()
}

/// What `found` says of `body`, with the faces named by `faces`. A body
/// the mesh finds clean is also held to the rule that no two of its
/// shells touch (`craft3d_kernel::boolean::parts_touch`: a vertex or
/// the middle of an edge of one shell on a face of another, which the
/// mesh does not see), at the linear tolerance `linear`.
fn verdict(
    body: &Body,
    found: Result<SelfCheck, craft3d_tess::TessError>,
    faces: &[PersistentName],
    linear: f64,
) -> Crossing {
    let found = match found {
        Ok(found) => found,
        Err(error) => return Crossing::Unknown(error.to_string()),
    };
    let name = |face: u32| {
        faces
            .get(face as usize)
            .map_or_else(|| format!("face {face}"), ToString::to_string)
    };
    if found.is_clean() {
        let tolerance = craft3d_math::Tolerance {
            linear,
            ..craft3d_math::Tolerance::default()
        };
        return match craft3d_kernel::boolean::parts_touch(body, &tolerance) {
            Ok(None) => Crossing::None,
            Ok(Some(touch)) => Crossing::Touching(touch.faces.map(|f| name(f.0)), touch.at, true),
            Err(error) => Crossing::Unknown(error.to_string()),
        };
    }
    let certain = found.certain_crossings > 0 || found.shells.iter().any(|s| s.certain);
    if let (false, Some(touch)) = (certain, found.touching.first()) {
        let named = |face: Option<usize>| {
            face.and_then(|f| u32::try_from(f).ok())
                .map_or_else(|| "a face".to_string(), name)
        };
        // A face at each, the first two whose names differ (pieces of
        // one face have one name), else the first two.
        let here = faces_at(body, None, touch.vertex);
        let there = match touch.touches {
            Touched::Vertex(v) => faces_at(body, None, v),
            Touched::Edge(e) => faces_at(body, Some(e), 0),
        };
        let pair = here
            .iter()
            .flat_map(|&f| there.iter().map(move |&g| (f, g)))
            .map(|(f, g)| [named(Some(f)), named(Some(g))])
            .find(|[a, b]| a != b)
            .unwrap_or_else(|| [named(here.first().copied()), named(there.first().copied())]);
        return Crossing::Touching(pair, touch.at, touch.between_shells());
    }
    Crossing::Found(
        found
            .crossings
            .iter()
            .find(|c| c.certain)
            .map(|c| c.faces.map(name)),
    )
}

/// Looks for places where `body` passes through itself, at the linear
/// tolerance `linear`. `faces[i]` is the name of face `i`.
pub(crate) fn crossing(body: &Body, faces: &[PersistentName], linear: f64) -> Crossing {
    verdict(
        body,
        body_self_intersections(body, &SelfCheckParams::new(linear)),
        faces,
        linear,
    )
}

/// [`crossing`] near what an operation changed: `roles` says per face
/// whether it is a piece of a face of a body that was looked at (and of
/// which) or new (`craft3d_tess::body_self_intersections_near`).
pub(crate) fn crossing_near(
    body: &Body,
    faces: &[PersistentName],
    linear: f64,
    roles: &[FaceRole],
) -> Crossing {
    verdict(
        body,
        body_self_intersections_near(body, &SelfCheckParams::new(linear), roles),
        faces,
        linear,
    )
}

/// The roles of the faces of a body made from earlier bodies, from what
/// each face has a piece of: a face with pieces of one of them only is
/// known (pieces of the faces of a body that does not pass through
/// itself do not pass through each other), one merged from pieces of
/// several is new. `new_from` says of which of them the faces are new
/// all the same (a body that was not looked at on its own, as the
/// groove of a thread).
pub(crate) fn roles_of(contributors: &Contributors, new_from: &[u64]) -> Vec<FaceRole> {
    contributors
        .iter()
        .map(|sources| {
            let mut instances = sources.iter().map(|&(instance, _)| instance);
            match instances.next() {
                Some(first)
                    if !new_from.contains(&first) && instances.all(|other| other == first) =>
                {
                    u32::try_from(first).map_or(FaceRole::New, FaceRole::Known)
                }
                _ => FaceRole::New,
            }
        })
        .collect()
}

/// Marks as new, besides the faces `roles` has as new, every face that
/// shares an edge with one of them: a face next to a new one may have
/// grown into it (a wall a chamfer's tool ends in takes the tool's end
/// face), and is then no longer a piece of what it was.
pub(crate) fn with_neighbours(body: &Body, roles: &mut [FaceRole]) {
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); body.edges.len()];
    for (index, face) in body.faces.iter().enumerate() {
        for coedge in face
            .loops
            .iter()
            .filter_map(|id| body.loop_(*id))
            .flat_map(|loop_| &loop_.coedges)
        {
            if let Some(list) = users.get_mut(coedge.edge.0 as usize) {
                list.push(index);
            }
        }
    }
    let new: Vec<bool> = roles.iter().map(|r| *r == FaceRole::New).collect();
    for list in &users {
        if list.iter().any(|&f| new.get(f).copied().unwrap_or(false)) {
            for &face in list {
                if let Some(role) = roles.get_mut(face) {
                    *role = FaceRole::New;
                }
            }
        }
    }
}
