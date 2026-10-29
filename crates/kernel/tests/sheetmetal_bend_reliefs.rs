//! Bend reliefs at the ends of a bend inside its edge
//! (docs/design/sheetmetal.md › *Bend reliefs*), held to closed forms at
//! 1e-3, 1 and 1e6: a base `W × D` with a tab `w` wide inset on its bottom
//! edge, bent up 90 degrees, `R = T = 1`, `K = ½` (and 0.33):
//!
//! - without a bend relief both ends of the bend are listed without one,
//!   and the base's edge turns into the strip's end there;
//! - with a relief of size `r` the base loses a notch at each end: `r²`
//!   (rect), `π r²/4` (round), `r² + 3π r²/4` (tear); the flat area and the
//!   volume (`T` times the flats plus the quarter rings) follow, the
//!   notch's corners lie where they must, and no name of the part without
//!   it is lost: the notches add only `edge(11, …)`;
//! - two tabs whose reliefs would leave less than a sliver between them,
//!   and a relief longer than the edge beside the tab, are refused by
//!   reason; a tab on a wall's tip with reliefs is built.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, Direction, FlangeSpec, LengthFrom, On, Part, Relief, ReliefKind, SheetFace, Spec,
    build, developed_allowance, edge_numbers, flat_area,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{FaceId, check, mass_properties};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The rectangle `0..w` × `0..d` scaled by `k`: curves 1 to 4 from the
/// bottom edge on.
fn rect(w: f64, d: f64, k: f64) -> Profile {
    let pts = [[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line {
                    start: [a[0] * k, a[1] * k],
                    end: [b[0] * k, b[1] * k],
                },
            }
        })
        .collect();
    let mut found = detect_profiles(&curves, &tolerance(k)).unwrap();
    found.remove(0)
}

fn flange(on: On, length: f64, r: f64, kf: f64, inset: [f64; 2]) -> FlangeSpec {
    FlangeSpec {
        on,
        length,
        angle: FRAC_PI_2,
        direction: Direction::Up,
        radius: r,
        k: kf,
        from: LengthFrom::Tangent,
        inset,
        outline: None,
    }
}

fn spec(base: Profile, t: f64, flanges: Vec<FlangeSpec>, relief: Option<Relief>) -> Spec {
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: t,
        flanges,
        relief: None,
        bend_relief: relief,
        corner: Closing::OPEN,
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

/// A quarter bend's material: `θ w ((R + T)² − R²) / 2`.
fn ring(w: f64, r: f64, t: f64) -> f64 {
    FRAC_PI_2 * w * ((r + t).powi(2) - r * r) / 2.0
}

fn reason(spec: &Spec, tol: &Tolerance) -> String {
    match build(spec, tol) {
        Ok(_) => panic!("built"),
        Err(KernelError::SheetMetal { reason, .. }) => reason,
        Err(other) => format!("other: {other}"),
    }
}

/// The notch's area for a relief of size `r` at a square corner.
fn notch_area(kind: ReliefKind, r: f64) -> f64 {
    match kind {
        ReliefKind::Rect => r * r,
        ReliefKind::Round => PI * r * r / 4.0,
        ReliefKind::Tear => r * r + 3.0 * PI * r * r / 4.0,
    }
}

/// The solid passes the checker; fold then unfold: every folded flat's
/// vertices land back on a corner of its outline at z = 0; each bend's
/// neutral band develops to its strip's width.
fn healthy(part: &Part, tol: &Tolerance) {
    let defects = check(&part.solid.body, tol);
    assert!(
        defects.is_empty(),
        "the solid fails the checker: {defects:?}"
    );
    for (f, role) in part.sheet_faces.iter().enumerate() {
        let SheetFace::Flat(i) = role else { continue };
        let flat = &part.flats[*i];
        let back = flat.transform.inverse().unwrap();
        let face = part.sheet.face(FaceId(f as u32)).unwrap();
        for l in &face.loops {
            for c in &part.sheet.loop_(*l).unwrap().coedges {
                let v = part.sheet.coedge_ends(*c).unwrap().0;
                let (x, y, z) = part.plane.local(part.sheet.vertex(v).unwrap().point);
                let p = back.point(Vec3::new(x, y, z));
                assert!(p.z.abs() <= tol.linear, "off the plane by {}", p.z);
                let on = flat
                    .outer
                    .iter()
                    .chain(flat.holes.iter().flatten())
                    .map(|s| (s.start[0] - p.x).hypot(s.start[1] - p.y))
                    .fold(f64::INFINITY, f64::min);
                assert!(on <= tol.linear, "a vertex lands {on} from its corner");
            }
        }
    }
    for b in &part.bends {
        near(
            developed_allowance(b, tol).unwrap(),
            b.allowance,
            4.0 * tol.linear,
            "the developed bend allowance",
        );
    }
}

/// Every edge name of the folded sheet.
fn names(part: &Part) -> BTreeSet<Vec<u64>> {
    part.edge_keys
        .iter()
        .map(|k| edge_numbers(part, *k).unwrap())
        .collect()
}

#[test]
fn a_tab_with_bend_reliefs_loses_their_notches_in_closed_form() {
    for k in SCALES {
        let tol = tolerance(k);
        let (wb, d, t, r, l) = (100.0 * k, 60.0 * k, 1.0 * k, 1.0 * k, 20.0 * k);
        let (i0, i1) = (30.0 * k, 30.0 * k);
        let w = wb - i0 - i1;
        for kf in [0.33, 0.5] {
            let ba = FRAC_PI_2 * (r + kf * t);
            let tab = flange(On::Base(1), l, r, kf, [i0, i1]);
            let plain = build(
                &spec(rect(100.0, 60.0, k), t, vec![tab.clone()], None),
                &tol,
            )
            .unwrap();
            healthy(&plain, &tol);
            // Both ends lie inside the edge, neither has a relief.
            assert_eq!(plain.bend_ends.len(), 2);
            assert!(plain.bend_ends.iter().all(|e| e.relief.is_none()));
            near(plain.bend_ends[0].point[0], i0, 0.0, "the bend's start");
            near(
                plain.bend_ends[1].point[0],
                wb - i1,
                1e-12 * k,
                "the bend's end",
            );
            let a0 = wb * d + w * (ba + l);
            near(
                flat_area(&plain),
                a0,
                1e-9 * a0,
                "the flat area without reliefs",
            );
            let before = names(&plain);

            for kind in [ReliefKind::Rect, ReliefKind::Round, ReliefKind::Tear] {
                let rs = 2.0 * k;
                let relief = Relief { kind, size: rs };
                let part = build(
                    &spec(rect(100.0, 60.0, k), t, vec![tab.clone()], Some(relief)),
                    &tol,
                )
                .unwrap();
                healthy(&part, &tol);
                assert_eq!(part.bend_ends.len(), 2);
                assert!(part.bend_ends.iter().all(|e| e.relief == Some(relief)));
                let cut = 2.0 * notch_area(kind, rs);
                near(
                    flat_area(&part),
                    a0 - cut,
                    1e-9 * a0,
                    "the flat area with reliefs",
                );
                // The volume: the flats' area times T plus the quarter ring
                // (the strip's own area times T is not the bend's material
                // unless K = ½).
                let v = t * (wb * d - cut + w * l) + ring(w, r, t);
                near(volume(&part), v, 1e-9 * v, "the volume");
                // The bend keeps its span; the edge beside it ends the size
                // short of it, and the notch reaches the size into the base.
                near(part.bends[0].start[0], i0, 0.0, "the bend's start");
                let base = &part.flats[0].outer;
                let starts: Vec<[f64; 2]> = base.iter().map(|s| s.start).collect();
                let has = |p: [f64; 2]| {
                    starts
                        .iter()
                        .any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) <= tol.linear)
                };
                assert!(
                    has([i0 - rs, 0.0]),
                    "the edge ends {rs} before the bend: {starts:?}"
                );
                assert!(has([wb - i1 + rs, 0.0]), "and starts {rs} after it");
                assert!(has([i0, rs]), "the notch reaches {rs} into the base");
                assert!(has([wb - i1, rs]));
                if kind == ReliefKind::Rect {
                    assert!(has([i0 - rs, rs]) && has([wb - i1 + rs, rs]));
                }
                // Every name of the part without reliefs is kept; what is
                // new is the notches' edges.
                let after = names(&part);
                assert!(
                    before.is_subset(&after),
                    "a name was lost: {:?}",
                    before.difference(&after).collect::<Vec<_>>()
                );
                let added: Vec<&Vec<u64>> = after.difference(&before).collect();
                assert!(!added.is_empty());
                assert!(added.iter().all(|n| n[0] == 11 && n[1] == 0), "{added:?}");
                // The same part twice is the same part.
                let again = build(
                    &spec(rect(100.0, 60.0, k), t, vec![tab.clone()], Some(relief)),
                    &tol,
                )
                .unwrap();
                assert_eq!(again, part);
            }
        }
    }
}

#[test]
fn reliefs_that_do_not_fit_beside_the_bend_are_refused_and_a_tab_on_a_tip_takes_them() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, l) = (1.0 * k, 1.0 * k, 20.0 * k);
        let relief = |size: f64| {
            Some(Relief {
                kind: ReliefKind::Rect,
                size: size * k,
            })
        };
        // Two tabs 6 apart: reliefs of 2 leave 2, of 3 leave nothing.
        let a = flange(On::Base(1), l, r, 0.5, [20.0 * k, 54.0 * k]);
        let b = flange(On::Base(1), l, r, 0.5, [52.0 * k, 20.0 * k]);
        let part = build(
            &spec(
                rect(100.0, 60.0, k),
                t,
                vec![a.clone(), b.clone()],
                relief(2.0),
            ),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
        assert_eq!(part.bend_ends.len(), 4);
        let why = reason(
            &spec(rect(100.0, 60.0, k), t, vec![a.clone(), b], relief(3.0)),
            &tol,
        );
        assert!(
            why.contains("bend relief") && why.contains("does not fit"),
            "{why}"
        );
        // A relief longer than the edge left beside the tab.
        let near_end = flange(On::Base(1), l, r, 0.5, [20.0 * k, 3.0 * k]);
        let why = reason(
            &spec(rect(100.0, 60.0, k), t, vec![near_end], relief(4.0)),
            &tol,
        );
        assert!(
            why.contains("bend relief") && why.contains("does not fit"),
            "{why}"
        );

        // A wall on the bottom edge and a tab inset on its tip: the
        // notches are cut into the wall, area 2 r² less.
        let wall = flange(On::Base(1), l, r, 0.5, [0.0; 2]);
        let tab = flange(On::Tip(0), 8.0 * k, r, 0.5, [30.0 * k, 30.0 * k]);
        let plain = build(
            &spec(
                rect(100.0, 60.0, k),
                t,
                vec![wall.clone(), tab.clone()],
                None,
            ),
            &tol,
        )
        .unwrap();
        let part = build(
            &spec(rect(100.0, 60.0, k), t, vec![wall, tab], relief(2.0)),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
        assert_eq!(part.bend_ends.len(), 2);
        assert!(part.bend_ends.iter().all(|e| e.bend == 1));
        let a = flat_area(&plain);
        near(
            flat_area(&part),
            a - 2.0 * 4.0 * k * k,
            1e-9 * a,
            "the tip tab's reliefs",
        );
        assert!(names(&plain).is_subset(&names(&part)));
        // A relief's size must be a size.
        let why = reason(
            &spec(rect(100.0, 60.0, k), t, vec![a_tab(k)], relief(0.0)),
            &tol,
        );
        assert!(why.contains("bend relief's size"), "{why}");
    }
}

fn a_tab(k: f64) -> FlangeSpec {
    flange(On::Base(1), 20.0 * k, 1.0 * k, 0.5, [30.0 * k, 30.0 * k])
}
