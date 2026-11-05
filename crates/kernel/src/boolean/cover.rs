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
//! Where two covers are asked whether they meet, the other face's
//! surface is known exactly where its box is not: a node further than the
//! margin from the other face's surface (by its signed distance, which
//! changes by no more than the distance moved, so at the box's middle it
//! must exceed half the box's diagonal) is that far from the other face,
//! which lies on it, and is passed over, for analytic surfaces. Two
//! leaves that come within the margin of each other are each looked at in
//! [`PARTS_OF_LEAF`]² parts, worked out and let go (the first [`PARTED`]
//! pairs of leaves of a question), each part passed over the same way.
//! These only prove more pairs further apart than the margin, which is
//! what passing a pair over rests on.
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
use super::hits::{ROUNDING, signed_distance, speed_bound, surface_size};
use super::outline::{Outline, Sheet, Side, boxes_meet, nearest};
use crate::boxtree::BoxTree;
use craft3d_geom::{Curve3, RationalBezierPatch, Surface};
use craft3d_math::Portable;
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

/// How many parts along each parameter a leaf is cut into when two
/// leaves come within the margin of each other ([`Cover::meets`]).
const PARTS_OF_LEAF: usize = 4;

/// How many pairs of leaves one question looks at in parts, at most;
/// past that a pair of leaves that meet answers that the faces may.
const PARTED: usize = 8;

/// A freeform face of more Bézier pieces than this has no cover (its box
/// stands for it).
const MOST_PIECES: usize = 4096;

/// How many first nodes a front holds at most: past that, nodes are
/// first nodes undivided (a coarser front, never a smaller cover). An
/// analytic face's front stays below it (about `(FINE · π)²`); a freeform
/// face's has a node per knot span at least.
const FIRST_NODES: usize = 1 << 12;

/// How far, relative to the largest coordinate, the control points of a
/// piece may have been moved by rounding, per unit of the spread of its
/// weights: a NURBS surface's pieces come from knot insertions and are
/// halved down to [`DEPTH`] times (with the cut-outs, some sixty
/// de Casteljau steps of a few roundings each, in homogeneous
/// coordinates, where every step is a convex combination); a control
/// point is then a homogeneous point over its weight, whose rounding
/// relative to the largest coordinate grows with the heaviest weight over
/// the lightest. Some 10⁻¹⁴ times that spread in all, and 10⁻¹² leaves
/// room. The box only grows by it, so a larger spread costs a looser
/// cover, never a point of the face; it is no margin of the boolean's
/// (those are tolerances), and with weights a million apart and
/// coordinates 10⁴ tolerances large it is still below a tolerance.
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
            let slope = c.half_angle().ptan();
            let r = (c.radius() + v.0 * slope)
                .abs()
                .max((c.radius() + v.1 * slope).abs());
            [r, slope.phypot(1.0), r, 0.0]
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
            let spread = match piece.weight_range() {
                Some((lightest, heaviest)) if lightest > 0.0 => (heaviest / lightest).max(1.0),
                _ if piece.is_polynomial() => 1.0,
                _ => return Err(()),
            };
            (hull, PIECE_ROUNDING * size * spread)
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
    /// How often it was asked, and how many nodes it made below its first
    /// ones: what its next answers depend on besides the question.
    #[cfg(test)]
    pub(super) fn history(&self) -> (usize, usize) {
        (
            self.asked.load(Ordering::Relaxed),
            self.made.load(Ordering::Relaxed),
        )
    }

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

    /// True when node `n`'s patch, cut [`PARTS_OF_LEAF`] ways along each
    /// parameter, has every part proven further than `margin` from the box
    /// `b` or from the surface `beside` (of the face `b` is a box of):
    /// then so is the patch. The parts are worked out and let go (a
    /// leaf's box is as fine as a cover keeps; its parts are finer where
    /// a pair of leaves needs it).
    fn clear_in_parts(
        &self,
        face: Face,
        n: &Node,
        b: &Bbox3,
        beside: &Surface,
        margin: f64,
    ) -> bool {
        let spans = self.spans.as_deref().unwrap_or_default();
        let (a0, a1, c0, c1) = n.rect;
        let k = PARTS_OF_LEAF as f64;
        for i in 0..PARTS_OF_LEAF {
            for j in 0..PARTS_OF_LEAF {
                let at = |from: f64, to: f64, x: usize| from + (to - from) * x as f64 / k;
                let rect = (
                    at(a0, a1, i),
                    at(a0, a1, i + 1),
                    at(c0, c1, j),
                    at(c0, c1, j + 1),
                );
                let piece = match n.span {
                    Some(span) => match piece_of(spans, span, rect) {
                        Some(piece) => Some(piece),
                        None => return false,
                    },
                    None => None,
                };
                // (Taken as inside: the outline is not asked about parts.)
                match node(face, rect, n.depth + 1, true, piece.as_ref().zip(n.span)) {
                    Ok(None) => {}
                    Ok(Some(part)) => {
                        if boxes_meet(&part.bbox, b, margin)
                            && !clear_of_surface(beside, &part.bbox, margin)
                        {
                            return false;
                        }
                    }
                    Err(()) => return false,
                }
            }
        }
        true
    }

    /// True when a leaf comes within `margin` of the box `b` and, when
    /// `beside` is given, of that surface (the surface of what `b` is the
    /// box of).
    pub(super) fn reaches(
        &self,
        face: Face,
        b: &Bbox3,
        beside: Option<&Surface>,
        margin: f64,
    ) -> bool {
        let mut stack = self.front(face).first(|n| boxes_meet(n, b, margin));
        while let Some(n) = stack.pop() {
            if !boxes_meet(&n.bbox, b, margin) {
                continue;
            }
            if beside.is_some_and(|surface| clear_of_surface(surface, &n.bbox, margin)) {
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
        let (mut looked, mut parted) = (0_usize, 0_usize);
        while let Some((m, n)) = stack.pop() {
            if !boxes_meet(&m.bbox, &n.bbox, margin) {
                continue;
            }
            looked += 1;
            if looked > PAIRS {
                return true;
            }
            // A node further than the margin from the other face's
            // surface is that far from the other face, which lies on it:
            // the surface is known exactly where the other's box is not.
            if clear_of_surface(other_face.surface, &m.bbox, margin)
                || clear_of_surface(face.surface, &n.bbox, margin)
            {
                continue;
            }
            // The larger of the two is divided (the other if it is a leaf).
            match (self.children(m, face), other.children(n, other_face)) {
                (None, None) => {
                    // Two leaves: each looked at in parts, not kept (for
                    // the first few pairs of leaves of a question).
                    parted += 1;
                    if parted <= PARTED
                        && (self.clear_in_parts(face, m, &n.bbox, other_face.surface, margin)
                            || other.clear_in_parts(other_face, n, &m.bbox, face.surface, margin))
                    {
                        continue;
                    }
                    return true;
                }
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

/// True when every point of the box `b` is proven further than `margin`
/// from the analytic surface: its signed distance changes by at most the
/// distance moved and is, in size, no more than the distance to the
/// surface (for a cone, to the cone continued past its apex), so it is
/// more than half the box's diagonal and the margin at its middle, with
/// room for rounding ([`ROUNDING`] units in the last place of the largest
/// number, as for the closed forms of `hits.rs`). False for a freeform
/// surface (its distance is only as near as a search gets).
pub(super) fn clear_of_surface(surface: &Surface, b: &Bbox3, margin: f64) -> bool {
    if is_freeform(surface) {
        return false;
    }
    let Some((lo, hi)) = b.corners() else {
        return false;
    };
    let middle = lo.lerp(hi, 0.5);
    let Some(d) = signed_distance(Sheet::bare(surface), middle) else {
        return false;
    };
    let rounding = ROUNDING * f64::EPSILON * (middle.length() + surface_size(surface));
    d.abs() > 0.5 * lo.distance(hi) + margin + rounding
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
    use craft3d_math::Portable;
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
                about + Vec3::new(angle.pcos(), angle.psin(), 0.3 * f64::from(i)) * (7.5 * k);
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

    /// A box called clear of a surface by its signed distance at the middle
    /// has every point (corners, middles of faces, random points) further
    /// than the margin from the surface, for every analytic kind, boxes of
    /// many sizes about a turned frame, at three scales.
    #[test]
    fn a_box_clear_of_a_surface_is_clear_everywhere() {
        use super::{Sheet, clear_of_surface, signed_distance};
        use craft3d_geom::{Cone, Cylinder, Plane, Sphere, Surface, Torus};
        use craft3d_math::Bbox3;
        let mut state = 41_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            ((state >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        };
        let mut cleared = 0;
        for k in SCALES {
            let tol = 1e-6 * k;
            let f = Frame::new(
                Vec3::new(1.0, -2.0, 0.5) * k,
                Vec3::new(0.3, -0.4, 1.0),
                Vec3::new(1.0, 0.2, 0.0),
            )
            .unwrap();
            let surfaces = [
                Surface::Plane(Plane::new(f)),
                Surface::Cylinder(Cylinder::new(f, 2.0 * k).unwrap()),
                Surface::Cone(Cone::new(f, 1.5 * k, 0.4).unwrap()),
                Surface::Sphere(Sphere::new(f, 3.0 * k).unwrap()),
                Surface::Torus(Torus::new(f, 4.0 * k, k).unwrap()),
            ];
            for surface in &surfaces {
                for _ in 0..2000 {
                    let middle = f.point(6.0 * k * next(), 6.0 * k * next(), 6.0 * k * next());
                    // Half the time a box about as large as its middle is
                    // far from the surface (where a bound too tight fails).
                    let size = if next() > 0.0 {
                        let far = signed_distance(Sheet::bare(surface), middle).unwrap().abs();
                        far * (1.0 + 0.6 * next())
                    } else {
                        k * 10f64.ppowf(-3.0 * next().abs()) * 2.0
                    };
                    let way = Vec3::new(next().abs(), next().abs(), next().abs());
                    let half = way * (size / way.length().max(1e-3));
                    let b = Bbox3::from_points([middle - half, middle + half]);
                    let margin = 16.0 * tol;
                    if !clear_of_surface(surface, &b, margin) {
                        continue;
                    }
                    cleared += 1;
                    let corners = (0..8).map(|c| {
                        let side = |bit: i32| if c & bit == 0 { -1.0 } else { 1.0 };
                        Vec3::new(side(1), side(2), side(4))
                    });
                    let inside: Vec<Vec3> =
                        (0..16).map(|_| Vec3::new(next(), next(), next())).collect();
                    for shares in corners.chain(inside) {
                        let p = middle
                            + Vec3::new(half.x * shares.x, half.y * shares.y, half.z * shares.z);
                        let d = signed_distance(Sheet::bare(surface), p).unwrap();
                        assert!(d.abs() > margin, "{} at {k}: {d}", surface.kind());
                    }
                }
            }
        }
        assert!(cleared > 10_000, "{cleared}");
    }

    /// A box called clear of a surface holds no point of the surface
    /// within the margin: points sampled on the surface itself (a grid of
    /// 192 × 192 parameters over the part of it near the boxes), for cones
    /// that widen either way, a cone of radius 0 at `v = 0` with boxes
    /// about its apex, tori with a thick tube (major 1, minor 0.99) and
    /// boxes on and about their axis, balls and cylinders, at three
    /// scales.
    #[test]
    fn a_box_clear_of_a_surface_holds_no_point_of_it() {
        use super::clear_of_surface;
        use craft3d_geom::{Cone, Cylinder, Sphere, Surface, Torus};
        use craft3d_math::Bbox3;
        let mut state = 7_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            ((state >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        };
        let mut cleared = 0;
        for k in SCALES {
            let tol = 1e-6 * k;
            let margin = 16.0 * tol;
            let f = Frame::new(
                Vec3::new(1.0, -2.0, 0.5) * k,
                Vec3::new(0.3, -0.4, 1.0),
                Vec3::new(1.0, 0.2, 0.0),
            )
            .unwrap();
            let surfaces = [
                Surface::Cone(Cone::new(f, 1.5 * k, 0.4).unwrap()),
                Surface::Cone(Cone::new(f, 1.5 * k, -0.7).unwrap()),
                Surface::Cone(Cone::new(f, 0.0, 0.5).unwrap()),
                Surface::Cone(Cone::new(f, 0.0, -0.3).unwrap()),
                Surface::Torus(Torus::new(f, k, 0.99 * k).unwrap()),
                Surface::Torus(Torus::new(f, 4.0 * k, k).unwrap()),
                Surface::Sphere(Sphere::new(f, 3.0 * k).unwrap()),
                Surface::Cylinder(Cylinder::new(f, 2.0 * k).unwrap()),
            ];
            for surface in &surfaces {
                // The surface's points: around, and along it within 6 of
                // the frame's origin (all of a torus or a ball).
                let ((u0, u1), (v0, v1)) = surface.domain();
                let (v0, v1) = match surface {
                    Surface::Cone(_) | Surface::Cylinder(_) => (v0.max(-6.0 * k), v1.min(6.0 * k)),
                    _ => (v0, v1),
                };
                let n = 192;
                let points: Vec<Vec3> = (0..=n)
                    .flat_map(|i| {
                        (0..=n).map(move |j| {
                            let s = |a: f64, b: f64, x: i32| a + (b - a) * f64::from(x) / 192.0;
                            surface.eval(s(u0, u1, i), s(v0, v1, j))
                        })
                    })
                    .collect();
                for b in 0..400 {
                    // Half the boxes about the frame's axis (a cone's apex
                    // where its radius is 0, a torus's hole), the others
                    // anywhere near.
                    let middle = if b % 2 == 0 {
                        f.point(0.2 * k * next(), 0.2 * k * next(), 3.0 * k * next())
                    } else {
                        f.point(6.0 * k * next(), 6.0 * k * next(), 6.0 * k * next())
                    };
                    let size = k * 10f64.ppowf(-2.0 * next().abs());
                    let half = Vec3::new(next().abs(), next().abs(), next().abs()) * size;
                    let bbox = Bbox3::from_points([middle - half, middle + half]);
                    if !clear_of_surface(surface, &bbox, margin) {
                        continue;
                    }
                    cleared += 1;
                    for p in &points {
                        assert!(
                            !bbox.contains(*p, margin),
                            "{} at {k}: {p:?}",
                            surface.kind()
                        );
                    }
                }
            }
        }
        assert!(cleared > 2000, "{cleared}");
    }
}
