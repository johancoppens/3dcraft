//! A fillet's radius as a law along its edge.
//!
//! **The law.** The radius is `radius` along the edge, and near an end of
//! the edge that has a [`Fade`] it runs to the fade's radius `to` at that
//! end, over the length `over` measured along the edge from the end. With
//! `τ` the distance from the end over `over`, inside the fade
//!
//! ```text
//! r = to + (radius − to) · f(τ),   f(τ) = 1 − (1 − τ)³
//! ```
//!
//! and `r = radius` from `τ = 1` on. Where the fade begins `f = 1` and
//! `f′ = f″ = 0`, so the radius has a continuous second derivative along
//! the edge (a family of sections only continuous in its first stalls the
//! refinement of a blend face); at the end `f ≈ 3τ`, linear, which is what
//! lets a later fade with `to = 0` close in a point with its borders at an
//! angle and not in a cusp.
//!
//! **Past the end of the edge** (`τ < 0`) a blend is followed until it
//! leaves the face it ends against, and where that face is not square to
//! the edge a strip of those sections is part of the fillet face. There
//! the law goes on as
//!
//! ```text
//! r = to + A · (1 − exp(a τ + b τ²)),   a = −3 (radius − to) / A,
//!                                       b = (6 (radius − to) / A − a²) / 2
//! ```
//!
//! which joins the cubic with its value, its first and its second
//! derivative, and runs on one way only (`b` is below zero, so the
//! exponent falls all the way): towards `to + A` and never past it. `A`
//! is `−to` where the radius falls towards the end (past it the radius
//! goes on falling towards nothing and never reaches it) and
//! `to − radius` where it grows (past it the radius goes on growing by no
//! more than it grew over the fade). So the radius neither runs to
//! nothing nor grows without bound however far the blend is followed, and
//! the borders past the end go on the way they came. (The cubic itself,
//! continued, does both within a few of its lengths: along a cone's cut
//! edge a radius growing from 0.15 to 0.2 over 0.3 reached 0.38 two steps
//! past the end and no ball was found. A continuation that turned back to
//! `to`, tried first, left a notch in the surface just past the end that
//! the validity check's nearest points fell into: a fade from 3 to 1 over
//! 1 was refused as an invalid body, its borders half a unit off its own
//! surface.) The continuation is this file's choice: the design of the
//! fade gives the law up to the end only.
//!
//! **A fade to nothing** (`to` zero: [`Fade::tip`]) ends the fillet in a
//! point of the edge: at the end of the edge, or `back` short of it, the
//! distance `τ` then counted from that point. The radius is nothing at
//! the point and beyond it, where there is no ball and no section: the
//! blend is followed up to the point and not past it, nothing is cut
//! there, and the continuation above has no part (it is a fade to a
//! radius, cut by a third face, that is followed past its end). At the
//! point `r ≈ 3 · radius · τ`: the borders of the fillet leave it at an
//! angle, the one whose tangent is `3 · radius / over` times how far a
//! foot lies from the edge per radius (`tan(φ/2)` between faces whose
//! normals turn by `φ`).
//!
//! **Along the edge** is by arc length: the law is a statement about the
//! edge, not about how its curve happens to be parametrised. The length of
//! the edge's curve from its start to the parameter `s` (0 at the start,
//! 1 at the end, as [`super::rolling`] counts along an edge) is the
//! integral of its speed (a line's speed times the step), by
//! Gauss-Legendre quadrature of eight points on
//! pieces of the curve between its knots (a quarter of a span each; a
//! sixteenth of the range of a curve without knots), summed once to the
//! pieces' ends; a length asked for is the sum to its piece and the
//! quadrature of the part of that piece. Past the ends the curve goes on
//! as it does for the blend's sections.
//!
//! **Which ball.** The ball in the plane across the edge through the
//! edge's point at length `a` has the radius `r(a)`: the fillet face is
//! the arcs between the feet of those balls (not the envelope of the
//! balls, which differs from it between the borders where the radius
//! changes).

use super::{BlendError, Reason, refused};
use crate::KernelError;
use craft3d_geom::Curve3;
use craft3d_math::{Portable, Vec3};
use craft3d_topo::EdgeId;

/// How a fillet's radius runs to another radius at one end of its edge,
/// or to nothing: a tip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    /// The radius where the fade ends: at the end of the edge itself, or,
    /// for a tip, nothing (zero) at `back` from it.
    pub to: f64,
    /// The length along the edge over which the radius runs from the
    /// fillet's own to `to`.
    pub over: f64,
    /// How far short of the end of the edge the fade ends. Only a fade to
    /// nothing may end short of it: the fillet face closes in a point of
    /// the edge there, and the edge beyond stays as it is.
    pub back: f64,
}

impl Fade {
    /// A fade to the radius `to` at the end of the edge, over `over`.
    pub fn to(to: f64, over: f64) -> Self {
        Self {
            to,
            over,
            back: 0.0,
        }
    }

    /// A fade to nothing over `over`: the fillet ends in a point of the
    /// edge, `back` short of the edge's end.
    pub fn tip(over: f64, back: f64) -> Self {
        Self {
            to: 0.0,
            over,
            back,
        }
    }

    /// Whether the fade runs to nothing.
    pub(crate) fn is_tip(&self) -> bool {
        self.to == 0.0
    }
}

/// A fillet's radius along its edge: see the module documentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadiusLaw {
    /// The radius away from the fades.
    pub radius: f64,
    /// The fade at the edge's start and the one at its end (the ends of
    /// the edge as the body has them, `Edge::start` and `Edge::end`).
    pub fades: [Option<Fade>; 2],
}

impl RadiusLaw {
    /// The law of one radius all along.
    pub fn constant(radius: f64) -> Self {
        Self {
            radius,
            fades: [None, None],
        }
    }

    /// The law with every number in it held to being a size: finite and
    /// above zero.
    pub(crate) fn checked(self) -> Result<Self, KernelError> {
        let size = |name: &'static str, value: f64| {
            if value.is_finite() && value > 0.0 {
                Ok(())
            } else {
                Err(KernelError::InvalidSize { name, value })
            }
        };
        size("radius", self.radius)?;
        for fade in self.fades.iter().flatten() {
            // Nothing, exactly, is a tip.
            if !fade.is_tip() {
                size("fade.to", fade.to)?;
            }
            size("fade.over", fade.over)?;
            // Short of the end only a tip ends, and by a length or by
            // nothing.
            let back = fade.back;
            if !(back.is_finite() && back >= 0.0 && (back == 0.0 || fade.is_tip())) {
                return Err(KernelError::InvalidSize {
                    name: "fade.back",
                    value: back,
                });
            }
        }
        Ok(self)
    }

    /// Whether the radius is the same all along: no fade, or none that
    /// changes it.
    pub(crate) fn is_constant(&self) -> bool {
        self.fades.iter().flatten().all(|f| f.to == self.radius)
    }

    /// The largest radius along the edge: the law runs from one radius to
    /// another without passing either, so one of those it names.
    pub(crate) fn largest(&self) -> f64 {
        self.fades
            .iter()
            .flatten()
            .fold(self.radius, |most, fade| most.max(fade.to))
    }

    /// The law with every radius `scale` times as large, over the same
    /// lengths.
    pub(crate) fn scaled(self, scale: f64) -> Self {
        Self {
            radius: self.radius * scale,
            fades: self.fades.map(|fade| {
                fade.map(|f| Fade {
                    to: f.to * scale,
                    over: f.over,
                    back: f.back,
                })
            }),
        }
    }
}

/// The nodes and weights of Gauss-Legendre quadrature of eight points on
/// (−1, 1) (Abramowitz and Stegun, table 25.4): exact for polynomials to
/// degree 15.
const GAUSS: [(f64, f64); 8] = [
    (-0.960_289_856_497_536_3, 0.101_228_536_290_376_3),
    (-0.796_666_477_413_626_7, 0.222_381_034_453_374_5),
    (-0.525_532_409_916_329, 0.313_706_645_877_887_3),
    (-0.183_434_642_495_649_8, 0.362_683_783_378_362),
    (0.183_434_642_495_649_8, 0.362_683_783_378_362),
    (0.525_532_409_916_329, 0.313_706_645_877_887_3),
    (0.796_666_477_413_626_7, 0.222_381_034_453_374_5),
    (0.960_289_856_497_536_3, 0.101_228_536_290_376_3),
];

/// Pieces a span between two knots is cut into, and the range of a curve
/// that has none.
const PER_SPAN: u32 = 4;
const WITHOUT_KNOTS: u32 = 16;
/// Most pieces of an edge, and most past one of its ends.
const MOST_PIECES: usize = 1 << 16;
const MOST_PAST: f64 = 4096.0;

/// A law along one edge: the law and the edge's curve by its length.
#[derive(Clone, Debug)]
pub(crate) struct Along<'a> {
    law: RadiusLaw,
    curve: &'a Curve3,
    range: (f64, f64),
    /// The ends of the pieces, in `s` from 0 to 1, and the length of the
    /// curve from its start to each.
    breaks: Vec<f64>,
    lengths: Vec<f64>,
    /// Per end of the edge, where a fade to nothing there ends: the `s`
    /// of the tip.
    tips: [Option<f64>; 2],
}

impl<'a> Along<'a> {
    /// `law` along the edge `edge`, whose curve is `curve` over `range`
    /// (from its start to its end). Refused: a number in the law that is
    /// not a size, a curve whose length cannot be had, and fades that are
    /// together longer than the edge.
    pub(crate) fn new(
        law: RadiusLaw,
        edge: EdgeId,
        curve: &'a Curve3,
        range: (f64, f64),
    ) -> Result<Along<'a>, BlendError> {
        let law = law.checked()?;
        let unmeasured = || refused(edge, Reason::FreeSurface);
        let (t0, t1) = range;
        if !(t0.is_finite() && t1.is_finite() && t0 != t1) {
            return Err(unmeasured());
        }
        // Where the curve's pieces join, in `s`.
        let mut joins = vec![0.0, 1.0];
        if let Curve3::Nurbs(spline) = curve {
            for knot in spline.knots() {
                let s = (knot - t0) / (t1 - t0);
                if s > 0.0 && s < 1.0 {
                    joins.push(s);
                }
            }
        }
        joins.sort_by(f64::total_cmp);
        joins.dedup();
        let per = match curve {
            Curve3::Line(_) => 1,
            Curve3::Nurbs(_) => PER_SPAN,
            Curve3::Circle(_) | Curve3::Ellipse(_) => WITHOUT_KNOTS,
        };
        let mut breaks = Vec::with_capacity(joins.len().saturating_mul(per as usize));
        for pair in joins.windows(2) {
            let [a, b] = pair else { continue };
            for k in 0..per {
                breaks.push(a + (b - a) * f64::from(k) / f64::from(per));
            }
        }
        breaks.push(1.0);
        if breaks.len() > MOST_PIECES {
            return Err(unmeasured());
        }
        let mut along = Along {
            law,
            curve,
            range,
            breaks,
            lengths: Vec::new(),
            tips: [None, None],
        };
        let mut lengths = Vec::with_capacity(along.breaks.len());
        let mut total = 0.0_f64;
        lengths.push(total);
        for pair in along.breaks.windows(2) {
            let [a, b] = pair else { continue };
            total += along.quadrature(*a, *b).ok_or_else(unmeasured)?;
            lengths.push(total);
        }
        if !(total.is_finite() && total > 0.0) {
            return Err(unmeasured());
        }
        along.lengths = lengths;
        // Two fades may meet in the middle; they may not overlap, and one
        // may not be longer than the edge.
        let faded: f64 = law.fades.iter().flatten().map(|f| f.over + f.back).sum();
        if faded > total * (1.0 + 1e-12) {
            return Err(refused(edge, Reason::LawTooLong));
        }
        // Where each tip lies along the edge.
        for (end, fade) in law.fades.iter().enumerate() {
            let Some(fade) = fade.as_ref().filter(|f| f.is_tip()) else {
                continue;
            };
            let s = if fade.back == 0.0 {
                if end == 0 { 0.0 } else { 1.0 }
            } else {
                let at = if end == 0 {
                    fade.back
                } else {
                    total - fade.back
                };
                along.place_of(at).ok_or_else(unmeasured)?
            };
            if let Some(slot) = along.tips.get_mut(end) {
                *slot = Some(s);
            }
        }
        Ok(along)
    }

    /// The length of the edge.
    pub(crate) fn length(&self) -> f64 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    /// Where the fillet ends in a point at the end `end` of the edge (0:
    /// its start, 1: its end): the `s` of the tip, where the fade there
    /// runs to nothing.
    pub(crate) fn tip(&self, end: usize) -> Option<f64> {
        self.tips.get(end).copied().flatten()
    }

    /// The part of the edge the fillet runs along, in `s`: from a tip at
    /// its start, or the start, to a tip at its end, or the end.
    pub(crate) fn span(&self) -> (f64, f64) {
        (self.tip(0).unwrap_or(0.0), self.tip(1).unwrap_or(1.0))
    }

    /// The `s` at which the curve is `length` long from the edge's start
    /// (within the edge), by bisection: the length grows with `s`.
    fn place_of(&self, length: f64) -> Option<f64> {
        if !(length >= 0.0 && length <= self.length()) {
            return None;
        }
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        for _ in 0..200 {
            let middle = 0.5 * (lo + hi);
            if !(middle > lo && middle < hi) {
                break;
            }
            if self.length_to(middle)? < length {
                lo = middle;
            } else {
                hi = middle;
            }
        }
        Some(0.5 * (lo + hi))
    }

    /// The radius at the end `end` of the edge (0: its start, 1: its end).
    pub(crate) fn at_end(&self, end: usize) -> f64 {
        match self.law.fades.get(end) {
            Some(Some(fade)) => fade.to,
            _ => self.law.radius,
        }
    }

    /// How fast the curve's point moves per unit of `s`.
    fn speed(&self, s: f64) -> Option<f64> {
        let (t0, t1) = self.range;
        let (_, d) = self.curve.derivatives(t0 + s * (t1 - t0));
        let speed = d.length() * (t1 - t0).abs();
        speed.is_finite().then_some(speed)
    }

    /// The length of the curve from `a` to `b` (in `s`, `a` below `b`),
    /// taken as one piece.
    fn quadrature(&self, a: f64, b: f64) -> Option<f64> {
        let (middle, half) = (0.5 * (a + b), 0.5 * (b - a));
        // A line moves at one speed: its length is that times the step,
        // with no rounding of the weights.
        if matches!(self.curve, Curve3::Line(_)) {
            let length = self.speed(middle)? * (b - a);
            return length.is_finite().then_some(length);
        }
        let mut sum = 0.0_f64;
        for (node, weight) in GAUSS {
            sum += weight * self.speed(middle + half * node)?;
        }
        let length = sum * half;
        length.is_finite().then_some(length)
    }

    /// The length of the curve from `a` to `b` (`a` below `b`) in pieces
    /// no longer than `piece`.
    fn in_pieces(&self, a: f64, b: f64, piece: f64) -> Option<f64> {
        let count = ((b - a) / piece).ceil();
        if !(count.is_finite() && count <= MOST_PAST) {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let count = (count.max(1.0)) as u32;
        let mut sum = 0.0_f64;
        for k in 0..count {
            let from = a + (b - a) * f64::from(k) / f64::from(count);
            let to = a + (b - a) * f64::from(k + 1) / f64::from(count);
            sum += self.quadrature(from, to)?;
        }
        Some(sum)
    }

    /// The length of the curve from the edge's start to `s`: below zero
    /// before the start, more than the edge's length past its end.
    pub(crate) fn length_to(&self, s: f64) -> Option<f64> {
        if !s.is_finite() {
            return None;
        }
        let piece =
            |i: usize| -> Option<f64> { Some(self.breaks.get(i + 1)? - self.breaks.get(i)?) };
        if s < 0.0 {
            return Some(-self.in_pieces(s, 0.0, piece(0)?)?);
        }
        if s > 1.0 {
            let last = self.breaks.len().checked_sub(2)?;
            return Some(self.length() + self.in_pieces(1.0, s, piece(last)?)?);
        }
        // The piece `s` lies in: the last break at or below it.
        let index = self
            .breaks
            .partition_point(|b| *b <= s)
            .checked_sub(1)?
            .min(self.breaks.len().checked_sub(2)?);
        let (from, so_far) = (*self.breaks.get(index)?, *self.lengths.get(index)?);
        if s == from {
            return Some(so_far);
        }
        Some(so_far + self.quadrature(from, s)?)
    }

    /// The radius at the length `at` along the edge from its start, and
    /// how fast it changes per unit of length.
    fn by_length(&self, at: f64) -> (f64, f64) {
        let RadiusLaw { radius, fades } = self.law;
        let length = self.length();
        let (mut r, mut rate) = (radius, 0.0);
        for (fade, (from_end, sign)) in fades.iter().zip([(at, 1.0), (length - at, -1.0)]) {
            let Some(fade) = fade else { continue };
            let tau = (from_end - fade.back) / fade.over;
            let change = radius - fade.to;
            if fade.is_tip() && tau <= 0.0 {
                // At the tip and beyond it there is no fillet.
                (r, rate) = (0.0, 0.0);
            } else if tau < 0.0 {
                // Past the end: levelling off (the module documentation).
                let room = if change > 0.0 { -fade.to } else { -change };
                if room == 0.0 {
                    continue;
                }
                let a = -3.0 * change / room;
                let b = 0.5 * (6.0 * change / room - a * a);
                let left = (a * tau + b * tau * tau).pexp();
                r = fade.to + room * (1.0 - left);
                rate = sign * -room * (a + 2.0 * b * tau) * left / fade.over;
            } else if tau < 1.0 {
                let m = 1.0 - tau;
                r = fade.to + change * (1.0 - m * m * m);
                rate = sign * change * 3.0 * m * m / fade.over;
            }
        }
        (r, rate)
    }

    /// The radius of the ball in the plane across the edge at `s`, and
    /// how fast it changes per unit of `s`; `None` where the law has run
    /// to nothing (past an end) or the length cannot be had.
    pub(crate) fn radius_and_rate(&self, s: f64) -> Option<(f64, f64)> {
        let (r, per_length) = self.by_length(self.length_to(s)?);
        let rate = per_length * self.speed(s)?;
        (r.is_finite() && r > 0.0 && rate.is_finite()).then_some((r, rate))
    }

    /// The radius of the ball in the plane across the edge at `s`.
    pub(crate) fn radius(&self, s: f64) -> Option<f64> {
        self.radius_and_rate(s).map(|(r, _)| r)
    }

    /// The radius of the ball that rests on `point` of a face beside the
    /// edge, `normal` the direction from the point to that ball's centre:
    /// the radius the law gives in the plane across the edge that holds
    /// the centre, found by trying a radius and reading the law where the
    /// centre it gives lies along the edge (the law changes by less than
    /// the centre moves, so this settles).
    pub(crate) fn radius_on(&self, point: Vec3, normal: Vec3) -> Option<f64> {
        let (t0, t1) = self.range;
        let place = |p: Vec3| -> Option<f64> {
            let t = self.curve.project(p).ok()?;
            let s = (t - t0) / (t1 - t0);
            s.is_finite().then_some(s)
        };
        let mut r = self.radius(place(point)?)?;
        for _ in 0..32 {
            let next = self.radius(place(point + normal * r)?)?;
            let settled = (next - r).abs() <= 1e-12 * r;
            r = next;
            if settled {
                break;
            }
        }
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::{Along, Fade, RadiusLaw};
    use craft3d_geom::{Circle, Curve3, Line, NurbsCurve};
    use craft3d_math::{Frame, Vec3};
    use craft3d_topo::EdgeId;

    fn law(radius: f64, start: Option<(f64, f64)>, end: Option<(f64, f64)>) -> RadiusLaw {
        let fade = |f: Option<(f64, f64)>| f.map(|(to, over)| Fade::to(to, over));
        RadiusLaw {
            radius,
            fades: [fade(start), fade(end)],
        }
    }

    #[test]
    fn the_length_of_a_line_and_of_an_arc_is_the_closed_form() {
        let line =
            Curve3::Line(Line::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 3.0, 4.0)).unwrap());
        let along = Along::new(RadiusLaw::constant(1.0), EdgeId(0), &line, (2.0, 12.0)).unwrap();
        assert_eq!(along.length(), 10.0);
        for (s, length) in [(0.25, 2.5), (1.0, 10.0), (-0.1, -1.0), (1.3, 13.0)] {
            let got = along.length_to(s).unwrap();
            assert!((got - length).abs() <= 1e-13, "{s}: {got}");
        }
        // A quarter circle of radius 7, followed from 0.2 to 0.2 + π/2.
        let circle = Curve3::Circle(Circle::new(Frame::WORLD, 7.0).unwrap());
        let quarter = std::f64::consts::FRAC_PI_2;
        let along = Along::new(
            RadiusLaw::constant(1.0),
            EdgeId(0),
            &circle,
            (0.2, 0.2 + quarter),
        )
        .unwrap();
        for s in [0.0, 0.1, 0.5, 0.77, 1.0, -0.05, 1.2] {
            let got = along.length_to(s).unwrap();
            assert!((got - 7.0 * quarter * s).abs() <= 1e-12, "{s}: {got}");
        }
    }

    /// The length of a spline against a fine sum of chords, improved by
    /// Richardson's step (the chords' error falls with the square of
    /// their length).
    #[test]
    fn the_length_of_a_spline_is_its_chords_in_the_limit() {
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(3.0, 2.5, 1.0),
            Vec3::new(4.0, 0.0, 1.5),
            Vec3::new(6.0, -1.0, 0.5),
            Vec3::new(7.0, 1.0, 0.0),
        ];
        let knots = vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.7, 1.0, 1.0, 1.0, 1.0];
        let spline = NurbsCurve::new(3, knots, points, vec![1.0, 1.0, 2.0, 0.5, 1.0, 1.0]).unwrap();
        let curve = Curve3::Nurbs(spline);
        let along = Along::new(RadiusLaw::constant(1.0), EdgeId(0), &curve, (0.0, 1.0)).unwrap();
        let chords = |to: f64, count: u32| -> f64 {
            (0..count)
                .map(|k| {
                    let a = to * f64::from(k) / f64::from(count);
                    let b = to * f64::from(k + 1) / f64::from(count);
                    curve.eval(a).distance(curve.eval(b))
                })
                .sum()
        };
        for s in [0.1, 0.3, 0.45, 0.7, 0.9, 1.0] {
            let (coarse, fine) = (chords(s, 20_000), chords(s, 40_000));
            let limit = fine + (fine - coarse) / 3.0;
            let got = along.length_to(s).unwrap();
            assert!((got - limit).abs() <= 1e-10, "{s}: {got} against {limit}");
        }
        // Against its curve the lengths are the same from the other end.
        let back = Along::new(RadiusLaw::constant(1.0), EdgeId(0), &curve, (1.0, 0.0)).unwrap();
        assert!((back.length() - along.length()).abs() <= 1e-13);
        let part = back.length_to(0.25).unwrap();
        let same = along.length() - along.length_to(0.75).unwrap();
        assert!((part - same).abs() <= 1e-12, "{part} against {same}");
    }

    #[test]
    fn the_radius_runs_from_one_to_the_other_with_no_jump_to_the_second_derivative() {
        let line = Curve3::Line(Line::new(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)).unwrap());
        // Radius 3 along 40, to 1 over 15 at the start, to 5 over 10 at
        // the end.
        let along = Along::new(
            law(3.0, Some((1.0, 15.0)), Some((5.0, 10.0))),
            EdgeId(0),
            &line,
            (0.0, 40.0),
        )
        .unwrap();
        let r = |x: f64| along.radius(x / 40.0).unwrap();
        assert_eq!(r(0.0), 1.0);
        assert_eq!(r(15.0), 3.0);
        assert_eq!(r(20.0), 3.0);
        assert_eq!(r(30.0), 3.0);
        assert!((r(40.0) - 5.0).abs() <= 1e-15);
        assert_eq!(along.at_end(0), 1.0);
        assert_eq!(along.at_end(1), 5.0);
        // The closed form inside the fades.
        for x in [1.0_f64, 7.5, 14.0] {
            let m = 1.0 - x / 15.0;
            assert!((r(x) - (1.0 + 2.0 * (1.0 - m * m * m))).abs() <= 1e-14);
        }
        for x in [31.0_f64, 35.0, 39.5] {
            let m = 1.0 - (40.0 - x) / 10.0;
            assert!((r(x) - (5.0 - 2.0 * (1.0 - m * m * m))).abs() <= 1e-14);
        }
        // The rate is the radius's own derivative, per unit of `s`.
        for x in [0.0, 3.0, 14.9, 15.0, 22.0, 30.0, 33.0, 40.0, -1.0, 41.0] {
            let h = 1e-5;
            let (_, rate) = along.radius_and_rate(x / 40.0).unwrap();
            let slope = (r(x + h) - r(x - h)) / (2.0 * h) * 40.0;
            assert!((rate - slope).abs() <= 1e-7, "{x}: {rate} against {slope}");
        }
        // Where a fade begins the first and the second derivative are
        // those of the constant: nothing to the second order.
        for (x, way) in [(15.0, -1.0), (30.0, 1.0)] {
            let h = 1e-3;
            let off = r(x + way * h) - 3.0;
            assert!(off.abs() <= 2.0 * h * h * h, "{x}: {off}");
        }
        // Past an end it goes on the way it came and levels off: falling
        // towards nothing and never there, growing by no more than it
        // grew over the fade, however far.
        let (mut low, mut high) = (1.0, 5.0);
        for past in [0.01, 0.1, 0.5, 1.0, 2.0, 3.0] {
            let (lower, higher) = (r(-past), r(40.0 + past));
            assert!(lower < low && lower > 0.0, "{past}: {lower}");
            assert!(higher > high && higher <= 7.0, "{past}: {higher}");
            (low, high) = (lower, higher);
        }
        // And joins the cubic with no jump to the second derivative.
        for (x, way, to) in [(0.0, -1.0, 1.0), (40.0, 1.0, 5.0)] {
            let h = 1e-3;
            let second = (r(x + way * 2.0 * h) - 2.0 * r(x + way * h) + r(x)) / (h * h);
            let inside = (r(x - way * 2.0 * h) - 2.0 * r(x - way * h) + r(x)) / (h * h);
            assert!(
                (second - inside).abs() <= 0.05 * inside.abs(),
                "{to}: {second} {inside}"
            );
        }
    }

    #[test]
    fn a_law_that_is_not_one_is_refused() {
        let line = Curve3::Line(Line::new(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)).unwrap());
        let make = |law: RadiusLaw| Along::new(law, EdgeId(3), &line, (0.0, 40.0));
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(make(RadiusLaw::constant(bad)).is_err(), "{bad}");
            // (To nothing, exactly, is a tip.)
            if bad != 0.0 {
                assert!(make(law(3.0, Some((bad, 5.0)), None)).is_err(), "{bad}");
            }
            assert!(make(law(3.0, None, Some((1.0, bad)))).is_err(), "{bad}");
        }
        // The whole edge, and two fades that meet in its middle: taken.
        assert!(make(law(3.0, Some((1.0, 40.0)), None)).is_ok());
        assert!(make(law(3.0, Some((1.0, 25.0)), Some((2.0, 15.0)))).is_ok());
        // Longer than the edge, and two that overlap: refused.
        assert!(make(law(3.0, Some((1.0, 40.001)), None)).is_err());
        assert!(make(law(3.0, Some((1.0, 25.0)), Some((2.0, 15.001)))).is_err());
        // Constant: no fade, or one that changes nothing.
        assert!(RadiusLaw::constant(2.0).is_constant());
        assert!(law(2.0, Some((2.0, 5.0)), None).is_constant());
        assert!(!law(2.0, Some((1.0, 5.0)), None).is_constant());
        assert_eq!(law(2.0, Some((1.0, 5.0)), Some((4.0, 5.0))).largest(), 4.0);
    }

    #[test]
    fn a_fade_to_nothing_ends_in_a_tip_at_the_end_or_short_of_it() {
        let line = Curve3::Line(Line::new(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)).unwrap());
        let make = |start: Option<Fade>, end: Option<Fade>| {
            Along::new(
                RadiusLaw {
                    radius: 3.0,
                    fades: [start, end],
                },
                EdgeId(0),
                &line,
                (0.0, 40.0),
            )
        };
        // At the end itself, and 5 short of the start.
        let along = make(Some(Fade::tip(10.0, 5.0)), Some(Fade::tip(15.0, 0.0))).unwrap();
        assert_eq!(along.tip(0), Some(0.125));
        assert_eq!(along.tip(1), Some(1.0));
        assert_eq!(along.span(), (0.125, 1.0));
        let r = |x: f64| along.radius(x / 40.0);
        // Nothing at a tip and beyond it; the cubic from it; the radius
        // itself from where the fade begins.
        for x in [-1.0, 0.0, 3.0, 5.0, 40.0, 41.0] {
            assert_eq!(r(x), None, "{x}");
        }
        for x in [5.5_f64, 9.0, 14.0] {
            let m = 1.0 - (x - 5.0) / 10.0;
            assert!(
                (r(x).unwrap() - 3.0 * (1.0 - m * m * m)).abs() <= 1e-14,
                "{x}"
            );
        }
        for x in [26.0_f64, 33.0, 39.9] {
            let m = 1.0 - (40.0 - x) / 15.0;
            assert!(
                (r(x).unwrap() - 3.0 * (1.0 - m * m * m)).abs() <= 1e-14,
                "{x}"
            );
        }
        assert_eq!(r(15.0), Some(3.0));
        assert_eq!(r(25.0), Some(3.0));
        // Close to a tip the radius is three times the distance over the
        // fade's length, times the radius.
        let near = r(40.0 - 1e-6).unwrap();
        assert!((near - 3.0 * 3.0 * 1e-6 / 15.0).abs() <= 1e-11, "{near}");
        // No tip without a fade to nothing: the edge's whole length.
        let plain = make(Some(Fade::to(1.0, 10.0)), None).unwrap();
        assert_eq!((plain.tip(0), plain.tip(1)), (None, None));
        assert_eq!(plain.span(), (0.0, 1.0));
        // Short of the end only a tip ends; by a length, not by less than
        // nothing; and the fade with it has to fit on the edge.
        let short = Fade {
            to: 1.0,
            over: 10.0,
            back: 2.0,
        };
        assert!(make(Some(short), None).is_err());
        for bad in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(make(Some(Fade::tip(10.0, bad)), None).is_err(), "{bad}");
        }
        assert!(make(Some(Fade::tip(10.0, 30.0)), None).is_ok());
        assert!(make(Some(Fade::tip(10.0, 30.001)), None).is_err());
        assert!(make(Some(Fade::tip(10.0, 12.0)), Some(Fade::tip(15.0, 3.0))).is_ok());
        assert!(make(Some(Fade::tip(10.0, 12.0)), Some(Fade::tip(15.0, 3.001))).is_err());
        // Along a spline the tip lies where the curve is that long.
        let spline = Curve3::Nurbs(
            NurbsCurve::new(
                3,
                vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                vec![
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 2.0, 0.0),
                    Vec3::new(3.0, 2.5, 1.0),
                    Vec3::new(4.0, 0.0, 1.5),
                ],
                vec![1.0; 4],
            )
            .unwrap(),
        );
        let law = RadiusLaw {
            radius: 0.3,
            fades: [None, Some(Fade::tip(1.0, 0.75))],
        };
        let along = Along::new(law, EdgeId(0), &spline, (0.0, 1.0)).unwrap();
        let at = along.tip(1).unwrap();
        let length = along.length_to(at).unwrap();
        assert!(
            (length - (along.length() - 0.75)).abs() <= 1e-12,
            "{length}"
        );
    }
}
