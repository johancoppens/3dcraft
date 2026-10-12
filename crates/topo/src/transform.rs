//! Moving, mirroring, merging and boxing bodies: [`Body::transformed`],
//! [`Body::bbox`] and [`merge`]. All of it works on the exact geometry;
//! nothing is approximated or tessellated.
//!
//! # Moving and mirroring
//!
//! [`Body::transformed`] takes an isometry: a rigid motion, or one that
//! also mirrors (determinant −1). Entity `i` of the result is the image of
//! entity `i`, for vertices, edges, loops, faces and shells alike.
//!
//! **Rigid motion.** Every curve and surface keeps its parametrisation
//! (`C'(t) = T(C(t))`, `S'(u, v) = T(S(u, v))`), so nothing else changes:
//! edge ranges, edge directions, coedges, loop order and `reversed` flags
//! are copied.
//!
//! **Mirror.** A mirror turns right-handed things left-handed, and three
//! conventions of this crate are about handedness. What is done about each:
//!
//! - *Edges do not change.* Every curve kind keeps its parametrisation
//!   under a mirror too (a circle's frame gets the opposite z axis, which a
//!   curve in the frame's xy plane does not notice). So an edge keeps its
//!   range, its direction and its start and end vertex.
//! - *Every loop is turned round.* Seen from outside, a face lies to the
//!   left of its loops; in the mirror image it would lie to the right. So
//!   each loop is walked the other way: its coedges in reverse order, each
//!   with its sense toggled. The loop still starts with the same edge:
//!   coedges `[c0, c1, …, cn]` become `[c0', cn', …, c1']`. Every edge of a
//!   solid is still used once in each direction.
//! - *A face's `reversed` flag toggles exactly when the surface's normal
//!   flips.* The outward normal of the result must be the image of the
//!   outward normal. Per surface kind (`craft3d_geom::MirrorRule`):
//!
//!   | kind | mirrored surface | `reversed` |
//!   |---|---|---|
//!   | plane | frame `(Tx, Ty, −Tz)`: same `(u, v)`, normal flipped | **toggled** |
//!   | Bézier, NURBS | control points mapped: same `(u, v)`, normal flipped | **toggled** |
//!   | cylinder | frame `(Tx, Ty, −Tz)`: `v` negated, normal still away from the axis | kept |
//!   | cone | the same, and the half-angle negated so it still opens the right way | kept |
//!   | sphere | frame `(Tx, Ty, −Tz)`: latitude negated, normal still away from the centre | kept |
//!   | torus | frame `(Tx, Ty, −Tz)`: `v` negated, normal still away from the tube's centre line | kept |
//!
//! Faces store no parameter-space curves, so the changed `v` of the
//! surfaces of revolution needs no further fix-up: whoever needs a face's
//! boundary in `(u, v)` projects the edges, as before.
//!
//! # What is an error
//!
//! A transform that is not an isometry (scaling, also uniform scaling,
//! shear, anything not finite), and a result that would not be finite.
//! Bodies that fail [`crate::check`] are transformed entity by entity
//! without following their references; they come out as invalid as they
//! went in, and never cause a panic.

use crate::{Body, Coedge, EdgeId, Face, FaceId, Loop, LoopId, Shell, VertexId};
use craft3d_geom::{Cone, Curve3, GeomError, ISOMETRY_EPSILON, MirrorRule, Surface};
use craft3d_math::{Bbox3, Frame, Transform, Vec3};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum TransformError {
    /// The transform is not a rigid motion or a mirror: it scales, shears
    /// or is not finite.
    NotIsometry,
    /// A vertex is not finite, or its image is not.
    Vertex { vertex: VertexId },
    /// An edge's curve cannot be transformed, or is not finite.
    Edge { edge: EdgeId, error: GeomError },
    /// A face's surface cannot be transformed.
    Face { face: FaceId, error: GeomError },
    /// An edge's parameter range is not finite.
    EdgeRange { edge: EdgeId },
    /// A face has no bounding box: a plane, cylinder or cone without loops.
    UnboundedFace { face: FaceId },
    /// A face or loop of body `body` refers to something that does not
    /// exist; run `check`.
    DanglingReference { body: usize },
    /// `merge` was given no bodies.
    NothingToMerge,
    /// `merge` was given solids and sheets together.
    MixedKinds,
    /// The merged body would have more than 2^32 − 1 entities of one kind.
    TooLarge,
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransformError::NotIsometry => write!(
                f,
                "the transform is not a rigid motion or a mirror: scaling, shear and non-finite \
                 transforms are not supported for bodies"
            ),
            TransformError::Vertex { vertex } => write!(
                f,
                "vertex {} is not finite, or would not be after the transform",
                vertex.0
            ),
            TransformError::Edge { edge, error } => write!(f, "edge {}: {error}", edge.0),
            TransformError::Face { face, error } => write!(f, "face {}: {error}", face.0),
            TransformError::EdgeRange { edge } => {
                write!(
                    f,
                    "edge {} has a parameter range that is not finite",
                    edge.0
                )
            }
            TransformError::UnboundedFace { face } => write!(
                f,
                "face {} is not bounded: an open surface without loops",
                face.0
            ),
            TransformError::DanglingReference { body } => write!(
                f,
                "body {body} refers to an entity that does not exist; check the body first"
            ),
            TransformError::NothingToMerge => write!(f, "there are no bodies to merge"),
            TransformError::MixedKinds => {
                write!(f, "solid and sheet bodies cannot be merged into one body")
            }
            TransformError::TooLarge => {
                write!(f, "the merged body would have too many entities")
            }
        }
    }
}

impl std::error::Error for TransformError {}

fn id(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// A loop walked the other way, starting with the same edge.
fn turned_round(loop_: &Loop) -> Loop {
    let toggled = |c: &Coedge| Coedge {
        edge: c.edge,
        reversed: !c.reversed,
    };
    let mut coedges = Vec::with_capacity(loop_.coedges.len());
    coedges.extend(loop_.coedges.first().map(toggled));
    coedges.extend(loop_.coedges.iter().skip(1).rev().map(toggled));
    Loop { coedges }
}

impl Body {
    /// The body moved by an isometry: a rigid motion, or one that mirrors
    /// (determinant −1), as `Transform::is_isometry` recognises it within
    /// `craft3d_geom::ISOMETRY_EPSILON`.
    ///
    /// Ids and order of vertices, edges, loops, faces and shells are
    /// preserved: entity `i` of the result is the image of entity `i`.
    /// Edges keep their parameter range and direction: the point at
    /// parameter `t` of edge `i` is the image of the point at `t` of the
    /// original edge. Under a mirror every loop is walked the other way
    /// round (starting with the same edge) and the faces on planes and
    /// freeform surfaces toggle `reversed`; see the module documentation
    /// for the rule per surface kind. A valid body gives a valid body with
    /// the same volume (positive for a solid, mirrored or not) and area.
    ///
    /// The result is exact up to rounding: no curve or surface changes
    /// kind, and nothing is approximated. The identity gives an equal body
    /// up to the last bit of its frames' axes, which are re-orthonormalised.
    pub fn transformed(&self, t: &Transform) -> Result<Body, TransformError> {
        if !t.is_isometry(ISOMETRY_EPSILON) {
            return Err(TransformError::NotIsometry);
        }
        let mirror = t.determinant() < 0.0;

        let mut body = Body::new(self.kind);
        body.vertices.reserve(self.vertices.len());
        for (i, vertex) in self.vertices.iter().enumerate() {
            let point = t.point(vertex.point);
            if !point.is_finite() {
                return Err(TransformError::Vertex {
                    vertex: VertexId(id(i)),
                });
            }
            body.vertices.push(crate::Vertex { point });
        }

        body.edges.reserve(self.edges.len());
        for (i, edge) in self.edges.iter().enumerate() {
            // The curve keeps its parametrisation, mirrored or not, so the
            // range and the direction stand.
            let curve = edge
                .curve
                .transformed(t)
                .map_err(|error| TransformError::Edge {
                    edge: EdgeId(id(i)),
                    error,
                })?;
            body.edges.push(crate::Edge {
                curve,
                range: edge.range,
                start: edge.start,
                end: edge.end,
            });
        }

        body.loops = if mirror {
            self.loops.iter().map(turned_round).collect()
        } else {
            self.loops.clone()
        };

        body.faces.reserve(self.faces.len());
        for (i, face) in self.faces.iter().enumerate() {
            let surface = face
                .surface
                .transformed(t)
                .map_err(|error| TransformError::Face {
                    face: FaceId(id(i)),
                    error,
                })?;
            // Outward must stay the image of outward: where the mirrored
            // surface's own normal is the opposite of the image of the
            // normal, the flag makes up for it.
            let flips = mirror && face.surface.mirror_rule() == MirrorRule::FlipsNormal;
            body.faces.push(Face {
                surface,
                reversed: face.reversed != flips,
                loops: face.loops.clone(),
            });
        }

        body.shells = self.shells.clone();
        Ok(body)
    }

    /// A box that certainly contains the body, computed from the exact
    /// geometry. Conservative, and not necessarily tight:
    ///
    /// - Every edge contributes the exact extent of its arc of line, circle
    ///   or ellipse; a NURBS edge the box of its whole control polygon.
    /// - A face on a plane or a cylinder lies within the convex hull of its
    ///   edges and adds nothing; so does a face on a cone, except for the
    ///   apex when the face surrounds it.
    /// - A face on a sphere or a torus contributes the box of the *whole*
    ///   sphere or torus, a face on a Bézier or NURBS surface the box of
    ///   the whole control net, however little of the surface the face
    ///   uses.
    ///
    /// So the box is the tight one (up to the padding below) for a body
    /// whose faces are planes, cylinders and cones and whose edges are
    /// lines, circles and ellipses, and for whole spheres and tori; for
    /// other bodies it is never larger than the box of the body's edges
    /// together with its whole spheres, tori and control nets. It is
    /// padded by a few rounding errors of the largest coordinate (a
    /// relative 1.5e-14), so that points computed on the geometry are
    /// inside whatever the last bit does.
    ///
    /// One step is not a proof: whether a cone face surrounds its apex is
    /// decided by following its boundary around the axis at sampled
    /// points. Where that is doubtful the apex is included.
    ///
    /// An empty box for a body without vertices, edges and faces. An error
    /// for a face that is not bounded (a plane, cylinder or cone without
    /// loops), for geometry that is not finite, and for a cone face whose
    /// loops refer to something missing.
    pub fn bbox(&self) -> Result<Bbox3, TransformError> {
        let mut extent = Extent::EMPTY;
        for (i, vertex) in self.vertices.iter().enumerate() {
            if !vertex.point.is_finite() {
                return Err(TransformError::Vertex {
                    vertex: VertexId(id(i)),
                });
            }
            extent.add(vertex.point);
        }
        for (i, edge) in self.edges.iter().enumerate() {
            let edge_id = EdgeId(id(i));
            edge_extent(&edge.curve, edge.range, edge_id, &mut extent)?;
            if !extent.finite {
                return Err(TransformError::Edge {
                    edge: edge_id,
                    error: GeomError::InvalidParameter("the curve is not finite over its range"),
                });
            }
        }
        for (i, face) in self.faces.iter().enumerate() {
            let face_id = FaceId(id(i));
            face_extent(self, face, face_id, &mut extent)?;
            if !extent.finite {
                return Err(TransformError::Face {
                    face: face_id,
                    error: GeomError::InvalidParameter("the surface is not finite"),
                });
            }
        }
        Ok(extent.padded())
    }
}

/// A box in the making: per axis the least and the greatest coordinate
/// seen, which need not come from the same point.
#[derive(Clone, Copy)]
struct Extent {
    min: [f64; 3],
    max: [f64; 3],
    /// False once anything not finite was added (`f64::min` would
    /// quietly drop a NaN).
    finite: bool,
}

impl Extent {
    const EMPTY: Extent = Extent {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
        finite: true,
    };

    /// Adds one coordinate along one axis (0, 1 or 2).
    fn add_axis(&mut self, axis: usize, value: f64) {
        if !value.is_finite() {
            self.finite = false;
        } else if let (Some(min), Some(max)) = (self.min.get_mut(axis), self.max.get_mut(axis)) {
            *min = min.min(value);
            *max = max.max(value);
        }
    }

    fn add(&mut self, p: Vec3) {
        for (axis, value) in components(p).into_iter().enumerate() {
            self.add_axis(axis, value);
        }
    }

    /// Adds `centre ± reach`, per axis.
    fn add_reach(&mut self, centre: Vec3, reach: Vec3) {
        self.add(centre - reach);
        self.add(centre + reach);
    }

    fn padded(&self) -> Bbox3 {
        let [x0, y0, z0] = self.min;
        let [x1, y1, z1] = self.max;
        if !self.finite || x0 > x1 {
            return Bbox3::EMPTY; // Nothing was added.
        }
        let (min, max) = (Vec3::new(x0, y0, z0), Vec3::new(x1, y1, z1));
        let largest = self
            .min
            .iter()
            .chain(&self.max)
            .fold(0.0_f64, |a, b| a.max(b.abs()));
        let pad = PAD * largest;
        let pad = Vec3::new(pad, pad, pad);
        // At the very edge of the representable range the padding would
        // overflow; the unpadded box is then the best there is.
        let (lo, hi) = (min - pad, max + pad);
        if lo.is_finite() && hi.is_finite() {
            Bbox3::from_points([lo, hi])
        } else {
            Bbox3::from_points([min, max])
        }
    }
}

/// Relative padding of [`Body::bbox`]: 64 rounding errors of the largest
/// coordinate.
const PAD: f64 = 64.0 * f64::EPSILON;

/// How far outside an arc's range an extreme point may be and still be
/// counted, in radians. Costs at most `radius × SLACK² / 2` of tightness.
const ANGLE_SLACK: f64 = 1e-9;

/// The components of `v`, in axis order.
fn components(v: Vec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

/// Adds the exact extent of the arc `range` of the conic
/// `origin + a cos t · x + b sin t · y`.
fn arc_extent(frame: &Frame, a: f64, b: f64, curve: &Curve3, range: (f64, f64), out: &mut Extent) {
    let (lo, hi) = (range.0.min(range.1), range.0.max(range.1));
    out.add(curve.eval(lo));
    out.add(curve.eval(hi));
    let origin = components(frame.origin());
    let (x, y) = (components(frame.x()), components(frame.y()));
    for (axis, ((o, x), y)) in origin.into_iter().zip(x).zip(y).enumerate() {
        // Along this axis the coordinate is o + p cos t + q sin t
        // = o + h cos(t − peak), greatest at `peak`, least half a turn on.
        let (p, q) = (a * x, b * y);
        let h = p.hypot(q);
        if h.is_nan() || h <= 0.0 {
            continue; // The conic is flat along this axis (or not finite).
        }
        let peak = q.atan2(p);
        for (extreme, value) in [(peak, o + h), (peak + PI, o - h)] {
            // The first angle equal to `extreme` (mod 2π) at or after the
            // start of the range.
            let turns = ((lo - ANGLE_SLACK - extreme) / TAU).ceil();
            let reached = extreme + turns * TAU <= hi + ANGLE_SLACK;
            // A range of a whole turn reaches everything; so does one too
            // large for the arithmetic above to mean anything.
            if reached || hi - lo >= TAU || !turns.is_finite() {
                out.add_axis(axis, value);
            }
        }
    }
}

fn edge_extent(
    curve: &Curve3,
    range: (f64, f64),
    edge: EdgeId,
    out: &mut Extent,
) -> Result<(), TransformError> {
    if !(range.0.is_finite() && range.1.is_finite()) {
        return Err(TransformError::EdgeRange { edge });
    }
    match curve {
        Curve3::Line(_) => {
            out.add(curve.eval(range.0));
            out.add(curve.eval(range.1));
        }
        Curve3::Circle(c) => arc_extent(c.frame(), c.radius(), c.radius(), curve, range, out),
        Curve3::Ellipse(e) => {
            let (a, b) = e.semi_axes();
            arc_extent(e.frame(), a, b, curve, range, out);
        }
        // Weights are positive, so the curve is inside the convex hull of
        // its control points.
        Curve3::Nurbs(c) => c.control_points().iter().for_each(|&p| out.add(p)),
    }
    Ok(())
}

/// Samples per edge for following a boundary around a cone's axis.
fn winding_samples(curve: &Curve3) -> usize {
    match curve {
        Curve3::Line(_) => 4,
        Curve3::Circle(_) | Curve3::Ellipse(_) => 64,
        Curve3::Nurbs(c) => c.control_points().len().saturating_mul(16).clamp(64, 4096),
    }
}

/// Whether a face on `cone` contains the cone's apex, in its interior or on
/// its boundary. Errs on the side of yes.
///
/// A face that does not reach the apex is cut off from it by its boundary:
/// either no loop goes around the axis, or as many go round one way as the
/// other (the two rims of a frustum). A face around the apex has a boundary
/// that goes around the axis once in total.
fn reaches_apex(body: &Body, face: &Face, cone: &Cone) -> Result<bool, TransformError> {
    // `bbox` looks at one body: number 0.
    let owner = 0;
    let frame = cone.frame();
    let apex = frame.point(0.0, 0.0, cone.apex_v());
    let mut total = 0.0;
    for loop_id in &face.loops {
        let loop_ = body
            .loop_(*loop_id)
            .ok_or(TransformError::DanglingReference { body: owner })?;
        for coedge in &loop_.coedges {
            let edge = body
                .edge(coedge.edge)
                .ok_or(TransformError::DanglingReference { body: owner })?;
            let (t0, t1) = edge.range;
            if !(t0.is_finite() && t1.is_finite()) {
                return Err(TransformError::EdgeRange { edge: coedge.edge });
            }
            let count = winding_samples(&edge.curve);
            let mut previous: Option<(f64, f64)> = None;
            let mut turn = 0.0;
            for k in 0..=count {
                let point = edge.curve.eval(t0 + (t1 - t0) * (k as f64 / count as f64));
                let (x, y, _) = frame.local(point);
                // On the axis (which a cone only meets at its apex) the
                // angle about it means nothing, and the apex is right here.
                let off_axis = x.hypot(y);
                let near = 1e-9 * point.distance(apex);
                if !(off_axis.is_finite() && near.is_finite()) || off_axis <= near {
                    return Ok(true);
                }
                if let Some((px, py)) = previous {
                    let step = (px * y - py * x).atan2(px * x + py * y);
                    // Too coarse to tell which way round it went.
                    if step.is_nan() || step.abs() > FRAC_PI_2 {
                        return Ok(true);
                    }
                    turn += step;
                }
                previous = Some((x, y));
            }
            total += if coedge.reversed { -turn } else { turn };
        }
    }
    Ok(total.abs() > PI)
}

fn face_extent(
    body: &Body,
    face: &Face,
    id: FaceId,
    out: &mut Extent,
) -> Result<(), TransformError> {
    let bounded = || {
        if face.loops.is_empty() {
            Err(TransformError::UnboundedFace { face: id })
        } else {
            Ok(())
        }
    };
    match &face.surface {
        // Planes are flat and cylinders ruled along their axis: every
        // point of a bounded face lies between two points of its boundary.
        Surface::Plane(_) | Surface::Cylinder(_) => bounded()?,
        // A cone is ruled through its apex: every point lies between two
        // boundary points, or between a boundary point and the apex.
        Surface::Cone(cone) => {
            bounded()?;
            if reaches_apex(body, face, cone)? {
                out.add(cone.frame().point(0.0, 0.0, cone.apex_v()));
            }
        }
        Surface::Sphere(sphere) => {
            let r = sphere.radius();
            out.add_reach(sphere.frame().origin(), Vec3::new(r, r, r));
        }
        Surface::Torus(torus) => {
            // A circle of radius R about the axis, thickened by r: along a
            // unit direction e it reaches R·|e − (e·z) z| + r.
            let (major, minor) = torus.radii();
            let [x, y, z] = components(torus.frame().z())
                .map(|c| major * (1.0 - c * c).max(0.0).sqrt() + minor);
            out.add_reach(torus.frame().origin(), Vec3::new(x, y, z));
        }
        // Convex hull property (weights are positive).
        Surface::Bezier(patch) => patch.control_points().iter().for_each(|&p| out.add(p)),
        Surface::Nurbs(surface) => surface.control_points().iter().for_each(|&p| out.add(p)),
    }
    Ok(())
}

/// Where the entities of one body start in the body [`merge`] made:
/// `FaceId(i)` of that body is `FaceId(offsets.faces + i)` of the result,
/// and likewise for the other entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Offsets {
    pub vertices: u32,
    pub edges: u32,
    pub loops: u32,
    pub faces: u32,
    pub shells: u32,
}

impl Offsets {
    /// The offsets after a body with these counts, or `None` when an id
    /// would no longer fit.
    fn after(&self, body: &Body) -> Option<Offsets> {
        let add = |offset: u32, count: usize| offset.checked_add(u32::try_from(count).ok()?);
        Some(Offsets {
            vertices: add(self.vertices, body.vertices.len())?,
            edges: add(self.edges, body.edges.len())?,
            loops: add(self.loops, body.loops.len())?,
            faces: add(self.faces, body.faces.len())?,
            shells: add(self.shells, body.shells.len())?,
        })
    }
}

/// All shells of `bodies` in one body, in order.
///
/// Entity ids of body `k` are offset by the entity counts of the bodies
/// before it; `offsets[k]` gives those offsets, so `FaceId(i)` of body `k`
/// is `FaceId(offsets[k].faces + i)` of the result. Geometry is copied
/// unchanged.
///
/// The bodies must be of the same kind. **Nothing checks that they are
/// disjoint:** merging bodies that overlap or touch gives a body that
/// passes [`crate::check`] (which does not look for self-intersection)
/// and is not a valid solid; uniting those is a boolean operation.
///
/// An error for no bodies at all, for solids and sheets together, for a
/// body that refers to an entity it does not have (the offset could make
/// such a reference point into another body), and when the result would
/// have more entities of a kind than an id can count.
pub fn merge(bodies: &[&Body]) -> Result<(Body, Vec<Offsets>), TransformError> {
    let first = bodies.first().ok_or(TransformError::NothingToMerge)?;
    if bodies.iter().any(|b| b.kind != first.kind) {
        return Err(TransformError::MixedKinds);
    }
    // All the offsets first: nothing is copied for a result that cannot be.
    let mut offsets = Vec::with_capacity(bodies.len());
    let mut next = Offsets::default();
    for body in bodies {
        offsets.push(next);
        next = next.after(body).ok_or(TransformError::TooLarge)?;
    }

    let mut merged = Body::new(first.kind);
    merged.vertices.reserve(next.vertices as usize);
    merged.edges.reserve(next.edges as usize);
    merged.loops.reserve(next.loops as usize);
    merged.faces.reserve(next.faces as usize);
    merged.shells.reserve(next.shells as usize);
    for (k, (body, offset)) in bodies.iter().zip(&offsets).enumerate() {
        let dangling = TransformError::DanglingReference { body: k };
        // An id of this body in the merged one; an error when the body
        // has no such entity.
        let shift = |index: u32, count: usize, offset: u32| {
            if (index as usize) < count {
                // Cannot overflow: index < count, and offset + count fits.
                index.checked_add(offset).ok_or(TransformError::TooLarge)
            } else {
                Err(dangling.clone())
            }
        };
        let vertex = |v: VertexId| shift(v.0, body.vertices.len(), offset.vertices).map(VertexId);
        merged.vertices.extend(body.vertices.iter().cloned());
        for edge in &body.edges {
            merged.edges.push(crate::Edge {
                curve: edge.curve.clone(),
                range: edge.range,
                start: vertex(edge.start)?,
                end: vertex(edge.end)?,
            });
        }
        for loop_ in &body.loops {
            let mut coedges = Vec::with_capacity(loop_.coedges.len());
            for coedge in &loop_.coedges {
                coedges.push(Coedge {
                    edge: EdgeId(shift(coedge.edge.0, body.edges.len(), offset.edges)?),
                    reversed: coedge.reversed,
                });
            }
            merged.loops.push(Loop { coedges });
        }
        for face in &body.faces {
            let mut loops = Vec::with_capacity(face.loops.len());
            for loop_id in &face.loops {
                loops.push(LoopId(shift(loop_id.0, body.loops.len(), offset.loops)?));
            }
            merged.faces.push(Face {
                surface: face.surface.clone(),
                reversed: face.reversed,
                loops,
            });
        }
        for shell in &body.shells {
            let mut faces = Vec::with_capacity(shell.faces.len());
            for face_id in &shell.faces {
                faces.push(FaceId(shift(face_id.0, body.faces.len(), offset.faces)?));
            }
            merged.shells.push(Shell { faces });
        }
    }
    Ok((merged, offsets))
}
