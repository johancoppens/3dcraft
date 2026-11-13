//! Exact signs of orientation determinants of points as stored (doubles),
//! by floating-point expansion arithmetic, written from the paper (J. R.
//! Shewchuk, *Adaptive Precision Floating-Point Arithmetic and Fast Robust
//! Geometric Predicates*, Discrete & Computational Geometry 18, 1997). An
//! expansion is a sum of doubles, nonoverlapping and in increasing order of
//! size (zeros left out), held exactly: Two-Sum and Two-Product give the
//! rounded result and its error as two doubles whose sum is exact, and
//! Expansion-Sum and Scale-Expansion add and scale expansions exactly. So
//! the sign of a determinant built from them is the sign of the determinant
//! of the stored coordinates, with nothing left to rounding.
//!
//! **The range it holds for, proven.** Every operation is exact when no
//! value overflows and every value is a multiple of a power of two no
//! smaller than 2⁻¹⁰⁷⁴ (then sums, products of halves and their errors are
//! all representable, subnormal or not). With every coordinate 0 or of a
//! size between 2⁻²⁹⁰ and 2²⁹⁰ ([`IN_RANGE`]), each is a multiple of 2⁻³⁴²
//! (the last place of 2⁻²⁹⁰), so differences are, and the products of three
//! of them that a 3 × 3 determinant forms are multiples of 2⁻¹⁰²⁶; their
//! sizes stay below 2²⁹¹ for a difference, 2⁸⁸¹ for a sum of 192 products of
//! three, and 2⁹⁰⁸ for Dekker's split of those, below 2¹⁰²⁴. Outside that
//! range the caller does not ask (`verify::meet` takes such triangles as
//! meeting). Round to nearest, no fused multiply-add (Rust contracts
//! nothing) and no `unsafe`.
//!
//! The expansions live on the stack, each in an array as long as the most
//! components it can have where it is made (a difference 2, a product of
//! two 8, a 2 × 2 determinant 16, a term of the 3 × 3 one 64, the whole
//! 192); a longer one cannot arise, and would give no sign rather than a
//! wrong one.

use craft3d_math::Vec3;

/// The least and the greatest size of a coordinate (other than 0) for
/// which the signs are proven exact: 2⁻²⁹⁰ and 2²⁹⁰.
pub(crate) const IN_RANGE: (f64, f64) = (f64::from_bits(733 << 52), f64::from_bits(1313 << 52));

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

/// An expansion of at most `N` components on the stack.
#[derive(Clone, Copy)]
struct Expansion<const N: usize> {
    parts: [f64; N],
    len: usize,
}

impl<const N: usize> Expansion<N> {
    fn new() -> Self {
        Expansion {
            parts: [0.0; N],
            len: 0,
        }
    }

    fn parts(&self) -> &[f64] {
        self.parts.get(..self.len).unwrap_or(&[])
    }

    /// Appends a component (none if it is 0): `None` past `N`.
    fn push(&mut self, x: f64) -> Option<()> {
        if x != 0.0 {
            *self.parts.get_mut(self.len)? = x;
            self.len += 1;
        }
        Some(())
    }

    /// Its sign: that of its largest component, the last.
    fn sign(&self) -> i8 {
        match self.parts().last() {
            Some(&x) if x > 0.0 => 1,
            Some(&x) if x < 0.0 => -1,
            _ => 0,
        }
    }
}

/// a − b, exactly.
fn difference(a: f64, b: f64) -> Expansion<2> {
    let (x, y) = two_sum(a, -b);
    let mut e = Expansion::new();
    // Two components fit in two.
    let _ = e.push(y);
    let _ = e.push(x);
    e
}

/// e + f, exactly (Expansion-Sum: f's components grown into e one by one,
/// each from the place the last one started; zeros left out at the end).
fn sum<const N: usize>(e: &[f64], f: &[f64]) -> Option<Expansion<N>> {
    let (m, n) = (e.len(), f.len());
    if m + n > N {
        return None;
    }
    let mut h = [0.0; N];
    for (slot, &x) in h.iter_mut().zip(e) {
        *slot = x;
    }
    for (i, &x) in f.iter().enumerate() {
        let mut q = x;
        for slot in h.iter_mut().skip(i).take(m) {
            let (s, t) = two_sum(q, *slot);
            *slot = t;
            q = s;
        }
        *h.get_mut(i + m)? = q;
    }
    let mut out = Expansion::new();
    for &x in h.iter().take(m + n) {
        out.push(x)?;
    }
    Some(out)
}

/// e · b, exactly (Scale-Expansion).
fn scale<const N: usize>(e: &[f64], b: f64) -> Option<Expansion<N>> {
    if 2 * e.len() > N {
        return None;
    }
    let mut h = Expansion::new();
    let mut parts = e.iter();
    let Some(&first) = parts.next() else {
        return Some(h);
    };
    let (mut q, low) = two_product(first, b);
    h.push(low)?;
    for &x in parts {
        let (big, small) = two_product(x, b);
        let (s, t) = two_sum(q, small);
        h.push(t)?;
        let (s2, t2) = two_sum(big, s);
        h.push(t2)?;
        q = s2;
    }
    h.push(q)?;
    Some(h)
}

/// e · f, exactly: the sum of e scaled by each component of f.
fn product<const N: usize>(e: &[f64], f: &[f64]) -> Option<Expansion<N>> {
    let mut acc: Expansion<N> = Expansion::new();
    for &x in f {
        let scaled: Expansion<N> = scale(e, x)?;
        acc = sum(acc.parts(), scaled.parts())?;
    }
    Some(acc)
}

/// e · g − f · h, exactly, for e, f, g, h of at most 2 components each.
fn cross_term(e: &[f64], g: &[f64], f: &[f64], h: &[f64]) -> Option<Expansion<16>> {
    let left: Expansion<8> = product(e, g)?;
    let right: Expansion<8> = product(f, h)?;
    let mut minus = right;
    for x in minus.parts.iter_mut() {
        *x = -*x;
    }
    sum(left.parts(), minus.parts())
}

/// The exact sign of ((q − p) × (r − p)) · (s − p): positive where `s`
/// lies on the side of the plane `p q r` its normal points to. `None` only
/// where an expansion would outgrow its array, which the sizes above rule
/// out.
pub(crate) fn orient3d(p: Vec3, q: Vec3, r: Vec3, s: Vec3) -> Option<i8> {
    let d = |a: Vec3| {
        [
            difference(a.x, p.x),
            difference(a.y, p.y),
            difference(a.z, p.z),
        ]
    };
    let ([ax, ay, az], [bx, by, bz], [cx, cy, cz]) = (d(q), d(r), d(s));
    let mx = cross_term(ay.parts(), bz.parts(), az.parts(), by.parts())?;
    let my = cross_term(az.parts(), bx.parts(), ax.parts(), bz.parts())?;
    let mz = cross_term(ax.parts(), by.parts(), ay.parts(), bx.parts())?;
    let tx: Expansion<64> = product(mx.parts(), cx.parts())?;
    let ty: Expansion<64> = product(my.parts(), cy.parts())?;
    let tz: Expansion<64> = product(mz.parts(), cz.parts())?;
    let xy: Expansion<128> = sum(tx.parts(), ty.parts())?;
    let v: Expansion<192> = sum(xy.parts(), tz.parts())?;
    Some(v.sign())
}

/// The exact sign of (b − a) × (c − a) for points of a plane: positive
/// where `a b c` turn counter-clockwise.
pub(crate) fn orient2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<i8> {
    let ([a0, a1], [b0, b1], [c0, c1]) = (a, b, c);
    let (bx, by) = (difference(b0, a0), difference(b1, a1));
    let (cx, cy) = (difference(c0, a0), difference(c1, a1));
    Some(cross_term(bx.parts(), cy.parts(), by.parts(), cx.parts())?.sign())
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
                        assert_eq!(got, Some(want), "{e} {big} {kind} {p:?} {q:?} {r:?} {s:?}");
                        // And through the filter (with its rule for a
                        // permanent of 0).
                        let filtered = crate::verify::exact_orient(
                            case.point(p),
                            case.point(q),
                            case.point(r),
                            case.point(s),
                        );
                        assert_eq!(filtered, Some(want), "filtered {e} {big} {kind}");
                        for k in 0..3 {
                            let pr = |a: [i64; 3]| [a[(k + 1) % 3], a[(k + 2) % 3]];
                            let pf = |a: Vec3| {
                                let c = [a.x, a.y, a.z];
                                [c[(k + 1) % 3], c[(k + 2) % 3]]
                            };
                            let want = det2(pr(p), pr(q), pr(r));
                            let got =
                                orient2d(pf(case.point(p)), pf(case.point(q)), pf(case.point(r)));
                            assert_eq!(got, Some(want), "2d {e} {big} {kind}");
                            let filtered = crate::verify::orient2d(
                                pf(case.point(p)),
                                pf(case.point(q)),
                                pf(case.point(r)),
                            );
                            assert_eq!(filtered, Some(want), "2d filtered {e} {big} {kind}");
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

    /// A signed integer of any size (tests only): its magnitude in base
    /// 2^32, least significant first.
    #[derive(Clone)]
    struct Big {
        neg: bool,
        mag: Vec<u32>,
    }

    impl Big {
        /// A double times 2^shift, exactly (`shift` large enough that it is
        /// an integer).
        fn of(x: f64, shift: i32) -> Big {
            let bits = x.to_bits();
            let neg = bits >> 63 == 1;
            let raw = ((bits >> 52) & 0x7ff) as i32;
            let frac = bits & ((1u64 << 52) - 1);
            let (m, e) = if raw == 0 {
                (frac, -1074)
            } else {
                (frac | (1u64 << 52), raw - 1075)
            };
            let s = e + shift;
            assert!(s >= 0 || m == 0, "{x} {shift}");
            let mut mag = vec![0u32; (s.max(0) as usize) / 32 + 4];
            let (word, bit) = ((s.max(0) as usize) / 32, (s.max(0) as usize) % 32);
            let wide = u128::from(m) << bit;
            for i in 0..3 {
                mag[word + i] = (wide >> (32 * i)) as u32;
            }
            Big { neg, mag }.trim()
        }

        fn trim(mut self) -> Big {
            while self.mag.last() == Some(&0) {
                self.mag.pop();
            }
            if self.mag.is_empty() {
                self.neg = false;
            }
            self
        }

        fn cmp_mag(a: &[u32], b: &[u32]) -> std::cmp::Ordering {
            a.len()
                .cmp(&b.len())
                .then_with(|| a.iter().rev().cmp(b.iter().rev()))
        }

        fn add(&self, o: &Big) -> Big {
            let n = self.mag.len().max(o.mag.len()) + 1;
            let get = |v: &[u32], i: usize| u64::from(v.get(i).copied().unwrap_or(0));
            if self.neg == o.neg {
                let (mut mag, mut carry) = (vec![0u32; n], 0u64);
                for (i, slot) in mag.iter_mut().enumerate() {
                    let s = get(&self.mag, i) + get(&o.mag, i) + carry;
                    *slot = s as u32;
                    carry = s >> 32;
                }
                return Big { neg: self.neg, mag }.trim();
            }
            let (big, small, neg) = if Big::cmp_mag(&self.mag, &o.mag).is_ge() {
                (&self.mag, &o.mag, self.neg)
            } else {
                (&o.mag, &self.mag, o.neg)
            };
            let (mut mag, mut borrow) = (vec![0u32; n], 0i64);
            for (i, slot) in mag.iter_mut().enumerate() {
                let mut d = get(big, i) as i64 - get(small, i) as i64 - borrow;
                borrow = 0;
                if d < 0 {
                    d += 1 << 32;
                    borrow = 1;
                }
                *slot = d as u32;
            }
            Big { neg, mag }.trim()
        }

        fn sub(&self, o: &Big) -> Big {
            self.add(&Big {
                neg: !o.neg,
                mag: o.mag.clone(),
            })
        }

        fn mul(&self, o: &Big) -> Big {
            let mut mag = vec![0u64; self.mag.len() + o.mag.len() + 1];
            for (i, &a) in self.mag.iter().enumerate() {
                let mut carry = 0u64;
                for (j, &b) in o.mag.iter().enumerate() {
                    let t = mag[i + j] + u64::from(a) * u64::from(b) + carry;
                    mag[i + j] = t & 0xffff_ffff;
                    carry = t >> 32;
                }
                let mut k = i + o.mag.len();
                while carry > 0 {
                    let t = mag[k] + carry;
                    mag[k] = t & 0xffff_ffff;
                    carry = t >> 32;
                    k += 1;
                }
            }
            Big {
                neg: self.neg != o.neg,
                mag: mag.into_iter().map(|x| x as u32).collect(),
            }
            .trim()
        }

        fn sign(&self) -> i8 {
            if self.mag.is_empty() {
                0
            } else if self.neg {
                -1
            } else {
                1
            }
        }
    }

    /// The exact sign of the 3 × 3 determinant of the stored points, in
    /// integers.
    fn big_orient3d(p: Vec3, q: Vec3, r: Vec3, s: Vec3) -> i8 {
        let all = [p, q, r, s];
        let low = all
            .iter()
            .flat_map(|v| [v.x, v.y, v.z])
            .filter(|x| *x != 0.0)
            .map(|x| {
                let raw = ((x.to_bits() >> 52) & 0x7ff) as i32;
                if raw == 0 { -1074 } else { raw - 1075 }
            })
            .min()
            .unwrap_or(0);
        let b = |x: f64| Big::of(x, -low);
        let d = |a: Vec3| {
            [
                b(a.x).sub(&b(p.x)),
                b(a.y).sub(&b(p.y)),
                b(a.z).sub(&b(p.z)),
            ]
        };
        let ([ax, ay, az], [bx, by, bz], [cx, cy, cz]) = (d(q), d(r), d(s));
        let mx = ay.mul(&bz).sub(&az.mul(&by));
        let my = az.mul(&bx).sub(&ax.mul(&bz));
        let mz = ax.mul(&by).sub(&ay.mul(&bx));
        cx.mul(&mx).add(&cy.mul(&my)).add(&cz.mul(&mz)).sign()
    }

    /// Against exact integer arithmetic of any size, on doubles whose
    /// differences round (coordinates near 2^30 with fractions beside ones
    /// near 2^-10, so a difference needs up to 93 bits and is carried as
    /// two parts), and on nearly coplanar points made by rounding a point
    /// of the plane: the signs, through the filter and alone.
    #[test]
    fn exact_signs_hold_where_differences_round() {
        let mut rng = Rng(0x5851_f42d_4c95_7f2d);
        let unit = |rng: &mut Rng| (rng.int(1 << 40) as f64) / (1u64 << 40) as f64;
        let (mut checked, mut two_part, mut zeros) = (0, 0, 0);
        for _ in 0..6000 {
            let coordinate = |rng: &mut Rng| {
                if rng.int(1) >= 0 {
                    (1u64 << 30) as f64 * (1.0 + unit(rng)) + unit(rng)
                } else {
                    unit(rng) / 1024.0
                }
            };
            let point =
                |rng: &mut Rng| Vec3::new(coordinate(rng), coordinate(rng), coordinate(rng));
            let (p, q, r) = (point(&mut rng), point(&mut rng), point(&mut rng));
            let s = if rng.int(1) >= 0 {
                point(&mut rng)
            } else {
                // A point of the plane, rounded: nearly or exactly coplanar.
                let (i, j) = (unit(&mut rng), unit(&mut rng));
                p + (q - p) * i + (r - p) * j
            };
            two_part += usize::from(
                [q, r, s]
                    .iter()
                    .any(|a| difference(a.x, p.x).len > 1 || difference(a.y, p.y).len > 1),
            );
            let want = big_orient3d(p, q, r, s);
            assert_eq!(orient3d(p, q, r, s), Some(want), "{p:?} {q:?} {r:?} {s:?}");
            assert_eq!(crate::verify::exact_orient(p, q, r, s), Some(want));
            checked += 1;
            zeros += usize::from(want == 0);
        }
        assert!(two_part > checked / 2, "{two_part} of {checked}");
        let _ = zeros;
    }
}
