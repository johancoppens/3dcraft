//! Blends: an edge of a solid replaced by a face. What [`crate::chamfer`]
//! and [`crate::fillet`] have in common, which is nearly everything: how
//! an edge is read from the solid, the tool that is swept along it, where
//! it ends, what happens where blends meet, and the check at the end.
//!
//! **The section.** Across the edge the two faces leave it in two
//! directions. A chamfer cuts each face back by a distance along it and
//! joins the two cut lines by a straight line; a fillet cuts both back to
//! where a ball of its radius touches them (the radius over the tangent
//! of half the angle between the two directions) and joins the two
//! tangent lines by the arc of the ball between them. Either way the
//! section of the tool is the region between the edge and that line or
//! arc. A convex edge loses it, a concave edge gains it.
//!
//! **Method: a tool per edge and the boolean.** The section is swept
//! along the edge: a prism along a straight edge between two planes, a
//! ring about the axis of a circular edge between a plane, a cylinder or
//! a cone that share that axis. The tool is taken out of the body
//! (convex) or added to it (concave) by [`crate::boolean`], one edge
//! after the other. Two faces of the tool lie in the two neighbours of
//! the edge, which the boolean treats as coincident faces; the third is
//! the blend face: a plane or a cone for a chamfer, a cylinder or a torus
//! for a fillet, which is tangent to both neighbours along the lines
//! where it ends. Every tool is made from the body as it was given, so
//! the order of the edges does not change the solid.
//!
//! **Where an edge ends** the blend face has to stop against whatever is
//! at the vertex:
//!
//! - When every other edge at the vertex is convex like the blended one
//!   (or every one concave), the body near the vertex is what its faces
//!   leave of space, the blend is one more such face, and the tool is
//!   made longer than the edge: what sticks out meets nothing (convex) or
//!   only material (concave).
//! - Otherwise the tool ends at the vertex, in the plane across the edge
//!   there. That is right when the face the blend runs into lies in that
//!   plane (an edge of a step that ends against a wall), and when the
//!   edge continues smoothly in another blended edge (a chain: the two
//!   tools end against each other).
//! - When the one other face at the vertex is a plane at a slant to the
//!   edge (a root that ends on a chamfer, a ledge that runs into a
//!   slanted wall), the tool is made long enough to reach that plane and
//!   cut there ([`end_plane`], [`trim`]): its end lies in the plane of the
//!   face, which the boolean merges it into.
//! - In every other case the end of the tool would be left in the result
//!   as a face nobody asked for, which the check at the end finds and
//!   refuses.
//!
//! **Corners of chamfers.** Where three chamfered edges of one kind (all
//! convex or all concave) meet in a vertex with nothing else, the corner
//! is a flat face through the three points in which the cut lines cross
//! on the three faces: a triangle whatever the distances are, since three
//! points are always in a plane. (When one of the three chamfers is
//! larger than the other two reach together, it takes the corner whole:
//! those crossings are not points of the body, and there is no corner
//! face.) Where two chamfered edges meet, their faces meet in the line
//! both planes share.
//!
//! **Corners of fillets.** Two filleted edges that meet, with the other
//! edges at the vertex left sharp: each tool is its whole section near
//! the vertex, so the two faces run out into each other in the curve the
//! two surfaces share (two cylinders: an arc of an ellipse). Three
//! straight filleted edges of one kind and one radius that meet in a
//! vertex with nothing else: the ball that rolls along each of them comes
//! to rest in the corner, touching all three faces, and the corner is the
//! piece of that ball between the three planes through its centre across
//! the three edges (on a block an eighth of a sphere). Each of the three
//! cylinders has its axis through the ball's centre and ends on the ball
//! in a great circle, where the two are tangent. It is made by one more
//! boolean with the tool "the cell those three planes cut out on the
//! vertex's side, less the ball". Three or more filleted edges at a
//! vertex in any other arrangement (different radii, a circular edge, a
//! fourth edge) need a vertex blend that is not built, and are refused.
//!
//! **Never a wrong solid.** Whether a size fits is not estimated: the
//! blend is built and checked, and a size that does not build is refused
//! with the largest that does, found by building again ([`fit`]). After
//! the last boolean the result is held
//! against what a blend is: every blend face is there, in one piece,
//! borders both neighbours of its edge, and borders nothing but those,
//! the faces at the ends of its edge and the blends and corners there; no
//! other face of a tool is left; and a fillet face is tangent to both its
//! neighbours, and to the ball in a corner, within the angular tolerance
//! at points sampled along the seams. Anything else is an error that
//! names the edge.

use crate::KernelError;
use crate::boolean::{
    BooleanOp, BooleanOptions, BooleanResult, EdgeSource, FaceSource, Operand, VertexSource,
    boolean_with,
};
use crate::chamfer::ChamferSize;
use crate::extrude::{Role, extrude};
use crate::primitives;
use crate::profile::{Curve2, SketchCurve, detect_profiles};
use crate::revolve::revolve;
use craft3d_geom::{Curve3, GeomError, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, EdgeId, FaceId, VertexId};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

mod fit;
mod rolling;
mod section;
use section::{Section, Trace};

/// Most edges one blend takes.
pub(crate) const MAX_EDGES: usize = 4096;

/// Which blend is asked for: the words of what is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Chamfer,
    Fillet,
}

/// The size of the blend of one edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Size {
    Chamfer(ChamferSize),
    /// The radius of a fillet.
    Radius(f64),
}

/// One edge to blend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Request {
    pub edge: EdgeId,
    /// Which of the two faces of the edge is the first, when that matters
    /// (a chamfer's first distance is measured along it).
    pub first: Option<FaceId>,
    pub size: Size,
}

/// Where a face of a blended body came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Face {
    /// A face of the body, cut back or grown: the faces of the body it is
    /// made of, in ascending order.
    Kept(Vec<FaceId>),
    /// The blend face of the edges with these indices in the request.
    Blend(Vec<usize>),
    /// The corner face at a vertex of the body where three blends meet.
    Corner(VertexId),
}

/// A blended body and where each of its entities came from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Blended {
    pub body: Body,
    pub faces: Vec<Face>,
    pub edges: Vec<Option<EdgeId>>,
    pub vertices: Vec<Option<VertexId>>,
}

/// Why a blend is refused. The text depends on the kind of blend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    NotAnEdge,
    NotAFace,
    Twice,
    NotSolid,
    Smooth,
    Angle,
    Kind,
    KindLine,
    KindCircle,
    KindEllipse,
    KindSpline,
    Spindle,
    EndsBadly,
    Reaches,
    LostNeighbour,
    InPieces,
    InAFace,
    TooMany,
    NoCorner,
    CornerRadii,
    CornerKind,
    NotTangent,
    Lost,
    FreeEnd,
    FreeClosed,
    FreeAngle,
    FreeSurface,
    FreeApproximation,
    FreeTooLarge,
    FreeMeet,
    FreeMixed,
    FreeJoint,
    FreeChainSize,
    FreeRunsOver,
}

impl Reason {
    pub(crate) fn text(self, kind: Kind) -> &'static str {
        match kind {
            Kind::Chamfer => self.chamfer(),
            Kind::Fillet => self.fillet(),
        }
    }

    fn chamfer(self) -> &'static str {
        match self {
            Reason::NotAnEdge => "there is no such edge in the body",
            Reason::NotAFace => "the face is not one of the two faces of the edge",
            Reason::Twice => "the edge is given twice",
            Reason::NotSolid => {
                "only an edge between two faces of a solid can be chamfered (not a seam, nor the border of a sheet)"
            }
            Reason::Smooth => {
                "the two faces are tangent along this edge: there is no corner to chamfer"
            }
            Reason::Angle => {
                "the angle has to be more than zero and less than what is left of half a turn by the angle between the faces"
            }
            Reason::Kind | Reason::Spindle => {
                "this kind of edge is not built yet: a chamfer takes a straight edge between planes and cylinders along it and a circular edge between a plane, a cylinder or a cone about its axis"
            }
            Reason::KindLine => {
                "a straight edge next to a face that is neither a plane nor a cylinder along the edge (a cone, a freeform face) is not built yet (the chamfer face would not be flat)"
            }
            Reason::KindCircle => {
                "a circular edge between faces that are not a plane, a cylinder or a cone about the circle's axis is not built yet"
            }
            Reason::KindEllipse => {
                "an elliptic edge is not built yet (the chamfer face would be a ruled surface)"
            }
            Reason::KindSpline => {
                "a freeform edge is not built yet (the chamfer face would be a ruled surface)"
            }
            Reason::EndsBadly => {
                "the chamfer does not end cleanly: at one end of the edge it would stop in a face of its own, because the faces there are neither all convex or all concave like the edge, nor square to it, nor does the edge go on in another chamfered edge; chamfer the edges it continues in as well, or leave this one out"
            }
            Reason::Reaches => {
                "the chamfer reaches a face that is neither next to the edge nor at one of its ends: the distance is too large, or two chamfers meet in the middle of a face"
            }
            Reason::LostNeighbour => {
                "the chamfer takes away the whole of a face next to the edge, or does not reach it: the distance is too large"
            }
            Reason::InPieces => {
                "the chamfer face came out in pieces or not at all: the distance is too large for what is near the edge"
            }
            Reason::InAFace => {
                "the chamfer face lies in the surface of a face of the body and became part of it"
            }
            Reason::TooMany => "more edges than one chamfer takes",
            Reason::NoCorner | Reason::CornerRadii | Reason::CornerKind | Reason::NotTangent => {
                "the corner where three chamfers meet could not be made flat"
            }
            Reason::FreeEnd => {
                "the chamfer of a freeform edge does not end cleanly: each end of the edge has to be a corner of exactly three faces whose third face the chamfer runs into across the edge"
            }
            Reason::FreeClosed => {
                "a chamfer along a closed freeform edge whose vertex other edges pass through (a seam) is not built yet"
            }
            Reason::FreeAngle => {
                "a chamfer of a freeform edge by a distance and an angle is not built yet: give two distances"
            }
            Reason::FreeSurface => {
                "the chamfer face along this edge could not be followed: the distance does not fit on a face somewhere along it"
            }
            Reason::FreeApproximation => {
                "the chamfer face along this edge could not be approximated within the tolerance"
            }
            Reason::FreeTooLarge => {
                "the chamfer reaches past an edge at the end of the edge: the distance is too large for that face"
            }
            Reason::FreeMeet => {
                "chamfers of freeform edges that meet in a vertex are not built yet: chamfer them one by one, or leave one out"
            }
            Reason::FreeMixed => {
                "a freeform edge cannot be chamfered together with edges between planes, cylinders and cones yet: chamfer them in two features"
            }
            Reason::FreeJoint => {
                "the chamfer cannot be carried on where this edge goes on smoothly into the next: a chain of freeform chamfers is built where both faces go on across the vertex, both edges convex or both concave, and not yet where one of the faces changes"
            }
            Reason::FreeChainSize => {
                "edges that go on smoothly into each other are chamfered by different distances: a chamfer along such a chain has one size; give them the same, or chamfer them in separate features"
            }
            Reason::FreeRunsOver => {
                "the chamfer's border runs off a face beside the edge across one of its other edges along the way: the distance is too large for that face, or the chamfer would have to carry on onto the next face, which is not built yet"
            }
            Reason::Lost => "the chamfer lost track of an entity; this is a bug",
        }
    }

    fn fillet(self) -> &'static str {
        match self {
            Reason::NotAnEdge => "there is no such edge in the body",
            Reason::NotAFace => "the face is not one of the two faces of the edge",
            Reason::Twice => "the edge is given twice",
            Reason::NotSolid => {
                "only an edge between two faces of a solid can be filleted (not a seam, nor the border of a sheet)"
            }
            Reason::Smooth | Reason::Angle => {
                "the two faces are tangent along this edge: there is no corner to round"
            }
            Reason::Kind => {
                "this kind of edge is not built yet: a fillet takes a straight edge between planes and cylinders along it and a circular edge between a plane, a cylinder or a cone about its axis"
            }
            Reason::KindLine => {
                "a straight edge next to a face that is neither a plane nor a cylinder along the edge (a cone, a freeform face) is not built yet (the fillet face would not be a cylinder)"
            }
            Reason::KindCircle => {
                "a circular edge between faces that are not a plane, a cylinder or a cone about the circle's axis is not built yet (the fillet face would not be a torus)"
            }
            Reason::KindEllipse => {
                "an elliptic edge is not built yet (the fillet face would be a canal surface that is neither a cylinder nor a torus)"
            }
            Reason::KindSpline => {
                "a freeform edge is not built yet (the fillet face would be a canal surface that is neither a cylinder nor a torus)"
            }
            Reason::Spindle => {
                "the radius is half the rim's or more: the ball's centre would come nearer to the axis than its own radius, and the fillet face would be a torus that reaches its own axis, which is not built yet"
            }
            Reason::EndsBadly => {
                "the fillet does not end cleanly: at one end of the edge it would stop in a face of its own, because the faces there are neither all convex or all concave like the edge, nor square to it, nor does the edge go on in another filleted edge; fillet the edges it continues in as well, or leave this one out"
            }
            Reason::Reaches => {
                "the fillet reaches a face that is neither next to the edge nor at one of its ends: the radius is too large, or two fillets meet in the middle of a face"
            }
            Reason::LostNeighbour => {
                "the fillet takes away the whole of a face next to the edge, or does not reach it: the radius is too large"
            }
            Reason::InPieces => {
                "the fillet face came out in pieces or not at all: the radius is too large for what is near the edge"
            }
            Reason::InAFace => {
                "the fillet face lies in the surface of a face of the body and became part of it"
            }
            Reason::TooMany => "more edges than one fillet takes",
            Reason::NoCorner => "the corner where three fillets meet could not be made",
            Reason::CornerRadii => {
                "fillets of different radii meet in this corner: the vertex blend between them is not built yet; give the edges that meet there one radius, or leave one of them sharp"
            }
            Reason::CornerKind => {
                "three or more fillets meet at the end of this edge in a corner that is not built yet: a corner is made where exactly three straight edges meet, all convex or all concave, all filleted with one radius; leave one of the edges sharp (two fillets run out into each other)"
            }
            Reason::NotTangent => {
                "the fillet face came out not tangent to a face it should run into smoothly; this is a bug"
            }
            Reason::FreeEnd => {
                "the fillet of a freeform edge does not end cleanly: each end of the edge has to be a corner of exactly three faces whose third face the fillet runs into across the edge"
            }
            Reason::FreeClosed => {
                "a fillet along a closed freeform edge whose vertex other edges pass through (a seam) is not built yet"
            }
            Reason::FreeAngle => "a fillet has no angle",
            Reason::FreeSurface => {
                "the rolling ball along this edge could not be followed: the radius does not fit between the faces somewhere along it, or a face turns too sharply under it"
            }
            Reason::FreeApproximation => {
                "the fillet face along this edge could not be approximated within the tolerance"
            }
            Reason::FreeTooLarge => {
                "the fillet reaches past an edge at the end of the edge: the radius is too large for that face"
            }
            Reason::FreeMeet => {
                "fillets of freeform edges that meet in a vertex are not built yet: fillet them one by one, or leave one out"
            }
            Reason::FreeMixed => {
                "a freeform edge cannot be filleted together with edges between planes, cylinders and cones yet: fillet them in two features"
            }
            Reason::FreeJoint => {
                "the fillet cannot be carried on where this edge goes on smoothly into the next: both edges have to be convex or both concave, and across the vertex either both faces go on or one of them goes on in a face tangent to it across a smooth edge, with no other edge there; otherwise the ball that touches both there could not be found"
            }
            Reason::FreeChainSize => {
                "edges that go on smoothly into each other are filleted with different radii: a fillet along such a chain has one radius; give them the same, or fillet them in separate features"
            }
            Reason::FreeRunsOver => {
                "the fillet's border runs off a face beside the edge across one of its other edges along the way (where the ball would roll on over a smooth edge onto the next face, or the face is too narrow for the radius); a fillet that carries on onto the next face in the middle of an edge is not built yet: give a smaller radius"
            }
            Reason::Lost => "the fillet lost track of an entity; this is a bug",
        }
    }
}

/// What a blend fails with; [`crate::chamfer`] and [`crate::fillet`] say
/// it in their own variants of [`KernelError`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BlendError {
    /// A size that is not one, a body that is not valid, geometry.
    Kernel(KernelError),
    /// A size for which the cut or tangent line would leave `face`.
    TooLarge {
        edge: EdgeId,
        face: FaceId,
        size: f64,
        largest: f64,
    },
    Refused {
        edge: Option<EdgeId>,
        face: Option<FaceId>,
        reason: Reason,
    },
    /// The boolean refused the tool of an edge.
    Boolean {
        edge: EdgeId,
        error: Box<KernelError>,
        /// Smaller sizes, tried, fail the same way.
        smaller_fails: bool,
    },
}

impl From<KernelError> for BlendError {
    fn from(error: KernelError) -> Self {
        BlendError::Kernel(error)
    }
}

impl From<GeomError> for BlendError {
    fn from(error: GeomError) -> Self {
        BlendError::Kernel(KernelError::Geometry(error))
    }
}

fn refused(edge: EdgeId, reason: Reason) -> BlendError {
    BlendError::Refused {
        edge: Some(edge),
        face: None,
        reason,
    }
}

fn lost() -> BlendError {
    BlendError::Refused {
        edge: None,
        face: None,
        reason: Reason::Lost,
    }
}

/// One use of an edge by a loop of a face.
#[derive(Clone, Copy, Debug)]
struct Use {
    face: FaceId,
    reversed: bool,
}

fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// Per edge, the faces that use it.
fn edge_uses(body: &Body) -> Vec<Vec<Use>> {
    let mut uses: Vec<Vec<Use>> = vec![Vec::new(); body.edges.len()];
    for (index, face) in body.faces.iter().enumerate() {
        for coedge in face
            .loops
            .iter()
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges)
        {
            if let Some(list) = uses.get_mut(coedge.edge.0 as usize) {
                list.push(Use {
                    face: FaceId(index_u32(index)),
                    reversed: coedge.reversed,
                });
            }
        }
    }
    uses
}

/// Per vertex, the edges that end in it.
fn vertex_edges(body: &Body) -> Vec<Vec<EdgeId>> {
    let mut edges: Vec<Vec<EdgeId>> = vec![Vec::new(); body.vertices.len()];
    for (index, edge) in body.edges.iter().enumerate() {
        let id = EdgeId(index_u32(index));
        for vertex in [edge.start, edge.end] {
            if let Some(list) = edges.get_mut(vertex.0 as usize)
                && !list.contains(&id)
            {
                list.push(id);
            }
        }
    }
    edges
}

/// The outward normal of `face` at the point of it nearest `point`.
fn outward(body: &Body, face: FaceId, point: Vec3) -> Result<Vec3, BlendError> {
    let face = body.face(face).ok_or_else(lost)?;
    let (u, v) = face.surface.project(point)?;
    let normal = face
        .surface
        .normal(u, v)
        .ok_or(GeomError::InvalidParameter(
            "a face has no normal where an edge runs along it",
        ))?;
    Ok(if face.reversed { -normal } else { normal })
}

/// The unit tangent of an edge's curve at `t`, in the curve's direction.
fn tangent(curve: &Curve3, t: f64) -> Result<Vec3, BlendError> {
    let (_, derivative) = curve.derivatives(t);
    derivative
        .normalized()
        .ok_or(BlendError::Kernel(KernelError::Geometry(
            GeomError::InvalidParameter("an edge without a direction"),
        )))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turn {
    Convex,
    Concave,
    Smooth,
}

/// What an edge looks like across itself at the middle: the directions
/// into its two faces, at right angles to the edge.
#[derive(Clone, Copy, Debug)]
struct Across {
    /// The two faces, in the order of their uses.
    faces: [FaceId; 2],
    /// The directions along the faces away from the edge.
    into: [Vec3; 2],
    turn: Turn,
}

/// Below this sine two faces are tangent along their edge.
const SMOOTH_SINE: f64 = 1e-6;

/// How far the two faces of `edge` turn against each other where it
/// passes nearest `point` (an end of it): the sine of the angle between
/// their normals, `None` where the edge is not between two faces.
fn sine_at(
    body: &Body,
    uses: &[Vec<Use>],
    edge: EdgeId,
    point: Vec3,
) -> Result<Option<f64>, BlendError> {
    let Some(data) = body.edge(edge) else {
        return Ok(None);
    };
    let Some([a, b]) = uses
        .get(edge.0 as usize)
        .and_then(|list| <[Use; 2]>::try_from(list.as_slice()).ok())
    else {
        return Ok(None);
    };
    if a.face == b.face {
        return Ok(None);
    }
    let t = data.curve.project(point)?;
    let at = data.curve.eval(t);
    let (na, nb) = (outward(body, a.face, at)?, outward(body, b.face, at)?);
    // Normals of opposite sense (a knife edge) are as far apart as can be.
    Ok(Some(if na.dot(nb) > 0.0 {
        na.cross(nb).length()
    } else {
        1.0
    }))
}

fn across(body: &Body, uses: &[Vec<Use>], edge: EdgeId) -> Result<Option<Across>, BlendError> {
    let Some(data) = body.edge(edge) else {
        return Ok(None);
    };
    let Some([a, b]) = uses
        .get(edge.0 as usize)
        .and_then(|list| <[Use; 2]>::try_from(list.as_slice()).ok())
    else {
        return Ok(None);
    };
    if a.face == b.face {
        return Ok(None);
    }
    let middle = 0.5 * (data.range.0 + data.range.1);
    let point = data.curve.eval(middle);
    let along = tangent(&data.curve, middle)?;
    let side = |used: Use| -> Result<(Vec3, Vec3), BlendError> {
        let normal = outward(body, used.face, point)?;
        let run = if used.reversed { -along } else { along };
        // Seen from outside the face lies to the left of its loops.
        let into = normal.cross(run).normalized().ok_or_else(lost)?;
        Ok((normal, into))
    };
    let (_, into_a) = side(a)?;
    let (normal_b, into_b) = side(b)?;
    let sine = into_a.dot(normal_b);
    let turn = if sine < -SMOOTH_SINE {
        Turn::Convex
    } else if sine > SMOOTH_SINE {
        Turn::Concave
    } else {
        Turn::Smooth
    };
    Ok(Some(Across {
        faces: [a.face, b.face],
        into: [into_a, into_b],
        turn,
    }))
}

/// What a tool is swept along.
#[derive(Clone, Copy, Debug)]
enum Path {
    /// A straight edge from `start`, `length` along `along`.
    Line {
        start: Vec3,
        along: Vec3,
        length: f64,
    },
    /// A circle, or an arc of one, about the axis through `centre` along
    /// `axis`. `radial` points from the axis to the middle of the edge,
    /// where the directions into the faces were taken.
    Circle {
        centre: Vec3,
        axis: Vec3,
        radial: Vec3,
        radius: f64,
        /// For an arc: the direction from the axis to its start, and the
        /// angle it turns through about the axis.
        arc: Option<(Vec3, f64)>,
    },
}

/// One edge, worked out.
#[derive(Clone, Debug)]
struct Plan {
    edge: EdgeId,
    /// The first and the second face.
    faces: [FaceId; 2],
    /// The radius of a fillet; `None` for a chamfer.
    round: Option<f64>,
    /// The directions into the first and the second face from the edge,
    /// at the point the section is drawn at.
    into: [Vec3; 2],
    distances: [f64; 2],
    convex: bool,
    path: Path,
    /// The vertices at the start and the end, when the edge has ends.
    ends: Option<[VertexId; 2]>,
    /// How far the tool goes on past the start and past the end: a
    /// length along a line, an angle along an arc.
    longer: [f64; 2],
    /// Where the tool ends at a vertex against a plane that is not square
    /// to the edge: a point of that plane and its normal away from the
    /// edge. The tool is cut there, so that its end lies in the plane of
    /// the face it runs into (see [`end_plane`]).
    trims: [Option<(Vec3, Vec3)>; 2],
    /// Along a straight edge next to a cylinder: the blend in the plane
    /// across the edge ([`section`]). `None` between two planes and along
    /// a circle.
    section: Option<Section>,
    /// What a distance along each face is in the size asked for: one for
    /// a chamfer, the radius over the distance for a fillet.
    scale: [f64; 2],
}

impl Plan {
    fn distance_on(&self, face: FaceId) -> Option<f64> {
        let [a, b] = self.faces;
        let [da, db] = self.distances;
        if face == a {
            Some(da)
        } else if face == b {
            Some(db)
        } else {
            None
        }
    }

    fn largest(&self) -> f64 {
        self.distances[0].max(self.distances[1])
    }

    /// Where the blend leaves the first and the second face: offsets from
    /// the edge in the plane across it.
    fn feet(&self) -> [Vec3; 2] {
        match &self.section {
            Some(section) => section.feet,
            None => [
                self.into[0] * self.distances[0],
                self.into[1] * self.distances[1],
            ],
        }
    }

    /// How far into the face on `side` the point `offset` from the edge
    /// is, measured along the face across the edge.
    fn coordinate(&self, side: usize, offset: Vec3) -> f64 {
        match self.section.as_ref().and_then(|s| s.traces.get(side)) {
            Some(trace) => trace.coordinate(offset),
            None => self.into.get(side).map_or(0.0, |into| offset.dot(*into)),
        }
    }

    /// Between two planes, or along a circle.
    fn flat(&self) -> bool {
        self.section.is_none()
    }
}

fn positive(name: &'static str, value: f64) -> Result<f64, BlendError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(KernelError::InvalidSize { name, value }.into())
    }
}

/// The two distances of a size, given the angle between the directions
/// into the two faces. A fillet's are the same: where the ball touches.
fn distances(
    edge: EdgeId,
    size: Size,
    between: f64,
    half_tan: f64,
    tol: f64,
) -> Result<[f64; 2], BlendError> {
    let (name, pair) = match size {
        Size::Chamfer(ChamferSize::Distances { first, second }) => (
            "distance",
            [positive("distance", first)?, positive("distance", second)?],
        ),
        Size::Chamfer(ChamferSize::DistanceAngle { distance, angle }) => {
            let distance = positive("distance", distance)?;
            if !(angle.is_finite() && angle > 0.0 && angle + between < PI) {
                return Err(refused(edge, Reason::Angle));
            }
            // The sine rule in the triangle of the section.
            let second = distance * angle.sin() / (angle + between).sin();
            if !(second.is_finite() && second > 0.0) {
                return Err(refused(edge, Reason::Angle));
            }
            ("distance", [distance, second])
        }
        Size::Radius(radius) => {
            let radius = positive("radius", radius)?;
            let distance = radius / half_tan;
            if !(half_tan.is_finite() && half_tan > 0.0 && distance.is_finite()) {
                return Err(refused(edge, Reason::Smooth));
            }
            // A fillet narrower than what the booleans tell apart.
            if radius <= 16.0 * tol {
                return Err(KernelError::InvalidSize {
                    name: "radius",
                    value: radius,
                }
                .into());
            }
            ("radius", [distance, distance])
        }
    };
    for value in pair {
        // A chamfer narrower than what the booleans tell apart.
        if value <= 16.0 * tol {
            return Err(KernelError::InvalidSize { name, value }.into());
        }
    }
    Ok(pair)
}

/// True when `surface` is turned about the line through `centre` along
/// `axis`, and is a plane across it, a cylinder or a cone.
fn about_axis(surface: &Surface, centre: Vec3, axis: Vec3, tol: f64, radius: f64) -> bool {
    let parallel = |other: Vec3| other.cross(axis).length() <= 1e-9;
    let on_axis = |point: Vec3| {
        let offset = point - centre;
        (offset - axis * offset.dot(axis)).length() <= tol.max(1e-12 * radius)
    };
    match surface {
        Surface::Plane(plane) => parallel(plane.frame().z()),
        Surface::Cylinder(cylinder) => {
            parallel(cylinder.frame().z()) && on_axis(cylinder.frame().origin())
        }
        Surface::Cone(cone) => parallel(cone.frame().z()) && on_axis(cone.frame().origin()),
        Surface::Sphere(_) | Surface::Torus(_) | Surface::Bezier(_) | Surface::Nurbs(_) => false,
    }
}

fn plan(body: &Body, uses: &[Vec<Use>], request: &Request, tol: f64) -> Result<Plan, BlendError> {
    let edge = request.edge;
    let data = body.edge(edge).ok_or(refused(edge, Reason::NotAnEdge))?;
    let Some(found) = across(body, uses, edge)? else {
        return Err(refused(edge, Reason::NotSolid));
    };
    let [found_a, found_b] = found.faces;
    let [into_a, into_b] = found.into;
    let (faces, into) = match request.first {
        None => (found.faces, found.into),
        Some(first) if first == found_a => (found.faces, found.into),
        Some(first) if first == found_b => ([found_b, found_a], [into_b, into_a]),
        Some(first) => {
            return Err(BlendError::Refused {
                edge: Some(edge),
                face: Some(first),
                reason: Reason::NotAFace,
            });
        }
    };
    let convex = match found.turn {
        Turn::Convex => true,
        Turn::Concave => false,
        Turn::Smooth => return Err(refused(edge, Reason::Smooth)),
    };
    let surfaces = [
        &body.face(faces[0]).ok_or_else(lost)?.surface,
        &body.face(faces[1]).ok_or_else(lost)?.surface,
    ];
    let (path, ends) = match &data.curve {
        Curve3::Line(line) => {
            let along = line.direction();
            let length = data.range.1 - data.range.0;
            if !(length.is_finite() && length > tol) {
                return Err(refused(edge, Reason::Kind));
            }
            (
                Path::Line {
                    start: data.curve.eval(data.range.0),
                    along,
                    length,
                },
                Some([data.start, data.end]),
            )
        }
        Curve3::Circle(circle) => {
            let (centre, axis) = (circle.frame().origin(), circle.frame().z());
            if !surfaces
                .iter()
                .all(|s| about_axis(s, centre, axis, tol, circle.radius()))
            {
                return Err(refused(edge, Reason::KindCircle));
            }
            let sweep = data.range.1 - data.range.0;
            if !(sweep.is_finite() && sweep * circle.radius() > tol && sweep <= TAU * (1.0 + 1e-9))
            {
                return Err(refused(edge, Reason::Kind));
            }
            let whole = data.start == data.end && (sweep - TAU).abs() <= 1e-9 * TAU;
            let from_axis = |t: f64| (data.curve.eval(t) - centre).normalized().ok_or_else(lost);
            let radial = from_axis(0.5 * (data.range.0 + data.range.1))?;
            let (arc, ends) = if whole {
                (None, None)
            } else if data.start == data.end {
                // An arc that closes on one vertex without being the
                // whole circle is not an edge of a valid body.
                return Err(refused(edge, Reason::Kind));
            } else {
                (
                    Some((from_axis(data.range.0)?, sweep)),
                    Some([data.start, data.end]),
                )
            };
            (
                Path::Circle {
                    centre,
                    axis,
                    radial,
                    radius: circle.radius(),
                    arc,
                },
                ends,
            )
        }
        Curve3::Ellipse(_) => return Err(refused(edge, Reason::KindEllipse)),
        Curve3::Nurbs(_) => return Err(refused(edge, Reason::KindSpline)),
    };
    let [into_first, into_second] = into;
    let cosine = into_first.dot(into_second).clamp(-1.0, 1.0);
    let between = cosine.acos();
    // The tangent of half that angle, from the cosine: exactly 1 for a
    // square corner, which the tangent of a rounded quarter turn is not.
    let half_tan = (1.0 - cosine * cosine).max(0.0).sqrt() / (1.0 + cosine);
    let mut distances = distances(edge, request.size, between, half_tan, tol)?;
    let round = match request.size {
        Size::Radius(radius) => Some(radius),
        Size::Chamfer(_) => None,
    };
    let mut scale = match round {
        Some(_) => [half_tan, half_tan],
        None => [1.0, 1.0],
    };
    let mut section = None;
    if let Path::Line { start, along, .. } = path
        && !surfaces.iter().all(|s| matches!(s, Surface::Plane(_)))
    {
        // A cylinder beside a straight edge: the blend is the same in
        // every plane across the edge (see `section`).
        let traced = [
            section::trace(surfaces[0], start, along, into_first, tol),
            section::trace(surfaces[1], start, along, into_second, tol),
        ];
        let [Some(t1), Some(t2)] = traced else {
            return Err(refused(edge, Reason::KindLine));
        };
        let normals = [
            outward(body, faces[0], start)?,
            outward(body, faces[1], start)?,
        ];
        let spread = 1.0 + cosine;
        if spread.is_nan() || spread <= 1e-12 {
            return Err(refused(edge, Reason::Smooth));
        }
        let estimate = (into_first + into_second) * (distances[0] / spread);
        let made = section::section(
            edge,
            faces,
            [t1, t2],
            normals,
            along,
            convex,
            request.size,
            estimate,
            tol,
        )?;
        distances = made.distances;
        if let Some(radius) = round {
            scale = [radius / distances[0], radius / distances[1]];
        }
        section = Some(made);
    }
    if let (
        Some(radius),
        Path::Circle {
            radius: rim,
            radial,
            ..
        },
    ) = (round, &path)
    {
        // The middle of the ball's section has to stay clear of the axis
        // by more than the radius: a torus that reaches its own axis is
        // not a surface the kernel has.
        let [distance, _] = distances;
        let centre = (into_first + into_second) * (distance / (1.0 + cosine));
        let from_axis = rim + centre.dot(*radial);
        if from_axis.is_finite() && from_axis > tol && from_axis - radius <= 2.0 * tol {
            return Err(refused(edge, Reason::Spindle));
        }
    }
    Ok(Plan {
        edge,
        faces,
        round,
        into,
        distances,
        convex,
        path,
        ends,
        longer: [0.0, 0.0],
        trims: [None, None],
        section,
        scale,
    })
}

/// The edges of the loops of `face`.
fn face_edges(body: &Body, face: FaceId) -> Vec<EdgeId> {
    let mut edges = Vec::new();
    for coedge in body
        .face(face)
        .into_iter()
        .flat_map(|f| &f.loops)
        .filter_map(|l| body.loop_(*l))
        .flat_map(|l| &l.coedges)
    {
        if !edges.contains(&coedge.edge) {
            edges.push(coedge.edge);
        }
    }
    edges
}

/// An edge at a vertex: which way it leaves, and how it turns.
struct Leaving {
    edge: EdgeId,
    direction: Vec3,
    turn: Turn,
}

fn leaving(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    vertex: VertexId,
) -> Result<Vec<Leaving>, BlendError> {
    let mut found = Vec::new();
    for &edge in at.get(vertex.0 as usize).into_iter().flatten() {
        let data = body.edge(edge).ok_or_else(lost)?;
        let turn = across(body, uses, edge)?.map_or(Turn::Smooth, |a| a.turn);
        if data.start == vertex {
            found.push(Leaving {
                edge,
                direction: tangent(&data.curve, data.range.0)?,
                turn,
            });
        }
        if data.end == vertex {
            found.push(Leaving {
                edge,
                direction: -tangent(&data.curve, data.range.1)?,
                turn,
            });
        }
    }
    Ok(found)
}

/// Works out how far each tool goes on past the ends of its edge (see
/// the module documentation).
fn lengthen(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    plans: &mut [Plan],
) -> Result<(), BlendError> {
    for plan in plans.iter_mut() {
        let Some(ends) = plan.ends else {
            continue;
        };
        let data = body.edge(plan.edge).ok_or_else(lost)?;
        let own = if plan.convex {
            Turn::Convex
        } else {
            Turn::Concave
        };
        let [f1, f2] = plan.feet();
        let beyond = [
            -tangent(&data.curve, data.range.0)?,
            tangent(&data.curve, data.range.1)?,
        ];
        let mut longer = [0.0, 0.0];
        let mut trims = [None, None];
        for ((slot, trim), (vertex, beyond)) in longer
            .iter_mut()
            .zip(trims.iter_mut())
            .zip(ends.into_iter().zip(beyond))
        {
            let others: Vec<Leaving> = leaving(body, uses, at, vertex)?
                .into_iter()
                .filter(|l| l.edge != plan.edge)
                .collect();
            let goes_on = others.iter().any(|l| l.direction.dot(beyond) > 1.0 - 1e-9);
            if goes_on {
                continue;
            }
            if others.iter().any(|l| l.turn != own) {
                if let Some((far, reach)) = end_plane(body, uses, at, plan, vertex, beyond)? {
                    let corner = body.vertex(vertex).ok_or_else(lost)?.point;
                    *trim = Some((corner, far));
                    *slot = reach;
                }
                continue;
            }
            match plan.path {
                Path::Line { start, along, .. } => {
                    // The chamfer plane holds the edge's direction and
                    // the line between the two cut lines. How far past
                    // the vertex does it cut the other edges there?
                    let Some(normal) = along.cross(f1 - f2).normalized() else {
                        return Err(lost());
                    };
                    let corner = body.vertex(vertex).ok_or_else(lost)?.point;
                    let height = normal.dot(corner - (start + f1));
                    let mut reach = 0.0_f64;
                    for other in &others {
                        let slope = normal.dot(other.direction);
                        if slope.abs() <= 1e-12 {
                            continue;
                        }
                        let s = -height / slope;
                        if s > 0.0 {
                            reach = reach.max(s * other.direction.dot(beyond));
                        }
                    }
                    let cap = 64.0 * plan.largest();
                    *slot = (1.5 * reach).min(cap) + plan.largest();
                }
                Path::Circle { radius, arc, .. } => {
                    // By an angle: as far along the circle as the
                    // chamfer is wide, twice over, and never so far that
                    // the tool closes on itself.
                    let sweep = arc.map_or(TAU, |(_, sweep)| sweep);
                    *slot = (2.0 * plan.largest() / radius).min(0.25 * (TAU - sweep));
                }
            }
        }
        plan.longer = longer;
        plan.trims = trims;
    }
    Ok(())
}

/// Where a straight edge ends at `vertex` against the faces there in a
/// way none of the rules above covers (not all its edges turn like the
/// blended one, and the edge does not go on): when exactly one other face
/// meets there and it is a plane that is not square to the edge, the
/// blend ends in that plane, as a fillet or chamfer that runs out on a
/// face it meets at an angle does in other modellers: the tool is made
/// long enough to reach the plane and cut there, and its end then lies
/// in the plane of that face, which the boolean merges it into. Gives the
/// normal of the plane away from the edge and how far the tool has to go
/// on past the vertex; `None` where this does not apply (the tool ends
/// at the vertex across the edge, as before).
fn end_plane(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    plan: &Plan,
    vertex: VertexId,
    beyond: Vec3,
) -> Result<Option<(Vec3, f64)>, BlendError> {
    if !matches!(plan.path, Path::Line { .. }) {
        return Ok(None);
    }
    let mut faces = BTreeSet::new();
    for edge in at.get(vertex.0 as usize).into_iter().flatten() {
        for used in uses.get(edge.0 as usize).into_iter().flatten() {
            faces.insert(used.face);
        }
    }
    for face in plan.faces {
        faces.remove(&face);
    }
    let [end] = faces.into_iter().collect::<Vec<_>>()[..] else {
        return Ok(None);
    };
    let face = body.face(end).ok_or_else(lost)?;
    let Surface::Plane(plane) = &face.surface else {
        return Ok(None);
    };
    let normal = plane.frame().z();
    let slope = normal.dot(beyond);
    // Square to the edge: the tool's own end lies in the plane already.
    // Along the edge: the plane does not cut it.
    if slope.abs() >= 1.0 - 1e-9 || slope.abs() <= 1e-6 {
        return Ok(None);
    }
    // The normal away from the edge, which lies behind the vertex.
    let far = if slope > 0.0 { normal } else { -normal };
    // How far past the vertex the plane is at the corners of the section
    // (the edge, the two feet and their sum, which hold a fillet's arc).
    let [f1, f2] = plan.feet();
    let mut reach = 0.0_f64;
    for offset in [f1, f2, f1 + f2] {
        let s = -far.dot(offset) / far.dot(beyond);
        if s.is_finite() {
            reach = reach.max(s);
        }
    }
    let largest = plan.largest();
    if reach > 64.0 * largest {
        return Ok(None);
    }
    Ok(Some((far, 1.5 * reach + largest)))
}

/// The tool cut by the plane through `point` square to `far`: what lies
/// beyond it (along `far`) is taken off. `size` bounds the tool.
fn trim(
    tool: Tool,
    point: Vec3,
    far: Vec3,
    size: f64,
    tolerance: &Tolerance,
) -> Result<Tool, BlendError> {
    let frame = Frame::from_z(point, far).ok_or_else(lost)?;
    let corner = frame.point(-size, -size, 0.0);
    let frame = frame.at(corner).ok_or_else(lost)?;
    let block = primitives::block(&frame, 2.0 * size, 2.0 * size, size)?;
    let options = BooleanOptions {
        merge: true,
        trust_operands: true,
        // A tool, not a result: what it touches is the next step's.
        refuse_zero_thickness: false,
    };
    let result = boolean_with(&tool.body, &block, BooleanOp::Subtract, tolerance, &options)?;
    let mut slope = None;
    let mut caps = Vec::new();
    for (index, source) in result.faces.iter().enumerate() {
        if source.operand != Operand::A {
            continue;
        }
        let id = FaceId(index_u32(index));
        if source.face == tool.slope {
            if slope.is_some() {
                return Err(lost());
            }
            slope = Some(id);
        } else if tool.caps.contains(&source.face) {
            caps.push(id);
        }
    }
    Ok(Tool {
        body: result.body,
        slope: slope.ok_or_else(lost)?,
        caps,
    })
}

/// A tool and which of its faces is the chamfer face.
struct Tool {
    body: Body,
    slope: FaceId,
    /// The faces at its two ends, when it has ends.
    caps: Vec<FaceId>,
}

fn line(id: u64, start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// The face of a swept triangle that the curve `1` swept.
fn slope_of(result: crate::extrude::OpResult) -> Result<Tool, BlendError> {
    let slope = result
        .origins
        .faces
        .iter()
        .position(|o| o.role == Role::Side && o.sources == [1])
        .ok_or_else(lost)?;
    let caps = result
        .origins
        .faces
        .iter()
        .enumerate()
        .filter(|(_, o)| matches!(o.role, Role::StartCap | Role::EndCap))
        .map(|(index, _)| FaceId(index_u32(index)))
        .collect();
    Ok(Tool {
        body: result.body,
        slope: FaceId(index_u32(slope)),
        caps,
    })
}

/// A side of a tool's section that lies in a face: a line in a plane, the
/// short arc of the circle in a cylinder.
fn along_trace(
    id: u64,
    trace: &Trace,
    from: [f64; 2],
    to: [f64; 2],
    flat: impl Fn(Vec3) -> [f64; 2],
) -> SketchCurve {
    match trace.circle() {
        None => line(id, from, to),
        Some((centre, _)) => SketchCurve {
            id,
            curve: short_arc(flat(centre), from, to),
        },
    }
}

/// The arc about `center` between two points the short way round,
/// counter-clockwise as a sketch arc runs.
fn short_arc(center: [f64; 2], p1: [f64; 2], p2: [f64; 2]) -> Curve2 {
    let turn =
        (p1[0] - center[0]) * (p2[1] - center[1]) - (p1[1] - center[1]) * (p2[0] - center[0]);
    let (start, end) = if turn > 0.0 { (p1, p2) } else { (p2, p1) };
    Curve2::Arc { center, start, end }
}

/// The curve `1` of a tool's section, from the cut line on the first
/// face to the one on the second: straight for a chamfer, and for a
/// fillet the arc of the ball that touches both faces there. `flat`
/// gives a direction in the coordinates of the section, whose origin is
/// at `rim`.
fn across_curve(
    plan: &Plan,
    rim: [f64; 2],
    p1: [f64; 2],
    p2: [f64; 2],
    flat: impl Fn(Vec3) -> [f64; 2],
) -> Result<SketchCurve, BlendError> {
    if plan.round.is_none() {
        return Ok(line(1, p1, p2));
    }
    let [b1, b2] = plan.into;
    let [d1, _] = plan.distances;
    let ball = match plan.section.as_ref().and_then(|s| s.centre) {
        Some(centre) => centre,
        None => {
            // The ball's centre lies on the bisector, where the
            // perpendiculars of the two tangent points meet.
            let spread = 1.0 + b1.dot(b2);
            if spread <= 1e-12 {
                return Err(lost());
            }
            (b1 + b2) * (d1 / spread)
        }
    };
    let [cx, cy] = flat(ball);
    let center = [rim[0] + cx, rim[1] + cy];
    Ok(SketchCurve {
        id: 1,
        curve: short_arc(center, p1, p2),
    })
}

fn tool(plan: &Plan, tolerance: &Tolerance) -> Result<Tool, BlendError> {
    let [d1, d2] = plan.distances;
    let [b1, b2] = plan.into;
    match plan.path {
        Path::Line {
            start,
            along,
            length,
        } => {
            let origin = start - along * plan.longer[0];
            let frame = Frame::new(origin, along, b1).ok_or_else(lost)?;
            let flat = |v: Vec3| [v.dot(frame.x()), v.dot(frame.y())];
            let curves = match &plan.section {
                None => {
                    let p1 = [d1, 0.0];
                    let p2 = [d2 * b2.dot(frame.x()), d2 * b2.dot(frame.y())];
                    [
                        line(0, [0.0, 0.0], p1),
                        across_curve(plan, [0.0, 0.0], p1, p2, flat)?,
                        line(2, p2, [0.0, 0.0]),
                    ]
                }
                Some(section) => {
                    let [f1, f2] = section.feet;
                    let (p1, p2) = (flat(f1), flat(f2));
                    let [t1, t2] = section.traces;
                    [
                        along_trace(0, &t1, [0.0, 0.0], p1, flat),
                        across_curve(plan, [0.0, 0.0], p1, p2, flat)?,
                        along_trace(2, &t2, p2, [0.0, 0.0], flat),
                    ]
                }
            };
            let profiles = detect_profiles(&curves, tolerance)?;
            let total = length + plan.longer[0] + plan.longer[1];
            let mut made = slope_of(extrude(&profiles, &frame, 0.0, total, tolerance)?)?;
            for (point, far) in plan.trims.into_iter().flatten() {
                let size = 4.0 * (total + plan.largest());
                made = trim(made, point, far, size, tolerance)?;
            }
            Ok(made)
        }
        Path::Circle {
            centre,
            axis,
            radial,
            radius,
            arc,
        } => {
            // The section in the half-plane through the axis: x away
            // from the axis, y along it. For an arc it is drawn where the
            // tool starts and turned from there.
            let (from, turn) = match arc {
                None => (radial, TAU),
                Some((start, sweep)) => {
                    let back = -plan.longer[0];
                    (
                        start * back.cos() + axis.cross(start) * back.sin(),
                        sweep + plan.longer[0] + plan.longer[1],
                    )
                }
            };
            let frame = Frame::new(centre, from.cross(axis), from).ok_or_else(lost)?;
            let flat = |v: Vec3| [v.dot(radial), v.dot(axis)];
            let rim = [radius, 0.0];
            let [x1, y1] = flat(b1 * d1);
            let [x2, y2] = flat(b2 * d2);
            let p1 = [rim[0] + x1, rim[1] + y1];
            let p2 = [rim[0] + x2, rim[1] + y2];
            let curves = [
                line(0, rim, p1),
                across_curve(plan, rim, p1, p2, flat)?,
                line(2, p2, rim),
            ];
            let profiles = detect_profiles(&curves, tolerance)?;
            slope_of(revolve(
                &profiles,
                &frame,
                [0.0, 0.0],
                [0.0, 1.0],
                0.0,
                turn,
                tolerance,
            )?)
        }
    }
}

/// A vertex where one filleted edge goes on smoothly in another: the
/// vertex and the two plans.
struct Joint {
    vertex: VertexId,
    plans: [usize; 2],
}

/// The vertices at which two fillets continue each other: exactly two
/// filleted edges of one kind end there, and one leaves where the other
/// arrives. Their two faces (a cylinder and a torus along the rim of a
/// slot) are tangent along the border they share, which no boolean
/// between two separate tools is asked to find: the tools are joined
/// into one first ([`join`]).
fn joints(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    plans: &[Plan],
) -> Result<Vec<Joint>, BlendError> {
    let planned: BTreeMap<EdgeId, usize> =
        plans.iter().enumerate().map(|(i, p)| (p.edge, i)).collect();
    let mut found = Vec::new();
    for index in 0..body.vertices.len() {
        let vertex = VertexId(index_u32(index));
        let away: Vec<(usize, Vec3)> = leaving(body, uses, at, vertex)?
            .into_iter()
            .filter_map(|l| planned.get(&l.edge).map(|i| (*i, l.direction)))
            .collect();
        let [(i, one), (j, two)] = away.as_slice() else {
            continue;
        };
        let (Some(a), Some(b)) = (plans.get(*i), plans.get(*j)) else {
            continue;
        };
        // Two chamfers whose sections are the same triangle at the
        // vertex: one distance on both sides of both edges.
        let same_chamfer = || {
            let d = a.distances[0];
            [a.distances, b.distances]
                .iter()
                .flatten()
                .all(|x| (x - d).abs() <= 1e-12 * d.abs())
        };
        let kinds = match (a.round, b.round) {
            (Some(_), Some(_)) => true,
            (None, None) => same_chamfer(),
            _ => false,
        };
        if i != j && kinds && a.convex == b.convex && one.dot(*two) < -1.0 + 1e-9 {
            found.push(Joint {
                vertex,
                plans: [*i, *j],
            });
        }
    }
    Ok(found)
}

/// Several tools as one body: the tools of edges that continue each other
/// smoothly, with the two ends that lie against each other at every joint
/// taken out and their borders made one. `blends[i]` is the plan whose
/// blend face `FaceId(i)` is.
struct Joined {
    body: Body,
    blends: Vec<Option<usize>>,
}

/// The loop of an end of a tool: its edges, and their vertices.
fn end_of(body: &Body, face: FaceId) -> Option<(Vec<EdgeId>, Vec<VertexId>)> {
    let [only] = body.face(face)?.loops.as_slice() else {
        return None;
    };
    let mut edges = Vec::new();
    let mut vertices = Vec::new();
    for coedge in &body.loop_(*only)?.coedges {
        let data = body.edge(coedge.edge)?;
        edges.push(coedge.edge);
        for vertex in [data.start, data.end] {
            if !vertices.contains(&vertex) {
                vertices.push(vertex);
            }
        }
    }
    Some((edges, vertices))
}

fn join(
    body: &Body,
    members: &[(usize, Tool)],
    joints: &[&Joint],
    tolerance: &Tolerance,
) -> Result<Joined, BlendError> {
    let near = 4.0 * tolerance.linear;
    // All the tools side by side in one body.
    let mut all = Body {
        kind: BodyKind::Solid,
        vertices: Vec::new(),
        edges: Vec::new(),
        loops: Vec::new(),
        faces: Vec::new(),
        shells: Vec::new(),
    };
    let mut blends: Vec<Option<usize>> = Vec::new();
    // Per member, its ends as faces of `all`.
    let mut ends: BTreeMap<usize, Vec<FaceId>> = BTreeMap::new();
    for (plan, tool) in members {
        let (v0, e0, l0, f0) = (
            index_u32(all.vertices.len()),
            index_u32(all.edges.len()),
            index_u32(all.loops.len()),
            index_u32(all.faces.len()),
        );
        all.vertices.extend(tool.body.vertices.iter().cloned());
        for edge in &tool.body.edges {
            let mut edge = edge.clone();
            edge.start = VertexId(edge.start.0.saturating_add(v0));
            edge.end = VertexId(edge.end.0.saturating_add(v0));
            all.edges.push(edge);
        }
        for ring in &tool.body.loops {
            let mut ring = ring.clone();
            for coedge in &mut ring.coedges {
                coedge.edge = EdgeId(coedge.edge.0.saturating_add(e0));
            }
            all.loops.push(ring);
        }
        for (index, face) in tool.body.faces.iter().enumerate() {
            let mut face = face.clone();
            for id in &mut face.loops {
                id.0 = id.0.saturating_add(l0);
            }
            all.faces.push(face);
            blends.push((FaceId(index_u32(index)) == tool.slope).then_some(*plan));
        }
        ends.insert(
            *plan,
            tool.caps
                .iter()
                .map(|f| FaceId(f.0.saturating_add(f0)))
                .collect(),
        );
    }
    let mut dead_faces: BTreeSet<FaceId> = BTreeSet::new();
    let mut dead_edges: BTreeMap<EdgeId, (EdgeId, bool)> = BTreeMap::new();
    let mut dead_vertices: BTreeMap<VertexId, VertexId> = BTreeMap::new();
    for joint in joints {
        let place = body.vertex(joint.vertex).ok_or_else(lost)?.point;
        // The end of each of the two tools that has the vertex in it.
        let end_at = |plan: usize| -> Option<(FaceId, Vec<EdgeId>, Vec<VertexId>)> {
            ends.get(&plan)?.iter().find_map(|face| {
                if dead_faces.contains(face) {
                    return None;
                }
                let (edges, vertices) = end_of(&all, *face)?;
                vertices
                    .iter()
                    .any(|v| {
                        all.vertex(*v)
                            .is_some_and(|v| v.point.distance(place) <= near)
                    })
                    .then_some((*face, edges, vertices))
            })
        };
        let [first, second] = joint.plans;
        let (Some((face_a, edges_a, vertices_a)), Some((face_b, edges_b, vertices_b))) =
            (end_at(first), end_at(second))
        else {
            return Err(lost());
        };
        // The vertices and edges of the second end, as those of the first.
        let mut same_vertex: BTreeMap<VertexId, VertexId> = BTreeMap::new();
        for vb in &vertices_b {
            let point = all.vertex(*vb).ok_or_else(lost)?.point;
            let twin = vertices_a.iter().copied().find(|va| {
                all.vertex(*va)
                    .is_some_and(|v| v.point.distance(point) <= near)
            });
            same_vertex.insert(*vb, twin.ok_or_else(lost)?);
        }
        let middle = |edge: EdgeId| {
            all.edge(edge)
                .map(|e| e.curve.eval(0.5 * (e.range.0 + e.range.1)))
        };
        let mut same_edge: BTreeMap<EdgeId, (EdgeId, bool)> = BTreeMap::new();
        for eb in &edges_b {
            let data = all.edge(*eb).ok_or_else(lost)?;
            let (Some(start), Some(end)) =
                (same_vertex.get(&data.start), same_vertex.get(&data.end))
            else {
                return Err(lost());
            };
            let at_b = middle(*eb).ok_or_else(lost)?;
            let twin = edges_a.iter().copied().find_map(|ea| {
                let other = all.edge(ea)?;
                let along = other.start == *start && other.end == *end;
                let against = other.start == *end && other.end == *start;
                ((along || against) && middle(ea)?.distance(at_b) <= near).then_some((ea, against))
            });
            same_edge.insert(*eb, twin.ok_or_else(lost)?);
        }
        if same_edge.len() != edges_a.len() || same_vertex.len() != vertices_a.len() {
            return Err(lost());
        }
        for ring in &mut all.loops {
            for coedge in &mut ring.coedges {
                if let Some((twin, against)) = same_edge.get(&coedge.edge) {
                    coedge.edge = *twin;
                    coedge.reversed ^= *against;
                }
            }
        }
        for edge in &mut all.edges {
            if let Some(twin) = same_vertex.get(&edge.start) {
                edge.start = *twin;
            }
            if let Some(twin) = same_vertex.get(&edge.end) {
                edge.end = *twin;
            }
        }
        dead_faces.extend([face_a, face_b]);
        dead_edges.extend(same_edge);
        dead_vertices.extend(same_vertex);
    }
    // What is left, counted anew.
    let mut vertex_now: Vec<Option<VertexId>> = Vec::with_capacity(all.vertices.len());
    let mut vertices = Vec::new();
    for (index, vertex) in all.vertices.iter().enumerate() {
        if dead_vertices.contains_key(&VertexId(index_u32(index))) {
            vertex_now.push(None);
        } else {
            vertex_now.push(Some(VertexId(index_u32(vertices.len()))));
            vertices.push(vertex.clone());
        }
    }
    let renamed_vertex = |id: VertexId| {
        vertex_now
            .get(id.0 as usize)
            .copied()
            .flatten()
            .ok_or_else(lost)
    };
    let mut edge_now: Vec<Option<EdgeId>> = Vec::with_capacity(all.edges.len());
    let mut edges = Vec::new();
    for (index, edge) in all.edges.iter().enumerate() {
        if dead_edges.contains_key(&EdgeId(index_u32(index))) {
            edge_now.push(None);
        } else {
            edge_now.push(Some(EdgeId(index_u32(edges.len()))));
            let mut edge = edge.clone();
            edge.start = renamed_vertex(edge.start)?;
            edge.end = renamed_vertex(edge.end)?;
            edges.push(edge);
        }
    }
    let mut loops = Vec::new();
    let mut faces = Vec::new();
    let mut kept_blends = Vec::new();
    for (index, face) in all.faces.iter().enumerate() {
        if dead_faces.contains(&FaceId(index_u32(index))) {
            continue;
        }
        let mut face = face.clone();
        for id in &mut face.loops {
            let mut ring = all.loop_(*id).ok_or_else(lost)?.clone();
            for coedge in &mut ring.coedges {
                coedge.edge = edge_now
                    .get(coedge.edge.0 as usize)
                    .copied()
                    .flatten()
                    .ok_or_else(lost)?;
            }
            *id = craft3d_topo::LoopId(index_u32(loops.len()));
            loops.push(ring);
        }
        faces.push(face);
        kept_blends.push(blends.get(index).copied().flatten());
    }
    let shell = craft3d_topo::Shell {
        faces: (0..faces.len()).map(|i| FaceId(index_u32(i))).collect(),
    };
    let joined = Body {
        kind: BodyKind::Solid,
        vertices,
        edges,
        loops,
        faces,
        shells: vec![shell],
    };
    let mut defects = craft3d_topo::check(&joined, tolerance);
    if !defects.is_empty() {
        defects.truncate(crate::MAX_REPORTED);
        return Err(BlendError::Kernel(KernelError::InvalidBody {
            operation: "joining the tools of a chain of blends",
            defects,
        }));
    }
    Ok(Joined {
        body: joined,
        blends: kept_blends,
    })
}

/// What a face of the body under construction is made of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Made {
    kept: BTreeSet<FaceId>,
    blends: BTreeSet<usize>,
    corners: BTreeSet<VertexId>,
    /// The edges (by their index in the request) of whose tools it is a
    /// face that is not a chamfer face.
    scrap: BTreeSet<usize>,
}

/// The body so far and where its entities came from.
struct State {
    body: Body,
    faces: Vec<Made>,
    edges: Vec<Option<EdgeId>>,
    vertices: Vec<Option<VertexId>>,
}

impl State {
    /// Takes the result of a boolean between the body so far (A) and
    /// the tool (B) of the edge with index `plan`, whose faces in `marks`
    /// are to be known as what is given there.
    fn take(&mut self, result: BooleanResult, marks: &BTreeMap<FaceId, Made>, plan: usize) {
        let origin = |source: &FaceSource| match source.operand {
            Operand::A => self
                .faces
                .get(source.face.0 as usize)
                .cloned()
                .unwrap_or_default(),
            Operand::B => marks.get(&source.face).cloned().unwrap_or_else(|| Made {
                scrap: BTreeSet::from([plan]),
                ..Made::default()
            }),
        };
        let mut faces = Vec::with_capacity(result.faces.len());
        for (index, source) in result.faces.iter().enumerate() {
            let mut made = origin(source);
            for other in result.merged.get(index).into_iter().flatten() {
                let more = origin(other);
                made.kept.extend(more.kept);
                made.blends.extend(more.blends);
                made.corners.extend(more.corners);
                made.scrap.extend(more.scrap);
            }
            faces.push(made);
        }
        let edges = result
            .edges
            .iter()
            .map(|source| match source {
                EdgeSource::Kept {
                    operand: Operand::A,
                    edge,
                } => self.edges.get(edge.0 as usize).copied().flatten(),
                _ => None,
            })
            .collect();
        let vertices = result
            .vertices
            .iter()
            .map(|source| match source {
                VertexSource::Kept {
                    operand: Operand::A,
                    vertex,
                } => self.vertices.get(vertex.0 as usize).copied().flatten(),
                _ => None,
            })
            .collect();
        self.body = result.body;
        self.faces = faces;
        self.edges = edges;
        self.vertices = vertices;
    }
}

fn combine(
    state: &mut State,
    tool: &Body,
    add: bool,
    marks: &BTreeMap<FaceId, Made>,
    first: bool,
    (plan, edge): (usize, EdgeId),
    tolerance: &Tolerance,
) -> Result<(), BlendError> {
    let op = if add {
        BooleanOp::Union
    } else {
        BooleanOp::Subtract
    };
    let options = BooleanOptions {
        merge: true,
        // What the kernel itself made has passed the check.
        trust_operands: !first,
        refuse_zero_thickness: true,
    };
    let result =
        boolean_with(&state.body, tool, op, tolerance, &options).map_err(|error| match error {
            KernelError::InvalidBody { .. } if first => BlendError::Kernel(error),
            other => BlendError::Boolean {
                edge,
                error: Box::new(other),
                smaller_fails: false,
            },
        })?;
    state.take(result, marks, plan);
    Ok(())
}

/// A vertex where three chamfered edges meet and the corner between them.
struct Corner {
    vertex: VertexId,
    /// The indices of the three plans.
    plans: [usize; 3],
    shape: CornerShape,
    add: bool,
}

enum CornerShape {
    /// Flat, through these three points (chamfers).
    Flat([Vec3; 3]),
    /// A piece of the ball about `centre` that touches the three faces,
    /// between the planes through its centre across the three edges,
    /// which leave the vertex along `leaving` (fillets).
    Ball {
        centre: Vec3,
        radius: f64,
        leaving: [Vec3; 3],
    },
}

fn corners(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    plans: &[Plan],
    tol: f64,
) -> Result<Vec<Corner>, BlendError> {
    let planned: BTreeMap<EdgeId, usize> =
        plans.iter().enumerate().map(|(i, p)| (p.edge, i)).collect();
    let mut found = Vec::new();
    for index in 0..body.vertices.len() {
        let vertex = VertexId(index_u32(index));
        let Some(list) = at.get(index) else { continue };
        let Ok([a, b, c]) = <[EdgeId; 3]>::try_from(list.as_slice()) else {
            continue;
        };
        let (Some(&ia), Some(&ib), Some(&ic)) = (planned.get(&a), planned.get(&b), planned.get(&c))
        else {
            continue;
        };
        let (Some(pa), Some(pb), Some(pc)) = (plans.get(ia), plans.get(ib), plans.get(ic)) else {
            continue;
        };
        let straight = |p: &Plan| matches!(p.path, Path::Line { .. }) && p.flat();
        if !(straight(pa) && straight(pb) && straight(pc))
            || pa.convex != pb.convex
            || pa.convex != pc.convex
        {
            continue;
        }
        let away = leaving(body, uses, at, vertex)?;
        let direction = |edge: EdgeId| away.iter().find(|l| l.edge == edge).map(|l| l.direction);
        let corner = body.vertex(vertex).ok_or_else(lost)?.point;
        // On the face two of the edges share, the two cut lines cross.
        let crossing = |p: &Plan, q: &Plan| -> Option<Vec3> {
            let face = p.faces.into_iter().find(|f| q.faces.contains(f))?;
            let (wp, wq) = (direction(p.edge)?, direction(q.edge)?);
            let sine = wp.cross(wq).length();
            if sine <= 1e-9 {
                return None;
            }
            Some(corner + wp * (q.distance_on(face)? / sine) + wq * (p.distance_on(face)? / sine))
        };
        let (Some(p0), Some(p1), Some(p2)) = (crossing(pa, pb), crossing(pb, pc), crossing(pc, pa))
        else {
            continue;
        };
        // Each crossing has to be a point of the chamfered body: behind
        // the chamfer of the third edge. Where one chamfer is larger than
        // the other two reach together it takes the corner whole, the
        // other two never meet, and there is no corner face.
        let behind = |point: Vec3, third: &Plan| -> bool {
            let Path::Line { start, along, .. } = third.path else {
                return false;
            };
            let [b1, b2] = third.into;
            let [d1, d2] = third.distances;
            let Some(normal) = along.cross(b1 * d1 - b2 * d2).normalized() else {
                return false;
            };
            let on = start + b1 * d1;
            let edge_side = normal.dot(start - on);
            let side = normal.dot(point - on);
            side * edge_side < 0.0 && side.abs() > 8.0 * tol
        };
        if !(behind(p0, pc) && behind(p1, pa) && behind(p2, pb)) {
            continue;
        }
        found.push(Corner {
            vertex,
            plans: [ia, ib, ic],
            shape: CornerShape::Flat([p0, p1, p2]),
            add: !pa.convex,
        });
    }
    Ok(found)
}

/// A block that lies against the plane through the corner's three points,
/// on the side of the vertex, and is as wide as the corner twice over.
fn corner_tool(
    body: &Body,
    corner: &Corner,
    edge: EdgeId,
    tolerance: &Tolerance,
) -> Result<Tool, BlendError> {
    let refuse = || BlendError::Refused {
        edge: None,
        face: None,
        reason: Reason::NoCorner,
    };
    let at = body.vertex(corner.vertex).ok_or_else(lost)?.point;
    let [p0, p1, p2] = match corner.shape {
        CornerShape::Flat(points) => points,
        CornerShape::Ball {
            centre,
            radius,
            leaving,
        } => return ball_tool(at, centre, radius, leaving, edge, tolerance),
    };
    let mut normal = (p1 - p0).cross(p2 - p0).normalized().ok_or_else(refuse)?;
    let mut depth = normal.dot(at - p0);
    if depth < 0.0 {
        normal = -normal;
        depth = -depth;
    }
    let centre = (p0 + p1 + p2) * (1.0 / 3.0);
    let half = 2.0
        * [p0, p1, p2]
            .iter()
            .map(|p| p.distance(centre))
            .fold(0.0, f64::max);
    if !(depth > 0.0 && half > 0.0) {
        return Err(refuse());
    }
    let frame = Frame::new(centre, normal, p1 - p0).ok_or_else(refuse)?;
    let frame = frame
        .at(frame.point(-half, -half, 0.0))
        .ok_or_else(refuse)?;
    let block = primitives::block(&frame, 2.0 * half, 2.0 * half, 2.0 * depth)?;
    // The face of a block at the bottom of its own axes is its first.
    Ok(Tool {
        body: block,
        slope: FaceId(0),
        caps: Vec::new(),
    })
}

/// The vertices where filleted edges meet in a corner that takes a piece
/// of a ball (see the module documentation), and a refusal for every
/// vertex where three or more of them meet in any other way.
///
/// **Why that piece of that ball.** Near a convex corner of three planes
/// the rounded solid is the union of all balls of the radius that lie
/// behind the three faces: their centres fill the corner moved inwards by
/// the radius, whose apex `c` is the radius from all three faces and
/// whose edges are the axes of the three cylinders. A point of space has
/// its nearest centre at `c` exactly when it lies in the cone at `c`
/// spanned by the three outward normals; that cone's faces are the planes
/// through `c` spanned by two normals each, which are the planes across
/// the three edges (an edge is at right angles to the normals of its two
/// faces). So inside that cone the solid is the ball about `c`, and
/// outside it the nearest centre lies on an axis or deeper, where the
/// tools of the edges have already left the cylinders. A concave corner
/// is the same with material and space changed round.
fn ball_corners(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    plans: &[Plan],
    tol: f64,
) -> Result<Vec<Corner>, BlendError> {
    let planned: BTreeMap<EdgeId, usize> =
        plans.iter().enumerate().map(|(i, p)| (p.edge, i)).collect();
    let mut found = Vec::new();
    for index in 0..body.vertices.len() {
        let vertex = VertexId(index_u32(index));
        let Some(list) = at.get(index) else { continue };
        let here: Vec<usize> = list
            .iter()
            .filter_map(|e| planned.get(e).copied())
            .collect();
        let Some(first) = here.first().and_then(|i| plans.get(*i)) else {
            continue;
        };
        if here.len() < 3 {
            continue;
        }
        let three = <[usize; 3]>::try_from(here.as_slice()).ok();
        let (Some([ia, ib, ic]), 3) = (three, list.len()) else {
            return Err(refused(first.edge, Reason::CornerKind));
        };
        let (Some(pa), Some(pb), Some(pc)) = (plans.get(ia), plans.get(ib), plans.get(ic)) else {
            return Err(lost());
        };
        let straight = |p: &Plan| matches!(p.path, Path::Line { .. }) && p.flat();
        if !(straight(pa) && straight(pb) && straight(pc))
            || pa.convex != pb.convex
            || pa.convex != pc.convex
        {
            return Err(refused(pa.edge, Reason::CornerKind));
        }
        let (Some(ra), Some(rb), Some(rc)) = (pa.round, pb.round, pc.round) else {
            return Err(lost());
        };
        if (ra - rb).abs() > tol || (ra - rc).abs() > tol {
            return Err(refused(pa.edge, Reason::CornerRadii));
        }
        let mut faces: Vec<FaceId> = Vec::new();
        for face in [pa, pb, pc].into_iter().flat_map(|p| p.faces) {
            if !faces.contains(&face) {
                faces.push(face);
            }
        }
        let Ok([fa, fb, fc]) = <[FaceId; 3]>::try_from(faces.as_slice()) else {
            return Err(refused(pa.edge, Reason::CornerKind));
        };
        let corner = body.vertex(vertex).ok_or_else(lost)?.point;
        // The ball's centre is the radius away from the three faces: in
        // the material at a convex corner, out of it at a concave one.
        let (na, nb, nc) = (
            outward(body, fa, corner)?,
            outward(body, fb, corner)?,
            outward(body, fc, corner)?,
        );
        let det = na.dot(nb.cross(nc));
        if det.abs() <= 1e-9 {
            return Err(refused(pa.edge, Reason::CornerKind));
        }
        let side = if pa.convex { -ra } else { ra };
        let offset = (nb.cross(nc) + nc.cross(na) + na.cross(nb)) * (side / det);
        let away = leaving(body, uses, at, vertex)?;
        let direction = |edge: EdgeId| away.iter().find(|l| l.edge == edge).map(|l| l.direction);
        let (Some(wa), Some(wb), Some(wc)) =
            (direction(pa.edge), direction(pb.edge), direction(pc.edge))
        else {
            return Err(lost());
        };
        // The ball has to sit before the vertex along every edge.
        if [wa, wb, wc].iter().any(|w| offset.dot(*w) <= 8.0 * tol) {
            return Err(refused(pa.edge, Reason::CornerKind));
        }
        found.push(Corner {
            vertex,
            plans: [ia, ib, ic],
            shape: CornerShape::Ball {
                centre: corner + offset,
                radius: ra,
                leaving: [wa, wb, wc],
            },
            add: !pa.convex,
        });
    }
    Ok(found)
}

/// The tool of a ball corner: the cell that the three planes through the
/// ball's centre across the three edges cut out on the vertex's side (as
/// far as the vertex and a little further), less the ball. Its one curved
/// face is the corner face.
fn ball_tool(
    vertex: Vec3,
    centre: Vec3,
    radius: f64,
    leaving: [Vec3; 3],
    edge: EdgeId,
    tolerance: &Tolerance,
) -> Result<Tool, BlendError> {
    let refuse = || BlendError::Refused {
        edge: Some(edge),
        face: None,
        reason: Reason::NoCorner,
    };
    let refused_by = |error: KernelError| BlendError::Boolean {
        edge,
        error: Box::new(error),
        smaller_fails: false,
    };
    let reach = 1.25 * vertex.distance(centre);
    if !(reach.is_finite() && reach > 0.0) {
        return Err(refuse());
    }
    let [wa, wb, wc] = leaving;
    // Three blocks, each with a face in one of the planes. They differ
    // in size, so that no two of their other faces fall together.
    let mut cell: Option<Body> = None;
    for (w, hint, grow) in [(wa, wb, 1.0), (wb, wc, 1.125), (wc, wa, 1.25)] {
        let frame = Frame::new(centre, -w, hint).ok_or_else(refuse)?;
        let half = 2.0 * grow * reach;
        let frame = frame
            .at(frame.point(-half, -half, 0.0))
            .ok_or_else(refuse)?;
        let block = primitives::block(&frame, 2.0 * half, 2.0 * half, grow * reach)?;
        cell = Some(match cell {
            None => block,
            Some(so_far) => {
                boolean_with(
                    &so_far,
                    &block,
                    BooleanOp::Intersect,
                    tolerance,
                    &BooleanOptions::default(),
                )
                .map_err(refused_by)?
                .body
            }
        });
    }
    let cell = cell.ok_or_else(refuse)?;
    let ball = primitives::sphere(&Frame::WORLD.at(centre).ok_or_else(refuse)?, radius)?;
    let cut = boolean_with(
        &cell,
        &ball,
        BooleanOp::Subtract,
        tolerance,
        &BooleanOptions::default(),
    )
    .map_err(refused_by)?;
    let round: Vec<usize> = cut
        .faces
        .iter()
        .enumerate()
        .filter(|(_, source)| source.operand == Operand::B)
        .map(|(index, _)| index)
        .collect();
    let [slope] = round.as_slice() else {
        return Err(refuse());
    };
    Ok(Tool {
        body: cut.body,
        slope: FaceId(index_u32(*slope)),
        caps: Vec::new(),
    })
}

/// Places along an edge at which a seam is measured, as fractions of its
/// range: away from the ends, where other faces meet it.
const SEAM_SAMPLES: [f64; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];

/// A fillet face has to run into both neighbours of its edge, and into
/// the ball of a corner, without a crease: at points along every edge it
/// shares with one of them the two normals agree within the angular
/// tolerance (and never closer than rounding allows to tell).
fn smooth_seams(
    state: &State,
    now: &[Vec<Use>],
    plans: &[Plan],
    tolerance: &Tolerance,
) -> Result<(), BlendError> {
    let limit = tolerance.angular.max(SEAM_ROUNDING);
    for (index, users) in now.iter().enumerate() {
        let [a, b] = users.as_slice() else { continue };
        for (fillet, other) in [(a.face, b.face), (b.face, a.face)] {
            let (Some(made), Some(beside)) = (
                state.faces.get(fillet.0 as usize),
                state.faces.get(other.0 as usize),
            ) else {
                continue;
            };
            let own: Vec<&Plan> = made
                .blends
                .iter()
                .filter_map(|i| plans.get(*i))
                .filter(|p| p.round.is_some())
                .collect();
            let Some(plan) = own.first() else { continue };
            let neighbour = beside
                .kept
                .iter()
                .copied()
                .find(|f| own.iter().any(|p| p.faces.contains(f)));
            if neighbour.is_none() && beside.corners.is_empty() {
                continue;
            }
            let Some(data) = state.body.edge(EdgeId(index_u32(index))) else {
                continue;
            };
            for s in SEAM_SAMPLES {
                let point = data
                    .curve
                    .eval(data.range.0 + s * (data.range.1 - data.range.0));
                let one = outward(&state.body, fillet, point)?;
                let two = outward(&state.body, other, point)?;
                let sine = one.cross(two).length();
                if !(sine <= limit && one.dot(two) > 0.0) {
                    return Err(BlendError::Refused {
                        edge: Some(plan.edge),
                        face: neighbour,
                        reason: Reason::NotTangent,
                    });
                }
            }
        }
    }
    Ok(())
}

/// What rounding leaves of the angle between two normals that are the
/// same: a seam is not held to less.
const SEAM_ROUNDING: f64 = 1e-9;

/// Holds the result against what a blend is (see the module
/// documentation) and turns it into what is reported.
fn finish(
    at: &[Vec<EdgeId>],
    uses: &[Vec<Use>],
    plans: &[Plan],
    mut state: State,
    tolerance: &Tolerance,
) -> Result<Blended, BlendError> {
    let mut faces = Vec::with_capacity(state.faces.len());
    let mut of_plan: Vec<Vec<usize>> = vec![Vec::new(); plans.len()];
    for (index, made) in state.faces.iter().enumerate() {
        let first = made
            .blends
            .iter()
            .chain(&made.scrap)
            .next()
            .and_then(|i| plans.get(*i))
            .map(|p| p.edge);
        let lone = |reason| BlendError::Refused {
            edge: first,
            face: None,
            reason,
        };
        let kinds = usize::from(!made.kept.is_empty())
            + usize::from(!made.blends.is_empty())
            + usize::from(!made.corners.is_empty());
        if kinds == 0 {
            return Err(lone(Reason::EndsBadly));
        }
        if kinds > 1 {
            return Err(lone(Reason::InAFace));
        }
        if !made.kept.is_empty() {
            faces.push(Face::Kept(made.kept.iter().copied().collect()));
        } else if let Some(vertex) = made.corners.iter().next() {
            faces.push(Face::Corner(*vertex));
        } else {
            for &plan in &made.blends {
                if let Some(list) = of_plan.get_mut(plan) {
                    list.push(index);
                }
            }
            faces.push(Face::Blend(made.blends.iter().copied().collect()));
        }
    }
    let now = edge_uses(&state.body);
    // The faces across the edges of each face.
    let mut beside: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); state.body.faces.len()];
    for users in &now {
        for a in users {
            for b in users {
                if a.face != b.face
                    && let Some(set) = beside.get_mut(a.face.0 as usize)
                {
                    set.insert(b.face.0 as usize);
                }
            }
        }
    }
    let faces_at = |vertex: VertexId| -> BTreeSet<FaceId> {
        at.get(vertex.0 as usize)
            .into_iter()
            .flatten()
            .filter_map(|e| uses.get(e.0 as usize))
            .flatten()
            .map(|u| u.face)
            .collect()
    };
    for (index, plan) in plans.iter().enumerate() {
        let Some([own]) = of_plan.get(index).map(Vec::as_slice) else {
            return Err(refused(plan.edge, Reason::InPieces));
        };
        let ends: Vec<VertexId> = plan.ends.into_iter().flatten().collect();
        let mut allowed: BTreeSet<FaceId> = plan.faces.into_iter().collect();
        for vertex in &ends {
            allowed.extend(faces_at(*vertex));
        }
        let mut met = [false, false];
        for &other in beside.get(*own).into_iter().flatten() {
            let Some(made) = state.faces.get(other) else {
                continue;
            };
            let fine = if !made.kept.is_empty() {
                for (flag, face) in met.iter_mut().zip(plan.faces) {
                    *flag |= made.kept.contains(&face);
                }
                made.kept.iter().any(|f| allowed.contains(f))
            } else if !made.corners.is_empty() {
                made.corners.iter().any(|v| ends.contains(v))
            } else {
                // Another chamfer: of an edge that ends where this one
                // does.
                made.blends.iter().any(|i| {
                    plans.get(*i).is_some_and(|p| {
                        p.ends
                            .into_iter()
                            .flatten()
                            .any(|vertex| ends.contains(&vertex))
                    })
                })
            };
            if !fine {
                return Err(BlendError::Refused {
                    edge: Some(plan.edge),
                    face: made.kept.iter().next().copied(),
                    reason: Reason::Reaches,
                });
            }
        }
        if met != [true, true] {
            let missing = if met[0] { plan.faces[1] } else { plan.faces[0] };
            return Err(BlendError::Refused {
                edge: Some(plan.edge),
                face: Some(missing),
                reason: Reason::LostNeighbour,
            });
        }
    }
    smooth_seams(&state, &now, plans, tolerance)?;
    for (index, plan) in plans.iter().enumerate() {
        reframe(&mut state.body, &of_plan, index, plan);
    }
    Ok(Blended {
        body: state.body,
        faces,
        edges: state.edges,
        vertices: state.vertices,
    })
}

/// Gives the flat chamfer face of a straight edge a frame that says
/// something: z out of the body, x along the edge in the direction of the
/// cross product of the directions into the first and the second face,
/// and the origin where the cut line on the first face starts in that
/// direction. (What the boolean left is the frame of the tool's face,
/// whose origin lies wherever the tool began.) The plane is the same
/// plane.
fn reframe(body: &mut Body, of_plan: &[Vec<usize>], index: usize, plan: &Plan) {
    let Path::Line {
        start,
        along,
        length,
    } = plan.path
    else {
        return;
    };
    let Some([own]) = of_plan.get(index).map(Vec::as_slice) else {
        return;
    };
    // A face merged from several chamfers keeps the frame of the first.
    if of_plan
        .iter()
        .take(index)
        .any(|earlier| earlier.contains(own))
    {
        return;
    }
    let Some(face) = body.faces.get_mut(*own) else {
        return;
    };
    let Surface::Plane(plane) = &face.surface else {
        return;
    };
    let normal = plane.frame().z();
    let outward = if face.reversed { -normal } else { normal };
    let [b1, b2] = plan.into;
    let forward = b1.cross(b2).dot(along) >= 0.0;
    let (x, from) = if forward {
        (along, start)
    } else {
        (-along, start + along * length)
    };
    let [f1, _] = plan.feet();
    if let Some(frame) = Frame::new(from + f1, outward, x) {
        face.surface = Surface::Plane(craft3d_geom::Plane::new(frame));
        face.reversed = false;
    }
}

/// Blends `edges` of the solid `body`: see the module documentation for
/// the method, for what the corner is where blends meet, and for what is
/// refused. The result has passed `craft3d_topo::check` with `tolerance`.
pub(crate) fn blend(
    body: &Body,
    edges: &[Request],
    tolerance: &Tolerance,
) -> Result<Blended, BlendError> {
    blend_inner(body, edges, tolerance, false)
}

/// Blends the edges no closed form covers (`free`, with every edge that
/// shares a vertex with one of them, so a chain that runs on from a
/// freeform edge into a line or an arc is one chain) in place first, and
/// then the others (`closed`, whose edges and faces keep their numbers
/// through that) by the tools; the sources of the second result are
/// carried back to `body`.
fn mixed(
    body: &Body,
    edges: &[Request],
    mut free: Vec<usize>,
    tolerance: &Tolerance,
) -> Result<Blended, BlendError> {
    let mut closed: Vec<usize> = (0..edges.len()).filter(|i| !free.contains(i)).collect();
    // Edges that touch a freeform one go with it.
    for _ in 0..=edges.len() {
        let touched: BTreeSet<VertexId> = free
            .iter()
            .filter_map(|i| body.edge(edges.get(*i)?.edge))
            .flat_map(|e| [e.start, e.end])
            .collect();
        let (joins, stays): (Vec<usize>, Vec<usize>) = closed.iter().partition(|i| {
            edges
                .get(**i)
                .and_then(|r| body.edge(r.edge))
                .is_some_and(|e| touched.contains(&e.start) || touched.contains(&e.end))
        });
        if joins.is_empty() {
            break;
        }
        free.extend(joins);
        free.sort_unstable();
        closed = stays;
    }
    let pick = |list: &[usize]| -> Vec<Request> {
        list.iter().filter_map(|i| edges.get(*i).copied()).collect()
    };
    let rolled = rolling::blend(body, &pick(&free), tolerance)?;
    if closed.is_empty() {
        // `free` is every request, in order.
        return Ok(rolled);
    }
    let second = blend_inner(&rolled.body, &pick(&closed), tolerance, true)?;
    let from_free = |i: &usize| free.get(*i).copied().unwrap_or(usize::MAX);
    let from_closed = |i: &usize| closed.get(*i).copied().unwrap_or(usize::MAX);
    let faces = second
        .faces
        .into_iter()
        .map(|face| match face {
            Face::Kept(list) => {
                let mut kept = BTreeSet::new();
                let mut blends = BTreeSet::new();
                for f in list {
                    match rolled.faces.get(f.0 as usize) {
                        Some(Face::Kept(origin)) => kept.extend(origin.iter().copied()),
                        Some(Face::Blend(of)) => blends.extend(of.iter().map(from_free)),
                        Some(Face::Corner(_)) | None => {}
                    }
                }
                if blends.is_empty() {
                    Face::Kept(kept.into_iter().collect())
                } else {
                    Face::Blend(blends.into_iter().collect())
                }
            }
            Face::Blend(of) => {
                let mut list: Vec<usize> = of.iter().map(from_closed).collect();
                list.sort_unstable();
                Face::Blend(list)
            }
            Face::Corner(vertex) => Face::Corner(
                rolled
                    .vertices
                    .get(vertex.0 as usize)
                    .copied()
                    .flatten()
                    .unwrap_or(vertex),
            ),
        })
        .collect();
    let edges_of = second
        .edges
        .into_iter()
        .map(|e| e.and_then(|e| rolled.edges.get(e.0 as usize).copied().flatten()))
        .collect();
    let vertices = second
        .vertices
        .into_iter()
        .map(|v| v.and_then(|v| rolled.vertices.get(v.0 as usize).copied().flatten()))
        .collect();
    Ok(Blended {
        body: second.body,
        faces,
        edges: edges_of,
        vertices,
    })
}

fn blend_inner(
    body: &Body,
    edges: &[Request],
    tolerance: &Tolerance,
    nested: bool,
) -> Result<Blended, BlendError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        }
        .into());
    }
    if edges.len() > MAX_EDGES {
        return Err(BlendError::Refused {
            edge: None,
            face: None,
            reason: Reason::TooMany,
        });
    }
    if body.kind != BodyKind::Solid {
        return Err(BlendError::Refused {
            edge: edges.first().map(|e| e.edge),
            face: None,
            reason: Reason::NotSolid,
        });
    }
    let uses = edge_uses(body);
    let at = vertex_edges(body);
    let mut seen = BTreeSet::new();
    let mut plans = Vec::with_capacity(edges.len());
    // Edges none of the closed forms covers go to `rolling` (see
    // `mixed`).
    let mut free = Vec::new();
    for (index, request) in edges.iter().enumerate() {
        if !seen.insert(request.edge) {
            return Err(refused(request.edge, Reason::Twice));
        }
        match plan(body, &uses, request, tol) {
            Ok(made) => plans.push(made),
            Err(BlendError::Refused { reason, .. }) if rolling::takes(reason) => {
                free.push(index);
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(first) = free.first() {
        if nested {
            let edge = edges.get(*first).map_or(EdgeId(u32::MAX), |r| r.edge);
            return Err(refused(edge, Reason::FreeMixed));
        }
        return mixed(body, edges, free, tolerance);
    }
    match build(body, &uses, &at, plans.clone(), tolerance) {
        Ok(made) => Ok(made),
        Err(failure) => Err(fit::explain(
            body, &uses, &at, edges, &plans, failure, tolerance,
        )),
    }
}

/// A build that failed, with the edge it failed at when that is known.
struct Failure {
    edge: Option<EdgeId>,
    error: BlendError,
}

impl From<BlendError> for Failure {
    fn from(error: BlendError) -> Self {
        let edge = match &error {
            BlendError::Refused { edge, .. } => *edge,
            BlendError::Boolean { edge, .. } | BlendError::TooLarge { edge, .. } => Some(*edge),
            BlendError::Kernel(_) => None,
        };
        Failure { edge, error }
    }
}

/// A failure at `edge`, unless the error names one itself.
fn at_edge(edge: EdgeId) -> impl Fn(BlendError) -> Failure {
    move |error| {
        let mut failure = Failure::from(error);
        failure.edge.get_or_insert(edge);
        failure
    }
}

/// Builds the blends the plans describe, exactly, and holds the result
/// against what a blend is ([`finish`]): everything after the plans.
fn build(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    mut plans: Vec<Plan>,
    tolerance: &Tolerance,
) -> Result<Blended, Failure> {
    let tol = tolerance.linear;
    lengthen(body, uses, at, &mut plans)?;
    let found = if plans.iter().any(|p| p.round.is_some()) {
        ball_corners(body, uses, at, &plans, tol)?
    } else {
        corners(body, uses, at, &plans, tol)?
    };

    let mut state = State {
        body: body.clone(),
        faces: (0..body.faces.len())
            .map(|i| Made {
                kept: BTreeSet::from([FaceId(index_u32(i))]),
                ..Made::default()
            })
            .collect(),
        edges: (0..body.edges.len())
            .map(|i| Some(EdgeId(index_u32(i))))
            .collect(),
        vertices: (0..body.vertices.len())
            .map(|i| Some(VertexId(index_u32(i))))
            .collect(),
    };
    // Blends that continue each other are one tool (see `joints`).
    let joined = joints(body, uses, at, &plans)?;
    // Per plan, the least plan it is joined to, through any number of
    // joints.
    let mut group: Vec<usize> = (0..plans.len()).collect();
    for _ in 0..plans.len() {
        let mut changed = false;
        for joint in &joined {
            let [a, b] = joint.plans;
            let least = group
                .get(a)
                .copied()
                .unwrap_or(a)
                .min(group.get(b).copied().unwrap_or(b));
            for index in [a, b] {
                if let Some(slot) = group.get_mut(index)
                    && *slot != least
                {
                    *slot = least;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let blend_of = |index: usize| Made {
        blends: BTreeSet::from([index]),
        ..Made::default()
    };
    let mut first = true;
    for (index, plan) in plans.iter().enumerate() {
        if group.get(index) != Some(&index) {
            // Made with the least of its group.
            continue;
        }
        let members: Vec<usize> = (index..plans.len())
            .filter(|i| group.get(*i) == Some(&index))
            .collect();
        if let [_] = members.as_slice() {
            let made = tool(plan, tolerance).map_err(at_edge(plan.edge))?;
            combine(
                &mut state,
                &made.body,
                !plan.convex,
                &BTreeMap::from([(made.slope, blend_of(index))]),
                first,
                (index, plan.edge),
                tolerance,
            )?;
        } else {
            let mut tools = Vec::with_capacity(members.len());
            for member in &members {
                let plan = plans.get(*member).ok_or_else(lost)?;
                tools.push((*member, tool(plan, tolerance).map_err(at_edge(plan.edge))?));
            }
            let between: Vec<&Joint> = joined
                .iter()
                .filter(|j| j.plans.iter().any(|p| members.contains(p)))
                .collect();
            let made = join(body, &tools, &between, tolerance).map_err(at_edge(plan.edge))?;
            let marks: BTreeMap<FaceId, Made> = made
                .blends
                .iter()
                .enumerate()
                .filter_map(|(face, plan)| Some((FaceId(index_u32(face)), blend_of((*plan)?))))
                .collect();
            combine(
                &mut state,
                &made.body,
                !plan.convex,
                &marks,
                first,
                (index, plan.edge),
                tolerance,
            )?;
        }
        first = false;
    }
    for corner in &found {
        let index = corner.plans.first().copied().unwrap_or(usize::MAX);
        let edge = plans.get(index).map_or(EdgeId(u32::MAX), |p| p.edge);
        let made = corner_tool(body, corner, edge, tolerance).map_err(at_edge(edge))?;
        let marked = Made {
            corners: BTreeSet::from([corner.vertex]),
            ..Made::default()
        };
        combine(
            &mut state,
            &made.body,
            corner.add,
            &BTreeMap::from([(made.slope, marked)]),
            first,
            (index, edge),
            tolerance,
        )?;
        first = false;
    }
    Ok(finish(at, uses, &plans, state, tolerance)?)
}

/// The edges that continue `edge` smoothly, with it: at each end the one
/// other edge that leaves the vertex straight on (the top rim of a slot:
/// lines and arcs), followed both ways until none does or the chain
/// closes. Every edge comes with the face that takes the place of `first`
/// along it: the face it has in common with the edge before it when that
/// is that edge's first face, and otherwise its other face. The edges are
/// in ascending order.
///
/// `None` when `edge` is not an edge between two faces of the solid of
/// which `first` is one.
pub fn tangent_chain(
    body: &Body,
    edge: EdgeId,
    first: FaceId,
) -> Result<Option<Vec<(EdgeId, FaceId)>>, KernelError> {
    chain_of(body, edge, first).map_err(crate::chamfer::said)
}

fn chain_of(
    body: &Body,
    edge: EdgeId,
    first: FaceId,
) -> Result<Option<Vec<(EdgeId, FaceId)>>, BlendError> {
    let uses = edge_uses(body);
    let at = vertex_edges(body);
    let faces = |edge: EdgeId| -> Option<[FaceId; 2]> {
        let list = uses.get(edge.0 as usize)?;
        let [a, b] = <[Use; 2]>::try_from(list.as_slice()).ok()?;
        (a.face != b.face).then_some([a.face, b.face])
    };
    let Some(own) = faces(edge) else {
        return Ok(None);
    };
    if !own.contains(&first) {
        return Ok(None);
    }
    let mut chain: BTreeMap<EdgeId, FaceId> = BTreeMap::from([(edge, first)]);
    let mut open = vec![(edge, first)];
    while let Some((current, current_first)) = open.pop() {
        if chain.len() > MAX_EDGES {
            break;
        }
        let Some(data) = body.edge(current) else {
            continue;
        };
        if data.start == data.end {
            continue;
        }
        let Some(current_faces) = faces(current) else {
            continue;
        };
        let ends = [
            (data.start, -tangent(&data.curve, data.range.0)?),
            (data.end, tangent(&data.curve, data.range.1)?),
        ];
        for (vertex, beyond) in ends {
            let next: Vec<Leaving> = leaving(body, &uses, &at, vertex)?
                .into_iter()
                .filter(|l| l.edge != current && l.direction.dot(beyond) > 1.0 - 1e-9)
                .collect();
            let [next] = next.as_slice() else {
                continue;
            };
            if chain.contains_key(&next.edge) || next.turn == Turn::Smooth {
                continue;
            }
            let Some(next_faces) = faces(next.edge) else {
                continue;
            };
            // The face that goes on is the one both edges have; the
            // other side goes on in whatever is tangent to it.
            let shared: Vec<FaceId> = next_faces
                .into_iter()
                .filter(|f| current_faces.contains(f))
                .collect();
            let next_first = match shared.as_slice() {
                [face] if *face == current_first => *face,
                [face] => {
                    let [a, b] = next_faces;
                    if a == *face { b } else { a }
                }
                _ => continue,
            };
            chain.insert(next.edge, next_first);
            open.push((next.edge, next_first));
        }
    }
    Ok(Some(chain.into_iter().collect()))
}
