//! Where a polynomial of low degree may change sign on an interval, for
//! the turning points of a distance along a curve (`hits.rs`): no formula
//! for the roots of a quartic (Ferrari's loses its digits near double
//! roots), but the roots of the derivative first, between which the
//! polynomial is monotone and changes sign at most once, found by
//! bisection; and so on down to a line.
//!
//! Every coefficient comes with the size of what it was made of (the sum
//! of the magnitudes of its terms), so that the rounding of a value is
//! bounded: where the value is no further from zero than that bound its
//! sign is not known, and the place is given as one where the sign may
//! change. A place given that is not a sign change only divides a curve
//! where nothing turns, which costs a stretch and changes no answer.

// `!(x > y)` is deliberate: unlike `x <= y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

/// How many units in the last place of the sizes a value of the
/// polynomial may be off. Each coefficient is made of the inputs (each
/// itself rounded a few times) in at most a dozen roundings (products of
/// four numbers and sums of three, `hits::turning_points`), relative to
/// its size; Horner's scheme adds two roundings per degree (eight), each
/// relative to the sum of the sizes times the powers of the argument.
/// About forty in all, and 256 leaves room: a bound too wide only adds
/// places where the sign may change.
const SLACK: f64 = 256.0;

/// How often an interval is halved, at most, to find a sign change (it
/// stops at a unit in the last place of the interval asked about, some
/// 53 halvings).
const HALVINGS: usize = 200;

/// A polynomial of degree at most four: `c[i]` the coefficient of `x^i`,
/// `size[i]` at least its magnitude and that of every term it was made
/// of.
#[derive(Clone, Copy, Debug)]
pub(super) struct Poly {
    pub c: [f64; 5],
    pub size: [f64; 5],
}

impl Poly {
    /// The value at `x`, and how far rounding may have put it from the
    /// value of the polynomial the inputs stand for.
    fn at(&self, x: f64) -> (f64, f64) {
        let ax = x.abs();
        let (mut value, mut bound) = (0.0, 0.0);
        for (c, size) in self.c.iter().zip(&self.size).rev() {
            value = value * x + c;
            bound = bound * ax + size.abs();
        }
        (value, SLACK * f64::EPSILON * bound)
    }

    /// The derivative (its sizes the sizes times the powers).
    fn derivative(&self) -> Poly {
        let mut d = Poly {
            c: [0.0; 5],
            size: [0.0; 5],
        };
        for i in 1..5 {
            let k = i as f64;
            if let (Some(c), Some(size), Some(dc), Some(ds)) = (
                self.c.get(i),
                self.size.get(i),
                d.c.get_mut(i - 1),
                d.size.get_mut(i - 1),
            ) {
                *dc = k * c;
                *ds = k * size.abs();
            }
        }
        d
    }

    /// The highest power with a coefficient that is not zero.
    fn degree(&self) -> usize {
        (0..5)
            .rev()
            .find(|&i| self.c.get(i).is_some_and(|&c| c != 0.0))
            .unwrap_or(0)
    }

    /// The places in `[lo, hi]`, ascending, where the polynomial may
    /// change sign: every root at which it does (found by bisection to the
    /// last bit), and every place considered (an end of the interval or of
    /// a stretch between the derivative's own places) where its sign is not
    /// known. Between two consecutive places given (and the ends) it keeps
    /// one sign, as far as the derivative's places are right; where it is
    /// monotone and of unknown sign at both ends of a stretch, a sign
    /// change within it is found when the values computed differ in sign,
    /// and is lost otherwise (the polynomial is then within its rounding
    /// of zero all along the stretch).
    pub fn changes(&self, lo: f64, hi: f64) -> Vec<f64> {
        let mut out = Vec::new();
        if !(lo.is_finite() && hi.is_finite() && lo < hi)
            || !self.c.iter().chain(&self.size).all(|x| x.is_finite())
        {
            return out;
        }
        self.changes_into(lo, hi, &mut out);
        out.sort_by(f64::total_cmp);
        out.dedup();
        out
    }

    fn changes_into(&self, lo: f64, hi: f64, out: &mut Vec<f64>) {
        let degree = self.degree();
        if degree == 0 {
            // A constant (to its rounding, perhaps zero): no sign change.
            return;
        }
        let slope = self.derivative();
        let mut ends = vec![lo];
        if degree >= 2 {
            ends.extend(slope.changes(lo, hi));
        }
        ends.push(hi);
        for pair in ends.windows(2) {
            let &[a, b] = pair else {
                continue;
            };
            if !(a < b) {
                continue;
            }
            let ((va, ea), (vb, eb)) = (self.at(a), self.at(b));
            let (unsure_a, unsure_b) = (!(va.abs() > ea), !(vb.abs() > eb));
            if unsure_a {
                out.push(a);
            }
            if unsure_b {
                out.push(b);
            }
            if (va > 0.0 && vb < 0.0) || (va < 0.0 && vb > 0.0) {
                out.push(self.root(&slope, (a, b), va > 0.0, hi - lo));
            }
        }
    }

    /// The value at `x`, alone.
    fn value(&self, x: f64) -> f64 {
        self.c.iter().rev().fold(0.0, |value, c| value * x + c)
    }

    /// The root in `(a, b)`, where the values have opposite signs
    /// (positive at `a` when `positive`), to a unit in the last place of
    /// `width` (the interval asked about: a place need be no nearer):
    /// Newton's steps from the middle with `slope` the derivative, kept
    /// within the bracket that the signs leave, which every step narrows;
    /// a step that would leave it, and every fourth, halves it instead.
    fn root(&self, slope: &Poly, (mut a, mut b): (f64, f64), positive: bool, width: f64) -> f64 {
        let close = f64::EPSILON * width;
        let mut x = 0.5 * (a + b);
        for step in 0..HALVINGS {
            let v = self.value(x);
            if v == 0.0 {
                return x;
            }
            if (v > 0.0) == positive {
                a = x;
            } else {
                b = x;
            }
            let mid = 0.5 * (a + b);
            if !(b - a > close) || !(mid > a && mid < b) {
                return mid;
            }
            let newton = x - v / slope.value(x);
            if step % 4 == 3 || !(newton > a && newton < b) {
                x = mid;
            } else if (newton - x).abs() <= close {
                return newton;
            } else {
                x = newton;
            }
        }
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The polynomial with these roots (and leading coefficient `lead`),
    /// its sizes the magnitudes of the expanded products.
    fn with_roots(lead: f64, roots: &[f64]) -> Poly {
        let mut c = [0.0; 5];
        let mut size = [0.0; 5];
        c[0] = lead;
        size[0] = lead.abs();
        for (n, &r) in roots.iter().enumerate() {
            for i in (0..=n + 1).rev() {
                let below = if i > 0 { c[i - 1] } else { 0.0 };
                let below_size = if i > 0 { size[i - 1] } else { 0.0 };
                c[i] = below - r * c[i];
                size[i] = below_size + r.abs() * size[i];
            }
        }
        Poly { c, size }
    }

    /// Simple roots are found to rounding, double roots (no sign change)
    /// give a place or none, roots outside the interval none, at several
    /// scales of the argument.
    #[test]
    fn sign_changes_are_found_where_the_roots_are() {
        for k in [1e-3, 1.0, 1e6] {
            // Four simple roots.
            let roots = [-2.0 * k, -0.5 * k, 0.3 * k, 1.7 * k];
            let p = with_roots(1.5, &roots);
            let found = p.changes(-3.0 * k, 3.0 * k);
            assert_eq!(found.len(), 4, "{found:?}");
            for (f, r) in found.iter().zip(roots) {
                assert!((f - r).abs() <= 1e-12 * k, "{f} {r}");
            }
            // Two roots outside, two inside; negative lead.
            let p = with_roots(-0.25, &roots);
            let found = p.changes(-k, k);
            assert_eq!(found.len(), 2, "{found:?}");
            // Two roots a millionth apart: both, or places about them.
            let close = [0.1 * k, 0.1 * k + 1e-6 * k, 2.0 * k, -2.0 * k];
            let p = with_roots(1.0, &close);
            let found = p.changes(-k, k);
            assert!(
                found
                    .iter()
                    .filter(|f| (*f - 0.1 * k).abs() < 1e-5 * k)
                    .count()
                    >= 2,
                "{found:?}"
            );
            // A double root: no sign change, perhaps a place at it.
            let p = with_roots(1.0, &[0.4 * k, 0.4 * k, -5.0 * k]);
            let found = p.changes(-k, k);
            assert!(
                found.iter().all(|f| (f - 0.4 * k).abs() < 1e-6 * k),
                "{found:?}"
            );
            // A triple root: a sign change, found.
            let p = with_roots(1.0, &[0.4 * k, 0.4 * k, 0.4 * k]);
            let found = p.changes(-k, k);
            assert!(!found.is_empty());
            assert!(
                found.iter().all(|f| (f - 0.4 * k).abs() < 1e-4 * k),
                "{found:?}"
            );
        }
    }

    /// Every sign change of random quartics, seen between samples, lies
    /// between two places given or at one: the polynomial keeps its sign
    /// between consecutive places.
    #[test]
    fn the_sign_is_kept_between_the_places_given() {
        let mut state = 5_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            ((state >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        };
        for _ in 0..2000 {
            let c = [next(), next(), next(), next(), next() * 0.1];
            let size = c.map(f64::abs);
            let p = Poly { c, size };
            let places = p.changes(-3.0, 3.0);
            let mut ends = vec![-3.0];
            ends.extend(&places);
            ends.push(3.0);
            for pair in ends.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let mut sign = 0.0_f64;
                for i in 1..64 {
                    let x = a + (b - a) * f64::from(i) / 64.0;
                    let (v, e) = p.at(x);
                    if v.abs() <= e {
                        continue;
                    }
                    if sign == 0.0 {
                        sign = v.signum();
                    }
                    assert_eq!(sign, v.signum(), "{c:?}: {places:?} at {x}");
                }
            }
        }
    }

    #[test]
    fn hostile_numbers_give_no_places() {
        let p = Poly {
            c: [f64::NAN, 1.0, 0.0, 0.0, 0.0],
            size: [1.0; 5],
        };
        assert!(p.changes(0.0, 1.0).is_empty());
        let p = Poly {
            c: [1.0, -2.0, 0.0, 0.0, 0.0],
            size: [1.0, 2.0, 0.0, 0.0, 0.0],
        };
        assert!(p.changes(1.0, 0.0).is_empty());
        assert!(p.changes(0.0, f64::INFINITY).is_empty());
        assert_eq!(p.changes(0.0, 1.0), vec![0.5]);
    }
}
