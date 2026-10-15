//! Faces on freeform surfaces that close on themselves: a NURBS surface
//! of revolution whose first and last rows of control points are the same
//! curve. `craft3d-geom` reports no period for such a surface; the
//! tessellator finds it from the control net and follows the face across
//! the seam as it does on a cylinder.
//!
//! The bodies are built by hand, as a STEP file gives them: the kernel
//! itself makes no face on a surface that closes (a full turn of a spline
//! is two half faces). The first of them is the body that came back as a
//! mesh that was not closed, without an error, on 2026-10-13.
//!
//! The oracle is closed form: a profile `(ρ(t), z(t))` turned about the z
//! axis encloses `π ∫ ρ² z′ dt` and has the side area `2π ∫ ρ |c′| dt`,
//! summed here by Gauss–Legendre on the profile's own Bézier form.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Line, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{Mesh, TessError, TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, LoopId, check, mass_properties};
use std::f64::consts::{FRAC_PI_4, PI, TAU};

fn params(tolerance: f64) -> TessParams {
    TessParams {
        chordal_tolerance: tolerance,
        ..TessParams::default()
    }
}

/// The unit circle as nine rational quadratic control points `(x, y, w)`
/// and its knots, starting at `from` radians: four quarter arcs.
fn ring(from: f64) -> (Vec<(f64, f64, f64)>, Vec<f64>) {
    let weight = FRAC_PI_4.cos();
    let points = (0..9)
        .map(|k| {
            let (sin, cos) = (from + FRAC_PI_4 * f64::from(k)).sin_cos();
            if k % 2 == 0 {
                (cos, sin, 1.0)
            } else {
                (cos / weight, sin / weight, weight)
            }
        })
        .collect();
    let knots = vec![
        0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
    ];
    (points, knots)
}

/// A cubic Bézier profile in the half-plane `(ρ, z)`.
type Profile = [(f64, f64); 4];

/// A vase: wide, narrow, wide.
fn vase(k: f64) -> Profile {
    [
        (1.0 * k, 0.0),
        (1.9 * k, 0.7 * k),
        (0.3 * k, 1.4 * k),
        (1.2 * k, 2.0 * k),
    ]
}

/// A barrel: a gentle bulge. (The body that came back open: its seam
/// points are projected now to one end of the surface's range, now to the
/// other.)
fn barrel(k: f64) -> Profile {
    [
        (1.0 * k, 0.0),
        (1.3 * k, 0.5 * k),
        (1.3 * k, 1.5 * k),
        (1.0 * k, 2.0 * k),
    ]
}

fn bezier_knots() -> Vec<f64> {
    vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]
}

/// The surface `profile` sweeps in a full turn about the z axis, starting
/// at `from` radians. With `around_v` the turn is the surface's second
/// parameter and the profile its first.
fn revolved(profile: &Profile, from: f64, around_v: bool) -> NurbsSurface {
    let (ring, ring_knots) = ring(from);
    let at = |(x, y, w): (f64, f64, f64), (rho, z): (f64, f64)| (Vec3::new(rho * x, rho * y, z), w);
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    if around_v {
        for &p in profile {
            for &r in &ring {
                let (point, w) = at(r, p);
                ctrl.push(point);
                weights.push(w);
            }
        }
        NurbsSurface::new(3, 2, bezier_knots(), ring_knots, 4, 9, ctrl, weights).unwrap()
    } else {
        for &r in &ring {
            for &p in profile {
                let (point, w) = at(r, p);
                ctrl.push(point);
                weights.push(w);
            }
        }
        NurbsSurface::new(2, 3, ring_knots, bezier_knots(), 9, 4, ctrl, weights).unwrap()
    }
}

/// The profile as a curve in space, in the half-plane at `angle`.
fn profile_curve(profile: &Profile, angle: f64) -> Curve3 {
    let (sin, cos) = angle.sin_cos();
    let ctrl = profile
        .iter()
        .map(|&(rho, z)| Vec3::new(rho * cos, rho * sin, z))
        .collect();
    Curve3::Nurbs(NurbsCurve::polynomial(3, bezier_knots(), ctrl).unwrap())
}

fn bernstein(profile: &Profile, t: f64) -> ((f64, f64), (f64, f64)) {
    let s = 1.0 - t;
    let b = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
    let d = [s * s, 2.0 * s * t, t * t];
    let mut point = (0.0, 0.0);
    let mut slope = (0.0, 0.0);
    for i in 0..4 {
        point.0 += b[i] * profile[i].0;
        point.1 += b[i] * profile[i].1;
    }
    for i in 0..3 {
        slope.0 += 3.0 * d[i] * (profile[i + 1].0 - profile[i].0);
        slope.1 += 3.0 * d[i] * (profile[i + 1].1 - profile[i].1);
    }
    (point, slope)
}

/// `∫₀¹ f(point, slope) dt` along the profile.
fn along(profile: &Profile, f: impl Fn((f64, f64), (f64, f64)) -> f64) -> f64 {
    along_part(profile, (0.0, 1.0), f)
}

/// `∫ f(point, slope) dt` along the profile over `range`: five
/// Gauss–Legendre points on each of 200 stretches.
fn along_part(
    profile: &Profile,
    range: (f64, f64),
    f: impl Fn((f64, f64), (f64, f64)) -> f64,
) -> f64 {
    const GAUSS: [(f64, f64); 5] = [
        (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
        (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
        (0.0, 0.568_888_888_888_888_9),
        (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
        (0.906_179_845_938_664, 0.236_926_885_056_189_1),
    ];
    let stretches = 200;
    let h = (range.1 - range.0) / f64::from(stretches);
    let mut sum = 0.0;
    for i in 0..stretches {
        for &(x, w) in &GAUSS {
            let t = range.0 + h * (f64::from(i) + (x + 1.0) / 2.0);
            let (point, slope) = bernstein(profile, t);
            sum += f(point, slope) * w * h / 2.0;
        }
    }
    sum
}

/// Volume and area of the solid the profile and its two discs bound.
fn exact(profile: &Profile) -> (f64, f64) {
    let volume = PI * along(profile, |(rho, _), (_, dz)| rho * rho * dz);
    let side = TAU * along(profile, |(rho, _), (dr, dz)| rho * dr.hypot(dz));
    let (bottom, top) = (profile[0].0, profile[3].0);
    (volume, side + PI * (bottom * bottom + top * top))
}

/// How the wall of a turned profile is given.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Wall {
    /// One loop: the lower circle, the seam up, the upper circle back, the
    /// seam down. What a STEP file has.
    Seam,
    /// Two loops, a circle each, and no seam edge: what the kernel's own
    /// cylinder has.
    Circles,
}

/// The solid `profile` bounds when turned about the z axis: one wall on a
/// surface that closes on itself, and two discs. The surface starts at
/// `from` radians and the seam edge, when there is one, lies there too.
fn turned(profile: &Profile, wall: Wall, from: f64, around_v: bool) -> Body {
    let mut body = Body::new(BodyKind::Solid);
    let (sin, cos) = from.sin_cos();
    let x_axis = Vec3::new(cos, sin, 0.0);
    let rim = |body: &mut Body, (rho, z): (f64, f64)| -> EdgeId {
        let vertex = body.add_vertex(Vec3::new(rho * cos, rho * sin, z));
        let frame = Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, x_axis).unwrap();
        let circle = Curve3::Circle(Circle::new(frame, rho).unwrap());
        body.add_edge(circle, (0.0, TAU), vertex, vertex)
    };
    let lower = rim(&mut body, profile[0]);
    let upper = rim(&mut body, profile[3]);
    let use_of = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
    // Around the axis first and the profile second, `∂u × ∂v` points away
    // from the axis; the other way round it points at it.
    let loops = match wall {
        Wall::Seam => {
            let (start, end) = (body.edges[0].start, body.edges[1].start);
            let seam = body.add_edge(profile_curve(profile, from), (0.0, 1.0), start, end);
            vec![body.add_loop(vec![
                use_of(lower, false),
                use_of(seam, false),
                use_of(upper, true),
                use_of(seam, true),
            ])]
        }
        Wall::Circles => vec![
            body.add_loop(vec![use_of(lower, false)]),
            body.add_loop(vec![use_of(upper, true)]),
        ],
    };
    let surface = Surface::Nurbs(revolved(profile, from, around_v));
    let side = body.add_face(surface, around_v, loops);
    let floor_loop = body.add_loop(vec![use_of(lower, true)]);
    let floor_frame = Frame::new(Vec3::new(0.0, 0.0, profile[0].1), -Vec3::Z, x_axis).unwrap();
    let floor = body.add_face(
        Surface::Plane(Plane::new(floor_frame)),
        false,
        vec![floor_loop],
    );
    let lid_loop = body.add_loop(vec![use_of(upper, false)]);
    let lid_frame = Frame::new(Vec3::new(0.0, 0.0, profile[3].1), Vec3::Z, x_axis).unwrap();
    let lid = body.add_face(Surface::Plane(Plane::new(lid_frame)), false, vec![lid_loop]);
    body.add_shell(vec![side, floor, lid]);
    body
}

fn same_bits(a: &Mesh, b: &Mesh) -> bool {
    let bits = |m: &Mesh| -> Vec<u64> {
        m.positions
            .iter()
            .chain(&m.normals)
            .flat_map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
            .collect()
    };
    bits(a) == bits(b) && a.triangles == b.triangles && a.face_ids == b.face_ids
}

/// The body is valid and measures what the closed form says; its mesh is
/// closed, faces outward, converges on the volume and the area, has its
/// vertices on the surface, and does not depend on the number of threads.
///
/// `fine` adds the finest tolerance; the vertices are held against the
/// surface at the coarse ones only (a projection onto a spline surface is
/// a search).
fn meshes_as(body: &Body, profile: &Profile, what: &str, fine: bool) {
    let k = profile[0].0;
    let (volume, area) = exact(profile);
    assert_eq!(check(body, &Tolerance::default()), vec![], "{what}");
    // The exact measure, whichever parameter the surface closes in.
    let measured = mass_properties(body).unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(
        (measured.volume - volume).abs() < 1e-9 * volume,
        "{what}: the exact measure gives {} for {volume}",
        measured.volume
    );
    assert!(
        (measured.area - area).abs() < 1e-9 * area,
        "{what}: the exact measure gives the area {} for {area}",
        measured.area
    );
    let mut last = f64::INFINITY;
    for relative in [0.1, 1e-2, 1e-3, 1e-4] {
        if relative < 1e-3 && !fine {
            continue;
        }
        let tolerance = relative * k;
        let mesh = tessellate_body(body, &params(tolerance))
            .unwrap_or_else(|e| panic!("{what} at {relative}: {e}"));
        let welded = mesh.weld().unwrap();
        assert!(
            welded.is_closed(),
            "{what} at {relative}: the mesh is not closed ({} triangles)",
            mesh.triangles.len()
        );
        let error = (welded.volume() - volume).abs();
        assert!(
            error <= area * tolerance,
            "{what} at {relative}: volume {} for {volume}",
            welded.volume()
        );
        assert!(
            error < last || error < 1e-9 * volume,
            "{what} at {relative}: the volume does not converge"
        );
        last = error;
        let mesh_area = mesh.area();
        assert!(mesh_area <= area * (1.0 + 1e-9), "{what} at {relative}");
        if relative <= 1e-3 {
            assert!(
                (mesh_area - area).abs() < 20.0 * relative * area,
                "{what} at {relative}: area {mesh_area} for {area}"
            );
        }
        // Every vertex of the wall is a point of the surface, and its
        // normal the surface's, away from the axis.
        let Surface::Nurbs(surface) = &body.faces[0].surface else {
            panic!("the wall is a NURBS face");
        };
        let wall = Surface::Nurbs(surface.clone());
        for (t, triangle) in mesh.triangles.iter().enumerate() {
            if mesh.face_ids[t] != 0 {
                continue;
            }
            for &v in triangle {
                let (p, n) = (mesh.positions[v as usize], mesh.normals[v as usize]);
                if relative > 1e-2 {
                    let (u, w) = wall.project(p).unwrap();
                    assert!(wall.eval(u, w).distance(p) < 1e-9 * k, "{what}: off");
                    let exact = wall.normal(u, w).unwrap()
                        * if body.faces[0].reversed { -1.0 } else { 1.0 };
                    assert!(n.distance(exact) < 1e-6, "{what}: {n:?} for {exact:?}");
                }
                let outward = Vec3::new(p.x, p.y, 0.0);
                assert!(n.dot(outward) > 0.0, "{what}: a normal faces the axis");
                assert!((n.length() - 1.0).abs() < 1e-12, "{what}: {n:?}");
            }
            let [a, b, c] = mesh.corners(t).unwrap();
            let facing = (b - a).cross(c - a);
            let middle = (a + b + c) * (1.0 / 3.0);
            assert!(
                facing.dot(Vec3::new(middle.x, middle.y, 0.0)) > 0.0,
                "{what} at {relative}: triangle {t} faces the axis"
            );
        }
        for threads in [1, 2, 8] {
            if relative < 1e-2 {
                break;
            }
            let again = tessellate_body_with_threads(body, &params(tolerance), threads).unwrap();
            assert!(same_bits(&mesh, &again), "{what} on {threads} threads");
        }
    }
}

/// The body of 2026-10-13: a full turn as one face with a seam edge.
#[test]
fn a_full_turn_with_a_seam_edge_is_meshed_closed() {
    for k in [1.0, 1e-3, 1e3] {
        let profile = vase(k);
        meshes_as(
            &turned(&profile, Wall::Seam, 0.0, false),
            &profile,
            &format!("seam, size {k}"),
            k == 1.0,
        );
    }
}

/// The same wall between two circles, without a seam edge: the
/// tessellator cuts it open itself.
#[test]
fn a_full_turn_between_two_circles_is_meshed_closed() {
    for k in [1.0, 1e-3, 1e3] {
        let profile = vase(k);
        meshes_as(
            &turned(&profile, Wall::Circles, 0.0, false),
            &profile,
            &format!("circles, size {k}"),
            k == 1.0,
        );
    }
}

/// The surface need not start where the seam edge of the face is, nor
/// where its circles start; and it may close in its second parameter.
#[test]
fn the_seam_of_the_surface_may_lie_anywhere_and_in_either_parameter() {
    for (name, profile) in [("vase", vase(1.0)), ("barrel", barrel(1.0))] {
        for wall in [Wall::Seam, Wall::Circles] {
            for from in [0.0, 0.4, 2.0, -1.3] {
                for around_v in [false, true] {
                    meshes_as(
                        &turned(&profile, wall, from, around_v),
                        &profile,
                        &format!("{name}, {wall:?} from {from}, around v: {around_v}"),
                        false,
                    );
                }
            }
        }
    }
}

/// Not the tessellator's: `craft3d_topo::mass_properties` on the same
/// walls when the surface closes in its second parameter (the profile
/// along u, the turn along v, the face reversed). Until 2026-10-14 the
/// measure refused some of them and gave a wrong volume for others without
/// an error (2.555 for the vase of 7.758): its boundary integral ran
/// against `du`, and the seam where the surface closes in v, which no edge
/// stands for in the parameter plane, is a line along u. Every one of them
/// is measured now, to the closed form.
#[test]
fn the_exact_measure_of_a_wall_that_closes_in_v() {
    let mut wrong = Vec::new();
    for (name, profile) in [("vase", vase(1.0)), ("barrel", barrel(1.0))] {
        let (volume, area) = exact(&profile);
        for wall in [Wall::Seam, Wall::Circles] {
            for from in [0.0, 0.4, 2.0, -1.3] {
                let body = turned(&profile, wall, from, true);
                assert_eq!(check(&body, &Tolerance::default()), vec![]);
                let what = format!("{name}, {wall:?} from {from}");
                match mass_properties(&body) {
                    Ok(measured)
                        if (measured.volume - volume).abs() < 1e-9 * volume
                            && (measured.area - area).abs() < 1e-9 * area => {}
                    Ok(measured) => wrong.push(format!(
                        "{what}: {} for {volume}, area {} for {area}",
                        measured.volume, measured.area
                    )),
                    Err(error) => wrong.push(format!("{what}: refused: {error}")),
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A sheet of one face.
fn sheet(surface: Surface, reversed: bool, build: impl FnOnce(&mut Body) -> Vec<LoopId>) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let loops = build(&mut body);
    let face = body.add_face(surface, reversed, loops);
    body.add_shell(vec![face]);
    body
}

/// The area of the mesh converges on `area` from below, and every
/// triangle faces away from the axis.
fn sheet_meshes_as(body: &Body, area: f64, k: f64, what: &str) {
    assert_eq!(check(body, &Tolerance::default()), vec![], "{what}");
    for relative in [0.1, 1e-2, 1e-3] {
        let mesh = tessellate_body(body, &params(relative * k))
            .unwrap_or_else(|e| panic!("{what} at {relative}: {e}"));
        let mesh_area = mesh.area();
        assert!(mesh_area <= area * (1.0 + 1e-9), "{what} at {relative}");
        assert!(
            (mesh_area - area).abs() < 30.0 * relative * area,
            "{what} at {relative}: area {mesh_area} for {area}"
        );
        for t in 0..mesh.triangles.len() {
            let [a, b, c] = mesh.corners(t).unwrap();
            let middle = (a + b + c) * (1.0 / 3.0);
            assert!(
                (b - a).cross(c - a).dot(Vec3::new(middle.x, middle.y, 0.0)) > 0.0,
                "{what} at {relative}: triangle {t} faces the axis"
            );
        }
        for threads in [1, 2, 8] {
            let again = tessellate_body_with_threads(body, &params(relative * k), threads).unwrap();
            assert!(same_bits(&mesh, &again), "{what} on {threads} threads");
        }
    }
}

/// A window in the wall, across the seam of the surface: on its own (a
/// patch whose loop crosses the seam) and as a hole in the wall (a face
/// that goes around, with a hole on the seam).
#[test]
fn a_window_across_the_seam_and_the_wall_around_it() {
    let profile = vase(1.0);
    let (t0, t1) = (0.3, 0.7);
    for from in [0.0, 2.0] {
        for (a0, a1) in [(from - 0.5, from + 0.6), (from + 0.3, from + 1.4)] {
            let strip = TAU * along(&profile, |(rho, _), (dr, dz)| rho * dr.hypot(dz));
            let pane =
                (a1 - a0) * along_part(&profile, (t0, t1), |(rho, _), (dr, dz)| rho * dr.hypot(dz));
            // The window's four edges; its loop as it runs around the
            // window, and backwards as a hole.
            let window = |body: &mut Body, hole: bool| -> LoopId {
                let ((low, _), (high, _)) = (bernstein(&profile, t0), bernstein(&profile, t1));
                let corner =
                    |(rho, z): (f64, f64), a: f64| Vec3::new(rho * a.cos(), rho * a.sin(), z);
                let corners = [
                    body.add_vertex(corner(low, a0)),
                    body.add_vertex(corner(low, a1)),
                    body.add_vertex(corner(high, a1)),
                    body.add_vertex(corner(high, a0)),
                ];
                let arc = |(rho, z): (f64, f64)| {
                    let x_axis = Vec3::new(a0.cos(), a0.sin(), 0.0);
                    let frame = Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, x_axis).unwrap();
                    Curve3::Circle(Circle::new(frame, rho).unwrap())
                };
                let below = body.add_edge(arc(low), (0.0, a1 - a0), corners[0], corners[1]);
                let above = body.add_edge(arc(high), (0.0, a1 - a0), corners[3], corners[2]);
                let right = body.add_edge(
                    profile_curve(&profile, a1),
                    (t0, t1),
                    corners[1],
                    corners[2],
                );
                let left = body.add_edge(
                    profile_curve(&profile, a0),
                    (t0, t1),
                    corners[0],
                    corners[3],
                );
                let use_of = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
                body.add_loop(if hole {
                    vec![
                        use_of(left, false),
                        use_of(above, false),
                        use_of(right, true),
                        use_of(below, true),
                    ]
                } else {
                    vec![
                        use_of(below, false),
                        use_of(right, false),
                        use_of(above, true),
                        use_of(left, true),
                    ]
                })
            };
            let surface = || Surface::Nurbs(revolved(&profile, from, false));
            let what = format!("from {from}, between {a0} and {a1}");
            let alone = sheet(surface(), false, |body| vec![window(body, false)]);
            sheet_meshes_as(&alone, pane, 1.0, &format!("a window {what}"));
            let around = sheet(surface(), false, |body| {
                let rim = |body: &mut Body, (rho, z): (f64, f64)| -> EdgeId {
                    let vertex = body.add_vertex(Vec3::new(rho, 0.0, z));
                    let frame = Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap();
                    let circle = Curve3::Circle(Circle::new(frame, rho).unwrap());
                    body.add_edge(circle, (0.0, TAU), vertex, vertex)
                };
                let lower = rim(body, profile[0]);
                let upper = rim(body, profile[3]);
                vec![
                    body.add_loop(vec![Coedge {
                        edge: lower,
                        reversed: false,
                    }]),
                    body.add_loop(vec![Coedge {
                        edge: upper,
                        reversed: true,
                    }]),
                    window(body, true),
                ]
            });
            sheet_meshes_as(
                &around,
                strip - pane,
                1.0,
                &format!("the wall around {what}"),
            );
        }
    }
}

/// A torus about the z axis as a NURBS surface: u around the axis from
/// the x axis, v around the tube from its outer equator; both close.
fn torus_surface(major: f64, minor: f64) -> Surface {
    let (ring, knots) = ring(0.0);
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for &(x, y, w) in &ring {
        for &(tx, tz, tw) in &ring {
            let rho = major + minor * tx;
            ctrl.push(Vec3::new(rho * x, rho * y, minor * tz));
            weights.push(w * tw);
        }
    }
    Surface::Nurbs(NurbsSurface::new(2, 2, knots.clone(), knots, 9, 9, ctrl, weights).unwrap())
}

/// A surface that closes in both parameters: a torus as a NURBS surface.
/// `step` gives it as a STEP file does, with its two seams as edges that
/// the one loop walks there and back; otherwise it has no edges at all.
fn nurbs_torus(major: f64, minor: f64, step: bool) -> Body {
    let surface = torus_surface(major, minor);
    let mut body = Body::new(BodyKind::Solid);
    let loops = if step {
        let vertex = body.add_vertex(Vec3::new(major + minor, 0.0, 0.0));
        let around_axis = Curve3::Circle(Circle::new(Frame::WORLD, major + minor).unwrap());
        // From the vertex up: counter-clockwise about −y.
        let tube = Frame::new(Vec3::new(major, 0.0, 0.0), -Vec3::Y, Vec3::X).unwrap();
        let around_tube = Curve3::Circle(Circle::new(tube, minor).unwrap());
        let long = body.add_edge(around_axis, (0.0, TAU), vertex, vertex);
        let short = body.add_edge(around_tube, (0.0, TAU), vertex, vertex);
        let use_of = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
        vec![body.add_loop(vec![
            use_of(long, false),
            use_of(short, false),
            use_of(long, true),
            use_of(short, true),
        ])]
    } else {
        Vec::new()
    };
    let face = body.add_face(surface, false, loops);
    body.add_shell(vec![face]);
    body
}

#[test]
fn a_surface_that_closes_both_ways_is_meshed_closed() {
    for k in [1.0, 1e-3, 1e3] {
        let (major, minor) = (3.0 * k, k);
        let (volume, area) = (
            2.0 * PI * PI * major * minor * minor,
            4.0 * PI * PI * major * minor,
        );
        for step in [false, true] {
            let body = nurbs_torus(major, minor, step);
            let what = format!("torus of size {k}, seams as edges: {step}");
            assert_eq!(check(&body, &Tolerance::default()), vec![], "{what}");
            let mut last = f64::INFINITY;
            for relative in [0.1, 1e-2, 1e-3] {
                let tolerance = relative * k;
                let mesh = tessellate_body(&body, &params(tolerance))
                    .unwrap_or_else(|e| panic!("{what} at {relative}: {e}"));
                let welded = mesh.weld().unwrap();
                assert!(welded.is_closed(), "{what} at {relative}: not closed");
                let error = (welded.volume() - volume).abs();
                assert!(error <= area * tolerance, "{what} at {relative}");
                assert!(error < last, "{what} at {relative}: no convergence");
                last = error;
                assert!(mesh.area() <= area * (1.0 + 1e-9), "{what} at {relative}");
                // One handle: V − E + F = 0.
                let (v, f) = (welded.positions.len() as i64, welded.triangles.len() as i64);
                assert_eq!(v - 3 * f / 2 + f, 0, "{what} at {relative}");
                for threads in [1, 2, 8] {
                    let again =
                        tessellate_body_with_threads(&body, &params(tolerance), threads).unwrap();
                    assert!(same_bits(&mesh, &again), "{what} on {threads} threads");
                }
            }
        }
    }
}

/// A profile from the axis back to the axis: the surface closes around
/// the axis and to a point at both ends, as a sphere does.
fn egg(k: f64) -> Profile {
    [
        (0.0, 0.0),
        (1.5 * k, 0.0),
        (1.5 * k, 2.0 * k),
        (0.0, 2.0 * k),
    ]
}

#[test]
fn a_surface_that_closes_to_a_point_at_both_ends_is_meshed_closed() {
    for k in [1.0, 1e-3, 1e3] {
        let profile = egg(k);
        let volume = PI * along(&profile, |(rho, _), (_, dz)| rho * rho * dz);
        let area = TAU * along(&profile, |(rho, _), (dr, dz)| rho * dr.hypot(dz));
        // Without edges, and with its seam as an edge from pole to pole
        // that the one loop walks there and back, as a STEP file has it:
        // the loop turns around each pole.
        for (from, seam) in [(0.0, false), (1.1, false), (0.0, true), (1.1, true)] {
            let mut body = Body::new(BodyKind::Solid);
            let surface = Surface::Nurbs(revolved(&profile, from, false));
            let loops = if seam {
                let (low, high) = (
                    body.add_vertex(Vec3::ZERO),
                    body.add_vertex(Vec3::new(0.0, 0.0, profile[3].1)),
                );
                let edge = body.add_edge(profile_curve(&profile, from), (0.0, 1.0), low, high);
                let use_of = |reversed: bool| Coedge { edge, reversed };
                vec![body.add_loop(vec![use_of(false), use_of(true)])]
            } else {
                Vec::new()
            };
            let face = body.add_face(surface, false, loops);
            body.add_shell(vec![face]);
            assert_eq!(check(&body, &Tolerance::default()), vec![]);
            let what = format!("egg of size {k} from {from}, seam as an edge: {seam}");
            let mut last = f64::INFINITY;
            for relative in [0.1, 1e-2, 1e-3] {
                let tolerance = relative * k;
                let mesh = tessellate_body(&body, &params(tolerance))
                    .unwrap_or_else(|e| panic!("{what} at {relative}: {e}"));
                let welded = mesh.weld().unwrap();
                assert!(welded.is_closed(), "{what} at {relative}: not closed");
                let error = (welded.volume() - volume).abs();
                assert!(error <= area * tolerance, "{what} at {relative}");
                assert!(error < last, "{what} at {relative}: no convergence");
                last = error;
                let (v, f) = (welded.positions.len() as i64, welded.triangles.len() as i64);
                assert_eq!(v - 3 * f / 2 + f, 2, "{what} at {relative}");
            }
        }
    }
}

/// What is not followed is refused, by an error at the face: never a
/// mesh of something else.
#[test]
fn what_is_not_followed_is_refused() {
    let refused = |body: &Body, what: &str, word: &str| {
        for tolerance in [0.3, 0.1, 1e-2, 1e-3] {
            match tessellate_body(body, &params(tolerance)) {
                Err(TessError::Face { face: 0, reason }) => {
                    assert!(reason.contains(word), "{what} at {tolerance}: {reason}");
                }
                Err(TessError::Unsupported { face: 0, .. }) if word.is_empty() => {}
                other => panic!(
                    "{what} at {tolerance}: {:?}",
                    other.map(|mesh| mesh.triangles.len())
                ),
            }
        }
    };

    // A tube that closes on knots that are not clamped (a uniform
    // quadratic B-spline through an octagon, its first two rows of control
    // points repeated at the end): the control net does not show it, so
    // the seam is not followed. Along the seam the boundary jumps from one
    // end of the surface's range to the other.
    let (radius, height) = (1.5, 2.0);
    let corner = |i: usize| {
        let a = TAU * (i % 8) as f64 / 8.0;
        Vec3::new(radius * a.cos(), radius * a.sin(), 0.0)
    };
    let knots: Vec<f64> = (0..13).map(f64::from).collect();
    let mut ctrl = Vec::new();
    for i in 0..10 {
        ctrl.push(corner(i));
        ctrl.push(corner(i) + Vec3::new(0.0, 0.0, height));
    }
    let tube = || {
        Surface::Nurbs(
            NurbsSurface::new(
                2,
                1,
                knots.clone(),
                vec![0.0, 0.0, 1.0, 1.0],
                10,
                2,
                ctrl.clone(),
                vec![1.0; 20],
            )
            .unwrap(),
        )
    };
    let rim = |z: f64| {
        let points = (0..10)
            .map(|i| corner(i) + Vec3::new(0.0, 0.0, z))
            .collect();
        Curve3::Nurbs(NurbsCurve::polynomial(2, knots.clone(), points).unwrap())
    };
    for with_seam in [true, false] {
        let body = sheet(tube(), false, |body| {
            let start = rim(0.0).eval(2.0);
            let (low, high) = (
                body.add_vertex(start),
                body.add_vertex(start + Vec3::new(0.0, 0.0, height)),
            );
            let lower = body.add_edge(rim(0.0), (2.0, 10.0), low, low);
            let upper = body.add_edge(rim(height), (2.0, 10.0), high, high);
            let use_of = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
            if with_seam {
                let line = Curve3::Line(
                    Line::through(start, start + Vec3::new(0.0, 0.0, height)).unwrap(),
                );
                let seam = body.add_edge(line, (0.0, height), low, high);
                vec![body.add_loop(vec![
                    use_of(lower, false),
                    use_of(seam, false),
                    use_of(upper, true),
                    use_of(seam, true),
                ])]
            } else {
                vec![
                    body.add_loop(vec![use_of(lower, false)]),
                    body.add_loop(vec![use_of(upper, true)]),
                ]
            }
        });
        assert_eq!(check(&body, &Tolerance::default()), vec![]);
        refused(&body, &format!("knots not clamped, seam: {with_seam}"), "");
    }
}

/// The exact measure of `body` is the closed form, to 1e-12; and the mesh
/// at a thousandth of `k` encloses what the exact measure says, within
/// the area times that tolerance.
fn measures_as(body: &Body, expected: (f64, f64), k: f64, what: &str) {
    measured_as(body, expected, what);
    if body.kind == BodyKind::Sheet {
        return;
    }
    let volume = mass_properties(body).unwrap().volume;
    let tolerance = 1e-3 * k;
    let mesh = tessellate_body(body, &params(tolerance))
        .unwrap_or_else(|e| panic!("{what}: {e}"))
        .weld()
        .unwrap();
    assert!(mesh.is_closed(), "{what}: the mesh is not closed");
    assert!(
        (mesh.volume() - volume).abs() <= expected.1 * tolerance,
        "{what}: the mesh encloses {} where the exact measure says {volume}",
        mesh.volume(),
    );
}

/// The exact measure of `body` is the closed form, to 1e-12 (measured:
/// 4e-15 in the area, 1.4e-14 in the volume, at worst).
fn measured_as(body: &Body, (volume, area): (f64, f64), what: &str) {
    assert_eq!(check(body, &Tolerance::default()), vec![], "{what}");
    let measured = mass_properties(body).unwrap_or_else(|e| panic!("{what}: {e}"));
    println!(
        "{what}: area off by {:.1e}, volume by {:.1e}",
        (measured.area - area).abs() / area,
        (measured.volume - volume).abs() / volume.abs().max(f64::MIN_POSITIVE)
    );
    assert!(
        (measured.area - area).abs() < 1e-12 * area,
        "{what}: the area {} for {area}",
        measured.area
    );
    if body.kind == BodyKind::Sheet {
        return;
    }
    assert!(
        (measured.volume - volume).abs() < 1e-12 * volume,
        "{what}: the volume {} for {volume}",
        measured.volume
    );
}

/// A straight profile: the wall of a cylinder of radius `k` and height
/// `2k`, as a cubic.
fn straight(k: f64) -> Profile {
    [
        (k, 0.0),
        (k, 2.0 * k / 3.0),
        (k, 4.0 * k / 3.0),
        (k, 2.0 * k),
    ]
}

/// Walls of revolution on a surface that closes around the axis, in
/// either parameter, with a seam edge or between two circles: a NURBS
/// cylinder (`π r² h`) and the vase (Pappus: `π ∫ ρ² dz`, which the
/// profile's Bézier form gives exactly), at three scales.
#[test]
fn walls_that_close_are_measured_at_three_scales() {
    for k in [1e-3, 1.0, 1e6] {
        let cylinder = straight(k);
        let (r, h) = (k, 2.0 * k);
        let closed_form = (PI * r * r * h, TAU * r * h + 2.0 * PI * r * r);
        let (volume, area) = exact(&cylinder);
        assert!((volume - closed_form.0).abs() < 1e-12 * closed_form.0);
        assert!((area - closed_form.1).abs() < 1e-12 * closed_form.1);
        for (name, profile, expected) in [
            ("cylinder", cylinder, closed_form),
            ("vase", vase(k), exact(&vase(k))),
        ] {
            for wall in [Wall::Seam, Wall::Circles] {
                for (from, around_v) in [(0.0, false), (2.0, false), (0.0, true), (2.0, true)] {
                    measures_as(
                        &turned(&profile, wall, from, around_v),
                        expected,
                        k,
                        &format!("{name} of size {k}, {wall:?} from {from}, around v: {around_v}"),
                    );
                }
            }
        }
    }
}

/// A NURBS torus, closed both ways: without edges, with its two seams as
/// edges walked there and back (a STEP file's whole torus), and cut in two
/// by a window across both seams (the window, and the torus around it as
/// a face of its own, whose loop is the window's backwards: the boundary
/// is unrolled as on an analytic torus). At three scales.
#[test]
fn a_nurbs_torus_is_measured_at_three_scales() {
    for k in [1e-3, 1.0, 1e6] {
        let (major, minor) = (3.0 * k, k);
        let whole = (
            2.0 * PI * PI * major * minor * minor,
            4.0 * PI * PI * major * minor,
        );
        for step in [false, true] {
            measures_as(
                &nurbs_torus(major, minor, step),
                whole,
                k,
                &format!("torus of size {k}, seams as edges: {step}"),
            );
        }
        // Around the axis from φ0 to φ1, around the tube from θ0 to θ1:
        // both ranges go over the seams at 0.
        for ((f0, f1), (t0, t1)) in [((-0.5, 0.6), (-0.4, 0.7)), ((2.0, 2.5), (-1.0, 0.3))] {
            let pane = minor * (f1 - f0) * (major * (t1 - t0) + minor * (t1.sin() - t0.sin()));
            let window = |body: &mut Body, hole: bool| -> LoopId {
                let at = |f: f64, t: f64| {
                    let rho = major + minor * t.cos();
                    Vec3::new(rho * f.cos(), rho * f.sin(), minor * t.sin())
                };
                let corners = [
                    body.add_vertex(at(f0, t0)),
                    body.add_vertex(at(f1, t0)),
                    body.add_vertex(at(f1, t1)),
                    body.add_vertex(at(f0, t1)),
                ];
                // Around the axis at tube angle `t`, from φ0.
                let around = |t: f64| {
                    let x_axis = Vec3::new(f0.cos(), f0.sin(), 0.0);
                    let frame =
                        Frame::new(Vec3::new(0.0, 0.0, minor * t.sin()), Vec3::Z, x_axis).unwrap();
                    Curve3::Circle(Circle::new(frame, major + minor * t.cos()).unwrap())
                };
                // Around the tube at axis angle `f`, from its outer equator.
                let tube = |f: f64| {
                    let radial = Vec3::new(f.cos(), f.sin(), 0.0);
                    let frame = Frame::new(radial * major, radial.cross(Vec3::Z), radial).unwrap();
                    Curve3::Circle(Circle::new(frame, minor).unwrap())
                };
                let below = body.add_edge(around(t0), (0.0, f1 - f0), corners[0], corners[1]);
                let right = body.add_edge(tube(f1), (t0, t1), corners[1], corners[2]);
                let above = body.add_edge(around(t1), (0.0, f1 - f0), corners[3], corners[2]);
                let left = body.add_edge(tube(f0), (t0, t1), corners[0], corners[3]);
                let use_of = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
                body.add_loop(if hole {
                    vec![
                        use_of(left, false),
                        use_of(above, false),
                        use_of(right, true),
                        use_of(below, true),
                    ]
                } else {
                    vec![
                        use_of(below, false),
                        use_of(right, false),
                        use_of(above, true),
                        use_of(left, true),
                    ]
                })
            };
            let what = format!("torus of size {k}, window {f0}..{f1} by {t0}..{t1}");
            let alone = sheet(torus_surface(major, minor), false, |body| {
                vec![window(body, false)]
            });
            measures_as(&alone, (0.0, pane), k, &format!("{what}: the window"));
            let around = sheet(torus_surface(major, minor), false, |body| {
                vec![window(body, true)]
            });
            measures_as(
                &around,
                (0.0, whole.1 - pane),
                k,
                &format!("{what}: the torus around it"),
            );
            // Both faces on the same edges: the whole torus as a solid.
            let mut body = Body::new(BodyKind::Solid);
            let inside = window(&mut body, false);
            let edges: Vec<Coedge> = body.loops[inside.0 as usize]
                .coedges
                .iter()
                .rev()
                .map(|c| Coedge {
                    edge: c.edge,
                    reversed: !c.reversed,
                })
                .collect();
            let outside = body.add_loop(edges);
            let a = body.add_face(torus_surface(major, minor), false, vec![inside]);
            let b = body.add_face(torus_surface(major, minor), false, vec![outside]);
            body.add_shell(vec![a, b]);
            // (The tessellator declines the face around the window: see
            // `what_the_tessellator_declines_on_closed_freeform`.)
            measured_as(&body, whole, &format!("{what}: both faces"));
        }
    }
}

/// Closed around the axis and to a point at both ends (the egg), in
/// either parameter, without edges and with its seam from pole to pole as
/// an edge walked there and back. At three scales.
#[test]
fn an_egg_is_measured_at_three_scales() {
    for k in [1e-3, 1.0, 1e6] {
        let profile = egg(k);
        let expected = (
            PI * along(&profile, |(rho, _), (_, dz)| rho * rho * dz),
            TAU * along(&profile, |(rho, _), (dr, dz)| rho * dr.hypot(dz)),
        );
        for around_v in [false, true] {
            for (from, seam) in [(0.0, false), (1.1, false), (0.0, true), (1.1, true)] {
                let mut body = Body::new(BodyKind::Solid);
                let surface = Surface::Nurbs(revolved(&profile, from, around_v));
                let loops = if seam {
                    let (low, high) = (
                        body.add_vertex(Vec3::ZERO),
                        body.add_vertex(Vec3::new(0.0, 0.0, profile[3].1)),
                    );
                    let edge = body.add_edge(profile_curve(&profile, from), (0.0, 1.0), low, high);
                    let use_of = |reversed: bool| Coedge { edge, reversed };
                    vec![body.add_loop(vec![use_of(false), use_of(true)])]
                } else {
                    Vec::new()
                };
                let face = body.add_face(surface, around_v, loops);
                body.add_shell(vec![face]);
                let what =
                    format!("egg of size {k} from {from}, seam: {seam}, around v: {around_v}");
                // (The tessellator declines a face without loops on a
                // surface that closes in its second parameter: see
                // `what_the_tessellator_declines_on_closed_freeform`.)
                if around_v && !seam {
                    measured_as(&body, expected, &what);
                } else {
                    measures_as(&body, expected, k, &what);
                }
            }
        }
    }
}

/// A window across the seam of a surface that closes in v, and the wall
/// around it: their areas, which the boundary integral against `dv` gives
/// (sheets: no volume). At three scales.
#[test]
fn a_window_across_a_seam_in_v_is_measured() {
    for k in [1e-3, 1.0, 1e6] {
        let profile = vase(k);
        let (t0, t1) = (0.3, 0.7);
        for (a0, a1) in [(-0.5, 0.6), (0.3, 1.4)] {
            let side = |range: (f64, f64)| {
                along_part(&profile, range, |(rho, _), (dr, dz)| rho * dr.hypot(dz))
            };
            let strip = TAU * side((0.0, 1.0));
            let pane = (a1 - a0) * side((t0, t1));
            let window = |body: &mut Body, hole: bool| -> LoopId {
                let ((low, _), (high, _)) = (bernstein(&profile, t0), bernstein(&profile, t1));
                let corner =
                    |(rho, z): (f64, f64), a: f64| Vec3::new(rho * a.cos(), rho * a.sin(), z);
                let corners = [
                    body.add_vertex(corner(low, a0)),
                    body.add_vertex(corner(low, a1)),
                    body.add_vertex(corner(high, a1)),
                    body.add_vertex(corner(high, a0)),
                ];
                let arc = |(rho, z): (f64, f64)| {
                    let x_axis = Vec3::new(a0.cos(), a0.sin(), 0.0);
                    let frame = Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, x_axis).unwrap();
                    Curve3::Circle(Circle::new(frame, rho).unwrap())
                };
                let below = body.add_edge(arc(low), (0.0, a1 - a0), corners[0], corners[1]);
                let above = body.add_edge(arc(high), (0.0, a1 - a0), corners[3], corners[2]);
                let right = body.add_edge(
                    profile_curve(&profile, a1),
                    (t0, t1),
                    corners[1],
                    corners[2],
                );
                let left = body.add_edge(
                    profile_curve(&profile, a0),
                    (t0, t1),
                    corners[0],
                    corners[3],
                );
                let use_of = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
                body.add_loop(if hole {
                    vec![
                        use_of(left, false),
                        use_of(above, false),
                        use_of(right, true),
                        use_of(below, true),
                    ]
                } else {
                    vec![
                        use_of(below, false),
                        use_of(right, false),
                        use_of(above, true),
                        use_of(left, true),
                    ]
                })
            };
            for around_v in [false, true] {
                let surface = || Surface::Nurbs(revolved(&profile, 0.0, around_v));
                let what = format!("size {k}, between {a0} and {a1}, around v: {around_v}");
                let alone = sheet(surface(), around_v, |body| vec![window(body, false)]);
                measures_as(&alone, (0.0, pane), k, &format!("a window, {what}"));
                let around = sheet(surface(), around_v, |body| {
                    let rim = |body: &mut Body, (rho, z): (f64, f64)| -> EdgeId {
                        let vertex = body.add_vertex(Vec3::new(rho, 0.0, z));
                        let frame = Frame::new(Vec3::new(0.0, 0.0, z), Vec3::Z, Vec3::X).unwrap();
                        let circle = Curve3::Circle(Circle::new(frame, rho).unwrap());
                        body.add_edge(circle, (0.0, TAU), vertex, vertex)
                    };
                    let lower = rim(body, profile[0]);
                    let upper = rim(body, profile[3]);
                    let use_of = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
                    vec![
                        body.add_loop(vec![use_of(lower, false)]),
                        body.add_loop(vec![use_of(upper, true)]),
                        window(body, true),
                    ]
                });
                measures_as(
                    &around,
                    (0.0, strip - pane),
                    k,
                    &format!("the wall around, {what}"),
                );
            }
        }
    }
}

/// Not the exact measure's, which takes both (the tests above): two faces
/// of closed freeform surfaces that the tessellator declines with an error
/// (not a wrong mesh), found on 2026-10-14. The egg turned about the axis
/// in its surface's second parameter, as a face without loops ("a face
/// without loops on a surface that is not closed"); and a NURBS torus
/// around a window across both of its seams, the window's loop backwards
/// ("its loops do not bound a region").
#[test]
#[ignore = "craft3d-tess: declines an egg without loops that closes in v, and a NURBS torus around \
            a window across both seams"]
fn what_the_tessellator_declines_on_closed_freeform() {
    let profile = egg(1.0);
    let mut body = Body::new(BodyKind::Solid);
    let face = body.add_face(
        Surface::Nurbs(revolved(&profile, 0.0, true)),
        true,
        Vec::new(),
    );
    body.add_shell(vec![face]);
    let expected = (
        PI * along(&profile, |(rho, _), (_, dz)| rho * rho * dz),
        TAU * along(&profile, |(rho, _), (dr, dz)| rho * dr.hypot(dz)),
    );
    measures_as(&body, expected, 1.0, "egg without loops, around v");

    let (major, minor) = (3.0, 1.0);
    let ((f0, f1), (t0, t1)) = ((-0.5, 0.6), (-0.4, 0.7));
    let at = |f: f64, t: f64| {
        let rho = major + minor * t.cos();
        Vec3::new(rho * f.cos(), rho * f.sin(), minor * t.sin())
    };
    let mut body = Body::new(BodyKind::Solid);
    let corners = [
        body.add_vertex(at(f0, t0)),
        body.add_vertex(at(f1, t0)),
        body.add_vertex(at(f1, t1)),
        body.add_vertex(at(f0, t1)),
    ];
    let around = |t: f64| {
        let x_axis = Vec3::new(f0.cos(), f0.sin(), 0.0);
        let frame = Frame::new(Vec3::new(0.0, 0.0, minor * t.sin()), Vec3::Z, x_axis).unwrap();
        Curve3::Circle(Circle::new(frame, major + minor * t.cos()).unwrap())
    };
    let tube = |f: f64| {
        let radial = Vec3::new(f.cos(), f.sin(), 0.0);
        let frame = Frame::new(radial * major, radial.cross(Vec3::Z), radial).unwrap();
        Curve3::Circle(Circle::new(frame, minor).unwrap())
    };
    let below = body.add_edge(around(t0), (0.0, f1 - f0), corners[0], corners[1]);
    let right = body.add_edge(tube(f1), (t0, t1), corners[1], corners[2]);
    let above = body.add_edge(around(t1), (0.0, f1 - f0), corners[3], corners[2]);
    let left = body.add_edge(tube(f0), (t0, t1), corners[0], corners[3]);
    let use_of = |edge: EdgeId, reversed: bool| Coedge { edge, reversed };
    let inside = body.add_loop(vec![
        use_of(below, false),
        use_of(right, false),
        use_of(above, true),
        use_of(left, true),
    ]);
    let outside = body.add_loop(vec![
        use_of(left, false),
        use_of(above, false),
        use_of(right, true),
        use_of(below, true),
    ]);
    let a = body.add_face(torus_surface(major, minor), false, vec![inside]);
    let b = body.add_face(torus_surface(major, minor), false, vec![outside]);
    body.add_shell(vec![a, b]);
    let whole = (
        2.0 * PI * PI * major * minor * minor,
        4.0 * PI * PI * major * minor,
    );
    measures_as(&body, whole, 1.0, "torus around a window across both seams");
}

/// At a pole where the profile meets the axis at right angles the surface
/// is as smooth as anywhere and its normal is the axis: every vertex of
/// the mesh there carries that normal, not the rounding of `∂u × ∂v` of
/// a parametrisation that stalls (the starburst a revolved spline dome
/// was shaded with at its pole, lamp T1, 2026-10-16). The vertices around
/// them keep the surface's own normal, so the shading is smooth.
#[test]
fn the_normal_at_a_smooth_pole_is_the_axis() {
    // Within this angle of the exact normal (radians). The limit is taken
    // a millionth of the range off the pole: far closer than this.
    let within = 1e-4_f64;
    for k in [1.0, 1e-3, 1e3] {
        let profile = egg(k);
        let top = profile[3].1;
        for (from, seam) in [(0.0, false), (1.1, false), (0.0, true), (1.1, true)] {
            let mut body = Body::new(BodyKind::Solid);
            let surface = Surface::Nurbs(revolved(&profile, from, false));
            let loops = if seam {
                let (low, high) = (
                    body.add_vertex(Vec3::ZERO),
                    body.add_vertex(Vec3::new(0.0, 0.0, top)),
                );
                let edge = body.add_edge(profile_curve(&profile, from), (0.0, 1.0), low, high);
                let use_of = |reversed: bool| Coedge { edge, reversed };
                vec![body.add_loop(vec![use_of(false), use_of(true)])]
            } else {
                Vec::new()
            };
            let face = body.add_face(surface, false, loops);
            body.add_shell(vec![face]);
            let what = format!("egg of size {k} from {from}, seam as an edge: {seam}");
            for relative in [0.1, 1e-2, 1e-3] {
                let mesh = tessellate_body(&body, &params(relative * k))
                    .unwrap_or_else(|e| panic!("{what} at {relative}: {e}"));
                let mut at_poles = [0, 0];
                for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
                    let (pole, exact) = if p.length() <= 1e-9 * k {
                        (0, -Vec3::Z)
                    } else if p.distance(Vec3::new(0.0, 0.0, top)) <= 1e-9 * k {
                        (1, Vec3::Z)
                    } else {
                        continue;
                    };
                    at_poles[pole] += 1;
                    let angle = n.cross(exact).length().atan2(n.dot(exact));
                    assert!(
                        (n.length() - 1.0).abs() < 1e-9 && angle <= within,
                        "{what} at {relative}: normal {n:?} at the pole {p:?}, {angle} rad off"
                    );
                }
                assert!(at_poles.iter().all(|&n| n > 0), "{what}: {at_poles:?}");
            }
        }
    }
}
