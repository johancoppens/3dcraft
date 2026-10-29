//! Eigenvalues of small symmetric matrices (the pivot blocks of the block
//! factorisation, at most a few dozen rows) by cyclic Jacobi rotations.

/// Sweeps allowed; Jacobi converges quadratically, a handful suffice for
/// the blocks met here, and the cap only stops a matrix of NaNs.
const MAX_SWEEPS: usize = 64;

/// The eigen-decomposition of the symmetric `n × n` row-major `a`: the
/// eigenvalues and the eigenvectors as the columns of a row-major `n × n`
/// matrix, eigenvalues in ascending order (ties in input order, so the
/// result is deterministic). `None` when anything is not finite or the
/// sizes do not match.
pub(crate) fn symmetric_eigen(a: &[f64], n: usize) -> Option<(Vec<f64>, Vec<f64>)> {
    if a.len() != n.checked_mul(n)? || a.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let mut m = a.to_vec();
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        *v.get_mut(i * n + i)? = 1.0;
    }
    let norm: f64 = m.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm == 0.0 {
        return Some((vec![0.0; n], v));
    }
    for _ in 0..MAX_SWEEPS {
        let mut off = 0.0;
        for p in 0..n {
            for q in (p + 1)..n {
                let x = *m.get(p * n + q)?;
                off += x * x;
            }
        }
        // Off-diagonal mass at rounding level of the matrix: diagonal.
        if off.sqrt() <= f64::EPSILON * norm {
            break;
        }
        for p in 0..n {
            for q in (p + 1)..n {
                let apq = *m.get(p * n + q)?;
                if apq == 0.0 {
                    continue;
                }
                let app = *m.get(p * n + p)?;
                let aqq = *m.get(q * n + q)?;
                let theta = (aqq - app) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let akp = *m.get(k * n + p)?;
                    let akq = *m.get(k * n + q)?;
                    *m.get_mut(k * n + p)? = c * akp - s * akq;
                    *m.get_mut(k * n + q)? = s * akp + c * akq;
                }
                for k in 0..n {
                    let apk = *m.get(p * n + k)?;
                    let aqk = *m.get(q * n + k)?;
                    *m.get_mut(p * n + k)? = c * apk - s * aqk;
                    *m.get_mut(q * n + k)? = s * apk + c * aqk;
                }
                for k in 0..n {
                    let vkp = *v.get(k * n + p)?;
                    let vkq = *v.get(k * n + q)?;
                    *v.get_mut(k * n + p)? = c * vkp - s * vkq;
                    *v.get_mut(k * n + q)? = s * vkp + c * vkq;
                }
            }
        }
    }
    let vals: Vec<f64> = (0..n)
        .map(|i| m.get(i * n + i).copied())
        .collect::<Option<_>>()?;
    if vals.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| {
        vals.get(i)
            .partial_cmp(&vals.get(j))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(i.cmp(&j))
    });
    let sorted_vals: Vec<f64> = order
        .iter()
        .map(|&i| vals.get(i).copied())
        .collect::<Option<_>>()?;
    let mut sorted_vecs = vec![0.0; n * n];
    for (new, &old) in order.iter().enumerate() {
        for k in 0..n {
            *sorted_vecs.get_mut(k * n + new)? = *v.get(k * n + old)?;
        }
    }
    Some((sorted_vals, sorted_vecs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_matrix_decomposes() {
        // [[2, 1], [1, 2]]: eigenvalues 1 and 3.
        let (vals, vecs) = symmetric_eigen(&[2.0, 1.0, 1.0, 2.0], 2).unwrap();
        assert!((vals[0] - 1.0).abs() < 1e-14 && (vals[1] - 3.0).abs() < 1e-14);
        // A v = λ v for the first.
        let (x, y) = (vecs[0], vecs[2]);
        assert!((2.0 * x + y - x).abs() < 1e-14);
        assert!((x + 2.0 * y - y).abs() < 1e-14);
    }

    #[test]
    fn non_finite_input_is_refused() {
        assert!(symmetric_eigen(&[f64::NAN, 0.0, 0.0, 1.0], 2).is_none());
        assert!(symmetric_eigen(&[1.0, 0.0, 0.0], 2).is_none());
    }
}
