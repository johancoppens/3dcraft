//! A curve imprinted on a face that leaves a vertex of the face's boundary
//! in the direction of the boundary and stays within a few tolerances of
//! it for longer than the pieces at that vertex reach (checklist item 2,
//! "tangent contact of higher order"; 2026-10-21).
//!
//! Two models hit it: the Perseverance's drill tip, a cone whose base
//! circle lies a few hundred tolerances off a cylinder's wall along a
//! tilted axis (its seam a few hundred tolerances from where the curve of
//! cone and wall crosses the base circle), and the Garabit pier's stair,
//! a tread whose end face touches the inside of the stringer near its
//! corner. Both are reduced here to analytic bodies, at three scales and
//! turned off the world's axes, and held to their closed forms, the
//! identities of sets and closed meshes.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::{BooleanOp, BooleanResult, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::Body;
use std::f64::consts::PI;

use BooleanOp::{Intersect, Subtract, Union};

/// A turn of space: the images of the world's axes.
#[derive(Clone, Copy)]
struct Turn([Vec3; 3]);

impl Turn {
    /// About `axis` by `angle` (Rodrigues).
    fn about(axis: [f64; 3], angle: f64) -> Turn {
        let k = Vec3::new(axis[0], axis[1], axis[2]).normalized().unwrap();
        let (s, c) = angle.sin_cos();
        let image = |v: Vec3| v * c + k.cross(v) * s + k * (k.dot(v) * (1.0 - c));
        Turn([
            image(Vec3::new(1.0, 0.0, 0.0)),
            image(Vec3::new(0.0, 1.0, 0.0)),
            image(Vec3::new(0.0, 0.0, 1.0)),
        ])
    }

    fn vector(&self, v: [f64; 3]) -> Vec3 {
        self.0[0] * v[0] + self.0[1] * v[1] + self.0[2] * v[2]
    }

    /// The frame at `origin` (times `k`) whose z and x axes are the
    /// turned `z` and `x`.
    fn frame(&self, k: f64, origin: [f64; 3], z: [f64; 3], x: [f64; 3]) -> Frame {
        let o = self.vector(origin) * k;
        Frame::new(o, self.vector(z), self.vector(x)).unwrap()
    }
}

/// Square to the world, and three turns off it.
fn turns() -> [Turn; 4] {
    [
        Turn::about([0.0, 0.0, 1.0], 0.0),
        Turn::about([1.0, 1.0, 1.0], 0.3),
        Turn::about([0.2, -1.0, 0.5], 1.1),
        Turn::about([1.0, 0.0, 0.0], 0.5 * PI - 0.01),
    ]
}

/// Whether the mesh of a body at the chordal tolerance `chord` is closed
/// and encloses its volume.
#[track_caller]
fn assert_meshed(body: &Body, chord: f64, what: &str) {
    let params = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
    let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert!(welded.is_closed(), "{what}: the mesh is not watertight");
    let exact = mass(body);
    assert!(
        (welded.volume() - exact.volume).abs() <= exact.area * chord,
        "{what}: mesh volume {} against {}",
        welded.volume(),
        exact.volume
    );
}

/// A volume within the tolerance times the result's area of what it
/// should be.
#[track_caller]
fn assert_volume(result: &BooleanResult, expected: f64, k: f64, what: &str) {
    let m = mass(&result.body);
    let slack = tolerance(k).linear * m.area;
    assert!(
        (m.volume - expected).abs() <= slack,
        "{what}: volume {} is not {expected} (off by {:e}, slack {slack:e})",
        m.volume,
        (m.volume - expected).abs()
    );
}

/// `∫₀ᵃ (R − √(R² − y²)) dy`: the area between a circle of radius `r`
/// and its tangent, across `a` from the point of contact.
fn beyond_circle(r: f64, a: f64) -> f64 {
    a * r - 0.5 * (a * (r * r - a * a).sqrt() + r * r * (a / r).asin())
}

/// The drill tip of the Perseverance (2026-10-21), at scale 1 a cylinder
/// of radius 1 and height 3 and a cone of radius 1 and height 2 standing
/// on it 0.1 below its lid, along one axis, the cone's base pushed `e`
/// off it square to the axis (13 tolerances; the rover's was 3.9e-4 of
/// 30). The curve of cone and wall then crosses the base circle at two
/// points, at an angle of about `2 e h / R²` (2.6e-5), and the cone's
/// seam is put `gap` along the circle from one of them, on the side the
/// curve leaves along: the short piece of the base circle there and the
/// curve leave the point together and are within the tolerance of each
/// other for much further than the short piece reaches. Refused "two
/// curves leave a point in the same direction" until 2026-10-21 (a
/// negative `gap`, the seam on the other side, was built before).
#[test]
fn a_cone_whose_base_circle_lies_off_a_cylinder_wall() {
    let (radius, height, tip, sunk, e) = (1.0, 3.0, 2.0, 0.1, 1.3e-5);
    let cylinder_volume = PI * radius * radius * height;
    let cone_volume = PI * radius * radius * tip / 3.0;
    // Above the lid: a cone of height `tip − sunk`.
    let top = radius * (1.0 - sunk / tip);
    let above = PI * top * top * (tip - sunk) / 3.0;
    for k in SCALES {
        for (t, turn) in turns().iter().enumerate() {
            for gap in [-1e-4, 1e-5, 1e-4, 1e-3] {
                let what = format!("scale {k}, turn {t}, seam {gap} from the crossing");
                let cylinder = primitives::cylinder(
                    &turn.frame(k, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
                    radius * k,
                    height * k,
                )
                .unwrap();
                // Pushed along x: the curves cross at ±y, and the curve
                // leaves +y towards +x; the seam is `gap` round from +y
                // towards +x.
                let seam = 0.5 * PI - gap / radius;
                let cone = primitives::cone(
                    &turn.frame(
                        k,
                        [e, 0.0, height - sunk],
                        [0.0, 0.0, 1.0],
                        [seam.cos(), seam.sin(), 0.0],
                    ),
                    radius * k,
                    tip * k,
                )
                .unwrap();
                let k3 = k * k * k;
                let union = solve(&cylinder, &cone, Union, k);
                assert_volume(&union, (cylinder_volume + above) * k3, k, &what);
                let common = solve(&cylinder, &cone, Intersect, k);
                assert_volume(&common, (cone_volume - above) * k3, k, &what);
                // The identity of sets, on the volumes the two make.
                let (u, c) = (mass(&union.body), mass(&common.body));
                let slack = tolerance(k).linear * (u.area + c.area);
                let sum = mass(&cylinder).volume + mass(&cone).volume;
                assert!((u.volume + c.volume - sum).abs() <= slack, "{what}");
                // The crescents between the base circle and the curve
                // are 26 tolerances across: meshed with chords that sag
                // less than that.
                let chord = e * k;
                assert_meshed(&union.body, chord, &format!("{what}: union"));
                assert_meshed(&common.body, chord, &format!("{what}: common"));
                // The cylinder less the cone keeps a ring that stands on
                // the rest along a strip at most `e` across, which ends
                // where the curves cross: where a refusal as zero
                // thickness is made, it is held to a place where parts
                // touch, and the result as made without it.
                let (less, _) = solve_parts(&cylinder, &cone, Subtract, k);
                assert_volume(
                    &less,
                    (cylinder_volume - cone_volume + above) * k3,
                    k,
                    &what,
                );
                let (rest, _) = solve_parts(&cone, &cylinder, Subtract, k);
                assert_volume(&rest, above * k3, k, &what);
            }
        }
    }
}

/// The tread of the Garabit pier's stair (2026-10-21), at scale 1 a
/// block whose end face touches the inside of a tube of radius 1 along a
/// line `notch` from one of its sides (the stair's stringer is a sweep
/// along a helix; here a tube): on its top and bottom faces the circle
/// of the tube leaves the end edge where it touches it, and on the short
/// side stays within the tolerance of it all the way to the corner (at
/// `notch` = 8e-3 they are 32 tolerances apart there, and within 4
/// tolerances of each other for a third of the way). Refused "two
/// curves leave a point in the same direction" until 2026-10-21.
#[test]
fn a_block_whose_end_touches_the_inside_of_a_tube_near_its_corner() {
    let (radius, wall, tall) = (1.0, 0.02, 0.5);
    let (inner, wide, low, thick) = (0.1, 0.1, 0.2, 0.1);
    let tube_volume = PI * ((radius + wall) * (radius + wall) - radius * radius) * tall;
    for k in SCALES {
        for (t, turn) in turns().iter().enumerate() {
            let outer = primitives::cylinder(
                &turn.frame(k, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
                (radius + wall) * k,
                tall * k,
            )
            .unwrap();
            let bore = primitives::cylinder(
                &turn.frame(k, [0.0, 0.0, -0.1], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
                radius * k,
                (tall + 0.2) * k,
            )
            .unwrap();
            let tube = boolean(&outer, &bore, Subtract, &tolerance(k))
                .unwrap()
                .body;
            for notch in [8e-3, 1e-2] {
                let what = format!("scale {k}, turn {t}, notch {notch}");
                let block = primitives::block(
                    &turn.frame(k, [inner, -notch, low], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
                    (radius - inner) * k,
                    (notch + wide) * k,
                    thick * k,
                )
                .unwrap();
                let k3 = k * k * k;
                let block_volume = (radius - inner) * (notch + wide) * thick;
                let common_volume =
                    thick * (beyond_circle(radius, notch) + beyond_circle(radius, wide));
                let union = solve(&tube, &block, Union, k);
                assert_volume(
                    &union,
                    (tube_volume + block_volume - common_volume) * k3,
                    k,
                    &what,
                );
                // The common part is two pieces that meet along the line
                // the end touches the tube on: zero thickness.
                let (common, refused) = solve_parts(&tube, &block, Intersect, k);
                assert!(refused.is_some(), "{what}: the common part is not refused");
                assert_volume(&common, common_volume * k3, k, &what);
                let (notched, _) = solve_parts(&tube, &block, Subtract, k);
                assert_volume(&notched, (tube_volume - common_volume) * k3, k, &what);
                let (trimmed, _) = solve_parts(&block, &tube, Subtract, k);
                assert_volume(&trimmed, (block_volume - common_volume) * k3, k, &what);
                let (u, c) = (mass(&union.body), mass(&common.body));
                let slack = tolerance(k).linear * (u.area + c.area);
                let sum = mass(&tube).volume + mass(&block).volume;
                assert!((u.volume + c.volume - sum).abs() <= slack, "{what}");
                assert_meshed(&union.body, 5e-3 * k, &format!("{what}: union"));
                assert_meshed(&notched.body, 5e-3 * k, &format!("{what}: notched"));
                assert_meshed(&trimmed.body, 5e-3 * k, &format!("{what}: trimmed"));
            }
        }
    }
}
