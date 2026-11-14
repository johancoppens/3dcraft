//! A vertex of four or more faces that the moved faces no longer meet in
//! (2026-11-14, the kernel set's H-01, H-06 and H-07): it becomes a tree
//! of vertices of three faces each, joined by new edges where two faces
//! that met only in the vertex meet now.
//!
//! **What the tree is.** Round the vertex the faces come one after the
//! other, each between the edge it comes in by and the edge it leaves by
//! (a *sector*). Each vertex of the tree is where three of them meet; the
//! old edges end in the vertex of the three that holds their two faces,
//! and every new edge lies between two faces that are not next to each
//! other round the vertex. Such a tree is a triangulation of the polygon
//! whose corners are the sectors (its sides the old edges, its diagonals
//! the new edges, its triangles the new vertices): two for four faces,
//! five for five, fourteen for six.
//!
//! **Which tree.** The moved faces decide, not a guess:
//! - all the edges at the vertex convex: the solid there is where it is
//!   behind every face, and so is the hollow behind the moved faces; its
//!   corners are the points where three moved faces meet that lie behind
//!   every other one (in front of none of them by more than the tolerance);
//! - all concave: the solid there is where it is behind any one face, the
//!   room outside it convex; the corners lie in front of every other face;
//! - edges of both kinds, or a smooth edge among them: only where exactly
//!   one tree can be made at all (every one of its corners exists where
//!   three moved faces meet; in H-01 the other tree needs the box's side
//!   and the cylinder's end, moved apart, to meet); otherwise refused as
//!   before.
//!
//! A corner whose side of a face cannot be told (its nearest point on
//! that face's surface is on a border, not square below it) fails its
//! tree.
//!
//! Exactly one tree must pass, else nothing is changed and the vertex is
//! refused as before ("the faces … no longer meet in one point").
//! Everything after (the edges found again, every check on the faces and
//! the body) is the same as for any vertex.

use super::{SQUARE, TANGENT, distance, meet};
use craft3d_geom::{Curve3, Line, Surface};
use craft3d_math::Vec3;
use craft3d_topo::{Body, Coedge, EdgeId, FaceId, LoopId, VertexId};
use std::collections::BTreeMap;

/// Most faces round a vertex that are split (132 trees for eight).
const MOST_SECTORS: usize = 8;

/// A face round the vertex: its loop, the place in it of the coedge that
/// comes in to the vertex, the edge it comes in by and the one it leaves by.
#[derive(Clone, Copy, Debug)]
struct Sector {
    face: FaceId,
    loop_: LoopId,
    incoming: usize,
    leaves_by: EdgeId,
    comes_by: EdgeId,
}

/// What the vertex became: the new vertices (the first in its own place)
/// and the new edges, to be found again like any other.
#[derive(Clone, Debug, Default)]
pub(super) struct Split {
    pub vertices: Vec<VertexId>,
    pub edges: Vec<EdgeId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Convex,
    Concave,
    Mixed,
}

/// The sectors round `vertex` in `made`, in order (each leaves by the edge
/// the next comes in by); `None` where they do not go round it once, a
/// face comes round twice, or an edge ends in it at both ends.
fn sectors(made: &Body, vertex: VertexId) -> Option<Vec<Sector>> {
    let mut found: Vec<Sector> = Vec::new();
    for (index, face) in made.faces.iter().enumerate() {
        let id = FaceId(u32::try_from(index).ok()?);
        for loop_ in &face.loops {
            let coedges = &made.loop_(*loop_)?.coedges;
            let count = coedges.len();
            for (at, coedge) in coedges.iter().enumerate() {
                let (from, to) = made.coedge_ends(*coedge)?;
                if from == vertex && to == vertex {
                    return None;
                }
                if to != vertex {
                    continue;
                }
                let next = coedges.get((at + 1) % count)?;
                let (leaves, _) = made.coedge_ends(*next)?;
                if leaves != vertex {
                    return None;
                }
                found.push(Sector {
                    face: id,
                    loop_: *loop_,
                    incoming: at,
                    leaves_by: next.edge,
                    comes_by: coedge.edge,
                });
            }
        }
    }
    // Each face once.
    for (i, a) in found.iter().enumerate() {
        if found.iter().skip(i + 1).any(|b| b.face == a.face) {
            return None;
        }
    }
    // In order round the vertex.
    let mut ordered: Vec<Sector> = Vec::with_capacity(found.len());
    let mut current = *found.first()?;
    for _ in 0..found.len() {
        ordered.push(current);
        let next = found
            .iter()
            .find(|s| s.comes_by == current.leaves_by && s.face != current.face)?;
        current = *next;
    }
    let first = ordered.first()?;
    (current.face == first.face && ordered.len() == found.len()).then_some(ordered)
}

/// Whether the edge `sector` leaves by, between its face and the next
/// sector's, is convex or concave at `at`, on the old faces of `body`;
/// `None` where it is smooth (the outward normals within the sine
/// `tweak::TANGENT`), which makes the corner one of both kinds
/// (`Kind::Mixed`): only a tree that alone can be made is taken there.
/// `TANGENT` is the face operations' one classification constant
/// (docs/design/face-operations.md: faces meeting at a smaller angle are
/// tangent by construction, and the iteration that finds the vertices
/// takes such a pair as one direction); the split has to call the same
/// pairs smooth as that iteration does, so it uses the same constant, not
/// the document's angular tolerance.
fn convexity(body: &Body, made: &Body, sector: &Sector, next: &Sector, at: Vec3) -> Option<Kind> {
    let edge = body.edge(sector.leaves_by)?;
    let coedge = made
        .loop_(sector.loop_)?
        .coedges
        .get((sector.incoming + 1) % made.loop_(sector.loop_)?.coedges.len())?;
    // The way the face's loop leaves the vertex along the edge.
    let along = if coedge.reversed {
        -edge.curve.derivatives(edge.range.1).1
    } else {
        edge.curve.derivatives(edge.range.0).1
    };
    let a = body.face(sector.face)?.outward(at)?;
    let b = body.face(next.face)?.outward(at)?;
    let cross = a.cross(b);
    if cross.length() <= TANGENT {
        return None;
    }
    let side = cross.dot(along.normalized()?);
    if !side.is_finite() {
        return None;
    }
    Some(if side > 0.0 {
        Kind::Convex
    } else {
        Kind::Concave
    })
}

/// Every triangulation of the polygon `0..n`, each a list of triangles
/// (three corners, ascending).
fn triangulations(n: usize) -> Vec<Vec<[usize; 3]>> {
    fn of(i: usize, j: usize) -> Vec<Vec<[usize; 3]>> {
        if j < i + 2 {
            return vec![Vec::new()];
        }
        let mut all = Vec::new();
        for k in i + 1..j {
            for left in of(i, k) {
                for right in of(k, j) {
                    let mut t = Vec::with_capacity(left.len() + right.len() + 1);
                    t.push([i, k, j]);
                    t.extend(left.iter().copied());
                    t.extend(right.iter().copied());
                    all.push(t);
                }
            }
        }
        all
    }
    if n < 3 {
        return Vec::new();
    }
    of(0, n - 1)
}

/// The outward signed distance from `point` to the face `face` of `made`
/// (its new surface): negative behind it. `None` where that cannot be
/// told: no nearest point with a normal, or the nearest point on a border
/// of the surface (a freeform face's edge, a cone's apex) and not square
/// below the point, where the distance has no side (the review of
/// 2026-11-14: `tweak::distance` gives the unsigned length there, and its
/// sign flipped for a reversed face would have passed a corner beyond the
/// border as behind it). Within the tolerance of the surface it is on it.
/// The test for a border foot is `tweak::distance_toward`'s.
fn in_front(made: &Body, face: FaceId, point: Vec3, tol: f64) -> Option<f64> {
    let face = made.face(face)?;
    let (u, v) = face.surface.project(point).ok()?;
    let normal = face.surface.normal(u, v)?;
    let offset = point - face.surface.eval(u, v);
    let along = offset.dot(normal);
    let aside = (offset - normal * along).length();
    if !(along.is_finite() && aside.is_finite()) {
        return None;
    }
    if offset.length() <= tol {
        return Some(0.0);
    }
    if aside > SQUARE * offset.length() + 1e-15 * (1.0 + point.length())
        && aside > along.abs() * 1e-3
    {
        return None;
    }
    Some(if face.reversed { -along } else { along })
}

/// How far from where the vertex was a corner of its tree may lie, in
/// times the farthest its faces moved from it. A budget, not a tuned
/// value: corners of faces that meet at an angle lie within a few times
/// the distance (in the kernel set, floors falling 0.6° and 1° to a drain,
/// within 1.0 to 2.2 times, measured); three faces of which two are
/// parallel to rounding (a face and the other side of the plane it lies
/// in, the box's side and the cylinder's end of H-01, turned off the
/// axes) meet, if at all, at the rounding's reciprocal.
const REACH: f64 = 1e3;

/// What is known of the corner of one triangle (three sectors): where its
/// faces' new surfaces meet nearest where the vertex was, within the
/// reach, and whether it lies on the right side of every other face for
/// the kind of the vertex. Worked out once per triangle (eight faces have
/// 132 trees but 56 triangles).
#[derive(Clone, Copy, Debug)]
struct Corner {
    point: Option<Vec3>,
    holds: bool,
}

/// The corner of the triangle `t` (see [`Corner`]): behind every other
/// moved face for a convex vertex, in front of every one for a concave
/// one, either for one of both kinds; a side that cannot be told fails.
#[allow(clippy::too_many_arguments)]
fn corner(
    made: &Body,
    sectors: &[Sector],
    t: &[usize; 3],
    kind: Kind,
    from: Vec3,
    reach: f64,
    tol: f64,
) -> Corner {
    let point = t
        .iter()
        .map(|k| Some(&made.face(sectors.get(*k)?.face)?.surface))
        .collect::<Option<Vec<&Surface>>>()
        .and_then(|surfaces| meet(&surfaces, from, tol))
        .filter(|p| p.distance(from) <= reach);
    let holds = point.is_some_and(|p| {
        kind == Kind::Mixed
            || sectors.iter().enumerate().all(|(k, s)| {
                t.contains(&k)
                    || in_front(made, s.face, p, tol).is_some_and(|d| match kind {
                        Kind::Convex => d <= tol,
                        Kind::Concave => d >= -tol,
                        Kind::Mixed => true,
                    })
            })
    });
    Corner { point, holds }
}

/// `vertex` of `made` (whose faces have their new surfaces, its edges
/// still the old ones) split into the tree its moved faces make, or
/// `None` (nothing changed) when that is not one tree (see the module
/// documentation). `body` is the body before: its faces say which edges
/// are convex; `was` is where the vertex was.
pub(super) fn split(
    body: &Body,
    made: &mut Body,
    vertex: VertexId,
    was: Vec3,
    tol: f64,
) -> Option<Split> {
    let sectors = sectors(made, vertex)?;
    let n = sectors.len();
    if !(4..=MOST_SECTORS).contains(&n) {
        return None;
    }
    // The kind of the vertex, from the old faces.
    let mut kinds = Vec::with_capacity(n);
    for (k, s) in sectors.iter().enumerate() {
        let next = sectors.get((k + 1) % n)?;
        kinds.push(convexity(body, made, s, next, was));
    }
    let kind = if kinds.iter().all(|k| *k == Some(Kind::Convex)) {
        Kind::Convex
    } else if kinds.iter().all(|k| *k == Some(Kind::Concave)) {
        Kind::Concave
    } else {
        Kind::Mixed
    };
    // How far the faces moved from the vertex.
    let mut moved = 0.0_f64;
    for s in &sectors {
        let (d, _) = distance(&made.face(s.face)?.surface, was)?;
        moved = moved.max(d.abs());
    }
    let reach = REACH * moved;
    if !(reach.is_finite() && reach > tol) {
        return None;
    }
    let mut known: BTreeMap<[usize; 3], Corner> = BTreeMap::new();
    let mut chosen: Option<(Vec<[usize; 3]>, Vec<Vec3>)> = None;
    for triangles in triangulations(n) {
        if craft3d_geom::allowance::pass() {
            return None;
        }
        let mut points = Vec::with_capacity(triangles.len());
        for t in &triangles {
            let c = *known
                .entry(*t)
                .or_insert_with(|| corner(made, &sectors, t, kind, was, reach, tol));
            match (c.point, c.holds) {
                (Some(p), true) => points.push(p),
                _ => break,
            }
        }
        if points.len() != triangles.len() {
            continue;
        }
        if chosen.is_some() {
            // Two trees: not decided by the faces, so not built.
            return None;
        }
        chosen = Some((triangles, points));
    }
    let (triangles, points) = chosen?;
    // Every two corners apart by more than the tolerance (an edge of the
    // tree shorter than that is no edge).
    for (i, p) in points.iter().enumerate() {
        if points.iter().skip(i + 1).any(|q| p.distance(*q) <= tol) {
            return None;
        }
    }
    apply(made, vertex, &sectors, &triangles, &points)
}

/// The triangle of `triangles` that holds every corner of `of`.
fn holding(triangles: &[[usize; 3]], of: &[usize]) -> Option<usize> {
    triangles
        .iter()
        .position(|t| of.iter().all(|k| t.contains(k)))
}

fn apply(
    made: &mut Body,
    vertex: VertexId,
    sectors: &[Sector],
    triangles: &[[usize; 3]],
    points: &[Vec3],
) -> Option<Split> {
    let n = sectors.len();
    let mut split = Split::default();
    // The new vertices: the first in the old one's place.
    for (i, p) in points.iter().enumerate() {
        let id = if i == 0 {
            made.vertices.get_mut(vertex.0 as usize)?.point = *p;
            vertex
        } else {
            made.add_vertex(*p)
        };
        split.vertices.push(id);
    }
    let corner = |t: usize| split.vertices.get(t).copied();
    // Each old edge ends in the corner of the triangle that holds its two
    // sectors (a side of the polygon).
    let mut ends: Vec<(EdgeId, VertexId)> = Vec::with_capacity(n);
    for (k, s) in sectors.iter().enumerate() {
        let t = holding(triangles, &[k, (k + 1) % n])?;
        ends.push((s.leaves_by, corner(t)?));
    }
    // Per sector, the coedges along the new edges from the corner its
    // incoming edge ends in to the one its outgoing edge starts from: its
    // fan of triangles, from the side it comes in by to the side it leaves
    // by. A diagonal (k, j) runs, in sector k's loop, from the triangle
    // beyond j (towards k − 1) to the one before it; its other sector runs
    // it the other way.
    let mut made_edges: Vec<((usize, usize), EdgeId, VertexId, VertexId)> = Vec::new();
    let mut inserts: Vec<(LoopId, usize, Vec<Coedge>)> = Vec::with_capacity(n);
    for (k, s) in sectors.iter().enumerate() {
        // The neighbours of k in the triangulation, in order round the
        // polygon from k + 1 to k − 1.
        let mut around: Vec<usize> = triangles
            .iter()
            .filter(|t| t.contains(&k))
            .flat_map(|t| t.iter().copied())
            .filter(|j| *j != k)
            .collect();
        around.sort_by_key(|j| (j + n - k - 1) % n);
        around.dedup();
        // The diagonals among them (not k ± 1), from the far end back.
        let mut chain: Vec<Coedge> = Vec::new();
        let count = around.len();
        for r in (1..count.saturating_sub(1)).rev() {
            let j = *around.get(r)?;
            let before = *around.get(r + 1)?;
            let after = *around.get(r - 1)?;
            let from = corner(holding(triangles, &[k, j, before])?)?;
            let to = corner(holding(triangles, &[k, j, after])?)?;
            let key = (k.min(j), k.max(j));
            let coedge = match made_edges.iter().find(|(d, ..)| *d == key) {
                Some((_, edge, start, end)) => {
                    // The other sector runs it the other way.
                    if *start != to || *end != from {
                        return None;
                    }
                    Coedge {
                        edge: *edge,
                        reversed: true,
                    }
                }
                None => {
                    let (a, b) = (made.vertex(from)?.point, made.vertex(to)?.point);
                    let line = Line::through(a, b).ok()?;
                    let edge = made.add_edge(Curve3::Line(line), (0.0, a.distance(b)), from, to);
                    made_edges.push((key, edge, from, to));
                    split.edges.push(edge);
                    Coedge {
                        edge,
                        reversed: false,
                    }
                }
            };
            chain.push(coedge);
        }
        inserts.push((s.loop_, s.incoming, chain));
    }
    // Every diagonal made once and run twice.
    if made_edges.len() + 3 != n {
        return None;
    }
    for (edge, to) in ends {
        let e = made.edges.get_mut(edge.0 as usize)?;
        if e.start == vertex {
            e.start = to;
        } else if e.end == vertex {
            e.end = to;
        } else {
            return None;
        }
    }
    // Into the loops, after the coedge coming in (several sectors may share
    // a loop only if a face came round twice, which is refused).
    for (loop_, incoming, chain) in inserts {
        let coedges = &mut made.loops.get_mut(loop_.0 as usize)?.coedges;
        let at = incoming + 1;
        if at > coedges.len() {
            return None;
        }
        for (i, c) in chain.into_iter().enumerate() {
            coedges.insert(at + i, c);
        }
    }
    Some(split)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::{in_front, triangulations};
    use craft3d_geom::{NurbsSurface, Surface};
    use craft3d_math::Vec3;
    use craft3d_topo::{Body, BodyKind, FaceId};

    /// A body of one face on the bilinear spline patch over the unit
    /// square in the plane z = 0 (its normal +z), facing that way or not.
    fn patch(reversed: bool) -> Body {
        let ctrl = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ];
        let surface = NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            2,
            ctrl,
            vec![1.0; 4],
        )
        .unwrap();
        let mut body = Body::new(BodyKind::Solid);
        body.add_face(Surface::Nurbs(surface), reversed, Vec::new());
        body
    }

    /// Square above or below the patch the side is told, signed by the
    /// face's own outside; beyond its border, where the nearest point is on
    /// the border and not square below, it cannot be told (the review of
    /// 2026-11-14: the unsigned length there, its sign flipped for a
    /// reversed face, passed a corner beyond the border as behind it); on
    /// the surface within the tolerance it is on it.
    #[test]
    fn a_side_beyond_a_border_cannot_be_told() {
        let tol = 1e-6;
        for reversed in [false, true] {
            let body = patch(reversed);
            let sign = if reversed { -1.0 } else { 1.0 };
            let side = |p: Vec3| in_front(&body, FaceId(0), p, tol);
            let above = side(Vec3::new(0.5, 0.25, 2.0)).unwrap();
            assert!((above - 2.0 * sign).abs() <= 1e-12, "{above}");
            let below = side(Vec3::new(0.25, 0.5, -0.5)).unwrap();
            assert!((below + 0.5 * sign).abs() <= 1e-12, "{below}");
            assert_eq!(side(Vec3::new(0.3, 0.3, 1e-8)), Some(0.0));
            for beyond in [
                Vec3::new(2.0, 0.5, 1.0),
                Vec3::new(2.0, 0.5, -1.0),
                Vec3::new(-0.5, 1.5, 0.0),
                Vec3::new(1.5, 1.5, 0.3),
            ] {
                assert_eq!(side(beyond), None, "{beyond:?}, reversed {reversed}");
            }
        }
    }

    /// Catalan's numbers, each triangulation n − 2 triangles that cover
    /// every side once.
    #[test]
    fn the_trees_round_a_vertex_are_the_triangulations() {
        for (n, count) in [(4, 2), (5, 5), (6, 14), (7, 42), (8, 132)] {
            let all = triangulations(n);
            assert_eq!(all.len(), count, "{n}");
            for t in &all {
                assert_eq!(t.len(), n - 2);
                for k in 0..n {
                    let side = |a: usize, b: usize| {
                        t.iter()
                            .filter(|tri| tri.contains(&a) && tri.contains(&b))
                            .count()
                    };
                    assert_eq!(side(k, (k + 1) % n), 1, "side {k} of {n}");
                }
            }
        }
    }
}
