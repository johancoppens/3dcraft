//! The reread of the fold rule after its search was rebuilt by nearness in
//! space (check/fold-check at cce6aa6c): the rule against an independent
//! scan of the definition (slots, a pleat, turned in space and drawn with
//! weights, at three sizes and two tolerances), the cone of normals of a
//! piece against the normals of its patch, valid faces with many parts
//! near one another against the bound of pairs, and a neck. Lines
//! `REREAD …` (with `-- --nocapture`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::type_complexity,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision
)]

use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Portable, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, Fold, FoldKind, folds, folds_examined, forget_folds};
use std::time::Instant;

fn tol(linear: f64) -> Tolerance {
    Tolerance {
        linear,
        angular: 1e-9,
    }
}

fn clamped(n: usize, degree: usize) -> Vec<f64> {
    let inner = n - degree - 1;
    let mut k = vec![0.0; degree + 1];
    for i in 1..=inner {
        k.push(i as f64 / (inner + 1) as f64);
    }
    k.extend(vec![1.0; degree + 1]);
    k
}

type M = [[f64; 3]; 3];
const ID: M = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
/// The turn of the unit quaternion (1, 2, 3, 4) / √30.
const ROT_A: M = [
    [-2.0 / 3.0, 2.0 / 15.0, 11.0 / 15.0],
    [2.0 / 3.0, -1.0 / 3.0, 2.0 / 3.0],
    [1.0 / 3.0, 14.0 / 15.0, 2.0 / 15.0],
];
/// The turn of the unit quaternion (2, -1, 1, 3) / √15.
const ROT_B: M = [
    [-1.0 / 3.0, -14.0 / 15.0, -2.0 / 15.0],
    [2.0 / 3.0, -1.0 / 3.0, 2.0 / 3.0],
    [-2.0 / 3.0, 2.0 / 15.0, 11.0 / 15.0],
];

fn turn(m: &M, p: Vec3) -> Vec3 {
    Vec3::new(
        m[0][0] * p.x + m[0][1] * p.y + m[0][2] * p.z,
        m[1][0] * p.x + m[1][1] * p.y + m[1][2] * p.z,
        m[2][0] * p.x + m[2][1] * p.y + m[2][2] * p.z,
    )
}

fn segment(a: Vec3, b: Vec3) -> Curve3 {
    Curve3::Nurbs(NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![a, b]).unwrap())
}

/// A sheet of one face: a cubic B-spline profile (control points in x, y,
/// their weights, uniform knots) drawn along z by `depth`, turned by `m`,
/// its outward side the one `towards` (before the turn) points to at the
/// profile's parameter `probe`. Its edges, all on the surface: the profile
/// at both ends of the depth and two segments along it.
fn sheet(
    profile: &[[f64; 2]],
    weights: &[f64],
    depth: f64,
    m: &M,
    probe: f64,
    towards: Vec3,
) -> Body {
    let n = profile.len();
    let knots = clamped(n, 3);
    let at = |p: &[f64; 2], z: f64| turn(m, Vec3::new(p[0], p[1], z));
    let mut net = Vec::with_capacity(2 * n);
    let mut w2 = Vec::with_capacity(2 * n);
    for (p, w) in profile.iter().zip(weights) {
        net.push(at(p, 0.0));
        net.push(at(p, depth));
        w2.push(*w);
        w2.push(*w);
    }
    let surface =
        NurbsSurface::new(3, 1, knots.clone(), vec![0.0, 0.0, 1.0, 1.0], n, 2, net, w2).unwrap();
    let row = |z: f64| -> Curve3 {
        Curve3::Nurbs(
            NurbsCurve::new(
                3,
                knots.clone(),
                profile.iter().map(|p| at(p, z)).collect(),
                weights.to_vec(),
            )
            .unwrap(),
        )
    };
    let s = Surface::Nurbs(surface);
    let mut body = Body::new(BodyKind::Sheet);
    let a0 = body.add_vertex(s.eval(0.0, 0.0));
    let b0 = body.add_vertex(s.eval(1.0, 0.0));
    let a1 = body.add_vertex(s.eval(0.0, 1.0));
    let b1 = body.add_vertex(s.eval(1.0, 1.0));
    let low = body.add_edge(row(0.0), (0.0, 1.0), a0, b0);
    let high = body.add_edge(row(depth), (0.0, 1.0), a1, b1);
    let end = body.add_edge(
        segment(s.eval(1.0, 0.0), s.eval(1.0, 1.0)),
        (0.0, 1.0),
        b0,
        b1,
    );
    let start = body.add_edge(
        segment(s.eval(0.0, 0.0), s.eval(0.0, 1.0)),
        (0.0, 1.0),
        a0,
        a1,
    );
    let lp = body.add_loop(vec![
        Coedge {
            edge: low,
            reversed: false,
        },
        Coedge {
            edge: end,
            reversed: false,
        },
        Coedge {
            edge: high,
            reversed: true,
        },
        Coedge {
            edge: start,
            reversed: true,
        },
    ]);
    let (_, su, sv) = s.derivatives(probe, 0.5);
    let reversed = su.cross(sv).dot(turn(m, towards)) < 0.0;
    let face = body.add_face(s, reversed, vec![lp]);
    body.add_shell(vec![face]);
    body
}

const LOWER: [(f64, f64); 7] = [
    (-0.923_879_532_511_286_7, -0.382_683_432_365_089_8),
    (-0.707_106_781_186_547_6, -0.707_106_781_186_547_6),
    (-0.382_683_432_365_089_8, -0.923_879_532_511_286_7),
    (0.0, -1.0),
    (0.382_683_432_365_089_8, -0.923_879_532_511_286_7),
    (0.707_106_781_186_547_6, -0.707_106_781_186_547_6),
    (0.923_879_532_511_286_7, -0.382_683_432_365_089_8),
];

/// The review's slot: walls `2 r` apart and `l` long, a round bottom.
fn slot(r: f64, l: f64) -> Vec<[f64; 2]> {
    let mut p = vec![[-r, l], [-r, 2.0 * l / 3.0], [-r, l / 3.0], [-r, 0.0]];
    for (c, s) in LOWER {
        p.push([r * c, r * s]);
    }
    p.extend([[r, 0.0], [r, l / 3.0], [r, 2.0 * l / 3.0], [r, l]]);
    p
}

/// The review's pleat: three layers `2 r` apart, two round bends.
fn pleat(r: f64, l: f64) -> Vec<[f64; 2]> {
    let mut p = vec![
        [0.0, 2.0 * r],
        [l / 3.0, 2.0 * r],
        [2.0 * l / 3.0, 2.0 * r],
        [l, 2.0 * r],
    ];
    for (c, s) in LOWER {
        p.push([l - s * r, r - c * r]);
    }
    p.extend([[l, 0.0], [2.0 * l / 3.0, 0.0], [l / 3.0, 0.0], [0.0, 0.0]]);
    for (c, s) in LOWER {
        p.push([s * r, -r - c * r]);
    }
    p.extend([
        [0.0, -2.0 * r],
        [l / 3.0, -2.0 * r],
        [2.0 * l / 3.0, -2.0 * r],
        [l, -2.0 * r],
    ]);
    p
}

/// A comb: `k` slots `w` wide and `h` deep with concave corners of radius
/// `rc` (drawn as quarter circles of control points), fins `f` wide.
fn comb(k: usize, w: f64, rc: f64, h: f64, f: f64) -> Vec<[f64; 2]> {
    let mut p = Vec::new();
    for i in 0..k {
        let x = i as f64 * (w + f);
        p.extend([[x, h], [x, 2.0 * h / 3.0], [x, h / 3.0], [x, rc]]);
        for (c, s) in [LOWER[0], LOWER[1], LOWER[2]] {
            p.push([x + rc + rc * c, rc + rc * s]);
        }
        let flat = w - 2.0 * rc;
        p.extend([
            [x + rc, 0.0],
            [x + rc + flat / 3.0, 0.0],
            [x + rc + 2.0 * flat / 3.0, 0.0],
            [x + w - rc, 0.0],
        ]);
        for (c, s) in [LOWER[4], LOWER[5], LOWER[6]] {
            p.push([x + w - rc + rc * c, rc + rc * s]);
        }
        p.extend([
            [x + w, rc],
            [x + w, h / 3.0],
            [x + w, 2.0 * h / 3.0],
            [x + w, h],
        ]);
        p.extend([[x + w + f / 3.0, h], [x + w + 2.0 * f / 3.0, h]]);
    }
    p
}

/// A hem: a top layer, a round turn of radius `rb` back under it, and a
/// lower layer whose smooth bump rises to within `gap` of the top layer
/// (no bend tighter than about a millimetre); with `notch`, the lower
/// layer ends in a 90° groove with a sharp root (a bend far tighter than
/// the tolerance, away from the bump).
fn hem(rb: f64, gap: f64, notch: bool) -> Vec<[f64; 2]> {
    let top = 2.0 * rb;
    let mut p = vec![[10.0, top], [7.0, top], [4.0, top], [1.0, top]];
    for (c, s) in LOWER {
        p.push([s * rb, rb - c * rb]);
    }
    // A raised control point among flat ones: the cubic peaks at 2/3 of
    // its height.
    let height = 1.5 * (top - gap);
    p.extend([
        [1.0, 0.0],
        [2.0, 0.0],
        [3.0, 0.0],
        [4.0, 0.0],
        [5.0, height],
        [6.0, 0.0],
        [7.0, 0.0],
        [8.0, 0.0],
    ]);
    if notch {
        let e = 0.002;
        p.extend([
            [8.5, 0.0],
            [9.0 - e, -0.5 + e],
            [9.0, -0.5],
            [9.0 + e, -0.5 + e],
            [9.5, 0.0],
        ]);
    }
    p.extend([[10.0, 0.0], [11.0, 0.0]]);
    p
}

/// The least gap of two points of the face at mid-depth that meet the
/// definition (outward normals more than 154° apart, each on the other's
/// outward side), up to `reach`, by a scan of `n + 1` points of the
/// profile and six finer scans about the best: with the surface's own
/// evaluation alone, not the rule's search. (On a sheet drawn along a
/// straight line the least gap of the whole face is at one depth.)
fn least_facing(body: &Body, reach: f64, n: usize) -> Option<(f64, f64, f64)> {
    let face = &body.faces[0];
    let sign = if face.reversed { -1.0 } else { 1.0 };
    let sample = |u: f64| -> Option<(Vec3, Vec3)> {
        let (p, a, b) = face.surface.derivatives(u, 0.5);
        Some((p, a.cross(b).normalized()? * sign))
    };
    let meets = |x: &(Vec3, Vec3), y: &(Vec3, Vec3)| -> Option<f64> {
        let d = y.0 - x.0;
        let gap = d.length();
        (gap < reach && x.1.dot(y.1) < -0.9 && d.dot(x.1) > 0.0 && -d.dot(y.1) > 0.0).then_some(gap)
    };
    let us: Vec<f64> = (0..=n).map(|i| i as f64 / n as f64).collect();
    let pts: Vec<Option<(Vec3, Vec3)>> = us.iter().map(|&u| sample(u)).collect();
    let mut best: Option<(f64, f64, f64)> = None;
    for i in 0..pts.len() {
        let Some(x) = &pts[i] else {
            continue;
        };
        for j in i + 1..pts.len() {
            let Some(y) = &pts[j] else {
                continue;
            };
            if let Some(g) = meets(x, y)
                && best.is_none_or(|b| g < b.0)
            {
                best = Some((g, us[i], us[j]));
            }
        }
    }
    let mut h = 2.0 / n as f64;
    for _ in 0..6 {
        let Some((_, a, b)) = best else {
            break;
        };
        for i in -20..=20 {
            for j in -20..=20 {
                let ua = (a + h * f64::from(i) / 20.0).clamp(0.0, 1.0);
                let ub = (b + h * f64::from(j) / 20.0).clamp(0.0, 1.0);
                if let (Some(x), Some(y)) = (sample(ua), sample(ub))
                    && let Some(g) = meets(&x, &y)
                    && best.is_none_or(|r| g < r.0)
                {
                    best = Some((g, ua, ub));
                }
            }
        }
        h /= 10.0;
    }
    best
}

fn layers(found: &[Fold]) -> Option<f64> {
    found
        .iter()
        .filter_map(|f| match f.kind {
            FoldKind::Layers { gap } => Some(gap),
            _ => None,
        })
        .reduce(f64::min)
}

/// What the rule says of `body` at `t` against the scan: a clear fold
/// (scan below 0.99 t) must be shown, its gap within 2 % of t of the
/// scan's; a clear valid body (no facing pair below 1.01 t) must be said
/// nothing; between them either.
fn judge(name: &str, body: &Body, t: f64, failures: &mut Vec<String>) {
    let scan = least_facing(body, 1.5 * t, 3000);
    forget_folds();
    let start = Instant::now();
    let found = folds(body, &tol(t));
    let spent = start.elapsed().as_secs_f64();
    let (_, work) = folds_examined(body, &tol(t), craft3d_topo::MOST_PAIRS);
    let gap = layers(&found);
    println!(
        "REREAD {name} t {t:e}: scan {:?} t, rule {:?} t, {} lines, {} pairs, {spent:.2} s; {:?}",
        scan.map(|s| s.0 / t),
        gap.map(|g| g / t),
        found.len(),
        work.most_pairs,
        found.iter().map(ToString::to_string).collect::<Vec<_>>()
    );
    match scan {
        Some((g, _, _)) if g < 0.99 * t => match gap {
            None => failures.push(format!(
                "{name} t {t:e}: missed, scan {} t: {found:?}",
                g / t
            )),
            Some(r) if (r - g).abs() > 0.02 * t => failures.push(format!(
                "{name} t {t:e}: gap {} t against the scan's {} t",
                r / t,
                g / t
            )),
            Some(_) => {}
        },
        Some((g, _, _)) if g < 1.01 * t => {}
        _ => {
            if !found.is_empty() {
                failures.push(format!("{name} t {t:e}: a valid face is said: {found:?}"));
            }
        }
    }
}

/// The slots of the review (0.5 t, 0.9 t, 1.1 t, 2 t) and its pleat, at
/// sizes 1e-3, 1 and 1e6 and tolerances of 1e-2 and 1e-6 of the size,
/// each as drawn and turned two ways in space (the boxes and cones then
/// lie askew); and U-shapes drawn with weights (rational spans). The rule
/// must agree with an independent scan of the definition.
#[test]
fn reread_the_rule_agrees_with_an_independent_scan() {
    let mut failures: Vec<String> = Vec::new();
    for k in [1e-3, 1.0, 1e6] {
        for rel in [1e-2, 1e-6] {
            let t = rel * k;
            let l = 10.0 * k;
            for (mi, m) in [ID, ROT_A, ROT_B].iter().enumerate() {
                for (what, r) in [("0.5", 0.25), ("0.9", 0.45), ("1.1", 0.55), ("2", 1.0)] {
                    let profile = slot(r * t, l);
                    let weights = vec![1.0; profile.len()];
                    let body = sheet(&profile, &weights, l, m, 0.05, Vec3::X);
                    judge(
                        &format!("slot {what} t x{k} turn {mi}"),
                        &body,
                        t,
                        &mut failures,
                    );
                }
                let profile = pleat(0.25 * t, l);
                let weights = vec![1.0; profile.len()];
                for towards in [Vec3::new(0.0, -1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)] {
                    let body = sheet(&profile, &weights, l, m, 0.05, towards);
                    judge(
                        &format!("pleat towards {} x{k} turn {mi}", towards.y),
                        &body,
                        t,
                        &mut failures,
                    );
                }
            }
        }
    }
    let mut seed: u64 = 7;
    let mut next = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1_u64 << 53) as f64
    };
    for (k, rel) in [(1.0, 1e-2), (1e6, 1e-6)] {
        let t = rel * k;
        for case in 0..16 {
            let r = [0.3, 0.5, 0.7, 1.0][case % 4] * t;
            let profile = slot(r, 10.0 * k);
            let weights: Vec<f64> = profile.iter().map(|_| 0.3 + 2.7 * next()).collect();
            let body = sheet(&profile, &weights, 10.0 * k, &ROT_B, 0.05, Vec3::X);
            judge(&format!("weighted U {case} x{k}"), &body, t, &mut failures);
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The cone the search gives a piece (`RationalBezierPatch::
/// weighted_normal_net`, its directions' mean and the widest of them, as
/// `topo::fold::cone_of` takes it) must hold every normal of the piece: a
/// normal outside it can let the search pass by a pair of layers. Random
/// rational patches of degrees 1 to 15, weights 0.05 to 20, and pieces of
/// them split again and again; patches with an edge drawn nearly into a
/// point.
#[test]
fn reread_the_normal_cone_of_a_piece_holds_every_normal_of_it() {
    let mut seed: u64 = 11;
    let mut next = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1_u64 << 53) as f64
    };
    let mut checked = 0_usize;
    let mut worst = f64::NEG_INFINITY;
    let mut failures: Vec<String> = Vec::new();
    for case in 0..120 {
        let degree = [1, 2, 3, 3, 4, 5, 7, 9, 12, 15][case % 10];
        let (m, n) = (degree, degree.min(5).max(1));
        let amp = [0.05, 0.3, 1.0][case % 3];
        let squeeze = if case % 7 == 6 { 1e-9 } else { 1.0 };
        let mut net = Vec::new();
        let mut weights = Vec::new();
        for i in 0..=m {
            for j in 0..=n {
                let x = i as f64 / m as f64 + 0.1 * (next() - 0.5);
                let y = (j as f64 / n as f64 + 0.1 * (next() - 0.5))
                    * if i == 0 { squeeze } else { 1.0 };
                net.push(Vec3::new(x, y, amp * (next() - 0.5)));
                weights.push(0.05 * (400.0_f64).ppowf(next()));
            }
        }
        let Ok(whole) = NurbsSurface::new(
            m,
            n,
            clamped(m + 1, m),
            clamped(n + 1, n),
            m + 1,
            n + 1,
            net,
            weights,
        ) else {
            continue;
        };
        let Ok(spans) = whole.bezier_spans_shared() else {
            continue;
        };
        let Some(first) = spans.first() else {
            continue;
        };
        // The patch and pieces of it: halves, quarters, … along u and v.
        let mut pieces = vec![first.patch.clone()];
        let mut grow = first.patch.clone();
        for d in 0..6 {
            let (a, b) = if d % 2 == 0 {
                grow.split_u(0.5)
            } else {
                grow.split_v(0.37)
            };
            pieces.push(a);
            grow = b.clone();
            pieces.push(b);
        }
        for patch in &pieces {
            let Some(cnet) = patch.weighted_normal_net() else {
                continue;
            };
            let units: Vec<Vec3> = cnet.iter().filter_map(|v| v.normalized()).collect();
            let Some(axis) = units.iter().fold(Vec3::ZERO, |a, b| a + *b).normalized() else {
                continue;
            };
            let half = units
                .iter()
                .map(|u| u.dot(axis).clamp(-1.0, 1.0).pacos())
                .fold(0.0_f64, f64::max);
            if !(half < std::f64::consts::FRAC_PI_2) {
                continue;
            }
            let (pm, pn) = patch.degrees();
            let Ok(own) = NurbsSurface::new(
                pm,
                pn,
                clamped(pm + 1, pm),
                clamped(pn + 1, pn),
                pm + 1,
                pn + 1,
                patch.control_points(),
                patch.weights(),
            ) else {
                continue;
            };
            let own = Surface::Nurbs(own);
            checked += 1;
            for i in 0..=40 {
                for j in 0..=40 {
                    let (_, su, sv) = own.derivatives(f64::from(i) / 40.0, f64::from(j) / 40.0);
                    let Some(normal) = su.cross(sv).normalized() else {
                        continue;
                    };
                    let angle = normal.dot(axis).clamp(-1.0, 1.0).pacos();
                    worst = worst.max(angle - half);
                    if angle > half + 1e-9 {
                        failures.push(format!(
                            "case {case} degree {pm}x{pn}: a normal {angle} rad from the axis, the cone {half} rad"
                        ));
                    }
                }
            }
        }
    }
    println!("REREAD cones: {checked} pieces checked, worst excess {worst:e} rad");
    failures.truncate(12);
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Valid faces with many parts near one another facing through air, each
/// searched whole (a sharp concave corner in each slot): combs of 10, 40
/// and 160 slots 1.6 t wide, and the 2 t slot drawn 100 times deeper than
/// its walls are long. They must be said nothing: not a fold, nor too
/// large to examine.
#[test]
fn reread_valid_faces_with_many_near_parts_are_said_nothing() {
    let t = 0.01;
    let mut failures: Vec<String> = Vec::new();
    for (w, counts) in [
        (1.6, vec![10_usize, 40, 160]),
        (1.2, vec![40]),
        (1.05, vec![40]),
    ] {
        // Every slot of a comb is the same: the scan reads a comb of three.
        let profile = comb(3, w * t, 0.3 * t, 0.5, 2.0 * t);
        let weights = vec![1.0; profile.len()];
        let three = sheet(&profile, &weights, 2.0, &ROT_A, 0.01, Vec3::X);
        let scan = least_facing(&three, 1.5 * t, 3000).map(|s| s.0);
        for k in counts {
            let profile = comb(k, w * t, 0.3 * t, 0.5, 2.0 * t);
            let weights = vec![1.0; profile.len()];
            let body = sheet(&profile, &weights, 2.0, &ROT_A, 0.01, Vec3::X);
            forget_folds();
            let start = Instant::now();
            let found = folds(&body, &tol(t));
            let spent = start.elapsed().as_secs_f64();
            let (_, work) = folds_examined(&body, &tol(t), craft3d_topo::MOST_PAIRS);
            if k == 160 {
                // What is said where the bound is reached (here 50 pairs).
                let (small, _) = folds_examined(&body, &tol(t), 50);
                println!(
                    "REREAD comb at a bound of 50 pairs: {:?}",
                    small.iter().map(ToString::to_string).collect::<Vec<_>>()
                );
            }
            println!(
                "REREAD comb of {k} slots {w} t: scan {:?} t, rule {:?} t, {} lines, {} pairs, {spent:.2} s; {:?}",
                scan.map(|g| g / t),
                layers(&found).map(|g| g / t),
                found.len(),
                work.most_pairs,
                found.iter().map(ToString::to_string).collect::<Vec<_>>()
            );
            match scan {
                Some(g) if g < 0.99 * t => {
                    if layers(&found).is_none() {
                        failures.push(format!("comb {k} {w} t: missed, scan {} t", g / t));
                    }
                }
                Some(g) if g < 1.01 * t => {}
                _ => {
                    if !found.is_empty() {
                        failures.push(format!("comb {k} {w} t: a valid face is said: {found:?}"));
                    }
                }
            }
        }
    }
    let profile = slot(t, 10.0);
    let weights = vec![1.0; profile.len()];
    let body = sheet(&profile, &weights, 1000.0, &ROT_B, 0.05, Vec3::X);
    judge("slot 2 t, 1000 deep", &body, t, &mut failures);
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The neck the builder owes: a hem whose lower layer rises to within
/// 0.5 t of its top layer, facing it through air, with no bend tighter
/// than about a millimetre on the face (so no span is a suspect and the
/// face is never searched); and the same hem with a sharp groove at its
/// far end (the face is then searched whole). Printed: what the scan and
/// the rule say of each.
#[test]
fn reread_a_neck_without_and_with_a_sharp_bend() {
    let t = 0.01;
    let mut failures = Vec::new();
    for notch in [false, true] {
        let profile = hem(0.2, 0.5 * t, notch);
        let weights = vec![1.0; profile.len()];
        let body = sheet(
            &profile,
            &weights,
            5.0,
            &ID,
            0.01,
            Vec3::new(0.0, -1.0, 0.0),
        );
        judge(&format!("hem, groove {notch}"), &body, t, &mut failures);
    }
    println!("REREAD neck: {failures:?}");
}
