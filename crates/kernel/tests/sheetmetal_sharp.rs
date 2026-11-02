//! Sharp bends (docs/design/sheetmetal.md › *Sharp bends*): a bend of
//! inner radius 0, its inner skin the line on its axis and its outer a
//! cylinder of radius `T`, held to the closed forms of every bend at
//! `R = 0`, at 1e-3, 1 and 1e6:
//!
//! - the bend allowance `θ K T`, the flat length `D + θ K T + L`, the
//!   deduction `2 T tan(θ/2) − θ K T`;
//! - the volume: the flats' slabs and per bend the sector `θ w T²/2`, so
//!   `A T` of the flat area at `K = ½` and less by `θ w T² (½ − K)`
//!   otherwise;
//! - the box: the outside `T` from the edge, the top `T + L` (bend up);
//! - the measured inner radius 0, the outer skin's cylinder of radius `T`;
//! - a U, a Z (a sharp bend down on a sharp bend up's tip), a box with
//!   rect reliefs, a sharp bend beside a rounded one, a corner closed by
//!   overlap; refusals by reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::plate::Made;
use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::sharp::{SharpFace, joined};
use craft3d_kernel::sheetmetal::{
    Closing, CornerKind, Cutout, Direction, FlangeSpec, LengthFrom, On, Part, Relief, ReliefKind,
    Spec, build, developed_allowance, flat_area, measured_angle, measured_radius,
};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{check, mass_properties};
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
/// counter-clockwise).
fn base(w: f64, d: f64, k: f64) -> Profile {
    let c = [[0.0, 0.0], [w * k, 0.0], [w * k, d * k], [0.0, d * k]];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
        .collect();
    let mut profiles = detect_profiles(&curves, &tolerance(k)).unwrap();
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
        inset: [0.0; 2],
        outline: None,
    }
}

fn spec(base: Profile, t: f64, flanges: Vec<FlangeSpec>, relief: Option<Relief>) -> Spec {
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: t,
        flanges,
        relief,
        bend_relief: None,
        corner: Closing::OPEN,
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

fn bbox(part: &Part) -> (Vec3, Vec3) {
    part.solid.body.bbox().unwrap().corners().unwrap()
}

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

#[track_caller]
fn healthy(part: &Part, tol: &Tolerance) {
    let defects = check(&part.solid.body, tol);
    assert!(
        defects.is_empty(),
        "the solid fails the checker: {defects:?}"
    );
}

/// Every sharp bend has its outer skin, a cylinder of radius `T`, and its
/// two end sectors, and no other face of the part is a cylinder of it.
#[track_caller]
fn sharp_faces(part: &Part, t: f64, tol: f64) {
    for (i, b) in part.bends.iter().enumerate() {
        if !b.sharp() {
            continue;
        }
        let faces: Vec<(usize, SharpFace)> = part
            .solid
            .faces
            .iter()
            .enumerate()
            .filter_map(|(f, m)| match m {
                Made::Joined(c) if joined(*c).0 == i => Some((f, joined(*c).1)),
                _ => None,
            })
            .collect();
        assert_eq!(faces.len(), 3, "a sharp bend's three faces");
        for (f, what) in faces {
            let surface = &part.solid.body.faces[f].surface;
            match what {
                SharpFace::Skin => {
                    let Surface::Cylinder(c) = surface else {
                        panic!("a sharp bend's skin is a cylinder")
                    };
                    near(c.radius(), t, tol, "the outer skin's radius");
                }
                SharpFace::End(_) => assert!(matches!(surface, Surface::Plane(_))),
            }
        }
        assert_eq!(measured_radius(part, i), Some(0.0));
    }
}

/// An L at 90, 60 and 120 degrees, up and down, K 0.33 and ½.
#[test]
fn a_sharp_l_by_its_closed_forms() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, d, t, l) = (40.0 * k, 30.0 * k, 2.0 * k, 25.0 * k);
        for theta in [FRAC_PI_2, PI / 3.0, 2.0 * PI / 3.0] {
            for direction in [Direction::Up, Direction::Down] {
                for kf in [0.33, 0.5] {
                    let part = build(
                        &spec(
                            base(40.0, 30.0, k),
                            t,
                            vec![flange(On::Base(1), l, theta, direction, 0.0, kf)],
                            None,
                        ),
                        &tol,
                    )
                    .unwrap_or_else(|e| panic!("{theta} {direction:?} {kf} at {k}: {e}"));
                    healthy(&part, &tol);
                    sharp_faces(&part, t, 1e-12 * k);
                    let b = part.bends[0];
                    assert!(b.sharp());
                    let ba = theta * kf * t;
                    near(b.allowance, ba, 1e-12 * k, "BA");
                    near(flat_extent(&part, 1), d + ba + l, 1e-9 * k, "flat length");
                    near(
                        flat_area(&part),
                        w * (d + ba + l),
                        1e-9 * k * k * 1e3,
                        "flat area",
                    );
                    let deduction = 2.0 * t * (0.5 * theta).ptan() - ba;
                    near(
                        b.deduction(tol.angular).unwrap(),
                        deduction,
                        1e-12 * k,
                        "the deduction",
                    );
                    // Two slabs and the sector of radius T.
                    let v = w * t * (d + l) + theta * w * t * t / 2.0;
                    near(volume(&part), v, 1e-9 * v, "volume");
                    near(
                        volume(&part) - flat_area(&part) * t,
                        theta * w * t * t * (0.5 - kf),
                        1e-9 * v,
                        "volume − A T",
                    );
                    near(measured_angle(&part, 0).unwrap(), theta, 1e-12, "the angle");
                    near(
                        developed_allowance(&b, &tol).unwrap(),
                        ba,
                        4.0 * tol.linear,
                        "the developed allowance",
                    );
                    // The box. Up: the axis at (y, z) = (0, T), the wall's
                    // inner skin through it at θ, its outer skin T further
                    // out, the sector between; down is the mirror in
                    // z = T/2.
                    let (lo, hi) = bbox(&part);
                    let (s, c) = theta.psin_cos();
                    let slack = 10.0 * tol.linear;
                    let top = t + l * s + (-c).max(0.0) * t;
                    let far = (-l * c - t * s)
                        .min(if theta >= FRAC_PI_2 { -t } else { -t * s })
                        .min(-l * c)
                        .min(0.0);
                    near(hi.y, d, slack, "the base's far edge");
                    near(lo.y, far, slack, "the wall's outside");
                    match direction {
                        Direction::Up => {
                            near(hi.z, top, slack, "the top");
                            near(lo.z, 0.0, slack, "the bottom");
                        }
                        Direction::Down => {
                            near(hi.z, t, slack, "the top");
                            near(lo.z, t - top, slack, "the bottom");
                        }
                    }
                }
            }
        }
    }
}

/// A U (sharp bends up on both long edges) and a Z (a sharp bend down on
/// a sharp bend up's tip): flat lengths `D + 2 (BA + L)` and
/// `D + BA + L₀ + BA + L₁`, volumes by slabs and sectors.
#[test]
fn a_sharp_u_and_z_by_their_closed_forms() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, d, t, l0, l1, kf) = (40.0 * k, 30.0 * k, 1.5 * k, 20.0 * k, 12.0 * k, 0.4);
        let ba = FRAC_PI_2 * kf * t;
        let sector = FRAC_PI_2 * w * t * t / 2.0;
        let u = build(
            &spec(
                base(40.0, 30.0, k),
                t,
                vec![
                    flange(On::Base(1), l0, FRAC_PI_2, Direction::Up, 0.0, kf),
                    flange(On::Base(3), l0, FRAC_PI_2, Direction::Up, 0.0, kf),
                ],
                None,
            ),
            &tol,
        )
        .unwrap();
        healthy(&u, &tol);
        sharp_faces(&u, t, 1e-12 * k);
        near(
            flat_extent(&u, 1),
            d + 2.0 * (ba + l0),
            1e-9 * k,
            "the U's flat",
        );
        let v = w * t * (d + 2.0 * l0) + 2.0 * sector;
        near(volume(&u), v, 1e-9 * v, "the U's volume");
        let (lo, hi) = bbox(&u);
        near(lo.y, -t, 10.0 * tol.linear, "the U's outside");
        near(hi.y, d + t, 10.0 * tol.linear, "the U's other outside");
        near(hi.z, t + l0, 10.0 * tol.linear, "the U's top");

        let z = build(
            &spec(
                base(40.0, 30.0, k),
                t,
                vec![
                    flange(On::Base(1), l0, FRAC_PI_2, Direction::Up, 0.0, kf),
                    flange(On::Tip(0), l1, FRAC_PI_2, Direction::Down, 0.0, kf),
                ],
                None,
            ),
            &tol,
        )
        .unwrap();
        healthy(&z, &tol);
        sharp_faces(&z, t, 1e-12 * k);
        near(
            flat_extent(&z, 1),
            d + ba + l0 + ba + l1,
            1e-9 * k,
            "the Z's flat",
        );
        let v = w * t * (d + l0 + l1) + 2.0 * sector;
        near(volume(&z), v, 1e-9 * v, "the Z's volume");
        // The wall's outside at y = −T, its top T + L₀; the second flange
        // turns down at the wall's top, outward: its axis on the wall's
        // outer skin at z = T + L₀, its far end L₁ + T past the wall's
        // inner skin.
        let (lo, hi) = bbox(&z);
        near(hi.z, t + l0 + t, 10.0 * tol.linear, "the Z's top");
        near(lo.y, -t - l1, 10.0 * tol.linear, "the Z's reach");
        for b in 0..2 {
            assert_eq!(measured_radius(&z, b), Some(0.0));
        }
    }
}

/// A box of four sharp flanges with rect reliefs: the flat area by the
/// notches and the shortened flanges, the volume `A T` at K = ½.
#[test]
fn a_sharp_box_with_reliefs() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, d, t, l, s, kf) = (60.0 * k, 40.0 * k, 1.0 * k, 10.0 * k, 3.0 * k, 0.5);
        let ba = FRAC_PI_2 * kf * t;
        for (kind, taken) in [
            (ReliefKind::Rect, s * s),
            (ReliefKind::Round, PI * s * s / 4.0),
        ] {
            let flanges: Vec<FlangeSpec> = (1..=4)
                .map(|c| flange(On::Base(c), l, FRAC_PI_2, Direction::Up, 0.0, kf))
                .collect();
            let part = build(
                &spec(
                    base(60.0, 40.0, k),
                    t,
                    flanges,
                    Some(Relief { kind, size: s }),
                ),
                &tol,
            )
            .unwrap_or_else(|e| panic!("{kind:?} at {k}: {e}"));
            healthy(&part, &tol);
            sharp_faces(&part, t, 1e-12 * k);
            let short = 2.0 * s;
            let area =
                w * d - 4.0 * taken + 2.0 * (w - short) * (ba + l) + 2.0 * (d - short) * (ba + l);
            near(flat_area(&part), area, 1e-9 * area, "the box's flat area");
            near(volume(&part), area * t, 1e-9 * area * t, "the box's volume");
            let (_, hi) = bbox(&part);
            near(hi.z, t + l, 10.0 * tol.linear, "the box's height");
        }
    }
}

/// A sharp flange beside a rounded one at a corner without a relief: the
/// sharp wall and the rounded bend meet only at the base's corner, and
/// the volume is the sum of the pieces.
#[test]
fn a_sharp_bend_beside_a_rounded_one() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, d, t, l, r, kf) = (40.0 * k, 30.0 * k, 1.0 * k, 10.0 * k, 2.0 * k, 0.4);
        let part = build(
            &spec(
                base(40.0, 30.0, k),
                t,
                vec![
                    flange(On::Base(1), l, FRAC_PI_2, Direction::Up, 0.0, kf),
                    flange(On::Base(2), l, FRAC_PI_2, Direction::Up, r, kf),
                ],
                None,
            ),
            &tol,
        )
        .unwrap();
        healthy(&part, &tol);
        sharp_faces(&part, t, 1e-12 * k);
        let v = w * d * t
            + w * l * t
            + FRAC_PI_2 * w * t * t / 2.0
            + d * l * t
            + FRAC_PI_2 * d * t * (r + t / 2.0);
        near(volume(&part), v, 1e-9 * v, "volume");
        near(
            measured_radius(&part, 1).unwrap(),
            r,
            1e-12 * k,
            "the rounded radius",
        );
        assert_eq!(measured_radius(&part, 0), Some(0.0));
    }
}

/// Sharp walls closed by overlap at a corner with a round relief (the
/// covered flange's end clear of its bend, so no bend is trimmed): the
/// volume `A T` at K = ½; sharp flanges with an inset tab and rect bend
/// reliefs, and a hole in a sharp flange's flat, the same.
#[test]
fn sharp_bends_closed_inset_and_with_holes() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, l, s, g, kf) = (1.0 * k, 10.0 * k, 3.0 * k, 0.5 * k, 0.5);
        let mut closed = spec(
            base(40.0, 30.0, k),
            t,
            vec![
                flange(On::Base(1), l, FRAC_PI_2, Direction::Up, 0.0, kf),
                flange(On::Base(2), l, FRAC_PI_2, Direction::Up, 0.0, kf),
            ],
            Some(Relief {
                kind: ReliefKind::Round,
                size: s,
            }),
        );
        closed.corner = Closing {
            kind: CornerKind::Overlap,
            gap: g,
            over: None,
        };
        let part = build(&closed, &tol).unwrap_or_else(|e| panic!("closed at {k}: {e}"));
        healthy(&part, &tol);
        sharp_faces(&part, t, 1e-12 * k);
        let a = flat_area(&part);
        near(volume(&part), a * t, 1e-9 * a * t, "closed: A T");

        let mut tab = flange(On::Base(1), l, FRAC_PI_2, Direction::Up, 0.0, kf);
        tab.inset = [10.0 * k, 10.0 * k];
        let mut inset = spec(base(40.0, 30.0, k), t, vec![tab], None);
        inset.bend_relief = Some(Relief {
            kind: ReliefKind::Rect,
            size: 2.0 * k,
        });
        let part = build(&inset, &tol).unwrap_or_else(|e| panic!("inset at {k}: {e}"));
        healthy(&part, &tol);
        sharp_faces(&part, t, 1e-12 * k);
        let ba = FRAC_PI_2 * kf * t;
        let area = 40.0 * 30.0 * k * k - 2.0 * 4.0 * k * k + 20.0 * k * (ba + l);
        near(flat_area(&part), area, 1e-9 * area, "inset: flat area");
        near(volume(&part), area * t, 1e-9 * area * t, "inset: A T");

        // A hole of radius 2 in the wall's flat, 5 past its tangent line.
        let mut holed = spec(
            base(40.0, 30.0, k),
            t,
            vec![flange(On::Base(1), l, FRAC_PI_2, Direction::Up, 0.0, kf)],
            None,
        );
        holed.cutouts = circles(&[([20.0, -(ba / k) - 5.0], 2.0)], k);
        let part = build(&holed, &tol).unwrap_or_else(|e| panic!("holed at {k}: {e}"));
        healthy(&part, &tol);
        let area = 40.0 * k * (30.0 * k + ba + l) - PI * 4.0 * k * k;
        near(flat_area(&part), area, 1e-9 * area, "holed: flat area");
        near(volume(&part), area * t, 1e-9 * area * t, "holed: A T");
    }
}

fn circles(list: &[([f64; 2], f64)], k: f64) -> Vec<Cutout> {
    let curves: Vec<SketchCurve> = list
        .iter()
        .enumerate()
        .map(|(i, (c, r))| SketchCurve {
            id: 20 + i as u64,
            curve: Curve2::Circle {
                center: [c[0] * k, c[1] * k],
                radius: r * k,
            },
        })
        .collect();
    detect_profiles(&curves, &tolerance(k))
        .unwrap()
        .into_iter()
        .map(|profile| Cutout {
            profile,
            from: "the test".to_string(),
        })
        .collect()
}

fn refused(spec: &Spec, tol: &Tolerance) -> (Option<usize>, String) {
    match build(spec, tol) {
        Ok(_) => panic!("the part was built"),
        Err(KernelError::SheetMetal { flange, reason }) => (flange, reason),
        Err(other) => panic!("refused for another reason: {other}"),
    }
}

/// What a sharp bend refuses, by reason: 180 degrees (a closed hem), K = 0
/// (a strip of no width), two sharp walls at an open corner without a
/// relief (they would touch along the corner's line), a hole across a
/// sharp bend; and a radius within the tolerance of 0 is 0.
#[test]
fn sharp_refusals_by_reason() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, l) = (1.0 * k, 10.0 * k);
        let one = |angle: f64, kf: f64, radius: f64| {
            spec(
                base(40.0, 30.0, k),
                t,
                vec![flange(On::Base(1), l, angle, Direction::Up, radius, kf)],
                None,
            )
        };
        let (f, why) = refused(&one(PI, 0.4, 0.0), &tol);
        assert_eq!(f, Some(0));
        assert!(why.contains("closed hem"), "{why}");
        let (_, why) = refused(&one(FRAC_PI_2, 0.0, 0.0), &tol);
        assert!(why.contains("K-factor above 0"), "{why}");
        // Within the tolerance of 0: sharp.
        let part = build(&one(FRAC_PI_2, 0.4, 0.5 * tol.linear), &tol).unwrap();
        assert!(part.bends[0].sharp());
        near(
            part.bends[0].allowance,
            FRAC_PI_2 * 0.4 * t,
            1e-12 * k,
            "BA",
        );

        let corner = spec(
            base(40.0, 30.0, k),
            t,
            vec![
                flange(On::Base(1), l, FRAC_PI_2, Direction::Up, 0.0, 0.4),
                flange(On::Base(2), l, FRAC_PI_2, Direction::Up, 0.0, 0.4),
            ],
            None,
        );
        let (f, why) = refused(&corner, &tol);
        assert_eq!(f, Some(1));
        assert!(why.contains("touch along the corner"), "{why}");
        // Bent opposite ways the walls part: built, the sum of the pieces.
        let mut apart = corner.clone();
        apart.flanges[1].direction = Direction::Down;
        let part = build(&apart, &tol).unwrap();
        healthy(&part, &tol);
        let (w, d) = (40.0 * k, 30.0 * k);
        let v = w * d * t + (w + d) * (l * t + FRAC_PI_2 * t * t / 2.0);
        near(volume(&part), v, 1e-9 * v, "walls bent opposite ways");

        let mut across = one(FRAC_PI_2, 0.4, 0.0);
        across.cutouts = circles(&[([20.0, 0.0], 2.0)], k);
        let (f, why) = refused(&across, &tol);
        assert_eq!(f, Some(0));
        assert!(why.contains("hole across a bend"), "{why}");
    }
}
