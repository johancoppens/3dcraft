//! Mass properties do not depend on how a surface is parametrised.
//!
//! A sphere is the same surface whichever way its poles point, and a cone
//! the same wherever along its axis its frame sits and whichever way that
//! frame looks. The bodies here are built by hand from planes and spheres
//! (or cones) placed in space; the frame of each sphere is then turned so
//! that a pole lies well inside a face, well outside it, on its boundary,
//! or within 1e-12 to 1e-2 of the boundary on either side. Every frame
//! must give the closed form, which is stated at each builder.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Cone, Curve3, Ellipse, Line, Plane, Sphere, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId, check, mass_properties};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

struct Solid {
    name: String,
    body: Body,
    volume: f64,
    area: f64,
    centroid: Vec3,
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

/// The direction away from the frame's axis at `angle` about it.
fn radial(frame: &Frame, angle: f64) -> Vec3 {
    frame.vector(angle.pcos(), angle.psin(), 0.0)
}

/// The direction a point at `angle` moves in when the angle grows.
fn onward(frame: &Frame, angle: f64) -> Vec3 {
    frame.vector(-angle.psin(), angle.pcos(), 0.0)
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

fn flat(body: &mut Body, origin: Vec3, normal: Vec3, coedges: Vec<Coedge>) -> FaceId {
    let loop_ = body.add_loop(coedges);
    let frame = Frame::from_z(origin, normal).unwrap();
    body.add_face(Surface::Plane(Plane::new(frame)), false, vec![loop_])
}

/// A whole circle about the axis of `place` (a frame at the sphere's
/// centre), `along` that axis from the centre, as an edge with one vertex.
fn parallel(body: &mut Body, place: &Frame, along: f64, radius: f64) -> EdgeId {
    let frame = place.at(place.point(0.0, 0.0, along)).unwrap();
    let vertex = body.add_vertex(frame.point(radius, 0.0, 0.0));
    body.add_edge(
        Curve3::Circle(Circle::new(frame, radius).unwrap()),
        (0.0, TAU),
        vertex,
        vertex,
    )
}

/// The same solid inside out: every face looks the other way and every
/// loop runs the other way. Its volume is the negative, its area the same.
fn inside_out(body: &Body) -> Body {
    let mut out = body.clone();
    for face in &mut out.faces {
        face.reversed = !face.reversed;
    }
    for loop_ in &mut out.loops {
        loop_.coedges.reverse();
        for coedge in &mut loop_.coedges {
            coedge.reversed = !coedge.reversed;
        }
    }
    out
}

/// A sphere of `radius` about the origin of `place` whose north pole is
/// `tilt` away from the axis of `place`, towards the direction at angle
/// `towards` about it, and whose longitude 0 is turned by `spin`.
fn turned_sphere(place: &Frame, radius: f64, tilt: f64, towards: f64, spin: f64) -> Surface {
    let pole = place.z() * tilt.pcos() + radial(place, towards) * tilt.psin();
    let across = onward(place, towards);
    let third = pole.cross(across);
    let frame = Frame::new(
        place.origin(),
        pole,
        across * spin.pcos() + third * spin.psin(),
    )
    .unwrap();
    Surface::Sphere(Sphere::new(frame, radius).unwrap())
}

/// The part of a ball beyond the plane `level` along the axis of `place`
/// (`keep_cap`), or the rest of the ball.
///
/// A cap of height h = r − level: V = πh²(3r − h)/3, curved area 2πrh,
/// centroid 3(2r − h)² / (4(3r − h)) from the centre.
fn cut_ball(place: &Frame, sphere: Surface, radius: f64, level: f64, keep_cap: bool) -> Solid {
    let rim_radius = (radius * radius - level * level).sqrt();
    let mut body = Body::new(BodyKind::Solid);
    let rim = parallel(&mut body, place, level, rim_radius);
    let (on_sphere, on_disc, normal) = if keep_cap {
        (fwd(rim), back(rim), -place.z())
    } else {
        (back(rim), fwd(rim), place.z())
    };
    let curved = body.add_loop(vec![on_sphere]);
    let curved = body.add_face(sphere, false, vec![curved]);
    let disc = flat(
        &mut body,
        place.point(0.0, 0.0, level),
        normal,
        vec![on_disc],
    );
    body.add_shell(vec![curved, disc]);
    let h = radius - level;
    let cap_volume = PI * h * h * (3.0 * radius - h) / 3.0;
    let cap_centre = 3.0 * (2.0 * radius - h).ppowi(2) / (4.0 * (3.0 * radius - h));
    let ball = 4.0 * PI * radius.ppowi(3) / 3.0;
    let disc_area = PI * rim_radius * rim_radius;
    let (volume, area, z) = if keep_cap {
        (cap_volume, TAU * radius * h + disc_area, cap_centre)
    } else {
        let rest = ball - cap_volume;
        (
            rest,
            4.0 * PI * radius * radius - TAU * radius * h + disc_area,
            -cap_volume * cap_centre / rest,
        )
    };
    Solid {
        name: format!("ball cut at {level}, cap kept: {keep_cap}"),
        body,
        volume,
        area,
        centroid: place.point(0.0, 0.0, z),
    }
}

/// The slice of a ball between two parallel planes.
///
/// V = π ∫ (r² − z²) dz, the zone's area is 2πr (high − low) (Archimedes),
/// the centroid is at ∫ z (r² − z²) dz / ∫ (r² − z²) dz on the axis.
fn zone(place: &Frame, sphere: Surface, radius: f64, low: f64, high: f64) -> Solid {
    let rim = |z: f64| (radius * radius - z * z).sqrt();
    let mut body = Body::new(BodyKind::Solid);
    let bottom = parallel(&mut body, place, low, rim(low));
    let top = parallel(&mut body, place, high, rim(high));
    let loops = vec![
        body.add_loop(vec![fwd(bottom)]),
        body.add_loop(vec![back(top)]),
    ];
    let curved = body.add_face(sphere, false, loops);
    let floor = flat(
        &mut body,
        place.point(0.0, 0.0, low),
        -place.z(),
        vec![back(bottom)],
    );
    let lid = flat(
        &mut body,
        place.point(0.0, 0.0, high),
        place.z(),
        vec![fwd(top)],
    );
    body.add_shell(vec![curved, floor, lid]);
    let r2 = radius * radius;
    let slab = |z: f64| r2 * z - z.ppowi(3) / 3.0;
    let moment = |z: f64| r2 * z * z / 2.0 - z.ppowi(4) / 4.0;
    let volume = PI * (slab(high) - slab(low));
    Solid {
        name: format!("zone {low}..{high}"),
        body,
        volume,
        area: TAU * radius * (high - low) + PI * (rim(low).ppowi(2) + rim(high).ppowi(2)),
        centroid: place.point(0.0, 0.0, PI * (moment(high) - moment(low)) / volume),
    }
}

/// A wedge of a ball between two half-planes through the axis of `place`,
/// at angles 0 and `sweep`: its curved face is a lune whose corners are
/// where that axis leaves the ball.
///
/// V = sweep 2r³/3, the lune is 2 sweep r², each flat side half a disc.
/// A half disc has ∫ρ dA = 2r³/3 and ∫ρ² dA = πr⁴/8, so a thin slice has
/// its centroid 3πr/16 from the axis; over the sweep that is times
/// sin(sweep/2) / (sweep/2).
fn wedge(place: &Frame, sphere: Surface, radius: f64, sweep: f64) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let south = body.add_vertex(place.point(0.0, 0.0, -radius));
    let north = body.add_vertex(place.point(0.0, 0.0, radius));
    let meridian = |angle: f64| {
        let frame =
            Frame::new(place.origin(), -onward(place, angle), radial(place, angle)).unwrap();
        Curve3::Circle(Circle::new(frame, radius).unwrap())
    };
    let quarter = (-FRAC_PI_2, FRAC_PI_2);
    let m0 = body.add_edge(meridian(0.0), quarter, south, north);
    let m1 = body.add_edge(meridian(sweep), quarter, south, north);
    let axis = line(&mut body, (south, north));
    let lune = body.add_loop(vec![fwd(m1), back(m0)]);
    let lune = body.add_face(sphere, false, vec![lune]);
    let start = flat(
        &mut body,
        place.origin(),
        -onward(place, 0.0),
        vec![fwd(m0), back(axis)],
    );
    let end = flat(
        &mut body,
        place.origin(),
        onward(place, sweep),
        vec![fwd(axis), back(m1)],
    );
    body.add_shell(vec![lune, start, end]);
    let reach = 3.0 * PI * radius / 16.0 * (sweep / 2.0).psin() / (sweep / 2.0);
    Solid {
        name: format!("wedge {sweep}"),
        body,
        volume: sweep * 2.0 * radius.ppowi(3) / 3.0,
        area: 2.0 * sweep * radius * radius + PI * radius * radius,
        centroid: place.origin() + radial(place, sweep / 2.0) * reach,
    }
}

/// What two balls share: the one of radius `r1` about the origin of
/// `place`, the other of radius `r2` at `apart` along its axis.
///
/// The spheres meet in a circle at d = (apart² + r1² − r2²) / (2 apart)
/// from the first centre; the lens is the cap of the first ball beyond it
/// and the cap of the second ball before it, each with the formulas at
/// [`cut_ball`].
fn lens(place: &Frame, spheres: (Surface, Surface), r1: f64, r2: f64, apart: f64) -> Solid {
    let d = (apart * apart + r1 * r1 - r2 * r2) / (2.0 * apart);
    let rim_radius = (r1 * r1 - d * d).sqrt();
    let mut body = Body::new(BodyKind::Solid);
    let rim = parallel(&mut body, place, d, rim_radius);
    let first = body.add_loop(vec![fwd(rim)]);
    let first = body.add_face(spheres.0, false, vec![first]);
    let second = body.add_loop(vec![back(rim)]);
    let second = body.add_face(spheres.1, false, vec![second]);
    body.add_shell(vec![first, second]);
    let (h1, h2) = (r1 - d, r2 - (apart - d));
    let cap = |r: f64, h: f64| PI * h * h * (3.0 * r - h) / 3.0;
    let centre = |r: f64, h: f64| 3.0 * (2.0 * r - h).ppowi(2) / (4.0 * (3.0 * r - h));
    let (v1, v2) = (cap(r1, h1), cap(r2, h2));
    let z = (v1 * centre(r1, h1) + v2 * (apart - centre(r2, h2))) / (v1 + v2);
    Solid {
        name: format!("lens of {r1} and {r2}, {apart} apart"),
        body,
        volume: v1 + v2,
        area: TAU * (r1 * h1 + r2 * h2),
        centroid: place.point(0.0, 0.0, z),
    }
}

/// Where the pole of a sphere is put, as an angle from the axis of the
/// cut, for a boundary circle `rim` (an angle too) away from that axis:
/// on the axis, well inside the circle, close to it on either side, on
/// it, well outside, and the same for the other pole.
fn tilts(rim: f64) -> Vec<f64> {
    let mut all = vec![0.0, rim / 3.0, (rim + PI) / 2.0, PI - rim / 3.0, PI];
    for near in [rim, PI - rim] {
        all.push(near);
        for off in [1e-12, 1e-9, 1e-7, 3e-6, 1e-4, 1e-2, 0.2] {
            all.push(near - off);
            all.push(near + off);
        }
    }
    all
}

/// Somewhere that is not the origin, on axes that are not the world's.
fn places(scale: f64) -> [Frame; 2] {
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

/// The worst relative errors seen, and every case that was off or refused.
#[derive(Default)]
struct Tally {
    count: usize,
    volume: f64,
    area: f64,
    centroid: f64,
    wrong: Vec<String>,
}

impl Tally {
    /// Holds `solid` to its closed form, as it is and inside out.
    fn hold(&mut self, solid: &Solid, what: &str, scale: f64) {
        let tolerance = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
        assert_eq!(
            check(&solid.body, &tolerance),
            vec![],
            "{what}: {}",
            solid.name
        );
        for (body, sign) in [(solid.body.clone(), 1.0), (inside_out(&solid.body), -1.0)] {
            self.count += 1;
            let what = format!(
                "{} {what}{}",
                solid.name,
                if sign < 0.0 { ", inside out" } else { "" }
            );
            let mass = match mass_properties(&body) {
                Ok(mass) => mass,
                Err(error) => {
                    self.wrong.push(format!("{what}: refused: {error}"));
                    continue;
                }
            };
            // 1e-11 relative: Gauss–Legendre sums of smooth integrands are
            // exact to rounding; a few digits go to the tilted frames.
            let volume = (mass.volume * sign - solid.volume).abs() / solid.volume;
            let area = (mass.area - solid.area).abs() / solid.area;
            let centroid = mass
                .centroid
                .map_or(f64::INFINITY, |c| c.distance(solid.centroid) / scale);
            self.volume = self.volume.max(volume);
            self.area = self.area.max(area);
            self.centroid = self.centroid.max(centroid);
            // `!(x <= bound)` so that a NaN counts as wrong.
            if !(volume <= 1e-11 && area <= 1e-11 && centroid <= 1e-10) {
                self.wrong.push(format!(
                    "{what}: volume {:e} vs {:e} ({volume:.1e}), area {:e} vs {:e} ({area:.1e}), \
                     centroid off by {centroid:.1e}",
                    mass.volume * sign,
                    solid.volume,
                    mass.area,
                    solid.area
                ));
            }
        }
    }

    fn finish(self, what: &str) {
        println!(
            "{what}: {} bodies; worst relative error: volume {:e}, area {:e}, centroid {:e}; {} wrong",
            self.count,
            self.volume,
            self.area,
            self.centroid,
            self.wrong.len()
        );
        assert!(
            self.wrong.is_empty(),
            "{what}: {} of {} bodies wrong or refused, the first of them:\n{}",
            self.wrong.len(),
            self.count,
            self.wrong
                .iter()
                .take(25)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

#[test]
fn a_cut_ball_is_the_same_in_every_frame_of_its_sphere() {
    let mut tally = Tally::default();
    for scale in SCALES {
        for place in places(scale) {
            let radius = 2.0 * scale;
            for level in [-1.6, -0.3, 0.0, 1.1, 1.9] {
                let rim = (level / 2.0_f64).pacos();
                for tilt in tilts(rim) {
                    for (towards, spin) in [(0.0, 0.0), (2.1, 1.3), (4.0, PI)] {
                        let what = format!(
                            "pole {tilt} from the axis ({:e} from the rim), towards {towards}, \
                             spin {spin}, scale {scale}",
                            tilt - rim
                        );
                        for keep_cap in [true, false] {
                            let sphere = turned_sphere(&place, radius, tilt, towards, spin);
                            let solid = cut_ball(&place, sphere, radius, level * scale, keep_cap);
                            tally.hold(&solid, &what, scale);
                        }
                    }
                }
            }
        }
    }
    tally.finish("cut balls");
}

#[test]
fn a_zone_is_the_same_in_every_frame_of_its_sphere() {
    let mut tally = Tally::default();
    for scale in SCALES {
        for place in places(scale) {
            let radius = 2.0 * scale;
            for (low, high) in [(-0.5, 1.2), (0.3, 1.9), (-1.7, -0.2), (-1.99, 1.99)] {
                // Poles about either rim, between the two (in the face),
                // and inside either hole.
                let mut all = tilts((high / 2.0_f64).pacos());
                all.extend(tilts((low / 2.0_f64).pacos()));
                for tilt in all {
                    for (towards, spin) in [(0.0, 0.0), (2.1, 1.3)] {
                        let what =
                            format!("pole {tilt} from the axis, towards {towards}, scale {scale}");
                        let sphere = turned_sphere(&place, radius, tilt, towards, spin);
                        let solid = zone(&place, sphere, radius, low * scale, high * scale);
                        tally.hold(&solid, &what, scale);
                    }
                }
            }
        }
    }
    tally.finish("zones");
}

/// Where the pole of a wedge's sphere is put: `(tilt, towards)` as in
/// [`turned_sphere`], relative to the axis the two flat sides share.
fn wedge_poles(sweep: f64) -> Vec<(f64, f64)> {
    let middle = sweep / 2.0;
    // At a corner of the lune, in the middle of it, opposite the middle,
    // in the middle of either edge.
    let mut all = vec![
        (0.0, 0.0),
        (PI, 0.0),
        (FRAC_PI_2, middle),
        (FRAC_PI_2, middle + PI),
        (FRAC_PI_2, 0.0),
        (FRAC_PI_2, sweep),
        (1.0, middle),
        (2.5, sweep + 1.0),
    ];
    for off in [1e-12, 1e-9, 1e-7, 3e-6, 1e-4, 1e-2, 0.2] {
        // Near a corner: inside the lune, outside it, along an edge.
        for towards in [middle, middle + PI, 0.0, sweep] {
            all.push((off, towards));
            all.push((PI - off, towards));
        }
        // Near the middle of an edge, on either side of it.
        for edge in [0.0, sweep] {
            all.push((FRAC_PI_2, edge - off));
            all.push((FRAC_PI_2, edge + off));
            all.push((0.4, edge + off));
        }
    }
    all
}

#[test]
fn a_wedge_is_the_same_in_every_frame_of_its_sphere() {
    let mut tally = Tally::default();
    for scale in SCALES {
        for place in places(scale) {
            let radius = 2.0 * scale;
            for sweep in [0.2, FRAC_PI_2, PI, 4.0, 6.1] {
                for (tilt, towards) in wedge_poles(sweep) {
                    for spin in [0.0, 1.3] {
                        let what = format!(
                            "pole {tilt:e} from the corner, towards {towards:e}, spin {spin}, \
                             scale {scale}"
                        );
                        let sphere = turned_sphere(&place, radius, tilt, towards, spin);
                        let solid = wedge(&place, sphere, radius, sweep);
                        tally.hold(&solid, &what, scale);
                    }
                }
            }
        }
    }
    tally.finish("wedges");
}

#[test]
fn a_lens_is_the_same_in_every_frame_of_its_spheres() {
    let mut tally = Tally::default();
    for scale in SCALES {
        for place in places(scale) {
            for (r1, r2, apart) in [(2.0, 2.0, 3.0), (2.0, 1.0, 2.2), (2.0, 0.7, 1.6)] {
                let (r1, r2, apart) = (r1 * scale, r2 * scale, apart * scale);
                let d = (apart * apart + r1 * r1 - r2 * r2) / (2.0 * apart);
                let other = place.at(place.point(0.0, 0.0, apart)).unwrap();
                // The rim as each sphere sees it from its own centre.
                let rim1 = (d / r1).pacos();
                let rim2 = ((d - apart) / r2).pacos();
                let (tilts1, tilts2) = (tilts(rim1), tilts(rim2));
                for (k, &tilt1) in tilts1.iter().enumerate() {
                    // Every tilt of the one against a different tilt of
                    // the other.
                    let tilt2 = tilts2[(k * 7 + 3) % tilts2.len()];
                    let what = format!(
                        "poles {tilt1} and {tilt2} from the axis ({:e} and {:e} from the rim), \
                         scale {scale}",
                        tilt1 - rim1,
                        tilt2 - rim2
                    );
                    let spheres = (
                        turned_sphere(&place, r1, tilt1, 0.7, 0.4),
                        turned_sphere(&other, r2, tilt2, 3.9, 2.0),
                    );
                    let solid = lens(&place, spheres, r1, r2, apart);
                    tally.hold(&solid, &what, scale);
                }
            }
        }
    }
    tally.finish("lenses");
}

/// The tip of a cone cut off by a plane that is not square to the axis:
/// the cone face holds the apex and its boundary is an ellipse around it.
///
/// With the apex at the origin of `place`, the half-angle `alpha` and the
/// plane through the axis at `height`, leaning by `lean`: the two rulings
/// in the plane of the lean meet the plane at t₁ = p cos τ / cos(α − τ)
/// and t₂ = p cos τ / cos(α + τ) from the apex; the ellipse has those two
/// points as the ends of its major axis and b² = t₁ t₂ sin²α. A cone over
/// a base of area E at distance q has V = E q / 3 and its centroid a
/// quarter of the way from the base's centre to the apex. Every piece of
/// the cone face is sin α times as large seen along the axis, and so is
/// the ellipse times cos τ: the cone face is E cos τ / sin α.
///
/// `shift` and `flipped` change the description of the same cone, not the
/// cone: where along the axis its frame sits, and whether the frame looks
/// along the axis or against it.
fn cone_tip(place: &Frame, alpha: f64, height: f64, lean: f64, shift: f64, flipped: bool) -> Solid {
    let ruling = |side: f64| {
        let t = height * lean.pcos() / (alpha - side * lean).pcos();
        place.point(side * t * alpha.psin(), 0.0, t * alpha.pcos())
    };
    let (p1, p2) = (ruling(1.0), ruling(-1.0));
    let apex = place.origin();
    let t1 = p1.distance(apex);
    let t2 = p2.distance(apex);
    let (a, b) = (p1.distance(p2) / 2.0, (t1 * t2).sqrt() * alpha.psin());
    let centre = p1.lerp(p2, 0.5);
    let normal = place.vector(lean.psin(), 0.0, lean.pcos());
    let on_plane = Frame::new(centre, normal, p1 - p2).unwrap();
    let frame = if flipped {
        place.reversed_at(place.point(0.0, 0.0, shift)).unwrap()
    } else {
        place.at(place.point(0.0, 0.0, shift)).unwrap()
    };
    let cone = Cone::new(
        frame,
        shift * alpha.ptan(),
        if flipped { -alpha } else { alpha },
    )
    .unwrap();
    let mut body = Body::new(BodyKind::Solid);
    let vertex = body.add_vertex(p1);
    let rim = body.add_edge(
        Curve3::Ellipse(Ellipse::new(on_plane, a, b).unwrap()),
        (0.0, TAU),
        vertex,
        vertex,
    );
    let side = body.add_loop(vec![back(rim)]);
    let side = body.add_face(Surface::Cone(cone), false, vec![side]);
    let lid = flat(&mut body, centre, normal, vec![fwd(rim)]);
    body.add_shell(vec![side, lid]);
    let ellipse = PI * a * b;
    Solid {
        name: format!(
            "cone tip, half-angle {alpha}, lean {lean}, frame {shift} along, flipped: {flipped}"
        ),
        body,
        volume: ellipse * height * lean.pcos() / 3.0,
        area: ellipse * lean.pcos() / alpha.psin() + ellipse,
        centroid: apex + (centre - apex) * 0.75,
    }
}

/// A slice of a cone between two half-planes through its axis, with the
/// tip cut off at the share `cut` of its height: a cone face that comes
/// within `cut` of the apex without reaching it (or reaches it, for 0).
///
/// A slice of a whole cone (base r, height h) has V = sweep r²h/6, the
/// cone face sweep r s/2 with s the slant height, each flat side r h/2,
/// and its centroid, for a thin slice, r/2 from the axis and h/4 up. The
/// tip is the same shape, `cut` times as large.
fn cone_slice(
    place: &Frame,
    radius: f64,
    height: f64,
    sweep: f64,
    cut: f64,
    shift: f64,
    flipped: bool,
) -> Solid {
    let mut body = Body::new(BodyKind::Solid);
    let low = place.origin();
    let high = place.point(0.0, 0.0, height * (1.0 - cut));
    let apex = place.point(0.0, 0.0, height);
    let rim = |centre: Vec3, r: f64, angle: f64| centre + radial(place, angle) * r;
    let c = body.add_vertex(low);
    let a0 = body.add_vertex(rim(low, radius, 0.0));
    let a1 = body.add_vertex(rim(low, radius, sweep));
    let arc = |body: &mut Body, centre: Vec3, r: f64, ends: (VertexId, VertexId)| {
        body.add_edge(
            Curve3::Circle(Circle::new(place.at(centre).unwrap(), r).unwrap()),
            (0.0, sweep),
            ends.0,
            ends.1,
        )
    };
    let bottom = arc(&mut body, low, radius, (a0, a1));
    let spoke0 = line(&mut body, (c, a0));
    let spoke1 = line(&mut body, (c, a1));
    // The cone as seen from a frame `shift` up the axis.
    let slope = radius / height;
    let (frame, half_angle) = if flipped {
        (
            place.reversed_at(place.point(0.0, 0.0, shift)).unwrap(),
            slope.patan(),
        )
    } else {
        (
            place.at(place.point(0.0, 0.0, shift)).unwrap(),
            -slope.patan(),
        )
    };
    let cone = Surface::Cone(Cone::new(frame, (height - shift) * slope, half_angle).unwrap());
    let tip = if cut == 0.0 {
        let top = body.add_vertex(apex);
        let ruling0 = line(&mut body, (a0, top));
        let ruling1 = line(&mut body, (a1, top));
        let axis = line(&mut body, (c, top));
        let side = body.add_loop(vec![fwd(bottom), fwd(ruling1), back(ruling0)]);
        let side = body.add_face(cone, false, vec![side]);
        let start = flat(
            &mut body,
            low,
            -onward(place, 0.0),
            vec![fwd(spoke0), fwd(ruling0), back(axis)],
        );
        let end = flat(
            &mut body,
            low,
            onward(place, sweep),
            vec![fwd(axis), back(ruling1), back(spoke1)],
        );
        vec![side, start, end]
    } else {
        let t = body.add_vertex(high);
        let b0 = body.add_vertex(rim(high, radius * cut, 0.0));
        let b1 = body.add_vertex(rim(high, radius * cut, sweep));
        let top = arc(&mut body, high, radius * cut, (b0, b1));
        let ruling0 = line(&mut body, (a0, b0));
        let ruling1 = line(&mut body, (a1, b1));
        let top0 = line(&mut body, (t, b0));
        let top1 = line(&mut body, (t, b1));
        let axis = line(&mut body, (c, t));
        let side = body.add_loop(vec![fwd(bottom), fwd(ruling1), back(top), back(ruling0)]);
        let side = body.add_face(cone, false, vec![side]);
        let lid = flat(
            &mut body,
            high,
            place.z(),
            vec![fwd(top0), fwd(top), back(top1)],
        );
        let start = flat(
            &mut body,
            low,
            -onward(place, 0.0),
            vec![fwd(spoke0), fwd(ruling0), back(top0), back(axis)],
        );
        let end = flat(
            &mut body,
            low,
            onward(place, sweep),
            vec![fwd(axis), fwd(top1), back(ruling1), back(spoke1)],
        );
        vec![side, lid, start, end]
    };
    let base = flat(
        &mut body,
        low,
        -place.z(),
        vec![fwd(spoke1), back(bottom), back(spoke0)],
    );
    let mut faces = tip;
    faces.push(base);
    body.add_shell(faces);

    let slant = radius.phypot(height);
    let chord = (sweep / 2.0).psin() / (sweep / 2.0);
    // The whole slice less the tip, which is `cut` times as large and
    // starts `1 − cut` of the height up.
    let whole = sweep * radius * radius * height / 6.0;
    let tip_volume = whole * cut.ppowi(3);
    let volume = whole - tip_volume;
    let out = (whole * radius / 2.0 - tip_volume * cut * radius / 2.0) * chord / volume;
    let up = (whole * height / 4.0 - tip_volume * height * (1.0 - cut + cut / 4.0)) / volume;
    Solid {
        name: format!(
            "cone slice {sweep}, tip cut at {cut:e}, frame {shift} along, flipped: {flipped}"
        ),
        body,
        volume,
        area: (sweep * radius * slant / 2.0 + radius * height) * (1.0 - cut * cut)
            + sweep * radius * radius / 2.0 * (1.0 + cut * cut),
        centroid: low + radial(place, sweep / 2.0) * out + place.z() * up,
    }
}

#[test]
fn a_cone_is_the_same_wherever_its_frame_sits() {
    let mut tally = Tally::default();
    for scale in SCALES {
        for place in places(scale) {
            for (alpha, lean) in [(0.5, 0.0), (0.5, 0.4), (0.5, 1.0), (1.2, 0.3), (0.1, 1.4)] {
                for (shift, flipped) in [(0.0, false), (1.5, false), (40.0, true), (0.3, true)] {
                    let solid = cone_tip(&place, alpha, 2.0 * scale, lean, shift * scale, flipped);
                    tally.hold(&solid, &format!("scale {scale}"), scale);
                }
            }
            for sweep in [0.2, FRAC_PI_2, 4.0, 6.1] {
                for cut in [0.0, 1e-12, 1e-9, 1e-7, 3e-6, 1e-4, 1e-2, 0.5] {
                    for (shift, flipped) in [(0.0, false), (2.0, true), (-30.0, false)] {
                        let solid = cone_slice(
                            &place,
                            1.5 * scale,
                            2.5 * scale,
                            sweep,
                            cut,
                            shift * scale,
                            flipped,
                        );
                        tally.hold(&solid, &format!("scale {scale}"), scale);
                    }
                }
            }
        }
    }
    tally.finish("cones");
}

/// A cone so slender that its apex is a million radii away: the face is a
/// band far from the apex, and what is measured must be the band, not the
/// difference of two cones a million times its size.
///
/// A frustum: V = πh (r₁² + r₁r₂ + r₂²) / 3, the wall π (r₁ + r₂) s with
/// s the slant height.
#[test]
fn a_band_far_from_the_apex_is_measured_as_a_band() {
    for scale in SCALES {
        for place in places(scale) {
            for half_angle in [1e-6_f64, -1e-6, 1e-3, 0.3] {
                let (r1, h) = (2.0 * scale, 3.0 * scale);
                let r2 = r1 + h * half_angle.ptan();
                let cone = Cone::new(place, r1, half_angle).unwrap();
                let mut body = Body::new(BodyKind::Solid);
                let bottom = parallel(&mut body, &place, 0.0, r1);
                let top = parallel(&mut body, &place, h, r2);
                let wall = vec![
                    body.add_loop(vec![fwd(bottom)]),
                    body.add_loop(vec![back(top)]),
                ];
                let wall = body.add_face(Surface::Cone(cone), false, wall);
                let floor = flat(&mut body, place.origin(), -place.z(), vec![back(bottom)]);
                let lid = flat(
                    &mut body,
                    place.point(0.0, 0.0, h),
                    place.z(),
                    vec![fwd(top)],
                );
                body.add_shell(vec![wall, floor, lid]);
                let mass = mass_properties(&body).unwrap();
                let volume = PI * h * (r1 * r1 + r1 * r2 + r2 * r2) / 3.0;
                let area = PI * (r1 + r2) * h.phypot(r2 - r1) + PI * (r1 * r1 + r2 * r2);
                let what = format!("half-angle {half_angle}, scale {scale}");
                assert!(
                    (mass.volume - volume).abs() <= 1e-11 * volume,
                    "{what}: volume {:e} vs {volume:e}",
                    mass.volume
                );
                assert!(
                    (mass.area - area).abs() <= 1e-11 * area,
                    "{what}: area {:e} vs {area:e}",
                    mass.area
                );
            }
        }
    }
}

/// A face that lies outside its own loops (they run clockwise around what
/// they enclose) has no area to give: an error, not a negative number.
#[test]
fn a_face_outside_its_loops_is_refused() {
    use craft3d_topo::MassError;
    let place = places(1.0)[1];
    // A disc whose rim runs the wrong way, and the wall of a cone tip
    // whose rim does.
    let mut disc = Body::new(BodyKind::Sheet);
    let rim = parallel(&mut disc, &place, 0.0, 2.0);
    let face = flat(&mut disc, place.origin(), place.z(), vec![back(rim)]);
    disc.add_shell(vec![face]);
    assert_eq!(
        mass_properties(&disc),
        Err(MassError::UnboundedFace { face: 0 })
    );

    let mut tip = cone_tip(&place, 0.5, 2.0, 0.4, 0.0, false).body;
    tip.kind = BodyKind::Sheet;
    let wall = tip.faces[0].loops[0].0 as usize;
    tip.loops[wall].coedges[0].reversed = false;
    assert_eq!(
        mass_properties(&tip),
        Err(MassError::UnboundedFace { face: 0 })
    );
}

/// A body a thousand million times smaller than it is far from the
/// origin is known to seven digits: its coordinates hold no more. It is
/// measured to those, not refused because its sums do not agree to
/// twelve.
#[test]
fn a_small_body_far_from_the_origin_is_measured_to_what_its_coordinates_hold() {
    let far = Frame::new(
        Vec3::new(700.0, -300.0, 500.0),
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap();
    let radius = 1e-6;
    for level in [-0.5e-6, 0.3e-6] {
        let sphere = turned_sphere(&far, radius, 0.4, 1.0, 0.3);
        let solid = cut_ball(&far, sphere, radius, level, true);
        let mass = mass_properties(&solid.body).unwrap();
        // Rounding of the coordinates: 1e-16 × 1e3 / 1e-6 = 1e-7 of the
        // size, a hundred times that allowed for.
        assert!(
            (mass.area - solid.area).abs() <= 1e-5 * solid.area,
            "{} vs {}",
            mass.area,
            solid.area
        );
        // Its volume is the small difference of the large numbers the
        // faces give, each good to 1e-7 of |position| × area.
        let slack = 1e-5 * 1e3 * solid.area;
        assert!(
            (mass.volume - solid.volume).abs() <= slack,
            "{} vs {}",
            mass.volume,
            solid.volume
        );
    }
}
