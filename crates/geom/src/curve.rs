//! Curves in space: analytic kinds stay analytic, everything else is NURBS.

use crate::{GeomError, NurbsCurve};
use craft3d_math::{Bits, Frame, Vec3};
use std::f64::consts::TAU;

/// An angle wrapped to `[0, 2π)`.
pub(crate) fn wrap_angle(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    // rem_euclid can return TAU itself for tiny negative input.
    if w >= TAU { 0.0 } else { w }
}

/// An infinite straight line; the parameter is the distance from `origin`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    origin: Vec3,
    direction: Vec3,
}

impl Line {
    /// `direction` is normalised; a zero or non-finite one is an error.
    pub fn new(origin: Vec3, direction: Vec3) -> Result<Self, GeomError> {
        let direction = direction.normalized().ok_or(GeomError::InvalidParameter(
            "a line needs a non-zero, finite direction",
        ))?;
        if !origin.is_finite() {
            return Err(GeomError::InvalidParameter("a line needs a finite origin"));
        }
        Ok(Self { origin, direction })
    }

    /// The line through `a` towards `b`, with parameter 0 at `a`.
    pub fn through(a: Vec3, b: Vec3) -> Result<Self, GeomError> {
        Self::new(a, b - a)
    }

    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    pub fn direction(&self) -> Vec3 {
        self.direction
    }
}

/// A circle in the xy plane of its frame; the parameter is the angle from
/// the frame's x axis, counter-clockwise about its z axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    frame: Frame,
    radius: f64,
}

impl Circle {
    pub fn new(frame: Frame, radius: f64) -> Result<Self, GeomError> {
        if !(radius.is_finite() && radius > 0.0) {
            return Err(GeomError::InvalidParameter(
                "radius must be positive and finite",
            ));
        }
        Ok(Self { frame, radius })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }
}

/// An ellipse in the xy plane of its frame with semi-axes `a` along x and
/// `b` along y; the parameter is the eccentric angle: `(a cos t, b sin t)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    frame: Frame,
    a: f64,
    b: f64,
}

impl Ellipse {
    pub fn new(frame: Frame, a: f64, b: f64) -> Result<Self, GeomError> {
        if !(a.is_finite() && b.is_finite() && a > 0.0 && b > 0.0) {
            return Err(GeomError::InvalidParameter(
                "semi-axes must be positive and finite",
            ));
        }
        Ok(Self { frame, a, b })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn semi_axes(&self) -> (f64, f64) {
        (self.a, self.b)
    }
}

/// Any curve the kernel knows. No code may assume a particular kind
/// (AGENTS.md rule 3).
#[derive(Clone, Debug, PartialEq)]
pub enum Curve3 {
    Line(Line),
    Circle(Circle),
    Ellipse(Ellipse),
    Nurbs(NurbsCurve),
}

impl Curve3 {
    /// A short name for messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Curve3::Line(_) => "line",
            Curve3::Circle(_) => "circle",
            Curve3::Ellipse(_) => "ellipse",
            Curve3::Nurbs(_) => "nurbs curve",
        }
    }

    /// `(start, end)` of the parameter domain; infinite for a line, one
    /// period for closed analytic curves.
    pub fn domain(&self) -> (f64, f64) {
        match self {
            Curve3::Line(_) => (f64::NEG_INFINITY, f64::INFINITY),
            Curve3::Circle(_) | Curve3::Ellipse(_) => (0.0, TAU),
            Curve3::Nurbs(c) => c.domain(),
        }
    }

    /// The period of the parameter, for curves that close on themselves by
    /// construction.
    pub fn period(&self) -> Option<f64> {
        match self {
            Curve3::Circle(_) | Curve3::Ellipse(_) => Some(TAU),
            Curve3::Line(_) | Curve3::Nurbs(_) => None,
        }
    }

    pub fn eval(&self, t: f64) -> Vec3 {
        self.derivatives(t).0
    }

    /// `(point, first derivative)` at `t`.
    pub fn derivatives(&self, t: f64) -> (Vec3, Vec3) {
        match self {
            Curve3::Line(l) => (l.origin + l.direction * t, l.direction),
            Curve3::Circle(c) => {
                let (s, co) = t.sin_cos();
                let r = c.radius;
                (
                    c.frame.point(r * co, r * s, 0.0),
                    c.frame.vector(-r * s, r * co, 0.0),
                )
            }
            Curve3::Ellipse(e) => {
                let (s, co) = t.sin_cos();
                (
                    e.frame.point(e.a * co, e.b * s, 0.0),
                    e.frame.vector(-e.a * s, e.b * co, 0.0),
                )
            }
            Curve3::Nurbs(c) => c.derivatives(t),
        }
    }

    /// The parameter of the point on the curve closest to `p`.
    ///
    /// Closed form for lines and circles (on a circle's axis every point is
    /// equally close and 0 is returned); Newton iteration from the best of a
    /// set of samples for ellipses and NURBS, which finds the closest point
    /// unless the curve is more wiggly than the sampling resolves.
    pub fn project(&self, p: Vec3) -> Result<f64, GeomError> {
        if !p.is_finite() {
            return Err(GeomError::InvalidParameter(
                "cannot project a non-finite point",
            ));
        }
        match self {
            Curve3::Line(l) => Ok((p - l.origin).dot(l.direction)),
            Curve3::Circle(c) => {
                let (x, y, _) = c.frame.local(p);
                Ok(if x == 0.0 && y == 0.0 {
                    0.0
                } else {
                    wrap_angle(y.atan2(x))
                })
            }
            Curve3::Ellipse(_) => self.project_numeric(p, 64).map(wrap_angle),
            Curve3::Nurbs(c) => {
                // Enough samples to see every span several times.
                let samples = (c.control_points().len() * 8).clamp(32, 4096);
                self.project_numeric(p, samples)
            }
        }
    }

    fn project_numeric(&self, p: Vec3, samples: usize) -> Result<f64, GeomError> {
        let (start, end) = self.domain();
        if !(start.is_finite() && end.is_finite()) {
            return Err(GeomError::Unsupported(
                "numeric projection onto an unbounded curve",
            ));
        }
        let periodic = self.period().is_some();
        let mut best = (f64::INFINITY, start);
        for i in 0..=samples {
            let t = start + (end - start) * i as f64 / samples as f64;
            let d = self.eval(t).distance(p);
            if d < best.0 {
                best = (d, t);
            }
        }
        // Minimise |C(t) - p|²: Newton on g(t) = (C - p)·C', with g' taken
        // by a central difference of C' (no second derivatives needed).
        let h = (end - start) * 1e-6;
        let mut t = best.1;
        for _ in 0..50 {
            let (c, d1) = self.derivatives(t);
            let g = (c - p).dot(d1);
            let d2 = (self.derivatives(t + h).1 - self.derivatives(t - h).1) * (0.5 / h);
            let slope = d1.length_squared() + (c - p).dot(d2);
            if !(slope.is_finite() && slope > 0.0) {
                break; // Not a minimum nearby; keep the sample.
            }
            let mut next = t - g / slope;
            if !periodic {
                next = next.clamp(start, end);
            }
            let moved = (next - t).abs();
            t = next;
            if moved <= 1e-15 * (end - start) {
                break;
            }
        }
        // Never return something worse than the best sample.
        Ok(if self.eval(t).distance(p) <= best.0 {
            t
        } else {
            best.1
        })
    }
}

impl Bits for Line {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Line { origin, direction } = self;
        origin.push_bits(out);
        direction.push_bits(out);
    }
}

impl Bits for Circle {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Circle { frame, radius } = self;
        frame.push_bits(out);
        radius.push_bits(out);
    }
}

impl Bits for Ellipse {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Ellipse { frame, a, b } = self;
        frame.push_bits(out);
        a.push_bits(out);
        b.push_bits(out);
    }
}

impl Bits for Curve3 {
    fn push_bits(&self, out: &mut Vec<u64>) {
        match self {
            Curve3::Line(c) => {
                out.push(0);
                c.push_bits(out);
            }
            Curve3::Circle(c) => {
                out.push(1);
                c.push_bits(out);
            }
            Curve3::Ellipse(c) => {
                out.push(2);
                c.push_bits(out);
            }
            Curve3::Nurbs(c) => {
                out.push(3);
                c.push_bits(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tilted() -> Frame {
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap()
    }

    fn curves(scale: f64) -> Vec<Curve3> {
        let frame = Frame::new(tilted().origin() * scale, tilted().z(), tilted().x()).unwrap();
        vec![
            Curve3::Line(
                Line::new(Vec3::new(1.0, 2.0, 3.0) * scale, Vec3::new(2.0, -1.0, 0.5)).unwrap(),
            ),
            Curve3::Circle(Circle::new(frame, 2.5 * scale).unwrap()),
            Curve3::Ellipse(Ellipse::new(frame, 3.0 * scale, 1.2 * scale).unwrap()),
            Curve3::Nurbs(NurbsCurve::circle(&frame, 2.0 * scale).unwrap()),
        ]
    }

    #[test]
    fn points_satisfy_the_defining_equations() {
        for scale in [1e-3, 1.0, 1e6] {
            let frame = Frame::new(tilted().origin() * scale, tilted().z(), tilted().x()).unwrap();
            let all = curves(scale);
            for i in 0..=64 {
                let t = TAU * f64::from(i) / 64.0;
                // Line: unit speed, so points are t apart from the origin.
                let Curve3::Line(l) = &all[0] else { panic!() };
                assert!(
                    (all[0].eval(t * scale).distance(l.origin()) - t * scale).abs()
                        <= 1e-12 * scale
                );
                // Circle: x² + y² = r², z = 0 in its frame.
                let (x, y, z) = frame.local(all[1].eval(t));
                assert!(
                    (x.hypot(y) - 2.5 * scale).abs() <= 1e-13 * scale && z.abs() <= 1e-13 * scale
                );
                // Ellipse: (x/a)² + (y/b)² = 1.
                let (x, y, z) = frame.local(all[2].eval(t));
                let e = (x / (3.0 * scale)).powi(2) + (y / (1.2 * scale)).powi(2);
                assert!((e - 1.0).abs() <= 1e-13 && z.abs() <= 1e-13 * scale);
            }
        }
    }

    #[test]
    fn derivatives_match_central_differences() {
        for curve in curves(1.0) {
            let (start, end) = curve.domain();
            let (a, b) = if start.is_finite() {
                (start, end)
            } else {
                (-5.0, 5.0)
            };
            let h = 1e-6 * (b - a);
            for i in 1..20 {
                // Off the round fractions, where the NURBS circle has its
                // knots: it is only C1 there, and a central difference
                // across a knot is first order.
                let t = a + (b - a) * (f64::from(i) + 0.37) / 21.0;
                let (p, d) = curve.derivatives(t);
                assert_eq!(p, curve.eval(t));
                let fd = (curve.eval(t + h) - curve.eval(t - h)) * (0.5 / h);
                assert!(
                    d.distance(fd) <= 1e-6 * d.length().max(1.0),
                    "{} at {t}",
                    curve.kind()
                );
            }
        }
    }

    #[test]
    fn projecting_a_point_of_the_curve_returns_its_parameter() {
        for scale in [1e-3, 1.0, 1e6] {
            for curve in curves(scale) {
                let (start, end) = curve.domain();
                let (a, b) = if start.is_finite() {
                    (start, end)
                } else {
                    (-5.0 * scale, 5.0 * scale)
                };
                for i in 1..24 {
                    let t = a + (b - a) * f64::from(i) / 24.0;
                    let back = curve.project(curve.eval(t)).unwrap();
                    assert!(
                        curve.eval(back).distance(curve.eval(t)) <= 1e-9 * scale,
                        "{} at {t}, scale {scale}: got {back}",
                        curve.kind()
                    );
                    if !matches!(curve, Curve3::Nurbs(_)) {
                        assert!(
                            (back - t).abs() <= 1e-8 * (b - a),
                            "{} at {t}: got {back}",
                            curve.kind()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_foot_of_an_off_curve_point_is_perpendicular() {
        for curve in curves(1.0) {
            for p in [
                Vec3::new(4.0, 1.0, 2.0),
                Vec3::new(-3.0, -5.0, 0.7),
                Vec3::new(1.5, -1.0, 6.0),
            ] {
                let t = curve.project(p).unwrap();
                let (foot, tangent) = curve.derivatives(t);
                let cosine = (p - foot).dot(tangent) / ((p - foot).length() * tangent.length());
                assert!(cosine.abs() < 1e-7, "{}: cos = {cosine}", curve.kind());
                // And it is the nearest of many samples.
                let (start, end) = curve.domain();
                let (a, b) = if start.is_finite() {
                    (start, end)
                } else {
                    (-50.0, 50.0)
                };
                for i in 0..=2000 {
                    let s = a + (b - a) * f64::from(i) / 2000.0;
                    assert!(
                        foot.distance(p) <= curve.eval(s).distance(p) + 1e-9,
                        "{}",
                        curve.kind()
                    );
                }
            }
        }
    }

    #[test]
    fn degenerate_cases_do_not_panic() {
        assert!(Line::new(Vec3::ZERO, Vec3::ZERO).is_err());
        assert!(Line::through(Vec3::X, Vec3::X).is_err());
        assert!(Line::new(Vec3::new(f64::NAN, 0.0, 0.0), Vec3::X).is_err());
        assert!(Circle::new(Frame::WORLD, 0.0).is_err());
        assert!(Circle::new(Frame::WORLD, -1.0).is_err());
        assert!(Ellipse::new(Frame::WORLD, 1.0, f64::INFINITY).is_err());
        let circle = Curve3::Circle(Circle::new(Frame::WORLD, 1.0).unwrap());
        // On the axis every point is equally close.
        assert_eq!(circle.project(Vec3::new(0.0, 0.0, 9.0)).unwrap(), 0.0);
        assert!(circle.project(Vec3::new(f64::NAN, 0.0, 0.0)).is_err());
        // Angles come back in [0, 2π).
        let t = circle.project(Vec3::new(1.0, -1e-300, 0.0)).unwrap();
        assert!((0.0..TAU).contains(&t));
        assert_eq!(wrap_angle(-1e-300), 0.0);
        assert!((wrap_angle(-1.0) - (TAU - 1.0)).abs() < 1e-15);
    }
}
