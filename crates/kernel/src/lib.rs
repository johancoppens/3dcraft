//! Modelling operations on B-rep bodies. Each is a pure function that
//! returns a body which passes `craft3d_topo::check`, or an error that says
//! why it cannot (docs/architecture.md §5.3).
//!
//! So far: primitive solids. Extrude, revolve, booleans and blends arrive
//! with their milestones.

pub mod primitives;

use craft3d_geom::GeomError;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum KernelError {
    /// A size that must be positive and finite is not.
    InvalidSize {
        name: &'static str,
        value: f64,
    },
    Geometry(GeomError),
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelError::InvalidSize { name, value } => {
                write!(f, "{name} must be a positive, finite length, got {value}")
            }
            KernelError::Geometry(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for KernelError {}

impl From<GeomError> for KernelError {
    fn from(e: GeomError) -> Self {
        KernelError::Geometry(e)
    }
}
