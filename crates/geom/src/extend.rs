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

/// Continues the splines (the rows) past the high end of their domain to
/// `to`.
fn extend_high(degree: usize, knots: &mut Vec<f64>, rows: &mut [Vec<W>], to: f64) -> Option<()> {
    let n = rows.first()?.len();
    let end = *knots.get(n)?;
    if !(to > end) {
        return Some(());
    }
    // The end knot of full multiplicity: degree + 1 copies from index n.
    let multiplicity = |knots: &[f64], x: f64| knots.iter().filter(|k| **k == x).count();
    let mut guard = 0;
    while multiplicity(knots, end) < degree + 1 {
        insert(degree, knots, rows, end)?;
        guard += 1;
        if guard > degree + 1 {
            return None;
        }
    }
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
    /// The same surface over a domain grown to take in `u` and `v` (the
    /// ends of the old domain inside them stay): every end span continued
    /// as the function it is. Refused where a weight would not stay
    /// positive (a rational span continued far).
    pub fn extended(&self, u: (f64, f64), v: (f64, f64)) -> Result<NurbsSurface, GeomError> {
        let refused = || GeomError::InvalidParameter("the surface cannot be continued that far");
        let (du, dv) = self.degrees();
        let (count_u, count_v) = self.counts();
        let ((u0, u1), (v0, v1)) = self.domain();
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
        let e = s.extended((-0.3, 1.25), (-0.2, 1.4)).unwrap();
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
        let one = s.extended((0.0, 1.5), (0.0, 1.0)).unwrap();
        assert_eq!(one.domain(), ((0.0, 1.5), (0.0, 1.0)));
        assert!(s.eval(1.4, 0.5).distance(one.eval(1.4, 0.5)) < 1e-12);
        let same = s.extended((0.2, 0.8), (0.1, 0.9)).unwrap();
        assert_eq!(same.domain(), s.domain());
        assert!(s.eval(0.37, 0.61).distance(same.eval(0.37, 0.61)) < 1e-12);
    }
}
