//! Formed louvres (docs/design/sheetmetal.md › *Formed louvres*), at 1e-3,
//! 1 and 1e6: a plate 100 × 60, `T` = 1, a lance from (30, 20) to (70, 20)
//! with a hood 8 wide and 3 high behind it (its left), closed and open,
//! up and down. The hood's section is an S of two arcs of radius
//! `ρ = (w² + h²)/(4h)` each turning `α = 2 atan(h/w)`; the part is the
//! plate less the footprint through the thickness and with the hood:
//!
//! - volume `A T − F T + V_hood`, `F = L w` (+ `π w²/2` closed), `V_hood =
//!   2αρ T L` (+ `π ∫ x dA` over the section closed, Pappus), `L` the
//!   middle (`ℓ − 2w` closed);
//! - the stretch `2αρ/w − 1`;
//! - the lance in the pattern: the front (closed) or the front and both
//!   sides (open); the footprint inside the plate;
//! - refusals by reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::formed::{FormedSpec, HoodEnds, bodies, place};
use craft3d_kernel::sheetmetal::{Closing, Spec, build};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn plate(k: f64) -> Profile {
    let pts = [[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line {
                    start: [a[0] * k, a[1] * k],
                    end: [b[0] * k, b[1] * k],
                },
            }
        })
        .collect();
    detect_profiles(&curves, &tolerance(k)).unwrap().remove(0)
}

fn spec(k: f64) -> Spec {
    Spec {
        base: plate(k),
        plane: Frame::WORLD,
        thickness: k,
        flanges: Vec::new(),
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs: Vec::new(),
        ids: Vec::new(),
    }
}

fn louvre(k: f64, ends: HoodEnds, up: bool) -> FormedSpec {
    FormedSpec {
        lance: [[30.0 * k, 20.0 * k], [70.0 * k, 20.0 * k]],
        width: 8.0 * k,
        height: 3.0 * k,
        ends,
        up,
    }
}

#[test]
fn a_formed_louvre_is_held_to_its_closed_forms() {
    for k in SCALES {
        let tol = tolerance(k);
        let part = build(&spec(k), &tol).unwrap();
        let t = k;
        for ends in [HoodEnds::Closed, HoodEnds::Open] {
            for up in [true, false] {
                let what = format!("scale {k}, {ends:?}, up {up}");
                let placed = place(&part, &[louvre(k, ends, up)], &tol).unwrap();
                let f = &placed[0];
                assert_eq!(f.flat, 0, "{what}");
                let (w, h) = (8.0 * k, 3.0 * k);
                let rho = (w * w + h * h) / (4.0 * h);
                let alpha = 2.0 * (h / w).patan();
                assert!((f.radius - rho).abs() <= 1e-12 * rho, "{what}");
                assert!(
                    (f.stretch() - (2.0 * alpha * rho / w - 1.0)).abs() <= 1e-12,
                    "{what}"
                );
                let l = if ends == HoodEnds::Closed {
                    40.0 * k - 2.0 * w
                } else {
                    40.0 * k
                };
                // The lance's pieces: closed the front, open the U.
                let cut: f64 = f
                    .lance
                    .iter()
                    .map(|s| (s.end[0] - s.start[0]).phypot(s.end[1] - s.start[1]))
                    .sum();
                let expect_cut = if ends == HoodEnds::Closed {
                    40.0 * k
                } else {
                    40.0 * k + 2.0 * w
                };
                assert!(
                    (cut - expect_cut).abs() <= 1e-9 * expect_cut,
                    "{what}: lance {cut}"
                );
                let (prism, hood) = bodies(&part, f, &tol).unwrap();
                let fp = f.footprint_area();
                let fp_exact = if ends == HoodEnds::Closed {
                    l * w + PI * w * w / 2.0
                } else {
                    l * w
                };
                assert!((fp - fp_exact).abs() <= 1e-12 * fp_exact, "{what}");
                let v_prism = mass_properties(&prism).unwrap().volume;
                assert!(
                    (v_prism - fp * t).abs() <= 1e-9 * fp * t,
                    "{what}: prism {v_prism}"
                );
                let v_hood = mass_properties(&hood).unwrap().volume;
                let exact = f.hood_volume(t);
                assert!(
                    (v_hood - exact).abs() <= 1e-8 * exact,
                    "{what}: hood {v_hood} against {exact}"
                );
                let cut_part = boolean(&part.solid.body, &prism, BooleanOp::Subtract, &tol)
                    .unwrap_or_else(|e| panic!("{what}: subtract {e}"));
                let made = boolean(&cut_part.body, &hood, BooleanOp::Union, &tol)
                    .unwrap_or_else(|e| panic!("{what}: union {e}"));
                assert!(check(&made.body, &tol).is_empty(), "{what}: invalid");
                let v = mass_properties(&made.body).unwrap().volume;
                let expected = 100.0 * 60.0 * k * k * t - fp * t + exact;
                assert!(
                    (v - expected).abs() <= 1e-8 * expected,
                    "{what}: part {v} against {expected}"
                );
            }
        }
    }
}
