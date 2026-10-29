//! Block-sparse `L D Lᵀ` factorisation of a symmetric positive
//! semi-definite matrix, with each pivot block eigen-decomposed.
//!
//! The matrix is split into blocks (an instance's six pose unknowns, a
//! mate's variables, or a mate's equations); a block pair is stored when
//! it may be non-zero. The blocks are eliminated in an order fixed up front
//! (minimum degree, or the given order when the order carries meaning), the
//! fill of each elimination computed once, so a factorisation touches only
//! the blocks it must.
//!
//! A pivot block `S` (the Schur complement of the blocks before it) is
//! decomposed as `V Λ Vᵀ`; `D⁻¹` is `V Λ⁺ Vᵀ` with every eigenvalue at or
//! below the threshold taken as zero. For a positive semi-definite matrix
//! `[[A, B], [Bᵀ, C]]` the range of `B` lies in that of `A`, so
//! `rank = rank A + rank(C − Bᵀ A⁺ B)`: the rank is the sum of the pivots'
//! ranks, and each zero eigenvector `v` of a pivot `p` gives a null vector
//! `x` of the whole by `Lᵀ x = e_p v`.

use crate::eigen::symmetric_eigen;
use std::collections::BTreeSet;

/// The arithmetic a factorisation may spend, in multiply-adds.
#[derive(Debug)]
pub(crate) struct Budget {
    used: u64,
    limit: u64,
}

impl Budget {
    pub(crate) fn new(limit: u64) -> Budget {
        Budget { used: 0, limit }
    }

    pub(crate) fn spend(&mut self, work: usize) -> Result<(), String> {
        self.used = self
            .used
            .saturating_add(u64::try_from(work).unwrap_or(u64::MAX));
        if self.used > self.limit {
            Err(format!(
                "more than {} multiply-adds of factorisation",
                self.limit
            ))
        } else {
            Ok(())
        }
    }
}

/// Which order the blocks are eliminated in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ordering {
    /// Minimum degree (fewest scalar neighbours first, ties by index):
    /// little fill, for solving.
    MinimumDegree,
    /// The blocks' own order: for blame, where the later block is the one
    /// that adds nothing.
    Natural,
}

/// The symbolic part: elimination order and the filled block structure.
#[derive(Debug)]
pub(crate) struct Pattern {
    /// Scalar size of each block (block order).
    sizes: Vec<usize>,
    /// Scalar offset of each block in a vector (block order).
    offsets: Vec<usize>,
    /// The block at each elimination position.
    block_at: Vec<usize>,
    /// The position of each block.
    pos_of: Vec<usize>,
    /// Per position, the later positions coupled to it after fill, sorted.
    cols: Vec<Vec<usize>>,
    /// Per position, the value offset of the diagonal block and of each
    /// entry of `cols` (blocks `size(q) × size(p)`, row-major).
    diag_slot: Vec<usize>,
    off_slot: Vec<Vec<usize>>,
    /// Total stored values.
    values: usize,
    /// Total scalar size.
    pub(crate) dim: usize,
}

impl Pattern {
    /// The pattern of a matrix of blocks `sizes` whose possibly non-zero
    /// off-diagonal pairs are `edges` (unordered, block indices).
    pub(crate) fn new(
        sizes: &[usize],
        edges: &BTreeSet<(usize, usize)>,
        ordering: Ordering,
        budget: &mut Budget,
    ) -> Result<Pattern, String> {
        let n = sizes.len();
        let mut adj: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        for &(i, j) in edges {
            if i == j || i >= n || j >= n {
                continue;
            }
            if let Some(s) = adj.get_mut(i) {
                s.insert(j);
            }
            if let Some(s) = adj.get_mut(j) {
                s.insert(i);
            }
        }
        let mut alive = vec![true; n];
        let mut block_at = Vec::with_capacity(n);
        let mut neighbours_at: Vec<Vec<usize>> = Vec::with_capacity(n);
        for step in 0..n {
            let pick = match ordering {
                Ordering::Natural => step,
                Ordering::MinimumDegree => {
                    let mut best: Option<(usize, usize)> = None;
                    for (b, nb) in adj.iter().enumerate() {
                        if !alive.get(b).copied().unwrap_or(false) {
                            continue;
                        }
                        let degree: usize =
                            nb.iter().map(|&q| sizes.get(q).copied().unwrap_or(0)).sum();
                        if best.is_none_or(|(d, _)| degree < d) {
                            best = Some((degree, b));
                        }
                    }
                    best.map_or(step, |(_, b)| b)
                }
            };
            budget.spend(n)?;
            let nb: Vec<usize> = adj
                .get(pick)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();
            // The neighbours form a clique once `pick` is gone.
            budget.spend(nb.len().saturating_mul(nb.len()))?;
            for &u in &nb {
                for &w in &nb {
                    if u != w
                        && let Some(s) = adj.get_mut(u)
                    {
                        s.insert(w);
                    }
                }
                if let Some(s) = adj.get_mut(u) {
                    s.remove(&pick);
                }
            }
            if let Some(a) = alive.get_mut(pick) {
                *a = false;
            }
            block_at.push(pick);
            neighbours_at.push(nb);
        }
        let mut pos_of = vec![0; n];
        for (p, &b) in block_at.iter().enumerate() {
            if let Some(slot) = pos_of.get_mut(b) {
                *slot = p;
            }
        }
        let mut cols = Vec::with_capacity(n);
        for nb in &neighbours_at {
            let mut c: Vec<usize> = nb.iter().filter_map(|&b| pos_of.get(b).copied()).collect();
            c.sort_unstable();
            cols.push(c);
        }
        let mut offsets = Vec::with_capacity(n);
        let mut dim = 0usize;
        for &s in sizes {
            offsets.push(dim);
            dim = dim.checked_add(s).ok_or("matrix too large")?;
        }
        let size_at = |p: usize| {
            block_at
                .get(p)
                .and_then(|&b| sizes.get(b))
                .copied()
                .unwrap_or(0)
        };
        let mut values = 0usize;
        let mut diag_slot = Vec::with_capacity(n);
        let mut off_slot = Vec::with_capacity(n);
        for (p, c) in cols.iter().enumerate() {
            let sp = size_at(p);
            diag_slot.push(values);
            values = values.checked_add(sp * sp).ok_or("matrix too large")?;
            let mut offs = Vec::with_capacity(c.len());
            for &q in c {
                offs.push(values);
                values = values
                    .checked_add(size_at(q) * sp)
                    .ok_or("matrix too large")?;
            }
            off_slot.push(offs);
        }
        budget.spend(values)?;
        Ok(Pattern {
            sizes: sizes.to_vec(),
            offsets,
            block_at,
            pos_of,
            cols,
            diag_slot,
            off_slot,
            values,
            dim,
        })
    }

    fn size_at(&self, p: usize) -> usize {
        self.block_at
            .get(p)
            .and_then(|&b| self.sizes.get(b))
            .copied()
            .unwrap_or(0)
    }

    fn offset_at(&self, p: usize) -> usize {
        self.block_at
            .get(p)
            .and_then(|&b| self.offsets.get(b))
            .copied()
            .unwrap_or(0)
    }

    /// The value offset of the stored block at positions `(q, p)`, `q ≥ p`.
    fn slot(&self, q: usize, p: usize) -> Option<usize> {
        if q == p {
            return self.diag_slot.get(p).copied();
        }
        let c = self.cols.get(p)?;
        let k = c.binary_search(&q).ok()?;
        self.off_slot.get(p)?.get(k).copied()
    }

    /// A zeroed value array for this pattern.
    pub(crate) fn zeros(&self) -> Vec<f64> {
        vec![0.0; self.values]
    }

    /// Adds the `size(i) × size(j)` row-major block `m` at blocks `(i, j)`
    /// (and so its transpose at `(j, i)`). For `i == j` the whole block is
    /// added. A pair outside the pattern is an error.
    pub(crate) fn add(&self, values: &mut [f64], i: usize, j: usize, m: &[f64]) -> Option<()> {
        let pi = *self.pos_of.get(i)?;
        let pj = *self.pos_of.get(j)?;
        let si = *self.sizes.get(i)?;
        let sj = *self.sizes.get(j)?;
        if m.len() != si * sj {
            return None;
        }
        if pi >= pj {
            let base = self.slot(pi, pj)?;
            for r in 0..si {
                for c in 0..sj {
                    *values.get_mut(base + r * sj + c)? += *m.get(r * sj + c)?;
                }
            }
        } else {
            let base = self.slot(pj, pi)?;
            for r in 0..si {
                for c in 0..sj {
                    *values.get_mut(base + c * si + r)? += *m.get(r * sj + c)?;
                }
            }
        }
        Some(())
    }

    /// Adds `value` to every diagonal entry.
    pub(crate) fn add_identity(&self, values: &mut [f64], value: f64) -> Option<()> {
        for p in 0..self.block_at.len() {
            let s = self.size_at(p);
            let base = *self.diag_slot.get(p)?;
            for k in 0..s {
                *values.get_mut(base + k * s + k)? += value;
            }
        }
        Some(())
    }

    /// The largest diagonal entry.
    pub(crate) fn max_diagonal(&self, values: &[f64]) -> f64 {
        let mut best = 0.0f64;
        for p in 0..self.block_at.len() {
            let s = self.size_at(p);
            let Some(&base) = self.diag_slot.get(p) else {
                continue;
            };
            for k in 0..s {
                if let Some(&v) = values.get(base + k * s + k) {
                    best = best.max(v);
                }
            }
        }
        best
    }
}

/// A computed factorisation.
#[derive(Debug)]
pub(crate) struct Factor<'p> {
    pattern: &'p Pattern,
    /// `L` blocks in the off-diagonal slots; the diagonal slots hold `D⁺`.
    values: Vec<f64>,
    /// Zero eigenvectors of pivots: `(position, vector)`.
    nulls: Vec<(usize, Vec<f64>)>,
    /// The sum of the pivots' ranks.
    pub(crate) rank: usize,
}

/// `C = A · Bᵀ` for row-major `A` (`r × k`) and `B` (`c × k`).
fn mul_bt(a: &[f64], b: &[f64], r: usize, c: usize, k: usize, out: &mut [f64]) -> Option<()> {
    for i in 0..r {
        for j in 0..c {
            let mut s = 0.0;
            for t in 0..k {
                s += a.get(i * k + t)? * b.get(j * k + t)?;
            }
            *out.get_mut(i * c + j)? = s;
        }
    }
    Some(())
}

/// Factors the matrix `values` (laid out by `pattern`), consuming it.
/// Pivot eigenvalues at or below `threshold` count as zero.
pub(crate) fn factor<'p>(
    pattern: &'p Pattern,
    mut values: Vec<f64>,
    threshold: f64,
    budget: &mut Budget,
) -> Result<Factor<'p>, String> {
    let n = pattern.block_at.len();
    let mut nulls = Vec::new();
    let mut rank = 0usize;
    for p in 0..n {
        let sp = pattern.size_at(p);
        let dslot = pattern.diag_slot.get(p).copied().ok_or("pattern")?;
        let d = values
            .get(dslot..dslot + sp * sp)
            .ok_or("pattern")?
            .to_vec();
        budget.spend(sp.saturating_pow(3).saturating_mul(8))?;
        let (lams, vecs) = symmetric_eigen(&d, sp).ok_or("a pivot that is not finite")?;
        // D⁺ = V Λ⁺ Vᵀ.
        let mut dinv = vec![0.0; sp * sp];
        for (k, &lam) in lams.iter().enumerate() {
            if lam > threshold {
                rank += 1;
                for i in 0..sp {
                    for j in 0..sp {
                        let vi = vecs.get(i * sp + k).copied().unwrap_or(0.0);
                        let vj = vecs.get(j * sp + k).copied().unwrap_or(0.0);
                        if let Some(e) = dinv.get_mut(i * sp + j) {
                            *e += vi * vj / lam;
                        }
                    }
                }
            } else {
                let v: Vec<f64> = (0..sp)
                    .map(|i| vecs.get(i * sp + k).copied().unwrap_or(0.0))
                    .collect();
                nulls.push((p, v));
            }
        }
        let cols = pattern.cols.get(p).cloned().unwrap_or_default();
        let offs = pattern.off_slot.get(p).cloned().unwrap_or_default();
        // W_q = A[q][p] (original), L_q = W_q D⁺.
        let mut ws: Vec<Vec<f64>> = Vec::with_capacity(cols.len());
        let mut ls: Vec<Vec<f64>> = Vec::with_capacity(cols.len());
        for (&q, &off) in cols.iter().zip(&offs) {
            let sq = pattern.size_at(q);
            let w = values.get(off..off + sq * sp).ok_or("pattern")?.to_vec();
            let mut l = vec![0.0; sq * sp];
            // L = W · D⁺ (D⁺ symmetric, so D⁺ᵀ = D⁺).
            mul_bt(&w, &dinv, sq, sp, sp, &mut l).ok_or("pattern")?;
            budget.spend(sq * sp * sp)?;
            ws.push(w);
            ls.push(l);
        }
        // A[q1][q2] −= L_q1 W_q2ᵀ for q1 ≥ q2.
        for (i1, &q1) in cols.iter().enumerate() {
            let s1 = pattern.size_at(q1);
            for (i2, &q2) in cols.iter().enumerate().take(i1 + 1) {
                let s2 = pattern.size_at(q2);
                let slot = pattern.slot(q1, q2).ok_or("fill outside the pattern")?;
                let (l1, w2) = (ls.get(i1).ok_or("pattern")?, ws.get(i2).ok_or("pattern")?);
                let mut upd = vec![0.0; s1 * s2];
                mul_bt(l1, w2, s1, s2, sp, &mut upd).ok_or("pattern")?;
                budget.spend(s1 * s2 * sp)?;
                for (k, u) in upd.iter().enumerate() {
                    if let Some(e) = values.get_mut(slot + k) {
                        *e -= u;
                    }
                }
            }
        }
        for (l, &off) in ls.iter().zip(&offs) {
            if let Some(dst) = values.get_mut(off..off + l.len()) {
                dst.copy_from_slice(l);
            }
        }
        if let Some(dst) = values.get_mut(dslot..dslot + sp * sp) {
            dst.copy_from_slice(&dinv);
        }
    }
    Ok(Factor {
        pattern,
        values,
        nulls,
        rank,
    })
}

impl Factor<'_> {
    /// `x = A⁺ b` in the factor's sense (exact for a non-singular matrix);
    /// `b` and `x` in block order.
    pub(crate) fn solve(&self, b: &[f64]) -> Option<Vec<f64>> {
        let pat = self.pattern;
        let n = pat.block_at.len();
        if b.len() != pat.dim {
            return None;
        }
        // Into position order.
        let mut y: Vec<Vec<f64>> = (0..n)
            .map(|p| {
                let o = pat.offset_at(p);
                b.get(o..o + pat.size_at(p)).map(<[f64]>::to_vec)
            })
            .collect::<Option<_>>()?;
        // Forward: y_q −= L[q][p] y_p.
        for p in 0..n {
            let sp = pat.size_at(p);
            let yp = y.get(p)?.clone();
            for (&q, &off) in pat.cols.get(p)?.iter().zip(pat.off_slot.get(p)?) {
                let sq = pat.size_at(q);
                let yq = y.get_mut(q)?;
                for r in 0..sq {
                    let mut s = 0.0;
                    for c in 0..sp {
                        s += self.values.get(off + r * sp + c)? * yp.get(c)?;
                    }
                    *yq.get_mut(r)? -= s;
                }
            }
        }
        // Diagonal.
        for p in 0..n {
            let sp = pat.size_at(p);
            let base = *pat.diag_slot.get(p)?;
            let yp = y.get(p)?.clone();
            let out = y.get_mut(p)?;
            for r in 0..sp {
                let mut s = 0.0;
                for c in 0..sp {
                    s += self.values.get(base + r * sp + c)? * yp.get(c)?;
                }
                *out.get_mut(r)? = s;
            }
        }
        self.back(&mut y, n)?;
        let mut x = vec![0.0; pat.dim];
        for (p, yp) in y.iter().enumerate() {
            let o = pat.offset_at(p);
            x.get_mut(o..o + yp.len())?.copy_from_slice(yp);
        }
        Some(x)
    }

    /// Backward substitution `x_p −= Σ L[q][p]ᵀ x_q` over positions below
    /// `upto`, in place.
    fn back(&self, y: &mut [Vec<f64>], upto: usize) -> Option<()> {
        let pat = self.pattern;
        for p in (0..upto).rev() {
            let sp = pat.size_at(p);
            let mut acc = vec![0.0; sp];
            for (&q, &off) in pat.cols.get(p)?.iter().zip(pat.off_slot.get(p)?) {
                let sq = pat.size_at(q);
                let xq = y.get(q)?;
                for r in 0..sq {
                    let xr = *xq.get(r)?;
                    for c in 0..sp {
                        *acc.get_mut(c)? += self.values.get(off + r * sp + c)? * xr;
                    }
                }
            }
            let yp = y.get_mut(p)?;
            for (v, a) in yp.iter_mut().zip(&acc) {
                *v -= a;
            }
        }
        Some(())
    }

    /// The null vectors of the matrix, one per zero pivot direction, in
    /// block order. With the natural ordering, a null vector has no
    /// component on blocks after the pivot it comes from.
    pub(crate) fn null_vectors(&self) -> Option<Vec<Vec<f64>>> {
        let pat = self.pattern;
        let n = pat.block_at.len();
        let mut out = Vec::with_capacity(self.nulls.len());
        for (p, v) in &self.nulls {
            let mut y: Vec<Vec<f64>> = (0..n).map(|q| vec![0.0; pat.size_at(q)]).collect();
            *y.get_mut(*p)? = v.clone();
            self.back(&mut y, *p)?;
            let mut x = vec![0.0; pat.dim];
            for (q, yq) in y.iter().enumerate() {
                let o = pat.offset_at(q);
                x.get_mut(o..o + yq.len())?.copy_from_slice(yq);
            }
            out.push(x);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dense(n: usize, a: &[f64], ordering: Ordering) -> (Pattern, Vec<f64>) {
        let sizes = vec![1; n];
        let mut edges = BTreeSet::new();
        for i in 0..n {
            for j in 0..i {
                if a[i * n + j] != 0.0 {
                    edges.insert((j, i));
                }
            }
        }
        let mut budget = Budget::new(1 << 30);
        let pat = Pattern::new(&sizes, &edges, ordering, &mut budget).unwrap();
        let mut vals = pat.zeros();
        for i in 0..n {
            for j in 0..=i {
                if i == j || a[i * n + j] != 0.0 {
                    pat.add(&mut vals, i, j, &[a[i * n + j]]).unwrap();
                }
            }
        }
        (pat, vals)
    }

    #[test]
    fn a_tridiagonal_system_solves_in_either_order() {
        let a = [4.0, 1.0, 0.0, 1.0, 4.0, 1.0, 0.0, 1.0, 4.0];
        let x = [1.0, -2.0, 3.0];
        let b: Vec<f64> = (0..3)
            .map(|i| (0..3).map(|j| a[i * 3 + j] * x[j]).sum())
            .collect();
        for ord in [Ordering::Natural, Ordering::MinimumDegree] {
            let (pat, vals) = dense(3, &a, ord);
            let mut budget = Budget::new(1 << 30);
            let f = factor(&pat, vals, 0.0, &mut budget).unwrap();
            let got = f.solve(&b).unwrap();
            for i in 0..3 {
                assert!((got[i] - x[i]).abs() < 1e-12);
            }
            assert_eq!(f.rank, 3);
        }
    }

    #[test]
    fn a_singular_matrix_has_its_null_vector() {
        // [[1, 1], [1, 1]]: null vector (1, −1).
        let a = [1.0, 1.0, 1.0, 1.0];
        let (pat, vals) = dense(2, &a, Ordering::Natural);
        let mut budget = Budget::new(1 << 30);
        let f = factor(&pat, vals, 1e-12, &mut budget).unwrap();
        assert_eq!(f.rank, 1);
        let nv = f.null_vectors().unwrap();
        assert_eq!(nv.len(), 1);
        assert!((nv[0][0] + nv[0][1]).abs() < 1e-12 && nv[0][1].abs() > 0.5);
    }
}
