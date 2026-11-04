//! B-splines for the solver: what a point-on-spline constraint needs.
//!
//! A spline is not an unknown of its own: its shape is that of its control
//! points, which are points of the sketch. A point `P` is on the spline
//! when its distance from the curve is zero, and that distance is taken at
//! the *foot* of `P`: the point `C(u)` of the curve nearest to it. There
//! `P − C` is at right angles to the curve, so the signed distance
//! `g = (P − C(u)) · n(u)` (`n` the unit normal to the left of the curve)
//! does not change, to first order, when `u` or the direction of `n`
//! does. Its derivatives are therefore those with `u` and `n` held where
//! they are: `n` with respect to `P`, and `−N_i(u) n` with respect to
//! control point `i`, with `N_i` the basis functions. One equation, no
//! extra unknown: the curve parameter is eliminated by the projection.
//!
//! Where the curve has a corner and `P` is nearest to the corner itself,
//! nothing is at right angles; the distance is then the plain distance to
//! that point of the curve, `|P − C(u)|`, whose derivatives are the same
//! with the direction from the corner to `P` in the place of `n`.
//!
//! Beyond its ends the curve goes on straight, along its first and its
//! last leg: a point whose foot would lie before the start or after the
//! end is held on that line (as a point on a line is held on the whole
//! line, not on the stretch between its ends).

use crate::dual::SLOTS;
use crate::equations::Pt;
use craft3d_math::Portable;

/// The highest degree of a spline a point can be put on: the equation
/// touches the point and `degree + 1` control points, two unknowns each.
pub(crate) const MAX_SPLINE_DEGREE: usize = SLOTS / 2 - 2;

/// How many places per knot span are looked at for the nearest.
const SAMPLES: u32 = 8;

/// A spline of the sketch: its control points as unknowns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SplineVars {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub control: Vec<Pt>,
}

/// Why the numbers of a spline are not those of one; `None` when they are.
pub(crate) fn fault(degree: usize, knots: &[f64], count: usize) -> Option<&'static str> {
    if degree == 0 || degree > MAX_SPLINE_DEGREE {
        return Some("the degree of the spline is zero or too high");
    }
    if count <= degree {
        return Some("the spline has fewer control points than its degree plus one");
    }
    if knots.len() != count.saturating_add(degree).saturating_add(1) {
        return Some("the spline does not have one knot per control point plus degree plus one");
    }
    if !knots.iter().all(|k| k.is_finite()) {
        return Some("a knot of the spline is not finite");
    }
    if knots.windows(2).any(|w| matches!(w, [a, b] if a > b)) {
        return Some("the knots of the spline go down");
    }
    match (
        knots.first(),
        knots.get(degree),
        knots.get(count),
        knots.last(),
    ) {
        (Some(first), Some(start), Some(end), Some(last))
            if first == start && end == last && start < end =>
        {
            None
        }
        _ => Some("the spline is not clamped: it does not end on its end control points"),
    }
}

/// The weights of the `order`th derivative of a clamped spline of
/// `degree` on `knots` and `count` control points at its start (on its
/// first `order + 1` control points) and at its end (on its last
/// `order + 1`), from the control points of its derivatives (*The NURBS
/// Book*, eq. 3.8). `None` for an order of 0 or of at least `count`, or
/// knots that do not fit.
pub(crate) fn seam_weights(
    knots: &[f64],
    degree: usize,
    count: usize,
    order: usize,
) -> Option<(Vec<f64>, Vec<f64>)> {
    if order == 0 || order >= count || order > degree {
        return None;
    }
    let weights = |from: usize| -> Option<Vec<f64>> {
        // Row j: the weights of derivative control point `from + j` of the
        // current order, on control points `from ..= from + order`.
        let mut rows: Vec<Vec<f64>> = (0..=order)
            .map(|j| {
                let mut row = vec![0.0; order + 1];
                if let Some(slot) = row.get_mut(j) {
                    *slot = 1.0;
                }
                row
            })
            .collect();
        for k in 1..=order {
            let mut next = Vec::with_capacity(rows.len().saturating_sub(1));
            for j in 0..=order - k {
                let i = from + j;
                let width = knots.get(i + degree + 1)? - knots.get(i + k)?;
                if !(width.is_finite() && width > 0.0) {
                    return None;
                }
                let factor = (degree + 1 - k) as f64 / width;
                let (a, b) = (rows.get(j + 1)?, rows.get(j)?);
                next.push(a.iter().zip(b).map(|(a, b)| factor * (a - b)).collect());
            }
            rows = next;
        }
        rows.into_iter().next()
    };
    Some((weights(0)?, weights(count.checked_sub(order + 1)?)?))
}

/// Where the curve is at one parameter.
#[derive(Clone, Copy, Debug)]
pub(crate) struct At {
    /// The first control point that counts here.
    pub first: usize,
    /// The basis functions of `first ..= first + degree`.
    pub basis: [f64; MAX_SPLINE_DEGREE + 1],
    /// Their derivatives with respect to the parameter.
    pub slopes: [f64; MAX_SPLINE_DEGREE + 1],
    pub point: [f64; 2],
    /// The derivative with respect to the parameter.
    pub tangent: [f64; 2],
}

/// The foot of a point on a spline.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Foot {
    /// Before the start: on the line of the first leg.
    BeforeStart,
    /// After the end: on the line of the last leg.
    AfterEnd,
    On(At),
    /// On the curve where it has a corner (a polyline's, or a knot as
    /// often as the degree) that is nearer than everything around it: the
    /// point is not straight across from the curve there, to either side.
    Corner(At),
}

/// The basis functions of degree `d` that are not zero in the span that
/// starts at knot `k`, at `u`: those of the control points `k − d ..= k`
/// (*The NURBS Book*, algorithm A2.2).
fn basis(knots: &[f64], k: usize, d: usize, u: f64) -> Option<[f64; MAX_SPLINE_DEGREE + 1]> {
    let mut values = [0.0; MAX_SPLINE_DEGREE + 1];
    let (mut left, mut right) = ([0.0; MAX_SPLINE_DEGREE + 1], [0.0; MAX_SPLINE_DEGREE + 1]);
    *values.first_mut()? = 1.0;
    for j in 1..=d {
        *left.get_mut(j)? = u - knots.get((k + 1).checked_sub(j)?)?;
        *right.get_mut(j)? = knots.get(k + j)? - u;
        let mut saved = 0.0;
        for r in 0..j {
            let (above, below) = (*right.get(r + 1)?, *left.get(j - r)?);
            let width = above + below;
            let share = if width > 0.0 {
                values.get(r)? / width
            } else {
                0.0
            };
            *values.get_mut(r)? = saved + above * share;
            saved = below * share;
        }
        *values.get_mut(j)? = saved;
    }
    Some(values)
}

impl SplineVars {
    fn domain(&self) -> Option<(f64, f64)> {
        Some((
            *self.knots.get(self.degree)?,
            *self.knots.get(self.control.len())?,
        ))
    }

    /// The curve at `u`, with `points` the places of its control points.
    pub(crate) fn at(&self, points: &[[f64; 2]], u: f64) -> Option<At> {
        let (p, n) = (self.degree, self.control.len());
        let (start, end) = self.domain()?;
        let u = u.clamp(start, end);
        // The span: the last knot not above `u` that starts one.
        let mut k = p;
        for candidate in p..n {
            let (low, high) = (*self.knots.get(candidate)?, *self.knots.get(candidate + 1)?);
            if low < high && low <= u {
                k = candidate;
            }
        }
        let values = basis(&self.knots, k, p, u)?;
        let lower = basis(&self.knots, k, p - 1, u)?;
        let first = k - p;
        // Degrees are at most seven, exact as f64.
        #[allow(clippy::cast_precision_loss)]
        let degree = p as f64;
        let (mut point, mut tangent) = ([0.0; 2], [0.0; 2]);
        let mut slopes = [0.0; MAX_SPLINE_DEGREE + 1];
        for r in 0..=p {
            let i = first + r;
            let control = points.get(i)?;
            // N'_i = p (N_{i,p−1} / (u_{i+p} − u_i) − N_{i+1,p−1} / (u_{i+p+1} − u_{i+1})),
            // where `lower[j]` is N_{first+1+j, p−1}.
            let share = |index: usize, value: Option<&f64>| -> Option<f64> {
                let width = self.knots.get(index + p)? - self.knots.get(index)?;
                Some(match value {
                    Some(v) if width > 0.0 => v / width,
                    _ => 0.0,
                })
            };
            let before = share(i, r.checked_sub(1).and_then(|j| lower.get(j)))?;
            let after = share(i + 1, if r < p { lower.get(r) } else { None })?;
            let slope = degree * (before - after);
            *slopes.get_mut(r)? = slope;
            let weight = *values.get(r)?;
            for axis in 0..2 {
                *point.get_mut(axis)? += weight * control.get(axis)?;
                *tangent.get_mut(axis)? += slope * control.get(axis)?;
            }
        }
        Some(At {
            first,
            basis: values,
            slopes,
            point,
            tangent,
        })
    }

    /// The parameter of an end of the curve.
    pub(crate) fn end(&self, end: crate::model::SplineEnd) -> Option<f64> {
        let (start, stop) = self.domain()?;
        Some(match end {
            crate::model::SplineEnd::Start => start,
            crate::model::SplineEnd::End => stop,
        })
    }

    /// Whether `u` lies in the curve's domain.
    pub(crate) fn covers(&self, u: f64) -> bool {
        self.domain()
            .is_some_and(|(start, end)| u.is_finite() && start <= u && u <= end)
    }

    /// The places [`SplineVars::foot`] and [`SplineVars::parallel`] look
    /// at: the start, then [`SAMPLES`] per knot span.
    fn places(&self) -> Option<Vec<f64>> {
        let (p, n) = (self.degree, self.control.len());
        let (start, _) = self.domain()?;
        let mut places = vec![start];
        for k in p..n {
            let (low, high) = (*self.knots.get(k)?, *self.knots.get(k + 1)?);
            if low < high {
                for step in 1..=SAMPLES {
                    places.push(low + (high - low) * f64::from(step) / f64::from(SAMPLES));
                }
            }
        }
        Some(places)
    }

    /// Where the curve runs parallel to the direction `along`, nearest to
    /// the line through `origin` along it: of the places where the cross
    /// product of `along` and the curve's tangent changes sign between two
    /// of the places looked at (or is zero at one), each found by
    /// bisection, the one at the least distance from that line. A corner
    /// of the curve where the tangent jumps across `along` is such a place
    /// too (a line touching a polyline at a corner). `None` when there is
    /// none: the curve never runs along `along`. Every loop is bounded.
    pub(crate) fn parallel(
        &self,
        points: &[[f64; 2]],
        along: [f64; 2],
        origin: [f64; 2],
    ) -> Option<Option<At>> {
        let turn = |u: f64| -> Option<f64> {
            let at = self.at(points, u)?;
            let value = along[0] * at.tangent[1] - along[1] * at.tangent[0];
            value.is_finite().then_some(value)
        };
        let off = |at: &At| {
            (along[0] * (at.point[1] - origin[1]) - along[1] * (at.point[0] - origin[0])).abs()
        };
        let places = self.places()?;
        let mut best: Option<(f64, At)> = None;
        let mut consider = |at: At| {
            let d = off(&at);
            if d.is_finite() && best.as_ref().is_none_or(|(least, _)| d < *least) {
                best = Some((d, at));
            }
        };
        let mut before: Option<(f64, f64)> = None;
        for &u in &places {
            let value = turn(u)?;
            if value == 0.0 {
                consider(self.at(points, u)?);
            } else if let Some((low_u, low)) = before
                && low != 0.0
                && (low < 0.0) != (value < 0.0)
            {
                let (mut a, mut b) = (low_u, u);
                for _ in 0..80 {
                    let middle = a + (b - a) / 2.0;
                    if !(middle > a && middle < b) {
                        break;
                    }
                    if (turn(middle)? < 0.0) == (low < 0.0) {
                        a = middle;
                    } else {
                        b = middle;
                    }
                }
                consider(self.at(points, a + (b - a) / 2.0)?);
            }
            before = Some((u, value));
        }
        Some(best.map(|(_, at)| at))
    }

    /// The foot of `target` on the curve: the nearest of the places looked
    /// at, [`SAMPLES`] per knot span, and from there the place between its
    /// neighbours where the curve stops coming nearer (by bisection: every
    /// loop here is bounded). `None` when the numbers are not finite.
    pub(crate) fn foot(&self, points: &[[f64; 2]], target: [f64; 2]) -> Option<Foot> {
        // How fast the distance grows along the curve: negative while the
        // curve still comes nearer.
        let approach = |u: f64| -> Option<f64> {
            let at = self.at(points, u)?;
            let value = (at.point[0] - target[0]) * at.tangent[0]
                + (at.point[1] - target[1]) * at.tangent[1];
            value.is_finite().then_some(value)
        };
        let places = self.places()?;
        let mut best: Option<(usize, f64)> = None;
        for (index, &u) in places.iter().enumerate() {
            let at = self.at(points, u)?;
            let distance = (at.point[0] - target[0]).phypot(at.point[1] - target[1]);
            if !distance.is_finite() {
                return None;
            }
            if best.is_none_or(|(_, least)| distance < least) {
                best = Some((index, distance));
            }
        }
        let (index, _) = best?;
        let here = *places.get(index)?;
        let before = index.checked_sub(1).and_then(|i| places.get(i)).copied();
        let after = places.get(index + 1).copied();
        // At an end with the curve still coming nearer beyond it.
        if before.is_none() && approach(here)? > 0.0 {
            return Some(Foot::BeforeStart);
        }
        if after.is_none() && approach(here)? < 0.0 {
            return Some(Foot::AfterEnd);
        }
        let (mut low, mut high) = (before.unwrap_or(here), after.unwrap_or(here));
        if approach(low)? > 0.0 || approach(high)? < 0.0 {
            // Not one dip between the neighbours: the place looked at.
            return self.at(points, here).map(Foot::On);
        }
        for _ in 0..80 {
            let middle = low + (high - low) / 2.0;
            if !(middle > low && middle < high) {
                break;
            }
            if approach(middle)? < 0.0 {
                low = middle;
            } else {
                high = middle;
            }
        }
        let at = self.at(points, low + (high - low) / 2.0)?;
        // Straight across from the curve, unless the bisection closed in
        // on a corner, where the curve turns from coming nearer to going
        // away without ever running across.
        let away = [target[0] - at.point[0], target[1] - at.point[1]];
        let along = away[0] * at.tangent[0] + away[1] * at.tangent[1];
        let size = away[0].phypot(away[1]) * at.tangent[0].phypot(at.tangent[1]);
        Some(if along.abs() > 1e-6 * size {
            Foot::Corner(at)
        } else {
            Foot::On(at)
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn uniform(degree: usize, count: usize) -> SplineVars {
        let spans = count - degree;
        let mut knots = vec![0.0; degree + 1];
        knots.extend((1..spans).map(|i| i as f64 / spans as f64));
        knots.extend(vec![1.0; degree + 1]);
        SplineVars {
            degree,
            knots,
            control: (0..count).map(|i| Pt::Own(2 * i)).collect(),
        }
    }

    const WAVE: [[f64; 2]; 7] = [
        [0.0, 0.0],
        [1.0, 2.0],
        [2.5, -1.0],
        [4.0, 3.0],
        [5.0, 0.5],
        [6.5, 1.0],
        [8.0, 0.0],
    ];

    /// De Boor's algorithm, which shares nothing with `at`.
    fn de_boor(spline: &SplineVars, points: &[[f64; 2]], u: f64) -> [f64; 2] {
        let (p, n) = (spline.degree, points.len());
        let k = (p..n)
            .rev()
            .find(|&k| spline.knots[k] <= u)
            .unwrap_or(p)
            .min(n - 1);
        let mut d = points[k - p..=k].to_vec();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = k - p + j;
                let alpha = (u - spline.knots[i]) / (spline.knots[i + p + 1 - r] - spline.knots[i]);
                let before = d[j - 1];
                for (here, before) in d[j].iter_mut().zip(before) {
                    *here = before + (*here - before) * alpha;
                }
            }
        }
        d[p]
    }

    #[test]
    fn the_curve_and_its_tangent_are_those_of_de_boor() {
        for degree in 1..=MAX_SPLINE_DEGREE.min(6) {
            let spline = uniform(degree, WAVE.len());
            assert_eq!(fault(degree, &spline.knots, WAVE.len()), None);
            for i in 0..=200 {
                let u = f64::from(i) / 200.0;
                let at = spline.at(&WAVE, u).unwrap();
                let theirs = de_boor(&spline, &WAVE, u.min(1.0 - 1e-15));
                assert!(
                    (at.point[0] - theirs[0]).phypot(at.point[1] - theirs[1]) < 1e-13,
                    "degree {degree} at {u}"
                );
                let total: f64 = at.basis.iter().sum();
                assert!((total - 1.0).abs() < 1e-14);
                // The tangent by central differences, away from the knots
                // of a polyline, where it jumps.
                let h = 1e-6;
                if degree > 1 && u > h && u < 1.0 - h {
                    let (a, b) = (
                        de_boor(&spline, &WAVE, u - h),
                        de_boor(&spline, &WAVE, u + h),
                    );
                    for axis in 0..2 {
                        let slope = (b[axis] - a[axis]) / (2.0 * h);
                        assert!(
                            (at.tangent[axis] - slope).abs() < 1e-5 * (1.0 + slope.abs()),
                            "degree {degree} at {u}: {} against {slope}",
                            at.tangent[axis]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_foot_is_the_nearest_point_of_the_curve() {
        for degree in 2..=5 {
            let spline = uniform(degree, WAVE.len());
            for target in [[2.0, 3.0], [4.0, -2.0], [6.0, 2.5], [3.1, 0.7], [7.5, 0.4]] {
                let Foot::On(at) = spline.foot(&WAVE, target).unwrap() else {
                    panic!("degree {degree} {target:?}: the foot is on the curve");
                };
                let distance = (at.point[0] - target[0]).phypot(at.point[1] - target[1]);
                // No finer look finds a nearer point.
                let mut least = f64::INFINITY;
                for i in 0..=20_000 {
                    let u = f64::from(i) / 20_000.0;
                    let p = de_boor(&spline, &WAVE, u.min(1.0 - 1e-15));
                    least = least.min((p[0] - target[0]).phypot(p[1] - target[1]));
                }
                assert!(distance <= least + 1e-9, "degree {degree} {target:?}");
                // At right angles to the curve there.
                let across = (target[0] - at.point[0]) * at.tangent[0]
                    + (target[1] - at.point[1]) * at.tangent[1];
                assert!(
                    across.abs() < 1e-9 * (1.0 + distance),
                    "degree {degree} {target:?}"
                );
            }
            // Beyond the ends.
            assert!(matches!(
                spline.foot(&WAVE, [-1.0, -2.5]),
                Some(Foot::BeforeStart)
            ));
            assert!(matches!(
                spline.foot(&WAVE, [9.0, -0.5]),
                Some(Foot::AfterEnd)
            ));
            assert!(spline.foot(&WAVE, [f64::NAN, 0.0]).is_none());
        }
    }

    #[test]
    fn numbers_that_are_not_a_spline_say_why() {
        let good = uniform(3, 7);
        assert_eq!(fault(3, &good.knots, 7), None);
        assert!(fault(0, &good.knots, 7).is_some());
        assert!(fault(MAX_SPLINE_DEGREE + 1, &good.knots, 7).is_some());
        assert!(fault(3, &good.knots, 3).is_some());
        assert!(fault(3, &good.knots[1..], 7).is_some());
        let mut knots = good.knots.clone();
        knots[5] = f64::NAN;
        assert!(fault(3, &knots, 7).is_some());
        knots[5] = 0.1;
        assert!(fault(3, &knots, 7).is_some());
        knots = good.knots.clone();
        knots[0] = -1.0;
        assert!(fault(3, &knots, 7).is_some());
        assert!(fault(3, &[0.5; 11], 7).is_some());
    }
}
