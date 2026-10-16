//! Whether the loops of a face lie where they bound a face: every loop on
//! the inside of every other one, none crossing another.
//!
//! **The test.** A face lies to the left of its loops seen from outside,
//! so in a chart of its surface whose orientation agrees with the outward
//! normal the region of the face is where the loops wind once in total. A
//! point of a loop has the face just to its left, where that loop alone
//! winds once if it runs anticlockwise (an outer loop) and not at all if
//! it runs clockwise (a hole); so the other loops have to wind around the
//! point not at all for an anticlockwise loop and once for a clockwise one.
//! A hole left outside the outer loop (what a blend that cuts a face back
//! past a hole would leave) winds zero there, a loop inside a hole minus
//! one: both found.
//!
//! **What is held to the test.** Faces with two or more loops on a plane,
//! whose chart is the plane's own frame (exact, cheap). The loops are
//! followed as polylines (a line by its ends; a circle, an ellipse or a
//! spline in steps fine enough that the polyline stays within a small
//! fraction of the curve's size of it); a point of a loop is used only
//! where it is further from every other loop's polyline than that error
//! plus ten tolerances, so the winding there is that of the curves. Where
//! no point of a loop is that clear (loops that touch) nothing is said:
//! the other checks report what is wrong there. Curved faces (cylinders,
//! cones, spheres, tori and splines, whose charts close on themselves or
//! are found numerically) are not held to it yet.

use crate::{Body, Coedge, Face, FaceId, LoopId};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::Tolerance;

/// Most points one edge is followed at.
const MOST_PER_EDGE: usize = 4096;
/// Most points one loop is followed at.
const MOST_PER_LOOP: usize = 1 << 16;
/// How far, relative to the radius, a polyline may stay from a conic.
const SAGITTA: f64 = 1e-4;

/// One loop in the chart: its points (closed: the last joins the first)
/// and how far its polyline may be from the curves.
struct Polyline {
    points: Vec<[f64; 2]>,
    error: f64,
}

/// The loops of `face` that do not lie where they bound it (outside the
/// region the others bound, or crossing one). Empty for a face this test
/// does not hold (one loop; a surface other than a plane).
pub fn stranded_loops(body: &Body, face: &Face, tol: &Tolerance) -> Vec<LoopId> {
    if face.loops.len() < 2 {
        return Vec::new();
    }
    let Surface::Plane(plane) = &face.surface else {
        return Vec::new();
    };
    let frame = plane.frame();
    // The chart: the plane's x and y, turned over where the face's
    // outward side is the frame's -z.
    let sign = if face.reversed { -1.0 } else { 1.0 };
    let chart = |p: craft3d_math::Vec3| {
        let (x, y, _) = frame.local(p);
        [x, sign * y]
    };
    let mut lines = Vec::with_capacity(face.loops.len());
    for id in &face.loops {
        let Some(ring) = body.loop_(*id) else {
            return Vec::new();
        };
        let Some(line) = follow(body, &ring.coedges, &chart) else {
            return Vec::new();
        };
        lines.push(line);
    }
    let mut stranded = Vec::new();
    for (i, (id, own)) in face.loops.iter().zip(&lines).enumerate() {
        let area = signed_area(&own.points);
        if area == 0.0 || !area.is_finite() {
            continue;
        }
        let want = if area > 0.0 { 0 } else { 1 };
        let margin = |j: usize| -> f64 {
            lines.get(j).map_or(0.0, |l| l.error) + own.error + 10.0 * tol.linear
        };
        // The first point of the loop clear of every other loop.
        let point = own.points.iter().find(|p| {
            lines
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .all(|(j, other)| distance(**p, &other.points) > margin(j))
        });
        let Some(point) = point else {
            continue;
        };
        let total: i64 = lines
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, other)| winding(*point, &other.points))
            .sum();
        if total != want {
            stranded.push(*id);
        }
    }
    stranded
}

/// Every face of `body` with a loop that does not lie where it bounds it,
/// with those loops.
pub fn stranded(body: &Body, tol: &Tolerance) -> Vec<(FaceId, LoopId)> {
    let mut found = Vec::new();
    for (i, face) in body.faces.iter().enumerate() {
        for l in stranded_loops(body, face, tol) {
            found.push((FaceId(u32::try_from(i).unwrap_or(u32::MAX)), l));
        }
    }
    found
}

/// The loop's coedges as one closed polyline in the chart, or `None`
/// where an edge is missing or not finite, or the loop needs more points
/// than allowed.
fn follow(
    body: &Body,
    coedges: &[Coedge],
    chart: &dyn Fn(craft3d_math::Vec3) -> [f64; 2],
) -> Option<Polyline> {
    let mut points = Vec::new();
    let mut error = 0.0_f64;
    for coedge in coedges {
        let edge = body.edge(coedge.edge)?;
        let (t0, t1) = edge.range;
        if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
            return None;
        }
        let (steps, own) = steps(&edge.curve, (t0, t1));
        error = error.max(own);
        if points.len() + steps > MOST_PER_LOOP {
            return None;
        }
        // Each edge from its start, without its end (the next one's start).
        for k in 0..steps {
            let f = k as f64 / steps as f64;
            let t = if coedge.reversed {
                t1 - (t1 - t0) * f
            } else {
                t0 + (t1 - t0) * f
            };
            let p = chart(edge.curve.eval(t));
            if !(p[0].is_finite() && p[1].is_finite()) {
                return None;
            }
            points.push(p);
        }
    }
    (points.len() >= 2).then_some(Polyline { points, error })
}

/// How many steps `curve` is followed in over `range`, and how far the
/// polyline may then be from it.
fn steps(curve: &Curve3, (t0, t1): (f64, f64)) -> (usize, f64) {
    // A circle or an ellipse of largest semi-axis a has |x''(t)| <= a, so a
    // chord over a step dt is within a dt² / 8 of it: steps of
    // sqrt(8 SAGITTA) keep the polyline within SAGITTA a.
    let conic = |radius: f64| {
        let n = ((t1 - t0) / (8.0 * SAGITTA).sqrt()).ceil();
        let n = if n.is_finite() {
            n.max(1.0) as usize
        } else {
            1
        };
        (n.clamp(1, MOST_PER_EDGE), SAGITTA * radius)
    };
    match curve {
        Curve3::Line(_) => (1, 0.0),
        Curve3::Circle(c) => conic(c.radius()),
        Curve3::Ellipse(e) => {
            let (a, b) = e.semi_axes();
            conic(a.max(b))
        }
        Curve3::Nurbs(_) => {
            // Measured: the gap between each chord's middle and the curve's
            // point there, on a fine polyline.
            let n = 256_usize;
            let mut gap = 0.0_f64;
            for k in 0..n {
                let a = t0 + (t1 - t0) * k as f64 / n as f64;
                let b = t0 + (t1 - t0) * (k + 1) as f64 / n as f64;
                let middle = (curve.eval(a) + curve.eval(b)) * 0.5;
                gap = gap.max(middle.distance(curve.eval(0.5 * (a + b))));
            }
            (n, 4.0 * gap)
        }
    }
}

fn signed_area(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (Some(a), Some(b)) = (points.get(i), points.get((i + 1) % n)) else {
                return 0.0;
            };
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        * 0.5
}

/// The distance from `p` to the closed polyline `points`.
fn distance(p: [f64; 2], points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    (0..n)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % n)?);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let length2 = dx * dx + dy * dy;
            let t = if length2 > 0.0 {
                (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (x, y) = (a[0] + t * dx - p[0], a[1] + t * dy - p[1]);
            Some((x * x + y * y).sqrt())
        })
        .fold(f64::INFINITY, f64::min)
}

/// How many times the closed polyline `points` winds around `p`
/// anticlockwise (crossings of the ray to the right of `p`, signed).
fn winding(p: [f64; 2], points: &[[f64; 2]]) -> i64 {
    let n = points.len();
    let mut w = 0_i64;
    for i in 0..n {
        let (Some(a), Some(b)) = (points.get(i), points.get((i + 1) % n)) else {
            continue;
        };
        let side = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
        if a[1] <= p[1] {
            if b[1] > p[1] && side > 0.0 {
                w += 1;
            }
        } else if b[1] <= p[1] && side < 0.0 {
            w -= 1;
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_winds_once_around_its_inside_and_not_outside() {
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert_eq!(winding([0.5, 0.5], &square), 1);
        assert_eq!(winding([1.5, 0.5], &square), 0);
        let turned: Vec<[f64; 2]> = square.iter().rev().copied().collect();
        assert_eq!(winding([0.5, 0.5], &turned), -1);
        assert!(signed_area(&square) > 0.0 && signed_area(&turned) < 0.0);
    }
}
