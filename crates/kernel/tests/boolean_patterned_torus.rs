//! A cut torus patterned about the middle of its extent (random design
//! 108069; docs/design/booleans.md › *A curve of touch kept past a face's
//! border*).
//!
//! The torus about the x axis (major radius 10, minor 5), kept where x is
//! below 2.5, is turned in steps of 45 degrees about the z axis through
//! (-1.25, 0.1, 0), the middle of its extent along x. The copy half a turn
//! on then has its torus tangent to the first copy's cut plane along a
//! circle, from within the first copy, and the other way round: four
//! instances (0 and 1, 4 and 5) have four such circles. Where a circle is
//! clear of the other two instances the union is solid on the cut plane's
//! inner side and empty beyond it, so the circle is no edge of the union,
//! and the union is a body: nothing of zero thickness is there. That is
//! held here without the kernel, and the volumes come from
//! `plan/patterned-torus/tools/quad_patterned.py` in the workshop: every
//! instance's axis is horizontal and its cut plane vertical, so a vertical
//! line meets each instance in one stretch in closed form, and the length
//! of the set along the line is integrated over x and y by the tanh-sinh
//! rule, in pieces between the places where the order of those stretches'
//! ends changes (the whole torus by it is 2π²Rr² to 1e-9 and the cut torus
//! its closed form to 7e-9; the pairs, instances 0, 1 and 4 and the four
//! were computed at two settings, which agree to 3e-6, and the other
//! figures at the finer one alone).
//!
//! `main` united two instances, and three, and refused (0 + 1) + (4 + 5),
//! the pattern's own order, as a body its own check found invalid. On
//! each circle a stretch was kept from where the torus of the second
//! instance on the plane's side crosses it to where it does again, though
//! the touching torus's face ends 0.0115 before, where the cut plane of
//! the second instance on its side cuts it: the edge there, a spline, only
//! touches the first plane, and its touch stood 6 to 12 tolerances off the
//! circle, so the circle had no cut where it leaves that face. Now a
//! stretch along which two faces only touch, inside both by its samples
//! but with an end clear outside one of them, is left out where the
//! result does not keep both faces.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::{BooleanOp, BooleanOptions, VerifyOptions, boolean, verify};
use craft3d_kernel::halfspace::{Kept, keep_below};
use craft3d_kernel::primitives;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, check, mass_properties};

const MAJOR: f64 = 10.0;
const MINOR: f64 = 5.0;
/// The source is kept where x is below this.
const CUT: f64 = 2.5;
/// The pivot of the pattern: the middle of the source's extent along x
/// (from -5 to 2.5), a tenth off the torus's axis.
const PIVOT: (f64, f64) = (-1.25, 0.1);

// The volumes, by `quad_patterned.py` (see the module's text), in the
// test's units.
/// One instance: 40π (5.412658773652741 + 25π/3), in closed form.
const ONE: f64 = 3_970.042_895_284;
/// Instances 0 and 4, half a turn apart.
const UNION_0_4: f64 = 4_561.990_806_18;
/// Instances 0 and 1 (and 4 and 5).
const UNION_0_1: f64 = 6_641.429_012_7;
/// Instances 0 and 5 (and 1 and 4).
const UNION_0_5: f64 = 6_640.170_013_7;
/// Instances 0, 1 and 4 (and, half a turn round, 0, 4 and 5).
const UNION_0_1_4: f64 = 7_074.889_833_2;
/// Instances 0, 1 and 5 (and 1, 4 and 5).
const UNION_0_1_5: f64 = 7_068.469_720_1;
/// Instances 0, 1, 4 and 5 (7458.390792 and 7458.390795 at the two
/// settings).
const UNION_FOUR: f64 = 7_458.390_795_2;
/// Instances 0 to 3.
const UNION_0_TO_3: f64 = 11_361.096_089_4;
/// The pattern of six.
const UNION_SIX: f64 = 11_935.951_438_6;
/// Instances 0, 1, 4 and 5 with the pivot at other distances from the
/// torus's axis: the distance and the volume. (At -0.1 the four are the
/// mirror image, turned by one step, of the four at 0.1, so the volume is
/// the same; the two figures, computed apart, differ by 2.3e-7.)
const PIVOT_Y: [(f64, f64); 9] = [
    (0.0, 7_452.309_562_5),
    (0.001, 7_452.310_272_6),
    (0.01, 7_452.375_581_9),
    (0.03, 7_452.881_980_8),
    (0.3, 7_503.918_820_9),
    (0.5, 7_586.713_768_2),
    (1.0, 7_904.808_181_3),
    (2.0, 8_800.000_689_6),
    (-0.1, 7_458.390_795_0),
];
/// The two instances at the start and the two half a turn on, at other
/// steps of the pattern: the step in degrees, the instance half a turn on
/// and the volume.
const STEPS: [(f64, u32, f64); 3] = [
    (60.0, 3, 7_834.687_840_4),
    (30.0, 6, 6_697.069_057_1),
    (20.0, 9, 6_002.137_862_0),
];

// ---------------------------------------------------------------------
// The geometry, written without the kernel (in the test's units).

/// `p` in the source's own axes, for the instance turned by `turn` radians
/// about the pivot.
fn to_source(p: Vec3, turn: f64, pivot: (f64, f64)) -> Vec3 {
    let (c, s) = (turn.pcos(), turn.psin());
    let (x, y) = (p.x - pivot.0, p.y - pivot.1);
    Vec3::new(pivot.0 + c * x + s * y, pivot.1 - s * x + c * y, p.z)
}

/// The source's point `q` in the instance turned by `turn`.
fn from_source(q: Vec3, turn: f64, pivot: (f64, f64)) -> Vec3 {
    let (c, s) = (turn.pcos(), turn.psin());
    let (x, y) = (q.x - pivot.0, q.y - pivot.1);
    Vec3::new(pivot.0 + c * x - s * y, pivot.1 + s * x + c * y, q.z)
}

/// Whether `q`, in the source's axes, is in the cut torus.
fn in_source(q: Vec3) -> bool {
    let rho = q.y.phypot(q.z);
    (rho - MAJOR) * (rho - MAJOR) + q.x * q.x <= MINOR * MINOR && q.x <= CUT
}

// ---------------------------------------------------------------------
// The bodies.

#[derive(Clone, Copy, Debug)]
struct Case {
    /// The scale of every length.
    k: f64,
    /// The pivot, in the test's units.
    pivot: (f64, f64),
    /// The step of the pattern, in degrees.
    step: f64,
    /// Turned out of the axes.
    turned: bool,
}

const PLAIN: Case = Case {
    k: 1.0,
    pivot: PIVOT,
    step: 45.0,
    turned: false,
};

impl Case {
    fn scales() -> Vec<Case> {
        [1e-3, 1.0, 1e3, 1e6]
            .into_iter()
            .flat_map(|k| [false, true].map(|turned| Case { k, turned, ..PLAIN }))
            .collect()
    }

    fn tolerance(self) -> Tolerance {
        Tolerance::new(1e-6 * self.k, 1e-9).unwrap()
    }

    fn turn(self) -> Transform {
        Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
            .unwrap()
            .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * self.k))
    }

    /// The angle of instance `i`, in radians.
    fn angle(self, i: u32) -> f64 {
        self.step.to_radians() * f64::from(i)
    }

    /// Whether `p`, in the test's units and axes, is in instance `i`.
    fn holds(self, i: u32, p: Vec3) -> bool {
        in_source(to_source(p, self.angle(i), self.pivot))
    }

    /// The torus cut by the plane x = 2.5, as the commands make it.
    fn source(self) -> Body {
        let tol = self.tolerance();
        let frame = Frame::new(Vec3::ZERO, Vec3::X, Vec3::Y).unwrap();
        let torus = primitives::torus(&frame, MAJOR * self.k, MINOR * self.k).unwrap();
        let origin = Vec3::new(CUT, 0.0, 0.0) * self.k;
        let plane = Frame::from_z(origin, -Vec3::X)
            .unwrap()
            .reversed_at(origin)
            .unwrap();
        match keep_below(&torus, &plane, &tol, &BooleanOptions::default()).unwrap() {
            Kept::Cut(result) => result.body,
            Kept::Whole => panic!("the plane cut nothing"),
        }
    }

    /// Instance `i` of `source`, as the pattern places it.
    fn instance(self, source: &Body, i: u32) -> Body {
        let placed = if i == 0 {
            source.clone()
        } else {
            let pivot = Vec3::new(self.pivot.0, self.pivot.1, 0.0) * self.k;
            let map = Transform::rotation_about(pivot, Vec3::Z, self.angle(i)).unwrap();
            source.transformed(&map).unwrap()
        };
        if self.turned {
            placed.transformed(&self.turn()).unwrap()
        } else {
            placed
        }
    }

    /// The instances `ids`.
    fn instances<const N: usize>(self, ids: [u32; N]) -> [Body; N] {
        let source = self.source();
        ids.map(|i| self.instance(&source, i))
    }
}

/// What a boolean came to.
enum Came {
    Built(Body),
    Refused(String),
    ZeroThickness(Vec3),
}

/// The boolean of `a` and `b`. Built, it passes `topo::check` and the
/// boolean's full check and its volume is `expected` (in the test's
/// units) to within the tolerance times its area and `slack` times its
/// area; an invalid body or a wrong result is a failure here.
fn came(
    case: Case,
    label: &str,
    a: &Body,
    b: &Body,
    op: BooleanOp,
    (expected, slack): (f64, f64),
) -> Came {
    let tol = case.tolerance();
    let result = match boolean(a, b, op, &tol) {
        Ok(result) => result,
        Err(KernelError::ZeroThickness { at, .. }) => return Came::ZeroThickness(at),
        Err(e @ (KernelError::InvalidBody { .. } | KernelError::WrongResult { .. })) => {
            panic!("{label} at {case:?}: {e}")
        }
        Err(e) => return Came::Refused(e.to_string()),
    };
    let defects = check(&result.body, &tol);
    assert!(defects.is_empty(), "{label} at {case:?}: {defects:?}");
    let found = verify(a, b, op, &result.body, &tol, &VerifyOptions::default()).unwrap();
    assert!(
        found.violations.is_empty(),
        "{label} at {case:?}: {:?}",
        found.violations
    );
    let mass = mass_properties(&result.body).unwrap();
    let k3 = case.k * case.k * case.k;
    let allowed = (tol.linear + slack * case.k) * mass.area;
    println!(
        "{label} at {case:?}: {} faces, {} shells, volume {:.9} against {expected:.9}",
        result.body.faces.len(),
        result.body.shells.len(),
        mass.volume / k3
    );
    assert!(
        (mass.volume - expected * k3).abs() <= allowed,
        "{label} at {case:?}: volume {:e} against {:e}, allowed {allowed:e}",
        mass.volume,
        expected * k3
    );
    Came::Built(result.body)
}

/// [`came`], built.
fn built(case: Case, label: &str, a: &Body, b: &Body, op: BooleanOp, expected: f64) -> Body {
    match came(case, label, a, b, op, (expected, 0.0)) {
        Came::Built(body) => body,
        Came::Refused(words) => panic!("{label} at {case:?}: refused: {words}"),
        Came::ZeroThickness(at) => panic!("{label} at {case:?}: zero thickness at {at:?}"),
    }
}

// ---------------------------------------------------------------------

/// The right answer, without the kernel: at a circle along which one
/// instance's torus touches the opposite instance's cut plane, where the
/// other two instances are clear, the union is solid on the plane's inner
/// side and empty beyond it. The cut face is the union's boundary there
/// and the circle is no edge of it: a body, and nothing of zero thickness.
/// (What the difference of the two leaves there is two wedges that meet
/// along the circle, which is the other thing.)
#[test]
fn the_union_is_solid_on_one_side_of_each_circle_and_empty_on_the_other() {
    let case = PLAIN;
    // (the instance whose cut plane it is, the instance whose torus
    // touches it, the other two)
    let circles = [
        (0, 4, [1, 5]),
        (4, 0, [1, 5]),
        (1, 5, [0, 4]),
        (5, 1, [0, 4]),
    ];
    for (plane, torus, others) in circles {
        // The torus's far circle, x = -5 in its own axes, and the cut
        // plane's outward normal.
        let on_circle = |t: f64| {
            let q = Vec3::new(-MINOR, MAJOR * t.pcos(), MAJOR * t.psin());
            from_source(q, case.angle(torus), case.pivot)
        };
        let normal = from_source(Vec3::new(1.0, 0.0, 0.0), case.angle(plane), case.pivot)
            - from_source(Vec3::ZERO, case.angle(plane), case.pivot);
        let mut places = 0;
        for step in 0..720_u32 {
            let c = on_circle(f64::from(step).to_radians() * 0.5);
            // On the cut plane, to rounding.
            let across = (to_source(c, case.angle(plane), case.pivot).x - CUT).abs();
            assert!(across < 1e-12, "{plane} {torus}: {across:e} off the plane");
            // A place counts where the other two instances are clear of
            // the largest ball looked at.
            let reach = 0.05;
            let clear = (-2..=2).all(|i| {
                (-2..=2).all(|j| {
                    (-2..=2).all(|l| {
                        let p =
                            c + Vec3::new(f64::from(i), f64::from(j), f64::from(l)) * (0.5 * reach);
                        !others.iter().any(|&o| case.holds(o, p))
                    })
                })
            });
            if !clear {
                continue;
            }
            places += 1;
            for e in [1e-2, 1e-3, 1e-4, 1e-5, 1e-6] {
                // Points of a ball of radius `e` about the place: along
                // the normal and at 45 degrees to it, eight ways round.
                for k in 0..8_u32 {
                    let round = f64::from(k) * std::f64::consts::FRAC_PI_4;
                    let side = Vec3::new(0.0, 0.0, 1.0) * round.pcos()
                        + normal.cross(Vec3::Z) * round.psin();
                    for lean in [0.0, 0.5, 0.9] {
                        let inward = (normal * -1.0 + side * lean) * e;
                        let p = c + inward;
                        assert!(
                            case.holds(plane, p),
                            "{plane} {torus}: {p:?} is not in instance {plane}"
                        );
                        let q = c - inward;
                        assert!(
                            !case.holds(plane, q) && !case.holds(torus, q),
                            "{plane} {torus}: {q:?} is in an instance"
                        );
                    }
                }
            }
        }
        println!("circle of torus {torus} on cut plane {plane}: {places} places of 720");
        assert!(places > 300, "{plane} {torus}: {places}");
    }
}

/// The reduction: (0 + 1) + (4 + 5), either way round, and the other
/// orders of the same four, at four scales, as placed and turned. On
/// `main` the first two are bodies its own check found invalid.
#[test]
fn four_instances_unite_whichever_way_they_are_paired() {
    for case in Case::scales() {
        let [i0, i1, i4, i5] = case.instances([0, 1, 4, 5]);
        let a = built(case, "0 + 1", &i0, &i1, Union, UNION_0_1);
        let b = built(case, "4 + 5", &i4, &i5, Union, UNION_0_1);
        let first = built(case, "(0 + 1) + (4 + 5)", &a, &b, Union, UNION_FOUR);
        assert_eq!(first.shells.len(), 1, "{case:?}");
        built(case, "(4 + 5) + (0 + 1)", &b, &a, Union, UNION_FOUR);
        if case.k == 1.0 {
            let three = built(case, "(0 + 1) + 4", &a, &i4, Union, UNION_0_1_4);
            built(case, "((0 + 1) + 4) + 5", &three, &i5, Union, UNION_FOUR);
            let c = built(case, "0 + 4", &i0, &i4, Union, UNION_0_4);
            let d = built(case, "1 + 5", &i1, &i5, Union, UNION_0_4);
            built(case, "(0 + 4) + (1 + 5)", &c, &d, Union, UNION_FOUR);
        }
    }
}

/// The pattern of six as the pattern joins it, ((0 + 1) + (2 + 3)) +
/// (4 + 5): on `main` the last join is the invalid body random design
/// 108069 met.
#[test]
fn the_pattern_of_six_unites() {
    for case in [
        PLAIN,
        Case {
            k: 1e-3,
            turned: true,
            ..PLAIN
        },
        Case {
            k: 1e3,
            turned: true,
            ..PLAIN
        },
    ] {
        let [i0, i1, i2, i3, i4, i5] = case.instances([0, 1, 2, 3, 4, 5]);
        let a = built(case, "0 + 1", &i0, &i1, Union, UNION_0_1);
        let b = built(case, "2 + 3", &i2, &i3, Union, UNION_0_1);
        let c = built(case, "4 + 5", &i4, &i5, Union, UNION_0_1);
        let ab = built(case, "(0 + 1) + (2 + 3)", &a, &b, Union, UNION_0_TO_3);
        built(case, "the pattern of six", &ab, &c, Union, UNION_SIX);
    }
}

/// Two and three of the four: built on `main` too, and the same bodies
/// now. They hold the volumes written here to bodies built before.
#[test]
fn fewer_instances_unite_as_before() {
    let case = PLAIN;
    let [i0, i1, i4, i5] = case.instances([0, 1, 4, 5]);
    let mass = mass_properties(&i0).unwrap();
    assert!(
        (mass.volume - ONE).abs() <= 1e-6 * mass.area,
        "{}",
        mass.volume
    );
    let a = built(case, "0 + 1", &i0, &i1, Union, UNION_0_1);
    let b = built(case, "4 + 5", &i4, &i5, Union, UNION_0_1);
    built(case, "0 + 4", &i0, &i4, Union, UNION_0_4);
    built(case, "4 + 0", &i4, &i0, Union, UNION_0_4);
    built(case, "1 + 5", &i1, &i5, Union, UNION_0_4);
    built(case, "0 + 5", &i0, &i5, Union, UNION_0_5);
    built(case, "(0 + 1) + 4", &a, &i4, Union, UNION_0_1_4);
    built(case, "(0 + 1) + 5", &a, &i5, Union, UNION_0_1_5);
    built(case, "0 + (4 + 5)", &i0, &b, Union, UNION_0_1_4);
    built(case, "1 + (4 + 5)", &i1, &b, Union, UNION_0_1_5);
    // The common part of the two that touch, by the identity of sets.
    built(case, "0 * 4", &i0, &i4, Intersect, 2.0 * ONE - UNION_0_4);
}

/// The common part of the two pairs is built (on `main` an invalid body),
/// and their difference is refused as zero thickness at a place where an
/// instance's torus touches the other pair's cut plane (on `main` an
/// invalid body too): what the difference leaves there is two wedges
/// between the plane and the torus that meet along the circle, and they
/// are two parts (cells of a grid joined face to face,
/// `plan/patterned-torus/tools/parts_diff.py` in the workshop), which the
/// decision of 2026-10-15 refuses.
#[test]
fn the_common_part_and_the_difference_of_the_two_pairs() {
    for case in [
        PLAIN,
        Case {
            k: 1e3,
            turned: true,
            ..PLAIN
        },
    ] {
        let [i0, i1, i4, i5] = case.instances([0, 1, 4, 5]);
        let a = built(case, "0 + 1", &i0, &i1, Union, UNION_0_1);
        let b = built(case, "4 + 5", &i4, &i5, Union, UNION_0_1);
        let common = 2.0 * UNION_0_1 - UNION_FOUR;
        built(case, "(0 + 1) * (4 + 5)", &a, &b, Intersect, common);
        built(case, "(4 + 5) * (0 + 1)", &b, &a, Intersect, common);
        for (label, x, y, circles) in [
            ("(0 + 1) - (4 + 5)", &a, &b, [(0, 4), (1, 5)]),
            ("(4 + 5) - (0 + 1)", &b, &a, [(4, 0), (5, 1)]),
        ] {
            match came(case, label, x, y, Subtract, (UNION_0_1 - common, 0.0)) {
                Came::Built(_) => panic!("{label} at {case:?}: built, two parts that touch"),
                Came::Refused(words) => panic!("{label} at {case:?}: refused: {words}"),
                Came::ZeroThickness(at) => {
                    // On one of the two circles of this difference: on the
                    // cut plane of the one and the torus of the other.
                    let local = if case.turned {
                        case.turn().inverse().unwrap().point(at)
                    } else {
                        at
                    };
                    let p = local * (1.0 / case.k);
                    let off = circles
                        .iter()
                        .map(|&(plane, torus)| {
                            let q = to_source(p, case.angle(torus), case.pivot);
                            let on_torus = (q.x + MINOR).abs().max((q.y.phypot(q.z) - MAJOR).abs());
                            let on_plane =
                                (to_source(p, case.angle(plane), case.pivot).x - CUT).abs();
                            on_torus.max(on_plane)
                        })
                        .fold(f64::INFINITY, f64::min);
                    println!("{label} at {case:?}: zero thickness at {p:?}, {off:e} off a circle");
                    assert!(off <= 2e-6, "{label} at {case:?}: {p:?} is on no circle");
                }
            }
        }
    }
}

/// The pivot moved along x by parts of the tolerance and more: the copies'
/// tori stay short of the opposite cut planes (to the left) or pass
/// through them (to the right). Moving the pivot by d moves three of the
/// four instances, each by up to 2 |d| (instance 4 by 2 |d|, 5 by
/// 2 |d| sin 67.5° and 1 by 2 |d| sin 22.5°; instance 0 not at all), so the
/// union's volume is within 2 |d| times its area of the unmoved one's. Where the tori pass through the planes by
/// up to twice the tolerance the boolean may refuse at the edge of
/// tolerance, as it does for any two faces; it never builds an invalid
/// body.
#[test]
fn the_pivot_moved_along_x_by_parts_of_the_tolerance() {
    let tol = 1e-6;
    for d in [
        -10.0, -2.0, -1.0, -0.55, -0.45, -0.3, -0.1, 0.1, 0.3, 0.45, 0.55, 1.0, 2.0, 10.0,
    ] {
        let case = Case {
            pivot: (PIVOT.0 + d * tol, PIVOT.1),
            ..PLAIN
        };
        let [i0, i1, i4, i5] = case.instances([0, 1, 4, 5]);
        let slack = 2.0 * (d * tol).abs();
        let pair = |label: &str, x: &Body, y: &Body| match came(
            case,
            label,
            x,
            y,
            Union,
            (UNION_0_1, 0.0),
        ) {
            Came::Built(body) => body,
            Came::Refused(words) => panic!("{label} at {d} tolerances: {words}"),
            Came::ZeroThickness(at) => panic!("{label} at {d} tolerances: {at:?}"),
        };
        let (a, b) = (pair("0 + 1", &i0, &i1), pair("4 + 5", &i4, &i5));
        for (label, x, y) in [("(0 + 1) + (4 + 5)", &a, &b), ("(4 + 5) + (0 + 1)", &b, &a)] {
            match came(case, label, x, y, Union, (UNION_FOUR, slack)) {
                Came::Built(_) => println!("{label}, the pivot {d} tolerances along x: built"),
                Came::Refused(words)
                    if (0.0..=1.0).contains(&d) && words.contains("at the edge of tolerance") =>
                {
                    println!("{label}, the pivot {d} tolerances along x: {words}");
                }
                Came::Refused(words) => panic!("{label} at {d} tolerances: {words}"),
                Came::ZeroThickness(at) => panic!("{label} at {d} tolerances: {at:?}"),
            }
        }
    }
}

/// The pivot at other distances from the torus's axis (on `main` built at
/// 0 and 0.001, an invalid body from 0.01 to 2, and at -0.1 *"imprint
/// lost"*), each against the volume written without the kernel for it.
#[test]
fn the_pivot_at_other_distances_from_the_axis() {
    for (y, expected) in PIVOT_Y {
        let case = Case {
            pivot: (PIVOT.0, y),
            ..PLAIN
        };
        let [i0, i1, i4, i5] = case.instances([0, 1, 4, 5]);
        let tol = case.tolerance();
        let a = boolean(&i0, &i1, Union, &tol).unwrap().body;
        let b = boolean(&i4, &i5, Union, &tol).unwrap().body;
        let label = format!("(0 + 1) + (4 + 5), the pivot at y = {y}");
        built(case, &label, &a, &b, Union, expected);
        built(case, &label, &b, &a, Union, expected);
    }
}

/// Other steps of the pattern: the two instances at the start and the two
/// half a turn on (at 60 degrees an invalid body on `main`).
#[test]
fn other_steps_of_the_pattern() {
    for (step, half, expected) in STEPS {
        let case = Case { step, ..PLAIN };
        let [i0, i1, j0, j1] = case.instances([0, 1, half, half + 1]);
        let tol = case.tolerance();
        let a = boolean(&i0, &i1, Union, &tol).unwrap().body;
        let b = boolean(&j0, &j1, Union, &tol).unwrap().body;
        let label = format!("(0 + 1) + ({half} + {}), step {step}", half + 1);
        built(case, &label, &a, &b, Union, expected);
    }
}
