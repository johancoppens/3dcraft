//! Shell: a solid made hollow, with walls of one thickness behind every
//! face except the ones left open.
//!
//! **Method.** The hollow is a body of its own: the solid with every
//! face that keeps a wall moved into the material by the thickness
//! ([`crate::offset_face`]: flat faces to parallel planes, cylinders,
//! cones and spheres about their axes, the edges between them found
//! again), and every open face moved outward by the same amount, so that
//! the hollow reaches through it and the opening is as wide as the
//! hollow is there. The shell is the solid less that body
//! ([`crate::boolean`]), which crosses the open faces squarely and
//! touches nothing else. Without open faces the hollow lies inside the
//! solid and the result has two shells: the skin and a cavity.
//!
//! The offset is the local operation, so the hollow has, face for face,
//! the faces of the solid: a face of the result is a piece of the skin
//! of the solid or the face of the hollow behind one, and is reported so.
//!
//! **Never a wrong solid.** A thickness at which a face of the hollow
//! vanishes (half the width of a wall, the radius of a rounded edge) is
//! refused with the face or the edge and the largest thickness that
//! fits. After the boolean the result is held against what a shell is:
//! every face that keeps a wall is there as skin and as a face of the
//! hollow, and the hollow comes out through every open face.

use crate::KernelError;
use crate::boolean::{BooleanOp, BooleanOptions, EdgeSource, Operand, VertexSource, boolean_with};
use crate::offset_face::{FaceOffset, offset_for, refuse};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, EdgeId, FaceId, VertexId};

const OPERATION: &str = "shell";

/// Which of the two skins of a shell an entity is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Skin {
    /// The skin of the solid that was made hollow.
    Outer,
    /// The hollow: its entities are those of the solid, moved inward.
    Inner,
}

/// A shell and where each of its entities came from: `faces[i]` says of
/// `FaceId(i)` of `body` which face of the solid it is a piece of (outer
/// skin) or lies behind (the hollow), and so on.
#[derive(Clone, Debug, PartialEq)]
pub struct Shelled {
    pub body: Body,
    pub faces: Vec<(Skin, FaceId)>,
    pub edges: Vec<Option<(Skin, EdgeId)>>,
    pub vertices: Vec<Option<(Skin, VertexId)>>,
}

fn skin(operand: Operand) -> Skin {
    match operand {
        Operand::A => Skin::Outer,
        Operand::B => Skin::Inner,
    }
}

/// `body` made hollow with walls of `thickness`, open at the faces
/// `open` (see the module documentation).
///
/// **Errors.** [`KernelError::FaceOpTooFar`] for a thickness at which an
/// edge or a face of the hollow would vanish, with the largest that
/// fits; [`KernelError::FaceOp`] for what is not built (the faces the
/// offset refuses) or not a face; [`KernelError::FaceOpBoolean`] when the
/// boolean refuses the hollow.
pub fn shell(
    body: &Body,
    thickness: f64,
    open: &[FaceId],
    tolerance: &Tolerance,
) -> Result<Shelled, KernelError> {
    if !(thickness.is_finite() && thickness > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "thickness",
            value: thickness,
        });
    }
    for (index, face) in open.iter().enumerate() {
        if body.face(*face).is_none() {
            return Err(refuse(
                OPERATION,
                vec![*face],
                None,
                "there is no such face in the body",
            ));
        }
        if open.iter().take(index).any(|earlier| earlier == face) {
            return Err(refuse(
                OPERATION,
                vec![*face],
                None,
                "the face is given twice",
            ));
        }
    }
    if open.len() >= body.faces.len() {
        return Err(refuse(
            OPERATION,
            Vec::new(),
            None,
            "every face is left open: there is nothing to leave a wall behind",
        ));
    }
    let offsets: Vec<FaceOffset> = (0..body.faces.len())
        .filter_map(|index| u32::try_from(index).ok().map(FaceId))
        .map(|face| FaceOffset {
            face,
            distance: if open.contains(&face) {
                thickness
            } else {
                -thickness
            },
        })
        .collect();
    let hollow = offset_for(body, &offsets, OPERATION, tolerance)?;
    // Both passed the checker a moment ago.
    let options = BooleanOptions {
        trust_operands: true,
        ..BooleanOptions::default()
    };
    let cut =
        boolean_with(body, &hollow, BooleanOp::Subtract, tolerance, &options).map_err(|error| {
            KernelError::FaceOpBoolean {
                operation: OPERATION,
                error: Box::new(error),
            }
        })?;
    let faces: Vec<(Skin, FaceId)> = cut
        .faces
        .iter()
        .map(|source| (skin(source.operand), source.face))
        .collect();
    // What a shell is.
    for index in 0..body.faces.len() {
        let Ok(face) = u32::try_from(index).map(FaceId) else {
            continue;
        };
        let has = |which: Skin| faces.contains(&(which, face));
        if open.contains(&face) {
            // The hollow comes out through it: the face is gone, or the
            // hollow cuts a hole in it. (Behind an open face between
            // walls that stand round it, as the floor of a pocket, the
            // hollow is wider than the opening, and a piece of the face
            // moved out is the hollow's ceiling there.)
            let crossed = cut
                .edges
                .iter()
                .any(|source| matches!(source, EdgeSource::Intersection { a, .. } if *a == face));
            if has(Skin::Outer) && !crossed {
                return Err(refuse(
                    OPERATION,
                    vec![face],
                    None,
                    "the hollow does not come out through this open face: it is closed off behind it",
                ));
            }
        } else if !has(Skin::Outer) || !has(Skin::Inner) {
            return Err(refuse(
                OPERATION,
                vec![face],
                None,
                "the wall behind this face did not come out whole: the hollow cuts through it or misses it",
            ));
        }
    }
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
        faces,
        edges,
        vertices,
    })
}
