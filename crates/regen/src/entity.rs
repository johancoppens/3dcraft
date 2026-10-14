//! Names for the edges and vertices of a body the kernel put together
//! out of other bodies: a boolean, a pattern whose instances are joined.
//!
//! Such a body's faces have names (each is a face, or a piece of one, of
//! an operand). Its edges and vertices are named by those: an edge by the
//! faces it lies between, a vertex by the faces that meet in it
//! (`F7:between[a,b]`, `F7:corner[a,b,c]`). That is what an edge or a
//! corner *is* to someone who points at it, it needs nothing from the
//! kernel but the body, and it does not depend on the order in which the
//! kernel made the entities.
//!
//! Several entities can lie on the same faces (the two halves of an edge
//! a slot cut through, the two points where an edge enters and leaves a
//! hole). Then none of them gets the bare name, and each also says which
//! other faces are near it (`F7:between[a,b|c]`): the faces at the
//! vertices one edge away, or as many edges away (up to
//! [`MAX_NEAR_HOPS`]) as it takes to tell all of them apart. Entities
//! that even that does not tell apart (two concentric circles in which a
//! torus cuts a plane) are numbered in the order of where they lie in
//! space (`F7:between[a,b|c](1)`); such a number is the one part of a
//! name an edit can move to another entity, which docs/architecture.md
//! §4.1 says.
//!
//! An entity an operand already had also keeps the name the primitive or
//! the sweep that first made it gave it, as a copy and as an alias
//! (`F7:copy(0)/F1:vertex(3)`), when it is the only one of the result
//! that came from it.

use craft3d_doc::{FeatureId, MAX_NAME_DEPTH, MAX_NAME_FACES, PersistentName};
use craft3d_math::Vec3;
use craft3d_topo::Body;
use std::collections::{BTreeMap, BTreeSet};

/// How many edges away from an edge or a vertex the faces are looked at
/// that tell it from the others on the same faces.
pub(crate) const MAX_NEAR_HOPS: usize = 4;

/// Most vertices looked at around one edge or vertex for the faces near
/// it: a bound on the work in a body with very many entities on the same
/// faces.
const MAX_NEAR_VERTICES: usize = 4096;

/// The names of the edges and of the vertices of one body, by index.
pub(crate) struct ByFaces {
    pub edges: Vec<PersistentName>,
    pub vertices: Vec<PersistentName>,
}

/// One edge or vertex as it is told from the others.
struct Item {
    /// The faces it lies on, as ranks of their names, ascending; an edge
    /// has one per side, so a seam has its face twice.
    on: Vec<u32>,
    /// Its vertices: the vertex itself, or the ends of the edge.
    ends: Vec<u32>,
    /// Where it lies: the vertex, or the middle of the edge.
    at: Vec3,
}

/// The vertices of a body, as far as telling entities apart needs them.
struct Around {
    /// Per vertex, the ranks of the faces that meet in it.
    faces: Vec<BTreeSet<u32>>,
    /// Per vertex, the vertices at the other ends of its edges.
    next: Vec<Vec<u32>>,
}

impl Around {
    /// The faces at the vertices within `hops` edges of `item`'s own,
    /// without the faces it lies on: ascending, without repeats.
    fn near(&self, item: &Item, hops: usize) -> Vec<u32> {
        let mut seen: BTreeSet<u32> = item.ends.iter().copied().collect();
        let mut rim: Vec<u32> = item.ends.clone();
        for _ in 0..hops {
            let mut reached = Vec::new();
            for vertex in &rim {
                for other in self.next.get(*vertex as usize).into_iter().flatten() {
                    if seen.len() < MAX_NEAR_VERTICES && seen.insert(*other) {
                        reached.push(*other);
                    }
                }
            }
            rim = reached;
        }
        let mut faces = BTreeSet::new();
        for vertex in seen {
            if let Some(there) = self.faces.get(vertex as usize) {
                faces.extend(there.iter().copied().filter(|rank| !item.on.contains(rank)));
            }
        }
        // A name is kept one that can be written; entities that the cut
        // makes alike are told apart as any others are.
        faces
            .into_iter()
            .take(MAX_NAME_FACES.saturating_sub(item.on.len()))
            .collect()
    }
}

/// Sorts `items` (index and position) by where they lie: by x, those
/// with one x by y, those with one y by z. Two coordinates are one when
/// they are within `tolerance` of each other or joined by a row of such
/// steps, so rounding in the last digits cannot change the order; what
/// lies in one place keeps the order of its indices.
pub(crate) fn sort_in_space(items: &mut [(usize, Vec3)], tolerance: f64, axis: usize) {
    let coordinate = |p: Vec3| match axis {
        0 => p.x,
        1 => p.y,
        _ => p.z,
    };
    items.sort_by(|a, b| {
        coordinate(a.1)
            .total_cmp(&coordinate(b.1))
            .then(a.0.cmp(&b.0))
    });
    if axis >= 2 {
        return;
    }
    let mut from = 0;
    while from < items.len() {
        let mut to = from.saturating_add(1);
        while let (Some(before), Some(here)) = (items.get(to.saturating_sub(1)), items.get(to)) {
            // Not-a-number compares as apart: such a body does not pass
            // the check anyway.
            if (coordinate(here.1) - coordinate(before.1)).abs() <= tolerance {
                to = to.saturating_add(1);
            } else {
                break;
            }
        }
        if let Some(run) = items.get_mut(from..to)
            && run.len() > 1
        {
            sort_in_space(run, tolerance, axis.saturating_add(1));
        }
        from = to;
    }
}

/// Names `items`, each by what it lies on, with as much more as it takes
/// to tell it from the others. `reach` is how many edges away the faces
/// near an item are looked at first: none for an edge (the faces at its
/// own ends), one for a vertex.
fn named(
    items: &[Item],
    around: &Around,
    reach: usize,
    ranked: &[&PersistentName],
    tolerance: f64,
    make: impl Fn(Vec<PersistentName>, Vec<PersistentName>, Option<u64>) -> PersistentName,
) -> Vec<PersistentName> {
    let faces = |ranks: &[u32]| -> Vec<PersistentName> {
        ranks
            .iter()
            .filter_map(|rank| ranked.get(*rank as usize))
            .map(|name| (*name).clone())
            .collect()
    };
    let mut names: Vec<Option<PersistentName>> = vec![None; items.len()];
    let mut put = |index: usize, name: PersistentName| {
        if let Some(slot) = names.get_mut(index) {
            *slot = Some(name);
        }
    };
    let mut same_faces: BTreeMap<&[u32], Vec<usize>> = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        same_faces.entry(&item.on).or_default().push(index);
    }
    for (on, group) in same_faces {
        if let [only] = group.as_slice() {
            put(*only, make(faces(on), Vec::new(), None));
            continue;
        }
        // All of the group say what is near them equally far away: the
        // nearest distance that tells them all apart, or else the nearest
        // that tells most of them apart.
        let mut best: BTreeMap<Vec<u32>, Vec<(usize, Vec3)>> = BTreeMap::new();
        for hops in 0..MAX_NEAR_HOPS {
            let mut same_near: BTreeMap<Vec<u32>, Vec<(usize, Vec3)>> = BTreeMap::new();
            for index in &group {
                if let Some(item) = items.get(*index) {
                    same_near
                        .entry(around.near(item, reach.saturating_add(hops)))
                        .or_default()
                        .push((*index, item.at));
                }
            }
            let all_apart = same_near.len() == group.len();
            if same_near.len() > best.len() {
                best = same_near;
            }
            if all_apart {
                break;
            }
        }
        for (near, mut tied) in best {
            if let [(only, _)] = tied.as_slice() {
                put(*only, make(faces(on), faces(&near), None));
                continue;
            }
            sort_in_space(&mut tied, tolerance, 0);
            for (number, (index, _)) in tied.iter().enumerate() {
                put(*index, make(faces(on), faces(&near), Some(number as u64)));
            }
        }
    }
    // Every item was in one group; the fallback is never used.
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            name.unwrap_or_else(|| make(Vec::new(), Vec::new(), Some(index as u64)))
        })
        .collect()
}

/// Names every edge and every vertex of `body` by the faces it lies on:
/// `faces[i]` is the name of face `i`. No two edges get one name, and no
/// two vertices; the names do not depend on the order of the entities in
/// the body, except where two of them lie in one place within
/// `tolerance`.
pub(crate) fn by_faces(
    feature: FeatureId,
    body: &Body,
    faces: &[PersistentName],
    tolerance: f64,
) -> ByFaces {
    // Names are compared often and are long: each face gets the rank of
    // its name among the names, and equal names (the pieces of one face)
    // get one rank.
    let ranked: Vec<&PersistentName> = faces.iter().collect::<BTreeSet<_>>().into_iter().collect();
    let rank_of: BTreeMap<&PersistentName, u32> = ranked
        .iter()
        .enumerate()
        .filter_map(|(rank, name)| Some((*name, u32::try_from(rank).ok()?)))
        .collect();

    let mut sides: Vec<Vec<u32>> = vec![Vec::new(); body.edges.len()];
    let mut around = Around {
        faces: vec![BTreeSet::new(); body.vertices.len()],
        next: vec![Vec::new(); body.vertices.len()],
    };
    for (face, name) in body.faces.iter().zip(faces) {
        let Some(&rank) = rank_of.get(name) else {
            continue;
        };
        let coedges = face
            .loops
            .iter()
            .filter_map(|id| body.loop_(*id))
            .flat_map(|l| &l.coedges);
        for coedge in coedges {
            let Some(edge) = body.edge(coedge.edge) else {
                continue;
            };
            if let Some(list) = sides.get_mut(coedge.edge.0 as usize) {
                list.push(rank);
            }
            for vertex in [edge.start, edge.end] {
                if let Some(set) = around.faces.get_mut(vertex.0 as usize) {
                    set.insert(rank);
                }
            }
        }
    }
    for edge in &body.edges {
        if edge.start == edge.end {
            continue;
        }
        for (from, to) in [(edge.start, edge.end), (edge.end, edge.start)] {
            if let Some(list) = around.next.get_mut(from.0 as usize)
                && !list.contains(&to.0)
            {
                list.push(to.0);
            }
        }
    }

    let edges: Vec<Item> = body
        .edges
        .iter()
        .zip(sides)
        .map(|(edge, mut on)| {
            on.sort_unstable();
            on.truncate(MAX_NAME_FACES);
            let middle = edge.range.0 + (edge.range.1 - edge.range.0) * 0.5;
            let mut ends = vec![edge.start.0];
            if edge.end != edge.start {
                ends.push(edge.end.0);
            }
            Item {
                on,
                ends,
                at: edge.curve.eval(middle),
            }
        })
        .collect();
    let vertices: Vec<Item> = body
        .vertices
        .iter()
        .zip(&around.faces)
        .enumerate()
        .map(|(index, (vertex, on))| Item {
            on: on.iter().copied().take(MAX_NAME_FACES).collect(),
            ends: u32::try_from(index).into_iter().collect(),
            at: vertex.point,
        })
        .collect();
    ByFaces {
        edges: named(
            &edges,
            &around,
            0,
            &ranked,
            tolerance,
            |on, near, number| PersistentName::between(feature, on, near, number),
        ),
        vertices: named(
            &vertices,
            &around,
            1,
            &ranked,
            tolerance,
            |on, near, number| PersistentName::corner(feature, on, near, number),
        ),
    }
}

/// The names entities keep from the bodies they came from. `kept[i]` is,
/// for entity `i` of the result, the instance and the entity of that
/// instance's source it is (or is a piece of), when the kernel says so;
/// `names_there(instance, entity)` gives every name it answers to in that
/// source. An entity is given those of them that a primitive or a sweep
/// gave, as copies, when it is the only one that came from there: the two
/// pieces of an edge that was cut through are neither of them "the" edge.
pub(crate) fn kept_names<'a, I>(
    feature: FeatureId,
    kept: &[Option<(u64, usize)>],
    names_there: impl Fn(u64, usize) -> I,
) -> BTreeMap<usize, Vec<PersistentName>>
where
    I: Iterator<Item = &'a PersistentName>,
{
    let mut times: BTreeMap<(u64, usize), usize> = BTreeMap::new();
    for from in kept.iter().flatten() {
        let count = times.entry(*from).or_default();
        *count = count.saturating_add(1);
    }
    let mut aliases = BTreeMap::new();
    for (index, from) in kept.iter().enumerate() {
        let Some(from) = from else {
            continue;
        };
        if times.get(from) != Some(&1) {
            continue;
        }
        let (instance, entity) = *from;
        // The name it was first given, by a primitive or a sweep, is the
        // one worth keeping: what it was called by its faces in a body
        // in between says nothing its faces here do not say.
        let copies: Vec<PersistentName> = names_there(instance, entity)
            .filter(|name| !name.root().role.is_by_faces())
            .filter(|name| name.depth() < MAX_NAME_DEPTH)
            .map(|name| PersistentName::copy(feature, instance, name.clone()))
            .collect();
        if !copies.is_empty() {
            aliases.insert(index, copies);
        }
    }
    aliases
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(points: &[(f64, f64, f64)], tolerance: f64) -> Vec<usize> {
        let mut items: Vec<(usize, Vec3)> = points
            .iter()
            .enumerate()
            .map(|(i, p)| (i, Vec3::new(p.0, p.1, p.2)))
            .collect();
        sort_in_space(&mut items, tolerance, 0);
        items.into_iter().map(|(i, _)| i).collect()
    }

    #[test]
    fn the_order_in_space_is_by_x_then_y_then_z_and_ignores_rounding() {
        assert_eq!(
            order(&[(2.0, 0.0, 0.0), (1.0, 5.0, 0.0), (1.0, 4.0, 9.0)], 1e-6),
            [2, 1, 0]
        );
        // The x of the first is a rounding error above that of the second:
        // y decides, as it does when they are exactly equal.
        assert_eq!(
            order(&[(1.0 + 1e-12, 0.0, 0.0), (1.0, 1.0, 0.0)], 1e-6),
            [0, 1]
        );
        assert_eq!(order(&[(1.0, 0.0, 0.0), (1.0, 1.0, 0.0)], 1e-6), [0, 1]);
        assert_eq!(
            order(&[(1.0, 1.0, 0.0), (1.0 + 1e-12, 0.0, 0.0)], 1e-6),
            [1, 0]
        );
        // z decides last.
        assert_eq!(
            order(
                &[(0.0, 0.0, 1.0), (1e-9, 1e-9, -1.0), (0.0, 0.0, 0.0)],
                1e-6
            ),
            [1, 2, 0]
        );
        // The same points in any order of the list give the same order in
        // space.
        let points = [
            (0.0, 0.0, 0.0),
            (0.0, 1.0, 0.0),
            (1.0, 0.0, 0.0),
            (1.0, 0.0, 1.0),
            (0.0, 1.0, -3.0),
        ];
        let sorted: Vec<_> = order(&points, 1e-6).iter().map(|i| points[*i]).collect();
        let mut reversed = points;
        reversed.reverse();
        let again: Vec<_> = order(&reversed, 1e-6)
            .iter()
            .map(|i| reversed[*i])
            .collect();
        assert_eq!(sorted, again);
        // Not-a-number and infinities sort somewhere and do not panic.
        assert_eq!(
            order(
                &[
                    (f64::NAN, 0.0, 0.0),
                    (f64::INFINITY, 0.0, 0.0),
                    (0.0, f64::NAN, 0.0)
                ],
                1e-6
            )
            .len(),
            3
        );
        assert!(order(&[], 1e-6).is_empty());
    }

    use crate::{Names, Status, regenerate};
    use craft3d_doc::{
        Block, Boolean, BooleanOp, Cylinder, Document, Expr, FeatureKind, Orientation, Units,
    };
    use craft3d_topo::{Coedge, EdgeId, VertexId};

    fn exprs(values: [&str; 3]) -> [Expr; 3] {
        values.map(Expr::new)
    }

    /// A block with a round notch across one edge (entities that the
    /// faces further away tell apart), or a cylinder cut in half
    /// lengthwise (entities that only a number tells apart): the body of
    /// the boolean and its names.
    fn cut(halved: bool) -> (Body, Names) {
        let mut doc = Document::new(Units::default());
        let block = |position: [&str; 3], size: [&str; 3]| {
            FeatureKind::Block(Block {
                position: exprs(position),
                size: exprs(size),
                orientation: Orientation::default(),
            })
        };
        let cylinder = |position: [&str; 3], radius: &str, height: &str, axis: [&str; 3]| {
            FeatureKind::Cylinder(Cylinder {
                position: exprs(position),
                radius: Expr::new(radius),
                height: Expr::new(height),
                orientation: Orientation {
                    axis: exprs(axis),
                    angle: Expr::new("0"),
                },
            })
        };
        let (target, tool) = if halved {
            (
                cylinder(["0", "0", "0"], "5", "10", ["0", "0", "1"]),
                block(["1", "-10", "-1"], ["10", "20", "12"]),
            )
        } else {
            (
                block(["0", "0", "0"], ["40", "20", "5"]),
                cylinder(["20", "-1", "5"], "3", "22", ["0", "1", "0"]),
            )
        };
        let target = doc.add_feature(None, target).unwrap();
        let tool = doc.add_feature(None, tool).unwrap();
        let made = doc
            .add_feature(
                None,
                FeatureKind::Boolean(Boolean {
                    op: BooleanOp::Subtract,
                    target,
                    tool,
                    keep_tool: false,
                }),
            )
            .unwrap();
        let regen = regenerate(&doc);
        let outcome = regen.feature(made).unwrap();
        assert_eq!(outcome.status, Status::Ok);
        (
            (**outcome.body.as_ref().unwrap()).clone(),
            (**outcome.names.as_ref().unwrap()).clone(),
        )
    }

    /// The same body with its vertices, edges and faces in another order:
    /// entity `i` becomes entity `(i * step + shift) % count`, and every
    /// loop starts at another of its coedges.
    fn shuffled(body: &Body, faces: &[PersistentName]) -> (Body, Vec<PersistentName>, Shuffle) {
        let place = |count: usize, step: usize, shift: usize| -> Vec<usize> {
            // A step that has no factor in common with the count visits
            // every place once.
            let step = (step..).find(|s| gcd(*s, count.max(1)) == 1).unwrap();
            (0..count).map(|i| (i * step + shift) % count).collect()
        };
        let shuffle = Shuffle {
            vertices: place(body.vertices.len(), 3, 1),
            edges: place(body.edges.len(), 5, 2),
            faces: place(body.faces.len(), 2, 3),
        };
        let mut out = body.clone();
        for (old, new) in shuffle.vertices.iter().enumerate() {
            out.vertices[*new] = body.vertices[old].clone();
        }
        for (old, new) in shuffle.edges.iter().enumerate() {
            let mut edge = body.edges[old].clone();
            edge.start = VertexId(shuffle.vertices[edge.start.0 as usize] as u32);
            edge.end = VertexId(shuffle.vertices[edge.end.0 as usize] as u32);
            out.edges[*new] = edge;
        }
        for l in &mut out.loops {
            let turned: Vec<Coedge> = l
                .coedges
                .iter()
                .map(|c| Coedge {
                    edge: EdgeId(shuffle.edges[c.edge.0 as usize] as u32),
                    ..*c
                })
                .collect();
            let by = 1 % turned.len().max(1);
            l.coedges = turned[by..].iter().chain(&turned[..by]).copied().collect();
        }
        let mut names = faces.to_vec();
        for (old, new) in shuffle.faces.iter().enumerate() {
            out.faces[*new] = body.faces[old].clone();
            names[*new] = faces[old].clone();
        }
        (out, names, shuffle)
    }

    struct Shuffle {
        vertices: Vec<usize>,
        edges: Vec<usize>,
        faces: Vec<usize>,
    }

    fn gcd(a: usize, b: usize) -> usize {
        if b == 0 { a } else { gcd(b, a % b) }
    }

    #[test]
    fn names_by_faces_do_not_depend_on_the_order_of_the_entities() {
        for halved in [false, true] {
            let (body, names) = cut(halved);
            let feature = names.faces[0].feature;
            let made = by_faces(feature, &body, &names.faces, 1e-6);
            // What regeneration stored is what this gives.
            assert_eq!(
                (&made.edges, &made.vertices),
                (&names.edges, &names.vertices)
            );
            let unique =
                |list: &[PersistentName]| list.iter().collect::<BTreeSet<_>>().len() == list.len();
            assert!(unique(&made.edges) && unique(&made.vertices));
            for name in made.edges.iter().chain(&made.vertices) {
                assert_eq!(name.check(), Ok(()), "{name}");
                assert_eq!(&name.to_string().parse::<PersistentName>().unwrap(), name);
            }
            let numbered = made
                .edges
                .iter()
                .chain(&made.vertices)
                .filter(|name| !name.sources.is_empty())
                .count();
            assert_eq!(numbered, if halved { 6 } else { 0 });
            let (other, other_faces, shuffle) = shuffled(&body, &names.faces);
            assert_eq!(shuffle.faces.len(), other_faces.len());
            let again = by_faces(feature, &other, &other_faces, 1e-6);
            for (old, new) in shuffle.edges.iter().enumerate() {
                assert_eq!(again.edges[*new], made.edges[old], "edge {old}");
            }
            for (old, new) in shuffle.vertices.iter().enumerate() {
                assert_eq!(again.vertices[*new], made.vertices[old], "vertex {old}");
            }
        }
    }

    #[test]
    fn a_body_that_is_not_one_is_named_without_a_panic() {
        let (mut body, names) = cut(false);
        let feature = names.faces[0].feature;
        // References that point nowhere, and fewer names than faces.
        body.edges[0].start = VertexId(u32::MAX);
        body.loops[0].coedges[0].edge = EdgeId(u32::MAX);
        body.faces[1].loops.push(craft3d_topo::LoopId(u32::MAX));
        body.vertices[0].point = Vec3::new(f64::NAN, f64::INFINITY, 0.0);
        let made = by_faces(feature, &body, &names.faces[..3], f64::NAN);
        assert_eq!(made.edges.len(), body.edges.len());
        assert_eq!(made.vertices.len(), body.vertices.len());
        let empty = by_faces(
            feature,
            &Body::new(craft3d_topo::BodyKind::Solid),
            &[],
            1e-6,
        );
        assert!(empty.edges.is_empty() && empty.vertices.is_empty());
    }

    #[test]
    fn an_entity_keeps_the_name_first_given_when_it_is_the_only_one_from_there() {
        let name = |text: &str| text.parse::<PersistentName>().unwrap();
        let first = [name("F1:vertex(3)")];
        let between = [
            name("F2:corner[F2:copy(0)/F1:face(0)]"),
            name("F2:copy(0)/F1:vertex(3)"),
        ];
        let there = |instance: u64, entity: usize| -> std::slice::Iter<'_, PersistentName> {
            match (instance, entity) {
                (0, 3) => first.iter(),
                (1, 0) => between.iter(),
                _ => [].iter(),
            }
        };
        let kept = [
            Some((0, 3)),
            None,
            Some((1, 0)),
            Some((0, 7)),
            Some((0, 7)),
            Some((1, 9)),
        ];
        let aliases = kept_names(FeatureId(5), &kept, there);
        // The two pieces of entity 7 are neither of them entity 7; a name
        // by faces in a body in between is not carried along.
        assert_eq!(
            aliases,
            BTreeMap::from([
                (0, vec![name("F5:copy(0)/F1:vertex(3)")]),
                (2, vec![name("F5:copy(1)/F2:copy(0)/F1:vertex(3)")]),
            ])
        );
    }
}
