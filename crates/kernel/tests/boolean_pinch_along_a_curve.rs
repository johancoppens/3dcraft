//! A body that meets itself only along a curve (random design 20781906;
//! docs/design/booleans.md › *A pinch along a curve*).
//!
//! The cavity a torus leaves in a block touches the plane x = 0 along a
//! circle C, from beyond it; a cutter whose end face is that plane takes
//! away what lies before it. Where C runs inside the block's section, two
//! arcs, the body left lies beyond x = 0 on both sides of C, between the
//! plane and the cavity, and the two sides meet only on C: the block falls
//! into three parts (the middle and two corners) that touch along the
//! arcs, zero thickness by the developer's rule of 2026-10-15. The cavity's
//! face is bounded there by splines (where the torus meets the block's
//! floor and walls), which cross C and only touch the plane where they do.
//! Their touches stood where the splines come nearest the plane, 3 to 9
//! tolerances off C, so C was not cut where it leaves the cavity's face,
//! went uncut as a touch running over a face's boundary, and was left out:
//! `main` built the three parts as one shell, the plane's face and the
//! cavity's undivided, touching along the arcs inside both, and nothing
//! looked there (the near look passes over two faces that share a vertex,
//! and the vertices at the ends of the arcs are both faces'). Now the
//! touch is taken onto C where the result keeps both faces there
//! (`corner::kept_both`), C is imprinted, and the boolean refuses the
//! parts that touch, naming a point of C.
//!
//! Held to the geometry written here without the kernel: which points are
//! in the body (across C, and the parts a grid of cells falls into), and
//! the volumes of the bodies that build, from
//! `plan/pinch-along-a-curve/tools/pinch_reduced.py` in the workshop (each
//! section of the torus an annulus clipped to the block's rectangle, in
//! closed form; along the torus's axis by quadrature between the places
//! where that clipped annulus changes form; the method gives the whole
//! torus's 2π²Rr² to 6e-12). At scales 1e-3, 1 and 1e3, as built and
//! turned out of the axes.

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

/// Where the torus's axis (along x) passes, in y and z: random design
/// 20781906's, in thousandths.
const CY: f64 = -0.693_073_289_086_241_3;
const CZ: f64 = 3.847_633_424_655_209;
/// The torus: centred at x = 5 on that axis, major radius 10, minor 5, so
/// it reaches x = 0 exactly, along C.
const XC: f64 = 5.0;
const MAJOR: f64 = 10.0;
const MINOR: f64 = 5.0;

/// The block (the slab): x from -5 to 2.5, the section y from -7.5 to 7.5
/// and z from -5 to 2.5. `LONG` runs on to x = 12, past the torus.
const SLAB: Cuboid = ([-5.0, -7.5, -5.0], [2.5, 7.5, 2.5]);
const LONG: Cuboid = ([-5.0, -7.5, -5.0], [12.0, 7.5, 2.5]);
/// The cutter: its end face x = 0 holds the whole of C. `CUTTER_6` ends at
/// y = -6 and 6, so that the slab beyond joins the corners to the middle.
const CUTTER: Cuboid = ([-10.0, -20.0, -20.0], [0.0, 20.0, 20.0]);
const CUTTER_6: Cuboid = ([-10.0, -6.0, -20.0], [0.0, 6.0, 20.0]);

type Cuboid = ([f64; 3], [f64; 3]);

/// The area of the slab's section.
const SECTION: f64 = 15.0 * 7.5;
/// The torus within the slab (x from 0 to 2.5) and within the long block
/// (x from 0 to 10), by `pinch_reduced.py`.
const TORUS_IN_SLAB: f64 = 137.593_010_358_941;
const TORUS_IN_LONG: f64 = 693.135_860_364_612;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

// ---------------------------------------------------------------------
// The geometry, written without the kernel (in the test's units).

fn in_cuboid(p: Vec3, (lo, hi): Cuboid) -> bool {
    (lo[0]..=hi[0]).contains(&p.x)
        && (lo[1]..=hi[1]).contains(&p.y)
        && (lo[2]..=hi[2]).contains(&p.z)
}

fn in_torus(p: Vec3) -> bool {
    let rho = (p.y - CY).phypot(p.z - CZ);
    (p.x - XC) * (p.x - XC) + (rho - MAJOR) * (rho - MAJOR) <= MINOR * MINOR
}

/// Whether `p` is in `block` less the torus less `cutter`.
fn in_result(p: Vec3, block: Cuboid, cutter: Cuboid) -> bool {
    in_cuboid(p, block) && !in_torus(p) && !in_cuboid(p, cutter)
}

/// The point of C at `angle`.
fn on_c(angle: f64) -> Vec3 {
    Vec3::new(0.0, CY + MAJOR * angle.pcos(), CZ + MAJOR * angle.psin())
}

/// The arcs of C inside the slab's section, as ranges of angle (sampled
/// every hundredth of a degree).
fn arcs() -> Vec<(f64, f64)> {
    let n: u32 = 36_000;
    let step = std::f64::consts::TAU / f64::from(n);
    let mut runs: Vec<(f64, f64)> = Vec::new();
    let mut open: Option<f64> = None;
    for k in 0..n {
        let angle = step * f64::from(k);
        let p = on_c(angle);
        let inside = SLAB.0[1] < p.y && p.y < SLAB.1[1] && SLAB.0[2] < p.z && p.z < SLAB.1[2];
        match (inside, open) {
            (true, None) => open = Some(angle),
            (false, Some(start)) => {
                runs.push((start, angle - step));
                open = None;
            }
            _ => {}
        }
    }
    if let Some(start) = open {
        runs.push((start, step * f64::from(n - 1)));
    }
    runs
}

/// How many parts the cells of an `n` grid over x from `x0` to the
/// block's end and over the slab's section fall into, joined face to face.
fn parts(block: Cuboid, cutter: Cuboid, x0: f64, n: [usize; 3]) -> usize {
    let (lo, hi) = (block.0, block.1);
    let size = [hi[0] - x0, hi[1] - lo[1], hi[2] - lo[2]];
    let start = [x0, lo[1], lo[2]];
    let centre =
        |axis: usize, i: usize| start[axis] + size[axis] * (i as f64 + 0.5) / n[axis] as f64;
    let index = |i: usize, j: usize, k: usize| (i * n[1] + j) * n[2] + k;
    let mut label: Vec<i32> = vec![-2; n[0] * n[1] * n[2]];
    for i in 0..n[0] {
        for j in 0..n[1] {
            for k in 0..n[2] {
                let p = Vec3::new(centre(0, i), centre(1, j), centre(2, k));
                if in_result(p, block, cutter) {
                    label[index(i, j, k)] = -1;
                }
            }
        }
    }
    let mut count = 0;
    for i in 0..n[0] {
        for j in 0..n[1] {
            for k in 0..n[2] {
                if label[index(i, j, k)] != -1 {
                    continue;
                }
                let mut stack = vec![[i, j, k]];
                label[index(i, j, k)] = count;
                while let Some(c) = stack.pop() {
                    let steps = [(0, 1), (0, -1), (1, 1), (1, -1), (2, 1), (2, -1)];
                    for (axis, step) in steps {
                        let mut q = c;
                        match q[axis].checked_add_signed(step) {
                            Some(v) if v < n[axis] => q[axis] = v,
                            _ => continue,
                        }
                        let at = index(q[0], q[1], q[2]);
                        if label[at] == -1 {
                            label[at] = count;
                            stack.push(q);
                        }
                    }
                }
                count += 1;
            }
        }
    }
    count as usize
}

// ---------------------------------------------------------------------
// The bodies.

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

    fn block(self, (lo, hi): Cuboid) -> Body {
        let frame = Frame::new(Vec3::new(lo[0], lo[1], lo[2]) * self.k, Vec3::Z, Vec3::X).unwrap();
        let [dx, dy, dz] = [0, 1, 2].map(|i| (hi[i] - lo[i]) * self.k);
        self.put(primitives::block(&frame, dx, dy, dz).unwrap())
    }

    fn torus(self) -> Body {
        let frame = Frame::new(Vec3::new(XC, CY, CZ) * self.k, Vec3::X, Vec3::Y).unwrap();
        self.put(primitives::torus(&frame, MAJOR * self.k, MINOR * self.k).unwrap())
    }

    /// `p` back in the case's own axes and units.
    fn local(self, p: Vec3) -> Vec3 {
        let p = if self.turned {
            self.turn().inverse().unwrap().point(p)
        } else {
            p
        };
        p * (1.0 / self.k)
    }

    /// `block` less the torus.
    fn hollowed(self, block: Cuboid) -> Body {
        let tol = self.tolerance();
        boolean(&self.block(block), &self.torus(), Subtract, &tol)
            .unwrap()
            .body
    }
}

/// The boolean of `a` and `b`, built: it passes `topo::check` and the
/// boolean's full check, and its volume is `expected` (in the test's
/// units) to within the tolerance times its area. Its shells.
fn built(case: Case, label: &str, a: &Body, b: &Body, op: BooleanOp, expected: f64) -> usize {
    let tol = case.tolerance();
    let result =
        boolean(a, b, op, &tol).unwrap_or_else(|e| panic!("{label} at {case:?}: refused: {e}"));
    let defects = check(&result.body, &tol);
    assert!(defects.is_empty(), "{label} at {case:?}: {defects:?}");
    let found = verify(a, b, op, &result.body, &tol, &VerifyOptions::default()).unwrap();
    assert!(
        found.violations.is_empty(),
        "{label} at {case:?}: {:?}",
        found.violations
    );
    let mass = mass_properties(&result.body).unwrap();
    let want = expected * case.k * case.k * case.k;
    println!(
        "{label} at {case:?}: {} shells, volume {:e} against {want:e}",
        result.body.shells.len(),
        mass.volume
    );
    assert!(
        (mass.volume - want).abs() <= tol.linear * mass.area,
        "{label} at {case:?}: volume {:e} against {want:e}, allowed {:e}",
        mass.volume,
        tol.linear * mass.area
    );
    result.body.shells.len()
}

/// The boolean of `a` and `b`, refused as zero thickness at a point of C
/// in the slab's section (on one of the arcs, or at an end of one), within
/// twice the tolerance.
fn zero_thickness_on_c(case: Case, label: &str, a: &Body, b: &Body, op: BooleanOp) {
    let tol = case.tolerance();
    let at = match boolean(a, b, op, &tol) {
        Err(KernelError::ZeroThickness { at, .. }) => at,
        Err(e) => panic!("{label} at {case:?}: refused otherwise: {e}"),
        Ok(r) => panic!("{label} at {case:?}: built, {} shells", r.body.shells.len()),
    };
    let p = case.local(at);
    let near = 2e-6;
    let off = ((p.y - CY).phypot(p.z - CZ) - MAJOR).abs();
    println!("{label} at {case:?}: zero thickness at {p:?}, {off:e} off C");
    assert!(
        p.x.abs() <= near && off <= near,
        "{label} at {case:?}: {p:?} is not on C"
    );
    let (lo, hi) = SLAB;
    assert!(
        (lo[1] - near..=hi[1] + near).contains(&p.y)
            && (lo[2] - near..=hi[2] + near).contains(&p.z),
        "{label} at {case:?}: {p:?} is outside the slab's section"
    );
}

// ---------------------------------------------------------------------

#[test]
fn the_body_lies_on_both_sides_of_the_arcs_in_three_parts_that_meet_only_there() {
    // Across C at the middle of each arc, beyond x = 0 by half of what the
    // cavity leaves there (its wall is e²/10 beyond the plane at e from
    // C): in the body outward and inward of C, not on C between them, and
    // nothing before the plane.
    let runs = arcs();
    assert_eq!(runs.len(), 2, "{runs:?}");
    for (a, b) in runs {
        let middle = 0.5 * (a + b);
        let c = on_c(middle);
        let out = Vec3::new(0.0, middle.pcos(), middle.psin());
        for e in [1e-1, 1e-2, 1e-3, 1e-4] {
            let x = Vec3::new(e * e / 20.0, 0.0, 0.0);
            let hold = |p: Vec3| in_result(p, SLAB, CUTTER);
            assert!(
                hold(c + x + out * e) && hold(c + x - out * e),
                "{middle} {e}"
            );
            assert!(!hold(c + x), "{middle} {e}");
            assert!(!hold(c - Vec3::new(e, 0.0, 0.0)), "{middle} {e}");
        }
    }
    // The parts beyond x = 0.05: three, so they meet only on the plane, on
    // C; the same with the block running past the torus (the corners end
    // where the torus's outer wall sweeps past them); one with the cutter
    // ending at y = -6 and 6 (the slab before x = 0 beyond them joins the
    // corners to the middle: a wall that thins to nothing within one
    // shell).
    assert_eq!(parts(SLAB, CUTTER, 0.05, [40, 120, 60]), 3);
    assert_eq!(parts(LONG, CUTTER, 0.05, [120, 120, 60]), 3);
    assert_eq!(parts(SLAB, CUTTER_6, -5.0, [60, 120, 60]), 1);
}

#[test]
fn a_cutters_end_touching_a_cavity_along_two_arcs_is_zero_thickness() {
    for case in Case::all() {
        let tol = case.tolerance();
        let cutter = case.block(CUTTER);
        for (label, block, point_set) in [
            ("slab", SLAB, 2.5 * SECTION - TORUS_IN_SLAB),
            ("long", LONG, 12.0 * SECTION - TORUS_IN_LONG),
        ] {
            let a = case.hollowed(block);
            zero_thickness_on_c(case, label, &a, &cutter, Subtract);
            // Made without the refusal: the three parts, apart, and the
            // volume of the set.
            let options = BooleanOptions {
                refuse_zero_thickness: false,
                ..BooleanOptions::default()
            };
            let made = boolean_with(&a, &cutter, Subtract, &tol, &options).unwrap();
            assert_eq!(made.body.shells.len(), 3, "{label} at {case:?}");
            assert!(check(&made.body, &tol).is_empty(), "{label} at {case:?}");
            let mass = mass_properties(&made.body).unwrap();
            let want = point_set * case.k * case.k * case.k;
            assert!(
                (mass.volume - want).abs() <= tol.linear * mass.area,
                "{label} at {case:?}: {:e} against {want:e}",
                mass.volume
            );
        }
    }
}

#[test]
fn the_same_cut_with_its_parts_joined_beyond_the_cutter_is_one_shell_that_thins_to_nothing() {
    for case in Case::all() {
        let a = case.hollowed(SLAB);
        let cutter = case.block(CUTTER_6);
        let shells = built(
            case,
            "slab less the cutter to y = ±6",
            &a,
            &cutter,
            Subtract,
            7.5 * SECTION - TORUS_IN_SLAB - 5.0 * 12.0 * 7.5,
        );
        assert_eq!(shells, 1, "{case:?}");
    }
}

#[test]
fn where_the_result_keeps_one_face_or_neither_the_touch_divides_nothing() {
    for case in Case::all() {
        let tol = case.tolerance();
        let a = case.hollowed(SLAB);
        let cutter = case.block(CUTTER);
        let inside = boolean(&case.block(SLAB), &case.torus(), Intersect, &tol)
            .unwrap()
            .body;
        let cutter_volume = 10.0 * 40.0 * 40.0;
        let before = 5.0 * SECTION;
        // The cavity's face kept on both sides of C, the plane's not.
        built(
            case,
            "union",
            &a,
            &cutter,
            Union,
            cutter_volume + 2.5 * SECTION - TORUS_IN_SLAB,
        );
        // The plane's face kept, the cavity's not.
        built(case, "intersection", &a, &cutter, Intersect, before);
        // Neither kept: what the slab leaves of the cutter.
        built(
            case,
            "the cutter less the slab",
            &cutter,
            &a,
            Subtract,
            cutter_volume - before,
        );
        // The torus within the slab, cut by the cutter it only touches:
        // its face kept, the plane's not; nothing changes.
        built(
            case,
            "the torus within the slab less the cutter",
            &inside,
            &cutter,
            Subtract,
            TORUS_IN_SLAB,
        );
    }
}

#[test]
fn two_parts_that_rest_on_each_other_along_the_arcs_are_zero_thickness() {
    for case in Case::all() {
        let tol = case.tolerance();
        let inside = boolean(&case.block(SLAB), &case.torus(), Intersect, &tol)
            .unwrap()
            .body;
        let cutter = case.block(CUTTER);
        zero_thickness_on_c(
            case,
            "the torus within the slab and the cutter",
            &inside,
            &cutter,
            Union,
        );
    }
}
