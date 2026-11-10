//! Jogs across a flat (docs/design/sheetmetal.md › *Jogs across a flat*),
//! held to closed forms at 1e-3, 1 and 1e6:
//!
//! - a plate 100 × 40 jogged across at x = 50 by `h` at the angle `α`:
//!   the web `w = (h − (2R + T)(1 − cos α)) / sin α`, the flat length
//!   `100 + 2 BA + w`, the flat area `40 (100 + 2 BA + w)`, the volume
//!   `T 40 (100 + w) + 2 α 40 T (R + T/2)` (exactly the area times `T` at
//!   K = ½), the far side's bottom skin at `z = h`, its near edge at
//!   `x = 50 + (2R + T) sin α + w cos α`; fold then unfold;
//! - the same jog across a wall equals the wall cut short with two tip
//!   flanges (the web and the rest) in area, volume and where the far
//!   side lies;
//! - two jogs, the second across the first's far side; a hole on the far
//!   side carried with it; a flange on a far-side edge;
//! - names: a jog renames only the edges it cuts, and a flange on the far
//!   side keeps every name;
//! - refusals by reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, CornerKind, Direction, FlangeSpec, JogSpec, LengthFrom, On, Part, SheetFace, Spec,
    build, developed_allowance, edge_numbers, flat_area, flat_box, web_length,
};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{FaceId, check, mass_properties};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// A polygon's lines (ids 1, 2, ... from the first point) and circles
/// (ids from 100), scaled by `k`, as one profile.
fn profile(points: &[[f64; 2]], circles: &[([f64; 2], f64)], k: f64) -> Profile {
    let n = points.len();
    let mut curves: Vec<SketchCurve> = (0..n)
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
    for (j, (c, r)) in circles.iter().enumerate() {
        curves.push(SketchCurve {
            id: 100 + j as u64,
            curve: Curve2::Circle {
                center: [c[0] * k, c[1] * k],
                radius: r * k,
            },
        });
    }
    detect_profiles(&curves, &tolerance(k)).unwrap().remove(0)
}

fn plate(k: f64) -> Profile {
    profile(
        &[[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [0.0, 40.0]],
        &[],
        k,
    )
}

fn flange(on: On, length: f64, angle: f64, direction: Direction, r: f64, kf: f64) -> FlangeSpec {
    FlangeSpec {
        on,
        length,
        angle,
        direction,
        radius: r,
        k: kf,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    }
}

/// A jog across the line from `a` to `b` (scaled by `k`).
#[allow(clippy::too_many_arguments)]
fn jog(
    a: [f64; 2],
    b: [f64; 2],
    k: f64,
    h: f64,
    angle: f64,
    r: f64,
    kf: f64,
    ids: [u64; 2],
) -> JogSpec {
    JogSpec {
        line: [[a[0] * k, a[1] * k], [b[0] * k, b[1] * k]],
        offset: h,
        angle,
        direction: Direction::Up,
        radius: r,
        k: kf,
        ids,
    }
}

fn spec(base: Profile, t: f64, flanges: Vec<FlangeSpec>, jogs: Vec<JogSpec>) -> Spec {
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: t,
        flanges,
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs,
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

fn refused(spec: &Spec, tol: &Tolerance) -> (Option<usize>, String) {
    match build(spec, tol) {
        Ok(_) => panic!("the part was built"),
        Err(KernelError::SheetMetal { flange, reason }) => (flange, reason),
        Err(other) => panic!("refused for another reason: {other}"),
    }
}

/// The solid passes the checker; fold then unfold: every folded flat's
/// vertices land on a corner of its outline at z = 0, and each bend's
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
                    .map(|s| (s.start[0] - p.x).phypot(s.start[1] - p.y))
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

/// The web's length, `(h − (2R + T)(1 − cos α)) / sin α`.
fn web(h: f64, alpha: f64, r: f64, t: f64) -> f64 {
    (h - (2.0 * r + t) * (1.0 - alpha.pcos())) / alpha.psin()
}

#[test]
fn a_plate_jogged_across_is_held_to_its_closed_forms() {
    for k in SCALES {
        let tol = tolerance(k);
        for kf in [0.33, 0.5] {
            for alpha in [FRAC_PI_2 / 1.5, FRAC_PI_2] {
                let (t, r, h) = (1.0 * k, 2.0 * k, 10.0 * k);
                let s = spec(
                    plate(k),
                    t,
                    Vec::new(),
                    vec![jog([50.0, -5.0], [50.0, 45.0], k, h, alpha, r, kf, [1, 2])],
                );
                let part = build(&s, &tol).unwrap();
                healthy(&part, &tol);
                let ba = alpha * (r + kf * t);
                let w = web(h, alpha, r, t);
                near(web_length(h, alpha, r, t).unwrap(), w, 1e-12 * k, "the web");
                assert_eq!(part.bends.len(), 2);
                assert_eq!(part.flats.len(), 3);
                near(
                    part.bends[0].flat_length,
                    w,
                    4.0 * tol.linear,
                    "the web's flat",
                );
                let ([x0, y0, x1, y1], exact) = flat_box(&part, &tol).unwrap();
                assert!(exact);
                near(
                    x1 - x0,
                    100.0 * k + 2.0 * ba + w,
                    4.0 * tol.linear,
                    "flat length",
                );
                near(y1 - y0, 40.0 * k, 4.0 * tol.linear, "flat width");
                let area = 40.0 * k * (100.0 * k + 2.0 * ba + w);
                near(flat_area(&part), area, 1e-9 * area, "flat area");
                let v = t * 40.0 * k * (100.0 * k + w) + 2.0 * alpha * 40.0 * k * t * (r + 0.5 * t);
                near(volume(&part), v, 1e-7 * v, "volume");
                if kf == 0.5 {
                    near(
                        volume(&part),
                        area * t,
                        1e-7 * v,
                        "volume = area T at K = 1/2",
                    );
                }
                // The far side lies parallel, its bottom skin at z = h,
                // its near edge where the two bends and the web put it.
                let far = &part.flats[2];
                let n = far.transform.vector(Vec3::Z);
                near(n.z, 1.0, 1e-9, "the far side's normal");
                let c = far.outer[0].end; // c₁ carried: the far side's corner at y = 0
                let p = far.transform.point(Vec3::new(c[0], c[1], 0.0));
                near(p.z, h, 4.0 * tol.linear, "the step");
                let run = (2.0 * r + t) * alpha.psin() + w * alpha.pcos();
                near(
                    p.x,
                    50.0 * k + run,
                    4.0 * tol.linear,
                    "the far side's near edge",
                );
                near(p.y, 0.0, 4.0 * tol.linear, "the far side's corner");
            }
        }
    }
}

#[test]
fn a_jog_down_steps_down() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, h) = (1.0 * k, 2.0 * k, 10.0 * k);
        let mut j = jog([50.0, -5.0], [50.0, 45.0], k, h, FRAC_PI_2, r, 0.4, [1, 2]);
        j.direction = Direction::Down;
        let part = build(&spec(plate(k), t, Vec::new(), vec![j]), &tol).unwrap();
        healthy(&part, &tol);
        let far = &part.flats[2];
        let c = far.outer[0].end;
        let p = far.transform.point(Vec3::new(c[0], c[1], 0.0));
        near(p.z, -h, 4.0 * tol.linear, "the step down");
        near(
            p.x,
            50.0 * k + 2.0 * r + t,
            4.0 * tol.linear,
            "a square jog's run",
        );
    }
}

/// A jog across a wall at `a` from its bend's tangent line is the wall cut
/// to `a` with a tip flange the web long and one the rest long on it.
#[test]
fn a_jog_across_a_wall_equals_two_tip_flanges() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, kf, h, alpha) = (1.0 * k, 2.0 * k, 0.33, 6.0 * k, PI / 3.0);
        let (wall, a) = (30.0 * k, 12.0 * k);
        // The wall on the top edge (3: (100, 40) → (0, 40)) grows to +y;
        // drawn with its bend of no width it lies over y in [40, 70].
        let jogged = spec(
            plate(k),
            t,
            vec![flange(On::Base(3), wall, FRAC_PI_2, Direction::Up, r, kf)],
            vec![jog([-5.0, 52.0], [105.0, 52.0], k, h, alpha, r, kf, [5, 6])],
        );
        let w = web(h, alpha, r, t);
        let tips = spec(
            plate(k),
            t,
            vec![
                flange(On::Base(3), a, FRAC_PI_2, Direction::Up, r, kf),
                flange(On::Tip(0), w, alpha, Direction::Up, r, kf),
                flange(On::Tip(1), wall - a, alpha, Direction::Down, r, kf),
            ],
            Vec::new(),
        );
        let one = build(&jogged, &tol).unwrap();
        let two = build(&tips, &tol).unwrap();
        healthy(&one, &tol);
        near(
            flat_area(&one),
            flat_area(&two),
            1e-9 * flat_area(&two),
            "flat area",
        );
        near(volume(&one), volume(&two), 1e-7 * volume(&two), "volume");
        // The far side lies where the last tip flange does.
        let probe = |part: &Part, f: usize| {
            let flat = &part.flats[f];
            let mut lo = [f64::INFINITY; 3];
            for s in &flat.outer {
                let p = flat.transform.point(Vec3::new(s.start[0], s.start[1], 0.0));
                lo = [lo[0].min(p.x), lo[1].min(p.y), lo[2].min(p.z)];
            }
            lo
        };
        let (p, q) = (probe(&one, 3), probe(&two, 3));
        for i in 0..3 {
            near(p[i], q[i], 8.0 * tol.linear, "the far side's place");
        }
    }
}

#[test]
fn two_jogs_a_hole_and_a_flange_on_the_far_side() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, kf, h, alpha) = (1.0 * k, 2.0 * k, 0.5, 8.0 * k, FRAC_PI_2);
        // A hole of radius 5 at (80, 20), on the far side of both jogs.
        let base = profile(
            &[[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [0.0, 40.0]],
            &[([85.0, 20.0], 5.0)],
            k,
        );
        let jogs = vec![
            jog([30.0, -5.0], [30.0, 45.0], k, h, alpha, r, kf, [11, 12]),
            jog([60.0, -5.0], [60.0, 45.0], k, h, alpha, r, kf, [13, 14]),
        ];
        let wall = flange(On::Base(2), 15.0 * k, FRAC_PI_2, Direction::Up, r, kf);
        let part = build(&spec(base, t, vec![wall], jogs), &tol).unwrap();
        healthy(&part, &tol);
        let ba = alpha * (r + kf * t);
        let w = web(h, alpha, r, t);
        // The wall's flat 15 long beyond its strip; the hole.
        let area = 40.0 * k * (100.0 * k + 2.0 * (2.0 * ba + w)) - PI * 25.0 * k * k
            + 40.0 * k * (ba + 15.0 * k);
        near(flat_area(&part), area, 1e-9 * area, "flat area");
        near(
            volume(&part),
            area * t,
            1e-7 * area * t,
            "volume at K = 1/2",
        );
        // The second jog's far side (flat 1 + 2·1 + 2 = 5) holds the
        // hole and carries the wall: two steps up.
        let far = &part.flats[5];
        assert_eq!(far.holes.len(), 1);
        let wall_flat = &part.flats[1];
        let top = wall_flat
            .outer
            .iter()
            .map(|s| {
                wall_flat
                    .transform
                    .point(Vec3::new(s.start[0], s.start[1], 0.0))
                    .z
            })
            .fold(f64::NEG_INFINITY, f64::max);
        near(
            top,
            2.0 * h + t + r + 15.0 * k,
            8.0 * tol.linear,
            "the wall's top",
        );
    }
}

#[test]
fn a_jog_renames_only_the_edges_it_cuts() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, kf) = (1.0 * k, 2.0 * k, 0.4);
        let wall = || flange(On::Base(2), 15.0 * k, FRAC_PI_2, Direction::Up, r, kf);
        let plain = build(&spec(plate(k), t, vec![wall()], Vec::new()), &tol).unwrap();
        let j = jog(
            [50.0, -5.0],
            [50.0, 45.0],
            k,
            9.0 * k,
            PI / 3.0,
            r,
            kf,
            [7, 8],
        );
        let jogged = build(&spec(plate(k), t, vec![wall()], vec![j.clone()]), &tol).unwrap();
        let (before, after) = (names(&plain), names(&jogged));
        // The two edges the line cuts (curves 1 and 3) are gone, every
        // other name is kept.
        let cut: BTreeSet<Vec<u64>> = [vec![1, 1, 0, 0], vec![1, 3, 0, 0]].into();
        for name in before.difference(&cut) {
            assert!(after.contains(name), "{name:?} was renamed by the jog");
        }
        for name in &cut {
            assert!(!after.contains(name), "{name:?} is still there");
        }
        // The new names are the jog's (its pieces, bends, web).
        for name in after.difference(&before) {
            assert!(
                matches!(name[0], 15 | 7 | 8 | 3) && (name[1] == 6 || name[1] == 7),
                "{name:?} is no name of the jog"
            );
        }
        // A second jog renames only what it cuts too.
        let j2 = jog(
            [75.0, -5.0],
            [75.0, 45.0],
            k,
            9.0 * k,
            PI / 3.0,
            r,
            kf,
            [9, 10],
        );
        let twice = build(&spec(plate(k), t, vec![wall()], vec![j.clone(), j2]), &tol).unwrap();
        let again = names(&twice);
        let gone: Vec<&Vec<u64>> = after.difference(&again).collect();
        assert!(
            gone.iter().all(|n| n[0] == 15),
            "the second jog renamed {gone:?}"
        );
        // The same part twice is the same.
        let other = build(&spec(plate(k), t, vec![wall()], vec![j]), &tol).unwrap();
        assert_eq!(other.solid.body, jogged.solid.body);
    }
}

#[test]
fn jogs_are_refused_with_the_reason() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, kf) = (1.0 * k, 2.0 * k, 0.4);
        let at = |a: [f64; 2], b: [f64; 2], h: f64, alpha: f64| {
            jog(a, b, k, h, alpha, r, kf, [101, 102])
        };
        let cases: Vec<(Spec, &str)> = vec![
            (
                spec(
                    plate(k),
                    t,
                    Vec::new(),
                    vec![at([50.0, -5.0], [50.0, 45.0], 1.0 * k, PI / 3.0)],
                ),
                "leaves a web",
            ),
            (
                spec(
                    plate(k),
                    t,
                    Vec::new(),
                    vec![at([-10.0, -4.0], [110.0, 44.0], 9.0 * k, PI / 3.0)],
                ),
                "corner of the flat",
            ),
            (
                spec(
                    plate(k),
                    t,
                    Vec::new(),
                    vec![at([150.0, -5.0], [150.0, 45.0], 9.0 * k, PI / 3.0)],
                ),
                "lies in no flat",
            ),
            (
                spec(
                    plate(k),
                    t,
                    Vec::new(),
                    vec![at([50.0, -5.0], [50.0, 45.0], 9.0 * k, PI)],
                ),
                "between 0 and 180",
            ),
            (
                spec(
                    plate(k),
                    t,
                    vec![flange(
                        On::Base(1),
                        10.0 * k,
                        FRAC_PI_2,
                        Direction::Up,
                        r,
                        kf,
                    )],
                    vec![at([50.0, -5.0], [50.0, 45.0], 9.0 * k, PI / 3.0)],
                ),
                "cut by a jog",
            ),
            (
                // A U: the line across both arms crosses four times.
                spec(
                    profile(
                        &[
                            [0.0, 0.0],
                            [100.0, 0.0],
                            [100.0, 40.0],
                            [70.0, 40.0],
                            [70.0, 10.0],
                            [30.0, 10.0],
                            [30.0, 40.0],
                            [0.0, 40.0],
                        ],
                        &[],
                        k,
                    ),
                    t,
                    Vec::new(),
                    vec![at([-5.0, 20.0], [35.0, 20.0], 9.0 * k, PI / 3.0)],
                ),
                "crosses the outline",
            ),
            (
                spec(
                    profile(
                        &[[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [0.0, 40.0]],
                        // A hole across the line is built
                        // (sheetmetal_holes_more.rs); one the line only
                        // touches is refused.
                        &[([55.0, 20.0], 5.0)],
                        k,
                    ),
                    t,
                    Vec::new(),
                    vec![at([50.0, -5.0], [50.0, 30.0], 9.0 * k, PI / 3.0)],
                ),
                "touches the circle",
            ),
        ];
        for (s, why) in cases {
            let (flange, reason) = refused(&s, &tol);
            assert!(reason.contains(why), "{reason} does not say {why}");
            assert!(flange.is_some(), "{reason} names nothing");
        }
        // A closed corner of a jogged wall.
        let mut s = spec(
            plate(k),
            t,
            vec![
                flange(On::Base(2), 30.0 * k, FRAC_PI_2, Direction::Up, r, kf),
                flange(On::Base(3), 30.0 * k, FRAC_PI_2, Direction::Up, r, kf),
            ],
            // Across wall 1 (on the top edge, drawn over y in [40, 70]).
            vec![at([20.0, 55.0], [-5.0, 55.0], 9.0 * k, PI / 3.0)],
        );
        s.corner = Closing {
            kind: CornerKind::Overlap,
            gap: 0.1 * k,
            over: None,
        };
        s.relief = Some(craft3d_kernel::sheetmetal::Relief {
            kind: craft3d_kernel::sheetmetal::ReliefKind::Rect,
            size: 2.0 * k,
        });
        let (_, reason) = refused(&s, &tol);
        assert!(reason.contains("jog crosses its flat"), "{reason}");
        // Ids: a jog's taken by a flange.
        let s = spec(
            plate(k),
            t,
            vec![flange(
                On::Base(2),
                10.0 * k,
                FRAC_PI_2,
                Direction::Up,
                r,
                kf,
            )],
            vec![jog(
                [50.0, -5.0],
                [50.0, 45.0],
                k,
                9.0 * k,
                PI / 3.0,
                r,
                kf,
                [1, 2],
            )],
        );
        let (_, reason) = refused(&s, &tol);
        assert!(reason.contains("another jog's"), "{reason}");
        // A line crossing a rounded corner's arc two tolerances from its
        // start leaves an arc piece shorter than the sliver floor:
        // refused, as a line's piece is (review of 47275cf1: arcs were
        // let through).
        let rounded = |k: f64| {
            let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
            let curves = vec![
                SketchCurve {
                    id: 1,
                    curve: Curve2::Line {
                        start: s([0.0, 0.0]),
                        end: s([90.0, 0.0]),
                    },
                },
                SketchCurve {
                    id: 2,
                    curve: Curve2::Arc {
                        center: s([90.0, 10.0]),
                        start: s([90.0, 0.0]),
                        end: s([100.0, 10.0]),
                    },
                },
                SketchCurve {
                    id: 3,
                    curve: Curve2::Line {
                        start: s([100.0, 10.0]),
                        end: s([100.0, 40.0]),
                    },
                },
                SketchCurve {
                    id: 4,
                    curve: Curve2::Line {
                        start: s([100.0, 40.0]),
                        end: s([0.0, 40.0]),
                    },
                },
                SketchCurve {
                    id: 5,
                    curve: Curve2::Line {
                        start: s([0.0, 40.0]),
                        end: s([0.0, 0.0]),
                    },
                },
            ];
            detect_profiles(&curves, &tolerance(k)).unwrap().remove(0)
        };
        let x = 90.0 + 2e-6;
        let s = spec(
            rounded(k),
            t,
            Vec::new(),
            vec![at([x, -5.0], [x, 45.0], 9.0 * k, PI / 3.0)],
        );
        let (_, reason) = refused(&s, &tol);
        assert!(reason.contains("of curve 2"), "{reason}");
        // Clear of the arc's end it is built.
        let s = spec(
            rounded(k),
            t,
            Vec::new(),
            vec![at([92.0, -5.0], [92.0, 45.0], 9.0 * k, PI / 3.0)],
        );
        build(&s, &tol).unwrap_or_else(|e| panic!("scale {k}: {e}"));
    }
}
