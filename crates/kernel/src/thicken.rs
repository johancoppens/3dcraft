//! Thicken: faces of a solid given a thickness, as a solid of their own.
//!
//! There are no sheet bodies to thicken yet (nothing in the history makes
//! one), so what is thickened is a set of faces of a solid: the layer
//! between those faces and the same faces moved along their normals by
//! the thickness, outside the solid (away from the material), inside it,
//! or half on each side. At its rim the layer is bounded by the faces
//! round the set, continued as [`crate::offset_face`] continues them: a
//! block's top thickened outward is a plate on it with the block's sides
//! carried up, the wall of a rod thickened is a tube as long as the rod,
//! the slanted top of a frustum thickened inward is a cap whose rim lies
//! in the slanted sides.
//!
//! **Method.** Two bodies made by the offset: the solid with the faces
//! moved outward by the part of the thickness that lies outside (the
//! solid itself when none does), and the solid with them moved inward by
//! the rest (the solid itself when none does). The layer is the first
//! less the second ([`crate::boolean`]): every face of it is a piece of a
//! face of one of the two, so a face of the layer is the face of the
//! solid it lies on or along ([`Skin::Outer`], from the first body) or
//! lies on the inside of ([`Skin::Inner`], from the second). Faces of the
//! set appear in both skins: the far side and the near side of the
//! layer.
//!
//! **Never a wrong solid.** A thickness at which a face of either body
//! vanishes is refused with the face or the edge and the largest that
//! fits; after the boolean every face of the set must be there on both
//! sides of the layer, or the layer is refused with that face.

use crate::KernelError;
use crate::boolean::{BooleanOp, BooleanOptions, EdgeSource, VertexSource, boolean_with};
use crate::offset_face::{Blends, FaceOffset, offset_for, refuse, usable};
use crate::shell::{Shelled, Skin};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, FaceId};

const OPERATION: &str = "thicken";

/// Where the layer lies against the faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Outside the solid: on the faces, away from the material.
    Outside,
    /// Inside the solid: under the faces, in the material.
    Inside,
    /// Half of the thickness on each side.
    Both,
}

/// The solid with `faces` moved along their normals by `distance`
/// (outward positive), or the solid itself for no distance.
fn moved(
    body: &Body,
    faces: &[FaceId],
    distance: f64,
    blends: &Blends,
    tolerance: &Tolerance,
) -> Result<Body, KernelError> {
    if distance == 0.0 {
        return Ok(body.clone());
    }
    let offsets: Vec<FaceOffset> = faces
        .iter()
        .map(|face| FaceOffset {
            face: *face,
            distance,
        })
        .collect();
    offset_for(body, &offsets, blends, OPERATION, tolerance)
}

/// A solid between `faces` of `body` and the same faces moved by
/// `thickness` to `side` (see the module documentation). `faces` of the
/// result says which face of `body` each face lies on or along
/// ([`Skin::Outer`]) or under ([`Skin::Inner`]).
///
/// **Errors.** [`KernelError::FaceOpTooFar`] for a thickness at which an
/// edge or a face would vanish, with the largest that fits (in the whole
/// thickness, also for [`Side::Both`]); [`KernelError::FaceOp`] for what
/// is not built or not a face; [`KernelError::FaceOpBoolean`] when the
/// boolean refuses the layer.
pub fn thicken(
    body: &Body,
    faces: &[FaceId],
    thickness: f64,
    side: Side,
    tolerance: &Tolerance,
) -> Result<Shelled, KernelError> {
    thicken_with(body, faces, thickness, side, &Blends::default(), tolerance)
}

/// [`thicken`], told how faces were made (`blends`: the radius each
/// fillet face was made with).
pub fn thicken_with(
    body: &Body,
    faces: &[FaceId],
    thickness: f64,
    side: Side,
    blends: &Blends,
    tolerance: &Tolerance,
) -> Result<Shelled, KernelError> {
    if !(thickness.is_finite() && thickness > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "thickness",
            value: thickness,
        });
    }
    usable(body, faces, OPERATION, tolerance)?;
    let (out, into) = match side {
        Side::Outside => (thickness, 0.0),
        Side::Inside => (0.0, thickness),
        Side::Both => (0.5 * thickness, 0.5 * thickness),
    };
    if out.max(into) <= tolerance.linear {
        return Err(refuse(
            OPERATION,
            Vec::new(),
            None,
            "the thickness cannot be told from none at this tolerance",
        ));
    }
    // A distance too far is one of the whole thickness.
    let whole = |error: KernelError| match (error, side) {
        (
            KernelError::FaceOpTooFar {
                operation,
                faces,
                edge,
                reason,
                value,
                largest,
            },
            Side::Both,
        ) => KernelError::FaceOpTooFar {
            operation,
            faces,
            edge,
            reason,
            value: 2.0 * value,
            largest: 2.0 * largest,
        },
        (other, _) => other,
    };
    let outer = moved(body, faces, out, blends, tolerance).map_err(whole)?;
    let inner = moved(body, faces, -into, blends, tolerance).map_err(whole)?;
    // Both passed the checker a moment ago.
    let options = BooleanOptions {
        trust_operands: true,
        ..BooleanOptions::default()
    };
    let cut = boolean_with(&outer, &inner, BooleanOp::Subtract, tolerance, &options).map_err(
        |error| KernelError::FaceOpBoolean {
            operation: OPERATION,
            error: Box::new(error),
        },
    )?;
    let skins: Vec<(Skin, FaceId)> = cut
        .faces
        .iter()
        .map(|source| {
            let skin = match source.operand {
                crate::boolean::Operand::A => Skin::Outer,
                crate::boolean::Operand::B => Skin::Inner,
            };
            (skin, source.face)
        })
        .collect();
    // What a layer is: both sides of every face of the set.
    for face in faces {
        for skin in [Skin::Outer, Skin::Inner] {
            if !skins.contains(&(skin, *face)) {
                return Err(refuse(
                    OPERATION,
                    vec![*face],
                    None,
                    "the layer behind this face did not come out whole: another part of the body runs into it",
                ));
            }
        }
    }
    let skin = |operand| match operand {
        crate::boolean::Operand::A => Skin::Outer,
        crate::boolean::Operand::B => Skin::Inner,
    };
    let edges = cut
        .edges
        .iter()
        .map(|source| match source {
            EdgeSource::Kept { operand, edge } => Some((skin(*operand), *edge)),
            EdgeSource::Intersection { .. } => None,
        })
        .collect();
    let vertices = cut
        .vertices
        .iter()
        .map(|source| match source {
            VertexSource::Kept { operand, vertex } => Some((skin(*operand), *vertex)),
            VertexSource::New { .. } => None,
        })
        .collect();
    Ok(Shelled {
        body: cut.body,
        faces: skins,
        edges,
        vertices,
    })
}
