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
//! | spline that stays clear of the axis but for its ends | rational B-spline surface |
//!
//! A spline sweeps a surface of revolution that is a NURBS surface exactly:
//! around the axis (u) the standard rational quadratic arcs of at most a
//! quarter turn each, along the spline (v) the spline's own degree, knots
//! and control points, each control point turned into the control polygon
//! of its circle. Its parameter u runs over `[0, 1]` for the angle turned
//! through and is not the angle. A spline that ends on the axis sweeps a
//! surface whose border there is one point (as a cone has its apex); one
//! that reaches the axis anywhere else is an error that names it.
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
//! A spline is the exception: `craft3d-geom` knows no period for a spline
//! surface, so a face that goes all the way around on one could not be
//! followed across its seam by anything downstream. A full revolution of
//! a loop with a spline in it is therefore made in two halves. Each
//! spline sweeps two faces, from the sketch plane half a turn on and from
//! there back, each on a surface of its own that ends where the face
//! does; they meet in the spline's edge in the sketch plane (its start
//! edge) and in its edge half a turn on (its end edge). At both ends of a
//! spline the circle about the axis is two arcs between a start vertex
//! and an end vertex, and a line or an arc next to a spline has a face
//! whose loop there is those two arcs. Of two entities with the same
//! role and sources the one of the second half is piece 1
//! ([`crate::extrude::Origin::piece`]; for a spline that was itself cut
//! into pieces, piece `2 k` and `2 k + 1` for its piece `k`). A vertex
//! where a spline ends on the axis is one vertex for both halves.
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

use crate::extrude::{OpResult, Origin, Origins, Role, cap_sources, each_curve_once};
use crate::profile::{Loop2, Profile, Segment, Shape};
use crate::spline2::{self, Part as CurvePart};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::{
    Circle, Cone, Curve3, Cylinder, GeomError, Line, NurbsCurve, NurbsSurface, Plane, Sphere,
    Surface, Torus,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, LoopId, VertexId, check};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

type P2 = [f64; 2];

/// Why a spline that reaches the axis between its ends is not revolved.
const PINCHED: &str = "the spline reaches the axis between its ends, where the surface it sweeps would pinch into a point; keep it clear of the axis, or end it there";

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
        if let Shape::Spline(spline) = &segment.shape {
            // The ends are the segment's own (they may have been moved
            // onto a neighbour); in between, the spline's extremes.
            let (least, most) = spline.extent(self.point, self.left);
            (low, high) = (low.min(least), high.max(most));
        }
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
    /// For a spline: its control points in the half-plane, in the spline's
    /// own order, as (distance from the axis, height along it). The end
    /// ones are the corners exactly. Empty for the other shapes.
    net: Vec<(f64, f64)>,
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

    fn vertex(&mut self, point: Vec3, origin: Origin) -> VertexId {
        self.origins.vertices.push(origin);
        self.body.add_vertex(point)
    }

    fn edge(
        &mut self,
        curve: Curve3,
        range: (f64, f64),
        ends: (VertexId, VertexId),
        origin: Origin,
    ) -> EdgeId {
        self.origins.edges.push(origin);
        self.body.add_edge(curve, range, ends.0, ends.1)
    }

    fn face(
        &mut self,
        surface: Surface,
        reversed: bool,
        loops: Vec<LoopId>,
        origin: Origin,
    ) -> FaceId {
        self.origins.faces.push(origin);
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
        let mut net = Vec::new();
        let face = match &segment.shape {
            Shape::Spline(spline) => {
                net = self.spline_net(axis, segment, spline, start, end)?;
                let turn = if self.full { PI } else { self.sweep };
                let surface = self.spline_surface(spline, &net, 0.0, turn)?;
                // The normal (around the axis crossed with along the
                // spline) is to the right of the spline in the half-plane,
                // which mirrors the sketch when the profile is to the left
                // of the axis: the inside, for a loop that runs the
                // spline's way.
                Some((
                    Surface::Nurbs(surface),
                    (self.side > 0.0) != segment.reversed,
                ))
            }
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
            &Shape::Arc { center, radius } | &Shape::Circle { center, radius } => {
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
            net,
            face,
        })
    }

    /// The control points of `spline` in the half-plane, in its own order,
    /// with its ends at the corners `start` and `end` of the loop. Errors
    /// when the spline reaches the axis anywhere but at an end.
    fn spline_net(
        &self,
        axis: &Axis,
        segment: &Segment,
        spline: &spline2::Spline2,
        start: Rz,
        end: Rz,
    ) -> Result<Vec<(f64, f64)>, KernelError> {
        let pinched = || KernelError::UnsupportedCurve {
            id: segment.source,
            reason: PINCHED,
        };
        let mut net: Vec<(f64, f64)> = spline
            .control
            .iter()
            .map(|&p| {
                let (left, along) = axis.coordinates(p);
                (self.side * left, along)
            })
            .collect();
        let (first, last) = if segment.reversed {
            (end, start)
        } else {
            (start, end)
        };
        if let Some(slot) = net.first_mut() {
            *slot = (first.rho, first.z);
        }
        if let Some(slot) = net.last_mut() {
            *slot = (last.rho, last.z);
        }
        if !net.iter().all(|p| p.0.is_finite() && p.1.is_finite()) {
            return Err(degenerate_frame());
        }
        // Where the spline is within tolerance of the axis: only a stretch
        // that reaches an end which is on the axis, and not all of it.
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for &(_, z) in &net {
            (low, high) = (low.min(z), high.max(z));
        }
        let reach = (high - low).abs() + self.linear;
        let on_axis = |z: f64| {
            [
                axis.point[0] + axis.direction[0] * z,
                axis.point[1] + axis.direction[1] * z,
            ]
        };
        let line = [CurvePart::line(on_axis(low - reach), on_axis(high + reach))];
        let runs = spline2::close_runs(&spline.pieces(), &line, self.linear).ok_or_else(pinched)?;
        for (from, to) in runs {
            let at_start = from <= spline2::RUN_GAP && first.rho == 0.0;
            let at_end = to >= 1.0 - spline2::RUN_GAP && last.rho == 0.0;
            if at_start == at_end {
                return Err(pinched());
            }
        }
        Ok(net)
    }

    /// The surface a spline sweeps from the angle `from` through `turn`:
    /// u around the axis, in rational quadratic arcs of at most a quarter
    /// turn, v along the spline.
    fn spline_surface(
        &self,
        spline: &spline2::Spline2,
        net: &[(f64, f64)],
        from: f64,
        turn: f64,
    ) -> Result<NurbsSurface, KernelError> {
        let ring = arcs_about(from, turn);
        let mut control = Vec::with_capacity(ring.points.len() * net.len());
        let mut weights = Vec::with_capacity(ring.points.len() * net.len());
        for &(x, y, weight) in &ring.points {
            for &(rho, z) in net {
                control.push(self.frame.point(rho * x, rho * y, z));
                weights.push(weight);
            }
        }
        Ok(NurbsSurface::new(
            2,
            spline.degree,
            ring.knots,
            spline.knots.clone(),
            ring.points.len(),
            net.len(),
            control,
            weights,
        )?)
    }

    /// The edge of `part` in the half-plane at `angle`, between the
    /// vertices at its start and end as the loop runs, and whether the loop
    /// runs against it.
    fn profile_edge(
        &mut self,
        part: &Part,
        angle: f64,
        ends: (VertexId, VertexId),
        origin: Origin,
    ) -> Result<(EdgeId, bool), KernelError> {
        let segment = part.segment;
        // The sketch plane turned to `angle`: its normal and its x axis.
        let turned = |b: &Builder, radius: f64, centre: Rz| -> Result<Curve3, KernelError> {
            let normal = b.onward(angle) * b.side;
            let x = b.radial(angle) * b.sketch_x.0 + b.frame.z() * b.sketch_x.1;
            let frame =
                Frame::new(b.point(centre, angle), normal, x).ok_or_else(degenerate_frame)?;
            Ok(Curve3::Circle(Circle::new(frame, radius)?))
        };
        match &segment.shape {
            Shape::Spline(spline) => {
                let control = part
                    .net
                    .iter()
                    .map(|&(rho, z)| self.point(Rz { rho, z }, angle))
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
                    self.edge(Curve3::Nurbs(curve), range, own, origin),
                    segment.reversed,
                ))
            }
            Shape::Line => {
                let (start, end) = (self.point(part.start, angle), self.point(part.end, angle));
                let line = Curve3::Line(Line::through(start, end)?);
                let range = (0.0, start.distance(end));
                Ok((self.edge(line, range, ends, origin), false))
            }
            &Shape::Arc { radius, .. } => {
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
                Ok((self.edge(circle, range, own, origin), segment.reversed))
            }
            &Shape::Circle { radius, .. } => {
                let circle = turned(self, radius, part.centre)?;
                Ok((
                    self.edge(circle, (0.0, TAU), ends, origin),
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
        let names: Vec<([u64; 2], u32)> = before
            .zip(segments)
            .map(|(arriving, leaving)| ([arriving.source, leaving.source], leaving.piece))
            .collect();
        let (full, sweep) = (self.full, self.sweep);
        // A full revolution of a loop with a spline in it is made in two
        // halves (see the module documentation): the entities of the
        // second half are the second their sources give.
        let halved = full && parts.iter().any(|p| !p.net.is_empty());
        let numbered = |piece: u32, half: u32| {
            if halved {
                piece.saturating_mul(2).saturating_add(half)
            } else {
                piece
            }
        };
        // The corners at an end of a spline: there the halves meet in a
        // vertex, and the circle about the axis is two arcs.
        let is_spline: Vec<bool> = parts.iter().map(|p| !p.net.is_empty()).collect();
        let spline_before = is_spline.iter().cycle().skip(n - 1);
        let cut: Vec<bool> = is_spline
            .iter()
            .zip(spline_before)
            .map(|(&leaving, &arriving)| full && (leaving || arriving))
            .collect();

        // Vertices: at the start, then at the end. One on the axis is the
        // same at both; a full revolution has only those at the start, and
        // none where no edge would use them: on the axis, and on a whole
        // circle. Where a full revolution is in two halves, the end is
        // half a turn on.
        let turn_end = if full { PI } else { sweep };
        let mut low: Vec<Option<VertexId>> = Vec::with_capacity(n);
        for ((part, (name, piece)), &cut_here) in parts.iter().zip(&names).zip(&cut) {
            let wanted = if full {
                (part.start.rho > 0.0 && !whole_circle) || cut_here
            } else {
                true
            };
            low.push(wanted.then(|| {
                let point = self.point(part.start, 0.0);
                let origin = Origin::of_piece(Role::StartVertex, name, numbered(*piece, 0));
                self.vertex(point, origin)
            }));
        }
        let mut high: Vec<Option<VertexId>> = Vec::with_capacity(n);
        for (((part, (name, piece)), &same), &cut_here) in
            parts.iter().zip(&names).zip(&low).zip(&cut)
        {
            high.push(if part.start.rho == 0.0 || (full && !cut_here) {
                same
            } else {
                let point = self.point(part.start, turn_end);
                let origin = Origin::of_piece(Role::EndVertex, name, numbered(*piece, 0));
                Some(self.vertex(point, origin))
            });
        }

        // The profile's edges at the start and at the end (in a full
        // revolution: of the splines only, at the start and half a turn
        // on). A line on the axis is one edge for both.
        let mut bottom: Vec<Option<(EdgeId, bool)>> = vec![None; n];
        let mut top: Vec<Option<(EdgeId, bool)>> = vec![None; n];
        for (layer, angle, role) in [(0, 0.0, Role::StartEdge), (1, turn_end, Role::EndEdge)] {
            let vertices = if layer == 0 { &low } else { &high };
            let after = vertices.iter().cycle().skip(1);
            let mut made = Vec::with_capacity(n);
            for (i, (part, (&here, &there))) in
                parts.iter().zip(vertices.iter().zip(after)).enumerate()
            {
                if full && part.net.is_empty() {
                    made.push(None);
                    continue;
                }
                let shared = bottom.get(i).copied().flatten();
                made.push(match (part.face.is_none(), shared, layer) {
                    (true, Some(edge), 1) => Some(edge),
                    _ => {
                        let (Some(here), Some(there)) = (here, there) else {
                            return Err(invalid("a loop lost a vertex"));
                        };
                        let origin = Origin::of_piece(
                            role,
                            &[part.segment.source],
                            numbered(part.segment.piece, 0),
                        );
                        Some(self.profile_edge(part, angle, (here, there), origin)?)
                    }
                });
            }
            if layer == 0 {
                bottom = made;
            } else {
                top = made;
            }
        }

        // The circle or arc about the axis that each vertex off it sweeps;
        // two arcs where a full revolution is in two halves.
        let mut arcs: Vec<Vec<EdgeId>> = Vec::with_capacity(n);
        for ((((part, (name, piece)), &from), &to), &cut_here) in
            parts.iter().zip(&names).zip(&low).zip(&high).zip(&cut)
        {
            let mut made = Vec::new();
            if part.start.rho > 0.0 && !whole_circle {
                let (Some(from), Some(to)) = (from, to) else {
                    return Err(invalid("a loop lost a vertex"));
                };
                let circle =
                    Curve3::Circle(Circle::new(self.level(part.start.z)?, part.start.rho)?);
                let stretches: &[Stretch] = if cut_here {
                    &[((0.0, PI), (from, to), 0), ((PI, TAU), (to, from), 1)]
                } else {
                    &[((0.0, if full { TAU } else { sweep }), (from, to), 0)]
                };
                for &(range, ends, half) in stretches {
                    let origin = Origin::of_piece(Role::LateralEdge, name, numbered(*piece, half));
                    made.push(self.edge(circle.clone(), range, ends, origin));
                }
            }
            arcs.push(made);
        }

        // Seen from outside, which is to the right of the loop, a profile
        // left of the axis turns away from the viewer's left: the face runs
        // along its start edge, up the arc of its end vertex, back along
        // its end edge and down the arc of its start vertex. To the right
        // of the axis it is the mirror image.
        let along = self.side > 0.0;
        let none: Vec<EdgeId> = Vec::new();
        let mut groups: Vec<Vec<FaceId>> = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            let Some((surface, reversed)) = part.face.clone() else {
                continue;
            };
            let here = arcs.get(i).unwrap_or(&none);
            let there = arcs.get((i + 1) % n).unwrap_or(&none);
            let (source, piece) = (part.segment.source, part.segment.piece);
            // The ring of a face between the profile's edge `first` where
            // its turn starts and `last` where it ends, in half `half`.
            let ring = |b: &mut Builder,
                        (first, first_against): (EdgeId, bool),
                        (last, last_against): (EdgeId, bool),
                        half: usize| {
                let (here, there) = (here.get(half).copied(), there.get(half).copied());
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
                b.body.add_loop(ring)
            };
            let edges = (
                bottom.get(i).copied().flatten(),
                top.get(i).copied().flatten(),
            );
            let mut made = Vec::with_capacity(2);
            if let (true, Shape::Spline(spline)) = (full, &part.segment.shape) {
                // Two halves: from the start edge to the one half a turn
                // on, and from that one back to the start edge.
                let (Some(first), Some(last)) = edges else {
                    return Err(invalid("a loop lost an edge"));
                };
                let second = self.spline_surface(spline, &part.net, PI, PI)?;
                let first_loop = ring(self, first, last, 0);
                let second_loop = ring(self, last, first, 1);
                for (surface, ring, half) in [
                    (surface, first_loop, 0),
                    (Surface::Nurbs(second), second_loop, 1),
                ] {
                    let origin = Origin::of_piece(Role::Side, &[source], numbered(piece, half));
                    made.push(self.face(surface, reversed, vec![ring], origin));
                }
            } else {
                let mut loops = Vec::new();
                if full {
                    // Left of the circle at its start when the profile is
                    // to the right of the axis, and the other way round. A
                    // circle in two arcs is gone round in their order, or
                    // against both from the second.
                    for (arcs, against) in [(here, along), (there, !along)] {
                        let mut round: Vec<Coedge> =
                            arcs.iter().map(|&arc| coedge(arc, against)).collect();
                        if against {
                            round.reverse();
                        }
                        if !round.is_empty() {
                            loops.push(self.body.add_loop(round));
                        }
                    }
                } else {
                    let (Some((first, first_against)), Some((last, last_against))) = edges else {
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
                        loops.push(ring(self, (first, first_against), (last, last_against), 0));
                    }
                }
                let origin = Origin::of_piece(Role::Side, &[source], numbered(piece, 0));
                made.push(self.face(surface, reversed, loops, origin));
            }
            // Faces either side of a vertex on the axis share no edge.
            match groups.last_mut() {
                Some(group) if part.start.rho > 0.0 => group.extend(made),
                _ => groups.push(made),
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

/// The control polygon of an arc about the origin, as rational quadratic
/// pieces: per control point its x, y and weight, with the knots.
struct Ring {
    points: Vec<(f64, f64, f64)>,
    knots: Vec<f64>,
}

/// The unit arc from the angle `from` through `sweep` (at most a full
/// turn) in equal pieces of at most a quarter turn: each a rational
/// quadratic Bézier arc whose middle control point is where the tangents
/// at its ends meet, with the cosine of half its angle as weight (*The
/// NURBS Book* §7.3). The parameter runs over `[0, 1]`.
fn arcs_about(from: f64, sweep: f64) -> Ring {
    let pieces = if sweep > 1.5 * PI + 1e-9 {
        4_u8
    } else if sweep > PI + 1e-9 {
        3
    } else if sweep > FRAC_PI_2 + 1e-9 {
        2
    } else {
        1
    };
    let half = sweep / (2.0 * f64::from(pieces));
    let weight = half.cos();
    let mut points = Vec::with_capacity(2 * usize::from(pieces) + 1);
    for k in 0..=2 * pieces {
        let (sin, cos) = (from + half * f64::from(k)).sin_cos();
        points.push(if k % 2 == 0 {
            (cos, sin, 1.0)
        } else {
            (cos / weight, sin / weight, weight)
        });
    }
    let mut knots = vec![0.0; 3];
    for k in 1..pieces {
        let knot = f64::from(k) / f64::from(pieces);
        knots.extend([knot, knot]);
    }
    knots.extend([1.0; 3]);
    Ring { points, knots }
}

/// A stretch of the circle a vertex sweeps: its range of angles, the
/// vertices at its ends and which half of a full revolution it is in.
type Stretch = ((f64, f64), (VertexId, VertexId), u32);

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
    each_curve_once(profiles)?;

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
        curves.dedup();
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
        let sources = cap_sources(&profile.outer);
        faces.push(b.face(
            Surface::Plane(Plane::new(start_cap)),
            false,
            start_loops,
            Origin::new(Role::StartCap, &sources),
        ));
        faces.push(b.face(
            Surface::Plane(Plane::new(end_cap)),
            false,
            end_loops,
            Origin::new(Role::EndCap, &sources),
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
