//! A spline carried on past the ends of its domain, bounded (2026-11-26).
//!
//! An edge's range may run past its curve's domain: [`NurbsCurve::eval`]
//! carries the end spans on there as the polynomials (or rational
//! functions) they are, and `craft3d_topo`'s check takes such a range. There
//! the curve leaves the convex hull of its control points, and the bounds
//! read off them (a box, a speed) do not hold it. This bounds the carried-on
//! part by its own control points: the Bézier piece of the end span
//! (`NurbsCurve::bezier_piece`) cut to where the range runs by de
//! Casteljau's algorithm, which is exact (the same function over another
//! interval), as `craft3d_topo`'s regions read such an edge (`region.rs` ›
//! `each_piece`, with `piece_points_over`).
//!
//! **Which piece.** Before the domain's start `eval` takes the first span
//! that has a width ([`NurbsCurve::span_at`] of the start); past its end
//! the span `count − 1` (`count` the number of control points), which is
//! the last piece only where it has a width: past the end of a domain
//! that ends on a span of none (knots not clamped, the end knot repeated)
//! `eval` carries on that empty span's polynomial, which no piece is, and
//! nothing is known.
//!
//! **Weights.** A rational piece lies in the hull of its control points
//! only where every weight is positive. Carried on, a piece's weights are
//! the Bernstein coefficients of its weight function over the longer
//! interval, which can be negative though the function is not (a quarter
//! circle carried ten spans on); such a stretch is halved, at most
//! [`MOST_HALVINGS`] times, until each half's are positive. A weight
//! function that falls to nothing on the range (the curve runs off to
//! infinity) never gets there: nothing is known.
//!
//! **Rounding.** Each of the `degree` rounds of a cut by a factor outside
//! `[0, 1]` multiplies an error by the sum of the sizes of its two weights
//! (`1 + 2 s` for a cut `s` span widths before the span, `2 s − 1` for one
//! reaching `s` times its width past its start), and `eval` carries the
//! span on by blends of the same sizes: both the cut and the curve as
//! evaluated grow their rounding by that product, the growth. The hull is
//! padded by 16 units in the last place of the largest coordinate involved
//! for each control point of the span, times the spread of the weights,
//! times the growth: `craft3d_topo`'s `hull_rounding` with the growth, as
//! `region.rs` pads its proofs. The derivative bounds are given only while
//! the growth stays under [`MOST_CONTINUED_GROWTH`] (`region.rs`'s
//! `MOST_GROWTH`): past it the points `eval` gives are themselves too
//! rounded for a bound on how fast the curve moves to say how far apart
//! they are.

// `!(x > y)` is deliberate: unlike `x <= y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::{CURVE_BOUND_HALVINGS, CurvePiece, H, NurbsCurve, bezier_curve_bounds, split_row};
use craft3d_math::{Portable, Vec3};

/// How often a stretch of a rational piece carried on past the domain is
/// halved at most for the weights of its cut to be positive: 64 stretches
/// an end.
const MOST_HALVINGS: u32 = 6;

/// How much the rounding of a piece carried on past its domain may grow
/// for [`NurbsCurve::continued_derivative_bounds`] to bound it: 2²⁰, as
/// `craft3d_topo`'s regions follow such an edge (`region.rs` ›
/// `MOST_GROWTH`, read off a measurement of 5,883 splines: two span widths
/// past, a piece of degree 8, a growth of 4e5, holds its curve to 7e-12 of
/// the cut's reach).
const MOST_CONTINUED_GROWTH: f64 = 1_048_576.0;

/// Which end of the domain a stretch lies past.
#[derive(Clone, Copy)]
enum Past {
    Start,
    End,
}

/// One stretch of an end piece carried on past the domain.
struct Stretch {
    /// The weighted control points of the piece cut to the stretch, in its
    /// own parameter over `[0, 1]`, every weight positive.
    net: Vec<H>,
    /// The curve's parameters the stretch runs over.
    over: (f64, f64),
    /// By how much the rounding of the cut, and of the curve as
    /// evaluated there, has grown.
    growth: f64,
    /// The largest coordinate involved: of the span's own control points,
    /// of its piece and of the cut.
    size: f64,
    /// The heaviest weight involved over the lightest.
    spread: f64,
    /// The width of the knot span the piece is of.
    span_width: f64,
}

impl Stretch {
    /// How far rounding can put the curve, as evaluated, and the cut's
    /// control points from where they would be: 16 units in the last place
    /// of the largest coordinate for each control point of the span, times
    /// the spread of the weights and the growth (the module's
    /// documentation). Not finite where it overflows.
    fn pad(&self, degree: usize) -> f64 {
        16.0 * (degree + 1) as f64 * f64::EPSILON * self.growth * self.spread * self.size
    }
}

impl NurbsCurve {
    /// Whether `range` (its two parameters in either order) is finite and
    /// runs past the domain at either end: where [`NurbsCurve::eval`]
    /// carries an end span on and the curve may leave the hull of its
    /// control points ([`NurbsCurve::continued_hull`]).
    pub fn runs_past_domain(&self, range: (f64, f64)) -> bool {
        let (r0, r1) = (range.0.min(range.1), range.0.max(range.1));
        let (d0, d1) = self.domain();
        r0.is_finite() && r1.is_finite() && (r0 < d0 || r1 > d1)
    }

    /// Points whose convex hull holds the curve as [`NurbsCurve::eval`]
    /// gives it over the parts of `range` (its two parameters in either
    /// order) that lie past the domain: the control points of each end
    /// piece cut to such a part (halved where a weight of the cut is not
    /// positive), each as the eight corners of a box about it padded for
    /// the rounding of the cut and of the curve carried on (see the
    /// module's documentation). Empty for a range within the domain, whose
    /// curve the control points hold. `None` where no such points are
    /// known: a range that is not finite, past the end of a domain that
    /// ends on a span of no width, or carried on to where a rational piece's
    /// weights do not come out positive.
    pub fn continued_hull(&self, range: (f64, f64)) -> Option<Vec<Vec3>> {
        let stretches = self.continued(range, false)?;
        let mut out = Vec::with_capacity(stretches.len() * 8 * (self.degree() + 1));
        for stretch in &stretches {
            let pad = stretch.pad(self.degree());
            if !pad.is_finite() {
                return None;
            }
            for h in &stretch.net {
                let p = h.p * (1.0 / h.w);
                for corner in 0..8_u8 {
                    let sign = |bit: u8| if corner & bit == 0 { -pad } else { pad };
                    let q = p + Vec3::new(sign(1), sign(2), sign(4));
                    if !q.is_finite() {
                        return None;
                    }
                    out.push(q);
                }
            }
        }
        Some(out)
    }

    /// Upper bounds on the lengths of the first and the second derivative
    /// of the curve, in its own parameter, over the parts of `range` that
    /// lie past the domain: those of the end pieces cut as for
    /// [`NurbsCurve::continued_hull`] but each over its whole span as well
    /// (from where the range starts to the end of the first span, from the
    /// start of the last span to where the range ends: a cut a billionth of
    /// a span long would lose the bound to the rounding of its control
    /// points' differences), by their nets' bounds, each net halved three
    /// times first, as [`NurbsCurve::derivative_bounds`] bounds a piece's
    /// (a rational piece's bound tightens as its weights even out), padded
    /// for the rounding of the curve as evaluated (the hull's pad, through
    /// the differences a derivative is made of). `(0, 0)` for a range
    /// within the domain. `None` where such a cut has
    /// no hull ([`NurbsCurve::continued_hull`]), or the rounding of a piece
    /// grows past [`MOST_CONTINUED_GROWTH`].
    pub fn continued_derivative_bounds(&self, range: (f64, f64)) -> Option<(f64, f64)> {
        let (mut first, mut second) = (0.0_f64, 0.0_f64);
        for stretch in &self.continued(range, true)? {
            if !(stretch.growth < MOST_CONTINUED_GROWTH) {
                return None;
            }
            let width = stretch.over.1 - stretch.over.0;
            if width == 0.0 {
                // One point: it moves nowhere.
                continue;
            }
            if !(width.is_finite() && width > 0.0) {
                return None;
            }
            let mut nets = vec![stretch.net.clone()];
            for _ in 0..CURVE_BOUND_HALVINGS {
                nets = nets
                    .iter()
                    .flat_map(|net| {
                        let (left, right) = split_row(net, 0.5);
                        [left, right]
                    })
                    .collect();
            }
            // Padded for rounding: the curve's points as evaluated are off
            // by up to the hull's pad, and its derivative is made from
            // differences of `degree` such points over gaps no narrower
            // than the span (its second from those differences' own), so
            // the speed `derivatives` gives may be off by twice the pad
            // times the degree over the span's width (the review of
            // 2026-11-26 measured it 2.4e-8 of the bound above the
            // unpadded bound).
            let (p, w) = (self.degree() as f64, stretch.span_width);
            let pad = stretch.pad(self.degree());
            let (pad_first, pad_second) = (2.0 * pad * p / w, 4.0 * pad * p * (p - 1.0) / (w * w));
            let part = width / nets.len() as f64;
            for net in &nets {
                let (l, m) = bezier_curve_bounds(net)?;
                first = first.max(l / part + pad_first);
                second = second.max(m / (part * part) + pad_second);
            }
        }
        (first.is_finite() && second.is_finite()).then_some((first, second))
    }

    /// The end pieces carried on past the domain over the parts of `range`
    /// that lie past it, each as its parameters and its control points with
    /// their weights (every weight positive), cut and halved as for
    /// [`NurbsCurve::continued_hull`] (without its pad), each within the
    /// range: what a search by Bézier pieces reads past the domain. Empty
    /// for a range within the domain; `None` where no such cut is known
    /// (as for `continued_hull`). Unlike `continued_hull` it gives pieces
    /// where only the hull's pad overflows (a piece carried on millions of
    /// span widths), since a search pads nothing: `craft3d_topo`'s check
    /// refuses such an edge, which has no box.
    pub fn continued_pieces(&self, range: (f64, f64)) -> Option<Vec<CurvePiece>> {
        Some(
            self.continued(range, false)?
                .into_iter()
                .map(|stretch| {
                    let net = stretch.net.iter().map(|h| (h.p * (1.0 / h.w), h.w));
                    (stretch.over, net.collect())
                })
                .collect(),
        )
    }

    /// The stretches of the end pieces carried on past the domain that
    /// `range` covers, before the start first; empty for a range within
    /// the domain. With `whole`, each with the rest of its span.
    fn continued(&self, range: (f64, f64), whole: bool) -> Option<Vec<Stretch>> {
        let (r0, r1) = (range.0.min(range.1), range.0.max(range.1));
        if !(r0.is_finite() && r1.is_finite()) {
            return None;
        }
        let (d0, d1) = self.domain();
        let mut out = Vec::new();
        if r0 < d0 {
            let span = self.span_at(d0);
            let piece = Piece::of(self, span)?;
            let to = if whole { piece.span.1 } else { r1.min(d0) };
            piece.halved(Past::Start, (r0, to), 0, &mut out)?;
        }
        if r1 > d1 {
            // `eval` carries on the span `count − 1` (`Along::new`), the
            // last piece where it has a width.
            let span = self.ctrl.len().checked_sub(1)?;
            let (lo, hi) = (*self.knots.get(span)?, *self.knots.get(span + 1)?);
            if !(hi > lo) {
                return None;
            }
            let piece = Piece::of(self, span)?;
            // From where the range starts, within that span or past the
            // domain's end (never from before the range: a search by pieces
            // would answer a parameter that is not the edge's, the review of
            // 2026-11-26), or from the span's start where the range starts
            // before it (the part within the domain is held by the control
            // points already). With `whole`, from the span's start.
            let from = if r0 > lo && r0 < r1 && !whole { r0 } else { lo };
            piece.halved(Past::End, (from, r1), 0, &mut out)?;
        }
        Some(out)
    }
}

/// The Bézier piece of one end span, with what its rounding is measured
/// by.
struct Piece {
    /// The span's parameters.
    span: (f64, f64),
    /// Its weighted control points.
    row: Vec<H>,
    /// The curve's degree: the number of rounds of a cut.
    rounds: i32,
    /// The largest coordinate of the span's own control points (those
    /// `eval` blends) and of the piece's.
    size: f64,
    /// The lightest and the heaviest of the piece's weights.
    weights: (f64, f64),
}

impl Piece {
    fn of(curve: &NurbsCurve, span: usize) -> Option<Piece> {
        let ((a, b), piece) = curve.bezier_piece(span)?;
        if !(b > a) {
            return None;
        }
        let first = span.checked_sub(curve.degree)?;
        let mut size = curve
            .ctrl
            .get(first..=span)?
            .iter()
            .fold(0.0_f64, |m, &p| m.max(largest(p)));
        let (mut lightest, mut heaviest) = (f64::INFINITY, 0.0_f64);
        for &(p, w) in &piece {
            if !(w.is_finite() && w > 0.0) {
                return None;
            }
            size = size.max(largest(p));
            lightest = lightest.min(w);
            heaviest = heaviest.max(w);
        }
        Some(Piece {
            span: (a, b),
            row: piece.iter().map(|&(p, w)| H { p: p * w, w }).collect(),
            rounds: i32::try_from(curve.degree).ok()?,
            size,
            weights: (lightest, heaviest),
        })
    }

    /// The piece cut to `over`, or its halves where a weight of the cut is
    /// not positive (at most [`MOST_HALVINGS`] deep), pushed to `out`.
    fn halved(
        &self,
        end: Past,
        over: (f64, f64),
        depth: u32,
        out: &mut Vec<Stretch>,
    ) -> Option<()> {
        let (net, growth) = self.cut(end, over)?;
        if net.iter().all(|h| h.w.is_finite() && h.w > 0.0) {
            out.push(self.stretch(net, over, growth)?);
            return Some(());
        }
        let middle = 0.5 * (over.0 + over.1);
        if depth >= MOST_HALVINGS || !(middle > over.0 && middle < over.1) {
            return None;
        }
        self.halved(end, (over.0, middle), depth + 1, out)?;
        self.halved(end, (middle, over.1), depth + 1, out)
    }

    /// The piece cut to `(from, to)` by de Casteljau's algorithm, and by
    /// how much the cut's rounding grows: before the domain (`from` before
    /// the span), the piece from `from` on, then that up to `to`;
    /// past it (`from` at least the span's start), the piece up to `to`,
    /// then that from `from`. Its weights may be anything.
    fn cut(&self, end: Past, (from, to): (f64, f64)) -> Option<(Vec<H>, f64)> {
        let (a, b) = self.span;
        let width = b - a;
        if !(from.is_finite() && to.is_finite() && to >= from) {
            return None;
        }
        match end {
            Past::Start => {
                if !(from < a && to <= b) {
                    return None;
                }
                let right = split_row(&self.row, (from - a) / width).1;
                let net = if to < b {
                    split_row(&right, (to - from) / (b - from)).0
                } else {
                    right
                };
                Some((net, (1.0 + 2.0 * (a - from) / width).ppowi(self.rounds)))
            }
            Past::End => {
                if !(from >= a && to > from) {
                    return None;
                }
                let reach = (to - a) / width;
                let left = split_row(&self.row, reach).0;
                let net = if from > a {
                    split_row(&left, (from - a) / (to - a)).1
                } else {
                    left
                };
                let growth = if reach > 1.0 {
                    (2.0 * reach - 1.0).ppowi(self.rounds)
                } else {
                    1.0
                };
                Some((net, growth))
            }
        }
    }

    /// A cut whose weights are positive, with what its rounding is
    /// measured by.
    fn stretch(&self, net: Vec<H>, over: (f64, f64), growth: f64) -> Option<Stretch> {
        let (mut lightest, mut heaviest) = self.weights;
        let mut size = self.size;
        for h in &net {
            size = size.max(largest(h.p * (1.0 / h.w)));
            lightest = lightest.min(h.w);
            heaviest = heaviest.max(h.w);
        }
        let spread = heaviest / lightest;
        (growth.is_finite() && size.is_finite() && spread.is_finite()).then_some(Stretch {
            net,
            over,
            growth,
            size,
            spread,
            span_width: self.span.1 - self.span.0,
        })
    }
}

/// The largest coordinate of `p`, in size.
fn largest(p: Vec3) -> f64 {
    p.x.abs().max(p.y.abs()).max(p.z.abs())
}
