//! Holes across a bend in a part whose closed corner trims a bend
//! (docs/design/sheetmetal.md › *Holes across a bend*, *Trimming a bend*),
//! held to closed forms at 1e-3, 1 and 1e6.
//!
//! The part: a base corner of 120 degrees without a relief, flanges up 90
//! degrees on both edges, `R = T`, closed by overlap with a gap `0.6 R`,
//! so the covered bend is trimmed at its start or its end (the closed
//! forms of `sheetmetal_trims.rs`). Round holes across the middle of both
//! bends, and a slot across the covered bend beside its trimmed end:
//!
//! - the volume: the trimmed part's closed form less each hole's area on
//!   the flats times `T` and on the strip times `T (R + T/2) / (R + K T)`;
//! - the flat area: the trimmed part's less the holes';
//! - fold then unfold: every edge of the folded bottom skin laid back (a
//!   flat by its motion, a bend by the inverse wrap) lies on the pattern's
//!   curves or, on a trimmed end, on the trim's sinusoid `u(s/ρ)`;
//! - the trimmed end keeps its names (its arc and its ellipse).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Curve3;
use craft3d_kernel::profile::{Curve2, Profile, Segment, Shape, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, CornerKind, Cutout, Direction, EdgeKey, FlangeSpec, LengthFrom, On, Part, SheetFace,
    Spec, build, edge_numbers, flat_area,
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

fn polygon(points: &[[f64; 2]], k: f64) -> Profile {
    let n = points.len();
    let curves: Vec<SketchCurve> = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            line(i as u64 + 1, [a[0] * k, a[1] * k], [b[0] * k, b[1] * k])
        })
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

/// A circle in model units (already scaled).
fn circle(id: u64, center: [f64; 2], radius: f64, k: f64) -> Cutout {
    cutout(
        vec![SketchCurve {
            id,
            curve: Curve2::Circle { center, radius },
        }],
        k,
    )
}

/// A quadrilateral in model units (already scaled).
fn quad(id: u64, p: [[f64; 2]; 4], k: f64) -> Cutout {
    cutout(
        (0..4)
            .map(|i| line(id + i as u64, p[i], p[(i + 1) % 4]))
            .collect(),
        k,
    )
}

fn flange(edge: u64, length: f64, angle: f64, r: f64, kf: f64) -> FlangeSpec {
    FlangeSpec {
        on: On::Base(edge),
        length,
        angle,
        direction: Direction::Up,
        radius: r,
        k: kf,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
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

/// `∫ sqrt(ρ² − a²) da` and `∫ a sqrt(ρ² − a²) da` from `x` to `ρ`.
fn root_integrals(rho: f64, x: f64) -> (f64, f64) {
    let at = |a: f64| {
        let q = (rho * rho - a * a).max(0.0).sqrt();
        (
            0.5 * (a * q + rho * rho * (a / rho).clamp(-1.0, 1.0).pasin()),
            -(q * q * q) / 3.0,
        )
    };
    let (f1, g1) = at(rho);
    let (f0, g0) = at(x);
    (f1 - f0, g1 - g0)
}

/// The area of the disc of radius `r` beyond a chord `d` from its centre
/// (`d` may be negative: more than half).
fn beyond(r: f64, d: f64) -> f64 {
    if d >= r {
        return 0.0;
    }
    r * r * (d / r).pacos() - d * (r * r - d * d).sqrt()
}

/// A point of the pattern by its place on a bend's strip.
fn at(b: &craft3d_kernel::sheetmetal::Bend, u: f64, s: f64) -> [f64; 2] {
    let len = (b.end[0] - b.start[0]).phypot(b.end[1] - b.start[1]);
    let d = [(b.end[0] - b.start[0]) / len, (b.end[1] - b.start[1]) / len];
    [
        b.start[0] + u * d[0] + s * b.across[0],
        b.start[1] + u * d[1] + s * b.across[1],
    ]
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

/// Fold then unfold: every edge of the folded bottom skin, sampled and
/// laid back, lies on a line, arc or circle of the pattern (the flats'
/// outlines and holes, the cuts, the strips' tangent lines and square
/// ends) or, on a bend, on a trimmed end's sinusoid.
fn round_trip(part: &Part, tol: &Tolerance) {
    let mut curves: Vec<Segment> = Vec::new();
    for flat in &part.flats {
        curves.extend(flat.outer.iter().cloned());
        curves.extend(flat.holes.iter().flatten().cloned());
    }
    for cut in &part.cuts {
        curves.extend(cut.segments.iter().cloned());
    }
    for b in &part.bends {
        let (a0, a1) = b.far_line();
        for (p, q) in [(b.start, a0), (b.end, a1), (a0, a1), (b.start, b.end)] {
            curves.push(Segment {
                source: 0,
                piece: 0,
                reversed: false,
                start: p,
                end: q,
                shape: Shape::Line,
            });
        }
    }
    let off = |p: [f64; 2]| {
        curves
            .iter()
            .map(|s| to_segment(p, s))
            .fold(f64::INFINITY, f64::min)
    };
    for (f, role) in part.sheet_faces.iter().enumerate() {
        let face = part.sheet.face(craft3d_topo::FaceId(f as u32)).unwrap();
        for l in &face.loops {
            for c in &part.sheet.loop_(*l).unwrap().coedges {
                let edge = part.sheet.edge(c.edge).unwrap();
                for i in 0..=8 {
                    let t = edge.range.0 + (edge.range.1 - edge.range.0) * f64::from(i) / 8.0;
                    let (x, y, z) = part.plane.local(edge.curve.eval(t));
                    match role {
                        SheetFace::Flat(n) | SheetFace::FlatPiece(n, _) => {
                            let back = part.flats[*n].transform.inverse().unwrap();
                            let p = back.point(Vec3::new(x, y, z));
                            assert!(p.z.abs() <= tol.linear, "off its flat by {}", p.z);
                            let d = off([p.x, p.y]);
                            assert!(d <= 2.0 * tol.linear, "a flat's edge lands {d:e} off");
                        }
                        SheetFace::Bend(n) | SheetFace::BendPiece(n, _) => {
                            let b = &part.bends[*n];
                            let back = part.flats[b.parent].transform.inverse().unwrap();
                            let p = back.point(Vec3::new(x, y, z));
                            let len = (b.end[0] - b.start[0]).phypot(b.end[1] - b.start[1]);
                            let d = [(b.end[0] - b.start[0]) / len, (b.end[1] - b.start[1]) / len];
                            let u = (p.x - b.start[0]) * d[0] + (p.y - b.start[1]) * d[1];
                            let (zc, sigma) = if b.angle > 0.0 {
                                (b.thickness + b.radius, 1.0)
                            } else {
                                (-b.radius, -1.0)
                            };
                            let out =
                                (p.x - b.start[0]) * b.across[0] + (p.y - b.start[1]) * b.across[1];
                            let up = sigma * (zc - p.z);
                            near(out.phypot(up), zc.abs(), tol.linear, "from the axis");
                            let phi = out.patan2(up);
                            let s = phi * b.neutral();
                            let on_trim = b.trim.iter().flatten().any(|tr| {
                                phi >= tr.phi - 1e-9 && (tr.u(phi) - u).abs() <= tol.linear
                            });
                            let dd = off(at(b, u, s));
                            assert!(
                                on_trim || dd <= 2.0 * tol.linear,
                                "a bend's edge lands {dd:e} off the pattern"
                            );
                        }
                        SheetFace::CurvedBend(_) | SheetFace::CurvedFlat(_) => {
                            panic!("no curved flange in these parts")
                        }
                    }
                }
            }
        }
    }
}

/// The 120-degree corner part (`sheetmetal_trims.rs`) with these cut-outs,
/// and its volume and flat area without them in closed form.
struct Corner {
    spec: Spec,
    volume: f64,
    area: f64,
}

fn corner(k: f64, kf: f64, over: Option<usize>) -> Corner {
    let (t, r, l) = (1.0 * k, 1.0 * k, 10.0 * k);
    let g = 0.6 * r;
    let base = polygon(&[[0.0, 0.0], [60.0, 0.0], [80.0, 34.64], [0.0, 34.64]], k);
    let len_a = 60.0 * k;
    let len_b = (20.0f64).phypot(34.64) * k;
    let (s6, c6) = (34.64 / (len_b / k), 20.0 / (len_b / k));
    let base_area = 0.5 * 34.64 * (60.0 + 80.0) * k * k;
    let spec = Spec {
        base,
        plane: Frame::WORLD,
        thickness: t,
        flanges: vec![
            flange(1, l, FRAC_PI_2, r, kf),
            flange(2, l, FRAC_PI_2, r, kf),
        ],
        relief: None,
        bend_relief: None,
        corner: Closing {
            kind: CornerKind::Overlap,
            gap: g,
            over,
        },
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs: Vec::new(),
        ids: Vec::new(),
    };
    let (ri, ro) = (r, r + t);
    let y0 = -r + g;
    let a_star = -y0 / c6;
    let (fo, go) = root_integrals(ro, a_star);
    let (fi, gi) = root_integrals(ri, a_star);
    let bend_cut = (c6 * (go - gi) + y0 * (fo - fi)) / s6;
    let wall_cut = l / s6 * (c6 * (ro * ro - ri * ri) / 2.0 + y0 * t);
    let cover_ext = l / s6 * (ro * t - c6 * (ro * ro - ri * ri) / 2.0);
    let bends_full = FRAC_PI_2 * (len_a + len_b) * t * (r + 0.5 * t);
    let volume =
        base_area * t + bends_full + l * t * (len_a + len_b) - bend_cut - wall_cut + cover_ext;
    let rho = r + kf * t;
    let ba = FRAC_PI_2 * rho;
    let phi1 = (a_star / ro).pasin();
    let strip_cut = (y0 * rho * (FRAC_PI_2 - phi1) + c6 * ro * rho * phi1.pcos()) / s6;
    let area = base_area + ba * (len_a + len_b) + l * (len_a + len_b)
        - strip_cut
        - l * (c6 * ro + y0) / s6
        + l * ro * (1.0 - c6) / s6;
    Corner { spec, volume, area }
}

/// Round holes of radius 2 across the middle of both bends (their centres
/// on the strips' middles, so each crosses both tangent lines), and a slot
/// 1.5 wide across the covered bend, 1 into each flat, its near side 1.5
/// from the trimmed end (the trim takes about 0.69 at the flange's tangent
/// line), for either flange covering.
#[test]
fn holes_across_the_bends_of_a_part_with_a_trimmed_bend() {
    for k in SCALES {
        let tol = tolerance(k);
        for (kf, over) in [(0.4, None), (0.5, Some(1)), (0.33, Some(0))] {
            let what = format!("scale {k}, K {kf}, over {over:?}");
            let c = corner(k, kf, over);
            // The bends as the part lays them out.
            let plain = build(&c.spec, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            near(
                volume(&plain),
                c.volume,
                1e-8 * c.volume,
                &format!("{what}: plain"),
            );
            let covered = if over == Some(1) { 0 } else { 1 };
            let end = if covered == 0 { 1 } else { 0 };
            assert!(plain.bends[covered].trim[end].is_some(), "{what}");
            let (t, r) = (1.0 * k, 1.0 * k);
            let rho = r + kf * t;
            let ba = FRAC_PI_2 * rho;
            let factor = t * (r + 0.5 * t) / rho;
            let rh = 2.0 * k;
            let mut cutouts = Vec::new();
            let mut taken = 0.0;
            let mut holes_area = 0.0;
            for (n, b) in plain.bends.iter().enumerate() {
                let len = (b.end[0] - b.start[0]).phypot(b.end[1] - b.start[1]);
                cutouts.push(circle(20 + n as u64, at(b, len / 2.0, ba / 2.0), rh, k));
                let flats = 2.0 * beyond(rh, ba / 2.0);
                let strip = PI * rh * rh - flats;
                taken += flats * t + strip * factor;
                holes_area += PI * rh * rh;
            }
            let b = &plain.bends[covered];
            let len = (b.end[0] - b.start[0]).phypot(b.end[1] - b.start[1]);
            let (u0, u1) = if end == 0 {
                (1.5 * k, 3.0 * k)
            } else {
                (len - 3.0 * k, len - 1.5 * k)
            };
            let (s0, s1) = (-k, ba + k);
            cutouts.push(quad(
                30,
                [at(b, u0, s0), at(b, u1, s0), at(b, u1, s1), at(b, u0, s1)],
                k,
            ));
            let w = u1 - u0;
            taken += 2.0 * w * k * t + w * ba * factor;
            holes_area += w * (s1 - s0);

            let mut spec = c.spec.clone();
            spec.cutouts = cutouts;
            let part = build(&spec, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(part, build(&spec, &tol).unwrap(), "{what}: deterministic");
            let defects = check(&part.solid.body, &tol);
            assert!(defects.is_empty(), "{what}: {defects:?}");
            assert_eq!(part.cuts.len(), 3, "{what}");
            assert_eq!(
                part.bends[covered].trim, plain.bends[covered].trim,
                "{what}"
            );
            let v = c.volume - taken;
            near(
                volume(&part),
                v,
                1e-8 * c.volume,
                &format!("{what}: volume"),
            );
            let a = c.area - holes_area;
            near(flat_area(&part), a, 1e-9 * c.area, &format!("{what}: area"));
            round_trip(&part, &tol);
            // The slot leaves the covered bend in two faces.
            assert!(
                part.sheet_faces.contains(&SheetFace::BendPiece(covered, 1)),
                "{what}: {:?}",
                part.sheet_faces
            );
            // The trimmed end keeps its arc and its ellipse, named as
            // without the holes.
            let trim = part
                .edge_keys
                .iter()
                .position(|key| *key == EdgeKey::Trim { bend: covered, end })
                .expect("the trimmed end's ellipse");
            assert!(matches!(part.sheet.edges[trim].curve, Curve3::Ellipse(_)));
            for key in [
                EdgeKey::Trim { bend: covered, end },
                EdgeKey::Arc { bend: covered, end },
            ] {
                assert!(part.edge_keys.contains(&key), "{what}: {key:?}");
                assert_eq!(
                    edge_numbers(&part, key),
                    edge_numbers(&plain, key),
                    "{what}: {key:?}"
                );
            }
        }
    }
}
