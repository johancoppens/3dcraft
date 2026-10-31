//! Forward-mode automatic differentiation over a handful of local variables.
//!
//! Every constraint equation touches at most [`SLOTS`] unknowns (four
//! points, or a point and the eight control points of a stretch of a spline
//! of degree seven), so a value plus a fixed array of partial derivatives is
//! enough to get an exact Jacobian row from the same expression that
//! computes the residual.

use craft3d_math::Portable;
use std::ops::{Add, Div, Mul, Neg, Sub};

/// The most unknowns one scalar equation can depend on: a point and the
/// eight control points that count at one place of a spline of degree
/// seven (the other equations need eight: four points).
pub(crate) const SLOTS: usize = 18;

/// A value and its partial derivatives with respect to the local variables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Dual {
    pub v: f64,
    pub d: [f64; SLOTS],
}

fn map(a: &[f64; SLOTS], f: impl Fn(f64) -> f64) -> [f64; SLOTS] {
    let mut out = [0.0; SLOTS];
    for (o, x) in out.iter_mut().zip(a) {
        *o = f(*x);
    }
    out
}

fn zip(a: &[f64; SLOTS], b: &[f64; SLOTS], f: impl Fn(f64, f64) -> f64) -> [f64; SLOTS] {
    let mut out = [0.0; SLOTS];
    for ((o, x), y) in out.iter_mut().zip(a).zip(b) {
        *o = f(*x, *y);
    }
    out
}

impl Dual {
    pub(crate) const fn constant(v: f64) -> Dual {
        Dual { v, d: [0.0; SLOTS] }
    }

    /// The local variable in `slot`; `None` when the slot does not exist.
    pub(crate) fn variable(v: f64, slot: usize) -> Option<Dual> {
        let mut d = [0.0; SLOTS];
        *d.get_mut(slot)? = 1.0;
        Some(Dual { v, d })
    }

    pub(crate) fn is_finite(&self) -> bool {
        self.v.is_finite() && self.d.iter().all(|x| x.is_finite())
    }

    /// The sine and the cosine.
    pub(crate) fn sin_cos(self) -> (Dual, Dual) {
        let (sin, cos) = self.v.psin_cos();
        (
            Dual {
                v: sin,
                d: map(&self.d, |a| a * cos),
            },
            Dual {
                v: cos,
                d: map(&self.d, |a| -a * sin),
            },
        )
    }

    /// Same derivatives, another value (used to wrap an angle).
    pub(crate) fn with_value(self, v: f64) -> Dual {
        Dual { v, d: self.d }
    }
}

impl Add for Dual {
    type Output = Dual;
    fn add(self, o: Dual) -> Dual {
        Dual {
            v: self.v + o.v,
            d: zip(&self.d, &o.d, |a, b| a + b),
        }
    }
}

impl Sub for Dual {
    type Output = Dual;
    fn sub(self, o: Dual) -> Dual {
        Dual {
            v: self.v - o.v,
            d: zip(&self.d, &o.d, |a, b| a - b),
        }
    }
}

impl Mul for Dual {
    type Output = Dual;
    // The product rule adds.
    #[allow(clippy::suspicious_arithmetic_impl)]
    fn mul(self, o: Dual) -> Dual {
        Dual {
            v: self.v * o.v,
            d: zip(&self.d, &o.d, |a, b| a * o.v + self.v * b),
        }
    }
}

impl Div for Dual {
    type Output = Dual;
    /// The caller guards the divisor; a zero divisor gives a non-finite
    /// result that the evaluator rejects.
    #[allow(clippy::suspicious_arithmetic_impl)]
    fn div(self, o: Dual) -> Dual {
        let inv = 1.0 / o.v;
        let q = self.v * inv;
        Dual {
            v: q,
            d: zip(&self.d, &o.d, |a, b| (a - q * b) * inv),
        }
    }
}

impl Neg for Dual {
    type Output = Dual;
    fn neg(self) -> Dual {
        Dual {
            v: -self.v,
            d: map(&self.d, |a| -a),
        }
    }
}

impl Mul<f64> for Dual {
    type Output = Dual;
    fn mul(self, k: f64) -> Dual {
        Dual {
            v: self.v * k,
            d: map(&self.d, |a| a * k),
        }
    }
}

impl Sub<f64> for Dual {
    type Output = Dual;
    fn sub(self, k: f64) -> Dual {
        Dual {
            v: self.v - k,
            d: self.d,
        }
    }
}

/// Euclidean length of `(x, y)`.
///
/// At the origin the length has no derivative; the +x direction is used
/// there so that a constraint between two coincident points still has a
/// deterministic, non-zero gradient to move along.
pub(crate) fn length(x: Dual, y: Dual) -> Dual {
    let len = x.v.phypot(y.v);
    if len > 0.0 {
        let (cx, cy) = (x.v / len, y.v / len);
        Dual {
            v: len,
            d: zip(&x.d, &y.d, |a, b| cx * a + cy * b),
        }
    } else {
        Dual { v: len, d: x.d }
    }
}

/// Direction angle `atan2(y, x)` of a vector; `None` at the origin.
pub(crate) fn angle_of(x: Dual, y: Dual) -> Option<Dual> {
    let sq = x.v * x.v + y.v * y.v;
    if !(sq > 0.0 && sq.is_finite()) {
        return None;
    }
    Some(Dual {
        v: y.v.patan2(x.v),
        d: zip(&x.d, &y.d, |a, b| (x.v * b - y.v * a) / sq),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(v: f64, slot: usize) -> Dual {
        Dual::variable(v, slot).unwrap()
    }

    /// Central difference of `f` in each of the first two variables.
    fn numeric(f: impl Fn(f64, f64) -> f64, a: f64, b: f64) -> [f64; 2] {
        let h = 1e-6;
        [
            (f(a + h, b) - f(a - h, b)) / (2.0 * h),
            (f(a, b + h) - f(a, b - h)) / (2.0 * h),
        ]
    }

    fn check(dual: impl Fn(Dual, Dual) -> Dual, plain: impl Fn(f64, f64) -> f64, a: f64, b: f64) {
        let got = dual(var(a, 0), var(b, 1));
        let want = numeric(&plain, a, b);
        // Value: the same arithmetic, so equal up to rounding (1e-12).
        assert!((got.v - plain(a, b)).abs() <= 1e-12, "value {got:?}");
        // Derivative: central differences with h = 1e-6 are good to ~1e-8.
        assert!(
            (got.d[0] - want[0]).abs() <= 1e-7,
            "d/da {} vs {}",
            got.d[0],
            want[0]
        );
        assert!(
            (got.d[1] - want[1]).abs() <= 1e-7,
            "d/db {} vs {}",
            got.d[1],
            want[1]
        );
        assert!(got.d[2..].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn arithmetic_derivatives_match_finite_differences() {
        check(|a, b| a + b, |a, b| a + b, 1.5, -2.0);
        check(|a, b| a - b, |a, b| a - b, 1.5, -2.0);
        check(|a, b| a * b, |a, b| a * b, 1.5, -2.0);
        check(|a, b| a / b, |a, b| a / b, 1.5, -2.0);
        check(|a, b| -(a * b), |a, b| -(a * b), 1.5, -2.0);
        check(
            |a, b| a * 3.0 - b * 0.5,
            |a, b| a * 3.0 - b * 0.5,
            1.5,
            -2.0,
        );
        check(|a, b| a * b - 4.0, |a, b| a * b - 4.0, 1.5, -2.0);
        check(length, f64::phypot, 3.0, -4.0);
        check(
            |a, b| angle_of(a, b).unwrap(),
            |a, b| b.patan2(a),
            -3.0,
            4.0,
        );
    }

    #[test]
    fn closed_form_derivatives() {
        // d|(3,4)| = (3/5, 4/5).
        let l = length(var(3.0, 0), var(4.0, 1));
        assert_eq!(l.v, 5.0);
        assert!((l.d[0] - 0.6).abs() < 1e-15 && (l.d[1] - 0.8).abs() < 1e-15);
        // d atan2(y, x) at (x, y) = (0, 2) is (-1/2, 0): turning left moves -x.
        let a = angle_of(var(0.0, 0), var(2.0, 1)).unwrap();
        assert!((a.v - std::f64::consts::FRAC_PI_2).abs() < 1e-15);
        assert!((a.d[0] + 0.5).abs() < 1e-15 && a.d[1].abs() < 1e-15);
    }

    #[test]
    fn degenerate_inputs_are_defined() {
        let l = length(var(0.0, 0), var(0.0, 1));
        assert_eq!(l.v, 0.0);
        assert_eq!((l.d[0], l.d[1]), (1.0, 0.0));
        assert!(angle_of(var(0.0, 0), var(0.0, 1)).is_none());
        assert!(angle_of(var(f64::NAN, 0), var(1.0, 1)).is_none());
        assert!(Dual::variable(1.0, SLOTS).is_none());
        assert!(!(var(1.0, 0) / Dual::constant(0.0)).is_finite());
    }
}
