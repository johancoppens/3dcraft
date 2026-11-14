//! The proof that a tip's faces are seen from the tip point from one
//! side: a polynomial of two variables shown positive over the unit
//! square on the convex hull of its Bernstein coefficients.
//!
//! A polynomial in Bernstein form lies between its least and its largest
//! coefficient, and its four corner coefficients are its values at the
//! corners. So a piece whose coefficients are all above the floor is
//! proven, a piece with a corner below it is refuted, and any other piece
//! is halved (de Casteljau's algorithm: the same polynomial over half the
//! range, its coefficients closer to it). Reference: Farin, *Curves and
//! Surfaces for CAGD*, 5th ed., ch. 5 (subdivision, the convex hull
//! property) and ch. 6 (products of Bernstein forms).

use craft3d_math::Vec3;
use std::ops::{Add, Mul};

/// How often one span is halved before the proof gives up.
const MAX_HALVINGS: u32 = 64;
/// The most pieces of one span that are looked at before the proof gives
/// up.
const MAX_PIECES: usize = 1 << 13;

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

/// The longest coefficient of the first line along u (`v = 0`), or of the
/// last (`v = 1`).
fn longest_at(grid: &Grid<Vec3>, last: bool) -> f64 {
    grid.c
        .iter()
        .skip(if last { grid.dv } else { 0 })
        .step_by(grid.dv + 1)
        .fold(0.0_f64, |m, v| m.max(v.length()))
}

/// For one Bézier span of a tip face: `det[Y, ∂Y/∂u, ∂Y/∂v]`, and the
/// product of the three factors' largest lengths along the rim (`v = 0`)
/// and along the row before the tip (`v = 1`): no value of the
/// determinant there is larger, so it is measured against them.
///
/// `rows[j]` are the span's control vectors of row `j` of the face
/// without its last row (the tip): the homogeneous control points taken
/// about the tip, `w (R − P)`. With `q` the degree across, the face about
/// the tip is `(1 − v) Y(u, v)`, `Y` of degree `q − 1` across with the
/// rows times `q / (q − j)`: the factor `1 − v` is the face closing into
/// the tip and takes no part in which way the face is seen. The
/// determinant is positive where the ray from the tip crosses the face
/// from one side, negative from the other, and zero where it touches.
pub(super) fn seen_from_tip(rows: &[Vec<Vec3>]) -> Option<(Grid<f64>, [f64; 2])> {
    let q = rows.len();
    let count = rows.first()?.len();
    if q < 2 || count < 2 || rows.iter().any(|row| row.len() != count) {
        return None;
    }
    let (du, dv) = (count - 1, q - 1);
    let mut y = Grid {
        du,
        dv,
        c: Vec::with_capacity(count * q),
    };
    for i in 0..count {
        for (j, row) in rows.iter().enumerate() {
            let raise = q as f64 / (q - j) as f64;
            y.c.push(*row.get(i)? * raise);
        }
    }
    let at = |i: usize, j: usize| y.c.get(i * q + j).copied();
    let mut along = Grid {
        du: du - 1,
        dv,
        c: Vec::with_capacity(du * q),
    };
    for i in 0..du {
        for j in 0..q {
            along.c.push((at(i + 1, j)? - at(i, j)?) * du as f64);
        }
    }
    let mut across = Grid {
        du,
        dv: dv - 1,
        c: Vec::with_capacity(count * dv),
    };
    for i in 0..count {
        for j in 0..dv {
            across.c.push((at(i, j + 1)? - at(i, j)?) * dv as f64);
        }
    }
    let normal = product(&along, &across, Vec3::ZERO, Vec3::cross);
    let seen = product(&y, &normal, 0.0, Vec3::dot);
    let sizes = [false, true]
        .map(|last| longest_at(&y, last) * longest_at(&along, last) * longest_at(&across, last));
    (sizes.iter().all(|s| s.is_finite() && *s > 0.0) && seen.c.iter().all(|x| x.is_finite()))
        .then_some((seen, sizes))
}

/// The polynomial less a floor that runs straight from `floors[0]` along
/// `v = 0` to `floors[1]` along `v = 1`: what is left is positive where
/// the polynomial is above the floor.
pub(super) fn above(grid: &Grid<f64>, floors: [f64; 2]) -> Grid<f64> {
    let [first, last] = floors;
    let mut out = grid.clone();
    for (index, x) in out.c.iter_mut().enumerate() {
        let share = if grid.dv == 0 {
            1.0
        } else {
            (index % (grid.dv + 1)) as f64 / grid.dv as f64
        };
        *x -= first * (1.0 - share) + last * share;
    }
    out
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

/// What the hulls say about a polynomial against a floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Verdict {
    /// Everywhere at least `least`, which is at least the floor.
    Proven { least: f64 },
    /// Below the floor at `(u, v)`, where it is `value`.
    Refuted { u: f64, v: f64, value: f64 },
    /// Neither after every halving allowed: around `(u, v)` the
    /// polynomial comes too near the floor to tell.
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
    grid: Grid<f64>,
    u: (f64, f64),
    v: (f64, f64),
    halvings: u32,
}

/// Whether the polynomial is at least `floor` over the whole unit square.
pub(super) fn at_least(grid: &Grid<f64>, floor: f64) -> Verdict {
    let mut stack = vec![Piece {
        grid: grid.clone(),
        u: (0.0, 1.0),
        v: (0.0, 1.0),
        halvings: 0,
    }];
    let mut least = f64::INFINITY;
    let mut pieces = 0_usize;
    while let Some(piece) = stack.pop() {
        pieces += 1;
        let Piece {
            grid,
            u,
            v,
            halvings,
        } = piece;
        let middle = (0.5 * (u.0 + u.1), 0.5 * (v.0 + v.1));
        let undecided = Verdict::Undecided {
            u: middle.0,
            v: middle.1,
        };
        let lowest = grid.c.iter().copied().fold(f64::INFINITY, f64::min);
        if !floor.is_finite() || grid.c.iter().any(|x| !x.is_finite()) {
            return undecided;
        }
        if lowest >= floor {
            least = least.min(lowest);
            continue;
        }
        // The corners' coefficients are the polynomial's values there.
        let (nu, nv) = (grid.du + 1, grid.dv + 1);
        let corners = [
            (0, u.0, v.0),
            (nv - 1, u.0, v.1),
            ((nu - 1) * nv, u.1, v.0),
            (nu * nv - 1, u.1, v.1),
        ];
        for (index, at_u, at_v) in corners {
            if let Some(&value) = grid.c.get(index)
                && value < floor
            {
                return Verdict::Refuted {
                    u: at_u,
                    v: at_v,
                    value,
                };
            }
        }
        if halvings >= MAX_HALVINGS || pieces >= MAX_PIECES {
            return undecided;
        }
        let along_u = grid.dv == 0 || (grid.du > 0 && spread(&grid, true) >= spread(&grid, false));
        let (low, high) = halve(&grid, along_u);
        let (first, second) = if along_u {
            (((u.0, middle.0), v), ((middle.0, u.1), v))
        } else {
            ((u, (v.0, middle.1)), (u, (middle.1, v.1)))
        };
        stack.push(Piece {
            grid: high,
            u: second.0,
            v: second.1,
            halvings: halvings + 1,
        });
        stack.push(Piece {
            grid: low,
            u: first.0,
            v: first.1,
            halvings: halvings + 1,
        });
    }
    Verdict::Proven { least }
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

    #[test]
    fn a_parabola_is_proven_refuted_or_left_open_by_its_least_value() {
        // (t − 1/3)² + c, in Bernstein form: least value c at t = 1/3.
        let parabola = |c: f64| line(&[1.0 / 9.0 + c, -2.0 / 9.0 + c, 4.0 / 9.0 + c]);
        // The hull of the whole range dips below zero; halving proves it.
        match at_least(&parabola(1e-6), 0.0) {
            Verdict::Proven { least } => assert!((0.0..=1e-6).contains(&least), "{least}"),
            other => panic!("{other:?}"),
        }
        // Below zero around its least: a corner of some piece shows it.
        match at_least(&parabola(-1e-6), 0.0) {
            Verdict::Refuted { u, value, .. } => {
                assert!(value < 0.0 && (u - 1.0 / 3.0).abs() < 2e-3, "{u} {value}");
            }
            other => panic!("{other:?}"),
        }
        // Touching zero at a place no halving reaches: never decided.
        assert!(matches!(
            at_least(&parabola(0.0), 0.0),
            Verdict::Undecided { .. }
        ));
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
        assert!(matches!(at_least(&grid(1e-4), 0.0), Verdict::Proven { .. }));
        match at_least(&grid(-1e-4), 0.0) {
            Verdict::Refuted { u, v, .. } => {
                assert!((u - 0.3).abs() < 0.02 && (v - 0.7).abs() < 0.02, "{u} {v}");
            }
            other => panic!("{other:?}"),
        }
        let edge = last_across(&grid(0.0));
        assert_eq!(edge.c.len(), 3);
        assert!((edge.c[0] - 0.18).abs() < 1e-15);
    }
}
