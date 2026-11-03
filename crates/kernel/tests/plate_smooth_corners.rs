//! The plate's corner where two square rims run on smoothly into each
//! other (docs/design/plates.md › *Corners*; found 2026-11-03): a hole of a
//! quarter, a half and a quarter circle in a sheet-metal part's flat,
//! plain and on a jog's far side (a flat folded away from the pattern's
//! plane), keeps the volume of the plate less the disc at 1e-3, 1 and 1e6.
//! Before the fix the corner between two rims on one cylinder was solved
//! on coinciding surfaces, drifted along their common line, and the edges
//! went round the wrong side of the circle: a body the checker passes,
//! 2 % short of its volume.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{Closing, Direction, JogSpec, Spec, build};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn plate(k: f64, extra: Vec<SketchCurve>) -> Profile {
    let c = [[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [0.0, 40.0]];
    let mut curves: Vec<SketchCurve> = (0..4)
        .map(|i| SketchCurve {
            id: i as u64 + 1,
            curve: Curve2::Line {
                start: [c[i][0] * k, c[i][1] * k],
                end: [c[(i + 1) % 4][0] * k, c[(i + 1) % 4][1] * k],
            },
        })
        .collect();
    curves.extend(extra);
    detect_profiles(&curves, &tolerance(k)).unwrap().remove(0)
}

fn spec(base: Profile, k: f64, jog: bool) -> Spec {
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: 1.0 * k,
        flanges: Vec::new(),
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs: if jog {
            vec![JogSpec {
                line: [[50.0 * k, -5.0 * k], [50.0 * k, 65.0 * k]],
                offset: 8.0 * k,
                angle: PI / 3.0,
                direction: Direction::Up,
                radius: 1.0 * k,
                k: 0.5,
                ids: [1, 2],
            }]
        } else {
            Vec::new()
        },
        ids: Vec::new(),
    }
}

#[test]
fn a_hole_of_arcs_of_one_circle_keeps_its_volume() {
    for k in SCALES {
        let tol = tolerance(k);
        for jog in [false, true] {
            for cx in [30.0, 75.0] {
                let c = [cx * k, 20.0 * k];
                let p = |deg: f64| {
                    let a = deg.to_radians();
                    [c[0] + 5.0 * k * a.cos(), c[1] + 5.0 * k * a.sin()]
                };
                let arc = |id: u64, from: f64, to: f64| SketchCurve {
                    id,
                    curve: Curve2::Arc {
                        center: c,
                        start: p(from),
                        end: p(to),
                    },
                };
                let holed = plate(
                    k,
                    vec![
                        arc(9, 0.0, 90.0),
                        arc(10, 90.0, 270.0),
                        arc(11, 270.0, 360.0),
                    ],
                );
                assert_eq!(holed.holes.len(), 1);
                let part = build(&spec(holed, k, jog), &tol).unwrap();
                let defects = check(&part.solid.body, &tol);
                assert!(defects.is_empty(), "{defects:?}");
                let plain = build(&spec(plate(k, Vec::new()), k, jog), &tol).unwrap();
                let v = mass_properties(&plain.solid.body).unwrap().volume - PI * 25.0 * k * k * k;
                let got = mass_properties(&part.solid.body).unwrap().volume;
                assert!(
                    (got - v).abs() <= 1e-9 * v,
                    "jog {jog}, centre {cx} at {k}: {got} against {v} ({:e})",
                    (got - v) / v
                );
            }
        }
    }
}
