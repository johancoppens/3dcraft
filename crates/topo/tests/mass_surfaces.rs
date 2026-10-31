//! Mass properties of bodies whose faces lie on spline surfaces.
//!
//! A spline surface is smooth only between its knot lines, so both the
//! integral across a face and the integral along its boundary are summed
//! piece by piece between them. The surfaces here are exact (cylinders,
//! spheres and tori as rational splines swept from rational circles), so
//! every body has a closed form, and the same body is built three ways
//! (an analytic surface, one spline, one spline cut into Bézier pieces)
//! that must agree.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Cylinder, Line, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{
    Body, BodyKind, Coedge, EdgeId, FaceId, MassProperties, check, mass_properties,
};
use std::f64::consts::{PI, TAU};

fn fwd(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: false,
    }
}

fn back(edge: EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: true,
    }
}

fn flat(body: &mut Body, origin: Vec3, normal: Vec3, coedges: Vec<Coedge>) -> FaceId {
    let loop_ = body.add_loop(coedges);
    let frame = Frame::from_z(origin, normal).unwrap();
    body.add_face(Surface::Plane(Plane::new(frame)), false, vec![loop_])
}

/// The profile of a surface of revolution in the half-plane (ρ, z) of a
/// frame: control points and weights of a spline of `degree` on `knots`.
struct Profile {
    degree: usize,
    knots: Vec<f64>,
    points: Vec<(f64, f64)>,
    weights: Vec<f64>,
}

/// The surface `profile` sweeps in a whole turn about the frame's z axis:
/// u around the axis as the nine-point rational circle goes (*The NURBS
/// Book* §8.5: each profile point sweeps a circle, with the weights
/// multiplied), v along the profile. `pieces_u` and `pieces_v` say where
/// every knot is given its full multiplicity, which changes nothing about
/// the surface and makes it a row of Bézier patches.
fn revolved(frame: &Frame, profile: &Profile) -> NurbsSurface {
    let unit = NurbsCurve::circle(&Frame::WORLD, 1.0).unwrap();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (around, &w_u) in unit.control_points().iter().zip(unit.weights()) {
        for (&(rho, z), &w_v) in profile.points.iter().zip(&profile.weights) {
            ctrl.push(frame.point(around.x * rho, around.y * rho, z));
            weights.push(w_u * w_v);
        }
    }
    NurbsSurface::new(
        2,
        profile.degree,
        unit.knots().to_vec(),
        profile.knots.clone(),
        9,
        profile.points.len(),
        ctrl,
        weights,
    )
    .unwrap()
}

/// A straight profile from (r, 0) to (r, h): the wall of a cylinder.
fn upright(r: f64, h: f64) -> Profile {
    Profile {
        degree: 1,
        knots: vec![0.0, 0.0, 1.0, 1.0],
        points: vec![(r, 0.0), (r, h)],
        weights: vec![1.0, 1.0],
    }
}

/// The same wall in three stretches of uneven length: the surface has
/// knot lines around it, creases as far as its parameters go.
fn upright_in_three(r: f64, h: f64) -> Profile {
    Profile {
        degree: 1,
        knots: vec![0.0, 0.0, 0.2, 0.9, 1.0, 1.0],
        points: vec![(r, 0.0), (r, 0.37 * h), (r, 0.52 * h), (r, h)],
        weights: vec![1.0; 4],
    }
}

/// How the wall of a can is described.
#[derive(Clone, Copy, Debug)]
enum Wall {
    Analytic,
    Spline,
    SplineInThree,
}

/// A cylinder of radius `r` and height `h` on `frame`. With a spline wall
/// its rims are the rational circles the wall was swept from and the wall
/// has a seam, used twice by its one loop; `turn` then turns the rims'
/// circles about the axis, so that they start somewhere other than on the
/// seam and cross the wall's knot lines away from their own knots.
/// V = πr²h, A = 2πr (r + h).
fn can(frame: &Frame, r: f64, h: f64, wall: Wall, turn: f64) -> Body {
    let top_frame = frame.at(frame.point(0.0, 0.0, h)).unwrap();
    let mut body = Body::new(BodyKind::Solid);
    let low = body.add_vertex(frame.point(r, 0.0, 0.0));
    let high = body.add_vertex(frame.point(r, 0.0, h));
    let profile = match wall {
        Wall::Analytic => {
            let bottom = body.add_edge(
                Curve3::Circle(Circle::new(*frame, r).unwrap()),
                (0.0, TAU),
                low,
                low,
            );
            let top = body.add_edge(
                Curve3::Circle(Circle::new(top_frame, r).unwrap()),
                (0.0, TAU),
                high,
                high,
            );
            let loops = vec![
                body.add_loop(vec![fwd(bottom)]),
                body.add_loop(vec![back(top)]),
            ];
            let wall = body.add_face(
                Surface::Cylinder(Cylinder::new(*frame, r).unwrap()),
                false,
                loops,
            );
            let floor = flat(&mut body, frame.origin(), -frame.z(), vec![back(bottom)]);
            let lid = flat(&mut body, top_frame.origin(), frame.z(), vec![fwd(top)]);
            body.add_shell(vec![wall, floor, lid]);
            return body;
        }
        Wall::Spline => upright(r, h),
        Wall::SplineInThree => upright_in_three(r, h),
    };
    let surface = revolved(frame, &profile);
    // The rims: the same circles, started `turn` round from the seam, as
    // two edges each so that the seam's ends are vertices.
    let turned = |at: &Frame| {
        let x = at.vector(turn.pcos(), turn.psin(), 0.0);
        NurbsCurve::circle(&Frame::new(at.origin(), at.z(), x).unwrap(), r).unwrap()
    };
    let (bottom_curve, top_curve) = (turned(frame), turned(&top_frame));
    // Where each rim passes the seam: the parameter of the seam's end.
    let split = |curve: &NurbsCurve, point: Vec3| {
        if turn == 0.0 {
            return None;
        }
        let t = Curve3::Nurbs(curve.clone()).project(point).unwrap();
        assert!(
            curve.eval(t).distance(point) <= 1e-12 * r,
            "seam off the rim"
        );
        Some(t)
    };
    let mut rim = |curve: NurbsCurve, vertex, point: Vec3| -> Vec<EdgeId> {
        match split(&curve, point) {
            None => vec![body.add_edge(Curve3::Nurbs(curve), (0.0, 1.0), vertex, vertex)],
            Some(t) => {
                let start = body.add_vertex(curve.eval(0.0));
                vec![
                    body.add_edge(Curve3::Nurbs(curve.clone()), (t, 1.0), vertex, start),
                    body.add_edge(Curve3::Nurbs(curve), (0.0, t), start, vertex),
                ]
            }
        }
    };
    let bottom = rim(bottom_curve, low, frame.point(r, 0.0, 0.0));
    let top = rim(top_curve, high, frame.point(r, 0.0, h));
    let seam = body.add_edge(
        Curve3::Line(Line::new(frame.point(r, 0.0, 0.0), frame.z()).unwrap()),
        (0.0, h),
        low,
        high,
    );
    let all = |edges: &[EdgeId], reversed: bool| -> Vec<Coedge> {
        let mut uses: Vec<Coedge> = edges
            .iter()
            .map(|&edge| Coedge { edge, reversed })
            .collect();
        if reversed {
            uses.reverse();
        }
        uses
    };
    let mut around = all(&bottom, false);
    around.push(fwd(seam));
    around.extend(all(&top, true));
    around.push(back(seam));
    let wall_loop = body.add_loop(around);
    let wall = body.add_face(Surface::Nurbs(surface), false, vec![wall_loop]);
    let floor = flat(&mut body, frame.origin(), -frame.z(), all(&bottom, true));
    let lid = flat(&mut body, top_frame.origin(), frame.z(), all(&top, false));
    body.add_shell(vec![wall, floor, lid]);
    body
}

fn frames(k: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * k,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap(),
    ]
}

fn measured(body: &Body, what: &str, k: f64) -> MassProperties {
    let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
    assert_eq!(check(body, &tolerance), vec![], "{what}");
    mass_properties(body).unwrap_or_else(|e| panic!("{what}: {e}"))
}

/// Cans of several sizes, their wall an analytic cylinder, one rational
/// spline, and a spline with knot lines around it; the rims starting on
/// the seam and turned away from it. Each is πr²h and 2πr (r + h) to
/// 1e-11, and the three descriptions of one can agree to 1e-12.
#[test]
fn a_can_is_a_can_however_its_wall_is_described() {
    let mut worst = (0.0_f64, 0.0_f64);
    for k in [1e-3, 1.0, 1e6] {
        for frame in frames(k) {
            for (r, h) in [(1.5, 2.0), (0.3, 5.0), (4.0, 0.25)] {
                let (r, h) = (r * k, h * k);
                let (volume, area) = (PI * r * r * h, TAU * r * (r + h));
                let reference = measured(&can(&frame, r, h, Wall::Analytic, 0.0), "analytic", k);
                for wall in [Wall::Analytic, Wall::Spline, Wall::SplineInThree] {
                    for turn in [0.0, 0.4, 2.0, 5.5] {
                        let what = format!("can {r} x {h}, {wall:?}, turned {turn}, scale {k}");
                        let mass = measured(&can(&frame, r, h, wall, turn), &what, k);
                        let off = (
                            (mass.volume - volume).abs() / volume,
                            (mass.area - area).abs() / area,
                        );
                        assert!(
                            off.0 <= 1e-11 && off.1 <= 1e-11,
                            "{what}: volume {} vs {volume} ({:e}), area {} vs {area} ({:e})",
                            mass.volume,
                            off.0,
                            mass.area,
                            off.1
                        );
                        assert!(
                            (mass.volume - reference.volume).abs() <= 1e-12 * volume
                                && (mass.area - reference.area).abs() <= 1e-12 * area,
                            "{what}: {mass:?} vs the analytic can's {reference:?}"
                        );
                        worst = (worst.0.max(off.0), worst.1.max(off.1));
                    }
                }
            }
        }
    }
    println!(
        "worst relative error: volume {:e}, area {:e}",
        worst.0, worst.1
    );
}

/// Half a circle of radius `r` about (0, 0) in the half-plane, from the
/// south pole to the north pole, as two rational quadratic arcs: the
/// profile of a sphere.
fn meridian(r: f64) -> Profile {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    Profile {
        degree: 2,
        knots: vec![0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
        points: vec![(0.0, -r), (r, -r), (r, 0.0), (r, r), (0.0, r)],
        weights: vec![1.0, s, 1.0, s, 1.0],
    }
}

/// A whole circle of radius `minor` about (`major`, 0) in the half-plane,
/// as four rational quadratic arcs: the profile of a torus.
fn tube(major: f64, minor: f64) -> Profile {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let square = [
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (-1.0, 1.0),
        (-1.0, 0.0),
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (1.0, 0.0),
    ];
    Profile {
        degree: 2,
        knots: vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ],
        points: square
            .iter()
            .map(|&(x, z)| (major + minor * x, minor * z))
            .collect(),
        weights: vec![1.0, s, 1.0, s, 1.0, s, 1.0, s, 1.0],
    }
}

/// A body of one face without loops: a whole closed surface.
fn whole(surface: NurbsSurface) -> Body {
    let mut body = Body::new(BodyKind::Solid);
    let face = body.add_face(Surface::Nurbs(surface), false, Vec::new());
    body.add_shell(vec![face]);
    body
}

/// A sphere and a torus as rational splines, each one face without
/// loops: 4πr³/3 and 4πr², 2π²Rr² and 4π²Rr.
#[test]
fn a_spline_sphere_and_a_spline_torus_match_their_closed_forms() {
    for k in [1e-3, 1.0, 1e6] {
        for frame in frames(k) {
            let r = 1.7 * k;
            let (major, minor) = (3.0 * k, 0.8 * k);
            for (name, body, volume, area) in [
                (
                    "sphere",
                    whole(revolved(&frame, &meridian(r))),
                    4.0 * PI * r.ppowi(3) / 3.0,
                    4.0 * PI * r * r,
                ),
                (
                    "torus",
                    whole(revolved(&frame, &tube(major, minor))),
                    2.0 * PI * PI * major * minor * minor,
                    4.0 * PI * PI * major * minor,
                ),
            ] {
                let what = format!("spline {name} at scale {k}");
                let mass = mass_properties(&body).unwrap_or_else(|e| panic!("{what}: {e}"));
                assert!(
                    (mass.volume - volume).abs() <= 1e-11 * volume,
                    "{what}: volume {} vs {volume}",
                    mass.volume
                );
                assert!(
                    (mass.area - area).abs() <= 1e-11 * area,
                    "{what}: area {} vs {area}",
                    mass.area
                );
            }
        }
    }
}

/// A whole closed surface (one face without loops: a sphere, a torus, a
/// spline sphere or torus) is a closed shell, which a sheet cannot be:
/// marked as a sheet it is `SheetClosed`, though it has no edge to tell
/// it by. A face without loops on a surface without an end (a plane, a
/// cylinder) is unbounded, not closed: no `SheetClosed` (the exporters
/// and the measure refuse it).
#[test]
fn a_whole_closed_surface_marked_as_a_sheet_is_closed() {
    use craft3d_geom::{Sphere, Torus};
    use craft3d_topo::{Defect, ShellId};
    let as_sheet = |surface: Surface| {
        let mut body = Body::new(BodyKind::Sheet);
        let face = body.add_face(surface, false, Vec::new());
        body.add_shell(vec![face]);
        body
    };
    for k in [1e-3, 1.0, 1e6] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for frame in frames(k) {
            for (name, surface) in [
                (
                    "sphere",
                    Surface::Sphere(Sphere::new(frame, 1.7 * k).unwrap()),
                ),
                (
                    "torus",
                    Surface::Torus(Torus::new(frame, 3.0 * k, 0.8 * k).unwrap()),
                ),
                (
                    "spline sphere",
                    Surface::Nurbs(revolved(&frame, &meridian(1.7 * k))),
                ),
                (
                    "spline torus",
                    Surface::Nurbs(revolved(&frame, &tube(3.0 * k, 0.8 * k))),
                ),
            ] {
                assert_eq!(
                    check(&as_sheet(surface), &tol),
                    [Defect::SheetClosed { shell: ShellId(0) }],
                    "{name} at {k}"
                );
            }
            for (name, surface) in [
                ("plane", Surface::Plane(Plane::new(frame))),
                (
                    "cylinder",
                    Surface::Cylinder(Cylinder::new(frame, 1.7 * k).unwrap()),
                ),
            ] {
                assert!(
                    !check(&as_sheet(surface), &tol)
                        .iter()
                        .any(|d| matches!(d, Defect::SheetClosed { .. })),
                    "{name} at {k}"
                );
            }
        }
    }
}

/// The slice of a spline ball between the planes z = `low` and `high`,
/// or, where one of them is `None`, everything below or above the other:
/// a cap, whose face holds a pole of the surface and has one rim, which
/// goes around the surface without coming back.
///
/// V = π ∫ (r² − z²) dz and the curved area is 2πr times the height
/// (Archimedes). The rims are spline circles started `turn` round from
/// the surface's seam.
fn ball_slice(
    frame: &Frame,
    r: f64,
    low: Option<f64>,
    high: Option<f64>,
    turn: f64,
) -> (Body, f64, f64) {
    let mut body = Body::new(BodyKind::Solid);
    let mut rim = |z: f64| {
        let radius = (r * r - z * z).sqrt();
        let x = frame.vector(turn.pcos(), turn.psin(), 0.0);
        let placed = Frame::new(frame.point(0.0, 0.0, z), frame.z(), x).unwrap();
        let curve = NurbsCurve::circle(&placed, radius).unwrap();
        let vertex = body.add_vertex(curve.eval(0.0));
        body.add_edge(Curve3::Nurbs(curve), (0.0, 1.0), vertex, vertex)
    };
    let (bottom, top) = (low.map(&mut rim), high.map(&mut rim));
    let mut loops = Vec::new();
    let mut faces = Vec::new();
    if let Some(bottom) = bottom {
        loops.push(body.add_loop(vec![fwd(bottom)]));
    }
    if let Some(top) = top {
        loops.push(body.add_loop(vec![back(top)]));
    }
    let surface = revolved(frame, &meridian(r));
    faces.push(body.add_face(Surface::Nurbs(surface), false, loops));
    if let (Some(bottom), Some(z)) = (bottom, low) {
        faces.push(flat(
            &mut body,
            frame.point(0.0, 0.0, z),
            -frame.z(),
            vec![back(bottom)],
        ));
    }
    if let (Some(top), Some(z)) = (top, high) {
        faces.push(flat(
            &mut body,
            frame.point(0.0, 0.0, z),
            frame.z(),
            vec![fwd(top)],
        ));
    }
    body.add_shell(faces);
    let (z0, z1) = (low.unwrap_or(-r), high.unwrap_or(r));
    let slab = |z: f64| r * r * z - z.ppowi(3) / 3.0;
    let disc = |z: Option<f64>| z.map_or(0.0, |z| PI * (r * r - z * z));
    (
        body,
        PI * (slab(z1) - slab(z0)),
        TAU * r * (z1 - z0) + disc(low) + disc(high),
    )
}

/// Zones and caps of a spline ball: rims that cross the surface's knot
/// lines anywhere, faces that lie across its equator (a knot line), and
/// faces that hold a pole.
#[test]
fn slices_of_a_spline_ball_match_their_closed_forms() {
    let mut worst = (0.0_f64, 0.0_f64);
    for k in [1e-3, 1.0, 1e6] {
        for frame in frames(k) {
            let r = 2.0 * k;
            for (low, high) in [
                (Some(-0.5), Some(1.2)),
                (Some(0.3), Some(1.9)),
                (Some(-1.7), Some(-0.2)),
                (Some(0.7), None),
                (Some(-1.1), None),
                (None, Some(0.4)),
                (None, Some(-1.5)),
            ] {
                for turn in [0.0, 0.7, 3.3] {
                    let (body, volume, area) =
                        ball_slice(&frame, r, low.map(|z| z * k), high.map(|z| z * k), turn);
                    let what = format!("slice {low:?}..{high:?}, turned {turn}, scale {k}");
                    let mass = measured(&body, &what, k);
                    let off = (
                        (mass.volume - volume).abs() / volume,
                        (mass.area - area).abs() / area,
                    );
                    assert!(
                        off.0 <= 1e-11 && off.1 <= 1e-11,
                        "{what}: volume {} vs {volume} ({:e}), area {} vs {area} ({:e})",
                        mass.volume,
                        off.0,
                        mass.area,
                        off.1
                    );
                    worst = (worst.0.max(off.0), worst.1.max(off.1));
                }
            }
        }
    }
    println!(
        "worst relative error: volume {:e}, area {:e}",
        worst.0, worst.1
    );
}

/// What still cannot be measured is refused: the wall of a can with the
/// top rim and no bottom one reaches to an end of the surface that is a
/// circle, not a point.
#[test]
fn a_face_that_runs_off_the_end_of_a_spline_surface_is_refused() {
    use craft3d_topo::MassError;
    let frame = frames(1.0)[1];
    let mut body = Body::new(BodyKind::Sheet);
    let curve = NurbsCurve::circle(&frame.at(frame.point(0.0, 0.0, 1.0)).unwrap(), 1.5).unwrap();
    let vertex = body.add_vertex(curve.eval(0.0));
    let rim = body.add_edge(Curve3::Nurbs(curve), (0.0, 1.0), vertex, vertex);
    let loop_ = body.add_loop(vec![fwd(rim)]);
    let surface = revolved(&frame, &upright(1.5, 2.0));
    let face = body.add_face(Surface::Nurbs(surface), false, vec![loop_]);
    body.add_shell(vec![face]);
    assert_eq!(
        mass_properties(&body),
        Err(MassError::UnboundedFace { face: 0 })
    );
}

/// A sheet body with a bicubic patch as its one face, bounded by its four
/// edges.
fn patch_sheet(patch: &craft3d_geom::BezierPatch) -> Body {
    let mut body = Body::new(BodyKind::Sheet);
    let corner = |i, j| patch.control_point(i, j).unwrap();
    let v: Vec<_> = [(0, 0), (3, 0), (3, 3), (0, 3)]
        .iter()
        .map(|&(i, j)| body.add_vertex(corner(i, j)))
        .collect();
    let cubic = |points: Vec<Vec3>| {
        Curve3::Nurbs(
            NurbsCurve::polynomial(3, vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], points)
                .unwrap(),
        )
    };
    let along_u = |j: usize| cubic((0..4).map(|i| corner(i, j)).collect());
    let along_v = |i: usize| cubic((0..4).map(|j| corner(i, j)).collect());
    let bottom = body.add_edge(along_u(0), (0.0, 1.0), v[0], v[1]);
    let right = body.add_edge(along_v(3), (0.0, 1.0), v[1], v[2]);
    let top = body.add_edge(along_u(3), (0.0, 1.0), v[3], v[2]);
    let left = body.add_edge(along_v(0), (0.0, 1.0), v[0], v[3]);
    let loop_ = body.add_loop(vec![fwd(bottom), fwd(right), back(top), back(left)]);
    let face = body.add_face(Surface::Bezier(patch.clone()), false, vec![loop_]);
    body.add_shell(vec![face]);
    body
}

/// The area of every patch of the Utah teapot (all thirty-two: the ones
/// with a collapsed edge are faces whose boundary passes through a pole
/// of their surface), against the integral of |Sᵤ × Sᵥ| over the unit square by
/// the rule in 48 × 48 panels: another route, with nothing of Green's
/// theorem in it. The integral across a patch has no closed form and no
/// panel count that is known to do, so it is doubled along with the
/// boundary's; with the two panels it had, four patches of the body were
/// 5e-11 off.
#[test]
fn the_patches_of_the_teapot_have_the_area_of_their_double_integral() {
    use craft3d_math::quadrature::Rule;
    let rule = Rule::new(16);
    let mut measured = 0;
    let mut worst = 0.0_f64;
    for (i, patch) in craft3d_testkit::teapot::teapot()
        .unwrap()
        .iter()
        .enumerate()
    {
        // Every patch, those with a collapsed edge too (the lid's and the
        // bottom's centres: a boundary through a pole).
        let mass =
            mass_properties(&patch_sheet(patch)).unwrap_or_else(|e| panic!("patch {i}: {e}"));
        let expected = rule.integrate(0.0, 1.0, 48, |u| {
            rule.integrate(0.0, 1.0, 48, |v| {
                let (_, su, sv) = patch.derivatives(u, v);
                su.cross(sv).length()
            })
        });
        let off = (mass.area - expected).abs() / expected;
        assert!(
            off <= 1e-12,
            "patch {i}: {} vs {expected} ({off:e})",
            mass.area
        );
        worst = worst.max(off);
        measured += 1;
    }
    println!("{measured} patches; worst relative error {worst:e}");
    assert_eq!(measured, 32);
}

/// A can cut off by a plane that leans: its wall ends in an ellipse,
/// which crosses the knot lines that run around a wall in three
/// stretches (as well as those that run up it). With the plane through
/// the axis at `mean` and rising `rise` from the axis to the wall:
/// V = πr² · mean, the wall 2πr · mean, the lid πr √(r² + rise²).
fn leaning_can(frame: &Frame, r: f64, mean: f64, rise: f64, wall: Wall) -> (Body, f64, f64) {
    use craft3d_geom::Ellipse;
    let mut body = Body::new(BodyKind::Solid);
    let centre = frame.point(0.0, 0.0, mean);
    let slant = r.phypot(rise);
    let uphill = frame.vector(r / slant, 0.0, rise / slant);
    let normal = frame.vector(-rise / slant, 0.0, r / slant);
    let on_plane = Frame::new(centre, normal, uphill).unwrap();
    let low = body.add_vertex(frame.point(r, 0.0, 0.0));
    let bottom = body.add_edge(
        Curve3::Circle(Circle::new(*frame, r).unwrap()),
        (0.0, TAU),
        low,
        low,
    );
    let high = body.add_vertex(on_plane.point(slant, 0.0, 0.0));
    let top = body.add_edge(
        Curve3::Ellipse(Ellipse::new(on_plane, slant, r).unwrap()),
        (0.0, TAU),
        high,
        high,
    );
    let height = 2.0 * mean;
    let surface = match wall {
        Wall::Analytic => Surface::Cylinder(Cylinder::new(*frame, r).unwrap()),
        Wall::Spline => Surface::Nurbs(revolved(frame, &upright(r, height))),
        Wall::SplineInThree => Surface::Nurbs(revolved(frame, &upright_in_three(r, height))),
    };
    let loops = vec![
        body.add_loop(vec![fwd(bottom)]),
        body.add_loop(vec![back(top)]),
    ];
    let side = body.add_face(surface, false, loops);
    let floor = flat(&mut body, frame.origin(), -frame.z(), vec![back(bottom)]);
    let lid = flat(&mut body, centre, normal, vec![fwd(top)]);
    body.add_shell(vec![side, floor, lid]);
    (
        body,
        PI * r * r * mean,
        TAU * r * mean + PI * r * r + PI * r * slant,
    )
}

#[test]
fn a_can_cut_off_by_a_leaning_plane_is_the_same_however_its_wall_is_described() {
    for k in [1e-3, 1.0, 1e6] {
        for frame in frames(k) {
            for (r, mean, rise) in [(1.5, 2.0, 1.3), (0.6, 1.0, 0.9), (2.0, 3.0, 0.2)] {
                let (r, mean, rise) = (r * k, mean * k, rise * k);
                let mut reference = None;
                for wall in [Wall::Analytic, Wall::Spline, Wall::SplineInThree] {
                    let (body, volume, area) = leaning_can(&frame, r, mean, rise, wall);
                    let what =
                        format!("leaning can {r} x {mean}, rise {rise}, {wall:?}, scale {k}");
                    let mass = measured(&body, &what, k);
                    assert!(
                        (mass.volume - volume).abs() <= 1e-11 * volume
                            && (mass.area - area).abs() <= 1e-11 * area,
                        "{what}: {mass:?} vs volume {volume}, area {area}"
                    );
                    let first = *reference.get_or_insert(mass);
                    assert!(
                        (mass.volume - first.volume).abs() <= 1e-12 * volume
                            && (mass.area - first.area).abs() <= 1e-12 * area,
                        "{what}: {mass:?} vs the analytic can's {first:?}"
                    );
                }
            }
        }
    }
}

/// SplitMix64: seeded, no dependency.
struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// The square of half-side `half` in the xy plane of `frame` as a
/// biquadratic spline surface whose parametrisation is warped: the
/// control points inside are pushed about in the plane, so the surface
/// is still the square, but its parameters are smooth only between its
/// knot lines (two each way, at 0.35 and 0.7, of the given multiplicity:
/// 2 is a crease in the parametrisation). Nothing about such a warp is a
/// product of one in u and one in v, so the integrand along any curve
/// that crosses a knot line has a kink there. `rational` adds uneven
/// weights, which keep the surface in its plane.
fn warped_square(
    frame: &Frame,
    half: f64,
    multiplicity: usize,
    rational: bool,
    random: &mut Random,
) -> NurbsSurface {
    let mut knots = vec![0.0; 3];
    for knot in [0.35, 0.7] {
        knots.extend(std::iter::repeat_n(knot, multiplicity));
    }
    knots.extend([1.0; 3]);
    let count = knots.len() - 3;
    // Where a control point sits when nothing is warped: the mean of its
    // two knots (Greville), which reproduces the plane's own coordinates.
    let home = |i: usize| -half + 2.0 * half * (knots[i + 1] + knots[i + 2]) / 2.0;
    let push = 0.15 * 2.0 * half / count as f64;
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for i in 0..count {
        for j in 0..count {
            let inside = i > 0 && i + 1 < count && j > 0 && j + 1 < count;
            let (dx, dy) = if inside {
                (
                    push * (2.0 * random.next() - 1.0),
                    push * (2.0 * random.next() - 1.0),
                )
            } else {
                (0.0, 0.0)
            };
            ctrl.push(frame.point(home(i) + dx, home(j) + dy, 0.0));
            weights.push(if rational && inside {
                0.6 + random.next()
            } else {
                1.0
            });
        }
    }
    NurbsSurface::new(2, 2, knots.clone(), knots, count, count, ctrl, weights).unwrap()
}

/// A can whose lid lies on a warped square: the rim, a plain circle,
/// crosses every knot line of the lid's surface, each at a slant and
/// nowhere near a quarter of the way round. The can does not know:
/// V = πr²h, A = 2πr (r + h). And the square itself, as a face without
/// loops, is (2 half)².
#[test]
fn a_face_on_a_warped_plane_has_the_area_of_the_plane() {
    let mut random = Random(0x51A9_E0C3);
    let mut worst = (0.0_f64, 0.0_f64);
    for k in [1e-3, 1.0, 1e6] {
        for frame in frames(k) {
            for multiplicity in [1, 2] {
                for rational in [false, true] {
                    let (r, h, half) = (1.1 * k, 0.8 * k, 2.0 * k);
                    let what =
                        format!("multiplicity {multiplicity}, rational: {rational}, scale {k}");
                    // The square is centred off the can's axis.
                    let top_frame = frame.at(frame.point(0.0, 0.0, h)).unwrap();
                    let square_frame = frame.at(frame.point(0.3 * k, -0.2 * k, h)).unwrap();
                    let surface =
                        warped_square(&square_frame, half, multiplicity, rational, &mut random);

                    let mut sheet = Body::new(BodyKind::Sheet);
                    let face = sheet.add_face(Surface::Nurbs(surface.clone()), false, Vec::new());
                    sheet.add_shell(vec![face]);
                    let area = mass_properties(&sheet)
                        .unwrap_or_else(|e| panic!("{what}: {e}"))
                        .area;
                    let square = 4.0 * half * half;
                    assert!(
                        (area - square).abs() <= 1e-11 * square,
                        "{what}: the square is {area}, not {square}"
                    );

                    let mut body = Body::new(BodyKind::Solid);
                    let low = body.add_vertex(frame.point(r, 0.0, 0.0));
                    let high = body.add_vertex(frame.point(r, 0.0, h));
                    let bottom = body.add_edge(
                        Curve3::Circle(Circle::new(frame, r).unwrap()),
                        (0.0, TAU),
                        low,
                        low,
                    );
                    let top = body.add_edge(
                        Curve3::Circle(Circle::new(top_frame, r).unwrap()),
                        (0.0, TAU),
                        high,
                        high,
                    );
                    let loops = vec![
                        body.add_loop(vec![fwd(bottom)]),
                        body.add_loop(vec![back(top)]),
                    ];
                    let wall = body.add_face(
                        Surface::Cylinder(Cylinder::new(frame, r).unwrap()),
                        false,
                        loops,
                    );
                    let floor = flat(&mut body, frame.origin(), -frame.z(), vec![back(bottom)]);
                    let rim = body.add_loop(vec![fwd(top)]);
                    let lid = body.add_face(Surface::Nurbs(surface), false, vec![rim]);
                    body.add_shell(vec![wall, floor, lid]);
                    let mass = measured(&body, &what, k);
                    let (volume, area) = (PI * r * r * h, TAU * r * (r + h));
                    let off = (
                        (mass.volume - volume).abs() / volume,
                        (mass.area - area).abs() / area,
                    );
                    assert!(
                        off.0 <= 1e-11 && off.1 <= 1e-11,
                        "{what}: volume {} vs {volume} ({:e}), area {} vs {area} ({:e})",
                        mass.volume,
                        off.0,
                        mass.area,
                        off.1
                    );
                    worst = (worst.0.max(off.0), worst.1.max(off.1));
                }
            }
        }
    }
    println!(
        "worst relative error: volume {:e}, area {:e}",
        worst.0, worst.1
    );
}
