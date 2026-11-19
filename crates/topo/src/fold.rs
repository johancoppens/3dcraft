//! A face folded over itself (docs/design/verification.md › *A face
//! folded over itself*): two points of a face within the document's
//! tolerance `t` of each other whose outward normals stand more than 154°
//! apart (cosine below −0.9), each on the other's outward side (air
//! between them), both in the face and outside the tip balls (four
//! tolerances, or a vertex's own, about the face's vertices and its
//! surface's poles); or a point of it without a normal. For now a warning
//! (`document.check` says each face that folds); [`FOLDS_REFUSED`] is
//! where it would become a defect of [`crate::check`].
//!
//! Per freeform face:
//!
//! 1. **Whether to look.** A face is searched when a Bézier span of its
//!    surface is not proven to have a regular offset by `t / 2` out of the
//!    material (`craft3d_geom::offset::regular_by_span`: a concave bend
//!    tighter than `t / 2`, no normal, not proven, degrees too high for
//!    the forms), or, on a surface of degree one, its normal turns by more
//!    than a right angle across a knot line. A face with none is passed.
//! 2. **No normal.** About a span the forms find no normal in, a pattern
//!    search on `|S_u × S_v|` for a point where it is below 1e-12 of the
//!    span's own: shown there when that point is in the face.
//! 3. **Two layers through air, by their nearness in space.** The face's
//!    surface in pieces: its spans in a tree by their boxes, each span
//!    halved as the search needs. A piece carries the box of its control
//!    points and the cone of its outward normals (from the hull of the net
//!    of `w³ (S_u × S_v)`), both holding every point of it. Two pieces are
//!    compared only while they can hold such a pair: their control points
//!    less than `t` apart along the boxes' axes and the cones' axes, normals
//!    that can stand more than 154° apart, and neither wholly behind the
//!    other's outward side. Pairs go nearest first; each pair of patches
//!    is refined to the nearest two points of its two layers (projections
//!    back and forth from their middles), and a refined pair that meets the
//!    definition is told in or out of the face. Pairs that cannot come
//!    nearer than the nearest fold found, less 1 % of `t`, are let go: the
//!    gap said is the least within 1 % of `t`. Its work grows with the
//!    parts of the face near one another with opposed normals, not with
//!    the face's size; past [`MOST_PAIRS`] pairs a face is said too large
//!    to examine.
//! 4. **In the face**, also on a surface with a pole: the loops followed in
//!    the surface's parameters, each edge in steps about a tolerance long
//!    (16 to 1024 of them), each point projected from the one before; a
//!    point at a pole takes the pole's border parameter and its
//!    neighbour's other one. A point is in where the loops wind round it
//!    once and it is farther than twice its chord's error from every
//!    chord; near; out; not told where the loops cannot be followed. Only
//!    a pair that meets the definition is told.
//!
//! Said, one line a face: a fold (certain), at the point of the face where
//! its layers come nearest; else the pairs that meet the definition only
//! near an edge or where the loops cannot be followed, or a search past
//! its work: not proven. Faces are remembered by everything the rule reads
//! (the surface, its sense, the tolerance, the loops' edges and the
//! vertices' own tolerances), and each surface's spans by the surface, its
//! sense and the tolerance.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::{Body, Face, FaceId};
use craft3d_geom::offset::{SpanRegularity, SpanVerdict, regular_by_span};
use craft3d_geom::{BezierSpan, NurbsSurface, RationalBezierPatch, Surface};
use craft3d_math::{Bits, Portable, Tolerance, Vec3};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::f64::consts::{FRAC_PI_2, PI};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// Whether [`crate::check`] refuses a body with a fold
/// ([`crate::Defect::FaceFolds`]): not yet. The rule is a warning of
/// `document.check` for one round first (the developer, 2026-11-18).
pub const FOLDS_REFUSED: bool = false;

/// The tip balls' radius, in tolerances.
const TIP: f64 = 4.0;
/// The cosine below which two normals stand against each other (154°).
const OPPOSITE: f64 = -0.9;
/// How near the gap said is to the least, as a share of the tolerance.
const EXACT: f64 = 0.01;
/// A piece is not halved once its box is this share of the tolerance
/// across, nor past [`DEEPEST`] halvings of its span.
const LEAF: f64 = 0.125;
const DEEPEST: u32 = 48;
/// Rounds of projections back and forth that refine a pair.
const ROUNDS: usize = 8;
/// Steps along each edge of the walk: about one a tolerance, within these.
const FEWEST_STEPS: usize = 16;
const MOST_STEPS: usize = 1024;
/// The work of a face: pairs of pieces compared, no-normal searches,
/// spans asked.
pub const MOST_PAIRS: u64 = 1 << 16;
const MOST_SEARCHES: usize = 32;
const MOST_SPANS: usize = 1 << 16;
/// What is remembered: faces and surfaces, and words of keys in all.
const REMEMBERED: usize = 4096;
const REMEMBERED_WORDS: usize = 1 << 22;

/// Why a place is not proven: pairs that meet the definition only at an
/// edge, or where the loops cannot be followed.
const AT_AN_EDGE: &str = "two of its layers face each other within the tolerance at an edge of the face, or where its loops cannot be followed";
/// Why a face is not proven: the search past its work.
const TOO_LARGE: &str =
    "too large to examine: 65 536 pairs of pieces of its surface compared and more left";

/// What a place found is.
#[derive(Clone, Debug, PartialEq)]
pub enum FoldKind {
    /// Two layers of the face `gap` apart through air.
    Layers { gap: f64 },
    /// No normal there.
    NoNormal,
    /// Neither shown nor ruled out there.
    NotProven { why: &'static str },
}

/// A place of a face the rule speaks of.
#[derive(Clone, Debug, PartialEq)]
pub struct Fold {
    pub face: FaceId,
    pub at: Vec3,
    pub kind: FoldKind,
    /// The tolerance it was asked at.
    pub tolerance: f64,
}

impl Fold {
    /// A fold shown (not a place left not proven).
    pub fn is_fold(&self) -> bool {
        !matches!(self.kind, FoldKind::NotProven { .. })
    }

    /// `fold` or `foldNotProven`, for the lists that say them.
    pub fn kind_name(&self) -> &'static str {
        if self.is_fold() {
            "fold"
        } else {
            "foldNotProven"
        }
    }
}

impl fmt::Display for Fold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (x, y, z) = (self.at.x, self.at.y, self.at.z);
        match &self.kind {
            FoldKind::Layers { gap } => write!(
                f,
                "face {} folds over itself at ({x}, {y}, {z}): two of its layers face each other {gap:e} apart with air between them, within the tolerance {:e}",
                self.face.0, self.tolerance
            ),
            FoldKind::NoNormal => write!(
                f,
                "face {} folds over itself at ({x}, {y}, {z}): its surface has no normal there",
                self.face.0
            ),
            FoldKind::NotProven { why } => write!(
                f,
                "face {} could not be shown free of folds at ({x}, {y}, {z}): {why}",
                self.face.0
            ),
        }
    }
}

static SPANS: AtomicU64 = AtomicU64::new(0);
static SEARCHES: AtomicU64 = AtomicU64::new(0);

/// The spans asked and the pairs of pieces compared since the process
/// started (each face or surface once; a remembered one is not counted
/// again): counts, not a clock.
pub fn fold_work() -> (u64, u64) {
    (
        SPANS.load(Ordering::Relaxed),
        SEARCHES.load(Ordering::Relaxed),
    )
}

/// What the search for two layers did ([`folds_examined`]): for tests and
/// probes.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldWork {
    /// Faces searched for two layers.
    pub faces: u64,
    /// Pairs of pieces compared, in all and on the face that took most.
    pub pairs: u64,
    pub most_pairs: u64,
    /// Faces whose search reached its bound.
    pub spent: u64,
    /// Pairs refined that meet the definition, by where their points lie:
    /// both in the face, one near an edge, one not told, one out.
    pub in_in: u64,
    pub near: u64,
    pub not_told: u64,
    pub out: u64,
    /// The least gap of such a pair in the face, and of one near an edge
    /// or not told.
    pub least_in_in: f64,
    pub least_doubtful: f64,
}

impl Default for FoldWork {
    fn default() -> Self {
        Self {
            faces: 0,
            pairs: 0,
            most_pairs: 0,
            spent: 0,
            in_in: 0,
            near: 0,
            not_told: 0,
            out: 0,
            least_in_in: f64::INFINITY,
            least_doubtful: f64::INFINITY,
        }
    }
}

type Found = Vec<(Vec3, FoldKind)>;
type Spans = Arc<Result<Vec<SpanVerdict>, &'static str>>;
type Rect = ((f64, f64), (f64, f64));

static FACES: Mutex<Vec<(Vec<u64>, Found)>> = Mutex::new(Vec::new());
static SURFACES: Mutex<Vec<(Vec<u64>, Spans)>> = Mutex::new(Vec::new());

/// Lets go of the faces and surfaces remembered; says how many faces.
pub fn forget_folds() -> usize {
    std::mem::take(&mut *SURFACES.lock().unwrap_or_else(PoisonError::into_inner));
    std::mem::take(&mut *FACES.lock().unwrap_or_else(PoisonError::into_inner)).len()
}

fn keep<T: Clone>(store: &Mutex<Vec<(Vec<u64>, T)>>, key: Vec<u64>, value: T) {
    if key.len() > REMEMBERED_WORDS {
        return;
    }
    let mut kept = store.lock().unwrap_or_else(PoisonError::into_inner);
    if kept.iter().any(|(k, _)| *k == key) {
        return;
    }
    kept.push((key, value));
    while kept.len() > REMEMBERED
        || kept.iter().map(|(k, _)| k.len()).sum::<usize>() > REMEMBERED_WORDS
    {
        if kept.is_empty() {
            break;
        }
        let _oldest = kept.remove(0);
    }
}

fn kept<T: Clone>(store: &Mutex<Vec<(Vec<u64>, T)>>, key: &[u64]) -> Option<T> {
    let kept = store.lock().unwrap_or_else(PoisonError::into_inner);
    kept.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

fn freeform(face: &Face) -> bool {
    matches!(face.surface, Surface::Nurbs(_) | Surface::Bezier(_))
}

fn face_id(index: usize) -> FaceId {
    FaceId(u32::try_from(index).unwrap_or(u32::MAX))
}

/// Every place of `body`'s freeform faces the rule speaks of, at the
/// tolerance `tol.linear`, faces in order.
pub fn folds(body: &Body, tol: &Tolerance) -> Vec<Fold> {
    let t = tol.linear;
    let mut out = Vec::new();
    if !(t.is_finite() && t > 0.0) {
        return out;
    }
    for (index, face) in body.faces.iter().enumerate() {
        if !freeform(face) {
            continue;
        }
        let key = face_key(body, face, t);
        let found = match kept(&FACES, &key) {
            Some(found) => found,
            None => {
                let mut work = FoldWork::default();
                let found = face_folds(body, face, t, MOST_PAIRS, &mut work);
                SEARCHES.fetch_add(work.pairs, Ordering::Relaxed);
                keep(&FACES, key, found.clone());
                found
            }
        };
        out.extend(found.into_iter().map(|(at, kind)| Fold {
            face: face_id(index),
            at,
            kind,
            tolerance: t,
        }));
    }
    out
}

/// [`folds`] without the store of faces, with `most` pairs of pieces a
/// face in place of [`MOST_PAIRS`], and what the search did: for tests
/// and probes.
pub fn folds_examined(body: &Body, tol: &Tolerance, most: u64) -> (Vec<Fold>, FoldWork) {
    let t = tol.linear;
    let mut work = FoldWork::default();
    let mut out = Vec::new();
    if !(t.is_finite() && t > 0.0) {
        return (out, work);
    }
    for (index, face) in body.faces.iter().enumerate() {
        if !freeform(face) {
            continue;
        }
        let found = face_folds(body, face, t, most, &mut work);
        out.extend(found.into_iter().map(|(at, kind)| Fold {
            face: face_id(index),
            at,
            kind,
            tolerance: t,
        }));
    }
    (out, work)
}

/// Everything the rule reads of a face, every bit.
fn face_key(body: &Body, face: &Face, t: f64) -> Vec<u64> {
    let mut key = Vec::new();
    t.push_bits(&mut key);
    face.surface.push_bits(&mut key);
    face.reversed.push_bits(&mut key);
    for id in &face.loops {
        key.push(u64::MAX);
        let Some(lp) = body.loop_(*id) else {
            continue;
        };
        for coedge in &lp.coedges {
            coedge.reversed.push_bits(&mut key);
            let Some(edge) = body.edge(coedge.edge) else {
                key.push(u64::MAX - 1);
                continue;
            };
            edge.curve.push_bits(&mut key);
            edge.range.push_bits(&mut key);
            for v in [edge.start, edge.end] {
                if let Some(vertex) = body.vertex(v) {
                    vertex.point.push_bits(&mut key);
                }
                body.vertex_tolerance(v).unwrap_or(0.0).push_bits(&mut key);
            }
        }
    }
    key
}

/// A surface's spans at `distance` out of the material, remembered by the
/// surface, its sense and the distance.
fn spans_of(surface: &Surface, distance: f64) -> Spans {
    let mut key = Vec::new();
    surface.push_bits(&mut key);
    distance.push_bits(&mut key);
    if let Some(found) = kept(&SURFACES, &key) {
        return found;
    }
    let found = Arc::new(match regular_by_span(surface, distance, MOST_SPANS) {
        Ok(spans) => {
            SPANS.fetch_add(spans.len() as u64, Ordering::Relaxed);
            Ok(spans)
        }
        Err(_) => Err("its surface has more spans than the rule asks, or none"),
    });
    keep(&SURFACES, key, found.clone());
    found
}

/// The Bézier spans of a freeform surface, each with its patch.
fn patches_of(surface: &Surface) -> Option<Arc<Vec<BezierSpan>>> {
    match surface {
        Surface::Nurbs(n) => n.bezier_spans_shared().ok(),
        Surface::Bezier(b) => NurbsSurface::from_bezier(b)
            .ok()
            .and_then(|n| n.bezier_spans_shared().ok()),
        _ => None,
    }
}

/// The point a border of the box `u × v` of `surface` is drawn into, for
/// each border that is one.
fn collapsed(surface: &Surface, u: (f64, f64), v: (f64, f64)) -> Vec<(usize, Vec3)> {
    let at = |a: f64, b: f64| surface.eval(a, b);
    let size = at(u.0, v.0)
        .distance(at(u.1, v.1))
        .max(at(u.0, v.1).distance(at(u.1, v.0)));
    let fine = 1e-9 * size.max(f64::MIN_POSITIVE);
    let mut found = Vec::new();
    for (k, a, b) in [
        (0, (u.0, v.0), (u.0, v.1)),
        (1, (u.1, v.0), (u.1, v.1)),
        (2, (u.0, v.0), (u.1, v.0)),
        (3, (u.0, v.1), (u.1, v.1)),
    ] {
        let p = at(a.0, a.1);
        let all = (0..=8).all(|i| {
            let s = f64::from(i) / 8.0;
            at(a.0 + (b.0 - a.0) * s, a.1 + (b.1 - a.1) * s).distance(p) <= fine
        });
        if all {
            found.push((k, p));
        }
    }
    found
}

/// A point of the face to say a face-wide line at: the first vertex of
/// its loops (on the face), else the middle of its surface.
fn anchor(body: &Body, face: &Face) -> Vec3 {
    face.loops
        .iter()
        .filter_map(|id| body.loop_(*id))
        .flat_map(|lp| lp.coedges.iter())
        .filter_map(|c| body.edge(c.edge))
        .find_map(|e| body.vertex(e.start).map(|v| v.point))
        .unwrap_or_else(|| {
            let ((u0, u1), (v0, v1)) = face.surface.domain();
            face.surface.eval(0.5 * (u0 + u1), 0.5 * (v0 + v1))
        })
}

fn face_folds(body: &Body, face: &Face, t: f64, most: u64, work: &mut FoldWork) -> Found {
    let surface = &face.surface;
    let outward = if face.reversed { -1.0 } else { 1.0 };
    let mut out: Found = Vec::new();
    let spans = spans_of(surface, 0.5 * t * outward);
    let spans = match spans.as_ref() {
        Ok(spans) => spans,
        Err(why) => {
            out.push((anchor(body, face), FoldKind::NotProven { why }));
            return out;
        }
    };
    // Tip balls: the face's vertices and its surface's poles.
    let mut tips: Vec<(Vec3, f64)> = Vec::new();
    for id in &face.loops {
        let Some(lp) = body.loop_(*id) else {
            continue;
        };
        for coedge in &lp.coedges {
            let Some(edge) = body.edge(coedge.edge) else {
                continue;
            };
            for v in [edge.start, edge.end] {
                if let Some(vertex) = body.vertex(v) {
                    tips.push((vertex.point, TIP * body.vertex_tolerance_at(v, t)));
                }
            }
        }
    }
    let mut look = false;
    let mut no_normals: Vec<((f64, f64), Rect)> = Vec::new();
    for span in spans {
        match &span.regularity {
            SpanRegularity::Proven => {}
            SpanRegularity::Pole { points } => {
                tips.extend(points.iter().map(|p| (*p, TIP * t)));
            }
            SpanRegularity::NoNormal { at } => {
                look = true;
                no_normals.push((*at, (span.u, span.v)));
            }
            SpanRegularity::Folds { .. }
            | SpanRegularity::Unproven { .. }
            | SpanRegularity::TooHigh => look = true,
        }
    }
    if !look && !creased(surface, outward, spans) {
        return out;
    }
    let chart = walk(body, face, t);
    let in_tip = |p: Vec3| tips.iter().any(|(q, r)| q.distance(p) <= *r);
    let mut unsettled: Option<Vec3> = None;
    for (at, span) in no_normals.iter().take(MOST_SEARCHES) {
        if in_tip(surface.eval(at.0, at.1)) {
            continue;
        }
        let places: Vec<((f64, f64), Vec3)> = no_normal(surface, *at, *span)
            .into_iter()
            .filter(|(_, q)| !in_tip(*q))
            .collect();
        if let Some(&(_, q)) = places.iter().find(|(uv, _)| held(&chart, *uv) == Held::In) {
            out.push((q, FoldKind::NoNormal));
            return out;
        }
        if unsettled.is_none() {
            unsettled = places.first().map(|(_, q)| *q);
        }
    }
    work.faces += 1;
    let found = match patches_of(surface) {
        Some(patches) => examine(
            Search::new(surface, outward, t, &tips, &chart),
            &patches,
            most,
            work,
        ),
        None => Examined {
            shown: None,
            doubtful: None,
            spent: true,
        },
    };
    if let Some((p, gap)) = found.shown {
        out.push((p, FoldKind::Layers { gap }));
        return out;
    }
    if let Some(p) = found.doubtful {
        out.push((p, FoldKind::NotProven { why: AT_AN_EDGE }));
    }
    if found.spent {
        out.push((anchor(body, face), FoldKind::NotProven { why: TOO_LARGE }));
    }
    if let Some(q) = unsettled {
        out.push((
            q,
            FoldKind::NotProven {
                why: "its surface has no normal at a place not told in or out of the face",
            },
        ));
    }
    out
}

/// On a surface of degree one across: whether its outward normal turns
/// by more than a right angle across one of its knot lines.
fn creased(surface: &Surface, outward: f64, spans: &[SpanVerdict]) -> bool {
    let Surface::Nurbs(n) = surface else {
        return false;
    };
    let (pu, pv) = n.degrees();
    if pu != 1 && pv != 1 {
        return false;
    }
    let ((u0, u1), (v0, v1)) = surface.domain();
    let normal = |u: f64, v: f64| {
        let (_, a, b) = surface.derivatives(u, v);
        a.cross(b).normalized().map(|n| n * outward)
    };
    let turns = |a: (f64, f64), b: (f64, f64)| matches!((normal(a.0, a.1), normal(b.0, b.1)), (Some(x), Some(y)) if x.dot(y) < 0.0);
    let lines = |pick: fn(&SpanVerdict) -> f64, from: f64| {
        let mut lines: Vec<f64> = spans.iter().map(pick).filter(|x| *x > from).collect();
        lines.sort_by(f64::total_cmp);
        lines.dedup();
        lines
    };
    let (eu, ev) = (1e-7 * (u1 - u0), 1e-7 * (v1 - v0));
    if pu == 1 {
        for u in lines(|s| s.u.0, u0) {
            for i in 0..=64 {
                let v = v0 + (v1 - v0) * f64::from(i) / 64.0;
                if turns((u - eu, v), (u + eu, v)) {
                    return true;
                }
            }
        }
    }
    if pv == 1 {
        for v in lines(|s| s.v.0, v0) {
            for i in 0..=64 {
                let u = u0 + (u1 - u0) * f64::from(i) / 64.0;
                if turns((u, v - ev), (u, v + ev)) {
                    return true;
                }
            }
        }
    }
    false
}

/// A point of the span near `start` where the surface has no normal: a
/// pattern search on `|S_u × S_v|` (eight directions, the step halved
/// when none is better), shown where it is below 1e-12 of the span's own
/// at its middle. Where a whole row has no normal (a loft through its
/// axis) the search may run along it to the face's edge: the row's place
/// across from the start is tried too.
fn no_normal(surface: &Surface, start: (f64, f64), span: Rect) -> Vec<((f64, f64), Vec3)> {
    let ((u0, u1), (v0, v1)) = span;
    let size = |u: f64, v: f64| {
        let (_, a, b) = surface.derivatives(u, v);
        (a.cross(b).length(), a.length() * b.length())
    };
    let (_, reference) = size(0.5 * (u0 + u1), 0.5 * (v0 + v1));
    if !(reference > 0.0 && reference.is_finite()) {
        return Vec::new();
    }
    let mut at = (start.0.max(u0).min(u1), start.1.max(v0).min(v1));
    let mut value = size(at.0, at.1).0;
    let (mut su, mut sv) = (0.25 * (u1 - u0), 0.25 * (v1 - v0));
    for _ in 0..400 {
        if !(value > 1e-12 * reference) {
            break;
        }
        let mut moved = false;
        for (du, dv) in [
            (1.0, 0.0),
            (-1.0, 0.0),
            (0.0, 1.0),
            (0.0, -1.0),
            (1.0, 1.0),
            (1.0, -1.0),
            (-1.0, 1.0),
            (-1.0, -1.0),
        ] {
            let next = (
                (at.0 + du * su).max(u0).min(u1),
                (at.1 + dv * sv).max(v0).min(v1),
            );
            let there = size(next.0, next.1).0;
            if there < value {
                at = next;
                value = there;
                moved = true;
            }
        }
        if !moved {
            su *= 0.5;
            sv *= 0.5;
            if su <= 1e-16 * (u1 - u0).abs() && sv <= 1e-16 * (v1 - v0).abs() {
                break;
            }
        }
    }
    if !(value <= 1e-12 * reference) {
        return Vec::new();
    }
    [
        at,
        (start.0.max(u0).min(u1), at.1),
        (at.0, start.1.max(v0).min(v1)),
    ]
    .into_iter()
    .filter(|c| size(c.0, c.1).0 <= 1e-12 * reference)
    .map(|c| (c, surface.eval(c.0, c.1)))
    .collect()
}

// ---------------------------------------------------------------------
// Two layers through air.

/// What the search for two layers found on a face.
struct Examined {
    /// The least gap of two layers in the face, and the point of the face
    /// it is at.
    shown: Option<(Vec3, f64)>,
    /// A point of the least pair that meets the definition near an edge
    /// or where the loops cannot be followed.
    doubtful: Option<Vec3>,
    /// The search reached its bound.
    spent: bool,
}

/// A piece of a face's surface: a group of its spans, or a patch (a span,
/// or part of one).
struct Piece {
    patch: Option<RationalBezierPatch>,
    members: Vec<usize>,
    rect: Rect,
    /// A box around every point of the piece.
    lo: Vec3,
    hi: Vec3,
    /// Every outward normal of the piece lies within `half` of `axis`;
    /// `None`: any direction.
    axis: Option<Vec3>,
    half: f64,
    depth: u32,
    children: Option<Vec<usize>>,
}

/// The least and the most of `e · x` over a piece: over its control points
/// (a patch lies in their hull), or over its box.
fn extent(piece: &Piece, e: Vec3) -> (f64, f64) {
    match &piece.patch {
        Some(patch) => patch
            .points()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |acc, x| {
                let s = e.dot(x);
                (acc.0.min(s), acc.1.max(s))
            }),
        None => {
            let pick = |l: f64, h: f64, k: f64| {
                if k >= 0.0 {
                    (l * k, h * k)
                } else {
                    (h * k, l * k)
                }
            };
            let (ax, bx) = pick(piece.lo.x, piece.hi.x, e.x);
            let (ay, by) = pick(piece.lo.y, piece.hi.y, e.y);
            let (az, bz) = pick(piece.lo.z, piece.hi.z, e.z);
            (ax + ay + az, bx + by + bz)
        }
    }
}

/// A distance two pieces' points cannot come nearer than: their boxes',
/// and their separation along each of their cones' axes.
fn lower_bound(p: &Piece, q: &Piece) -> f64 {
    let apart = |a: f64, b: f64, c: f64, d: f64| (c - b).max(a - d).max(0.0);
    let dx = apart(p.lo.x, p.hi.x, q.lo.x, q.hi.x);
    let dy = apart(p.lo.y, p.hi.y, q.lo.y, q.hi.y);
    let dz = apart(p.lo.z, p.hi.z, q.lo.z, q.hi.z);
    let mut bound = (dx * dx + dy * dy + dz * dz).sqrt();
    for e in [p.axis, q.axis].into_iter().flatten() {
        let (a0, a1) = extent(p, e);
        let (b0, b1) = extent(q, e);
        bound = bound.max(b0 - a1).max(a0 - b1);
    }
    bound
}

/// Whether no point of one piece can lie on the outward side of a point
/// of the other (so no air between them): `(q − p) · n ≤ cos α (most of
/// (q − p) · e) + sin α |q − p|` for a normal `n` within `α` of `e`.
fn behind(p: &Piece, q: &Piece) -> bool {
    let across = (p.hi.max(q.hi) - p.lo.min(q.lo)).length();
    for (x, y) in [(p, q), (q, p)] {
        let Some(e) = x.axis else {
            continue;
        };
        if !(x.half < FRAC_PI_2) {
            continue;
        }
        let (x0, _) = extent(x, e);
        let (_, y1) = extent(y, e);
        let ahead = y1 - x0;
        if ahead < 0.0 && ahead * x.half.pcos() + across * x.half.psin() < 0.0 {
            return true;
        }
    }
    false
}

/// The cone of the directions of `net` times `outward`: every positive
/// combination of them lies within it while it is narrower than a right
/// angle; else any direction.
fn cone_of(net: &[Vec3], outward: f64) -> (Option<Vec3>, f64) {
    let any = (None, PI);
    let mut units = Vec::with_capacity(net.len());
    for v in net {
        if !v.is_finite() {
            return any;
        }
        if let Some(u) = (*v * outward).normalized() {
            units.push(u);
        }
    }
    let Some(axis) = units.iter().fold(Vec3::ZERO, |a, b| a + *b).normalized() else {
        return any;
    };
    let half = units
        .iter()
        .map(|u| u.dot(axis).clamp(-1.0, 1.0).pacos())
        .fold(0.0_f64, f64::max);
    if half < FRAC_PI_2 {
        (Some(axis), half)
    } else {
        any
    }
}

/// A cone holding every one of `cones`.
fn union_cone(cones: &[(Option<Vec3>, f64)]) -> (Option<Vec3>, f64) {
    let any = (None, PI);
    let mut sum = Vec3::ZERO;
    for (axis, _) in cones {
        let Some(axis) = axis else {
            return any;
        };
        sum += *axis;
    }
    let Some(axis) = sum.normalized() else {
        return any;
    };
    let mut half = 0.0_f64;
    for (a, h) in cones {
        if let Some(a) = a {
            half = half.max(a.dot(axis).clamp(-1.0, 1.0).pacos() + h);
        }
    }
    if half < PI { (Some(axis), half) } else { any }
}

fn middle(rect: Rect) -> (f64, f64) {
    (0.5 * (rect.0.0 + rect.0.1), 0.5 * (rect.1.0 + rect.1.1))
}

/// The search's state on one face.
struct Search<'a> {
    surface: &'a Surface,
    outward: f64,
    t: f64,
    tips: &'a [(Vec3, f64)],
    chart: &'a Chart,
    /// The loops' box in the surface's parameters: pieces outside it are
    /// not of the face.
    window: Option<Rect>,
    /// The angle past which two normals stand against each other.
    opposite: f64,
    pieces: Vec<Piece>,
}

impl<'a> Search<'a> {
    fn new(
        surface: &'a Surface,
        outward: f64,
        t: f64,
        tips: &'a [(Vec3, f64)],
        chart: &'a Chart,
    ) -> Self {
        Search {
            surface,
            outward,
            t,
            tips,
            chart,
            window: window(chart),
            opposite: OPPOSITE.pacos(),
            pieces: Vec::new(),
        }
    }

    /// A patch as a piece, unless it lies wholly in a tip ball or out of
    /// the loops' box, or cannot be bounded.
    fn patch_piece(&self, patch: RationalBezierPatch, rect: Rect, depth: u32) -> Option<Piece> {
        let (lo, hi) = patch.control_bbox().corners()?;
        if !(lo.is_finite() && hi.is_finite()) {
            return None;
        }
        let inside = |c: Vec3, r: f64| {
            let far = Vec3::new(
                (c.x - lo.x).abs().max((hi.x - c.x).abs()),
                (c.y - lo.y).abs().max((hi.y - c.y).abs()),
                (c.z - lo.z).abs().max((hi.z - c.z).abs()),
            );
            far.length() <= r
        };
        if self.tips.iter().any(|(c, r)| inside(*c, *r)) {
            return None;
        }
        if let Some(((wu0, wu1), (wv0, wv1))) = self.window {
            let ((u0, u1), (v0, v1)) = rect;
            if u1 < wu0 || u0 > wu1 || v1 < wv0 || v0 > wv1 {
                return None;
            }
        }
        let (axis, half) = match patch.weighted_normal_net() {
            Some(net) => cone_of(&net, self.outward),
            None => (None, PI),
        };
        Some(Piece {
            patch: Some(patch),
            members: Vec::new(),
            rect,
            lo,
            hi,
            axis,
            half,
            depth,
            children: None,
        })
    }

    fn push(&mut self, piece: Piece) -> usize {
        self.pieces.push(piece);
        self.pieces.len() - 1
    }

    /// A group of pieces (two or more).
    fn group(&mut self, members: Vec<usize>) -> usize {
        let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut hi = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut rect = (
            (f64::INFINITY, f64::NEG_INFINITY),
            (f64::INFINITY, f64::NEG_INFINITY),
        );
        let mut cones = Vec::with_capacity(members.len());
        for p in members.iter().filter_map(|m| self.pieces.get(*m)) {
            lo = lo.min(p.lo);
            hi = hi.max(p.hi);
            rect = (
                (rect.0.0.min(p.rect.0.0), rect.0.1.max(p.rect.0.1)),
                (rect.1.0.min(p.rect.1.0), rect.1.1.max(p.rect.1.1)),
            );
            cones.push((p.axis, p.half));
        }
        let (axis, half) = union_cone(&cones);
        self.push(Piece {
            patch: None,
            members,
            rect,
            lo,
            hi,
            axis,
            half,
            depth: 0,
            children: None,
        })
    }

    /// How much the surface turns times how long it runs across `rect`
    /// along u (or v), at its middle: which way halving bounds it best.
    fn bends(&self, rect: Rect, along_u: bool) -> (f64, f64) {
        let ((u0, u1), (v0, v1)) = rect;
        let (um, vm) = middle(rect);
        let (a, b) = if along_u {
            ((u0, vm), (u1, vm))
        } else {
            ((um, v0), (um, v1))
        };
        let (pa, au, av) = self.surface.derivatives(a.0, a.1);
        let (pb, bu, bv) = self.surface.derivatives(b.0, b.1);
        let length = pa.distance(pb);
        let turn = match (au.cross(av).normalized(), bu.cross(bv).normalized()) {
            (Some(x), Some(y)) => x.distance(y),
            _ => 2.0,
        };
        let finite = |x: f64| if x.is_finite() { x } else { 0.0 };
        (finite(turn * length), finite(length))
    }

    /// The pieces a piece is halved into (made once).
    fn children(&mut self, k: usize) -> Vec<usize> {
        if let Some(made) = self.pieces.get(k).and_then(|p| p.children.clone()) {
            return made;
        }
        let made = self.make_children(k);
        if let Some(p) = self.pieces.get_mut(k) {
            p.children = Some(made.clone());
        }
        made
    }

    fn make_children(&mut self, k: usize) -> Vec<usize> {
        let Some(piece) = self.pieces.get(k) else {
            return Vec::new();
        };
        let Some(patch) = piece.patch.clone() else {
            // A group: its members halved along the longest side of its
            // box, by the middles of their boxes.
            let size = piece.hi - piece.lo;
            let key = |p: &Piece| {
                let m = p.lo + p.hi;
                if size.x >= size.y && size.x >= size.z {
                    m.x
                } else if size.y >= size.z {
                    m.y
                } else {
                    m.z
                }
            };
            let mut keyed: Vec<(f64, usize)> = piece
                .members
                .iter()
                .filter_map(|m| self.pieces.get(*m).map(|p| (key(p), *m)))
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let ordered: Vec<usize> = keyed.into_iter().map(|(_, m)| m).collect();
            if ordered.len() < 2 {
                return ordered;
            }
            let (left, right) = ordered.split_at(ordered.len() / 2);
            let mut out = Vec::with_capacity(2);
            for half in [left.to_vec(), right.to_vec()] {
                match half.len() {
                    0 => {}
                    1 => out.extend(half.first().copied()),
                    _ => out.push(self.group(half)),
                }
            }
            return out;
        };
        let across = (piece.hi - piece.lo).length();
        if !(across > LEAF * self.t) || piece.depth >= DEEPEST {
            return Vec::new();
        }
        let (rect, depth) = (piece.rect, piece.depth + 1);
        let ((u0, u1), (v0, v1)) = rect;
        let (um, vm) = middle(rect);
        let (bend_u, long_u) = self.bends(rect, true);
        let (bend_v, long_v) = self.bends(rect, false);
        let along_u = if bend_u > 1e-9 * (long_u + long_v) || bend_v > 1e-9 * (long_u + long_v) {
            bend_u >= bend_v
        } else {
            long_u >= long_v
        };
        let halves = if along_u {
            let (a, b) = patch.split_u(0.5);
            [(a, ((u0, um), (v0, v1))), (b, ((um, u1), (v0, v1)))]
        } else {
            let (a, b) = patch.split_v(0.5);
            [(a, ((u0, u1), (v0, vm))), (b, ((u0, u1), (vm, v1)))]
        };
        let mut out = Vec::with_capacity(2);
        for (half, rect) in halves {
            if let Some(piece) = self.patch_piece(half, rect, depth) {
                out.push(self.push(piece));
            }
        }
        out
    }

    fn opposable(&self, p: &Piece, q: &Piece) -> bool {
        let (Some(a), Some(b)) = (p.axis, q.axis) else {
            return true;
        };
        a.dot(b).clamp(-1.0, 1.0).pacos() + p.half + q.half > self.opposite
    }

    /// Which of two pieces to halve, and the other: a group before a
    /// patch, the larger of two.
    fn which(&self, a: usize, b: usize) -> (usize, usize) {
        let (Some(p), Some(q)) = (self.pieces.get(a), self.pieces.get(b)) else {
            return (a, b);
        };
        let first = match (&p.patch, &q.patch) {
            (None, Some(_)) => true,
            (Some(_), None) => false,
            (None, None) => p.members.len() >= q.members.len(),
            (Some(_), Some(_)) => (p.hi - p.lo).length() >= (q.hi - q.lo).length(),
        };
        if first { (a, b) } else { (b, a) }
    }

    /// The pair `(x, y)` on the heap, nearest first, when its pieces can
    /// hold two layers facing each other through air nearer than `limit`.
    fn offer(&self, heap: &mut Heap, seq: &mut u64, x: usize, y: usize, limit: f64) {
        let (Some(p), Some(q)) = (self.pieces.get(x), self.pieces.get(y)) else {
            return;
        };
        let bound = if x == y {
            if p.axis.is_some() && !(2.0 * p.half > self.opposite) {
                return;
            }
            0.0
        } else {
            if !self.opposable(p, q) || behind(p, q) {
                return;
            }
            lower_bound(p, q)
        };
        if !(bound < limit) {
            return;
        }
        *seq += 1;
        heap.push(Reverse((bound.max(0.0).to_bits(), *seq, x, y)));
    }

    /// Two points of the surface (parameters) that meet the definition:
    /// their gap and the first.
    fn facing(&self, a: (f64, f64), b: (f64, f64)) -> Option<(f64, Vec3)> {
        let (p, pu, pv) = self.surface.derivatives(a.0, a.1);
        let (q, qu, qv) = self.surface.derivatives(b.0, b.1);
        let m = pu.cross(pv).normalized()? * self.outward;
        let n = qu.cross(qv).normalized()? * self.outward;
        let d = q - p;
        let gap = d.length();
        let room = 1e-12 * p.length().max(self.t);
        if !(gap < self.t - room) || !(m.dot(n) < OPPOSITE) {
            return None;
        }
        if !(d.dot(m) > room && -d.dot(n) > room) {
            return None;
        }
        let in_tip = |x: Vec3| self.tips.iter().any(|(c, r)| c.distance(x) <= *r);
        if in_tip(p) || in_tip(q) {
            return None;
        }
        Some((gap, p))
    }
}

type Heap = BinaryHeap<Reverse<(u64, u64, usize, usize)>>;

/// The nearest two points of the two layers about `a` and `b`
/// (parameters): projections back and forth.
fn refine(surface: &Surface, a: (f64, f64), b: (f64, f64)) -> ((f64, f64), (f64, f64)) {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let scale = (u1 - u0).abs() + (v1 - v0).abs();
    let (mut a, mut b) = (a, b);
    for _ in 0..ROUNDS {
        let q = surface.eval(b.0, b.1);
        let Ok(a2) = surface.project_from(q, a) else {
            break;
        };
        let p = surface.eval(a2.0, a2.1);
        let Ok(b2) = surface.project_from(p, b) else {
            a = a2;
            break;
        };
        let moved =
            (a2.0 - a.0).abs() + (a2.1 - a.1).abs() + (b2.0 - b.0).abs() + (b2.1 - b.1).abs();
        a = a2;
        b = b2;
        if !(moved > 1e-12 * scale) {
            break;
        }
    }
    (a, b)
}

/// The search for two layers of a face facing each other through air
/// within the tolerance (the module's step 3).
fn examine(
    mut search: Search<'_>,
    spans: &[BezierSpan],
    most: u64,
    work: &mut FoldWork,
) -> Examined {
    let t = search.t;
    let mut leaves = Vec::new();
    for span in spans {
        if let Some(piece) = search.patch_piece(span.patch.clone(), (span.u, span.v), 0) {
            leaves.push(search.push(piece));
        }
    }
    let mut found = Examined {
        shown: None,
        doubtful: None,
        spent: false,
    };
    let root = match leaves.len() {
        0 => return found,
        1 => match leaves.first() {
            Some(one) => *one,
            None => return found,
        },
        _ => search.group(leaves),
    };
    let mut doubtful_gap = f64::INFINITY;
    let limit = |shown: &Option<(Vec3, f64)>| match shown {
        Some((_, gap)) => (gap - EXACT * t).min(t),
        None => t,
    };
    let mut heap: Heap = BinaryHeap::new();
    let mut seq = 0_u64;
    let mut pairs = 0_u64;
    search.offer(&mut heap, &mut seq, root, root, t);
    while let Some(Reverse((bits, _, a, b))) = heap.pop() {
        let bound = f64::from_bits(bits);
        if !(bound < limit(&found.shown)) {
            break;
        }
        pairs += 1;
        if pairs > most {
            found.spent = true;
            break;
        }
        if a == b {
            let kids = search.children(a);
            for (i, &x) in kids.iter().enumerate() {
                let now = limit(&found.shown);
                search.offer(&mut heap, &mut seq, x, x, now);
                for &y in kids.iter().skip(i + 1) {
                    search.offer(&mut heap, &mut seq, x, y, now);
                }
            }
            continue;
        }
        let both = match (search.pieces.get(a), search.pieces.get(b)) {
            (Some(p), Some(q)) if p.patch.is_some() && q.patch.is_some() => {
                Some((middle(p.rect), middle(q.rect)))
            }
            _ => None,
        };
        if let Some((ma, mb)) = both {
            for (ua, ub) in [(ma, mb), refine(search.surface, ma, mb)] {
                let Some((gap, p)) = search.facing(ua, ub) else {
                    continue;
                };
                let better = found.shown.is_none_or(|s| gap < s.1);
                if !better && !(gap < doubtful_gap) {
                    continue;
                }
                match (held(search.chart, ua), held(search.chart, ub)) {
                    (Held::In, Held::In) => {
                        work.in_in += 1;
                        work.least_in_in = work.least_in_in.min(gap);
                        if better {
                            found.shown = Some((p, gap));
                        }
                    }
                    (Held::Out, _) | (_, Held::Out) => work.out += 1,
                    (x, y) => {
                        if x == Held::Near || y == Held::Near {
                            work.near += 1;
                        } else {
                            work.not_told += 1;
                        }
                        work.least_doubtful = work.least_doubtful.min(gap);
                        if gap < doubtful_gap {
                            doubtful_gap = gap;
                            found.doubtful = Some(p);
                        }
                    }
                }
            }
        }
        let (split, other) = search.which(a, b);
        let (mut kids, mut keep) = (search.children(split), other);
        if kids.is_empty() {
            kids = search.children(other);
            keep = split;
        }
        let now = limit(&found.shown);
        for x in kids {
            search.offer(&mut heap, &mut seq, x, keep, now);
        }
    }
    work.pairs += pairs;
    work.most_pairs = work.most_pairs.max(pairs);
    if found.spent {
        work.spent += 1;
    }
    found
}

// ---------------------------------------------------------------------
// In the face.

/// The face's loops in its surface's parameters.
struct Chart {
    rings: Vec<Vec<(f64, f64)>>,
    /// Per ring, per chord: how far it strays from the edge's feet at its
    /// middle.
    sags: Vec<Vec<f64>>,
    followed: bool,
}

/// The loops' box in parameters, grown by twice the largest chord error.
fn window(chart: &Chart) -> Option<Rect> {
    if !chart.followed {
        return None;
    }
    let mut rect = (
        (f64::INFINITY, f64::NEG_INFINITY),
        (f64::INFINITY, f64::NEG_INFINITY),
    );
    for &(u, v) in chart.rings.iter().flatten() {
        rect = (
            (rect.0.0.min(u), rect.0.1.max(u)),
            (rect.1.0.min(v), rect.1.1.max(v)),
        );
    }
    let sag = chart.sags.iter().flatten().fold(0.0_f64, |a, b| a.max(*b));
    let room = 2.0 * sag + 1e-9 * ((rect.0.1 - rect.0.0).abs() + (rect.1.1 - rect.1.0).abs());
    let grown = (
        (rect.0.0 - room, rect.0.1 + room),
        (rect.1.0 - room, rect.1.1 + room),
    );
    let finite = [grown.0.0, grown.0.1, grown.1.0, grown.1.1]
        .iter()
        .all(|x| x.is_finite());
    finite.then_some(grown)
}

fn walk(body: &Body, face: &Face, t: f64) -> Chart {
    let surface = &face.surface;
    let ((u0, u1), (v0, v1)) = surface.domain();
    let poles = collapsed(surface, (u0, u1), (v0, v1));
    let border = |k: usize| match k {
        0 => u0,
        1 => u1,
        2 => v0,
        _ => v1,
    };
    let size = surface
        .eval(u0, v0)
        .distance(surface.eval(u1, v1))
        .max(surface.eval(u0, v1).distance(surface.eval(u1, v0)));
    let at_pole = |p: Vec3| {
        poles
            .iter()
            .find(|(_, q)| q.distance(p) <= 1e-9 * size.max(f64::MIN_POSITIVE))
            .map(|(k, _)| *k)
    };
    let mut out = Chart {
        rings: Vec::new(),
        sags: Vec::new(),
        followed: true,
    };
    for id in &face.loops {
        let Some(lp) = body.loop_(*id) else {
            out.followed = false;
            return out;
        };
        let mut ring: Vec<(f64, f64)> = Vec::new();
        let mut sags: Vec<f64> = Vec::new();
        for coedge in &lp.coedges {
            let Some(edge) = body.edge(coedge.edge) else {
                out.followed = false;
                return out;
            };
            let (t0, t1) = edge.range;
            // About one step a tolerance of the edge's length.
            let length: f64 = (0..32_u32)
                .map(|i| {
                    let at = |k: u32| edge.curve.eval(t0 + (t1 - t0) * f64::from(k) / 32.0);
                    at(i).distance(at(i + 1))
                })
                .sum();
            let mut count = FEWEST_STEPS;
            while count < MOST_STEPS && (count as f64) * t < length {
                count *= 2;
            }
            let steps = count as f64;
            let along = |i: f64| {
                let s = i / steps;
                let s = if coedge.reversed { 1.0 - s } else { s };
                edge.curve.eval(t0 + (t1 - t0) * s)
            };
            let points: Vec<Vec3> = (0..=count).map(|i| along(i as f64)).collect();
            let mut uvs: Vec<Option<(f64, f64)>> = Vec::with_capacity(points.len());
            for p in &points {
                let before = uvs
                    .iter()
                    .rev()
                    .find_map(|x| *x)
                    .or_else(|| ring.last().copied());
                uvs.push(match before {
                    Some(start) => surface
                        .project_from(*p, start)
                        .ok()
                        .or_else(|| surface.project(*p).ok()),
                    None => surface.project(*p).ok(),
                });
            }
            // A point at a pole: the border's parameter, its
            // neighbour's other one.
            for i in 0..points.len() {
                let Some(k) = points.get(i).and_then(|p| at_pole(*p)) else {
                    continue;
                };
                let neighbour = (1..points.len()).find_map(|d| {
                    [i.checked_sub(d), i.checked_add(d)]
                        .into_iter()
                        .flatten()
                        .find(|&j| points.get(j).is_some_and(|q| at_pole(*q).is_none()))
                        .and_then(|j| uvs.get(j).copied().flatten())
                });
                if let (Some((nu, nv)), Some(slot)) = (neighbour, uvs.get_mut(i)) {
                    *slot = Some(if k < 2 {
                        (border(k), nv)
                    } else {
                        (nu, border(k))
                    });
                }
            }
            let Some(firm) = uvs.into_iter().collect::<Option<Vec<(f64, f64)>>>() else {
                out.followed = false;
                return out;
            };
            let mut own = vec![0.0_f64; firm.len()];
            for i in 0..count {
                let middle = along(i as f64 + 0.5);
                if at_pole(middle).is_some() {
                    continue;
                }
                let (Some(a), Some(b)) = (firm.get(i), firm.get(i + 1)) else {
                    continue;
                };
                if let (Ok(m), Some(slot)) = (surface.project_from(middle, *a), own.get_mut(i)) {
                    let chord = (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1));
                    let off = ((m.0 - chord.0) * (m.0 - chord.0)
                        + (m.1 - chord.1) * (m.1 - chord.1))
                        .sqrt();
                    if off.is_finite() {
                        *slot = off;
                    }
                }
            }
            sags.extend(own);
            ring.extend(firm);
        }
        out.rings.push(ring);
        out.sags.push(sags);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Held {
    In,
    Near,
    Out,
    NotTold,
}

fn winding(p: (f64, f64), ring: &[(f64, f64)]) -> i64 {
    let mut w = 0_i64;
    let n = ring.len();
    for i in 0..n {
        let (Some(a), Some(b)) = (ring.get(i), ring.get((i + 1) % n)) else {
            continue;
        };
        let side = (b.0 - a.0) * (p.1 - a.1) - (p.0 - a.0) * (b.1 - a.1);
        if a.1 <= p.1 {
            if b.1 > p.1 && side > 0.0 {
                w += 1;
            }
        } else if b.1 <= p.1 && side < 0.0 {
            w -= 1;
        }
    }
    w
}

fn to_chord(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let d = (b.0 - a.0, b.1 - a.1);
    let l = d.0 * d.0 + d.1 * d.1;
    let s = if l > 0.0 {
        (((p.0 - a.0) * d.0 + (p.1 - a.1) * d.1) / l).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (x, y) = (p.0 - a.0 - d.0 * s, p.1 - a.1 - d.1 * s);
    (x * x + y * y).sqrt()
}

fn held(chart: &Chart, uv: (f64, f64)) -> Held {
    if !chart.followed {
        return Held::NotTold;
    }
    for (ring, sags) in chart.rings.iter().zip(&chart.sags) {
        let n = ring.len();
        for i in 0..n {
            let (Some(a), Some(b)) = (ring.get(i), ring.get((i + 1) % n)) else {
                continue;
            };
            let room = 2.0 * sags.get(i).copied().unwrap_or(0.0) + 1e-12;
            if to_chord(uv, *a, *b) <= room {
                return Held::Near;
            }
        }
    }
    let total: i64 = chart.rings.iter().map(|r| winding(uv, r)).sum();
    match total {
        0 => Held::Out,
        1 | -1 => Held::In,
        _ => Held::NotTold,
    }
}
