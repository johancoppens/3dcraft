//! A face folded over itself (docs/design/verification.md › *A face
//! folded over itself*): two layers of a face within the document's
//! tolerance `t` of each other, facing each other through air, or a point
//! of it without a normal. For now a warning (`document.check` says each
//! place); [`FOLDS_REFUSED`] is where it would become a defect of
//! [`crate::check`].
//!
//! Per freeform face:
//!
//! 1. **Suspects.** Every Bézier span of its surface whose offset by `t / 2`
//!    out of the material is not proven not to fold
//!    (`craft3d_geom::offset::regular_by_span`): a concave bend tighter
//!    than `t / 2`, no normal, or not proven; a span with a border drawn
//!    into a point is a pole's, no suspect. On a surface of degree one,
//!    the places on its knot lines where its normal turns by more than a
//!    right angle (inside such a span nothing bends). A span of degrees
//!    too high for the forms is said as not proven.
//! 2. **Two layers through air.** About each suspect, and about the least
//!    concave radius of its span, the face's points within eight
//!    tolerances on a grid of its parameters: two of them within `t` of
//!    each other, their outward normals more than 154° apart (cosine
//!    below −0.9) and each on the other's outward side, both in the face
//!    and outside the tip balls (four tolerances, or a vertex's own, about
//!    the face's vertices and its surface's poles).
//! 3. **No normal.** About a span the forms find no normal in, a search
//!    on `|S_u × S_v|` (a pattern search, halving its step) for a point
//!    where it is below 1e-12 of the span's own: shown there.
//! 4. **In the face**, also on a surface with a pole: the loops followed
//!    in the surface's parameters, each point of an edge projected from
//!    the one before; a point at a pole takes the pole's border parameter
//!    and its neighbour's other one, so that the stretch of the border
//!    between the edge into the pole and the edge out of it is part of the
//!    loop. A point is in where the loops wind round it once and it is
//!    farther than twice its chord's error from every chord; near; out;
//!    not told where the loops cannot be followed.
//!
//! Shown: a fold (certain). Pairs only near an edge or not told, a search
//! past its work, spans the forms cannot take: not proven. A suspect
//! searched without a pair passes. Faces are remembered by everything the
//! rule reads (the surface, its sense, the tolerance, the loops' edges and
//! the vertices' own tolerances), and each surface's spans by the surface,
//! its sense and the tolerance.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::{Body, Face, FaceId};
use craft3d_geom::Surface;
use craft3d_geom::offset::{SpanRegularity, SpanVerdict, regular_by_span};
use craft3d_math::{Bits, Tolerance, Vec3};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// Whether [`crate::check`] refuses a body with a fold
/// ([`crate::Defect::FaceFolds`]): not yet. The rule is a warning of
/// `document.check` for one round first (the developer, 2026-11-18).
pub const FOLDS_REFUSED: bool = false;

/// The tip balls' radius, in tolerances.
const TIP: f64 = 4.0;
/// How near a search looks about a suspect, in tolerances.
const REACH: f64 = 8.0;
/// The cosine below which two normals stand against each other (154°).
const OPPOSITE: f64 = -0.9;
/// The search's grid over its box, a side.
const GRID: usize = 120;
/// Steps along each edge of the walk.
const PER_EDGE: usize = 1024;
/// The work of a face: suspects searched, pairs looked at in one search,
/// spans asked.
const MOST_SEARCHES: usize = 32;
const MOST_PAIRS: u64 = 4_000_000;
const MOST_SPANS: usize = 1 << 16;
/// What is remembered: faces and surfaces, and words of keys in all.
const REMEMBERED: usize = 4096;
const REMEMBERED_WORDS: usize = 1 << 22;

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

/// The spans asked and the searches made since the process started (each
/// face or surface once; a remembered one is not counted again): counts,
/// not a clock.
pub fn fold_work() -> (u64, u64) {
    (
        SPANS.load(Ordering::Relaxed),
        SEARCHES.load(Ordering::Relaxed),
    )
}

type Found = Vec<(Vec3, FoldKind)>;
type Spans = Arc<Result<Vec<SpanVerdict>, &'static str>>;

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

/// Every place of `body`'s freeform faces the rule speaks of, at the
/// tolerance `tol.linear`, faces in order.
pub fn folds(body: &Body, tol: &Tolerance) -> Vec<Fold> {
    let t = tol.linear;
    let mut out = Vec::new();
    if !(t.is_finite() && t > 0.0) {
        return out;
    }
    for (index, face) in body.faces.iter().enumerate() {
        if !matches!(face.surface, Surface::Nurbs(_) | Surface::Bezier(_)) {
            continue;
        }
        let id = FaceId(u32::try_from(index).unwrap_or(u32::MAX));
        let key = face_key(body, face, t);
        let found = match kept(&FACES, &key) {
            Some(found) => found,
            None => {
                let found = face_folds(body, face, t);
                keep(&FACES, key, found.clone());
                found
            }
        };
        out.extend(found.into_iter().map(|(at, kind)| Fold {
            face: id,
            at,
            kind,
            tolerance: t,
        }));
    }
    out
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

/// A place to search about, and the span it was found in.
#[derive(Clone, Copy)]
struct Suspect {
    uv: (f64, f64),
    span: ((f64, f64), (f64, f64)),
    no_normal: bool,
}

fn face_folds(body: &Body, face: &Face, t: f64) -> Found {
    let surface = &face.surface;
    let outward = if face.reversed { -1.0 } else { 1.0 };
    let mut out: Found = Vec::new();
    let spans = spans_of(surface, 0.5 * t * outward);
    let spans = match spans.as_ref() {
        Ok(spans) => spans,
        Err(why) => {
            let ((u0, u1), (v0, v1)) = surface.domain();
            out.push((
                surface.eval(0.5 * (u0 + u1), 0.5 * (v0 + v1)),
                FoldKind::NotProven { why },
            ));
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
    let mut suspects: Vec<Suspect> = Vec::new();
    let mut too_high: Vec<(f64, f64)> = Vec::new();
    for span in spans {
        let middle = (0.5 * (span.u.0 + span.u.1), 0.5 * (span.v.0 + span.v.1));
        let boxed = (span.u, span.v);
        match &span.regularity {
            SpanRegularity::Proven => {}
            SpanRegularity::Pole { points } => {
                tips.extend(points.iter().map(|p| (*p, TIP * t)));
            }
            SpanRegularity::Folds { at, .. } | SpanRegularity::Unproven { at } => {
                suspects.push(Suspect {
                    uv: *at,
                    span: boxed,
                    no_normal: false,
                });
            }
            SpanRegularity::NoNormal { at } => suspects.push(Suspect {
                uv: *at,
                span: boxed,
                no_normal: true,
            }),
            SpanRegularity::TooHigh => too_high.push(middle),
        }
    }
    creases(surface, outward, spans, &mut suspects);
    let in_tip = |p: Vec3| tips.iter().any(|(q, r)| q.distance(p) <= *r);
    for uv in too_high {
        let p = surface.eval(uv.0, uv.1);
        if !in_tip(p) {
            out.push((
                p,
                FoldKind::NotProven {
                    why: "a span of its surface is of degrees too high for the proof",
                },
            ));
        }
    }
    if suspects.is_empty() {
        return out;
    }
    let chart = walk(body, face);
    let mut searched: Vec<Vec3> = Vec::new();
    let mut spent = false;
    for suspect in &suspects {
        let p = surface.eval(suspect.uv.0, suspect.uv.1);
        if in_tip(p) || searched.iter().any(|q| q.distance(p) <= 2.0 * t) {
            continue;
        }
        if searched.len() >= MOST_SEARCHES {
            spent = true;
            break;
        }
        searched.push(p);
        SEARCHES.fetch_add(1, Ordering::Relaxed);
        if suspect.no_normal
            && let Some((uv, q)) = no_normal(surface, suspect.uv, suspect.span)
            && !in_tip(q)
        {
            match held(&chart, uv) {
                Held::In => out.push((q, FoldKind::NoNormal)),
                _ => out.push((
                    q,
                    FoldKind::NotProven {
                        why: "its surface has no normal at a place not told in or out of the face",
                    },
                )),
            }
            continue;
        }
        let mut starts = vec![suspect.uv];
        if let Some(uv) = least_concave(surface, outward, suspect.span, &tips, t) {
            starts.push(uv);
        }
        let mut shown: Option<(Vec3, f64)> = None;
        let mut doubtful = false;
        for start in starts {
            let found = layers(surface, outward, start, t, &tips, &chart);
            if let Some((at, gap)) = found.shown
                && shown.is_none_or(|s| gap < s.1)
            {
                shown = Some((at, gap));
            }
            doubtful |= found.doubtful;
        }
        match shown {
            Some((at, gap)) => out.push((at, FoldKind::Layers { gap })),
            None if doubtful => out.push((
                p,
                FoldKind::NotProven {
                    why: "two of its layers come within the tolerance of each other near an edge, or where its loops cannot be followed",
                },
            )),
            None => {}
        }
    }
    if spent && let Some(p) = suspects.last().map(|s| surface.eval(s.uv.0, s.uv.1)) {
        out.push((
            p,
            FoldKind::NotProven {
                why: "the search ran out of its work on this face",
            },
        ));
    }
    out
}

/// On a surface of degree one across: the places on its knot lines where
/// the outward normal turns by more than a right angle.
fn creases(surface: &Surface, outward: f64, spans: &[SpanVerdict], out: &mut Vec<Suspect>) {
    let (pu, pv) = match surface {
        Surface::Nurbs(n) => n.degrees(),
        Surface::Bezier(_) => return,
        _ => return,
    };
    if pu != 1 && pv != 1 {
        return;
    }
    let ((u0, u1), (v0, v1)) = surface.domain();
    let normal = |u: f64, v: f64| {
        let (_, a, b) = surface.derivatives(u, v);
        a.cross(b).normalized().map(|n| n * outward)
    };
    let mut lines_u: Vec<f64> = spans.iter().map(|s| s.u.0).filter(|u| *u > u0).collect();
    lines_u.sort_by(f64::total_cmp);
    lines_u.dedup();
    let mut lines_v: Vec<f64> = spans.iter().map(|s| s.v.0).filter(|v| *v > v0).collect();
    lines_v.sort_by(f64::total_cmp);
    lines_v.dedup();
    let (eu, ev) = (1e-7 * (u1 - u0), 1e-7 * (v1 - v0));
    let place = |uv: (f64, f64), span: ((f64, f64), (f64, f64))| Suspect {
        uv,
        span,
        no_normal: false,
    };
    if pu == 1 {
        for &u in &lines_u {
            for i in 0..=64 {
                let v = v0 + (v1 - v0) * f64::from(i) / 64.0;
                if let (Some(a), Some(b)) = (normal(u - eu, v), normal(u + eu, v))
                    && a.dot(b) < 0.0
                {
                    out.push(place((u, v), ((u, u), (v, v))));
                }
            }
        }
    }
    if pv == 1 {
        for &v in &lines_v {
            for i in 0..=64 {
                let u = u0 + (u1 - u0) * f64::from(i) / 64.0;
                if let (Some(a), Some(b)) = (normal(u, v - ev), normal(u, v + ev))
                    && a.dot(b) < 0.0
                {
                    out.push(place((u, v), ((u, u), (v, v))));
                }
            }
        }
    }
}

/// The least concave radius over a span, away from the tips: a grid of
/// 24 × 24 over the span, then four rounds of a finer one about the best
/// place; where it is below `t`.
fn least_concave(
    surface: &Surface,
    outward: f64,
    span: ((f64, f64), (f64, f64)),
    tips: &[(Vec3, f64)],
    t: f64,
) -> Option<(f64, f64)> {
    let ((u0, u1), (v0, v1)) = span;
    if !(u1 > u0 && v1 > v0) {
        return None;
    }
    let concave = |u: f64, v: f64| -> Option<f64> {
        let (k, h) = craft3d_geom::curvature::curvatures(surface, u, v)?;
        let root = (h * h - k).max(0.0).sqrt();
        let towards = (outward * (h + root)).max(outward * (h - root));
        Some(if towards > 0.0 {
            1.0 / towards
        } else {
            f64::INFINITY
        })
    };
    let mut best: Option<(f64, (f64, f64))> = None;
    let (mut cu, mut cv) = (0.5 * (u0 + u1), 0.5 * (v0 + v1));
    let (mut hu, mut hv) = (0.5 * (u1 - u0), 0.5 * (v1 - v0));
    for _ in 0..5 {
        let n = 24_u32;
        for i in 0..=n {
            for j in 0..=n {
                let u = (cu + hu * (2.0 * f64::from(i) / f64::from(n) - 1.0)).clamp(u0, u1);
                let v = (cv + hv * (2.0 * f64::from(j) / f64::from(n) - 1.0)).clamp(v0, v1);
                let Some(r) = concave(u, v) else {
                    continue;
                };
                if best.is_some_and(|b| !(r < b.0)) {
                    continue;
                }
                let p = surface.eval(u, v);
                if tips.iter().any(|(q, radius)| q.distance(p) <= *radius) {
                    continue;
                }
                best = Some((r, (u, v)));
            }
        }
        if let Some((_, (u, v))) = best {
            cu = u;
            cv = v;
        }
        hu /= 6.0;
        hv /= 6.0;
    }
    best.filter(|b| b.0 < t).map(|b| b.1)
}

/// A point of the span near `start` where the surface has no normal: a
/// pattern search on `|S_u × S_v|` (eight directions, the step halved
/// when none is better), shown where it is below 1e-12 of the span's own
/// at its middle.
fn no_normal(
    surface: &Surface,
    start: (f64, f64),
    span: ((f64, f64), (f64, f64)),
) -> Option<((f64, f64), Vec3)> {
    let ((u0, u1), (v0, v1)) = span;
    let size = |u: f64, v: f64| {
        let (_, a, b) = surface.derivatives(u, v);
        (a.cross(b).length(), a.length() * b.length())
    };
    let (_, reference) = size(0.5 * (u0 + u1), 0.5 * (v0 + v1));
    if !(reference > 0.0 && reference.is_finite()) {
        return None;
    }
    let mut at = (start.0.clamp(u0, u1), start.1.clamp(v0, v1));
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
                (at.0 + du * su).clamp(u0, u1),
                (at.1 + dv * sv).clamp(v0, v1),
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
    (value <= 1e-12 * reference).then(|| (at, surface.eval(at.0, at.1)))
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

fn walk(body: &Body, face: &Face) -> Chart {
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
    let steps = f64::from(u32::try_from(PER_EDGE).unwrap_or(1024));
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
            let along = |i: f64| {
                let s = i / steps;
                let s = if coedge.reversed { 1.0 - s } else { s };
                edge.curve.eval(t0 + (t1 - t0) * s)
            };
            let points: Vec<Vec3> = (0..=PER_EDGE).map(|i| along(i as f64)).collect();
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
            for i in 0..PER_EDGE {
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

// ---------------------------------------------------------------------
// Two layers through air.

struct Layers {
    /// The least gap of two layers shown, and its middle.
    shown: Option<(Vec3, f64)>,
    /// Pairs near an edge, not told, or past the work.
    doubtful: bool,
}

type Sample = ((f64, f64), Vec3, Option<Vec3>);

fn layers(
    surface: &Surface,
    outward: f64,
    at: (f64, f64),
    t: f64,
    tips: &[(Vec3, f64)],
    chart: &Chart,
) -> Layers {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let p0 = surface.eval(at.0, at.1);
    let reach = REACH * t;
    let grid = |hu: f64, hv: f64, n: usize| -> Vec<Sample> {
        let mut found = Vec::with_capacity((n + 1) * (n + 1));
        let steps = n as f64;
        for i in 0..=n {
            for j in 0..=n {
                let u = (at.0 + hu * (2.0 * i as f64 / steps - 1.0)).clamp(u0, u1);
                let v = (at.1 + hv * (2.0 * j as f64 / steps - 1.0)).clamp(v0, v1);
                let (p, a, b) = surface.derivatives(u, v);
                found.push(((u, v), p, a.cross(b).normalized().map(|m| m * outward)));
            }
        }
        found
    };
    let far = |hu: f64, hv: f64| -> f64 {
        grid(hu, hv, 6)
            .iter()
            .map(|(_, p, _)| p.distance(p0))
            .fold(0.0_f64, f64::max)
    };
    let (mut hu, mut hv) = (0.25 * (u1 - u0), 0.25 * (v1 - v0));
    for _ in 0..80 {
        let (along_u, along_v) = (far(hu, 0.0), far(0.0, hv));
        if along_u <= reach && along_v <= reach {
            break;
        }
        if along_u > reach {
            hu *= 0.5;
        }
        if along_v > reach {
            hv *= 0.5;
        }
    }
    let all = grid(hu, hv, GRID);
    let mut out = Layers {
        shown: None,
        doubtful: false,
    };
    // The points near the place, outside the tip balls, with a normal,
    // each told in or out of the face once.
    let mut told: BTreeMap<usize, Held> = BTreeMap::new();
    let mut cells: BTreeMap<(i64, i64, i64), Vec<usize>> = BTreeMap::new();
    let cell = |p: Vec3| {
        (
            (p.x / t).floor() as i64,
            (p.y / t).floor() as i64,
            (p.z / t).floor() as i64,
        )
    };
    for (k, (uv, p, m)) in all.iter().enumerate() {
        if m.is_none() || p.distance(p0) > reach || tips.iter().any(|(q, r)| q.distance(*p) <= *r) {
            continue;
        }
        told.insert(k, held(chart, *uv));
        cells.entry(cell(*p)).or_default().push(k);
    }
    let room = 1e-12 * p0.length().max(t);
    let mut pairs = 0_u64;
    for (&k, &held_p) in &told {
        let Some(&(_, p, Some(m))) = all.get(k) else {
            continue;
        };
        let c = cell(p);
        for dx in -1..=1_i64 {
            for dy in -1..=1_i64 {
                for dz in -1..=1_i64 {
                    let Some(list) = cells.get(&(c.0 + dx, c.1 + dy, c.2 + dz)) else {
                        continue;
                    };
                    for &l in list {
                        if l <= k {
                            continue;
                        }
                        pairs += 1;
                        if pairs > MOST_PAIRS {
                            out.doubtful = true;
                            return out;
                        }
                        let Some(&(_, q, Some(other))) = all.get(l) else {
                            continue;
                        };
                        let d = q - p;
                        let gap = d.length();
                        if !(gap < t - room) || !(m.dot(other) < OPPOSITE) {
                            continue;
                        }
                        if !(d.dot(m) > room && -d.dot(other) > room) {
                            continue;
                        }
                        let held_q = told.get(&l).copied().unwrap_or(Held::NotTold);
                        if held_p == Held::In && held_q == Held::In {
                            if out.shown.is_none_or(|s| gap < s.1) {
                                out.shown = Some(((p + q) * 0.5, gap));
                            }
                        } else if held_p != Held::Out && held_q != Held::Out {
                            out.doubtful = true;
                        }
                    }
                }
            }
        }
    }
    out
}
