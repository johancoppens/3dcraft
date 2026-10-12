//! Signs of small determinants, exactly.
//!
//! Whether two triangles of a mesh meet is decided by the signs of 3x3 and
//! 2x2 determinants of coordinate differences. In a planar face those
//! determinants are zero or a rounding error away from it, which is where
//! plain `f64` arithmetic gives the wrong sign. Each predicate here first
//! evaluates the determinant in `f64` with a bound on its rounding error
//! (a static filter); only when the value is within the bound is it
//! evaluated again without any rounding, as a sum of non-overlapping
//! `f64` components.
//!
//! Written from J. R. Shewchuk, "Adaptive Precision Floating-Point
//! Arithmetic and Fast Robust Geometric Predicates" (Discrete &
//! Computational Geometry 18, 1997): the error-free sum and product, the
//! growing and scaling of an expansion, and the error bounds of the first
//! stage. The later adaptive stages of the paper are left out: the exact
//! stage is simply the whole determinant.
//!
//! Exact as long as no product underflows, which takes coordinate
//! differences below about 1e-100; callers refuse coordinates above 1e100.

/// A point as three coordinates.
pub(crate) type P = [f64; 3];

/// `a + b` as a rounded sum and what the rounding lost.
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    let b_virtual = sum - a;
    let a_virtual = sum - b_virtual;
    (sum, (a - a_virtual) + (b - b_virtual))
}

/// `a * b` as a rounded product and what the rounding lost.
fn two_product(a: f64, b: f64) -> (f64, f64) {
    let product = a * b;
    // A fused multiply-add rounds once, so this is the exact remainder.
    (product, a.mul_add(b, -product))
}

/// A number as a sum of `f64` components that do not overlap, smallest
/// first, without zeros. The empty expansion is zero.
type Expansion = Vec<f64>;

/// `e + b`.
fn grow(e: &[f64], b: f64) -> Expansion {
    let mut out = Vec::with_capacity(e.len() + 1);
    let mut carry = b;
    for &component in e {
        let (sum, lost) = two_sum(carry, component);
        if lost != 0.0 {
            out.push(lost);
        }
        carry = sum;
    }
    if carry != 0.0 {
        out.push(carry);
    }
    out
}

/// `e + f`.
fn sum(e: &[f64], f: &[f64]) -> Expansion {
    let mut out = e.to_vec();
    for &component in f {
        out = grow(&out, component);
    }
    out
}

/// `e - f`.
fn difference(e: &[f64], f: &[f64]) -> Expansion {
    let mut out = e.to_vec();
    for &component in f {
        out = grow(&out, -component);
    }
    out
}

/// `e * b`.
fn scale(e: &[f64], b: f64) -> Expansion {
    let mut out = Vec::with_capacity(2 * e.len());
    let Some((&first, rest)) = e.split_first() else {
        return out;
    };
    let (mut carry, lost) = two_product(first, b);
    if lost != 0.0 {
        out.push(lost);
    }
    for &component in rest {
        let (high, low) = two_product(component, b);
        let (partial, lost) = two_sum(carry, low);
        if lost != 0.0 {
            out.push(lost);
        }
        let (next, lost) = two_sum(high, partial);
        if lost != 0.0 {
            out.push(lost);
        }
        carry = next;
    }
    if carry != 0.0 {
        out.push(carry);
    }
    out
}

/// `e * f`.
fn product(e: &[f64], f: &[f64]) -> Expansion {
    let mut out = Vec::new();
    for &component in f {
        out = sum(&out, &scale(e, component));
    }
    out
}

/// The sign of an expansion: that of its largest component.
fn sign(e: &[f64]) -> i8 {
    match e.last() {
        Some(&top) if top > 0.0 => 1,
        Some(&top) if top < 0.0 => -1,
        _ => 0,
    }
}

/// `a - b`, exactly.
fn exact_difference(a: f64, b: f64) -> Expansion {
    let (high, low) = two_sum(a, -b);
    let mut out = Vec::with_capacity(2);
    if low != 0.0 {
        out.push(low);
    }
    if high != 0.0 {
        out.push(high);
    }
    out
}

fn sign_of(value: f64) -> i8 {
    if value > 0.0 {
        1
    } else if value < 0.0 {
        -1
    } else {
        0
    }
}

/// Bound on the rounding error of the `f64` 3x3 determinant, as a factor
/// of the sum of the absolute values of its terms: (7 + 56 e) e with
/// e = 2^-53 is 7.77e-16 (Shewchuk, section 4.3); rounded up.
const ORIENT3D_BOUND: f64 = 8.0e-16;

/// Likewise for the 2x2 determinant: (3 + 16 e) e is 3.33e-16.
const ORIENT2D_BOUND: f64 = 3.4e-16;

/// The sign of the determinant of the rows `a - d`, `b - d`, `c - d`:
/// negative when `d` is on the side of the triangle `a b c` that its
/// normal `(b - a) x (c - a)` points to, positive on the other side, zero
/// when the four points are in one plane.
pub(crate) fn orient3d(a: &P, b: &P, c: &P, d: &P) -> i8 {
    let [ax, ay, az] = *a;
    let [bx, by, bz] = *b;
    let [cx, cy, cz] = *c;
    let [dx, dy, dz] = *d;
    let (adx, ady, adz) = (ax - dx, ay - dy, az - dz);
    let (bdx, bdy, bdz) = (bx - dx, by - dy, bz - dz);
    let (cdx, cdy, cdz) = (cx - dx, cy - dy, cz - dz);

    let (bdy_cdz, bdz_cdy) = (bdy * cdz, bdz * cdy);
    let (cdy_adz, cdz_ady) = (cdy * adz, cdz * ady);
    let (ady_bdz, adz_bdy) = (ady * bdz, adz * bdy);
    let det = adx * (bdy_cdz - bdz_cdy) + bdx * (cdy_adz - cdz_ady) + cdx * (ady_bdz - adz_bdy);
    let permanent = (bdy_cdz.abs() + bdz_cdy.abs()) * adx.abs()
        + (cdy_adz.abs() + cdz_ady.abs()) * bdx.abs()
        + (ady_bdz.abs() + adz_bdy.abs()) * cdx.abs();
    if det.abs() > ORIENT3D_BOUND * permanent {
        return sign_of(det);
    }
    // Every term has a zero factor: a difference of two `f64` is zero only
    // when they are equal, so the determinant is zero, not just small.
    if permanent == 0.0 {
        return 0;
    }

    let (adx, ady, adz) = (
        exact_difference(ax, dx),
        exact_difference(ay, dy),
        exact_difference(az, dz),
    );
    let (bdx, bdy, bdz) = (
        exact_difference(bx, dx),
        exact_difference(by, dy),
        exact_difference(bz, dz),
    );
    let (cdx, cdy, cdz) = (
        exact_difference(cx, dx),
        exact_difference(cy, dy),
        exact_difference(cz, dz),
    );
    let minor =
        |p: &[f64], q: &[f64], r: &[f64], s: &[f64]| difference(&product(p, q), &product(r, s));
    let first = product(&adx, &minor(&bdy, &cdz, &bdz, &cdy));
    let second = product(&bdx, &minor(&cdy, &adz, &cdz, &ady));
    let third = product(&cdx, &minor(&ady, &bdz, &adz, &bdy));
    sign(&sum(&sum(&first, &second), &third))
}

/// The sign of `(a - c) x (b - c)` for points in a plane: positive when
/// `a`, `b`, `c` turn counter-clockwise, zero when they are on one line.
pub(crate) fn orient2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> i8 {
    let (acx, acy) = (a[0] - c[0], a[1] - c[1]);
    let (bcx, bcy) = (b[0] - c[0], b[1] - c[1]);
    let (left, right) = (acx * bcy, acy * bcx);
    let det = left - right;
    let permanent = left.abs() + right.abs();
    if det.abs() > ORIENT2D_BOUND * permanent {
        return sign_of(det);
    }
    if permanent == 0.0 {
        return 0;
    }
    let (acx, acy) = (exact_difference(a[0], c[0]), exact_difference(a[1], c[1]));
    let (bcx, bcy) = (exact_difference(b[0], c[0]), exact_difference(b[1], c[1]));
    sign(&difference(&product(&acx, &bcy), &product(&acy, &bcx)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SplitMix64.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        /// An integer of up to `bits` bits, either sign.
        fn int(&mut self, bits: u32) -> i64 {
            let magnitude = (self.next() >> (64 - bits)) as i64;
            if self.next() & 1 == 0 {
                magnitude
            } else {
                -magnitude
            }
        }
    }

    fn det3(m: [[i128; 3]; 3]) -> i128 {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            + m[1][0] * (m[2][1] * m[0][2] - m[2][2] * m[0][1])
            + m[2][0] * (m[0][1] * m[1][2] - m[0][2] * m[1][1])
    }

    /// The same determinant in integers, which do not round.
    fn orient3d_integers(p: [[i64; 3]; 4]) -> i8 {
        let row = |k: usize| {
            [
                i128::from(p[k][0]) - i128::from(p[3][0]),
                i128::from(p[k][1]) - i128::from(p[3][1]),
                i128::from(p[k][2]) - i128::from(p[3][2]),
            ]
        };
        det3([row(0), row(1), row(2)]).signum() as i8
    }

    fn to_f64(p: [i64; 3], unit: f64) -> P {
        [p[0] as f64 * unit, p[1] as f64 * unit, p[2] as f64 * unit]
    }

    #[test]
    fn the_error_free_operations_are_error_free() {
        let (s, e) = two_sum(1.0, 1e-30);
        assert_eq!((s, e), (1.0, 1e-30));
        let (p, e) = two_product(1.0 + f64::EPSILON, 1.0 + f64::EPSILON);
        assert_eq!(p, 1.0 + 2.0 * f64::EPSILON);
        assert_eq!(e, f64::EPSILON * f64::EPSILON);
        // (1 + e)^2 - (1 + 2e) - e^2 is zero, exactly.
        let x = [1.0, f64::EPSILON];
        let x = [x[1], x[0]];
        let square = product(&x, &x);
        let rest = difference(
            &square,
            &[f64::EPSILON * f64::EPSILON, 2.0 * f64::EPSILON, 1.0],
        );
        assert_eq!(sign(&rest), 0, "{rest:?}");
    }

    #[test]
    fn orient3d_has_the_sign_it_says() {
        let (a, b, c) = ([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(orient3d(&a, &b, &c, &[0.0, 0.0, 1.0]), -1);
        assert_eq!(orient3d(&a, &b, &c, &[0.0, 0.0, -1.0]), 1);
        assert_eq!(orient3d(&a, &b, &c, &[0.3, 0.3, 0.0]), 0);
        assert_eq!(orient2d([0.0, 0.0], [1.0, 0.0], [0.0, 1.0]), 1);
        assert_eq!(orient2d([0.0, 0.0], [0.0, 1.0], [1.0, 0.0]), -1);
        assert_eq!(orient2d([0.0, 0.0], [1.0, 1.0], [2.0, 2.0]), 0);
    }

    /// Random points with integer coordinates: integer arithmetic knows
    /// the answer. Wide coordinates, so the `f64` determinant rounds.
    #[test]
    fn orient3d_agrees_with_integers() {
        let mut rng = Rng(7);
        for unit in [1.0, 2f64.powi(-30), 2f64.powi(40)] {
            for _ in 0..2000 {
                let p: [[i64; 3]; 4] =
                    std::array::from_fn(|_| std::array::from_fn(|_| rng.int(36)));
                let [a, b, c, d] = p.map(|q| to_f64(q, unit));
                assert_eq!(orient3d(&a, &b, &c, &d), orient3d_integers(p), "{p:?}");
            }
        }
    }

    /// Four points in one plane, or one unit of the last place off it:
    /// the cases the filter cannot decide.
    #[test]
    fn orient3d_is_right_where_rounding_decides() {
        let mut rng = Rng(11);
        let mut zero = 0;
        let mut undecided = 0;
        for _ in 0..4000 {
            // a + s (b - a) + t (c - a) with integer s, t is in the plane.
            let a: [i64; 3] = std::array::from_fn(|_| rng.int(30));
            let u: [i64; 3] = std::array::from_fn(|_| rng.int(20));
            let v: [i64; 3] = std::array::from_fn(|_| rng.int(20));
            let (s, t) = (rng.int(8), rng.int(8));
            let add = |k: usize, m: i64, n: i64| a[k] + m * u[k] + n * v[k];
            let b: [i64; 3] = std::array::from_fn(|k| add(k, 1, 0));
            let c: [i64; 3] = std::array::from_fn(|k| add(k, 0, 1));
            let mut d: [i64; 3] = std::array::from_fn(|k| add(k, s, t));
            // Off the plane by one, or not at all.
            let nudge = rng.int(2).signum();
            d[(rng.next() % 3) as usize] += nudge;
            let p = [a, b, c, d];
            let expected = orient3d_integers(p);
            let [fa, fb, fc, fd] = p.map(|q| to_f64(q, 1.0));
            assert_eq!(orient3d(&fa, &fb, &fc, &fd), expected, "{p:?}");
            zero += usize::from(expected == 0);
            // Count how often the plain determinant would not have known.
            let row = |q: &P| [q[0] - fd[0], q[1] - fd[1], q[2] - fd[2]];
            let (r0, r1, r2) = (row(&fa), row(&fb), row(&fc));
            let det = r0[0] * (r1[1] * r2[2] - r1[2] * r2[1])
                + r1[0] * (r2[1] * r0[2] - r2[2] * r0[1])
                + r2[0] * (r0[1] * r1[2] - r0[2] * r1[1]);
            undecided += usize::from(sign_of(det) != expected);
        }
        // The test means something only when both kinds occur.
        assert!(zero > 500 && zero < 3500, "{zero}");
        assert!(undecided > 0, "the plain determinant was always right");
    }

    #[test]
    fn orient2d_agrees_with_integers() {
        let mut rng = Rng(13);
        for _ in 0..4000 {
            let a = [rng.int(50), rng.int(50)];
            let d = [rng.int(20), rng.int(20)];
            let (s, t) = (rng.int(10), rng.int(10));
            let b = [a[0] + s * d[0], a[1] + s * d[1]];
            let mut c = [a[0] + t * d[0], a[1] + t * d[1]];
            c[0] += rng.int(2).signum();
            let exact = ((i128::from(a[0]) - i128::from(c[0]))
                * (i128::from(b[1]) - i128::from(c[1]))
                - (i128::from(a[1]) - i128::from(c[1])) * (i128::from(b[0]) - i128::from(c[0])))
            .signum() as i8;
            let f = |p: [i64; 2]| [p[0] as f64, p[1] as f64];
            assert_eq!(orient2d(f(a), f(b), f(c)), exact, "{a:?} {b:?} {c:?}");
        }
    }

    /// Swapping two points flips the sign; a point twice gives zero.
    #[test]
    fn orient3d_is_antisymmetric() {
        let mut rng = Rng(17);
        for _ in 0..500 {
            let p: [P; 4] = std::array::from_fn(|_| {
                std::array::from_fn(|_| rng.int(40) as f64 * 2f64.powi(-20))
            });
            let s = orient3d(&p[0], &p[1], &p[2], &p[3]);
            assert_eq!(orient3d(&p[1], &p[0], &p[2], &p[3]), -s);
            assert_eq!(orient3d(&p[0], &p[1], &p[3], &p[2]), -s);
            assert_eq!(orient3d(&p[0], &p[1], &p[2], &p[0]), 0);
        }
    }
}
