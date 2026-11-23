//! The second pass of a section's sums, held to the rounding of its
//! coordinates (fault M, 2026-11-22; the corrections after its review,
//! 2026-11-23; docs/design/sections-and-levels.md › *Findings*): loops of
//! edges in a slanted plane, near the origin and far from it, summed by
//! [`edge_sums`] and chosen between by [`held_sums`] as a body's section
//! is, against closed forms.
//!
//! - A conic arc closed by its chord, of weights that spread at most
//!   twofold (and raised to degrees 3 and 6, the same curve), held to the
//!   rounding from 0 to 1e10 times its size from the origin: its area to
//!   the closed form `T g(k)`. Of weights that spread more (3 to 1e16 and
//!   1/3 to 1e-10): never held to the rounding, the strict pass's sums or
//!   its refusal in its words, and never a wrong number (the review found
//!   the rounding pass answering with the whole area missing).
//! - What `held_sums` chooses, by stubs: the strict pass first, the
//!   rounding only after a failure to settle and only where it may, the
//!   body's flag, the words.
//! - A polygon of 70,000 sides, whose panels run out in both passes: the
//!   new words.
//! - A segment of a circle of radius 1e5 and 1e7 about the origin, and a
//!   triangle with an edge on a line through the origin 1e5 to 1e10 away
//!   (the circle's radius and the line's parameter in what a point is made
//!   from), and a polynomial spline of degree 24 1e3 to 1e10 away against
//!   its own strict sums at the origin (a rounding allowed far too widely
//!   takes its first panel with its second moments some hundreds of units
//!   off: `plan/measure-far/round2/wavy.py` in the workshop).
//! - After the second review (2026-11-23): a body's face with an edge on
//!   a conic of wide spread, summed as a body's section is (`faces_sums`
//!   under `held_sums`, not this file's `loop_sums`, which makes the check
//!   itself); and rational splines of spread 2 of degrees 16 to 32, beyond
//!   the conic the bound on its poles was argued for, far away against
//!   their own strict sums at the origin.

use super::*;
use craft3d_geom::Plane;
use craft3d_topo::Coedge;
use std::cell::RefCell;
use std::f64::consts::{FRAC_1_SQRT_2, SQRT_2};

/// One edge of a loop: its curve, its range, whether it runs reversed.
struct Edge {
    curve: Curve3,
    range: (f64, f64),
    reversed: bool,
}

/// The plane every loop lies in, slanted to every world axis.
fn side() -> CutPlane {
    let normal = Vec3::new(0.3, -0.5, 0.8).normalized().unwrap();
    CutPlane::new(normal, normal.cross(Vec3::Z).normalized().unwrap()).unwrap()
}

/// A direction from the origin out of the plane, along which a loop is
/// placed (unit).
const OUT: Vec3 = Vec3::new(0.6, -0.48, 0.64);

/// The point `(u, v)` in the plane's own axes from `c`.
fn at(c: Vec3, u: f64, v: f64) -> Vec3 {
    let s = side();
    c + s.x_axis * u + s.y_axis() * v
}

fn line(a: Vec3, b: Vec3) -> Edge {
    Edge {
        curve: Curve3::Line(craft3d_geom::Line::through(a, b).unwrap()),
        range: (0.0, (b - a).length()),
        reversed: false,
    }
}

/// One Bézier span of the plane's local points `net` from `c`, with
/// `weights`.
fn bezier(c: Vec3, net: &[(f64, f64)], weights: &[f64], reversed: bool) -> Edge {
    let degree = net.len() - 1;
    let mut knots = vec![0.0; degree + 1];
    knots.extend(vec![1.0; degree + 1]);
    let ctrl = net.iter().map(|&(u, v)| at(c, u, v)).collect();
    Edge {
        curve: Curve3::Nurbs(
            craft3d_geom::NurbsCurve::new(degree, knots, ctrl, weights.to_vec()).unwrap(),
        ),
        range: (0.0, 1.0),
        reversed,
    }
}

/// A loop's six sums about `o` held as `held`, with the section's `size`,
/// and the panels spent; held to the rounding, a loop with an edge on a
/// rational spline of wide spread is [`WIDE_SPREAD`] at once, as
/// [`faces_sums`] makes a body's.
fn loop_sums(
    edges: &[Edge],
    o: Vec3,
    size: f64,
    held: Held,
) -> Result<([f64; 6], usize), &'static str> {
    if held == Held::Rounding && edges.iter().any(|e| spreads_widely(&e.curve)) {
        return Err(WIDE_SPREAD);
    }
    let rule = (Rule::new(8), Rule::new(16));
    let mut budget = MAX_PANELS;
    let mut sums = [0.0; 6];
    for e in edges {
        let along = edge_sums(
            &e.curve,
            e.range,
            &side(),
            (o, size),
            &rule,
            &mut budget,
            held,
        )?;
        let sign = if e.reversed { -1.0 } else { 1.0 };
        for (s, p) in sums.iter_mut().zip(along) {
            *s += sign * p;
        }
    }
    Ok((sums, MAX_PANELS - budget))
}

/// A loop's sums as a body's section in a call takes them at its first
/// offset ([`held_sums`], its flag not yet set).
fn section_sums(edges: &[Edge], o: Vec3, size: f64) -> Result<[f64; 6], &'static str> {
    held_sums(
        |held| loop_sums(edges, o, size, held).map(|(sums, _)| sums),
        true,
        Some(&Cell::new(false)),
    )
}

// ------------------------------------------------------------- the conic

/// The area between a conic arc and its chord as a share of the triangle
/// of its three control points, for the shape `k = w₁ / √(w₀ w₂)` of its
/// weights (an ellipse's arc below 1, a parabola's at 1, a hyperbola's
/// above): `k/(1 − k²) (acos k / √(1 − k²) − k)`, `2/3`, `k/(k² − 1) (k −
/// acosh k / √(k² − 1))`.
fn conic_share(k: f64) -> f64 {
    if (k - 1.0).abs() < 1e-12 {
        return 2.0 / 3.0;
    }
    if k < 1.0 {
        let q = 1.0 - k * k;
        k / q * (k.pacos() / q.sqrt() - k)
    } else {
        let q = k * k - 1.0;
        k / q * (k - k.pacosh() / q.sqrt())
    }
}

/// The same rational curve one degree higher: its weighted points and its
/// weights blended, `Qᵢ' = i/(n+1) Qᵢ₋₁ + (1 − i/(n+1)) Qᵢ`.
fn elevated(net: &[(f64, f64)], weights: &[f64]) -> (Vec<(f64, f64)>, Vec<f64>) {
    let q: Vec<[f64; 3]> = net
        .iter()
        .zip(weights)
        .map(|(&(u, v), &w)| [w * u, w * v, w])
        .collect();
    let m = q.len() as f64;
    let mut out = vec![q[0]];
    for (i, pair) in q.windows(2).enumerate() {
        let a = (i + 1) as f64 / m;
        out.push([0, 1, 2].map(|k| a * pair[0][k] + (1.0 - a) * pair[1][k]));
    }
    out.push(q[q.len() - 1]);
    (
        out.iter().map(|h| (h[0] / h[2], h[1] / h[2])).collect(),
        out.iter().map(|h| h[2]).collect(),
    )
}

/// The conic of `net` and `weights` over the chord `s` long, `h` below its
/// shoulder, about `c`: the chord along the bottom, then the arc back over
/// it (counter-clockwise about the plane's normal).
fn conic_loop(c: Vec3, s: f64, h: f64, net: &[(f64, f64)], weights: &[f64]) -> Vec<Edge> {
    vec![
        line(at(c, -0.5 * s, -0.5 * h), at(c, 0.5 * s, -0.5 * h)),
        bezier(c, net, weights, true),
    ]
}

/// The quadratic net of [`conic_loop`].
fn conic_net(s: f64, h: f64) -> Vec<(f64, f64)> {
    vec![(-0.5 * s, -0.5 * h), (0.0, 0.5 * h), (0.5 * s, -0.5 * h)]
}

/// Conics whose weights spread at most twofold, held to the rounding of
/// their coordinates (the second pass alone, as a body's later sections in
/// a call are summed), from 0 to 1e10 times their size from the origin, at
/// three scales: the area within 64 units of `ε` times the distance along
/// the perimeter (and 1e-12 of the size squared) of the closed form `T
/// g(k)`, and the first moment across a symmetric one zero to as much
/// times its reach. Weights 1, w, 1 for w from ½ to 2 (a circle's arc at
/// 1/√2, the boolean's hyperbola arcs at most 1.13), and 1, 1, 2, 2, 2, 1
/// and 1, 2, 1.5 (spread 2 with the ends weighted); the first and the
/// last raised to degrees 3 and 6 (the same curve, the same area).
#[test]
fn a_conic_of_little_spread_is_held_to_its_rounding_to_its_closed_form() {
    // The weights, whether they are raised to degrees 3 and 6, and whether
    // the arc is symmetric (its first moment across zero).
    let weights: [([f64; 3], bool, bool); 8] = [
        ([1.0, 0.5, 1.0], true, true),
        ([1.0, FRAC_1_SQRT_2, 1.0], false, true),
        ([1.0, 1.13, 1.0], false, true),
        ([1.0, 1.5, 1.0], false, true),
        ([1.0, 2.0, 1.0], true, true),
        ([1.0, 1.0, 2.0], false, false),
        ([2.0, 2.0, 1.0], false, false),
        ([1.0, 2.0, 1.5], false, false),
    ];
    let mut worst = 0.0_f64;
    for scale in [1e-3, 1.0, 1e6] {
        let (s, h) = (scale, 0.5 * scale);
        let size = 0.5 * s.phypot(h);
        let perimeter = s + 2.0 * (0.5 * s).phypot(h);
        for (w, raise, symmetric) in weights {
            let exact = 0.5 * s * h * conic_share(w[1] / (w[0] * w[2]).sqrt());
            let mut nets = vec![(conic_net(s, h), w.to_vec())];
            if raise {
                let mut net = (conic_net(s, h), w.to_vec());
                for degree in 3..=6 {
                    net = elevated(&net.0, &net.1);
                    if degree == 3 || degree == 6 {
                        nets.push(net.clone());
                    }
                }
            }
            for (net, ws) in &nets {
                for ratio in [0.0, 10.0, 1e3, 1e4, 1e5, 1e7, 1e10] {
                    let what = format!(
                        "weights {w:?} degree {} scale {scale:e} at {ratio:e} sizes",
                        net.len() - 1
                    );
                    let c = OUT * (ratio * size);
                    let edges = conic_loop(c, s, h, net, ws);
                    let (sums, panels) = loop_sums(&edges, c, size, Held::Rounding)
                        .unwrap_or_else(|e| panic!("{what}: {e}"));
                    let unit = f64::EPSILON * (ratio * size).max(size) * perimeter;
                    let off = (sums[0] - exact).abs();
                    worst = worst.max(off / unit);
                    println!(
                        "CONIC {what}: area {:.17e} exact {exact:.17e} {panels} panels, {:.3} units",
                        sums[0],
                        off / unit
                    );
                    assert!(
                        off <= 1e-12 * size * size + 64.0 * unit,
                        "{what}: area {:e}, want {exact:e} ({} units off)",
                        sums[0],
                        off / unit
                    );
                    if symmetric {
                        assert!(
                            sums[1].abs() <= 1e-12 * size.ppowi(3) + 64.0 * unit * s,
                            "{what}: the first moment across {:e}",
                            sums[1]
                        );
                    }
                }
            }
        }
    }
    println!("CONIC worst {worst:.3} units");
}

/// Conics whose weights spread more than twofold, 1, w, 1 for w from 3 to
/// 1e16 and from 1/3 to 1e-10, at 0, 1e3, 1e5 and 1e7 times their size
/// from the origin: never held to the rounding (`WIDE_SPREAD` at once), so
/// a section of one is the strict pass's to the bit, its sums or its
/// refusal in its words, at its first offset and at the later ones of a
/// call already held to the rounding; and never a wrong number, refused
/// or within 64 units (and 1e-12 of the size squared) of the closed form.
/// The rounding pass alone answered w = 1e10 at the origin, 1e8 at 1e3
/// and 1e6 at 1e5 (and wider) with the whole area missing, where both
/// rules miss the curve's turn alike (the review of 2026-11-22).
#[test]
fn a_conic_of_wide_spread_keeps_the_strict_answer_or_refusal() {
    let (s, h) = (1.0, 0.5);
    let size = 0.5 * s.phypot(h);
    let perimeter = s + 2.0 * (0.5 * s).phypot(h);
    let bits = |found: Result<[f64; 6], &'static str>| found.map(|sums| sums.map(f64::to_bits));
    for w in [
        3.0,
        1e2,
        1e4,
        1e6,
        1e8,
        1e10,
        1e12,
        1e16,
        1.0 / 3.0,
        1e-3,
        1e-6,
        1e-10,
    ] {
        let exact = 0.5 * s * h * conic_share(w);
        for ratio in [0.0, 1e3, 1e5, 1e7] {
            let what = format!("w {w:e} at {ratio:e} sizes");
            let c = OUT * (ratio * size);
            let edges = conic_loop(c, s, h, &conic_net(s, h), &[1.0, w, 1.0]);
            assert_eq!(
                loop_sums(&edges, c, size, Held::Rounding).err(),
                Some(WIDE_SPREAD),
                "{what}"
            );
            let strict = loop_sums(&edges, c, size, Held::Strict).map(|(sums, _)| sums);
            let first = section_sums(&edges, c, size);
            let later = held_sums(
                |held| loop_sums(&edges, c, size, held).map(|(sums, _)| sums),
                true,
                Some(&Cell::new(true)),
            );
            println!("WIDE {what}: exact {exact:.17e} strict {strict:?}");
            assert_eq!(bits(first), bits(strict), "{what}: the first offset");
            assert_eq!(bits(later), bits(strict), "{what}: a later offset");
            match strict {
                Ok(sums) => {
                    let unit = f64::EPSILON * (ratio * size).max(size) * perimeter;
                    assert!(
                        (sums[0] - exact).abs() <= 1e-12 * size * size + 64.0 * unit,
                        "{what}: area {:e}, want {exact:e}",
                        sums[0]
                    );
                }
                Err(e) => assert!(unsettled(e), "{what}: {e}"),
            }
        }
    }
}

/// The conic of wide spread as a body's face in the plane (its chord and
/// its arc, one loop), summed as `sums_about` sums a body's section:
/// [`faces_sums`] under [`held_sums`]. Held to the rounding the face is
/// [`WIDE_SPREAD`] before any panel, and its sums at the first offset and
/// with the body's flag set are the strict pass's to the bit, its sums or
/// its refusal in its words. `loop_sums` above makes the spread check
/// itself, so without this no test held a body's section to it (the
/// second review of 2026-11-23: the check in `faces_sums` and the one in
/// `Reach::of` both taken out, every other test of this file passed,
/// Check 45235939588).
#[test]
fn a_face_with_an_edge_of_wide_spread_keeps_the_strict_answer() {
    let side = side();
    let (s, h) = (1.0, 0.5);
    let size = 0.5 * s.phypot(h);
    let rule = (Rule::new(8), Rule::new(16));
    let bits = |found: Result<Vec<[f64; 6]>, &'static str>| {
        found.map(|faces| {
            faces
                .iter()
                .map(|sums| sums.map(f64::to_bits))
                .collect::<Vec<_>>()
        })
    };
    for w in [3.0, 1e4, 1e10, 1.0 / 3.0] {
        for ratio in [0.0, 1e3, 1e5] {
            let what = format!("w {w:e} at {ratio:e} sizes");
            let c = OUT * (ratio * size);
            let net = conic_net(s, h);
            let (a, b) = (at(c, net[0].0, net[0].1), at(c, net[2].0, net[2].1));
            let chord = line(a, b);
            let arc = bezier(c, &net, &[1.0, w, 1.0], false);
            let mut body = Body::new(BodyKind::Sheet);
            let (va, vb) = (body.add_vertex(a), body.add_vertex(b));
            let along = body.add_edge(chord.curve, chord.range, va, vb);
            let back = body.add_edge(arc.curve, arc.range, va, vb);
            let lp = body.add_loop(vec![
                Coedge {
                    edge: along,
                    reversed: false,
                },
                Coedge {
                    edge: back,
                    reversed: true,
                },
            ]);
            let frame = Frame::new(c, side.normal, side.x_axis).unwrap();
            body.add_face(Surface::Plane(Plane::new(frame)), false, vec![lp]);
            let sum = |held: Held| faces_sums(&body, &[0], &side, (c, size), &rule, held);
            assert_eq!(sum(Held::Rounding).err(), Some(WIDE_SPREAD), "{what}");
            let strict = bits(sum(Held::Strict));
            println!("WIDE FACE {what}: strict {strict:?}");
            for flagged in [false, true] {
                let found = held_sums(sum, true, Some(&Cell::new(flagged)));
                assert_eq!(bits(found), strict, "{what}, flagged {flagged}");
            }
        }
    }
}

// ----------------------------------------------------------- the choice

/// What [`held_sums`] does with each answer of the two passes, by a stub
/// that says what each pass gives and records the passes asked for: the
/// strict pass first; any refusal of it but the two failures to settle,
/// and every one where the rounding may not be used, the answer as it was
/// (and the body's flag left alone, so the call's later sections are
/// summed strictly first again); a failure to settle summed again with the
/// rounding, the flag set; with the flag set the rounding at once; a
/// rounding pass that does not settle either in the words that say so;
/// one that meets a rational spline of wide spread, the strict pass's
/// answer or refusal in its words.
#[test]
fn held_sums_takes_the_rounding_only_after_a_failure_to_settle() {
    use Held::{Rounding, Strict};
    const OTHER: &str = "a face in the plane whose loops enclose no area";
    type Outcome = Result<u32, &'static str>;
    let choose = |strict: Outcome, rounding: Outcome, may_round: bool, flagged: bool| {
        let asked = RefCell::new(Vec::new());
        let flag = Cell::new(flagged);
        let found = held_sums(
            |held| {
                asked.borrow_mut().push(held);
                match held {
                    Held::Strict => strict,
                    Held::Rounding => rounding,
                }
            },
            may_round,
            Some(&flag),
        );
        (found, asked.into_inner(), flag.get())
    };
    // Settled strictly: the strict sums, nothing else asked.
    assert_eq!(
        choose(Ok(1), Ok(2), true, false),
        (Ok(1), vec![Strict], false)
    );
    // Another refusal: as it was, the flag left alone.
    assert_eq!(
        choose(Err(OTHER), Ok(2), true, false),
        (Err(OTHER), vec![Strict], false)
    );
    // Not settled: summed again with the rounding, the flag set.
    for failure in [TOO_MANY_PANELS, NOT_SETTLED] {
        assert_eq!(
            choose(Err(failure), Ok(2), true, false),
            (Ok(2), vec![Strict, Rounding], true)
        );
        // Where the rounding may not be used: the strict refusal, in its
        // words, flagged or not.
        for flagged in [false, true] {
            assert_eq!(
                choose(Err(failure), Ok(2), false, flagged),
                (Err(failure), vec![Strict], flagged)
            );
        }
        // Not settled either way: the words that say so.
        for again in [TOO_MANY_PANELS, NOT_SETTLED] {
            assert_eq!(
                choose(Err(failure), Err(again), true, false),
                (Err(NOT_SETTLED_ROUNDED), vec![Strict, Rounding], true)
            );
        }
        // A rational spline of wide spread: the strict refusal's words.
        assert_eq!(
            choose(Err(failure), Err(WIDE_SPREAD), true, false),
            (Err(failure), vec![Strict, Rounding], true)
        );
    }
    // Flagged: the rounding at once; a rational spline of wide spread
    // then summed strictly after all.
    assert_eq!(
        choose(Ok(1), Ok(2), true, true),
        (Ok(2), vec![Rounding], true)
    );
    assert_eq!(
        choose(Ok(1), Err(WIDE_SPREAD), true, true),
        (Ok(1), vec![Rounding, Strict], true)
    );
    assert_eq!(
        choose(Err(NOT_SETTLED), Err(WIDE_SPREAD), true, true),
        (Err(NOT_SETTLED), vec![Rounding, Strict], true)
    );
    // Another refusal of the rounding pass: as it is.
    assert_eq!(
        choose(Ok(1), Err(OTHER), true, true),
        (Err(OTHER), vec![Rounding], true)
    );
    assert!(NOT_SETTLED_ROUNDED.contains("held only to the rounding of their coordinates"));
}

/// A polygon of 70,000 sides about the origin: its panels, one a side, run
/// out in the strict pass ("its edges take more panels than a section
/// may") and held to the rounding as well, so the section is refused in
/// the words that say so; where the rounding may not be used, in the
/// strict pass's.
#[test]
fn a_section_whose_panels_run_out_held_to_the_rounding_says_so() {
    let n = 70_000;
    let corners: Vec<Vec3> = (0..n)
        .map(|i| {
            let (s, c) = (2.0 * PI * i as f64 / n as f64).psin_cos();
            at(Vec3::ZERO, c, s)
        })
        .collect();
    let edges: Vec<Edge> = (0..n)
        .map(|i| line(corners[i], corners[(i + 1) % n]))
        .collect();
    let size = SQRT_2;
    assert_eq!(
        loop_sums(&edges, Vec3::ZERO, size, Held::Strict).err(),
        Some(TOO_MANY_PANELS)
    );
    assert_eq!(
        section_sums(&edges, Vec3::ZERO, size).err(),
        Some(NOT_SETTLED_ROUNDED)
    );
    assert_eq!(
        held_sums(
            |held| loop_sums(&edges, Vec3::ZERO, size, held).map(|(sums, _)| sums),
            false,
            Some(&Cell::new(false)),
        )
        .err(),
        Some(TOO_MANY_PANELS)
    );
}

// --------------------------------------- what a point is made from

/// A segment of a circle of radius R about the world origin, an arc of
/// 1/R radians closed by its chord (1 long): its points are made from the
/// radius, so they carry its rounding, as a far circle's carry its
/// centre's. Its area R²/2 (x − sin x), x = 1/R, within 64 units of `ε R`
/// along its perimeter (2) at R = 1e3 (summed strictly), 1e5 and 1e7
/// (refused strictly: "its edges take more panels than a section may",
/// "… do not settle"; held to the rounding on two panels). Without the
/// radius in what a circle's point is made from, the rounding pass is
/// refused at 1e5 and 1e7 (the review's mutant 6).
#[test]
fn a_segment_of_a_large_circle_about_the_origin_is_measured() {
    let side = side();
    let frame = Frame::new(Vec3::ZERO, side.normal, side.x_axis).unwrap();
    for radius in [1e3_f64, 1e5, 1e7] {
        let theta = 0.5 / radius;
        let circle = Curve3::Circle(craft3d_geom::Circle::new(frame, radius).unwrap());
        let (a, b) = (circle.eval(theta), circle.eval(-theta));
        let edges = [
            Edge {
                curve: circle,
                range: (-theta, theta),
                reversed: false,
            },
            line(a, b),
        ];
        let o = side.x_axis * radius;
        let x = 2.0 * theta;
        let exact =
            0.5 * radius * radius * (x.ppowi(3) / 6.0 - x.ppowi(5) / 120.0 + x.ppowi(7) / 5040.0);
        let unit = f64::EPSILON * radius * 2.0;
        let strict = loop_sums(&edges, o, 0.5, Held::Strict);
        let area = section_sums(&edges, o, 0.5).unwrap_or_else(|e| panic!("R {radius:e}: {e}"))[0];
        println!(
            "SEGMENT R {radius:e}: area {area:.17e} exact {exact:.17e} ({:.3} units), strictly {:?}",
            (area - exact).abs() / unit,
            strict.map(|(sums, panels)| (sums[0], panels))
        );
        assert!(
            (area - exact).abs() <= 64.0 * unit,
            "R {radius:e}: area {area:e}, want {exact:e}"
        );
        assert_eq!(strict.is_err(), radius >= 1e5, "R {radius:e}: {strict:?}");
    }
}

/// A right triangle of side 1 whose first edge lies on a line through the
/// world origin (its origin there, its parameter the distance along it),
/// 1e5, 1e7 and 1e10 from the origin: that edge's points are made from its
/// parameter times its direction, so they carry the parameter's rounding.
/// Refused strictly; held to the rounding its area ½ within 64 units of
/// `ε` times the distance along its perimeter. Without the parameter in
/// what a point is made from, the rounding pass is refused (the review's
/// mutant 12).
#[test]
fn a_triangle_on_a_line_through_the_origin_is_measured() {
    let side = side();
    let size = 0.5 * SQRT_2;
    for far in [1e5, 1e7, 1e10] {
        let c = side.x_axis * far;
        let d = c.dot(side.x_axis);
        let (a, b, p) = (at(c, 0.0, 0.0), at(c, 1.0, 0.0), at(c, 0.0, 1.0));
        let edges = [
            Edge {
                curve: Curve3::Line(craft3d_geom::Line::new(Vec3::ZERO, side.x_axis).unwrap()),
                range: (d, d + 1.0),
                reversed: false,
            },
            line(b, p),
            line(p, a),
        ];
        let unit = f64::EPSILON * far * (2.0 + SQRT_2);
        let strict = loop_sums(&edges, c, size, Held::Strict);
        let area = section_sums(&edges, c, size).unwrap_or_else(|e| panic!("at {far:e}: {e}"))[0];
        println!(
            "RAY at {far:e}: area {area:.17e} ({:.3} units), strictly {:?}",
            (area - 0.5).abs() / unit,
            strict.map(|(sums, panels)| (sums[0], panels))
        );
        assert!(strict.is_err(), "at {far:e}: settled strictly");
        assert!(
            (area - 0.5).abs() <= 64.0 * unit,
            "at {far:e}: area {area:e}"
        );
    }
}

/// A polynomial spline of degree 24 (one span, its net zigzagging across
/// both axes) closed by its chord, 1e3 to 1e10 from the origin, against
/// its own strict sums at the origin: every integral within 64 units of
/// `ε` times the distance along its perimeter (under 3) times its reach
/// (0.75) to the integral's power less two, and 1e-12 of its size to the
/// power. Its integrands are polynomials of degree up to 95, which neither
/// rule sums exactly on its one span; held to 16 units in the last place
/// its panels are halved to some 16, within half a unit in a model of the
/// sums in doubles (`plan/measure-far/round2/wavy.py` in the workshop);
/// with 1e6 units allowed, its first panel is taken 1e7 and 3e7 away with
/// ∫ab some 450 and 150 units off and ∫b² 200 and 65 (the review's
/// mutant 16).
#[test]
fn a_spline_of_high_degree_far_away_is_its_own_sums_at_the_origin() {
    let n = 24;
    let net: Vec<(f64, f64)> = (0..=n)
        .map(|i| {
            let mut u = -0.5 + i as f64 / n as f64;
            let v = if i == 0 || i == n {
                -0.3
            } else if i % 2 == 1 {
                u += 0.35 / n as f64;
                0.55
            } else {
                u -= 0.35 / n as f64;
                0.05
            };
            (u, v)
        })
        .collect();
    let ones = vec![1.0; n + 1];
    let size = 0.5 * 1.0_f64.phypot(0.9);
    let wavy = |c: Vec3| {
        vec![
            line(at(c, -0.5, -0.3), at(c, 0.5, -0.3)),
            bezier(c, &net, &ones, true),
        ]
    };
    let (reference, _) = loop_sums(&wavy(Vec3::ZERO), Vec3::ZERO, size, Held::Strict).unwrap();
    for far in [1e3, 1e5, 1e7, 3e7, 1e10] {
        let c = OUT * far;
        let sums = section_sums(&wavy(c), c, size).unwrap_or_else(|e| panic!("at {far:e}: {e}"));
        // Each integral's gap from the origin's, its unit and its power.
        let gaps: Vec<(f64, f64, i32)> = sums
            .iter()
            .zip(&reference)
            .zip(POWERS)
            .map(|((got, want), power)| {
                let unit = f64::EPSILON * far * 3.0 * 0.75_f64.ppowi(power - 2);
                ((got - want).abs(), unit, power)
            })
            .collect();
        let units: Vec<f64> = gaps.iter().map(|(gap, unit, _)| gap / unit).collect();
        println!("WAVY at {far:e}: {sums:?} against {reference:?}, units {units:.3?}");
        for (k, (gap, unit, power)) in gaps.into_iter().enumerate() {
            assert!(
                gap <= 1e-12 * size.ppowi(power) + 64.0 * unit,
                "at {far:e}: integral {k} {:e}, at the origin {:e} ({:.1} units)",
                sums[k],
                reference[k],
                gap / unit
            );
        }
    }
}

/// Rational splines whose weights spread twofold, of degrees beyond the
/// conic's (the second review of 2026-11-23): one span of degree 16, 24
/// and 32, its weights 1, 2, 1, 2, … (a denominator whose zeros come
/// nearest the span for that spread: outside the ellipse |t| + |1 − t| =
/// 3^(1/n), `ρ` 1.30 at degree 32 against the conic's 3.15) or 1, 2, …, 2,
/// 1, on a zigzag net and on a hairpin net, closed by its chord, held to
/// the rounding 1e3 to 1e10 times its size from the origin: every integral
/// within 64 units of `ε` times the distance along its perimeter (its
/// control polygon and chord) times its size to the integral's power less
/// two of its own strict sums at the origin. At these degrees the 16-node
/// sum over the whole span is not always nearer the integral than the
/// 8-node one (its error up to 2.5 times the 8-node one's at degree 24),
/// so the pair does not tell the first panel's error as it does a
/// conic's, and the error taken grows with the allowance: held to 16 units
/// in the last place the panels are halved to 8 to 28 and the worst is
/// 0.25 units (over the review's 1,620 such sums of degrees 2 to 32, 0.48
/// out to 1e10 and 0.75 at 1e13); held to 64 units in the last place 1.3, to 1e3 12.6, and
/// to 1e4 degree 24's area 72 units (2e-3 of itself) off 1e10 away, which
/// this test refuses (Check 45235939588).
#[test]
fn a_rational_spline_of_spread_two_and_high_degree_far_away_is_its_own_sums_at_the_origin() {
    let nets = |n: usize| -> [(&'static str, Vec<(f64, f64)>); 2] {
        let nf = n as f64;
        let zigzag: Vec<(f64, f64)> = (0..=n)
            .map(|i| {
                let mut u = -0.5 + i as f64 / nf;
                let v = if i == 0 || i == n {
                    -0.3
                } else if i % 2 == 1 {
                    u += 0.35 / nf;
                    0.55
                } else {
                    u -= 0.35 / nf;
                    0.05
                };
                (u, v)
            })
            .collect();
        let hairpin: Vec<(f64, f64)> = (0..=n)
            .map(|i| {
                let s = i as f64 / nf;
                if s <= 0.5 {
                    (-0.5 + 2.0 * s, -0.05 - 0.8 * s)
                } else {
                    (1.5 - 2.0 * s, 0.85 - 0.8 * s)
                }
            })
            .collect();
        [("zigzag", zigzag), ("hairpin", hairpin)]
    };
    let mut worst = 0.0_f64;
    for n in [16, 24, 32] {
        let alternating: Vec<f64> = (0..=n)
            .map(|i| if i % 2 == 0 { 1.0 } else { 2.0 })
            .collect();
        let mut middle = vec![2.0; n + 1];
        middle[0] = 1.0;
        middle[n] = 1.0;
        for (name, net) in nets(n) {
            let (mut lo, mut hi) = (
                (f64::INFINITY, f64::INFINITY),
                (-f64::INFINITY, -f64::INFINITY),
            );
            for &(u, v) in &net {
                lo = (lo.0.min(u), lo.1.min(v));
                hi = (hi.0.max(u), hi.1.max(v));
            }
            let size = 0.5 * (hi.0 - lo.0).phypot(hi.1 - lo.1);
            let (first, last) = (net[0], net[n]);
            let perimeter = net
                .windows(2)
                .map(|w| (w[1].0 - w[0].0).phypot(w[1].1 - w[0].1))
                .sum::<f64>()
                + (last.0 - first.0).phypot(last.1 - first.1);
            for (weighting, weights) in [("1, 2, 1, …", &alternating), ("1, 2, …, 2, 1", &middle)]
            {
                let closed = |c: Vec3| {
                    vec![
                        bezier(c, &net, weights, false),
                        line(at(c, last.0, last.1), at(c, first.0, first.1)),
                    ]
                };
                let (reference, _) =
                    loop_sums(&closed(Vec3::ZERO), Vec3::ZERO, size, Held::Strict).unwrap();
                for far in [1e3, 1e5, 1e7, 1e10] {
                    let what = format!("degree {n} {name} weights {weighting} at {far:e} sizes");
                    let c = OUT * (far * size);
                    let (sums, panels) = loop_sums(&closed(c), c, size, Held::Rounding)
                        .unwrap_or_else(|e| panic!("{what}: {e}"));
                    let mut most = 0.0_f64;
                    for (k, power) in POWERS.into_iter().enumerate() {
                        let unit = f64::EPSILON * far * size * perimeter * size.ppowi(power - 2);
                        let gap = (sums[k] - reference[k]).abs();
                        most = most.max(gap / unit);
                        assert!(
                            gap <= 1e-12 * size.ppowi(power) + 64.0 * unit,
                            "{what}: integral {k} {:e}, at the origin {:e} ({:.1} units)",
                            sums[k],
                            reference[k],
                            gap / unit
                        );
                    }
                    worst = worst.max(most);
                    println!("SPREAD TWO {what}: {panels} panels, {most:.3} units");
                }
            }
        }
    }
    println!("SPREAD TWO worst {worst:.3} units");
}
