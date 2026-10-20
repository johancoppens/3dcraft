//! A boolean whose result has lumps too thin to measure: a cone less a
//! cylinder whose rim is tangent to the cone's base circle (the fuzzer's
//! finding 20756595 of 2026-10-20). The cylinder runs along x through the
//! cone's base, its axis a few thousand tolerances under the base's centre
//! and tilted against the base plane by 0.01: the base circle leaves the
//! cylinder along two arcs of 15,000 tolerances, by 11 tolerances at
//! most, and the cone's side just above them by 66, and the cone re-enters
//! the cylinder before it leaves it for good higher up. So the exact
//! difference is three lumps: the cone above the cylinder, and two slivers
//! at the sides of the base, each a few tolerances thick, whose volume
//! (`1e-21` at scale 1e-3) is far below what the tolerance lets the
//! integrals of its faces be off by (the gaps between them along the
//! edges, `1e-16`). The sorting of shells into lumps read the slivers'
//! volumes as negative and refused the body as a cavity outside every
//! lump; now where a shell lies decides, and a volume says something only
//! beyond what it can be off by.
//!
//! The oracles: the result passes the checker and the boolean's own
//! check, falls into three lumps without cavities, only the large shell's
//! volume has a sign to read, and the difference and the common part make
//! up the cone (the identity of sets, within the tolerance band of the
//! operands' areas), at scales from 1e-3 to 1e3.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract};
use craft3d_kernel::boolean::boolean;
use craft3d_kernel::lumps::{lumps, outer_shells};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, check, shell_volumes};

/// The scales these tests run at (the tolerance scales along).
const AT: [f64; 3] = [1e-3, 1.0, 1e3];

/// The cylinder of the finding, times `k`: radius 2.5, along x from
/// x = −10 to 5, its axis at y = −2.5, z = 7.5.
fn rod(k: f64) -> Body {
    let frame = Frame::from_z(Vec3::new(-10.0, -2.5, 7.5) * k, Vec3::X).unwrap();
    primitives::cylinder(&frame, 2.5 * k, 15.0 * k).unwrap()
}

/// The cone of the finding, times `k`: radius 2.5, height 15, its base's
/// centre 7.3e-3 (times `k`, thousands of tolerances) above the
/// cylinder's axis and its own axis leaning by 0.01 in x.
fn cone(k: f64) -> Body {
    let frame = Frame::from_z(
        Vec3::new(-5.0, -2.49267244644295, 7.5) * k,
        Vec3::new(0.01, 1.0, 0.0),
    )
    .unwrap();
    primitives::cone(&frame, 2.5 * k, 15.0 * k).unwrap()
}

#[test]
fn slivers_a_boolean_leaves_as_lumps_of_their_own_are_sorted_by_where_they_lie() {
    for k in AT {
        let tol = tolerance(k);
        let (a, b) = (cone(k), rod(k));
        let what = format!("cone less rod at scale {k}");
        let result = match boolean(&a, &b, Subtract, &tol) {
            Ok(r) => r.body,
            Err(e) => panic!("{what}: {e}"),
        };
        assert_eq!(check(&result, &tol), [], "{what}");
        verified(&a, &b, Subtract, &result, &tol, &what);
        // Three shells: the cone above the cylinder (its side and the
        // cylinder's arch) and two slivers of three faces (a piece of the
        // base, of the side and of the cylinder each).
        let mut sizes: Vec<usize> = result.shells.iter().map(|s| s.faces.len()).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, [2, 3, 3], "{what}: faces per shell");
        // Sorted into three lumps without cavities (refused before the
        // fix: "a cavity that lies in no part of the body").
        let found = match lumps(&result, &tol) {
            Ok(found) => found,
            Err(e) => panic!("{what}: lumps: {e}"),
        };
        assert_eq!(found.len(), 3, "{what}: lumps {found:?}");
        assert!(
            found.iter().all(|l| l.cavities.is_empty()),
            "{what}: cavities {found:?}"
        );
        // Only the large shell's volume has a sign to read: the slivers'
        // are within what the tolerance lets them be off by.
        let volumes = shell_volumes(&result);
        let signs = outer_shells(&result, &volumes, &tol).unwrap();
        for (shell, sign) in result.shells.iter().zip(&signs) {
            if shell.faces.len() == 2 {
                assert_eq!(*sign, Some(true), "{what}: the cone's shell");
            } else {
                assert_eq!(*sign, None, "{what}: a sliver's shell");
            }
        }
        // The difference and the common part make up the cone.
        let common = match boolean(&a, &b, Intersect, &tol) {
            Ok(r) => r.body,
            Err(e) => panic!("{what}: common part: {e}"),
        };
        assert_eq!(check(&common, &tol), [], "{what}: common part");
        let (va, sa) = (mass(&a).volume, mass(&a).area);
        let sb = mass(&b).area;
        let total = mass(&result).volume + mass(&common).volume;
        let band = 4.0 * tol.linear * (sa + sb) + 1e-9 * va;
        assert!(
            (total - va).abs() <= band,
            "{what}: {total} against the cone's {va} (band {band})"
        );
    }
}
