//! Oracles for the floating equilibrium, at three scales with the
//! tolerance scaled along: a wall-sided box upright (its GM in both
//! directions), at its angles of loll where it is unstable upright
//! (tan φ = √(−2 GM / BM)) and heeled by a centre of gravity off its
//! middle (tan φ (GM + BM tan² φ / 2) = g); a horizontal cylinder with
//! its centre of gravity off its axis, which rolls until it is below the
//! axis (GM about the axis the offset); a sphere, neutral about its
//! centre, and with its centre of gravity off the centre; and the errors.

use super::*;
use crate::below::{CutPlane, Stability};
use crate::primitives::{block, cylinder, sphere};
use craft3d_math::Frame;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tol(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn upright() -> CutPlane {
    CutPlane::new(Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)).unwrap()
}

#[track_caller]
fn close(got: f64, want: f64, allowed: f64, what: &str) {
    assert!(
        (got - want).abs() <= allowed,
        "{what}: got {got}, want {want} (allowed {allowed})"
    );
}

/// A box L × B × D (L along x, B along y), centred on the z axis, its
/// bottom at z = 0.
fn boxed(k: f64, (l, b, d): (f64, f64, f64)) -> Body {
    let frame = Frame::new(
        Vec3::new(-0.5 * l * k, -0.5 * b * k, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .unwrap();
    block(&frame, l * k, b * k, d * k).unwrap()
}

/// The heel of a normal from z, in radians.
fn tilt(n: Vec3) -> f64 {
    n.z.clamp(-1.0, 1.0).pacos()
}

/// The GM of `found` for a turn about `axis` (one of its principal axes).
fn gm_about(found: &Equilibrium, axis: Vec3) -> f64 {
    found
        .principal
        .iter()
        .find(|p| p.about.cross(axis).length() < 1e-6)
        .map(|p| p.gm)
        .unwrap_or_else(|| panic!("no principal axis along {axis:?}: {:?}", found.principal))
}

// A box 10 × 2 × 2 at a draft of 1: KB = 1/2, BM = B²/(12 T) = 1/3,
// BM along its length L²/(12 T) = 100/12.
const BOX: (f64, f64, f64) = (10.0, 2.0, 2.0);

#[test]
fn a_box_upright_is_stable_with_its_metacentric_heights() {
    for k in SCALES {
        let body = boxed(k, BOX);
        let volume = 10.0 * 2.0 * 1.0 * k.ppowi(3);
        let centre = Vec3::new(0.0, 0.0, 0.5 * k);
        let found = floating(&[&body], &upright(), volume, centre, &tol(k)).unwrap();
        assert_eq!(found.equilibria.len(), 1, "{found:?}");
        let e = found.equilibria[0];
        assert_eq!(e.iterations, 0);
        assert!(e.is_stable(), "{e:?}");
        close(tilt(e.plane.normal), 0.0, 1e-12, "upright");
        close(e.level.below.offset, k, 1e-9 * k, "draft");
        close(
            gm_about(&e, Vec3::X),
            k / 3.0,
            1e-9 * k,
            "GM = KB + BM - KG",
        );
        close(
            gm_about(&e, Vec3::Y),
            100.0 / 12.0 * k,
            1e-9 * k,
            "GM along",
        );
    }
}

#[test]
fn a_box_unstable_upright_finds_its_two_angles_of_loll() {
    for k in SCALES {
        let body = boxed(k, BOX);
        let volume = 20.0 * k.ppowi(3);
        // KG 0.9: GM = 0.5 + 1/3 − 0.9 < 0; wall-sided, the loll is at
        // tan² φ = −2 GM / BM, deck and bilge clear (B/2 tan φ < T).
        let kg: f64 = 0.9;
        let gm = 0.5 + 1.0 / 3.0 - kg;
        let phi = (-2.0 * gm / (1.0 / 3.0)).sqrt().patan();
        let centre = Vec3::new(0.0, 0.0, kg * k);
        let found = floating(&[&body], &upright(), volume, centre, &tol(k)).unwrap();
        assert!(found.unfinished.is_empty(), "{:?}", found.unfinished);
        assert_eq!(found.equilibria.len(), 3, "{found:?}");
        let upright_one = found.equilibria[0];
        close(tilt(upright_one.plane.normal), 0.0, 1e-12, "upright first");
        assert!(upright_one.is_unstable());
        close(gm_about(&upright_one, Vec3::X), gm * k, 1e-9 * k, "GM < 0");
        let mut sides = Vec::new();
        for e in &found.equilibria[1..] {
            assert!(e.is_stable(), "{e:?}");
            close(tilt(e.plane.normal), phi, 1e-7, "angle of loll");
            close(e.plane.normal.x, 0.0, 1e-9, "no trim");
            // Wall-sided: the plane turns about the middle of the deck
            // line at the draft.
            close(
                e.level.below.offset,
                k * phi.pcos(),
                1e-8 * k,
                "the plane through the middle at the draft",
            );
            sides.push(e.plane.normal.y.signum());
        }
        sides.sort_by(f64::total_cmp);
        assert_eq!(sides, [-1.0, 1.0], "one to each side");
    }
}

#[test]
fn a_box_heels_by_a_centre_of_gravity_off_its_middle() {
    for k in SCALES {
        let body = boxed(k, BOX);
        let volume = 20.0 * k.ppowi(3);
        let (kg, g): (f64, f64) = (0.5, 0.1);
        let (gm, bm) = (0.5 + 1.0 / 3.0 - kg, 1.0 / 3.0);
        // t (GM + BM t² / 2) = g, by Newton's method.
        let mut t = g / gm;
        for _ in 0..50 {
            t -= (t * (gm + 0.5 * bm * t * t) - g) / (gm + 1.5 * bm * t * t);
        }
        let centre = Vec3::new(0.0, g * k, kg * k);
        let found = floating(&[&body], &upright(), volume, centre, &tol(k)).unwrap();
        assert_eq!(found.equilibria.len(), 1, "{found:?}");
        let e = found.equilibria[0];
        assert!(e.is_stable() && e.settled, "{e:?}");
        close(tilt(e.plane.normal).ptan(), t, 1e-7, "tan φ");
        // The weight's side goes down: the normal tips away from it.
        assert!(e.plane.normal.y < 0.0, "{e:?}");
        assert!(e.arm <= 1e-8 * k, "{e:?}");
    }
}

#[test]
fn a_cylinder_rolls_until_its_centre_of_gravity_is_below_its_axis() {
    for k in SCALES {
        let (r, l) = (k, 4.0 * k);
        // Along x, its axis through the origin.
        let frame = Frame::new(
            Vec3::new(-0.5 * l, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
        .unwrap();
        let body = cylinder(&frame, r, l).unwrap();
        let volume = 0.5 * std::f64::consts::PI * r * r * l;
        let (e_off, alpha) = (0.3 * k, 30f64.to_radians());
        let centre = Vec3::new(0.0, e_off * alpha.psin(), -e_off * alpha.pcos());
        let found = floating(&[&body], &upright(), volume, centre, &tol(k)).unwrap();
        let e = found.equilibria[0];
        assert!(e.is_stable(), "{e:?}");
        // G on the normal through the axis: n along (0, −sin α, cos α).
        close(e.plane.normal.y, -alpha.psin(), 1e-8, "roll");
        close(e.plane.normal.z, alpha.pcos(), 1e-8, "roll");
        close(e.plane.normal.x, 0.0, 1e-9, "no trim");
        // Half the volume: the plane through the axis.
        close(e.level.below.offset, 0.0, 1e-8 * k, "through the axis");
        // The metacentre of a roll is the axis: GM is the offset.
        close(gm_about(&e, Vec3::X), e_off, 1e-7 * k, "GM about the axis");
    }
}

#[test]
fn a_sphere_about_its_centre_is_neutral_and_off_it_hangs_below_it() {
    for k in SCALES {
        let r = k;
        let body = sphere(&Frame::WORLD, r).unwrap();
        let whole = 4.0 / 3.0 * std::f64::consts::PI * r.ppowi(3);
        let found = floating(&[&body], &upright(), 0.3 * whole, Vec3::ZERO, &tol(k)).unwrap();
        assert_eq!(found.equilibria.len(), 1, "{found:?}");
        let e = found.equilibria[0];
        assert_eq!(e.iterations, 0);
        for p in e.principal {
            assert_eq!(p.stability, Stability::Neutral, "{e:?}");
            assert!(p.gm.abs() <= found.floor, "{e:?}");
        }
        // Off the centre: it turns until G is below the centre, GM the
        // distance between them in every direction.
        let centre = Vec3::new(0.0, 0.2 * k, -0.3 * k);
        let found = floating(&[&body], &upright(), 0.3 * whole, centre, &tol(k)).unwrap();
        let e = found.equilibria[0];
        let want = (-centre).normalized().unwrap();
        close(e.plane.normal.y, want.y, 1e-8, "normal");
        close(e.plane.normal.z, want.z, 1e-8, "normal");
        for p in e.principal {
            assert_eq!(p.stability, Stability::Stable);
            close(p.gm, centre.length(), 1e-7 * k, "GM");
        }
    }
}

#[test]
fn what_cannot_float_is_said() {
    let body = boxed(1.0, BOX);
    let t = tol(1.0);
    assert!(matches!(
        floating(
            &[&body],
            &upright(),
            20.0,
            Vec3::new(f64::NAN, 0.0, 0.0),
            &t
        ),
        Err(CutError::NotFinite(_))
    ));
    assert!(matches!(
        floating(&[&body], &upright(), 50.0, Vec3::ZERO, &t),
        Err(CutError::OutOfRange { .. })
    ));
    assert!(matches!(
        floating(&[&body], &upright(), f64::INFINITY, Vec3::ZERO, &t),
        Err(CutError::NotFinite(_))
    ));
    assert!(floating(&[], &upright(), 1.0, Vec3::ZERO, &t).is_err());
}

#[test]
fn the_principal_directions_and_the_turn_are_what_they_say() {
    // eigen: (2, 1; 1, 2) has 1 along (−1, 1)/√2 and 3 along (1, 1)/√2.
    let [(small, u), (large, v)] = eigen([2.0, 1.0, 2.0]);
    close(small, 1.0, 1e-15, "small");
    close(large, 3.0, 1e-15, "large");
    close(u[0] * v[0] + u[1] * v[1], 0.0, 1e-15, "square");
    close((u[1] - u[0]).abs(), 2f64.sqrt(), 1e-15, "along (−1, 1)");
    // A turn of 30° towards y keeps x and tips the normal.
    let p = turned(&upright(), [0.0, 30f64.to_radians()]).unwrap();
    close(p.normal.y, 0.5, 1e-15, "sin 30°");
    close(p.x_axis.x, 1.0, 1e-15, "x kept");
    assert_eq!(turned(&upright(), [0.0, 0.0]).unwrap(), upright());
    assert!(turned(&upright(), [f64::NAN, 0.0]).is_err());
}
