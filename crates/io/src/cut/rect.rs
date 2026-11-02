//! The bounding rectangle a part is cut from: the rectangle of least area
//! around its outline, its longer side the length.
//!
//! The rectangle of least area around a convex polygon has a side along
//! one of the polygon's edges (Freeman and Shapira, "Determining the
//! minimum-area encasing rectangle for an arbitrary closed curve", CACM
//! 18(7), 1975). The outline's convex hull is taken of its points (every
//! segment's ends and 32 steps along each curved piece), the directions of
//! the hull's edges are tried, and in each the outline's extent is
//! measured exactly (lines, arcs, circles and ellipses; a spline's
//! furthest point by sampling refined by a ternary search). For an outline whose hull has straight
//! sides (any polygon, a rounded rectangle, a slot) the rectangle is the
//! least one exactly; where the hull is round there, it is the least over
//! the directions of the hull's chords.

use super::{Motion, Shape, moved_path, norm, one_layer, samples, sub};
use craft3d_drawing::{Path, Point, Segment, Spline};

/// Most directions tried.
const MAX_DIRECTIONS: usize = 8192;

/// The rectangle around an outline: `x` (a unit vector) along its length,
/// `length` ≥ `width`, and `corner` its corner of least x and y in the
/// rectangle's own axes, in the outline's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: Point,
    pub length: f64,
    pub width: f64,
    pub corner: Point,
}

/// The direction `d` turned by quarter turns into `[0°, 90°)`: x > 0,
/// y ≥ 0. A rectangle along a direction is the rectangle along the
/// direction turned by a quarter.
fn first_quadrant(d: Point) -> Point {
    let mut d = d;
    for _ in 0..4 {
        if d[0] > 0.0 && d[1] >= 0.0 {
            return d;
        }
        d = [d[1], -d[0]];
    }
    [1.0, 0.0]
}

/// The extent of `outline` in the axes x along `d`: `[min x, min y, max
/// x, max y]` there.
pub(super) fn extent(outline: &Path, d: Point) -> Result<[f64; 4], String> {
    let turned = moved_path(outline, &Motion::x_along(d, [0.0, 0.0]))?;
    // A spline's box is found on the curve itself (the drawing's box takes
    // its control polygon, which only bounds it): the chain measured with
    // each spline as the line between its ends, then each spline's
    // furthest point each way.
    let (plain, splines) = match &turned {
        Path::Chain {
            start,
            segments,
            closed,
        } => {
            let mut splines = Vec::new();
            let segments = segments
                .iter()
                .map(|s| match s {
                    Segment::Spline(spline) => {
                        splines.push(spline);
                        Segment::Line { to: spline.end() }
                    }
                    other => other.clone(),
                })
                .collect();
            (
                Path::Chain {
                    start: *start,
                    segments,
                    closed: *closed,
                },
                splines,
            )
        }
        other => (other.clone(), Vec::new()),
    };
    let mut b = one_layer(vec![plain])
        .bbox()
        .ok_or_else(|| "the outline has nothing to measure".to_string())?;
    for spline in splines {
        for (axis, sign) in [(0, -1.0), (1, -1.0), (0, 1.0), (1, 1.0)] {
            let far = sign * furthest(spline, axis, sign);
            match (axis, sign > 0.0) {
                (0, false) => b[0] = b[0].min(far),
                (1, false) => b[1] = b[1].min(far),
                (0, true) => b[2] = b[2].max(far),
                _ => b[3] = b[3].max(far),
            }
        }
    }
    Ok(b)
}

/// The largest `sign × p[axis]` over the spline: sampled (16 steps per
/// control point, at most 4096), then refined by a ternary search about
/// the best sample (60 steps: to rounding where the curve is smooth).
fn furthest(spline: &Spline, axis: usize, sign: f64) -> f64 {
    let f = |t: f64| {
        spline
            .eval(t)
            .ok()
            .and_then(|p| p.get(axis).copied())
            .map_or(f64::NEG_INFINITY, |v| sign * v)
    };
    let (lo, hi) = spline.domain();
    let n = spline
        .control_points()
        .len()
        .saturating_mul(16)
        .clamp(16, 4096);
    let step = (hi - lo) / n as f64;
    let mut best = (f(lo), lo);
    for i in 1..=n {
        let t = lo + step * i as f64;
        let v = f(t);
        if v > best.0 {
            best = (v, t);
        }
    }
    let (mut a, mut b) = ((best.1 - step).max(lo), (best.1 + step).min(hi));
    for _ in 0..60 {
        let (m1, m2) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
        if f(m1) < f(m2) {
            a = m1;
        } else {
            b = m2;
        }
    }
    best.0.max(f(0.5 * (a + b)))
}

/// The convex hull of `points` (Andrew's monotone chain), counter-clockwise.
fn hull(mut points: Vec<Point>) -> Vec<Point> {
    points.retain(|p| p[0].is_finite() && p[1].is_finite());
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let turn = |o: Point, a: Point, b: Point| super::cross(sub(a, o), sub(b, o));
    let mut lower: Vec<Point> = Vec::new();
    for p in &points {
        while let [.., a, b] = lower.as_slice() {
            if turn(*a, *b, *p) <= 0.0 {
                lower.pop();
            } else {
                break;
            }
        }
        lower.push(*p);
    }
    let mut upper: Vec<Point> = Vec::new();
    for p in points.iter().rev() {
        while let [.., a, b] = upper.as_slice() {
            if turn(*a, *b, *p) <= 0.0 {
                upper.pop();
            } else {
                break;
            }
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The rectangle of least area around `shape`'s outline. Of several of
/// the same area (within 1e-12 of it), the one turned least from the
/// frame's x axis is taken, so the result does not depend on the order of
/// the outline's segments.
pub fn bounding_rect(shape: &Shape) -> Result<Rect, String> {
    let mut points = Vec::new();
    samples(&shape.outline, 32, &mut points);
    let hull = hull(points);
    let mut directions: Vec<Point> = vec![[1.0, 0.0]];
    for (i, a) in hull.iter().enumerate() {
        let Some(b) = hull.get((i + 1) % hull.len()) else {
            continue;
        };
        let e = sub(*b, *a);
        let l = norm(e);
        if l.is_finite() && l > 0.0 {
            directions.push(first_quadrant([e[0] / l, e[1] / l]));
        }
    }
    // In the first quadrant the angle grows with the sine.
    directions.sort_by(|a, b| a[1].total_cmp(&b[1]));
    directions.dedup_by(|a, b| (a[1] - b[1]).abs() <= 1e-12 && (a[0] - b[0]).abs() <= 1e-12);
    if directions.len() > MAX_DIRECTIONS {
        let step = directions.len().div_ceil(MAX_DIRECTIONS);
        directions = directions.into_iter().step_by(step).collect();
    }
    let mut best: Option<(f64, Point, [f64; 4])> = None;
    for d in directions {
        let b = extent(&shape.outline, d)?;
        let area = (b[2] - b[0]) * (b[3] - b[1]);
        if !area.is_finite() {
            return Err("the outline has no finite extent".into());
        }
        if best.is_none_or(|(least, _, _)| area < least * (1.0 - 1e-12)) {
            best = Some((area, d, b));
        }
    }
    let (_, d, b) = best.ok_or_else(|| "the outline has no extent".to_string())?;
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    if h > w {
        // The longer side along x: the direction turned a quarter.
        let d = [-d[1], d[0]];
        let b = extent(&shape.outline, d)?;
        return Ok(Rect {
            x: d,
            length: b[2] - b[0],
            width: b[3] - b[1],
            corner: [b[0], b[1]],
        });
    }
    Ok(Rect {
        x: d,
        length: w,
        width: h,
        corner: [b[0], b[1]],
    })
}

/// The shape in its own frame: x along the length of its bounding
/// rectangle, the rectangle's corner at the origin, so the part lies in
/// `[0, length] × [0, width]`. Returns the moved shape, the motion from
/// the old frame to the new one, and the rectangle.
pub fn own_frame(shape: &Shape) -> Result<(Shape, Motion, Rect), String> {
    let rect = bounding_rect(shape)?;
    let motion = Motion::x_along(rect.x, [-rect.corner[0], -rect.corner[1]]);
    Ok((shape.moved(&motion)?, motion, rect))
}
