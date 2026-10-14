//! Booleans found by modelling a helmet: a shell turned from two splines
//! that close on the axis (a pole on each), cut by radial frustums.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;
mod common;

use boolean_common::*;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::boolean::boolean;
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
