//! The offset of a rolling-ball fillet is the fillet rebuilt between the
//! offset faces it rolls along, with its radius moved by the distance
//! (`docs/decisions.md`, 2026-10-16), against closed forms.
//!
//! The fillets here are made by the rolling ball (`blend::rolling`), so
//! their faces are fitted splines, though what they are exactly has a
//! closed form: the faces they roll along are freeform twins of a
//! cylinder (a rational cubic, as in `offset_fillet.rs`) or of a plane (a
//! bilinear patch), so the kernel sees splines everywhere it looks, and
//! the volume of the result is still Pappus's rule or a box less a
//! corner. Each case moves the fillet together with the faces it rolls
//! along: the fillet's offset is then a piece of a torus (or a cylinder)
//! whose tube radius is the fillet's moved by the distance, tangent to
//! the moved faces; at 1e-3, 1 and 1e6.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;
mod rational_common;

use boolean_common::{SCALES, boxed, mass, rod, solve, tolerance};
use craft3d_geom::refine::{elevated, with_knots};
use craft3d_geom::{NurbsCurve, NurbsSurface, Surface, allowance};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract};
use craft3d_kernel::fillet::{FilletEdge, FilletFace, fillet};
use craft3d_kernel::offset_face::{Blends, FaceOffset, offset_faces, offset_faces_with};
use craft3d_kernel::shell::{Skin, shell_with};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use rational_common::rationalized;
use std::f64::consts::PI;
use std::time::Instant;

/// The quarter of a rod of radius 2 and height 1 standing on the origin
/// (x and y at least 0): its rim at the top is convex.
fn wedge(k: f64) -> Body {
    let rod = rod(k, [0.0, 0.0, 0.0], 2.0, 1.0);
    let quadrant = boxed(k, [0.0, 0.0, -1.0], [3.0, 3.0, 2.0]);
    solve(&rod, &quadrant, Intersect, k).body
}

/// A block 3 × 3 × 1.5 with the quarter of a pocket of radius 2 and depth
/// 1 at its corner on the z axis: the foot of the pocket's wall on its
/// floor (z = 0.5) is concave.
fn pocket(k: f64) -> Body {
    let block = boxed(k, [0.0, 0.0, 0.0], [3.0, 3.0, 1.5]);
    let rod = rod(k, [0.0, 0.0, 0.5], 2.0, 2.0);
    solve(&block, &rod, Subtract, k).body
}

/// The same rational surface with its first parameter raised to degree
/// three and the knots 1 and 2 inserted: the same points.
fn cubic(surface: &NurbsSurface) -> NurbsSurface {
    let (count_u, count_v) = surface.counts();
    assert_eq!(surface.degrees().0, 2);
    let lines: Vec<NurbsCurve> = (0..count_v)
        .map(|j| {
            let ctrl = (0..count_u)
                .map(|i| surface.control_points()[i * count_v + j])
                .collect();
            let weights = (0..count_u)
                .map(|i| surface.weights()[i * count_v + j])
                .collect();
            let line = NurbsCurve::new(2, surface.knots_u().to_vec(), ctrl, weights).unwrap();
            with_knots(&elevated(&line, 3).unwrap(), &[1.0, 2.0]).unwrap()
        })
        .collect();
    let new_u = lines[0].control_points().len();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..new_u {
        for line in &lines {
            ctrl.push(line.control_points()[i]);
            weights.push(line.weights()[i]);
        }
    }
    NurbsSurface::new(
        3,
        surface.degrees().1,
        lines[0].knots().to_vec(),
        surface.knots_v().to_vec(),
        new_u,
        count_v,
        ctrl,
        weights,
    )
    .unwrap()
}

/// `body` with its cylinder face on the rational cubic that is that piece
/// of the cylinder, and the face.
fn twin(body: &Body) -> (Body, FaceId) {
    let (mut twin, replaced) = rationalized(body, 0.0, false);
    assert_eq!(replaced, 1);
    let index = twin
        .faces
        .iter()
        .position(|f| matches!(f.surface, Surface::Nurbs(_)))
        .unwrap();
    let Surface::Nurbs(quadratic) = &twin.faces[index].surface else {
        panic!("not the rational patch")
    };
    twin.faces[index].surface = Surface::Nurbs(cubic(quadratic));
    assert_eq!(check(&twin, &tolerance(1.0)), []);
    (twin, FaceId(index as u32))
}

/// The faces of `body` that use `edge`.
fn users(body: &Body, edge: EdgeId) -> Vec<FaceId> {
    (0..body.faces.len())
        .filter(|f| {
            body.faces[*f]
                .loops
                .iter()
                .flat_map(|l| &body.loop_(*l).unwrap().coedges)
                .any(|c| c.edge == edge)
        })
        .map(|f| FaceId(f as u32))
        .collect()
}

/// The edge between `face` and the flat face at the height `z`, and that
/// flat face.
fn rim(body: &Body, face: FaceId, z: f64) -> (EdgeId, FaceId) {
    let found: Vec<(EdgeId, FaceId)> = (0..body.edges.len())
        .filter_map(|e| {
            let edge_id = EdgeId(e as u32);
            let faces = users(body, edge_id);
            if !faces.contains(&face) {
                return None;
            }
            let other = *faces.iter().find(|f| **f != face)?;
            let flat = matches!(body.face(other).unwrap().surface, Surface::Plane(_));
            let edge = &body.edges[e];
            let p = edge.curve.eval(0.5 * (edge.range.0 + edge.range.1));
            (flat && (p.z - z).abs() <= 1e-6 * (1.0 + z.abs())).then_some((edge_id, other))
        })
        .collect();
    assert_eq!(found.len(), 1, "one edge at the height {z}: {found:?}");
    found[0]
}

/// A body whose rim (between the freeform `wall` and the flat face at
/// height `z`) is filleted with radius `rho` by the rolling ball: the
/// body, the fillet face, and the two faces it rolls along.
struct Filleted {
    body: Body,
    fillet: FaceId,
    supports: [FaceId; 2],
}

fn filleted(body: &Body, wall: FaceId, z: f64, rho: f64, tol: &Tolerance) -> Filleted {
    let (edge, flat) = rim(body, wall, z);
    let made = fillet(body, &[FilletEdge { edge, radius: rho }], tol).unwrap();
    assert_eq!(check(&made.body, tol), []);
    let find = |want: &dyn Fn(&FilletFace) -> bool| {
        let found: Vec<usize> = (0..made.faces.len())
            .filter(|i| want(&made.faces[*i]))
            .collect();
        assert_eq!(found.len(), 1, "{:?}", made.faces);
        FaceId(found[0] as u32)
    };
    let fillet_face = find(&|f| matches!(f, FilletFace::Fillet(_)));
    let wall_now = find(&|f| matches!(f, FilletFace::Kept(k) if k.contains(&wall)));
    let flat_now = find(&|f| matches!(f, FilletFace::Kept(k) if k.contains(&flat)));
    assert!(
        matches!(
            made.body.face(fillet_face).unwrap().surface,
            Surface::Nurbs(_)
        ),
        "the fillet is made by the rolling ball"
    );
    Filleted {
        body: made.body,
        fillet: fillet_face,
        supports: [wall_now, flat_now],
    }
}

/// `faces` of `body` moved by `d`, told the fillet's radius.
fn moved(body: &Body, faces: &[FaceId], d: f64, blends: &Blends, tol: &Tolerance) -> Body {
    let offsets: Vec<FaceOffset> = faces
        .iter()
        .map(|face| FaceOffset {
            face: *face,
            distance: d,
        })
        .collect();
    let started = Instant::now();
    let made = offset_faces_with(body, &offsets, blends, tol)
        .unwrap_or_else(|e| panic!("offset {d}: {e}"));
    println!("offset {d}: {:?}", started.elapsed());
    assert_eq!(check(&made.body, tol), [], "offset {d}");
    made.body
}

/// Where the centroid of the corner a ball of radius ρ rounds off lies,
/// from the corner, along either face: ρ (10 − 3π) / (12 − 3π).
fn centroid_in(rho: f64) -> f64 {
    rho * (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI)
}

/// The corner a ball of radius ρ rounds off: ρ² (1 − π / 4).
fn corner(rho: f64) -> f64 {
    rho * rho * (1.0 - PI / 4.0)
}

/// The distances tried at scale `k`: out at the smallest scale, in at the
/// largest, both at 1.
fn moves(k: f64, [out, inward]: [f64; 2]) -> Vec<f64> {
    if k < 1.0 {
        vec![out]
    } else if k > 1.0 {
        vec![inward]
    } else {
        vec![out, inward]
    }
}

/// The fillet's face is the torus (or the cylinder) it is: every point of
/// it at the tube's radius from the circle (or line) of centres.
fn on_tube(body: &Body, face: FaceId, centre: &dyn Fn(Vec3) -> Vec3, radius: f64, tol: f64) {
    let surface = &body.face(face).unwrap().surface;
    let ((u0, u1), (v0, v1)) = surface.domain();
    let mut worst = 0.0_f64;
    for i in 0..=32 {
        for j in 0..=8 {
            let p = surface.eval(
                u0 + (u1 - u0) * f64::from(i) / 32.0,
                v0 + (v1 - v0) * f64::from(j) / 8.0,
            );
            worst = worst.max((p.distance(centre(p)) - radius).abs());
        }
    }
    assert!(worst <= 0.25 * tol, "off the tube by {worst:e}");
}

/// The rim of a quarter rod filleted by the rolling ball, moved out and in
/// with the top and the wall it rolls along: a quarter of a rod of the
/// radius `2 + d` and the height `1 + d` whose rim is the torus of the
/// tube radius `ρ + d` (Pappus's rule over the corner the ball cuts off).
#[test]
fn a_convex_fillet_moved_with_its_faces_is_the_torus_of_the_radius_plus_the_distance() {
    let rho = 0.25;
    for k in SCALES {
        let tol = tolerance(k);
        let (body, wall) = twin(&wedge(k));
        let made = filleted(&body, wall, k, rho * k, &tol);
        let blends = Blends {
            radii: [(made.fillet, rho * k)].into_iter().collect(),
        };
        for d in moves(k, [0.3, -0.2]) {
            let faces = [made.fillet, made.supports[0], made.supports[1]];
            let result = moved(&made.body, &faces, d * k, &blends, &tol);
            let m = mass(&result);
            let (r, h, tube) = (2.0 + d, 1.0 + d, rho + d);
            let expected = PI / 4.0 * r * r * h - PI / 2.0 * (r - centroid_in(tube)) * corner(tube);
            let slack = 0.5 * tol.linear * m.area;
            let off = m.volume - expected * k.powi(3);
            println!(
                "convex, k {k}, d {d}: volume {}, closed form {}, off {off:e}, slack {slack:e}",
                m.volume,
                expected * k.powi(3)
            );
            assert!(off.abs() <= slack, "k {k}, d {d}: off by {off:e}");
            let ring = (2.0 - rho) * k;
            let top = (1.0 - rho) * k;
            on_tube(
                &result,
                made.fillet,
                &|p| {
                    let radial = Vec3::new(p.x, p.y, 0.0).normalized().unwrap();
                    Vec3::new(radial.x * ring, radial.y * ring, top)
                },
                tube * k,
                tol.linear,
            );
        }
    }
}

/// The foot of a quarter pocket's wall filleted by the rolling ball,
/// moved with the floor and the wall: the pocket's radius `2 − d`, its
/// floor `d` higher, the ring the ball fills of the tube radius `ρ − d`.
#[test]
fn a_concave_fillet_moved_with_its_faces_is_the_torus_of_the_radius_less_the_distance() {
    let rho = 0.25;
    for k in SCALES {
        let tol = tolerance(k);
        let (body, wall) = twin(&pocket(k));
        let made = filleted(&body, wall, 0.5 * k, rho * k, &tol);
        let blends = Blends::default();
        for d in moves(k, [0.2, -0.3]) {
            let faces = [made.fillet, made.supports[0], made.supports[1]];
            let result = moved(&made.body, &faces, d * k, &blends, &tol);
            let m = mass(&result);
            let (r, depth, tube) = (2.0 - d, 1.0 - d, rho - d);
            let expected =
                13.5 - PI / 4.0 * r * r * depth + PI / 2.0 * (r - centroid_in(tube)) * corner(tube);
            let slack = 0.5 * tol.linear * m.area;
            let off = m.volume - expected * k.powi(3);
            println!(
                "concave, k {k}, d {d}: volume {}, closed form {}, off {off:e}, slack {slack:e}",
                m.volume,
                expected * k.powi(3)
            );
            assert!(off.abs() <= slack, "k {k}, d {d}: off by {off:e}");
        }
    }
}

/// Two balls of radius `big` whose centres lie `2 a` apart on the x axis,
/// joined (`union`) or as their lens (intersection), and the circle where
/// they meet filleted with radius `rho` by the rolling ball (the balls'
/// frames do not have the circle's axis, so no closed form is chosen):
/// the body and its fillet face.
fn balls(k: f64, union: bool, big: f64, a: f64, rho: f64, tol: &Tolerance) -> (Body, FaceId) {
    use craft3d_kernel::boolean::BooleanOp::Union;
    let first = boolean_common::ball(k, [-a, 0.0, 0.0], big);
    let second = boolean_common::ball(k, [a, 0.0, 0.0], big);
    let joined = solve(&first, &second, if union { Union } else { Intersect }, k).body;
    let edge = (0..joined.edges.len())
        .map(|e| EdgeId(e as u32))
        .find(|e| users(&joined, *e).len() == 2)
        .unwrap();
    let made = fillet(
        &joined,
        &[FilletEdge {
            edge,
            radius: rho * k,
        }],
        tol,
    )
    .unwrap_or_else(|e| panic!("fillet of the balls at {k}: {e}"));
    assert_eq!(check(&made.body, tol), []);
    let face = (0..made.faces.len())
        .find(|i| matches!(made.faces[*i], FilletFace::Fillet(_)))
        .map(|i| FaceId(i as u32))
        .unwrap();
    assert!(
        matches!(made.body.face(face).unwrap().surface, Surface::Nurbs(_)),
        "the ring is made by the rolling ball"
    );
    (made.body, face)
}

/// `∫ sqrt(r² − z²) dz` from `z0` to `z1` (`|z| ≤ r`).
fn half_disc(r: f64, z0: f64, z1: f64) -> f64 {
    let f = |z: f64| 0.5 * (z * (r * r - z * z).max(0.0).sqrt() + r * r * (z / r).asin());
    f(z1) - f(z0)
}

/// The volume of the solid of revolution about the x axis of two balls of
/// radius `big` about `(∓a, 0, 0)`, joined or as their lens, with the
/// circle where they meet rounded by a ring of tube radius `rho`
/// (concave, filling, for the union; convex, cutting, for the lens):
/// `π ∫ ρ(x)² dx` in closed form, piece by piece.
fn balls_volume(union: bool, big: f64, a: f64, rho: f64) -> f64 {
    // The ring's centre circle: `big ± rho` from both centres.
    let reach = if union { big + rho } else { big - rho };
    let centre = (reach * reach - a * a).sqrt();
    // Where the ring touches the balls, along x.
    let touch = a * rho / reach;
    // π ∫ (big² − (x ∓ a)²) dx over a cap of a ball: the ball's part
    // beyond `from` (distance from its centre along x) to its pole.
    let cap = |from: f64| PI * (big * big * (big - from) - (big.powi(3) - from.powi(3)) / 3.0);
    // π ∫ (centre ∓ sqrt(rho² − x²))² dx over |x| ≤ touch.
    let ring = |sign: f64| {
        let length = 2.0 * touch;
        PI * (centre * centre * length
            + 2.0 * sign * centre * half_disc(rho, -touch, touch)
            + rho * rho * length
            - 2.0 * touch.powi(3) / 3.0)
    };
    if union {
        // The far ball beyond x = touch is the near ball's... each ball
        // from the touching plane out: from `touch + a` from its centre?
        // For x > touch the ball about +a bounds it, from x − a = touch − a.
        2.0 * cap(touch - a) + ring(-1.0)
    } else {
        // For x > touch the ball about −a bounds the lens, from x + a.
        2.0 * cap(touch + a) + ring(1.0)
    }
}

/// The ring where two balls meet, filleted by the rolling ball, moved out
/// and in with both balls: the balls' radii and the ring's tube radius
/// moved by the distance (the ring's grows into the material at the
/// concave ring of the union, shrinks at the convex rim of the lens),
/// the volume in closed form (Pappus's rule, piece by piece).
#[test]
fn a_ring_between_two_balls_moved_with_them_is_the_ring_of_the_radius_moved() {
    let (big, a, rho) = (2.0, 1.0, 0.5);
    for k in SCALES {
        let tol = tolerance(k);
        for union in [true, false] {
            let (body, fillet_face) = balls(k, union, big, a, rho, &tol);
            let m = mass(&body);
            let made = balls_volume(union, big, a, rho);
            println!(
                "balls {union}, k {k}: volume {} against {}",
                m.volume,
                made * k.powi(3)
            );
            assert!((m.volume - made * k.powi(3)).abs() <= 0.5 * tol.linear * m.area);
            let all: Vec<FaceId> = (0..body.faces.len()).map(|f| FaceId(f as u32)).collect();
            for d in moves(k, [0.2, -0.15]) {
                let result = moved(&body, &all, d * k, &Blends::default(), &tol);
                let m = mass(&result);
                let tube = if union { rho - d } else { rho + d };
                let expected = balls_volume(union, big + d, a, tube);
                let slack = 0.5 * tol.linear * m.area;
                let off = m.volume - expected * k.powi(3);
                println!(
                    "balls {union}, k {k}, d {d}: volume {}, closed form {}, off {off:e}, slack {slack:e}",
                    m.volume,
                    expected * k.powi(3)
                );
                assert!(
                    off.abs() <= slack,
                    "union {union}, k {k}, d {d}: off by {off:e}"
                );
                // Rebuilt in the shape a rolling-ball fillet has.
                let Surface::Nurbs(surface) = &result.face(fillet_face).unwrap().surface else {
                    panic!("not a spline");
                };
                assert_eq!(surface.degrees().1, 2);
            }
        }
    }
}

/// The lens of two balls with its rim filleted, made hollow: the hollow is
/// the lens of the balls `t` smaller with the rim's fillet `t` smaller
/// (the fillet's face of the hollow rebuilt between the balls' faces of
/// the hollow), a cavity; the union of two balls with its concave ring
/// filleted, made hollow likewise, its ring `t` larger.
#[test]
fn balls_with_their_ring_filleted_shell_to_the_closed_form() {
    let (big, a, rho, t) = (2.0, 1.0, 0.5, 0.2);
    for k in SCALES {
        let tol = tolerance(k);
        for union in [true, false] {
            let (body, _) = balls(k, union, big, a, rho, &tol);
            let started = Instant::now();
            let made = shell_with(&body, t * k, &[], &Blends::default(), &tol)
                .unwrap_or_else(|e| panic!("shell, union {union}, k {k}: {e}"));
            println!("shell {union} at {k}: {:?}", started.elapsed());
            assert_eq!(check(&made.body, &tol), []);
            assert_eq!(made.body.shells.len(), 2);
            let m = mass(&made.body);
            let hollow_tube = if union { rho + t } else { rho - t };
            let expected =
                balls_volume(union, big, a, rho) - balls_volume(union, big - t, a, hollow_tube);
            let slack = 0.5 * tol.linear * m.area;
            let off = m.volume - expected * k.powi(3);
            println!(
                "shell {union}, k {k}: volume {}, closed form {}, off {off:e}",
                m.volume,
                expected * k.powi(3)
            );
            assert!(off.abs() <= slack, "union {union}, k {k}: off by {off:e}");
        }
    }
}

/// The work (`geom::allowance` units, not the clock) of offsetting the
/// fillet with both faces it rolls along by `distance`, which fits.
fn built(made: &Filleted, distance: f64, tol: &Tolerance) -> u64 {
    let offsets: Vec<FaceOffset> = [made.fillet, made.supports[0], made.supports[1]]
        .iter()
        .map(|face| FaceOffset {
            face: *face,
            distance,
        })
        .collect();
    let (made, units) =
        allowance::measured(|| offset_faces_with(&made.body, &offsets, &Blends::default(), tol));
    made.unwrap_or_else(|e| panic!("offset {distance}: {e}"));
    units
}

/// A distance at which the fillet's radius reaches zero is too far, with
/// the radius as what fits, found in closed form: nothing is built in
/// the search (it ends at once).
#[test]
fn a_fillet_whose_radius_would_reach_zero_is_too_far_by_its_radius() {
    let rho = 0.25;
    for k in SCALES {
        let tol = tolerance(k);
        let (body, wall) = twin(&wedge(k));
        let made = filleted(&body, wall, k, rho * k, &tol);
        let faces = [made.fillet, made.supports[0], made.supports[1]];
        let offsets: Vec<FaceOffset> = faces
            .iter()
            .map(|face| FaceOffset {
                face: *face,
                distance: -0.4 * k,
            })
            .collect();
        let (error, refusing) = allowance::measured(|| {
            offset_faces_with(&made.body, &offsets, &Blends::default(), &tol).unwrap_err()
        });
        // Nothing built: a small part of the work of an offset that fits.
        let building = built(&made, -0.1 * k, &tol);
        println!("too far at {k}: {refusing} units, an offset that fits {building}");
        assert!(refusing * 4 < building, "{refusing} against {building}");
        let KernelError::FaceOpTooFar {
            faces,
            largest,
            reason,
            ..
        } = error
        else {
            panic!("not too far: {error}");
        };
        assert_eq!(faces, vec![made.fillet]);
        assert!(reason.contains("radius would reach zero"), "{reason}");
        assert!(
            (largest - (rho * k - tol.linear)).abs() <= 1e-9 * k,
            "what fits: {largest} at {k}"
        );
    }
}

/// A fillet moved without a face it rolls along is refused before
/// anything is built, with both faces: its offset would not be tangent to
/// the face that stays.
#[test]
fn a_fillet_moved_without_the_faces_it_rolls_along_is_refused_at_once() {
    let k = 1.0;
    let tol = tolerance(k);
    let (body, wall) = twin(&wedge(k));
    let made = filleted(&body, wall, k, 0.25, &tol);
    for faces in [vec![made.fillet], vec![made.fillet, made.supports[0]]] {
        let offsets: Vec<FaceOffset> = faces
            .iter()
            .map(|face| FaceOffset {
                face: *face,
                distance: 0.1,
            })
            .collect();
        let (error, refusing) =
            allowance::measured(|| offset_faces(&made.body, &offsets, &tol).unwrap_err());
        let building = built(&made, 0.1, &tol);
        println!("alone: {refusing} units, with its faces {building}");
        assert!(refusing * 4 < building, "{refusing} against {building}");
        let KernelError::FaceOp { faces, reason, .. } = error else {
            panic!("not refused: {error}");
        };
        assert_eq!(faces.first(), Some(&made.fillet));
        assert!(reason.contains("does not move with it"), "{reason}");
    }
    // Moved by different distances: the same.
    let offsets = [
        FaceOffset {
            face: made.fillet,
            distance: 0.1,
        },
        FaceOffset {
            face: made.supports[0],
            distance: 0.1,
        },
        FaceOffset {
            face: made.supports[1],
            distance: 0.2,
        },
    ];
    assert!(matches!(
        offset_faces(&made.body, &offsets, &tol),
        Err(KernelError::FaceOp { .. })
    ));
}

/// The radius a fillet was made with, when told (from its name), is the
/// one rebuilt from: the same body as measuring it gives, to the bit, when
/// they agree; a radius told that the face does not have is not taken
/// (the face is measured instead).
#[test]
fn the_radius_told_is_taken_only_where_the_face_has_it() {
    let k = 1.0;
    let tol = tolerance(k);
    let (body, wall) = twin(&wedge(k));
    let made = filleted(&body, wall, k, 0.25, &tol);
    let faces = [made.fillet, made.supports[0], made.supports[1]];
    let told = |radius: f64| Blends {
        radii: [(made.fillet, radius)].into_iter().collect(),
    };
    let measured = moved(&made.body, &faces, 0.1, &Blends::default(), &tol);
    let right = moved(&made.body, &faces, 0.1, &told(0.25), &tol);
    let wrong = moved(&made.body, &faces, 0.1, &told(0.3), &tol);
    let volume = |b: &Body| mass(b).volume;
    // Measured at the exact sections the face ends in, to rounding.
    assert!((volume(&measured) - volume(&right)).abs() <= 1e-12);
    assert_eq!(volume(&wrong), volume(&measured));
}

/// The same offset twice is the same body, bit for bit.
#[test]
fn a_rebuilt_fillet_is_the_same_every_time() {
    let tol = tolerance(1.0);
    let (body, wall) = twin(&wedge(1.0));
    let made = filleted(&body, wall, 1.0, 0.25, &tol);
    let faces = [made.fillet, made.supports[0], made.supports[1]];
    let a = moved(&made.body, &faces, 0.2, &Blends::default(), &tol);
    let b = moved(&made.body, &faces, 0.2, &Blends::default(), &tol);
    assert_eq!(a, b);
}

/// A fillet rolled along a face an offset made (the wall moved out first,
/// so the fillet's support is itself a fitted offset), moved again with
/// both: the rod's radius moved twice, the ring's tube once.
#[test]
fn a_fillet_on_an_offset_face_moved_again_with_it() {
    let rho = 0.25;
    for k in SCALES {
        let tol = tolerance(k);
        let (body, wall) = twin(&wedge(k));
        let first = offset_faces(
            &body,
            &[FaceOffset {
                face: wall,
                distance: 0.2 * k,
            }],
            &tol,
        )
        .unwrap();
        let made = filleted(&first.body, wall, k, rho * k, &tol);
        let d = if k > 1.0 { -0.1 } else { 0.15 };
        let faces = [made.fillet, made.supports[0], made.supports[1]];
        let result = moved(&made.body, &faces, d * k, &Blends::default(), &tol);
        let m = mass(&result);
        let (r, h, tube) = (2.2 + d, 1.0 + d, rho + d);
        let expected = PI / 4.0 * r * r * h - PI / 2.0 * (r - centroid_in(tube)) * corner(tube);
        let off = m.volume - expected * k.powi(3);
        let slack = 0.5 * tol.linear * m.area;
        println!("twice, k {k}: off {off:e}, slack {slack:e}");
        assert!(off.abs() <= slack, "k {k}: off by {off:e}");
    }
}

/// A block `a × b × c` on the origin whose top is a bilinear spline patch
/// (the same plane, a quarter of its size past each edge), with the edge
/// between its top and its front (y = 0) filleted with radius `rho` by
/// the rolling ball: a cylinder, as a fitted spline. The body, the
/// fillet, the top and the front.
fn rounded_block(
    k: f64,
    [a, b, c]: [f64; 3],
    rho: f64,
    tol: &Tolerance,
) -> (Body, FaceId, FaceId, FaceId) {
    let mut body = boxed(k, [0.0, 0.0, 0.0], [a, b, c]);
    let top = (0..body.faces.len())
        .find(|f| {
            let face = &body.faces[*f];
            matches!(&face.surface, Surface::Plane(p) if {
                let n = p.frame().z() * if face.reversed { -1.0 } else { 1.0 };
                n.z > 0.5
            })
        })
        .unwrap();
    let (x0, x1, y0, y1) = (-0.25 * a * k, 1.25 * a * k, -0.25 * b * k, 1.25 * b * k);
    let z = c * k;
    let patch = NurbsSurface::new(
        1,
        1,
        vec![0.0, 0.0, 1.0, 1.0],
        vec![0.0, 0.0, 1.0, 1.0],
        2,
        2,
        vec![
            Vec3::new(x0, y0, z),
            Vec3::new(x0, y1, z),
            Vec3::new(x1, y0, z),
            Vec3::new(x1, y1, z),
        ],
        vec![1.0; 4],
    )
    .unwrap();
    // ∂u × ∂v is +z, out of the block.
    body.faces[top].surface = Surface::Nurbs(patch);
    body.faces[top].reversed = false;
    assert_eq!(check(&body, tol), []);
    let top = FaceId(top as u32);
    let (edge, front) = (0..body.edges.len())
        .map(|e| EdgeId(e as u32))
        .find_map(|e| {
            let faces = users(&body, e);
            if !faces.contains(&top) {
                return None;
            }
            let other = *faces.iter().find(|f| **f != top)?;
            let data = &body.edges[e.0 as usize];
            let p = data.curve.eval(0.5 * (data.range.0 + data.range.1));
            (p.y.abs() <= 1e-9 * k).then_some((e, other))
        })
        .unwrap();
    let made = fillet(
        &body,
        &[FilletEdge {
            edge,
            radius: rho * k,
        }],
        tol,
    )
    .unwrap();
    assert_eq!(check(&made.body, tol), []);
    let find = |want: &dyn Fn(&FilletFace) -> bool| {
        FaceId(
            (0..made.faces.len())
                .find(|i| want(&made.faces[*i]))
                .unwrap() as u32,
        )
    };
    let fillet_face = find(&|f| matches!(f, FilletFace::Fillet(_)));
    let top_now = find(&|f| matches!(f, FilletFace::Kept(k) if k.contains(&top)));
    let front_now = find(&|f| matches!(f, FilletFace::Kept(k) if k.contains(&front)));
    assert!(matches!(
        made.body.face(fillet_face).unwrap().surface,
        Surface::Nurbs(_)
    ));
    (made.body, fillet_face, top_now, front_now)
}

/// The rounded block's top, front and fillet moved together: the block
/// `d` taller and deeper, the fillet's offset a cylinder of the radius
/// plus the distance along the same axis; the whole block shelled open at
/// its bottom (the hollow's fillet the radius less the thickness); the
/// three faces thickened outward (the layer between the two).
#[test]
fn a_rounded_block_moved_shelled_and_thickened_to_the_closed_form() {
    use craft3d_kernel::thicken::{Side, thicken_with};
    let ([a, b, c], rho) = ([3.0, 2.0, 1.5], 0.4);
    let block = |w: f64, depth: f64, height: f64, r: f64| w * depth * height - w * corner(r);
    for k in SCALES {
        let tol = tolerance(k);
        let (body, fillet_face, top, front) = rounded_block(k, [a, b, c], rho, &tol);
        let faces = [fillet_face, top, front];
        for d in moves(k, [0.25, -0.3]) {
            let result = moved(&body, &faces, d * k, &Blends::default(), &tol);
            let m = mass(&result);
            let expected = block(a, b + d, c + d, rho + d);
            let off = m.volume - expected * k.powi(3);
            let slack = 0.5 * tol.linear * m.area;
            println!("block, k {k}, d {d}: off {off:e}, slack {slack:e}");
            assert!(off.abs() <= slack, "k {k}, d {d}: off by {off:e}");
            // The cylinder about the fillet's axis, moved out of the
            // corner: x along it, the axis at y = ρ, z = c − ρ.
            on_tube(
                &result,
                fillet_face,
                &|p| Vec3::new(p.x, rho * k, (c - rho) * k),
                (rho + d) * k,
                tol.linear,
            );
        }
        // Shelled 0.15 thick, open at the bottom.
        let t = 0.15;
        let bottom = (0..body.faces.len())
            .map(|f| FaceId(f as u32))
            .find(|f| {
                let face = body.face(*f).unwrap();
                matches!(&face.surface, Surface::Plane(p) if {
                    let n = p.frame().z() * if face.reversed { -1.0 } else { 1.0 };
                    n.z < -0.5
                })
            })
            .unwrap();
        let shelled = shell_with(&body, t * k, &[bottom], &Blends::default(), &tol)
            .unwrap_or_else(|e| panic!("shell at {k}: {e}"));
        assert_eq!(check(&shelled.body, &tol), []);
        let m = mass(&shelled.body);
        let expected = block(a, b, c, rho) - block(a - 2.0 * t, b - 2.0 * t, c - t, rho - t);
        let off = m.volume - expected * k.powi(3);
        println!("block shelled, k {k}: off {off:e}");
        assert!(
            off.abs() <= 0.5 * tol.linear * m.area,
            "shell at {k}: off by {off:e}"
        );
        // The three faces thickened outward: the layer between the block
        // and the block moved out.
        let layer = thicken_with(
            &body,
            &faces,
            t * k,
            Side::Outside,
            &Blends::default(),
            &tol,
        )
        .unwrap_or_else(|e| panic!("thicken at {k}: {e}"));
        assert_eq!(check(&layer.body, &tol), []);
        let m = mass(&layer.body);
        let expected = block(a, b + t, c + t, rho + t) - block(a, b, c, rho);
        let off = m.volume - expected * k.powi(3);
        println!("block thickened, k {k}: off {off:e}");
        assert!(
            off.abs() <= 0.5 * tol.linear * m.area,
            "thicken at {k}: off by {off:e}"
        );
    }
}

/// The rounded block (its fillet a fitted spline between a spline top and
/// a flat front) shelled open at its bottom as thick as the fillet's
/// radius or more (2026-10-21): the hollow's edge there is sharp, the
/// fillet collapsed into it, so the hollow is a plain block.
#[test]
fn a_freeform_fillet_shelled_past_its_radius_leaves_a_sharp_hollow() {
    let ([a, b, c], rho) = ([3.0, 2.0, 1.5], 0.4);
    let block = |w: f64, depth: f64, height: f64, r: f64| w * depth * height - w * corner(r);
    for k in SCALES {
        let tol = tolerance(k);
        let (body, fillet_face, _, _) = rounded_block(k, [a, b, c], rho, &tol);
        let bottom = (0..body.faces.len())
            .map(|f| FaceId(f as u32))
            .find(|f| {
                let face = body.face(*f).unwrap();
                matches!(&face.surface, Surface::Plane(p) if {
                    let n = p.frame().z() * if face.reversed { -1.0 } else { 1.0 };
                    n.z < -0.5
                })
            })
            .unwrap();
        for t in [0.4, 0.5] {
            let shelled = shell_with(&body, t * k, &[bottom], &Blends::default(), &tol)
                .unwrap_or_else(|e| panic!("shell {t} at {k}: {e}"));
            assert_eq!(check(&shelled.body, &tol), []);
            let m = mass(&shelled.body);
            let expected = block(a, b, c, rho) - (a - 2.0 * t) * (b - 2.0 * t) * (c - t);
            let off = m.volume - expected * k.powi(3);
            println!("block shelled {t}, k {k}: off {off:e}");
            assert!(
                off.abs() <= 0.5 * tol.linear * m.area,
                "shell {t} at {k}: off by {off:e}"
            );
            // The fillet has a skin and no face in the hollow.
            assert!(shelled.faces.contains(&(Skin::Outer, fillet_face)));
            assert!(!shelled.faces.contains(&(Skin::Inner, fillet_face)));
        }
    }
}

/// What one operation does does not change what the next one gives: a
/// shell after an offset of the same body is the shell alone, bit for bit.
#[test]
fn a_shell_after_an_offset_is_the_shell_alone() {
    let tol = tolerance(1.0);
    let (body, _) = balls(1.0, true, 2.0, 1.0, 0.5, &tol);
    let alone = shell_with(&body, 0.2, &[], &Blends::default(), &tol).unwrap();
    let all: Vec<FaceId> = (0..body.faces.len()).map(|f| FaceId(f as u32)).collect();
    let _ = moved(&body, &all, 0.2, &Blends::default(), &tol);
    let after = shell_with(&body, 0.2, &[], &Blends::default(), &tol).unwrap();
    println!(
        "alone {}, after {}",
        mass(&alone.body).volume,
        mass(&after.body).volume
    );
    assert_eq!(alone, after);
}

/// The rounded block's fillet face replaced by a spline of 33 rows across,
/// of degree one along the fillet, each row the quarter arc of the ball at
/// that place (the exact cylinder: rows that are translates interpolate
/// to translates), but for `tilt` (the middle control point of each inner
/// row turned about the chord by that angle) and `bump` (the middle weight
/// of every odd row times that): the face is then no fillet, though its
/// borders stay on its faces and its end rows are the fillet's.
fn imposter(k: f64, tilt: f64, bump: f64) -> (Body, FaceId, [FaceId; 3]) {
    let ([a, b, c], rho) = ([3.0, 2.0, 1.5], 0.4);
    let tol = tolerance(k);
    let (mut body, fillet_face, top, front) = rounded_block(k, [a, b, c], rho, &tol);
    let Surface::Nurbs(old) = &body.face(fillet_face).unwrap().surface else {
        panic!("not a spline")
    };
    let ((u0, u1), _) = old.domain();
    // Which way the old surface runs: v = 0 on the front or on the top,
    // u along +x or −x.
    let start = old.eval(u0, 0.0);
    let on_front = start.y.abs() <= 1e-6 * k;
    let along = if old.eval(u1, 0.0).x > start.x {
        1.0
    } else {
        -1.0
    };
    let rows = 33;
    let w = std::f64::consts::FRAC_1_SQRT_2;
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..rows {
        let t = i as f64 / (rows - 1) as f64;
        let x = if along > 0.0 { a * t } else { a * (1.0 - t) } * k;
        let front_foot = Vec3::new(x, 0.0, (c - rho) * k);
        let top_foot = Vec3::new(x, rho * k, c * k);
        let corner = Vec3::new(x, 0.0, c * k);
        let (q1, q2) = if on_front {
            (front_foot, top_foot)
        } else {
            (top_foot, front_foot)
        };
        let inner = i > 0 && i + 1 < rows;
        let mut middle = corner;
        if inner && tilt != 0.0 {
            // Turned about the chord q1 → q2.
            let axis = (q2 - q1).normalized().unwrap();
            let off = corner - q1;
            let along_axis = axis * off.dot(axis);
            let radial = off - along_axis;
            let turned = radial * tilt.cos()
                + axis.cross(radial) * tilt.sin()
                + axis * axis.dot(radial) * (1.0 - tilt.cos());
            middle = q1 + along_axis + turned;
        }
        let weight = if inner && i % 2 == 1 { w * bump } else { w };
        ctrl.extend([q1, middle, q2]);
        weights.extend([1.0, weight, 1.0]);
    }
    let mut knots_u = vec![u0, u0];
    for i in 1..rows - 1 {
        knots_u.push(u0 + (u1 - u0) * i as f64 / (rows - 1) as f64);
    }
    knots_u.extend([u1, u1]);
    let surface = NurbsSurface::new(
        1,
        2,
        knots_u,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        rows,
        3,
        ctrl,
        weights,
    )
    .unwrap();
    body.faces[fillet_face.0 as usize].surface = Surface::Nurbs(surface);
    (body, fillet_face, [fillet_face, top, front])
}

/// A fillet face is taken for one, and rebuilt, only where it is proven
/// to be the rolling-ball fillet between its faces: rows turned off the
/// ball's great circle (each still an arc on a ball through both feet,
/// tangent to both faces) and rows that hold only where they are sampled
/// (the odd ones bulged by their weight) are not, at three scales; the
/// same spline with true rows is.
#[test]
fn a_face_that_only_looks_like_a_fillet_is_not_rebuilt() {
    for k in SCALES {
        let tol = tolerance(k);
        let (body, face, faces) = imposter(k, 0.0, 1.0);
        assert_eq!(check(&body, &tol), [], "the true rows at {k}");
        assert_eq!(
            craft3d_kernel::offset_face::fillets_recognised(&body, &faces, &tol),
            vec![face],
            "the true rows at {k}"
        );
        for tilt in [0.05, 0.2, 0.5] {
            let (body, _, faces) = imposter(k, tilt, 1.0);
            assert_eq!(
                craft3d_kernel::offset_face::fillets_recognised(&body, &faces, &tol),
                vec![],
                "rows turned {tilt} at {k}"
            );
        }
        for bump in [1.02, 1.1] {
            let (body, _, faces) = imposter(k, 0.0, bump);
            assert_eq!(
                craft3d_kernel::offset_face::fillets_recognised(&body, &faces, &tol),
                vec![],
                "odd rows bulged {bump} at {k}"
            );
        }
    }
}
