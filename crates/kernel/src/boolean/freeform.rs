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
//! A border of a patch may be one point (a pole: the top of a dome swept
//! about an axis, the apex of a ruled patch). The parameters stall there
//! and are no chart of the surface, so a surface with poles has a chart
//! of its own, the parameter rectangle drawn as a wedge or a lens
//! ([`PatchTree::chart`]), with a speed bounded from nets divided by the
//! parameter across the pole (`wedge_sigma`); the side of the surface a
//! point is on, where its nearest point is the pole, comes from the
//! meridian nearest to it (`craft3d_geom::pole`); and the directions
//! around the pole are told apart seen along the direction the surface
//! closes around (`pole_axes`). docs/design/booleans.md › *Freeform
//! faces with a pole* has the arguments.
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
use craft3d_geom::closure::closes as closure;
use craft3d_geom::pole::{Border, Pole, poles};
use craft3d_geom::{BezierPatch, NurbsSurface, RationalBezierPatch, Surface};
use craft3d_math::Portable;
use craft3d_math::{Bbox3, Vec3};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::{Arc, Mutex, PoisonError};

pub(super) const WEIGHTS: Reason = "a rational freeform face whose weights differ by more than a factor of two million within one span: its derivatives cannot be bounded usefully";
pub(super) const IRREGULAR: Reason = "a freeform face whose parametrisation stalls or folds somewhere (a collapsed or self-overlapping patch); its chart cannot be followed";
pub(super) const POLES_BOTH_WAYS: Reason = "a freeform face whose patch closes into a point on two adjacent sides (collapsed borders both along u and along v); its chart cannot be followed";
pub(super) const POLE_SHAPE: Reason = "a freeform face that closes into a point too sharply or too unevenly for the directions around that point to be told apart (a cusp, or a point the surface folds back over)";
const TOO_MANY: Reason = "a freeform face with more pieces than booleans can index";

/// How many times a Bézier span is quartered at most, and how many leaves
/// a whole surface may have.
const MAX_DEPTH: usize = 5;
const MAX_LEAVES: usize = 1 << 15;
/// A surface whose spans times `4^depth` (the most leaves quartering to
/// the depth could make, not the leaves it makes: a piece stops when it
/// is small) is more than this halves a piece that is short one way only
/// across the other ([`long_ways`]); smaller trees are quartered
/// throughout, as they were before 2026-10-23, so that what they answer
/// stays the same to the bit. At the full depth of five (1,024 leaves a
/// span at most) that is five spans or more: four spans are quartered,
/// five are halved, whatever their shape. The strips of fitted formula
/// curves (30 spans and more) are halved; a single span (a fillet's
/// surface, a patch) is quartered.
const HALVED_FROM: usize = 1 << 12;
/// How far apart the weights of one Bézier span of a rational surface may
/// be. Every bound on its derivatives carries a power of the ratio within
/// a piece, so the bounds are taken from pieces small enough for their
/// weights to be close ([`PatchTree::settled_sigma`], [`PatchTree::finer`],
/// and the marcher's `Pieces` in `geom`). The cap is what is held: the
/// twins of `boolean_rational.rs` skewed a thousandfold both ways, 1.4e6
/// within a span (a thousand until 2026-10-17, when the bounds were taken
/// over a whole leaf or span). The measure (`topo::mass_properties`) holds
/// less: weights more than 1e4 apart after evening by Möbius maps.
const MAX_WEIGHT_RATIO: f64 = 2e6;
/// A leaf's bound on the least singular value is refined where it is more
/// than this many times below the value measured at the corners and the
/// middle of a piece ([`PatchTree::finer`]): a bound that pairs extremes
/// over a piece is within a small factor of the truth wherever the
/// Jacobian changes little over it, and four leaves room for that.
const SIGMA_SLACK: f64 = 4.0;
/// How often a leaf is halved for it at most, and how many pieces one
/// surface keeps (16,384: at most about 11 MB for a bicubic surface, the
/// store emptied when full; [`PatchTree::finer`]'s answers do not depend
/// on it): next to a heavy corner the Jacobian settles within
/// a piece about the weights' ratio smaller than the corner's leaf, and
/// `2^-FINE_DEPTH` of a leaf (`2^-21` of a span) covers ratios to the cap
/// of a span ([`MAX_WEIGHT_RATIO`]).
const FINE_DEPTH: usize = 16;
/// How many pieces a leaf's bound is settled from at most when the tree
/// is built ([`PatchTree::settled_sigma`]).
const LEAF_PIECES: usize = 1 << 8;
const FINE_PIECES: usize = 1 << 14;
/// How many pieces one query of [`PatchTree::finer`] takes before it
/// answers with the lowest bound left, which holds as well.
const FINE_LOOKS: usize = 1 << 10;
/// A piece whose box is more than this many times the reach across is
/// split ([`PatchTree::finer`]): its box says little about where the piece
/// is at that scale. Within a few times the reach the box is as near as
/// the piece, give or take the reach.
const FINE_REACH: f64 = 4.0;
/// How many nearest points a tree remembers before it forgets them all.
const MAX_MEMO: usize = 1 << 14;
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
    children: Option<Kids>,
    /// A leaf keeps its piece, to split further when a query needs a
    /// bound finer than its box, and the box of the piece in its own
    /// frame, thinner than `bbox`, by which the nearest-point search
    /// prunes.
    patch: Option<RationalBezierPatch>,
    slab: Option<Slab>,
    /// A lower bound on the smallest singular value of the Jacobian over
    /// this piece; zero when none is known.
    sigma: f64,
    /// The least `sigma` of the leaves under this node (its own, for a
    /// leaf; not a number when one of them is not): no node under it, and
    /// no piece a leaf under it is split into, has a lower `sigma`, so a
    /// search for the least bound that has found one as low passes it by
    /// ([`PatchTree::sigma_search`]).
    floor: f64,
}

/// The children of a node of a [`PatchTree`]: its four quarters, or its
/// two halves across the one direction in which it is long
/// ([`long_ways`]).
/// An enum, so that `Option<Kids>` takes no more room in a [`Node`] than
/// the four indices of before.
#[derive(Clone, Copy, Debug)]
enum Kids {
    Two([usize; 2]),
    Four([usize; 4]),
}

impl Kids {
    fn four(ids: [usize; 4]) -> Kids {
        Kids::Four(ids)
    }

    fn two(a: usize, b: usize) -> Kids {
        Kids::Two([a, b])
    }

    fn ids(&self) -> &[usize] {
        match self {
            Kids::Two(ids) => ids,
            Kids::Four(ids) => ids,
        }
    }
}

/// Whether `patch` is longer than half of `small` along u and along v: the
/// longest of the rows of its control net that run that way, measured
/// along the net. Any two points of the net are joined by a row one way
/// and a row the other, so they are at most the two lengths apart; the
/// box's diagonal may still be up to √3 times the farthest two points, so
/// a piece above the leaf size can be short both ways, and is then
/// quartered as before. Not a number counts as long.
fn long_ways(patch: &RationalBezierPatch, small: f64) -> (bool, bool) {
    let (du, dv) = patch.degrees();
    let points = patch.control_points();
    let width = dv + 1;
    let at = |i: usize, j: usize| points.get(i.saturating_mul(width).saturating_add(j));
    let mut unknown = false;
    let mut along_u = 0.0_f64;
    for j in 0..=dv {
        let mut length = 0.0;
        for i in 0..du {
            if let (Some(a), Some(b)) = (at(i, j), at(i + 1, j)) {
                length += a.distance(*b);
            }
        }
        unknown |= length.is_nan();
        along_u = along_u.max(length);
    }
    let mut along_v = 0.0_f64;
    for i in 0..=du {
        let mut length = 0.0;
        for j in 0..dv {
            if let (Some(a), Some(b)) = (at(i, j), at(i, j + 1)) {
                length += a.distance(*b);
            }
        }
        unknown |= length.is_nan();
        along_v = along_v.max(length);
    }
    let half = 0.5 * small;
    (unknown || !(along_u <= half), unknown || !(along_v <= half))
}

/// The leaves of a [`PatchTree`] split further where its bound on the
/// least singular value of the Jacobian is far below what the surface has
/// ([`PatchTree::finer`]): a tree of pieces under each leaf that was asked
/// about, by the leaf's index.
/// A float ordered by `total_cmp`, for a heap.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ordered(f64);

impl Eq for Ordered {}

impl PartialOrd for Ordered {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ordered {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

#[derive(Debug, Default)]
struct Fine {
    /// The pieces under each leaf asked about, by the leaf's index (the
    /// leaf's own piece first).
    roots: BTreeMap<usize, FineRoot>,
    /// How many pieces all of them hold.
    pieces: usize,
    /// Counts the queries, to tell which leaf was asked about last.
    clock: u64,
}

#[derive(Debug, Default)]
struct FineRoot {
    nodes: Vec<FinePiece>,
    used: u64,
}

#[derive(Debug)]
struct FinePiece {
    bbox: Bbox3,
    u: (f64, f64),
    v: (f64, f64),
    sigma: f64,
    /// The least and the largest of the least singular values at the
    /// corners and the middle.
    measured: (f64, f64),
    /// Those values, at `(u.0, v.0)`, `(u.0, v.1)`, `(u.1, v.0)`, `(u.1,
    /// v.1)` and the middle: the corners of a piece's halves and quarters
    /// are among its corners and middle, and are not worked out again.
    at: [f64; 5],
    depth: usize,
    /// Kept until the piece is split.
    patch: Option<RationalBezierPatch>,
    children: Option<[usize; 4]>,
    /// The work making it took ([`craft3d_geom::allowance::cost_of`]),
    /// charged once per operation that goes into it, made then or kept.
    units: u64,
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
    /// The borders of the surface that are one point, the direction the
    /// surface closes around at each, and the layout of the chart they
    /// call for.
    poles: Vec<Pole>,
    axes: Vec<Option<Vec3>>,
    wedge: Option<Result<PoleChart, Reason>>,
    /// Whether the surface closes on itself in u and in v ([`closure`]).
    closes: [bool; 2],
    /// The nearest points found so far, by the bits of the point asked
    /// about: the same points are asked about again and again (the ends
    /// and the bisections of a face's boundary, for every winding number
    /// taken about a point of it). A pure function of the point, so the
    /// answer from here is the answer the search would give.
    memo: Memo<[u64; 3], Option<Nearest>>,
    /// The answers of [`Self::sigma_near`] so far, by the bits of the
    /// point and the reach: a winding number about each of many points
    /// halves every piece of a face's boundary the same way and asks the
    /// same of each stretch (the chart's speed near it does not depend on
    /// the point wound about). A pure function of the two (the answer of
    /// [`Self::finer`] does not depend on what it kept), so the answer
    /// from here is the answer the search would give.
    sigma_memo: Memo<[u64; 4], Option<f64>>,
    /// The answers of [`Self::lower_bound`] so far, by the bits of the
    /// point, the resolution and the threshold: the root isolation along
    /// a curve looks at the ends and the quarters of every stretch it
    /// halves, so each end is asked about again by the stretch next to
    /// it and each quarter by the halves. A pure function of the three.
    bound_memo: Memo<[u64; 5], f64>,
    /// How many fine pieces [`Self::fine`] holds, for [`Self::bytes`]
    /// without its lock.
    fine_pieces: std::sync::atomic::AtomicUsize,
    /// What the tree takes without what it remembers: its nodes and the
    /// pieces its leaves keep.
    base_bytes: usize,
    /// About what one fine piece takes, its patch with it.
    fine_piece_bytes: usize,
    /// How many fine pieces the tree keeps at most ([`FINE_PIECES`]; less
    /// in the tests of letting them go).
    fine_cap: usize,
    /// Whether a piece short one way is halved only across the other
    /// ([`HALVED_FROM`]).
    halves: bool,
    /// The leaves split further for [`Self::sigma_near`], as far as its
    /// queries called for ([`Fine`]).
    fine: Mutex<Fine>,
    /// The tree of the same surface drawn more evenly
    /// ([`PatchTree::evened`]), once asked for.
    evened: std::sync::OnceLock<Option<Arc<PatchTree>>>,
    /// A number of the surface, the same for every tree of it (a hash of
    /// its numbers written out), for what an operation remembers of it
    /// ([`ScopedMemos`]).
    id: u64,
    /// The work building the tree took ([`craft3d_geom::allowance::cost_of`]).
    units: u64,
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
/// The quadratic is evaluated as the sum of the squared offsets, each
/// `alpha + beta t` taken as it is: expanded into `qa t² + qb t + qc`, a
/// distance that is zero at a cut reads as the rounding of `alpha²`,
/// which for a box a thousand units from the segment's start is
/// `√(1e6 ε) ≈ 1.5e-5` units, and a segment through a flat box (a leaf
/// of a planar piece) was proven clear of it by that much
/// (docs/design/booleans.md, *The sea kayak's tips*, 2026-10-27).
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
        let mut offsets = [(0.0_f64, 0.0_f64); 3];
        for (offset, ((&start, &step), (&low, &high))) in offsets
            .iter_mut()
            .zip(a3.iter().zip(&d3).zip(lo3.iter().zip(&hi3)))
        {
            let x = start + step * mid;
            *offset = if x < low {
                (low - start, -step)
            } else if x > high {
                (start - high, step)
            } else {
                (0.0, 0.0)
            };
        }
        let (qa, qb) = offsets.iter().fold((0.0, 0.0), |(qa, qb), &(alpha, beta)| {
            (qa + beta * beta, qb + alpha * beta)
        });
        let mut candidates = vec![t0, t1];
        if qa > 0.0 {
            let vertex = -qb / qa;
            if vertex > t0 && vertex < t1 {
                candidates.push(vertex);
            }
        }
        for t in candidates {
            let squared: f64 = offsets
                .iter()
                .map(|&(alpha, beta)| {
                    let off = alpha + beta * t;
                    off * off
                })
                .sum();
            best = best.min(squared);
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
        let (m, n) = patch.degrees();
        let at = |i: usize, j: usize| patch.control_point(i * (n + 1) + j);
        let frame = (|| {
            let (p00, pm0, p0n, pmn) = (at(0, 0)?, at(m, 0)?, at(0, n)?, at(m, n)?);
            let along_u = (pm0 - p00) + (pmn - p0n);
            let along_v = (p0n - p00) + (pmn - pm0);
            let first = along_u.normalized()?;
            let third = along_u.cross(along_v).normalized()?;
            Some([first, third.cross(first), third])
        })();
        let (Some(axes), Some(origin)) = (frame, patch.control_point(0)) else {
            return Slab::axis_aligned(patch.control_bbox());
        };
        let slab = Slab {
            origin,
            axes,
            local: Bbox3::EMPTY,
        };
        let local = Bbox3::from_points(patch.points().map(|p| slab.to_local(p)));
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

/// The largest squared lengths of the two partial derivatives of a patch
/// and the length of its shortest normal, over the piece's own `[0, 1]²`,
/// as far as its nets bound them.
fn piece_bounds(patch: &RationalBezierPatch) -> Option<(f64, f64, f64)> {
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
    }
}

/// A lower bound on the smallest singular value of the Jacobian of a
/// patch over all of it, in the parameters of the surface it is a piece
/// of (the piece covers `width.0` by `width.1` of them), or zero when the
/// nets allow a vanishing or folding normal.
fn sigma_bound(patch: &RationalBezierPatch, width: (f64, f64)) -> f64 {
    // The largest squared lengths of the two partial derivatives and the
    // shortest normal, over the piece's own `[0, 1]²`.
    let Some((longest_u, longest_v, normal)) = piece_bounds(patch) else {
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
    let product = (normal / (wu * wv)).ppowi(2);
    least_singular(sum, product)
}

/// The smallest singular value of a Jacobian `[a b]` whose `|a|² + |b|²`
/// is at most `sum` and whose `|a × b|²` is at least `product`, at least.
fn least_singular(sum: f64, product: f64) -> f64 {
    if !(sum > 0.0 && sum.is_finite() && product.is_finite()) {
        return 0.0;
    }
    // The smaller root of x² − sum x + product, written so that a product
    // far below the sum squared does not cancel.
    let least = 2.0 * product / (sum + (sum * sum - 4.0 * product).max(0.0).sqrt());
    least.max(0.0).sqrt()
}

/// How the chart of a surface with poles is laid out (see
/// [`PatchTree::chart`]).
#[derive(Clone, Copy, Debug)]
struct Wedge {
    /// Whether the poles are on borders of constant v (else of constant
    /// u).
    across_v: bool,
    /// A pole at the start of the parameter across the borders, and at
    /// its end.
    low: bool,
    high: bool,
    /// The order in which the surface's width vanishes at the pole at the
    /// start and at the end ([`pole_order`]): one where it leaves the
    /// pole in a fan, two at a cusp (a profile that leaves the axis along
    /// it); zero where there is no pole.
    orders: (i32, i32),
    /// The domain of the surface.
    u: (f64, f64),
    v: (f64, f64),
}

/// The norm of the shear `[1 k; 0 1]` with `|k| ≤ ½`, at most: what the
/// wedge chart stretches a step by beyond the Jacobian it is bounded
/// with (`√(1 + k²/2 + |k| √(1 + k²/4))` is 1.2808 at ½).
const SHEAR: f64 = 1.29;
/// A row of a net this share of its longest vector long is zero: the
/// field vanishes along that border.
const ZERO_ROW: f64 = 1e-9;
/// The limit normals around a pole all make an angle with their mean whose
/// cosine is more than this, or the directions around the pole are not
/// told apart (see [`pole_axes`]).
const AXIS_MARGIN: f64 = 0.05;

impl Wedge {
    /// The layout for the poles of a surface: `None` without poles, an
    /// error for poles both ways across.
    fn of(surface: &Surface, poles: &[Pole]) -> Option<Result<PoleChart, Reason>> {
        let first = poles.first()?;
        let across_v = first.across_v();
        if poles.iter().any(|pole| pole.across_v() != across_v) {
            return Some(
                Corner::of(surface, poles)
                    .map(PoleChart::Corner)
                    .ok_or(POLES_BOTH_WAYS),
            );
        }
        let (u, v) = surface.domain();
        // The order of the pole on each side, none without one; a pole
        // whose order cannot be told has no chart.
        let order =
            |a: Border, b: Border| match poles.iter().find(|p| p.border == a || p.border == b) {
                Some(pole) => pole_order(surface, pole),
                None => Some(0),
            };
        let (Some(low), Some(high)) = (
            order(Border::UStart, Border::VStart),
            order(Border::UEnd, Border::VEnd),
        ) else {
            return Some(Err(POLE_SHAPE));
        };
        let orders = (low, high);
        Some(Ok(PoleChart::Wedge(Wedge {
            across_v,
            low: orders.0 > 0,
            high: orders.1 > 0,
            orders,
            u,
            v,
        })))
    }

    /// How wide the surface is across a pole's borders at the share `t`
    /// of the way between them, as the chart draws it: zero at a pole, to
    /// the pole's order (`t`, `1 − t`, `t (1 − t)`; `t²` at a cusp).
    fn width(&self, t: f64) -> f64 {
        t.ppowi(self.orders.0) * (1.0 - t).ppowi(self.orders.1)
    }

    /// Where the width is greatest when there is a pole at either end.
    fn widest_at(&self) -> f64 {
        let (a, b) = (f64::from(self.orders.0), f64::from(self.orders.1));
        if a + b > 0.0 { a / (a + b) } else { 0.5 }
    }

    /// The norm of the shear `[1 k; 0 1]` the chart stretches a step by
    /// beyond the Jacobian it is bounded with, at most: `|k| = |ω′ (s −
    /// ½)|`, and `|ω′| ≤ max(a, b)` for `ω = tᵃ (1 − t)ᵇ` (`ω′ = tᵃ⁻¹ (1 −
    /// t)ᵇ⁻¹ (a (1 − t) − b t)`). [`SHEAR`] for the orders one.
    fn shear(&self) -> f64 {
        let k = 0.5 * f64::from(self.orders.0.max(self.orders.1).max(1));
        if k <= 0.5 {
            return SHEAR;
        }
        1.01 * (1.0 + 0.5 * k * k + k * (1.0 + 0.25 * k * k).sqrt()).sqrt()
    }

    /// The parameters as shares of the domain: along the poles' borders,
    /// and across them.
    fn shares(&self, u: f64, v: f64) -> (f64, f64) {
        let share = |x: f64, (lo, hi): (f64, f64)| (x - lo) / (hi - lo);
        if self.across_v {
            (share(u, self.u), share(v, self.v))
        } else {
            (share(v, self.v), share(u, self.u))
        }
    }

    /// The chart coordinates of the point at `(u, v)`.
    fn chart(&self, u: f64, v: f64) -> (f64, f64) {
        let (s, t) = self.shares(u, v);
        let across = self.width(t) * (s - 0.5);
        // Turning the same way as the parameters do.
        if self.across_v {
            (across, t)
        } else {
            (t, across)
        }
    }
}

/// A net of degree `degree` in its second index (`degree + 1` vectors a
/// row) divided by that parameter (`low`) or by one less it (`!low`):
/// the net, one degree lower, of the field over the parameter, which is a
/// polynomial when the field vanishes on that border, that is when the
/// net's row there is zero. `None` when it is not (to [`ZERO_ROW`] of the
/// net's longest vector).
///
/// `B_j^n(t) / t = (n / j) B_{j−1}^{n−1}(t)` and `B_j^n(t) / (1 − t) =
/// (n / (n − j)) B_j^{n−1}(t)`.
fn divided(net: &[Vec3], degree: usize, low: bool) -> Option<Vec<Vec3>> {
    if degree == 0 || !net.len().is_multiple_of(degree + 1) {
        return None;
    }
    let longest = net.iter().map(|v| v.length()).fold(0.0, f64::max);
    let dropped = if low { 0 } else { degree };
    let mut out = Vec::with_capacity(net.len() / (degree + 1) * degree);
    for row in net.chunks(degree + 1) {
        // Not `>`: a NaN is no zero.
        if !(row.get(dropped)?.length() <= ZERO_ROW * longest) {
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

/// How the chart of a surface with poles is laid out (see
/// [`PatchTree::chart`]).
#[derive(Clone, Copy, Debug)]
enum PoleChart {
    /// Poles on borders across one parameter: a wedge, or a lens.
    Wedge(Wedge),
    /// One pole on two adjacent borders: a corner of the parameters.
    Corner(Corner),
}

/// The chart of a surface whose patch closes into one point on two
/// adjacent borders, one across u and one across v (an L of the parameter
/// rectangle is one point; the other two borders meet at an ordinary
/// corner, as a lune of a ball between two meridians meets the far pole).
///
/// With `s` and `t` the shares of the domain measured from the pole's
/// borders (`s = 0` on the one across u, `t = 0` on the one across v),
/// the point goes to `(s t (s − t), s t)`, its first coordinate negated
/// when exactly one of the two is measured backwards (so that the chart
/// turns the way the parameters do). The L is the origin; `(s − t, s t)`
/// is one to one on the square (`s` is the positive root of
/// `x² + (t − s) x − s t`), so the chart is; and near the origin it is a
/// wedge `|x| ≤ y`, the meridians leaving the pole along the L in their
/// order on it, from `(0, 1)` through `(0, 0)` to `(1, 0)`.
///
/// Its speed. With `τ = s t` and `σ = t − s` the chart is `(−τ σ, τ)`
/// (up to the sign), so a step moves it by `(−σ dτ − τ dσ, dτ)`: by at
/// most `√(σ² + 2) ≤ √3` ([`CORNER_STRETCH`]) times `|(dτ, τ dσ)|`.
/// `S − P` vanishes on both borders, so `S_s = t G` and `S_t = s H` with
/// `G` and `H` bounded, and the same step moves the surface by `A dτ + B
/// τ dσ` (`corner_sigma`); so the chart moves at most `√3 / σ` per length
/// on the surface, `σ` a bound below the least singular value of `[A B]`.
#[derive(Clone, Copy, Debug)]
struct Corner {
    /// Whether the pole's border across u is at u's end (else its start),
    /// and the same of v.
    u_end: bool,
    v_end: bool,
    /// The domain of the surface.
    u: (f64, f64),
    v: (f64, f64),
}

/// `√3`, rounded up: how much the corner chart stretches `(dτ, τ dσ)` at
/// most (see [`Corner`]).
const CORNER_STRETCH: f64 = 1.7321;

impl Corner {
    /// The corner layout for poles on one border across u and one across
    /// v; `None` for any other set of poles.
    fn of(surface: &Surface, poles: &[Pole]) -> Option<Corner> {
        let [a, b] = poles else {
            return None;
        };
        let (across_u, across_v) = match (a.across_v(), b.across_v()) {
            (false, true) => (a, b),
            (true, false) => (b, a),
            _ => return None,
        };
        let (u, v) = surface.domain();
        Some(Corner {
            u_end: across_u.border == Border::UEnd,
            v_end: across_v.border == Border::VEnd,
            u,
            v,
        })
    }

    /// The shares of the domain measured from the pole's borders. A
    /// parameter on such a border has the share zero exactly.
    fn shares(&self, u: f64, v: f64) -> (f64, f64) {
        let share = |x: f64, (lo, hi): (f64, f64), end: bool| {
            if end {
                (hi - x) / (hi - lo)
            } else {
                (x - lo) / (hi - lo)
            }
        };
        (share(u, self.u, self.u_end), share(v, self.v, self.v_end))
    }

    /// The chart coordinates of the point at `(u, v)`.
    fn chart(&self, u: f64, v: f64) -> (f64, f64) {
        let (s, t) = self.shares(u, v);
        let r = s * t;
        let x = r * (s - t);
        (if self.u_end == self.v_end { x } else { -x }, r)
    }
}

/// A net of `rows` by `columns` vectors with its indices exchanged.
fn transposed_net(net: &[Vec3], rows: usize, columns: usize) -> Option<Vec<Vec3>> {
    if net.len() != rows.checked_mul(columns)? {
        return None;
    }
    let mut out = Vec::with_capacity(net.len());
    for j in 0..columns {
        for i in 0..rows {
            out.push(*net.get(i * columns + j)?);
        }
    }
    Some(out)
}

/// A net of degrees `(m, n)` (`n + 1` vectors a row) divided by its
/// first parameter (`low`) or by one less it: see [`divided`].
fn divided_first(net: &[Vec3], m: usize, n: usize, low: bool) -> Option<Vec<Vec3>> {
    let turned = transposed_net(net, m + 1, n + 1)?;
    let quotient = divided(&turned, m, low)?;
    transposed_net(&quotient, n + 1, m)
}

/// A net of degrees `(p, q)` read from the other end in its first
/// parameter (`first`) and in its second (`second`): the net of the same
/// polynomial in `1 − α` or `1 − β`.
fn flipped_net(net: &[Vec3], p: usize, q: usize, first: bool, second: bool) -> Option<Vec<Vec3>> {
    let columns = q.checked_add(1)?;
    if net.len() != p.checked_add(1)?.checked_mul(columns)? {
        return None;
    }
    let mut out = Vec::with_capacity(net.len());
    for i in 0..=p {
        for j in 0..=q {
            let (a, b) = (
                if first { p - i } else { i },
                if second { q - j } else { j },
            );
            out.push(*net.get(a * columns + b)?);
        }
    }
    Some(out)
}

/// A net of degrees `(p, q)` times the linear polynomial `c₀ (1 − β) +
/// c₁ β` of its second parameter: the net of degrees `(p, q + 1)`
/// (`B_j^q B_k^1 = C(q, j) / C(q + 1, j + k) B_{j+k}^{q+1}`).
fn times_linear(net: &[Vec3], q: usize, (c0, c1): (f64, f64)) -> Option<Vec<Vec3>> {
    let width = q.checked_add(1)?;
    if !net.len().is_multiple_of(width) {
        return None;
    }
    let top = (q + 1) as f64;
    let mut out = Vec::with_capacity(net.len() / width * (width + 1));
    for row in net.chunks(width) {
        for k in 0..=q + 1 {
            let low = row
                .get(k)
                .map_or(Vec3::ZERO, |&f| f * (c0 * (top - k as f64) / top));
            let high = k
                .checked_sub(1)
                .and_then(|j| row.get(j))
                .map_or(Vec3::ZERO, |&f| f * (c1 * k as f64 / top));
            out.push(low + high);
        }
    }
    Some(out)
}

/// The same times a linear polynomial of its first parameter.
fn times_linear_first(net: &[Vec3], p: usize, q: usize, c: (f64, f64)) -> Option<Vec<Vec3>> {
    let turned = transposed_net(net, p + 1, q + 1)?;
    let product = times_linear(&turned, p, c)?;
    transposed_net(&product, q + 1, p + 2)
}

/// A field `X` of degrees `(p, q)` that vanishes at `(0, 0)` (its corner
/// coefficient zero to [`ZERO_ROW`] of its longest), written `X = α X_a +
/// β X_b` with `X_a = (X(α, β) − X(0, β)) / α` (degrees `(p − 1, q)`) and
/// `X_b = (X(0, β) − X(0, 0)) / β` (degree `q − 1`, a curve): then for
/// positive `a` and `b`, `X / (a α + b β)` is a mean of `X_a / a` and
/// `X_b / b` with the weights `a α` and `b β`, in the hull of the two
/// nets so scaled. `None` when the corner is not zero.
fn over_sum(net: &[Vec3], p: usize, q: usize) -> Option<(Vec<Vec3>, Vec<Vec3>)> {
    let width = q.checked_add(1)?;
    let longest = net.iter().map(|v| v.length()).fold(0.0, f64::max);
    let corner = *net.first()?;
    if !(corner.length() <= ZERO_ROW * longest) {
        return None;
    }
    let first_row: Vec<Vec3> = net.get(..width)?.to_vec();
    let mut less = Vec::with_capacity(net.len());
    for row in net.chunks(width) {
        for (j, &x) in row.iter().enumerate() {
            less.push(x - *first_row.get(j)?);
        }
    }
    let a = divided_first(&less, p, q, true)?;
    let curve: Vec<Vec3> = first_row.iter().map(|&x| x - corner).collect();
    let b = divided(&curve, q, true)?;
    Some((a, b))
}

/// What the corner chart needs of a piece of a surface, in its own
/// parameters read from the pole's side (`α = 0` on the pole's border
/// across u where the piece reaches it, `β = 0` likewise):
///
/// - `G` and `H`: the nets of `w² S_α` divided by `β` and of `w² S_β`
///   divided by `α` where the piece reaches the border (else undivided);
/// - `E`: the net of `w² (t/w_t S_β − s/w_s S_α)`, which is `w² s t (H −
///   G)` in the notation of [`Corner`], divided by `α` and `β` likewise;
/// - `K`: the net of `w³ (S_α × S_β)` divided likewise, its sign as the
///   surface's `∂u × ∂v` has it;
///
/// and the degrees of `E` and of `K`, the lightest and the heaviest
/// weight.
struct CornerNets {
    g: Vec<Vec3>,
    h: Vec<Vec3>,
    e: Vec<Vec3>,
    e_degrees: (usize, usize),
    k: Vec<Vec3>,
    k_degrees: (usize, usize),
    lightest: f64,
    heaviest: f64,
}

/// The nets of [`CornerNets`] for a piece that reaches the pole's border
/// across u (`on_u`) and across v (`on_v`), at shares `s_lo` and `t_lo`
/// from them and `ws` by `wt` wide.
fn corner_nets(
    patch: &RationalBezierPatch,
    corner: &Corner,
    (on_u, on_v): (bool, bool),
    (s_lo, t_lo, ws, wt): (f64, f64, f64, f64),
) -> Option<CornerNets> {
    let (m, n) = patch.degrees();
    let (lightest, heaviest) = patch.weight_range()?;
    let (du, dv) = patch.weighted_derivative_nets()?;
    let normal = patch.weighted_normal_net()?;
    let (fu, fv) = (corner.u_end, corner.v_end);
    // Read from the pole's side: a derivative along a parameter read
    // backwards changes sign; the normal keeps the surface's.
    let sign = |flip: bool| if flip { -1.0 } else { 1.0 };
    let n1: Vec<Vec3> = flipped_net(&du, (2 * m).checked_sub(1)?, 2 * n, fu, fv)?
        .into_iter()
        .map(|x| x * sign(fu))
        .collect();
    let n2: Vec<Vec3> = flipped_net(&dv, 2 * m, (2 * n).checked_sub(1)?, fu, fv)?
        .into_iter()
        .map(|x| x * sign(fv))
        .collect();
    let (nm, nn) = ((3 * m).checked_sub(1)?, (3 * n).checked_sub(1)?);
    let mut k = flipped_net(&normal, nm, nn, fu, fv)?;
    // `s / w_s = s_lo / w_s + α` and `t / w_t` likewise, as linear
    // Bernstein forms; `s_lo` is zero where the piece reaches the border.
    let (cs, ct) = (s_lo / ws, t_lo / wt);
    let along_t = times_linear(&n2, (2 * n).checked_sub(1)?, (ct, ct + 1.0))?;
    let along_s = times_linear_first(&n1, (2 * m).checked_sub(1)?, 2 * n, (cs, cs + 1.0))?;
    let mut e: Vec<Vec3> = along_t.iter().zip(&along_s).map(|(&a, &b)| a - b).collect();
    if e.len() != along_t.len() || along_t.len() != along_s.len() {
        return None;
    }
    let (mut g, mut h) = (n1, n2);
    let (mut em, mut en) = (2 * m, 2 * n);
    let (mut km, mut kn) = (nm, nn);
    if on_v {
        g = divided(&g, 2 * n, true)?;
        e = divided(&e, en, true)?;
        k = divided(&k, kn, true)?;
        en = en.checked_sub(1)?;
        kn = kn.checked_sub(1)?;
    }
    if on_u {
        h = divided_first(&h, 2 * m, (2 * n).checked_sub(1)?, true)?;
        e = divided_first(&e, em, en, true)?;
        k = divided_first(&k, km, kn, true)?;
        em = em.checked_sub(1)?;
        km = km.checked_sub(1)?;
    }
    Some(CornerNets {
        g,
        h,
        e,
        e_degrees: (em, en),
        k,
        k_degrees: (km, kn),
        lightest,
        heaviest,
    })
}

/// A lower bound on the least singular value of the Jacobian of the
/// corner chart's inverse over a piece of a surface with a corner pole
/// (see [`Corner`]), in the shares of its domain: how far a point of the
/// surface moves, at least, per unit that `(dτ, τ dσ)` moves. Zero when
/// the nets give none.
///
/// A step moves the surface by `A dτ + B τ dσ` with `A = (t G + s H) /
/// (s + t)`, a mean of `G` and `H`, and `B = (H − G) / (s + t)`; and
/// `A × B = (G × H) / (s + t)`. With the piece's own parameters (`s =
/// s₀ + w_s α`, `t` likewise), `G = S_α / (w_s t)`, `H = S_β / (w_t s)`,
/// `H − G = E / (w² s t)` and `G × H = K / (w³ w_s w_t s t)` with the
/// nets of [`CornerNets`] before their division. Where the piece reaches
/// a border its parameter divides out (`s = w_s α`); elsewhere the share
/// is bounded by the piece's least or greatest. At the corner itself
/// `H − G` and `G × H` vanish as `s + t` does: there `E` and `K` divided
/// by both are split by [`over_sum`], and the quotient by `s + t` is in
/// the hull of the two halves.
fn corner_sigma(patch: &RationalBezierPatch, u: (f64, f64), v: (f64, f64), corner: &Corner) -> f64 {
    corner_sigma_of(patch, u, v, corner).unwrap_or(0.0)
}

fn corner_sigma_of(
    patch: &RationalBezierPatch,
    u: (f64, f64),
    v: (f64, f64),
    corner: &Corner,
) -> Option<f64> {
    let (sa, ta) = corner.shares(u.0, v.0);
    let (sb, tb) = corner.shares(u.1, v.1);
    let (s_lo, s_hi) = (sa.min(sb), sa.max(sb));
    let (t_lo, t_hi) = (ta.min(tb), ta.max(tb));
    let (ws, wt) = (s_hi - s_lo, t_hi - t_lo);
    if !(ws > 0.0 && wt > 0.0 && s_lo >= 0.0 && t_lo >= 0.0) {
        return None;
    }
    // A piece reaches a pole's border where its share is zero (exactly:
    // see `Corner::shares`).
    let (on_u, on_v) = (s_lo == 0.0, t_lo == 0.0);
    let nets = corner_nets(patch, corner, (on_u, on_v), (s_lo, t_lo, ws, wt))?;
    let longest = |net: &[Vec3]| net.iter().map(|v| v.length()).fold(0.0, f64::max);
    let light = nets.lightest * nets.lightest;
    let heavy = nets.heaviest * nets.heaviest * nets.heaviest;
    // |G| and |H|, and so |A|.
    let g = longest(&nets.g) / (light * ws * if on_v { wt } else { t_lo });
    let h = longest(&nets.h) / (light * wt * if on_u { ws } else { s_lo });
    let a = g.max(h);
    // |B| = |H − G| / (s + t) at most, and |A × B| at least.
    let (b, cross) = if on_u && on_v {
        let (em, en) = nets.e_degrees;
        let (ea, eb) = over_sum(&nets.e, em, en)?;
        let (km, kn) = nets.k_degrees;
        let (ka, kb) = over_sum(&nets.k, km, kn)?;
        let hull: Vec<Vec3> = ka
            .iter()
            .map(|&x| x * (1.0 / ws))
            .chain(kb.iter().map(|&x| x * (1.0 / wt)))
            .collect();
        (
            (longest(&ea) / ws).max(longest(&eb) / wt) / (light * ws * wt),
            clearance_of(&hull) / (heavy * ws * ws * wt * wt),
        )
    } else {
        // Of `s t` what is not divided out, at least and at most.
        let (least, most) = match (on_u, on_v) {
            (true, false) => (ws * t_lo, ws * t_hi),
            (false, true) => (s_lo * wt, s_hi * wt),
            _ => (s_lo * t_lo, s_hi * t_hi),
        };
        let (near, far) = (s_lo + t_lo, s_hi + t_hi);
        (
            longest(&nets.e) / (light * least * near),
            clearance_of(&nets.k) / (heavy * ws * wt * most * far),
        )
    };
    let normal = cross;
    if !(a.is_finite() && b.is_finite() && normal.is_finite()) {
        return None;
    }
    // A normal that short against the derivatives is rounding (see
    // `sigma_bound`).
    if !(normal * normal > ROUNDING * ROUNDING * a * a * b * b) {
        return None;
    }
    Some(least_singular(a * a + b * b, normal * normal))
}

/// The direction a surface closes around at a corner pole (see
/// [`pole_axes`]): the mean of its limit normals along the whole L,
/// `None` when they are not all within a quarter turn (less
/// [`AXIS_MARGIN`]) of it. The limit normals are the directions of `G ×
/// H` on the L: of the net `K` of [`CornerNets`] on its border where a
/// span reaches one, and at the corner of the two halves [`over_sum`]
/// splits it into (on the border across u `K = β K_b`, across v `K = α
/// K_a`).
fn corner_axis(surface: &Surface, corner: &Corner) -> Option<Vec3> {
    let spans = spans_of(surface)?;
    let mut rows: Vec<Vec3> = Vec::new();
    for (u, v, patch) in &spans {
        let (sa, ta) = corner.shares(u.0, v.0);
        let (sb, tb) = corner.shares(u.1, v.1);
        let (s_lo, t_lo) = (sa.min(sb), ta.min(tb));
        let (ws, wt) = (sa.max(sb) - s_lo, ta.max(tb) - t_lo);
        let (on_u, on_v) = (s_lo == 0.0, t_lo == 0.0);
        if !(on_u || on_v) {
            continue;
        }
        let nets = corner_nets(patch, corner, (on_u, on_v), (s_lo, t_lo, ws, wt))?;
        let (km, kn) = nets.k_degrees;
        let width = kn + 1;
        if on_u && on_v {
            let (ka, kb) = over_sum(&nets.k, km, kn)?;
            rows.extend(kb);
            rows.extend(ka.chunks(width).filter_map(|row| row.first().copied()));
        } else if on_u {
            rows.extend(nets.k.iter().take(width).copied());
        } else {
            rows.extend(nets.k.chunks(width).filter_map(|row| row.first().copied()));
        }
    }
    let mut mean = Vec3::ZERO;
    for row in &rows {
        mean += row.normalized()?;
    }
    let axis = mean.normalized()?;
    rows.iter()
        .all(|row| row.dot(axis) > AXIS_MARGIN * row.length())
        .then_some(axis)
}

/// What the wedge chart needs of a piece next to a pole: the piece with
/// the parameter across the poles' borders second, the nets of `w² ∂σ`
/// and of `w³ (∂σ × ∂τ)` divided by `τ` (a pole at the piece's start),
/// by `1 − τ` (at its end) or by both, with the degree of each in `τ`,
/// and the longest `w² ∂τ`.
struct BesidePole {
    along: Vec<Vec3>,
    normal: Vec<Vec3>,
    /// The degree of `normal` across the borders.
    normal_degree: usize,
    /// The net of `w² ∂τ` (not divided), and its degree in `τ`.
    across: Vec<Vec3>,
    across_degree: usize,
    longest_across: f64,
    lightest: f64,
    heaviest: f64,
}

fn beside_pole(
    patch: &RationalBezierPatch,
    across_v: bool,
    start: i32,
    end: i32,
) -> Option<BesidePole> {
    let turned;
    let patch = if across_v {
        patch
    } else {
        turned = patch.transposed();
        &turned
    };
    let (_, n) = patch.degrees();
    let (lightest, heaviest) = patch.weight_range()?;
    let (mut along, across) = patch.weighted_derivative_nets()?;
    let mut normal = patch.weighted_normal_net()?;
    let (mut along_degree, mut normal_degree) = (2 * n, (3 * n).checked_sub(1)?);
    for (times, low) in [(start, true), (end, false)] {
        for _ in 0..times {
            along = divided(&along, along_degree, low)?;
            normal = divided(&normal, normal_degree, low)?;
            along_degree = along_degree.checked_sub(1)?;
            normal_degree = normal_degree.checked_sub(1)?;
        }
    }
    if !across_v {
        // The transposed piece's normal points the other way.
        for vector in &mut normal {
            *vector = -*vector;
        }
    }
    Some(BesidePole {
        along,
        normal,
        normal_degree,
        across_degree: (2 * n).checked_sub(1)?,
        longest_across: across.iter().map(|v| v.length()).fold(0.0, f64::max),
        across,
        lightest,
        heaviest,
    })
}

/// A lower bound on the smallest singular value of the Jacobian of the
/// wedge chart's inverse over a piece of a surface with poles: how far a
/// point of the surface moves, at least, per unit that its image in the
/// chart moves, up to the shear ([`SHEAR`]). Zero when the nets give
/// none.
///
/// With `s` and `t` the shares of the domain along and across the poles'
/// borders and `ω(t)` the chart's width there, a step `(ds, dt)` moves
/// the chart's point by the shear of `(ω ds, dt)` and the surface's by
/// `E ω ds + F dt` with `E = S_s / ω` and `F = S_t`. `S_s` vanishes at a
/// pole as `ω` does, so `E` is bounded: away from a pole by the nets of
/// the piece over the least `ω`, next to one by the net of `w² S_s`
/// divided by the piece's own parameter ([`divided`]), and likewise the
/// normal `E × F` below.
fn wedge_sigma(patch: &RationalBezierPatch, u: (f64, f64), v: (f64, f64), wedge: &Wedge) -> f64 {
    let (across, whole) = if wedge.across_v {
        (v, wedge.v)
    } else {
        (u, wedge.u)
    };
    let at_start = wedge.low && across.0 == whole.0;
    let at_end = wedge.high && across.1 == whole.1;
    // The piece's shares of the domain: where it starts and ends along
    // the borders (`s`) and across them (`t`), and how wide it is.
    let ((s0, t0), (s1, t1)) = (wedge.shares(u.0, v.0), wedge.shares(u.1, v.1));
    let (ws, wt) = (s1 - s0, t1 - t0);
    if !(ws > 0.0 && wt > 0.0) {
        return 0.0;
    }
    let (longest_e, longest_f, normal) = if !at_start && !at_end {
        let Some((longest_u, longest_v, normal)) = piece_bounds(patch) else {
            return 0.0;
        };
        let (longest_s, longest_t) = if wedge.across_v {
            (longest_u, longest_v)
        } else {
            (longest_v, longest_u)
        };
        // The chart's width is concave: least at an end of the piece,
        // greatest at an end or half way between two poles.
        let narrowest = wedge.width(t0).min(wedge.width(t1));
        let mut widest = wedge.width(t0).max(wedge.width(t1));
        let peak = wedge.widest_at();
        if wedge.low && wedge.high && t0 < peak && t1 > peak {
            widest = widest.max(wedge.width(peak));
        }
        (
            longest_s.sqrt() / (ws * narrowest),
            longest_t.sqrt() / wt,
            normal / (ws * wt * widest),
        )
    } else {
        let (start, end) = (
            if at_start { wedge.orders.0 } else { 0 },
            if at_end { wedge.orders.1 } else { 0 },
        );
        let Some(nets) = beside_pole(patch, wedge.across_v, start, end) else {
            return 0.0;
        };
        let longest = nets.along.iter().map(|v| v.length()).fold(0.0, f64::max);
        let light = nets.lightest * nets.lightest;
        let heavy = nets.heaviest * nets.heaviest * nets.heaviest;
        // Next to one pole the width is the piece's own parameter (or one
        // less it) times the piece's share `wt`, times the factor of the
        // other pole, which is least at the piece's far end and at most
        // one; next to both it is the product of the two, the piece being
        // the whole way across.
        let (scale, least_factor) = match (at_start, at_end) {
            (true, true) => (1.0, 1.0),
            (true, false) => (wt.ppowi(wedge.orders.0), (1.0 - t1).ppowi(wedge.orders.1)),
            (false, true) => (wt.ppowi(wedge.orders.1), t0.ppowi(wedge.orders.0)),
            (false, false) => return 0.0,
        };
        (
            longest / (light * ws * scale * least_factor),
            nets.longest_across / (light * wt),
            clearance_of(&nets.normal) / (heavy * ws * wt * scale),
        )
    };
    // A normal that short against the derivatives is rounding (see
    // `sigma_bound`).
    if !(normal * normal > ROUNDING * ROUNDING * longest_e * longest_e * longest_f * longest_f) {
        return 0.0;
    }
    least_singular(
        longest_e * longest_e + longest_f * longest_f,
        normal * normal,
    )
}

/// The Bézier spans of a freeform surface.
fn spans_of(surface: &Surface) -> Option<Vec<Span>> {
    match surface {
        Surface::Bezier(patch) => Some(vec![(
            (0.0, 1.0),
            (0.0, 1.0),
            RationalBezierPatch::from_bezier(patch),
        )]),
        Surface::Nurbs(s) => Some(
            NurbsSurface::bezier_spans(s)
                .ok()?
                .into_iter()
                .map(|span| (span.u, span.v, span.patch))
                .collect(),
        ),
        _ => None,
    }
}

/// Per pole of a surface, the direction the surface closes around there:
/// the mean of the unit normals it reaches the pole with along its
/// meridians, pointing the way `∂u × ∂v` does. Where the surface closes
/// up smoothly that is its normal at the pole; at a cone-like point it
/// is the axis of the cone, near enough, on the side the normals lean
/// to. `None` for a pole whose limit normals are not all within a
/// quarter turn (less [`AXIS_MARGIN`]) of that mean.
///
/// What it is for: seen along this direction, the meridians leave the
/// pole in the order they have on the surface. A meridian's direction,
/// dropped onto the plane across the axis, turns at a rate that has the
/// sign of the axis times the limit normal there (the normal is the
/// meridian's direction crossed with its rate of change), so with every
/// limit normal on the axis's side the turning never reverses. The limit
/// normals along a pole's border are, up to a positive factor, the field
/// `w³ (∂u × ∂v)` divided by the parameter across the border, whose
/// border row of its net holds them all in its hull: the test is on the
/// vectors of those rows, of every span at the pole.
fn pole_axes(surface: &Surface, poles: &[Pole]) -> Vec<Option<Vec3>> {
    if poles.is_empty() {
        return Vec::new();
    }
    let spans = spans_of(surface).unwrap_or_default();
    poles
        .iter()
        .map(|pole| {
            let across_v = pole.across_v();
            let at_start = matches!(pole.border, Border::UStart | Border::VStart);
            let order = pole_order(surface, pole)?;
            let (start, end) = if at_start { (order, 0) } else { (0, order) };
            // The limit normals, and at a cusp the meridians' directions
            // and how the normals lean along them.
            let mut rows: Vec<Vec3> = Vec::new();
            let mut tangents: Vec<Vec3> = Vec::new();
            let mut leans: Vec<Vec3> = Vec::new();
            for (_, _, patch) in spans.iter().filter(|span| touches(surface, pole, span)) {
                let nets = beside_pole(patch, across_v, start, end)?;
                let width = nets.normal_degree + 1;
                let (pick, next) = if at_start {
                    (0, 1)
                } else {
                    (nets.normal_degree, nets.normal_degree.checked_sub(1)?)
                };
                for row in nets.normal.chunks(width) {
                    rows.push(*row.get(pick)?);
                    // `f′` at the border is `n (b₁ − b₀)` (from the end:
                    // `n (b_n − b_{n−1})`, with the parameter running
                    // into the pole).
                    leans.push(*row.get(next)? - *row.get(pick)?);
                }
                let wide = nets.across_degree + 1;
                let at = if at_start { 0 } else { nets.across_degree };
                for row in nets.across.chunks(wide) {
                    let tangent = *row.get(at)?;
                    // Pointing away from the pole, into the surface.
                    tangents.push(if at_start { tangent } else { -tangent });
                }
            }
            if order <= 1 {
                let axis = mean_direction(&rows)?;
                return rows
                    .iter()
                    .all(|row| row.dot(axis) > AXIS_MARGIN * row.length())
                    .then_some(axis);
            }
            // A cusp: every meridian leaves the pole along one line, and
            // the limit normals are square to it, all around. The
            // direction the surface closes around is that line, on the
            // side the normals lean to as they leave the pole (as at the
            // apex of a cone, which a cusp is with no angle).
            let line = mean_direction(&tangents)?;
            if !tangents
                .iter()
                .all(|t| t.dot(line) > (1.0 - AXIS_MARGIN) * t.length())
            {
                return None;
            }
            let side: Vec<f64> = leans.iter().map(|lean| lean.dot(line)).collect();
            if side.iter().all(|&x| x > 0.0) {
                Some(line)
            } else if side.iter().all(|&x| x < 0.0) {
                Some(-line)
            } else {
                None
            }
        })
        .collect()
}

/// Whether a span reaches the border of a pole.
fn touches(surface: &Surface, pole: &Pole, (u, v, _): &Span) -> bool {
    let ((u0, u1), (v0, v1)) = surface.domain();
    match pole.border {
        Border::UStart => u.0 == u0,
        Border::UEnd => u.1 == u1,
        Border::VStart => v.0 == v0,
        Border::VEnd => v.1 == v1,
    }
}

/// The mean of the directions of some vectors, `None` when one has no
/// direction or they cancel.
fn mean_direction(vectors: &[Vec3]) -> Option<Vec3> {
    let mut mean = Vec3::ZERO;
    for vector in vectors {
        mean += vector.normalized()?;
    }
    mean.normalized()
}

/// The order in which a surface narrows to a pole: how many times the net
/// of `w² ∂σ` (the derivative along the pole's border) of every span at
/// the pole can be divided by the parameter across ([`divided`]): as
/// long as its row at the border is zero, up to the net's degree. One
/// where the surface leaves the pole in a fan (its width grows as the
/// distance from the pole), two at a cusp (a profile that leaves the axis
/// along it: the width grows as the square of the distance). `None` where
/// it cannot be told: a surface without Bézier spans, a net that cannot
/// be formed, or a border whose net does not divide at all (it is no
/// point, to the rounding the division allows).
fn pole_order(surface: &Surface, pole: &Pole) -> Option<i32> {
    let at_start = matches!(pole.border, Border::UStart | Border::VStart);
    let mut least: Option<usize> = None;
    for span in spans_of(surface)?.iter() {
        if !touches(surface, pole, span) {
            continue;
        }
        let patch = if pole.across_v() {
            span.2.clone()
        } else {
            span.2.transposed()
        };
        let (mut along, _) = patch.weighted_derivative_nets()?;
        let mut degree = 2 * patch.degrees().1;
        let mut times = 0_usize;
        while degree > 0 {
            match divided(&along, degree, at_start) {
                Some(next) => {
                    along = next;
                    degree -= 1;
                    times += 1;
                }
                None => break,
            }
        }
        least = Some(least.map_or(times, |l| l.min(times)));
    }
    let order = i32::try_from(least?).ok()?;
    (order > 0).then_some(order)
}

/// For a point of a freeform surface that is a pole of it: the direction
/// the surface closes around there ([`pole_axes`]), or `None` inside when
/// it has none. `None` outside for any other point.
pub(super) fn pole_axis(
    surface: &Surface,
    tree: Option<&PatchTree>,
    u: f64,
    v: f64,
) -> Option<Option<Vec3>> {
    if !is_freeform(surface) {
        return None;
    }
    if let Some(tree) = tree {
        let index = tree.poles.iter().position(|p| p.holds(surface, u, v))?;
        return Some(tree.axes.get(index).copied().flatten());
    }
    let pole = craft3d_geom::pole::pole_at(surface, u, v)?;
    let all = poles(surface);
    let index = all.iter().position(|p| p.border == pole.border)?;
    Some(pole_axes(surface, &all).get(index).copied().flatten())
}

/// Whether a freeform surface has no border at all: it closes on itself
/// both ways (a torus), or one way with a pole at either end of the other
/// (a ball). A face without loops on it is the whole surface, as on a
/// sphere or a torus.
pub(super) fn borderless(surface: &Surface) -> bool {
    if !is_freeform(surface) {
        return false;
    }
    match closure(surface) {
        [true, true] => true,
        [false, false] => false,
        [around_u, _] => {
            let (start, end) = if around_u {
                (Border::VStart, Border::VEnd)
            } else {
                (Border::UStart, Border::UEnd)
            };
            let found = poles(surface);
            found.iter().any(|p| p.border == start) && found.iter().any(|p| p.border == end)
        }
    }
}

/// How a freeform surface that closes on itself is to be charted
/// (`chart.rs`, `Layout`): which way it closes, its poles, and how fast
/// its parameters can move.
#[derive(Clone, Copy, Debug)]
pub(super) struct Closure {
    /// It closes in u (the share `s` around is u's, the share `t` across
    /// is v's); else in v.
    pub around_u: bool,
    /// It closes in both (a torus).
    pub both: bool,
    /// A pole at the start and at the end of `t`.
    pub low: bool,
    pub high: bool,
    /// The order of the pole there is (1, or 2 at a cusp: see
    /// `pole_order`); 0 without poles.
    pub order: i32,
    /// Without poles: how far `(s, t)` can move per unit of length
    /// travelled on the surface, at most. With them: how far `(ω ds, dt)`
    /// can, `ω` the wedge's width (`t`, `1 − t` or their product): see
    /// `wedge_sigma`.
    pub speed: f64,
}

pub(super) const CLOSED_POLE: Reason = "a freeform face that closes on itself and into a point along the same parameter, or both ways round and into a point; its chart cannot be followed";

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
            // more from a tighter bound. What is returned must still be
            // below the distance: the nearest piece found so far is not,
            // when another piece not looked at yet is nearer (between the
            // threshold and what is asked of it, the bound would prove a
            // stretch clear that runs through the surface): the pieces
            // left are counted by their boxes.
            if threshold.is_finite() && best < threshold {
                return pending
                    .iter()
                    .map(|(_, slab, _)| distance(slab))
                    .fold(best, f64::min);
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

/// What a tree remembers of the answers to one kind of question, by the
/// bits of what was asked: at most [`MAX_MEMO`] answers, all let go when
/// full. How many it holds is kept beside the map, so that the shelf can
/// count what the tree takes without taking its lock.
#[derive(Debug)]
struct Memo<K, V> {
    map: Mutex<BTreeMap<K, V>>,
    len: std::sync::atomic::AtomicUsize,
}

impl<K, V> Default for Memo<K, V> {
    fn default() -> Self {
        Memo {
            map: Mutex::new(BTreeMap::new()),
            len: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl<K: Ord, V: Copy> Memo<K, V> {
    fn get(&self, key: &K) -> Option<V> {
        self.map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }

    fn put(&self, key: K, value: V) {
        let mut map = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        if map.len() >= MAX_MEMO {
            map.clear();
        }
        map.insert(key, value);
        self.len
            .store(map.len(), std::sync::atomic::Ordering::Relaxed);
    }

    /// About what the answers take: each key and answer, and as much
    /// again for the map's own nodes.
    fn bytes(&self) -> usize {
        self.len
            .load(std::sync::atomic::Ordering::Relaxed)
            .saturating_mul(2 * (std::mem::size_of::<K>() + std::mem::size_of::<V>()))
    }
}

/// What an operation under a work bound remembers for itself
/// (`craft3d_geom::allowance::scope`), in place of a tree's own memos:
/// those are shared with every operation that asked before, and an answer
/// found there costs nothing where finding it costs a search, so what an
/// operation counted would depend on what ran before it. Here a search is
/// counted, and remembered for the rest of the operation only. By the
/// tree's number ([`PatchTree::id`]) and the question.
#[derive(Default)]
struct ScopedMemos {
    scope: u64,
    nearest: BTreeMap<(u64, [u64; 3]), Option<Nearest>>,
    sigma: BTreeMap<(u64, [u64; 4]), Option<f64>>,
    bound: BTreeMap<(u64, [u64; 5]), f64>,
}

impl ScopedMemos {
    fn len(&self) -> usize {
        self.nearest.len() + self.sigma.len() + self.bound.len()
    }
}

thread_local! {
    static SCOPED: std::cell::RefCell<ScopedMemos> =
        std::cell::RefCell::new(ScopedMemos::default());
}

/// Lets go of what the operation that just ended remembered.
fn forget_scoped() {
    let _ = SCOPED.try_with(|memos| {
        if let Ok(mut memos) = memos.try_borrow_mut() {
            *memos = ScopedMemos::default();
        }
    });
}

/// The answer to a question of the operation of bound `scope`: remembered
/// by it (`find`), or searched (`search`, counted) and remembered
/// (`keep`). What another operation remembered is let go first. At most
/// [`MAX_MEMO`] answers of each kind, all let go when full (at the same
/// question of the operation whatever ran before it).
fn scoped<V: Copy>(
    scope: u64,
    find: impl Fn(&ScopedMemos) -> Option<V>,
    search: impl FnOnce() -> V,
    keep: impl FnOnce(&mut ScopedMemos, V),
) -> V {
    let found = SCOPED
        .try_with(|memos| {
            let mut memos = memos.try_borrow_mut().ok()?;
            if memos.scope != scope {
                *memos = ScopedMemos {
                    scope,
                    ..ScopedMemos::default()
                };
                craft3d_geom::allowance::at_scope_end(forget_scoped);
            }
            find(&memos)
        })
        .ok()
        .flatten();
    if let Some(found) = found {
        return found;
    }
    // Not borrowed while searching: a search may ask other questions.
    let found = search();
    let _ = SCOPED.try_with(|memos| {
        if let Ok(mut memos) = memos.try_borrow_mut()
            && memos.scope == scope
        {
            if memos.len() >= 3 * MAX_MEMO {
                *memos = ScopedMemos {
                    scope,
                    ..ScopedMemos::default()
                };
            }
            keep(&mut memos, found);
        }
    });
    found
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

/// Trees built lately, by the surface they index (written out in full, so
/// that two surfaces are the same key only when every number of them is
/// the same, bit for bit), with what each takes ([`PatchTree::bytes`]),
/// the most recently used last.
type Shelf = Vec<(String, Arc<PatchTree>)>;
static SHELF: Mutex<Shelf> = Mutex::new(Vec::new());
/// How many trees are kept, and how much memory all of them take at most
/// ([`PatchTree::bytes`]). A boolean holds the trees of every freeform
/// face of both operands while it works, so a body of many freeform faces
/// takes that much anyway; the shelf keeps the trees of such a body (the
/// impeller of the showcase: 62 freeform faces, some 860 000 nodes, about
/// 450 MB; with the trees of the operations before it 600 MB) for the next
/// operation on it. Smaller, the trees of one body
/// went round the shelf in the order they were asked for, each pushing
/// out the one asked for next, and every boolean on it built them all
/// again: most of the time of a chamfer far from the blades.
///
/// In a browser (WebAssembly) memory is scarcer: an eighth of the trees
/// and a sixth of the bytes.
#[cfg(not(target_arch = "wasm32"))]
const SHELF_TREES: usize = 1024;
#[cfg(not(target_arch = "wasm32"))]
const SHELF_BYTES: usize = 768 << 20;
#[cfg(target_arch = "wasm32")]
const SHELF_TREES: usize = 128;
#[cfg(target_arch = "wasm32")]
const SHELF_BYTES: usize = 128 << 20;

/// Lets go of the trees on the shelf whose surface is not the surface of a
/// face of any of `bodies` (the bodies something still holds: every open
/// document's), so that a big freeform model that was closed or edited
/// away does not keep its trees. Returns how many trees were let go. A
/// tree is a function of its surface alone, so this changes no result:
/// a tree let go is built again, the same, when it is asked for.
pub fn release_unused(bodies: &[&craft3d_topo::Body]) -> usize {
    let mut held = std::collections::BTreeSet::new();
    for body in bodies {
        for face in &body.faces {
            if is_freeform(&face.surface) {
                held.insert(format!("{:?}", face.surface));
            }
        }
    }
    let mut shelf = SHELF.lock().unwrap_or_else(PoisonError::into_inner);
    let before = shelf.len();
    shelf.retain(|(key, _)| held.contains(key));
    before.saturating_sub(shelf.len())
}

/// How many trees the shelf holds, and about how many bytes they take.
pub fn shelf_size() -> (usize, usize) {
    let shelf = SHELF.lock().unwrap_or_else(PoisonError::into_inner);
    (shelf.len(), shelf_bytes(&shelf))
}

/// What the trees on the shelf take now, what they remember included.
fn shelf_bytes(shelf: &Shelf) -> usize {
    shelf
        .iter()
        .map(|(_, tree)| tree.bytes())
        .fold(0_usize, usize::saturating_add)
}

/// Lets go of the trees asked for longest ago while the shelf holds more
/// trees or bytes than it may.
fn keep_within_budget(shelf: &mut Shelf) {
    let mut total = shelf_bytes(shelf);
    while shelf.len() > SHELF_TREES || total > SHELF_BYTES {
        if shelf.is_empty() {
            break;
        }
        let (_, forgotten) = shelf.remove(0);
        total = total.saturating_sub(forgotten.bytes());
    }
}

/// How much memory the trees one operation holds may take at most
/// ([`holding`]): their nodes and the pieces their leaves keep
/// ([`PatchTree::base_bytes`]), each tree counted once. A boolean holds the
/// tree of every freeform face of both operands it looks at while it
/// works, and each tree is capped ([`MAX_LEAVES`], about 22 MB), but their
/// number is not: a spur gear of twenty teeth drawn from formulas, its hub
/// covering the face the teeth end in, held 160 trees and 2.2 GB, and one
/// of a hundred teeth took a laptop's memory (2026-10-23). The impeller of
/// the showcase holds about 450 MB. A browser has less memory: 512 MiB
/// there, room for the impeller still (the developer, 2026-10-23).
#[cfg(not(target_arch = "wasm32"))]
pub(super) const HELD_BYTES: usize = 2 << 30;
#[cfg(target_arch = "wasm32")]
pub(super) const HELD_BYTES: usize = 512 << 20;
/// [`HELD_BYTES`].
pub(super) const TOO_MUCH_HELD: Reason = "the freeform faces it meets need more memory for their indexes than one operation may take (too many freeform faces, or faces of too many pieces)";

/// Freeform operations (outermost [`holding`]s) that run at once in the
/// whole process, at most; a thread that works in the background
/// (`craft3d_math::pool::alone`: a motion's builder) takes one fewer, so
/// the work someone waits for always finds a slot. [`HELD_BYTES`] bounds
/// one operation's trees, deterministically (it refuses the same whatever
/// else runs); this bounds how many such budgets are held at once, so
/// the trees of the whole process stay below `OPERATIONS_AT_ONCE ×
/// HELD_BYTES` plus the shelf ([`SHELF_BYTES`]): 6.75 GiB natively. An
/// operation beyond it waits for a slot, which changes when it runs,
/// never what it answers. Before 2026-11-02 the budget was per thread
/// only: a motion's three builder threads, the window's thread and the
/// pattern unions' helpers could each hold 2 GiB (an independent review).
/// No freeform operation waits for another thread while it holds a slot
/// (none starts threads), so the slots cannot deadlock.
pub const OPERATIONS_AT_ONCE: usize = 3;

struct Gate {
    running: Mutex<usize>,
    freed: std::sync::Condvar,
}

static GATE: Gate = Gate {
    running: Mutex::new(0),
    freed: std::sync::Condvar::new(),
};

/// A slot of [`OPERATIONS_AT_ONCE`], given back when dropped.
struct Slot;

impl Slot {
    /// Waits for a slot.
    fn take() -> Self {
        let limit = if craft3d_math::pool::alone() {
            OPERATIONS_AT_ONCE.saturating_sub(1).max(1)
        } else {
            OPERATIONS_AT_ONCE
        };
        let mut running = GATE.running.lock().unwrap_or_else(PoisonError::into_inner);
        while *running >= limit {
            running = GATE
                .freed
                .wait(running)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *running = running.saturating_add(1);
        Slot
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut running = GATE.running.lock().unwrap_or_else(PoisonError::into_inner);
        *running = running.saturating_sub(1);
        drop(running);
        GATE.freed.notify_all();
    }
}

/// How many freeform operations run now in the whole process (for tests
/// of [`OPERATIONS_AT_ONCE`]).
pub fn operations_running() -> usize {
    *GATE.running.lock().unwrap_or_else(PoisonError::into_inner)
}

thread_local! {
    /// The trees the operation running on this thread asked for
    /// ([`holding`]), by their numbers, and the bytes they take; `None`
    /// outside one.
    static HELD: std::cell::RefCell<Option<Held>> = const { std::cell::RefCell::new(None) };
    /// How deep in [`holding`] this thread is.
    static HOLDING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Runs `work` as one operation whose trees are held to [`HELD_BYTES`]
/// together: inside it, [`PatchTree::shared`] refuses ([`TOO_MUCH_HELD`])
/// once the distinct trees asked for take more, and builds no more. Each
/// tree counts by what it is made of, which depends on its surface alone,
/// whether it was built then or found on the shelf, so the refusal is the
/// same on every run and whatever ran before. Nested, the outermost counts.
pub(super) fn holding<T>(
    work: impl FnOnce() -> Result<T, crate::KernelError>,
) -> Result<T, crate::KernelError> {
    holding_within(HELD_BYTES, work)
}

/// The trees an operation asked for ([`holding`]).
struct Held {
    ids: std::collections::BTreeSet<u64>,
    bytes: usize,
    limit: usize,
    /// A tree was refused: the operation is refused whatever it made of
    /// that (a caller that takes a face it cannot outline for one that
    /// does not hold a point would otherwise answer wrongly).
    refused: bool,
}

/// [`holding`] with a budget of `limit` bytes.
///
/// Once a tree was refused within it, the outermost operation is refused
/// ([`TOO_MUCH_HELD`]) whatever its work returned: some callers take a
/// face they cannot outline as one that holds nothing (`.ok()`,
/// `is_ok_and`), which would turn the refusal into fewer contacts or a
/// wrong side.
fn holding_within<T>(
    limit: usize,
    work: impl FnOnce() -> Result<T, crate::KernelError>,
) -> Result<T, crate::KernelError> {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            let depth = HOLDING.with(|d| {
                let left = d.get().saturating_sub(1);
                d.set(left);
                left
            });
            if depth == 0 {
                HELD.with(|held| {
                    if let Ok(mut held) = held.try_borrow_mut() {
                        *held = None;
                    }
                });
            }
        }
    }
    let depth = HOLDING.with(|d| {
        let was = d.get();
        d.set(was.saturating_add(1));
        was
    });
    // Taken before `Leave`, so it is given back after the trees are let
    // go of.
    let _slot = (depth == 0).then(Slot::take);
    let _leave = Leave;
    if depth == 0 {
        HELD.with(|held| {
            if let Ok(mut held) = held.try_borrow_mut() {
                *held = Some(Held {
                    ids: std::collections::BTreeSet::new(),
                    bytes: 0,
                    limit,
                    refused: false,
                });
            }
        });
    }
    let done = work();
    if depth > 0 {
        return done;
    }
    let refused = HELD.with(|held| {
        held.try_borrow()
            .ok()
            .and_then(|h| h.as_ref().map(|held| held.refused))
            .unwrap_or(false)
    });
    match done {
        Ok(_) if refused => Err(crate::KernelError::Unsupported {
            operation: super::OPERATION,
            reason: TOO_MUCH_HELD,
            faces: Vec::new(),
        }),
        other => other,
    }
}

/// Inside [`holding`]: counts the tree `tree` (its number and bytes) when
/// given and not counted yet, and refuses when the trees counted take more
/// than [`HELD_BYTES`]. Outside, nothing.
fn held_within_budget(tree: Option<(u64, usize)>) -> Result<(), Reason> {
    HELD.with(|held| {
        let Ok(mut held) = held.try_borrow_mut() else {
            return Ok(());
        };
        let Some(held) = held.as_mut() else {
            return Ok(());
        };
        if let Some((id, size)) = tree
            && held.ids.insert(id)
        {
            held.bytes = held.bytes.saturating_add(size);
        }
        if held.bytes > held.limit {
            held.refused = true;
            Err(TOO_MUCH_HELD)
        } else {
            Ok(())
        }
    })
}

/// How many bytes of trees the operation running on this thread has asked
/// for so far ([`holding`]); zero outside one.
#[cfg(test)]
pub(super) fn held_bytes() -> usize {
    HELD.with(|held| {
        held.try_borrow()
            .ok()
            .and_then(|h| h.as_ref().map(|held| held.bytes))
            .unwrap_or(0)
    })
}

thread_local! {
    /// Trees this thread built ([`trees_built`]).
    static BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many trees [`PatchTree::shared`] built on this thread (not found on
/// the shelf) so far: for tests that hold a row of booleans on one body
/// to building each tree once.
#[cfg(test)]
pub(super) fn trees_built() -> usize {
    BUILT.with(std::cell::Cell::get)
}

impl PatchTree {
    /// The tree of `surface`, shared with the booleans before that asked
    /// for the same surface. A tree is a function of its surface alone, so
    /// one built earlier is the one that would be built now; a row of
    /// booleans on one body (each operand of the next the result of the
    /// last, its faces on the same surfaces) indexes each surface once.
    pub fn shared(surface: &Surface) -> Result<Arc<PatchTree>, Reason> {
        // Within an operation already over its budget of trees, none is
        // built or handed out ([`holding`]).
        held_within_budget(None)?;
        // Built uncounted, found or not; asking for a tree charges by its
        // nodes, the same either way (`craft3d_geom::allowance::uncounted`).
        let tree = craft3d_geom::allowance::uncounted(|| PatchTree::shelved(surface))?;
        held_within_budget(Some((tree.id, tree.base_bytes)))?;
        craft3d_geom::allowance::once(tree.id, tree.units);
        Ok(tree)
    }

    /// [`Self::shared`]: from the shelf, or built and put there.
    fn shelved(surface: &Surface) -> Result<Arc<PatchTree>, Reason> {
        let key = format!("{surface:?}");
        {
            let mut shelf = SHELF.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(at) = shelf.iter().position(|(k, _)| *k == key) {
                let entry = shelf.remove(at);
                let tree = Arc::clone(&entry.1);
                shelf.push(entry);
                // What the trees remember grew since they were put on the
                // shelf: counted again each time a tree is asked for.
                keep_within_budget(&mut shelf);
                return Ok(tree);
            }
        }
        // Built outside the lock: another thread may build the same tree
        // meanwhile, which is only work done twice.
        let tree = Arc::new(PatchTree::new(surface)?);
        BUILT.with(|built| built.set(built.get().saturating_add(1)));
        crate::work::patch_tree();
        if tree.bytes() <= SHELF_BYTES {
            let mut shelf = SHELF.lock().unwrap_or_else(PoisonError::into_inner);
            if !shelf.iter().any(|(k, _)| *k == key) {
                shelf.push((key, Arc::clone(&tree)));
            }
            keep_within_budget(&mut shelf);
        }
        Ok(tree)
    }

    /// About how much memory the tree takes now: its nodes and the pieces
    /// its leaves keep, and what it remembers (its memos and the fine
    /// pieces of its leaves), which grows as it is used, up to their caps.
    fn bytes(&self) -> usize {
        let fine = self
            .fine_pieces
            .load(std::sync::atomic::Ordering::Relaxed)
            .saturating_mul(self.fine_piece_bytes);
        self.base_bytes
            .saturating_add(self.memo.bytes())
            .saturating_add(self.sigma_memo.bytes())
            .saturating_add(self.bound_memo.bytes())
            .saturating_add(fine)
    }

    /// [`Self::base_bytes`] and [`Self::fine_piece_bytes`] of a tree just
    /// made.
    fn measure_base(&mut self) {
        let patch_bytes = |p: &RationalBezierPatch| {
            let (du, dv) = p.degrees();
            (du + 1)
                .saturating_mul(dv + 1)
                .saturating_mul(4 * std::mem::size_of::<f64>())
        };
        let leaves: usize = self
            .nodes
            .iter()
            .filter_map(|n| n.patch.as_ref())
            .map(patch_bytes)
            .fold(0_usize, usize::saturating_add);
        let largest = self
            .nodes
            .iter()
            .filter_map(|n| n.patch.as_ref())
            .map(patch_bytes)
            .max()
            .unwrap_or(0);
        self.base_bytes = self
            .nodes
            .len()
            .saturating_mul(std::mem::size_of::<Node>())
            .saturating_add(leaves);
        self.fine_piece_bytes = std::mem::size_of::<FinePiece>().saturating_add(largest);
    }

    pub fn new(surface: &Surface) -> Result<PatchTree, Reason> {
        // What building it took, kept with it, charged once per operation
        // that asks for it, built then or found on the shelf.
        let (made, units) = craft3d_geom::allowance::cost_of(|| PatchTree::built(surface));
        made.map(|mut tree| {
            tree.units = units;
            tree
        })
    }

    /// [`Self::new`]: the building.
    fn built(surface: &Surface) -> Result<PatchTree, Reason> {
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
        let found = poles(surface);
        let mut tree = PatchTree {
            surface: surface.clone(),
            nodes: Vec::new(),
            roots: Vec::new(),
            second,
            axes: match Corner::of(surface, &found) {
                Some(corner) => vec![corner_axis(surface, &corner); found.len()],
                None => pole_axes(surface, &found),
            },
            wedge: Wedge::of(surface, &found),
            closes: closure(surface),
            poles: found,
            memo: Memo::default(),
            sigma_memo: Memo::default(),
            bound_memo: Memo::default(),
            fine_pieces: std::sync::atomic::AtomicUsize::new(0),
            base_bytes: 0,
            fine_piece_bytes: 0,
            fine_cap: FINE_PIECES,
            halves: spans.len().saturating_mul(1 << (2 * depth)) > HALVED_FROM,
            fine: Mutex::new(Fine::default()),
            evened: std::sync::OnceLock::new(),
            id: craft3d_geom::allowance::number_of(&format!("{surface:?}")),
            units: 0,
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
        tree.measure_base();
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
        let sigma = match &self.wedge {
            None => sigma_bound(patch, (u.1 - u.0, v.1 - v.0)),
            Some(Ok(PoleChart::Wedge(wedge))) => wedge_sigma(patch, u, v, wedge),
            Some(Ok(PoleChart::Corner(corner))) => corner_sigma(patch, u, v, corner),
            Some(Err(_)) => 0.0,
        };
        let children = if depth > 0 && bbox.diagonal() > small {
            let um = 0.5 * (u.0 + u.1);
            let vm = 0.5 * (v.0 + v.1);
            let next = depth - 1;
            // Halved only across the direction it is long in when it is
            // short the other way ([`Kids`]): a strip along a fitted curve
            // (a flank of a gear tooth, 32 or more spans each a hundredth
            // of a millimetre wide and the face's width long) was quartered
            // into 1024 leaves a span, 22 MB a face.
            let (long_u, long_v) = if self.halves {
                long_ways(patch, small)
            } else {
                (true, true)
            };
            if long_u && !long_v {
                let (left, right) = patch.split_u(0.5);
                Some(Kids::two(
                    self.subdivide(&left, (u.0, um), v, next, small),
                    self.subdivide(&right, (um, u.1), v, next, small),
                ))
            } else if long_v && !long_u {
                let (low, high) = patch.split_v(0.5);
                Some(Kids::two(
                    self.subdivide(&low, u, (v.0, vm), next, small),
                    self.subdivide(&high, u, (vm, v.1), next, small),
                ))
            } else {
                let (left, right) = patch.split_u(0.5);
                let (ll, lh) = left.split_v(0.5);
                let (rl, rh) = right.split_v(0.5);
                Some(Kids::four([
                    self.subdivide(&ll, (u.0, um), (v.0, vm), next, small),
                    self.subdivide(&lh, (u.0, um), (vm, v.1), next, small),
                    self.subdivide(&rl, (um, u.1), (v.0, vm), next, small),
                    self.subdivide(&rh, (um, u.1), (vm, v.1), next, small),
                ]))
            }
        } else {
            None
        };
        // A piece's own bound is at least as good as its parent's; the
        // parent takes the least of its children's, which is tighter.
        let sigma = match children {
            Some(kids) => kids
                .ids()
                .iter()
                .filter_map(|&k| self.nodes.get(k).map(|n| n.sigma))
                .fold(f64::INFINITY, f64::min)
                .max(sigma),
            None => sigma,
        };
        // A leaf's bound, settled where it is far below the surface's.
        let sigma = if children.is_none() && self.wedge.is_none() {
            self.settled_sigma(patch, u, v, sigma)
        } else {
            sigma
        };
        // (A parent's `sigma` is at least the least of its children's, and
        // a leaf's pieces keep at least its own: the floor is below all.)
        let floor = match children {
            Some(kids) => kids
                .ids()
                .iter()
                .filter_map(|&k| self.nodes.get(k).map(|n| n.floor))
                .fold(f64::INFINITY, |a, b| {
                    if a.is_nan() || b.is_nan() {
                        f64::NAN
                    } else {
                        a.min(b)
                    }
                }),
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
            floor,
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

    /// The least of the bounds of the pieces: how far a point of the
    /// surface moves, at least, per unit that its chart moves; `None`
    /// when a piece has no bound.
    fn least_sigma(&self) -> Option<f64> {
        let sigma = self
            .roots
            .iter()
            .filter_map(|&r| self.nodes.get(r))
            .map(|n| n.sigma)
            .fold(f64::INFINITY, f64::min);
        (sigma.is_finite() && sigma > 0.0).then_some(sigma)
    }

    /// The tree of the same surface drawn as evenly as its weights allow
    /// ([`NurbsSurface::evened`]: each knot span reparametrised by the
    /// Möbius map that brings the weights at its ends together), or this
    /// tree where that changes nothing or gives no tree. A chart for
    /// winding numbers ([`super::chart::Chart`]): next to a knot where two
    /// spans are drawn at paces a thousand apart (rational weights far
    /// apart in a product form, as on a surface of revolution skewed
    /// span by span) the parameters move a thousand times faster on one
    /// side than on the other, and a stretch of a boundary along the knot
    /// takes the bound of the fast side; drawn evenly they do not. And
    /// with u drawn at the pace of v where the two are far apart
    /// ([`NurbsSurface::paces_matched`]): the chart's speed is the slower
    /// direction's, and a boundary along the faster one took as many
    /// times too many steps as the paces are apart (the sea kayak's plate
    /// rims, four millimetres by five metres: six thousand steps a
    /// winding, a seventh of a second for each point told in or out, kayak
    /// repro 25).
    ///
    /// What is remembered with the tree is what the surface gives: the
    /// evened tree, or none where the surface has no evened form or that
    /// has no tree. A refusal of the operation's budget ([`holding`]) is
    /// not remembered (it would leave later operations, in this process
    /// only, with the uneven chart: results that depend on what ran
    /// before); this call answers with this tree then, and the operation
    /// is refused anyway. An evened tree remembered counts against the
    /// budget of the operation that asks, as a tree found on the shelf.
    pub(super) fn evened(tree: &Arc<PatchTree>) -> Arc<PatchTree> {
        let made = craft3d_geom::allowance::uncounted(|| {
            if let Some(made) = tree.evened.get() {
                if let Some(even) = made {
                    // A refusal here marks the operation refused.
                    let _ = held_within_budget(Some((even.id, even.base_bytes)));
                }
                return made.clone();
            }
            let Surface::Nurbs(s) = &tree.surface else {
                return tree.evened.get_or_init(|| None).clone();
            };
            let evened = s.evened();
            let matched = evened.as_ref().unwrap_or(s).paces_matched();
            let Some(even) = matched.or(evened) else {
                return tree.evened.get_or_init(|| None).clone();
            };
            // On the shelf like any other tree, so that what it holds is
            // counted against the shelf's bytes.
            match PatchTree::shared(&Surface::Nurbs(even)) {
                Ok(even) => tree.evened.get_or_init(|| Some(even)).clone(),
                Err(TOO_MUCH_HELD) => None,
                Err(_) => tree.evened.get_or_init(|| None).clone(),
            }
        });
        let even = made.as_ref().map_or_else(|| Arc::clone(tree), Arc::clone);
        // Charged as asking for a tree is, made now or before.
        craft3d_geom::allowance::once(even.id, even.units);
        even
    }

    /// A lower bound on the smallest singular value of the Jacobian over
    /// the pieces of the surface that come within `reach` of `p`: the
    /// least of the bounds of the smallest pieces there. Where the
    /// parametrisation is far from uniform (rational weights far apart)
    /// this is what a curve near `p` needs, not the least over the whole
    /// surface, and never less than that. `None` where no piece is that
    /// near, or where a piece has no bound.
    pub(super) fn sigma_near(&self, p: Vec3, reach: f64) -> Option<f64> {
        // A search splits pieces of leaves: past the work bound of an
        // operation, when one is in force, the surface's own bound is the
        // answer (true, if less sharp).
        if craft3d_geom::allowance::exhausted() {
            return self.least_sigma();
        }
        let key = [p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), reach.to_bits()];
        if let Some(scope) = craft3d_geom::allowance::scope() {
            return scoped(
                scope,
                |m| m.sigma.get(&(self.id, key)).copied(),
                || self.sigma_search(p, reach),
                |m, found| {
                    m.sigma.insert((self.id, key), found);
                },
            );
        }
        if let Some(found) = self.sigma_memo.get(&key) {
            return found;
        }
        let found = self.sigma_search(p, reach);
        self.sigma_memo.put(key, found);
        found
    }

    /// [`Self::sigma_near`]: the search.
    fn sigma_search(&self, p: Vec3, reach: f64) -> Option<f64> {
        let mut least = f64::INFINITY;
        let mut stack: Vec<usize> = self.roots.clone();
        let mut visited = 0_usize;
        while let Some(index) = stack.pop() {
            visited += 1;
            if visited > 1 << 16 {
                return self.least_sigma();
            }
            let node = self.nodes.get(index)?;
            // Nothing under the node is lower than what was found: the
            // least stays what it is whatever is looked at there (every
            // node after it is asked with the same `least`). A search
            // whose reach takes in most of a surface went through every
            // node of its tree, 17 500 a question, 3000 questions in one
            // try of an offset of a chamfer on a torus (48 s; the fuzzer's
            // seed 20795706, 2026-10-21).
            if node.floor >= least {
                continue;
            }
            if !(box_distance(&node.bbox, p) <= reach) {
                continue;
            }
            // A piece wholly within reach takes its own bound (a bound
            // for all of it); one partly within reach is looked at closer.
            let inside = node.bbox.corners().is_some_and(|(lo, hi)| {
                let far = Vec3::new(
                    (p.x - lo.x).abs().max((hi.x - p.x).abs()),
                    (p.y - lo.y).abs().max((hi.y - p.y).abs()),
                    (p.z - lo.z).abs().max((hi.z - p.z).abs()),
                );
                far.length() <= reach
            });
            match node.children {
                Some(kids) if !inside => stack.extend(kids.ids()),
                Some(_) => least = least.min(node.sigma),
                // A leaf's bound may be far below the least singular value
                // over it: its pieces, split where that is so, are looked
                // at instead (those within reach only).
                None => match self.finer(index, node, p, reach, least) {
                    Some(sigma) => least = least.min(sigma),
                    None => least = least.min(node.sigma),
                },
            }
        }
        (least.is_finite() && least > 0.0).then_some(least)
    }

    /// The least of the bounds on the least singular value of the Jacobian
    /// over the parts of a leaf within `reach` of `p`, from its pieces, or
    /// a value of at least `cutoff` where none of them is below it. The
    /// pieces are taken lowest bound first, and the lowest is split (the
    /// split kept: [`Fine`]) while that can raise it; the first that
    /// cannot is the answer, since every other is at least as high. A
    /// piece is split where its box is more than [`FINE_REACH`] times as
    /// large as the reach (its box may come within reach where nothing of
    /// it does: the iso-curves of a patch whose weights are far apart fan
    /// out from a heavy corner, and the box of a narrow fan is wide), or
    /// where its bound is more than [`SIGMA_SLACK`] times below the least
    /// singular value measured at its corners and its middle: below the
    /// least of them for a piece wholly within reach (the bound pairs the
    /// shortest normal of a piece with its longest derivatives, and where
    /// the Jacobian changes by orders of magnitude within one leaf, next
    /// to a heavy corner, that is orders of magnitude below what any point
    /// of it has), below the largest of them for one that reaches out of
    /// it (the part within reach may have that much more). For a Jacobian
    /// that is the same all over a piece the bound is exact, so the slack
    /// goes as the pieces shrink. A piece keeps at least its parent's
    /// bound, which holds over it too. `None` on a surface drawn in
    /// another chart than its parameters (a pole's: the measured value is
    /// not of that map).
    fn finer(&self, index: usize, node: &Node, p: Vec3, reach: f64, cutoff: f64) -> Option<f64> {
        if self.wedge.is_some() {
            return None;
        }
        let patch = node.patch.as_ref()?;
        let mut guard = self.fine.lock().unwrap_or_else(PoisonError::into_inner);
        let fine = &mut *guard;
        fine.clock = fine.clock.wrapping_add(1);
        // The splits are kept only to be found again: the answer is the
        // same whatever was kept (a kept split this query would not make
        // is not looked into). Room for what this query may add is made
        // by letting go of the pieces of the leaves asked about longest
        // ago, a leaf's all at once: the queries come along the curves of
        // a boundary, near the last ones, and emptying the whole store
        // made the same pieces again and again (36.7 million of them in
        // one boolean of the helmet, for a store of 16,384).
        while fine.pieces.saturating_add(4 * FINE_LOOKS + 1) > self.fine_cap {
            let oldest = fine
                .roots
                .iter()
                .filter(|(leaf, _)| **leaf != index)
                .min_by_key(|(_, root)| root.used)
                .map(|(leaf, _)| *leaf)
                .or_else(|| fine.roots.contains_key(&index).then_some(index));
            let Some(oldest) = oldest else {
                fine.pieces = 0;
                break;
            };
            if let Some(gone) = fine.roots.remove(&oldest) {
                fine.pieces = fine.pieces.saturating_sub(gone.nodes.len());
            }
        }
        let clock = fine.clock;
        let root = match fine.roots.entry(index) {
            std::collections::btree_map::Entry::Occupied(found) => found.into_mut(),
            std::collections::btree_map::Entry::Vacant(free) => {
                let (mut piece, units) = craft3d_geom::allowance::uncounted(|| {
                    craft3d_geom::allowance::cost_of(|| {
                        self.fine_piece(patch.clone(), node.u, node.v, node.sigma, 0)
                    })
                });
                piece.units = units;
                fine.pieces = fine.pieces.saturating_add(1);
                free.insert(FineRoot {
                    nodes: vec![piece],
                    used: clock,
                })
            }
        };
        if let Some(first) = root.nodes.first() {
            self.charge_piece(first);
        }
        root.used = clock;
        let before = root.nodes.len();
        let answer = self.finer_in(&mut root.nodes, p, reach, cutoff);
        let added = root.nodes.len().saturating_sub(before);
        fine.pieces = fine.pieces.saturating_add(added);
        self.fine_pieces
            .store(fine.pieces, std::sync::atomic::Ordering::Relaxed);
        answer
    }

    /// Charges the fine piece over `u` × `v` (`craft3d_geom::allowance`):
    /// once per operation under a bound, made now or kept from before (the
    /// pieces a query splits are kept for the next, of this operation or
    /// another: what a query costs depends on what was kept, so the making
    /// is not counted, and each piece the operation goes into is, once).
    fn charge_piece(&self, piece: &FinePiece) {
        let (u, v) = (piece.u, piece.v);
        let thing = [u.0, u.1, v.0, v.1].iter().fold(self.id, |h, x| {
            (h ^ x.to_bits()).wrapping_mul(0x0100_0000_01b3)
        });
        craft3d_geom::allowance::once(thing, piece.units);
    }

    /// [`Self::finer`] in the pieces kept under one leaf, `nodes` (the
    /// leaf's own piece first), adding the pieces it splits.
    fn finer_in(
        &self,
        nodes: &mut Vec<FinePiece>,
        p: Vec3,
        reach: f64,
        cutoff: f64,
    ) -> Option<f64> {
        let root = 0_usize;
        // Lowest bound first; ties by where the piece is (not by its index
        // in the store, which depends on what was kept before), so that the
        // order is fixed.
        let mut heap = std::collections::BinaryHeap::new();
        let key = |piece: &FinePiece, at: usize| {
            std::cmp::Reverse((
                Ordered(piece.sigma),
                Ordered(piece.u.0),
                Ordered(piece.v.0),
                piece.depth,
                at,
            ))
        };
        let near = |piece: &FinePiece| box_distance(&piece.bbox, p) <= reach;
        if let Some(piece) = nodes.get(root).filter(|piece| near(piece)) {
            heap.push(key(piece, root));
        }
        let mut looks = 0_usize;
        while let Some(std::cmp::Reverse((Ordered(sigma), _, _, _, at))) = heap.pop() {
            looks += 1;
            if !(sigma < cutoff) || looks > FINE_LOOKS {
                return Some(sigma);
            }
            let Some(piece) = nodes.get(at) else {
                continue;
            };
            let inside = piece.bbox.corners().is_some_and(|(lo, hi)| {
                let far = Vec3::new(
                    (p.x - lo.x).abs().max((hi.x - p.x).abs()),
                    (p.y - lo.y).abs().max((hi.y - p.y).abs()),
                    (p.z - lo.z).abs().max((hi.z - p.z).abs()),
                );
                far.length() <= reach
            });
            let measured = if inside {
                piece.measured.0
            } else {
                piece.measured.1
            };
            let (um, vm) = (0.5 * (piece.u.0 + piece.u.1), 0.5 * (piece.v.0 + piece.v.1));
            let split = piece.depth < FINE_DEPTH
                && um > piece.u.0
                && um < piece.u.1
                && vm > piece.v.0
                && vm < piece.v.1
                && (piece.bbox.diagonal() > FINE_REACH * reach
                    || piece.sigma * SIGMA_SLACK < measured);
            if !split {
                return Some(sigma);
            }
            if let Some(kids) = piece.children {
                for k in kids {
                    if let Some(kid) = nodes.get(k) {
                        self.charge_piece(kid);
                    }
                }
                for k in kids {
                    if let Some(kid) = nodes.get(k).filter(|kid| near(kid)) {
                        heap.push(key(kid, k));
                    }
                }
                continue;
            }
            let Some(patch) = piece.patch.clone() else {
                return Some(sigma);
            };
            let (mut made, units) = craft3d_geom::allowance::uncounted(|| {
                craft3d_geom::allowance::cost_of(|| self.quartered(piece, &patch, sigma))
            });
            for kid in &mut made {
                kid.units = units / 4;
                self.charge_piece(kid);
            }
            let mut kids = [0_usize; 4];
            for (slot, made) in kids.iter_mut().zip(made) {
                *slot = nodes.len();
                if near(&made) {
                    heap.push(key(&made, *slot));
                }
                nodes.push(made);
            }
            if let Some(piece) = nodes.get_mut(at) {
                piece.children = Some(kids);
                piece.patch = None;
            }
        }
        // Nothing of the leaf within reach: it has nothing to add.
        Some(f64::INFINITY)
    }

    /// A leaf's bound on the least singular value of the Jacobian over all
    /// of it (at least `sigma`, its own): where it is more than
    /// [`SIGMA_SLACK`] times below the least singular value measured at the
    /// corners and the middle of a piece, the piece is halved, the way that
    /// raises the lower of its halves' bounds more, and the least of the
    /// pieces' bounds taken, within [`LEAF_PIECES`] pieces and
    /// [`FINE_DEPTH`] halvings each way. A leaf with no bound at all (none
    /// that clears the rounding of its nets: the normal of a rational piece
    /// is bounded through the cube of its heaviest weight, and with the
    /// weights ten thousand apart along a span that is a factor of 3e7
    /// within the leaf next to its light end) gets one where the surface
    /// has one, and the surface a parameter speed. (Where the
    /// parametrisation stalls the measured value is zero, nothing is
    /// halved, and the leaf has none, as before.)
    fn settled_sigma(
        &self,
        patch: &RationalBezierPatch,
        u: (f64, f64),
        v: (f64, f64),
        sigma: f64,
    ) -> f64 {
        let first = self.fine_piece(patch.clone(), u, v, sigma, 0);
        if !(first.sigma * SIGMA_SLACK < first.measured.0) {
            return sigma;
        }
        let mut least = f64::INFINITY;
        let mut stack = vec![(first, 0_usize, 0_usize)];
        let mut pieces = 1_usize;
        while let Some((piece, along_u, along_v)) = stack.pop() {
            let Some(patch) = piece.patch.as_ref() else {
                least = least.min(piece.sigma);
                continue;
            };
            let slack = piece.sigma * SIGMA_SLACK < piece.measured.0;
            if !slack || pieces.saturating_add(1) > LEAF_PIECES {
                least = least.min(piece.sigma);
                continue;
            }
            let (pu, pv, floor) = (piece.u, piece.v, piece.sigma);
            let (um, vm) = (0.5 * (pu.0 + pu.1), 0.5 * (pv.0 + pv.1));
            let halves_u = (along_u < FINE_DEPTH && um > pu.0 && um < pu.1)
                .then(|| self.halved(&piece, patch, true, floor));
            let halves_v = (along_v < FINE_DEPTH && vm > pv.0 && vm < pv.1)
                .then(|| self.halved(&piece, patch, false, floor));
            let lower = |halves: &Option<[FinePiece; 2]>| {
                halves
                    .as_ref()
                    .map_or(f64::NEG_INFINITY, |[a, b]| a.sigma.min(b.sigma))
            };
            let (by_u, by_v) = (lower(&halves_u), lower(&halves_v));
            let (halves, next) = if by_u >= by_v {
                (halves_u, (along_u + 1, along_v))
            } else {
                (halves_v, (along_u, along_v + 1))
            };
            let Some(halves) = halves else {
                least = least.min(piece.sigma);
                continue;
            };
            pieces += 1;
            for half in halves {
                stack.push((half, next.0, next.1));
            }
        }
        if least.is_finite() {
            least.max(sigma)
        } else {
            sigma
        }
    }

    /// A piece of a leaf for [`Self::finer`]: its box, its bound (at least
    /// `floor`, its parent's) and the least singular values at its corners
    /// and its middle, least and largest.
    fn fine_piece(
        &self,
        patch: RationalBezierPatch,
        u: (f64, f64),
        v: (f64, f64),
        floor: f64,
        depth: usize,
    ) -> FinePiece {
        self.fine_piece_with(patch, u, v, floor, depth, [None; 4])
    }

    /// The four quarters of `piece` (its patch `patch`), each at least
    /// `floor`: [`Self::fine_piece`] of each, with the values at their
    /// corners taken from the piece's corners and middle and from the
    /// middles of its sides (the same parameters, bit for bit).
    fn quartered(
        &self,
        piece: &FinePiece,
        patch: &RationalBezierPatch,
        floor: f64,
    ) -> [FinePiece; 4] {
        let (u, v, depth) = (piece.u, piece.v, piece.depth + 1);
        let (um, vm) = (0.5 * (u.0 + u.1), 0.5 * (v.0 + v.1));
        let [c00, c01, c10, c11, m] = piece.at.map(Some);
        let (left, right) = patch.split_u(0.5);
        let ((ll, lh), (rl, rh)) = (left.split_v(0.5), right.split_v(0.5));
        let side_l = Some(self.sigma_at(u.0, vm));
        let side_r = Some(self.sigma_at(u.1, vm));
        let side_b = Some(self.sigma_at(um, v.0));
        let side_t = Some(self.sigma_at(um, v.1));
        [
            (ll, (u.0, um), (v.0, vm), [c00, side_l, side_b, m]),
            (lh, (u.0, um), (vm, v.1), [side_l, c01, m, side_t]),
            (rl, (um, u.1), (v.0, vm), [side_b, m, c10, side_r]),
            (rh, (um, u.1), (vm, v.1), [m, side_t, side_r, c11]),
        ]
        .map(|(part, pu, pv, corners)| self.fine_piece_with(part, pu, pv, floor, depth, corners))
    }

    /// The two halves of `piece` across u (`along_u`) or across v, each at
    /// least `floor`, as [`Self::quartered`] makes its quarters.
    fn halved(
        &self,
        piece: &FinePiece,
        patch: &RationalBezierPatch,
        along_u: bool,
        floor: f64,
    ) -> [FinePiece; 2] {
        let (pu, pv) = (piece.u, piece.v);
        let (um, vm) = (0.5 * (pu.0 + pu.1), 0.5 * (pv.0 + pv.1));
        let [c00, c01, c10, c11, _] = piece.at.map(Some);
        if along_u {
            let (a, b) = patch.split_u(0.5);
            let (bottom, top) = (Some(self.sigma_at(um, pv.0)), Some(self.sigma_at(um, pv.1)));
            [
                self.fine_piece_with(a, (pu.0, um), pv, floor, 0, [c00, c01, bottom, top]),
                self.fine_piece_with(b, (um, pu.1), pv, floor, 0, [bottom, top, c10, c11]),
            ]
        } else {
            let (a, b) = patch.split_v(0.5);
            let (left, right) = (Some(self.sigma_at(pu.0, vm)), Some(self.sigma_at(pu.1, vm)));
            [
                self.fine_piece_with(a, pu, (pv.0, vm), floor, 0, [c00, left, c10, right]),
                self.fine_piece_with(b, pu, (vm, pv.1), floor, 0, [left, c01, right, c11]),
            ]
        }
    }

    /// The least singular value of the Jacobian at `(a, b)`.
    fn sigma_at(&self, a: f64, b: f64) -> f64 {
        let (_, su, sv) = self.surface.derivatives(a, b);
        let sum = su.length_squared() + sv.length_squared();
        least_singular(sum, su.cross(sv).length_squared())
    }

    /// [`Self::fine_piece`] with the values at its corners that are known
    /// already (those of the piece it was split from, at the same
    /// parameters, bit for bit), in the order of [`FinePiece::at`].
    fn fine_piece_with(
        &self,
        patch: RationalBezierPatch,
        u: (f64, f64),
        v: (f64, f64),
        floor: f64,
        depth: usize,
        corners: [Option<f64>; 4],
    ) -> FinePiece {
        let places = [
            (u.0, v.0),
            (u.0, v.1),
            (u.1, v.0),
            (u.1, v.1),
            (0.5 * (u.0 + u.1), 0.5 * (v.0 + v.1)),
        ];
        let mut at = [0.0; 5];
        let (mut least, mut most) = (f64::INFINITY, 0.0_f64);
        for (k, (slot, (a, b))) in at.iter_mut().zip(places).enumerate() {
            let known = corners.get(k).copied().flatten();
            let sigma = known.unwrap_or_else(|| self.sigma_at(a, b));
            *slot = sigma;
            least = least.min(sigma);
            most = most.max(sigma);
        }
        FinePiece {
            bbox: patch.control_bbox(),
            u,
            v,
            sigma: sigma_bound(&patch, (u.1 - u.0, v.1 - v.0)).max(floor),
            measured: (least, most),
            at,
            depth,
            patch: Some(patch),
            children: None,
            units: 0,
        }
    }

    /// How far the surface's parameters can change per unit of length
    /// travelled on it, at most; `None` when the surface may stall or fold
    /// (then its parameters are no chart of it: a surface with a pole
    /// stalls there, and has the chart of [`Self::chart`] instead).
    pub fn parameter_speed(&self) -> Option<f64> {
        if self.wedge.is_some() {
            return None;
        }
        self.least_sigma().map(|sigma| 1.0 / sigma)
    }

    /// How fast the chart of the surface can move per unit of length
    /// travelled on it, at most, or why the surface has no chart that can
    /// be followed.
    ///
    /// Without poles the chart is the parameters themselves
    /// ([`Self::parameter_speed`]). With them see [`Self::chart`]: the
    /// speed is the shear over the least singular value of the chart's
    /// inverse, bounded piece by piece, and the directions around every
    /// pole must be told apart (`pole_axes`).
    pub fn chart_speed(&self) -> Result<f64, Reason> {
        match &self.wedge {
            None => self.parameter_speed().ok_or(IRREGULAR),
            Some(Err(reason)) => Err(reason),
            Some(Ok(layout)) => {
                if self.axes.iter().any(Option::is_none) {
                    return Err(POLE_SHAPE);
                }
                let stretch = match layout {
                    PoleChart::Wedge(wedge) => wedge.shear(),
                    PoleChart::Corner(_) => CORNER_STRETCH,
                };
                self.least_sigma()
                    .map(|sigma| stretch / sigma)
                    .ok_or(IRREGULAR)
            }
        }
    }

    /// The chart coordinates of the point of the surface at `(u, v)`.
    ///
    /// A surface without poles is its own chart: its parameters. At a
    /// pole a whole side of the parameter rectangle is one point, and the
    /// rectangle is no chart of the surface there: a loop through the
    /// pole is not closed in it, and nothing bounds how fast the
    /// parameter along the border changes near it. So the rectangle is
    /// drawn as the surface is: with `s` and `t` the shares of the domain
    /// along and across the poles' borders, the point is put at
    /// `(ω(t) (s − ½), t)`, where the width `ω` is `t` for a pole at the
    /// start, `1 − t` for one at the end and `t (1 − t)` for both: a
    /// wedge, or a lens between two points. Every pole is one point of
    /// it, the map from the surface is one to one and continuous, and
    /// its speed is bounded wherever the surface leaves the pole in a
    /// fan (`wedge_sigma`).
    pub fn chart(&self, u: f64, v: f64) -> (f64, f64) {
        match &self.wedge {
            Some(Ok(PoleChart::Wedge(wedge))) => wedge.chart(u, v),
            Some(Ok(PoleChart::Corner(corner))) => corner.chart(u, v),
            _ => (u, v),
        }
    }

    /// How the surface closes on itself, or `None` when it does not (then
    /// [`Self::chart`] is its chart); an error when it closes in a way no
    /// chart is built for.
    ///
    /// The bound on the speed is that of [`Self::chart_speed`] in the
    /// shares of the domain: a step `(ds, dt)` is a step of the parameters
    /// `(W_u ds, W_v dt)`, at least `min W` times as long.
    pub fn closure(&self) -> Option<Result<Closure, Reason>> {
        let [in_u, in_v] = self.closes;
        if !(in_u || in_v) {
            return None;
        }
        Some((|| {
            let around_u = in_u;
            let both = in_u && in_v;
            let ((u0, u1), (v0, v1)) = self.surface.domain();
            let (low, high, order) = match &self.wedge {
                None => (false, false, 0),
                Some(Err(reason)) => return Err(*reason),
                // The poles must be on the borders across the way round:
                // of constant v when it closes in u.
                Some(Ok(PoleChart::Wedge(wedge))) if !both && wedge.across_v == around_u => {
                    // (A cusp at either end of a surface with a pole at
                    // each has no chart drawn for it: see `chart.rs`.)
                    if wedge.low && wedge.high && wedge.orders.0.max(wedge.orders.1) > 1 {
                        return Err(POLE_SHAPE);
                    }
                    (wedge.low, wedge.high, wedge.orders.0.max(wedge.orders.1))
                }
                Some(Ok(_)) => return Err(CLOSED_POLE),
            };
            let sigma = self.least_sigma().ok_or(IRREGULAR)?;
            let speed = if low || high {
                if self.axes.iter().any(Option::is_none) {
                    return Err(POLE_SHAPE);
                }
                1.0 / sigma
            } else {
                let narrowest = (u1 - u0).min(v1 - v0);
                if !(narrowest > 0.0) {
                    return Err(IRREGULAR);
                }
                1.0 / (sigma * narrowest)
            };
            if !(speed.is_finite() && speed > 0.0) {
                return Err(IRREGULAR);
            }
            Ok(Closure {
                around_u,
                both,
                low,
                high,
                order,
                speed,
            })
        })())
    }

    /// The parameters as shares of the domain, `(s, t)`: around and
    /// across the way the surface closes (see [`Closure`]).
    pub fn shares(&self, around_u: bool, u: f64, v: f64) -> (f64, f64) {
        let ((u0, u1), (v0, v1)) = self.surface.domain();
        let (su, sv) = ((u - u0) / (u1 - u0), (v - v0) / (v1 - v0));
        if around_u { (su, sv) } else { (sv, su) }
    }

    /// The point of the surface at the shares `(s, t)`.
    pub fn at_shares(&self, around_u: bool, s: f64, t: f64) -> Vec3 {
        let ((u0, u1), (v0, v1)) = self.surface.domain();
        let (su, sv) = if around_u { (s, t) } else { (t, s) };
        self.surface.eval(u0 + su * (u1 - u0), v0 + sv * (v1 - v0))
    }

    /// The size of the surface: the diagonal of the box around it.
    pub fn size(&self) -> f64 {
        let mut all = Bbox3::EMPTY;
        for node in self.roots.iter().filter_map(|&r| self.nodes.get(r)) {
            if let Some((lo, hi)) = node.bbox.corners() {
                all.add(lo);
                all.add(hi);
            }
        }
        all.diagonal()
    }

    /// The unit normal `∂u × ∂v` of the surface at the nearest point
    /// `near` of `p`; at a pole, the one of the meridian that leaves it
    /// nearest to the direction of `p`, which is the one that tells the
    /// side `p` is on (`craft3d_geom::pole::Pole::normal_towards`).
    fn normal_for(&self, near: &Nearest, p: Vec3) -> Option<Vec3> {
        match self
            .poles
            .iter()
            .find(|pole| pole.holds(&self.surface, near.u, near.v))
        {
            Some(pole) => pole.normal_towards(&self.surface, p),
            None => self.surface.normal(near.u, near.v),
        }
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
        let key = [
            p.x.to_bits(),
            p.y.to_bits(),
            p.z.to_bits(),
            resolution.to_bits(),
            threshold.to_bits(),
        ];
        let search = || {
            self.bound_by(
                &|slab| box_distance(&slab.local, slab.to_local(p)),
                resolution,
                MAX_LEAVES,
                threshold,
            )
        };
        if let Some(scope) = craft3d_geom::allowance::scope() {
            return scoped(
                scope,
                |m| m.bound.get(&(self.id, key)).copied(),
                search,
                |m, found| {
                    m.bound.insert((self.id, key), found);
                },
            );
        }
        if let Some(found) = self.bound_memo.get(&key) {
            return found;
        }
        let found = search();
        self.bound_memo.put(key, found);
        found
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
                    for &k in kids.ids() {
                        if let Some(child) = self.nodes.get(k) {
                            heap.push(Pending(distance(&Slab::axis_aligned(child.bbox)), k));
                        }
                    }
                }
                None => match &node.patch {
                    Some(patch) => {
                        // The leaf's own box is in its frame, thinner than
                        // the box of the tree that found it (kept with the
                        // leaf, or made again).
                        let slab = node.slab.clone().unwrap_or_else(|| Slab::around(patch));
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
                // (As in `refine`: what is left is counted by the nearest
                // box of it, so that the bound stays below the distance.)
                if let Some(Pending(next, _)) = heap.peek() {
                    best = best.min(*next);
                }
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
        // Each step evaluates the surface at three values of each parameter
        // (the curvature from differences of the tangents): the blend
        // factors of a spline kept for them, the same bits.
        let cache = std::cell::RefCell::new(craft3d_geom::DerivativeCache::default());
        let derivatives = |u: f64, v: f64| match &self.surface {
            Surface::Nurbs(s) => s.derivatives_cached(u, v, &mut cache.borrow_mut()),
            surface => surface.derivatives(u, v),
        };
        let (mut u, mut v) = self.clamp(start.0, start.1);
        let (mut point, _, _) = derivatives(u, v);
        let mut distance = point.distance(p);
        let mut damping = 1e-6;
        let (hu, hv) = (1e-6 * (u1 - u0), 1e-6 * (v1 - v0));
        // Whether the curvature terms are taken from the start of a step.
        let mut curved = false;
        for _ in 0..NEWTON_STEPS {
            let (s, su, sv) = derivatives(u, v);
            let r = s - p;
            let (g1, g2) = (su.dot(r), sv.dot(r));
            if !(g1.phypot(g2) > 0.0) {
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
                let (_, su_up, sv_up) = derivatives((u + hu).min(u1), v);
                let (_, su_down, sv_down) = derivatives((u - hu).max(u0), v);
                let (_, _, sv_vup) = derivatives(u, (v + hv).min(v1));
                let (_, _, sv_vdown) = derivatives(u, (v - hv).max(v0));
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
                    let step = (nu - u).phypot(nv - v);
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

    /// The point of the surface nearest to `p`, remembered.
    pub fn nearest(&self, p: Vec3) -> Option<Nearest> {
        let key = [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
        if let Some(scope) = craft3d_geom::allowance::scope() {
            return scoped(
                scope,
                |m| m.nearest.get(&(self.id, key)).copied(),
                || self.search(p),
                |m, found| {
                    m.nearest.insert((self.id, key), found);
                },
            );
        }
        if let Some(found) = self.memo.get(&key) {
            return found;
        }
        let found = self.search(p);
        self.memo.put(key, found);
        found
    }

    /// The point of the surface nearest to `p`: the search.
    fn search(&self, p: Vec3) -> Option<Nearest> {
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
                    for &k in kids.ids() {
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
        let normal = self.normal_for(&near, p)?;
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

    /// At most [`OPERATIONS_AT_ONCE`] freeform operations run at once in
    /// the whole process, and one fewer of those of background threads;
    /// the others wait and then run (2026-11-02).
    #[test]
    fn freeform_operations_at_once_are_bounded_process_wide() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for background in [false, true] {
            let inside = AtomicUsize::new(0);
            let most = AtomicUsize::new(0);
            let most_anywhere = AtomicUsize::new(0);
            let done = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for _ in 0..8 {
                    scope.spawn(|| {
                        if background {
                            craft3d_math::pool::work_alone();
                        }
                        let ran = holding(|| {
                            let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                            most.fetch_max(now, Ordering::SeqCst);
                            most_anywhere.fetch_max(operations_running(), Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(20));
                            inside.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        });
                        assert!(ran.is_ok());
                        done.fetch_add(1, Ordering::SeqCst);
                    });
                }
            });
            assert_eq!(done.load(Ordering::SeqCst), 8);
            let limit = if background {
                OPERATIONS_AT_ONCE - 1
            } else {
                OPERATIONS_AT_ONCE
            };
            assert!(most.load(Ordering::SeqCst) <= limit, "{background}");
            assert!(most.load(Ordering::SeqCst) >= 1);
            assert!(most_anywhere.load(Ordering::SeqCst) <= OPERATIONS_AT_ONCE);
        }
    }

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

    /// A row of booleans on a body of many freeform faces asks for the
    /// trees of all of them every time, in the same order: the shelf keeps
    /// them all, so the second round builds none (with room for 32 trees
    /// the first round pushed out each tree before it was asked for again,
    /// and every boolean built them all).
    #[test]
    fn the_trees_of_a_body_of_many_freeform_faces_are_built_once() {
        let surfaces: Vec<Surface> = (0..48)
            .map(|k| {
                let frame = Frame::WORLD
                    .at(Vec3::new(10.0 * f64::from(k), 0.0, 0.0))
                    .unwrap();
                let roof = craft3d_testkit::trimmed::domed_block(&frame, 4.0, 3.0, 2.0, k % 2 == 0)
                    .unwrap()
                    .body;
                roof.faces
                    .iter()
                    .map(|f| f.surface.clone())
                    .find(is_freeform)
                    .unwrap()
            })
            .collect();
        let before = trees_built();
        for surface in &surfaces {
            PatchTree::shared(surface).unwrap();
        }
        let first = trees_built() - before;
        assert!(first <= surfaces.len(), "{first}");
        for _round in 0..2 {
            let before = trees_built();
            for surface in &surfaces {
                PatchTree::shared(surface).unwrap();
            }
            assert_eq!(trees_built() - before, 0);
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
            (false, std::f64::consts::FRAC_PI_2.pcos()),
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

    /// What a tree remembers counts in what it takes (the shelf's budget):
    /// its bytes grow with the answers it keeps and the fine pieces of its
    /// leaves, and stay below what the caps allow.
    #[test]
    fn what_a_tree_remembers_counts_in_its_bytes() {
        let tree = PatchTree::new(&dome()).unwrap();
        let bare = tree.bytes();
        assert_eq!(bare, tree.base_bytes);
        for k in 0..300 {
            let t = f64::from(k) / 299.0;
            let p = Vec3::new(4.0 * t, 3.0 * (1.0 - t), 1.0 + t);
            let _ = tree.nearest(p);
            let _ = tree.lower_bound(p, 1e-3, 0.1);
            let _ = tree.sigma_near(p, 10f64.ppowi(-(k % 5)));
        }
        let used = tree.bytes();
        assert!(used > bare, "{used} {bare}");
        let memo = |k: usize, v: usize| MAX_MEMO * 2 * (k + v);
        let most =
            bare + memo(
                std::mem::size_of::<[u64; 3]>(),
                std::mem::size_of::<Option<Nearest>>(),
            ) + memo(
                std::mem::size_of::<[u64; 4]>(),
                std::mem::size_of::<Option<f64>>(),
            ) + memo(std::mem::size_of::<[u64; 5]>(), std::mem::size_of::<f64>())
                + FINE_PIECES * tree.fine_piece_bytes;
        assert!(used <= most);
    }

    /// The quarters and halves of a fine piece take the values at their
    /// corners from the piece; each is the piece [`PatchTree::fine_piece`]
    /// makes of the same patch and range afresh, bit for bit, on a rational
    /// patch with weights far apart and on the dome, three levels down.
    #[test]
    fn quarters_and_halves_are_the_pieces_made_afresh() {
        let skewed = Surface::Nurbs(
            NurbsSurface::new(
                2,
                1,
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                3,
                2,
                vec![
                    Vec3::new(0.0, 0.0, 2.0),
                    Vec3::new(0.0, 3.0, 2.0),
                    Vec3::new(2.0, 0.0, 2.7),
                    Vec3::new(2.0, 3.0, 1.6),
                    Vec3::new(4.0, 0.0, 2.0),
                    Vec3::new(4.0, 3.0, 2.0),
                ],
                vec![1.0, 700.0, 30.0, 1.0, 1.0, 1e3],
            )
            .unwrap(),
        );
        let same = |a: &FinePiece, b: &FinePiece, what: &str| {
            assert_eq!(format!("{a:?}"), format!("{b:?}"), "{what}");
        };
        for surface in [skewed, dome()] {
            let tree = PatchTree::new(&surface).unwrap();
            let Some(leaf) = tree.nodes.iter().find(|n| n.patch.is_some()) else {
                panic!("a leaf with its patch");
            };
            let patch = leaf.patch.clone().unwrap();
            let mut pieces = vec![tree.fine_piece(patch, leaf.u, leaf.v, 0.25 * leaf.sigma, 0)];
            for level in 0..3 {
                let mut next = Vec::new();
                for piece in &pieces {
                    let patch = piece.patch.clone().unwrap();
                    let floor = piece.sigma * 0.5;
                    let quarters = tree.quartered(piece, &patch, floor);
                    let (left, right) = patch.split_u(0.5);
                    let ((ll, lh), (rl, rh)) = (left.split_v(0.5), right.split_v(0.5));
                    let (u, v) = (piece.u, piece.v);
                    let (um, vm) = (0.5 * (u.0 + u.1), 0.5 * (v.0 + v.1));
                    for (got, (part, pu, pv)) in quarters.iter().zip([
                        (ll, (u.0, um), (v.0, vm)),
                        (lh, (u.0, um), (vm, v.1)),
                        (rl, (um, u.1), (v.0, vm)),
                        (rh, (um, u.1), (vm, v.1)),
                    ]) {
                        let want = tree.fine_piece(part, pu, pv, floor, piece.depth + 1);
                        same(got, &want, &format!("quarter at level {level}"));
                    }
                    for along_u in [true, false] {
                        let halves = tree.halved(piece, &patch, along_u, floor);
                        let (a, b, ra, rb) = if along_u {
                            let (a, b) = patch.split_u(0.5);
                            (a, b, ((u.0, um), v), ((um, u.1), v))
                        } else {
                            let (a, b) = patch.split_v(0.5);
                            (a, b, (u, (v.0, vm)), (u, (vm, v.1)))
                        };
                        for (got, (part, (pu, pv))) in halves.iter().zip([(a, ra), (b, rb)]) {
                            let want = tree.fine_piece(part, pu, pv, floor, 0);
                            same(got, &want, &format!("half at level {level}"));
                        }
                    }
                    next.extend(quarters);
                }
                pieces = next;
            }
        }
    }

    /// The fine store lets go of the leaves asked about longest ago when
    /// full: whatever it let go of, every answer is the one a tree that
    /// never kept anything gives, bit for bit, and it counts what it holds
    /// right. With room for one query only (everything let go before each),
    /// a little more (some leaves kept), and the store of a tree in use.
    #[test]
    fn a_full_fine_store_lets_go_of_old_leaves_and_answers_the_same() {
        let plate = Surface::Nurbs(
            NurbsSurface::new(
                1,
                1,
                vec![0.0, 0.0, 0.5, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                3,
                2,
                vec![
                    Vec3::new(0.0, 0.0, 2.0),
                    Vec3::new(0.0, 3.0, 2.0),
                    Vec3::new(2.0, 0.0, 2.3),
                    Vec3::new(2.0, 3.0, 1.8),
                    Vec3::new(4.0, 0.0, 2.0),
                    Vec3::new(4.0, 3.0, 2.0),
                ],
                vec![1.0, 1000.0, 300.0, 1.0, 1.0, 1000.0],
            )
            .unwrap(),
        );
        let mut seed = 20_261_018_u64;
        let mut unit = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let questions: Vec<(Vec3, f64)> = (0..400)
            .map(|_| {
                let p = Vec3::new(4.0 * unit(), 3.0 * unit(), 1.8 + 0.5 * unit());
                (p, 10f64.ppowf(-6.0 * unit()))
            })
            .collect();
        let room = 4 * FINE_LOOKS + 1;
        let mut most = 0;
        for cap in [room, room + 300, FINE_PIECES] {
            let mut kept = PatchTree::new(&plate).unwrap();
            kept.fine_cap = cap;
            let mut let_go = 0;
            let mut before = 0;
            for &(p, r) in &questions {
                let fresh = PatchTree::new(&plate).unwrap();
                let got = kept.sigma_search(p, r);
                let want = fresh.sigma_search(p, r);
                assert_eq!(
                    got.map(f64::to_bits),
                    want.map(f64::to_bits),
                    "{cap}: {p:?} {r}"
                );
                let fine = kept.fine.lock().unwrap();
                let held: usize = fine.roots.values().map(|root| root.nodes.len()).sum();
                assert_eq!(fine.pieces, held, "{cap}");
                assert!(fine.pieces <= cap, "{cap}: {}", fine.pieces);
                assert_eq!(
                    kept.fine_pieces.load(std::sync::atomic::Ordering::Relaxed),
                    held
                );
                // Fewer than before the query: leaves were let go.
                if held < before {
                    let_go += 1;
                }
                before = held;
                most = most.max(held);
            }
            if cap < FINE_PIECES {
                assert!(let_go > 0, "{cap}: nothing was let go");
            }
        }
        // The queries need more than the small stores hold.
        assert!(most > room + 300, "{most}");
    }

    #[test]
    fn the_bound_near_a_point_does_not_depend_on_what_was_asked_before() {
        // Review of 2026-10-17: `finer` keeps the splits it makes, and the
        // answer to a question must be the same whatever was asked of the
        // tree before (rule 5: a document regenerated step by step and the
        // same document loaded from a file give the same bodies).
        let plate = |heavy: f64| {
            Surface::Nurbs(
                NurbsSurface::new(
                    1,
                    1,
                    vec![0.0, 0.0, 1.0, 1.0],
                    vec![0.0, 0.0, 1.0, 1.0],
                    2,
                    2,
                    vec![
                        Vec3::new(0.0, 0.0, 2.0),
                        Vec3::new(0.0, 3.0, 2.0),
                        Vec3::new(4.0, 0.0, 2.0),
                        Vec3::new(4.0, 3.0, 2.0),
                    ],
                    vec![1.0, heavy, heavy, 1.0],
                )
                .unwrap(),
            )
        };
        let questions: Vec<(Vec3, f64)> = (0..60)
            .map(|k| {
                let t = f64::from(k) / 59.0;
                let p = match k % 3 {
                    0 => Vec3::new(4.0 * t, 0.0, 2.0),
                    1 => Vec3::new(0.0, 3.0 * t, 2.0),
                    _ => Vec3::new(4.0 * t, 3.0 * (1.0 - t), 2.0),
                };
                (p, 10f64.ppowi(-(k % 7)))
            })
            .collect();
        let surface = plate(1000.0);
        let first = PatchTree::new(&surface).unwrap();
        let answers: Vec<Option<f64>> = questions
            .iter()
            .map(|&(p, r)| first.sigma_near(p, r))
            .collect();
        let second = PatchTree::new(&surface).unwrap();
        for &(p, r) in questions.iter().rev() {
            let _ = second.sigma_near(p * 1.0001, r * 3.0);
            let _ = second.sigma_near(p, r);
        }
        for (&(p, r), want) in questions.iter().zip(&answers) {
            let got = second.sigma_near(p, r);
            assert_eq!(got.map(f64::to_bits), want.map(f64::to_bits), "{p:?} {r}");
            assert!(got.is_some_and(|g| g > 0.0), "{p:?} {r}");
        }
    }

    /// The tree of a surface drawn more evenly counts the same toward an
    /// operation's bound made now or found made: building it asks for its
    /// tree inside an uncounted region, which must not mark it charged.
    #[test]
    fn an_evened_tree_counts_the_same_made_now_or_before() {
        use craft3d_geom::allowance;
        let surface = Surface::Nurbs(
            NurbsSurface::new(
                1,
                1,
                vec![0.0, 0.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                2,
                2,
                vec![
                    Vec3::new(0.0, 0.0, 1.0),
                    Vec3::new(0.0, 3.0, 1.2),
                    Vec3::new(4.0, 0.0, 0.8),
                    Vec3::new(4.0, 3.0, 1.0),
                ],
                vec![1.0, 1.0, 4.0, 4.0],
            )
            .unwrap(),
        );
        let count = || {
            allowance::within(u64::MAX, || {
                allowance::measured(|| {
                    let tree = PatchTree::shared(&surface).unwrap();
                    let even = PatchTree::evened(&tree);
                    assert!(!Arc::ptr_eq(&tree, &even), "nothing to even");
                })
                .1
            })
        };
        let cold = count();
        let warm = count();
        assert!(cold > 0);
        assert_eq!(cold, warm);
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
        // (A thousand until 2026-10-17, when the bounds came to be taken
        // piece by piece where the weights are uneven: expectation changed,
        // the twins skewed a thousandfold both ways, 1.4e6 within a span,
        // are held in `boolean_rational.rs`.)
        assert!(PatchTree::new(&surface(1100.0)).is_ok());
        assert!(PatchTree::new(&surface(1.9e6)).is_ok());
        assert_eq!(PatchTree::new(&surface(2.1e6)).unwrap_err(), WEIGHTS);
        assert_eq!(PatchTree::new(&surface(1.0 / 2.1e6)).unwrap_err(), WEIGHTS);
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

    /// The sea kayak's tips (2026-10-27): a ray 2600 mm long through a
    /// flat leaf of a ruled spline face read as 1.5e-5 mm clear of it, so
    /// the face's crossing was not counted and points outside the deck's
    /// cylinder classified inside. A segment through a flat (or nearly
    /// flat) box is at distance zero from it, at every scale.
    #[test]
    fn a_segment_through_a_flat_box_far_from_its_start_meets_it() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for scale in [1e-3, 1.0, 1e6] {
            let mut worst = 0.0_f64;
            for thick in [0.0, 1e-9] {
                for _ in 0..500 {
                    let mut at = |lo: f64, hi: f64| scale * (lo + (hi - lo) * next());
                    let z = at(-500.0, 500.0);
                    let bbox = Bbox3::from_points([
                        Vec3::new(-1000.0 * scale, -1000.0 * scale, z),
                        Vec3::new(1000.0 * scale, 1000.0 * scale, z + thick * scale),
                    ]);
                    let a = Vec3::new(at(-900.0, 900.0), at(-900.0, 900.0), z - at(100.0, 2600.0));
                    let b = Vec3::new(at(-900.0, 900.0), at(-900.0, 900.0), z + at(100.0, 2600.0));
                    worst = worst.max(segment_box_distance(&bbox, a, b));
                    // Either way round.
                    worst = worst.max(segment_box_distance(&bbox, b, a));
                }
            }
            // Rounding of the coordinates only (before: up to 4e-8 of
            // the scale, for these sizes).
            assert!(worst <= 1e-11 * scale, "{worst} at scale {scale}");
        }
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

    /// The quarter of a surface of revolution about the frame's z axis
    /// that a rational quadratic profile (ρ, z, weight) sweeps: u around
    /// the axis, v along the profile; `flip` exchanges the two.
    fn swept(profile: &[(f64, f64, f64)], knots: Vec<f64>, flip: bool) -> Surface {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let ring = [(1.0, 0.0, 1.0), (1.0, 1.0, s), (0.0, 1.0, 1.0)];
        let frame = Frame::new(
            Vec3::new(1.0, -2.0, 0.5),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap();
        let point = |i: usize, j: usize| {
            let ((x, y, wu), (rho, z, wv)) = (ring[i], profile[j]);
            (frame.point(x * rho, y * rho, z), wu * wv)
        };
        let around = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
        let n = profile.len();
        let surface = if flip {
            for j in 0..n {
                for i in 0..3 {
                    let (p, w) = point(i, j);
                    ctrl.push(p);
                    weights.push(w);
                }
            }
            NurbsSurface::new(2, 2, knots, around, n, 3, ctrl, weights)
        } else {
            for i in 0..3 {
                for j in 0..n {
                    let (p, w) = point(i, j);
                    ctrl.push(p);
                    weights.push(w);
                }
            }
            NurbsSurface::new(2, 2, around, knots, 3, n, ctrl, weights)
        };
        Surface::Nurbs(surface.unwrap())
    }

    /// A rational patch of bidegree (3, 3) from homogeneous coordinates
    /// `(x w, y w, z w, w)` that are polynomials of bidegree (2, 2) at
    /// most in `(s, t)`, placed on the frame of [`swept`]: the Bernstein
    /// coefficients of degree 2 from the values at 0, ½ and 1, raised by
    /// one degree (which makes every weight of the lune positive). With
    /// `turn`, the parameters are exchanged.
    fn bidegree_two(f: &dyn Fn(f64, f64) -> [f64; 4], turn: bool) -> Surface {
        let frame = Frame::new(
            Vec3::new(1.0, -2.0, 0.5),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap();
        let at = [0.0, 0.5, 1.0];
        let value = |i: usize, j: usize| {
            if turn {
                f(at[j], at[i])
            } else {
                f(at[i], at[j])
            }
        };
        let solve = |v: [[f64; 4]; 3]| {
            let mut c = [[0.0; 4]; 3];
            for k in 0..4 {
                c[0][k] = v[0][k];
                c[2][k] = v[2][k];
                c[1][k] = 2.0 * v[1][k] - 0.5 * (v[0][k] + v[2][k]);
            }
            c
        };
        let mut half = [[[0.0; 4]; 3]; 3];
        for (i, row) in half.iter_mut().enumerate() {
            *row = solve([value(i, 0), value(i, 1), value(i, 2)]);
        }
        let mut two = [[[0.0; 4]; 3]; 3];
        for j in 0..3 {
            let c = solve([half[0][j], half[1][j], half[2][j]]);
            for i in 0..3 {
                two[i][j] = c[i];
            }
        }
        // Raised: c'_k = k/3 c_{k−1} + (1 − k/3) c_k, both ways.
        let raise = |c: &dyn Fn(usize) -> [f64; 4], k: usize| {
            let a = k as f64 / 3.0;
            let low = if k > 0 { c(k - 1) } else { [0.0; 4] };
            let high = if k < 3 { c(k) } else { [0.0; 4] };
            [0, 1, 2, 3].map(|x| a * low[x] + (1.0 - a) * high[x])
        };
        let mut rows = [[[0.0; 4]; 3]; 4];
        for (i, row) in rows.iter_mut().enumerate() {
            for (j, slot) in row.iter_mut().enumerate() {
                *slot = raise(&|k| two[k][j], i);
            }
        }
        let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
        for row in &rows {
            for j in 0..4 {
                let h = raise(&|k| row[k], j);
                assert!(h[3] > 0.0);
                ctrl.push(frame.point(h[0] / h[3], h[1] / h[3], h[2] / h[3]));
                weights.push(h[3]);
            }
        }
        let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        Surface::Nurbs(NurbsSurface::new(3, 3, knots.clone(), knots, 4, 4, ctrl, weights).unwrap())
    }

    /// The lune of the ball of radius 2 about the frame's origin with x
    /// and y positive, by inverse stereographic projection from the north
    /// pole of `((1 − s) / (s t), (1 − t) / (s t))`: the north pole on
    /// the two borders `s = 0` and `t = 0`, read from the start or the end
    /// of each parameter (`back`).
    fn corner_lune(back: (bool, bool), turn: bool) -> Surface {
        let f = move |a: f64, b: f64| {
            let s = if back.0 { 1.0 - a } else { a };
            let t = if back.1 { 1.0 - b } else { b };
            let (x, y, w) = (1.0 - s, 1.0 - t, s * t);
            let q = x * x + y * y;
            [4.0 * x * w, 4.0 * y * w, 2.0 * (q - w * w), q + w * w]
        };
        bidegree_two(&f, turn)
    }

    /// A point the surface comes to as a cone, at a corner: `s t (1 − s,
    /// 1 − t, 1)`, a cone with its apex at the origin over the curve the
    /// L of the parameters is taken to.
    fn corner_cone(turn: bool) -> Surface {
        let f = |s: f64, t: f64| [s * t * (1.0 - s), s * t * (1.0 - t), s * t, 1.0];
        bidegree_two(&f, turn)
    }

    /// Surfaces with poles: an octant of a ball (one pole), a quarter of
    /// a ball from pole to pole (two), a quarter of a cone up to its apex
    /// (a point that is no smooth one) and a drop whose profile bends;
    /// a lune with its pole at a corner of the parameters (at each of the
    /// four), and a cone with its apex at one.
    fn with_poles(flip: bool) -> Vec<(&'static str, Surface)> {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let one = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let two = vec![0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0];
        vec![
            (
                "octant",
                swept(
                    &[(2.0, 0.0, 1.0), (2.0, 2.0, s), (0.0, 2.0, 1.0)],
                    one.clone(),
                    flip,
                ),
            ),
            (
                "lune",
                swept(
                    &[
                        (0.0, -2.0, 1.0),
                        (2.0, -2.0, s),
                        (2.0, 0.0, 1.0),
                        (2.0, 2.0, s),
                        (0.0, 2.0, 1.0),
                    ],
                    two,
                    flip,
                ),
            ),
            (
                "cone",
                swept(
                    &[(0.0, 0.0, 1.0), (0.75, 1.0, 1.0), (1.5, 2.0, 1.0)],
                    one.clone(),
                    flip,
                ),
            ),
            (
                "drop",
                swept(
                    &[(0.0, 0.0, 1.0), (2.0, 0.5, 1.0), (1.0, 3.0, 1.0)],
                    one,
                    flip,
                ),
            ),
            ("corner lune", corner_lune((false, false), flip)),
            ("corner lune, u back", corner_lune((true, false), flip)),
            ("corner lune, v back", corner_lune((false, true), flip)),
            ("corner lune, both back", corner_lune((true, true), flip)),
            ("corner cone", corner_cone(flip)),
            // A cusp: the profile leaves the axis along it, and the width
            // grows as the square of the distance from the point
            // (`pole_order` 2).
            (
                "cusp",
                swept(
                    &[(0.0, 0.0, 1.0), (0.0, 1.0, 1.0), (1.5, 2.0, 1.0)],
                    vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    flip,
                ),
            ),
        ]
    }

    /// The speed of the chart is a bound: no step on the surface, however
    /// near a pole and in whatever direction, moves the chart faster. And
    /// not a uselessly loose one.
    #[test]
    fn the_chart_of_a_surface_with_poles_moves_no_faster_than_its_speed() {
        for flip in [false, true] {
            for (name, surface) in with_poles(flip) {
                let tree = PatchTree::new(&surface).unwrap();
                assert_eq!(tree.parameter_speed(), None, "{name}");
                let speed = tree
                    .chart_speed()
                    .unwrap_or_else(|e| panic!("{name}, flipped {flip}: {e}"));
                let ((u0, u1), (v0, v1)) = surface.domain();
                let mut fastest = 0.0_f64;
                let places = [0.0, 1e-9, 1e-6, 1e-3, 0.02, 0.1, 0.27, 0.5, 0.73, 0.9, 0.98];
                let mut all = places.to_vec();
                all.extend(places.iter().map(|x| 1.0 - x));
                for &a in &all {
                    for &b in &all {
                        let (u, v) = (u0 + (u1 - u0) * a, v0 + (v1 - v0) * b);
                        for turn in 0..16 {
                            let angle = f64::from(turn) * std::f64::consts::PI / 8.0;
                            for reach in [1e-4, 1e-7] {
                                let (nu, nv) = tree.clamp(
                                    u + reach * (u1 - u0) * angle.pcos(),
                                    v + reach * (v1 - v0) * angle.psin(),
                                );
                                let travelled = surface.eval(u, v).distance(surface.eval(nu, nv));
                                if !(travelled > 1e-13) {
                                    continue;
                                }
                                let (x0, y0) = tree.chart(u, v);
                                let (x1, y1) = tree.chart(nu, nv);
                                let pace = (x1 - x0).phypot(y1 - y0) / travelled;
                                assert!(
                                    pace <= speed * (1.0 + 1e-6),
                                    "{name}, flipped {flip}: at ({a}, {b}) the chart moves {pace} per length, above {speed}"
                                );
                                fastest = fastest.max(pace);
                            }
                        }
                    }
                }
                println!("{name}, flipped {flip}: bound {speed}, fastest seen {fastest}");
                assert!(speed <= 30.0 * fastest, "{name}: {speed} against {fastest}");
            }
        }
    }

    /// A pole is one point of the chart, whatever parameter it is found
    /// with, and the chart turns the way the parameters do.
    #[test]
    fn a_pole_is_one_point_of_the_chart() {
        for flip in [false, true] {
            for (name, surface) in with_poles(flip) {
                let tree = PatchTree::new(&surface).unwrap();
                let ((u0, u1), (v0, v1)) = surface.domain();
                for pole in &tree.poles {
                    let at = |along: f64| match pole.border {
                        Border::UStart => tree.chart(u0, v0 + (v1 - v0) * along),
                        Border::UEnd => tree.chart(u1, v0 + (v1 - v0) * along),
                        Border::VStart => tree.chart(u0 + (u1 - u0) * along, v0),
                        Border::VEnd => tree.chart(u0 + (u1 - u0) * along, v1),
                    };
                    for along in [0.3, 1.0] {
                        assert_eq!(at(0.0), at(along), "{name}");
                    }
                }
                // Counter-clockwise in the parameters is counter-clockwise
                // in the chart.
                let (u, v) = (u0 + 0.4 * (u1 - u0), v0 + 0.3 * (v1 - v0));
                let h = 1e-4;
                let (o, a, b) = (
                    tree.chart(u, v),
                    tree.chart(u + h * (u1 - u0), v),
                    tree.chart(u, v + h * (v1 - v0)),
                );
                let turn = (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
                assert!(turn > 0.0, "{name}, flipped {flip}");
            }
        }
    }

    /// The signed distance next to a pole: to a ball it is the distance
    /// from the centre less the radius also straight above and below the
    /// pole, where the nearest point is the pole itself; behind the apex
    /// of a cone it is positive on the side the cone's normal points to.
    #[test]
    fn the_signed_distance_at_a_pole_has_the_sign_of_the_side() {
        let frame = Frame::new(
            Vec3::new(1.0, -2.0, 0.5),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap();
        for flip in [false, true] {
            let surfaces = with_poles(flip);
            let side = if flip { -1.0 } else { 1.0 };
            // The octant: u × v points away from the centre (towards it
            // when the parameters are exchanged).
            let tree = PatchTree::new(&surfaces[0].1).unwrap();
            for (x, y, z) in [
                (0.0, 0.0, 2.5),
                (0.0, 0.0, 1.5),
                (1e-9, 0.0, 2.3),
                (0.01, 0.02, 2.2),
                (0.01, 0.02, 1.8),
                (-0.2, -0.1, 2.4),
                (0.3, 0.2, 2.1),
                (0.3, 0.2, 1.7),
            ] {
                let p = frame.point(x, y, z);
                let d = tree.signed_distance(p).unwrap();
                // Past the borders of the octant the nearest point is on
                // a border: the sign is still the ball's there.
                let ball = p.distance(frame.origin()) - 2.0;
                if x >= 0.0 && y >= 0.0 {
                    assert!(
                        (d - side * ball).abs() < 1e-9,
                        "({x}, {y}, {z}): {d} vs {ball}"
                    );
                } else {
                    assert_eq!(d > 0.0, side * ball > 0.0, "({x}, {y}, {z})");
                }
            }
            // The cone (apex at the frame's origin, opening along z at
            // 0.75 per unit): u × v points away from the axis and back.
            let tree = PatchTree::new(&surfaces[2].1).unwrap();
            for (x, y, z, outside) in [
                (0.1, 0.1, -0.5, true),
                (0.3, 0.05, -0.1, true),
                (0.02, 0.3, -0.2, true),
                (0.05, 0.05, 0.5, false),
                (0.2, 0.1, 0.6, false),
                (0.5, 0.4, 0.1, true),
            ] {
                let p = frame.point(x, y, z);
                let d = tree.signed_distance(p).unwrap();
                assert_eq!(d * side > 0.0, outside, "cone ({x}, {y}, {z}): {d}");
            }
        }
    }

    /// The signed distance next to a pole at a corner of the parameters:
    /// to the lune it is the distance from the centre less the radius,
    /// also straight above and below the pole, where the nearest point is
    /// the pole itself, wherever the corner is.
    #[test]
    fn the_signed_distance_at_a_corner_pole_is_that_of_the_ball() {
        let frame = Frame::new(
            Vec3::new(1.0, -2.0, 0.5),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap();
        for turn in [false, true] {
            for back in [(false, false), (true, false), (false, true), (true, true)] {
                let surface = corner_lune(back, turn);
                let tree = PatchTree::new(&surface).unwrap();
                // Which way the patch's normal points: out of the ball or in.
                let at = surface.eval(0.4, 0.6);
                let side = if surface.normal(0.4, 0.6).unwrap().dot(at - frame.origin()) > 0.0 {
                    1.0
                } else {
                    -1.0
                };
                for (x, y, z) in [
                    (0.0, 0.0, 2.5),
                    (0.0, 0.0, 1.5),
                    (1e-9, 0.0, 2.3),
                    (0.01, 0.02, 2.2),
                    (0.01, 0.02, 1.8),
                    (0.02, 0.01, 1.8),
                    (-0.2, -0.1, 2.4),
                    (0.3, 0.2, 2.1),
                    (0.3, 0.2, 1.7),
                    (0.2, 0.3, -1.7),
                ] {
                    let p = frame.point(x, y, z);
                    let d = tree.signed_distance(p).unwrap();
                    let ball = p.distance(frame.origin()) - 2.0;
                    if x >= 0.0 && y >= 0.0 {
                        assert!(
                            (d - side * ball).abs() < 1e-9,
                            "{back:?}, {turn}: ({x}, {y}, {z}): {d} vs {ball}"
                        );
                    } else {
                        assert_eq!(d > 0.0, side * ball > 0.0, "({x}, {y}, {z})");
                    }
                }
            }
        }
    }

    /// What has no chart is refused with its reason: a patch that closes
    /// into a point on two adjacent sides but does not leave it in a fan.
    #[test]
    fn poles_without_a_chart_are_refused_with_the_reason() {
        // A triangle-like patch: the borders u = 0 and v = 0 are both the
        // origin, and every meridian from the border v = 0 leaves it the
        // same way (the patch is `u v R(v)`): no fan. (Until 2026-10-15
        // every pole at a corner was refused, `POLES_BOTH_WAYS`.)
        let corner = Surface::Bezier(
            BezierPatch::new(
                1,
                2,
                vec![
                    Vec3::ZERO,
                    Vec3::ZERO,
                    Vec3::ZERO,
                    Vec3::ZERO,
                    Vec3::new(1.0, 0.5, 0.2),
                    Vec3::new(1.0, 2.0, 0.0),
                ],
            )
            .unwrap(),
        );
        let tree = PatchTree::new(&corner).unwrap();
        assert!(matches!(
            tree.chart_speed(),
            Err(reason) if reason == IRREGULAR || reason == POLE_SHAPE
        ));
        // (A cusp, where the profile leaves the axis along it, was refused
        // here until 2026-10-15: it is in `with_poles` now.)
    }

    /// A net divided by its parameter is the net of the quotient.
    #[test]
    fn a_net_with_a_zero_row_is_divided_by_its_parameter() {
        // t² (1 − t) + 2 t (1 − t)² in the Bernstein form of degree 3:
        // 0, 2/3, 1/3, 0.
        let net: Vec<Vec3> = [0.0, 2.0 / 3.0, 1.0 / 3.0, 0.0]
            .iter()
            .map(|&c| Vec3::new(c, 0.0, 0.0))
            .collect();
        let bernstein = |net: &[Vec3], t: f64| {
            let n = net.len() - 1;
            let mut value = 0.0;
            let mut choose = 1.0;
            for (j, c) in net.iter().enumerate() {
                value += choose * t.ppowi(j as i32) * (1.0 - t).ppowi((n - j) as i32) * c.x;
                choose = choose * (n - j) as f64 / (j + 1) as f64;
            }
            value
        };
        let low = divided(&net, 3, true).unwrap();
        let both = divided(&low, 2, false).unwrap();
        for t in [0.1, 0.5, 0.8] {
            let f = t * t * (1.0 - t) + 2.0 * t * (1.0 - t) * (1.0 - t);
            assert!((bernstein(&low, t) - f / t).abs() < 1e-14);
            assert!((bernstein(&both, t) - f / (t * (1.0 - t))).abs() < 1e-14);
        }
        // A row that is not zero cannot be divided out.
        assert!(divided(&low, 2, true).is_none());
    }

    #[test]
    fn a_nearest_point_remembered_is_the_one_the_search_finds() {
        let surface = dome();
        let tree = PatchTree::new(&surface).unwrap();
        let mut seed = 3_u64;
        let mut unit = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let points: Vec<Vec3> = (0..200)
            .map(|_| Vec3::new(4.0 * unit() + 2.0, 3.0 * unit() + 1.5, 3.0 * unit() + 2.0))
            .chain([Vec3::new(f64::NAN, 0.0, 0.0), Vec3::new(0.0, -0.0, 2.0)])
            .collect();
        // Asked twice: the second answer comes from the memo.
        for _ in 0..2 {
            for &p in &points {
                assert_eq!(
                    format!("{:?}", tree.nearest(p)),
                    format!("{:?}", tree.search(p)),
                    "{p:?}"
                );
            }
        }
        // (0, 0, 2) and (0, -0, 2) are two points to the memo, with the
        // same answer.
        assert_eq!(
            format!("{:?}", tree.nearest(Vec3::new(0.0, 0.0, 2.0))),
            format!("{:?}", tree.search(Vec3::new(0.0, -0.0, 2.0)))
        );
    }

    #[test]
    fn a_tree_is_shared_by_equal_surfaces_only() {
        let a = PatchTree::shared(&dome()).unwrap();
        let b = PatchTree::shared(&dome()).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        let Surface::Bezier(patch) = dome() else {
            panic!("the dome is a Bézier patch");
        };
        let mut ctrl = patch.control_points().to_vec();
        ctrl[5].z += 1e-12;
        let moved = Surface::Bezier(BezierPatch::new(3, 3, ctrl).unwrap());
        let c = PatchTree::shared(&moved).unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
        assert_eq!(c.surface, moved);
    }

    /// A strip along a cubic B-spline of `spans` spans winding in the xy
    /// plane (an involute-like spiral `radius` across), extruded `width`
    /// along z: the face a fitted formula curve is extruded into, as the
    /// flank of a gear tooth. `turn` turns it about z, so that strips of
    /// different turns are different surfaces.
    fn strip(spans: usize, radius: f64, width: f64, turn: f64) -> Surface {
        let count_u = spans + 3;
        let mut knots_u = vec![0.0; 4];
        for k in 1..spans {
            knots_u.push(k as f64 / spans as f64);
        }
        knots_u.extend([1.0; 4]);
        let mut ctrl = Vec::new();
        for i in 0..count_u {
            let t = 0.4 * i as f64 / (count_u - 1) as f64;
            let r = radius * (1.0 + t * t);
            let a = turn + t;
            let p = Vec3::new(r * a.pcos(), r * a.psin(), 0.0);
            ctrl.push(p);
            ctrl.push(p + Vec3::new(0.0, 0.0, width));
        }
        let weights = vec![1.0; ctrl.len()];
        Surface::Nurbs(
            NurbsSurface::new(
                3,
                1,
                knots_u,
                vec![0.0, 0.0, 1.0, 1.0],
                count_u,
                2,
                ctrl,
                weights,
            )
            .unwrap(),
        )
    }

    /// The flank of a tooth of the spur gear drawn from formulas: 30 cubic
    /// spans along a 3 mm involute, 10 mm wide. Quartered evenly, its tree
    /// had 1024 leaves a span, 40,950 nodes and 21.6 MB, and a boolean
    /// meeting the 80 faces of a gear's teeth held 2.2 GB of them
    /// (2026-10-23). Halved only across the way a piece is long (the
    /// strip's width), each span takes 32 to 64 leaves.
    #[test]
    fn a_thin_strip_along_a_fitted_curve_is_indexed_by_few_leaves() {
        let surface = strip(30, 10.0, 10.0, 0.0);
        let tree = PatchTree::new(&surface).unwrap();
        let leaves = tree.nodes.iter().filter(|n| n.patch.is_some()).count();
        // (Quartered: 30,720; the spans here a sixth of a millimetre
        // along, some halved along too.)
        assert!(leaves <= 2 * 30 * 32, "{leaves} leaves");
        assert!(tree.base_bytes <= 2 << 20, "{} bytes", tree.base_bytes);
        // Every leaf is still small in space, as small as quartering made
        // it the way the strip is long.
        let small = tree
            .roots
            .iter()
            .filter_map(|&r| tree.nodes.get(r))
            .map(|n| n.bbox.diagonal())
            .fold(0.0, f64::max)
            / 32.0;
        for node in tree.nodes.iter().filter(|n| n.patch.is_some()) {
            assert!(node.bbox.diagonal() <= 2.0 * small, "{:?}", node.bbox);
        }
        // And the nearest point is found as before.
        let p = surface.eval(0.37, 0.61);
        let found = tree.nearest(p).unwrap();
        assert!(found.distance <= 1e-9, "{found:?}");
    }

    /// One operation's trees are held to a budget together: past it,
    /// [`PatchTree::shared`] refuses and builds no more, and the trees
    /// asked for again count once. Outside an operation nothing is held.
    #[test]
    fn the_trees_of_one_operation_are_held_to_a_budget() {
        let first = PatchTree::new(&strip(30, 10.0, 10.0, 0.5)).unwrap();
        let each = first.base_bytes;
        assert!(each > 0);
        let limit = 5 * each + each / 2;
        let built_before = trees_built();
        let seen = std::cell::Cell::new((0, None, 0));
        let outcome = holding_within(limit, || {
            let mut kept = Vec::new();
            for k in 0..20 {
                match PatchTree::shared(&strip(30, 10.0, 10.0, 0.5 + 0.01 * k as f64)) {
                    Ok(tree) => {
                        // Asked for again: counted once.
                        PatchTree::shared(&tree.surface).unwrap();
                        kept.push(tree);
                    }
                    Err(reason) => {
                        seen.set((kept.len(), Some(reason), held_bytes()));
                        // The refusal swallowed: the operation still fails.
                        return Ok(());
                    }
                }
            }
            Ok(())
        });
        let seen = seen.get();
        assert_eq!(seen.0, 5, "{seen:?}");
        assert_eq!(seen.1, Some(TOO_MUCH_HELD));
        assert!(seen.2 <= limit + each, "{seen:?}");
        match outcome {
            Err(crate::KernelError::Unsupported { reason, .. }) => {
                assert_eq!(reason, TOO_MUCH_HELD);
            }
            other => panic!("{other:?}"),
        }
        // The refusal came before a seventh tree was built.
        assert!(trees_built() - built_before <= 6);
        assert_eq!(held_bytes(), 0);
        assert!(PatchTree::shared(&strip(30, 10.0, 10.0, 0.9)).is_ok());
        // Within its budget, an operation answers its own answer.
        let fine = holding_within(limit, || {
            PatchTree::shared(&strip(30, 10.0, 10.0, 0.95)).map_err(|reason| {
                crate::KernelError::Unsupported {
                    operation: "test",
                    reason,
                    faces: Vec::new(),
                }
            })?;
            Ok(7)
        });
        assert_eq!(fine.unwrap(), 7);
    }

    /// An evened tree is not remembered as missing when the operation's
    /// budget refused it: the next operation, within its budget, gets it,
    /// and a remembered one counts against the budget of whoever asks.
    #[test]
    fn a_refused_evened_tree_is_not_remembered() {
        let surface = Surface::Nurbs(
            NurbsSurface::new(
                1,
                1,
                vec![0.0, 0.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                2,
                2,
                vec![
                    Vec3::new(0.0, 0.0, 1.3),
                    Vec3::new(0.0, 3.0, 1.5),
                    Vec3::new(4.0, 0.0, 1.1),
                    Vec3::new(4.0, 3.0, 1.3),
                ],
                vec![1.0, 1.0, 4.0, 4.0],
            )
            .unwrap(),
        );
        let tree = PatchTree::shared(&surface).unwrap();
        let limit = tree.base_bytes;
        let refused = holding_within(limit, || {
            assert!(PatchTree::shared(&surface).is_ok());
            let even = PatchTree::evened(&tree);
            assert!(Arc::ptr_eq(&even, &tree));
            Ok(())
        });
        assert!(refused.is_err(), "{refused:?}");
        assert!(tree.evened.get().is_none());
        let even = holding(|| Ok(PatchTree::evened(&tree))).unwrap();
        assert!(!Arc::ptr_eq(&even, &tree));
        assert!(matches!(tree.evened.get(), Some(Some(_))));
        // Remembered now, and still counted: refused again under the same
        // budget, though nothing is built.
        let again = holding_within(limit, || {
            PatchTree::shared(&surface).ok();
            PatchTree::evened(&tree);
            Ok(())
        });
        assert!(again.is_err(), "{again:?}");
    }

    /// An interference between two freeform bodies past the budget of its
    /// trees is refused, not answered from the faces it could outline
    /// (`faces_at` takes a face it cannot outline as one the point is not
    /// on): within it, the same question has an answer.
    #[test]
    fn an_interference_past_the_budget_is_refused_not_answered_short() {
        let a = craft3d_testkit::trimmed::domed_block(&Frame::WORLD, 4.0, 3.0, 2.0, true)
            .unwrap()
            .body;
        let at = Frame::WORLD.at(Vec3::new(1.0, 0.5, 0.0)).unwrap();
        let b = craft3d_testkit::trimmed::domed_block(&at, 4.0, 3.0, 2.0, true)
            .unwrap()
            .body;
        let tol = craft3d_math::Tolerance::default();
        let answered = super::super::interference(&a, &b, &tol);
        assert!(answered.is_ok(), "{answered:?}");
        let refused = holding_within(1, || super::super::interference(&a, &b, &tol));
        match refused {
            Err(crate::KernelError::Unsupported { reason, .. }) => {
                assert_eq!(reason, TOO_MUCH_HELD);
            }
            other => panic!("{other:?}"),
        }
    }

    /// The children of a node take no more room than four indices did.
    #[test]
    fn a_node_did_not_grow() {
        assert_eq!(
            std::mem::size_of::<Option<Kids>>(),
            std::mem::size_of::<Option<[usize; 4]>>()
        );
    }
}
