//! Offset face: faces of a solid moved along their normals, with the
//! faces next to them following so that the body stays closed.
//!
//! A flat face moves to the parallel plane at the distance; a cylinder, a
//! sphere and a torus's tube grow or shrink by it, and a cone keeps its
//! angle and moves its apex along its axis. Positive is outward, away
//! from the material: a boss grows and a hole narrows; negative is into
//! the material. The neighbours are not moved: their boundaries are, to
//! where their surfaces meet the moved one ([`crate::tweak`], which says
//! what is built and how a wrong solid is kept out).
//!
//! **Method: a local operation, not a tool and the boolean.** Inward, the
//! body cut by the half-space behind a moved plane would be the answer;
//! outward, the new material is bounded by the neighbours' continued
//! surfaces, which no tool that is easy to make has as its faces, and
//! which is the same computation as closing the body directly. So the
//! body is closed directly in both directions: one route, the faces stay
//! the faces they were (a block whose top moves has six faces, not
//! seven), and every entity keeps its number, which is what its name
//! follows (docs/decisions.md, 14 October 2026).
//!
//! **Too far.** A distance at which an edge shrinks to nothing, a face is
//! turned inside out, a hole leaves its face or the faces at a vertex no
//! longer meet is refused with the entity and the largest distance that
//! fits, found by halving. When no distance at all can be made, the
//! geometry is one that is not built (a neighbour whose continuation does
//! not meet the moved face in a line or a circle), and that is said
//! instead.

use crate::KernelError;
use crate::tweak::{self, TweakError};
use craft3d_geom::{Cone, Cylinder, Plane, Sphere, Surface, Torus};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, BodyKind, EdgeId, Face, FaceId, VertexId};
use std::collections::BTreeMap;

const OPERATION: &str = "offset face";

/// Most faces one offset takes.
pub const MAX_FACES: usize = 4096;

/// One face to move and how far along its outward normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceOffset {
    pub face: FaceId,
    pub distance: f64,
}

/// A body with faces moved and where each of its entities came from:
/// `faces[i]` is the face of the body given that `FaceId(i)` of `body`
/// is, and so on.
#[derive(Clone, Debug, PartialEq)]
pub struct Offset {
    pub body: Body,
    pub faces: Vec<FaceId>,
    pub edges: Vec<Option<EdgeId>>,
    pub vertices: Vec<Option<VertexId>>,
}

/// Why a surface has no offset.
pub(crate) enum NoOffset {
    /// The offset is nothing: a radius that reaches zero.
    Gone,
    /// The offset of this kind of surface is not built.
    Kind,
}

/// The surface at `distance` from that of `face`, along its outward
/// normal.
pub(crate) fn offset_surface(face: &Face, distance: f64) -> Result<Surface, NoOffset> {
    // Along the surface's own normal.
    let along = if face.reversed { -distance } else { distance };
    match &face.surface {
        Surface::Plane(plane) => {
            let frame = plane.frame();
            frame
                .at(frame.origin() + frame.z() * along)
                .map(|frame| Surface::Plane(Plane::new(frame)))
                .ok_or(NoOffset::Gone)
        }
        Surface::Cylinder(cylinder) => Cylinder::new(*cylinder.frame(), cylinder.radius() + along)
            .map(Surface::Cylinder)
            .map_err(|_| NoOffset::Gone),
        Surface::Sphere(sphere) => Sphere::new(*sphere.frame(), sphere.radius() + along)
            .map(Surface::Sphere)
            .map_err(|_| NoOffset::Gone),
        Surface::Torus(torus) => {
            let (major, minor) = torus.radii();
            Torus::new(*torus.frame(), major, minor + along)
                .map(Surface::Torus)
                .map_err(|_| NoOffset::Gone)
        }
        Surface::Cone(cone) => {
            // The same angle about the same axis; the radius at every
            // height changes by the distance over the cosine.
            let (slope, cosine) = (cone.half_angle().tan(), cone.half_angle().cos());
            if !(cosine > 0.0 && slope != 0.0) {
                return Err(NoOffset::Gone);
            }
            let radius = cone.radius() + along / cosine;
            let frame = cone.frame();
            let (frame, radius) = if radius >= 0.0 {
                (*frame, radius)
            } else {
                // The height the frame stands at is past the new apex:
                // stand it where the radius is what it was.
                let lift = (cone.radius() - radius) / slope;
                let Some(lifted) = frame.at(frame.origin() + frame.z() * lift) else {
                    return Err(NoOffset::Gone);
                };
                (lifted, cone.radius())
            };
            Cone::new(frame, radius, cone.half_angle())
                .map(Surface::Cone)
                .map_err(|_| NoOffset::Gone)
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => Err(NoOffset::Kind),
    }
}

/// Halvings in the search for the largest distance that fits.
const HALVINGS: usize = 48;

pub(crate) fn refuse(
    operation: &'static str,
    faces: Vec<FaceId>,
    edge: Option<EdgeId>,
    reason: &'static str,
) -> KernelError {
    KernelError::FaceOp {
        operation,
        faces,
        edge,
        reason,
    }
}

/// The body with every face of `offsets` at `fraction` of its distance.
fn at(
    body: &Body,
    offsets: &[FaceOffset],
    fraction: f64,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    let mut surfaces: BTreeMap<FaceId, Surface> = BTreeMap::new();
    for offset in offsets {
        let Some(face) = body.face(offset.face) else {
            return Err(TweakError::Unsupported {
                faces: vec![offset.face],
                edge: None,
                reason: "there is no such face in the body",
            });
        };
        let surface = offset_surface(face, offset.distance * fraction).map_err(|no| match no {
            NoOffset::Gone => TweakError::Vanishes {
                faces: vec![offset.face],
                edge: None,
                reason: "the radius of the face would reach zero: the face would vanish",
            },
            NoOffset::Kind => TweakError::Unsupported {
                faces: vec![offset.face],
                edge: None,
                reason: "the offset of a freeform face is not built (it is not a surface of the same kind)",
            },
        })?;
        surfaces.insert(offset.face, surface);
    }
    tweak::resurface(body, &surfaces, operation, tolerance)
}

/// [`offset_faces`] in the words of `operation`, for the operations that
/// are made of it.
pub(crate) fn offset_for(
    body: &Body,
    offsets: &[FaceOffset],
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0 && tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if body.kind != BodyKind::Solid {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "only the faces of a solid can be moved",
        ));
    }
    if offsets.len() > MAX_FACES {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "more faces than one operation takes",
        ));
    }
    let mut largest = 0.0_f64;
    for (index, offset) in offsets.iter().enumerate() {
        if !offset.distance.is_finite() {
            return Err(KernelError::InvalidSize {
                name: "distance",
                value: offset.distance,
            });
        }
        if offsets
            .iter()
            .take(index)
            .any(|earlier| earlier.face == offset.face)
        {
            return Err(refuse(
                operation,
                vec![offset.face],
                None,
                "the face is given twice",
            ));
        }
        largest = largest.max(offset.distance.abs());
    }
    if offsets.is_empty() || largest <= tol {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "there is nothing to move: no face, or a distance that cannot be told from none at this tolerance",
        ));
    }
    let defects = craft3d_topo::check(body, tolerance);
    if !defects.is_empty() {
        return Err(KernelError::InvalidBody {
            operation,
            defects: defects.into_iter().take(crate::MAX_REPORTED).collect(),
        });
    }
    let (faces, edge, reason) = match at(body, offsets, 1.0, operation, tolerance) {
        Ok(made) => return Ok(made),
        Err(TweakError::Kernel(error)) => return Err(error),
        Err(TweakError::Unsupported {
            faces,
            edge,
            reason,
        }) => return Err(refuse(operation, faces, edge, reason)),
        Err(TweakError::Vanishes {
            faces,
            edge,
            reason,
        }) => (faces, edge, reason),
    };
    // The largest part of the distance that can be made, by halving.
    let (mut fits, mut fails) = (0.0_f64, 1.0_f64);
    for _ in 0..HALVINGS {
        let middle = 0.5 * (fits + fails);
        if middle * largest <= tol {
            break;
        }
        match at(body, offsets, middle, operation, tolerance) {
            Ok(_) => fits = middle,
            Err(_) => fails = middle,
        }
    }
    if fits == 0.0 {
        // Not a matter of how far.
        return Err(refuse(operation, faces, edge, reason));
    }
    Err(KernelError::FaceOpTooFar {
        operation,
        faces,
        edge,
        reason,
        value: largest,
        largest: fits * largest,
    })
}

/// `body` with the faces of `offsets` moved along their outward normals
/// and the faces next to them following (see the module documentation).
///
/// **Errors.** [`KernelError::FaceOpTooFar`] for a distance at which an
/// edge or a face would vanish, with the largest that fits;
/// [`KernelError::FaceOp`] for what is not built or not a face;
/// [`KernelError::InvalidBody`] when the body given, or (a bug, or input
/// at the edge of tolerance) the result fails the checker.
pub fn offset_faces(
    body: &Body,
    offsets: &[FaceOffset],
    tolerance: &Tolerance,
) -> Result<Offset, KernelError> {
    let made = offset_for(body, offsets, OPERATION, tolerance)?;
    let numbered = |count: usize| (0..count).map(|i| u32::try_from(i).unwrap_or(u32::MAX));
    Ok(Offset {
        faces: numbered(made.faces.len()).map(FaceId).collect(),
        edges: numbered(made.edges.len())
            .map(|i| Some(EdgeId(i)))
            .collect(),
        vertices: numbered(made.vertices.len())
            .map(|i| Some(VertexId(i)))
            .collect(),
        body: made,
    })
}
