//! Closed corners of a flange with an outline of its own, and of a flange
//! whose side at the corner carries a flange (docs/design/sheetmetal.md ›
//! *Closed corners*), held to closed forms at 1e-3, 1 and 1e6:
//!
//! - a trapezoid wall on the bottom edge of a base `W × D` (its outline
//!   narrowing by `c` at either side over its length `L`) beside a
//!   rectangle wall on the right edge, both up 90 degrees, the corner
//!   closed by overlap, either covering: the trapezoid's slanted side is
//!   replaced by the end and its tip run on to it, so its flat gains the
//!   reach at the tangent line times `L` plus the triangle `c L / 2`; at
//!   K = ½ every end is square, so the volume is the flat area times `T`;
//! - a tab on the corner side of the bottom wall, bent up round the corner
//!   over the right wall's outside: the bottom wall covers by overlap, so
//!   its side moves `R + T` along its edge and the tab with it; the tab's
//!   flat stands `R₂` off the right wall's outside face; a mitre of walls
//!   leaning in (whose end does not lie along the side) is refused;
//! - names: every name of the part with the corner open is kept.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, CornerKind, Direction, FlangeSpec, LengthFrom, On, Part, Spec, build,
    developed_allowance, edge_numbers, flat_area, flat_faces, outline,
};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{check, mass_properties};
use std::collections::BTreeSet;
use std::f64::consts::FRAC_PI_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn polygon(points: &[[f64; 2]], k: f64) -> Profile {
    let n = points.len();
    let curves: Vec<SketchCurve> = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line {
                    start: [a[0] * k, a[1] * k],
                    end: [b[0] * k, b[1] * k],
                },
            }
        })
        .collect();
    detect_profiles(&curves, &tolerance(k)).unwrap().remove(0)
}

fn flange(on: On, length: f64, direction: Direction, r: f64, kf: f64) -> FlangeSpec {
    FlangeSpec {
        on,
        length,
        angle: FRAC_PI_2,
        direction,
        radius: r,
        k: kf,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    }
}

fn spec(k: f64, t: f64, flanges: Vec<FlangeSpec>, corner: Closing) -> Spec {
    Spec {
        base: polygon(&[[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]], k),
        plane: Frame::WORLD,
        thickness: t,
        flanges,
        relief: None,
        bend_relief: None,
        corner,
        corners: Vec::new(),
        cutouts: Vec::new(),
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

fn names(part: &Part) -> BTreeSet<Vec<u64>> {
    part.edge_keys
        .iter()
        .map(|k| edge_numbers(part, *k).unwrap())
        .collect()
}

fn healthy(part: &Part, tol: &Tolerance) {
    let defects = check(&part.solid.body, tol);
    assert!(
        defects.is_empty(),
        "the solid fails the checker: {defects:?}"
    );
    for b in &part.bends {
        near(
            developed_allowance(b, tol).unwrap(),
            b.allowance,
            4.0 * tol.linear,
            "the developed bend allowance",
        );
    }
    assert_eq!(outline(part, tol).len(), 1, "one chain");
}

/// The least and the most of `f` over flat `flat`'s vertices on the solid.
fn span(part: &Part, flat: usize, f: &dyn Fn(craft3d_math::Vec3) -> f64) -> (f64, f64) {
    let body = &part.solid.body;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for face in flat_faces(part, flat) {
        for l in &body.faces[face].loops {
            for c in &body.loop_(*l).unwrap().coedges {
                let p = body.vertex(body.coedge_ends(*c).unwrap().0).unwrap().point;
                lo = lo.min(f(p));
                hi = hi.max(f(p));
            }
        }
    }
    (lo, hi)
}

#[test]
fn an_outlined_flange_closes_its_corner() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, l, g, c) = (1.0 * k, 1.0 * k, 10.0 * k, 0.25 * k, 5.0 * k);
        let (w, d) = (60.0 * k, 40.0 * k);
        let kf = 0.5;
        let ba = FRAC_PI_2 * (r + kf * t);
        // The trapezoid drawn from the bottom edge, below it.
        let trapezoid = polygon(&[[5.0, -10.0], [55.0, -10.0], [60.0, 0.0], [0.0, 0.0]], k);
        let mut wall = flange(On::Base(1), l, Direction::Up, r, kf);
        wall.outline = Some(trapezoid);
        let flanges = vec![wall, flange(On::Base(2), l, Direction::Up, r, kf)];
        for over in [None, Some(1)] {
            let what = format!("scale {k}, over {over:?}");
            let corner = Closing {
                kind: CornerKind::Overlap,
                gap: g,
                over,
            };
            let sp = spec(k, t, flanges.clone(), corner);
            let part = build(&sp, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(part, build(&sp, &tol).unwrap(), "{what}: deterministic");
            healthy(&part, &tol);
            assert_eq!(
                part.corners.len(),
                1,
                "{what}: the outlined flange takes the corner"
            );
            // The trapezoid's reach past its tangent line's end, and the
            // rectangle's.
            let (reach0, reach1) = if over.is_none() {
                (r + t, r - g)
            } else {
                (r - g, r + t)
            };
            let trapezoid_area = 0.5 * (w + (w - 2.0 * c)) * l;
            let area = w * d
                + ba * (w + d)
                + trapezoid_area
                + l * reach0
                + 0.5 * c * l
                + l * d
                + l * reach1;
            near(
                flat_area(&part),
                area,
                1e-9 * area,
                &format!("{what}: area"),
            );
            near(
                volume(&part),
                area * t,
                1e-8 * area * t,
                &format!("{what}: volume"),
            );
            // The trapezoid's end: flush with the right wall's outside, or
            // the gap from its inside.
            let (_, x_hi) = span(&part, 1, &|p| p.x);
            near(
                x_hi,
                w + r + reach0 - r,
                10.0 * tol.linear,
                &format!("{what}: its end"),
            );
            let open = build(&spec(k, t, flanges.clone(), Closing::OPEN), &tol).unwrap();
            let (n_open, n_closed) = (names(&open), names(&part));
            assert!(
                n_open.is_subset(&n_closed),
                "{what}: lost {:?}",
                n_open.difference(&n_closed).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn a_tab_on_the_corner_side_moves_with_the_closed_end() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, l, g) = (1.0 * k, 1.0 * k, 10.0 * k, 0.25 * k);
        let (r2, l2) = (0.5 * k, 6.0 * k);
        let (w, d) = (60.0 * k, 40.0 * k);
        let kf = 0.5;
        let ba = FRAC_PI_2 * (r + kf * t);
        let ba2 = FRAC_PI_2 * (r2 + kf * t);
        let flanges = vec![
            flange(On::Base(1), l, Direction::Up, r, kf),
            flange(On::Base(2), l, Direction::Up, r, kf),
            flange(On::Side(0, 1), l2, Direction::Up, r2, kf),
        ];
        let what = format!("scale {k}");
        let sp = spec(
            k,
            t,
            flanges.clone(),
            Closing {
                kind: CornerKind::Overlap,
                gap: g,
                over: None,
            },
        );
        let part = build(&sp, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(part, build(&sp, &tol).unwrap(), "{what}: deterministic");
        healthy(&part, &tol);
        let area = w * d + ba * (w + d) + l * (w + r + t) + l * (d + r - g) + l * (ba2 + l2);
        near(
            flat_area(&part),
            area,
            1e-9 * area,
            &format!("{what}: area"),
        );
        near(
            volume(&part),
            area * t,
            1e-8 * area * t,
            &format!("{what}: volume"),
        );
        // The tab's flat stands R₂ off the right wall's outside x = W + R + T.
        let (x_lo, x_hi) = span(&part, 3, &|p| p.x);
        near(
            x_lo,
            w + r + t + r2,
            10.0 * tol.linear,
            &format!("{what}: the tab's inside"),
        );
        near(
            x_hi,
            w + r + 2.0 * t + r2,
            10.0 * tol.linear,
            &format!("{what}: the tab's outside"),
        );
        // Names: the corner open (the tab bent down, away from the right
        // wall it would run into there; names do not depend on it).
        let mut away = flanges.clone();
        away[2].direction = Direction::Down;
        let open = build(&spec(k, t, away, Closing::OPEN), &tol).unwrap();
        let (n_open, n_closed) = (names(&open), names(&part));
        assert!(
            n_open.is_subset(&n_closed),
            "{what}: lost {:?}",
            n_open.difference(&n_closed).collect::<Vec<_>>()
        );
        // Walls leaning in (120 degrees), mitred: the end slants across
        // the wall, so it does not lie along the side: refused.
        let mut leaning = flanges.clone();
        leaning[0].angle = 2.0 * std::f64::consts::FRAC_PI_3;
        leaning[1].angle = 2.0 * std::f64::consts::FRAC_PI_3;
        let mitred = spec(
            k,
            t,
            leaning,
            Closing {
                kind: CornerKind::Miter,
                gap: g,
                over: None,
            },
        );
        match build(&mitred, &tol) {
            Err(KernelError::SheetMetal { flange, reason }) => {
                assert_eq!(flange, Some(0));
                assert!(reason.contains("does not lie along the side"), "{reason}");
            }
            other => panic!("{what}: the mitre was not refused: {other:?}"),
        }
    }
}
