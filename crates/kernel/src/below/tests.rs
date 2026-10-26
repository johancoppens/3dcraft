//! Oracles: closed forms for a box, a sphere (the spherical cap), a
//! horizontal cylinder (the circular segment), a V-shaped prism and a
//! spline cone cut by a plane; the section's principal moments of a turned
//! rectangle; the offset for a volume; invariance under a rigid motion; at
//! three scales with the tolerance scaled along. A few tests derive from
//! these the quantities a floating-body Jig computes (BM = I/V, GM, the
//! wall-sided righting arm of Tupper, *Introduction to Naval
//! Architecture*), as a check that the general measures carry them.

use super::*;
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

/// A box barge L × B × D, centred on the z axis, its bottom at z = 0.
fn barge(k: f64, (l, b, d): (f64, f64, f64)) -> Body {
    let frame = Frame::new(
        Vec3::new(-0.5 * l * k, -0.5 * b * k, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .unwrap();
    block(&frame, l * k, b * k, d * k).unwrap()
}

#[test]
fn box_barge_matches_its_closed_forms() {
    for k in SCALES {
        let (l, b, t) = (4.0 * k, 1.0 * k, 0.3 * k);
        let body = barge(k, (4.0, 1.0, 1.0));
        let im = below(&[&body], &upright(), t, &tol(k)).unwrap();
        close(im.volume, l * b * t, k * k * k, "volume");
        let at = im.centroid.unwrap();
        close(at.z, t / 2.0, k, "KB = T/2");
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
        close(wp.i_x / im.volume, b * b / (12.0 * t), k, "BM = B²/(12 T)");
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
            // The metacentre of a sphere is its centre at every draft.
            let km = im.centroid.unwrap().z + im.section.i_x / im.volume;
            close(km, 0.0, k, "KM of a sphere is R");
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
                "I_T",
            );
            close(im.section.i_y, 2.0 * c * l.powi(3) / 12.0, k.powi(4), "I_L");
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
/// 90°, so ∇ = L T², KB = 2T/3, BM = 2T/3 (tan 45° = 1).
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
        close(im.centroid.unwrap().z, 2.0 * t / 3.0, k, "KB");
        close(im.section.area, 2.0 * t * l, k * k, "A");
        close(im.section.i_x / im.volume, 2.0 * t / 3.0, k, "BM");
    }
}

/// The same barge moved and turned in space, measured in a plane turned
/// with it, gives the same numbers.
#[test]
fn results_do_not_depend_on_where_the_body_lies() {
    for k in SCALES {
        let body = barge(k, (4.0, 1.0, 1.0));
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
        close(im.section.i_x, reference.section.i_x, k.powi(4), "I_T");
        close(im.section.i_y, reference.section.i_y, k.powi(4), "I_L");
        close(im.surface, reference.surface, k * k, "surface below");
    }
}

/// A wall-sided box heeled while its deck edge stays dry and its bilge
/// under: GZ = sin φ (GM + BM tan²φ / 2) (Tupper).
#[test]
fn a_heeled_box_follows_the_wall_sided_formula() {
    for k in SCALES {
        let (b, t, kg) = (k, 0.4 * k, 0.45 * k);
        let body = barge(k, (4.0, 1.0, 1.0));
        let volume = 4.0 * k * b * t;
        let up = level_for(&[&body], &upright(), volume, &tol(k)).unwrap();
        let bm = up.below.section.i_x / up.below.volume;
        let gm = t / 2.0 + bm - kg;
        close(bm, b * b / (12.0 * t), k, "BM");
        let g = Vec3::new(0.0, 0.0, kg);
        for degrees in [5.0_f64, 10.0, -10.0] {
            let phi = degrees.to_radians();
            let plane = turned(&upright(), phi);
            let heeled = level_for(&[&body], &plane, volume, &tol(k)).unwrap();
            let at = heeled.below.centroid.unwrap();
            let gz = (at - g).dot(plane.normal.cross(plane.x_axis));
            let want = phi.sin() * (gm + 0.5 * bm * phi.tan().powi(2));
            close(gz, want, k, "GZ");
        }
    }
}

/// GM changes sign at B/D = √(6 s (1 − s)) for a box of uniform relative
/// density s floating upright with G at its middle.
#[test]
fn gm_of_a_box_changes_sign_at_its_stability_limit() {
    let s = 0.5_f64;
    let limit = (6.0 * s * (1.0 - s)).sqrt();
    for (ratio, stable) in [(limit * 1.02, true), (limit * 0.98, false)] {
        let body = barge(1.0, (4.0, ratio, 1.0));
        let volume = 4.0 * ratio * s;
        let found = level_for(&[&body], &upright(), volume, &tol(1.0)).unwrap();
        close(found.below.offset, s, 1.0, "draft");
        let im = found.below;
        let gm = im.centroid.unwrap().z + im.section.i_x / im.volume - 0.5;
        assert_eq!(gm > 0.0, stable, "B/D {ratio}: GM {gm}");
    }
}

#[test]
fn what_cannot_be_cut_is_said() {
    let body = barge(1.0, (4.0, 1.0, 1.0));
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

/// Two bodies  are summed: twice one hull, the section's
/// second moment about the middle by the parallel-axis rule.
#[test]
fn two_hulls_sum_and_move_their_moments() {
    let hull = |y: f64| {
        let frame = Frame::new(
            Vec3::new(-2.0, y - 0.25, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap();
        block(&frame, 4.0, 0.5, 1.0).unwrap()
    };
    let (a, b) = (hull(-1.0), hull(1.0));
    let im = below(&[&a, &b], &upright(), 0.2, &tol(1.0)).unwrap();
    close(im.volume, 2.0 * 4.0 * 0.5 * 0.2, 1.0, "V");
    let one = 4.0 * 0.5_f64.powi(3) / 12.0;
    close(im.section.i_x, 2.0 * (one + 4.0 * 0.5 * 1.0), 1.0, "I_T");
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
        within(im.centroid.unwrap().z, 0.75 * h, 1, "KB = 3h/4");
        within(im.section.area, PI * r * r, 2, "section");
        within(im.section.i_x, PI * r.powi(4) / 4.0, 4, "I_T");
        within(im.section.i_y, PI * r.powi(4) / 4.0, 4, "I_L");
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
