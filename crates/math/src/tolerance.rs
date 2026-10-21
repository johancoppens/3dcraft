//! Modelling tolerance as data (docs/design/plan.md): every predicate
//! takes one, nothing uses a global epsilon.

/// The cap on any vertex's or edge's own tolerance (tolerant entities,
/// `craft3d_topo::Tolerances`) when a document sets none, as a multiple of
/// its linear tolerance: 1 µm at the default 1e-6 mm (the decision of
/// 2026-10-18). The one source for `craft3d_topo` and `craft3d_doc`.
pub const DEFAULT_LOOSEST: f64 = 1000.0;

/// How close counts as the same place or the same direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// Distance below which two points coincide, in model units.
    pub linear: f64,
    /// Angle below which two directions are parallel, in radians.
    pub angular: f64,
}

impl Default for Tolerance {
    /// 1e-6 model units and 1e-9 rad (docs/decisions.md).
    fn default() -> Self {
        Self {
            linear: 1e-6,
            angular: 1e-9,
        }
    }
}

impl Tolerance {
    /// `None` unless both values are positive and finite.
    pub fn new(linear: f64, angular: f64) -> Option<Self> {
        let ok = |v: f64| v.is_finite() && v > 0.0;
        (ok(linear) && ok(angular)).then_some(Self { linear, angular })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_positive_finite_tolerances_exist() {
        assert_eq!(
            Tolerance::new(1e-3, 1e-6),
            Some(Tolerance {
                linear: 1e-3,
                angular: 1e-6
            })
        );
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(Tolerance::new(bad, 1e-9).is_none());
            assert!(Tolerance::new(1e-6, bad).is_none());
        }
        assert_eq!(Tolerance::default().linear, 1e-6);
    }
}
