//! B-rep topology: vertices, edges, loops, faces and shells over the curves
//! and surfaces of `craft3d-geom`, with a validity checker ([`check`]) and
//! mass properties computed from the exact geometry ([`mass_properties`]).
//!
//! Conventions:
//! - An edge runs along its curve from `range.0` to `range.1`, from its
//!   start vertex to its end vertex. A closed edge (a full circle) has one
//!   vertex as both.
//! - A face's outward normal is its surface's normal `∂u × ∂v`, or the
//!   opposite when `reversed`.
//! - Loops are oriented so that, seen from outside (looking against the
//!   outward normal), the face lies to the left: outer boundaries run
//!   counter-clockwise, holes clockwise.
//! - A face without loops is a whole closed surface (a sphere, a torus).
//!
//! Not here yet: parameter-space curves on faces (they are recovered by
//! projection where needed), per-entity tolerances, persistent names
//! (docs/architecture.md §4.1, milestone M6).

mod bits;
mod check;
mod mass;
mod transform;

pub use check::{Defect, check, genus};
pub use mass::{MassError, MassProperties, mass_properties};
pub use transform::{Offsets, TransformError, merge};

use craft3d_geom::{Curve3, Surface};
use craft3d_math::Vec3;

macro_rules! id {
    ($name:ident, $what:literal) => {
        #[doc = concat!("Index of ", $what, " within its [`Body`].")]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);
    };
}
id!(VertexId, "a vertex");
id!(EdgeId, "an edge");
id!(LoopId, "a loop");
id!(FaceId, "a face");
id!(ShellId, "a shell");

#[derive(Clone, Debug, PartialEq)]
pub struct Vertex {
    pub point: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub curve: Curve3,
    /// Curve parameters of the start and the end; `range.0 < range.1`.
    pub range: (f64, f64),
    pub start: VertexId,
    pub end: VertexId,
}

/// The use of an edge by a loop, in the edge's own direction or against it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coedge {
    pub edge: EdgeId,
    pub reversed: bool,
}

/// A closed chain of coedges: each ends where the next begins.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Loop {
    pub coedges: Vec<Coedge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub surface: Surface,
    /// True when the outward normal is opposite to the surface normal.
    pub reversed: bool,
    pub loops: Vec<LoopId>,
}

/// A connected set of faces.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Shell {
    pub faces: Vec<FaceId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// Closed shells enclosing a volume.
    Solid,
    /// Open faces.
    Sheet,
}

/// A boundary representation. Entities refer to each other by index; the
/// `add_*` methods return the index of what they added. Nothing is checked
/// on construction: run [`check`] on a finished body.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub kind: BodyKind,
    pub vertices: Vec<Vertex>,
    pub edges: Vec<Edge>,
    pub loops: Vec<Loop>,
    pub faces: Vec<Face>,
    pub shells: Vec<Shell>,
}

fn next_id(len: usize) -> u32 {
    // A body with 2^32 entities does not fit in memory; saturating keeps
    // the id invalid rather than wrapping onto an existing entity.
    u32::try_from(len).unwrap_or(u32::MAX)
}

impl Body {
    pub fn new(kind: BodyKind) -> Self {
        Self {
            kind,
            vertices: Vec::new(),
            edges: Vec::new(),
            loops: Vec::new(),
            faces: Vec::new(),
            shells: Vec::new(),
        }
    }

    pub fn add_vertex(&mut self, point: Vec3) -> VertexId {
        let id = VertexId(next_id(self.vertices.len()));
        self.vertices.push(Vertex { point });
        id
    }

    pub fn add_edge(
        &mut self,
        curve: Curve3,
        range: (f64, f64),
        start: VertexId,
        end: VertexId,
    ) -> EdgeId {
        let id = EdgeId(next_id(self.edges.len()));
        self.edges.push(Edge {
            curve,
            range,
            start,
            end,
        });
        id
    }

    pub fn add_loop(&mut self, coedges: Vec<Coedge>) -> LoopId {
        let id = LoopId(next_id(self.loops.len()));
        self.loops.push(Loop { coedges });
        id
    }

    pub fn add_face(&mut self, surface: Surface, reversed: bool, loops: Vec<LoopId>) -> FaceId {
        let id = FaceId(next_id(self.faces.len()));
        self.faces.push(Face {
            surface,
            reversed,
            loops,
        });
        id
    }

    pub fn add_shell(&mut self, faces: Vec<FaceId>) -> ShellId {
        let id = ShellId(next_id(self.shells.len()));
        self.shells.push(Shell { faces });
        id
    }

    pub fn vertex(&self, id: VertexId) -> Option<&Vertex> {
        self.vertices.get(id.0 as usize)
    }

    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(id.0 as usize)
    }

    pub fn loop_(&self, id: LoopId) -> Option<&Loop> {
        self.loops.get(id.0 as usize)
    }

    pub fn face(&self, id: FaceId) -> Option<&Face> {
        self.faces.get(id.0 as usize)
    }

    /// The vertices a coedge runs from and to.
    pub fn coedge_ends(&self, coedge: Coedge) -> Option<(VertexId, VertexId)> {
        let edge = self.edge(coedge.edge)?;
        Some(if coedge.reversed {
            (edge.end, edge.start)
        } else {
            (edge.start, edge.end)
        })
    }
}
