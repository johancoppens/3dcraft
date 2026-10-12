//! Why a solve could not produce a solution.

use crate::model::{EntityId, EntityKind};

/// Where in the input a problem was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    /// Index into [`crate::Sketch::entities`].
    Entity(usize),
    /// Index into [`crate::Sketch::constraints`].
    Constraint(usize),
}

impl std::fmt::Display for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Site::Entity(i) => write!(f, "entity {i}"),
            Site::Constraint(i) => write!(f, "constraint {i}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SolveError {
    /// An entity or constraint refers to an id that is not in the sketch.
    UnknownEntity { at: Site, id: EntityId },
    /// An entity or constraint refers to an entity of the wrong kind.
    WrongKind {
        at: Site,
        id: EntityId,
        expected: &'static str,
        found: EntityKind,
    },
    /// A coordinate, radius or constraint value is NaN or infinite, or
    /// evaluating a constraint or an arc at the starting guess overflows.
    NonFinite { at: Site },
    /// An entity that is not a shape (a circle with a negative radius).
    InvalidEntity { entity: usize, reason: &'static str },
    /// A constraint value that no geometry can have (a negative distance).
    InvalidValue {
        constraint: usize,
        reason: &'static str,
    },
    /// The options are unusable (tolerance or rank threshold out of range).
    InvalidOptions { reason: &'static str },
    /// No longer returned. A constraint that is undefined where the sketch
    /// is (the direction of a zero-length line) used to be this error; it
    /// is now a `Conflicting` constraint with an infinite residual and the
    /// reason in `ConstraintReport::undefined`, so that a sketch solved
    /// into such a state can be solved again. The variant stays until the
    /// callers that match on it are adapted.
    Degenerate {
        constraint: usize,
        reason: &'static str,
    },
    /// The sketch is larger than the solver accepts.
    TooLarge {
        what: &'static str,
        count: usize,
        limit: usize,
    },
    /// The iteration ended without reaching the tolerance, and the search
    /// for constraints to blame did not explain why.
    DidNotConverge { iterations: usize },
    /// The solve needed more arithmetic than `SolveOptions::work_limit`.
    WorkLimitExceeded { limit: u64 },
    /// The constraint is not implemented for these entity kinds.
    Unsupported {
        constraint: usize,
        what: &'static str,
    },
}

impl std::fmt::Display for SolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolveError::UnknownEntity { at, id } => {
                write!(f, "{at} refers to entity {}, which does not exist", id.0)
            }
            SolveError::WrongKind {
                at,
                id,
                expected,
                found,
            } => write!(
                f,
                "{at} needs entity {} to be a {expected}, but it is a {found}",
                id.0
            ),
            SolveError::NonFinite { at } => write!(f, "{at} has a value that is not finite"),
            SolveError::InvalidEntity { entity, reason } => {
                write!(f, "entity {entity} is invalid: {reason}")
            }
            SolveError::InvalidValue { constraint, reason } => {
                write!(f, "constraint {constraint} has an invalid value: {reason}")
            }
            SolveError::InvalidOptions { reason } => write!(f, "invalid solve options: {reason}"),
            SolveError::Degenerate { constraint, reason } => {
                write!(f, "constraint {constraint} is undefined here: {reason}")
            }
            SolveError::TooLarge { what, count, limit } => {
                write!(
                    f,
                    "the sketch has {count} {what}; the solver accepts {limit}"
                )
            }
            SolveError::DidNotConverge { iterations } => write!(
                f,
                "the sketch did not solve within tolerance after {iterations} iterations"
            ),
            SolveError::WorkLimitExceeded { limit } => {
                write!(f, "the solve exceeded its work limit of {limit} operations")
            }
            SolveError::Unsupported { constraint, what } => {
                write!(f, "constraint {constraint} is not supported: {what}")
            }
        }
    }
}

impl std::error::Error for SolveError {}
