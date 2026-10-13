//! Freeform (Bézier and NURBS) faces in booleans.
//!
//! A Bézier patch lies in the convex hull of its control points, and so
//! does every piece it is split into. A [`PatchTree`] splits a surface
//! into pieces, each with the box around its control points, and gives
//!
//! - a lower bound on the distance from a point to the surface that never
//!   overestimates: the distance to the nearest box. It is what makes a
//!   stretch of a curve provably clear of the surface (`hits.rs`);
//! - the nearest point of the surface: Newton's iteration from the middle
//!   of the nearest piece, and from the next pieces as long as their boxes
//!   are nearer than the best point found, so that a nearer-looking local
//!   minimum is not taken for the nearest point;
//! - the signed distance, by the side of the surface the point is on;
//! - a proof that a stretch of a curve stays within a tolerance of it
//!   (`stretch_within`), from the distances at its ends and a bound on the
//!   second derivatives of the surface over its domain, again from the
//!   hull of a net;
//! - how fast its parameters can change per length travelled on it
//!   (`chart.rs` needs that to follow a loop in the chart of the
//!   parameters), from the Bézier nets of the derivatives and of the
//!   normal field: the smallest singular value of the Jacobian is at least
//!   `|∂u × ∂v| / sqrt(|∂u|² + |∂v|²)`, and each of those is bounded by
//!   the hull of its net.
//!
//! A rational surface (weights that are not all one) lies in the hull of
//! its control points as a polynomial one does, so the boxes and all that
//! is built on them are the same. Its derivatives are no polynomials, but
//! they are bounded through its nets all the same
//! (`RationalBezierPatch::derivative_bounds`), and its normal times the
//! cube of its weight is a polynomial with a net of its own
//! (`RationalBezierPatch::weighted_normal_net`), so the two bounds above
//! exist for it too. They loosen as the weights of a piece differ, and
//! tighten as the piece shrinks (its weights even out): they are taken
//! from the leaves of the tree.

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::Reason;
use craft3d_geom::{BezierPatch, NurbsSurface, RationalBezierPatch, Surface};
use craft3d_math::{Bbox3, Vec3};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub(super) const WEIGHTS: Reason = "a rational freeform face whose weights differ by more than a factor of a thousand within one span: its derivatives cannot be bounded usefully";
pub(super) const IRREGULAR: Reason = "a freeform face whose parametrisation stalls or folds somewhere (a collapsed or self-overlapping patch); its chart cannot be followed";
const TOO_MANY: Reason = "a freeform face with more pieces than booleans can index";

/// How many times a Bézier span is quartered at most, and how many leaves
/// a whole surface may have.
const MAX_DEPTH: usize = 5;
const MAX_LEAVES: usize = 1 << 15;
/// How far apart the weights of one Bézier span of a rational surface may
/// be: every bound on its derivatives carries a power of this ratio.
const MAX_WEIGHT_RATIO: f64 = 1e3;
/// Newton's iteration for the nearest point.
const NEWTON_STEPS: usize = 40;

/// A Bézier piece of the surface: the part of the domain it covers, and the
/// patch.
type Span = ((f64, f64), (f64, f64), RationalBezierPatch);

#[derive(Debug)]
struct Node {
    bbox: Bbox3,
    u: (f64, f64),
    v: (f64, f64),
    children: Option<[usize; 4]>,
    /// A leaf keeps its piece, to split further when a query needs a
    /// bound finer than its box, and the box of the piece in its own
    /// frame, thinner than `bbox`, by which the nearest-point search
    /// prunes.
    patch: Option<RationalBezierPatch>,
    slab: Option<Slab>,
    /// A lower bound on the smallest singular value of the Jacobian over
    /// this piece; zero when none is known.
    sigma: f64,
}

/// A surface with a tree of boxes around pieces of it.
#[derive(Debug)]
pub(super) struct PatchTree {
    surface: Surface,
    nodes: Vec<Node>,
    roots: Vec<usize>,
    /// An upper bound on the lengths of the second partial derivatives of
    /// the surface over its whole domain, in its own parameters; `None`
    /// where the nets give none.
    second: Option<f64>,
}

/// The nearest point of the surface to a point.
#[derive(Clone, Copy, Debug)]
pub(super) struct Nearest {
    pub u: f64,
    pub v: f64,
    pub point: Vec3,
    pub distance: f64,
}

pub(super) fn is_freeform(surface: &Surface) -> bool {
    matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_))
}

/// The distance from `p` to the box; zero inside it.
fn box_distance(bbox: &Bbox3, p: Vec3) -> f64 {
    let Some((lo, hi)) = bbox.corners() else {
        return f64::INFINITY;
    };
    let d = Vec3::new(
        (lo.x - p.x).max(p.x - hi.x).max(0.0),
        (lo.y - p.y).max(p.y - hi.y).max(0.0),
        (lo.z - p.z).max(p.z - hi.z).max(0.0),
    );
    d.length()
}

/// The distance from the segment `a`–`b` to the box; zero where they
/// meet. Between the parameters where a coordinate of the segment crosses
/// a side of the box, each coordinate stays on one side of it, so the
/// squared distance is one quadratic there: the least over the segment
/// is at such a parameter, at an end, or at the vertex of a quadratic.
fn segment_box_distance(bbox: &Bbox3, a: Vec3, b: Vec3) -> f64 {
    let Some((lo, hi)) = bbox.corners() else {
        return f64::INFINITY;
    };
    if !(a.is_finite() && b.is_finite()) {
        return 0.0;
    }
    let d = b - a;
    let (a3, d3) = ([a.x, a.y, a.z], [d.x, d.y, d.z]);
    let (lo3, hi3) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
    let mut cuts = vec![0.0, 1.0];
    for ((&start, &step), (&low, &high)) in a3.iter().zip(&d3).zip(lo3.iter().zip(&hi3)) {
        if step != 0.0 {
            for side in [low, high] {
                let t = (side - start) / step;
                if t > 0.0 && t < 1.0 {
                    cuts.push(t);
                }
            }
        }
    }
    cuts.sort_by(f64::total_cmp);
    let mut best = f64::INFINITY;
    for pair in cuts.windows(2) {
        let &[t0, t1] = pair else {
            continue;
        };
        let mid = 0.5 * (t0 + t1);
        // The offset of each coordinate from the box, linear in t on
        // this piece: alpha + beta t, with the side taken in the middle.
        let (mut qa, mut qb, mut qc) = (0.0_f64, 0.0_f64, 0.0_f64);
        for ((&start, &step), (&low, &high)) in a3.iter().zip(&d3).zip(lo3.iter().zip(&hi3)) {
            let x = start + step * mid;
            let (alpha, beta) = if x < low {
                (low - start, -step)
            } else if x > high {
                (start - high, step)
            } else {
                (0.0, 0.0)
            };
            qa += beta * beta;
            qb += 2.0 * alpha * beta;
            qc += alpha * alpha;
        }
        let mut candidates = vec![t0, t1];
        if qa > 0.0 {
            let vertex = -qb / (2.0 * qa);
            if vertex > t0 && vertex < t1 {
                candidates.push(vertex);
            }
        }
        for t in candidates {
            best = best.min((qa * t * t + qb * t + qc).max(0.0));
        }
    }
    best.sqrt()
}

/// A box around a piece of the surface in a frame along the piece: its
/// third axis is the piece's normal, near enough, so that the box is as
/// thick as the piece bends, not as it tilts. The box around the control
/// points in any frame holds the piece, so the distance to it is below the
/// distance to the piece, and short of it by no more than the thickness
/// and the bulge of the net's sides.
#[derive(Clone, Debug)]
struct Slab {
    origin: Vec3,
    axes: [Vec3; 3],
    local: Bbox3,
}

impl Slab {
    /// A box as it is, in the world's axes.
    fn axis_aligned(bbox: Bbox3) -> Self {
        Slab {
            origin: Vec3::ZERO,
            axes: [Vec3::X, Vec3::Y, Vec3::Z],
            local: bbox,
        }
    }

    /// The box around a piece's control points, in the frame of its
    /// corners; in the world's axes when the corners give no frame.
    fn around(patch: &RationalBezierPatch) -> Self {
        let points = patch.control_points();
        let (m, n) = patch.degrees();
        let at = |i: usize, j: usize| points.get(i * (n + 1) + j).copied();
        let frame = (|| {
            let (p00, pm0, p0n, pmn) = (at(0, 0)?, at(m, 0)?, at(0, n)?, at(m, n)?);
            let along_u = (pm0 - p00) + (pmn - p0n);
            let along_v = (p0n - p00) + (pmn - pm0);
            let first = along_u.normalized()?;
            let third = along_u.cross(along_v).normalized()?;
            Some([first, third.cross(first), third])
        })();
        let (Some(axes), Some(&origin)) = (frame, points.first()) else {
            return Slab::axis_aligned(patch.control_bbox());
        };
        let slab = Slab {
            origin,
            axes,
            local: Bbox3::EMPTY,
        };
        let local = Bbox3::from_points(points.iter().map(|&p| slab.to_local(p)));
        Slab { local, ..slab }
    }

    fn to_local(&self, p: Vec3) -> Vec3 {
        let d = p - self.origin;
        Vec3::new(
            d.dot(self.axes[0]),
            d.dot(self.axes[1]),
            d.dot(self.axes[2]),
        )
    }

    fn diagonal(&self) -> f64 {
        self.local.diagonal()
    }

    /// The extent along the piece's normal.
    fn thickness(&self) -> f64 {
        self.local
            .corners()
            .map_or(f64::INFINITY, |(lo, hi)| hi.z - lo.z)
    }
}

/// The distance from the origin to the box around some vectors; zero when
/// the box holds the origin.
fn clearance_of(vectors: &[Vec3]) -> f64 {
    box_distance(&Bbox3::from_points(vectors.iter().copied()), Vec3::ZERO)
}

/// Below this fraction of the product of the longest derivatives, the
/// length of a normal is taken for nothing.
const ROUNDING: f64 = 1e-9;

/// A lower bound on the smallest singular value of the Jacobian of a
/// patch over all of it, in the parameters of the surface it is a piece
/// of (the piece covers `width.0` by `width.1` of them), or zero when the
/// nets allow a vanishing or folding normal.
fn sigma_bound(patch: &RationalBezierPatch, width: (f64, f64)) -> f64 {
    // The largest squared lengths of the two partial derivatives and the
    // shortest normal, over the piece's own `[0, 1]²`.
    let bounds =
        if let (Some((du, dv)), Some(normal)) = (patch.derivative_nets(), patch.normal_net()) {
            let longest = |net: &[Vec3]| net.iter().map(|v| v.length_squared()).fold(0.0, f64::max);
            Some((longest(&du), longest(&dv), clearance_of(&normal)))
        } else {
            // Rational: the derivatives are bounded through the nets of the
            // weighted points and of the weights, and the normal is the
            // polynomial `w³ (∂u × ∂v)` over the cube of the weight, which is
            // at most the heaviest.
            (|| {
                let bounds = patch.derivative_bounds()?;
                let normal = patch.weighted_normal_net()?;
                let (_, heaviest) = patch.weight_range()?;
                Some((
                    bounds.first_u * bounds.first_u,
                    bounds.first_v * bounds.first_v,
                    clearance_of(&normal) / (heaviest * heaviest * heaviest),
                ))
            })()
        };
    let Some((longest_u, longest_v, normal)) = bounds else {
        return 0.0;
    };
    // A normal that short against the derivatives is rounding, not a
    // bound: where a border is collapsed into a point the net of the
    // normal is zero only up to the rounding of its products, and a box
    // around such a net may miss the origin by an ulp. (It did: an octant
    // of a sphere as a rational patch got a parameter speed of 1e17 in
    // place of a refusal.)
    if !(normal * normal > ROUNDING * ROUNDING * longest_u * longest_v) {
        return 0.0;
    }
    let (wu, wv) = width;
    if !(wu > 0.0 && wv > 0.0) {
        return 0.0;
    }
    // For the Jacobian J = [a b], the eigenvalues of JᵀJ have the sum
    // |a|² + |b|² and the product |a × b|²; the smaller one falls as the
    // sum grows and rises with the product, so the largest sum and the
    // smallest product the nets allow give a bound below it. In the
    // surface's parameters `a` and `b` are the piece's own over the
    // widths of the piece.
    let sum = longest_u / (wu * wu) + longest_v / (wv * wv);
    let product = (normal / (wu * wv)).powi(2);
    if !(sum > 0.0 && sum.is_finite() && product.is_finite()) {
        return 0.0;
    }
    // The smaller root of x² − sum x + product, written so that a product
    // far below the sum squared does not cancel.
    let least = 2.0 * product / (sum + (sum * sum - 4.0 * product).max(0.0).sqrt());
    least.max(0.0).sqrt()
}

/// An upper bound on the second partial derivatives of polynomial spans
/// over all of them, in the surface's own parameters: each span's bound
/// comes from the differences of its net over `[0, 1]²`, and is scaled by
/// the part of the domain the span covers. `None` when a span is rational
/// or degenerate.
fn second_bound(spans: &[Span]) -> Option<f64> {
    let mut bound = 0.0_f64;
    for (u, v, patch) in spans {
        let (m, n) = patch.degrees();
        let bezier = BezierPatch::new(m, n, patch.control_points()).ok()?;
        let (uu, uv, vv) = bezier.second_derivative_bounds();
        let (wu, wv) = (u.1 - u.0, v.1 - v.0);
        if !(wu > 0.0 && wv > 0.0) {
            return None;
        }
        bound = bound
            .max(uu / (wu * wu))
            .max(uv / (wu * wv))
            .max(vv / (wv * wv));
    }
    bound.is_finite().then_some(bound)
}

/// How many times a leaf may be quartered for one query.
const EXTRA_DEPTH: usize = 40;
/// How many pieces a query for a segment may split: a segment that runs
/// on the surface would otherwise split every piece along it down to the
/// resolution.
const SEGMENT_BUDGET: usize = 4096;

/// Splits the pieces on `pending` until their boxes are within
/// `resolution`, keeping only the pieces nearer than the best bound so far;
/// returns the bound.
fn refine(
    pending: &mut Vec<(RationalBezierPatch, Slab, usize)>,
    distance: &dyn Fn(&Slab) -> f64,
    resolution: f64,
    budget: usize,
    threshold: f64,
    mut best: f64,
) -> f64 {
    let mut steps = 0_usize;
    while let Some((patch, slab, depth)) = pending.pop() {
        steps += 1;
        let d = distance(&slab);
        if d >= best {
            continue;
        }
        // Small enough: within `resolution`; or, for a bound as tight as
        // can be, thin against the distance itself (a bound a little
        // short still proves a far stretch clear; splitting on would visit
        // every piece within a disc that grows as the pieces shrink). A
        // query up to a threshold splits on instead until the piece
        // passes the threshold, or is as small as the resolution.
        let size = slab.diagonal();
        let tight = !threshold.is_finite();
        if size <= resolution
            || (tight && (slab.thickness() <= 0.1 * d || size <= 0.25 * d))
            || depth >= EXTRA_DEPTH
            || steps > budget
        {
            best = best.min(d);
            // Below a finite threshold already: the asker learns nothing
            // more from a tighter bound.
            if threshold.is_finite() && best < threshold {
                return best;
            }
            continue;
        }
        let (left, right) = patch.split_u(0.5);
        let mut quarters = Vec::with_capacity(4);
        for half in [left, right] {
            let (low, high) = half.split_v(0.5);
            quarters.push(low);
            quarters.push(high);
        }
        // Nearest last: it is popped first and narrows the bound soonest.
        let mut boxed: Vec<(RationalBezierPatch, Slab, f64)> = quarters
            .into_iter()
            .map(|q| {
                let slab = Slab::around(&q);
                let d = distance(&slab);
                (q, slab, d)
            })
            .filter(|(_, _, d)| *d < best)
            .collect();
        boxed.sort_by(|a, b| b.2.total_cmp(&a.2));
        pending.extend(boxed.into_iter().map(|(q, b, _)| (q, b, depth + 1)));
    }
    best
}

/// A min-heap entry: a node and its box distance.
struct Pending(f64, usize);

impl PartialEq for Pending {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Pending {}
impl PartialOrd for Pending {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Pending {
    fn cmp(&self, o: &Self) -> Ordering {
        // Reversed, so the heap pops the nearest; ties by index.
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}

impl PatchTree {
    pub fn new(surface: &Surface) -> Result<PatchTree, Reason> {
        let spans: Vec<Span> = match surface {
            Surface::Bezier(patch) => vec![(
                (0.0, 1.0),
                (0.0, 1.0),
                RationalBezierPatch::from_bezier(patch),
            )],
            Surface::Nurbs(s) => NurbsSurface::bezier_spans(s)
                .map_err(|_| IRREGULAR)?
                .into_iter()
                .map(|span| (span.u, span.v, span.patch))
                .collect(),
            _ => return Err(IRREGULAR),
        };
        if spans.is_empty() {
            return Err(IRREGULAR);
        }
        let polynomial = spans.iter().all(|s| s.2.is_polynomial());
        if !polynomial {
            for (_, _, patch) in &spans {
                let (lightest, heaviest) = patch.weight_range().ok_or(IRREGULAR)?;
                if !(heaviest <= MAX_WEIGHT_RATIO * lightest) {
                    return Err(WEIGHTS);
                }
            }
        }
        let second = polynomial.then(|| second_bound(&spans)).flatten();
        // Quarter each span until its pieces are small, within a budget
        // of leaves for the whole surface.
        let mut depth = MAX_DEPTH;
        while depth > 0 && spans.len().saturating_mul(1 << (2 * depth)) > MAX_LEAVES {
            depth -= 1;
        }
        if spans.len() > MAX_LEAVES {
            return Err(TOO_MANY);
        }
        let mut tree = PatchTree {
            surface: surface.clone(),
            nodes: Vec::new(),
            roots: Vec::new(),
            second,
        };
        let whole = spans
            .iter()
            .map(|s| s.2.control_bbox().diagonal())
            .fold(0.0, f64::max);
        for (u, v, patch) in &spans {
            let root = tree.subdivide(patch, *u, *v, depth, whole / 32.0);
            tree.roots.push(root);
        }
        if !polynomial {
            tree.second = tree.second_of_leaves();
        }
        Ok(tree)
    }

    fn subdivide(
        &mut self,
        patch: &RationalBezierPatch,
        u: (f64, f64),
        v: (f64, f64),
        depth: usize,
        small: f64,
    ) -> usize {
        let bbox = patch.control_bbox();
        let sigma = sigma_bound(patch, (u.1 - u.0, v.1 - v.0));
        let children = if depth > 0 && bbox.diagonal() > small {
            let um = 0.5 * (u.0 + u.1);
            let vm = 0.5 * (v.0 + v.1);
            let (left, right) = patch.split_u(0.5);
            let (ll, lh) = left.split_v(0.5);
            let (rl, rh) = right.split_v(0.5);
            let next = depth - 1;
            Some([
                self.subdivide(&ll, (u.0, um), (v.0, vm), next, small),
                self.subdivide(&lh, (u.0, um), (vm, v.1), next, small),
                self.subdivide(&rl, (um, u.1), (v.0, vm), next, small),
                self.subdivide(&rh, (um, u.1), (vm, v.1), next, small),
            ])
        } else {
            None
        };
        // A piece's own bound is at least as good as its parent's; the
        // parent takes the least of its children's, which is tighter.
        let sigma = match children {
            Some(kids) => kids
                .iter()
                .filter_map(|&k| self.nodes.get(k).map(|n| n.sigma))
                .fold(f64::INFINITY, f64::min)
                .max(sigma),
            None => sigma,
        };
        let slab = children.is_none().then(|| Slab::around(patch));
        let patch = children.is_none().then(|| patch.clone());
        self.nodes.push(Node {
            bbox,
            u,
            v,
            children,
            patch,
            slab,
            sigma,
        });
        self.nodes.len() - 1
    }

    /// An upper bound on the second partial derivatives of a rational
    /// surface over its whole domain, in its own parameters: the largest
    /// over the leaves, which cover the domain, each scaled by the part of
    /// the domain it covers (a rational piece's bound tightens as it
    /// shrinks). `None` when a leaf has no bound.
    fn second_of_leaves(&self) -> Option<f64> {
        let mut bound = 0.0_f64;
        for node in &self.nodes {
            let Some(patch) = &node.patch else {
                continue;
            };
            let (wu, wv) = (node.u.1 - node.u.0, node.v.1 - node.v.0);
            if !(wu > 0.0 && wv > 0.0) {
                return None;
            }
            let b = patch.derivative_bounds()?.over(wu, wv);
            bound = bound.max(b.second_uu).max(b.second_uv).max(b.second_vv);
        }
        bound.is_finite().then_some(bound)
    }

    /// How far the surface's parameters can change per unit of length
    /// travelled on it, at most; `None` when the surface may stall or fold
    /// (then its chart cannot be trusted).
    pub fn parameter_speed(&self) -> Option<f64> {
        let sigma = self
            .roots
            .iter()
            .filter_map(|&r| self.nodes.get(r))
            .map(|n| n.sigma)
            .fold(f64::INFINITY, f64::min);
        (sigma.is_finite() && sigma > 0.0).then(|| 1.0 / sigma)
    }

    /// A distance no larger than the distance from `p` to the surface, and
    /// within `resolution` or a quarter of it: the leaves near `p` are
    /// split further, on the spot, until their boxes are that small (a box
    /// that small is as far from `p` as the piece in it, to within its
    /// size). Nearest
    /// first, so that the bound is tight after the first leaf and every
    /// farther piece is dropped unsplit. The splits are not kept. The
    /// bound is capped at `threshold` (see [`Self::segment_lower_bound`]).
    pub fn lower_bound(&self, p: Vec3, resolution: f64, threshold: f64) -> f64 {
        self.bound_by(
            &|slab| box_distance(&slab.local, slab.to_local(p)),
            resolution,
            MAX_LEAVES,
            threshold,
        )
    }

    /// A distance no larger than the distance from any point of the
    /// straight segment `a`–`b` to the surface, as [`Self::lower_bound`]
    /// is for a point, within a budget of pieces split for it: past the
    /// budget the bound is what the pieces so far allow, which is still
    /// below the distance, only less tight. It is what proves a whole
    /// stretch of a curve clear of the surface at once, where a point and
    /// how far the curve can travel from it would need pieces the size
    /// of the tolerance.
    pub fn segment_lower_bound(&self, a: Vec3, b: Vec3, resolution: f64, threshold: f64) -> f64 {
        self.bound_by(
            &|slab| segment_box_distance(&slab.local, slab.to_local(a), slab.to_local(b)),
            resolution,
            SEGMENT_BUDGET,
            threshold,
        )
    }

    /// The bound, no larger than `threshold`: a piece farther than that
    /// is dropped unsplit, so that a query that only asks whether the
    /// distance exceeds the threshold (the answer is then the threshold
    /// itself) splits nothing beyond it. Pass infinity for the bound as
    /// tight as the resolution allows.
    fn bound_by(
        &self,
        distance: &dyn Fn(&Slab) -> f64,
        resolution: f64,
        budget: usize,
        threshold: f64,
    ) -> f64 {
        let resolution = resolution.max(0.0);
        let threshold = if threshold.is_nan() {
            f64::INFINITY
        } else {
            threshold
        };
        let mut best = threshold;
        let mut heap = BinaryHeap::new();
        for &root in &self.roots {
            if let Some(node) = self.nodes.get(root) {
                heap.push(Pending(distance(&Slab::axis_aligned(node.bbox)), root));
            }
        }
        let mut pending: Vec<(RationalBezierPatch, Slab, usize)> = Vec::new();
        while let Some(Pending(d, index)) = heap.pop() {
            if d >= best {
                break;
            }
            let Some(node) = self.nodes.get(index) else {
                continue;
            };
            match node.children {
                Some(kids) => {
                    for k in kids {
                        if let Some(child) = self.nodes.get(k) {
                            heap.push(Pending(distance(&Slab::axis_aligned(child.bbox)), k));
                        }
                    }
                }
                None => match &node.patch {
                    Some(patch) => {
                        // The leaf's own box is in its frame, thinner than
                        // the box of the tree that found it.
                        let slab = Slab::around(patch);
                        if node.bbox.diagonal() > resolution {
                            pending.push((patch.clone(), slab, 0));
                            best =
                                refine(&mut pending, distance, resolution, budget, threshold, best);
                        } else {
                            best = best.min(distance(&slab));
                        }
                    }
                    None => best = best.min(d),
                },
            }
            if threshold.is_finite() && best < threshold {
                break;
            }
        }
        best
    }

    fn clamp(&self, u: f64, v: f64) -> (f64, f64) {
        let ((u0, u1), (v0, v1)) = self.surface.domain();
        (u.clamp(u0, u1), v.clamp(v0, v1))
    }

    /// Newton's iteration with damping from `(u, v)` towards the point of
    /// the surface nearest to `p`, within the domain: a step that would
    /// leave the domain through a side is taken along that side instead.
    /// The second derivatives come from differences of the first, so that
    /// the iteration converges quadratically also far from the surface,
    /// where Gauss–Newton alone crawls.
    fn descend(&self, p: Vec3, start: (f64, f64)) -> Nearest {
        let ((u0, u1), (v0, v1)) = self.surface.domain();
        let (mut u, mut v) = self.clamp(start.0, start.1);
        let (mut point, _, _) = self.surface.derivatives(u, v);
        let mut distance = point.distance(p);
        let mut damping = 1e-6;
        let (hu, hv) = (1e-6 * (u1 - u0), 1e-6 * (v1 - v0));
        // Whether the curvature terms are taken from the start of a step.
        let mut curved = false;
        for _ in 0..NEWTON_STEPS {
            let (s, su, sv) = self.surface.derivatives(u, v);
            let r = s - p;
            let (g1, g2) = (su.dot(r), sv.dot(r));
            if !(g1.hypot(g2) > 0.0) {
                break;
            }
            // Descent is held to the sides of the domain it stands on.
            let held_u = (u <= u0 && g1 > 0.0) || (u >= u1 && g1 < 0.0);
            let held_v = (v <= v0 && g2 > 0.0) || (v >= v1 && g2 < 0.0);
            if held_u && held_v {
                break;
            }
            // The Hessian of half the squared distance: first-order terms
            // (Gauss–Newton, enough near the surface, where the residual
            // and with it the curvature terms vanish) plus, when a step
            // from those alone is refused or gains little, the curvature
            // terms from differences of the first derivatives, which
            // matter when `r` is long.
            let gauss = (su.dot(su), su.dot(sv), sv.dot(sv));
            let curvature = |full: &mut bool| {
                *full = true;
                let (_, su_up, sv_up) = self.surface.derivatives((u + hu).min(u1), v);
                let (_, su_down, sv_down) = self.surface.derivatives((u - hu).max(u0), v);
                let (_, _, sv_vup) = self.surface.derivatives(u, (v + hv).min(v1));
                let (_, _, sv_vdown) = self.surface.derivatives(u, (v - hv).max(v0));
                let suu = (su_up - su_down) * (0.5 / hu);
                let suv = (sv_up - sv_down) * (0.5 / hu);
                let svv = (sv_vup - sv_vdown) * (0.5 / hv);
                (
                    gauss.0 + suu.dot(r),
                    gauss.1 + suv.dot(r),
                    gauss.2 + svv.dot(r),
                )
            };
            let mut full = curved;
            let (mut a, mut b, mut c) = if full { curvature(&mut full) } else { gauss };
            let scale = su.dot(su).max(sv.dot(sv)).max(f64::MIN_POSITIVE);
            let mut moved = false;
            for _ in 0..16 {
                // Levenberg–Marquardt: the damping adds to the diagonal in
                // units of the Jacobian's own size, so the step turns from
                // Newton's towards steepest descent when Newton's fails.
                let (aa, cc) = (a + damping * scale, c + damping * scale);
                let (du, dv) = if held_u {
                    (0.0, if cc > 0.0 { -g2 / cc } else { 0.0 })
                } else if held_v {
                    (if aa > 0.0 { -g1 / aa } else { 0.0 }, 0.0)
                } else {
                    let det = aa * cc - b * b;
                    if !(det > 0.0 && aa > 0.0) {
                        if !full {
                            (a, b, c) = curvature(&mut full);
                        } else {
                            damping = (damping * 10.0).max(1e-9);
                        }
                        continue;
                    }
                    (-(cc * g1 - b * g2) / det, -(aa * g2 - b * g1) / det)
                };
                let (nu, nv) = self.clamp(u + du, v + dv);
                let candidate = self.surface.eval(nu, nv);
                let d = candidate.distance(p);
                if d < distance || (d == distance && (nu != u || nv != v) && damping < 1e-3) {
                    let step = (nu - u).hypot(nv - v);
                    // A step from the first-order terms that gains little
                    // is a far point converging slowly: the curvature
                    // terms from the next step on.
                    curved = full || d > 0.25 * distance;
                    u = nu;
                    v = nv;
                    point = candidate;
                    distance = d;
                    damping = (damping * 0.3).max(1e-12);
                    moved = step > 0.0;
                    break;
                }
                if !full {
                    (a, b, c) = curvature(&mut full);
                } else {
                    damping *= 10.0;
                }
            }
            if !moved {
                break;
            }
        }
        Nearest {
            u,
            v,
            point,
            distance,
        }
    }

    /// The point of the surface nearest to `p`.
    pub fn nearest(&self, p: Vec3) -> Option<Nearest> {
        if !p.is_finite() {
            return None;
        }
        let mut heap = BinaryHeap::new();
        for &root in &self.roots {
            if let Some(node) = self.nodes.get(root) {
                heap.push(Pending(box_distance(&node.bbox, p), root));
            }
        }
        // (A root that is itself a leaf is queued by its world box, which
        // is below its slab's distance: safe, only looser.)
        let mut best: Option<Nearest> = None;
        let mut visited = 0_usize;
        // A leaf is queued by the distance to the box in its own frame,
        // which is short of the distance to the piece by its bend only:
        // far from the surface, the pieces whose boxes in the world's axes
        // reach nearer than the nearest point found form a ring that
        // would all be descended from.
        let key = |node: &Node| match &node.slab {
            Some(slab) => box_distance(&slab.local, slab.to_local(p)),
            None => box_distance(&node.bbox, p),
        };
        while let Some(Pending(d, index)) = heap.pop() {
            if best.is_some_and(|b| d >= b.distance) {
                break;
            }
            let Some(node) = self.nodes.get(index) else {
                continue;
            };
            match node.children {
                Some(kids) => {
                    for k in kids {
                        if let Some(child) = self.nodes.get(k) {
                            heap.push(Pending(key(child), k));
                        }
                    }
                }
                None => {
                    visited += 1;
                    if visited > MAX_LEAVES {
                        break;
                    }
                    let start = (0.5 * (node.u.0 + node.u.1), 0.5 * (node.v.0 + node.v.1));
                    let found = self.descend(p, start);
                    if found.distance.is_finite()
                        && best.is_none_or(|b| found.distance < b.distance)
                    {
                        best = Some(found);
                    }
                }
            }
        }
        best
    }

    /// The signed distance from `p` to the surface: positive on the side
    /// its normal points to. `None` where the surface has no normal at
    /// the nearest point.
    pub fn signed_distance(&self, p: Vec3) -> Option<f64> {
        let near = self.nearest(p)?;
        let normal = self.surface.normal(near.u, near.v)?;
        let side = (p - near.point).dot(normal);
        Some(if side < 0.0 {
            -near.distance
        } else {
            near.distance
        })
    }

    /// Whether every point of a stretch of a curve from `pa` to `pb` is
    /// within `tol` of the surface: proven, not sampled. The stretch is
    /// compared with the surface along the straight path in the
    /// parameters between the feet of its ends; their difference is
    /// small at both ends (the distances there) and bends by no more than
    /// `turn` (the curve's second derivative over the stretch, with the
    /// stretch as the unit of parameter) plus what the second derivatives
    /// of the surface allow over the path, so it is within an eighth of
    /// that of the larger end. `false` where that cannot be shown, which
    /// is not a disproof.
    pub fn stretch_within(&self, pa: Vec3, pb: Vec3, turn: f64, tol: f64) -> bool {
        let Some(second) = self.second else {
            return false;
        };
        if !(turn >= 0.0 && tol > 0.0) {
            return false;
        }
        let (Some(a), Some(b)) = (self.nearest(pa), self.nearest(pb)) else {
            return false;
        };
        let ends = a.distance.max(b.distance);
        if !(ends <= tol) {
            return false;
        }
        let step = (b.u - a.u).abs() + (b.v - a.v).abs();
        let bend = turn + second * step * step;
        bend.is_finite() && ends + 0.125 * bend <= tol
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_geom::BezierPatch;
    use craft3d_math::Frame;

    fn dome() -> Surface {
        let roof = craft3d_testkit::trimmed::domed_block(&Frame::WORLD, 4.0, 3.0, 2.0, false)
            .unwrap()
            .body;
        roof.faces
            .iter()
            .map(|f| f.surface.clone())
            .find(|s| matches!(s, Surface::Bezier(_)))
            .unwrap()
    }

    fn brute_force(surface: &Surface, p: Vec3) -> (f64, (f64, f64)) {
        let ((u0, u1), (v0, v1)) = surface.domain();
        let mut best = (f64::INFINITY, (0.0, 0.0));
        for i in 0..=400 {
            for j in 0..=400 {
                let u = u0 + (u1 - u0) * i as f64 / 400.0;
                let v = v0 + (v1 - v0) * j as f64 / 400.0;
                let d = surface.eval(u, v).distance(p);
                if d < best.0 {
                    best = (d, (u, v));
                }
            }
        }
        best
    }

    #[test]
    fn the_nearest_point_of_a_dome_and_a_bound_below_it() {
        let surface = dome();
        let tree = PatchTree::new(&surface).unwrap();
        let mut seed = 7_u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..200 {
            let p = Vec3::new(next() * 6.0 - 1.0, next() * 5.0 - 1.0, next() * 6.0 - 1.0);
            let (brute, _) = brute_force(&surface, p);
            let lower = tree.lower_bound(p, 1e-9, f64::INFINITY);
            let near = tree.nearest(p).unwrap();
            assert!(lower <= brute + 1e-12, "{p:?}: bound {lower} above {brute}");
            assert!(lower <= near.distance + 1e-12, "{p:?}");
            // The grid is a hundredth of the patch wide: Newton must beat it.
            assert!(
                near.distance <= brute + 1e-9,
                "{p:?}: nearest {} but the grid finds {brute}",
                near.distance
            );
            assert!(near.point.distance(surface.eval(near.u, near.v)) < 1e-12);
        }
    }

    #[test]
    fn points_of_the_surface_are_at_distance_zero_with_their_parameters() {
        let surface = dome();
        let tree = PatchTree::new(&surface).unwrap();
        for (u, v) in [
            (0.0, 0.0),
            (0.37, 0.81),
            (0.5, 0.5),
            (1.0, 0.25),
            (0.99, 0.99),
        ] {
            let p = surface.eval(u, v);
            let near = tree.nearest(p).unwrap();
            assert!(near.distance < 1e-10, "({u}, {v}): {}", near.distance);
            assert!(
                (near.u - u).abs() < 1e-7 && (near.v - v).abs() < 1e-7,
                "({u}, {v}) found as ({}, {})",
                near.u,
                near.v
            );
            assert!(tree.signed_distance(p).unwrap().abs() < 1e-10);
        }
    }

    #[test]
    fn the_sign_tells_above_from_below_the_roof() {
        let surface = dome();
        let tree = PatchTree::new(&surface).unwrap();
        let top = surface.eval(0.4, 0.6);
        // The dome's normal points up (u along x, v along y).
        assert!(
            tree.signed_distance(top + Vec3::new(0.0, 0.0, 0.5))
                .unwrap()
                > 0.4
        );
        assert!(
            tree.signed_distance(top - Vec3::new(0.0, 0.0, 0.5))
                .unwrap()
                < -0.4
        );
        // Far below, the nearest point is still on the roof and the sign
        // still negative.
        assert!(tree.signed_distance(Vec3::new(2.0, 1.5, -10.0)).unwrap() < -11.0);
    }

    #[test]
    fn a_flat_patch_has_the_parameter_speed_of_its_sides() {
        let flat = BezierPatch::new(
            1,
            1,
            vec![
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, 3.0, 1.0),
                Vec3::new(4.0, 0.0, 1.0),
                Vec3::new(4.0, 3.0, 1.0),
            ],
        )
        .unwrap();
        let tree = PatchTree::new(&Surface::Bezier(flat)).unwrap();
        // u moves 1 per 4 units of x, v 1 per 3 of y: the faster is v, and
        // on a flat patch the bound is exact.
        let speed = tree.parameter_speed().unwrap();
        assert!((speed - 1.0 / 3.0).abs() < 1e-9, "{speed}");
        let dome = PatchTree::new(&dome()).unwrap();
        let speed = dome.parameter_speed().unwrap();
        assert!(speed > 1.0 / 3.0 && speed < 1.0, "{speed}");
    }

    #[test]
    fn the_teapot_patches_are_indexed_and_their_nearest_points_found() {
        let patches = craft3d_testkit::teapot::teapot().unwrap();
        let mut folded = 0;
        for patch in &patches {
            let surface = Surface::Bezier(patch.clone());
            let tree = PatchTree::new(&surface).unwrap();
            if tree.parameter_speed().is_none() {
                folded += 1;
            }
            for (u, v) in [(0.2, 0.3), (0.7, 0.9)] {
                let p = surface.eval(u, v) + Vec3::new(0.05, -0.02, 0.03);
                let (brute, _) = brute_force(&surface, p);
                let near = tree.nearest(p).unwrap();
                assert!(
                    near.distance <= brute + 1e-6,
                    "{} vs {brute}",
                    near.distance
                );
                assert!(tree.lower_bound(p, 1e-9, f64::INFINITY) <= near.distance + 1e-12);
            }
        }
        // The lid's top and the bottom patches collapse to a point: no
        // chart for those, the rest are regular.
        assert!(folded < patches.len() / 2, "{folded} of {}", patches.len());
    }

    /// A cylinder of radius 1 and height 2 as a rational surface: the
    /// circle swept along z. u goes once around over `[0, 1]`, v up.
    fn rational_cylinder() -> Surface {
        let circle = craft3d_geom::NurbsCurve::circle(&Frame::WORLD, 1.0).unwrap();
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for &p in circle.control_points() {
            ctrl.push(p);
            ctrl.push(p + Vec3::new(0.0, 0.0, 2.0));
        }
        for &w in circle.weights() {
            weights.push(w);
            weights.push(w);
        }
        Surface::Nurbs(
            NurbsSurface::new(
                2,
                1,
                circle.knots().to_vec(),
                vec![0.0, 0.0, 1.0, 1.0],
                9,
                2,
                ctrl,
                weights,
            )
            .unwrap(),
        )
    }

    #[test]
    fn a_rational_surface_is_indexed_and_has_a_parameter_speed() {
        // (Until rational faces came in, a rational surface had no bound
        // on its parameter speed and its chart was refused.)
        let surface = rational_cylinder();
        let tree = PatchTree::new(&surface).unwrap();
        let d = tree.signed_distance(Vec3::new(3.0, 0.0, 1.0)).unwrap();
        assert!((d.abs() - 2.0).abs() < 1e-9, "{d}");
        assert!(tree.lower_bound(Vec3::new(3.0, 0.0, 1.0), 1e-9, f64::INFINITY) <= 2.0);
        // v moves 1 per 2 of height; u moves fastest at the knots, where a
        // quarter of the circle starts at √2 per unit of its own
        // parameter, a quarter of u: 1 per 4√2. The slower direction of
        // the surface is v, so the parameters move at most 1/2 per
        // length, and the bound is not far above that.
        let speed = tree.parameter_speed().unwrap();
        let mut fastest = 0.0_f64;
        for i in 0..=400 {
            let (_, su, sv) = surface.derivatives(f64::from(i) / 400.0, 0.3);
            // The directions are square to each other on a cylinder.
            assert!(su.dot(sv).abs() < 1e-9);
            fastest = fastest.max(1.0 / su.length()).max(1.0 / sv.length());
        }
        assert!((fastest - 0.5).abs() < 1e-12, "{fastest}");
        assert!(speed >= fastest, "{speed} below {fastest}");
        assert!(speed <= 1.2 * fastest, "{speed} far above {fastest}");
    }

    #[test]
    fn the_second_derivatives_of_a_rational_surface_are_bounded() {
        let surface = rational_cylinder();
        let tree = PatchTree::new(&surface).unwrap();
        let bound = tree.second.unwrap();
        let h = 1e-6;
        let mut steepest = 0.0_f64;
        for i in 0..400 {
            // Between the knots, which are at the quarters.
            let u = (f64::from(i) + 0.5) / 400.0;
            let (_, up, _) = surface.derivatives(u + h, 0.5);
            let (_, down, _) = surface.derivatives(u - h, 0.5);
            steepest = steepest.max((up - down).length() / (2.0 * h));
        }
        assert!(steepest <= bound, "{steepest} above the bound {bound}");
        assert!(bound <= 1.5 * steepest, "{bound} far above {steepest}");
        // A chord of a curve on it is proven within the tolerance when
        // short, as on a polynomial surface, and what is proven is true.
        let tol = 1e-4;
        let from = surface.eval(0.1, 0.4);
        let step = (2.0 * tol / bound).sqrt();
        let near = surface.eval(0.1 + step, 0.4);
        assert!(tree.stretch_within(from, near, 0.0, tol));
        let middle = (from + near) * 0.5;
        assert!(tree.nearest(middle).unwrap().distance <= tol);
        let far = surface.eval(0.2, 0.4);
        assert!(!tree.stretch_within(from, far, 0.0, tol));
        assert!(tree.nearest((from + far) * 0.5).unwrap().distance > tol);
    }

    #[test]
    fn the_parameter_speed_is_in_the_parameters_of_the_surface() {
        // The same flat patch, 4 by 3, over [0, 1]² and as a B-spline
        // over [0, 10] × [0, 5]: there u moves 10 per 4 and v 5 per 3.
        // (The bound was the piece's own before, right only for a span
        // one wide: on this surface it said 1/3, too slow by a factor 5,
        // and a chart followed by it could step over a turn.)
        let points = vec![
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 3.0, 1.0),
            Vec3::new(4.0, 0.0, 1.0),
            Vec3::new(4.0, 3.0, 1.0),
        ];
        let wide = NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, 10.0, 10.0],
            vec![0.0, 0.0, 5.0, 5.0],
            2,
            2,
            points.clone(),
            vec![1.0; 4],
        )
        .unwrap();
        let tree = PatchTree::new(&Surface::Nurbs(wide)).unwrap();
        let speed = tree.parameter_speed().unwrap();
        assert!((speed - 2.5).abs() < 1e-9, "{speed}");
        // With weights that are not all one the patch is the same piece
        // of the plane, reparametrised: its parameters crowd towards the
        // heavy corners, by no more than the weights differ.
        let weighted = NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            2,
            points,
            vec![1.0, 2.0, 2.0, 1.0],
        )
        .unwrap();
        let surface = Surface::Nurbs(weighted);
        let tree = PatchTree::new(&surface).unwrap();
        let speed = tree.parameter_speed().unwrap();
        let mut fastest = 0.0_f64;
        for i in 0..=100 {
            for j in 0..=100 {
                let (_, su, sv) = surface.derivatives(f64::from(i) / 100.0, f64::from(j) / 100.0);
                // The smallest singular value of [su sv].
                let (sum, product) = (
                    su.length_squared() + sv.length_squared(),
                    su.cross(sv).length_squared(),
                );
                let least = 0.5 * (sum - (sum * sum - 4.0 * product).max(0.0).sqrt());
                fastest = fastest.max(1.0 / least.sqrt());
            }
        }
        assert!(speed >= fastest, "{speed} below {fastest}");
        assert!(speed <= 3.0 * fastest, "{speed} far above {fastest}");
    }

    #[test]
    fn the_bounds_of_a_weighted_roof_hold_and_are_not_far_off() {
        // The bicubic roof of the domed block with the weights 1, 2, 0.5
        // and 3: no quadric, and its parameters crowd where it is heavy.
        let roof = craft3d_testkit::trimmed::domed_block(&Frame::WORLD, 4.0, 3.0, 2.0, true)
            .unwrap()
            .body
            .faces
            .iter()
            .find_map(|f| match &f.surface {
                Surface::Nurbs(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap();
        let weights: Vec<f64> = (0..16)
            .map(|n| [1.0, 2.0, 0.5, 3.0][(n / 4 + 3 * (n % 4) + (n / 4) * (n % 4)) % 4])
            .collect();
        let surface = Surface::Nurbs(
            NurbsSurface::new(
                3,
                3,
                roof.knots_u().to_vec(),
                roof.knots_v().to_vec(),
                4,
                4,
                roof.control_points().to_vec(),
                weights,
            )
            .unwrap(),
        );
        let tree = PatchTree::new(&surface).unwrap();
        let (mut fastest, mut steepest) = (0.0_f64, 0.0_f64);
        let h = 1e-6;
        for i in 0..=200 {
            for j in 0..=200 {
                let (u, v) = (f64::from(i) / 200.0, f64::from(j) / 200.0);
                let (_, su, sv) = surface.derivatives(u, v);
                let (sum, product) = (
                    su.length_squared() + sv.length_squared(),
                    su.cross(sv).length_squared(),
                );
                let least = 2.0 * product / (sum + (sum * sum - 4.0 * product).max(0.0).sqrt());
                fastest = fastest.max(1.0 / least.sqrt());
                let (_, au, av) = surface.derivatives((u + h).min(1.0), v);
                let (_, bu, bv) = surface.derivatives((u - h).max(0.0), v);
                let (_, _, cv) = surface.derivatives(u, (v + h).min(1.0));
                let (_, _, dv) = surface.derivatives(u, (v - h).max(0.0));
                // (At a border the difference is one-sided: half as wide.)
                let wide = |x: f64| (x + h).min(1.0) - (x - h).max(0.0);
                steepest = steepest
                    .max((au - bu).length() / wide(u))
                    .max((av - bv).length() / wide(u))
                    .max((cv - dv).length() / wide(v));
            }
        }
        // As measured: 1.84 against 1.35, and 208 against 168.
        let speed = tree.parameter_speed().unwrap();
        assert!(speed >= fastest, "{speed} below {fastest}");
        assert!(speed <= 1.5 * fastest, "{speed} far above {fastest}");
        let second = tree.second.unwrap();
        assert!(
            second >= steepest * (1.0 - 1e-6),
            "{second} below {steepest}"
        );
        assert!(second <= 1.4 * steepest, "{second} far above {steepest}");
    }

    #[test]
    fn a_rational_patch_with_a_collapsed_border_has_no_parameter_speed() {
        // A quarter of a cone up to its apex: the border at the apex is
        // one point, and the parameter around the axis stalls there. Its
        // normal net is zero at that border only up to rounding, which
        // must not pass for a bound.
        let s = std::f64::consts::FRAC_1_SQRT_2;
        // Collapsed exactly, and as a quarter turn's cosine leaves it:
        // 6e-17 from the axis.
        let cases = [
            (false, 0.0),
            (true, 0.0),
            (false, std::f64::consts::FRAC_PI_2.cos()),
        ];
        for (tilted, top) in cases {
            let frame = if tilted {
                Frame::new(
                    Vec3::new(1.0, -2.0, 0.5),
                    Vec3::new(0.3, -0.4, 1.0),
                    Vec3::new(1.0, 0.2, 0.0),
                )
                .unwrap()
            } else {
                Frame::WORLD
            };
            let mut ctrl = Vec::new();
            let mut weights = Vec::new();
            for (x, y, w) in [(1.0, 0.0, 1.0), (1.0, 1.0, s), (0.0, 1.0, 1.0)] {
                ctrl.push(frame.point(x, y, 0.0));
                ctrl.push(frame.point(top * x, top * y, 1.5));
                weights.extend([w, w]);
            }
            let cone = NurbsSurface::new(
                2,
                1,
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                3,
                2,
                ctrl,
                weights,
            )
            .unwrap();
            let tree = PatchTree::new(&Surface::Nurbs(cone)).unwrap();
            assert_eq!(tree.parameter_speed(), None, "tilted {tilted}, top {top}");
        }
    }

    #[test]
    fn weights_too_far_apart_are_refused_with_the_reason() {
        let surface = |heavy: f64| {
            Surface::Nurbs(
                NurbsSurface::new(
                    1,
                    1,
                    vec![0.0, 0.0, 1.0, 1.0],
                    vec![0.0, 0.0, 1.0, 1.0],
                    2,
                    2,
                    vec![
                        Vec3::new(0.0, 0.0, 1.0),
                        Vec3::new(0.0, 3.0, 1.0),
                        Vec3::new(4.0, 0.0, 1.0),
                        Vec3::new(4.0, 3.0, 1.0),
                    ],
                    vec![1.0, heavy, 1.0, 1.0],
                )
                .unwrap(),
            )
        };
        assert!(PatchTree::new(&surface(900.0)).is_ok());
        assert_eq!(PatchTree::new(&surface(1100.0)).unwrap_err(), WEIGHTS);
        assert_eq!(PatchTree::new(&surface(1.0 / 1100.0)).unwrap_err(), WEIGHTS);
    }

    #[test]
    fn a_short_chord_of_the_dome_is_proven_within_tolerance_and_a_long_one_is_not() {
        let surface = dome();
        let tree = PatchTree::new(&surface).unwrap();
        let second = tree.second.unwrap();
        assert!(second > 0.0, "the dome bends");
        let tol = 1e-4;
        let from = surface.eval(0.3, 0.4);
        // Short: a step over which the surface's bend stays at half the
        // tolerance; the proof holds, and what it proves is true.
        let step = (4.0 * tol / second).sqrt().min(0.1);
        let near = surface.eval(0.3 + step, 0.4);
        assert!(
            tree.stretch_within(from, near, 0.0, tol),
            "step {step}, second {second}"
        );
        for k in 0..=16 {
            let p = from + (near - from) * (f64::from(k) / 16.0);
            let distance = tree.nearest(p).unwrap().distance;
            assert!(distance <= tol, "{distance} at {k}");
        }
        // Long: the chord really leaves the tolerance, and the proof
        // does not pretend otherwise.
        let far = surface.eval(0.8, 0.4);
        assert!(!tree.stretch_within(from, far, 0.0, tol));
        let middle = (from + far) * 0.5;
        assert!(tree.nearest(middle).unwrap().distance > tol);
        // An end off the surface: nothing to prove.
        let lifted = near + Vec3::new(0.0, 0.0, 10.0 * tol);
        assert!(!tree.stretch_within(from, lifted, 0.0, tol));
        // A curve that bends more than the tolerance allows over the
        // stretch is not proven either.
        assert!(!tree.stretch_within(from, near, 9.0 * tol, tol));
        // Garbage in, no proof out.
        assert!(!tree.stretch_within(from, near, f64::NAN, tol));
        assert!(!tree.stretch_within(from, near, 0.0, 0.0));
    }

    #[test]
    fn the_distance_from_a_segment_to_a_box_is_the_least_over_the_segment() {
        let bbox = Bbox3::from_points([Vec3::new(-1.0, -2.0, 0.0), Vec3::new(1.0, 2.0, 3.0)]);
        let segments = [
            (Vec3::new(-5.0, 0.0, 1.0), Vec3::new(5.0, 0.0, 1.0)),
            (Vec3::new(2.0, 3.0, 4.0), Vec3::new(6.0, 7.0, 8.0)),
            (Vec3::new(-3.0, -3.0, -1.0), Vec3::new(3.0, 3.0, -1.0)),
            (Vec3::new(2.0, -5.0, 1.5), Vec3::new(2.0, 5.0, 1.5)),
            (Vec3::new(0.2, 0.3, 1.0), Vec3::new(0.4, 0.1, 2.0)),
            (Vec3::new(-4.0, 5.0, -2.0), Vec3::new(4.0, -5.0, 7.0)),
        ];
        for (a, b) in segments {
            let exact = segment_box_distance(&bbox, a, b);
            let n = 100_000;
            let sampled = (0..=n)
                .map(|k| box_distance(&bbox, a + (b - a) * (k as f64 / n as f64)))
                .fold(f64::INFINITY, f64::min);
            // Sampling never finds a point nearer than the least, and
            // misses it by no more than a sample step.
            let step = (b - a).length() / n as f64;
            assert!(
                exact <= sampled + 1e-12,
                "{exact} > {sampled} for {a:?}-{b:?}"
            );
            assert!(
                exact >= sampled - step,
                "{exact} < {sampled} - {step} for {a:?}-{b:?}"
            );
        }
        assert_eq!(segment_box_distance(&bbox, Vec3::ZERO, Vec3::ZERO), 0.0);
        assert_eq!(
            segment_box_distance(&bbox, Vec3::new(f64::NAN, 0.0, 0.0), Vec3::ZERO),
            0.0
        );
    }

    #[test]
    fn a_segment_grazing_the_dome_is_bounded_below_at_once() {
        let surface = dome();
        let tree = PatchTree::new(&surface).unwrap();
        let tol = 1e-6;
        // A chord of a curve on the roof, lifted a little: nearer than
        // the lift nowhere, and the bound says so to within the
        // resolution (the lift itself is the distance at the ends).
        let (from, to) = (surface.eval(0.3, 0.4), surface.eval(0.32, 0.4));
        let normal = surface.normal(0.31, 0.4).unwrap();
        let lift = 20.0 * tol;
        let bound = tree.segment_lower_bound(
            from + normal * lift,
            to + normal * lift,
            0.25 * tol,
            f64::INFINITY,
        );
        // Asked only whether it exceeds a threshold, the answer is the
        // threshold, after far fewer splits.
        let capped = tree.segment_lower_bound(
            from + normal * lift,
            to + normal * lift,
            0.25 * tol,
            2.0 * tol,
        );
        assert_eq!(capped, 2.0 * tol);
        assert!(bound > 2.0 * tol, "{bound}");
        let least = (0..=64)
            .map(|k| {
                let p = (from + normal * lift) + (to - from) * (f64::from(k) / 64.0);
                tree.nearest(p).unwrap().distance
            })
            .fold(f64::INFINITY, f64::min);
        assert!(bound <= least + 1e-12, "{bound} > {least}");
        // A chord through the roof: no distance to bound.
        let through = tree.segment_lower_bound(from - normal, to + normal, 0.25 * tol, 2.0 * tol);
        assert!(through <= 0.25 * tol, "{through}");
    }

    #[test]
    fn a_tilted_flat_patch_is_bounded_tightly_however_coarse_the_resolution() {
        // A flat square tilted out of every axis: its axis-aligned box is
        // thick, the box in its own frame is not.
        let (o, a, b) = (
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(3.0, 1.0, 2.0),
            Vec3::new(-1.0, 2.0, 0.5),
        );
        let patch = BezierPatch::new(1, 1, vec![o, o + b, o + a, o + a + b]).unwrap();
        let surface = Surface::Bezier(patch);
        let tree = PatchTree::new(&surface).unwrap();
        let normal = a.cross(b).normalized().unwrap();
        let p = o + (a + b) * 0.5 + normal * 0.3;
        for resolution in [1e-9, 0.1, 1.0, 10.0] {
            let bound = tree.lower_bound(p, resolution, f64::INFINITY);
            assert!(bound <= 0.3 + 1e-12, "{bound} at {resolution}");
            assert!(bound >= 0.3 - 1e-9, "{bound} at {resolution}");
        }
        let chord = tree.segment_lower_bound(p, p + a * 0.2, 10.0, f64::INFINITY);
        assert!((chord - 0.3).abs() <= 1e-9, "{chord}");
    }
}
