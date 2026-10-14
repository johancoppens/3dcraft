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
            None => match cone_translates(c, d, tol)? {
                Some(found) => Ok(Some(found)),
                None => shared_ruling(c, d, tol),
            },
        },
        (Surface::Cylinder(c), Surface::Torus(t)) => match coaxial(a, b, tol)? {
            Some(found) => Ok(Some(found)),
            None => continued(c, t, tol),
        },
        (Surface::Torus(t), Surface::Torus(u)) => match coaxial(a, b, tol)? {
            Some(found) => Ok(Some(found)),
            None => continued_tori(t, u, tol),
        },
        _ => coaxial(a, b, tol),
    }
}

/// A cylinder that goes on in a torus: the cylinder's radius is the
/// tube's, and its axis lies in the plane of the tube's middle circle and
/// is tangent to it (the fillet of a straight edge and of the arc that
/// edge continues in). `None` for any other position.
///
/// Both are the points at the distance `r` from a spine, a line and a
/// circle of radius `R` in one plane that touch at `O`. With `x` along the
/// line, `y` from `O` to the circle's centre and `z` along the torus'
/// axis, and `δ₁ = y`, `δ₂ = R − ρ` the distances to the spines within the
/// plane (`ρ` from the centre), a point is on both where `δ₁² = δ₂²` and
/// `z² = r² − δ₁²`:
///
/// - `δ₁ = δ₂` is the line `x = 0`: the circle of radius `r` about `O`
///   across the line, along which the two are tangent and do not cross;
/// - `δ₁ = −δ₂` is the parabola `x² = 4 R y`, on which `y = r cos² ψ`
///   with `x = 2 √(R r) cos ψ`, so `z = r sin ψ √(1 + cos² ψ)`: a closed
///   curve along which they cross, through the top and the bottom of that
///   circle and at right angles to it.
///
/// Curves end where they cross: two half circles and the two halves of
/// the second curve, all four between the top and the bottom.
fn continued(c: &Cylinder, t: &Torus, tol: &Tolerance) -> Result<Option<Closed>, GeomError> {
    let (cf, tf) = (c.frame(), t.frame());
    let (major, minor) = t.radii();
    let (a, n) = (cf.z(), tf.z());
    let r = c.radius();
    if !(major > minor + tol.linear) || a.dot(n).abs() > tol.angular {
        return Ok(None);
    }
    // The point of the axis nearest the torus' centre.
    let foot = cf.origin() + a * (tf.origin() - cf.origin()).dot(a);
    let out = foot - tf.origin();
    let lift = out.dot(n);
    let radial = across(out, n);
    let reach = radial.length();
    // How far the position is from the one the formula is for: with the
    // fit of the second curve it stays within the tolerance.
    let slack = (r - minor).abs() + lift.abs() + (reach - major).abs();
    if !(slack <= 0.5 * tol.linear) || !(reach > 0.0) {
        return Ok(None);
    }
    let inward = radial * (-1.0 / reach);
    let frame = Frame::new(foot, a, n).ok_or_else(degenerate)?;
    let circle = Circle::new(frame, r)?;
    let half = std::f64::consts::FRAC_PI_2;
    let lobe = Shape::Lobe {
        o: foot,
        a: a * (2.0 * (major * r).sqrt()),
        b: inward * r,
        c: frame.x() * r,
        k: 0.0,
        slack,
    };
    if !lobe.eval(0.0).is_finite() {
        return Err(degenerate());
    }
    let piece = |shape: &Shape, range: (f64, f64), tangent: bool| ExactCurve {
        shape: shape.clone(),
        range,
        tangent,
    };
    let round = Shape::Circle(circle);
    Ok(Some(meets(
        vec![
            piece(&round, (0.0, 2.0 * half), true),
            piece(&round, (2.0 * half, 4.0 * half), true),
            piece(&lobe, (-half, half), false),
            piece(&lobe, (half, 3.0 * half), false),
        ],
        Vec::new(),
    )))
}

/// A torus that goes on in a torus: the tubes have one radius, the axes
/// are parallel and the middle circles lie in one plane and touch, from
/// outside (an S, as a fillet along two arcs that turn opposite ways) or
/// from inside (two arcs that turn the same way with different radii).
/// `None` for any other position, and for circles that touch from inside
/// with radii less than the tube's radius apart (the second curve does
/// not reach the plane then).
///
/// As in [`continued`], with `O` where the middle circles (radii `R₁`,
/// `R₂`, centres `Δ` apart) touch, `x` along their common tangent, `y`
/// from `O` to the first centre and `z` along the axes, a point is on both
/// where `δ₁² = δ₂²` and `z² = r² − δ₁²`, `δᵢ = ρᵢ − Rᵢ` the signed
/// distances to the circles within the plane. One sign is the circle of
/// radius `r` about `O` across the plane `x = 0`, along which the tubes
/// are tangent. The other is a conic with the centres as foci (a branch
/// of a hyperbola from outside, an ellipse from inside) on which, with
/// `u = |δ₁|`, the point at `u` is where the circles about the centres of
/// radii `ρᵢ = Rᵢ + u` (outside) or `R₁ − u` and `R₂ + u` (inside) meet:
///
/// - from outside (`Δ = R₁ + R₂`): `y = (R₂ − R₁) u / Δ`,
///   `x² = (4 R₁ R₂ / Δ) u (1 + u / Δ)`;
/// - from inside (`Δ = R₁ − R₂ > 0`): `y = (R₁ + R₂) u / Δ`,
///   `x² = (4 R₁ R₂ / Δ) u (1 − u / Δ)`.
///
/// With `u = r cos² ψ` all of it is regular, through `u = r` where the
/// curve meets the plane, as long as `1 ± r/Δ > 0`: the
/// [`Shape::Lobe`] with `k = ±r/Δ` (and, as `R₂` grows without bound,
/// the cylinder's parabola).
fn continued_tori(t: &Torus, u: &Torus, tol: &Tolerance) -> Result<Option<Closed>, GeomError> {
    // The larger middle circle first, so that the inside case has
    // `R₁ > R₂`; the order given when they are equal.
    let (t, u) = if u.radii().0 > t.radii().0 {
        (u, t)
    } else {
        (t, u)
    };
    let (tf, uf) = (t.frame(), u.frame());
    let ((r1, minor), (r2, minor2)) = (t.radii(), u.radii());
    let n = tf.z();
    if !(r1 > minor + tol.linear && r2 > minor2 + tol.linear)
        || across(uf.z(), n).length() > tol.angular
    {
        return Ok(None);
    }
    let r = minor;
    let between = uf.origin() - tf.origin();
    let lift = between.dot(n);
    let planar = across(between, n);
    let apart = planar.length();
    let off = (r - minor2).abs() + lift.abs();
    let outside = (apart - (r1 + r2)).abs();
    let inside = (apart - (r1 - r2)).abs();
    let (slack, delta, from_outside) = if outside <= inside {
        (off + outside, r1 + r2, true)
    } else {
        (off + inside, r1 - r2, false)
    };
    if !(slack <= 0.5 * tol.linear) || !(apart > 0.0) || !(delta > r + tol.linear) {
        return Ok(None);
    }
    let out = planar * (1.0 / apart);
    let o = tf.origin() + out * r1;
    let toward = out * -1.0;
    let along = n.cross(toward).normalized().ok_or_else(degenerate)?;
    let frame = Frame::new(o, along, n).ok_or_else(degenerate)?;
    let circle = Circle::new(frame, r)?;
    let (rise, k) = if from_outside {
        ((r2 - r1) / delta, r / delta)
    } else {
        ((r1 + r2) / delta, -r / delta)
    };
    let half = std::f64::consts::FRAC_PI_2;
    let lobe = Shape::Lobe {
        o,
        a: along * (2.0 * (r1 * r2 * r / delta).sqrt()),
        b: toward * (rise * r),
        c: n * r,
        k,
        slack,
    };
    if !lobe.eval(0.0).is_finite() {
        return Err(degenerate());
    }
    let piece = |shape: &Shape, range: (f64, f64), tangent: bool| ExactCurve {
        shape: shape.clone(),
        range,
        tangent,
    };
    let round = Shape::Circle(circle);
    Ok(Some(meets(
        vec![
            piece(&round, (0.0, 2.0 * half), true),
            piece(&round, (2.0 * half, 4.0 * half), true),
            piece(&lobe, (-half, half), false),
            piece(&lobe, (half, 3.0 * half), false),
        ],
        Vec::new(),
    )))
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

/// The parameter of the point `p` on an exact curve, when it is on it
/// within `tol`.
fn parameter_at(shape: &Shape, p: Vec3, tol: f64) -> Option<f64> {
    let t = match shape {
        Shape::Line(l) => (p - l.origin()).dot(l.direction()),
        Shape::Circle(c) => {
            let (x, y, _) = c.frame().local(p);
            super::wrap(y.atan2(x))
        }
        Shape::Ellipse(e) => {
            let (x, y, _) = e.frame().local(p);
            let (a, b) = e.semi_axes();
            super::wrap((y / b).atan2(x / a))
        }
        Shape::Parabola { o, d1, .. } => (p - *o).dot(*d1) / d1.length_squared(),
        Shape::Hyperbola { c, e2, .. } => ((p - *c).dot(*e2) / e2.length_squared()).asinh(),
        Shape::Lobe { .. } => return None,
    };
    (t.is_finite() && shape.eval(t).distance(p) <= tol).then_some(t)
}

/// Two cones with one apex meet in the rulings whose direction `u` makes
/// the half-angle with either axis: `u·a₁ = cos α₁`, `u·a₂ = cos α₂`, two
/// circles on the unit sphere, which cross in two points, touch in one
/// (the axes are `α₁ + α₂` or `|α₁ − α₂|` apart) or miss each other, and
/// then the apex is all the cones share.
fn same_apex(
    apex: Vec3,
    (a1, alpha1): (Vec3, f64),
    (a2, alpha2): (Vec3, f64),
    tol: &Tolerance,
) -> Result<Option<Closed>, GeomError> {
    let cross = a1.cross(a2);
    let sine = cross.length();
    if sine <= tol.angular {
        // One axis, or opposite ones: not this case.
        return Ok(None);
    }
    let between = sine.atan2(a1.dot(a2));
    let ray = |direction: Vec3, tangent: bool| -> Result<ExactCurve, GeomError> {
        Ok(ExactCurve {
            shape: Shape::Line(Line::new(apex, direction)?),
            range: (0.0, f64::INFINITY),
            tangent,
        })
    };
    // In the plane of the axes, `e` across the first.
    let e = across(a2, a1).normalized().ok_or_else(degenerate)?;
    let (x, y) = (
        alpha1.cos(),
        (alpha2.cos() - alpha1.cos() * a1.dot(a2)) / sine,
    );
    let touching = (between - (alpha1 + alpha2)).abs() <= tol.angular
        || (between - (alpha1 - alpha2).abs()).abs() <= tol.angular;
    if touching {
        return Ok(Some(meets(vec![ray(a1 * x + e * y, true)?], Vec::new())));
    }
    let z2 = 1.0 - x * x - y * y;
    if !(z2 > 0.0) {
        return Ok(Some(meets(Vec::new(), vec![apex])));
    }
    let side = a1.cross(e) * z2.sqrt();
    Ok(Some(meets(
        vec![
            ray(a1 * x + e * y - side, false)?,
            ray(a1 * x + e * y + side, false)?,
        ],
        Vec::new(),
    )))
}

/// Two cones that are tangent along a ruling they share, or that have one
/// apex. `None` for any other position.
///
/// A ruling of both passes through both apexes. With `s` along that line
/// `L` from the first apex, `ν` off the tangent plane `Π` the cones share
/// along it and `τ` across the line within the plane, the axes lie in the
/// plane `τ = 0`, and a cone with its apex at `s₀` and its axis
/// `(a_s, 0, a_ν)` there (`a_s² = cos² α`, because `L` is a ruling) is
///
/// `Q = ν (2 a_s a_ν (s − s₀) + (a_ν² − cos² α) ν) − cos² α τ²`.
///
/// So `Q₁ − λ Q₂` with `λ = cos² α₁ / cos² α₂` has no `τ²` left and is
/// `ν` times a linear form `Σ`: everything the cones share lies in the
/// plane `Π` (the line, along which they are tangent) or in the plane
/// `Σ = 0` (a conic, along which they cross). Where `Σ` passes through
/// `L` the conic crosses the line, and the cones change sides there; the
/// line and the conic are both cut at that point. Only what is on the own
/// nappe of both cones is kept: a conic that does not pass through the
/// second apex is on one nappe of the second cone with all of itself.
fn shared_ruling(c: &Cone, d: &Cone, tol: &Tolerance) -> Result<Option<Closed>, GeomError> {
    let ((p1, a1), (p2, a2)) = (cone_apex_axis(c), cone_apex_axis(d));
    let (alpha1, alpha2) = (c.half_angle().abs(), d.half_angle().abs());
    let offset = p2 - p1;
    let len = offset.length();
    if !(len > tol.linear) {
        return same_apex(p1, (a1, alpha1), (a2, alpha2), tol);
    }
    let along = offset * (1.0 / len);
    // The line through the apexes is a ruling of both (of either nappe),
    // and their tangent planes along it are one plane.
    let mut normals = [Vec3::ZERO; 2];
    for ((axis, alpha), normal) in [(a1, alpha1), (a2, alpha2)].into_iter().zip(&mut normals) {
        let off = across(axis, along);
        let angle = off.length().atan2(axis.dot(along).abs());
        if (angle - alpha).abs() > tol.angular {
            return Ok(None);
        }
        *normal = off.normalized().ok_or_else(degenerate)?;
    }
    let [n, other] = normals;
    if n.cross(other).length() > tol.angular {
        return Ok(None);
    }
    let (cos1, cos2) = (alpha1.cos(), alpha2.cos());
    let lambda = (cos1 * cos1) / (cos2 * cos2);
    // Q = ν (a s + b ν + c) − cos² α τ², per cone.
    let form = |axis: Vec3, cos: f64, s0: f64| {
        let (along_axis, off_axis) = (axis.dot(along), axis.dot(n));
        let a = 2.0 * along_axis * off_axis;
        (a, off_axis * off_axis - cos * cos, -a * s0)
    };
    let ((f1a, f1b, f1c), (f2a, f2b, f2c)) = (form(a1, cos1, 0.0), form(a2, cos2, len));
    let (gs, gn, h) = (f1a - lambda * f2a, f1b - lambda * f2b, f1c - lambda * f2c);
    if !(gs.is_finite() && gn.is_finite() && h.is_finite()) {
        return Err(degenerate());
    }
    // The stretch of the line on the own nappe of both cones.
    let (mut low, mut high) = (f64::NEG_INFINITY, f64::INFINITY);
    for (axis, s0) in [(a1, 0.0), (a2, len)] {
        if axis.dot(along) > 0.0 {
            low = low.max(s0);
        } else {
            high = high.min(s0);
        }
    }
    // Where the second plane passes through the line.
    let crossing = (gs.abs() > tol.angular).then(|| -h / gs);
    let line = Shape::Line(Line::new(p1, along)?);
    let mut curves = Vec::new();
    if high > low {
        let cut = crossing.filter(|s| *s > low + tol.linear && *s < high - tol.linear);
        let mut stops = vec![low];
        stops.extend(cut);
        stops.push(high);
        for (from, to) in stops.iter().zip(stops.iter().skip(1)) {
            curves.push(ExactCurve {
                shape: line.clone(),
                range: (*from, *to),
                tangent: true,
            });
        }
    }
    let size = gs.hypot(gn);
    if !(size > tol.angular) {
        // No second plane: the line is all there is.
        return Ok(Some(meets(curves, Vec::new())));
    }
    let normal = along * gs + n * gn;
    let origin = p1 - normal * (h / (size * size));
    let Some(frame) = Frame::from_z(origin, normal) else {
        return Ok(None);
    };
    let section = match plane_cone(&Plane::new(frame), c, tol) {
        Ok(Closed::Meets { curves, points }) if points.is_empty() => curves,
        Ok(Closed::None) => Vec::new(),
        // Through the first apex, or too near a ruling's direction.
        Ok(_) | Err(GeomError::Unsupported(_)) => return Ok(None),
        Err(other) => return Err(other),
    };
    let through = crossing.map(|s| p1 + along * s);
    for conic in section {
        let (from, to) = conic.range;
        let at = through.and_then(|p| parameter_at(&conic.shape, p, tol.linear));
        let closed = matches!(conic.shape, Shape::Circle(_) | Shape::Ellipse(_));
        let pieces = match at {
            Some(t) if closed => {
                let half = std::f64::consts::PI;
                vec![(t, t + half), (t + half, t + 2.0 * half)]
            }
            Some(t) if t > from && t < to => vec![(from, t), (t, to)],
            _ => vec![(from, to)],
        };
        for (t0, t1) in pieces {
            // One point of a piece decides for all of it.
            let middle = match (t0.is_finite(), t1.is_finite()) {
                (true, true) => 0.5 * (t0 + t1),
                (true, false) => t0 + 1.0,
                (false, true) => t1 - 1.0,
                (false, false) => 0.0,
            };
            if (conic.shape.eval(middle) - p2).dot(a2) > 0.0 {
                curves.push(ExactCurve {
                    shape: conic.shape.clone(),
                    range: (t0, t1),
                    tangent: false,
                });
            }
        }
    }
    Ok(Some(meets(curves, Vec::new())))
}
