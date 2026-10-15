//! Contact of more than the second order along one direction: tacnodes
//! and cusps.
//!
//! Where the Hessian of `f` has rank one at a point where the surfaces
//! touch, `f` is, after a change of coordinates (the splitting lemma),
//! `λ/2 s² + g(t)`: `s` across, along the eigenvector of the Hessian's
//! eigenvalue `λ`, and `t` along the direction in which it vanishes, with
//! `g` the value of `f` along its *valley* (the curve on which the
//! derivative across is zero) and of order three or more. On a side of the
//! point where `g` has the sign opposite to `λ` the zero set is two
//! curves, one on either side of the valley, `s ≈ ± sqrt(−2 g / λ)`; on a
//! side where it has the sign of `λ`, nothing. So the point has four arms
//! (a tacnode: a cylinder touching the top of a torus' tube from inside,
//! `g ∝ t⁴`), two that leave it in the same direction (a cusp, `g ∝ t³`),
//! or none (a touch, which [`Marcher::one_sided`] settles).
//!
//! The arms are curves that leave the point tangent to each other, closer
//! than the tolerance for a while, and neither the cells nor a march can
//! tell them apart there: around the point, in a disc of radius `Z`, they
//! are made instead, as graphs over `t`. On the line across at `t` the
//! valley is found by Newton's method on the derivative across, and the
//! arm on either side of it by bracketing from there outwards. The disc is
//! as small as it can be while the arms at its rim are 64 tolerances apart
//! and `g` there is clear of zero, and the model is checked on lines across
//! out to twice that radius: on every line `f` is monotonic away from the
//! valley (so the line holds the two arms or nothing) and `g` keeps the
//! sign the rim says. Seeds on the arms at the rim are marched from there;
//! a march that comes back within a quarter more than the radius ends on
//! the point, and the arms inside are made by the graphs.
//!
//! Where the surfaces are a little apart at the point (`f = δ` there,
//! within the tolerance), the arms are those of `f − δ w(t / Z)` with
//! `w(x) = (1 − x⁴)²` for `|x| < 1` and nothing beyond: that function has
//! the contact itself at the point, equals `f` from the rim on, and its arms
//! are within `|δ|` of both surfaces. (As a crossing a little apart is
//! drawn to its point.)

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::{Eval, Marcher, Pt, Stationary};
use craft3d_math::Vec3;

/// A point of contact with arms, and what makes its arms.
#[derive(Clone, Debug)]
pub(super) struct Tacnode {
    pub(super) u: f64,
    pub(super) v: f64,
    pub(super) p: Vec3,
    /// The lengths of the chart's derivatives at the point: the arms are
    /// made in `(Δu lu, Δv lv)`.
    lu: f64,
    lv: f64,
    /// Unit directions across the arms (the Hessian's eigenvector of
    /// `lambda`) and along them.
    across: (f64, f64),
    along: (f64, f64),
    lambda: f64,
    /// `f` at the point.
    lift: f64,
    /// The radius of the disc inside which the arms are made.
    pub(super) radius: f64,
    /// How far along the valley the gradient of `f` stays below what
    /// counts as tangent: a search for a stationary point that ends there
    /// has found this point again.
    claim: f64,
    /// Seeds on the arms at the rim of the disc.
    pub(super) seeds: Vec<(f64, f64)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    /// Two arms leave the point on this side.
    Arms,
    /// None.
    Bare,
    /// Not clear at this radius.
    Unclear,
}

/// The eigenvalue of the larger magnitude of `[a b; b c]` and its unit
/// eigenvector.
fn strong(a: f64, b: f64, c: f64) -> Option<(f64, (f64, f64))> {
    let mean = 0.5 * (a + c);
    let r = (0.25 * (a - c) * (a - c) + b * b).sqrt();
    let lambda = if mean >= 0.0 { mean + r } else { mean - r };
    // Two forms of the eigenvector; the longer one is the better
    // conditioned.
    let (x1, y1) = (lambda - c, b);
    let (x2, y2) = (b, lambda - a);
    let (x, y) = if x1.hypot(y1) >= x2.hypot(y2) {
        (x1, y1)
    } else {
        (x2, y2)
    };
    let len = x.hypot(y);
    (lambda.is_finite() && lambda != 0.0 && len > 0.0 && len.is_finite())
        .then_some((lambda, (x / len, y / len)))
}

impl Marcher {
    /// The chart's parameters of the place `t` along and `s` across.
    fn tac_place(k: &Tacnode, t: f64, s: f64) -> (f64, f64) {
        (
            k.u + (t * k.along.0 + s * k.across.0) / k.lu,
            k.v + (t * k.along.1 + s * k.across.1) / k.lv,
        )
    }

    /// `(t, s)` of a place of the chart.
    fn tac_coords(&self, k: &Tacnode, u: f64, v: f64) -> (f64, f64) {
        let c = &self.chart;
        let u = Self::unwrap(u, k.u, c.periodic_u, c.u.1 - c.u.0);
        let v = Self::unwrap(v, k.v, c.periodic_v, c.v.1 - c.v.0);
        let (x, y) = ((u - k.u) * k.lu, (v - k.v) * k.lv);
        (
            x * k.along.0 + y * k.along.1,
            x * k.across.0 + y * k.across.1,
        )
    }

    /// `w(t / Z)` and its derivative in `t`.
    fn tac_weight(k: &Tacnode, t: f64) -> (f64, f64) {
        let x = t / k.radius;
        if !(x.abs() < 1.0) || k.lift == 0.0 {
            return (0.0, 0.0);
        }
        let q = 1.0 - x.powi(4);
        (q * q, -8.0 * q * x.powi(3) / k.radius)
    }

    /// The modified distance at `(t, s)`, its derivative across, and the
    /// evaluation with the gradient of the modified distance.
    fn tac_eval(&mut self, k: &Tacnode, t: f64, s: f64) -> Option<(Eval, f64, f64, f64)> {
        let (u, v) = Self::tac_place(k, t, s);
        let mut e = self.eval(u, v)?;
        let (w, dw) = Self::tac_weight(k, t);
        e.f -= k.lift * w;
        e.gu -= k.lift * dw * k.lu * k.along.0;
        e.gv -= k.lift * dw * k.lv * k.along.1;
        let slope = e.gu / k.lu * k.across.0 + e.gv / k.lv * k.across.1;
        Some((e, slope, u, v))
    }

    /// The place across at `t` where the derivative across vanishes, by
    /// Newton's method with the Hessian at the point.
    fn tac_valley(&mut self, k: &Tacnode, t: f64) -> Option<(f64, Eval)> {
        let mut s = 0.0;
        let limit = 4.0 * t.abs() + 64.0 * self.tol.linear;
        for _ in 0..48 {
            let (e, slope, _, _) = self.tac_eval(k, t, s)?;
            let ds = -slope / k.lambda;
            if !ds.is_finite() {
                return None;
            }
            if ds.abs() <= 0.01 * self.on {
                return Some((s, e));
            }
            s += ds;
            if !(s.abs() <= limit) {
                return None;
            }
        }
        None
    }

    /// The arm on side `side` (±1) of the valley at `t`: `None` where the
    /// line across holds no arm.
    fn tac_root(&mut self, k: &Tacnode, t: f64, side: f64) -> Option<(f64, Eval, f64, f64)> {
        let (valley, e0) = self.tac_valley(k, t)?;
        let sign = k.lambda.signum();
        if !(e0.f * sign < 0.0) {
            return None;
        }
        // From the valley outwards to where `f` has the sign of `λ`.
        let guess = (2.0 * e0.f.abs() / k.lambda.abs()).sqrt().max(self.on);
        let (mut lo, mut hi) = (0.0, 2.0 * guess);
        let mut found = false;
        for _ in 0..24 {
            let (e, _, _, _) = self.tac_eval(k, t, valley + side * hi)?;
            if e.f * sign >= 0.0 {
                found = true;
                break;
            }
            lo = hi;
            hi *= 2.0;
            if !(hi <= k.radius + 4.0 * t.abs()) {
                return None;
            }
        }
        if !found {
            return None;
        }
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            let (e, _, u, v) = self.tac_eval(k, t, valley + side * mid)?;
            if e.f.abs() <= self.on || (hi - lo) <= 1e-3 * self.on {
                return Some((valley + side * mid, e, u, v));
            }
            if e.f * sign < 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        None
    }

    /// The point of the arm on side `side` at `t`, travelling along
    /// `toward` (or against it, when that is how the arm runs).
    fn tac_point(&mut self, k: &Tacnode, t: f64, side: f64, toward: Vec3) -> Option<Pt> {
        let (_, e, u, v) = self.tac_root(k, t, side)?;
        let (tu, tv) = (-e.gv, e.gu);
        let dir = e.su * tu + e.sv * tv;
        let len = dir.length();
        if !(len > 0.0 && len.is_finite()) {
            return None;
        }
        let sign = if dir.dot(toward) >= 0.0 { 1.0 } else { -1.0 };
        Some(Pt {
            u,
            v,
            p: e.p,
            t: dir * (sign / len),
            du: tu * sign / len,
            dv: tv * sign / len,
        })
    }

    /// A Newton search that took no step because the Hessian is nearly
    /// singular, carried on: along the valley of the strong direction to
    /// where the derivative along it changes sign. `None` where there is
    /// no such place nearby.
    pub(super) fn settle_flat(&mut self, s: &Stationary) -> Option<Stationary> {
        let (ha, hb, hc) = s.h;
        let (lambda, across) = strong(ha, hb, hc)?;
        let k = Tacnode {
            u: s.u,
            v: s.v,
            p: s.e.p,
            lu: s.lu,
            lv: s.lv,
            across,
            along: (-across.1, across.0),
            lambda,
            lift: 0.0,
            radius: 0.0,
            claim: 0.0,
            seeds: Vec::new(),
        };
        let slope_along = |m: &mut Self, t: f64| -> Option<(f64, f64, Eval)> {
            let (sv, e) = m.tac_valley(&k, t)?;
            Some((e.gu / k.lu * k.along.0 + e.gv / k.lv * k.along.1, sv, e))
        };
        let (g0, _, _) = slope_along(self, 0.0)?;
        let mut bracket = None;
        let mut d = 4.0 * self.on;
        let limit = 2.0 * self.hmax;
        'grow: while d <= limit {
            for dir in [1.0, -1.0] {
                if let Some((g, _, _)) = slope_along(self, dir * d)
                    && g * g0 <= 0.0
                {
                    bracket = Some((0.0, dir * d));
                    break 'grow;
                }
            }
            d *= 2.0;
        }
        let (mut lo, mut hi) = bracket?;
        for _ in 0..100 {
            if (hi - lo).abs() <= self.on {
                break;
            }
            let mid = 0.5 * (lo + hi);
            let (g, _, _) = slope_along(self, mid)?;
            if g * g0 > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (_, sv, _) = slope_along(self, 0.5 * (lo + hi))?;
        let (u, v) = Self::tac_place(&k, 0.5 * (lo + hi), sv);
        self.stationary_at(u, v)
    }

    /// The arms of a point where the surfaces touch and the Hessian of `f`
    /// has rank one: `None` where there are none (a touch, or a contact
    /// along a curve) or the model does not hold.
    pub(super) fn tacnode(&mut self, s: &Stationary, finest: f64) -> Option<Tacnode> {
        let (ha, hb, hc) = s.h;
        let (lambda, across) = strong(ha, hb, hc)?;
        let mut k = Tacnode {
            u: s.u,
            v: s.v,
            p: s.e.p,
            lu: s.lu,
            lv: s.lv,
            across,
            along: (-across.1, across.0),
            lambda,
            lift: s.e.f,
            radius: 0.0,
            claim: 0.0,
            seeds: Vec::new(),
        };
        let sign = lambda.signum();
        let c = &self.chart;
        let widest = 0.25 * ((c.u.1 - c.u.0) * s.lu).min((c.v.1 - c.v.0) * s.lv);
        let clear = 64.0 * self.on;
        let apart = 64.0 * self.tol.linear;
        // The smallest radius at which each side is clear.
        let mut z = 8.0 * finest;
        let sides = loop {
            if !(z.is_finite() && z <= widest) {
                return None;
            }
            k.radius = z;
            let mut sides = [Side::Unclear; 2];
            for (slot, dir) in sides.iter_mut().zip([1.0, -1.0]) {
                let (_, e) = self.tac_valley(&k, dir * z)?;
                let depth = e.f * sign;
                *slot = if depth < -clear && -depth >= 4.0 * k.lift.abs() {
                    match (
                        self.tac_root(&k, dir * z, 1.0),
                        self.tac_root(&k, dir * z, -1.0),
                    ) {
                        (Some((a, ..)), Some((b, ..))) if (a - b).abs() >= apart => Side::Arms,
                        (Some(_), Some(_)) => Side::Unclear,
                        _ => return None,
                    }
                } else if depth > clear && depth >= 4.0 * k.lift.abs() {
                    Side::Bare
                } else {
                    Side::Unclear
                };
            }
            if sides.iter().all(|s| *s == Side::Bare) {
                return None; // A touch.
            }
            if sides.iter().all(|s| *s != Side::Unclear) {
                break sides;
            }
            z *= 2.0;
        };
        // The model, on lines across out to twice the radius.
        const LINES: u32 = 32;
        for (side, dir) in sides.iter().zip([1.0, -1.0]) {
            for j in 1..=LINES {
                self.spent().ok()?;
                let t = dir * 2.0 * z * f64::from(j) / f64::from(LINES);
                let (valley, e) = self.tac_valley(&k, t)?;
                let depth = e.f * sign;
                match side {
                    Side::Arms => {
                        if depth > self.on {
                            return None;
                        }
                        if depth < -self.on {
                            for across in [1.0, -1.0] {
                                let (at, ..) = self.tac_root(&k, t, across)?;
                                // Arms that keep close to the direction
                                // along where a march coming in ends on
                                // the point.
                                if t.abs() <= 1.25 * z && !(at.abs() <= 0.15 * t.abs()) {
                                    return None;
                                }
                            }
                        }
                    }
                    _ => {
                        if depth < -self.on {
                            return None;
                        }
                    }
                }
                // Away from the valley `f` heads for the sign of `λ`: the
                // line holds the two arms or nothing.
                for q in [-1.0, -0.5, 0.5, 1.0] {
                    let (_, slope, _, _) = self.tac_eval(&k, t, valley + q * z)?;
                    if !(slope * sign * q > 0.0) {
                        return None;
                    }
                }
            }
        }
        // Where along the valley the gradient becomes more than tangent.
        let (ha, hb, hc) = s.h;
        let size = ha.abs().max(hb.abs()).max(hc.abs());
        let tangent = 1e-6_f64.max(4.0 * size * self.tol.linear);
        let mut claim = 2.0 * z;
        while claim < widest {
            let mut steep = true;
            for dir in [1.0, -1.0] {
                if let Some((_, e)) = self.tac_valley(&k, dir * claim) {
                    let along = e.gu / k.lu * k.along.0 + e.gv / k.lv * k.along.1;
                    steep &= along.abs() > tangent;
                }
            }
            if steep {
                break;
            }
            claim *= 2.0;
        }
        k.claim = claim;
        for (side, dir) in sides.iter().zip([1.0, -1.0]) {
            if *side == Side::Arms {
                for across in [1.0, -1.0] {
                    let (_, _, u, v) = self.tac_root(&k, dir * z, across)?;
                    k.seeds.push((u, v));
                }
            }
        }
        Some(k)
    }

    /// True when a stationary point at `(u, v)` is a tacnode's own: on its
    /// valley, where the gradient is no more than tangent.
    pub(super) fn tac_claims(&self, u: f64, v: f64) -> bool {
        self.tacnodes.iter().any(|k| {
            let (t, s) = self.tac_coords(k, u, v);
            t.abs() <= k.claim && s.abs() <= k.radius
        })
    }

    /// True when `(u, v)` lies nearer to a tacnode along its arms than
    /// their seeds: there the arms are made, and a seed that cells found
    /// is not marched.
    pub(super) fn tac_inside(&self, u: f64, v: f64) -> bool {
        self.tacnodes.iter().any(|k| {
            let (t, s) = self.tac_coords(k, u, v);
            t.abs() < 0.999 * k.radius && s.abs() <= k.radius
        })
    }

    /// The point halfway between `a` and `b` along an arm of a tacnode,
    /// when both lie where its arms are made: `Some(None)` when that fails.
    pub(super) fn tac_between(&mut self, a: &Pt, b: &Pt) -> Option<Option<Pt>> {
        let tacnodes = self.tacnodes.clone();
        for k in &tacnodes {
            let (ta, sa) = self.tac_coords(k, a.u, a.v);
            let (tb, sb) = self.tac_coords(k, b.u, b.v);
            let reach = 2.0 * k.radius * (1.0 + 1e-9);
            if !(ta.abs() <= reach
                && tb.abs() <= reach
                && sa.abs() <= k.radius
                && sb.abs() <= k.radius)
                || (ta * tb < 0.0 && ta.abs().min(tb.abs()) > 1e-9 * k.radius)
            {
                continue;
            }
            // The side across is the one of the end farther out.
            let (t, s) = if ta.abs() >= tb.abs() {
                (ta, sa)
            } else {
                (tb, sb)
            };
            if t == 0.0 {
                continue;
            }
            let Some((valley, _)) = self.tac_valley(k, t) else {
                return Some(None);
            };
            let side = if s >= valley { 1.0 } else { -1.0 };
            return Some(self.tac_point(k, 0.5 * (ta + tb), side, b.p - a.p));
        }
        None
    }

    /// The step a march may take at `p` near a tacnode: a tenth of the
    /// distance to it, so that it keeps to its arm where the other arm is
    /// close.
    pub(super) fn tac_step(&self, p: Vec3, h: f64) -> f64 {
        self.tacnodes.iter().fold(h, |h, k| {
            let d = p.distance(k.p);
            if d <= 64.0 * k.radius {
                h.min(0.1 * d)
            } else {
                h
            }
        })
    }
}
