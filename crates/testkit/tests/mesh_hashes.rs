//! The meshes of a set of bodies as hashes, to compare before and after a
//! change to the tessellator that should leave them alone; and, for the
//! bodies with freeform faces, how far their meshes are from the surfaces
//! they stand for. Not a test of anything on its own account (but for the
//! thread counts, which must agree): run it for the numbers, in release,
//! and compare two runs with `diff`:
//!
//! `cargo test --release -p craft3d-testkit --test mesh_hashes -- --ignored --nocapture`
//!
//! Lines start with `hash` (one per body, size and tolerance) and with
//! `freeform` (triangles, measured deviation over the tolerance, time).

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Surface;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body_with_threads};
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::Body;
use std::f64::consts::{PI, TAU};
use std::time::Instant;

type P = [f64; 2];

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    }
}

/// FNV-1a over every bit of the mesh.
fn hash(mesh: &Mesh) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |x: u64| {
        for byte in x.to_le_bytes() {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for p in mesh.positions.iter().chain(&mesh.normals) {
        for x in [p.x, p.y, p.z] {
            eat(x.to_bits());
        }
    }
    for t in &mesh.triangles {
        for &v in t {
            eat(u64::from(v));
        }
    }
    for &f in &mesh.face_ids {
        eat(u64::from(f));
    }
    h
}

fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

fn line(id: u64, start: P, end: P) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

fn spline(id: u64, degree: usize, control: &[P]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree,
            knots: knots(degree, control.len()),
            control: control.to_vec(),
        }),
    }
}

fn scaled(points: &[P], k: f64) -> Vec<P> {
    points.iter().map(|p| [p[0] * k, p[1] * k]).collect()
}

/// The wall of a vase, from its foot at (3, 0) up to its rim at (2, 10).
const VASE: [P; 7] = [
    [3.0, 0.0],
    [5.5, 1.0],
    [6.0, 4.0],
    [1.0, 5.5],
    [1.5, 8.0],
    [3.5, 9.0],
    [2.0, 10.0],
];

/// The section of a vase in the sketch plane, the axis along y.
fn vase(k: f64, degree: usize) -> Vec<SketchCurve> {
    vec![
        line(1, [0.0, 10.0 * k], [0.0, 0.0]),
        line(2, [0.0, 0.0], [3.0 * k, 0.0]),
        spline(3, degree, &scaled(&VASE, k)),
        line(4, [2.0 * k, 10.0 * k], [0.0, 10.0 * k]),
    ]
}

/// A leaf: two cubic splines between the same two points.
fn leaf(k: f64, fat: f64) -> Vec<SketchCurve> {
    vec![
        spline(
            1,
            3,
            &[
                [-2.0 * k, 0.0],
                [-k, -fat * k],
                [0.5 * k, -1.2 * fat * k],
                [1.5 * k, -0.6 * fat * k],
                [2.0 * k, 0.0],
            ],
        ),
        spline(
            2,
            3,
            &[
                [2.0 * k, 0.0],
                [1.0 * k, fat * k],
                [-k, 0.8 * fat * k],
                [-2.0 * k, 0.0],
            ],
        ),
    ]
}

fn section(curves: &[SketchCurve], plane: Frame) -> Section {
    let profiles = detect_profiles(curves, &Tolerance::default()).unwrap();
    Section {
        plane,
        outline: profiles[0].outer.clone(),
        start: None,
    }
}

/// Bodies of the order of `k` in size, with a name.
fn bodies(k: f64) -> Vec<(String, Body)> {
    let tol = Tolerance::default();
    let mut all: Vec<(String, Body)> = Vec::new();
    let mut references = solids::all(&Frame::WORLD, k).unwrap();
    references.extend(trimmed::all(&Frame::WORLD, k).unwrap());
    references.push(trimmed::pipe_elbow(&Frame::WORLD, 3.0 * k, k, 1.9).unwrap());
    references.push(trimmed::sphere_octant(&Frame::WORLD, 2.0 * k).unwrap());
    // (Two of the trimmed solids are domed blocks: tell them apart.)
    for (n, reference) in references.into_iter().enumerate() {
        all.push((format!("{} #{n}", reference.name), reference.body));
    }
    // The same solids in a frame that is not the world's.
    let askew = Frame::new(
        Vec3::new(3.0 * k, -2.0 * k, 5.0 * k),
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(2.0, -1.0, 0.0),
    )
    .unwrap();
    let mut tilted = solids::all(&askew, k).unwrap();
    tilted.extend(trimmed::all(&askew, k).unwrap());
    for (n, reference) in tilted.into_iter().enumerate() {
        all.push((format!("askew {} #{n}", reference.name), reference.body));
    }

    // Boolean results.
    let boxed = |lo: [f64; 3], hi: [f64; 3]| {
        let frame = Frame::WORLD
            .at(Vec3::new(lo[0] * k, lo[1] * k, lo[2] * k))
            .unwrap();
        primitives::block(
            &frame,
            (hi[0] - lo[0]) * k,
            (hi[1] - lo[1]) * k,
            (hi[2] - lo[2]) * k,
        )
        .unwrap()
    };
    let rod = |base: [f64; 3], radius: f64, height: f64| {
        let frame = Frame::WORLD
            .at(Vec3::new(base[0] * k, base[1] * k, base[2] * k))
            .unwrap();
        primitives::cylinder(&frame, radius * k, height * k).unwrap()
    };
    let scaled_tol = Tolerance {
        linear: tol.linear * k.max(1e-3),
        ..tol
    };
    let run = |a: &Body, b: &Body, op| boolean(a, b, op, &scaled_tol).map(|r| r.body);
    let plate = boxed([0.0, 0.0, 0.0], [4.0, 3.0, 1.0]);
    let mut booleans = vec![
        (
            "plate with a hole",
            run(
                &plate,
                &rod([1.5, 1.25, -1.0], 0.75, 3.0),
                BooleanOp::Subtract,
            ),
        ),
        (
            "plate with a blind hole",
            run(
                &plate,
                &rod([2.0, 1.5, 0.25], 0.5, 3.0),
                BooleanOp::Subtract,
            ),
        ),
        (
            "post on a plate",
            run(&plate, &rod([2.0, 1.5, 1.0], 0.5, 3.0), BooleanOp::Union),
        ),
        (
            "two blocks",
            run(
                &plate,
                &boxed([1.0, 1.0, 0.5], [5.0, 2.0, 2.0]),
                BooleanOp::Union,
            ),
        ),
        (
            "block and rod in common",
            run(
                &plate,
                &rod([0.0, 0.0, -1.0], 2.0, 4.0),
                BooleanOp::Intersect,
            ),
        ),
    ];
    let ball = primitives::sphere(
        &Frame::WORLD
            .at(Vec3::new(2.0 * k, 1.5 * k, 1.0 * k))
            .unwrap(),
        0.8 * k,
    )
    .unwrap();
    booleans.push(("ball on a plate", run(&plate, &ball, BooleanOp::Union)));
    booleans.push(("plate less a ball", run(&plate, &ball, BooleanOp::Subtract)));
    for nurbs in [false, true] {
        let dome = trimmed::domed_block(&Frame::WORLD, 3.0 * k, 2.0 * k, 1.5 * k, nurbs)
            .unwrap()
            .body;
        let hole = rod([2.0, 1.2, -1.0], 0.5, 5.0);
        booleans.push((
            if nurbs {
                "domed block (nurbs) with a hole"
            } else {
                "domed block with a hole"
            },
            run(&dome, &hole, BooleanOp::Subtract),
        ));
        let cut = boxed([1.0, -1.0, 0.5], [2.0, 3.0, 4.0]);
        booleans.push((
            if nurbs {
                "domed block (nurbs) less a block"
            } else {
                "domed block less a block"
            },
            run(&dome, &cut, BooleanOp::Subtract),
        ));
    }
    for (name, result) in booleans {
        match result {
            Ok(body) => all.push((name.to_string(), body)),
            Err(error) => println!("skipped {name} at {k}: {error}"),
        }
    }

    // Sweeps of splines.
    let sweeps: Vec<(String, Result<Body, String>)> = vec![
        (
            "extruded leaf".into(),
            detect_profiles(&leaf(k, 1.0), &tol)
                .map_err(|e| e.to_string())
                .and_then(|p| {
                    extrude(&p, &Frame::WORLD, 0.0, 3.0 * k, &tol)
                        .map(|r| r.body)
                        .map_err(|e| e.to_string())
                }),
        ),
        (
            "lofted leaves".into(),
            loft(
                &[
                    section(&leaf(k, 1.0), Frame::WORLD),
                    section(
                        &leaf(1.5 * k, 1.4),
                        Frame::WORLD.at(Vec3::new(0.0, 0.0, 4.0 * k)).unwrap(),
                    ),
                    section(
                        &leaf(0.8 * k, 0.7),
                        Frame::WORLD.at(Vec3::new(0.0, 0.0, 7.0 * k)).unwrap(),
                    ),
                ],
                &Options::default(),
                &tol,
            )
            .map(|l| l.body)
            .map_err(|e| e.to_string()),
        ),
    ];
    for (name, result) in sweeps {
        match result {
            Ok(body) => all.push((name, body)),
            Err(error) => println!("skipped {name} at {k}: {error}"),
        }
    }
    for degree in [2, 3, 5] {
        for (from, to, turn) in [
            (0.0, TAU, "full"),
            (-0.7, -0.7 + 1.5 * PI, "three quarters"),
        ] {
            let name = format!("vase of degree {degree}, {turn} turn");
            let made = detect_profiles(&vase(k, degree), &tol)
                .map_err(|e| e.to_string())
                .and_then(|p| {
                    revolve(&p, &Frame::WORLD, [0.0, 0.0], [0.0, 1.0], from, to, &tol)
                        .map_err(|e| e.to_string())
                });
            match made {
                Ok(result) => all.push((name, result.body)),
                Err(error) => println!("skipped {name} at {k}: {error}"),
            }
        }
    }
    all
}

/// A length the body is of the order of: the diagonal of its vertices,
/// or `k` for a body without any.
fn size(body: &Body, k: f64) -> f64 {
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for v in &body.vertices {
        for (i, x) in [v.point.x, v.point.y, v.point.z].into_iter().enumerate() {
            low[i] = low[i].min(x);
            high[i] = high[i].max(x);
        }
    }
    let d = (0..3)
        .map(|i| (high[i] - low[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    if d.is_finite() && d > 0.0 { d } else { k }
}

#[test]
#[ignore = "prints hashes of meshes; run in release with --ignored --nocapture"]
fn hashes_of_meshes() {
    let mut count = 0;
    for k in [1e-3, 1.0, 1e3] {
        for (name, body) in bodies(k) {
            for relative in [1e-2, 1e-3, 1e-4] {
                let tolerance = relative * k;
                let meshed = tessellate_body_with_threads(&body, &params(tolerance), 0);
                let line = match &meshed {
                    Ok(mesh) => {
                        for threads in [1, 2, 8] {
                            let again =
                                tessellate_body_with_threads(&body, &params(tolerance), threads)
                                    .unwrap();
                            assert_eq!(
                                hash(&again),
                                hash(mesh),
                                "{name} at {k}, {relative}: {threads} threads"
                            );
                        }
                        format!("{:>9} {:016x}", mesh.triangles.len(), hash(mesh))
                    }
                    Err(error) => format!("error: {error}"),
                };
                count += 1;
                println!("hash {name:42} {k:>6.0e} {relative:>6.0e} {line}");
            }
        }
    }
    println!("{count} meshes, each on 1, 2, 8 and all threads");
}

/// The largest distance from a lattice of points on the triangles of the
/// freeform faces of `body` to the surfaces of those faces.
fn deviation(body: &Body, mesh: &Mesh) -> f64 {
    const N: u32 = 4;
    let mut worst = 0.0_f64;
    for (t, triangle) in mesh.triangles.iter().enumerate() {
        let face = &body.faces[mesh.face_ids[t] as usize];
        if !matches!(face.surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
            continue;
        }
        let [a, b, c] = triangle.map(|v| mesh.positions[v as usize]);
        for i in 0..=N {
            for j in 0..=N - i {
                // (The corners are points of the surface.)
                if [i, j, N - i - j].contains(&N) {
                    continue;
                }
                let (s, r) = (f64::from(i) / f64::from(N), f64::from(j) / f64::from(N));
                let p = a * (1.0 - s - r) + b * s + c * r;
                let (u, v) = face.surface.project(p).unwrap();
                worst = worst.max(face.surface.eval(u, v).distance(p));
            }
        }
    }
    worst
}

#[test]
#[ignore = "prints deviations and timings; run in release with --ignored --nocapture"]
fn freeform_meshes_against_their_surfaces() {
    for k in [1e-3, 1.0, 1e3] {
        for (name, body) in bodies(k) {
            let freeform = body
                .faces
                .iter()
                .filter(|f| matches!(f.surface, Surface::Bezier(_) | Surface::Nurbs(_)))
                .count();
            if freeform == 0 {
                continue;
            }
            let length = size(&body, k);
            for relative in [1e-2, 1e-3, 1e-4] {
                let tolerance = relative * k;
                let mut best = f64::INFINITY;
                let mut meshed = None;
                for _ in 0..3 {
                    let start = Instant::now();
                    meshed = Some(tessellate_body_with_threads(&body, &params(tolerance), 1));
                    best = best.min(start.elapsed().as_secs_f64() * 1e3);
                }
                match meshed.unwrap() {
                    Ok(mesh) => {
                        let on_freeform = mesh
                            .face_ids
                            .iter()
                            .filter(|&&f| {
                                matches!(
                                    body.faces[f as usize].surface,
                                    Surface::Bezier(_) | Surface::Nurbs(_)
                                )
                            })
                            .count();
                        let off = deviation(&body, &mesh) / tolerance;
                        println!(
                            "freeform {name:42} {k:>6.0e} {relative:>6.0e} {:>8} {on_freeform:>8} \
                             {off:>6.3} {best:>9.2} ms{}",
                            mesh.triangles.len(),
                            if off > 1.0 { "  OVER" } else { "" }
                        );
                    }
                    Err(error) => {
                        println!("freeform {name:42} {k:>6.0e} {relative:>6.0e} error: {error}");
                    }
                }
            }
            let _ = length;
        }
    }
}
