//! The sparse solve of a large linkage (docs/design/origami.md › *A
//! sparse solve*): loops written in their own frame over a short cycle
//! basis, so each loop's rows depend only on the joints round it, and the
//! normal equations factored as a skyline `L D Lᵀ` in reverse
//! Cuthill–McKee order.
//!
//! The dense solve writes every loop in the frame of the fixed body, so a
//! loop's rows depend on every joint on the paths of its two bodies from
//! the root, and the normal equations of a large sheet are dense. Here a
//! loop is a chain of bodies from the left body of its closing joint
//! (placed at the identity) to the right one, and its residual is the
//! closing joint's three sample points carried both ways round: the same
//! lengths as before (a rigid motion of the whole loop does not change
//! them), now depending only on the loop's joints. The loops are a cycle
//! basis of the bodies' graph: the shortest cycle through each joint
//! (breadth-first, without the joint), shortest first, kept when it is
//! independent of those kept over GF(2), and completed by the forest's
//! fundamental cycles, so the basis spans every loop whatever the graph
//! (holes, cuts, pieces). In a crease pattern the shortest cycle through
//! a crease is the fan round one of its vertices, so the rows are local.
//!
//! Reverse Cuthill–McKee (Cuthill and McKee 1969; George and Liu, *Computer
//! Solution of Large Sparse Positive Definite Systems*, 1981, §4.3) orders
//! the unknowns so that the normal equations' nonzeros lie near the
//! diagonal; the skyline (envelope) factorisation keeps per row the
//! entries from its first nonzero to the diagonal, which is where all the
//! fill of `L D Lᵀ` lies (George and Liu §4.2). The rank test is the same
//! factorisation of the unit-scaled normal equations in that order: for a
//! symmetric positive semi-definite matrix a zero pivot in any symmetric
//! order means the column depends on the earlier ones (the Schur
//! complement of a semi-definite matrix is semi-definite, so a zero
//! diagonal there has a zero row), and a pivot below the rank threshold
//! leaves its unknown undetermined, as the dense elimination does.

use super::FoldError;
use super::solve::{BROKEN, Budget, Joint, Linkage, Row};
use craft3d_math::{Transform, Vec3};
use std::collections::{BTreeMap, VecDeque};

/// The most entries the skyline may hold (8·10⁷ bytes per 10⁷): past it
/// the fold needs more memory than allowed and is refused as too much
/// work.
pub const MAX_PROFILE: usize = 20_000_000;

/// One loop as a chain of bodies: from the left body of `closing` (at the
/// identity) along `steps` (joint, the body before, the body after) to its
/// right body.
#[derive(Clone, Debug)]
pub(super) struct Chain {
    pub closing: usize,
    pub steps: Vec<(usize, usize, usize)>,
}

/// The rows of the chains at the angles `rho`, as `solve::rows` writes
/// those of the forest's loops: per loop three sample points, each a gap
/// and its derivative by the columns `column` gives.
pub(super) fn chain_rows(
    link: &Linkage,
    chains: &[Chain],
    rho: &[f64],
    column: Option<&[Option<usize>]>,
    budget: &mut Budget,
) -> Result<Vec<Row>, FoldError> {
    let mut out = Vec::with_capacity(chains.len() * 3);
    let mut work = 0u64;
    for chain in chains {
        let joint = link.joints.get(chain.closing).ok_or(BROKEN)?;
        let r_close = *rho.get(chain.closing).ok_or(BROKEN)?;
        // Place the chain: T_after = T_before ∘ motion, the motion or its
        // inverse as the body before is the joint's left or right one; per
        // step the placement its velocity is taken in, and its sign.
        let mut t = Transform::IDENTITY;
        let mut placed: Vec<(usize, Transform, f64)> = Vec::with_capacity(chain.steps.len());
        for &(k, before, _after) in &chain.steps {
            let j: &Joint = link.joints.get(k).ok_or(BROKEN)?;
            let forward = j.motion(*rho.get(k).ok_or(BROKEN)?).ok_or(BROKEN)?;
            let left_before = j.left == before;
            let motion = if left_before {
                forward
            } else {
                forward.inverse().ok_or(BROKEN)?
            };
            let next = motion.then(&t);
            placed.push((
                k,
                if left_before { t } else { next },
                if left_before { -1.0 } else { 1.0 },
            ));
            t = next;
        }
        work = work.saturating_add(40u64.saturating_mul(chain.steps.len() as u64 + 1));
        let motion = joint.motion(r_close).ok_or(BROKEN)?;
        let (p, d, s, r) = (joint.point, joint.direction, joint.side, joint.reach);
        for q in [p, p + d * r, p + s * r] {
            work = work.saturating_add(60);
            let x1 = motion.point(q);
            let x2 = t.point(q);
            let mut by: BTreeMap<usize, Vec3> = BTreeMap::new();
            if let Some(column) = column {
                work = work.saturating_add(60u64.saturating_mul(1 + chain.steps.len() as u64));
                let mut add = |k: usize, v: Vec3| {
                    if let Some(Some(c)) = column.get(k) {
                        *by.entry(*c).or_insert(Vec3::ZERO) += v;
                    }
                };
                // x2 is carried along the chain: as `solve::rows` takes a
                // point of the right body's path (s = −1).
                for (k, tk, sign) in &placed {
                    if let Some(j) = link.joints.get(*k) {
                        add(
                            *k,
                            j.velocity(tk, x2, rho.get(*k).copied().unwrap_or(0.0)) * *sign,
                        );
                    }
                }
                add(
                    chain.closing,
                    joint.velocity(&Transform::IDENTITY, x1, r_close),
                );
            }
            out.push(Row {
                gap: x1 - x2,
                by: by.into_iter().collect(),
            });
        }
    }
    budget.spend(work)?;
    Ok(out)
}

/// A set of joints over GF(2), as bits.
fn bits(nj: usize, chain: &Chain) -> Vec<u64> {
    let mut v = vec![0u64; nj.div_ceil(64)];
    for k in std::iter::once(chain.closing).chain(chain.steps.iter().map(|s| s.0)) {
        if let Some(w) = v.get_mut(k / 64) {
            *w ^= 1u64 << (k % 64);
        }
    }
    v
}

/// A GF(2) basis kept reduced by lowest set bit.
struct Basis {
    rows: BTreeMap<usize, Vec<u64>>,
}

impl Basis {
    /// Adds `v` when it is independent of the basis; the work it took.
    fn insert(&mut self, mut v: Vec<u64>) -> (bool, u64) {
        let mut work = 0u64;
        loop {
            let Some(p) = v
                .iter()
                .enumerate()
                .find(|(_, w)| **w != 0)
                .map(|(i, w)| i * 64 + w.trailing_zeros() as usize)
            else {
                return (false, work);
            };
            match self.rows.get(&p) {
                Some(b) => {
                    work = work.saturating_add(v.len() as u64);
                    for (x, y) in v.iter_mut().zip(b) {
                        *x ^= *y;
                    }
                }
                None => {
                    self.rows.insert(p, v);
                    return (true, work);
                }
            }
        }
    }
}

/// A cycle basis of the linkage's graph of `wanted` loops: the shortest
/// cycle through each joint, shortest first (ties by joint), each kept when
/// it is independent of those kept, then the fundamental cycles of the
/// forest (`fundamental`, also chains) until there are `wanted`.
pub(super) fn cycle_basis(
    link: &Linkage,
    fundamental: Vec<Chain>,
    budget: &mut Budget,
) -> Result<Vec<Chain>, FoldError> {
    let wanted = fundamental.len();
    let n = link.bodies;
    let nj = link.joints.len();
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (k, j) in link.joints.iter().enumerate() {
        if let Some(list) = adj.get_mut(j.left) {
            list.push((k, j.right));
        }
        if let Some(list) = adj.get_mut(j.right) {
            list.push((k, j.left));
        }
    }
    let mut candidates: Vec<Chain> = Vec::new();
    // Only joints on a loop: those of the fundamental cycles.
    let mut on_loop = vec![false; nj];
    for c in &fundamental {
        for k in std::iter::once(c.closing).chain(c.steps.iter().map(|s| s.0)) {
            if let Some(m) = on_loop.get_mut(k) {
                *m = true;
            }
        }
    }
    let mut from: Vec<Option<(usize, usize)>> = vec![None; n];
    let mut seen = vec![usize::MAX; n];
    for (k, j) in link.joints.iter().enumerate() {
        if !on_loop.get(k).copied().unwrap_or(false) || j.left == j.right {
            continue;
        }
        // Breadth-first from the left body to the right one without `k`.
        budget.spend((n as u64).saturating_add(2 * nj as u64))?;
        let mut queue = VecDeque::from([j.left]);
        if let Some(s) = seen.get_mut(j.left) {
            *s = k;
        }
        if let Some(f) = from.get_mut(j.left) {
            *f = None;
        }
        let mut reached = false;
        while let Some(b) = queue.pop_front() {
            if b == j.right {
                reached = true;
                break;
            }
            for &(kk, g) in adj.get(b).into_iter().flatten() {
                if kk == k || seen.get(g).copied() == Some(k) {
                    continue;
                }
                if let Some(s) = seen.get_mut(g) {
                    *s = k;
                }
                if let Some(f) = from.get_mut(g) {
                    *f = Some((kk, b));
                }
                queue.push_back(g);
            }
        }
        if !reached {
            continue;
        }
        let mut steps = Vec::new();
        let mut at = j.right;
        while let Some(Some((kk, b))) = from.get(at).copied() {
            steps.push((kk, b, at));
            at = b;
            if steps.len() > n {
                return Err(FoldError::Broken {
                    reason: "a path between two bodies does not end",
                });
            }
        }
        steps.reverse();
        candidates.push(Chain { closing: k, steps });
    }
    candidates.sort_by_key(|c| (c.steps.len(), c.closing));
    let mut basis = Basis {
        rows: BTreeMap::new(),
    };
    let mut out = Vec::with_capacity(wanted);
    for c in candidates.into_iter().chain(fundamental) {
        if out.len() >= wanted {
            break;
        }
        let (fresh, work) = basis.insert(bits(nj, &c));
        budget.spend(work.saturating_add(nj as u64 / 64 + 1))?;
        if fresh {
            out.push(c);
        }
    }
    if out.len() != wanted {
        return Err(FoldError::Broken {
            reason: "the loops do not make a cycle basis",
        });
    }
    Ok(out)
}

/// The shape of the normal equations: the unknowns in reverse
/// Cuthill–McKee order and per row of the skyline its first column.
pub(super) struct Pattern {
    /// The new place of each unknown, and the unknown at each place.
    at: Vec<usize>,
    of: Vec<usize>,
    first: Vec<usize>,
    /// Where each row of the skyline starts in the values.
    start: Vec<usize>,
    len: usize,
    /// The multiplications of one factorisation.
    pub flops: u64,
}

impl Pattern {
    /// The pattern of `n` unknowns whose rows couple the columns of each
    /// set in `cliques`.
    pub fn new(
        n: usize,
        cliques: &[Vec<usize>],
        budget: &mut Budget,
    ) -> Result<Pattern, FoldError> {
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut work = 0u64;
        for c in cliques {
            work = work.saturating_add((c.len() as u64).saturating_pow(2));
            for &a in c {
                for &b in c {
                    if a != b
                        && let Some(list) = adj.get_mut(a)
                    {
                        list.push(b);
                    }
                }
            }
        }
        budget.spend(work)?;
        for list in &mut adj {
            list.sort_unstable();
            list.dedup();
        }
        let degree = |v: usize| adj.get(v).map_or(0, Vec::len);
        // Cuthill–McKee per component, from a pseudo-peripheral node (the
        // lowest degree, then the far end of its breadth-first levels).
        let mut order: Vec<usize> = Vec::with_capacity(n);
        let mut placed = vec![false; n];
        let mut by_degree: Vec<usize> = (0..n).collect();
        by_degree.sort_by_key(|&v| (degree(v), v));
        let mut level = vec![usize::MAX; n];
        for &seed in &by_degree {
            if placed.get(seed).copied().unwrap_or(true) {
                continue;
            }
            // Two sweeps to the far end.
            let mut root = seed;
            for _ in 0..2 {
                let mut queue = VecDeque::from([root]);
                let mut touched = vec![root];
                if let Some(l) = level.get_mut(root) {
                    *l = 0;
                }
                let mut last = root;
                while let Some(v) = queue.pop_front() {
                    last = v;
                    let lv = level.get(v).copied().unwrap_or(0);
                    for &w in adj.get(v).into_iter().flatten() {
                        if level.get(w).copied() == Some(usize::MAX)
                            && !placed.get(w).copied().unwrap_or(true)
                        {
                            if let Some(l) = level.get_mut(w) {
                                *l = lv + 1;
                            }
                            touched.push(w);
                            queue.push_back(w);
                        }
                    }
                }
                budget.spend(touched.len() as u64 * 4 + 1)?;
                // The far end: the last level's node of lowest degree.
                let far = level.get(last).copied().unwrap_or(0);
                root = touched
                    .iter()
                    .copied()
                    .filter(|&v| level.get(v).copied() == Some(far))
                    .min_by_key(|&v| (degree(v), v))
                    .unwrap_or(last);
                for v in touched {
                    if let Some(l) = level.get_mut(v) {
                        *l = usize::MAX;
                    }
                }
            }
            let mut queue = VecDeque::from([root]);
            if let Some(p) = placed.get_mut(root) {
                *p = true;
            }
            while let Some(v) = queue.pop_front() {
                order.push(v);
                let mut next: Vec<usize> = adj
                    .get(v)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&w| !placed.get(w).copied().unwrap_or(true))
                    .collect();
                next.sort_by_key(|&w| (degree(w), w));
                for w in next {
                    if let Some(p) = placed.get_mut(w) {
                        *p = true;
                    }
                    queue.push_back(w);
                }
            }
        }
        order.reverse();
        let of = order;
        let mut at = vec![0usize; n];
        for (i, &v) in of.iter().enumerate() {
            if let Some(slot) = at.get_mut(v) {
                *slot = i;
            }
        }
        let first: Vec<usize> = of
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                adj.get(v)
                    .into_iter()
                    .flatten()
                    .filter_map(|&w| at.get(w).copied())
                    .fold(i, usize::min)
            })
            .collect();
        let mut start = Vec::with_capacity(n);
        let mut len = 0usize;
        let mut flops = 0u64;
        for (i, &f) in first.iter().enumerate() {
            start.push(len);
            len = len.saturating_add(i - f + 1);
            for j in f..i {
                let fj = first.get(j).copied().unwrap_or(j);
                flops = flops.saturating_add((j - f.max(fj)) as u64);
            }
            flops = flops.saturating_add((i - f) as u64);
            if len > MAX_PROFILE {
                return Err(FoldError::TooMuchWork);
            }
        }
        budget.spend(len as u64)?;
        Ok(Pattern {
            at,
            of,
            first,
            start,
            len,
            flops,
        })
    }

    fn index(&self, i: usize, j: usize) -> Option<usize> {
        // Row i (new order), column j ≤ i.
        let f = *self.first.get(i)?;
        (j >= f && j <= i).then(|| self.start.get(i).map(|s| s + j - f))?
    }

    /// `JᵀJ` in the skyline (new order) and `Jᵀr` (old order), from the
    /// rows' entries; columns from `n` on are left out.
    pub fn normal(&self, rows: &[Row], n: usize) -> (Vec<f64>, Vec<f64>) {
        let mut vals = vec![0.0; self.len];
        let mut grad = vec![0.0; n];
        for row in rows {
            for &(a, va) in &row.by {
                if a >= n {
                    continue;
                }
                if let Some(g) = grad.get_mut(a) {
                    *g += va.dot(row.gap);
                }
                let Some(&ia) = self.at.get(a) else { continue };
                for &(b, vb) in &row.by {
                    if b >= n {
                        continue;
                    }
                    let Some(&ib) = self.at.get(b) else { continue };
                    if ib <= ia
                        && let Some(slot) = self.index(ia, ib).and_then(|x| vals.get_mut(x))
                    {
                        *slot += va.dot(vb);
                    }
                }
            }
        }
        (vals, grad)
    }

    /// The diagonal entry of unknown `c` (old order).
    pub fn diagonal_mut<'v>(&self, vals: &'v mut [f64], c: usize) -> Option<&'v mut f64> {
        let i = *self.at.get(c)?;
        vals.get_mut(self.index(i, i)?)
    }

    /// Scales the normal equations to unit diagonal: entry `(i, j)` by
    /// `1 / √(dᵢ dⱼ)`, `diag` in the old order (0 where either is not
    /// positive).
    pub fn scale(&self, vals: &mut [f64], diag: &[f64]) {
        for (i, (&f, &s)) in self.first.iter().zip(&self.start).enumerate() {
            let di = self
                .of
                .get(i)
                .and_then(|&c| diag.get(c))
                .copied()
                .unwrap_or(0.0);
            for j in f..=i {
                let dj = self
                    .of
                    .get(j)
                    .and_then(|&c| diag.get(c))
                    .copied()
                    .unwrap_or(0.0);
                if let Some(v) = vals.get_mut(s + j - f) {
                    *v = if di > 0.0 && dj > 0.0 {
                        *v / (di * dj).sqrt()
                    } else {
                        0.0
                    };
                }
            }
        }
    }

    /// Factors `vals` as `L D Lᵀ` in place (`L` below the diagonal, `D` on
    /// it). With `rank` a pivot at most `rank` is a dependent column: its
    /// `D` is 0 and its column of `L` too, and the unknowns (old order) are
    /// returned; without, such a pivot (or one not finite) is `None`.
    pub fn factor(&self, vals: &mut [f64], rank: Option<f64>) -> Option<Vec<usize>> {
        let n = self.of.len();
        let mut loose = Vec::new();
        for i in 0..n {
            let fi = *self.first.get(i)?;
            let si = *self.start.get(i)?;
            for j in fi..i {
                let fj = *self.first.get(j)?;
                let sj = *self.start.get(j)?;
                let dj = *vals.get(sj + j - fj)?;
                let mut sum = *vals.get(si + j - fi)?;
                for k in fi.max(fj)..j {
                    let dk = *vals.get(*self.start.get(k)? + k - *self.first.get(k)?)?;
                    sum -= vals.get(si + k - fi)? * dk * vals.get(sj + k - fj)?;
                }
                *vals.get_mut(si + j - fi)? = if dj == 0.0 { 0.0 } else { sum / dj };
            }
            let mut d = *vals.get(si + i - fi)?;
            for k in fi..i {
                let dk = *vals.get(*self.start.get(k)? + k - *self.first.get(k)?)?;
                let l = *vals.get(si + k - fi)?;
                d -= l * l * dk;
            }
            match rank {
                Some(threshold) => {
                    if d.is_nan() || d <= threshold {
                        d = 0.0;
                        loose.push(*self.of.get(i)?);
                    }
                }
                None => {
                    if !(d.is_finite() && d > 0.0) {
                        return None;
                    }
                }
            }
            *vals.get_mut(si + i - fi)? = d;
        }
        loose.sort_unstable();
        Some(loose)
    }

    /// Solves `L D Lᵀ x = b` with the factored `vals`; `b` and the answer
    /// in the old order.
    pub fn solve(&self, vals: &[f64], b: &[f64]) -> Option<Vec<f64>> {
        let n = self.of.len();
        let mut y: Vec<f64> = self
            .of
            .iter()
            .map(|&c| b.get(c).copied().unwrap_or(0.0))
            .collect();
        for i in 0..n {
            let fi = *self.first.get(i)?;
            let si = *self.start.get(i)?;
            let mut sum = *y.get(i)?;
            for k in fi..i {
                sum -= vals.get(si + k - fi)? * y.get(k)?;
            }
            *y.get_mut(i)? = sum;
        }
        for i in 0..n {
            let d = *vals.get(*self.start.get(i)? + i - *self.first.get(i)?)?;
            let v = y.get_mut(i)?;
            *v = if d == 0.0 { 0.0 } else { *v / d };
        }
        for i in (0..n).rev() {
            let fi = *self.first.get(i)?;
            let si = *self.start.get(i)?;
            let yi = *y.get(i)?;
            for k in fi..i {
                *y.get_mut(k)? -= vals.get(si + k - fi)? * yi;
            }
        }
        let mut x = vec![0.0; n];
        for (i, &c) in self.of.iter().enumerate() {
            *x.get_mut(c)? = *y.get(i)?;
        }
        x.iter().all(|v| v.is_finite()).then_some(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skyline_solves_a_banded_system_and_finds_its_rank() {
        // A path of 6 unknowns, scrambled: the tridiagonal 2, −1 matrix.
        let n = 6;
        let perm = [3usize, 0, 5, 1, 4, 2];
        let cliques: Vec<Vec<usize>> = (0..n - 1).map(|i| vec![perm[i], perm[i + 1]]).collect();
        let mut budget = Budget(0);
        let pattern = Pattern::new(n, &cliques, &mut budget).unwrap();
        // Bandwidth 1 after the ordering.
        assert_eq!(pattern.len, 2 * n - 1);
        let mut vals = vec![0.0; pattern.len];
        for i in 0..n {
            *pattern.diagonal_mut(&mut vals, perm[i]).unwrap() = 2.0;
        }
        for i in 0..n - 1 {
            let (a, b) = (pattern.at[perm[i]], pattern.at[perm[i + 1]]);
            let x = pattern.index(a.max(b), a.min(b)).unwrap();
            vals[x] = -1.0;
        }
        let a = vals.clone();
        assert!(pattern.factor(&mut vals, None).unwrap().is_empty());
        // x = 1, 2, …, 6 along the path: b = A x.
        let mut want = vec![0.0; n];
        for i in 0..n {
            want[perm[i]] = i as f64 + 1.0;
        }
        let mut b = vec![0.0; n];
        for i in 0..n {
            let mut s = 2.0 * want[perm[i]];
            if i > 0 {
                s -= want[perm[i - 1]];
            }
            if i + 1 < n {
                s -= want[perm[i + 1]];
            }
            b[perm[i]] = s;
        }
        let x = pattern.solve(&vals, &b).unwrap();
        for c in 0..n {
            assert!((x[c] - want[c]).abs() < 1e-12, "{x:?} vs {want:?}");
        }
        // The graph Laplacian (diagonal 1 at the ends) has rank n − 1.
        let mut lap = a;
        *pattern.diagonal_mut(&mut lap, perm[0]).unwrap() = 1.0;
        *pattern.diagonal_mut(&mut lap, perm[n - 1]).unwrap() = 1.0;
        let loose = pattern.factor(&mut lap, Some(1e-10)).unwrap();
        assert_eq!(loose.len(), 1);
        // Not positive definite: refused without the rank test.
        let mut lap2 = vec![0.0; pattern.len];
        *pattern.diagonal_mut(&mut lap2, 0).unwrap() = -1.0;
        assert!(pattern.factor(&mut lap2, None).is_none());
    }
}
