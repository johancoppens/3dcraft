//! The shells of a body one by one: a shell as a body of its own, the
//! groups of faces that hang together through edges, and the counts of
//! Euler–Poincaré per shell.
//!
//! A solid is a set of closed shells. Which of them bound a lump from
//! outside and which a cavity in one is geometry (the sign of the volume
//! a shell encloses, and which shells lie inside which), told by
//! `craft3d_kernel::lumps`; what is here needs only the topology.

use crate::{Body, Coedge, EdgeId, FaceId, ShellId, VertexId};
use std::collections::{BTreeMap, BTreeSet};

/// Shell `shell` of `body` as a body of its own (of the same kind): its
/// faces, and only the loops, edges and vertices they use, in the order
/// they are first used. `None` for a shell, face, loop, edge or vertex
/// that does not exist.
pub fn shell_alone(body: &Body, shell: ShellId) -> Option<Body> {
    shells_alone(body, &[shell]).map(|(body, _)| body)
}

/// Which entity of the body a shell was taken from each entity of a body
/// of some of its shells is ([`shells_alone`]): `faces[i]` is the face of
/// the whole that face `i` of the part is, and so on.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Taken {
    pub faces: Vec<FaceId>,
    pub edges: Vec<EdgeId>,
    pub vertices: Vec<VertexId>,
}

/// The shells `shells` of `body`, in that order, as a body of their own
/// (of the same kind): their faces, and only the loops, edges and
/// vertices they use, in the order they are first used, with which
/// entity of `body` each entity of the part is. A shell listed twice is
/// taken once. `None` for a shell, face, loop, edge or vertex that does
/// not exist. Shells that share an edge (which `check` refuses of a
/// solid) share it in the part too.
pub fn shells_alone(body: &Body, shells: &[ShellId]) -> Option<(Body, Taken)> {
    let mut out = Body::new(body.kind);
    let mut taken = Taken::default();
    let mut vertices: BTreeMap<u32, VertexId> = BTreeMap::new();
    let mut edges: BTreeMap<u32, EdgeId> = BTreeMap::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for shell in shells {
        if !seen.insert(shell.0) {
            continue;
        }
        let mut faces = Vec::new();
        for &face_id in &body.shells.get(shell.0 as usize)?.faces {
            let face = body.face(face_id)?;
            let mut loops = Vec::with_capacity(face.loops.len());
            for &loop_id in &face.loops {
                let mut coedges = Vec::new();
                for coedge in &body.loop_(loop_id)?.coedges {
                    let edge = match edges.get(&coedge.edge.0) {
                        Some(&edge) => edge,
                        None => {
                            let old = body.edge(coedge.edge)?;
                            let mut ends = [old.start, old.end];
                            for end in &mut ends {
                                *end = match vertices.get(&end.0) {
                                    Some(&vertex) => vertex,
                                    None => {
                                        let new = out.add_vertex(body.vertex(*end)?.point);
                                        vertices.insert(end.0, new);
                                        taken.vertices.push(*end);
                                        new
                                    }
                                };
                            }
                            let [start, end] = ends;
                            let new = out.add_edge(old.curve.clone(), old.range, start, end);
                            edges.insert(coedge.edge.0, new);
                            taken.edges.push(coedge.edge);
                            new
                        }
                    };
                    coedges.push(Coedge {
                        edge,
                        reversed: coedge.reversed,
                    });
                }
                loops.push(out.add_loop(coedges));
            }
            faces.push(out.add_face(face.surface.clone(), face.reversed, loops));
            taken.faces.push(face_id);
        }
        out.add_shell(faces);
    }
    Some((out, taken))
}

/// The faces of `body` in groups that hang together through edges (two
/// faces whose loops use one edge are in one group), each group in the
/// order of its faces and the groups in the order of their first face.
/// A reference that does not exist is skipped: `check` reports it.
pub fn face_groups(body: &Body) -> Vec<Vec<FaceId>> {
    let n = body.faces.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        // Halving keeps the paths short; every step moves towards a root,
        // so this ends after at most `parent.len()` steps.
        for _ in 0..parent.len() {
            let Some(&up) = parent.get(i) else {
                return i;
            };
            if up == i {
                return i;
            }
            let grand = parent.get(up).copied().unwrap_or(up);
            if let Some(slot) = parent.get_mut(i) {
                *slot = grand;
            }
            i = grand;
        }
        i
    }
    let mut first_face: BTreeMap<u32, usize> = BTreeMap::new();
    for (f, face) in body.faces.iter().enumerate() {
        for loop_id in &face.loops {
            let Some(loop_) = body.loop_(*loop_id) else {
                continue;
            };
            for coedge in &loop_.coedges {
                let other = *first_face.entry(coedge.edge.0).or_insert(f);
                let (a, b) = (root(&mut parent, f), root(&mut parent, other));
                if a != b {
                    let (low, high) = (a.min(b), a.max(b));
                    if let Some(slot) = parent.get_mut(high) {
                        *slot = low;
                    }
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<FaceId>> = BTreeMap::new();
    for f in 0..n {
        let r = root(&mut parent, f);
        groups
            .entry(r)
            .or_default()
            .push(FaceId(u32::try_from(f).unwrap_or(u32::MAX)));
    }
    groups.into_values().collect()
}

/// Euler–Poincaré counts of one shell: `(V, E, F, R, H)`, its vertices,
/// edges and faces, the loops beyond the first of each face (a face
/// without loops counts −1), and its faces that keep a torus's handle.
/// `None` when a reference does not exist.
pub(crate) fn shell_counts(body: &Body, shell: ShellId) -> Option<(usize, usize, usize, i64, i64)> {
    let mut vertices = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut rings = 0_i64;
    let mut handles = 0_i64;
    let faces = &body.shells.get(shell.0 as usize)?.faces;
    for &face_id in faces {
        let face = body.face(face_id)?;
        rings += i64::try_from(face.loops.len()).ok()? - 1;
        if crate::check::keeps_handle(body, face) {
            handles += 1;
        }
        for &loop_id in &face.loops {
            for coedge in &body.loop_(loop_id)?.coedges {
                if edges.insert(coedge.edge.0) {
                    let edge = body.edge(coedge.edge)?;
                    vertices.insert(edge.start.0);
                    vertices.insert(edge.end.0);
                }
            }
        }
    }
    Some((vertices.len(), edges.len(), faces.len(), rings, handles))
}

/// How many loops the boundary edges of shell `shell` make: the edges its
/// faces use once, joined where they share a vertex (a sheet's boundary,
/// where no more than two boundary edges end at a vertex, falls into
/// closed loops; 0 for a closed shell). `None` when a reference does not
/// exist.
pub fn boundary_loops(body: &Body, shell: ShellId) -> Option<usize> {
    let mut uses: BTreeMap<u32, usize> = BTreeMap::new();
    for &face_id in &body.shells.get(shell.0 as usize)?.faces {
        for &loop_id in &body.face(face_id)?.loops {
            for coedge in &body.loop_(loop_id)?.coedges {
                *uses.entry(coedge.edge.0).or_insert(0) += 1;
            }
        }
    }
    // Union-find over the vertices the boundary edges join.
    let mut index: BTreeMap<u32, usize> = BTreeMap::new();
    let mut parent: Vec<usize> = Vec::new();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        for _ in 0..parent.len() {
            let Some(&up) = parent.get(i) else {
                return i;
            };
            if up == i {
                return i;
            }
            let grand = parent.get(up).copied().unwrap_or(up);
            if let Some(slot) = parent.get_mut(i) {
                *slot = grand;
            }
            i = grand;
        }
        i
    }
    for (&edge, _) in uses.iter().filter(|&(_, &n)| n == 1) {
        let e = body.edge(EdgeId(edge))?;
        let mut at = [0_usize; 2];
        for (slot, vertex) in at.iter_mut().zip([e.start.0, e.end.0]) {
            let next = parent.len();
            *slot = *index.entry(vertex).or_insert(next);
            if *slot == next {
                parent.push(next);
            }
        }
        let (a, b) = (root(&mut parent, at[0]), root(&mut parent, at[1]));
        if a != b
            && let Some(slot) = parent.get_mut(a.max(b))
        {
            *slot = a.min(b);
        }
    }
    let roots: BTreeSet<usize> = (0..parent.len()).map(|i| root(&mut parent, i)).collect();
    Some(roots.len())
}

/// The genus of each shell of a solid (its handles: 0 for a ball or a
/// cavity like one, 1 for a ring), from Euler–Poincaré per shell,
/// `V − E + F − R − 2H = 2 (1 − G)`. `None` when a shell's counts fit no
/// whole `G ≥ 0`, or a reference does not exist.
pub fn shell_genera(body: &Body) -> Option<Vec<u32>> {
    (0..body.shells.len())
        .map(|s| {
            let shell = ShellId(u32::try_from(s).ok()?);
            let (v, e, f, rings, handles) = shell_counts(body, shell)?;
            let chi = i64::try_from(v).ok()? - i64::try_from(e).ok()? + i64::try_from(f).ok()?
                - rings
                - 2 * handles;
            let twice = 2 - chi;
            (twice >= 0 && twice % 2 == 0)
                .then(|| u32::try_from(twice / 2).ok())
                .flatten()
        })
        .collect()
}
