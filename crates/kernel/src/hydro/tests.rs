//! Oracles: closed forms of naval architecture for a box barge, a
//! sphere, a horizontal cylinder and a V-shaped prism (Biran &
//! López-Pulido; Tupper), the wall-sided formula for the
//! righting arm of a heeled box, invariance under a rigid motion, at three
//! scales with the tolerance scaled along.

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

fn upright() -> Water {
    Water::new(Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)).unwrap()
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
        let im = immersion(&[&body], &upright(), t, &tol(k)).unwrap();
        close(im.volume, l * b * t, k * k * k, "volume");
        let at = im.buoyancy.unwrap();
        close(at.z, t / 2.0, k, "KB = T/2");
        close(at.x, 0.0, k, "LCB");
        close(at.y, 0.0, k, "TCB");
        let wp = im.waterplane;
        close(wp.area, l * b, k * k, "waterplane area");
        close(wp.i_along, l * b * b * b / 12.0, k.powi(4), "I_T = L B³/12");
        close(
            wp.i_across,
            b * l * l * l / 12.0,
            k.powi(4),
            "I_L = B L³/12",
        );
        close(wp.product, 0.0, k.powi(4), "product");
        close(wp.centroid.unwrap().z, t, k, "waterplane in the plane");
        close(im.wetted, l * b + 2.0 * (l + b) * t, k * k, "wetted");
        close(
            wp.i_along / im.volume,
            b * b / (12.0 * t),
            k,
            "BM = B²/(12 T)",
        );
        // Floated by its displacement: the waterline T = m / (ρ L B).
        let floated = float(&[&body], &upright(), l * b * t, &tol(k)).unwrap();
        close(floated.immersion.level, t, k, "waterline");
        assert!(floated.steps <= 8, "{} steps", floated.steps);
    }
}

#[test]
fn a_sphere_matches_the_spherical_cap() {
    for k in SCALES {
        let r = k;
        let body = sphere(&Frame::WORLD, r).unwrap();
        for h in [-0.5 * k, 0.0, 0.3 * k] {
            let im = immersion(&[&body], &upright(), h, &tol(k)).unwrap();
            let s = h + r;
            close(
                im.volume,
                PI * s * s * (3.0 * r - s) / 3.0,
                k.powi(3),
                "cap volume",
            );
            let d = 3.0 * (2.0 * r - s).powi(2) / (4.0 * (3.0 * r - s));
            close(im.buoyancy.unwrap().z, -d, k, "cap centroid");
            let a2 = r * r - h * h;
            close(im.waterplane.area, PI * a2, k * k, "waterplane circle");
            close(im.waterplane.i_along, PI * a2 * a2 / 4.0, k.powi(4), "I");
            close(im.waterplane.i_across, PI * a2 * a2 / 4.0, k.powi(4), "I");
            close(im.wetted, 2.0 * PI * r * s, k * k, "cap area");
            // The metacentre of a sphere is its centre at every draft.
            let km = im.buoyancy.unwrap().z + im.waterplane.i_along / im.volume;
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
            let im = immersion(&[&body], &upright(), h, &tol(k)).unwrap();
            let theta = 2.0 * (-h / r).acos();
            let segment = 0.5 * r * r * (theta - theta.sin());
            close(im.volume, l * segment, k.powi(3), "segment volume");
            let d = 4.0 * r * (0.5 * theta).sin().powi(3) / (3.0 * (theta - theta.sin()));
            close(im.buoyancy.unwrap().z, -d, k, "segment centroid");
            let c = (r * r - h * h).sqrt();
            close(im.waterplane.area, 2.0 * c * l, k * k, "waterplane");
            close(
                im.waterplane.i_along,
                l * (2.0 * c).powi(3) / 12.0,
                k.powi(4),
                "I_T",
            );
            close(
                im.waterplane.i_across,
                2.0 * c * l.powi(3) / 12.0,
                k.powi(4),
                "I_L",
            );
            close(im.wetted, 2.0 * segment + l * r * theta, k * k, "wetted");
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
        let im = immersion(&[&body], &upright(), t, &tol(k)).unwrap();
        close(im.volume, l * t * t, k.powi(3), "V");
        close(im.buoyancy.unwrap().z, 2.0 * t / 3.0, k, "KB");
        close(im.waterplane.area, 2.0 * t * l, k * k, "A");
        close(im.waterplane.i_along / im.volume, 2.0 * t / 3.0, k, "BM");
    }
}

/// The same barge moved and turned in space, measured in water turned
/// with it, gives the same numbers.
#[test]
fn results_do_not_depend_on_where_the_body_lies() {
    for k in SCALES {
        let body = barge(k, (4.0, 1.0, 1.0));
        let water = upright();
        let t = 0.3 * k;
        let reference = immersion(&[&body], &water, t, &tol(k)).unwrap();
        let turn = Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 1.1).unwrap();
        let frame = Frame::new(
            turn.point(Vec3::new(-2.0 * k, -0.5 * k, 0.0)),
            turn.vector(Vec3::new(0.0, 0.0, 1.0)),
            turn.vector(Vec3::new(1.0, 0.0, 0.0)),
        )
        .unwrap();
        let moved = block(&frame, 4.0 * k, k, k).unwrap();
        let turned = Water::new(
            turn.vector(Vec3::new(0.0, 0.0, 1.0)),
            turn.vector(Vec3::new(1.0, 0.0, 0.0)),
        )
        .unwrap();
        let im = immersion(&[&moved], &turned, t, &tol(k)).unwrap();
        close(im.volume, reference.volume, k.powi(3), "V");
        let b = im.buoyancy.unwrap();
        close(b.dot(turned.up), reference.buoyancy.unwrap().z, k, "KB");
        close(
            im.waterplane.i_along,
            reference.waterplane.i_along,
            k.powi(4),
            "I_T",
        );
        close(
            im.waterplane.i_across,
            reference.waterplane.i_across,
            k.powi(4),
            "I_L",
        );
        close(im.wetted, reference.wetted, k * k, "wetted");
    }
}

/// A wall-sided box heeled while its deck edge stays dry and its bilge
/// under water: GZ = sin φ (GM + BM tan²φ / 2) (Tupper).
#[test]
fn a_heeled_box_follows_the_wall_sided_formula() {
    for k in SCALES {
        let (b, t, kg) = (k, 0.4 * k, 0.45 * k);
        let body = barge(k, (4.0, 1.0, 1.0));
        let volume = 4.0 * k * b * t;
        let up = float(&[&body], &upright(), volume, &tol(k)).unwrap();
        let bm = up.immersion.waterplane.i_along / up.immersion.volume;
        let gm = t / 2.0 + bm - kg;
        close(bm, b * b / (12.0 * t), k, "BM");
        let g = Vec3::new(0.0, 0.0, kg);
        for degrees in [5.0_f64, 10.0, -10.0] {
            let phi = degrees.to_radians();
            let water = upright().heeled(phi).unwrap();
            let heeled = float(&[&body], &water, volume, &tol(k)).unwrap();
            let at = heeled.immersion.buoyancy.unwrap();
            let gz = (at - g).dot(water.up.cross(water.along));
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
        let floated = float(&[&body], &upright(), volume, &tol(1.0)).unwrap();
        close(floated.immersion.level, s, 1.0, "draft");
        let im = floated.immersion;
        let gm = im.buoyancy.unwrap().z + im.waterplane.i_along / im.volume - 0.5;
        assert_eq!(gm > 0.0, stable, "B/D {ratio}: GM {gm}");
    }
}

#[test]
fn what_cannot_float_is_said() {
    let body = barge(1.0, (4.0, 1.0, 1.0));
    let water = upright();
    let t = &tol(1.0);
    assert!(matches!(
        float(&[&body], &water, 4.5, t),
        Err(HydroError::Sinks { .. })
    ));
    assert!(matches!(
        float(&[&body], &water, 0.0, t),
        Err(HydroError::NoDisplacement { .. })
    ));
    assert!(matches!(
        float(&[&body], &water, f64::NAN, t),
        Err(HydroError::NotFinite(_))
    ));
    assert!(matches!(
        float(&[], &water, 1.0, t),
        Err(HydroError::NoBodies)
    ));
    assert!(matches!(
        immersion(&[&body], &water, f64::INFINITY, t),
        Err(HydroError::NotFinite(_))
    ));
    let mut sheet = body.clone();
    sheet.kind = BodyKind::Sheet;
    assert!(matches!(
        immersion(&[&sheet], &water, 0.5, t),
        Err(HydroError::NotSolid { .. })
    ));
    assert!(Water::new(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)).is_err());
    assert!(Water::new(Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 1.0)).is_err());
    assert!(upright().heeled(f64::NAN).is_err());
    // Above and below the body: nothing, and all of it.
    let none = immersion(&[&body], &water, -1.0, t).unwrap();
    assert_eq!(none.volume, 0.0);
    assert!(none.buoyancy.is_none());
    let all = immersion(&[&body], &water, 2.0, t).unwrap();
    close(all.volume, 4.0, 1.0, "all of it");
    assert_eq!(all.waterplane.area, 0.0);
}

/// Two bodies (a catamaran) are summed: twice one hull, the waterplane's
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
    let im = immersion(&[&a, &b], &upright(), 0.2, &tol(1.0)).unwrap();
    close(im.volume, 2.0 * 4.0 * 0.5 * 0.2, 1.0, "V");
    let one = 4.0 * 0.5_f64.powi(3) / 12.0;
    close(
        im.waterplane.i_along,
        2.0 * (one + 4.0 * 0.5 * 1.0),
        1.0,
        "I_T",
    );
    close(im.waterplane.centroid.unwrap().y, 0.0, 1.0, "centre");
}

/// A freeform body: the cone a cubic spline sweeps about an axis (a
/// spline surface), apex down. The waterplane is bounded by the curve in
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
        let im = immersion(&[&body], &upright(), h, &t).unwrap();
        let r = r_top * h / height;
        let within = |got: f64, want: f64, power: i32, what: &str| {
            let allowed = 1e-9 * want.abs() + 20.0 * t.linear * k.powi(power - 1);
            assert!(
                (got - want).abs() <= allowed,
                "{what} at scale {k}: got {got}, want {want} (allowed {allowed})"
            );
        };
        within(im.volume, PI * r * r * h / 3.0, 3, "volume");
        within(im.buoyancy.unwrap().z, 0.75 * h, 1, "KB = 3h/4");
        within(im.waterplane.area, PI * r * r, 2, "waterplane");
        within(im.waterplane.i_along, PI * r.powi(4) / 4.0, 4, "I_T");
        within(im.waterplane.i_across, PI * r.powi(4) / 4.0, 4, "I_L");
        within(im.wetted, PI * r * r.hypot(h), 2, "wetted");
        // Floated back to that level by its own displacement.
        let floated = float(&[&body], &upright(), im.volume, &t).unwrap();
        within(floated.immersion.level, h, 1, "waterline");
    }
}
