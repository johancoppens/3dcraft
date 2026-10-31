//! The curves of a projected view as the curves they are
//! (docs/file-formats.md › *Projected views*, `view_export.rs`): what an
//! edge's curve is in the drawing, the chords a line is followed along,
//! the record of what is drawn so that a line lying on another is drawn
//! once, and the paths that come out.
//!
//! - A line is a line under any projection.
//! - A circle or an ellipse of the model is `c + U cos t + V sin t` (its
//!   semi-axes as vectors). Orthographic, the drawing is the affine image
//!   `c' + U' cos t + V' sin t`, an ellipse whose semi-axes are the
//!   singular values of `(U' V')`: a circle where its plane is square to
//!   the view, a segment where it is seen edge on. In perspective it is the
//!   conic `adj(H)ᵀ diag(1, 1, −1) adj(H)` of the plane's homography `H`,
//!   an ellipse where the whole curve lies in front of the eye. Either way
//!   the ellipse is checked against the projected points of the curve
//!   before it is used; one seen edge on (its shorter semi-axis within the
//!   drawing's resolution) is drawn as its chords, which join into
//!   straight lines.
//! - A NURBS curve's control points projected, with its knots, are its
//!   drawing orthographically; in perspective the weights are multiplied
//!   by each control point's depth (the projective image of a rational
//!   curve: *The NURBS Book*, §1.5), where every control point is in front
//!   of the eye. Checked against projected points as the ellipse is.
//! - Anything else is its chords, as before: within the tolerance.

use crate::view_export::View;
use craft3d_drawing::{Path as DrawingPath, Point, Segment, Spline};
use craft3d_geom::Curve3;
use craft3d_math::Portable;
use craft3d_math::Vec3;
use std::collections::BTreeMap;

/// What an edge is in the drawing.
#[derive(Clone, Debug)]
pub(crate) enum Form {
    Line,
    /// An ellipse (a circle when `ratio` is 1 up to rounding) about
    /// `center` with the longer semi-axis `major` and the shorter `ratio`
    /// times as long.
    Conic {
        center: Point,
        major: Point,
        ratio: f64,
    },
    /// The curve over its own parameter.
    Spline(Spline),
    /// Straight chords between its breaks.
    Chords,
}

/// An ellipse whose ratio is this near 1 is a circle: its plane square to
/// the view up to rounding.
const CIRCLE: f64 = 1e-9;

/// Points of a projected curve must lie this near the ellipse or spline
/// found for it, relative to its size, for that to be used.
const ON_CURVE: f64 = 1e-8;

/// Most chords of one edge.
pub(crate) const MAX_BREAKS: usize = 1 << 15;

/// Deepest halving of a chord.
const MAX_HALVINGS: u32 = 30;

fn sub2(p: Point, q: Point) -> Point {
    [p[0] - q[0], p[1] - q[1]]
}

fn dot2(p: Point, q: Point) -> f64 {
    p[0] * q[0] + p[1] * q[1]
}

fn cross2(p: Point, q: Point) -> f64 {
    p[0] * q[1] - p[1] * q[0]
}

pub(crate) fn dist2(p: Point, q: Point) -> f64 {
    let d = sub2(p, q);
    dot2(d, d).sqrt()
}

/// The distance from `p` to the segment from `a` to `b`.
pub(crate) fn to_segment(p: Point, a: Point, b: Point) -> f64 {
    let d = sub2(b, a);
    let len2 = dot2(d, d);
    let t = if len2 > 0.0 {
        (dot2(sub2(p, a), d) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist2(p, [a[0] + d[0] * t, a[1] + d[1] * t])
}

/// How far `p` is from the segment from `a` to `b` where it lies beside
/// it (its foot on the segment, within `close` of its ends); `None` past
/// its ends. A piece covers what runs beside it, not what goes on beyond
/// its end.
fn beside(p: Point, a: Point, b: Point, close: f64) -> Option<f64> {
    let d = sub2(b, a);
    let length = dot2(d, d).sqrt();
    if length.is_nan() || length <= 0.0 {
        return (dist2(p, a) <= close).then_some(dist2(p, a));
    }
    let along = dot2(sub2(p, a), d) / length;
    if along < -close || along > length + close {
        return None;
    }
    Some(cross2(d, sub2(p, a)).abs() / length)
}

/// The same in space.
fn to_segment3(p: Vec3, a: Vec3, b: Vec3) -> f64 {
    let d = b - a;
    let len2 = d.dot(d);
    let t = if len2 > 0.0 {
        ((p - a).dot(d) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + d * t)
}

/// The eigenvalues of the symmetric `(a, b; b, c)`, the smaller first,
/// and the unit eigenvector of the larger.
fn eigen2(a: f64, b: f64, c: f64) -> (f64, f64, Point) {
    let mean = 0.5 * (a + c);
    let radius = (0.5 * (a - c)).phypot(b);
    let theta = 0.5 * (2.0 * b).patan2(a - c);
    (mean - radius, mean + radius, [theta.pcos(), theta.psin()])
}

/// The parameters at which an edge on `curve` over `range` is cut into
/// chords that keep within `target` of it (checked at a quarter, half and
/// three quarters of each): a line is one chord; a circle or an ellipse
/// starts at a sixteenth of a turn, a spline at its knots in `degree + 1`
/// parts each.
pub(crate) fn breaks_of(curve: &Curve3, range: (f64, f64), target: f64) -> Vec<f64> {
    let (t0, t1) = range;
    if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
        return Vec::new();
    }
    let mut first: Vec<f64> = vec![t0];
    match curve {
        Curve3::Line(_) => {}
        Curve3::Circle(_) | Curve3::Ellipse(_) => {
            let n = ((t1 - t0) / (std::f64::consts::PI / 8.0))
                .ceil()
                .clamp(1.0, 64.0) as usize;
            first.extend((1..n).map(|k| t0 + (t1 - t0) * k as f64 / n as f64));
        }
        Curve3::Nurbs(nurbs) => {
            let mut knots: Vec<f64> = nurbs
                .knots()
                .iter()
                .copied()
                .filter(|k| *k > t0 && *k < t1)
                .collect();
            knots.dedup();
            let parts = nurbs.degree().clamp(1, 8) + 1;
            let mut from = t0;
            for to in knots.into_iter().chain([t1]) {
                for k in 1..parts {
                    first.push(from + (to - from) * k as f64 / parts as f64);
                }
                if to < t1 {
                    first.push(to);
                }
                from = to;
            }
            first.truncate(MAX_BREAKS / 2);
        }
    }
    first.push(t1);
    let deviates = |a: f64, b: f64| {
        if matches!(curve, Curve3::Line(_)) {
            return false;
        }
        let (p, q) = (curve.eval(a), curve.eval(b));
        [0.25, 0.5, 0.75]
            .iter()
            .any(|s| to_segment3(curve.eval(a + (b - a) * s), p, q) > target)
    };
    let mut out = vec![t0];
    let mut stack: Vec<(f64, f64, u32)> = first
        .windows(2)
        .rev()
        .filter_map(|w| match w {
            [a, b] if b > a => Some((*a, *b, 0)),
            _ => None,
        })
        .collect();
    while let Some((a, b, depth)) = stack.pop() {
        let m = 0.5 * (a + b);
        if depth < MAX_HALVINGS
            && out.len() + stack.len() < MAX_BREAKS
            && m > a
            && m < b
            && deviates(a, b)
        {
            stack.push((m, b, depth + 1));
            stack.push((a, m, depth + 1));
        } else {
            out.push(b);
        }
    }
    out
}

/// What `curve` is in the drawing of `view`; `resolution` is the
/// drawing's (a semi-axis within it is none).
pub(crate) fn form_of(curve: &Curve3, view: &View, resolution: f64) -> Form {
    let form = match curve {
        Curve3::Line(_) => return Form::Line,
        Curve3::Circle(circle) => {
            let frame = circle.frame();
            let r = circle.radius();
            conic(
                view,
                frame.origin(),
                frame.x() * r,
                frame.y() * r,
                resolution,
            )
        }
        Curve3::Ellipse(ellipse) => {
            let frame = ellipse.frame();
            let (a, b) = ellipse.semi_axes();
            conic(
                view,
                frame.origin(),
                frame.x() * a,
                frame.y() * b,
                resolution,
            )
        }
        Curve3::Nurbs(nurbs) => spline(view, nurbs),
    };
    let Some(form) = form else {
        return Form::Chords;
    };
    // Checked against the curve's own points, projected.
    let (t0, t1) = match curve.domain() {
        (a, b) if a.is_finite() && b.is_finite() && b > a => (a, b),
        _ => return Form::Chords,
    };
    let fits = (0..=16).all(|k| {
        let t = t0 + (t1 - t0) * f64::from(k) / 16.0;
        let Some((p, _)) = view.project(curve.eval(t)) else {
            return false;
        };
        match &form {
            Form::Conic {
                center,
                major,
                ratio,
            } => {
                let a = dot2(*major, *major).sqrt();
                off_ellipse(p, *center, *major, *ratio) <= ON_CURVE * a
            }
            Form::Spline(s) => s.eval(t).is_ok_and(|q| {
                let size = s
                    .control_points()
                    .iter()
                    .map(|c| dist2(*c, s.start()))
                    .fold(0.0, f64::max)
                    .max(f64::MIN_POSITIVE);
                dist2(p, q) <= ON_CURVE * size
            }),
            Form::Line | Form::Chords => true,
        }
    });
    if fits { form } else { Form::Chords }
}

/// How far `p` is from the ellipse, by the point of it nearest in angle.
fn off_ellipse(p: Point, center: Point, major: Point, ratio: f64) -> f64 {
    let a2 = dot2(major, major);
    let minor = [-major[1] * ratio, major[0] * ratio];
    let b2 = dot2(minor, minor);
    if !(a2 > 0.0 && b2 > 0.0) {
        return f64::INFINITY;
    }
    let d = sub2(p, center);
    let t = (dot2(d, minor) / b2).patan2(dot2(d, major) / a2);
    let (s, c) = t.psin_cos();
    dist2(
        p,
        [
            center[0] + major[0] * c + minor[0] * s,
            center[1] + major[1] * c + minor[1] * s,
        ],
    )
}

/// The drawing of `c + u cos t + v sin t`.
fn conic(view: &View, c: Vec3, u: Vec3, v: Vec3, resolution: f64) -> Option<Form> {
    match view.perspective {
        None => {
            let (cu, cv) = (
                [u.dot(view.right), u.dot(view.up)],
                [v.dot(view.right), v.dot(view.up)],
            );
            let center = [c.dot(view.right), c.dot(view.up)];
            // The singular values of (u' v'): of (u' u'ᵀ + v' v'ᵀ).
            let s00 = cu[0] * cu[0] + cv[0] * cv[0];
            let s01 = cu[0] * cu[1] + cv[0] * cv[1];
            let s11 = cu[1] * cu[1] + cv[1] * cv[1];
            let (_, large, along) = eigen2(s00, s01, s11);
            let a = large.sqrt();
            let b = cross2(cu, cv).abs() / a;
            ellipse(center, along, a, b, resolution)
        }
        Some((eye, scale)) => {
            let q = c - eye;
            let f = view.forward;
            // Every point of the whole curve in front of the eye.
            let nearest = q.dot(f) - u.dot(f).phypot(v.dot(f));
            if nearest.is_nan() || nearest <= 1e-6 * q.length() {
                return None;
            }
            let col = |w: Vec3, z: f64| [scale * w.dot(view.right), scale * w.dot(view.up), z];
            let h = [col(u, u.dot(f)), col(v, v.dot(f)), col(q, q.dot(f))];
            // h[j] is the column j; m[i][j] = h[j][i].
            let m = |i: usize, j: usize| -> f64 {
                h.get(j).and_then(|col| col.get(i)).copied().unwrap_or(0.0)
            };
            let cofactor = |i: usize, j: usize| -> f64 {
                let (r0, r1) = ((i + 1) % 3, (i + 2) % 3);
                let (c0, c1) = ((j + 1) % 3, (j + 2) % 3);
                m(r0, c0) * m(r1, c1) - m(r0, c1) * m(r1, c0)
            };
            // adj[i][j] = cofactor(j, i); Q' = adjᵀ diag(1, 1, −1) adj.
            let adj = |i: usize, j: usize| cofactor(j, i);
            let q_of = |i: usize, j: usize| -> f64 {
                adj(0, i) * adj(0, j) + adj(1, i) * adj(1, j) - adj(2, i) * adj(2, j)
            };
            let (a00, a01, a11) = (q_of(0, 0), q_of(0, 1), q_of(1, 1));
            let (b0, b1, cc) = (q_of(0, 2), q_of(1, 2), q_of(2, 2));
            let det = a00 * a11 - a01 * a01;
            if !(det > 0.0 && det.is_finite()) {
                return None;
            }
            let center = [-(a11 * b0 - a01 * b1) / det, -(a00 * b1 - a01 * b0) / det];
            let s = -(b0 * center[0] + b1 * center[1]) - cc;
            let (small, large, along_large) = eigen2(a00, a01, a11);
            let (l1, l2) = (small / s, large / s);
            if !(l1 > 0.0 && l2 > 0.0 && l1.is_finite() && l2.is_finite()) {
                return None;
            }
            // The longer semi-axis goes with the smaller of l1, l2.
            let (a, b, along) = if l1 <= l2 {
                (
                    1.0 / l1.sqrt(),
                    1.0 / l2.sqrt(),
                    [-along_large[1], along_large[0]],
                )
            } else {
                (1.0 / l2.sqrt(), 1.0 / l1.sqrt(), along_large)
            };
            ellipse(center, along, a, b, resolution)
        }
    }
}

fn ellipse(center: Point, along: Point, a: f64, b: f64, resolution: f64) -> Option<Form> {
    if !(a.is_finite() && b.is_finite() && a > 0.0) || b <= resolution {
        return None;
    }
    let ratio = (b / a).min(1.0);
    let ratio = if ratio >= 1.0 - CIRCLE { 1.0 } else { ratio };
    Some(Form::Conic {
        center,
        major: [along[0] * a, along[1] * a],
        ratio,
    })
}

/// The drawing of a NURBS curve.
fn spline(view: &View, nurbs: &craft3d_geom::NurbsCurve) -> Option<Form> {
    let mut control = Vec::with_capacity(nurbs.control_points().len());
    let mut weights = Vec::with_capacity(control.capacity());
    for (p, w) in nurbs.control_points().iter().zip(nurbs.weights()) {
        let (at, depth) = view.project(*p)?;
        control.push(at);
        weights.push(match view.perspective {
            None => *w,
            Some(_) => w * depth,
        });
    }
    let heaviest = weights.iter().copied().fold(0.0, f64::max);
    if !(heaviest > 0.0 && heaviest.is_finite()) {
        return None;
    }
    let first = weights.first().copied();
    let rational = weights.iter().any(|w| Some(*w) != first);
    let weights = rational.then(|| weights.iter().map(|w| w / heaviest).collect());
    Spline::new(nurbs.degree(), nurbs.knots().to_vec(), control, weights)
        .ok()
        .map(Form::Spline)
}

/// A piece of a line already drawn, as its chord in the drawing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Drawn {
    pub a: Point,
    pub b: Point,
    /// How far what it stands for may lie from the chord.
    pub slack: f64,
    pub line: usize,
    pub t0: f64,
    pub t1: f64,
    /// The whole chord it is a piece of, from its start to its end: the
    /// way the line runs there.
    pub chord: Point,
}

/// What is drawn so far, in a grid over the drawing.
pub(crate) struct Registry {
    cell: f64,
    cap: usize,
    grid: BTreeMap<(i64, i64), Vec<usize>>,
    drawn: Vec<Drawn>,
    /// The largest slack of a piece drawn.
    widest: f64,
}

/// The cell of a grid of `cell` that `v` is in.
fn index(v: f64, cell: f64) -> i64 {
    let k = (v / cell).floor();
    if k.is_finite() {
        k.clamp(-1e12, 1e12) as i64
    } else {
        0
    }
}

impl Registry {
    /// A grid of cells at least `cell` wide; a piece crosses at most
    /// `cap` of them.
    pub(crate) fn new(cell: f64, cap: usize) -> Self {
        Self {
            cell: cell.max(f64::MIN_POSITIVE),
            cap,
            grid: BTreeMap::new(),
            drawn: Vec::new(),
            widest: 0.0,
        }
    }

    pub(crate) fn add(&mut self, piece: Drawn) {
        let at = self.drawn.len();
        for key in crate::view_export::cells_crossed(piece.a, piece.b, self.cell, self.cap) {
            self.grid.entry(key).or_default().push(at);
        }
        if piece.slack.is_finite() {
            self.widest = self.widest.max(piece.slack);
        }
        self.drawn.push(piece);
    }

    /// Whether `p`, of the chord of `line` over `chord` drawn in the
    /// direction `dir` and lying within `slack` of what it stands for, is
    /// on a piece drawn before: within the two slacks and `close` of its
    /// chord, which runs along this one. A line does not cover itself next to where it is (within the
    /// chord's own span of parameter, or across the start of a closed one
    /// of `period`) unless it runs back there. Returns the tests made.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn covers(
        &self,
        p: Point,
        slack: f64,
        line: usize,
        chord: (f64, f64),
        dir: Point,
        period: Option<f64>,
        close: f64,
    ) -> (bool, usize) {
        let (i, j) = (index(p[0], self.cell), index(p[1], self.cell));
        let mut tests = 0;
        let width = chord.1 - chord.0;
        let near = |d: &Drawn| {
            let gap = |shift: f64| {
                (d.t0 + shift - chord.1)
                    .max(chord.0 - d.t1 - shift)
                    .max(0.0)
            };
            let mut least = gap(0.0);
            if let Some(period) = period {
                least = least.min(gap(period)).min(gap(-period));
            }
            least <= width
        };
        // The cells next to it only where it lies within reach of their
        // border: the grid is at least as wide as the reach.
        let reach = close + slack + self.widest;
        let side = |v: f64, k: i64| -> (i64, i64) {
            let low = v - k as f64 * self.cell;
            let high = (k + 1) as f64 * self.cell - v;
            (
                if low <= reach { -1 } else { 0 },
                if high <= reach { 1 } else { 0 },
            )
        };
        let ((i0, i1), (j0, j1)) = (side(p[0], i), side(p[1], j));
        let mut seen: Vec<usize> = Vec::new();
        for di in i0..=i1 {
            for dj in j0..=j1 {
                if let Some(list) = self.grid.get(&(i + di, j + dj)) {
                    seen.extend(list.iter().copied());
                }
            }
        }
        seen.sort_unstable();
        seen.dedup();
        for id in seen {
            let Some(d) = self.drawn.get(id) else {
                continue;
            };
            tests += 1;
            if d.line == line && near(d) && dot2(dir, sub2(d.b, d.a)) >= 0.0 {
                continue;
            }
            let reach = close + slack + d.slack;
            if beside(p, d.a, d.b, close).is_none_or(|off| off > reach) {
                continue;
            }
            // Only a piece that runs along it: two chords within `reach`
            // of one curve turn from each other by at most about
            // 8 reach / length, so a line that meets or crosses it at an
            // angle covers nothing of it.
            let along = d.chord;
            let (l1, l2) = (dot2(dir, dir).sqrt(), dot2(along, along).sqrt());
            let shorter = l1.min(l2);
            // A chord too short to tell which way it runs (an edge-on
            // circle where it turns) covers only its own line.
            if shorter.is_nan() || shorter <= 16.0 * reach {
                if d.line == line {
                    return (true, tests);
                }
                continue;
            }
            let sine = cross2(dir, along).abs() / (l1 * l2);
            if sine <= 16.0 * reach / shorter {
                return (true, tests);
            }
        }
        (false, tests)
    }
}

/// Points joined into straight segments: a point between two others is
/// left out when it lies within `close` of the line through them and the
/// line goes on the same way.
pub(crate) fn simplified(points: &[Point], close: f64) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last().is_some_and(|last| dist2(*last, p) <= close) {
            continue;
        }
        if let [.., a, b] = out.as_slice() {
            let (a, b) = (*a, *b);
            let along = dot2(sub2(b, a), sub2(p, b)) > 0.0;
            if along && to_segment(b, a, p) <= close {
                if let Some(last) = out.last_mut() {
                    *last = p;
                }
                continue;
            }
        }
        out.push(p);
    }
    out
}

/// The path of an open run of points, or of a closed loop.
pub(crate) fn chain_of(points: &[Point], closed: bool, close: f64) -> Option<DrawingPath> {
    let mut points = simplified(points, close);
    if closed
        && points.len() > 3
        && let (Some(first), Some(last)) = (points.first().copied(), points.last().copied())
        && dist2(first, last) <= close
    {
        points.pop();
    }
    let (&start, rest) = points.split_first()?;
    if rest.is_empty() {
        return None;
    }
    let mut segments: Vec<Segment> = rest.iter().map(|p| Segment::Line { to: *p }).collect();
    let closed = closed && segments.len() >= 2;
    if closed {
        segments.push(Segment::Line { to: start });
    }
    Some(DrawingPath::Chain {
        start,
        segments,
        closed,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use craft3d_math::Portable;

    fn front() -> View {
        View {
            right: Vec3::X,
            up: Vec3::Z,
            forward: Vec3::Y,
            perspective: None,
            near: 0.0,
        }
    }

    fn circle(axis: Vec3, r: f64) -> Curve3 {
        let frame = craft3d_math::Frame::new(Vec3::new(1.0, 2.0, 3.0), axis, {
            let x = if axis.cross(Vec3::X).length() > 0.1 {
                Vec3::X
            } else {
                Vec3::Y
            };
            (x - axis * x.dot(axis)).normalized().unwrap()
        })
        .unwrap();
        Curve3::Circle(craft3d_geom::Circle::new(frame, r).unwrap())
    }

    #[test]
    fn a_circle_is_a_circle_face_on_an_ellipse_tilted_and_chords_edge_on() {
        let view = front();
        match form_of(&circle(Vec3::Y, 2.0), &view, 1e-9) {
            Form::Conic {
                center,
                major,
                ratio,
            } => {
                assert_eq!(ratio, 1.0);
                assert!((dist2(major, [0.0, 0.0]) - 2.0).abs() < 1e-12);
                assert!(dist2(center, [1.0, 3.0]) < 1e-12);
            }
            other => panic!("{other:?}"),
        }
        // Tilted by 30° about x: semi-axes 2 and 2 cos 30°.
        let (s, c) = 30f64.to_radians().psin_cos();
        match form_of(&circle(Vec3::new(0.0, c, s), 2.0), &view, 1e-9) {
            Form::Conic { major, ratio, .. } => {
                assert!((dist2(major, [0.0, 0.0]) - 2.0).abs() < 1e-12);
                assert!((ratio - c).abs() < 1e-12, "{ratio}");
                // The longer axis along x.
                assert!(major[1].abs() < 1e-12, "{major:?}");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            form_of(&circle(Vec3::Z, 2.0), &view, 1e-9),
            Form::Chords
        ));
    }

    #[test]
    fn a_circle_in_perspective_is_the_ellipse_of_its_points() {
        let view = View {
            right: Vec3::X,
            up: Vec3::Z,
            forward: Vec3::Y,
            perspective: Some((Vec3::new(0.5, -20.0, 1.0), 20.0)),
            near: 1.0,
        };
        let (s, c) = 40f64.to_radians().psin_cos();
        let curve = circle(Vec3::new(0.3, c, s).normalized().unwrap(), 2.0);
        let Form::Conic {
            center,
            major,
            ratio,
        } = form_of(&curve, &view, 1e-9)
        else {
            panic!("not an ellipse");
        };
        for k in 0..50 {
            let t = f64::from(k) * 0.1257;
            let p = view.project(curve.eval(t)).unwrap().0;
            assert!(off_ellipse(p, center, major, ratio) < 1e-9, "{k}");
        }
        // Behind the eye in part: chords.
        let behind = View {
            perspective: Some((Vec3::new(1.0, 2.5, 3.0), 20.0)),
            ..view
        };
        assert!(matches!(form_of(&curve, &behind, 1e-9), Form::Chords));
    }

    #[test]
    fn the_breaks_keep_within_the_target_and_a_line_is_one_chord() {
        let curve = circle(Vec3::Z, 1.0);
        let breaks = breaks_of(&curve, (0.0, std::f64::consts::TAU), 1e-4);
        assert!(breaks.windows(2).all(|w| w[1] > w[0]));
        let step = breaks.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max);
        // Sagitta r (1 − cos(Δ/2)) within the target.
        assert!(1.0 - (0.5 * step).pcos() <= 1e-4, "{step}");
        let line = Curve3::Line(craft3d_geom::Line::new(Vec3::ZERO, Vec3::X).unwrap());
        assert_eq!(breaks_of(&line, (0.0, 3.0), 1e-9), [0.0, 3.0]);
        assert!(breaks_of(&line, (3.0, 0.0), 1e-9).is_empty());
        assert!(breaks_of(&line, (f64::NAN, 1.0), 1e-9).is_empty());
    }

    #[test]
    fn points_in_line_join_and_a_line_running_back_covers_itself() {
        let joined = simplified(&[[0.0, 0.0], [1.0, 1e-12], [2.0, 0.0], [2.0, 1.0]], 1e-9);
        assert_eq!(joined, [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0]]);
        let mut registry = Registry::new(1.0, 64);
        registry.add(Drawn {
            a: [0.0, 0.0],
            b: [1.0, 0.0],
            slack: 0.0,
            line: 0,
            t0: 0.0,
            t1: 1.0,
            chord: [1.0, 0.0],
        });
        // The next chord of the same line going on: not covered.
        let (on, _) = registry.covers([1.0, 0.0], 0.0, 0, (1.0, 2.0), [1.0, 0.0], None, 1e-9);
        assert!(!on);
        // Coming back over it: covered.
        let (back, _) = registry.covers([0.5, 0.0], 0.0, 0, (1.0, 2.0), [-1.0, 0.0], None, 1e-9);
        assert!(back);
        // Another line on it: covered.
        let (other, _) = registry.covers([0.5, 0.0], 0.0, 1, (0.0, 1.0), [1.0, 0.0], None, 1e-9);
        assert!(other);
        let (apart, _) = registry.covers([0.5, 0.1], 0.0, 1, (0.0, 1.0), [1.0, 0.0], None, 1e-9);
        assert!(!apart);
        // Another line meeting it square: not covered where they meet.
        let (square, _) = registry.covers([1.0, 0.0], 0.0, 1, (0.0, 1.0), [0.0, 1.0], None, 1e-9);
        assert!(!square);
    }
}
