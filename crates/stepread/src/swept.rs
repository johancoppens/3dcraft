//! Swept and offset surfaces (a curve extruded or turned, a surface
//! offset) as the kinds of `craft3d-geom`, whatever file they come from:
//! STEP (`SURFACE_OF_LINEAR_EXTRUSION`, `SURFACE_OF_REVOLUTION`,
//! `OFFSET_SURFACE`) or a FreeCAD document's BRep (surfaces 6, 7, 11).
//! A swept surface that is an elementary one is read as that, and
//! whether its normal (`∂u × ∂v` of the swept surface's own
//! parametrisation: `u` along the curve and `v` along the direction for
//! an extrusion, `u` round the axis and `v` along the curve for a
//! revolution, in both formats) is the elementary surface's or the
//! opposite is measured at a point of it.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]
use crate::geometry::{as_nurbs, extruded, revolved};
use craft3d_geom::{Cone, Curve3, Cylinder, NurbsCurve, Plane, Sphere, Surface, Torus};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use crate::spindle::{Normal, Sheet, Spindle, flipped};
use craft3d_topo::{Body, LoopId};

/// An extrusion as read: done, with whether the file's normal is
/// opposite to the surface; or an extrusion of a curve that is no
/// elementary kind, which becomes a B-spline surface once the faces on
/// it say how far it reaches ([`bounded_extrusion`]).
#[derive(Clone, Debug)]
pub enum Swept {
    Done(Surface, bool),
    Unbounded {
        curve: NurbsCurve,
        along: Vec3,
    },
    /// A horn or spindle torus: which of its parts a face is on, its edges
    /// tell ([`spindle_part`]).
    Spindle(Spindle),
}

/// Whether `normal`, the file's normal at `point`, is opposite to
/// `surface`'s normal there.
pub fn opposite(surface: &Surface, point: Vec3, normal: Vec3) -> Result<bool, String> {
    let ours = surface
        .project(point)
        .ok()
        .and_then(|(u, v)| surface.normal(u, v));
    match ours {
        Some(ours) if ours.dot(normal).abs() > 1e-9 * normal.length() => Ok(ours.dot(normal) < 0.0),
        _ => Err("cannot tell which way the swept surface's normal points".to_string()),
    }
}

/// Points along a curve where its derivative is not zero, for the
/// normal of a surface swept from it.
fn samples(curve: &Curve3) -> Vec<(Vec3, Vec3)> {
    let (t0, t1) = match curve.domain() {
        (a, b) if a.is_finite() && b.is_finite() => (a, b),
        _ => (0.0, 1.0),
    };
    [0.37, 0.61, 0.13, 0.89]
        .iter()
        .map(|s| curve.derivatives(t0 + (t1 - t0) * s))
        .collect()
}

/// `SURFACE_OF_LINEAR_EXTRUSION(name, swept_curve, extrusion_axis)`:
/// `C(u) + v V`, normal `C'(u) × V`. A line extruded is a plane, a
/// circle along its axis a cylinder; any other curve (a circle
/// slantwise, an ellipse, a polyline, a B-spline) is the exact
/// B-spline surface of degree one across, once the faces on it say
/// how far it reaches ([`Read::Extrusion`], [`extruded`]).
pub fn extrusion(curve: &Curve3, along: Vec3) -> Result<Swept, String> {
    let surface = match curve {
        Curve3::Line(line) => {
            let normal = line.direction().cross(along);
            let frame = Frame::new(line.origin(), normal, line.direction());
            match frame {
                Some(frame) => Surface::Plane(Plane::new(frame)),
                None => return Err("a line extruded along itself".to_string()),
            }
        }
        Curve3::Circle(circle)
            if circle.frame().z().cross(along).length() <= 1e-9 * along.length() =>
        {
            let cylinder =
                Cylinder::new(*circle.frame(), circle.radius()).map_err(|e| e.to_string())?;
            Surface::Cylinder(cylinder)
        }
        other => {
            let Some(nurbs) = as_nurbs(other) else {
                return Err(format!("an extrusion of a {} is not read", other.kind()));
            };
            // The B-spline's own parametrisation is the file's (u
            // along the curve, v along V), so its normal is the
            // file's.
            return Ok(Swept::Unbounded {
                curve: nurbs,
                along,
            });
        }
    };
    for (point, tangent) in samples(curve) {
        let normal = tangent.cross(along);
        if normal.length() > 0.0 && normal.length().is_finite() {
            return Ok(Swept::Done(
                surface.clone(),
                opposite(&surface, point, normal)?,
            ));
        }
    }
    Err("the swept curve has no direction".to_string())
}

/// `SURFACE_OF_REVOLUTION(name, swept_curve, axis_position)`: the curve
/// turned by `u` about the axis, `v` the curve's parameter; normal
/// `∂u × ∂v`, where `∂u = a × (C(v) − L)` at `u = 0`. A line or a
/// circle in a plane through the axis turns into one of the elementary
/// surfaces; any other curve (a B-spline, an ellipse, a circle out of
/// such a plane or one whose torus would reach the axis) into the
/// exact rational B-spline surface ([`revolved`]).
/// A circle whose torus reaches the axis is a spindle or horn torus
/// ([`Spindle`]), whose part a face chooses.
pub fn revolution(curve: &Curve3, at: Vec3, axis: Vec3) -> Result<Swept, String> {
    let geom = |e: craft3d_geom::GeomError| e.to_string();
    // Small compared with the size of what is turned.
    let scale = |p: Vec3| (p - at).length().max(1e-300);
    let elementary = match curve {
        Curve3::Line(line) => {
            let d = line.direction();
            let p = line.origin();
            let off = p - at;
            if off.dot(axis.cross(d)).abs() > 1e-9 * scale(p) {
                return Err(
                    "a line turned about an axis it does not meet (a hyperboloid) is not read"
                        .to_string(),
                );
            }
            let radial = off - axis * off.dot(axis);
            if axis.cross(d).length() <= 1e-12 {
                let radius = radial.length();
                let frame =
                    Frame::new(at, axis, radial).ok_or_else(|| "a line on its axis".to_string())?;
                Some(Surface::Cylinder(
                    Cylinder::new(frame, radius).map_err(geom)?,
                ))
            } else if axis.dot(d).abs() <= 1e-12 {
                let frame = Frame::from_z(at + axis * off.dot(axis), axis)
                    .ok_or_else(|| "no plane".to_string())?;
                Some(Surface::Plane(Plane::new(frame)))
            } else {
                // The apex: where the line meets the axis.
                let along = d.dot(axis);
                let t = -radial.dot(d - axis * along) / (d - axis * along).length_squared();
                let apex_on_line = p + d * t;
                let apex = at + axis * (apex_on_line - at).dot(axis);
                // The nappe the line's own point is on (a cone here
                // ends at its apex).
                let far = if (p - apex_on_line).length() > 1e-9 * scale(p) {
                    p
                } else {
                    p + d * scale(p).max(1.0)
                };
                let height = (far - apex).dot(axis);
                let reach = (far - apex - axis * height).length();
                let half_angle = (reach / height).patan();
                let frame = Frame::from_z(apex, axis).ok_or_else(|| "no axis".to_string())?;
                Some(Surface::Cone(
                    Cone::new(frame, 0.0, half_angle).map_err(geom)?,
                ))
            }
        }
        Curve3::Circle(circle) => {
            let frame = circle.frame();
            let centre = frame.origin();
            let off = centre - at;
            let size = circle.radius() + off.length();
            if frame.z().dot(axis).abs() > 1e-9 || off.dot(frame.z()).abs() > 1e-9 * size {
                // Out of every plane through the axis.
                None
            } else {
                let foot = at + axis * off.dot(axis);
                let major = (centre - foot).length();
                if major <= 1e-12 * size {
                    let frame = Frame::from_z(centre, axis).ok_or_else(|| "no axis".to_string())?;
                    Some(Surface::Sphere(
                        Sphere::new(frame, circle.radius()).map_err(geom)?,
                    ))
                } else {
                    let frame =
                        Frame::new(foot, axis, centre - foot).ok_or_else(|| "no axis".to_string())?;
                    if major > circle.radius() {
                        Some(Surface::Torus(
                            Torus::new(frame, major, circle.radius()).map_err(geom)?,
                        ))
                    } else {
                        // A horn or spindle torus: the file's normal
                        // against the standard torus's, at a point of the
                        // circle clear of the axis.
                        let flip = samples(curve).into_iter().find_map(|(point, tangent)| {
                            let normal = axis.cross(point - at).cross(tangent);
                            flipped(&frame, major, point, normal)
                        });
                        let Some(flip) = flip else {
                            return Err(
                                "cannot tell which way the swept surface's normal points"
                                    .to_string(),
                            );
                        };
                        return Spindle::toroidal(
                            frame,
                            major,
                            circle.radius(),
                            None,
                            Normal::Swept { flip },
                        )
                        .and_then(Spindle::read);
                    }
                }
            }
        }
        _ => None,
    };
    let surface = match elementary {
        Some(surface) => surface,
        None => {
            let Some(nurbs) = as_nurbs(curve) else {
                return Err(format!("a revolution of a {} is not read", curve.kind()));
            };
            // u round the axis as the angle runs, v along the curve:
            // the file's normal, whatever the parametrisation of u.
            let surface = revolved(&nurbs, at, axis).map_err(|e| e.to_string())?;
            return Ok(Swept::Done(Surface::Nurbs(surface), false));
        }
    };
    for (point, tangent) in samples(curve) {
        let du = axis.cross(point - at);
        let normal = du.cross(tangent);
        if normal.length() > 1e-9 * scale(point) * tangent.length() {
            return Ok(Swept::Done(
                surface.clone(),
                opposite(&surface, point, normal)?,
            ));
        }
    }
    Err("cannot tell which way the swept surface's normal points".to_string())
}

/// The surface at `distance` from `surface` along the file's normal
/// (`surface`'s own, or its opposite when `opposite`): an offset of a
/// plane, a cylinder, a cone, a sphere or a torus is the same kind of
/// surface again (a plane moved, a radius changed); of a B-spline it is
/// no kind here and is refused.
/// A ring torus offset until its tube reaches the axis is a horn or
/// spindle torus.
pub fn offset(surface: &Surface, opposite: bool, distance: f64) -> Result<Swept, String> {
    let geom = |e: craft3d_geom::GeomError| e.to_string();
    // The file's normal at a point of the basis, and whether it points
    // away from the axis or centre (`outward` of that point).
    let file_normal = |u: f64, v: f64| -> Result<Vec3, String> {
        let ours = surface
            .normal(u, v)
            .ok_or_else(|| "the basis has no normal".to_string())?;
        Ok(if opposite { -ours } else { ours })
    };
    let gone = || "the offset goes through the basis's axis or centre; it is not read".to_string();
    let moved = match surface {
        Surface::Plane(plane) => {
            let n = file_normal(0.0, 0.0)?;
            let frame = plane.frame();
            let frame = frame.at(frame.origin() + n * distance).ok_or_else(gone)?;
            Surface::Plane(Plane::new(frame))
        }
        Surface::Cylinder(c) => {
            let p = surface.eval(0.0, 0.0);
            let radial = p
                - c.frame().origin()
                - c.frame().z() * (p - c.frame().origin()).dot(c.frame().z());
            let sign = file_normal(0.0, 0.0)?.dot(radial).signum();
            let radius = c.radius() + sign * distance;
            if !(radius > 0.0) {
                return Err(gone());
            }
            Surface::Cylinder(Cylinder::new(*c.frame(), radius).map_err(geom)?)
        }
        Surface::Sphere(s) => {
            let p = surface.eval(0.0, 0.0);
            let sign = file_normal(0.0, 0.0)?.dot(p - s.frame().origin()).signum();
            let radius = s.radius() + sign * distance;
            if !(radius > 0.0) {
                return Err(gone());
            }
            Surface::Sphere(Sphere::new(*s.frame(), radius).map_err(geom)?)
        }
        Surface::Torus(t) => {
            let (major, minor) = t.radii();
            // The tube's centre line at u = 0 is the frame's x axis
            // at the major radius.
            let p = surface.eval(0.0, 0.0);
            let centre = t.frame().point(major, 0.0, 0.0);
            let sign = file_normal(0.0, 0.0)?.dot(p - centre).signum();
            let minor = minor + sign * distance;
            if !(minor > 0.0) {
                return Err(gone());
            }
            if minor >= major {
                // The tube now reaches the axis: a horn or spindle
                // torus, both of whose parts are where the ring's
                // points went, along the same normal.
                let sheet = Sheet {
                    minor,
                    outward: sign > 0.0,
                };
                return Spindle {
                    frame: *t.frame(),
                    major,
                    apple: sheet,
                    lemon: sheet,
                    select: None,
                }
                .offset(0.0);
            }
            Surface::Torus(Torus::new(*t.frame(), major, minor).map_err(geom)?)
        }
        Surface::Cone(c) => {
            // A point of the cone's own nappe (where its radius is
            // positive: past the apex in the direction it opens), its
            // normal's part away from the axis: the radius at v = 0
            // grows by d / cos α.
            let v = c.apex_v() + c.half_angle().ptan().signum();
            let p = surface.eval(0.0, v);
            let o = c.frame().origin();
            let radial = p - o - c.frame().z() * (p - o).dot(c.frame().z());
            let sign = file_normal(0.0, v)?.dot(radial).signum();
            let radius = c.radius() + sign * distance / c.half_angle().pcos();
            if !(radius >= 0.0) {
                return Err(gone());
            }
            Surface::Cone(Cone::new(*c.frame(), radius, c.half_angle()).map_err(geom)?)
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            return Err(
                "an offset of a B-spline surface is not read (no exact surface of that kind)"
                    .to_string(),
            );
        }
    };
    Ok(Swept::Done(moved, opposite))
}

/// The B-spline surface of an extrusion over as much of its direction
/// as the face's edges reach: `v` of a point `X = C(u) + v V` lies
/// between `(X·V − max P·V) / |V|²` and `(X·V − min P·V) / |V|²` over
/// the control points `P` of the curve (their hull holds the curve),
/// taken over points along every edge and widened by a quarter and
/// one unit of `V` each way (past its ends the surface goes on as the
/// straight lines it is, so the width only keeps projections inside).
pub fn bounded_extrusion(
    body: &Body,
    curve: &NurbsCurve,
    along: Vec3,
    loops: &[LoopId],
) -> Result<Surface, String> {
    let length = along.length_squared();
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in curve.control_points() {
        let x = p.dot(along);
        (low, high) = (low.min(x), high.max(x));
    }
    let (mut v0, mut v1) = (f64::INFINITY, f64::NEG_INFINITY);
    for l in loops {
        let Some(l) = body.loop_(*l) else {
            continue;
        };
        for c in &l.coedges {
            let Some(edge) = body.edge(c.edge) else {
                continue;
            };
            let (t0, t1) = edge.range;
            for k in 0..=32 {
                let x = edge
                    .curve
                    .eval(t0 + (t1 - t0) * f64::from(k) / 32.0)
                    .dot(along);
                v0 = v0.min((x - high) / length);
                v1 = v1.max((x - low) / length);
            }
        }
    }
    if !(v0.is_finite() && v1.is_finite() && v1 >= v0 && length > 0.0) {
        return Err(
            "an extrusion without bounding edges, or edges that are not finite".to_string(),
        );
    }
    let margin = 0.25 * (v1 - v0) + 1.0;
    extruded(curve, along, (v0 - margin, v1 + margin))
        .map(Surface::Nurbs)
        .map_err(|e| e.to_string())
}

/// The part of a horn or spindle torus a face is on, measured on its
/// edges (33 points along each), and whether the file's normal is
/// opposite to its surface's.
pub fn spindle_part(
    body: &Body,
    spindle: &Spindle,
    loops: &[LoopId],
    tolerance: f64,
) -> Result<(Surface, bool), String> {
    if loops.is_empty() {
        return Err(
            "a face without an edge loop on a torus whose tube reaches its axis (a spindle or horn torus) is not read"
                .to_string(),
        );
    }
    let mut points = Vec::new();
    for l in loops {
        let Some(l) = body.loop_(*l) else {
            continue;
        };
        for c in &l.coedges {
            let Some(edge) = body.edge(c.edge) else {
                continue;
            };
            let (t0, t1) = edge.range;
            for k in 0..=32 {
                points.push(edge.curve.eval(t0 + (t1 - t0) * f64::from(k) / 32.0));
            }
        }
    }
    spindle
        .sheet_of(&points, tolerance)
        .and_then(|outer| spindle.surface(outer))
}
