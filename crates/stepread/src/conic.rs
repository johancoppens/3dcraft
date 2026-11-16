//! Hyperbolas and parabolas (ISO 10303-42 `hyperbola`, `parabola`) and
//! the exact rational quadratic B-splines of their arcs.
//!
//! The standard's parametrisations, on the placement's origin `C` and its
//! axes `x`, `y` (`z` the normal of the conic's plane):
//!
//! - `hyperbola(position, semi_axis a, semi_imag_axis b)`, both positive
//!   lengths: `λ(u) = C + a cosh u x + b sinh u y`, `u` over all reals,
//!   the sense that of increasing `u`. It is one branch: the one on the
//!   `x` side of its centre (`x²/a² − y²/b² = 1` with `x > 0`); the other
//!   branch is a hyperbola on a placement whose `x` points the other way.
//! - `parabola(position, focal_dist F)`, a length that is not zero:
//!   `λ(u) = C + F (u² x + 2u y)`, the vertex at `C`, `4Fx = y²`, opening
//!   along `x` when `F > 0` and along `−x` when `F < 0`.
//!
//! Neither closes nor ends, so a file bounds them: an edge's vertices, a
//! trimmed curve's trims, or the faces a swept surface of one carries.
//! `craft3d-geom` has no curve of either kind; an arc of either is exactly
//! a rational quadratic B-spline, which every part of the kernel takes
//! (and which the kernel's own booleans make of a plane cut through a
//! cone, `geom::intersect`).
//!
//! **The arcs.** An arc over `[u0, u1]` is `n = ⌈u1 − u0⌉` rational
//! quadratic Bézier pieces of equal width (one unit of `u` each, at most
//! [`MAX_PIECES`], beyond that that many equal pieces), the knots at the
//! piece ends being the standard's `u` there; each piece runs from
//! `λ(lo)` to `λ(hi)`. Pieces of a unit: an exact curve in one long span
//! near the apex of a wide cone was not proven on it by `geom::certify`
//! (until its careful second proof, 2026-11-15), in a few unit spans it
//! was; both are now, and the pieces stay as they were
//! (`crates/io/tests/step_conics.rs`). A parabola's piece has its middle
//! control point at the polar form of `λ`, `C + F (lo hi x + (lo + hi) y)`, weight 1: `λ`
//! is a quadratic polynomial in `u`, so the B-spline is `λ` itself,
//! parameter for parameter. A hyperbola's piece of half width `d` about
//! `m` has the middle control point `C + E(m) / cosh d` where
//! `E(m) = a cosh m x + b sinh m y`, and weight `cosh d` (the hyperbolic
//! twin of the circle's `cos d`, as the kernel's booleans make the
//! hyperbola of a plane through a cone, `geom::intersect`). *Why it is
//! exact:* the linear map that takes `(a cosh u,
//! b sinh u)` to `(a cosh (u + m), b sinh (u + m))` (a hyperbolic
//! rotation) takes a rational Bézier curve to the one of its mapped
//! control points with the same weights, so it is enough to look at the
//! piece about `m = 0`: control points `(a cosh d, −b sinh d)`,
//! `(a / cosh d, 0)`, `(a cosh d, b sinh d)` with weights `1, cosh d, 1`.
//! Written with `σ = 2s − 1` for the piece's parameter `s ∈ [0, 1]`, its
//! point is `(a (1 + τ²), 2bτ) / (1 − τ²)` with `τ = tanh(d / 2) σ`, which
//! is `(a cosh u, b sinh u)` for `tanh(u / 2) = τ`: every point lies on
//! the branch, and `s ↦ u` runs monotonically from `−d` to `d`
//! (the tests' `arc_parameter`). Held by the tests below to rounding.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]
use craft3d_geom::NurbsCurve;
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};

/// Most pieces of an arc: one per unit of its parameter up to
/// this many units, then this many equal pieces (`2 MAX_PIECES + 1`
/// control points at most, which the control-point budget counts).
pub const MAX_PIECES: usize = 16;

/// A hyperbola or a parabola, with the standard's parametrisation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Conic {
    Hyperbola { frame: Frame, a: f64, b: f64 },
    Parabola { frame: Frame, focal: f64 },
}

impl Conic {
    /// A hyperbola with semi-axes `a` (along the frame's x) and `b`: both
    /// positive and finite (`positive_length_measure`).
    pub fn hyperbola(frame: Frame, a: f64, b: f64) -> Result<Conic, String> {
        if !(a.is_finite() && a > 0.0 && b.is_finite() && b > 0.0) {
            return Err(format!(
                "the semi-axes of a hyperbola must be positive and finite (semi_axis {a}, semi_imag_axis {b}); the other branch is a placement turned about its axis"
            ));
        }
        Ok(Conic::Hyperbola { frame, a, b })
    }

    /// A parabola with focal distance `focal` (signed: negative opens it
    /// along the frame's −x), not zero and finite.
    pub fn parabola(frame: Frame, focal: f64) -> Result<Conic, String> {
        if !(focal.is_finite() && focal != 0.0) {
            return Err(format!(
                "the focal distance of a parabola must be finite and not zero (focal_dist {focal})"
            ));
        }
        Ok(Conic::Parabola { frame, focal })
    }

    /// A short name for messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Conic::Hyperbola { .. } => "hyperbola",
            Conic::Parabola { .. } => "parabola",
        }
    }

    pub fn frame(&self) -> &Frame {
        match self {
            Conic::Hyperbola { frame, .. } | Conic::Parabola { frame, .. } => frame,
        }
    }

    /// The point at the standard's parameter `u`.
    pub fn point(&self, u: f64) -> Vec3 {
        self.derivatives(u).0
    }

    /// The point and its first and second derivatives at `u`.
    pub fn derivatives(&self, u: f64) -> (Vec3, Vec3, Vec3) {
        match *self {
            Conic::Hyperbola { frame, a, b } => {
                let (ch, sh) = (u.pcosh(), u.psinh());
                (
                    frame.point(a * ch, b * sh, 0.0),
                    frame.vector(a * sh, b * ch, 0.0),
                    frame.vector(a * ch, b * sh, 0.0),
                )
            }
            Conic::Parabola { frame, focal } => (
                frame.point(focal * u * u, 2.0 * focal * u, 0.0),
                frame.vector(2.0 * focal * u, 2.0 * focal, 0.0),
                frame.vector(2.0 * focal, 0.0, 0.0),
            ),
        }
    }

    /// The same points with the parameter running the other way: the
    /// conic on the frame seen from the other side (`y` and `z` turned),
    /// whose `λ(u)` is this one's `λ(−u)`.
    pub fn reversed(&self) -> Conic {
        let turn = |frame: &Frame| frame.reversed_at(frame.origin()).unwrap_or(*frame);
        match *self {
            Conic::Hyperbola { frame, a, b } => Conic::Hyperbola {
                frame: turn(&frame),
                a,
                b,
            },
            Conic::Parabola { frame, focal } => Conic::Parabola {
                frame: turn(&frame),
                focal,
            },
        }
    }

    /// The parameter of the point of the conic nearest `p`, for a point
    /// on or near it: the inverse of the parametrisation in the plane
    /// (`u = asinh(y / b)`, `u = y / 2F`), then Newton steps on the
    /// distance, never ending further than it started. `None` when `p` or
    /// what comes out is not finite.
    pub fn parameter(&self, p: Vec3) -> Option<f64> {
        if !p.is_finite() {
            return None;
        }
        let (_, y, _) = self.frame().local(p);
        let start = match *self {
            Conic::Hyperbola { b, .. } => (y / b).pasinh(),
            Conic::Parabola { focal, .. } => y / (2.0 * focal),
        };
        if !start.is_finite() {
            return None;
        }
        let distance = |u: f64| self.point(u).distance(p);
        let mut u = start;
        for _ in 0..32 {
            let (c, d1, d2) = self.derivatives(u);
            let g = (c - p).dot(d1);
            let slope = d1.length_squared() + (c - p).dot(d2);
            if !(slope.is_finite() && slope > 0.0) {
                break;
            }
            let step = (g / slope).clamp(-1.0, 1.0);
            if !step.is_finite() {
                break;
            }
            u -= step;
            if step.abs() <= 1e-16 * (1.0 + u.abs()) {
                break;
            }
        }
        let best = if distance(u) <= distance(start) {
            u
        } else {
            start
        };
        best.is_finite().then_some(best)
    }

    /// The exact B-spline of the arc from `u0` to `u1` (`u0 < u1`, both
    /// finite), running with the conic; its domain is `[u0, u1]`.
    pub fn arc(&self, u0: f64, u1: f64) -> Result<NurbsCurve, String> {
        if !(u0.is_finite() && u1.is_finite() && u0 < u1) {
            return Err(format!(
                "an arc of a {} from {u0} to {u1}: its ends must be finite and in order",
                self.kind()
            ));
        }
        let too_far = |e: craft3d_geom::GeomError| {
            format!(
                "the arc of the {} from {u0} to {u1} cannot be made ({e}); it reaches too far",
                self.kind()
            )
        };
        let pieces = pieces(u0, u1);
        let step = (u1 - u0) / pieces as f64;
        let end = |k: usize| {
            if k == 0 {
                u0
            } else if k >= pieces {
                u1
            } else {
                u0 + step * k as f64
            }
        };
        let mut knots = vec![u0; 3];
        let mut ctrl = vec![self.point(u0)];
        let mut weights = vec![1.0];
        for k in 0..pieces {
            let (lo, hi) = (end(k), end(k + 1));
            let (middle, weight) = match *self {
                // The polar form of λ at (lo, hi).
                Conic::Parabola { frame, focal } => {
                    (frame.point(focal * lo * hi, focal * (lo + hi), 0.0), 1.0)
                }
                Conic::Hyperbola { frame, a, b } => {
                    let (m, d) = (0.5 * (lo + hi), 0.5 * (hi - lo));
                    let w = d.pcosh();
                    (frame.point(a * m.pcosh() / w, b * m.psinh() / w, 0.0), w)
                }
            };
            ctrl.push(middle);
            weights.push(weight);
            ctrl.push(self.point(hi));
            weights.push(1.0);
            if k + 1 < pieces {
                knots.extend([hi, hi]);
            }
        }
        knots.extend([u1; 3]);
        NurbsCurve::new(2, knots, ctrl, weights).map_err(too_far)
    }

    /// The parameter of [`Conic::arc`]`(u0, u1)` at which it passes the
    /// conic's point `λ(u)`, for `u` in `[u0, u1]`: `u` itself on a
    /// parabola; on a hyperbola, within the piece `[k, k']` of half width
    /// `d` about `m`, `k + (k' − k) (1 + tanh((u − m) / 2) / tanh(d / 2)) / 2`.
    /// For the tests, which hold the arcs to the standard's points by it.
    #[cfg(test)]
    fn arc_parameter(&self, u0: f64, u1: f64, u: f64) -> f64 {
        match self {
            Conic::Parabola { .. } => u,
            Conic::Hyperbola { .. } => {
                let pieces = pieces(u0, u1);
                let step = (u1 - u0) / pieces as f64;
                let k = (((u - u0) / step).floor().max(0.0) as usize).min(pieces - 1);
                let lo = if k == 0 { u0 } else { u0 + step * k as f64 };
                let hi = if k + 1 >= pieces {
                    u1
                } else {
                    u0 + step * (k + 1) as f64
                };
                let (m, d) = (0.5 * (lo + hi), 0.5 * (hi - lo));
                let s = 0.5 * (1.0 + (0.5 * (u - m)).ptanh() / (0.5 * d).ptanh());
                lo + (hi - lo) * s
            }
        }
    }
}

/// The pieces of an arc over `[u0, u1]` (`u0 < u1`).
fn pieces(u0: f64, u1: f64) -> usize {
    let width = (u1 - u0).ceil();
    if width.is_finite() && width >= 1.0 {
        (width.min(MAX_PIECES as f64) as usize).max(1)
    } else if width.is_finite() {
        1
    } else {
        MAX_PIECES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(scale: f64) -> Vec<Frame> {
        vec![
            Frame::WORLD,
            Frame::new(
                Vec3::new(1.0, -2.0, 0.5) * scale,
                Vec3::new(0.3, -0.4, 1.0),
                Vec3::new(1.0, 0.2, 0.0),
            )
            .unwrap(),
        ]
    }

    fn conics(scale: f64) -> Vec<Conic> {
        let mut out = Vec::new();
        for frame in frames(scale) {
            for (a, b) in [(1.0, 1.0), (2.5, 0.4), (0.3, 3.0)] {
                out.push(Conic::hyperbola(frame, a * scale, b * scale).unwrap());
            }
            for focal in [0.7, -1.3, 0.05] {
                out.push(Conic::parabola(frame, focal * scale).unwrap());
            }
        }
        out
    }

    /// Arcs by the standard's parameter: narrow, wide (several pieces,
    /// and more than `MAX_PIECES` units), across the vertex, on one side.
    const RANGES: [(f64, f64); 7] = [
        (-0.3, 0.2),
        (0.1, 0.10001),
        (-2.5, 2.5),
        (0.5, 3.75),
        (-4.0, -1.0),
        (-9.0, 11.5),
        (-0.0001, 7.0),
    ];

    /// Every point of every arc is the standard's point at the parameter
    /// the construction says, to rounding, and lies on the conic's
    /// equation; the arcs end where the standard's do and run its way.
    #[test]
    fn the_arcs_are_the_standards_curves_to_rounding() {
        let mut worst = 0.0_f64;
        for scale in [1e-3, 1.0, 1e6] {
            for conic in conics(scale) {
                for (u0, u1) in RANGES {
                    let arc = conic.arc(u0, u1).unwrap();
                    assert_eq!(arc.domain(), (u0, u1));
                    let size = conic.point(u0).distance(conic.frame().origin())
                        + conic.point(u1).distance(conic.frame().origin())
                        + scale;
                    assert!(arc.eval(u0).distance(conic.point(u0)) <= 1e-15 * size);
                    assert!(arc.eval(u1).distance(conic.point(u1)) <= 1e-15 * size);
                    for k in 0..=400 {
                        let u = u0 + (u1 - u0) * f64::from(k) / 400.0;
                        let t = conic.arc_parameter(u0, u1, u);
                        assert!((u0..=u1).contains(&t), "{t} outside");
                        let (p, d) = arc.derivatives(t);
                        let (q, dq, _) = conic.derivatives(u);
                        let gap = p.distance(q) / size;
                        worst = worst.max(gap);
                        assert!(gap <= 1e-13, "{conic:?} {u0}..{u1} at {u}: {gap:e}");
                        // The same sense: the derivatives point the same way.
                        assert!(d.dot(dq) > 0.0, "{conic:?} at {u}");
                        let (x, y, z) = conic.frame().local(p);
                        let on = match conic {
                            Conic::Hyperbola { a, b, .. } => {
                                x > 0.0
                                    && ((x / a).ppowi(2) - (y / b).ppowi(2) - 1.0).abs()
                                        <= 1e-12 * (x / a).ppowi(2).max(1.0)
                            }
                            Conic::Parabola { focal, .. } => {
                                (4.0 * focal * x - y * y).abs()
                                    <= 1e-12 * (y * y).max(focal * focal)
                            }
                        };
                        assert!(on && z.abs() <= 1e-15 * size, "{conic:?} at {u}: off");
                    }
                    // Parameters run monotonically with the standard's.
                    let mut last = f64::NEG_INFINITY;
                    for k in 0..=97 {
                        let u = u0 + (u1 - u0) * f64::from(k) / 97.0;
                        let t = conic.arc_parameter(u0, u1, u);
                        assert!(t >= last);
                        last = t;
                    }
                }
            }
        }
        println!("worst gap to the standard's points, relative: {worst:e}");
    }

    /// A parabola's arc is polynomial, in pieces of a unit (8 here), with
    /// the standard's parameter; a hyperbola's of 20 units is 16 pieces.
    #[test]
    fn a_parabolas_arc_has_the_standards_parameter() {
        let conic = Conic::parabola(frames(1.0)[1], -0.8).unwrap();
        let arc = conic.arc(-3.0, 5.0).unwrap();
        assert_eq!(arc.control_points().len(), 2 * 8 + 1);
        assert!(arc.weights().iter().all(|&w| w == 1.0));
        for k in 0..=50 {
            let u = -3.0 + 8.0 * f64::from(k) / 50.0;
            assert!(arc.eval(u).distance(conic.point(u)) <= 1e-14 * 50.0);
        }
        // A hyperbola of 20 units: 16 pieces, 33 control points.
        let conic = Conic::hyperbola(Frame::WORLD, 1.0, 1.0).unwrap();
        let arc = conic.arc(-10.0, 10.0).unwrap();
        assert_eq!(arc.control_points().len(), 2 * MAX_PIECES + 1);
        assert_eq!(conic.arc(0.0, 2.5).unwrap().control_points().len(), 7);
    }

    #[test]
    fn the_parameter_of_a_point_is_its_foot() {
        for scale in [1e-3, 1.0, 1e6] {
            for conic in conics(scale) {
                for u in [-3.0, -0.7, 0.0, 0.2, 1.9, 4.5] {
                    let back = conic.parameter(conic.point(u)).unwrap();
                    assert!(
                        (back - u).abs() <= 1e-12 * (1.0 + u.abs()),
                        "{conic:?}: {u} {back}"
                    );
                    // A point off the curve along its normal in the plane
                    // comes back to the same foot.
                    let (p, d1, _) = conic.derivatives(u);
                    let normal = conic.frame().z().cross(d1).normalized().unwrap();
                    let off = p + normal * (1e-4 * scale);
                    let back = conic.parameter(off).unwrap();
                    assert!(
                        conic.point(back).distance(p) <= 1e-9 * scale * (1.0 + d1.length() / scale),
                        "{conic:?}: {u} {back}"
                    );
                    // And out of the plane.
                    let back = conic.parameter(p + conic.frame().z() * scale).unwrap();
                    assert!((back - u).abs() <= 1e-9 * (1.0 + u.abs()));
                }
            }
        }
        let conic = Conic::hyperbola(Frame::WORLD, 1.0, 2.0).unwrap();
        assert_eq!(conic.parameter(Vec3::new(f64::NAN, 0.0, 0.0)), None);
    }

    #[test]
    fn a_reversed_conic_runs_the_other_way_through_the_same_points() {
        for conic in conics(1.0) {
            let back = conic.reversed();
            for u in [-2.0, -0.3, 0.0, 1.1, 3.0] {
                assert!(
                    back.point(u).distance(conic.point(-u))
                        <= 1e-14 * (1.0 + conic.point(u).length())
                );
            }
        }
    }

    #[test]
    fn hostile_numbers_are_errors() {
        for (a, b) in [
            (0.0, 1.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (1.0, -2.0),
            (f64::NAN, 1.0),
            (1.0, f64::INFINITY),
            (f64::MIN_POSITIVE / 4.0, -0.0),
        ] {
            assert!(Conic::hyperbola(Frame::WORLD, a, b).is_err());
        }
        for f in [0.0, -0.0, f64::NAN, f64::NEG_INFINITY] {
            assert!(Conic::parabola(Frame::WORLD, f).is_err());
        }
        let h = Conic::hyperbola(Frame::WORLD, 1.0, 1.0).unwrap();
        let p = Conic::parabola(Frame::WORLD, 1.0).unwrap();
        for conic in [h, p] {
            for (u0, u1) in [
                (1.0, 1.0),
                (2.0, 1.0),
                (f64::NAN, 1.0),
                (0.0, f64::INFINITY),
                (f64::NEG_INFINITY, f64::INFINITY),
                (-1e300, 1e300),
                (0.0, 800.0),
            ] {
                let made = conic.arc(u0, u1);
                if conic == h || !(u0.is_finite() && u1.is_finite() && u0 < u1) {
                    assert!(made.is_err(), "{conic:?} {u0} {u1}");
                }
            }
        }
        // Huge semi-axes: the points overflow, an error rather than a panic.
        let huge = Conic::hyperbola(Frame::WORLD, 1e307, 1e307).unwrap();
        assert!(huge.arc(-5.0, 5.0).is_err());
        assert!(
            Conic::parabola(Frame::WORLD, 1e307)
                .unwrap()
                .arc(-5.0, 5.0)
                .is_err()
        );
    }
}
