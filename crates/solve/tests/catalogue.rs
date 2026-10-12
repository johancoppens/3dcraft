//! Oracle 6: sketches that solve to coordinates worked out by hand, with
//! hand-counted degrees of freedom (the reasoning is at each case in
//! `common/mod.rs`), at three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{Case, catalogue, options, statuses};
use craft3d_solve::{ConstraintStatus, solve};

/// Solves every case at scale `k` and compares with the hand calculation.
fn check_catalogue(k: f64) {
    let cases = catalogue(k);
    assert!(cases.len() >= 20, "the catalogue has {} cases", cases.len());
    for Case {
        name,
        sketch,
        points,
        radii,
        dof,
    } in cases
    {
        let solution = match solve(&sketch, &options()) {
            Ok(s) => s,
            Err(e) => panic!("{name} at scale {k}: {e}"),
        };
        // Coordinates are compared to 1e-9 of the scale: three orders
        // tighter than the modelling tolerance at scale 1, and far above
        // the rounding of a well-conditioned solve (1e-13).
        let close = 1e-9 * k;
        for (id, [x, y]) in points {
            let got = solution.point(id).unwrap();
            assert!(
                (got[0] - x * k).abs() <= close && (got[1] - y * k).abs() <= close,
                "{name} at scale {k}: point {id:?} is {got:?}, expected {:?}",
                [x * k, y * k]
            );
        }
        for (id, r) in radii {
            let got = solution.radius(id).unwrap();
            assert!(
                (got - r * k).abs() <= close,
                "{name} at scale {k}: radius {id:?} is {got}, expected {}",
                r * k
            );
        }
        assert_eq!(solution.dof, dof, "{name} at scale {k}: degrees of freedom");
        assert_eq!(
            solution.unknowns - solution.rank,
            solution.dof,
            "{name}: dof is unknowns minus rank"
        );
        // None of these sketches has a constraint too many.
        assert!(
            statuses(&solution)
                .iter()
                .all(|s| *s == ConstraintStatus::Satisfied),
            "{name} at scale {k}: {:?}",
            statuses(&solution)
        );
        // Oracle 6: every residual within the tolerance that was passed in.
        let tol = options().tolerance;
        for (i, report) in solution.constraints.iter().enumerate() {
            assert!(
                report.residual <= tol.linear.max(tol.angular),
                "{name} at scale {k}: constraint {i} residual {}",
                report.residual
            );
        }
    }
}

#[test]
fn catalogue_at_scale_one() {
    check_catalogue(1.0);
}

#[test]
fn catalogue_at_scale_one_thousandth() {
    check_catalogue(1e-3);
}

#[test]
fn catalogue_at_scale_one_million() {
    check_catalogue(1e6);
}
