//! A cover of a face by boxes of patches of its surface, so that two faces
//! whose boxes meet but which lie apart are told apart before their
//! surfaces are intersected, and a point far from a face is known to be
//! outside it without its outline being asked (docs/design/booleans.md ›
//! *Faces that lie near one another*).
//!
//! The box of a curved face is loose: a piece of a cylinder turned off the
//! world's axes fills its box only thinly, and in a body of many such
//! pieces (rings and tubes of a pattern, all near one another) nearly
//! every pair's boxes meet, while their faces do not. A face's cover is a
//! tree over its surface's parameters, halved along the direction in
//! which a node is longer in space, each node the box of its patch: the
//! box of the patch's four corners grown by how far the patch can stray
//! from the bilinear patch through them, `(hu² max|S_uu| + hv² max|S_vv|)
//! / 8` (the error bound of bilinear interpolation, along any direction in
//! space), so every point of the patch is in its box. A node is left out
//! only when its patch lies wholly outside the face: no piece of the
//! face's boundary comes within its box (each piece measured: a line or a
//! circle in closed form, any other curve by halving with its speed bound,
//! never by sampling), so the boundary does not cross the patch, which
//! hangs together and is then all on one side of it, and its middle is
//! outside the face. A node is not divided below a part of the face's box
//! ([`PARTS`]) or a depth ([`DEPTH`]). So every point of the face is in a
//! leaf of its cover, and two faces whose covers do not come within a
//! margin of each other are further apart than that margin.
//!
//! The tree is made as it is asked: a node's children the first time a
//! question needs them, so the work goes where the questions are (a face
//! asked once costs a few patches, not a whole cover). Every node is a
//! function of the face alone. How far a question goes down does depend
//! on what was asked before (a face asked often starts from finer first
//! nodes, a question gives up after [`PAIRS`] pairs of nodes, a tree stops
//! dividing after [`NODES`] nodes), but only ever towards the answer "may
//! meet" or "may hold", which is what the boolean assumed without a
//! cover: a cover is only used to pass over what is proven not to meet, or
//! to answer "outside" where the outline would, so what remains is worked
//! on exactly as before and the results are the same bit for bit, in any
//! order of questions and on any thread.
//!
//! Memory: a tree's first nodes are made at once, no shorter than a
//! quarter (a sixteenth, for the finer ones) of the face box's diagonal:
//! for an analytic face at most `(FRONT · π)²` of them, a few hundred
//! (`(FINE · π)²`, about 2,500); below them at most [`NODES`] more are made
//! per tree. A cover lives as long as the boolean that made it.
//!
//! A freeform face (Bézier or NURBS) is covered the same way from the
//! Bézier pieces of its surface, one first node per knot span: a piece
//! lies in the convex hull of its control points (its weights are
//! positive), so the box of those points, grown by what rounding may have
//! moved them by ([`PIECE_ROUNDING`]), holds it, and a node's halves are
//! its piece split in two by de Casteljau's algorithm. How long a node is
//! comes from the rows of its control polygon. A node keeps only the
//! index of its span: its piece is cut out of the span's when the node
//! is halved, so the cover holds the surface's pieces once (a bicubic
//! one some 512 bytes) beside its nodes. A surface of more than
//! [`MOST_PIECES`] spans, or with a weight that is not positive, gets no
//! cover (its box stands for it), and a front holds at most
//! [`FIRST_NODES`] nodes. Being outside a freeform face by its
//! cover rests on what it rests on for an analytic one: a point within
//! the tolerance of the surface and further than the cover's margin from
//! the face has its nearest point of the surface outside the face, which
//! is what the outline reads too.

use super::freeform::is_freeform;
use super::hits::speed_bound;
use super::outline::{Outline, Side, boxes_meet, nearest};
use crate::boxtree::BoxTree;
use craft3d_geom::{Curve3, RationalBezierPatch, Surface};
use craft3d_math::{Bbox3, Vec3};
use std::f64::consts::{FRAC_PI_2, TAU};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A leaf is no longer than this part of the face's box's diagonal, along
/// either direction of the parameters.
const PARTS: f64 = 64.0;

/// The first nodes are no longer than this part of the face's box's
/// diagonal, along either direction of the parameters, and those of a
/// face asked about often no longer than [`FINE`].
const FRONT: f64 = 4.0;
const FINE: f64 = 16.0;

/// How many questions a face is asked before it gets its finer first
/// nodes.
const OFTEN: usize = 64;

/// Fewer first nodes than this are not worth a tree.
const SMALL: usize = 16;

/// Nodes deeper than this are not divided (a leaf stays longer).
const DEPTH: u32 = 16;

/// How many nodes a tree makes below its first nodes, at most; past that
/// its nodes are leaves (a coarser cover, never a smaller one).
const NODES: usize = 1 << 13;

/// How many pairs of nodes one question of whether two covers meet looks
/// at, at most, before it answers that they may (the answer it would give
/// without a cover).
const PAIRS: usize = 4096;

/// A freeform face of more Bézier pieces than this has no cover (its box
/// stands for it).
const MOST_PIECES: usize = 4096;

/// How many first nodes a front holds at most: past that, nodes are
/// first nodes undivided (a coarser front, never a smaller cover). An
/// analytic face's front stays below it (about `(FINE · π)²`); a freeform
/// face's has a node per knot span at least.
const FIRST_NODES: usize = 1 << 12;

/// How far, relative to the largest coordinate, the control points of a
/// piece may have been moved by rounding: a NURBS surface's pieces come
/// from knot insertions and are halved down to [`DEPTH`] times, each a
/// few roundings per point relative to that size (some 10⁻¹⁴ in all);
/// the cover's margins are tolerances beyond that.
const PIECE_ROUNDING: f64 = 1e-12;

const _: () = assert!(MOST_PIECES <= FIRST_NODES);

/// What a face's cover is made from: its surface, its box, its outline
/// (none: no patch is left out by it) and the tolerance.
#[derive(Clone, Copy)]
pub(super) struct Face<'a> {
    pub surface: &'a Surface,
    pub bbox: &'a Bbox3,
    pub outline: Option<&'a Outline>,
    pub tol: f64,
}

/// The boxes that cover a face: its first nodes, no longer than a part of
/// the face's box, made at once and held in a tree of their boxes, and
/// below them a tree made as it is asked. A face asked about often gets
/// finer first nodes ([`FINE`]) once it has been asked [`OFTEN`] times:
/// what a question costs then goes into the tree of boxes, which answers
/// it faster than going down from coarse nodes each time.
pub(super) struct Cover {
    /// Of a freeform face: the Bézier pieces of its surface.
    spans: Option<Vec<(Rect, RationalBezierPatch)>>,
    coarse: Front,
    fine: OnceLock<Option<Front>>,
    asked: AtomicUsize,
    /// Nodes made below the first ones, for [`NODES`].
    made: AtomicUsize,
    /// How long a leaf is at most, along either direction.
    widest: f64,
}

/// First nodes, and the tree of their boxes when there are many.
struct Front {
    roots: Vec<Node>,
    tree: Option<BoxTree<Bbox3>>,
}

struct Node {
    /// The patch's parameters: `u` from `.0` to `.1`, `v` from `.2` to `.3`.
    rect: (f64, f64, f64, f64),
    bbox: Bbox3,
    /// Wholly inside the face: its children are too.
    inside: bool,
    depth: u32,
    /// The children that are not wholly outside the face (perhaps none);
    /// `None` for a leaf.
    children: OnceLock<Option<Vec<Node>>>,
    /// Of a freeform face: the knot span the patch lies in (its piece is
    /// cut out of the span's piece when the node is halved).
    span: Option<u32>,
}

/// The Bézier pieces of a freeform face's surface, one per knot span,
/// with the parameters each covers.
type Spans = [(Rect, RationalBezierPatch)];

/// Per direction of the parameters the speed (`|S_u|`, `|S_v|`) and the
/// bend (`|S_uu|`, `|S_vv|`) at most, over the parameters with `v` in
/// `v`.
fn bounds(surface: &Surface, v: (f64, f64)) -> Option<[f64; 4]> {
    Some(match surface {
        Surface::Plane(_) => [1.0, 1.0, 0.0, 0.0],
        Surface::Cylinder(c) => [c.radius(), 1.0, c.radius(), 0.0],
        Surface::Cone(c) => {
            let slope = c.half_angle().tan();
            let r = (c.radius() + v.0 * slope)
                .abs()
                .max((c.radius() + v.1 * slope).abs());
            [r, slope.hypot(1.0), r, 0.0]
        }
        Surface::Sphere(s) => [s.radius(), s.radius(), s.radius(), s.radius()],
        Surface::Torus(t) => {
            let (major, minor) = t.radii();
            let ring = major.abs() + minor.abs();
            [ring, minor.abs(), ring, minor.abs()]
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    })
}

/// The rectangle of parameters the face's points lie in, from its box:
/// a whole turn where the surface closes, and along a direction in which
/// it is unbounded the stretch the box's corners give.
fn parameters(surface: &Surface, face: &Bbox3) -> Option<(f64, f64, f64, f64)> {
    let (lo, hi) = face.corners()?;
    let corners = [
        Vec3::new(lo.x, lo.y, lo.z),
        Vec3::new(hi.x, lo.y, lo.z),
        Vec3::new(lo.x, hi.y, lo.z),
        Vec3::new(hi.x, hi.y, lo.z),
        Vec3::new(lo.x, lo.y, hi.z),
        Vec3::new(hi.x, lo.y, hi.z),
        Vec3::new(lo.x, hi.y, hi.z),
        Vec3::new(hi.x, hi.y, hi.z),
    ];
    let span = |origin: Vec3, direction: Vec3| {
        let mut s = (f64::INFINITY, f64::NEG_INFINITY);
        for c in corners {
            let t = (c - origin).dot(direction);
            s = (s.0.min(t), s.1.max(t));
        }
        s
    };
    let turn = (0.0, TAU);
    let (u, v) = match surface {
        Surface::Plane(p) => {
            let f = p.frame();
            (span(f.origin(), f.x()), span(f.origin(), f.y()))
        }
        Surface::Cylinder(c) => (turn, span(c.frame().origin(), c.frame().z())),
        Surface::Cone(c) => {
            let along = span(c.frame().origin(), c.frame().z());
            // Only the nappe the surface is.
            let (_, (from, to)) = surface.domain();
            (turn, (along.0.max(from), along.1.min(to)))
        }
        Surface::Sphere(_) => (turn, (-FRAC_PI_2, FRAC_PI_2)),
        Surface::Torus(_) => (turn, turn),
        Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    };
    let ok = |(a, b): (f64, f64)| a.is_finite() && b.is_finite() && a <= b;
    (ok(u) && ok(v)).then_some((u.0, u.1, v.0, v.1))
}

/// The node over `rect`, or `None` when its patch is wholly outside the
/// face (beyond its box, or proven outside its outline). `Err(())` when
/// the numbers are not finite: the face then has no cover.
fn node(
    face: Face,
    rect: Rect,
    depth: u32,
    inside: bool,
    piece: Option<(&RationalBezierPatch, u32)>,
) -> Result<Option<Node>, ()> {
    let (a, b, c, d) = rect;
    let (corners, stray) = match piece {
        // The convex hull of the piece's control points holds it (the
        // weights are positive), grown by what rounding may have moved
        // them by in the knot insertions and halvings that made them.
        Some((piece, _)) => {
            let hull = piece.control_bbox();
            let (lo, hi) = hull.corners().ok_or(())?;
            let size = [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]
                .into_iter()
                .fold(0.0, |m: f64, x| m.max(x.abs()));
            (hull, PIECE_ROUNDING * size)
        }
        None => {
            let [_, _, bend_u, bend_v] = bounds(face.surface, (c, d)).ok_or(())?;
            let (hu, hv) = (b - a, d - c);
            let mut corners = Bbox3::EMPTY;
            for (u, v) in [(a, c), (b, c), (a, d), (b, d)] {
                corners.add(face.surface.eval(u, v));
            }
            (corners, (hu * hu * bend_u + hv * hv * bend_v) / 8.0)
        }
    };
    let (lo, hi) = corners.corners().ok_or(())?;
    if !(stray.is_finite() && lo.is_finite() && hi.is_finite()) {
        return Err(());
    }
    let grow = Vec3::new(stray, stray, stray);
    let (lo, hi) = (lo - grow, hi + grow);
    // Nothing of the face is outside its box (grown by the tolerance): the
    // patch's box is cut to it, and a patch beyond it holds nothing of
    // the face. (The boundary lies in that box too, so where it does not
    // come within the cut box it does not cross the patch.)
    let (face_lo, face_hi) = face.bbox.corners().ok_or(())?;
    let pad = Vec3::new(face.tol, face.tol, face.tol);
    let (lo, hi) = (lo.max(face_lo - pad), hi.min(face_hi + pad));
    if !(lo.x <= hi.x && lo.y <= hi.y && lo.z <= hi.z) {
        return Ok(None);
    }
    let bbox = Bbox3::from_points([lo, hi]);
    let mut inside = inside;
    if !inside && let Some(outline) = face.outline {
        let middle = lo.lerp(hi, 0.5);
        let reach = 0.5 * lo.distance(hi) + 2.0 * face.tol;
        if !boundary_within(outline, middle, reach) {
            let at = face.surface.eval(0.5 * (a + b), 0.5 * (c + d));
            match outline.side(at, face.tol) {
                Ok(Side::Out) => return Ok(None),
                Ok(Side::In) => inside = true,
                _ => {}
            }
        }
    }
    Ok(Some(Node {
        rect,
        bbox,
        inside,
        depth,
        children: OnceLock::new(),
        span: piece.map(|(_, span)| span),
    }))
}

/// Parameters: `u` from `.0` to `.1`, `v` from `.2` to `.3`.
type Rect = (f64, f64, f64, f64);

/// A half of a node: its parameters and, of a freeform face, its piece.
type Half = (Rect, Option<RationalBezierPatch>);

/// The two halves of node `n`'s parameters, halved along the direction
/// in which it is longer in space, or `None` when it is no longer than
/// `widest` either way (or as deep as nodes go). How long a node of a
/// freeform face is comes from its piece: no curve of constant `v` (or
/// `u`) on it is longer than the longest row (column) of its control
/// polygon (a plane meets the curve no more often than the row, so by
/// Crofton's formula the curve is no longer).
fn halves(face: Face, n: &Node, widest: f64, spans: &Spans) -> Option<[Half; 2]> {
    let (a, b, c, d) = n.rect;
    let piece = match n.span {
        Some(span) => Some(piece_of(spans, span, n.rect)?),
        None => None,
    };
    let (long_u, long_v) = match &piece {
        Some(piece) => polygon_lengths(piece)?,
        None => {
            let [speed_u, speed_v, _, _] = bounds(face.surface, (c, d))?;
            ((b - a) * speed_u, (d - c) * speed_v)
        }
    };
    if n.depth >= DEPTH || !(long_u > widest || long_v > widest) {
        return None;
    }
    Some(if long_u >= long_v {
        let m = 0.5 * (a + b);
        let (low, high) = match &piece {
            Some(piece) => {
                let (low, high) = piece.split_u(0.5);
                (Some(low), Some(high))
            }
            None => (None, None),
        };
        [((a, m, c, d), low), ((m, b, c, d), high)]
    } else {
        let m = 0.5 * (c + d);
        let (low, high) = match &piece {
            Some(piece) => {
                let (low, high) = piece.split_v(0.5);
                (Some(low), Some(high))
            }
            None => (None, None),
        };
        [((a, b, c, m), low), ((a, b, m, d), high)]
    })
}

/// The piece of the surface over `rect`, cut out of the piece of knot
/// span `span` (split where `rect` begins and ends; the halves of a node
/// are then split from it in the middle).
fn piece_of(spans: &Spans, span: u32, rect: Rect) -> Option<RationalBezierPatch> {
    let ((u0, u1, v0, v1), whole) = spans.get(usize::try_from(span).ok()?)?;
    let (a, b, c, d) = rect;
    let share = |x: f64, from: f64, to: f64| ((x - from) / (to - from)).clamp(0.0, 1.0);
    let (s0, s1) = (share(a, *u0, *u1), share(b, *u0, *u1));
    let (t0, t1) = (share(c, *v0, *v1), share(d, *v0, *v1));
    if !(s0 < s1 && t0 < t1) {
        return None;
    }
    let mut piece = whole.clone();
    if s1 < 1.0 {
        piece = piece.split_u(s1).0;
    }
    if s0 > 0.0 {
        piece = piece.split_u(s0 / s1).1;
    }
    if t1 < 1.0 {
        piece = piece.split_v(t1).0;
    }
    if t0 > 0.0 {
        piece = piece.split_v(t0 / t1).1;
    }
    Some(piece)
}

/// The longest row of a piece's control polygon along `u` and the
/// longest along `v`: bounds on the lengths of its curves of constant
/// `v` and of constant `u`.
fn polygon_lengths(piece: &RationalBezierPatch) -> Option<(f64, f64)> {
    let (du, dv) = piece.degrees();
    let points = piece.control_points();
    let at = |i: usize, j: usize| points.get(i.checked_mul(dv + 1)?.checked_add(j)?).copied();
    let (mut long_u, mut long_v) = (0.0_f64, 0.0_f64);
    for j in 0..=dv {
        let mut length = 0.0;
        for i in 0..du {
            length += at(i + 1, j)?.distance(at(i, j)?);
        }
        long_u = long_u.max(length);
    }
    for i in 0..=du {
        let mut length = 0.0;
        for j in 0..dv {
            length += at(i, j + 1)?.distance(at(i, j)?);
        }
        long_v = long_v.max(length);
    }
    (long_u.is_finite() && long_v.is_finite()).then_some((long_u, long_v))
}

/// The Bézier pieces of a freeform surface with the parameters each
/// covers, or `None` for an analytic surface, for weights that are not
/// all positive (no hull holds the surface then), and for more pieces
/// than [`MOST_PIECES`].
fn pieces(surface: &Surface) -> Option<Vec<(Rect, RationalBezierPatch)>> {
    let spans = match surface {
        Surface::Bezier(patch) => vec![(
            (0.0, 1.0, 0.0, 1.0),
            RationalBezierPatch::from_bezier(patch),
        )],
        Surface::Nurbs(s) => {
            if s.weights().iter().any(|w| w.is_nan() || *w <= 0.0) {
                return None;
            }
            let spans = s.bezier_spans().ok()?;
            if spans.len() > MOST_PIECES {
                return None;
            }
            spans
                .into_iter()
                .map(|span| ((span.u.0, span.u.1, span.v.0, span.v.1), span.patch))
                .collect()
        }
        _ => return None,
    };
    let positive = spans.iter().all(|(_, piece)| {
        piece.is_polynomial()
            || piece
                .weight_range()
                .is_some_and(|(lightest, _)| lightest > 0.0)
    });
    positive.then_some(spans)
}

/// The children of node `n` no longer than `widest`: those not wholly
/// outside the face (perhaps none), or `None` for a leaf. Where a child's
/// numbers are not finite the node stays a leaf (nothing of the face is
/// lost).
fn divide(face: Face, n: &Node, widest: f64, spans: &Spans) -> Option<Vec<Node>> {
    let mut kids = Vec::with_capacity(2);
    for (rect, piece) in halves(face, n, widest, spans)? {
        let piece = piece.as_ref().zip(n.span);
        match node(face, rect, n.depth + 1, n.inside, piece) {
            Ok(Some(kid)) => kids.push(kid),
            Ok(None) => {}
            Err(()) => return None,
        }
    }
    Some(kids)
}

impl Front {
    /// The first nodes of `face`'s cover no longer than its box's diagonal
    /// over `parts`, or `None` where it has none.
    fn new(face: Face, parts: f64, spans: Option<&Spans>) -> Option<Front> {
        let mut stack = Vec::new();
        if let Some(spans) = spans {
            // A root per Bézier piece of the surface; pieces wholly
            // outside the face are left out.
            for (index, (rect, piece)) in spans.iter().enumerate() {
                let span = u32::try_from(index).ok()?;
                match node(face, *rect, 0, false, Some((piece, span))) {
                    Ok(Some(root)) => stack.push(root),
                    Ok(None) => {}
                    Err(()) => return None,
                }
            }
            stack.reverse();
        } else {
            let rect = parameters(face.surface, face.bbox)?;
            match node(face, rect, 0, false, None) {
                Ok(Some(root)) => stack.push(root),
                Ok(None) | Err(()) => return None,
            }
        }
        if stack.is_empty() {
            // Nothing of the surface is in the face: a face has points, so
            // this is a face its box must stand for.
            return None;
        }
        let front = face.bbox.diagonal() / parts;
        let spans = spans.unwrap_or_default();
        let mut roots = Vec::new();
        while let Some(n) = stack.pop() {
            // (Past [`FIRST_NODES`] the rest are first nodes as they are.)
            if roots.len() + stack.len() >= FIRST_NODES {
                roots.push(n);
                continue;
            }
            match divide(face, &n, front, spans) {
                Some(kids) => stack.extend(kids),
                None => roots.push(n),
            }
        }
        let tree =
            (roots.len() >= SMALL).then(|| BoxTree::new(roots.iter().map(|n| n.bbox).collect()));
        Some(Front { roots, tree })
    }

    /// The first nodes whose boxes pass `test` (a test that passes for a
    /// box whenever it passes for a box inside it), in no set order.
    fn first(&self, test: impl Fn(&Bbox3) -> bool) -> Vec<&Node> {
        match &self.tree {
            Some(tree) => {
                let (mut found, mut budget) = (Vec::new(), usize::MAX);
                tree.search(&test, &mut found, &mut budget);
                found.iter().filter_map(|&i| self.roots.get(i)).collect()
            }
            None => self.roots.iter().filter(|n| test(&n.bbox)).collect(),
        }
    }
}

impl Cover {
    /// The cover of `face`, or `None` where it has none (a freeform
    /// surface of too many pieces or of weights that are not positive,
    /// numbers that are not finite): its box then stands for it.
    pub(super) fn new(face: Face) -> Option<Cover> {
        let widest = face.bbox.diagonal() / PARTS;
        if !(widest.is_finite() && face.tol.is_finite()) {
            return None;
        }
        let spans = if is_freeform(face.surface) {
            Some(pieces(face.surface)?)
        } else {
            None
        };
        Some(Cover {
            coarse: Front::new(face, FRONT, spans.as_deref())?,
            spans,
            fine: OnceLock::new(),
            asked: AtomicUsize::new(0),
            made: AtomicUsize::new(0),
            widest,
        })
    }

    /// The first nodes to start a question from: the finer ones once the
    /// face has been asked about often.
    fn front(&self, face: Face) -> &Front {
        if self.asked.fetch_add(1, Ordering::Relaxed) < OFTEN {
            return &self.coarse;
        }
        self.fine
            .get_or_init(|| Front::new(face, FINE, self.spans.as_deref()))
            .as_ref()
            .unwrap_or(&self.coarse)
    }

    /// The children of `n`, made the first time they are asked for: `None`
    /// for a leaf.
    fn children<'n>(&self, n: &'n Node, face: Face) -> Option<&'n [Node]> {
        n.children
            .get_or_init(|| {
                if self.made.load(Ordering::Relaxed) >= NODES {
                    return None;
                }
                let kids = divide(
                    face,
                    n,
                    self.widest,
                    self.spans.as_deref().unwrap_or_default(),
                );
                if let Some(kids) = &kids {
                    self.made.fetch_add(kids.len(), Ordering::Relaxed);
                }
                kids
            })
            .as_deref()
    }

    /// True when a leaf comes within `margin` of `p`: false proves `p`
    /// further than `margin` from the face.
    pub(super) fn holds(&self, face: Face, p: Vec3, margin: f64) -> bool {
        let mut stack = self.front(face).first(|b| b.contains(p, margin));
        while let Some(n) = stack.pop() {
            if !n.bbox.contains(p, margin) {
                continue;
            }
            // (Near a patch inside the face, nothing further is proven.)
            if n.inside {
                return true;
            }
            match self.children(n, face) {
                None => return true,
                Some(kids) => stack.extend(kids.iter()),
            }
        }
        false
    }

    /// True when a leaf comes within `margin` of the box `b`.
    pub(super) fn reaches(&self, face: Face, b: &Bbox3, margin: f64) -> bool {
        let mut stack = self.front(face).first(|n| boxes_meet(n, b, margin));
        while let Some(n) = stack.pop() {
            if !boxes_meet(&n.bbox, b, margin) {
                continue;
            }
            match self.children(n, face) {
                None => return true,
                Some(kids) => stack.extend(kids.iter()),
            }
        }
        false
    }

    /// True when a leaf of `self` comes within `margin` of a leaf of
    /// `other`, or the question takes more than [`PAIRS`] pairs of nodes:
    /// false proves the two faces further apart than `margin`.
    pub(super) fn meets(&self, face: Face, other: &Cover, other_face: Face, margin: f64) -> bool {
        let mut stack = Vec::new();
        let (mine, theirs) = (self.front(face), other.front(other_face));
        for m in mine.first(|b| boxes_meet(b, other_face.bbox, margin)) {
            for n in theirs.first(|b| boxes_meet(&m.bbox, b, margin)) {
                stack.push((m, n));
            }
        }
        let mut looked = 0_usize;
        while let Some((m, n)) = stack.pop() {
            if !boxes_meet(&m.bbox, &n.bbox, margin) {
                continue;
            }
            looked += 1;
            if looked > PAIRS {
                return true;
            }
            // The larger of the two is divided (the other if it is a leaf).
            match (self.children(m, face), other.children(n, other_face)) {
                (None, None) => return true,
                (Some(mk), None) => stack.extend(mk.iter().map(|k| (k, n))),
                (None, Some(nk)) => stack.extend(nk.iter().map(|k| (m, k))),
                (Some(mk), Some(nk)) => {
                    if m.bbox.diagonal() >= n.bbox.diagonal() {
                        stack.extend(mk.iter().map(|k| (k, n)));
                    } else {
                        stack.extend(nk.iter().map(|k| (m, k)));
                    }
                }
            }
        }
        false
    }
}

/// How many stretches [`may_come_within`] halves before it gives up and
/// says the piece may come that near.
const STRETCHES: usize = 4096;

/// True unless every piece of the face's boundary is proven further than
/// `r` from `p`: the pieces whose box comes that near, each by
/// [`may_come_within`].
fn boundary_within(outline: &Outline, p: Vec3, r: f64) -> bool {
    outline.pieces_near(p, r).into_iter().any(|(l, k)| {
        outline
            .loops
            .get(l)
            .and_then(|loop_| loop_.get(k))
            .is_none_or(|seg| may_come_within(&seg.curve, seg.range, p, r))
    })
}

/// False only when the piece of `curve` over `range` is proven further
/// than `r` from `p`. A line or a circle has its nearest point in closed
/// form; any other curve is cut into stretches, each at least as far as
/// the nearer of its ends less half its length (the curve's speed bound),
/// until each is proven far or one end is that near; a curve that would
/// take more than [`STRETCHES`] is said to come near.
fn may_come_within(curve: &Curve3, range: (f64, f64), p: Vec3, r: f64) -> bool {
    // (The nearest point of a line or a circle is exact to rounding.)
    let room = r * (1.0 + 1e-9) + 1e-300;
    if matches!(curve, Curve3::Line(_) | Curve3::Circle(_)) {
        return nearest(curve, range, p).is_none_or(|(_, d)| d <= room);
    }
    let speed = speed_bound(curve);
    let (t0, t1) = range;
    if !(speed.is_finite() && t0.is_finite() && t1.is_finite() && t0 <= t1 && p.is_finite()) {
        return true;
    }
    let distance = |t: f64| curve.eval(t).distance(p);
    let (d0, d1) = (distance(t0), distance(t1));
    let mut stack = vec![(t0, d0, t1, d1)];
    let mut steps = 0_usize;
    while let Some((ta, da, tb, db)) = stack.pop() {
        if !(da > room && db > room) {
            // (Also a distance that is not a number.)
            return true;
        }
        if da.min(db) - 0.5 * speed * (tb - ta) > room {
            continue;
        }
        steps += 1;
        let tm = 0.5 * (ta + tb);
        if steps > STRETCHES || !(tm > ta && tm < tb) {
            return true;
        }
        let dm = distance(tm);
        stack.push((ta, da, tm, dm));
        stack.push((tm, dm, tb, db));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::super::section::tests::{NO_COVERS, PAIRS_MET};
    use super::super::section::{Solid, prepare};
    use super::super::{BooleanOp, BooleanResult, Operand, Vertices, boolean};
    use crate::primitives;
    use craft3d_math::{Frame, Tolerance, Transform, Vec3};
    use craft3d_topo::Body;
    use std::f64::consts::TAU;

    const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

    /// Square to the world, and turned off it.
    fn turns() -> [Transform; 2] {
        [
            Transform::scaling(1.0),
            Transform::rotation_about(Vec3::ZERO, Vec3::new(0.3, -0.5, 0.8), 0.7).unwrap(),
        ]
    }

    fn tolerance(k: f64) -> Tolerance {
        Tolerance {
            linear: 1e-6 * k,
            ..Tolerance::default()
        }
    }

    fn solve(a: &Body, b: &Body, op: BooleanOp, k: f64) -> Body {
        boolean(a, b, op, &tolerance(k)).unwrap().body
    }

    /// A tube along `axis` from `start`: a cylinder of radius `outer` less
    /// one of radius `inner`, `length` long.
    fn tube(start: Vec3, axis: Vec3, outer: f64, inner: f64, length: f64, k: f64) -> Body {
        let frame = Frame::new(start, axis, Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let hole = Frame::new(
            start - axis.normalized().unwrap() * length,
            axis,
            Vec3::new(0.0, 0.0, 1.0),
        )
        .unwrap();
        solve(
            &primitives::cylinder(&frame, outer, length).unwrap(),
            &primitives::cylinder(&hole, inner, 3.0 * length).unwrap(),
            BooleanOp::Subtract,
            k,
        )
    }

    /// Three tubes turned about an upright axis through `about` by a fifth
    /// of a turn each and united: tilted walls whose boxes all meet (the
    /// fuzzer's seed 20786729, smaller).
    fn tubes(about: Vec3, k: f64) -> Body {
        let mut all: Option<Body> = None;
        for i in 0..3 {
            let turn = Transform::rotation_about(
                about,
                Vec3::new(0.0, 0.0, 1.0),
                f64::from(i) * TAU / 5.0,
            )
            .unwrap();
            let one = tube(
                turn.point(Vec3::ZERO),
                turn.vector(Vec3::new(0.0, -1.0, 0.0)),
                10.0 * k,
                9.6875 * k,
                15.0 * k,
                k,
            );
            all = Some(match all {
                None => one,
                Some(so_far) => solve(&so_far, &one, BooleanOp::Union, k),
            });
        }
        all.unwrap()
    }

    /// Three hollow balls (radius 10, wall 1.25) around `about`, united.
    fn shells(about: Vec3, k: f64) -> Body {
        let mut all: Option<Body> = None;
        for i in 0..3 {
            let angle = f64::from(i) * TAU / 3.0;
            let centre =
                about + Vec3::new(angle.cos(), angle.sin(), 0.3 * f64::from(i)) * (7.5 * k);
            let frame =
                Frame::new(centre, Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
            let one = solve(
                &primitives::sphere(&frame, 10.0 * k).unwrap(),
                &primitives::sphere(&frame, 8.75 * k).unwrap(),
                BooleanOp::Subtract,
                k,
            );
            all = Some(match all {
                None => one,
                Some(so_far) => solve(&so_far, &one, BooleanOp::Union, k),
            });
        }
        all.unwrap()
    }

    /// A boolean's result, or its refusal, and the pairs of faces it
    /// intersected.
    type Run = (Result<BooleanResult, String>, usize);

    /// The boolean with covers and without.
    fn both(a: &Body, b: &Body, op: BooleanOp, k: f64) -> (Run, Run) {
        let run = |off: bool| {
            NO_COVERS.with(|c| c.set(off));
            PAIRS_MET.with(|n| n.set(0));
            let found = boolean(a, b, op, &tolerance(k)).map_err(|e| e.to_string());
            NO_COVERS.with(|c| c.set(false));
            (found, PAIRS_MET.with(std::cell::Cell::get))
        };
        (run(false), run(true))
    }

    /// Bodies whose faces all lie near one another, united, cut and
    /// intersected with covers and without: the same results bit for bit,
    /// with far fewer pairs of surfaces intersected, at three scales and
    /// turned.
    #[test]
    fn covers_change_nothing_but_the_pairs_intersected() {
        for k in SCALES {
            for (t, turn) in turns().iter().enumerate() {
                let place = |b: Body| b.transformed(turn).unwrap();
                for (what, a, b) in [
                    (
                        "tubes",
                        place(tubes(Vec3::new(7.5, 2.5, 0.0) * k, k)),
                        place(
                            tubes(Vec3::new(8.6, 2.6, 0.0) * k, k)
                                .transformed(
                                    &Transform::rotation_about(
                                        Vec3::new(8.6, 2.6, 0.0) * k,
                                        Vec3::new(0.0, 0.0, 1.0),
                                        0.6,
                                    )
                                    .unwrap(),
                                )
                                .unwrap(),
                        ),
                    ),
                    (
                        "shells",
                        place(shells(Vec3::ZERO, k)),
                        place(shells(Vec3::new(4.0, -3.0, 2.0) * k, k)),
                    ),
                ] {
                    for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
                        let ((with, pairs), (without, all)) = both(&a, &b, op, k);
                        let name = format!("{what} {op:?} at {k}, turn {t}");
                        match (&with, &without) {
                            (Ok(x), Ok(y)) => assert!(x == y, "{name}: the results differ"),
                            (Err(x), Err(y)) => assert_eq!(x, y, "{name}"),
                            _ => panic!("{name}: {:?} against {:?}", with.is_ok(), without.is_ok()),
                        }
                        assert!(with.is_ok(), "{name}: {:?}", with.err());
                        // (A third or less: 49 of 179 for the tubes, 184
                        // of 576 for the shells.)
                        assert!(3 * pairs <= all, "{name}: {pairs} pairs of {all}");
                    }
                }
            }
        }
    }

    /// Faces near one another by a few tolerances either side of the
    /// distance at which covers tell them apart (`APART`): two blocks a
    /// gap apart, a cylinder beside a block, a ball beside a block, square
    /// and turned, at three scales; the same results with covers and
    /// without.
    #[test]
    fn faces_near_the_margin_are_judged_as_without_covers() {
        for k in SCALES {
            let tol = 1e-6 * k;
            for turn in turns() {
                let at = |origin: Vec3| {
                    turn.frame(
                        &Frame::new(origin, Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0))
                            .unwrap(),
                    )
                    .unwrap()
                };
                let block = primitives::block(&at(Vec3::ZERO), 2.0 * k, 2.0 * k, 2.0 * k).unwrap();
                for gap in [-3.0, 0.0, 0.25, 1.0, 3.0, 15.0, 16.0, 17.0, 40.0] {
                    let x = 2.0 * k + gap * tol;
                    let others = [
                        primitives::block(
                            &at(Vec3::new(x, 0.5 * k, 0.5 * k)),
                            2.0 * k,
                            2.0 * k,
                            2.0 * k,
                        )
                        .unwrap(),
                        primitives::cylinder(&at(Vec3::new(x + k, 1.0 * k, -k)), k, 4.0 * k)
                            .unwrap(),
                        primitives::sphere(&at(Vec3::new(x + k, 1.0 * k, k)), k).unwrap(),
                    ];
                    for other in &others {
                        for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
                            let ((with, _), (without, _)) = both(&block, other, op, k);
                            match (&with, &without) {
                                (Ok(x), Ok(y)) => {
                                    assert!(x == y, "gap {gap} at {k}: the results differ")
                                }
                                (Err(x), Err(y)) => assert_eq!(x, y, "gap {gap} at {k}"),
                                _ => panic!(
                                    "gap {gap} at {k}: {with:?} against {without:?}",
                                    with = with.is_ok(),
                                    without = without.is_ok()
                                ),
                            }
                        }
                    }
                }
            }
        }
    }

    fn solid(body: &Body, k: f64) -> Solid<'_> {
        let mut vertices = Vertices::new(0.5e-6 * k, 2e-6 * k);
        prepare(body, Operand::A, &mut vertices, &tolerance(k), true).unwrap()
    }

    /// Every point of every face is in the face's cover: points inside the
    /// faces, and along their edges, of primitives and of bodies made by
    /// booleans (trimmed faces of tilted tubes and of hollow balls), at
    /// three scales and turned.
    #[test]
    fn every_point_of_a_face_is_in_its_cover() {
        for k in SCALES {
            let tol = 1e-6 * k;
            for turn in turns() {
                let frame = turn
                    .frame(
                        &Frame::new(
                            Vec3::new(0.3, -0.2, 0.1) * k,
                            Vec3::new(0.2, 0.1, 1.0),
                            Vec3::new(1.0, 0.0, 0.0),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                let bodies = [
                    primitives::block(&frame, 2.0 * k, 3.0 * k, 4.0 * k).unwrap(),
                    primitives::cylinder(&frame, k, 3.0 * k).unwrap(),
                    primitives::cone(&frame, k, 2.0 * k).unwrap(),
                    primitives::sphere(&frame, 2.0 * k).unwrap(),
                    primitives::torus(&frame, 3.0 * k, k).unwrap(),
                    tubes(Vec3::new(7.5, 2.5, 0.0) * k, k)
                        .transformed(&turn)
                        .unwrap(),
                    shells(Vec3::ZERO, k).transformed(&turn).unwrap(),
                ];
                for body in &bodies {
                    let solid = solid(body, k);
                    for face in 0..body.faces.len() {
                        let Some((cover, of)) = solid.cover(face, tol) else {
                            panic!("an analytic face has a cover");
                        };
                        let outline = solid.outline(face).unwrap();
                        let mut points = outline.interior_points(tol, 4.0 * tol, 24).unwrap();
                        for seg in outline.loops.iter().flatten() {
                            points.extend((0..=8).map(|i| seg.at(f64::from(i) / 8.0)));
                        }
                        for p in points {
                            assert!(cover.holds(of, p, 2.0 * tol), "face {face} at {k}: {p:?}");
                        }
                    }
                }
            }
        }
    }

    /// A point on a face's surface far from the face is outside the
    /// cover: half a cylinder (the other half cut away) and its far side,
    /// at three scales and turned.
    #[test]
    fn a_point_far_from_a_face_is_outside_its_cover() {
        for k in SCALES {
            let tol = 1e-6 * k;
            for turn in turns() {
                let at = |origin: Vec3| {
                    turn.frame(
                        &Frame::new(origin, Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0))
                            .unwrap(),
                    )
                    .unwrap()
                };
                let half = solve(
                    &primitives::cylinder(&at(Vec3::ZERO), k, 2.0 * k).unwrap(),
                    &primitives::block(
                        &at(Vec3::new(-2.0 * k, -2.0 * k, -k)),
                        2.0 * k,
                        4.0 * k,
                        4.0 * k,
                    )
                    .unwrap(),
                    BooleanOp::Subtract,
                    k,
                );
                let solid = solid(&half, k);
                let wall = (0..half.faces.len())
                    .find(|&f| matches!(half.faces[f].surface, craft3d_geom::Surface::Cylinder(_)))
                    .unwrap();
                let (cover, of) = solid.cover(wall, tol).unwrap();
                // On the wall that was cut away, at mid height.
                let away = turn.point(Vec3::new(-k, 0.0, k));
                assert!(!cover.holds(of, away, 2.0 * tol), "at {k}");
                // And on the wall that is there.
                let there = turn.point(Vec3::new(k, 0.0, k));
                assert!(cover.holds(of, there, 2.0 * tol), "at {k}");
            }
        }
    }

    /// A block with a freeform roof (a bicubic Bézier patch, or the same
    /// as a NURBS surface), at scale `k` in `frame`.
    fn domed(frame: &Frame, k: f64, nurbs: bool) -> Body {
        craft3d_testkit::trimmed::domed_block(frame, 4.0 * k, 3.0 * k, 2.0 * k, nurbs)
            .unwrap()
            .body
    }

    fn freeform(body: &Body, face: usize) -> bool {
        matches!(
            body.faces[face].surface,
            craft3d_geom::Surface::Bezier(_) | craft3d_geom::Surface::Nurbs(_)
        )
    }

    /// Every point of a freeform face is in its cover: the roof of a
    /// domed block (Bézier and NURBS), the same roof with a round hole
    /// through it and with a ball sunk into it (trimmed faces), points
    /// inside the faces and along their edges, at three scales and
    /// turned; and a point of the roof's surface in the middle of the
    /// hole, half the hole's radius from the face, is outside the cover.
    #[test]
    fn every_point_of_a_freeform_face_is_in_its_cover() {
        for k in SCALES {
            let tol = 1e-6 * k;
            for turn in turns() {
                let frame = turn
                    .frame(
                        &Frame::new(
                            Vec3::new(0.3, -0.2, 0.1) * k,
                            Vec3::new(0.0, 0.0, 1.0),
                            Vec3::new(1.0, 0.0, 0.0),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                for nurbs in [false, true] {
                    let roof = domed(&frame, k, nurbs);
                    let drill = turn
                        .frame(
                            &Frame::new(
                                Vec3::new(2.3, 1.2, -1.0) * k,
                                Vec3::new(0.0, 0.0, 1.0),
                                Vec3::new(1.0, 0.0, 0.0),
                            )
                            .unwrap(),
                        )
                        .unwrap();
                    let holed = solve(
                        &roof,
                        &primitives::cylinder(&drill, 0.5 * k, 6.0 * k).unwrap(),
                        BooleanOp::Subtract,
                        k,
                    );
                    let ball = turn
                        .frame(
                            &Frame::new(
                                Vec3::new(1.2, 1.8, 2.1) * k,
                                Vec3::new(0.0, 0.0, 1.0),
                                Vec3::new(1.0, 0.0, 0.0),
                            )
                            .unwrap(),
                        )
                        .unwrap();
                    let sunk = solve(
                        &roof,
                        &primitives::sphere(&ball, 0.6 * k).unwrap(),
                        BooleanOp::Subtract,
                        k,
                    );
                    for body in [&roof, &holed, &sunk] {
                        let solid = solid(body, k);
                        let mut asked = 0;
                        for face in (0..body.faces.len()).filter(|&f| freeform(body, f)) {
                            let Some((cover, of)) = solid.cover(face, tol) else {
                                panic!("a freeform face has a cover");
                            };
                            asked += 1;
                            let outline = solid.outline(face).unwrap();
                            let mut points = outline.interior_points(tol, 4.0 * tol, 24).unwrap();
                            for seg in outline.loops.iter().flatten() {
                                points.extend((0..=8).map(|i| seg.at(f64::from(i) / 8.0)));
                            }
                            for p in points {
                                assert!(cover.holds(of, p, 2.0 * tol), "face {face} at {k}: {p:?}");
                            }
                        }
                        assert_eq!(asked, 1);
                    }
                    // The roof's surface over the middle of the hole.
                    let solid = solid(&holed, k);
                    let face = (0..holed.faces.len())
                        .find(|&f| freeform(&holed, f))
                        .unwrap();
                    let (cover, of) = solid.cover(face, tol).unwrap();
                    // (The drill's axis is at x 2, y 1.4 of the roof's
                    // 4 by 3: half way along u, 1.4 / 3 along v.)
                    let surface = &holed.faces[face].surface;
                    let ((u0, u1), (v0, v1)) = surface.domain();
                    let on = surface.eval(u0 + 0.5 * (u1 - u0), v0 + (1.4 / 3.0) * (v1 - v0));
                    assert!(!cover.holds(of, on, 2.0 * tol), "at {k}");
                }
            }
        }
    }

    /// Freeform faces with covers and without: a domed block with a block
    /// inside it under the roof (the roof's box holds the block's top and
    /// sides, its cover does not come near them), with a ball sunk into
    /// the roof, and with a second domed block moved and turned so that
    /// the roofs cross; united, cut and intersected, Bézier and NURBS
    /// roofs, at three scales and turned: the same results bit for bit,
    /// and fewer pairs intersected beside the inner block.
    #[test]
    fn freeform_covers_change_nothing_but_the_pairs_intersected() {
        for k in SCALES {
            for (t, turn) in turns().iter().enumerate() {
                let at = |origin: Vec3| {
                    turn.frame(
                        &Frame::new(
                            origin * k,
                            Vec3::new(0.0, 0.0, 1.0),
                            Vec3::new(1.0, 0.0, 0.0),
                        )
                        .unwrap(),
                    )
                    .unwrap()
                };
                for nurbs in [false, true] {
                    let roof = domed(&at(Vec3::ZERO), k, nurbs);
                    let inner =
                        primitives::block(&at(Vec3::new(1.5, 1.0, 0.5)), k, k, 0.95 * k).unwrap();
                    let ball = primitives::sphere(&at(Vec3::new(1.2, 1.8, 2.1)), 0.6 * k).unwrap();
                    let other = domed(
                        &turn
                            .frame(
                                &Frame::new(
                                    Vec3::new(1.1, 0.7, 0.3) * k,
                                    Vec3::new(0.05, -0.03, 1.0),
                                    Vec3::new(1.0, 0.2, 0.0),
                                )
                                .unwrap(),
                            )
                            .unwrap(),
                        k,
                        !nurbs,
                    );
                    for (what, b) in [("inner", &inner), ("ball", &ball), ("roofs", &other)] {
                        for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
                            let ((with, pairs), (without, all)) = both(&roof, b, op, k);
                            let name = format!("{what} {op:?} at {k}, turn {t}, nurbs {nurbs}");
                            match (&with, &without) {
                                (Ok(x), Ok(y)) => assert!(x == y, "{name}: the results differ"),
                                (Err(x), Err(y)) => assert_eq!(x, y, "{name}"),
                                _ => panic!(
                                    "{name}: {:?} against {:?}",
                                    with.is_ok(),
                                    without.is_ok()
                                ),
                            }
                            assert!(pairs <= all, "{name}");
                            if what == "inner" {
                                assert!(pairs < all, "{name}: {pairs} pairs of {all}");
                            }
                        }
                    }
                }
            }
        }
    }
}
