//! `file.export` of the model as seen, to DXF and SVG: a projected view
//! (docs/file-formats.md › *Projected views*).
//!
//! What is drawn: every edge of the B-rep of the bodies that are not
//! hidden (`feature.visible`), and the silhouettes of curved faces, each
//! split where it goes behind a face, projected onto the view plane. Seen
//! parts are on the layer `visible` (silhouettes on `silhouette`), parts
//! behind a face are left out or drawn dashed on `hidden`.
//!
//! The method:
//!
//! - Every body is meshed at the export's chordal tolerance (never finer
//!   than [`TOLERANCE_FLOOR`] of the model's size, and within a budget of
//!   [`MAX_TRIANGLES`] for all bodies together), with its edges as
//!   polylines through the very points the mesh has along them
//!   (`craft3d_tess::tessellate_body_and_edges`). A seam (an edge both of
//!   whose sides are the same face) is not drawn: it is no edge one sees.
//!   Smooth (tangent) edges are drawn, as the viewport draws them.
//! - A silhouette is where a face turns from facing the viewer to facing
//!   away: the edges of the mesh, inside one face, between a triangle that
//!   faces the viewer and one that faces away (the contour generator of
//!   the polyhedron; within the tolerance of the surface's own, since the
//!   mesh is). An edge of the mesh with other than two triangles, and a
//!   triangle without a normal, make none. Flat faces have none; a face
//!   seen edge on has none either (its boundary is drawn as its edges).
//! - Visibility is decided per point, exactly for the mesh: a point is
//!   hidden when a triangle of any body covers it in the drawing (strictly
//!   inside, or on a side inside its face where the face goes on without
//!   folding over) and is nearer to the viewer there by more than
//!   [`DEPTH_MARGIN`] tolerances. The triangles are found through a
//!   quadtree over the drawing, each triangle in the cells it overlaps.
//!   Lines are sampled at twice the tolerance (at least a two-thousandth of
//!   the model's size) and split where the verdict changes, the change
//!   found by bisection; a run shorter than the tolerance between runs of
//!   the other kind takes their kind. So a line that goes behind
//!   something for less than the sample spacing may stay drawn as seen
//!   there. A line on the boundary of what hides it (an outline, a back
//!   edge in line with a front one) counts as seen.
//! - Drawn lines are straight pieces: a curve is the polyline of the
//!   mesh, within the tolerance. Pieces in line are merged, a piece that
//!   lies on one drawn before it is left out (visible first, then
//!   silhouettes, then hidden lines), and what follows on joins into one
//!   path. The same document gives the same bytes.
//! - The work is bounded: triangle tests (bisection included) and overlap
//!   tests count against [`MAX_WORK`], and a view that needs more is
//!   refused with the reason.
//!
//! Orthographic views are to scale: one drawing unit is one model unit,
//! the model's origin projected to the drawing's. A perspective view is
//! scaled so that what lies at the depth of the camera's target is true
//! size (nothing else is); an eye inside the model, or nearer to it than
//! [`NEAR_FRACTION`] of its distance to the target, is refused.
//!
//! Not drawn: datums and sketches, the section of the window (a drawing of
//! a cut is a separate matter: the bodies are drawn whole), and the scrub
//! (the document as it is). `view: "current"` is the window's camera
//! ([`Session::set_window_camera`]) for the document it was given for;
//! otherwise, and by default, the iso view.

use crate::commands::{Run, fail};
use crate::params::Params;
use crate::{CommandError, Session};
use craft3d_drawing::{Color, Drawing, Layer, Path as DrawingPath, Point, Segment};
use craft3d_io::DrawingFormat;
use craft3d_math::{Bbox3, Vec3};
use craft3d_render::Projection;
use craft3d_tess::{Mesh, TessError};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

const ID: &str = "file.export";

/// The parameters of a projected view.
pub(crate) const VIEW_KEYS: [&str; 3] = ["view", "projection", "hidden"];

/// Most triangles of all the bodies together.
#[cfg(not(target_arch = "wasm32"))]
const MAX_TRIANGLES: usize = 1_000_000;
#[cfg(target_arch = "wasm32")]
const MAX_TRIANGLES: usize = 250_000;
/// Most triangle entries in the quadtree's cells.
#[cfg(not(target_arch = "wasm32"))]
const MAX_GRID_ENTRIES: usize = 12_000_000;
#[cfg(target_arch = "wasm32")]
const MAX_GRID_ENTRIES: usize = 3_000_000;
/// Most triangle and overlap tests in one export.
#[cfg(not(target_arch = "wasm32"))]
const MAX_WORK: u64 = 1_000_000_000;
#[cfg(target_arch = "wasm32")]
const MAX_WORK: u64 = 200_000_000;
/// Most points sampled along the lines in one export.
const MAX_SAMPLES: usize = 4_000_000;
/// The finest chordal tolerance, as a fraction of the model's diagonal.
pub(crate) const TOLERANCE_FLOOR: f64 = 1e-5;
/// How much nearer than a point, in tolerances, a triangle must be to hide
/// it: the mesh lies within one tolerance of the surface, so two surfaces
/// that touch (a pin in its hole) do not hide each other.
pub(crate) const DEPTH_MARGIN: f64 = 1.25;
/// A perspective eye must be at least this fraction of its distance to the
/// target in front of every point of the model.
pub(crate) const NEAR_FRACTION: f64 = 0.05;
/// Bisection steps that place a change of visibility along a line.
const BISECTIONS: usize = 30;
/// Triangles a quadtree cell holds before it is split.
const LEAF: usize = 24;
/// Deepest level of the quadtree.
const MAX_DEPTH: usize = 14;

/// The named views: the direction to the eye and up, as `view.render`.
const VIEWS: [(&str, Vec3, Vec3); 7] = [
    ("iso", Vec3::new(1.0, -1.0, 0.8), Vec3::Z),
    ("front", Vec3::new(0.0, -1.0, 0.0), Vec3::Z),
    ("back", Vec3::new(0.0, 1.0, 0.0), Vec3::Z),
    ("right", Vec3::new(1.0, 0.0, 0.0), Vec3::Z),
    ("left", Vec3::new(-1.0, 0.0, 0.0), Vec3::Z),
    ("top", Vec3::new(0.0, 0.0, 1.0), Vec3::Y),
    ("bottom", Vec3::new(0.0, 0.0, -1.0), Vec3::Y),
];

/// What a drawn line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Visible,
    Silhouette,
    Hidden,
}

/// What a point of a line is, seen from the eye.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    Yes,
    Behind,
    /// Too near the eye of a perspective view, or behind it: not drawn.
    Out,
}

/// The work done so far, against [`MAX_WORK`].
struct Budget {
    spent: u64,
    limit: u64,
}

impl Budget {
    fn spend(&mut self, work: usize) -> Result<(), CommandError> {
        self.spent = self
            .spent
            .saturating_add(u64::try_from(work).unwrap_or(u64::MAX));
        if self.spent > self.limit {
            return fail(format!(
                "{ID}: too much work to decide what is hidden in this view (more than {} tests); use a coarser `tolerance` or a simpler view",
                self.limit
            ));
        }
        Ok(())
    }
}

/// How model points land in the drawing.
struct View {
    /// Unit vectors: right and up in the drawing, and away from the eye.
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    /// A perspective view's eye and the distance of its target, at which
    /// the drawing is true size.
    perspective: Option<(Vec3, f64)>,
    /// Nearest depth drawn, for a perspective view.
    near: f64,
}

impl View {
    /// The point in the drawing and its depth along the view, or `None`
    /// nearer than the near plane.
    fn project(&self, p: Vec3) -> Option<(Point, f64)> {
        match self.perspective {
            None => Some(([p.dot(self.right), p.dot(self.up)], p.dot(self.forward))),
            Some((eye, scale)) => {
                let q = p - eye;
                let depth = q.dot(self.forward);
                if depth.is_nan() || depth < self.near {
                    return None;
                }
                let k = scale / depth;
                Some(([q.dot(self.right) * k, q.dot(self.up) * k], depth))
            }
        }
    }

    /// The unit vector from `p` towards the eye.
    fn toward_eye(&self, p: Vec3) -> Option<Vec3> {
        match self.perspective {
            None => Some(-self.forward),
            Some((eye, _)) => (eye - p).normalized(),
        }
    }
}

/// A triangle as the drawing sees it.
struct Flat {
    at: [Point; 3],
    /// Depth at the corners (orthographic) or its reciprocal (perspective:
    /// that is what goes linearly across the drawing).
    depth: [f64; 3],
    /// Twice the signed area in the drawing.
    area2: f64,
    /// For the side opposite each corner: true when it lies inside a face
    /// (another triangle of the face is on its other side, turned the same
    /// way). A point on such a side is inside the face: covered.
    inner: [bool; 3],
}

/// A point of a mesh rounded to a grid, so that the two sides of a seam
/// (the same point, made twice) are one.
type Key3 = (i64, i64, i64);

/// A side of a seen triangle: its rounded ends, its face, which way the
/// triangle turns in the drawing, the triangle and the corner opposite.
type SideOf = ((Key3, Key3), u32, bool, usize, usize);

/// A point rounded to a grid of `quantum`.
fn quantize(p: Vec3, quantum: f64) -> Key3 {
    let k = |v: f64| {
        let k = (v / quantum).round();
        if k.is_finite() {
            k.clamp(-1e15, 1e15) as i64
        } else {
            0
        }
    };
    (k(p.x), k(p.y), k(p.z))
}

/// A side of a triangle by its rounded ends, the lower first.
fn side_key(p: Vec3, q: Vec3, quantum: f64) -> (Key3, Key3) {
    let (kp, kq) = (quantize(p, quantum), quantize(q, quantum));
    if kp < kq { (kp, kq) } else { (kq, kp) }
}

fn edge_fn(p: Point, q: Point, s: Point) -> f64 {
    (q[0] - p[0]) * (s[1] - p[1]) - (q[1] - p[1]) * (s[0] - p[0])
}

/// A triangle of a mesh as the drawing sees it, or `None` for one that is
/// seen (nearly) edge on, or reaches nearer than the near plane.
fn flat_of(mesh: &Mesh, t: usize, view: &View, tiny_area: f64) -> Option<([Vec3; 3], Flat)> {
    let corners = mesh.corners(t)?;
    let mut at = [[0.0; 2]; 3];
    let mut depth = [0.0; 3];
    for ((corner, a), z) in corners.iter().zip(at.iter_mut()).zip(depth.iter_mut()) {
        let (point, d) = view.project(*corner)?;
        *a = point;
        *z = if view.perspective.is_some() {
            1.0 / d
        } else {
            d
        };
    }
    let [a, b, c] = at;
    let area2 = edge_fn(a, b, c);
    // Seen (nearly) edge on, the triangle is a sliver in the drawing, whose
    // corner weights at a point are rounding noise: it hides nothing.
    let longest = [sub2(b, a), sub2(c, b), sub2(a, c)]
        .iter()
        .map(|d| dot2(*d, *d))
        .fold(0.0, f64::max);
    let thin = area2.abs() <= 1e-10 * longest;
    (area2.is_finite() && area2.abs() > tiny_area && !thin).then_some((
        corners,
        Flat {
            at,
            depth,
            area2,
            inner: [false; 3],
        },
    ))
}

/// Whether the triangle `at` and the box `lo`..`hi` overlap (the
/// separating axis test: the box's two axes and the triangle's three side
/// normals). Touching counts as overlapping.
fn overlaps(at: &[Point; 3], lo: Point, hi: Point) -> bool {
    for (coordinate, low_end, high_end) in [(0_usize, lo[0], hi[0]), (1, lo[1], hi[1])] {
        let of = |p: &Point| if coordinate == 0 { p[0] } else { p[1] };
        let low = at.iter().map(of).fold(f64::INFINITY, f64::min);
        let high = at.iter().map(of).fold(f64::NEG_INFINITY, f64::max);
        if high < low_end || low > high_end {
            return false;
        }
    }
    let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]];
    for k in 0..3 {
        let (Some(&p), Some(&q), Some(&r)) = (at.get(k), at.get((k + 1) % 3), at.get((k + 2) % 3))
        else {
            continue;
        };
        let normal = [p[1] - q[1], q[0] - p[0]];
        let side = dot2(normal, sub2(r, p));
        let toward: Vec<f64> = corners.iter().map(|c| dot2(normal, sub2(*c, p))).collect();
        // Every corner of the box on the far side of this side's line.
        if side > 0.0 && toward.iter().all(|v| *v < 0.0) {
            return false;
        }
        if side < 0.0 && toward.iter().all(|v| *v > 0.0) {
            return false;
        }
    }
    true
}

/// A cell of the quadtree: its box, and its triangles or its children.
struct Node {
    lo: Point,
    hi: Point,
    /// The first of its four children (x low/high, then y low/high), or
    /// none for a leaf.
    children: Option<usize>,
    triangles: Vec<u32>,
}

/// The triangles of every body, in a quadtree over the drawing.
struct Occluders {
    flats: Vec<Flat>,
    nodes: Vec<Node>,
    perspective: bool,
    /// How much nearer a triangle must be to hide a point.
    depth_margin: f64,
}

impl Occluders {
    fn new(
        meshes: &[Mesh],
        view: &View,
        scale: f64,
        depth_margin: f64,
        quantum: f64,
        budget: &mut Budget,
    ) -> Result<Self, CommandError> {
        let mut flats = Vec::new();
        let tiny_area = (scale * 1e-9).powi(2);
        for mesh in meshes {
            let seen: Vec<([Vec3; 3], Flat, u32)> = (0..mesh.triangles.len())
                .filter_map(|t| {
                    let (corners, flat) = flat_of(mesh, t, view, tiny_area)?;
                    Some((corners, flat, *mesh.face_ids.get(t)?))
                })
                .collect();
            // The sides of the triangles by their rounded ends, sorted: a
            // side inside a face is one of exactly two, of one face, turned
            // the same way.
            let mut sides: Vec<SideOf> = Vec::new();
            for (n, ([a, b, c], flat, face)) in seen.iter().enumerate() {
                // The side opposite corner k.
                for (k, (p, q)) in [(*b, *c), (*c, *a), (*a, *b)].into_iter().enumerate() {
                    sides.push((side_key(p, q, quantum), *face, flat.area2 > 0.0, n, k));
                }
            }
            sides.sort_by_key(|x| (x.0, x.3, x.4));
            let first = flats.len();
            flats.extend(seen.into_iter().map(|(_, flat, _)| flat));
            let mut i = 0;
            while let Some(side) = sides.get(i) {
                let mut j = i + 1;
                while sides.get(j).is_some_and(|other| other.0 == side.0) {
                    j += 1;
                }
                if let (2, Some(a), Some(b)) = (j - i, sides.get(i), sides.get(i + 1))
                    && a.1 == b.1
                    && a.2 == b.2
                {
                    for (_, _, _, n, k) in [a, b] {
                        if let Some(slot) = flats
                            .get_mut(first + *n)
                            .and_then(|flat| flat.inner.get_mut(*k))
                        {
                            *slot = true;
                        }
                    }
                }
                i = j;
            }
        }
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for flat in &flats {
            for p in flat.at {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
        }
        if !(lo[0].is_finite() && lo[1].is_finite()) {
            (lo, hi) = ([0.0, 0.0], [0.0, 0.0]);
        }
        // A little room around, so that nothing lies on the root's border
        // only by rounding.
        let pad = scale * 1e-9;
        let root = Node {
            lo: [lo[0] - pad, lo[1] - pad],
            hi: [hi[0] + pad, hi[1] + pad],
            children: None,
            triangles: (0..flats.len())
                .filter_map(|n| u32::try_from(n).ok())
                .collect(),
        };
        let mut entries = root.triangles.len();
        let mut nodes = vec![root];
        let mut stack = vec![(0_usize, 0_usize)];
        while let Some((index, depth)) = stack.pop() {
            let Some(node) = nodes.get(index) else {
                continue;
            };
            if node.triangles.len() <= LEAF || depth >= MAX_DEPTH {
                continue;
            }
            let (lo, hi) = (node.lo, node.hi);
            let mid = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])];
            let boxes = [
                (lo, mid),
                ([mid[0], lo[1]], [hi[0], mid[1]]),
                ([lo[0], mid[1]], [mid[0], hi[1]]),
                (mid, hi),
            ];
            let parent: Vec<u32> = node.triangles.clone();
            budget.spend(parent.len().saturating_mul(4))?;
            let first = nodes.len();
            for (blo, bhi) in boxes {
                let inside: Vec<u32> = parent
                    .iter()
                    .copied()
                    .filter(|n| {
                        flats
                            .get(*n as usize)
                            .is_some_and(|flat| overlaps(&flat.at, blo, bhi))
                    })
                    .collect();
                entries = entries.saturating_add(inside.len());
                if entries > MAX_GRID_ENTRIES {
                    return fail(format!(
                        "{ID}: the view has too many overlapping triangles to decide what is hidden; use a coarser `tolerance` or a simpler view"
                    ));
                }
                nodes.push(Node {
                    lo: blo,
                    hi: bhi,
                    children: None,
                    triangles: inside,
                });
            }
            if let Some(node) = nodes.get_mut(index) {
                node.children = Some(first);
                entries = entries.saturating_sub(node.triangles.len());
                node.triangles = Vec::new();
            }
            for k in 0..4 {
                stack.push((first + k, depth + 1));
            }
        }
        Ok(Self {
            flats,
            nodes,
            perspective: view.perspective.is_some(),
            depth_margin,
        })
    }

    /// The leaf whose cell holds `s`.
    fn leaf(&self, s: Point) -> Option<&Node> {
        let mut node = self.nodes.first()?;
        let inside =
            |n: &Node| s[0] >= n.lo[0] && s[0] <= n.hi[0] && s[1] >= n.lo[1] && s[1] <= n.hi[1];
        if !inside(node) {
            return None;
        }
        for _ in 0..=MAX_DEPTH {
            let Some(first) = node.children else {
                return Some(node);
            };
            let mid = [
                0.5 * (node.lo[0] + node.hi[0]),
                0.5 * (node.lo[1] + node.hi[1]),
            ];
            let k = usize::from(s[0] >= mid[0]) + 2 * usize::from(s[1] >= mid[1]);
            node = self.nodes.get(first + k)?;
        }
        Some(node)
    }

    /// True when a triangle covers `s` nearer than `depth` by more than the
    /// margin.
    fn hides(&self, s: Point, depth: f64, budget: &mut Budget) -> Result<bool, CommandError> {
        let Some(leaf) = self.leaf(s) else {
            return Ok(false);
        };
        budget.spend(leaf.triangles.len().max(1))?;
        const INSIDE: f64 = 1e-9;
        for &n in &leaf.triangles {
            let Some(flat) = self.flats.get(n as usize) else {
                continue;
            };
            let [a, b, c] = flat.at;
            let wa = edge_fn(b, c, s) / flat.area2;
            let wb = edge_fn(c, a, s) / flat.area2;
            let wc = edge_fn(a, b, s) / flat.area2;
            // Strictly inside, or on a side inside the face.
            let covers = |w: f64, inner: bool| w > INSIDE || (inner && w > -INSIDE);
            let [ia, ib, ic] = flat.inner;
            if !(covers(wa, ia) && covers(wb, ib) && covers(wc, ic)) {
                continue;
            }
            let [da, db, dc] = flat.depth;
            let at = wa * da + wb * db + wc * dc;
            let there = if self.perspective { 1.0 / at } else { at };
            if there < depth - self.depth_margin {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Where a point is, seen from the eye.
fn seen(
    view: &View,
    occluders: &Occluders,
    p: Vec3,
    budget: &mut Budget,
) -> Result<Seen, CommandError> {
    Ok(match view.project(p) {
        None => Seen::Out,
        Some((s, depth)) if occluders.hides(s, depth, budget)? => Seen::Behind,
        Some(_) => Seen::Yes,
    })
}

/// A piece of a line of one kind, in model space.
#[derive(Clone, Copy, Debug)]
struct Piece3 {
    from: Vec3,
    to: Vec3,
    seen: Seen,
}

/// What the lines are sampled at, and the limits.
struct Sampling<'a> {
    view: &'a View,
    occluders: &'a Occluders,
    spacing: f64,
    samples: usize,
    budget: Budget,
}

impl Sampling<'_> {
    /// Splits the line from `a` to `b` where its visibility changes.
    fn classify(&mut self, a: Vec3, b: Vec3) -> Result<Vec<Piece3>, CommandError> {
        let length = a.distance(b);
        if !(length.is_finite() && length > 0.0) {
            return Ok(Vec::new());
        }
        let n = (length / self.spacing).ceil();
        let n = if n.is_finite() && n >= 1.0 {
            n.min(MAX_SAMPLES as f64) as usize
        } else {
            1
        };
        self.samples = self.samples.saturating_add(n);
        if self.samples > MAX_SAMPLES {
            return fail(format!(
                "{ID}: the view has too many lines to decide what is hidden ({MAX_SAMPLES} points); use a coarser `tolerance` or a simpler view"
            ));
        }
        let (view, occluders) = (self.view, self.occluders);
        let budget = &mut self.budget;
        let at = |t: f64| a.lerp(b, t);
        let centre = |k: usize| (k as f64 + 0.5) / n as f64;
        let mut pieces = Vec::new();
        let mut start = 0.0;
        let mut state = seen(view, occluders, at(centre(0)), budget)?;
        for k in 1..n {
            let next = seen(view, occluders, at(centre(k)), budget)?;
            if next == state {
                continue;
            }
            // Between the two centres: where it changes.
            let (mut lo, mut hi) = (centre(k - 1), centre(k));
            for _ in 0..BISECTIONS {
                let mid = 0.5 * (lo + hi);
                if seen(view, occluders, at(mid), budget)? == state {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let cut = 0.5 * (lo + hi);
            pieces.push(Piece3 {
                from: at(start),
                to: at(cut),
                seen: state,
            });
            start = cut;
            state = next;
        }
        pieces.push(Piece3 {
            from: at(start),
            to: b,
            seen: state,
        });
        Ok(pieces)
    }
}

/// A run shorter than `shortest` between two runs of one other kind takes
/// theirs: a verdict that flickers at the tolerance's scale is noise.
fn smooth(pieces: &mut [Piece3], shortest: f64) {
    // Runs of one verdict: [first, last] piece.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (k, piece) in pieces.iter().enumerate() {
        match runs.last_mut() {
            Some((first, last)) if pieces.get(*first).is_some_and(|p| p.seen == piece.seen) => {
                *last = k;
            }
            _ => runs.push((k, k)),
        }
    }
    for r in 0..runs.len() {
        let Some(&(first, last)) = runs.get(r) else {
            continue;
        };
        // The kind around it: of both neighbours when they agree; at an
        // end of the line, of its one neighbour (a stub where a line
        // leaves a face it ends on).
        let neighbour = |k: Option<usize>| {
            k.and_then(|k| runs.get(k))
                .and_then(|&(at, _)| pieces.get(at))
                .map(|p| p.seen)
        };
        let before = neighbour(r.checked_sub(1));
        let after = neighbour(r.checked_add(1));
        let around = match (before, after) {
            (Some(b), Some(a)) if a == b => a,
            (Some(b), None) => b,
            (None, Some(a)) => a,
            _ => continue,
        };
        let length: f64 = pieces
            .get(first..=last)
            .map_or(0.0, |run| run.iter().map(|p| p.from.distance(p.to)).sum());
        if length < shortest {
            for piece in pieces.get_mut(first..=last).into_iter().flatten() {
                piece.seen = around;
            }
        }
    }
}

/// A straight piece of the drawing.
#[derive(Clone, Copy, Debug)]
struct Piece {
    kind: Kind,
    a: Point,
    b: Point,
    /// The line it came from, so pieces join only along their line.
    line: usize,
}

fn sub2(p: Point, q: Point) -> Point {
    [p[0] - q[0], p[1] - q[1]]
}

fn cross2(p: Point, q: Point) -> f64 {
    p[0] * q[1] - p[1] * q[0]
}

fn dot2(p: Point, q: Point) -> f64 {
    p[0] * q[0] + p[1] * q[1]
}

fn dist2(p: Point, q: Point) -> f64 {
    let d = sub2(p, q);
    dot2(d, d).sqrt()
}

/// Joins pieces in line that follow on from each other.
fn merge_in_line(pieces: Vec<Piece>, close: f64) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        if let Some(last) = out.last_mut() {
            let d1 = sub2(last.b, last.a);
            let d2 = sub2(piece.b, piece.a);
            let same_way = dot2(d1, d2) > 0.0;
            let in_line = cross2(d1, d2).abs() <= 1e-9 * dot2(d1, d1).sqrt() * dot2(d2, d2).sqrt();
            if last.kind == piece.kind
                && last.line == piece.line
                && dist2(last.b, piece.a) <= close
                && same_way
                && in_line
            {
                last.b = piece.b;
                continue;
            }
        }
        out.push(piece);
    }
    out
}

/// Cells of a grid of `cell` that the segment from `a` to `b` crosses, in
/// order (a walk from cell to cell along it), at most `cap` of them.
fn cells_crossed(a: Point, b: Point, cell: f64, cap: usize) -> Vec<(i64, i64)> {
    let index = |v: f64| -> i64 {
        let k = (v / cell).floor();
        if k.is_finite() {
            k.clamp(-1e12, 1e12) as i64
        } else {
            0
        }
    };
    let (mut i, mut j) = (index(a[0]), index(a[1]));
    let (end_i, end_j) = (index(b[0]), index(b[1]));
    let d = sub2(b, a);
    let step = |v: f64| -> i64 {
        if v > 0.0 {
            1
        } else if v < 0.0 {
            -1
        } else {
            0
        }
    };
    let (si, sj) = (step(d[0]), step(d[1]));
    // Along the segment (0..1): where the next cell border is crossed in x
    // and in y, and how far apart the borders are.
    let border = |k: i64, s: i64, from: f64, delta: f64| -> (f64, f64) {
        if s == 0 || delta == 0.0 {
            return (f64::INFINITY, f64::INFINITY);
        }
        let next = if s > 0 { (k + 1) as f64 } else { k as f64 } * cell;
        ((next - from) / delta, cell / delta.abs())
    };
    let (mut tx, dx) = border(i, si, a[0], d[0]);
    let (mut ty, dy) = border(j, sj, a[1], d[1]);
    let mut out = vec![(i, j)];
    while (i, j) != (end_i, end_j) && out.len() < cap {
        if tx < ty {
            i += si;
            tx += dx;
        } else {
            j += sj;
            ty += dy;
        }
        if !(tx.is_finite() || ty.is_finite()) {
            break;
        }
        out.push((i, j));
        if tx > 1.0 && ty > 1.0 {
            break;
        }
    }
    out
}

/// Leaves out what lies on a piece drawn before it: the pieces are taken
/// in order (visible, silhouettes, hidden), and of each what another one
/// already covers is cut away. Pieces are found through the cells they
/// cross.
fn without_overlaps(
    pieces: &[Piece],
    scale: f64,
    close: f64,
    budget: &mut Budget,
) -> Result<Vec<Piece>, CommandError> {
    const CELLS: f64 = 64.0;
    let cell = (scale / CELLS).max(f64::MIN_POSITIVE);
    // A piece of the drawing crosses at most this many cells.
    let cap = (CELLS as usize).saturating_mul(4).saturating_add(8);
    let mut grid: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    let mut kept: Vec<Piece> = Vec::new();
    let mut order: Vec<&Piece> = pieces.iter().collect();
    order.sort_by_key(|p| p.kind);
    for piece in order {
        let d = sub2(piece.b, piece.a);
        let len2 = dot2(d, d);
        if len2.is_nan() || len2 <= close * close {
            continue;
        }
        let len = len2.sqrt();
        // What is left of it, as parameters along it.
        let mut left: Vec<(f64, f64)> = vec![(0.0, 1.0)];
        let mut candidates: Vec<usize> = Vec::new();
        for key in cells_crossed(piece.a, piece.b, cell, cap) {
            if let Some(list) = grid.get(&key) {
                candidates.extend(list.iter().copied());
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        budget.spend(candidates.len())?;
        for id in candidates {
            let Some(other) = kept.get(id) else {
                continue;
            };
            let e = sub2(other.b, other.a);
            let elen = dot2(e, e).sqrt();
            if elen.is_nan() || elen <= 0.0 {
                continue;
            }
            let off_a = cross2(e, sub2(piece.a, other.a)).abs() / elen;
            let off_b = cross2(e, sub2(piece.b, other.a)).abs() / elen;
            if off_a > close || off_b > close {
                continue;
            }
            let ta = dot2(sub2(other.a, piece.a), d) / len2;
            let tb = dot2(sub2(other.b, piece.a), d) / len2;
            let (lo, hi) = (ta.min(tb), ta.max(tb));
            let mut next = Vec::with_capacity(left.len() + 1);
            for (s, t) in left {
                if hi <= s || lo >= t {
                    next.push((s, t));
                    continue;
                }
                if lo > s {
                    next.push((s, lo));
                }
                if hi < t {
                    next.push((hi, t));
                }
            }
            left = next;
            if left.is_empty() {
                break;
            }
        }
        for (s, t) in left {
            if (t - s) * len <= close {
                continue;
            }
            let at = |u: f64| [piece.a[0] + d[0] * u, piece.a[1] + d[1] * u];
            let part = Piece {
                kind: piece.kind,
                a: at(s),
                b: at(t),
                line: piece.line,
            };
            let index = kept.len();
            for key in cells_crossed(part.a, part.b, cell, cap) {
                grid.entry(key).or_default().push(index);
            }
            kept.push(part);
        }
    }
    Ok(kept)
}

/// The pieces of one kind as paths: a piece that starts where the one
/// before it ended, on the same line, goes on its path.
fn paths_of(pieces: &[Piece], kind: Kind, close: f64) -> Vec<DrawingPath> {
    let mut paths: Vec<DrawingPath> = Vec::new();
    let mut current: Option<(Point, Vec<Segment>, usize, Point)> = None;
    let finish = |current: Option<(Point, Vec<Segment>, usize, Point)>,
                  paths: &mut Vec<DrawingPath>| {
        if let Some((start, mut segments, _, end)) = current {
            let closed = segments.len() > 2 && dist2(start, end) <= close;
            if closed {
                // The last piece ends at the start itself.
                if let Some(Segment::Line { to }) = segments.last_mut() {
                    *to = start;
                }
            }
            paths.push(DrawingPath::Chain {
                start,
                segments,
                closed,
            });
        }
    };
    for piece in pieces.iter().filter(|p| p.kind == kind) {
        match &mut current {
            Some((_, segments, line, end))
                if *line == piece.line && dist2(*end, piece.a) <= close =>
            {
                segments.push(Segment::Line { to: piece.b });
                *end = piece.b;
            }
            _ => {
                finish(current.take(), &mut paths);
                current = Some((
                    piece.a,
                    vec![Segment::Line { to: piece.b }],
                    piece.line,
                    piece.b,
                ));
            }
        }
    }
    finish(current, &mut paths);
    paths
}

/// Which way each triangle of `mesh` faces: 1 towards the eye, -1 away, 0
/// for a triangle without a normal (no area). A triangle seen edge on
/// counts as facing the eye: a strip of a cylinder seen exactly edge on is
/// then on the front side of the contour, which lies along it either way
/// (both its long sides fall on one line).
fn facing(mesh: &Mesh, view: &View) -> Vec<i8> {
    (0..mesh.triangles.len())
        .map(|t| {
            let (Some(c), Some(tri)) = (mesh.corners(t), mesh.triangles.get(t)) else {
                return 0;
            };
            let [a, b, cc] = c;
            let mut n = (b - a).cross(cc - a);
            // Outward as the mesh's normals say, where it has them.
            let summed = tri
                .iter()
                .map(|i| mesh.normals.get(*i as usize).copied().unwrap_or(Vec3::ZERO))
                .fold(Vec3::ZERO, |s, v| s + v);
            if n.dot(summed) < 0.0 {
                n = -n;
            }
            let centroid = (a + b + cc) * (1.0 / 3.0);
            match (n.normalized(), view.toward_eye(centroid)) {
                (Some(n), Some(v)) => {
                    if n.dot(v) >= -1e-9 {
                        1
                    } else {
                        -1
                    }
                }
                _ => 0,
            }
        })
        .collect()
}

/// The silhouettes of a mesh: its edges, inside one face, between a
/// triangle facing the eye and one facing away, joined into polylines. An
/// edge of the mesh with other than two triangles makes none.
fn silhouettes(mesh: &Mesh, view: &View, quantum: f64) -> Vec<Vec<Vec3>> {
    let key = |p: Vec3| quantize(p, quantum);
    let facing = facing(mesh, view);
    // Every side of every triangle, by its rounded ends; sorted, the two
    // triangles of a side of the mesh are next to each other.
    let mut sides: Vec<((Key3, Key3), usize, Vec3, Vec3)> = Vec::new();
    for t in 0..mesh.triangles.len() {
        let Some([a, b, c]) = mesh.corners(t) else {
            continue;
        };
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let k = side_key(p, q, quantum);
            if k.0 != k.1 {
                sides.push((k, t, p, q));
            }
        }
    }
    sides.sort_by_key(|x| (x.0, x.1));
    let mut segments: Vec<(Vec3, Vec3)> = Vec::new();
    let mut i = 0;
    while let Some(side) = sides.get(i) {
        let mut j = i + 1;
        while sides.get(j).is_some_and(|other| other.0 == side.0) {
            j += 1;
        }
        if let (2, Some(&(_, t1, p, q)), Some(&(_, t2, ..))) =
            (j - i, sides.get(i), sides.get(i + 1))
        {
            let same_face = mesh.face_ids.get(t1) == mesh.face_ids.get(t2);
            let (f1, f2) = (
                facing.get(t1).copied().unwrap_or(0),
                facing.get(t2).copied().unwrap_or(0),
            );
            if same_face && f1 * f2 == -1 {
                segments.push((p, q));
            }
        }
        i = j;
    }
    chain(&segments, key)
}

/// Joins segments that share ends into polylines, in a fixed order.
fn chain<K: Ord + Copy>(segments: &[(Vec3, Vec3)], key: impl Fn(Vec3) -> K) -> Vec<Vec<Vec3>> {
    let mut at: BTreeMap<K, Vec<usize>> = BTreeMap::new();
    for (i, (p, q)) in segments.iter().enumerate() {
        at.entry(key(*p)).or_default().push(i);
        at.entry(key(*q)).or_default().push(i);
    }
    let mut used = vec![false; segments.len()];
    let mut lines = Vec::new();
    for start in 0..segments.len() {
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        if let Some(u) = used.get_mut(start) {
            *u = true;
        }
        let Some(&(p, q)) = segments.get(start) else {
            continue;
        };
        let mut line = std::collections::VecDeque::from(vec![p, q]);
        // Forward from the end, then backward from the start.
        for forward in [true, false] {
            loop {
                let end = if forward { line.back() } else { line.front() };
                let Some(&end) = end else { break };
                let next = at.get(&key(end)).and_then(|list| {
                    list.iter()
                        .copied()
                        .find(|i| !used.get(*i).copied().unwrap_or(true))
                });
                let Some(next) = next else { break };
                if let Some(u) = used.get_mut(next) {
                    *u = true;
                }
                let Some(&(a, b)) = segments.get(next) else {
                    break;
                };
                let other = if key(a) == key(end) { b } else { a };
                if forward {
                    line.push_back(other);
                } else {
                    line.push_front(other);
                }
            }
        }
        lines.push(line.into_iter().collect());
    }
    lines
}

/// The camera of the view asked for: its name, the direction to the eye,
/// up, and a perspective eye and target of the window's.
type Camera = (&'static str, Vec3, Vec3, Option<(Vec3, Vec3)>);

/// The camera `view` names: `current` is the window's for the active
/// document, when the window gave one, else iso.
fn camera_of(
    session: &Session,
    view: &str,
) -> Result<(Camera, Option<&'static str>), CommandError> {
    if view == "current" {
        let active = session.document_status().map(|status| status.id);
        return match session.window_camera {
            Some((document, camera)) if Some(document) == active => {
                let Some(dir) = (camera.eye - camera.target).normalized() else {
                    return fail(format!("{ID}: the window's camera looks nowhere"));
                };
                let eye = match camera.projection {
                    Projection::Perspective { .. } => Some((camera.eye, camera.target)),
                    Projection::Orthographic { .. } => None,
                };
                Ok((("current", dir, camera.up, eye), None))
            }
            _ => Ok((
                ("iso", VIEWS[0].1, VIEWS[0].2, None),
                Some("no window shows this document: the iso view"),
            )),
        };
    }
    let Some(&(name, dir, up)) = VIEWS.iter().find(|(name, ..)| *name == view) else {
        return fail(format!(
            "{ID}: unknown view `{view}`; use current (the window's), iso, front, back, left, right, top or bottom"
        ));
    };
    Ok(((name, dir, up, None), None))
}

/// Writes the drawing of the model as seen.
pub(crate) fn export(
    session: &mut Session,
    p: &Params,
    path: &str,
    resolved: &Path,
    format: DrawingFormat,
) -> Run {
    for key in ["feature", "construction", "xAxis", "origin", "gap", "width"] {
        if p.has(key) {
            return fail(format!(
                "{ID}: `{key}` is about the outline of a sketch or a face; a drawing of the model as seen (no `sketch`, `face` or `faces`) takes `view`, `projection` and `hidden`"
            ));
        }
    }
    let view_name = p.str("view")?.unwrap_or("iso");
    let perspective = match p.str("projection")?.unwrap_or("orthographic") {
        "orthographic" => false,
        "perspective" => true,
        other => {
            return fail(format!(
                "{ID}: unknown projection `{other}`; use orthographic (to scale) or perspective"
            ));
        }
    };
    let dashed = match p.str("hidden")?.unwrap_or("omit") {
        "omit" => false,
        "dashed" => true,
        other => {
            return fail(format!(
                "{ID}: unknown `hidden` `{other}`; use omit (hidden lines left out) or dashed"
            ));
        }
    };
    let ((shown_view, toward_eye, up, window_eye), note) = camera_of(session, view_name)?;
    let Some(dir) = toward_eye.normalized() else {
        return fail(format!("{ID}: the view looks nowhere"));
    };
    let Some(right) = up.cross(dir).normalized() else {
        return fail(format!("{ID}: the view's up lies along it"));
    };
    let view_up = dir.cross(right);

    // The bodies, meshed with their edges, within one budget of triangles
    // and never finer than the floor.
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = std::sync::Arc::clone(open.history.current());
    let shown: Vec<_> = regen
        .bodies()
        .filter(|(id, _)| !doc.is_hidden(*id))
        .collect();
    if shown.is_empty() {
        return fail(format!(
            "{ID}: there are no bodies to draw (hidden ones are left out); add a feature, or show one with feature.visible"
        ));
    }
    let mut brep = Bbox3::EMPTY;
    for (_, body) in &shown {
        if let Ok(bbox) = body.bbox()
            && let Some((lo, hi)) = bbox.corners()
        {
            brep.add(lo);
            brep.add(hi);
        }
    }
    let size = brep.diagonal();
    if !(size.is_finite() && size > 0.0) {
        return fail(format!("{ID}: the bodies have no size to draw"));
    }
    let mut tess = crate::commands::tolerance_param(p, ID, &regen)?;
    tess.chordal_tolerance = tess.chordal_tolerance.max(size * TOLERANCE_FLOOR);
    let tolerance = tess.chordal_tolerance;
    let mut meshes = Vec::new();
    let mut lines: Vec<Vec<Vec3>> = Vec::new();
    let mut triangles = 0_usize;
    for (id, body) in &shown {
        let name = doc
            .feature(*id)
            .map_or_else(|| id.to_string(), |f| f.name.clone());
        let params = craft3d_tess::TessParams {
            max_triangles: Some(MAX_TRIANGLES.saturating_sub(triangles)),
            ..tess
        };
        let too_many = || {
            CommandError::new(format!(
                "{ID}: the bodies have more than {MAX_TRIANGLES} triangles at the tolerance {tolerance}; use a coarser `tolerance`"
            ))
        };
        let made = craft3d_tess::tessellate_body_and_edges(body, &params).map_err(|e| match e {
            TessError::OverBudget { .. } => too_many(),
            e => CommandError::new(format!("{ID}: {name}: {e}")),
        })?;
        let mesh = made.mesh.map_err(|e| match e {
            TessError::OverBudget { .. } => too_many(),
            e => CommandError::new(format!("{ID}: {name}: {e}")),
        })?;
        triangles = triangles.saturating_add(mesh.triangles.len());
        if triangles > MAX_TRIANGLES {
            return Err(too_many());
        }
        // A seam (both sides on one face) is no edge one sees.
        let uses = craft3d_topo::edge_faces(body);
        for (index, line) in made.edges.into_iter().enumerate() {
            let seam = matches!(uses.get(index).map(Vec::as_slice), Some([a, b]) if a == b);
            if !seam {
                lines.push(line);
            }
        }
        meshes.push(mesh);
    }
    let bodies = meshes.len();
    let model = Bbox3::from_points(meshes.iter().flat_map(|m| m.positions.iter().copied()));
    let diagonal = model.diagonal();
    if !(diagonal.is_finite() && diagonal > 0.0) {
        return fail(format!("{ID}: the bodies have no size to draw"));
    }
    let perspective_eye = if perspective {
        let (eye, target) = match window_eye {
            Some(pair) => pair,
            None => {
                let camera = craft3d_render::Camera::framing(&model, dir, up, 30.0, 1.0)
                    .ok_or_else(|| {
                        CommandError::new(format!("{ID}: no perspective camera frames the bodies"))
                    })?;
                (camera.eye, camera.target)
            }
        };
        Some((eye, eye.distance(target)))
    } else {
        None
    };
    let near = match perspective_eye {
        Some((eye, distance)) => {
            if !(distance.is_finite() && distance > 0.0) {
                return fail(format!("{ID}: the camera's eye is on its target"));
            }
            // Every point of the model well in front of the eye: one near
            // it would blow the drawing up.
            let nearest = meshes
                .iter()
                .flat_map(|m| m.positions.iter())
                .map(|p| (*p - eye).dot(-dir))
                .fold(f64::INFINITY, f64::min);
            let near = NEAR_FRACTION * distance;
            if nearest.is_nan() || nearest < near {
                return fail(format!(
                    "{ID}: the eye of the perspective view is inside the model or too near it; use an orthographic projection, a named view, or move the camera back"
                ));
            }
            near
        }
        None => 0.0,
    };
    let view = View {
        right,
        up: view_up,
        forward: -dir,
        perspective: perspective_eye,
        near,
    };
    // The size of the drawing, for what counts as the same place.
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for mesh in &meshes {
        for p in &mesh.positions {
            if let Some((s, _)) = view.project(*p) {
                lo = [lo[0].min(s[0]), lo[1].min(s[1])];
                hi = [hi[0].max(s[0]), hi[1].max(s[1])];
            }
        }
    }
    let scale = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    if !(scale.is_finite() && scale > 0.0) {
        return fail(format!("{ID}: the bodies have no size in this view"));
    }
    let mut budget = Budget {
        spent: 0,
        limit: MAX_WORK,
    };
    let occluders = Occluders::new(
        &meshes,
        &view,
        scale,
        DEPTH_MARGIN * tolerance + 1e-9 * diagonal,
        diagonal * 1e-7,
        &mut budget,
    )?;
    // The silhouettes come after the edges of every body.
    let edges_count = lines.len();
    for mesh in &meshes {
        lines.extend(silhouettes(mesh, &view, diagonal * 1e-7));
    }

    let close = scale * 1e-7;
    let mut sampling = Sampling {
        view: &view,
        occluders: &occluders,
        spacing: (2.0 * tolerance).max(diagonal / 2000.0),
        samples: 0,
        budget,
    };
    let mut pieces: Vec<Piece> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let base = if index < edges_count {
            Kind::Visible
        } else {
            Kind::Silhouette
        };
        let mut parts: Vec<Piece3> = Vec::new();
        for pair in line.windows(2) {
            let [a, b] = pair else { continue };
            parts.extend(sampling.classify(*a, *b)?);
        }
        // Within the depth margin of a surface the verdict is a guess.
        smooth(&mut parts, (1.0 + DEPTH_MARGIN) * tolerance);
        let mut flat: Vec<Piece> = Vec::new();
        for part in parts {
            let kind = match part.seen {
                Seen::Out => continue,
                Seen::Yes => base,
                Seen::Behind => Kind::Hidden,
            };
            let (Some((a, _)), Some((b, _))) = (view.project(part.from), view.project(part.to))
            else {
                continue;
            };
            if dist2(a, b) <= close {
                continue;
            }
            flat.push(Piece {
                kind,
                a,
                b,
                line: index,
            });
        }
        pieces.extend(merge_in_line(flat, close));
    }
    let mut budget = sampling.budget;
    let kept = without_overlaps(&pieces, scale, close, &mut budget)?;
    let count = |kind: Kind| kept.iter().filter(|p| p.kind == kind).count();
    let (visible, silhouette, hidden) = (
        count(Kind::Visible),
        count(Kind::Silhouette),
        count(Kind::Hidden),
    );
    let dash = (scale / 60.0).max(f64::MIN_POSITIVE);
    let mut layers = Vec::new();
    for (name, kind, dash) in [
        ("visible", Kind::Visible, None),
        ("silhouette", Kind::Silhouette, None),
        ("hidden", Kind::Hidden, Some(dash)),
    ] {
        if kind == Kind::Hidden && !dashed {
            continue;
        }
        let paths = paths_of(&kept, kind, close);
        if !paths.is_empty() {
            layers.push(Layer {
                name: name.to_string(),
                color: Color::Black,
                dash,
                paths,
            });
        }
    }
    if layers.is_empty() {
        return fail(format!("{ID}: nothing of the bodies is seen in this view"));
    }
    let unit = doc.units.length;
    let drawing = Drawing {
        unit: crate::drawing_cmds::drawing_unit(unit),
        layers,
    };
    let (bytes, _) = craft3d_io::encode_drawing(&drawing, format, tolerance)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let size = bytes.len();
    session.write_resolved(resolved, bytes)?;
    let counts = drawing.counts();
    let vec3 = |v: Vec3| json!([v.x, v.y, v.z]);
    Ok(json!({
        "path": path,
        "format": format.name(),
        "bytes": size,
        "unit": unit.symbol(),
        "view": shown_view,
        "note": note,
        // The camera used: towards the eye, up, and a perspective's eye.
        "camera": {
            "towardEye": vec3(dir),
            "up": vec3(view_up),
            "eye": perspective_eye.map(|(eye, _)| vec3(eye)),
        },
        "projection": if perspective { "perspective" } else { "orthographic" },
        "scale": if perspective {
            "perspective: true size only at the depth of the camera's target"
        } else {
            "1:1 (one drawing unit is one model unit)"
        },
        "hidden": if dashed { "dashed" } else { "omit" },
        "bodies": bodies,
        "tolerance": tolerance,
        // Straight pieces of each kind, after joining those in line.
        "visible": visible,
        "silhouette": silhouette,
        "hiddenLines": hidden,
        "paths": counts.paths,
        "lines": counts.lines,
        "length": drawing.length(),
        "bbox": drawing.bbox(),
        "layers": drawing.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>(),
        "version": (format == DrawingFormat::Dxf).then_some(craft3d_drawing::DXF_VERSION),
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn front() -> View {
        View {
            right: Vec3::X,
            up: Vec3::Z,
            forward: Vec3::Y,
            perspective: None,
            near: 0.0,
        }
    }

    /// Triangles from corner lists, all of face 0 unless given.
    fn mesh(triangles: &[[Vec3; 3]], faces: &[u32]) -> Mesh {
        let mut m = Mesh::default();
        for (t, corners) in triangles.iter().enumerate() {
            let base = m.positions.len() as u32;
            m.positions.extend(corners.iter().copied());
            m.normals.extend([Vec3::ZERO; 3]);
            m.triangles.push([base, base + 1, base + 2]);
            m.face_ids.push(faces.get(t).copied().unwrap_or(0));
        }
        m
    }

    #[test]
    fn a_side_of_three_triangles_or_a_flat_triangle_makes_no_silhouette() {
        let v = |x: f64, y: f64, z: f64| Vec3::new(x, y, z);
        // A fold seen from the front: one triangle facing the eye, one
        // away, along the side (0,0,0)-(0,0,1).
        let folded = [
            [v(0.0, 0.0, 0.0), v(0.0, 0.0, 1.0), v(1.0, -1.0, 0.5)],
            [v(0.0, 0.0, 1.0), v(0.0, 0.0, 0.0), v(1.0, 1.0, 0.5)],
        ];
        let lines = silhouettes(&mesh(&folded, &[]), &front(), 1e-9);
        assert_eq!(lines.len(), 1);
        // A third triangle on that side: not a manifold side, no contour.
        let mut three = folded.to_vec();
        three.push([v(0.0, 0.0, 0.0), v(0.0, 0.0, 1.0), v(-1.0, 0.0, 0.5)]);
        assert!(silhouettes(&mesh(&three, &[]), &front(), 1e-9).is_empty());
        // A triangle without area beside a good one: none either.
        let flat = [
            [v(0.0, 0.0, 0.0), v(0.0, 0.0, 1.0), v(1.0, -1.0, 0.5)],
            [v(0.0, 0.0, 1.0), v(0.0, 0.0, 0.0), v(0.0, 0.0, 0.5)],
        ];
        assert!(silhouettes(&mesh(&flat, &[]), &front(), 1e-9).is_empty());
        // Two faces: their meeting is an edge of the B-rep, no contour.
        assert!(silhouettes(&mesh(&folded, &[0, 1]), &front(), 1e-9).is_empty());
    }

    #[test]
    fn the_triangle_box_test_is_the_overlap() {
        let t = [[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]];
        assert!(overlaps(&t, [1.0, 1.0], [2.0, 2.0]));
        // In the bounding box but beyond the long side.
        assert!(!overlaps(&t, [3.0, 3.0], [4.0, 4.0]));
        assert!(!overlaps(&t, [5.0, 0.0], [6.0, 1.0]));
        // Touching the side counts.
        assert!(overlaps(&t, [2.0, 2.0], [3.0, 3.0]));
    }

    #[test]
    fn a_walk_crosses_the_cells_in_order_and_stops() {
        let cells = cells_crossed([0.5, 0.5], [3.5, 0.5], 1.0, 100);
        assert_eq!(cells, [(0, 0), (1, 0), (2, 0), (3, 0)]);
        let diagonal = cells_crossed([0.5, 0.5], [2.5, 2.5], 1.0, 100);
        assert_eq!(diagonal.first(), Some(&(0, 0)));
        assert_eq!(diagonal.last(), Some(&(2, 2)));
        assert!(diagonal.len() <= 5);
        assert_eq!(cells_crossed([0.0, 0.0], [1e9, 0.0], 1.0, 10).len(), 10);
        // Not a number: one cell, no endless walk.
        assert!(cells_crossed([f64::NAN, 0.0], [1.0, 0.0], 1.0, 10).len() <= 10);
    }

    #[test]
    fn the_budget_says_too_much_work() {
        let mut budget = Budget {
            spent: 0,
            limit: 10,
        };
        assert!(budget.spend(10).is_ok());
        let error = budget.spend(1).unwrap_err().to_string();
        assert!(
            error.contains("too much work") && error.contains("coarser"),
            "{error}"
        );
    }
}
