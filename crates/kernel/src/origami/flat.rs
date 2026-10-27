//! The flat-folded state: every fold crease folded by 180 degrees.
//!
//! A fold of 180 degrees about a crease in the sheet's plane is the
//! half-turn about that line, which keeps the plane and is, within it, the
//! reflection across the line (the facet comes down face over). So the
//! flat-folded state needs no solve: each facet's place is the product of
//! the reflections across the creases on its path from the fixed facet,
//! `T_child = R(crease) then T_parent`, built from the two mirrors whose
//! product the half-turn is (one across the vertical plane through the
//! crease, one across the sheet's plane), so the sheet's plane is kept
//! exactly: every z is 0, not 1e-16. Mountain and valley fold to the
//! same place; only the order of the layers tells them apart
//! (`layers.rs`). A crease off the forest closes a loop and holds when its
//! two facets agree; round a vertex that is Kawasaki's condition (the
//! product of the reflections round a vertex is the identity exactly when
//! the alternating sum of the sector angles is 0, for an even number of
//! creases: Demaine and O'Rourke 2007, §12.2.1).

use super::{Angle, Arrangement, FoldError, Folded, lift};
use craft3d_math::{Bbox3, Transform, Vec3};
use std::collections::VecDeque;
use std::f64::consts::PI;

/// The half-turn about the line through `a` along the unit `d` in the
/// plane z = 0: the mirror across the vertical plane through the line,
/// then the mirror across z = 0.
fn half_turn(a: Vec3, d: Vec3) -> Option<Transform> {
    let across = Transform::mirror(a, Vec3::Z.cross(d))?;
    let flip = Transform::mirror(Vec3::ZERO, Vec3::Z)?;
    Some(across.then(&flip))
}

/// The arrangement folded flat, keeping facet `fixed` in place (and the
/// first facet of every other piece of a cut sheet): every hinge folds by
/// 180 degrees its kind's way. `angles` (one per hinge) say which way:
/// a given angle by its sign (its size must be 180 degrees, which the
/// caller checks), a solved one by its kind's sign. Fails, naming the
/// crease, when a loop of facets does not close by more than `tol`
/// (`NotFlat`: Kawasaki's condition fails at a vertex on it).
pub fn fold_flat(
    arr: &Arrangement,
    angles: &[Angle],
    fixed: usize,
    tol: f64,
) -> Result<Folded, FoldError> {
    if !(tol.is_finite() && tol > 0.0) {
        return Err(FoldError::Tolerance);
    }
    let n = arr.facets.len();
    if angles.len() != arr.hinges.len() || fixed >= n {
        return Err(FoldError::Broken {
            reason: "the angles do not match the creases",
        });
    }
    let box_ = Bbox3::from_points(arr.vertices.iter().map(|&p| lift(p)));
    let centre = box_.center().unwrap_or(Vec3::ZERO);
    // Per hinge: its reflection about the middle of the box, its ends and
    // the side point its loop is sampled at, and its fold angle.
    let mut turns = Vec::with_capacity(arr.hinges.len());
    let mut folded_angles = Vec::with_capacity(arr.hinges.len());
    for (k, h) in arr.hinges.iter().enumerate() {
        let e = arr.edges.get(h.edge).ok_or(FoldError::Broken {
            reason: "a hinge has no edge",
        })?;
        let (Some(a), Some(b)) = (arr.point(e.from), arr.point(e.to)) else {
            return Err(FoldError::Broken {
                reason: "an edge has no vertex",
            });
        };
        let (a, b) = (lift(a) - centre, lift(b) - centre);
        let d = (b - a)
            .normalized()
            .ok_or(FoldError::TooShort { id: e.source })?;
        let turn = half_turn(a, d).ok_or(FoldError::NotFinite { id: e.source })?;
        let side = a + Vec3::Z.cross(d) * a.distance(b);
        turns.push((turn, [a, b, side], e.source));
        let sign = match angles.get(k) {
            Some(Angle::Given(v)) if *v < 0.0 => -1.0,
            Some(Angle::Given(v)) if *v > 0.0 => 1.0,
            Some(Angle::Free { sign }) if *sign != 0.0 => sign.signum(),
            _ => {
                return Err(FoldError::AngleRange { id: e.source });
            }
        };
        folded_angles.push(sign * PI);
    }

    // The forest, breadth first from the fixed facet, then from the first
    // facet of every other piece.
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (k, h) in arr.hinges.iter().enumerate() {
        if let Some(list) = adj.get_mut(h.left) {
            list.push((k, h.right));
        }
        if let Some(list) = adj.get_mut(h.right) {
            list.push((k, h.left));
        }
    }
    let mut placed: Vec<Option<Transform>> = vec![None; n];
    let mut tree = vec![false; arr.hinges.len()];
    let mut piece = vec![0usize; n];
    let mut roots = Vec::new();
    for root in std::iter::once(fixed).chain(0..n) {
        if placed.get(root).is_none_or(Option::is_some) {
            continue;
        }
        let index = roots.len();
        roots.push(root);
        if let Some(slot) = placed.get_mut(root) {
            *slot = Some(Transform::IDENTITY);
        }
        let mut queue = VecDeque::from([root]);
        while let Some(f) = queue.pop_front() {
            if let Some(p) = piece.get_mut(f) {
                *p = index;
            }
            let Some(here) = placed.get(f).copied().flatten() else {
                continue;
            };
            for &(k, g) in adj.get(f).into_iter().flatten() {
                if placed.get(g).is_none_or(Option::is_some) {
                    continue;
                }
                let Some((turn, _, _)) = turns.get(k) else {
                    continue;
                };
                if let Some(slot) = placed.get_mut(g) {
                    *slot = Some(turn.then(&here));
                }
                if let Some(t) = tree.get_mut(k) {
                    *t = true;
                }
                queue.push_back(g);
            }
        }
    }

    // Every other hinge closes a loop.
    let mut gap = 0.0f64;
    let mut worst: Option<(f64, u64)> = None;
    for (k, h) in arr.hinges.iter().enumerate() {
        if tree.get(k).copied().unwrap_or(true) {
            continue;
        }
        let (Some(Some(left)), Some(Some(right)), Some((turn, points, id))) =
            (placed.get(h.left), placed.get(h.right), turns.get(k))
        else {
            return Err(FoldError::Broken {
                reason: "a facet was not placed",
            });
        };
        let via = turn.then(left);
        let off = points
            .iter()
            .map(|&p| via.point(p).distance(right.point(p)))
            .fold(0.0, f64::max);
        if off.is_nan() {
            return Err(FoldError::NotFinite { id: *id });
        }
        gap = gap.max(off);
        if off > tol && worst.is_none_or(|(w, _)| off > w) {
            worst = Some((off, *id));
        }
    }
    if let Some((gap, id)) = worst {
        return Err(FoldError::NotFlat { id, gap });
    }
    let (from, to) = (
        Transform::translation(-centre),
        Transform::translation(centre),
    );
    let mut transforms = Vec::with_capacity(n);
    for t in placed {
        let t = t.ok_or(FoldError::Broken {
            reason: "a facet was not placed",
        })?;
        transforms.push(from.then(&t).then(&to));
    }
    Ok(Folded {
        transforms,
        angles: folded_angles,
        gap,
        roots,
        piece,
    })
}

/// Whether `angles` (one per hinge) fold the sheet flat: true when at
/// least one angle is given and every given one is within `angular` of
/// 180 degrees (the solved creases then fold by 180 degrees too, their
/// kinds' way: the flat-folded state is the limit of the rigid folds of a
/// flat-foldable pattern, where every crease reaches 180 degrees at once);
/// false when none is. A given angle of 180 degrees beside one that is
/// not is `MixedFlat`, naming the crease: two facets folded onto each
/// other are layered only in the flat-folded state (layers where a stack
/// folds on by other angles are owed).
pub fn folds_flat(arr: &Arrangement, angles: &[Angle], angular: f64) -> Result<bool, FoldError> {
    let mut flat = None;
    let mut other = false;
    for (k, a) in angles.iter().enumerate() {
        if let Angle::Given(v) = a {
            if (v.abs() - PI).abs() <= angular {
                if flat.is_none() {
                    let id = arr
                        .hinges
                        .get(k)
                        .and_then(|h| arr.edges.get(h.edge))
                        .map_or(0, |e| e.source);
                    flat = Some(id);
                }
            } else {
                other = true;
            }
        }
    }
    match (flat, other) {
        (Some(id), true) => Err(FoldError::MixedFlat { id }),
        (Some(_), false) => Ok(true),
        (None, _) => Ok(false),
    }
}
