//! An operation on one body with loose entities (fillet, chamfer, the face
//! operations, shell, thicken): the decision of 2026-10-18, option (4),
//! as the boolean has it since slice 1 (`boolean/loose.rs`), for the
//! operations that change one body (slice 2, 2026-10-22;
//! `docs/design/booleans.md`, *Tolerant modelling, slice 2*).
//!
//! - **The body given** is checked first at the document's tolerance with
//!   its own table: a gap no entity records is refused, never hidden by
//!   the working tolerance.
//! - **The working tolerance** is the body's largest (its own table and
//!   the document's); the operation runs unchanged at that number.
//! - **What the result's entities carry.** An edge whose source (the
//!   operation's own map of where each entity came from) was loose and
//!   whose curve is still that edge's curve keeps that tolerance; a vertex
//!   whose source was loose and that is still at its point keeps its. An
//!   entity with another curve or point, or none from the body, is *new*.
//!   The result is checked at the document's tolerance with what it
//!   carries, and each entity that is not within its own and that the
//!   operation could have moved (a new or loose one, an exact edge ending
//!   at one or bounding a face the operation made or changed, an exact
//!   vertex at the end of such an edge) is measured and given what it
//!   needs, never more than the working tolerance. What still fails is
//!   refused.
//! - **No growth beyond the input.** No entity of the result is looser
//!   than the body's working tolerance, which its document's cap already
//!   held (regeneration checks every body against it), so the cap is not
//!   needed here: an operation never raises the largest tolerance of a
//!   body.
//!
//! A body without loose entities never comes here: such an operation is
//! what it was before tolerant entities existed.

use crate::{KernelError, MAX_REPORTED};
use craft3d_math::Tolerance;
use craft3d_topo::{
    Body, Defect, EdgeId, FaceId, RECORD_MARGIN, VertexId, default_loosest, edge_faces,
    measure_edge, measure_vertex,
};
use std::collections::BTreeSet;

/// Checks of the result before giving up (as the boolean's).
const ROUNDS: usize = 3;

/// Where the entities of an operation's result came from, in the body
/// given: per face its face when it lies on that face's surface still
/// (`None` for a face made, merged or moved), per edge and vertex its
/// source (`None` for a new one).
pub(crate) struct Sources {
    pub faces: Vec<Option<FaceId>>,
    pub edges: Vec<Option<EdgeId>>,
    pub vertices: Vec<Option<VertexId>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    New,
    Loose,
    Exact,
}

/// Runs `op` on `body` as the module documentation says: unchanged for a
/// body without loose entities.
pub(crate) fn run<R>(
    operation: &'static str,
    body: &Body,
    tolerance: &Tolerance,
    op: impl FnOnce(&Tolerance) -> Result<R, KernelError>,
    parts: impl FnOnce(&mut R) -> (&mut Body, Sources),
) -> Result<R, KernelError> {
    if !body.is_tolerant() {
        return op(tolerance);
    }
    let linear = tolerance.linear;
    let working = body.working_tolerance(linear);
    let cap = default_loosest(linear).max(working);
    let mut defects = craft3d_topo::check_remembered_capped(body, tolerance, cap);
    if !defects.is_empty() {
        defects.truncate(MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "the body given to an operation",
            defects,
        });
    }
    let at = Tolerance {
        linear: working,
        ..*tolerance
    };
    let mut result = op(&at)?;
    let (out, sources) = parts(&mut result);
    carry(operation, body, out, &sources, tolerance, cap, working)?;
    Ok(result)
}

fn carry(
    operation: &'static str,
    body: &Body,
    out: &mut Body,
    sources: &Sources,
    tolerance: &Tolerance,
    cap: f64,
    working: f64,
) -> Result<(), KernelError> {
    let lost = || KernelError::Unsupported {
        operation,
        reason: "the operation's map of where its entities came from does not fit its body; this is a bug",
        faces: Vec::new(),
    };
    // Start from nothing: what the operation's own steps recorded was
    // recorded at the working tolerance.
    out.tolerances = craft3d_topo::Tolerances::default();
    let mut edge_origin = Vec::with_capacity(out.edges.len());
    for (i, edge) in out.edges.iter().enumerate() {
        let source = sources.edges.get(i).copied().flatten();
        let origin = match source.and_then(|e| body.edge(e).map(|s| (e, s))) {
            Some((e, s)) if s.curve == edge.curve => {
                if body.edge_tolerance(e).is_some() {
                    Origin::Loose
                } else {
                    Origin::Exact
                }
            }
            _ => Origin::New,
        };
        edge_origin.push(origin);
    }
    let mut vertex_origin = Vec::with_capacity(out.vertices.len());
    for (i, vertex) in out.vertices.iter().enumerate() {
        let source = sources.vertices.get(i).copied().flatten();
        let origin = match source.and_then(|v| body.vertex(v).map(|s| (v, s))) {
            Some((v, s)) if s.point == vertex.point => {
                if body.vertex_tolerance(v).is_some() {
                    Origin::Loose
                } else {
                    Origin::Exact
                }
            }
            _ => Origin::New,
        };
        vertex_origin.push(origin);
    }
    // Loose entities kept keep their tolerance.
    for (i, origin) in edge_origin.iter().enumerate() {
        if *origin == Origin::Loose
            && let Some(e) = sources.edges.get(i).copied().flatten()
            && let Some(t) = body.edge_tolerance(e)
        {
            out.set_edge_tolerance(EdgeId(id(i)), t)
                .map_err(|_| lost())?;
        }
    }
    for (i, origin) in vertex_origin.iter().enumerate() {
        let v = VertexId(id(i));
        if *origin == Origin::Loose
            && let Some(s) = sources.vertices.get(i).copied().flatten()
            && let Some(t) = body.vertex_tolerance(s)
            && out.vertex_tolerance(v).is_none_or(|had| had < t)
        {
            out.set_vertex_tolerance(v, t).map_err(|_| lost())?;
        }
    }
    // Which entities the operation could have moved.
    let changed = |o: Option<&Origin>| matches!(o, Some(Origin::New | Origin::Loose));
    let face_changed: Vec<bool> = out
        .faces
        .iter()
        .enumerate()
        .map(|(f, face)| {
            match sources
                .faces
                .get(f)
                .copied()
                .flatten()
                .and_then(|s| body.face(s))
            {
                Some(s) => s.surface != face.surface || s.reversed != face.reversed,
                None => true,
            }
        })
        .collect();
    let faces_of = edge_faces(out);
    let movable_edges: Vec<bool> = out
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            changed(edge_origin.get(i))
                || changed(vertex_origin.get(e.start.0 as usize))
                || changed(vertex_origin.get(e.end.0 as usize))
                || faces_of.get(i).is_some_and(|faces| {
                    faces
                        .iter()
                        .any(|f| face_changed.get(f.0 as usize).copied().unwrap_or(true))
                })
        })
        .collect();
    let mut movable_vertices: Vec<bool> = vertex_origin
        .iter()
        .map(|o| matches!(o, Origin::New | Origin::Loose))
        .collect();
    for (i, e) in out.edges.iter().enumerate() {
        if movable_edges.get(i).copied().unwrap_or(false) {
            for v in [e.start, e.end] {
                if let Some(slot) = movable_vertices.get_mut(v.0 as usize) {
                    *slot = true;
                }
            }
        }
    }
    settle(
        operation,
        out,
        &movable_edges,
        &movable_vertices,
        tolerance,
        cap,
        working,
    )
}

/// Measures every entity of `out` not within its own tolerance that may
/// have moved and gives it what it needs (at most `working`), then
/// checks: what still fails is refused.
fn settle(
    operation: &'static str,
    out: &mut Body,
    movable_edges: &[bool],
    movable_vertices: &[bool],
    tolerance: &Tolerance,
    cap: f64,
    working: f64,
) -> Result<(), KernelError> {
    let linear = tolerance.linear;
    let mut defects = craft3d_topo::check_capped(out, tolerance, cap);
    for _ in 0..ROUNDS {
        if defects.is_empty() {
            break;
        }
        let mut edges: BTreeSet<EdgeId> = BTreeSet::new();
        let mut vertices: BTreeSet<VertexId> = BTreeSet::new();
        for defect in &defects {
            match *defect {
                Defect::EdgeOffSurface { edge, .. } | Defect::EdgeNotProven { edge, .. } => {
                    edges.insert(edge);
                }
                Defect::VertexOffEdge { vertex, .. }
                | Defect::VertexTighterThanEdge { vertex, .. } => {
                    vertices.insert(vertex);
                }
                _ => {}
            }
        }
        let mut given = false;
        for edge in edges {
            if !movable_edges.get(edge.0 as usize).copied().unwrap_or(false) {
                continue;
            }
            let measured = measure_edge(out, edge, tolerance, cap).ok().flatten();
            let t = needed(measured, working);
            if t > out.edge_tolerance_at(edge, linear) {
                out.set_edge_tolerance(edge, t)
                    .map_err(|_| invalid(operation, defects.clone()))?;
                given = true;
            }
        }
        for vertex in vertices {
            if !movable_vertices
                .get(vertex.0 as usize)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let measured = measure_vertex(out, vertex).ok().flatten();
            let t = needed(measured, working);
            if t > out.vertex_tolerance_at(vertex, linear) {
                out.set_vertex_tolerance(vertex, t)
                    .map_err(|_| invalid(operation, defects.clone()))?;
                given = true;
            }
        }
        if !given {
            break;
        }
        defects = craft3d_topo::check_capped(out, tolerance, cap);
    }
    if defects.is_empty() {
        Ok(())
    } else {
        Err(invalid(operation, defects))
    }
}

fn invalid(operation: &'static str, mut defects: Vec<Defect>) -> KernelError {
    defects.truncate(MAX_REPORTED);
    KernelError::InvalidBody { operation, defects }
}

/// What an entity measured at `measured` is given: that, a little larger
/// (`RECORD_MARGIN`), never more than the working tolerance; one that
/// could not be measured gets the working tolerance, which the final
/// check holds it to.
fn needed(measured: Option<f64>, working: f64) -> f64 {
    measured.map_or(working, |m| (m * RECORD_MARGIN).min(working))
}

fn id(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}
