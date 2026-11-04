//! What a wall carries kept clear of the other wall at a corner
//! (docs/design/sheetmetal.md › *Closed corners* › *What a wall carries*),
//! at 1e-3, 1 and 1e6.
//!
//! The part: a base 60 × 40, walls up 90 degrees, 10 long, on its bottom
//! and right edges, `R = T = 1`, K 0.4, the corner mitred with a gap `g`.
//! Wall 0's flat ends (on its bottom skin `y = −2`) at
//! `x = 60 + R + T − g/√2`: the halving plane is `x + y = 60`, each wall
//! `g/2` short of it. Until 2026-11-04 nothing looked at what a wall
//! carries against its neighbour:
//!
//! - a tab on wall 0's side at the corner (`On::Side(0, 1)`), turned in or
//!   out, built a body through itself: its bend (radii 1 and 2 about the
//!   vertical line through the side, `R + T` beside the wall's bottom
//!   skin) runs into wall 1's extension. Now refused; a point inside both
//!   is shown (inside the part without the tab, by the kernel's point
//!   classification, and inside the tab's bend in closed form);
//! - returns on both walls' tips, turned in over the base, overlap at the
//!   corner: refused, a point inside both shown;
//! - the clear variants build, the volume the part's without them plus
//!   each one's closed form `w (θ T (R + T/2) + L T)` along its edge `w`:
//!   tabs on the open ends' sides, a tab at a corner closed by overlap
//!   with wall 0 covering (it lies beside wall 1), the returns inset 8 at
//!   the corner.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::contains_points;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, CornerKind, Direction, FlangeSpec, LengthFrom, On, Part, Spec, build,
};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{check, mass_properties};
use std::f64::consts::FRAC_PI_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn base(k: f64) -> Profile {
    let c = [
        [0.0, 0.0],
        [60.0 * k, 0.0],
        [60.0 * k, 40.0 * k],
        [0.0, 40.0 * k],
    ];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| SketchCurve {
            id: i as u64 + 1,
            curve: Curve2::Line {
                start: c[i],
                end: c[(i + 1) % 4],
            },
        })
        .collect();
    detect_profiles(&curves, &tolerance(k)).unwrap().remove(0)
}

fn flange(on: On, length: f64, direction: Direction, k: f64) -> FlangeSpec {
    FlangeSpec {
        on,
        length,
        angle: FRAC_PI_2,
        direction,
        radius: k,
        k: 0.4,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    }
}

fn spec(k: f64, gap: f64, more: Vec<FlangeSpec>) -> Spec {
    let mut flanges = vec![
        flange(On::Base(1), 10.0 * k, Direction::Up, k),
        flange(On::Base(2), 10.0 * k, Direction::Up, k),
    ];
    flanges.extend(more);
    Spec {
        base: base(k),
        plane: Frame::WORLD,
        thickness: k,
        flanges,
        relief: None,
        bend_relief: None,
        corner: Closing {
            kind: CornerKind::Miter,
            gap: gap * k,
            over: None,
        },
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs: Vec::new(),
        ids: Vec::new(),
    }
}

#[track_caller]
fn near(actual: f64, expected: f64, slack: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} against {expected} (off by {:e}, allowed {slack:e})",
        actual - expected
    );
}

fn volume(part: &Part) -> f64 {
    mass_properties(&part.solid.body).unwrap().volume
}

fn refused(spec: &Spec, tol: &Tolerance) -> (Option<usize>, String) {
    match build(spec, tol) {
        Ok(_) => panic!("the part was built"),
        Err(KernelError::SheetMetal { flange, reason }) => (flange, reason),
        Err(other) => panic!("refused for another reason: {other}"),
    }
}

/// A flange 90 degrees about `R = T = 1` along an edge `w` long, its flat
/// `l` beyond the bend: `w (θ T (R + T/2) + l T)`.
fn flange_volume(w: f64, l: f64, k: f64) -> f64 {
    let (t, r) = (k, k);
    w * (FRAC_PI_2 * t * (r + 0.5 * t) + l * t)
}

/// Whether `q` (in plan) lies in the bend of a tab on wall 0's side at
/// `x = side`: radii `R` and `R + T` (1 and 2) about the vertical line
/// `(side, axis_y)`, on the quarter towards `+x`.
fn in_tab_bend(q: [f64; 2], side: f64, axis_y: f64, k: f64) -> bool {
    let r = (q[0] - side).phypot(q[1] - axis_y);
    r > k && r < 2.0 * k && q[0] > side
}

/// Whether `q` (in plan) lies in wall 1's flat with its extension, the
/// gap `g`: between its skins `x = 61` and `62` and past its end plane
/// `x + y = 60 + g/√2`.
fn in_wall_1(q: [f64; 2], g: f64, k: f64) -> bool {
    q[0] > 61.0 * k && q[0] < 62.0 * k && q[0] + q[1] > 60.0 * k + g * k / 2f64.sqrt()
}

#[test]
fn a_tab_at_the_corner_that_runs_into_the_other_wall_is_refused() {
    for k in SCALES {
        let tol = tolerance(k);
        let g = 0.3;
        let plain = build(&spec(k, g, Vec::new()), &tol).unwrap();
        let side = 62.0 * k - g * k / 2f64.sqrt();
        for (dir, axis_y, q) in [
            // Turned in: the axis `R + T` inside the wall's bottom skin.
            (Direction::Up, 0.0, [61.9, -1.2]),
            // Turned out: `R` outside it.
            (Direction::Down, -3.0, [61.9, -1.3]),
        ] {
            let what = format!("scale {k}, {dir:?}");
            let tab = flange(On::Side(0, 1), 5.0 * k, dir, k);
            let (f, why) = refused(&spec(k, g, vec![tab]), &tol);
            assert_eq!(f, Some(0), "{what}: {why}");
            assert!(why.contains("not proven clear"), "{what}: {why}");
            assert!(why.contains("flange 2's bend"), "{what}: {why}");
            // The contact is real: a point inside both, at the wall's mid
            // height (the tab's side runs from z = 2 to 12, wall 1's
            // extension from z = 2 up).
            let q = [q[0] * k, q[1] * k];
            assert!(in_tab_bend(q, side, axis_y * k, k), "{what}");
            assert!(in_wall_1(q, g, k), "{what}");
            let w = Vec3::new(q[0], q[1], 6.0 * k);
            assert_eq!(
                contains_points(&plain.solid.body, &[w], &tol).unwrap(),
                vec![Some(true)],
                "{what}: the point is in wall 1"
            );
        }
        // Clear: the corner closed by overlap with wall 0 covering, so
        // wall 0 runs to wall 1's outside face `x = 62` and wall 1 stops
        // the gap short of wall 0's inside face, at `y = −1 + g`. The tab
        // on wall 0's side there lies at `x ≥ 62`, beside wall 1, and
        // meets that plane only along wall 0's own end (`y` from −2 to −1),
        // `g` from wall 1. Built, turned in or out, the part without it
        // plus the tab along the wall's side (10).
        let mut lap = spec(k, g, Vec::new());
        lap.corner = Closing {
            kind: CornerKind::Overlap,
            gap: g * k,
            over: Some(0),
        };
        let plain = build(&lap, &tol).unwrap();
        for dir in [Direction::Up, Direction::Down] {
            let mut with = lap.clone();
            with.flanges.push(flange(On::Side(0, 1), 5.0 * k, dir, k));
            let part = build(&with, &tol).unwrap_or_else(|e| panic!("scale {k}, {dir:?}: {e}"));
            assert!(check(&part.solid.body, &tol).is_empty(), "scale {k}");
            let v = volume(&plain) + flange_volume(10.0 * k, 5.0 * k, k);
            near(
                volume(&part),
                v,
                1e-9 * v,
                &format!("scale {k}, {dir:?}: beside wall 1"),
            );
        }
    }
}

#[test]
fn tabs_on_the_open_ends_build() {
    for k in SCALES {
        let tol = tolerance(k);
        let plain = build(&spec(k, 0.3, Vec::new()), &tol).unwrap();
        for dir in [Direction::Up, Direction::Down] {
            let tab = flange(On::Side(0, 0), 5.0 * k, dir, k);
            let part =
                build(&spec(k, 0.3, vec![tab]), &tol).unwrap_or_else(|e| panic!("scale {k}: {e}"));
            assert!(check(&part.solid.body, &tol).is_empty(), "scale {k}");
            let v = volume(&plain) + flange_volume(10.0 * k, 5.0 * k, k);
            near(volume(&part), v, 1e-9 * v, &format!("scale {k}, {dir:?}"));
        }
    }
}

/// Returns 15 long on both walls' tips, turned in over the base: both lie
/// between `z = 13` and `14` (the tip `R + T + 10` up, the flat `R`
/// above it), return 0 over `x` 0 to 60 and `y` 0 to 15, return 1 over
/// `x` 45 to 60 and `y` 0 to 40: they overlap at the corner, refused, the
/// point `(58, 3, 13.5)` inside both. Inset 8 at the corner and 5 long,
/// return 0 ends at `x = 52` and return 1 starts at `y = 8`, each reaching
/// 5 in over the base: built, the part plus both returns in closed form
/// along their edges 52 and 32.
#[test]
fn returns_that_meet_at_the_corner_are_refused_and_inset_ones_build() {
    for k in SCALES {
        let tol = tolerance(k);
        let plain = build(&spec(k, 0.3, Vec::new()), &tol).unwrap();
        let ret = |tip: usize, length: f64, inset: [f64; 2]| {
            let mut f = flange(On::Tip(tip), length * k, Direction::Up, k);
            f.inset = [inset[0] * k, inset[1] * k];
            f
        };
        let (f, why) = refused(
            &spec(
                k,
                0.3,
                vec![ret(0, 15.0, [0.0, 0.0]), ret(1, 15.0, [0.0, 0.0])],
            ),
            &tol,
        );
        assert!(f == Some(0) || f == Some(1), "scale {k}: {why}");
        assert!(why.contains("not proven clear"), "scale {k}: {why}");
        let q = [58.0, 3.0, 13.5];
        let in_return_0 =
            q[0] < 60.0 && (0.0..15.0).contains(&q[1]) && (13.0..14.0).contains(&q[2]);
        let in_return_1 =
            (45.0..60.0).contains(&q[0]) && q[1] > 0.0 && (13.0..14.0).contains(&q[2]);
        assert!(in_return_0 && in_return_1);

        // Tip 0 runs along x from 0 to 60 (its end at the corner), tip 1
        // along y from 0 (its start at the corner) to 40.
        let part = build(
            &spec(
                k,
                0.3,
                vec![ret(0, 5.0, [0.0, 8.0]), ret(1, 5.0, [8.0, 0.0])],
            ),
            &tol,
        )
        .unwrap_or_else(|e| panic!("scale {k}: {e}"));
        assert!(check(&part.solid.body, &tol).is_empty(), "scale {k}");
        let v = volume(&plain)
            + flange_volume(52.0 * k, 5.0 * k, k)
            + flange_volume(32.0 * k, 5.0 * k, k);
        near(
            volume(&part),
            v,
            1e-9 * v,
            &format!("scale {k}: inset returns"),
        );
    }
}
