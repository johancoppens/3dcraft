//! A boolean on bodies with loose entities (vertices and edges with a
//! tolerance of their own, `craft3d_topo::Tolerances`): the decision of
//! 2026-10-18, option (4). The boolean works at the operands' largest
//! tolerance and changes no rule of its own: only the number it is given
//! (`docs/design/booleans.md`, Method › *Tolerant operands*).
//!
//! - **The working tolerance** is the largest of the document's and every
//!   entity tolerance of both operands. Above the cap (the document's,
//!   `BooleanOptions::loosest`) the boolean refuses
//!   ([`KernelError::ToleranceAboveCap`]) rather than let a tolerance grow
//!   unseen.
//! - **What the result's entities carry.** A vertex or an edge kept from
//!   an operand (or a piece of one) keeps that entity's tolerance; one the
//!   boolean made (an intersection edge, a vertex where entities of the two
//!   meet) carries the working tolerance it was built at. Then the result
//!   is checked entity by entity at the document's tolerance with its
//!   table, and every entity that is not within its own tolerance (a kept
//!   vertex merged with a point up to half the working tolerance away, an
//!   edge that now bounds a face of the other operand's surface) is
//!   raised to the working tolerance, at which the boolean built it. What
//!   still fails is refused ([`KernelError::InvalidBody`]), never
//!   returned.
//!
//! A body without loose entities never comes here: the boolean of two
//! exact bodies is what it was before tolerant entities existed.

use super::{BooleanOptions, BooleanResult, EdgeSource, Operand, VertexSource};
use crate::{KernelError, MAX_REPORTED};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, Defect, EdgeId, VertexId};

const OPERATION: &str = "boolean";

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

/// Gives the result's entities the tolerances they carry (see the module
/// documentation) and settles them: `result` was built at `working`.
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
    let built = (working > tolerance.linear).then_some(working);
    let body = &mut result.body;
    for (i, edge) in result.edges.iter().enumerate() {
        let own = match *edge {
            EdgeSource::Kept { operand, edge } => source(operand).edge_tolerance(edge),
            EdgeSource::Intersection { .. } => built,
        };
        if let Some(t) = own {
            body.set_edge_tolerance(EdgeId(id(i)), t)
                .map_err(|_| lost())?;
        }
    }
    for (i, vertex) in result.vertices.iter().enumerate() {
        let own = match *vertex {
            VertexSource::Kept { operand, vertex } => source(operand).vertex_tolerance(vertex),
            VertexSource::New { .. } => built,
        };
        let v = VertexId(id(i));
        if let Some(t) = own
            && body.vertex_tolerance(v).is_none_or(|had| had < t)
        {
            body.set_vertex_tolerance(v, t).map_err(|_| lost())?;
        }
    }
    settle(body, tolerance, cap, working)
}

/// Raises every entity of `body` that is not within its own tolerance to
/// `working`, then checks: what still fails is refused.
fn settle(
    body: &mut Body,
    tolerance: &Tolerance,
    cap: f64,
    working: f64,
) -> Result<(), KernelError> {
    // Raising an edge raises its vertices with it, so a second round only
    // finds what the first could not help.
    for _ in 0..2 {
        let defects = craft3d_topo::check_capped(body, tolerance, cap);
        if defects.is_empty() {
            return Ok(());
        }
        let mut raised = false;
        for defect in &defects {
            let (vertex, edge) = match *defect {
                Defect::VertexOffEdge { vertex, .. }
                | Defect::VertexTighterThanEdge { vertex, .. } => (Some(vertex), None),
                Defect::EdgeOffSurface { edge, .. } | Defect::EdgeNotProven { edge, .. } => {
                    (None, Some(edge))
                }
                _ => (None, None),
            };
            if let Some(vertex) = vertex
                && body.vertex_tolerance_at(vertex, tolerance.linear) < working
            {
                body.set_vertex_tolerance(vertex, working)
                    .map_err(|_| lost())?;
                raised = true;
            }
            if let Some(edge) = edge
                && body.edge_tolerance_at(edge, tolerance.linear) < working
            {
                body.set_edge_tolerance(edge, working).map_err(|_| lost())?;
                raised = true;
            }
        }
        if !raised {
            break;
        }
    }
    let mut defects = craft3d_topo::check_capped(body, tolerance, cap);
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
