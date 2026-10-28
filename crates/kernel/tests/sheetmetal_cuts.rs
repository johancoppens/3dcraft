//! Holes across a bend (docs/design/sheetmetal.md › *Holes across a
//! bend*): a cut-out of the flat pattern that crosses a bend's tangent
//! lines, cut through the bend where the bend allowance puts it, held to
//! closed forms at 1e-3, 1 and 1e6:
//!
//! - the volume: the part without the hole less the hole's area on the
//!   flats times `T` and its area on the strip times `T (R + T/2) / (R + K T)`
//!   (a strip's element `du ds` is the bend's `du dφ` with `φ = s / (R + K T)`,
//!   whose material is `T (R + T/2) dφ du` between the skins `R` and `R + T`);
//! - the flat area: the pattern's less the hole's;
//! - the faces: a slot across the whole strip leaves the bend in two;
//! - fold then unfold: every edge of the folded bottom skin, laid back by
//!   the inverse motion (a flat) or the inverse wrap (a bend: the angle
//!   about the axis times `R + K T` across the strip), lies on the
//!   pattern's curves;
//! - the flat pattern does not overlap itself: its outline is one region
//!   with every hole and the flat area;
//! - refusals by reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Curve3;
use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{
    Curve2, Profile, Segment, Shape, SketchCurve, Spline2, detect_profiles,
};
use craft3d_kernel::sheetmetal::{
    Closing, Cutout, Direction, FlangeSpec, LengthFrom, On, Part, SheetFace, Spec, build,
    flat_area, outline,
};
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

/// The 40 × 30 base (curves 1 to 4, the right edge is curve 2).
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

/// The closed profile of these curves, as a cut-out.
fn cutout(curves: Vec<SketchCurve>, k: f64) -> Cutout {
    let mut found = detect_profiles(&curves, &tolerance(k)).unwrap();
    assert_eq!(found.len(), 1);
    Cutout {
        profile: found.remove(0),
        from: "the test".to_string(),
    }
}

/// A polygon cut-out (curves 20, 21, ...), its points in model units.
fn polygon(points: &[[f64; 2]], k: f64) -> Cutout {
    let n = points.len();
    cutout(
        (0..n)
            .map(|i| {
                let (a, b) = (points[i], points[(i + 1) % n]);
                line(20 + i as u64, [a[0] * k, a[1] * k], [b[0] * k, b[1] * k])
            })
            .collect(),
        k,
    )
}

fn circle(center: [f64; 2], radius: f64, k: f64) -> Cutout {
    cutout(
        vec![SketchCurve {
            id: 20,
            curve: Curve2::Circle {
                center: [center[0] * k, center[1] * k],
                radius: radius * k,
            },
        }],
        k,
    )
}

/// The part: a flange 15 long on the base's right edge (x = 40), bent up
/// 90 degrees with the inner radius 2 and the thickness 1 (times `k`), the
/// K-factor `kf`, and these cut-outs.
fn part_spec(k: f64, kf: f64, cutouts: Vec<Cutout>) -> Spec {
    Spec {
        base: base(k),
        plane: Frame::WORLD,
        thickness: k,
        flanges: vec![FlangeSpec {
            on: On::Base(2),
            length: 15.0 * k,
            angle: FRAC_PI_2,
            direction: Direction::Up,
            radius: 2.0 * k,
            k: kf,
            from: LengthFrom::Tangent,
        }],
        relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts,
    }
}

/// The bend allowance of `part_spec`, in model units over `k`.
fn ba(kf: f64) -> f64 {
    FRAC_PI_2 * (2.0 + kf)
}

/// The volume of `part_spec` without holes.
fn whole_volume(k: f64) -> f64 {
    let (t, r) = (k, 2.0 * k);
    40.0 * 30.0 * k * k * t + 15.0 * 30.0 * k * k * t + FRAC_PI_2 * 30.0 * k * t * (r + 0.5 * t)
}

/// What a hole's area on the strip takes out of the bend, per unit area.
fn strip_factor(k: f64, kf: f64) -> f64 {
    let (t, r) = (k, 2.0 * k);
    t * (r + 0.5 * t) / (r + kf * t)
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

/// The distance from `p` to a pattern segment (lines, arcs, circles).
fn to_segment(p: [f64; 2], s: &Segment) -> f64 {
    match &s.shape {
        Shape::Line => {
            let d = [s.end[0] - s.start[0], s.end[1] - s.start[1]];
            let l2 = d[0] * d[0] + d[1] * d[1];
            let t =
                (((p[0] - s.start[0]) * d[0] + (p[1] - s.start[1]) * d[1]) / l2).clamp(0.0, 1.0);
            (p[0] - s.start[0] - t * d[0]).hypot(p[1] - s.start[1] - t * d[1])
        }
        Shape::Circle { center, radius } => {
            ((p[0] - center[0]).hypot(p[1] - center[1]) - radius).abs()
        }
        Shape::Arc { center, radius } => {
            let on = ((p[0] - center[0]).hypot(p[1] - center[1]) - radius).abs();
            let ends = (p[0] - s.start[0])
                .hypot(p[1] - s.start[1])
                .min((p[0] - s.end[0]).hypot(p[1] - s.end[1]));
            // On the arc's circle near the arc: good enough for a test that
            // samples the arc's own edges.
            on.min(ends)
        }
        Shape::Spline(_) => f64::INFINITY,
    }
}

/// Every curve of the flat pattern: the flats' outlines and holes, the
/// strips' ends and the cuts.
fn pattern_curves(part: &Part) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for flat in &part.flats {
        out.extend(flat.outer.iter().cloned());
        out.extend(flat.holes.iter().flatten().cloned());
    }
    for cut in &part.cuts {
        out.extend(cut.segments.iter().cloned());
    }
    for b in &part.bends {
        let (a0, a1) = b.far_line();
        for (p, q) in [(b.start, a0), (b.end, a1), (a0, a1)] {
            out.push(Segment {
                source: 0,
                piece: 0,
                reversed: false,
                start: p,
                end: q,
                shape: Shape::Line,
            });
        }
    }
    out
}

/// Fold then unfold: every edge of the folded bottom skin, sampled and
/// laid back by its face's flat motion or bend wrap, lies on the pattern's
/// curves, at z = 0 of a flat or on the radius of a bend's skin.
fn round_trip(part: &Part, tol: &Tolerance) {
    let curves = pattern_curves(part);
    let on_pattern = |p: [f64; 2], what: &str| {
        let d = curves
            .iter()
            .map(|s| to_segment(p, s))
            .fold(f64::INFINITY, f64::min);
        assert!(
            d <= 2.0 * tol.linear,
            "{what}: lands {d:e} off the pattern at {p:?}"
        );
    };
    for (f, role) in part.sheet_faces.iter().enumerate() {
        let face = part.sheet.face(craft3d_topo::FaceId(f as u32)).unwrap();
        let edges: Vec<_> = face
            .loops
            .iter()
            .flat_map(|l| part.sheet.loop_(*l).unwrap().coedges.clone())
            .map(|c| part.sheet.edge(c.edge).unwrap().clone())
            .collect();
        for edge in edges {
            for i in 0..=8 {
                let t = edge.range.0 + (edge.range.1 - edge.range.0) * f64::from(i) / 8.0;
                let world = edge.curve.eval(t);
                let (x, y, z) = part.plane.local(world);
                match role {
                    SheetFace::Flat(n) | SheetFace::FlatPiece(n, _) => {
                        let back = part.flats[*n].transform.inverse().unwrap();
                        let p = back.point(Vec3::new(x, y, z));
                        assert!(
                            p.z.abs() <= tol.linear,
                            "a flat's edge is {} off its plane",
                            p.z
                        );
                        on_pattern([p.x, p.y], "a flat's edge");
                    }
                    SheetFace::Bend(n) | SheetFace::BendPiece(n, _) => {
                        let b = &part.bends[*n];
                        let back = part.flats[b.parent].transform.inverse().unwrap();
                        let p = back.point(Vec3::new(x, y, z));
                        let len = (b.end[0] - b.start[0]).hypot(b.end[1] - b.start[1]);
                        let d = [(b.end[0] - b.start[0]) / len, (b.end[1] - b.start[1]) / len];
                        let u = (p.x - b.start[0]) * d[0] + (p.y - b.start[1]) * d[1];
                        let zc = if b.angle > 0.0 {
                            b.thickness + b.radius
                        } else {
                            -b.radius
                        };
                        let foot = [b.start[0] + u * d[0], b.start[1] + u * d[1]];
                        // From the axis: across the strip and up.
                        let v = (
                            (p.x - foot[0]) * b.across[0] + (p.y - foot[1]) * b.across[1],
                            p.z - zc,
                        );
                        let r0 = (0.0, -zc);
                        let rb = zc.abs();
                        near(
                            v.0.hypot(v.1),
                            rb,
                            tol.linear,
                            "a bend's edge from its axis",
                        );
                        let phi = (r0.0 * v.1 - r0.1 * v.0)
                            .abs()
                            .atan2(r0.0 * v.0 + r0.1 * v.1);
                        let s = phi * b.neutral();
                        let q = [foot[0] + s * b.across[0], foot[1] + s * b.across[1]];
                        on_pattern(q, "a bend's edge");
                    }
                }
            }
        }
    }
}

/// The flat pattern is one region: its outline (the chains the export
/// draws) through the sketch's profile detection is one profile with every
/// hole and the flat area.
fn one_sheet(part: &Part, tol: &Tolerance) {
    let chains = outline(part, tol);
    let curves: Vec<SketchCurve> = chains
        .iter()
        .flatten()
        .enumerate()
        .map(|(i, s)| SketchCurve {
            id: i as u64 + 1,
            curve: match &s.shape {
                Shape::Line => Curve2::Line {
                    start: s.start,
                    end: s.end,
                },
                Shape::Circle { center, radius } => Curve2::Circle {
                    center: *center,
                    radius: *radius,
                },
                Shape::Arc { center, .. } => {
                    let (a, b) = if s.reversed {
                        (s.end, s.start)
                    } else {
                        (s.start, s.end)
                    };
                    Curve2::Arc {
                        center: *center,
                        start: a,
                        end: b,
                    }
                }
                Shape::Spline(sp) => Curve2::Spline(sp.clone()),
            },
        })
        .collect();
    let found = detect_profiles(&curves, tol).unwrap();
    assert_eq!(found.len(), 1, "the flat pattern is one region");
    let holes: usize = part.flats.iter().map(|f| f.holes.len()).sum::<usize>() + part.cuts.len();
    assert_eq!(found[0].holes.len(), holes);
    let a = flat_area(part);
    near(found[0].area(), a, 1e-9 * a, "the outline's area");
}

fn healthy(part: &Part, tol: &Tolerance) {
    let defects = check(&part.solid.body, tol);
    assert!(
        defects.is_empty(),
        "the solid fails the checker: {defects:?}"
    );
    assert_eq!(part.sheet_faces.len(), part.sheet.faces.len());
}

fn refused(spec: &Spec, tol: &Tolerance) -> (Option<usize>, String) {
    match build(spec, tol) {
        Ok(_) => panic!("the part was built"),
        Err(KernelError::SheetMetal { flange, reason }) => (flange, reason),
        Err(other) => panic!("refused for another reason: {other}"),
    }
}

/// A slot 6 wide across the whole strip, 3 into the base and 4 into the
/// flange: the bend is left in two faces, and the volume and the flat area
/// lose the slot by its closed form.
#[test]
fn a_slot_across_the_whole_bend() {
    for k in SCALES {
        let tol = tolerance(k);
        for kf in [0.0, 0.33, 0.5, 1.0] {
            let ba = ba(kf);
            let (x0, x1) = (37.0, 40.0 + ba + 4.0);
            let slot = polygon(&[[x0, 12.0], [x1, 12.0], [x1, 18.0], [x0, 18.0]], k);
            let part = build(&part_spec(k, kf, vec![slot]), &tol).unwrap();
            healthy(&part, &tol);
            assert_eq!(part.cuts.len(), 1);
            assert_eq!(
                part.sheet_faces,
                vec![
                    SheetFace::Flat(0),
                    SheetFace::Flat(1),
                    SheetFace::Bend(0),
                    SheetFace::BendPiece(0, 1)
                ]
            );
            let w = 6.0 * k;
            let want = whole_volume(k) - w * 7.0 * k * k - w * ba * k * strip_factor(k, kf);
            near(volume(&part), want, 1e-7 * want, "the volume");
            let area = (40.0 * 30.0 + ba * 30.0 + 15.0 * 30.0 - 6.0 * (7.0 + ba)) * k * k;
            near(flat_area(&part), area, 1e-9 * area, "the flat area");
            one_sheet(&part, &tol);
            round_trip(&part, &tol);
        }
    }
}

/// Round holes: one wholly inside the strip (the bend face gets a hole),
/// one whose centre is on the base's tangent line (half on the base, half
/// on the strip).
#[test]
fn round_holes_on_the_strip_and_across_a_tangent_line() {
    for k in SCALES {
        let tol = tolerance(k);
        for kf in [0.33, 0.5] {
            let ba = ba(kf);
            let rho = ba / 4.0;
            let inside = circle([40.0 + ba / 2.0, 10.0], rho, k);
            let across = circle([40.0, 22.0], 1.0, k);
            let part = build(&part_spec(k, kf, vec![inside, across]), &tol).unwrap();
            healthy(&part, &tol);
            assert_eq!(part.cuts.len(), 2);
            assert_eq!(part.sheet_faces.len(), 3);
            let f = strip_factor(k, kf);
            let a_in = PI * rho * rho * k * k;
            let half = 0.5 * PI * k * k;
            let want = whole_volume(k) - a_in * f - half * k - half * f;
            near(volume(&part), want, 1e-7 * want, "the volume");
            one_sheet(&part, &tol);
            round_trip(&part, &tol);
        }
    }
}

/// A diamond whose two corners lie on the base's tangent line: crossed at
/// its vertices, half on each side.
#[test]
fn a_hole_crossed_at_its_corners() {
    for k in SCALES {
        let tol = tolerance(k);
        let kf = 0.33;
        let diamond = polygon(&[[38.5, 15.0], [40.0, 13.0], [41.5, 15.0], [40.0, 17.0]], k);
        let part = build(&part_spec(k, kf, vec![diamond]), &tol).unwrap();
        healthy(&part, &tol);
        let want = whole_volume(k) - 3.0 * k * k * k - 3.0 * k * k * strip_factor(k, kf);
        near(volume(&part), want, 1e-7 * want, "the volume");
        one_sheet(&part, &tol);
        round_trip(&part, &tol);
    }
}

/// A slot from the middle of the strip into the flange: the far tangent
/// line is crossed, the base's not.
#[test]
fn a_slot_from_the_strip_into_the_flange() {
    for k in SCALES {
        let tol = tolerance(k);
        for kf in [0.0, 0.5] {
            let ba = ba(kf);
            let (x0, x1) = (40.0 + ba / 2.0, 40.0 + ba + 5.0);
            let slot = polygon(&[[x0, 5.0], [x1, 5.0], [x1, 9.0], [x0, 9.0]], k);
            let part = build(&part_spec(k, kf, vec![slot]), &tol).unwrap();
            healthy(&part, &tol);
            assert_eq!(part.sheet_faces.len(), 3);
            let want = whole_volume(k)
                - 4.0 * 5.0 * k * k * k
                - 4.0 * (ba / 2.0) * k * k * strip_factor(k, kf);
            near(volume(&part), want, 1e-7 * want, "the volume");
            one_sheet(&part, &tol);
            round_trip(&part, &tol);
        }
    }
}

/// A spline hole wholly inside the strip is wrapped onto the bend.
#[test]
fn a_spline_hole_inside_the_strip() {
    for k in SCALES {
        let tol = tolerance(k);
        let kf = 0.5;
        let ba = ba(kf);
        let (xa, xb) = (40.0 + 0.2 * ba, 40.0 + 0.8 * ba);
        let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
        let curves = vec![
            line(20, s([xb, 20.0]), s([xa, 20.0])),
            SketchCurve {
                id: 21,
                curve: Curve2::Spline(Spline2 {
                    degree: 3,
                    knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                    control: [[xa, 20.0], [xa, 23.0], [xb, 24.0], [xb, 20.0]]
                        .iter()
                        .map(|p| s(*p))
                        .collect(),
                }),
            },
        ];
        let part = build(&part_spec(k, kf, vec![cutout(curves, k)]), &tol).unwrap();
        healthy(&part, &tol);
        let wrapped = part
            .sheet
            .edges
            .iter()
            .filter(|e| matches!(e.curve, Curve3::Nurbs(_)))
            .count();
        assert!(wrapped >= 1, "the spline is wrapped onto the bend");
        // At K = ½ the volume is the flat area times T.
        let want = flat_area(&part) * k;
        near(volume(&part), want, 1e-6 * want, "the volume at K = ½");
        one_sheet(&part, &tol);
    }
}

/// Refused by reason, naming the cut-out's curves: one touching a tangent
/// line from the strip, one crossing it with a spline, one across the
/// strip's end (outside the pattern).
#[test]
fn holes_across_a_bend_refused_by_reason() {
    for k in SCALES {
        let tol = tolerance(k);
        let kf = 0.5;
        let ba = ba(kf);
        let rho = 0.25 * ba;
        let touching = circle([40.0 + ba - rho, 15.0], rho, k);
        let (flange, reason) = refused(&part_spec(k, kf, vec![touching]), &tol);
        assert_eq!(flange, None);
        assert!(
            reason.contains("curve 20") && reason.contains("touches"),
            "{reason}"
        );

        let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
        let curves = vec![
            line(20, s([42.0, 20.0]), s([38.0, 20.0])),
            SketchCurve {
                id: 21,
                curve: Curve2::Spline(Spline2 {
                    degree: 3,
                    knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                    control: [[38.0, 20.0], [38.0, 23.0], [42.0, 23.0], [42.0, 20.0]]
                        .iter()
                        .map(|p| s(*p))
                        .collect(),
                }),
            },
        ];
        let (_, reason) = refused(&part_spec(k, kf, vec![cutout(curves, k)]), &tol);
        assert!(reason.contains("spline"), "{reason}");

        let past = circle([40.0 + ba / 2.0, 0.0], 0.5, k);
        let (_, reason) = refused(&part_spec(k, kf, vec![past]), &tol);
        assert!(
            reason.contains("curve 20") && reason.contains("no one flat"),
            "{reason}"
        );
    }
}
