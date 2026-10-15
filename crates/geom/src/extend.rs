//! A spline surface continued past its domain as it is: the polynomial
//! (or rational function) of each end span goes on unchanged, so every
//! point of the old surface keeps its parameters and the new part is what
//! evaluating past the domain gave already. Used where an operation makes
//! a face larger than the patch it was made on (a blend that adds
//! material beyond the end of a lofted face).
//!
//! **How.** Per direction and end: the knot at the domain's end is made
//! of full multiplicity and the last inner knot of multiplicity `degree`
//! (Boehm's knot insertion, which does not change the surface), so the
//! end span is a Bézier piece; its control points are those of the same
//! piece over the longer interval (de Casteljau's subdivision at a
//! parameter past 1, on the weighted points), and the end knots move to
//! the new end.

// `!(x > y)` is deliberate: unlike `x <= y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::GeomError;
use crate::nurbs::NurbsSurface;
use craft3d_math::Vec3;

/// A weighted control point: (w x, w y, w z, w).
type W = [f64; 4];

fn lerp(a: W, b: W, t: f64) -> W {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// Inserts the knot `u` once into the spline of `degree` with `knots` and
/// the rows of weighted points `rows` (each row one spline along this
/// direction: the same insertion for all).
fn insert(degree: usize, knots: &mut Vec<f64>, rows: &mut [Vec<W>], u: f64) -> Option<()> {
    // The span: knots[k] <= u < knots[k + 1], with k the last such index
    // below the domain's end (u at the end goes into the last span).
    let n = rows.first()?.len();
    let end = *knots.get(n)?;
    let k = if u >= end {
        // Into the last nonempty span.
        (degree..n)
            .rev()
            .find(|i| knots.get(*i).is_some_and(|x| *x < end))?
    } else {
        (degree..n)
            .rev()
            .find(|i| knots.get(*i).is_some_and(|x| *x <= u))?
    };
    for row in rows.iter_mut() {
        let mut out = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let p = if i + degree <= k {
                *row.get(i)?
            } else if i > k {
                *row.get(i - 1)?
            } else {
                let (a, b) = (*knots.get(i)?, *knots.get(i + degree)?);
                let alpha = if b > a { (u - a) / (b - a) } else { 0.0 };
                lerp(*row.get(i.checked_sub(1)?)?, *row.get(i)?, alpha)
            };
            out.push(p);
        }
        *row = out;
    }
    knots.insert(k + 1, u);
    Some(())
}

/// Clamps the splines (the rows) at the high end of their domain: the end
/// knot of full multiplicity, and what lay past it dropped (an unclamped
/// tail's knots and control points shape nothing of the domain). Returns
/// the end.
fn clamp_high(degree: usize, knots: &mut Vec<f64>, rows: &mut [Vec<W>]) -> Option<f64> {
    let n = rows.first()?.len();
    let end = *knots.get(n)?;
    let multiplicity = |knots: &[f64], x: f64| knots.iter().filter(|k| **k == x).count();
    let mut guard = 0;
    while multiplicity(knots, end) < degree + 1 {
        insert(degree, knots, rows, end)?;
        guard += 1;
        if guard > degree + 1 {
            return None;
        }
    }
    let count = knots.iter().position(|k| *k == end)?;
    if count < degree + 1 {
        return None;
    }
    knots.truncate(count + degree + 1);
    for row in rows.iter_mut() {
        row.truncate(count);
    }
    Some(end)
}

/// Clamps the splines at both ends of their domain (`clamp_high`, and the
/// same on the mirrored splines).
fn clamp(degree: usize, knots: &mut Vec<f64>, rows: &mut [Vec<W>]) -> Option<()> {
    clamp_high(degree, knots, rows)?;
    let mut mirrored: Vec<f64> = knots.iter().rev().map(|k| -k).collect();
    for row in rows.iter_mut() {
        row.reverse();
    }
    clamp_high(degree, &mut mirrored, rows)?;
    for row in rows.iter_mut() {
        row.reverse();
    }
    *knots = mirrored.iter().rev().map(|k| -k).collect();
    Some(())
}

/// Continues the splines (the rows) past the high end of their domain to
/// `to`.
fn extend_high(degree: usize, knots: &mut Vec<f64>, rows: &mut [Vec<W>], to: f64) -> Option<()> {
    let n = rows.first()?.len();
    let end = *knots.get(n)?;
    if !(to > end) {
        return Some(());
    }
    // Clamped there, the end span has to be taken from the domain's own
    // last piece.
    clamp_high(degree, knots, rows)?;
    let multiplicity = |knots: &[f64], x: f64| knots.iter().filter(|k| **k == x).count();
    let n = rows.first()?.len();
    // The last inner knot, of multiplicity `degree`.
    let start = knots
        .iter()
        .copied()
        .filter(|k| *k < end)
        .fold(f64::NEG_INFINITY, f64::max);
    if !start.is_finite() {
        return None;
    }
    let mut guard = 0;
    while multiplicity(knots, start) < degree {
        insert(degree, knots, rows, start)?;
        guard += 1;
        if guard > degree {
            return None;
        }
    }
    let n2 = rows.first()?.len();
    if n2 < degree + 1 || n2 < n {
        return None;
    }
    let tau = (to - start) / (end - start);
    for row in rows.iter_mut() {
        let first = n2 - degree - 1;
        let mut level: Vec<W> = row.get(first..)?.to_vec();
        let mut left = Vec::with_capacity(degree + 1);
        left.push(*level.first()?);
        for _ in 0..degree {
            level = level
                .windows(2)
                .map(|w| match w {
                    [a, b] => lerp(*a, *b, tau),
                    _ => [0.0; 4],
                })
                .collect();
            left.push(*level.first()?);
        }
        for (slot, p) in row.get_mut(first..)?.iter_mut().zip(left) {
            *slot = p;
        }
    }
    let total = knots.len();
    for k in knots.get_mut(total - degree - 1..)? {
        *k = to;
    }
    Some(())
}

/// Continues the splines past both ends of their domain to `(lo, hi)`
/// (an end inside the domain stays where it is).
fn extend(
    degree: usize,
    knots: &mut Vec<f64>,
    rows: &mut [Vec<W>],
    (lo, hi): (f64, f64),
) -> Option<()> {
    extend_high(degree, knots, rows, hi)?;
    // The low end as the high end of the mirrored splines.
    let mut mirrored: Vec<f64> = knots.iter().rev().map(|k| -k).collect();
    for row in rows.iter_mut() {
        row.reverse();
    }
    extend_high(degree, &mut mirrored, rows, -lo)?;
    for row in rows.iter_mut() {
        row.reverse();
    }
    *knots = mirrored.iter().rev().map(|k| -k).collect();
    Some(())
}

impl NurbsSurface {
    /// Whether the surface closes on itself along u (`along_u`) or v: its
    /// two ends in that direction meet within `tol` (checked at nine
    /// places across).
    pub fn closes(&self, along_u: bool, tol: f64) -> bool {
        // Clamped at both ends in that direction, the surface's two end
        // curves are the rational curves of its first and last rows of
        // control points: the same curve when the rows' points meet within
        // `tol` and their weights agree (up to one factor for the row).
        let (du, dv) = self.degrees();
        let (count_u, count_v) = self.counts();
        let weighted: Vec<W> = self
            .control_points()
            .iter()
            .zip(self.weights())
            .map(|(p, w)| [p.x * w, p.y * w, p.z * w, *w])
            .collect();
        let at = |i: usize, j: usize| weighted.get(i * count_v + j).copied();
        let (degree, mut knots, mut lines): (usize, Vec<f64>, Vec<Vec<W>>) = if along_u {
            let lines = (0..count_v)
                .map(|j| (0..count_u).filter_map(|i| at(i, j)).collect())
                .collect();
            (du, self.knots_u().to_vec(), lines)
        } else {
            let lines = (0..count_u)
                .map(|i| (0..count_v).filter_map(|j| at(i, j)).collect())
                .collect();
            (dv, self.knots_v().to_vec(), lines)
        };
        if clamp(degree, &mut knots, &mut lines).is_none() {
            return false;
        }
        let ends: Option<Vec<(W, W)>> = lines
            .iter()
            .map(|l| Some((*l.first()?, *l.last()?)))
            .collect();
        let Some(ends) = ends else { return false };
        let Some(&(a0, b0)) = ends.first() else {
            return false;
        };
        if !(a0[3] > 0.0 && b0[3] > 0.0) {
            return false;
        }
        let factor = b0[3] / a0[3];
        ends.iter().all(|(a, b)| {
            let (wa, wb) = (a[3], b[3]);
            let pa = Vec3::new(a[0] / wa, a[1] / wa, a[2] / wa);
            let pb = Vec3::new(b[0] / wb, b[1] / wb, b[2] / wb);
            pa.distance(pb) <= tol && (wb - factor * wa).abs() <= 1e-12 * wb.abs().max(wa.abs())
        })
    }

    /// The same surface over a domain grown to take in `u` and `v` (the
    /// ends of the old domain inside them stay): every end span continued
    /// as the function it is. Refused where a weight would not stay
    /// positive (a rational span continued far), and in a direction in
    /// which the surface closes on itself (its two ends meet within `tol`:
    /// a revolved face's seam), where going on past an end means coming
    /// round over the surface again.
    pub fn extended(
        &self,
        u: (f64, f64),
        v: (f64, f64),
        tol: f64,
    ) -> Result<NurbsSurface, GeomError> {
        let refused = || GeomError::InvalidParameter("the surface cannot be continued that far");
        let (du, dv) = self.degrees();
        let (count_u, count_v) = self.counts();
        let ((u0, u1), (v0, v1)) = self.domain();
        let grows_u = u.0 < u0 || u.1 > u1;
        let grows_v = v.0 < v0 || v.1 > v1;
        if (grows_u && self.closes(true, tol)) || (grows_v && self.closes(false, tol)) {
            return Err(GeomError::InvalidParameter(
                "a surface that closes on itself cannot be continued past its seam",
            ));
        }
        let weighted: Vec<W> = self
            .control_points()
            .iter()
            .zip(self.weights())
            .map(|(p, w)| [p.x * w, p.y * w, p.z * w, *w])
            .collect();
        // Along u: the columns (fixed v index).
        let mut columns: Vec<Vec<W>> = (0..count_v)
            .map(|j| {
                (0..count_u)
                    .map(|i| weighted.get(i * count_v + j).copied().unwrap_or([0.0; 4]))
                    .collect()
            })
            .collect();
        let mut knots_u = self.knots_u().to_vec();
        extend(du, &mut knots_u, &mut columns, (u.0.min(u0), u.1.max(u1))).ok_or_else(refused)?;
        let new_u = columns.first().map_or(0, Vec::len);
        // Along v: the rows (fixed u index).
        let mut rows: Vec<Vec<W>> = (0..new_u)
            .map(|i| {
                columns
                    .iter()
                    .map(|c| c.get(i).copied().unwrap_or([0.0; 4]))
                    .collect()
            })
            .collect();
        let mut knots_v = self.knots_v().to_vec();
        extend(dv, &mut knots_v, &mut rows, (v.0.min(v0), v.1.max(v1))).ok_or_else(refused)?;
        let new_v = rows.first().map_or(0, Vec::len);
        let mut ctrl = Vec::with_capacity(new_u * new_v);
        let mut weights = Vec::with_capacity(new_u * new_v);
        for row in &rows {
            for p in row {
                let w = p[3];
                if !(w > 0.0 && w.is_finite()) {
                    return Err(refused());
                }
                ctrl.push(Vec3::new(p[0] / w, p[1] / w, p[2] / w));
                weights.push(w);
            }
        }
        NurbsSurface::new(du, dv, knots_u, knots_v, new_u, new_v, ctrl, weights)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    /// A bicubic patch of two spans by three, with weights.
    fn patch() -> NurbsSurface {
        let (nu, nv) = (5, 6);
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for i in 0..nu {
            for j in 0..nv {
                let (x, y) = (i as f64, j as f64);
                ctrl.push(Vec3::new(
                    x,
                    y,
                    (0.7 * x).sin() + 0.3 * (y * 0.9).cos() + 0.1 * x * y,
                ));
                weights.push(1.0 + 0.05 * x + 0.03 * y);
            }
        }
        NurbsSurface::new(
            3,
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.4, 1.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.7, 1.0, 1.0, 1.0, 1.0],
            nu,
            nv,
            ctrl,
            weights,
        )
        .unwrap()
    }

    #[test]
    fn a_continued_surface_is_the_same_function_inside_and_past_the_domain() {
        let s = patch();
        let e = s.extended((-0.3, 1.25), (-0.2, 1.4), 1e-9).unwrap();
        assert_eq!(e.domain(), ((-0.3, 1.25), (-0.2, 1.4)));
        let mut worst = 0.0_f64;
        for i in 0..=30 {
            for j in 0..=30 {
                let u = -0.3 + 1.55 * f64::from(i) / 30.0;
                let v = -0.2 + 1.6 * f64::from(j) / 30.0;
                // The old surface past its domain continues its end spans.
                worst = worst.max(s.eval(u, v).distance(e.eval(u, v)));
            }
        }
        assert!(worst < 1e-12, "{worst}");
        // Only one end, and nothing at all.
        let one = s.extended((0.0, 1.5), (0.0, 1.0), 1e-9).unwrap();
        assert_eq!(one.domain(), ((0.0, 1.5), (0.0, 1.0)));
        assert!(s.eval(1.4, 0.5).distance(one.eval(1.4, 0.5)) < 1e-12);
        let same = s.extended((0.2, 0.8), (0.1, 0.9), 1e-9).unwrap();
        assert_eq!(same.domain(), s.domain());
        assert!(s.eval(0.37, 0.61).distance(same.eval(0.37, 0.61)) < 1e-12);
    }

    /// Unclamped knots (uniform, the domain the middle spans) and rational
    /// weights: the continued surface is the end spans' own functions,
    /// evaluated directly past the domain; the tail knots and points
    /// past the domain shape nothing of it.
    #[test]
    fn an_unclamped_rational_surface_is_continued_from_its_last_spans() {
        // Cubic in u over knots 0..=9 (domain [3, 6]), quadratic in v
        // over 0..=7 (domain [2, 5]).
        let (nu, nv) = (6, 5);
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for i in 0..nu {
            for j in 0..nv {
                let (x, y) = (i as f64, j as f64);
                ctrl.push(Vec3::new(x, y, (0.9 * x).cos() + 0.4 * (0.7 * y).sin()));
                weights.push(1.0 + 0.1 * x + 0.05 * y + 0.02 * x * y);
            }
        }
        let s = NurbsSurface::new(
            3,
            2,
            (0..=9).map(f64::from).collect(),
            (0..=7).map(f64::from).collect(),
            nu,
            nv,
            ctrl,
            weights,
        )
        .unwrap();
        assert_eq!(s.domain(), ((3.0, 6.0), (2.0, 5.0)));
        let e = s.extended((2.5, 6.4), (1.6, 5.5), 1e-9).unwrap();
        assert_eq!(e.domain(), ((2.5, 6.4), (1.6, 5.5)));
        let mut worst = 0.0_f64;
        for i in 0..=26 {
            for j in 0..=26 {
                let u = 2.5 + 3.9 * f64::from(i) / 26.0;
                let v = 1.6 + 3.9 * f64::from(j) / 26.0;
                worst = worst.max(s.eval(u, v).distance(e.eval(u, v)));
            }
        }
        assert!(worst < 1e-11, "{worst}");
    }

    /// A surface whose u ends meet (a revolved face's seam) is not grown
    /// along u; along v it is.
    #[test]
    fn a_surface_that_closes_on_itself_is_not_continued_past_its_seam() {
        let mut ctrl = Vec::new();
        for i in 0..4 {
            for j in 0..3 {
                // The first and the last row coincide.
                let a = if i == 3 { 0.0 } else { f64::from(i) };
                ctrl.push(Vec3::new(a.cos(), a.sin(), f64::from(j)));
            }
        }
        let s = NurbsSurface::new(
            3,
            2,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            4,
            3,
            ctrl,
            vec![1.0; 12],
        )
        .unwrap();
        assert!(s.extended((0.0, 1.2), (0.0, 1.0), 1e-9).is_err());
        assert!(s.extended((0.0, 1.0), (0.0, 1.3), 1e-9).is_ok());
    }
}
