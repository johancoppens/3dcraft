//! The folded sheet as a body: one planar face per facet, one straight
//! edge per crease piece shared by the two facets it joins, one per side
//! of a cut and one per piece of the boundary.
//!
//! Round a vertex of the arrangement the facets meet in *fans*: runs of
//! facets joined by fold creases, ended by the boundary or a cut. Each fan
//! is one vertex of the body (an interior vertex is one fan all round; a
//! cut that ends on the boundary gives the boundary vertex two), so the
//! facets of different pieces of a cut sheet never share a vertex and the
//! boundary passes each vertex once. Faces, edges and vertices are named
//! as a fill's (`extrude::fill`): a facet as a start cap after the curves
//! of its outer boundary, an edge after its curve and piece (a cut's two
//! sides are pieces `2 i` and `2 i + 1`), a vertex after the curves that
//! meet there and its fan.

use super::{Arrangement, FoldError, Folded, lift};
use crate::extrude::{OpResult, Origin, Origins, Role};
use craft3d_geom::{Curve3, Line, Plane, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, VertexId};
use std::collections::BTreeMap;

fn build_error(reason: &str) -> FoldError {
    FoldError::Build(reason.to_string())
}

/// Builds the folded sheet in the plane `plane` (the sketch's): its x and
/// y are the arrangement's coordinates, its z the side valleys fold to.
pub fn folded_sheet(
    arr: &Arrangement,
    folded: &Folded,
    plane: &Frame,
    tolerance: &Tolerance,
) -> Result<OpResult, FoldError> {
    let nv = arr.vertices.len();
    if folded.transforms.len() != arr.facets.len() {
        return Err(build_error("the fold does not match the facets"));
    }
    // The fan of every wedge round every vertex (wedge k lies between the
    // half-edges k and k + 1 that leave the vertex).
    let mut fan_of: Vec<Vec<usize>> = Vec::with_capacity(nv);
    for v in 0..nv {
        let around = arr.around.get(v).map(Vec::as_slice).unwrap_or(&[]);
        let deg = around.len();
        let separates = |k: usize| around.get(k).is_some_and(|&h| arr.separates(h / 2));
        let start = (0..deg).find(|&k| separates(k)).unwrap_or(0);
        let mut fans = vec![0usize; deg];
        let mut fan = 0usize;
        for step in 0..deg {
            let k = (start + step) % deg;
            if step > 0 && separates(k) {
                fan += 1;
            }
            if let Some(slot) = fans.get_mut(k) {
                *slot = fan;
            }
        }
        fan_of.push(fans);
    }
    let corner = |v: usize, h: usize| -> Option<(usize, usize)> {
        let k = *arr.position.get(h)?;
        Some((v, *fan_of.get(v)?.get(k)?))
    };

    let mut body = Body::new(BodyKind::Sheet);
    let mut origins = Origins::default();
    let world = |p: Vec3| plane.point(p.x, p.y, p.z);

    // Vertices, made as the facets need them.
    let mut vertex_ids: BTreeMap<(usize, usize), VertexId> = BTreeMap::new();
    let mut vertex_at = |body: &mut Body,
                         origins: &mut Origins,
                         (v, fan): (usize, usize),
                         facet: usize|
     -> Result<VertexId, FoldError> {
        if let Some(&id) = vertex_ids.get(&(v, fan)) {
            return Ok(id);
        }
        let p = arr
            .point(v)
            .ok_or_else(|| build_error("a vertex is missing"))?;
        let t = folded
            .transforms
            .get(facet)
            .ok_or_else(|| build_error("a facet has no place"))?;
        let mut sources: Vec<u64> = arr
            .around
            .get(v)
            .into_iter()
            .flatten()
            .filter_map(|&h| arr.edges.get(h / 2).map(|e| e.source))
            .collect();
        sources.sort_unstable();
        sources.dedup();
        let piece = u32::try_from(fan).unwrap_or(u32::MAX);
        origins
            .vertices
            .push(Origin::of_piece(Role::StartVertex, &sources, piece));
        let id = body.add_vertex(world(t.point(lift(p))));
        vertex_ids.insert((v, fan), id);
        Ok(id)
    };

    // Edges: per half-edge with a facet on its left, the body's edge and
    // whether the facet's loop runs against it.
    let mut edge_of: BTreeMap<usize, (EdgeId, bool)> = BTreeMap::new();
    for (e, edge) in arr.edges.iter().enumerate() {
        let joined = !arr.separates(e);
        for side in 0..2usize {
            let h = 2 * e + side;
            let Some(&Some(f)) = arr.face_of.get(h) else {
                continue;
            };
            if joined && side == 1 && edge_of.contains_key(&(2 * e)) {
                if let Some(&(id, _)) = edge_of.get(&(2 * e)) {
                    edge_of.insert(h, (id, true));
                }
                continue;
            }
            let (Some(tail), Some(head), Some(&next)) = (arr.tail(h), arr.head(h), arr.next.get(h))
            else {
                return Err(build_error("a half-edge is missing"));
            };
            let a = corner(tail, h).ok_or_else(|| build_error("a corner is missing"))?;
            let b = corner(head, next).ok_or_else(|| build_error("a corner is missing"))?;
            let va = vertex_at(&mut body, &mut origins, a, f)?;
            let vb = vertex_at(&mut body, &mut origins, b, f)?;
            let at = |id: VertexId| {
                usize::try_from(id.0)
                    .ok()
                    .and_then(|i| body.vertices.get(i))
            };
            let (Some(pa), Some(pb)) = (at(va), at(vb)) else {
                return Err(build_error("a vertex is missing"));
            };
            let (pa, pb) = (pa.point, pb.point);
            let line = Line::through(pa, pb).map_err(|e| FoldError::Build(e.to_string()))?;
            let piece = if !joined && matches!(edge.kind, super::EdgeKind::Crease(_)) {
                edge.piece
                    .saturating_mul(2)
                    .saturating_add(u32::try_from(side).unwrap_or(1))
            } else {
                edge.piece
            };
            origins
                .edges
                .push(Origin::of_piece(Role::StartEdge, &[edge.source], piece));
            let id = body.add_edge(Curve3::Line(line), (0.0, pa.distance(pb)), va, vb);
            edge_of.insert(h, (id, false));
        }
    }

    // Faces, named after the curves of their outer boundary.
    let sources_of = |cycle: &[usize]| {
        let mut s: Vec<u64> = cycle
            .iter()
            .filter_map(|&h| arr.edges.get(h / 2).map(|e| e.source))
            .collect();
        s.sort_unstable();
        s.dedup();
        s
    };
    let mut used: BTreeMap<Vec<u64>, u32> = BTreeMap::new();
    let mut faces = Vec::with_capacity(arr.facets.len());
    for (f, facet) in arr.facets.iter().enumerate() {
        let t = folded
            .transforms
            .get(f)
            .ok_or_else(|| build_error("a facet has no place"))?;
        let mut loops = Vec::with_capacity(1 + facet.holes.len());
        for cycle in std::iter::once(&facet.outer).chain(&facet.holes) {
            let mut coedges = Vec::with_capacity(cycle.len());
            for &h in cycle {
                let &(edge, reversed) = edge_of
                    .get(&h)
                    .ok_or_else(|| build_error("a facet's edge was not made"))?;
                coedges.push(Coedge { edge, reversed });
            }
            loops.push(body.add_loop(coedges));
        }
        let origin = world(t.point(Vec3::ZERO));
        let normal = {
            let n = t.vector(Vec3::Z);
            plane.vector(n.x, n.y, n.z)
        };
        let x = {
            let v = t.vector(Vec3::X);
            plane.vector(v.x, v.y, v.z)
        };
        let frame = Frame::new(origin, normal, x)
            .ok_or_else(|| build_error("a facet's plane is not finite"))?;
        let sources = sources_of(&facet.outer);
        let piece = used.entry(sources.clone()).or_insert(0);
        origins
            .faces
            .push(Origin::of_piece(Role::StartCap, &sources, *piece));
        *piece = piece.saturating_add(1);
        faces.push(body.add_face(Surface::Plane(Plane::new(frame)), false, loops));
    }

    // One shell per piece of the sheet, as the fold's forest found them.
    let mut pieces: Vec<Vec<usize>> = vec![Vec::new(); folded.roots.len()];
    for (f, &piece) in folded.piece.iter().enumerate() {
        pieces
            .get_mut(piece)
            .ok_or_else(|| build_error("a facet belongs to no piece"))?
            .push(f);
    }
    for members in pieces {
        body.add_shell(
            members
                .iter()
                .filter_map(|&f| faces.get(f).copied())
                .collect(),
        );
    }

    let mut defects = craft3d_topo::check_remembered(&body, tolerance);
    if !defects.is_empty() {
        defects.truncate(crate::MAX_REPORTED);
        let text: Vec<String> = defects.iter().map(ToString::to_string).collect();
        return Err(FoldError::Build(format!(
            "the folded sheet fails the validity check: {}",
            text.join("; ")
        )));
    }
    Ok(OpResult { body, origins })
}
