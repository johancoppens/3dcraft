//! Ignored in the gate (a probe: it prints and holds nothing; fold-check
//! 2026-11-19, after the review): run it with
//! `cargo test -p craft3d-engine --test fold_layers -- --ignored --nocapture`.
//!
//! fold-check, round 2: a probe (printed, held to nothing) of the revised
//! definition of a face folded over itself (docs/design/verification.md ›
//! *A face folded over itself*). Per freeform face, at a tolerance `t`
//! (the document's, and for the corpus also a coarser one):
//!
//! 1. *Suspects*: every Bézier span whose offset by `t / 2` out of the
//!    material `geom::offset::regular` does not prove (a concave bend
//!    tighter than `t / 2`, no normal, or not proven); a span with a
//!    border drawn into a point is a pole's and no suspect; on a surface
//!    of degree one, the places on its knot lines where the normal turns
//!    by more than 60°.
//! 2. *The layer search about each suspect*: the face's points within
//!    eight tolerances of it on a grid of its parameters; two of them
//!    within `t` of each other, their outward normals more than 154°
//!    apart (cosine below −0.9), each on the other's outward side (air
//!    between them), both outside the tip balls (four tolerances about
//!    the face's vertices and its surface's poles) and both in the face:
//!    two layers shown. The grid's own triangles crossing each other away
//!    from their neighbours: a crossing seen.
//! 3. *In the face*, also on a surface with a pole: the loops followed in
//!    the surface's parameters (each edge's points projected), a point of
//!    an edge at a pole given the pole's border parameter and its
//!    neighbour's other one, so that the border's stretch between two
//!    edges at the pole is part of the loop; in where the loops wind
//!    round the point and it is more than `t` from every edge, near
//!    within `t` of one, not told where the loops cannot be followed.
//!
//! Lines `LAYERS …` (with `-- --nocapture`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::type_complexity,
    clippy::neg_cmp_op_on_partial_ord
)]
#![allow(dead_code)]

mod common;

use craft3d_doc::Document;
use craft3d_geom::offset::{OffsetError, regular};
use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Plane, RationalBezierPatch, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, Face, FaceId, VertexId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// The tip balls' radius, in tolerances.
const TIP: f64 = 4.0;
/// How near the search looks about a suspect, in tolerances.
const REACH: f64 = 8.0;
/// The cosine below which two normals stand against each other (154°).
const OPPOSITE: f64 = -0.9;

fn tolerance(linear: f64) -> Tolerance {
    Tolerance::new(linear, 1e-9).unwrap()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn nurbs_of(surface: &Surface) -> Option<NurbsSurface> {
    match surface {
        Surface::Nurbs(n) => Some(n.clone()),
        Surface::Bezier(b) => NurbsSurface::from_bezier(b).ok(),
        _ => None,
    }
}

/// One Bézier span as a surface of its own.
fn span_surface(patch: &RationalBezierPatch) -> Option<Surface> {
    let (m, n) = patch.degrees();
    let clamped = |d: usize| {
        let mut k = vec![0.0; d + 1];
        k.extend(vec![1.0; d + 1]);
        k
    };
    NurbsSurface::new(
        m,
        n,
        clamped(m),
        clamped(n),
        m + 1,
        n + 1,
        patch.control_points(),
        patch.weights(),
    )
    .ok()
    .map(Surface::Nurbs)
}

/// The point a border of the box `u × v` of `surface` is drawn into, and
/// which border (0: u start, 1: u end, 2: v start, 3: v end).
fn collapsed(surface: &Surface, u: (f64, f64), v: (f64, f64)) -> Vec<(usize, Vec3)> {
    let at = |a: f64, b: f64| surface.eval(a, b);
    let size = at(u.0, v.0)
        .distance(at(u.1, v.1))
        .max(at(u.0, v.1).distance(at(u.1, v.0)));
    let fine = 1e-9 * size.max(1e-300);
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

// ---------------------------------------------------------------------
// In the face, also on a surface with a pole.

/// The face's loops in its surface's parameters.
struct Chart {
    rings: Vec<Vec<(f64, f64)>>,
    /// Each edge's points in space, and how far its curve strays from
    /// the chords between them.
    edges: Vec<(Vec<Vec3>, f64)>,
    followed: Result<(), String>,
    poles: usize,
    /// How far the walk's chords may stray from the edges' feet, in
    /// parameters (measured at the middles of the chords).
    margin: f64,
    /// Per ring, per segment (from a point to the next): how far the
    /// walk's chord strays from the edge's feet at its middle.
    sags: Vec<Vec<f64>>,
}

const PER_EDGE: usize = 1024;

fn chart(body: &Body, face: &Face) -> Chart {
    let surface = &face.surface;
    let ((u0, u1), (v0, v1)) = surface.domain();
    let poles = collapsed(surface, (u0, u1), (v0, v1));
    let border_value = |k: usize| match k {
        0 => u0,
        1 => u1,
        2 => v0,
        _ => v1,
    };
    let size = surface
        .eval(u0, v0)
        .distance(surface.eval(u1, v1))
        .max(surface.eval(u0, v1).distance(surface.eval(u1, v0)));
    let mut out = Chart {
        rings: Vec::new(),
        edges: Vec::new(),
        followed: Ok(()),
        poles: poles.len(),
        margin: 0.0,
        sags: Vec::new(),
    };
    for id in &face.loops {
        let Some(lp) = body.loop_(*id) else {
            out.followed = Err("a loop is missing".into());
            return out;
        };
        let mut ring: Vec<(f64, f64)> = Vec::new();
        let mut ring_sags: Vec<f64> = Vec::new();
        for coedge in &lp.coedges {
            let Some(edge) = body.edge(coedge.edge) else {
                out.followed = Err("an edge is missing".into());
                return out;
            };
            let (t0, t1) = edge.range;
            let mut points = Vec::with_capacity(PER_EDGE + 1);
            for i in 0..=PER_EDGE {
                let s = f64::from(i as u32) / PER_EDGE as f64;
                let s = if coedge.reversed { 1.0 - s } else { s };
                points.push(edge.curve.eval(t0 + (t1 - t0) * s));
            }
            let mut sag = 0.0_f64;
            for i in 0..PER_EDGE {
                let s = (f64::from(i as u32) + 0.5) / PER_EDGE as f64;
                let s = if coedge.reversed { 1.0 - s } else { s };
                let middle = edge.curve.eval(t0 + (t1 - t0) * s);
                sag = sag.max(middle.distance((points[i] + points[i + 1]) * 0.5));
            }
            // Parameters, a point at a pole marked.
            let mut uvs: Vec<Option<(f64, f64)>> = Vec::with_capacity(points.len());
            let mut at_pole: Vec<Option<usize>> = Vec::with_capacity(points.len());
            for p in &points {
                let pole = poles
                    .iter()
                    .find(|(_, q)| q.distance(*p) <= 1e-9 * size.max(1e-300) + 1e-12)
                    .map(|(k, _)| *k);
                at_pole.push(pole);
                // Each point from the one before (the walk the region rule
                // makes): on a face that turns back on itself the nearest
                // foot may be on another layer.
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
            for i in 0..points.len() {
                let Some(k) = at_pole[i] else {
                    continue;
                };
                // The neighbour along the edge that is not at the pole.
                let neighbour = (1..points.len()).find_map(|d| {
                    [i.checked_sub(d), i.checked_add(d)]
                        .into_iter()
                        .flatten()
                        .find(|&j| j < points.len() && at_pole[j].is_none())
                        .and_then(|j| uvs[j])
                });
                if let Some((nu, nv)) = neighbour {
                    uvs[i] = Some(if k < 2 {
                        (border_value(k), nv)
                    } else {
                        (nu, border_value(k))
                    });
                }
            }
            let Some(firm) = uvs.into_iter().collect::<Option<Vec<(f64, f64)>>>() else {
                out.followed = Err("a point of an edge has no foot on the surface".into());
                return out;
            };
            let mut own_sags = vec![0.0_f64; firm.len()];
            for i in 0..PER_EDGE {
                let s = (f64::from(i as u32) + 0.5) / PER_EDGE as f64;
                let s = if coedge.reversed { 1.0 - s } else { s };
                let middle = edge.curve.eval(t0 + (t1 - t0) * s);
                if poles
                    .iter()
                    .any(|(_, q)| q.distance(middle) <= 1e-6 * size.max(1e-300))
                {
                    continue;
                }
                if let Ok(m) = surface.project_from(middle, firm[i]) {
                    let chord = (
                        0.5 * (firm[i].0 + firm[i + 1].0),
                        0.5 * (firm[i].1 + firm[i + 1].1),
                    );
                    let off = ((m.0 - chord.0) * (m.0 - chord.0)
                        + (m.1 - chord.1) * (m.1 - chord.1))
                        .sqrt();
                    if off.is_finite() {
                        out.margin = out.margin.max(off);
                        own_sags[i] = off;
                    }
                }
            }
            ring_sags.extend(own_sags);
            for uv in firm {
                if let Some(last) = ring.last() {
                    let jump_u = (uv.0 - last.0).abs() / (u1 - u0).abs().max(1e-300);
                    let jump_v = (uv.1 - last.1).abs() / (v1 - v0).abs().max(1e-300);
                    if (jump_u > 0.3 || jump_v > 0.3) && poles.is_empty() {
                        out.followed = Err(format!(
                            "the loop jumps by {jump_u:.2} and {jump_v:.2} of the domain (a seam)"
                        ));
                    }
                }
                ring.push(uv);
            }
            out.edges.push((points, sag));
        }
        out.rings.push(ring);
        out.sags.push(ring_sags);
    }
    out
}

fn winding(p: (f64, f64), ring: &[(f64, f64)]) -> i64 {
    let mut w = 0_i64;
    let n = ring.len();
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
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

fn to_segment(p: Vec3, a: Vec3, b: Vec3) -> f64 {
    let d = b - a;
    let l = d.length_squared();
    if !(l > 0.0) {
        return p.distance(a);
    }
    let s = ((p - a).dot(d) / l).clamp(0.0, 1.0);
    p.distance(a + d * s)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Held {
    In,
    Near,
    Out,
    NotTold,
}

fn to_segment_2d(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
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
    if chart.followed.is_err() {
        return Held::NotTold;
    }
    for (ring, sags) in chart.rings.iter().zip(&chart.sags) {
        let n = ring.len();
        for i in 0..n {
            let room = 2.0 * sags.get(i).copied().unwrap_or(chart.margin) + 1e-12;
            if to_segment_2d(uv, ring[i], ring[(i + 1) % n]) <= room {
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
// Suspects.

#[derive(Clone, Debug)]
struct Suspect {
    uv: (f64, f64),
    span: ((f64, f64), (f64, f64)),
    what: &'static str,
}

#[derive(Default, Clone, Debug)]
struct Suspects {
    spans: usize,
    proven: usize,
    poles: usize,
    places: Vec<Suspect>,
    tips: Vec<Vec3>,
}

fn suspects(surface: &Surface, outward: f64, t: f64) -> Suspects {
    let mut out = Suspects::default();
    let Some(nurbs) = nurbs_of(surface) else {
        return out;
    };
    let Ok(spans) = nurbs.bezier_spans() else {
        return out;
    };
    out.spans = spans.len();
    for span in &spans {
        let poles = collapsed(surface, span.u, span.v);
        for (_, p) in &poles {
            if out.tips.iter().all(|q| q.distance(*p) > t) {
                out.tips.push(*p);
            }
        }
        let middle = (0.5 * (span.u.0 + span.u.1), 0.5 * (span.v.0 + span.v.1));
        let global = |at: (f64, f64)| {
            (
                span.u.0 + (span.u.1 - span.u.0) * at.0,
                span.v.0 + (span.v.1 - span.v.0) * at.1,
            )
        };
        let Some(own) = span_surface(&span.patch) else {
            out.places.push(Suspect {
                uv: middle,
                span: (span.u, span.v),
                what: "no span",
            });
            continue;
        };
        match regular(&own, 0.5 * t * outward) {
            Ok(()) => out.proven += 1,
            Err(OffsetError::NoNormal { at }) => {
                if poles.is_empty() {
                    out.places.push(Suspect {
                        uv: global(at),
                        span: (span.u, span.v),
                        what: "no normal",
                    });
                } else {
                    out.poles += 1;
                }
            }
            Err(OffsetError::Folds { at, .. }) => out.places.push(Suspect {
                uv: global(at),
                span: (span.u, span.v),
                what: "concave",
            }),
            Err(OffsetError::Unproven { at }) => out.places.push(Suspect {
                uv: global(at),
                span: (span.u, span.v),
                what: "unproven",
            }),
            Err(_) => out.places.push(Suspect {
                uv: middle,
                span: (span.u, span.v),
                what: "other",
            }),
        }
    }
    // Degree one: the normal jumps at the knot lines.
    let (pu, pv) = nurbs.degrees();
    if pu == 1 || pv == 1 {
        let ((u0, u1), (v0, v1)) = nurbs.domain();
        let normal = |u: f64, v: f64| {
            let (_, a, b) = nurbs.derivatives(u, v);
            a.cross(b).normalized().map(|n| n * outward)
        };
        let mut lines_u: Vec<f64> = spans.iter().map(|s| s.u.0).filter(|u| *u > u0).collect();
        lines_u.sort_by(f64::total_cmp);
        lines_u.dedup();
        let mut lines_v: Vec<f64> = spans.iter().map(|s| s.v.0).filter(|v| *v > v0).collect();
        lines_v.sort_by(f64::total_cmp);
        lines_v.dedup();
        let eps_u = 1e-7 * (u1 - u0);
        let eps_v = 1e-7 * (v1 - v0);
        if pu == 1 {
            for &u in &lines_u {
                for i in 0..=64 {
                    let v = v0 + (v1 - v0) * f64::from(i) / 64.0;
                    if let (Some(a), Some(b)) = (normal(u - eps_u, v), normal(u + eps_u, v))
                        && a.dot(b) < 0.5
                    {
                        out.places.push(Suspect {
                            uv: (u, v),
                            span: ((u, u), (v, v)),
                            what: "crease",
                        });
                    }
                }
            }
        }
        if pv == 1 {
            for &v in &lines_v {
                for i in 0..=64 {
                    let u = u0 + (u1 - u0) * f64::from(i) / 64.0;
                    if let (Some(a), Some(b)) = (normal(u, v - eps_v), normal(u, v + eps_v))
                        && a.dot(b) < 0.5
                    {
                        out.places.push(Suspect {
                            uv: (u, v),
                            span: ((u, u), (v, v)),
                            what: "crease",
                        });
                    }
                }
            }
        }
    }
    out
}

/// The least concave radius over a span, away from the tips: a grid of
/// 24 × 24 over the span, then four rounds of a finer one about the best
/// place (the review's way to the worst place).
fn least_concave(
    surface: &Surface,
    outward: f64,
    span: ((f64, f64), (f64, f64)),
    tips: &[Vec3],
    t: f64,
) -> Option<(f64, f64)> {
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
    let ((u0, u1), (v0, v1)) = span;
    let mut best: Option<(f64, (f64, f64))> = None;
    let (mut cu, mut cv) = (0.5 * (u0 + u1), 0.5 * (v0 + v1));
    let (mut hu, mut hv) = (0.5 * (u1 - u0), 0.5 * (v1 - v0));
    for _ in 0..5 {
        let n = 24;
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
                if tips.iter().any(|tip| tip.distance(p) <= TIP * t) {
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

// ---------------------------------------------------------------------
// The layer search about a place.

#[derive(Default, Clone, Debug)]
struct Layers {
    points: usize,
    /// Pairs of the definition with both points in the face.
    shown: usize,
    least: f64,
    least_at: Vec3,
    /// Pairs whose points are near an edge, out, or not told.
    near: usize,
    out: usize,
    not_told: usize,
    /// Through material, within `t`, normals opposite (a blade).
    material: f64,
    crossings: usize,
    /// The least sine between the parameter directions in the box (no
    /// normal where it is below rounding).
    least_sine: f64,
}

fn segment_hits(p: Vec3, q: Vec3, a: Vec3, b: Vec3, c: Vec3) -> bool {
    let d = q - p;
    let e1 = b - a;
    let e2 = c - a;
    let h = d.cross(e2);
    let det = e1.dot(h);
    if !(det.abs() > 1e-14 * e1.length() * h.length()) {
        return false;
    }
    let inv = 1.0 / det;
    let s = p - a;
    let u = s.dot(h) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let r = s.cross(e1);
    let v = d.dot(r) * inv;
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    let w = e2.dot(r) * inv;
    (0.0..=1.0).contains(&w)
}

fn triangles_cross(a: [Vec3; 3], b: [Vec3; 3]) -> bool {
    (0..3).any(|i| segment_hits(a[i], a[(i + 1) % 3], b[0], b[1], b[2]))
        || (0..3).any(|i| segment_hits(b[i], b[(i + 1) % 3], a[0], a[1], a[2]))
}

fn layers(
    surface: &Surface,
    outward: f64,
    at: (f64, f64),
    t: f64,
    tips: &[Vec3],
    chart: &Chart,
) -> Layers {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let p0 = surface.eval(at.0, at.1);
    let reach = REACH * t;
    let n = 120_usize;
    let grid = |hu: f64, hv: f64, n: usize| -> Vec<((f64, f64), Vec3, Option<Vec3>)> {
        let mut found = Vec::with_capacity((n + 1) * (n + 1));
        for i in 0..=n {
            for j in 0..=n {
                let u = (at.0 + hu * (2.0 * i as f64 / n as f64 - 1.0)).clamp(u0, u1);
                let v = (at.1 + hv * (2.0 * j as f64 / n as f64 - 1.0)).clamp(v0, v1);
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
    let all = grid(hu, hv, n);
    let mut out = Layers {
        least: f64::INFINITY,
        material: f64::INFINITY,
        ..Layers::default()
    };
    // Points near the place, outside the tip balls, with a normal.
    let keep: Vec<usize> = (0..all.len())
        .filter(|&k| {
            let (_, p, m) = &all[k];
            m.is_some()
                && p.distance(p0) <= reach
                && !tips.iter().any(|tip| tip.distance(*p) <= TIP * t)
        })
        .collect();
    out.points = keep.len();
    let mut told: BTreeMap<usize, Held> = BTreeMap::new();
    out.least_sine = f64::INFINITY;
    for &k in &keep {
        told.insert(k, held(chart, all[k].0));
        let (_, a, b) = surface.derivatives(all[k].0.0, all[k].0.1);
        let size = a.length() * b.length();
        if size > 0.0 {
            out.least_sine = out.least_sine.min(a.cross(b).length() / size);
        }
    }
    // Pairs within t: a grid of cells t wide.
    let cell = |p: Vec3| {
        (
            (p.x / t).floor() as i64,
            (p.y / t).floor() as i64,
            (p.z / t).floor() as i64,
        )
    };
    let mut cells: BTreeMap<(i64, i64, i64), Vec<usize>> = BTreeMap::new();
    for &k in &keep {
        cells.entry(cell(all[k].1)).or_default().push(k);
    }
    for &k in &keep {
        let (uv_p, p, m) = all[k];
        let Some(m) = m else { continue };
        let c = cell(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(list) = cells.get(&(c.0 + dx, c.1 + dy, c.2 + dz)) else {
                        continue;
                    };
                    for &l in list {
                        if l <= k {
                            continue;
                        }
                        let (uv_q, q, other) = all[l];
                        let Some(other) = other else { continue };
                        let d = q - p;
                        let gap = d.length();
                        if !(gap < t) || !(m.dot(other) < OPPOSITE) {
                            continue;
                        }
                        let room = 1e-12 * p0.length().max(t);
                        if d.dot(m) > room && -d.dot(other) > room {
                            let _ = (uv_p, uv_q);
                            let held_p = told.get(&k).copied().unwrap_or(Held::NotTold);
                            let held_q = told.get(&l).copied().unwrap_or(Held::NotTold);
                            match (held_p, held_q) {
                                (Held::In, Held::In) => {
                                    out.shown += 1;
                                    if gap < out.least {
                                        out.least = gap;
                                        out.least_at = (p + q) * 0.5;
                                    }
                                }
                                (Held::NotTold, _) | (_, Held::NotTold) => out.not_told += 1,
                                (Held::Out, _) | (_, Held::Out) => out.out += 1,
                                _ => out.near += 1,
                            }
                        } else if d.dot(m) < -room && -d.dot(other) < -room {
                            out.material = out.material.min(gap);
                        }
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------
// A body.

#[derive(Default, Clone)]
struct Sum {
    faces: usize,
    freeform: usize,
    spans: usize,
    proven: usize,
    pole_spans: usize,
    suspects: BTreeMap<&'static str, usize>,
    searched: usize,
    shown_faces: usize,
    shown_pairs: usize,
    least: f64,
    crossing_faces: usize,
    not_told: usize,
    near: usize,
    charts_not_followed: usize,
    material: f64,
    least_sine: f64,
    suspect_ms: f64,
    search_ms: f64,
}

impl Sum {
    fn new() -> Sum {
        Sum {
            least: f64::INFINITY,
            material: f64::INFINITY,
            least_sine: f64::INFINITY,
            ..Sum::default()
        }
    }

    fn add(&mut self, o: &Sum) {
        self.faces += o.faces;
        self.freeform += o.freeform;
        self.spans += o.spans;
        self.proven += o.proven;
        self.pole_spans += o.pole_spans;
        for (k, v) in &o.suspects {
            *self.suspects.entry(*k).or_default() += *v;
        }
        self.searched += o.searched;
        self.shown_faces += o.shown_faces;
        self.shown_pairs += o.shown_pairs;
        self.least = self.least.min(o.least);
        self.crossing_faces += o.crossing_faces;
        self.not_told += o.not_told;
        self.near += o.near;
        self.charts_not_followed += o.charts_not_followed;
        self.material = self.material.min(o.material);
        self.least_sine = self.least_sine.min(o.least_sine);
        self.suspect_ms += o.suspect_ms;
        self.search_ms += o.search_ms;
    }

    fn said(&self, t: f64) -> String {
        format!(
            "faces {} freeform {} spans {} proven {} pole spans {} | suspects {:?}, {} searched | layers shown on {} faces ({} pairs, least {:.4} t) | least sine searched {:.2e} | pairs near an edge {}, not told {} | charts not followed {} | through material least {:.3} t | {:.1} ms suspects, {:.1} ms searches",
            self.faces,
            self.freeform,
            self.spans,
            self.proven,
            self.pole_spans,
            self.suspects,
            self.searched,
            self.shown_faces,
            self.shown_pairs,
            self.least / t,
            self.least_sine,
            self.near,
            self.not_told,
            self.charts_not_followed,
            self.material / t,
            self.suspect_ms,
            self.search_ms
        )
    }
}

/// The definition asked of every freeform face of `body` at `t`; each
/// face's findings printed under `tag` when it has any (`detail`).
fn rule(tag: &str, body: &Body, t: f64, detail: bool) -> Sum {
    let mut sum = Sum::new();
    for (index, face) in body.faces.iter().enumerate() {
        sum.faces += 1;
        if !matches!(face.surface, Surface::Nurbs(_) | Surface::Bezier(_)) {
            continue;
        }
        sum.freeform += 1;
        let outward = if face.reversed { -1.0 } else { 1.0 };
        let start = Instant::now();
        let mut found = suspects(&face.surface, outward, t);
        sum.suspect_ms += ms(start);
        sum.spans += found.spans;
        sum.proven += found.proven;
        sum.pole_spans += found.poles;
        if found.places.is_empty() {
            continue;
        }
        let start = Instant::now();
        for lp in &face.loops {
            if let Some(lp) = body.loop_(*lp) {
                for c in &lp.coedges {
                    if let Some(e) = body.edge(c.edge) {
                        for v in [e.start, e.end] {
                            if let Some(v) = body.vertex(v) {
                                found.tips.push(v.point);
                            }
                        }
                    }
                }
            }
        }
        let map = chart(body, face);
        if map.followed.is_err() {
            sum.charts_not_followed += 1;
        }
        // One search per cluster of suspects (2 t apart).
        let mut centres: Vec<(Suspect, Vec3)> = Vec::new();
        let mut starts: Vec<Suspect> = Vec::new();
        for s in &found.places {
            *sum.suspects.entry(s.what).or_default() += 1;
            starts.push(s.clone());
            if s.span.0.1 > s.span.0.0 && s.span.1.1 > s.span.1.0 {
                let best = least_concave(&face.surface, outward, s.span, &found.tips, t);
                if let Some(uv) = best {
                    starts.push(Suspect {
                        uv,
                        span: s.span,
                        what: "least concave",
                    });
                }
            }
        }
        for s in &starts {
            let p = face.surface.eval(s.uv.0, s.uv.1);
            if found.tips.iter().any(|tip| tip.distance(p) <= TIP * t) {
                *sum.suspects.entry("in a tip ball").or_default() += 1;
                continue;
            }
            if centres.iter().all(|(_, q)| q.distance(p) > 2.0 * t) && centres.len() < 200 {
                centres.push((s.clone(), p));
            }
        }
        let mut face_shown = 0_usize;
        let mut face_least = f64::INFINITY;
        let mut face_at = Vec3::ZERO;
        let mut face_crossings = 0_usize;
        let mut face_material = f64::INFINITY;
        for (s, _) in &centres {
            sum.searched += 1;
            let l = layers(&face.surface, outward, s.uv, t, &found.tips, &map);
            face_shown += l.shown;
            if l.least < face_least {
                face_least = l.least;
                face_at = l.least_at;
            }
            face_crossings += l.crossings;
            face_material = face_material.min(l.material);
            sum.not_told += l.not_told;
            sum.least_sine = sum.least_sine.min(l.least_sine);
            sum.near += l.near;
        }
        sum.search_ms += ms(start);
        sum.shown_pairs += face_shown;
        sum.material = sum.material.min(face_material);
        if face_shown > 0 {
            sum.shown_faces += 1;
            sum.least = sum.least.min(face_least);
        }
        if face_crossings > 0 {
            sum.crossing_faces += 1;
        }
        if detail {
            println!(
                "LAYERS {tag} | face {} ({} spans, {} poles, chart {}) | suspects {} searched {} | layers shown {} least {:.4} t at ({:.4}, {:.4}, {:.4}) | crossings {} | through material {:.3} t",
                FaceId(index as u32).0,
                found.spans,
                map.poles,
                match &map.followed {
                    Ok(()) => "followed".to_string(),
                    Err(why) => format!("not followed: {why}"),
                },
                found.places.len(),
                centres.len(),
                face_shown,
                face_least / t,
                face_at.x,
                face_at.y,
                face_at.z,
                face_crossings,
                face_material / t
            );
        }
    }
    sum
}

/// What the definition says of `body` at `t`, with the other checks'
/// verdicts beside it.
fn case(tag: &str, body: &Body, t: f64) {
    let tol = tolerance(t);
    let defects = craft3d_topo::check(body, &tol).len();
    let sum = rule(tag, body, t, true);
    println!(
        "LAYERS {tag} | t {t:e} | topo::check {defects} defects | {}",
        sum.said(t)
    );
}

// ---------------------------------------------------------------------
// The folds and the bodies that must pass.

fn read_step(path: &Path, t: f64) -> Option<Body> {
    let bytes = std::fs::read(path).ok()?;
    let tol = tolerance(t);
    let options = craft3d_stepread::Options {
        metres_per_unit: 1e-3,
        tolerance: tol,
        loosest: 1000.0 * tol.linear,
    };
    match craft3d_stepread::parse(&bytes).and_then(|p| p.build(0, &options)) {
        Ok(read) => Some(read.body),
        Err(e) => {
            println!("LAYERS {} | not read: {e}", path.display());
            None
        }
    }
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn the_seven_folded_fades() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fold_probe/fades");
    for name in [
        "fade-3-6-0.33",
        "fade-3-6-0.5",
        "fade-4-8-0.33",
        "fade-4-8-0.5",
        "fade-5-10-1",
        "fade-3-3-2",
        "fade-4-8-1",
    ] {
        if let Some(body) = read_step(&dir.join(format!("{name}.step")), 0.02) {
            case(name, &body, 0.02);
        }
    }
    // The hull it was cut from: must pass.
    let hull = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fold_probe/trimmed.step");
    if let Some(body) = read_step(&hull, 0.02) {
        case("the hull, no fillet", &body, 0.02);
    }
}

const N: usize = 13;

fn knots(degree: usize) -> Vec<f64> {
    let inner = N - degree - 1;
    let mut k = vec![0.0; degree + 1];
    for i in 1..=inner {
        k.push(i as f64 / (inner + 1) as f64);
    }
    k.extend(vec![1.0; degree + 1]);
    k
}

fn axis(l: f64, s: f64) -> [f64; N] {
    let c = 0.5 * l;
    let a = c - 4.0 * s;
    [
        0.0,
        0.5 * a,
        a,
        c - 3.0 * s,
        c - 2.0 * s,
        c - s,
        c,
        c + s,
        c + 2.0 * s,
        c + 3.0 * s,
        c + 4.0 * s,
        c + 4.0 * s + 0.5 * a,
        l,
    ]
}

const ROWS: [f64; N] = [
    0.0, 0.0, 0.0, 0.3, 0.7, 1.0, 1.0, 1.0, 0.7, 0.3, 0.0, 0.0, 0.0,
];

/// fold_probe's block, its top pulled through itself (`looped`) or into
/// an overhang, of degree `degree` (the review's).
fn roof_block(l: f64, s: f64, looped: bool, degree: usize) -> Body {
    let xs = axis(l, s);
    let h = l / 3.0;
    let (mut dx, mut dz) = ([0.0; N], [0.0; N]);
    if looped {
        dx[5] = 2.2 * s;
        dx[7] = -2.2 * s;
        dz[5] = 1.2 * s;
        dz[6] = 2.0 * s;
        dz[7] = 1.2 * s;
    } else {
        dx[5] = 1.6 * s;
        dx[7] = -1.6 * s;
        let rise = [0.0, 0.3, 0.7, 1.1, 1.5, 1.8, 2.0, 2.0, 2.0, 2.0];
        for (k, r) in rise.iter().enumerate() {
            dz[3 + k] = r * s;
        }
    }
    let mut net = Vec::with_capacity(N * N);
    for i in 0..N {
        for j in 0..N {
            let a = ROWS[j];
            net.push(Vec3::new(xs[i] + a * dx[i], xs[j], h + a * dz[i]));
        }
    }
    let surface = NurbsSurface::new(
        degree,
        degree,
        knots(degree),
        knots(degree),
        N,
        N,
        net.clone(),
        vec![1.0; N * N],
    )
    .unwrap();
    let at = |i: usize, j: usize| net[i * N + j];
    let mut body = Body::new(BodyKind::Solid);
    let ground = [(0.0, 0.0), (l, 0.0), (l, l), (0.0, l)];
    let low: Vec<VertexId> = ground
        .iter()
        .map(|&(x, y)| body.add_vertex(Vec3::new(x, y, 0.0)))
        .collect();
    let high: Vec<VertexId> = [(0, 0), (N - 1, 0), (N - 1, N - 1), (0, N - 1)]
        .iter()
        .map(|&(i, j)| body.add_vertex(at(i, j)))
        .collect();
    let spline = |points: Vec<Vec3>| {
        Curve3::Nurbs(NurbsCurve::polynomial(degree, knots(degree), points).unwrap())
    };
    let along_u = |j: usize| spline((0..N).map(|i| at(i, j)).collect());
    let along_v = |i: usize| spline((0..N).map(|j| at(i, j)).collect());
    let roof = [
        (
            body.add_edge(along_u(0), (0.0, 1.0), high[0], high[1]),
            true,
        ),
        (
            body.add_edge(along_v(N - 1), (0.0, 1.0), high[1], high[2]),
            true,
        ),
        (
            body.add_edge(along_u(N - 1), (0.0, 1.0), high[3], high[2]),
            false,
        ),
        (
            body.add_edge(along_v(0), (0.0, 1.0), high[0], high[3]),
            false,
        ),
    ];
    let straight = |body: &mut Body, from: VertexId, to: VertexId| -> EdgeId {
        let (a, b) = (
            body.vertex(from).unwrap().point,
            body.vertex(to).unwrap().point,
        );
        body.add_edge(
            Curve3::Line(Line::through(a, b).unwrap()),
            (0.0, a.distance(b)),
            from,
            to,
        )
    };
    let floor = [
        straight(&mut body, low[0], low[1]),
        straight(&mut body, low[1], low[2]),
        straight(&mut body, low[2], low[3]),
        straight(&mut body, low[3], low[0]),
    ];
    let up = [
        straight(&mut body, low[0], high[0]),
        straight(&mut body, low[1], high[1]),
        straight(&mut body, low[2], high[2]),
        straight(&mut body, low[3], high[3]),
    ];
    let coedge = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
    let mut faces: Vec<FaceId> = Vec::new();
    for k in 0..4 {
        let next = (k + 1) % 4;
        let lp = body.add_loop(vec![
            coedge(floor[k], false),
            coedge(up[next], false),
            coedge(roof[k].0, roof[k].1),
            coedge(up[k], true),
        ]);
        let start = Vec3::new(ground[k].0, ground[k].1, 0.0);
        let direction = Vec3::new(ground[next].0, ground[next].1, 0.0) - start;
        let frame = Frame::new(start, direction.cross(Vec3::Z), direction).unwrap();
        faces.push(body.add_face(Surface::Plane(Plane::new(frame)), false, vec![lp]));
    }
    let floor_loop = body.add_loop(floor.iter().rev().map(|&e| coedge(e, true)).collect());
    let floor_frame = Frame::new(Vec3::ZERO, -Vec3::Z, Vec3::X).unwrap();
    faces.push(body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    ));
    let roof_loop = body.add_loop(roof.iter().map(|&(e, along)| coedge(e, !along)).collect());
    faces.push(body.add_face(Surface::Nurbs(surface), false, vec![roof_loop]));
    body.add_shell(faces);
    body
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn tops_pulled_through_themselves_and_an_overhang() {
    for (looped, degree) in [(true, 3), (true, 1), (false, 3), (false, 1)] {
        for times in [3.0, 100.0, 10_000.0] {
            let tag = format!(
                "top {} degree {degree} x{times}",
                if looped { "looped" } else { "overhang" }
            );
            let body = roof_block(times * 6.0, 1.0, looped, degree);
            case(&tag, &body, 1e-3);
            // A global layer search over the top without a suspect: the
            // points a grid of t / 4 would need.
            if let Ok(m) = craft3d_topo::mass_properties(&body) {
                println!(
                    "LAYERS {tag} | a search without suspects over the whole body at t / 4: about {:.3e} points (area {:.3e})",
                    m.area / (0.25e-3 * 0.25e-3),
                    m.area
                );
            }
        }
    }
}

/// A new document at `linear`, the commands run, its bodies (the
/// review's).
fn script(tag: &str, linear: f64, steps: &[(&str, Value)]) -> Vec<Arc<Body>> {
    let dir = common::workdir("fold-layers", tag);
    let mut s = common::session_in(&dir);
    if let Err(e) = s.execute("file.new", &json!({"units": "mm", "tolerance": linear})) {
        println!("LAYERS {tag} | file.new refused: {e}");
        return Vec::new();
    }
    for (id, params) in steps {
        match s.execute(id, params) {
            Ok(value) => {
                if value.get("status").and_then(Value::as_str) == Some("failed") {
                    println!(
                        "LAYERS {tag} | {id} failed: {}",
                        value.get("error").map(Value::to_string).unwrap_or_default()
                    );
                }
            }
            Err(e) => println!("LAYERS {tag} | {id} refused: {e}"),
        }
    }
    let Some(Ok(doc)) = s.document_bytes().map(|b| craft3d_format::from_bytes(&b)) else {
        println!("LAYERS {tag} | no document");
        return Vec::new();
    };
    let regen = craft3d_regen::regenerate(&doc);
    regen.bodies().map(|(_, b)| b.clone()).collect()
}

/// The review's notch: a block whose top is one cubic spline, its root a
/// smooth inside turn.
fn notch(tag: &str, linear: f64, control: &[[f64; 2]], k: f64) {
    let control: Vec<[f64; 2]> = control.iter().map(|p| [p[0] * k, p[1] * k]).collect();
    let line = |a: [f64; 2], b: [f64; 2]| {
        (
            "sketch.line",
            json!({"sketch": "P", "from": [a[0] * k, a[1] * k], "to": [b[0] * k, b[1] * k]}),
        )
    };
    let bodies = script(
        tag,
        linear,
        &[
            ("sketch.create", json!({"name": "P"})),
            (
                "sketch.spline",
                json!({"sketch": "P", "control": control, "degree": 3}),
            ),
            line([0.0, 30.0], [0.0, 0.0]),
            line([0.0, 0.0], [40.0, 0.0]),
            line([40.0, 0.0], [40.0, 30.0]),
            (
                "feature.extrude",
                json!({"sketch": "P", "distance": 10.0 * k, "name": "B"}),
            ),
        ],
    );
    for body in &bodies {
        case(tag, body, linear);
    }
}

fn right_angle(e: f64) -> Vec<[f64; 2]> {
    vec![
        [40.0, 30.0],
        [32.0, 30.0],
        [20.0 + e, 20.0 + e],
        [20.0, 20.0],
        [20.0 - e, 20.0 + e],
        [8.0, 30.0],
        [0.0, 30.0],
    ]
}

fn narrow(e: f64) -> Vec<[f64; 2]> {
    vec![
        [40.0, 30.0],
        [30.0, 30.0],
        [22.68, 30.0],
        [20.0 + 0.259 * e, 20.0 + 0.966 * e],
        [20.0, 20.0],
        [20.0 - 0.259 * e, 20.0 + 0.966 * e],
        [17.32, 30.0],
        [10.0, 30.0],
        [0.0, 30.0],
    ]
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn notches_that_must_pass() {
    for e in [0.2, 0.05, 0.02] {
        notch(&format!("notch-90 e{e}"), 0.02, &right_angle(e), 1.0);
    }
    notch("notch-90 small", 0.02e-3, &right_angle(0.05), 1e-3);
    notch("notch-90 large", 0.02e4, &right_angle(0.05), 1e4);
    for e in [0.2, 0.05] {
        notch(&format!("notch-30 e{e}"), 0.02, &narrow(e), 1.0);
    }
}

/// The review's L with its inside edge rounded by fades the kernel
/// builds (its own refusal on): must pass.
#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn fades_on_a_concave_edge_that_must_pass() {
    use craft3d_kernel::fillet::{Fade, FilletLawEdge, RadiusLaw, fillet_by_law};
    let linear = 0.02;
    let line =
        |a: [f64; 2], b: [f64; 2]| ("sketch.line", json!({"sketch": "L", "from": a, "to": b}));
    let bodies = script(
        "concave-L",
        linear,
        &[
            ("sketch.create", json!({"name": "L"})),
            line([0.0, 0.0], [40.0, 0.0]),
            line([40.0, 0.0], [40.0, 20.0]),
            line([40.0, 20.0], [20.0, 20.0]),
            line([20.0, 20.0], [20.0, 40.0]),
            line([20.0, 40.0], [0.0, 40.0]),
            line([0.0, 40.0], [0.0, 0.0]),
            (
                "feature.extrude",
                json!({"sketch": "L", "distance": 30, "name": "B"}),
            ),
        ],
    );
    let Some(body) = bodies.first() else {
        println!("LAYERS concave-L | no body");
        return;
    };
    let tol = tolerance(linear);
    let mut edge = None;
    for (i, e) in body.edges.iter().enumerate() {
        let (a, b) = (
            body.vertices[e.start.0 as usize].point,
            body.vertices[e.end.0 as usize].point,
        );
        let on = |p: Vec3| (p.x - 20.0).abs() < 1e-6 && (p.y - 20.0).abs() < 1e-6;
        if on(a) && on(b) && (a.z - b.z).abs() > 1.0 {
            edge = Some(EdgeId(i as u32));
        }
    }
    let Some(edge) = edge else {
        println!("LAYERS concave-L | inside edge not found");
        return;
    };
    for (radius, over, back) in [
        (3.0, 6.0, 0.33),
        (3.0, 6.0, 0.0),
        (2.0, 10.0, 0.5),
        (3.0, 3.0, 2.0),
        (3.0, 3.0, 0.0),
        (5.0, 5.0, 0.5),
        (5.0, 3.0, 0.5),
    ] {
        let tag = format!("concave-L fade r{radius} over{over} back{back}");
        let law = RadiusLaw {
            radius,
            fades: [Some(Fade::tip(over, back)), None],
        };
        match fillet_by_law(body, &[FilletLawEdge { edge, law }], &tol) {
            Ok(made) => case(&tag, &made.body, linear),
            Err(e) => println!("LAYERS {tag} | refused by the fillet: {e}"),
        }
    }
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn a_loft_through_its_axis() {
    use craft3d_kernel::loft::{Options as LoftOptions, Section, loft};
    use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
    let arcs = |first: u64, r: f64| -> Vec<SketchCurve> {
        let points = [[r, 0.0], [0.0, r], [-r, 0.0], [0.0, -r]];
        (0..4)
            .map(|k| SketchCurve {
                id: first + k as u64,
                curve: Curve2::Arc {
                    center: [0.0, 0.0],
                    start: points[k],
                    end: points[(k + 1) % 4],
                },
            })
            .collect()
    };
    for k in [1.0, 10_000.0] {
        let tol = tolerance(1e-6 * k);
        let low = detect_profiles(&arcs(1, k), &tol).unwrap();
        let high = detect_profiles(&arcs(11, 1.2 * k), &tol).unwrap();
        let sections = [
            Section {
                plane: Frame::WORLD,
                outline: low[0].outer.clone(),
                start: None,
            },
            Section {
                plane: Frame::new(Vec3::new(0.0, 0.0, 2.0 * k), Vec3::Z, Vec3::X).unwrap(),
                outline: high[0].outer.clone(),
                start: Some(13),
            },
        ];
        match loft(
            &sections,
            &LoftOptions {
                ruled: true,
                closed: false,
            },
            &tol,
        ) {
            Ok(made) => case(&format!("loft through its axis x{k}"), &made.body, 1e-6 * k),
            Err(e) => println!("LAYERS loft x{k} | refused: {e}"),
        }
    }
}

// ---------------------------------------------------------------------
// The repository's documents and scripts, at their own tolerance and at
// a coarse one.

fn steps(path: &Path) -> Vec<(String, Value)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let list = match &value {
        Value::Array(list) => list.clone(),
        Value::Object(map) => map
            .get("steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    list.iter()
        .filter_map(|step| match step {
            Value::Array(pair) => Some((pair.first()?.as_str()?.to_string(), pair.get(1)?.clone())),
            Value::Object(map) => Some((
                map.get("command")?.as_str()?.to_string(),
                map.get("params").cloned().unwrap_or(json!({})),
            )),
            _ => None,
        })
        .collect()
}

fn listed(folder: &str, suffix: &str) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(root().join(folder)) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = dir
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(suffix))
        .collect();
    found.sort();
    found
}

thread_local! {
    /// The rule as built (`craft3d_topo::folds`), per test thread: at the
    /// documents' own tolerance and at 0.01: warnings, places not proven,
    /// milliseconds cold, bodies, milliseconds warm (asked again at once).
    static RULE: std::cell::RefCell<[(usize, usize, f64, usize, f64); 2]> =
        const { std::cell::RefCell::new([(0, 0, 0.0, 0, 0.0); 2]) };
}

/// Every own body of `doc` asked by the rule as built, at its tolerance
/// and at `coarse` (when that is coarser), cold for each document;
/// documents with a place printed; and the regeneration's time.
fn document(name: &str, doc: &Document, coarse: f64, totals: &mut [Sum; 2], regen_ms: &mut f64) {
    let _ = totals;
    craft3d_topo::forget_folds();
    let start = Instant::now();
    let regen = craft3d_regen::regenerate(doc);
    let regenerated = ms(start);
    *regen_ms += regenerated;
    let own = doc.tolerance.linear;
    for (k, t) in [own, coarse.max(own)].into_iter().enumerate() {
        if k == 1 && !(coarse > own) {
            continue;
        }
        let tol = tolerance(t);
        let start = Instant::now();
        let mut found = Vec::new();
        let mut bodies = 0_usize;
        for (_, body) in regen.bodies() {
            bodies += 1;
            found.extend(craft3d_topo::folds(body, &tol));
        }
        let spent = ms(start);
        let start = Instant::now();
        for (_, body) in regen.bodies() {
            let _ = craft3d_topo::folds(body, &tol);
        }
        let warm = ms(start);
        let warnings = found.iter().filter(|f| f.is_fold()).count();
        let unproven = found.len() - warnings;
        RULE.with(|r| {
            let mut r = r.borrow_mut();
            r[k].0 += warnings;
            r[k].1 += unproven;
            r[k].2 += spent;
            r[k].3 += bodies;
            r[k].4 += warm;
        });
        if !found.is_empty() || spent > 50.0 || spent > 0.05 * regenerated.max(20.0) {
            println!(
                "LAYERS rule {name} | t {t:e} | {warnings} warnings, {unproven} not proven | rule {spent:.1} ms cold, {warm:.2} ms warm, regeneration {regenerated:.1} ms, {bodies} bodies | first: {}",
                found.first().map(ToString::to_string).unwrap_or_default()
            );
        }
    }
}

fn corpus(tag: &str, files: &[&str], scripts: &[(&str, &str)]) {
    let mut totals = [Sum::new(), Sum::new()];
    let mut regen_ms = 0.0;
    let mut documents = 0_usize;
    for folder in files {
        for path in listed(folder, ".3dcraft") {
            let name = format!("{folder}/{}", path.file_name().unwrap().to_string_lossy());
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if let Ok(doc) = craft3d_format::from_bytes(&bytes) {
                documents += 1;
                document(&name, &doc, 0.01, &mut totals, &mut regen_ms);
            }
        }
    }
    let dir = common::workdir("fold-layers", tag);
    for (folder, suffix) in scripts {
        for path in listed(folder, suffix) {
            let name = format!("{folder}/{}", path.file_name().unwrap().to_string_lossy());
            let list = steps(&path);
            if list.is_empty() {
                continue;
            }
            let mut s = common::session_in(&dir);
            if list.first().is_none_or(|(id, _)| id != "file.new") {
                let _ = s.execute("file.new", &json!({"units": "mm"}));
            }
            for (id, params) in list {
                if (id.starts_with("file.")
                    || id.starts_with("view.")
                    || id.starts_with("ui.")
                    || id.starts_with("assembly.")
                    || id == "document.check")
                    && id != "file.new"
                {
                    continue;
                }
                let _ = s.execute(&id, &params);
            }
            if let Some(Ok(doc)) = s.document_bytes().map(|b| craft3d_format::from_bytes(&b)) {
                documents += 1;
                document(&name, &doc, 0.01, &mut totals, &mut regen_ms);
            }
        }
    }
    let mut line =
        format!("LAYERS corpus tally {tag}: {documents} documents, regeneration {regen_ms:.0} ms");
    RULE.with(|r| {
        let r = r.borrow();
        for (k, what) in [(0, "own tolerance"), (1, "0.01 where coarser")] {
            let _ = write!(
                line,
                " | rule at {what}: {} warnings, {} not proven, {:.0} ms cold over {} bodies ({:.2} % of the regeneration), {:.1} ms warm",
                r[k].0,
                r[k].1,
                r[k].2,
                r[k].3,
                100.0 * r[k].2 / regen_ms.max(1e-9),
                r[k].4
            );
        }
    });
    println!("{line}");
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn corpus_files() {
    corpus(
        "files",
        &[
            "docs/examples/published",
            "docs/showcase",
            "crates/engine/tests/fillet_fade",
            "crates/engine/tests/stale_sketches",
            "crates/format/tests/fixtures/versions",
            "crates/format/tests/fixtures/migrations",
        ],
        &[],
    );
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn corpus_scripts() {
    corpus(
        "scripts",
        &[],
        &[
            ("docs/examples", ".json"),
            ("docs/tasks", ".json"),
            ("crates/engine/tests/example_models", ".json"),
            ("crates/engine/tests/bench_models", ".steps.json"),
            ("crates/engine/tests/edit_path", ".steps.json"),
            ("crates/engine/tests/self_check", ".json"),
            ("crates/engine/tests/blend_findings", ".json"),
            ("crates/engine/tests/fuzz_findings", ".json"),
            ("crates/engine/tests/kayak_findings", ".json"),
            ("crates/engine/tests/boolean_findings", ".json"),
        ],
    );
}

/// The repository's documents whose own tolerance is coarse (the
/// review's four), each body with detail.
#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn coarse_documents_in_detail() {
    for name in [
        "docs/examples/published/ship-propeller.3dcraft",
        "docs/showcase/rotor.3dcraft",
        "crates/engine/tests/fillet_fade/hull.3dcraft",
        "crates/format/tests/fixtures/versions/v59.3dcraft",
    ] {
        let Ok(bytes) = std::fs::read(root().join(name)) else {
            continue;
        };
        let Ok(doc) = craft3d_format::from_bytes(&bytes) else {
            continue;
        };
        let t = doc.tolerance.linear;
        let regen = craft3d_regen::regenerate(&doc);
        let mut sum = Sum::new();
        for (_, body) in regen.bodies() {
            sum.add(&rule(name, body, t, true));
        }
        println!("LAYERS {name} | t {t:e} | {}", sum.said(t));
    }
}

/// Every 40th model of the ABC cache on the build server, at 1e-6 and at
/// its working tolerance (the review's way of reading them).
#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn abc_models() {
    let Some(folder) = std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join("3dcraft-abc"))
        .filter(|f| f.is_dir())
    else {
        println!("LAYERS abc | no cache on this machine: not checked");
        return;
    };
    let mut files: Vec<PathBuf> = Vec::new();
    let mut stack = vec![(folder, 0_u32)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 6 || files.len() > 200_000 {
            continue;
        }
        let Ok(list) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in list.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                stack.push((path, depth + 1));
            } else if kind.is_file() {
                let name = path.to_string_lossy().to_lowercase();
                if name.ends_with(".step") || name.ends_with(".stp") {
                    files.push(path);
                }
            }
        }
    }
    files.sort();
    let started = Instant::now();
    let stride = (files.len() / 500).max(1);
    let (mut asked, mut read) = (0_usize, 0_usize);
    let mut totals = [Sum::new(), Sum::new()];
    for path in files.iter().step_by(stride) {
        if started.elapsed().as_secs() > 480 {
            break;
        }
        if !std::fs::metadata(path).is_ok_and(|m| m.len() <= 600_000) {
            continue;
        }
        asked += 1;
        let Some(body) = read_step(path, 1e-6) else {
            continue;
        };
        read += 1;
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let start = Instant::now();
        let found = craft3d_topo::folds(&body, &tolerance(1e-6));
        let spent = ms(start);
        let warned = found.iter().filter(|f| f.is_fold()).count();
        RULE.with(|r| {
            let mut r = r.borrow_mut();
            r[0].0 += warned;
            r[0].1 += found.len() - warned;
            r[0].2 += spent;
            r[0].3 += 1;
        });
        if !found.is_empty() {
            println!(
                "LAYERS rule abc {name} | {} places | first: {}",
                found.len(),
                found[0]
            );
        }
        let sum = rule(&format!("abc {name}"), &body, 1e-6, false);
        if sum.shown_faces > 0 || sum.crossing_faces > 0 {
            println!("LAYERS abc {name} | t 1e-6 | {}", sum.said(1e-6));
        }
        totals[0].add(&sum);
        let working = body.working_tolerance(1e-6);
        if working > 1e-6 {
            let sum = rule(&format!("abc {name} loose"), &body, working, false);
            if sum.shown_faces > 0 || sum.crossing_faces > 0 {
                println!(
                    "LAYERS abc {name} | t {working:e} (its working tolerance) | {}",
                    sum.said(working)
                );
            }
            totals[1].add(&sum);
        }
    }
    println!(
        "LAYERS abc tally: {asked} asked (every {stride}th of {}), {read} read, {:.0} s | the rule at 1e-6: {:?} | the probe at 1e-6: {} | loose ones at their working tolerance: {}",
        files.len(),
        started.elapsed().as_secs_f64(),
        RULE.with(|r| r.borrow()[0]),
        totals[0].said(1.0),
        totals[1].said(1.0)
    );
}
