//! Oracles: closed forms for a box, a sphere (the spherical cap), a
//! horizontal cylinder (the circular segment), a V-shaped prism and a
//! spline cone cut by a plane; the section's principal moments of a turned
//! rectangle; the offset for a volume; invariance under a rigid motion; at
//! three scales with the tolerance scaled along; a section with a hole,
//! a body with two section faces, a thin tilted rod cut far from the
//! middle of its box, and a level search that lands on a flat face.

use super::*;
use crate::boolean::{BooleanOp, boolean};
use crate::primitives::{block, cylinder, sphere};
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
