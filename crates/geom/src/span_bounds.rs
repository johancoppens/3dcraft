//! Bounds on the second derivatives of a rational Bézier patch, and how
//! far its first derivatives travel across it: what the tessellator sizes
//! the grid over a spline face by (`craft3d_tess` › `face.rs`,
//! `spline_parts`; moved here from there on 2026-11-17, unchanged), and
//! kept with a spline surface per whole knot span once made
//! ([`NurbsSurface::whole_span_bounds`]): a face is given its grid once
//! per round of its body's mesh, and every mesh, the check of a body
//! against itself and a measure ask again, while a span's bounds depend
//! on the span alone. On a sheet of 145 × 113 points (16 128 spans) they
//! were found again some twenty times by one STL export.

use crate::{NurbsSurface, RationalBezierPatch};
use craft3d_math::Vec3;

/// How often a patch is cut in halves before its derivatives are bounded
/// ([`second_bounds`]). Measured on surfaces of revolution, whose
/// rational arcs around the axis are where the hull of the control net
/// is loosest (`craft3d_tess` › `face.rs` ›
/// `tests::the_bounds_on_a_spline_surface_are_above_its_second_differences`):
/// uncut, the bounds are 2.7 to 4.7 times the largest second
/// differences; cut three times 1.6 to 2.4 times, four times 1.5 to 2.1
/// times, six times 1.4 to 1.8 times (on a polynomial surface 1.2 times,
/// cut or not). The cells of the grid go by the root of that. Every cut
/// doubles the pieces, and both ways of cutting are tried: 60 nets are
/// bounded for four cuts, 252 for six.
pub const HALVINGS: u32 = 4;

/// Bounds `(Muu, Muv, Mvv)` on the second derivatives of `patch`, which
/// is a part of a surface `width` wide in the surface's parameters.
///
/// The bound of a patch is that of the hull of its control net
/// (`RationalBezierPatch::derivative_bounds`), which is the nearer to
/// the patch the smaller the patch is; so the patch is cut in halves
/// [`HALVINGS`] times, each time across the parameter in which that
/// brings the bounds down most (a surface of revolution gains by being
/// cut around its axis, where its weights vary, and nothing along it),
/// and the bound is the largest over the pieces. Every round bounds the
/// same patch, so the smallest bound of all rounds stands.
pub fn second_bounds(patch: RationalBezierPatch, width: (f64, f64)) -> Option<(f64, f64, f64)> {
    let over = |pieces: &[RationalBezierPatch], width: (f64, f64)| -> Option<(f64, f64, f64)> {
        let (mut uu, mut uv, mut vv) = (0.0_f64, 0.0_f64, 0.0_f64);
        for piece in pieces {
            let bounds = piece.derivative_bounds()?.over(width.0, width.1);
            uu = uu.max(bounds.second_uu);
            uv = uv.max(bounds.second_uv);
            vv = vv.max(bounds.second_vv);
        }
        Some((uu, uv, vv))
    };
    // What the cells of the grid go by.
    let weight = |(uu, uv, vv): (f64, f64, f64)| uu + 2.0 * uv + vv;
    let mut pieces = vec![patch];
    let mut width = width;
    let mut best = over(&pieces, width)?;
    for _ in 0..HALVINGS {
        let in_u: Vec<RationalBezierPatch> = pieces
            .iter()
            .flat_map(|piece| {
                let (a, b) = piece.split_u(0.5);
                [a, b]
            })
            .collect();
        let in_v: Vec<RationalBezierPatch> = pieces
            .iter()
            .flat_map(|piece| {
                let (a, b) = piece.split_v(0.5);
                [a, b]
            })
            .collect();
        let (half_u, half_v) = ((width.0 / 2.0, width.1), (width.0, width.1 / 2.0));
        let (bound_u, bound_v) = (over(&in_u, half_u)?, over(&in_v, half_v)?);
        let bound = if weight(bound_u) <= weight(bound_v) {
            (pieces, width) = (in_u, half_u);
            bound_u
        } else {
            (pieces, width) = (in_v, half_v);
            bound_v
        };
        best = (
            best.0.min(bound.0),
            best.1.min(bound.1),
            best.2.min(bound.2),
        );
    }
    Some(best)
}

/// Bounds on how far `∂/∂u` travels as `u` crosses a part `width` wide
/// whose patch is `patch`, along any segment through it (`∫ |∂²S/∂u²| du`,
/// whatever `v` does on the way), and the same for `∂/∂v` and `v`:
/// `Σ_k max_j |ΔQ_kj|` over the differences of the Bézier net `Q` of that
/// derivative. Infinite for a rational patch.
pub fn travel(patch: &RationalBezierPatch, width: (f64, f64)) -> (f64, f64) {
    let unknown = (f64::INFINITY, f64::INFINITY);
    let Some((along_u, along_v)) = patch.derivative_nets() else {
        return unknown;
    };
    let (m, n) = patch.degrees();
    // `∂²S/∂u² = (m − 1) Σ_k B_k(u) Σ_j B_j(v) ΔQ_kj` (Bernstein bases of
    // degrees m − 2 and n, `ΔQ_kj = Q_{k+1,j} − Q_kj` of the net of
    // ∂/∂u), and `∫₀¹ B_k(u) du = 1/(m − 1)`. Along a segment on which
    // `u` moves one way, the `v` of each point blends the `ΔQ_kj` of
    // column `k` with weights that sum to one, so `∫ |∂²S/∂u²| du` is at
    // most `Σ_k max_j |ΔQ_kj|`. The net of ∂/∂u has `m` rows of `n + 1`
    // (`i (n + 1) + j`), that of ∂/∂v `m + 1` rows of `n` (`i n + j`).
    let travel = |net: &[Vec3], steps: usize, lines: usize, at: &dyn Fn(usize, usize) -> usize| {
        let mut sum = 0.0_f64;
        for k in 1..steps {
            let mut most = 0.0_f64;
            for l in 0..lines {
                let (Some(&a), Some(&b)) = (net.get(at(k - 1, l)), net.get(at(k, l))) else {
                    return f64::INFINITY;
                };
                most = most.max(a.distance(b));
            }
            sum += most;
        }
        sum
    };
    let in_u = travel(&along_u, m, n + 1, &|k, l| k * (n + 1) + l);
    let in_v = travel(&along_v, n, m + 1, &|k, l| l * n + k);
    // Local parameters run over 0..1: a derivative in them is `width`
    // times the surface's.
    let known = |t: f64| if t.is_finite() { t } else { f64::INFINITY };
    (known(in_u / width.0), known(in_v / width.1))
}

/// What [`NurbsSurface::whole_span_bounds`] keeps of one knot span: the
/// [`second_bounds`] (`None` where there is none) and the [`travel`] of
/// its Bézier patch over the span's width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpanBounds {
    pub second: Option<(f64, f64, f64)>,
    pub travel: (f64, f64),
}

impl SpanBounds {
    /// The bounds of `patch`, a span `width` wide: as the tessellator made
    /// them (its travel, then its second bounds).
    pub fn of(patch: &RationalBezierPatch, width: (f64, f64)) -> SpanBounds {
        let travel = travel(patch, width);
        SpanBounds {
            second: second_bounds(patch.clone(), width),
            travel,
        }
    }
}

/// Where a spline surface keeps its spans' bounds once made: one slot per
/// span of [`NurbsSurface::bezier_spans_shared`], each with the work its
/// making was charged (`crate::allowance`), which finding it charges
/// again. Shared by the surface's clones.
#[derive(Clone, Default)]
pub(crate) struct BoundsCache(std::sync::Arc<std::sync::OnceLock<Vec<Kept>>>);

/// A span's bounds once made, with the work making them was charged.
type Kept = std::sync::OnceLock<(SpanBounds, u64)>;

impl NurbsSurface {
    /// The bounds of the whole knot span `index` of
    /// [`Self::bezier_spans_shared`] (`SpanBounds::of` its patch, over
    /// `(u₁ − u₀, v₁ − v₀)`), made once per span and kept with the
    /// surface and its clones. A span found made charges the work making
    /// it was charged (`crate::allowance`: a cache keeps what a miss cost);
    /// one made while the work is not counted is not kept. `None` for a
    /// surface whose spans cannot be made, or an index past them.
    pub fn whole_span_bounds(&self, index: usize) -> Option<SpanBounds> {
        let spans = self.bezier_spans_shared().ok()?;
        let span = spans.get(index)?;
        let slots = self.bounds.0.get_or_init(|| {
            (0..spans.len())
                .map(|_| std::sync::OnceLock::new())
                .collect()
        });
        let slot = slots.get(index)?;
        if let Some(&(bounds, units)) = slot.get() {
            crate::allowance::charge(units);
            return Some(bounds);
        }
        let width = (span.u.1 - span.u.0, span.v.1 - span.v.0);
        let (bounds, units) = crate::allowance::measured(|| SpanBounds::of(&span.patch, width));
        if crate::allowance::counting() {
            let _ = slot.set((bounds, units));
        }
        Some(bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Portable;

    /// A rational spline surface of 7 × 5 points with uneven knots and
    /// weights, and a polynomial one through a grid of points.
    fn surfaces() -> Vec<NurbsSurface> {
        let (nu, nv) = (7, 5);
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for i in 0..nu {
            for j in 0..nv {
                let (x, y) = (i as f64, j as f64 * 1.3);
                ctrl.push(Vec3::new(x, y, (0.7 * x).psin() * (0.4 * y).pcos()));
                weights.push(1.0 + 0.3 * ((i + 2 * j) % 3) as f64);
            }
        }
        let rational = NurbsSurface::new(
            3,
            2,
            vec![0.0, 0.0, 0.0, 0.0, 0.2, 0.45, 0.5, 1.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 0.0, 0.3, 0.8, 1.0, 1.0, 1.0],
            nu,
            nv,
            ctrl,
            weights,
        )
        .unwrap();
        let (rows, columns) = (9, 12);
        let points: Vec<Vec3> = (0..rows)
            .flat_map(|r| (0..columns).map(move |c| (r, c)))
            .map(|(r, c)| {
                let (x, y) = (c as f64 * 0.5, r as f64 * 0.5);
                Vec3::new(x, y, (x * 1.1).psin() + 0.2 * y * y)
            })
            .collect();
        let grid = crate::grid::interpolate(
            &points,
            rows,
            columns,
            crate::grid::Spacing::Even,
            1e-6,
            1e-9,
        )
        .unwrap();
        vec![rational, grid.surface]
    }

    fn bits(b: &SpanBounds) -> Vec<u64> {
        let second = b.second.map_or(vec![u64::MAX; 3], |(a, c, d)| {
            vec![a.to_bits(), c.to_bits(), d.to_bits()]
        });
        [second, vec![b.travel.0.to_bits(), b.travel.1.to_bits()]].concat()
    }

    /// Kept bounds are the bounds made afresh, bit for bit; found kept they
    /// charge the work their making was charged; a clone made before they
    /// were made finds them; made while the work is not counted they are
    /// given and not kept.
    #[test]
    fn a_spans_kept_bounds_are_its_bounds_and_cost_what_they_cost() {
        for surface in surfaces() {
            let clone = surface.clone();
            let spans = surface.bezier_spans_shared().unwrap();
            assert!(spans.len() > 4);
            for (index, span) in spans.iter().enumerate() {
                let width = (span.u.1 - span.u.0, span.v.1 - span.v.0);
                let (fresh, cost) =
                    crate::allowance::measured(|| SpanBounds::of(&span.patch, width));
                assert!(cost > 0, "splitting a patch is counted");
                let (made, made_cost) =
                    crate::allowance::measured(|| surface.whole_span_bounds(index).unwrap());
                let (found, found_cost) =
                    crate::allowance::measured(|| clone.whole_span_bounds(index).unwrap());
                assert_eq!(bits(&made), bits(&fresh), "span {index}");
                assert_eq!(bits(&found), bits(&fresh), "span {index}");
                assert_eq!((made_cost, found_cost), (cost, cost), "span {index}");
            }
            assert!(surface.whole_span_bounds(spans.len()).is_none());
            let (du, dv) = surface.degrees();
            let (cu, cv) = surface.counts();
            let anew = NurbsSurface::new(
                du,
                dv,
                surface.knots_u().to_vec(),
                surface.knots_v().to_vec(),
                cu,
                cv,
                surface.control_points().to_vec(),
                surface.weights().to_vec(),
            )
            .unwrap();
            let uncounted = crate::allowance::uncounted(|| anew.whole_span_bounds(0));
            let (counted, cost) = crate::allowance::measured(|| anew.whole_span_bounds(0));
            assert_eq!(uncounted.map(|b| bits(&b)), counted.map(|b| bits(&b)));
            assert!(
                cost > 0,
                "made again, and charged, after an uncounted making"
            );
        }
    }
}
