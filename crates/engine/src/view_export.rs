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
//! - Every body is meshed at the export's chordal tolerance, with its
//!   edges as polylines through the very points the mesh has along them
//!   (`craft3d_tess::tessellate_body_and_edges`).
//! - A silhouette is where a face turns from facing the viewer to facing
//!   away: the edges of the mesh, inside one face, between a triangle that
//!   faces the viewer and one that faces away (the contour generator of
//!   the polyhedron; within the tolerance of the surface's own, since the
//!   mesh is). Flat faces have none; a face seen edge on has none either
//!   (its boundary is drawn as its edges).
//! - Visibility is decided per point, exactly for the mesh: a point is
//!   hidden when a triangle of any body covers it, strictly inside, and is
//!   nearer to the viewer there by more than half the tolerance. Lines are
//!   sampled at a spacing tied to the tolerance and the size of the model,
//!   and split where the verdict changes, the change found by bisection;
//!   a run shorter than the tolerance between runs of the other kind takes
//!   their kind. A line on the boundary of what hides it (an outline, a
//!   back edge in line with a front one) counts as seen.
//! - Drawn lines are straight pieces: a curve is the polyline of the
//!   mesh, within the tolerance. Pieces in line are merged, a piece that
//!   lies on one drawn before it is left out (visible first, then
//!   silhouettes, then hidden lines), and what follows on joins into one
//!   path. The same document gives the same bytes.
//!
//! Orthographic views are to scale: one drawing unit is one model unit,
//! the model's origin projected to the drawing's. A perspective view is
//! scaled so that what lies at the depth of the camera's target is true
//! size (nothing else is).
//!
//! Not drawn: datums and sketches, the section of the window (a drawing of
//! a cut is a separate matter: the bodies are drawn whole), and the scrub
//! (the document as it is). `view: "current"` is the window's camera
//! ([`Session::set_window_camera`]); without a window it is the iso view.

use crate::commands::{Run, fail};
use crate::params::Params;
use crate::{CommandError, Session};
use craft3d_drawing::{Color, Drawing, Layer, Path as DrawingPath, Point, Segment};
use craft3d_io::DrawingFormat;
use craft3d_math::{Bbox3, Vec3};
use craft3d_render::Projection;
use craft3d_tess::Mesh;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

const ID: &str = "file.export";

/// The parameters of a projected view.
pub(crate) const VIEW_KEYS: [&str; 3] = ["view", "projection", "hidden"];

/// Most points whose visibility is decided in one export.
const MAX_SAMPLES: usize = 4_000_000;
/// Most triangles of all the bodies.
const MAX_TRIANGLES: usize = 4_000_000;
/// Most entries in the grid of triangles.
const MAX_GRID_ENTRIES: usize = 64_000_000;
/// Bisection steps that place a change of visibility along a line.
const BISECTIONS: usize = 30;

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
    /// At or behind the eye of a perspective view: not drawn.
    Out,
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
    /// behind the eye.
    fn project(&self, p: Vec3) -> Option<(Point, f64)> {
        match self.perspective {
            None => Some(([p.dot(self.right), p.dot(self.up)], p.dot(self.forward))),
            Some((eye, scale)) => {
                let q = p - eye;
                let depth = q.dot(self.forward);
                if depth.is_nan() || depth <= self.near {
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
    /// (another triangle of the face is on its other side). A point on
    /// such a side is inside the face, so it counts as covered.
    inner: [bool; 3],
}

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

/// A triangle of a mesh as the drawing sees it, or `None` for one that is
/// seen edge on (no area in the drawing) or reaches behind the eye.
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

/// The sides inside a face as the drawing sees it: two triangles of the
/// same face meet there, both seen with area and turned the same way (the
/// face goes on across the side without folding over). A side next to a
/// triangle seen edge on, or where the face turns away, is an outline.
fn inner_sides(flats: &[([Vec3; 3], Flat, u32)], quantum: f64) -> BTreeMap<(Key3, Key3), bool> {
    let mut sides: BTreeMap<(Key3, Key3), Vec<(u32, bool)>> = BTreeMap::new();
    for ([a, b, c], flat, face) in flats {
        for (p, q) in [(*a, *b), (*b, *c), (*c, *a)] {
            sides
                .entry(side_key(p, q, quantum))
                .or_default()
                .push((*face, flat.area2 > 0.0));
        }
    }
    sides
        .into_iter()
        .map(|(key, list)| (key, matches!(list.as_slice(), [f, g] if f == g)))
        .collect()
}

/// The triangles of every body, in a grid over the drawing.
struct Occluders {
    flats: Vec<Flat>,
    origin: Point,
    cell: f64,
    side: usize,
    cells: Vec<Vec<u32>>,
    perspective: bool,
    /// How much nearer a triangle must be to hide a point.
    depth_margin: f64,
}

fn edge_fn(p: Point, q: Point, s: Point) -> f64 {
    (q[0] - p[0]) * (s[1] - p[1]) - (q[1] - p[1]) * (s[0] - p[0])
}

impl Occluders {
    fn new(
        meshes: &[Mesh],
        view: &View,
        scale: f64,
        depth_margin: f64,
        quantum: f64,
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
            let inner = inner_sides(&seen, quantum);
            let is_inner = |p: Vec3, q: Vec3| {
                inner
                    .get(&side_key(p, q, quantum))
                    .copied()
                    .unwrap_or(false)
            };
            for ([p, q, r], mut flat, _) in seen {
                flat.inner = [is_inner(q, r), is_inner(r, p), is_inner(p, q)];
                flats.push(flat);
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
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        let side = ((flats.len() as f64).sqrt().ceil() as usize).clamp(1, 512);
        let cell = if extent.is_finite() && extent > 0.0 {
            extent / side as f64 * (1.0 + 1e-9)
        } else {
            1.0
        };
        let origin = if lo[0].is_finite() { lo } else { [0.0, 0.0] };
        let mut cells: Vec<Vec<u32>> = vec![Vec::new(); side.saturating_mul(side)];
        let mut entries = 0_usize;
        let index = |v: f64, o: f64| -> usize {
            let i = ((v - o) / cell).floor();
            if i.is_finite() && i > 0.0 {
                (i as usize).min(side - 1)
            } else {
                0
            }
        };
        for (n, flat) in flats.iter().enumerate() {
            let xs = flat.at.map(|p| p[0]);
            let ys = flat.at.map(|p| p[1]);
            let (x0, x1) = (
                xs.iter().copied().fold(f64::INFINITY, f64::min),
                xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            );
            let (y0, y1) = (
                ys.iter().copied().fold(f64::INFINITY, f64::min),
                ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            );
            let (i0, i1) = (index(x0, origin[0]), index(x1, origin[0]));
            let (j0, j1) = (index(y0, origin[1]), index(y1, origin[1]));
            entries = entries.saturating_add((i1 - i0 + 1).saturating_mul(j1 - j0 + 1));
            if entries > MAX_GRID_ENTRIES {
                return fail(format!(
                    "{ID}: the view has too many triangles to decide what is hidden; give a coarser `tolerance`"
                ));
            }
            let id = u32::try_from(n)
                .map_err(|_| CommandError::new(format!("{ID}: too many triangles")))?;
            for j in j0..=j1 {
                for i in i0..=i1 {
                    if let Some(list) = cells.get_mut(j * side + i) {
                        list.push(id);
                    }
                }
            }
        }
        Ok(Self {
            flats,
            origin,
            cell,
            side,
            cells,
            perspective: view.perspective.is_some(),
            depth_margin,
        })
    }

    /// True when a triangle covers `s` (strictly inside) nearer than
    /// `depth` by more than the margin.
    fn hides(&self, s: Point, depth: f64) -> bool {
        let i = ((s[0] - self.origin[0]) / self.cell).floor();
        let j = ((s[1] - self.origin[1]) / self.cell).floor();
        if !(i >= 0.0 && j >= 0.0 && i < self.side as f64 && j < self.side as f64) {
            return false;
        }
        let Some(list) = self.cells.get(j as usize * self.side + i as usize) else {
            return false;
        };
        const INSIDE: f64 = 1e-9;
        for &n in list {
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
                return true;
            }
        }
        false
    }
}

/// Where a point is, seen from the eye.
fn seen(view: &View, occluders: &Occluders, p: Vec3) -> Seen {
    match view.project(p) {
        None => Seen::Out,
        Some((s, depth)) if occluders.hides(s, depth) => Seen::Behind,
        Some(_) => Seen::Yes,
    }
}

/// A piece of a line of one kind, in model space.
#[derive(Clone, Copy, Debug)]
struct Piece3 {
    from: Vec3,
    to: Vec3,
    seen: Seen,
}

/// Splits the line from `a` to `b` where its visibility changes.
fn classify(
    view: &View,
    occluders: &Occluders,
    a: Vec3,
    b: Vec3,
    spacing: f64,
    samples: &mut usize,
) -> Result<Vec<Piece3>, CommandError> {
    let length = a.distance(b);
    if !(length.is_finite() && length > 0.0) {
        return Ok(Vec::new());
    }
    let n = (length / spacing).ceil();
    let n = if n.is_finite() && n >= 1.0 {
        n as usize
    } else {
        1
    };
    *samples = samples.saturating_add(n);
    if *samples > MAX_SAMPLES {
        return fail(format!(
            "{ID}: the view has too many lines to decide what is hidden ({MAX_SAMPLES} points); give a coarser `tolerance`"
        ));
    }
    let at = |t: f64| a.lerp(b, t);
    let centre = |k: usize| (k as f64 + 0.5) / n as f64;
    let mut pieces = Vec::new();
    let mut start = 0.0;
    let mut state = seen(view, occluders, at(centre(0)));
    for k in 1..n {
        let next = seen(view, occluders, at(centre(k)));
        if next == state {
            continue;
        }
        // Between the two centres: where it changes.
        let (mut lo, mut hi) = (centre(k - 1), centre(k));
        for _ in 0..BISECTIONS {
            let mid = 0.5 * (lo + hi);
            if seen(view, occluders, at(mid)) == state {
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
    for r in 1..runs.len().saturating_sub(1) {
        let (Some(&(before, _)), Some(&(first, last)), Some(&(after, _))) =
            (runs.get(r - 1), runs.get(r), runs.get(r + 1))
        else {
            continue;
        };
        let (Some(b), Some(a)) = (pieces.get(before), pieces.get(after)) else {
            continue;
        };
        let around = b.seen;
        if around != a.seen {
            continue;
        }
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

/// Leaves out what lies on a piece drawn before it: the pieces are taken
/// in order (visible, silhouettes, hidden), and of each what another one
/// already covers is cut away.
fn without_overlaps(pieces: &[Piece], scale: f64, close: f64) -> Vec<Piece> {
    let cell = (scale / 64.0).max(f64::MIN_POSITIVE);
    let key = |v: f64| -> i64 {
        let k = (v / cell).floor();
        if k.is_finite() {
            k.clamp(-1e12, 1e12) as i64
        } else {
            0
        }
    };
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
        let (x0, x1) = (
            piece.a[0].min(piece.b[0]) - close,
            piece.a[0].max(piece.b[0]) + close,
        );
        let (y0, y1) = (
            piece.a[1].min(piece.b[1]) - close,
            piece.a[1].max(piece.b[1]) + close,
        );
        let mut seen_ids: Vec<usize> = Vec::new();
        for i in key(x0)..=key(x1) {
            for j in key(y0)..=key(y1) {
                if let Some(list) = grid.get(&(i, j)) {
                    seen_ids.extend(list.iter().copied());
                }
            }
        }
        seen_ids.sort_unstable();
        seen_ids.dedup();
        for id in seen_ids {
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
            let (x0, x1) = (part.a[0].min(part.b[0]), part.a[0].max(part.b[0]));
            let (y0, y1) = (part.a[1].min(part.b[1]), part.a[1].max(part.b[1]));
            for i in key(x0)..=key(x1) {
                for j in key(y0)..=key(y1) {
                    grid.entry((i, j)).or_default().push(index);
                }
            }
            kept.push(part);
        }
    }
    // Back in the order they were made, kind by kind.
    kept
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

/// A point of a mesh rounded to a grid, so that the two sides of a seam
/// (the same point, made twice) are one.
type Key3 = (i64, i64, i64);

/// A side of a triangle: the triangle, and its two ends.
type MeshEdge = (usize, Vec3, Vec3);

/// The silhouettes of a mesh: its edges, inside one face, between a
/// triangle facing the eye and one facing away, joined into polylines.
fn silhouettes(mesh: &Mesh, view: &View, quantum: f64) -> Vec<Vec<Vec3>> {
    let key = |p: Vec3| quantize(p, quantum);
    // Which way each triangle faces: 1 towards the eye, -1 away, 0 for a
    // triangle without a normal.
    let facing: Vec<i8> = (0..mesh.triangles.len())
        .map(|t| {
            let (Some(c), Some(tri)) = (mesh.corners(t), mesh.triangles.get(t)) else {
                return 0;
            };
            let [a, b, cc] = c;
            let mut n = (b - a).cross(cc - a);
            // Outward as the mesh's normals say, where it has them.
            let normals = tri.map(|i| mesh.normals.get(i as usize).copied().unwrap_or(Vec3::ZERO));
            let summed = normals[0] + normals[1] + normals[2];
            if n.dot(summed) < 0.0 {
                n = -n;
            }
            let centroid = (a + b + cc) * (1.0 / 3.0);
            match (n.normalized(), view.toward_eye(centroid)) {
                // A triangle seen edge on counts as facing the eye: a
                // strip of a cylinder seen exactly edge on is then on the
                // front side of the contour, which lies along it either
                // way (both its long sides fall on one line).
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
        .collect();
    let mut edges: BTreeMap<(Key3, Key3), Vec<MeshEdge>> = BTreeMap::new();
    for t in 0..mesh.triangles.len() {
        let Some(c) = mesh.corners(t) else {
            continue;
        };
        for (p, q) in [(c[0], c[1]), (c[1], c[2]), (c[2], c[0])] {
            let (kp, kq) = (key(p), key(q));
            if kp == kq {
                continue;
            }
            let k = if kp < kq { (kp, kq) } else { (kq, kp) };
            edges.entry(k).or_default().push((t, p, q));
        }
    }
    let mut segments: Vec<(Vec3, Vec3)> = Vec::new();
    for list in edges.values() {
        let [(t1, p, q), (t2, ..)] = list.as_slice() else {
            continue;
        };
        let same_face = mesh.face_ids.get(*t1) == mesh.face_ids.get(*t2);
        let (f1, f2) = (
            facing.get(*t1).copied().unwrap_or(0),
            facing.get(*t2).copied().unwrap_or(0),
        );
        if same_face && f1 * f2 == -1 {
            segments.push((*p, *q));
        }
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
    let view_name = p.str("view")?.unwrap_or("current");
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
    // The camera: a named view, or the window's.
    let window = session.window_camera;
    let (shown_view, toward_eye, up, window_eye) = if view_name == "current" {
        match window {
            Some(camera) => {
                let Some(dir) = (camera.eye - camera.target).normalized() else {
                    return fail(format!("{ID}: the window's camera looks nowhere"));
                };
                let eye = match camera.projection {
                    Projection::Perspective { .. } => Some((camera.eye, camera.target)),
                    Projection::Orthographic { .. } => None,
                };
                ("current", dir, camera.up, eye)
            }
            None => ("iso", VIEWS[0].1, VIEWS[0].2, None),
        }
    } else {
        let Some(&(name, dir, up)) = VIEWS.iter().find(|(name, ..)| *name == view_name) else {
            return fail(format!(
                "{ID}: unknown view `{view_name}`; use current (the window's), iso, front, back, left, right, top or bottom"
            ));
        };
        (name, dir, up, None)
    };
    let Some(dir) = toward_eye.normalized() else {
        return fail(format!("{ID}: the view looks nowhere"));
    };
    let Some(right) = up.cross(dir).normalized() else {
        return fail(format!("{ID}: the view's up lies along it"));
    };
    let view_up = dir.cross(right);

    // The bodies, meshed with their edges.
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = std::sync::Arc::clone(open.history.current());
    let tess = crate::commands::tolerance_param(p, ID, &regen)?;
    let tolerance = tess.chordal_tolerance;
    let mut meshes = Vec::new();
    let mut lines: Vec<Vec<Vec3>> = Vec::new();
    let mut bodies = 0_usize;
    let mut triangles = 0_usize;
    for (id, body) in regen.bodies().filter(|(id, _)| !doc.is_hidden(*id)) {
        let name = doc
            .feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone());
        let made = craft3d_tess::tessellate_body_and_edges(body, &tess)
            .map_err(|e| CommandError::new(format!("{ID}: {name}: {e}")))?;
        let mesh = made
            .mesh
            .map_err(|e| CommandError::new(format!("{ID}: {name}: {e}")))?;
        triangles = triangles.saturating_add(mesh.triangles.len());
        if triangles > MAX_TRIANGLES {
            return fail(format!(
                "{ID}: the bodies have more than {MAX_TRIANGLES} triangles at this tolerance; give a coarser `tolerance`"
            ));
        }
        lines.extend(made.edges);
        meshes.push(mesh);
        bodies += 1;
    }
    if bodies == 0 {
        return fail(format!(
            "{ID}: there are no bodies to draw (hidden ones are left out); add a feature, or show one with feature.visible"
        ));
    }
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
        Some((eye, eye.distance(target).max(f64::MIN_POSITIVE)))
    } else {
        None
    };
    let view = View {
        right,
        up: view_up,
        forward: -dir,
        perspective: perspective_eye,
        near: diagonal * 1e-6,
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
    let occluders = Occluders::new(
        &meshes,
        &view,
        scale,
        0.5 * tolerance + 1e-9 * diagonal,
        diagonal * 1e-7,
    )?;
    // The silhouettes come after the edges of every body.
    let edges_count = lines.len();
    for mesh in &meshes {
        lines.extend(silhouettes(mesh, &view, diagonal * 1e-7));
    }

    let spacing = (2.0 * tolerance).max(diagonal / 2000.0);
    let close = scale * 1e-7;
    let mut samples = 0_usize;
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
            parts.extend(classify(&view, &occluders, *a, *b, spacing, &mut samples)?);
        }
        smooth(&mut parts, tolerance);
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
    let kept = without_overlaps(&pieces, scale, close);
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
    Ok(json!({
        "path": path,
        "format": format.name(),
        "bytes": size,
        "unit": unit.symbol(),
        "view": shown_view,
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
