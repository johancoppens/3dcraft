//! Undoing what a boolean splits needlessly.
//!
//! **Faces.** Two faces of the result that share an edge are one face when
//! they lie on one surface, face the same way, and are either pieces of
//! one face of an operand or pieces of a face of A and a face of B that
//! are coincident. Faces joined like that, directly or through others,
//! form a group; every edge with the group on both sides goes, and the
//! group's loops are spliced together where it was. The face that remains
//! is the first of the group (the lowest-numbered face of A in it, or
//! else of B) and has its surface and its source.
//!
//! **Vertices.** A vertex left with two edge ends, of two edges that
//! continue each other along one curve, is not needed: the edges become
//! one, with the lesser of their sources. The exception is a vertex that
//! its operand already had like that (between two edges of that operand
//! along one curve): the operand's own design is not tidied up.
//!
//! Nothing here looks at positions to decide what is connected: faces,
//! loops and edges are spliced by their uses of each other, and positions
//! only confirm that two edges continue each other.

use super::outline::{lift, nearest};
use super::sew::{Origin, Work, WorkEdge, root, unite};
use super::{EdgeSource, LOST, Operand, Stop};
use craft3d_geom::{Curve3, Surface};
use craft3d_topo::Body;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

/// How far, in tolerances, an edge may be from the curve of the edge it
/// is joined to.
const SAME_CURVE: f64 = 1e-2;

fn closed_surface(surface: &Surface) -> bool {
    matches!(surface, Surface::Sphere(_) | Surface::Torus(_))
}

/// The two uses of each edge: `(face, loop, position)`.
fn uses(work: &Work) -> BTreeMap<usize, Vec<(usize, usize, usize)>> {
    let mut found: BTreeMap<usize, Vec<(usize, usize, usize)>> = BTreeMap::new();
    for (face_index, face) in work.faces.iter().enumerate() {
        let Some(face) = face else { continue };
        for (loop_index, loop_) in face.loops.iter().enumerate() {
            for (at, &(edge, _)) in loop_.iter().enumerate() {
                found
                    .entry(edge)
                    .or_default()
                    .push((face_index, loop_index, at));
            }
        }
    }
    found
}

fn merge_faces(
    work: &mut Work,
    coincident: &BTreeMap<usize, Vec<(usize, bool)>>,
) -> Result<(), Stop> {
    let mut groups: Vec<usize> = (0..work.faces.len()).collect();
    let face = |index: usize| work.faces.get(index).and_then(Option::as_ref);
    for sides in uses(work).values() {
        let &[(f, ..), (g, ..)] = sides.as_slice() else {
            continue;
        };
        let (Some(first), Some(second)) = (face(f), face(g)) else {
            continue;
        };
        if f == g {
            continue;
        }
        let one = if first.source == second.source {
            // Pieces of one face lie on its surface and were turned over
            // together or not at all.
            first.reversed == second.reversed
        } else if first.source.operand != second.source.operand {
            let (of_a, of_b) = if first.source.operand == Operand::A {
                (first, second)
            } else {
                (second, first)
            };
            coincident
                .get(&(of_a.source.face.0 as usize))
                .into_iter()
                .flatten()
                .any(|&(other, same_normal)| {
                    other == of_b.source.face.0 as usize
                        && same_normal == (of_a.reversed == of_b.reversed)
                })
        } else {
            false
        };
        if one {
            unite(&mut groups, f, g);
        }
    }

    // Per group its members; the first is the one that stays.
    let mut members: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..work.faces.len() {
        if work.faces.get(index).is_some_and(Option::is_some) {
            members
                .entry(root(&mut groups, index))
                .or_default()
                .push(index);
        }
    }
    for (_, group) in members {
        let [stays, rest @ ..] = group.as_slice() else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        // The lowest-numbered face of A, else of B: faces are in that
        // order already, and pieces of one face are together.
        let mut loops = Vec::new();
        let mut contributors = Vec::new();
        let mut source = None;
        // Per edge the faces of the group that use it. An edge between
        // two of them has the merged face on both sides and goes; an edge
        // that one face uses twice (a seam of the operand) stays.
        let mut users: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for &index in &group {
            let Some(face) = work.faces.get_mut(index).ok_or(Stop::of(LOST))? else {
                return Err(Stop::of(LOST));
            };
            for &(edge, _) in face.loops.iter().flatten() {
                users.entry(edge).or_default().insert(index);
            }
            loops.append(&mut face.loops);
            contributors.append(&mut face.contributors);
            source = Some(source.map_or(face.source, |least: super::FaceSource| {
                least.min(face.source)
            }));
        }
        for (edge, _) in users.into_iter().filter(|(_, faces)| faces.len() == 2) {
            splice_out(&mut loops, edge)?;
            if let Some(slot) = work.edges.get_mut(edge) {
                *slot = None;
            }
        }
        for &index in rest {
            if let Some(slot) = work.faces.get_mut(index) {
                *slot = None;
            }
        }
        let Some(face) = work.faces.get_mut(*stays).ok_or(Stop::of(LOST))? else {
            return Err(Stop::of(LOST));
        };
        if loops.is_empty() && !closed_surface(&face.surface) {
            return Err(Stop::of(LOST));
        }
        face.loops = loops;
        contributors.sort();
        contributors.dedup();
        face.contributors = contributors;
        if let Some(source) = source {
            face.source = source;
        }
    }
    Ok(())
}

/// Removes an edge that the loops use twice, and joins the loops where it
/// was: two loops become one, or one loop falls into two.
fn splice_out(loops: &mut Vec<Vec<(usize, bool)>>, edge: usize) -> Result<(), Stop> {
    let mut places = Vec::with_capacity(2);
    for (loop_index, loop_) in loops.iter().enumerate() {
        for (at, &(used, _)) in loop_.iter().enumerate() {
            if used == edge {
                places.push((loop_index, at));
            }
        }
    }
    let &[(first_loop, first_at), (second_loop, second_at)] = places.as_slice() else {
        return Err(Stop::of(LOST));
    };
    if first_loop == second_loop {
        // [x, e, y, e', z] falls into [x, z] and [y].
        let loop_ = loops.get_mut(first_loop).ok_or(Stop::of(LOST))?;
        let tail: Vec<(usize, bool)> = loop_.drain(second_at..).skip(1).collect();
        let middle: Vec<(usize, bool)> = loop_.drain(first_at..).skip(1).collect();
        loop_.extend(tail);
        loops.push(middle);
    } else {
        // [a, e, b] and [c, e', d] become [a, d, c, b].
        let other = std::mem::take(loops.get_mut(second_loop).ok_or(Stop::of(LOST))?);
        let (before, after) = other.split_at(second_at.min(other.len()));
        let mut inserted: Vec<(usize, bool)> = after.iter().skip(1).copied().collect();
        inserted.extend_from_slice(before);
        let loop_ = loops.get_mut(first_loop).ok_or(Stop::of(LOST))?;
        loop_.splice(first_at..=first_at, inserted);
    }
    loops.retain(|loop_| !loop_.is_empty());
    Ok(())
}

/// The range of `first` extended over `second`, which continues it past
/// the vertex `at`, when the two run along one curve; and whether the
/// extension is at the start of `first`.
fn joined(
    first: &WorkEdge,
    second: &WorkEdge,
    at: usize,
    work: &Work,
    tol: f64,
) -> Option<(f64, f64)> {
    let at_end = first.end == at;
    let leaves = second.start == at;
    let far = if leaves { second.end } else { second.start };
    let far_point = work.vertices.get(far)?.point;
    let length = second.range.1 - second.range.0;
    // Where `first` is going at the vertex, away from itself, and where
    // `second` goes from there.
    let (edge_at, heading) = if at_end {
        (first.range.1, first.curve.derivatives(first.range.1).1)
    } else {
        (first.range.0, -first.curve.derivatives(first.range.0).1)
    };
    let onward = if leaves {
        second.curve.derivatives(second.range.0).1
    } else {
        -second.curve.derivatives(second.range.1).1
    };
    if heading.dot(onward) <= 0.99 * heading.length() * onward.length() {
        return None;
    }
    let step = if at_end { 1.0 } else { -1.0 };
    let reach = match &first.curve {
        Curve3::Line(_) => first.curve.project(far_point).ok()?,
        Curve3::Circle(_) | Curve3::Ellipse(_) => {
            // The parameter of the far end, on the turn the edge gets to.
            let expected = edge_at + step * length;
            let t = first.curve.project(far_point).ok()?;
            lift(&first.curve, (expected, expected), t)
        }
        Curve3::Nurbs(_) => {
            // Only the very same curve, where one edge stops and the
            // other starts.
            let meets = if at_end {
                second.range.0
            } else {
                second.range.1
            };
            let scale = (first.range.1 - first.range.0).abs() + length.abs();
            if first.curve != second.curve
                || leaves != at_end
                || (meets - edge_at).abs() > 1e-9 * scale
            {
                return None;
            }
            edge_at + step * length
        }
    };
    let range = if at_end {
        (first.range.0, reach)
    } else {
        (reach, first.range.1)
    };
    let extension = if at_end {
        (edge_at, reach)
    } else {
        (reach, edge_at)
    };
    if !(reach.is_finite() && extension.1 > extension.0) {
        return None;
    }
    if first.curve.period().is_some() && range.1 - range.0 > TAU * (1.0 + 1e-12) {
        return None;
    }
    // The other edge really runs along this curve, and ends where it says.
    let near = SAME_CURVE * tol;
    if first.curve.eval(reach).distance(far_point) > near {
        return None;
    }
    for k in [0.25, 0.5, 0.75] {
        let p = second.curve.eval(second.range.0 + length * k);
        if nearest(&first.curve, extension, p).is_none_or(|(_, d)| d > near) {
            return None;
        }
    }
    Some(range)
}

fn merge_vertices(work: &mut Work, a: &Body, b: &Body, tol: f64) -> Result<(), Stop> {
    // How many edge ends each vertex of the operands has.
    let valence = |body: &Body| {
        let mut ends = vec![0_usize; body.vertices.len()];
        for edge in &body.edges {
            for vertex in [edge.start, edge.end] {
                if let Some(slot) = ends.get_mut(vertex.0 as usize) {
                    *slot += 1;
                }
            }
        }
        ends
    };
    let (ends_a, ends_b) = (valence(a), valence(b));
    for _ in 0..=work.vertices.len() {
        let mut around: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (index, edge) in work.edges.iter().enumerate() {
            let Some(edge) = edge else { continue };
            around.entry(edge.start).or_default().push(index);
            around.entry(edge.end).or_default().push(index);
        }
        let mut changed = false;
        // Edges that were joined or removed in this pass: what is known
        // about their ends is out of date.
        let mut touched: BTreeSet<usize> = BTreeSet::new();
        for (vertex, edges) in around {
            let &[first_index, second_index] = edges.as_slice() else {
                continue;
            };
            if first_index == second_index
                || touched.contains(&first_index)
                || touched.contains(&second_index)
            {
                continue;
            }
            let (Some(Some(first)), Some(Some(second)), Some(made)) = (
                work.edges.get(first_index),
                work.edges.get(second_index),
                work.vertices.get(vertex),
            ) else {
                return Err(Stop::of(LOST));
            };
            // The operand's own vertex between two of its own edges.
            if let Origin::Kept { operand, vertex } = made.origin {
                let own = |source: EdgeSource| matches!(source, EdgeSource::Kept { operand: o, .. } if o == operand);
                let ends = match operand {
                    Operand::A => &ends_a,
                    Operand::B => &ends_b,
                };
                if ends.get(vertex.0 as usize) == Some(&2)
                    && own(first.source)
                    && own(second.source)
                {
                    continue;
                }
            }
            let Some(range) = joined(first, second, vertex, work, tol) else {
                continue;
            };
            let at_end = first.end == vertex;
            let far = if second.start == vertex {
                second.end
            } else {
                second.start
            };
            let source = first.source.min(second.source);
            let Some(Some(first)) = work.edges.get_mut(first_index) else {
                return Err(Stop::of(LOST));
            };
            first.range = range;
            first.source = source;
            if at_end {
                first.end = far;
            } else {
                first.start = far;
            }
            if let Some(slot) = work.edges.get_mut(second_index) {
                *slot = None;
            }
            // Wherever the second edge was used, the first is next to it
            // and now covers both.
            for face in work.faces.iter_mut().flatten() {
                for loop_ in &mut face.loops {
                    loop_.retain(|&(edge, _)| edge != second_index);
                }
            }
            touched.insert(first_index);
            touched.insert(second_index);
            changed = true;
        }
        if !changed {
            return Ok(());
        }
    }
    Ok(())
}

pub(super) fn merge(
    work: &mut Work,
    coincident: &BTreeMap<usize, Vec<(usize, bool)>>,
    a: &Body,
    b: &Body,
    tol: f64,
) -> Result<(), Stop> {
    merge_faces(work, coincident)?;
    merge_vertices(work, a, b, tol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splicing_an_edge_out_joins_two_loops_or_parts_one() {
        // Two squares side by side, sharing edge 9: forwards in one,
        // backwards in the other.
        let mut loops = vec![
            vec![(0, false), (9, false), (1, false), (2, false)],
            vec![(3, false), (4, false), (5, false), (9, true)],
        ];
        splice_out(&mut loops, 9).unwrap();
        assert_eq!(
            loops,
            [vec![
                (0, false),
                (3, false),
                (4, false),
                (5, false),
                (1, false),
                (2, false)
            ]]
        );
        // A loop that goes out along a bridge, around a hole and back:
        // without the bridge the hole's rim is a loop of its own.
        let mut loops = vec![vec![
            (0, false),
            (9, false),
            (7, true),
            (9, true),
            (1, false),
        ]];
        splice_out(&mut loops, 9).unwrap();
        assert_eq!(loops, [vec![(0, false), (1, false)], vec![(7, true)]]);
        // A disc in a ring: the rim goes, and with it both loops.
        let mut loops = vec![vec![(0, false)], vec![(9, true)], vec![(9, false)]];
        splice_out(&mut loops, 9).unwrap();
        assert_eq!(loops, [vec![(0, false)]]);
        // An edge used once cannot be spliced out.
        assert!(splice_out(&mut loops, 0).is_err());
    }
}
