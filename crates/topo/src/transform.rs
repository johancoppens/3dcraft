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
        // Distances, which an isometry keeps; the ids are the same.
        body.tolerances = self.tolerances.clone();
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
    ///   sphere or torus ([`Body::held_bbox`] does better), a face on a
    ///   Bézier or NURBS surface the box of the whole control net, however
    ///   little of the surface the face uses.
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
        self.boxed(Round::Whole)
    }

    /// A box that certainly contains the body, as [`Body::bbox`] but with
    /// a face on a sphere or a torus boxed by what it holds: such a face
    /// reaches past its boundary along an axis only at a point where the
    /// axis is normal to the surface (two on a sphere, four on a torus, or
    /// a circle along the top or the bottom of its tube), and each such
    /// point that the boundary does not already reach adds its coordinate
    /// along that axis unless it is certainly not in the face. A face
    /// without loops is the whole surface. So the box is the tight one for
    /// a body whose faces are planes, cylinders, cones, spheres and tori
    /// and whose edges are lines, circles and ellipses, wherever the test
    /// that a point is not in a face gives an answer.
    ///
    /// That test is not a proof either: whether a point of a sphere or a
    /// torus is in a face is decided by walking from it along circles of
    /// the surface to the first place where the face's boundary crosses
    /// them (found on sampled edges and then by regula falsi), whose side
    /// tells; where no walk tells for certain the point is included. It
    /// costs about a tenth of a millisecond per such face whose boundary
    /// does not already reach as far, which is why [`Body::bbox`], which
    /// the booleans call on every operand, does not do it (and why its
    /// looser box, which the booleans' margins are measured against, stays
    /// what it was). Errors as [`Body::bbox`].
    pub fn held_bbox(&self) -> Result<Bbox3, TransformError> {
        self.boxed(Round::Held)
    }

    fn boxed(&self, round: Round) -> Result<Bbox3, TransformError> {
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
            face_extent(self, face, face_id, round, &mut extent)?;
            if !extent.finite {
                return Err(TransformError::Face {
                    face: face_id,
                    error: GeomError::InvalidParameter("the surface is not finite"),
                });
            }
        }
        Ok(extent.padded())
    }

    /// A box that certainly contains the edge `id` and its vertices, by
    /// the rules of [`Body::bbox`]. An error for an edge or a vertex that
    /// does not exist, and as [`Body::bbox`].
    pub fn edge_bbox(&self, id: EdgeId) -> Result<Bbox3, TransformError> {
        let Some(edge) = self.edge(id) else {
            return Err(TransformError::DanglingReference { body: 0 });
        };
        let mut extent = Extent::EMPTY;
        for vertex in [edge.start, edge.end] {
            match self.vertex(vertex) {
                Some(v) if v.point.is_finite() => extent.add(v.point),
                _ => return Err(TransformError::Vertex { vertex }),
            }
        }
        edge_extent(&edge.curve, edge.range, id, &mut extent)?;
        if !extent.finite {
            return Err(TransformError::EdgeRange { edge: id });
        }
        Ok(extent.padded())
    }

    /// A box that certainly contains the loop `id`: its edges and their
    /// vertices, by the rules of [`Body::bbox`]. An error for a loop, an
    /// edge or a vertex that does not exist, and as [`Body::bbox`].
    pub fn loop_bbox(&self, id: LoopId) -> Result<Bbox3, TransformError> {
        let Some(loop_) = self.loop_(id) else {
            return Err(TransformError::DanglingReference { body: 0 });
        };
        let mut extent = Extent::EMPTY;
        for coedge in &loop_.coedges {
            let Some(edge) = self.edge(coedge.edge) else {
                return Err(TransformError::DanglingReference { body: 0 });
            };
            for vertex in [edge.start, edge.end] {
                match self.vertex(vertex) {
                    Some(v) if v.point.is_finite() => extent.add(v.point),
                    _ => return Err(TransformError::Vertex { vertex }),
                }
            }
            edge_extent(&edge.curve, edge.range, coedge.edge, &mut extent)?;
        }
        if !extent.finite {
            return Err(TransformError::EdgeRange {
                edge: loop_.coedges.first().map_or(EdgeId(u32::MAX), |c| c.edge),
            });
        }
        Ok(extent.padded())
    }

    /// A box that certainly contains the face `face`, by the rules of
    /// [`Body::bbox`] applied to the face, the edges of its loops and their
    /// vertices: the box of the body is the union of these. An error for a
    /// face that does not exist or refers to an edge that does not, and as
    /// [`Body::bbox`].
    pub fn face_bbox(&self, face: FaceId) -> Result<Bbox3, TransformError> {
        let missing = TransformError::Face {
            face,
            error: GeomError::InvalidParameter("the face does not exist"),
        };
        let shape = self.face(face).ok_or(missing)?;
        let mut extent = Extent::EMPTY;
        for &loop_id in &shape.loops {
            let Some(loop_) = self.loop_(loop_id) else {
                return Err(TransformError::Face {
                    face,
                    error: GeomError::InvalidParameter("a loop of the face does not exist"),
                });
            };
            for coedge in &loop_.coedges {
                let Some(edge) = self.edge(coedge.edge) else {
                    return Err(TransformError::Face {
                        face,
                        error: GeomError::InvalidParameter("an edge of the face does not exist"),
                    });
                };
                for vertex in [edge.start, edge.end] {
                    match self.vertex(vertex) {
                        Some(v) if v.point.is_finite() => extent.add(v.point),
                        _ => return Err(TransformError::Vertex { vertex }),
                    }
                }
                edge_extent(&edge.curve, edge.range, coedge.edge, &mut extent)?;
            }
        }
        face_extent(self, shape, face, Round::Whole, &mut extent)?;
        if !extent.finite {
            return Err(TransformError::Face {
                face,
                error: GeomError::InvalidParameter("the surface is not finite"),
            });
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

    /// Whether the extent already reaches `value` along `axis`: adding it
    /// would change nothing.
    fn covers(&self, axis: usize, value: f64) -> bool {
        match (self.min.get(axis), self.max.get(axis)) {
            (Some(&min), Some(&max)) => min <= value && value <= max,
            _ => false,
        }
    }

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

/// The points of `torus` where the direction `e` is normal to it: where
/// `e · p` can have an extreme inside a face, in groups of points with the
/// same `e · p`. Generally four groups of one, on the section of the tube
/// in the plane of the axis and `e`. Where `e` is along the axis, `e · p`
/// is greatest and least on the circles along the top and the bottom of
/// the tube, each a group of points spread round it: where one of them is
/// certainly not in a face, either the face holds none of the circle or
/// its boundary meets the circle and so reaches the same `e · p`. They
/// are spread away from angle 0, where a revolution starts and a section
/// plane would hold its edge.
fn torus_extremes(torus: &craft3d_geom::Torus, e: Vec3) -> Vec<Vec<Vec3>> {
    let frame = torus.frame();
    let (major, minor) = torus.radii();
    let (ex, ey, ez) = (
        e.dot(frame.vector(1.0, 0.0, 0.0)),
        e.dot(frame.vector(0.0, 1.0, 0.0)),
        e.dot(frame.z()),
    );
    let flat = ex.hypot(ey);
    let at = |u: f64, v: f64| {
        let (su, cu) = u.sin_cos();
        let (sv, cv) = v.sin_cos();
        let ring = major + minor * cv;
        frame.point(ring * cu, ring * su, minor * sv)
    };
    let mut groups = Vec::with_capacity(4);
    if flat > 0.0 {
        let u = ey.atan2(ex);
        for (u, toward) in [(u, flat), (u + PI, -flat)] {
            let v = ez.atan2(toward);
            groups.push(vec![at(u, v)]);
            groups.push(vec![at(u, v + PI)]);
        }
    } else {
        for v in [FRAC_PI_2, -FRAC_PI_2] {
            groups.push([1.0, 2.5, 4.0, 5.5].map(|u| at(u, v)).to_vec());
        }
    }
    groups
}

/// An arc of a circle in which a plane cuts a face's surface: from
/// `start` counter-clockwise about `normal` round `centre` to `end` (all
/// the way round when there is none). Where the plane cuts the surface in
/// more than this circle, `keep` tells which points are on it (`None`:
/// cannot tell).
struct Arc<'a> {
    start: Vec3,
    end: Option<Vec3>,
    normal: Vec3,
    centre: Vec3,
    keep: Box<dyn Fn(Vec3) -> Option<bool> + 'a>,
}

/// Up to `most` points of the face's boundary: the middles of its first
/// edges, where a path that ends on one certainly crosses the boundary.
fn boundary_marks(body: &Body, face: &Face, most: usize) -> Vec<Vec3> {
    face.loops
        .iter()
        .filter_map(|l| body.loop_(*l))
        .flat_map(|l| l.coedges.iter())
        .filter_map(|c| body.edge(c.edge))
        .map(|e| e.curve.eval(0.5 * (e.range.0 + e.range.1)))
        .filter(|p| p.is_finite())
        .take(most)
        .collect()
}

/// Whether the start of `path` (a point of the face's surface) is in the
/// face, as far as walking along the path tells: `None` where it does not.
///
/// The first place along the path where the face's boundary crosses it
/// tells: the face lies to the left of its boundary seen from outside
/// (`outward` gives the outward normal at a point of the surface), so
/// where it lies beyond the crossing the start is outside, and where it
/// lies back towards the start, inside. Crossings are found on samples of
/// the edges, as in [`reaches_apex`], and then by bisection; one missed in
/// a pair that dips across the plane and back does not change the answer,
/// since along the path the boundary is crossed into and out of the face
/// in turn. No answer where the path meets no boundary, where the first
/// crossing grazes it, is at the start of an arc, or is within rounding of
/// another that says the opposite, or where a crossing is not clearly on
/// the arc's circle or off it.
fn walk(
    body: &Body,
    face: &Face,
    path: &[Arc],
    outward: &dyn Fn(Vec3) -> Option<Vec3>,
) -> Result<Option<bool>, TransformError> {
    let owner = 0;
    let near = 1e-9 * TAU;
    for arc in path {
        let Some(along) = (arc.start - arc.centre).normalized() else {
            return Ok(None);
        };
        let across = arc.normal.cross(along);
        let angle = |p: Vec3| {
            let d = p - arc.centre;
            let a = d.dot(across).atan2(d.dot(along));
            if a <= 0.0 { a + TAU } else { a }
        };
        let reach = arc.end.map_or(TAU, angle);
        // Per crossing: its angle from the start, and whether the start is
        // inside, or `None` where it grazes.
        let mut crossings: Vec<(f64, Option<bool>)> = Vec::new();
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
                // The ends are the vertices, so that where edges meet (a
                // closed edge with itself, too) both see the same side,
                // and a crossing there is counted once.
                let ends = [edge.start, edge.end].map(|v| body.vertex(v).map(|v| v.point));
                let [Some(first), Some(last)] = ends else {
                    return Err(TransformError::DanglingReference { body: owner });
                };
                let side = |p: Vec3| arc.normal.dot(p - arc.start);
                let mut previous: Option<(f64, f64)> = None;
                for k in 0..=count {
                    let t = t0 + (t1 - t0) * (k as f64 / count as f64);
                    let g = side(if k == 0 {
                        first
                    } else if k == count {
                        last
                    } else {
                        edge.curve.eval(t)
                    });
                    if !g.is_finite() {
                        return Ok(None);
                    }
                    let crossed = previous.filter(|&(_, h)| (h < 0.0) != (g < 0.0));
                    previous = Some((t, g));
                    let Some((s, h)) = crossed else {
                        continue;
                    };
                    // Where the side changes, by regula falsi with the
                    // Illinois step (which keeps the bracket shrinking from
                    // both ends), to a bracket as narrow as rounding allows
                    // or 48 steps, so that a crossing is sharp.
                    let (mut low, mut high, mut g_low, mut g_high) = (s, t, h, g);
                    let mut kept_end = 0_i8;
                    for _ in 0..48 {
                        let mut middle = low + (high - low) * g_low / (g_low - g_high);
                        if !(middle > low && middle < high) {
                            middle = 0.5 * (low + high);
                            if !(middle > low && middle < high) {
                                break;
                            }
                        }
                        let g_middle = side(edge.curve.eval(middle));
                        if g_middle == 0.0 || !g_middle.is_finite() {
                            (low, high) = (middle, middle);
                            break;
                        }
                        if (g_middle < 0.0) == (g_low < 0.0) {
                            (low, g_low) = (middle, g_middle);
                            if kept_end == 1 {
                                g_high *= 0.5;
                            }
                            kept_end = 1;
                        } else {
                            (high, g_high) = (middle, g_middle);
                            if kept_end == -1 {
                                g_low *= 0.5;
                            }
                            kept_end = -1;
                        }
                    }
                    let (p, mut tangent) = edge.curve.derivatives(0.5 * (low + high));
                    match (arc.keep)(p) {
                        Some(true) => {}
                        Some(false) => continue,
                        None => return Ok(None),
                    }
                    let at = angle(p);
                    if at > reach + near {
                        continue;
                    }
                    if coedge.reversed {
                        tangent = -tangent;
                    }
                    let Some(normal) = outward(p) else {
                        return Ok(None);
                    };
                    let left = normal.cross(tangent);
                    let onward = arc.normal.cross(p - arc.centre);
                    let ahead = left.dot(onward);
                    let scale = left.length() * onward.length();
                    crossings.push((at, (ahead.abs() > 0.1 * scale).then_some(ahead < 0.0)));
                }
            }
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let Some(&(first, verdict)) = crossings.first() else {
            continue;
        };
        let agreed = crossings
            .iter()
            .take_while(|c| c.0 - first <= near)
            .all(|c| c.1 == verdict);
        if !agreed || first <= near || first >= TAU - near {
            return Ok(None);
        }
        return Ok(verdict);
    }
    Ok(None)
}

/// Whether a point of the face's surface is certainly not in the face:
/// the first of the paths from it that gives an answer ([`walk`]) says so.
/// Errs on the side of no.
fn certainly_outside(
    body: &Body,
    face: &Face,
    paths: &[Vec<Arc>],
    outward: &dyn Fn(Vec3) -> Option<Vec3>,
) -> Result<bool, TransformError> {
    for path in paths {
        if let Some(inside) = walk(body, face, path, outward)? {
            return Ok(!inside);
        }
    }
    Ok(false)
}

/// How many points of a face's boundary the paths from a point to it go
/// to ([`boundary_marks`]).
const MARKS: usize = 3;

/// How a face on a sphere or a torus is boxed: by the whole surface, or by
/// what the face holds ([`Body::held_bbox`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Round {
    Whole,
    Held,
}

fn face_extent(
    body: &Body,
    face: &Face,
    id: FaceId,
    round: Round,
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
            let centre = sphere.frame().origin();
            if face.loops.is_empty() || round == Round::Whole {
                out.add_reach(centre, Vec3::new(r, r, r));
                return Ok(());
            }
            // Along each axis the sphere is furthest at `centre ± r e`,
            // its only points where the axis is normal to it; one is left
            // out where it is certainly not in the face. The paths from it
            // are great circles: through points of the boundary, and two
            // turned away from the coordinate planes and their diagonals
            // (where boundaries lie most often: an octant, a half), since a
            // walk along a boundary that lies in its plane has no answer.
            let sign = if face.reversed { -1.0 } else { 1.0 };
            let outward = |p: Vec3| (p - centre).normalized().map(|n| n * sign);
            let marks = boundary_marks(body, face, MARKS);
            let axes = [Vec3::X, Vec3::Y, Vec3::Z];
            for (i, e) in axes.into_iter().enumerate() {
                let (j, k) = (
                    axes.get((i + 1) % 3).copied().unwrap_or(Vec3::X),
                    axes.get((i + 2) % 3).copied().unwrap_or(Vec3::Y),
                );
                for q in [centre + e * r, centre - e * r] {
                    if components(q).get(i).is_some_and(|&v| out.covers(i, v)) {
                        continue;
                    }
                    let normals = marks
                        .iter()
                        .filter_map(|&b| (q - centre).cross(b - centre).normalized())
                        .chain([0.5_f64, 2.0].map(|a| j * a.cos() + k * a.sin()));
                    let paths: Vec<Vec<Arc>> = normals
                        .map(|normal| {
                            vec![Arc {
                                start: q,
                                end: None,
                                normal,
                                centre,
                                keep: Box::new(|_| Some(true)),
                            }]
                        })
                        .collect();
                    if !certainly_outside(body, face, &paths, &outward)? {
                        out.add_axis(i, components(q).get(i).copied().unwrap_or(f64::NAN));
                    }
                }
            }
        }
        Surface::Torus(torus) => {
            // A circle of radius R about the axis, thickened by r: along a
            // unit direction e it reaches R·|e − (e·z) z| + r.
            let (major, minor) = torus.radii();
            let frame = torus.frame();
            if face.loops.is_empty() || round == Round::Whole {
                let [x, y, z] =
                    components(frame.z()).map(|c| major * (1.0 - c * c).max(0.0).sqrt() + minor);
                out.add_reach(frame.origin(), Vec3::new(x, y, z));
                return Ok(());
            }
            let sign = if face.reversed { -1.0 } else { 1.0 };
            // Away from the tube's middle circle.
            let outward = |p: Vec3| {
                let (x, y, _) = frame.local(p);
                let ring = x.hypot(y);
                let middle = frame.point(major * x / ring, major * y / ring, 0.0);
                (p - middle).normalized().map(|n| n * sign)
            };
            let at = |u: f64, v: f64| {
                let (su, cu) = u.sin_cos();
                let (sv, cv) = v.sin_cos();
                let ring = major + minor * cv;
                frame.point(ring * cu, ring * su, minor * sv)
            };
            // A point's angles about the axis and about the tube.
            let angles = |p: Vec3| {
                let (x, y, z) = frame.local(p);
                (y.atan2(x), z.atan2(x.hypot(y) - major))
            };
            // The circle about the axis through the point at `v`, and the
            // tube's section at `u`. Each plane cuts the torus in two
            // circles; a crossing is on the arc's when it is clearly nearer
            // to it than to the other (within a quarter of their distance
            // apart).
            let parallel = |start: Vec3, end: Option<Vec3>| {
                let (_, v) = angles(start);
                let ring = major + minor * v.cos();
                let other = major - minor * v.cos();
                Arc {
                    start,
                    end,
                    normal: frame.z(),
                    centre: frame.point(0.0, 0.0, minor * v.sin()),
                    keep: Box::new(move |p: Vec3| {
                        let (px, py, _) = frame.local(p);
                        let r = px.hypot(py);
                        let (own, far) = ((r - ring).abs(), (r - other).abs());
                        let apart = 0.25 * (ring - other).abs();
                        if own <= apart && own < far {
                            Some(true)
                        } else if far <= apart && far < own {
                            Some(false)
                        } else {
                            None
                        }
                    }),
                }
            };
            // The two sections are centred `major` either side of the
            // axis, their points at least `major − minor` from it.
            let section = |start: Vec3, end: Option<Vec3>| {
                let (u, _) = angles(start);
                let (su, cu) = u.sin_cos();
                Arc {
                    start,
                    end,
                    normal: frame.vector(-su, cu, 0.0),
                    centre: frame.point(major * cu, major * su, 0.0),
                    keep: Box::new(move |p: Vec3| {
                        let (px, py, _) = frame.local(p);
                        let along = px * cu + py * su;
                        (along.abs() >= 0.5 * (major - minor)).then_some(along > 0.0)
                    }),
                }
            };
            let marks: Vec<(Vec3, (f64, f64))> = boundary_marks(body, face, MARKS)
                .into_iter()
                .map(|b| (b, angles(b)))
                .collect();
            for (axis, e) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
                for group in torus_extremes(torus, e) {
                    // The boundary (added before the faces) may already
                    // reach as far.
                    let reach = group
                        .first()
                        .and_then(|q| components(*q).get(axis).copied());
                    if reach.is_some_and(|value| out.covers(axis, value)) {
                        continue;
                    }
                    let mut left_out = false;
                    for &q in &group {
                        // The whole circles through q, and to each mark a
                        // path along a section and then a circle about the
                        // axis, or the other way round.
                        let (u, v) = angles(q);
                        let mut paths = vec![vec![parallel(q, None)], vec![section(q, None)]];
                        for &(b, (ub, vb)) in &marks {
                            let corner = at(u, vb);
                            paths.push(vec![section(q, Some(corner)), parallel(corner, Some(b))]);
                            let corner = at(ub, v);
                            paths.push(vec![parallel(q, Some(corner)), section(corner, Some(b))]);
                        }
                        if certainly_outside(body, face, &paths, &outward)? {
                            left_out = true;
                            break;
                        }
                    }
                    if !left_out && let Some(q) = group.first() {
                        out.add_axis(axis, components(*q).get(axis).copied().unwrap_or(f64::NAN));
                    }
                }
            }
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
        merged.tolerances.extend_shifted(&body.tolerances, |v| vertex(v), |e| {
            shift(e.0, body.edges.len(), offset.edges).map(EdgeId)
        })?;
    }
    Ok((merged, offsets))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coedge(edge: u32, reversed: bool) -> Coedge {
        Coedge {
            edge: EdgeId(edge),
            reversed,
        }
    }

    #[test]
    fn a_loop_turned_round_starts_with_the_same_edge() {
        let loop_ = Loop {
            coedges: vec![
                coedge(7, false),
                coedge(3, true),
                coedge(5, false),
                coedge(9, false),
            ],
        };
        let turned = turned_round(&loop_);
        assert_eq!(
            turned.coedges,
            [
                coedge(7, true),
                coedge(9, true),
                coedge(5, true),
                coedge(3, false)
            ]
        );
        assert_eq!(turned_round(&turned), loop_);
        // One coedge (a full circle) and none at all.
        let single = Loop {
            coedges: vec![coedge(1, false)],
        };
        assert_eq!(turned_round(&single).coedges, [coedge(1, true)]);
        assert_eq!(turned_round(&Loop::default()), Loop::default());
    }

    #[test]
    fn offsets_that_no_longer_fit_an_id_are_refused() {
        // A body with 2^32 entities cannot be built in a test; the
        // arithmetic that would meet it can.
        let mut body = Body::new(crate::BodyKind::Solid);
        body.add_vertex(Vec3::ZERO);
        let room = Offsets {
            vertices: u32::MAX - 1,
            ..Offsets::default()
        };
        assert_eq!(
            room.after(&body),
            Some(Offsets {
                vertices: u32::MAX,
                ..Offsets::default()
            })
        );
        for field in 0..5 {
            let mut at = Offsets::default();
            let mut body = Body::new(crate::BodyKind::Solid);
            match field {
                0 => {
                    at.vertices = u32::MAX;
                    body.add_vertex(Vec3::ZERO);
                }
                1 => {
                    at.edges = u32::MAX;
                    body.add_edge(
                        Curve3::Line(craft3d_geom::Line::new(Vec3::ZERO, Vec3::X).unwrap()),
                        (0.0, 1.0),
                        VertexId(0),
                        VertexId(0),
                    );
                }
                2 => {
                    at.loops = u32::MAX;
                    body.add_loop(Vec::new());
                }
                3 => {
                    at.faces = u32::MAX;
                    body.add_face(
                        Surface::Plane(craft3d_geom::Plane::new(Frame::WORLD)),
                        false,
                        Vec::new(),
                    );
                }
                _ => {
                    at.shells = u32::MAX;
                    body.add_shell(Vec::new());
                }
            }
            assert_eq!(at.after(&body), None, "field {field}");
        }
    }

    #[test]
    fn extents_remember_anything_not_finite() {
        let mut extent = Extent::EMPTY;
        assert!(extent.padded().is_empty());
        extent.add(Vec3::new(1.0, 2.0, 3.0));
        extent.add(Vec3::new(f64::NAN, 0.0, 0.0));
        extent.add(Vec3::new(-1.0, 0.0, 0.0));
        assert!(!extent.finite);
        let mut extent = Extent::EMPTY;
        extent.add_axis(1, f64::INFINITY);
        assert!(!extent.finite);
        // An axis that does not exist is ignored, not indexed.
        let mut extent = Extent::EMPTY;
        extent.add_axis(7, 1.0);
        assert!(extent.finite && extent.padded().is_empty());
        // At the edge of the representable range the padding is dropped
        // rather than overflowing.
        let mut extent = Extent::EMPTY;
        extent.add(Vec3::new(f64::MAX, 0.0, 0.0));
        let (lo, hi) = extent.padded().corners().unwrap();
        assert!(lo.is_finite() && hi.is_finite());
    }
}
