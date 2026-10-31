//! The inertia tensor from the exact B-rep against closed forms: a block,
//! a cylinder, a sphere and a torus, tilted and moved off the origin, at
//! three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_range_loop
)]

use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::{inertia, mass_and_inertia, mass_properties};

/// A frame tilted about all three axes, away from the origin.
fn tilted(scale: f64) -> Frame {
    Frame::new(
        Vec3::new(3.0, -2.0, 5.0) * scale,
        Vec3::new(0.3, -0.5, 0.8),
        Vec3::new(1.0, 0.7, 0.1),
    )
    .unwrap()
}

/// `R diag(d) Rᵀ`, with the frame's axes as the columns of `R`.
fn in_world(frame: &Frame, d: [f64; 3]) -> [[f64; 3]; 3] {
    let axes = [frame.x(), frame.y(), frame.z()];
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3)
                .map(|k| {
                    let a = axes[k];
                    let c = |v: Vec3, n: usize| [v.x, v.y, v.z][n];
                    c(a, i) * c(a, j) * d[k]
                })
                .sum();
        }
    }
    out
}

fn assert_tensor(name: &str, got: [[f64; 3]; 3], want: [[f64; 3]; 3]) {
    let size = want.iter().flatten().fold(0.0_f64, |m, x| m.max(x.abs()));
    for i in 0..3 {
        for j in 0..3 {
            let error = (got[i][j] - want[i][j]).abs() / size;
            assert!(
                error < 1e-9,
                "{name}: I[{i}][{j}] is {} but {} (relative error {error:e})",
                got[i][j],
                want[i][j]
            );
        }
    }
}

#[test]
fn the_tensors_of_the_reference_solids_are_their_closed_forms_at_three_scales() {
    for scale in [1e-3, 1.0, 1e3] {
        let frame = tilted(scale);
        let (a, b, c) = (2.0 * scale, 3.0 * scale, 5.0 * scale);
        let block = solids::block(&frame, a, b, c).unwrap();
        let v = a * b * c;
        let want = in_world(
            &frame,
            [
                v * (b * b + c * c) / 12.0,
                v * (a * a + c * c) / 12.0,
                v * (a * a + b * b) / 12.0,
            ],
        );
        let got = inertia(&block.body).unwrap();
        assert!((got.volume - v).abs() < 1e-12 * v);
        assert!((got.centroid - block.centroid).length() < 1e-9 * scale);
        assert_tensor(&format!("block at {scale}"), got.tensor, want);

        let (r, h) = (1.5 * scale, 4.0 * scale);
        let cylinder = solids::cylinder(&frame, r, h).unwrap();
        let v = std::f64::consts::PI * r * r * h;
        let side = v * (3.0 * r * r + h * h) / 12.0;
        let want = in_world(&frame, [side, side, v * r * r / 2.0]);
        let got = inertia(&cylinder.body).unwrap();
        assert!((got.centroid - cylinder.centroid).length() < 1e-9 * scale);
        assert_tensor(&format!("cylinder at {scale}"), got.tensor, want);

        let r = 2.5 * scale;
        let sphere = solids::sphere(&frame, r).unwrap();
        let v = 4.0 / 3.0 * std::f64::consts::PI * r * r * r;
        let i = 0.4 * v * r * r;
        let got = inertia(&sphere.body).unwrap();
        assert!((got.centroid - sphere.centroid).length() < 1e-9 * scale);
        assert_tensor(
            &format!("sphere at {scale}"),
            got.tensor,
            in_world(&frame, [i; 3]),
        );

        let (major, minor) = (4.0 * scale, 1.0 * scale);
        let torus = solids::torus(&frame, major, minor).unwrap();
        let v = 2.0 * std::f64::consts::PI.ppowi(2) * major * minor * minor;
        let axial = v * (major * major + 0.75 * minor * minor);
        let side = v * (0.5 * major * major + 0.625 * minor * minor);
        let got = inertia(&torus.body).unwrap();
        assert_tensor(
            &format!("torus at {scale}"),
            got.tensor,
            in_world(&frame, [side, side, axial]),
        );
    }
}

#[test]
fn inertia_agrees_with_the_mass_properties_it_extends() {
    let frame = tilted(1.0);
    for reference in solids::all(&frame, 1.0).unwrap() {
        let mass = mass_properties(&reference.body).unwrap();
        let got = inertia(&reference.body).unwrap();
        assert!(
            (got.volume - mass.volume).abs() <= 1e-12 * mass.volume.abs(),
            "{}",
            reference.name
        );
        let centroid = mass.centroid.unwrap();
        assert!(
            (got.centroid - centroid).length() < 1e-9,
            "{}",
            reference.name
        );
        // Symmetric, with positive principal moments that satisfy the
        // triangle inequality: Ixx + Iyy >= Izz and so on.
        let t = got.tensor;
        for i in 0..3 {
            for j in 0..3 {
                assert_eq!(t[i][j], t[j][i], "{}", reference.name);
            }
        }
        let d = [t[0][0], t[1][1], t[2][2]];
        assert!(d.iter().all(|&x| x > 0.0), "{}", reference.name);
        assert!(d[0] + d[1] >= d[2] * (1.0 - 1e-9), "{}", reference.name);
    }
}

/// `mass_and_inertia` integrates every face once, with the second
/// moments, and takes the sums without them where those settle: what it
/// gives is what `mass_properties` and `inertia` give on their own, bit
/// for bit (and error for error), on analytic and freeform faces alike.
#[test]
fn mass_and_inertia_at_once_are_each_bit_for_bit_what_they_are_alone() {
    let frame = tilted(1.0);
    let mut bodies = solids::all(&frame, 1.0).unwrap();
    bodies.extend(trimmed::all(&frame, 1.0).unwrap());
    bodies.extend(trimmed::all(&tilted(1e3), 1e3).unwrap());
    for nurbs in [false, true] {
        bodies.push(trimmed::domed_block(&frame, 4.0, 3.0, 2.0, nurbs).unwrap());
    }
    assert!(bodies.len() > 10);
    for reference in &bodies {
        let (mass, turned) = mass_and_inertia(&reference.body);
        assert_eq!(mass, mass_properties(&reference.body), "{}", reference.name);
        assert_eq!(turned, inertia(&reference.body), "{}", reference.name);
    }
}
