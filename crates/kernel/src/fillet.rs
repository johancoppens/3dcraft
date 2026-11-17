//! Fillets: an edge of a solid replaced by the face a ball of the radius
//! touches as it rolls along the edge in contact with both faces. The new
//! face is tangent to both. A convex edge loses material (a round), a
//! concave edge gains it. The radius is one all along ([`fillet`]) or
//! varies along the edge by a law ([`fillet_by_law`], [`RadiusLaw`]).
//!
//! Along a straight edge between two planes the face is a piece of a
//! cylinder of the radius, whose axis runs parallel to the edge at the
//! radius from both planes; along a circular edge between a plane, a
//! cylinder or a cone about the circle's axis it is a piece of a torus
//! whose tube has the radius. Where three straight filleted edges of one
//! radius meet, the corner is a piece of the ball itself.
//!
//! The method (a tool per edge and the boolean), where an edge ends,
//! corners and the check that no wrong solid is returned are described in
//! [`crate::blend`], which fillets and chamfers share.

use crate::KernelError;
use crate::blend::{self, BlendError, Blended, Face, Kind, Request, Size};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, EdgeId, FaceId, VertexId};

pub use crate::blend::law::{Fade, RadiusLaw};
pub use crate::blend::tangent_chain;

/// Most edges one fillet takes.
pub const MAX_FILLET_EDGES: usize = blend::MAX_EDGES;

/// One edge to fillet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilletEdge {
    pub edge: EdgeId,
    pub radius: f64,
}

/// One edge to fillet with a radius that may vary along it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilletLawEdge {
    pub edge: EdgeId,
    pub law: RadiusLaw,
}

/// Where a face of a filleted body came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilletFace {
    /// A face of the body, cut back or grown: the faces of the body it is
    /// made of (more than one when the fillet left faces on one surface
    /// that are one face now), in ascending order.
    Kept(Vec<FaceId>),
    /// The fillet face of the edges with these indices in the request
    /// (more than one when fillet faces on one surface became one face).
    Fillet(Vec<usize>),
    /// The piece of a ball at a vertex of the body where three fillets
    /// meet.
    Corner(VertexId),
}

/// A filleted body and where each of its entities came from: `faces[i]`
/// belongs to `FaceId(i)` of `body`, and so on. An edge or a vertex has
/// the id it had in the body when it is one of that body.
#[derive(Clone, Debug, PartialEq)]
pub struct FilletResult {
    pub body: Body,
    pub faces: Vec<FilletFace>,
    pub edges: Vec<Option<EdgeId>>,
    pub vertices: Vec<Option<VertexId>>,
}

/// A blend's refusal in the words of a fillet.
fn said(error: BlendError) -> KernelError {
    match error {
        BlendError::Kernel(error) => error,
        BlendError::TooLarge {
            edge,
            face,
            size,
            largest,
        } => KernelError::FilletTooLarge {
            edge,
            face,
            radius: size,
            largest,
        },
        BlendError::Refused { edge, face, reason } => KernelError::Fillet {
            edge,
            face,
            reason: reason.text(Kind::Fillet),
        },
        BlendError::Spindle {
            edge,
            radius,
            below,
            own,
        } => KernelError::FilletSpindle {
            edge,
            radius,
            below,
            own,
        },
        BlendError::Boolean {
            edge,
            error,
            smaller_fails,
        } => KernelError::FilletBoolean {
            edge,
            error,
            smaller_fails,
        },
        BlendError::Pinched { edge, pinch } => KernelError::FilletPinched {
            edge,
            at: pinch.at,
            why: pinch.why,
            places: pinch.places,
        },
    }
}

/// Fillets `edges` of the solid `body`, each with its radius.
///
/// The result has passed `craft3d_topo::check` with `tolerance`, every
/// fillet face in it is tangent to both neighbours of its edge within
/// `tolerance.angular`, and it is reported with the source of every face,
/// edge and vertex. See [`crate::blend`] for the method, for corners, and
/// for what is refused.
///
/// **Errors.** [`KernelError::FilletTooLarge`] for a radius whose tangent
/// line leaves a face, with the edge, the face and the largest radius
/// that fits; [`KernelError::Fillet`] for an edge that cannot be filleted
/// or is not built yet, for a corner that is not built, and for a result
/// that is not a fillet; [`KernelError::FilletBoolean`] when the boolean
/// refuses a tool, with the edge and the boolean's own error.
pub fn fillet(
    body: &Body,
    edges: &[FilletEdge],
    tolerance: &Tolerance,
) -> Result<FilletResult, KernelError> {
    fillet_capped(body, edges, tolerance, None)
}

/// [`fillet`] held to the document's cap on an entity's own tolerance
/// (`loosest`; `None` for the default, `topo::DEFAULT_LOOSEST` times the
/// linear tolerance): a body with loose entities above it is refused.
pub fn fillet_capped(
    body: &Body,
    edges: &[FilletEdge],
    tolerance: &Tolerance,
    loosest: Option<f64>,
) -> Result<FilletResult, KernelError> {
    crate::loose_op::run(
        "fillet",
        body,
        tolerance,
        loosest,
        |at| fillet_at(body, edges, at),
        |r| {
            let sources = crate::loose_op::Sources {
                faces: r
                    .faces
                    .iter()
                    .map(|f| match f {
                        FilletFace::Kept(kept) if kept.len() == 1 => kept.first().copied(),
                        _ => None,
                    })
                    .collect(),
                edges: r.edges.clone(),
                vertices: r.vertices.clone(),
            };
            (&mut r.body, sources)
        },
    )
}

/// Fillets `edges` of the solid `body`, each with its radius as a law
/// along the edge ([`RadiusLaw`]): one radius along it, running to another
/// at an end that has a [`Fade`], over the fade's length along the edge.
///
/// An edge whose law is one radius all along is filleted as [`fillet`]
/// fillets it. An edge whose radius varies is built in place by the
/// rolling ball whose radius is the law's in each plane across the edge:
/// the fillet face is the arcs between the points where those balls touch
/// the two faces, tangent to both faces along its borders. It is built
/// along an open edge on its own whose ends are corners of exactly three
/// faces; round a closed edge, in a chain, where another requested edge
/// ends in one of its vertices and where its border would cross a smooth
/// edge of a face beside it, it is refused ([`KernelError::Fillet`]).
///
/// **Errors.** Those of [`fillet`], and [`KernelError::InvalidSize`] for
/// a radius or a length in a law that is not a size (finite and above
/// zero: a radius that runs to nothing at an end is not built yet).
pub fn fillet_by_law(
    body: &Body,
    edges: &[FilletLawEdge],
    tolerance: &Tolerance,
) -> Result<FilletResult, KernelError> {
    fillet_by_law_capped(body, edges, tolerance, None)
}

/// [`fillet_by_law`] held to the document's cap on an entity's own
/// tolerance, as [`fillet_capped`] is.
pub fn fillet_by_law_capped(
    body: &Body,
    edges: &[FilletLawEdge],
    tolerance: &Tolerance,
    loosest: Option<f64>,
) -> Result<FilletResult, KernelError> {
    let mut requests = Vec::with_capacity(edges.len());
    for e in edges {
        let law = e.law.checked()?;
        requests.push(Request {
            edge: e.edge,
            first: None,
            // One radius all along is the fillet of that radius, built as
            // every such fillet is.
            size: if law.is_constant() {
                Size::Radius(law.radius)
            } else {
                Size::Law(law)
            },
        });
    }
    crate::loose_op::run(
        "fillet",
        body,
        tolerance,
        loosest,
        |at| fillet_requests(body, &requests, at),
        loose_sources,
    )
}

/// What `loose_op` needs of a result: its body and where its entities
/// came from.
fn loose_sources(r: &mut FilletResult) -> (&mut Body, crate::loose_op::Sources) {
    let sources = crate::loose_op::Sources {
        faces: r
            .faces
            .iter()
            .map(|f| match f {
                FilletFace::Kept(kept) if kept.len() == 1 => kept.first().copied(),
                _ => None,
            })
            .collect(),
        edges: r.edges.clone(),
        vertices: r.vertices.clone(),
    };
    (&mut r.body, sources)
}

/// The operation itself, at the tolerance given.
fn fillet_at(
    body: &Body,
    edges: &[FilletEdge],
    tolerance: &Tolerance,
) -> Result<FilletResult, KernelError> {
    let requests: Vec<Request> = edges
        .iter()
        .map(|e| Request {
            edge: e.edge,
            first: None,
            size: Size::Radius(e.radius),
        })
        .collect();
    fillet_requests(body, &requests, tolerance)
}

/// The requests blended, in the words of a fillet.
fn fillet_requests(
    body: &Body,
    requests: &[Request],
    tolerance: &Tolerance,
) -> Result<FilletResult, KernelError> {
    let Blended {
        body,
        faces,
        edges,
        vertices,
    } = blend::blend(body, requests, tolerance).map_err(said)?;
    let faces = faces
        .into_iter()
        .map(|face| match face {
            Face::Kept(faces) => FilletFace::Kept(faces),
            Face::Blend(edges) => FilletFace::Fillet(edges),
            Face::Corner(vertex) => FilletFace::Corner(vertex),
        })
        .collect();
    Ok(FilletResult {
        body,
        faces,
        edges,
        vertices,
    })
}
