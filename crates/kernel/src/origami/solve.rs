//! Rigid folding: each facet moves as a rigid body, turned about the
//! creases it shares with its neighbours by their fold angles.
//!
//! The facets and fold creases form a graph; a breadth-first forest from
//! the fixed facet (and from the first facet of each other piece, which
//! stays where it is) places every facet by the product of the rotations
//! along its path: `T_child = T_parent ∘ R(crease, ±ρ)`, each rotation
//! about the crease as it lies in the flat sheet. Every crease that is not
//! in the forest closes a loop, and holds only when the two facets it
//! joins agree: `T_right = T_left ∘ R(crease, −ρ)`. That is the
//! loop-closure condition of rigid origami (Belcastro and Hull's
//! condition, Demaine and O'Rourke 2007, §13.1), here for every
//! fundamental cycle of the graph, so it holds for any sheet, with holes,
//! cuts and pieces. It is written as three points of the right facet that
//! must land in the same place by both ways round (nine numbers per loop,
//! lengths, zero when the loop closes), and the unknown angles are found by
//! Levenberg–Marquardt on them, with the exact derivative: turning crease
//! `k` moves a point `X` beyond it by `s D × (X − P)` per radian, `P` and
//! `D` the crease's place and direction as its parent facet has it. The
//! given angles are approached from the flat sheet in steps (continuation),
//! each step starting from the last, so the fold stays on the branch it
//! started on; the first guess turns every unknown crease the way its kind
//! says. At the end the solution must close within the tolerance, every
//! solved crease must turn the way its kind says, and the angles must be
//! determined: the derivative's columns, scaled to unit length, have full
//! rank (a pivoted elimination; a pivot below 1e-10 leaves its crease
//! free).

use super::rigid::lift;
use super::{Arrangement, FoldError, MAX_FREE, Rigid};
use craft3d_math::Vec3;
use std::collections::{BTreeMap, VecDeque};
use std::f64::consts::PI;

/// The fold angle of one hinge: signed, valley positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Angle {
    /// Given, in radians.
    Given(f64),
    /// Solved for; `sign` is +1 for a valley and −1 for a mountain.
    Free { sign: f64 },
}

/// A folded sheet: where each facet went, and the angle of every hinge.
#[derive(Clone, Debug, PartialEq)]
pub struct Folded {
    /// Per facet, from its flat place to its folded one, in the sheet's
    /// plane coordinates.
    pub transforms: Vec<Rigid>,
    /// Per hinge, the fold angle (valley positive), in radians.
    pub angles: Vec<f64>,
    /// The largest distance by which a loop fails to close.
    pub gap: f64,
    /// The facets that stay in place: the fixed one, then one per other
    /// piece of the sheet.
    pub roots: Vec<usize>,
}

/// Steps from the flat sheet to the given angles: one per this many
/// radians of the largest given angle (10 degrees).
const STEP: f64 = PI / 18.0;
const MAX_STEPS: usize = 18;
const MAX_ITERATIONS: usize = 60;
/// How far the first step's guesses turn the unknown creases, in units of
/// the largest given angle.
const FIRST_GUESSES: [f64; 4] = [1.0, 3.0, 0.3, 10.0];
/// A pivot of the scaled normal equations below this leaves its crease
/// undetermined.
const RANK_PIVOT: f64 = 1e-10;

struct Forest {
    /// Per facet: the hinge to its parent and the parent, `None` for a
    /// root.
    parent: Vec<Option<(usize, usize)>>,
    /// Facets in the order they were reached.
    order: Vec<usize>,
    depth: Vec<usize>,
    /// Per hinge: whether it is in the forest.
    tree: Vec<bool>,
    /// Per hinge in the forest: −1 when its parent facet is on its left
    /// (the child turns by −ρ), +1 when on its right.
    sign: Vec<f64>,
    roots: Vec<usize>,
}

fn forest(arr: &Arrangement, fixed: usize) -> Forest {
    let nf = arr.facets.len();
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nf];
    for (k, h) in arr.hinges.iter().enumerate() {
        if let Some(list) = adj.get_mut(h.left) {
            list.push((k, h.right));
        }
        if let Some(list) = adj.get_mut(h.right) {
            list.push((k, h.left));
        }
    }
    let mut parent = vec![None; nf];
    let mut seen = vec![false; nf];
    let mut depth = vec![0usize; nf];
    let mut tree = vec![false; arr.hinges.len()];
    let mut sign = vec![0.0; arr.hinges.len()];
    let mut order = Vec::with_capacity(nf);
    let mut roots = Vec::new();
    let starts = std::iter::once(fixed).chain(0..nf);
    for root in starts {
        if seen.get(root).copied() != Some(false) {
            continue;
        }
        roots.push(root);
        if let Some(s) = seen.get_mut(root) {
            *s = true;
        }
        let mut queue = VecDeque::from([root]);
        while let Some(f) = queue.pop_front() {
            order.push(f);
            let d = depth.get(f).copied().unwrap_or(0);
            for &(k, g) in adj.get(f).into_iter().flatten() {
                if seen.get(g).copied() != Some(false) {
                    continue;
                }
                if let Some(s) = seen.get_mut(g) {
                    *s = true;
                }
                if let Some(slot) = parent.get_mut(g) {
                    *slot = Some((k, f));
                }
                if let Some(slot) = depth.get_mut(g) {
                    *slot = d + 1;
                }
                if let Some(t) = tree.get_mut(k) {
                    *t = true;
                }
                let left = arr.hinges.get(k).is_some_and(|h| h.left == f);
                if let Some(s) = sign.get_mut(k) {
                    *s = if left { -1.0 } else { 1.0 };
                }
                queue.push_back(g);
            }
        }
    }
    Forest {
        parent,
        order,
        depth,
        tree,
        sign,
        roots,
    }
}

/// A crease's line in the flat sheet: a point, the unit direction, and
/// its length.
fn axis(arr: &Arrangement, hinge: usize) -> Option<(Vec3, Vec3, f64)> {
    let h = arr.hinges.get(hinge)?;
    let e = arr.edges.get(h.edge)?;
    let a = lift(arr.point(e.from)?);
    let b = lift(arr.point(e.to)?);
    let len = a.distance(b);
    Some((a, (b - a).normalized()?, len))
}

/// Everything one evaluation gives.
struct State {
    transforms: Vec<Rigid>,
    /// Per hinge in the forest: its line as its parent facet has it.
    world: Vec<(Vec3, Vec3)>,
}

fn place(arr: &Arrangement, forest: &Forest, rho: &[f64]) -> Option<State> {
    let nf = arr.facets.len();
    let mut transforms = vec![Rigid::IDENTITY; nf];
    let mut world = vec![(Vec3::ZERO, Vec3::Z); arr.hinges.len()];
    for &f in &forest.order {
        let Some(Some((k, p))) = forest.parent.get(f).copied() else {
            continue;
        };
        let tp = *transforms.get(p)?;
        let (point, dir, _) = axis(arr, k)?;
        let angle = forest.sign.get(k)? * rho.get(k)?;
        if let Some(slot) = world.get_mut(k) {
            *slot = (tp.point(point), tp.vector(dir));
        }
        let t = tp.after(&Rigid::about(point, dir, angle));
        if let Some(slot) = transforms.get_mut(f) {
            *slot = t;
        }
    }
    Some(State { transforms, world })
}

/// The hinges on the path from facet `f` up to its root.
fn path(forest: &Forest, mut f: usize) -> Vec<usize> {
    let mut out = Vec::new();
    while let Some(Some((k, p))) = forest.parent.get(f).copied() {
        out.push(k);
        f = p;
        if out.len() > forest.parent.len() {
            break;
        }
    }
    out
}

/// One row of three: a gap vector and its derivative per unknown.
struct Row {
    gap: Vec3,
    by: Vec<(usize, Vec3)>,
}

/// The loops' gaps (and derivatives, when `column` gives the unknown of
/// each hinge) at the angles `rho`.
fn rows(
    arr: &Arrangement,
    forest: &Forest,
    loops: &[(usize, Vec<usize>, Vec<usize>)],
    rho: &[f64],
    column: Option<&[Option<usize>]>,
) -> Option<Vec<Row>> {
    let state = place(arr, forest, rho)?;
    let mut out = Vec::with_capacity(loops.len() * 3);
    for (l, path_a, path_b) in loops {
        let h = arr.hinges.get(*l)?;
        let (p, d, len) = axis(arr, *l)?;
        let ta = *state.transforms.get(h.left)?;
        let tb = *state.transforms.get(h.right)?;
        let r = Rigid::about(p, d, -*rho.get(*l)?);
        let side = Vec3::new(-d.y, d.x, 0.0);
        for q in [p, p + d * len, p + side * len] {
            let x1 = ta.point(r.point(q));
            let x2 = tb.point(q);
            let mut by: BTreeMap<usize, Vec3> = BTreeMap::new();
            if let Some(column) = column {
                let mut add = |k: usize, v: Vec3| {
                    if let Some(Some(c)) = column.get(k) {
                        let slot = by.entry(*c).or_insert(Vec3::ZERO);
                        *slot += v;
                    }
                };
                for &k in path_a {
                    if let (Some(&(wp, wd)), Some(&s)) = (state.world.get(k), forest.sign.get(k)) {
                        add(k, wd.cross(x1 - wp) * s);
                    }
                }
                for &k in path_b {
                    if let (Some(&(wp, wd)), Some(&s)) = (state.world.get(k), forest.sign.get(k)) {
                        add(k, wd.cross(x2 - wp) * (-s));
                    }
                }
                add(*l, -(ta.vector(d).cross(x1 - ta.point(p))));
            }
            out.push(Row {
                gap: x1 - x2,
                by: by.into_iter().collect(),
            });
        }
    }
    Some(out)
}

fn largest_gap(rows: &[Row]) -> f64 {
    rows.iter().map(|r| r.gap.length()).fold(0.0, f64::max)
}

fn cost(rows: &[Row]) -> f64 {
    rows.iter().map(|r| r.gap.length_squared()).sum()
}

/// Folds the arrangement by `angles` (one per hinge), keeping facet
/// `fixed` in place. `tol` and `angular` are the document's tolerances.
pub fn fold(
    arr: &Arrangement,
    angles: &[Angle],
    fixed: usize,
    tol: f64,
    angular: f64,
) -> Result<Folded, FoldError> {
    if !(tol.is_finite() && tol > 0.0 && angular.is_finite() && angular > 0.0) {
        return Err(FoldError::Tolerance);
    }
    let nh = arr.hinges.len();
    if angles.len() != nh || fixed >= arr.facets.len() {
        return Err(FoldError::Broken {
            reason: "the angles do not match the creases",
        });
    }
    let source = |k: usize| {
        arr.hinges
            .get(k)
            .and_then(|h| arr.edges.get(h.edge))
            .map_or(0, |e| e.source)
    };
    for (k, a) in angles.iter().enumerate() {
        if let Angle::Given(v) = a
            && !(v.is_finite() && v.abs() <= PI)
        {
            return Err(FoldError::AngleRange { id: source(k) });
        }
    }
    let forest = forest(arr, fixed);

    // The loops: each hinge outside the forest, with the paths of its two
    // facets; the forest's hinges that lie on some loop.
    let mut loops = Vec::new();
    let mut on_loop = vec![false; nh];
    for (k, h) in arr.hinges.iter().enumerate() {
        if forest.tree.get(k).copied().unwrap_or(false) {
            continue;
        }
        let pa = path(&forest, h.left);
        let pb = path(&forest, h.right);
        // Below the common ancestor the two paths are the loop.
        let (mut a, mut b) = (h.left, h.right);
        let mut guard = 0usize;
        while a != b && guard <= 2 * arr.facets.len() {
            guard += 1;
            let (da, db) = (
                forest.depth.get(a).copied().unwrap_or(0),
                forest.depth.get(b).copied().unwrap_or(0),
            );
            let up = |f: usize| forest.parent.get(f).copied().flatten();
            if da >= db {
                let Some((kk, p)) = up(a) else { break };
                if let Some(m) = on_loop.get_mut(kk) {
                    *m = true;
                }
                a = p;
            } else {
                let Some((kk, p)) = up(b) else { break };
                if let Some(m) = on_loop.get_mut(kk) {
                    *m = true;
                }
                b = p;
            }
        }
        if let Some(m) = on_loop.get_mut(k) {
            *m = true;
        }
        loops.push((k, pa, pb));
    }

    // The unknowns.
    let mut column: Vec<Option<usize>> = vec![None; nh];
    let mut signs = Vec::new();
    let mut free = Vec::new();
    let mut lonely = Vec::new();
    for (k, a) in angles.iter().enumerate() {
        if let Angle::Free { sign } = a {
            if !on_loop.get(k).copied().unwrap_or(false) {
                lonely.push(source(k));
                continue;
            }
            if let Some(slot) = column.get_mut(k) {
                *slot = Some(free.len());
            }
            free.push(k);
            signs.push(*sign);
        }
    }
    if !lonely.is_empty() {
        lonely.sort_unstable();
        lonely.dedup();
        return Err(FoldError::Undetermined { ids: lonely });
    }
    if free.len() > MAX_FREE {
        return Err(FoldError::TooManyFree { count: free.len() });
    }
    let given = |scale: f64| -> Vec<f64> {
        angles
            .iter()
            .map(|a| match a {
                Angle::Given(v) => v * scale,
                Angle::Free { .. } => 0.0,
            })
            .collect()
    };
    let largest = angles
        .iter()
        .map(|a| match a {
            Angle::Given(v) => v.abs(),
            Angle::Free { .. } => 0.0,
        })
        .fold(0.0, f64::max);
    let span = arr
        .vertices
        .iter()
        .map(|p| p[0].abs().max(p[1].abs()))
        .fold(0.0, f64::max);
    let goal = (tol * 1e-3).max(span * 1e-13);
    let broken = || FoldError::Broken {
        reason: "a crease has no length",
    };

    let mut rho = given(1.0);
    let mut x = vec![0.0; free.len()];
    let mut exhausted = false;
    if largest > 0.0 && !free.is_empty() {
        let steps = ((largest / STEP).ceil() as usize).clamp(1, MAX_STEPS);
        let weight = span.max(f64::MIN_POSITIVE);
        let mut previous: Option<Vec<f64>> = None;
        for step in 1..=steps {
            let scale = step as f64 / steps as f64;
            let base = given(scale);
            // The guesses: on the first step the unknown creases turned the
            // way their kinds say, as far as the largest given one, then
            // three and a third and ten times as far, until one closes with
            // every crease turned its kind's way (near the flat sheet the
            // branches of a vertex meet, and the kinds pick one); later
            // steps go on from the last two.
            let guesses: Vec<Vec<f64>> = match &previous {
                None => FIRST_GUESSES
                    .iter()
                    .map(|f| signs.iter().map(|s| s * largest * scale * f).collect())
                    .collect(),
                Some(before) => vec![
                    x.iter()
                        .zip(before)
                        .map(|(now, was)| 2.0 * now - was)
                        .collect(),
                ],
            };
            previous = Some(x.clone());
            let mut best: Option<(f64, Vec<f64>, bool)> = None;
            for guess in guesses {
                let (solved, ran_out) = levenberg(
                    arr,
                    &forest,
                    &loops,
                    &base,
                    (&free, &column, &signs),
                    guess,
                    (goal, weight),
                )
                .ok_or_else(broken)?;
                let mut rho_try = base.clone();
                for (c, &k) in free.iter().enumerate() {
                    if let (Some(slot), Some(&v)) = (rho_try.get_mut(k), solved.get(c)) {
                        *slot = v;
                    }
                }
                let here = rows(arr, &forest, &loops, &rho_try, None).ok_or_else(broken)?;
                let wrong = solved.iter().zip(&signs).any(|(v, s)| s * v < -angular);
                let score = largest_gap(&here);
                let good = score <= goal.max(tol * 1e-2) && !wrong;
                let better = best.as_ref().is_none_or(|(b, ..)| score < *b);
                if good || better {
                    best = Some((score, solved, ran_out));
                }
                if good {
                    break;
                }
            }
            if let Some((_, solved, ran_out)) = best {
                x = solved;
                exhausted = ran_out;
            }
        }
    }
    for (c, &k) in free.iter().enumerate() {
        if let (Some(slot), Some(&v)) = (rho.get_mut(k), x.get(c)) {
            *slot = v;
        }
    }
    let final_rows = rows(arr, &forest, &loops, &rho, Some(&column)).ok_or_else(broken)?;
    let gap = largest_gap(&final_rows);
    if gap > tol {
        if exhausted {
            return Err(FoldError::NotConverged { gap });
        }
        let worst = final_rows
            .chunks(3)
            .zip(&loops)
            .map(|(r, (k, ..))| (largest_gap(r), *k))
            .fold((0.0, 0usize), |a, b| if b.0 > a.0 { b } else { a });
        return Err(FoldError::NotClosing {
            id: source(worst.1),
            gap,
        });
    }
    if largest > 0.0 && !free.is_empty() {
        let loose = undetermined(&final_rows, free.len());
        if !loose.is_empty() {
            let mut ids: Vec<u64> = loose
                .iter()
                .filter_map(|&c| free.get(c))
                .map(|&k| source(k))
                .collect();
            ids.sort_unstable();
            ids.dedup();
            return Err(FoldError::Undetermined { ids });
        }
    }
    // Every solved crease turns the way its kind says, and no further
    // than flat on itself.
    for (c, &k) in free.iter().enumerate() {
        let (Some(&v), Some(&s)) = (rho.get(k), signs.get(c)) else {
            continue;
        };
        let wrapped = v.sin().atan2(v.cos());
        if s * wrapped < -angular || (v.abs() > PI + angular) {
            return Err(FoldError::WrongWay {
                id: source(k),
                angle: wrapped,
            });
        }
        if let Some(slot) = rho.get_mut(k) {
            *slot = wrapped;
        }
    }
    let state = place(arr, &forest, &rho).ok_or_else(broken)?;
    if !state.transforms.iter().all(Rigid::is_finite) {
        return Err(FoldError::NotConverged { gap: f64::INFINITY });
    }
    Ok(Folded {
        transforms: state.transforms,
        angles: rho,
        gap,
        roots: forest.roots,
    })
}

/// Levenberg–Marquardt on the loops' gaps from `x`; the solution and
/// whether the iterations ran out first.
fn levenberg(
    arr: &Arrangement,
    forest: &Forest,
    loops: &[(usize, Vec<usize>, Vec<usize>)],
    base: &[f64],
    (free, column, signs): (&[usize], &[Option<usize>], &[f64]),
    mut x: Vec<f64>,
    (goal, weight): (f64, f64),
) -> Option<(Vec<f64>, bool)> {
    let n = free.len();
    // The crease's kind says which way it turns: an unknown turned the
    // other way costs `weight` (the sheet's size) per radian, as a gap
    // would. Zero at every solution that keeps the kinds, so it only
    // keeps the solve off the branches that do not.
    let penalty = |x: &[f64]| -> f64 {
        x.iter()
            .zip(signs)
            .map(|(v, s)| {
                let wrong = (s * v).min(0.0) * weight;
                wrong * wrong
            })
            .sum()
    };
    let with = |x: &[f64]| -> Vec<f64> {
        let mut rho = base.to_vec();
        for (c, &k) in free.iter().enumerate() {
            if let (Some(slot), Some(&v)) = (rho.get_mut(k), x.get(c)) {
                *slot = v;
            }
        }
        rho
    };
    let mut lambda = 1e-3;
    for _ in 0..MAX_ITERATIONS {
        let now = rows(arr, forest, loops, &with(&x), Some(column))?;
        if largest_gap(&now) <= goal {
            return Some((x, false));
        }
        let c0 = cost(&now) + penalty(&x);
        let (mut gram, mut grad) = normal(&now, n);
        for (i, (v, s)) in x.iter().zip(signs).enumerate() {
            if s * v < 0.0 {
                if let Some(d) = gram.get_mut(i * n + i) {
                    *d += weight * weight;
                }
                if let Some(g) = grad.get_mut(i) {
                    *g += weight * weight * v;
                }
            }
        }
        let mut moved = false;
        for _ in 0..12 {
            let mut a = gram.clone();
            for i in 0..n {
                if let Some(d) = a.get_mut(i * n + i) {
                    *d += lambda * (*d + 1e-12);
                }
            }
            let mut b: Vec<f64> = grad.iter().map(|g| -g).collect();
            if solve_dense(&mut a, &mut b, n).is_some() {
                let trial: Vec<f64> = x.iter().zip(&b).map(|(x, d)| x + d).collect();
                let rows_trial = rows(arr, forest, loops, &with(&trial), None)?;
                if cost(&rows_trial) + penalty(&trial) < c0 {
                    x = trial;
                    lambda = (lambda / 3.0).max(1e-15);
                    moved = true;
                    break;
                }
            }
            lambda *= 4.0;
        }
        if !moved {
            // Stalled: this is as close as the loops come.
            return Some((x, false));
        }
    }
    Some((x, true))
}

/// `JᵀJ` (dense, row-major) and `Jᵀr`.
fn normal(rows: &[Row], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut gram = vec![0.0; n * n];
    let mut grad = vec![0.0; n];
    for row in rows {
        for &(a, va) in &row.by {
            if let Some(g) = grad.get_mut(a) {
                *g += va.dot(row.gap);
            }
            for &(b, vb) in &row.by {
                if let Some(slot) = gram.get_mut(a * n + b) {
                    *slot += va.dot(vb);
                }
            }
        }
    }
    (gram, grad)
}

/// Solves `a x = b` in place (`b` becomes `x`) by Gaussian elimination
/// with partial pivoting; `None` when `a` is singular.
fn solve_dense(a: &mut [f64], b: &mut [f64], n: usize) -> Option<()> {
    for col in 0..n {
        let mut best = col;
        let mut size = 0.0;
        for row in col..n {
            let v = a.get(row * n + col)?.abs();
            if v > size {
                size = v;
                best = row;
            }
        }
        if !(size > 0.0 && size.is_finite()) {
            return None;
        }
        if best != col {
            for k in 0..n {
                a.swap(best * n + k, col * n + k);
            }
            b.swap(best, col);
        }
        let pivot = *a.get(col * n + col)?;
        for row in col + 1..n {
            let factor = a.get(row * n + col)? / pivot;
            if factor == 0.0 {
                continue;
            }
            for k in col..n {
                let v = *a.get(col * n + k)?;
                *a.get_mut(row * n + k)? -= factor * v;
            }
            let v = *b.get(col)?;
            *b.get_mut(row)? -= factor * v;
        }
    }
    for col in (0..n).rev() {
        let mut sum = *b.get(col)?;
        for k in col + 1..n {
            sum -= a.get(col * n + k)? * b.get(k)?;
        }
        *b.get_mut(col)? = sum / a.get(col * n + col)?;
    }
    b.iter().all(|v| v.is_finite()).then_some(())
}

/// The unknowns the derivative leaves free: a symmetric pivoted
/// elimination of the normal equations with their columns scaled to unit
/// length, the columns never pivoted.
fn undetermined(rows: &[Row], n: usize) -> Vec<usize> {
    let (mut gram, _) = normal(rows, n);
    let diag: Vec<f64> = (0..n)
        .map(|i| gram.get(i * n + i).copied().unwrap_or(0.0))
        .collect();
    let mut loose = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let (di, dj) = (
                diag.get(i).copied().unwrap_or(0.0),
                diag.get(j).copied().unwrap_or(0.0),
            );
            if let Some(v) = gram.get_mut(i * n + j) {
                *v = if di > 0.0 && dj > 0.0 {
                    *v / (di * dj).sqrt()
                } else {
                    0.0
                };
            }
        }
    }
    let mut left: Vec<usize> = (0..n).collect();
    while !left.is_empty() {
        let (at, &p) = match left.iter().enumerate().max_by(|a, b| {
            let va = gram.get(a.1 * n + a.1).copied().unwrap_or(0.0);
            let vb = gram.get(b.1 * n + b.1).copied().unwrap_or(0.0);
            va.total_cmp(&vb).then(b.1.cmp(a.1))
        }) {
            Some(found) => found,
            None => break,
        };
        let pivot = gram.get(p * n + p).copied().unwrap_or(0.0);
        if !(pivot > RANK_PIVOT) {
            loose.extend(left.iter().copied());
            break;
        }
        left.remove(at);
        for &i in &left {
            let f = gram.get(i * n + p).copied().unwrap_or(0.0) / pivot;
            for &j in &left {
                let v = gram.get(p * n + j).copied().unwrap_or(0.0);
                if let Some(slot) = gram.get_mut(i * n + j) {
                    *slot -= f * v;
                }
            }
        }
    }
    loose.sort_unstable();
    loose
}
