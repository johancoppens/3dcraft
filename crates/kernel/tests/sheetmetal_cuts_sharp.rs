//! Holes across a bend in a part with a sharp bend (docs/design/sheetmetal.md
//! › *Sharp bends*, *Holes across a bend*), held to closed forms at 1e-3, 1
//! and 1e6:
//!
//! - a slot straight across a sharp bend (its sides square to the bend
//!   line, from one tangent line to the other): the bend is left in
//!   stretches, each joined in as a sharp bend of its own, the slot's sides
//!   there the sectors between the axis and the outer skin; the volume is
//!   the flats' slabs and per stretch the sector `θ w T²/2`, so the slot
//!   takes its area on the flats times `T` and on the strip times `T/(2K)`
//!   (`T (R + T/2)/(R + K T)` at `R = 0`); the flat area less the slot's;
//! - a hole across a rounded bend in a part that has a sharp bend
//!   elsewhere: the rounded closed forms;
//! - fold then unfold: every flat's edge laid back lies on the pattern;
//! - refusals by reason: a hole whose edge runs over a sharp bend other
//!   than straight across it (a round hole on its tangent line, a slot
//!   across it at a slant, a hole inside its strip).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::plate::Made;
use craft3d_kernel::profile::{Curve2, Profile, Segment, Shape, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::sharp::SharpFace;
use craft3d_kernel::sheetmetal::{
    Closing, Cutout, Direction, EdgeKey, FlangeSpec, LengthFrom, On, Part, SheetFace, Spec, build,
    flat_area, measured_radius,
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

/// The rectangle `0..40` × `0..30` (curves 1 to 4 from the bottom edge
/// counter-clockwise).
fn base(k: f64) -> Profile {
    let c = [
        [0.0, 0.0],
        [40.0 * k, 0.0],
        [40.0 * k, 30.0 * k],
        [0.0, 30.0 * k],
    ];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
        .collect();
    detect_profiles(&curves, &tolerance(k)).unwrap().remove(0)
}

fn cutout(curves: Vec<SketchCurve>, k: f64) -> Cutout {
    let mut found = detect_profiles(&curves, &tolerance(k)).unwrap();
    assert_eq!(found.len(), 1);
    Cutout {
        profile: found.remove(0),
        from: "the test".to_string(),
    }
}

/// A polygon cut-out, its points in model units over `k`.
fn polygon(id: u64, points: &[[f64; 2]], k: f64) -> Cutout {
    let n = points.len();
    cutout(
        (0..n)
            .map(|i| {
                let (a, b) = (points[i], points[(i + 1) % n]);
                line(id + i as u64, [a[0] * k, a[1] * k], [b[0] * k, b[1] * k])
            })
            .collect(),
        k,
    )
}

fn circle(id: u64, center: [f64; 2], radius: f64, k: f64) -> Cutout {
    cutout(
        vec![SketchCurve {
            id,
            curve: Curve2::Circle {
                center: [center[0] * k, center[1] * k],
                radius: radius * k,
            },
        }],
        k,
    )
}

fn flange(edge: u64, angle: f64, direction: Direction, radius: f64, kf: f64, k: f64) -> FlangeSpec {
    FlangeSpec {
        on: On::Base(edge),
        length: 10.0 * k,
        angle,
        direction,
        radius,
        k: kf,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    }
}

fn spec(k: f64, flanges: Vec<FlangeSpec>, cutouts: Vec<Cutout>) -> Spec {
    Spec {
        base: base(k),
        plane: Frame::WORLD,
        thickness: k,
        flanges,
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts,
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

/// The distance from `p` to a pattern line or circle.
fn to_segment(p: [f64; 2], s: &Segment) -> f64 {
    match &s.shape {
        Shape::Line => {
            let d = [s.end[0] - s.start[0], s.end[1] - s.start[1]];
            let l2 = d[0] * d[0] + d[1] * d[1];
            let t =
                (((p[0] - s.start[0]) * d[0] + (p[1] - s.start[1]) * d[1]) / l2).clamp(0.0, 1.0);
            (p[0] - s.start[0] - t * d[0]).phypot(p[1] - s.start[1] - t * d[1])
        }
        Shape::Circle { center, radius } | Shape::Arc { center, radius } => {
            ((p[0] - center[0]).phypot(p[1] - center[1]) - radius).abs()
        }
        Shape::Spline(_) => f64::INFINITY,
    }
}

/// The solid is valid; fold then unfold: every edge of a flat's face of
/// the folded sheet, laid back by the flat's motion, lies on the pattern
/// (the flats' outlines and holes, the cuts); every sharp bend's joined
/// faces are its outer skin's stretches (cylinders of radius `T`) and
/// planar sectors, and its measured radius is 0.
fn healthy(part: &Part, tol: &Tolerance) {
    let defects = check(&part.solid.body, tol);
    assert!(
        defects.is_empty(),
        "the solid fails the checker: {defects:?}"
    );
    let mut curves: Vec<Segment> = Vec::new();
    for flat in &part.flats {
        curves.extend(flat.outer.iter().cloned());
        curves.extend(flat.holes.iter().flatten().cloned());
    }
    for cut in &part.cuts {
        curves.extend(cut.segments.iter().cloned());
    }
    for (f, role) in part.sheet_faces.iter().enumerate() {
        let (SheetFace::Flat(n) | SheetFace::FlatPiece(n, _)) = role else {
            continue;
        };
        let back = part.flats[*n].transform.inverse().unwrap();
        let face = part.sheet.face(craft3d_topo::FaceId(f as u32)).unwrap();
        for l in &face.loops {
            for c in &part.sheet.loop_(*l).unwrap().coedges {
                let edge = part.sheet.edge(c.edge).unwrap();
                for i in 0..=8 {
                    let s = edge.range.0 + (edge.range.1 - edge.range.0) * f64::from(i) / 8.0;
                    let (x, y, z) = part.plane.local(edge.curve.eval(s));
                    let p = back.point(Vec3::new(x, y, z));
                    assert!(p.z.abs() <= tol.linear, "off its flat by {}", p.z);
                    let d = curves
                        .iter()
                        .map(|seg| to_segment([p.x, p.y], seg))
                        .fold(f64::INFINITY, f64::min);
                    assert!(d <= 2.0 * tol.linear, "a flat's edge lands {d:e} off");
                }
            }
        }
    }
    let t = part.thickness;
    for (i, b) in part.bends.iter().enumerate() {
        if !b.sharp() {
            continue;
        }
        assert_eq!(measured_radius(part, i), Some(0.0));
        for (f, m) in part.solid.faces.iter().enumerate() {
            let Made::Joined(c) = m else { continue };
            let (bend, what) = part.joined_face(*c).unwrap();
            if bend != i {
                continue;
            }
            let surface = &part.solid.body.faces[f].surface;
            match what {
                SharpFace::Skin | SharpFace::SkinPiece(_) => {
                    let Surface::Cylinder(cy) = surface else {
                        panic!("a sharp bend's skin is a cylinder")
                    };
                    near(cy.radius(), t, 1e-12 * t, "the outer skin's radius");
                }
                SharpFace::End(_) | SharpFace::Side(..) => {
                    assert!(matches!(surface, Surface::Plane(_)))
                }
            }
        }
    }
}

/// The faces joined in at bend `i`.
fn joined(part: &Part, i: usize) -> Vec<SharpFace> {
    part.solid
        .faces
        .iter()
        .filter_map(|m| match m {
            Made::Joined(c) => part
                .joined_face(*c)
                .filter(|(b, _)| *b == i)
                .map(|(_, f)| f),
            _ => None,
        })
        .collect()
}

/// Slots straight across a sharp wall's bend on the base's bottom edge
/// (the wall 10 long, the base 40 × 30, `T` 1), 3 into the base and 4 into
/// the wall: one 6 wide at `x` 15 to 21, and with it a second 2 wide at
/// `x` 30 to 32; at 60, 90 and 120 degrees, up and down, K 0.33 and ½.
#[test]
fn slots_straight_across_a_sharp_bend() {
    for k in SCALES {
        let tol = tolerance(k);
        let t = k;
        for (angle, dir) in [
            (FRAC_PI_2, Direction::Up),
            (FRAC_PI_2, Direction::Down),
            (PI / 3.0, Direction::Up),
            (2.0 * PI / 3.0, Direction::Down),
        ] {
            for kf in [0.33, 0.5] {
                let what = format!("scale {k}, {} degrees {dir:?}, K {kf}", angle.to_degrees());
                let ba = angle * kf * t;
                let slot = |id: u64, x0: f64, x1: f64| {
                    let y1 = -(ba / k) - 4.0;
                    polygon(id, &[[x0, 3.0], [x0, y1], [x1, y1], [x1, 3.0]], k)
                };
                let wall = flange(1, angle, dir, 0.0, kf, k);
                let (w, d, l) = (40.0 * k, 30.0 * k, 10.0 * k);
                let whole = w * d * t + w * l * t + angle * w * t * t / 2.0;
                let area0 = w * (d + ba + l);
                for slots in [vec![(15.0, 21.0)], vec![(15.0, 21.0), (30.0, 32.0)]] {
                    let cutouts: Vec<Cutout> = slots
                        .iter()
                        .enumerate()
                        .map(|(n, &(a, b))| slot(20 + 10 * n as u64, a, b))
                        .collect();
                    let sp = spec(k, vec![wall.clone()], cutouts);
                    let part = build(&sp, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
                    assert_eq!(part, build(&sp, &tol).unwrap(), "{what}: deterministic");
                    healthy(&part, &tol);
                    assert_eq!(part.cuts.len(), slots.len());
                    let widths: f64 = slots.iter().map(|(a, b)| (b - a) * k).sum();
                    let v = whole - widths * 7.0 * k * t - widths * ba * t / (2.0 * kf);
                    near(volume(&part), v, 1e-9 * whole, &format!("{what}: volume"));
                    let a = area0 - widths * (7.0 * k + ba);
                    near(flat_area(&part), a, 1e-9 * area0, &format!("{what}: area"));
                    // The bend in stretches: per stretch its skin, the
                    // bend's two ends and the slots' two sides each.
                    let n = slots.len();
                    let faces = joined(&part, 0);
                    assert_eq!(faces.len(), 3 * (n + 1), "{what}: {faces:?}");
                    assert!(faces.contains(&SharpFace::Skin));
                    assert!(faces.contains(&SharpFace::End(0)));
                    assert!(faces.contains(&SharpFace::End(1)));
                    for j in 1..=n {
                        assert!(faces.contains(&SharpFace::SkinPiece(j)), "{what}");
                    }
                    let sides = faces
                        .iter()
                        .filter(|f| matches!(f, SharpFace::Side(_, EdgeKey::Cut { .. })))
                        .count();
                    assert_eq!(sides, 2 * n, "{what}");
                }
            }
        }
    }
}

/// A sharp wall on the base's bottom edge and a rounded one (`R` 2) on
/// its top edge, a round hole of radius 2 across the middle of the
/// rounded bend: the rounded bend's closed forms, the sharp bend as built
/// without the hole.
#[test]
fn a_hole_across_a_rounded_bend_beside_a_sharp_one() {
    for k in SCALES {
        let tol = tolerance(k);
        let t = k;
        for kf in [0.33, 0.5] {
            let what = format!("scale {k}, K {kf}");
            let r = 2.0 * k;
            let rho = r + kf * t;
            let ba = FRAC_PI_2 * rho;
            let centre = [20.0, 30.0 + ba / (2.0 * k)];
            let flanges = vec![
                flange(1, FRAC_PI_2, Direction::Up, 0.0, kf, k),
                flange(3, FRAC_PI_2, Direction::Up, r, kf, k),
            ];
            let sp = spec(k, flanges.clone(), vec![circle(20, centre, 2.0, k)]);
            let part = build(&sp, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            healthy(&part, &tol);
            assert_eq!(part.cuts.len(), 1, "{what}");
            let (w, d, l) = (40.0 * k, 30.0 * k, 10.0 * k);
            let whole = w * d * t
                + 2.0 * w * l * t
                + FRAC_PI_2 * w * t * t / 2.0
                + FRAC_PI_2 * w * t * (r + 0.5 * t);
            let rh = 2.0 * k;
            let half = ba / 2.0;
            let beyond = rh * rh * (half / rh).pacos() - half * (rh * rh - half * half).sqrt();
            let strip = PI * rh * rh - 2.0 * beyond;
            let v = whole - 2.0 * beyond * t - strip * t * (r + 0.5 * t) / rho;
            near(volume(&part), v, 1e-9 * whole, &format!("{what}: volume"));
            let plain = build(&spec(k, flanges, Vec::new()), &tol).unwrap();
            assert_eq!(joined(&part, 0), joined(&plain, 0), "{what}");
            let a = flat_area(&plain) - PI * rh * rh;
            near(flat_area(&part), a, 1e-9 * a, &format!("{what}: area"));
        }
    }
}

/// What a hole across a sharp bend does not build, by reason: a round
/// hole whose centre is on the tangent line, a slot across the bend at a
/// slant, a round hole inside the strip; a stretch left between two
/// slots narrower than the sliver floor.
#[test]
fn holes_over_a_sharp_bend_other_than_straight_across_are_refused() {
    for k in SCALES {
        let tol = tolerance(k);
        let kf = 0.4;
        let ba = FRAC_PI_2 * kf;
        let wall = flange(1, FRAC_PI_2, Direction::Up, 0.0, kf, k);
        let y1 = -ba - 4.0;
        for cut in [
            circle(20, [20.0, 0.0], 2.0, k),
            polygon(20, &[[15.0, 3.0], [16.0, y1], [22.0, y1], [21.0, 3.0]], k),
            circle(20, [20.0, -ba / 2.0], ba / 4.0, k),
        ] {
            let (f, why) = refused(&spec(k, vec![wall.clone()], vec![cut]), &tol);
            assert_eq!(f, None, "scale {k}: {why}");
            assert!(why.contains("hole across a bend"), "{why}");
            assert!(why.contains("sharp"), "{why}");
            assert!(why.contains("straight across"), "{why}");
        }
        // Two slots straight across, the stretch of the bend between them
        // 2 tolerances wide: a sliver, refused (review of c85bf890: built).
        let gap = 2e-6;
        let slots = vec![
            polygon(20, &[[15.0, 3.0], [15.0, y1], [21.0, y1], [21.0, 3.0]], k),
            polygon(
                30,
                &[[21.0 + gap, 3.0], [21.0 + gap, y1], [25.0, y1], [25.0, 3.0]],
                k,
            ),
        ];
        let (f, why) = refused(&spec(k, vec![wall.clone()], slots), &tol);
        assert_eq!(f, Some(0), "scale {k}: {why}");
        assert!(why.contains("sliver"), "scale {k}: {why}");
    }
}
