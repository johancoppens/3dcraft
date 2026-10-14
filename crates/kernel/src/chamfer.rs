//! Chamfers: an edge of a solid replaced by a face. The two faces next to
//! the edge are cut back by a distance each, measured along the face and
//! at right angles to the edge, and the new face joins the two cut lines.
//! A convex edge loses material, a concave edge gains it.
//!
//! **Method: a tool per edge and the boolean.** For every edge a tool is
//! built whose cross-section is the triangle between the edge and the two
//! cut lines: a prism along a straight edge between two planes, a ring
//! about the axis of a circular edge between a plane, a cylinder or a cone
//! that share that axis. The tool is taken out of the body (convex) or
//! added to it (concave) by [`crate::boolean`], one edge after the other.
//! Two faces of the tool lie in the two neighbours of the edge, which the
//! boolean treats as coincident faces; the third is the chamfer face.
//! Every tool is made from the body as it was given, so the order of the
//! edges does not change the solid: chamfers that meet are each the whole
//! half-space of their face near the vertex (a mitre).
//!
//! **Where an edge ends** the chamfer face has to stop against whatever is
//! at the vertex:
//!
//! - When every other edge at the vertex is convex like the chamfered one
//!   (or every one concave), the body near the vertex is what its faces
//!   leave of space, the chamfer is one more such face, and the tool is
//!   made longer than the edge: what sticks out meets nothing (convex) or
//!   only material (concave).
//! - Otherwise the tool ends at the vertex, in the plane across the edge
//!   there. That is right when the face the chamfer runs into lies in
//!   that plane (an edge of a step that ends against a wall), and when the
//!   edge continues smoothly in another chamfered edge (a chain: the two
//!   tools end against each other). In every other case the end of the
//!   tool would be left in the result as a face nobody asked for, which
//!   the check at the end finds and refuses.
//!
//! **Corners.** Where three chamfered edges of one kind (all convex or all
//! concave) meet in a vertex with nothing else, the corner is a flat face
//! through the three points in which the cut lines cross on the three
//! faces: a triangle whatever the distances are, since three points are
//! always in a plane. (When one of the three chamfers is larger than the
//! other two reach together, it takes the corner whole: those crossings
//! are not points of the body, and there is no corner face.) Where two
//! chamfered edges meet, their faces meet in the line both planes share.
//!
//! **Never a wrong solid.** A distance for which a cut line would leave
//! its face is refused before anything is built, with the largest distance
//! that fits. After the last boolean the result is held against what a
//! chamfer is: every chamfer face is there, borders both neighbours of its
//! edge, and borders nothing but those, the faces at the ends of its edge
//! and the chamfers and corners there; no other face of a tool is left.
//! Anything else is an error that names the edge.

use crate::KernelError;
use crate::boolean::{
    BooleanOp, BooleanOptions, BooleanResult, EdgeSource, FaceSource, Operand, VertexSource,
    boolean_with,
};
use crate::extrude::{Role, extrude};
use crate::primitives;
use crate::profile::{Curve2, SketchCurve, detect_profiles};
use crate::revolve::revolve;
use craft3d_geom::{Curve3, GeomError, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, EdgeId, FaceId, VertexId};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

/// Most edges one chamfer takes.
pub const MAX_CHAMFER_EDGES: usize = 4096;

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

const NOT_AN_EDGE: &str = "there is no such edge in the body";
const NOT_A_FACE: &str = "the face is not one of the two faces of the edge";
const TWICE: &str = "the edge is given twice";
const NOT_SOLID: &str = "only an edge between two faces of a solid can be chamfered (not a seam, nor the border of a sheet)";
const SMOOTH: &str = "the two faces are tangent along this edge: there is no corner to chamfer";
const ANGLE: &str = "the angle has to be more than zero and less than what is left of half a turn by the angle between the faces";
const KIND: &str = "this kind of edge is not built yet: a chamfer takes a straight edge between two planes and a circular edge between a plane, a cylinder or a cone about its axis";
const KIND_LINE: &str = "a straight edge between faces that are not both planes is not built yet";
const KIND_CIRCLE: &str = "a circular edge between faces that are not a plane, a cylinder or a cone about the circle's axis is not built yet";
const KIND_ELLIPSE: &str =
    "an elliptic edge is not built yet (the chamfer face would be a ruled surface)";
const KIND_SPLINE: &str =
    "a freeform edge is not built yet (the chamfer face would be a ruled surface)";
const ENDS_BADLY: &str = "the chamfer does not end cleanly: at one end of the edge it would stop in a face of its own, because the faces there are neither all convex or all concave like the edge, nor square to it, nor does the edge go on in another chamfered edge; chamfer the edges it continues in as well, or leave this one out";
const REACHES: &str = "the chamfer reaches a face that is neither next to the edge nor at one of its ends: the distance is too large, or two chamfers meet in the middle of a face";
const LOST_NEIGHBOUR: &str = "the chamfer takes away the whole of a face next to the edge, or does not reach it: the distance is too large";
const IN_PIECES: &str = "the chamfer face came out in pieces or not at all: the distance is too large for what is near the edge";
const IN_A_FACE: &str =
    "the chamfer face lies in the surface of a face of the body and became part of it";
const TOO_MANY: &str = "more edges than one chamfer takes";
const NO_CORNER: &str = "the corner where three chamfers meet could not be made flat";
const LOST: &str = "the chamfer lost track of an entity; this is a bug";

fn refused(edge: EdgeId, reason: &'static str) -> KernelError {
    KernelError::Chamfer {
        edge: Some(edge),
        face: None,
        reason,
    }
}

fn lost() -> KernelError {
    KernelError::Chamfer {
        edge: None,
        face: None,
        reason: LOST,
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
fn outward(body: &Body, face: FaceId, point: Vec3) -> Result<Vec3, KernelError> {
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
fn tangent(curve: &Curve3, t: f64) -> Result<Vec3, KernelError> {
    let (_, derivative) = curve.derivatives(t);
    derivative
        .normalized()
        .ok_or(KernelError::Geometry(GeomError::InvalidParameter(
            "an edge without a direction",
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

fn across(body: &Body, uses: &[Vec<Use>], edge: EdgeId) -> Result<Option<Across>, KernelError> {
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
    let side = |used: Use| -> Result<(Vec3, Vec3), KernelError> {
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
}

fn positive(name: &'static str, value: f64) -> Result<f64, KernelError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(KernelError::InvalidSize { name, value })
    }
}

/// The two distances of a size, given the angle between the directions
/// into the two faces.
fn distances(
    edge: EdgeId,
    size: ChamferSize,
    between: f64,
    tol: f64,
) -> Result<[f64; 2], KernelError> {
    let pair = match size {
        ChamferSize::Distances { first, second } => {
            [positive("distance", first)?, positive("distance", second)?]
        }
        ChamferSize::DistanceAngle { distance, angle } => {
            let distance = positive("distance", distance)?;
            if !(angle.is_finite() && angle > 0.0 && angle + between < PI) {
                return Err(refused(edge, ANGLE));
            }
            // The sine rule in the triangle of the section.
            let second = distance * angle.sin() / (angle + between).sin();
            if !(second.is_finite() && second > 0.0) {
                return Err(refused(edge, ANGLE));
            }
            [distance, second]
        }
    };
    for value in pair {
        // A chamfer narrower than what the booleans tell apart.
        if value <= 16.0 * tol {
            return Err(KernelError::InvalidSize {
                name: "distance",
                value,
            });
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

fn plan(
    body: &Body,
    uses: &[Vec<Use>],
    request: &ChamferEdge,
    tol: f64,
) -> Result<Plan, KernelError> {
    let edge = request.edge;
    let data = body.edge(edge).ok_or(refused(edge, NOT_AN_EDGE))?;
    let Some(found) = across(body, uses, edge)? else {
        return Err(refused(edge, NOT_SOLID));
    };
    let (faces, into) = if request.first == found.faces[0] {
        (found.faces, found.into)
    } else if request.first == found.faces[1] {
        (
            [found.faces[1], found.faces[0]],
            [found.into[1], found.into[0]],
        )
    } else {
        return Err(KernelError::Chamfer {
            edge: Some(edge),
            face: Some(request.first),
            reason: NOT_A_FACE,
        });
    };
    let convex = match found.turn {
        Turn::Convex => true,
        Turn::Concave => false,
        Turn::Smooth => return Err(refused(edge, SMOOTH)),
    };
    let surfaces = [
        &body.face(faces[0]).ok_or_else(lost)?.surface,
        &body.face(faces[1]).ok_or_else(lost)?.surface,
    ];
    let (path, ends) = match &data.curve {
        Curve3::Line(line) => {
            if !surfaces.iter().all(|s| matches!(s, Surface::Plane(_))) {
                return Err(refused(edge, KIND_LINE));
            }
            let along = line.direction();
            let length = data.range.1 - data.range.0;
            if !(length.is_finite() && length > tol) {
                return Err(refused(edge, KIND));
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
                return Err(refused(edge, KIND_CIRCLE));
            }
            let sweep = data.range.1 - data.range.0;
            if !(sweep.is_finite() && sweep * circle.radius() > tol && sweep <= TAU * (1.0 + 1e-9))
            {
                return Err(refused(edge, KIND));
            }
            let whole = data.start == data.end && (sweep - TAU).abs() <= 1e-9 * TAU;
            let from_axis = |t: f64| (data.curve.eval(t) - centre).normalized().ok_or_else(lost);
            let radial = from_axis(0.5 * (data.range.0 + data.range.1))?;
            let (arc, ends) = if whole {
                (None, None)
            } else if data.start == data.end {
                // An arc that closes on one vertex without being the
                // whole circle is not an edge of a valid body.
                return Err(refused(edge, KIND));
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
        Curve3::Ellipse(_) => return Err(refused(edge, KIND_ELLIPSE)),
        Curve3::Nurbs(_) => return Err(refused(edge, KIND_SPLINE)),
    };
    let between = into[0].dot(into[1]).clamp(-1.0, 1.0).acos();
    let distances = distances(edge, request.size, between, tol)?;
    Ok(Plan {
        edge,
        faces,
        into,
        distances,
        convex,
        path,
        ends,
        longer: [0.0, 0.0],
    })
}

/// Points along an edge for measuring: its ends, and for a curve some in
/// between.
fn samples(body: &Body, edge: EdgeId, fine: bool) -> Vec<Vec3> {
    let Some(data) = body.edge(edge) else {
        return Vec::new();
    };
    let count: u32 = match data.curve {
        Curve3::Line(_) if !fine => 1,
        Curve3::Line(_) | Curve3::Circle(_) | Curve3::Ellipse(_) | Curve3::Nurbs(_) => 64,
    };
    (0..=count)
        .map(|i| {
            let s = f64::from(i) / f64::from(count);
            data.curve
                .eval(data.range.0 + s * (data.range.1 - data.range.0))
        })
        .collect()
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

/// Refuses a distance for which the cut line would leave its face, with
/// the largest distance that fits. The measure is how far the rest of the
/// face's boundary is from the edge, along the face: for a straight edge
/// the part of the boundary beside the edge, less the distance of another
/// chamfer that cuts the face from there.
fn fits(body: &Body, plans: &[Plan], tol: f64) -> Result<(), KernelError> {
    let planned: BTreeMap<EdgeId, &Plan> = plans.iter().map(|p| (p.edge, p)).collect();
    for plan in plans {
        // The vertices of the edge: its two ends, or the one vertex of a
        // whole circle.
        let ends: Vec<VertexId> = body
            .edge(plan.edge)
            .map(|e| vec![e.start, e.end])
            .unwrap_or_default();
        for (side, face) in plan.faces.into_iter().enumerate() {
            let (Some(&into), Some(&distance)) = (plan.into.get(side), plan.distances.get(side))
            else {
                continue;
            };
            let mut largest = f64::INFINITY;
            for other in face_edges(body, face) {
                if other == plan.edge {
                    continue;
                }
                let Some(data) = body.edge(other) else {
                    continue;
                };
                let touches = ends.contains(&data.start) || ends.contains(&data.end);
                // What another chamfer takes of the face from that side.
                let taken = match planned.get(&other) {
                    Some(there) if !touches => there.distance_on(face).unwrap_or(0.0),
                    _ => 0.0,
                };
                let points = samples(body, other, matches!(plan.path, Path::Circle { .. }));
                let reach = match plan.path {
                    Path::Line {
                        start,
                        along,
                        length,
                    } => {
                        // How far into the face a point is, and where
                        // along the edge.
                        let place = |p: Vec3| ((p - start).dot(into), (p - start).dot(along));
                        if touches {
                            // An edge that leaves from an end of this one
                            // bounds the strip up to its own far end.
                            points
                                .iter()
                                .map(|p| place(*p).0)
                                .fold(0.0_f64, f64::max)
                                .max(0.0)
                        } else {
                            let mut nearest = f64::INFINITY;
                            for pair in points.windows(2) {
                                let [p, q] = pair else { continue };
                                let ((sp, tp), (sq, tq)) = (place(*p), place(*q));
                                // The part of the piece beside the edge.
                                let (lo, hi) = (tp.min(tq), tp.max(tq));
                                if hi <= tol || lo >= length - tol {
                                    continue;
                                }
                                let at = |t: f64| {
                                    if (tq - tp).abs() <= f64::MIN_POSITIVE {
                                        sp.min(sq)
                                    } else {
                                        sp + (sq - sp) * ((t - tp) / (tq - tp)).clamp(0.0, 1.0)
                                    }
                                };
                                // Boundary behind the edge's own line is
                                // not beside it on the face.
                                for s in [at(0.0_f64.max(lo)), at(length.min(hi))] {
                                    if s > tol {
                                        nearest = nearest.min(s);
                                    }
                                }
                            }
                            nearest
                        }
                    }
                    Path::Circle {
                        centre, axis, arc, ..
                    } => {
                        // Along a plane: across the axis; along a cylinder
                        // or a cone: the same measured along the slope.
                        // Beside an arc only what lies in its sector.
                        let beside = |p: Vec3| match arc {
                            None => true,
                            Some((from, sweep)) => {
                                let offset = p - centre;
                                let turned = offset
                                    .dot(axis.cross(from))
                                    .atan2(offset.dot(from))
                                    .rem_euclid(TAU);
                                turned > 1e-9 && turned < sweep - 1e-9
                            }
                        };
                        let start = body
                            .edge(plan.edge)
                            .map(|e| e.curve.eval(e.range.0))
                            .unwrap_or(centre);
                        let level = |p: Vec3| {
                            let offset = p - centre;
                            let height = offset.dot(axis);
                            ((offset - axis * height).length(), height)
                        };
                        let (r0, h0) = level(start);
                        let away = points.iter().filter(|p| touches || beside(**p)).map(|p| {
                            let (r, h) = level(*p);
                            (r - r0).hypot(h - h0)
                        });
                        if touches {
                            // A seam: it leaves the circle and bounds
                            // nothing before its far end.
                            away.fold(0.0_f64, f64::max)
                        } else {
                            away.fold(f64::INFINITY, f64::min)
                        }
                    }
                };
                if reach.is_finite() && (reach > tol || !touches) {
                    largest = largest.min(reach - taken);
                }
            }
            // A cut line that runs into the axis leaves nothing of a face
            // across it.
            if let Path::Circle {
                axis,
                radial,
                radius,
                ..
            } = plan.path
            {
                let inward = -into.dot(radial);
                if inward > 1e-9 && into.dot(axis).abs() < 1.0 {
                    largest = largest.min(radius / inward);
                }
            }
            if distance >= largest - 2.0 * tol {
                return Err(KernelError::ChamferTooLarge {
                    edge: plan.edge,
                    face,
                    distance,
                    largest: largest.max(0.0),
                });
            }
        }
    }
    Ok(())
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
) -> Result<Vec<Leaving>, KernelError> {
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
) -> Result<(), KernelError> {
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
        let [d1, d2] = plan.distances;
        let [b1, b2] = plan.into;
        let beyond = [
            -tangent(&data.curve, data.range.0)?,
            tangent(&data.curve, data.range.1)?,
        ];
        let mut longer = [0.0, 0.0];
        for (slot, (vertex, beyond)) in longer.iter_mut().zip(ends.into_iter().zip(beyond)) {
            let others: Vec<Leaving> = leaving(body, uses, at, vertex)?
                .into_iter()
                .filter(|l| l.edge != plan.edge)
                .collect();
            let goes_on = others.iter().any(|l| l.direction.dot(beyond) > 1.0 - 1e-9);
            if goes_on || others.iter().any(|l| l.turn != own) {
                continue;
            }
            match plan.path {
                Path::Line { start, along, .. } => {
                    // The chamfer plane holds the edge's direction and
                    // the line between the two cut lines. How far past
                    // the vertex does it cut the other edges there?
                    let Some(normal) = along.cross(b1 * d1 - b2 * d2).normalized() else {
                        return Err(refused(plan.edge, LOST));
                    };
                    let corner = body.vertex(vertex).ok_or_else(lost)?.point;
                    let height = normal.dot(corner - (start + b1 * d1));
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
    }
    Ok(())
}

/// A tool and which of its faces is the chamfer face.
struct Tool {
    body: Body,
    slope: FaceId,
}

fn line(id: u64, start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// The face of a swept triangle that the curve `1` swept.
fn slope_of(result: crate::extrude::OpResult) -> Result<Tool, KernelError> {
    let slope = result
        .origins
        .faces
        .iter()
        .position(|o| o.role == Role::Side && o.sources == [1])
        .ok_or_else(lost)?;
    Ok(Tool {
        body: result.body,
        slope: FaceId(index_u32(slope)),
    })
}

fn tool(plan: &Plan, tolerance: &Tolerance) -> Result<Tool, KernelError> {
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
            let p1 = [d1, 0.0];
            let p2 = [d2 * b2.dot(frame.x()), d2 * b2.dot(frame.y())];
            let curves = [
                line(0, [0.0, 0.0], p1),
                line(1, p1, p2),
                line(2, p2, [0.0, 0.0]),
            ];
            let profiles = detect_profiles(&curves, tolerance)?;
            let total = length + plan.longer[0] + plan.longer[1];
            slope_of(extrude(&profiles, &frame, 0.0, total, tolerance)?)
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
            let curves = [line(0, rim, p1), line(1, p1, p2), line(2, p2, rim)];
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

/// What a face of the body under construction is made of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Made {
    kept: BTreeSet<FaceId>,
    chamfers: BTreeSet<usize>,
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
    /// the tool (B) of the edge with index `plan`, whose face `mark` is to
    /// be known as `marked`.
    fn take(&mut self, result: BooleanResult, mark: FaceId, marked: &Made, plan: usize) {
        let origin = |source: &FaceSource| match source.operand {
            Operand::A => self
                .faces
                .get(source.face.0 as usize)
                .cloned()
                .unwrap_or_default(),
            Operand::B if source.face == mark => marked.clone(),
            Operand::B => Made {
                scrap: BTreeSet::from([plan]),
                ..Made::default()
            },
        };
        let mut faces = Vec::with_capacity(result.faces.len());
        for (index, source) in result.faces.iter().enumerate() {
            let mut made = origin(source);
            for other in result.merged.get(index).into_iter().flatten() {
                let more = origin(other);
                made.kept.extend(more.kept);
                made.chamfers.extend(more.chamfers);
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
    tool: &Tool,
    add: bool,
    marked: &Made,
    first: bool,
    (plan, edge): (usize, EdgeId),
    tolerance: &Tolerance,
) -> Result<(), KernelError> {
    let op = if add {
        BooleanOp::Union
    } else {
        BooleanOp::Subtract
    };
    let options = BooleanOptions {
        merge: true,
        // What the kernel itself made has passed the check.
        trust_operands: !first,
    };
    let result = boolean_with(&state.body, &tool.body, op, tolerance, &options).map_err(
        |error| match error {
            KernelError::InvalidBody { .. } if first => error,
            other => KernelError::ChamferBoolean {
                edge,
                error: Box::new(other),
            },
        },
    )?;
    state.take(result, tool.slope, marked, plan);
    Ok(())
}

/// A vertex where three chamfered edges meet and the corner between them.
struct Corner {
    vertex: VertexId,
    /// The indices of the three plans.
    plans: [usize; 3],
    /// The three points the corner face goes through.
    points: [Vec3; 3],
    add: bool,
}

fn corners(
    body: &Body,
    uses: &[Vec<Use>],
    at: &[Vec<EdgeId>],
    plans: &[Plan],
    tol: f64,
) -> Result<Vec<Corner>, KernelError> {
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
        let straight = |p: &Plan| matches!(p.path, Path::Line { .. });
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
            points: [p0, p1, p2],
            add: !pa.convex,
        });
    }
    Ok(found)
}

/// A block that lies against the plane through the corner's three points,
/// on the side of the vertex, and is as wide as the corner twice over.
fn corner_tool(body: &Body, corner: &Corner) -> Result<Tool, KernelError> {
    let [p0, p1, p2] = corner.points;
    let at = body.vertex(corner.vertex).ok_or_else(lost)?.point;
    let refuse = || KernelError::Chamfer {
        edge: None,
        face: None,
        reason: NO_CORNER,
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
    })
}

/// Holds the result against what a chamfer is (see the module
/// documentation) and turns it into what is reported.
fn finish(
    at: &[Vec<EdgeId>],
    uses: &[Vec<Use>],
    plans: &[Plan],
    mut state: State,
) -> Result<ChamferResult, KernelError> {
    let mut faces = Vec::with_capacity(state.faces.len());
    let mut of_plan: Vec<Vec<usize>> = vec![Vec::new(); plans.len()];
    for (index, made) in state.faces.iter().enumerate() {
        let first = made
            .chamfers
            .iter()
            .chain(&made.scrap)
            .next()
            .and_then(|i| plans.get(*i))
            .map(|p| p.edge);
        let lone = |reason| KernelError::Chamfer {
            edge: first,
            face: None,
            reason,
        };
        let kinds = usize::from(!made.kept.is_empty())
            + usize::from(!made.chamfers.is_empty())
            + usize::from(!made.corners.is_empty());
        if kinds == 0 {
            return Err(lone(ENDS_BADLY));
        }
        if kinds > 1 {
            return Err(lone(IN_A_FACE));
        }
        if !made.kept.is_empty() {
            faces.push(ChamferFace::Kept(made.kept.iter().copied().collect()));
        } else if let Some(vertex) = made.corners.iter().next() {
            faces.push(ChamferFace::Corner(*vertex));
        } else {
            for &plan in &made.chamfers {
                if let Some(list) = of_plan.get_mut(plan) {
                    list.push(index);
                }
            }
            faces.push(ChamferFace::Chamfer(
                made.chamfers.iter().copied().collect(),
            ));
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
            return Err(refused(plan.edge, IN_PIECES));
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
                made.chamfers.iter().any(|i| {
                    plans.get(*i).is_some_and(|p| {
                        p.ends
                            .into_iter()
                            .flatten()
                            .any(|vertex| ends.contains(&vertex))
                    })
                })
            };
            if !fine {
                return Err(KernelError::Chamfer {
                    edge: Some(plan.edge),
                    face: made.kept.iter().next().copied(),
                    reason: REACHES,
                });
            }
        }
        if met != [true, true] {
            let missing = if met[0] { plan.faces[1] } else { plan.faces[0] };
            return Err(KernelError::Chamfer {
                edge: Some(plan.edge),
                face: Some(missing),
                reason: LOST_NEIGHBOUR,
            });
        }
    }
    for (index, plan) in plans.iter().enumerate() {
        reframe(&mut state.body, &of_plan, index, plan);
    }
    Ok(ChamferResult {
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
    let [d1, _] = plan.distances;
    let forward = b1.cross(b2).dot(along) >= 0.0;
    let (x, from) = if forward {
        (along, start)
    } else {
        (-along, start + along * length)
    };
    if let Some(frame) = Frame::new(from + b1 * d1, outward, x) {
        face.surface = Surface::Plane(craft3d_geom::Plane::new(frame));
        face.reversed = false;
    }
}

/// Chamfers `edges` of the solid `body`.
///
/// The result has passed `craft3d_topo::check` with `tolerance` and is
/// reported with the source of every face, edge and vertex. See the
/// module documentation for the method, for what the corner is where
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
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if edges.len() > MAX_CHAMFER_EDGES {
        return Err(KernelError::Chamfer {
            edge: None,
            face: None,
            reason: TOO_MANY,
        });
    }
    if body.kind != BodyKind::Solid {
        return Err(KernelError::Chamfer {
            edge: edges.first().map(|e| e.edge),
            face: None,
            reason: NOT_SOLID,
        });
    }
    let uses = edge_uses(body);
    let at = vertex_edges(body);
    let mut seen = BTreeSet::new();
    let mut plans = Vec::with_capacity(edges.len());
    for request in edges {
        if !seen.insert(request.edge) {
            return Err(refused(request.edge, TWICE));
        }
        plans.push(plan(body, &uses, request, tol)?);
    }
    fits(body, &plans, tol)?;
    lengthen(body, &uses, &at, &mut plans)?;
    let found = corners(body, &uses, &at, &plans, tol)?;

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
    let mut first = true;
    for (index, plan) in plans.iter().enumerate() {
        let made = tool(plan, tolerance)?;
        let marked = Made {
            chamfers: BTreeSet::from([index]),
            ..Made::default()
        };
        combine(
            &mut state,
            &made,
            !plan.convex,
            &marked,
            first,
            (index, plan.edge),
            tolerance,
        )?;
        first = false;
    }
    for corner in &found {
        let made = corner_tool(body, corner)?;
        let marked = Made {
            corners: BTreeSet::from([corner.vertex]),
            ..Made::default()
        };
        let index = corner.plans.first().copied().unwrap_or(usize::MAX);
        let edge = plans.get(index).map_or(EdgeId(u32::MAX), |p| p.edge);
        combine(
            &mut state,
            &made,
            corner.add,
            &marked,
            first,
            (index, edge),
            tolerance,
        )?;
        first = false;
    }
    finish(&at, &uses, &plans, state)
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
        if chain.len() > MAX_CHAMFER_EDGES {
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
