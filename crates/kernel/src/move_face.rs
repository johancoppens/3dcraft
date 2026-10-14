//! Move face: faces of a solid moved together by one rigid motion (a
//! translation, a rotation about a line, or a rotation followed by a
//! translation), with the faces next to them following so that the body
//! stays closed.
//!
//! The moved faces keep their kind and their shape: each surface is the
//! image of what it was ([`craft3d_geom::Surface::transformed`]). What
//! lies between two moved faces (an edge both of whose faces move, a
//! vertex all of whose faces do) is carried along exactly, whatever its
//! kind; what lies between a moved face and one that stays is found
//! again where the surfaces meet now, as [`crate::offset_face`] finds it
//! ([`crate::tweak`]). So a block's top tilted about a line across it
//! gives a wedge whose sides follow, a boss's wall and top moved
//! sideways give the boss somewhere else on its face, and a hole's wall
//! moved sideways gives the hole somewhere else.
//!
//! **Where the topology would have to change** the move is refused with
//! the faces and the reason, never built wrong: an edge that would shrink
//! to nothing or a face that would vanish (with the largest part of the
//! motion that fits, found by halving), the faces at a vertex that no
//! longer meet in one point, and an edge between a moved face and one
//! that stays that would no longer be a line or a circle (a cylinder
//! tilted against a flat face meets it in an ellipse).

use crate::KernelError;
use crate::offset_face::{Offset, fitting, refuse, same_numbers, usable};
use crate::tweak::{self, TweakError};
use craft3d_geom::Surface;
use craft3d_math::{Tolerance, Transform, Vec3};
use craft3d_topo::{Body, FaceId};
use std::collections::BTreeMap;

const OPERATION: &str = "move face";

/// A rigid motion: a turn by `angle` (radians, counter-clockwise seen
/// from the tip of `axis`) about the line through `pivot` along `axis`,
/// then a shift by `translate`. With no angle the axis is not looked at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub translate: Vec3,
    pub pivot: Vec3,
    pub axis: Vec3,
    pub angle: f64,
}

impl Motion {
    /// The motion scaled down to `fraction` of itself: that part of the
    /// shift and of the turn.
    pub fn part(&self, fraction: f64) -> Option<Transform> {
        let shift = Transform::translation(self.translate * fraction);
        if self.angle == 0.0 {
            return Some(shift);
        }
        Transform::rotation_about(self.pivot, self.axis, self.angle * fraction)
            .map(|turn| turn.then(&shift))
    }
}

/// `body` with `faces` moved by `motion` and the faces next to them
/// following (see the module documentation). Every face, edge and vertex
/// of the result is the one with the same number in `body`.
///
/// **Errors.** [`KernelError::FaceOpTooFar`] for a motion at which an
/// edge or a face would vanish: `value` is how far the farthest moved
/// vertex goes, `largest` how far it may go (the same part of the
/// motion); [`KernelError::FaceOp`] for what is not built or not a face;
/// [`KernelError::InvalidSize`] for a motion that is not finite or not
/// rigid.
pub fn move_faces(
    body: &Body,
    faces: &[FaceId],
    motion: &Motion,
    tolerance: &Tolerance,
) -> Result<Offset, KernelError> {
    usable(body, faces, OPERATION, tolerance)?;
    let finite = motion.translate.is_finite()
        && motion.pivot.is_finite()
        && motion.axis.is_finite()
        && motion.angle.is_finite();
    let Some(whole) = motion.part(1.0).filter(|_| finite) else {
        return Err(KernelError::InvalidSize {
            name: "motion",
            value: f64::NAN,
        });
    };
    // How far the farthest vertex of a moved face goes.
    let mut largest = 0.0_f64;
    for face in faces {
        for vertex in tweak::face_vertices(body, *face) {
            if let Some(point) = body.vertex(vertex).map(|v| v.point) {
                largest = largest.max(whole.point(point).distance(point));
            }
        }
        // A face without vertices (a whole sphere) moves with its centre.
        if let Some(Surface::Sphere(sphere)) = body.face(*face).map(|f| &f.surface) {
            let centre = sphere.frame().origin();
            largest = largest.max(whole.point(centre).distance(centre));
        }
    }
    if !(largest.is_finite() && largest > tolerance.linear) {
        return Err(refuse(
            OPERATION,
            Vec::new(),
            None,
            "there is nothing to move: the motion moves no face by more than the tolerance",
        ));
    }
    let attempt = |fraction: f64| -> Result<Body, TweakError> {
        let Some(map) = motion.part(fraction) else {
            return Err(TweakError::Kernel(KernelError::InvalidSize {
                name: "motion",
                value: fraction,
            }));
        };
        let mut surfaces: BTreeMap<FaceId, Surface> = BTreeMap::new();
        for face in faces {
            let Some(surface) = body.face(*face).map(|f| &f.surface) else {
                continue;
            };
            surfaces.insert(*face, surface.transformed(&map).map_err(KernelError::from)?);
        }
        tweak::resurface_rigid(body, &surfaces, &map, OPERATION, tolerance)
    };
    fitting(attempt, largest, OPERATION, tolerance).map(same_numbers)
}
