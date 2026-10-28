//! A flat-folded stack folded on: some creases at 180 degrees, the
//! others by other angles (docs/design/origami.md › *A stack folded on*).
//!
//! A crease at 180 degrees joins its two facets rigidly, face over (the
//! half-turn about it), so the facets the 180-degree creases join make
//! one rigid body, a *stack*, placed as the pattern of those creases
//! alone folds flat. That is the flat-folded state of a pattern of its
//! own: the sheet with only the 180-degree creases (and the cuts), every
//! one at 180 degrees, folded by reflections and its layers ordered as
//! any flat fold's ([`fold_flat`], [`layer_order`]); a facet of the full
//! pattern lies in one facet of it and takes its place and its layer.
//! The other creases are the joints of a linkage of the stacks: a crease
//! piece between facets `a` and `b` (which the flat state places alike,
//! `R_a = R_b`, as the crease is not folded there) turns the stack of
//! `b` against the stack of `a` about the crease where the flat state
//! put it, `R_a(axis)`, by its own angle: from `T_f = B_stack(f) ∘ R_f`
//! and `T_b = T_a ∘ M(ρ)`, `B_b = B_a ∘ R_a M(ρ) R_a⁻¹`, the turn by the
//! same angle about the carried axis. That linkage is folded as any
//! ([`fold_linkage`]: given angles, solved ones, the continuation from
//! the stacks lying flat), and every facet is placed by
//! `T_f = B_stack(f) ∘ R_f`. Facets of one stack keep the flat state's
//! layers; facets of different stacks may not meet, except along a line
//! both are folded about (the layers of a stack folded on bend round the
//! same line: in the zero-thickness model they touch there).

use super::{
    Angle, Arrangement, BoundarySegment, Crease, CreaseKind, EdgeKind, FoldError, Folded, Joint,
    Layers, Linkage, arrange, fold_flat, fold_linkage, layer_order, lift,
};
use craft3d_math::{Bbox3, Transform, Vec3};
use std::collections::BTreeSet;
use std::f64::consts::PI;

/// A stack folded on: the fold, the stack of each facet, the facets'
/// layers within their stacks, and the edges of the creases the stacks
/// are folded about.
#[derive(Clone, Debug, PartialEq)]
pub struct Stacked {
    pub folded: Folded,
    /// Per facet, its stack (a number from 0, the fixed facet's first).
    pub stack: Vec<usize>,
    /// The layers of the flat-folded stacks, per facet of the full
    /// pattern: a facet's layer and side are those of the facet of the
    /// 180-degree pattern it lies in; `above` holds the pairs of facets of
    /// one stack that overlap.
    pub layers: Layers,
    /// Per facet, the direction its stack's layers rise (the flat state's
    /// +z, carried by the stack's motion).
    pub up: Vec<Vec3>,
    /// The arrangement edges of the creases that fold the stacks.
    pub bends: BTreeSet<usize>,
}

/// Whether the angles are a stack folded on: at least one given angle at
/// 180 degrees (within `angular`) and at least one other given angle that
/// is not 0 or 180.
pub fn folds_stacked(angles: &[Angle], angular: f64) -> bool {
    let at_flat = |v: f64| (v.abs() - PI).abs() <= angular;
    let flat = angles
        .iter()
        .any(|a| matches!(a, Angle::Given(v) if at_flat(*v)));
    let other = angles
        .iter()
        .any(|a| matches!(a, Angle::Given(v) if !at_flat(*v)));
    flat && other
}

/// Folds the arrangement with the 180-degree creases as stacks and the
/// others as the stacks' joints; see the module documentation.
pub fn fold_stacked(
    arr: &Arrangement,
    angles: &[Angle],
    fixed: usize,
    tol: f64,
    angular: f64,
) -> Result<Stacked, FoldError> {
    if !(tol.is_finite() && tol > 0.0 && angular.is_finite() && angular > 0.0) {
        return Err(FoldError::Tolerance);
    }
    let n = arr.facets.len();
    if angles.len() != arr.hinges.len() || fixed >= n {
        return Err(FoldError::Broken {
            reason: "the angles do not match the creases",
        });
    }
    let at_flat = |a: &Angle| matches!(a, Angle::Given(v) if (v.abs() - PI).abs() <= angular);
    // The creases at 180 degrees: every piece of a line with an angle
    // folds by it, so a crease is in or out as a whole.
    let mut flat_crease = vec![false; arr.creases.len()];
    for (h, a) in arr.hinges.iter().zip(angles) {
        if at_flat(a)
            && let Some(slot) = flat_crease.get_mut(h.crease)
        {
            *slot = true;
        }
    }
    for (h, a) in arr.hinges.iter().zip(angles) {
        if flat_crease.get(h.crease).copied().unwrap_or(false) && !at_flat(a) {
            let id = arr.edges.get(h.edge).map_or(0, |e| e.source);
            return Err(FoldError::MixedFlat { id });
        }
    }

    // The pattern of the 180-degree creases alone, folded flat.
    let boundary: Vec<BoundarySegment> = arr
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Boundary)
        .filter_map(|e| {
            Some(BoundarySegment {
                id: e.source,
                start: arr.point(e.from)?,
                end: arr.point(e.to)?,
                reversed: false,
            })
        })
        .collect();
    let creases: Vec<Crease> = arr
        .creases
        .iter()
        .zip(&flat_crease)
        .filter(|(c, flat)| **flat || c.kind == CreaseKind::Cut)
        .map(|(c, _)| c.clone())
        .collect();
    let reduced = arrange(&boundary, &creases, tol)?;
    let inside = |f: usize| -> Option<[f64; 2]> {
        let facet = arr.facets.get(f)?;
        let rings: Vec<Vec<[f64; 2]>> = std::iter::once(&facet.outer)
            .chain(&facet.holes)
            .map(|c| arr.polygon(c))
            .collect();
        super::layers::interior_point(&rings)
    };
    let mut in_reduced = Vec::with_capacity(n);
    for f in 0..n {
        let r = inside(f)
            .and_then(|p| reduced.facet_at(p))
            .ok_or(FoldError::Broken {
                reason: "a facet lies in no facet of the 180-degree pattern",
            })?;
        in_reduced.push(r);
    }
    let reduced_angles: Vec<Angle> = reduced
        .hinges
        .iter()
        .map(|h| {
            let sign = reduced.creases.get(h.crease).map_or(1.0, |c| c.kind.sign());
            Angle::Given(sign * PI)
        })
        .collect();
    let fixed_reduced = *in_reduced.get(fixed).ok_or(FoldError::Broken {
        reason: "the fixed facet is missing",
    })?;
    let flat = fold_flat(&reduced, &reduced_angles, fixed_reduced, tol)?;
    let reduced_layers = layer_order(&reduced, &flat, tol)?;
    let place =
        |f: usize| -> Option<Transform> { flat.transforms.get(*in_reduced.get(f)?).copied() };

    // The stacks: facets joined by 180-degree creases.
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        let mut guard = 0usize;
        while let Some(&p) = parent.get(i) {
            if p == i || guard > parent.len() {
                break;
            }
            guard += 1;
            i = p;
        }
        i
    }
    for (h, a) in arr.hinges.iter().zip(angles) {
        if at_flat(a) {
            let (x, y) = (root(&mut parent, h.left), root(&mut parent, h.right));
            if x != y
                && let Some(slot) = parent.get_mut(x.max(y))
            {
                *slot = x.min(y);
            }
        }
    }
    let roots: Vec<usize> = (0..n).map(|f| root(&mut parent, f)).collect();
    let mut number: Vec<Option<usize>> = vec![None; n];
    let mut count = 0usize;
    for f in std::iter::once(fixed).chain(0..n) {
        let Some(&r) = roots.get(f) else { continue };
        if number.get(r).copied().flatten().is_none() {
            if let Some(slot) = number.get_mut(r) {
                *slot = Some(count);
            }
            count += 1;
        }
    }
    let stack: Vec<usize> = roots
        .iter()
        .map(|&r| number.get(r).copied().flatten().unwrap_or(0))
        .collect();

    // The stacks' linkage: every other crease piece, its axis where the
    // flat state put it, about the box's middle.
    let box_ = Bbox3::from_points(arr.vertices.iter().map(|&p| lift(p)));
    let centre = box_.center().unwrap_or(Vec3::ZERO);
    let mut joints = Vec::new();
    let mut joint_angles = Vec::new();
    let mut joint_of_hinge: Vec<Option<usize>> = vec![None; arr.hinges.len()];
    let mut bends = BTreeSet::new();
    for (k, (h, a)) in arr.hinges.iter().zip(angles).enumerate() {
        if at_flat(a) {
            continue;
        }
        let e = arr.edges.get(h.edge).ok_or(FoldError::Broken {
            reason: "a hinge has no edge",
        })?;
        let (Some(p), Some(q), Some(r)) = (arr.point(e.from), arr.point(e.to), place(h.left))
        else {
            return Err(FoldError::Broken {
                reason: "an edge has no vertex",
            });
        };
        let (p, q) = (r.point(lift(p)) - centre, r.point(lift(q)) - centre);
        let direction = (q - p)
            .normalized()
            .ok_or(FoldError::TooShort { id: e.source })?;
        let side = r.vector(Vec3::Z).cross(direction);
        let (Some(&sl), Some(&sr)) = (stack.get(h.left), stack.get(h.right)) else {
            continue;
        };
        if let Some(slot) = joint_of_hinge.get_mut(k) {
            *slot = Some(joints.len());
        }
        joints.push(Joint::crease(
            sl,
            sr,
            p,
            direction,
            side,
            p.distance(q),
            e.source,
        ));
        joint_angles.push(*a);
        bends.insert(h.edge);
    }
    let link = Linkage {
        bodies: count,
        joints,
        size: box_.diagonal(),
    };
    // The fixed facet's stack is number 0.
    let moved = fold_linkage(&link, &joint_angles, 0, tol, angular)?;

    // Every facet: its flat place, then its stack's motion.
    let (from, to) = (
        Transform::translation(-centre),
        Transform::translation(centre),
    );
    let mut transforms = Vec::with_capacity(n);
    let mut up = Vec::with_capacity(n);
    for (f, &s) in stack.iter().enumerate() {
        let r = place(f).ok_or(FoldError::Broken {
            reason: "a facet has no flat place",
        })?;
        let b = moved.transforms.get(s).ok_or(FoldError::Broken {
            reason: "a stack was not placed",
        })?;
        transforms.push(r.then(&from).then(b).then(&to));
        up.push(b.vector(Vec3::Z));
    }
    let folded_angles: Vec<f64> = arr
        .hinges
        .iter()
        .zip(angles)
        .enumerate()
        .map(
            |(k, (_, a))| match (a, joint_of_hinge.get(k).copied().flatten()) {
                (Angle::Given(v), None) => v.signum() * PI,
                (_, Some(j)) => moved.angles.get(j).copied().unwrap_or(0.0),
                (Angle::Free { sign }, None) => sign * PI,
            },
        )
        .collect();
    let piece: Vec<usize> = stack
        .iter()
        .map(|&s| moved.piece.get(s).copied().unwrap_or(0))
        .collect();
    let roots: Vec<usize> = moved
        .roots
        .iter()
        .map(|&s| stack.iter().position(|&x| x == s).unwrap_or(0))
        .collect();

    // The layers per facet, and the pairs of one stack that overlap.
    let layer: Vec<usize> = in_reduced
        .iter()
        .map(|&r| reduced_layers.layer.get(r).copied().unwrap_or(0))
        .collect();
    let flipped: Vec<bool> = in_reduced
        .iter()
        .map(|&r| reduced_layers.flipped.get(r).copied().unwrap_or(false))
        .collect();
    let above_reduced: BTreeSet<[usize; 2]> = reduced_layers.above.iter().copied().collect();
    let flat_rings: Vec<Vec<Vec<[f64; 2]>>> = (0..n)
        .map(|f| {
            let (Some(facet), Some(t)) = (arr.facets.get(f), place(f)) else {
                return Vec::new();
            };
            std::iter::once(&facet.outer)
                .chain(&facet.holes)
                .map(|c| {
                    arr.polygon(c)
                        .into_iter()
                        .map(|p| {
                            let q = t.point(lift(p));
                            [q.x, q.y]
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    let mut above = Vec::new();
    for f in 0..n {
        for g in 0..n {
            let (Some(&rf), Some(&rg)) = (in_reduced.get(f), in_reduced.get(g)) else {
                continue;
            };
            if stack.get(f) != stack.get(g) || !above_reduced.contains(&[rf, rg]) {
                continue;
            }
            if let (Some(a), Some(b)) = (flat_rings.get(f), flat_rings.get(g))
                && overlap(a, b, tol)
            {
                above.push([f, g]);
            }
        }
    }
    above.sort_unstable();
    let layers = Layers {
        above,
        layer,
        flipped,
        cyclic: reduced_layers.cyclic,
        cells: reduced_layers.cells,
        stacked: reduced_layers.stacked,
        work: reduced_layers.work,
    };
    Ok(Stacked {
        folded: Folded {
            transforms,
            angles: folded_angles,
            gap: moved.gap.max(flat.gap),
            roots,
            piece,
        },
        stack,
        layers,
        up,
        bends,
    })
}

/// Whether two flat regions overlap in area: a point inside one lies
/// inside the other (further than the tolerance from its edges), or their
/// edges cross properly.
fn overlap(a: &[Vec<[f64; 2]>], b: &[Vec<[f64; 2]>], tol: f64) -> bool {
    let strictly_in = |p: [f64; 2], rings: &[Vec<[f64; 2]>]| {
        super::inside_loops(p, rings.iter().map(Vec::as_slice))
            && super::ring_distance(rings, p) > tol
    };
    if let Some(p) = super::layers::interior_point(a)
        && strictly_in(p, b)
    {
        return true;
    }
    if let Some(p) = super::layers::interior_point(b)
        && strictly_in(p, a)
    {
        return true;
    }
    let edges = |r: &[Vec<[f64; 2]>]| -> Vec<([f64; 2], [f64; 2])> {
        r.iter()
            .flat_map(|ring| {
                let n = ring.len();
                (0..n).filter_map(move |i| Some((*ring.get(i)?, *ring.get((i + 1) % n)?)))
            })
            .collect()
    };
    let (ea, eb) = (edges(a), edges(b));
    ea.iter().any(|&(p, q)| {
        eb.iter().any(|&(r, s)| {
            let (d1, d2) = (super::sub(q, p), super::sub(s, r));
            let den = super::cross(d1, d2);
            if den == 0.0 {
                return false;
            }
            let w = super::sub(r, p);
            let (t, u) = (super::cross(w, d2) / den, super::cross(w, d1) / den);
            let margin = |len: f64| if len > 0.0 { tol / len } else { 1.0 };
            let (m1, m2) = (
                margin(super::dot(d1, d1).sqrt()),
                margin(super::dot(d2, d2).sqrt()),
            );
            t > m1 && t < 1.0 - m1 && u > m2 && u < 1.0 - m2
        })
    })
}
