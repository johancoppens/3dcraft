//! The developable connection of two curves: the sheet between two rails
//! whose rulings are chosen so that its Gaussian curvature is zero, the
//! surface a flat sheet bends onto without stretching.
//!
//! A ruled surface between `A(s)` and `B(t)` is developable exactly when
//! the tangent plane is the same all along each ruling, that is when
//! `A'(s)`, `B'(t)` and `B(t) − A(s)` are coplanar (Pottmann & Wallner,
//! *Computational Line Geometry*, §6.1; do Carmo §3-5). With unit vectors,
//! `f(s, t) = det[Â', B̂', D̂]`, `D = B − A`, is independent of how the
//! rails are parametrized. The rulings are a path through the rectangle of
//! `(s, t)` from the first ends to the last, increasing in both (rulings
//! that never cross), along the zero set of `f`, with *fans* where the
//! path has to run along a side of the rectangle: the rulings from one
//! point of a rail to a stretch of the other are a cone, developable too,
//! and meet the rulings beside them with the same tangent plane.
//!
//! Where the zero set turns back in one parameter (later rulings would
//! cross earlier ones: one rail inflects where the other does not), the
//! gap is closed by a fan whose end rulings both satisfy the condition
//! (see [`trace`]). Where it turns back in both (a rail that winds about
//! the other), vanishes over a stretch (both rails in one plane there), or
//! a ruling shrinks to nothing between the ends, the connection is
//! refused with the place. Rails that lie in one plane are
//! connected by the planar region between them. The method, its sources
//! and its oracles: docs/design/developable.md.

use crate::KernelError;
use craft3d_geom::refine::interpolate_with_end_derivatives;
use craft3d_geom::{Curve3, Line, NurbsSurface, Plane, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId};
use std::collections::BTreeMap;

const OPERATION: &str = "developable";

/// Most curves in one rail.
pub const MAX_RAIL_PIECES: usize = 4096;
/// Most rulings the faces are fitted through.
const MAX_RULINGS: usize = 8192;
/// Most steps of the walk along the zero set.
const MAX_STEPS: usize = 1 << 16;
/// The longest step of the walk, in the rectangle scaled to the unit square.
const STEP_MAX: f64 = 1.0 / 128.0;
/// Rounds of adding rulings where the fit is not yet within tolerance.
const MAX_ROUNDS: usize = 16;

/// One curve of a rail, run from `range.0` to `range.1` in the curve's
/// own direction, or back when `reversed`.
#[derive(Clone, Debug, PartialEq)]
pub struct RailPiece {
    pub curve: Curve3,
    pub range: (f64, f64),
    pub reversed: bool,
    /// The caller's id (a sketch curve, an edge), for messages.
    pub source: u64,
}

/// A chain of curves end to end, the way it is run. Its parameter runs
/// from 0 at its first end to the number of its pieces at its last, piece
/// `i` over `[i, i + 1]`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Rail {
    pub pieces: Vec<RailPiece>,
}

/// Which face of the sheet, named by its place (2026-10-28: by place,
/// not by count, so that a fold that appears or goes at one end of a
/// sheet leaves the names of the faces at the other as they are).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Piece {
    /// The first run of rulings that satisfy the condition (the only one
    /// of a sheet without folds or corners).
    Rulings,
    /// The cone from the first end of one rail to the start of the other.
    StartFan,
    /// The cone from the last end of one rail to the end of the other.
    EndFan,
    /// The fan at a fold of the condition (a cone at one point of a rail,
    /// across a stretch of the other) or from a corner of a rail.
    Fan(Mark),
    /// A run of rulings after a corner, or after a fold counted from the
    /// start.
    After(Mark),
    /// A run of rulings, after a fold counted from the end, before the
    /// mark that follows it.
    Before(Mark),
    /// The run of rulings that reaches the end, when it is not the first.
    Last,
}

/// What parts two runs of rulings of a sheet: a fold of the condition
/// (closed by a fan) or a corner of a rail (a fan from it, or the faces
/// part there).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    /// A fold in the first half of the sheet (by the mean of the unit rail
    /// parameters of its fan's first ruling), the `n`-th counted from the
    /// start (from 0).
    FoldFromStart(u32),
    /// A fold in the second half, the `n`-th counted from the end.
    FoldFromEnd(u32),
    /// The corner of rail `a` (`rail` 0) or `b` (1) before its piece whose
    /// [`RailPiece::source`] is `source`.
    Corner { rail: u8, source: u64 },
}

/// What [`connect`] made.
#[derive(Clone, Debug)]
pub struct Connected {
    /// A sheet of one shell.
    pub body: Body,
    /// Per face of `body`, in order, which piece it is.
    pub faces: Vec<Piece>,
    /// The rulings found, as `(s, t)` rail parameters, in order.
    pub rulings: Vec<(f64, f64)>,
    /// The largest distance found between the faces and the true
    /// developable (its rails and its rulings), in model units. At a
    /// face's end rulings: the turn of the tangent plane it is built with
    /// (from the rails' tangents) from the developable's, times the
    /// ruling's length, which is at most half the tolerance (else refused).
    pub deviation: f64,
    /// An estimate, not a bound, and not counted in `deviation`: the
    /// largest angle found between a stored face's normal at an end ruling
    /// and the tangent plane it was built with there, times the ruling's
    /// length (model units). The face's rows leave their end rulings along
    /// the rails' tangents; the stored control points next to the end
    /// carry the rounding of their coordinates, about `ε |p|` each, over a
    /// first control leg as short as a third of the end interval, which
    /// turns the stored normal by about `ε |p| / leg`. Measured, so
    /// whatever the rounding does at that end; near the origin a small
    /// fraction of the tolerance, far from it over a short end interval
    /// (the sea kayak's panels lie some 2e9 tolerances out) it can reach
    /// the tolerance. Positions are not affected beyond `ε |p|`.
    pub unresolved: f64,
    /// The largest |K| found on the faces.
    pub curvature: f64,
    /// Whether the rails lie in one plane (the sheet is that plane).
    pub planar: bool,
    /// Whether `b` was run the other way (only that gives a developable).
    pub reversed_b: bool,
}

fn refused(reason: String) -> KernelError {
    KernelError::Refused {
        operation: OPERATION,
        reason,
    }
}

fn fmt_point(p: Vec3) -> String {
    format!("({:.6}, {:.6}, {:.6})", p.x, p.y, p.z)
}

impl Rail {
    fn len(&self) -> f64 {
        self.pieces.len() as f64
    }

    /// The piece at `s`, the curve parameter there and `dc/ds`; at a joint between two pieces (`s` a whole
    /// number inside the rail), the piece before it when `before`, else
    /// the one after (the two sides of a corner).
    fn locate_side(&self, s: f64, before: bool) -> Option<(&RailPiece, f64, f64)> {
        let count = self.pieces.len();
        if count == 0 || !s.is_finite() {
            return None;
        }
        let s = s.clamp(0.0, count as f64);
        let joint = before && s >= 1.0 && s.fract() == 0.0;
        let index = if joint {
            s as usize - 1
        } else {
            (s.floor() as usize).min(count - 1)
        };
        let piece = self.pieces.get(index)?;
        let local = s - index as f64;
        let (a, b) = piece.range;
        let span = b - a;
        Some(if piece.reversed {
            (piece, b - local * span, -span)
        } else {
            (piece, a + local * span, span)
        })
    }

    /// `(point, d/ds)` at `s`.
    fn at(&self, s: f64) -> Option<(Vec3, Vec3)> {
        self.at_side(s, false)
    }

    /// `(point, d/ds)` at `s`, on the side before a joint when `before`
    /// ([`Rail::locate_side`]).
    fn at_side(&self, s: f64, before: bool) -> Option<(Vec3, Vec3)> {
        let (piece, c, rate) = self.locate_side(s, before)?;
        let (p, d) = piece.curve.derivatives(c);
        Some((p, d * rate))
    }

    /// The joints where the rail has a corner: the tangent turns there by
    /// more than `angular` (as whole-number rail parameters, in order).
    fn corners(&self, angular: f64) -> Vec<f64> {
        let turn = |u: Vec3, w: Vec3| u.cross(w).length().atan2(u.dot(w));
        (1..self.pieces.len())
            .filter_map(|k| {
                let s = k as f64;
                let (_, before) = self.at_side(s, true)?;
                let (_, after) = self.at_side(s, false)?;
                let (u, w) = (before.normalized()?, after.normalized()?);
                (turn(u, w) > angular).then_some(s)
            })
            .collect()
    }

    fn point(&self, s: f64) -> Option<Vec3> {
        self.at(s).map(|(p, _)| p)
    }

    /// The length of the rail between `from` and `to` (`from <= to`), by
    /// Gauss–Legendre quadrature per piece.
    fn length_between(&self, from: f64, to: f64) -> f64 {
        let rule = craft3d_math::quadrature::Rule::new(8);
        let mut total = 0.0;
        let (from, to) = (from.max(0.0), to.min(self.len()));
        let mut a = from;
        while a < to {
            let b = (a.floor() + 1.0).min(to);
            let panels = 8;
            total += rule.integrate(a, b, panels, |s| {
                self.at(s).map_or(0.0, |(_, d)| d.length())
            });
            if b <= a {
                break;
            }
            a = b;
        }
        total
    }

    /// The rail parameter nearest `p`, from `start` (Newton on the
    /// squared distance, kept in `[lo, hi]`).
    fn nearest(&self, p: Vec3, start: f64, lo: f64, hi: f64) -> f64 {
        let mut s = start.clamp(lo, hi);
        for _ in 0..32 {
            let Some((q, d)) = self.at(s) else {
                return s;
            };
            let dd = d.length_squared();
            if !(dd > 0.0 && dd.is_finite()) {
                return s;
            }
            let step = (p - q).dot(d) / dd;
            let next = (s + step).clamp(lo, hi);
            if (next - s).abs() <= 1e-15 * (1.0 + s.abs()) {
                return next;
            }
            s = next;
        }
        s
    }

    fn check(&self, name: &str, tolerance: &Tolerance) -> Result<(), KernelError> {
        let (tolerance, angular) = (tolerance.linear, tolerance.angular);
        if self.pieces.is_empty() {
            return Err(refused(format!("{name} has no curves")));
        }
        if self.pieces.len() > MAX_RAIL_PIECES {
            return Err(refused(format!(
                "{name} has {} curves; a rail takes at most {MAX_RAIL_PIECES}",
                self.pieces.len()
            )));
        }
        for (index, piece) in self.pieces.iter().enumerate() {
            let (a, b) = piece.range;
            if !(a.is_finite() && b.is_finite() && a < b) {
                return Err(refused(format!(
                    "{name}: curve {} has no range to run along",
                    piece.source
                )));
            }
            if let Some(next) = self.pieces.get(index + 1) {
                let start = if next.reversed {
                    next.curve.eval(next.range.1)
                } else {
                    next.curve.eval(next.range.0)
                };
                let finish = if piece.reversed {
                    piece.curve.eval(a)
                } else {
                    piece.curve.eval(b)
                };
                if finish.distance(start) > tolerance {
                    return Err(refused(format!(
                        "{name}: curve {} ends at {} and curve {} starts at {}; a rail is a chain of curves end to end",
                        piece.source,
                        fmt_point(finish),
                        next.source,
                        fmt_point(start)
                    )));
                }
                // A cusp: the tangent turns back where two curves meet (a
                // corner is a fan from it, [`trace`]).
                let (_, d_out) = piece.curve.derivatives(if piece.reversed { a } else { b });
                let (_, d_in) = next.curve.derivatives(if next.reversed {
                    next.range.1
                } else {
                    next.range.0
                });
                let d_out = if piece.reversed { -d_out } else { d_out };
                let d_in = if next.reversed { -d_in } else { d_in };
                let turn = |u: Vec3, w: Vec3| u.cross(w).length().atan2(u.dot(w));
                if let (Some(u), Some(w)) = (d_out.normalized(), d_in.normalized())
                    && turn(u, w) >= std::f64::consts::PI - angular
                {
                    return Err(refused(format!(
                        "{name} turns back on itself at {} between curves {} and {} (a cusp: the tangent turns by {:.3} degrees); no sheet follows it there",
                        fmt_point(finish),
                        piece.source,
                        next.source,
                        turn(u, w).to_degrees()
                    )));
                }
            }
        }
        Ok(())
    }
}

/// The two rails, scaled so that each runs over `[0, 1]`, and their
/// corners there (the lines of the square on which the condition jumps).
struct Pair<'a> {
    a: &'a Rail,
    b: &'a Rail,
    corners_a: Vec<f64>,
    corners_b: Vec<f64>,
    /// Both rails close on themselves (a tube): their ends are one point
    /// each, and the last ruling is the first.
    closed: bool,
}

impl<'a> Pair<'a> {
    fn new(a: &'a Rail, b: &'a Rail, angular: f64) -> Pair<'a> {
        let unit = |rail: &Rail| -> Vec<f64> {
            let len = rail.len();
            rail.corners(angular).into_iter().map(|s| s / len).collect()
        };
        Pair {
            corners_a: unit(a),
            corners_b: unit(b),
            a,
            b,
            closed: false,
        }
    }
}

/// A rail parameter within rounding of a joint between pieces is the
/// joint (a corner's line, `k / n` in the unit square, scaled back).
fn snapped(s: f64, len: f64) -> f64 {
    let r = s.round();
    if (s - r).abs() <= 1e-12 * len.max(1.0) {
        r
    } else {
        s
    }
}

impl Pair<'_> {
    fn sa(&self, x: f64) -> f64 {
        snapped(x * self.a.len(), self.a.len())
    }
    fn tb(&self, y: f64) -> f64 {
        snapped(y * self.b.len(), self.b.len())
    }
    fn points(&self, x: f64, y: f64) -> Option<(Vec3, Vec3)> {
        Some((self.a.point(self.sa(x))?, self.b.point(self.tb(y))?))
    }

    /// The condition: `det[Â', B̂', D̂]`, `None` where a tangent or the
    /// ruling has no direction.
    fn f(&self, x: f64, y: f64) -> Option<f64> {
        self.f_side(x, y, false, false)
    }

    /// The condition with each rail's tangent taken before a joint where
    /// asked (the two sides of a corner, [`Rail::locate_side`]).
    fn f_side(&self, x: f64, y: f64, before_a: bool, before_b: bool) -> Option<f64> {
        let (pa, da) = self.a.at_side(self.sa(x), before_a)?;
        let (pb, db) = self.b.at_side(self.tb(y), before_b)?;
        let (ua, ub, ud) = (da.normalized()?, db.normalized()?, (pb - pa).normalized()?);
        let value = ua.dot(ub.cross(ud));
        value.is_finite().then_some(value)
    }

    /// The gradient of `f` by central differences, one-sided at the
    /// sides of the square and beside a corner's line (on the side the
    /// point is on; on the line, the side after it).
    fn gradient(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let h = 1e-7;
        let diff = |lo: f64, hi: f64, at: &dyn Fn(f64) -> Option<f64>| -> Option<f64> {
            let (a, b) = (lo.max(0.0), hi.min(1.0));
            if b <= a {
                return None;
            }
            Some((at(b)? - at(a)?) / (b - a))
        };
        let sided = |here: f64, corners: &[f64]| -> (f64, f64) {
            match corners.iter().find(|c| (here - **c).abs() < h) {
                Some(&c) if here >= c => (here, here + h),
                Some(_) => (here - h, here),
                None => (here - h, here + h),
            }
        };
        let (x0, x1) = sided(x, &self.corners_a);
        let (y0, y1) = sided(y, &self.corners_b);
        let gx = diff(x0, x1, &|x| self.f(x, y))?;
        let gy = diff(y0, y1, &|y| self.f(x, y))?;
        (gx.is_finite() && gy.is_finite()).then_some((gx, gy))
    }

    /// Whether the stretch from `p` to `q` crosses a corner's line (one it
    /// does not end on, nor start from).
    fn crosses_corner(&self, p: (f64, f64), q: (f64, f64)) -> bool {
        let across = |a: f64, b: f64, lines: &[f64]| {
            let (lo, hi) = (a.min(b), a.max(b));
            lines.iter().any(|c| *c > lo && *c < hi)
        };
        across(p.0, q.0, &self.corners_a) || across(p.1, q.1, &self.corners_b)
    }

    fn where_(&self, x: f64, y: f64) -> String {
        match self.points(x, y) {
            Some((pa, pb)) => format!(
                "between {} on the first curve and {} on the second",
                fmt_point(pa),
                fmt_point(pb)
            ),
            None => format!("at ({x}, {y}) along the rails"),
        }
    }
}

/// The first zero of `g` on `(lo, 1]` by sampling and bisection.
fn first_root(g: &dyn Fn(f64) -> Option<f64>, lo: f64) -> Option<f64> {
    const SAMPLES: usize = 512;
    let mut before: Option<(f64, f64)> = None;
    for k in 0..=SAMPLES {
        let x = lo + (1.0 - lo) * k as f64 / SAMPLES as f64;
        let Some(value) = g(x) else {
            before = None;
            continue;
        };
        if value == 0.0 {
            return Some(x);
        }
        if let Some((x0, v0)) = before
            && (v0 < 0.0) != (value < 0.0)
        {
            let (mut a, mut b, mut va) = (x0, x, v0);
            for _ in 0..80 {
                let m = 0.5 * (a + b);
                let Some(vm) = g(m) else {
                    break;
                };
                if (vm < 0.0) == (va < 0.0) {
                    a = m;
                    va = vm;
                } else {
                    b = m;
                }
            }
            return Some(0.5 * (a + b));
        }
        before = Some((x, value));
    }
    None
}

/// Newton back onto `f = 0` along the gradient, from `(x, y)`.
fn onto_zero(pair: &Pair, mut x: f64, mut y: f64) -> Option<(f64, f64)> {
    for _ in 0..24 {
        let value = pair.f(x, y)?;
        if value.abs() <= 1e-14 {
            return Some((x, y));
        }
        let (gx, gy) = pair.gradient(x, y)?;
        let g2 = gx * gx + gy * gy;
        if g2.is_nan() || g2 <= 1e-24 {
            return None;
        }
        x = (x - value * gx / g2).clamp(0.0, 1.0);
        y = (y - value * gy / g2).clamp(0.0, 1.0);
    }
    let value = pair.f(x, y)?;
    (value.abs() <= 1e-10).then_some((x, y))
}

/// Where the path through the square starts, and the fan before it.
enum Start {
    /// From `(0, 0)` on.
    Corner,
    /// A fan from the first end of A to `B(y)`: the path starts at `(0, y)`.
    FromA(f64),
    /// A fan from the first end of B to `A(x)`.
    FromB(f64),
}

/// The tangent of the zero set at `(x, y)`, oriented to increase `x + y`.
fn tangent_at(pair: &Pair, x: f64, y: f64) -> Option<(f64, f64)> {
    let (gx, gy) = pair.gradient(x, y)?;
    let norm = gx.hypot(gy);
    if norm.is_nan() || norm <= 1e-9 {
        return None;
    }
    let (tx, ty) = (gy / norm, -gx / norm);
    Some(if tx + ty < 0.0 { (-tx, -ty) } else { (tx, ty) })
}

/// The shortest step the walk tries (in the unit square) before it gives
/// up where the condition cannot be followed forward.
const MIN_STEP: f64 = 1e-10;

/// The step towards a saddle `far` ahead ([`saddle_ahead`]): half its
/// distance, so the walk keeps to its own branch, but never below twice
/// [`MIN_STEP`]. A saddle nearer than that is within what the walk
/// resolves, not ahead of it: halving towards it would take the step under
/// the floor without a step having failed, and the walk would give up
/// there ("cannot be followed forward") for no reason of the condition's.
/// A step across such a saddle is still rejected by [`split_between`] and
/// halved like any other.
fn approach(h: f64, far: f64) -> f64 {
    h.min(0.5 * far).max(2.0 * MIN_STEP).min(h)
}

/// The largest turn of the zero set's tangent between two steps of the
/// walk (radians): a step that turns it more is halved, so a fold of the
/// zero set is walked into, not jumped over.
const MAX_TURN: f64 = 0.2;

/// The path of the rulings through the square: its points, and per point
/// whether the stretch to the next one is a fan (at one `t`, its apex on
/// B, or at one `s`, its apex on A) rather than rulings on the zero set.
#[derive(Clone, Debug, Default)]
struct Path {
    points: Vec<(f64, f64)>,
    fan_after: Vec<bool>,
    /// Per point, whether the faces part there (a corner of a rail whose
    /// fan is no longer than the tolerance: the rulings go on, the faces
    /// do not, as a rail's row cannot follow a corner).
    break_at: Vec<bool>,
}

impl Path {
    fn push(&mut self, p: (f64, f64)) {
        self.points.push(p);
        self.fan_after.push(false);
        self.break_at.push(false);
    }
    /// The faces part at the last point.
    fn mark_break(&mut self) {
        if let Some(last) = self.break_at.last_mut() {
            *last = true;
        }
    }
    /// The stretch from the last point to `p` is a fan.
    fn push_fan(&mut self, p: (f64, f64)) {
        if let Some(last) = self.fan_after.last_mut() {
            *last = true;
        }
        self.push(p);
    }
    /// Whether the stretch after point `k` is a fan.
    fn special_after(&self, k: usize) -> bool {
        self.fan_after.get(k).copied().unwrap_or(false)
    }
    /// Whether point `k` ends or starts a fan, or the faces part there (it
    /// is kept as it is).
    fn fixed(&self, k: usize) -> bool {
        self.special_after(k)
            || k.checked_sub(1).is_some_and(|j| self.special_after(j))
            || self.break_at.get(k).copied().unwrap_or(false)
    }
}

/// A fold of the zero set the walk is going round: where it turned (the
/// last point of the path), whether `t` falls there (else `s`), and the
/// bridge across, if there is one (below).
#[derive(Clone, Copy, Debug)]
struct Fold {
    at: (f64, f64),
    falls_t: bool,
    bridge: Option<(f64, f64)>,
}

impl Fold {
    /// The fan's parameter while the walk goes round: the `t` (or `s`)
    /// where the zero set turned.
    fn level(&self) -> f64 {
        if self.falls_t { self.at.1 } else { self.at.0 }
    }
}

const WINDS: &str =
    "the condition turns back in both curves there (a curve that winds about the other)";

/// The walk along the zero set of `f` from `start` until it reaches the
/// side `x = 1` or `y = 1`.
///
/// Where the zero set turns back in one parameter (a fold: the rulings
/// beyond it would cross those before it, as where one rail inflects and
/// the other does not), the rulings that would cross are left out and the
/// gap is closed by a fan, a cone with its apex at the point of a rail
/// where the zero set turned, whose two end rulings both satisfy the
/// condition, so that it shares its tangent planes with the rulings on
/// either side (a cone is developable: Pottmann & Wallner §6.1; Frey &
/// Bindschadler 1993 join developable pieces through cones the same way).
/// There are two such fans at a fold where `t` would fall at `(s*, t*)`
/// (and likewise with `s` and `t` swapped):
///
/// - *across*, at `s = s*` (apex `A(s*)`), from `t*` up to the next zero
///   of `f(s*, ·)` from which the zero set goes on forward: where two
///   branches of the zero set pass each other (the crossing a rail's
///   inflection makes, split because the other rail does not inflect
///   with it), the bridge from one to the other;
/// - *along*, at `t = t*` (apex `B(t*)`), from `s*` to where the zero set,
///   followed round the fold without keeping its points, comes back to
///   `t*` going forward: a dip.
///
/// Of the two that exist, the one that covers less of its rail. A
/// crossing of two branches within the tolerance is stepped over instead
/// ([`cross`]). Where neither fan exists (both parameters turn back at
/// once, the curves wind about each other; or the zero set leaves the
/// square before it comes back, with nothing across), refused.
fn trace(
    pair: &Pair,
    start: (f64, f64),
    tolerance: f64,
    tips: [bool; 2],
    reach: f64,
) -> Result<Path, KernelError> {
    let mut path = Path::default();
    path.push(start);
    let (mut x, mut y) = start;
    let mut h = STEP_MAX / 4.0;
    let mut steps = 0;
    let mut fold: Option<Fold> = None;
    // The tangent of the last step, to orient the next one by.
    let mut before = (
        std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    );
    // A tangent along a side of the square (the zero set leaves a corner
    // vertically, as where one rail's parameter runs as the other's
    // square) has a component lost in the gradient's rounding: clamped to
    // zero. A turn back is a component clearly below it.
    const BACK: f64 = -1e-6;
    // Going round a fold that does not come back: the bridge across, or
    // the reason.
    macro_rules! give_up {
        ($error:expr) => {{
            match fold.take().and_then(|f| f.bridge) {
                Some(q) => {
                    path.push_fan(q);
                    (x, y) = q;
                    h = STEP_MAX / 4.0;
                    before = (
                        std::f64::consts::FRAC_1_SQRT_2,
                        std::f64::consts::FRAC_1_SQRT_2,
                    );
                    continue;
                }
                None => return Err($error),
            }
        }};
    }
    while x < 1.0 && y < 1.0 {
        steps += 1;
        if steps > MAX_STEPS {
            give_up!(refused(format!(
                "the rulings could not be followed in {MAX_STEPS} steps {}",
                pair.where_(x, y)
            )));
        }
        let Some((gx, gy)) = pair.gradient(x, y) else {
            give_up!(refused(format!(
                "a tangent of a curve or a ruling has no direction {}",
                pair.where_(x, y)
            )));
        };
        let norm = gx.hypot(gy);
        if norm.is_nan() || norm <= 1e-9 {
            if fold.is_none()
                && let Some(q) = cross(
                    pair,
                    (x, y),
                    heading(&path.points),
                    STEP_MAX / 4.0,
                    tolerance,
                )
            {
                (x, y) = q;
                path.push(q);
                h = STEP_MAX / 4.0;
                continue;
            }
            give_up!(refused(format!(
                "the condition for the rulings vanishes over a stretch {}: the curves lie in one plane there (or the rulings branch), so the developable is not unique; split the curves there",
                pair.where_(x, y)
            )));
        }
        // Oriented as the walk was going.
        let (mut tx, mut ty) = (gy / norm, -gx / norm);
        if tx * before.0 + ty * before.1 < 0.0 {
            tx = -tx;
            ty = -ty;
        }
        let winds = match fold {
            None => tx < BACK && ty < BACK,
            Some(f) if f.falls_t => tx < BACK,
            Some(_) => ty < BACK,
        };
        if winds {
            give_up!(refused(format!(
                "the rulings would cross {}: {WINDS}",
                pair.where_(x, y)
            )));
        }
        // A saddle ahead where the branches pass each other by more than
        // `cross` steps over: approached, so that the walk stays on its
        // branch and meets the fold where it turns ([`saddle_ahead`]).
        if fold.is_none()
            && let Some(far) = saddle_ahead(pair, (x, y), (tx, ty), h, tolerance)
        {
            h = approach(h, far);
        }
        if fold.is_none() && (tx < BACK || ty < BACK) {
            if let Some(q) = cross(
                pair,
                (x, y),
                heading(&path.points),
                STEP_MAX / 4.0,
                tolerance,
            ) {
                (x, y) = q;
                path.push(q);
                h = STEP_MAX / 4.0;
                continue;
            }
            // Round the fold: where it turned is found between the last
            // point kept and this one (the walk notices the turn a step
            // after it), and kept in place of this one.
            let falls_t = ty < BACK;
            let at = fold_start(pair, &mut path, (x, y), falls_t);
            fold = Some(Fold {
                at,
                falls_t,
                bridge: bridge(pair, at, falls_t, tolerance),
            });
        }
        // The direction to step in: forward in what must go forward (both
        // parameters on the zero set, the one that does not turn back
        // round a fold).
        let (tx, ty) = {
            let (cx, cy) = match fold {
                None => (tx.max(0.0), ty.max(0.0)),
                Some(f) if f.falls_t => (tx.max(0.0), ty),
                Some(_) => (tx, ty.max(0.0)),
            };
            let n = cx.hypot(cy).max(f64::MIN_POSITIVE);
            (cx / n, cy / n)
        };
        // Predictor, stopped at the sides of the square.
        let mut step = h;
        if tx > 0.0 {
            step = step.min((1.0 - x) / tx);
        }
        if ty > 0.0 {
            step = step.min((1.0 - y) / ty);
        }
        if tx < 0.0 {
            step = step.min(x / -tx);
        }
        if ty < 0.0 {
            step = step.min(y / -ty);
        }
        // Stopped at the next corner's line ahead (the condition jumps
        // there): `(on A's line, the line, coming from below)`.
        let mut on_corner: Option<(bool, f64, bool)> = None;
        for (along_a, direction, here, lines) in [
            (true, tx, x, &pair.corners_a),
            (false, ty, y, &pair.corners_b),
        ] {
            let next = if direction > 0.0 {
                lines.iter().copied().find(|c| *c > here)
            } else if direction < 0.0 {
                lines.iter().rev().copied().find(|c| *c < here)
            } else {
                None
            };
            if let Some(line) = next {
                let reach = (line - here) / direction;
                if reach > 0.0 && reach <= step {
                    step = reach;
                    on_corner = Some((along_a, line, direction > 0.0));
                }
            }
        }
        // Both rails' corners ahead within the step, and the ruling between
        // the two corners within the tolerance of a developable on the side
        // the walk comes from: the walk goes to that ruling (a crease of the
        // sheet, as a generalised cylinder over a profile with a corner has),
        // whichever line rounding would have it reach first. Without this
        // the corrector on one line meets the jump of the condition on the
        // other within rounding of the root and finds no root on either
        // side.
        if fold.is_none()
            && tx > 0.0
            && ty > 0.0
            && let (Some(la), Some(lb)) = (
                pair.corners_a.iter().copied().find(|c| *c > x),
                pair.corners_b.iter().copied().find(|c| *c > y),
            )
            && (la - x) / tx <= h
            && (lb - y) / ty <= h
            && ruling_twist_side(pair, la, lb, true, true).is_some_and(|d| d <= 0.5 * tolerance)
        {
            if ruling_twist(pair, la, lb).is_none_or(|d| d > 0.5 * tolerance) {
                return Err(refused(format!(
                    "both curves have a corner on the ruling {}, and the rulings after the corners do not start from it: not built (split the curves there)",
                    pair.where_(la, lb)
                )));
            }
            let moved = (la - x).hypot(lb - y);
            if moved > 0.0 {
                before = ((la - x) / moved, (lb - y) / moved);
            }
            path.push((la, lb));
            path.mark_break();
            (x, y) = (la, lb);
            h = STEP_MAX / 4.0;
            continue;
        }
        let (px, py) = match on_corner {
            Some((true, line, _)) => (line, (y + step * ty).clamp(0.0, 1.0)),
            Some((false, line, _)) => ((x + step * tx).clamp(0.0, 1.0), line),
            None => (
                (x + step * tx).clamp(0.0, 1.0),
                (y + step * ty).clamp(0.0, 1.0),
            ),
        };
        let at_side = on_corner.is_none() && (px >= 1.0 || py >= 1.0);
        // Corrector: back onto the zero set; on a side, along that side;
        // on a corner's line, along it, with the tangent of the side the
        // walk comes from.
        let corrected = if let Some((along_a, line, rising)) = on_corner {
            if along_a {
                nearest_root(&|l| pair.f_side(line, py + l, rising, false), 2.0 * step)
                    .map(|l| (line, (py + l).clamp(0.0, 1.0)))
            } else {
                nearest_root(&|l| pair.f_side(px + l, line, false, rising), 2.0 * step)
                    .map(|l| ((px + l).clamp(0.0, 1.0), line))
            }
        } else if at_side {
            if px >= 1.0 {
                root_between(
                    &|v| pair.f(1.0, v),
                    y.min(py),
                    (y.max(py) + 2.0 * step).min(1.0),
                )
                .map(|v| (1.0, v))
            } else {
                root_between(
                    &|u| pair.f(u, 1.0),
                    x.min(px),
                    (x.max(px) + 2.0 * step).min(1.0),
                )
                .map(|u| (u, 1.0))
            }
            .or_else(|| onto_zero(pair, px, py))
        } else {
            onto_zero(pair, px, py)
        };
        // Accepted only forward, about a step away, and with the
        // tangent turned by little: a corrector that lands on another
        // branch, or a step over a fold, is retried shorter.
        let accepted = corrected.filter(|&(qx, qy)| {
            let moved = (qx - x).hypot(qy - y);
            // On a corner's line the tangent is taken a little way back,
            // on the side the walk came from.
            let (ax, ay) = match on_corner {
                Some((true, line, _)) => (
                    line - (2e-7_f64).min(0.5 * (line - x).abs()).copysign(line - x),
                    qy,
                ),
                Some((false, line, _)) => (
                    qx,
                    line - (2e-7_f64).min(0.5 * (line - y).abs()).copysign(line - y),
                ),
                None => (qx, qy),
            };
            let turned = tangent_at(pair, ax, ay)
                .is_some_and(|(ux, uy)| (ux * tx + uy * ty).abs() >= MAX_TURN.cos());
            let forward = match fold {
                None => qx >= x - 1e-12 && qy >= y - 1e-12,
                Some(f) if f.falls_t => qx >= x - 1e-12,
                Some(_) => qy >= y - 1e-12,
            };
            forward
                && moved <= 1.5 * step + 1e-12
                && moved > 0.0
                && (turned || at_side)
                && !pair.crosses_corner((x, y), (qx, qy))
                && (fold.is_some() || !split_between(pair, (x, y), (qx, qy), tolerance))
        });
        let Some((qx, qy)) = accepted else {
            h *= 0.5;
            if h < MIN_STEP {
                if fold.is_none()
                    && let Some(q) = cross(
                        pair,
                        (x, y),
                        heading(&path.points),
                        STEP_MAX / 4.0,
                        tolerance,
                    )
                {
                    (x, y) = q;
                    path.push(q);
                    h = STEP_MAX / 4.0;
                    continue;
                }
                give_up!(refused(format!(
                    "the rulings would cross {}: the condition cannot be followed forward there; split the curves there",
                    pair.where_(x, y)
                )));
            }
            continue;
        };
        let moved = (qx - x).hypot(qy - y);
        if moved > 0.0 {
            before = ((qx - x) / moved, (qy - y) / moved);
        }
        h = (h * 1.5).min(STEP_MAX);
        if fold.is_some() && on_corner.is_some() {
            give_up!(refused(format!(
                "the condition turns back at a corner of a curve {}: not built",
                pair.where_(qx, qy)
            )));
        }
        if let Some(f) = fold {
            let level = f.level();
            let back = if f.falls_t { qy >= level } else { qx >= level };
            if back {
                // Back at the fan's parameter: the fan along ends where
                // the zero set crosses it; the one that covers less of its
                // rail is taken, this one or the bridge across.
                let end = if f.falls_t {
                    (
                        root_between(&|u| pair.f(u, level), x, qx).unwrap_or(qx),
                        level,
                    )
                } else {
                    (
                        level,
                        root_between(&|v| pair.f(level, v), y, qy).unwrap_or(qy),
                    )
                };
                let along = if f.falls_t {
                    pair.a.length_between(pair.sa(f.at.0), pair.sa(end.0))
                } else {
                    pair.b.length_between(pair.tb(f.at.1), pair.tb(end.1))
                };
                let chosen = match f.bridge {
                    Some(q) if bridge_length(pair, f.at, q, f.falls_t) < along => q,
                    _ => end,
                };
                path.push_fan(chosen);
                (x, y) = chosen;
                fold = None;
                continue;
            }
            if qx >= 1.0 || qy >= 1.0 || qx <= 0.0 || qy <= 0.0 {
                give_up!(refused(format!(
                    "the rulings would cross {}: the condition turns back there and does not come back before the end of a curve, nor goes on across, so no smooth developable joins the curves past it (a fan from there would crease the sheet); split the curves there",
                    pair.where_(f.at.0, f.at.1)
                )));
            }
            (x, y) = (qx, qy);
            continue;
        }
        // At a corner: the fan from it to where the condition on the side
        // beyond goes on forward, or the faces part there.
        if let Some((along_a, line, rising)) = on_corner {
            if !rising {
                return Err(refused(format!(
                    "the condition turns back across a corner of a curve {}: not built",
                    pair.where_(qx, qy)
                )));
            }
            // A corner of the other rail on the same ruling (within the
            // tolerance along that rail): the ruling is both corners'.
            if let Some(both) = double_corner(pair, along_a, (qx, qy), tolerance) {
                // The faces part there; the rulings after it start there
                // too, or no developable turns both corners.
                if ruling_twist(pair, both.0, both.1).is_none_or(|d| d > 0.5 * tolerance) {
                    return Err(refused(format!(
                        "both curves have a corner on the ruling {}, and the rulings after the corners do not start from it: not built (split the curves there)",
                        pair.where_(both.0, both.1)
                    )));
                }
                path.push(both);
                path.mark_break();
                (x, y) = both;
                h = STEP_MAX / 4.0;
                continue;
            }
            path.push((qx, qy));
            let beyond = corner_fan(
                pair,
                along_a,
                line,
                if along_a { qy } else { qx },
                tolerance,
            )?;
            match beyond {
                Some(q) => {
                    path.push_fan(q);
                    (x, y) = q;
                }
                None => {
                    path.mark_break();
                    (x, y) = (qx, qy);
                }
            }
            h = STEP_MAX / 4.0;
            continue;
        }
        // A ruling that shrinks to nothing away from ends that meet.
        if let Some((pa, pb)) = pair.points(qx, qy) {
            // Near ends that meet the rulings shrink to nothing: within
            // `reach` (an arc length) of such an end along the first rail.
            let s = pair.sa(qx);
            let near_tip = (tips[0] && pair.a.length_between(0.0, s) <= reach)
                || (tips[1] && pair.a.length_between(s, pair.a.len()) <= reach);
            if pa.distance(pb) <= tolerance && !near_tip {
                return Err(refused(format!(
                    "the curves meet {}: a ruling has no length there",
                    pair.where_(qx, qy)
                )));
            }
        }
        x = qx;
        y = qy;
        path.push((x, y));
    }
    Ok(path)
}

/// Where a fold of the zero set the walk noticed at `here` (the last point
/// of `path`) starts, with that point updated: the turn lies between the
/// point before and `here` (the walk notices it a step late), found there
/// ([`turning_point`]) and kept in place of `here`. Where `here` ends a
/// fan (the fan landed on a branch that already turns back, as where a
/// fan across a fold lands on the other branch short of that branch's own
/// fold), the point before lies across the fan, not on the branch, and
/// the fan's end must stay where it is: the fold starts at `here`. Its
/// fan then starts on the ruling the first one ended on, both on the
/// zero set, so the two share their tangent planes there.
fn fold_start(pair: &Pair, path: &mut Path, here: (f64, f64), falls_t: bool) -> (f64, f64) {
    let Some(last) = path.points.len().checked_sub(1) else {
        return here;
    };
    if path.fixed(last) {
        return here;
    }
    let previous = last
        .checked_sub(1)
        .and_then(|k| path.points.get(k))
        .copied()
        .unwrap_or(here);
    let at = turning_point(pair, previous, here, falls_t).unwrap_or(here);
    if let Some(point) = path.points.get_mut(last) {
        *point = at;
    }
    at
}

/// The path with fans that follow one another with one apex made one:
/// two such fans are one cone, and a face of each would split it at a
/// ruling no geometry marks (review of 2026-10-29: the walk round a fold
/// that starts where a fan landed can meet the fan's line again within
/// rounding on its first step, and end a fan along of no width there,
/// followed by the fan along it was going round to). A fan is at one `s`
/// (apex on A) or one `t` (apex on B) between its two points.
fn merged_fans(mut path: Path) -> Path {
    let mut k = 1;
    while k + 1 < path.points.len() {
        let (Some(&p), Some(&q), Some(&r)) = (
            path.points.get(k - 1),
            path.points.get(k),
            path.points.get(k + 1),
        ) else {
            break;
        };
        let both_fans = path.special_after(k - 1) && path.special_after(k);
        let one_apex = (p.0 == q.0 && q.0 == r.0) || (p.1 == q.1 && q.1 == r.1);
        if both_fans && one_apex && !path.break_at.get(k).copied().unwrap_or(false) {
            path.points.remove(k);
            path.fan_after.remove(k);
            path.break_at.remove(k);
        } else {
            k += 1;
        }
    }
    path
}

/// The fans at the sheet's ends with a fan of the path beside them that
/// has the same apex made one (as [`merged_fans`] makes fans in a row
/// with one apex one; review of 2026-10-29): the start's fan (apex at the
/// first end of A for [`Start::FromA`], of B for [`Start::FromB`]) and a
/// fan the path starts with whose apex is that end, likewise at the end
/// (`end` the fan from the last end of A, [`Start::FromA`], or of B). The
/// path's fan is taken into the end's fan, which keeps its name (the
/// faces that hold the sheet's ends keep theirs, [`middle_strips`]); a
/// path of only that fan is left as it is (it would have no rulings).
fn merged_end_fans(
    mut path: Path,
    start: Start,
    end: Option<Start>,
) -> (Path, Start, Option<Start>) {
    let start = match (start, path.points.first(), path.points.get(1)) {
        (Start::FromA(_), Some(&p), Some(&q))
            if path.points.len() >= 3 && path.special_after(0) && p.0 == 0.0 && q.0 == 0.0 =>
        {
            path.points.remove(0);
            path.fan_after.remove(0);
            path.break_at.remove(0);
            Start::FromA(q.1)
        }
        (Start::FromB(_), Some(&p), Some(&q))
            if path.points.len() >= 3 && path.special_after(0) && p.1 == 0.0 && q.1 == 0.0 =>
        {
            path.points.remove(0);
            path.fan_after.remove(0);
            path.break_at.remove(0);
            Start::FromB(q.0)
        }
        (start, _, _) => start,
    };
    let n = path.points.len();
    let last_two = n
        .checked_sub(2)
        .and_then(|k| Some((k, *path.points.get(k)?, *path.points.get(k + 1)?)));
    let end = match (end, last_two) {
        (Some(Start::FromA(_)), Some((k, p, q)))
            if n >= 3 && path.special_after(k) && p.0 >= 1.0 && q.0 >= 1.0 =>
        {
            path.points.pop();
            path.fan_after.pop();
            path.break_at.pop();
            if let Some(f) = path.fan_after.last_mut() {
                *f = false;
            }
            Some(Start::FromA(p.1))
        }
        (Some(Start::FromB(_)), Some((k, p, q)))
            if n >= 3 && path.special_after(k) && p.1 >= 1.0 && q.1 >= 1.0 =>
        {
            path.points.pop();
            path.fan_after.pop();
            path.break_at.pop();
            if let Some(f) = path.fan_after.last_mut() {
                *f = false;
            }
            Some(Start::FromB(p.0))
        }
        (end, _) => end,
    };
    (path, start, end)
}

/// Where the walk reached a corner's line of one rail at `q` (of A when
/// `along_a`), the corner of the other rail that the ruling there meets
/// within the tolerance along that rail, if any: the point on both lines.
/// There the ruling is a crease of the sheet (both rails turn on it, as a
/// generalised cylinder over a profile with a corner does), and the faces
/// part along it.
fn double_corner(pair: &Pair, along_a: bool, q: (f64, f64), tolerance: f64) -> Option<(f64, f64)> {
    if along_a {
        pair.corners_b
            .iter()
            .copied()
            .find(|c| {
                let (lo, hi) = (q.1.min(*c), q.1.max(*c));
                pair.b.length_between(pair.tb(lo), pair.tb(hi)) <= tolerance
            })
            .map(|c| (q.0, c))
    } else {
        pair.corners_a
            .iter()
            .copied()
            .find(|c| {
                let (lo, hi) = (q.0.min(*c), q.0.max(*c));
                pair.a.length_between(pair.sa(lo), pair.sa(hi)) <= tolerance
            })
            .map(|c| (c, q.1))
    }
}

/// The fan at a corner of a rail that the walk reached on its line at
/// `here` (the other parameter): a corner of A at `x = line` when
/// `along_a`, else of B at `y = line`. On the corner's line the condition
/// jumps (the rail's tangent does), so the zero set the walk followed with
/// the tangent before the corner ends there, and the one with the tangent
/// after it starts from another point of the line: the gap is a cone with
/// its apex at the corner, from the walk's ruling to the first ruling
/// along the line, forward, from which the condition after the corner
/// goes on forward (as [`bridge`] does at a fold). Its two end rulings
/// satisfy the condition of the face on their side, so the fan shares its
/// tangent planes with both: the sheet is smooth, its rail has the
/// corner. `None` when that ruling is within the tolerance of the walk's
/// (the faces part there and the rulings go on); refused when none goes on
/// forward (the rulings beyond the corner would cross those before it) or
/// the fan would reach a corner of the other rail.
fn corner_fan(
    pair: &Pair,
    along_a: bool,
    line: f64,
    here: f64,
    tolerance: f64,
) -> Result<Option<(f64, f64)>, KernelError> {
    let g = |w: f64| {
        if along_a {
            pair.f(line, w)
        } else {
            pair.f(w, line)
        }
    };
    let at = |w: f64| if along_a { (line, w) } else { (w, line) };
    // The length of the other rail the fan would cover.
    let covered = |w: f64| {
        let (lo, hi) = (here.min(w), here.max(w));
        if along_a {
            pair.b.length_between(pair.tb(lo), pair.tb(hi))
        } else {
            pair.a.length_between(pair.sa(lo), pair.sa(hi))
        }
    };
    let (p, q) = at(here);
    let where_ = pair.where_(p, q);
    // A root within the tolerance of the walk's ruling: no fan.
    let speed = {
        let (rail, w) = if along_a {
            (pair.b, pair.tb(here))
        } else {
            (pair.a, pair.sa(here))
        };
        rail.at(w).map_or(0.0, |(_, d)| d.length()) * rail.len()
    };
    if speed > 0.0 && speed.is_finite() {
        let window = (2.0 * tolerance / speed).min(1e-3);
        if let Some(l) = nearest_root(&|l| g(here + l), window)
            && covered(here + l) <= tolerance
        {
            return Ok(None);
        }
    }
    let gap = 1e-9;
    let mut from = here;
    for _ in 0..64 {
        if from >= 1.0 {
            break;
        }
        let Some(root) = first_root(&g, from) else {
            break;
        };
        let (rx, ry) = at(root);
        // Forward on the side after the corner, a little way past its line.
        let probe = if along_a {
            ((rx + 2e-7).min(1.0), ry)
        } else {
            (rx, (ry + 2e-7).min(1.0))
        };
        if let Some((tx, ty)) = tangent_at(pair, probe.0, probe.1) {
            let going_on = if along_a { tx >= 0.0 } else { ty >= 0.0 };
            if going_on {
                if covered(root) <= tolerance {
                    return Ok(None);
                }
                let crossed = if along_a {
                    pair.corners_b.iter().any(|c| *c > here && *c < root)
                } else {
                    pair.corners_a.iter().any(|c| *c > here && *c < root)
                };
                if crossed {
                    return Err(refused(format!(
                        "the fan from a corner of one curve {where_} would reach past a corner of the other: not built"
                    )));
                }
                return Ok(Some(at(root)));
            }
        }
        from = root + gap;
    }
    Err(refused(format!(
        "the rulings would cross at a corner of a curve {where_}: no ruling from the corner on goes on forward beyond it, so no developable turns the corner (split the curves there)"
    )))
}

/// The fan across a fold at `at` where `t` falls (`falls_t`; else `s`):
/// at the fold's `s` (apex on A), from its `t` up to the next zero of
/// `f(s, ·)` where the zero set goes on forward (not back in `s`); `None`
/// when there is none before the end of the rail.
///
/// A zero whose fan would cover no more than the tolerance of its rail is
/// the fold's own (where the zero set touches the fan's line, rounding
/// makes a pair of zeros either side of it), or one no fan is made for, as
/// at a corner ([`corner_fan`]): the search starts past it, the tolerance
/// along the rail in its unit parameter by the rail's speed at the fold.
fn bridge(pair: &Pair, at: (f64, f64), falls_t: bool, tolerance: f64) -> Option<(f64, f64)> {
    let gap = bridge_gap(pair, at, falls_t, tolerance)?;
    let mut from = if falls_t { at.1 } else { at.0 } + gap;
    for _ in 0..64 {
        if from >= 1.0 {
            return None;
        }
        let root = if falls_t {
            first_root(&|v| pair.f(at.0, v), from)?
        } else {
            first_root(&|u| pair.f(u, at.1), from)?
        };
        let q = if falls_t { (at.0, root) } else { (root, at.1) };
        if let Some((tx, ty)) = tangent_at(pair, q.0, q.1) {
            let going_on = if falls_t { tx >= 0.0 } else { ty >= 0.0 };
            if going_on {
                return Some(q);
            }
        }
        from = root + gap;
    }
    None
}

/// The tolerance along the rail a fan across a fold at `at` covers (B
/// when `falls_t`, else A), in the unit parameter: `tol / (|r'| n)` with
/// `r'` the rail's derivative there and `n` its count of pieces; no less
/// than the rounding of the parameter itself. `None` where the rail has no
/// speed there.
fn bridge_gap(pair: &Pair, at: (f64, f64), falls_t: bool, tolerance: f64) -> Option<f64> {
    let (rail, w) = if falls_t {
        (pair.b, pair.tb(at.1))
    } else {
        (pair.a, pair.sa(at.0))
    };
    let speed = rail.at(w).map(|(_, d)| d.length() * rail.len())?;
    let gap = tolerance / speed;
    (gap.is_finite() && gap > 0.0).then_some(gap.max(64.0 * f64::EPSILON))
}

/// How much of its rail the fan across a fold covers.
fn bridge_length(pair: &Pair, at: (f64, f64), to: (f64, f64), falls_t: bool) -> f64 {
    if falls_t {
        pair.b.length_between(pair.tb(at.1), pair.tb(to.1))
    } else {
        pair.a.length_between(pair.sa(at.0), pair.sa(to.0))
    }
}

/// Where the zero set turns between `from` (going forward in both) and
/// `to` (one parameter falling): where its tangent is square to the axis
/// of that parameter (`t` when `falls_t`), by bisection along the chord,
/// each point put on the zero set; `None` when a point cannot be put on
/// it between the two.
fn turning_point(
    pair: &Pair,
    from: (f64, f64),
    to: (f64, f64),
    falls_t: bool,
) -> Option<(f64, f64)> {
    let (lo_x, hi_x) = (from.0.min(to.0), from.0.max(to.0));
    let (lo_y, hi_y) = (from.1.min(to.1), from.1.max(to.1));
    let room = 2.0 * (hi_x - lo_x).max(hi_y - lo_y);
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx.hypot(dy);
    if !(length > 0.0 && length.is_finite()) {
        return None;
    }
    let (nx, ny) = (-dy / length, dx / length);
    let at = |l: f64| -> Option<((f64, f64), bool)> {
        let (cx, cy) = (from.0 + l * (to.0 - from.0), from.1 + l * (to.1 - from.1));
        let along = |m: f64| {
            let (px, py) = (cx + m * nx, cy + m * ny);
            if (0.0..=1.0).contains(&px) && (0.0..=1.0).contains(&py) {
                pair.f(px, py)
            } else {
                None
            }
        };
        let m = nearest_root(&along, room)?;
        let (qx, qy) = (cx + m * nx, cy + m * ny);
        if !(qx >= lo_x - room && qx <= hi_x + room && qy >= lo_y - room && qy <= hi_y + room) {
            return None;
        }
        let (tx, ty) = tangent_at(pair, qx, qy)?;
        // Still going forward in the parameter that falls at `to`.
        Some(((qx, qy), if falls_t { ty >= 0.0 } else { tx >= 0.0 }))
    };
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    let mut best = at(0.0).map(|(p, _)| p)?;
    for _ in 0..48 {
        let mid = 0.5 * (lo + hi);
        let (p, rising) = at(mid)?;
        if rising {
            lo = mid;
            best = p;
        } else {
            hi = mid;
        }
    }
    Some(best)
}

/// The direction the walk came from: the chord of its path over the last
/// two longest steps (or all of it, when shorter), in the unit square.
fn heading(path: &[(f64, f64)]) -> (f64, f64) {
    let Some(&(lx, ly)) = path.last() else {
        return (
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        );
    };
    let from = path
        .iter()
        .rev()
        .find(|(px, py)| (lx - px).hypot(ly - py) >= 2.0 * STEP_MAX)
        .or(path.first())
        .copied()
        .unwrap_or((lx, ly));
    let (dx, dy) = (lx - from.0, ly - from.1);
    let n = dx.hypot(dy);
    if n > 0.0 && n.is_finite() {
        (dx / n, dy / n)
    } else {
        (
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        )
    }
}

/// The second derivatives of `f` at `(x, y)` by central differences of
/// `f` itself (a step of [`HESSIAN_STEP`], the centre moved inside the
/// square where it is nearer a side than that).
fn hessian(pair: &Pair, x: f64, y: f64) -> Option<[f64; 3]> {
    let h = HESSIAN_STEP;
    let (x, y) = (x.clamp(h, 1.0 - h), y.clamp(h, 1.0 - h));
    let f = |dx: f64, dy: f64| pair.f(x + dx, y + dy);
    let c = f(0.0, 0.0)?;
    let fxx = (f(h, 0.0)? - 2.0 * c + f(-h, 0.0)?) / (h * h);
    let fyy = (f(0.0, h)? - 2.0 * c + f(0.0, -h)?) / (h * h);
    let fxy = (f(h, h)? - f(h, -h)? - f(-h, h)? + f(-h, -h)?) / (4.0 * h * h);
    [fxx, fxy, fyy]
        .iter()
        .all(|v| v.is_finite())
        .then_some([fxx, fxy, fyy])
}

/// The step of the differences [`hessian`] takes, in the unit square:
/// large enough that the rounding of `f` (about 1e-16) stays below 1e-8
/// of the second derivatives, small against a step of the walk.
const HESSIAN_STEP: f64 = 1e-4;

/// A crossing of two branches of the zero set of `f` (a simple
/// bifurcation point; Allgower & Georg, *Numerical Continuation Methods*,
/// ch. 8), exact or split into two arcs that pass each other by less than
/// the tolerance resolves, within one step of the walk at `(x, y)`: the
/// branch that goes on forward and nearest the direction the walk came
/// from, `heading`, is taken past it. This is where an exact developable's
/// rail inflects (a generalised cylinder whose section inflects): the
/// condition vanishes to first order on the ruling there and another
/// family of rulings crosses the developable's own. It is found from the
/// quadratic model of `f` at `(x, y)`: a saddle (the Hessian indefinite)
/// at `d* = −H⁻¹∇f`, its asymptotic directions the null directions of
/// `H`. Split arcs are taken as one crossing only when the model's value
/// at the saddle, a sine of the angle by which the tangent plane turns
/// along the ruling there, times the ruling's length, is within half the
/// tolerance: the rulings across are then developable within it (and the
/// fit holds them to it like every other). The walk steps to the saddle
/// and on along the chosen direction by `step` (or twice the saddle's
/// distance, if more), then back onto the zero set on the line square to
/// that direction (the root nearest the line's middle). `None` when there
/// is no such crossing (a fold of the zero set, where it truly turns
/// back; a saddle more than a step away or split by more than the
/// tolerance; no branch that goes on forward).
fn cross(
    pair: &Pair,
    (x, y): (f64, f64),
    heading: (f64, f64),
    step: f64,
    tolerance: f64,
) -> Option<(f64, f64)> {
    let model = model_saddle(pair, (x, y))?;
    let (dx, dy) = model.offset;
    let far = dx.hypot(dy);
    let (cx, cy) = (x + dx, y + dy);
    if !(far <= STEP_MAX && (0.0..=1.0).contains(&cx) && (0.0..=1.0).contains(&cy)) {
        return None;
    }
    let (pa, pb) = pair.points(cx, cy)?;
    let split = model.value.abs() * pa.distance(pb);
    if split.is_nan() || split > 0.5 * tolerance {
        return None;
    }
    past_saddle(
        pair,
        (x, y),
        (cx, cy),
        model.hessian,
        heading,
        step.max(2.0 * far),
    )
    .filter(|&q| !pair.crosses_corner((x, y), q))
}

/// The critical point of the quadratic model of `f` at a point where the
/// model is a saddle (its Hessian indefinite): `d* = −H⁻¹∇f` from the
/// point, the model's value there, and the Hessian `[f_xx, f_xy, f_yy]`.
#[derive(Clone, Copy, Debug)]
struct ModelSaddle {
    offset: (f64, f64),
    value: f64,
    hessian: [f64; 3],
}

fn model_saddle(pair: &Pair, (x, y): (f64, f64)) -> Option<ModelSaddle> {
    let value = pair.f(x, y)?;
    let (gx, gy) = pair.gradient(x, y)?;
    let [hxx, hxy, hyy] = hessian(pair, x, y)?;
    let det = hxx * hyy - hxy * hxy;
    if det.is_nan() || det >= 0.0 {
        return None;
    }
    let (dx, dy) = (-(hyy * gx - hxy * gy) / det, -(hxx * gy - hxy * gx) / det);
    let saddle_value = value + 0.5 * (gx * dx + gy * dy);
    (dx.is_finite() && dy.is_finite() && saddle_value.is_finite()).then_some(ModelSaddle {
        offset: (dx, dy),
        value: saddle_value,
        hessian: [hxx, hxy, hyy],
    })
}

/// Past the saddle of the model at `(cx, cy)`: along the asymptotic
/// direction of the model (a null direction of `H`) that goes on forward
/// and nearest `heading`, by `reach`, then back onto the zero set on the
/// line square to that direction (the root nearest the line's middle).
/// `None` when no branch goes on forward, or the point found is not
/// forward of `(x, y)` or not on the branch chosen.
fn past_saddle(
    pair: &Pair,
    (x, y): (f64, f64),
    (cx, cy): (f64, f64),
    [hxx, hxy, hyy]: [f64; 3],
    heading: (f64, f64),
    reach: f64,
) -> Option<(f64, f64)> {
    // The eigenvalues and eigenvectors of H (symmetric, one of each sign).
    let mean = 0.5 * (hxx + hyy);
    let radius = (0.5 * (hxx - hyy)).hypot(hxy);
    let (l1, l2) = (mean + radius, mean - radius);
    let angle = 0.5 * (2.0 * hxy).atan2(hxx - hyy);
    let (v1, v2) = ((angle.cos(), angle.sin()), (-angle.sin(), angle.cos()));
    let (a, b) = (l2.abs().sqrt(), l1.abs().sqrt());
    let norm = a.hypot(b);
    if !(norm > 0.0 && norm.is_finite() && l1 > 0.0 && l2 < 0.0) {
        return None;
    }
    let forward = |e: (f64, f64)| -> (f64, f64) { if e.0 + e.1 < 0.0 { (-e.0, -e.1) } else { e } };
    let candidates = [1.0_f64, -1.0].map(|sign| {
        forward((
            (a * v1.0 + sign * b * v2.0) / norm,
            (a * v1.1 + sign * b * v2.1) / norm,
        ))
    });
    let (ex, ey) = candidates
        .into_iter()
        .filter(|&(ex, ey)| ex >= -1e-9 && ey >= -1e-9)
        .max_by(|p, q| {
            (p.0 * heading.0 + p.1 * heading.1).total_cmp(&(q.0 * heading.0 + q.1 * heading.1))
        })?;
    // Stopped at the sides x = 1 or y = 1.
    let mut r = reach;
    if ex > 0.0 {
        r = r.min((1.0 - cx) / ex);
    }
    if ey > 0.0 {
        r = r.min((1.0 - cy) / ey);
    }
    let (qx, qy) = (cx + r * ex, cy + r * ey);
    // Back onto the zero set across the direction, the root nearest.
    let (nx, ny) = (-ey, ex);
    let along = |l: f64| {
        let (px, py) = (qx + l * nx, qy + l * ny);
        if (0.0..=1.0).contains(&px) && (0.0..=1.0).contains(&py) {
            pair.f(px, py)
        } else {
            None
        }
    };
    let l = nearest_root(&along, 0.5 * reach)?;
    let (px, py) = ((qx + l * nx).clamp(0.0, 1.0), (qy + l * ny).clamp(0.0, 1.0));
    // Forward of where the walk was, and on the branch it chose.
    if !(px >= x && py >= y && (px - x).hypot(py - y) > 0.0) {
        return None;
    }
    let on =
        tangent_at(pair, px, py).is_none_or(|(ux, uy)| (ux * ex + uy * ey).abs() >= MAX_TURN.cos());
    on.then_some((px, py))
}

/// The distance to a saddle of the model of `f` at `p` that lies ahead of
/// the walk (along `tangent`) within the step `h`, where the branches of
/// the zero set are split by more than [`cross`] steps over (the model's
/// value at the saddle times the ruling's length more than half the
/// tolerance).
///
/// Wherever two branches of the zero set pass each other (a saddle of
/// `f`, split), a predictor step whose length is comparable to their
/// distance can be corrected onto the other branch (Allgower & Georg,
/// *Numerical Continuation Methods*, ch. 8: branch jumping near a
/// bifurcation), and the fold where the walk's own branch turns is then
/// never seen. This is step control, not a proof: the walk keeps its step
/// to half the distance to the saddle the model at its point sees
/// ([`approach`]), so that the corrector stays on its branch; a step
/// accepted with such a saddle between its ends is retried shorter
/// ([`split_between`], the model at the step's middle, which sees what the
/// model at its start may not); and a step whose tangent turned by more
/// than [`MAX_TURN`] is retried too. A jump that gets past all three
/// lands on a branch whose rulings do not join those before it: no ruling
/// lies between the two on the zero set, and the fit refuses it ("the
/// rulings between two found ones cannot be followed"), or, where the
/// model sees the saddle between them ([`crossed`]), it holds the rulings
/// interpolated across to the twist every ruling is held to (half the
/// tolerance), refining or refusing: a refusal, not a wrong sheet,
/// whichever branch the walk then follows. (History: the sea kayak's sheer
/// panel, 2026-10-28, where steps of 1/128 of the square jumped between
/// branches passing 0.85 mm apart along the rails.) `None` when there is no
/// such saddle (none, behind, beyond the step, outside the square, or one
/// [`cross`] steps over).
fn saddle_ahead(
    pair: &Pair,
    p: (f64, f64),
    tangent: (f64, f64),
    h: f64,
    tolerance: f64,
) -> Option<f64> {
    let model = model_saddle(pair, p)?;
    let (dx, dy) = model.offset;
    let far = dx.hypot(dy);
    if !(far > 0.0 && far <= h) || dx * tangent.0 + dy * tangent.1 <= 0.0 {
        return None;
    }
    let c = (p.0 + dx, p.1 + dy);
    if !((0.0..=1.0).contains(&c.0) && (0.0..=1.0).contains(&c.1)) {
        return None;
    }
    let (pa, pb) = pair.points(c.0, c.1)?;
    (model.value.abs() * pa.distance(pb) > 0.5 * tolerance).then_some(far)
}

/// Whether a saddle of the model of `f` at the middle of `p` and `q` lies
/// between them where the branches are split by more than [`cross`]
/// steps over: a step from one to the other may have landed on the other
/// branch (the check behind [`saddle_ahead`], whose model at `p` may not
/// see so far).
fn split_between(pair: &Pair, p: (f64, f64), q: (f64, f64), tolerance: f64) -> bool {
    let middle = (0.5 * (p.0 + q.0), 0.5 * (p.1 + q.1));
    let Some(model) = model_saddle(pair, middle) else {
        return false;
    };
    let (dx, dy) = model.offset;
    let c = (middle.0 + dx, middle.1 + dy);
    let half = 0.5 * (q.0 - p.0).hypot(q.1 - p.1);
    if dx.hypot(dy).partial_cmp(&half) != Some(std::cmp::Ordering::Less) {
        return false;
    }
    pair.points(c.0, c.1)
        .is_some_and(|(pa, pb)| model.value.abs() * pa.distance(pb) > 0.5 * tolerance)
}

/// The root of `g` on `[−half, half]` nearest 0, by sampling and
/// bisection each way.
fn nearest_root(g: &dyn Fn(f64) -> Option<f64>, half: f64) -> Option<f64> {
    let up = root_between(g, 0.0, half);
    let down = root_between(&|l| g(-l), 0.0, half).map(|l| -l);
    match (up, down) {
        (Some(u), Some(d)) => Some(if u <= -d { u } else { d }),
        (Some(u), None) => Some(u),
        (None, Some(d)) => Some(d),
        (None, None) => None,
    }
}

/// The ruling between two found ones at `p` and `q` (unit square): the
/// zero of `f` on the line through their middle square to their chord,
/// the one nearest the middle, within half the chord's length of it (the
/// corrector of a pseudo-arclength continuation: it stays on the branch
/// the two lie on).
fn across_chord(pair: &Pair, p: (f64, f64), q: (f64, f64)) -> Option<(f64, f64)> {
    let (mx, my) = (0.5 * (p.0 + q.0), 0.5 * (p.1 + q.1));
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let length = dx.hypot(dy);
    if !(length > 0.0 && length.is_finite()) {
        return None;
    }
    let (nx, ny) = (-dy / length, dx / length);
    let along = |l: f64| {
        let (px, py) = (mx + l * nx, my + l * ny);
        if (0.0..=1.0).contains(&px) && (0.0..=1.0).contains(&py) {
            pair.f(px, py)
        } else {
            None
        }
    };
    let l = nearest_root(&along, 0.5 * length)?;
    Some((mx + l * nx, my + l * ny))
}

/// Whether the walk crossed between `p` and `q` (unit square): a saddle
/// of the quadratic model of `f` at their middle lies between them, the
/// place [`cross`] steps over.
fn crossed(pair: &Pair, p: (f64, f64), q: (f64, f64)) -> bool {
    let (mx, my) = (0.5 * (p.0 + q.0), 0.5 * (p.1 + q.1));
    let (Some((gx, gy)), Some([hxx, hxy, hyy])) = (pair.gradient(mx, my), hessian(pair, mx, my))
    else {
        return false;
    };
    let det = hxx * hyy - hxy * hxy;
    if det.is_nan() || det >= 0.0 {
        return false;
    }
    let (dx, dy) = (-(hyy * gx - hxy * gy) / det, -(hxx * gy - hxy * gx) / det);
    let length = (q.0 - p.0).hypot(q.1 - p.1);
    dx.hypot(dy) <= length
}

/// A zero of `g` between `a` and `b` (both in `[0, 1]`), by sampling and
/// bisection; the one nearest `a`.
fn root_between(g: &dyn Fn(f64) -> Option<f64>, a: f64, b: f64) -> Option<f64> {
    if b <= a {
        return None;
    }
    let samples = 16;
    let mut before = (a, g(a)?);
    if before.1 == 0.0 {
        return Some(a);
    }
    for k in 1..=samples {
        let x = a + (b - a) * k as f64 / samples as f64;
        let value = g(x)?;
        if value == 0.0 || (value < 0.0) != (before.1 < 0.0) {
            let (mut lo, mut hi, mut vlo) = (before.0, x, before.1);
            for _ in 0..80 {
                let m = 0.5 * (lo + hi);
                let vm = g(m)?;
                if (vm < 0.0) == (vlo < 0.0) {
                    lo = m;
                    vlo = vm;
                } else {
                    hi = m;
                }
            }
            return Some(0.5 * (lo + hi));
        }
        before = (x, value);
    }
    None
}

/// The plane both rails lie in, if they do (every sampled point within
/// the tolerance of it).
fn common_plane(a: &Rail, b: &Rail, tolerance: f64) -> Option<Frame> {
    let mut points = Vec::new();
    for rail in [a, b] {
        let n = rail.pieces.len() * 32;
        for k in 0..=n {
            points.push(rail.point(rail.len() * k as f64 / n as f64)?);
        }
    }
    let p0 = *points.first()?;
    let p1 = *points
        .iter()
        .max_by(|p, q| p.distance(p0).total_cmp(&q.distance(p0)))?;
    let axis = (p1 - p0).normalized()?;
    let off = |p: &Vec3| {
        let d = *p - p0;
        (d - axis * d.dot(axis)).length()
    };
    let p2 = *points.iter().max_by(|p, q| off(p).total_cmp(&off(q)))?;
    let normal = axis.cross(p2 - p0).normalized()?;
    if points
        .iter()
        .all(|p| (*p - p0).dot(normal).abs() <= 0.5 * tolerance)
    {
        Frame::new(p0, normal, axis)
    } else {
        None
    }
}

/// One row of a face: a stretch of a rail, or one point of it (the apex
/// of a fan).
#[derive(Clone, Copy, Debug)]
struct Row {
    /// 0 for A, 1 for B.
    side: usize,
    from: f64,
    to: f64,
}

impl Row {
    fn is_point(&self) -> bool {
        self.to <= self.from
    }
}

/// One face to build: its rows, and for a face of rulings the rulings.
struct Strip {
    piece: Piece,
    lower: Row,
    upper: Row,
    /// `(s, t)` rail parameters of the rulings the face is fitted
    /// through, for a face of rulings; empty for a fan (fitted along its
    /// stretch).
    rulings: Vec<(f64, f64)>,
}

impl Rail {
    /// The same chain run the other way.
    pub fn reversed(&self) -> Rail {
        Rail {
            pieces: self
                .pieces
                .iter()
                .rev()
                .map(|p| RailPiece {
                    reversed: !p.reversed,
                    ..p.clone()
                })
                .collect(),
        }
    }
}

impl Rail {
    /// A closed rail started at `t` (a rail parameter): the piece there
    /// split at it, the chain run on from there round to it. `None` when
    /// `t` is not inside the rail.
    fn rotated(&self, t: f64) -> Option<Rail> {
        let n = self.pieces.len();
        if !(t.is_finite() && t >= 0.0 && t < n as f64) {
            return None;
        }
        let index = (t.floor() as usize).min(n.checked_sub(1)?);
        let local = t - index as f64;
        let piece = self.pieces.get(index)?;
        let mut pieces = Vec::with_capacity(n + 1);
        if local <= 0.0 {
            pieces.extend(self.pieces.iter().skip(index).cloned());
            pieces.extend(self.pieces.iter().take(index).cloned());
            return Some(Rail { pieces });
        }
        let (a, b) = piece.range;
        let cut = if piece.reversed {
            b - local * (b - a)
        } else {
            a + local * (b - a)
        };
        // In the chain's direction: the part from `t` on, then the part up
        // to it.
        let (after, before) = if piece.reversed {
            ((a, cut), (cut, b))
        } else {
            ((cut, b), (a, cut))
        };
        pieces.push(RailPiece {
            range: after,
            ..piece.clone()
        });
        pieces.extend(self.pieces.iter().skip(index + 1).cloned());
        pieces.extend(self.pieces.iter().take(index).cloned());
        pieces.push(RailPiece {
            range: before,
            ..piece.clone()
        });
        Some(Rail { pieces })
    }
}

/// Most starts on the second rail a tube is tried from ([`tube`]).
const MAX_TUBE_STARTS: usize = 16;

/// Most points a closed rail is sampled at for the checks of a tube
/// ([`loop_samples`]).
const MAX_LOOP_SAMPLES: usize = 4096;

/// Points along a rail, evenly in its parameter: 64 per piece, at most
/// [`MAX_LOOP_SAMPLES`], both ends included.
fn loop_samples(rail: &Rail) -> Vec<(f64, Vec3)> {
    let n = rail
        .pieces
        .len()
        .saturating_mul(64)
        .clamp(16, MAX_LOOP_SAMPLES);
    (0..=n)
        .filter_map(|k| {
            let s = rail.len() * k as f64 / n as f64;
            rail.point(s).map(|p| (s, p))
        })
        .collect()
}

/// The distance from `p` to `rail` and the parameter where it is
/// nearest: from the nearest of `samples`, refined by Newton within the
/// stretches beside it.
fn distance_to(rail: &Rail, samples: &[(f64, Vec3)], p: Vec3) -> Option<(f64, f64)> {
    let (k, _) = samples
        .iter()
        .enumerate()
        .min_by(|x, y| x.1.1.distance(p).total_cmp(&y.1.1.distance(p)))?;
    let (s, _) = *samples.get(k)?;
    let lo = k
        .checked_sub(1)
        .and_then(|j| samples.get(j))
        .map_or(s, |x| x.0);
    let hi = samples.get(k + 1).map_or(s, |x| x.0);
    let at = rail.nearest(p, s, lo, hi);
    let d = rail.point(at)?.distance(p);
    d.is_finite().then_some((d, at))
}

/// Where two closed rails come within the tolerance of each other, if
/// they do: from each local minimum of the distance of `b`'s samples to
/// `a`, the nearest points of the two curves are found by projecting from
/// one onto the other in turn (which converges to where two curves cross,
/// as well as to where they touch).
fn touching(
    a: &Rail,
    samples_a: &[(f64, Vec3)],
    b: &Rail,
    samples_b: &[(f64, Vec3)],
    tol: f64,
) -> Option<Vec3> {
    let d: Vec<Option<(f64, f64)>> = samples_b
        .iter()
        .map(|(_, p)| distance_to(a, samples_a, *p))
        .collect();
    let n = d.len();
    for k in 0..n {
        let here = d.get(k).copied().flatten();
        let Some((dk, sk)) = here else {
            continue;
        };
        // The samples wrap round (the first and the last are one point).
        let before = d
            .get(if k == 0 { n.saturating_sub(2) } else { k - 1 })
            .copied()
            .flatten()
            .map_or(f64::INFINITY, |x| x.0);
        let after = d
            .get(if k + 1 >= n { 1 } else { k + 1 })
            .copied()
            .flatten()
            .map_or(f64::INFINITY, |x| x.0);
        if dk > before || dk > after {
            continue;
        }
        let (Some(&(t0, _)), step) = (samples_b.get(k), b.len() / samples_b.len().max(1) as f64)
        else {
            continue;
        };
        let step_a = a.len() / samples_a.len().max(1) as f64;
        let (mut s, mut t) = (sk, t0);
        for _ in 0..64 {
            let (Some(pa), Some(pb)) = (a.point(s), b.point(t)) else {
                break;
            };
            if pa.distance(pb) <= tol {
                return Some(pa);
            }
            t = b.nearest(pa, t, (t - step).max(0.0), (t + step).min(b.len()));
            let Some(pb) = b.point(t) else {
                break;
            };
            s = a.nearest(pb, s, (s - step_a).max(0.0), (s + step_a).min(a.len()));
        }
    }
    None
}

/// The winding number of the closed rail `rail` about the point `q`, in
/// the plane of `frame`, where `q` lies `d` (more than zero) from the
/// rail: the sum of the angles the rail turns through about `q`, over
/// stretches shorter than `d / 2`, along each of which the angle is that
/// between its ends (a stretch shorter than its distance from `q` does not
/// pass round it). `None` when that needs more than `2¹⁶` stretches.
fn winding(rail: &Rail, frame: &Frame, q: Vec3, d: f64) -> Option<i64> {
    if !(d > 0.0 && d.is_finite()) {
        return None;
    }
    let (qx, qy, _) = frame.local(q);
    let flat = |s: f64| -> Option<(f64, f64)> {
        let (x, y, _) = frame.local(rail.point(s)?);
        Some((x - qx, y - qy))
    };
    let mut total = 0.0;
    let mut stack: Vec<(f64, f64)> = (0..rail.pieces.len())
        .rev()
        .map(|k| (k as f64, k as f64 + 1.0))
        .collect();
    let mut count = 0usize;
    while let Some((s0, s1)) = stack.pop() {
        count += 1;
        if count > 1 << 16 {
            return None;
        }
        if rail.length_between(s0, s1) >= 0.5 * d {
            let m = 0.5 * (s0 + s1);
            stack.push((m, s1));
            stack.push((s0, m));
            continue;
        }
        let (u, w) = (flat(s0)?, flat(s1)?);
        total += (u.0 * w.1 - u.1 * w.0).atan2(u.0 * w.0 + u.1 * w.1);
    }
    let turns = total / std::f64::consts::TAU;
    turns.is_finite().then(|| turns.round() as i64)
}

/// The signed area a closed rail's samples enclose in the plane of
/// `frame` (positive when they run counterclockwise about its z).
fn signed_area(samples: &[(f64, Vec3)], frame: &Frame) -> f64 {
    let flat: Vec<(f64, f64)> = samples
        .iter()
        .map(|(_, p)| {
            let (x, y, _) = frame.local(*p);
            (x, y)
        })
        .collect();
    0.5 * flat
        .windows(2)
        .map(|w| match w {
            [(x0, y0), (x1, y1)] => x0 * y1 - x1 * y0,
            _ => 0.0,
        })
        .sum::<f64>()
}

/// The developable tube between two closed rails (2026-10-28). The square
/// of the two rails' parameters is a torus here, and the developable is a
/// path along the zero set of the condition that goes once round both:
/// from a ruling `A(0) B(t*)` that satisfies the condition, round to the
/// same ruling. `B` is started at `t*` ([`Rail::rotated`]), so the walk
/// starts at the corner of the square on the zero set, and the tube is
/// built as a sheet whose last ruling is its first (one edge, its seam,
/// used by the faces on both sides of it; the rails' ends one vertex
/// each). Where the walk reaches the end of one rail before the other, the
/// rest is a fan from that end, which closes the tube as smoothly as at
/// the ends of an open sheet (its first ruling satisfies the condition and
/// its last is the seam). Of the rulings from `A(0)` that satisfy the
/// condition (the zeros of `f(0, ·)`), the shortest is tried first, then
/// the others in order of length, at most [`MAX_TUBE_STARTS`]: each that
/// goes round gives a developable (two coaxial circles: the frustum's
/// rulings, and the ones across the axis), and the shortest is the one a
/// tube between two loops is. The seam lies at `A`'s start, which must be
/// smooth: where `A` closes at a corner (a loop's start is where its
/// curves were found to close, not chosen), it is started inside its first
/// piece instead ([`Rail::rotated`], as `B` is), and the corner is then
/// one like any other along it. Rails in one plane give the flat ring
/// between them, cut along the shortest segment from `A(0)` to `B`.
fn tube(a: &Rail, b: &Rail, tolerance: &Tolerance) -> Result<Connected, KernelError> {
    let tol = tolerance.linear;
    let closes_at_corner = |rail: &Rail| -> Result<bool, KernelError> {
        let (Some((_, d_start)), Some((_, d_end))) =
            (rail.at_side(0.0, false), rail.at_side(rail.len(), false))
        else {
            return Err(no_point());
        };
        let turn = |u: Vec3, w: Vec3| u.cross(w).length().atan2(u.dot(w));
        Ok(match (d_end.normalized(), d_start.normalized()) {
            (Some(u), Some(w)) => turn(u, w) > tolerance.angular,
            _ => false,
        })
    };
    let started;
    let a = if closes_at_corner(a)? {
        started = a.rotated(0.5).ok_or_else(no_point)?;
        if closes_at_corner(&started)? {
            let p = started.point(0.0).ok_or_else(no_point)?;
            return Err(refused(format!(
                "the first curve has a corner in the middle of its first piece too, at {}: a tube's seam needs a smooth point to start from",
                fmt_point(p)
            )));
        }
        &started
    } else {
        a
    };
    let (samples_a, samples_b) = (loop_samples(a), loop_samples(b));
    let farthest = |from: &[(f64, Vec3)], to: &Rail, to_samples: &[(f64, Vec3)]| {
        from.iter()
            .filter_map(|(_, p)| distance_to(to, to_samples, *p).map(|(d, _)| (d, *p)))
            .max_by(|p, q| p.0.total_cmp(&q.0))
    };
    let (Some((far_b, qb)), Some((far_a, qa))) = (
        farthest(&samples_b, a, &samples_a),
        farthest(&samples_a, b, &samples_b),
    ) else {
        return Err(no_point());
    };
    if far_b <= tol && far_a <= tol {
        return Err(refused(
            "the two closed curves are the same (each lies within the tolerance of the other)"
                .to_string(),
        ));
    }
    let a0 = a.point(0.0).ok_or_else(no_point)?;
    let lengths = (
        a.length_between(0.0, a.len()),
        b.length_between(0.0, b.len()),
    );
    if let Some(frame) = common_plane(a, b, tol) {
        // A flat ring lies between two loops one inside the other that do
        // not touch, run the same way round.
        if let Some(at) = touching(a, &samples_a, b, &samples_b, tol) {
            return Err(refused(format!(
                "the two closed curves lie in one plane and touch or cross each other at {}: a flat ring lies between two loops one inside the other",
                fmt_point(at)
            )));
        }
        let (Some(around_b), Some(around_a)) =
            (winding(a, &frame, qb, far_b), winding(b, &frame, qa, far_a))
        else {
            return Err(refused(
                "the two closed curves lie in one plane, and whether one encloses the other could not be told".to_string(),
            ));
        };
        if around_b == 0 && around_a == 0 {
            return Err(refused(
                "the two closed curves lie in one plane and neither encloses the other: a flat ring lies between two loops one inside the other".to_string(),
            ));
        }
        let (area_a, area_b) = (
            signed_area(&samples_a, &frame),
            signed_area(&samples_b, &frame),
        );
        if (area_a * area_b).partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return Err(refused(
                "the two closed curves lie in one plane and run opposite ways round: draw them the same way round for the flat ring between them".to_string(),
            ));
        }
        // The point of B nearest A(0), from the nearest of its samples.
        let samples = b.pieces.len() * 64;
        let start = (0..samples)
            .map(|k| b.len() * k as f64 / samples as f64)
            .filter_map(|t| b.point(t).map(|p| (t, p.distance(a0))))
            .min_by(|p, q| p.1.total_cmp(&q.1))
            .map(|(t, _)| t)
            .ok_or_else(no_point)?;
        let step = b.len() / samples as f64;
        let t = b.nearest(
            a0,
            start,
            (start - step).max(0.0),
            (start + step).min(b.len()),
        );
        let t = if t >= b.len() { 0.0 } else { t };
        let b = b.rotated(t).ok_or_else(no_point)?;
        return planar(a, &b, frame, tolerance, [false, false], true);
    }
    let pair = Pair::new(a, b, tolerance.angular);
    // The rulings from A(0) that satisfy the condition, shortest first.
    let mut starts: Vec<(f64, f64)> = Vec::new();
    let mut from = 0.0;
    if pair.f(0.0, 0.0).is_some_and(|v| v == 0.0) {
        starts.push((0.0, a0.distance(b.point(0.0).ok_or_else(no_point)?)));
        from = 1e-9;
    }
    while starts.len() < 4 * MAX_TUBE_STARTS {
        let Some(y) = first_root(&|y| pair.f(0.0, y), from) else {
            break;
        };
        if y >= 1.0 {
            break;
        }
        let length = b
            .point(pair.tb(y))
            .map_or(f64::INFINITY, |p| p.distance(a0));
        starts.push((y, length));
        from = y + 1e-9;
    }
    starts.sort_by(|p, q| p.1.total_cmp(&q.1));
    let mut first_error = None;
    for (y, length) in starts.into_iter().take(MAX_TUBE_STARTS) {
        if length <= tol {
            continue;
        }
        let Some(rotated) = b.rotated(pair.tb(y)) else {
            continue;
        };
        let mut turned = Pair::new(a, &rotated, tolerance.angular);
        turned.closed = true;
        match along_pair(&turned, tolerance, [false, false], lengths) {
            Ok(made) => return Ok(made),
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    Err(first_error.unwrap_or_else(|| {
        refused(format!(
            "no ruling from the start of the first curve, {}, to the second satisfies the condition (tangents and ruling in one plane): these closed curves have no developable tube between them",
            fmt_point(a0)
        ))
    }))
}

/// The sheet between the rails `a` and `b`. `a` is run as given; `b` as
/// given, or the other way when only that gives a developable (rails
/// drawn in opposite directions; with both ways possible, as given, so a
/// sheet that builds keeps its shape and its faces' names).
pub fn connect(a: &Rail, b: &Rail, tolerance: &Tolerance) -> Result<Connected, KernelError> {
    match connect_as_given(a, b, tolerance) {
        Err(first @ KernelError::Refused { .. }) => {
            match connect_as_given(a, &b.reversed(), tolerance) {
                Ok(made) => Ok(Connected {
                    reversed_b: true,
                    ..made
                }),
                // Two closed rails: either way round is a tube of its own,
                // and why each was refused can differ (the way given may
                // run opposite to A, the other way fail for another
                // reason): both are told.
                Err(KernelError::Refused {
                    reason: other_way, ..
                }) if closed(a, tolerance) && closed(b, tolerance) => {
                    let KernelError::Refused { reason, .. } = first else {
                        return Err(first);
                    };
                    if reason == other_way {
                        Err(refused(reason))
                    } else {
                        Err(refused(format!(
                            "{reason}; with the second curve run the other way round: {other_way}"
                        )))
                    }
                }
                Err(_) => Err(first),
            }
        }
        other => other,
    }
}

/// Whether a rail ends where it starts, within the tolerance.
fn closed(rail: &Rail, tolerance: &Tolerance) -> bool {
    match (rail.point(0.0), rail.point(rail.len())) {
        (Some(p), Some(q)) => p.distance(q) <= tolerance.linear,
        _ => false,
    }
}

fn connect_as_given(a: &Rail, b: &Rail, tolerance: &Tolerance) -> Result<Connected, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    a.check("the first curve", tolerance)?;
    b.check("the second curve", tolerance)?;
    let (a0, a1) = (
        a.point(0.0).ok_or_else(no_point)?,
        a.point(a.len()).ok_or_else(no_point)?,
    );
    let (b0, b1) = (
        b.point(0.0).ok_or_else(no_point)?,
        b.point(b.len()).ok_or_else(no_point)?,
    );
    let (length_a, length_b) = (
        a.length_between(0.0, a.len()),
        b.length_between(0.0, b.len()),
    );
    if !(length_a > tol && length_b > tol) {
        return Err(refused(format!(
            "a curve is no longer than the tolerance (lengths {length_a} and {length_b})"
        )));
    }
    match (a0.distance(a1) <= tol, b0.distance(b1) <= tol) {
        (true, true) => return tube(a, b, tolerance),
        (false, false) => {}
        (closed_a, _) => {
            let (name, at) = if closed_a {
                ("first", a0)
            } else {
                ("second", b0)
            };
            return Err(refused(format!(
                "the {name} curve is closed (it ends where it starts, at {}) and the other is not: a developable joins two open curves, or two closed ones (a tube)",
                fmt_point(at)
            )));
        }
    }
    let tips = [a0.distance(b0) <= tol, a1.distance(b1) <= tol];
    if tips[0] && tips[1] && a.len() == b.len() && a == b {
        return Err(refused("the two curves are the same".to_string()));
    }
    if let Some(frame) = common_plane(a, b, tol) {
        return planar(a, b, frame, tolerance, tips, false);
    }
    let pair = Pair::new(a, b, tolerance.angular);
    along_pair(&pair, tolerance, tips, (length_a, length_b))
}

/// The developable of a pair of rails that do not lie in one plane: the
/// walk from the start, the faces, the fit. For a pair of closed rails
/// (`pair.closed`, a tube) the walk starts at the corner of the square,
/// on the zero set by construction ([`tube`]).
fn along_pair(
    pair: &Pair,
    tolerance: &Tolerance,
    tips: [bool; 2],
    (length_a, length_b): (f64, f64),
) -> Result<Connected, KernelError> {
    let tol = tolerance.linear;
    let (a, b) = (pair.a, pair.b);

    // The start: a fan from an end to the first ruling that satisfies the
    // condition, unless the corner's ruling does.
    let start = if pair.closed {
        (Start::Corner, None)
    } else if tips[0] {
        // Ends that meet: the zero set leaves the corner along a line
        // (`α s² = β t²` to second order); find it on a small arc.
        // A radius of about a hundred tolerances along the rails.
        let r0 = (REACH * tol / length_a.min(length_b)).clamp(1e-9, 1e-2);
        let along = |phi: f64| pair.f(r0 * phi.cos(), r0 * phi.sin());
        match first_root(&|u| along(u * std::f64::consts::FRAC_PI_2), 0.0) {
            Some(u) => {
                let phi = u * std::f64::consts::FRAC_PI_2;
                (Start::Corner, Some((r0 * phi.cos(), r0 * phi.sin())))
            }
            None => (start_fan(pair)?.0, None),
        }
    } else if corner_reach(pair) <= tol {
        // One test at both ends: a fan that would cover no more than the
        // tolerance of its rail is no fan, and the first ruling is the
        // corner's (as the last is snapped to the far corner, below).
        // The root by the corner is found by a Newton step from it along
        // each side: a sampled search of a side cannot see a root that
        // lies on the corner itself.
        (Start::Corner, None)
    } else {
        // A fan within the tolerance of the strip that would run on to
        // the corner instead is no fan either ([`fan_within_tolerance`]).
        match start_fan(pair)? {
            (_, covered) if covered <= tol => (Start::Corner, Some((0.0, 0.0))),
            (Start::FromA(y), _) if fan_within_tolerance(pair, (0.0, y), (0.0, 0.0), tol) => {
                (Start::Corner, Some((0.0, 0.0)))
            }
            (Start::FromB(x), _) if fan_within_tolerance(pair, (x, 0.0), (0.0, 0.0), tol) => {
                (Start::Corner, Some((0.0, 0.0)))
            }
            (kind, _) => (kind, None),
        }
    };
    let (kind, first) = start;
    let snapped = matches!(first, Some((0.0, 0.0)));
    let origin = match kind {
        Start::Corner if snapped => start_root(pair)?,
        Start::Corner => first.unwrap_or((0.0, 0.0)),
        Start::FromA(y) => (0.0, y),
        Start::FromB(x) => (x, 0.0),
    };
    let mut path = merged_fans(trace(pair, origin, tol, tips, REACH * tol)?);
    if snapped {
        if let Some(head) = path.points.first_mut() {
            *head = (0.0, 0.0);
        }
    } else if first.is_some() {
        path.points.insert(0, (0.0, 0.0));
        path.fan_after.insert(0, false);
        path.break_at.insert(0, false);
    }
    let &(ex, ey) = path.points.last().ok_or_else(no_point)?;
    // The end: the rest of a rail as a fan from the other's last end,
    // unless what is left is within the tolerance.
    let rest_a = a.length_between(pair.sa(ex), a.len());
    let rest_b = b.length_between(pair.tb(ey), b.len());
    // Or the fan the rest would be lies within the tolerance of the strip
    // run on to the far corner (as at the start), unless the path ends in
    // a fan at a fold, whose parameter it would change.
    let ends_in_fan = path
        .fan_after
        .len()
        .checked_sub(2)
        .and_then(|k| path.fan_after.get(k))
        .copied()
        .unwrap_or(false);
    let rest_within = !ends_in_fan
        && if ex >= 1.0 {
            fan_within_tolerance(pair, (1.0, ey), (1.0, 1.0), tol)
        } else {
            fan_within_tolerance(pair, (ex, 1.0), (1.0, 1.0), tol)
        };
    let end = if (rest_a <= tol && rest_b <= tol) || rest_within {
        // Snapped to the far corner; a fan that ends there would change
        // the parameter it is at (and so its kind): refused.
        let n = path.points.len();
        if n >= 2 && path.fan_after.get(n - 2).copied().unwrap_or(false) {
            return Err(refused(format!(
                "a fan at a fold of the condition reaches the ends of both curves {}: not built",
                pair.where_(ex, ey)
            )));
        }
        if let Some(last) = path.points.last_mut() {
            *last = (1.0, 1.0);
        }
        None
    } else if ex >= 1.0 {
        Some(Start::FromA(ey))
    } else {
        Some(Start::FromB(ex))
    };
    let (path, kind, end) = merged_end_fans(path, kind, end);
    let path = thinned(pair, path, tol);
    if path.points.len() < 2 {
        return Err(refused(format!(
            "no rulings satisfy the condition between the curves {}",
            pair.where_(origin.0, origin.1)
        )));
    }
    let rulings: Vec<(f64, f64)> = path
        .points
        .iter()
        .map(|&(x, y)| (pair.sa(x), pair.tb(y)))
        .collect();
    let (&(sa, ta), &(sb, tb)) = (
        rulings.first().ok_or_else(no_point)?,
        rulings.last().ok_or_else(no_point)?,
    );
    let mut strips = Vec::new();
    match kind {
        Start::FromA(_) => strips.push(Strip {
            piece: Piece::StartFan,
            lower: Row {
                side: 0,
                from: 0.0,
                to: 0.0,
            },
            upper: Row {
                side: 1,
                from: 0.0,
                to: ta,
            },
            rulings: Vec::new(),
        }),
        Start::FromB(_) => strips.push(Strip {
            piece: Piece::StartFan,
            lower: Row {
                side: 0,
                from: 0.0,
                to: sa,
            },
            upper: Row {
                side: 1,
                from: 0.0,
                to: 0.0,
            },
            rulings: Vec::new(),
        }),
        Start::Corner => {}
    }
    strips.extend(middle_strips(
        pair,
        &rulings,
        &path.fan_after,
        &path.break_at,
    ));
    match end {
        Some(Start::FromA(_)) => strips.push(Strip {
            piece: Piece::EndFan,
            lower: Row {
                side: 0,
                from: a.len(),
                to: a.len(),
            },
            upper: Row {
                side: 1,
                from: tb,
                to: b.len(),
            },
            rulings: Vec::new(),
        }),
        Some(Start::FromB(_)) => strips.push(Strip {
            piece: Piece::EndFan,
            lower: Row {
                side: 0,
                from: sb,
                to: a.len(),
            },
            upper: Row {
                side: 1,
                from: b.len(),
                to: b.len(),
            },
            rulings: Vec::new(),
        }),
        _ => {}
    }
    build(pair, strips, tolerance, tips)
}

/// The path's points as the faces are fitted through them:
///
/// - Points within the tolerance of the one before (or of the last) are
///   one point: a snapped end leaves no sliver or backward step smaller
///   than the tolerance beside it.
/// - A point that leaves a stretch shorter than half the one before it is
///   dropped, and so is the point before the last while the last stretch
///   is shorter than half the one before it. Where the walk crept (steps
///   halved by a turn, or short at a side where it stopped), its points
///   lie far closer together than those beside them, and an interpolation
///   through spacings that unequal overshoots between them by more than
///   any refinement takes back (the sea kayak's panels, 2026-10-27:
///   rulings 2e-6 apart beside rulings 3e-3 apart). The fit puts rulings
///   back where they are needed, at the middles of the stretches, which
///   keeps neighbouring stretches within a factor of two.
///
/// The ends of a fan are kept as they are. Lengths are the moves of the
/// rulings' ends along both rails, as in the fit.
fn thinned(pair: &Pair, path: Path, tol: f64) -> Path {
    let moved = |p: (f64, f64), q: (f64, f64)| -> f64 {
        match (pair.points(p.0, p.1), pair.points(q.0, q.1)) {
            (Some((a0, b0)), Some((a1, b1))) => a0.distance(a1) + b0.distance(b1),
            _ => f64::INFINITY,
        }
    };
    let count = path.points.len();
    if count < 2 {
        return path;
    }
    let (Some(&head), Some(&tail)) = (path.points.first(), path.points.last()) else {
        return path;
    };
    let mut out = Path::default();
    out.push(head);
    let mut fixed_out = vec![path.fixed(0)];
    // The length of the last stretch kept that is not a fan.
    let mut last_stretch: Option<f64> = None;
    for k in 1..count.saturating_sub(1) {
        let Some(&q) = path.points.get(k) else {
            continue;
        };
        let last = out.points.last().copied().unwrap_or(head);
        let fixed = path.fixed(k);
        let after_fan = out.fan_after.last().copied().unwrap_or(false);
        let length = moved(last, q);
        let keep = fixed
            || after_fan
            || (length > tol
                && moved(q, tail) > tol
                && last_stretch.is_none_or(|before| length >= 0.5 * before));
        if keep {
            last_stretch = (!after_fan).then_some(length);
            out.push(q);
            fixed_out.push(fixed);
            if path.fan_after.get(k).copied().unwrap_or(false)
                && let Some(f) = out.fan_after.last_mut()
            {
                *f = true;
            }
            if path.break_at.get(k).copied().unwrap_or(false) {
                out.mark_break();
            }
        }
    }
    out.push(tail);
    fixed_out.push(path.fixed(count - 1));
    // The last stretch against the one before it.
    loop {
        let n = out.points.len();
        if n < 3 {
            break;
        }
        let (Some(&p0), Some(&p1), Some(&p2)) = (
            out.points.get(n - 3),
            out.points.get(n - 2),
            out.points.get(n - 1),
        ) else {
            break;
        };
        let fixed = fixed_out.get(n - 2).copied().unwrap_or(true)
            || out.special_after(n - 3)
            || out.special_after(n - 2);
        if fixed || moved(p1, p2) >= 0.5 * moved(p0, p1) {
            break;
        }
        out.points.remove(n - 2);
        out.fan_after.remove(n - 2);
        out.break_at.remove(n - 2);
        fixed_out.remove(n - 2);
    }
    out
}

/// What lies between two runs of rulings while the faces are put in order
/// ([`middle_strips`]): a fan (at a fold or from a corner), or a parting at
/// a corner (no fan, or a crease).
enum Item {
    Run(Strip),
    Fan(Strip, Place),
    Part(Place),
}

/// Where a fan or a parting is, before the folds are counted: a fold at
/// the given place along the sheet (the mean of the unit parameters of
/// its first ruling), or a corner of a rail ([`Mark::Corner`]).
#[derive(Clone, Copy, Debug)]
enum Place {
    Fold(f64),
    Corner { rail: u8, source: u64 },
}

/// The corner of `rail` (0 for A, 1 for B) at the rail parameter `s`, if
/// `s` is one of its corners: the [`Mark::Corner`] of the piece after it.
fn corner_at(pair: &Pair, rail: u8, s: f64) -> Option<Place> {
    let (r, corners) = if rail == 0 {
        (pair.a, &pair.corners_a)
    } else {
        (pair.b, &pair.corners_b)
    };
    let len = r.len();
    let joint = s.round();
    if (s - joint).abs() > 1e-9 * len.max(1.0)
        || !corners
            .iter()
            .any(|c| (c * len - joint).abs() <= 1e-9 * len.max(1.0))
    {
        return None;
    }
    let piece = r.pieces.get(joint as usize)?;
    Some(Place::Corner {
        rail,
        source: piece.source,
    })
}

/// The faces between the start and the end: the path split at its fans
/// and where the faces part, each run of rulings a face and each fan a
/// face, named by place ([`Piece`]): the first run of rulings is
/// [`Piece::Rulings`] and the run that reaches the end [`Piece::Last`]; a
/// fan from a corner is named by the corner, a fan at a fold by its count
/// from the nearer end of the sheet (the folds in the first half from the
/// start, those in the second half from the end); a run between them by
/// the corner or the fold counted from the start before it, or else by
/// the fold counted from the end after it. A fold that appears or goes at
/// one end, or between, so leaves the names at the other end as they
/// were, and the faces that hold the sheet's ends keep theirs.
fn middle_strips(
    pair: &Pair,
    rulings: &[(f64, f64)],
    fan_after: &[bool],
    break_at: &[bool],
) -> Vec<Strip> {
    let mut items: Vec<Item> = Vec::new();
    let mut run: Vec<(f64, f64)> = Vec::new();
    let close = |run: &mut Vec<(f64, f64)>, items: &mut Vec<Item>| {
        if let (Some(&(s0, t0)), Some(&(s1, t1)), true) = (run.first(), run.last(), run.len() >= 2)
        {
            items.push(Item::Run(Strip {
                piece: Piece::Rulings,
                lower: Row {
                    side: 0,
                    from: s0,
                    to: s1,
                },
                upper: Row {
                    side: 1,
                    from: t0,
                    to: t1,
                },
                rulings: std::mem::take(run),
            }));
        }
        run.clear();
    };
    let unit = |(s, t): (f64, f64)| 0.5 * (s / pair.a.len() + t / pair.b.len());
    for (k, &p) in rulings.iter().enumerate() {
        run.push(p);
        // The faces part here: the run ends and the next starts at `p`.
        if break_at.get(k).copied().unwrap_or(false)
            && !fan_after.get(k).copied().unwrap_or(false)
            && k + 1 < rulings.len()
        {
            close(&mut run, &mut items);
            let place = corner_at(pair, 0, p.0)
                .or_else(|| corner_at(pair, 1, p.1))
                .unwrap_or(Place::Fold(unit(p)));
            items.push(Item::Part(place));
            run.push(p);
            continue;
        }
        if fan_after.get(k).copied().unwrap_or(false)
            && let Some(&q) = rulings.get(k + 1)
        {
            close(&mut run, &mut items);
            // At one `t` (apex on B) or at one `s` (apex on A).
            let apex_on_b = p.1 == q.1;
            let (lower, upper) = if apex_on_b {
                ((p.0, q.0), (p.1, p.1))
            } else {
                ((p.0, p.0), (p.1, q.1))
            };
            let place = if apex_on_b {
                corner_at(pair, 1, p.1)
            } else {
                corner_at(pair, 0, p.0)
            }
            .unwrap_or(Place::Fold(unit(p)));
            items.push(Item::Fan(
                Strip {
                    piece: Piece::Rulings,
                    lower: Row {
                        side: 0,
                        from: lower.0,
                        to: lower.1,
                    },
                    upper: Row {
                        side: 1,
                        from: upper.0,
                        to: upper.1,
                    },
                    rulings: Vec::new(),
                },
                place,
            ));
        }
    }
    close(&mut run, &mut items);
    // The marks: folds in the first half counted from the start, those in
    // the second from the end.
    let places: Vec<Option<Place>> = items
        .iter()
        .map(|item| match item {
            Item::Run(_) => None,
            Item::Fan(_, place) | Item::Part(place) => Some(*place),
        })
        .collect();
    let folds: Vec<(usize, f64)> = places
        .iter()
        .enumerate()
        .filter_map(|(i, p)| match p {
            Some(Place::Fold(at)) => Some((i, *at)),
            _ => None,
        })
        .collect();
    let mut marks: Vec<Option<Mark>> = places
        .iter()
        .map(|p| match p {
            Some(Place::Corner { rail, source }) => Some(Mark::Corner {
                rail: *rail,
                source: *source,
            }),
            _ => None,
        })
        .collect();
    let mut from_start = 0u32;
    for &(i, at) in &folds {
        if from_start_side(at)
            && let Some(m) = marks.get_mut(i)
        {
            *m = Some(Mark::FoldFromStart(from_start));
            from_start = from_start.saturating_add(1);
        }
    }
    let mut from_end = 0u32;
    for &(i, at) in folds.iter().rev() {
        if !from_start_side(at)
            && let Some(m) = marks.get_mut(i)
        {
            *m = Some(Mark::FoldFromEnd(from_end));
            from_end = from_end.saturating_add(1);
        }
    }
    let runs: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| matches!(item, Item::Run(_)).then_some(i))
        .collect();
    let (first_run, last_run) = (runs.first().copied(), runs.last().copied());
    let mut strips = Vec::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        match item {
            Item::Part(_) => {}
            Item::Fan(mut strip, _) => {
                if let Some(Some(mark)) = marks.get(i) {
                    strip.piece = Piece::Fan(*mark);
                }
                strips.push(strip);
            }
            Item::Run(mut strip) => {
                strip.piece = if Some(i) == first_run {
                    Piece::Rulings
                } else if Some(i) == last_run {
                    Piece::Last
                } else {
                    let before = marks.get(..i).and_then(|m| m.iter().rev().flatten().next());
                    let after = marks.get(i + 1..).and_then(|m| m.iter().flatten().next());
                    match (before, after) {
                        (Some(m @ (Mark::Corner { .. } | Mark::FoldFromStart(_))), _) => {
                            Piece::After(*m)
                        }
                        (_, Some(m)) => Piece::Before(*m),
                        (Some(m), None) => Piece::After(*m),
                        (None, None) => Piece::Rulings,
                    }
                };
                strips.push(strip);
            }
        }
    }
    strips
}

/// How far past the sheet's middle (in the parameter square's diagonal,
/// `(s/len_a + t/len_b)/2`, which runs over `[0, 1]`) a fold is still
/// counted from the start ([`from_start_side`]).
const MIDDLE_MARGIN: f64 = 1e-6;

/// Whether a fold at `at` (its place along the sheet, `0` at the start and
/// `1` at the end) is counted from the start: below the middle plus
/// [`MIDDLE_MARGIN`]. Rails that are symmetric about the sheet's middle
/// (a boat's chine and sheer nearly are) put a fold at the middle itself,
/// where the rounding of the walk would turn a test at `1/2` either way
/// from one edit to the next and retarget the fold's name (rule 4; review
/// of 2026-10-29). Off the middle by a millionth of the sheet, far more
/// than that rounding and than any place symmetry gives a fold, the
/// choice is fixed under such edits; a fold that does move across it is
/// renumbered (the limit of counting from the nearer end, as a fold that
/// appears nearer the same end).
fn from_start_side(at: f64) -> bool {
    at < 0.5 + MIDDLE_MARGIN
}

fn no_point() -> KernelError {
    refused("a curve has a point that is not finite".to_string())
}

/// How far the sheet strays from a developable along the ruling at
/// `(x, y)`: the angle by which the tangent plane turns from the ruling's
/// end on A to its end on B, times the ruling's length, the measure the
/// fit holds every ruling to (half the tolerance). Both planes hold the
/// ruling `D̂`, the one at `A(s)` also `Â'`, the one at `B(t)` also `B̂'`,
/// so the sine of the angle between them is
/// `|det[Â', B̂', D̂]| / (|Â' × D̂| |B̂' × D̂|)`. `None` where a tangent runs
/// along the ruling (no plane there) or the ruling has no length.
fn ruling_twist(pair: &Pair, x: f64, y: f64) -> Option<f64> {
    ruling_twist_side(pair, x, y, false, false)
}

/// [`ruling_twist`] with each rail's tangent taken before a joint where
/// asked (the side of a corner the walk comes from, [`Rail::locate_side`]).
fn ruling_twist_side(pair: &Pair, x: f64, y: f64, before_a: bool, before_b: bool) -> Option<f64> {
    let (angle, length) = ruling_turn_side(pair, x, y, before_a, before_b)?;
    let twist = angle * length;
    twist.is_finite().then_some(twist)
}

/// The angle by which the tangent plane turns along the ruling at
/// `(x, y)` ([`ruling_twist`]) and the ruling's length, on the given sides
/// of joints.
fn ruling_turn_side(
    pair: &Pair,
    x: f64,
    y: f64,
    before_a: bool,
    before_b: bool,
) -> Option<(f64, f64)> {
    let (pa, da) = pair.a.at_side(pair.sa(x), before_a)?;
    let (pb, db) = pair.b.at_side(pair.tb(y), before_b)?;
    let d = pb - pa;
    let (ua, ub, ud) = (da.normalized()?, db.normalized()?, d.normalized()?);
    let (sa, sb) = (ua.cross(ud).length(), ub.cross(ud).length());
    if !(sa > 1e-9 && sb > 1e-9) {
        return None;
    }
    let sine = ua.dot(ub.cross(ud)).abs() / (sa * sb);
    let angle = sine.min(1.0).asin();
    let length = d.length();
    (angle.is_finite() && length.is_finite()).then_some((angle, length))
}

/// Whether the fan at an end, from the first ruling that satisfies the
/// condition (`from`) to the corner's ruling (`to`), lies within the
/// tolerance of the strip of rulings that would run on to the corner in
/// its place: every ruling of the fan (sampled along its stretch, the
/// corner's included) strays from a developable by no more than half the
/// tolerance ([`ruling_twist`]). Such a fan is no more than rails whose
/// ends line up with a ruling within the tolerance (curves fitted to it, a
/// rounding), and making it a face would give a sheet a face, and its
/// edges names, that a perturbation within the tolerance makes or takes
/// away. The fit then holds the strip's last interval to the tolerance
/// like every other.
fn fan_within_tolerance(pair: &Pair, from: (f64, f64), to: (f64, f64), tol: f64) -> bool {
    const SAMPLES: usize = 16;
    (0..=SAMPLES).all(|k| {
        let w = k as f64 / SAMPLES as f64;
        let (x, y) = (from.0 + w * (to.0 - from.0), from.1 + w * (to.1 - from.1));
        ruling_twist(pair, x, y).is_some_and(|d| d <= 0.5 * tol)
    })
}

/// The fan at the start: from `A(0)` to the first ruling along B that
/// satisfies the condition, or from `B(0)` along A; the one that covers
/// less of its curve.
fn start_fan(pair: &Pair) -> Result<(Start, f64), KernelError> {
    let along_b = first_root(&|y| pair.f(0.0, y), 0.0);
    let along_a = first_root(&|x| pair.f(x, 0.0), 0.0);
    let cover_b = along_b.map(|y| pair.b.length_between(0.0, pair.tb(y)));
    let cover_a = along_a.map(|x| pair.a.length_between(0.0, pair.sa(x)));
    match (along_b.zip(cover_b), along_a.zip(cover_a)) {
        (Some((y, cb)), Some((x, ca))) => Ok(if cb <= ca {
            (Start::FromA(y), cb)
        } else {
            (Start::FromB(x), ca)
        }),
        (Some((y, cb)), None) => Ok((Start::FromA(y), cb)),
        (None, Some((x, ca))) => Ok((Start::FromB(x), ca)),
        (None, None) => Err(refused(format!(
            "no ruling from an end of one curve to the other satisfies the condition (tangents and ruling in one plane) {}: these curves have no developable between them that starts at their ends",
            pair.where_(0.0, 0.0)
        ))),
    }
}

/// Where the zero set starts by the start corner, for a start whose fan
/// would be shorter than the tolerance: the nearer root on the two sides.
fn start_root(pair: &Pair) -> Result<(f64, f64), KernelError> {
    Ok(match start_fan(pair)?.0 {
        Start::FromA(y) => (0.0, y),
        Start::FromB(x) => (x, 0.0),
        Start::Corner => (0.0, 0.0),
    })
}

/// How much of a rail a fan at the start corner would cover, by a Newton
/// step from the corner along each side to the zero of the condition (the
/// lesser of the two; infinite where neither side has a step).
fn corner_reach(pair: &Pair) -> f64 {
    let (Some(f0), Some((gx, gy))) = (pair.f(0.0, 0.0), pair.gradient(0.0, 0.0)) else {
        return f64::INFINITY;
    };
    let speed = |rail: &Rail| rail.at(0.0).map_or(f64::INFINITY, |(_, d)| d.length()) * rail.len();
    let side = |g: f64, rail: &Rail| {
        if g == 0.0 || !g.is_finite() {
            f64::INFINITY
        } else {
            (f0 / g).abs() * speed(rail)
        }
    };
    side(gx, pair.a).min(side(gy, pair.b))
}

/// Rulings shrinking to nothing near ends that meet are let be within
/// this many tolerances (an arc length) of those ends.
const REACH: f64 = 100.0;

/// Rails in one plane: the planar region between them.
fn planar(
    a: &Rail,
    b: &Rail,
    frame: Frame,
    tolerance: &Tolerance,
    tips: [bool; 2],
    closed: bool,
) -> Result<Connected, KernelError> {
    let mut pair = Pair::new(a, b, tolerance.angular);
    pair.closed = closed;
    let strips = vec![Strip {
        piece: Piece::Rulings,
        lower: Row {
            side: 0,
            from: 0.0,
            to: a.len(),
        },
        upper: Row {
            side: 1,
            from: 0.0,
            to: b.len(),
        },
        rulings: vec![(0.0, 0.0), (a.len(), b.len())],
    }];
    // The plane faces the way the loop (A forward, B back) runs round
    // (Newell's normal of its points), so no surface is fitted at all.
    let mut normal = Vec3::ZERO;
    let mut points = Vec::new();
    for k in 0..=64 {
        points.push(a.point(a.len() * k as f64 / 64.0).ok_or_else(no_point)?);
    }
    for k in (0..=64).rev() {
        points.push(b.point(b.len() * k as f64 / 64.0).ok_or_else(no_point)?);
    }
    for (k, p) in points.iter().enumerate() {
        let q = points.get((k + 1) % points.len()).copied().unwrap_or(*p);
        normal += (*p - frame.origin()).cross(q - frame.origin());
    }
    let z = if normal.dot(frame.z()) < 0.0 {
        -frame.z()
    } else {
        frame.z()
    };
    let plane = Frame::new(frame.origin(), z, frame.x()).ok_or_else(no_point)?;
    let rulings = vec![(0.0, 0.0), (a.len(), b.len())];
    let mut made = sheet(
        &pair,
        &strips,
        vec![Surface::Plane(Plane::new(plane))],
        tolerance,
        tips,
        0.0,
        rulings,
    )?;
    made.planar = true;
    Ok(made)
}

/// The rail of a side.
fn rail_of<'a>(pair: &Pair<'a>, side: usize) -> &'a Rail {
    if side == 0 { pair.a } else { pair.b }
}

/// The parameters along the path of the rulings: the length of the
/// rulings' ends' moves, both rails added, scaled to `[0, 1]`.
fn path_params(points: &[(Vec3, Vec3)]) -> Option<Vec<f64>> {
    let mut params = Vec::with_capacity(points.len());
    let mut total = 0.0;
    let mut before: Option<(Vec3, Vec3)> = None;
    for &(p, q) in points {
        if let Some((bp, bq)) = before {
            total += p.distance(bp) + q.distance(bq);
        }
        params.push(total);
        before = Some((p, q));
    }
    if !(total > 0.0 && total.is_finite()) {
        return None;
    }
    for t in &mut params {
        *t /= total;
    }
    // Strictly increasing: equal parameters cannot be interpolated.
    if params.windows(2).any(|w| matches!(w, [a, b] if b <= a)) {
        return None;
    }
    Some(params)
}

/// The end derivatives of a face's two rows along the path's parameter
/// (`[[lower at start, upper at start], [lower at end, upper at end]]`,
/// [`fitted`]): each a rail's own tangent at the end ruling (on the face's
/// side of a joint, [`Rail::at_side`]), so the face's tangent plane there
/// is the plane of the ruling and the rails' tangents, as the
/// developable's is, to the relative rounding of those tangents and not to
/// the rounding of points a short end interval apart (review of
/// 2026-10-29: a natural interpolation's end derivatives come from such
/// points). The length is the row's rate at that end as the quadratic
/// through its first (last) three points has it (Bessel's end condition),
/// taken along the tangent and never backward; zero for a row that stays
/// at one point (a fan's apex). The direction of a rail whose tangent
/// vanishes there is the quadratic's own.
fn end_derivatives(
    pair: &Pair,
    rulings: &[(f64, f64)],
    points: &[(Vec3, Vec3)],
    params: &[f64],
    moving: [bool; 2],
) -> Option<[[Vec3; 2]; 2]> {
    let last = points.len().checked_sub(1)?;
    let mut out = [[Vec3::ZERO; 2]; 2];
    for (end, slot) in out.iter_mut().enumerate() {
        let before = end == 1;
        // The three points nearest the end, in the order from the end
        // inward, and their parameters' distances from the end.
        let order: Vec<usize> = if before {
            (0..=last).rev().take(3).collect()
        } else {
            (0..=last).take(3).collect()
        };
        let &first = order.first()?;
        let &(s, t) = rulings.get(first)?;
        let u_at = |i: usize| params.get(i).copied();
        let u0 = u_at(first)?;
        for (side, d) in slot.iter_mut().enumerate() {
            if !moving.get(side).copied().unwrap_or(false) {
                continue;
            }
            let value = |i: usize| points.get(i).map(|(p, q)| if side == 0 { *p } else { *q });
            // The rate along the path at the end: the quadratic through
            // the three points (two: the chord), differentiated at the end.
            let rate = match order.as_slice() {
                [i0, i1, i2] => {
                    let (h1, h2) = ((u_at(*i1)? - u0).abs(), (u_at(*i2)? - u_at(*i1)?).abs());
                    let (q0, q1, q2) = (value(*i0)?, value(*i1)?, value(*i2)?);
                    let f01 = (q1 - q0) * (1.0 / h1);
                    let f12 = (q2 - q1) * (1.0 / h2);
                    // Divided differences taken inward from the end.
                    let rate_in = f01 - (f12 - f01) * (h1 / (h1 + h2));
                    if before { -rate_in } else { rate_in }
                }
                [i0, i1] => {
                    let h = (u_at(*i1)? - u0).abs();
                    let inward = (value(*i1)? - value(*i0)?) * (1.0 / h);
                    if before { -inward } else { inward }
                }
                _ => return None,
            };
            let rail = rail_of(pair, side);
            let at = if side == 0 { s } else { t };
            let tangent = rail.at_side(at, before).and_then(|(_, d)| d.normalized());
            *d = match tangent {
                Some(unit) => unit * rate.dot(unit).max(0.0),
                None => rate,
            };
            if !(d.x.is_finite() && d.y.is_finite() && d.z.is_finite()) {
                return None;
            }
        }
    }
    Some(out)
}

/// The surface of degree 3 along and 1 across whose rows interpolate
/// `points` at `params`, each row with the end derivatives given
/// (`ends[end][row]`, [`end_derivatives`]).
fn fitted(
    params: &[f64],
    points: &[(Vec3, Vec3)],
    ends: [[Vec3; 2]; 2],
) -> Result<NurbsSurface, KernelError> {
    let lower: Vec<Vec<f64>> = points.iter().map(|(p, _)| vec![p.x, p.y, p.z]).collect();
    let upper: Vec<Vec<f64>> = points.iter().map(|(_, q)| vec![q.x, q.y, q.z]).collect();
    let v = |d: Vec3| vec![d.x, d.y, d.z];
    let [[lo0, hi0], [lo1, hi1]] = ends;
    let (knots, lo) = interpolate_with_end_derivatives(params, &lower, &v(lo0), &v(lo1))
        .map_err(KernelError::Geometry)?;
    let (_, hi) = interpolate_with_end_derivatives(params, &upper, &v(hi0), &v(hi1))
        .map_err(KernelError::Geometry)?;
    let at = |v: &Vec<f64>| {
        Vec3::new(
            v.first().copied().unwrap_or(f64::NAN),
            v.get(1).copied().unwrap_or(f64::NAN),
            v.get(2).copied().unwrap_or(f64::NAN),
        )
    };
    let mut ctrl = Vec::with_capacity(2 * lo.len());
    for (p, q) in lo.iter().zip(&hi) {
        ctrl.push(at(p));
        ctrl.push(at(q));
    }
    let count = lo.len();
    NurbsSurface::new(
        3,
        1,
        knots,
        vec![0.0, 0.0, 1.0, 1.0],
        count,
        2,
        ctrl,
        vec![1.0; 2 * count],
    )
    .map_err(KernelError::Geometry)
}

/// The rail parameters of a strip's rulings: its own for a face of
/// rulings, evenly along its stretch for a fan.
fn strip_rulings(strip: &Strip, count: usize) -> Vec<(f64, f64)> {
    if !strip.rulings.is_empty() {
        return strip.rulings.clone();
    }
    (0..=count)
        .map(|k| {
            let f = k as f64 / count as f64;
            let lerp = |row: &Row| row.from + (row.to - row.from) * f;
            (lerp(&strip.lower), lerp(&strip.upper))
        })
        .collect()
}

/// Fits each strip's surface within half the tolerance, then puts the
/// sheet together.
fn build(
    pair: &Pair,
    strips: Vec<Strip>,
    tolerance: &Tolerance,
    tips: [bool; 2],
) -> Result<Connected, KernelError> {
    let tol = tolerance.linear;
    let mut surfaces = Vec::with_capacity(strips.len());
    let mut deviation: f64 = 0.0;
    let mut unresolved: f64 = 0.0;
    let mut all_rulings = Vec::new();
    for strip in &strips {
        let mut rulings = strip_rulings(strip, 16);
        let mut fit = None;
        let moving = [!strip.lower.is_point(), !strip.upper.is_point()];
        for _round in 0..MAX_ROUNDS {
            let points: Vec<(Vec3, Vec3)> = rulings
                .iter()
                .map(|&(s, t)| Some((pair.a.point(s)?, pair.b.point(t)?)))
                .collect::<Option<_>>()
                .ok_or_else(no_point)?;
            let params = path_params(&points)
                .ok_or_else(|| refused("the rulings do not move along the curves".to_string()))?;
            let ends = end_derivatives(pair, &rulings, &points, &params, moving)
                .ok_or_else(|| refused("the rows' end tangents cannot be taken".to_string()))?;
            let surface = fitted(&params, &points, ends)?;
            // Where the fit is farther than half the tolerance from the
            // true developable, a ruling is added at the middle.
            // Per interval: the true ruling at its middle, and whether the
            // fit is off there.
            let mut middles: Vec<(f64, f64)> = Vec::new();
            let mut bad: Vec<bool> = Vec::new();
            let mut worst: f64 = 0.0;
            // What the rounding of the coordinates accounts for at the
            // face's end rulings (below).
            let mut rounded: f64 = 0.0;
            for (k, w) in rulings.windows(2).enumerate() {
                // `windows(2)` gives pairs and `params` has one per ruling.
                let ([(s0, t0), (s1, t1)], Some(&u0), Some(&u1)) =
                    (w, params.get(k), params.get(k + 1))
                else {
                    return Err(no_point());
                };
                let um = 0.5 * (u0 + u1);
                let true_ruling = if strip.rulings.is_empty() {
                    // Along a fan, the rulings are interpolated.
                    Some((0.5 * (s0 + s1), 0.5 * (t0 + t1)))
                } else {
                    let (x, y) = (
                        0.5 * (s0 + s1) / pair.a.len(),
                        0.5 * (t0 + t1) / pair.b.len(),
                    );
                    // Kept between the two rulings it is added between: a
                    // ruling outside them would cross them.
                    let between = |&(sm, tm): &(f64, f64)| {
                        sm >= s0.min(*s1)
                            && sm <= s0.max(*s1)
                            && tm >= t0.min(*t1)
                            && tm <= t0.max(*t1)
                    };
                    let ends = (
                        (*s0 / pair.a.len(), *t0 / pair.b.len()),
                        (*s1 / pair.a.len(), *t1 / pair.b.len()),
                    );
                    across_chord(pair, ends.0, ends.1)
                        .map(|(x, y)| (pair.sa(x), pair.tb(y)))
                        .filter(between)
                        .or_else(|| {
                            onto_zero(pair, x, y)
                                .map(|(x, y)| (pair.sa(x), pair.tb(y)))
                                .filter(between)
                        })
                        .or_else(|| {
                            // Between two rulings found on either side of a
                            // crossing whose branches rounding has split
                            // (`cross`), no ruling satisfies the condition
                            // exactly: the chord's middle, held to the
                            // tolerance by the fit like every other.
                            crossed(pair, ends.0, ends.1).then(|| (pair.sa(x), pair.tb(y)))
                        })
                };
                let Some((sm, tm)) = true_ruling else {
                    return Err(refused(format!(
                        "the rulings between two found ones cannot be followed {}",
                        pair.where_(*s0 / pair.a.len(), *t0 / pair.b.len())
                    )));
                };
                let lo = surface.eval(um, 0.0);
                let hi = surface.eval(um, 1.0);
                let on_a = pair
                    .a
                    .point(pair.a.nearest(lo, sm, *s0, *s1))
                    .map(|p| p.distance(lo));
                let on_b = pair
                    .b
                    .point(pair.b.nearest(hi, tm, *t0, *t1))
                    .map(|p| p.distance(hi));
                let (Some(pa), Some(pb)) = (pair.a.point(sm), pair.b.point(tm)) else {
                    return Err(no_point());
                };
                let mut off = on_a
                    .unwrap_or(f64::INFINITY)
                    .max(on_b.unwrap_or(f64::INFINITY));
                if !strip.rulings.is_empty() {
                    // Developable: the tangent plane is the same all along
                    // a ruling. A fit that turns it (rows that wiggle
                    // against each other between rulings) is refined until
                    // the turn, over the ruling's length, is within half
                    // the tolerance: within the tolerance of a surface
                    // whose rulings keep their tangent plane.
                    let fitted = Surface::Nurbs(surface.clone());
                    if let (Some(n0), Some(n1)) = (fitted.normal(um, 0.0), fitted.normal(um, 1.0)) {
                        let turn = n0.cross(n1).length().atan2(n0.dot(n1));
                        off = off.max(turn * pa.distance(pb));
                    }
                    // The true ruling's middle against the surface.
                    let middle = pa.lerp(pb, 0.5);
                    let near = Surface::Nurbs(surface.clone())
                        .project_from(middle, (um, 0.5))
                        .map(|(u, v)| surface.eval(u, v).distance(middle))
                        .unwrap_or(f64::INFINITY);
                    off = off.max(near);
                }
                // The rows run forward along their rails: a row that turns
                // back between rulings (an interpolation that overshoots)
                // lies on its rail and still folds the face.
                let (_, d_lo, _) = surface.derivatives(um, 0.0);
                let (_, d_hi, _) = surface.derivatives(um, 1.0);
                let forward = |d: Vec3, rail: &Rail, s: f64, moving: bool| {
                    !moving || rail.at(s).is_some_and(|(_, t)| d.dot(t) > 0.0)
                };
                if !(forward(d_lo, pair.a, sm, s1 > s0) && forward(d_hi, pair.b, tm, t1 > t0)) {
                    off = f64::INFINITY;
                }
                worst = worst.max(off);
                middles.push((sm, tm));
                // An interval whose rulings are within the tolerance of each
                // other is not cut finer: nothing below the tolerance is
                // resolved (a ruling snapped to a corner turns the plane
                // there by as much as it was moved).
                let span = points
                    .get(k)
                    .zip(points.get(k + 1))
                    .map_or(0.0, |((a0, b0), (a1, b1))| {
                        a0.distance(*a1) + b0.distance(*b1)
                    });
                bad.push(off > 0.5 * tol && span > tol);
            }
            // The face's end rulings are where it meets the next face of
            // the sheet (or ends): its tangent plane there is held to the
            // developable's (the plane of the ruling and its rails'
            // tangents, on the face's side of a corner), the turn between
            // them over the ruling's length within half the tolerance, so
            // that faces meeting there are tangent to within the tolerance.
            // The rows' end derivatives are the rails' tangents
            // ([`end_derivatives`]), so the face's tangent plane along its
            // end ruling is the plane of the ruling and those derivatives
            // (`S_u = (D_A + D_B)/2`, `S_v = B - A` at `v = 1/2`): taken
            // from them it is exact to their relative rounding, and held
            // with no allowance. A face whose end ruling is twisted (its
            // rails' tangents and the ruling not in one plane) cannot be
            // made tangent there by more rulings: refused. The stored
            // surface's normal there departs from that plane by what the
            // rounding of its control points (`ε |p|` each, over the first
            // control leg) leaves unresolved: measured, reported as
            // `unresolved`, not counted in `deviation`.
            let last = rulings.len().saturating_sub(1);
            let fitted = Surface::Nurbs(surface.clone());
            for (end, before) in [(0, false), (last, true)] {
                let (Some(&(s, t)), Some(&u), Some(&(pa, pb))) =
                    (rulings.get(end), params.get(end), points.get(end))
                else {
                    continue;
                };
                let [d_lo, d_hi] = if before { ends[1] } else { ends[0] };
                let Some(intended) = (d_lo + d_hi).cross(pb - pa).normalized() else {
                    continue;
                };
                let Some(off) = end_turn(pair, intended, (s, t), before, moving) else {
                    continue;
                };
                if off > 0.5 * tol {
                    return Err(refused(format!(
                        "the face's tangent plane at its end ruling {} turns from the developable's by {off} over the ruling (more than half the tolerance): the ruling and its curves' tangents there are not in one plane",
                        pair.where_(s / pair.a.len(), t / pair.b.len())
                    )));
                }
                worst = worst.max(off);
                if let Some(stored) = fitted.normal(u, 0.5).and_then(|n| n.normalized()) {
                    let turn = stored.cross(intended).length().atan2(stored.dot(intended));
                    let gap = turn * pa.distance(pb);
                    if gap.is_finite() {
                        rounded = rounded.max(gap);
                    }
                }
            }
            // Refined where off and beside it, so that neighbouring
            // intervals stay within a factor of two of each other (an
            // interpolation through rulings spaced very unevenly
            // overshoots between them).
            let refine: Vec<bool> = (0..bad.len())
                .map(|k| {
                    [k.checked_sub(1), Some(k), k.checked_add(1)]
                        .into_iter()
                        .flatten()
                        .any(|j| bad.get(j).copied().unwrap_or(false))
                })
                .collect();
            let added: Vec<(usize, (f64, f64))> = refine
                .iter()
                .enumerate()
                .filter(|(_, r)| **r)
                .filter_map(|(k, _)| middles.get(k).map(|m| (k + 1, *m)))
                .collect();
            if !bad.iter().any(|b| *b) {
                deviation = deviation.max(worst);
                unresolved = unresolved.max(rounded);
                fit = Some(surface);
                break;
            }
            if rulings.len() + added.len() > MAX_RULINGS {
                return Err(refused(format!(
                    "the faces could not be fitted within half the tolerance with {MAX_RULINGS} rulings (still {worst} off)"
                )));
            }
            for (at, ruling) in added.into_iter().rev() {
                rulings.insert(at, ruling);
            }
        }
        let surface = fit.ok_or_else(|| {
            refused(format!(
                "the faces could not be fitted within half the tolerance in {MAX_ROUNDS} rounds"
            ))
        })?;
        if let Some(w) = rulings
            .windows(2)
            .find(|w| matches!(w, [(s0, t0), (s1, t1)] if s1 < s0 || t1 < t0))
            && let [(s0, t0), _] = w
        {
            return Err(refused(format!(
                "the rulings would cross {}",
                pair.where_(*s0 / pair.a.len(), *t0 / pair.b.len())
            )));
        }
        if !strip.rulings.is_empty() {
            all_rulings.extend(rulings);
        }
        surfaces.push(Surface::Nurbs(surface));
    }
    let mut made = sheet(
        pair,
        &strips,
        surfaces,
        tolerance,
        tips,
        deviation,
        all_rulings,
    )?;
    made.unresolved = unresolved;
    Ok(made)
}

/// How far a face's tangent plane along its end ruling `(s, t)` (rail
/// parameters; `normal` the face's unit normal there) strays from the
/// developable's: the angle between `normal` and the normal of the plane
/// of the ruling and a rail's tangent (at a joint, on the side given), the
/// larger over the rails whose rows move along the face, times the
/// ruling's length. A developable's tangent plane along a ruling holds
/// both rails' tangents; a ruling snapped to a corner within the
/// tolerance holds each within the tolerance. A fan's apex row has no
/// tangent (`moving` false). `None` where a normal has no direction.
fn end_turn(
    pair: &Pair,
    normal: Vec3,
    (s, t): (f64, f64),
    before: bool,
    moving: [bool; 2],
) -> Option<f64> {
    let (pa, da) = pair.a.at_side(s, before)?;
    let (pb, db) = pair.b.at_side(t, before)?;
    let d = pb - pa;
    let ud = d.normalized()?;
    let n = normal.normalized()?;
    let mut angle: f64 = 0.0;
    for (row, tangent) in [(moving[0], da), (moving[1], db)] {
        if !row {
            continue;
        }
        let expected = tangent.normalized()?.cross(ud).normalized()?;
        angle = angle.max(n.cross(expected).length().min(1.0).asin());
    }
    let off = angle * d.length();
    off.is_finite().then_some(off)
}

/// Vertices and edges of the sheet, shared where the faces meet.
struct Parts {
    body: Body,
    vertices: BTreeMap<(usize, u64), VertexId>,
    rulings: BTreeMap<(u64, u64), EdgeId>,
}

impl Parts {
    fn vertex(
        &mut self,
        pair: &Pair,
        side: usize,
        at: f64,
        tips: [bool; 2],
    ) -> Result<VertexId, KernelError> {
        let rail = rail_of(pair, side);
        // A closed rail's ends are one vertex.
        let at = if pair.closed && at >= rail.len() {
            0.0
        } else {
            at
        };
        // Ends that meet are one vertex: B's is A's.
        let (side, at) = if side == 1 && tips[0] && at <= 0.0 {
            (0, 0.0)
        } else if side == 1 && tips[1] && at >= rail.len() {
            (0, pair.a.len())
        } else {
            (side, at)
        };
        let key = (side, at.to_bits());
        if let Some(&v) = self.vertices.get(&key) {
            return Ok(v);
        }
        let point = rail_of(pair, side).point(at).ok_or_else(no_point)?;
        let v = self.body.add_vertex(point);
        self.vertices.insert(key, v);
        Ok(v)
    }

    /// The edges of a rail from `from` to `to`, one per curve it crosses,
    /// as coedges in the rail's direction (or against it when `back`).
    fn rail_edges(
        &mut self,
        pair: &Pair,
        row: Row,
        back: bool,
        tips: [bool; 2],
    ) -> Result<Vec<Coedge>, KernelError> {
        let rail = rail_of(pair, row.side);
        let mut out = Vec::new();
        let mut s = row.from;
        while s < row.to {
            let index = (s.floor() as usize).min(rail.pieces.len().saturating_sub(1));
            let end = ((index + 1) as f64).min(row.to);
            if end <= s {
                break;
            }
            let piece = rail.pieces.get(index).ok_or_else(no_point)?;
            let (lo, hi) = (s - index as f64, end - index as f64);
            let (a, b) = piece.range;
            let span = b - a;
            let (c_lo, c_hi) = if piece.reversed {
                (b - lo * span, b - hi * span)
            } else {
                (a + lo * span, a + hi * span)
            };
            let (v_lo, v_hi) = (
                self.vertex(pair, row.side, s, tips)?,
                self.vertex(pair, row.side, end, tips)?,
            );
            let edge = if piece.reversed {
                self.body
                    .add_edge(piece.curve.clone(), (c_hi, c_lo), v_hi, v_lo)
            } else {
                self.body
                    .add_edge(piece.curve.clone(), (c_lo, c_hi), v_lo, v_hi)
            };
            out.push(Coedge {
                edge,
                reversed: piece.reversed,
            });
            s = end;
        }
        if back {
            out.reverse();
            for c in &mut out {
                c.reversed = !c.reversed;
            }
        }
        Ok(out)
    }

    /// The straight edge of the ruling `A(s)B(t)`, as a coedge from A to
    /// B (or back); `None` when it has no length (ends that meet).
    fn ruling(
        &mut self,
        pair: &Pair,
        (s, t): (f64, f64),
        back: bool,
        tol: f64,
        tips: [bool; 2],
    ) -> Result<Option<Coedge>, KernelError> {
        let (pa, pb) = (
            pair.a.point(s).ok_or_else(no_point)?,
            pair.b.point(t).ok_or_else(no_point)?,
        );
        let length = pa.distance(pb);
        if length <= tol {
            return Ok(None);
        }
        // A tube's last ruling is its first.
        let key = if pair.closed && s >= pair.a.len() && t >= pair.b.len() {
            (0.0_f64.to_bits(), 0.0_f64.to_bits())
        } else {
            (s.to_bits(), t.to_bits())
        };
        let edge = match self.rulings.get(&key) {
            Some(&e) => e,
            None => {
                let line = Line::through(pa, pb).map_err(KernelError::Geometry)?;
                let (va, vb) = (
                    self.vertex(pair, 0, s, tips)?,
                    self.vertex(pair, 1, t, tips)?,
                );
                let e = self
                    .body
                    .add_edge(Curve3::Line(line), (0.0, length), va, vb);
                self.rulings.insert(key, e);
                e
            }
        };
        Ok(Some(Coedge {
            edge,
            reversed: back,
        }))
    }
}

fn sheet(
    pair: &Pair,
    strips: &[Strip],
    surfaces: Vec<Surface>,
    tolerance: &Tolerance,
    tips: [bool; 2],
    deviation: f64,
    rulings: Vec<(f64, f64)>,
) -> Result<Connected, KernelError> {
    let tol = tolerance.linear;
    let mut parts = Parts {
        body: Body::new(BodyKind::Sheet),
        vertices: BTreeMap::new(),
        rulings: BTreeMap::new(),
    };
    let mut faces = Vec::new();
    let mut ids: Vec<FaceId> = Vec::new();
    let mut curvature: f64 = 0.0;
    for (strip, surface) in strips.iter().zip(surfaces) {
        let (lo, up) = (strip.lower, strip.upper);
        let mut coedges = Vec::new();
        if !lo.is_point() {
            coedges.extend(parts.rail_edges(pair, lo, false, tips)?);
        }
        if let Some(c) = parts.ruling(pair, (lo.to, up.to), false, tol, tips)? {
            coedges.push(c);
        }
        if !up.is_point() {
            coedges.extend(parts.rail_edges(pair, up, true, tips)?);
        }
        if let Some(c) = parts.ruling(pair, (lo.from, up.from), true, tol, tips)? {
            coedges.push(c);
        }
        // Per ruling interval (every knot span of the fitted face), not
        // on a grid coarser than the rulings.
        let ((u0, u1), _) = surface.domain();
        for u in crate::unfold::curvature_samples(&surface, true, (u0, u1), 8) {
            for v in [0.25, 0.5, 0.75] {
                if let Some(kk) = craft3d_geom::curvature::gaussian_curvature(&surface, u, v) {
                    curvature = curvature.max(kk.abs());
                }
            }
        }
        let lp = parts.body.add_loop(coedges);
        let id = parts.body.add_face(surface, false, vec![lp]);
        ids.push(id);
        faces.push(strip.piece);
    }
    // Neighbouring faces must face the same way where they meet.
    for w in ids.windows(2) {
        let [f0, f1] = w else { continue };
        let (Some(a), Some(b)) = (parts.body.face(*f0), parts.body.face(*f1)) else {
            continue;
        };
        let (n0, n1) = (a.surface.normal(1.0, 0.5), b.surface.normal(0.0, 0.5));
        if let (Some(n0), Some(n1)) = (n0, n1)
            && n0.dot(n1) < 0.0
        {
            return Err(refused(
                "the fan at an end and the rulings beside it face opposite ways: the curves pass on either side of the ruling they share; no developable sheet joins them there".to_string(),
            ));
        }
    }
    parts.body.add_shell(ids);
    let mut defects = craft3d_topo::check_remembered(&parts.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(crate::MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: OPERATION,
            defects,
        });
    }
    Ok(Connected {
        body: parts.body,
        faces,
        rulings,
        deviation,
        unresolved: 0.0,
        curvature,
        planar: false,
        reversed_b: false,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn line_rail(from: Vec3, to: Vec3) -> Rail {
        Rail {
            pieces: vec![RailPiece {
                curve: Curve3::Line(Line::through(from, to).unwrap()),
                range: (0.0, from.distance(to)),
                reversed: false,
                source: 0,
            }],
        }
    }

    /// Two arcs of radius `r` about the z axis, at heights 0 and `h`, over
    /// one radian: a cylinder, its rulings `(s, s)`.
    fn cylinder(r: f64, h: f64) -> (Rail, Rail) {
        let arc = |z: f64| Rail {
            pieces: vec![RailPiece {
                curve: Curve3::Circle(
                    craft3d_geom::Circle::new(
                        Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap(),
                        r,
                    )
                    .unwrap(),
                ),
                range: (0.0, 1.0),
                reversed: false,
                source: 0,
            }],
        };
        (arc(0.0), arc(h))
    }

    /// A strip of the cylinder from `first` on, its rulings `(s, s)` at
    /// spacings doubling from `step` (as [`thinned`] leaves them beside a
    /// short one), to `(1, 1)`.
    fn cylinder_strip(first: (f64, f64), step: f64) -> Strip {
        let mut rulings = vec![first];
        let mut s = step;
        while s < 1.0 {
            rulings.push((s, s));
            s *= 2.0;
        }
        rulings.push((1.0, 1.0));
        Strip {
            piece: Piece::Rulings,
            lower: Row {
                side: 0,
                from: first.0,
                to: 1.0,
            },
            upper: Row {
                side: 1,
                from: first.1,
                to: 1.0,
            },
            rulings,
        }
    }

    /// The end interval of a face no longer than the tolerance, as beside
    /// a fan whose end the walk keeps whatever its length ([`thinned`]),
    /// is checked like every other end near the origin, where the rounding
    /// of the coordinates resolves its plane (review of 2026-10-29: it was
    /// skipped wherever it was no longer than the tolerance, so the
    /// tangency of the face to the fan beside it went unchecked). A strip
    /// of a cylinder of radius `k/10` and height `k` whose end ruling runs
    /// from `A(0)` to `B(σ)` (twisted by `σ k`, 2.5 tolerances) with the
    /// next ruling `(2σ, 2σ)` 0.75 tolerances away is refused; the same
    /// strip from `(0, 0)` is built within half the tolerance, its end
    /// counted. At three scales.
    #[test]
    fn a_short_end_interval_is_held_to_the_tolerance_near_the_origin() {
        for k in [1e-3, 1.0, 1e6] {
            let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
            let (r, h) = (0.1 * k, k);
            let (a, b) = cylinder(r, h);
            let pair = Pair::new(&a, &b, tolerance.angular);
            let sigma = tolerance.linear / (4.0 * r);
            let twisted = cylinder_strip((0.0, sigma), 2.0 * sigma);
            let e = build(&pair, vec![twisted], &tolerance, [false, false])
                .map(|made| (made.deviation, made.unresolved))
                .expect_err("the twisted end is refused");
            assert!(matches!(e, KernelError::Refused { .. }), "{k}: {e}");
            let exact = cylinder_strip((0.0, 0.0), 2.0 * sigma);
            let made = build(&pair, vec![exact], &tolerance, [false, false])
                .unwrap_or_else(|e| panic!("{k}: {e}"));
            assert!(
                made.deviation <= 0.5 * tolerance.linear,
                "{k}: {}",
                made.deviation
            );
            assert!(
                made.unresolved <= 1e-3 * tolerance.linear,
                "{k}: {}",
                made.unresolved
            );
        }
    }

    /// A face's rows leave its end rulings along the rails' own tangents
    /// (Hermite end conditions, review of 2026-10-29), so the stored face's
    /// tangent plane there is the developable's to the rounding of its
    /// control points: a strip of a cylinder whose rulings are spaced
    /// unevenly (doubling from the end), its rows' derivatives at both ends
    /// along the arcs' tangents within 1e-12 rad, at three scales.
    #[test]
    fn a_faces_rows_leave_its_ends_along_the_rails() {
        for k in [1e-3, 1.0, 1e6] {
            let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
            let (a, b) = cylinder(0.1 * k, k);
            let pair = Pair::new(&a, &b, tolerance.angular);
            let made = build(
                &pair,
                vec![cylinder_strip((0.0, 0.0), 1e-3)],
                &tolerance,
                [false, false],
            )
            .unwrap_or_else(|e| panic!("{k}: {e}"));
            let Surface::Nurbs(surface) = &made.body.faces[0].surface else {
                panic!("{k}: a face of rulings is a spline");
            };
            let ctrl = surface.control_points();
            let n = surface.counts().0;
            for (u, s, end, next) in [(0.0, 0.0, 0, 1), (1.0, 1.0, n - 1, n - 2)] {
                for (v, rail, row) in [(0.0, &a, 0), (1.0, &b, 1)] {
                    let (_, du, _) = surface.derivatives(u, v);
                    let (_, tangent) = rail.at(s).unwrap();
                    let angle = du.cross(tangent).length().atan2(du.dot(tangent));
                    // What the control points' coordinates resolve of the
                    // direction of the first control leg.
                    let (p0, p1) = (ctrl[2 * end + row], ctrl[2 * next + row]);
                    let resolved =
                        16.0 * f64::EPSILON * p0.length().max(p1.length()) / p0.distance(p1);
                    assert!(
                        angle <= resolved && resolved < 1e-9,
                        "{k}: ({u}, {v}) leaves at {angle} rad ({resolved} resolved)"
                    );
                }
            }
        }
    }

    /// A cylinder of radius `r` and height `h` about a vertical axis
    /// through `center`, its arcs over one radian from the +x side.
    fn cylinder_at(center: Vec3, r: f64, h: f64) -> (Rail, Rail) {
        let arc = |z: f64| Rail {
            pieces: vec![RailPiece {
                curve: Curve3::Circle(
                    craft3d_geom::Circle::new(
                        Frame::new(center + Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap(),
                        r,
                    )
                    .unwrap(),
                ),
                range: (0.0, 1.0),
                reversed: false,
                source: 0,
            }],
        };
        (arc(0.0), arc(h))
    }

    /// A twisted end ruling far from the origin is refused (review of
    /// 2026-10-29: the end check allowed the rounding of the coordinates
    /// over the end interval, `ε |p| L / span`, which far from the origin
    /// over a short end interval outgrew any real defect, and grew as
    /// refinement halved the span). A cylinder of radius `k/10` and height
    /// `10 k` 2.4e9 tolerances from the origin (the sea kayak's sheer panel
    /// lies so far out), its end ruling from `A(0)` to `B(σ)`, twisted by
    /// four tolerances, the next ruling `(2e-5, 2e-5)` (an end interval of
    /// four tolerances): the old allowance there was 1.3 tolerances, and
    /// doubled with each refinement of the end interval. The old code
    /// refused this strip too, but only because rulings that close cannot
    /// be followed this far out (measured 2026-10-29, run 40823084121: "the
    /// rulings between two found ones cannot be followed"), so this test
    /// pins the end check's refusal by name, not an acceptance before. The
    /// rows' end tangents are now the rails' and the end held with no
    /// allowance: refused, naming the end ruling; the same strip with
    /// rulings 1e4 tolerances apart and an end on the cylinder is built
    /// within half the tolerance,
    /// what the stored face's end normals leave unresolved a small part of
    /// the tolerance. At three scales.
    #[test]
    fn a_twisted_end_far_from_the_origin_is_refused() {
        for k in [1e-3, 1.0, 1e6] {
            let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
            let center = Vec3::new(2.4e3 * k, 0.0, 0.0);
            let (r, h) = (0.1 * k, 10.0 * k);
            let (a, b) = cylinder_at(center, r, h);
            let pair = Pair::new(&a, &b, tolerance.angular);
            let sigma = 4.0 * tolerance.linear / h;
            let twisted = cylinder_strip((0.0, sigma), 2e-5);
            let e = build(&pair, vec![twisted], &tolerance, [false, false])
                .map(|made| (made.deviation, made.unresolved))
                .expect_err("the twisted end is refused");
            assert!(
                matches!(e, KernelError::Refused { .. }) && e.to_string().contains("end ruling"),
                "{k}: {e}"
            );
            let exact = cylinder_strip((0.0, 0.0), 0.1);
            let made = build(&pair, vec![exact], &tolerance, [false, false])
                .unwrap_or_else(|e| panic!("{k}: {e}"));
            eprintln!(
                "{k}: deviation {:e}, unresolved {:e} (tolerances)",
                made.deviation / tolerance.linear,
                made.unresolved / tolerance.linear
            );
            assert!(
                made.deviation <= 0.5 * tolerance.linear,
                "{k}: {}",
                made.deviation
            );
            assert!(
                made.unresolved <= 5e-2 * tolerance.linear,
                "{k}: {}",
                made.unresolved
            );
        }
    }

    /// A fold noticed where a fan has just landed (the fan across a fold
    /// landing on a branch that already turns back) starts at the fan's
    /// end, which is kept (review of 2026-10-29: it was overwritten by a
    /// "turning point" found across the fan, which moved the fan's end off
    /// its line and misread its kind).
    #[test]
    fn a_fold_where_a_fan_lands_keeps_the_fans_end() {
        let (a, b) = cylinder(1.0, 1.0);
        let pair = Pair::new(&a, &b, 1e-9);
        let mut path = Path::default();
        path.push((0.2, 0.2));
        path.push((0.3, 0.3));
        path.push_fan((0.3, 0.5));
        let at = fold_start(&pair, &mut path, (0.3, 0.5), true);
        assert_eq!(at, (0.3, 0.5));
        assert_eq!(path.points, vec![(0.2, 0.2), (0.3, 0.3), (0.3, 0.5)]);
        assert_eq!(path.fan_after, vec![false, true, false]);
    }

    /// Two fans in a row with one apex are one fan; two with apexes on
    /// different rails, or a fan and rulings, are kept.
    #[test]
    fn fans_in_a_row_with_one_apex_are_one() {
        let mut path = Path::default();
        path.push((0.1, 0.1));
        path.push((0.2, 0.2));
        path.push_fan((0.2, 0.3)); // apex on A, s = 0.2
        path.push_fan((0.2, 0.300000001)); // the same apex
        path.push_fan((0.4, 0.300000001)); // apex on B
        path.push((0.5, 0.5));
        let merged = merged_fans(path);
        assert_eq!(
            merged.points,
            vec![
                (0.1, 0.1),
                (0.2, 0.2),
                (0.2, 0.300000001),
                (0.4, 0.300000001),
                (0.5, 0.5)
            ]
        );
        assert_eq!(merged.fan_after, vec![false, true, true, false, false]);
    }

    /// The step towards a saddle ahead is half its distance, never below
    /// twice the least step without a step having failed, and never more
    /// than it was.
    #[test]
    fn a_step_towards_a_saddle_stays_above_the_least_step() {
        assert_eq!(approach(STEP_MAX, 1e-3), 5e-4);
        assert_eq!(approach(STEP_MAX, 1e-12), 2.0 * MIN_STEP);
        assert_eq!(approach(1e-10, 1e-12), 1e-10);
        assert_eq!(approach(1e-4, 1.0), 1e-4);
    }

    /// The names of the faces of a path with fans at folds at the given
    /// places (each a fan at one `s`, apex on A, across `t` from just
    /// below to just above the place).
    fn names(folds: &[f64]) -> Vec<Piece> {
        let a = line_rail(Vec3::ZERO, Vec3::X);
        let b = line_rail(Vec3::Z, Vec3::new(1.0, 0.0, 1.0));
        let pair = Pair::new(&a, &b, 1e-9);
        let mut rulings = vec![(0.0, 0.0)];
        let mut fan_after = vec![false];
        for &x in folds {
            rulings.push((x, x - 0.01));
            fan_after.push(true);
            rulings.push((x, x + 0.01));
            fan_after.push(false);
        }
        rulings.push((1.0, 1.0));
        fan_after.push(false);
        let break_at = vec![false; rulings.len()];
        middle_strips(&pair, &rulings, &fan_after, &break_at)
            .into_iter()
            .map(|s| s.piece)
            .collect()
    }

    /// The name of the fan at a fold whose place along the sheet is `at`
    /// (the fan's first ruling `(at, at)` on rails of one piece each, apex
    /// on A).
    fn fold_name_at(at: f64) -> Piece {
        let a = line_rail(Vec3::ZERO, Vec3::X);
        let b = line_rail(Vec3::Z, Vec3::new(1.0, 0.0, 1.0));
        let pair = Pair::new(&a, &b, 1e-9);
        let rulings = vec![(0.0, 0.0), (at, at), (at, at + 0.02), (1.0, 1.0)];
        let fan_after = vec![false, true, false, false];
        let break_at = vec![false; rulings.len()];
        middle_strips(&pair, &rulings, &fan_after, &break_at)[1].piece
    }

    /// A fold at the sheet's middle keeps its name when rounding moves it
    /// either way (review of 2026-10-29: `at < 1/2` counted it from the
    /// start and `at >= 1/2` from the end, so symmetric rails, whose fold
    /// lies at the middle, had its name turned by the rounding of an edit
    /// elsewhere, rule 4). At the middle and within a millionth of the
    /// sheet of it, the first fold from the start; past that margin, from
    /// the end.
    #[test]
    fn a_fold_at_the_middle_keeps_its_name() {
        use Mark::{FoldFromEnd as E, FoldFromStart as S};
        for at in [
            0.5 - 1e-9,
            0.5 - 1e-15,
            0.5,
            0.5 + 1e-15,
            0.5 + 1e-12,
            0.5 + 1e-9,
            0.5 + 0.5 * MIDDLE_MARGIN,
        ] {
            assert_eq!(fold_name_at(at), Piece::Fan(S(0)), "{at}");
        }
        for at in [0.5 + 2.0 * MIDDLE_MARGIN, 0.6, 0.9] {
            assert_eq!(fold_name_at(at), Piece::Fan(E(0)), "{at}");
        }
        const { assert!(MIDDLE_MARGIN >= 1e-6) };
    }

    /// An end's fan and a fan of the path beside it with the same apex are
    /// one fan, named as the end's (review of 2026-10-29: an end fan added
    /// after [`merged_fans`] was not merged with the fold's fan beside it);
    /// fans with other apexes, and a path that is only the fan, are kept.
    #[test]
    fn an_end_fan_with_the_apex_of_the_fan_beside_it_is_one() {
        // The path ends in a fan along A's last end (apex A(1)), and the end
        // fan from A's last end runs on to B's: one cone.
        let mut path = Path::default();
        path.push((0.0, 0.0));
        path.push((0.5, 0.5));
        path.push((1.0, 0.7));
        path.push_fan((1.0, 0.8));
        let (merged, _, end) = merged_end_fans(path, Start::Corner, Some(Start::FromA(0.8)));
        assert_eq!(merged.points, vec![(0.0, 0.0), (0.5, 0.5), (1.0, 0.7)]);
        assert_eq!(merged.fan_after, vec![false, false, false]);
        assert!(matches!(end, Some(Start::FromA(y)) if y == 0.7));
        // At the start: the start's fan from A's first end, and a fan along
        // A's first end the path starts with.
        let mut path = Path::default();
        path.push((0.0, 0.2));
        path.push_fan((0.0, 0.3));
        path.push((0.5, 0.5));
        path.push((1.0, 1.0));
        let (merged, start, _) = merged_end_fans(path, Start::FromA(0.2), None);
        assert_eq!(merged.points, vec![(0.0, 0.3), (0.5, 0.5), (1.0, 1.0)]);
        assert_eq!(merged.fan_after, vec![false, false, false]);
        assert!(matches!(start, Start::FromA(y) if y == 0.3));
        // The path's last fan has its apex on B: two cones, kept.
        let mut path = Path::default();
        path.push((0.0, 0.0));
        path.push((0.5, 0.5));
        path.push((0.9, 0.8));
        path.push_fan((1.0, 0.8));
        let (kept, _, end) = merged_end_fans(path, Start::Corner, Some(Start::FromA(0.8)));
        assert_eq!(kept.points.len(), 4);
        assert!(matches!(end, Some(Start::FromA(y)) if y == 0.8));
        // Only the fan: kept (no rulings would be left).
        let mut path = Path::default();
        path.push((1.0, 0.7));
        path.push_fan((1.0, 0.8));
        let (kept, _, _) = merged_end_fans(path, Start::Corner, Some(Start::FromA(0.8)));
        assert_eq!(kept.points.len(), 2);
    }

    /// Faces are named by place (2026-10-28): a fold that appears or goes
    /// at one end, or between, leaves the names at the other end as they
    /// are (rule 4, docs/design/names.md).
    #[test]
    fn folds_are_named_from_the_nearer_end() {
        use Mark::{FoldFromEnd as E, FoldFromStart as S};
        assert_eq!(names(&[]), vec![Piece::Rulings]);
        let both = names(&[0.2, 0.8]);
        assert_eq!(
            both,
            vec![
                Piece::Rulings,
                Piece::Fan(S(0)),
                Piece::After(S(0)),
                Piece::Fan(E(0)),
                Piece::Last
            ]
        );
        // Without the bow's fold, or with one more there: the stern's
        // names are as they were.
        assert_eq!(
            names(&[0.8]),
            vec![Piece::Rulings, Piece::Fan(E(0)), Piece::Last]
        );
        let more = names(&[0.1, 0.2, 0.8]);
        assert_eq!(more[more.len() - 2..], both[both.len() - 2..]);
        assert_eq!(more[..3], both[..3]);
        // Without the stern's fold: the bow's names are as they were.
        let bow = names(&[0.2]);
        assert_eq!(bow[..2], both[..2]);
        // Folds in the second half, counted from the end; the runs between
        // them named by the fold after them.
        assert_eq!(
            names(&[0.6, 0.7, 0.9]),
            vec![
                Piece::Rulings,
                Piece::Fan(E(2)),
                Piece::Before(E(1)),
                Piece::Fan(E(1)),
                Piece::Before(E(0)),
                Piece::Fan(E(0)),
                Piece::Last
            ]
        );
    }
}
