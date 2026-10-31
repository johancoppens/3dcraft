//! Mass properties of faces whose boundary passes through a pole of a
//! spline surface: a border of the surface's parameter range that is one
//! point in space, where no edge of the body stands for the stretch of the
//! parameter boundary that the point is.
//!
//! Every body has a closed form: wedges, octants, caps and halves of a
//! ball whose surface is a rational spline swept from its meridian (a
//! pole at each end of it), and cones whose side is a ruled spline patch
//! with its apex as a collapsed border. Each is built with the poles on
//! borders of constant v and, transposed, of constant u.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Line, NurbsSurface, Plane, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId, check, mass_properties};
use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, PI, TAU};

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

/// The direction away from the axis at `angle` about the frame's z.
fn radial(frame: &Frame, angle: f64) -> Vec3 {
    frame.vector(angle.pcos(), angle.psin(), 0.0)
}

/// The direction a point at `angle` moves in when the angle grows.
fn onward(frame: &Frame, angle: f64) -> Vec3 {
    frame.vector(-angle.psin(), angle.pcos(), 0.0)
}

/// A circle of `radius` about a point of the axis at height `z`, in the
/// half-plane at `angle` through the axis; its parameter is the angle
/// from the radial direction towards the axis direction.
fn meridian_circle(frame: &Frame, angle: f64, z: f64, radius: f64) -> Curve3 {
    let centre = frame.point(0.0, 0.0, z);
    let placed = Frame::new(centre, -onward(frame, angle), radial(frame, angle)).unwrap();
    Curve3::Circle(Circle::new(placed, radius).unwrap())
}

/// The profile of a surface of revolution in the half-plane (ρ, z).
struct Profile {
    degree: usize,
    knots: Vec<f64>,
    points: Vec<(f64, f64)>,
    weights: Vec<f64>,
}

/// The meridian of a sphere of radius `r`, pole to pole: two rational
/// quadratic arcs.
fn meridian(r: f64) -> Profile {
    let s = FRAC_1_SQRT_2;
    Profile {
        degree: 2,
        knots: vec![0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
        points: vec![(0.0, -r), (r, -r), (r, 0.0), (r, r), (0.0, r)],
        weights: vec![1.0, s, 1.0, s, 1.0],
    }
}

/// Its upper half, from the equator to the north pole.
fn upper(r: f64) -> Profile {
    Profile {
        degree: 2,
        knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        points: vec![(r, 0.0), (r, r), (0.0, r)],
        weights: vec![1.0, FRAC_1_SQRT_2, 1.0],
    }
}

/// The ruling of a cone from its rim (`r`, 0) to its apex (0, `h`).
fn ruling(r: f64, h: f64) -> Profile {
    Profile {
        degree: 1,
        knots: vec![0.0, 0.0, 1.0, 1.0],
        points: vec![(r, 0.0), (0.0, h)],
        weights: vec![1.0, 1.0],
    }
}

/// The surface `profile` sweeps in `quarters` quarter turns about the
/// frame's z axis from its x axis: u around the axis over `[0, 1]`, in
/// rational quadratic arcs of a quarter turn, v along the profile.
fn swept(frame: &Frame, profile: &Profile, quarters: usize) -> NurbsSurface {
    let mut knots = vec![0.0; 3];
    for q in 1..quarters {
        knots.extend([q as f64 / quarters as f64; 2]);
    }
    knots.extend([1.0; 3]);
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for k in 0..=2 * quarters {
        let angle = k as f64 * PI / 4.0;
        let (reach, w_u) = if k % 2 == 0 {
            (1.0, 1.0)
        } else {
            (2.0_f64.sqrt(), FRAC_1_SQRT_2)
        };
        for (&(rho, z), &w_v) in profile.points.iter().zip(&profile.weights) {
            let rho = rho * reach;
            ctrl.push(frame.point(angle.pcos() * rho, angle.psin() * rho, z));
            weights.push(w_u * w_v);
        }
    }
    NurbsSurface::new(
        2,
        profile.degree,
        knots,
        profile.knots.clone(),
        2 * quarters + 1,
        profile.points.len(),
        ctrl,
        weights,
    )
    .unwrap()
}

/// The same surface with its parameters exchanged: what was a border of
/// constant v is one of constant u. Its normal points the other way.
fn transposed(surface: &NurbsSurface) -> NurbsSurface {
    let (count_u, count_v) = surface.counts();
    let (degree_u, degree_v) = surface.degrees();
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for j in 0..count_v {
        for i in 0..count_u {
            ctrl.push(surface.control_points()[i * count_v + j]);
            weights.push(surface.weights()[i * count_v + j]);
        }
    }
    NurbsSurface::new(
        degree_v,
        degree_u,
        surface.knots_v().to_vec(),
        surface.knots_u().to_vec(),
        count_v,
        count_u,
        ctrl,
        weights,
    )
    .unwrap()
}

/// A face on `surface`, or on the same surface transposed.
fn spline_face(body: &mut Body, surface: NurbsSurface, flip: bool, coedges: Vec<Coedge>) -> FaceId {
    let loop_ = body.add_loop(coedges);
    if flip {
        body.add_face(Surface::Nurbs(transposed(&surface)), true, vec![loop_])
    } else {
        body.add_face(Surface::Nurbs(surface), false, vec![loop_])
    }
}

struct Solid {
    name: String,
    body: Body,
    volume: f64,
    area: f64,
    centroid: Vec3,
}

/// The wedge of a ball of radius `r` between the half-planes at the
/// angles 0 and `quarters` quarter turns: its curved face runs from pole
/// to pole between two meridians and has no other edge.
///
/// With α the angle: V = 2r³α/3, the curved face 2r²α, two half discs
/// πr²/2 each, and the centroid 3πr/16 · sin(α/2)/(α/2) from the axis on
/// the bisector (3r/8 for half a ball).
fn ball_wedge(frame: &Frame, r: f64, quarters: usize, flip: bool) -> Solid {
    let alpha = quarters as f64 * FRAC_PI_2;
    let mut body = Body::new(BodyKind::Solid);
    let south = body.add_vertex(frame.point(0.0, 0.0, -r));
    let north = body.add_vertex(frame.point(0.0, 0.0, r));
    let up = (-FRAC_PI_2, FRAC_PI_2);
    let first = body.add_edge(meridian_circle(frame, 0.0, 0.0, r), up, south, north);
    let last = body.add_edge(meridian_circle(frame, alpha, 0.0, r), up, south, north);
    let axis = body.add_edge(
        Curve3::Line(Line::through(frame.point(0.0, 0.0, r), frame.point(0.0, 0.0, -r)).unwrap()),
        (0.0, 2.0 * r),
        north,
        south,
    );
    let curved = spline_face(
        &mut body,
        swept(frame, &meridian(r), quarters),
        flip,
        vec![fwd(last), back(first)],
    );
    let at_start = flat(
        &mut body,
        frame.origin(),
        -onward(frame, 0.0),
        vec![fwd(first), fwd(axis)],
    );
    let at_end = flat(
        &mut body,
        frame.origin(),
        onward(frame, alpha),
        vec![back(last), back(axis)],
    );
    body.add_shell(vec![curved, at_start, at_end]);
    let off = 3.0 * PI * r / 16.0 * (alpha / 2.0).psin() / (alpha / 2.0);
    Solid {
        name: format!("wedge of {quarters} quarters, flipped {flip}"),
        body,
        volume: 2.0 * r.ppowi(3) * alpha / 3.0,
        area: 2.0 * r * r * alpha + PI * r * r,
        centroid: frame.origin() + radial(frame, alpha / 2.0) * off,
    }
}

/// The eighth of a ball with x, y and z positive in the frame: the curved
/// face is bounded by a quarter of the equator and two meridians that
/// meet in the north pole. On the whole meridian's surface (`whole`: the
/// surface has a second pole that the face stays clear of) or on the
/// upper half's.
///
/// V = πr³/6, the curved face πr²/2, three quarter discs πr²/4 each, the
/// centroid 3r/8 along each axis.
fn ball_octant(frame: &Frame, r: f64, whole: bool, flip: bool) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let centre = body.add_vertex(frame.origin());
    let a = body.add_vertex(frame.point(r, 0.0, 0.0));
    let b = body.add_vertex(frame.point(0.0, r, 0.0));
    let pole = body.add_vertex(frame.point(0.0, 0.0, r));
    let quarter = (0.0, FRAC_PI_2);
    let equator = body.add_edge(
        Curve3::Circle(Circle::new(*frame, r).unwrap()),
        quarter,
        a,
        b,
    );
    let a_up = body.add_edge(meridian_circle(frame, 0.0, 0.0, r), quarter, a, pole);
    let b_up = body.add_edge(meridian_circle(frame, FRAC_PI_2, 0.0, r), quarter, b, pole);
    let mut spoke = |to: Vec3, end: VertexId| {
        body.add_edge(
            Curve3::Line(Line::through(frame.origin(), to).unwrap()),
            (0.0, r),
            centre,
            end,
        )
    };
    let to_a = spoke(frame.point(r, 0.0, 0.0), a);
    let to_b = spoke(frame.point(0.0, r, 0.0), b);
    let to_pole = spoke(frame.point(0.0, 0.0, r), pole);
    let profile = if whole { meridian(r) } else { upper(r) };
    let curved = spline_face(
        &mut body,
        swept(frame, &profile, 1),
        flip,
        vec![fwd(equator), fwd(b_up), back(a_up)],
    );
    let floor = flat(
        &mut body,
        frame.origin(),
        -frame.z(),
        vec![fwd(to_b), back(equator), back(to_a)],
    );
    let behind = flat(
        &mut body,
        frame.origin(),
        -frame.x(),
        vec![back(b_up), back(to_b), fwd(to_pole)],
    );
    let beside = flat(
        &mut body,
        frame.origin(),
        -frame.y(),
        vec![fwd(a_up), back(to_pole), fwd(to_a)],
    );
    body.add_shell(vec![curved, floor, behind, beside]);
    let c = 3.0 * r / 8.0;
    Solid {
        name: format!("octant, whole meridian {whole}, flipped {flip}"),
        body,
        volume: PI * r.ppowi(3) / 6.0,
        area: PI * r * r / 2.0 + 3.0 * PI * r * r / 4.0,
        centroid: frame.point(c, c, c),
    }
}

/// A cone of radius `r` and height `h` whose side is two ruled spline
/// patches of half a turn each, every one with its apex as a collapsed
/// border and bounded by two rulings and half the rim.
///
/// V = πr²h/3, the side πr√(r² + h²), the base πr², the centroid h/4 up
/// the axis.
fn ruled_cone(frame: &Frame, r: f64, h: f64, flip: bool) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let apex = body.add_vertex(frame.point(0.0, 0.0, h));
    let east = body.add_vertex(frame.point(r, 0.0, 0.0));
    let west = body.add_vertex(frame.point(-r, 0.0, 0.0));
    let slant = r.phypot(h);
    let mut up = |from: Vec3, start: VertexId| {
        body.add_edge(
            Curve3::Line(Line::through(from, frame.point(0.0, 0.0, h)).unwrap()),
            (0.0, slant),
            start,
            apex,
        )
    };
    let east_up = up(frame.point(r, 0.0, 0.0), east);
    let west_up = up(frame.point(-r, 0.0, 0.0), west);
    let rim = Curve3::Circle(Circle::new(*frame, r).unwrap());
    let front = body.add_edge(rim.clone(), (0.0, PI), east, west);
    let rear = body.add_edge(rim, (PI, TAU), west, east);
    let turned = Frame::new(frame.origin(), frame.z(), -frame.x()).unwrap();
    let first = spline_face(
        &mut body,
        swept(frame, &ruling(r, h), 2),
        flip,
        vec![fwd(front), fwd(west_up), back(east_up)],
    );
    let second = spline_face(
        &mut body,
        swept(&turned, &ruling(r, h), 2),
        flip,
        vec![fwd(rear), fwd(east_up), back(west_up)],
    );
    let base = flat(
        &mut body,
        frame.origin(),
        -frame.z(),
        vec![back(front), back(rear)],
    );
    body.add_shell(vec![first, second, base]);
    Solid {
        name: format!("ruled cone, flipped {flip}"),
        body,
        volume: PI * r * r * h / 3.0,
        area: PI * r * slant + PI * r * r,
        centroid: frame.point(0.0, 0.0, h / 4.0),
    }
}

/// The cap of a ball of radius `r` that a plane cuts off whose circle
/// passes through the north pole, and, with `gamma` a quarter turn,
/// through the south pole too (half the ball): one closed edge that
/// passes through the poles on its way, on a surface that goes all the
/// way around its axis.
///
/// With h = r (1 − cos γ): V = πh² (3r − h) / 3, the cap 2πrh, the disc
/// π (r sin γ)², the centroid 3 (2r − h)² / (4 (3r − h)) from the centre.
fn cap_through_the_pole(frame: &Frame, r: f64, gamma: f64, flip: bool) -> Solid {
    // The cap looks along −x, tilted up by a quarter turn less `gamma`:
    // half a turn from the seam of the surface.
    let towards = frame.vector(-gamma.psin(), 0.0, gamma.pcos());
    let centre = frame.origin() + towards * (r * gamma.pcos());
    let pole = frame.point(0.0, 0.0, r);
    // The pole a quarter of the way round the circle, not at its vertex.
    let placed = Frame::new(centre, towards, (pole - centre).cross(towards)).unwrap();
    let circle = Curve3::Circle(Circle::new(placed, r * gamma.psin()).unwrap());
    assert!(circle.eval(FRAC_PI_2).distance(pole) < 1e-9 * r);
    let mut body = Body::new(BodyKind::Solid);
    let vertex = body.add_vertex(circle.eval(0.0));
    let rim = body.add_edge(circle, (0.0, TAU), vertex, vertex);
    let curved = spline_face(
        &mut body,
        swept(frame, &meridian(r), 4),
        flip,
        vec![fwd(rim)],
    );
    let disc = flat(&mut body, centre, -towards, vec![back(rim)]);
    body.add_shell(vec![curved, disc]);
    let h = r * (1.0 - gamma.pcos());
    let off = 3.0 * (2.0 * r - h).ppowi(2) / (4.0 * (3.0 * r - h));
    Solid {
        name: format!("cap of {gamma} through the pole, flipped {flip}"),
        body,
        volume: PI * h * h * (3.0 * r - h) / 3.0,
        area: TAU * r * h + PI * (r * gamma.psin()).ppowi(2),
        centroid: frame.origin() + towards * off,
    }
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

/// Checks a solid against its closed forms to 1e-9 and gives how far off
/// it was: volume, area, centroid (as a share of `size`).
fn held(solid: &Solid, k: f64, size: f64) -> [f64; 3] {
    let what = format!("{} at scale {k}", solid.name);
    let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
    assert_eq!(check(&solid.body, &tolerance), vec![], "{what}");
    let mass = mass_properties(&solid.body).unwrap_or_else(|e| panic!("{what}: {e}"));
    let off = [
        (mass.volume - solid.volume).abs() / solid.volume,
        (mass.area - solid.area).abs() / solid.area,
        mass.centroid.unwrap().distance(solid.centroid) / size,
    ];
    assert!(
        off.iter().all(|&off| off <= 1e-9),
        "{what}: volume {} vs {}, area {} vs {}, centroid {:?} vs {:?}: {off:?}",
        mass.volume,
        solid.volume,
        mass.area,
        solid.area,
        mass.centroid,
        solid.centroid
    );
    off
}

fn worst(all: &mut [f64; 3], off: [f64; 3]) {
    for (all, off) in all.iter_mut().zip(off) {
        *all = all.max(off);
    }
}

/// A boundary that ends in one pole: octants of a ball and cones with a
/// spline side, the pole on a border of constant v and of constant u.
#[test]
fn faces_through_one_pole_match_their_closed_forms() {
    let mut all = [0.0; 3];
    for k in [1e-3, 1.0, 1e6] {
        for frame in frames(k) {
            let r = 2.0 * k;
            for flip in [false, true] {
                for whole in [false, true] {
                    worst(&mut all, held(&ball_octant(&frame, r, whole, flip), k, r));
                }
                worst(&mut all, held(&ruled_cone(&frame, r, 3.0 * k, flip), k, r));
                worst(&mut all, held(&ruled_cone(&frame, r, 0.4 * k, flip), k, r));
            }
        }
    }
    println!("worst relative error: volume, area, centroid {all:?}");
}

/// A boundary that passes through both poles of its surface, with no
/// edge between them on either side: wedges of a ball.
#[test]
fn faces_from_pole_to_pole_match_their_closed_forms() {
    let mut all = [0.0; 3];
    for k in [1e-3, 1.0, 1e6] {
        for frame in frames(k) {
            let r = 2.0 * k;
            for flip in [false, true] {
                for quarters in [1, 2, 3] {
                    worst(&mut all, held(&ball_wedge(&frame, r, quarters, flip), k, r));
                }
            }
        }
    }
    println!("worst relative error: volume, area, centroid {all:?}");
}

/// An edge that passes through a pole on its way, and through both.
#[test]
fn an_edge_through_a_pole_on_its_way_is_cut_there() {
    let mut all = [0.0; 3];
    for k in [1e-3, 1.0, 1e6] {
        for frame in frames(k) {
            let r = 2.0 * k;
            for flip in [false, true] {
                for gamma in [0.6, 1.1, FRAC_PI_2] {
                    worst(
                        &mut all,
                        held(&cap_through_the_pole(&frame, r, gamma, flip), k, r),
                    );
                }
            }
        }
    }
    println!("worst relative error: volume, area, centroid {all:?}");
}
