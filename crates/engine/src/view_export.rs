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
use crate::view_curves::{self, Drawn, Form, Registry};
use crate::{CommandError, Session};
use craft3d_drawing::{Color, Drawing, Layer, Path as DrawingPath, Point, Segment};
use craft3d_geom::Curve3;
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
/// The chords an edge is followed along keep within this share of the
/// tolerance of its curve.
const CHORD: f64 = 0.25;
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
pub(crate) struct View {
    /// Unit vectors: right and up in the drawing, and away from the eye.
    pub(crate) right: Vec3,
    pub(crate) up: Vec3,
    pub(crate) forward: Vec3,
    /// A perspective view's eye and the distance of its target, at which
    /// the drawing is true size.
    pub(crate) perspective: Option<(Vec3, f64)>,
    /// Nearest depth drawn, for a perspective view.
    pub(crate) near: f64,
}

impl View {
    /// The point in the drawing and its depth along the view, or `None`
    /// nearer than the near plane.
    pub(crate) fn project(&self, p: Vec3) -> Option<(Point, f64)> {
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

/// A drawn line: an edge of a body along its exact curve, or a
/// silhouette, a polyline of the mesh.
struct Track<'a> {
    base: Kind,
    /// The edge's curve; `None` for a silhouette.
    curve: Option<&'a Curve3>,
    /// A silhouette's points; its parameter is the place along them.
    points: Vec<Vec3>,
    /// The parameters of the ends of its chords, increasing.
    breaks: Vec<f64>,
    closed: bool,
    form: Form,
}

impl Track<'_> {
    /// The point at parameter `t`.
    fn at(&self, t: f64) -> Vec3 {
        if let Some(curve) = self.curve {
            return curve.eval(t);
        }
        let last = self.points.len().saturating_sub(1);
        let t = if t.is_finite() {
            t.clamp(0.0, last as f64)
        } else {
            0.0
        };
        let k = (t.floor() as usize).min(last.saturating_sub(1));
        match (self.points.get(k), self.points.get(k + 1)) {
            (Some(a), Some(b)) => a.lerp(*b, t - k as f64),
            (Some(a), None) => *a,
            _ => Vec3::ZERO,
        }
    }

    /// The first and the last parameter.
    fn ends(&self) -> Option<(f64, f64)> {
        Some((*self.breaks.first()?, *self.breaks.last()?))
    }

    /// The length of its parameter round a closed line.
    fn period(&self) -> Option<f64> {
        let (a, b) = self.ends()?;
        self.closed.then_some(b - a)
    }

    /// Where `t` lands in the drawing.
    fn drawn(&self, view: &View, t: f64) -> Option<Point> {
        view.project(self.at(t)).map(|(p, _)| p)
    }
}

/// A stretch of a track of one verdict, by its parameters, and its length
/// in model space.
#[derive(Clone, Copy, Debug)]
struct Stretch {
    t0: f64,
    t1: f64,
    seen: Seen,
    length: f64,
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
    /// How many samples a stretch of `length` takes, counted against
    /// [`MAX_SAMPLES`].
    fn count(&mut self, length: f64) -> Result<usize, CommandError> {
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
        Ok(n)
    }

    /// Splits the chord of `track` from parameter `a` to `b` where its
    /// visibility changes, its points on the exact curve.
    fn classify(
        &mut self,
        track: &Track<'_>,
        a: f64,
        b: f64,
    ) -> Result<Vec<Stretch>, CommandError> {
        let length = track.at(a).distance(track.at(b));
        if !(length.is_finite() && length > 0.0 && b > a) {
            return Ok(Vec::new());
        }
        let n = self.count(length)?;
        let (view, occluders) = (self.view, self.occluders);
        let budget = &mut self.budget;
        let param = |s: f64| a + (b - a) * s;
        let centre = |k: usize| (k as f64 + 0.5) / n as f64;
        let mut runs = Vec::new();
        let mut start = a;
        let mut state = seen(view, occluders, track.at(param(centre(0))), budget)?;
        for k in 1..n {
            let next = seen(view, occluders, track.at(param(centre(k))), budget)?;
            if next == state {
                continue;
            }
            // Between the two centres: where it changes.
            let (mut lo, mut hi) = (centre(k - 1), centre(k));
            for _ in 0..BISECTIONS {
                let mid = 0.5 * (lo + hi);
                if seen(view, occluders, track.at(param(mid)), budget)? == state {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let cut = param(0.5 * (lo + hi));
            runs.push(Stretch {
                t0: start,
                t1: cut,
                seen: state,
                length: 0.0,
            });
            start = cut;
            state = next;
        }
        runs.push(Stretch {
            t0: start,
            t1: b,
            seen: state,
            length: 0.0,
        });
        for run in &mut runs {
            run.length = track.at(run.t0).distance(track.at(run.t1));
        }
        Ok(runs)
    }
}

/// A run shorter than `shortest` between two runs of one other kind takes
/// theirs: a verdict that flickers at the tolerance's scale is noise.
fn smooth(pieces: &mut [Stretch], shortest: f64) {
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
            .map_or(0.0, |run| run.iter().map(|p| p.length).sum());
        if length < shortest {
            for piece in pieces.get_mut(first..=last).into_iter().flatten() {
                piece.seen = around;
            }
        }
    }
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

/// Cells of a grid of `cell` that the segment from `a` to `b` crosses, in
/// order (a walk from cell to cell along it), at most `cap` of them.
pub(crate) fn cells_crossed(a: Point, b: Point, cell: f64, cap: usize) -> Vec<(i64, i64)> {
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

/// A stretch of a track kept for the drawing.
#[derive(Clone, Copy, Debug)]
struct Kept {
    line: usize,
    kind: Kind,
    t0: f64,
    t1: f64,
}

/// What the overlap step needs.
struct Overlaps<'a> {
    view: &'a View,
    /// How far a silhouette of the mesh may lie from the true one.
    tolerance: f64,
    close: f64,
    registry: Registry,
}

impl Overlaps<'_> {
    /// Leaves out of the stretch `t0`..`t1` of `track` (number `line`)
    /// what lies on what is drawn before it, chord by chord: its points
    /// are sampled and a change between covered and not placed by
    /// bisection, as visibility is. What is left is drawn and recorded.
    #[allow(clippy::too_many_arguments)]
    fn keep(
        &mut self,
        track: &Track<'_>,
        line: usize,
        kind: Kind,
        (t0, t1): (f64, f64),
        sampling: &mut Sampling<'_>,
        kept: &mut Vec<Kept>,
    ) -> Result<(), CommandError> {
        let view = self.view;
        let close = self.close;
        let period = track.period();
        let first = track.breaks.partition_point(|b| *b <= t0).saturating_sub(1);
        for k in first..track.breaks.len() {
            let (Some(&c0), Some(&c1)) = (track.breaks.get(k), track.breaks.get(k + 1)) else {
                break;
            };
            if c0 >= t1 {
                break;
            }
            let (s0, s1) = (t0.max(c0), t1.min(c1));
            if !(s1 > s0) {
                continue;
            }
            let (Some(p0), Some(p1)) = (track.drawn(view, c0), track.drawn(view, c1)) else {
                continue;
            };
            // How far the curve may lie from this chord in the drawing.
            let slack = if track.curve.is_some() {
                [0.25, 0.5, 0.75]
                    .iter()
                    .filter_map(|u| track.drawn(view, c0 + (c1 - c0) * u))
                    .map(|q| view_curves::to_segment(q, p0, p1))
                    .fold(0.0, f64::max)
            } else {
                self.tolerance
            };
            let dir = sub2(p1, p0);
            let (Some(q0), Some(q1)) = (track.drawn(view, s0), track.drawn(view, s1)) else {
                continue;
            };
            let length = view_curves::dist2(q0, q1);
            if !(length > close) {
                continue;
            }
            let n = sampling.count(length)?;
            let registry = &self.registry;
            let budget = &mut sampling.budget;
            let mut covered = |s: f64| -> Result<bool, CommandError> {
                let Some(p) = track.drawn(view, s0 + (s1 - s0) * s) else {
                    return Ok(true);
                };
                let (on, tests) = registry.covers(p, slack, line, (c0, c1), dir, period, close);
                budget.spend(tests)?;
                Ok(on)
            };
            let centre = |j: usize| (j as f64 + 0.5) / n as f64;
            let mut stretches: Vec<(f64, f64)> = Vec::new();
            let mut start = 0.0;
            let mut state = covered(centre(0))?;
            for j in 1..n {
                let next = covered(centre(j))?;
                if next == state {
                    continue;
                }
                let (mut lo, mut hi) = (centre(j - 1), centre(j));
                for _ in 0..BISECTIONS {
                    let mid = 0.5 * (lo + hi);
                    if covered(mid)? == state {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let cut = 0.5 * (lo + hi);
                if !state {
                    stretches.push((start, cut));
                }
                start = cut;
                state = next;
            }
            if !state {
                stretches.push((start, 1.0));
            }
            for (a, b) in stretches {
                // The ends at the chord's own parameters where they are.
                let u0 = if a == 0.0 { s0 } else { s0 + (s1 - s0) * a };
                let u1 = if b == 1.0 { s1 } else { s0 + (s1 - s0) * b };
                let (Some(a), Some(b)) = (track.drawn(view, u0), track.drawn(view, u1)) else {
                    continue;
                };
                if view_curves::dist2(a, b) <= close {
                    continue;
                }
                self.registry.add(Drawn {
                    a,
                    b,
                    slack,
                    line,
                    t0: u0,
                    t1: u1,
                });
                kept.push(Kept {
                    line,
                    kind,
                    t0: u0,
                    t1: u1,
                });
            }
        }
        Ok(())
    }
}

/// The path of the stretches `pieces` of `track` (one, or two that join
/// across the start of a closed line), whole when it is all of a closed
/// line; and how many pieces of the drawing it has.
fn path_of(
    track: &Track<'_>,
    pieces: &[(f64, f64)],
    whole: bool,
    view: &View,
    close: f64,
) -> Option<(DrawingPath, usize)> {
    let at = |t: f64| track.drawn(view, t);
    let (&(first, _), &(_, last)) = (pieces.first()?, pieces.last()?);
    let chords = || {
        let mut points = Vec::new();
        for &(a, b) in pieces {
            points.push(at(a)?);
            for t in track.breaks.iter().filter(|t| **t > a && **t < b) {
                points.push(at(*t)?);
            }
            points.push(at(b)?);
        }
        let path = view_curves::chain_of(&points, whole, close)?;
        let count = match &path {
            DrawingPath::Chain { segments, .. } => segments.len(),
            _ => 1,
        };
        Some((path, count))
    };
    match &track.form {
        Form::Line => {
            let (a, b) = (at(first)?, at(last)?);
            (view_curves::dist2(a, b) > close).then(|| {
                (
                    DrawingPath::Chain {
                        start: a,
                        segments: vec![Segment::Line { to: b }],
                        closed: false,
                    },
                    1,
                )
            })
        }
        Form::Conic {
            center,
            major,
            ratio,
        } => {
            let (center, major, ratio) = (*center, *major, *ratio);
            let radius = dot2(major, major).sqrt();
            let full = || {
                if ratio == 1.0 {
                    DrawingPath::Circle { center, radius }
                } else {
                    DrawingPath::Ellipse {
                        center,
                        major,
                        ratio,
                    }
                }
            };
            if whole {
                return Some((full(), 1));
            }
            let (from, to) = (at(first)?, at(last)?);
            if view_curves::dist2(from, to) <= close {
                // Round all but a sliver: the whole curve.
                let span: f64 = pieces.iter().map(|(a, b)| b - a).sum();
                let half = track.period().map_or(f64::INFINITY, |p| 0.5 * p);
                return (span > half).then(|| (full(), 1));
            }
            // Which way it runs: at the middle of its first stretch.
            let (a, b) = pieces.first().copied()?;
            let (mid, h) = (0.5 * (a + b), 1e-3 * (b - a));
            let turn = cross2(sub2(at(mid)?, center), sub2(at(mid + h)?, at(mid - h)?));
            let ccw = turn > 0.0;
            let segment = if ratio == 1.0 {
                Segment::Arc {
                    to,
                    center,
                    radius,
                    ccw,
                }
            } else {
                Segment::EllipticArc {
                    to,
                    center,
                    major,
                    ratio,
                    ccw,
                }
            };
            Some((
                DrawingPath::Chain {
                    start: from,
                    segments: vec![segment],
                    closed: false,
                },
                1,
            ))
        }
        Form::Spline(spline) => {
            let start = at(first)?;
            let mut segments = Vec::new();
            for &(a, b) in pieces {
                let piece = spline.between(a, b).ok();
                let (Some(piece), Some(from), Some(to)) = (piece, at(a), at(b)) else {
                    return chords();
                };
                let to = if whole { start } else { to };
                segments.push(Segment::Spline(piece.with_ends(from, to)));
            }
            let count = segments.len();
            Some((
                DrawingPath::Chain {
                    start,
                    segments,
                    closed: whole,
                },
                count,
            ))
        }
        Form::Chords => chords(),
    }
}

/// The paths of the kept stretches of one kind: the stretches of a line
/// that follow on from each other make one path, and on a closed line the
/// last and the first join across its start.
fn paths_of(
    tracks: &[Track<'_>],
    kept: &[Kept],
    kind: Kind,
    view: &View,
    close: f64,
) -> (Vec<DrawingPath>, usize) {
    let mut by_line: BTreeMap<usize, Vec<(f64, f64)>> = BTreeMap::new();
    for piece in kept.iter().filter(|k| k.kind == kind) {
        by_line
            .entry(piece.line)
            .or_default()
            .push((piece.t0, piece.t1));
    }
    let mut paths = Vec::new();
    let mut count = 0;
    for (line, mut stretches) in by_line {
        let Some(track) = tracks.get(line) else {
            continue;
        };
        stretches.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut groups: Vec<(f64, f64)> = Vec::new();
        for (a, b) in stretches {
            match groups.last_mut() {
                Some(last) if last.1 == a => last.1 = b,
                _ => groups.push((a, b)),
            }
        }
        let Some((start, end)) = track.ends() else {
            continue;
        };
        let mut lists: Vec<(Vec<(f64, f64)>, bool)> = Vec::new();
        let ends = (groups.first().copied(), groups.last().copied());
        let inner: &[(f64, f64)] = match ends {
            (Some(first), Some(last))
                if track.closed && groups.len() >= 2 && first.0 == start && last.1 == end =>
            {
                lists.push((vec![last, first], false));
                groups
                    .get(1..groups.len().saturating_sub(1))
                    .unwrap_or_default()
            }
            _ => &groups,
        };
        for g in inner {
            let whole = track.closed && g.0 == start && g.1 == end;
            lists.push((vec![*g], whole));
        }
        for (pieces, whole) in lists {
            if let Some((path, n)) = path_of(track, &pieces, whole, view, close) {
                paths.push(path);
                count += n;
            }
        }
    }
    (paths, count)
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
    let asked = match p.number("tolerance")? {
        Some(t) if t > 0.0 && t.is_finite() => t,
        Some(_) => return fail(format!("{ID}: `tolerance` must be positive")),
        None => crate::commands::automatic_tolerance(&regen),
    };
    let tolerance: f64 = asked.max(size * TOLERANCE_FLOOR);
    let tess = craft3d_tess::TessParams {
        chordal_tolerance: tolerance,
        ..craft3d_tess::TessParams::default()
    };
    let mut meshes = Vec::new();
    let mut tracks: Vec<Track<'_>> = Vec::new();
    let mut breaks_total = 0_usize;
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
        // A seam (both sides on one face) is no edge one sees. Every
        // other edge is followed along its own curve.
        let uses = craft3d_topo::edge_faces(body);
        for (index, edge) in body.edges.iter().enumerate() {
            let seam = matches!(uses.get(index).map(Vec::as_slice), Some([a, b]) if a == b);
            if seam {
                continue;
            }
            let breaks = view_curves::breaks_of(&edge.curve, edge.range, CHORD * tolerance);
            breaks_total = breaks_total.saturating_add(breaks.len());
            if breaks_total > MAX_SAMPLES {
                return fail(format!(
                    "{ID}: the view has too many lines to decide what is hidden ({MAX_SAMPLES} points); use a coarser `tolerance` or a simpler view"
                ));
            }
            if breaks.len() >= 2 {
                tracks.push(Track {
                    base: Kind::Visible,
                    curve: Some(&edge.curve),
                    points: Vec::new(),
                    breaks,
                    closed: edge.start == edge.end,
                    form: Form::Chords,
                });
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
    let close = scale * 1e-7;
    // What each edge is in the drawing.
    for track in &mut tracks {
        if let Some(curve) = track.curve {
            track.form = view_curves::form_of(curve, &view, close);
        }
    }
    // The silhouettes come after the edges of every body.
    for mesh in &meshes {
        for points in silhouettes(mesh, &view, diagonal * 1e-7) {
            let closed = points.len() > 3
                && matches!((points.first(), points.last()), (Some(a), Some(b)) if a.distance(*b) <= diagonal * 1e-7);
            tracks.push(Track {
                base: Kind::Silhouette,
                curve: None,
                breaks: (0..points.len()).map(|k| k as f64).collect(),
                points,
                closed,
                form: Form::Chords,
            });
        }
    }

    let mut sampling = Sampling {
        view: &view,
        occluders: &occluders,
        spacing: (2.0 * tolerance).max(diagonal / 2000.0),
        samples: 0,
        budget,
    };
    // Each line split where it goes behind something, in its parameter.
    let mut runs: Vec<(usize, Kind, f64, f64)> = Vec::new();
    for (index, track) in tracks.iter().enumerate() {
        let mut parts: Vec<Stretch> = Vec::new();
        for pair in track.breaks.windows(2) {
            let [a, b] = pair else { continue };
            parts.extend(sampling.classify(track, *a, *b)?);
        }
        // Within the depth margin of a surface the verdict is a guess.
        smooth(&mut parts, (1.0 + DEPTH_MARGIN) * tolerance);
        for part in parts {
            let kind = match part.seen {
                Seen::Out => continue,
                Seen::Yes => track.base,
                Seen::Behind if dashed => Kind::Hidden,
                Seen::Behind => continue,
            };
            match runs.last_mut() {
                Some(last) if last.0 == index && last.1 == kind && last.3 == part.t0 => {
                    last.3 = part.t1;
                }
                _ => runs.push((index, kind, part.t0, part.t1)),
            }
        }
    }
    // What lies on a line drawn before it is left out: visible first,
    // then silhouettes, then hidden lines.
    runs.sort_by_key(|run| run.1);
    let reach = close + 2.0 * tolerance;
    let mut overlaps = Overlaps {
        view: &view,
        tolerance,
        close,
        registry: Registry::new((scale / 256.0).max(4.0 * reach), 1024),
    };
    let mut kept: Vec<Kept> = Vec::new();
    for (line, kind, t0, t1) in runs {
        let Some(track) = tracks.get(line) else {
            continue;
        };
        overlaps.keep(track, line, kind, (t0, t1), &mut sampling, &mut kept)?;
    }
    let mut kinds: BTreeMap<Kind, usize> = BTreeMap::new();
    let mut layers = Vec::new();
    let dash = (scale / 60.0).max(f64::MIN_POSITIVE);
    for (name, kind, dash) in [
        ("visible", Kind::Visible, None),
        ("silhouette", Kind::Silhouette, None),
        ("hidden", Kind::Hidden, Some(dash)),
    ] {
        if kind == Kind::Hidden && !dashed {
            continue;
        }
        let (paths, count) = paths_of(&tracks, &kept, kind, &view, close);
        kinds.insert(kind, count);
        if !paths.is_empty() {
            layers.push(Layer {
                name: name.to_string(),
                color: Color::Black,
                dash,
                dot: false,
                paths,
            });
        }
    }
    if layers.is_empty() {
        return fail(format!("{ID}: nothing of the bodies is seen in this view"));
    }
    let of_kind = |kind: Kind| kinds.get(&kind).copied().unwrap_or(0);
    let (visible, silhouette, hidden) = (
        of_kind(Kind::Visible),
        of_kind(Kind::Silhouette),
        of_kind(Kind::Hidden),
    );
    let unit = doc.units.length;
    let drawing = Drawing {
        unit: crate::drawing_cmds::drawing_unit(unit),
        layers,
    };
    let stroke = crate::drawing_cmds::stroke(p, format)?;
    let (bytes, _) = craft3d_io::encode_drawing_stroked(&drawing, format, tolerance, stroke)
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
