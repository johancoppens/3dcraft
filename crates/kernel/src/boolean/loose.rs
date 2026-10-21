//! A boolean on bodies with loose entities (vertices and edges with a
//! tolerance of their own, `craft3d_topo::Tolerances`): the decision of
//! 2026-10-18, option (4). The boolean works at the operands' largest
//! tolerance and changes no rule of its own: only the number it is given
//! (`docs/design/booleans.md`, Method › *Tolerant operands*).
//!
//! - **The operands** are checked first at the document's tolerance with
//!   their own tables (and the document's cap): a gap that no entity
//!   records is a defect of the operand and refused, never hidden by the
//!   working tolerance.
//! - **The working tolerance** is the largest of the document's and every
//!   entity tolerance of both operands. Above the cap the boolean refuses
//!   ([`KernelError::ToleranceAboveCap`]).
//! - **What the result's entities carry.** A vertex or an edge kept from an
//!   operand in which it was loose keeps that tolerance; every other
//!   entity starts exact. Then the result is checked at the document's
//!   tolerance with its table, and an entity that is not within its own is
//!   *measured* (`topo::measure_edge`, `topo::measure_vertex`) and given
//!   what it needs, at most the working tolerance it was built at. Only an
//!   entity the boolean could have moved may be given one: one it made,
//!   one loose in its operand, or an exact one next to those (an end of an
//!   edge it made or kept loose, an edge ending at such a vertex, an edge
//!   of a face merged from both operands or lying on the other operand's
//!   surface). Every tolerance given is reported
//!   ([`super::BooleanResult::raised`]); what still fails is refused
//!   ([`KernelError::InvalidBody`]), never returned.
//!
//! A body without loose entities never comes here: the boolean of two
//! exact operands is what it was before tolerant entities existed.

use super::{
    BooleanOptions, BooleanResult, EdgeSource, Operand, Origin, Raised, RaisedEntity, VertexSource,
};
use crate::{KernelError, MAX_REPORTED};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, Defect, EdgeId, RECORD_MARGIN, VertexId, measure_edge, measure_vertex};
use std::collections::BTreeSet;

const OPERATION: &str = "boolean";

/// Checks of the result before giving up: each may give tolerances that
/// the next finds a vertex tighter than (raised edges raise their
/// vertices, so two suffice; one more costs nothing when clean).
const ROUNDS: usize = 3;

/// The tolerance a boolean of `a` and `b` works at, or the refusal when it
/// is above `cap`.
pub(crate) fn working_tolerance(
    a: &Body,
    b: &Body,
    linear: f64,
    cap: f64,
) -> Result<f64, KernelError> {
    let working = a.working_tolerance(linear).max(b.working_tolerance(linear));
    if working <= cap {
        Ok(working)
    } else {
        Err(KernelError::ToleranceAboveCap {
            operation: OPERATION,
            tolerance: working,
            cap,
        })
    }
}

/// The cap a boolean holds entity tolerances to.
pub(crate) fn cap(tolerance: &Tolerance, options: &BooleanOptions) -> f64 {
    options
        .loosest
        .unwrap_or_else(|| craft3d_topo::default_loosest(tolerance.linear))
}

/// Each operand passes the check at the document's tolerance with its own
/// table and the cap.
pub(crate) fn operands_valid(
    a: &Body,
    b: &Body,
    tolerance: &Tolerance,
    cap: f64,
) -> Result<(), KernelError> {
    for (body, operation) in [
        (a, "the first operand of a boolean"),
        (b, "the second operand of a boolean"),
    ] {
        let mut defects = craft3d_topo::check_remembered_capped(body, tolerance, cap);
        if !defects.is_empty() {
            defects.truncate(MAX_REPORTED);
            return Err(KernelError::InvalidBody { operation, defects });
        }
    }
    Ok(())
}

/// Gives the result's entities the tolerances they carry (see the module
/// documentation): `result` was built at `working`.
pub(crate) fn carry(
    result: &mut BooleanResult,
    a: &Body,
    b: &Body,
    tolerance: &Tolerance,
    cap: f64,
    working: f64,
) -> Result<(), KernelError> {
    let source = |operand: Operand| match operand {
        Operand::A => a,
        Operand::B => b,
    };
    let edge_origin: Vec<Origin> = result
        .edges
        .iter()
        .map(|edge| match *edge {
            EdgeSource::Kept { operand, edge } => {
                if source(operand).edge_tolerance(edge).is_some() {
                    Origin::Loose
                } else {
                    Origin::Exact
                }
            }
            EdgeSource::Intersection { .. } => Origin::New,
        })
        .collect();
    let vertex_origin: Vec<Origin> = result
        .vertices
        .iter()
        .map(|vertex| match *vertex {
            VertexSource::Kept { operand, vertex } => {
                if source(operand).vertex_tolerance(vertex).is_some() {
                    Origin::Loose
                } else {
                    Origin::Exact
                }
            }
            VertexSource::New { .. } => Origin::New,
        })
        .collect();
    // Kept loose entities keep their tolerance.
    for (i, edge) in result.edges.iter().enumerate() {
        if let EdgeSource::Kept { operand, edge } = *edge
            && let Some(t) = source(operand).edge_tolerance(edge)
        {
            result
                .body
                .set_edge_tolerance(EdgeId(id(i)), t)
                .map_err(|_| lost())?;
        }
    }
    for (i, vertex) in result.vertices.iter().enumerate() {
        let v = VertexId(id(i));
        if let VertexSource::Kept { operand, vertex } = *vertex
            && let Some(t) = source(operand).vertex_tolerance(vertex)
            && result.body.vertex_tolerance(v).is_none_or(|had| had < t)
        {
            result.body.set_vertex_tolerance(v, t).map_err(|_| lost())?;
        }
    }
    let movable = Movable::new(result, &edge_origin, &vertex_origin);
    let kept = result.body.tolerances.clone();
    settle(result, &movable, tolerance, cap, working)?;
    // Every tolerance the result has that it did not keep from a loose
    // entity of an operand, as it holds it now (a vertex raised with its
    // edge included).
    let mut raised = Vec::new();
    for (v, t) in result.body.tolerances.vertices() {
        if kept.vertices().all(|(k, had)| k != v || had != t) {
            raised.push(Raised {
                entity: RaisedEntity::Vertex(v),
                tolerance: t,
                origin: vertex_origin
                    .get(v.0 as usize)
                    .copied()
                    .unwrap_or(Origin::New),
            });
        }
    }
    for (e, t) in result.body.tolerances.edges() {
        if kept.edges().all(|(k, had)| k != e || had != t) {
            raised.push(Raised {
                entity: RaisedEntity::Edge(e),
                tolerance: t,
                origin: edge_origin
                    .get(e.0 as usize)
                    .copied()
                    .unwrap_or(Origin::New),
            });
        }
    }
    result.raised = raised;
    Ok(())
}

/// Which entities of the result the boolean may have moved, and so may
/// give a tolerance (module documentation).
struct Movable {
    edges: Vec<Option<Origin>>,
    vertices: Vec<Option<Origin>>,
}

impl Movable {
    fn new(result: &BooleanResult, edge_origin: &[Origin], vertex_origin: &[Origin]) -> Movable {
        let body = &result.body;
        let changed = |o: Option<&Origin>| matches!(o, Some(Origin::New | Origin::Loose));
        // Edges of faces the boolean joined across the operands: merged
        // from faces of both, or bounded by an edge of the other operand.
        let mut across: BTreeSet<EdgeId> = BTreeSet::new();
        for (f, face) in body.faces.iter().enumerate() {
            let merged = result.merged.get(f).map_or(&[][..], Vec::as_slice);
            let operands: BTreeSet<Operand> = merged.iter().map(|s| s.operand).collect();
            let face_operand = result.faces.get(f).map(|s| s.operand);
            for l in face.loops.iter().filter_map(|l| body.loop_(*l)) {
                for c in &l.coedges {
                    let other = match result.edges.get(c.edge.0 as usize) {
                        Some(EdgeSource::Kept { operand, .. }) => Some(*operand) != face_operand,
                        _ => false,
                    };
                    if operands.len() > 1 || other {
                        across.insert(c.edge);
                    }
                }
            }
        }
        let edges: Vec<Option<Origin>> = body
            .edges
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let origin = edge_origin.get(i).copied();
                let touched = changed(vertex_origin.get(e.start.0 as usize))
                    || changed(vertex_origin.get(e.end.0 as usize))
                    || across.contains(&EdgeId(id(i)));
                match origin {
                    Some(Origin::Exact) if !touched => None,
                    other => other,
                }
            })
            .collect();
        let mut near_vertex = vec![false; body.vertices.len()];
        for (i, e) in body.edges.iter().enumerate() {
            if changed(edge_origin.get(i)) || across.contains(&EdgeId(id(i))) {
                for v in [e.start, e.end] {
                    if let Some(slot) = near_vertex.get_mut(v.0 as usize) {
                        *slot = true;
                    }
                }
            }
        }
        let vertices = vertex_origin
            .iter()
            .zip(&near_vertex)
            .map(|(origin, near)| match origin {
                Origin::Exact if !near => None,
                other => Some(*other),
            })
            .collect();
        Movable { edges, vertices }
    }
}

/// Measures every entity of the result that is not within its own
/// tolerance and may have been moved, and gives it what it needs (at most
/// `working`), then checks: what still fails is refused.
fn settle(
    result: &mut BooleanResult,
    movable: &Movable,
    tolerance: &Tolerance,
    cap: f64,
    working: f64,
) -> Result<(), KernelError> {
    let linear = tolerance.linear;
    let mut defects = craft3d_topo::check_capped(&result.body, tolerance, cap);
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
        let body = &mut result.body;
        let mut given = false;
        // Edges first: an edge raises its vertices with it.
        for edge in edges {
            let Some(&Some(_)) = movable.edges.get(edge.0 as usize) else {
                continue;
            };
            let measured = measure_edge(body, edge, tolerance, cap).ok().flatten();
            let t = needed(measured, working);
            if t > body.edge_tolerance_at(edge, linear) {
                body.set_edge_tolerance(edge, t).map_err(|_| lost())?;
                given = true;
            }
        }
        for vertex in vertices {
            let Some(&Some(_)) = movable.vertices.get(vertex.0 as usize) else {
                continue;
            };
            let measured = measure_vertex(body, vertex).ok().flatten();
            let t = needed(measured, working);
            if t > body.vertex_tolerance_at(vertex, linear) {
                body.set_vertex_tolerance(vertex, t).map_err(|_| lost())?;
                given = true;
            }
        }
        if !given {
            break;
        }
        defects = craft3d_topo::check_capped(&result.body, tolerance, cap);
    }
    if defects.is_empty() {
        Ok(())
    } else {
        defects.truncate(MAX_REPORTED);
        Err(KernelError::InvalidBody {
            operation: OPERATION,
            defects,
        })
    }
}

/// What an entity measured at `measured` is given: that, a little larger
/// (`RECORD_MARGIN`), but never more than the working tolerance it was
/// built at; an entity that could not be measured gets the working
/// tolerance, which the final check holds it to.
fn needed(measured: Option<f64>, working: f64) -> f64 {
    measured.map_or(working, |m| (m * RECORD_MARGIN).min(working))
}

fn id(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// An entity of the result that the result does not have: the sources
/// and the body disagree, which is a bug.
fn lost() -> KernelError {
    KernelError::Unsupported {
        operation: OPERATION,
        reason: super::LOST,
        faces: Vec::new(),
    }
}
