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
//! Where the zero set turns back (later rulings would cross earlier ones:
//! an inflection, a rail that winds about the other), branches or
//! vanishes over a stretch (both rails in one plane there), or a ruling
//! shrinks to nothing between the ends, the connection is refused with the
//! place; it is not split by a guess. Rails that lie in one plane are
//! connected by the planar region between them. The method, its sources
//! and its oracles: docs/design/developable.md.

use crate::KernelError;
use craft3d_geom::refine::interpolate;
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

/// Which face of the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Piece {
    /// The rulings that satisfy the condition.
    Rulings,
    /// The cone from the first end of one rail to the start of the other.
    StartFan,
    /// The cone from the last end of one rail to the end of the other.
    EndFan,
    /// A face after the first run of rulings, in order along the rails
    /// (from 0): a fan where the condition turns back (a cone at one
    /// point of a rail, across a stretch of the other), or a run of
    /// rulings after such a fan.
    Inner(u32),
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
    /// developable (its rails and its rulings), in model units.
    pub deviation: f64,
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

    /// The piece at `s`, the curve parameter there and `dc/ds`.
    fn locate(&self, s: f64) -> Option<(&RailPiece, f64, f64)> {
        let count = self.pieces.len();
        if count == 0 || !s.is_finite() {
            return None;
        }
        let s = s.clamp(0.0, count as f64);
        let index = (s.floor() as usize).min(count - 1);
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
        let (piece, c, rate) = self.locate(s)?;
        let (p, d) = piece.curve.derivatives(c);
        Some((p, d * rate))
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
                // A corner: the tangent turns where two curves meet.
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
                    && turn(u, w) > angular
                {
                    return Err(refused(format!(
                        "{name} has a corner at {} between curves {} and {} (the tangent turns by {:.3} degrees); a developable through a corner of a rail (a fan from the corner) is not built yet: connect the smooth stretches one by one",
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

/// The two rails, scaled so that each runs over `[0, 1]`.
struct Pair<'a> {
    a: &'a Rail,
    b: &'a Rail,
}

impl Pair<'_> {
    fn sa(&self, x: f64) -> f64 {
        x * self.a.len()
    }
    fn tb(&self, y: f64) -> f64 {
        y * self.b.len()
    }
    fn points(&self, x: f64, y: f64) -> Option<(Vec3, Vec3)> {
        Some((self.a.point(self.sa(x))?, self.b.point(self.tb(y))?))
    }

    /// The condition: `det[Â', B̂', D̂]`, `None` where a tangent or the
    /// ruling has no direction.
    fn f(&self, x: f64, y: f64) -> Option<f64> {
        let (pa, da) = self.a.at(self.sa(x))?;
        let (pb, db) = self.b.at(self.tb(y))?;
        let (ua, ub, ud) = (da.normalized()?, db.normalized()?, (pb - pa).normalized()?);
        let value = ua.dot(ub.cross(ud));
        value.is_finite().then_some(value)
    }

    /// The gradient of `f` by central differences, one-sided at the
    /// sides of the square.
    fn gradient(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let h = 1e-7;
        let diff = |lo: f64, hi: f64, at: &dyn Fn(f64) -> Option<f64>| -> Option<f64> {
            let (a, b) = (lo.max(0.0), hi.min(1.0));
            if b <= a {
                return None;
            }
            Some((at(b)? - at(a)?) / (b - a))
        };
        let gx = diff(x - h, x + h, &|x| self.f(x, y))?;
        let gy = diff(y - h, y + h, &|y| self.f(x, y))?;
        (gx.is_finite() && gy.is_finite()).then_some((gx, gy))
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
}

impl Path {
    fn push(&mut self, p: (f64, f64)) {
        self.points.push(p);
        self.fan_after.push(false);
    }
    /// The stretch from the last point to `p` is a fan.
    fn push_fan(&mut self, p: (f64, f64)) {
        if let Some(last) = self.fan_after.last_mut() {
            *last = true;
        }
        self.push(p);
    }
    /// Whether point `k` ends or starts a fan (it is kept as it is).
    fn fixed(&self, k: usize) -> bool {
        self.fan_after.get(k).copied().unwrap_or(false)
            || k.checked_sub(1)
                .is_some_and(|j| self.fan_after.get(j).copied().unwrap_or(false))
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
            let previous = path
                .points
                .len()
                .checked_sub(2)
                .and_then(|k| path.points.get(k))
                .copied()
                .unwrap_or((x, y));
            let at = turning_point(pair, previous, (x, y), falls_t).unwrap_or((x, y));
            if let Some(last) = path.points.last_mut() {
                *last = at;
            }
            fold = Some(Fold {
                at,
                falls_t,
                bridge: bridge(pair, at, falls_t),
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
        let (px, py) = (
            (x + step * tx).clamp(0.0, 1.0),
            (y + step * ty).clamp(0.0, 1.0),
        );
        let at_side = px >= 1.0 || py >= 1.0;
        // Corrector: back onto the zero set; on a side, along that side.
        let corrected = if at_side {
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
            let turned = tangent_at(pair, qx, qy)
                .is_some_and(|(ux, uy)| (ux * tx + uy * ty).abs() >= MAX_TURN.cos());
            let forward = match fold {
                None => qx >= x - 1e-12 && qy >= y - 1e-12,
                Some(f) if f.falls_t => qx >= x - 1e-12,
                Some(_) => qy >= y - 1e-12,
            };
            forward && moved <= 1.5 * step + 1e-12 && moved > 0.0 && (turned || at_side)
        });
        let Some((qx, qy)) = accepted else {
            h *= 0.5;
            if h < 1e-10 {
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

/// The fan across a fold at `at` where `t` falls (`falls_t`; else `s`):
/// at the fold's `s` (apex on A), from its `t` up to the next zero of
/// `f(s, ·)` where the zero set goes on forward (not back in `s`); `None`
/// when there is none before the end of the rail.
fn bridge(pair: &Pair, at: (f64, f64), falls_t: bool) -> Option<(f64, f64)> {
    // A zero next to the fold's own is its own, met again by rounding.
    let gap = 1e-9;
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
    let value = pair.f(x, y)?;
    let (gx, gy) = pair.gradient(x, y)?;
    let [hxx, hxy, hyy] = hessian(pair, x, y)?;
    let det = hxx * hyy - hxy * hxy;
    if !(det < 0.0) {
        return None;
    }
    // The saddle of the model and the value there.
    let (dx, dy) = (-(hyy * gx - hxy * gy) / det, -(hxx * gy - hxy * gx) / det);
    let saddle_value = value + 0.5 * (gx * dx + gy * dy);
    let far = dx.hypot(dy);
    let (cx, cy) = (x + dx, y + dy);
    if !(far <= STEP_MAX && (0.0..=1.0).contains(&cx) && (0.0..=1.0).contains(&cy)) {
        return None;
    }
    let (pa, pb) = pair.points(cx, cy)?;
    if !(saddle_value.abs() * pa.distance(pb) <= 0.5 * tolerance) {
        return None;
    }
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
    let reach = step.max(2.0 * far);
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
    if !(det < 0.0) {
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

/// The sheet between the rails `a` and `b`. `a` is run as given; `b` as
/// given, or the other way when only that gives a developable (rails
/// drawn in opposite directions). When both ways give one, the one of
/// fewer faces (fewer fans), and as given when they tie: a sheet of one
/// face of rulings is taken at once. Since fans close the folds of the
/// condition (rulings that would cross), the way the rails were not meant
/// to be run may give a sheet too, of fans the other way does not need.
pub fn connect(a: &Rail, b: &Rail, tolerance: &Tolerance) -> Result<Connected, KernelError> {
    let given = connect_as_given(a, b, tolerance);
    if let Ok(made) = &given
        && made.faces.len() == 1
    {
        return given;
    }
    if matches!(given, Err(ref e) if !matches!(e, KernelError::Refused { .. })) {
        return given;
    }
    let turned = connect_as_given(a, &b.reversed(), tolerance).map(|made| Connected {
        reversed_b: true,
        ..made
    });
    match (given, turned) {
        (Ok(g), Ok(t)) => Ok(if t.faces.len() < g.faces.len() { t } else { g }),
        (Ok(g), Err(_)) => Ok(g),
        (Err(_), Ok(t)) => Ok(t),
        (Err(first), Err(_)) => Err(first),
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
    for (name, start, end) in [("first", a0, a1), ("second", b0, b1)] {
        if start.distance(end) <= tol {
            return Err(refused(format!(
                "the {name} curve is closed (it ends where it starts, at {}); a developable between closed curves (a tube) is not built yet: connect two open stretches",
                fmt_point(start)
            )));
        }
    }
    let tips = [a0.distance(b0) <= tol, a1.distance(b1) <= tol];
    if tips[0] && tips[1] && a.len() == b.len() && a == b {
        return Err(refused("the two curves are the same".to_string()));
    }
    let (length_a, length_b) = (
        a.length_between(0.0, a.len()),
        b.length_between(0.0, b.len()),
    );
    if !(length_a > tol && length_b > tol) {
        return Err(refused(format!(
            "a curve is no longer than the tolerance (lengths {length_a} and {length_b})"
        )));
    }
    if let Some(frame) = common_plane(a, b, tol) {
        return planar(a, b, frame, tolerance, tips);
    }
    let pair = Pair { a, b };

    // The start: a fan from an end to the first ruling that satisfies the
    // condition, unless the corner's ruling does.
    let start = if tips[0] {
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
            None => (start_fan(&pair)?.0, None),
        }
    } else if corner_reach(&pair) <= tol {
        // One test at both ends: a fan that would cover no more than the
        // tolerance of its rail is no fan, and the first ruling is the
        // corner's (as the last is snapped to the far corner, below).
        // The root by the corner is found by a Newton step from it along
        // each side: a sampled search of a side cannot see a root that
        // lies on the corner itself.
        (Start::Corner, None)
    } else {
        match start_fan(&pair)? {
            (_, covered) if covered <= tol => (Start::Corner, Some((0.0, 0.0))),
            (kind, _) => (kind, None),
        }
    };
    let (kind, first) = start;
    let snapped = matches!(first, Some((0.0, 0.0)));
    let origin = match kind {
        Start::Corner if snapped => start_root(&pair)?,
        Start::Corner => first.unwrap_or((0.0, 0.0)),
        Start::FromA(y) => (0.0, y),
        Start::FromB(x) => (x, 0.0),
    };
    let mut path = trace(&pair, origin, tol, tips, REACH * tol)?;
    if snapped {
        if let Some(head) = path.points.first_mut() {
            *head = (0.0, 0.0);
        }
    } else if first.is_some() {
        path.points.insert(0, (0.0, 0.0));
        path.fan_after.insert(0, false);
    }
    let &(ex, ey) = path.points.last().ok_or_else(no_point)?;
    // The end: the rest of a rail as a fan from the other's last end,
    // unless what is left is within the tolerance.
    let rest_a = a.length_between(pair.sa(ex), a.len());
    let rest_b = b.length_between(pair.tb(ey), b.len());
    let end = if rest_a <= tol && rest_b <= tol {
        if let Some(last) = path.points.last_mut() {
            *last = (1.0, 1.0);
        }
        None
    } else if ex >= 1.0 {
        Some(Start::FromA(ey))
    } else {
        Some(Start::FromB(ex))
    };
    let path = thinned(&pair, path, tol);
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
    strips.extend(middle_strips(&rulings, &path.fan_after));
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
    build(&pair, strips, tolerance, tips)
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
            || out.fan_after.get(n - 3).copied().unwrap_or(true)
            || out.fan_after.get(n - 2).copied().unwrap_or(true);
        if fixed || moved(p1, p2) >= 0.5 * moved(p0, p1) {
            break;
        }
        out.points.remove(n - 2);
        out.fan_after.remove(n - 2);
        fixed_out.remove(n - 2);
    }
    out
}

/// The faces between the start and the end: the path split at its fans,
/// each run of rulings a face and each fan a face. The first run of
/// rulings is [`Piece::Rulings`]; every other face, in order along the
/// rails, is [`Piece::Inner`] with its number.
fn middle_strips(rulings: &[(f64, f64)], fan_after: &[bool]) -> Vec<Strip> {
    let mut strips: Vec<Strip> = Vec::new();
    let mut run: Vec<(f64, f64)> = Vec::new();
    let close = |run: &mut Vec<(f64, f64)>, strips: &mut Vec<Strip>| {
        if let (Some(&(s0, t0)), Some(&(s1, t1)), true) = (run.first(), run.last(), run.len() >= 2)
        {
            strips.push(Strip {
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
            });
        }
        run.clear();
    };
    for (k, &p) in rulings.iter().enumerate() {
        run.push(p);
        if fan_after.get(k).copied().unwrap_or(false)
            && let Some(&q) = rulings.get(k + 1)
        {
            close(&mut run, &mut strips);
            // At one `t` (apex on B) or at one `s` (apex on A).
            let (lower, upper) = if p.1 == q.1 {
                ((p.0, q.0), (p.1, p.1))
            } else {
                ((p.0, p.0), (p.1, q.1))
            };
            strips.push(Strip {
                piece: Piece::Inner(0),
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
            });
        }
    }
    close(&mut run, &mut strips);
    // Numbered in order, the first run of rulings apart.
    let mut first = true;
    let mut number = 0;
    for strip in &mut strips {
        if first && !strip.rulings.is_empty() {
            strip.piece = Piece::Rulings;
            first = false;
        } else {
            strip.piece = Piece::Inner(number);
            number += 1;
        }
    }
    strips
}

fn no_point() -> KernelError {
    refused("a curve has a point that is not finite".to_string())
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
) -> Result<Connected, KernelError> {
    let pair = Pair { a, b };
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

/// The surface of degree 3 (or less, for few points) along and 1 across
/// whose rows interpolate `points` at `params`.
fn fitted(params: &[f64], points: &[(Vec3, Vec3)]) -> Result<NurbsSurface, KernelError> {
    let degree = 3.min(points.len().saturating_sub(1)).max(1);
    let lower: Vec<Vec<f64>> = points.iter().map(|(p, _)| vec![p.x, p.y, p.z]).collect();
    let upper: Vec<Vec<f64>> = points.iter().map(|(_, q)| vec![q.x, q.y, q.z]).collect();
    let (knots, lo) = interpolate(params, degree, &lower).map_err(KernelError::Geometry)?;
    let (_, hi) = interpolate(params, degree, &upper).map_err(KernelError::Geometry)?;
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
        degree,
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
    let mut all_rulings = Vec::new();
    for strip in &strips {
        let mut rulings = strip_rulings(strip, 16);
        let mut fit = None;
        for _round in 0..MAX_ROUNDS {
            let points: Vec<(Vec3, Vec3)> = rulings
                .iter()
                .map(|&(s, t)| Some((pair.a.point(s)?, pair.b.point(t)?)))
                .collect::<Option<_>>()
                .ok_or_else(no_point)?;
            let params = path_params(&points)
                .ok_or_else(|| refused("the rulings do not move along the curves".to_string()))?;
            let surface = fitted(&params, &points)?;
            // Where the fit is farther than half the tolerance from the
            // true developable, a ruling is added at the middle.
            // Per interval: the true ruling at its middle, and whether the
            // fit is off there.
            let mut middles: Vec<(f64, f64)> = Vec::new();
            let mut bad: Vec<bool> = Vec::new();
            let mut worst: f64 = 0.0;
            for (k, w) in rulings.windows(2).enumerate() {
                // `windows(2)` gives pairs and `params` has one per ruling.
                let ([(s0, t0), (s1, t1)], Some(&u0), Some(&u1)) =
                    (w, params.get(k), params.get(k + 1))
                else {
                    return Err(no_point());
                };
                let um = 0.5 * (u0 + u1);
                let true_ruling = if strip.rulings.is_empty() {
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
    sheet(
        pair,
        &strips,
        surfaces,
        tolerance,
        tips,
        deviation,
        all_rulings,
    )
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
        let key = (s.to_bits(), t.to_bits());
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
        curvature,
        planar: false,
        reversed_b: false,
    })
}
