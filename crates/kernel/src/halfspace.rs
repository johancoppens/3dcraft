//! A solid cut by a plane, one side kept (docs/design/plane-cut.md).
//!
//! Method. The plane is a frame: its origin a point of the plane, its z
//! axis the normal. *Below* is the side the normal points away from, and
//! what is kept; the normal points to what is cut away, and out of the
//! body that is left. What lies below is the boolean intersection of the
//! body with a block this module makes from the body's own box
//! ([`tool`]): its top face is the plane itself, and it reaches past the
//! body on every other side, so nobody sizes a tool and no digits are
//! lost to one sized "large enough". It is the cut `crate::below` makes
//! to measure what lies below a plane, here returned as a body with the
//! source of every entity.
//!
//! The block. With `c` the centre of the body's box ([`Body::bbox`],
//! which holds the body and is the box the booleans measure their own
//! margins against), `r` half its diagonal and
//! `m = r / 4 + MARGIN tolerances`: the block reaches `r + m` to every
//! side of `c` dropped onto the plane, and down to `r + m` below `c`.
//! Every point of the body is within `r` of `c`, so the block holds all
//! of the body below the plane, and its five faces away from the plane
//! are at least `m` from every point of the body: beyond every distance a
//! rule of the boolean reads (16 tolerances), so no pair of such a face
//! with a face of the body gets past the first box test. No length is
//! built in: `r` is the body's and the tolerance the caller's, so a body
//! a million times larger gets the same block a million times larger.
//! That the block held the body is checked, not trusted ([`keep_below`]).
//!
//! Before any boolean the eight corners of the body's box are measured
//! against the plane: a body wholly below it, clear by more than
//! `CLEAR` tolerances, is kept whole, and one wholly above by more than
//! that leaves nothing. Nearer than that the boolean decides, so every
//! rule of touch and tolerance is the boolean's own, in one place.

use crate::KernelError;
use crate::boolean::{BooleanOp, BooleanOptions, BooleanResult, FaceSource, Operand, boolean_with};
use crate::primitives::block;
use craft3d_geom::{Plane, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, FaceId};

const OPERATION: &str = "plane cut";

/// The face of the [`tool`]'s block that lies in the plane: the faces of
/// a block are -z, +z, -y, +y, -x, +x of its frame, and the plane is its
/// +z.
pub const CAP: FaceId = FaceId(1);

/// How far beyond the body the block reaches, in tolerances, besides a
/// quarter of the half-diagonal of the body's box: twice the 16
/// tolerances that are the furthest a rule of the boolean reads.
const MARGIN: f64 = 32.0;

/// How far from the plane, in tolerances, the body's box must be for the
/// body to be answered without a boolean: the distance from which the
/// boolean takes two faces as apart.
const CLEAR: f64 = 2.0;

/// A sheet has no inside to keep a side of.
const NOT_SOLID: &str = "a body that is a sheet, which has no inside: a plane cuts solids";

/// What a plane leaves to do on a body.
#[derive(Clone, Debug, PartialEq)]
pub enum Tool {
    /// The body lies wholly below the plane and clear of it: nothing is
    /// cut.
    Whole,
    /// The block to intersect the body with. Its face [`CAP`] lies in the
    /// plane, on the surface of the frame given, bit for bit.
    Block(Body),
}

/// What a plane left of a body.
#[derive(Clone, Debug, PartialEq)]
pub enum Kept {
    /// All of it: the plane cuts nothing (the body lies below it, touches
    /// it from below, or has a face in it and lies behind that face).
    Whole,
    /// The part below the plane, as the boolean made it: operand A is the
    /// body, and the only face of operand B any face has a piece of is
    /// [`CAP`], the plane.
    Cut(Box<BooleanResult>),
}

fn refused(reason: &str) -> KernelError {
    KernelError::Refused {
        operation: OPERATION,
        reason: reason.to_string(),
    }
}

/// The block that holds everything of `body` below `plane` (the module's
/// description says how large and why), or that there is nothing to cut.
///
/// **Errors.** [`KernelError::EmptyResult`] when the body lies wholly
/// above the plane; `Unsupported` for a sheet; `Refused` for a body
/// without a finite box.
pub fn tool(body: &Body, plane: &Frame, tolerance: &Tolerance) -> Result<Tool, KernelError> {
    if body.kind != BodyKind::Solid {
        return Err(KernelError::Unsupported {
            operation: OPERATION,
            reason: NOT_SOLID,
            faces: Vec::new(),
        });
    }
    let bbox = body.bbox().map_err(|error| KernelError::Refused {
        operation: OPERATION,
        reason: format!("the body has no box to measure against the plane: {error}"),
    })?;
    let (Some((lo, hi)), Some(center)) = (bbox.corners(), bbox.center()) else {
        return Err(refused("the body is empty"));
    };
    let tol = tolerance.linear;
    let r = 0.5 * bbox.diagonal();
    if !(r.is_finite() && r > 0.0 && tol.is_finite() && tol > 0.0) {
        return Err(refused("the body's box has no finite size"));
    }
    let (origin, normal) = (plane.origin(), plane.z());
    // How high the box reaches over the plane, and how low under it.
    let (mut least, mut most) = (f64::INFINITY, f64::NEG_INFINITY);
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let height = (Vec3::new(x, y, z) - origin).dot(normal);
                least = least.min(height);
                most = most.max(height);
            }
        }
    }
    if !(least.is_finite() && most.is_finite()) {
        return Err(refused("the plane is not finite"));
    }
    let clear = CLEAR * tol;
    if most < -clear {
        return Ok(Tool::Whole);
    }
    if least > clear {
        return Err(KernelError::EmptyResult {
            operation: OPERATION,
        });
    }
    let margin = 0.25 * r + MARGIN * tol;
    let half = r + margin;
    let height = (center - origin).dot(normal);
    // The centre dropped onto the plane, and how far the block goes down
    // from the plane: to `half` below the centre. A corner of the box is
    // no higher than `clear`, and the centre no more than `r` above it,
    // so the depth is at least the margin less `clear`.
    let foot = center - normal * height;
    let depth = half - height;
    if !(depth.is_finite() && depth > 0.0) {
        return Err(refused("the plane is not finite"));
    }
    let corner = foot - plane.x() * half - plane.y() * half - normal * depth;
    let frame = plane
        .at(corner)
        .ok_or_else(|| refused("the plane is not finite"))?;
    let mut made = block(&frame, 2.0 * half, 2.0 * half, depth)?;
    // The top lies in the plane to rounding; its surface is the plane as
    // given, so that two cuts by one plane have one surface, and what is
    // placed on the cut face has the plane's own origin and axes.
    let Some(cap) = made.faces.get_mut(CAP.0 as usize) else {
        return Err(refused(craft3d_math::bug_text!(
            "the block of a plane cut has no top face"
        )));
    };
    cap.surface = Surface::Plane(Plane::new(*plane));
    Ok(Tool::Block(made))
}

/// The source of the plane in a result.
fn cap() -> FaceSource {
    FaceSource {
        operand: Operand::B,
        face: CAP,
    }
}

/// The part of the solid `body` below `plane` (the side its z axis points
/// away from): the body intersected with the block of [`tool`].
///
/// [`Kept::Whole`] when the plane cuts nothing: no face of the result has
/// a piece of the plane, so the result is the body, which is then not
/// handed back as a copy. Otherwise the boolean's result, in which every
/// face is a piece of a face of `body` (operand A) or of the plane
/// (operand B, [`CAP`]); a face the boolean merged from both lists both
/// under [`BooleanResult::merged`].
///
/// **Errors.** [`KernelError::EmptyResult`] when nothing lies below the
/// plane; every refusal of the boolean (faces at the edge of tolerance,
/// parts that would touch, a meeting of surfaces it cannot trace); and
/// `Refused`, worded as a bug, should a face of the block other than the
/// plane be in the result: the block did not hold the body, and the body
/// is refused rather than returned cut short.
pub fn keep_below(
    body: &Body,
    plane: &Frame,
    tolerance: &Tolerance,
    options: &BooleanOptions,
) -> Result<Kept, KernelError> {
    let block = match tool(body, plane, tolerance)? {
        Tool::Whole => return Ok(Kept::Whole),
        Tool::Block(block) => block,
    };
    let result = boolean_with(body, &block, BooleanOp::Intersect, tolerance, options)?;
    let far = |source: &FaceSource| source.operand == Operand::B && source.face != CAP;
    if result.faces.iter().any(far) || result.merged.iter().flatten().any(far) {
        return Err(refused(craft3d_math::bug_text!(
            "the block of a plane cut did not hold the body: a face of it other than the plane is in the result"
        )));
    }
    let in_plane = cap();
    let cuts = result.faces.contains(&in_plane)
        || result
            .merged
            .iter()
            .any(|sources| sources.contains(&in_plane));
    Ok(if cuts {
        Kept::Cut(Box::new(result))
    } else {
        Kept::Whole
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn cube(side: f64) -> Body {
        block(&Frame::WORLD, side, side, side).unwrap()
    }

    /// The block's top face is the plane as given, bit for bit, whatever
    /// the plane's origin; its other faces are further from the body's
    /// box than the margin.
    #[test]
    fn the_block_ends_on_the_plane_itself_and_clears_the_body() {
        for k in [1e-3, 1.0, 1e6] {
            let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
            let body = cube(10.0 * k);
            // Through the cube's centre, from an origin far beside it:
            // 0.1 (5 + 400) − 0.2 (5 − 250) + (5 − 94.5) = 0.
            let plane = Frame::new(
                Vec3::new(-400.0 * k, 250.0 * k, 94.5 * k),
                Vec3::new(0.1, -0.2, 1.0),
                Vec3::X,
            )
            .unwrap();
            let Tool::Block(made) = tool(&body, &plane, &tolerance).unwrap() else {
                panic!("the plane crosses the cube at scale {k}");
            };
            let top = made.faces.get(CAP.0 as usize).unwrap();
            assert_eq!(top.surface, Surface::Plane(Plane::new(plane)));
            assert!(!top.reversed);
            let r = 0.5 * body.bbox().unwrap().diagonal();
            let center = Vec3::new(5.0 * k, 5.0 * k, 5.0 * k);
            for vertex in &made.vertices {
                let height = (vertex.point - plane.origin()).dot(plane.z());
                // On the plane to rounding, or under it.
                assert!(height <= 1e-9 * k, "scale {k}: {height}");
                // Every corner is further from the body's centre than the
                // body reaches, by the margin at the least.
                assert!(vertex.point.distance(center) >= 1.25 * r, "scale {k}");
            }
        }
    }

    /// A body clear of the plane is answered from its box: whole below
    /// it, nothing above it; a sheet is refused.
    #[test]
    fn a_body_clear_of_the_plane_needs_no_block() {
        let tolerance = Tolerance::new(1e-6, 1e-9).unwrap();
        let body = cube(10.0);
        let over = Frame::new(Vec3::new(0.0, 0.0, 11.0), Vec3::Z, Vec3::X).unwrap();
        assert_eq!(tool(&body, &over, &tolerance), Ok(Tool::Whole));
        let under = Frame::new(Vec3::new(0.0, 0.0, -1.0), Vec3::Z, Vec3::X).unwrap();
        assert!(matches!(
            tool(&body, &under, &tolerance),
            Err(KernelError::EmptyResult { .. })
        ));
        // In the top face: the boolean's to decide.
        let on = Frame::new(Vec3::new(0.0, 0.0, 10.0), Vec3::Z, Vec3::X).unwrap();
        assert!(matches!(tool(&body, &on, &tolerance), Ok(Tool::Block(_))));
        let mut sheet = body.clone();
        sheet.kind = BodyKind::Sheet;
        assert!(matches!(
            tool(&sheet, &on, &tolerance),
            Err(KernelError::Unsupported { .. })
        ));
    }
}
