//! The poles of a freeform surface: borders of its parameter range that
//! are one point in space.
//!
//! A Bézier or NURBS surface may have a whole border in one point: the
//! pole of a dome swept about an axis, the apex of a ruled patch, the top
//! of the teapot's lid. The parametrisation stalls there (the derivative
//! along the border is zero, the normal `∂u × ∂v` has no length), but the
//! surface itself is as good a surface as anywhere when it closes up
//! smoothly, and a cone-like point when it does not. What reads the
//! surface through its parameters asks here what it cannot ask there.
//!
//! A border is a pole when its control points are all one point, which is
//! what a border that is one point comes to (the basis functions of a
//! border are independent). One point within [`SAME`] of the size of the
//! control net: control points that differ by rounding are one.

use crate::{RationalBezierPatch, Surface};
use craft3d_math::Portable;
use craft3d_math::{Bbox3, Vec3};

/// Control points this share of the size of the control net apart are one.
const SAME: f64 = 1e-9;
/// How far from a pole, as a share of the parameter range across it, the
/// surface is looked at for what holds at the pole in the limit.
const NUDGE: f64 = 1e-6;
/// Within this share of the parameter range of a pole's border a point of
/// the surface is the pole: nearer than that the derivative along the
/// border is rounding.
const AT: f64 = 1e-9;
/// Places along a border at which the meridians are compared before the
/// best is narrowed down.
const LOOKS: u32 = 32;

/// A border of the parameter range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Border {
    UStart,
    UEnd,
    VStart,
    VEnd,
}

/// A border of a freeform surface that is one point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pole {
    pub border: Border,
    pub point: Vec3,
}

/// The poles of a Bézier or NURBS surface, in the order of [`Border`];
/// none for the other kinds, whose poles and apexes are known by their
/// formulas.
pub fn poles(surface: &Surface) -> Vec<Pole> {
    let (points, count_v): (&[Vec3], usize) = match surface {
        Surface::Bezier(patch) => (patch.control_points(), patch.degree_v() + 1),
        Surface::Nurbs(s) => (s.control_points(), s.counts().1),
        _ => return Vec::new(),
    };
    if count_v == 0 || !points.len().is_multiple_of(count_v) || points.len() < 2 * count_v {
        return Vec::new();
    }
    let ((u0, u1), (v0, v1)) = surface.domain();
    if ![u0, u1, v0, v1].iter().all(|x| x.is_finite()) {
        return Vec::new();
    }
    let count_u = points.len() / count_v;
    let same = SAME * Bbox3::from_points(points.iter().copied()).diagonal();
    let one_point = |border: &mut dyn Iterator<Item = usize>| -> Option<Vec3> {
        let first = *points.get(border.next()?)?;
        for index in border {
            let apart = points.get(index)?.distance(first);
            // A NaN is no point.
            if apart.is_nan() || apart > same {
                return None;
            }
        }
        first.is_finite().then_some(first)
    };
    let mut poles = Vec::new();
    let mut look = |border: Border, indices: &mut dyn Iterator<Item = usize>| {
        if let Some(point) = one_point(indices) {
            poles.push(Pole { border, point });
        }
    };
    look(Border::UStart, &mut (0..count_v));
    look(
        Border::UEnd,
        &mut (0..count_v).map(|j| (count_u - 1) * count_v + j),
    );
    look(Border::VStart, &mut (0..count_u).map(|i| i * count_v));
    look(
        Border::VEnd,
        &mut (0..count_u).map(|i| i * count_v + count_v - 1),
    );
    poles
}

impl Pole {
    /// Whether the border is one of constant v (the surface does not move
    /// with u along it).
    pub fn across_v(&self) -> bool {
        matches!(self.border, Border::VStart | Border::VEnd)
    }

    /// The range of the parameter that runs along the border, the
    /// parameter the border is at, and the range of the parameter across.
    fn ranges(&self, surface: &Surface) -> ((f64, f64), f64, f64) {
        let ((u0, u1), (v0, v1)) = surface.domain();
        match self.border {
            Border::UStart => ((v0, v1), u0, u1 - u0),
            Border::UEnd => ((v0, v1), u1, u0 - u1),
            Border::VStart => ((u0, u1), v0, v1 - v0),
            Border::VEnd => ((u0, u1), v1, v0 - v1),
        }
    }

    /// The parameter the border is at.
    pub fn at(&self, surface: &Surface) -> f64 {
        self.ranges(surface).1
    }

    /// The parameters of the place `along` the border and `off` it, as a
    /// share of the range across.
    fn place(&self, surface: &Surface, along: f64, off: f64) -> (f64, f64) {
        let (_, at, across) = self.ranges(surface);
        if self.across_v() {
            (along, at + across * off)
        } else {
            (at + across * off, along)
        }
    }

    /// How far `(u, v)` is from the pole's border, as a share of the
    /// parameter range across.
    pub fn share_off(&self, surface: &Surface, u: f64, v: f64) -> f64 {
        let (_, at, across) = self.ranges(surface);
        let here = if self.across_v() { v } else { u };
        ((here - at) / across).abs()
    }

    /// Whether the point of the surface at `(u, v)` is this pole: on its
    /// border, to rounding.
    pub fn holds(&self, surface: &Surface, u: f64, v: f64) -> bool {
        let (_, at, across) = self.ranges(surface);
        let here = if self.across_v() { v } else { u };
        // Not `>`: a NaN is nowhere.
        (here - at).abs() <= AT * across.abs()
    }

    /// The unit normal `∂u × ∂v` of the surface at the pole as it is
    /// reached along the meridian at `along` (the parameter that runs
    /// along the border): the limit, taken a little way off the pole.
    /// Where the surface closes up smoothly it is the same along every
    /// meridian; at a cone-like point it is the normal of the cone along
    /// that ruling. `None` where the surface has no normal nearby either.
    ///
    /// At an end of the border that is the border of another pole (a pole
    /// at a corner of the parameters) every place off this border near
    /// that end is next to that pole too, and both derivatives are small
    /// there: the place is kept as far from that end as from this border.
    pub fn normal_along(&self, surface: &Surface, along: f64) -> Option<Vec3> {
        let ((low, high), _, _) = self.ranges(surface);
        let (mut low_pole, mut high_pole) = (false, false);
        for other in poles(surface) {
            if other.across_v() == self.across_v() {
                continue;
            }
            match other.border {
                Border::UStart | Border::VStart => low_pole = true,
                Border::UEnd | Border::VEnd => high_pole = true,
            }
        }
        [NUDGE, 1e-4, 1e-2].into_iter().find_map(|off| {
            let step = off * (high - low);
            let mut along = along;
            if low_pole {
                along = along.max(low + step);
            }
            if high_pole {
                along = along.min(high - step);
            }
            let (u, v) = self.place(surface, along, off);
            let (_, su, sv) = surface.derivatives(u, v);
            let cross = su.cross(sv);
            // A cross product that short against its factors is rounding.
            (cross.length() > 1e-9 * su.length() * sv.length())
                .then(|| cross.normalized())
                .flatten()
        })
    }

    /// The unit normal of the surface for a point `p` whose nearest point
    /// of the surface is the pole: the limit normal along the meridian
    /// that leaves the pole nearest to the direction of `p`.
    ///
    /// The side of the surface `p` is on is the sign of `(p − pole) ·
    /// normal`. Why this meridian: a point a little further along whose
    /// foot is still an ordinary point of the surface near the pole has
    /// that foot on the meridian nearest to its direction, and is on the
    /// side that foot's normal says; the sign cannot change on the way to
    /// a point whose foot is the pole, because it would have to pass
    /// through zero, which takes a point straight behind that meridian
    /// (in its tangent plane and across the border), and for such a point
    /// that meridian is the farthest in direction, not the nearest. Where
    /// the surface closes up smoothly every meridian gives the same
    /// normal.
    ///
    /// Where the pole is at a corner of the parameters (two adjacent
    /// borders are this one point), the meridians leave it from both
    /// borders, and the nearest of all is taken.
    pub fn normal_towards(&self, surface: &Surface, p: Vec3) -> Option<Vec3> {
        let (along, score) = self.best_meridian(surface, p);
        let mut pick = (score, *self, along);
        for other in poles(surface) {
            // A border across the other parameter that is a pole too
            // shares a corner with this one, and so is this point.
            if other.across_v() == self.across_v() {
                continue;
            }
            let (along, score) = other.best_meridian(surface, p);
            if score > pick.0 {
                pick = (score, other, along);
            }
        }
        pick.1.normal_along(surface, pick.2)
    }

    /// The parameter along the border of the meridian that leaves the
    /// pole nearest to the direction of `p` (the middle of the border for
    /// `p` at the pole).
    pub fn meridian_towards(&self, surface: &Surface, p: Vec3) -> f64 {
        self.best_meridian(surface, p).0
    }

    /// [`Self::meridian_towards`], and how well that meridian points
    /// towards `p` (the cosine of the angle between them; minus infinity
    /// when nothing points anywhere).
    fn best_meridian(&self, surface: &Surface, p: Vec3) -> (f64, f64) {
        let ((low, high), _, _) = self.ranges(surface);
        let from = p - self.point;
        let middle = 0.5 * (low + high);
        if !(from.length() > 0.0 && from.is_finite()) {
            return (middle, f64::NEG_INFINITY);
        }
        // How well the meridian at `along` points towards `p`.
        let towards = |along: f64| {
            let (u, v) = self.place(surface, along, NUDGE);
            (surface.eval(u, v) - self.point)
                .normalized()
                .map_or(f64::NEG_INFINITY, |leaving| leaving.dot(from))
        };
        let step = (high - low) / f64::from(LOOKS);
        let mut best = (f64::NEG_INFINITY, middle);
        for k in 0..=LOOKS {
            let along = low + step * f64::from(k);
            let value = towards(along);
            if value > best.0 {
                best = (value, along);
            }
        }
        // Narrowed between the neighbours of the best place.
        let (mut a, mut b) = ((best.1 - step).max(low), (best.1 + step).min(high));
        for _ in 0..48 {
            let (m1, m2) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
            if !(m1 > a && m2 < b && m1 < m2) {
                break;
            }
            if towards(m1) >= towards(m2) {
                b = m2;
            } else {
                a = m1;
            }
        }
        let narrowed = 0.5 * (a + b);
        let value = towards(narrowed);
        let (along, value) = if value >= best.0 {
            (narrowed, value)
        } else {
            (best.1, best.0)
        };
        (along, value / from.length())
    }

    /// The parameters `(u, v)` of the place on the meridian at `along`
    /// that is `off` of the parameter range across from the pole.
    pub fn off_pole(&self, surface: &Surface, along: f64, off: f64) -> (f64, f64) {
        self.place(surface, along, off)
    }
}

/// A row of a net this share of its longest vector long is zero (a border
/// collapsed up to rounding is collapsed, as [`SAME`] takes it).
const ZERO_ROW: f64 = 1e-9;

/// How a freeform surface narrows to one of its poles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Narrowing {
    /// The order: 1 where the surface leaves the pole in a fan (its
    /// width grows as the distance from the pole), 2 or more at a cusp (a
    /// profile that leaves the axis along it: the width grows as the
    /// square of the distance, or faster).
    pub order: u32,
    /// The parameter range across of the spans at the pole.
    pub span: f64,
    /// An upper bound on the length of the iso-curve along the border at
    /// the share `β` of [`Narrowing::span`] from it: `width · β^order`.
    pub width: f64,
    /// Upper bounds, in the surface's own parameters and within the
    /// spans at the pole (`β ≤ 1`), on the derivatives along the border:
    /// `|S_σ| ≤ speed · β^order`, `|S_σσ| ≤ bend · β^order` and `|S_στ|
    /// ≤ twist · β^(order − 1)` (`τ` the parameter across).
    pub speed: f64,
    pub bend: f64,
    pub twist: f64,
}

impl Narrowing {
    /// The parameter distance from the pole's border within which every
    /// iso-curve along it is at most `length` long (at most `span`).
    pub fn within(&self, length: f64) -> f64 {
        if !(self.width > 0.0 && length > 0.0) {
            return self.span;
        }
        let share = (length / self.width).ppowf(1.0 / f64::from(self.order.max(1)));
        self.span * share.min(1.0)
    }
}

/// How a Bézier or NURBS surface narrows to `pole`
/// ([`Narrowing`]), from the Bézier spans at the pole.
///
/// The derivative along the pole's border, `S_σ`, vanishes on the
/// border; the net of the polynomial `w² S_σ`
/// ([`crate::RationalBezierPatch::weighted_derivative_nets`]) has a row
/// of zeros there, and a net whose border row is zero is, one degree
/// lower, the net of the field divided by the parameter across
/// (`B_j^n(β) / β = (n / j) B_{j−1}^{n−1}(β)`). The order is how many
/// times every span's net divides so; with it, `|S_σ| ≤ β^order · |N| /
/// w²` with `N` the divided net (its hull) and `w` the lightest weight,
/// so the iso-curve at `β` is at most `β^order` times the sum over the
/// spans of `max |N| / w_min²` long. `None` for a surface without Bézier
/// spans, a weight that is not positive, or a border whose net does not
/// divide at all.
pub fn narrowing(surface: &Surface, pole: &Pole) -> Option<Narrowing> {
    type Span = ((f64, f64), (f64, f64), RationalBezierPatch);
    let spans: Vec<Span> = match surface {
        Surface::Bezier(patch) => vec![(
            (0.0, 1.0),
            (0.0, 1.0),
            RationalBezierPatch::from_bezier(patch),
        )],
        Surface::Nurbs(s) => s
            .bezier_spans()
            .ok()?
            .into_iter()
            .map(|span| (span.u, span.v, span.patch))
            .collect(),
        _ => return None,
    };
    let ((u0, u1), (v0, v1)) = surface.domain();
    let at_start = matches!(pole.border, Border::UStart | Border::VStart);
    // The spans at the pole, each turned so that the pole is across its
    // second parameter: its net of `w² S_σ`, the degree of that across,
    // its ranges along and across, and its weights.
    struct AtPole {
        net: Vec<Vec3>,
        degree: usize,
        along: f64,
        across: f64,
        weights: Vec<f64>,
        degrees: (usize, usize),
    }
    let mut found: Vec<AtPole> = Vec::new();
    for (u, v, patch) in spans {
        let touches = match pole.border {
            Border::UStart => u.0 == u0,
            Border::UEnd => u.1 == u1,
            Border::VStart => v.0 == v0,
            Border::VEnd => v.1 == v1,
        };
        if !touches {
            continue;
        }
        let (patch, along, across) = if pole.across_v() {
            (patch, u.1 - u.0, v.1 - v.0)
        } else {
            (patch.transposed(), v.1 - v.0, u.1 - u.0)
        };
        patch.weight_range()?;
        let (net, _) = patch.weighted_derivative_nets()?;
        found.push(AtPole {
            net,
            degree: 2 * patch.degrees().1,
            along: along.abs(),
            across: across.abs(),
            weights: patch.weights(),
            degrees: patch.degrees(),
        });
    }
    // The order: how often every span's net divides.
    let mut order: Option<usize> = None;
    for span in &found {
        let (mut net, mut degree, mut times) = (span.net.clone(), span.degree, 0_usize);
        while degree > 0 {
            match divided(&net, degree, at_start) {
                Some(next) => {
                    net = next;
                    degree -= 1;
                    times += 1;
                }
                None => break,
            }
        }
        order = Some(order.map_or(times, |o| o.min(times)));
    }
    let order = order?;
    if order == 0 {
        return None;
    }
    let mut out = Narrowing {
        order: u32::try_from(order).ok()?,
        span: f64::INFINITY,
        width: 0.0,
        speed: 0.0,
        bend: 0.0,
        twist: 0.0,
    };
    for span in found {
        // `N`: the net divided `order` times, of degrees `(rows − 1,
        // degree)`.
        let (mut net, mut degree) = (span.net, span.degree);
        for _ in 0..order {
            net = divided(&net, degree, at_start)?;
            degree -= 1;
        }
        let columns = degree + 1;
        let rows = net.len() / columns;
        let at = |i: usize, j: usize| net.get(i * columns + j).copied();
        let longest = net.iter().map(|v| v.length()).fold(0.0_f64, f64::max);
        // The derivatives of `N` along and across, from the differences of
        // its net.
        let (mut n_along, mut n_across) = (0.0_f64, 0.0_f64);
        for i in 0..rows {
            for j in 0..columns {
                let here = at(i, j)?;
                if i + 1 < rows {
                    n_along = n_along.max((at(i + 1, j)? - here).length() * (rows - 1) as f64);
                }
                if j + 1 < columns {
                    n_across = n_across.max((at(i, j + 1)? - here).length() * degree as f64);
                }
            }
        }
        // The weight's derivatives, from the differences of the weights.
        let (p, q) = span.degrees;
        let w = |i: usize, j: usize| span.weights.get(i * (q + 1) + j).copied();
        let (mut w_along, mut w_across, mut lightest) = (0.0_f64, 0.0_f64, f64::INFINITY);
        for i in 0..=p {
            for j in 0..=q {
                let here = w(i, j)?;
                lightest = lightest.min(here);
                if i < p {
                    w_along = w_along.max((w(i + 1, j)? - here).abs() * p as f64);
                }
                if j < q {
                    w_across = w_across.max((w(i, j + 1)? - here).abs() * q as f64);
                }
            }
        }
        if !(lightest > 0.0 && span.along > 0.0 && span.across > 0.0) {
            return None;
        }
        let (w2, w3) = (lightest * lightest, lightest * lightest * lightest);
        // `S_σ = N β^k / w²`, so with `w` at least the lightest weight:
        // `|S_σσ| ≤ β^k (|N_σ| / w² + 2 |N| |w_σ| / w³)` and `|S_στ| ≤
        // β^(k−1) ((k |N| + |N_τ|) / w² + 2 |N| |w_τ| / w³)` for `β ≤ 1`.
        let k = order as f64;
        out.width += longest / w2;
        out.speed = out.speed.max(longest / w2 / span.along);
        out.bend = out
            .bend
            .max((n_along / w2 + 2.0 * longest * w_along / w3) / (span.along * span.along));
        out.twist = out.twist.max(
            ((k * longest + n_across) / w2 + 2.0 * longest * w_across / w3)
                / (span.along * span.across),
        );
        out.span = out.span.min(span.across);
    }
    [out.width, out.speed, out.bend, out.twist, out.span]
        .iter()
        .all(|x| x.is_finite())
        .then_some(out)
}

/// The net of a field divided by the parameter across, when the net's
/// row at the border is zero (to [`ZERO_ROW`] of its longest vector):
/// rows of `degree + 1` along the parameter across, the border at the
/// start (`low`) or at the end of each.
fn divided(net: &[Vec3], degree: usize, low: bool) -> Option<Vec<Vec3>> {
    if degree == 0 || !net.len().is_multiple_of(degree + 1) {
        return None;
    }
    let longest = net.iter().map(|v| v.length()).fold(0.0, f64::max);
    let dropped = if low { 0 } else { degree };
    let mut out = Vec::with_capacity(net.len() / (degree + 1) * degree);
    for row in net.chunks(degree + 1) {
        // Not `>`: a NaN is no zero.
        let length = row.get(dropped)?.length();
        if length.is_nan() || length > ZERO_ROW * longest {
            return None;
        }
        for (j, &vector) in row.iter().enumerate() {
            if j == dropped {
                continue;
            }
            let over = if low { j } else { degree - j };
            out.push(vector * (degree as f64 / over as f64));
        }
    }
    Some(out)
}

/// The pole of `surface` that its point at `(u, v)` is, when it is one.
pub fn pole_at(surface: &Surface, u: f64, v: f64) -> Option<Pole> {
    // Only a point on a border of the range can be: nothing is looked up
    // for any other.
    let ((u0, u1), (v0, v1)) = surface.domain();
    let on_border = (u - u0).abs() <= AT * (u1 - u0)
        || (u1 - u).abs() <= AT * (u1 - u0)
        || (v - v0).abs() <= AT * (v1 - v0)
        || (v1 - v).abs() <= AT * (v1 - v0);
    if !on_border {
        return None;
    }
    poles(surface)
        .into_iter()
        .find(|pole| pole.holds(surface, u, v))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::{BezierPatch, NurbsSurface};

    /// A cone of half-angle `alpha` about z with its apex at the origin,
    /// a quarter turn of it: u around the axis, v from the apex out.
    fn quarter_cone(alpha: f64) -> Surface {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let (r, h) = (alpha.psin(), alpha.pcos());
        let ring = [(1.0, 0.0, 1.0), (1.0, 1.0, s), (0.0, 1.0, 1.0)];
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for (x, y, w) in ring {
            for reach in [0.0, 2.0] {
                ctrl.push(Vec3::new(x * r * reach, y * r * reach, h * reach));
                weights.push(w);
            }
        }
        Surface::Nurbs(
            NurbsSurface::new(
                2,
                1,
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                3,
                2,
                ctrl,
                weights,
            )
            .unwrap(),
        )
    }

    /// The surface a profile `(ρ, y)` (a cubic Bézier) sweeps in a whole
    /// turn about the y axis: u around (a rational circle of four
    /// quarters), v along the profile; the pole is at v = 0 when the
    /// profile starts on the axis.
    fn swept(profile: [(f64, f64); 4], k: f64) -> Surface {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let ring = [
            (1.0, 0.0, 1.0),
            (1.0, 1.0, s),
            (0.0, 1.0, 1.0),
            (-1.0, 1.0, s),
            (-1.0, 0.0, 1.0),
            (-1.0, -1.0, s),
            (0.0, -1.0, 1.0),
            (1.0, -1.0, s),
            (1.0, 0.0, 1.0),
        ];
        let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
        for (x, z, w) in ring {
            for (r, y) in profile {
                ctrl.push(Vec3::new(x * r * k, y * k, z * r * k));
                weights.push(w);
            }
        }
        Surface::Nurbs(
            NurbsSurface::new(
                2,
                3,
                vec![
                    0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
                ],
                vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                9,
                4,
                ctrl,
                weights,
            )
            .unwrap(),
        )
    }

    /// The order of a cusp and a fan, and that the bounds hold: the
    /// length of the iso-curve along the border and the derivatives
    /// along it, sampled at shares from a millionth to the whole span,
    /// stay below `width β^k`, `speed β^k`, `bend β^k` and `twist
    /// β^(k−1)`, at three scales.
    #[test]
    fn a_cusp_narrows_to_the_second_order_within_its_bounds() {
        let cusp = [(0.0, 0.0), (0.0, 2.0), (3.0, 4.0), (3.0, 6.0)];
        let cone = [(0.0, 0.0), (1.0, 2.0), (2.0, 4.0), (3.0, 6.0)];
        for k in [1e-3, 1.0, 1e6] {
            for (profile, order) in [(cusp, 2), (cone, 1)] {
                let surface = swept(profile, k);
                let found = poles(&surface);
                assert_eq!(found.len(), 1);
                let n = narrowing(&surface, &found[0]).unwrap();
                assert_eq!(n.order, order);
                let m = order as i32;
                for beta in [1e-6, 1e-3, 0.01, 0.1, 0.5, 1.0] {
                    let mut length = 0.0;
                    let samples = 4000;
                    let at = |u: f64, v: f64| surface.derivatives(u, v);
                    let mut last = at(0.0, beta).0;
                    let h = 1e-5;
                    for i in 1..=samples {
                        let u = f64::from(i) / f64::from(samples);
                        let (p, su, _) = at(u, beta);
                        length += p.distance(last);
                        last = p;
                        assert!(su.length() <= n.speed * beta.ppowi(m) * (1.0 + 1e-9));
                        let a = u.clamp(h, 1.0 - h);
                        let suu = (at(a + h, beta).1 - at(a - h, beta).1) * (0.5 / h);
                        assert!(suu.length() <= n.bend * beta.ppowi(m) * 1.01, "{beta}");
                        let b = beta.max(h);
                        let suv = (at(u, b + h).1 - at(u, b - h).1) * (0.5 / h);
                        let reach = (b + h).min(1.0);
                        assert!(
                            suv.length() <= n.twist * reach.ppowi(m - 1) * 1.01,
                            "{beta}"
                        );
                    }
                    assert!(length <= n.width * beta.ppowi(m) * (1.0 + 1e-9), "{beta}");
                    // Not loose by more than the weights and the hull
                    // allow (a factor of two here).
                    if (1e-3..=0.1).contains(&beta) {
                        assert!(length >= 0.25 * n.width * beta.ppowi(m), "{beta}: {length}");
                    }
                }
                // Within the width asked for, every iso-curve is as long.
                let near = n.within(1e-6 * k);
                assert!(near > 0.0 && near <= n.span);
            }
        }
    }

    #[test]
    fn a_collapsed_border_is_a_pole_and_no_other_is() {
        let cone = quarter_cone(0.5);
        let found = poles(&cone);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].border, Border::VStart);
        assert_eq!(found[0].point, Vec3::ZERO);
        assert!(pole_at(&cone, 0.3, 0.0).is_some());
        assert!(pole_at(&cone, 0.3, 1.0).is_none());
        assert!(pole_at(&cone, 0.3, 0.5).is_none());
        // A flat patch has none; a plane neither.
        let flat = Surface::Bezier(
            BezierPatch::new(
                1,
                1,
                vec![Vec3::ZERO, Vec3::Y, Vec3::X, Vec3::new(1.0, 1.0, 0.0)],
            )
            .unwrap(),
        );
        assert!(poles(&flat).is_empty());
    }

    #[test]
    fn the_normal_at_an_apex_is_that_of_the_ruling_nearest_to_the_point() {
        for alpha in [0.2_f64, 0.5, 1.2] {
            let cone = quarter_cone(alpha);
            let pole = poles(&cone)[0];
            // Along a ruling the normal is the cone's there.
            for along in [0.0, 0.3, 1.0] {
                let n = pole.normal_along(&cone, along).unwrap();
                let (u, v) = (along, 0.5);
                let (_, su, sv) = cone.derivatives(u, v);
                assert!(n.distance(su.cross(sv).normalized().unwrap()) < 1e-6);
            }
            // Points behind the apex, nearest to it: outside the cone,
            // which is where its normal `∂u × ∂v` points (away from the
            // axis and back).
            for (x, y) in [(0.3, 0.1), (0.1, 0.4), (0.2, 0.2)] {
                let ruling = Vec3::new(x, y, 0.0).normalized().unwrap() * alpha.psin()
                    + Vec3::Z * alpha.pcos();
                // Square to a ruling and outwards, then pushed back.
                let out = (Vec3::new(x, y, 0.0).normalized().unwrap() * alpha.pcos()
                    - Vec3::Z * alpha.psin())
                    * 0.3
                    - ruling * 0.2;
                let n = pole.normal_towards(&cone, out).unwrap();
                assert!(out.dot(n) > 0.0, "{alpha} {x} {y}");
            }
        }
    }
}
