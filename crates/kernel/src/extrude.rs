//! Extrusion: the solid swept by profiles along the normal of their plane.
//!
//! Per profile one shell: a planar cap at each end, a planar side face per
//! line, a cylindrical one per arc or circle and a B-spline one (the
//! spline's degree by 1, polynomial) per spline. Per loop of a profile, at
//! each end, one vertex where two segments meet and one edge per segment;
//! between the ends one straight lateral edge per vertex. A full circle has
//! one vertex at each end (at its angle 0) and no lateral edge: its wall is
//! bounded by the two circles alone, like `primitives::cylinder`.
//!
//! A line's edges run the way the loop does. The edge of an arc or circle
//! always runs counter-clockwise about the plane normal, as its curve does,
//! and a loop that goes clockwise uses it reversed. The edge of a spline
//! always runs from the spline's first control point to its last, and a
//! loop that goes the other way uses it reversed. So the body depends on
//! the profile only, not on the direction its lines were drawn in.
//!
//! Every entity is reported with its origin (see [`Origin`]), from which
//! persistent names are made (docs/architecture.md §4.1).

use crate::profile::{Loop2, Profile, Segment, Shape};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::{
    Circle, Curve3, Cylinder, GeomError, Line, NurbsCurve, NurbsSurface, Plane, Surface,
};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, LoopId, VertexId, check};
use std::f64::consts::TAU;

/// What an entity of an extrusion is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// The cap at the `from` offset. Sources: the curve ids of the
    /// profile's outer loop, ascending.
    StartCap,
    /// The cap at the `to` offset. Sources as for `StartCap`.
    EndCap,
    /// The face swept by a curve. Sources: that curve.
    Side,
    /// The edge of the start cap made from a curve. Sources: that curve.
    StartEdge,
    /// The edge of the end cap made from a curve. Sources: that curve.
    EndEdge,
    /// The straight edge swept by a profile vertex. Sources: the curve the
    /// loop arrives by and the curve it leaves by, in that order.
    LateralEdge,
    /// A vertex of the start cap. Sources as for `LateralEdge`; for the one
    /// vertex of a full circle, that circle twice.
    StartVertex,
    /// A vertex of the end cap. Sources as for `StartVertex`.
    EndVertex,
}

/// Where an entity came from: unique within the body, and the same whenever
/// the same curves bound the same profile.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Origin {
    pub role: Role,
    pub sources: Vec<u64>,
}

/// Where every entity of the body came from: `faces[i]` belongs to face
/// `FaceId(i)`, and so on.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Origins {
    pub faces: Vec<Origin>,
    pub edges: Vec<Origin>,
    pub vertices: Vec<Origin>,
}

/// A body and the origin of each of its entities.
#[derive(Clone, Debug, PartialEq)]
pub struct OpResult {
    pub body: Body,
    pub origins: Origins,
}

fn invalid(reason: &'static str) -> KernelError {
    KernelError::InvalidProfile { reason }
}

fn degenerate_frame() -> KernelError {
    KernelError::Geometry(GeomError::InvalidParameter(
        "the plane or the profile puts a point at a non-finite position",
    ))
}

/// The body under construction, with an origin for everything added.
struct Builder<'a> {
    body: Body,
    origins: Origins,
    plane: &'a Frame,
}

impl Builder<'_> {
    fn vertex(&mut self, p: [f64; 2], z: f64, role: Role, sources: [u64; 2]) -> VertexId {
        self.origins.vertices.push(Origin {
            role,
            sources: sources.to_vec(),
        });
        self.body.add_vertex(self.plane.point(p[0], p[1], z))
    }

    fn edge(
        &mut self,
        curve: Curve3,
        range: (f64, f64),
        ends: (VertexId, VertexId),
        role: Role,
        sources: &[u64],
    ) -> EdgeId {
        self.origins.edges.push(Origin {
            role,
            sources: sources.to_vec(),
        });
        self.body.add_edge(curve, range, ends.0, ends.1)
    }

    fn face(
        &mut self,
        surface: Surface,
        reversed: bool,
        loops: Vec<LoopId>,
        role: Role,
        sources: Vec<u64>,
    ) -> FaceId {
        self.origins.faces.push(Origin { role, sources });
        self.body.add_face(surface, reversed, loops)
    }

    /// The plane's axes at sketch position `p`, offset `z`.
    fn frame_at(&self, p: [f64; 2], z: f64) -> Result<Frame, KernelError> {
        self.plane
            .at(self.plane.point(p[0], p[1], z))
            .ok_or_else(degenerate_frame)
    }

    /// The edge of `segment` at offset `z`, between the vertices at its
    /// start and end as the loop runs, and whether the loop runs against it.
    fn layer_edge(
        &mut self,
        segment: &Segment,
        z: f64,
        ends: (VertexId, VertexId),
        role: Role,
    ) -> Result<(EdgeId, bool), KernelError> {
        let sources = [segment.source];
        match &segment.shape {
            Shape::Spline(spline) => {
                let control = spline
                    .control
                    .iter()
                    .map(|p| self.plane.point(p[0], p[1], z))
                    .collect();
                let curve = NurbsCurve::polynomial(spline.degree, spline.knots.clone(), control)?;
                let range = curve.domain();
                // The curve runs the spline's own way: from the loop's end
                // vertex to its start vertex when the loop goes against it.
                let own = if segment.reversed {
                    (ends.1, ends.0)
                } else {
                    ends
                };
                Ok((
                    self.edge(Curve3::Nurbs(curve), range, own, role, &sources),
                    segment.reversed,
                ))
            }
            Shape::Line => {
                let start = self.plane.point(segment.start[0], segment.start[1], z);
                let end = self.plane.point(segment.end[0], segment.end[1], z);
                let line = Curve3::Line(Line::through(start, end)?);
                let range = (0.0, start.distance(end));
                Ok((self.edge(line, range, ends, role, &sources), false))
            }
            &Shape::Arc { center, radius } => {
                let (begin, sweep) = segment.angles().ok_or(invalid("an arc without angles"))?;
                if !(sweep > 0.0 && sweep < TAU) {
                    return Err(invalid(
                        "an arc whose start and end coincide; a full turn is a circle",
                    ));
                }
                let circle = Curve3::Circle(Circle::new(self.frame_at(center, z)?, radius)?);
                let range = (begin, begin + sweep);
                // The circle runs counter-clockwise: from the loop's end
                // vertex to its start vertex when the loop goes clockwise.
                let own = if segment.reversed {
                    (ends.1, ends.0)
                } else {
                    ends
                };
                Ok((
                    self.edge(circle, range, own, role, &sources),
                    segment.reversed,
                ))
            }
            &Shape::Circle { center, radius } => {
                let circle = Curve3::Circle(Circle::new(self.frame_at(center, z)?, radius)?);
                Ok((
                    self.edge(circle, (0.0, TAU), ends, role, &sources),
                    segment.reversed,
                ))
            }
        }
    }

    /// Sweeps one loop from offset `from` to `to`: adds its vertices, edges
    /// and side faces, and returns the side faces with the coedges of the
    /// loop at the start and at the end, both in the direction of the loop.
    fn sweep(&mut self, path: &Loop2, from: f64, to: f64) -> Result<Swept, KernelError> {
        let segments = &path.segments;
        let n = segments.len();
        let whole_circle = match (segments.first(), n) {
            (None, _) => return Err(invalid("a loop without segments")),
            (Some(first), 1) => {
                if !matches!(first.shape, Shape::Circle { .. }) {
                    return Err(invalid(
                        "a loop of one segment that is not a circle is not closed",
                    ));
                }
                true
            }
            _ => {
                if segments
                    .iter()
                    .any(|s| matches!(s.shape, Shape::Circle { .. }))
                {
                    return Err(invalid("a circle is a loop on its own"));
                }
                false
            }
        };
        // Vertex i is where segment i starts: the loop arrives by the
        // segment before it and leaves by segment i.
        let before = segments.iter().cycle().skip(n - 1);
        let names: Vec<[u64; 2]> = before
            .zip(segments)
            .map(|(arriving, leaving)| [arriving.source, leaving.source])
            .collect();
        let mut layers = Vec::with_capacity(2);
        for (z, vertex_role, edge_role) in [
            (from, Role::StartVertex, Role::StartEdge),
            (to, Role::EndVertex, Role::EndEdge),
        ] {
            let vertices: Vec<VertexId> = segments
                .iter()
                .zip(&names)
                .map(|(segment, name)| self.vertex(segment.start, z, vertex_role, *name))
                .collect();
            let after = vertices.iter().cycle().skip(1);
            let mut edges = Vec::with_capacity(n);
            for (segment, (&here, &there)) in segments.iter().zip(vertices.iter().zip(after)) {
                edges.push(self.layer_edge(segment, z, (here, there), edge_role)?);
            }
            layers.push((vertices, edges));
        }
        let (Some((top_vertices, top)), Some((bottom_vertices, bottom))) =
            (layers.pop(), layers.pop())
        else {
            return Err(invalid("a loop lost its layers"));
        };

        let mut laterals = Vec::with_capacity(n);
        if !whole_circle {
            for ((&low, &high), name) in bottom_vertices.iter().zip(&top_vertices).zip(&names) {
                let (Some(a), Some(b)) = (self.body.vertex(low), self.body.vertex(high)) else {
                    return Err(invalid("a loop lost a vertex"));
                };
                let (a, b) = (a.point, b.point);
                let line = Curve3::Line(Line::through(a, b)?);
                let range = (0.0, a.distance(b));
                laterals.push(self.edge(line, range, (low, high), Role::LateralEdge, name));
            }
        }

        let coedge = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
        let mut faces = Vec::with_capacity(n);
        let next_lateral = laterals.iter().cycle().skip(1);
        let mut uprights = laterals.iter().zip(next_lateral);
        for (segment, (&(low, low_against), &(high, high_against))) in
            segments.iter().zip(bottom.iter().zip(&top))
        {
            // Seen from outside, which is to the right of the loop: along
            // the bottom, up, back along the top, down.
            let loops = match uprights.next() {
                Some((&here, &there)) => vec![self.body.add_loop(vec![
                    coedge(low, low_against),
                    coedge(there, false),
                    coedge(high, !high_against),
                    coedge(here, true),
                ])],
                None => vec![
                    self.body.add_loop(vec![coedge(low, low_against)]),
                    self.body.add_loop(vec![coedge(high, !high_against)]),
                ],
            };
            let (surface, reversed) = match &segment.shape {
                Shape::Spline(spline) => {
                    // The spline at the start offset and at the end offset,
                    // joined by straight lines: u along the spline, v from
                    // 0 at the start to 1 at the end. The normal is the
                    // spline's direction crossed with the plane's z, which
                    // is to the right of the spline: outside when the loop
                    // runs the spline's way, as an outer loop
                    // (counter-clockwise) has the outside on its right.
                    let mut control = Vec::with_capacity(spline.control.len() * 2);
                    for p in &spline.control {
                        control.push(self.plane.point(p[0], p[1], from));
                        control.push(self.plane.point(p[0], p[1], to));
                    }
                    let weights = vec![1.0; control.len()];
                    let wall = NurbsSurface::new(
                        spline.degree,
                        1,
                        spline.knots.clone(),
                        vec![0.0, 0.0, 1.0, 1.0],
                        spline.control.len(),
                        2,
                        control,
                        weights,
                    )?;
                    (Surface::Nurbs(wall), segment.reversed)
                }
                Shape::Line => {
                    let start = self.plane.point(segment.start[0], segment.start[1], from);
                    let along = self.plane.point(segment.end[0], segment.end[1], from) - start;
                    let wall = Frame::new(start, along.cross(self.plane.z()), along)
                        .ok_or(invalid("a line without length"))?;
                    (Surface::Plane(Plane::new(wall)), false)
                }
                &Shape::Arc { center, radius } | &Shape::Circle { center, radius } => {
                    let axis = self.frame_at(center, from)?;
                    // A cylinder's normal points away from its axis; going
                    // clockwise the outside is towards the axis.
                    (
                        Surface::Cylinder(Cylinder::new(axis, radius)?),
                        segment.reversed,
                    )
                }
            };
            faces.push(self.face(surface, reversed, loops, Role::Side, vec![segment.source]));
        }
        Ok(Swept {
            faces,
            start: bottom
                .iter()
                .map(|&(e, against)| coedge(e, against))
                .collect(),
            end: top.iter().map(|&(e, against)| coedge(e, against)).collect(),
        })
    }
}

struct Swept {
    faces: Vec<FaceId>,
    start: Vec<Coedge>,
    end: Vec<Coedge>,
}

/// Extrudes the profiles along the normal (z) of `plane`, from offset
/// `from` to offset `to` (`from < to`, either may be negative). `plane`
/// maps sketch (x, y) to space with its x and y axes.
///
/// The body is a solid with one shell per profile and has passed
/// `craft3d_topo::check` with `tolerance`. Entities are added in a fixed
/// order: per profile, per loop (the outer one first) its start vertices,
/// start edges, end vertices, end edges, lateral edges and side faces, then
/// the profile's start cap and end cap.
///
/// The profiles are trusted not to overlap each other, as those of
/// [`crate::profile::detect_profiles`] do not; curve ids must be unique
/// across them.
pub fn extrude(
    profiles: &[Profile],
    plane: &Frame,
    from: f64,
    to: f64,
    tolerance: &Tolerance,
) -> Result<OpResult, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if !(from.is_finite() && to.is_finite() && to - from > tol) {
        return Err(KernelError::InvalidOffsets { from, to });
    }
    if profiles.is_empty() {
        return Err(KernelError::NoProfile);
    }
    // Origins are only unique when every curve is used once.
    let mut ids: Vec<u64> = profiles
        .iter()
        .flat_map(|p| std::iter::once(&p.outer).chain(&p.holes))
        .flat_map(|l| l.segments.iter().map(|s| s.source))
        .collect();
    ids.sort_unstable();
    if ids.iter().zip(ids.iter().skip(1)).any(|(a, b)| a == b) {
        return Err(invalid("a curve id is used by more than one segment"));
    }

    let mut b = Builder {
        body: Body::new(BodyKind::Solid),
        origins: Origins::default(),
        plane,
    };
    let floor = plane
        .reversed_at(plane.point(0.0, 0.0, from))
        .ok_or_else(degenerate_frame)?;
    let lid = plane
        .at(plane.point(0.0, 0.0, to))
        .ok_or_else(degenerate_frame)?;
    for profile in profiles {
        let mut faces = Vec::new();
        let mut floor_loops = Vec::new();
        let mut lid_loops = Vec::new();
        for path in std::iter::once(&profile.outer).chain(&profile.holes) {
            let swept = b.sweep(path, from, to)?;
            faces.extend(swept.faces);
            // The start cap looks along −z: seen from outside its loops run
            // the other way round.
            let backwards = swept.start.iter().rev().map(|c| Coedge {
                edge: c.edge,
                reversed: !c.reversed,
            });
            floor_loops.push(b.body.add_loop(backwards.collect()));
            lid_loops.push(b.body.add_loop(swept.end));
        }
        let mut sources: Vec<u64> = profile.outer.segments.iter().map(|s| s.source).collect();
        sources.sort_unstable();
        faces.push(b.face(
            Surface::Plane(Plane::new(floor)),
            false,
            floor_loops,
            Role::StartCap,
            sources.clone(),
        ));
        faces.push(b.face(
            Surface::Plane(Plane::new(lid)),
            false,
            lid_loops,
            Role::EndCap,
            sources,
        ));
        b.body.add_shell(faces);
    }

    let mut defects = check(&b.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "extrude",
            defects,
        });
    }
    Ok(OpResult {
        body: b.body,
        origins: b.origins,
    })
}
