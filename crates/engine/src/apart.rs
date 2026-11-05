//! A proof, from meshes, that two solids have no point in common: the
//! test between the box test and the exact boolean of a collision query
//! (`document.interference`, the overlap measure, a motion's collisions).
//! Pairs it proves apart are what the exact query answers for them (clear,
//! no common volume) at the cost of a mesh distance; every other pair goes
//! to the exact query as before, so every touch, overlap volume, contact
//! area and refusal reported is the kernel's own (docs/design/booleans.md
//! › *A proof of apart before the boolean*).
//!
//! **The argument.** Each body is meshed (`craft3d_tess::tessellate_body`)
//! at a chordal tolerance `ε` of its own; the tessellator's promise is that
//! every point of a face lies within `ε` of the face's triangles (the
//! promise `verify::Part::err` builds the clearance measure's lower bound
//! on). So if the meshes are further apart than `D`, the surfaces are
//! further apart than `D − εa − εb`. `D` is taken as
//! `2 (εa + εb) + 2 w + s`: twice the promise, twice the working
//! tolerance `w` (the largest of the document's and every entity
//! tolerance of either body) within which the boolean takes faces for
//! touching (the box test before it grows the boxes by the same), and `s`
//! a rounding slack of 1e-8 of the bodies' extent and their distance from
//! the origin. Surfaces that far apart have no point in common, unless one
//! solid lies inside the other: a corner of each mesh is then tested
//! against the other (its winding number, a sum of solid angles, must be 0
//! within a tenth; the corner is further than `D` from the other mesh, so
//! the sum is well conditioned). Whatever the test cannot settle (a mesh
//! that fails, a winding number not near 0, meshes within `D`) is not
//! proven, never "apart".
//!
//! **Static data once.** A placed body (an assembly's instance, moved by
//! its pose, `meshes::note_placed`) is tested through the body it was
//! placed from: that body's mesh and its tree of boxes are made once and
//! kept ([`SHAPES`]), and each moment of a motion only brings the pose. The
//! chordal tolerances stand on a ladder of powers of two (1/8192 of the
//! body's size first; finer, once, when the meshes come within `D` but
//! further apart than touching surfaces would leave them), so the meshes
//! made for one moment serve the next.

use craft3d_math::{Bbox3, Portable, Tolerance, Transform, Vec3};
use craft3d_tess::TessParams;
use craft3d_topo::{Body, BodyKind};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

/// Triangles per leaf of a tree of boxes.
const LEAF: usize = 8;

/// The first chordal tolerance, relative to the body's size (the diagonal
/// of its box), before it is put on the ladder of powers of two.
const FIRST: f64 = 1.0 / 8192.0;

/// The finest chordal tolerance a body is meshed at, relative to its
/// size: below it the mesh costs more than the boolean it saves.
const FINEST: f64 = 1.0 / 131_072.0;

/// Most triangles of one mesh made for the test; a body that needs more
/// at a tolerance is not proven apart at that tolerance.
const MAX_TRIANGLES: usize = 1_500_000;

/// Most bytes the shapes kept take ([`SHAPES`]); past it the least
/// recently used go.
#[cfg(not(target_arch = "wasm32"))]
const KEPT_BYTES: usize = 512 << 20;
#[cfg(target_arch = "wasm32")]
const KEPT_BYTES: usize = 64 << 20;

/// Most pairs of nodes one test looks at, so a pathological pair (two
/// fine meshes lying against each other everywhere) ends as not proven.
const MAX_VISITS: usize = 20_000_000;

/// A mesh with its tree of boxes, in the frame of the body it was made
/// of.
pub(crate) struct Shape {
    positions: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
    /// The tree: the root is node 0.
    nodes: Vec<Node>,
    /// The triangles in the order of the leaves.
    order: Vec<u32>,
    /// The chordal tolerance the mesh was made at.
    chord: f64,
    bbox: Bbox3,
}

#[derive(Clone, Copy)]
struct Node {
    lo: Vec3,
    hi: Vec3,
    /// A leaf: `order[first..first + count]`; else the children `first`
    /// and `count`.
    first: u32,
    count: u32,
    leaf: bool,
}

impl Shape {
    fn bytes(&self) -> usize {
        let v = std::mem::size_of::<Vec3>();
        self.positions
            .len()
            .saturating_mul(v)
            .saturating_add(self.triangles.len().saturating_mul(16))
            .saturating_add(self.nodes.len().saturating_mul(std::mem::size_of::<Node>()))
    }

    fn corners(&self, t: u32) -> Option<[Vec3; 3]> {
        let [a, b, c] = *self.triangles.get(t as usize)?;
        Some([
            *self.positions.get(a as usize)?,
            *self.positions.get(b as usize)?,
            *self.positions.get(c as usize)?,
        ])
    }

    /// The mesh of `body` at `chord`, with its tree; `None` when it cannot
    /// be made (the body cannot be meshed there, too many triangles, a
    /// mesh that is empty).
    fn of(body: &Body, chord: f64) -> Option<Shape> {
        let params = TessParams {
            chordal_tolerance: chord,
            max_triangles: Some(MAX_TRIANGLES),
            ..TessParams::default()
        };
        let mesh = craft3d_tess::tessellate_body(body, &params).ok()?;
        if mesh.triangles.is_empty() || !mesh.positions.iter().all(|p| p.is_finite()) {
            return None;
        }
        let count = u32::try_from(mesh.triangles.len()).ok()?;
        let mut shape = Shape {
            bbox: mesh.bbox(),
            positions: mesh.positions,
            triangles: mesh.triangles,
            nodes: Vec::new(),
            order: (0..count).collect(),
            chord,
        };
        // Every corner is a position.
        for t in 0..count {
            shape.corners(t)?;
        }
        let centroids: Vec<Vec3> = (0..count)
            .map(|t| {
                shape
                    .corners(t)
                    .map_or(Vec3::ZERO, |[a, b, c]| (a + b + c) * (1.0 / 3.0))
            })
            .collect();
        shape.nodes.push(Node {
            lo: Vec3::ZERO,
            hi: Vec3::ZERO,
            first: 0,
            count,
            leaf: true,
        });
        // Split nodes until their leaves are small: a stack of nodes to
        // look at, each splitting its range of `order` at the median of
        // the centroids along the longest side of its box.
        let mut stack = vec![0_usize];
        let mut guard = 0_usize;
        while let Some(at) = stack.pop() {
            guard = guard.saturating_add(1);
            if guard > 4 * count as usize + 8 {
                return None;
            }
            let node = *shape.nodes.get(at)?;
            let (first, n) = (node.first as usize, node.count as usize);
            let range = shape.order.get(first..first.checked_add(n)?)?;
            let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
            let mut hi = -lo;
            for &t in range {
                for p in shape.corners(t)? {
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
            }
            if let Some(slot) = shape.nodes.get_mut(at) {
                slot.lo = lo;
                slot.hi = hi;
            }
            if n <= LEAF {
                continue;
            }
            let side = hi - lo;
            let axis = |p: Vec3| {
                if side.x >= side.y && side.x >= side.z {
                    p.x
                } else if side.y >= side.z {
                    p.y
                } else {
                    p.z
                }
            };
            let range = shape.order.get_mut(first..first + n)?;
            range.sort_by(|&a, &b| {
                let ca = centroids.get(a as usize).map_or(0.0, |c| axis(*c));
                let cb = centroids.get(b as usize).map_or(0.0, |c| axis(*c));
                ca.total_cmp(&cb).then(a.cmp(&b))
            });
            let half = n / 2;
            let left = u32::try_from(shape.nodes.len()).ok()?;
            let make = |first: usize, count: usize| -> Option<Node> {
                Some(Node {
                    lo: Vec3::ZERO,
                    hi: Vec3::ZERO,
                    first: u32::try_from(first).ok()?,
                    count: u32::try_from(count).ok()?,
                    leaf: true,
                })
            };
            shape.nodes.push(make(first, half)?);
            shape.nodes.push(make(first + half, n - half)?);
            if let Some(slot) = shape.nodes.get_mut(at) {
                slot.first = left;
                slot.count = left.checked_add(1)?;
                slot.leaf = false;
            }
            stack.push(left as usize);
            stack.push(left as usize + 1);
        }
        Some(shape)
    }
}

/// A shape kept: the body it is of (held weakly, so the address is not
/// another body's while the entry exists), the tolerance's rung, and the
/// shape, made once by whichever caller asks first (others wait for it).
struct Kept {
    body: Weak<Body>,
    shape: Arc<OnceLock<Option<Arc<Shape>>>>,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Store {
    by_body: BTreeMap<(usize, i32), Kept>,
    bytes: usize,
    tick: u64,
}

/// The shapes made so far, by the address of their body and the rung of
/// their tolerance: a function of the body and the rung alone, so which
/// caller made one changes nothing.
static SHAPES: Mutex<Option<Store>> = Mutex::new(None);

/// The shape of `body` at the chordal tolerance `2^rung`.
fn shape(body: &Arc<Body>, rung: i32) -> Option<Arc<Shape>> {
    let key = (Arc::as_ptr(body) as usize, rung);
    let cell = {
        let mut guard = SHAPES.lock().unwrap_or_else(PoisonError::into_inner);
        let store = guard.get_or_insert_with(Store::default);
        store.tick = store.tick.wrapping_add(1);
        let tick = store.tick;
        let fresh = store
            .by_body
            .get(&key)
            .is_none_or(|k| !std::ptr::eq(k.body.as_ptr(), Arc::as_ptr(body)));
        if fresh {
            if let Some(old) = store.by_body.remove(&key) {
                store.bytes = store.bytes.saturating_sub(old.bytes);
            }
            store.by_body.insert(
                key,
                Kept {
                    body: Arc::downgrade(body),
                    shape: Arc::new(OnceLock::new()),
                    bytes: 0,
                    used: tick,
                },
            );
        }
        let kept = store.by_body.get_mut(&key)?;
        kept.used = tick;
        Arc::clone(&kept.shape)
    };
    let made = cell
        .get_or_init(|| Shape::of(body, pow2(rung)).map(Arc::new))
        .clone();
    // Counted, and the store held to its budget.
    let mut guard = SHAPES.lock().unwrap_or_else(PoisonError::into_inner);
    let store = guard.get_or_insert_with(Store::default);
    if let Some(kept) = store.by_body.get_mut(&key)
        && kept.bytes == 0
        && Arc::ptr_eq(&kept.shape, &cell)
    {
        kept.bytes = made.as_ref().map_or(64, |s| s.bytes()).max(64);
        store.bytes = store.bytes.saturating_add(kept.bytes);
    }
    let gone: Vec<(usize, i32)> = store
        .by_body
        .iter()
        .filter(|(_, k)| k.body.strong_count() == 0)
        .map(|(key, _)| *key)
        .collect();
    for key in gone {
        if let Some(old) = store.by_body.remove(&key) {
            store.bytes = store.bytes.saturating_sub(old.bytes);
        }
    }
    while store.bytes > KEPT_BYTES && store.by_body.len() > 1 {
        let Some(oldest) = store
            .by_body
            .iter()
            .filter(|(k, _)| **k != key)
            .min_by_key(|(_, k)| k.used)
            .map(|(key, _)| *key)
        else {
            break;
        };
        if let Some(old) = store.by_body.remove(&oldest) {
            store.bytes = store.bytes.saturating_sub(old.bytes);
        }
    }
    made
}

/// Lets go of the shapes of bodies that are gone (a document closed):
/// those of live bodies stay, whichever session has them.
pub(crate) fn forget_gone_shapes() {
    let mut guard = SHAPES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(store) = guard.as_mut() {
        store.by_body.retain(|_, k| k.body.strong_count() > 0);
        store.bytes = store.by_body.values().map(|k| k.bytes).sum();
    }
}

/// `2^k`, exactly (for `k` within the normal range).
fn pow2(k: i32) -> f64 {
    let k = k.clamp(-1000, 1000);
    f64::from_bits(((k + 1023) as u64) << 52)
}

/// The rung at or below `x`: the `k` of the largest `2^k ≤ x`, for a
/// positive normal `x`.
fn rung_below(x: f64) -> Option<i32> {
    if !(x.is_finite() && x >= f64::MIN_POSITIVE) {
        return None;
    }
    let exponent = ((x.to_bits() >> 52) & 0x7ff) as i32;
    Some(exponent - 1023)
}

/// How many pairs the test proved apart and how many it left to the
/// exact query, since the process started (for the record of a run).
pub(crate) static PROVEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub(crate) static LEFT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Whether `a` and `b` are proven to have no point in common within the
/// tolerance: then the exact query answers clear (no common volume) for
/// them. False when that is not proven, which says nothing.
pub(crate) fn proven_apart(a: &Arc<Body>, b: &Arc<Body>, tolerance: &Tolerance) -> bool {
    use std::sync::atomic::Ordering;
    let proven = proven_apart_inner(a, b, tolerance);
    if proven {
        PROVEN.fetch_add(1, Ordering::Relaxed);
    } else {
        LEFT.fetch_add(1, Ordering::Relaxed);
    }
    proven
}

/// Whether the test is made at all (true but in tests that compare its
/// answers with the kernel's alone).
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turns the proof from meshes on or off for the whole process: off,
/// every pair past the box test goes to the exact query, as before
/// 2026-11-05. For tests and timings that compare the two; answers do not
/// depend on it.
pub fn set_mesh_proof(on: bool) {
    ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn proven_apart_inner(a: &Arc<Body>, b: &Arc<Body>, tolerance: &Tolerance) -> bool {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    let linear = tolerance.linear;
    if !(linear.is_finite() && linear > 0.0) {
        return false;
    }
    if a.kind != BodyKind::Solid || b.kind != BodyKind::Solid {
        return false;
    }
    let working = a.working_tolerance(linear).max(b.working_tolerance(linear));
    if !working.is_finite() {
        return false;
    }
    let (sa, pa) = crate::meshes::placed_from(a);
    let (sb, pb) = crate::meshes::placed_from(b);
    // From b's source to a's: everything is compared in a's source frame.
    let Some(back) = pa.inverse() else {
        return false;
    };
    let rel = pb.then(&back);
    if !rel.is_isometry(1e-9) || rel.determinant() <= 0.0 {
        return false;
    }
    let size = |body: &Body| {
        body.bbox()
            .ok()
            .map(|b| b.diagonal())
            .filter(|d| d.is_finite() && *d > 0.0)
    };
    let (Some(size_a), Some(size_b)) = (size(&sa), size(&sb)) else {
        return false;
    };
    let reach = pa.point(Vec3::ZERO).length() + pb.point(Vec3::ZERO).length();
    let slack = 1e-8 * (size_a + size_b + reach);
    let touch = 2.0 * working + slack;
    let first = |s: f64| rung_below((s * FIRST).max(linear));
    let finest = |s: f64| rung_below((s * FINEST).max(linear));
    let (Some(mut ka), Some(mut kb)) = (first(size_a), first(size_b)) else {
        return false;
    };
    let (Some(fa), Some(fb)) = (finest(size_a), finest(size_b)) else {
        return false;
    };
    for _ in 0..2 {
        let (Some(ma), Some(mb)) = (shape(&sa, ka), shape(&sb, kb)) else {
            return false;
        };
        let reach_d = 2.0 * (ma.chord + mb.chord) + touch;
        match nearer_than(&ma, &mb, &rel, reach_d, touch) {
            Near::Apart => return !inside_either(&ma, &mb, &rel),
            Near::Touching | Near::Unknown => return false,
            Near::Within(found) => {
                // Surfaces that touch leave meshes within the touching
                // distance and the two chords: finer meshes cannot prove
                // those apart, so they are not made.
                if found <= touch + ma.chord + mb.chord {
                    return false;
                }
                // Meshes fine enough to tell `found` from touching: an
                // eighth of the room between them each, on the ladder.
                let room = (found - touch) / 8.0;
                let Some(wanted) = rung_below(room) else {
                    return false;
                };
                // Only a body whose mesh is coarser than the room asks
                // gets a finer one, never finer than its finest.
                let (na, nb) = (wanted.max(fa).min(ka), wanted.max(fb).min(kb));
                if na == ka && nb == kb {
                    return false;
                }
                ka = na;
                kb = nb;
            }
        }
    }
    false
}

/// What [`nearer_than`] found.
enum Near {
    /// Every pair of triangles is further apart than the distance asked.
    Apart,
    /// Some pair is within the touching distance.
    Touching,
    /// The nearest pair found is this far apart, within the distance asked.
    Within(f64),
    /// The test was cut short.
    Unknown,
}

/// The boxes of `b`'s nodes in `a`'s frame (`rel` takes `b` there): each
/// node's box moved, then boxed again (a box around the moved box).
fn moved_boxes(b: &Shape, rel: &Transform) -> Vec<(Vec3, Vec3)> {
    let cols = [
        rel.vector(Vec3::X),
        rel.vector(Vec3::Y),
        rel.vector(Vec3::Z),
    ];
    let abs = |v: Vec3| Vec3::new(v.x.abs(), v.y.abs(), v.z.abs());
    b.nodes
        .iter()
        .map(|n| {
            let c = (n.lo + n.hi) * 0.5;
            let h = (n.hi - n.lo) * 0.5;
            let c = rel.point(c);
            let h = abs(cols[0]) * h.x + abs(cols[1]) * h.y + abs(cols[2]) * h.z;
            (c - h, c + h)
        })
        .collect()
}

/// The distance between two boxes (0 when they meet).
fn box_gap(alo: Vec3, ahi: Vec3, blo: Vec3, bhi: Vec3) -> f64 {
    let gap = |a0: f64, a1: f64, b0: f64, b1: f64| (b0 - a1).max(a0 - b1).max(0.0);
    let x = gap(alo.x, ahi.x, blo.x, bhi.x);
    let y = gap(alo.y, ahi.y, blo.y, bhi.y);
    let z = gap(alo.z, ahi.z, blo.z, bhi.z);
    (x * x + y * y + z * z).sqrt()
}

/// Whether the meshes `a` and `b` (moved by `rel`) come within `reach`,
/// stopping at the first pair within `touch`.
fn nearer_than(a: &Shape, b: &Shape, rel: &Transform, reach: f64, touch: f64) -> Near {
    let boxes_b = moved_boxes(b, rel);
    let moved: Vec<Vec3> = b.positions.iter().map(|&p| rel.point(p)).collect();
    let corners_b = |t: u32| -> Option<[Vec3; 3]> {
        let [i, j, k] = *b.triangles.get(t as usize)?;
        Some([
            *moved.get(i as usize)?,
            *moved.get(j as usize)?,
            *moved.get(k as usize)?,
        ])
    };
    let mut best = f64::INFINITY;
    let mut stack: Vec<(u32, u32)> = vec![(0, 0)];
    let mut visits = 0_usize;
    while let Some((ia, ib)) = stack.pop() {
        visits = visits.saturating_add(1);
        if visits > MAX_VISITS {
            return Near::Unknown;
        }
        let (Some(na), Some(&(blo, bhi)), Some(nb)) = (
            a.nodes.get(ia as usize),
            boxes_b.get(ib as usize),
            b.nodes.get(ib as usize),
        ) else {
            return Near::Unknown;
        };
        let cut = reach.min(best);
        if box_gap(na.lo, na.hi, blo, bhi) > cut {
            continue;
        }
        match (na.leaf, nb.leaf) {
            (true, true) => {
                let (fa, ca, fb, cb) = (
                    na.first as usize,
                    na.count as usize,
                    nb.first as usize,
                    nb.count as usize,
                );
                let (Some(ta), Some(tb)) = (
                    a.order.get(fa..fa.saturating_add(ca)),
                    b.order.get(fb..fb.saturating_add(cb)),
                ) else {
                    return Near::Unknown;
                };
                for &x in ta {
                    let Some(p) = a.corners(x) else {
                        return Near::Unknown;
                    };
                    let (plo, phi) = tri_box(&p);
                    for &y in tb {
                        let Some(q) = corners_b(y) else {
                            return Near::Unknown;
                        };
                        let (qlo, qhi) = tri_box(&q);
                        if box_gap(plo, phi, qlo, qhi) > reach.min(best) {
                            continue;
                        }
                        let d = triangle_distance(&p, &q);
                        if !d.is_finite() {
                            return Near::Unknown;
                        }
                        if d < best {
                            best = d;
                            if best <= touch {
                                return Near::Touching;
                            }
                        }
                    }
                }
            }
            // The larger node is split (a leaf against an inner node: the
            // inner one).
            (leaf_a, leaf_b) => {
                let size = |lo: Vec3, hi: Vec3| (hi - lo).length_squared();
                let split_a = !leaf_a && (leaf_b || size(na.lo, na.hi) >= size(blo, bhi));
                if split_a {
                    stack.push((na.first, ib));
                    stack.push((na.count, ib));
                } else {
                    stack.push((ia, nb.first));
                    stack.push((ia, nb.count));
                }
            }
        }
    }
    if best <= reach {
        Near::Within(best)
    } else {
        Near::Apart
    }
}

fn tri_box(t: &[Vec3; 3]) -> (Vec3, Vec3) {
    (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]))
}

/// Whether a corner of either mesh lies inside the other, or that cannot
/// be told: true unless both winding numbers are 0 (within a tenth).
fn inside_either(a: &Shape, b: &Shape, rel: &Transform) -> bool {
    let Some(back) = rel.inverse() else {
        return true;
    };
    let (Some(&pa), Some(&pb)) = (a.positions.first(), b.positions.first()) else {
        return true;
    };
    // A corner of b in a's frame, against a; one of a in b's, against b.
    !(outside(a, rel.point(pb)) && outside(b, back.point(pa)))
}

/// Whether `p` is outside the closed mesh `shape`: outside its box, or its
/// winding number is 0 within a tenth.
fn outside(shape: &Shape, p: Vec3) -> bool {
    if !p.is_finite() {
        return false;
    }
    if !shape.bbox.contains(p, 0.0) {
        return true;
    }
    let mut sum = 0.0;
    for t in 0..shape.triangles.len() {
        let Some([a, b, c]) = u32::try_from(t).ok().and_then(|t| shape.corners(t)) else {
            return false;
        };
        sum += solid_angle(a - p, b - p, c - p);
    }
    let winding = sum / (4.0 * std::f64::consts::PI);
    winding.is_finite() && winding.abs() < 0.1
}

/// The signed solid angle of the triangle `a b c` seen from the origin
/// (Van Oosterom and Strackee, 1983).
fn solid_angle(a: Vec3, b: Vec3, c: Vec3) -> f64 {
    let (la, lb, lc) = (a.length(), b.length(), c.length());
    let num = a.dot(b.cross(c));
    let den = la * lb * lc + a.dot(b) * lc + a.dot(c) * lb + b.dot(c) * la;
    2.0 * num.patan2(den)
}

/// The distance between two triangles: 0 when they meet; else the least
/// of the distances from each corner to the other triangle and between
/// their sides.
pub(crate) fn triangle_distance(p: &[Vec3; 3], q: &[Vec3; 3]) -> f64 {
    let sides = |t: &[Vec3; 3]| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])];
    for (s0, s1) in sides(p) {
        if segment_crosses(s0, s1, q) {
            return 0.0;
        }
    }
    for (s0, s1) in sides(q) {
        if segment_crosses(s0, s1, p) {
            return 0.0;
        }
    }
    let mut best = f64::INFINITY;
    for &v in p {
        best = best.min(point_triangle(v, q));
    }
    for &v in q {
        best = best.min(point_triangle(v, p));
    }
    for (a0, a1) in sides(p) {
        for (b0, b1) in sides(q) {
            best = best.min(segment_segment(a0, a1, b0, b1));
        }
    }
    best
}

/// Whether the segment `s0 s1` passes through the triangle (its ends on
/// opposite sides of the triangle's plane, and the crossing inside it).
/// A segment that only grazes it is found by the distances instead.
fn segment_crosses(s0: Vec3, s1: Vec3, t: &[Vec3; 3]) -> bool {
    let [a, b, c] = *t;
    let n = (b - a).cross(c - a);
    let d0 = n.dot(s0 - a);
    let d1 = n.dot(s1 - a);
    if !(d0 < 0.0 && d1 > 0.0 || d0 > 0.0 && d1 < 0.0) {
        return false;
    }
    let x = s0 + (s1 - s0) * (d0 / (d0 - d1));
    let e = [
        (b - a).cross(x - a).dot(n),
        (c - b).cross(x - b).dot(n),
        (a - c).cross(x - c).dot(n),
    ];
    e.iter().all(|&s| s >= 0.0) || e.iter().all(|&s| s <= 0.0)
}

/// The distance from `p` to the triangle `t` (Ericson, *Real-Time
/// Collision Detection*, 5.1.5: the nearest point by its Voronoi region).
fn point_triangle(p: Vec3, t: &[Vec3; 3]) -> f64 {
    let [a, b, c] = *t;
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return p.distance(a);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return p.distance(b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return p.distance(a + ab * v);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return p.distance(c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return p.distance(a + ac * w);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return p.distance(b + (c - b) * w);
    }
    let sum = va + vb + vc;
    if !(sum.is_finite() && sum > 0.0) {
        // A triangle without area: its sides.
        return segment_point(a, b, p)
            .min(segment_point(b, c, p))
            .min(segment_point(c, a, p));
    }
    let (v, w) = (vb / sum, vc / sum);
    p.distance(a + ab * v + ac * w)
}

fn segment_point(a: Vec3, b: Vec3, p: Vec3) -> f64 {
    let ab = b - a;
    let l = ab.length_squared();
    let s = if l > 0.0 {
        ((p - a).dot(ab) / l).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * s)
}

/// The distance between the segments `p0 p1` and `q0 q1` (Ericson, 5.1.9).
fn segment_segment(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> f64 {
    let (d1, d2, r) = (p1 - p0, q1 - q0, p0 - q0);
    let (a, e, f) = (d1.length_squared(), d2.length_squared(), d2.dot(r));
    if a <= 0.0 && e <= 0.0 {
        return p0.distance(q0);
    }
    let (s, t) = if a <= 0.0 {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= 0.0 {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s = if denom > 0.0 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    (p0 + d1 * s).distance(q0 + d2 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3::new(x, y, z)
    }

    #[test]
    fn triangle_distances_in_closed_form() {
        let t = [v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)];
        // Above the inside, beside a side, beyond a corner.
        let lift = |d: Vec3| [t[0] + d, t[1] + d, t[2] + d];
        assert!((triangle_distance(&t, &lift(v(0.0, 0.0, 0.5))) - 0.5).abs() < 1e-15);
        assert!((point_triangle(v(0.5, -2.0, 0.0), &t) - 2.0).abs() < 1e-15);
        assert!((point_triangle(v(-3.0, -4.0, 0.0), &t) - 5.0).abs() < 1e-15);
        // A triangle piercing another: 0, though no corner is near.
        let pierce = [v(0.2, 0.2, -1.0), v(0.3, 0.2, 1.0), v(0.2, 0.3, 1.0)];
        assert_eq!(triangle_distance(&t, &pierce), 0.0);
        // Crossed sides, a segment's distance.
        assert!(
            (segment_segment(
                v(0.0, 0.0, 0.0),
                v(1.0, 0.0, 0.0),
                v(0.5, -1.0, 0.25),
                v(0.5, 1.0, 0.25)
            ) - 0.25)
                .abs()
                < 1e-15
        );
    }

    #[test]
    fn the_ladder_is_exact() {
        assert_eq!(pow2(-3), 0.125);
        assert_eq!(rung_below(0.3), Some(-2));
        assert_eq!(rung_below(1.0), Some(0));
        assert_eq!(rung_below(0.0), None);
        assert_eq!(rung_below(f64::NAN), None);
    }
}
