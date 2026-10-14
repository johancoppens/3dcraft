//! Booleans found by modelling a helmet: a shell turned from two splines
//! that close on the axis (a pole on each), cut by radial frustums.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;
mod common;

use boolean_common::*;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::{boolean, contains_points};
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_topo::{Body, check};
use std::f64::consts::TAU;

type P = [f64; 2];

fn spline(id: u64, control: &[P]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control: control.to_vec(),
        }),
    }
}

/// The helmet's shell, `k` times its size in millimetres: turned about
/// the z axis from an outer and an inner spline, closed by a ring at the
/// bottom. Its top is `vent` from the axis: at 0 both splines end on the
/// axis (poles at z = 122 and z = 118), otherwise a short wall of that
/// radius joins them.
fn shell(k: f64, vent: f64) -> Body {
    // Points as (radius, height); the sketch has them the other way round.
    let s = |p: P| [p[1] * k, p[0] * k];
    let curves = vec![
        spline(
            1,
            &[
                s([110.0, 0.0]),
                s([112.0, 75.0]),
                s([62.0, 122.0]),
                s([vent, 122.0]),
            ],
        ),
        common::line(3, s([vent, 122.0]), s([vent, 118.0])),
        spline(
            5,
            &[
                s([vent, 118.0]),
                s([59.0, 118.0]),
                s([108.0, 73.0]),
                s([106.0, 0.0]),
            ],
        ),
        common::line(7, s([106.0, 0.0]), s([110.0, 0.0])),
    ];
    let tol = tolerance(k);
    let profiles = detect_profiles(&curves, &tol).unwrap();
    // As the document's zx plane: sketch x along the world's z, sketch y
    // along its x; turned about the sketch's x axis.
    let frame = Frame::new(Vec3::ZERO, Vec3::Y, Vec3::Z).unwrap();
    let body = revolve(&profiles, &frame, [0.0, 0.0], [1.0, 0.0], 0.0, TAU, &tol)
        .unwrap()
        .body;
    assert_eq!(check(&body, &tol), []);
    body
}

/// A frustum: the polygon `near` at the height `h0` and the polygon `far`
/// at the height `h1`, lofted (ruled), then turned by `angle` degrees
/// about `axis` through the origin.
fn frustum(
    k: f64,
    (near, h0): (&[P], f64),
    (far, h1): (&[P], f64),
    axis: Vec3,
    angle: f64,
) -> Body {
    let tol = tolerance(k);
    let section = |h: f64| {
        let points: Vec<P> = if h == h0 { near } else { far }
            .iter()
            .map(|p| [p[0] * k, p[1] * k])
            .collect();
        let curves = common::polygon(1, 2, &points);
        let profiles = detect_profiles(&curves, &tol).unwrap();
        Section {
            plane: Frame::WORLD.at(Vec3::new(0.0, 0.0, h * k)).unwrap(),
            outline: profiles[0].outer.clone(),
            start: None,
        }
    };
    let options = Options {
        ruled: true,
        closed: false,
    };
    let body = loft(&[section(h0), section(h1)], &options, &tol)
        .unwrap()
        .body;
    body.transformed(&Transform::rotation(axis, angle.to_radians()).unwrap())
        .unwrap()
}

/// A cell of the helmet: the five corners near and far, the turn.
struct Cell {
    near: &'static [P],
    /// The height of the near corners (the far ones are at 170).
    from: f64,
    far: &'static [P],
    axis: [f64; 3],
    angle: f64,
    /// The radius of the shell's vent at the top (0: a pole).
    vent: f64,
}

/// Cell 1 (polar angle 4.7°: the pole is inside the frustum, near a side).
const CELL1: Cell = Cell {
    near: &[
        [-9.03678, 1.15395],
        [-0.808, -9.46478],
        [3.74477, -9.8357],
        [8.55055, 0.76463],
        [-0.31843, 11.02043],
    ],
    from: 100.0,
    far: &[
        [-15.36254, 1.96171],
        [-1.37361, -16.09014],
        [6.36611, -16.7207],
        [14.53593, 1.29989],
        [-0.54134, 18.73475],
    ],
    axis: [0.932032423813, 0.36237489008, 0.0],
    angle: 4.679481278,
    vent: 0.0,
};

/// Cell 8 (polar angle 18°: its sides pass about 20 mm from the pole).
const CELL8: Cell = Cell {
    near: &[
        [-1.72167, 8.82927],
        [-9.14915, 2.6702],
        [-9.31308, -5.27057],
        [6.33467, -8.06997],
        [7.41392, 6.76645],
    ],
    from: 100.0,
    far: &[
        [-2.92683, 15.00976],
        [-15.55355, 4.53934],
        [-15.83224, -8.95995],
        [10.76894, -13.71893],
        [12.60365, 11.50296],
    ],
    axis: [-0.751169318396, 0.660109578101, 0.0],
    angle: 18.194872339,
    vent: 0.0,
};

/// Cell 3, from 60 mm (polar angle 10.5°), against the shell with a
/// vent.
const CELL3: Cell = Cell {
    near: &[
        [-4.06911, 4.96657],
        [-6.47441, 2.20259],
        [-6.52241, 1.10303],
        [-1.16338, -5.0177],
        [3.44555, -4.01199],
        [5.81747, 2.42354],
        [5.29266, 3.14302],
    ],
    from: 60.0,
    far: &[
        [-11.52913, 14.07195],
        [-18.34417, 6.24066],
        [-18.48017, 3.12524],
        [-3.29624, -14.21681],
        [9.76239, -11.36731],
        [16.48283, 6.86668],
        [14.99587, 8.90523],
    ],
    axis: [-0.279503761323, 0.960144597134, 0.0],
    angle: 10.475313843,
    vent: 14.0,
};

/// Cell 63 (polar angle 54°), against the shell with a vent.
const CELL63: Cell = Cell {
    near: &[
        [-9.16175, 4.71317],
        [3.31517, 9.18747],
        [6.08775, 7.75556],
        [8.20351, -6.11715],
        [-5.17689, -8.12045],
        [-9.50355, 3.96532],
    ],
    from: 100.0,
    far: &[
        [-15.57497, 8.01239],
        [5.63579, 15.61871],
        [10.34917, 13.18445],
        [13.94597, -10.39915],
        [-8.80072, -13.80477],
        [-16.15603, 6.74104],
    ],
    axis: [-0.716481518039, 0.697606073875, 0.0],
    angle: 54.314665287,
    vent: 14.0,
};

/// Cells 12 and 54 from 60 mm (polar angles 23° and 50°, some 29° apart:
/// they do not meet).
const CELL12: Cell = Cell {
    near: &[
        [3.2489, 4.40096],
        [-2.59632, 4.73781],
        [-5.60987, -2.58366],
        [-5.4072, -3.09794],
        [1.79008, -5.3116],
        [5.4606, -2.68955],
    ],
    from: 60.0,
    far: &[
        [9.20523, 12.4694],
        [-7.35624, 13.42379],
        [-15.89463, -7.32038],
        [-15.32039, -8.7775],
        [5.0719, -15.04954],
        [15.47171, -7.62038],
    ],
    axis: [0.624707383802, -0.780858940285, 0.0],
    angle: 22.581637152,
    vent: 14.0,
};

const CELL54: Cell = Cell {
    near: &[
        [4.73289, 2.9341],
        [-2.89332, 5.42169],
        [-3.44412, 5.09354],
        [-5.13789, -3.23214],
        [-4.47979, -4.03896],
        [3.52227, -4.72607],
        [4.85925, -3.36936],
    ],
    from: 60.0,
    far: &[
        [13.40986, 8.31329],
        [-8.19775, 15.36147],
        [-9.75835, 14.43169],
        [-14.55735, -9.15772],
        [-12.69272, -11.44373],
        [9.97975, -13.39054],
        [13.76788, -9.54651],
    ],
    axis: [0.396100439176, -0.918207189083, 0.0],
    angle: 49.959170756,
    vent: 14.0,
};

fn cell(k: f64, c: &Cell) -> Body {
    frustum(
        k,
        (c.near, c.from),
        (c.far, 170.0),
        Vec3::new(c.axis[0], c.axis[1], c.axis[2]),
        c.angle,
    )
}

/// The same cell with its far section the near one scaled exactly, so
/// that its sides are planes: the frustum of the helmet's sketches has
/// its far corners rounded to five decimals, which makes each side a
/// bilinear patch a hundred-thousandth of a millimetre off a plane.
fn planar_cell(k: f64, c: &Cell) -> Body {
    let far: Vec<P> = c
        .near
        .iter()
        .map(|p| [p[0] * 170.0 / c.from, p[1] * 170.0 / c.from])
        .collect();
    frustum(
        k,
        (c.near, c.from),
        (&far, 170.0),
        Vec3::new(c.axis[0], c.axis[1], c.axis[2]),
        c.angle,
    )
}

#[track_caller]
fn near(actual: f64, expected: f64, rel: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= rel * expected.abs().max(actual.abs()),
        "{what}: {actual} is not {expected} (off by {:e})",
        (actual - expected).abs() / expected.abs().max(actual.abs())
    );
}

/// The three booleans of the shell and a cell at a scale: valid, held to
/// the identities of sets (within what curves fitted to the tolerance
/// allow), and the common part held to the same boolean
/// with the planar twin of the cell (whose sides the shell is walked
/// against, the other way round: its field is the plane's).
fn cuts(c: &Cell, k: f64) {
    let tol = tolerance(k);
    let (shell, tool, twin) = (shell(k, c.vent), cell(k, c), planar_cell(k, c));
    let run = |a: &Body, b: &Body, op| match boolean(a, b, op, &tol) {
        Ok(r) => {
            assert_eq!(check(&r.body, &tol), [], "{op:?} at {k}");
            mass(&r.body)
        }
        Err(e) => panic!("{op:?} at {k}: {e}"),
    };
    let (u, i, d) = (
        run(&shell, &tool, Union),
        run(&shell, &tool, Intersect),
        run(&shell, &tool, Subtract),
    );
    let (a, b) = (mass(&shell), mass(&tool));
    near(i.volume + d.volume, a.volume, 1e-7, "parts");
    near(u.volume + i.volume, a.volume + b.volume, 1e-7, "union");
    near(u.area + i.area, a.area + b.area, 1e-7, "area");
    assert!(i.volume > 1e-3 * b.volume, "nothing in common at {k}");
    // The twin's sides are off the cell's by 1e-5 mm at most, over some
    // 1500 mm² of side within the shell: 0.015 mm³ against some 1200.
    let planar = run(&shell, &twin, Intersect);
    near(
        i.volume,
        planar.volume,
        2e-5,
        "common, against the planar twin",
    );
}

#[test]
fn a_frustum_with_freeform_sides_cuts_a_shell_near_its_pole() {
    for k in SCALES {
        cuts(&CELL8, k);
    }
}

#[test]
fn a_frustum_with_freeform_sides_cuts_a_shell_around_its_pole() {
    for k in SCALES {
        cuts(&CELL1, k);
    }
}

#[test]
fn a_frustum_with_freeform_sides_cuts_a_shell_with_a_vent() {
    for k in SCALES {
        cuts(&CELL3, k);
    }
}

#[test]
fn a_frustum_with_freeform_sides_cuts_a_shell_low_on_its_side() {
    for k in SCALES {
        cuts(&CELL63, k);
    }
}

/// Two cells that do not meet: their union is both, as two shells, they
/// have nothing in common, and the one less the other is the one. (The
/// bound below the distance from a segment to a lofted side was cut short
/// once it fell under its threshold and returned the nearest piece seen so
/// far, not the nearest piece: a ray through the frustum was proven clear
/// of the side it leaves by, the far frustum's face was taken for inside
/// the near one and left out, and the result had edges used once.)
#[test]
fn two_frustums_apart_unite_into_two_shells() {
    for k in SCALES {
        let tol = tolerance(k);
        let (a, b) = (cell(k, &CELL12), cell(k, &CELL54));
        let (ma, mb) = (mass(&a), mass(&b));
        let run = |op| match boolean(&a, &b, op, &tol) {
            Ok(r) => {
                assert_eq!(check(&r.body, &tol), [], "{op:?} at {k}");
                Some(r.body)
            }
            Err(KernelError::EmptyResult { .. }) => None,
            Err(e) => panic!("{op:?} at {k}: {e}"),
        };
        let both = run(Union).unwrap();
        assert_eq!(both.shells.len(), 2, "at {k}");
        near(mass(&both).volume, ma.volume + mb.volume, 1e-12, "union");
        near(mass(&both).area, ma.area + mb.area, 1e-12, "union area");
        assert!(run(Intersect).is_none(), "at {k}");
        let rest = run(Subtract).unwrap();
        near(mass(&rest).volume, ma.volume, 1e-12, "difference");
        // The point of the far frustum's face that was taken for inside
        // the near one, by the ray casting alone.
        let p = Vec3::new(-43.34176271351926, -23.01408217567925, 38.169427535269126) * k;
        assert_eq!(
            contains_points(&a, &[p], &tol).unwrap(),
            [Some(false)],
            "at {k}"
        );
    }
}
