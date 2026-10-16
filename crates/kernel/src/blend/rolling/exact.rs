//! Closed forms as exact surfaces of the one blend construction.
//!
//! **The surface is chosen by a symmetry of the configuration.** When one
//! motion carries the edge and both of its faces into themselves, every
//! section of the blend (a fillet's arc, a chamfer's chord) is the first
//! one moved by it, and the blend face is that section swept by the
//! motion: in closed form. A translation along a straight edge (planes
//! and cylinders parallel to it) sweeps a fillet's arc into a cylinder
//! and a chamfer's chord into a plane. Which surface results is decided
//! by the faces, not by the kind of the edge alone.
//!
//! **The edges in closed form where their two surfaces meet in closed
//! form.** Each edge of the blend face lies on two surfaces, the blend's
//! and the face across it: where `geom::intersect` gives the curve in
//! which those meet as a line, a circle or an ellipse that passes through
//! the edge's two vertices and its fitted curve's middle, the edge is that
//! curve between them; otherwise the fitted curve stays, which is proven
//! on both surfaces already.

use super::{Section, Shape, Side, Spine};
use crate::blend::{BlendError, lost};
use craft3d_geom::intersect::{SurfaceIntersection, intersect_surfaces};
use craft3d_geom::{Curve3, Cylinder, Line, Plane, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId};

/// Sine below which two directions are taken as parallel (or a
/// direction as lying in a plane) for a symmetry: what rounding leaves
/// of exactly parallel, far below any tolerance.
const PARALLEL: f64 = 1e-9;

/// A motion that carries the edge and both faces into themselves.
#[derive(Clone, Copy, Debug)]
pub(super) enum Motion {
    /// A translation along this unit direction.
    Translation(Vec3),
}

/// The motion under which the spine and both sides are invariant, if
/// there is one.
pub(super) fn motion(spine: &Spine<'_>, sides: [Side<'_>; 2]) -> Option<Motion> {
    let Curve3::Line(line) = spine.curve else {
        return None;
    };
    if spine.closed || spine.joints.iter().any(Option::is_some) {
        return None;
    }
    let along = line.direction();
    let invariant = |surface: &Surface| match surface {
        Surface::Plane(plane) => plane.frame().z().dot(along).abs() <= PARALLEL,
        Surface::Cylinder(cylinder) => cylinder.frame().z().cross(along).length() <= PARALLEL,
        Surface::Cone(_)
        | Surface::Sphere(_)
        | Surface::Torus(_)
        | Surface::Bezier(_)
        | Surface::Nurbs(_) => false,
    };
    sides
        .iter()
        .all(|side| invariant(side.surface))
        .then_some(Motion::Translation(along))
}

/// The sections of a blend under a motion: the seed, moved.
#[derive(Clone, Copy, Debug)]
pub(super) struct Moved {
    pub motion: Motion,
    seed: Section,
    /// The seed's place along the spine.
    at: f64,
    /// How far the motion moves per unit of the spine's parameter.
    per: Vec3,
}

impl Moved {
    pub(super) fn new(motion: Motion, spine: &Spine<'_>, seed: Section, at: f64) -> Option<Moved> {
        let Motion::Translation(_) = motion;
        let per = spine.plain(1.0)?.point - spine.plain(0.0)?.point;
        per.is_finite().then_some(Moved {
            motion,
            seed,
            at,
            per,
        })
    }

    /// The section at `s`: the seed moved there, with the faces'
    /// parameters of its feet found again.
    pub(super) fn at(&self, s: f64, sides: [Side<'_>; 2]) -> Option<Section> {
        let shift = self.per * (s - self.at);
        let mut section = self.seed;
        for (k, side) in sides.iter().enumerate() {
            let foot = section.feet.get_mut(k)?;
            *foot += shift;
            *section.params.get_mut(k)? = side.surface.project(*foot).ok()?;
        }
        section.centre = section.centre.map(|c| c + shift);
        Some(section)
    }
}

/// The section swept by the motion: the exact surface of the blend face.
pub(super) fn swept(motion: Motion, section: &Section, shape: Shape) -> Option<Surface> {
    let Motion::Translation(along) = motion;
    let [q1, q2] = section.feet;
    match (shape, section.centre) {
        (Shape::Ball { radius, .. }, Some(centre)) => {
            let frame = Frame::new(centre, along, q1 - centre)?;
            Cylinder::new(frame, radius).ok().map(Surface::Cylinder)
        }
        (Shape::Cut { .. }, _) => {
            let normal = (q2 - q1).cross(along).normalized()?;
            Some(Surface::Plane(Plane::new(Frame::new(q1, normal, along)?)))
        }
        (Shape::Ball { .. }, None) => None,
    }
}

/// Gives the blend face `face` of `body` the exact `surface` (the same
/// surface as its fitted one, within rounding), keeping which way is out,
/// and each of its edges the closed-form curve in which its two surfaces
/// meet, where there is one through its vertices.
pub(super) fn make_exact(
    body: &mut Body,
    face: FaceId,
    surface: Surface,
    tolerance: &Tolerance,
) -> Result<(), BlendError> {
    let own = body.face(face).ok_or_else(lost)?;
    // Which way is out, at a point of the face: the middle of its first
    // edge, moved nowhere (it lies on both surfaces).
    let first = own
        .loops
        .first()
        .and_then(|l| body.loop_(*l))
        .and_then(|l| l.coedges.first())
        .ok_or_else(lost)?
        .edge;
    let data = body.edge(first).ok_or_else(lost)?;
    let p = data.curve.eval(0.5 * (data.range.0 + data.range.1));
    let normal_of = |s: &Surface| -> Option<Vec3> {
        let (u, v) = s.project(p).ok()?;
        s.normal(u, v)
    };
    let fitted = normal_of(&own.surface).ok_or_else(lost)?;
    let outward = if own.reversed { -fitted } else { fitted };
    let exact = normal_of(&surface).ok_or_else(lost)?;
    if !(exact.cross(outward).length() <= 1e-6) {
        // Not the same surface: keep the fitted one.
        return Ok(());
    }
    let reversed = exact.dot(outward) < 0.0;
    // The edges of the face, with the face across each.
    let uses = crate::blend::edge_uses(body);
    let mut edges: Vec<(EdgeId, FaceId)> = Vec::new();
    for l in &own.loops {
        for coedge in &body.loop_(*l).ok_or_else(lost)?.coedges {
            let across = uses
                .get(coedge.edge.0 as usize)
                .into_iter()
                .flatten()
                .map(|u| u.face)
                .find(|f| *f != face);
            if let Some(across) = across {
                edges.push((coedge.edge, across));
            }
        }
    }
    for (edge, across) in edges {
        let other = &body.face(across).ok_or_else(lost)?.surface;
        let data = body.edge(edge).ok_or_else(lost)?;
        let (Some(from), Some(to)) = (
            body.vertex(data.start).map(|v| v.point),
            body.vertex(data.end).map(|v| v.point),
        ) else {
            return Err(lost());
        };
        let through = data.curve.eval(0.5 * (data.range.0 + data.range.1));
        if let Some((curve, range)) = closed_form(&surface, other, [from, through, to], tolerance)
            && let Some(slot) = body.edges.get_mut(edge.0 as usize)
        {
            slot.curve = curve;
            slot.range = range;
        }
    }
    let slot = body.faces.get_mut(face.0 as usize).ok_or_else(lost)?;
    slot.surface = surface;
    slot.reversed = reversed;
    Ok(())
}

/// The curve in which `a` and `b` meet in closed form that runs from
/// `points[0]` through `points[1]` to `points[2]` (each within the
/// tolerance of it), turned to run that way, and its range.
fn closed_form(
    a: &Surface,
    b: &Surface,
    [from, through, to]: [Vec3; 3],
    tolerance: &Tolerance,
) -> Option<(Curve3, (f64, f64))> {
    let curves = match intersect_surfaces(a, b, tolerance).ok()? {
        SurfaceIntersection::Curves(list) | SurfaceIntersection::Tangent(list) => list,
        SurfaceIntersection::None | SurfaceIntersection::Coincident { .. } => return None,
    };
    let tol = tolerance.linear;
    let on = |curve: &Curve3, p: Vec3| -> Option<f64> {
        let t = curve.project(p).ok()?;
        (curve.eval(t).distance(p) <= tol).then_some(t)
    };
    for curve in curves {
        let (Some(_), Some(_), Some(_)) = (on(&curve, from), on(&curve, through), on(&curve, to))
        else {
            continue;
        };
        for turned in [curve.clone(), reversed(&curve)?] {
            let (Some(t0), Some(tm), Some(t1)) =
                (on(&turned, from), on(&turned, through), on(&turned, to))
            else {
                continue;
            };
            let range = match turned.period() {
                None => (t1 > tm && tm > t0).then_some((t0, t1)),
                Some(period) => {
                    let up = |t: f64| t0 + (t - t0).rem_euclid(period);
                    let (tm, t1) = (up(tm), up(t1));
                    (t1 > tm && tm > t0).then_some((t0, t1))
                }
            };
            if let Some(range) = range {
                return Some((turned, range));
            }
        }
    }
    None
}

/// `curve` run the other way round.
fn reversed(curve: &Curve3) -> Option<Curve3> {
    Some(match curve {
        Curve3::Line(line) => Curve3::Line(Line::new(line.origin(), -line.direction()).ok()?),
        Curve3::Circle(circle) => {
            let f = circle.frame();
            Curve3::Circle(
                craft3d_geom::Circle::new(Frame::new(f.origin(), -f.z(), f.x())?, circle.radius())
                    .ok()?,
            )
        }
        Curve3::Ellipse(ellipse) => {
            let f = ellipse.frame();
            let (a, b) = ellipse.semi_axes();
            Curve3::Ellipse(
                craft3d_geom::Ellipse::new(Frame::new(f.origin(), -f.z(), f.x())?, a, b).ok()?,
            )
        }
        Curve3::Nurbs(_) => return None,
    })
}
