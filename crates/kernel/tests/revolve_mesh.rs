//! The tessellator meshes what a revolution makes: a closed, consistently
//! oriented mesh on every face, whose volume converges to the exact one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod revolve_support;

use common::{catalogue, frames};
use craft3d_kernel::profile::SketchCurve;
use craft3d_math::Frame;
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, mass_properties};
use revolve_support::{Axis, build, clear_axis, half_disc, lathe_polygon, opposite, tolerance};
use std::f64::consts::{PI, TAU};

const SWEEPS: [(f64, f64); 4] = [(0.0, TAU), (-0.7, -0.7 + 1.5 * PI), (2.0, 2.3), (-3.0, 3.1)];

/// Meshes `body` at two chordal tolerances and checks both meshes. Returns
/// what went wrong, if anything.
fn mesh_check(body: &Body, k: f64) -> Result<(), String> {
    let exact = mass_properties(body).map_err(|e| e.to_string())?;
    // Convergence is the bound itself, at every chord down to a sixteenth
    // of the first. (The error need not fall from one chord to the next:
    // a body with convex and concave faces has errors of both signs that
    // cancel by chance, and a thin sliver is meshed with the same few
    // segments at several chords.)
    for chord in [0.02 * k, 0.005 * k, 0.00125 * k] {
        let params = TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        };
        let mesh = tessellate_body(body, &params).map_err(|e| format!("at {chord}: {e}"))?;
        for face in 0..body.faces.len() {
            if !mesh.face_ids.contains(&(face as u32)) {
                return Err(format!("at {chord}: face {face} has no triangles"));
            }
        }
        let welded = mesh
            .weld()
            .map_err(|t| format!("at {chord}: triangle {t} is broken"))?;
        if !welded.is_closed() {
            return Err(format!("at {chord}: the mesh is not a closed manifold"));
        }
        let error = (welded.volume() - exact.volume).abs();
        // A mesh within `chord` of the surface encloses the exact volume
        // to within the area times the chord.
        if error > exact.area * chord {
            return Err(format!(
                "at {chord}: mesh volume {} vs {} is further off than area × chord",
                welded.volume(),
                exact.volume
            ));
        }
        // The mesh's own area, at the finest chord only: a chord of
        // sagitta s on a radius r is shorter than its arc by s / 3r, which
        // at the coarse chords is several per cent on the small radii
        // here, and under 1 % at the finest.
        let area = mesh.area();
        if chord < 0.002 * k && (area - exact.area).abs() > 0.01 * exact.area {
            return Err(format!("at {chord}: mesh area {area} vs {}", exact.area));
        }
    }
    Ok(())
}

fn run(
    name: &str,
    curves: &[SketchCurve],
    plane: &Frame,
    axis: Axis,
    k: f64,
    failures: &mut Vec<String>,
) -> usize {
    let mut count = 0;
    for sweep in SWEEPS {
        let result = build(curves, plane, axis, sweep, &tolerance(k)).unwrap();
        if let Err(why) = mesh_check(&result.body, k) {
            failures.push(format!(
                "{name} at scale {k}, axis {axis:?}, {}..{}: {why}",
                sweep.0, sweep.1
            ));
        }
        count += 1;
    }
    count
}

/// Every catalogue profile about two axes clear of it.
#[test]
fn the_catalogue_s_revolutions_are_meshed() {
    let mut failures = Vec::new();
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        let plane = frames(k)[1];
        for case in catalogue(k) {
            let up = clear_axis(&case.curves, [0.0, 2.0], 3.0 * k);
            let oblique = opposite(clear_axis(&case.curves, [-0.6, -0.3], 0.5 * k));
            for axis in [up, oblique] {
                // The catalogue's profiles are tens of units across.
                count += run(
                    case.name,
                    &case.curves,
                    &plane,
                    axis,
                    10.0 * k,
                    &mut failures,
                );
            }
        }
    }
    println!("{count} bodies meshed, {} failures", failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Profiles on the axis: faces through poles and apexes, discs, a cavity.
#[test]
fn revolutions_that_touch_the_axis_are_meshed() {
    let mut failures = Vec::new();
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        let plane = frames(k)[1];
        let axis = Axis {
            point: [-2.0 * k, 0.5 * k],
            direction: [-1.0, 0.4],
        };
        for side in [1.0, -1.0] {
            let poly = |points: &[(f64, f64)]| {
                let scaled: Vec<(f64, f64)> = points.iter().map(|&(r, h)| (r * k, h * k)).collect();
                lathe_polygon(axis, side, &scaled)
            };
            let cases = vec![
                (
                    "cylinder",
                    poly(&[(0.0, 0.0), (1.5, 0.0), (1.5, 2.5), (0.0, 2.5)]),
                ),
                ("cone", poly(&[(0.0, 0.0), (1.5, 0.0), (0.0, 2.5)])),
                ("ball", half_disc(axis, side, 0.7 * k, 1.5 * k)),
                (
                    "triangle on its tip",
                    poly(&[(0.0, 1.0), (2.0, 0.0), (2.0, 2.0)]),
                ),
                (
                    "chevron",
                    poly(&[(0.0, 0.0), (2.0, 1.0), (0.0, 2.0), (1.0, 1.0)]),
                ),
                (
                    "cup",
                    poly(&[
                        (0.0, 0.0),
                        (2.0, 0.0),
                        (2.0, 2.0),
                        (1.5, 2.0),
                        (1.5, 0.5),
                        (0.0, 0.5),
                    ]),
                ),
            ];
            for (name, curves) in cases {
                count += run(name, &curves, &plane, axis, k, &mut failures);
            }
        }
    }
    println!("{count} bodies meshed, {} failures", failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
