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
use std::collections::{BTreeMap, BTreeSet};

/// Most triangles of the mesh of one body.
const MAX_TRIANGLES: usize = 400_000;
/// The angle criterion of the meshes (radians): what bounds how far a
/// round face can lie from its triangles.
const MESH_ANGLE: f64 = 12.0 * std::f64::consts::PI / 180.0;
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
    /// Every face of the body (taking part or not): its triangles and box.
    pub every: BTreeMap<usize, Vec<[Vec3; 3]>>,
    pub every_boxes: BTreeMap<usize, Bbox3>,
    /// Per face of the body: how far a point of the exact face can be from
    /// the face's triangles. 0 for a flat face bounded by lines (its
    /// triangles cover it exactly), the chordal tolerance otherwise (the
    /// tessellator's promise).
    pub err: Vec<f64>,
    /// The document's linear tolerance.
    pub linear: f64,
    /// Per edge of the body: the box of [`EDGE_SAMPLES`] points along it
    /// grown by the chord, when first needed.
    edge_boxes: std::cell::OnceCell<Vec<Bbox3>>,
}

impl<'a> Part<'a> {
    /// Per edge of the body, the box around points along it.
    fn edge_boxes(&self) -> &[Bbox3] {
        self.edge_boxes.get_or_init(|| {
            self.body()
                .edges
                .iter()
                .map(|e| {
                    grown(
                        &Bbox3::from_points(edge_points(e, EDGE_SAMPLES)),
                        self.chord,
                    )
                })
                .collect()
        })
    }

    /// `faces`: the faces taking part, or every face when `None`.
    pub fn new(
        body: &'a Body,
        faces: Option<&[usize]>,
        tolerance: &craft3d_math::Tolerance,
    ) -> Answer<Self> {
        let probe = Probe::new(body, tolerance).map_err(|e| e.to_string())?;
        let (mesh, chord) = mesh_of(body, tolerance.linear)?;
        let mut every: BTreeMap<usize, Vec<[Vec3; 3]>> = BTreeMap::new();
        for face in 0..body.faces.len() {
            every.insert(face, Vec::new());
        }
        for (t, &face) in mesh.face_ids.iter().enumerate() {
            if let (Some(corners), Some(list)) = (mesh.corners(t), every.get_mut(&(face as usize)))
            {
                list.push(corners);
            }
        }
        let wanted = |face: usize| faces.is_none_or(|list| list.contains(&face));
        let triangles: BTreeMap<usize, Vec<[Vec3; 3]>> = every
            .iter()
            .filter(|(f, _)| wanted(**f))
            .map(|(f, t)| (*f, t.clone()))
            .collect();
        let box_of = |face: usize, tris: &[[Vec3; 3]]| {
            let mut b = Bbox3::from_points(tris.iter().flatten().copied());
            if b.is_empty() {
                b = face_box(body, face);
            }
            grown(&b, chord)
        };
        let every_boxes: BTreeMap<usize, Bbox3> =
            every.iter().map(|(&f, t)| (f, box_of(f, t))).collect();
        let boxes = triangles
            .keys()
            .filter_map(|f| every_boxes.get(f).map(|b| (*f, *b)))
            .collect();
        let err = body
            .faces
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let straight = face_edges(body, i).all(|e| matches!(e.curve, Curve3::Line(_)));
                // The mesh holds a curved face's triangles to the angle
                // (corner normals at most MESH_ANGLE apart) unless a side
                // is shorter than the chord: so a round of radius r is
                // off by no more than r (1 − cos(MESH_ANGLE / 2)), or the
                // sagitta of a chord-long side.
                let round = |r: f64| {
                    let sagitta = if chord >= 2.0 * r {
                        r
                    } else {
                        r - (r * r - chord * chord / 4.0).max(0.0).sqrt()
                    };
                    chord.min((r * (1.0 - (0.5 * MESH_ANGLE).cos())).max(sagitta))
                };
                match &f.surface {
                    Surface::Plane(_) if straight => 0.0,
                    Surface::Cylinder(c) => round(c.radius()),
                    Surface::Sphere(c) => round(c.radius()),
                    Surface::Torus(t) => {
                        let (major, minor) = t.radii();
                        round(major + minor)
                    }
                    _ => chord,
                }
            })
            .collect();
        Ok(Part {
            probe,
            chord,
            triangles,
            boxes,
            every,
            every_boxes,
            err,
            linear: tolerance.linear,
            edge_boxes: std::cell::OnceCell::new(),
        })
    }

    pub fn err(&self, face: usize) -> f64 {
        self.err.get(face).copied().unwrap_or(self.chord)
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
            max_angle: Some(MESH_ANGLE),
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

/// Whether two boxes share a point.
fn overlaps(a: &Bbox3, b: &Bbox3) -> bool {
    match (a.corners(), b.corners()) {
        (Some((alo, ahi)), Some((blo, bhi))) => {
            alo.x <= bhi.x
                && blo.x <= ahi.x
                && alo.y <= bhi.y
                && blo.y <= ahi.y
                && alo.z <= bhi.z
                && blo.z <= ahi.z
        }
        _ => false,
    }
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
/// (closed form for lines and circles), kept within the edge; and its
/// parameter on the curve.
fn nearest_on_edge(edge: &Edge, p: Vec3) -> Option<(Vec3, f64)> {
    let (t0, t1) = edge.range;
    let mut best: Option<(f64, Vec3, f64)> = None;
    let mut consider = |t: f64| {
        let q = edge.curve.eval(t);
        let d = q.distance(p);
        if q.is_finite() && best.is_none_or(|(b, _, _)| d < b) {
            best = Some((d, q, t));
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
    best.map(|(_, q, t)| (q, t))
}

/// Most Newton steps of [`nearest_on_edge_from`].
const EDGE_STEPS: usize = 50;

/// The point of an edge nearest to `p` found by Newton's method on the
/// curve from the parameter `start` alone (a few evaluations, where the
/// curve's own projection of a freeform curve samples it whole), with
/// its parameter: `None` unless it settles strictly inside the edge's
/// range (at an end the nearest point may be on the next edge), with the
/// distance to `p` square to the curve there and no further from `p`
/// than the point at `start`. For a point near one found on the edge
/// before.
fn nearest_on_edge_from(edge: &Edge, p: Vec3, start: f64) -> Option<(Vec3, f64)> {
    let (t0, t1) = edge.range;
    let span = t1 - t0;
    if !(span.is_finite() && span > 0.0 && start.is_finite()) {
        return None;
    }
    let h = span * 1e-6;
    let mut t = start.clamp(t0, t1);
    let mut settled = false;
    for _ in 0..EDGE_STEPS {
        // Minimise |C(t) - p|²: Newton on g(t) = (C - p)·C'.
        let (c, d1) = edge.curve.derivatives(t);
        let g = (c - p).dot(d1);
        let d2 = (edge.curve.derivatives(t + h).1 - edge.curve.derivatives(t - h).1) * (0.5 / h);
        let slope = d1.length_squared() + (c - p).dot(d2);
        if !(slope.is_finite() && slope > 0.0) {
            return None;
        }
        let next = (t - g / slope).clamp(t0, t1);
        let moved = (next - t).abs();
        t = next;
        if moved <= 1e-15 * span {
            settled = true;
            break;
        }
    }
    let inside = t > t0 + 1e-9 * span && t < t1 - 1e-9 * span;
    let q = edge.curve.eval(t);
    let from = edge.curve.eval(start.clamp(t0, t1));
    (settled && inside && q.is_finite() && q.distance(p) <= from.distance(p)).then_some((q, t))
}

/// The point of face `face` nearest to `p`: the foot on its surface when
/// that lies on the face, else the nearest point of its edges.
pub(crate) fn nearest_on_face(probe: &Probe<'_>, face: usize, p: Vec3) -> Answer<Vec3> {
    let mut near = Near::default();
    nearest_on_face_at(probe, face, p, &mut near)
}

/// Where the last answer of [`nearest_on_face_near`] on a face was: the
/// parameters of the foot on the face's surface (on the face or not),
/// and when the answer was on an edge, that edge (its index in the body)
/// and the parameter on its curve.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Near {
    foot: Option<(f64, f64)>,
    edge: Option<(usize, f64)>,
}

/// [`nearest_on_face`], noting in `near` where the answer is.
fn nearest_on_face_at(probe: &Probe<'_>, face: usize, p: Vec3, near: &mut Near) -> Answer<Vec3> {
    let body = probe.body();
    let f = body.faces.get(face).ok_or("a face is missing")?;
    *near = Near::default();
    if let Ok((u, v)) = f.surface.project(p) {
        near.foot = Some((u, v));
        let foot = f.surface.eval(u, v);
        if foot.is_finite() && probe.on_face(face, foot).map_err(|e| e.to_string())? != OnFace::Out
        {
            return Ok(foot);
        }
    }
    let mut best: Option<(f64, Vec3, usize, f64)> = None;
    for coedge in face_coedges(body, face) {
        let Some(edge) = body.edges.get(coedge) else {
            continue;
        };
        if let Some((q, t)) = nearest_on_edge(edge, p) {
            let d = q.distance(p);
            if best.is_none_or(|(b, ..)| d < b) {
                best = Some((d, q, coedge, t));
            }
        }
    }
    match best {
        Some((_, q, edge, t)) => {
            near.edge = Some((edge, t));
            Ok(q)
        }
        // A face without edges (a whole sphere): its surface's foot.
        None => f
            .surface
            .project(p)
            .map(|(u, v)| f.surface.eval(u, v))
            .map_err(|e| e.to_string()),
    }
}

/// The indices (in the body) of the edges of a face, each as often as
/// its loops use it: [`face_edges`] by index.
fn face_coedges(body: &Body, face: usize) -> impl Iterator<Item = usize> + '_ {
    body.faces
        .get(face)
        .into_iter()
        .flat_map(|f| f.loops.iter())
        .filter_map(|l| body.loops.get(l.0 as usize))
        .flat_map(|l| l.coedges.iter())
        .map(|c| c.edge.0 as usize)
}

/// [`nearest_on_face`] for a point near the one the last call for this
/// face answered (`near` says where that was): each step starts from
/// there instead of searching the whole surface and every edge, which
/// for freeform faces costs hundreds of evaluations where a descent
/// costs a few (`Surface::project_from`, [`nearest_on_edge_from`]). The
/// surface's foot found from the last foot is the answer when it lies on
/// the face; else, when the last answer was on an edge, that edge's
/// nearest point found from there, when it lies strictly inside the
/// edge. Anything else (no last answer, a descent that does not settle,
/// a point that reaches the end of its edge) is [`nearest_on_face`].
/// The answer is always a point of the face; it is the face's nearest
/// to `p` as far as the descents can tell, which for a run of points
/// that each move a little, as [`settle`] makes, is where the search
/// lands too.
fn nearest_on_face_near(probe: &Probe<'_>, face: usize, p: Vec3, near: &mut Near) -> Answer<Vec3> {
    let body = probe.body();
    let f = body.faces.get(face).ok_or("a face is missing")?;
    if let Some(start) = near.foot
        && let Ok((u, v)) = f.surface.project_from(p, start)
    {
        let foot = f.surface.eval(u, v);
        if foot.is_finite() {
            if probe.on_face(face, foot).map_err(|e| e.to_string())? != OnFace::Out {
                *near = Near {
                    foot: Some((u, v)),
                    edge: None,
                };
                return Ok(foot);
            }
            if let Some((index, t)) = near.edge
                && let Some(edge) = body.edges.get(index)
                && let Some((q, t)) = nearest_on_edge_from(edge, p, t)
            {
                *near = Near {
                    foot: Some((u, v)),
                    edge: Some((index, t)),
                };
                return Ok(q);
            }
        }
    }
    nearest_on_face_at(probe, face, p, near)
}

// ------------------------------------------------------------ distance

/// The nearest places of two sets of faces.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nearest {
    /// Between the two places found: no less than the least distance.
    pub distance: f64,
    /// Proven no more than the least distance: from the mesh, less what
    /// the mesh can be off by.
    pub lower: f64,
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
    // Per pair of faces compared: what the mesh proves.
    let mut meshed: Vec<(Key, f64)> = Vec::new();
    // Per pair of faces compared on their triangles: the least distance of
    // the pairs of triangles not left out (none: every one was).
    let mut compared: Vec<(Key, Option<f64>)> = Vec::new();
    // The pairs not compared are at least this far apart.
    let mut beyond = f64::INFINITY;
    let err = |parts: &[Part<'_>], (i, f): (usize, usize)| parts.get(i).map_or(chord, |p| p.err(f));
    for (box_distance, (ka, kb)) in pairs {
        if box_distance > best + slack {
            beyond = box_distance;
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
            meshed.push(((ka, kb), box_distance));
            found.push((f64::INFINITY, (ka, kb), Vec3::ZERO, Vec3::ZERO));
        } else {
            compared.push(((ka, kb), pair_best.map(|(d, ..)| d)));
        }
    }
    for (key, least) in compared {
        meshed.push((
            key,
            mesh_proves(least, best, slack, err(a, key.0), err(b, key.1)),
        ));
    }
    found.retain(|(d, ..)| *d <= best + slack || !d.is_finite());
    found.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    found.truncate(MAX_REFINED);
    // The document's tolerance (the largest of the parts').
    let linear = a
        .iter()
        .chain(b.iter())
        .map(|p| p.linear)
        .fold(0.0_f64, f64::max);
    // Each candidate first takes a few steps; then, in order, the ones
    // that at the rate of their last steps could still come nearer than
    // the nearest settled so far take the rest of theirs ([`Settling`]).
    let mut settling: Vec<(f64, Key, Settling)> = Vec::new();
    for (mesh_d, (ka, kb), p0, _) in found {
        let (Some(pa), Some(pb)) = (a.get(ka.0), b.get(kb.0)) else {
            continue;
        };
        let start_p = if mesh_d.is_finite() {
            p0
        } else {
            face_box(pa.body(), ka.1).center().unwrap_or(p0)
        };
        let Ok(mut s) = Settling::start(&pa.probe, ka.1, &pb.probe, kb.1, start_p) else {
            continue;
        };
        if s.advance(
            &pa.probe,
            ka.1,
            &pb.probe,
            kb.1,
            FIRST_STEPS,
            linear,
            f64::INFINITY,
        )
        .is_ok()
        {
            settling.push((mesh_d, (ka, kb), s));
        }
    }
    // The exact places are no further apart than the mesh's, beyond what
    // the mesh can be off by.
    let fits = |mesh_d: f64, d: f64| !(mesh_d.is_finite() && d > mesh_d + slack);
    let mut nearest = settling
        .iter()
        .map(|(mesh_d, _, s)| (*mesh_d, s.distance()))
        .filter(|(mesh_d, d)| fits(*mesh_d, *d))
        .fold(f64::INFINITY, |n, (_, d)| n.min(d));
    let mut answer: Option<Nearest> = None;
    for (mesh_d, (ka, kb), mut s) in settling {
        let (Some(pa), Some(pb)) = (a.get(ka.0), b.get(kb.0)) else {
            continue;
        };
        let steps = MAX_STEPS.saturating_sub(s.taken);
        if s.advance(&pa.probe, ka.1, &pb.probe, kb.1, steps, linear, nearest)
            .is_err()
        {
            continue;
        }
        let d = s.distance();
        if !fits(mesh_d, d) {
            continue;
        }
        nearest = nearest.min(d);
        if answer.is_none_or(|n| d < n.distance) {
            answer = Some(Nearest {
                distance: d,
                lower: 0.0,
                a: s.p,
                b: s.q,
                face_a: ka,
                face_b: kb,
            });
        }
    }
    let mut answer = answer.ok_or("the nearest places could not be settled on the exact faces")?;
    // What the mesh cannot prove close enough, the exact faces do
    // (verify_bound.rs): per pair of faces the better of the two.
    let close = (0.25 * linear).max(1e-12 * answer.distance);
    let short: Vec<Key> = meshed
        .iter()
        .filter(|(_, l)| *l < answer.distance - close)
        .map(|(k, _)| *k)
        .collect();
    let mut proven: BTreeMap<Key, f64> = BTreeMap::new();
    if !short.is_empty() && answer.distance > close {
        let proof = crate::verify_bound::prove(a, b, &short, answer.distance, close, linear);
        for (key, lower) in short.iter().zip(proof.lower) {
            // No pair of strata bounded (all ruled out) is not taken as
            // a proof of anything.
            if let Some(lower) = lower.filter(|l| l.is_finite()) {
                proven.insert(*key, lower);
            }
        }
        if let Some(n) = proof.nearer
            && n.distance < answer.distance
        {
            answer.distance = n.distance;
            answer.a = n.a;
            answer.b = n.b;
            answer.face_a = n.face_a;
            answer.face_b = n.face_b;
        }
    }
    let lower = meshed
        .iter()
        .map(|(k, l)| proven.get(k).map_or(*l, |p| p.max(*l)))
        .fold(beyond, f64::min);
    answer.lower = lower.min(answer.distance).max(0.0);
    Ok(answer)
}

/// What the mesh proves of a pair of faces compared on their triangles:
/// `least` the least distance of their pairs of triangles not left out
/// (`None`: every one was). A pair of triangles left out was further
/// apart than `best + slack` at the time, so than the final `best +
/// slack`: the pair of faces is at least the lesser of that and its own
/// least, less what each face can be off by (`err_a`, `err_b`). A pair all
/// of whose triangles were left out proves `best + slack` less that, not
/// nothing: the pair is counted.
pub(crate) fn mesh_proves(
    least: Option<f64>,
    best: f64,
    slack: f64,
    err_a: f64,
    err_b: f64,
) -> f64 {
    let d = least.map_or(best + slack, |d| d.min(best + slack));
    d - err_a - err_b
}

/// Most times one step of [`settle`] doubles its jump ahead.
const MAX_JUMPS: usize = 30;
/// The steps over which [`settle`] measures how fast it still comes
/// nearer.
const STALL_WINDOW: usize = 8;
/// What the steps a settling has left may still gain, at the rate of its
/// last [`STALL_WINDOW`], before it stops: this fraction of the tolerance.
const STALL: f64 = 1e-4;
/// The steps every candidate of a [`distance`] takes before those that
/// can no longer come nearer than the nearest one so far are left.
const FIRST_STEPS: usize = 2 * STALL_WINDOW;

/// Nearest points of two faces by projecting onto each in turn from the
/// mesh's nearest pair: [`Settling`] taken to its end.
#[cfg(test)]
fn settle(
    a: &Probe<'_>,
    fa: usize,
    b: &Probe<'_>,
    fb: usize,
    p0: Vec3,
    tol: f64,
) -> Answer<(Vec3, Vec3)> {
    let mut s = Settling::start(a, fa, b, fb, p0)?;
    s.advance(a, fa, b, fb, MAX_STEPS, tol, f64::INFINITY)?;
    Ok((s.p, s.q))
}

/// The settling of the nearest points of two faces, `p` on face `fa` of
/// `a` and `q` on `fb` of `b`, projecting onto each in turn from the
/// mesh's nearest pair, which can be taken further ([`Settling::advance`]).
///
/// Projecting in turn closes in on the nearest points at a constant ratio
/// per step, which is near 1 where the faces run nearly parallel (the
/// flanks of two gears in mesh, a shaft in its bore): there it took all
/// [`MAX_STEPS`] and still stopped short. So each step that came nearer
/// also jumps ahead along the way it moved, twice as far each time while
/// that comes nearer still (the point moved by the step, carried on and
/// projected back onto both faces in turn); a jump that does not come
/// nearer is not taken. Every pair of points kept lies on the two faces
/// and is no further apart than the one before, so the answer is never
/// worse than projecting in turn alone. Each projection starts from where
/// the last one on the same face landed ([`nearest_on_face_near`]).
///
/// Where the two faces run beside each other along a line and part only
/// slowly along it (a hole's wall beside a plate's rim, both nearly along
/// the plate's normal), each step creeps along that line and comes nearer
/// by a hundred-billionth of the distance, and every one of the steps was
/// taken, each projection onto a long freeform face costing up to a fifth
/// of a second (the sea kayak's holes to their own plate's rim, repro 25:
/// thirteen minutes). So a settling also stops when what it came nearer
/// over its last [`STALL_WINDOW`] steps, spread over the steps it has
/// left, is less than [`STALL`] times the tolerance: a settling that
/// closes in at a ratio, or at a rate that falls, gains no more than that
/// by going on to the end of its steps. (Where it closes in fast, the
/// jumps ahead take it there first, and it stops as before.) By the same
/// rate, one that could not come nearer than `beat` (the nearest pair of
/// another settling) in the steps it has left stops too: it is not the
/// nearest (repro 25 again: the walls of a few holes crept beside the rim
/// for all their steps, a seventh of a millimetre further than the
/// nearest hole). Most of the time there went in telling the rim's
/// points in or out of the rim face, a winding number on a strip four
/// millimetres by five metres; that is the chart's matter
/// (`PatchTree::evened` in the kernel), and takes milliseconds now.
struct Settling {
    p: Vec3,
    q: Vec3,
    near_a: Near,
    near_b: Near,
    /// The distance after each step taken, the first before any.
    came: Vec<f64>,
    /// The steps taken, of [`MAX_STEPS`].
    taken: usize,
    /// It has settled, or stopped for good.
    done: bool,
}

impl Settling {
    fn start(a: &Probe<'_>, fa: usize, b: &Probe<'_>, fb: usize, p0: Vec3) -> Answer<Self> {
        let (mut near_a, mut near_b) = (Near::default(), Near::default());
        let p = nearest_on_face_near(a, fa, p0, &mut near_a)?;
        let q = nearest_on_face_near(b, fb, p, &mut near_b)?;
        Ok(Settling {
            p,
            q,
            near_a,
            near_b,
            came: vec![p.distance(q)],
            taken: 0,
            done: false,
        })
    }

    fn distance(&self) -> f64 {
        self.p.distance(self.q)
    }

    /// At most `steps` more steps (and [`MAX_STEPS`] in all), stopping
    /// where it has settled, where it has stalled (to `tol`) or where it
    /// could not come nearer than `beat` ([`Settling`]).
    #[allow(clippy::too_many_arguments)]
    fn advance(
        &mut self,
        a: &Probe<'_>,
        fa: usize,
        b: &Probe<'_>,
        fb: usize,
        steps: usize,
        tol: f64,
        beat: f64,
    ) -> Answer<()> {
        let mut steps = steps.min(MAX_STEPS.saturating_sub(self.taken));
        while !self.done && steps > 0 {
            steps -= 1;
            self.step(a, fa, b, fb, tol, beat)?;
        }
        Ok(())
    }

    fn step(
        &mut self,
        a: &Probe<'_>,
        fa: usize,
        b: &Probe<'_>,
        fb: usize,
        tol: f64,
        beat: f64,
    ) -> Answer<()> {
        let (p, q) = (self.p, self.q);
        let mut p2 = nearest_on_face_near(a, fa, q, &mut self.near_a)?;
        let mut q2 = nearest_on_face_near(b, fb, p2, &mut self.near_b)?;
        let better = p2.distance(q2) <= p.distance(q);
        if better {
            let way = p2 - p;
            let mut ahead = 1.0;
            for _ in 0..MAX_JUMPS {
                let target = p2 + way * ahead;
                if !target.is_finite() {
                    break;
                }
                // A jump that cannot be projected is not taken.
                let (mut na, mut nb) = (self.near_a, self.near_b);
                let Ok(p3) = nearest_on_face_near(a, fa, target, &mut na) else {
                    break;
                };
                let Ok(q3) = nearest_on_face_near(b, fb, p3, &mut nb) else {
                    break;
                };
                if p3.distance(q3) >= p2.distance(q2) {
                    break;
                }
                (p2, q2, self.near_a, self.near_b) = (p3, q3, na, nb);
                ahead *= 2.0;
            }
        }
        self.taken += 1;
        let scale = 1.0 + p2.length().max(q2.length());
        let moved = p2.distance(p).max(q2.distance(q));
        if better {
            self.p = p2;
            self.q = q2;
        }
        if moved <= 1e-13 * scale || !better || self.taken >= MAX_STEPS {
            self.done = true;
            return Ok(());
        }
        let last = self.distance();
        self.came.push(last);
        let left = MAX_STEPS.saturating_sub(self.taken) as f64;
        if let Some(&before) = self
            .came
            .len()
            .checked_sub(STALL_WINDOW + 1)
            .and_then(|i| self.came.get(i))
        {
            let rate = (before - last) / STALL_WINDOW as f64;
            if rate * left <= STALL * tol || last - rate * left > beat {
                self.done = true;
            }
        }
        Ok(())
    }
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
    /// The least found: the thickness at a real place, so no less than the
    /// least there is.
    pub thickness: f64,
    /// Proven no more than the least there is.
    pub lower: f64,
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
    limit: f64,
) -> Answer<Option<(f64, Vec3, usize)>> {
    let Some(normal) = part.probe.normal(face, p).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let from = 4.0 * tol;
    let hits = part
        .probe
        .line_hits(p, -normal, from, limit)
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

/// What a face is, for bounding how short a line into the material from
/// one face to another can be.
#[derive(Clone, Copy)]
enum Shape {
    /// A flat face: a point of its plane and its outward normal.
    Flat(Vec3, Vec3),
    /// A cylinder, sphere or torus: its axis (a point and a direction; a
    /// sphere's centre), its radius (a torus's tube radius and the radius
    /// of its middle circle), and whether the material is inside the
    /// round (convex: the outward normal points away from the axis).
    Round {
        origin: Vec3,
        axis: Vec3,
        radius: f64,
        major: Option<f64>,
        sphere: bool,
        convex: bool,
    },
    Other,
}

fn shape(body: &Body, face: usize) -> Shape {
    let Some(f) = body.faces.get(face) else {
        return Shape::Other;
    };
    let sign = if f.reversed { -1.0 } else { 1.0 };
    match &f.surface {
        Surface::Plane(p) => Shape::Flat(p.frame().origin(), p.frame().z() * sign),
        Surface::Cylinder(c) => Shape::Round {
            origin: c.frame().origin(),
            axis: c.frame().z(),
            radius: c.radius(),
            major: None,
            sphere: false,
            convex: !f.reversed,
        },
        Surface::Sphere(c) => Shape::Round {
            origin: c.frame().origin(),
            axis: c.frame().z(),
            radius: c.radius(),
            major: None,
            sphere: true,
            convex: !f.reversed,
        },
        Surface::Torus(t) => {
            let (major, minor) = t.radii();
            Shape::Round {
                origin: t.frame().origin(),
                axis: t.frame().z(),
                radius: minor,
                major: Some(major),
                sphere: false,
                convex: !f.reversed,
            }
        }
        _ => Shape::Other,
    }
}

/// The outward normal of a flat or round face at `p` (a point of it), and
/// how much the normal can turn between the corners of one of its
/// triangles and inside it (as a change of a dot product).
fn normal_at(part: &Part<'_>, shape: Shape, p: Vec3) -> Option<(Vec3, f64)> {
    match shape {
        Shape::Flat(_, n) => Some((n, 0.0)),
        Shape::Round {
            origin,
            axis,
            radius,
            major,
            sphere,
            convex,
        } => {
            let centre = if sphere {
                origin
            } else {
                let along = origin + axis * (p - origin).dot(axis);
                match major {
                    // The nearest point of the tube's middle circle.
                    Some(major) => {
                        origin
                            + (along - origin + (p - along))
                                .normalized()
                                .map_or(Vec3::ZERO, |d| {
                                    let flat = d - axis * d.dot(axis);
                                    flat.normalized().map_or(Vec3::ZERO, |f| f * major)
                                })
                    }
                    None => along,
                }
            };
            let n = (p - centre).normalized()?;
            let n = if convex { n } else { -n };
            // A side shorter than the chord is not held to the angle.
            Some((n, MESH_ANGLE.max(part.chord / radius.max(1e-300))))
        }
        Shape::Other => None,
    }
}

/// [`least_height`] over the triangles of `face` that hold a point where
/// `keep` (of the face's normal there, within how much it turns) may hold:
/// `None` when none does.
fn least_height_where(
    part: &Part<'_>,
    face: usize,
    o: Vec3,
    n: Vec3,
    keep: impl Fn(Vec3, f64) -> bool,
) -> Option<f64> {
    let tris = part.every.get(&face)?;
    let shape = shape(part.body(), face);
    let mut kept = Vec::new();
    for t in tris {
        let ok = t.iter().any(|&c| match normal_at(part, shape, c) {
            Some((m, turn)) => keep(m, turn),
            None => true,
        });
        if ok {
            kept.push(*t);
        }
    }
    if kept.is_empty() {
        return None;
    }
    Some(heights(&kept, o, n, part.err(face)))
}

fn heights(tris: &[[Vec3; 3]], o: Vec3, n: Vec3, err: f64) -> f64 {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in tris.iter().flatten() {
        let h = n.dot(*p - o);
        lo = lo.min(h);
        hi = hi.max(h);
    }
    if !(lo.is_finite() && hi.is_finite()) {
        return 0.0;
    }
    let least = if lo > 0.0 {
        lo
    } else if hi < 0.0 {
        -hi
    } else {
        0.0
    };
    (least - err).max(0.0)
}

/// The least of `|n·(p − o)|` over the face of `part` (its triangles'
/// corners, less how far the exact face can lie from them), or 0 when the
/// face reaches the plane.
fn least_height(part: &Part<'_>, face: usize, o: Vec3, n: Vec3) -> f64 {
    let Some(tris) = part.every.get(&face) else {
        return 0.0;
    };
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in tris.iter().flatten() {
        let h = n.dot(*p - o);
        lo = lo.min(h);
        hi = hi.max(h);
    }
    if !(lo.is_finite() && hi.is_finite()) {
        return 0.0;
    }
    let err = part.err(face);
    let least = if lo > 0.0 {
        lo
    } else if hi < 0.0 {
        -hi
    } else {
        0.0
    };
    (least - err).max(0.0)
}

/// A bound, proven, below the length of any line that leaves face `a` of
/// `part` into the material along its inward normal and comes out through
/// face `b` (`None`: no such line can come out there), and where to look
/// for a short one.
fn pair_bound(part: &Part<'_>, a: usize, b: usize, tol: f64) -> (Option<f64>, Vec<(usize, Vec3)>) {
    let body = part.body();
    let (sa, sb) = (shape(body, a), shape(body, b));
    let mut seeds = Vec::new();
    if a == b {
        let bound = match sa {
            // A line into the material from a flat face never meets its
            // plane again; one from a hole's wall never comes back to it.
            Shape::Flat(..) => None,
            Shape::Round {
                convex: false,
                major: None,
                ..
            } => None,
            // Through the axis (the centre, the tube's middle) to the other
            // side; a torus's line may meet the tube across its axis
            // first, no nearer than 2 (R − r).
            Shape::Round {
                convex: true,
                radius,
                major,
                ..
            } => Some(match major {
                Some(major) => (2.0 * radius).min(2.0 * (major - radius)).max(0.0),
                None => 2.0 * radius,
            }),
            Shape::Round {
                convex: false,
                radius,
                major: Some(major),
                ..
            } => Some((2.0 * (major - radius)).max(0.0)),
            Shape::Other => Some(0.0),
        };
        return (bound, seeds);
    }
    let boxes = match (part.every_boxes.get(&a), part.every_boxes.get(&b)) {
        (Some(x), Some(y)) => boxes_distance(x, y),
        _ => 0.0,
    };
    // A line square to a cylinder's axis never comes out through a flat
    // face square to that axis, and a line along the axis never through
    // the cylinder: they only graze.
    let cylinder = |s: Shape| match s {
        Shape::Round {
            axis,
            major: None,
            sphere: false,
            ..
        } => Some(axis),
        _ => None,
    };
    match (sa, sb) {
        (Shape::Flat(_, na), _) if cylinder(sb).is_some_and(|z| na.cross(z).length() < 1e-12) => {
            return (None, seeds);
        }
        (_, Shape::Flat(_, nb)) if cylinder(sa).is_some_and(|z| nb.cross(z).length() < 1e-12) => {
            return (None, seeds);
        }
        _ => {}
    }
    let mut bound = boxes;
    match (sa, sb) {
        (Shape::Flat(oa, na), Shape::Flat(ob, nb)) => {
            // Leaving `a` along −na and coming out of `b` needs na·nb < 0.
            if na.dot(nb) > -1e-12 {
                return (None, seeds);
            }
            if na.cross(nb).length() < 1e-12 {
                bound = bound.max(nb.dot(oa - ob).abs());
            }
            bound = bound
                .max(least_height(part, a, ob, nb))
                .max(least_height(part, b, oa, na));
        }
        // A line leaves a round face `a` along −n_a and comes out of the
        // flat `b` only where n_a·n_b < 0: only the triangles of `a` that
        // may hold such a point count.
        (_, Shape::Flat(ob, nb)) => {
            match least_height_where(part, a, ob, nb, |m, turn| m.dot(nb) <= turn) {
                Some(h) => bound = bound.max(h),
                None => return (None, seeds),
            }
        }
        // Leaving the flat `a` along −n_a, it comes out of `b` only where
        // n_b·n_a < 0, after the depth of that point below `a`'s plane.
        (Shape::Flat(oa, na), _) => {
            match least_height_where(part, b, oa, na, |m, turn| m.dot(na) <= turn) {
                Some(h) => bound = bound.max(h),
                None => return (None, seeds),
            }
        }
        (
            Shape::Round {
                origin: o1,
                axis: z1,
                radius: r1,
                major: None,
                sphere: false,
                ..
            },
            Shape::Round {
                origin: o2,
                axis: z2,
                radius: r2,
                major: None,
                sphere: false,
                ..
            },
        ) if z1.cross(z2).length() < 1e-12
            && (o2 - o1 - z1 * (o2 - o1).dot(z1)).length() <= tol =>
        {
            // Coaxial cylinders: every point of one is |r1 − r2| from the
            // other's surface.
            bound = bound.max((r1 - r2).abs());
        }
        _ => {}
    }
    // Where to look: the corner of each face nearest the other's plane,
    // or the nearest corners of their triangles.
    let near_plane = |from: usize, to_o: Vec3, to_n: Vec3| {
        part.every
            .get(&from)
            .and_then(|t| {
                t.iter().flatten().min_by(|x, y| {
                    to_n.dot(**x - to_o)
                        .abs()
                        .total_cmp(&to_n.dot(**y - to_o).abs())
                })
            })
            .copied()
    };
    if let Shape::Flat(ob, nb) = sb
        && let Some(v) = near_plane(a, ob, nb)
    {
        seeds.push((a, v));
        seeds.push((b, v - nb * nb.dot(v - ob)));
    }
    if let Shape::Flat(oa, na) = sa
        && let Some(v) = near_plane(b, oa, na)
    {
        seeds.push((b, v));
        seeds.push((a, v - na * na.dot(v - oa)));
    }
    (Some(bound), seeds)
}

/// The mesh's least distance between faces `a` and `b`, less what the
/// mesh can be off by, with the nearest corners; at most `budget` pairs of
/// triangles (an unfinished search bounds nothing: `None`).
fn mesh_bound(part: &Part<'_>, a: usize, b: usize, budget: &mut u64) -> Option<(f64, Vec3, Vec3)> {
    let (ta, tb) = (part.every.get(&a)?, part.every.get(&b)?);
    let (sa, sb) = (shape(part.body(), a), shape(part.body(), b));
    // Per triangle, its corners' normals and how much they turn: a line
    // leaving `a` along −n_a comes out of `b` only where n_a·n_b < 0, so a
    // pair of triangles where that cannot be is passed over.
    let normals = |tris: &[[Vec3; 3]], s: Shape| -> Vec<Option<([Vec3; 3], f64)>> {
        tris.iter()
            .map(|t| {
                let [x, y, z] = *t;
                let (nx, k) = normal_at(part, s, x)?;
                let (ny, _) = normal_at(part, s, y)?;
                let (nz, _) = normal_at(part, s, z)?;
                Some(([nx, ny, nz], k))
            })
            .collect()
    };
    let (na, nb) = (normals(ta, sa), normals(tb, sb));
    let mut best: Option<(f64, Vec3, Vec3)> = None;
    for (t, ma) in ta.iter().zip(&na) {
        for (u, mb) in tb.iter().zip(&nb) {
            if let (Some((ma, ka)), Some((mb, kb))) = (ma, mb) {
                let least = ma
                    .iter()
                    .flat_map(|x| mb.iter().map(move |y| x.dot(*y)))
                    .fold(f64::INFINITY, f64::min);
                if least > ka + kb {
                    continue;
                }
            }
            *budget = budget.checked_sub(1)?;
            let (d, p, q) = triangles_nearest(t, u);
            if best.is_none_or(|(x, _, _)| d < x) {
                best = Some((d, p, q));
            }
        }
    }
    // No pair of triangles where a line could come out: no bound needed,
    // nothing below the boxes'.
    Some(
        best.map_or((f64::INFINITY, Vec3::ZERO, Vec3::ZERO), |(d, p, q)| {
            ((d - part.err(a) - part.err(b)).max(0.0), p, q)
        }),
    )
}

/// Most pairs of triangles the bound of a thickness compares.
const BOUND_WORK: u64 = 20_000_000;
/// Most places a thickness starts a search from, beyond its samples.
const MAX_SEEDS: usize = 256;

/// A bound, proven, below the thickness at every point of the faces of
/// `part` taking part: below the length of every line into the material
/// along the inward normal from one of them. Per pair of faces it could
/// leave and come out through: their boxes' distance, the distance of one
/// to the other's plane, coaxial cylinders' radii, a round's own diameter,
/// the mesh's distance less what the mesh can be off by. Pairs are taken
/// in order of their cheap bound, until that bound is above the least
/// found. With the places where it was least, to search from.
fn thickness_bound(part: &Part<'_>, tol: f64, below: f64) -> (f64, Vec<(usize, Vec3)>) {
    // Only pairs whose boxes are nearer than `below` (the least found):
    // the others cannot bring the bound under it, and the bound is never
    // given above what was found.
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (&a, ba) in part
        .triangles
        .keys()
        .filter_map(|a| part.every_boxes.get(a).map(|b| (a, b)))
    {
        for (&b, bb) in &part.every_boxes {
            let d = if a == b { 0.0 } else { boxes_distance(ba, bb) };
            if d < below {
                pairs.push((d, a, b));
            }
        }
    }
    pairs.sort_by(|x, y| x.0.total_cmp(&y.0).then((x.1, x.2).cmp(&(y.1, y.2))));
    let mut lower = f64::INFINITY;
    if let Some(&(d, ..)) = pairs.get(MAX_BOUND_PAIRS) {
        // Past the cap, what is left is bounded by its boxes.
        lower = d;
        pairs.truncate(MAX_BOUND_PAIRS);
    }
    let mut seeds: Vec<(usize, Vec3)> = Vec::new();
    let mut budget = BOUND_WORK;
    let flat = |f| matches!(shape(part.body(), f), Shape::Flat(..));
    for (boxes, a, b) in pairs {
        if boxes >= lower {
            break;
        }
        let (Some(cheap), found) = pair_bound(part, a, b, tol) else {
            continue;
        };
        if seeds.len() < MAX_SEEDS {
            seeds.extend(found);
        }
        if cheap >= lower {
            continue;
        }
        // Between flat faces the planes bound it as well as the mesh; the
        // mesh is asked when a face is round.
        let bound = if a != b && !(flat(a) && flat(b)) {
            match mesh_bound(part, a, b, &mut budget) {
                Some((d, p, q)) => {
                    if seeds.len() < MAX_SEEDS && d.is_finite() {
                        seeds.push((a, p));
                        seeds.push((b, q));
                    }
                    cheap.max(d)
                }
                // Out of work: what is left is bounded by its boxes, the
                // least of them this pair's.
                None => {
                    lower = lower.min(boxes);
                    break;
                }
            }
        } else {
            cheap
        };
        lower = lower.min(bound);
    }
    (lower.max(0.0), seeds)
}

/// Most pairs of faces the bound of a thickness looks at.
const MAX_BOUND_PAIRS: usize = 200_000;

/// The thinnest place of the faces of `part`: the least thickness found
/// (at sampled points of every face, at the places the bound points to,
/// each of the least few moved along its face to where it is least), and
/// a bound, proven, below which no thickness lies.
pub(crate) fn thickness(part: &Part<'_>, tol: f64) -> Answer<Option<Thin>> {
    let faces: Vec<usize> = part.triangles.keys().copied().collect();
    if faces.is_empty() {
        return Ok(None);
    }
    let per_face = (MAX_SAMPLES / faces.len()).max(3);
    let mut samples: Vec<(f64, usize, Vec3, Vec3, usize)> = Vec::new();
    let mut count = 0_usize;
    // A line need go no further than twice the least found so far (with
    // room for the faces' own least, to be refined): what is longer
    // cannot be the thinnest place, and a short line meets few faces.
    let mut best = f64::INFINITY;
    let mut try_at = |face: usize, c: Vec3, samples: &mut Vec<_>| -> Answer<()> {
        let Some(p) = onto_face(part, face, c)? else {
            return Ok(());
        };
        count += 1;
        let limit = if best.is_finite() {
            2.0 * best + 10.0 * tol
        } else {
            f64::INFINITY
        };
        if let Some((d, to, other)) = thickness_at(part, face, p, tol, limit)? {
            best = best.min(d);
            samples.push((d, face, p, to, other));
        }
        Ok(())
    };
    for &face in &faces {
        let tris = part.triangles.get(&face).map_or(&[][..], Vec::as_slice);
        let stride = tris.len().div_ceil(per_face).max(1);
        for t in tris.iter().step_by(stride) {
            try_at(face, (t[0] + t[1] + t[2]) * (1.0 / 3.0), &mut samples)?;
        }
    }
    let found = samples.iter().map(|s| s.0).fold(f64::INFINITY, f64::min);
    let (lower, seeds) = thickness_bound(part, tol, found);
    // The places the bound points to, nudged off the face's edges.
    for &(face, p) in seeds.iter().take(MAX_SEEDS) {
        if !part.triangles.contains_key(&face) {
            continue;
        }
        let nudge = part
            .every
            .get(&face)
            .and_then(|tris| {
                tris.iter()
                    .map(|t| (t[0] + t[1] + t[2]) * (1.0 / 3.0))
                    .min_by(|x, y| x.distance(p).total_cmp(&y.distance(p)))
            })
            .unwrap_or(p);
        for k in [1e-6, 1e-3, 1e-1] {
            try_at(face, p + (nudge - p) * k, &mut samples)?;
        }
    }
    samples.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut best: Option<Thin> = None;
    let mut tried: Vec<usize> = Vec::new();
    for &(d, face, p, to, other) in &samples {
        if tried.len() >= MAX_TRIED_FACES {
            break;
        }
        if tried.contains(&face) {
            continue;
        }
        tried.push(face);
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
                lower: 0.0,
                at: p,
                to,
                face,
                opposite: other,
                samples: count,
            });
        }
    }
    Ok(best.map(|mut b| {
        b.lower = lower.min(b.thickness);
        b
    }))
}

/// Most points of a face a plate thickness casts a line from, spread over
/// its triangles.
const PLATE_SAMPLES: usize = 32;
/// Most vertices behind a face, projecting within its box, a plate
/// thickness samples around; a face with more has no thickness (no
/// answer rather than one that may have missed a step).
const PLATE_VERTICES: usize = 4096;

/// The plate thickness of face `face`: the distance through the body
/// from the face to the face directly opposite it, measured along the
/// inward normal (`select.query` `thickness`). `None` unless the face is
/// flat, every line into the material from a point of it first leaves
/// the body through a flat face parallel to it (facing the other way),
/// and every such line is equally long within the tolerance: a face over
/// a step, a pocket or a slanted back has no one thickness.
///
/// The points are the centroids of a grid over the face's triangles and,
/// because a step behind the face need not show in its triangles, the
/// points on either side of where each vertex of the body behind the face
/// projects onto it (where the distance can change: across the edges of
/// the faces behind it, which end in vertices). Each line is exact (the
/// line against the faces' surfaces).
pub(crate) fn plate_thickness(
    part: &Part<'_>,
    face: usize,
    tolerance: &craft3d_math::Tolerance,
) -> Answer<Option<f64>> {
    let body = part.body();
    let Some(f) = body.faces.get(face) else {
        return Ok(None);
    };
    let Surface::Plane(_) = &f.surface else {
        return Ok(None);
    };
    let tol = tolerance.linear;
    let Some(tris) = part.every.get(&face).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    let Some(first) = tris.first() else {
        return Ok(None);
    };
    let centre = (first[0] + first[1] + first[2]) * (1.0 / 3.0);
    let Some(normal) = part.probe.normal(face, centre).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let Some((t1, t2)) = tangents(normal) else {
        return Ok(None);
    };
    let line = PlateLine {
        part,
        face,
        normal,
        tol,
        cos_parallel: tolerance.angular.cos(),
    };
    let mut range: Option<(f64, f64)> = None;
    // A grid of `level`² small triangles in each triangle, at their
    // centroids; its lines first, so that a face over a step that shows
    // in the grid is told before any vertex is looked at.
    let mut level = 1_usize;
    while (level + 1) * (level + 1) * tris.len() <= PLATE_SAMPLES {
        level += 1;
    }
    let step = 1.0 / level as f64;
    let stride = tris.len().div_ceil(PLATE_SAMPLES).max(1);
    for t in tris.iter().step_by(stride) {
        let (u, v) = (t[1] - t[0], t[2] - t[0]);
        for i in 0..level {
            for j in 0..level - i {
                let (a, b) = (i as f64, j as f64);
                let up = t[0] + u * ((a + 1.0 / 3.0) * step) + v * ((b + 1.0 / 3.0) * step);
                let down = (i + j + 1 < level)
                    .then(|| t[0] + u * ((a + 2.0 / 3.0) * step) + v * ((b + 2.0 / 3.0) * step));
                for c in std::iter::once(up).chain(down) {
                    if !line.fits(c, &mut range)? {
                        return Ok(None);
                    }
                }
            }
        }
    }
    // Either side of where the vertices behind the face project onto it:
    // only those that project within the face's box can change what a
    // line from the face meets, and only those no deeper than the deepest
    // line met so far (plus the tolerance): wherever the length of the
    // lines changes away from a length `d` that some line has, an edge no
    // deeper than `d` projects there (the opposite face's own edge, at
    // `d`, or the edge of a face in front of it), and an edge of the body
    // ends at vertices; one ending deeper leaves the face's box or the
    // opposite face's plane at a vertex of that face, at `d`. Vertices
    // within the tolerance of one already taken give the same points
    // within the tolerance and are taken once. More of them than the cap
    // is no answer rather than one that may have missed a step.
    let nudge = 100.0 * tol;
    let Some(face_box) = part.every_boxes.get(&face) else {
        return Ok(None);
    };
    let deepest = range.map_or(f64::INFINITY, |(_, hi)| hi + 2.0 * tol);
    let mut taken: BTreeSet<[i64; 3]> = BTreeSet::new();
    let mut points: Vec<Vec3> = Vec::new();
    let mut sampled: Vec<bool> = vec![false; body.vertices.len()];
    // Where a point behind the face projects onto it, when it is behind
    // (no deeper than `deepest`) and within the face's box.
    let onto = |point: Vec3| -> Option<Vec3> {
        let depth = normal.dot(centre - point);
        if !(depth > tol && depth <= deepest && depth.is_finite()) {
            return None;
        }
        let q = point + normal * depth;
        face_box.contains(q, 2.0 * nudge).then_some(q)
    };
    // Takes `q` once (within the tolerance); `false` past the cap.
    let mut take = |q: Vec3, points: &mut Vec<Vec3>| -> bool {
        // `as` saturates (and makes a NaN 0): a key, never a panic.
        let key = [
            (q.x / tol).round() as i64,
            (q.y / tol).round() as i64,
            (q.z / tol).round() as i64,
        ];
        if !taken.insert(key) {
            return true;
        }
        if taken.len() > PLATE_VERTICES {
            return false;
        }
        for (a, b) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            points.push(q + (t1 * a + t2 * b) * nudge);
        }
        true
    };
    for (index, vertex) in body.vertices.iter().enumerate() {
        let Some(q) = onto(vertex.point) else {
            // A vertex in the face's plane, within its box, ends an edge
            // as well as a sampled one does: the edge's projection starts
            // at the face's own outline.
            if normal.dot(centre - vertex.point).abs() <= tol
                && face_box.contains(vertex.point, 2.0 * nudge)
                && let Some(seen) = sampled.get_mut(index)
            {
                *seen = true;
            }
            continue;
        };
        if let Some(seen) = sampled.get_mut(index) {
            *seen = true;
        }
        if !take(q, &mut points) {
            return Ok(None);
        }
    }
    // The vertices' lines before the edges are looked at.
    for c in points.drain(..) {
        if !line.fits(c, &mut range)? {
            return Ok(None);
        }
    }
    // An edge that crosses the face's box with an end that is not sampled
    // (outside the box, or deeper): the points along it that project
    // within the box stand for its ends, so a step bounded by a long edge
    // is told without a grid point falling on it.
    let reach = grown(face_box, 2.0 * nudge);
    // The box around the face's box swept to `deepest`: an edge whose box
    // misses it is not behind the face (a quick test before the exact
    // one; with no deepest yet, every edge is tried).
    let swept = reach
        .corners()
        .filter(|_| deepest.is_finite())
        .map(|(lo, hi)| {
            let d = normal * -deepest;
            Bbox3::from_points([lo, hi, lo + d, hi + d])
        });
    for (edge, edge_box) in body.edges.iter().zip(part.edge_boxes()) {
        if swept.as_ref().is_some_and(|w| !overlaps(w, edge_box)) {
            continue;
        }
        if [edge.start, edge.end]
            .iter()
            .all(|v| sampled.get(v.0 as usize).copied().unwrap_or(false))
        {
            continue;
        }
        let Some((lo, hi)) = edge_box.corners() else {
            continue;
        };
        let corners = (0..8).map(|k| {
            Vec3::new(
                if k & 1 == 0 { lo.x } else { hi.x },
                if k & 2 == 0 { lo.y } else { hi.y },
                if k & 4 == 0 { lo.z } else { hi.z },
            )
        });
        let (mut shallow, mut deep) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut shadow = Bbox3::EMPTY;
        for c in corners {
            let depth = normal.dot(centre - c);
            shallow = shallow.min(depth);
            deep = deep.max(depth);
            shadow.add(c + normal * depth);
        }
        if !(deep > tol && shallow <= deepest) || !overlaps(&shadow, &reach) {
            continue;
        }
        for point in edge_points(edge, EDGE_SAMPLES) {
            if let Some(q) = onto(point)
                && !take(q, &mut points)
            {
                return Ok(None);
            }
        }
    }
    for c in points {
        if !line.fits(c, &mut range)? {
            return Ok(None);
        }
    }
    Ok(range.map(|(lo, _)| lo))
}

/// The lines of a plate thickness: from points of one flat face along
/// its inward normal ([`plate_thickness`]).
struct PlateLine<'p, 'a> {
    part: &'p Part<'a>,
    face: usize,
    normal: Vec3,
    tol: f64,
    cos_parallel: f64,
}

impl PlateLine<'_, '_> {
    /// Whether the line from the point of the face nearest `c` leaves the
    /// body through a flat face opposite and parallel, at a distance that
    /// keeps `range` (the least and the most distance so far) within the
    /// tolerance; `range` takes it in. A point off the face fits.
    ///
    /// Once a distance is known, the line is followed no further than the
    /// least one and four tolerances: a line that leaves later makes the
    /// range wider than the tolerance whatever it meets there, and every
    /// face met within the tolerance of where it leaves lies before that.
    fn fits(&self, c: Vec3, range: &mut Option<(f64, f64)>) -> Answer<bool> {
        let (part, face, normal, tol) = (self.part, self.face, self.normal, self.tol);
        // Only points of the face itself.
        let Some(p) = onto_face(part, face, c)? else {
            return Ok(true);
        };
        let from = 4.0 * tol;
        let to = range.map_or(f64::INFINITY, |(lo, _)| lo + 4.0 * tol);
        let hits = part
            .probe
            .line_hits(p, -normal, from, to)
            .map_err(|e| e.to_string())?;
        let crossing: Vec<&LineHit> = hits.iter().filter(|h| h.t >= from && !h.along).collect();
        let Some(nearest) = crossing.first().map(|h| h.t) else {
            return Ok(false);
        };
        // Where the line leaves: one face, or the faces meeting at an
        // edge it crosses; one of them must face the other way, flat and
        // parallel.
        let mut opposite = false;
        for h in crossing.iter().take_while(|h| h.t <= nearest + tol) {
            let Some(g) = part.body().faces.get(h.face) else {
                continue;
            };
            if h.face == face || !matches!(g.surface, Surface::Plane(_)) {
                continue;
            }
            let Some(other) = part
                .probe
                .normal(h.face, h.point)
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            if -other.dot(normal) >= self.cos_parallel {
                opposite = true;
                break;
            }
        }
        if !opposite {
            return Ok(false);
        }
        let (lo, hi) = range.unwrap_or((nearest, nearest));
        let (lo, hi) = (lo.min(nearest), hi.max(nearest));
        if hi - lo > tol {
            return Ok(false);
        }
        *range = Some((lo, hi));
        Ok(true)
    }
}

/// Most faces whose least sample is moved along the face.
const MAX_TRIED_FACES: usize = 8;

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
    let stop = (tol * 0.1).max(1e-12 * (1.0 + cur.1.length()));
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
            if let Some((d, to, other)) = thickness_at(part, face, p, tol, cur.0)?
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
/// conical face, across the whole of its narrowest circle.
#[derive(Clone, Debug)]
pub(crate) struct Through {
    pub through: bool,
    /// The axis: a point on it in the middle of the face and its
    /// direction.
    pub middle: Vec3,
    pub axis: Vec3,
    /// The radius of the face's narrowest circle: the bore that must be
    /// clear.
    pub radius: f64,
    /// Why not, when it is not.
    pub why: Option<String>,
    /// Where material is in the bore: a point of it (the middle of the
    /// common volume, or a place of contact) and the faces of the body
    /// that bound it.
    pub blocked: Vec<(Vec3, Vec<usize>)>,
}

/// The axis of a cylinder or a cone.
pub(crate) fn axis_of(surface: &Surface) -> Option<(Vec3, Vec3)> {
    match surface {
        Surface::Cylinder(s) => Some((s.frame().origin(), s.frame().z())),
        Surface::Cone(s) => Some((s.frame().origin(), s.frame().z())),
        _ => None,
    }
}

/// Whether the body of `part` leaves the bore of face `face` clear: a
/// cylinder on the face's axis, of the radius of its narrowest circle less
/// a margin, as long as the body is along the axis and beyond, has no
/// point in common with the body (`craft3d_kernel::boolean::interference`:
/// exact, the boolean's intersection). One ray down the axis would let a
/// blind hole with a vent or a bore half blocked by a rib pass.
pub(crate) fn through(
    part: &Part<'_>,
    face: usize,
    tolerance: &craft3d_math::Tolerance,
) -> Answer<Through> {
    let tol = tolerance.linear;
    let body = part.body();
    let f = body.faces.get(face).ok_or("a face is missing")?;
    let (origin, axis) = axis_of(&f.surface).ok_or_else(|| {
        format!(
            "the face is a {}; through is asked of a cylindrical or conical face",
            f.surface.kind()
        )
    })?;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut narrowest = f64::INFINITY;
    for edge in face_edges(body, face) {
        for p in edge_points(edge, EDGE_SAMPLES) {
            let s = (p - origin).dot(axis);
            let off = (p - origin - axis * s).length();
            if s.is_finite() && off.is_finite() {
                lo = lo.min(s);
                hi = hi.max(s);
                narrowest = narrowest.min(off);
            }
        }
    }
    if let Surface::Cylinder(c) = &f.surface {
        narrowest = c.radius();
    }
    if !(lo.is_finite() && hi.is_finite() && narrowest.is_finite()) {
        return Err("the face has no bounded extent along its axis".into());
    }
    let middle = origin + axis * (0.5 * (lo + hi));
    let mut result = Through {
        through: false,
        middle,
        axis,
        radius: narrowest,
        why: None,
        blocked: Vec::new(),
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
    let margin = (10.0 * tol).max(1e-6 * narrowest);
    let bore = narrowest - margin;
    if bore.is_nan() || bore <= 0.0 {
        return Err(format!(
            "the face's narrowest circle (radius {narrowest:e}) is too small to tell"
        ));
    }
    // From before the body to beyond it, along the axis.
    let (blo, bhi) = body
        .bbox()
        .ok()
        .and_then(|b| b.corners())
        .ok_or("the body has no box")?;
    let (mut from, mut to) = (f64::INFINITY, f64::NEG_INFINITY);
    for x in [blo.x, bhi.x] {
        for y in [blo.y, bhi.y] {
            for z in [blo.z, bhi.z] {
                let s = (Vec3::new(x, y, z) - middle).dot(axis);
                from = from.min(s);
                to = to.max(s);
            }
        }
    }
    let pad = 0.01 * (to - from) + 10.0 * tol + narrowest;
    let (from, to) = (from - pad, to + pad);
    let frame = craft3d_math::Frame::from_z(middle + axis * from, axis)
        .ok_or("the face's axis has no direction")?;
    let tool =
        craft3d_kernel::primitives::cylinder(&frame, bore, to - from).map_err(|e| e.to_string())?;
    use craft3d_kernel::boolean::{Interference, Operand, interference};
    match interference(body, &tool, tolerance)
        .map_err(|e| format!("whether the bore is clear cannot be told: {e}"))?
    {
        Interference::Clear => result.through = true,
        Interference::Touch { at, faces_a, .. } => {
            let faces = faces_a.iter().map(|f| f.0 as usize).collect::<Vec<_>>();
            for p in at.into_iter().take(4) {
                result.blocked.push((p, faces.clone()));
            }
            result.why = Some("the body reaches into the bore".into());
        }
        Interference::Overlap(common) => {
            let faces: Vec<usize> = common
                .merged
                .iter()
                .flatten()
                .filter(|s| s.operand == Operand::A)
                .map(|s| s.face.0 as usize)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            let at = craft3d_topo::mass_properties(&common.body)
                .ok()
                .and_then(|m| m.centroid)
                .or_else(|| common.body.bbox().ok().and_then(|b| b.center()))
                .unwrap_or(middle);
            result.blocked.push((at, faces));
            result.why = Some("material lies in the bore".into());
        }
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
mod proof_tests;

#[cfg(test)]
mod settle_tests;

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
