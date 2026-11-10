//! Closed corners that cut a flange back (docs/design/sheetmetal.md ›
//! *Trimming a bend*), held to closed forms at 1e-3, 1 and 1e6:
//!
//! - walls that lean towards each other at a square corner, mitred: the
//!   mitre plane is `x + y = W` whatever the angle (the outer skins'
//!   bisector), so each wall's flat ends on `x + y = W − g/√2` and its tip
//!   is trimmed back, while the bends are not touched; the flat area and
//!   the volume in closed form;
//! - a base corner of 120 degrees without a relief, closed by overlap: the
//!   covered flange's end plane (the cover's inside face moved by the gap)
//!   cuts its bend over `[φ₁, θ]` on both skins and its wall; the bend's
//!   end is its arc to `φ₁` and the plane's ellipse beyond; the volume
//!   taken away is `∫ (c a + y₀) / s da dh` over the quarter annulus beyond
//!   `a = −y₀ / c` (closed form), the strip's area taken away likewise;
//! - fold then unfold, names kept, refusals by reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Curve3;
use craft3d_kernel::KernelError;
use craft3d_kernel::profile::{Curve2, Profile, Shape, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, CornerKind, Direction, EdgeKey, FlangeSpec, LengthFrom, On, Part, SheetFace, Spec,
    build, clearance_past_end, developed_allowance, edge_numbers, flat_area, outline,
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

fn spec(base: Profile, t: f64, flanges: Vec<FlangeSpec>, corner: Closing) -> Spec {
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: t,
        flanges,
        relief: None,
        bend_relief: None,
        corner,
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
/// vertices land on a corner of its outline at z = 0, each bend's neutral
/// band develops to its strip's width, and every trimmed end's ellipse,
/// laid flat by the inverse wrap (`u` along the axis, `s = ρ φ`), lies on
/// the trim's sinusoid; the pattern's spline holds the sinusoid within a
/// quarter of the tolerance.
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
    for (e, key) in part.edge_keys.iter().enumerate() {
        let EdgeKey::Trim { bend, end } = *key else {
            continue;
        };
        let b = &part.bends[bend];
        let t = b.trim[end].unwrap();
        let w = part.flats[b.parent].transform;
        let back = w.inverse().unwrap();
        let edge = &part.sheet.edges[e];
        assert!(matches!(edge.curve, Curve3::Ellipse(_)));
        let d = {
            let v = [b.end[0] - b.start[0], b.end[1] - b.start[1]];
            let l = v[0].phypot(v[1]);
            [v[0] / l, v[1] / l]
        };
        let (zc, sigma) = if b.angle > 0.0 {
            (b.thickness + b.radius, 1.0)
        } else {
            (-b.radius, -1.0)
        };
        for j in 0..=32 {
            let s = edge.range.0 + (edge.range.1 - edge.range.0) * j as f64 / 32.0;
            let world = edge.curve.eval(s);
            let (x, y, z) = part.plane.local(world);
            let p = back.point(Vec3::new(x, y, z));
            let rel = [p.x - b.start[0], p.y - b.start[1]];
            let u = rel[0] * d[0] + rel[1] * d[1];
            let out = rel[0] * b.across[0] + rel[1] * b.across[1];
            let up = sigma * (zc - p.z);
            let phi = out.patan2(up);
            near(t.u(phi), u, tol.linear, "the trimmed end laid flat");
        }
        // The pattern's spline of this end, piece by piece (double inner
        // knots: the Bézier joints are the middles of the inner controls).
        let chain: Vec<_> = outline(part, tol).into_iter().flatten().collect();
        let mut held = false;
        for seg in &chain {
            let Shape::Spline(sp) = &seg.shape else {
                continue;
            };
            let c = &sp.control;
            let n = (c.len() - 2) / 2;
            let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            let first = c[0];
            let last = c[c.len() - 1];
            let on_strip = |q: [f64; 2]| {
                let rel = [q[0] - b.start[0], q[1] - b.start[1]];
                let s = rel[0] * b.across[0] + rel[1] * b.across[1];
                let u = rel[0] * d[0] + rel[1] * d[1];
                (u, s)
            };
            let (_, s0) = on_strip(first);
            let (_, s1) = on_strip(last);
            if !(s0 > -tol.linear && s1 <= b.allowance + tol.linear) {
                continue;
            }
            let (u_first, _) = on_strip(first);
            if (u_first - t.u(s0 / b.neutral())).abs() > tol.linear {
                continue;
            }
            held = true;
            for k in 0..n {
                let p0 = if k == 0 {
                    first
                } else {
                    mid(c[2 * k], c[2 * k + 1])
                };
                let p3 = if k + 1 == n {
                    last
                } else {
                    mid(c[2 * k + 2], c[2 * k + 3])
                };
                let (p1, p2) = (c[2 * k + 1], c[2 * k + 2]);
                for j in 0..=8 {
                    let v = j as f64 / 8.0;
                    let w = 1.0 - v;
                    let q = [
                        w * w * w * p0[0]
                            + 3.0 * w * w * v * p1[0]
                            + 3.0 * w * v * v * p2[0]
                            + v * v * v * p3[0],
                        w * w * w * p0[1]
                            + 3.0 * w * w * v * p1[1]
                            + 3.0 * w * v * v * p2[1]
                            + v * v * v * p3[1],
                    ];
                    let (u, s) = on_strip(q);
                    near(
                        u,
                        t.u(s / b.neutral()),
                        0.25 * tol.linear,
                        "the pattern's spline against the sinusoid",
                    );
                }
            }
        }
        assert!(held, "the trimmed end's spline is in the outline");
    }
}

/// Walls bent up θ = 120 degrees on the bottom and right edges of a base
/// `W × D`, the corner mitred with a gap `g`.
#[test]
fn walls_that_lean_towards_each_other_are_mitred_and_trimmed() {
    for k in SCALES {
        let tol = tolerance(k);
        let (w, dd, t, r, l, g) = (60.0 * k, 40.0 * k, 1.0 * k, 1.0 * k, 10.0 * k, 0.3 * k);
        let theta = 2.0 * PI / 3.0;
        for kf in [0.33, 0.5] {
            let what = format!("scale {k}, K {kf}");
            let ba = theta * (r + kf * t);
            let base = polygon(&[[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]], k);
            let sp = spec(
                base.clone(),
                t,
                vec![flange(1, l, theta, r, kf), flange(2, l, theta, r, kf)],
                Closing {
                    kind: CornerKind::Miter,
                    gap: g,
                    over: None,
                },
            );
            let part = build(&sp, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(part, build(&sp, &tol).unwrap(), "{what}: deterministic");
            healthy(&part, &tol);
            assert!(part.bends.iter().all(|b| b.trim == [None, None]));
            let ro = r + t;
            let (s, c) = theta.psin_cos();
            let shift = ro * s - g / 2f64.sqrt();
            // Flat area: base, strips, and each wall `L (edge + shift) +
            // L² cos θ / 2`.
            let walls = l * (w + shift) + l * (dd + shift) + l * l * c;
            let area = w * dd + ba * (w + dd) + walls;
            near(
                flat_area(&part),
                area,
                1e-9 * area,
                &format!("{what}: area"),
            );
            // Volume: base, bends, and each wall `L T (edge + shift + L
            // cos θ / 2 − T sin θ / 2)`.
            let bends = theta * (w + dd) * t * (r + 0.5 * t);
            let wall = |edge: f64| l * t * (edge + shift + 0.5 * l * c - 0.5 * t * s);
            let v = w * dd * t + bends + wall(w) + wall(dd);
            near(volume(&part), v, 1e-8 * v, &format!("{what}: volume"));
            // The two walls' ends lie g/√2 either side of x + y = W.
            let body = &part.solid.body;
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for f in craft3d_kernel::sheetmetal::flat_faces(&part, 1) {
                for lp in &body.faces[f].loops {
                    for co in &body.loop_(*lp).unwrap().coedges {
                        let p = body.vertex(body.coedge_ends(*co).unwrap().0).unwrap().point;
                        hi = hi.max(p.x + p.y);
                    }
                }
            }
            for f in craft3d_kernel::sheetmetal::flat_faces(&part, 2) {
                for lp in &body.faces[f].loops {
                    for co in &body.loop_(*lp).unwrap().coedges {
                        let p = body.vertex(body.coedge_ends(*co).unwrap().0).unwrap().point;
                        lo = lo.min(p.x + p.y);
                    }
                }
            }
            let root2 = 2f64.sqrt();
            near(
                hi,
                w - g / root2,
                10.0 * tol.linear,
                &format!("{what}: 0's end"),
            );
            near(
                lo,
                w + g / root2,
                10.0 * tol.linear,
                &format!("{what}: 1's end"),
            );
            // Names: every name of the part with the corner open is kept.
            let open = build(&spec(base, t, sp.flanges.clone(), Closing::OPEN), &tol).unwrap();
            let (n_open, n_closed) = (names(&open), names(&part));
            assert!(
                n_open.is_subset(&n_closed),
                "{what}: lost {:?}",
                n_open.difference(&n_closed).collect::<Vec<_>>()
            );
            assert_eq!(outline(&part, &tol).len(), 1, "{what}: one chain");
        }
    }
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

/// A base corner of 120 degrees (the bottom edge to `(60, 0)`, then on
/// towards `(80, 34.64)`), flanges up 90 degrees on both edges, `R = T`,
/// no relief, closed by overlap with a gap `0.6 R`. The covered flange's
/// end plane is the cover's inside face moved by the gap, `y₀ = −R + g`
/// across the cover: at `a` across the covered bend (its outward reach,
/// `r sin φ`) it takes away `(c a + y₀) / s` along the bend wherever that
/// is positive (`s = sin 60°`, `c = cos 60°`), so beyond `a* = −y₀/c`:
/// on the bottom skin from `sin φ₁ = a*/(R + T)`, on the top from
/// `sin φ₁′ = a*/R`. The cover runs on to the covered wall's outside, by
/// `(R + T − c a)/s` at `a` across it. Either flange may cover: the
/// closed forms are the same, the trim at the covered bend's start or end.
#[test]
fn a_wide_corner_without_a_relief_trims_the_covered_bend() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, l) = (1.0 * k, 1.0 * k, 10.0 * k);
        let g = 0.6 * r;
        let pts = [[0.0, 0.0], [60.0, 0.0], [80.0, 34.64], [0.0, 34.64]];
        let base = polygon(&pts, k);
        let len_a = 60.0 * k;
        let len_b = (20.0f64).phypot(34.64) * k;
        let dir = [20.0 / (len_b / k), 34.64 / (len_b / k)];
        // The turn at the corner, from the bottom edge's direction (1, 0).
        let (s6, c6) = (dir[1], dir[0]);
        let base_area = 0.5 * 34.64 * (60.0 + 80.0) * k * k;
        for (kf, over) in [(0.4, None), (0.5, Some(1)), (0.33, Some(0))] {
            let what = format!("scale {k}, K {kf}, over {over:?}");
            let mut sp = spec(
                base.clone(),
                t,
                vec![
                    flange(1, l, FRAC_PI_2, r, kf),
                    flange(2, l, FRAC_PI_2, r, kf),
                ],
                Closing {
                    kind: CornerKind::Overlap,
                    gap: g,
                    over,
                },
            );
            let part = build(&sp, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            healthy(&part, &tol);
            let covered = if over == Some(1) { 0 } else { 1 };
            let end = if covered == 0 { 1 } else { 0 };
            let trim = part.bends[covered].trim[end].expect("the covered bend is trimmed");
            assert!(part.bends[1 - covered].trim == [None, None]);
            let (ri, ro) = (r, r + t);
            let y0 = -r + g;
            let a_star = -y0 / c6;
            near(trim.phi.psin(), a_star / ro, 1e-12, &format!("{what}: φ₁"));
            near(
                trim.phi_top.psin(),
                a_star / ri,
                1e-12,
                &format!("{what}: φ₁′"),
            );
            // The material taken from the bend: ∫ (c a + y₀)/s H(a) da
            // over a > a*, H the quarter annulus's height at a.
            let (fo, go) = root_integrals(ro, a_star);
            let (fi, gi) = root_integrals(ri, a_star);
            let bend_cut = (c6 * (go - gi) + y0 * (fo - fi)) / s6;
            // From the covered wall (a in [R, R + T], every a > a*).
            let wall_cut = l / s6 * (c6 * (ro * ro - ri * ri) / 2.0 + y0 * t);
            // The cover's extension: (R + T − c a)/s over a in [R, R + T].
            let cover_ext = l / s6 * (ro * t - c6 * (ro * ro - ri * ri) / 2.0);
            let bends_full = FRAC_PI_2 * (len_a + len_b) * t * (r + 0.5 * t);
            let v = base_area * t + bends_full + l * t * (len_a + len_b) - bend_cut - wall_cut
                + cover_ext;
            near(volume(&part), v, 1e-8 * v, &format!("{what}: volume"));
            // The flat: the strip loses ∫ (y₀ + c (R+T) sin(s/ρ))/s ds
            // over [ρ φ₁, BA], the wall (c (R + T) + y₀)/s × L, the cover
            // gains (R + T)(1 − c)/s × L.
            let rho = r + kf * t;
            let ba = FRAC_PI_2 * rho;
            let strip_cut =
                (y0 * rho * (FRAC_PI_2 - trim.phi) + c6 * ro * rho * trim.phi.pcos()) / s6;
            near(
                trim.area,
                strip_cut,
                1e-9 * ba * l,
                &format!("{what}: the strip's cut"),
            );
            let area = base_area + ba * (len_a + len_b) + l * (len_a + len_b)
                - strip_cut
                - l * (c6 * ro + y0) / s6
                + l * ro * (1.0 - c6) / s6;
            near(
                flat_area(&part),
                area,
                1e-9 * area,
                &format!("{what}: area"),
            );
            assert_eq!(outline(&part, &tol).len(), 1, "{what}: one chain");
            // Names: the open part's names all kept, and one new edge
            // name per trim (`[14, b, end]`).
            sp.corner = Closing::OPEN;
            let open = build(&sp, &tol).unwrap();
            let (n_open, n_closed) = (names(&open), names(&part));
            assert!(
                n_open.is_subset(&n_closed),
                "{what}: lost {:?}",
                n_open.difference(&n_closed).collect::<Vec<_>>()
            );
            assert!(n_closed.contains(&vec![14, covered as u64, end as u64]));
        }
    }
}

/// What a trim does not build is refused, naming the flange and the way
/// out: a gap that leaves the top skin uncut where the bottom is cut (a
/// step), a gap so large that the plane reaches the parent's tangent
/// line.
#[test]
fn trims_refused_by_reason() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, r, l) = (1.0 * k, 1.0 * k, 10.0 * k);
        let pts = [[0.0, 0.0], [60.0, 0.0], [80.0, 34.64], [0.0, 34.64]];
        let base = polygon(&pts, k);
        let with = |g: f64| {
            spec(
                base.clone(),
                t,
                vec![
                    flange(1, l, FRAC_PI_2, r, 0.4),
                    flange(2, l, FRAC_PI_2, r, 0.4),
                ],
                Closing {
                    kind: CornerKind::Overlap,
                    gap: g,
                    over: None,
                },
            )
        };
        let (f, why) = refused(&with(0.3 * r), &tol);
        assert_eq!(f, Some(1));
        assert!(why.contains("but not its top skin"), "{why}");
        assert!(why.contains("relief"), "{why}");
        let (f, why) = refused(&with(1.5 * r), &tol);
        assert_eq!(f, Some(1));
        assert!(why.contains("parent's tangent line"), "{why}");
        // The same corner bent down is the mirror image through the
        // sheet's middle: the plane cuts the covered bend's top skin (its
        // outside) and not its bottom one, so the flat is not cut back on
        // the bottom skin and no trim is made; left square, the bend would
        // run through the cover (review of 47275cf1: built, a body
        // through itself, no refusal).
        let mut down = with(0.3 * r);
        for f in &mut down.flanges {
            f.direction = Direction::Down;
        }
        let (f, why) = refused(&down, &tol);
        assert_eq!(f, Some(1));
        assert!(why.contains("top skin"), "{why}");
        assert!(why.contains("not supported"), "{why}");
    }
}

/// A mitre of walls unlike each other (inner radius 1 on one, 3 on the
/// other) at a corner without a relief: the halving plane does not pass
/// through the corner's point, so one bend's root reaches past its end
/// plane by more than half the gap (review of cc7b9228: let through as the
/// corner's own cut, with nothing proving the other wall leaves room). It
/// is kept only when the other flange's flat and what it carries are
/// proven apart from the bend: square walls (the wall `R + T` out from its
/// edge, every point of it past the bend's end) build, a valid solid, both
/// bends left square; a return on the other wall's tip turned in over the
/// base, high above the bend, is proven apart by the bend's cross-section
/// (a separating axis square to its axis) and builds, its volume the
/// return's closed form (until 2026-11-04 it was refused: the proof was one
/// half-space along the tangent line); a wall leaning in at 120 degrees
/// beside a square one of another radius, whose flat comes back over the
/// square one's bend end, is refused by reason, never left square.
#[test]
fn a_mitre_of_unlike_walls_is_kept_only_where_the_other_wall_is_clear() {
    for k in SCALES {
        let tol = tolerance(k);
        let (t, l) = (1.0 * k, 10.0 * k);
        let base = polygon(&[[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]], k);
        let corner = Closing {
            kind: CornerKind::Miter,
            gap: 0.3 * k,
            over: None,
        };
        let square = spec(
            base.clone(),
            t,
            vec![
                flange(1, l, FRAC_PI_2, 1.0 * k, 0.4),
                flange(2, l, FRAC_PI_2, 3.0 * k, 0.4),
            ],
            corner,
        );
        let part = build(&square, &tol).unwrap_or_else(|e| panic!("scale {k}: {e}"));
        assert!(check(&part.solid.body, &tol).is_empty(), "scale {k}");
        assert!(
            part.bends.iter().all(|b| b.trim == [None, None]),
            "scale {k}"
        );
        // Kept by the clearance, not because nothing is cut. The halving
        // plane through the outer skins' meeting line (x = 60 + R_B + T,
        // y = −(R_A + T)) misses the corner (60, 0) by (R_B − R_A)/√2, so
        // the wall of radius 3 (flange 1, its start at the corner) has
        // its root cut by (R_B − R_A) + g/√2 along its tangent line, far
        // past the tolerance, and the other wall's flat lies past that
        // bend's square end by its inner skin's distance from the edge,
        // R_A, exactly (its extension runs along x at y ≤ −R_A).
        let (ra, rb, g) = (1.0 * k, 3.0 * k, 0.3 * k);
        let root = (rb - ra) + g / 2f64.sqrt();
        assert!(root > 1e3 * tol.linear, "scale {k}");
        let clear = clearance_past_end(&part, 1, 0, 0).unwrap();
        near(clear, ra, 1e-9 * k, &format!("scale {k}: the clearance"));
        // A return on that other wall's tip, turned in over the base: it
        // comes back past the bend's end along its tangent line (the
        // measure is negative), but lies wholly above the bend, which
        // reaches `R_B + T` above the base's bottom skin; the return starts
        // at the wall's tip, `R_A + T + L` up (and its flat `R` higher
        // still). Proven apart, built: the volume is the square part's
        // and the return's, its bend `θ W T (R + T/2)` and its flat
        // `W L_r T` along the tip `W` (the tip ends at the corner: the
        // closed corner runs the flat on along its tangent line, not its
        // tip). Before 2026-11-04 refused (one half-space).
        let mut returned = square.clone();
        returned.flanges.push(FlangeSpec {
            on: On::Tip(0),
            ..flange(1, 15.0 * k, FRAC_PI_2, 1.0 * k, 0.4)
        });
        assert!(
            clearance_past_end(&build(&square, &tol).unwrap(), 1, 0, 0).unwrap() > 0.0,
            "scale {k}"
        );
        let with = build(&returned, &tol).unwrap_or_else(|e| panic!("scale {k}: {e}"));
        assert!(check(&with.solid.body, &tol).is_empty(), "scale {k}");
        assert!(
            clearance_past_end(&with, 1, 0, 0).unwrap() < 0.0,
            "scale {k}: the return comes back past the bend's end"
        );
        let (w, lr, rr) = (60.0 * k, 15.0 * k, 1.0 * k);
        let ret = FRAC_PI_2 * w * t * (rr + 0.5 * t) + w * lr * t;
        let v = volume(&part) + ret;
        near(
            volume(&with),
            v,
            1e-8 * v,
            &format!("scale {k}: the return"),
        );
        // Apart for real: the return's lowest point is the wall's tip,
        // above the bend's highest.
        let body = &with.solid.body;
        let mut low = f64::INFINITY;
        for flat in [3usize] {
            for f in craft3d_kernel::sheetmetal::flat_faces(&with, flat) {
                for lp in &body.faces[f].loops {
                    for co in &body.loop_(*lp).unwrap().coedges {
                        let p = body.vertex(body.coedge_ends(*co).unwrap().0).unwrap().point;
                        low = low.min(p.z);
                    }
                }
            }
        }
        near(
            low,
            ra + t + l + rr,
            10.0 * tol.linear,
            &format!("scale {k}: the return's flat"),
        );
        assert!(low > rb + t, "scale {k}");
        // The return as low as the wall allows (the wall 1 long): still
        // apart, since the return lies on the base's side of the bend's
        // end plane `x = W` (the tip ends at the corner) and the bend
        // meets that plane only between the base's skins, `z` in [0, T],
        // while the return starts at the tip, `R_A + T + 1` up. Built, the
        // same closed form.
        let mut low_return = returned.clone();
        low_return.flanges[0].length = 1.0 * k;
        let mut low_square = square.clone();
        low_square.flanges[0].length = 1.0 * k;
        let with = build(&low_return, &tol).unwrap_or_else(|e| panic!("scale {k}: {e}"));
        assert!(check(&with.solid.body, &tol).is_empty(), "scale {k}");
        let v = volume(&build(&low_square, &tol).unwrap()) + ret;
        near(
            volume(&with),
            v,
            1e-8 * v,
            &format!("scale {k}: the low return"),
        );
        // A tab on the wall's side at its open end, turned in: it comes
        // back past the bend's end along the tangent line too, but lies
        // `W` from the bend across it. Built (refused before 2026-11-04).
        let mut far_tab = square.clone();
        far_tab.flanges.push(FlangeSpec {
            on: On::Side(0, 0),
            ..flange(1, 5.0 * k, FRAC_PI_2, 1.0 * k, 0.4)
        });
        let with = build(&far_tab, &tol).unwrap_or_else(|e| panic!("scale {k}: {e}"));
        assert!(check(&with.solid.body, &tol).is_empty(), "scale {k}");
        // A tab on the side at the corner moves with it to the corner's
        // end (`x = W + R_B + T − g/√2` on the wall's bottom skin) and,
        // turned in, its bend (about the vertical axis through that side
        // `R + T` inside the wall's bottom skin, radii 1 and 2) runs into
        // the other wall's extension (between `x = W + R_B` and `W + R_B +
        // T`, on the far side of the mitre plane `x + y = W + R_B − R_A +
        // g/√2`): refused, naming the tab's wall (flange 0): what a wall
        // carries is proven clear of the other wall at every corner
        // (`sheetmetal_corner_carried.rs`).
        let mut near_tab = square.clone();
        near_tab.flanges.push(FlangeSpec {
            on: On::Side(0, 1),
            ..flange(1, 5.0 * k, FRAC_PI_2, 1.0 * k, 0.4)
        });
        let (f, why) = refused(&near_tab, &tol);
        assert_eq!(f, Some(0), "scale {k}: {why}");
        assert!(why.contains("not proven clear"), "scale {k}: {why}");
        // The contact is real: a point inside both (in plan; at any height
        // between `R_B + T`, where the other wall starts, and the tab's top).
        let side_x = 60.0 * k + rb + t - g / 2f64.sqrt();
        let axis = [side_x, -(ra + t) + 1.0 * k + t];
        let q = [side_x + 0.1 * k, -1.5 * k];
        let r = (q[0] - axis[0]).phypot(q[1] - axis[1]);
        assert!(r > 1.0 * k && r < 2.0 * k && q[0] > axis[0] && q[1] < axis[1]);
        assert!(q[0] > 60.0 * k + rb && q[0] < 60.0 * k + rb + t);
        assert!(q[0] + q[1] > 60.0 * k + rb - ra + g / 2f64.sqrt());
        // And by the kernel's point classification: the point, at a height
        // both reach, lies in the part without the tab (in the other
        // wall's extension).
        let w = Vec3::new(q[0], q[1], 8.0 * k);
        assert_eq!(
            craft3d_kernel::boolean::contains_points(&part.solid.body, &[w], &tol).unwrap(),
            vec![Some(true)],
            "scale {k}: the point is in the other wall"
        );
        // The wall of radius 1 leaning in at 120 degrees, the other square
        // with radius 3: the plane cuts the square wall's bend at its root
        // by about 2, and the leaning wall's flat comes back over that
        // bend's end: refused, naming why (before: the plate failed with a
        // reason about its own corners).
        let leaning = spec(
            base,
            t,
            vec![
                flange(1, l, 2.0 * PI / 3.0, 1.0 * k, 0.4),
                flange(2, l, FRAC_PI_2, 3.0 * k, 0.4),
            ],
            corner,
        );
        let (f, why) = refused(&leaning, &tol);
        assert_eq!(f, Some(1), "scale {k}: {why}");
        assert!(
            why.contains("comes back over the bend's end"),
            "scale {k}: {why}"
        );
    }
}
