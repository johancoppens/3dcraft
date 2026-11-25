//! The review of the pinch along a curve (random design 20781906;
//! docs/design/booleans.md › *A pinch along a curve*): the same cavity and
//! cutter as `boolean_pinch_along_a_curve.rs`, in the operations and
//! operand orders that test was not written for, and near the move's
//! thresholds.
//!
//! - An intersection that keeps both faces: the slab less the torus
//!   intersected with the block beyond x = 0, which is the same point set
//!   as the cut (three parts that touch along the arcs), in either order.
//! - A union and a cut in the other order (the edges on B).
//! - The edges on the tool of a cut that keeps both faces: a block beyond
//!   x = 0 less the torus within the slab, a void whose wall touches the
//!   block's face along the arcs, a wall that thins to nothing within one
//!   part.
//! - The torus within the slab joined to the cutter by a bridge, so that
//!   the two rest on each other along the arcs and are one part.
//! - The cutter turned about the line where the slab's floor meets the
//!   plane x = 0 (through both places where C crosses the floor), so that
//!   its face crosses the cavity at a small angle there: three parts that
//!   touch only in those two points.
//!
//! Held to the geometry written without the kernel: the point sets are
//! those of `pinch_reduced.py` and the reviewer's own oracle
//! (`plan/pinch-along-a-curve/review/review_oracle.py` in the workshop:
//! the volume of a torus within a box by integrating over the distance
//! from its axis, which gives the builder's numbers to all twelve
//! decimals and the whole torus's 2π²Rr² to 3e-15; the parts by a flood
//! fill of a grid's cells). Each test gathers every case's failure before
//! it fails, so that one run says how each case fares; what `main` gave is
//! said at each test (Check 45594454223, this file on `main` 22f0bb13).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::{
    BooleanOp, BooleanOptions, VerifyOptions, boolean, boolean_with, verify,
};
use craft3d_kernel::primitives;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, check, mass_properties};

/// The torus of random design 20781906 reduced: its axis along x through
/// (y, z) = (CY, CZ), centred at x = 5, major radius 10, minor 5, so that
/// it reaches x = 0 exactly along the circle C.
const CY: f64 = -0.693_073_289_086_241_3;
const CZ: f64 = 3.847_633_424_655_209;
const XC: f64 = 5.0;
const MAJOR: f64 = 10.0;
const MINOR: f64 = 5.0;

type Cuboid = ([f64; 3], [f64; 3]);

/// The slab, the cutter whose face x = 0 holds C, the block beyond that
/// face, and a bridge that overlaps both the cutter and the torus within
/// the slab, away from the arcs.
const SLAB: Cuboid = ([-5.0, -7.5, -5.0], [2.5, 7.5, 2.5]);
const CUTTER: Cuboid = ([-10.0, -20.0, -20.0], [0.0, 20.0, 20.0]);
const BEYOND: Cuboid = ([0.0, -20.0, -20.0], [10.0, 20.0, 20.0]);
const BRIDGE: Cuboid = ([-1.0, -2.0, -4.5], [1.0, 1.0, -3.5]);

/// Volumes in the test's units, from the oracles: the slab less the torus
/// beyond x = 0 (the cut, and the intersection with the block beyond);
/// the block beyond less the torus within the slab; the slab less the
/// torus united with the cutter; the torus within the slab and the bridge
/// united with the cutter.
const CUT: f64 = 143.656_989_641_059;
const BEYOND_LESS_TORUS: f64 = 15_862.406_989_641_058;
const UNION: f64 = 16_143.656_989_641_058;
const BRIDGED: f64 = 16_139.017_634_515_8;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

#[derive(Clone, Copy, Debug)]
struct Case {
    k: f64,
    turned: bool,
}

impl Case {
    fn all() -> Vec<Case> {
        SCALES
            .iter()
            .flat_map(|&k| [false, true].map(|turned| Case { k, turned }))
            .collect()
    }

    fn unit() -> Vec<Case> {
        [false, true].map(|turned| Case { k: 1.0, turned }).to_vec()
    }

    fn turn(self) -> Transform {
        Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
            .unwrap()
            .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * self.k))
    }

    fn put(self, body: Body) -> Body {
        if self.turned {
            body.transformed(&self.turn()).unwrap()
        } else {
            body
        }
    }

    fn tolerance(self) -> Tolerance {
        Tolerance::new(1e-6 * self.k, 1e-9).unwrap()
    }

    fn raw_block(self, (lo, hi): Cuboid) -> Body {
        let frame = Frame::new(Vec3::new(lo[0], lo[1], lo[2]) * self.k, Vec3::Z, Vec3::X).unwrap();
        let [dx, dy, dz] = [0, 1, 2].map(|i| (hi[i] - lo[i]) * self.k);
        primitives::block(&frame, dx, dy, dz).unwrap()
    }

    fn block(self, cuboid: Cuboid) -> Body {
        self.put(self.raw_block(cuboid))
    }

    /// The cutter turned by `angle` about the line x = 0, z = -5 (along
    /// y): its face is the plane x = tan(angle) (z + 5).
    fn tilted_cutter(self, angle: f64) -> Body {
        let turn = Transform::rotation_about(
            Vec3::new(0.0, 0.0, -5.0 * self.k),
            Vec3::new(0.0, 1.0, 0.0),
            angle,
        )
        .unwrap();
        self.put(self.raw_block(CUTTER).transformed(&turn).unwrap())
    }

    fn torus(self) -> Body {
        let frame = Frame::new(Vec3::new(XC, CY, CZ) * self.k, Vec3::X, Vec3::Y).unwrap();
        self.put(primitives::torus(&frame, MAJOR * self.k, MINOR * self.k).unwrap())
    }

    fn local(self, p: Vec3) -> Vec3 {
        let p = if self.turned {
            self.turn().inverse().unwrap().point(p)
        } else {
            p
        };
        p * (1.0 / self.k)
    }

    /// The slab less the torus.
    fn hollowed(self) -> Body {
        boolean(
            &self.block(SLAB),
            &self.torus(),
            Subtract,
            &self.tolerance(),
        )
        .unwrap()
        .body
    }

    /// The torus within the slab.
    fn inside(self) -> Body {
        boolean(
            &self.block(SLAB),
            &self.torus(),
            Intersect,
            &self.tolerance(),
        )
        .unwrap()
        .body
    }
}

/// What the boolean gives, in words, for the record.
fn outcome(case: Case, a: &Body, b: &Body, op: BooleanOp) -> String {
    match boolean(a, b, op, &case.tolerance()) {
        Ok(r) => {
            let volume = mass_properties(&r.body).map_or(f64::NAN, |m| m.volume);
            format!(
                "built: {} shells, {} faces, volume {:e}",
                r.body.shells.len(),
                r.body.faces.len(),
                volume / (case.k * case.k * case.k)
            )
        }
        Err(KernelError::ZeroThickness { at, .. }) => {
            let p = case.local(at);
            let off = ((p.y - CY).phypot(p.z - CZ) - MAJOR).abs();
            format!("zero thickness at {p:?}, {off:e} off C")
        }
        Err(e) => format!("refused: {e}"),
    }
}

/// Whether `p` (in the case's units) is on C, in the slab's section,
/// within twice the tolerance.
fn on_c(p: Vec3) -> bool {
    let near = 2e-6;
    let off = ((p.y - CY).phypot(p.z - CZ) - MAJOR).abs();
    let (lo, hi) = SLAB;
    p.x.abs() <= near
        && off <= near
        && (lo[1] - near..=hi[1] + near).contains(&p.y)
        && (lo[2] - near..=hi[2] + near).contains(&p.z)
}

/// Refused as zero thickness at a point of C; else what is wrong.
fn zero_thickness_on_c(
    case: Case,
    label: &str,
    a: &Body,
    b: &Body,
    op: BooleanOp,
) -> Result<(), String> {
    let said = outcome(case, a, b, op);
    println!("{label} at {case:?}: {said}");
    match boolean(a, b, op, &case.tolerance()) {
        Err(KernelError::ZeroThickness { at, .. }) if on_c(case.local(at)) => Ok(()),
        _ => Err(format!(
            "{label} at {case:?}: {said}, not zero thickness on C"
        )),
    }
}

/// Built, valid, of the volume `expected` (in the test's units) to within
/// the tolerance times its area, in `shells` shells; else what is wrong.
fn built(
    case: Case,
    label: &str,
    (a, b, op): (&Body, &Body, BooleanOp),
    expected: f64,
    shells: usize,
) -> Result<(), String> {
    let tol = case.tolerance();
    let result =
        boolean(a, b, op, &tol).map_err(|e| format!("{label} at {case:?}: refused: {e}"))?;
    let defects = check(&result.body, &tol);
    if !defects.is_empty() {
        return Err(format!("{label} at {case:?}: {defects:?}"));
    }
    let found = verify(a, b, op, &result.body, &tol, &VerifyOptions::default())
        .map_err(|e| format!("{label} at {case:?}: verify: {e}"))?;
    if !found.violations.is_empty() {
        return Err(format!("{label} at {case:?}: {:?}", found.violations));
    }
    let mass = mass_properties(&result.body).map_err(|e| format!("{label}: {e}"))?;
    let want = expected * case.k * case.k * case.k;
    println!(
        "{label} at {case:?}: built, {} shells, volume {:e} against {want:e}",
        result.body.shells.len(),
        mass.volume
    );
    if (mass.volume - want).abs() > tol.linear * mass.area {
        return Err(format!(
            "{label} at {case:?}: volume {:e} against {want:e}, allowed {:e}",
            mass.volume,
            tol.linear * mass.area
        ));
    }
    if result.body.shells.len() != shells {
        return Err(format!(
            "{label} at {case:?}: {} shells, not {shells}",
            result.body.shells.len()
        ));
    }
    Ok(())
}

/// Made without the refusal of zero thickness: its shells and volume (in
/// the test's units).
fn made_anyway(case: Case, label: &str, a: &Body, b: &Body, op: BooleanOp) -> Option<(usize, f64)> {
    let options = BooleanOptions {
        refuse_zero_thickness: false,
        ..BooleanOptions::default()
    };
    match boolean_with(a, b, op, &case.tolerance(), &options) {
        Ok(made) => {
            let volume = mass_properties(&made.body).map_or(f64::NAN, |m| m.volume)
                / (case.k * case.k * case.k);
            let defects = check(&made.body, &case.tolerance());
            println!(
                "{label} at {case:?}, made without the refusal: {} shells, volume {volume:e}, {} defects",
                made.body.shells.len(),
                defects.len()
            );
            Some((made.body.shells.len(), volume))
        }
        Err(e) => {
            println!("{label} at {case:?}, made without the refusal: refused: {e}");
            None
        }
    }
}

/// Every case's failure, together.
fn none_failed(failures: &[String]) {
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ---------------------------------------------------------------------

/// The slab less the torus intersected with the block beyond x = 0, in
/// the order `swap` says: the cut's point set, three parts that touch along
/// the arcs. An intersection keeps the cavity's face (inside the block)
/// and the block's face x = 0 (inside the hollowed slab). `main` built it
/// as one shell of 11 faces (Check 45594454223).
fn intersection(swap: bool) {
    let mut failures = Vec::new();
    for case in Case::all() {
        let hollowed = case.hollowed();
        let beyond = case.block(BEYOND);
        let (a, b, label) = if swap {
            (&beyond, &hollowed, "beyond * hollowed")
        } else {
            (&hollowed, &beyond, "hollowed * beyond")
        };
        failures.extend(zero_thickness_on_c(case, label, a, b, Intersect).err());
        match made_anyway(case, label, a, b, Intersect) {
            Some((3, volume)) if (volume - CUT).abs() <= 1e-6 * 400.0 => {}
            other => failures.push(format!(
                "{label} at {case:?}: made without the refusal {other:?}, not 3 shells of {CUT:e}"
            )),
        }
    }
    none_failed(&failures);
}

#[test]
fn an_intersection_that_keeps_both_faces_is_zero_thickness() {
    intersection(false);
}

#[test]
fn an_intersection_that_keeps_both_faces_is_zero_thickness_the_other_way_round() {
    intersection(true);
}

#[test]
fn the_cutter_and_the_torus_within_the_slab_rest_on_each_other_the_other_way_round() {
    // Two parts that rest on each other along the arcs, the edges on B.
    // `main` refused it too, at a hit 6.3 tolerances off C.
    let mut failures = Vec::new();
    for case in Case::all() {
        let cutter = case.block(CUTTER);
        let inside = case.inside();
        failures
            .extend(zero_thickness_on_c(case, "cutter + inside", &cutter, &inside, Union).err());
    }
    none_failed(&failures);
}

#[test]
fn the_cutter_united_with_the_hollowed_slab_keeps_one_face() {
    // The cavity's face kept, the cutter's face not: the touch divides
    // nothing.
    let mut failures = Vec::new();
    for case in Case::all() {
        let cutter = case.block(CUTTER);
        let a = case.hollowed();
        let ops = (&cutter, &a, Union);
        failures.extend(built(case, "cutter + hollowed", ops, UNION, 1).err());
    }
    none_failed(&failures);
}

#[test]
#[ignore = "owed, as on main: the block's outer shell and the void's touch along the arcs and are refused as two parts (Check 45626434190)"]
fn a_void_whose_wall_touches_the_blocks_face_along_the_arcs_is_one_part() {
    // The block beyond x = 0 less the torus within the slab: the void's
    // wall touches the block's face x = 0 along the arcs, from inside the
    // block; the wall between them thins to nothing there from both sides
    // and the block stays one part (as the hole whose wall touches the
    // plate's side, 2026-10-18: a wall that thins to nothing within one
    // shell is a valid body). The edges that cross C are the tool's (B),
    // and the cut keeps both faces. `main` refused it as zero thickness at
    // a hit 6.3 tolerances off C (its outer shell and the void's touching:
    // made without the refusal, two shells of the set's volume); the
    // branch the same at a point of C, every case (Check 45626434190):
    // the arcs, imprinted, are slits in the block's face that end inside
    // it, and the void stays a shell of its own. Whether such a void is a
    // wall that thins to nothing within one shell (2026-10-18) or two
    // shells that touch (2026-10-15) is the developer's to say.
    let mut failures = Vec::new();
    for case in Case::all() {
        let beyond = case.block(BEYOND);
        let inside = case.inside();
        let label = "beyond - inside";
        println!(
            "{label} at {case:?}: {}",
            outcome(case, &beyond, &inside, Subtract)
        );
        made_anyway(case, label, &beyond, &inside, Subtract);
        let ops = (&beyond, &inside, Subtract);
        failures.extend(built(case, label, ops, BEYOND_LESS_TORUS, 1).err());
    }
    none_failed(&failures);
}

#[test]
fn the_torus_and_the_cutter_joined_by_a_bridge_rest_on_each_other_along_the_arcs() {
    // One part: the bridge overlaps the cutter and the torus within the
    // slab away from the arcs. Along the arcs the union's two faces rest on
    // each other, the space between them thinning to nothing from both
    // sides. Either built, valid and of the set's volume, or refused as
    // zero thickness at a point of C: nothing else. `main` refused it at a
    // hit 6.3 tolerances off C (made without the refusal, one shell of the
    // set's volume).
    let mut failures = Vec::new();
    for case in Case::unit() {
        let tol = case.tolerance();
        let joined = boolean(&case.inside(), &case.block(BRIDGE), Union, &tol)
            .unwrap()
            .body;
        let cutter = case.block(CUTTER);
        for (label, a, b) in [
            ("joined + cutter", &joined, &cutter),
            ("cutter + joined", &cutter, &joined),
        ] {
            let said = outcome(case, a, b, Union);
            println!("{label} at {case:?}: {said}");
            made_anyway(case, label, a, b, Union);
            match boolean(a, b, Union, &tol) {
                Err(KernelError::ZeroThickness { at, .. }) if on_c(case.local(at)) => {}
                Ok(_) => failures.extend(built(case, label, (a, b, Union), BRIDGED, 1).err()),
                _ => failures.push(format!("{label} at {case:?}: {said}")),
            }
        }
    }
    none_failed(&failures);
}

#[test]
fn a_cutter_turned_a_little_about_the_floors_line_leaves_parts_that_touch_in_points() {
    // The cutter's face x = tan(angle) (z + 5) goes through the two places
    // where C crosses the slab's floor and runs into the cavity everywhere
    // else above the floor: the middle and the two corners are apart
    // except at those two places, where they touch in a point (zero
    // thickness, 2026-10-15). At 1e-5 and 1e-4 the walks of `side_of`
    // find the cavity's face on one side of the cutter's (they do up to
    // 3.75e-4, `side_of_sim.py` in the review's records), so the move
    // takes the touch onto the place; at 9e-4 they find both sides; at
    // 1.1e-3 the angle is past `TANGENT`. Whatever the reason, a body is
    // not built (`main` refused every one; with the move, 1e-4 as built is
    // refused as zero thickness at the floor's point of C, made without
    // the refusal three shells, where `main` could not divide a face).
    let mut failures = Vec::new();
    for case in Case::unit() {
        let a = case.hollowed();
        for angle in [1e-5, 1e-4, 9e-4, 1.1e-3] {
            let cutter = case.tilted_cutter(angle);
            let label = format!("hollowed - cutter turned {angle:e}");
            let said = outcome(case, &a, &cutter, Subtract);
            println!("{label} at {case:?}: {said}");
            made_anyway(case, &label, &a, &cutter, Subtract);
            if boolean(&a, &cutter, Subtract, &case.tolerance()).is_ok() {
                failures.push(format!("{label} at {case:?}: {said}"));
            }
        }
    }
    none_failed(&failures);
}
