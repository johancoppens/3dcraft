//! Intersections of surfaces with surfaces and of curves with surfaces.
//!
//! [`meet_surfaces`] and [`intersect_curve_surface`] cover every kind: in
//! closed form where there is one, numerically otherwise (the submodules
//! say how, and what is proved about completeness). A numeric result is
//! within the tolerance or an error, never a guess.
//!
//! [`intersect_surfaces`] is the older interface for planes and cylinders
//! only, in closed form, without a box; it refuses everything else.
//!
//! Derived from first principles (analytic geometry of quadrics; see for
//! instance Mortenson, *Geometric Modeling*, the chapters on intersections).
//!
//! Tolerances. Two directions are parallel when the sine of their angle is
//! at most `tolerance.angular`; two places coincide within
//! `tolerance.linear`. A touch (the curve or surface comes within the linear
//! tolerance of the other without crossing it by more) is reported as
//! *tangent*, so that callers can refuse it. A configuration that is neither
//! parallel nor safely apart from parallel ([`NEARLY_PARALLEL`]) has an
//! intersection whose position amplifies rounding by a million or more and
//! is refused.

use craft3d_math::Portable;
mod clip;
mod curves;
mod exact;
mod field;
mod march;
mod meet;

pub use meet::{Meeting, MeetingCurve, SurfaceMeeting, meet_surfaces, meet_surfaces_numeric};

/// Lets go of what the intersections keep between calls: the
/// intersections of surfaces asked for lately and the projectors and
/// pieces of the surfaces they met. Each is worked out again, the same,
/// when asked for (only the time differs). Returns how many there were.
pub fn forget_kept() -> usize {
    meet::forget_shelf().saturating_add(field::forget_shelves())
}

/// How many intersections this thread marched a second time with the
/// bound that leaves out the cells of surfaces side by side, after the
/// first march ran out of cells (for tests to tell that path was taken).
pub fn bounded_marches() -> u64 {
    meet::bounded_marches()
}

/// How many intersections, projectors and pieces of surfaces are kept.
pub fn kept() -> usize {
    meet::shelf_len().saturating_add(field::shelves_len())
}

use crate::{Circle, Curve3, Ellipse, GeomError, Line, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use std::f64::consts::TAU;

/// Sine of the angle below which a configuration that is not parallel within
/// the angular tolerance is too close to parallel to intersect reliably.
pub const NEARLY_PARALLEL: f64 = 1e-6;

/// How two surfaces meet.
#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceIntersection {
    /// They do not come within the linear tolerance of each other.
    None,
    /// They are the same surface. `same_normal` says whether their normals
    /// agree where they coincide.
    Coincident { same_normal: bool },
    /// They cross along these curves.
    Curves(Vec<Curve3>),
    /// They touch along these curves without crossing.
    Tangent(Vec<Curve3>),
}

/// A point where a curve meets a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveHit {
    /// The curve parameter; in `[0, 2π)` for circles and ellipses.
    pub t: f64,
    pub point: Vec3,
    /// True when the curve touches the surface here without crossing it by
    /// more than the linear tolerance.
    pub tangent: bool,
}

/// How a curve (the whole, unbounded or closed curve) meets a surface.
#[derive(Clone, Debug, PartialEq)]
pub enum CurveIntersection {
    /// The curve lies on the surface.
    OnSurface,
    /// It meets the surface at these points, in ascending parameter order;
    /// none when the list is empty.
    Hits(Vec<CurveHit>),
}

fn unsupported(what: &'static str) -> GeomError {
    GeomError::Unsupported(what)
}

fn degenerate() -> GeomError {
    GeomError::InvalidParameter("an intersection has no finite position")
}

fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

/// `v` without its component along the unit vector `axis`.
fn across(v: Vec3, axis: Vec3) -> Vec3 {
    v - axis * v.dot(axis)
}

/// The curves in which two surfaces meet.
pub fn intersect_surfaces(
    a: &Surface,
    b: &Surface,
    tolerance: &Tolerance,
) -> Result<SurfaceIntersection, GeomError> {
    match (a, b) {
        (Surface::Plane(p), Surface::Plane(q)) => plane_plane(p.frame(), q.frame(), tolerance),
        (Surface::Plane(p), Surface::Cylinder(c)) | (Surface::Cylinder(c), Surface::Plane(p)) => {
            plane_cylinder(p.frame(), c.frame(), c.radius(), tolerance)
        }
        (Surface::Cylinder(c), Surface::Cylinder(d)) => {
            cylinder_cylinder(c.frame(), c.radius(), d.frame(), d.radius(), tolerance)
        }
        _ => Err(unsupported(
            "intersections of surfaces other than planes and cylinders",
        )),
    }
}

fn plane_plane(p: &Frame, q: &Frame, tol: &Tolerance) -> Result<SurfaceIntersection, GeomError> {
    let (n1, n2) = (p.z(), q.z());
    let cross = n1.cross(n2);
    let sine = cross.length();
    if sine <= tol.angular {
        let distance = (q.origin() - p.origin()).dot(n1);
        return Ok(if distance.abs() <= tol.linear {
            SurfaceIntersection::Coincident {
                same_normal: n1.dot(n2) > 0.0,
            }
        } else {
            SurfaceIntersection::None
        });
    }
    if sine < NEARLY_PARALLEL {
        return Err(unsupported("planes that are nearly, but not, parallel"));
    }
    let direction = cross * (1.0 / sine);
    // The point of the line closest to the first plane's origin: move in
    // that plane, across the line, until the second plane is reached.
    let m = direction.cross(n1);
    let s = (q.origin() - p.origin()).dot(n2) / m.dot(n2);
    let line = Line::new(p.origin() + m * s, direction)?;
    Ok(SurfaceIntersection::Curves(vec![Curve3::Line(line)]))
}

fn plane_cylinder(
    plane: &Frame,
    cylinder: &Frame,
    radius: f64,
    tol: &Tolerance,
) -> Result<SurfaceIntersection, GeomError> {
    let (n, axis) = (plane.z(), cylinder.z());
    let cosine = n.dot(axis);
    let sine = n.cross(axis).length();
    if cosine.abs() <= tol.angular {
        // The plane is parallel to the axis: two rulings, one, or none.
        let Some(across_axis) = across(n, axis).normalized() else {
            return Err(degenerate());
        };
        let distance = (cylinder.origin() - plane.origin()).dot(across_axis);
        if distance.abs() > radius + tol.linear {
            return Ok(SurfaceIntersection::None);
        }
        let foot = cylinder.origin() - across_axis * distance;
        if (distance.abs() - radius).abs() <= tol.linear {
            let line = Line::new(foot, axis)?;
            return Ok(SurfaceIntersection::Tangent(vec![Curve3::Line(line)]));
        }
        let half = ((radius - distance) * (radius + distance)).max(0.0).sqrt();
        let side = axis.cross(across_axis);
        return Ok(SurfaceIntersection::Curves(vec![
            Curve3::Line(Line::new(foot - side * half, axis)?),
            Curve3::Line(Line::new(foot + side * half, axis)?),
        ]));
    }
    if cosine.abs() < NEARLY_PARALLEL {
        return Err(unsupported(
            "a plane that is nearly, but not, parallel to a cylinder's axis",
        ));
    }
    let v = (plane.origin() - cylinder.origin()).dot(n) / cosine;
    let center = cylinder.origin() + axis * v;
    if sine <= tol.angular {
        // Perpendicular to the axis: a circle with the cylinder's own axes,
        // so that its angle is the cylinder's u.
        let frame = cylinder.at(center).ok_or_else(degenerate)?;
        let circle = Circle::new(frame, radius)?;
        return Ok(SurfaceIntersection::Curves(vec![Curve3::Circle(circle)]));
    }
    // Oblique: an ellipse whose major axis is the axis' shadow on the plane.
    // Its normal is turned to the axis' side, so that the eccentric angle
    // runs the same way round as the cylinder's u.
    let normal = if cosine > 0.0 { n } else { -n };
    let frame = Frame::new(center, normal, axis).ok_or_else(degenerate)?;
    let ellipse = Ellipse::new(frame, radius / cosine.abs(), radius)?;
    Ok(SurfaceIntersection::Curves(vec![Curve3::Ellipse(ellipse)]))
}

/// Where two circles in one plane meet: `None` when they do not, otherwise
/// the distance from the first centre towards the second of the chord they
/// share, half the chord's length, and whether they only touch.
///
/// `distance` between the centres must exceed the linear tolerance.
fn circle_circle(distance: f64, r1: f64, r2: f64, tol: f64) -> Option<(f64, f64, bool)> {
    if distance > r1 + r2 + tol || distance < (r1 - r2).abs() - tol {
        return None;
    }
    if (distance - (r1 + r2)).abs() <= tol {
        return Some((r1, 0.0, true));
    }
    if (distance - (r1 - r2).abs()).abs() <= tol {
        return Some((if r1 >= r2 { r1 } else { -r1 }, 0.0, true));
    }
    let along = (distance * distance + r1 * r1 - r2 * r2) / (2.0 * distance);
    let half = ((r1 - along) * (r1 + along)).max(0.0).sqrt();
    Some((along, half, false))
}

fn cylinder_cylinder(
    c: &Frame,
    r1: f64,
    d: &Frame,
    r2: f64,
    tol: &Tolerance,
) -> Result<SurfaceIntersection, GeomError> {
    let (z1, z2) = (c.z(), d.z());
    let cross = z1.cross(z2);
    let sine = cross.length();
    let offset = d.origin() - c.origin();
    if sine > tol.angular {
        // Skew or crossing axes: apart when the axes are further from each
        // other than the radii reach; the general curve is a quartic.
        let apart = offset.dot(cross).abs() / sine;
        if apart > r1 + r2 + tol.linear {
            return Ok(SurfaceIntersection::None);
        }
        return Err(unsupported(
            "cylinders whose axes are not parallel (the intersection is a quartic curve)",
        ));
    }
    let between = across(offset, z1);
    let distance = between.length();
    if distance <= tol.linear {
        return Ok(if (r1 - r2).abs() <= tol.linear {
            SurfaceIntersection::Coincident { same_normal: true }
        } else {
            SurfaceIntersection::None
        });
    }
    let Some((along, half, touching)) = circle_circle(distance, r1, r2, tol.linear) else {
        return Ok(SurfaceIntersection::None);
    };
    let toward = between * (1.0 / distance);
    let foot = c.origin() + toward * along;
    if touching {
        let line = Line::new(foot, z1)?;
        return Ok(SurfaceIntersection::Tangent(vec![Curve3::Line(line)]));
    }
    let side = z1.cross(toward);
    Ok(SurfaceIntersection::Curves(vec![
        Curve3::Line(Line::new(foot - side * half, z1)?),
        Curve3::Line(Line::new(foot + side * half, z1)?),
    ]))
}

/// Where a curve meets a surface, for every kind of curve and surface. The
/// curve is taken whole: a line is infinite, a circle or ellipse closed, a
/// NURBS curve its domain; the caller keeps the hits in the range it cares
/// about.
///
/// Closed form: a line with a plane, cylinder, sphere or cone; a circle or
/// ellipse with a plane, and with a cylinder it is a section of a parallel
/// cylinder to; a circle with a sphere, and with any surface of revolution
/// about the circle's axis. Everything else is found numerically
/// (`intersect/curves.rs`), and fails with [`GeomError::NoConvergence`]
/// where the curve runs along the surface for a stretch without lying on
/// it everywhere. A line along a ruling of a cone is `OnSurface`.
pub fn intersect_curve_surface(
    curve: &Curve3,
    surface: &Surface,
    tolerance: &Tolerance,
) -> Result<CurveIntersection, GeomError> {
    match (curve, surface) {
        (Curve3::Line(l), Surface::Plane(p)) => Ok(line_plane(l, p.frame(), tolerance)),
        (Curve3::Line(l), Surface::Cylinder(c)) => {
            Ok(line_cylinder(l, c.frame(), c.radius(), tolerance))
        }
        (Curve3::Circle(_) | Curve3::Ellipse(_), Surface::Plane(p)) => {
            let (center, x, y) = conic(curve).ok_or_else(degenerate)?;
            Ok(conic_plane(center, x, y, p.frame(), tolerance))
        }
        (Curve3::Circle(_) | Curve3::Ellipse(_), Surface::Cylinder(c)) => {
            let (center, x, y) = conic(curve).ok_or_else(degenerate)?;
            match conic_cylinder(center, x, y, c.frame(), c.radius(), tolerance) {
                // Not a section of a parallel cylinder: no closed form.
                Err(GeomError::Unsupported(_)) => curves::numeric(curve, surface, tolerance),
                other => other,
            }
        }
        (Curve3::Line(l), Surface::Sphere(s)) => Ok(curves::line_sphere(l, s, tolerance)),
        (Curve3::Line(l), Surface::Cone(c)) => curves::line_cone(l, c, tolerance),
        (Curve3::Circle(c), Surface::Sphere(s)) => Ok(curves::circle_sphere(c, s, tolerance)),
        _ => curves::numeric(curve, surface, tolerance),
    }
}

/// A circle or ellipse as `center + x cos t + y sin t`.
fn conic(curve: &Curve3) -> Option<(Vec3, Vec3, Vec3)> {
    match curve {
        Curve3::Circle(c) => {
            let f = c.frame();
            Some((f.origin(), f.x() * c.radius(), f.y() * c.radius()))
        }
        Curve3::Ellipse(e) => {
            let (f, (a, b)) = (e.frame(), e.semi_axes());
            Some((f.origin(), f.x() * a, f.y() * b))
        }
        Curve3::Line(_) | Curve3::Nurbs(_) => None,
    }
}

fn line_plane(line: &Line, plane: &Frame, tol: &Tolerance) -> CurveIntersection {
    let n = plane.z();
    let rate = line.direction().dot(n);
    let height = (line.origin() - plane.origin()).dot(n);
    if rate.abs() <= tol.angular {
        return if height.abs() <= tol.linear {
            CurveIntersection::OnSurface
        } else {
            CurveIntersection::Hits(Vec::new())
        };
    }
    let t = -height / rate;
    let point = line.origin() + line.direction() * t;
    CurveIntersection::Hits(if point.is_finite() {
        vec![CurveHit {
            t,
            point,
            tangent: false,
        }]
    } else {
        Vec::new()
    })
}

fn line_cylinder(line: &Line, cylinder: &Frame, radius: f64, tol: &Tolerance) -> CurveIntersection {
    let axis = cylinder.z();
    let d = across(line.direction(), axis);
    let p = across(line.origin() - cylinder.origin(), axis);
    let a = d.length_squared();
    if a.sqrt() <= tol.angular {
        return if (p.length() - radius).abs() <= tol.linear {
            CurveIntersection::OnSurface
        } else {
            CurveIntersection::Hits(Vec::new())
        };
    }
    // Closest approach to the axis, and how far from it.
    let nearest = -p.dot(d) / a;
    let reach = (p + d * nearest).length();
    let hit = |t: f64, tangent: bool| CurveHit {
        t,
        point: line.origin() + line.direction() * t,
        tangent,
    };
    let hits = if reach > radius + tol.linear {
        Vec::new()
    } else if (reach - radius).abs() <= tol.linear {
        vec![hit(nearest, true)]
    } else {
        let half = ((radius - reach) * (radius + reach)).max(0.0).sqrt() / a.sqrt();
        vec![hit(nearest - half, false), hit(nearest + half, false)]
    };
    CurveIntersection::Hits(hits.into_iter().filter(|h| h.point.is_finite()).collect())
}

/// The hits of `center + x cos t + y sin t` at the given angles, ascending.
fn conic_hits(center: Vec3, x: Vec3, y: Vec3, angles: &[f64], tangent: bool) -> Vec<CurveHit> {
    let mut hits: Vec<CurveHit> = angles
        .iter()
        .map(|&angle| {
            let t = wrap(angle);
            let (s, c) = t.psin_cos();
            CurveHit {
                t,
                point: center + x * c + y * s,
                tangent,
            }
        })
        .filter(|h| h.point.is_finite())
        .collect();
    hits.sort_by(|a, b| a.t.total_cmp(&b.t));
    hits
}

fn conic_plane(
    center: Vec3,
    x: Vec3,
    y: Vec3,
    plane: &Frame,
    tol: &Tolerance,
) -> CurveIntersection {
    // Height above the plane: h + a cos t + b sin t = h + r cos(t - phase).
    let n = plane.z();
    let h = (center - plane.origin()).dot(n);
    let (a, b) = (x.dot(n), y.dot(n));
    let r = a.phypot(b);
    if r <= tol.linear {
        return if h.abs() <= tol.linear {
            CurveIntersection::OnSurface
        } else {
            CurveIntersection::Hits(Vec::new())
        };
    }
    if h.abs() > r + tol.linear {
        return CurveIntersection::Hits(Vec::new());
    }
    let phase = b.patan2(a);
    if (h.abs() - r).abs() <= tol.linear {
        // The lowest point touches when the centre is above, the highest
        // when it is below.
        let angle = if h > 0.0 {
            phase + std::f64::consts::PI
        } else {
            phase
        };
        return CurveIntersection::Hits(conic_hits(center, x, y, &[angle], true));
    }
    let spread = (-h / r).clamp(-1.0, 1.0).pacos();
    CurveIntersection::Hits(conic_hits(
        center,
        x,
        y,
        &[phase - spread, phase + spread],
        false,
    ))
}

fn conic_cylinder(
    center: Vec3,
    x: Vec3,
    y: Vec3,
    cylinder: &Frame,
    radius: f64,
    tol: &Tolerance,
) -> Result<CurveIntersection, GeomError> {
    let axis = cylinder.z();
    // Seen along the axis the cylinder is a circle; the curve is one too
    // when it is a section of a cylinder with a parallel axis.
    let (px, py) = (across(x, axis), across(y, axis));
    let delta = across(center - cylinder.origin(), axis);
    let (lx, ly) = (px.length(), py.length());
    let distance = delta.length();
    let round = (lx - ly).abs() <= tol.linear && px.dot(py).abs() <= tol.angular * lx * ly;
    if !round {
        let reach = x.length().max(y.length());
        let from_axis = distance; // of the centre
        if from_axis > reach + radius + tol.linear || from_axis + reach < radius - tol.linear {
            return Ok(CurveIntersection::Hits(Vec::new()));
        }
        return Err(unsupported(
            "a circle or ellipse with a cylinder it is not a section of a parallel cylinder to",
        ));
    }
    let r = lx;
    if distance <= tol.linear {
        return Ok(if (r - radius).abs() <= tol.linear {
            CurveIntersection::OnSurface
        } else {
            CurveIntersection::Hits(Vec::new())
        });
    }
    // The curve's shadow is the circle of radius r about `delta`; the
    // cylinder's the circle of `radius` about 0.
    let Some((along, half, touching)) = circle_circle(distance, r, radius, tol.linear) else {
        return Ok(CurveIntersection::Hits(Vec::new()));
    };
    let toward = delta * (-1.0 / distance); // from the curve's centre to the axis
    let side = axis.cross(toward);
    let angle_of = |q: Vec3| q.dot(py).patan2(q.dot(px));
    let q1 = toward * along - side * half;
    let q2 = toward * along + side * half;
    Ok(CurveIntersection::Hits(if touching {
        conic_hits(center, x, y, &[angle_of(toward * along)], true)
    } else {
        conic_hits(center, x, y, &[angle_of(q1), angle_of(q2)], false)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cylinder, Plane};

    fn tol() -> Tolerance {
        Tolerance::default()
    }

    fn tilted() -> Frame {
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap()
    }

    fn distance_to(surface: &Surface, p: Vec3) -> f64 {
        let (u, v) = surface.project(p).unwrap();
        surface.eval(u, v).distance(p)
    }

    /// Every sample of every curve lies on both surfaces.
    fn assert_on_both(curves: &[Curve3], a: &Surface, b: &Surface, scale: f64) {
        for curve in curves {
            for k in 0..16 {
                let t = match curve {
                    Curve3::Line(_) => (f64::from(k) - 8.0) * scale,
                    _ => f64::from(k) * TAU / 16.0,
                };
                let p = curve.eval(t);
                assert!(distance_to(a, p) < 1e-12 * scale.max(1.0), "{curve:?}");
                assert!(distance_to(b, p) < 1e-12 * scale.max(1.0), "{curve:?}");
            }
        }
    }

    #[test]
    fn planes_meet_in_a_line_or_coincide_or_miss() {
        let f = tilted();
        let a = Surface::Plane(Plane::new(f));
        let other = Frame::new(f.point(3.0, 1.0, 2.0), f.vector(1.0, 0.2, 0.5), f.z()).unwrap();
        let b = Surface::Plane(Plane::new(other));
        let SurfaceIntersection::Curves(curves) = intersect_surfaces(&a, &b, &tol()).unwrap()
        else {
            panic!("expected a line");
        };
        assert_eq!(curves.len(), 1);
        assert_on_both(&curves, &a, &b, 1.0);

        let same = Surface::Plane(Plane::new(f.at(f.point(5.0, -3.0, 0.0)).unwrap()));
        assert_eq!(
            intersect_surfaces(&a, &same, &tol()).unwrap(),
            SurfaceIntersection::Coincident { same_normal: true }
        );
        let back = Surface::Plane(Plane::new(f.reversed_at(f.point(1.0, 1.0, 0.0)).unwrap()));
        assert_eq!(
            intersect_surfaces(&a, &back, &tol()).unwrap(),
            SurfaceIntersection::Coincident { same_normal: false }
        );
        let lifted = Surface::Plane(Plane::new(f.at(f.point(0.0, 0.0, 1e-3)).unwrap()));
        assert_eq!(
            intersect_surfaces(&a, &lifted, &tol()).unwrap(),
            SurfaceIntersection::None
        );
        let nearly = Frame::new(f.origin(), f.vector(1e-8, 0.0, 1.0), f.x()).unwrap();
        assert!(matches!(
            intersect_surfaces(&a, &Surface::Plane(Plane::new(nearly)), &tol()),
            Err(GeomError::Unsupported(_))
        ));
    }

    #[test]
    fn a_plane_cuts_a_cylinder_in_a_circle_lines_or_an_ellipse() {
        for scale in [1e-3, 1.0, 1e6] {
            let tol = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
            let f = tilted();
            let f = f.at(f.origin() * scale).unwrap();
            let r = 2.0 * scale;
            let cylinder = Surface::Cylinder(Cylinder::new(f, r).unwrap());

            let across = Surface::Plane(Plane::new(
                f.at(f.point(scale, scale, 3.0 * scale)).unwrap(),
            ));
            let SurfaceIntersection::Curves(curves) =
                intersect_surfaces(&across, &cylinder, &tol).unwrap()
            else {
                panic!("expected a circle");
            };
            assert!(matches!(curves.as_slice(), [Curve3::Circle(_)]));
            assert_on_both(&curves, &across, &cylinder, scale);
            // The circle's angle is the cylinder's u.
            let (u, _) = cylinder.project(curves[0].eval(1.25)).unwrap();
            assert!((u - 1.25).abs() < 1e-12);

            let along = Frame::new(f.point(scale, 0.0, 0.0), f.x(), f.z()).unwrap();
            let along = Surface::Plane(Plane::new(along));
            let SurfaceIntersection::Curves(curves) =
                intersect_surfaces(&cylinder, &along, &tol).unwrap()
            else {
                panic!("expected two lines");
            };
            assert!(matches!(
                curves.as_slice(),
                [Curve3::Line(_), Curve3::Line(_)]
            ));
            assert_on_both(&curves, &along, &cylinder, scale);
            assert!(curves[0].eval(0.0).distance(curves[1].eval(0.0)) > scale);

            let touching = Frame::new(f.point(r, 0.0, 0.0), f.x(), f.z()).unwrap();
            assert!(matches!(
                intersect_surfaces(&cylinder, &Surface::Plane(Plane::new(touching)), &tol).unwrap(),
                SurfaceIntersection::Tangent(lines) if lines.len() == 1
            ));
            let outside = Frame::new(f.point(r * 1.01, 0.0, 0.0), f.x(), f.z()).unwrap();
            assert_eq!(
                intersect_surfaces(&cylinder, &Surface::Plane(Plane::new(outside)), &tol).unwrap(),
                SurfaceIntersection::None
            );

            for lean in [0.3, -0.7, 2.5] {
                let oblique = Frame::new(
                    f.point(0.3 * scale, -0.2 * scale, scale),
                    f.vector(lean, 0.4, 1.0),
                    f.x(),
                )
                .unwrap();
                let oblique = Surface::Plane(Plane::new(oblique));
                let SurfaceIntersection::Curves(curves) =
                    intersect_surfaces(&oblique, &cylinder, &tol).unwrap()
                else {
                    panic!("expected an ellipse");
                };
                assert!(matches!(curves.as_slice(), [Curve3::Ellipse(_)]));
                assert_on_both(&curves, &oblique, &cylinder, scale);
                // The eccentric angle runs with the cylinder's u, at unit
                // rate: the booleans rely on it.
                let (u0, _) = cylinder.project(curves[0].eval(0.5)).unwrap();
                let (u1, _) = cylinder.project(curves[0].eval(1.5)).unwrap();
                assert!((wrap(u1 - u0) - 1.0).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn cylinders_with_parallel_axes_meet_in_lines() {
        let f = tilted();
        let a = Surface::Cylinder(Cylinder::new(f, 2.0).unwrap());
        let b =
            Surface::Cylinder(Cylinder::new(f.at(f.point(2.5, 0.5, 7.0)).unwrap(), 1.5).unwrap());
        let SurfaceIntersection::Curves(curves) = intersect_surfaces(&a, &b, &tol()).unwrap()
        else {
            panic!("expected two lines");
        };
        assert_eq!(curves.len(), 2);
        assert_on_both(&curves, &a, &b, 1.0);

        let same = Cylinder::new(f.reversed_at(f.point(0.0, 0.0, 4.0)).unwrap(), 2.0).unwrap();
        assert_eq!(
            intersect_surfaces(&a, &Surface::Cylinder(same), &tol()).unwrap(),
            SurfaceIntersection::Coincident { same_normal: true }
        );
        let inside = Cylinder::new(f.at(f.point(0.2, 0.0, 0.0)).unwrap(), 1.0).unwrap();
        assert_eq!(
            intersect_surfaces(&a, &Surface::Cylinder(inside), &tol()).unwrap(),
            SurfaceIntersection::None
        );
        let kissing = Cylinder::new(f.at(f.point(3.0, 0.0, 0.0)).unwrap(), 1.0).unwrap();
        assert!(matches!(
            intersect_surfaces(&a, &Surface::Cylinder(kissing), &tol()).unwrap(),
            SurfaceIntersection::Tangent(_)
        ));
        let inner_kiss = Cylinder::new(f.at(f.point(1.0, 0.0, 0.0)).unwrap(), 1.0).unwrap();
        assert!(matches!(
            intersect_surfaces(&a, &Surface::Cylinder(inner_kiss), &tol()).unwrap(),
            SurfaceIntersection::Tangent(_)
        ));
        let crossing = Frame::new(f.origin(), f.x(), f.z()).unwrap();
        assert!(matches!(
            intersect_surfaces(
                &a,
                &Surface::Cylinder(Cylinder::new(crossing, 1.0).unwrap()),
                &tol()
            ),
            Err(GeomError::Unsupported(_))
        ));
        let far = Frame::new(f.point(0.0, 0.0, 0.0) + f.y() * 10.0, f.x(), f.z()).unwrap();
        assert_eq!(
            intersect_surfaces(
                &a,
                &Surface::Cylinder(Cylinder::new(far, 1.0).unwrap()),
                &tol()
            )
            .unwrap(),
            SurfaceIntersection::None
        );
    }

    #[test]
    fn other_surface_kinds_are_refused() {
        let sphere = Surface::Sphere(crate::Sphere::new(Frame::WORLD, 1.0).unwrap());
        let plane = Surface::Plane(Plane::new(Frame::WORLD));
        assert!(matches!(
            intersect_surfaces(&sphere, &plane, &tol()),
            Err(GeomError::Unsupported(_))
        ));
        // Curves meet every kind of surface now: a diameter of the sphere.
        let line = Curve3::Line(Line::new(Vec3::ZERO, Vec3::X).unwrap());
        assert_hits_are_exact(&line, &sphere, 2, false);
    }

    fn hits(curve: &Curve3, surface: &Surface) -> Vec<CurveHit> {
        match intersect_curve_surface(curve, surface, &tol()).unwrap() {
            CurveIntersection::Hits(h) => h,
            CurveIntersection::OnSurface => panic!("on the surface"),
        }
    }

    fn assert_hits_are_exact(curve: &Curve3, surface: &Surface, count: usize, tangent: bool) {
        let found = hits(curve, surface);
        assert_eq!(found.len(), count, "{curve:?} {surface:?}");
        for h in found {
            assert!(curve.eval(h.t).distance(h.point) < 1e-12);
            let slack = if tangent { 2e-6 } else { 1e-12 };
            assert!(distance_to(surface, h.point) < slack);
            assert_eq!(h.tangent, tangent);
        }
    }

    #[test]
    fn lines_meet_planes_and_cylinders() {
        let f = tilted();
        let plane = Surface::Plane(Plane::new(f));
        let cylinder = Surface::Cylinder(Cylinder::new(f, 2.0).unwrap());
        let through =
            Curve3::Line(Line::new(f.point(0.5, 0.3, -4.0), f.vector(0.2, 0.1, 1.0)).unwrap());
        assert_hits_are_exact(&through, &plane, 1, false);
        let chord =
            Curve3::Line(Line::new(f.point(-5.0, 0.5, 1.0), f.vector(1.0, 0.1, 0.3)).unwrap());
        assert_hits_are_exact(&chord, &cylinder, 2, false);
        let h = hits(&chord, &cylinder);
        assert!(h[0].t < h[1].t);
        let graze =
            Curve3::Line(Line::new(f.point(-5.0, 2.0, 1.0), f.vector(1.0, 0.0, 0.3)).unwrap());
        assert_hits_are_exact(&graze, &cylinder, 1, true);
        let miss =
            Curve3::Line(Line::new(f.point(-5.0, 2.1, 1.0), f.vector(1.0, 0.0, 0.3)).unwrap());
        assert_hits_are_exact(&miss, &cylinder, 0, false);

        let in_plane = Curve3::Line(Line::new(f.point(1.0, 1.0, 0.0), f.x()).unwrap());
        assert_eq!(
            intersect_curve_surface(&in_plane, &plane, &tol()).unwrap(),
            CurveIntersection::OnSurface
        );
        let above = Curve3::Line(Line::new(f.point(1.0, 1.0, 0.1), f.x()).unwrap());
        assert_hits_are_exact(&above, &plane, 0, false);
        let ruling = Curve3::Line(Line::new(f.point(2.0, 0.0, 0.0), f.z()).unwrap());
        assert_eq!(
            intersect_curve_surface(&ruling, &cylinder, &tol()).unwrap(),
            CurveIntersection::OnSurface
        );
        let inner = Curve3::Line(Line::new(f.point(1.0, 0.0, 0.0), f.z()).unwrap());
        assert_hits_are_exact(&inner, &cylinder, 0, false);
    }

    #[test]
    fn circles_and_ellipses_meet_planes_and_cylinders() {
        let f = tilted();
        let circle = Curve3::Circle(Circle::new(f, 2.0).unwrap());
        let ellipse = Curve3::Ellipse(Ellipse::new(f, 3.0, 1.5).unwrap());
        let cut = Frame::new(f.point(0.5, 0.0, 0.0), f.vector(1.0, 0.3, 0.4), f.z()).unwrap();
        let plane = Surface::Plane(Plane::new(cut));
        assert_hits_are_exact(&circle, &plane, 2, false);
        assert_hits_are_exact(&ellipse, &plane, 2, false);
        let touching = Frame::new(f.point(2.0, 0.0, 0.0), f.x(), f.z()).unwrap();
        assert_hits_are_exact(&circle, &Surface::Plane(Plane::new(touching)), 1, true);
        let beyond = Frame::new(f.point(3.5, 0.0, 0.0), f.x(), f.z()).unwrap();
        assert_hits_are_exact(&ellipse, &Surface::Plane(Plane::new(beyond)), 0, false);
        assert_eq!(
            intersect_curve_surface(&circle, &Surface::Plane(Plane::new(f)), &tol()).unwrap(),
            CurveIntersection::OnSurface
        );
        let lifted = f.at(f.point(0.0, 0.0, 1.0)).unwrap();
        assert_hits_are_exact(&ellipse, &Surface::Plane(Plane::new(lifted)), 0, false);

        // A circle about a parallel axis: two circles in one plane.
        let shifted = Cylinder::new(f.at(f.point(2.5, 0.5, -3.0)).unwrap(), 1.5).unwrap();
        assert_hits_are_exact(&circle, &Surface::Cylinder(shifted), 2, false);
        let own = Cylinder::new(f.at(f.point(0.0, 0.0, 9.0)).unwrap(), 2.0).unwrap();
        assert_eq!(
            intersect_curve_surface(&circle, &Surface::Cylinder(own), &tol()).unwrap(),
            CurveIntersection::OnSurface
        );
        let kissing = Cylinder::new(f.at(f.point(3.0, 0.0, 0.0)).unwrap(), 1.0).unwrap();
        assert_hits_are_exact(&circle, &Surface::Cylinder(kissing), 1, true);
        let within = Cylinder::new(f.at(f.point(0.1, 0.0, 0.0)).unwrap(), 5.0).unwrap();
        assert_hits_are_exact(&circle, &Surface::Cylinder(within), 0, false);

        // An oblique section of one cylinder against a parallel cylinder,
        // and against its own.
        let base = Cylinder::new(f, 2.0).unwrap();
        let oblique = Frame::new(f.point(0.0, 0.0, 1.0), f.vector(0.5, 0.2, 1.0), f.x()).unwrap();
        let SurfaceIntersection::Curves(section) = intersect_surfaces(
            &Surface::Plane(Plane::new(oblique)),
            &Surface::Cylinder(base),
            &tol(),
        )
        .unwrap() else {
            panic!("expected an ellipse");
        };
        assert_hits_are_exact(&section[0], &Surface::Cylinder(shifted), 2, false);
        assert_eq!(
            intersect_curve_surface(&section[0], &Surface::Cylinder(own), &tol()).unwrap(),
            CurveIntersection::OnSurface
        );

        // Another axis direction: apart, inside, or found numerically
        // (2 sin t = ±1: four hits).
        let crossing = Frame::new(f.origin(), f.x(), f.z()).unwrap();
        assert_hits_are_exact(
            &circle,
            &Surface::Cylinder(Cylinder::new(crossing, 1.0).unwrap()),
            4,
            false,
        );
        let far = Frame::new(f.point(0.0, 0.0, 10.0), f.x(), f.z()).unwrap();
        assert_hits_are_exact(
            &circle,
            &Surface::Cylinder(Cylinder::new(far, 1.0).unwrap()),
            0,
            false,
        );
        assert_hits_are_exact(
            &circle,
            &Surface::Cylinder(Cylinder::new(crossing, 50.0).unwrap()),
            0,
            false,
        );
    }
}
