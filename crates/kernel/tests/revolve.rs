//! Revolution against Pappus's theorems and against the primitives: every
//! body is valid, and its volume and area from the exact B-rep match the
//! closed forms, at three scales, on tilted planes, about axes that are
//! neither through the origin nor along x or y.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod revolve_support;

use common::{P, catalogue, frames};
use revolve_support::{SWEEPS, assert_pappus, build, clear_axis, opposite, tolerance};

/// The whole profile catalogue (rectangle, L, triangle, circle, plates with
/// holes, slot, D, lens, rounded rectangle, ring, island, two rectangles,
/// …) about axes clear of it: straight up, the same line the other way
/// (the profile is then on the other side), and oblique.
#[test]
fn the_catalogue_revolves_to_its_pappus_volume_and_area() {
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        let tol = tolerance(k);
        for plane in frames(k) {
            for case in catalogue(k) {
                let regions: Vec<(f64, P)> =
                    case.parts.iter().map(|p| (p.area, p.centroid)).collect();
                let up = clear_axis(&case.curves, [0.0, 2.0], 3.0 * k);
                let oblique = clear_axis(&case.curves, [-0.6, -0.3], 0.5 * k);
                for axis in [up, opposite(up), oblique, opposite(oblique)] {
                    for (from, to) in SWEEPS {
                        let what =
                            format!("{} at scale {k}, axis {axis:?}, {from}..{to}", case.name);
                        let result = build(&case.curves, &plane, axis, (from, to), &tol)
                            .unwrap_or_else(|e| panic!("{what}: {e}"));
                        assert_pappus(&result, &case.curves, &regions, axis, to - from, k, &what);
                        count += 1;
                    }
                }
            }
        }
    }
    println!("{count} revolutions of catalogue profiles");
}
