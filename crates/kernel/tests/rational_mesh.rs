//! Faces on rational surfaces whose weights are far apart, meshed.
//!
//! A rational surface with uneven weights is drawn at an uneven pace: a
//! mesh laid evenly in its parameters is wasteful where it is slow and
//! too coarse where it is fast, though its *shape* may be mild. The
//! tessellator's even grid, sized by a bound on the second derivatives in
//! the parameters, asked such faces for thousands to billions of cells a
//! side and refused them; since 2026-11-15 a face whose even grid is
//! refused gets a grid graded to its shape (`tess/src/graded.rs`). The
//! measure is held to the closed forms wherever it gives one; a flat face
//! whose weights have no product form and are ten thousand and more apart
//! it still refuses, and those are counted.
//!
//! The oracle is apart from the tessellator and from the measure: every
//! body here is a shape with a closed form, drawn unevenly. The twins are
//! pieces of a cylinder, a sphere, a cone and a torus as rational patches
//! (`rational_common`), each span redrawn by a Möbius map so that its
//! weights are a thousand to a million apart: the same point set, so
//! every point of the mesh is held to the *analytic* surface in closed
//! form, and the volume to the closed form. The blocks have sides that
//! are planes drawn with such weights, with and without a product form.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_math::Portable;
mod boolean_common;
mod rational_common;

use boolean_common::*;
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::boolean::BooleanOp::{self, Intersect, Subtract, Union};
use craft3d_kernel::boolean::boolean;
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_tess::{Mesh, TessError, TessParams, tessellate_body_with_threads};
use craft3d_topo::{Body, MassError, check, mass_properties};
use rational_common::{rationalized, skewed_apart};
use std::f64::consts::PI;

/// The solids of `boolean_rational.rs` with curved faces clear of poles
/// and seams, each with its volume at scale 1 and a block that crosses
/// its curved face.
fn solids(k: f64) -> Vec<(&'static str, Body, f64, Body)> {
    let cut = |a: &Body, b: &Body| solve(a, b, Intersect, k).body;
    let section = 3.0_f64.sqrt() / 2.0 + PI / 3.0;
    let (low, high, height) = (1.75_f64, 1.0_f64, 1.5_f64);
    vec![
        (
            "quarter rod",
            cut(
                &rod(k, [0.0, 0.0, 0.0], 2.0, 3.0),
                &boxed(k, [0.0, 0.0, -1.0], [3.0, 3.0, 4.0]),
            ),
            PI * 4.0 * 3.0 / 4.0,
            boxed(k, [1.0, 0.7, 1.1], [2.6, 2.3, 2.2]),
        ),
        (
            "ball zone",
            cut(
                &ball(k, [0.0, 0.0, 0.0], 2.0),
                &boxed(k, [0.0, 0.0, -1.0], [3.0, 3.0, 1.0]),
            ),
            PI * (8.0 - 2.0 / 3.0) / 4.0,
            boxed(k, [0.9, 0.8, -0.4], [2.4, 2.1, 0.5]),
        ),
        (
            "frustum",
            cut(
                &spike(k, [0.0, 0.0, 0.0], 2.0, 4.0),
                &boxed(k, [0.0, 0.0, 0.5], [3.0, 3.0, 2.0]),
            ),
            PI * height * (low * low + low * high + high * high) / 3.0 / 4.0,
            boxed(k, [0.6, 0.5, 0.9], [2.0, 1.6, 1.6]),
        ),
        (
            "ring slice",
            cut(
                &ring(k, [0.0, 0.0, 0.0], 3.0, 1.0),
                &boxed(k, [0.0, 0.0, -0.5], [5.0, 5.0, 0.5]),
            ),
            PI / 2.0 * 3.0 * section,
            boxed(k, [2.3, 1.4, -0.3], [3.6, 2.4, 0.35]),
        ),
    ]
}

/// A tilt and a shift that leave nothing along an axis.
fn tilt(k: f64) -> Transform {
    Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
}

/// The plane over `[0, a] × [0, b]` of `frame` as a sheet of `degree`,
/// one span each way: its control points on an even lattice, its weights
/// `cu^(i/d) cv^(j/d) corner^(i j / d²)`. Whatever the weights, it is the
/// plane, and its borders are the rectangle's sides. `cu` and `cv` are a
/// Möbius map of each parameter (a product form, the end weights of a
/// span that far apart); `corner` and `ridge` are what no
/// reparametrisation removes: a heavy corner (for degree 1 the plate with
/// the weights 1, 1, 1, h) and, times `ridge^(|i − j| / d)`, two heavy
/// corners across each other (for degree 1 the plate 1, h, h, 1 of the
/// boolean checklist, whose end weights agree on average along either
/// parameter).
fn sheet(
    frame: &Frame,
    (a, b): (f64, f64),
    degree: usize,
    (cu, cv): (f64, f64),
    (corner, ridge): (f64, f64),
) -> NurbsSurface {
    let n = degree + 1;
    let d = degree as f64;
    let mut knots = vec![0.0; n];
    knots.extend(vec![1.0; n]);
    let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
    for i in 0..n {
        for j in 0..n {
            ctrl.push(frame.point(a * i as f64 / d, b * j as f64 / d, 0.0));
            weights.push(
                cu.ppowf(i as f64 / d)
                    * cv.ppowf(j as f64 / d)
                    * corner.ppowf((i * j) as f64 / (d * d))
                    * ridge.ppowf(i.abs_diff(j) as f64 / d),
            );
        }
    }
    NurbsSurface::new(degree, degree, knots.clone(), knots, n, n, ctrl, weights).unwrap()
}

/// The block `size` at the origin with its faces facing +x, +y and +z on
/// the sheets `make` gives for them (a frame at the face's corner with z
/// outward, and the face's two sides).
fn block_with(size: Vec3, make: &mut dyn FnMut(&Frame, (f64, f64)) -> NurbsSurface) -> Body {
    let mut body = primitives::block(&Frame::WORLD, size.x, size.y, size.z).unwrap();
    for index in 0..body.faces.len() {
        let Surface::Plane(plane) = &body.faces[index].surface else {
            continue;
        };
        let f = *plane.frame();
        if !(f.z().x > 0.99 || f.z().y > 0.99 || f.z().z > 0.99) {
            continue;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for vertex in &body.vertices {
            let (x, y, z) = f.local(vertex.point);
            if z.abs() <= 1e-9 * size.length() {
                lo = [lo[0].min(x), lo[1].min(y)];
                hi = [hi[0].max(x), hi[1].max(y)];
            }
        }
        let at = Frame::new(f.point(lo[0], lo[1], 0.0), f.z(), f.x()).unwrap();
        body.faces[index].surface = Surface::Nurbs(make(&at, (hi[0] - lo[0], hi[1] - lo[1])));
    }
    body
}

/// The block of `classify_far.rs`: its sides facing +x, +y and +z flat
/// bicubic sheets of three spans each way whose weights run from 1e-3 to
/// 1e3, seeded, with no form at all.
fn seeded_block(size: Vec3, seed: u64) -> Body {
    let mut state = seed;
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];
    block_with(size, &mut |f, (a, b)| {
        let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
        for i in 0..6 {
            for j in 0..6 {
                ctrl.push(f.point(a * i as f64 / 5.0, b * j as f64 / 5.0, 0.0));
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let next = (state >> 11) as f64 / (1u64 << 53) as f64;
                weights.push((6.0 * (next - 0.5) * std::f64::consts::LN_10).pexp());
            }
        }
        NurbsSurface::new(3, 3, knots.clone(), knots.clone(), 6, 6, ctrl, weights).unwrap()
    })
}

/// Whether two meshes are the same, bit for bit.
fn same(a: &Mesh, b: &Mesh) -> bool {
    let bits = |p: &Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    a.triangles == b.triangles
        && a.face_ids == b.face_ids
        && a.positions.len() == b.positions.len()
        && a.positions
            .iter()
            .zip(&b.positions)
            .all(|(p, q)| bits(p) == bits(q))
}

/// What a mesh is held to.
struct Held<'a> {
    /// How far a point is from the closed form's boundary, given the face
    /// of the mesh it is a point of.
    off: &'a dyn Fn(usize, Vec3) -> f64,
    volume: f64,
    /// How far the mesh's volume may be from it.
    volume_within: f64,
    tolerance: f64,
    /// The most triangles the mesh may have.
    most: usize,
}

/// The mesh of `body` at `params`, held to its closed form: the same on
/// one thread, on two and on all; closed in double precision and in
/// single; every corner, the middle of every side and of every triangle
/// within the tolerance of the closed form's surface; the volume; no more
/// triangles than the shape asks for. How many triangles it has, and how
/// many of them fold to a line in single precision (a file leaves those
/// out, and says so).
#[track_caller]
fn held(body: &Body, params: &TessParams, to: &Held, what: &str) -> (usize, usize) {
    let mesh = match tessellate_body_with_threads(body, params, 1) {
        Ok(mesh) => mesh,
        Err(e) => panic!("{what}: not meshed: {e}"),
    };
    for threads in [2, 0] {
        let again = tessellate_body_with_threads(body, params, threads).unwrap();
        assert!(same(&mesh, &again), "{what}: differs on {threads} threads");
    }
    let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert!(welded.is_closed(), "{what}: the mesh is not watertight");
    assert!(
        (welded.volume() - to.volume).abs() <= to.volume_within,
        "{what}: mesh volume {} for {} (within {:e})",
        welded.volume(),
        to.volume,
        to.volume_within
    );
    // In single precision, as a file has it.
    let single = |p: &Vec3| Vec3::new(p.x as f32 as f64, p.y as f32 as f64, p.z as f32 as f64);
    let rounded = Mesh {
        positions: mesh.positions.iter().map(single).collect(),
        ..mesh.clone()
    };
    assert!(
        rounded.weld().is_ok_and(|w| w.is_closed()),
        "{what}: not closed in single precision"
    );
    let (mut worst, mut folded) = (0.0_f64, 0);
    for (t, corners) in mesh.triangles.iter().enumerate() {
        let [a, b, c] = corners.map(|i| mesh.positions[i as usize]);
        let face = mesh.face_ids[t] as usize;
        let [ra, rb, rc] = [a, b, c].map(|p| single(&p));
        if (rb - ra).cross(rc - ra).length() <= 0.0 {
            folded += 1;
        }
        for point in [
            a,
            b,
            c,
            a.lerp(b, 0.5),
            b.lerp(c, 0.5),
            c.lerp(a, 0.5),
            (a + b + c) * (1.0 / 3.0),
        ] {
            worst = worst.max((to.off)(face, point));
        }
    }
    assert!(
        worst <= to.tolerance * (1.0 + 1e-9),
        "{what}: a point of the mesh {worst:e} from the closed form, tolerance {:e}",
        to.tolerance
    );
    assert!(
        mesh.triangles.len() <= to.most,
        "{what}: {} triangles, where the shape asks for {} at most",
        mesh.triangles.len(),
        to.most
    );
    (mesh.triangles.len(), folded)
}

/// Whether the measure gives `body`'s volume and area, which are then
/// held to the closed form's within 1e-9 (and its centroid, where one is
/// given, within 1e-9 of `size`). Not where it refuses the body for
/// weights its own parameters do not measure (further apart within a
/// span than it takes, or sums that do not settle): any other refusal is
/// a failure.
#[track_caller]
fn measured(
    body: &Body,
    (volume, area): (f64, f64),
    centre: Option<(Vec3, f64)>,
    what: &str,
) -> bool {
    match mass_properties(body) {
        Ok(found) => {
            assert!(
                (found.volume - volume).abs() <= 1e-9 * volume
                    && (found.area - area).abs() <= 1e-9 * area,
                "{what}: measured {found:?} for {volume} and {area}"
            );
            if let Some((centre, size)) = centre {
                assert!(
                    found.centroid.unwrap().distance(centre) <= 1e-9 * size,
                    "{what}: centroid"
                );
            }
            true
        }
        Err(MassError::UnevenWeights { .. }) => false,
        Err(MassError::Unsupported { what: why, .. }) if why.contains("does not settle") => false,
        Err(e) => panic!("{what}: {e}"),
    }
}

/// How far `point` is from an analytic surface, in closed form.
fn off_surface(surface: &Surface, point: Vec3) -> f64 {
    let (u, v) = surface.project(point).unwrap();
    surface.eval(u, v).distance(point)
}

/// The twins with each span's weights a thousand to a million apart
/// along u, along v and both, at three scales, straight and tilted: every
/// one was refused before 2026-11-15 (its even grid asked for 1 660 to
/// 544 million cells a side, against 1 024). Each now meshes closed,
/// every point of its mesh within the tolerance of the *analytic*
/// surface its rational face is, its volume within the tolerance times
/// its area of the closed form, the same on any number of threads, with
/// no more triangles than its evenly weighted twin (measured: 0.90 to
/// 0.96 of them; held to 1.25 times), and measures as the closed form.
#[test]
fn twins_redrawn_at_an_uneven_pace_mesh_as_their_even_twins() {
    let mut meshed = 0;
    for k in SCALES {
        for (name, plain, volume, _) in solids(k) {
            let (twin, _) = rationalized(&plain, 0.2, true);
            for tilted in [false, true] {
                let moved = |body: &Body| {
                    if tilted {
                        body.transformed(&tilt(k)).unwrap()
                    } else {
                        body.clone()
                    }
                };
                let analytic = moved(&plain);
                let exact = mass(&analytic);
                let params = TessParams {
                    chordal_tolerance: 3e-3 * k,
                    ..TessParams::default()
                };
                let off =
                    |face: usize, point: Vec3| off_surface(&analytic.faces[face].surface, point);
                let mut to = Held {
                    off: &off,
                    volume: exact.volume,
                    volume_within: exact.area * params.chordal_tolerance,
                    tolerance: params.chordal_tolerance,
                    most: usize::MAX,
                };
                assert!((exact.volume - volume * k * k * k).abs() <= 1e-9 * exact.volume);
                let (even, folded) =
                    held(&moved(&twin), &params, &to, &format!("{name} at {k}, even"));
                assert_eq!(folded, 0, "{name} at {k}, even");
                to.most = even + even / 4;
                for (along_u, along_v) in [
                    (1e3, 1e3),
                    (1e4, 1.0),
                    (1e4, 1e4),
                    (1e6, 1.0),
                    (1.0, 1e6),
                    (1e6, 1e6),
                ] {
                    let what =
                        format!("{name} at {k}, tilted {tilted}, drawn {along_u} by {along_v}");
                    let body = moved(&skewed_apart(&twin, along_u, along_v));
                    assert_eq!(check(&body, &tolerance(k)), [], "{what}");
                    let (_, folded) = held(&body, &params, &to, &what);
                    assert_eq!(folded, 0, "{what}: folds in single precision");
                    // (A reparametrisation evens these weights out: measured.)
                    assert!(
                        measured(&body, (exact.volume, exact.area), None, &what),
                        "{what}: not measured"
                    );
                    meshed += 1;
                }
            }
        }
    }
    assert_eq!(meshed, 144);
}

/// Blocks whose three outer sides are planes drawn with weights a
/// thousand to a million apart: by a Möbius map of one parameter or of
/// both, and with one heavy corner or two across each other, which no
/// reparametrisation removes (the second the plate of the checklist);
/// degrees 1 to 3; a plate 4 by 3 by 2 at three scales and a block 30 by
/// 900 by 20, whose sheets run unevenly the short way and the long way;
/// straight and tilted. A plane is a plane however it is drawn: where the
/// even grid is refused (it asked for up to nine million million cells a
/// side) the mesh is that of the plain block but for a few points more
/// (12 to 30 triangles; held to 64), every point of it on the block's own
/// planes, its volume the block's to rounding. Where the weights are near
/// enough for the even grid to fit its limit, the face still has that
/// grid, with hundreds of thousands of triangles (owed: it is every
/// evenly gridded mesh that would change); those are counted here, under
/// a budget, not made. And each block measures as the block wherever the
/// measure gives one: every one whose weights a reparametrisation evens
/// out; those with heavy corners ten thousand and more apart it refuses
/// (counted).
#[test]
fn blocks_with_planes_drawn_at_an_uneven_pace_mesh_as_blocks() {
    let (mut graded, mut even, mut unmeasured) = (0, 0, 0);
    let plate = Vec3::new(4.0, 3.0, 2.0);
    {
        for (k, size) in [
            (1e-3, plate),
            (1.0, plate),
            (1e6, plate),
            (1.0, Vec3::new(30.0, 900.0, 20.0)),
        ] {
            let size = size * k;
            let plain = primitives::block(&Frame::WORLD, size.x, size.y, size.z).unwrap();
            let volume = size.x * size.y * size.z;
            let area = 2.0 * (size.x * size.y + size.y * size.z + size.z * size.x);
            let reach = size.x.max(size.y).max(size.z);
            for degree in 1..=3 {
                for ratio in [1e3, 1e4, 1e6] {
                    for (cu, cv, corner, ridge, how) in [
                        (ratio, 1.0, 1.0, 1.0, "along u"),
                        (1.0, ratio, 1.0, 1.0, "along v"),
                        (ratio, 1.0 / ratio, 1.0, 1.0, "both"),
                        (1.0, 1.0, ratio, 1.0, "at a corner"),
                        (1.0, 1.0, 1.0, ratio, "at two corners across"),
                    ] {
                        for tilted in [false, true] {
                            let what = format!(
                                "block {size:?} degree {degree}, weights {ratio} apart {how}, tilted {tilted}"
                            );
                            let mut body = block_with(size, &mut |f, sides| {
                                sheet(f, sides, degree, (cu, cv), (corner, ridge))
                            });
                            let mut analytic = plain.clone();
                            if tilted {
                                body = body.transformed(&tilt(k)).unwrap();
                                analytic = analytic.transformed(&tilt(k)).unwrap();
                            }
                            let tol = Tolerance::new(1e-6 * reach, 1e-9).unwrap();
                            let started = std::time::Instant::now();
                            assert_eq!(check(&body, &tol), [], "{what}");
                            let checked = started.elapsed();
                            let found = measured(&body, (volume, area), None, &what);
                            assert!(found || corner > 1.0 || ridge > 1.0, "{what}: not measured");
                            unmeasured += usize::from(!found);
                            if std::env::var_os("RATIONAL_TIMINGS").is_some() {
                                eprintln!(
                                    "{what}: checked in {checked:.2?}, measured in {:.2?}",
                                    started.elapsed() - checked
                                );
                            }
                            let params = TessParams {
                                chordal_tolerance: 1e-3 * reach,
                                max_triangles: Some(4_000),
                                ..TessParams::default()
                            };
                            match tessellate_body_with_threads(&body, &params, 1) {
                                // Its even grid fits the limit: as before.
                                Err(TessError::OverBudget { .. }) => {
                                    even += 1;
                                    continue;
                                }
                                Err(e) => panic!("{what}: not meshed: {e}"),
                                Ok(_) => {}
                            }
                            let off = |face: usize, point: Vec3| {
                                off_surface(&analytic.faces[face].surface, point)
                            };
                            let to = Held {
                                off: &off,
                                volume,
                                volume_within: 1e-9 * volume,
                                // On the planes to rounding: nothing of
                                // the tolerance is used.
                                tolerance: 1e-9 * reach,
                                most: 64,
                            };
                            let (_, folded) = held(&body, &params, &to, &what);
                            assert_eq!(folded, 0, "{what}: folds in single precision");
                            graded += 1;
                        }
                    }
                }
            }
        }
    }
    println!(
        "blocks: {graded} with a mesh of the plain block's order, {even} with the even grid; {unmeasured} not measured"
    );
    assert_eq!(graded + even, 360);
    assert!(graded * 3 >= 360 * 2, "{graded} of 360");
}

/// The block 30 by 900 by 20 whose sides are flat bicubic sheets with
/// seeded weights from 1e-3 to 1e3 (`classify_far.rs`; no product form,
/// up to two million apart within a span after evening), and the same 30
/// by 30 by 20, five seeds each. Until 2026-11-15 no such block could be
/// meshed (0.07 to 4.4 million cells a side). Each now meshes as the
/// block it is (36 to 40 triangles; held to 64). None is measured yet:
/// its weights are further apart than the measure's own parameters take
/// (counted, and held to the block wherever one is).
#[test]
fn blocks_with_seeded_weights_mesh_as_blocks() {
    let mut unmeasured = 0;
    for seed in 1..=5 {
        for size in [Vec3::new(30.0, 30.0, 20.0), Vec3::new(30.0, 900.0, 20.0)] {
            for tilted in [false, true] {
                let what = format!("seed {seed}, {size:?}, tilted {tilted}");
                let mut body = seeded_block(size, seed);
                let mut analytic =
                    primitives::block(&Frame::WORLD, size.x, size.y, size.z).unwrap();
                if tilted {
                    body = body.transformed(&tilt(1.0)).unwrap();
                    analytic = analytic.transformed(&tilt(1.0)).unwrap();
                }
                let volume = size.x * size.y * size.z;
                let area = 2.0 * (size.x * size.y + size.y * size.z + size.z * size.x);
                let centre = if tilted {
                    tilt(1.0).point(size * 0.5)
                } else {
                    size * 0.5
                };
                unmeasured += usize::from(!measured(
                    &body,
                    (volume, area),
                    Some((centre, size.y)),
                    &what,
                ));
                let off =
                    |face: usize, point: Vec3| off_surface(&analytic.faces[face].surface, point);
                for chord in [1e-2, 1e-3, 1e-5] {
                    let params = TessParams {
                        chordal_tolerance: chord * size.y,
                        ..TessParams::default()
                    };
                    let to = Held {
                        off: &off,
                        volume,
                        volume_within: 1e-9 * volume,
                        tolerance: 1e-9 * size.y,
                        most: 64,
                    };
                    let (_, folded) = held(&body, &params, &to, &format!("{what}, chord {chord}"));
                    assert_eq!(folded, 0, "{what}: folds in single precision");
                }
            }
        }
    }
    println!("seeded blocks: {unmeasured} of 20 not measured");
}

/// Signed distance to the box from `lo` to `hi` (negative inside).
fn box_distance(p: Vec3, lo: Vec3, hi: Vec3) -> f64 {
    let (c, h) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
    let q = Vec3::new(
        (p.x - c.x).abs() - h.x,
        (p.y - c.y).abs() - h.y,
        (p.z - c.z).abs() - h.z,
    );
    let outside = Vec3::new(q.x.max(0.0), q.y.max(0.0), q.z.max(0.0)).length();
    outside + q.x.max(q.y).max(q.z).min(0.0)
}

/// The booleans of the long block with seeded weights and a block
/// through three of its weighted sides (`classify_far.rs` holds that
/// they are built) can be shown: each meshes closed, every point of its
/// mesh on the boundary of the closed form (the signed distance of the
/// two boxes' union, intersection or difference, apart from the kernel),
/// its volume the closed form's to rounding, the same on any number of
/// threads. (Not measured yet, as the block itself is not.)
#[test]
fn the_booleans_of_a_block_with_seeded_weights_mesh() {
    let tol = tolerance(1.0);
    let long = seeded_block(Vec3::new(30.0, 900.0, 20.0), 1);
    let (lo, hi) = (Vec3::new(12.0, 315.0, 9.0), Vec3::new(39.0, 1080.0, 25.0));
    let tool = primitives::block(&Frame::WORLD.at(lo).unwrap(), 27.0, 765.0, 16.0).unwrap();
    let whole = Vec3::new(30.0, 900.0, 20.0);
    let shared = 18.0 * 585.0 * 11.0;
    for (op, volume) in [
        (Intersect, shared),
        (Union, 30.0 * 900.0 * 20.0 + 27.0 * 765.0 * 16.0 - shared),
        (Subtract, 30.0 * 900.0 * 20.0 - shared),
    ] {
        let made = boolean(&long, &tool, op, &tol)
            .unwrap_or_else(|e| panic!("{op:?}: {e}"))
            .body;
        let distance = |p: Vec3| {
            let (a, b) = (box_distance(p, Vec3::ZERO, whole), box_distance(p, lo, hi));
            match op {
                BooleanOp::Union => a.min(b),
                BooleanOp::Intersect => a.max(b),
                BooleanOp::Subtract => a.max(-b),
            }
        };
        let off = |_: usize, point: Vec3| distance(point).abs();
        let to = Held {
            off: &off,
            volume,
            volume_within: 1e-9 * volume,
            tolerance: tol.linear,
            most: 600,
        };
        let params = TessParams {
            chordal_tolerance: 0.9,
            ..TessParams::default()
        };
        // (Counted, not held: straight cuts across such a face are cut
        // where their chords cross in its parameters, and a triangle
        // between two of those points may fold in single precision.)
        let (triangles, folded) = held(&made, &params, &to, &format!("{op:?}"));
        println!("{op:?}: {triangles} triangles, {folded} that fold in single precision");
    }
}

/// The booleans of the twins drawn a thousandfold unevenly both ways (1.4
/// million within a span) with a block through their curved face, built
/// since 2026-10-17 and not meshed then: each meshes closed, every point
/// of its mesh within the tolerance of a surface of the analytic solid or
/// of the block (their closed forms), its volume within the tolerance
/// times its area of the same boolean of the analytic solid.
///
/// Two things are counted and not held, both older than the graded grid
/// and neither the tessellator's sizing. The curves in which the boolean
/// cuts such a face have knots a billionth of the size apart where its
/// march crossed a knot of the surface, the tessellator takes every knot
/// of a spline edge for a point of it, and the triangles between such
/// points fold to lines in single precision (the mesh is closed in
/// either; a file leaves them out and says so). And a cut face whose box
/// is small enough for its even grid to fit the limit still has that
/// grid, with tens of thousands of triangles.
#[test]
fn the_booleans_of_twins_drawn_unevenly_mesh_on_their_closed_forms() {
    let k = 1.0;
    for (name, plain, _, tool) in solids(k) {
        let (twin, _) = rationalized(&plain, 0.2, true);
        let body = skewed_apart(&twin, 1e3, 1e3);
        for op in [Union, Intersect, Subtract] {
            let what = format!("{name}, {op:?}");
            let made = boolean(&body, &tool, op, &tolerance(k))
                .unwrap_or_else(|e| panic!("{what}: {e}"))
                .body;
            let exact = mass(&solve(&plain, &tool, op, k).body);
            let params = TessParams {
                chordal_tolerance: 3e-3 * k,
                ..TessParams::default()
            };
            let off = |_: usize, point: Vec3| {
                plain
                    .faces
                    .iter()
                    .chain(&tool.faces)
                    .map(|face| off_surface(&face.surface, point))
                    .fold(f64::INFINITY, f64::min)
            };
            let to = Held {
                off: &off,
                volume: exact.volume,
                volume_within: exact.area * params.chordal_tolerance,
                tolerance: params.chordal_tolerance,
                most: usize::MAX,
            };
            let (triangles, folded) = held(&made, &params, &to, &what);
            println!("{what}: {triangles} triangles, {folded} that fold in single precision");
        }
    }
}
