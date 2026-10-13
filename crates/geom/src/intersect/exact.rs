//! Surface pairs whose intersection has a closed form.
//!
//! Derived from the implicit equations of the quadrics and the torus
//! (analytic geometry; Mortenson, *Geometric Modeling*, on quadric
//! intersections). Three ideas cover everything here:
//!
//! - a plane cuts a quadric in a conic, found by writing the quadric's
//!   equation in coordinates of the plane;
//! - surfaces of revolution about one axis meet in circles about it, where
//!   their profiles in a half-plane through the axis meet;
//! - two translates of one quadric (equal spheres, equal cylinders or cones
//!   with parallel axes, and equal cylinders mirrored in a plane) differ by
//!   a linear equation, so they meet in a plane.

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::clip::{ExactCurve, Shape};
use super::field::cone_apex_axis;
use super::{
    NEARLY_PARALLEL, SurfaceIntersection, across, circle_circle, cylinder_cylinder, plane_cylinder,
    plane_plane,
};
use crate::{
    Circle, Cone, Curve3, Cylinder, Ellipse, GeomError, Line, Plane, Sphere, Surface, Torus,
};
use craft3d_math::{Frame, Tolerance, Vec3};

/// An intersection in closed form, before it is clipped to a box.
pub(super) enum Closed {
    None,
    Coincident {
        same_normal: bool,
    },
    Meets {
        curves: Vec<ExactCurve>,
        points: Vec<Vec3>,
    },
}

fn degenerate() -> GeomError {
    GeomError::InvalidParameter("an intersection has no finite position")
}

fn meets(curves: Vec<ExactCurve>, points: Vec<Vec3>) -> Closed {
    if curves.is_empty() && points.is_empty() {
        Closed::None
    } else {
        Closed::Meets { curves, points }
    }
}

fn from_legacy(found: Result<SurfaceIntersection, GeomError>) -> Result<Option<Closed>, GeomError> {
    let exact = |curves: Vec<Curve3>, tangent: bool| {
        meets(
            curves
                .iter()
                .filter_map(|c| ExactCurve::from_curve(c, tangent))
                .collect(),
            Vec::new(),
        )
    };
    Ok(Some(match found {
        Ok(SurfaceIntersection::None) => Closed::None,
        Ok(SurfaceIntersection::Coincident { same_normal }) => Closed::Coincident { same_normal },
        Ok(SurfaceIntersection::Curves(curves)) => exact(curves, false),
        Ok(SurfaceIntersection::Tangent(curves)) => exact(curves, true),
        // Too close to parallel for the closed form: left to the marcher.
        Err(GeomError::Unsupported(_)) => return Ok(None),
        Err(other) => return Err(other),
    }))
}

/// The closed form for `a` and `b`, or `None` when there is none. The
/// kinds are in the order plane, cylinder, cone, sphere, torus.
pub(super) fn closed_form(
    a: &Surface,
    b: &Surface,
    tol: &Tolerance,
) -> Result<Option<Closed>, GeomError> {
    match (a, b) {
        (Surface::Plane(p), Surface::Plane(q)) => {
            from_legacy(plane_plane(p.frame(), q.frame(), tol))
        }
        (Surface::Plane(p), Surface::Cylinder(c)) => {
            from_legacy(plane_cylinder(p.frame(), c.frame(), c.radius(), tol))
        }
        (Surface::Plane(p), Surface::Cone(c)) => plane_cone(p, c, tol).map(Some),
        (Surface::Plane(p), Surface::Sphere(s)) => plane_sphere(p, s, tol).map(Some),
        (Surface::Plane(p), Surface::Torus(t)) => plane_torus(p, t, tol),
        (Surface::Cylinder(c), Surface::Cylinder(d)) => cylinders(c, d, tol),
        (Surface::Sphere(s), Surface::Sphere(t)) => sphere_sphere(s, t, tol).map(Some),
        (Surface::Cone(c), Surface::Cone(d)) => match coaxial(a, b, tol)? {
            Some(found) => Ok(Some(found)),
            None => cone_translates(c, d, tol),
        },
        _ => coaxial(a, b, tol),
    }
}

fn plane_sphere(p: &Plane, s: &Sphere, tol: &Tolerance) -> Result<Closed, GeomError> {
    let (f, r) = (p.frame(), s.radius());
    let n = f.z();
    let center = s.frame().origin();
    let d = (center - f.origin()).dot(n);
    if d.abs() > r + tol.linear {
        return Ok(Closed::None);
    }
    let foot = center - n * d;
    if (d.abs() - r).abs() <= tol.linear {
        return Ok(meets(Vec::new(), vec![foot]));
    }
    let radius = ((r - d) * (r + d)).max(0.0).sqrt();
    let circle = Circle::new(f.at(foot).ok_or_else(degenerate)?, radius)?;
    Ok(meets(
        vec![ExactCurve::whole(Shape::Circle(circle), false)],
        Vec::new(),
    ))
}

fn sphere_sphere(s: &Sphere, t: &Sphere, tol: &Tolerance) -> Result<Closed, GeomError> {
    let (c1, c2) = (s.frame().origin(), t.frame().origin());
    let (r1, r2) = (s.radius(), t.radius());
    let distance = c1.distance(c2);
    if distance <= tol.linear {
        return Ok(if (r1 - r2).abs() <= tol.linear {
            Closed::Coincident { same_normal: true }
        } else {
            Closed::None
        });
    }
    let Some((along, half, touching)) = circle_circle(distance, r1, r2, tol.linear) else {
        return Ok(Closed::None);
    };
    let toward = (c2 - c1) * (1.0 / distance);
    let center = c1 + toward * along;
    // A circle no wider than the tolerance is a touch as well: the
    // spheres are apart by the tolerance to the last bit, which is neither
    // "apart" nor "touching" above.
    if touching || half <= tol.linear {
        return Ok(meets(Vec::new(), vec![center]));
    }
    let frame = Frame::from_z(center, toward).ok_or_else(degenerate)?;
    Ok(meets(
        vec![ExactCurve::whole(
            Shape::Circle(Circle::new(frame, half)?),
            false,
        )],
        Vec::new(),
    ))
}

/// A plane and a cone. In the plane, with the origin at the foot of the
/// apex, `e1` across the axis and `e2` along the axis' shadow, a point
/// `x e1 + y e2` lies at `q = -d n + x e1 + y e2` from the apex (`d` the
/// apex's height above the plane), and the cone `(q·a)² = cos²α |q|²` reads
///
/// `cos²α x² + (cos²α - s²) y² + 2 d c s y + (cos²α - c²) d² = 0`
///
/// with `c = n·a`, `s = |n × a|`. The sign of `cos²α - s²` tells ellipse,
/// parabola and hyperbola apart; `q·a > 0` keeps the cone's own nappe.
fn plane_cone(p: &Plane, cone: &Cone, tol: &Tolerance) -> Result<Closed, GeomError> {
    let f = p.frame();
    let n = f.z();
    let (apex, a) = cone_apex_axis(cone);
    let (sin_a, cos_a) = cone.half_angle().abs().sin_cos();
    let d = (apex - f.origin()).dot(n);
    let c = n.dot(a);
    let cross = n.cross(a);
    let s = cross.length();
    // The plane's angle with the axis against the half-angle, by sines.
    let steeper = c.abs() - sin_a;
    let kind = if steeper.abs() <= tol.angular {
        0
    } else if steeper.abs() < NEARLY_PARALLEL {
        return Err(GeomError::Unsupported(
            "a plane that is nearly, but not, parallel to a ruling of a cone",
        ));
    } else if steeper > 0.0 {
        1 // ellipse
    } else {
        -1 // hyperbola
    };
    let (cos2, s2) = (cos_a * cos_a, s * s);
    let b2 = cos2 - s2;
    if d.abs() <= tol.linear {
        // Through the apex: the apex alone, one ruling, or two.
        if kind > 0 {
            return Ok(meets(Vec::new(), vec![apex]));
        }
        let e1 = cross.normalized().ok_or_else(degenerate)?;
        let e2 = e1.cross(n);
        let ray = |direction: Vec3, tangent: bool| -> Result<ExactCurve, GeomError> {
            Ok(ExactCurve {
                shape: Shape::Line(Line::new(apex, direction)?),
                range: (0.0, f64::INFINITY),
                tangent,
            })
        };
        if kind == 0 {
            return Ok(meets(vec![ray(e2, true)?], Vec::new()));
        }
        let spread = (-b2 / cos2).max(0.0).sqrt();
        return Ok(meets(
            vec![ray(e2 - e1 * spread, false)?, ray(e2 + e1 * spread, false)?],
            Vec::new(),
        ));
    }
    if kind >= 0 && d * c >= 0.0 {
        return Ok(Closed::None); // The section is on the other nappe.
    }
    if s <= tol.angular {
        // Across the axis: a circle with the cone's own axes.
        let along = -d / c;
        let center = apex + a * along;
        let frame = cone.frame().at(center).ok_or_else(degenerate)?;
        let circle = Circle::new(frame, along * sin_a / cos_a)?;
        return Ok(meets(
            vec![ExactCurve::whole(Shape::Circle(circle), false)],
            Vec::new(),
        ));
    }
    let e1 = cross * (1.0 / s);
    let e2 = e1.cross(n);
    let foot = apex - n * d;
    let shape = if kind > 0 {
        let center = foot + e2 * (-d * c * s / b2);
        let major = d.abs() * cos_a * sin_a / b2;
        let minor = d.abs() * sin_a / b2.sqrt();
        let normal = if n.dot(cone.frame().z()) > 0.0 { n } else { -n };
        let frame = Frame::new(center, normal, e2).ok_or_else(degenerate)?;
        Shape::Ellipse(Ellipse::new(frame, major, minor)?)
    } else if kind == 0 {
        let k = -1.0 / (2.0 * d * c * s);
        Shape::Parabola {
            o: foot + e2 * (k * (cos2 - c * c) * d * d),
            d1: e1,
            d2: e2 * (k * cos2),
        }
    } else {
        let bp = -b2;
        let y0 = d * c * s / bp;
        let hy = d.abs() * cos_a * sin_a / bp;
        let hx = d.abs() * sin_a / bp.sqrt();
        // The branch on the cone's nappe: q·a = y s - d c > 0 at its vertex.
        let side = if (y0 + hy) * s - d * c > 0.0 {
            1.0
        } else {
            -1.0
        };
        Shape::Hyperbola {
            c: foot + e2 * y0,
            e1: e2 * (side * hy),
            e2: e1 * hx,
        }
    };
    if !shape.eval(0.0).is_finite() {
        return Err(degenerate());
    }
    Ok(meets(vec![ExactCurve::whole(shape, false)], Vec::new()))
}

/// A plane across a torus' axis (circles about it) or through the axis (two
/// sections of the tube); every other position is a quartic.
fn plane_torus(p: &Plane, t: &Torus, tol: &Tolerance) -> Result<Option<Closed>, GeomError> {
    let (pf, tf) = (p.frame(), t.frame());
    let (major, minor) = t.radii();
    let n = pf.z();
    let z = tf.z();
    let whole = |circle: Circle, tangent: bool| ExactCurve::whole(Shape::Circle(circle), tangent);
    if n.cross(z).length() <= tol.angular {
        let d = (pf.origin() - tf.origin()).dot(z);
        if d.abs() > minor + tol.linear {
            return Ok(Some(Closed::None));
        }
        let frame = tf.at(tf.origin() + z * d).ok_or_else(degenerate)?;
        if (d.abs() - minor).abs() <= tol.linear {
            return Ok(Some(meets(
                vec![whole(Circle::new(frame, major)?, true)],
                Vec::new(),
            )));
        }
        let w = ((minor - d) * (minor + d)).max(0.0).sqrt();
        return Ok(Some(meets(
            vec![
                whole(Circle::new(frame, major - w)?, false),
                whole(Circle::new(frame, major + w)?, false),
            ],
            Vec::new(),
        )));
    }
    if n.dot(z).abs() <= tol.angular && (tf.origin() - pf.origin()).dot(n).abs() <= tol.linear {
        let e = z.cross(n).normalized().ok_or_else(degenerate)?;
        let mut curves = Vec::new();
        for radial in [e, -e] {
            // x outward and y along the axis: the angle is the torus' v.
            let frame = Frame::new(tf.origin() + radial * major, radial.cross(z), radial)
                .ok_or_else(degenerate)?;
            curves.push(whole(Circle::new(frame, minor)?, false));
        }
        return Ok(Some(meets(curves, Vec::new())));
    }
    Ok(None)
}

/// Two cylinders: parallel axes (lines), or equal radii about axes that
/// cross, which meet in the two mirror planes that swap the axes: two
/// ellipses through the two points where the cylinders touch. Each ellipse
/// is given as two halves that end on those points.
fn cylinders(c: &Cylinder, d: &Cylinder, tol: &Tolerance) -> Result<Option<Closed>, GeomError> {
    let (cf, df) = (c.frame(), d.frame());
    let (z1, z2) = (cf.z(), df.z());
    let cross = z1.cross(z2);
    let sine = cross.length();
    if sine <= tol.angular {
        return from_legacy(cylinder_cylinder(cf, c.radius(), df, d.radius(), tol));
    }
    let offset = df.origin() - cf.origin();
    let apart = offset.dot(cross).abs() / sine;
    if apart > c.radius() + d.radius() + tol.linear {
        return Ok(Some(Closed::None));
    }
    if sine < NEARLY_PARALLEL || apart > tol.linear || (c.radius() - d.radius()).abs() > tol.linear
    {
        return Ok(None);
    }
    let r = c.radius();
    let m = cross * (1.0 / sine);
    // Where the axes cross: on the first axis, in the plane of the second
    // axis and the common normal.
    let across_second = z2.cross(m);
    let along = offset.dot(across_second) / z1.dot(across_second);
    let center = cf.origin() + z1 * along;
    if !center.is_finite() {
        return Err(degenerate());
    }
    let mut curves = Vec::new();
    for normal in [z1 - z2, z1 + z2] {
        let Some(mirror) = Frame::new(center, normal, m) else {
            return Ok(None);
        };
        let Ok(SurfaceIntersection::Curves(section)) = plane_cylinder(&mirror, cf, r, tol) else {
            return Ok(None);
        };
        let Some(Curve3::Ellipse(ellipse)) = section.first() else {
            return Ok(None);
        };
        let (ef, (a, b)) = (ellipse.frame(), ellipse.semi_axes());
        let angle = |p: Vec3| {
            let (x, y, _) = ef.local(p);
            super::wrap((y / b).atan2(x / a))
        };
        let (t0, t1) = (angle(center + m * r), angle(center - m * r));
        let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
        for range in [(t0, t1), (t1, t0 + std::f64::consts::TAU)] {
            curves.push(ExactCurve {
                shape: Shape::Ellipse(*ellipse),
                range,
                tangent: false,
            });
        }
    }
    Ok(Some(meets(curves, Vec::new())))
}

/// A profile in the half-plane `(ρ, z)` through a common axis.
#[derive(Clone, Copy)]
enum Profile {
    /// `ρ = p + q z`, where that is not negative.
    Ruling {
        p: f64,
        q: f64,
    },
    Round {
        rho: f64,
        z: f64,
        r: f64,
    },
}

/// The frame of a surface of revolution, when it has one axis.
fn axis_of(s: &Surface) -> Option<&Frame> {
    match s {
        Surface::Cylinder(c) => Some(c.frame()),
        Surface::Cone(c) => Some(c.frame()),
        Surface::Torus(t) => Some(t.frame()),
        _ => None,
    }
}

/// The profile of `s` about the z axis of `frame`, or `None` when `s` is
/// not a surface of revolution about that axis.
fn profile(s: &Surface, frame: &Frame, tol: &Tolerance) -> Option<Profile> {
    let z = frame.z();
    let on_axis = |own: &Frame| {
        let offset = own.origin() - frame.origin();
        (z.cross(own.z()).length() <= tol.angular && across(offset, z).length() <= tol.linear)
            .then(|| (offset.dot(z), if z.dot(own.z()) > 0.0 { 1.0 } else { -1.0 }))
    };
    match s {
        Surface::Cylinder(c) => on_axis(c.frame()).map(|_| Profile::Ruling {
            p: c.radius(),
            q: 0.0,
        }),
        Surface::Cone(c) => on_axis(c.frame()).map(|(h, side)| {
            let q = c.half_angle().tan() * side;
            Profile::Ruling {
                p: c.radius() - q * h,
                q,
            }
        }),
        Surface::Sphere(sp) => {
            let offset = sp.frame().origin() - frame.origin();
            (across(offset, z).length() <= tol.linear).then(|| Profile::Round {
                rho: 0.0,
                z: offset.dot(z),
                r: sp.radius(),
            })
        }
        Surface::Torus(t) => on_axis(t.frame()).map(|(h, _)| Profile::Round {
            rho: t.radii().0,
            z: h,
            r: t.radii().1,
        }),
        Surface::Plane(_) | Surface::Bezier(_) | Surface::Nurbs(_) => None,
    }
}

/// Surfaces of revolution about one axis: circles where their profiles
/// meet. `None` when they do not share an axis.
fn coaxial(a: &Surface, b: &Surface, tol: &Tolerance) -> Result<Option<Closed>, GeomError> {
    let Some(frame) = axis_of(a).or_else(|| axis_of(b)) else {
        return Ok(None);
    };
    let (Some(pa), Some(pb)) = (profile(a, frame, tol), profile(b, frame, tol)) else {
        return Ok(None);
    };
    let t = tol.linear;
    // (ρ, z, tangent) of every point the profiles share.
    let mut hits: Vec<(f64, f64, bool)> = Vec::new();
    match (pa, pb) {
        (Profile::Ruling { p: p1, q: q1 }, Profile::Ruling { p: p2, q: q2 }) => {
            if (q1 - q2).abs() <= tol.angular {
                let gap = (p1 - p2).abs() / q1.hypot(1.0);
                return Ok(Some(if gap <= t {
                    Closed::Coincident { same_normal: true }
                } else {
                    Closed::None
                }));
            }
            let z = (p2 - p1) / (q1 - q2);
            hits.push((p1 + q1 * z, z, false));
        }
        (Profile::Ruling { p, q }, Profile::Round { rho, z, r })
        | (Profile::Round { rho, z, r }, Profile::Ruling { p, q }) => {
            let norm = q.hypot(1.0);
            // The centre's distance from the line, along its normal (1, -q).
            let s = (rho - p - q * z) / norm;
            if s.abs() > r + t {
                return Ok(Some(Closed::None));
            }
            let (fr, fz) = (rho - s / norm, z + s * q / norm);
            if (s.abs() - r).abs() <= t {
                hits.push((fr, fz, true));
            } else {
                let half = ((r - s) * (r + s)).max(0.0).sqrt() / norm;
                hits.push((fr - q * half, fz - half, false));
                hits.push((fr + q * half, fz + half, false));
            }
        }
        (
            Profile::Round {
                rho: r1,
                z: z1,
                r: a1,
            },
            Profile::Round {
                rho: r2,
                z: z2,
                r: a2,
            },
        ) => {
            let distance = (r2 - r1).hypot(z2 - z1);
            if distance <= t {
                return Ok(Some(if (a1 - a2).abs() <= t {
                    Closed::Coincident { same_normal: true }
                } else {
                    Closed::None
                }));
            }
            let Some((along, half, touching)) = circle_circle(distance, a1, a2, t) else {
                return Ok(Some(Closed::None));
            };
            let (dr, dz) = ((r2 - r1) / distance, (z2 - z1) / distance);
            let (fr, fz) = (r1 + dr * along, z1 + dz * along);
            if touching {
                hits.push((fr, fz, true));
            } else {
                hits.push((fr + dz * half, fz - dr * half, false));
                hits.push((fr - dz * half, fz + dr * half, false));
            }
        }
    }
    let (mut curves, mut points) = (Vec::new(), Vec::new());
    for (rho, z, tangent) in hits {
        if !(rho.is_finite() && z.is_finite()) {
            return Err(degenerate());
        }
        if rho < -t {
            continue; // On the far side of the axis: not on the surfaces.
        }
        let center = frame.point(0.0, 0.0, z);
        if rho <= t {
            points.push(center);
        } else {
            let circle = Circle::new(frame.at(center).ok_or_else(degenerate)?, rho)?;
            curves.push(ExactCurve::whole(Shape::Circle(circle), tangent));
        }
    }
    Ok(Some(meets(curves, points)))
}

/// Two cones with parallel axes and equal half-angles are translates of one
/// quadric: subtracting their equations leaves the plane they meet in. With
/// `w` across and `h` along the first axis from the first apex to the
/// second, and `k = tan α`: `q·(k² h a - w) = (k² h² - |w|²) / 2`.
fn cone_translates(c: &Cone, d: &Cone, tol: &Tolerance) -> Result<Option<Closed>, GeomError> {
    let ((apex1, a1), (apex2, a2)) = (cone_apex_axis(c), cone_apex_axis(d));
    if a1.cross(a2).length() > tol.angular
        || (c.half_angle().abs() - d.half_angle().abs()).abs() > tol.angular
    {
        return Ok(None);
    }
    let k2 = c.half_angle().tan().powi(2);
    let t = apex2 - apex1;
    let h = t.dot(a1);
    let w = t - a1 * h;
    let normal = a1 * (k2 * h) - w;
    let length = normal.length();
    let offset = 0.5 * (k2 * h * h - w.length_squared());
    if !(length > 0.0) || offset.abs() <= tol.linear * length {
        // The plane passes through both apexes: rulings shared or crossing
        // at an apex, which the marcher may or may not resolve.
        return Ok(None);
    }
    let origin = apex1 + normal * (offset / (length * length));
    let Some(frame) = Frame::from_z(origin, normal) else {
        return Ok(None);
    };
    let Closed::Meets { curves, .. } = plane_cone(&Plane::new(frame), c, tol)? else {
        return Ok(Some(Closed::None));
    };
    // A conic that misses the second apex lies on one nappe of the second
    // cone: one point decides for the whole curve.
    let kept = curves
        .into_iter()
        .filter(|curve| (curve.shape.eval(0.0) - apex2).dot(a2) > 0.0)
        .collect();
    Ok(Some(meets(kept, Vec::new())))
}
