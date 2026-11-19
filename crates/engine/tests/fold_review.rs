//! Ignored in the gate (a probe: it prints and holds nothing; fold-check
//! 2026-11-19, after the review): run it with
//! `cargo test -p craft3d-engine --test fold_review -- --ignored --nocapture`.
//!
//! review-fold-design: a reviewer's probes of the design *A face folded
//! over itself* (docs/design/verification.md), printed and held to
//! nothing. The rule is asked here in the form the design defines it, as
//! near as a probe gets without building it: per Bézier span the offset
//! out of the material (the concave side) by half the tolerance and by
//! the whole of it (`geom::offset::regular`), then, on every span the
//! whole tolerance does not prove, a dense sample of the concave radius,
//! each sample told in or out of the face (`topo::face_holds`) and near
//! or away from a tip (four tolerances about the face's vertices and
//! about a border of a span drawn into a point). For the worst place of a
//! face: how far the normal turns about it, and whether two layers of the
//! face stand opposite each other there. Lines `REVIEW …` (with
//! `-- --nocapture`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
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
use craft3d_io::{StepBody, StepOptions, export_step};
use craft3d_kernel::fillet::{Fade, FilletLawEdge, RadiusLaw, fillet_by_law};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{SelfCheckParams, body_self_intersections};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId, check, mass_properties};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// The tip balls' radius, in tolerances.
const TIP: f64 = 4.0;

struct Fnv(u64);

impl std::fmt::Write for Fnv {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        for b in s.bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Ok(())
    }
}

fn hash(value: &impl std::fmt::Debug) -> u64 {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    write!(h, "{value:?}").unwrap();
    h.0
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tolerance(linear: f64) -> Tolerance {
    Tolerance::new(linear, 1e-9).unwrap()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
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

/// The least concave radius of curvature at `(u, v)` (`sign` is 1 for a
/// face whose outward side is its surface's normal, −1 for a reversed
/// one): infinite where nothing bends towards the outside, `None` where
/// the surface has no normal.
fn concave(surface: &Surface, sign: f64, u: f64, v: f64) -> Option<f64> {
    let (k, h) = craft3d_geom::curvature::curvatures(surface, u, v)?;
    let root = (h * h - k).max(0.0).sqrt();
    let towards = (sign * (h + root)).max(sign * (h - root));
    Some(if towards > 0.0 {
        1.0 / towards
    } else {
        f64::INFINITY
    })
}

/// The least concave radius near `at`, looked for on ever finer grids
/// about the best place so far, away from the tips.
fn refined(
    surface: &Surface,
    sign: f64,
    at: (f64, f64),
    half: (f64, f64),
    tips: &[Vec3],
    t: f64,
) -> (f64, (f64, f64)) {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let mut best = (
        concave(surface, sign, at.0, at.1).unwrap_or(f64::INFINITY),
        at,
    );
    let (mut hu, mut hv) = half;
    for _ in 0..6 {
        let centre = best.1;
        let n = 24;
        for i in 0..=n {
            for j in 0..=n {
                let u = (centre.0 + hu * (2.0 * i as f64 / n as f64 - 1.0)).clamp(u0, u1);
                let v = (centre.1 + hv * (2.0 * j as f64 / n as f64 - 1.0)).clamp(v0, v1);
                let Some(r) = concave(surface, sign, u, v) else {
                    continue;
                };
                if !(r < best.0) {
                    continue;
                }
                let p = surface.eval(u, v);
                if tips.iter().any(|tip| tip.distance(p) <= TIP * t) {
                    continue;
                }
                best = (r, (u, v));
            }
        }
        hu /= 6.0;
        hv /= 6.0;
    }
    best
}

/// The point a border of the span `u × v` of `surface` is drawn into, if
/// one is: a pole.
fn collapsed(surface: &Surface, u: (f64, f64), v: (f64, f64), t: f64) -> Option<Vec3> {
    let at = |a: f64, b: f64| surface.eval(a, b);
    let size = at(u.0, v.0)
        .distance(at(u.1, v.1))
        .max(at(u.0, v.1).distance(at(u.1, v.0)));
    let fine = 1e-9 * size.max(t);
    let (um, vm) = (0.5 * (u.0 + u.1), 0.5 * (v.0 + v.1));
    for (p, q, m) in [
        (at(u.0, v.0), at(u.0, v.1), at(u.0, vm)),
        (at(u.1, v.0), at(u.1, v.1), at(u.1, vm)),
        (at(u.0, v.0), at(u.1, v.0), at(um, v.0)),
        (at(u.0, v.1), at(u.1, v.1), at(um, v.1)),
    ] {
        if p.distance(q) <= fine && p.distance(m) <= fine {
            return Some(p);
        }
    }
    None
}

/// About a place of a face, over the points of the face within eight
/// tolerances of it and outside the tip balls (a grid of 121 × 121 over a
/// box of the parameters halved until it stays that near): how far the
/// outward normal turns within 1, 2 and 8 tolerances (the least cosine
/// against the normal at the place: −1 is turned right round), and the
/// least distance between two points whose normals stand against each
/// other, told apart by what lies between them: air (each on the other's
/// outward side: two layers of a fold out of the material; with the
/// normals past 120° and past 154° apart) or material (a blade); and how
/// far from the place two layers past 154° through air stay within the
/// tolerance of each other.
fn turn(surface: &Surface, sign: f64, at: (f64, f64), t: f64, tips: &[Vec3]) -> String {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let (p0, du, dv) = surface.derivatives(at.0, at.1);
    let Some(n0) = du.cross(dv).normalized() else {
        return "no normal at the place".to_string();
    };
    let n0 = n0 * sign;
    let reach = 8.0 * t;
    let grid = |hu: f64, hv: f64, n: usize| -> Vec<(Vec3, Vec3)> {
        let mut found = Vec::with_capacity((n + 1) * (n + 1));
        for i in 0..=n {
            for j in 0..=n {
                let u = (at.0 + hu * (2.0 * i as f64 / n as f64 - 1.0)).clamp(u0, u1);
                let v = (at.1 + hv * (2.0 * j as f64 / n as f64 - 1.0)).clamp(v0, v1);
                let (p, a, b) = surface.derivatives(u, v);
                let m = a.cross(b).normalized().map_or(Vec3::ZERO, |m| m * sign);
                found.push((p, m));
            }
        }
        found
    };
    let far = |hu: f64, hv: f64| -> f64 {
        grid(hu, hv, 6)
            .iter()
            .map(|(p, _)| p.distance(p0))
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
    let near: Vec<(Vec3, Vec3)> = grid(hu, hv, 120)
        .into_iter()
        .filter(|(p, m)| {
            p.distance(p0) <= reach
                && *m != Vec3::ZERO
                && !tips.iter().any(|tip| tip.distance(*p) <= TIP * t)
        })
        .collect();
    let mut said = format!(
        "a box {hu:.2e} by {hv:.2e} of the parameters, {} points within 8 t;",
        near.len()
    );
    for rho in [1.0, 2.0, 8.0] {
        let least = near
            .iter()
            .filter(|(p, _)| p.distance(p0) <= rho * t)
            .map(|(_, m)| m.dot(n0))
            .fold(1.0_f64, f64::min);
        let _ = write!(said, " cos within {rho} t {least:.3};");
    }
    let (mut air, mut air_opposite, mut material) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut together = 0.0_f64;
    for (i, &(p, m)) in near.iter().enumerate() {
        for &(q, k) in &near[i + 1..] {
            let c = m.dot(k);
            if !(c < -0.5) {
                continue;
            }
            let d = q - p;
            let gap = d.length();
            if d.dot(m) > 0.0 && d.dot(k) < 0.0 {
                air = air.min(gap);
                if c < -0.9 {
                    air_opposite = air_opposite.min(gap);
                    if gap <= t {
                        together = together.max(p.distance(p0).max(q.distance(p0)));
                    }
                }
            } else if d.dot(m) < 0.0 && d.dot(k) > 0.0 {
                material = material.min(gap);
            }
        }
    }
    let _ = write!(
        said,
        " layers through air {:.3} t apart (normals past 120 degrees), {:.3} t (past 154), those within t of each other out to {:.1} t from the place; through material {:.3} t",
        air / t,
        air_opposite / t,
        together / t,
        material / t
    );
    said
}

/// What the rule finds on a body.
#[derive(Default, Clone)]
struct Sum {
    faces: usize,
    freeform: usize,
    spans: usize,
    /// Spans the offset out of the material by the whole tolerance proves.
    proven_full: usize,
    /// What the offset by half the tolerance says of the spans.
    half: BTreeMap<&'static str, usize>,
    /// Samples away from the tips, in the face / not told / out of it:
    /// concave below half the tolerance; between half and the whole; no
    /// normal.
    away: [[usize; 3]; 3],
    /// The same three kinds within a tip ball.
    near_tip: [usize; 3],
    analytic_half: usize,
    analytic_full: usize,
    /// The least concave radius found away from the tips, in tolerances.
    least: f64,
    nanos_half: u128,
    nanos_full: u128,
    slowest: u128,
}

impl Sum {
    fn new() -> Self {
        Sum {
            least: f64::INFINITY,
            ..Sum::default()
        }
    }

    fn add(&mut self, other: &Sum) {
        self.faces += other.faces;
        self.freeform += other.freeform;
        self.spans += other.spans;
        self.proven_full += other.proven_full;
        for (k, v) in &other.half {
            *self.half.entry(*k).or_default() += *v;
        }
        for a in 0..3 {
            for b in 0..3 {
                self.away[a][b] += other.away[a][b];
            }
            self.near_tip[a] += other.near_tip[a];
        }
        self.analytic_half += other.analytic_half;
        self.analytic_full += other.analytic_full;
        self.least = self.least.min(other.least);
        self.nanos_half += other.nanos_half;
        self.nanos_full += other.nanos_full;
        self.slowest = self.slowest.max(other.slowest);
    }

    fn said(&self) -> String {
        format!(
            "faces {} freeform {} spans {} | concave at t: {} spans proven | at t/2: {:?} | samples away from tips [in, not told, out]: below t/2 {:?}, t/2 to t {:?}, no normal {:?} | within a tip ball [below t/2, to t, no normal] {:?} | analytic concave below t/2 {} below t {} | least concave radius away from tips {:.4} t | the rule at t/2 {:.1} ms, at t {:.1} ms, slowest span {:.2} ms",
            self.faces,
            self.freeform,
            self.spans,
            self.proven_full,
            self.half,
            self.away[0],
            self.away[1],
            self.away[2],
            self.near_tip,
            self.analytic_half,
            self.analytic_full,
            self.least,
            self.nanos_half as f64 / 1e6,
            self.nanos_full as f64 / 1e6,
            self.slowest as f64 / 1e6
        )
    }
}

/// The rule asked of every face of `body` at the tolerance `t`; a line per
/// face it has something to say of, the turn of the normal at the worst
/// place of up to `turns` faces. `seen` keeps the faces asked before (by
/// surface, sense and vertices).
fn rule(
    tag: &str,
    body: &Body,
    tol: &Tolerance,
    t: f64,
    turns: usize,
    seen: &mut BTreeMap<u64, ()>,
) -> Sum {
    let mut sum = Sum::new();
    let mut turns_left = turns;
    // Spans sampled on this body so far: at most 3000.
    let mut sampled = 0_usize;
    for (index, face) in body.faces.iter().enumerate() {
        let mut corners: Vec<Vec3> = Vec::new();
        for lp in &face.loops {
            if let Some(lp) = body.loop_(*lp) {
                for coedge in &lp.coedges {
                    if let Some(edge) = body.edge(coedge.edge) {
                        for v in [edge.start, edge.end] {
                            if let Some(vertex) = body.vertex(v) {
                                corners.push(vertex.point);
                            }
                        }
                    }
                }
            }
        }
        if seen
            .insert(
                hash(&(&face.surface, face.reversed, &corners, t.to_bits())),
                (),
            )
            .is_some()
        {
            continue;
        }
        sum.faces += 1;
        let sign = if face.reversed { -1.0 } else { 1.0 };
        let analytic = match &face.surface {
            Surface::Cylinder(c) => face.reversed.then(|| ("cylinder", c.radius())),
            Surface::Sphere(s) => face.reversed.then(|| ("sphere", s.radius())),
            Surface::Torus(torus) => {
                let (major, minor) = torus.radii();
                Some(if face.reversed {
                    ("torus (its tube)", minor)
                } else {
                    ("torus (its inner side, if the face has it)", major - minor)
                })
            }
            _ => None,
        };
        if let Some((kind, radius)) = analytic {
            if radius < t {
                sum.analytic_full += 1;
                if radius < 0.5 * t {
                    sum.analytic_half += 1;
                }
                println!(
                    "REVIEW {tag} | face {index} | a concave {kind} of radius {radius:.4e} ({:.3} t)",
                    radius / t
                );
            }
            continue;
        }
        if !matches!(face.surface, Surface::Nurbs(_) | Surface::Bezier(_)) {
            continue;
        }
        let nurbs = match &face.surface {
            Surface::Nurbs(n) => n.clone(),
            Surface::Bezier(b) => match NurbsSurface::from_bezier(b) {
                Ok(n) => n,
                Err(_) => continue,
            },
            _ => continue,
        };
        sum.freeform += 1;
        let Ok(spans) = nurbs.bezier_spans() else {
            continue;
        };
        let mut own_half: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut flagged: Vec<usize> = Vec::new();
        let mut shown: Vec<String> = Vec::new();
        for (k, span) in spans.iter().enumerate() {
            sum.spans += 1;
            let Some(own) = span_surface(&span.patch) else {
                *own_half.entry("no span surface").or_default() += 1;
                continue;
            };
            let start = Instant::now();
            let half = regular(&own, sign * 0.5 * t);
            let took = start.elapsed().as_nanos();
            sum.nanos_half += took;
            sum.slowest = sum.slowest.max(took);
            let start = Instant::now();
            let full = regular(&own, sign * t);
            sum.nanos_full += start.elapsed().as_nanos();
            if full.is_ok() {
                sum.proven_full += 1;
            }
            let pole = if half.is_ok() {
                None
            } else {
                collapsed(&face.surface, span.u, span.v, t)
            };
            let what = match &half {
                Ok(()) => "proven",
                Err(OffsetError::Folds { at, radius }) => {
                    if shown.len() < 3 {
                        let p = nurbs.eval(
                            span.u.0 + (span.u.1 - span.u.0) * at.0,
                            span.v.0 + (span.v.1 - span.v.0) * at.1,
                        );
                        let near = corners
                            .iter()
                            .map(|c| c.distance(p))
                            .fold(f64::INFINITY, f64::min);
                        shown.push(format!(
                            "a fold shown at ({:.4}, {:.4}, {:.4}) radius {:.3} t, {:.1} t from a vertex",
                            p.x,
                            p.y,
                            p.z,
                            radius / t,
                            near / t
                        ));
                    }
                    "fold"
                }
                Err(OffsetError::NoNormal { at }) | Err(OffsetError::Unproven { at }) => {
                    if pole.is_some() {
                        "not proven (a pole's span)"
                    } else {
                        if shown.len() < 3 {
                            let p = nurbs.eval(
                                span.u.0 + (span.u.1 - span.u.0) * at.0,
                                span.v.0 + (span.v.1 - span.v.0) * at.1,
                            );
                            let near = corners
                                .iter()
                                .map(|c| c.distance(p))
                                .fold(f64::INFINITY, f64::min);
                            shown.push(format!(
                                "not proven near ({:.4}, {:.4}, {:.4}), {:.1} t from a vertex",
                                p.x,
                                p.y,
                                p.z,
                                near / t
                            ));
                        }
                        if matches!(half, Err(OffsetError::NoNormal { .. })) {
                            "no normal (no pole)"
                        } else {
                            "unproven (no pole)"
                        }
                    }
                }
                Err(OffsetError::Spent) => "spent",
                Err(_) => "other",
            };
            *own_half.entry(what).or_default() += 1;
            if !(full.is_ok() && half.is_ok()) {
                flagged.push(k);
            }
        }
        for (k, v) in &own_half {
            *sum.half.entry(*k).or_default() += *v;
        }
        if flagged.is_empty() {
            continue;
        }
        // The tips: the face's vertices, and a border of a flagged span
        // drawn into a point.
        let mut tips = corners.clone();
        let at = |u: f64, v: f64| face.surface.eval(u, v);
        let most = 200_usize.min(3000_usize.saturating_sub(sampled));
        for &k in flagged.iter().take(most) {
            let span = &spans[k];
            if let Some(p) = collapsed(&face.surface, span.u, span.v, t) {
                tips.push(p);
            }
        }
        // The concave radius over each flagged span.
        let mut cands: Vec<(f64, Vec3, (f64, f64), (f64, f64))> = Vec::new();
        let mut near_tip = [0_usize; 3];
        let class = |r: f64| -> usize {
            if r == 0.0 {
                2
            } else if r < 0.5 * t {
                0
            } else {
                1
            }
        };
        let n = 32;
        sampled += flagged.len().min(most);
        for &k in flagged.iter().take(most) {
            let span = &spans[k];
            let (hu, hv) = (
                (span.u.1 - span.u.0) / n as f64,
                (span.v.1 - span.v.0) / n as f64,
            );
            for i in 0..n {
                for j in 0..n {
                    let u = span.u.0 + hu * (i as f64 + 0.5);
                    let v = span.v.0 + hv * (j as f64 + 0.5);
                    let r = concave(&face.surface, sign, u, v).unwrap_or(0.0);
                    if !(r < t) {
                        continue;
                    }
                    let p = at(u, v);
                    if tips.iter().any(|tip| tip.distance(p) <= TIP * t) {
                        near_tip[class(r)] += 1;
                        continue;
                    }
                    cands.push((r, p, (u, v), (hu, hv)));
                }
            }
        }
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        let total = cands.len();
        // The no-normal samples come first (radius 0); keep some of each.
        let mut kept: Vec<(f64, Vec3, (f64, f64), (f64, f64))> = Vec::new();
        let mut of_kind = [0_usize; 3];
        for c in &cands {
            let kind = class(c.0);
            if of_kind[kind] < 120 {
                of_kind[kind] += 1;
                kept.push(*c);
            }
        }
        let points: Vec<Vec3> = kept.iter().map(|c| c.1).collect();
        let held = if points.is_empty() {
            None
        } else {
            craft3d_topo::face_holds(body, face, tol, &points)
        };
        let mut counts = [[0_usize; 3]; 3];
        let mut worst: Option<(f64, Vec3, (f64, f64), (f64, f64), &'static str)> = None;
        for (i, c) in kept.iter().enumerate() {
            let (place, word): (usize, &'static str) = match held.as_ref().and_then(|h| h.get(i)) {
                Some(true) => (0, "in the face"),
                Some(false) => (2, "out of the face"),
                None => (1, "not told in or out"),
            };
            let kind = class(c.0);
            counts[kind][place] += 1;
            if place != 2 && kind != 2 && worst.is_none() {
                worst = Some((c.0, c.1, c.2, c.3, word));
            }
        }
        for a in 0..3 {
            for b in 0..3 {
                sum.away[a][b] += counts[a][b];
            }
            sum.near_tip[a] += near_tip[a];
        }
        let mut line = format!(
            "REVIEW {tag} | face {index} reversed {} | {} spans, {} not proven at t | at t/2: {own_half:?} | samples away from tips ({total} below t, {} asked) [in, not told, out]: below t/2 {:?}, t/2 to t {:?}, no normal {:?} | within a tip ball {near_tip:?}",
            face.reversed,
            spans.len(),
            flagged.len(),
            kept.len(),
            counts[0],
            counts[1],
            counts[2]
        );
        for s in &shown {
            let _ = write!(line, " | {s}");
        }
        if let Some((r, p, uv, steps, word)) = worst {
            let (least, place) = refined(&face.surface, sign, uv, steps, &tips, t);
            let q = at(place.0, place.1);
            let near = tips
                .iter()
                .map(|c| c.distance(q))
                .fold(f64::INFINITY, f64::min);
            sum.least = sum.least.min(least / t);
            let _ = write!(
                line,
                " | worst sample {:.4} t at ({:.4}, {:.4}, {:.4}) {word}; refined {:.4} t at ({:.4}, {:.4}, {:.4}), {:.1} t from a tip",
                r / t,
                p.x,
                p.y,
                p.z,
                least / t,
                q.x,
                q.y,
                q.z,
                near / t
            );
            if least < 0.5 * t && own_half.keys().all(|k| *k == "proven") {
                let _ = write!(
                    line,
                    " | CONTRADICTION: every span proven at t/2, a sample below it"
                );
            }
            if turns_left > 0 {
                turns_left -= 1;
                let _ = write!(
                    line,
                    " | turn: {}",
                    turn(&face.surface, sign, place, t, &tips)
                );
            }
        }
        println!("{line}");
    }
    sum
}

/// What today's checks say of a body: the validity check, the measure,
/// the deep check.
fn valid(tag: &str, body: &Body, tol: &Tolerance) {
    let defects = check(body, tol);
    let mass = match mass_properties(body) {
        Ok(m) => format!("volume {:.9e}", m.volume),
        Err(e) => format!("measure error {e:?}"),
    };
    let deep = match body_self_intersections(body, &SelfCheckParams::new(tol.linear)) {
        Ok(f) => format!(
            "deep check certain {} marginal {} (chord {:.3e})",
            f.certain_crossings, f.marginal_crossings, f.chordal_tolerance
        ),
        Err(e) => format!("deep check error {e}"),
    };
    println!(
        "REVIEW {tag} | today's checks: {} faces, {} defects {:?} | {mass} | {deep}",
        body.faces.len(),
        defects.len(),
        defects.first()
    );
}

// ---------------------------------------------------------------------
// Documents: the repository's and the side repositories' whose tolerance
// is coarse (where the rule's radius is a length a design has).

fn document(name: &str, doc: &Document, turns: usize) {
    let Some(tol) = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular) else {
        println!("REVIEW document {name} | no tolerance");
        return;
    };
    let start = Instant::now();
    let regen = craft3d_regen::regenerate(doc);
    let took = ms(start);
    let with = regen.features.iter().filter(|f| f.body.is_some()).count();
    let mut total = Sum::new();
    let mut seen: BTreeMap<u64, ()> = BTreeMap::new();
    let mut bodies: BTreeMap<usize, ()> = BTreeMap::new();
    let mut working = tol.linear;
    for (feature, result) in doc.features.iter().zip(&regen.features) {
        let Some(body) = &result.body else {
            continue;
        };
        if bodies.insert(Arc::as_ptr(body) as usize, ()).is_some() {
            continue;
        }
        working = working.max(body.working_tolerance(tol.linear));
        let sum = rule(
            &format!("{name} | {}", feature.name),
            body,
            &tol,
            tol.linear,
            turns,
            &mut seen,
        );
        total.add(&sum);
    }
    println!(
        "REVIEW document {name} | tolerance {:e} (working at most {:e}) | {} features, {} with a body, {} bodies | regenerated in {:.0} ms | {}",
        tol.linear,
        working,
        doc.features.len(),
        with,
        bodies.len(),
        took,
        total.said()
    );
}

fn file(path: &std::path::Path, turns: usize) {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let Ok(bytes) = std::fs::read(path) else {
        println!("REVIEW document {name} | not read");
        return;
    };
    match craft3d_format::from_bytes(&bytes) {
        Ok(doc) => document(&name, &doc, turns),
        Err(e) => println!("REVIEW document {name} | unreadable: {e}"),
    }
}

/// The side repositories' documents kept beside this test (copies, for
/// the review only), every fourth from `part`.
fn side(part: usize) {
    let Ok(dir) = std::fs::read_dir(root().join("crates/engine/tests/fold_review/side")) else {
        println!("REVIEW side | no folder");
        return;
    };
    let mut found: Vec<PathBuf> = dir.filter_map(|e| e.ok().map(|e| e.path())).collect();
    found.sort();
    for (i, path) in found.iter().enumerate() {
        if i % 4 == part {
            file(path, 3);
        }
    }
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn side_documents_a() {
    side(0);
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn side_documents_b() {
    side(1);
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn side_documents_c() {
    side(2);
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn side_documents_d() {
    side(3);
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn the_repositorys_coarse_documents() {
    for name in [
        "docs/examples/published/ship-propeller.3dcraft",
        "docs/showcase/rotor.3dcraft",
        "crates/engine/tests/fillet_fade/hull.3dcraft",
        "crates/format/tests/fixtures/versions/v59.3dcraft",
    ] {
        file(&root().join(name), 6);
    }
}

// ---------------------------------------------------------------------
// Bodies made by commands.

/// A new document at `linear`, the commands run in it (a refusal or a
/// failed feature said), and its bodies.
fn script(tag: &str, linear: f64, steps: &[(&str, Value)]) -> Vec<Arc<Body>> {
    let dir = common::workdir("fold-review", tag);
    let mut s = common::session_in(&dir);
    if let Err(e) = s.execute("file.new", &json!({"units": "mm", "tolerance": linear})) {
        println!("REVIEW {tag} | file.new refused: {e}");
        return Vec::new();
    }
    for (id, params) in steps {
        match s.execute(id, params) {
            Ok(value) => {
                if value.get("status").and_then(Value::as_str) == Some("failed") {
                    println!(
                        "REVIEW {tag} | {id} failed: {}",
                        value.get("error").map(Value::to_string).unwrap_or_default()
                    );
                }
            }
            Err(e) => println!("REVIEW {tag} | {id} refused: {e}"),
        }
    }
    let Some(Ok(doc)) = s.document_bytes().map(|b| craft3d_format::from_bytes(&b)) else {
        println!("REVIEW {tag} | no document");
        return Vec::new();
    };
    let regen = craft3d_regen::regenerate(&doc);
    regen.bodies().map(|(_, b)| b.clone()).collect()
}

/// A block 40 × 30 × 10 (times `k`) whose top is one cubic spline with
/// the control points `control` (times `k`), from (40, 30) to (0, 30): a
/// notch whose root is a smooth inside turn.
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
    let tol = tolerance(linear);
    if bodies.is_empty() {
        println!("REVIEW {tag} | no body");
    }
    for body in &bodies {
        valid(tag, body, &tol);
        let sum = rule(tag, body, &tol, linear, 2, &mut BTreeMap::new());
        println!("REVIEW {tag} | tolerance {linear:e} | {}", sum.said());
    }
}

/// A notch 10 deep whose walls stand at a right angle: at its root three
/// control points `e` apart.
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

/// A notch 10 deep whose walls stand 30 degrees apart.
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
fn a_profile_with_a_tight_inside_turn() {
    // At a coarse tolerance, the root's control points ten, 2.5 and one
    // tolerance apart.
    for e in [0.2, 0.05, 0.02] {
        notch(&format!("notch-90-t0.02-e{e}"), 0.02, &right_angle(e), 1.0);
    }
    // The same body a thousand times smaller and ten thousand times
    // larger, its tolerance with it.
    notch("notch-90-small", 0.02e-3, &right_angle(0.05), 1e-3);
    notch("notch-90-large", 0.02e4, &right_angle(0.05), 1e4);
    // A narrow notch: walls 30 degrees apart.
    for e in [0.2, 0.05] {
        notch(&format!("notch-30-t0.02-e{e}"), 0.02, &narrow(e), 1.0);
    }
    // At the default tolerance: a root 3 tolerances wide.
    notch("notch-90-t1e-6", 1e-6, &right_angle(3e-6), 1.0);
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn text_extruded() {
    for (linear, size) in [(1e-6, 10.0), (0.01, 10.0), (0.01, 2.0), (0.05, 3.0)] {
        let tag = format!("text-t{linear:e}-size{size}");
        let bodies = script(
            &tag,
            linear,
            &[
                ("sketch.create", json!({"name": "S"})),
                (
                    "sketch.text",
                    json!({"sketch": "S", "text": "Rag&08Bas", "size": size}),
                ),
                (
                    "feature.extrude",
                    json!({"sketch": "S", "distance": 1, "name": "T"}),
                ),
            ],
        );
        let tol = tolerance(linear);
        let mut total = Sum::new();
        let mut seen = BTreeMap::new();
        for body in &bodies {
            total.add(&rule(&tag, body, &tol, linear, 1, &mut seen));
        }
        println!("REVIEW {tag} | {} bodies | {}", bodies.len(), total.said());
    }
}

// The path of fold_probe's sweeps: a cubic that crosses itself.
const LOOP_PATH: [[f64; 2]; 4] = [
    [0.0, 0.0],
    [0.0, 1.886_796],
    [-1.165_998, 0.0212],
    [0.529_999, 0.847_998],
];

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn a_thin_tube_meshed_with_the_angle() {
    use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    use craft3d_kernel::sweep::sweep;
    use craft3d_tess::{TessParams, tessellate_body};
    // The deep check's first mesh (a chord of a thousandth of the body's
    // diagonal) of a tube along a path that crosses itself, with and
    // without the angle of 12 degrees: what the angle adds where the
    // tube's radius is a few chords, one, and a fraction of one.
    let length = 2.195_798_812_420_925;
    for times in [30.0, 100.0, 1000.0, 10_000.0] {
        let tol = tolerance(1e-6);
        let rho = length / times;
        let profile = detect_profiles(
            &[SketchCurve {
                id: 1,
                curve: Curve2::Circle {
                    center: [0.0, 0.0],
                    radius: rho,
                },
            }],
            &tol,
        )
        .unwrap();
        let path = [SketchCurve {
            id: 2,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: LOOP_PATH.iter().map(|p| [p[0], p[1]]).collect(),
            }),
        }];
        let path_plane = Frame::new(Vec3::ZERO, Vec3::new(0.0, -1.0, 0.0), Vec3::X).unwrap();
        let made = match sweep(&profile[0], &Frame::WORLD, &path, &path_plane, &tol) {
            Ok(made) => made,
            Err(e) => {
                println!("REVIEW tube length/radius {times} | refused: {e}");
                continue;
            }
        };
        let diagonal = made.body.bbox().map(|b| b.diagonal()).unwrap_or(f64::NAN);
        let chord = 1e-3 * diagonal;
        for angle in [None, Some(12.0_f64.to_radians())] {
            match tessellate_body(
                &made.body,
                &TessParams {
                    chordal_tolerance: chord,
                    max_angle: angle,
                    ..TessParams::default()
                },
            ) {
                Ok(mesh) => {
                    // The sides of the tube's cross-section where it
                    // starts: the mesh's points on the rim of its first
                    // cap (the circle of radius rho about the origin in
                    // the plane z = 0).
                    let mut rim: BTreeMap<(i64, i64), ()> = BTreeMap::new();
                    for q in &mesh.positions {
                        let r = (q.x * q.x + q.y * q.y).sqrt();
                        if q.z.abs() <= 1e-9 && (r - rho).abs() <= 1e-6 * rho {
                            rim.insert(
                                (
                                    (q.x / rho * 1e7).round() as i64,
                                    (q.y / rho * 1e7).round() as i64,
                                ),
                                (),
                            );
                        }
                    }
                    println!(
                        "REVIEW tube length/radius {times} | radius {rho:.3e}, {:.2} chords of {chord:.3e} | angle {:?}: {} triangles, {} sides round the tube at its start",
                        rho / chord,
                        angle.map(f64::to_degrees),
                        mesh.triangles.len(),
                        rim.len()
                    );
                }
                Err(e) => println!(
                    "REVIEW tube length/radius {times} | angle {:?}: error {e}",
                    angle.map(f64::to_degrees)
                ),
            }
        }
    }
}

// ---------------------------------------------------------------------
// Fillets: on a concave edge (constant and fading), then the review's
// seven folded fades on the hull's stem and what a STEP file keeps of
// them. One test, because the fillet's own refusal is switched off for
// the whole process while the folded ones are built.

fn hull() -> (Body, Tolerance) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fold_probe/trimmed.step");
    let bytes = std::fs::read(path).unwrap();
    let tol = tolerance(0.02);
    let options = craft3d_stepread::Options {
        metres_per_unit: 1e-3,
        tolerance: tol,
        loosest: 1000.0 * tol.linear,
    };
    (
        craft3d_stepread::parse(&bytes)
            .unwrap()
            .build(0, &options)
            .unwrap()
            .body,
        tol,
    )
}

fn stem(body: &Body) -> (EdgeId, bool) {
    let foot = Vec3::new(5.0, 0.0, -8.9);
    let mut found = None;
    for (i, e) in body.edges.iter().enumerate() {
        let (a, b) = (
            body.vertices[e.start.0 as usize].point,
            body.vertices[e.end.0 as usize].point,
        );
        for (here, there, at_start) in [(a, b, true), (b, a, false)] {
            if here.distance(foot) < 0.5 && there.z > here.z + 20.0 {
                found = Some((EdgeId(i as u32), at_start));
            }
        }
    }
    found.unwrap()
}

/// The body written as a STEP file and read back.
fn through_step(tag: &str, body: &Body, tol: &Tolerance) -> Option<Body> {
    let options = StepOptions {
        tolerance: *tol,
        ..StepOptions::default()
    };
    let bytes = match export_step(&[StepBody::new("probe", body)], &options) {
        Ok((bytes, _)) => bytes,
        Err(e) => {
            println!("REVIEW {tag} | STEP not written: {e}");
            return None;
        }
    };
    let options = craft3d_stepread::Options {
        metres_per_unit: 1e-3,
        tolerance: *tol,
        loosest: 1000.0 * tol.linear,
    };
    match craft3d_stepread::parse(&bytes).and_then(|parsed| parsed.build(0, &options)) {
        Ok(read) => {
            println!(
                "REVIEW {tag} | STEP of {} bytes read back: {} faces",
                bytes.len(),
                read.body.faces.len()
            );
            Some(read.body)
        }
        Err(e) => {
            println!("REVIEW {tag} | STEP of {} bytes refused: {e}", bytes.len());
            None
        }
    }
}

fn on_a_concave_edge() {
    // An L: 40 × 40 with a 20 × 20 corner left out, 30 high. Its inside
    // edge runs up at (20, 20).
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
        println!("REVIEW concave-L | no body");
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
        println!("REVIEW concave-L | inside edge not found");
        return;
    };
    let block = (40.0 * 40.0 - 20.0 * 20.0) * 30.0;
    let quarter = 1.0 - std::f64::consts::FRAC_PI_4;
    // Constant radii of 0.3 to 5 tolerances, then fades to a point.
    for times in [0.3, 0.45, 1.0, 2.0, 5.0] {
        let radius = times * linear;
        let tag = format!("concave-L constant {times} t");
        match craft3d_kernel::fillet::fillet(
            body,
            &[craft3d_kernel::fillet::FilletEdge { edge, radius }],
            &tol,
        ) {
            Ok(made) => {
                println!(
                    "REVIEW {tag} | built; closed form volume {:.9e}",
                    block + quarter * radius * radius * 30.0
                );
                valid(&tag, &made.body, &tol);
                let sum = rule(&tag, &made.body, &tol, linear, 1, &mut BTreeMap::new());
                println!("REVIEW {tag} | {}", sum.said());
            }
            Err(e) => println!("REVIEW {tag} | refused: {e}"),
        }
    }
    let mut laws: Vec<(String, RadiusLaw, Option<f64>)> = Vec::new();
    for (radius, over, back) in [
        (3.0, 6.0, 0.33),
        (3.0, 6.0, 0.0),
        (2.0, 10.0, 0.5),
        (3.0, 3.0, 2.0),
        (3.0, 3.0, 0.0),
        (5.0, 5.0, 0.5),
        (5.0, 3.0, 0.5),
    ] {
        laws.push((
            format!("concave-L fade r{radius} over{over} back{back}"),
            RadiusLaw {
                radius,
                fades: [Some(Fade::tip(over, back)), None],
            },
            None,
        ));
    }
    for (tag, law, expected) in laws {
        let start = Instant::now();
        match fillet_by_law(body, &[FilletLawEdge { edge, law }], &tol) {
            Ok(made) => {
                println!(
                    "REVIEW {tag} | built with the fillet's own refusal on ({:.0} ms); closed form volume {:?}",
                    ms(start),
                    expected
                );
                valid(&tag, &made.body, &tol);
                let sum = rule(&tag, &made.body, &tol, linear, 2, &mut BTreeMap::new());
                println!("REVIEW {tag} | {}", sum.said());
            }
            Err(e) => println!("REVIEW {tag} | refused with the fillet's own refusal on: {e}"),
        }
    }
}

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn fillets_concave_and_the_seven_fades() {
    on_a_concave_edge();
    // The seven folded fades, read from the STEP fixtures (the fillet
    // refuses them now; fold-check, Check 44328620802).
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fold_probe/fades");
    let tol = tolerance(0.02);
    for name in [
        "fade-3-6-0.33",
        "fade-3-6-0.5",
        "fade-4-8-0.33",
        "fade-4-8-0.5",
        "fade-5-10-1",
        "fade-3-3-2",
        "fade-4-8-1",
    ] {
        let options = craft3d_stepread::Options {
            metres_per_unit: 1e-3,
            tolerance: tol,
            loosest: 1000.0 * tol.linear,
        };
        let Ok(bytes) = std::fs::read(dir.join(format!("{name}.step"))) else {
            println!("REVIEW {name} | fixture not read");
            continue;
        };
        let body = match craft3d_stepread::parse(&bytes).and_then(|p| p.build(0, &options)) {
            Ok(read) => read.body,
            Err(e) => {
                println!("REVIEW {name} | refused: {e}");
                continue;
            }
        };
        let defects = check(&body, &tol);
        let sum = rule(name, &body, &tol, tol.linear, 3, &mut BTreeMap::new());
        println!(
            "REVIEW {name} | check {} defects | {}",
            defects.len(),
            sum.said()
        );
    }
    // The sound law, built with the refusal on.
    let (body, tol) = hull();
    let (edge, foot_at_start) = stem(&body);
    let fade = Some(Fade::tip(15.0, 0.33));
    let law = RadiusLaw {
        radius: 3.0,
        fades: if foot_at_start {
            [fade, None]
        } else {
            [None, fade]
        },
    };
    match fillet_by_law(&body, &[FilletLawEdge { edge, law }], &tol) {
        Ok(made) => {
            let sum = rule(
                "fade r3 over15 back0.33",
                &made.body,
                &tol,
                tol.linear,
                3,
                &mut BTreeMap::new(),
            );
            println!("REVIEW fade r3 over15 back0.33 | {}", sum.said());
        }
        Err(e) => println!("REVIEW fade r3 over15 back0.33 | refused: {e}"),
    }
}

// ---------------------------------------------------------------------
// A block whose top is a spline over a net of 13 × 13 points, of degree 3
// (fold_probe's) or 1: with degree 1 every span is a flat or twisted
// quadrilateral, and a net pulled through itself folds only between
// spans.

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

/// fold_probe's block with its top pulled through itself (`looped`) or
/// into an overhang, the top of degree `degree`.
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
fn a_top_pulled_through_itself_of_degree_one_and_three() {
    let s = 1.0;
    let tol = tolerance(1e-3 * s);
    for (looped, degree) in [(true, 3), (false, 3), (true, 1), (false, 1)] {
        for times in [3.0, 100.0, 10_000.0] {
            let tag = format!(
                "roof {} degree {degree} x{times}",
                if looped { "looped" } else { "overhang" }
            );
            let body = roof_block(times * 6.0 * s, s, looped, degree);
            valid(&tag, &body, &tol);
            let sum = rule(&tag, &body, &tol, tol.linear, 1, &mut BTreeMap::new());
            println!("REVIEW {tag} | {}", sum.said());
        }
    }
}

// ---------------------------------------------------------------------
// What one span costs by its degree, where the net's bound does not
// prove it and the forms are built.

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn what_a_span_costs_by_its_degree() {
    let mut slow = false;
    for p in [3_usize, 5, 7, 9, 11, 13, 15, 16] {
        if slow {
            println!("REVIEW degree {p} | not asked: the degree before took too long");
            continue;
        }
        let count = p + 1;
        let mut net = Vec::with_capacity(count * count);
        for i in 0..count {
            for j in 0..count {
                let (x, y) = (i as f64 / p as f64 - 0.5, j as f64 / p as f64 - 0.5);
                net.push(Vec3::new(x, y, 0.8 * (x * x + y * y) + 0.3 * x * x * x));
            }
        }
        let clamped = |d: usize| {
            let mut k = vec![0.0; d + 1];
            k.extend(vec![1.0; d + 1]);
            k
        };
        let surface = match NurbsSurface::new(
            p,
            p,
            clamped(p),
            clamped(p),
            count,
            count,
            net,
            vec![1.0; count * count],
        ) {
            Ok(s) => Surface::Nurbs(s),
            Err(e) => {
                println!("REVIEW degree {p} | no surface: {e:?}");
                continue;
            }
        };
        let mut least = f64::INFINITY;
        for i in 0..=60 {
            for j in 0..=60 {
                let (u, v) = (i as f64 / 60.0, j as f64 / 60.0);
                for sign in [1.0, -1.0] {
                    if let Some(r) = concave(&surface, sign, u, v) {
                        least = least.min(r);
                    }
                }
            }
        }
        for share in [1e-4, 0.5, 0.9] {
            for sign in [1.0, -1.0] {
                let before = craft3d_geom::allowance::spent();
                let start = Instant::now();
                let found = craft3d_geom::allowance::within(40_000_000, || {
                    regular(&surface, sign * share * least)
                });
                let took = ms(start);
                println!(
                    "REVIEW degree {p} | offset by {} of the least radius {least:.4} | {} | {took:.1} ms, {} work units",
                    sign * share,
                    match found {
                        Ok(()) => "proven".to_string(),
                        Err(e) => format!("{e:?}"),
                    },
                    craft3d_geom::allowance::spent() - before
                );
                if took > 30_000.0 {
                    slow = true;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// The ABC dataset's STEP models, where the build server keeps them (the
// cache of `cargo xtask abc`; `CRAFT3D_FOLD_ABC` names another folder):
// ids and counts only, nothing of a model is printed.

#[test]
#[ignore = "a probe: run with --ignored --nocapture"]
fn abc_models() {
    let Some(folder) = std::env::var("CRAFT3D_FOLD_ABC")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join("3dcraft-abc"))
        })
        .filter(|f| f.is_dir())
    else {
        println!("REVIEW abc | no cache on this machine: not checked");
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
    println!("REVIEW abc | {} STEP files in the cache", files.len());
    let started = Instant::now();
    let stride = (files.len() / 500).max(1);
    let (mut asked, mut read, mut refused, mut large, mut tolerant) =
        (0_usize, 0_usize, 0_usize, 0_usize, 0_usize);
    let mut total = Sum::new();
    let mut loose = Sum::new();
    for path in files.iter().step_by(stride) {
        if started.elapsed().as_secs() > 540 {
            break;
        }
        if std::fs::metadata(path).map_or(true, |m| m.len() > 600_000) {
            large += 1;
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        asked += 1;
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let tol = tolerance(1e-6);
        let options = craft3d_stepread::Options {
            metres_per_unit: 1e-3,
            tolerance: tol,
            loosest: 1000.0 * tol.linear,
        };
        let body = match craft3d_stepread::parse(&bytes).and_then(|p| p.build(0, &options)) {
            Ok(made) => made.body,
            Err(_) => {
                refused += 1;
                continue;
            }
        };
        read += 1;
        let tag = format!("abc {name}");
        let sum = rule(&tag, &body, &tol, tol.linear, 0, &mut BTreeMap::new());
        total.add(&sum);
        let working = body.working_tolerance(tol.linear);
        if working > tol.linear {
            tolerant += 1;
            let tag = format!("abc {name} at its working tolerance {working:e}");
            let sum = rule(&tag, &body, &tol, working, 0, &mut BTreeMap::new());
            loose.add(&sum);
        }
    }
    println!(
        "REVIEW abc | {asked} models asked (every {stride}th; {large} over 600 kB left out), {read} read, {refused} refused, in {:.0} s | at 1e-6: {}",
        started.elapsed().as_secs_f64(),
        total.said()
    );
    println!(
        "REVIEW abc | {tolerant} read with a tolerance of their own; at each one's working tolerance: {}",
        loose.said()
    );
}
