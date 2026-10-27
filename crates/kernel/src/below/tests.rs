//! Oracles: closed forms for a box, a sphere (the spherical cap), a
//! horizontal cylinder (the circular segment), a V-shaped prism and a
//! spline cone cut by a plane; the section's principal moments of a turned
//! rectangle; the offset for a volume; invariance under a rigid motion; at
//! three scales with the tolerance scaled along; a section with a hole,
//! a body with two section faces, a thin tilted rod cut far from the
//! middle of its box, and a level search that lands on a flat face.

use super::*;
use crate::boolean::{BooleanOp, boolean};
use crate::primitives::{block, cylinder, sphere, torus};
use craft3d_math::Transform;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tol(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

#[track_caller]
fn close(got: f64, want: f64, scale: f64, what: &str) {
    let allowed = 1e-9 * want.abs().max(scale);
    assert!(
        (got - want).abs() <= allowed,
        "{what}: got {got}, want {want} (allowed {allowed})"
    );
}

/// The plane turned by `angle` about its x axis (Rodrigues; the x axis
/// is square to the normal).
fn turned(plane: &CutPlane, angle: f64) -> CutPlane {
    let (s, c) = angle.sin_cos();
    CutPlane::new(
        plane.normal * c + plane.x_axis.cross(plane.normal) * s,
        plane.x_axis,
    )
    .unwrap()
}

fn upright() -> CutPlane {
    CutPlane::new(Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)).unwrap()
}

/// A box L × B × D, centred on the z axis, its bottom at z = 0.
fn boxed(k: f64, (l, b, d): (f64, f64, f64)) -> Body {
    let frame = Frame::new(
        Vec3::new(-0.5 * l * k, -0.5 * b * k, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .unwrap();
    block(&frame, l * k, b * k, d * k).unwrap()
}

#[test]
fn a_box_matches_its_closed_forms() {
    for k in SCALES {
        let (l, b, t) = (4.0 * k, 1.0 * k, 0.3 * k);
        let body = boxed(k, (4.0, 1.0, 1.0));
        let im = below(&[&body], &upright(), t, &tol(k)).unwrap();
        close(im.volume, l * b * t, k * k * k, "volume");
        let at = im.centroid.unwrap();
        close(at.z, t / 2.0, k, "centroid T/2");
        close(at.x, 0.0, k, "LCB");
        close(at.y, 0.0, k, "TCB");
        let wp = im.section;
        close(wp.area, l * b, k * k, "section area");
        close(wp.i_x, l * b * b * b / 12.0, k.powi(4), "I_T = L B³/12");
        close(wp.i_y, b * l * l * l / 12.0, k.powi(4), "I_L = B L³/12");
        close(wp.product, 0.0, k.powi(4), "product");
        close(wp.centroid.unwrap().z, t, k, "section in the plane");
        close(
            im.surface,
            l * b + 2.0 * (l + b) * t,
            k * k,
            "surface below",
        );
        close(
            wp.i_x / im.volume,
            b * b / (12.0 * t),
            k,
            "ix / V = B²/(12 T)",
        );
        // Found again from its volume: the offset T = V / (L B).
        let found = level_for(&[&body], &upright(), l * b * t, &tol(k)).unwrap();
        close(found.below.offset, t, k, "offset");
        assert!(found.steps <= 8, "{} steps", found.steps);
    }
}

#[test]
fn a_sphere_matches_the_spherical_cap() {
    for k in SCALES {
        let r = k;
        let body = sphere(&Frame::WORLD, r).unwrap();
        for h in [-0.5 * k, 0.0, 0.3 * k] {
            let im = below(&[&body], &upright(), h, &tol(k)).unwrap();
            let s = h + r;
            close(
                im.volume,
                PI * s * s * (3.0 * r - s) / 3.0,
                k.powi(3),
                "cap volume",
            );
            let d = 3.0 * (2.0 * r - s).powi(2) / (4.0 * (3.0 * r - s));
            close(im.centroid.unwrap().z, -d, k, "cap centroid");
            let a2 = r * r - h * h;
            close(im.section.area, PI * a2, k * k, "section circle");
            close(im.section.i_x, PI * a2 * a2 / 4.0, k.powi(4), "I");
            close(im.section.i_y, PI * a2 * a2 / 4.0, k.powi(4), "I");
            close(im.surface, 2.0 * PI * r * s, k * k, "cap area");
            // For a sphere the centroid height plus ix / V is its centre at every level.
            let km = im.centroid.unwrap().z + im.section.i_x / im.volume;
            close(km, 0.0, k, "centroid height + ix / V is the centre");
        }
    }
}

#[test]
fn a_horizontal_cylinder_matches_the_circular_segment() {
    for k in SCALES {
        let (r, l) = (k, 3.0 * k);
        let frame = Frame::new(
            Vec3::new(-0.5 * l, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
        .unwrap();
        let body = cylinder(&frame, r, l).unwrap();
        for h in [-0.2 * k, 0.4 * k] {
            let im = below(&[&body], &upright(), h, &tol(k)).unwrap();
            let theta = 2.0 * (-h / r).acos();
            let segment = 0.5 * r * r * (theta - theta.sin());
            close(im.volume, l * segment, k.powi(3), "segment volume");
            let d = 4.0 * r * (0.5 * theta).sin().powi(3) / (3.0 * (theta - theta.sin()));
            close(im.centroid.unwrap().z, -d, k, "segment centroid");
            let c = (r * r - h * h).sqrt();
            close(im.section.area, 2.0 * c * l, k * k, "section");
            close(
                im.section.i_x,
                l * (2.0 * c).powi(3) / 12.0,
                k.powi(4),
                "ix",
            );
            close(im.section.i_y, 2.0 * c * l.powi(3) / 12.0, k.powi(4), "iy");
            close(
                im.surface,
                2.0 * segment + l * r * theta,
                k * k,
                "surface below",
            );
        }
    }
}

/// A square prism standing on an edge: below its half-diagonal a V of
/// 90°, so V = L T², centroid 2T/3, ix / V = 2T/3 (tan 45° = 1).
#[test]
fn a_v_prism_matches_the_triangle() {
    for k in SCALES {
        let (l, side, t) = (2.0 * k, k, 0.4 * k);
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let frame = Frame::new(
            Vec3::new(-0.5 * l, 0.0, 0.0),
            Vec3::new(0.0, -s, s),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap();
        let body = block(&frame, l, side, side).unwrap();
        let im = below(&[&body], &upright(), t, &tol(k)).unwrap();
        close(im.volume, l * t * t, k.powi(3), "V");
        close(im.centroid.unwrap().z, 2.0 * t / 3.0, k, "centroid");
        close(im.section.area, 2.0 * t * l, k * k, "A");
        close(im.section.i_x / im.volume, 2.0 * t / 3.0, k, "ix / V");
    }
}

/// The same box moved and turned in space, measured in a plane turned
/// with it, gives the same numbers.
#[test]
fn results_do_not_depend_on_where_the_body_lies() {
    for k in SCALES {
        let body = boxed(k, (4.0, 1.0, 1.0));
        let plane = upright();
        let t = 0.3 * k;
        let reference = below(&[&body], &plane, t, &tol(k)).unwrap();
        let turn = Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 1.1).unwrap();
        let frame = Frame::new(
            turn.point(Vec3::new(-2.0 * k, -0.5 * k, 0.0)),
            turn.vector(Vec3::new(0.0, 0.0, 1.0)),
            turn.vector(Vec3::new(1.0, 0.0, 0.0)),
        )
        .unwrap();
        let moved = block(&frame, 4.0 * k, k, k).unwrap();
        let turned = CutPlane::new(
            turn.vector(Vec3::new(0.0, 0.0, 1.0)),
            turn.vector(Vec3::new(1.0, 0.0, 0.0)),
        )
        .unwrap();
        let im = below(&[&moved], &turned, t, &tol(k)).unwrap();
        close(im.volume, reference.volume, k.powi(3), "V");
        let b = im.centroid.unwrap();
        close(
            b.dot(turned.normal),
            reference.centroid.unwrap().z,
            k,
            "centroid height",
        );
        close(im.section.i_x, reference.section.i_x, k.powi(4), "ix");
        close(im.section.i_y, reference.section.i_y, k.powi(4), "iy");
        close(im.surface, reference.surface, k * k, "surface below");
    }
}

/// A box under a plane turned by φ about its x axis, with the same
/// volume below as upright (depth T) while the plane stays on its side
/// walls: the centroid below moves sideways, and its lever about a point
/// q on the box's axis at height c, (centroid − q) · (normal × x), is the
/// closed form sin φ (T/2 + r − c + r tan²φ / 2) with r = ix / V upright
/// = B²/(12 T) (the wall-sided formula; Tupper, *Introduction to Naval
/// Architecture*).
#[test]
fn a_turned_plane_moves_the_centroid_below_by_the_wall_sided_form() {
    for k in SCALES {
        let (b, t, c) = (k, 0.4 * k, 0.45 * k);
        let body = boxed(k, (4.0, 1.0, 1.0));
        let volume = 4.0 * k * b * t;
        let upright_level = level_for(&[&body], &upright(), volume, &tol(k)).unwrap();
        let r = upright_level.below.section.i_x / upright_level.below.volume;
        close(r, b * b / (12.0 * t), k, "ix / V = B²/(12 T)");
        let q = Vec3::new(0.0, 0.0, c);
        for degrees in [5.0_f64, 10.0, -10.0] {
            let phi = degrees.to_radians();
            let plane = turned(&upright(), phi);
            let found = level_for(&[&body], &plane, volume, &tol(k)).unwrap();
            let at = found.below.centroid.unwrap();
            let lever = (at - q).dot(plane.normal.cross(plane.x_axis));
            let want = phi.sin() * (t / 2.0 + r - c + 0.5 * r * phi.tan().powi(2));
            close(lever, want, k, "the centroid's lever");
        }
    }
}

#[test]
fn what_cannot_be_cut_is_said() {
    let body = boxed(1.0, (4.0, 1.0, 1.0));
    let plane = upright();
    let t = &tol(1.0);
    assert!(matches!(
        level_for(&[&body], &plane, 4.5, t),
        Err(CutError::OutOfRange { .. })
    ));
    assert!(matches!(
        level_for(&[&body], &plane, 0.0, t),
        Err(CutError::OutOfRange { .. })
    ));
    assert!(matches!(
        level_for(&[&body], &plane, f64::NAN, t),
        Err(CutError::NotFinite(_))
    ));
    assert!(matches!(
        level_for(&[], &plane, 1.0, t),
        Err(CutError::NoBodies)
    ));
    assert!(matches!(
        below(&[&body], &plane, f64::INFINITY, t),
        Err(CutError::NotFinite(_))
    ));
    let mut sheet = body.clone();
    sheet.kind = BodyKind::Sheet;
    assert!(matches!(
        below(&[&sheet], &plane, 0.5, t),
        Err(CutError::NotSolid { .. })
    ));
    assert!(CutPlane::new(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)).is_err());
    assert!(CutPlane::new(Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 1.0)).is_err());
    // Above and below the body: nothing, and all of it.
    let none = below(&[&body], &plane, -1.0, t).unwrap();
    assert_eq!(none.volume, 0.0);
    assert!(none.centroid.is_none());
    let all = below(&[&body], &plane, 2.0, t).unwrap();
    close(all.volume, 4.0, 1.0, "all of it");
    assert_eq!(all.section.area, 0.0);
}

/// Two bodies  are summed: twice one, the section's
/// second moment about the middle by the parallel-axis rule.
#[test]
fn two_bodies_sum_and_move_their_moments() {
    let part = |y: f64| {
        let frame = Frame::new(
            Vec3::new(-2.0, y - 0.25, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap();
        block(&frame, 4.0, 0.5, 1.0).unwrap()
    };
    let (a, b) = (part(-1.0), part(1.0));
    let im = below(&[&a, &b], &upright(), 0.2, &tol(1.0)).unwrap();
    close(im.volume, 2.0 * 4.0 * 0.5 * 0.2, 1.0, "V");
    let one = 4.0 * 0.5_f64.powi(3) / 12.0;
    close(im.section.i_x, 2.0 * (one + 4.0 * 0.5 * 1.0), 1.0, "ix");
    close(im.section.centroid.unwrap().y, 0.0, 1.0, "centre");
}

/// A freeform body: the cone a cubic spline sweeps about an axis (a
/// spline surface), apex down. The section is bounded by the curve in
/// which the plane meets the spline surface, which lies on both within
/// the modelling tolerance: so the closed forms hold to that tolerance
/// times the size to the integral's power less one.
#[test]
fn a_spline_cone_apex_down_matches_the_cone() {
    for k in SCALES {
        let (r_top, height, h) = (0.5 * k, k, 0.6 * k);
        let frame = Frame::new(
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
        .unwrap();
        let body = craft3d_testkit::solids::spline_cone(&frame, r_top, height)
            .unwrap()
            .body;
        let t = tol(k);
        let im = below(&[&body], &upright(), h, &t).unwrap();
        let r = r_top * h / height;
        let within = |got: f64, want: f64, power: i32, what: &str| {
            let allowed = 1e-9 * want.abs() + 20.0 * t.linear * k.powi(power - 1);
            assert!(
                (got - want).abs() <= allowed,
                "{what} at scale {k}: got {got}, want {want} (allowed {allowed})"
            );
        };
        within(im.volume, PI * r * r * h / 3.0, 3, "volume");
        within(im.centroid.unwrap().z, 0.75 * h, 1, "centroid 3h/4");
        within(im.section.area, PI * r * r, 2, "section");
        within(im.section.i_x, PI * r.powi(4) / 4.0, 4, "ix");
        within(im.section.i_y, PI * r.powi(4) / 4.0, 4, "iy");
        within(im.surface, PI * r * r.hypot(h), 2, "surface below");
        // Found again from its own volume.
        let found = level_for(&[&body], &upright(), im.volume, &t).unwrap();
        within(found.below.offset, h, 1, "offset");
    }
}

/// The section's principal moments: a rectangle w × h turned by an angle
/// in the plane has i_x, i_y and a product that mix, and gives its
/// principal moments h w³/12 and w h³/12 back (w > h), the major one about
/// the axis square to the long side.
#[test]
fn a_turned_rectangle_gives_its_principal_moments_back() {
    for k in SCALES {
        let (w, h, angle) = (3.0 * k, k, 0.4_f64);
        let (s, c) = angle.sin_cos();
        let long = Vec3::new(c, s, 0.0);
        let frame = Frame::new(Vec3::new(0.0, 0.0, -k), Vec3::new(0.0, 0.0, 1.0), long).unwrap();
        let body = block(&frame, w, h, 2.0 * k).unwrap();
        let plane = upright();
        let im = below(&[&body], &plane, 0.0, &tol(k)).unwrap();
        assert!(
            im.section.product.abs() > 1e-3 * k.powi(4),
            "the product mixes"
        );
        let (major, minor, axis) = im.section.principal(&plane).unwrap();
        close(major, h * w.powi(3) / 12.0, k.powi(4), "major");
        close(minor, w * h.powi(3) / 12.0, k.powi(4), "minor");
        close(
            axis.dot(long).abs(),
            0.0,
            1.0,
            "major axis square to the long side",
        );
        close(im.section.area, w * h, k * k, "area");
    }
}

/// A thin rod (radius r, length 1000 r) tilted by α from the plane's
/// normal and cut near its lower end, far from the middle of its box: the
/// section is an ellipse with semi-axes r / cos α (along x) and r, and its
/// moments are π a b³/4 and π a³ b/4, to 1e-9 of themselves although they
/// are a ten-billionth of the rod's size to the fourth power.
#[test]
fn a_thin_tilted_rod_cut_far_from_its_middle_keeps_its_section_moments() {
    for k in SCALES {
        let (r, l, alpha) = (k, 1000.0 * k, 0.5_f64);
        let axis = Vec3::new(alpha.sin(), 0.0, alpha.cos());
        let frame = Frame::new(Vec3::ZERO, axis, Vec3::new(0.0, 1.0, 0.0)).unwrap();
        let body = cylinder(&frame, r, l).unwrap();
        let im = below(&[&body], &upright(), 5.0 * k, &tol(k)).unwrap();
        let (a, b) = (r / alpha.cos(), r);
        let s = im.section;
        close(s.area, PI * a * b, k * k, "ellipse area");
        let rel = |got: f64, want: f64, what: &str| {
            assert!(
                (got - want).abs() <= 1e-9 * want,
                "{what} at scale {k}: got {got}, want {want}"
            );
        };
        rel(s.i_x, PI * a * b.powi(3) / 4.0, "ix");
        rel(s.i_y, PI * a.powi(3) * b / 4.0, "iy");
        assert!(s.product.abs() <= 1e-9 * s.i_y, "ixy {}", s.product);
    }
}

/// A tube (a cylinder less a coaxial one) standing on the plane's normal:
/// its section is an annulus, the inner loop a hole, A = π (R² − r²) and
/// ix = iy = π (R⁴ − r⁴) / 4.
#[test]
fn a_section_with_a_hole_takes_its_inner_loop_off() {
    for k in SCALES {
        let (big, small, h) = (2.0 * k, k, 3.0 * k);
        let outer = cylinder(&Frame::WORLD, big, h).unwrap();
        let low = Frame::new(
            Vec3::new(0.0, 0.0, -k),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap();
        let inner = cylinder(&low, small, h + 2.0 * k).unwrap();
        let tube = boolean(&outer, &inner, BooleanOp::Subtract, &tol(k))
            .unwrap()
            .body;
        let im = below(&[&tube], &upright(), 1.5 * k, &tol(k)).unwrap();
        close(
            im.section.area,
            PI * (big * big - small * small),
            k * k,
            "annulus",
        );
        let i = PI * (big.powi(4) - small.powi(4)) / 4.0;
        close(im.section.i_x, i, k.powi(4), "ix");
        close(im.section.i_y, i, k.powi(4), "iy");
        close(
            im.volume,
            PI * (big * big - small * small) * 1.5 * k,
            k.powi(3),
            "volume",
        );
    }
}

/// A U (a block with a slot cut from its top) cut above the slot's
/// floor: one body, two section faces, each w × d at ±(gap + w)/2.
#[test]
fn one_body_with_two_section_faces_sums_them() {
    for k in SCALES {
        let (w, gap, d) = (k, 2.0 * k, 3.0 * k);
        let width = 2.0 * w + gap;
        let at = |x: f64, y: f64, z: f64| {
            Frame::new(
                Vec3::new(x, y, z),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 0.0),
            )
            .unwrap()
        };
        let solid = block(&at(-width / 2.0, -d / 2.0, 0.0), width, d, 3.0 * k).unwrap();
        let slot = block(&at(-gap / 2.0, -d, k), gap, 2.0 * d, 3.0 * k).unwrap();
        let u = boolean(&solid, &slot, BooleanOp::Subtract, &tol(k))
            .unwrap()
            .body;
        let im = below(&[&u], &upright(), 2.0 * k, &tol(k)).unwrap();
        close(im.section.area, 2.0 * w * d, k * k, "two rectangles");
        let off = 0.5 * (gap + w);
        close(
            im.section.i_y,
            2.0 * (d * w.powi(3) / 12.0 + w * d * off * off),
            k.powi(4),
            "iy by the parallel-axis rule",
        );
        close(im.section.i_x, 2.0 * w * d.powi(3) / 12.0, k.powi(4), "ix");
        close(im.section.centroid.unwrap().x, 0.0, k, "centred");
    }
}

/// A stepped block (2 × 1 × 1 with 1 × 1 × 1 on top): the level for a
/// volume of 2 is the shoulder, where Newton's first step lands exactly
/// and the plane lies on the step's flat face (coincident faces, which the
/// boolean handles); and the measure at exactly that offset.
#[test]
fn a_level_on_a_flat_face_of_the_body_is_found() {
    for k in SCALES {
        let at = |x: f64, z: f64| {
            Frame::new(
                Vec3::new(x, 0.0, z),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 0.0),
            )
            .unwrap()
        };
        let base = block(&at(0.0, 0.0), 2.0 * k, k, k).unwrap();
        let top = block(&at(0.0, k), k, k, k).unwrap();
        let stepped = boolean(&base, &top, BooleanOp::Union, &tol(k))
            .unwrap()
            .body;
        let found = level_for(&[&stepped], &upright(), 2.0 * k.powi(3), &tol(k)).unwrap();
        close(found.below.offset, k, k, "the shoulder");
        let at_shoulder = below(&[&stepped], &upright(), k, &tol(k)).unwrap();
        close(
            at_shoulder.volume,
            2.0 * k.powi(3),
            k.powi(3),
            "volume at the shoulder",
        );
        close(
            at_shoulder.section.area,
            2.0 * k * k,
            k * k,
            "the section at the shoulder",
        );
    }
}

/// A sphere cut along an oblique normal: its extent along it is the
/// sphere's own (from the held box of the body turned onto the normal,
/// not the corners of its axis-aligned box), so the tangent plane meets
/// nothing and a plane a little inside it cuts the spherical cap.
#[test]
fn an_oblique_tangent_plane_to_a_sphere_cuts_nothing() {
    for k in SCALES {
        let r = k;
        let body = sphere(&Frame::WORLD, r).unwrap();
        let plane = CutPlane::new(Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.0, -1.0, 0.0)).unwrap();
        let (lo, hi, _) = extent(&[&body], plane.normal).unwrap();
        close(lo, -r, k, "the sphere's own extent");
        close(hi, r, k, "the sphere's own extent");
        let none = below(&[&body], &plane, -r, &tol(k)).unwrap();
        assert_eq!(none.volume, 0.0);
        let s = 0.1 * r;
        let cap = below(&[&body], &plane, -r + s, &tol(k)).unwrap();
        close(
            cap.volume,
            PI * s * s * (3.0 * r - s) / 3.0,
            k.powi(3),
            "cap",
        );
    }
}

// ------------------------------------------------ nothing below, soundly

/// `nothing_below` for the one body `body` along `plane`.
fn nothing_at(body: &Body, plane: &CutPlane, offset: f64, k: f64) -> Result<Part, CutError> {
    let prepared = prepare(&[body], plane.normal).unwrap();
    nothing_below(&prepared[0], plane, offset, &tol(k))
}

#[track_caller]
fn lost_by(result: Result<Part, CutError>, want: f64, scale: f64, what: &str) {
    match result {
        Err(CutError::LostBelow { depth, .. }) => close(depth, want, scale, what),
        Err(other) => panic!("{what}: {other}"),
        Ok(_) => panic!("{what}: nothing below, yet the body reaches {want} below"),
    }
}

/// A sphere has no vertices, so the check that nothing found below the
/// plane is right cannot look at vertices: it is bounded by the sphere
/// itself, its lowest point `c · n − r` along the normal, at three scales
/// and along an oblique normal.
#[test]
fn a_sphere_found_empty_above_its_bottom_is_lost_by_its_depth() {
    for k in SCALES {
        let (r, t) = (k, tol(k).linear);
        let centre = Vec3::new(0.3 * k, -0.2 * k, 0.5 * k);
        let body = sphere(&Frame::new(centre, Vec3::Z, Vec3::X).unwrap(), r).unwrap();
        assert!(body.vertices.is_empty());
        for plane in [
            upright(),
            CutPlane::new(Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.0, -1.0, 0.0)).unwrap(),
        ] {
            let lowest = centre.dot(plane.normal) - r;
            lost_by(
                nothing_at(&body, &plane, lowest + 0.5 * r, k),
                0.5 * r,
                k,
                "half a radius into the sphere",
            );
            lost_by(
                nothing_at(&body, &plane, lowest + 2.0 * t, k),
                2.0 * t,
                k,
                "twice the tolerance into the sphere",
            );
            // Within the tolerance of its bottom: nothing below is right.
            let none = nothing_at(&body, &plane, lowest + 0.5 * t, k).unwrap();
            assert_eq!(none.volume, 0.0);
        }
    }
}

/// A torus has no vertices either: its lowest along a unit `n` is
/// `c · n − (R |n − (n · a) a| + r)` for its axis `a`, with the axis
/// tilted and the plane upright or oblique, at three scales.
#[test]
fn a_torus_found_empty_above_its_bottom_is_lost_by_its_depth() {
    for k in SCALES {
        let (major, minor, t) = (3.0 * k, k, tol(k).linear);
        let centre = Vec3::new(-0.4 * k, 0.7 * k, 0.2 * k);
        let theta: f64 = 0.4;
        let axis = Vec3::new(theta.sin(), 0.0, theta.cos());
        let x = Vec3::new(theta.cos(), 0.0, -theta.sin());
        let body = torus(&Frame::new(centre, axis, x).unwrap(), major, minor).unwrap();
        assert!(body.vertices.is_empty());
        for plane in [
            upright(),
            CutPlane::new(Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.0, -1.0, 0.0)).unwrap(),
        ] {
            let n = plane.normal;
            let reach = major * (n - axis * n.dot(axis)).length() + minor;
            let lowest = centre.dot(n) - reach;
            lost_by(
                nothing_at(&body, &plane, lowest + 0.25 * minor, k),
                0.25 * minor,
                k,
                "a quarter of the tube into the torus",
            );
            lost_by(
                nothing_at(&body, &plane, lowest + 2.0 * t, k),
                2.0 * t,
                k,
                "twice the tolerance into the torus",
            );
            let none = nothing_at(&body, &plane, lowest + 0.5 * t, k).unwrap();
            assert_eq!(none.volume, 0.0);
        }
    }
}

/// The volume below a plane just above the bottom of a sphere and of a
/// torus, through `below`: the spherical cap `π h² (3r − h) / 3`, and for
/// the torus (axis upright, so its bottom is a circle) the circular
/// segment of its tube, `r² acos((r − h)/r) − (r − h) √(2rh − h²)`, swept
/// about the axis at the radius of its centroid, `R` (Pappus), with the
/// section the annulus `4π R √(2rh − h²)`.
#[test]
fn a_sphere_and_a_torus_cut_near_their_bottoms_match_their_closed_forms() {
    for k in SCALES {
        let r = k;
        let ball = sphere(&Frame::WORLD, r).unwrap();
        for h in [1e-4 * k, 1e-2 * k, 0.5 * k] {
            let im = below(&[&ball], &upright(), -r + h, &tol(k)).unwrap();
            close(
                im.volume,
                PI * h * h * (3.0 * r - h) / 3.0,
                k.powi(3),
                "cap volume",
            );
            close(
                im.section.area,
                PI * (2.0 * r * h - h * h),
                k * k,
                "cap section",
            );
        }
        let (major, minor) = (3.0 * k, k);
        let ring = torus(&Frame::WORLD, major, minor).unwrap();
        for h in [1e-2 * k, 0.5 * k, 1.5 * k] {
            let im = below(&[&ring], &upright(), -minor + h, &tol(k)).unwrap();
            let half = (2.0 * minor * h - h * h).sqrt();
            let segment = minor * minor * ((minor - h) / minor).acos() - (minor - h) * half;
            close(
                im.volume,
                2.0 * PI * major * segment,
                k.powi(3),
                "torus below",
            );
            close(
                im.section.area,
                4.0 * PI * major * half,
                k * k,
                "torus section",
            );
        }
    }
}

/// A ring with a rounded bottom (a cubic spline from (2, 0) through the
/// control points (3, −2) and (4, −2) to (5, 0), closed by lines to the
/// height 1, swept about the y axis): its bottom is a circle on its spline
/// face at y = −1.5 (the spline's height is −6 t (1 − t), least at t = ½),
/// where it has no vertex, and its control net reaches −2. Nothing below
/// is right only below −1.5 (within the tolerance), and the depth it is
/// lost by above is that of the face itself, not of its net.
#[test]
fn a_revolved_spline_found_empty_above_its_bottom_is_lost_by_its_depth() {
    use crate::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    use crate::revolve::revolve;
    for k in SCALES {
        let t = tol(k).linear;
        let p = |x: f64, y: f64| [x * k, y * k];
        let mut curves = vec![SketchCurve {
            id: 1,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: vec![p(2.0, 0.0), p(3.0, -2.0), p(4.0, -2.0), p(5.0, 0.0)],
            }),
        }];
        for (id, (start, end)) in [
            (p(5.0, 0.0), p(5.0, 1.0)),
            (p(5.0, 1.0), p(2.0, 1.0)),
            (p(2.0, 1.0), p(2.0, 0.0)),
        ]
        .into_iter()
        .enumerate()
        {
            curves.push(SketchCurve {
                id: 3 + 2 * id as u64,
                curve: Curve2::Line { start, end },
            });
        }
        let profiles = detect_profiles(&curves, &tol(k)).unwrap();
        let ring = revolve(
            &profiles,
            &Frame::WORLD,
            [0.0, 0.0],
            [0.0, 1.0],
            0.0,
            2.0 * PI,
            &tol(k),
        )
        .unwrap()
        .body;
        assert!(ring.faces.iter().any(|f| f.surface.is_freeform()));
        let plane = CutPlane::new(Vec3::Y, Vec3::X).unwrap();
        let (lo, _, _) = extent(&[&ring], plane.normal).unwrap();
        assert!(lo < -1.9 * k, "the held box is the net's: {lo}");
        let bottom = -1.5 * k;
        // The face's least is closed in on to a hundredth of the
        // tolerance: the depth is a bound, no less than the true one.
        for (into, what) in [(0.1 * k, "into the ring"), (2.0 * t, "just into the ring")] {
            match nothing_at(&ring, &plane, bottom + into, k) {
                Err(CutError::LostBelow { depth, .. }) => assert!(
                    depth >= into - 1e-12 * k && depth <= into + 0.02 * t,
                    "{what} at {k}: lost by {depth}, not {into}"
                ),
                Err(other) => panic!("{what} at {k}: {other}"),
                Ok(_) => panic!("{what} at {k}: nothing below"),
            }
        }
        for offset in [bottom - 0.1 * k, bottom - 0.5 * t, bottom + 0.5 * t] {
            let none = nothing_at(&ring, &plane, offset, k).unwrap();
            assert_eq!(none.volume, 0.0);
        }
        // Through `below`, between the net and the face nothing is below.
        let none = below(&[&ring], &plane, bottom - 0.2 * k, &tol(k)).unwrap();
        assert_eq!(none.volume, 0.0);
    }
}

/// A prism whose side is a trimmed spline face whose surface reaches far
/// below it: a profile with a cubic spline side (from (0, 0) through the
/// control points (1, −1) and (2, −1) to (3, 0), closed by lines through
/// (3, 2) and (0, 2)) extruded from z = −100 to 1, then cut to z ∈ [0, 1]
/// by a block. The spline face keeps its surface, 101 high, trimmed to
/// the top 1; the held box, and so the level search's bracket, reaches
/// −100, and the search's first step lands far below the prism, where
/// nothing is below by the face's trim, not by its surface (it was
/// refused, "it may reach … below it", while the face was bound by its
/// whole surface). The level for a share s of the prism's volume is s
/// (a prism's volume grows linearly with the level), at three scales.
#[test]
fn a_trimmed_spline_face_reaching_far_below_its_trim_gives_the_level() {
    use crate::extrude::extrude;
    use crate::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    for k in SCALES {
        let t = tol(k);
        let p = |x: f64, y: f64| [x * k, y * k];
        let mut curves = vec![SketchCurve {
            id: 1,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: vec![p(0.0, 0.0), p(1.0, -1.0), p(2.0, -1.0), p(3.0, 0.0)],
            }),
        }];
        for (id, (start, end)) in [
            (p(3.0, 0.0), p(3.0, 2.0)),
            (p(3.0, 2.0), p(0.0, 2.0)),
            (p(0.0, 2.0), p(0.0, 0.0)),
        ]
        .into_iter()
        .enumerate()
        {
            curves.push(SketchCurve {
                id: 3 + 2 * id as u64,
                curve: Curve2::Line { start, end },
            });
        }
        let profiles = detect_profiles(&curves, &t).unwrap();
        let tall = extrude(&profiles, &Frame::WORLD, -100.0 * k, k, &t)
            .unwrap()
            .body;
        let cutter = block(
            &Frame::new(Vec3::new(-k, -2.0 * k, 0.0), Vec3::Z, Vec3::X).unwrap(),
            5.0 * k,
            5.0 * k,
            k,
        )
        .unwrap();
        let prism = boolean(&tall, &cutter, BooleanOp::Intersect, &t)
            .unwrap()
            .body;
        assert!(prism.faces.iter().any(|f| f.surface.is_freeform()));
        let plane = upright();
        let (lo, hi, _) = extent(&[&prism], plane.normal).unwrap();
        assert!(lo < -99.0 * k, "the held box is the surface's: {lo}");
        // Far below the prism, and between its bottom and the surface's,
        // nothing is below, by the face's trim.
        for offset in [-50.0 * k, -0.1 * k, -0.5 * t.linear] {
            let none = nothing_at(&prism, &plane, offset, k).unwrap();
            assert_eq!(none.volume, 0.0);
        }
        lost_by(
            nothing_at(&prism, &plane, 0.1 * k, k),
            0.1 * k,
            k,
            "a tenth into the prism",
        );
        let total = mass_properties(&prism).unwrap().volume;
        for share in [0.3, 0.7] {
            let found = level_for(&[&prism], &plane, share * total, &t).unwrap();
            let allowed = 1e-9 * k + 20.0 * t.linear;
            assert!(
                (found.below.offset - share * k).abs() <= allowed,
                "the level for {share} of the prism at {k}: {} (bracket {lo}..{hi}, {} steps)",
                found.below.offset,
                found.steps
            );
        }
    }
}

// ------------------------------------------------ the search for a level

/// What lies below at `offset` with this volume and section area (the
/// rest is not looked at by the search).
fn measured_at(offset: f64, volume: f64, area: f64) -> BelowPlane {
    BelowPlane {
        offset,
        volume,
        centroid: None,
        surface: 0.0,
        section: Section {
            area,
            centroid: None,
            i_x: 0.0,
            i_y: 0.0,
            product: 0.0,
        },
    }
}

fn refused(offset: f64) -> CutError {
    CutError::Kernel {
        body: 0,
        offset,
        error: KernelError::InvalidSize {
            name: "a face in the plane",
            value: offset,
        },
    }
}

/// The volume below `x` is `x²` on `[0, 1]` (section `2x`), refused by
/// the boolean on the calls `refuse` names (counted from 1): the offsets
/// asked for, and the outcome.
fn squared(volume: f64, refuse: &[usize]) -> (Vec<f64>, Result<Level, CutError>) {
    let mut asked = Vec::new();
    let found = search((0.0, 1.0), 1.0, volume, |x| {
        asked.push(x);
        if refuse.contains(&asked.len()) {
            return Err(refused(x));
        }
        Ok(measured_at(x, x * x, 2.0 * x))
    });
    (asked, found)
}

/// An offset the boolean refuses is stepped around by halving the
/// bracket instead (`level_for`'s documentation): refused at the first
/// guess (0.3 for a volume of 0.3), the search goes on from the middle of
/// the bracket (0.5) and finds √0.3; refused again at the Newton step
/// after that (0.55), it halves again (0.75) and still finds it.
#[test]
fn a_level_refused_where_a_step_lands_is_found_by_halving_instead() {
    let want = 0.3_f64.sqrt();
    let (asked, found) = squared(0.3, &[1]);
    let found = found.unwrap();
    assert_eq!(asked[..3], [0.3, 0.5, 0.55]);
    assert!((found.below.offset - want).abs() <= 1e-10, "{found:?}");
    assert!(found.residual <= SETTLED * 0.3);
    assert_eq!(found.steps, asked.len());
    let (asked, found) = squared(0.3, &[1, 3]);
    let found = found.unwrap();
    assert_eq!(asked[..5], [0.3, 0.5, 0.55, 0.75, 0.625]);
    assert!((found.below.offset - want).abs() <= 1e-10, "{found:?}");
}

/// One refused where the halving lands is the error: the halving after a
/// refused first guess, and a halving the search took because a Newton
/// step did not halve the residual (0.65, then the middle 0.475).
#[test]
fn a_level_refused_where_the_halving_lands_is_the_error() {
    let (asked, found) = squared(0.3, &[1, 2]);
    assert_eq!(asked, [0.3, 0.5]);
    assert!(
        matches!(found, Err(CutError::Kernel { offset, .. }) if offset == 0.5),
        "{found:?}"
    );
    let (asked, found) = squared(0.3, &[3]);
    assert_eq!(asked.len(), 3);
    assert!(
        (asked[1] - 0.65).abs() < 1e-12 && asked[2] == 0.475,
        "{asked:?}"
    );
    assert!(
        matches!(found, Err(CutError::Kernel { offset, .. }) if offset == 0.475),
        "{found:?}"
    );
}

/// Where the bracket cannot be narrowed (the volume jumps by `jump` at
/// 0.3, so no offset leaves half of the jump below), the offset found is
/// the answer when the volume is within `UNSETTLED` of the one asked for,
/// and "did not converge" further off.
#[test]
fn a_bracket_as_narrow_as_it_gets_is_an_answer_only_near_the_volume() {
    let jumping = |jump: f64| {
        let volume = 0.3 + 0.5 * jump;
        let found = search((0.0, 1.0), 1.0 + jump, volume, |x| {
            Ok(measured_at(x, if x < 0.3 { x } else { x + jump }, 1.0))
        });
        (volume, found)
    };
    // Half the jump is 5e-9 of the volume: past SETTLED, within UNSETTLED.
    let (volume, found) = jumping(3e-9);
    let found = found.unwrap();
    assert!(found.residual > SETTLED * volume, "{found:?}");
    assert!(found.residual <= UNSETTLED * volume, "{found:?}");
    assert!((found.below.offset - 0.3).abs() <= 1e-15, "{found:?}");
    assert!(found.steps < MAX_STEPS, "{found:?}");
    // Half the jump is 1.7e-4 of it: not converged, the bracket at 0.3.
    let (volume, found) = jumping(1e-4);
    match found {
        Err(CutError::NotConverged {
            steps,
            low,
            high,
            residual,
        }) => {
            assert!(steps < MAX_STEPS);
            assert!(
                low < 0.3 && high >= 0.3 && high - low <= 1e-15,
                "{low} {high}"
            );
            assert!(residual > UNSETTLED * volume);
        }
        other => panic!("{other:?}"),
    }
}
