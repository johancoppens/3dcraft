//! The geometry of design verification (docs/design/verification.md):
//! the distance between faces, the thinnest place of a body, a line cast
//! through the model, whether a hole goes through, symmetry about a plane.
//!
//! Every answer is found on a mesh first and then settled on the exact
//! geometry: the nearest points of two faces by projecting onto their
//! surfaces and edges in turn, a thickness by the exact intersection of a
//! line with the faces (`craft3d_kernel::boolean::probe`), moved along the
//! face to where it is least. Work is bounded: the mesh has a triangle
//! budget, the pairs of triangles compared and the samples taken are
//! capped, and every iteration has a limit; past a cap the answer is an
//! error that says so, never a guess.

use craft3d_geom::{Curve3, Surface};
use craft3d_kernel::boolean::probe::{LineHit, OnFace, Place, Probe};
use craft3d_math::{Bbox3, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_topo::{Body, Edge};
use std::collections::BTreeMap;

/// Most triangles of the mesh of one body.
const MAX_TRIANGLES: usize = 400_000;
/// Most pairs of triangles one distance compares.
const MAX_PAIRS: u64 = 60_000_000;
/// Most points a thickness or a symmetry samples.
pub(crate) const MAX_SAMPLES: usize = 6_000;
/// Most candidate places a distance or a thickness settles exactly.
const MAX_REFINED: usize = 16;
/// Most steps of one settling.
const MAX_STEPS: usize = 400;
/// Most points sampled along an edge for a hole's extent.
const EDGE_SAMPLES: u32 = 32;

pub(crate) type Answer<T> = Result<T, String>;

/// A body with its mesh, the faces of it a question is about, and their
/// triangles and boxes.
pub(crate) struct Part<'a> {
    pub probe: Probe<'a>,
    /// The chordal tolerance the mesh was made with.
    pub chord: f64,
    /// Per face taking part: its triangles' corners.
    pub triangles: BTreeMap<usize, Vec<[Vec3; 3]>>,
    /// Per face taking part: the box of its triangles grown by the chord.
    pub boxes: BTreeMap<usize, Bbox3>,
}

impl<'a> Part<'a> {
    /// `faces`: the faces taking part, or every face when `None`.
    pub fn new(
        body: &'a Body,
        faces: Option<&[usize]>,
        tolerance: &craft3d_math::Tolerance,
    ) -> Answer<Self> {
        let probe = Probe::new(body, tolerance).map_err(|e| e.to_string())?;
        let (mesh, chord) = mesh_of(body, tolerance.linear)?;
        let mut triangles: BTreeMap<usize, Vec<[Vec3; 3]>> = BTreeMap::new();
        let wanted = |face: usize| faces.is_none_or(|list| list.contains(&face));
        for (t, &face) in mesh.face_ids.iter().enumerate() {
            let face = face as usize;
            if !wanted(face) {
                continue;
            }
            if let Some(corners) = mesh.corners(t) {
                triangles.entry(face).or_default().push(corners);
            }
        }
        if let Some(list) = faces {
            // A face without a triangle (none should be) is still asked.
            for &face in list {
                if face < body.faces.len() {
                    triangles.entry(face).or_default();
                }
            }
        }
        let boxes = triangles
            .iter()
            .map(|(&face, tris)| {
                let mut b = Bbox3::from_points(tris.iter().flatten().copied());
                if b.is_empty() {
                    b = face_box(body, face);
                }
                (face, grown(&b, chord))
            })
            .collect();
        Ok(Part {
            probe,
            chord,
            triangles,
            boxes,
        })
    }

    pub fn body(&self) -> &'a Body {
        self.probe.body()
    }
}

/// The mesh of `body` at a chordal tolerance relative to its size, made
/// coarser while it is over the triangle budget.
fn mesh_of(body: &Body, tol: f64) -> Answer<(Mesh, f64)> {
    let size = body
        .bbox()
        .ok()
        .map(|b| b.diagonal())
        .filter(|d| d.is_finite() && *d > 0.0)
        .ok_or("the body has no finite size")?;
    let mut chord = (size * 1e-3).max(10.0 * tol);
    for _ in 0..4 {
        let params = TessParams {
            chordal_tolerance: chord,
            max_triangles: Some(MAX_TRIANGLES),
            max_angle: Some(12.0_f64.to_radians()),
            ..TessParams::default()
        };
        match tessellate_body(body, &params) {
            Ok(mesh) => return Ok((mesh, chord)),
            Err(craft3d_tess::TessError::OverBudget { .. }) => chord *= 4.0,
            Err(error) => return Err(format!("the body cannot be meshed: {error}")),
        }
    }
    Err(format!(
        "the body needs more than {MAX_TRIANGLES} triangles even at a chordal tolerance of {chord:e}"
    ))
}

fn grown(b: &Bbox3, by: f64) -> Bbox3 {
    match b.corners() {
        Some((lo, hi)) => {
            let d = Vec3::new(by, by, by);
            Bbox3::from_points([lo - d, hi + d])
        }
        None => *b,
    }
}

/// The box of a face from its edges' points (for a face the mesh left
/// without triangles).
fn face_box(body: &Body, face: usize) -> Bbox3 {
    let mut b = Bbox3::from_points([]);
    for edge in face_edges(body, face) {
        for p in edge_points(edge, 8) {
            b.add(p);
        }
    }
    b
}

/// The distance from `p` to the box, 0 inside it.
pub(crate) fn box_distance(b: &Bbox3, p: Vec3) -> f64 {
    let Some((lo, hi)) = b.corners() else {
        return f64::INFINITY;
    };
    let d = |v: f64, l: f64, h: f64| (l - v).max(0.0).max(v - h);
    Vec3::new(d(p.x, lo.x, hi.x), d(p.y, lo.y, hi.y), d(p.z, lo.z, hi.z)).length()
}

fn boxes_distance(a: &Bbox3, b: &Bbox3) -> f64 {
    let (Some((alo, ahi)), Some((blo, bhi))) = (a.corners(), b.corners()) else {
        return f64::INFINITY;
    };
    let d = |al: f64, ah: f64, bl: f64, bh: f64| (bl - ah).max(0.0).max(al - bh);
    Vec3::new(
        d(alo.x, ahi.x, blo.x, bhi.x),
        d(alo.y, ahi.y, blo.y, bhi.y),
        d(alo.z, ahi.z, blo.z, bhi.z),
    )
    .length()
}

/// The edges of a face, each as often as its loops use it.
pub(crate) fn face_edges(body: &Body, face: usize) -> impl Iterator<Item = &Edge> {
    body.faces
        .get(face)
        .into_iter()
        .flat_map(|f| f.loops.iter())
        .filter_map(|l| body.loops.get(l.0 as usize))
        .flat_map(|l| l.coedges.iter())
        .filter_map(|c| body.edges.get(c.edge.0 as usize))
}

/// `n + 1` points along an edge, ends included.
pub(crate) fn edge_points(edge: &Edge, n: u32) -> impl Iterator<Item = Vec3> + '_ {
    let (t0, t1) = edge.range;
    (0..=n).map(move |k| {
        let s = f64::from(k) / f64::from(n.max(1));
        edge.curve.eval(t0 + (t1 - t0) * s)
    })
}

/// The point of an edge nearest to `p`, from the curve's own projection
/// (closed form for lines and circles), kept within the edge.
fn nearest_on_edge(edge: &Edge, p: Vec3) -> Option<Vec3> {
    let (t0, t1) = edge.range;
    let mut best: Option<(f64, Vec3)> = None;
    let mut consider = |t: f64| {
        let q = edge.curve.eval(t);
        let d = q.distance(p);
        if q.is_finite() && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, q));
        }
    };
    consider(t0);
    consider(t1);
    if let Ok(t) = edge.curve.project(p) {
        let periodic = matches!(edge.curve, Curve3::Circle(_) | Curve3::Ellipse(_));
        let turn = std::f64::consts::TAU;
        let shifts: &[f64] = if periodic {
            &[-2.0, -1.0, 0.0, 1.0, 2.0]
        } else {
            &[0.0]
        };
        for k in shifts {
            let t = t + k * turn;
            if t >= t0 && t <= t1 {
                consider(t);
            }
        }
    }
    best.map(|(_, q)| q)
}

/// The point of face `face` nearest to `p`: the foot on its surface when
/// that lies on the face, else the nearest point of its edges.
pub(crate) fn nearest_on_face(probe: &Probe<'_>, face: usize, p: Vec3) -> Answer<Vec3> {
    let body = probe.body();
    let f = body.faces.get(face).ok_or("a face is missing")?;
    if let Ok((u, v)) = f.surface.project(p) {
        let foot = f.surface.eval(u, v);
        if foot.is_finite() && probe.on_face(face, foot).map_err(|e| e.to_string())? != OnFace::Out
        {
            return Ok(foot);
        }
    }
    let mut best: Option<(f64, Vec3)> = None;
    for edge in face_edges(body, face) {
        if let Some(q) = nearest_on_edge(edge, p) {
            let d = q.distance(p);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, q));
            }
        }
    }
    match best {
        Some((_, q)) => Ok(q),
        // A face without edges (a whole sphere): its surface's foot.
        None => f
            .surface
            .project(p)
            .map(|(u, v)| f.surface.eval(u, v))
            .map_err(|e| e.to_string()),
    }
}

// ------------------------------------------------------------ distance

/// The nearest places of two sets of faces.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nearest {
    pub distance: f64,
    pub a: Vec3,
    pub b: Vec3,
    /// The faces they are on: (part, face) of each side.
    pub face_a: (usize, usize),
    pub face_b: (usize, usize),
}

/// The least distance between the faces of the parts `a` and those of
/// `b`.
pub(crate) fn distance(a: &[Part<'_>], b: &[Part<'_>]) -> Answer<Nearest> {
    type Key = ((usize, usize), (usize, usize));
    let mut faces_a: Vec<((usize, usize), &Bbox3)> = Vec::new();
    for (i, part) in a.iter().enumerate() {
        faces_a.extend(part.boxes.iter().map(|(&f, b)| ((i, f), b)));
    }
    let mut faces_b: Vec<((usize, usize), &Bbox3)> = Vec::new();
    for (i, part) in b.iter().enumerate() {
        faces_b.extend(part.boxes.iter().map(|(&f, b)| ((i, f), b)));
    }
    if faces_a.is_empty() || faces_b.is_empty() {
        return Err("there is nothing to measure from or to".into());
    }
    let mut pairs: Vec<(f64, Key)> = Vec::new();
    for (ka, ba) in &faces_a {
        for (kb, bb) in &faces_b {
            pairs.push((boxes_distance(ba, bb), (*ka, *kb)));
            if pairs.len() > 4_000_000 {
                return Err("more than four million pairs of faces; name fewer faces".into());
            }
        }
    }
    pairs.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let chord = a
        .iter()
        .chain(b.iter())
        .map(|p| p.chord)
        .fold(0.0_f64, f64::max);
    let slack = 2.0 * chord;
    let tris = |parts: &[Part<'_>], (i, f): (usize, usize)| -> Vec<[Vec3; 3]> {
        parts
            .get(i)
            .and_then(|p| p.triangles.get(&f))
            .cloned()
            .unwrap_or_default()
    };
    let mut best = f64::INFINITY;
    let mut work = 0_u64;
    let mut found: Vec<(f64, Key, Vec3, Vec3)> = Vec::new();
    for (box_distance, (ka, kb)) in pairs {
        if box_distance > best + slack {
            break;
        }
        let (ta, tb) = (tris(a, ka), tris(b, kb));
        let tb_boxes: Vec<Bbox3> = tb
            .iter()
            .map(|t| Bbox3::from_points(t.iter().copied()))
            .collect();
        let mut pair_best: Option<(f64, Vec3, Vec3)> = None;
        for t in &ta {
            let ta_box = Bbox3::from_points(t.iter().copied());
            for (u, ub) in tb.iter().zip(&tb_boxes) {
                if boxes_distance(&ta_box, ub) > best + slack {
                    continue;
                }
                work = work.saturating_add(1);
                if work > MAX_PAIRS {
                    return Err(format!(
                        "more than {MAX_PAIRS} pairs of triangles lie near one another; name fewer faces"
                    ));
                }
                let (d, p, q) = triangles_nearest(t, u);
                if pair_best.is_none_or(|(b, _, _)| d < b) {
                    pair_best = Some((d, p, q));
                }
                best = best.min(d);
            }
        }
        if let Some((d, p, q)) = pair_best {
            found.push((d, (ka, kb), p, q));
        }
        // A face pair without triangles: from points of their edges.
        if ta.is_empty() || tb.is_empty() {
            found.push((f64::INFINITY, (ka, kb), Vec3::ZERO, Vec3::ZERO));
        }
    }
    found.retain(|(d, ..)| *d <= best + slack || !d.is_finite());
    found.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    found.truncate(MAX_REFINED);
    let mut answer: Option<Nearest> = None;
    for (mesh_d, (ka, kb), p0, q0) in found {
        let (Some(pa), Some(pb)) = (a.get(ka.0), b.get(kb.0)) else {
            continue;
        };
        let start_p = if mesh_d.is_finite() {
            p0
        } else {
            face_box(pa.body(), ka.1).center().unwrap_or(p0)
        };
        let Ok((p, q)) = settle(&pa.probe, ka.1, &pb.probe, kb.1, start_p, q0) else {
            continue;
        };
        let d = p.distance(q);
        // The exact places are no further apart than the mesh's, beyond
        // what the mesh can be off by.
        if mesh_d.is_finite() && d > mesh_d + slack {
            continue;
        }
        if answer.is_none_or(|n| d < n.distance) {
            answer = Some(Nearest {
                distance: d,
                a: p,
                b: q,
                face_a: ka,
                face_b: kb,
            });
        }
    }
    answer.ok_or_else(|| "the nearest places could not be settled on the exact faces".into())
}

/// Nearest points of two faces by projecting onto each in turn from the
/// mesh's nearest pair.
fn settle(
    a: &Probe<'_>,
    fa: usize,
    b: &Probe<'_>,
    fb: usize,
    p0: Vec3,
    _q0: Vec3,
) -> Answer<(Vec3, Vec3)> {
    let mut p = nearest_on_face(a, fa, p0)?;
    let mut q = nearest_on_face(b, fb, p)?;
    for _ in 0..MAX_STEPS {
        let p2 = nearest_on_face(a, fa, q)?;
        let q2 = nearest_on_face(b, fb, p2)?;
        let scale = 1.0 + p2.length().max(q2.length());
        let moved = p2.distance(p).max(q2.distance(q));
        let better = p2.distance(q2) <= p.distance(q);
        if better {
            p = p2;
            q = q2;
        }
        if moved <= 1e-13 * scale || !better {
            break;
        }
    }
    Ok((p, q))
}

// ------------------------------------------------ triangle distances

fn closest_on_triangle(p: Vec3, [a, b, c]: &[Vec3; 3]) -> Vec3 {
    // After Ericson, Real-Time Collision Detection, 5.1.5.
    let (ab, ac, ap) = (*b - *a, *c - *a, p - *a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return *a;
    }
    let bp = p - *b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return *b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return if v.is_finite() { *a + ab * v } else { *a };
    }
    let cp = p - *c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return *c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return if w.is_finite() { *a + ac * w } else { *a };
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return if w.is_finite() {
            *b + (*c - *b) * w
        } else {
            *b
        };
    }
    let denom = va + vb + vc;
    let (v, w) = (vb / denom, vc / denom);
    if v.is_finite() && w.is_finite() {
        *a + ab * v + ac * w
    } else {
        *a
    }
}

fn segments_nearest(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    // After Ericson, 5.1.9.
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let eps = 1e-300;
    let (s, t);
    if a <= eps && e <= eps {
        return (p1, p2);
    }
    if a <= eps {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= eps {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let s0 = if denom > 0.0 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            } else {
                t = t0;
                s = s0;
            }
        }
    }
    let s = if s.is_finite() { s } else { 0.0 };
    let t = if t.is_finite() { t } else { 0.0 };
    (p1 + d1 * s, p2 + d2 * t)
}

/// The least distance between two triangles that do not cross, with the
/// nearest points.
fn triangles_nearest(t: &[Vec3; 3], u: &[Vec3; 3]) -> (f64, Vec3, Vec3) {
    let mut best = (f64::INFINITY, t[0], u[0]);
    let mut take = |p: Vec3, q: Vec3| {
        let d = p.distance(q);
        if d < best.0 {
            best = (d, p, q);
        }
    };
    for &p in t {
        take(p, closest_on_triangle(p, u));
    }
    for &q in u {
        take(closest_on_triangle(q, t), q);
    }
    let [t0, t1, t2] = *t;
    let [u0, u1, u2] = *u;
    for (a, b) in [(t0, t1), (t1, t2), (t2, t0)] {
        for (c, d) in [(u0, u1), (u1, u2), (u2, u0)] {
            let (p, q) = segments_nearest(a, b, c, d);
            take(p, q);
        }
    }
    best
}

// ----------------------------------------------------------- thickness

/// The thinnest place of a body.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Thin {
    pub thickness: f64,
    /// Where it is measured from, on face `face`.
    pub at: Vec3,
    /// Where the line into the material from there comes out, on face
    /// `opposite`.
    pub to: Vec3,
    pub face: usize,
    pub opposite: usize,
    pub samples: usize,
}

/// The wall thickness at `p` of face `face`: how far a line into the
/// material along the inward normal goes before it leaves the body.
fn thickness_at(
    part: &Part<'_>,
    face: usize,
    p: Vec3,
    tol: f64,
) -> Answer<Option<(f64, Vec3, usize)>> {
    let Some(normal) = part.probe.normal(face, p).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let from = 4.0 * tol;
    let hits = part
        .probe
        .line_hits(p, -normal, from, f64::INFINITY)
        .map_err(|e| e.to_string())?;
    Ok(hits
        .iter()
        .find(|h: &&LineHit| h.t >= from && !h.along)
        .map(|h| (h.t, h.point, h.face)))
}

/// A point of face `face` near `p` on its surface, when it lies on the
/// face.
fn onto_face(part: &Part<'_>, face: usize, p: Vec3) -> Answer<Option<Vec3>> {
    let f = part.body().faces.get(face).ok_or("a face is missing")?;
    let Ok((u, v)) = f.surface.project(p) else {
        return Ok(None);
    };
    let foot = f.surface.eval(u, v);
    if !foot.is_finite() {
        return Ok(None);
    }
    Ok((part.probe.on_face(face, foot).map_err(|e| e.to_string())? == OnFace::In).then_some(foot))
}

/// Two directions square to `n` and to each other.
fn tangents(n: Vec3) -> Option<(Vec3, Vec3)> {
    let helper = if n.x.abs() < 0.6 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let e1 = n.cross(helper).normalized()?;
    let e2 = n.cross(e1).normalized()?;
    Some((e1, e2))
}

/// The thinnest place of the faces of `part` (the least of the
/// thicknesses at sampled points of every face, each of the least few
/// then moved along its face to where it is least).
pub(crate) fn thickness(part: &Part<'_>, tol: f64) -> Answer<Option<Thin>> {
    let faces: Vec<usize> = part.triangles.keys().copied().collect();
    if faces.is_empty() {
        return Ok(None);
    }
    let per_face = (MAX_SAMPLES / faces.len()).max(3);
    let mut samples: Vec<(f64, usize, Vec3, Vec3, usize)> = Vec::new();
    let mut count = 0_usize;
    for &face in &faces {
        let tris = part.triangles.get(&face).map_or(&[][..], Vec::as_slice);
        let stride = tris.len().div_ceil(per_face).max(1);
        for t in tris.iter().step_by(stride) {
            let c = (t[0] + t[1] + t[2]) * (1.0 / 3.0);
            let Some(p) = onto_face(part, face, c)? else {
                continue;
            };
            count += 1;
            if let Some((d, to, other)) = thickness_at(part, face, p, tol)? {
                samples.push((d, face, p, to, other));
            }
        }
    }
    samples.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut best: Option<Thin> = None;
    let mut tried_faces: Vec<usize> = Vec::new();
    for &(d, face, p, to, other) in &samples {
        if tried_faces.len() >= MAX_REFINED / 2 {
            break;
        }
        if tried_faces.contains(&face) {
            continue;
        }
        tried_faces.push(face);
        let step0 = part
            .triangles
            .get(&face)
            .and_then(|tris| tris.first())
            .map_or(part.chord, |t| t[0].distance(t[1]).max(t[1].distance(t[2])))
            .max(10.0 * tol);
        let (d, p, to, other) = descend(part, face, (d, p, to, other), step0, tol)?;
        if best.is_none_or(|b| d < b.thickness) {
            best = Some(Thin {
                thickness: d,
                at: p,
                to,
                face,
                opposite: other,
                samples: count,
            });
        }
    }
    Ok(best)
}

/// A pattern search along the face for where the thickness is least.
fn descend(
    part: &Part<'_>,
    face: usize,
    start: (f64, Vec3, Vec3, usize),
    step0: f64,
    tol: f64,
) -> Answer<(f64, Vec3, Vec3, usize)> {
    let mut cur = start;
    let mut step = step0;
    let stop = (tol * 1e-3).max(1e-12 * (1.0 + cur.1.length()));
    for _ in 0..MAX_STEPS {
        if step < stop {
            break;
        }
        let Some(normal) = part.probe.normal(face, cur.1).map_err(|e| e.to_string())? else {
            break;
        };
        let Some((e1, e2)) = tangents(normal) else {
            break;
        };
        let r = std::f64::consts::FRAC_1_SQRT_2;
        let mut moved = false;
        for dir in [
            e1,
            -e1,
            e2,
            -e2,
            (e1 + e2) * r,
            (e1 - e2) * r,
            (e2 - e1) * r,
            -(e1 + e2) * r,
        ] {
            let Some(p) = onto_face(part, face, cur.1 + dir * step)? else {
                continue;
            };
            if let Some((d, to, other)) = thickness_at(part, face, p, tol)?
                && d < cur.0
            {
                cur = (d, p, to, other);
                moved = true;
                break;
            }
        }
        if !moved {
            step *= 0.5;
        }
    }
    Ok(cur)
}

// ------------------------------------------------------------- through

/// Whether the material is cut through along the axis of a cylindrical or
/// conical face.
#[derive(Clone, Debug)]
pub(crate) struct Through {
    pub through: bool,
    /// The axis: a point on it in the middle of the face and its
    /// direction.
    pub middle: Vec3,
    pub axis: Vec3,
    /// How far the face reaches either way from `middle` along the axis.
    pub half_length: f64,
    /// Why not, when it is not.
    pub why: Option<String>,
    /// The first face met each way (+axis, -axis), with its distance from
    /// `middle`.
    pub blocked: [Option<(usize, f64, Vec3)>; 2],
}

/// The axis of a cylinder or a cone.
pub(crate) fn axis_of(surface: &Surface) -> Option<(Vec3, Vec3)> {
    match surface {
        Surface::Cylinder(s) => Some((s.frame().origin(), s.frame().z())),
        Surface::Cone(s) => Some((s.frame().origin(), s.frame().z())),
        _ => None,
    }
}

pub(crate) fn through(part: &Part<'_>, face: usize, tol: f64) -> Answer<Through> {
    let body = part.body();
    let f = body.faces.get(face).ok_or("a face is missing")?;
    let (origin, axis) = axis_of(&f.surface).ok_or_else(|| {
        format!(
            "the face is a {}; through is asked of a cylindrical or conical face",
            f.surface.kind()
        )
    })?;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for edge in face_edges(body, face) {
        for p in edge_points(edge, EDGE_SAMPLES) {
            let s = (p - origin).dot(axis);
            if s.is_finite() {
                lo = lo.min(s);
                hi = hi.max(s);
            }
        }
    }
    if !(lo.is_finite() && hi.is_finite()) {
        return Err("the face has no bounded extent along its axis".into());
    }
    let half_length = 0.5 * (hi - lo);
    let middle = origin + axis * (0.5 * (lo + hi));
    let mut result = Through {
        through: false,
        middle,
        axis,
        half_length,
        why: None,
        blocked: [None, None],
    };
    match part.probe.place(middle).map_err(|e| e.to_string())? {
        Place::Outside => {}
        Place::Inside => {
            result.why =
                Some("the axis runs in the material: the face is not the wall of a hole".into());
            return Ok(result);
        }
        Place::OnBoundary => {
            result.why = Some("the axis lies on a face of the body".into());
            return Ok(result);
        }
        Place::Unknown => {
            return Err("where the axis lies could not be told (every ray grazed the body)".into());
        }
    }
    let from = 2.0 * tol;
    for (k, dir) in [axis, -axis].into_iter().enumerate() {
        let hits = part
            .probe
            .line_hits(middle, dir, from, f64::INFINITY)
            .map_err(|e| e.to_string())?;
        if let (Some(hit), Some(slot)) = (hits.first(), result.blocked.get_mut(k)) {
            *slot = Some((hit.face, hit.t, hit.point));
        }
    }
    result.through = result.blocked.iter().all(Option::is_none);
    if !result.through {
        result.why = Some("material lies on the axis beyond the hole".into());
    }
    Ok(result)
}

// ------------------------------------------------------------ symmetry

/// How far the faces are from being their own mirror image.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mirror {
    /// The largest distance from a mirrored point of a face to the nearest
    /// face.
    pub deviation: f64,
    /// The point (before mirroring) where it is largest.
    pub worst: Vec3,
    pub worst_part: usize,
    pub worst_face: usize,
    pub samples: usize,
}

/// Mirrors points of the faces of `parts` (their mesh's vertices, which
/// lie on the exact faces, and the middles of its triangles put on them) in the plane through `point` square to
/// `normal` and finds how far each lands from the nearest of those faces.
/// Every face is mirrored, so a feature on one side only is found from
/// its own faces, which land in the material or the air of the other.
pub(crate) fn symmetry(parts: &[Part<'_>], point: Vec3, normal: Vec3) -> Answer<Mirror> {
    let n = normal.normalized().ok_or("the plane's normal is zero")?;
    let mut samples: Vec<(usize, usize, Vec3)> = Vec::new();
    let mut total = 0_usize;
    for part in parts {
        total += part.triangles.values().map(Vec::len).sum::<usize>() * 4;
    }
    let stride = total.div_ceil(MAX_SAMPLES).max(1);
    // A mesh may have no vertex inside a face (a cylinder's strip from
    // circle to circle): each triangle's middle, put on the face's
    // surface, samples the inside too.
    let mut k = 0_usize;
    for (i, part) in parts.iter().enumerate() {
        for (&face, tris) in &part.triangles {
            let surface = part.body().faces.get(face).map(|f| &f.surface);
            for t in tris {
                let middle = (t[0] + t[1] + t[2]) * (1.0 / 3.0);
                let inside = surface
                    .and_then(|s| s.project(middle).ok().map(|(u, v)| s.eval(u, v)))
                    .filter(|p| p.is_finite());
                for p in t.iter().copied().chain(inside) {
                    if k.is_multiple_of(stride) {
                        samples.push((i, face, p));
                    }
                    k += 1;
                }
            }
        }
    }
    if samples.is_empty() {
        return Err("there is nothing to mirror".into());
    }
    let mut boxes: Vec<(usize, usize, &Bbox3)> = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        boxes.extend(part.boxes.iter().map(|(&f, b)| (i, f, b)));
    }
    let mut worst = Mirror {
        deviation: 0.0,
        worst: point,
        worst_part: 0,
        worst_face: 0,
        samples: samples.len(),
    };
    for &(i, face, p) in &samples {
        let m = p - n * (2.0 * (p - point).dot(n));
        let mut order: Vec<(f64, usize, usize)> = boxes
            .iter()
            .map(|&(j, f, b)| (box_distance(b, m), j, f))
            .collect();
        order.sort_by(|x, y| x.0.total_cmp(&y.0).then((x.1, x.2).cmp(&(y.1, y.2))));
        let mut best = f64::INFINITY;
        for (lower, j, f) in order {
            if lower > best {
                break;
            }
            let Some(part) = parts.get(j) else {
                continue;
            };
            let q = nearest_on_face(&part.probe, f, m)?;
            best = best.min(q.distance(m));
        }
        if best > worst.deviation {
            worst.deviation = best;
            worst.worst = p;
            worst.worst_part = i;
            worst.worst_face = face;
        }
    }
    Ok(worst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_distances_are_exact_for_simple_cases() {
        let t = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let u = [
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(1.0, 0.0, 2.0),
            Vec3::new(0.0, 1.0, 2.0),
        ];
        assert!((triangles_nearest(&t, &u).0 - 2.0).abs() < 1e-15);
        let v = [
            Vec3::new(2.0, 2.0, 0.0),
            Vec3::new(3.0, 2.0, 0.0),
            Vec3::new(2.0, 3.0, 0.0),
        ];
        // From the hypotenuse's middle (0.5, 0.5) to the corner (2, 2).
        assert!((triangles_nearest(&t, &v).0 - (4.5_f64).sqrt()).abs() < 1e-12);
    }
}
