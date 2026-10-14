//! Faces bounded by nothing but a seam walked there and back, as a STEP
//! file gives a whole sphere or a whole torus: one loop that runs along
//! each seam edge once each way. Such a face is its whole surface; its
//! boundary integral alone cannot say so (it is zero, or the whole surface,
//! to rounding), so `mass_properties` recognises the loop and measures the
//! whole surface.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Circle, Curve3, Sphere, Surface, Torus};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, check, mass_properties};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// A sphere of radius `r` about `centre` whose one loop walks the meridian
/// at longitude 0 from the south pole to the north pole and back.
fn sphere_with_seam(centre: Vec3, r: f64) -> Body {
    let mut body = Body::new(BodyKind::Solid);
    let south = body.add_vertex(centre - Vec3::Z * r);
    let north = body.add_vertex(centre + Vec3::Z * r);
    // In the xz plane: r (cos t x + sin t z).
    let frame = Frame::new(centre, -Vec3::Y, Vec3::X).unwrap();
    let meridian = Curve3::Circle(Circle::new(frame, r).unwrap());
    let edge = body.add_edge(meridian, (-FRAC_PI_2, FRAC_PI_2), south, north);
    let seam = body.add_loop(vec![
        Coedge {
            edge,
            reversed: false,
        },
        Coedge {
            edge,
            reversed: true,
        },
    ]);
    let surface =
        Surface::Sphere(Sphere::new(Frame::new(centre, Vec3::Z, Vec3::X).unwrap(), r).unwrap());
    let face = body.add_face(surface, false, vec![seam]);
    body.add_shell(vec![face]);
    body
}

/// A torus whose one loop walks its two seams, the circle around the axis
/// and the circle around the tube through the same vertex, each there and
/// back.
fn torus_with_seams(centre: Vec3, major: f64, minor: f64) -> Body {
    let mut body = Body::new(BodyKind::Solid);
    let vertex = body.add_vertex(centre + Vec3::X * (major + minor));
    let around_axis = Frame::new(centre, Vec3::Z, Vec3::X).unwrap();
    let tube = Frame::new(centre + Vec3::X * major, -Vec3::Y, Vec3::X).unwrap();
    let long = body.add_edge(
        Curve3::Circle(Circle::new(around_axis, major + minor).unwrap()),
        (0.0, TAU),
        vertex,
        vertex,
    );
    let short = body.add_edge(
        Curve3::Circle(Circle::new(tube, minor).unwrap()),
        (0.0, TAU),
        vertex,
        vertex,
    );
    let use_of = |edge, reversed| Coedge { edge, reversed };
    let seams = body.add_loop(vec![
        use_of(long, false),
        use_of(short, false),
        use_of(long, true),
        use_of(short, true),
    ]);
    let surface = Surface::Torus(Torus::new(around_axis, major, minor).unwrap());
    let face = body.add_face(surface, false, vec![seams]);
    body.add_shell(vec![face]);
    body
}

fn close(measured: f64, exact: f64, what: &str) {
    assert!(
        (measured - exact).abs() <= 1e-12 * exact.abs(),
        "{what}: {measured} for {exact}"
    );
}

#[test]
fn a_sphere_bounded_by_its_seam_is_the_whole_sphere() {
    for k in [1e-3, 1.0, 1e6] {
        let centre = Vec3::new(0.3, -0.2, 0.7) * k;
        let body = sphere_with_seam(centre, k);
        assert_eq!(check(&body, &Tolerance::default()), vec![]);
        let m = mass_properties(&body).unwrap();
        let what = format!("sphere of radius {k}");
        close(m.area, 4.0 * PI * k * k, &what);
        close(m.volume, 4.0 / 3.0 * PI * k * k * k, &what);
        assert!(m.centroid.unwrap().distance(centre) <= 1e-12 * k, "{what}");
    }
}

#[test]
fn a_torus_bounded_by_its_seams_is_the_whole_torus() {
    for k in [1e-3, 1.0, 1e6] {
        let (major, minor) = (3.0 * k, k);
        let centre = Vec3::new(-0.4, 0.1, 0.5) * k;
        let body = torus_with_seams(centre, major, minor);
        assert_eq!(check(&body, &Tolerance::default()), vec![]);
        let m = mass_properties(&body).unwrap();
        let what = format!("torus of size {k}");
        close(m.area, 4.0 * PI * PI * major * minor, &what);
        close(m.volume, 2.0 * PI * PI * major * minor * minor, &what);
        assert!(m.centroid.unwrap().distance(centre) <= 1e-12 * k, "{what}");
    }
}
