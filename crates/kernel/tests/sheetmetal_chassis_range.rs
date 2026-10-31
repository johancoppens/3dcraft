//! What the chassis showcase's range check found (docs/design/sheetmetal.md
//! › *Findings and history*, 2026-10-31): a closed mitre corner of walls of
//! a thin sheet with a small radius and K-factor runs both flats on past
//! their bends into the same corner of the flat pattern. The refusal said
//! only that the flanges cross in the flat; it now says that they meet at
//! a closed corner and what helps, and a larger gap does help. At 1e-3, 1
//! and 1e6.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, CornerKind, CornerSpec, Direction, FlangeSpec, LengthFrom, On, Relief, ReliefKind,
    Spec, build,
};
use craft3d_math::{Frame, Tolerance};
use std::f64::consts::FRAC_PI_2;

fn spec(k: f64, gap: f64) -> (Spec, Tolerance) {
    let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
    let pts = [[0.0, 0.0], [100.0, 0.0], [100.0, 80.0], [0.0, 80.0]];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| SketchCurve {
            id: i as u64 + 1,
            curve: Curve2::Line {
                start: [pts[i][0] * k, pts[i][1] * k],
                end: [pts[(i + 1) % 4][0] * k, pts[(i + 1) % 4][1] * k],
            },
        })
        .collect();
    let base = detect_profiles(&curves, &tol).unwrap().remove(0);
    let wall = |edge: u64| FlangeSpec {
        on: On::Base(edge),
        length: 30.0 * k,
        angle: FRAC_PI_2,
        direction: Direction::Up,
        radius: 0.3 * k,
        k: 0.0,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    };
    let spec = Spec {
        base,
        plane: Frame::WORLD,
        thickness: 0.5 * k,
        flanges: vec![wall(1), wall(2)],
        relief: Some(Relief {
            kind: ReliefKind::Round,
            size: 2.0 * k,
        }),
        bend_relief: None,
        corner: Closing::OPEN,
        corners: vec![CornerSpec {
            flanges: [0, 1],
            closing: Closing {
                kind: CornerKind::Miter,
                gap: gap * k,
                over: None,
            },
        }],
        cutouts: Vec::new(),
        ids: Vec::new(),
    };
    (spec, tol)
}

#[test]
fn a_mitre_whose_extensions_meet_in_the_flat_says_so_and_a_larger_gap_builds() {
    for k in [1e-3, 1.0, 1e6] {
        let (tight, tol) = spec(k, 0.3);
        match build(&tight, &tol) {
            Err(KernelError::SheetMetal { reason, .. }) => {
                assert!(reason.contains("overlaps itself"), "{reason}");
                assert!(reason.contains("closed corner (miter)"), "{reason}");
                assert!(reason.contains("larger gap"), "{reason}");
            }
            Err(other) => panic!("scale {k}: {other}"),
            Ok(_) => panic!("scale {k}: built"),
        }
        let (wide, tol) = spec(k, 1.5);
        build(&wide, &tol).unwrap_or_else(|e| panic!("scale {k}: {e}"));
    }
}
