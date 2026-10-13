//! `mass_properties` of a sliver: the common part of a cone set ten
//! tolerances deep into a block under a B-spline roof (the corpus case
//! `near/cone-spline/002`). Its face on the roof is a strip 1e-5 wide and
//! 1.5 long, whose sums agree from one doubling to the next to the
//! rounding of its coordinates over its width and no further: judged by
//! the rounding over its length they never settled, and measuring the
//! body took the seven doublings of a boundary of spline edges on a
//! spline surface, more than ten minutes. It lives here because `topo`
//! cannot see the kernel.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_testkit::trimmed::domed_block;
use craft3d_topo::{check, mass_properties};
use std::f64::consts::PI;

/// What FreeCAD 1.1.0 (OCCT 7.8.1) measures of the common part: the
/// reference of the corpus case, a kernel that shares nothing with ours.
const REFERENCE_VOLUME: f64 = 0.000025896168424455972;
const REFERENCE_AREA: f64 = 5.179299125135435;

/// The sliver is measured, in seconds, and to what the modelling
/// tolerance leaves of it. Its spline edges lie within the tolerance of
/// the faces they bound and not on them, so its faces close up to a few
/// 1e-8 of area and its volume, a slab 1e-5 thick, is known to a few
/// 1e-9: that is how far it is from the reference kernel's, and from
/// what the union (which has no thin face) and the two solids (whose
/// volumes are closed forms) imply, volume and area being additive over
/// a union and what it has in common.
#[test]
fn a_sliver_ten_tolerances_thick_is_measured() {
    let tol = Tolerance::new(1e-6, 1e-9).unwrap();
    let (radius, height) = (1.5, 1.625);
    let cone = primitives::cone(
        &Frame::new(Vec3::new(2.0, 1.625, 0.0), Vec3::Z, Vec3::X).unwrap(),
        radius,
        height,
    )
    .unwrap();
    let frame = Frame::new(
        Vec3::new(2.75, 1.0125000000000002, -1.99999),
        Vec3::Y,
        -Vec3::X,
    )
    .unwrap();
    let dome = domed_block(&frame, 1.5, 2.0, 1.75, true).unwrap();
    let cone_volume = PI * radius * radius * height / 3.0;
    let cone_area = PI * radius * (radius + radius.hypot(height));

    let measure = |op: BooleanOp| {
        let body = boolean(&cone, &dome.body, op, &tol).unwrap().body;
        let defects = check(&body, &tol);
        assert!(defects.is_empty(), "{op:?}: {defects:?}");
        mass_properties(&body).unwrap_or_else(|error| panic!("{op:?}: {error}"))
    };
    let union = measure(BooleanOp::Union);
    let common = measure(BooleanOp::Intersect);
    let rest = measure(BooleanOp::Subtract);

    // Measured: 2.7e-9 and 9.4e-8 from the reference.
    assert!(
        (common.volume - REFERENCE_VOLUME).abs() < 5e-9,
        "volume {:e}",
        common.volume
    );
    assert!(
        (common.area - REFERENCE_AREA).abs() < 2e-7,
        "area {}",
        common.area
    );
    // Measured: 7.0e-8 and 6.8e-8, the tolerance of the curves of the
    // union, which are long.
    let implied = cone_volume + dome.volume - union.volume;
    assert!(
        (common.volume - implied).abs() < 2e-7,
        "volume {:e}, the union implies {implied:e}",
        common.volume
    );
    let implied = cone_area + dome.area - union.area;
    assert!(
        (common.area - implied).abs() < 2e-7,
        "area {}, the union implies {implied}",
        common.area
    );
    // The cone less the sliver has the sliver's thin faces the other way
    // around: measured, the two differ by 4.5e-15.
    let implied = cone_volume - rest.volume;
    assert!(
        (common.volume - implied).abs() < 1e-12,
        "volume {:e}, the rest implies {implied:e}",
        common.volume
    );
    // A slab lies halfway up: its centre is at half its thickness.
    let centroid = common.centroid.unwrap();
    assert!((centroid.z - 5e-6).abs() < 1e-8, "centroid {centroid:?}");
}
