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
//! - for a polynomial surface, how fast its parameters can change per
//!   length travelled on it (`chart.rs` needs that to follow a loop in the
//!   chart of the parameters), from the Bézier nets of the derivatives and
//!   of the normal field: the smallest singular value of the Jacobian is
//!   at least `|∂u × ∂v| / sqrt(|∂u|² + |∂v|²)`, and each of those is
//!   bounded by the hull of its net.

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::Reason;
use craft3d_geom::{NurbsSurface, RationalBezierPatch, Surface};
use craft3d_math::{Bbox3, Vec3};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub(super) const RATIONAL: Reason = "a rational freeform face (weights that are not all one); polynomial Bézier and B-spline faces are handled";
pub(super) const IRREGULAR: Reason = "a freeform face whose parametrisation stalls or folds somewhere (a collapsed or self-overlapping patch); its chart cannot be followed";
const TOO_MANY: Reason = "a freeform face with more pieces than booleans can index";

/// How many times a Bézier span is quartered at most, and how many leaves
/// a whole surface may have.
const MAX_DEPTH: usize = 5;
const MAX_LEAVES: usize = 1 << 15;
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
    /// bound finer than its box.
    patch: Option<RationalBezierPatch>,
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
    polynomial: bool,
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

/// The distance from the origin to the box around some vectors; zero when
/// the box holds the origin.
fn clearance_of(vectors: &[Vec3]) -> f64 {
    box_distance(&Bbox3::from_points(vectors.iter().copied()), Vec3::ZERO)
}

/// A lower bound on the smallest singular value of the Jacobian of a
/// polynomial patch over all of it, or zero when the nets allow a
/// vanishing or folding normal.
fn sigma_bound(patch: &RationalBezierPatch) -> f64 {
    let (Some((du, dv)), Some(normal)) = (patch.derivative_nets(), patch.normal_net()) else {
        return 0.0;
    };
    // For the Jacobian J = [a b], the eigenvalues of JᵀJ have the sum
    // |a|² + |b|² and the product |a × b|²; the smaller one falls as the
    // sum grows and rises with the product, so the largest sum and the
    // smallest product the nets allow give a bound below it.
    let longest = |net: &[Vec3]| net.iter().map(|v| v.length_squared()).fold(0.0, f64::max);
    let sum = longest(&du) + longest(&dv);
    let product = clearance_of(&normal).powi(2);
    if !(sum > 0.0 && product.is_finite()) {
        return 0.0;
    }
    let least = 0.5 * (sum - (sum * sum - 4.0 * product).max(0.0).sqrt());
    least.max(0.0).sqrt()
}

/// How many times a leaf may be quartered for one query.
const EXTRA_DEPTH: usize = 40;

/// Splits the pieces on `pending` until their boxes are within
/// `resolution`, keeping only the pieces nearer than the best bound so far;
/// returns the bound.
fn refine(
    pending: &mut Vec<(RationalBezierPatch, Bbox3, usize)>,
    p: Vec3,
    resolution: f64,
    mut best: f64,
) -> f64 {
    let mut steps = 0_usize;
    while let Some((patch, bbox, depth)) = pending.pop() {
        steps += 1;
        let d = box_distance(&bbox, p);
        if d >= best {
            continue;
        }
        // Small enough: within `resolution`, or within a quarter of the
        // distance itself (a bound a quarter short still proves a far
        // stretch clear; splitting on would visit every piece within a
        // disc that grows as the pieces shrink).
        let size = bbox.diagonal();
        if size <= resolution || size <= 0.25 * d || depth >= EXTRA_DEPTH || steps > MAX_LEAVES {
            best = best.min(d);
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
        let mut boxed: Vec<(RationalBezierPatch, Bbox3, f64)> = quarters
            .into_iter()
            .map(|q| {
                let b = q.control_bbox();
                let d = box_distance(&b, p);
                (q, b, d)
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
            polynomial,
        };
        let whole = spans
            .iter()
            .map(|s| s.2.control_bbox().diagonal())
            .fold(0.0, f64::max);
        for (u, v, patch) in &spans {
            let root = tree.subdivide(patch, *u, *v, depth, whole / 32.0);
            tree.roots.push(root);
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
        let sigma = if self.polynomial {
            sigma_bound(patch)
        } else {
            0.0
        };
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
        let patch = children.is_none().then(|| patch.clone());
        self.nodes.push(Node {
            bbox,
            u,
            v,
            children,
            patch,
            sigma,
        });
        self.nodes.len() - 1
    }

    pub fn is_polynomial(&self) -> bool {
        self.polynomial
    }

    /// How far the surface's parameters can change per unit of length
    /// travelled on it, at most; `None` when the surface is rational, or
    /// may stall or fold (then its chart cannot be trusted).
    pub fn parameter_speed(&self) -> Option<f64> {
        if !self.polynomial {
            return None;
        }
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
    /// farther piece is dropped unsplit. The splits are not kept.
    pub fn lower_bound(&self, p: Vec3, resolution: f64) -> f64 {
        let resolution = resolution.max(0.0);
        let mut best = f64::INFINITY;
        let mut heap = BinaryHeap::new();
        for &root in &self.roots {
            if let Some(node) = self.nodes.get(root) {
                heap.push(Pending(box_distance(&node.bbox, p), root));
            }
        }
        let mut pending: Vec<(RationalBezierPatch, Bbox3, usize)> = Vec::new();
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
                            heap.push(Pending(box_distance(&child.bbox, p), k));
                        }
                    }
                }
                None => match &node.patch {
                    Some(patch) if node.bbox.diagonal() > resolution => {
                        pending.push((patch.clone(), node.bbox, 0));
                        best = refine(&mut pending, p, resolution, best);
                    }
                    _ => best = best.min(d),
                },
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
        for _ in 0..NEWTON_STEPS {
            let (s, su, sv) = self.surface.derivatives(u, v);
            let r = s - p;
            let (g1, g2) = (su.dot(r), sv.dot(r));
            if !(g1.hypot(g2) > 0.0) {
                break;
            }
            // The Hessian of half the squared distance: first-order terms
            // plus the curvature terms, which matter when `r` is long.
            let (_, su_up, sv_up) = self.surface.derivatives((u + hu).min(u1), v);
            let (_, su_down, sv_down) = self.surface.derivatives((u - hu).max(u0), v);
            let (_, _, sv_vup) = self.surface.derivatives(u, (v + hv).min(v1));
            let (_, _, sv_vdown) = self.surface.derivatives(u, (v - hv).max(v0));
            let suu = (su_up - su_down) * (0.5 / hu);
            let suv = (sv_up - sv_down) * (0.5 / hu);
            let svv = (sv_vup - sv_vdown) * (0.5 / hv);
            let (a, b, c) = (
                su.dot(su) + suu.dot(r),
                su.dot(sv) + suv.dot(r),
                sv.dot(sv) + svv.dot(r),
            );
            // Descent is held to the sides of the domain it stands on.
            let held_u = (u <= u0 && g1 > 0.0) || (u >= u1 && g1 < 0.0);
            let held_v = (v <= v0 && g2 > 0.0) || (v >= v1 && g2 < 0.0);
            if held_u && held_v {
                break;
            }
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
                        damping = (damping * 10.0).max(1e-9);
                        continue;
                    }
                    (-(cc * g1 - b * g2) / det, -(aa * g2 - b * g1) / det)
                };
                let (nu, nv) = self.clamp(u + du, v + dv);
                let candidate = self.surface.eval(nu, nv);
                let d = candidate.distance(p);
                if d < distance || (d == distance && (nu != u || nv != v) && damping < 1e-3) {
                    let step = (nu - u).hypot(nv - v);
                    u = nu;
                    v = nv;
                    point = candidate;
                    distance = d;
                    damping = (damping * 0.3).max(1e-12);
                    moved = step > 0.0;
                    break;
                }
                damping *= 10.0;
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
        let mut best: Option<Nearest> = None;
        let mut visited = 0_usize;
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
                            heap.push(Pending(box_distance(&child.bbox, p), k));
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
        assert!(tree.is_polynomial());
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
            let lower = tree.lower_bound(p, 1e-9);
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
                assert!(tree.lower_bound(p, 1e-9) <= near.distance + 1e-12);
            }
        }
        // The lid's top and the bottom patches collapse to a point: no
        // chart for those, the rest are regular.
        assert!(folded < patches.len() / 2, "{folded} of {}", patches.len());
    }

    #[test]
    fn a_rational_surface_is_indexed_but_has_no_parameter_speed() {
        let frame = Frame::WORLD;
        let circle = craft3d_geom::NurbsCurve::circle(&frame, 1.0).unwrap();
        // A cylinder as a rational surface: the circle swept along z.
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
        let surface = NurbsSurface::new(
            2,
            1,
            circle.knots().to_vec(),
            vec![0.0, 0.0, 1.0, 1.0],
            9,
            2,
            ctrl,
            weights,
        )
        .unwrap();
        let tree = PatchTree::new(&Surface::Nurbs(surface)).unwrap();
        assert!(!tree.is_polynomial());
        assert!(tree.parameter_speed().is_none());
        let d = tree.signed_distance(Vec3::new(3.0, 0.0, 1.0)).unwrap();
        assert!((d.abs() - 2.0).abs() < 1e-9, "{d}");
        assert!(tree.lower_bound(Vec3::new(3.0, 0.0, 1.0), 1e-9) <= 2.0);
    }
}
