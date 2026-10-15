//! Where the shells of a body touch, themselves or each other, at a vertex
//! (docs/architecture.md §5.5 › *Never through itself*).
//!
//! [`check`](crate::check) sees how a body hangs together; it does not see
//! that a vertex of the body lies on another vertex or on an edge it is not
//! an end of. Within one shell that is a surface no closed mesh can be
//! made of (two walls of one shell touching along a line, or in a point);
//! between two shells of one solid it is geometry of zero thickness (two
//! blocks of one body sharing an edge or a corner), which other modellers
//! refuse too ("Cannot create zero-thickness geometry"; the developer's
//! decision of 2026-10-15). Separate bodies may touch: they are not
//! looked at together here.

use crate::{Body, BodyKind, EdgeId};
use craft3d_math::Vec3;
use std::fmt;

/// What a vertex touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Touched {
    /// Another vertex.
    Vertex(u32),
    /// An edge it is not an end of.
    Edge(u32),
}

/// A place where a body's shells touch: a vertex lies, within the
/// distance asked for, on another vertex or on an edge it is not an end
/// of. `shells` are the shell of the vertex and that of what it touches:
/// the same shell for a shell that touches itself, two shells for
/// geometry of zero thickness between them. Faces that only touch (a
/// cylinder resting on a plane) have no vertex there and are not this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    pub vertex: u32,
    pub touches: Touched,
    pub at: Vec3,
    pub distance: f64,
    pub shells: [u32; 2],
}

impl Touch {
    /// Two shells touch, rather than one shell itself.
    pub fn between_shells(&self) -> bool {
        self.shells[0] != self.shells[1]
    }
}

/// Which touches [`touches`] looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchScope {
    /// Within one shell, and (in a solid) between two shells.
    All,
    /// Between two shells of a solid only.
    BetweenShells,
}

/// Why [`touches`] could not look.
#[derive(Clone, Debug, PartialEq)]
pub enum TouchError {
    /// The distance is negative or not finite.
    InvalidDistance(f64),
    /// More vertices or edges than ids can number.
    TooMany,
    /// A vertex that is not finite, or an edge without a box.
    InvalidBody(String),
}

impl fmt::Display for TouchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TouchError::InvalidDistance(d) => {
                write!(f, "a distance must be finite and not negative, got {d}")
            }
            TouchError::TooMany => write!(f, "more vertices or edges than ids can number"),
            TouchError::InvalidBody(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for TouchError {}

/// Where the shells of `body` touch (see [`Touch`]): vertices closer than
/// `near` to another vertex or to an edge they are not an end of, of the
/// same shell, or (in a solid, [`BodyKind::Solid`]) of another shell;
/// `scope` narrows that to pairs of shells. At most `limit`, in vertex
/// order. On the B-rep, not on a mesh: vertices sorted along x and edges
/// by their boxes, so it costs little more than sorting them. A vertex or
/// edge that belongs to no shell is not looked at.
pub fn touches(
    body: &Body,
    near: f64,
    limit: usize,
    scope: TouchScope,
) -> Result<Vec<Touch>, TouchError> {
    if !(near.is_finite() && near >= 0.0) {
        return Err(TouchError::InvalidDistance(near));
    }
    let mut points: Vec<(f64, u32, Vec3)> = Vec::with_capacity(body.vertices.len());
    for (v, vertex) in body.vertices.iter().enumerate() {
        let id = u32::try_from(v).map_err(|_| TouchError::TooMany)?;
        if !vertex.point.is_finite() {
            return Err(TouchError::InvalidBody(format!("vertex {v} is not finite")));
        }
        points.push((vertex.point.x, id, vertex.point));
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    // The shell of every vertex and edge.
    let mut vertex_shell: Vec<Option<u32>> = vec![None; body.vertices.len()];
    let mut edge_shell: Vec<Option<u32>> = vec![None; body.edges.len()];
    for (s, shell) in body.shells.iter().enumerate() {
        let s = u32::try_from(s).map_err(|_| TouchError::TooMany)?;
        for face in shell.faces.iter().filter_map(|f| body.face(*f)) {
            for coedge in face
                .loops
                .iter()
                .filter_map(|l| body.loop_(*l))
                .flat_map(|l| &l.coedges)
            {
                if let Some(slot) = edge_shell.get_mut(coedge.edge.0 as usize) {
                    *slot = Some(s);
                }
                if let Some(edge) = body.edge(coedge.edge) {
                    for v in [edge.start, edge.end] {
                        if let Some(slot) = vertex_shell.get_mut(v.0 as usize) {
                            *slot = Some(s);
                        }
                    }
                }
            }
        }
    }
    let solid = body.kind == BodyKind::Solid;
    // The shells of a pair, when the pair is one to look at.
    let wanted = |a: Option<u32>, b: Option<u32>| -> Option<[u32; 2]> {
        let (a, b) = (a?, b?);
        let looked_at = match scope {
            TouchScope::All => a == b || solid,
            TouchScope::BetweenShells => a != b && solid,
        };
        looked_at.then_some([a, b])
    };
    let shell_of_vertex = |v: u32| vertex_shell.get(v as usize).copied().flatten();
    let mut pairs: Vec<Touch> = Vec::new();
    // Vertices on vertices.
    for (i, &(x, a, p)) in points.iter().enumerate() {
        for &(y, b, q) in points.iter().skip(i + 1) {
            if y - x > near {
                break;
            }
            let Some(shells) = wanted(shell_of_vertex(a), shell_of_vertex(b)) else {
                continue;
            };
            let distance = p.distance(q);
            if distance <= near {
                let (first, second) = (a.min(b), a.max(b));
                let shells = if first == a {
                    shells
                } else {
                    [shells[1], shells[0]]
                };
                pairs.push(Touch {
                    vertex: first,
                    touches: Touched::Vertex(second),
                    at: p,
                    distance,
                    shells,
                });
            }
        }
    }
    // Vertices on edges they are not an end of.
    for (e, edge) in body.edges.iter().enumerate() {
        let id = u32::try_from(e).map_err(|_| TouchError::TooMany)?;
        let Some(shell) = edge_shell.get(e).copied().flatten() else {
            continue;
        };
        let bbox = body
            .edge_bbox(EdgeId(id))
            .map_err(|err| TouchError::InvalidBody(format!("edge {e} has no box: {err}")))?;
        let Some((lo, hi)) = bbox.corners() else {
            continue;
        };
        let first = points.partition_point(|&(x, _, _)| x < lo.x - near);
        for &(x, v, p) in points.iter().skip(first) {
            if x > hi.x + near {
                break;
            }
            if v == edge.start.0 || v == edge.end.0 || !bbox.contains(p, near) {
                continue;
            }
            let Some(shells) = wanted(shell_of_vertex(v), Some(shell)) else {
                continue;
            };
            let Ok(t) = edge.curve.project(p) else {
                continue;
            };
            let (t0, t1) = edge.range;
            // Into the edge's range where the curve is periodic.
            let t = match edge.curve.period() {
                Some(period) if period > 0.0 => {
                    let turns = ((t - t0) / period).floor();
                    t - turns * period
                }
                _ => t,
            };
            if !(t > t0 && t < t1) {
                continue;
            }
            let distance = edge.curve.eval(t).distance(p);
            if distance <= near {
                pairs.push(Touch {
                    vertex: v,
                    touches: Touched::Edge(id),
                    at: p,
                    distance,
                    shells,
                });
            }
        }
    }
    pairs.sort_by(|a, b| {
        let key = |t: &Touch| match t.touches {
            Touched::Vertex(o) => (t.vertex, 0, o),
            Touched::Edge(o) => (t.vertex, 1, o),
        };
        key(a).cmp(&key(b))
    });
    pairs.truncate(limit);
    Ok(pairs)
}
