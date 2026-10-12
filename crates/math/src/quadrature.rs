//! Gauss–Legendre quadrature, used to integrate over curves and surfaces
//! (mass properties).

/// Largest supported number of nodes.
pub const MAX_NODES: usize = 64;

/// The `n` Gauss–Legendre nodes and weights on `[-1, 1]`, in increasing
/// order: exact for polynomials up to degree `2n - 1`. `n` is clamped to
/// `1..=MAX_NODES`.
///
/// Nodes are the roots of the Legendre polynomial `P_n`, found by Newton's
/// method from the Chebyshev-like initial guess; weights are
/// `2 / ((1 - x²) P_n'(x)²)` (Abramowitz & Stegun 25.4.29).
pub fn gauss_legendre(n: usize) -> Vec<(f64, f64)> {
    let n = n.clamp(1, MAX_NODES);
    let nf = n as f64;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut x = -(std::f64::consts::PI * (i as f64 + 0.75) / (nf + 0.5)).cos();
        let mut derivative = 1.0;
        // Converges quadratically from this guess; the cap is a guard.
        for _ in 0..100 {
            // Recurrence (k + 1) P_{k+1} = (2k + 1) x P_k - k P_{k-1}.
            let (mut p_prev, mut p) = (1.0, x);
            for k in 1..n {
                let kf = k as f64;
                let next = ((2.0 * kf + 1.0) * x * p - kf * p_prev) / (kf + 1.0);
                p_prev = p;
                p = next;
            }
            derivative = nf * (x * p - p_prev) / (x * x - 1.0);
            let step = p / derivative;
            x -= step;
            if step.abs() <= 1e-16 {
                break;
            }
        }
        out.push((x, 2.0 / ((1.0 - x * x) * derivative * derivative)));
    }
    out
}

/// A quadrature rule to reuse across many integrals.
#[derive(Clone, Debug)]
pub struct Rule {
    nodes: Vec<(f64, f64)>,
}

impl Rule {
    pub fn new(n: usize) -> Self {
        Self {
            nodes: gauss_legendre(n),
        }
    }

    /// `(point, weight)` pairs for integrating over `[a, b]`.
    pub fn points(&self, a: f64, b: f64) -> impl Iterator<Item = (f64, f64)> + '_ {
        let (mid, half) = (0.5 * (a + b), 0.5 * (b - a));
        self.nodes
            .iter()
            .map(move |&(x, w)| (mid + half * x, w * half))
    }

    /// The integral of `f` over `[a, b]`, split into `panels` equal parts.
    pub fn integrate(&self, a: f64, b: f64, panels: usize, mut f: impl FnMut(f64) -> f64) -> f64 {
        let panels = panels.max(1);
        let width = (b - a) / panels as f64;
        let mut sum = 0.0;
        for k in 0..panels {
            let start = a + width * k as f64;
            for (x, w) in self.points(start, start + width) {
                sum += w * f(x);
            }
        }
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_and_three_point_rules_match_the_textbook() {
        let two = gauss_legendre(2);
        let s = 1.0 / 3.0_f64.sqrt();
        assert!((two[0].0 + s).abs() < 1e-15 && (two[1].0 - s).abs() < 1e-15);
        assert!((two[0].1 - 1.0).abs() < 1e-15 && (two[1].1 - 1.0).abs() < 1e-15);
        let three = gauss_legendre(3);
        let r = 0.6_f64.sqrt();
        assert!(
            (three[0].0 + r).abs() < 1e-15
                && three[1].0.abs() < 1e-15
                && (three[2].0 - r).abs() < 1e-15
        );
        assert!((three[0].1 - 5.0 / 9.0).abs() < 1e-15 && (three[1].1 - 8.0 / 9.0).abs() < 1e-15);
    }

    #[test]
    fn weights_sum_to_two_and_nodes_are_symmetric() {
        for n in 1..=MAX_NODES {
            let rule = gauss_legendre(n);
            assert_eq!(rule.len(), n);
            assert!(
                (rule.iter().map(|p| p.1).sum::<f64>() - 2.0).abs() < 1e-13,
                "n = {n}"
            );
            for (a, b) in rule.iter().zip(rule.iter().rev()) {
                assert!(
                    (a.0 + b.0).abs() < 1e-14 && (a.1 - b.1).abs() < 1e-14,
                    "n = {n}"
                );
            }
            assert!(rule.windows(2).all(|w| w[0].0 < w[1].0));
        }
        assert_eq!(gauss_legendre(0).len(), 1);
        assert_eq!(gauss_legendre(1000).len(), MAX_NODES);
    }

    #[test]
    fn polynomials_up_to_degree_2n_minus_1_are_exact() {
        for n in [1_usize, 2, 5, 8, 16] {
            let rule = Rule::new(n);
            for degree in 0..2 * n {
                // ∫_0^2 x^d dx = 2^(d+1) / (d+1)
                let exact = 2.0_f64.powi(degree as i32 + 1) / (degree as f64 + 1.0);
                let got = rule.integrate(0.0, 2.0, 1, |x| x.powi(degree as i32));
                assert!(
                    (got - exact).abs() <= 1e-12 * exact,
                    "n = {n}, degree {degree}: {got} vs {exact}"
                );
            }
        }
    }

    #[test]
    fn smooth_functions_converge_fast_and_panels_add_up() {
        let rule = Rule::new(16);
        assert!((rule.integrate(0.0, std::f64::consts::PI, 1, f64::sin) - 2.0).abs() < 1e-14);
        assert!(
            (rule.integrate(0.0, 1.0, 1, f64::exp) - (std::f64::consts::E - 1.0)).abs() < 1e-15
        );
        let whole = rule.integrate(-1.0, 3.0, 1, |x| (3.0 * x).cos());
        let split = rule.integrate(-1.0, 3.0, 7, |x| (3.0 * x).cos());
        let exact = (9.0_f64.sin() + 3.0_f64.sin()) / 3.0;
        assert!((whole - exact).abs() < 1e-12 && (split - exact).abs() < 1e-14);
        // A reversed interval gives the negated integral.
        assert!((rule.integrate(3.0, -1.0, 2, |x| (3.0 * x).cos()) + exact).abs() < 1e-13);
    }
}
