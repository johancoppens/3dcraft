//! Revolution: the solid swept by profiles turning about an axis that lies
//! in their plane.
//!
//! Each curve sweeps the surface its shape and its position relative to the
//! axis give:
//!
//! | Curve | Surface |
//! |---|---|
//! | line on the axis | none: its ends are points of the axis |
//! | line perpendicular to the axis | plane (a disc or an annulus) |
//! | line parallel to the axis | cylinder |
//! | any other line | cone, through its apex when the line ends on the axis |
//! | arc centred on the axis | sphere, through a pole when the arc ends on the axis |
//! | arc or circle whose whole circle stays clear of the axis | torus |
//!
//! An arc or circle whose own circle reaches the axis without being
//! centred on it would sweep a spindle torus, which `craft3d-geom` does not
//! have: an error that names the curve. A profile with points on both
//! sides of the axis is an error too; touching the axis is fine.
//!
//! A partial revolution is built like an extrusion bent around the axis:
//! per profile one shell with the profile as a planar cap at each end, per
//! curve a side face between its edge on the start cap and its edge on the
//! end cap, and per profile vertex an arc about the axis between the two.
//! What lies on the axis does not move: a vertex there is one vertex for
//! both caps and sweeps no arc, and a line there is one edge that the two
//! caps share. A whole circle has one vertex on each cap and its face is
//! bounded by the two circles alone.
//!
//! A full revolution has no caps and no profile edges: per profile vertex
//! off the axis one closed circle with one vertex (in the sketch plane, on
//! the profile), and per curve a face bounded by the circles of its ends,
//! like the wall of `primitives::cylinder`; a curve that ends on the axis
//! has no loop there, and nothing is made at the axis at all. A whole
//! circle sweeps a torus without edges, a half circle on the axis a sphere
//! without edges, as the primitives are.
//!
//! All surfaces share one frame about the axis whose angle 0 is where the
//! revolution starts, so no face crosses the seam of its surface around
//! the axis. Edges run as in `extrude`: a line's the way the loop does, an
//! arc's counter-clockwise about the (turned) sketch normal, the circles
//! about the axis counter-clockwise about its direction.
//!
//! Every entity is reported with its origin, in the roles of
//! [`crate::extrude::Role`] (see [`revolve`] for the two cases an
//! extrusion does not have).

use crate::extrude::{OpResult, Origin, Origins, Role};
use crate::profile::{Loop2, Profile, Segment, Shape};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::{
    Circle, Cone, Curve3, Cylinder, GeomError, Line, Plane, Sphere, Surface, Torus,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, LoopId, VertexId, check};
use std::f64::consts::TAU;

type P2 = [f64; 2];

fn invalid(reason: &'static str) -> KernelError {
    KernelError::InvalidProfile { reason }
}

fn degenerate_frame() -> KernelError {
    KernelError::Geometry(GeomError::InvalidParameter(
        "the plane, the axis or the profile puts a point at a non-finite position",
    ))
}

fn coedge(edge: EdgeId, reversed: bool) -> Coedge {
    Coedge { edge, reversed }
}

/// An angle wrapped to `[0, 2π)`.
fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

/// The axis in the sketch: a point on it, its unit direction, and the unit
/// normal to its left.
struct Axis {
    point: P2,
    direction: P2,
    left: P2,
}

impl Axis {
    /// How far `p` is to the left of the axis (negative: to the right), and
    /// how far along it.
    fn coordinates(&self, p: P2) -> (f64, f64) {
        let v = [p[0] - self.point[0], p[1] - self.point[1]];
        (
            v[0] * self.left[0] + v[1] * self.left[1],
            v[0] * self.direction[0] + v[1] * self.direction[1],
        )
    }

    /// The least and greatest distance to the left of the axis that
    /// `segment` reaches.
    fn reach(&self, segment: &Segment) -> (f64, f64) {
        let (a, _) = self.coordinates(segment.start);
        let (b, _) = self.coordinates(segment.end);
        let (mut low, mut high) = (a.min(b), a.max(b));
        if let Shape::Arc { center, radius } | Shape::Circle { center, radius } = segment.shape {
            let (c, _) = self.coordinates(center);
            let (begin, sweep) = segment.angles().unwrap_or((0.0, TAU));
            // Furthest left where the circle's radius points along `left`.
            let leftmost = self.left[1].atan2(self.left[0]);
            if wrap(leftmost - begin) <= sweep {
                high = high.max(c + radius);
            }
            if wrap(leftmost + std::f64::consts::PI - begin) <= sweep {
                low = low.min(c - radius);
            }
        }
        (low, high)
    }
}

/// A point of the half-plane that turns about the axis: its distance from
/// the axis and its height along it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rz {
    rho: f64,
    z: f64,
}

/// A segment in the turning half-plane, with the face it sweeps.
struct Part<'a> {
    segment: &'a Segment,
    start: Rz,
    end: Rz,
    /// The centre of an arc or circle.
    centre: Rz,
    /// The surface and whether the outside is against its normal; `None`
    /// for a line on the axis.
    face: Option<(Surface, bool)>,
}

/// The body under construction, with an origin for everything added.
struct Builder {
    body: Body,
    origins: Origins,
    /// About the axis: z along it, x towards the profile where the
    /// revolution starts.
    frame: Frame,
    /// The angle turned through.
    sweep: f64,
    full: bool,
    /// +1 when the profile is to the left of the axis, −1 to the right.
    side: f64,
    /// The sketch's x axis in the turning half-plane: its components away
    /// from the axis and along it.
    sketch_x: (f64, f64),
    linear: f64,
    angular: f64,
}

impl Builder {
    fn radial(&self, angle: f64) -> Vec3 {
        self.frame.vector(angle.cos(), angle.sin(), 0.0)
    }

    /// The direction a point at `angle` moves in as it turns.
    fn onward(&self, angle: f64) -> Vec3 {
        self.frame.vector(-angle.sin(), angle.cos(), 0.0)
    }

    fn point(&self, p: Rz, angle: f64) -> Vec3 {
        self.frame
            .point(p.rho * angle.cos(), p.rho * angle.sin(), p.z)
    }

    /// The frame's axes at height `z` on the axis.
    fn level(&self, z: f64) -> Result<Frame, KernelError> {
        self.frame
            .at(self.frame.point(0.0, 0.0, z))
            .ok_or_else(degenerate_frame)
    }

    fn vertex(&mut self, point: Vec3, role: Role, sources: [u64; 2]) -> VertexId {
        self.origins.vertices.push(Origin {
            role,
            sources: sources.to_vec(),
        });
        self.body.add_vertex(point)
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

    /// A sketch point in the turning half-plane. What is within tolerance
    /// of the axis is on it.
    fn place(&self, axis: &Axis, p: P2) -> Rz {
        let (left, along) = axis.coordinates(p);
        let rho = self.side * left;
        Rz {
            rho: if rho <= self.linear { 0.0 } else { rho },
            z: along,
        }
    }

    /// The segment in the half-plane and the surface it sweeps.
    /// For an arc about a point of the axis: the pole of its sphere
    /// nearest to height `z`.
    fn pole(&self, axis: &Axis, segment: &Segment, z: f64) -> Option<f64> {
        let Shape::Arc { center, radius } = segment.shape else {
            return None;
        };
        let (left, along) = axis.coordinates(center);
        let pole = if z >= along {
            along + radius
        } else {
            along - radius
        };
        ((self.side * left).abs() <= self.linear).then_some(pole)
    }

    /// The segment between `start` and `end` in the half-plane, and the
    /// surface it sweeps.
    fn part<'a>(
        &self,
        axis: &Axis,
        segment: &'a Segment,
        start: Rz,
        end: Rz,
    ) -> Result<Part<'a>, KernelError> {
        let unsupported = |reason: &'static str| KernelError::UnsupportedCurve {
            id: segment.source,
            reason,
        };
        let mut centre = start;
        let face = match segment.shape {
            Shape::Line => {
                let (across, along) = (end.rho - start.rho, end.z - start.z);
                let length = across.hypot(along);
                // Close enough to perpendicular or parallel that both ends
                // are on the plane or cylinder within the tolerance.
                let slack = (self.angular * length).min(self.linear);
                // Seen along the loop the outside is to the right, which in
                // the half-plane is (−along, across) for a profile left of
                // the axis and the opposite to the right of it.
                if start.rho == 0.0 && end.rho == 0.0 {
                    None
                } else if along.abs() <= slack {
                    let up = self.side * across > 0.0;
                    let level = self.level(start.z)?;
                    let frame = if up {
                        level
                    } else {
                        self.frame
                            .reversed_at(level.origin())
                            .ok_or_else(degenerate_frame)?
                    };
                    Some((Surface::Plane(Plane::new(frame)), false))
                } else {
                    let inward = self.side * along > 0.0;
                    if across.abs() <= slack {
                        let radius = 0.5 * (start.rho + end.rho);
                        let cylinder = Cylinder::new(self.level(start.z)?, radius)?;
                        Some((Surface::Cylinder(cylinder), inward))
                    } else {
                        // From the end on the axis, when there is one: the
                        // apex is then exactly the vertex.
                        let from = if end.rho == 0.0 { end } else { start };
                        let cone =
                            Cone::new(self.level(from.z)?, from.rho, (across / along).atan())?;
                        Some((Surface::Cone(cone), inward))
                    }
                }
            }
            Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
                let (left, along) = axis.coordinates(center);
                let rho = self.side * left;
                if rho.abs() <= self.linear {
                    centre = Rz { rho: 0.0, z: along };
                    let sphere = Sphere::new(self.level(along)?, radius)?;
                    Some((Surface::Sphere(sphere), segment.reversed))
                } else if rho - radius > self.linear {
                    centre = Rz { rho, z: along };
                    let torus = Torus::new(self.level(along)?, rho, radius)?;
                    Some((Surface::Torus(torus), segment.reversed))
                } else {
                    return Err(unsupported(
                        "its circle reaches the axis without being centred on it, which \
                         sweeps a spindle torus; centre it on the axis or keep the whole \
                         circle clear of it",
                    ));
                }
            }
        };
        Ok(Part {
            segment,
            start,
            end,
            centre,
            face,
        })
    }

    /// The edge of `part` in the half-plane at `angle`, between the
    /// vertices at its start and end as the loop runs, and whether the loop
    /// runs against it.
    fn profile_edge(
        &mut self,
        part: &Part,
        angle: f64,
        ends: (VertexId, VertexId),
        role: Role,
    ) -> Result<(EdgeId, bool), KernelError> {
        let segment = part.segment;
        let sources = [segment.source];
        // The sketch plane turned to `angle`: its normal and its x axis.
        let turned = |b: &Builder, radius: f64, centre: Rz| -> Result<Curve3, KernelError> {
            let normal = b.onward(angle) * b.side;
            let x = b.radial(angle) * b.sketch_x.0 + b.frame.z() * b.sketch_x.1;
            let frame =
                Frame::new(b.point(centre, angle), normal, x).ok_or_else(degenerate_frame)?;
            Ok(Curve3::Circle(Circle::new(frame, radius)?))
        };
        match segment.shape {
            Shape::Line => {
                let (start, end) = (self.point(part.start, angle), self.point(part.end, angle));
                let line = Curve3::Line(Line::through(start, end)?);
                let range = (0.0, start.distance(end));
                Ok((self.edge(line, range, ends, role, &sources), false))
            }
            Shape::Arc { radius, .. } => {
                // The angles in the sketch that its ends are at, from where
                // the vertices are: that is where the sketch has them,
                // except that what is within tolerance of the axis is on
                // it, and the edge must end at its vertex all the same.
                let angle = |p: Rz| {
                    let (across, along) = (p.rho - part.centre.rho, p.z - part.centre.z);
                    let (x_across, x_along) = self.sketch_x;
                    (self.side * (x_along * across - x_across * along))
                        .atan2(x_across * across + x_along * along)
                };
                let (first, last) = if segment.reversed {
                    (part.end, part.start)
                } else {
                    (part.start, part.end)
                };
                let begin = angle(first);
                let sweep = wrap(angle(last) - begin);
                if !(sweep > 0.0 && sweep < TAU) {
                    return Err(invalid(
                        "an arc whose start and end coincide; a full turn is a circle",
                    ));
                }
                let circle = turned(self, radius, part.centre)?;
                // The circle runs counter-clockwise: from the loop's end
                // vertex to its start vertex when the loop goes clockwise.
                let own = if segment.reversed {
                    (ends.1, ends.0)
                } else {
                    ends
                };
                let range = (begin, begin + sweep);
                Ok((
                    self.edge(circle, range, own, role, &sources),
                    segment.reversed,
                ))
            }
            Shape::Circle { radius, .. } => {
                let circle = turned(self, radius, part.centre)?;
                Ok((
                    self.edge(circle, (0.0, TAU), ends, role, &sources),
                    segment.reversed,
                ))
            }
        }
    }

    /// Sweeps one loop: adds its vertices, edges and side faces.
    fn sweep_loop(&mut self, axis: &Axis, path: &Loop2) -> Result<Swept, KernelError> {
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
        for (segment, next) in segments.iter().zip(segments.iter().cycle().skip(1)) {
            let gap = (segment.end[0] - next.start[0]).hypot(segment.end[1] - next.start[1]);
            if gap > self.linear || gap.is_nan() {
                return Err(invalid(
                    "a loop that is not closed: a segment does not start where the one before it ends",
                ));
            }
        }
        // Where each segment starts. Where an arc about a point of the
        // axis ends on the axis, that is the pole of its sphere, exactly:
        // the two meridians a partial turn makes of the arc can only meet
        // there, however loosely the sketch put the arc's end on the axis.
        let mut corners: Vec<Rz> = segments.iter().map(|s| self.place(axis, s.start)).collect();
        let before = segments.iter().cycle().skip(n - 1);
        for ((corner, arriving), leaving) in corners.iter_mut().zip(before).zip(segments) {
            if corner.rho == 0.0
                && let Some(z) = [leaving, arriving]
                    .into_iter()
                    .find_map(|s| self.pole(axis, s, corner.z))
            {
                corner.z = z;
            }
        }
        let mut parts = Vec::with_capacity(n);
        let after = corners.iter().cycle().skip(1);
        for (segment, (&start, &end)) in segments.iter().zip(corners.iter().zip(after)) {
            parts.push(self.part(axis, segment, start, end)?);
        }
        // Vertex i is where segment i starts: the loop arrives by the
        // segment before it and leaves by segment i.
        let before = segments.iter().cycle().skip(n - 1);
        let names: Vec<[u64; 2]> = before
            .zip(segments)
            .map(|(arriving, leaving)| [arriving.source, leaving.source])
            .collect();
        let (full, sweep) = (self.full, self.sweep);

        // Vertices: at the start, then at the end. One on the axis is the
        // same at both; a full revolution has only those at the start, and
        // none where no edge would use them: on the axis, and on a whole
        // circle.
        let mut low: Vec<Option<VertexId>> = Vec::with_capacity(n);
        for (part, name) in parts.iter().zip(&names) {
            let wanted = if full {
                part.start.rho > 0.0 && !whole_circle
            } else {
                true
            };
            low.push(wanted.then(|| {
                let point = self.point(part.start, 0.0);
                self.vertex(point, Role::StartVertex, *name)
            }));
        }
        let mut high: Vec<Option<VertexId>> = Vec::with_capacity(n);
        for ((part, name), &same) in parts.iter().zip(&names).zip(&low) {
            high.push(if full || part.start.rho == 0.0 {
                same
            } else {
                let point = self.point(part.start, sweep);
                Some(self.vertex(point, Role::EndVertex, *name))
            });
        }

        // The profile's edges at the start and at the end. A line on the
        // axis is one edge for both.
        let mut bottom: Vec<Option<(EdgeId, bool)>> = vec![None; n];
        let mut top: Vec<Option<(EdgeId, bool)>> = vec![None; n];
        if !full {
            for (layer, angle, role) in [(0, 0.0, Role::StartEdge), (1, sweep, Role::EndEdge)] {
                let vertices = if layer == 0 { &low } else { &high };
                let after = vertices.iter().cycle().skip(1);
                let mut made = Vec::with_capacity(n);
                for (i, (part, (&here, &there))) in
                    parts.iter().zip(vertices.iter().zip(after)).enumerate()
                {
                    let shared = bottom.get(i).copied().flatten();
                    made.push(match (part.face.is_none(), shared, layer) {
                        (true, Some(edge), 1) => Some(edge),
                        _ => {
                            let (Some(here), Some(there)) = (here, there) else {
                                return Err(invalid("a loop lost a vertex"));
                            };
                            Some(self.profile_edge(part, angle, (here, there), role)?)
                        }
                    });
                }
                if layer == 0 {
                    bottom = made;
                } else {
                    top = made;
                }
            }
        }

        // The circle or arc about the axis that each vertex off it sweeps.
        let mut arcs: Vec<Option<EdgeId>> = Vec::with_capacity(n);
        for (((part, name), &from), &to) in parts.iter().zip(&names).zip(&low).zip(&high) {
            arcs.push(if part.start.rho > 0.0 && !whole_circle {
                let (Some(from), Some(to)) = (from, to) else {
                    return Err(invalid("a loop lost a vertex"));
                };
                let circle =
                    Curve3::Circle(Circle::new(self.level(part.start.z)?, part.start.rho)?);
                let range = (0.0, if full { TAU } else { sweep });
                Some(self.edge(circle, range, (from, to), Role::LateralEdge, name))
            } else {
                None
            });
        }

        // Seen from outside, which is to the right of the loop, a profile
        // left of the axis turns away from the viewer's left: the face runs
        // along its start edge, up the arc of its end vertex, back along
        // its end edge and down the arc of its start vertex. To the right
        // of the axis it is the mirror image.
        let along = self.side > 0.0;
        let mut groups: Vec<Vec<FaceId>> = Vec::new();
        let after = arcs.iter().cycle().skip(1);
        for (i, (part, (&here, &there))) in parts.iter().zip(arcs.iter().zip(after)).enumerate() {
            let Some((surface, reversed)) = part.face.clone() else {
                continue;
            };
            let mut loops = Vec::new();
            if full {
                // Left of the circle at its start when the profile is to
                // the right of the axis, and the other way round.
                for (arc, against) in [(here, along), (there, !along)] {
                    if let Some(arc) = arc {
                        loops.push(self.body.add_loop(vec![coedge(arc, against)]));
                    }
                }
            } else {
                let (Some(&Some((first, first_against))), Some(&Some((last, last_against)))) =
                    (bottom.get(i), top.get(i))
                else {
                    return Err(invalid("a loop lost an edge"));
                };
                if whole_circle {
                    loops.push(
                        self.body
                            .add_loop(vec![coedge(first, first_against != !along)]),
                    );
                    loops.push(
                        self.body
                            .add_loop(vec![coedge(last, last_against != along)]),
                    );
                } else {
                    let mut ring = Vec::with_capacity(4);
                    if along {
                        ring.push(coedge(first, first_against));
                        ring.extend(there.map(|arc| coedge(arc, false)));
                        ring.push(coedge(last, !last_against));
                        ring.extend(here.map(|arc| coedge(arc, true)));
                    } else {
                        ring.push(coedge(first, !first_against));
                        ring.extend(here.map(|arc| coedge(arc, false)));
                        ring.push(coedge(last, last_against));
                        ring.extend(there.map(|arc| coedge(arc, true)));
                    }
                    loops.push(self.body.add_loop(ring));
                }
            }
            let face = self.face(
                surface,
                reversed,
                loops,
                Role::Side,
                vec![part.segment.source],
            );
            // Faces either side of a vertex on the axis share no edge.
            match groups.last_mut() {
                Some(group) if part.start.rho > 0.0 => group.push(face),
                _ => groups.push(vec![face]),
            }
        }
        // The loop is closed: its last faces join its first across vertex 0.
        if groups.len() > 1
            && parts.first().is_some_and(|p| p.start.rho > 0.0)
            && let Some(last) = groups.pop()
            && let Some(first) = groups.first_mut()
        {
            first.extend(last);
        }
        let in_loop_order = |edges: &[Option<(EdgeId, bool)>]| {
            edges
                .iter()
                .flatten()
                .map(|&(edge, against)| coedge(edge, against))
                .collect()
        };
        Ok(Swept {
            groups,
            start: in_loop_order(&bottom),
            end: in_loop_order(&top),
        })
    }
}

struct Swept {
    /// The side faces, grouped by what is connected through shared edges.
    groups: Vec<Vec<FaceId>>,
    /// The loop's coedges at the start and at the end, in its direction.
    start: Vec<Coedge>,
    end: Vec<Coedge>,
}

/// A loop run the other way.
fn backwards(coedges: &[Coedge]) -> Vec<Coedge> {
    coedges
        .iter()
        .rev()
        .map(|c| coedge(c.edge, !c.reversed))
        .collect()
}

/// Revolves the profiles about an axis that lies in the sketch plane.
///
/// `axis_point` and `axis_direction` are in sketch (x, y) coordinates of
/// `plane`; angles in radians, counter-clockwise about the axis direction
/// (right-hand rule), `from < to`, `to - from <= TAU`. A sweep of `TAU`
/// within the angular tolerance is a full revolution: no caps, and `from`
/// does not matter (the body is the same for any).
///
/// The body is a solid that has passed `craft3d_topo::check` with
/// `tolerance`: one shell per profile for a partial revolution; for a full
/// one, one shell per closed surface (a hole in a profile becomes a cavity
/// with a shell of its own, and so does a part of a loop between two
/// places where it touches the axis). Entities are added in a fixed order:
/// per profile, per loop (the outer one first) its start vertices, end
/// vertices, start edges, end edges, the arcs about the axis and the side
/// faces, then the profile's start cap and end cap.
///
/// Origins use the roles of an extrusion with the same meaning: `StartCap`
/// and `EndCap` are the profile at `from` and at `to`, `Side` the face a
/// curve sweeps, `StartEdge` and `EndEdge` a curve's edge on either cap,
/// `LateralEdge` the arc or circle about the axis that a profile vertex
/// sweeps, `StartVertex` and `EndVertex` its ends. Where the start and the
/// end are one and the same entity it is reported as the start one, and
/// there is no end one: the one vertex of the circle a vertex sweeps in a
/// full revolution, a vertex on the axis, and the edge of a line on the
/// axis.
///
/// The profiles are trusted not to overlap each other, as those of
/// [`crate::profile::detect_profiles`] do not, and must all be on the same
/// side of the axis; curve ids must be unique across them.
pub fn revolve(
    profiles: &[Profile],
    plane: &Frame,
    axis_point: [f64; 2],
    axis_direction: [f64; 2],
    from: f64,
    to: f64,
    tolerance: &Tolerance,
) -> Result<OpResult, KernelError> {
    let (linear, angular) = (tolerance.linear, tolerance.angular);
    for (name, value) in [("tolerance", linear), ("angular tolerance", angular)] {
        if !(value.is_finite() && value > 0.0) {
            return Err(KernelError::InvalidSize { name, value });
        }
    }
    if !axis_point.iter().all(|v| v.is_finite()) {
        return Err(KernelError::InvalidAxis {
            reason: "its point is not finite",
        });
    }
    let length = axis_direction[0].hypot(axis_direction[1]);
    let direction = [axis_direction[0] / length, axis_direction[1] / length];
    if !(length.is_finite() && length > 0.0 && direction.iter().all(|v| v.is_finite())) {
        return Err(KernelError::InvalidAxis {
            reason: "its direction has no length or is not finite",
        });
    }
    let sweep = to - from;
    if !(from.is_finite() && to.is_finite() && sweep > angular && sweep <= TAU + angular) {
        return Err(KernelError::InvalidAngles { from, to });
    }
    let full = sweep >= TAU - angular;
    if profiles.is_empty() {
        return Err(KernelError::NoProfile);
    }
    // Origins are only unique when every curve is used once.
    let all_segments = || {
        profiles
            .iter()
            .flat_map(|p| std::iter::once(&p.outer).chain(&p.holes))
            .flat_map(|l| l.segments.iter())
    };
    let mut ids: Vec<u64> = all_segments().map(|s| s.source).collect();
    ids.sort_unstable();
    if ids.iter().zip(ids.iter().skip(1)).any(|(a, b)| a == b) {
        return Err(invalid("a curve id is used by more than one segment"));
    }

    let axis = Axis {
        point: axis_point,
        direction,
        left: [-direction[1], direction[0]],
    };
    // Which side of the axis the profiles are on.
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for segment in all_segments() {
        let (a, b) = axis.reach(segment);
        if !(a.is_finite() && b.is_finite()) {
            return Err(degenerate_frame());
        }
        (low, high) = (low.min(a), high.max(b));
    }
    if low < -linear && high > linear {
        // Name what is on the side with less of the profiles' area.
        let mut moment = 0.0;
        for profile in profiles {
            if let Some(centre) = profile.centroid() {
                moment += profile.area() * axis.coordinates(centre).0;
            }
        }
        let mostly_left = if moment.is_finite() && moment != 0.0 {
            moment > 0.0
        } else {
            high >= -low
        };
        let mut curves: Vec<u64> = all_segments()
            .filter(|s| {
                let (a, b) = axis.reach(s);
                if mostly_left { a < -linear } else { b > linear }
            })
            .map(|s| s.source)
            .collect();
        curves.sort_unstable();
        curves.truncate(MAX_REPORTED);
        return Err(KernelError::CrossesAxis { curves });
    }
    let side = if high > linear {
        1.0
    } else if low < -linear {
        -1.0
    } else {
        return Err(invalid("the profile lies on the axis"));
    };
    // The longest arc swept must be longer than the tolerance.
    let reach = high.max(-low);
    if sweep * reach <= linear {
        return Err(KernelError::InvalidAngles { from, to });
    }

    // Angle 0 of every surface is where the revolution starts; a full one
    // starts in the sketch plane.
    let start = if full { 0.0 } else { from };
    let origin = plane.point(axis_point[0], axis_point[1], 0.0);
    let along = plane.vector(direction[0], direction[1], 0.0);
    let towards = plane.vector(axis.left[0], axis.left[1], 0.0) * (side * start.cos())
        + plane.z() * (side * start.sin());
    let frame = Frame::new(origin, along, towards).ok_or_else(degenerate_frame)?;
    let mut b = Builder {
        body: Body::new(BodyKind::Solid),
        origins: Origins::default(),
        frame,
        sweep: if full { TAU } else { sweep },
        full,
        side,
        // The sketch's x axis has components (left, along) = (left.x,
        // direction.x) on the axis's own axes.
        sketch_x: (side * axis.left[0], direction[0]),
        linear,
        angular,
    };
    // The start cap looks back along the turn, the end cap forward.
    let caps = if full {
        None
    } else {
        Some((
            Frame::new(origin, -b.onward(0.0), b.frame.z()).ok_or_else(degenerate_frame)?,
            Frame::new(origin, b.onward(sweep), b.frame.z()).ok_or_else(degenerate_frame)?,
        ))
    };
    for profile in profiles {
        let mut faces = Vec::new();
        let mut start_loops = Vec::new();
        let mut end_loops = Vec::new();
        for path in std::iter::once(&profile.outer).chain(&profile.holes) {
            let swept = b.sweep_loop(&axis, path)?;
            if full {
                for group in swept.groups {
                    b.body.add_shell(group);
                }
                continue;
            }
            faces.extend(swept.groups.into_iter().flatten());
            // Seen from outside a cap, the loops of a profile left of the
            // axis run backwards on the start cap; to the right of it, on
            // the end cap.
            let (at_start, at_end) = if side > 0.0 {
                (backwards(&swept.start), swept.end)
            } else {
                (swept.start, backwards(&swept.end))
            };
            start_loops.push(b.body.add_loop(at_start));
            end_loops.push(b.body.add_loop(at_end));
        }
        let Some((start_cap, end_cap)) = caps else {
            continue;
        };
        let mut sources: Vec<u64> = profile.outer.segments.iter().map(|s| s.source).collect();
        sources.sort_unstable();
        faces.push(b.face(
            Surface::Plane(Plane::new(start_cap)),
            false,
            start_loops,
            Role::StartCap,
            sources.clone(),
        ));
        faces.push(b.face(
            Surface::Plane(Plane::new(end_cap)),
            false,
            end_loops,
            Role::EndCap,
            sources,
        ));
        b.body.add_shell(faces);
    }

    let mut defects = check(&b.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "revolve",
            defects,
        });
    }
    Ok(OpResult {
        body: b.body,
        origins: b.origins,
    })
}
