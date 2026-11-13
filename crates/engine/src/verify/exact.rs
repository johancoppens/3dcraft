//! Exact signs of orientation determinants of points as stored (doubles),
//! by floating-point expansion arithmetic, written from the paper (J. R.
//! Shewchuk, *Adaptive Precision Floating-Point Arithmetic and Fast Robust
//! Geometric Predicates*, Discrete & Computational Geometry 18, 1997). An
//! expansion is a sum of doubles, nonoverlapping and in increasing order of
//! size (zeros left out), held exactly: `two_sum` and `two_product` give
//! the rounded result and its error as two doubles whose sum is exact,
//! `grow` adds a double to an expansion exactly, and sums and products of
//! expansions are built from these. So the sign of a determinant built
//! from expansions is the sign of the determinant of the stored
//! coordinates, with nothing left to rounding, as long as no product
//! overflows and none of the errors falls below the smallest normal
//! double: for coordinates and their differences between about 1e-90 and
//! 1e90 (the workspace's scale rule asks 1e-3 to 1e6). Round to nearest,
//! no fused multiply-add (Rust contracts nothing) and no `unsafe`.

use craft3d_math::Vec3;

/// 2^27 + 1: splits a double into halves of at most 26 bits, whose
/// products are exact (Dekker).
const SPLITTER: f64 = 134_217_729.0;

/// a + b as the rounded sum and its error (Knuth's Two-Sum: no condition
/// on the sizes).
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let b_virtual = x - a;
    let a_virtual = x - b_virtual;
    (x, (a - a_virtual) + (b - b_virtual))
}

/// `a` as two halves whose sum is `a`.
fn split(a: f64) -> (f64, f64) {
    let c = SPLITTER * a;
    let big = c - a;
    let hi = c - big;
    (hi, a - hi)
}

/// a · b as the rounded product and its error (Dekker's Two-Product).
fn two_product(a: f64, b: f64) -> (f64, f64) {
    let x = a * b;
    let (a_hi, a_lo) = split(a);
    let (b_hi, b_lo) = split(b);
    let err = ((x - a_hi * b_hi) - a_lo * b_hi) - a_hi * b_lo;
    (x, a_lo * b_lo - err)
}

/// e + b, exactly (Grow-Expansion, zeros left out).
fn grow(e: &[f64], b: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(e.len() + 1);
    let mut q = b;
    for &x in e {
        let (s, t) = two_sum(q, x);
        if t != 0.0 {
            out.push(t);
        }
        q = s;
    }
    if q != 0.0 {
        out.push(q);
    }
    out
}

/// e + f, exactly.
fn sum(e: &[f64], f: &[f64]) -> Vec<f64> {
    f.iter().fold(e.to_vec(), |acc, &x| grow(&acc, x))
}

/// e · b, exactly.
fn scale(e: &[f64], b: f64) -> Vec<f64> {
    let mut out = Vec::new();
    for &x in e {
        let (hi, lo) = two_product(x, b);
        out = grow(&out, lo);
        out = grow(&out, hi);
    }
    out
}

/// e · f, exactly.
fn product(e: &[f64], f: &[f64]) -> Vec<f64> {
    f.iter().fold(Vec::new(), |acc, &x| sum(&acc, &scale(e, x)))
}

/// e · g − f · h, exactly.
fn cross_term(e: &[f64], g: &[f64], f: &[f64], h: &[f64]) -> Vec<f64> {
    let minus: Vec<f64> = product(f, h).iter().map(|x| -x).collect();
    sum(&product(e, g), &minus)
}

/// a − b, exactly.
fn difference(a: f64, b: f64) -> Vec<f64> {
    grow(&[a], -b)
}

/// The sign of an expansion: that of its largest component, the last.
fn sign(e: &[f64]) -> i8 {
    match e.last() {
        Some(&x) if x > 0.0 => 1,
        Some(&x) if x < 0.0 => -1,
        _ => 0,
    }
}

/// The exact sign of ((q − p) × (r − p)) · (s − p): positive where `s`
/// lies on the side of the plane `p q r` its normal points to.
pub(crate) fn orient3d(p: Vec3, q: Vec3, r: Vec3, s: Vec3) -> i8 {
    let d = |a: Vec3| {
        [
            difference(a.x, p.x),
            difference(a.y, p.y),
            difference(a.z, p.z),
        ]
    };
    let ([ax, ay, az], [bx, by, bz], [cx, cy, cz]) = (d(q), d(r), d(s));
    let mx = cross_term(&ay, &bz, &az, &by);
    let my = cross_term(&az, &bx, &ax, &bz);
    let mz = cross_term(&ax, &by, &ay, &bx);
    let v = sum(
        &sum(&product(&cx, &mx), &product(&cy, &my)),
        &product(&cz, &mz),
    );
    sign(&v)
}

/// The exact sign of (b − a) × (c − a) for points of a plane: positive
/// where `a b c` turn counter-clockwise.
pub(crate) fn orient2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> i8 {
    let (bx, by) = (difference(b[0], a[0]), difference(b[1], a[1]));
    let (cx, cy) = (difference(c[0], a[0]), difference(c[1], a[1]));
    sign(&cross_term(&bx, &cy, &by, &cx))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]
    use super::*;

    /// Points with integer coordinates `base + δ` scaled by 2^e: as doubles
    /// exact (|base + δ| < 2^53), and their determinants exact in i128
    /// from the integer differences (|δ| < 2^31: products of three below
    /// 2^99).
    struct Case {
        base: [i64; 3],
        scale: f64,
    }

    impl Case {
        fn point(&self, d: [i64; 3]) -> Vec3 {
            let c = |k: usize| (self.base[k] + d[k]) as f64 * self.scale;
            Vec3::new(c(0), c(1), c(2))
        }
    }

    fn det3(p: [i64; 3], q: [i64; 3], r: [i64; 3], s: [i64; 3]) -> i8 {
        let d = |a: [i64; 3]| {
            [
                i128::from(a[0] - p[0]),
                i128::from(a[1] - p[1]),
                i128::from(a[2] - p[2]),
            ]
        };
        let (a, b, c) = (d(q), d(r), d(s));
        let v = c[0] * (a[1] * b[2] - a[2] * b[1])
            + c[1] * (a[2] * b[0] - a[0] * b[2])
            + c[2] * (a[0] * b[1] - a[1] * b[0]);
        v.signum() as i8
    }

    fn det2(a: [i64; 2], b: [i64; 2], c: [i64; 2]) -> i8 {
        let v = i128::from(b[0] - a[0]) * i128::from(c[1] - a[1])
            - i128::from(b[1] - a[1]) * i128::from(c[0] - a[0]);
        v.signum() as i8
    }

    /// Against exact integer arithmetic (i128), directly and through the
    /// filters, on seeded random points and
    /// on the hard ones: coplanar (the fourth point an integer
    /// combination of the others), collinear, coincident corners, slivers
    /// (a third corner one unit off the line of the other two), nearly
    /// coplanar (one unit off the plane), with coordinates from about
    /// 1e-6 to 1e9 and far from the origin (offsets up to 2^48 units with
    /// differences of a few units: a part in 1e14 of the coordinates).
    /// A seeded xorshift.
    struct Rng(u64);

    impl Rng {
        /// An integer in [−range, range].
        fn int(&mut self, range: i64) -> i64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % (2 * range as u64 + 1)) as i64 - range
        }

        fn triple(&mut self, range: i64) -> [i64; 3] {
            [self.int(range), self.int(range), self.int(range)]
        }
    }

    /// 2^e, exactly.
    fn two_to(e: i32) -> f64 {
        let (step, n) = if e >= 0 { (2.0, e) } else { (0.5, -e) };
        (0..n).fold(1.0, |s, _| s * step)
    }

    #[test]
    fn exact_signs_agree_with_integer_arithmetic() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut checked = 0;
        let mut zeros = 0;
        for e in [-40, -20, 0, 20, 30] {
            let scale = two_to(e);
            for big in [0_i64, 1 << 30, 1 << 48] {
                for kind in 0..6 {
                    for _ in 0..400 {
                        let base = rng.triple(big);
                        let case = Case { base, scale };
                        let span = if kind == 0 { 1 << 30 } else { 1 << 12 };
                        let (p, q, r) = (rng.triple(span), rng.triple(span), rng.triple(span));
                        let (i, j) = (rng.int(5), rng.int(5));
                        let off = (rng.int(1) + 1) as usize;
                        let fourth = rng.triple(span);
                        let comb = |a: [i64; 3], b: [i64; 3], c: [i64; 3], i: i64, j: i64| {
                            [
                                a[0] + i * (b[0] - a[0]) + j * (c[0] - a[0]),
                                a[1] + i * (b[1] - a[1]) + j * (c[1] - a[1]),
                                a[2] + i * (b[2] - a[2]) + j * (c[2] - a[2]),
                            ]
                        };
                        let (q, r, s) = match kind {
                            // Random.
                            0 => (q, r, fourth),
                            // Coplanar.
                            1 => (q, r, comb(p, q, r, i, j)),
                            // Nearly coplanar: one unit off.
                            2 => {
                                let mut s = comb(p, q, r, i, j);
                                s[off] += 1;
                                (q, r, s)
                            }
                            // Collinear corners.
                            3 => (q, comb(p, q, q, i, 0), fourth),
                            // A sliver: r one unit off the line p q.
                            4 => {
                                let mut r = comb(p, q, q, i, 0);
                                r[off] += 1;
                                (q, r, comb(p, q, r, j, i))
                            }
                            // Coincident corners.
                            _ => (p, r, fourth),
                        };
                        let want = det3(p, q, r, s);
                        let got =
                            orient3d(case.point(p), case.point(q), case.point(r), case.point(s));
                        assert_eq!(got, want, "{e} {big} {kind} {p:?} {q:?} {r:?} {s:?}");
                        // And through the filter (with its rule for a
                        // permanent of 0).
                        let filtered = crate::verify::exact_orient(
                            case.point(p),
                            case.point(q),
                            case.point(r),
                            case.point(s),
                        );
                        assert_eq!(filtered, want, "filtered {e} {big} {kind}");
                        for k in 0..3 {
                            let pr = |a: [i64; 3]| [a[(k + 1) % 3], a[(k + 2) % 3]];
                            let pf = |a: Vec3| {
                                let c = [a.x, a.y, a.z];
                                [c[(k + 1) % 3], c[(k + 2) % 3]]
                            };
                            let want = det2(pr(p), pr(q), pr(r));
                            let got =
                                orient2d(pf(case.point(p)), pf(case.point(q)), pf(case.point(r)));
                            assert_eq!(got, want, "2d {e} {big} {kind}");
                            let filtered = crate::verify::orient2d(
                                pf(case.point(p)),
                                pf(case.point(q)),
                                pf(case.point(r)),
                            );
                            assert_eq!(filtered, want, "2d filtered {e} {big} {kind}");
                        }
                        checked += 1;
                        zeros += usize::from(want == 0);
                    }
                }
            }
        }
        assert_eq!(checked, 5 * 3 * 6 * 400);
        // The hard cases are there: a third of them exactly 0.
        assert!(zeros > checked / 4, "{zeros} of {checked}");
    }
}
