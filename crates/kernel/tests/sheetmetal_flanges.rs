//! Sheet-metal flanges beyond a rectangle across a whole edge
//! (docs/design/sheetmetal.md › *Inset flanges, outlines, sides and
//! holes*), held to closed forms at 1e-3, 1 and 1e6:
//!
//! - an inset flange and two tabs on one edge: the flat area `A` of the
//!   base and `w (BA + L)` per tab, the volume `T` times the flats plus
//!   `θ w ((R + T)² − R²)/2` per bend, the box;
//! - flanges on a flange's sides (one inset, one bent down): the same sums,
//!   each side `L` long, and where the tabs stand;
//! - a tab bent from a hole's edge (a louver): the hole's area less, the
//!   tab hanging below the base, no hole-to-bend distance to its own edge;
//! - a flange with an outline of its own (an arch with a hole): its area
//!   and reach in closed form;
//! - fold then unfold for every part; the pattern checked against
//!   overlapping itself (two inset flanges at a reflex corner); refusals
//!   by reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, Direction, FlangeSpec, LengthFrom, On, Part, SheetFace, Spec, build,
    developed_allowance, flange_length, flat_area, holes_to_bends, measured_angle,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{FaceId, check, mass_properties};
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// Closed polygons (each a list of points, scaled by `k`) as sketch lines
/// numbered from 1 in order, and their one profile.
fn profile(loops: &[&[[f64; 2]]], k: f64) -> Profile {
    let mut curves = Vec::new();
    for pts in loops {
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            curves.push(SketchCurve {
                id: curves.len() as u64 + 1,
                curve: Curve2::Line {
                    start: [a[0] * k, a[1] * k],
                    end: [b[0] * k, b[1] * k],
                },
            });
        }
    }
    let mut found = detect_profiles(&curves, &tolerance(k)).unwrap();
    assert_eq!(found.len(), 1);
    found.remove(0)
}

/// The rectangle `0..w` × `0..d`: curves 1 to 4 from the bottom edge on.
fn rect(w: f64, d: f64, k: f64) -> Profile {
    profile(&[&[[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]]], k)
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

fn inset(f: FlangeSpec, a: f64, b: f64) -> FlangeSpec {
    FlangeSpec { inset: [a, b], ..f }
}

fn spec(base: Profile, t: f64, flanges: Vec<FlangeSpec>) -> Spec {
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: t,
        flanges,
        relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts: Vec::new(),
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

fn bbox(part: &Part) -> (Vec3, Vec3) {
    part.solid.body.bbox().unwrap().corners().unwrap()
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

/// The solid passes the checker; fold then unfold: every folded flat's
/// vertices, carried back by its inverse motion, land at z = 0 on its
/// outline in the pattern, and each bend's neutral band develops to its
/// strip's width.
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

#[test]
fn an_inset_flange_and_two_tabs_on_one_edge() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, d, t, r, l) = (40.0 * k, 30.0 * k, 1.0 * k, 2.0 * k, 12.0 * k);
        for kf in [0.33, 0.5] {
            let ba = FRAC_PI_2 * (r + kf * t);
            // One flange, 5 in from the edge's start and 8 from its end.
            let one = inset(
                flange(On::Base(1), l, Direction::Up, r, kf),
                5.0 * k,
                8.0 * k,
            );
            let part = build(&spec(rect(40.0, 30.0, k), t, vec![one]), &tol).unwrap();
            healthy(&part, &tol);
            let wf = w - 13.0 * k;
            near(part.bends[0].start[0], 5.0 * k, 0.0, "the bend's start");
            near(
                part.bends[0].end[0],
                w - 8.0 * k,
                1e-12 * k,
                "the bend's end",
            );
            // The base's edge is in three: before, the tangent line, after.
            assert_eq!(part.flats[0].outer.len(), 6);
            near(
                flat_area(&part),
                w * d + wf * (ba + l),
                1e-9 * w * d,
                "flat area",
            );
            let v = t * (w * d + wf * l) + ring(wf, r, t);
            near(volume(&part), v, 1e-9 * v, "volume");
            let (lo, hi) = bbox(&part);
            near(lo.y, -(r + t), 10.0 * tol.linear, "the flange's outside");
            near(hi.z, t + r + l, 10.0 * tol.linear, "the flange's top");
            near(lo.x, 0.0, 10.0 * tol.linear, "the base's start");
            near(hi.x, w, 10.0 * tol.linear, "the base's end");

            // Two tabs, [5, 15] and [25, 35] along the 40 of the edge.
            let a = inset(
                flange(On::Base(1), l, Direction::Up, r, kf),
                5.0 * k,
                25.0 * k,
            );
            let b = inset(
                flange(On::Base(1), l, Direction::Up, r, kf),
                25.0 * k,
                5.0 * k,
            );
            let part = build(&spec(rect(40.0, 30.0, k), t, vec![a.clone(), b]), &tol).unwrap();
            healthy(&part, &tol);
            assert_eq!(part.flats[0].outer.len(), 8);
            let tab = 10.0 * k;
            near(
                flat_area(&part),
                w * d + 2.0 * tab * (ba + l),
                1e-9 * w * d,
                "flat area of two tabs",
            );
            let v = t * (w * d + 2.0 * tab * l) + 2.0 * ring(tab, r, t);
            near(volume(&part), v, 1e-9 * v, "volume of two tabs");

            // Tabs that overlap or touch on the edge are refused.
            for second in [15.0, 20.0] {
                let b = inset(
                    flange(On::Base(1), l, Direction::Up, r, kf),
                    second * k,
                    5.0 * k,
                );
                let a = inset(a.clone(), 5.0 * k, 20.0 * k);
                let why = reason(&spec(rect(40.0, 30.0, k), t, vec![a, b]), &tol);
                assert!(why.contains("has a flange already"), "{why}");
            }
        }
    }
}

#[test]
fn flanges_on_a_flanges_sides() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, d, t, r) = (40.0 * k, 30.0 * k, 1.0 * k, 2.0 * k);
        let (l0, l1, l2) = (15.0 * k, 10.0 * k, 8.0 * k);
        let kf = 0.4;
        let ba = FRAC_PI_2 * (r + kf * t);
        let wall = flange(On::Base(1), l0, Direction::Up, r, kf);
        // At the wall's end (x = w): bent up, round to the base's side.
        let end = flange(On::Side(0, 1), l1, Direction::Up, r, kf);
        let part = build(
            &spec(rect(40.0, 30.0, k), t, vec![wall.clone(), end.clone()]),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
        near(
            flat_area(&part),
            w * d + w * (ba + l0) + l0 * (ba + l1),
            1e-9 * w * d,
            "flat area",
        );
        let v = t * (w * d + w * l0 + l0 * l1) + ring(w, r, t) + ring(l0, r, t);
        near(volume(&part), v, 1e-9 * v, "volume");
        near(
            measured_angle(&part, 1).unwrap(),
            FRAC_PI_2,
            1e-12,
            "the side's angle",
        );
        let (lo, hi) = bbox(&part);
        // The tab stands R + T beyond the base's end, from y = 0 to l1.
        near(hi.x, w + r + t, 10.0 * tol.linear, "the tab's outside");
        near(lo.y, -(r + t), 10.0 * tol.linear, "the wall's outside");
        near(hi.z, t + r + l0, 10.0 * tol.linear, "the tops");

        // And at the wall's start, inset 2 and 3 and bent down (away).
        let start = inset(
            flange(On::Side(0, 0), l2, Direction::Down, r, kf),
            2.0 * k,
            3.0 * k,
        );
        let part = build(&spec(rect(40.0, 30.0, k), t, vec![wall, end, start]), &tol).unwrap();
        healthy(&part, &tol);
        let ws = l0 - 5.0 * k;
        near(
            flat_area(&part),
            w * d + w * (ba + l0) + l0 * (ba + l1) + ws * (ba + l2),
            1e-9 * w * d,
            "flat area with both sides",
        );
        let v = t * (w * d + w * l0 + l0 * l1 + ws * l2)
            + ring(w, r, t)
            + ring(l0, r, t)
            + ring(ws, r, t);
        near(volume(&part), v, 1e-9 * v, "volume with both sides");
        let (lo, _) = bbox(&part);
        // Bent down from the wall: its outside R + T before the base's
        // start, its tip 2R + T + l2 outside the wall's outer skin plane.
        near(lo.x, -(r + t), 10.0 * tol.linear, "the down tab's outside");
        near(
            lo.y,
            -(2.0 * r + t + l2),
            10.0 * tol.linear,
            "the down tab's tip",
        );
    }
}

#[test]
fn a_tab_bent_from_a_holes_edge() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, l) = (1.0 * k, 1.0 * k, 10.0 * k);
        let kf = 0.5;
        let ba = FRAC_PI_2 * (r + kf * t);
        // A 60 × 40 plate with a 20 × 20 hole; the hole's bottom edge is
        // curve 5, the tab 16 wide (2 in at each end), bent down.
        let base = || {
            profile(
                &[
                    &[[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]],
                    &[[20.0, 10.0], [40.0, 10.0], [40.0, 30.0], [20.0, 30.0]],
                ],
                k,
            )
        };
        let tab = inset(
            flange(On::Base(5), l, Direction::Down, r, kf),
            2.0 * k,
            2.0 * k,
        );
        let part = build(&spec(base(), t, vec![tab.clone()]), &tol).unwrap();
        healthy(&part, &tol);
        let (a, hole, wt) = (2400.0 * k * k, 400.0 * k * k, 16.0 * k);
        near(
            flat_area(&part),
            a - hole + wt * (ba + l),
            1e-9 * a,
            "flat area",
        );
        let v = t * (a - hole + wt * l) + ring(wt, r, t);
        near(volume(&part), v, 1e-9 * v, "volume");
        let (lo, _) = bbox(&part);
        near(
            lo.z,
            -(r + l),
            10.0 * tol.linear,
            "the tab's tip below the base",
        );
        // The hole is the tab's edge, not a hole beside a bend.
        assert!(holes_to_bends(&part, &tol).is_empty());
        // Across the whole edge the strip's ends lie along the hole's sides.
        let whole = FlangeSpec {
            inset: [0.0; 2],
            ..tab.clone()
        };
        let why = reason(&spec(base(), t, vec![whole]), &tol);
        assert!(why.contains("overlaps itself"), "{why}");
        // Too long for the hole: it runs into the hole's far side.
        let long = FlangeSpec {
            length: 20.0 * k,
            ..tab
        };
        let why = reason(&spec(base(), t, vec![long]), &tol);
        assert!(why.contains("overlaps itself"), "{why}");
    }
}

/// An arch 20 wide on the base's bottom edge (from x = 10 to 30), 10 high
/// and closed by a half circle of radius 10, with a round hole of radius 3
/// at the arch's centre.
fn arch(k: f64, lift: f64, from: f64) -> Profile {
    let s = |x: f64, y: f64| [x * k, y * k];
    let a = from;
    let curves = vec![
        SketchCurve {
            id: 1,
            curve: Curve2::Line {
                start: s(a + 20.0, lift),
                end: s(a, lift),
            },
        },
        SketchCurve {
            id: 2,
            curve: Curve2::Line {
                start: s(a, lift),
                end: s(a, lift - 10.0),
            },
        },
        SketchCurve {
            id: 3,
            curve: Curve2::Arc {
                center: s(a + 10.0, lift - 10.0),
                start: s(a, lift - 10.0),
                end: s(a + 20.0, lift - 10.0),
            },
        },
        SketchCurve {
            id: 4,
            curve: Curve2::Line {
                start: s(a + 20.0, lift - 10.0),
                end: s(a + 20.0, lift),
            },
        },
        SketchCurve {
            id: 5,
            curve: Curve2::Circle {
                center: s(a + 10.0, lift - 10.0),
                radius: 3.0 * k,
            },
        },
    ];
    let found = detect_profiles(&curves, &tolerance(k)).unwrap();
    let shapes: Vec<(Vec<u64>, usize)> = found
        .iter()
        .map(|p| {
            (
                p.outer.segments.iter().map(|g| g.source).collect(),
                p.holes.len(),
            )
        })
        .collect();
    let arch = found
        .into_iter()
        .find(|p| p.outer.segments.iter().any(|g| g.source == 1))
        .unwrap_or_else(|| panic!("no arch among {shapes:?}"));
    assert_eq!(arch.holes.len(), 1, "the arch's profiles: {shapes:?}");
    arch
}

#[test]
fn a_flange_with_an_outline_of_its_own() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, d, t, r) = (40.0 * k, 30.0 * k, 1.0 * k, 2.0 * k);
        let kf = 0.4;
        let ba = FRAC_PI_2 * (r + kf * t);
        let own = |outline: Profile| FlangeSpec {
            length: 0.0,
            outline: Some(outline),
            ..flange(On::Base(1), 0.0, Direction::Up, r, kf)
        };
        let part = build(
            &spec(rect(40.0, 30.0, k), t, vec![own(arch(k, 0.0, 10.0))]),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
        let ao = (200.0 + PI * 50.0 - PI * 9.0) * k * k;
        near(
            part.bends[0].start[0],
            10.0 * k,
            1e-12 * k,
            "the bend's start",
        );
        near(part.bends[0].end[0], 30.0 * k, 1e-12 * k, "the bend's end");
        near(
            flat_area(&part),
            w * d + 20.0 * k * ba + ao,
            1e-9 * w * d,
            "flat area",
        );
        let v = t * (w * d + ao) + ring(20.0 * k, r, t);
        near(volume(&part), v, 1e-9 * v, "volume");
        let (lo, hi) = bbox(&part);
        near(lo.y, -(r + t), 10.0 * tol.linear, "the flange's outside");
        near(hi.z, t + r + 20.0 * k, 10.0 * tol.linear, "the arch's top");
        near(
            flange_length(&part, 0).unwrap(),
            20.0 * k,
            1e-9 * k,
            "flange length",
        );
        // The hole in the outline is a hole beside the bend: 10 − 3 from
        // the tangent line.
        let found = holes_to_bends(&part, &tol);
        let per_flat: Vec<usize> = part.flats.iter().map(|f| f.holes.len()).collect();
        assert_eq!(found.len(), 1, "{found:?}, holes per flat {per_flat:?}");
        near(found[0].distance, 7.0 * k, 1e-9 * k, "the hole to the bend");

        // Refused: off the edge's line, past its end, reaching back across
        // it, with insets, to the mould line, and a tip of it.
        let cases: Vec<(FlangeSpec, &str)> = vec![
            (own(arch(k, -1.0, 10.0)), "no line on its edge"),
            (own(arch(k, 0.0, 25.0)), "past the edge's ends"),
            (
                FlangeSpec {
                    inset: [1.0 * k, 0.0],
                    ..own(arch(k, 0.0, 10.0))
                },
                "no insets",
            ),
            (
                FlangeSpec {
                    from: LengthFrom::Mould,
                    ..own(arch(k, 0.0, 10.0))
                },
                "mould",
            ),
        ];
        for (f, why) in cases {
            let got = reason(&spec(rect(40.0, 30.0, k), t, vec![f]), &tol);
            assert!(got.contains(why), "{why}: {got}");
        }
        let back = profile(
            &[&[
                [30.0, 0.0],
                [10.0, 0.0],
                [10.0, -10.0],
                [35.0, -10.0],
                [35.0, 2.0],
            ]],
            k,
        );
        let got = reason(&spec(rect(40.0, 30.0, k), t, vec![own(back)]), &tol);
        assert!(got.contains("reaches back"), "{got}");
        let on_tip = flange(On::Tip(0), 5.0 * k, Direction::Up, r, kf);
        let got = reason(
            &spec(
                rect(40.0, 30.0, k),
                t,
                vec![own(arch(k, 0.0, 10.0)), on_tip],
            ),
            &tol,
        );
        assert!(got.contains("outline of its own"), "{got}");
    }
}

/// Two inset flanges beside a reflex corner of an L-shaped base (the one
/// on the edge leaving the corner inset there, or its strip's end would
/// lie along the other edge): their flats overlap in the flat pattern
/// unless the insets keep them apart.
#[test]
fn inset_flanges_at_a_reflex_corner_are_checked_against_overlapping() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r) = (1.0 * k, 1.0 * k);
        let l_base = || {
            profile(
                &[&[
                    [0.0, 0.0],
                    [40.0, 0.0],
                    [40.0, 10.0],
                    [10.0, 10.0],
                    [10.0, 30.0],
                    [0.0, 30.0],
                ]],
                k,
            )
        };
        let f = |on, a, b| inset(flange(on, 5.0 * k, Direction::Up, r, 0.5), a * k, b * k);
        let why = reason(
            &spec(
                l_base(),
                t,
                vec![f(On::Base(3), 0.0, 2.0), f(On::Base(4), 3.0, 0.0)],
            ),
            &tol,
        );
        assert!(
            why.contains("overlaps itself") && why.contains("flange 0") && why.contains("flange 1"),
            "{why}"
        );
        let part = build(
            &spec(
                l_base(),
                t,
                vec![f(On::Base(3), 0.0, 10.0), f(On::Base(4), 3.0, 0.0)],
            ),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
    }
}

#[test]
fn bad_insets_and_sides_are_refused_by_reason() {
    let k = 1.0;
    let tol = tolerance(k);
    let f = flange(On::Base(1), 10.0, Direction::Up, 1.0, 0.4);
    let cases: Vec<(Vec<FlangeSpec>, &str)> = vec![
        (vec![inset(f.clone(), 20.0, 20.0)], "leave nothing"),
        (vec![inset(f.clone(), -1.0, 0.0)], "inset"),
        (vec![inset(f.clone(), f64::NAN, 0.0)], "inset"),
        (
            vec![FlangeSpec {
                on: On::Side(0, 0),
                ..f.clone()
            }],
            "earlier",
        ),
        (
            vec![
                f.clone(),
                FlangeSpec {
                    on: On::Side(0, 2),
                    ..f.clone()
                },
            ],
            "side is 0",
        ),
        (
            vec![
                f.clone(),
                inset(
                    FlangeSpec {
                        on: On::Side(0, 1),
                        ..f.clone()
                    },
                    6.0,
                    6.0,
                ),
            ],
            "leave nothing",
        ),
    ];
    for (flanges, why) in cases {
        let got = reason(&spec(rect(40.0, 30.0, k), 1.0, flanges), &tol);
        assert!(got.contains(why), "{why}: {got}");
    }
}

/// An inset is 0 (within the tolerance) or at least
/// `SLIVER_TOLERANCES` tolerances, and two bends on one edge are at least
/// that far apart: a piece of the edge between is never a sliver of a
/// rim (review 2026-10-28). Held at three scales: within the tolerance it
/// is the edge's end, at twice the tolerance it is refused, at the floor
/// it is built.
#[test]
fn an_inset_or_a_gap_is_never_a_sliver() {
    use craft3d_kernel::sheetmetal::SLIVER_TOLERANCES;
    for k in SCALES {
        let tol = tolerance(k);
        let e = tol.linear;
        let f = flange(On::Base(1), 10.0 * k, Direction::Up, 1.0 * k, 0.4);
        let floor = SLIVER_TOLERANCES * e;
        let base = || rect(40.0, 30.0, k);
        let snapped = build(
            &spec(base(), 1.0 * k, vec![inset(f.clone(), 0.5 * e, 0.0)]),
            &tol,
        )
        .unwrap();
        assert_eq!(snapped.bends[0].start, [0.0, 0.0], "within the tolerance");
        let got = reason(
            &spec(base(), 1.0 * k, vec![inset(f.clone(), 2.0 * e, 0.0)]),
            &tol,
        );
        assert!(got.contains("narrower than"), "{got}");
        let got = reason(
            &spec(base(), 1.0 * k, vec![inset(f.clone(), 0.0, 2.0 * e)]),
            &tol,
        );
        assert!(got.contains("narrower than"), "{got}");
        let built = build(
            &spec(base(), 1.0 * k, vec![inset(f.clone(), floor, 0.0)]),
            &tol,
        )
        .unwrap();
        healthy(&built, &tol);
        // Two tabs on one edge: 2 tolerances apart refused, the floor built.
        for (gap, ok) in [(2.0 * e, false), (1.01 * floor, true)] {
            let half = 20.0 * k;
            let tabs = vec![
                inset(f.clone(), 0.0, half + 0.5 * gap),
                inset(f.clone(), half + 0.5 * gap, 0.0),
            ];
            let s = spec(base(), 1.0 * k, tabs);
            if ok {
                healthy(&build(&s, &tol).unwrap(), &tol);
            } else {
                let got = reason(&s, &tol);
                assert!(got.contains("apart"), "{got}");
            }
        }
    }
}

/// An outline on a flange's tip is drawn where the tip lies with every
/// bend between the base and it of no width, so changing the thickness,
/// the radius or the K-factor moves it with the tip instead of refusing
/// it (review 2026-10-28): the same arch on the tip of a 10 long wall is
/// built for two thicknesses and radii, its bend where the tip really is
/// (10 plus the wall's bend allowance from the base) and as far reaching.
#[test]
fn an_outline_on_a_tip_moves_with_the_tip() {
    for k in SCALES {
        let tol = tolerance(k);
        for (t, r, kf) in [(1.0 * k, 2.0 * k, 0.4), (2.0 * k, 3.0 * k, 0.33)] {
            let wall = flange(On::Base(1), 10.0 * k, Direction::Up, r, kf);
            let ear = FlangeSpec {
                on: On::Tip(0),
                length: 0.0,
                outline: Some(arch(k, -10.0, 10.0)),
                ..flange(On::Base(1), 0.0, Direction::Up, r, kf)
            };
            let part = build(&spec(rect(40.0, 30.0, k), t, vec![wall, ear]), &tol).unwrap();
            healthy(&part, &tol);
            let ba = FRAC_PI_2 * (r + kf * t);
            near(
                part.bends[1].start[0],
                10.0 * k,
                1e-12 * k,
                "the ear's start",
            );
            near(part.bends[1].end[0], 30.0 * k, 1e-12 * k, "the ear's end");
            near(
                part.bends[1].start[1],
                -(10.0 * k + ba),
                1e-9 * k,
                "the ear's bend on the wall's tip",
            );
            near(
                flange_length(&part, 1).unwrap(),
                20.0 * k,
                1e-9 * k,
                "the ear's reach",
            );
        }
    }
}
