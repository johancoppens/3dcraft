//! `meet_surfaces`: the intersection of any two surfaces inside a box.

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::clip;
use super::exact::{self, Closed};
use super::field::{Chart, Field};
use super::march::Marcher;
use crate::{Curve3, GeomError, Surface};
use craft3d_math::{Bbox3, Frame, Tolerance, Vec3};
use std::cmp::Ordering;

/// One curve along which two surfaces meet.
#[derive(Clone, Debug, PartialEq)]
pub struct MeetingCurve {
    pub curve: Curve3,
    /// The part of `curve` that is meant: `(t0, t1)` with `t0 < t1`; for a
    /// closed curve one full period. On a circle or ellipse `t1` may exceed
    /// `2π` (a piece across the angle 0).
    pub range: (f64, f64),
    pub closed: bool,
    /// True when the surfaces touch along it without crossing (a cylinder
    /// lying on a plane).
    pub tangent: bool,
    /// True when `curve` is the exact intersection (line, circle, ellipse,
    /// exact rational NURBS conic); false when it is an approximation, and
    /// then `deviation` bounds its distance to both surfaces.
    pub exact: bool,
    pub deviation: f64,
}

/// The curves and points in which two surfaces meet.
///
/// Curves do not cross each other except at their ends: where two branches
/// of the intersection cross (the surfaces are tangent there), every arm is
/// a curve of its own that ends on the crossing point.
#[derive(Clone, Debug, PartialEq)]
pub struct Meeting {
    /// In a deterministic order.
    pub curves: Vec<MeetingCurve>,
    /// Isolated points where the surfaces touch (a sphere resting on a
    /// plane) and no curve passes.
    pub points: Vec<Vec3>,
}

/// How two surfaces meet inside a box.
#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceMeeting {
    /// They do not come within the linear tolerance of each other there.
    None,
    /// They are the same surface. `same_normal` says whether their normals
    /// agree where they coincide.
    Coincident {
        same_normal: bool,
    },
    Meets(Meeting),
}

fn rank(s: &Surface) -> u8 {
    match s {
        Surface::Plane(_) => 0,
        Surface::Cylinder(_) => 1,
        Surface::Cone(_) => 2,
        Surface::Sphere(_) => 3,
        Surface::Torus(_) => 4,
        Surface::Bezier(_) => 5,
        Surface::Nurbs(_) => 6,
    }
}

/// The numbers that define a surface, for a total order on surfaces.
fn key(s: &Surface) -> Vec<f64> {
    fn vec(out: &mut Vec<f64>, v: Vec3) {
        out.extend([v.x, v.y, v.z]);
    }
    fn frame(out: &mut Vec<f64>, f: &Frame) {
        for v in [f.origin(), f.x(), f.y(), f.z()] {
            vec(out, v);
        }
    }
    let mut out = Vec::new();
    match s {
        Surface::Plane(p) => frame(&mut out, p.frame()),
        Surface::Cylinder(c) => {
            frame(&mut out, c.frame());
            out.push(c.radius());
        }
        Surface::Cone(c) => {
            frame(&mut out, c.frame());
            out.extend([c.radius(), c.half_angle()]);
        }
        Surface::Sphere(sp) => {
            frame(&mut out, sp.frame());
            out.push(sp.radius());
        }
        Surface::Torus(t) => {
            frame(&mut out, t.frame());
            out.extend([t.radii().0, t.radii().1]);
        }
        Surface::Bezier(b) => {
            out.extend([b.degree_u() as f64, b.degree_v() as f64]);
            for p in b.control_points() {
                vec(&mut out, *p);
            }
        }
        Surface::Nurbs(n) => {
            out.extend([n.degrees().0 as f64, n.degrees().1 as f64]);
            out.extend(n.knots_u());
            out.extend(n.knots_v());
            for p in n.control_points() {
                vec(&mut out, *p);
            }
            out.extend(n.weights());
        }
    }
    out
}

/// The two surfaces in an order that does not depend on the order they were
/// given in, so that `meet(a, b)` and `meet(b, a)` are the same computation.
fn ordered<'a>(a: &'a Surface, b: &'a Surface) -> (&'a Surface, &'a Surface) {
    let order = rank(a).cmp(&rank(b)).then_with(|| {
        let (ka, kb) = (key(a), key(b));
        ka.iter()
            .zip(&kb)
            .map(|(x, y)| x.total_cmp(y))
            .find(|o| *o != Ordering::Equal)
            .unwrap_or_else(|| ka.len().cmp(&kb.len()))
    });
    if order == Ordering::Greater {
        (b, a)
    } else {
        (a, b)
    }
}

fn check(within: &Bbox3, tol: &Tolerance) -> Result<(), GeomError> {
    if !(tol.linear.is_finite() && tol.linear > 0.0 && tol.angular.is_finite() && tol.angular > 0.0)
    {
        return Err(GeomError::InvalidParameter(
            "a tolerance must be positive and finite",
        ));
    }
    match within.corners() {
        Some((lo, hi)) if lo.is_finite() && hi.is_finite() => Ok(()),
        _ => Err(GeomError::InvalidParameter(
            "the box to intersect in is empty or not finite",
        )),
    }
}

fn finish(closed: Closed, within: &Bbox3, tol: &Tolerance) -> Result<SurfaceMeeting, GeomError> {
    Ok(match closed {
        Closed::None => SurfaceMeeting::None,
        Closed::Coincident { same_normal } => SurfaceMeeting::Coincident { same_normal },
        Closed::Meets { curves, points } => {
            let mut clipped = Vec::new();
            for curve in &curves {
                clipped.extend(clip::clip(curve, within, tol)?);
            }
            let points: Vec<Vec3> = points
                .into_iter()
                .filter(|p| within.contains(*p, tol.linear))
                .collect();
            if clipped.is_empty() && points.is_empty() {
                SurfaceMeeting::None
            } else {
                SurfaceMeeting::Meets(Meeting {
                    curves: clipped,
                    points,
                })
            }
        }
    })
}

/// Which of two surfaces is walked in its parameters: freeform surfaces
/// first (the other is then a distance in closed form), then the kinds
/// whose parametrisation has no pole or apex.
fn chart_rank(s: &Surface) -> u8 {
    match s {
        Surface::Bezier(_) | Surface::Nurbs(_) => 0,
        Surface::Plane(_) => 1,
        Surface::Cylinder(_) => 2,
        Surface::Torus(_) => 3,
        Surface::Sphere(_) => 4,
        Surface::Cone(_) => 5,
    }
}

fn numeric(
    a: &Surface,
    b: &Surface,
    within: &Bbox3,
    tol: &Tolerance,
) -> Result<SurfaceMeeting, GeomError> {
    let (walked, other) = if chart_rank(b) < chart_rank(a) {
        (b, a)
    } else {
        (a, b)
    };
    let field = Field::new(other)?;
    let back = Field::new(walked)?;
    // One lying on the other (a patch of a sphere on that sphere).
    if let Some(same_normal) = Marcher::lies_on(walked, &field, within, tol)
        .or_else(|| Marcher::lies_on(other, &back, within, tol))
    {
        return Ok(SurfaceMeeting::Coincident { same_normal });
    }
    let Some(chart) = Chart::new(walked, within, tol, &field)? else {
        return Ok(SurfaceMeeting::None);
    };
    let meeting = Marcher::new(chart, field, within, tol)?.solve(within)?;
    Ok(if meeting.curves.is_empty() && meeting.points.is_empty() {
        SurfaceMeeting::None
    } else {
        SurfaceMeeting::Meets(meeting)
    })
}

/// Everything of the intersection of `a` and `b` that lies inside `within`.
///
/// Curves are clipped to the box; a curve that leaves it gives one
/// [`MeetingCurve`] per piece inside. The box is what makes unbounded
/// intersections (the hyperbola of a plane and a cone, crossing cylinders)
/// finite. The result does not depend on the order of `a` and `b`.
///
/// Pairs with a closed form (see `docs/architecture.md` §5.1) give exact
/// curves; every other pair is intersected numerically, and fails with
/// [`GeomError::NoConvergence`] or [`GeomError::Unsupported`] rather than
/// return a curve that is incomplete or out of tolerance.
pub fn meet_surfaces(
    a: &Surface,
    b: &Surface,
    within: &Bbox3,
    tolerance: &Tolerance,
) -> Result<SurfaceMeeting, GeomError> {
    check(within, tolerance)?;
    let (a, b) = ordered(a, b);
    match exact::closed_form(a, b, tolerance)? {
        Some(closed) => finish(closed, within, tolerance),
        None => numeric(a, b, within, tolerance),
    }
}

/// [`meet_surfaces`] without its closed forms: the numeric marcher on any
/// pair. For tests that hold the marcher against the exact results.
#[doc(hidden)]
pub fn meet_surfaces_numeric(
    a: &Surface,
    b: &Surface,
    within: &Bbox3,
    tolerance: &Tolerance,
) -> Result<SurfaceMeeting, GeomError> {
    check(within, tolerance)?;
    let (a, b) = ordered(a, b);
    numeric(a, b, within, tolerance)
}
