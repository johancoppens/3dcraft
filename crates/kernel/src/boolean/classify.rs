//! Where the regions of one solid's faces are with respect to the other
//! solid, and which of them the operation keeps.

use super::hits::{curve_hits, signed_distance};
use super::outline::{Outline, Side, line_in_box};
use super::regions::{self, Half};
use super::section::Solid;
use super::{Arcs, BooleanOp, FaceSource, LOST, NO_RAY, ON_A_FACE, Operand, Stop, Vertices};
use super::{face_id, geometry};
use craft3d_geom::intersect::CurveIntersection;
use craft3d_geom::{Curve3, Line, Surface};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::Coedge;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    In,
    Out,
    /// On a face of the other solid whose outward normal is the same.
    OnSame,
    /// On a face of the other solid that faces the other way.
    OnOpposite,
}

/// Directions for the rays of [`inside`]: none along an axis or a diagonal.
const RAYS: [(f64, f64, f64); 12] = [
    (0.577, 0.211, 0.789),
    (-0.343, 0.871, 0.351),
    (0.129, -0.497, 0.858),
    (-0.811, -0.263, 0.523),
    (0.657, 0.731, -0.183),
    (0.279, -0.913, -0.297),
    (-0.593, 0.437, -0.677),
    (0.913, -0.151, 0.379),
    (-0.117, 0.337, -0.934),
    (0.453, 0.619, 0.641),
    (-0.767, -0.587, -0.259),
    (0.211, -0.143, -0.967),
];

/// How many points of a region are tried before giving up on telling where
/// it is.
const TRIES: usize = 4;

/// True when `p`, which is not on its boundary, is inside `solid`: a ray
/// from it crosses the boundary an odd number of times. A ray that meets
/// an edge, a vertex, or a face tangentially is abandoned for the next.
pub(super) fn inside(solid: &Solid, p: Vec3, tolerance: &Tolerance) -> Result<bool, Stop> {
    let tol = tolerance.linear;
    // A point within reach of a face of the solid is neither inside nor
    // outside it by any margin that means something.
    for (face, (outline, box_)) in solid.outlines.iter().zip(&solid.boxes).enumerate() {
        if !box_.contains(p, 2.0 * tol) {
            continue;
        }
        let near = signed_distance(&outline.surface, p).is_some_and(|d| d.abs() < 2.0 * tol);
        if near && let Ok((u, v)) = outline.surface.project(p) {
            let named = [solid.name(face)];
            let foot = outline.surface.eval(u, v);
            if outline
                .side(foot, tol)
                .map_err(|r| Stop::of(r).at(&named))?
                != Side::Out
            {
                return Err(Stop::of(ON_A_FACE).at(&named));
            }
        }
    }
    'rays: for (x, y, z) in RAYS {
        let Ok(line) = Line::new(p, Vec3::new(x, y, z)) else {
            continue;
        };
        let ray = Curve3::Line(line);
        let mut crossings = 0_usize;
        for (face, (outline, box_)) in solid.outlines.iter().zip(&solid.boxes).enumerate() {
            let Some((enter, leave)) =
                line_in_box(line.origin(), line.direction(), box_, 2.0 * tol)
            else {
                continue;
            };
            // Behind the point there is nothing to count. The stretch
            // looked at starts at the point and ends well beyond the
            // face, so that neither end of it is a place where the ray
            // is found near the surface without having crossed it.
            let from = -2.0 * tol;
            if !(leave > from) {
                continue;
            }
            let leave = leave + 16.0 * tol + 0.01 * (leave - enter.max(from));
            let named = [solid.name(face)];
            let side = |q: Vec3| outline.side(q, tol).map_err(|r| Stop::of(r).at(&named));
            let hits = match curve_hits(&ray, (from, leave), &outline.surface, tolerance)
                .map_err(|e| geometry(e, &named))?
            {
                CurveIntersection::OnSurface => continue 'rays,
                CurveIntersection::Hits(hits) => hits,
            };
            for hit in hits {
                if hit.t.abs() <= tol {
                    // The point is on this surface: fine when the face is
                    // elsewhere on it.
                    if side(p)? != Side::Out {
                        return Err(Stop::of(ON_A_FACE).at(&named));
                    }
                    continue;
                }
                if hit.t < 0.0 || !box_.contains(hit.point, 2.0 * tol) {
                    continue;
                }
                match side(hit.point)? {
                    Side::Out => {}
                    Side::On => continue 'rays,
                    Side::In if hit.tangent => continue 'rays,
                    Side::In => crossings += 1,
                }
            }
        }
        return Ok(crossings % 2 == 1);
    }
    Err(Stop::of(NO_RAY))
}

/// Where a region of a face of `own` is with respect to `other`.
fn classify(
    region: &Outline,
    other: &Solid,
    coincident: &[(usize, bool)],
    tolerance: &Tolerance,
) -> Result<Class, Stop> {
    let tol = tolerance.linear;
    let candidates = region
        .interior_points(tol, 4.0 * tol, TRIES)
        .map_err(Stop::of)?;
    let mut failure = None;
    'points: for p in candidates {
        for &(face, same_normal) in coincident {
            let outline = other.outlines.get(face).ok_or(Stop::of(LOST))?;
            match outline
                .side(p, tol)
                .map_err(|r| Stop::of(r).at(&[other.name(face)]))?
            {
                Side::Out => {}
                // A boundary of the other body that was not imprinted
                // here: no other point of the region would mean more.
                Side::On => return Err(Stop::of(ON_A_FACE).at(&[other.name(face)])),
                Side::In => {
                    let same = same_normal == (region.reversed == outline.reversed);
                    return Ok(if same {
                        Class::OnSame
                    } else {
                        Class::OnOpposite
                    });
                }
            }
        }
        // A point may by chance be where the bodies touch without
        // crossing, or have no usable ray: the next one decides as well.
        match inside(other, p, tolerance) {
            Ok(true) => return Ok(Class::In),
            Ok(false) => return Ok(Class::Out),
            Err(stop @ Stop::Unsupported { .. }) => {
                failure = failure.or(Some(stop));
                continue 'points;
            }
            Err(stop) => return Err(stop),
        }
    }
    Err(failure.unwrap_or(Stop::of(NO_RAY)))
}

/// A region of a face of an operand that becomes a face of the result.
pub(super) struct Kept {
    pub source: FaceSource,
    pub surface: Surface,
    pub reversed: bool,
    pub loops: Vec<Vec<Half>>,
}

/// The boundary of a face in arcs: per loop, the uses of the pieces of its
/// edges in the loop's direction.
fn boundary(solid: &Solid, face: usize, pieces: &[Vec<Half>]) -> Result<Vec<Vec<Half>>, Stop> {
    let f = solid.body.faces.get(face).ok_or(Stop::of(LOST))?;
    let mut loops = Vec::with_capacity(f.loops.len());
    for loop_ in f.loops.iter().filter_map(|&l| solid.body.loop_(l)) {
        let mut halves = Vec::new();
        for &Coedge { edge, reversed } in &loop_.coedges {
            let of_edge = pieces.get(edge.0 as usize).ok_or(Stop::of(LOST))?;
            if reversed {
                halves.extend(of_edge.iter().rev().map(|&(arc, flipped)| (arc, !flipped)));
            } else {
                halves.extend(of_edge.iter().copied());
            }
        }
        loops.push(halves);
    }
    Ok(loops)
}

/// The regions of every face of `own`, classified against `other`, and of
/// those the ones the operation keeps.
#[allow(clippy::too_many_arguments)]
pub(super) fn select(
    own: &Solid,
    other: &Solid,
    pieces: &[Vec<Half>],
    interior: &BTreeMap<usize, BTreeSet<usize>>,
    coincident: &BTreeMap<usize, Vec<(usize, bool)>>,
    arcs: &Arcs,
    vertices: &Vertices,
    op: BooleanOp,
    tolerance: &Tolerance,
    kept: &mut Vec<Kept>,
) -> Result<(), Stop> {
    let tol = tolerance.linear;
    let none = BTreeSet::new();
    for (face, outline) in own.outlines.iter().enumerate() {
        let named = [own.name(face)];
        let loops = boundary(own, face, pieces)?;
        let across = interior.get(&face).unwrap_or(&none);
        let whole = across.is_empty()
            && own.edges_of(face).iter().all(|edge| {
                pieces
                    .get(edge.0 as usize)
                    .is_some_and(|halves| halves.len() == 1)
            });
        let regions = if whole {
            vec![loops]
        } else {
            regions::build(
                &outline.surface,
                outline.reversed,
                &loops,
                across,
                &arcs.arcs,
                &vertices.points,
                tol,
            )
            .map_err(|r| Stop::of(r).at(&named))?
        };
        for loops in regions {
            // A face that nothing divides is its own outline.
            let made;
            let region = if whole {
                outline
            } else {
                let mut segs = Vec::with_capacity(loops.len());
                for loop_ in &loops {
                    segs.push(
                        regions::segs(loop_, &arcs.arcs, &vertices.points)
                            .map_err(|r| Stop::of(r).at(&named))?,
                    );
                }
                made = Outline::new(outline.surface.clone(), outline.reversed, segs, 4.0 * tol)
                    .map_err(|r| Stop::of(r).at(&named))?;
                &made
            };
            let coincident = coincident.get(&face).map_or(&[][..], Vec::as_slice);
            let class = classify(region, other, coincident, tolerance).map_err(|s| s.at(&named))?;
            // Of two coincident regions with the same normal A's is the
            // one kept; with opposite normals the solids lie on either
            // side and only A minus B has a face there, A's.
            let (keep, flip) = match (op, own.operand, class) {
                (BooleanOp::Union, _, Class::Out) => (true, false),
                (BooleanOp::Union, Operand::A, Class::OnSame) => (true, false),
                (BooleanOp::Intersect, _, Class::In) => (true, false),
                (BooleanOp::Intersect, Operand::A, Class::OnSame) => (true, false),
                (BooleanOp::Subtract, Operand::A, Class::Out | Class::OnOpposite) => (true, false),
                (BooleanOp::Subtract, Operand::B, Class::In) => (true, true),
                _ => (false, false),
            };
            if !keep {
                continue;
            }
            let loops = if flip {
                loops
                    .into_iter()
                    .map(|l| l.into_iter().rev().map(|(arc, r)| (arc, !r)).collect())
                    .collect()
            } else {
                loops
            };
            kept.push(Kept {
                source: FaceSource {
                    operand: own.operand,
                    face: face_id(face),
                },
                surface: outline.surface.clone(),
                reversed: outline.reversed != flip,
                loops,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::section::prepare;
    use crate::primitives;
    use craft3d_math::Frame;
    use craft3d_topo::Body;

    fn prepared(body: &Body) -> Solid<'_> {
        let tolerance = Tolerance::default();
        let mut vertices = Vertices {
            points: Vec::new(),
            meetings: Vec::new(),
            merge: 0.5 * tolerance.linear,
            guard: 2.0 * tolerance.linear,
        };
        match prepare(body, Operand::A, &mut vertices, &tolerance) {
            Ok(solid) => solid,
            Err(_) => panic!("a primitive is a valid operand"),
        }
    }

    fn is_inside(solid: &Solid, p: Vec3) -> bool {
        match inside(solid, p, &Tolerance::default()) {
            Ok(inside) => inside,
            Err(_) => panic!("no ray could tell"),
        }
    }

    /// The first ray tried, as a unit vector.
    fn first_ray() -> Vec3 {
        let (x, y, z) = RAYS[0];
        Vec3::new(x, y, z).normalized().unwrap()
    }

    #[test]
    fn a_ray_through_an_edge_or_a_vertex_is_not_counted() {
        let block = primitives::block(&Frame::WORLD, 2.0, 2.0, 2.0).unwrap();
        let solid = prepared(&block);
        let d = first_ray();
        // From inside, straight at the middle of an edge and at a corner:
        // the ray leaves through two or three faces at once.
        for target in [Vec3::new(2.0, 1.0, 2.0), Vec3::new(2.0, 2.0, 2.0)] {
            let p = target - d;
            assert!(block_contains(p));
            assert!(is_inside(&solid, p), "towards {target:?}");
        }
        // From outside, in through a face and out through the edge.
        let p = Vec3::new(2.0, 1.0, 2.0) - d * 3.0;
        assert!(!block_contains(p));
        assert!(!is_inside(&solid, p));
        // And the plain cases.
        assert!(is_inside(&solid, Vec3::new(1.0, 1.0, 1.0)));
        assert!(!is_inside(&solid, Vec3::new(3.0, 1.0, 1.0)));
        assert!(!is_inside(&solid, Vec3::new(1.0, 1.0, -0.5)));
    }

    fn block_contains(p: Vec3) -> bool {
        [p.x, p.y, p.z].iter().all(|c| (0.0..=2.0).contains(c))
    }

    #[test]
    fn a_ray_that_grazes_a_cylinder_is_not_counted() {
        let cylinder = primitives::cylinder(&Frame::WORLD, 1.0, 2.0).unwrap();
        let solid = prepared(&cylinder);
        let d = first_ray();
        // The point of the wall where the first ray is tangent to it.
        let across = Vec3::new(-d.y, d.x, 0.0).normalized().unwrap();
        let touch = across + Vec3::Z;
        let p = touch - d;
        assert!(p.x.hypot(p.y) > 1.0);
        assert!(!is_inside(&solid, p));
        assert!(is_inside(&solid, Vec3::new(0.2, -0.3, 1.0)));
        // Through the rim, where wall and lid meet.
        let rim = Vec3::new(0.6, 0.8, 2.0);
        assert!(is_inside(&solid, rim - d * 0.5));
    }

    #[test]
    fn inside_and_outside_of_cones_spheres_and_tori() {
        let frame = Frame::new(
            Vec3::new(3.0, -1.0, 2.0),
            Vec3::new(0.2, -0.5, 1.0),
            Vec3::new(1.0, 0.3, 0.0),
        )
        .unwrap();
        let cone = primitives::cone(&frame, 2.0, 4.0).unwrap();
        let ball = primitives::sphere(&frame, 2.0).unwrap();
        let ring = primitives::torus(&frame, 4.0, 1.0).unwrap();
        let (cone, ball, ring) = (prepared(&cone), prepared(&ball), prepared(&ring));
        let mut seed = 11_u64;
        let mut unit = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let mut counts = [0_usize; 3];
        for _ in 0..600 {
            let (x, y, z) = (unit() * 12.0, unit() * 12.0, unit() * 10.0);
            let p = frame.point(x, y, z);
            let rho = x.hypot(y);
            // Well away from the boundary, where rounding cannot decide.
            let in_cone = (rho - (2.0 - 0.5 * z), -z);
            let in_ball = Vec3::new(x, y, z).length() - 2.0;
            let in_ring = (rho - 4.0).hypot(z) - 1.0;
            if in_cone.0.abs() > 1e-3 && in_cone.1.abs() > 1e-3 {
                let expected = in_cone.0 < 0.0 && in_cone.1 < 0.0;
                assert_eq!(is_inside(&cone, p), expected, "cone at ({x}, {y}, {z})");
                counts[0] += usize::from(expected);
            }
            if in_ball.abs() > 1e-3 {
                assert_eq!(
                    is_inside(&ball, p),
                    in_ball < 0.0,
                    "ball at ({x}, {y}, {z})"
                );
                counts[1] += usize::from(in_ball < 0.0);
            }
            if in_ring.abs() > 1e-3 {
                assert_eq!(
                    is_inside(&ring, p),
                    in_ring < 0.0,
                    "ring at ({x}, {y}, {z})"
                );
                counts[2] += usize::from(in_ring < 0.0);
            }
        }
        // Some of the points were inside each.
        assert!(counts.iter().all(|&n| n > 3), "{counts:?}");
        // From the middle of the ring's hole every ray crosses the tube
        // twice or not at all.
        assert!(!is_inside(&ring, frame.origin()));
    }
}
