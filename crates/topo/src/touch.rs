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
    let grid = PointGrid::new(&points, near);
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
    let reach = Vec3::new(near, near, near);
    for &(_, a, p) in &points {
        for b in grid.within(p - reach, p + reach) {
            if b <= a {
                continue;
            }
            let Some(q) = body.vertices.get(b as usize).map(|v| v.point) else {
                continue;
            };
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
    // The edges between two vertices, by their ends (the smaller first).
    let mut by_ends: std::collections::BTreeMap<(u32, u32), Vec<u32>> =
        std::collections::BTreeMap::new();
    for (e, edge) in body.edges.iter().enumerate() {
        let id = u32::try_from(e).map_err(|_| TouchError::TooMany)?;
        let (a, b) = (edge.start.0, edge.end.0);
        by_ends.entry((a.min(b), a.max(b))).or_default().push(id);
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
        for v in grid.within(lo - reach, hi + reach) {
            let Some(p) = body.vertices.get(v as usize).map(|v| v.point) else {
                continue;
            };
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
            if distance <= near && !at_a_corner(body, &by_ends, v, edge, p, distance, near) {
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

/// True when vertex `v` (at `p`) is near `edge` only because the two
/// leave a common vertex at a sharp angle: an edge joins `v` to an end
/// `w` of `edge`, and `v` is within `near / sin(φ / 2)` of `w`, φ the
/// angle between the two edges' directions out of `w`. Two curves that
/// leave a point at an angle φ stay within `near` of each other for about
/// `near / φ` from it, so a face with a corner sharper than the tolerance
/// can resolve (the end wall of a crack that thins to a point: the
/// Garabit pier's diagonal, 2026-10-21) has the far end of one side
/// within `near` of the other side without the shell coming back to
/// itself there; the same margin as the boolean's for two faces at a
/// shared edge (`wedged`). A vertex further from the corner than that is
/// looked at as before. The corner has to open (sin(φ/2) above
/// [`CORNER_GUARD`]) and the vertex has to be as far from the edge as
/// the corner puts it (at least half of `d sin φ`, `d` its distance from
/// `w`): two edges that leave a vertex along one line, or that come back
/// towards each other, are the shell meeting itself (a grid of blocks
/// sharing an edge).
fn at_a_corner(
    body: &Body,
    by_ends: &std::collections::BTreeMap<(u32, u32), Vec<u32>>,
    v: u32,
    edge: &crate::Edge,
    p: Vec3,
    distance: f64,
    near: f64,
) -> bool {
    // The unit direction out of vertex `w` along `e`.
    let out_of = |e: &crate::Edge, w: u32| -> Option<Vec3> {
        let (t, sign) = if e.start.0 == w {
            (e.range.0, 1.0)
        } else if e.end.0 == w {
            (e.range.1, -1.0)
        } else {
            return None;
        };
        (e.curve.derivatives(t).1 * sign).normalized()
    };
    [edge.start.0, edge.end.0].into_iter().any(|w| {
        let Some(q) = body.vertices.get(w as usize).map(|x| x.point) else {
            return false;
        };
        let Some(along) = out_of(edge, w) else {
            return false;
        };
        by_ends
            .get(&(v.min(w), v.max(w)))
            .into_iter()
            .flatten()
            .filter_map(|&k| body.edges.get(k as usize))
            .any(|other| {
                let Some(side) = out_of(other, w) else {
                    return false;
                };
                // sin(φ/2) from the two unit directions.
                let half = 0.5 * (side - along).length();
                let d = q.distance(p);
                half > CORNER_GUARD && d * half <= near && distance >= d * half
            })
    })
}

/// Below this sine of half the angle two edges leave a vertex along one
/// line, to rounding: the guard tangency is read by in the boolean.
const CORNER_GUARD: f64 = 1e-6;

/// The vertices in a grid of cells, about one vertex per cell (never
/// narrower than the distance looked for), so that the vertices in a box
/// are found without looking at all of them, wherever they lie (sorted
/// along one axis, vertices that share it would all be looked at).
struct PointGrid {
    low: Vec3,
    cell: f64,
    size: [usize; 3],
    cells: std::collections::BTreeMap<usize, Vec<u32>>,
    all: Vec<u32>,
}

impl PointGrid {
    fn new(points: &[(f64, u32, Vec3)], near: f64) -> PointGrid {
        let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut hi = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for &(_, _, p) in points {
            lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        let extent = hi - lo;
        let largest = extent.x.max(extent.y).max(extent.z);
        let thin = (largest * 1e-3).max(near);
        let count = points.len().max(1) as f64;
        let cell = (extent.x.max(thin) * extent.y.max(thin) * extent.z.max(thin) / count)
            .cbrt()
            .max(near);
        let all: Vec<u32> = points.iter().map(|&(_, id, _)| id).collect();
        let mut grid = PointGrid {
            low: lo,
            cell,
            size: [1, 1, 1],
            cells: std::collections::BTreeMap::new(),
            all,
        };
        if !(cell.is_finite() && cell > 0.0 && largest.is_finite()) {
            return grid;
        }
        let along = |e: f64| ((e / cell).floor() as usize).saturating_add(1).min(1 << 20);
        grid.size = [along(extent.x), along(extent.y), along(extent.z)];
        for &(_, id, p) in points {
            let key = grid.key(grid.index(p));
            grid.cells.entry(key).or_default().push(id);
        }
        grid
    }

    fn index(&self, p: Vec3) -> [usize; 3] {
        let at = |x: f64, low: f64, size: usize| {
            let i = ((x - low) / self.cell).floor();
            if i.is_finite() && i > 0.0 {
                (i as usize).min(size.saturating_sub(1))
            } else {
                0
            }
        };
        let [sx, sy, sz] = self.size;
        [
            at(p.x, self.low.x, sx),
            at(p.y, self.low.y, sy),
            at(p.z, self.low.z, sz),
        ]
    }

    fn key(&self, [x, y, z]: [usize; 3]) -> usize {
        let [sx, sy, _] = self.size;
        x.saturating_add(sx.saturating_mul(y.saturating_add(sy.saturating_mul(z))))
    }

    /// The vertices in the cells the box from `lo` to `hi` overlaps (a
    /// superset of those in the box), or all of them when that is more
    /// cells than vertices.
    fn within(&self, lo: Vec3, hi: Vec3) -> Vec<u32> {
        if self.cells.is_empty() {
            return self.all.clone();
        }
        let ([x0, y0, z0], [x1, y1, z1]) = (self.index(lo), self.index(hi));
        let span = (x1.saturating_sub(x0) + 1)
            .saturating_mul(y1.saturating_sub(y0) + 1)
            .saturating_mul(z1.saturating_sub(z0) + 1);
        if span > self.all.len().saturating_add(8) {
            return self.all.clone();
        }
        let mut found = Vec::new();
        for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    if let Some(cell) = self.cells.get(&self.key([x, y, z])) {
                        found.extend_from_slice(cell);
                    }
                }
            }
        }
        found
    }
}
