//! The offset of a freeform surface: the points `S(u, v) + d N(u, v)` at
//! the distance `d` along the unit normal `N = (∂u × ∂v) / |∂u × ∂v|`.
//!
//! The offset of a spline surface is not a spline (the unit normal has a
//! square root in it), so what is built is a B-spline surface within a
//! limit of it, point for point in the same parameters: the offset is
//! sampled on a grid, a quintic is laid through the samples
//! ([`crate::refine::interpolate`] along v and then along u) piece by
//! piece between the knots at which the offset is less smooth (the
//! breaks), and the surface is held against the exact offset between the
//! samples, which are added where it does not hold, until it does
//! everywhere it is looked at (the middle of every step while refining,
//! and the quarters at the end). The same parameters mean the same domain
//! and the same seams: a surface that closes on itself has an offset that
//! does too.
//!
//! **Where the offset is not regular** it folds over: where `d` reaches a
//! radius of curvature on the side the surface is offset to (inward on a
//! ball by its radius or more), the offset turns back on itself and is no
//! surface a solid can have. That is decided before anything is fitted,
//! not by sampling alone: per Bézier span of the surface, the matrix
//! `I − d II` (the first fundamental form less `d` times the second) has
//! to be positive definite everywhere, which is the same as `1 − d κ > 0`
//! for both principal curvatures `κ`. In homogeneous terms (`S = P / w`)
//! it is, up to a positive factor, `|n| G − d W`, where `n`, `G` and `W`
//! are polynomials (Bernstein forms of the span, built exactly by products
//! and derivatives of its control net) and `|n|` is the length of the
//! normal `n = w⁴ (∂u × ∂v)`. Per piece of a span, cheapest first:
//! Gershgorin's discs on the ranges of the entries (scaled by the
//! diagonal of `G`); the matrix diagonally dominant point by point with
//! `|n|` replaced by forms linear in `n · n` below and above it (the
//! chord and a tangent of the square root); then, with `|n|` at both
//! bounds the Bernstein form of `n · n` gives it (the matrix is affine in
//! `|n|`, so positive definiteness at both proves it in between), the
//! least Bernstein coefficient of the leading entry and of the
//! determinant (a polynomial of twice the degree, built only where
//! needed). A piece that is not proven is halved both ways; a piece at whose middle
//! the matrix is not positive definite is a fold, proven, with the radius
//! of curvature there. A piece that is neither when the pieces run out is
//! reported as not proven: the distance comes within a hair of a radius
//! of curvature. A surface whose normal vanishes somewhere (a pole, a
//! border drawn into a point) has no offset there and is refused; one of
//! degrees too high for the forms (about sixteen times its degrees) is
//! refused as such.
//!
//! **What is proven and what is measured.** The regularity is proven for
//! the surface's own normal ([`offset_along`] may turn it a little; the
//! turn is the caller's to bound). The fit is held against the exact
//! offset at the middles and then the quarters of its steps: measured
//! there, not bounded between them (in fuzzing, one fit in a hundred
//! exceeded its limit between samples by 0.3 %; the callers' limit is a
//! quarter of the tolerance, so that leaves the body within it).

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::refine::{interpolate, interpolate_on};
use crate::{GeomError, NurbsSurface, Surface, pole};
use craft3d_math::Portable;
use craft3d_math::Vec3;

/// Why there is no offset.
#[derive(Clone, Debug, PartialEq)]
pub enum OffsetError {
    /// The normal of the surface is not defined everywhere (a pole, a
    /// border or a corner drawn into a point, a crease where it vanishes):
    /// near `at` the offset is not defined.
    NoNormal { at: (f64, f64) },
    /// The offset folds over: at `at` the radius of curvature on the side
    /// the surface is offset to is `radius`, which the distance reaches.
    Folds { at: (f64, f64), radius: f64 },
    /// That the offset does not fold could not be proven near `at` in the
    /// pieces allowed: the distance comes within a hair of the smallest
    /// radius of curvature on that side.
    Unproven { at: (f64, f64) },
    /// The approximation did not come within the limit in the samples
    /// allowed.
    NoFit,
    /// The work bound of the operation this offset is part of was reached
    /// ([`crate::allowance`]) before the offset was proven regular and
    /// fitted.
    Spent,
    /// Input that is no surface or no distance.
    Invalid(GeomError),
    /// The fit did not come within the limit in the evaluations of the
    /// exact offset one fit may make ([`MAX_EVALUATIONS`]).
    OverBudget,
}

impl From<GeomError> for OffsetError {
    fn from(error: GeomError) -> Self {
        OffsetError::Invalid(error)
    }
}

/// The exact offset point at `(u, v)`; `None` where the normal is not
/// defined.
pub fn exact(surface: &Surface, distance: f64, u: f64, v: f64) -> Option<Vec3> {
    let (point, du, dv) = surface.derivatives(u, v);
    let normal = du.cross(dv).normalized()?;
    let at = point + normal * distance;
    at.is_finite().then_some(at)
}

/// The offset of the Bézier or NURBS `surface` at `distance` along its
/// normal `∂u × ∂v`, as a B-spline surface over the same parameters that
/// is within `limit` of the exact offset at every point it was held
/// against (see the module documentation). Refused when the offset is not
/// regular ([`OffsetError::Folds`], [`OffsetError::Unproven`]), where the
/// normal is not defined ([`OffsetError::NoNormal`]), or when the
/// approximation does not converge ([`OffsetError::NoFit`]).
pub fn offset(surface: &Surface, distance: f64, limit: f64) -> Result<NurbsSurface, OffsetError> {
    let nurbs = freeform(surface)?;
    offset_over(surface, distance, limit, nurbs.domain())
}

/// A box of parameters: `((u start, u end), (v start, v end))`.
pub type Domain = ((f64, f64), (f64, f64));

/// [`offset`] over the parameters `over`, which may reach past the
/// surface's domain: there the surface is continued as the functions its
/// end spans are (as evaluating it past its domain does), so the offset
/// of a face that grows past the borders of its surface is there. The
/// regularity is proven over all of `over`. A box that does not take in
/// the whole domain is widened to it.
pub fn offset_over(
    surface: &Surface,
    distance: f64,
    limit: f64,
    over: Domain,
) -> Result<NurbsSurface, OffsetError> {
    offset_along(surface, distance, limit, over, &|_, _| Vec3::ZERO)
}

/// [`offset_over`] along the normal `N + turn(u, v)`, made unit again:
/// the normal turned a little where the caller knows better than the
/// surface which way it points (along an edge where a face is tangent to
/// its neighbour only to within what fitting left, the normal both
/// offsets share). The turn is meant to be small: the regularity is
/// proven for the surface's own normal. It has to be twice continuously
/// differentiable, past the domain as well when `over` reaches past it:
/// the fit is made in one piece across the domain's ends.
pub fn offset_along(
    surface: &Surface,
    distance: f64,
    limit: f64,
    over: Domain,
    turn: &dyn Fn(f64, f64) -> Vec3,
) -> Result<NurbsSurface, OffsetError> {
    offset_held(surface, distance, limit, over, turn, None)
}

/// Where the fit of an offset is held to its limit: the points of the
/// surface that lie within `reach`, a length measured along the surface
/// (along the straight path in parameters from the box's nearest point),
/// of the box of parameters `around` (the points of the box itself
/// included).
///
/// A box of parameters is what a tensor-product surface can be continued
/// by, but what a caller needs past a border is a length: a developable
/// whose rulings shrink towards a tip is continued across them by a
/// parameter that is large where they are short, so the box reaches far
/// beyond the length needed wherever they are long (the sea kayak's
/// panel, 2026-10-27: ten rulings' lengths past its chines in the middle
/// of the panel, for 14 mm at its tip). The offset is then fitted over
/// the whole box, through samples of the exact offset everywhere, but
/// held to the limit, and refined, only where it is used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Held {
    pub around: Domain,
    pub reach: f64,
}

impl Held {
    /// How far the point of `surface` at `(u, v)` is from the box, by the
    /// measure the fit is held by: the length along the surface of the
    /// straight path in parameters from the box's nearest point (each
    /// parameter clamped to it), nothing inside the box, infinite where
    /// it is not a number. A caller checks with it that what it made lies
    /// where the fit was held.
    pub fn length(&self, surface: &NurbsSurface, u: f64, v: f64) -> f64 {
        let ((a0, a1), (b0, b1)) = self.around;
        if !(u.is_finite() && v.is_finite()) {
            return f64::INFINITY;
        }
        let (cu, cv) = (u.clamp(a0, a1), v.clamp(b0, b1));
        let (du, dv) = (u - cu, v - cv);
        if du == 0.0 && dv == 0.0 {
            return 0.0;
        }
        const STEPS: usize = 4;
        let mut length = 0.0;
        for k in 0..STEPS {
            for (x, w) in GAUSS3 {
                let t = (k as f64 + x) / STEPS as f64;
                let (_, su, sv) = surface.derivatives(cu + du * t, cv + dv * t);
                length += w * (su * du + sv * dv).length() / STEPS as f64;
            }
            if length > self.reach {
                break;
            }
        }
        if length.is_nan() {
            f64::INFINITY
        } else {
            length
        }
    }
}

/// [`offset_along`] held to `limit` only where `held` says (all of `over`
/// for `None`). Past the held region the result interpolates the exact
/// offset at its samples and is not measured: a caller uses it only within
/// the held region. The regularity is proven over all of `over`.
pub fn offset_held(
    surface: &Surface,
    distance: f64,
    limit: f64,
    over: Domain,
    turn: &dyn Fn(f64, f64) -> Vec3,
    held: Option<&Held>,
) -> Result<NurbsSurface, OffsetError> {
    let nurbs = freeform(surface)?;
    if let Some(h) = held {
        let ((a, b), (c, d)) = h.around;
        if ![a, b, c, d, h.reach].iter().all(|x| x.is_finite()) || !(h.reach >= 0.0) {
            return Err(OffsetError::Invalid(GeomError::InvalidParameter(
                "the region an offset is held in needs finite parameters and a reach of no less than nothing",
            )));
        }
    }
    if !(distance.is_finite() && limit.is_finite() && limit > 0.0) {
        return Err(OffsetError::Invalid(GeomError::InvalidParameter(
            "an offset needs a finite distance and a positive limit",
        )));
    }
    let over = wide(&nurbs, over)?;
    match regular_over(surface, distance, over) {
        Ok(()) => {}
        // Folded, or not proven not to, only where the surface goes on past
        // its domain: there it goes on as its end spans' whole polynomials,
        // which a span of high degree changed close to its border (a sheet
        // held G2 to its neighbour) turns sharply. Continued instead by its
        // second-order Taylor expansion across each border (the same surface
        // inside, as curved past the border as at it), whose domain is
        // `over` (2026-11-01: a mouse's top, shelled 1.8 mm, folded 0.04 of
        // its parameter past a corner with a radius of 0.3 mm).
        Err(OffsetError::Folds { at, .. } | OffsetError::Unproven { at })
            if !inside(nurbs.domain(), at) =>
        {
            let tol = 0.25 * limit;
            let taylor = nurbs
                .extended_taylor(over.0, over.1, tol)
                .map_err(|_| OffsetError::Unproven { at })?;
            if taylor.domain() == nurbs.domain() {
                return Err(OffsetError::Unproven { at });
            }
            return offset_held(&Surface::Nurbs(taylor), distance, limit, over, turn, held);
        }
        Err(error) => return Err(error),
    }
    // Fitted a step past every end that is not a seam, and cut back to
    // `over`: the borders of the face then lie inside the fit, where its
    // derivatives (the normal, which a face tangent to it across a border
    // has to share) are as good as anywhere, not at the ends of an
    // interpolation, where they are worst. Without the step where the
    // surface continued that far is not proven regular, or its offset there
    // cannot be fitted (a rational span continued past its domain may run
    // towards a weight of nothing): the fit then ends at `over`, as it did
    // before the step was taken.
    let past = margins(&nurbs, over, limit);
    if past != over
        && regular_over(surface, distance, past).is_ok()
        && let Ok(fitted) = fit(
            surface,
            &nurbs,
            distance,
            limit,
            past,
            turn,
            held,
            MAX_EVALUATIONS,
        )
        && let Ok(cut) = fitted.restricted(over.0, over.1)
    {
        return Ok(cut);
    }
    fit(
        surface,
        &nurbs,
        distance,
        limit,
        over,
        turn,
        held,
        MAX_EVALUATIONS,
    )
}

/// Whether `at` lies inside `domain` (its border included).
fn inside(domain: Domain, at: (f64, f64)) -> bool {
    let ((u0, u1), (v0, v1)) = domain;
    at.0 >= u0 && at.0 <= u1 && at.1 >= v0 && at.1 <= v1
}

/// `over` with a step of the fit's first samples added past each end in a
/// direction in which the surface does not close on itself (its ends meet
/// within `tol`): the first or the last knot span of the surface's own
/// domain over [`START`], also where `over` reaches past the domain (the
/// steps past the domain are as long as those of that span, [`start`], so
/// the margin is one such step whatever `over` is).
fn margins(nurbs: &NurbsSurface, over: Domain, tol: f64) -> Domain {
    let ((u0, u1), (v0, v1)) = nurbs.domain();
    let step = |knots: &[f64], (lo, hi): (f64, f64)| -> (f64, f64) {
        let inner: Vec<f64> = knots
            .iter()
            .copied()
            .filter(|k| *k > lo && *k < hi)
            .collect();
        let first = inner.first().copied().unwrap_or(hi) - lo;
        let last = hi - inner.last().copied().unwrap_or(lo);
        (first / START as f64, last / START as f64)
    };
    let widen = |(a, b): (f64, f64), (before, after): (f64, f64), closes: bool| {
        if closes || !(before > 0.0 && after > 0.0 && before.is_finite() && after.is_finite()) {
            (a, b)
        } else {
            (a - before, b + after)
        }
    };
    (
        widen(
            over.0,
            step(nurbs.knots_u(), (u0, u1)),
            nurbs.closes(true, tol),
        ),
        widen(
            over.1,
            step(nurbs.knots_v(), (v0, v1)),
            nurbs.closes(false, tol),
        ),
    )
}

/// `over` widened to take in the domain of `nurbs`, or an error when it
/// is not a box of numbers.
fn wide(nurbs: &NurbsSurface, over: Domain) -> Result<Domain, OffsetError> {
    let ((u0, u1), (v0, v1)) = nurbs.domain();
    let ((a, b), (c, d)) = over;
    if ![a, b, c, d].iter().all(|x| x.is_finite()) {
        return Err(OffsetError::Invalid(GeomError::InvalidParameter(
            "the parameters an offset is made over must be finite",
        )));
    }
    Ok(((a.min(u0), b.max(u1)), (c.min(v0), d.max(v1))))
}

fn freeform(surface: &Surface) -> Result<NurbsSurface, OffsetError> {
    match surface {
        Surface::Nurbs(s) => Ok(s.clone()),
        Surface::Bezier(patch) => Ok(NurbsSurface::from_bezier(patch)?),
        _ => Err(OffsetError::Invalid(GeomError::InvalidParameter(
            "only a Bézier or NURBS surface is offset here; the analytic kinds have closed forms",
        ))),
    }
}

// ---------------------------------------------------------------------
// Regularity.

/// Most pieces the regularity of one surface is looked at in.
const MAX_PIECES: usize = 1 << 12;
/// How many times a span may be halved (each way) on the way to a piece.
const MAX_DEPTH: u32 = 14;

/// Proves that the offset of `surface` at `distance` does not fold over
/// and that its normal is defined everywhere, or says where it does not
/// (see the module documentation).
pub fn regular(surface: &Surface, distance: f64) -> Result<(), OffsetError> {
    let nurbs = freeform(surface)?;
    regular_over(surface, distance, nurbs.domain())
}

/// [`regular`] over the parameters `over` (see [`offset_over`]).
pub fn regular_over(surface: &Surface, distance: f64, over: Domain) -> Result<(), OffsetError> {
    let nurbs = freeform(surface)?;
    let over = wide(&nurbs, over)?;
    let ((u0, u1), (v0, v1)) = nurbs.domain();
    if let Some(found) = pole::poles(surface).first() {
        let at = match found.border {
            pole::Border::UStart => (u0, 0.5 * (v0 + v1)),
            pole::Border::UEnd => (u1, 0.5 * (v0 + v1)),
            pole::Border::VStart => (0.5 * (u0 + u1), v0),
            pole::Border::VEnd => (0.5 * (u0 + u1), v1),
        };
        return Err(OffsetError::NoNormal { at });
    }
    let spans = nurbs.bezier_spans()?;
    let mut budget = MAX_PIECES;
    for span in &spans {
        if crate::allowance::pass() {
            return Err(OffsetError::Spent);
        }
        let points = span.patch.control_points();
        let weights = span.patch.weights();
        let (m, n) = span.patch.degrees();
        // The determinant's forms have about sixteen times the degrees of
        // the span in each direction (homogeneous products); past the
        // coefficients a form may have, nothing can be proven.
        if (16 * m + 1).saturating_mul(16 * n + 1) > MAX_COEFFICIENTS {
            return Err(OffsetError::Invalid(GeomError::Unsupported(
                "the degrees of the freeform face are too high for its offset to be proven free of folds",
            )));
        }
        // The span's own parameters of the box it stands for: past its
        // border where it is a border span and the box reaches further.
        let local = |(lo, hi): (f64, f64), (start, end): (f64, f64), (from, to): (f64, f64)| {
            let width = hi - lo;
            (
                if lo <= start {
                    (from - lo) / width
                } else {
                    0.0
                },
                if hi >= end { (to - lo) / width } else { 1.0 },
            )
        };
        let box_u = local(span.u, (u0, u1), over.0);
        let box_v = local(span.v, (v0, v1), over.1);
        if (box_u, box_v) == ((0.0, 1.0), (0.0, 1.0)) && plainly_regular(&span.patch, distance) {
            continue;
        }
        let Some(nets) = Nets::of(m, n, &points, &weights, box_u, box_v) else {
            return Err(OffsetError::Invalid(GeomError::InvalidParameter(
                "a span of the surface has no usable control net",
            )));
        };
        let mut stack = vec![(nets, (0.0, 1.0), (0.0, 1.0), 0_u32)];
        while let Some((mut piece, s, t, depth)) = stack.pop() {
            if crate::allowance::pass() {
                return Err(OffsetError::Spent);
            }
            let global = |a: f64, b: f64| {
                (
                    span.u.0 + (span.u.1 - span.u.0) * (box_u.0 + (box_u.1 - box_u.0) * a),
                    span.v.0 + (span.v.1 - span.v.0) * (box_v.0 + (box_v.1 - box_v.0) * b),
                )
            };
            let middle = global(0.5 * (s.0 + s.1), 0.5 * (t.0 + t.1));
            if piece.proves(distance) {
                continue;
            }
            // A fold at the middle is proven.
            match piece.at_middle(distance) {
                Middle::Fold(radius) => {
                    return Err(OffsetError::Folds { at: middle, radius });
                }
                Middle::NoNormal => return Err(OffsetError::NoNormal { at: middle }),
                Middle::Fine => {}
            }
            budget = budget.saturating_sub(1);
            if depth >= MAX_DEPTH || budget < 4 {
                return Err(if piece.normal_may_vanish() {
                    OffsetError::NoNormal { at: middle }
                } else {
                    OffsetError::Unproven { at: middle }
                });
            }
            let (sm, tm) = (0.5 * (s.0 + s.1), 0.5 * (t.0 + t.1));
            let (left, right) = piece.split_u();
            let (ll, lr) = left.split_v();
            let (rl, rr) = right.split_v();
            stack.push((ll, (s.0, sm), (t.0, tm), depth + 1));
            stack.push((lr, (s.0, sm), (tm, t.1), depth + 1));
            stack.push((rl, (sm, s.1), (t.0, tm), depth + 1));
            stack.push((rr, (sm, s.1), (tm, t.1), depth + 1));
        }
    }
    Ok(())
}

/// How many times [`plainly_regular`] halves a patch (each way) before it
/// leaves the patch to the Bernstein forms.
const PLAIN_DEPTH: u32 = 3;

/// A sufficient test, cheap next to the Bernstein forms: a bound on every
/// normal curvature of the patch, from bounds proven from its control net
/// alone ([`crate::RationalBezierPatch::derivative_bounds`] for the first
/// and second derivatives, the hull of the net of `w³ (S_u × S_v)`
/// ([`crate::RationalBezierPatch::weighted_normal_net`]) for the least
/// length `n` of the normal). In parameters scaled so that the first
/// derivatives are at most 1, the second fundamental form is at most the
/// largest eigenvalue of `[[B_uu, B_uv], [B_uv, B_vv]]` (scaled alike)
/// times `a² + b²`, and the first at least `n² / (1 + √(1 − n²))` times it
/// (its determinant is `n²`, its largest eigenvalue at most `max(E, G) +
/// |F|` with `F² = EG − n²`). The distance times their ratio below one
/// proves the offset regular on the patch; else the patch is halved each
/// way, [`PLAIN_DEPTH`] times at most, every piece to pass.
fn plainly_regular(patch: &crate::RationalBezierPatch, distance: f64) -> bool {
    plainly(patch, distance, 0)
}

fn plainly(patch: &crate::RationalBezierPatch, distance: f64, depth: u32) -> bool {
    if curvature_bound(patch).is_some_and(|k| distance.abs() * k < 1.0) {
        return true;
    }
    if depth >= PLAIN_DEPTH {
        return false;
    }
    let (left, right) = patch.split_u(0.5);
    [left, right].iter().all(|half| {
        let (a, b) = half.split_v(0.5);
        plainly(&a, distance, depth + 1) && plainly(&b, distance, depth + 1)
    })
}

/// The bound on the normal curvatures of [`plainly_regular`].
fn curvature_bound(patch: &crate::RationalBezierPatch) -> Option<f64> {
    let bounds = patch.derivative_bounds()?;
    let net = patch.weighted_normal_net()?;
    let (_, heaviest) = patch.weight_range()?;
    let towards = net.iter().fold(Vec3::ZERO, |a, b| a + *b).normalized()?;
    let least = net
        .iter()
        .map(|d| d.dot(towards))
        .fold(f64::INFINITY, f64::min)
        / (heaviest * heaviest * heaviest);
    let (fu, fv) = (bounds.first_u, bounds.first_v);
    if !(least > 0.0 && least.is_finite() && fu > 0.0 && fv > 0.0) {
        return None;
    }
    let m = (least / (fu * fv)).min(1.0);
    let (a, b, c) = (
        bounds.second_uu / (fu * fu),
        bounds.second_vv / (fv * fv),
        bounds.second_uv / (fu * fv),
    );
    let second = 0.5 * (a + b) + (0.25 * (a - b) * (a - b) + c * c).sqrt();
    let first = m * m / (1.0 + (1.0 - m * m).max(0.0).sqrt());
    let bound = second / first;
    bound.is_finite().then_some(bound)
}

/// What holds at the middle of a piece.
enum Middle {
    Fine,
    /// The offset folds there; the radius of curvature on that side.
    Fold(f64),
    NoNormal,
}

/// The Bernstein forms over one piece of a span that decide whether the
/// offset is regular there: the weight `w` (which has to stay positive;
/// past the domain it may not), `n · n` and the entries of `G` and `W`.
/// (`det G = |n|²` by Lagrange's identity.)
#[derive(Clone, Debug)]
struct Nets {
    w: Bern,
    nn: Bern,
    g11: Bern,
    g12: Bern,
    g22: Bern,
    w11: Bern,
    w12: Bern,
    w22: Bern,
    /// `n · n` times the entries of `G`.
    ng11: Bern,
    ng12: Bern,
    ng22: Bern,
    /// The leading entries and the parts of the determinant at one degree
    /// each (`G₁₁`, `W₁₁`, `det G`, `D₁`, `det W`), once a piece needed
    /// them; its pieces split them along.
    exact: Option<Vec<Bern>>,
}

impl Nets {
    /// From a rational Bézier patch of degrees `(m, n)`, its control
    /// points and weights row-major, over the box `box_u × box_v` of its
    /// own parameters (`[0, 1]²` for the patch itself).
    fn of(
        m: usize,
        n: usize,
        points: &[Vec3],
        weights: &[f64],
        box_u: (f64, f64),
        box_v: (f64, f64),
    ) -> Option<Nets> {
        let origin = *points.first()?;
        let count = (m + 1).checked_mul(n + 1)?;
        if points.len() != count || weights.len() != count {
            return None;
        }
        let mut px = Vec::with_capacity(count);
        let mut py = Vec::with_capacity(count);
        let mut pz = Vec::with_capacity(count);
        for (p, w) in points.iter().zip(weights) {
            let local = (*p - origin) * *w;
            px.push(local.x);
            py.push(local.y);
            pz.push(local.z);
        }
        let polynomial = weights.iter().all(|w| *w == 1.0);
        let over = |b: Bern| b.over(box_u, box_v);
        let w = if polynomial {
            Bern::constant(1.0)
        } else {
            over(Bern::new(m, n, weights.to_vec())?)
        };
        let p = [
            over(Bern::new(m, n, px)?),
            over(Bern::new(m, n, py)?),
            over(Bern::new(m, n, pz)?),
        ];
        let (wu, wv) = (w.du(), w.dv());
        let (wuu, wuv, wvv) = (wu.du(), wu.dv(), wv.dv());
        let pu = map(&p, Bern::du);
        let pv = map(&p, Bern::dv);
        let puu = map(&pu, Bern::du);
        let puv = map(&pu, Bern::dv);
        let pvv = map(&pv, Bern::dv);
        // X_u = P_u w − P w_u (= w² S_u), and along v.
        let xu = sub3(&scale3(&pu, &w), &scale3(&p, &wu));
        let xv = sub3(&scale3(&pv, &w), &scale3(&p, &wv));
        let normal = cross(&xu, &xv);
        let ww = w.mul(&w);
        // w³ S_uu = P_uu w² − (2 P_u w_u + P w_uu) w + 2 P w_u².
        let yuu = {
            let a = scale3(&puu, &ww);
            let b = scale3(&add3(&scale3(&pu, &wu.times(2.0)), &scale3(&p, &wuu)), &w);
            let c = scale3(&p, &wu.mul(&wu).times(2.0));
            add3(&sub3(&a, &b), &c)
        };
        // w³ S_uv = P_uv w² − (P_u w_v + P_v w_u + P w_uv) w + 2 P w_u w_v.
        let yuv = {
            let a = scale3(&puv, &ww);
            let inner = add3(
                &add3(&scale3(&pu, &wv), &scale3(&pv, &wu)),
                &scale3(&p, &wuv),
            );
            let b = scale3(&inner, &w);
            let c = scale3(&p, &wu.mul(&wv).times(2.0));
            add3(&sub3(&a, &b), &c)
        };
        let yvv = {
            let a = scale3(&pvv, &ww);
            let b = scale3(&add3(&scale3(&pv, &wv.times(2.0)), &scale3(&p, &wvv)), &w);
            let c = scale3(&p, &wv.mul(&wv).times(2.0));
            add3(&sub3(&a, &b), &c)
        };
        let (g11, g12, g22) = (dot(&xu, &xu), dot(&xu, &xv), dot(&xv, &xv));
        let nn = dot(&normal, &normal);
        // Each entry's three forms at one degree, so that the tests
        // combine them coefficient by coefficient.
        let entry = |g: Bern, w: Bern| {
            let ng = nn.mul(&g);
            let (m, n) = (g.m.max(w.m).max(ng.m), g.n.max(w.n).max(ng.n));
            (g.elevated(m, n), ng.elevated(m, n), w.elevated(m, n))
        };
        let (g11, ng11, w11) = entry(g11, w.mul(&dot(&yuu, &normal)));
        let (g12, ng12, w12) = entry(g12, w.mul(&dot(&yuv, &normal)));
        let (g22, ng22, w22) = entry(g22, w.mul(&dot(&yvv, &normal)));
        Some(Nets {
            ng11,
            ng12,
            ng22,
            g11,
            g12,
            g22,
            w11,
            w12,
            w22,
            nn,
            w,
            exact: None,
        })
    }

    fn all(&self) -> [&Bern; 11] {
        [
            &self.w, &self.nn, &self.g11, &self.g12, &self.g22, &self.w11, &self.w12, &self.w22,
            &self.ng11, &self.ng12, &self.ng22,
        ]
    }

    fn rebuilt(parts: Vec<Bern>, exact: Option<Vec<Bern>>) -> Option<Nets> {
        let mut parts = parts.into_iter();
        Some(Nets {
            w: parts.next()?,
            nn: parts.next()?,
            g11: parts.next()?,
            g12: parts.next()?,
            g22: parts.next()?,
            w11: parts.next()?,
            w12: parts.next()?,
            w22: parts.next()?,
            ng11: parts.next()?,
            ng12: parts.next()?,
            ng22: parts.next()?,
            exact,
        })
    }

    fn split_u(&self) -> (Nets, Nets) {
        self.split(Bern::split_u)
    }

    fn split_v(&self) -> (Nets, Nets) {
        self.split(Bern::split_v)
    }

    fn split(&self, how: fn(&Bern) -> (Bern, Bern)) -> (Nets, Nets) {
        let (left, right): (Vec<Bern>, Vec<Bern>) = self.all().into_iter().map(how).unzip();
        let (exact_left, exact_right) = match &self.exact {
            Some(forms) => {
                let (l, r): (Vec<Bern>, Vec<Bern>) = forms.iter().map(how).unzip();
                (Some(l), Some(r))
            }
            None => (None, None),
        };
        // Eleven in, eleven out: `rebuilt` cannot fail; keep the piece whole
        // rather than lose it if it ever did.
        match (
            Nets::rebuilt(left, exact_left),
            Nets::rebuilt(right, exact_right),
        ) {
            (Some(l), Some(r)) => (l, r),
            _ => (self.clone(), self.clone()),
        }
    }

    fn normal_may_vanish(&self) -> bool {
        !(self.nn.range().0 > 0.0 && self.w.range().0 > 0.0)
    }

    /// Positive definiteness of `|n| G − d W` over the piece. First by
    /// the ranges of the entries alone: the least eigenvalue of `G` is at
    /// least `det G` over its largest (Gershgorin), and `d xᵀ W x` at most
    /// the bound Gershgorin's discs give it on the side of `d`; that holds
    /// wherever the distance is well short of a radius of curvature. Else
    /// for `|n|` at both bounds `s` of it on the piece (the matrix is
    /// affine in `s`), by the least Bernstein coefficient of the leading
    /// entry and of the determinant `s² det G − s d D₁ + d² det W`, forms
    /// of twice the degree, built only here.
    fn proves(&mut self, distance: f64) -> bool {
        let (low, high) = self.nn.range();
        if !(low > 0.0 && high.is_finite() && self.w.range().0 > 0.0) {
            return false;
        }
        // Scaled by the diagonal of G (which leaves positive definiteness
        // as it is), so that a parametrisation that runs much faster one
        // way than the other does not spoil the discs.
        let (g11, g22) = (self.g11.range(), self.g22.range());
        let (a, b) = (g11.1, g22.1);
        if !(a > 0.0 && b > 0.0) {
            return false;
        }
        let ab = (a * b).sqrt();
        let scaled = |r: (f64, f64), by: f64| (r.0 / by, r.1 / by);
        let (g11, g22, g12) = (scaled(g11, a), scaled(g22, b), scaled(self.g12.range(), ab));
        let off_g = g12.0.abs().max(g12.1.abs());
        let largest = g11.1.max(g22.1) + off_g;
        let least_g = (low / (a * b) / largest).max(g11.0.min(g22.0) - off_g);
        let (w11, w22, w12) = (
            scaled(self.w11.range(), a),
            scaled(self.w22.range(), b),
            scaled(self.w12.range(), ab),
        );
        let off_w = w12.0.abs().max(w12.1.abs());
        let bent = if distance >= 0.0 {
            distance * (w11.1.max(w22.1) + off_w)
        } else {
            distance * (w11.0.min(w22.0) - off_w)
        };
        if low.sqrt() * least_g - bent > 0.0 {
            return true;
        }
        // Then point by point, without the products of the determinant:
        // `|n|` lies between two forms linear in `n · n`, the chord of the
        // square root over the range of `n · n` (below it: the root is
        // concave) and its tangent at the middle of that range (above
        // it); `s G − d W`, scaled alike, with `s` either of them, is
        // diagonally dominant (each diagonal entry above the off-diagonal
        // one in size), by the least Bernstein coefficient of each of the
        // four combinations.
        let (root_low, root_high) = (low.sqrt(), high.sqrt());
        let chord = if high > low {
            let slope = (root_high - root_low) / (high - low);
            (root_low - slope * low, slope)
        } else {
            (root_low, 0.0)
        };
        let middle = 0.5 * (low + high);
        let tangent = (0.5 * middle.sqrt(), 0.5 / middle.sqrt());
        let dominant = [chord, tangent].iter().all(|&(constant, slope)| {
            let entry = |g: &Bern, ng: &Bern, w: &Bern, by: f64| Bern {
                m: g.m,
                n: g.n,
                c: g.c
                    .iter()
                    .zip(&ng.c)
                    .zip(&w.c)
                    .map(|((g, ng), w)| (constant * g + slope * ng - distance * w) / by)
                    .collect(),
            };
            let m11 = entry(&self.g11, &self.ng11, &self.w11, a);
            let m22 = entry(&self.g22, &self.ng22, &self.w22, b);
            let m12 = entry(&self.g12, &self.ng12, &self.w12, ab);
            [m11.sub(&m12), m11.add(&m12), m22.sub(&m12), m22.add(&m12)]
                .iter()
                .all(|form| form.range().0 > 0.0)
        });
        if dominant {
            return true;
        }
        if self.exact.is_none() {
            self.exact = Some(self.exact_forms());
        }
        let Some([g, w, d0, d1, d2]) = self.exact.as_deref() else {
            return false;
        };
        for s in [low.sqrt(), high.sqrt()] {
            let lowest =
                g.c.iter()
                    .zip(&w.c)
                    .map(|(g, w)| s * g - distance * w)
                    .fold(f64::INFINITY, f64::min);
            if !(lowest > 0.0) {
                return false;
            }
            let lowest =
                d0.c.iter()
                    .zip(&d1.c)
                    .zip(&d2.c)
                    .map(|((a, b), c)| s * s * a - s * distance * b + distance * distance * c)
                    .fold(f64::INFINITY, f64::min);
            if !(lowest > 0.0) {
                return false;
            }
        }
        true
    }

    /// `G₁₁` and `W₁₁` at one degree, and `det G`, `D₁ = G₁₁ W₂₂ + G₂₂
    /// W₁₁ − 2 G₁₂ W₁₂` and `det W` at one degree.
    fn exact_forms(&self) -> Vec<Bern> {
        let (m, n) = (self.g11.m.max(self.w11.m), self.g11.n.max(self.w11.n));
        let (g, w) = (self.g11.elevated(m, n), self.w11.elevated(m, n));
        let d1 = self
            .g11
            .mul(&self.w22)
            .add(&self.g22.mul(&self.w11))
            .sub(&self.g12.mul(&self.w12).times(2.0));
        let d2 = self.w11.mul(&self.w22).sub(&self.w12.mul(&self.w12));
        let (m, n) = (self.nn.m.max(d1.m).max(d2.m), self.nn.n.max(d1.n).max(d2.n));
        vec![
            g,
            w,
            self.nn.elevated(m, n),
            d1.elevated(m, n),
            d2.elevated(m, n),
        ]
    }

    /// The matrix at the middle of the piece, exactly.
    fn at_middle(&self, distance: f64) -> Middle {
        let at = |b: &Bern| b.eval(0.5, 0.5);
        let nn = at(&self.nn);
        if !(nn > 0.0 && nn.is_finite() && at(&self.w) > 0.0) {
            return Middle::NoNormal;
        }
        let s = nn.sqrt();
        let (g11, g12, g22) = (at(&self.g11), at(&self.g12), at(&self.g22));
        let (w11, w12, w22) = (at(&self.w11), at(&self.w12), at(&self.w22));
        let d0 = g11 * g22 - g12 * g12;
        let d1 = g11 * w22 + g22 * w11 - 2.0 * g12 * w12;
        let d2 = w11 * w22 - w12 * w12;
        let lead = s * g11 - distance * w11;
        let det = s * s * d0 - s * distance * d1 + distance * distance * d2;
        if lead > 0.0 && det > 0.0 {
            return Middle::Fine;
        }
        // The distances at which the matrix is singular, on the side of
        // `distance`: `d2 x² − s d1 x + s² d0 = 0`.
        let roots: Vec<f64> = if d2.abs() <= 1e-12 * (s * d1).abs().max(f64::MIN_POSITIVE) {
            if d1 != 0.0 {
                vec![s * d0 / d1]
            } else {
                Vec::new()
            }
        } else {
            let disc = (s * d1) * (s * d1) - 4.0 * d2 * s * s * d0;
            let root = disc.max(0.0).sqrt();
            vec![(s * d1 + root) / (2.0 * d2), (s * d1 - root) / (2.0 * d2)]
        };
        let radius = roots
            .into_iter()
            .filter(|x| x.is_finite() && x * distance > 0.0)
            .map(f64::abs)
            .fold(f64::INFINITY, f64::min);
        Middle::Fold(if radius.is_finite() {
            radius
        } else {
            distance.abs()
        })
    }
}

// ---------------------------------------------------------------------
// Bernstein forms over [0, 1]².

/// A polynomial of degrees `(m, n)` in Bernstein form over `[0, 1]²`:
/// coefficient `(i, j)` at `i * (n + 1) + j`. Its values lie between its
/// least and greatest coefficient.
#[derive(Clone, Debug)]
struct Bern {
    m: usize,
    n: usize,
    c: Vec<f64>,
}

/// `C(n, 0), …, C(n, n)`.
fn binomials(n: usize) -> Vec<f64> {
    let mut row = Vec::with_capacity(n + 1);
    let mut value = 1.0;
    for k in 0..=n {
        row.push(value);
        value = value * (n - k) as f64 / (k + 1) as f64;
    }
    row
}

fn binomial(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    let mut value = 1.0;
    for i in 0..k {
        value *= (n - i) as f64 / (i + 1) as f64;
    }
    value
}

/// Most coefficients one form may have.
const MAX_COEFFICIENTS: usize = 1 << 16;

impl Bern {
    fn new(m: usize, n: usize, c: Vec<f64>) -> Option<Bern> {
        (c.len() == (m + 1) * (n + 1)).then_some(Bern { m, n, c })
    }

    fn constant(value: f64) -> Bern {
        Bern {
            m: 0,
            n: 0,
            c: vec![value],
        }
    }

    fn zero(m: usize, n: usize) -> Bern {
        Bern {
            m,
            n,
            c: vec![0.0; (m + 1) * (n + 1)],
        }
    }

    fn at(&self, i: usize, j: usize) -> f64 {
        self.c.get(i * (self.n + 1) + j).copied().unwrap_or(0.0)
    }

    fn du(&self) -> Bern {
        if self.m == 0 {
            return Bern::zero(0, self.n);
        }
        let mut out = Bern::zero(self.m - 1, self.n);
        let factor = self.m as f64;
        for i in 0..self.m {
            for j in 0..=self.n {
                if let Some(slot) = out.c.get_mut(i * (self.n + 1) + j) {
                    *slot = factor * (self.at(i + 1, j) - self.at(i, j));
                }
            }
        }
        out
    }

    fn dv(&self) -> Bern {
        if self.n == 0 {
            return Bern::zero(self.m, 0);
        }
        let mut out = Bern::zero(self.m, self.n - 1);
        let factor = self.n as f64;
        for i in 0..=self.m {
            for j in 0..self.n {
                if let Some(slot) = out.c.get_mut(i * self.n + j) {
                    *slot = factor * (self.at(i, j + 1) - self.at(i, j));
                }
            }
        }
        out
    }

    fn times(&self, factor: f64) -> Bern {
        Bern {
            m: self.m,
            n: self.n,
            c: self.c.iter().map(|x| x * factor).collect(),
        }
    }

    fn mul(&self, other: &Bern) -> Bern {
        // A multiplication per pair of coefficients.
        crate::allowance::charge(
            ((self.c.len() as u64).saturating_mul(other.c.len() as u64)
                / crate::allowance::ARITHMETIC_PER_UNIT)
                .saturating_add(1),
        );
        let (m, n) = (self.m + other.m, self.n + other.n);
        if (m + 1).saturating_mul(n + 1) > MAX_COEFFICIENTS {
            // Never for the degrees of a surface (MAX_DEGREE bounds them
            // far below); a form that says nothing proves nothing.
            return Bern {
                m: 0,
                n: 0,
                c: vec![f64::NAN],
            };
        }
        // In the scaled basis (coefficient times its binomial) a product
        // is a convolution; scaled back by the binomials of the product.
        let scaled = |b: &Bern| {
            let (rows, cols) = (binomials(b.m), binomials(b.n));
            let mut out = b.c.clone();
            for (i, r) in rows.iter().enumerate() {
                for (j, c) in cols.iter().enumerate() {
                    if let Some(slot) = out.get_mut(i * (b.n + 1) + j) {
                        *slot *= r * c;
                    }
                }
            }
            out
        };
        let (a, b) = (scaled(self), scaled(other));
        let mut out = Bern::zero(m, n);
        for i in 0..=self.m {
            for j in 0..=self.n {
                let x = a.get(i * (self.n + 1) + j).copied().unwrap_or(0.0);
                if x == 0.0 {
                    continue;
                }
                for k in 0..=other.m {
                    let base = (i + k) * (n + 1) + j;
                    for l in 0..=other.n {
                        let y = b.get(k * (other.n + 1) + l).copied().unwrap_or(0.0);
                        if let Some(slot) = out.c.get_mut(base + l) {
                            *slot += x * y;
                        }
                    }
                }
            }
        }
        let (rows, cols) = (binomials(m), binomials(n));
        for (i, r) in rows.iter().enumerate() {
            for (j, c) in cols.iter().enumerate() {
                if let Some(slot) = out.c.get_mut(i * (n + 1) + j) {
                    *slot /= r * c;
                }
            }
        }
        out
    }

    /// The same polynomial at degrees `(m, n)` (at least its own): the
    /// product with 1 written at the degrees it is raised by (all of whose
    /// coefficients are 1).
    fn elevated(&self, m: usize, n: usize) -> Bern {
        if m == self.m && n == self.n {
            return self.clone();
        }
        let (rm, rn) = (m.max(self.m) - self.m, n.max(self.n) - self.n);
        self.mul(&Bern {
            m: rm,
            n: rn,
            c: vec![1.0; (rm + 1) * (rn + 1)],
        })
    }

    fn combined(&self, other: &Bern, sign: f64) -> Bern {
        let (m, n) = (self.m.max(other.m), self.n.max(other.n));
        let (a, b) = (self.elevated(m, n), other.elevated(m, n));
        Bern {
            m,
            n,
            c: a.c.iter().zip(&b.c).map(|(x, y)| x + sign * y).collect(),
        }
    }

    fn add(&self, other: &Bern) -> Bern {
        self.combined(other, 1.0)
    }

    fn sub(&self, other: &Bern) -> Bern {
        self.combined(other, -1.0)
    }

    /// The least and the greatest coefficient (NaN anywhere: NaN).
    fn range(&self) -> (f64, f64) {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for x in &self.c {
            if x.is_nan() {
                return (f64::NAN, f64::NAN);
            }
            low = low.min(*x);
            high = high.max(*x);
        }
        (low, high)
    }

    /// The same polynomial over the box `[a₀, a₁] × [b₀, b₁]` of its
    /// parameters (which may reach past `[0, 1]`) as the new `[0, 1]²`.
    fn over(&self, (a0, a1): (f64, f64), (b0, b1): (f64, f64)) -> Bern {
        if (a0, a1, b0, b1) == (0.0, 1.0, 0.0, 1.0) {
            return self.clone();
        }
        let mut out = self.clone();
        for j in 0..=self.n {
            let column: Vec<f64> = (0..=self.m).map(|i| self.at(i, j)).collect();
            for (i, value) in Bern::segment(&column, a0, a1).into_iter().enumerate() {
                if let Some(slot) = out.c.get_mut(i * (self.n + 1) + j) {
                    *slot = value;
                }
            }
        }
        let rows = out.clone();
        for i in 0..=self.m {
            let row: Vec<f64> = (0..=self.n).map(|j| rows.at(i, j)).collect();
            for (j, value) in Bern::segment(&row, b0, b1).into_iter().enumerate() {
                if let Some(slot) = out.c.get_mut(i * (self.n + 1) + j) {
                    *slot = value;
                }
            }
        }
        out
    }

    /// The coefficients of a polynomial of one variable over `[a, b]`
    /// (with `b > 0`): its piece over `[0, b]` by de Casteljau at `b`,
    /// then of that the piece from `a / b`.
    fn segment(values: &[f64], a: f64, b: f64) -> Vec<f64> {
        if !(b > 0.0) {
            return values.to_vec();
        }
        let (left, _) = Bern::cut(values, b);
        let (_, right) = Bern::cut(&left, a / b);
        right
    }

    /// de Casteljau at `t` (any number): the coefficients over `[0, t]`
    /// and over `[t, 1]`.
    fn cut(values: &[f64], t: f64) -> (Vec<f64>, Vec<f64>) {
        let mut work = values.to_vec();
        let count = work.len();
        let mut left = Vec::with_capacity(count);
        let mut right = vec![0.0; count];
        for level in 0..count {
            if let Some(first) = work.first() {
                left.push(*first);
            }
            if let (Some(last), Some(slot)) = (
                work.get(count - 1 - level),
                right.get_mut(count - 1 - level),
            ) {
                *slot = *last;
            }
            for k in 0..count.saturating_sub(level + 1) {
                let (Some(a), Some(b)) = (work.get(k).copied(), work.get(k + 1).copied()) else {
                    continue;
                };
                if let Some(slot) = work.get_mut(k) {
                    *slot = a + (b - a) * t;
                }
            }
        }
        (left, right)
    }

    /// de Casteljau along one row of coefficients: the two halves at 1/2.
    fn halves(values: &[f64]) -> (Vec<f64>, Vec<f64>) {
        let mut work = values.to_vec();
        let count = work.len();
        let mut left = Vec::with_capacity(count);
        let mut right = vec![0.0; count];
        for level in 0..count {
            if let Some(first) = work.first() {
                left.push(*first);
            }
            if let (Some(last), Some(slot)) = (
                work.get(count - 1 - level),
                right.get_mut(count - 1 - level),
            ) {
                *slot = *last;
            }
            for k in 0..count.saturating_sub(level + 1) {
                let (Some(a), Some(b)) = (work.get(k).copied(), work.get(k + 1).copied()) else {
                    continue;
                };
                if let Some(slot) = work.get_mut(k) {
                    *slot = 0.5 * (a + b);
                }
            }
        }
        (left, right)
    }

    /// Charges a split along a direction of degree `degree`: de Casteljau
    /// on every line of coefficients, half the square of its length in
    /// additions each.
    fn charge_split(&self, degree: usize) {
        let work = (self.c.len() as u64).saturating_mul(degree as u64 + 1) / 2;
        crate::allowance::charge(work / crate::allowance::ARITHMETIC_PER_UNIT + 1);
    }

    fn split_u(&self) -> (Bern, Bern) {
        self.charge_split(self.m);
        let (mut left, mut right) = (Bern::zero(self.m, self.n), Bern::zero(self.m, self.n));
        for j in 0..=self.n {
            let column: Vec<f64> = (0..=self.m).map(|i| self.at(i, j)).collect();
            let (l, r) = Bern::halves(&column);
            for (i, (a, b)) in l.into_iter().zip(r).enumerate() {
                let index = i * (self.n + 1) + j;
                if let Some(slot) = left.c.get_mut(index) {
                    *slot = a;
                }
                if let Some(slot) = right.c.get_mut(index) {
                    *slot = b;
                }
            }
        }
        (left, right)
    }

    fn split_v(&self) -> (Bern, Bern) {
        self.charge_split(self.n);
        let (mut left, mut right) = (Bern::zero(self.m, self.n), Bern::zero(self.m, self.n));
        for i in 0..=self.m {
            let row: Vec<f64> = (0..=self.n).map(|j| self.at(i, j)).collect();
            let (l, r) = Bern::halves(&row);
            for (j, (a, b)) in l.into_iter().zip(r).enumerate() {
                let index = i * (self.n + 1) + j;
                if let Some(slot) = left.c.get_mut(index) {
                    *slot = a;
                }
                if let Some(slot) = right.c.get_mut(index) {
                    *slot = b;
                }
            }
        }
        (left, right)
    }

    fn eval(&self, s: f64, t: f64) -> f64 {
        let mut sum = 0.0;
        for i in 0..=self.m {
            let bu = binomial(self.m, i) * s.ppowi(i as i32) * (1.0 - s).ppowi((self.m - i) as i32);
            for j in 0..=self.n {
                let bv =
                    binomial(self.n, j) * t.ppowi(j as i32) * (1.0 - t).ppowi((self.n - j) as i32);
                sum += self.at(i, j) * bu * bv;
            }
        }
        sum
    }
}

type B3 = [Bern; 3];

fn map(v: &B3, f: fn(&Bern) -> Bern) -> B3 {
    [f(&v[0]), f(&v[1]), f(&v[2])]
}

fn scale3(v: &B3, by: &Bern) -> B3 {
    [v[0].mul(by), v[1].mul(by), v[2].mul(by)]
}

fn add3(a: &B3, b: &B3) -> B3 {
    [a[0].add(&b[0]), a[1].add(&b[1]), a[2].add(&b[2])]
}

fn sub3(a: &B3, b: &B3) -> B3 {
    [a[0].sub(&b[0]), a[1].sub(&b[1]), a[2].sub(&b[2])]
}

fn dot(a: &B3, b: &B3) -> Bern {
    a[0].mul(&b[0]).add(&a[1].mul(&b[1])).add(&a[2].mul(&b[2]))
}

fn cross(a: &B3, b: &B3) -> B3 {
    [
        a[1].mul(&b[2]).sub(&a[2].mul(&b[1])),
        a[2].mul(&b[0]).sub(&a[0].mul(&b[2])),
        a[0].mul(&b[1]).sub(&a[1].mul(&b[0])),
    ]
}

// ---------------------------------------------------------------------
// The approximation.

/// Most samples in one direction (what the banded interpolation takes),
/// and in all: the fit has a control point per sample, and a surface may
/// have [`crate::MAX_CONTROL_POINTS`] in each direction; the bound in all is the
/// work and memory of the fit (each pass holds the fit against the exact
/// offset at up to sixteen points per sample). It was
/// `MAX_CONTROL_POINTS` in all until 2026-10-27: a developable panel of
/// 782 control points along it (3912 samples) could then be refined
/// across it to 16 samples only, where the sea kayak's panel near its tip
/// needs 27 (repro 14).
const MAX_PER_DIRECTION: usize = crate::refine::MAX_INTERPOLATED_BANDED;
const MAX_SAMPLES: usize = 1 << 18;

/// Most evaluations of the exact offset one fit makes, its samples and
/// the points it is held at together: 2²⁵, about ten seconds in release.
/// With [`MAX_SAMPLES`] a fit of many passes could otherwise make about
/// 10⁸ (sixteen points per sample at the quarters, two dozen passes),
/// minutes; the sea kayak's longest panel makes about 10⁷.
pub const MAX_EVALUATIONS: u64 = 1 << 25;
/// Passes of the refinement.
const PASSES: usize = 24;
/// Steps each knot span of the surface is cut into to begin with: enough
/// for a cubic in every piece between two breaks.
const START: usize = FIT_DEGREE;
/// The degree of the fit between breaks: a quintic through samples of an
/// analytic function converges at sixth order, so a tight tolerance on a
/// large face takes few samples.
const FIT_DEGREE: usize = 5;

/// A break of the fit: where, and whether the offset is once
/// continuously differentiable across it (the fit then joins with its
/// first derivative too).
type Break = (f64, bool);

/// The knots of a B-spline of `degree` in its domain at which its offset
/// is less smooth than twice continuously differentiable (the normal
/// costs one derivative: at a simple knot of a cubic the offset's
/// curvature jumps), with the ends of the domain: the breaks of the fit in
/// that direction. Between two breaks the offset is analytic and the fit
/// converges at its full order; across a knot where the offset's
/// curvature jumps a fit that goes on smoothly converges at second order
/// only, and refining there inserts whole rows of samples across the
/// surface. Where the surface is twice continuously differentiable (a
/// simple knot of a cubic) its normal and so its offset are once: the fit
/// joins there with its first derivative, so the offset's normal does not
/// jump by what two fits on either side leave (a ball rolling along the
/// face would see that as a crease); where the surface is less smooth the
/// fit only joins.
fn breaks(knots: &[f64], degree: usize, (lo, hi): (f64, f64)) -> Vec<Break> {
    let mut out = vec![(lo, false)];
    let mut index = 0;
    while let Some(&k) = knots.get(index) {
        let multiplicity = knots
            .iter()
            .skip(index)
            .take_while(|x| **x == k)
            .count()
            .max(1);
        // Continuity C^(degree − multiplicity); the offset's one less.
        if k > lo && k < hi && degree < multiplicity + 3 {
            out.push((k, degree >= multiplicity + 2));
        }
        index += multiplicity;
    }
    out.push((hi, false));
    out
}

/// The parameters to begin with in one direction: every knot span of the
/// surface in its domain `(lo, hi)` cut into [`START`] steps, and what
/// `over` reaches past it on either side in steps as long as those of the
/// span next to it (at least one): the fit goes on across the domain's
/// ends, and an interpolating spline through steps that change in length
/// by a large factor at once swings between them.
fn start(knots: &[f64], (lo, hi): (f64, f64), over: (f64, f64)) -> Vec<f64> {
    let mut distinct: Vec<f64> = Vec::new();
    if over.0 < lo {
        distinct.push(over.0);
    }
    for &k in knots {
        if k >= lo && k <= hi && distinct.last().is_none_or(|last| k > *last) {
            distinct.push(k);
        }
    }
    if over.1 > hi {
        distinct.push(over.1);
    }
    if distinct.len() < 2 {
        distinct = vec![over.0, over.1];
    }
    let mut out = Vec::with_capacity(distinct.len() * START);
    let last_pair = distinct.len().saturating_sub(2);
    for (index, pair) in distinct.windows(2).enumerate() {
        let [a, b] = pair else { continue };
        // A part past the domain: as many steps as the span next to it
        // would have over that length.
        let next = if index == 0 && over.0 < lo {
            distinct.get(1..3)
        } else if index == last_pair && over.1 > hi && index > 0 {
            distinct.get(index - 1..index + 1)
        } else {
            None
        };
        let steps = match next {
            Some([c, d]) if d > c => {
                let count = (START as f64 * (b - a) / (d - c)).ceil();
                if count.is_finite() && count >= 1.0 {
                    (count as usize).min(START)
                } else {
                    1
                }
            }
            _ => START,
        };
        for k in 0..steps {
            out.push(a + (b - a) * k as f64 / steps as f64);
        }
    }
    if let Some(last) = distinct.last() {
        out.push(*last);
    }
    out
}

/// The B-spline of [`FIT_DEGREE`] (lower with fewer points in a piece)
/// through `values` at `params`, laid piece by piece between the
/// `breaks` (parameters among `params`, the first and the last of them
/// included) and joined there: where the break is smooth ([`breaks`]) by
/// a knot of the degree less one, so the spline is once continuously
/// differentiable across it ([`joined`]), else by a knot of full
/// multiplicity, the pieces sharing the control point at the break, which
/// is the value there.
fn pieces(
    params: &[f64],
    breaks: &[Break],
    values: &[Vec<f64>],
) -> Option<(usize, Vec<f64>, Vec<Vec<f64>>)> {
    let mut found: Vec<(usize, bool)> = breaks
        .iter()
        .filter_map(|(b, smooth)| params.iter().position(|p| p == b).map(|i| (i, *smooth)))
        .collect();
    found.dedup_by_key(|(i, _)| *i);
    let bounds: Vec<usize> = found.iter().map(|(i, _)| *i).collect();
    if bounds.first() != Some(&0) || bounds.last() != Some(&(params.len().checked_sub(1)?)) {
        return None;
    }
    let shortest = bounds.windows(2).map(|w| match w {
        [a, b] => b - a,
        _ => 0,
    });
    let degree = shortest.min()?.min(FIT_DEGREE);
    if degree == 0 {
        return None;
    }
    // The inner breaks that are smooth.
    let smooth: Vec<bool> = found
        .iter()
        .skip(1)
        .take(bounds.len().saturating_sub(2))
        .map(|(_, s)| *s)
        .collect();
    if degree >= 2
        && smooth.iter().any(|s| *s)
        && let Some(fit) = joined(params, &bounds, &smooth, degree, values)
    {
        return Some(fit);
    }
    let count = bounds.len() - 1;
    let mut knots = Vec::new();
    let mut ctrl: Vec<Vec<f64>> = Vec::new();
    for (k, pair) in bounds.windows(2).enumerate() {
        let [a, b] = pair else { return None };
        let (piece_knots, piece_ctrl) =
            interpolate(params.get(*a..=*b)?, degree, values.get(*a..=*b)?).ok()?;
        let first = k == 0;
        let last = k + 1 == count;
        let from = if first { 0 } else { degree + 1 };
        let to = if last {
            piece_knots.len()
        } else {
            piece_knots.len() - 1
        };
        knots.extend(piece_knots.get(from..to)?.iter().copied());
        ctrl.extend(piece_ctrl.into_iter().skip(usize::from(!first)));
    }
    Some((degree, knots, ctrl))
}

/// [`pieces`] in one interpolation across the inner breaks: per piece its
/// own averaged knots (as [`interpolate`] lays them), a knot of the degree
/// less one at a smooth break and of the degree at another; a smooth
/// break takes one control point less than a piece joined at a shared
/// point would, so the piece after it is given one knot more (its knots
/// the averages of one parameter fewer). `None` where that knot vector
/// does not give one solution (the Schoenberg–Whitney condition).
fn joined(
    params: &[f64],
    bounds: &[usize],
    smooth: &[bool],
    degree: usize,
    values: &[Vec<f64>],
) -> Option<(usize, Vec<f64>, Vec<Vec<f64>>)> {
    let (first, last) = (*params.first()?, *params.last()?);
    let mut knots = vec![first; degree + 1];
    let mut extra = false;
    for (k, pair) in bounds.windows(2).enumerate() {
        let [a, b] = pair else { return None };
        let piece = params.get(*a..=*b)?;
        let window = if extra { degree - 1 } else { degree };
        let n = piece.len();
        for j in 1..n.checked_sub(window)? {
            let sum: f64 = piece.iter().skip(j).take(window).sum();
            knots.push(sum / window as f64);
        }
        if k + 2 < bounds.len() {
            let at = *piece.last()?;
            let soft = smooth.get(k).copied().unwrap_or(false);
            knots.extend(std::iter::repeat_n(
                at,
                if soft { degree - 1 } else { degree },
            ));
            extra = soft;
        }
    }
    knots.extend(std::iter::repeat_n(last, degree + 1));
    let ctrl = interpolate_on(params, degree, &knots, values).ok()?;
    Some((degree, knots, ctrl))
}

/// The B-spline through the grid `rows[i][j]` at `(us[i], vs[j])`, cubic
/// between the breaks of each direction.
fn tensor(
    us: &[f64],
    vs: &[f64],
    (breaks_u, breaks_v): (&[Break], &[Break]),
    rows: &[Vec<Vec3>],
) -> Option<NurbsSurface> {
    let mut knots_v = Vec::new();
    let mut degree_v = 0;
    let mut along_v: Vec<Vec<f64>> = Vec::with_capacity(us.len());
    for row in rows {
        let values: Vec<Vec<f64>> = row.iter().map(|p| vec![p.x, p.y, p.z]).collect();
        let (degree, knots, ctrl) = pieces(vs, breaks_v, &values)?;
        (degree_v, knots_v) = (degree, knots);
        along_v.push(ctrl.into_iter().flatten().collect());
    }
    let (degree_u, knots_u, net) = pieces(us, breaks_u, &along_v)?;
    let count_v = knots_v.len().checked_sub(degree_v + 1)?;
    let count_u = net.len();
    let mut ctrl = Vec::with_capacity(count_u * count_v);
    for row in &net {
        for j in 0..count_v {
            let at = |k: usize| row.get(3 * j + k).copied();
            ctrl.push(Vec3::new(at(0)?, at(1)?, at(2)?));
        }
    }
    let weights = vec![1.0; ctrl.len()];
    NurbsSurface::new(
        degree_u, degree_v, knots_u, knots_v, count_u, count_v, ctrl, weights,
    )
    .ok()
}

/// How fast a change at one sample of an interpolating spline dies away
/// from sample to sample (evenly spaced, continuous up to the degree less
/// one): the root of the Euler–Frobenius polynomial of the degree nearest
/// −1 from inside, in size. For a cubic, `2 − √3` (the root of
/// `t² + 4t + 1`); for a quintic, the root of `t⁴ + 26t³ + 66t² + 26t + 1`
/// near −0.43.
pub const CUBIC_DECAY: f64 = 0.267_949_192_431_122_7;
pub const QUINTIC_DECAY: f64 = 0.430_575_347_099_973_8;

/// Which steps of an interpolating spline to halve, from how far over its
/// bound it is at each (`over`, 1 at the bound): every step over it, and
/// the steps next to one that the halving of it can push over it (a
/// change at one sample dies away by `decay` per sample; halving only
/// what is over makes the fit creep outward one step per pass).
pub fn steps_to_halve(over: &[f64], decay: f64) -> Vec<bool> {
    let n = over.len();
    let mut split: Vec<bool> = over.iter().map(|o| !(*o <= 1.0)).collect();
    let largest_within = over
        .iter()
        .copied()
        .filter(|o| *o <= 1.0)
        .fold(0.0_f64, f64::max);
    for (i, &o) in over.iter().enumerate() {
        if o <= 1.0 || !o.is_finite() {
            continue;
        }
        let mut push = o;
        for k in 1..n {
            push *= decay;
            if push + largest_within <= 1.0 {
                break;
            }
            for j in [i.checked_sub(k), (i + k < n).then_some(i + k)]
                .into_iter()
                .flatten()
            {
                if over.get(j).is_some_and(|x| x + push > 1.0)
                    && let Some(slot) = split.get_mut(j)
                {
                    *slot = true;
                }
            }
        }
    }
    split
}

/// Where between two samples the fit is held against the exact offset:
/// the middle while refining, quarters at the end.
const MIDDLE: [f64; 2] = [0.0, 0.5];
const QUARTERS: [f64; 4] = [0.0, 0.25, 0.5, 0.75];

/// The parameters at which the fit is held along one direction: at each
/// fraction of every step, with the step it lies in (`None` at the
/// samples themselves), and the last sample.
fn lattice(params: &[f64], fractions: &[f64]) -> Vec<(f64, Option<usize>)> {
    let mut out = Vec::with_capacity(params.len() * fractions.len() + 1);
    for (i, pair) in params.windows(2).enumerate() {
        let [a, b] = pair else { continue };
        for &f in fractions {
            out.push((a + (b - a) * f, (f > 0.0).then_some(i)));
        }
    }
    if let Some(last) = params.last() {
        out.push((*last, None));
    }
    out
}

/// Gauss–Legendre nodes and weights on [0, 1], three points: exact for
/// polynomials of degree five, which is what the length of a short step
/// of a smooth surface's parameter line needs.
const GAUSS3: [(f64, f64); 3] = [
    (0.112_701_665_379_258_31, 5.0 / 18.0),
    (0.5, 8.0 / 18.0),
    (0.887_298_334_620_741_7, 5.0 / 18.0),
];

/// The lengths along a parameter line from the range `(lo, hi)` to each
/// of the sorted `params` (nothing inside the range), as the integral of
/// `speed` step by step outward, so each length costs one step; a length
/// past `reach` is infinite, and so is every one further out (the
/// lengths grow outward). A speed that is not a number counts as
/// infinite: a point past it is not held.
fn outward(
    params: &[f64],
    (lo, hi): (f64, f64),
    reach: f64,
    speed: &dyn Fn(f64) -> f64,
) -> Vec<f64> {
    let mut out = vec![0.0; params.len()];
    let step = |a: f64, b: f64| -> f64 {
        let length: f64 = GAUSS3
            .iter()
            .map(|(x, w)| w * speed(a + (b - a) * x))
            .sum::<f64>()
            * (b - a).abs();
        if length.is_nan() {
            f64::INFINITY
        } else {
            length
        }
    };
    // Past the end, ascending.
    let (mut at, mut sum) = (hi, 0.0_f64);
    for (slot, &p) in out.iter_mut().zip(params) {
        if p <= hi {
            continue;
        }
        if sum <= reach {
            sum += step(at, p);
            at = p;
        }
        *slot = if sum <= reach { sum } else { f64::INFINITY };
    }
    // Before the start, descending.
    let (mut at, mut sum) = (lo, 0.0_f64);
    for (slot, &p) in out.iter_mut().zip(params).rev() {
        if p >= lo {
            continue;
        }
        if sum <= reach {
            sum += step(p, at);
            at = p;
        }
        *slot = if sum <= reach { sum } else { f64::INFINITY };
    }
    out
}

/// Which points of the lattice `us × vs` (each sorted) lie within the
/// region `held` says: the length along the surface of the straight path
/// in parameters from the nearest point of the box `around` (each
/// parameter clamped to the box) to the point, no more than the reach.
/// A path is no shorter than the surface's own distance between its
/// ends, so a point within the reach of the box on the surface may be
/// left out only where the parameters run far from straight across the
/// surface; the caller's reach carries the margin for that.
fn held_lattice(nurbs: &NurbsSurface, held: &Held, us: &[f64], vs: &[f64]) -> Vec<Vec<bool>> {
    let ((a0, a1), (b0, b1)) = held.around;
    let reach = held.reach;
    let inside_u = |u: f64| u >= a0 && u <= a1;
    let inside_v = |v: f64| v >= b0 && v <= b1;
    // Lines of u past the box at each v inside it, of v at each u inside
    // it: lengths step by step outward.
    let any_outside_u = us.iter().any(|u| !inside_u(*u));
    let along_u: Vec<Option<Vec<f64>>> = vs
        .iter()
        .map(|&v| {
            (inside_v(v) && any_outside_u).then(|| {
                outward(us, (a0, a1), reach, &|u: f64| {
                    nurbs.derivatives(u, v).1.length()
                })
            })
        })
        .collect();
    // Past a corner of the box: the straight path from that corner.
    let diagonal = |u: f64, v: f64| -> f64 { held.length(nurbs, u, v) };
    us.iter()
        .enumerate()
        .map(|(i, &u)| {
            // Past a bound the rest is not measured (the caller refuses).
            if crate::allowance::exhausted() {
                return vec![false; vs.len()];
            }
            let along_v = inside_u(u).then(|| {
                outward(vs, (b0, b1), reach, &|v: f64| {
                    nurbs.derivatives(u, v).2.length()
                })
            });
            vs.iter()
                .enumerate()
                .map(|(j, &v)| {
                    let length = match (inside_u(u), inside_v(v)) {
                        (true, true) => 0.0,
                        (true, false) => along_v
                            .as_ref()
                            .and_then(|l| l.get(j).copied())
                            .unwrap_or(f64::INFINITY),
                        (false, true) => along_u
                            .get(j)
                            .and_then(|l| l.as_ref())
                            .and_then(|l| l.get(i).copied())
                            .unwrap_or(f64::INFINITY),
                        (false, false) => diagonal(u, v),
                    };
                    length <= reach
                })
                .collect()
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn fit(
    surface: &Surface,
    nurbs: &NurbsSurface,
    distance: f64,
    limit: f64,
    over: Domain,
    turn: &dyn Fn(f64, f64) -> Vec3,
    held: Option<&Held>,
    budget: u64,
) -> Result<NurbsSurface, OffsetError> {
    let (du, dv) = nurbs.domain();
    let (degree_u, degree_v) = nurbs.degrees();
    // Past the domain the box's ends are the ends of the fit, in place of
    // the domain's: the surface goes on there as its end spans are, and
    // what the offset is made along has to go on smoothly too (the turn,
    // see [`offset_along`]), so the fit goes on across the domain's ends
    // as one piece. A break there would make the surface only continuous
    // along the border of a face that grows past it, where a blend that
    // rolls along the face asks for its normal.
    let widened = |mut found: Vec<Break>, (lo, hi): (f64, f64)| {
        if let Some((first, _)) = found.first_mut()
            && lo < *first
        {
            *first = lo;
        }
        if let Some((last, _)) = found.last_mut()
            && hi > *last
        {
            *last = hi;
        }
        found
    };
    let breaks_u = widened(breaks(nurbs.knots_u(), degree_u, du), over.0);
    let breaks_v = widened(breaks(nurbs.knots_v(), degree_v, dv), over.1);
    let mut us = start(nurbs.knots_u(), du, over.0);
    let mut vs = start(nurbs.knots_v(), dv, over.1);
    if us.len() > MAX_PER_DIRECTION
        || vs.len() > MAX_PER_DIRECTION
        || us.len().saturating_mul(vs.len()) > MAX_SAMPLES
    {
        return Err(OffsetError::NoFit);
    }
    let evaluations = std::cell::Cell::new(0_u64);
    let point = |u: f64, v: f64| {
        evaluations.set(evaluations.get().saturating_add(1));
        if evaluations.get() > budget {
            return Err(OffsetError::OverBudget);
        }
        let (at, du, dv) = surface.derivatives(u, v);
        du.cross(dv)
            .normalized()
            .and_then(|n| (n + turn(u, v)).normalized())
            .map(|n| at + n * distance)
            .filter(|p| p.is_finite())
            .ok_or(OffsetError::NoNormal { at: (u, v) })
    };
    let mut rows: Vec<Vec<Vec3>> = Vec::with_capacity(us.len());
    for &u in &us {
        rows.push(vs.iter().map(|&v| point(u, v)).collect::<Result<_, _>>()?);
    }
    let mut verified = false;
    for _ in 0..PASSES {
        if crate::allowance::pass() {
            return Err(OffsetError::Spent);
        }
        let fitted = tensor(&us, &vs, (&breaks_u, &breaks_v), &rows).ok_or(OffsetError::NoFit)?;
        let fractions: &[f64] = if verified { &QUARTERS } else { &MIDDLE };
        let (at_u, at_v) = (lattice(&us, fractions), lattice(&vs, fractions));
        // A point on a line of samples across a step says how the fit
        // holds along that direction alone; a point inside a cell is put
        // down to the direction whose steps hold worse, or to both when
        // neither alone is over (halving the steps of the other direction
        // as well would multiply the samples to no purpose).
        let mut over_u = vec![0.0_f64; us.len().saturating_sub(1)];
        let mut over_v = vec![0.0_f64; vs.len().saturating_sub(1)];
        let mut inside: Vec<(usize, usize, f64)> = Vec::new();
        let kept = held.map(|h| {
            let us: Vec<f64> = at_u.iter().map(|(u, _)| *u).collect();
            let vs: Vec<f64> = at_v.iter().map(|(v, _)| *v).collect();
            held_lattice(nurbs, h, &us, &vs)
        });
        // The held region measured point by point past the box's corners
        // is the dearest part of a pass on a large lattice: a bound
        // reached there ends the fit (what it marked is not trusted).
        if crate::allowance::exhausted() {
            return Err(OffsetError::Spent);
        }
        for (i, &(u, step_u)) in at_u.iter().enumerate() {
            for (j, &(v, step_v)) in at_v.iter().enumerate() {
                // Not used: not held to the limit (`Held`).
                if let Some(kept) = &kept
                    && !kept
                        .get(i)
                        .and_then(|row| row.get(j))
                        .copied()
                        .unwrap_or(false)
                {
                    continue;
                }
                let off = match (step_u, step_v) {
                    // A sample: the fit passes through it.
                    (None, None) => continue,
                    _ => fitted.eval(u, v).distance(point(u, v)?) / limit,
                };
                let off = if off.is_nan() { f64::INFINITY } else { off };
                match (step_u, step_v) {
                    (Some(i), Some(j)) => inside.push((i, j, off)),
                    (Some(i), None) => {
                        if let Some(slot) = over_u.get_mut(i) {
                            *slot = slot.max(off);
                        }
                    }
                    (None, Some(j)) => {
                        if let Some(slot) = over_v.get_mut(j) {
                            *slot = slot.max(off);
                        }
                    }
                    (None, None) => {}
                }
            }
        }
        let (along_u, along_v) = (over_u.clone(), over_v.clone());
        for (i, j, off) in inside {
            let (pu, pv) = (
                along_u.get(i).copied().unwrap_or(0.0),
                along_v.get(j).copied().unwrap_or(0.0),
            );
            let both = pu <= 1.0 && pv <= 1.0;
            if (both || pu >= pv)
                && let Some(slot) = over_u.get_mut(i)
            {
                *slot = slot.max(off);
            }
            if (both || pv >= pu)
                && let Some(slot) = over_v.get_mut(j)
            {
                *slot = slot.max(off);
            }
        }
        let holds = over_u.iter().chain(&over_v).all(|o| *o <= 1.0);
        if holds {
            if verified {
                return Ok(fitted);
            }
            // Held at the middles: look once more, finer.
            verified = true;
            continue;
        }
        let split_u = steps_to_halve(&over_u, QUINTIC_DECAY);
        let split_v = steps_to_halve(&over_v, QUINTIC_DECAY);
        let new_u: Vec<f64> = us
            .windows(2)
            .zip(&split_u)
            .filter(|(_, s)| **s)
            .filter_map(|(w, _)| match w {
                [a, b] => Some(0.5 * (a + b)),
                _ => None,
            })
            .collect();
        let new_v: Vec<f64> = vs
            .windows(2)
            .zip(&split_v)
            .filter(|(_, s)| **s)
            .filter_map(|(w, _)| match w {
                [a, b] => Some(0.5 * (a + b)),
                _ => None,
            })
            .collect();
        let (count_u, count_v) = (us.len() + new_u.len(), vs.len() + new_v.len());
        if count_u > MAX_PER_DIRECTION
            || count_v > MAX_PER_DIRECTION
            || count_u.saturating_mul(count_v) > MAX_SAMPLES
        {
            return Err(OffsetError::NoFit);
        }
        // The new columns in every row, then the new rows.
        for v in new_v {
            let at = vs.partition_point(|x| *x < v);
            for (row, &u) in rows.iter_mut().zip(&us) {
                row.insert(at, point(u, v)?);
            }
            vs.insert(at, v);
        }
        for u in new_u {
            let at = us.partition_point(|x| *x < u);
            let row = vs.iter().map(|&v| point(u, v)).collect::<Result<_, _>>()?;
            rows.insert(at, row);
            us.insert(at, u);
        }
    }
    Err(OffsetError::NoFit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sheet `(u, v, h (v − a)⁵)` over `[0, 1]²`, a quintic across, gently
    /// curved inside (its curvature at most `20 h (1 − a)³`) and sharply
    /// past its border `v = 1`, where the quintic goes on turning.
    fn quintic_sheet(h: f64, a: f64, scale: f64) -> Surface {
        // The Bézier points of `(v − a)⁵` are its polar form at 0 and 1:
        // `(1 − a)ᵏ (−a)⁵⁻ᵏ`.
        let z = |k: i32| h * (1.0 - a).ppowi(k) * (-a).ppowi(5 - k);
        let mut ctrl = Vec::new();
        for i in 0..2 {
            for k in 0..6 {
                ctrl.push(Vec3::new(f64::from(i), f64::from(k) / 5.0, z(k)) * scale);
            }
        }
        Surface::Nurbs(
            NurbsSurface::new(
                1,
                5,
                vec![0.0, 0.0, 1.0, 1.0],
                vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
                2,
                6,
                ctrl,
                vec![1.0; 12],
            )
            .unwrap(),
        )
    }

    /// Where a surface's offset folds only past its domain, as its end
    /// span's polynomial goes on there, it is continued by its Taylor
    /// expansion instead (offset_held): the offset over a box past the
    /// border is fitted within the limit inside the domain, over the box
    /// asked for.
    #[test]
    fn an_offset_that_folds_only_past_the_border_is_made_over_the_taylor_continuation() {
        for scale in [1e-3, 1.0, 1e6] {
            let sheet = quintic_sheet(1.0, 0.9, scale);
            let (d, limit) = (1.0 * scale, 1e-6 * scale);
            let over = ((0.0, 1.0), (0.0, 1.35));
            // The plain continuation folds past the border (the curvature
            // reaches 1 / d near v = 1.3), and only there.
            assert!(regular_over(&sheet, d, ((0.0, 1.0), (0.0, 1.0))).is_ok());
            assert!(
                matches!(
                    regular_over(&sheet, d, over),
                    Err(OffsetError::Folds { .. } | OffsetError::Unproven { .. })
                ),
                "scale {scale}"
            );
            let fitted = offset_over(&sheet, d, limit, over)
                .unwrap_or_else(|e| panic!("scale {scale}: {e:?}"));
            // Over the box asked for, which the Taylor continuation spans.
            let ((u0, u1), (v0, v1)) = fitted.domain();
            assert!(u0 <= 0.0 && u1 >= 1.0 && v0 <= 0.0, "{:?}", fitted.domain());
            assert!((v1 - 1.35).abs() <= 1e-12, "{:?}", fitted.domain());
            for i in 0..=8 {
                for j in 0..=8 {
                    let (u, v) = (f64::from(i) / 8.0, f64::from(j) / 8.0);
                    let exact = exact(&sheet, d, u, v).unwrap();
                    let off = fitted.eval(u, v).distance(exact);
                    assert!(
                        off <= 1.01 * limit,
                        "scale {scale}: off by {off:e} at ({u}, {v})"
                    );
                }
            }
        }
    }

    /// A fit that would need more evaluations of the exact offset than
    /// its budget stops and says so, rather than going on.
    #[test]
    fn a_fit_past_its_budget_is_refused_as_over_budget() {
        let ctrl: Vec<Vec3> = (0..4)
            .flat_map(|i| {
                (0..4).map(move |j| {
                    let (x, y) = (f64::from(i) / 3.0, f64::from(j) / 3.0);
                    Vec3::new(x, y, (3.0 * x).psin() * (2.0 * y).pcos())
                })
            })
            .collect();
        let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        let nurbs =
            NurbsSurface::new(3, 3, knots.clone(), knots, 4, 4, ctrl, vec![1.0; 16]).unwrap();
        let surface = Surface::Nurbs(nurbs.clone());
        let over = nurbs.domain();
        let zero = |_: f64, _: f64| Vec3::ZERO;
        assert_eq!(
            fit(&surface, &nurbs, 0.01, 1e-9, over, &zero, None, 50),
            Err(OffsetError::OverBudget)
        );
        assert!(
            fit(
                &surface,
                &nurbs,
                0.01,
                1e-6,
                over,
                &zero,
                None,
                MAX_EVALUATIONS
            )
            .is_ok()
        );
    }

    /// A simple knot of a cubic is a smooth break (its offset is once
    /// differentiable there), a double one is not; the pieces through
    /// samples of a function that is once differentiable at a smooth
    /// break join with their first derivative, through every sample.
    #[test]
    fn pieces_join_with_their_slope_at_a_smooth_break() {
        let knots = [0.0, 0.0, 0.0, 0.0, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0, 1.0];
        assert_eq!(
            breaks(&knots, 3, (0.0, 1.0)),
            vec![(0.0, false), (0.5, true), (0.75, false), (1.0, false)]
        );
        // |t − 0.5|³ + t: once differentiable at 0.5 (twice, in fact, but
        // not three times), analytic on either side.
        let f = |t: f64| (t - 0.5).abs().ppowi(3) + t;
        let params: Vec<f64> = (0..=20).map(|i| f64::from(i) / 20.0).collect();
        let values: Vec<Vec<f64>> = params.iter().map(|t| vec![f(*t), 0.0, 0.0]).collect();
        let (degree, knots, ctrl) =
            pieces(&params, &[(0.0, false), (0.5, true), (1.0, false)], &values).unwrap();
        assert_eq!(degree, FIT_DEGREE);
        assert_eq!(knots.iter().filter(|k| **k == 0.5).count(), FIT_DEGREE - 1);
        let points: Vec<Vec3> = ctrl.iter().map(|r| Vec3::new(r[0], r[1], r[2])).collect();
        let curve = crate::NurbsCurve::polynomial(degree, knots, points).unwrap();
        for (t, v) in params.iter().zip(&values) {
            assert!((curve.eval(*t).x - v[0]).abs() < 1e-13, "{t}");
        }
        let (_, left) = curve.derivatives(0.5 - 1e-12);
        let (_, right) = curve.derivatives(0.5 + 1e-12);
        assert!((left.x - right.x).abs() < 1e-9, "{left:?} {right:?}");
    }

    #[test]
    fn products_and_splits_of_bernstein_forms_are_the_polynomials() {
        // (1 + s)(2 t) over [0, 1]², split and raised.
        let a = Bern::new(1, 0, vec![1.0, 2.0]).unwrap();
        let b = Bern::new(0, 1, vec![0.0, 2.0]).unwrap();
        let p = a.mul(&b);
        for (s, t) in [(0.0, 0.0), (0.3, 0.7), (1.0, 1.0)] {
            assert!((p.eval(s, t) - (1.0 + s) * 2.0 * t).abs() < 1e-12);
            let e = p.elevated(3, 2);
            assert!((e.eval(s, t) - p.eval(s, t)).abs() < 1e-12);
        }
        let (l, r) = p.split_u();
        assert!((l.eval(0.5, 0.4) - p.eval(0.25, 0.4)).abs() < 1e-12);
        assert!((r.eval(0.5, 0.4) - p.eval(0.75, 0.4)).abs() < 1e-12);
        let (l, r) = p.split_v();
        assert!((l.eval(0.4, 0.5) - p.eval(0.4, 0.25)).abs() < 1e-12);
        assert!((r.eval(0.4, 0.5) - p.eval(0.4, 0.75)).abs() < 1e-12);
        let d = p.du();
        assert!((d.eval(0.2, 0.6) - 2.0 * 0.6).abs() < 1e-12);
        let d = p.dv();
        assert!((d.eval(0.2, 0.6) - 2.0 * 1.2).abs() < 1e-12);
    }

    #[test]
    fn a_halving_spreads_to_the_steps_it_would_push_over() {
        let split = steps_to_halve(&[0.1, 0.99, 4.0, 0.1, 0.1], CUBIC_DECAY);
        assert!((CUBIC_DECAY - (2.0 - 3.0_f64.sqrt())).abs() < 1e-15);
        let t = -QUINTIC_DECAY;
        assert!((t.ppowi(4) + 26.0 * t.ppowi(3) + 66.0 * t * t + 26.0 * t + 1.0).abs() < 1e-12);
        assert_eq!(split, vec![false, true, true, true, false]);
    }
}
