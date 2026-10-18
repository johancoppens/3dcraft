//! Draft: faces of a solid tilted about a neutral plane, so that the part
//! comes out of a mould along a pull direction.
//!
//! The neutral plane is square to the pull direction; a drafted face
//! keeps the line or the circle in which it meets that plane, and leans
//! in by the angle as it goes along the pull direction (out by it against
//! the pull): a block drafted from its bottom narrows toward its top, a
//! hole drafted from the bottom widens toward the top. Positive is that
//! way; a negative angle leans the other way.
//!
//! **Closed forms.** A flat face along the pull direction turns about the
//! line it shares with the neutral plane: still a plane. A cylinder about
//! an axis along the pull direction becomes the cone about that axis
//! with the cylinder's radius at the neutral plane and the angle as its
//! half-angle. The faces round them follow as the offset makes them
//! follow ([`crate::tweak`]): the top of a drafted block shrinks, the
//! edges between drafted faces are found again where the new surfaces
//! meet, and a flat face tangent to a cylinder stays tangent to its cone
//! (both are drafted from the same line), so a rounded box drafted is a
//! rounded frustum.
//!
//! **Refused, with the face and the reason:** a face that does not run
//! along the pull direction (a flat face whose normal is not square to
//! it, a cylinder whose axis is not along it), a cone, a sphere, a torus
//! and a freeform face (their draft is not a surface of a kind we build
//! in closed form). An angle at which a face would vanish (the top of a
//! frustum drawn to a point) is refused with the largest angle that fits.

use crate::KernelError;
use crate::offset_face::{Offset, fitting, refuse, same_numbers, usable};
use crate::tweak::{self, TweakError};
use craft3d_geom::{Cone, Plane, Surface};
use craft3d_math::{Tolerance, Transform, Vec3};
use craft3d_topo::{Body, FaceId};
use std::collections::BTreeMap;
use std::f64::consts::FRAC_PI_2;

const OPERATION: &str = "draft";

/// How far from square to the pull direction a face's normal, or from
/// along it a cylinder's axis, may be (a sine) and still be drafted.
const ALONG: f64 = 1e-9;

/// Faces to tilt, the neutral plane (through `point`, square to `pull`)
/// and the angle (radians).
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub faces: Vec<FaceId>,
    pub point: Vec3,
    pub pull: Vec3,
    pub angle: f64,
}

/// The surface of `face` drafted by `angle` about the neutral plane
/// through `point` square to the unit vector `pull`.
fn drafted(
    body: &Body,
    face: FaceId,
    point: Vec3,
    pull: Vec3,
    angle: f64,
) -> Result<Surface, TweakError> {
    let not = |reason: &'static str| TweakError::Unsupported {
        faces: vec![face],
        edge: None,
        reason,
    };
    let Some(found) = body.face(face) else {
        return Err(not("there is no such face in the body"));
    };
    let outward = |normal: Vec3| if found.reversed { -normal } else { normal };
    match &found.surface {
        Surface::Plane(plane) => {
            let normal = outward(plane.frame().z());
            if normal.dot(pull).abs() > ALONG {
                return Err(not(
                    "the face does not run along the pull direction: only faces along it are drafted (its normal has to be square to it)",
                ));
            }
            // The line the face shares with the neutral plane, and the
            // turn about it that leans the face in along the pull.
            let hinge = normal * normal.dot(plane.frame().origin()) + pull * pull.dot(point);
            let Some(axis) = normal.cross(pull).normalized() else {
                return Err(not("the face's normal has no direction"));
            };
            let turn = Transform::rotation_about(hinge, axis, angle)
                .ok_or_else(|| not("the turn of the face is not finite"))?;
            let frame = turn
                .frame(plane.frame())
                .ok_or_else(|| not("the turned face is not finite"))?;
            Ok(Surface::Plane(Plane::new(frame)))
        }
        Surface::Cylinder(cylinder) => {
            let frame = cylinder.frame();
            let along = frame.z().dot(pull);
            if frame.z().cross(pull).length() > ALONG {
                return Err(not(
                    "the axis of the cylinder does not run along the pull direction: it would not become a cone",
                ));
            }
            // Where the axis meets the neutral plane.
            let origin = frame.origin() + frame.z() * ((point - frame.origin()).dot(pull) / along);
            let Some(at) = frame.at(origin) else {
                return Err(not("the axis of the cylinder is not finite"));
            };
            // Narrowing along the pull for a boss, widening for a hole.
            let sense = if found.reversed { 1.0 } else { -1.0 };
            let half = sense * angle * along.signum();
            if !half.is_finite() || half.abs() >= FRAC_PI_2 {
                return Err(not("the angle is not below 90 degrees"));
            }
            Cone::new(at, cylinder.radius(), half)
                .map(Surface::Cone)
                .map_err(|_| TweakError::Vanishes {
                    faces: vec![face],
                    edge: None,
                    reason: "the cone the cylinder would become cannot be made",
                })
        }
        Surface::Cone(_) => Err(not(
            "a cone is not drafted: it has an angle of its own, and changing it is not built",
        )),
        Surface::Sphere(_) | Surface::Torus(_) => Err(not(
            "a sphere or a torus is not drafted: its draft is not a surface of a kind that is built",
        )),
        Surface::Bezier(_) | Surface::Nurbs(_) => Err(not(
            "a freeform face is not drafted: its draft is not a surface of a kind that is built",
        )),
    }
}

/// `body` with the faces of `draft` tilted (see the module
/// documentation). Every face, edge and vertex of the result is the one
/// with the same number in `body`.
///
/// **Errors.** [`KernelError::FaceOpTooFar`] for an angle at which an
/// edge or a face would vanish: `value` is the angle times the greatest
/// distance of a drafted face from the neutral plane, `largest` the same
/// for the angle that fits; [`KernelError::FaceOp`] for a face that is
/// not drafted, with the reason; [`KernelError::InvalidSize`] for an
/// angle that is not between -90 and 90 degrees or a pull direction that
/// is none.
pub fn draft(body: &Body, draft: &Draft, tolerance: &Tolerance) -> Result<Offset, KernelError> {
    usable(body, &draft.faces, OPERATION, tolerance)?;
    if !(draft.angle.is_finite() && draft.angle.abs() < FRAC_PI_2) {
        return Err(KernelError::InvalidSize {
            name: "angle",
            value: draft.angle,
        });
    }
    let Some(pull) = draft.pull.normalized().filter(|p| p.is_finite()) else {
        return Err(KernelError::InvalidSize {
            name: "pull direction",
            value: draft.pull.length(),
        });
    };
    if !draft.point.is_finite() {
        return Err(KernelError::InvalidSize {
            name: "neutral plane",
            value: f64::NAN,
        });
    }
    // Every face is one that is drafted, whatever else is said of it.
    for face in &draft.faces {
        match drafted(body, *face, draft.point, pull, draft.angle) {
            Ok(_) | Err(TweakError::Vanishes { .. }) => {}
            Err(TweakError::Unsupported {
                faces,
                edge,
                reason,
            }) => return Err(refuse(OPERATION, faces, edge, reason)),
            Err(TweakError::Kernel(error)) => return Err(error),
        }
    }
    // How far the farthest vertex of a drafted face is from the neutral
    // plane: the angle times that is how far it moves, about.
    let mut reach = 0.0_f64;
    for face in &draft.faces {
        for vertex in tweak::face_vertices(body, *face) {
            if let Some(at) = body.vertex(vertex).map(|v| v.point) {
                reach = reach.max((at - draft.point).dot(pull).abs());
            }
        }
    }
    let largest = draft.angle.abs() * reach;
    if !(largest.is_finite() && largest > tolerance.linear) {
        return Err(refuse(
            OPERATION,
            Vec::new(),
            None,
            "there is nothing to tilt: no angle, or faces that do not reach away from the neutral plane",
        ));
    }
    let attempt = |fraction: f64| -> Result<Body, TweakError> {
        let mut surfaces: BTreeMap<FaceId, Surface> = BTreeMap::new();
        for face in &draft.faces {
            surfaces.insert(
                *face,
                drafted(body, *face, draft.point, pull, draft.angle * fraction)?,
            );
        }
        tweak::resurface(body, &surfaces, OPERATION, tolerance)
    };
    let bound = crate::offset_face::face_op_work(body, &draft.faces);
    fitting(attempt, largest, OPERATION, tolerance, bound).map(same_numbers)
}
