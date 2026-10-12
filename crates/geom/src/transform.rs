//! Moving and mirroring curves and surfaces, exactly.
//!
//! Every `transformed` method takes an *isometry*: a rigid motion
//! (determinant +1) or one that also mirrors (determinant −1), as
//! [`Transform::is_isometry`] recognises it within [`ISOMETRY_EPSILON`].
//! Anything else (scaling, shear, a non-finite entry) is an error: the
//! analytic kinds have no exact image under it.
//!
//! What the result promises about its parametrisation:
//!
//! **Curves, every kind, rigid or mirrored:** the same parametrisation.
//! `C'(t) = T(C(t))` for every `t`, with the same domain and period.
//!
//! **Surfaces under a rigid motion, every kind:** the same parametrisation.
//! `S'(u, v) = T(S(u, v))`, and the normal is the image of the normal.
//!
//! **Surfaces under a mirror** cannot have both: a mirror turns the
//! right-handed pair `(∂u, ∂v, normal)` left-handed. Which of the two gives
//! way depends on the kind ([`Surface::mirror_rule`]):
//!
//! | kind | image | parameters | normal `∂u × ∂v` |
//! |---|---|---|---|
//! | plane | frame `(Tx, Ty, −Tz)` | `S'(u, v) = T(S(u, v))` | **opposite** of the image normal |
//! | Bézier, NURBS | control points mapped | `S'(u, v) = T(S(u, v))` | **opposite** of the image normal |
//! | cylinder | frame `(Tx, Ty, −Tz)`, same radius | `S'(u, −v) = T(S(u, v))` | the image normal |
//! | cone | frame `(Tx, Ty, −Tz)`, same radius, half-angle **negated** | `S'(u, −v) = T(S(u, v))` | the image normal |
//! | sphere | frame `(Tx, Ty, −Tz)`, same radius | `S'(u, −v) = T(S(u, v))` | the image normal |
//! | torus | frame `(Tx, Ty, −Tz)`, same radii | `S'(u, −v) = T(S(u, v))` | the image normal |
//!
//! The reason is one rule for frames: frames are right-handed by
//! construction, so the image of a frame under a mirror keeps the images of
//! x and y and takes the opposite of the image of z
//! ([`Transform::frame`]). Local coordinates `(a, b, c)` become
//! `(a, b, −c)`. A circle or an ellipse lies in `c = 0` and does not notice;
//! a plane's normal *is* z and flips; the surfaces of revolution have
//! their height or latitude along z, so `v` changes sign, and their normal
//! points away from the axis or centre whichever way the frame looks. A
//! cone that widened along +z now widens along −z of its new frame, which
//! is what the negated half-angle says.
//!
//! Circles (and ellipses) that shared the axes of a surface's frame bit for
//! bit still do afterwards, so their angle 0 still agrees with the
//! surface's `u = 0`.

use crate::{
    BezierPatch, Circle, Cone, Curve3, Cylinder, Ellipse, GeomError, Line, NurbsCurve,
    NurbsSurface, Plane, Sphere, Surface, Torus,
};
use craft3d_math::{Frame, Transform};

/// How far `M Mᵀ` of a transform's linear part may be from the identity,
/// per entry, for the transform to count as an isometry here. Rotations and
/// mirrors built by `craft3d-math`, and long chains of them, are within a
/// few 1e-16; a matrix typed in with nine digits is not, and must be made
/// orthonormal by whoever reads it.
pub const ISOMETRY_EPSILON: f64 = 1e-12;

/// What [`Surface::transformed`] does to a surface's parametrisation when
/// the transform mirrors (see the module documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorRule {
    /// `S'(u, v) = T(S(u, v))`; the normal of `S'` is the opposite of the
    /// image of the normal. Planes, Bézier patches, NURBS surfaces.
    FlipsNormal,
    /// `S'(u, −v) = T(S(u, v))`; the normal of `S'` is the image of the
    /// normal. Cylinders, cones, spheres, tori.
    NegatesV,
}

const NOT_ISOMETRY: GeomError = GeomError::InvalidParameter(
    "the transform is not an isometry (a rigid motion or a mirror); scaling and shear are not supported",
);

/// `Ok(true)` when `t` mirrors, `Ok(false)` when it is a rigid motion.
fn mirrors(t: &Transform) -> Result<bool, GeomError> {
    if !t.is_isometry(ISOMETRY_EPSILON) {
        return Err(NOT_ISOMETRY);
    }
    Ok(t.determinant() < 0.0)
}

fn frame(t: &Transform, frame: &Frame) -> Result<Frame, GeomError> {
    mirrors(t)?;
    t.frame(frame).ok_or(GeomError::InvalidParameter(
        "the transform moves a frame to somewhere not finite",
    ))
}

impl Line {
    /// The image of the line under an isometry, with the same
    /// parametrisation: `L'(t) = T(L(t))`.
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        mirrors(t)?;
        Line::new(t.point(self.origin()), t.vector(self.direction()))
    }
}

impl Circle {
    /// The image of the circle under an isometry, with the same
    /// parametrisation: `C'(t) = T(C(t))`. Under a mirror the frame's z
    /// axis is the opposite of the image of z.
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        Circle::new(frame(t, self.frame())?, self.radius())
    }
}

impl Ellipse {
    /// The image of the ellipse under an isometry, with the same
    /// parametrisation: `E'(t) = T(E(t))`. Under a mirror the frame's z
    /// axis is the opposite of the image of z.
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        let (a, b) = self.semi_axes();
        Ellipse::new(frame(t, self.frame())?, a, b)
    }
}

impl NurbsCurve {
    /// The image of the curve under an isometry: every control point
    /// mapped, knots and weights kept, so `C'(t) = T(C(t))`.
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        mirrors(t)?;
        NurbsCurve::new(
            self.degree(),
            self.knots().to_vec(),
            self.control_points().iter().map(|&p| t.point(p)).collect(),
            self.weights().to_vec(),
        )
    }
}

impl Curve3 {
    /// The image of the curve under an isometry (a rigid motion or a
    /// mirror), with the same parametrisation: `C'(t) = T(C(t))` for every
    /// `t`, the same domain, the same period. An error for a transform that
    /// is not an isometry or that moves the curve to somewhere not finite.
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        Ok(match self {
            Curve3::Line(c) => Curve3::Line(c.transformed(t)?),
            Curve3::Circle(c) => Curve3::Circle(c.transformed(t)?),
            Curve3::Ellipse(c) => Curve3::Ellipse(c.transformed(t)?),
            Curve3::Nurbs(c) => Curve3::Nurbs(c.transformed(t)?),
        })
    }
}

impl Plane {
    /// The image of the plane under an isometry, with the same
    /// parametrisation: `P'(u, v) = T(P(u, v))`. Under a mirror the normal
    /// is the opposite of the image of the normal
    /// ([`MirrorRule::FlipsNormal`]).
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        Ok(Plane::new(frame(t, self.frame())?))
    }
}

impl Cylinder {
    /// The image of the cylinder under an isometry. Rigid:
    /// `S'(u, v) = T(S(u, v))`. Mirrored: `S'(u, −v) = T(S(u, v))`
    /// ([`MirrorRule::NegatesV`]). The normal is the image of the normal
    /// either way.
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        Cylinder::new(frame(t, self.frame())?, self.radius())
    }
}

impl Cone {
    /// The image of the cone under an isometry. Rigid:
    /// `S'(u, v) = T(S(u, v))`. Mirrored: `S'(u, −v) = T(S(u, v))`
    /// ([`MirrorRule::NegatesV`]), with the half-angle negated because the
    /// new frame's z axis looks the other way; the apex is at `−apex_v`.
    /// The normal is the image of the normal either way.
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        let half_angle = if mirrors(t)? {
            -self.half_angle()
        } else {
            self.half_angle()
        };
        Cone::new(frame(t, self.frame())?, self.radius(), half_angle)
    }
}

impl Sphere {
    /// The image of the sphere under an isometry. Rigid:
    /// `S'(u, v) = T(S(u, v))`. Mirrored: `S'(u, −v) = T(S(u, v))`
    /// ([`MirrorRule::NegatesV`]). The normal is the image of the normal
    /// either way.
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        Sphere::new(frame(t, self.frame())?, self.radius())
    }
}

impl Torus {
    /// The image of the torus under an isometry. Rigid:
    /// `S'(u, v) = T(S(u, v))`. Mirrored: `S'(u, −v) = T(S(u, v))`
    /// ([`MirrorRule::NegatesV`]). The normal is the image of the normal
    /// either way.
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        let (major, minor) = self.radii();
        Torus::new(frame(t, self.frame())?, major, minor)
    }
}

impl BezierPatch {
    /// The image of the patch under an isometry: every control point
    /// mapped, so `S'(u, v) = T(S(u, v))`. Under a mirror the normal is the
    /// opposite of the image of the normal ([`MirrorRule::FlipsNormal`]).
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        mirrors(t)?;
        self.map_points(|p| t.point(p))
    }
}

impl NurbsSurface {
    /// The image of the surface under an isometry: every control point
    /// mapped, knots and weights kept, so `S'(u, v) = T(S(u, v))`. Under a
    /// mirror the normal is the opposite of the image of the normal
    /// ([`MirrorRule::FlipsNormal`]).
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        mirrors(t)?;
        let (degree_u, degree_v) = self.degrees();
        let (count_u, count_v) = self.counts();
        NurbsSurface::new(
            degree_u,
            degree_v,
            self.knots_u().to_vec(),
            self.knots_v().to_vec(),
            count_u,
            count_v,
            self.control_points().iter().map(|&p| t.point(p)).collect(),
            self.weights().to_vec(),
        )
    }
}

impl Surface {
    /// The image of the surface under an isometry (a rigid motion or a
    /// mirror).
    ///
    /// Under a rigid motion the parametrisation is kept:
    /// `S'(u, v) = T(S(u, v))`. Under a mirror, [`Surface::mirror_rule`]
    /// says what happens: either the parametrisation is kept and the normal
    /// is the opposite of the image of the normal, or `v` changes sign and
    /// the normal is the image of the normal. An error for a transform that
    /// is not an isometry or that moves the surface to somewhere not
    /// finite.
    pub fn transformed(&self, t: &Transform) -> Result<Self, GeomError> {
        Ok(match self {
            Surface::Plane(s) => Surface::Plane(s.transformed(t)?),
            Surface::Cylinder(s) => Surface::Cylinder(s.transformed(t)?),
            Surface::Cone(s) => Surface::Cone(s.transformed(t)?),
            Surface::Sphere(s) => Surface::Sphere(s.transformed(t)?),
            Surface::Torus(s) => Surface::Torus(s.transformed(t)?),
            Surface::Bezier(s) => Surface::Bezier(s.transformed(t)?),
            Surface::Nurbs(s) => Surface::Nurbs(s.transformed(t)?),
        })
    }

    /// What a mirroring [`Surface::transformed`] does to this kind's
    /// parametrisation and normal.
    pub fn mirror_rule(&self) -> MirrorRule {
        match self {
            Surface::Plane(_) | Surface::Bezier(_) | Surface::Nurbs(_) => MirrorRule::FlipsNormal,
            Surface::Cylinder(_) | Surface::Cone(_) | Surface::Sphere(_) | Surface::Torus(_) => {
                MirrorRule::NegatesV
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Vec3;
    use std::f64::consts::TAU;

    fn tilted(scale: f64) -> Frame {
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap()
    }

    fn wiggle(scale: f64) -> NurbsCurve {
        let ctrl = [
            (0.0, 0.0, 0.0),
            (1.0, 2.0, 0.5),
            (2.5, -1.0, 1.0),
            (4.0, 1.5, -0.5),
            (5.0, 0.0, 2.0),
            (6.5, 1.0, 0.0),
        ]
        .iter()
        .map(|&(x, y, z)| Vec3::new(x, y, z) * scale)
        .collect();
        NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.7, 1.0, 1.0, 1.0, 1.0],
            ctrl,
            vec![1.0, 0.7, 1.3, 1.0, 2.0, 1.0],
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

    /// A rational surface with inner knots: a quarter of a bent tube.
    fn rational(scale: f64) -> NurbsSurface {
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for i in 0..4 {
            for j in 0..3 {
                let (x, y) = (f64::from(i) * 1.2, f64::from(j));
                ctrl.push(Vec3::new(x, y, 0.4 * x * y - 0.1 * x * x) * scale);
                weights.push(if (i + j) % 2 == 0 { 1.0 } else { 0.6 });
            }
        }
        NurbsSurface::new(
            2,
            2,
            vec![0.0, 0.0, 0.0, 0.4, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            4,
            3,
            ctrl,
            weights,
        )
        .unwrap()
    }

    fn curves(scale: f64) -> Vec<Curve3> {
        let f = tilted(scale);
        vec![
            Curve3::Line(
                Line::new(Vec3::new(1.0, 2.0, 3.0) * scale, Vec3::new(2.0, -1.0, 0.5)).unwrap(),
            ),
            Curve3::Circle(Circle::new(f, 2.5 * scale).unwrap()),
            Curve3::Ellipse(Ellipse::new(f, 3.0 * scale, 1.2 * scale).unwrap()),
            Curve3::Nurbs(NurbsCurve::circle(&f, 2.0 * scale).unwrap()),
            Curve3::Nurbs(wiggle(scale)),
        ]
    }

    fn surfaces(scale: f64) -> Vec<Surface> {
        let f = tilted(scale);
        vec![
            Surface::Plane(Plane::new(f)),
            Surface::Cylinder(Cylinder::new(f, 2.0 * scale).unwrap()),
            Surface::Cone(Cone::new(f, 1.5 * scale, 0.4).unwrap()),
            Surface::Cone(Cone::new(f, 1.5 * scale, -0.7).unwrap()),
            Surface::Cone(Cone::new(f, 0.0, 0.3).unwrap()),
            Surface::Sphere(Sphere::new(f, 3.0 * scale).unwrap()),
            Surface::Torus(Torus::new(f, 4.0 * scale, 1.0 * scale).unwrap()),
            Surface::Bezier(bumpy(scale)),
            Surface::Nurbs(NurbsSurface::from_bezier(&bumpy(scale)).unwrap()),
            Surface::Nurbs(rational(scale)),
        ]
    }

    /// `(transform, mirrors)`: rigid motions and mirrors, plain and
    /// composed, at the size of the geometry.
    fn isometries(scale: f64) -> Vec<(Transform, bool)> {
        let spin = Transform::rotation_about(
            Vec3::new(3.0, 1.0, -2.0) * scale,
            Vec3::new(1.0, 1.0, 1.0),
            2.1,
        )
        .unwrap();
        let slide = Transform::translation(Vec3::new(-7.0, 0.25, 40.0) * scale);
        let flip =
            Transform::mirror(Vec3::new(0.0, 5.0, 0.0) * scale, Vec3::new(1.0, -3.0, 2.0)).unwrap();
        vec![
            (Transform::IDENTITY, false),
            (slide, false),
            (spin, false),
            (spin.then(&slide), false),
            (Transform::mirror(Vec3::ZERO, Vec3::Z).unwrap(), true),
            (flip, true),
            (spin.then(&flip).then(&slide), true),
            // Two mirrors make a rotation; three a mirror again.
            (
                flip.then(&Transform::mirror(Vec3::ZERO, Vec3::X).unwrap()),
                false,
            ),
            (Transform::scaling(-1.0), true),
        ]
    }

    /// Sample parameters of a curve, some outside one period.
    fn curve_samples(c: &Curve3, scale: f64) -> Vec<f64> {
        let (a, b) = match c.domain() {
            (a, b) if a.is_finite() && b.is_finite() => {
                if c.period().is_some() {
                    (a - 2.0, b + 3.0)
                } else {
                    (a, b)
                }
            }
            _ => (-5.0 * scale, 7.0 * scale),
        };
        (0..=24)
            .map(|i| a + (b - a) * f64::from(i) / 24.0)
            .collect()
    }

    /// Sample parameters of a surface, away from poles and apexes.
    fn surface_samples(s: &Surface, scale: f64) -> Vec<(f64, f64)> {
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
    fn curves_keep_their_parametrisation() {
        for scale in [1e-3, 1.0, 1e6] {
            for (t, _) in isometries(scale) {
                for c in curves(scale) {
                    let image = c.transformed(&t).unwrap();
                    assert_eq!(image.kind(), c.kind());
                    assert_eq!(image.domain(), c.domain());
                    assert_eq!(image.period(), c.period());
                    for s in curve_samples(&c, scale) {
                        let (p, d) = c.derivatives(s);
                        let (q, e) = image.derivatives(s);
                        // The mapped points are up to ~50 scale from the
                        // origin, so a few 1e-16 of that.
                        assert!(
                            q.distance(t.point(p)) <= 1e-13 * scale,
                            "{} at {s}, scale {scale}: {:e}",
                            c.kind(),
                            q.distance(t.point(p))
                        );
                        assert!(
                            e.distance(t.vector(d)) <= 1e-12 * d.length(),
                            "{} tangent at {s}, scale {scale}",
                            c.kind()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn surfaces_follow_their_rule() {
        for scale in [1e-3, 1.0, 1e6] {
            for (t, mirror) in isometries(scale) {
                for s in surfaces(scale) {
                    let image = s.transformed(&t).unwrap();
                    assert_eq!(image.kind(), s.kind());
                    assert_eq!(
                        (image.period_u(), image.period_v()),
                        (s.period_u(), s.period_v())
                    );
                    let (v_sign, n_sign) = match (mirror, s.mirror_rule()) {
                        (false, _) => (1.0, 1.0),
                        (true, MirrorRule::FlipsNormal) => (1.0, -1.0),
                        (true, MirrorRule::NegatesV) => (-1.0, 1.0),
                    };
                    // The domain follows: the same in u, negated in v when
                    // the rule says so.
                    let ((u0, u1), (v0, v1)) = s.domain();
                    let ((iu0, iu1), (iv0, iv1)) = image.domain();
                    assert_eq!((iu0, iu1), (u0, u1));
                    let close = |a: f64, b: f64| a == b || (a - b).abs() <= 1e-12 * scale;
                    // (A torus is periodic in v: one period, as before.)
                    if v_sign > 0.0 || s.period_v().is_some() {
                        assert!(close(iv0, v0) && close(iv1, v1), "{}", s.kind());
                    } else {
                        assert!(close(iv0, -v1) && close(iv1, -v0), "{}", s.kind());
                    }
                    for (u, v) in surface_samples(&s, scale) {
                        let what = format!("{} at ({u}, {v}), scale {scale}", s.kind());
                        let (p, du, dv) = s.derivatives(u, v);
                        let (q, eu, ev) = image.derivatives(u, v_sign * v);
                        assert!(
                            q.distance(t.point(p)) <= 1e-13 * scale,
                            "{what}: {:e}",
                            q.distance(t.point(p))
                        );
                        assert!(eu.distance(t.vector(du)) <= 1e-12 * du.length(), "{what}");
                        assert!(
                            ev.distance(t.vector(dv) * v_sign) <= 1e-12 * dv.length(),
                            "{what}"
                        );
                        let n = s.normal(u, v).unwrap();
                        let m = image.normal(u, v_sign * v).unwrap();
                        assert!(m.distance(t.vector(n) * n_sign) <= 1e-9, "{what}");
                        // The image point is on the image surface, found
                        // again by projection (independent of the rule).
                        let (pu, pv) = image.project(t.point(p)).unwrap();
                        assert!(
                            image.eval(pu, pv).distance(t.point(p)) <= 1e-9 * scale,
                            "{what}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_mirrored_cone_still_opens_the_right_way() {
        let f = tilted(1.0);
        for half_angle in [0.4, -0.7] {
            let cone = Cone::new(f, 1.5, half_angle).unwrap();
            let t = Transform::mirror(Vec3::new(0.0, 5.0, 0.0), Vec3::new(1.0, -3.0, 2.0)).unwrap();
            let image = cone.transformed(&t).unwrap();
            assert_eq!(image.half_angle(), -half_angle);
            assert_eq!(image.radius(), 1.5);
            assert!((image.apex_v() + cone.apex_v()).abs() < 1e-15);
            // The apex is the image of the apex.
            let apex = f.point(0.0, 0.0, cone.apex_v());
            let image_apex = image.frame().point(0.0, 0.0, image.apex_v());
            assert!(image_apex.distance(t.point(apex)) < 1e-14);
            // Radii along the axis: the image of the circle at height v has
            // the radius the image cone has at -v.
            for v in [0.5, 1.0, 2.0] {
                let v = v * half_angle.signum();
                let p = t.point(Surface::Cone(cone).eval(1.0, v));
                let (x, y, z) = image.frame().local(p);
                assert!((z + v).abs() < 1e-13);
                assert!((x.hypot(y) - (1.5 + v * half_angle.tan())).abs() < 1e-13);
            }
        }
    }

    #[test]
    fn frames_that_shared_axes_still_do() {
        // A circle at the top of a cylinder starts at the same angle as the
        // cylinder's u = 0 only when the axes agree bit for bit.
        let f = tilted(1.0);
        let top = f.at(f.point(0.0, 0.0, 3.0)).unwrap();
        for (t, _) in isometries(1.0) {
            let wall = Cylinder::new(f, 2.0).unwrap().transformed(&t).unwrap();
            let rim = Circle::new(top, 2.0).unwrap().transformed(&t).unwrap();
            assert_eq!(
                (wall.frame().x(), wall.frame().y(), wall.frame().z()),
                (rim.frame().x(), rim.frame().y(), rim.frame().z())
            );
        }
    }

    #[test]
    fn group_laws_hold() {
        let scale = 1.0;
        let all = isometries(scale);
        for (t, _) in &all {
            let back = t.inverse().unwrap();
            for c in curves(scale) {
                let round = c.transformed(t).unwrap().transformed(&back).unwrap();
                for s in curve_samples(&c, scale) {
                    assert!(round.eval(s).distance(c.eval(s)) <= 1e-12, "{}", c.kind());
                }
            }
            for s in surfaces(scale) {
                // There and back mirrors twice or not at all: the
                // parametrisation is the original one.
                let round = s.transformed(t).unwrap().transformed(&back).unwrap();
                for (u, v) in surface_samples(&s, scale) {
                    assert!(
                        round.eval(u, v).distance(s.eval(u, v)) <= 1e-12,
                        "{}",
                        s.kind()
                    );
                    let (n, m) = (s.normal(u, v).unwrap(), round.normal(u, v).unwrap());
                    assert!(n.distance(m) <= 1e-9, "{}", s.kind());
                }
            }
        }
        // One after the other is the composition, as point sets and (for
        // curves) as parametrisations.
        let (a, b) = (all[3].0, all[5].0);
        for c in curves(scale) {
            let stepwise = c.transformed(&a).unwrap().transformed(&b).unwrap();
            let at_once = c.transformed(&a.then(&b)).unwrap();
            for s in curve_samples(&c, scale) {
                assert!(stepwise.eval(s).distance(at_once.eval(s)) <= 1e-12);
            }
        }
        for s in surfaces(scale) {
            let stepwise = s.transformed(&a).unwrap().transformed(&b).unwrap();
            let at_once = s.transformed(&a.then(&b)).unwrap();
            for (u, v) in surface_samples(&s, scale) {
                assert!(stepwise.eval(u, v).distance(at_once.eval(u, v)) <= 1e-12);
            }
        }
    }

    #[test]
    fn results_are_bit_identical_on_repeat() {
        for (t, _) in isometries(1.0) {
            for c in curves(1.0) {
                assert_eq!(c.transformed(&t).unwrap(), c.transformed(&t).unwrap());
            }
            for s in surfaces(1.0) {
                assert_eq!(s.transformed(&t).unwrap(), s.transformed(&t).unwrap());
            }
        }
    }

    #[test]
    fn anything_but_an_isometry_is_an_error() {
        let shear = Transform::rotation(Vec3::Z, 0.3)
            .unwrap()
            .then(&Transform::scaling(1.0 + 1e-6));
        let bad = [
            Transform::scaling(2.0),
            Transform::scaling(0.0),
            Transform::scaling(f64::NAN),
            Transform::scaling(f64::INFINITY),
            Transform::translation(Vec3::new(f64::NAN, 0.0, 0.0)),
            Transform::translation(Vec3::new(0.0, f64::INFINITY, 0.0)),
            shear,
        ];
        for t in bad {
            for c in curves(1.0) {
                assert!(
                    matches!(c.transformed(&t), Err(GeomError::InvalidParameter(_))),
                    "{}",
                    c.kind()
                );
            }
            for s in surfaces(1.0) {
                assert!(
                    matches!(s.transformed(&t), Err(GeomError::InvalidParameter(_))),
                    "{}",
                    s.kind()
                );
            }
        }
    }

    #[test]
    fn overflowing_the_coordinates_is_an_error() {
        // An isometry, and finite, but the geometry ends up at infinity.
        let far = Transform::translation(Vec3::new(f64::MAX, 0.0, 0.0));
        let f = Frame::WORLD.at(Vec3::new(f64::MAX, 0.0, 0.0)).unwrap();
        let ctrl = vec![Vec3::new(f64::MAX, 0.0, 0.0), Vec3::new(f64::MAX, 1.0, 0.0)];
        let edge_of_the_world: Vec<Curve3> = vec![
            Curve3::Line(Line::new(f.origin(), Vec3::X).unwrap()),
            Curve3::Circle(Circle::new(f, 1.0).unwrap()),
            Curve3::Ellipse(Ellipse::new(f, 2.0, 1.0).unwrap()),
            Curve3::Nurbs(NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], ctrl).unwrap()),
        ];
        for c in edge_of_the_world {
            assert!(c.transformed(&far).is_err(), "{}", c.kind());
        }
        let patch = BezierPatch::new(
            1,
            1,
            vec![
                Vec3::new(f64::MAX, 0.0, 0.0),
                Vec3::new(f64::MAX, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
        )
        .unwrap();
        let surfaces = vec![
            Surface::Plane(Plane::new(f)),
            Surface::Cylinder(Cylinder::new(f, 1.0).unwrap()),
            Surface::Cone(Cone::new(f, 1.0, 0.5).unwrap()),
            Surface::Sphere(Sphere::new(f, 1.0).unwrap()),
            Surface::Torus(Torus::new(f, 2.0, 1.0).unwrap()),
            Surface::Nurbs(NurbsSurface::from_bezier(&patch).unwrap()),
            Surface::Bezier(patch),
        ];
        for s in surfaces {
            assert!(s.transformed(&far).is_err(), "{}", s.kind());
        }
    }

    #[test]
    fn full_turns_of_period_stay_whole() {
        // Not a property of the transform, but what the topology relies on:
        // a closed edge over (0, TAU) is still closed.
        let c = Curve3::Circle(Circle::new(tilted(1.0), 2.0).unwrap());
        for (t, _) in isometries(1.0) {
            let image = c.transformed(&t).unwrap();
            assert!(image.eval(0.0).distance(image.eval(TAU)) < 1e-14);
            assert!(image.eval(0.0).distance(t.point(c.eval(0.0))) < 1e-13);
        }
    }
}
