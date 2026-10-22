//! Loose bodies for the tests of tolerant modelling: faces that share
//! nothing, as a file from another modeller has them, and T-junctions
//! (one face's whole side against the pieces its neighbours have).

use craft3d_geom::{Curve3, Line, Plane, Surface};
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, VertexId};
use std::collections::BTreeMap;

/// `sources[0]` as loose faces, each face with its own copies of its
/// edges and vertices (a seam the face uses twice stays one edge of it),
/// the shells holding the same faces; face `f` is taken from
/// `sources[1]` where `second(f)` (two bodies with the same faces, the
/// second with some edges whole where the first has them in pieces: a
/// T-junction), moved rigidly by `moves[f]` (none past the end), and
/// `nudge` moves one vertex of a face (the face, its k-th vertex, by).
/// `None` when the sources do not have the same faces or refer to
/// entities they do not have.
pub fn exploded_from(
    sources: [&Body; 2],
    second: impl Fn(usize) -> bool,
    moves: &[Vec3],
    nudge: Option<(usize, usize, Vec3)>,
) -> Option<Body> {
    let [first, other] = sources;
    if first.faces.len() != other.faces.len() {
        return None;
    }
    let mut out = Body::new(first.kind);
    for f in 0..first.faces.len() {
        let body = if second(f) { other } else { first };
        let face = body.faces.get(f)?;
        let t = Transform::translation(moves.get(f).copied().unwrap_or(Vec3::ZERO));
        let mut vertices: BTreeMap<VertexId, VertexId> = BTreeMap::new();
        let mut edges: BTreeMap<EdgeId, EdgeId> = BTreeMap::new();
        let mut loops = Vec::new();
        for l in &face.loops {
            let mut coedges = Vec::new();
            for c in &body.loop_(*l)?.coedges {
                let edge = match edges.get(&c.edge) {
                    Some(e) => *e,
                    None => {
                        let e = body.edge(c.edge)?;
                        let mut vertex = |v: VertexId| -> Option<VertexId> {
                            if let Some(n) = vertices.get(&v) {
                                return Some(*n);
                            }
                            let n = out.add_vertex(t.point(body.vertex(v)?.point));
                            vertices.insert(v, n);
                            Some(n)
                        };
                        let (a, b) = (vertex(e.start)?, vertex(e.end)?);
                        let ne = out.add_edge(e.curve.transformed(&t).ok()?, e.range, a, b);
                        edges.insert(c.edge, ne);
                        ne
                    }
                };
                coedges.push(Coedge {
                    edge,
                    reversed: c.reversed,
                });
            }
            loops.push(out.add_loop(coedges));
        }
        if let Some((nf, k, by)) = nudge
            && nf == f
            && let Some(v) = vertices.values().nth(k % vertices.len().max(1))
            && let Some(slot) = out.vertices.get_mut(v.0 as usize)
        {
            slot.point += by;
        }
        out.add_face(face.surface.transformed(&t).ok()?, face.reversed, loops);
    }
    for shell in &first.shells {
        out.add_shell(shell.faces.clone());
    }
    Some(out)
}

/// The faces of [`split_top_block`] whose top sides are split.
pub const SPLIT_TOP_SIDES: [usize; 2] = [3, 4];

/// A 2 × 1 × 1 block (times `scale`, moved off the origin) whose top is
/// two faces, split across x = 1: the front (face 3) and the back (face
/// 4) each have their top side in two pieces where the split meets them
/// when `split` (a valid body), or whole (not a valid body: the faces of
/// a file that leaves the T-junctions in, to be taken by
/// [`exploded_from`]). `None` for a scale that gives no frame.
pub fn split_top_block(scale: f64, split: bool) -> Option<Body> {
    let p = [
        (0.0, 0.0, 0.0),
        (2.0, 0.0, 0.0),
        (2.0, 1.0, 0.0),
        (0.0, 1.0, 0.0),
        (0.0, 0.0, 1.0),
        (2.0, 0.0, 1.0),
        (2.0, 1.0, 1.0),
        (0.0, 1.0, 1.0),
        (1.0, 0.0, 1.0),
        (1.0, 1.0, 1.0),
    ]
    .map(|(x, y, z)| (Vec3::new(x, y, z) + Vec3::new(0.3, -0.2, 0.1)) * scale);
    let faces: [&[usize]; 7] = [
        &[0, 3, 2, 1],
        &[4, 8, 9, 7],
        &[8, 5, 6, 9],
        if split {
            &[0, 1, 5, 8, 4]
        } else {
            &[0, 1, 5, 4]
        },
        if split {
            &[2, 3, 7, 9, 6]
        } else {
            &[2, 3, 7, 6]
        },
        &[0, 4, 7, 3],
        &[1, 2, 6, 5],
    ];
    let mut body = Body::new(BodyKind::Solid);
    let v: Vec<VertexId> = p.iter().map(|x| body.add_vertex(*x)).collect();
    let mut edges: BTreeMap<(usize, usize), EdgeId> = BTreeMap::new();
    let mut ids = Vec::new();
    for poly in faces {
        let mut coedges = Vec::new();
        for i in 0..poly.len() {
            let (a, b) = (*poly.get(i)?, *poly.get((i + 1) % poly.len())?);
            let key = (a.min(b), a.max(b));
            let edge = match edges.get(&key) {
                Some(e) => *e,
                None => {
                    let (s, e) = (*p.get(key.0)?, *p.get(key.1)?);
                    let id = body.add_edge(
                        Curve3::Line(Line::through(s, e).ok()?),
                        (0.0, s.distance(e)),
                        *v.get(key.0)?,
                        *v.get(key.1)?,
                    );
                    edges.insert(key, id);
                    id
                }
            };
            coedges.push(Coedge {
                edge,
                reversed: a > b,
            });
        }
        let l = body.add_loop(coedges);
        let (o, x, y) = (
            *p.get(*poly.first()?)?,
            *p.get(*poly.get(1)?)?,
            *p.get(*poly.get(2)?)?,
        );
        let normal = (x - o).cross(y - x).normalized()?;
        let frame = Frame::new(o, normal, x - o)?;
        ids.push(body.add_face(Surface::Plane(Plane::new(frame)), false, vec![l]));
    }
    body.add_shell(ids);
    Some(body)
}
