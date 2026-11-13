//! Faces a blend uses up exactly.
//!
//! A blend cuts back the faces beside its edge to its borders. Where a
//! face is exactly as narrow as the blends that run along it take (two
//! rounds of half its width on both its edges, a round as long as the
//! face it runs into), the borders on it come to lie on its other edges
//! from end to end, and the corners that end them on the vertices there
//! (`cut_back` takes the short edges between whole). What is left of the
//! face is a loop whose edges cancel in pairs: two edges between the same
//! two vertices, run one way and back, one curve within the tolerance.
//! It bounds no area, so it is no face (the tessellator cannot mesh it,
//! and nothing could be built on it): the face goes, and each pair
//! becomes one edge, which the faces on its two sides now share.
//!
//! **Why this is general.** Nothing here asks what the curves are, or how
//! the face came to be so. A face is taken as used up only when every
//! coedge of its loops is paired with another of the same face that runs
//! between the same two vertices the other way, and the two curves are
//! proven one within the tolerance (`craft3d_geom::certify::curves_within`:
//! every point of either near the other, both ways, not sampled): its
//! boundary cancels. A region of a surface whose boundary cancels is made
//! of whole pieces of the surface that close up on themselves, with
//! nothing round them. A surface with a border of its own has none: a
//! plane, a cylinder and a cone run off to infinity, and a spline patch
//! has the edges of its parameter rectangle wherever they are neither
//! joined to the opposite edge nor pinched to a point. There the region
//! is empty. On a surface that closes up everywhere (a sphere, a torus, a
//! spline closed both ways, or closed one way and pinched to a pole at
//! both ends of the other) the same loops bound nothing or the whole
//! surface (a band of no width round a torus, and the torus less that
//! circle, have one and the same loops), and they cannot tell which: the
//! face is left as it is there, and the body is refused further on, as
//! before (`closes_up`). Only faces the blend changed are looked at (the
//! body had them, and their loops are not the ones it had), so a face the
//! blend did not touch stays as it was, bit for bit. The edge kept of a
//! pair is proven on the surface of the face across the one dropped (to
//! the tolerance, `curve_on_surface`), as every edge of a face has to lie
//! on its surface; the lower-numbered edge first, the other if that
//! fails, and the face stays (refused further on) where neither holds or
//! any pair is missing.

use craft3d_geom::Surface;
use craft3d_geom::certify::{CurveBound, Within, curve_on_surface, curves_within};
use craft3d_geom::pole::{Border, poles};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, Coedge, Edge, EdgeId, FaceId};
use std::collections::BTreeSet;

/// One pair of a used-up face: the edge kept, the edge it replaces, whether
/// the two run opposite ways along their curves, and the face across the
/// edge replaced, which takes the kept one.
struct Merge {
    keep: EdgeId,
    drop: EdgeId,
    flip: bool,
    across: FaceId,
}

/// Drops the faces of `body` that a blend of `before` used up (see the
/// module): of the faces `before` had, those whose loops the blend
/// changed, on a surface that does not close up. Returns the edges kept
/// of the pairs, which lie between other faces now; the edges dropped and
/// the faces' loops are left without uses, for `compact` and
/// `compact_faces` to take out.
pub(super) fn drop_used_up(
    body: &mut Body,
    before: &Body,
    tolerance: &Tolerance,
) -> BTreeSet<EdgeId> {
    let mut kept = BTreeSet::new();
    // Which faces a shell has, once: a face taken out is in none.
    let mut in_shell = vec![false; body.faces.len()];
    for shell in &body.shells {
        for face in &shell.faces {
            if let Some(flag) = in_shell.get_mut(face.0 as usize) {
                *flag = true;
            }
        }
    }
    for index in 0..before.faces.len().min(body.faces.len()) {
        let face = FaceId(u32::try_from(index).unwrap_or(u32::MAX));
        if !in_shell.get(index).copied().unwrap_or(false) || !changed(body, before, face) {
            continue;
        }
        if body.face(face).is_none_or(|f| closes_up(&f.surface)) {
            continue;
        }
        let coedges = coedges_of(body, face);
        if coedges.is_empty() {
            continue;
        }
        let Some(merges) = used_up(body, face, &coedges, tolerance) else {
            continue;
        };
        apply(body, face, &merges);
        if let Some(flag) = in_shell.get_mut(index) {
            *flag = false;
        }
        kept.extend(merges.iter().map(|m| m.keep));
    }
    kept
}

/// Whether the loops of `face` in `body` are not the ones it had in
/// `before` (compared in place, nothing copied).
fn changed(body: &Body, before: &Body, face: FaceId) -> bool {
    let (Some(now), Some(then)) = (body.face(face), before.face(face)) else {
        return true;
    };
    now.loops != then.loops
        || now
            .loops
            .iter()
            .any(|l| body.loop_(*l).map(|x| &x.coedges) != before.loop_(*l).map(|x| &x.coedges))
}

/// Whether `surface` closes up everywhere, so that a boundary that
/// cancels could bound the whole of it (see the module): a sphere and a
/// torus; a spline whose parameter rectangle has every side joined to
/// the opposite one or pinched to a point (`craft3d_geom::closure`,
/// `craft3d_geom::pole`, the tests the booleans and the tessellator take).
fn closes_up(surface: &Surface) -> bool {
    match surface {
        Surface::Sphere(_) | Surface::Torus(_) => true,
        Surface::Plane(_) | Surface::Cylinder(_) | Surface::Cone(_) => false,
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            let [u, v] = craft3d_geom::closure::closes(surface);
            let found = poles(surface);
            let pinched = |a: Border, b: Border| {
                found.iter().any(|p| p.border == a) && found.iter().any(|p| p.border == b)
            };
            (u || pinched(Border::UStart, Border::UEnd))
                && (v || pinched(Border::VStart, Border::VEnd))
        }
    }
}

/// The coedges of every loop of `face`, in order.
fn coedges_of(body: &Body, face: FaceId) -> Vec<Coedge> {
    body.face(face)
        .map(|f| {
            f.loops
                .iter()
                .filter_map(|l| body.loop_(*l))
                .flat_map(|l| l.coedges.iter().copied())
                .collect()
        })
        .unwrap_or_default()
}

/// The faces that use `edge`, once per use.
fn users(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    let mut out = Vec::new();
    for shell in &body.shells {
        for face in &shell.faces {
            let uses = coedges_of(body, *face)
                .iter()
                .filter(|c| c.edge == edge)
                .count();
            out.extend(std::iter::repeat_n(*face, uses));
        }
    }
    out
}

/// The merges that take out `face`, when its coedges cancel in pairs;
/// `None` when one does not, or a pair cannot be made one edge.
fn used_up(
    body: &Body,
    face: FaceId,
    coedges: &[Coedge],
    tolerance: &Tolerance,
) -> Option<Vec<Merge>> {
    let mut paired = vec![false; coedges.len()];
    let mut merges = Vec::new();
    for (i, first) in coedges.iter().enumerate() {
        if paired.get(i).copied().unwrap_or(true) {
            continue;
        }
        let found = coedges
            .iter()
            .enumerate()
            .skip(i + 1)
            .find_map(|(j, second)| {
                if paired.get(j).copied().unwrap_or(true) {
                    return None;
                }
                let same_way = cancel(body, *first, *second, tolerance)?;
                Some((j, same_way))
            })?;
        let (j, same_way) = found;
        let second = coedges.get(j)?;
        merges.push(merge(
            body,
            face,
            first.edge,
            second.edge,
            !same_way,
            tolerance,
        )?);
        for k in [i, j] {
            if let Some(flag) = paired.get_mut(k) {
                *flag = true;
            }
        }
    }
    Some(merges)
}

/// Whether the coedges `a` and `b` (of one face) cancel: two edges between
/// the same two vertices, run opposite ways by the face, on one curve
/// within the tolerance. `Some(same_way)` when they do, `same_way` saying
/// whether the two edges run the same way along their curves.
fn cancel(body: &Body, a: Coedge, b: Coedge, tolerance: &Tolerance) -> Option<bool> {
    if a.edge == b.edge {
        return None;
    }
    let (ea, eb) = (body.edge(a.edge)?, body.edge(b.edge)?);
    let same_way = if ea.start == ea.end || eb.start == eb.end {
        // Closed edges at one vertex: the way their tangents leave it.
        if !(ea.start == ea.end && eb.start == eb.end && ea.start == eb.start) {
            return None;
        }
        let (_, ta) = ea.curve.derivatives(ea.range.0);
        let (_, tb) = eb.curve.derivatives(eb.range.0);
        ta.dot(tb) > 0.0
    } else if ea.start == eb.start && ea.end == eb.end {
        true
    } else if ea.start == eb.end && ea.end == eb.start {
        false
    } else {
        return None;
    };
    // Run opposite ways by the face.
    if (a.reversed != b.reversed) != same_way {
        return None;
    }
    one_curve(ea, eb, same_way, tolerance).then_some(same_way)
}

/// Whether the curves of `a` and `b` are one within the tolerance, proven.
fn one_curve(a: &Edge, b: &Edge, same_way: bool, tolerance: &Tolerance) -> bool {
    curves_within(
        &CurveBound::new(&a.curve),
        a.range,
        &CurveBound::new(&b.curve),
        b.range,
        same_way,
        Within {
            target: tolerance.linear,
            most: super::PROOF_PIECES,
        },
    )
    .is_ok()
}

/// How the pair of edges `a` and `b` of the used-up `face` becomes one:
/// the edge kept lies on the surface of the face across the other, proven
/// (the lower-numbered one tried first). `None` where an edge has other
/// uses than the face and one face across, the faces across are one, or
/// neither edge is proven on the other's face across.
fn merge(
    body: &Body,
    face: FaceId,
    a: EdgeId,
    b: EdgeId,
    flip: bool,
    tolerance: &Tolerance,
) -> Option<Merge> {
    let across = |edge: EdgeId| -> Option<FaceId> {
        match users(body, edge).as_slice() {
            [x, y] if *x == face && *y != face => Some(*y),
            [x, y] if *y == face && *x != face => Some(*x),
            _ => None,
        }
    };
    let (across_a, across_b) = (across(a)?, across(b)?);
    if across_a == across_b {
        return None;
    }
    let (low, high) = if a.0 <= b.0 {
        ((a, across_a), (b, across_b))
    } else {
        ((b, across_b), (a, across_a))
    };
    for ((keep, _), (drop, across)) in [(low, high), (high, low)] {
        if on_face(body, keep, across, tolerance) {
            return Some(Merge {
                keep,
                drop,
                flip,
                across,
            });
        }
    }
    None
}

/// Whether the curve of `edge` lies on the surface of `face` within the
/// tolerance, proven.
fn on_face(body: &Body, edge: EdgeId, face: FaceId, tolerance: &Tolerance) -> bool {
    let (Some(data), Some(face)) = (body.edge(edge), body.face(face)) else {
        return false;
    };
    curve_on_surface(
        &CurveBound::new(&data.curve),
        data.range,
        &face.surface,
        None,
        &[],
        Within {
            target: tolerance.linear,
            most: super::PROOF_PIECES,
        },
    )
    .is_ok()
}

/// Takes `face` out of its shells and empties its loops; in each face
/// across a dropped edge, the kept edge takes the dropped one's place,
/// run the same way as the face ran the dropped one.
fn apply(body: &mut Body, face: FaceId, merges: &[Merge]) {
    for merge in merges {
        let loops = body
            .face(merge.across)
            .map(|f| f.loops.clone())
            .unwrap_or_default();
        for id in loops {
            if let Some(ring) = body.loops.get_mut(id.0 as usize) {
                for coedge in &mut ring.coedges {
                    if coedge.edge == merge.drop {
                        *coedge = Coedge {
                            edge: merge.keep,
                            reversed: coedge.reversed != merge.flip,
                        };
                    }
                }
            }
        }
    }
    let loops = body.face(face).map(|f| f.loops.clone()).unwrap_or_default();
    for id in loops {
        if let Some(ring) = body.loops.get_mut(id.0 as usize) {
            ring.coedges.clear();
        }
    }
    for shell in &mut body.shells {
        shell.faces.retain(|f| *f != face);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;
    use craft3d_geom::{Circle, Cone, Curve3, Cylinder, Plane, Sphere, Torus};
    use craft3d_math::{Frame, Vec3};
    use craft3d_topo::{BodyKind, Face, Loop, LoopId, Shell, Vertex, VertexId};
    use std::f64::consts::TAU;

    fn coedge(edge: u32, reversed: bool) -> Coedge {
        Coedge {
            edge: EdgeId(edge),
            reversed,
        }
    }

    /// Three faces: one on `surface` bounded by a circle run each way (two
    /// edges on the one circle), and two flat faces across, one on each
    /// edge; and the same before the blend, when the first face had the
    /// first edge only (so the blend changed it). The circle, of radius 6
    /// about the z axis in z = 0, lies on the plane z = 0, on the torus of
    /// radii 5 and 1 about the z axis (its outer equator) and on the
    /// sphere of radius 6 (its equator).
    fn bodies(surface: Surface) -> (Body, Body) {
        let circle = Curve3::Circle(Circle::new(Frame::WORLD, 6.0).unwrap());
        let edge = || Edge {
            curve: circle.clone(),
            range: (0.0, TAU),
            start: VertexId(0),
            end: VertexId(0),
        };
        let flat = Surface::Plane(Plane::new(Frame::WORLD));
        let face = |surface: Surface, l: u32| Face {
            surface,
            reversed: false,
            loops: vec![LoopId(l)],
        };
        let body = |first: Vec<Coedge>| Body {
            kind: BodyKind::Solid,
            vertices: vec![Vertex {
                point: Vec3::new(6.0, 0.0, 0.0),
            }],
            edges: vec![edge(), edge()],
            loops: vec![
                Loop { coedges: first },
                Loop {
                    coedges: vec![coedge(0, true)],
                },
                Loop {
                    coedges: vec![coedge(1, false)],
                },
            ],
            faces: vec![
                face(surface.clone(), 0),
                face(flat.clone(), 1),
                face(flat.clone(), 2),
            ],
            shells: vec![Shell {
                faces: vec![FaceId(0), FaceId(1), FaceId(2)],
            }],
            tolerances: Default::default(),
        };
        (
            body(vec![coedge(0, false), coedge(1, true)]),
            body(vec![coedge(0, false)]),
        )
    }

    #[test]
    fn a_cancelling_boundary_on_a_surface_that_closes_up_is_left_alone() {
        let tolerance = Tolerance::new(1e-6, 1e-9).unwrap();
        // On a torus and on a sphere the circle run each way bounds
        // nothing or the whole surface less the circle: the face stays.
        for surface in [
            Surface::Torus(Torus::new(Frame::WORLD, 5.0, 1.0).unwrap()),
            Surface::Sphere(Sphere::new(Frame::WORLD, 6.0).unwrap()),
        ] {
            let kind = surface.kind();
            let (mut body, before) = bodies(surface);
            let kept = drop_used_up(&mut body, &before, &tolerance);
            assert!(kept.is_empty(), "{kind}");
            assert_eq!(body.shells[0].faces.len(), 3, "{kind}");
            assert_eq!(body.loops[0].coedges.len(), 2, "{kind}");
        }
        // On the plane the same loops bound nothing: the face goes, and
        // the face across the dropped edge takes the one kept, run the
        // same way.
        let (mut body, before) = bodies(Surface::Plane(Plane::new(Frame::WORLD)));
        let kept = drop_used_up(&mut body, &before, &tolerance);
        assert_eq!(kept.into_iter().collect::<Vec<_>>(), [EdgeId(0)]);
        assert_eq!(body.shells[0].faces, [FaceId(1), FaceId(2)]);
        assert!(body.loops[0].coedges.is_empty());
        assert_eq!(body.loops[1].coedges, [coedge(0, true)]);
        assert_eq!(body.loops[2].coedges, [coedge(0, false)]);
        // Unchanged since before: not looked at.
        let (mut body, _) = bodies(Surface::Plane(Plane::new(Frame::WORLD)));
        let same = body.clone();
        assert!(drop_used_up(&mut body, &same, &tolerance).is_empty());
        assert_eq!(body, same);
    }

    #[test]
    fn surfaces_that_close_up_everywhere() {
        let f = Frame::WORLD;
        assert!(closes_up(&Surface::Torus(Torus::new(f, 5.0, 1.0).unwrap())));
        assert!(closes_up(&Surface::Sphere(Sphere::new(f, 2.0).unwrap())));
        assert!(!closes_up(&Surface::Plane(Plane::new(f))));
        assert!(!closes_up(&Surface::Cylinder(
            Cylinder::new(f, 2.0).unwrap()
        )));
        assert!(!closes_up(&Surface::Cone(Cone::new(f, 2.0, 0.3).unwrap())));
    }
}
