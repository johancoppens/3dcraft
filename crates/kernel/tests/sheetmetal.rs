//! Sheet metal (docs/design/sheetmetal.md): flanges bent with an inner
//! radius and a K-factor, folded by the origami linkage and plated, held
//! to closed forms at 1e-3, 1 and 1e6:
//!
//! - the flat length of an L, a U, a Z and a box is the base and the
//!   flanges' flats plus `Σ θᵢ (Rᵢ + Kᵢ T)` (the bend allowance);
//! - the folded part's box by the bend's geometry (the flange's outside at
//!   `R + T` from the edge, its top at `T + R + L`; to the mould line, `L`);
//! - the volume: the flats' slabs and per bend `θ w ((R + T)² − R²) / 2`;
//!   that is `A T` of the flat area exactly when `K = ½` and less by
//!   `θ w T² (½ − K)` per bend otherwise (the model keeps the skins at `R`
//!   and `R + T`; the neutral fibre is where the length is kept);
//! - the bend's inner skin has the radius `R`, the flats meet at θ, and the
//!   neutral band developed by the unfold is `θ (R + K T)` wide;
//! - fold then unfold: the folded flats mapped back by the inverse motions
//!   land on the pattern;
//! - corner reliefs (rect, round, tear) by the area they take out;
//! - the hole-to-bend distance of a round hole;
//! - refusals by reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::plate::Made;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Direction, FlangeSpec, LengthFrom, On, Part, Relief, ReliefKind, SheetFace, Spec, build,
    developed_allowance, flange_length, flat_area, holes_to_bends, measured_angle, measured_radius,
    outline,
};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{FaceId, check, mass_properties};
use std::f64::consts::{FRAC_PI_2, PI};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn line(id: u64, start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// The rectangle `0..w` × `0..d` (curves 1 to 4 from the bottom edge
/// counter-clockwise), with an optional round hole (curve 9).
fn base(w: f64, d: f64, hole: Option<([f64; 2], f64)>, k: f64) -> Profile {
    let c = [[0.0, 0.0], [w * k, 0.0], [w * k, d * k], [0.0, d * k]];
    let mut curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
        .collect();
    if let Some((center, radius)) = hole {
        curves.push(SketchCurve {
            id: 9,
            curve: Curve2::Circle {
                center: [center[0] * k, center[1] * k],
                radius: radius * k,
            },
        });
    }
    let mut profiles = detect_profiles(&curves, &tolerance(k)).unwrap();
    assert_eq!(profiles.len(), 1);
    profiles.remove(0)
}

fn flange(
    on: On,
    length: f64,
    angle: f64,
    direction: Direction,
    radius: f64,
    k: f64,
) -> FlangeSpec {
    FlangeSpec {
        on,
        length,
        angle,
        direction,
        radius,
        k,
        from: LengthFrom::Tangent,
    }
}

fn spec(base: Profile, t: f64, flanges: Vec<FlangeSpec>, relief: Option<Relief>) -> Spec {
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: t,
        flanges,
        relief,
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

/// The extent of the flat pattern along `axis` (0 x, 1 y).
fn flat_extent(part: &Part, axis: usize) -> f64 {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for flat in &part.flats {
        for s in &flat.outer {
            for p in [s.start, s.end] {
                lo = lo.min(p[axis]);
                hi = hi.max(p[axis]);
            }
        }
    }
    hi - lo
}

fn healthy(part: &Part, tol: &Tolerance) {
    let defects = check(&part.solid.body, tol);
    assert!(
        defects.is_empty(),
        "the solid fails the checker: {defects:?}"
    );
    assert_eq!(part.sheet_faces.len(), part.sheet.faces.len());
}

/// Fold then unfold: every folded flat's vertices, carried back by the
/// inverse of its motion, lie at z = 0 on the flat's outline in the
/// pattern; and each bend's strip, developed on its neutral cylinder, is
/// as wide as the pattern has it.
fn round_trip(part: &Part, tol: &Tolerance) {
    for (f, role) in part.sheet_faces.iter().enumerate() {
        let SheetFace::Flat(i) = role else { continue };
        let flat = &part.flats[*i];
        let back = flat.transform.inverse().unwrap();
        let face = FaceId(f as u32);
        let f_ = part.sheet.face(face).unwrap();
        let vertices: Vec<_> = f_
            .loops
            .iter()
            .flat_map(|l| part.sheet.loop_(*l).unwrap().coedges.clone())
            .map(|c| part.sheet.coedge_ends(c).unwrap().0)
            .collect();
        for v in vertices {
            let world = part.sheet.vertex(v).unwrap().point;
            let (x, y, z) = part.plane.local(world);
            let p = back.point(Vec3::new(x, y, z));
            assert!(
                p.z.abs() <= tol.linear,
                "a flat's vertex is off the pattern's plane by {}",
                p.z
            );
            let on = flat
                .outer
                .iter()
                .chain(flat.holes.iter().flatten())
                .map(|s| (s.start[0] - p.x).hypot(s.start[1] - p.y))
                .fold(f64::INFINITY, f64::min);
            assert!(
                on <= tol.linear,
                "a flat's vertex lands {on} from its corner in the pattern"
            );
        }
    }
    for b in &part.bends {
        let developed = developed_allowance(b, tol).unwrap();
        near(
            developed,
            b.allowance,
            4.0 * tol.linear,
            "the developed bend allowance",
        );
    }
}

#[test]
fn an_l_bracket_by_its_closed_forms() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, d, t, r, l) = (40.0 * k, 30.0 * k, 2.0 * k, 3.0 * k, 25.0 * k);
        for kf in [0.0, 0.33, 0.5, 1.0] {
            let part = build(
                &spec(
                    base(40.0, 30.0, None, k),
                    t,
                    vec![flange(On::Base(1), l, FRAC_PI_2, Direction::Up, r, kf)],
                    None,
                ),
                &tol,
            )
            .unwrap();
            healthy(&part, &tol);
            let theta = FRAC_PI_2;
            let ba = theta * (r + kf * t);
            near(part.bends[0].allowance, ba, 1e-12 * k, "BA");
            // The flat pattern: the base, the strip and the flange in line.
            near(flat_extent(&part, 1), d + ba + l, 1e-9 * k, "flat length");
            near(
                flat_area(&part),
                w * (d + ba + l),
                1e-9 * k * k * 1e3,
                "flat area",
            );
            // The part: the flange's outside at y = −(R + T), its top at
            // z = T + R + L.
            let (lo, hi) = bbox(&part);
            near(lo.y, -(r + t), 10.0 * tol.linear, "the flange's outside");
            near(hi.z, t + r + l, 10.0 * tol.linear, "the flange's top");
            near(hi.y, d, 10.0 * tol.linear, "the base's far edge");
            // The volume: two slabs and the bend's quarter ring.
            let v = w * t * (d + l) + theta * w * ((r + t).powi(2) - r * r) / 2.0;
            let rel = 1e-9 * v;
            near(volume(&part), v, rel, "volume");
            // A T of the flat pattern, exactly at K = ½.
            let at = flat_area(&part) * t;
            near(
                volume(&part) - at,
                theta * w * t * t * (0.5 - kf),
                rel,
                "volume − A T",
            );
            near(
                measured_radius(&part, 0).unwrap(),
                r,
                1e-12 * k,
                "the inner radius",
            );
            near(measured_angle(&part, 0).unwrap(), theta, 1e-12, "the angle");
            near(
                flange_length(&part, 0).unwrap(),
                l,
                1e-9 * k,
                "the flange length",
            );
            round_trip(&part, &tol);
        }
    }
}

#[test]
fn a_flange_measured_to_the_mould_line_reaches_its_length_from_the_virtual_sharp() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, l) = (1.5 * k, 2.0 * k, 20.0 * k);
        for (deg, dir) in [
            (90.0f64, Direction::Up),
            (60.0, Direction::Up),
            (120.0, Direction::Down),
        ] {
            let theta = deg.to_radians();
            let mut f = flange(On::Base(1), l, theta, dir, r, 0.4);
            f.from = LengthFrom::Mould;
            let part = build(&spec(base(30.0, 20.0, None, k), t, vec![f], None), &tol).unwrap();
            healthy(&part, &tol);
            let setback = (r + t) * (theta / 2.0).tan();
            near(
                part.bends[0].flat_length,
                l - setback,
                1e-9 * k,
                "the flat beyond the strip",
            );
            if deg == 90.0 {
                // The outside of the flange reaches L above the base's
                // bottom (the virtual sharp of the outside surfaces).
                let (_, hi) = bbox(&part);
                near(hi.z, l, 10.0 * tol.linear, "the mould line's length");
            }
            let bd = part.bends[0].deduction(tol.angular).unwrap();
            near(
                bd,
                2.0 * setback - part.bends[0].allowance,
                1e-12 * k,
                "bend deduction",
            );
        }
    }
}

#[test]
fn a_u_channel_and_a_z_by_their_closed_forms() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, d, t, r, l) = (50.0 * k, 20.0 * k, 1.0 * k, 1.5 * k, 15.0 * k);
        let kf = 0.42;
        let ba = FRAC_PI_2 * (r + kf * t);
        // U: flanges on the bottom and the top edge, both up.
        let part = build(
            &spec(
                base(50.0, 20.0, None, k),
                t,
                vec![
                    flange(On::Base(1), l, FRAC_PI_2, Direction::Up, r, kf),
                    flange(On::Base(3), l, FRAC_PI_2, Direction::Up, r, kf),
                ],
                None,
            ),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
        near(
            flat_extent(&part, 1),
            d + 2.0 * (ba + l),
            1e-9 * k,
            "U flat width",
        );
        let (lo, hi) = bbox(&part);
        near(lo.y, -(r + t), 10.0 * tol.linear, "U outside");
        near(hi.y, d + r + t, 10.0 * tol.linear, "U outside, far");
        near(hi.z, t + r + l, 10.0 * tol.linear, "U height");
        let v = w * t * (d + 2.0 * l) + 2.0 * FRAC_PI_2 * w * ((r + t).powi(2) - r * r) / 2.0;
        near(volume(&part), v, 1e-9 * v, "U volume");
        round_trip(&part, &tol);

        // Z: up 90 on the bottom edge, then down 90 on that flange's tip.
        let (r1, l1) = (2.5 * k, 12.0 * k);
        let ba1 = FRAC_PI_2 * (r1 + kf * t);
        let part = build(
            &spec(
                base(50.0, 20.0, None, k),
                t,
                vec![
                    flange(On::Base(1), l, FRAC_PI_2, Direction::Up, r, kf),
                    flange(On::Tip(0), l1, FRAC_PI_2, Direction::Down, r1, kf),
                ],
                None,
            ),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
        near(
            flat_extent(&part, 1),
            d + ba + l + ba1 + l1,
            1e-9 * k,
            "Z flat length",
        );
        let (lo, hi) = bbox(&part);
        near(hi.z, t + r + l + r1 + t, 10.0 * tol.linear, "Z top");
        near(lo.y, -(r + t) - r1 - l1, 10.0 * tol.linear, "Z reach");
        near(
            measured_radius(&part, 1).unwrap(),
            r1,
            1e-12 * k,
            "Z second radius",
        );
        round_trip(&part, &tol);
    }
}

#[test]
fn a_box_with_four_flanges_and_corner_reliefs() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, d, t, r, l, s) = (60.0 * k, 40.0 * k, 1.0 * k, 1.0 * k, 10.0 * k, 3.0 * k);
        let kf = 0.5;
        let ba = FRAC_PI_2 * (r + kf * t);
        let flanges: Vec<FlangeSpec> = (1..=4)
            .map(|c| flange(On::Base(c), l, FRAC_PI_2, Direction::Up, r, kf))
            .collect();
        for (relief, taken) in [
            (None, 0.0),
            (Some(ReliefKind::Rect), s * s),
            (Some(ReliefKind::Round), PI * s * s / 4.0),
            (Some(ReliefKind::Tear), s * s + 0.75 * PI * s * s),
        ] {
            let part = build(
                &spec(
                    base(60.0, 40.0, None, k),
                    t,
                    flanges.clone(),
                    relief.map(|kind| Relief { kind, size: s }),
                ),
                &tol,
            )
            .unwrap();
            healthy(&part, &tol);
            assert_eq!(part.corners.len(), 4);
            assert!(
                part.corners
                    .iter()
                    .all(|c| c.relief.is_some() == relief.is_some())
            );
            let short = if relief.is_some() { 2.0 * s } else { 0.0 };
            let area =
                w * d - 4.0 * taken + 2.0 * (w - short) * (ba + l) + 2.0 * (d - short) * (ba + l);
            near(flat_area(&part), area, 1e-9 * area, "the box's flat area");
            // K = ½: the volume is the flat area times the thickness.
            near(volume(&part), area * t, 1e-9 * area * t, "the box's volume");
            let (_, hi) = bbox(&part);
            near(hi.z, t + r + l, 10.0 * tol.linear, "the box's height");
            // The outline is one closed chain.
            let chains = outline(&part, &tol);
            assert_eq!(chains.len(), 1, "one outer chain and no holes");
            let chain = &chains[0];
            let (first, last) = (chain.first().unwrap(), chain.last().unwrap());
            assert!(
                (first.start[0] - last.end[0]).hypot(first.start[1] - last.end[1]) <= tol.linear
            );
            round_trip(&part, &tol);
        }
    }
}

#[test]
fn the_distance_from_a_hole_to_the_bend() {
    for k in SCALES {
        let tol = tolerance(k);
        let part = build(
            &spec(
                base(40.0, 30.0, Some(([20.0, 7.0], 2.5)), k),
                1.0 * k,
                vec![
                    flange(
                        On::Base(1),
                        10.0 * k,
                        FRAC_PI_2,
                        Direction::Up,
                        1.0 * k,
                        0.4,
                    ),
                    flange(
                        On::Base(2),
                        10.0 * k,
                        FRAC_PI_2,
                        Direction::Down,
                        1.0 * k,
                        0.4,
                    ),
                ],
                None,
            ),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
        let found = holes_to_bends(&part, &tol);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].bend, 0);
        assert!(found[0].exact);
        near(found[0].distance, 4.5 * k, 1e-12 * k, "hole to bend");
        // The volume less the hole.
        let (w, d, t, r, l) = (40.0 * k, 30.0 * k, 1.0 * k, 1.0 * k, 10.0 * k);
        let ring = FRAC_PI_2 * ((r + t).powi(2) - r * r) / 2.0;
        let v = (w * d - PI * (2.5 * k).powi(2)) * t + (w + d) * (l * t + ring);
        near(volume(&part), v, 1e-6 * v, "volume with a hole");
    }
}

#[test]
fn a_hem_and_a_sharp_angle_fold_back() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r) = (1.0 * k, 0.8 * k);
        let part = build(
            &spec(
                base(30.0, 20.0, None, k),
                t,
                vec![flange(On::Base(1), 8.0 * k, PI, Direction::Up, r, 0.45)],
                None,
            ),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
        // Folded back over the base: its bottom skin at z = 2 (R + T), on
        // top of the base's top skin `T` with a gap of 2R.
        let (lo, hi) = bbox(&part);
        near(hi.z, 2.0 * (r + t), 10.0 * tol.linear, "the hem's top");
        near(lo.y, -(r + t), 10.0 * tol.linear, "the hem's round");
        near(
            measured_angle(&part, 0).unwrap(),
            PI,
            1e-9,
            "the hem's angle",
        );
        round_trip(&part, &tol);
    }
}

#[test]
fn faces_are_the_flats_and_the_bends_and_the_plate_names_them() {
    let tol = tolerance(1.0);
    let part = build(
        &spec(
            base(40.0, 30.0, None, 1.0),
            2.0,
            vec![
                flange(On::Base(1), 20.0, FRAC_PI_2, Direction::Up, 3.0, 0.4),
                flange(On::Tip(0), 10.0, FRAC_PI_2 / 2.0, Direction::Down, 2.0, 0.4),
            ],
            None,
        ),
        &tol,
    )
    .unwrap();
    assert_eq!(
        part.sheet_faces,
        vec![
            SheetFace::Flat(0),
            SheetFace::Flat(1),
            SheetFace::Flat(2),
            SheetFace::Bend(0),
            SheetFace::Bend(1)
        ]
    );
    // Every face of the solid comes from a face or an edge of the sheet:
    // two skins per face of the sheet.
    let skins = part
        .solid
        .faces
        .iter()
        .filter(|m| matches!(m, Made::Front(_) | Made::Back(_)))
        .count();
    assert_eq!(skins, 2 * part.sheet.faces.len());
    // The same part twice is the same body.
    let again = build(
        &spec(
            base(40.0, 30.0, None, 1.0),
            2.0,
            vec![
                flange(On::Base(1), 20.0, FRAC_PI_2, Direction::Up, 3.0, 0.4),
                flange(On::Tip(0), 10.0, FRAC_PI_2 / 2.0, Direction::Down, 2.0, 0.4),
            ],
            None,
        ),
        &tol,
    )
    .unwrap();
    assert_eq!(
        format!("{:?}", part.solid.body),
        format!("{:?}", again.solid.body)
    );
}

#[test]
fn twenty_bends_build_quickly() {
    // A base with ten flanges on a zigzag of edges... a simple stand-in:
    // a rectangle's bottom edge with a chain of 20 flanges, each on the
    // tip of the one before, alternately up and down (a staircase).
    let tol = tolerance(1.0);
    let mut flanges = vec![flange(
        On::Base(1),
        10.0,
        FRAC_PI_2,
        Direction::Up,
        1.0,
        0.4,
    )];
    for i in 1..20 {
        let dir = if i % 2 == 1 {
            Direction::Down
        } else {
            Direction::Up
        };
        flanges.push(flange(On::Tip(i - 1), 10.0, FRAC_PI_2, dir, 1.0, 0.4));
    }
    let started = std::time::Instant::now();
    let part = build(&spec(base(40.0, 30.0, None, 1.0), 1.0, flanges, None), &tol).unwrap();
    let took = started.elapsed();
    eprintln!(
        "twenty bends: {took:?}, {} faces",
        part.solid.body.faces.len()
    );
    healthy(&part, &tol);
    let ba = FRAC_PI_2 * (1.0 + 0.4);
    near(
        flat_extent(&part, 1),
        30.0 + 20.0 * (ba + 10.0),
        1e-9,
        "flat length",
    );
    round_trip(&part, &tol);
}

#[test]
fn refusals_by_reason() {
    let tol = tolerance(1.0);
    let reason = |flanges: Vec<FlangeSpec>, relief: Option<Relief>| -> String {
        match build(
            &spec(
                base(40.0, 30.0, Some(([20.0, 15.0], 3.0)), 1.0),
                1.0,
                flanges,
                relief,
            ),
            &tol,
        ) {
            Ok(_) => panic!("built"),
            Err(KernelError::SheetMetal { reason, .. }) => reason,
            Err(other) => format!("other: {other}"),
        }
    };
    let f = |on, radius, k, angle| flange(on, 10.0, angle, Direction::Up, radius, k);
    assert!(reason(vec![f(On::Base(1), -1.0, 0.4, 1.0)], None).contains("below zero"));
    assert!(reason(vec![f(On::Base(1), 0.0, 0.4, 1.0)], None).contains("sharp bend"));
    assert!(reason(vec![f(On::Base(1), 1.0, 1.5, 1.0)], None).contains("K-factor"));
    assert!(reason(vec![f(On::Base(1), 1.0, -0.1, 1.0)], None).contains("K-factor"));
    assert!(reason(vec![f(On::Base(1), 1.0, 0.4, 0.0)], None).contains("angle"));
    assert!(reason(vec![f(On::Base(1), 1.0, 0.4, 3.5)], None).contains("angle"));
    assert!(reason(vec![f(On::Base(9), 1.0, 0.4, 1.0)], None).contains("hole"));
    assert!(reason(vec![f(On::Base(7), 1.0, 0.4, 1.0)], None).contains("no edge"));
    assert!(reason(vec![f(On::Tip(0), 1.0, 0.4, 1.0)], None).contains("earlier"));
    assert!(
        reason(
            vec![f(On::Base(1), 1.0, 0.4, 1.0), f(On::Base(1), 1.0, 0.4, 1.0)],
            None
        )
        .contains("has a flange already")
    );
    assert!(
        reason(
            vec![f(On::Base(1), 1.0, 0.4, 1.0), f(On::Base(2), 1.0, 0.4, 1.0)],
            Some(Relief {
                kind: ReliefKind::Rect,
                size: 35.0
            })
        )
        .contains("does not fit")
    );
    let mut mould = f(On::Base(1), 1.0, 0.4, PI);
    mould.from = LengthFrom::Mould;
    assert!(reason(vec![mould], None).contains("mould"));
    let mut short = f(On::Base(1), 3.0, 0.4, FRAC_PI_2);
    short.from = LengthFrom::Mould;
    short.length = 2.0;
    assert!(reason(vec![short], None).contains("setback"));

    // A reflex corner: an L-shaped base with flanges on both edges at its
    // inner corner.
    let pts = [
        [0.0, 0.0],
        [40.0, 0.0],
        [40.0, 10.0],
        [10.0, 10.0],
        [10.0, 30.0],
        [0.0, 30.0],
    ];
    let curves: Vec<SketchCurve> = (0..6)
        .map(|i| line(i as u64 + 1, pts[i], pts[(i + 1) % 6]))
        .collect();
    let l_base = detect_profiles(&curves, &tol).unwrap().remove(0);
    let err = build(
        &spec(
            l_base,
            1.0,
            vec![
                flange(On::Base(3), 5.0, FRAC_PI_2, Direction::Up, 1.0, 0.4),
                flange(On::Base(4), 5.0, FRAC_PI_2, Direction::Up, 1.0, 0.4),
            ],
            None,
        ),
        &tol,
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("reflex"), "{err}");
}

/// A hole bounded by a cubic spline whose lowest point lies between any
/// fixed samples: the distance to the bend is searched to an eighth of the
/// tolerance, `lower` is proven below the true one, and it is not called
/// exact.
#[test]
fn the_distance_from_a_spline_hole_to_the_bend_is_bounded() {
    use craft3d_kernel::profile::Spline2;
    for k in SCALES {
        let tol = tolerance(k);
        let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
        let control = [[20.0, 15.0], [24.0, 2.0], [33.0, 4.0], [40.0, 15.0]];
        let c = [
            [0.0, 0.0],
            [60.0 * k, 0.0],
            [60.0 * k, 40.0 * k],
            [0.0, 40.0 * k],
        ];
        let mut curves: Vec<SketchCurve> = (0..4)
            .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
            .collect();
        curves.push(line(9, s([40.0, 15.0]), s([20.0, 15.0])));
        curves.push(SketchCurve {
            id: 10,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: control.iter().map(|p| s(*p)).collect(),
            }),
        });
        let base = detect_profiles(&curves, &tol).unwrap().remove(0);
        assert_eq!(base.holes.len(), 1);
        let part = build(
            &spec(
                base,
                1.0 * k,
                vec![flange(
                    On::Base(1),
                    10.0 * k,
                    FRAC_PI_2,
                    Direction::Up,
                    k,
                    0.4,
                )],
                None,
            ),
            &tol,
        )
        .unwrap();
        // The lowest point of the Bézier, by a fine search of y(t).
        let y = |t: f64| {
            let u = 1.0 - t;
            k * (15.0 * u * u * u
                + 3.0 * 2.0 * u * u * t
                + 3.0 * 4.0 * u * t * t
                + 15.0 * t * t * t)
        };
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..200 {
            let (a, b) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
            if y(a) < y(b) {
                hi = b;
            } else {
                lo = a;
            }
        }
        let truth = y(0.5 * (lo + hi));
        let found = holes_to_bends(&part, &tol);
        assert_eq!(found.len(), 1);
        let h = found[0];
        assert!(!h.exact);
        assert!(
            (h.distance - truth).abs() <= tol.linear / 8.0 + 1e-12 * k,
            "{} against {truth}",
            h.distance
        );
        assert!(
            h.lower <= truth && truth - h.lower <= tol.linear / 4.0,
            "{} against {truth}",
            h.lower
        );
    }
}

/// A bend joint reached from its right body places the left one by the
/// inverse of the bend's motion (not the motion at −ρ, which is the
/// inverse only for a crease), and its velocity is the inverse's: two
/// copies of one bend between two bodies, one given and one solved, close
/// whichever body stays, the solved one at the given angle, and the two
/// bodies sit the same way relative to each other.
#[test]
fn a_bend_reached_from_its_right_body_folds_as_from_its_left() {
    use craft3d_kernel::origami::{Angle, Joint, Linkage, fold_linkage};
    let tol = tolerance(1.0);
    let bend = |id: u64| Joint {
        left: 0,
        right: 1,
        point: Vec3::new(0.0, 0.0, 0.6),
        direction: Vec3::X,
        side: Vec3::Y,
        reach: 10.0,
        id,
        radius: 2.4,
        across: -Vec3::Y,
    };
    let link = Linkage {
        bodies: 2,
        joints: vec![bend(1), bend(2)],
        size: 20.0,
    };
    let theta = 1.1;
    let relative = |fixed: usize| {
        let folded = fold_linkage(
            &link,
            &[Angle::Free { sign: 1.0 }, Angle::Given(theta)],
            fixed,
            tol.linear,
            tol.angular,
        )
        .unwrap();
        assert!(
            (folded.angles[0] - theta).abs() < 1e-9,
            "fixed {fixed}: {:?}",
            folded.angles
        );
        let back = folded.transforms[0].inverse().unwrap();
        let rel = folded.transforms[1].then(&back);
        [Vec3::ZERO, Vec3::X, Vec3::Y * 5.0 - Vec3::Y * 9.0, Vec3::Z].map(|p| rel.point(p))
    };
    let (from_left, from_right) = (relative(0), relative(1));
    for (a, b) in from_left.iter().zip(&from_right) {
        assert!(a.distance(*b) < 1e-9, "{a:?} against {b:?}");
    }
    // And the given bend alone, reached from the right, lands the left
    // body where the motion's inverse puts it: the right body's strip end
    // (y = −θ r) back on the axis's tangent point.
    let one = Linkage {
        bodies: 2,
        joints: vec![bend(1)],
        size: 20.0,
    };
    let a = fold_linkage(&one, &[Angle::Given(theta)], 0, tol.linear, tol.angular).unwrap();
    let b = fold_linkage(&one, &[Angle::Given(theta)], 1, tol.linear, tol.angular).unwrap();
    let ra = a.transforms[1].then(&a.transforms[0].inverse().unwrap());
    let rb = b.transforms[1].then(&b.transforms[0].inverse().unwrap());
    for p in [Vec3::ZERO, Vec3::new(3.0, -7.0, 0.6), Vec3::Z] {
        assert!(ra.point(p).distance(rb.point(p)) < 1e-9);
    }
}

/// A relief at a corner that is not square: the rect relief's notch is the
/// parallelogram of the two edges, area `r² sin α`; and a tear relief at a
/// corner near 180 degrees, whose circle (radius `r tan(α/2)`) would grow
/// past the base, is refused by name.
#[test]
fn reliefs_at_a_slanted_corner_and_near_a_straight_one() {
    for k in SCALES {
        let tol = tolerance(k);
        let pts = [[0.0, 0.0], [60.0, 0.0], [50.0, 30.0], [10.0, 30.0]];
        let curves: Vec<SketchCurve> = (0..4)
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % 4]);
                line(i as u64 + 1, [a[0] * k, a[1] * k], [b[0] * k, b[1] * k])
            })
            .collect();
        let trapezoid = detect_profiles(&curves, &tol).unwrap().remove(0);
        let (t, r, l, s) = (1.0 * k, 1.0 * k, 8.0 * k, 3.0 * k);
        let part = build(
            &spec(
                trapezoid,
                t,
                vec![
                    flange(On::Base(1), l, FRAC_PI_2, Direction::Up, r, 0.5),
                    flange(On::Base(2), l, FRAC_PI_2, Direction::Up, r, 0.5),
                ],
                Some(Relief {
                    kind: ReliefKind::Rect,
                    size: s,
                }),
            ),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
        let alpha = (10.0f64 / 1000f64.sqrt()).acos();
        let ba = FRAC_PI_2 * (r + 0.5 * t);
        let slant = 1000f64.sqrt() * k;
        let area = 1500.0 * k * k - s * s * alpha.sin()
            + (60.0 * k - s) * (ba + l)
            + (slant - s) * (ba + l);
        near(
            flat_area(&part),
            area,
            1e-9 * area,
            "the slanted corner's flat area",
        );
        near(
            volume(&part),
            area * t,
            1e-9 * area * t,
            "its volume at K = 1/2",
        );
        round_trip(&part, &tol);

        // A corner of about 179.4 degrees: the tear's circle would have a
        // radius of some 570 sizes.
        let pts = [
            [0.0, 0.0],
            [30.0, 0.0],
            [60.0, 0.3],
            [60.0, 30.0],
            [0.0, 30.0],
        ];
        let curves: Vec<SketchCurve> = (0..5)
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % 5]);
                line(i as u64 + 1, [a[0] * k, a[1] * k], [b[0] * k, b[1] * k])
            })
            .collect();
        let flat = detect_profiles(&curves, &tol).unwrap().remove(0);
        let err = build(
            &spec(
                flat,
                t,
                vec![
                    flange(On::Base(1), l, FRAC_PI_2, Direction::Up, r, 0.5),
                    flange(On::Base(2), l, FRAC_PI_2, Direction::Up, r, 0.5),
                ],
                Some(Relief {
                    kind: ReliefKind::Tear,
                    size: s,
                }),
            ),
            &tol,
        )
        .err()
        .unwrap();
        assert!(
            err.to_string().contains("cut across the base's outline"),
            "{err}"
        );
    }
}
