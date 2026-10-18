//! Where a mesh passes through itself.
//!
//! Two triangles of a valid mesh meet only where they share a vertex or an
//! edge. This module finds the pairs that meet anywhere else, and says how
//! deep.
//!
//! **Whether** two triangles meet is decided exactly, on the coordinates
//! as they are (`exact.rs`): a planar face is full of triangles that are
//! coplanar to the last bit, and neighbours there must not be reported
//! because of a rounding error. Triangles are neighbours when they share
//! bit-identical corners, which is how the tessellator joins faces
//! (`weld.rs`):
//! - no shared corner: they meet when an edge of one meets the other;
//! - one shared corner: they meet beyond it when the edge opposite the
//!   corner in one meets the other (if they have a second point `x` in
//!   common they have the whole segment from the corner to `x`, which
//!   leaves one of the two through its opposite edge);
//! - a shared edge: only when they are in one plane on the same side of
//!   the edge, folded flat onto each other;
//! - all three: the same triangle twice.
//!
//! Triangles in one plane count as an *overlap* when they share area;
//! everything else is a *crossing*, which includes mere contact. A caller
//! may also name a distance within which triangles of two different faces
//! count as lying in one plane (`Rules::near`; `f64`, not exact): two
//! flat faces lying on each other anywhere but in a coordinate plane are
//! a rounding error out of one plane, and weave through each other by
//! that error or miss each other altogether.
//!
//! **How deep** is a measure, in `f64`. For a crossing of triangle `X`
//! with triangle `Y`: take the column over `Y` (all points whose foot on
//! the plane of `Y` is in `Y`), and the part of the surface around `X`
//! that is connected to `X` inside that column. If it reaches `d+` above
//! the plane of `Y` and `d-` below, the surface around `X` passes through
//! `Y` by `min(d+, d-)`: it cannot get from one side to the other inside
//! the column except through `Y`. That number is how far `Y` could be
//! moved along its normal, either way, and still be crossed. The depth of
//! the pair is the *smaller* of the two directions, `X` through `Y` and
//! `Y` through `X`: how far either could be moved along its own normal
//! without undoing the crossing. So:
//! - a surface that only touches `Y` has depth zero, however large the
//!   triangles;
//! - a tip that pricks a surface by a hair has the depth of the hair,
//!   although the surface it pricks reaches far on both sides of each of
//!   the tip's steep triangles;
//! - a surface that crosses `Y` along an edge of its own triangulation,
//!   where no single triangle is on both sides, has the depth of the two
//!   triangles together: it is the surface that is followed, not the
//!   triangle.
//!
//! The column is taken a hair wider than `Y` (1e-9 of its size) across
//! every edge of `Y` beyond which the surface around `Y` goes on, so that
//! a surface standing exactly on such an edge is in it: it passes through
//! the surface around `Y` between `Y` and its neighbour. Across an edge
//! where the surface around `Y` turns away square or folds back, and
//! across a border, the column ends at the edge: what stands there
//! touches (`Depths::column`).
//!
//! For an overlap the depth is how wide the common area of the two
//! triangles is: twice its area over its perimeter.
//!
//! Candidates come from a bounding-volume hierarchy over the triangles,
//! searched on several threads; results are merged in triangle order, so
//! they do not depend on the number of threads.

use crate::exact::{P, orient2d, orient3d};
use crate::{Mesh, TessError};
use craft3d_math::Vec3;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

/// How two triangles meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrossingKind {
    /// They are not in one plane and have a point in common: one passes
    /// through the other, or touches it.
    Crossing,
    /// They are in one plane and share area.
    CoplanarOverlap,
}

/// Two triangles that meet although they are not neighbours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crossing {
    /// Indices into `Mesh::triangles`, the smaller first.
    pub triangles: [usize; 2],
    /// Their face ids.
    pub faces: [u32; 2],
    /// A point where they meet (approximately; for looking, not deciding).
    pub at: Vec3,
    pub kind: CrossingKind,
    /// How far the surface of one goes through the other, or how far the
    /// overlap reaches into both; see the module documentation. Measured
    /// up to four times the margin it is compared with; beyond that it is
    /// a lower bound.
    pub depth: f64,
}

/// What [`self_intersection_report`] found.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SelfIntersections {
    /// In triangle order, at most `limit` of them.
    pub crossings: Vec<Crossing>,
    /// Triangles left out because they have no area: two corners at the
    /// same place, or three on one line.
    pub degenerate_triangles: usize,
    /// True when there were more crossings than `limit`.
    pub truncated: bool,
}

/// Pairs of triangles of `mesh` that cross or overlap although they are
/// not neighbours, deeper than `tolerance`.
///
/// Triangles are neighbours where their corners are bit-identical, as the
/// tessellator makes them. With a `tolerance` of zero every crossing is
/// reported that is deeper than the rounding of the depth measure (2e-13
/// of the largest coordinate); contact without crossing never is.
/// `limit` caps the result. Triangles without area are ignored.
///
/// An error for a tolerance that is negative or not finite, for a mesh
/// whose triangles refer to vertices it does not have, that has no face id
/// per triangle, or whose coordinates are not finite or above 1e100.
pub fn self_intersections(
    mesh: &Mesh,
    tolerance: f64,
    limit: usize,
) -> Result<Vec<Crossing>, TessError> {
    Ok(self_intersection_report(mesh, tolerance, limit, 0)?.crossings)
}

/// [`self_intersections`] with the count of ignored triangles, on a given
/// number of threads (0 means "as many as the machine offers"; one on
/// `wasm32`). The result is the same whatever the number.
pub fn self_intersection_report(
    mesh: &Mesh,
    tolerance: f64,
    limit: usize,
    threads: usize,
) -> Result<SelfIntersections, TessError> {
    if !(tolerance.is_finite() && tolerance >= 0.0) {
        return Err(TessError::InvalidTolerance(tolerance));
    }
    let rules = Rules {
        margin: &|_, _| tolerance,
        near: &|_, _| 0.0,
        reach: 0.0,
        looked_at: None,
    };
    let found = contacts(mesh, &rules, threads, raw_cap(limit))?;
    let threshold = tolerance.max(found.rounding);
    let mut crossings: Vec<Crossing> = found
        .contacts
        .into_iter()
        .filter(|c| c.depth > threshold)
        .collect();
    let truncated = found.truncated || crossings.len() > limit;
    crossings.truncate(limit);
    Ok(SelfIntersections {
        crossings,
        degenerate_triangles: found.degenerate,
        truncated,
    })
}

/// How many meeting pairs are looked at for a result of `limit`: most of
/// them may turn out shallower than the margin.
pub(crate) fn raw_cap(limit: usize) -> usize {
    limit.saturating_mul(64).clamp(4096, 1 << 20)
}

/// Every pair that meets, with its depth, whatever the depth.
pub(crate) struct Contacts {
    /// In triangle order.
    pub contacts: Vec<Crossing>,
    pub degenerate: usize,
    /// More pairs meet than the cap.
    pub truncated: bool,
    /// Depths below this are rounding.
    pub rounding: f64,
}

/// No more threads than this by default.
#[cfg(not(target_arch = "wasm32"))]
const MAX_AUTOMATIC_THREADS: usize = 16;

fn automatic_threads() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(MAX_AUTOMATIC_THREADS)
    }
}

/// Coordinates above this are refused: their products overflow.
const MAX_COORDINATE: f64 = 1e100;

/// Triangles per leaf of the hierarchy.
const LEAF: usize = 4;

/// Triangles a thread takes at a time.
const CHUNK: usize = 2048;

/// The surface around a crossing is followed over at most this many
/// triangles.
const PATCH_CAP: usize = 8192;

fn point(v: Vec3) -> P {
    [v.x, v.y, v.z]
}

fn vector(p: &P) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

/// The side of the plane of `t` that `p` is on: 1 where the normal
/// `(t1 - t0) x (t2 - t0)` points, -1 on the other side, 0 in the plane.
fn side(t: &[P; 3], p: &P) -> i8 {
    let [a, b, c] = t;
    -orient3d(a, b, c, p)
}

/// Two coordinate axes in which the triangle keeps its area, and its
/// orientation seen along the third.
fn flat_axes(t: &[P; 3]) -> Option<([usize; 2], i8)> {
    let [a, b, c] = t;
    let normal = (vector(b) - vector(a)).cross(vector(c) - vector(a));
    // The axis the triangle faces most squarely first; any with a non-zero
    // projection is right.
    let mut order = [
        (normal.x.abs(), [1, 2]),
        (normal.y.abs(), [2, 0]),
        (normal.z.abs(), [0, 1]),
    ];
    order.sort_by(|p, q| q.0.total_cmp(&p.0));
    for (_, axes) in order {
        let turn = orient2d(flat(a, axes), flat(b, axes), flat(c, axes));
        if turn != 0 {
            return Some((axes, turn));
        }
    }
    None
}

fn flat(p: &P, axes: [usize; 2]) -> [f64; 2] {
    let [u, v] = axes;
    [
        p.get(u).copied().unwrap_or(0.0),
        p.get(v).copied().unwrap_or(0.0),
    ]
}

/// The three edges of a triangle as corner indices.
const EDGES: [(usize, usize); 3] = [(0, 1), (1, 2), (2, 0)];

fn corner(t: &[P; 3], k: usize) -> &P {
    let [a, b, c] = t;
    match k % 3 {
        0 => a,
        1 => b,
        _ => c,
    }
}

fn pick<T: Copy>(values: &[T; 3], k: usize) -> T {
    let [a, b, c] = *values;
    match k % 3 {
        0 => a,
        1 => b,
        _ => c,
    }
}

/// Do the segment `p q` and the triangle `t` have a point in common, all
/// in one plane? By separating lines: they are apart exactly when a line
/// along an edge of the triangle, or the line of the segment, has them
/// strictly on different sides.
fn segment_meets_triangle_flat(p: &P, q: &P, t: &[P; 3]) -> bool {
    let Some((axes, turn)) = flat_axes(t) else {
        return false;
    };
    let (p, q) = (flat(p, axes), flat(q, axes));
    let corners = [0, 1, 2].map(|k| flat(corner(t, k), axes));
    for (i, j) in EDGES {
        let (a, b) = (pick(&corners, i), pick(&corners, j));
        if orient2d(a, b, p) * turn < 0 && orient2d(a, b, q) * turn < 0 {
            return false;
        }
    }
    let sides = corners.map(|c| orient2d(p, q, c));
    !(sides.iter().all(|&s| s > 0) || sides.iter().all(|&s| s < 0))
}

/// Do the segment `p q` and the triangle `t` have a point in common?
/// `sp` and `sq` are the sides of the plane of `t` that `p` and `q` are on.
fn segment_meets_triangle(p: &P, q: &P, sp: i8, sq: i8, t: &[P; 3]) -> bool {
    if sp * sq > 0 {
        return false;
    }
    if sp == 0 && sq == 0 {
        return segment_meets_triangle_flat(p, q, t);
    }
    // The line meets the plane in one point. It is in the triangle when it
    // is not strictly outside any edge: the three tetrahedra on the
    // segment and an edge then have the same orientation, or none.
    let turns = EDGES.map(|(i, j)| orient3d(p, q, corner(t, i), corner(t, j)));
    !(turns.iter().any(|&s| s > 0) && turns.iter().any(|&s| s < 0))
}

/// Do two triangles in one plane share area? Not when all of one is on or
/// outside the line of an edge of the other.
fn share_area(a: &[P; 3], b: &[P; 3]) -> bool {
    let Some((axes, turn_a)) = flat_axes(a) else {
        return false;
    };
    let fa = [0, 1, 2].map(|k| flat(corner(a, k), axes));
    let fb = [0, 1, 2].map(|k| flat(corner(b, k), axes));
    let turn_b = orient2d(pick(&fb, 0), pick(&fb, 1), pick(&fb, 2));
    if turn_b == 0 {
        return false;
    }
    let outside = |of: &[[f64; 2]; 3], turn: i8, other: &[[f64; 2]; 3]| {
        EDGES.iter().any(|&(i, j)| {
            other
                .iter()
                .all(|&c| orient2d(pick(of, i), pick(of, j), c) * turn <= 0)
        })
    };
    !(outside(&fa, turn_a, &fb) || outside(&fb, turn_b, &fa))
}

/// Do two triangles in one plane have a point in common?
fn touch_flat(a: &[P; 3], b: &[P; 3]) -> bool {
    EDGES
        .iter()
        .any(|&(i, j)| segment_meets_triangle_flat(corner(a, i), corner(a, j), b))
        || EDGES
            .iter()
            .any(|&(i, j)| segment_meets_triangle_flat(corner(b, i), corner(b, j), a))
}

/// How two triangles meet beyond the corners they share: `None` when they
/// do not, `Some(true)` when they are in one plane and share area,
/// `Some(false)` for every other meeting. `wa` and `wb` name the corners:
/// equal names are the same point.
fn meeting(a: &[P; 3], wa: &[u32; 3], b: &[P; 3], wb: &[u32; 3]) -> Option<bool> {
    // For each corner of `a`, the corner of `b` it is.
    let twin = wa.map(|w| wb.iter().position(|&x| x == w));
    let shared = twin.iter().flatten().count();
    if shared == 3 {
        return Some(true);
    }
    let is_shared_b = |k: usize| twin.contains(&Some(k));
    // Sides of each corner relative to the other triangle's plane; a
    // shared corner is in both planes.
    let sa = [0, 1, 2].map(|k| {
        if pick(&twin, k).is_some() {
            0
        } else {
            side(b, corner(a, k))
        }
    });
    if sa.iter().all(|&s| s > 0) || sa.iter().all(|&s| s < 0) {
        return None;
    }
    let sb = [0, 1, 2].map(|k| {
        if is_shared_b(k) {
            0
        } else {
            side(a, corner(b, k))
        }
    });
    if sb.iter().all(|&s| s > 0) || sb.iter().all(|&s| s < 0) {
        return None;
    }
    let coplanar = sa == [0, 0, 0] && sb == [0, 0, 0];
    match shared {
        0 => {
            if coplanar {
                if share_area(a, b) {
                    Some(true)
                } else {
                    touch_flat(a, b).then_some(false)
                }
            } else {
                let through = |x: &[P; 3], sx: &[i8; 3], y: &[P; 3]| {
                    EDGES.iter().any(|&(i, j)| {
                        segment_meets_triangle(
                            corner(x, i),
                            corner(x, j),
                            pick(sx, i),
                            pick(sx, j),
                            y,
                        )
                    })
                };
                (through(a, &sa, b) || through(b, &sb, a)).then_some(false)
            }
        }
        1 => {
            let ia = twin.iter().position(Option::is_some)?;
            let ib = pick(&twin, ia)?;
            if coplanar && share_area(a, b) {
                return Some(true);
            }
            // The edge opposite the shared corner, against the other.
            let opposite = |x: &[P; 3], sx: &[i8; 3], k: usize, y: &[P; 3]| {
                segment_meets_triangle(
                    corner(x, k + 1),
                    corner(x, k + 2),
                    pick(sx, k + 1),
                    pick(sx, k + 2),
                    y,
                )
            };
            (opposite(a, &sa, ia, b) || opposite(b, &sb, ib, a)).then_some(false)
        }
        // A shared edge: beyond it only when folded flat onto each other.
        _ => (coplanar && share_area(a, b)).then_some(true),
    }
}

/// An axis-aligned box.
#[derive(Clone, Copy)]
struct Aabb {
    lo: P,
    hi: P,
}

impl Aabb {
    const EMPTY: Aabb = Aabb {
        lo: [f64::INFINITY; 3],
        hi: [f64::NEG_INFINITY; 3],
    };

    fn of(t: &[P; 3]) -> Aabb {
        let mut out = Aabb::EMPTY;
        for p in t {
            out.add(p);
        }
        out
    }

    fn add(&mut self, p: &P) {
        for ((lo, hi), &x) in self.lo.iter_mut().zip(self.hi.iter_mut()).zip(p) {
            *lo = lo.min(x);
            *hi = hi.max(x);
        }
    }

    fn join(&mut self, o: &Aabb) {
        self.add(&o.lo);
        self.add(&o.hi);
    }

    fn meets(&self, o: &Aabb) -> bool {
        self.lo.iter().zip(&o.hi).all(|(lo, hi)| lo <= hi)
            && o.lo.iter().zip(&self.hi).all(|(lo, hi)| lo <= hi)
    }

    fn extent(&self, axis: usize) -> f64 {
        let lo = self.lo.get(axis).copied().unwrap_or(0.0);
        let hi = self.hi.get(axis).copied().unwrap_or(0.0);
        hi - lo
    }

    fn center(&self, axis: usize) -> f64 {
        let lo = self.lo.get(axis).copied().unwrap_or(0.0);
        let hi = self.hi.get(axis).copied().unwrap_or(0.0);
        0.5 * lo + 0.5 * hi
    }
}

#[derive(Clone, Copy)]
struct Node {
    bounds: Aabb,
    /// A leaf: the range `first..last` of `Bvh::items`. Otherwise the two
    /// children.
    first: u32,
    last: u32,
    leaf: bool,
}

/// A bounding-volume hierarchy over triangles: a binary tree that halves
/// its triangles along the longest axis of their centres.
struct Bvh {
    nodes: Vec<Node>,
    items: Vec<u32>,
}

impl Bvh {
    /// Over the triangles `items`, whose boxes are `boxes[item]`.
    fn build(boxes: &[Aabb], mut items: Vec<u32>) -> Bvh {
        let mut nodes = Vec::new();
        if items.is_empty() {
            return Bvh { nodes, items };
        }
        let blank = Node {
            bounds: Aabb::EMPTY,
            first: 0,
            last: 0,
            leaf: true,
        };
        nodes.push(blank);
        // (node, first, last): ranges halve, so the stack stays shallow.
        let mut stack = vec![(0_usize, 0_usize, items.len())];
        while let Some((index, first, last)) = stack.pop() {
            let Some(range) = items.get_mut(first..last) else {
                continue;
            };
            let mut bounds = Aabb::EMPTY;
            let mut centers = Aabb::EMPTY;
            for &item in range.iter() {
                if let Some(b) = boxes.get(item as usize) {
                    bounds.join(b);
                    centers.add(&[b.center(0), b.center(1), b.center(2)]);
                }
            }
            let children = nodes.len();
            let Some(node) = nodes.get_mut(index) else {
                continue;
            };
            node.bounds = bounds;
            if range.len() <= LEAF {
                // Counts fit: triangle indices are u32 in the mesh.
                (node.first, node.last, node.leaf) = (first as u32, last as u32, true);
                continue;
            }
            let extent = |axis: usize| centers.extent(axis);
            let axis = if extent(0) >= extent(1) && extent(0) >= extent(2) {
                0
            } else if extent(1) >= extent(2) {
                1
            } else {
                2
            };
            let key = |item: u32| boxes.get(item as usize).map_or(0.0, |b| b.center(axis));
            let half = range.len() / 2;
            // The index breaks ties, so the order never depends on how the
            // selection happens to move equal keys.
            range.select_nth_unstable_by(half, |&p, &q| key(p).total_cmp(&key(q)).then(p.cmp(&q)));
            (node.first, node.last, node.leaf) = (children as u32, children as u32 + 1, false);
            nodes.push(blank);
            nodes.push(blank);
            stack.push((children, first, first + half));
            stack.push((children + 1, first + half, last));
        }
        Bvh { nodes, items }
    }

    /// Calls `found` for every triangle whose leaf box meets `query`.
    fn search(&self, query: &Aabb, stack: &mut Vec<u32>, mut found: impl FnMut(u32)) {
        stack.clear();
        stack.push(0);
        while let Some(index) = stack.pop() {
            let Some(node) = self.nodes.get(index as usize) else {
                continue;
            };
            if !node.bounds.meets(query) {
                continue;
            }
            if node.leaf {
                let range = node.first as usize..node.last as usize;
                for &item in self.items.get(range).unwrap_or(&[]) {
                    found(item);
                }
            } else {
                stack.push(node.first);
                stack.push(node.last);
            }
        }
    }
}

/// A mesh prepared for the search.
struct Prepared<'a> {
    mesh: &'a Mesh,
    /// Per triangle, its corners by name: vertices at the same place have
    /// the same name.
    names: Vec<[u32; 3]>,
    /// Per triangle: does it have area?
    solid: Vec<bool>,
    boxes: Vec<Aabb>,
    bvh: Bvh,
    degenerate: usize,
    largest: f64,
}

/// A pair that meets: triangles `i < j`, and whether as an overlap.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Pair {
    i: u32,
    j: u32,
    overlap: bool,
    /// An overlap of triangles that are not in one plane exactly, only
    /// within [`Rules::near`] of it.
    nearly: bool,
}

/// What counts, per pair of faces.
pub(crate) struct Rules<'a> {
    /// The depth a crossing between two faces must exceed to count;
    /// depths are measured up to four times it.
    pub margin: &'a (dyn Fn(u32, u32) -> f64 + Sync),
    /// Triangles of two different faces whose corners are within this of
    /// each other's planes lie on each other as if they were in one
    /// plane. Zero: only those that are in one plane exactly.
    pub near: &'a (dyn Fn(u32, u32) -> f64 + Sync),
    /// The largest value `near` gives.
    pub reach: f64,
    /// Whether triangles of these two faces (the same face twice for two
    /// triangles of one) are looked at; `None`: all are.
    pub looked_at: Option<&'a (dyn Fn(u32, u32) -> bool + Sync)>,
}

/// Are all corners of each triangle within `near` of the plane of the
/// other?
fn lie_on_each_other(a: &[P; 3], b: &[P; 3], near: f64) -> bool {
    let one_way = |x: &[P; 3], y: &[P; 3]| {
        let [y0, y1, y2] = y.map(|p| vector(&p));
        let Some(normal) = (y1 - y0).cross(y2 - y0).normalized() else {
            return false;
        };
        x.iter().all(|p| (vector(p) - y0).dot(normal).abs() <= near)
    };
    one_way(a, b) && one_way(b, a)
}

impl<'a> Prepared<'a> {
    fn new(mesh: &'a Mesh) -> Result<Self, TessError> {
        let bad = |what: String| TessError::InvalidBody(what);
        if mesh.face_ids.len() != mesh.triangles.len() {
            return Err(bad(format!(
                "the mesh has {} triangles and {} face ids",
                mesh.triangles.len(),
                mesh.face_ids.len()
            )));
        }
        if u32::try_from(mesh.triangles.len()).is_err()
            || u32::try_from(mesh.positions.len()).is_err()
        {
            return Err(TessError::TooManyVertices);
        }
        let mut largest = 0.0_f64;
        for (i, p) in mesh.positions.iter().enumerate() {
            let size = p.x.abs().max(p.y.abs()).max(p.z.abs());
            // `max` skips a NaN, so finiteness is asked of the point.
            if !p.is_finite() || size > MAX_COORDINATE {
                return Err(bad(format!(
                    "vertex {i} of the mesh is not finite or out of range"
                )));
            }
            largest = largest.max(size);
        }

        // Vertices at the same place get the name of the first of them.
        // -0.0 and 0.0 are the same place.
        let key = |v: &Vec3| [v.x + 0.0, v.y + 0.0, v.z + 0.0].map(f64::to_bits);
        let mut order: Vec<u32> = (0..mesh.positions.len() as u32).collect();
        let keys: Vec<[u64; 3]> = mesh.positions.iter().map(key).collect();
        let key_of = |i: u32| keys.get(i as usize).copied().unwrap_or([0; 3]);
        order.sort_unstable_by(|&p, &q| key_of(p).cmp(&key_of(q)).then(p.cmp(&q)));
        let mut name = vec![0_u32; mesh.positions.len()];
        let mut current = (None, 0_u32);
        for &i in &order {
            let k = key_of(i);
            if current.0 != Some(k) {
                current = (Some(k), i);
            }
            if let Some(slot) = name.get_mut(i as usize) {
                *slot = current.1;
            }
        }

        let mut names = Vec::with_capacity(mesh.triangles.len());
        let mut solid = Vec::with_capacity(mesh.triangles.len());
        let mut boxes = Vec::with_capacity(mesh.triangles.len());
        let mut items = Vec::with_capacity(mesh.triangles.len());
        let mut degenerate = 0;
        for (t, tri) in mesh.triangles.iter().enumerate() {
            let named = tri.map(|k| name.get(k as usize).copied());
            let ([Some(a), Some(b), Some(c)], Some(corners)) = (named, mesh.corners(t)) else {
                return Err(bad(format!(
                    "triangle {t} of the mesh refers to a vertex that does not exist"
                )));
            };
            let corners = corners.map(point);
            let has_area = a != b && b != c && c != a && flat_axes(&corners).is_some();
            names.push([a, b, c]);
            solid.push(has_area);
            boxes.push(Aabb::of(&corners));
            if has_area {
                items.push(t as u32);
            } else {
                degenerate += 1;
            }
        }
        let bvh = Bvh::build(&boxes, items);
        Ok(Self {
            mesh,
            names,
            solid,
            boxes,
            bvh,
            degenerate,
            largest,
        })
    }

    fn corners(&self, t: u32) -> Option<[P; 3]> {
        self.mesh.corners(t as usize).map(|c| c.map(point))
    }

    fn points(&self, t: u32) -> Option<[Vec3; 3]> {
        self.mesh.corners(t as usize)
    }

    fn names(&self, t: u32) -> [u32; 3] {
        self.names.get(t as usize).copied().unwrap_or([u32::MAX; 3])
    }

    fn face(&self, t: u32) -> u32 {
        self.mesh
            .face_ids
            .get(t as usize)
            .copied()
            .unwrap_or(u32::MAX)
    }

    /// The pairs `(i, j)` with `i` in `range` and `j > i` that meet, in
    /// order; at most `cap` of them, and whether there were more.
    fn pairs_in(
        &self,
        range: std::ops::Range<usize>,
        cap: usize,
        rules: &Rules<'_>,
    ) -> (Vec<Pair>, bool) {
        let reach = if rules.reach > 0.0 { rules.reach } else { 0.0 };
        let mut out = Vec::new();
        let mut stack = Vec::new();
        let mut candidates = Vec::new();
        for i in range {
            let i = i as u32;
            if !self.solid.get(i as usize).copied().unwrap_or(false) {
                continue;
            }
            let (Some(bounds), Some(a)) = (self.boxes.get(i as usize), self.corners(i)) else {
                continue;
            };
            // Wider by the reach, so that triangles lying on each other
            // without meeting are candidates too.
            let wide = Aabb {
                lo: bounds.lo.map(|x| x - reach),
                hi: bounds.hi.map(|x| x + reach),
            };
            candidates.clear();
            self.bvh.search(&wide, &mut stack, |j| {
                if j > i && self.boxes.get(j as usize).is_some_and(|b| b.meets(&wide)) {
                    candidates.push(j);
                }
            });
            candidates.sort_unstable();
            let wa = self.names(i);
            let face = self.face(i);
            for &j in &candidates {
                if rules
                    .looked_at
                    .is_some_and(|looked_at| !looked_at(face, self.face(j)))
                {
                    continue;
                }
                let Some(b) = self.corners(j) else {
                    continue;
                };
                let met = if self.boxes.get(j as usize).is_some_and(|b| b.meets(bounds)) {
                    meeting(&a, &wa, &b, &self.names(j))
                } else {
                    None
                };
                let pair = if met == Some(true) {
                    Some((true, false))
                } else {
                    let other = self.face(j);
                    let near = if reach > 0.0 && other != face {
                        (rules.near)(face, other)
                    } else {
                        0.0
                    };
                    if near > 0.0 && lie_on_each_other(&a, &b, near) {
                        Some((true, true))
                    } else {
                        met.map(|overlap| (overlap, false))
                    }
                };
                if let Some((overlap, nearly)) = pair {
                    if out.len() >= cap {
                        return (out, true);
                    }
                    out.push(Pair {
                        i,
                        j,
                        overlap,
                        nearly,
                    });
                }
            }
        }
        (out, false)
    }

    /// All pairs that meet, in order, at most `cap`.
    fn pairs(
        &self,
        threads: usize,
        cap: usize,
        rules: &Rules<'_>,
    ) -> Result<(Vec<Pair>, bool), TessError> {
        let count = self.mesh.triangles.len();
        let chunks = count.div_ceil(CHUNK);
        let one = |chunk: usize| {
            let first = chunk.saturating_mul(CHUNK);
            self.pairs_in(first..first.saturating_add(CHUNK).min(count), cap, rules)
        };
        let wanted = if threads == 0 {
            automatic_threads()
        } else {
            threads
        };
        let threads = wanted.min(chunks);
        let mut done: Vec<(usize, (Vec<Pair>, bool))> = Vec::new();
        if threads <= 1 || cfg!(target_arch = "wasm32") {
            done.extend((0..chunks).map(|chunk| (chunk, one(chunk))));
        } else {
            // Each worker takes the next chunk not yet taken; which worker
            // does which varies, the results do not.
            // The work counted for a bound (`craft3d_geom::allowance`) is
            // added to this thread's as each worker is joined.
            let next = AtomicUsize::new(0);
            let work = || {
                let before = craft3d_geom::allowance::spent();
                let mut mine = Vec::new();
                loop {
                    let chunk = next.fetch_add(1, Ordering::Relaxed);
                    if chunk >= chunks {
                        return (
                            mine,
                            craft3d_geom::allowance::spent().saturating_sub(before),
                        );
                    }
                    mine.push((chunk, one(chunk)));
                }
            };
            let mut panicked = false;
            std::thread::scope(|scope| {
                let mut workers = Vec::new();
                for _ in 1..threads {
                    // A thread that cannot be started is not an error: the
                    // others do its share.
                    if let Ok(worker) = std::thread::Builder::new().spawn_scoped(scope, work) {
                        workers.push(worker);
                    }
                }
                done = work().0;
                for worker in workers {
                    match worker.join() {
                        Ok((more, spent)) => {
                            done.extend(more);
                            craft3d_geom::allowance::charge(spent);
                        }
                        Err(_) => panicked = true,
                    }
                }
            });
            if panicked {
                return Err(TessError::Internal(
                    "a thread searching for intersections stopped unexpectedly".into(),
                ));
            }
        }
        if done.len() != chunks {
            return Err(TessError::Internal(
                "part of the mesh was not searched".into(),
            ));
        }
        done.sort_unstable_by_key(|&(chunk, _)| chunk);
        let mut all = Vec::new();
        let mut truncated = false;
        for (_, (pairs, more)) in done {
            truncated |= more;
            all.extend(pairs);
        }
        if all.len() > cap {
            all.truncate(cap);
            truncated = true;
        }
        Ok((all, truncated))
    }
}

/// Which triangles are on each edge: `(corner name, corner name,
/// triangle)` with the smaller name first, sorted.
struct EdgeMap {
    uses: Vec<(u32, u32, u32)>,
}

impl EdgeMap {
    fn new(mesh: &Prepared<'_>) -> Self {
        let mut uses = Vec::new();
        for (t, names) in mesh.names.iter().enumerate() {
            if !mesh.solid.get(t).copied().unwrap_or(false) {
                continue;
            }
            for (i, j) in EDGES {
                let (a, b) = (pick(names, i), pick(names, j));
                uses.push((a.min(b), a.max(b), t as u32));
            }
        }
        uses.sort_unstable();
        Self { uses }
    }

    /// The triangles on the edge between corners `a` and `b`.
    fn on(&self, a: u32, b: u32) -> impl Iterator<Item = u32> + '_ {
        let key = (a.min(b), a.max(b));
        let first = self.uses.partition_point(|&(p, q, _)| (p, q) < key);
        self.uses
            .iter()
            .skip(first)
            .take_while(move |&&(p, q, _)| (p, q) == key)
            .map(|&(_, _, t)| t)
    }
}

/// The column over a triangle: the points whose foot on its plane is in
/// it, narrowed by an offset per edge.
struct Column {
    origin: Vec3,
    normal: Vec3,
    /// Per edge: a point on it, the unit vector into the triangle, and how
    /// far in the wall stands.
    walls: [(Vec3, Vec3, f64); 3],
}

impl Column {
    fn new(t: &[Vec3; 3], offsets: [f64; 3]) -> Option<Column> {
        let [a, b, c] = *t;
        let normal = (b - a).cross(c - a).normalized()?;
        let wall = |from: Vec3, to: Vec3, offset: f64| {
            Some((from, normal.cross(to - from).normalized()?, offset))
        };
        let [oa, ob, oc] = offsets;
        Some(Column {
            origin: a,
            normal,
            walls: [wall(a, b, oa)?, wall(b, c, ob)?, wall(c, a, oc)?],
        })
    }

    fn height(&self, p: Vec3) -> f64 {
        (p - self.origin).dot(self.normal)
    }

    /// The part of the polygon inside the column (Sutherland-Hodgman).
    fn clip(&self, polygon: &[Vec3]) -> Vec<Vec3> {
        let mut current = polygon.to_vec();
        for &(on, inward, offset) in &self.walls {
            let inside = |p: Vec3| (p - on).dot(inward) - offset;
            let mut next = Vec::with_capacity(current.len() + 1);
            let Some(&last) = current.last() else {
                return current;
            };
            let mut previous = last;
            let mut previous_in = inside(previous);
            for &p in &current {
                let p_in = inside(p);
                if (p_in >= 0.0) != (previous_in >= 0.0) {
                    let span = previous_in - p_in;
                    if span != 0.0 {
                        next.push(previous.lerp(p, previous_in / span));
                    }
                }
                if p_in >= 0.0 {
                    next.push(p);
                }
                previous = p;
                previous_in = p_in;
            }
            current = next;
        }
        current
    }

    /// Is a piece of the segment, longer than nothing, inside the column?
    fn passes(&self, p: Vec3, q: Vec3) -> bool {
        let (mut from, mut to) = (0.0_f64, 1.0_f64);
        for &(on, inward, offset) in &self.walls {
            let (dp, dq) = ((p - on).dot(inward) - offset, (q - on).dot(inward) - offset);
            if dp < 0.0 && dq < 0.0 {
                return false;
            }
            if dp < 0.0 {
                from = from.max(dp / (dp - dq));
            } else if dq < 0.0 {
                to = to.min(dp / (dp - dq));
            }
        }
        from < to
    }
}

/// False for zero, a negative number and NaN.
fn positive(x: f64) -> bool {
    x > 0.0
}

/// Twice the area of a polygon in space.
fn double_area(polygon: &[Vec3]) -> f64 {
    let Some((&first, rest)) = polygon.split_first() else {
        return 0.0;
    };
    let mut total = Vec3::ZERO;
    for (p, q) in rest.iter().zip(rest.iter().skip(1)) {
        total += (*p - first).cross(*q - first);
    }
    total.length()
}

/// The search for depths: the prepared mesh with its edges.
struct Depths<'a, 'm> {
    mesh: &'a Prepared<'m>,
    edges: EdgeMap,
}

impl Depths<'_, '_> {
    /// The column over triangle `t`, a hair wider than the triangle where
    /// the surface goes on beyond it: what stands exactly on an edge of
    /// `t` is inside when, seen along the normal of `t`, a neighbour
    /// continues on the other side of that edge. Then it passes through
    /// the surface around `t` there, between the two.
    ///
    /// Not across an edge where the surface turns away square or folds
    /// back (seen along the normal, the neighbour does not reach beyond
    /// the edge), nor across a border: there the column ends a hair
    /// inside the edge. What stands on such an edge has the surface
    /// around `t` on one side only: it touches. A body whose
    /// parts meet along an edge has four faces there, two and two in one
    /// plane; each lies in the wall of the column over the next, and with
    /// a hair to spare all round every one of them would pass "through"
    /// its neighbour by going around its end.
    fn column(&self, t: u32) -> Option<Column> {
        let corners = self.mesh.points(t)?;
        let [a, b, c] = corners;
        let size = a.distance(b).max(b.distance(c)).max(c.distance(a));
        let hair = 1e-9 * size + 16.0 * f64::EPSILON * self.mesh.largest;
        let tight = Column::new(&corners, [0.0; 3])?;
        let names = self.mesh.names(t);
        let mut offsets = [0.0; 3];
        for ((slot, (i, j)), &(on, inward, _)) in offsets.iter_mut().zip(EDGES).zip(&tight.walls) {
            let (from, to) = (pick(&names, i), pick(&names, j));
            let goes_on = self.edges.on(from, to).any(|next| {
                next != t
                    && self
                        .mesh
                        .names(next)
                        .iter()
                        .zip(self.mesh.points(next).iter().flatten())
                        .any(|(&name, &corner)| {
                            name != from && name != to && (corner - on).dot(inward) < -hair
                        })
            });
            // Out by a hair, or in by one: what lies in the plane of the
            // wall, to rounding, is then on one side of it for certain.
            *slot = if goes_on { -hair } else { hair };
        }
        Column::new(&corners, offsets)
    }

    /// How far the surface around triangle `x` passes through triangle
    /// `y`: the smaller of how far it reaches on either side of the plane
    /// of `y`, inside the column over `y`. Followed until both sides are
    /// beyond `enough`.
    fn through(&self, x: u32, y: u32, enough: f64) -> f64 {
        let Some(column) = self.column(y) else {
            return 0.0;
        };
        let (mut above, mut below) = (0.0_f64, 0.0_f64);
        let mut seen = BTreeSet::from([x]);
        let mut queue = std::collections::VecDeque::from([x]);
        while let Some(t) = queue.pop_front() {
            let Some(corners) = self.mesh.points(t) else {
                continue;
            };
            let inside = column.clip(&corners);
            if inside.len() < 3 || !positive(double_area(&inside)) {
                continue;
            }
            let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
            for &p in &inside {
                let h = column.height(p);
                low = low.min(h);
                high = high.max(h);
            }
            above = above.max(high);
            below = below.max(-low);
            if above > enough && below > enough {
                break;
            }
            // Nothing comes back from beyond: the other side is reached
            // through `y` only, which is near.
            if low > enough || high < -enough {
                continue;
            }
            let names = self.mesh.names(t);
            for (i, j) in EDGES {
                if !column.passes(pick(&corners, i), pick(&corners, j)) {
                    continue;
                }
                for next in self.edges.on(pick(&names, i), pick(&names, j)) {
                    if next != y && seen.len() < PATCH_CAP && seen.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
        }
        above.min(below)
    }

    /// How wide the common area of two triangles in one plane is: twice
    /// its area over its perimeter, which for a disc is its radius and for
    /// a strip its width. Two triangles that share a corner or an edge
    /// and no area have a common "area" that is a rounding error wide.
    fn overlap(&self, i: u32, j: u32) -> f64 {
        let (Some(a), Some(b)) = (self.mesh.points(i), self.mesh.points(j)) else {
            return 0.0;
        };
        let Some(column) = Column::new(&b, [0.0; 3]) else {
            return 0.0;
        };
        let common = column.clip(&a);
        let area = double_area(&common);
        let Some(&last) = common.last() else {
            return 0.0;
        };
        let mut perimeter = 0.0;
        let mut previous = last;
        for &p in &common {
            perimeter += previous.distance(p);
            previous = p;
        }
        if common.len() < 3 || !positive(area) || !positive(perimeter) {
            return 0.0;
        }
        area / perimeter
    }

    /// A point where two crossing triangles meet: the mean of where the
    /// edges of one go through the other.
    fn place(&self, i: u32, j: u32, overlap: bool) -> Vec3 {
        let (Some(a), Some(b)) = (self.mesh.points(i), self.mesh.points(j)) else {
            return Vec3::ZERO;
        };
        let mean = |points: &[Vec3]| {
            let mut total = Vec3::ZERO;
            for &p in points {
                total += p * (1.0 / points.len().max(1) as f64);
            }
            total
        };
        let mut found = Vec::new();
        if overlap {
            if let Some(column) = Column::new(&b, [0.0; 3]) {
                found = column.clip(&a);
            }
        } else {
            for (x, y) in [(&a, &b), (&b, &a)] {
                let Some(column) = Column::new(y, [0.0; 3]) else {
                    continue;
                };
                // A little slack: a point on an edge is in.
                let slack = 1e-9 * (pick(y, 1) - pick(y, 0)).length();
                for (from, to) in EDGES {
                    let (p, q) = (pick(x, from), pick(x, to));
                    let (hp, hq) = (column.height(p), column.height(q));
                    if hp * hq > 0.0 || hp == hq {
                        continue;
                    }
                    let at = p.lerp(q, hp / (hp - hq));
                    if column
                        .walls
                        .iter()
                        .all(|&(on, inward, _)| (at - on).dot(inward) >= -slack)
                    {
                        found.push(at);
                    }
                }
            }
        }
        if found.is_empty() {
            mean(&[mean(&a), mean(&b)])
        } else {
            mean(&found)
        }
    }
}

/// Every pair of triangles of `mesh` that meets, with its depth.
pub(crate) fn contacts(
    mesh: &Mesh,
    rules: &Rules<'_>,
    threads: usize,
    cap: usize,
) -> Result<Contacts, TessError> {
    let prepared = Prepared::new(mesh)?;
    let (pairs, truncated) = prepared.pairs(threads, cap, rules)?;
    // Heights are dot products with a normalised normal of points that
    // were clipped three times: some dozens of rounding errors of the
    // largest coordinate, with room to spare.
    let rounding = 1024.0 * f64::EPSILON * prepared.largest;
    let mut contacts = Vec::with_capacity(pairs.len());
    if !pairs.is_empty() {
        let depths = Depths {
            edges: EdgeMap::new(&prepared),
            mesh: &prepared,
        };
        for Pair {
            i,
            j,
            overlap,
            nearly,
        } in pairs
        {
            let faces = [prepared.face(i), prepared.face(j)];
            let depth = if overlap {
                let depth = depths.overlap(i, j);
                // Side by side in nearly one plane, sharing no area: two
                // faces of one flat side of a body. Nothing.
                if nearly && depth <= rounding {
                    continue;
                }
                depth
            } else {
                let [fi, fj] = faces;
                let margin = (rules.margin)(fi, fj).max(0.0);
                // With no margin the whole patch is followed.
                let enough = if margin > 0.0 {
                    4.0 * margin
                } else {
                    f64::INFINITY
                };
                // The smaller of the two: see the module documentation.
                let forth = depths.through(i, j, enough);
                if forth > 0.0 {
                    forth.min(depths.through(j, i, enough))
                } else {
                    0.0
                }
            };
            contacts.push(Crossing {
                triangles: [i as usize, j as usize],
                faces,
                at: depths.place(i, j, overlap),
                kind: if overlap {
                    CrossingKind::CoplanarOverlap
                } else {
                    CrossingKind::Crossing
                },
                depth,
            });
        }
    }
    Ok(Contacts {
        contacts,
        degenerate: prepared.degenerate,
        truncated,
        rounding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [P; 3] {
        [a, b, c]
    }

    const APART: ([u32; 3], [u32; 3]) = ([0, 1, 2], [3, 4, 5]);

    fn meet(a: &[P; 3], b: &[P; 3]) -> Option<bool> {
        let both = meeting(a, &APART.0, b, &APART.1);
        // The same either way round, and however the corners are ordered.
        assert_eq!(meeting(b, &APART.1, a, &APART.0), both);
        let turned = [a[1], a[2], a[0]];
        assert_eq!(meeting(&turned, &APART.0, b, &APART.1), both);
        let flipped = [b[0], b[2], b[1]];
        assert_eq!(meeting(a, &APART.0, &flipped, &APART.1), both);
        both
    }

    const GROUND: [P; 3] = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]];

    #[test]
    fn triangles_apart_do_not_meet() {
        let above = tri([0.0, 0.0, 1.0], [4.0, 0.0, 1.0], [0.0, 4.0, 2.0]);
        assert_eq!(meet(&GROUND, &above), None);
        // Crossing the plane, but beside the triangle.
        let beside = tri([5.0, 5.0, -1.0], [6.0, 5.0, 1.0], [5.0, 6.0, 1.0]);
        assert_eq!(meet(&GROUND, &beside), None);
        // In the plane, beside it.
        let flat = tri([3.0, 3.0, 0.0], [5.0, 3.0, 0.0], [3.0, 5.0, 0.0]);
        assert_eq!(meet(&GROUND, &flat), None);
    }

    #[test]
    fn a_triangle_through_another_meets_it() {
        let through = tri([1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [3.0, 3.0, 1.0]);
        assert_eq!(meet(&GROUND, &through), Some(false));
        // One corner exactly on the other triangle.
        let standing = tri([1.0, 1.0, 0.0], [1.0, 1.0, 1.0], [2.0, 1.0, 1.0]);
        assert_eq!(meet(&GROUND, &standing), Some(false));
        // An edge lying in it.
        let lying = tri([0.5, 0.5, 0.0], [1.5, 0.5, 0.0], [1.0, 0.5, 1.0]);
        assert_eq!(meet(&GROUND, &lying), Some(false));
        // Edges that cross in one point.
        let edge = tri([2.0, -1.0, -1.0], [2.0, -1.0, 1.0], [2.0, 1.0, 0.0]);
        assert_eq!(meet(&GROUND, &edge), Some(false));
    }

    #[test]
    fn triangles_in_one_plane() {
        let over = tri([1.0, 1.0, 0.0], [5.0, 1.0, 0.0], [1.0, 5.0, 0.0]);
        assert_eq!(meet(&GROUND, &over), Some(true));
        let inside = tri([0.5, 0.5, 0.0], [1.0, 0.5, 0.0], [0.5, 1.0, 0.0]);
        assert_eq!(meet(&GROUND, &inside), Some(true));
        // Along an edge only: contact, no area.
        let along = tri([1.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, -1.0, 0.0]);
        assert_eq!(meet(&GROUND, &along), Some(false));
        // A corner on an edge.
        let tip = tri([2.0, 2.0, 0.0], [5.0, 3.0, 0.0], [3.0, 5.0, 0.0]);
        assert_eq!(meet(&GROUND, &tip), Some(false));
    }

    #[test]
    fn neighbours_meet_only_beyond_what_they_share() {
        // A shared edge 0-1 of the ground triangle.
        let shared = |c: [f64; 3]| {
            let other = tri(GROUND[1], GROUND[0], c);
            meeting(&GROUND, &[0, 1, 2], &other, &[1, 0, 3])
        };
        assert_eq!(shared([1.0, -3.0, 0.0]), None, "flat, on the other side");
        assert_eq!(shared([1.0, 1.0, 2.0]), None, "folded up");
        assert_eq!(shared([1.0, 1.0, 1e-300]), None, "folded up by nothing");
        assert_eq!(shared([1.0, 1.0, 0.0]), Some(true), "folded flat");
        assert_eq!(shared([9.0, 9.0, 0.0]), Some(true), "folded flat, larger");

        // A shared corner 0.
        let corner = |b: [f64; 3], c: [f64; 3]| {
            let other = tri(GROUND[0], b, c);
            let found = meeting(&GROUND, &[0, 1, 2], &other, &[0, 3, 4]);
            assert_eq!(meeting(&other, &[0, 3, 4], &GROUND, &[0, 1, 2]), found);
            found
        };
        assert_eq!(
            corner([-1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
            None,
            "flat, apart"
        );
        assert_eq!(corner([-1.0, 0.0, 1.0], [0.0, -1.0, 1.0]), None);
        assert_eq!(corner([1.0, 1.0, 1.0], [1.0, 1.0, 2.0]), None, "above");
        assert_eq!(
            corner([1.0, 1.0, -1.0], [1.0, 1.0, 1.0]),
            Some(false),
            "through"
        );
        assert_eq!(
            corner([3.0, 3.0, -1.0], [3.0, 3.0, 1.0]),
            Some(false),
            "through, leaving by the far edge"
        );
        assert_eq!(
            corner([-3.0, -3.0, -1.0], [-3.0, -3.0, 1.0]),
            None,
            "through the plane, away from the triangle"
        );
        assert_eq!(
            corner([1.0, 0.5, 0.0], [0.5, 1.0, 0.0]),
            Some(true),
            "flat, inside"
        );
        assert_eq!(
            corner([2.0, 0.0, 0.0], [1.0, -1.0, 0.0]),
            Some(false),
            "flat, along an edge"
        );
        // The same triangle, either way up.
        assert_eq!(
            meeting(&GROUND, &[0, 1, 2], &GROUND, &[0, 1, 2]),
            Some(true)
        );
        let turned = [GROUND[0], GROUND[2], GROUND[1]];
        assert_eq!(
            meeting(&GROUND, &[0, 1, 2], &turned, &[0, 2, 1]),
            Some(true)
        );
    }

    /// Triangles a hair apart, or a hair through each other, at sizes
    /// where the hair is far below the rounding of a plain determinant.
    #[test]
    fn a_hair_decides() {
        for scale in [1e-3, 1.0, 1e6] {
            let hair = scale * f64::EPSILON;
            let ground = GROUND.map(|p| p.map(|x| x * scale));
            let over = |z: f64| {
                tri(
                    [scale, scale, z],
                    [scale, scale, scale],
                    [2.0 * scale, scale, scale],
                )
            };
            assert_eq!(meet(&ground, &over(hair)), None, "{scale}");
            assert_eq!(meet(&ground, &over(0.0)), Some(false), "{scale}");
            assert_eq!(meet(&ground, &over(-hair)), Some(false), "{scale}");
        }
    }

    #[test]
    fn the_hierarchy_finds_what_brute_force_finds() {
        // Boxes scattered by a small generator, some of them overlapping.
        let mut state = 5_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as f64 / (1u64 << 31) as f64
        };
        let boxes: Vec<Aabb> = (0..500)
            .map(|_| {
                let lo = [next() * 10.0, next() * 10.0, next() * 10.0];
                let size = [next() * 3.0, next() * 0.6, next() * 6.0];
                Aabb {
                    lo,
                    hi: [lo[0] + size[0], lo[1] + size[1], lo[2] + size[2]],
                }
            })
            .collect();
        let bvh = Bvh::build(&boxes, (0..500).collect());
        let mut sorted = bvh.items.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..500).collect::<Vec<u32>>());
        let mut stack = Vec::new();
        let mut total = 0;
        for (i, query) in boxes.iter().enumerate() {
            let mut found = Vec::new();
            bvh.search(query, &mut stack, |j| {
                if boxes[j as usize].meets(query) {
                    found.push(j);
                }
            });
            found.sort_unstable();
            let brute: Vec<u32> = (0..500)
                .filter(|&j| boxes[j as usize].meets(query))
                .collect();
            assert_eq!(found, brute, "box {i}");
            total += brute.len();
        }
        assert!(total > 1000, "the boxes hardly overlap: {total}");
        // An empty hierarchy finds nothing and does not fall over.
        let empty = Bvh::build(&boxes, Vec::new());
        empty.search(&boxes[0], &mut stack, |_| panic!("found something"));
    }

    /// No more pairs are kept than the cap, per chunk and in all.
    #[test]
    fn the_cap_on_pairs_is_kept() {
        // A hundred triangles through one line: every pair meets.
        let mut mesh = Mesh::default();
        for k in 0..100 {
            let angle = f64::from(k) * 0.03;
            let base = mesh.positions.len() as u32;
            mesh.positions.extend([
                Vec3::new(0.0, 0.0, -1.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(angle.cos(), angle.sin(), 0.1),
            ]);
            // Not neighbours: each a little further along the line.
            for p in mesh.positions.iter_mut().skip(base as usize).take(2) {
                p.z += f64::from(k) * 1e-3;
            }
            mesh.normals.extend([Vec3::ZERO; 3]);
            mesh.triangles.push([base, base + 1, base + 2]);
            mesh.face_ids.push(0);
        }
        let prepared = Prepared::new(&mesh).unwrap();
        let rules = Rules {
            margin: &|_, _| 0.0,
            near: &|_, _| 0.0,
            reach: 0.0,
            looked_at: None,
        };
        let (all, more) = prepared.pairs(1, 1_000_000, &rules).unwrap();
        assert!(all.len() > 1000 && !more, "{}", all.len());
        for cap in [0, 1, 10, 999] {
            let (some, more) = prepared.pairs(1, cap, &rules).unwrap();
            assert_eq!(some.len(), cap);
            assert!(more);
            assert!(some == all[..cap]);
            let (chunk, more) = prepared.pairs_in(0..100, cap, &rules);
            assert_eq!(chunk.len(), cap);
            assert!(more);
        }
    }

    #[test]
    fn a_column_clips() {
        let column = Column::new(&GROUND.map(|p| vector(&p)), [0.0, 0.0, 0.5]).unwrap();
        // A big square at height 3: what is left is the triangle less a
        // strip of 0.5 along x = 0.
        let square = [
            Vec3::new(-9.0, -9.0, 3.0),
            Vec3::new(9.0, -9.0, 3.0),
            Vec3::new(9.0, 9.0, 3.0),
            Vec3::new(-9.0, 9.0, 3.0),
        ];
        let inside = column.clip(&square);
        assert!((double_area(&inside) - 3.5 * 3.5).abs() < 1e-12);
        assert!(inside.iter().all(|&p| column.height(p) == 3.0));
        assert!(column.passes(Vec3::new(-1.0, 1.0, 0.0), Vec3::new(2.0, 1.0, 5.0)));
        assert!(!column.passes(Vec3::new(-1.0, 1.0, 0.0), Vec3::new(0.4, 1.0, 5.0)));
        assert!(!column.passes(Vec3::new(5.0, 5.0, 0.0), Vec3::new(6.0, 5.0, 5.0)));
    }
}
