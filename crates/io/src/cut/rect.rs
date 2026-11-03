//! The bounding rectangle a part is cut from: the rectangle of least area
//! around its outline, its longer side the length.
//!
//! The rectangle of least area around a convex polygon has a side along
//! one of the polygon's edges (Freeman and Shapira, "Determining the
//! minimum-area encasing rectangle for an arbitrary closed curve", CACM
//! 18(7), 1975). The outline's convex hull is taken of its points (every
//! segment's ends and 32 steps along each curved piece), the directions of
//! the hull's edges are tried, and in each the outline's extent is
//! measured exactly (lines, arcs, circles and ellipses; a spline to
//! within 1e-9 of its size, never below its extent: its Bézier pieces
//! halved where their control points reach further than the curve). For an outline whose hull has straight
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

/// Most halvings of Bézier pieces one spline's extreme may take.
const MAX_SPLITS: usize = 512;

/// An upper bound of `sign × p[axis]` over the spline, within 1e-9 of the
/// spline's size of the true largest value: the spline's Bézier pieces
/// halved by de Casteljau's algorithm (on the weighted points, for a
/// rational one) where their control points (which hold the curve, the
/// weights being positive) reach further than any point found on the curve
/// by more than that. Never below the curve's extreme: when the halvings run
/// out, the largest control point left counts.
fn furthest(spline: &Spline, axis: usize, sign: f64) -> f64 {
    let value = |p: &[f64; 3]| sign * p.get(axis).copied().unwrap_or(0.0) / p[2];
    let all = || {
        spline
            .control_points()
            .iter()
            .filter_map(|p| p.get(axis).map(|v| sign * v))
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let Ok(pieces) = spline.beziers() else {
        // The B-spline's control points hold it too.
        return all();
    };
    let size = {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in spline.control_points() {
            for v in p {
                lo = lo.min(*v);
                hi = hi.max(*v);
            }
        }
        (hi - lo).abs()
    };
    let eps = 1e-9 * size;
    let mut best = f64::NEG_INFINITY;
    let mut stack: Vec<Vec<[f64; 3]>> = Vec::new();
    for piece in &pieces {
        let w = |i: usize| {
            piece
                .weights
                .as_ref()
                .and_then(|w| w.get(i).copied())
                .unwrap_or(1.0)
        };
        let homogeneous: Vec<[f64; 3]> = piece
            .control
            .iter()
            .enumerate()
            .map(|(i, p)| [p[0] * w(i), p[1] * w(i), w(i)])
            .collect();
        for end in [homogeneous.first(), homogeneous.last()]
            .into_iter()
            .flatten()
        {
            best = best.max(value(end));
        }
        stack.push(homogeneous);
    }
    let upper = |c: &[[f64; 3]]| c.iter().map(value).fold(f64::NEG_INFINITY, f64::max);
    let mut splits = 0;
    let mut left_over = f64::NEG_INFINITY;
    while let Some(c) = stack.pop() {
        let top = upper(&c);
        if top.partial_cmp(&(best + eps)) != Some(std::cmp::Ordering::Greater) {
            continue;
        }
        if splits >= MAX_SPLITS {
            left_over = left_over.max(top);
            continue;
        }
        splits += 1;
        // De Casteljau at 1/2: the left half's points are the first of
        // each row, the right half's the last, reversed.
        let mut row = c.clone();
        let mut left = Vec::with_capacity(c.len());
        let mut right = Vec::with_capacity(c.len());
        while let Some(first) = row.first() {
            left.push(*first);
            if let Some(last) = row.last() {
                right.push(*last);
            }
            row = row
                .windows(2)
                .map(|w| match w {
                    [a, b] => [
                        0.5 * (a[0] + b[0]),
                        0.5 * (a[1] + b[1]),
                        0.5 * (a[2] + b[2]),
                    ],
                    _ => [0.0, 0.0, 1.0],
                })
                .collect();
        }
        right.reverse();
        if let Some(mid) = left.last() {
            best = best.max(value(mid));
        }
        stack.push(left);
        stack.push(right);
    }
    best.max(left_over)
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
    // A spline's extent costs halvings in each direction: fewer of them.
    let splines = matches!(&shape.outline, Path::Chain { segments, .. }
        if segments.iter().any(|s| matches!(s, Segment::Spline(_))));
    let most = if splines {
        MAX_DIRECTIONS / 8
    } else {
        MAX_DIRECTIONS
    };
    if directions.len() > most {
        let step = directions.len().div_ceil(most);
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
