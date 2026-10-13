//! The genus of bodies whose faces are not discs: faces of a torus that
//! keep its handle and faces that do not, faces of spheres, cylinders and
//! cones with several loops, cavities and several shells.
//!
//! Every body is built by hand here and its genus is known from what it
//! is: a ball with holes drilled through it has as many handles as holes.
//! Each also passes `check`, and its volume is held to a closed form, so
//! that a face on the wrong side of its loops is not counted as a body.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Cone, Curve3, Cylinder, Line, Plane, Sphere, Surface, Torus};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_testkit::solids;
use craft3d_topo::{
    Body, BodyKind, Coedge, EdgeId, FaceId, VertexId, check, genus, mass_properties, merge,
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

/// The same loop walked the other way.
fn turned(mut coedges: Vec<Coedge>) -> Vec<Coedge> {
    coedges.reverse();
    for coedge in &mut coedges {
        coedge.reversed = !coedge.reversed;
    }
    coedges
}

fn radial(frame: &Frame, angle: f64) -> Vec3 {
    frame.vector(angle.cos(), angle.sin(), 0.0)
}

fn onward(frame: &Frame, angle: f64) -> Vec3 {
    frame.vector(-angle.sin(), angle.cos(), 0.0)
}

/// A circle about the frame's axis at height `z`; its parameter is the
/// angle about the axis.
fn level_circle(frame: &Frame, z: f64, radius: f64) -> Curve3 {
    Curve3::Circle(Circle::new(frame.at(frame.point(0.0, 0.0, z)).unwrap(), radius).unwrap())
}

/// The circle of a torus's tube in the half-plane at `angle`; its
/// parameter is the torus's v.
fn tube_circle(frame: &Frame, angle: f64, major: f64, minor: f64) -> Curve3 {
    let centre = frame.origin() + radial(frame, angle) * major;
    let placed = Frame::new(centre, -onward(frame, angle), radial(frame, angle)).unwrap();
    Curve3::Circle(Circle::new(placed, minor).unwrap())
}

/// A whole circle about the frame's axis as an edge with one vertex.
fn ring(body: &mut Body, frame: &Frame, z: f64, radius: f64) -> EdgeId {
    let vertex = body.add_vertex(frame.point(radius, 0.0, z));
    body.add_edge(level_circle(frame, z, radius), (0.0, TAU), vertex, vertex)
}

fn line(body: &mut Body, ends: (VertexId, VertexId)) -> EdgeId {
    let (from, to) = (
        body.vertex(ends.0).unwrap().point,
        body.vertex(ends.1).unwrap().point,
    );
    body.add_edge(
        Curve3::Line(Line::through(from, to).unwrap()),
        (0.0, from.distance(to)),
        ends.0,
        ends.1,
    )
}

fn flat(body: &mut Body, origin: Vec3, normal: Vec3, loops: Vec<Vec<Coedge>>) -> FaceId {
    let loops = loops.into_iter().map(|l| body.add_loop(l)).collect();
    let frame = Frame::from_z(origin, normal).unwrap();
    body.add_face(Surface::Plane(Plane::new(frame)), false, loops)
}

fn curved(body: &mut Body, surface: Surface, reversed: bool, loops: Vec<Vec<Coedge>>) -> FaceId {
    let loops = loops.into_iter().map(|l| body.add_loop(l)).collect();
    body.add_face(surface, reversed, loops)
}

/// The same solid inside out: a cavity of its shape.
fn inside_out(body: &Body) -> Body {
    let mut out = body.clone();
    for face in &mut out.faces {
        face.reversed = !face.reversed;
    }
    for loop_ in &mut out.loops {
        loop_.coedges = turned(loop_.coedges.clone());
    }
    out
}

fn together(bodies: &[&Body]) -> Body {
    merge(bodies).unwrap().0
}

struct Case {
    name: String,
    body: Body,
    genus: u32,
    /// `None` where no closed form is at hand; the body must still be
    /// measurable.
    volume: Option<f64>,
}

const MAJOR: f64 = 3.0;
const MINOR: f64 = 1.0;

fn torus_surface(frame: &Frame, k: f64) -> Surface {
    Surface::Torus(Torus::new(*frame, MAJOR * k, MINOR * k).unwrap())
}

/// A torus with a hole through its tube, from top to bottom, and the plug
/// that came out of it.
///
/// The hole is the part of the tube between the half-planes at the angles
/// `u0` and `u1` about the axis and between the cylinders of radius
/// `MAJOR + x0` and `MAJOR + x1` about it. Planes through the axis meet a
/// torus in circles of its tube and cylinders about the axis in circles
/// about the axis, so every edge is a line or a circle.
///
/// The holed torus has the torus's handle and the hole's: genus 2. Its
/// torus face has two loops, each around a hole in the face, and neither
/// goes around the torus. The plug is a ball: its two torus faces are
/// patches with one loop each.
///
/// The plug's volume is (u1 − u0) ∫ 2 √(r² − x²) (R + x) dx
/// = (u1 − u0) [R (x √(r² − x²) + r² asin(x/r)) − ⅔ (r² − x²)^{3/2}].
fn holed_torus_and_plug(
    frame: &Frame,
    k: f64,
    (u0, u1): (f64, f64),
    (x0, x1): (f64, f64),
) -> [Case; 2] {
    let (major, minor) = (MAJOR * k, MINOR * k);
    let (x0, x1) = (x0 * k, x1 * k);
    let (rho0, rho1) = (major + x0, major + x1);
    // Where the two cylinders meet the upper half of the tube.
    let (va, vb) = ((x1 / minor).acos(), (x0 / minor).acos());
    let build = |plug: bool| {
        let mut body = Body::new(BodyKind::Solid);
        let point = |u: f64, v: f64| {
            frame.origin()
                + radial(frame, u) * (major + minor * v.cos())
                + frame.z() * (minor * v.sin())
        };
        // Named by angle (0, 1), cylinder (0, 1) and top or bottom.
        let t00 = body.add_vertex(point(u0, vb));
        let t01 = body.add_vertex(point(u0, va));
        let t10 = body.add_vertex(point(u1, vb));
        let t11 = body.add_vertex(point(u1, va));
        let b00 = body.add_vertex(point(u0, -vb));
        let b01 = body.add_vertex(point(u0, -va));
        let b10 = body.add_vertex(point(u1, -vb));
        let b11 = body.add_vertex(point(u1, -va));
        let (h0, h1) = (minor * vb.sin(), minor * va.sin());
        let lat_top0 = body.add_edge(level_circle(frame, h0, rho0), (u0, u1), t00, t10);
        let lat_top1 = body.add_edge(level_circle(frame, h1, rho1), (u0, u1), t01, t11);
        let lat_bot0 = body.add_edge(level_circle(frame, -h0, rho0), (u0, u1), b00, b10);
        let lat_bot1 = body.add_edge(level_circle(frame, -h1, rho1), (u0, u1), b01, b11);
        let tube = |u: f64| tube_circle(frame, u, major, minor);
        let mer_top0 = body.add_edge(tube(u0), (va, vb), t01, t00);
        let mer_top1 = body.add_edge(tube(u1), (va, vb), t11, t10);
        let mer_bot0 = body.add_edge(tube(u0), (-vb, -va), b00, b01);
        let mer_bot1 = body.add_edge(tube(u1), (-vb, -va), b10, b11);
        let l00 = line(&mut body, (b00, t00));
        let l01 = line(&mut body, (b01, t01));
        let l10 = line(&mut body, (b10, t10));
        let l11 = line(&mut body, (b11, t11));
        // Around each opening as the holed torus has it: clockwise in
        // (u, v), the face outside.
        let top = vec![fwd(mer_top0), fwd(lat_top0), back(mer_top1), back(lat_top1)];
        let bottom = vec![fwd(mer_bot0), fwd(lat_bot1), back(mer_bot1), back(lat_bot0)];
        // The walls of the hole, looking into it.
        let wall_u0 = vec![back(mer_top0), back(l01), back(mer_bot0), fwd(l00)];
        let wall_u1 = vec![fwd(mer_top1), back(l10), fwd(mer_bot1), fwd(l11)];
        let wall_rho1 = vec![fwd(lat_top1), back(l11), back(lat_bot1), fwd(l01)];
        let wall_rho0 = vec![back(lat_top0), back(l00), fwd(lat_bot0), fwd(l10)];
        let side = |loop_: Vec<Coedge>| if plug { turned(loop_) } else { loop_ };
        let sign = if plug { -1.0 } else { 1.0 };
        let torus = torus_surface(frame, k);
        let mut faces = if plug {
            vec![
                curved(&mut body, torus.clone(), false, vec![turned(top)]),
                curved(&mut body, torus, false, vec![turned(bottom)]),
            ]
        } else {
            vec![curved(&mut body, torus, false, vec![top, bottom])]
        };
        faces.push(flat(
            &mut body,
            frame.origin(),
            onward(frame, u0) * sign,
            vec![side(wall_u0)],
        ));
        faces.push(flat(
            &mut body,
            frame.origin(),
            -onward(frame, u1) * sign,
            vec![side(wall_u1)],
        ));
        let outer = Surface::Cylinder(Cylinder::new(*frame, rho1).unwrap());
        let inner = Surface::Cylinder(Cylinder::new(*frame, rho0).unwrap());
        // A cylinder's own normal points away from the axis.
        faces.push(curved(&mut body, outer, !plug, vec![side(wall_rho1)]));
        faces.push(curved(&mut body, inner, plug, vec![side(wall_rho0)]));
        body.add_shell(faces);
        body
    };
    let part = |x: f64| {
        let root = (minor * minor - x * x).sqrt();
        major * (x * root + minor * minor * (x / minor).asin()) - 2.0 / 3.0 * root.powi(3)
    };
    let plug = (u1 - u0) * (part(x1) - part(x0));
    let whole = 2.0 * PI * PI * major * minor * minor;
    [
        Case {
            name: format!("a torus with a hole through its tube, {u0}..{u1}, {x0}..{x1}"),
            body: build(false),
            genus: 2,
            volume: Some(whole - plug),
        },
        Case {
            name: format!("the plug out of a torus, {u0}..{u1}, {x0}..{x1}"),
            body: build(true),
            genus: 0,
            volume: Some(plug),
        },
    ]
}

/// The part of a torus between the half-planes at `ua` and `ub` about its
/// axis: a bent rod. Its torus face has two loops and both go around the
/// tube: it is a band, not a torus with holes. Pappus: (ub − ua) R πr².
fn torus_sector(frame: &Frame, k: f64, (ua, ub): (f64, f64)) -> Case {
    let (major, minor) = (MAJOR * k, MINOR * k);
    let mut body = Body::new(BodyKind::Solid);
    let mut end = |u: f64| {
        let vertex = body.add_vertex(frame.origin() + radial(frame, u) * (major + minor));
        body.add_edge(
            tube_circle(frame, u, major, minor),
            (0.0, TAU),
            vertex,
            vertex,
        )
    };
    let (start, stop) = (end(ua), end(ub));
    let band = curved(
        &mut body,
        torus_surface(frame, k),
        false,
        vec![vec![fwd(stop)], vec![back(start)]],
    );
    let first = flat(
        &mut body,
        frame.origin() + radial(frame, ua) * major,
        -onward(frame, ua),
        vec![vec![fwd(start)]],
    );
    let last = flat(
        &mut body,
        frame.origin() + radial(frame, ub) * major,
        onward(frame, ub),
        vec![vec![back(stop)]],
    );
    body.add_shell(vec![band, first, last]);
    Case {
        name: format!("a torus from {ua} to {ub} about its axis"),
        body,
        genus: 0,
        volume: Some((ub - ua) * major * PI * minor * minor),
    }
}

/// The half of a torus above (or below) the plane of its tube's centre: a
/// ring still. Its torus face has two loops and both go around the axis;
/// so has the flat face. Each half has half the volume, by symmetry.
fn torus_half(frame: &Frame, k: f64, upper: bool) -> Case {
    let (major, minor) = (MAJOR * k, MINOR * k);
    let mut body = Body::new(BodyKind::Solid);
    let outer = ring(&mut body, frame, 0.0, major + minor);
    let inner = ring(&mut body, frame, 0.0, major - minor);
    // The upper half is v from 0 (outside) to π (inside): left of the
    // outer circle running forward.
    let (on_torus, on_plane, normal) = if upper {
        (
            vec![vec![fwd(outer)], vec![back(inner)]],
            vec![vec![back(outer)], vec![fwd(inner)]],
            -frame.z(),
        )
    } else {
        (
            vec![vec![back(outer)], vec![fwd(inner)]],
            vec![vec![fwd(outer)], vec![back(inner)]],
            frame.z(),
        )
    };
    let band = curved(&mut body, torus_surface(frame, k), false, on_torus);
    let cut = flat(&mut body, frame.origin(), normal, on_plane);
    body.add_shell(vec![band, cut]);
    Case {
        name: format!("half a torus, cut across its axis, upper: {upper}"),
        body,
        genus: 1,
        volume: Some(PI * PI * major * minor * minor),
    }
}

/// A ball with the three caps beyond `level` along its frame's axes cut
/// off: a sphere face with three loops. A cap of height h has
/// V = πh²(3r − h)/3.
fn ball_less_three_caps(frame: &Frame, radius: f64, level: f64) -> Case {
    let mut body = Body::new(BodyKind::Solid);
    let rim = (radius * radius - level * level).sqrt();
    let mut loops = Vec::new();
    let mut faces = Vec::new();
    for (axis, hint) in [
        (frame.x(), frame.y()),
        (frame.y(), frame.z()),
        (frame.z(), frame.x()),
    ] {
        let centre = frame.origin() + axis * level;
        let placed = Frame::new(centre, axis, hint).unwrap();
        let vertex = body.add_vertex(placed.point(rim, 0.0, 0.0));
        let edge = body.add_edge(
            Curve3::Circle(Circle::new(placed, rim).unwrap()),
            (0.0, TAU),
            vertex,
            vertex,
        );
        loops.push(vec![back(edge)]);
        faces.push(flat(&mut body, centre, axis, vec![vec![fwd(edge)]]));
    }
    let sphere = Surface::Sphere(Sphere::new(*frame, radius).unwrap());
    faces.push(curved(&mut body, sphere, false, loops));
    body.add_shell(faces);
    let h = radius - level;
    Case {
        name: "a ball less three caps".to_owned(),
        body,
        genus: 0,
        volume: Some(4.0 * PI * radius.powi(3) / 3.0 - PI * h * h * (3.0 * radius - h)),
    }
}

/// A ball with a round hole through its middle: a sphere face and a
/// cylinder face with two loops each, and nothing else. The napkin ring:
/// V = 4π/3 (r² − a²)^{3/2}.
fn napkin_ring(frame: &Frame, radius: f64, hole: f64) -> Case {
    let mut body = Body::new(BodyKind::Solid);
    let half = (radius * radius - hole * hole).sqrt();
    let bottom = ring(&mut body, frame, -half, hole);
    let top = ring(&mut body, frame, half, hole);
    let sphere = Surface::Sphere(Sphere::new(*frame, radius).unwrap());
    let cylinder = Surface::Cylinder(Cylinder::new(*frame, hole).unwrap());
    let zone = curved(
        &mut body,
        sphere,
        false,
        vec![vec![fwd(bottom)], vec![back(top)]],
    );
    let bore = curved(
        &mut body,
        cylinder,
        true,
        vec![vec![back(bottom)], vec![fwd(top)]],
    );
    body.add_shell(vec![zone, bore]);
    Case {
        name: "a ball with a hole through it".to_owned(),
        body,
        genus: 1,
        volume: Some(4.0 * PI / 3.0 * half.powi(3)),
    }
}

/// A cone frustum (radius `r1` at the bottom, `r2` at the top, `height`)
/// with a round hole of radius `hole` along its axis: a cone face, a
/// cylinder face and two flat rings, two loops each.
/// V = πh (r₁² + r₁r₂ + r₂²)/3 − πa²h.
fn bored_frustum(frame: &Frame, r1: f64, r2: f64, height: f64, hole: f64) -> Case {
    let mut body = Body::new(BodyKind::Solid);
    let outer_low = ring(&mut body, frame, 0.0, r1);
    let outer_high = ring(&mut body, frame, height, r2);
    let inner_low = ring(&mut body, frame, 0.0, hole);
    let inner_high = ring(&mut body, frame, height, hole);
    let cone = Cone::new(*frame, r1, ((r2 - r1) / height).atan()).unwrap();
    let wall = curved(
        &mut body,
        Surface::Cone(cone),
        false,
        vec![vec![fwd(outer_low)], vec![back(outer_high)]],
    );
    let bore = curved(
        &mut body,
        Surface::Cylinder(Cylinder::new(*frame, hole).unwrap()),
        true,
        vec![vec![back(inner_low)], vec![fwd(inner_high)]],
    );
    let floor = flat(
        &mut body,
        frame.origin(),
        -frame.z(),
        vec![vec![back(outer_low)], vec![fwd(inner_low)]],
    );
    let lid = flat(
        &mut body,
        frame.point(0.0, 0.0, height),
        frame.z(),
        vec![vec![fwd(outer_high)], vec![back(inner_high)]],
    );
    body.add_shell(vec![wall, bore, floor, lid]);
    Case {
        name: "a cone frustum with a hole along its axis".to_owned(),
        body,
        genus: 1,
        volume: Some(PI * height * (r1 * r1 + r1 * r2 + r2 * r2) / 3.0 - PI * hole * hole * height),
    }
}

fn cases(frame: &Frame, k: f64) -> Vec<Case> {
    let mut all = Vec::new();
    // Holes through the tube: narrow and wide, off to the inside and to
    // the outside, across the seam of the torus's u.
    for (angles, offsets) in [
        ((0.4, 1.1), (-0.3, 0.5)),
        ((2.0, 5.5), (-0.9, -0.2)),
        ((-0.5, 0.6), (0.1, 0.95)),
    ] {
        all.extend(holed_torus_and_plug(frame, k, angles, offsets));
    }
    // A torus cut in two by a plane through its axis, and by two such
    // planes into a short piece and a long one.
    for angles in [(0.0, PI), (PI, TAU), (0.3, 1.0), (1.0, TAU + 0.3)] {
        all.push(torus_sector(frame, k, angles));
    }
    all.push(torus_half(frame, k, true));
    all.push(torus_half(frame, k, false));
    all.push(ball_less_three_caps(frame, 2.0 * k, 1.6 * k));
    all.push(napkin_ring(frame, 2.0 * k, 0.7 * k));
    all.push(bored_frustum(frame, 2.0 * k, 1.2 * k, 1.5 * k, 0.5 * k));

    // Several shells. The pieces of a cut torus as one body:
    let halves = [
        torus_sector(frame, k, (0.0, PI)),
        torus_sector(frame, k, (PI, TAU)),
    ];
    all.push(Case {
        name: "both halves of a torus cut through its axis".to_owned(),
        body: together(&[&halves[0].body, &halves[1].body]),
        genus: 0,
        volume: Some(2.0 * PI * PI * MAJOR * MINOR * MINOR * k.powi(3)),
    });
    let halves = [torus_half(frame, k, true), torus_half(frame, k, false)];
    all.push(Case {
        name: "both halves of a torus cut across its axis".to_owned(),
        body: together(&[&halves[0].body, &halves[1].body]),
        genus: 2,
        volume: Some(2.0 * PI * PI * MAJOR * MINOR * MINOR * k.powi(3)),
    });
    // Cavities: a block around the torus (which reaches 4 from its axis
    // and 1 either side of its plane), with a void in it.
    let block_frame = frame.at(frame.point(-5.0 * k, -5.0 * k, -2.0 * k)).unwrap();
    let block = solids::block(&block_frame, 10.0 * k, 10.0 * k, 4.0 * k).unwrap();
    let torus = solids::torus(frame, MAJOR * k, MINOR * k).unwrap();
    let ball = solids::sphere(frame, 1.5 * k).unwrap();
    all.push(Case {
        name: "a block with a torus-shaped tunnel closed in it".to_owned(),
        body: together(&[&block.body, &inside_out(&torus.body)]),
        genus: 1,
        volume: Some(block.volume - torus.volume),
    });
    all.push(Case {
        name: "a block with a round void".to_owned(),
        body: together(&[&block.body, &inside_out(&ball.body)]),
        genus: 0,
        volume: Some(block.volume - ball.volume),
    });
    all.push(Case {
        name: "a block with a void in the shape of a torus and a ball in the middle of it"
            .to_owned(),
        body: together(&[
            &block.body,
            &inside_out(&torus.body),
            &inside_out(&ball.body),
        ]),
        genus: 1,
        volume: Some(block.volume - torus.volume - ball.volume),
    });
    let [holed, _] = holed_torus_and_plug(frame, k, (0.4, 1.1), (-0.3, 0.5));
    all.push(Case {
        name: "a block with a void in the shape of a torus with a hole through its tube".to_owned(),
        body: together(&[&block.body, &inside_out(&holed.body)]),
        genus: 2,
        volume: Some(block.volume - holed.volume.unwrap()),
    });
    let thin = solids::torus(frame, MAJOR * k, 0.4 * MINOR * k).unwrap();
    all.push(Case {
        name: "a hollow torus".to_owned(),
        body: together(&[&torus.body, &inside_out(&thin.body)]),
        genus: 2,
        volume: Some(torus.volume - thin.volume),
    });
    all
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

#[test]
fn bodies_with_faces_that_are_not_discs_have_the_genus_they_have() {
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for frame in frames(k) {
            for case in cases(&frame, k) {
                let what = format!("{} at scale {k}", case.name);
                assert_eq!(check(&case.body, &tolerance), vec![], "{what}");
                // The body is what it is said to be: a face on the wrong
                // side of its loops would not have this volume.
                let mass = mass_properties(&case.body).unwrap_or_else(|e| panic!("{what}: {e}"));
                if let Some(volume) = case.volume {
                    assert!(
                        (mass.volume - volume).abs() <= 1e-10 * volume,
                        "{what}: volume {} vs {volume}",
                        mass.volume
                    );
                }
                assert_eq!(genus(&case.body), Some(case.genus), "{what}");
                // Inside out it is the same surface.
                assert_eq!(
                    genus(&inside_out(&case.body)),
                    Some(case.genus),
                    "{what}, inside out"
                );
                count += 1;
            }
        }
    }
    println!("{count} bodies");
}

/// The fixtures of `craft3d-testkit` state their genus; the pipe elbow
/// and the octant have torus and sphere faces with loops.
#[test]
fn the_trimmed_fixtures_have_the_genus_they_state() {
    use craft3d_testkit::trimmed;
    for frame in frames(1.0) {
        for solid in trimmed::all(&frame, 1.0).unwrap() {
            assert_eq!(genus(&solid.body), Some(solid.genus), "{}", solid.name);
        }
    }
}

/// A genus is a count of handles of something closed: a body whose counts
/// fit none has none, also when a torus face is among them.
#[test]
fn a_body_that_is_not_closed_has_no_genus() {
    let frame = Frame::WORLD;
    let [holed, plug] = holed_torus_and_plug(&frame, 1.0, (0.4, 1.1), (-0.3, 0.5));
    for case in [holed, plug] {
        let mut body = case.body;
        // A vertex that nothing uses makes the count odd.
        body.add_vertex(Vec3::new(9.0, 9.0, 9.0));
        assert_eq!(genus(&body), None, "{}", case.name);
    }
}

/// A solid pinched at a point: a cylinder with a conical dent in either
/// end, the two dents meeting tip to tip in the middle (what a triangle
/// standing on the axis with one corner sweeps in a full turn). Its skin
/// touches itself at that one point and nowhere else.
///
/// `check` allows it, and this pins that down. Nothing in how the body
/// hangs together shows the pinch: neither cone has a vertex at its apex,
/// every edge is used twice, and the counts are those of a ball, which is
/// what the skin is once the two tips are told apart. Seeing it takes
/// geometry (that two faces reach the same point without sharing an edge
/// or a vertex there), which is contact without overlap: the province of
/// `tess::body_self_intersections`, to which it is a marginal finding at
/// most.
/// V = πr²h − 2 πr²(h/2)/3.
#[test]
fn a_solid_pinched_at_a_point_passes_and_counts_as_a_ball() {
    for k in [1e-3, 1.0, 1e6] {
        let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for frame in frames(k) {
            let (r, h) = (2.0 * k, 2.0 * k);
            let mut body = Body::new(BodyKind::Solid);
            let bottom = ring(&mut body, &frame, 0.0, r);
            let top = ring(&mut body, &frame, h, r);
            let wall = curved(
                &mut body,
                Surface::Cylinder(Cylinder::new(frame, r).unwrap()),
                false,
                vec![vec![fwd(bottom)], vec![back(top)]],
            );
            // Both cones have their apex at h/2 and the solid outside.
            let slope = (r / (h / 2.0)).atan();
            let lower = Cone::new(frame, r, -slope).unwrap();
            let upper = Cone::new(frame.at(frame.point(0.0, 0.0, h)).unwrap(), r, slope).unwrap();
            let low = curved(
                &mut body,
                Surface::Cone(lower),
                true,
                vec![vec![back(bottom)]],
            );
            let high = curved(&mut body, Surface::Cone(upper), true, vec![vec![fwd(top)]]);
            body.add_shell(vec![wall, low, high]);
            assert_eq!(check(&body, &tolerance), vec![], "scale {k}");
            assert_eq!(genus(&body), Some(0), "scale {k}");
            let volume = PI * r * r * h * (1.0 - 1.0 / 3.0);
            let mass = mass_properties(&body).unwrap();
            assert!(
                (mass.volume - volume).abs() <= 1e-11 * volume,
                "scale {k}: {} vs {volume}",
                mass.volume
            );
        }
    }
}

/// Two cones tip to tip with the solid *inside* them are two lumps that
/// share a point. As two shells that is two solids that touch, which
/// `check` does not look for (`merge` makes such bodies); as one shell it
/// is not a closed surface of any genus, and the counts say so.
#[test]
fn two_lumps_that_touch_in_a_point_are_two_shells_or_a_defect() {
    use craft3d_topo::Defect;
    let frame = frames(1.0)[1];
    let tolerance = Tolerance::new(1e-6, 1e-9).unwrap();
    let (r, h) = (2.0, 2.0);
    let build = |one_shell: bool| {
        let mut body = Body::new(BodyKind::Solid);
        let bottom = ring(&mut body, &frame, 0.0, r);
        let top = ring(&mut body, &frame, h, r);
        let slope = (r / (h / 2.0)).atan();
        let lower = Cone::new(frame, r, -slope).unwrap();
        let upper = Cone::new(frame.at(frame.point(0.0, 0.0, h)).unwrap(), r, slope).unwrap();
        let low = curved(
            &mut body,
            Surface::Cone(lower),
            false,
            vec![vec![fwd(bottom)]],
        );
        let floor = flat(
            &mut body,
            frame.origin(),
            -frame.z(),
            vec![vec![back(bottom)]],
        );
        let high = curved(
            &mut body,
            Surface::Cone(upper),
            false,
            vec![vec![back(top)]],
        );
        let lid = flat(
            &mut body,
            frame.point(0.0, 0.0, h),
            frame.z(),
            vec![vec![fwd(top)]],
        );
        if one_shell {
            body.add_shell(vec![low, floor, high, lid]);
        } else {
            body.add_shell(vec![low, floor]);
            body.add_shell(vec![high, lid]);
        }
        body
    };
    let two = build(false);
    assert_eq!(check(&two, &tolerance), vec![]);
    assert_eq!(genus(&two), Some(0));
    let one = build(true);
    assert_eq!(genus(&one), None);
    assert!(
        check(&one, &tolerance)
            .iter()
            .any(|d| matches!(d, Defect::Euler { .. }))
    );
}
