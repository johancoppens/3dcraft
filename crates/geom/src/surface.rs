//! Surfaces: analytic kinds stay analytic, everything else is Bézier or NURBS.
//!
//! Every analytic surface is placed by a [`Frame`] and parametrised so that
//! `∂u × ∂v` points away from the axis or centre (outward).

use crate::curve::wrap_angle;
use crate::{BezierPatch, GeomError, NurbsSurface};
use craft3d_math::{Frame, Vec3};
use std::f64::consts::{FRAC_PI_2, TAU};

fn positive(value: f64, message: &'static str) -> Result<f64, GeomError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(GeomError::InvalidParameter(message))
    }
}

/// The xy plane of a frame: `(u, v) ↦ origin + u x + v y`, normal `z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    frame: Frame,
}

impl Plane {
    pub fn new(frame: Frame) -> Self {
        Self { frame }
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }
}

/// A cylinder about the frame's z axis: `(r cos u, r sin u, v)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cylinder {
    frame: Frame,
    radius: f64,
}

impl Cylinder {
    pub fn new(frame: Frame, radius: f64) -> Result<Self, GeomError> {
        Ok(Self {
            frame,
            radius: positive(radius, "radius must be positive and finite")?,
        })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }
}

/// A cone about the frame's z axis: at height `v` the radius is
/// `radius + v tan(half_angle)`. `v` is limited to where that is not
/// negative, so the surface ends at its apex.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cone {
    frame: Frame,
    radius: f64,
    half_angle: f64,
}

impl Cone {
    /// `radius ≥ 0` is the radius at `v = 0`; `half_angle` is the angle
    /// between axis and surface, in `(-π/2, π/2)` and not zero (that would
    /// be a cylinder). Positive widens along +z.
    pub fn new(frame: Frame, radius: f64, half_angle: f64) -> Result<Self, GeomError> {
        if !(radius.is_finite() && radius >= 0.0) {
            return Err(GeomError::InvalidParameter(
                "cone radius must be finite and not negative",
            ));
        }
        if !(half_angle.is_finite() && half_angle.abs() < FRAC_PI_2 && half_angle != 0.0) {
            return Err(GeomError::InvalidParameter(
                "cone half-angle must be non-zero and strictly between -90° and 90°",
            ));
        }
        Ok(Self {
            frame,
            radius,
            half_angle,
        })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }

    pub fn half_angle(&self) -> f64 {
        self.half_angle
    }

    /// The `v` of the apex.
    pub fn apex_v(&self) -> f64 {
        -self.radius / self.half_angle.tan()
    }

    fn radius_at(&self, v: f64) -> f64 {
        self.radius + v * self.half_angle.tan()
    }
}

/// A sphere about the frame's origin: longitude `u`, latitude `v` in
/// `[-π/2, π/2]`: `r (cos v cos u, cos v sin u, sin v)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    frame: Frame,
    radius: f64,
}

impl Sphere {
    pub fn new(frame: Frame, radius: f64) -> Result<Self, GeomError> {
        Ok(Self {
            frame,
            radius: positive(radius, "radius must be positive and finite")?,
        })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }
}

/// A ring torus about the frame's z axis:
/// `((R + r cos v) cos u, (R + r cos v) sin u, r sin v)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Torus {
    frame: Frame,
    major: f64,
    minor: f64,
}

impl Torus {
    /// `major > minor > 0`: the tube must not reach the axis.
    pub fn new(frame: Frame, major: f64, minor: f64) -> Result<Self, GeomError> {
        positive(minor, "torus radii must be positive and finite")?;
        positive(major, "torus radii must be positive and finite")?;
        if major <= minor {
            return Err(GeomError::InvalidParameter(
                "torus major radius must be larger than its minor radius",
            ));
        }
        Ok(Self {
            frame,
            major,
            minor,
        })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// `(major, minor)`.
    pub fn radii(&self) -> (f64, f64) {
        (self.major, self.minor)
    }
}

/// Any surface the kernel knows. No code may assume a particular kind
/// (AGENTS.md rule 3).
#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    Plane(Plane),
    Cylinder(Cylinder),
    Cone(Cone),
    Sphere(Sphere),
    Torus(Torus),
    Bezier(BezierPatch),
    Nurbs(NurbsSurface),
}

impl Surface {
    /// A short name for messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Surface::Plane(_) => "plane",
            Surface::Cylinder(_) => "cylinder",
            Surface::Cone(_) => "cone",
            Surface::Sphere(_) => "sphere",
            Surface::Torus(_) => "torus",
            Surface::Bezier(_) => "bezier patch",
            Surface::Nurbs(_) => "nurbs surface",
        }
    }

    /// `((u start, u end), (v start, v end))`; infinite where the surface
    /// is unbounded, one period where it closes on itself.
    pub fn domain(&self) -> ((f64, f64), (f64, f64)) {
        let all = (f64::NEG_INFINITY, f64::INFINITY);
        let turn = (0.0, TAU);
        match self {
            Surface::Plane(_) => (all, all),
            Surface::Cylinder(_) => (turn, all),
            Surface::Cone(c) => {
                let apex = c.apex_v();
                (
                    turn,
                    if c.half_angle > 0.0 {
                        (apex, f64::INFINITY)
                    } else {
                        (f64::NEG_INFINITY, apex)
                    },
                )
            }
            Surface::Sphere(_) => (turn, (-FRAC_PI_2, FRAC_PI_2)),
            Surface::Torus(_) => (turn, turn),
            Surface::Bezier(_) => ((0.0, 1.0), (0.0, 1.0)),
            Surface::Nurbs(s) => s.domain(),
        }
    }

    /// The period in u, for surfaces closed around their axis.
    pub fn period_u(&self) -> Option<f64> {
        match self {
            Surface::Cylinder(_) | Surface::Cone(_) | Surface::Sphere(_) | Surface::Torus(_) => {
                Some(TAU)
            }
            Surface::Plane(_) | Surface::Bezier(_) | Surface::Nurbs(_) => None,
        }
    }

    /// The period in v (only the torus).
    pub fn period_v(&self) -> Option<f64> {
        matches!(self, Surface::Torus(_)).then_some(TAU)
    }

    pub fn eval(&self, u: f64, v: f64) -> Vec3 {
        self.derivatives(u, v).0
    }

    /// `(point, ∂/∂u, ∂/∂v)` at `(u, v)`.
    pub fn derivatives(&self, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
        let (su, cu) = u.sin_cos();
        match self {
            Surface::Plane(p) => (p.frame.point(u, v, 0.0), p.frame.x(), p.frame.y()),
            Surface::Cylinder(c) => {
                let r = c.radius;
                (
                    c.frame.point(r * cu, r * su, v),
                    c.frame.vector(-r * su, r * cu, 0.0),
                    c.frame.z(),
                )
            }
            Surface::Cone(c) => {
                let (r, slope) = (c.radius_at(v), c.half_angle.tan());
                (
                    c.frame.point(r * cu, r * su, v),
                    c.frame.vector(-r * su, r * cu, 0.0),
                    c.frame.vector(slope * cu, slope * su, 1.0),
                )
            }
            Surface::Sphere(s) => {
                let (sv, cv) = v.sin_cos();
                let r = s.radius;
                (
                    s.frame.point(r * cv * cu, r * cv * su, r * sv),
                    s.frame.vector(-r * cv * su, r * cv * cu, 0.0),
                    s.frame.vector(-r * sv * cu, -r * sv * su, r * cv),
                )
            }
            Surface::Torus(t) => {
                let (sv, cv) = v.sin_cos();
                let ring = t.major + t.minor * cv;
                (
                    t.frame.point(ring * cu, ring * su, t.minor * sv),
                    t.frame.vector(-ring * su, ring * cu, 0.0),
                    t.frame
                        .vector(-t.minor * sv * cu, -t.minor * sv * su, t.minor * cv),
                )
            }
            Surface::Bezier(b) => b.derivatives(u, v),
            Surface::Nurbs(s) => s.derivatives(u, v),
        }
    }

    /// Unit normal `∂u × ∂v` at `(u, v)`. Analytic kinds use their closed
    /// form, which also holds at the poles of a sphere; `None` at a cone's
    /// apex and wherever a freeform surface has no normal.
    pub fn normal(&self, u: f64, v: f64) -> Option<Vec3> {
        let (su, cu) = u.sin_cos();
        match self {
            Surface::Plane(p) => Some(p.frame.z()),
            Surface::Cylinder(c) => Some(c.frame.vector(cu, su, 0.0)),
            Surface::Cone(c) => {
                // At the apex, up to the rounding of `radius + v tan α`.
                let slope = c.half_angle.tan();
                if c.radius_at(v) <= 1e-12 * (c.radius + (v * slope).abs()) {
                    return None;
                }
                c.frame.vector(cu, su, -c.half_angle.tan()).normalized()
            }
            Surface::Sphere(s) => {
                let (sv, cv) = v.sin_cos();
                Some(s.frame.vector(cv * cu, cv * su, sv))
            }
            Surface::Torus(t) => {
                let (sv, cv) = v.sin_cos();
                Some(t.frame.vector(cv * cu, cv * su, sv))
            }
            Surface::Bezier(b) => b.normal(u, v),
            Surface::Nurbs(s) => {
                let (_, du, dv) = s.derivatives(u, v);
                du.cross(dv).normalized()
            }
        }
    }

    /// The parameters of the point on the surface closest to `p`.
    ///
    /// Closed form for the analytic kinds; where the closest point is not
    /// unique (on a cylinder's axis, at a sphere's centre) the free
    /// parameter is 0. Gauss–Newton from the best of a grid of samples for
    /// Bézier and NURBS surfaces, which finds the closest point unless the
    /// surface is more wiggly than the grid resolves.
    pub fn project(&self, p: Vec3) -> Result<(f64, f64), GeomError> {
        if !p.is_finite() {
            return Err(GeomError::InvalidParameter(
                "cannot project a non-finite point",
            ));
        }
        let angle = |y: f64, x: f64| {
            if x == 0.0 && y == 0.0 {
                0.0
            } else {
                wrap_angle(y.atan2(x))
            }
        };
        Ok(match self {
            Surface::Plane(s) => {
                let (x, y, _) = s.frame.local(p);
                (x, y)
            }
            Surface::Cylinder(s) => {
                let (x, y, z) = s.frame.local(p);
                (angle(y, x), z)
            }
            Surface::Cone(s) => {
                let (x, y, z) = s.frame.local(p);
                // In the half-plane (ρ, z) the cone is the ray from the
                // apex along (tan α, 1); project onto it, stopping at the apex.
                let slope = s.half_angle.tan();
                let v = ((x.hypot(y) - s.radius) * slope + z) / (slope * slope + 1.0);
                let apex = s.apex_v();
                (
                    angle(y, x),
                    if s.half_angle > 0.0 {
                        v.max(apex)
                    } else {
                        v.min(apex)
                    },
                )
            }
            Surface::Sphere(s) => {
                let (x, y, z) = s.frame.local(p);
                let rho = x.hypot(y);
                (
                    angle(y, x),
                    if rho == 0.0 && z == 0.0 {
                        0.0
                    } else {
                        z.atan2(rho)
                    },
                )
            }
            Surface::Torus(s) => {
                let (x, y, z) = s.frame.local(p);
                let off = x.hypot(y) - s.major;
                (angle(y, x), angle(z, off))
            }
            Surface::Bezier(_) | Surface::Nurbs(_) => self.project_numeric(p)?,
        })
    }

    fn project_numeric(&self, p: Vec3) -> Result<(f64, f64), GeomError> {
        let ((u0, u1), (v0, v1)) = self.domain();
        if ![u0, u1, v0, v1].iter().all(|x| x.is_finite()) {
            return Err(GeomError::Unsupported(
                "numeric projection onto an unbounded surface",
            ));
        }
        const GRID: u32 = 16;
        let mut best = (f64::INFINITY, u0, v0);
        for i in 0..=GRID {
            for j in 0..=GRID {
                let u = u0 + (u1 - u0) * f64::from(i) / f64::from(GRID);
                let v = v0 + (v1 - v0) * f64::from(j) / f64::from(GRID);
                let d = self.eval(u, v).distance(p);
                if d < best.0 {
                    best = (d, u, v);
                }
            }
        }
        // Gauss–Newton on the residual S(u, v) - p: solve the 2 × 2 normal
        // equations of [Su Sv] δ = p - S.
        let (mut u, mut v) = (best.1, best.2);
        for _ in 0..50 {
            let (s, su, sv) = self.derivatives(u, v);
            let r = p - s;
            let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
            let det = a * c - b * b;
            if !(det.is_finite() && det > 1e-14 * a * c) {
                break; // Degenerate tangent plane; keep what we have.
            }
            let (ru, rv) = (r.dot(su), r.dot(sv));
            let next_u = (u + (c * ru - b * rv) / det).clamp(u0, u1);
            let next_v = (v + (a * rv - b * ru) / det).clamp(v0, v1);
            let moved = (next_u - u).abs() / (u1 - u0) + (next_v - v).abs() / (v1 - v0);
            (u, v) = (next_u, next_v);
            if moved <= 1e-15 {
                break;
            }
        }
        // Never return something worse than the best sample.
        Ok(if self.eval(u, v).distance(p) <= best.0 {
            (u, v)
        } else {
            (best.1, best.2)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(scale: f64) -> Frame {
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap()
    }

    fn bumpy(scale: f64) -> BezierPatch {
        let mut net = [[Vec3::ZERO; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            for (j, p) in row.iter_mut().enumerate() {
                let (x, y) = (i as f64, j as f64 * 1.5);
                *p = Vec3::new(x, y, ((i * 7 + j * 3) % 5) as f64 * 0.3 - 0.2 * x) * scale;
            }
        }
        BezierPatch::bicubic(net).unwrap()
    }

    fn surfaces(scale: f64) -> Vec<Surface> {
        let f = frame(scale);
        vec![
            Surface::Plane(Plane::new(f)),
            Surface::Cylinder(Cylinder::new(f, 2.0 * scale).unwrap()),
            Surface::Cone(Cone::new(f, 1.5 * scale, 0.4).unwrap()),
            Surface::Cone(Cone::new(f, 1.5 * scale, -0.7).unwrap()),
            Surface::Sphere(Sphere::new(f, 3.0 * scale).unwrap()),
            Surface::Torus(Torus::new(f, 4.0 * scale, 1.0 * scale).unwrap()),
            Surface::Bezier(bumpy(scale)),
            Surface::Nurbs(NurbsSurface::from_bezier(&bumpy(scale)).unwrap()),
        ]
    }

    /// Interior sample parameters of a surface, away from poles and apexes.
    fn samples(s: &Surface, scale: f64) -> Vec<(f64, f64)> {
        let ((u0, u1), (v0, v1)) = s.domain();
        let span = |a: f64, b: f64| match (a.is_finite(), b.is_finite()) {
            (true, true) => (a, b),
            (true, false) => (a, a + 4.0 * scale),
            (false, true) => (b - 4.0 * scale, b),
            (false, false) => (-3.0 * scale, 3.0 * scale),
        };
        let ((u0, u1), (v0, v1)) = (span(u0, u1), span(v0, v1));
        let mut out = Vec::new();
        for i in 1..8 {
            for j in 1..8 {
                out.push((
                    u0 + (u1 - u0) * f64::from(i) / 8.0,
                    v0 + (v1 - v0) * f64::from(j) / 8.0,
                ));
            }
        }
        out
    }

    #[test]
    fn points_satisfy_the_implicit_equations() {
        for scale in [1e-3, 1.0, 1e6] {
            let f = frame(scale);
            for s in surfaces(scale) {
                for (u, v) in samples(&s, scale) {
                    let (x, y, z) = f.local(s.eval(u, v));
                    let rho = x.hypot(y);
                    let residual = match &s {
                        Surface::Plane(_) => z,
                        Surface::Cylinder(c) => rho - c.radius(),
                        Surface::Cone(c) => rho - (c.radius() + z * c.half_angle().tan()),
                        Surface::Sphere(sp) => (rho * rho + z * z).sqrt() - sp.radius(),
                        Surface::Torus(t) => {
                            ((rho - t.radii().0).powi(2) + z * z).sqrt() - t.radii().1
                        }
                        Surface::Bezier(_) | Surface::Nurbs(_) => 0.0,
                    };
                    assert!(
                        residual.abs() <= 1e-12 * scale,
                        "{} at ({u}, {v}), scale {scale}",
                        s.kind()
                    );
                }
            }
        }
    }

    #[test]
    fn derivatives_match_central_differences_and_normals_are_outward() {
        for scale in [1e-3, 1.0, 1e6] {
            let f = frame(scale);
            for s in surfaces(scale) {
                for (u, v) in samples(&s, scale) {
                    let (p, du, dv) = s.derivatives(u, v);
                    assert_eq!(p, s.eval(u, v));
                    let analytic = !matches!(s, Surface::Bezier(_) | Surface::Nurbs(_));
                    // Angles and Bézier parameters are O(1); lengths scale.
                    let hu = if matches!(s, Surface::Plane(_)) {
                        1e-6 * scale
                    } else {
                        1e-6
                    };
                    let hv = if matches!(
                        s,
                        Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_)
                    ) {
                        1e-6 * scale
                    } else {
                        1e-6
                    };
                    let fd_u = (s.eval(u + hu, v) - s.eval(u - hu, v)) * (0.5 / hu);
                    let fd_v = (s.eval(u, v + hv) - s.eval(u, v - hv)) * (0.5 / hv);
                    assert!(
                        du.distance(fd_u) <= 1e-6 * du.length().max(1e-300),
                        "{} du at ({u}, {v})",
                        s.kind()
                    );
                    assert!(
                        dv.distance(fd_v) <= 1e-6 * dv.length().max(1e-300),
                        "{} dv at ({u}, {v})",
                        s.kind()
                    );

                    let n = s.normal(u, v).unwrap();
                    assert!((n.length() - 1.0).abs() < 1e-14);
                    assert!(
                        n.dot(du).abs() <= 1e-12 * du.length()
                            && n.dot(dv).abs() <= 1e-12 * dv.length()
                    );
                    assert!(
                        n.dot(du.cross(dv)) > 0.0,
                        "{}: normal must be du × dv",
                        s.kind()
                    );
                    if analytic && !matches!(s, Surface::Plane(_)) {
                        // Away from the axis (for the torus: from the tube's centre circle).
                        let (x, y, z) = f.local(p);
                        let from = match &s {
                            Surface::Sphere(_) => f.origin(),
                            Surface::Torus(t) => {
                                let k = t.radii().0 / x.hypot(y);
                                f.point(x * k, y * k, 0.0)
                            }
                            _ => f.point(0.0, 0.0, z),
                        };
                        assert!(
                            n.dot(p - from) > 0.0,
                            "{}: normal must point outward",
                            s.kind()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn projecting_a_point_of_the_surface_returns_its_parameters() {
        for scale in [1e-3, 1.0, 1e6] {
            for s in surfaces(scale) {
                let ((u0, u1), (v0, v1)) = s.domain();
                for (u, v) in samples(&s, scale) {
                    let p = s.eval(u, v);
                    let (pu, pv) = s.project(p).unwrap();
                    assert!(
                        s.eval(pu, pv).distance(p) <= 1e-9 * scale,
                        "{} at ({u}, {v}), scale {scale}: got ({pu}, {pv})",
                        s.kind()
                    );
                    let size = |a: f64, b: f64| if (b - a).is_finite() { b - a } else { scale };
                    assert!(
                        (pu - u).abs() <= 1e-7 * size(u0, u1),
                        "{} u at ({u}, {v}): {pu}",
                        s.kind()
                    );
                    assert!(
                        (pv - v).abs() <= 1e-7 * size(v0, v1),
                        "{} v at ({u}, {v}): {pv}",
                        s.kind()
                    );
                }
            }
        }
    }

    #[test]
    fn the_foot_of_an_off_surface_point_lies_along_the_normal() {
        for s in surfaces(1.0) {
            for (u, v) in samples(&s, 1.0).into_iter().step_by(5) {
                let (foot, n) = (s.eval(u, v), s.normal(u, v).unwrap());
                // Lift the point off the surface by less than any radius of curvature.
                for lift in [0.3, -0.2] {
                    let p = foot + n * lift;
                    let (pu, pv) = s.project(p).unwrap();
                    let found = s.eval(pu, pv);
                    assert!(
                        found.distance(foot) <= 1e-7,
                        "{} at ({u}, {v}) lift {lift}: {found:?}",
                        s.kind()
                    );
                }
            }
        }
    }

    #[test]
    fn poles_apexes_and_axes_are_handled() {
        let f = Frame::WORLD;
        let sphere = Surface::Sphere(Sphere::new(f, 2.0).unwrap());
        // Closed-form normal at the pole, where ∂u vanishes.
        assert!(sphere.derivatives(1.0, FRAC_PI_2).1.length() < 1e-15);
        assert!(sphere.normal(1.0, FRAC_PI_2).unwrap().distance(Vec3::Z) < 1e-15);
        assert_eq!(sphere.project(Vec3::ZERO).unwrap(), (0.0, 0.0));
        assert_eq!(
            sphere.project(Vec3::new(0.0, 0.0, 5.0)).unwrap(),
            (0.0, FRAC_PI_2)
        );

        let cone = Cone::new(f, 1.0, 0.5_f64.atan()).unwrap();
        assert!((cone.apex_v() + 2.0).abs() < 1e-14);
        let cone = Surface::Cone(cone);
        assert!(cone.normal(0.3, -2.0).is_none(), "no normal at the apex");
        assert!(cone.eval(0.3, -2.0).distance(Vec3::new(0.0, 0.0, -2.0)) < 1e-14);
        // Points beyond the apex project onto the apex, not the other nappe.
        let (_, v) = cone.project(Vec3::new(0.1, 0.0, -9.0)).unwrap();
        assert!((v + 2.0).abs() < 1e-14);
        assert!((cone.domain().1.0 + 2.0).abs() < 1e-14);

        let cylinder = Surface::Cylinder(Cylinder::new(f, 1.0).unwrap());
        assert_eq!(
            cylinder.project(Vec3::new(0.0, 0.0, 3.0)).unwrap(),
            (0.0, 3.0)
        );
        let (u, _) = cylinder.project(Vec3::new(1.0, -1e-300, 0.0)).unwrap();
        assert!((0.0..TAU).contains(&u));
        for s in surfaces(1.0) {
            assert!(
                s.project(Vec3::new(0.0, f64::NAN, 0.0)).is_err(),
                "{}",
                s.kind()
            );
        }
    }

    #[test]
    fn invalid_dimensions_are_rejected() {
        let f = Frame::WORLD;
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(Cylinder::new(f, bad).is_err());
            assert!(Sphere::new(f, bad).is_err());
            assert!(Torus::new(f, 5.0, bad).is_err());
        }
        assert!(Torus::new(f, 1.0, 1.0).is_err(), "tube touches the axis");
        assert!(Torus::new(f, 1.0, 2.0).is_err());
        assert!(Cone::new(f, -1.0, 0.3).is_err());
        assert!(Cone::new(f, 1.0, 0.0).is_err(), "that is a cylinder");
        assert!(Cone::new(f, 1.0, FRAC_PI_2).is_err());
        assert!(Cone::new(f, 1.0, f64::NAN).is_err());
        assert!(Cone::new(f, 0.0, 0.3).is_ok(), "apex at the origin");
    }
}
