//! Mass properties of the faces a revolution makes, against closed forms:
//! torus faces bounded around the tube, around the axis or both, sphere
//! faces through one or both poles, cone faces through the apex. The
//! bodies are built by hand here (the kernel is above this crate), each
//! with the formulas it is checked against in a comment.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Cone, Curve3, Cylinder, Line, Plane, Sphere, Surface, Torus};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_testkit::trimmed;
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, check, mass_properties};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

struct Solid {
    name: String,
    body: Body,
    volume: f64,
    area: f64,
    centroid: Option<Vec3>,
}

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

/// The direction away from the axis at `angle` about the frame's z.
fn radial(frame: &Frame, angle: f64) -> Vec3 {
    frame.vector(angle.cos(), angle.sin(), 0.0)
}

/// The direction a point at `angle` moves in when the angle grows.
fn onward(frame: &Frame, angle: f64) -> Vec3 {
    frame.vector(-angle.sin(), angle.cos(), 0.0)
}

/// A circle about the frame's axis at height `z`; its parameter is the
/// angle about the axis.
fn level_circle(frame: &Frame, z: f64, radius: f64) -> Curve3 {
    Curve3::Circle(Circle::new(frame.at(frame.point(0.0, 0.0, z)).unwrap(), radius).unwrap())
}

/// A circle in the half-plane at `angle` through the axis, about the point
/// at distance `rho` from the axis and height `z`. Its parameter is the
/// angle from the radial direction towards the axis direction.
fn meridian_circle(frame: &Frame, angle: f64, rho: f64, z: f64, radius: f64) -> Curve3 {
    let centre = frame.origin() + radial(frame, angle) * rho + frame.z() * z;
    let placed = Frame::new(centre, -onward(frame, angle), radial(frame, angle)).unwrap();
    Curve3::Circle(Circle::new(placed, radius).unwrap())
}

fn line(body: &mut Body, from: Vec3, to: Vec3, ends: (u32, u32)) -> EdgeId {
    body.add_edge(
        Curve3::Line(Line::through(from, to).unwrap()),
        (0.0, from.distance(to)),
        craft3d_topo::VertexId(ends.0),
        craft3d_topo::VertexId(ends.1),
    )
}

fn flat(body: &mut Body, origin: Vec3, normal: Vec3, hint: Vec3, coedges: Vec<Coedge>) -> FaceId {
    let loop_ = body.add_loop(coedges);
    let frame = Frame::new(origin, normal, hint).unwrap();
    body.add_face(Surface::Plane(Plane::new(frame)), false, vec![loop_])
}

/// `sin(x) / x` for the centroid of a swept shape: the mean of the radial
/// direction over a sweep of `2x`.
fn chord_factor(sweep: f64) -> f64 {
    (sweep / 2.0).sin() / (sweep / 2.0)
}

/// The part of a torus tube (`major`, `minor`) cut off by a cylinder about
/// the axis: the segment of the tube's disc within the angle `half` either
/// side of the outward direction (`outer`) or of the inward one, revolved
/// through `sweep`, or all the way round when that is `None`.
///
/// Pappus: the segment has area `r² (2a − sin 2a) / 2` with its centroid
/// `4 r sin³a / (3 (2a − sin 2a))` from the tube's centre; its arc has
/// length `2ar` with its centroid `r sin a / a` from the centre; its chord
/// has length `2 r sin a` at `r cos a` from the centre.
fn torus_segment(
    frame: &Frame,
    major: f64,
    minor: f64,
    half: f64,
    outer: bool,
    sweep: Option<f64>,
) -> Solid {
    let side = if outer { 1.0 } else { -1.0 };
    let wall = major + side * minor * half.cos();
    let height = minor * half.sin();
    let torus = Surface::Torus(Torus::new(*frame, major, minor).unwrap());
    let cylinder = Surface::Cylinder(Cylinder::new(*frame, wall).unwrap());
    // The tube angle the arc starts and ends at, and the height there.
    let (v0, v1) = if outer {
        (-half, half)
    } else {
        (PI - half, PI + half)
    };
    let (z0, z1) = (minor * v0.sin(), minor * v1.sin());
    let mut body = Body::new(BodyKind::Solid);
    let corner = |angle: f64, z: f64| frame.origin() + radial(frame, angle) * wall + frame.z() * z;
    match sweep {
        None => {
            let a = body.add_vertex(corner(0.0, z0));
            let b = body.add_vertex(corner(0.0, z1));
            let first = body.add_edge(level_circle(frame, z0, wall), (0.0, TAU), a, a);
            let second = body.add_edge(level_circle(frame, z1, wall), (0.0, TAU), b, b);
            // On the torus the face is the band from v0 up to v1: left of
            // the circle at v0 running forward.
            let band = vec![
                body.add_loop(vec![fwd(first)]),
                body.add_loop(vec![back(second)]),
            ];
            let band = body.add_face(torus, false, band);
            let wall_loops = vec![
                body.add_loop(vec![back(first)]),
                body.add_loop(vec![fwd(second)]),
            ];
            // The outer segment's flat side looks towards the axis.
            let wall_face = body.add_face(cylinder, outer, wall_loops);
            body.add_shell(vec![band, wall_face]);
        }
        Some(sweep) => {
            let a0 = body.add_vertex(corner(0.0, z0));
            let b0 = body.add_vertex(corner(0.0, z1));
            let a1 = body.add_vertex(corner(sweep, z0));
            let b1 = body.add_vertex(corner(sweep, z1));
            let first = body.add_edge(level_circle(frame, z0, wall), (0.0, sweep), a0, a1);
            let second = body.add_edge(level_circle(frame, z1, wall), (0.0, sweep), b0, b1);
            let arc0 = body.add_edge(
                meridian_circle(frame, 0.0, major, 0.0, minor),
                (v0, v1),
                a0,
                b0,
            );
            let arc1 = body.add_edge(
                meridian_circle(frame, sweep, major, 0.0, minor),
                (v0, v1),
                a1,
                b1,
            );
            let chord0 = line(&mut body, corner(0.0, z0), corner(0.0, z1), (a0.0, b0.0));
            let chord1 = line(
                &mut body,
                corner(sweep, z0),
                corner(sweep, z1),
                (a1.0, b1.0),
            );
            // The rectangle [0, sweep] × [v0, v1], counter-clockwise.
            let patch = body.add_loop(vec![fwd(first), fwd(arc1), back(second), back(arc0)]);
            let patch = body.add_face(torus, false, vec![patch]);
            let wall_loop =
                body.add_loop(vec![back(first), fwd(chord0), fwd(second), back(chord1)]);
            let wall_face = body.add_face(cylinder, outer, vec![wall_loop]);
            let start = flat(
                &mut body,
                frame.origin(),
                -onward(frame, 0.0),
                frame.z(),
                vec![fwd(arc0), back(chord0)],
            );
            let end = flat(
                &mut body,
                frame.origin(),
                onward(frame, sweep),
                frame.z(),
                vec![fwd(chord1), back(arc1)],
            );
            body.add_shell(vec![patch, wall_face, start, end]);
        }
    }
    let segment = minor * minor * (2.0 * half - (2.0 * half).sin()) / 2.0;
    let lever = 4.0 * minor * half.sin().powi(3) / (3.0 * (2.0 * half - (2.0 * half).sin()));
    let angle = sweep.unwrap_or(TAU);
    let caps = if sweep.is_some() { 2.0 * segment } else { 0.0 };
    Solid {
        name: format!(
            "torus segment, {} side, half angle {half}, sweep {sweep:?}",
            if outer { "outer" } else { "inner" }
        ),
        body,
        volume: angle * (major + side * lever) * segment,
        area: angle * (major + side * minor * half.sin() / half) * 2.0 * half * minor
            + angle * wall * 2.0 * height
            + caps,
        // All the way round the centroid is on the axis, in the plane of
        // the tube's centre.
        centroid: sweep.is_none().then(|| frame.origin()),
    }
}

/// The slice of a ball between the planes z = `low` and z = `high`: a zone
/// of the sphere between two parallels, and two discs.
///
/// V = π ∫ (r² − z²) dz, the zone's area is 2πr (high − low) (Archimedes),
/// and the centroid is at ∫ z (r² − z²) dz / ∫ (r² − z²) dz on the axis.
fn sphere_segment(frame: &Frame, radius: f64, low: f64, high: f64) -> Solid {
    let rim = |z: f64| (radius * radius - z * z).sqrt();
    let mut body = Body::new(BodyKind::Solid);
    let a = body.add_vertex(frame.point(rim(low), 0.0, low));
    let b = body.add_vertex(frame.point(rim(high), 0.0, high));
    let bottom = body.add_edge(level_circle(frame, low, rim(low)), (0.0, TAU), a, a);
    let top = body.add_edge(level_circle(frame, high, rim(high)), (0.0, TAU), b, b);
    let zone = vec![
        body.add_loop(vec![fwd(bottom)]),
        body.add_loop(vec![back(top)]),
    ];
    let zone = body.add_face(
        Surface::Sphere(Sphere::new(*frame, radius).unwrap()),
        false,
        zone,
    );
    let floor = flat(
        &mut body,
        frame.point(0.0, 0.0, low),
        -frame.z(),
        frame.x(),
        vec![back(bottom)],
    );
    let lid = flat(
        &mut body,
        frame.point(0.0, 0.0, high),
        frame.z(),
        frame.x(),
        vec![fwd(top)],
    );
    body.add_shell(vec![zone, floor, lid]);
    let r2 = radius * radius;
    let slab = |z: f64| r2 * z - z.powi(3) / 3.0;
    let moment = |z: f64| r2 * z * z / 2.0 - z.powi(4) / 4.0;
    let volume = PI * (slab(high) - slab(low));
    Solid {
        name: format!("sphere segment {low}..{high}"),
        body,
        volume,
        area: TAU * radius * (high - low) + PI * (rim(low).powi(2) + rim(high).powi(2)),
        centroid: Some(frame.point(0.0, 0.0, PI * (moment(high) - moment(low)) / volume)),
    }
}

/// A wedge of a ball: the part between the half-planes through the axis at
/// angles 0 and `sweep`. Its curved face is a lune that runs from pole to
/// pole.
///
/// V = (sweep / 2π) 4πr³/3, the lune is the same share of 4πr², each flat
/// side is half a disc. A half disc has ∫ρ dA = 2r³/3 and ∫ρ² dA = πr⁴/8,
/// so each thin slice has its centroid 3πr/16 from the axis.
fn sphere_wedge(frame: &Frame, radius: f64, sweep: f64) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let south = body.add_vertex(frame.point(0.0, 0.0, -radius));
    let north = body.add_vertex(frame.point(0.0, 0.0, radius));
    let quarter = (-FRAC_PI_2, FRAC_PI_2);
    let m0 = body.add_edge(
        meridian_circle(frame, 0.0, 0.0, 0.0, radius),
        quarter,
        south,
        north,
    );
    let m1 = body.add_edge(
        meridian_circle(frame, sweep, 0.0, 0.0, radius),
        quarter,
        south,
        north,
    );
    let axis = line(
        &mut body,
        frame.point(0.0, 0.0, -radius),
        frame.point(0.0, 0.0, radius),
        (south.0, north.0),
    );
    // Up the far meridian, down the near one.
    let lune = body.add_loop(vec![fwd(m1), back(m0)]);
    let lune = body.add_face(
        Surface::Sphere(Sphere::new(*frame, radius).unwrap()),
        false,
        vec![lune],
    );
    let start = flat(
        &mut body,
        frame.origin(),
        -onward(frame, 0.0),
        frame.z(),
        vec![fwd(m0), back(axis)],
    );
    let end = flat(
        &mut body,
        frame.origin(),
        onward(frame, sweep),
        frame.z(),
        vec![fwd(axis), back(m1)],
    );
    body.add_shell(vec![lune, start, end]);
    let reach = 3.0 * PI * radius / 16.0 * chord_factor(sweep);
    Solid {
        name: format!("sphere wedge {sweep}"),
        body,
        volume: sweep * 2.0 * radius.powi(3) / 3.0,
        area: 2.0 * sweep * radius * radius + PI * radius * radius,
        centroid: Some(frame.origin() + radial(frame, sweep / 2.0) * reach),
    }
}

/// Half a ball, cut by a plane *through* the sphere's axis: the boundary
/// of its curved face is one closed great circle that passes through both
/// poles between its ends. V = 2πr³/3, A = 2πr² + πr², centroid 3r/8 from
/// the flat side.
fn sideways_hemisphere(frame: &Frame, radius: f64) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let vertex = body.add_vertex(frame.point(radius, 0.0, 0.0));
    // Runs counter-clockwise about −y: the ball is kept on the +y side.
    let rim = body.add_edge(
        meridian_circle(frame, 0.0, 0.0, 0.0, radius),
        (0.0, TAU),
        vertex,
        vertex,
    );
    let dome = body.add_loop(vec![back(rim)]);
    let dome = body.add_face(
        Surface::Sphere(Sphere::new(*frame, radius).unwrap()),
        false,
        vec![dome],
    );
    let disc = flat(
        &mut body,
        frame.origin(),
        -frame.y(),
        frame.x(),
        vec![fwd(rim)],
    );
    body.add_shell(vec![dome, disc]);
    Solid {
        name: "sideways hemisphere".to_owned(),
        body,
        volume: TAU * radius.powi(3) / 3.0,
        area: 3.0 * PI * radius * radius,
        centroid: Some(frame.point(0.0, 3.0 * radius / 8.0, 0.0)),
    }
}

/// A ball cut by the plane x = `level`, which misses the sphere's axis:
/// the cap beyond the plane (`keep_cap`), whose face is inside a loop that
/// circles neither pole, or the rest of the ball, whose face is *outside*
/// that loop and holds both poles.
///
/// A cap of height h has V = πh²(3r − h)/3 and curved area 2πrh; its
/// centroid is 3(2r − h)² / (4(3r − h)) from the centre.
fn sideways_cut(frame: &Frame, radius: f64, level: f64, keep_cap: bool) -> Solid {
    let rim_radius = (radius * radius - level * level).sqrt();
    let mut body = Body::new(BodyKind::Solid);
    // A circle about the frame's x axis.
    let placed = Frame::new(frame.point(level, 0.0, 0.0), frame.x(), frame.y()).unwrap();
    let vertex = body.add_vertex(placed.point(rim_radius, 0.0, 0.0));
    let rim = body.add_edge(
        Curve3::Circle(Circle::new(placed, rim_radius).unwrap()),
        (0.0, TAU),
        vertex,
        vertex,
    );
    let (on_sphere, on_disc, normal) = if keep_cap {
        (fwd(rim), back(rim), -frame.x())
    } else {
        (back(rim), fwd(rim), frame.x())
    };
    let curved = body.add_loop(vec![on_sphere]);
    let curved = body.add_face(
        Surface::Sphere(Sphere::new(*frame, radius).unwrap()),
        false,
        vec![curved],
    );
    let disc = flat(
        &mut body,
        frame.point(level, 0.0, 0.0),
        normal,
        frame.y(),
        vec![on_disc],
    );
    body.add_shell(vec![curved, disc]);
    let h = radius - level;
    let cap_volume = PI * h * h * (3.0 * radius - h) / 3.0;
    let cap_centre = 3.0 * (2.0 * radius - h).powi(2) / (4.0 * (3.0 * radius - h));
    let ball = 4.0 * PI * radius.powi(3) / 3.0;
    let disc_area = PI * rim_radius * rim_radius;
    let (volume, area, x) = if keep_cap {
        (cap_volume, TAU * radius * h + disc_area, cap_centre)
    } else {
        // The ball's own centroid is at the centre: the moments cancel.
        let rest = ball - cap_volume;
        (
            rest,
            4.0 * PI * radius * radius - TAU * radius * h + disc_area,
            -cap_volume * cap_centre / rest,
        )
    };
    Solid {
        name: format!("ball cut at x = {level}, cap kept: {keep_cap}"),
        body,
        volume,
        area,
        centroid: Some(frame.point(x, 0.0, 0.0)),
    }
}

/// A slice of a cone (base `radius` on the frame's xy plane, apex at
/// `height`) between the half-planes at angles 0 and `sweep`: a cone face
/// whose boundary runs through the apex.
///
/// V = sweep r²h/6; the cone face is sweep r s/2 with s the slant height,
/// the base sweep r²/2, each flat side r h/2. A thin slice (a triangle)
/// has ∫ρ dA = r²h/6 and ∫ρ² dA = r³h/12: its centroid is r/2 from the
/// axis, and h/4 up.
fn cone_wedge(frame: &Frame, radius: f64, height: f64, sweep: f64) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let centre = frame.origin();
    let apex = frame.point(0.0, 0.0, height);
    let (p0, p1) = (
        centre + radial(frame, 0.0) * radius,
        centre + radial(frame, sweep) * radius,
    );
    let c = body.add_vertex(centre);
    let top = body.add_vertex(apex);
    let a0 = body.add_vertex(p0);
    let a1 = body.add_vertex(p1);
    let arc = body.add_edge(level_circle(frame, 0.0, radius), (0.0, sweep), a0, a1);
    let ruling0 = line(&mut body, p0, apex, (a0.0, top.0));
    let ruling1 = line(&mut body, p1, apex, (a1.0, top.0));
    let spoke0 = line(&mut body, centre, p0, (c.0, a0.0));
    let spoke1 = line(&mut body, centre, p1, (c.0, a1.0));
    let axis = line(&mut body, centre, apex, (c.0, top.0));
    let side = body.add_loop(vec![fwd(arc), fwd(ruling1), back(ruling0)]);
    let cone = Cone::new(*frame, radius, -(radius / height).atan()).unwrap();
    let side = body.add_face(Surface::Cone(cone), false, vec![side]);
    let base = flat(
        &mut body,
        centre,
        -frame.z(),
        frame.x(),
        vec![fwd(spoke1), back(arc), back(spoke0)],
    );
    let start = flat(
        &mut body,
        centre,
        -onward(frame, 0.0),
        frame.z(),
        vec![fwd(spoke0), fwd(ruling0), back(axis)],
    );
    let end = flat(
        &mut body,
        centre,
        onward(frame, sweep),
        frame.z(),
        vec![fwd(axis), back(ruling1), back(spoke1)],
    );
    body.add_shell(vec![side, base, start, end]);
    let slant = radius.hypot(height);
    let reach = radius / 2.0 * chord_factor(sweep);
    Solid {
        name: format!("cone wedge {sweep}"),
        body,
        volume: sweep * radius * radius * height / 6.0,
        area: sweep * radius * slant / 2.0 + sweep * radius * radius / 2.0 + radius * height,
        centroid: Some(centre + radial(frame, sweep / 2.0) * reach + frame.z() * (height / 4.0)),
    }
}

fn frames(scale: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(1.0, -2.0, 0.5) * scale,
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap(),
    ]
}

fn solids(frame: &Frame, scale: f64) -> Vec<Solid> {
    let mut all = Vec::new();
    for outer in [true, false] {
        for half in [0.4, FRAC_PI_2, 2.5] {
            for sweep in [None, Some(0.3), Some(FRAC_PI_2), Some(4.0), Some(6.0)] {
                all.push(torus_segment(
                    frame,
                    3.0 * scale,
                    1.0 * scale,
                    half,
                    outer,
                    sweep,
                ));
            }
        }
    }
    all.push(sphere_segment(
        frame,
        2.0 * scale,
        -0.5 * scale,
        1.2 * scale,
    ));
    all.push(sphere_segment(frame, 2.0 * scale, 0.3 * scale, 1.9 * scale));
    all.push(sphere_segment(
        frame,
        2.0 * scale,
        -1.7 * scale,
        -0.2 * scale,
    ));
    for sweep in [0.2, FRAC_PI_2, PI, 4.0, 6.1] {
        all.push(sphere_wedge(frame, 2.0 * scale, sweep));
        all.push(cone_wedge(frame, 1.5 * scale, 2.5 * scale, sweep));
    }
    all.push(cone_wedge(frame, 3.0 * scale, 0.2 * scale, 1.0));
    all.push(cone_wedge(frame, 0.2 * scale, 3.0 * scale, 5.0));
    all.push(sideways_hemisphere(frame, 2.0 * scale));
    for level in [-1.2, 0.0, 0.7, 1.9] {
        for keep_cap in [true, false] {
            all.push(sideways_cut(frame, 2.0 * scale, level * scale, keep_cap));
        }
    }
    all
}

#[test]
fn revolved_faces_match_their_closed_forms() {
    let mut worst = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut count = 0;
    for scale in [1e-3, 1.0, 1e6] {
        let tolerance = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
        for frame in frames(scale) {
            for solid in solids(&frame, scale) {
                let what = format!("{} at scale {scale}", solid.name);
                assert_eq!(check(&solid.body, &tolerance), vec![], "{what}");
                let mass = mass_properties(&solid.body).unwrap_or_else(|e| panic!("{what}: {e}"));
                // 1e-11 relative: Gauss–Legendre sums of trigonometric
                // integrands, exact to rounding; the frames are tilted, so
                // a few digits go to the rotation.
                let volume = (mass.volume - solid.volume).abs() / solid.volume;
                let area = (mass.area - solid.area).abs() / solid.area;
                assert!(
                    volume <= 1e-11,
                    "{what}: volume {} vs {}",
                    mass.volume,
                    solid.volume
                );
                assert!(
                    area <= 1e-11,
                    "{what}: area {} vs {}",
                    mass.area,
                    solid.area
                );
                worst = (worst.0.max(volume), worst.1.max(area), worst.2);
                if let Some(expected) = solid.centroid {
                    let off = mass.centroid.unwrap().distance(expected) / scale;
                    assert!(
                        off <= 1e-10,
                        "{what}: centroid {:?} vs {expected:?}",
                        mass.centroid
                    );
                    worst.2 = worst.2.max(off);
                }
                count += 1;
            }
        }
    }
    println!(
        "{count} solids; worst relative error: volume {:e}, area {:e}, centroid {:e}",
        worst.0, worst.1, worst.2
    );
}

/// The two fixtures of `craft3d-testkit` that `mass_properties` used to
/// refuse, at every scale and in a tilted frame, centroid included.
#[test]
fn the_pipe_elbow_and_the_sphere_octant_match_their_closed_forms() {
    for scale in [1e-3, 1.0, 1e6] {
        for frame in frames(scale) {
            let mut all = vec![trimmed::sphere_octant(&frame, 2.0 * scale).unwrap()];
            for sweep in [0.1, 1.2, 1.9, PI, 4.0, 6.2] {
                all.push(trimmed::pipe_elbow(&frame, 3.0 * scale, 1.0 * scale, sweep).unwrap());
            }
            for solid in all {
                let what = format!("{} at scale {scale}", solid.name);
                let mass = mass_properties(&solid.body).unwrap_or_else(|e| panic!("{what}: {e}"));
                assert!(
                    (mass.volume - solid.volume).abs() <= 1e-11 * solid.volume,
                    "{what}: volume {} vs {}",
                    mass.volume,
                    solid.volume
                );
                assert!(
                    (mass.area - solid.area).abs() <= 1e-11 * solid.area,
                    "{what}: area {} vs {}",
                    mass.area,
                    solid.area
                );
                let centroid = mass.centroid.unwrap();
                assert!(
                    centroid.distance(solid.centroid) <= 1e-10 * scale,
                    "{what}: centroid {centroid:?} vs {:?}",
                    solid.centroid
                );
            }
        }
    }
}

/// Parts add up: wedges of a ball, of a cone and of a torus segment whose
/// sweeps make a whole turn have the whole solid's volume, whatever the
/// formulas above say.
#[test]
fn wedges_of_a_whole_turn_add_up_to_the_whole() {
    let frame = frames(1.0)[1];
    let cuts = [0.0, 0.7, 2.9, 4.4, TAU];
    let volume = |solid: Solid| mass_properties(&solid.body).unwrap().volume;
    let mut ball = 0.0;
    let mut cone = 0.0;
    let mut ring = 0.0;
    for pair in cuts.windows(2) {
        let sweep = pair[1] - pair[0];
        ball += volume(sphere_wedge(&frame, 2.0, sweep));
        cone += volume(cone_wedge(&frame, 1.5, 2.5, sweep));
        ring += volume(torus_segment(&frame, 3.0, 1.0, 1.1, true, Some(sweep)));
    }
    let whole_ring = volume(torus_segment(&frame, 3.0, 1.0, 1.1, true, None));
    assert!((ball - 4.0 * PI * 8.0 / 3.0).abs() < 1e-11 * ball, "{ball}");
    assert!(
        (cone - PI * 1.5 * 1.5 * 2.5 / 3.0).abs() < 1e-11 * cone,
        "{cone}"
    );
    assert!((ring - whole_ring).abs() < 1e-11 * ring, "{ring}");
}

/// What the integrator still cannot do stays an error, not a number.
#[test]
fn what_cannot_be_measured_is_refused() {
    use craft3d_topo::MassError;
    // A torus face with a single loop around the axis is not bounded by it.
    let frame = Frame::WORLD;
    let mut body = Body::new(BodyKind::Sheet);
    let vertex = body.add_vertex(frame.point(4.0, 0.0, 0.0));
    let circle = body.add_edge(level_circle(&frame, 0.0, 4.0), (0.0, TAU), vertex, vertex);
    let loop_ = body.add_loop(vec![fwd(circle)]);
    let face = body.add_face(
        Surface::Torus(Torus::new(frame, 3.0, 1.0).unwrap()),
        false,
        vec![loop_],
    );
    body.add_shell(vec![face]);
    assert_eq!(
        mass_properties(&body),
        Err(MassError::UnboundedFace { face: 0 })
    );

    // A sphere face whose boundary goes up a meridian to the pole and
    // straight back down it encloses nothing or everything.
    let mut body = Body::new(BodyKind::Sheet);
    let south = body.add_vertex(frame.point(0.0, 0.0, -2.0));
    let north = body.add_vertex(frame.point(0.0, 0.0, 2.0));
    let up = body.add_edge(
        meridian_circle(&frame, 0.0, 0.0, 0.0, 2.0),
        (-FRAC_PI_2, FRAC_PI_2),
        south,
        north,
    );
    let loop_ = body.add_loop(vec![fwd(up), back(up)]);
    let face = body.add_face(
        Surface::Sphere(Sphere::new(frame, 2.0).unwrap()),
        false,
        vec![loop_],
    );
    body.add_shell(vec![face]);
    let error = mass_properties(&body).unwrap_err();
    assert!(error.to_string().contains("doubles back"), "{error}");
}

/// A sphere less an octant, as a sheet: a face that holds one pole and
/// whose boundary passes through the other. 7/8 of 4πr².
#[test]
fn a_sphere_less_an_octant() {
    for frame in frames(1.0) {
        let octant = trimmed::sphere_octant(&frame, 2.0).unwrap().body;
        let dome = octant
            .faces
            .iter()
            .find(|f| matches!(f.surface, Surface::Sphere(_)))
            .unwrap();
        let mut around: Vec<Coedge> = octant.loops[dome.loops[0].0 as usize].coedges.clone();
        around.reverse();
        for c in &mut around {
            c.reversed = !c.reversed;
        }
        let mut sheet = Body::new(BodyKind::Sheet);
        sheet.vertices = octant.vertices.clone();
        sheet.edges = octant.edges.clone();
        let rest = sheet.add_loop(around);
        let face = sheet.add_face(dome.surface.clone(), false, vec![rest]);
        sheet.add_shell(vec![face]);
        let area = mass_properties(&sheet).unwrap().area;
        let expected = 3.5 * PI * 4.0;
        assert!(
            (area - expected).abs() <= 1e-11 * expected,
            "{area} vs {expected}"
        );
    }
}
