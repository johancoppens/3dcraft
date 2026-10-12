//! Rank-revealing factorisation of the Jacobian, row by row in input order.
//!
//! The rows (one per scalar equation) are orthogonalised against the rows
//! before them (modified Gram-Schmidt, twice for stability). Each row is
//! first scaled to unit length, so its remainder after projection is the
//! sine of its angle to the span of the earlier rows: below the rank
//! threshold the row is *dependent*, which is exactly "this equation adds
//! nothing to the earlier ones" and makes the later constraint the one to
//! blame. The independent rows give `J_I = L Q` with orthonormal `Q`, from
//! which the minimum-norm solution of `J_I dx = rhs` is `Q^T L^-1 rhs`.
//!
//! Rows and basis vectors are stored sparse: a sketch constraint touches at
//! most eight unknowns and the basis stays sparse for typical sketches.

use crate::MAX_FACTOR_ENTRIES;
use crate::error::SolveError;

/// Basis entries smaller than this (relative to a unit vector) are rounding
/// residue of an exact cancellation; dropping them keeps the basis sparse.
const DROP: f64 = 1e-15;

/// Arithmetic budget of a solve.
pub(crate) struct Work {
    used: u64,
    limit: u64,
}

impl Work {
    pub(crate) fn new(limit: u64) -> Self {
        Work { used: 0, limit }
    }

    fn spend(&mut self, operations: usize) -> Result<(), SolveError> {
        self.used = self
            .used
            .saturating_add(u64::try_from(operations).unwrap_or(u64::MAX));
        if self.used > self.limit {
            Err(SolveError::WorkLimitExceeded { limit: self.limit })
        } else {
            Ok(())
        }
    }
}

/// A sparse matrix in row order.
pub(crate) struct Jacobian {
    cols: usize,
    row_start: Vec<usize>,
    idx: Vec<usize>,
    val: Vec<f64>,
}

impl Jacobian {
    pub(crate) fn new(cols: usize) -> Self {
        Jacobian {
            cols,
            row_start: vec![0],
            idx: Vec::new(),
            val: Vec::new(),
        }
    }

    /// Adds an entry to the row being built. Entries outside the matrix are
    /// ignored; a column may appear once per row.
    pub(crate) fn push(&mut self, col: usize, value: f64) {
        if col < self.cols && value != 0.0 {
            self.idx.push(col);
            self.val.push(value);
        }
    }

    pub(crate) fn end_row(&mut self) {
        self.row_start.push(self.idx.len());
    }

    pub(crate) fn rows(&self) -> usize {
        self.row_start.len().saturating_sub(1)
    }

    pub(crate) fn row(&self, i: usize) -> (&[usize], &[f64]) {
        let range = match (self.row_start.get(i), self.row_start.get(i + 1)) {
            (Some(a), Some(b)) => *a..*b,
            _ => 0..0,
        };
        (
            self.idx.get(range.clone()).unwrap_or(&[]),
            self.val.get(range).unwrap_or(&[]),
        )
    }
}

struct Pivot {
    /// The Jacobian row this basis vector came from.
    row: usize,
    /// Length of that row.
    norm: f64,
    /// What was left of the unit row after projection.
    rho: f64,
    q: (usize, usize),
    c: (usize, usize),
}

pub(crate) struct Factor {
    cols: usize,
    /// Per row: linearly dependent on the earlier rows (false for skipped
    /// rows).
    pub dependent: Vec<bool>,
    pivots: Vec<Pivot>,
    q_idx: Vec<usize>,
    q_val: Vec<f64>,
    /// Per pivot: its unit row's components along the earlier basis vectors.
    c_idx: Vec<usize>,
    c_val: Vec<f64>,
}

fn pair<'a>(idx: &'a [usize], val: &'a [f64], range: (usize, usize)) -> (&'a [usize], &'a [f64]) {
    (
        idx.get(range.0..range.1).unwrap_or(&[]),
        val.get(range.0..range.1).unwrap_or(&[]),
    )
}

impl Factor {
    pub(crate) fn rank(&self) -> usize {
        self.pivots.len()
    }

    /// Factorises the rows of `jac` that are not in `skip`.
    ///
    /// `negligible` gives per row the length at or below which the row is
    /// no gradient at all but the rounding residue of one that vanishes
    /// (the angle of a line with itself): such a row is dependent. Without
    /// it, scaling to unit length would turn the residue into a direction.
    pub(crate) fn new(
        jac: &Jacobian,
        skip: &[bool],
        negligible: &[f64],
        threshold: f64,
        work: &mut Work,
    ) -> Result<Factor, SolveError> {
        let cols = jac.cols;
        let mut f = Factor {
            cols,
            dependent: vec![false; jac.rows()],
            pivots: Vec::new(),
            q_idx: Vec::new(),
            q_val: Vec::new(),
            c_idx: Vec::new(),
            c_val: Vec::new(),
        };
        // The row being reduced, dense, with the list of entries it touched.
        let mut v = vec![0.0f64; cols];
        let mut marked = vec![false; cols];
        let mut touched: Vec<usize> = Vec::new();
        let mut coefficients: Vec<f64> = Vec::new();

        for row in 0..jac.rows() {
            if skip.get(row).copied().unwrap_or(false) {
                continue;
            }
            let (idx, val) = jac.row(row);
            let norm = val.iter().map(|a| a * a).sum::<f64>().sqrt();
            let least = negligible.get(row).copied().unwrap_or(0.0);
            if !(norm > least && norm.is_finite()) {
                if let Some(d) = f.dependent.get_mut(row) {
                    *d = true;
                }
                continue;
            }
            for (j, a) in idx.iter().zip(val) {
                if let (Some(slot), Some(mark)) = (v.get_mut(*j), marked.get_mut(*j)) {
                    *slot += a / norm;
                    if !*mark {
                        *mark = true;
                        touched.push(*j);
                    }
                }
            }

            coefficients.clear();
            coefficients.resize(f.pivots.len(), 0.0);
            for _pass in 0..2 {
                let mut spent = 0usize;
                for (pivot, coefficient) in f.pivots.iter().zip(coefficients.iter_mut()) {
                    let (qi, qv) = pair(&f.q_idx, &f.q_val, pivot.q);
                    spent = spent.saturating_add(qi.len());
                    let dot: f64 = qi
                        .iter()
                        .zip(qv)
                        .map(|(j, q)| v.get(*j).copied().unwrap_or(0.0) * q)
                        .sum();
                    if dot == 0.0 {
                        continue;
                    }
                    *coefficient += dot;
                    for (j, q) in qi.iter().zip(qv) {
                        if let (Some(slot), Some(mark)) = (v.get_mut(*j), marked.get_mut(*j)) {
                            *slot -= dot * q;
                            if !*mark {
                                *mark = true;
                                touched.push(*j);
                            }
                        }
                    }
                }
                work.spend(spent)?;
            }

            let entry = |j: &usize| v.get(*j).copied().unwrap_or(0.0);
            let rho = touched
                .iter()
                .map(|j| entry(j) * entry(j))
                .sum::<f64>()
                .sqrt();
            // A NaN remainder is dependent too.
            if rho.is_nan() || rho < threshold {
                if let Some(d) = f.dependent.get_mut(row) {
                    *d = true;
                }
            } else {
                let q_from = f.q_idx.len();
                // Ascending column order: the result must not depend on the
                // order in which entries were touched.
                touched.sort_unstable();
                for j in &touched {
                    let q = entry(j) / rho;
                    if q.abs() > DROP {
                        f.q_idx.push(*j);
                        f.q_val.push(q);
                    }
                }
                let c_from = f.c_idx.len();
                for (k, c) in coefficients.iter().enumerate() {
                    if *c != 0.0 {
                        f.c_idx.push(k);
                        f.c_val.push(*c);
                    }
                }
                f.pivots.push(Pivot {
                    row,
                    norm,
                    rho,
                    q: (q_from, f.q_idx.len()),
                    c: (c_from, f.c_idx.len()),
                });
                let entries = f.q_idx.len().saturating_add(f.c_idx.len());
                if entries > MAX_FACTOR_ENTRIES {
                    return Err(SolveError::TooLarge {
                        what: "factor entries",
                        count: entries,
                        limit: MAX_FACTOR_ENTRIES,
                    });
                }
            }
            for j in touched.drain(..) {
                if let (Some(slot), Some(mark)) = (v.get_mut(j), marked.get_mut(j)) {
                    *slot = 0.0;
                    *mark = false;
                }
            }
        }
        Ok(f)
    }

    /// The shortest `dx` with `row_i . dx = rhs[i]` for every independent
    /// row. Dependent and skipped rows are ignored.
    pub(crate) fn min_norm(&self, rhs: &[f64]) -> Vec<f64> {
        // Forward substitution through L, one pivot at a time.
        let mut y: Vec<f64> = Vec::with_capacity(self.pivots.len());
        for pivot in &self.pivots {
            let (ci, cv) = pair(&self.c_idx, &self.c_val, pivot.c);
            let known: f64 = ci
                .iter()
                .zip(cv)
                .map(|(k, c)| c * y.get(*k).copied().unwrap_or(0.0))
                .sum();
            let target = rhs.get(pivot.row).copied().unwrap_or(0.0) / pivot.norm;
            y.push((target - known) / pivot.rho);
        }
        let mut dx = vec![0.0; self.cols];
        for (pivot, y) in self.pivots.iter().zip(&y) {
            let (qi, qv) = pair(&self.q_idx, &self.q_val, pivot.q);
            for (j, q) in qi.iter().zip(qv) {
                if let Some(slot) = dx.get_mut(*j) {
                    *slot += y * q;
                }
            }
        }
        dx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(cols: usize, rows: &[&[f64]]) -> Jacobian {
        let mut j = Jacobian::new(cols);
        for row in rows {
            for (col, value) in row.iter().enumerate() {
                j.push(col, *value);
            }
            j.end_row();
        }
        j
    }

    fn factor(j: &Jacobian, threshold: f64) -> Factor {
        Factor::new(j, &[], &[], threshold, &mut Work::new(u64::MAX)).unwrap()
    }

    fn close(a: &[f64], b: &[f64]) -> bool {
        // Small integer data: results are exact to a few ulps (1e-12).
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-12)
    }

    #[test]
    fn minimum_norm_solution_of_one_equation() {
        // x + y = 2: the point of that line nearest the origin is (1, 1).
        let j = matrix(2, &[&[1.0, 1.0]]);
        let f = factor(&j, 1e-8);
        assert_eq!(f.rank(), 1);
        assert!(close(&f.min_norm(&[2.0]), &[1.0, 1.0]));
        // 3x + 4z = 25 in three unknowns: (3, 0, 4).
        let j = matrix(3, &[&[3.0, 0.0, 4.0]]);
        assert!(close(&factor(&j, 1e-8).min_norm(&[25.0]), &[3.0, 0.0, 4.0]));
    }

    #[test]
    fn square_system_is_solved_exactly() {
        // 2x + y = 5, x + 3y = 10 -> x = 1, y = 3.
        let j = matrix(2, &[&[2.0, 1.0], &[1.0, 3.0]]);
        let f = factor(&j, 1e-8);
        assert_eq!(f.rank(), 2);
        assert_eq!(f.dependent, [false, false]);
        assert!(close(&f.min_norm(&[5.0, 10.0]), &[1.0, 3.0]));
    }

    #[test]
    fn two_equations_in_three_unknowns() {
        // x + y + z = 3 and x - y = 0: minimum norm is (1, 1, 1).
        let j = matrix(3, &[&[1.0, 1.0, 1.0], &[1.0, -1.0, 0.0]]);
        assert!(close(
            &factor(&j, 1e-8).min_norm(&[3.0, 0.0]),
            &[1.0, 1.0, 1.0]
        ));
    }

    #[test]
    fn the_later_row_is_the_dependent_one() {
        let a: &[f64] = &[1.0, 2.0, 0.0];
        let b: &[f64] = &[0.0, 1.0, -1.0];
        let sum: &[f64] = &[1.0, 3.0, -1.0];
        let f = factor(&matrix(3, &[a, b, sum]), 1e-8);
        assert_eq!(f.dependent, [false, false, true]);
        assert_eq!(f.rank(), 2);
        let f = factor(&matrix(3, &[sum, a, b]), 1e-8);
        assert_eq!(f.dependent, [false, false, true]);
        let f = factor(&matrix(3, &[a, a, b, &[0.0, 0.0, 0.0]]), 1e-8);
        assert_eq!(f.dependent, [false, true, false, true]);
        assert_eq!(f.rank(), 2);
        // The right-hand sides of dependent rows are ignored: x + 2y = 5 and
        // y - z = 0 remain. Minimise x^2 + y^2 + z^2 with z = y, x = 5 - 2y:
        // -4 (5 - 2y) + 4y = 0, so y = 5/3 and x = 5 - 10/3 = 5/3.
        let dx = f.min_norm(&[5.0, 99.0, 0.0, 99.0]);
        assert!(close(&dx, &[5.0 / 3.0, 5.0 / 3.0, 5.0 / 3.0]));
    }

    #[test]
    fn the_threshold_is_the_sine_of_the_angle_to_the_earlier_rows() {
        // (1, 1e-6) makes an angle of 1e-6 with (1, 0).
        let j = matrix(2, &[&[1.0, 0.0], &[1.0, 1e-6]]);
        assert_eq!(factor(&j, 1e-8).dependent, [false, false]);
        assert_eq!(factor(&j, 1e-4).dependent, [false, true]);
        // Row scale does not matter: the same rows times 1e9 and 1e-9.
        let j = matrix(2, &[&[1e9, 0.0], &[1e-9, 1e-15]]);
        assert_eq!(factor(&j, 1e-8).dependent, [false, false]);
        assert_eq!(factor(&j, 1e-4).dependent, [false, true]);
    }

    #[test]
    fn skipped_rows_take_no_part() {
        let j = matrix(2, &[&[1.0, 0.0], &[1.0, 0.0], &[0.0, 1.0]]);
        let f = Factor::new(
            &j,
            &[true, false, false],
            &[],
            1e-8,
            &mut Work::new(u64::MAX),
        )
        .unwrap();
        assert_eq!(f.dependent, [false, false, false]);
        assert_eq!(f.rank(), 2);
        assert!(close(&f.min_norm(&[7.0, 2.0, 3.0]), &[2.0, 3.0]));
    }

    #[test]
    fn negligible_rows_are_dependent() {
        // The second row is rounding residue; scaled to unit length it
        // would look like an independent direction.
        let j = matrix(2, &[&[1.0, 0.0], &[0.0, 1e-17], &[0.0, 1e-17]]);
        assert_eq!(factor(&j, 1e-8).dependent, [false, false, true]);
        let f = Factor::new(
            &j,
            &[],
            &[1e-10, 1e-10, 1e-18],
            1e-8,
            &mut Work::new(u64::MAX),
        )
        .unwrap();
        assert_eq!(f.dependent, [false, true, false]);
    }

    #[test]
    fn work_is_bounded() {
        let j = matrix(2, &[&[1.0, 1.0], &[1.0, -1.0], &[1.0, 2.0]]);
        let err = Factor::new(&j, &[], &[], 1e-8, &mut Work::new(3)).err();
        assert_eq!(err, Some(SolveError::WorkLimitExceeded { limit: 3 }));
    }

    #[test]
    fn non_finite_rows_are_dependent_not_fatal() {
        let j = matrix(2, &[&[f64::NAN, 1.0], &[f64::INFINITY, 0.0], &[0.0, 1.0]]);
        let f = factor(&j, 1e-8);
        assert_eq!(f.dependent, [true, true, false]);
        assert!(f.min_norm(&[1.0, 1.0, 2.0]).iter().all(|v| v.is_finite()));
    }
}
