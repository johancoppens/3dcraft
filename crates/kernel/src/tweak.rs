//! The local operation the face operations share: faces of a solid are
//! given new surfaces, and the body is closed again around them.
//!
//! **The recipe** (docs/design/face-operations.md › *Face operations as
//! built*). Nothing about how the body hangs together changes: every
//! face, edge and vertex of the result is the face, edge and vertex of
//! the body with the same number. What changes is where they lie:
//!
//! 1. every vertex on a face that got a new surface is found again as the
//!    point where the surfaces of its faces meet, nearest to where it was
//!    (Newton's iteration on the distances to the surfaces; one step for
//!    planes);
//! 2. every edge that ends in such a vertex is made again on the surfaces
//!    of its two faces: a straight edge through its two vertices, a
//!    circular edge about the axis its faces share. That is "extend the
//!    neighbours and intersect again" for the curves two such surfaces
//!    meet in: a neighbour is not cut or lengthened as a separate step,
//!    its boundary simply moves;
//! 3. the result is held against what a body is.
//!
//! **Never a wrong solid.** A vertex whose faces no longer meet in a
//! point, an edge that would no longer lie on both its faces, an edge
//! that shrinks to nothing or turns round (which is how a neighbour that
//! vanishes shows), a flat face whose outline crosses itself or whose
//! hole leaves it, and a round face whose rims change places are each an
//! error that names the entity. What such a local check cannot see, two
//! faces far apart running into each other, is left to the caller's
//! check on the whole body (regeneration meshes the result and looks for
//! places where it passes through itself).

use crate::KernelError;
use crate::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_geom::{Circle, Curve3, Line, Surface};
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, VertexId};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

/// Why a body could not be closed around the new surfaces.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TweakError {
    Kernel(KernelError),
    /// The geometry is not built: the faces and the edge it is about.
    Unsupported {
        faces: Vec<FaceId>,
        edge: Option<EdgeId>,
        reason: &'static str,
    },
    /// The faces moved so far that this edge or face is no longer there,
    /// or that these faces no longer meet where they did.
    Vanishes {
        faces: Vec<FaceId>,
        edge: Option<EdgeId>,
        reason: &'static str,
    },
}

impl From<KernelError> for TweakError {
    fn from(error: KernelError) -> Self {
        TweakError::Kernel(error)
    }
}

pub(crate) const EDGE_GONE: &str =
    "the edge shrinks to nothing or turns round: a face next to it would vanish";
pub(crate) const NO_MEETING: &str =
    "the faces that meet in a vertex there no longer meet in one point";
pub(crate) const OFF_SURFACE: &str = "the edge would no longer be a line or a circle on both its faces: continuing these two surfaces until they meet again is not built";
pub(crate) const CURVE_KIND: &str = "an elliptic or freeform edge next to a face that moves is not built: these two surfaces would have to be intersected again";
pub(crate) const FREEFORM: &str = "a freeform face next to a face that moves cannot be continued past its edge, and the faces no longer meet in the vertex there";
pub(crate) const OUTLINE: &str =
    "the outline of the face would cross itself, or a hole in it would leave it";
pub(crate) const RIMS: &str = "the two rims of the face would change places: the face would vanish";
pub(crate) const THROUGH: &str =
    "the face would pass through another part of the body, or lie in another face of it";
pub(crate) const TURNED: &str = "the face would be turned inside out: it would vanish";

fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// Per edge, the faces that use it (twice the same face for a seam).
pub(crate) fn edge_faces(body: &Body) -> Vec<Vec<FaceId>> {
    let mut uses: Vec<Vec<FaceId>> = vec![Vec::new(); body.edges.len()];
    for (index, face) in body.faces.iter().enumerate() {
        for coedge in face
            .loops
            .iter()
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges)
        {
            if let Some(list) = uses.get_mut(coedge.edge.0 as usize) {
                list.push(FaceId(index_u32(index)));
            }
        }
    }
    uses
}

/// The signed distance from `point` to `surface` and the direction it
/// grows in. `None` where the surface has no nearest point with a normal
/// (a cone's apex, the axis of a cylinder).
fn distance(surface: &Surface, point: Vec3) -> Option<(f64, Vec3)> {
    let (u, v) = surface.project(point).ok()?;
    let foot = surface.eval(u, v);
    let normal = surface.normal(u, v)?;
    let offset = point - foot;
    // Along the normal when the foot is the nearest point; at the rim of
    // a bounded surface (the apex of a cone) it is not, and the whole
    // distance counts.
    let along = offset.dot(normal);
    let aside = (offset - normal * along).length();
    let value = if aside > along.abs() {
        offset.length()
    } else {
        along
    };
    (value.is_finite() && normal.is_finite()).then_some((value, normal))
}

/// Normals nearer to each other than this sine are one direction: the
/// faces are tangent there, and one of them says nothing the other does
/// not.
const SAME_DIRECTION: f64 = 1e-6;

/// Most steps of the iteration for one vertex.
const MEET_STEPS: usize = 64;

/// The point nearest `start` where `surfaces` meet, within `tol` of each.
fn meet(surfaces: &[&Surface], start: Vec3, tol: f64) -> Option<Vec3> {
    let mut point = start;
    for _ in 0..MEET_STEPS {
        // An orthonormal basis of the normals, and for each one that adds
        // a direction, its distance and its components along the basis.
        let mut basis: Vec<Vec3> = Vec::with_capacity(3);
        let mut rows: Vec<(f64, Vec<f64>)> = Vec::with_capacity(3);
        for surface in surfaces {
            let (value, normal) = distance(surface, point)?;
            let mut rest = normal;
            let mut components = Vec::with_capacity(3);
            for axis in &basis {
                let along = normal.dot(*axis);
                components.push(along);
                rest = rest - *axis * along;
            }
            let own = rest.length();
            if own <= SAME_DIRECTION || basis.len() >= 3 {
                continue;
            }
            components.push(own);
            basis.push(rest * (1.0 / own));
            rows.push((value, components));
        }
        // The smallest step that brings the point onto every surface that
        // adds a direction: forward substitution, the rows are triangular.
        let mut step = Vec3::ZERO;
        let mut solved: Vec<f64> = Vec::with_capacity(3);
        for (value, components) in &rows {
            let mut right = -*value;
            let mut own = 0.0;
            for (k, component) in components.iter().enumerate() {
                match solved.get(k) {
                    Some(c) => right -= component * c,
                    None => own = *component,
                }
            }
            if own == 0.0 {
                return None;
            }
            solved.push(right / own);
        }
        for (c, axis) in solved.iter().zip(&basis) {
            step += *axis * *c;
        }
        if !step.is_finite() {
            return None;
        }
        point += step;
        if step.length() <= 1e-15 * (1.0 + point.length()) {
            break;
        }
    }
    let on_all = surfaces
        .iter()
        .all(|surface| distance(surface, point).is_some_and(|(value, _)| value.abs() <= tol));
    (point.is_finite() && on_all).then_some(point)
}

/// The angle nearest `near` that is `angle` up to whole turns.
pub(crate) fn unwrapped(angle: f64, near: f64) -> f64 {
    let mut difference = (angle - near) % TAU;
    if difference > PI {
        difference -= TAU;
    } else if difference < -PI {
        difference += TAU;
    }
    near + difference
}

/// The axis a circle on `surface` turns about, when the surface has one:
/// a point of it and its direction. A sphere turns about any line through
/// its centre, so about the one along `along`.
fn axis_of(surface: &Surface, along: Vec3) -> Option<(Vec3, Vec3)> {
    match surface {
        Surface::Cylinder(s) => Some((s.frame().origin(), s.frame().z())),
        Surface::Cone(s) => Some((s.frame().origin(), s.frame().z())),
        Surface::Torus(s) => Some((s.frame().origin(), s.frame().z())),
        Surface::Sphere(s) => Some((s.frame().origin(), along)),
        Surface::Plane(_) | Surface::Bezier(_) | Surface::Nurbs(_) => None,
    }
}

/// Where along an edge it is held against its faces.
const ON_SURFACE_SAMPLES: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

fn lies_on(curve: &Curve3, range: (f64, f64), surfaces: &[&Surface], tol: f64) -> bool {
    ON_SURFACE_SAMPLES.iter().all(|s| {
        let point = curve.eval(range.0 + (range.1 - range.0) * s);
        surfaces
            .iter()
            .all(|surface| distance(surface, point).is_some_and(|(value, _)| value.abs() <= tol))
    })
}

/// What an edge is made into.
enum Remade {
    Edge(Curve3, (f64, f64)),
    Gone,
    OffSurface,
    Kind,
}

/// True when `a` is more than `b`, and both are numbers.
fn more(a: f64, b: f64) -> bool {
    a.is_finite() && b.is_finite() && a > b
}

/// The edge with curve `old` over `range` made again from `start` to
/// `end` on `surfaces`, the surfaces of its faces; `moved` says whether
/// one of them is new.
#[allow(clippy::too_many_arguments)]
fn remake(
    old: &Curve3,
    range: (f64, f64),
    start: Vec3,
    end: Vec3,
    closed: bool,
    surfaces: &[&Surface],
    moved: bool,
    tolerance: &Tolerance,
) -> Remade {
    let tol = tolerance.linear;
    let middle = 0.5 * (range.0 + range.1);
    match old {
        Curve3::Line(_) => {
            let (_, was) = old.derivatives(middle);
            let length = start.distance(end);
            if !more(length, tol) || (end - start).dot(was) <= 0.0 {
                return Remade::Gone;
            }
            let Ok(line) = Line::through(start, end) else {
                return Remade::Gone;
            };
            let curve = Curve3::Line(line);
            if lies_on(&curve, (0.0, length), surfaces, tol) {
                Remade::Edge(curve, (0.0, length))
            } else {
                Remade::OffSurface
            }
        }
        Curve3::Circle(circle) => {
            let along = circle.frame().z();
            let parallel =
                |direction: Vec3| direction.cross(along).length() <= tolerance.angular.max(1e-12);
            let (origin, _) = surfaces
                .iter()
                .filter_map(|surface| axis_of(surface, along))
                .find(|(_, direction)| parallel(*direction))
                .unwrap_or((circle.frame().origin(), along));
            let centre = origin + along * (start - origin).dot(along);
            let radius = start.distance(centre);
            if !more(radius, tol) {
                return Remade::Gone;
            }
            let Some(frame) = circle.frame().at(centre) else {
                return Remade::Gone;
            };
            let Ok(made) = Circle::new(frame, radius) else {
                return Remade::Gone;
            };
            let curve = Curve3::Circle(made);
            let (Ok(at_start), Ok(at_end)) = (curve.project(start), curve.project(end)) else {
                return Remade::Gone;
            };
            let from = unwrapped(at_start, range.0);
            // Angles that did not change stay what they were, bit for bit.
            let from = if (from - range.0).abs() <= 1e-12 {
                range.0
            } else {
                from
            };
            let to = if closed {
                from + (range.1 - range.0)
            } else {
                let to = unwrapped(at_end, range.1);
                if (to - range.1).abs() <= 1e-12 {
                    range.1
                } else {
                    to
                }
            };
            if !more(to, from) || (to - from) * radius <= tol || to - from > TAU + 1e-9 {
                return Remade::Gone;
            }
            if lies_on(&curve, (from, to), surfaces, tol) {
                Remade::Edge(curve, (from, to))
            } else {
                Remade::OffSurface
            }
        }
        Curve3::Ellipse(_) | Curve3::Nurbs(_) => {
            if moved {
                return Remade::Kind;
            }
            // The same curve, between the places its ends moved to.
            let (Ok(at_start), Ok(at_end)) = (old.project(start), old.project(end)) else {
                return Remade::Kind;
            };
            let (from, to) = match old.period() {
                Some(_) if closed => {
                    let from = unwrapped(at_start, range.0);
                    (from, from + (range.1 - range.0))
                }
                Some(_) => (unwrapped(at_start, range.0), unwrapped(at_end, range.1)),
                None => (at_start, at_end),
            };
            if old.eval(from).distance(start) > tol || old.eval(to).distance(end) > tol {
                return Remade::Kind;
            }
            if !more(to, from) {
                return Remade::Gone;
            }
            Remade::Edge(old.clone(), (from, to))
        }
    }
}

/// Points along an edge, in the direction of its curve: enough to tell
/// which way a loop turns and where it lies.
fn sampled(body: &Body, edge: EdgeId) -> Vec<Vec3> {
    let Some(edge) = body.edge(edge) else {
        return Vec::new();
    };
    let steps = match edge.curve {
        Curve3::Line(_) => 1_u32,
        _ => 16,
    };
    (0..=steps)
        .map(|i| {
            let s = f64::from(i) / f64::from(steps);
            edge.curve
                .eval(edge.range.0 + (edge.range.1 - edge.range.0) * s)
        })
        .collect()
}

/// The points of a loop in order, each edge in the loop's direction.
pub(crate) fn loop_points(body: &Body, loop_: craft3d_topo::LoopId) -> Vec<Vec3> {
    let mut points = Vec::new();
    for coedge in body.loop_(loop_).into_iter().flat_map(|l| &l.coedges) {
        let mut along = sampled(body, coedge.edge);
        if coedge.reversed {
            along.reverse();
        }
        points.extend(along);
    }
    points
}

/// The area a closed row of points in the plane of `frame` goes round,
/// counter-clockwise positive seen against the frame's z.
fn signed_area(frame: &Frame, points: &[Vec3]) -> f64 {
    let flat: Vec<(f64, f64)> = points
        .iter()
        .map(|p| {
            let (x, y, _) = frame.local(*p);
            (x, y)
        })
        .collect();
    let mut twice = 0.0;
    for (index, (x0, y0)) in flat.iter().enumerate() {
        let Some((x1, y1)) = flat.get((index + 1) % flat.len().max(1)) else {
            continue;
        };
        twice += x0 * y1 - x1 * y0;
    }
    0.5 * twice
}

/// Whether the loops of the flat face `face` are simple curves that do
/// not meet, one of them around the others. `None` when the face has an
/// edge this cannot tell for (an ellipse, a spline).
fn outline_is_simple(body: &Body, face: FaceId, tolerance: &Tolerance) -> Option<bool> {
    let face = body.face(face)?;
    let Surface::Plane(plane) = &face.surface else {
        return None;
    };
    let frame = plane.frame();
    let flat = |p: Vec3| {
        let (x, y, _) = frame.local(p);
        [x, y]
    };
    let mut curves = Vec::new();
    let mut seen: BTreeSet<EdgeId> = BTreeSet::new();
    for coedge in face
        .loops
        .iter()
        .filter_map(|l| body.loop_(*l))
        .flat_map(|l| &l.coedges)
    {
        if !seen.insert(coedge.edge) {
            // A seam in a flat face: not an outline this can judge.
            return None;
        }
        let edge = body.edge(coedge.edge)?;
        let (start, end) = (body.vertex(edge.start)?.point, body.vertex(edge.end)?.point);
        let curve = match &edge.curve {
            Curve3::Line(_) => Curve2::Line {
                start: flat(start),
                end: flat(end),
            },
            Curve3::Circle(circle) => {
                let centre = flat(circle.frame().origin());
                if edge.start == edge.end {
                    Curve2::Circle {
                        center: centre,
                        radius: circle.radius(),
                    }
                } else if circle.frame().z().dot(frame.z()) > 0.0 {
                    Curve2::Arc {
                        center: centre,
                        start: flat(start),
                        end: flat(end),
                    }
                } else {
                    Curve2::Arc {
                        center: centre,
                        start: flat(end),
                        end: flat(start),
                    }
                }
            }
            Curve3::Ellipse(_) | Curve3::Nurbs(_) => return None,
        };
        curves.push(SketchCurve {
            id: u64::from(coedge.edge.0),
            curve,
        });
    }
    if curves.is_empty() {
        return Some(true);
    }
    Some(match detect_profiles(&curves, tolerance) {
        Ok(profiles) => match profiles.as_slice() {
            [one] => one.holes.len() + 1 == face.loops.len(),
            _ => false,
        },
        Err(_) => false,
    })
}

/// Per loop of a round face about an axis, where along the axis it lies
/// (the mean over its points).
fn heights(body: &Body, face: FaceId) -> Option<Vec<f64>> {
    let face = body.face(face)?;
    let (origin, along) = match &face.surface {
        Surface::Cylinder(s) => (s.frame().origin(), s.frame().z()),
        Surface::Cone(s) => (s.frame().origin(), s.frame().z()),
        Surface::Sphere(s) => (s.frame().origin(), s.frame().z()),
        _ => return None,
    };
    Some(
        face.loops
            .iter()
            .map(|l| {
                let points = loop_points(body, *l);
                let sum: f64 = points.iter().map(|p| (*p - origin).dot(along)).sum();
                sum / (points.len().max(1) as f64)
            })
            .collect(),
    )
}

/// The faces that have an edge among `edges`, ascending.
fn faces_with(uses: &[Vec<FaceId>], edges: &BTreeSet<EdgeId>) -> BTreeSet<FaceId> {
    edges
        .iter()
        .filter_map(|edge| uses.get(edge.0 as usize))
        .flatten()
        .copied()
        .collect()
}

/// `body` with the surfaces of `surfaces` in the place of those of its
/// faces, closed again around them (see the module documentation). The
/// result has the faces, edges and vertices of `body`, each under its
/// own number, and passes `craft3d_topo::check`.
pub(crate) fn resurface(
    body: &Body,
    surfaces: &BTreeMap<FaceId, Surface>,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    resurface_at(body, surfaces, &BTreeSet::new(), operation, tolerance)
}

/// [`resurface`], with the vertices of `also` found again too, although
/// none of their faces has a new surface: the vertices a change of
/// topology put where they do not lie yet (delete face merges the two
/// ends of a strip into one vertex, which lies where the faces round it
/// meet). The edges that end in them are made again likewise.
pub(crate) fn resurface_at(
    body: &Body,
    surfaces: &BTreeMap<FaceId, Surface>,
    also: &BTreeSet<VertexId>,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    resurface_with(body, surfaces, also, None, operation, tolerance)
}

/// [`resurface`] for faces that all move by one rigid motion `rigid`
/// (their new surfaces are the images of the old ones): a vertex all of
/// whose faces move, and an edge both of whose faces do, are the images
/// of what they were, exactly, whatever their kind; the rest is found
/// again as [`resurface`] finds it.
pub(crate) fn resurface_rigid(
    body: &Body,
    surfaces: &BTreeMap<FaceId, Surface>,
    rigid: &Transform,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    resurface_with(
        body,
        surfaces,
        &BTreeSet::new(),
        Some(rigid),
        operation,
        tolerance,
    )
}

fn resurface_with(
    body: &Body,
    surfaces: &BTreeMap<FaceId, Surface>,
    also: &BTreeSet<VertexId>,
    rigid: Option<&Transform>,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    let tol = tolerance.linear;
    // Whether every one of `faces` moves with the rigid motion.
    let carried = |faces: &[FaceId]| {
        rigid.is_some() && !faces.is_empty() && faces.iter().all(|face| surfaces.contains_key(face))
    };
    let uses = edge_faces(body);
    let mut made = body.clone();
    for (face, surface) in surfaces {
        let Some(slot) = made.faces.get_mut(face.0 as usize) else {
            return Err(TweakError::Unsupported {
                faces: vec![*face],
                edge: None,
                reason: "there is no such face in the body",
            });
        };
        slot.surface = surface.clone();
    }

    // The faces around every vertex, and the vertices on a new surface.
    let mut around: Vec<BTreeSet<FaceId>> = vec![BTreeSet::new(); body.vertices.len()];
    for (index, edge) in body.edges.iter().enumerate() {
        for vertex in [edge.start, edge.end] {
            if let (Some(list), Some(faces)) = (around.get_mut(vertex.0 as usize), uses.get(index))
            {
                list.extend(faces.iter().copied());
            }
        }
    }
    let mut moved_vertices: BTreeSet<VertexId> = BTreeSet::new();
    for (index, faces) in around.iter().enumerate() {
        let vertex = VertexId(index_u32(index));
        if !(also.contains(&vertex) || faces.iter().any(|face| surfaces.contains_key(face))) {
            continue;
        }
        let Some(was) = body.vertex(vertex).map(|v| v.point) else {
            continue;
        };
        if let Some(motion) = rigid
            && carried(&faces.iter().copied().collect::<Vec<_>>())
        {
            let point = motion.point(was);
            if !point.is_finite() {
                return Err(TweakError::Kernel(KernelError::InvalidSize {
                    name: "motion",
                    value: f64::NAN,
                }));
            }
            if let Some(slot) = made.vertices.get_mut(index) {
                slot.point = point;
            }
            moved_vertices.insert(vertex);
            continue;
        }
        let meeting: Vec<&Surface> = faces
            .iter()
            .filter_map(|face| made.face(*face).map(|f| &f.surface))
            .collect();
        let Some(point) = meet(&meeting, was, tol) else {
            if meeting
                .iter()
                .any(|s| matches!(s, Surface::Bezier(_) | Surface::Nurbs(_)))
            {
                return Err(TweakError::Unsupported {
                    faces: faces.iter().copied().collect(),
                    edge: None,
                    reason: FREEFORM,
                });
            }
            return Err(TweakError::Vanishes {
                faces: faces.iter().copied().collect(),
                edge: None,
                reason: NO_MEETING,
            });
        };
        if let Some(slot) = made.vertices.get_mut(index) {
            slot.point = point;
        }
        moved_vertices.insert(vertex);
    }

    // Every edge that ends in a vertex that moved, on its two surfaces.
    let mut remade: BTreeSet<EdgeId> = BTreeSet::new();
    for (index, edge) in body.edges.iter().enumerate() {
        if !(moved_vertices.contains(&edge.start) || moved_vertices.contains(&edge.end)) {
            continue;
        }
        let id = EdgeId(index_u32(index));
        let faces: Vec<FaceId> = uses.get(index).cloned().unwrap_or_default();
        let on: Vec<&Surface> = faces
            .iter()
            .filter_map(|face| made.face(*face).map(|f| &f.surface))
            .collect();
        let moved = faces.iter().any(|face| surfaces.contains_key(face));
        let (Some(start), Some(end)) = (
            made.vertex(edge.start).map(|v| v.point),
            made.vertex(edge.end).map(|v| v.point),
        ) else {
            continue;
        };
        if let Some(motion) = rigid
            && carried(&faces)
        {
            // The image of the edge, with the same parameters.
            let curve = edge.curve.transformed(motion).map_err(KernelError::from)?;
            if let Some(slot) = made.edges.get_mut(index) {
                slot.curve = curve;
            }
            remade.insert(id);
            continue;
        }
        let closed = edge.start == edge.end;
        let fail = |vanishes: bool, reason: &'static str| {
            let (faces, edge) = (faces.clone(), Some(id));
            if vanishes {
                TweakError::Vanishes {
                    faces,
                    edge,
                    reason,
                }
            } else {
                TweakError::Unsupported {
                    faces,
                    edge,
                    reason,
                }
            }
        };
        match remake(
            &edge.curve,
            edge.range,
            start,
            end,
            closed,
            &on,
            moved,
            tolerance,
        ) {
            Remade::Edge(curve, range) => {
                if let Some(slot) = made.edges.get_mut(index) {
                    slot.curve = curve;
                    slot.range = range;
                }
            }
            Remade::Gone => return Err(fail(true, EDGE_GONE)),
            // Far enough, a line or a circle leaves its surfaces because
            // they no longer meet as they did; near enough it never does
            // when the kind is one this builds. The caller tells the two
            // apart by trying a smaller move.
            Remade::OffSurface => return Err(fail(true, OFF_SURFACE)),
            Remade::Kind => return Err(fail(false, CURVE_KIND)),
        }
        remade.insert(id);
    }

    // What a face next to a moved edge has to be still.
    for face in faces_with(&uses, &remade) {
        let (Some(before), Some(after)) = (body.face(face), made.face(face)) else {
            continue;
        };
        let gone = |reason: &'static str| TweakError::Vanishes {
            faces: vec![face],
            edge: None,
            reason,
        };
        if let (Surface::Plane(was), Surface::Plane(is)) = (&before.surface, &after.surface) {
            for loop_ in &before.loops {
                let area_before = signed_area(was.frame(), &loop_points(body, *loop_));
                let area_after = signed_area(is.frame(), &loop_points(&made, *loop_));
                if area_before * area_after <= 0.0 && area_before != 0.0 {
                    return Err(gone(TURNED));
                }
            }
            if outline_is_simple(&made, face, tolerance) == Some(false)
                && outline_is_simple(body, face, tolerance) == Some(true)
            {
                return Err(gone(OUTLINE));
            }
        }
        if let (Some(was), Some(is)) = (heights(body, face), heights(&made, face)) {
            for (i, (a0, a1)) in was.iter().zip(&is).enumerate() {
                for (b0, b1) in was.iter().zip(&is).skip(i + 1) {
                    let (before, after) = (a0 - b0, a1 - b1);
                    if before.abs() > tol && (after.abs() <= tol || before * after < 0.0) {
                        return Err(gone(RIMS));
                    }
                }
            }
        }
    }

    let defects = craft3d_topo::check(&made, tolerance);
    if !defects.is_empty() {
        return Err(TweakError::Kernel(KernelError::InvalidBody {
            operation,
            defects: defects.into_iter().take(crate::MAX_REPORTED).collect(),
        }));
    }
    if let Some(face) = buried(&made, &uses, &remade, tolerance) {
        return Err(TweakError::Vanishes {
            faces: vec![face],
            edge: None,
            reason: THROUGH,
        });
    }
    Ok(made)
}

/// How far inside a face from its edge, and how far off the face, the
/// points lie that are asked about, in tolerances.
const INSIDE_FACE: f64 = 8.0;
const OFF_FACE: f64 = 4.0;

/// A face next to an edge of `remade` that is not skin of `made` any
/// more: just outside it there is material, or just inside it there is
/// none, which is what a body that passes through itself looks like next
/// to the faces that moved. The question is put to the classification of
/// the booleans (a ray from the point), at a point beside the middle of
/// every such edge; a point it cannot tell for says nothing.
fn buried(
    made: &Body,
    uses: &[Vec<FaceId>],
    remade: &BTreeSet<EdgeId>,
    tolerance: &Tolerance,
) -> Option<FaceId> {
    let tol = tolerance.linear;
    let mut asked: Vec<(FaceId, bool)> = Vec::new();
    let mut points: Vec<Vec3> = Vec::new();
    for id in faces_with(uses, remade) {
        let Some(face) = made.face(id) else {
            continue;
        };
        let outward = |at: Vec3| {
            let (u, v) = face.surface.project(at).ok()?;
            let normal = face.surface.normal(u, v)?;
            let foot = face.surface.eval(u, v);
            Some((foot, if face.reversed { -normal } else { normal }))
        };
        for coedge in face
            .loops
            .iter()
            .filter_map(|l| made.loop_(*l))
            .flat_map(|l| &l.coedges)
        {
            if !remade.contains(&coedge.edge) {
                continue;
            }
            let Some(edge) = made.edge(coedge.edge) else {
                continue;
            };
            let (middle, along) = edge.curve.derivatives(0.5 * (edge.range.0 + edge.range.1));
            let along = if coedge.reversed { -along } else { along };
            let Some((_, normal)) = outward(middle) else {
                continue;
            };
            // The face lies to the left of its edges, seen from outside.
            let Some(into) = normal.cross(along).normalized() else {
                continue;
            };
            let Some((foot, normal)) = outward(middle + into * (INSIDE_FACE * tol)) else {
                continue;
            };
            points.push(foot + normal * (OFF_FACE * tol));
            asked.push((id, false));
            points.push(foot - normal * (OFF_FACE * tol));
            asked.push((id, true));
        }
    }
    let found = crate::boolean::contains_points(made, &points, tolerance).ok()?;
    asked
        .iter()
        .zip(found)
        .find(|((_, expected), inside)| inside.is_some_and(|inside| inside != *expected))
        .map(|((face, _), _)| *face)
}

/// The vertices on the loops of `face`, ascending, each once.
pub(crate) fn face_vertices(body: &Body, face: FaceId) -> BTreeSet<VertexId> {
    body.face(face)
        .into_iter()
        .flat_map(|f| &f.loops)
        .filter_map(|l| body.loop_(*l))
        .flat_map(|l| &l.coedges)
        .filter_map(|c| body.edge(c.edge))
        .flat_map(|e| [e.start, e.end])
        .collect()
}
