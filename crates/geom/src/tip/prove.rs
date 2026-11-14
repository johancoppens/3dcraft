//! The proof that a tip's faces are seen from the tip point from one
//! side: a polynomial of two variables shown positive over the unit
//! square on the convex hull of its Bernstein coefficients.
//!
//! A polynomial in Bernstein form lies between its least and its largest
//! coefficient, and its four corner coefficients are its values at the
//! corners. So a piece whose coefficients are all above the floor is
//! proven, a piece with a corner below what rounding could have made of
//! a zero is refuted, and any other piece is halved (de Casteljau's
//! algorithm: the same polynomial over half the range, its coefficients
//! closer to it).
//!
//! The floor is what rounding can make of an exact zero, and nothing
//! else. Beside the determinant goes a bound on what it can be wrong by,
//! as a count of units in the last place: the product of the three
//! vectors' lengths (a difference taken as the sum of its two lengths),
//! and the same with each vector in turn taken as long as its points lie
//! from the origin, which is what the last place of a point's
//! coordinates is worth. The determinant must stand above [`ROUNDING`]
//! times that. Both scale alike with the model's size, with every
//! weight and with the fullness, so the floor depends on none of them: a
//! tip is accepted when its faces are certainly seen from one side,
//! however little. It does grow with the distance from the origin, as
//! the coordinates' last place does. Reference: Farin, *Curves and
//! Surfaces for CAGD*, 5th ed., ch. 5 (subdivision, the convex hull
//! property) and ch. 6 (products of Bernstein forms).

use craft3d_math::Vec3;
use std::ops::{Add, Mul};

/// How often one span is halved before the proof gives up.
const MAX_HALVINGS: u32 = 64;
/// The most pieces of one span that are looked at before the proof gives
/// up.
const MAX_PIECES: usize = 1 << 13;
/// The share of the terms' sizes below which a determinant may be a zero
/// that rounding moved: 1024 units in the last place, where the sums and
/// halvings behind a coefficient lose a few hundred at most.
pub(super) const ROUNDING: f64 = 1024.0 * f64::EPSILON;
/// How often a span of the shoulder is halved for a bound on its sine.
const SINE_HALVINGS: u32 = 6;

/// The work a proof may still do, in coefficients written: one count for
/// everything one call proves, so that no input makes it run on.
pub(super) struct Work(pub u64);

impl Work {
    /// Takes `cost` from what is left; false when there is not enough.
    pub fn spend(&mut self, cost: usize) -> bool {
        let cost = u64::try_from(cost).unwrap_or(u64::MAX);
        if self.0 < cost {
            self.0 = 0;
            return false;
        }
        self.0 -= cost;
        true
    }

    pub fn spent(&self) -> bool {
        self.0 == 0
    }
}

/// A polynomial of two variables over the unit square in Bernstein form:
/// degree `du` by `dv`, coefficient `(i, j)` at `i * (dv + 1) + j`.
#[derive(Clone, Debug)]
pub(super) struct Grid<T> {
    pub du: usize,
    pub dv: usize,
    pub c: Vec<T>,
}

/// `n` over `k`, as a float (the degrees are small).
fn binomial(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    let mut value = 1.0;
    for i in 0..k {
        value *= (n - i) as f64 / (i + 1) as f64;
    }
    value
}

/// `C(a, i) C(b, k) / C(a + b, i + k)` at `i * (b + 1) + k`: how the
/// coefficients of two Bernstein forms of degrees `a` and `b` blend into
/// those of their product. The numbers of one `i + k` add up to 1, so a
/// product's coefficient is an average of products of coefficients.
fn blend(a: usize, b: usize) -> Vec<f64> {
    let mut table = Vec::with_capacity((a + 1) * (b + 1));
    for i in 0..=a {
        for k in 0..=b {
            table.push(binomial(a, i) * binomial(b, k) / binomial(a + b, i + k));
        }
    }
    table
}

/// The product of two Bernstein forms, `mul` the product of two
/// coefficients.
fn product<A: Copy, B: Copy, O>(
    f: &Grid<A>,
    g: &Grid<B>,
    zero: O,
    mul: impl Fn(A, B) -> O,
) -> Grid<O>
where
    O: Copy + Add<Output = O> + Mul<f64, Output = O>,
{
    let (du, dv) = (f.du + g.du, f.dv + g.dv);
    let (blend_u, blend_v) = (blend(f.du, g.du), blend(f.dv, g.dv));
    let mut c = vec![zero; (du + 1) * (dv + 1)];
    for (index, &a) in f.c.iter().enumerate() {
        let (i, j) = (index / (f.dv + 1), index % (f.dv + 1));
        for (other, &b) in g.c.iter().enumerate() {
            let (k, l) = (other / (g.dv + 1), other % (g.dv + 1));
            let (Some(&bu), Some(&bv)) = (
                blend_u.get(i * (g.du + 1) + k),
                blend_v.get(j * (g.dv + 1) + l),
            ) else {
                continue;
            };
            if let Some(slot) = c.get_mut((i + k) * (dv + 1) + j + l) {
                *slot = *slot + mul(a, b) * (bu * bv);
            }
        }
    }
    Grid { du, dv, c }
}

/// The lengths of a form's coefficients: a form that is nowhere below the
/// length of the one it is made from.
fn lengths(grid: &Grid<Vec3>) -> Grid<f64> {
    Grid {
        du: grid.du,
        dv: grid.dv,
        c: grid.c.iter().map(|v| v.length()).collect(),
    }
}

/// For one Bézier span of a tip face: `det[Y, ∂Y/∂u, ∂Y/∂v]`, and beside
/// it what rounding is worth in it, in units of the last place: the
/// product of the three vectors' lengths (each difference taken as the
/// sum of its two lengths), plus that product with each vector in turn
/// replaced by `reach`, how far its control points lie from the origin
/// (times their weights): what the determinant is measured against,
/// coefficient by coefficient.
///
/// `rows[j]` are the span's control vectors of row `j` of the face
/// without its last row (the tip): the homogeneous control points taken
/// about the tip, `w (R − P)`. With `q` the degree across, the face about
/// the tip is `(1 − v) Y(u, v)`, `Y` of degree `q − 1` across with the
/// rows times `q / (q − j)`: the factor `1 − v` is the face closing into
/// the tip and takes no part in which way the face is seen. The
/// determinant is positive where the ray from the tip crosses the face
/// from one side, negative from the other, and zero where it touches.
pub(super) fn seen_from_tip(
    rows: &[Vec<Vec3>],
    reach: &[Vec<f64>],
) -> Option<(Grid<f64>, Grid<f64>)> {
    let q = rows.len();
    let count = rows.first()?.len();
    if q < 2
        || count < 2
        || reach.len() != q
        || rows.iter().any(|row| row.len() != count)
        || reach.iter().any(|row| row.len() != count)
    {
        return None;
    }
    let (du, dv) = (count - 1, q - 1);
    let mut y = Grid {
        du,
        dv,
        c: Vec::with_capacity(count * q),
    };
    let mut far = Grid {
        du,
        dv,
        c: Vec::with_capacity(count * q),
    };
    for i in 0..count {
        for (j, (row, reach)) in rows.iter().zip(reach).enumerate() {
            let raise = q as f64 / (q - j) as f64;
            y.c.push(*row.get(i)? * raise);
            far.c.push(*reach.get(i)? * raise);
        }
    }
    let at = |i: usize, j: usize| y.c.get(i * q + j).copied();
    let far_at = |i: usize, j: usize| far.c.get(i * q + j).copied();
    let mut along = Grid {
        du: du - 1,
        dv,
        c: Vec::with_capacity(du * q),
    };
    let mut along_size = Grid {
        du: du - 1,
        dv,
        c: Vec::with_capacity(du * q),
    };
    let mut along_far = along_size.clone();
    for i in 0..du {
        for j in 0..q {
            let (a, b) = (at(i, j)?, at(i + 1, j)?);
            along.c.push((b - a) * du as f64);
            along_size.c.push((a.length() + b.length()) * du as f64);
            along_far
                .c
                .push((far_at(i, j)? + far_at(i + 1, j)?) * du as f64);
        }
    }
    let mut across = Grid {
        du,
        dv: dv - 1,
        c: Vec::with_capacity(count * dv),
    };
    let mut across_size = Grid {
        du,
        dv: dv - 1,
        c: Vec::with_capacity(count * dv),
    };
    let mut across_far = across_size.clone();
    for i in 0..count {
        for j in 0..dv {
            let (a, b) = (at(i, j)?, at(i, j + 1)?);
            across.c.push((b - a) * dv as f64);
            across_size.c.push((a.length() + b.length()) * dv as f64);
            across_far
                .c
                .push((far_at(i, j)? + far_at(i, j + 1)?) * dv as f64);
        }
    }
    let normal = product(&along, &across, Vec3::ZERO, Vec3::cross);
    let seen = product(&y, &normal, 0.0, Vec3::dot);
    let times = |a: &Grid<f64>, b: &Grid<f64>, c: &Grid<f64>| {
        product(a, &product(b, c, 0.0, |x, y| x * y), 0.0, |x, y| x * y)
    };
    let near = lengths(&y);
    let mut size = times(&near, &along_size, &across_size);
    for other in [
        times(&far, &along_size, &across_size),
        times(&near, &along_far, &across_size),
        times(&near, &along_size, &across_far),
    ] {
        for (sum, x) in size.c.iter_mut().zip(other.c) {
            *sum += x;
        }
    }
    let finite = |grid: &Grid<f64>| grid.c.iter().all(|x| x.is_finite());
    (finite(&seen) && finite(&size)).then_some((seen, size))
}

/// A lower bound on the sine of the angle at which rays from the tip
/// cross the curve of the row before the tip (the shoulder), over one
/// Bézier span of it: `points` are its control vectors about the tip
/// times their weights, `weights` the weights, `normal` the unit normal
/// of the tip's plane on the side from which the curve runs
/// counter-clockwise. 0 where no bound is found.
///
/// With `A = Σ B a` and `W = Σ B w` the curve about the tip is `A / W`,
/// its tangent `(A′ W − A W′) / W²`, and the sine of the angle between
/// the two is `n · (A × A′) W / (|A| |A′ W − A W′|)`. The numerator is at
/// least its least Bernstein coefficient times the lightest weight, the
/// denominator at most the longest control vector times the longest
/// coefficient of `A′ W − A W′`. The span is halved three times (the
/// bound is poor on a long span) and on until the numerator's
/// coefficients are positive, six times at most.
pub(super) fn shoulder_sine(points: &[Vec3], weights: &[f64], normal: Vec3) -> f64 {
    let mut stack = vec![(points.to_vec(), weights.to_vec(), 0_u32)];
    let mut least = f64::INFINITY;
    while let Some((points, weights, halvings)) = stack.pop() {
        let Some(degree) = points.len().checked_sub(1).filter(|d| *d >= 1) else {
            return 0.0;
        };
        if weights.len() != points.len() {
            return 0.0;
        }
        let curve = Grid {
            du: degree,
            dv: 0,
            c: points.clone(),
        };
        let weight = Grid {
            du: degree,
            dv: 0,
            c: weights.clone(),
        };
        let steps = |c: &[Vec3]| -> Vec<Vec3> {
            c.iter()
                .zip(c.iter().skip(1))
                .map(|(a, b)| (*b - *a) * degree as f64)
                .collect()
        };
        let slope = Grid {
            du: degree - 1,
            dv: 0,
            c: steps(&points),
        };
        let weight_slope = Grid {
            du: degree - 1,
            dv: 0,
            c: weights
                .iter()
                .zip(weights.iter().skip(1))
                .map(|(a, b)| (b - a) * degree as f64)
                .collect(),
        };
        let turn = product(&curve, &slope, 0.0, |a, b| normal.dot(a.cross(b)));
        let run = product(&slope, &weight, Vec3::ZERO, |a, w| a * w);
        let back = product(&curve, &weight_slope, Vec3::ZERO, |a, w| a * w);
        let lowest = turn.c.iter().copied().fold(f64::INFINITY, f64::min);
        let lightest = weights.iter().copied().fold(f64::INFINITY, f64::min);
        let longest = points.iter().fold(0.0_f64, |m, p| m.max(p.length()));
        let fastest = run
            .c
            .iter()
            .zip(&back.c)
            .fold(0.0_f64, |m, (a, b)| m.max((*a - *b).length()));
        let bound = lowest * lightest / (longest * fastest);
        // Three halvings first: the bound is poor on a long span.
        if halvings >= 3 && lowest > 0.0 && bound.is_finite() {
            least = least.min(bound);
            continue;
        }
        if halvings >= SINE_HALVINGS {
            return 0.0;
        }
        // The two halves: de Casteljau on the vectors and the weights.
        let mut work: Vec<(Vec3, f64)> = points.into_iter().zip(weights).collect();
        let mut left = Vec::with_capacity(work.len());
        let mut right = Vec::with_capacity(work.len());
        while let (Some(&first), Some(&last)) = (work.first(), work.last()) {
            left.push(first);
            right.push(last);
            work = work
                .windows(2)
                .map(|pair| match pair {
                    [(a, x), (b, y)] => ((*a + *b) * 0.5, 0.5 * (x + y)),
                    _ => (Vec3::ZERO, 0.0),
                })
                .collect();
        }
        right.reverse();
        for half in [left, right] {
            let (points, weights) = half.into_iter().unzip();
            stack.push((points, weights, halvings + 1));
        }
    }
    if least.is_finite() {
        least.max(0.0)
    } else {
        0.0
    }
}

/// The polynomial along `v = 1`: the last coefficient of every line
/// across.
pub(super) fn last_across(grid: &Grid<f64>) -> Grid<f64> {
    Grid {
        du: grid.du,
        dv: 0,
        c: grid
            .c
            .iter()
            .skip(grid.dv)
            .step_by(grid.dv + 1)
            .copied()
            .collect(),
    }
}

/// What the hulls say about a polynomial against its floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Verdict {
    /// Everywhere above the floor.
    Proven,
    /// Below nothing at `(u, v)`, by more than the floor: negative there
    /// whatever rounding did.
    Refuted { u: f64, v: f64 },
    /// Neither: around `(u, v)` the polynomial is within the floor of
    /// nothing, or the work allowed ran out there.
    Undecided { u: f64, v: f64 },
}

/// De Casteljau's algorithm at one half on one line of coefficients: the
/// coefficients of the two halves.
fn halve_line(line: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut work = line.to_vec();
    let mut left = Vec::with_capacity(line.len());
    let mut right = Vec::with_capacity(line.len());
    while let (Some(&first), Some(&last)) = (work.first(), work.last()) {
        left.push(first);
        right.push(last);
        work = work
            .windows(2)
            .map(|pair| match pair {
                [a, b] => 0.5 * (a + b),
                _ => 0.0,
            })
            .collect();
    }
    right.reverse();
    (left, right)
}

/// The two halves of a polynomial, cut along u or along v.
fn halve(grid: &Grid<f64>, along_u: bool) -> (Grid<f64>, Grid<f64>) {
    let (nu, nv) = (grid.du + 1, grid.dv + 1);
    let mut low = grid.clone();
    let mut high = grid.clone();
    let (lines, length, step, stride) = if along_u {
        (nv, nu, nv, 1)
    } else {
        (nu, nv, 1, nv)
    };
    for line in 0..lines {
        let index = |k: usize| line * stride + k * step;
        let values: Vec<f64> = (0..length)
            .filter_map(|k| grid.c.get(index(k)).copied())
            .collect();
        let (left, right) = halve_line(&values);
        for (k, (a, b)) in left.into_iter().zip(right).enumerate() {
            if let Some(slot) = low.c.get_mut(index(k)) {
                *slot = a;
            }
            if let Some(slot) = high.c.get_mut(index(k)) {
                *slot = b;
            }
        }
    }
    (low, high)
}

/// How far the coefficients of one line differ at most, over the lines
/// along u (`along_u`) or across.
fn spread(grid: &Grid<f64>, along_u: bool) -> f64 {
    let (nu, nv) = (grid.du + 1, grid.dv + 1);
    let (lines, length, step, stride) = if along_u {
        (nv, nu, nv, 1)
    } else {
        (nu, nv, 1, nv)
    };
    let mut most = 0.0_f64;
    for line in 0..lines {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for k in 0..length {
            if let Some(&x) = grid.c.get(line * stride + k * step) {
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
        most = most.max(hi - lo);
    }
    most
}

struct Piece {
    /// The polynomial less its floor, and plus it.
    low: Grid<f64>,
    high: Grid<f64>,
    u: (f64, f64),
    v: (f64, f64),
    halvings: u32,
}

/// Whether the polynomial `seen` is above `ROUNDING` times `size` (a
/// polynomial that is nowhere negative) over the whole unit square.
pub(super) fn certain(seen: &Grid<f64>, size: &Grid<f64>, work: &mut Work) -> Verdict {
    let with = |sign: f64| Grid {
        du: seen.du,
        dv: seen.dv,
        c: seen
            .c
            .iter()
            .zip(&size.c)
            .map(|(x, room)| x + sign * ROUNDING * room)
            .collect(),
    };
    let whole = Verdict::Undecided { u: 0.5, v: 0.5 };
    if seen.c.len() != size.c.len() || (seen.du, seen.dv) != (size.du, size.dv) {
        return whole;
    }
    let mut stack = vec![Piece {
        low: with(-1.0),
        high: with(1.0),
        u: (0.0, 1.0),
        v: (0.0, 1.0),
        halvings: 0,
    }];
    let mut pieces = 0_usize;
    while let Some(piece) = stack.pop() {
        pieces += 1;
        let Piece {
            low,
            high,
            u,
            v,
            halvings,
        } = piece;
        let middle = (0.5 * (u.0 + u.1), 0.5 * (v.0 + v.1));
        let undecided = Verdict::Undecided {
            u: middle.0,
            v: middle.1,
        };
        if low.c.iter().chain(&high.c).any(|x| !x.is_finite()) {
            return undecided;
        }
        if low.c.iter().all(|x| *x > 0.0) {
            continue;
        }
        // The corners' coefficients are the polynomial's values there.
        let (nu, nv) = (low.du + 1, low.dv + 1);
        let corners = [
            (0, u.0, v.0),
            (nv - 1, u.0, v.1),
            ((nu - 1) * nv, u.1, v.0),
            (nu * nv - 1, u.1, v.1),
        ];
        for (index, at_u, at_v) in corners {
            let (Some(&least), Some(&most)) = (low.c.get(index), high.c.get(index)) else {
                continue;
            };
            if most < 0.0 {
                return Verdict::Refuted { u: at_u, v: at_v };
            }
            if least <= 0.0 {
                // Within the floor of nothing at a corner: no halving
                // moves a corner's value.
                return Verdict::Undecided { u: at_u, v: at_v };
            }
        }
        let cost = 2 * low.c.len() * nu.max(nv);
        if halvings >= MAX_HALVINGS || pieces >= MAX_PIECES || !work.spend(cost) {
            return undecided;
        }
        let along_u = low.dv == 0 || (low.du > 0 && spread(&low, true) >= spread(&low, false));
        let (low_first, low_second) = halve(&low, along_u);
        let (high_first, high_second) = halve(&high, along_u);
        let (first, second) = if along_u {
            (((u.0, middle.0), v), ((middle.0, u.1), v))
        } else {
            ((u, (v.0, middle.1)), (u, (middle.1, v.1)))
        };
        stack.push(Piece {
            low: low_second,
            high: high_second,
            u: second.0,
            v: second.1,
            halvings: halvings + 1,
        });
        stack.push(Piece {
            low: low_first,
            high: high_first,
            u: first.0,
            v: first.1,
            halvings: halvings + 1,
        });
    }
    Verdict::Proven
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(c: &[f64]) -> Grid<f64> {
        Grid {
            du: c.len() - 1,
            dv: 0,
            c: c.to_vec(),
        }
    }

    #[test]
    fn a_product_of_bernstein_forms_is_the_product_of_the_polynomials() {
        // (1 − t) + 3 t  times  2 (1 − t)² + (1 − t) t · 2 · 0.5 + 4 t².
        let f = line(&[1.0, 3.0]);
        let g = line(&[2.0, 0.5, 4.0]);
        let h = product(&f, &g, 0.0, |a, b| a * b);
        assert_eq!((h.du, h.dv), (3, 0));
        let power = |x: f64, n: usize| (0..n).fold(1.0, |p, _| p * x);
        let value = |c: &[f64], t: f64| {
            let n = c.len() - 1;
            c.iter()
                .enumerate()
                .map(|(i, x)| binomial(n, i) * power(t, i) * power(1.0 - t, n - i) * x)
                .sum::<f64>()
        };
        for k in 0..=10 {
            let t = f64::from(k) / 10.0;
            let want = value(&f.c, t) * value(&g.c, t);
            assert!((value(&h.c, t) - want).abs() < 1e-14, "{t}");
        }
    }

    #[test]
    fn halving_keeps_the_polynomial() {
        let (left, right) = halve_line(&[1.0, -2.0, 5.0, 0.5]);
        // The ends, and the shared middle value: the polynomial at 1/2.
        assert_eq!((left[0], right[3]), (1.0, 0.5));
        assert_eq!(left[3], right[0]);
        let at_half = (1.0 + 3.0 * -2.0 + 3.0 * 5.0 + 0.5) / 8.0;
        assert!((left[3] - at_half).abs() < 1e-15);
    }

    /// A form of the same degrees with every coefficient `x`.
    fn level(like: &Grid<f64>, x: f64) -> Grid<f64> {
        Grid {
            du: like.du,
            dv: like.dv,
            c: vec![x; like.c.len()],
        }
    }

    fn judge(grid: &Grid<f64>) -> Verdict {
        // A size of one: the floor is `ROUNDING` itself.
        certain(grid, &level(grid, 1.0), &mut Work(1 << 30))
    }

    #[test]
    fn a_parabola_is_proven_refuted_or_left_open_by_its_least_value() {
        // (t − 1/3)² + c, in Bernstein form: least value c at t = 1/3.
        let parabola = |c: f64| line(&[1.0 / 9.0 + c, -2.0 / 9.0 + c, 4.0 / 9.0 + c]);
        // The hull of the whole range dips below zero; halving proves it.
        assert_eq!(judge(&parabola(1e-6)), Verdict::Proven);
        // Below zero around its least: a corner of some piece shows it.
        match judge(&parabola(-1e-6)) {
            Verdict::Refuted { u, .. } => assert!((u - 1.0 / 3.0).abs() < 2e-3, "{u}"),
            other => panic!("{other:?}"),
        }
        // Touching zero, and within the floor of it either way: never
        // decided, and not called negative.
        for c in [0.0, 0.5 * ROUNDING, -0.5 * ROUNDING] {
            assert!(
                matches!(judge(&parabola(c)), Verdict::Undecided { .. }),
                "{c}"
            );
        }
        // The floor goes with the size of the terms, not with the
        // polynomial's own: the same parabola a million times smaller
        // against terms of its own size is proven as before.
        let small = line(
            &parabola(1e-6)
                .c
                .iter()
                .map(|x| x * 1e-6)
                .collect::<Vec<_>>(),
        );
        let verdict = certain(&small, &level(&small, 1e-6), &mut Work(1 << 30));
        assert_eq!(verdict, Verdict::Proven);
        // And with no work allowed nothing is decided.
        let verdict = certain(&parabola(1e-6), &level(&small, 1.0), &mut Work(0));
        assert!(matches!(verdict, Verdict::Undecided { .. }));
    }

    #[test]
    fn two_variables_are_halved_both_ways() {
        // (u − 0.3)² + (v − 0.7)² + c: degree 2 by 2.
        let grid = |c: f64| {
            let fu = [0.09, -0.21, 0.49];
            let fv = [0.49, -0.21, 0.09];
            Grid {
                du: 2,
                dv: 2,
                c: fu
                    .iter()
                    .flat_map(|a| fv.iter().map(move |b| a + b + c))
                    .collect(),
            }
        };
        assert_eq!(judge(&grid(1e-4)), Verdict::Proven);
        match judge(&grid(-1e-4)) {
            Verdict::Refuted { u, v } => {
                assert!((u - 0.3).abs() < 0.02 && (v - 0.7).abs() < 0.02, "{u} {v}");
            }
            other => panic!("{other:?}"),
        }
        let edge = last_across(&grid(0.0));
        assert_eq!(edge.c.len(), 3);
        assert!((edge.c[0] - 0.18).abs() < 1e-15);
    }

    #[test]
    fn the_sine_at_which_a_circle_is_seen_from_inside_is_bounded_from_below() {
        // A quarter of the unit circle about the origin, seen from a
        // point at `off` along x: the rays cross it at a sine of at least
        // √(1 − off²).
        let s = std::f64::consts::FRAC_1_SQRT_2;
        for off in [0.0, 0.5, 0.9] {
            let about = |x: f64, y: f64, w: f64| Vec3::new(x - off, y, 0.0) * w;
            let points = [
                about(1.0, 0.0, 1.0),
                about(1.0, 1.0, s),
                about(0.0, 1.0, 1.0),
            ];
            let bound = shoulder_sine(&points, &[1.0, s, 1.0], Vec3::Z);
            let truth = (1.0 - off * off).sqrt();
            assert!(
                bound > 0.5 * truth && bound <= truth,
                "{off}: {bound} against {truth}"
            );
            // The other way round it is seen from the wrong side.
            assert_eq!(shoulder_sine(&points, &[1.0, s, 1.0], -Vec3::Z), 0.0);
            // And it does not depend on the weights' common factor or on
            // the size.
            let scaled: Vec<Vec3> = points.iter().map(|p| *p * 3e7).collect();
            let other = shoulder_sine(&scaled, &[1e3, 1e3 * s, 1e3], Vec3::Z);
            assert!((other / bound - 1.0).abs() < 1e-12);
        }
    }
}
