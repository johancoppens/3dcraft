//! What an import's refusal of an edge says about it: how far the edge's
//! curve lies off each face's surface at dense samples, what the prover
//! reaches past the cap, and which entities of the file the curve and the
//! surfaces are. Only for messages: nothing here accepts or refuses.

use crate::finish::Entity;
use crate::geometry::Reader;
use crate::keywords;
use craft3d_geom::{Curve3, Surface};
use craft3d_step::Param;
use craft3d_topo::{Body, EdgeId};
use std::fmt::Write as _;

/// Places along the edge where its distance to each face is sampled.
const PLACES: u32 = 1024;
/// How far past the cap the prover is asked to go for the message: at
/// two, eight and sixty-four times the cap, each in at most
/// [`DIAGNOSE_PIECES`] pieces, so that a refusal costs a bounded amount
/// of work beyond what the import spent (review of 2026-11-02).
const PAST_CAP: [f64; 3] = [2.0, 8.0, 64.0];
/// The pieces each of those proofs may take.
const DIAGNOSE_PIECES: usize = 4096;
/// Every so many places the surface's own projection is asked (between
/// them the foot is walked from the last), and every so many more the
/// search by the surface's spans.
const PROJECT_EVERY: u32 = 16;
const SEED_EVERY: u32 = 256;

/// The largest sampled distance of `curve` over `range` from `surface`,
/// and where along the range it is (0 to 1): the foot followed from the
/// last one, and the surface's own projection, the nearer.
pub(crate) fn sampled(curve: &Curve3, range: (f64, f64), surface: &Surface) -> (f64, f64) {
    let (t0, t1) = range;
    let mut worst = (0.0_f64, 0.0_f64);
    let mut last: Option<(f64, f64)> = None;
    for k in 0..=PLACES {
        let s = f64::from(k) / f64::from(PLACES);
        let p = curve.eval(t0 + (t1 - t0) * s);
        let mut best = f64::INFINITY;
        let mut foot = None;
        // The surface's own projection may land in another dip of the
        // distance on a spline of many spans: a search by its spans at
        // five places.
        let projected = (k % PROJECT_EVERY == 0 || last.is_none())
            .then(|| surface.project(p).ok())
            .flatten();
        let seeded = (k % SEED_EVERY == 0)
            .then(|| craft3d_topo::nearest_foot(surface, p))
            .flatten();
        for at in [
            last.and_then(|l| surface.project_from(p, l).ok()),
            projected,
            seeded,
        ]
        .into_iter()
        .flatten()
        {
            let d = surface.eval(at.0, at.1).distance(p);
            if d.is_finite() && d < best {
                best = d;
                foot = Some(at);
            }
        }
        last = foot.or(last);
        if best > worst.0 {
            worst = (best, s);
        }
    }
    worst
}

/// The keyword of the entity `id`, through a `SURFACE_CURVE`,
/// `SEAM_CURVE` or `INTERSECTION_CURVE` to its 3D curve.
fn curve_keyword(reader: &Reader<'_>, id: u64) -> String {
    let Ok(instance) = reader.parsed.instance(id) else {
        return "?".to_string();
    };
    let outer = keywords(&instance.entity);
    for kind in ["SURFACE_CURVE", "SEAM_CURVE", "INTERSECTION_CURVE"] {
        if instance.entity.is(kind)
            && let Ok(record) = reader.parsed.record(id, kind)
            && let Some(inner) = record.params.get(1).and_then(Param::as_ref)
            && let Ok(inner) = reader.parsed.instance(inner)
        {
            return format!("{outer} of {}", keywords(&inner.entity));
        }
    }
    outer
}

/// The id of the entity attribute `index` of `id`'s record `keyword`
/// refers to.
fn attribute(reader: &Reader<'_>, id: u64, keyword: &str, index: usize) -> Option<u64> {
    reader
        .parsed
        .record(id, keyword)
        .ok()?
        .params
        .get(index)
        .and_then(Param::as_ref)
}

/// The file's entity behind an edge's curve (through a surface curve to
/// its 3D curve) or a face's surface, by keyword: what a STEP file's
/// refusal names (`finish::finish_described`).
pub(crate) fn step_entity(reader: &Reader<'_>, entity: Entity) -> Option<String> {
    match entity {
        Entity::Edge(id) => {
            attribute(reader, id, "EDGE_CURVE", 3).map(|g| curve_keyword(reader, g))
        }
        Entity::Face(id) => surface_of_face(reader, id)
            .and_then(|s| reader.parsed.instance(s).ok())
            .map(|i| keywords(&i.entity)),
    }
}

/// The face's surface entity in the file (an advanced face's
/// `face_geometry`).
fn surface_of_face(reader: &Reader<'_>, face: u64) -> Option<u64> {
    ["ADVANCED_FACE", "FACE_SURFACE"]
        .into_iter()
        .find_map(|k| attribute(reader, face, k, 2))
}

fn shape(curve: &Curve3) -> String {
    match curve {
        Curve3::Nurbs(n) => {
            let mut spans = n.knots().to_vec();
            spans.dedup();
            format!(
                ", degree {}, {} spans",
                n.degree(),
                spans.len().saturating_sub(1)
            )
        }
        _ => String::new(),
    }
}

fn surface_shape(surface: &Surface) -> String {
    match surface {
        Surface::Nurbs(n) => {
            let count = |k: &[f64]| {
                let mut k = k.to_vec();
                k.dedup();
                k.len().saturating_sub(1)
            };
            let (du, dv) = n.degrees();
            format!(
                ", degrees {du} x {dv}, {} x {} spans",
                count(n.knots_u()),
                count(n.knots_v())
            )
        }
        _ => String::new(),
    }
}

/// What the prover proves of `curve` on `surface` past the cap, in a
/// bounded number of pieces: the first of [`PAST_CAP`] times the cap it
/// proves, or where the last proof stopped.
fn past_cap(
    curve: &Curve3,
    range: (f64, f64),
    surface: &Surface,
    tolerance: &craft3d_math::Tolerance,
    cap: f64,
) -> String {
    use craft3d_geom::certify::{CurveBound, Within, curve_on_surface};
    let bounded = CurveBound::new(curve);
    let mut stopped = 0.5 * (range.0 + range.1);
    let mut last = cap;
    for times in PAST_CAP {
        last = (times * cap).max(tolerance.linear);
        let within = Within {
            target: last,
            most: DIAGNOSE_PIECES,
        };
        match curve_on_surface(&bounded, range, surface, None, &[], within) {
            Ok(p) => return format!("proven within {:.2e}", p.bound),
            Err(not) => stopped = not.at,
        }
    }
    let (t0, t1) = range;
    let s = if t1 > t0 {
        (stopped - t0) / (t1 - t0)
    } else {
        0.0
    };
    format!("not proven within {last:.0e} in {DIAGNOSE_PIECES} pieces (stopped at {s:.3} of it)")
}

/// What the faces of `edge` and the samples say about it, for the
/// message of a refusal: per face its sampled distance and where, the
/// file's surface entity (`describe`, else the surface's kind) and what
/// the prover proves past the cap ([`past_cap`]); the edge's
/// curve entity (likewise) and its length.
#[allow(clippy::too_many_arguments)]
pub(crate) fn edge(
    body: &Body,
    edge: EdgeId,
    edge_step: u64,
    face_ids: &[u64],
    tolerance: &craft3d_math::Tolerance,
    cap: f64,
    uncertainty: Option<f64>,
    describe: &dyn Fn(Entity) -> Option<String>,
) -> String {
    let Some(e) = body.edge(edge) else {
        return String::new();
    };
    let mut out = String::new();
    let curve = describe(Entity::Edge(edge_step)).unwrap_or_else(|| e.curve.kind().to_string());
    let length = {
        let (t0, t1) = e.range;
        let mut sum = 0.0;
        let mut prev = e.curve.eval(t0);
        for k in 1..=64 {
            let p = e.curve.eval(t0 + (t1 - t0) * f64::from(k) / 64.0);
            sum += p.distance(prev);
            prev = p;
        }
        sum
    };
    let _ = write!(out, "a {curve}{} {length:.3e} long; ", shape(&e.curve));
    let mut first = true;
    for (f, face) in body.faces.iter().enumerate() {
        let uses = face.loops.iter().any(|l| {
            body.loop_(*l)
                .is_some_and(|l| l.coedges.iter().any(|c| c.edge == edge))
        });
        if !uses {
            continue;
        }
        let face_step = face_ids.get(f).copied().unwrap_or(0);
        let surface =
            describe(Entity::Face(face_step)).unwrap_or_else(|| face.surface.kind().to_string());
        let (distance, at) = sampled(&e.curve, e.range, &face.surface);
        let proven = past_cap(&e.curve, e.range, &face.surface, tolerance, cap);
        let _ = write!(
            out,
            "{}face #{face_step} ({surface}{}): sampled up to {distance:.2e} off at {at:.3} of it, {proven}",
            if first { "" } else { "; " },
            surface_shape(&face.surface)
        );
        first = false;
    }
    if let Some(u) = uncertainty {
        let _ = write!(out, "; the file's uncertainty {u:.1e}");
    }
    out
}
