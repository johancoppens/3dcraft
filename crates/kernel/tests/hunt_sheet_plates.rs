//! A hunt for wrong plates (docs/design/plates.md): flat outlines of
//! lines, arcs and splines, with holes, plated square, at seeded random
//! sizes, attitudes, scales (1e-3 to 1e6), places far from the origin and
//! sides, held to oracles that do not ask how the plate was built:
//!
//! - the volume is the outline's area times the thickness, the area two
//!   areas plus the perimeter times the thickness, with the area and the
//!   perimeter worked out apart from the kernel (Green's theorem on the
//!   exact curves, Gauss–Legendre on the splines' spans);
//! - one face per skin and one rim per piece of the outline;
//! - `topo::check`, the measure, the mesh's volume and that the body does
//!   not pass through itself;
//! - points classified by rays: in the material at mid-level, past each
//!   skin, and a small way off each piece of the outline to either side.
//!
//! The kinds of outline: star polygons, rounded rectangles, slots whose
//! half circles are split into arcs, discs of 2 to 7 arcs of one circle,
//! gears whose root between two teeth is two arcs meeting end to end (a
//! chained ring, as the A-019 flange's), gear sectors, rectangles with a
//! spline top, and a "D" of two arcs of nearly one radius meeting at a
//! crease of a tiny angle (none at all, or 1e-12 to 1e-3 rad, the radii
//! equal or off by 1e-12 to 1e-3), holes of most of these in them.
//!
//! The sweeps that run in the gate take about a minute; the wide ones are
//! `#[ignore]`d and run by hand (`--ignored --nocapture`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod hunt_sheet_common;

use boolean_common::Rng;
use craft3d_kernel::extrude::fill;
use craft3d_kernel::plate::{Side, plate};
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::FaceId;
use hunt_sheet_common::*;
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

const SCALES: [f64; 4] = [1e-3, 1.0, 1.0, 1e6];

/// The kinds of outer outline.
const KINDS: usize = 8;

/// A tiny number, or 0: `±10^−u` for `u` in 3..=12, or exactly 0.
fn tiny(rng: &mut Rng) -> f64 {
    if rng.below(3) == 0 {
        return 0.0;
    }
    let u = rng.between(3.0, 12.0);
    let s = if rng.below(2) == 0 { 1.0 } else { -1.0 };
    s * 10f64.ppowf(-u)
}

/// The least distance from the origin to the segment `a b`.
fn to_segment(a: P2, b: P2) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let s = ((-a[0] * d[0] - a[1] * d[1]) / l2).clamp(0.0, 1.0);
    (a[0] + s * d[0]).phypot(a[1] + s * d[1])
}

/// An outer outline of `kind`, at scale 1, about the origin.
fn outer(rng: &mut Rng, kind: usize) -> Shape {
    let shape = |what: String, outer: Vec<Seg>, inner: (P2, f64), feature: f64| Shape {
        what,
        outer,
        holes: Vec::new(),
        inner,
        hole_discs: Vec::new(),
        feature,
    };
    match kind {
        0 => {
            let n = 3 + rng.below(8) as usize;
            let pts = star(rng, n, 20.0, 40.0);
            let inner = (0..n)
                .map(|i| to_segment(pts[i], pts[(i + 1) % n]))
                .fold(f64::INFINITY, f64::min);
            let shortest = (0..n)
                .map(|i| {
                    let (a, b) = (pts[i], pts[(i + 1) % n]);
                    (b[0] - a[0]).phypot(b[1] - a[1])
                })
                .fold(f64::INFINITY, f64::min);
            shape(
                format!("polygon of {n}"),
                polygon_loop(&pts),
                ([0.0, 0.0], 0.95 * inner),
                inner.min(shortest),
            )
        }
        1 => {
            let w = rng.between(30.0, 80.0);
            let h = rng.between(20.0, 60.0);
            let m = 0.5 * w.min(h);
            let r = if rng.below(4) == 0 {
                // Near the most it can be: lines of almost no length.
                m * (1.0 - 10f64.ppowf(-rng.between(2.0, 5.0)))
            } else {
                rng.between(0.3, m - 0.3)
            };
            shape(
                format!("rounded rectangle {w:.3} x {h:.3} r {r:.6}"),
                rounded_rect(w, h, r),
                ([0.0, 0.0], 0.95 * m),
                m.min(r.max(1.0)),
            )
        }
        2 => {
            let l = rng.between(2.0, 30.0);
            let r = rng.between(3.0, 15.0);
            let split = 1 + rng.below(3) as usize;
            shape(
                format!("slot l {l:.4} r {r:.4} in {split}"),
                slot(l, r, split),
                ([0.0, 0.0], 0.95 * r),
                r,
            )
        }
        3 => {
            let r = rng.between(5.0, 40.0);
            let n = 2 + rng.below(6) as usize;
            let start = rng.between(0.0, TAU);
            let c = cuts(rng, n, start);
            shape(
                format!("disc r {r:.4} of {n} arcs at {c:?}"),
                split_circle([0.0, 0.0], r, &c),
                ([0.0, 0.0], 0.95 * r),
                r,
            )
        }
        4 => {
            let teeth = 6 + rng.below(19) as usize;
            let rr = rng.between(15.0, 40.0);
            let rt = rr + rng.between(1.5, 5.0);
            let tip = rng.between(0.3, 0.6);
            let phase = rng.between(0.0, TAU);
            let pitch = TAU / teeth as f64;
            let feature = (rt - rr)
                .min(rt * pitch * tip)
                .min(rr * pitch * (1.0 - tip));
            shape(
                format!("gear of {teeth} rr {rr:.4} rt {rt:.4} tip {tip:.3} phase {phase:.4}"),
                gear(teeth, rr, rt, tip, phase),
                ([0.0, 0.0], 0.95 * rr),
                feature,
            )
        }
        5 => {
            let teeth = 2 + rng.below(5) as usize;
            let span = rng.between(40.0, 200.0).to_radians();
            let rr = rng.between(15.0, 40.0);
            let rt = rr + rng.between(1.5, 5.0);
            let tip = rng.between(0.3, 0.6);
            let pitch = span / teeth as f64;
            let half = 0.5 * span;
            let inner = if span < PI {
                let d = rr / (1.0 + half.psin());
                (polar([0.0, 0.0], d, half), 0.95 * d * half.psin())
            } else {
                (polar([0.0, 0.0], 0.5 * rr, half), 0.45 * rr)
            };
            let feature = (rt - rr)
                .min(rt * pitch * tip)
                .min(rr * pitch * (1.0 - tip) * 0.5)
                .min(inner.1);
            shape(
                format!(
                    "gear sector of {teeth} over {:.4} deg rr {rr:.4} rt {rt:.4}",
                    span.to_degrees()
                ),
                gear_sector(teeth, span, rr, rt, tip),
                inner,
                feature,
            )
        }
        6 => {
            let w = rng.between(30.0, 80.0);
            let h = rng.between(20.0, 50.0);
            let n = 4 + rng.below(5) as usize;
            let rho = (0.5 * w).min(0.3 * h);
            shape(
                format!("spline top {w:.3} x {h:.3} of {n}"),
                spline_top(rng, w, h, n),
                ([0.0, 0.0], 0.95 * rho),
                rho,
            )
        }
        _ => {
            let r = rng.between(10.0, 40.0);
            let at = rng.between(-40.0, 40.0);
            let eps = tiny(rng);
            let delta = tiny(rng);
            shape(
                format!("crease D r {r:.4} at {at:.4} eps {eps:e} delta {delta:e}"),
                crease_d(r, at, eps, delta),
                ([0.58 * r, 0.0], 0.36 * r),
                0.3 * r,
            )
        }
    }
}

/// A hole of size `s` about `c` (clockwise), its kind drawn.
fn hole(rng: &mut Rng, c: P2, s: f64) -> (Vec<Seg>, String, f64) {
    match rng.below(6) {
        0 => (
            vec![Seg::Circle {
                c,
                r: s,
                ccw: false,
            }],
            format!("circle r {s:.4}"),
            s,
        ),
        1 => {
            let n = 2 + rng.below(6) as usize;
            let start = rng.between(0.0, TAU);
            let cu = cuts(rng, n, start);
            (
                reversed(&split_circle(c, s, &cu)),
                format!("circle r {s:.4} of {n} arcs"),
                s,
            )
        }
        2 => {
            let r = s * rng.between(0.2, 0.5);
            let l = s - r;
            let split = 1 + rng.below(3) as usize;
            (
                reversed(&moved(&slot(l, r, split), c)),
                format!("slot l {l:.4} r {r:.4}"),
                r,
            )
        }
        3 => {
            let w = s * rng.between(0.8, 1.4);
            let h = s * rng.between(0.6, 1.2);
            let w = w.min(1.9 * s);
            let h = h
                .min((4.0 * s * s - w * w).max(0.0).sqrt() * 0.98)
                .max(0.2 * s);
            let r = rng.between(0.05, 0.49) * w.min(h);
            (
                reversed(&moved(&rounded_rect(w, h, r), c)),
                format!("rounded rectangle {w:.4} x {h:.4} r {r:.4}"),
                r.max(0.2 * w.min(h)),
            )
        }
        4 => {
            let n = 3 + rng.below(6) as usize;
            let pts: Vec<P2> = star(rng, n, 0.6 * s, s)
                .iter()
                .map(|p| [p[0] + c[0], p[1] + c[1]])
                .collect();
            (
                reversed(&polygon_loop(&pts)),
                format!("polygon of {n}"),
                0.3 * s,
            )
        }
        _ => {
            let teeth = 6 + rng.below(8) as usize;
            let rr = 0.75 * s;
            let rt = s;
            let phase = rng.between(0.0, TAU);
            let pitch = TAU / teeth as f64;
            (
                reversed(&moved(&gear(teeth, rr, rt, 0.45, phase), c)),
                format!("gear of {teeth}"),
                (rt - rr).min(rr * pitch * 0.5),
            )
        }
    }
}

/// A shape of `kind` with up to two holes in its inner disc.
fn shape(rng: &mut Rng, kind: usize, holes: usize) -> Shape {
    let mut s = outer(rng, kind);
    let (c, rho) = s.inner;
    let dir = rng.between(0.0, TAU);
    let mut what = Vec::new();
    for h in 0..holes.min(2) {
        let (centre, size) = if holes == 1 {
            let off = polar(
                [0.0, 0.0],
                rng.between(0.0, 0.3) * rho,
                rng.between(0.0, TAU),
            );
            ([c[0] + off[0], c[1] + off[1]], rng.between(0.1, 0.4) * rho)
        } else {
            let sign = if h == 0 { 1.0 } else { -1.0 };
            let off = polar([0.0, 0.0], sign * 0.45 * rho, dir);
            (
                [c[0] + off[0], c[1] + off[1]],
                rng.between(0.08, 0.28) * rho,
            )
        };
        let (segs, name, feature) = hole(rng, centre, size);
        s.holes.push(segs);
        s.hole_discs.push((centre, size));
        s.feature = s.feature.min(feature);
        what.push(name);
    }
    if !what.is_empty() {
        s.what = format!("{} with holes [{}]", s.what, what.join(", "));
    }
    s
}

/// One case: `shape` drawn by `placing` in `frame` at scale `placing.k`,
/// plated `t` thick on `side`; its outcome recorded in `tally`.
#[allow(clippy::too_many_arguments)]
fn run_case(
    tally: &mut Tally,
    n: usize,
    rng: &mut Rng,
    shape: &Shape,
    placing: &Placing,
    frame: &Frame,
    t: f64,
    side: Side,
    points: usize,
) {
    let k = placing.k;
    let tol = tolerance(k);
    let what = format!(
        "{} at k {k}, turn {:.17}, at {:?}, t {t:e}, {side:?}, frame {:?}",
        shape.what, placing.turn, placing.at, frame
    );
    let curves = sketch(shape, placing, 1);
    let profiles = match detect_profiles(&curves, &tol) {
        Ok(p) if p.len() == 1 && p[0].holes.len() == shape.holes.len() => p,
        Ok(p) => {
            tally.notes.push(format!(
                "case {n}: {what}: {} profiles, {:?} holes",
                p.len(),
                p.iter().map(|q| q.holes.len()).collect::<Vec<_>>()
            ));
            tally.outcomes.push('S');
            return;
        }
        Err(e) => {
            tally
                .notes
                .push(format!("case {n}: {what}: no profile: {e}"));
            tally.outcomes.push('S');
            return;
        }
    };
    let sheet = match fill(&profiles, frame, &tol) {
        Ok(s) => s.body,
        Err(e) => {
            tally.notes.push(format!("case {n}: {what}: no fill: {e}"));
            tally.outcomes.push('S');
            return;
        }
    };
    let edges = sheet.edges.len();
    if edges != shape.pieces() {
        tally.notes.push(format!(
            "case {n}: the sheet has {edges} edges for {} pieces",
            shape.pieces()
        ));
    }
    let made = guarded(|| plate(&sheet, &[FaceId(0)], t, side, &BTreeMap::new(), &tol));
    let made = match made {
        Err(p) => {
            tally.wrong.push(format!("case {n} (P): {what}: {p}"));
            tally.outcomes.push('P');
            return;
        }
        Ok(Err(e)) => {
            tally.refused(&e.to_string());
            return;
        }
        Ok(Ok(m)) => m,
    };
    let body = &made.body;
    let mut found = Found::default();
    let area = shape.area() * k * k;
    let perimeter = shape.perimeter() * k;
    let (v, a) = (area * t, 2.0 * area + perimeter * t);
    if let Some(m) = sound(body, &tol, &mut found) {
        let free = any_freeform(body) || shape.has_spline();
        let slack_v = m.bound + 1e-9 * v + if free { 0.5 * tol.linear * a } else { 0.0 };
        let slack_a = 1e-9 * a
            + if free {
                2.0 * tol.linear * perimeter
            } else {
                0.0
            };
        if (m.volume - v).abs() > slack_v {
            found.problems.push(format!(
                "volume {:.17e} against the closed form {v:.17e} ({:.3e} of it)",
                m.volume,
                m.volume / v - 1.0
            ));
        }
        if (m.area - a).abs() > slack_a {
            found.problems.push(format!(
                "area {:.17e} against the closed form {a:.17e} ({:.3e} of it)",
                m.area,
                m.area / a - 1.0
            ));
        }
        let faces = body.faces.len();
        if faces != 2 + edges {
            found
                .notes
                .push(format!("{faces} faces for {edges} edges of the outline"));
        }
        let pts = plate_points(rng, shape, placing, frame, side.levels(t), points);
        rays(body, &pts, &tol, &mut found);
    }
    let crosses = crosses_itself(body, &tol);
    tally.record(n, &what, &found, crosses);
}

fn side(rng: &mut Rng, t: f64) -> Side {
    match rng.below(4) {
        0 => Side::Out,
        1 => Side::In,
        2 => Side::Mid,
        _ => {
            let a = rng.between(-1.0, 1.0) * t;
            Side::Levels(a, a + t)
        }
    }
}

/// Seeded random plates: case `n` of `range`.
fn seeded(range: std::ops::Range<usize>, points: usize) -> Tally {
    let mut tally = Tally::new();
    for n in range {
        let mut rng = Rng(case_seed(0x5EED_5EE7, n));
        let kind = rng.below(KINDS as u64) as usize;
        let holes = rng.below(3) as usize;
        let s = shape(&mut rng, kind, holes);
        let k = SCALES[rng.below(SCALES.len() as u64) as usize];
        let far = match rng.below(3) {
            0 => 0.0,
            1 => 100.0,
            _ => 5e4,
        };
        let placing = Placing {
            turn: rng.between(0.0, TAU),
            k,
            at: [rng.between(-far, far) * k, rng.between(-far, far) * k],
            flip_lines: rng.below(2) == 0,
        };
        let f = frame(&mut rng, k);
        let t = 10f64.ppowf(rng.between(-1.3, 1.0)) * k;
        let sd = side(&mut rng, t);
        run_case(&mut tally, n, &mut rng, &s, &placing, &f, t, sd, points);
    }
    tally
}

/// KNOWN WRONG (the rays, not the plate): case 123 of the seeded sweep.
/// A plate of a rounded rectangle 54.667 × 20.261 with corners of radius
/// 10.126 (its short sides 0.0086 long), 6.286 thick about the middle,
/// all times 1e6, drawn turned by 3.1035 rad in a frame turned and some
/// 2.6e8 from the origin. The point 29 of the case, 6600 (as many
/// tolerances) outside the middle of a short side at mid-level, is placed
/// **inside** by `classify_points`; the mesh's winding number about it is
/// 0, and its distance outside the outline worked out here is 6.6e-3 at
/// scale 1. The same plate (with or without its two holes) and point in
/// the world, with the frame's axes at the origin, or with the world's
/// axes at the frame's origin, are placed outside. The plate is right
/// (its volume is the closed form). When the rays are fixed this test
/// fails: take the case off.
#[test]
fn case_123_a_point_beside_the_plate_is_placed_inside_by_the_rays() {
    use craft3d_kernel::boolean::{Place, classify_points};
    let n = 123;
    let mut rng = Rng(case_seed(0x5EED_5EE7, n));
    let kind = rng.below(KINDS as u64) as usize;
    let holes = rng.below(3) as usize;
    let s = shape(&mut rng, kind, holes);
    let k = SCALES[rng.below(SCALES.len() as u64) as usize];
    let far = match rng.below(3) {
        0 => 0.0,
        1 => 100.0,
        _ => 5e4,
    };
    let placing = Placing {
        turn: rng.between(0.0, TAU),
        k,
        at: [rng.between(-far, far) * k, rng.between(-far, far) * k],
        flip_lines: rng.below(2) == 0,
    };
    let f = frame(&mut rng, k);
    let t = 10f64.ppowf(rng.between(-1.3, 1.0)) * k;
    let sd = side(&mut rng, t);
    let pts = {
        let mut r2 = Rng(rng.0);
        plate_points(&mut r2, &s, &placing, &f, sd.levels(t), 6)
    };
    let (p29, inside) = pts[29];
    assert!(!inside);
    // Where the point is, worked out apart from the kernel.
    let Seg::Arc { c, r, .. } = &s.outer[0] else {
        panic!("a rounded rectangle")
    };
    let (half_w, half_h, r) = (c[0] + r, r - c[1], *r);
    let (lx, ly, lz) = f.local(p29);
    let (dx, dy) = (lx - placing.at[0], ly - placing.at[1]);
    let (sn, cs) = (placing.turn.psin(), placing.turn.pcos());
    let (u, v) = ((cs * dx + sn * dy) / k, (-sn * dx + cs * dy) / k);
    let out = ((u.abs() - (half_w - r)).max(0.0)).phypot((v.abs() - (half_h - r)).max(0.0)) - r;
    assert!(
        out > 6e-3 && lz.abs() < 1.0,
        "outside by {out} at level {lz}"
    );
    let mut bare = s.clone();
    bare.holes.clear();
    let tol = tolerance(k);
    let mut seen = Vec::new();
    for (name, sh, fr) in [
        ("as found", &s, f),
        ("without holes", &bare, f),
        ("in the world", &bare, Frame::WORLD),
        (
            "axes at the origin",
            &bare,
            f.at(craft3d_math::Vec3::new(0.0, 0.0, 0.0)).unwrap(),
        ),
        (
            "world axes far",
            &bare,
            Frame::WORLD.at(f.origin()).unwrap(),
        ),
    ] {
        let curves = sketch(sh, &placing, 1);
        let profiles = detect_profiles(&curves, &tol).unwrap();
        let sheet = fill(&profiles, &fr, &tol).unwrap().body;
        let made = plate(&sheet, &[FaceId(0)], t, sd, &BTreeMap::new(), &tol).unwrap();
        let volume = sh.area() * k * k * t;
        let m = craft3d_topo::mass_properties(&made.body).unwrap();
        assert!(
            (m.volume / volume - 1.0).abs() < 1e-9,
            "{name}: the plate is right"
        );
        let p = fr.point(lx, ly, lz);
        let w = mesh_winding(&made.body, 1e-4 * size_of(&made.body), p).unwrap();
        assert!(w.abs() < 1e-6, "{name}: the mesh's winding number {w}");
        seen.push((name, classify_points(&made.body, &[p], &tol).unwrap()[0]));
    }
    assert_eq!(
        seen,
        vec![
            ("as found", Place::Inside),
            ("without holes", Place::Inside),
            ("in the world", Place::Outside),
            ("axes at the origin", Place::Outside),
            ("world axes far", Place::Outside),
        ],
        "known wrong: the rays place the point inside in the frame as found"
    );
}

/// Cases of the seeded sweep known wrong, by number, each with what is
/// wrong (none found).
const KNOWN_SEEDED: &[usize] = &[];

#[test]
fn seeded_plates_are_their_closed_forms() {
    let tally = seeded(0..160, 6);
    tally.hold("seeded plates", 0, KNOWN_SEEDED, 100);
}

/// The wide sweep, by hand: `HUNT_FROM`, `HUNT_TO` (default 160..3160).
#[test]
#[ignore = "a wide sweep, run by hand"]
fn seeded_plates_wide() {
    let from = std::env::var("HUNT_FROM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(160);
    let to = std::env::var("HUNT_TO")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3160);
    let tally = seeded(from..to, 6);
    tally.hold("seeded plates (wide)", from, KNOWN_SEEDED, 0);
}

/// A disc of `n` arcs of one circle (2 to 7), and a plate with a hole of
/// such arcs, turned through `turns` attitudes in its plane (and each
/// time cut at other places) at every scale: the fault of A-019 showed in
/// 30 of 180 placings of the disc.
fn split_disc_attitudes(turns: usize, seed: u64) -> Tally {
    let mut tally = Tally::new();
    let mut n = 0;
    for k in [1e-3, 1.0, 1e6] {
        for arcs in 2..=7 {
            for i in 0..turns {
                let mut rng = Rng(case_seed(seed, n));
                let r = rng.between(5.0, 40.0);
                let c = cuts(&mut rng, arcs, 0.0);
                let holed = i % 2 == 1;
                let s = if holed {
                    let w = 3.0 * r;
                    let mut s = Shape {
                        what: format!("square {w:.4} with a hole r {r:.4} of {arcs} arcs at {c:?}"),
                        outer: polygon_loop(&[[-w, -w], [w, -w], [w, w], [-w, w]]),
                        holes: vec![reversed(&split_circle([0.0, 0.0], r, &c))],
                        inner: ([0.0, 0.0], 2.9 * r),
                        hole_discs: vec![([0.0, 0.0], r)],
                        feature: r,
                    };
                    s.inner = ([0.0, 0.0], 2.9 * r);
                    s
                } else {
                    Shape {
                        what: format!("disc r {r:.4} of {arcs} arcs at {c:?}"),
                        outer: split_circle([0.0, 0.0], r, &c),
                        holes: Vec::new(),
                        inner: ([0.0, 0.0], 0.95 * r),
                        hole_discs: Vec::new(),
                        feature: r,
                    }
                };
                let placing = Placing {
                    turn: TAU * i as f64 / turns as f64 + rng.between(0.0, 0.01),
                    k,
                    at: [rng.between(-50.0, 50.0) * k, rng.between(-50.0, 50.0) * k],
                    flip_lines: false,
                };
                let t = rng.between(0.3, 3.0) * k;
                let f = if i % 3 == 0 {
                    Frame::WORLD
                } else {
                    frame(&mut rng, k)
                };
                run_case(&mut tally, n, &mut rng, &s, &placing, &f, t, Side::Out, 3);
                n += 1;
            }
        }
    }
    tally
}

#[test]
fn split_discs_and_holes_in_many_attitudes_are_their_closed_forms() {
    let tally = split_disc_attitudes(24, 0xD15C);
    tally.hold("split discs in many attitudes", 0, &[], 300);
}

#[test]
#[ignore = "a wide sweep, run by hand"]
fn split_discs_and_holes_in_many_attitudes_wide() {
    let tally = split_disc_attitudes(180, 0xD15C_0001);
    tally.hold("split discs in many attitudes (wide)", 0, &[], 0);
}

/// Outlines turned by steps of 1e-9 rad (in the sketch, and the frame
/// turned about its x axis), `steps` of each: a disc of three arcs, a
/// gear, a slot split into arcs, a rounded rectangle with a hole of arcs.
fn fine_turns(steps: usize) -> Tally {
    let mut tally = Tally::new();
    let mut rng = Rng(0xF1AE);
    let shapes = vec![
        Shape {
            what: "disc r 7 of 3 arcs".into(),
            outer: split_circle([0.0, 0.0], 7.0, &[0.5, 2.6, 4.6]),
            holes: Vec::new(),
            inner: ([0.0, 0.0], 6.6),
            hole_discs: Vec::new(),
            feature: 7.0,
        },
        Shape {
            what: "gear of 12".into(),
            outer: gear(12, 19.0, 22.0, 0.45, 0.3),
            holes: Vec::new(),
            inner: ([0.0, 0.0], 18.0),
            hole_discs: Vec::new(),
            feature: 3.0,
        },
        Shape {
            what: "slot of arcs".into(),
            outer: slot(10.0, 5.0, 3),
            holes: Vec::new(),
            inner: ([0.0, 0.0], 4.7),
            hole_discs: Vec::new(),
            feature: 5.0,
        },
        Shape {
            what: "rounded rectangle with a hole of four arcs".into(),
            outer: rounded_rect(60.0, 40.0, 8.0),
            holes: vec![reversed(&split_circle(
                [0.0, 0.0],
                9.0,
                &[0.1, 1.7, 3.3, 4.8],
            ))],
            inner: ([0.0, 0.0], 19.0),
            hole_discs: vec![([0.0, 0.0], 9.0)],
            feature: 8.0,
        },
    ];
    let mut n = 0;
    for s in &shapes {
        for base in [0.0, std::f64::consts::FRAC_PI_4, 2.0] {
            for i in 0..steps {
                let step = 1e-9 * i as f64;
                let (turn, f) = if i % 2 == 0 {
                    (base + step, Frame::WORLD)
                } else {
                    let f = Frame::new(
                        Vec3::new(0.0, 0.0, 0.0),
                        Vec3::new(0.0, -step.psin(), step.pcos()),
                        Vec3::X,
                    )
                    .unwrap();
                    (base, f)
                };
                let placing = Placing {
                    turn,
                    k: 1.0,
                    at: [0.0, 0.0],
                    flip_lines: false,
                };
                run_case(&mut tally, n, &mut rng, s, &placing, &f, 1.0, Side::Out, 2);
                n += 1;
            }
        }
    }
    tally
}

#[test]
fn outlines_turned_by_billionths_are_their_closed_forms() {
    let tally = fine_turns(30);
    tally.hold("outlines turned by 1e-9 rad", 0, &[], 300);
}

#[test]
#[ignore = "a wide sweep, run by hand"]
fn outlines_turned_by_billionths_wide() {
    let tally = fine_turns(400);
    tally.hold("outlines turned by 1e-9 rad (wide)", 0, &[], 0);
}

/// Two arcs meeting at a crease of the angle `delta`, their radii off by
/// `eps` relative (each 0 or ±1e-12 to ±1e-3), as the outline and as a
/// hole, in several attitudes and at three scales.
fn creases(attitudes: usize) -> Tally {
    let mut tally = Tally::new();
    let tinies = [
        0.0, 1e-12, -1e-12, 1e-10, -1e-9, 1e-8, -1e-7, 1e-6, -1e-5, 1e-4, -1e-3,
    ];
    let mut n = 0;
    for k in [1e-3, 1.0, 1e6] {
        for &eps in &tinies {
            for &delta in &tinies {
                for i in 0..attitudes {
                    let mut rng = Rng(case_seed(0xC4EA5E, n));
                    let r = rng.between(10.0, 40.0);
                    let at = rng.between(-40.0, 40.0);
                    let d = crease_d(r, at, eps, delta);
                    let s = if i % 2 == 0 {
                        Shape {
                            what: format!("crease D r {r} at {at} eps {eps:e} delta {delta:e}"),
                            outer: d,
                            holes: Vec::new(),
                            inner: ([0.58 * r, 0.0], 0.36 * r),
                            hole_discs: Vec::new(),
                            feature: 0.3 * r,
                        }
                    } else {
                        let w = 2.5 * r;
                        Shape {
                            what: format!(
                                "square with a crease D hole r {r} at {at} eps {eps:e} delta {delta:e}"
                            ),
                            outer: polygon_loop(&[[-w, -w], [w, -w], [w, w], [-w, w]]),
                            holes: vec![reversed(&d)],
                            inner: ([-1.6 * r, 0.0], 0.8 * r),
                            hole_discs: vec![([0.0, 0.0], 1.05 * r)],
                            feature: 0.3 * r,
                        }
                    };
                    let placing = Placing {
                        turn: rng.between(0.0, TAU),
                        k,
                        at: [rng.between(-50.0, 50.0) * k, rng.between(-50.0, 50.0) * k],
                        flip_lines: rng.below(2) == 0,
                    };
                    let t = rng.between(0.3, 3.0) * k;
                    run_case(
                        &mut tally,
                        n,
                        &mut rng,
                        &s,
                        &placing,
                        &Frame::WORLD,
                        t,
                        Side::Out,
                        2,
                    );
                    n += 1;
                }
            }
        }
    }
    tally
}

#[test]
fn arcs_at_a_crease_of_a_tiny_angle_are_their_closed_forms() {
    let tally = creases(2);
    tally.hold("arcs at tiny creases", 0, &[], 500);
}

#[test]
#[ignore = "a wide sweep, run by hand"]
fn arcs_at_a_crease_of_a_tiny_angle_wide() {
    let tally = creases(12);
    tally.hold("arcs at tiny creases (wide)", 0, &[], 0);
}
