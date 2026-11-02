//! What every imported body goes through once its faces are read, from
//! a STEP file or a FreeCAD document alike: the file's repeated edges
//! joined, free edges sewn within the document's cap, shells turned the
//! right way out, every edge proven on its faces and every vertex on its
//! edges (a tolerance of its own where it is off by more than the
//! tolerance, up to the cap), and the whole body checked. Geometry is
//! never moved (docs/decisions.md, 2026-10-18).

use crate::{ImportError, Imported, Options, Report, err};
use craft3d_topo::{Body, BodyKind, Coedge, Defect, EdgeId, FaceId, VertexId};
use std::collections::BTreeMap;

/// A body as its faces were read, before [`finish`]: per shell whether it
/// is a cavity, and per face, edge and vertex (by index) the number the
/// file gives it, for messages (`#12`).
#[derive(Clone, Debug)]
pub struct Assembled {
    pub body: Body,
    pub cavities: Vec<bool>,
    pub face_ids: Vec<u64>,
    pub edge_ids: Vec<u64>,
    pub vertex_ids: Vec<u64>,
}

/// A loop run the other way: the coedges in reverse order, each used the
/// other way.
pub fn reverse(coedges: &mut [Coedge]) {
    coedges.reverse();
    for c in coedges {
        c.reversed = !c.reversed;
    }
}

struct Finisher<'r> {
    body: Body,
    /// The distance within which two vertices of the file are one.
    merge: f64,
    face_ids: Vec<u64>,
    edge_ids: Vec<u64>,
    vertex_ids: Vec<u64>,
    report: &'r mut Report,
}

impl Finisher<'_> {
    /// Joins edges used once each that run between the same two vertices
    /// with their midpoints within the merge distance.
    fn sew_edges(&mut self) {
        let mut uses = vec![0usize; self.body.edges.len()];
        for l in &self.body.loops {
            for c in &l.coedges {
                if let Some(n) = uses.get_mut(c.edge.0 as usize) {
                    *n += 1;
                }
            }
        }
        // Open edges by the pair of vertices they join.
        let mut open: BTreeMap<(u32, u32), Vec<EdgeId>> = BTreeMap::new();
        for (i, edge) in self.body.edges.iter().enumerate() {
            if uses.get(i) == Some(&1) {
                let pair = (edge.start.0.min(edge.end.0), edge.start.0.max(edge.end.0));
                open.entry(pair).or_default().push(EdgeId(i as u32));
            }
        }
        let mid = |body: &Body, e: EdgeId| {
            body.edge(e)
                .map(|edge| edge.curve.eval(0.5 * (edge.range.0 + edge.range.1)))
        };
        // Edge → (edge it becomes, whether it then runs the other way).
        let mut into: BTreeMap<u32, (EdgeId, bool)> = BTreeMap::new();
        for group in open.values() {
            let mut taken = vec![false; group.len()];
            for (i, &first) in group.iter().enumerate() {
                if taken.get(i).copied().unwrap_or(true) {
                    continue;
                }
                for (j, &second) in group.iter().enumerate().skip(i + 1) {
                    if taken.get(j).copied().unwrap_or(true) {
                        continue;
                    }
                    let (Some(a), Some(b)) = (mid(&self.body, first), mid(&self.body, second))
                    else {
                        continue;
                    };
                    let gap = a.distance(b);
                    if gap <= self.merge {
                        let same_way = self
                            .body
                            .edge(first)
                            .zip(self.body.edge(second))
                            .is_some_and(|(x, y)| {
                                if x.start != x.end {
                                    return x.start == y.start;
                                }
                                // A closed edge: by the way the curves run
                                // at the midpoint.
                                let along = |e: &craft3d_topo::Edge| {
                                    e.curve.derivatives(0.5 * (e.range.0 + e.range.1)).1
                                };
                                along(x).dot(along(y)) > 0.0
                            });
                        into.insert(second.0, (first, !same_way));
                        if let Some(t) = taken.get_mut(j) {
                            *t = true;
                        }
                        if let Some(t) = taken.get_mut(i) {
                            *t = true;
                        }
                        self.report.merged_edges += 1;
                        self.report.edge_gap = self.report.edge_gap.max(gap);
                        break;
                    }
                }
            }
        }
        if into.is_empty() {
            return;
        }
        for l in &mut self.body.loops {
            for c in &mut l.coedges {
                if let Some(&(edge, flip)) = into.get(&c.edge.0) {
                    c.edge = edge;
                    c.reversed ^= flip;
                }
            }
        }
        // Drop the edges that went into others, keeping the order.
        let mut renumber = vec![u32::MAX; self.body.edges.len()];
        let mut kept = Vec::with_capacity(self.body.edges.len());
        let mut kept_ids = Vec::with_capacity(self.body.edges.len());
        for (i, edge) in std::mem::take(&mut self.body.edges).into_iter().enumerate() {
            if into.contains_key(&(i as u32)) {
                continue;
            }
            if let Some(slot) = renumber.get_mut(i) {
                *slot = kept.len() as u32;
            }
            kept.push(edge);
            kept_ids.push(self.edge_ids.get(i).copied().unwrap_or(0));
        }
        self.body.edges = kept;
        self.edge_ids = kept_ids;
        for l in &mut self.body.loops {
            for c in &mut l.coedges {
                if let Some(&n) = renumber.get(c.edge.0 as usize) {
                    c.edge = EdgeId(n);
                }
            }
        }
    }

    /// Sews the edges still used once to free edges that run within the
    /// cap of them (`topo::sew`: faces the file does not join, or joins
    /// only within a gap): the file's own sharing first (`sew_edges`),
    /// then this, at the document's cap on an entity's tolerance. Nothing
    /// is moved; what it joins is measured and recorded. A solid left
    /// with an edge used once is refused naming it and how far the
    /// nearest free edge is.
    fn sew_loose(
        &mut self,
        name: &str,
        tolerance: &craft3d_math::Tolerance,
        cap: f64,
    ) -> Result<(), ImportError> {
        let step_id = |ids: &[u64], k: u32| ids.get(k as usize).copied().unwrap_or(0);
        let sewn = match craft3d_topo::sew(&self.body, tolerance, cap, cap) {
            Ok(sewn) => sewn,
            Err(craft3d_topo::SewError::Gap {
                edge,
                vertex,
                measured,
                cap,
            }) => {
                let what = match (edge, vertex) {
                    (Some(e), _) => format!("a sewn edge (from edge {})", e.0),
                    (None, Some(v)) => format!("a sewn vertex (from vertex {})", v.0),
                    (None, None) => "an entity".to_string(),
                };
                let how = measured.map_or_else(
                    || "could not be proven within".to_string(),
                    |m| format!("lies {m:e} off what it joins, more than"),
                );
                return err(format!(
                    "`{name}`: {what} {how} the cap {cap:e} on an entity's own tolerance (the document's `loosest`); the body is refused rather than moved"
                ));
            }
            Err(e) => return err(format!("`{name}`: the faces could not be sewn: {e}")),
        };
        if sewn.joined > 0 {
            // The file's ids of the entities kept: the first that became
            // each (the lowest, kept with its curve or point).
            let mut edge_ids = vec![0u64; sewn.body.edges.len()];
            let mut edge_seen = vec![false; sewn.body.edges.len()];
            for (old, (new, _)) in sewn.edges.iter().enumerate() {
                let k = new.0 as usize;
                if let Some(slot) = edge_seen.get_mut(k)
                    && !*slot
                {
                    *slot = true;
                    if let Some(id) = edge_ids.get_mut(k) {
                        *id = step_id(&self.edge_ids, u32::try_from(old).unwrap_or(u32::MAX));
                    }
                }
            }
            let mut vertex_ids = vec![0u64; sewn.body.vertices.len()];
            let mut seen = vec![false; sewn.body.vertices.len()];
            for (old, new) in sewn.vertices.iter().enumerate() {
                let k = new.0 as usize;
                if let Some(slot) = seen.get_mut(k)
                    && !*slot
                {
                    *slot = true;
                    if let Some(id) = vertex_ids.get_mut(k) {
                        *id = step_id(&self.vertex_ids, u32::try_from(old).unwrap_or(u32::MAX));
                    }
                }
            }
            // The pieces of an edge split at a T-junction carry the
            // file's id of the edge they are part of, where no edge of the
            // file became them already.
            for (old, pieces) in &sewn.split {
                for (new, _) in pieces {
                    let k = new.0 as usize;
                    if let Some(slot) = edge_seen.get_mut(k)
                        && !*slot
                    {
                        *slot = true;
                        if let Some(id) = edge_ids.get_mut(k) {
                            *id = step_id(&self.edge_ids, old.0);
                        }
                    }
                }
            }
            self.edge_ids = edge_ids;
            self.vertex_ids = vertex_ids;
            self.report.split_edges += sewn.split.len();
            self.report.sewn_edges += sewn.joined;
            self.report.sewn_vertices += sewn.merged_vertices;
            self.report.sewn_gap = self.report.sewn_gap.max(sewn.largest_gap);
        }
        let open = sewn.worst_open();
        self.body = sewn.body;
        if self.body.kind == BodyKind::Solid
            && let Some((edge, nearest)) = open
        {
            let id = step_id(&self.edge_ids, edge.0);
            let nearest = nearest.map_or_else(
                || "no other free edge".to_string(),
                |d| format!("the nearest free edge is {d:e} away"),
            );
            return err(format!(
                "`{name}`: edge #{id} bounds only one face and no free edge runs within the cap {cap:e} of it ({nearest}; the document's `loosest`): the solid is not closed and is refused rather than moved"
            ));
        }
        Ok(())
    }

    /// Turns a shell over: every face's normal and every loop.
    fn turn_shell(&mut self, shell: usize) {
        let faces = self
            .body
            .shells
            .get(shell)
            .map(|s| s.faces.clone())
            .unwrap_or_default();
        for face in faces {
            let loops = match self.body.faces.get_mut(face.0 as usize) {
                Some(f) => {
                    f.reversed = !f.reversed;
                    f.loops.clone()
                }
                None => continue,
            };
            for l in loops {
                if let Some(l) = self.body.loops.get_mut(l.0 as usize) {
                    reverse(&mut l.coedges);
                }
            }
        }
    }
}

/// Points along an edge, per face that uses it, all within `tolerance` of
/// the face's surface: 1025 places along the edge (the ends included).
fn densely_within(body: &Body, edge: EdgeId, tolerance: f64) -> bool {
    const PLACES: u32 = 1024;
    let Some(e) = body.edge(edge) else {
        return false;
    };
    let (t0, t1) = e.range;
    let mut any = false;
    for face in &body.faces {
        let uses = face.loops.iter().any(|l| {
            body.loop_(*l)
                .is_some_and(|l| l.coedges.iter().any(|c| c.edge == edge))
        });
        if !uses {
            continue;
        }
        any = true;
        for k in 0..=PLACES {
            let p = e
                .curve
                .eval(t0 + (t1 - t0) * f64::from(k) / f64::from(PLACES));
            let near = face
                .surface
                .project(p)
                .ok()
                .map(|(u, v)| face.surface.eval(u, v).distance(p));
            if !near.is_some_and(|d| d <= tolerance) {
                return false;
            }
        }
    }
    any
}

/// The worst distance among the defects that say an entity lies off
/// another (a gap a tolerance per entity would hold), with the defect.
fn worst_gap(defects: &[Defect]) -> Option<(f64, &Defect)> {
    let mut worst: Option<(f64, &Defect)> = None;
    for defect in defects {
        let distance = match defect {
            Defect::VertexOffEdge { distance, .. } | Defect::EdgeOffSurface { distance, .. } => {
                *distance
            }
            _ => continue,
        };
        let distance = if distance.is_finite() {
            distance
        } else {
            f64::INFINITY
        };
        if worst.is_none_or(|(d, _)| distance > d) {
            worst = Some((distance, defect));
        }
    }
    worst
}

/// The refusal of a body whose reading spent its work
/// (`crate::IMPORT_WORK`), naming the stage and where it was; `None`
/// while there is work left.
fn over_budget(name: &str, stage: &str, at: Option<String>) -> Option<ImportError> {
    craft3d_geom::allowance::exhausted().then(|| {
        ImportError(format!(
            "`{name}`: reading the body took more work than an import may (`stepread::IMPORT_WORK`) while {stage}{}; the body is refused",
            at.map_or_else(String::new, |a| format!(" (at {a})"))
        ))
    })
}

/// An edge whose tolerance is at least this share of the body's largest
/// is measured again near its gap ([`tighten_loosest`]).
const NEAR_LARGEST: f64 = 0.5;

/// The edges whose tolerance sets or comes near the body's largest (at
/// least [`NEAR_LARGEST`] of it), which the kernel then works at, measured
/// again near their gap (`topo::tighten_edge`), loosest first, until the
/// next is below that share of the largest left; the rest keep the bound
/// of the first proof (the developer, 2026-11-02: tighten where it
/// matters, at the cost of the import's time elsewhere). Edges sewing
/// gave a tolerance (`sewn`) are left. The vertices at an edge tightened
/// lose what it raised them to; they are measured after. The report's
/// count and largest follow.
fn tighten_loosest(
    body: &mut craft3d_topo::Body,
    sewn: &std::collections::BTreeSet<EdgeId>,
    tolerance: &craft3d_math::Tolerance,
    cap: f64,
    report: &mut Report,
) {
    let mut loose: Vec<(EdgeId, f64)> = body
        .tolerances
        .edges()
        .filter(|(e, _)| !sewn.contains(e))
        .collect();
    loose.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut largest_done = 0.0_f64;
    for &(edge, had) in &loose {
        // The largest there will be: what was tightened, this one as it
        // is, and the edges sewing gave.
        let left = sewn
            .iter()
            .filter_map(|e| body.edge_tolerance(*e))
            .fold(largest_done.max(had), f64::max);
        if had < NEAR_LARGEST * left {
            break;
        }
        let ends = body.edge(edge).map(|e| [e.start, e.end]);
        let now = craft3d_topo::tighten_edge(body, edge, tolerance, cap).unwrap_or(Some(had));
        largest_done = largest_done.max(now.unwrap_or(0.0));
        if now.is_none_or(|t| t < had) {
            for v in ends.into_iter().flatten() {
                body.clear_vertex_tolerance(v);
            }
        }
        if now.is_none() {
            report.loose_edges = report.loose_edges.saturating_sub(1);
        }
    }
    report.largest_tolerance = body.tolerances.edges().map(|(_, t)| t).fold(0.0, f64::max);
}

/// Finishes a body read from a file: `name` names it in messages,
/// `merge` is the distance within which the file's edges are one (its
/// uncertainty, within the tolerance and the cap).
pub fn finish(
    assembled: Assembled,
    name: String,
    merge: f64,
    options: &Options,
    report: &mut Report,
) -> Result<Imported, ImportError> {
    finish_described(assembled, name, merge, options, report, &|_| None)
}

/// An entity of the file, by its id, for a refusal's message.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Entity {
    Edge(u64),
    Face(u64),
}

/// [`finish`], with `describe` naming the file's entity behind an edge's
/// curve or a face's surface in a refusal's message (a STEP file's
/// keywords; `None` for the kind the body holds).
pub(crate) fn finish_described(
    assembled: Assembled,
    name: String,
    merge: f64,
    options: &Options,
    report: &mut Report,
    describe: &dyn Fn(Entity) -> Option<String>,
) -> Result<Imported, ImportError> {
    let tolerance = options.tolerance.linear;
    let cap = options.loosest;
    let Assembled {
        body,
        cavities,
        face_ids,
        edge_ids,
        vertex_ids,
    } = assembled;
    let kind = body.kind;
    let mut finisher = Finisher {
        body,
        merge,
        face_ids,
        edge_ids,
        vertex_ids,
        report: &mut *report,
    };
    finisher.sew_edges();
    if let Err(e) = finisher.sew_loose(&name, &options.tolerance, cap) {
        return Err(over_budget(&name, "sewing its faces", None).unwrap_or(e));
    }
    // A solid's outside encloses a positive volume and a cavity a
    // negative one; a shell the other way round is turned over (a cavity
    // the file gives as a plain closed shell).
    if kind == BodyKind::Solid {
        let volumes = craft3d_topo::shell_volumes(&finisher.body);
        for (shell, (measured, cavity)) in volumes.iter().zip(&cavities).enumerate() {
            if let Ok(measured) = measured
                && measured.volume.abs() > measured.bound
                && (measured.volume < 0.0) != *cavity
            {
                finisher.turn_shell(shell);
                finisher.report.turned_shells += 1;
            }
        }
    }
    // Every edge proven on the surfaces of its faces over its whole
    // length (not sampled), and every vertex measured against the ends of
    // its edges: what lies off by more than the tolerance gets what was
    // proven as its own tolerance, up to the cap; beyond it, or when no
    // proof within the cap is found, the body is refused naming the
    // entity. Geometry is never moved.
    let step_id = |ids: &[u64], k: u32| ids.get(k as usize).copied().unwrap_or(0);
    // Edges sewing gave a tolerance (it holds the edge it dropped too):
    // never measured tighter on their own curve alone.
    let sewn: std::collections::BTreeSet<EdgeId> =
        finisher.body.tolerances.edges().map(|(e, _)| e).collect();
    let count = finisher.body.edges.len();
    for k in 0..count {
        let edge = EdgeId(u32::try_from(k).unwrap_or(u32::MAX));
        let recorded = craft3d_topo::record_edge(&mut finisher.body, edge, &options.tolerance, cap);
        if let Some(e) = over_budget(
            &name,
            "proving its edges on their faces",
            Some(format!(
                "edge #{}, {k} of {count} edges proven",
                step_id(&finisher.edge_ids, edge.0)
            )),
        ) {
            return Err(e);
        }
        match recorded {
            Ok(Some(t)) => {
                finisher.report.loose_edges += 1;
                finisher.report.largest_tolerance = finisher.report.largest_tolerance.max(t);
            }
            Ok(None) => {}
            // No proof within the cap was found. Where the prover gives
            // out on an edge that is exact (an arc through a sphere's
            // pole), a dense sampling within the tolerance accepts it,
            // counted; anything sampled off by more is refused.
            Err(craft3d_topo::ToleranceError::EdgeNotMeasured { .. })
                if densely_within(&finisher.body, edge, tolerance) =>
            {
                finisher.report.unproven_edges += 1;
            }
            Err(e) => {
                let what = match e {
                    craft3d_topo::ToleranceError::AboveCap { measured, .. } => {
                        format!(
                            "lies {:e} off the surfaces of its faces",
                            measured / craft3d_topo::RECORD_MARGIN
                        )
                    }
                    craft3d_topo::ToleranceError::EdgeNotMeasured { .. } => {
                        "could not be proven within the cap of its faces' surfaces".to_string()
                    }
                    other => other.to_string(),
                };
                let step = step_id(&finisher.edge_ids, edge.0);
                let why = crate::diagnose::edge(
                    &finisher.body,
                    edge,
                    step,
                    &finisher.face_ids,
                    &options.tolerance,
                    cap,
                    finisher.report.uncertainty,
                    describe,
                );
                return err(format!(
                    "`{name}`: edge #{step} {what}, more than the cap {cap:e} on an entity's own tolerance (the document's `loosest`); the body is refused rather than moved ({why})"
                ));
            }
        }
    }
    tighten_loosest(
        &mut finisher.body,
        &sewn,
        &options.tolerance,
        cap,
        finisher.report,
    );
    if let Some(e) = over_budget(&name, "measuring its loosest edges again", None) {
        return Err(e);
    }
    // Every vertex, with one pass over the edges.
    let every: Vec<VertexId> = (0..finisher.body.vertices.len())
        .map(|k| VertexId(u32::try_from(k).unwrap_or(u32::MAX)))
        .collect();
    match craft3d_topo::record_vertices(&mut finisher.body, &every, &options.tolerance, cap) {
        Ok(recorded) => {
            for t in recorded.into_iter().filter_map(|(_, t)| t) {
                finisher.report.loose_vertices += 1;
                finisher.report.largest_tolerance = finisher.report.largest_tolerance.max(t);
            }
        }
        Err((vertex, e)) => {
            let what = match e {
                craft3d_topo::ToleranceError::AboveCap { measured, .. } => {
                    format!(
                        "lies {:e} from the ends of its edges",
                        measured / craft3d_topo::RECORD_MARGIN
                    )
                }
                other => other.to_string(),
            };
            return err(format!(
                "`{name}`: vertex #{} {what}, more than the cap {cap:e} on an entity's own tolerance (the document's `loosest`); the body is refused rather than moved",
                step_id(&finisher.vertex_ids, vertex.0)
            ));
        }
    }
    let defects = craft3d_topo::check_capped(&finisher.body, &options.tolerance, cap);
    let face_of = |f: FaceId| {
        finisher
            .face_ids
            .get(f.0 as usize)
            .map_or_else(|| format!("face {}", f.0), |id| format!("face #{id}"))
    };
    let edge_of = |e: EdgeId| {
        finisher
            .edge_ids
            .get(e.0 as usize)
            .map_or_else(|| format!("edge {}", e.0), |id| format!("edge #{id}"))
    };
    let vertex_of = |v: VertexId| {
        finisher
            .vertex_ids
            .get(v.0 as usize)
            .map_or_else(|| format!("vertex {}", v.0), |id| format!("vertex #{id}"))
    };
    if let Some((distance, defect)) = worst_gap(&defects) {
        let what = match defect {
            Defect::VertexOffEdge { edge, vertex, .. } => format!(
                "{} lies {distance:e} from the end of {}",
                vertex_of(*vertex),
                edge_of(*edge)
            ),
            Defect::EdgeOffSurface { face, edge, .. } => format!(
                "{} lies {distance:e} off the surface of {}",
                edge_of(*edge),
                face_of(*face)
            ),
            other => other.to_string(),
        };
        let gaps = defects
            .iter()
            .filter(|d| {
                matches!(
                    d,
                    Defect::VertexOffEdge { .. } | Defect::EdgeOffSurface { .. }
                )
            })
            .count();
        return err(format!(
            "`{name}`: {what}, more than its tolerance ({gaps} such gaps, this the largest); the body is refused rather than moved"
        ));
    }
    if let Some(first) = defects.first() {
        let shown: Vec<String> = defects.iter().take(4).map(ToString::to_string).collect();
        return err(format!(
            "`{name}` is not a valid body ({} defects): {}; faces, edges and vertices are numbered in the order the file's shells reach them (first: {first})",
            defects.len(),
            shown.join("; ")
        ));
    }
    let Finisher {
        body,
        face_ids,
        edge_ids,
        vertex_ids,
        ..
    } = finisher;
    Ok(Imported {
        body,
        name,
        path: String::new(),
        color: None,
        report: report.clone(),
        face_ids,
        edge_ids,
        vertex_ids,
    })
}
