//! Exact intersection curves (lines and conics) and their clipping to a box.

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::meet::MeetingCurve;
use crate::{Circle, Curve3, Ellipse, GeomError, Line, NurbsCurve};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use std::f64::consts::TAU;

/// A curve with a closed-form parametrisation.
#[derive(Clone, Debug)]
pub(super) enum Shape {
    Line(Line),
    Circle(Circle),
    Ellipse(Ellipse),
    /// `o + d1 t + d2 t²`.
    Parabola {
        o: Vec3,
        d1: Vec3,
        d2: Vec3,
    },
    /// One branch of a hyperbola: `c + e1 cosh s + e2 sinh s`.
    Hyperbola {
        c: Vec3,
        e1: Vec3,
        e2: Vec3,
    },
    /// `o + a cos ψ + b cos² ψ + c sin ψ √(1 + cos² ψ)`: the closed curve
    /// in which two tubes of one radius cross whose spines, a line and a
    /// circle, are tangent (see `exact::continued`). It has no exact
    /// NURBS: a piece of it is fitted, and `slack` is how far the surfaces
    /// are from the position the formula is for.
    Lobe {
        o: Vec3,
        a: Vec3,
        b: Vec3,
        c: Vec3,
        slack: f64,
    },
}

/// The samples per turn in which a [`Shape::Lobe`] is searched for the
/// places where it passes a side of the box.
const LOBE_SAMPLES: u32 = 720;
/// The most spans a fitted piece of a [`Shape::Lobe`] may need.
const LOBE_SPANS: usize = 1 << 14;

/// An exact curve and the part of it that is meant; the full turn
/// `(0, 2π)` for a whole circle or ellipse, infinite ends for a whole line,
/// parabola or hyperbola.
#[derive(Clone, Debug)]
pub(super) struct ExactCurve {
    pub shape: Shape,
    pub range: (f64, f64),
    pub tangent: bool,
}

impl ExactCurve {
    pub(super) fn whole(shape: Shape, tangent: bool) -> Self {
        let range = match shape {
            Shape::Circle(_) | Shape::Ellipse(_) => (0.0, TAU),
            _ => (f64::NEG_INFINITY, f64::INFINITY),
        };
        Self {
            shape,
            range,
            tangent,
        }
    }

    pub(super) fn from_curve(curve: &Curve3, tangent: bool) -> Option<Self> {
        let shape = match curve {
            Curve3::Line(l) => Shape::Line(*l),
            Curve3::Circle(c) => Shape::Circle(*c),
            Curve3::Ellipse(e) => Shape::Ellipse(*e),
            Curve3::Nurbs(_) => return None,
        };
        Some(Self::whole(shape, tangent))
    }
}

impl Shape {
    fn is_trig(&self) -> bool {
        matches!(self, Shape::Circle(_) | Shape::Ellipse(_))
    }

    /// `center + x cos t + y sin t` of a circle or ellipse.
    fn trig(&self) -> Option<(Vec3, Vec3, Vec3)> {
        match self {
            Shape::Circle(c) => {
                let f = c.frame();
                Some((f.origin(), f.x() * c.radius(), f.y() * c.radius()))
            }
            Shape::Ellipse(e) => {
                let (f, (a, b)) = (e.frame(), e.semi_axes());
                Some((f.origin(), f.x() * a, f.y() * b))
            }
            _ => None,
        }
    }

    pub(super) fn eval(&self, t: f64) -> Vec3 {
        match self {
            Shape::Line(l) => l.origin() + l.direction() * t,
            Shape::Circle(_) | Shape::Ellipse(_) => {
                let (c, x, y) = self.trig().unwrap_or((Vec3::ZERO, Vec3::ZERO, Vec3::ZERO));
                let (s, co) = t.sin_cos();
                c + x * co + y * s
            }
            Shape::Parabola { o, d1, d2 } => *o + *d1 * t + *d2 * (t * t),
            Shape::Hyperbola { c, e1, e2 } => *c + *e1 * t.cosh() + *e2 * t.sinh(),
            Shape::Lobe { o, a, b, c, .. } => {
                let (s, co) = t.sin_cos();
                *o + *a * co + *b * (co * co) + *c * (s * (1.0 + co * co).sqrt())
            }
        }
    }

    /// The derivative of a [`Shape::Lobe`] by its parameter; it is never
    /// zero where `a` and `c` are not.
    fn lobe_derivative(&self, t: f64) -> Option<Vec3> {
        let Shape::Lobe { a, b, c, .. } = self else {
            return None;
        };
        let (s, co) = t.sin_cos();
        let w = (1.0 + co * co).sqrt();
        Some(*a * (-s) + *b * (-2.0 * co * s) + *c * (co * w - s * s * co / w))
    }

    /// The parameters at which `pick` of the curve's point equals `bound`.
    fn roots(&self, pick: &dyn Fn(Vec3) -> f64, bound: f64, out: &mut Vec<f64>) {
        let quadratic = |a: f64, b: f64, c: f64, out: &mut Vec<f64>| {
            // a t² + b t + c = 0, in the form that does not cancel.
            if a == 0.0 {
                if b != 0.0 {
                    out.push(-c / b);
                }
                return;
            }
            let disc = b * b - 4.0 * a * c;
            if disc < 0.0 {
                return;
            }
            let q = -0.5 * (b + b.signum() * disc.sqrt());
            if q != 0.0 {
                out.push(c / q);
            }
            out.push(q / a);
        };
        match self {
            Shape::Line(l) => quadratic(0.0, pick(l.direction()), pick(l.origin()) - bound, out),
            Shape::Circle(_) | Shape::Ellipse(_) => {
                let Some((c, x, y)) = self.trig() else {
                    return;
                };
                let (a, b) = (pick(x), pick(y));
                let r = a.hypot(b);
                let h = (bound - pick(c)) / r;
                if !(r > 0.0 && h.abs() <= 1.0) {
                    return;
                }
                let (phase, spread) = (b.atan2(a), h.acos());
                out.push(phase - spread);
                out.push(phase + spread);
            }
            Shape::Parabola { o, d1, d2 } => quadratic(pick(*d2), pick(*d1), pick(*o) - bound, out),
            Shape::Hyperbola { c, e1, e2 } => {
                // With y = e^s: (β + γ)/2 y² + (c - bound) y + (β - γ)/2 = 0.
                let (beta, gamma) = (pick(*e1), pick(*e2));
                let mut ys = Vec::new();
                quadratic(
                    0.5 * (beta + gamma),
                    pick(*c) - bound,
                    0.5 * (beta - gamma),
                    &mut ys,
                );
                out.extend(ys.into_iter().filter(|y| *y > 0.0).map(f64::ln));
            }
            Shape::Lobe { .. } => {
                // No closed form: a change of sign between samples, then
                // bisection. (A curve that only touches a side of the box
                // from inside is not cut there, which loses nothing.) Two
                // turns, for a range that starts anywhere in the first.
                let g = |t: f64| pick(self.eval(t)) - bound;
                let step = TAU / f64::from(LOBE_SAMPLES);
                let start = -TAU;
                let mut before = g(start);
                for k in 1..=3 * LOBE_SAMPLES {
                    let (t0, t1) = (start + step * f64::from(k - 1), start + step * f64::from(k));
                    let after = g(t1);
                    if before == 0.0 {
                        out.push(t0);
                    } else if (before < 0.0) != (after < 0.0) && after != 0.0 {
                        let (mut lo, mut hi) = (t0, t1);
                        for _ in 0..60 {
                            let mid = 0.5 * (lo + hi);
                            if (g(mid) < 0.0) == (before < 0.0) {
                                lo = mid;
                            } else {
                                hi = mid;
                            }
                        }
                        out.push(0.5 * (lo + hi));
                    }
                    before = after;
                }
            }
        }
    }

    /// The C¹ cubic within a quarter of the tolerance of the piece
    /// `[t0, t1]` of a [`Shape::Lobe`], the length of its parameter range
    /// and how far it is from the curve where that was measured: the spans
    /// are halved until every one is that near at seven places, each held
    /// against the nearest point of the curve itself.
    fn lobe_arc(
        &self,
        t0: f64,
        t1: f64,
        tol: &Tolerance,
    ) -> Result<(NurbsCurve, f64, f64), GeomError> {
        let degenerate = || GeomError::InvalidParameter("an intersection has no finite position");
        let target = 0.25 * tol.linear;
        let mut spans = 4_usize;
        loop {
            let mut nodes = Vec::with_capacity(spans + 1);
            for k in 0..=spans {
                let t = if k == spans {
                    t1
                } else {
                    t0 + (t1 - t0) * (k as f64 / spans as f64)
                };
                let tangent = self
                    .lobe_derivative(t)
                    .and_then(Vec3::normalized)
                    .ok_or_else(degenerate)?;
                nodes.push((t, self.eval(t), tangent));
            }
            let mut knots = vec![0.0; 4];
            let mut ctrl = Vec::with_capacity(3 * spans + 1);
            let (mut total, mut worst) = (0.0, 0.0_f64);
            for ((ta, pa, da), (tb, pb, db)) in nodes.iter().zip(nodes.iter().skip(1)) {
                let chord = pa.distance(*pb);
                if !(chord > 0.0) {
                    return Err(degenerate());
                }
                // The handle that makes a circular arc of the same turn
                // exact to sixth order, as the marcher's spans have it.
                let theta = da.dot(*db).clamp(-1.0, 1.0).acos().min(2.0);
                let handle = if theta < 1e-4 {
                    1.0 / 3.0
                } else {
                    (2.0 / 3.0) * (0.25 * theta).tan() / (0.5 * theta).sin()
                };
                let l = chord * handle;
                let (c1, c2) = (*pa + *da * l, *pb - *db * l);
                if ctrl.is_empty() {
                    ctrl.push(*pa);
                } else {
                    knots.push(total);
                    knots.push(total);
                }
                total += 3.0 * l;
                ctrl.extend([c1, c2]);
                for k in 1..8 {
                    let s = f64::from(k) / 8.0;
                    let r = 1.0 - s;
                    let q = *pa * (r * r * r)
                        + c1 * (3.0 * r * r * s)
                        + c2 * (3.0 * r * s * s)
                        + *pb * (s * s * s);
                    // The nearest point of the curve, from the parameter
                    // the same share of the way along the span.
                    let mut t = ta + (tb - ta) * s;
                    for _ in 0..4 {
                        let Some(d) = self.lobe_derivative(t) else {
                            break;
                        };
                        let step = (q - self.eval(t)).dot(d) / d.length_squared();
                        if !step.is_finite() {
                            break;
                        }
                        t += step;
                    }
                    worst = worst.max(q.distance(self.eval(t)));
                }
            }
            if worst <= target {
                let last = nodes.last().ok_or_else(degenerate)?;
                ctrl.push(last.1);
                knots.extend([total; 4]);
                return Ok((NurbsCurve::polynomial(3, knots, ctrl)?, total, worst));
            }
            spans *= 2;
            if spans > LOBE_SPANS {
                return Err(GeomError::NoConvergence(
                    "an intersection curve could not be fitted within the tolerance",
                ));
            }
        }
    }
}

/// The exact NURBS of a parabola over `[t0, t1]`, with the same parameter.
fn parabola_arc(o: Vec3, d1: Vec3, d2: Vec3, t0: f64, t1: f64) -> Result<NurbsCurve, GeomError> {
    let at = |t: f64| o + d1 * t + d2 * (t * t);
    let slope = d1 + d2 * (2.0 * t0);
    NurbsCurve::polynomial(
        2,
        vec![t0, t0, t0, t1, t1, t1],
        vec![at(t0), at(t0) + slope * (0.5 * (t1 - t0)), at(t1)],
    )
}

/// The exact rational quadratic NURBS of a hyperbola branch over
/// `[s0, s1]`, in arcs of at most one unit of `s`. A symmetric arc of half
/// width `d` about `m` has its middle control point where the end tangents
/// meet, `c + E/cosh d` with `E` the radius vector at `m`, and weight
/// `cosh d` (the hyperbolic twin of the circle's `cos d`).
fn hyperbola_arc(c: Vec3, e1: Vec3, e2: Vec3, s0: f64, s1: f64) -> Result<NurbsCurve, GeomError> {
    let width = s1 - s0;
    let pieces = if width > 1.0 {
        width.ceil().min(256.0) as usize
    } else {
        1
    };
    let at = |s: f64| c + e1 * s.cosh() + e2 * s.sinh();
    let step = (s1 - s0) / pieces as f64;
    let (mut knots, mut ctrl, mut weights) = (vec![s0; 3], vec![at(s0)], vec![1.0]);
    for k in 0..pieces {
        let a = s0 + step * k as f64;
        let b = if k + 1 == pieces { s1 } else { a + step };
        let (m, d) = (0.5 * (a + b), 0.5 * (b - a));
        let radius = e1 * m.cosh() + e2 * m.sinh();
        ctrl.push(c + radius * (1.0 / d.cosh()));
        weights.push(d.cosh());
        ctrl.push(at(b));
        weights.push(1.0);
        knots.push(b);
        knots.push(b);
    }
    knots.push(s1);
    NurbsCurve::new(2, knots, ctrl, weights)
}

/// The pieces of an exact curve inside a box.
pub(super) fn clip(
    curve: &ExactCurve,
    within: &Bbox3,
    tol: &Tolerance,
) -> Result<Vec<MeetingCurve>, GeomError> {
    let Some((lo, hi)) = within.corners() else {
        return Ok(Vec::new());
    };
    let shape = &curve.shape;
    let (r0, r1) = curve.range;
    let trig = shape.is_trig();
    let full = trig && (r1 - r0) >= TAU * (1.0 - 1e-14);
    let mut cuts = Vec::new();
    type Pick<'a> = (&'a dyn Fn(Vec3) -> f64, f64, f64);
    let picks: [Pick; 3] = [
        (&|p: Vec3| p.x, lo.x, hi.x),
        (&|p: Vec3| p.y, lo.y, hi.y),
        (&|p: Vec3| p.z, lo.z, hi.z),
    ];
    for (pick, low, high) in picks {
        shape.roots(pick, low, &mut cuts);
        shape.roots(pick, high, &mut cuts);
    }
    let mut stops: Vec<f64> = cuts
        .into_iter()
        .filter(|t| t.is_finite())
        .map(|t| {
            if trig {
                r0 + (t - r0).rem_euclid(TAU)
            } else {
                t
            }
        })
        .filter(|t| full || (*t > r0 && *t < r1))
        .collect();
    if !full {
        stops.extend([r0, r1].into_iter().filter(|t| t.is_finite()));
    }
    stops.sort_by(f64::total_cmp);
    stops.dedup();
    let inside = |t: f64| within.contains(shape.eval(t), tol.linear);
    let mut kept: Vec<(f64, f64)> = Vec::new();
    let mut closed = false;
    if full && stops.is_empty() {
        if inside(0.0) && inside(2.0) && inside(4.0) {
            kept.push((0.0, TAU));
            closed = true;
        }
    } else {
        let mut spans: Vec<(f64, f64)> = stops
            .iter()
            .zip(stops.iter().skip(1))
            .map(|(a, b)| (*a, *b))
            .collect();
        if full && let (Some(first), Some(last)) = (stops.first(), stops.last()) {
            spans.push((*last, *first + TAU));
        }
        for (a, b) in spans {
            if !(b > a) || !inside(0.5 * (a + b)) {
                continue;
            }
            match kept.last_mut() {
                Some(last) if last.1 == a => last.1 = b,
                _ => kept.push((a, b)),
            }
        }
        // Around the seam of a closed curve.
        if full && kept.len() > 1 {
            if let (Some(first), Some(last)) = (kept.first().copied(), kept.last().copied())
                && last.1 == first.0 + TAU
            {
                kept.pop();
                if let Some(head) = kept.first_mut() {
                    *head = (last.0 - TAU, first.1);
                }
            }
        } else if full
            && let Some(only) = kept.first_mut()
            && only.1 - only.0 >= TAU * (1.0 - 1e-14)
        {
            // Cut only where it touches the box from inside.
            *only = (0.0, TAU);
            closed = true;
        }
    }
    let mut out = Vec::new();
    for (a, b) in kept {
        let (pa, pm, pb) = (shape.eval(a), shape.eval(0.5 * (a + b)), shape.eval(b));
        if !closed && pa.distance(pb) <= tol.linear && pa.distance(pm) <= tol.linear {
            continue; // A piece too short to be a curve.
        }
        let (curve3, range) = match shape {
            Shape::Line(l) => (Curve3::Line(*l), (a, b)),
            Shape::Circle(c) => (Curve3::Circle(*c), (a, b)),
            Shape::Ellipse(e) => (Curve3::Ellipse(*e), (a, b)),
            Shape::Parabola { o, d1, d2 } => {
                (Curve3::Nurbs(parabola_arc(*o, *d1, *d2, a, b)?), (a, b))
            }
            Shape::Hyperbola { c, e1, e2 } => {
                (Curve3::Nurbs(hyperbola_arc(*c, *e1, *e2, a, b)?), (a, b))
            }
            Shape::Lobe { slack, .. } => {
                let (fitted, length, deviation) = shape.lobe_arc(a, b, tol)?;
                out.push(MeetingCurve {
                    curve: Curve3::Nurbs(fitted),
                    range: (0.0, length),
                    closed: false,
                    tangent: curve.tangent,
                    exact: false,
                    deviation: deviation + slack,
                });
                continue;
            }
        };
        out.push(MeetingCurve {
            curve: curve3,
            range,
            closed,
            tangent: curve.tangent,
            exact: true,
            deviation: 0.0,
        });
    }
    Ok(out)
}
