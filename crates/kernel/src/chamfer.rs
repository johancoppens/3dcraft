//! Chamfers: an edge of a solid replaced by a flat or conical face. The
//! two faces next to the edge are cut back by a distance each, measured
//! along the face and at right angles to the edge, and the new face joins
//! the two cut lines. A convex edge loses material, a concave edge gains
//! it.
//!
//! The method (a tool per edge and the boolean), where an edge ends, the
//! flat corner where three chamfers meet and the check that no wrong
//! solid is returned are described in [`crate::blend`], which chamfers
//! and fillets share.

use crate::KernelError;
use crate::blend::{self, BlendError, Blended, Face, Kind, Request, Size};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, EdgeId, FaceId, VertexId};

pub use crate::blend::tangent_chain;

/// Most edges one chamfer takes.
pub const MAX_CHAMFER_EDGES: usize = blend::MAX_EDGES;

/// How far back the two faces of an edge are cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChamferSize {
    /// `first` along the first face, `second` along the other.
    Distances { first: f64, second: f64 },
    /// `distance` along the first face; the chamfer face leaves that face
    /// at `angle` (radians, between the first face and the chamfer face,
    /// measured inside the corner that is cut away or filled).
    DistanceAngle { distance: f64, angle: f64 },
}

/// One edge to chamfer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChamferEdge {
    pub edge: EdgeId,
    /// Which of the two faces of the edge the first distance is measured
    /// along.
    pub first: FaceId,
    pub size: ChamferSize,
}

/// Where a face of a chamfered body came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChamferFace {
    /// A face of the body, cut back or grown: the faces of the body it is
    /// made of (more than one when the chamfer left faces on one surface
    /// that are one face now), in ascending order.
    Kept(Vec<FaceId>),
    /// The chamfer face of the edges with these indices in the request
    /// (more than one when chamfer faces in one plane became one face).
    Chamfer(Vec<usize>),
    /// The corner face at a vertex of the body where three chamfers meet.
    Corner(VertexId),
}

/// A chamfered body and where each of its entities came from:
/// `faces[i]` belongs to `FaceId(i)` of `body`, and so on. An edge or a
/// vertex has the id it had in the body when it is one of that body.
#[derive(Clone, Debug, PartialEq)]
pub struct ChamferResult {
    pub body: Body,
    pub faces: Vec<ChamferFace>,
    pub edges: Vec<Option<EdgeId>>,
    pub vertices: Vec<Option<VertexId>>,
}

/// A blend's refusal in the words of a chamfer.
pub(crate) fn said(error: BlendError) -> KernelError {
    match error {
        BlendError::Kernel(error) => error,
        BlendError::TooLarge {
            edge,
            face,
            size,
            largest,
        } => KernelError::ChamferTooLarge {
            edge,
            face,
            distance: size,
            largest,
        },
        BlendError::Refused { edge, face, reason } => KernelError::Chamfer {
            edge,
            face,
            reason: reason.text(Kind::Chamfer),
        },
        BlendError::Boolean { edge, error } => KernelError::ChamferBoolean { edge, error },
    }
}

/// Chamfers `edges` of the solid `body`.
///
/// The result has passed `craft3d_topo::check` with `tolerance` and is
/// reported with the source of every face, edge and vertex. See
/// [`crate::blend`] for the method, for what the corner is where
/// chamfers meet, and for what is refused.
///
/// **Errors.** [`KernelError::ChamferTooLarge`] for a distance whose cut
/// line leaves a face, with the edge, the face and the largest distance
/// that fits; [`KernelError::Chamfer`] for an edge that cannot be
/// chamfered or is not built yet, and for a result that is not a chamfer;
/// [`KernelError::ChamferBoolean`] when the boolean refuses a tool, with
/// the edge and the boolean's own error.
pub fn chamfer(
    body: &Body,
    edges: &[ChamferEdge],
    tolerance: &Tolerance,
) -> Result<ChamferResult, KernelError> {
    let requests: Vec<Request> = edges
        .iter()
        .map(|e| Request {
            edge: e.edge,
            first: Some(e.first),
            size: Size::Chamfer(e.size),
        })
        .collect();
    let Blended {
        body,
        faces,
        edges,
        vertices,
    } = blend::blend(body, &requests, tolerance).map_err(said)?;
    let faces = faces
        .into_iter()
        .map(|face| match face {
            Face::Kept(faces) => ChamferFace::Kept(faces),
            Face::Blend(edges) => ChamferFace::Chamfer(edges),
            Face::Corner(vertex) => ChamferFace::Corner(vertex),
        })
        .collect();
    Ok(ChamferResult {
        body,
        faces,
        edges,
        vertices,
    })
}
