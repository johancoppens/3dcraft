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
use crate::conic::Conic;
use crate::geometry::{as_nurbs, extruded, revolved};
use crate::spindle::{Normal, Sheet, Spindle, flipped};
use craft3d_geom::{Cone, Curve3, Cylinder, NurbsCurve, Plane, Sphere, Surface, Torus};
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
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
    /// A profile turned about an axis it crosses, in a plane through the
    /// axis: its parts on either side sweep surfaces of their own (the
    /// same one twice, with opposite normals, where they mirror each
    /// other); which part a face is on, its edges tell ([`turned_part`]).
    Turned {
        curve: NurbsCurve,
        at: Vec3,
        axis: Vec3,
    },
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
    // Where the file's normal is measured, when not at `samples`: on a
    // cone, between its apex and the line's point on its own nappe (fixed
    // parameters of the line could lie past the apex at a small scale).
    let mut on_nappe: Option<Vec<(Vec3, Vec3)>> = None;
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
                on_nappe = Some(
                    [0.37, 0.61, 0.13, 0.89]
                        .iter()
                        .map(|f| (apex_on_line + (far - apex_on_line) * *f, d))
                        .collect(),
                );
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
                    let frame = Frame::new(foot, axis, centre - foot)
                        .ok_or_else(|| "no axis".to_string())?;
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
                            return Err("cannot tell which way the swept surface's normal points"
                                .to_string());
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
            if !crossings(&nurbs, at, axis).cuts.is_empty() {
                // Its parts on either side of the axis: the faces choose.
                return Ok(Swept::Turned {
                    curve: nurbs,
                    at,
                    axis,
                });
            }
            // u round the axis as the angle runs, v along the curve:
            // the file's normal, whatever the parametrisation of u.
            let surface = revolved(&nurbs, at, axis).map_err(|e| e.to_string())?;
            return Ok(Swept::Done(Surface::Nurbs(surface), false));
        }
    };
    for (point, tangent) in on_nappe.unwrap_or_else(|| samples(curve)) {
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
    let gone = || {
        format!(
            "the offset by {distance} goes through the {} basis's axis or centre (nothing of the surface is left on its side of it): not read; a file that gives the face's surface as the plain surface it is (or as a B-spline) reads",
            surface.kind()
        )
    };
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
            let alpha = c.half_angle();
            let radius = c.radius() + sign * distance / alpha.pcos();
            if !radius.is_finite() {
                return Err(gone());
            }
            if radius >= 0.0 {
                Surface::Cone(Cone::new(*c.frame(), radius, alpha).map_err(geom)?)
            } else {
                // The offset's apex lies past the frame, where the
                // radius `radius + v tan α` is zero: an offset towards
                // the axis larger than the radius at v = 0 (any such
                // offset of a cone whose frame is at its apex, as a
                // line turned about an axis it meets is read). It is
                // the same cone with its frame there: the part of the
                // offset that lies past its apex is no face's (the
                // offsets of points near the basis's apex, which cross
                // the axis), and an edge there is not proven on it.
                let apex = -radius / alpha.ptan();
                let frame = c
                    .frame()
                    .at(c.frame().point(0.0, 0.0, apex))
                    .ok_or_else(gone)?;
                Surface::Cone(Cone::new(frame, 0.0, alpha).map_err(geom)?)
            }
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

/// How an untrimmed hyperbola or parabola is swept: along a vector, or
/// round an axis through a point (a unit direction).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConicSweep {
    Extrusion(Vec3),
    Revolution { at: Vec3, axis: Vec3 },
}

/// Whether `conic` swept as `sweep` is read, asked before any face is.
/// Refused: an extrusion along a direction in the conic's own plane (it
/// sweeps no surface, only a region of the plane), and a turn about an
/// axis in its plane that the conic crosses or touches (a paraboloid, a
/// hyperboloid of two sheets), which sweeps its surface twice, once with
/// each normal: which half a face is on, and so which way its normal
/// points, is in the face's parameter-space curves, which are not read.
pub fn conic_sweep_check(conic: &Conic, sweep: &ConicSweep) -> Result<(), String> {
    let frame = conic.frame();
    let n = frame.z();
    let centre = frame.origin();
    match *sweep {
        ConicSweep::Extrusion(along) => {
            if !(along.dot(n).abs() > 1e-9 * along.length()) {
                return Err(format!(
                    "a {} extruded along a direction in its own plane sweeps no surface (only a region of the plane): not read",
                    conic.kind()
                ));
            }
        }
        ConicSweep::Revolution { at, axis } => {
            let size = (at - centre).length()
                + match *conic {
                    Conic::Hyperbola { a, b, .. } => a.max(b),
                    Conic::Parabola { focal, .. } => focal.abs(),
                };
            let in_plane = axis.dot(n).abs() <= 1e-9 && (at - centre).dot(n).abs() <= 1e-9 * size;
            if !in_plane {
                return Ok(());
            }
            let Some(across) = n.cross(axis).normalized() else {
                return Err("the axis of the surface of revolution has no direction".to_string());
            };
            // `(λ(u) − at)·across`, the signed distance from the axis in
            // the plane, as a quadratic: in `e^u` for a hyperbola (times
            // `2 e^u`), in `u` for a parabola.
            let alpha = (centre - at).dot(across);
            let crosses = match *conic {
                Conic::Hyperbola { a, b, .. } => {
                    let (beta, gamma) = (a * frame.x().dot(across), b * frame.y().dot(across));
                    has_root(beta + gamma, 2.0 * alpha, beta - gamma, true)
                }
                Conic::Parabola { focal, .. } => has_root(
                    focal * frame.x().dot(across),
                    2.0 * focal * frame.y().dot(across),
                    alpha,
                    false,
                ),
            };
            if crosses {
                return Err(format!(
                    "a {} turned about an axis in its plane that it crosses sweeps its surface twice, once with each normal, and which half a face is on is in its parameter-space curves, which are not read: not read (a file that gives the profile as a trimmed curve on one side of the axis, or the surface as a B-spline, reads)",
                    conic.kind()
                ));
            }
        }
    }
    Ok(())
}

/// Whether `a2 t² + a1 t + a0` has a real root (a positive one when
/// `positive`), a double root counting.
fn has_root(a2: f64, a1: f64, a0: f64, positive: bool) -> bool {
    let size = a2.abs().max(a1.abs()).max(a0.abs());
    if !(size > 0.0) {
        return true;
    }
    if a2.abs() <= 1e-12 * size {
        if a1.abs() <= 1e-12 * size {
            return a0.abs() <= 1e-12 * size;
        }
        return !positive || -a0 / a1 > 0.0;
    }
    let d = a1 * a1 - 4.0 * a2 * a0;
    if d < -1e-12 * (a1 * a1).max((4.0 * a2 * a0).abs()) {
        return false;
    }
    let s = d.max(0.0).sqrt();
    let (t1, t2) = ((-a1 - s) / (2.0 * a2), (-a1 + s) / (2.0 * a2));
    !positive || t1 > 0.0 || t2 > 0.0
}

/// Points along every edge of `loops`, `per_edge + 1` each.
fn edge_points(body: &Body, loops: &[LoopId], per_edge: u32) -> Vec<Vec3> {
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
            for k in 0..=per_edge {
                points.push(
                    edge.curve
                        .eval(t0 + (t1 - t0) * f64::from(k) / f64::from(per_edge)),
                );
            }
        }
    }
    points
}

/// The exact B-spline surface of `conic` swept as `sweep` (checked by
/// [`conic_sweep_check`]), over as much of the conic as the faces' edges
/// in `loops` reach, its parameters running as the file's do (`u` along
/// the conic and `v` along the direction for an extrusion, `u` round the
/// axis and `v` along the conic for a revolution), so its normal is the
/// file's.
///
/// *Extruded:* a point `X = λ(u) + vV` of the surface is moved along `V`
/// into the conic's plane, where its `u` is its foot's; the range of `u`
/// over 33 points per edge, widened by a quarter (at most one unit, at
/// least a thousandth), is the arc, extruded as a spline is
/// ([`bounded_extrusion`]). *Turned:* a turn keeps the distance from a
/// point `L` of the axis, so a face whose edges lie within `R` of `L`
/// lies on the turn of the part of the conic within `R` of `L`, which lies
/// within `|u| ≤ U`: on a hyperbola `a cosh u ≤ |λ(u) − C| ≤ R + |C − L|`
/// and `b |sinh u| ≤ R + |C − L|`, on a parabola `|F| u² ≤ …` and
/// `2 |F u| ≤ …` (`R` taken a quarter larger than the points show).
pub fn conic_surface(
    body: &Body,
    conic: &Conic,
    sweep: &ConicSweep,
    loops: &[LoopId],
) -> Result<Surface, String> {
    let points = edge_points(body, loops, 32);
    if points.is_empty() {
        return Err(format!(
            "a face on a swept {} without edges to bound it",
            conic.kind()
        ));
    }
    let frame = conic.frame();
    let centre = frame.origin();
    match *sweep {
        ConicSweep::Extrusion(along) => {
            let n = frame.z();
            let across = along.dot(n);
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for x in &points {
                let flat = *x - along * ((*x - centre).dot(n) / across);
                let u = conic
                    .parameter(flat)
                    .ok_or_else(|| "an edge of a face on it is not finite".to_string())?;
                (lo, hi) = (lo.min(u), hi.max(u));
            }
            let margin = (0.25 * (hi - lo)).clamp(1e-3, 1.0);
            let arc = conic.arc(lo - margin, hi + margin)?;
            bounded_extrusion(body, &arc, along, loops)
        }
        ConicSweep::Revolution { at, axis } => {
            let reach = points
                .iter()
                .map(|x| x.distance(at))
                .fold(0.0_f64, f64::max)
                * 1.25
                + centre.distance(at);
            let bound = match *conic {
                Conic::Hyperbola { a, b, .. } => {
                    (reach / a).max(1.0).pacosh().min((reach / b).pasinh())
                }
                Conic::Parabola { focal, .. } => {
                    let f = focal.abs();
                    (reach / f).sqrt().min(reach / (2.0 * f))
                }
            };
            if !bound.is_finite() {
                return Err("its faces' edges are not finite".to_string());
            }
            let bound = bound.max(1e-3);
            let arc = conic.arc(-bound, bound)?;
            revolved(&arc, at, axis)
                .map(Surface::Nurbs)
                .map_err(|e| e.to_string())
        }
    }
}

/// Where a profile turned about the axis through `at` along `axis` (a
/// unit vector) goes across it inside its domain, as seen in the plane
/// through the axis and the radial direction of its control point
/// farthest from the axis (`across`): the parameters at which the signed
/// distance along `across` changes sign, each found by bisection to
/// rounding, and how far the profile lies out of that plane (`off`, the
/// largest distance of a control point from it; the curve lies in the
/// hull of its control points). Whether such a sign change is a crossing
/// of the axis is for [`turned_part`] to say, against the file's
/// uncertainty: for a profile within it of the plane it is; for one
/// farther out the profile may pass the axis anywhere. A profile that only
/// starts or ends on the axis (a pole) has none. A Bézier piece whose
/// signed distances of its control points (times their weights, which are
/// positive) all have one sign has none (the convex hull); the others are
/// looked at in 64 steps, a sign change between two steps bisected.
pub(crate) struct Crossings {
    pub across: Vec3,
    pub cuts: Vec<f64>,
    pub off: f64,
}

pub(crate) fn crossings(curve: &NurbsCurve, at: Vec3, axis: Vec3) -> Crossings {
    let radial = |p: Vec3| {
        let off = p - at;
        off - axis * off.dot(axis)
    };
    let points = curve.control_points();
    let size = points
        .iter()
        .map(|p| (*p - at).length())
        .fold(0.0_f64, f64::max);
    let widest = points
        .iter()
        .map(|&p| radial(p))
        .max_by(|a, b| a.length().total_cmp(&b.length()));
    let none = |across: Vec3| Crossings {
        across,
        cuts: Vec::new(),
        off: 0.0,
    };
    let Some(across) = widest.and_then(Vec3::normalized) else {
        return none(Vec3::ZERO);
    };
    if !(size.is_finite() && size > 0.0) {
        return none(across);
    }
    let normal = axis.cross(across);
    let off = points
        .iter()
        .map(|&p| (p - at).dot(normal).abs())
        .fold(0.0_f64, f64::max);
    let small = 1e-14 * size;
    let side = |t: f64| {
        let s = (curve.eval(t) - at).dot(across);
        if s > small {
            1
        } else if s < -small {
            -1
        } else {
            0
        }
    };
    let (d0, d1) = curve.domain();
    let mut out = Vec::new();
    // The last place a side was seen, and that side.
    let mut last: Option<(f64, i32)> = None;
    let mut see = |t: f64, s: i32, out: &mut Vec<f64>| {
        if s == 0 {
            return;
        }
        if let Some((t0, s0)) = last
            && s0 != s
        {
            let (mut lo, mut hi) = (t0, t);
            for _ in 0..80 {
                let mid = 0.5 * (lo + hi);
                if !(mid > lo && mid < hi) {
                    break;
                }
                if side(mid) == s0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            out.push(0.5 * (lo + hi));
        }
        last = Some((t, s));
    };
    for ((lo, hi), piece) in curve.bezier_pieces() {
        let signs: Vec<i32> = piece
            .iter()
            .map(|&(p, w)| {
                let s = (p - at).dot(across) * w;
                if s > small * w {
                    1
                } else if s < -small * w {
                    -1
                } else {
                    0
                }
            })
            .collect();
        if let Some(&first) = signs.first()
            && first != 0
            && signs.iter().all(|&s| s == first)
        {
            see(0.5 * (lo + hi), first, &mut out);
            continue;
        }
        for k in 0..=64 {
            let t = lo + (hi - lo) * f64::from(k) / 64.0;
            see(t, side(t), &mut out);
        }
    }
    let width = d1 - d0;
    out.retain(|&t| t > d0 + 1e-12 * width && t < d1 - 1e-12 * width);
    Crossings {
        across,
        cuts: out,
        off,
    }
}

/// A profile's meridian trace: what a turn about the axis keeps of each of
/// its points, the height along the axis and the distance from it, at 64
/// steps per Bézier piece.
struct Trace<'a> {
    curve: &'a NurbsCurve,
    at: Vec3,
    axis: Vec3,
    steps: Vec<(f64, f64, f64)>,
}

impl<'a> Trace<'a> {
    fn new(curve: &'a NurbsCurve, at: Vec3, axis: Vec3) -> Self {
        let mut trace = Trace {
            curve,
            at,
            axis,
            steps: Vec::new(),
        };
        let mut ts = Vec::new();
        for ((lo, hi), _) in curve.bezier_pieces() {
            for k in 0..64 {
                ts.push(lo + (hi - lo) * f64::from(k) / 64.0);
            }
        }
        ts.push(curve.domain().1);
        trace.steps = ts
            .into_iter()
            .map(|t| {
                let (z, r) = trace.meridian(curve.eval(t));
                (t, z, r)
            })
            .collect();
        trace
    }

    /// Height along the axis and distance from it.
    fn meridian(&self, p: Vec3) -> (f64, f64) {
        let off = p - self.at;
        let z = off.dot(self.axis);
        (z, (off - self.axis * z).length())
    }

    /// The parameter in `[lo, hi]` whose meridian point is nearest to that
    /// of `p`, and how near: the nearest step, refined by golden-section
    /// search between its neighbours.
    fn nearest(&self, p: Vec3, (lo, hi): (f64, f64)) -> (f64, f64) {
        let (zp, rp) = self.meridian(p);
        let gap = |t: f64| {
            let (z, r) = self.meridian(self.curve.eval(t));
            (z - zp).hypot(r - rp)
        };
        let mut best: Option<(usize, f64)> = None;
        for (k, &(t, z, r)) in self.steps.iter().enumerate() {
            if t < lo || t > hi {
                continue;
            }
            let d = (z - zp).hypot(r - rp);
            if best.is_none_or(|(_, b)| d < b) {
                best = Some((k, d));
            }
        }
        let Some((k, _)) = best else {
            return (lo, f64::INFINITY);
        };
        let step = |j: usize| self.steps.get(j).map(|s| s.0);
        let mut a = k.checked_sub(1).and_then(step).unwrap_or(lo).max(lo);
        let mut b = step(k + 1).unwrap_or(hi).min(hi);
        let g = 0.5 * (5.0f64.sqrt() - 1.0);
        for _ in 0..60 {
            let (c, d) = (b - g * (b - a), a + g * (b - a));
            if gap(c) < gap(d) {
                b = d;
            } else {
                a = c;
            }
        }
        let t = 0.5 * (a + b);
        let at_step = step(k).unwrap_or(t);
        if gap(at_step) < gap(t) {
            (at_step, gap(at_step))
        } else {
            (t, gap(t))
        }
    }
}

/// For a profile that goes across the plane through its axis and lies out
/// of it by more than the file's uncertainty (so it cannot be cut on the
/// axis without moving it by more than that), whether a face on its whole
/// turn is a face the file can mean, else why not:
///
/// - its edges must not lie on two of the profile's sides at once
///   (the sides between the places it goes across the plane, `cuts`):
///   where they do, the sides sweep the same surface twice, as mirror
///   images, and which one the face is on is not in what is read;
/// - its region on the surface must not reach an end of the profile that
///   is not on the axis (a turn there is no pole: the face would not be
///   closed). The region is told by the loops that go round the axis: a
///   face lies on the left of each, seen from its normal (`∂u × ∂v`, `u`
///   round the axis counter-clockwise from its tip, `v` along the
///   profile, turned over where the face is `reversed`), so a loop that
///   goes round in the sense of `u` has the face towards larger `v`, the
///   other way smaller; the face reaches the profile's start where its
///   lowest such loop has it towards smaller `v`, its end where its
///   highest has it towards larger. A face without such a loop is a patch
///   its loops close.
fn whole_turn_holds(
    body: &Body,
    trace: &Trace<'_>,
    loops: &[LoopId],
    reversed: bool,
    merge: f64,
    cuts: &[f64],
) -> Result<(), String> {
    let (d0, d1) = trace.curve.domain();
    let mut ends = vec![d0];
    ends.extend(cuts.iter().copied());
    ends.push(d1);
    let points = edge_points(body, loops, 32);
    let on_two = points.iter().all(|&p| {
        ends.windows(2)
            .filter(|w| match **w {
                [lo, hi] => trace.nearest(p, (lo, hi)).1 <= merge,
                _ => false,
            })
            .count()
            >= 2
    });
    if !points.is_empty() && on_two {
        return Err(
            "its profile goes across the axis it is turned about, and its sides sweep the same surface twice, as mirror images; the face's edges lie on both, and which side it is on is in its parameter-space curves, which are not read: not read"
                .to_string(),
        );
    }
    // The loops that go round the axis: where along the profile, and the
    // way the face lies from them.
    let mut around: Vec<(f64, f64)> = Vec::new();
    for l in loops {
        let Some(l) = body.loop_(*l) else {
            continue;
        };
        let mut pts = Vec::new();
        for c in &l.coedges {
            let Some(edge) = body.edge(c.edge) else {
                continue;
            };
            let (t0, t1) = edge.range;
            let mut run: Vec<Vec3> = (0..=32)
                .map(|k| edge.curve.eval(t0 + (t1 - t0) * f64::from(k) / 32.0))
                .collect();
            if c.reversed {
                run.reverse();
            }
            pts.extend(run);
        }
        let radial = |p: Vec3| {
            let off = p - trace.at;
            off - trace.axis * off.dot(trace.axis)
        };
        let mut turn = 0.0;
        for (k, &p) in pts.iter().enumerate() {
            let Some(&q) = pts.get(k + 1).or_else(|| pts.first()) else {
                continue;
            };
            let (a, b) = (radial(p), radial(q));
            if a.length() <= merge || b.length() <= merge {
                continue;
            }
            turn += trace.axis.dot(a.cross(b)).patan2(a.dot(b));
        }
        let winding = (turn / std::f64::consts::TAU).round();
        if winding == 0.0 || !winding.is_finite() {
            continue;
        }
        let mut ts: Vec<f64> = pts.iter().map(|&p| trace.nearest(p, (d0, d1)).0).collect();
        ts.sort_by(f64::total_cmp);
        let Some(&middle) = ts.get(ts.len() / 2) else {
            continue;
        };
        let towards = winding.signum() * if reversed { -1.0 } else { 1.0 };
        around.push((middle, towards));
    }
    around.sort_by(|a, b| a.0.total_cmp(&b.0));
    let pole = |t: f64| trace.meridian(trace.curve.eval(t)).1 <= merge;
    if let Some(&(_, towards)) = around.first()
        && towards < 0.0
        && !pole(d0)
    {
        return Err(
            "its profile goes across the axis it is turned about out of a plane through the axis by more than the file's uncertainty, and the face on it reaches the profile's start, which is not on the axis: the face is not closed there; not read"
                .to_string(),
        );
    }
    if let Some(&(_, towards)) = around.last()
        && towards > 0.0
        && !pole(d1)
    {
        return Err(
            "its profile goes across the axis it is turned about out of a plane through the axis by more than the file's uncertainty, and the face on it reaches the profile's end, which is not on the axis: the face is not closed there; not read"
                .to_string(),
        );
    }
    Ok(())
}

/// The surface of a [`Swept::Turned`] a face lies on: the profile goes
/// across the plane through its axis inside its domain ([`crossings`]).
///
/// *Within the file's uncertainty `merge` of that plane*, it crosses the
/// axis there (a point whose signed distance along the plane is zero lies
/// as far from the axis as from the plane): the profile is cut there and
/// each cut end put on the axis, a move of at most its distance from the
/// plane, so within `merge`, as two vertices of the file within it are
/// one; the rest of each part is the file's curve as it is, not moved
/// into the plane. The face's edges (33 points each), each turned into the
/// plane on the side of a part, are held to every part; the part they lie
/// nearest to is the face's when no other part is within four times as
/// near and `merge` more (the parts lie within `merge` of the plane, so
/// this holds them to it), and it is turned: the exact rational B-spline
/// surface, its normal the file's. Where two parts are that near (parts
/// that mirror each other sweep the same surface twice, with opposite
/// normals), which the face is on is in its parameter-space curves, which
/// are not read, and it is refused.
///
/// *Farther from the plane* the profile cannot be cut on the axis without
/// moving it by more than `merge`, so it is turned whole, as a profile that
/// does not go across is; the face is then held to what such a turn can
/// carry ([`whole_turn_holds`]): its edges not on two mirrored sides at
/// once, and its region not reaching an end of the profile off the axis
/// (a profile across its axis but out of its plane passes it at a neck,
/// and a cup on one side runs through it onto the other). Either fails,
/// the face is refused, saying which. So for every distance from the plane
/// a face is read on a turn it is closed on, or refused (`reversed`: the
/// face turned over against the surface's normal).
pub fn turned_part(
    body: &Body,
    curve: &NurbsCurve,
    at: Vec3,
    axis: Vec3,
    loops: &[LoopId],
    reversed: bool,
    merge: f64,
) -> Result<(Surface, bool), String> {
    let Crossings { across, cuts, off } = crossings(curve, at, axis);
    if !(off <= merge) || cuts.is_empty() {
        let trace = Trace::new(curve, at, axis);
        whole_turn_holds(body, &trace, loops, reversed, merge, &cuts)?;
        return revolved(curve, at, axis)
            .map(|s| (Surface::Nurbs(s), false))
            .map_err(|e| e.to_string());
    }
    let points = edge_points(body, loops, 32);
    if points.is_empty() {
        return Err(
            "a face without edges on a profile turned about an axis it crosses: which part of it the face is on cannot be told; not read"
                .to_string(),
        );
    }
    let (d0, d1) = curve.domain();
    let mut ends = vec![d0];
    ends.extend(cuts.iter().copied());
    ends.push(d1);
    let foot = |p: Vec3| at + axis * (p - at).dot(axis);
    let mut parts: Vec<(f64, NurbsCurve)> = Vec::new();
    for (k, pair) in ends.windows(2).enumerate() {
        let &[lo, hi] = pair else {
            continue;
        };
        if !(hi > lo) {
            continue;
        }
        let part = crate::bounded::part(&Curve3::Nurbs(curve.clone()), lo, hi, true)?;
        // The cut ends on the axis exactly.
        let mut ctrl = part.control_points().to_vec();
        if k > 0
            && let Some(first) = ctrl.first_mut()
        {
            *first = foot(*first);
        }
        if k + 2 < ends.len()
            && let Some(last) = ctrl.last_mut()
        {
            *last = foot(*last);
        }
        let part = NurbsCurve::new(
            part.degree(),
            part.knots().to_vec(),
            ctrl,
            part.weights().to_vec(),
        )
        .map_err(|e| e.to_string())?;
        let side = (curve.eval(0.5 * (lo + hi)) - at).dot(across).signum();
        let shape = Curve3::Nurbs(part.clone());
        let mut worst = 0.0_f64;
        for &x in &points {
            let off = x - at;
            let z = off.dot(axis);
            let r = (off - axis * z).length();
            let flat = at + axis * z + across * (side * r);
            let d = shape
                .project(flat)
                .map(|t| shape.eval(t).distance(flat))
                .unwrap_or(f64::INFINITY);
            worst = worst.max(if d.is_finite() { d } else { f64::INFINITY });
        }
        parts.push((worst, part));
    }
    let nearest = parts.iter().map(|(d, _)| *d).fold(f64::INFINITY, f64::min);
    if !nearest.is_finite() {
        return Err("the face's edges lie on no part of the profile turned".to_string());
    }
    let mut near = parts.iter().filter(|(d, _)| *d <= 4.0 * nearest + merge);
    let (Some((_, part)), None) = (near.next(), near.next()) else {
        return Err(format!(
            "its profile crosses the axis it is turned about, so its parts sweep the same surface twice, once with each normal; the face's edges lie on {} of them, and which it is on is in its parameter-space curves, which are not read: not read (a profile that does not cross the axis, or the surface as a B-spline, reads)",
            parts
                .iter()
                .filter(|(d, _)| *d <= 4.0 * nearest + merge)
                .count()
        ));
    };
    revolved(part, at, axis)
        .map(|s| (Surface::Nurbs(s), false))
        .map_err(|e| e.to_string())
}
