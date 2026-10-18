//! Where a freeform surface closes on itself.
//!
//! `geom` gives a Bézier or NURBS surface no period: its parameters run
//! over a rectangle. A surface whose control net ends on the row it
//! starts with (a NURBS cylinder, a vase or a torus as a STEP file brings
//! them) is closed all the same, and whatever follows a curve over it
//! must go over the seam: the surface intersections walk it as a periodic
//! chart, and the booleans chart its faces by an angle around.

use crate::Surface;
use craft3d_math::{Bbox3, Vec3};

/// How near, as a share of the size of the control net, two rows of
/// control points must be to count as the same row.
const SAME_ROW: f64 = 1e-9;

/// Whether a Bézier or NURBS surface closes on itself in u and in v: its
/// control net ends there on the row it starts with (the same points to
/// [`SAME_ROW`] of the net's size, and weights in one proportion, so that
/// the two rows are the same curve) on clamped knots, and the rows in
/// between are not all that one row (which is a pole, not a seam).
/// `[false, false]` for the analytic kinds, which say how they close
/// themselves (`Surface::period_u`).
///
/// The test the booleans, the surface intersections and `craft3d_topo`'s
/// measure (`spline_closure`) take; the tessellator keeps its own
/// (`net_closes`), written over the control net alone.
pub fn closes(surface: &Surface) -> [bool; 2] {
    let (points, weights, (count_u, count_v), clamped): (&[Vec3], Option<&[f64]>, _, [bool; 2]) =
        match surface {
            Surface::Bezier(patch) => (
                patch.control_points(),
                None,
                (patch.degree_u() + 1, patch.degree_v() + 1),
                [true, true],
            ),
            Surface::Nurbs(s) => {
                let (degree_u, degree_v) = s.degrees();
                (
                    s.control_points(),
                    Some(s.weights()),
                    s.counts(),
                    [
                        clamped(s.knots_u(), degree_u),
                        clamped(s.knots_v(), degree_v),
                    ],
                )
            }
            _ => return [false, false],
        };
    if count_u < 2 || count_v < 2 || count_u.checked_mul(count_v) != Some(points.len()) {
        return [false, false];
    }
    let near = SAME_ROW * Bbox3::from_points(points.iter().copied()).diagonal();
    if !(near.is_finite() && near > 0.0) {
        return [false, false];
    }
    let at = |i: usize, j: usize| {
        let index = i.checked_mul(count_v)?.checked_add(j)?;
        let weight = match weights {
            Some(weights) => *weights.get(index)?,
            None => 1.0,
        };
        Some((*points.get(index)?, weight))
    };
    // Along `rows` rows of `across` points each, the k-th of row r being
    // `get(r, k)`.
    let closes = |rows: usize, across: usize, get: &dyn Fn(usize, usize) -> Option<(Vec3, f64)>| {
        let (Some((_, first)), Some((_, last))) = (get(0, 0), get(rows - 1, 0)) else {
            return false;
        };
        let mut moves = false;
        for k in 0..across {
            let (Some((p, w)), Some((q, x))) = (get(0, k), get(rows - 1, k)) else {
                return false;
            };
            // `w / first == x / last`, without dividing. Not `>`: a NaN
            // closes nothing.
            if !(p.distance(q) <= near
                && (w * last - x * first).abs() <= SAME_ROW * (w * last).abs())
            {
                return false;
            }
            for row in 1..rows - 1 {
                moves |= get(row, k).is_some_and(|(r, _)| r.distance(p) > near);
            }
        }
        moves
    };
    let ((u0, u1), (v0, v1)) = surface.domain();
    let wide = |low: f64, high: f64| low.is_finite() && high.is_finite() && high > low;
    let [clamped_u, clamped_v] = clamped;
    [
        wide(u0, u1) && clamped_u && closes(count_u, count_v, &at),
        wide(v0, v1) && clamped_v && closes(count_v, count_u, &|j, i| at(i, j)),
    ]
}

/// Whether a knot vector is clamped: its first and its last `degree + 1`
/// knots are one value each, so that the spline starts on its first row
/// of control points and ends on its last.
pub fn clamped(knots: &[f64], degree: usize) -> bool {
    let (Some(first), Some(last)) = (knots.first(), knots.last()) else {
        return false;
    };
    knots.len() > 2 * degree + 1
        && knots.iter().take(degree + 1).all(|k| k == first)
        && knots.iter().rev().take(degree + 1).all(|k| k == last)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::{BezierPatch, NurbsSurface};

    /// A ring of nine control points around the z axis, rational
    /// quadratic quarters, at the radius and height given.
    fn ring(radius: f64, height: f64, from: f64) -> Vec<(Vec3, f64)> {
        let weight = std::f64::consts::FRAC_PI_4.cos();
        (0..9)
            .map(|k| {
                let (sin, cos) = (from + std::f64::consts::FRAC_PI_4 * f64::from(k)).sin_cos();
                if k % 2 == 0 {
                    (Vec3::new(radius * cos, radius * sin, height), 1.0)
                } else {
                    (
                        Vec3::new(radius * cos / weight, radius * sin / weight, height),
                        weight,
                    )
                }
            })
            .collect()
    }

    fn turned(rows: &[Vec<(Vec3, f64)>], around_v: bool) -> Surface {
        let ring_knots = vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ];
        let line = vec![0.0, 0.0, 1.0, 1.0];
        let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
        if around_v {
            for row in rows {
                for &(p, w) in row {
                    ctrl.push(p);
                    weights.push(w);
                }
            }
            Surface::Nurbs(NurbsSurface::new(1, 2, line, ring_knots, 2, 9, ctrl, weights).unwrap())
        } else {
            for k in 0..9 {
                for row in rows {
                    ctrl.push(row[k].0);
                    weights.push(row[k].1);
                }
            }
            Surface::Nurbs(NurbsSurface::new(2, 1, ring_knots, line, 9, 2, ctrl, weights).unwrap())
        }
    }

    #[test]
    fn a_turned_net_closes_the_way_it_turns_and_a_pole_does_not() {
        for from in [0.0, 0.4, 3.0] {
            let wall = [ring(2.0, 0.0, from), ring(2.0, 3.0, from)];
            assert_eq!(closes(&turned(&wall, false)), [true, false]);
            assert_eq!(closes(&turned(&wall, true)), [false, true]);
            // Three quarters of a turn do not close.
            let mut open: Vec<Vec<(Vec3, f64)>> = wall.to_vec();
            for row in &mut open {
                row[8].0 = row[6].0 * 1.5;
            }
            assert_eq!(closes(&turned(&open, false)), [false, false]);
            // A cone to a point: the apex is a pole, not a seam.
            let cone = [ring(2.0, 0.0, from), ring(0.0, 3.0, from)];
            assert_eq!(closes(&turned(&cone, false)), [true, false]);
        }
        // A row of one point repeated is a pole both ways round.
        let point = vec![(Vec3::new(1.0, 2.0, 3.0), 1.0); 9];
        assert_eq!(
            closes(&turned(&[point.clone(), point], false)),
            [false, false]
        );
        // A flat Bézier patch closes nowhere; analytic surfaces say it
        // themselves.
        let flat = BezierPatch::new(
            1,
            1,
            vec![Vec3::ZERO, Vec3::Y, Vec3::X, Vec3::new(1.0, 1.0, 0.0)],
        )
        .unwrap();
        assert_eq!(closes(&Surface::Bezier(flat)), [false, false]);
    }
}
