//! Branches that run back over themselves (2026-11-23, docs/design/
//! booleans.md › *A step through a point where two curves of a crossing
//! cross each other: traced and mended*).
//!
//! Where two curves of a crossing cross each other the surfaces touch,
//! and the sense in which the march travels a curve flips at that point.
//! A step through such a point can land on the same curve beyond it with
//! the tangent there agreeing with the one the step left with, which is
//! all a step's turn is held to; the run then goes on from there back
//! towards the point, over the stretch the fit fills the step's span
//! with. The fit accepts that branch: it is within the tolerance of both
//! surfaces all its length. Found between round surfaces of one small
//! radius (fillets' tubes) in two random designs, 106507 and 118868.
//! Since 2026-11-23 a branch whose points run back over themselves is not
//! accepted: the step the fold lies on is taken up again, as a span the
//! fit gave up on is, and asked as it was first made the march refuses
//! it, in words of its own.
//!
//! Each pair is held to its crossing as known without the marcher: an
//! angle that runs one way along every curve of the crossing between the
//! points where its curves cross, and the places of the crossing at each
//! angle, in closed form. Every point of every curve is on the crossing;
//! along every curve the angle does not run back (by more than a few
//! tolerances of length); the curves are as long together as the
//! crossing is inside the box; and every place of the crossing in the box
//! away from where its curves cross is on a curve. Where the march as
//! first made answers, its answer is the same bit for bit.
//!
//! - The two designs' pairs, as their fillets asked them (the surfaces and
//!   the box written out by a probe of the march, Check 45260043956): a
//!   cylinder through a torus of its own tube's radius, and two tori of
//!   one size side by side; each at four scales, upright and turned, and
//!   in boxes of many sizes.
//! - Two cylinders of one radius whose axes cross at right angles: the
//!   crossing is two ellipses that cross each other at the two points
//!   where the cylinders touch (Steinmetz's). Asked in boxes of many
//!   sizes, the march's first step is many times the radius and is
//!   halved until it is taken: in some of them it is taken across an
//!   ellipse, through a point where they cross.
//! - A ball and a cylinder of half its radius inside it, touching it
//!   (Viviani's figure of eight): one curve that crosses itself where the
//!   two touch. Asked in the same boxes. No step through that point can
//!   be taken: wherever beyond it a step lands, the tangent there, the
//!   sense of travel flipped, turns from the one it left with by more
//!   than a step may (read from the closed form, below).

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// `!(x <= y)` is deliberate: unlike `x > y` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

mod common;

use common::*;
use craft3d_geom::Surface;
use craft3d_geom::intersect::{
    Meeting, SurfaceMeeting, meet_surfaces, meet_surfaces_numeric, meet_surfaces_numeric_once,
    meet_surfaces_once,
};
use craft3d_math::{Bbox3, Frame, Portable, Tolerance, Vec3};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// A line on the test's output that the harness does not hold back: the
/// build server's log keeps it whether the test passes or fails.
fn say(line: &str) {
    let _ = std::io::Write::write_all(&mut std::io::stderr(), format!("{line}\n").as_bytes());
}

/// The four scales a pair is asked at.
const SCALES_FOUR: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

/// Where a pair's places go: scaled by `k` about the origin of the
/// coordinates it is written in, then put in `frame`. Upright (the world's
/// frame) the places and directions are the pair's own, bit for bit, at
/// `k` = 1.
#[derive(Clone, Copy)]
struct Placed {
    k: f64,
    frame: Frame,
}

impl Placed {
    fn point(&self, p: Vec3) -> Vec3 {
        if self.frame == Frame::WORLD {
            p * self.k
        } else {
            self.frame.point(self.k * p.x, self.k * p.y, self.k * p.z)
        }
    }

    fn vector(&self, d: Vec3) -> Vec3 {
        if self.frame == Frame::WORLD {
            d
        } else {
            self.frame.vector(d.x, d.y, d.z)
        }
    }

    /// The place in the pair's own coordinates.
    fn local(&self, p: Vec3) -> Vec3 {
        let (x, y, z) = self.frame.local(p);
        Vec3::new(x / self.k, y / self.k, z / self.k)
    }

    fn frame_of(&self, origin: Vec3, z: Vec3, x: Vec3) -> Frame {
        Frame::new(self.point(origin), self.vector(z), self.vector(x)).unwrap()
    }

    /// The box, along the world's axes, that holds the box with these
    /// corners in the pair's own coordinates.
    fn boxed(&self, lo: Vec3, hi: Vec3) -> Bbox3 {
        let mut corners = Vec::new();
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    corners.push(self.point(Vec3::new(x, y, z)));
                }
            }
        }
        Bbox3::from_points(corners)
    }
}

/// Upright (the pair as written, scaled) and turned (askew to the
/// world's axes and moved off its origin).
fn placings(k: f64) -> [(&'static str, Placed); 2] {
    let turned = Frame::new(
        Vec3::new(0.37, -0.81, 0.52) * k,
        Vec3::new(0.3, -0.4, 1.0),
        Vec3::new(1.0, 0.2, 0.0),
    )
    .unwrap();
    [
        (
            "upright",
            Placed {
                k,
                frame: Frame::WORLD,
            },
        ),
        ("turned", Placed { k, frame: turned }),
    ]
}

/// A pair, the box and tolerance it is asked in, and its crossing as known
/// without the marcher.
struct Pair {
    what: String,
    a: Surface,
    b: Surface,
    within: Bbox3,
    tol: Tolerance,
    /// The angle of a place, which runs one way along each curve of the
    /// crossing between the points where its curves cross.
    angle: Box<dyn Fn(Vec3) -> f64>,
    /// The places of the crossing at an angle, in the same order at every
    /// angle (one sheet of the crossing at each place of the list).
    at: Box<dyn Fn(f64) -> Vec<Vec3>>,
    /// The angles at which its curves cross each other.
    crossing: Vec<f64>,
}

/// `d` brought into `(-π, π]`.
fn wrapped(d: f64) -> f64 {
    d - TAU * (d / TAU).round()
}

/// The distance from `q` to a polyline.
fn to_line(points: &[Vec3], q: Vec3) -> f64 {
    let mut best = f64::INFINITY;
    for w in points.windows(2) {
        let d = w[1] - w[0];
        let l2 = d.length_squared();
        let s = if l2 > 0.0 {
            ((q - w[0]).dot(d) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        best = best.min((w[0] + d * s).distance(q));
    }
    best
}

/// How long the crossing is inside the box: each of its sheets sampled at
/// `n` angles, the pieces whose two ends are inside.
fn known_length(pair: &Pair, n: usize) -> f64 {
    let rows: Vec<Vec<Vec3>> = (0..=n)
        .map(|i| (pair.at)(TAU * i as f64 / n as f64))
        .collect();
    let sheets = rows[0].len();
    let mut length = 0.0;
    for sheet in 0..sheets {
        for w in rows.windows(2) {
            let (p, q) = (w[0][sheet], w[1][sheet]);
            if pair.within.contains(p, 0.0) && pair.within.contains(q, 0.0) {
                length += p.distance(q);
            }
        }
    }
    length
}

/// What the marcher answered is the crossing: every curve on both
/// surfaces and inside the box (`check_meeting`), every point of it on the
/// crossing, none running back over itself, all of them together as long
/// as the crossing is inside the box, and every place of the crossing in
/// the box away from where its curves cross on one of them.
fn check(pair: &Pair, m: &Meeting) {
    let what = &pair.what;
    let t = pair.tol.linear;
    check_meeting(m, &pair.a, &pair.b, &pair.within, &pair.tol);
    assert!(!m.curves.is_empty(), "{what}: no curve");
    let lines: Vec<Vec<Vec3>> = m.curves.iter().map(|c| samples(c, 2000)).collect();
    let mut length = 0.0;
    for (n, points) in lines.iter().enumerate() {
        for p in points {
            let off = (pair.at)((pair.angle)(*p))
                .iter()
                .map(|q| q.distance(*p))
                .fold(f64::INFINITY, f64::min);
            assert!(
                off <= 4.0 * t,
                "{what}: curve {n} is {} tolerances off the crossing at {p:?}",
                off / t
            );
        }
        let turns: Vec<f64> = points
            .windows(2)
            .map(|w| wrapped((pair.angle)(w[1]) - (pair.angle)(w[0])))
            .collect();
        let total: f64 = turns.iter().sum();
        let back: f64 = turns
            .iter()
            .zip(points.windows(2))
            .filter(|(d, _)| **d * total < 0.0)
            .map(|(_, w)| w[0].distance(w[1]))
            .sum();
        assert!(
            back <= 4.0 * t,
            "{what}: curve {n} runs back over itself for {} tolerances",
            back / t
        );
        length += points.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>();
    }
    let known = known_length(pair, 40_000);
    assert!(
        (length - known).abs() <= 32.0 * t + 1e-5 * known,
        "{what}: the curves are {} tolerances long, the crossing in the box {}",
        length / t,
        known / t
    );
    for i in 0..64 {
        let angle = TAU * (f64::from(i) + 0.5) / 64.0;
        if pair
            .crossing
            .iter()
            .any(|c| wrapped(angle - c).abs() < 0.05)
        {
            continue;
        }
        for q in (pair.at)(angle) {
            if !pair.within.contains(q, -4.0 * t) {
                continue;
            }
            let near = lines.iter().filter(|l| to_line(l, q) <= 2.0 * t).count();
            assert!(
                near >= 1,
                "{what}: the crossing at {q:?} (angle {angle}) is on no curve"
            );
        }
    }
}

/// How the march as it is first made answered a pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum First {
    /// It answered, and the march with branches taken up again answered
    /// the same, bit for bit.
    Same,
    /// A branch it fitted ran back over itself.
    Folded,
    /// It refused otherwise.
    Refused,
}

/// Whom a pair is asked of.
#[derive(Clone, Copy)]
enum By {
    /// The marcher alone (`meet_surfaces_numeric_once`, then
    /// `meet_surfaces_numeric`): a pair with a closed form.
    Marcher,
    /// As a boolean asks it (`meet_surfaces_once`, then `meet_surfaces`).
    Boolean,
}

/// Asks a pair as first made and with branches taken up again: the
/// second held to the crossing, the first the same where it answers.
fn asked(pair: &Pair, by: By) -> First {
    let (once, again) = match by {
        By::Marcher => (
            meet_surfaces_numeric_once(&pair.a, &pair.b, &pair.within, &pair.tol),
            meet_surfaces_numeric(&pair.a, &pair.b, &pair.within, &pair.tol),
        ),
        By::Boolean => (
            meet_surfaces_once(&pair.a, &pair.b, &pair.within, &pair.tol),
            meet_surfaces(&pair.a, &pair.b, &pair.within, &pair.tol),
        ),
    };
    let again = match again {
        Ok(SurfaceMeeting::Meets(m)) => m,
        other => panic!("{}: {other:?} (first asked: {once:?})", pair.what),
    };
    check(pair, &again);
    match once {
        Ok(SurfaceMeeting::Meets(first)) => {
            assert_eq!(
                format!("{first:?}"),
                format!("{again:?}"),
                "{}: answered first, and otherwise when taken up again",
                pair.what
            );
            First::Same
        }
        Err(e) if e.to_string().contains("run back over themselves") => First::Folded,
        Err(_) => First::Refused,
        Ok(other) => panic!("{}: first asked, {other:?}", pair.what),
    }
}

/// The boxes a pair of radius `r` is asked in, in its own coordinates,
/// about `centre`: cubes of `2 least` radii a side and `sizes − 1` more,
/// each 2^(1/8) times the last (at the least of 200 and 24 sizes, 400 to
/// 2940 radii: the march's first step, a 128th of the diagonal, 5 to 40
/// radii), each about the centre and off it.
fn boxes(r: f64, least: f64, sizes: i32, centre: Vec3) -> Vec<(Vec3, Vec3)> {
    let offsets = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.37, -0.21, 0.13)];
    let mut found = Vec::new();
    for j in 0..sizes {
        let half = least * r * 2f64.ppowf(f64::from(j) / 8.0);
        for off in offsets {
            let middle = centre + off * half;
            let corner = Vec3::new(half, half, half);
            found.push((middle - corner, middle + corner));
        }
    }
    found
}

/// How the march as first made answered, by scale, placing and answer.
type Counted = BTreeMap<(u64, &'static str, First), usize>;

/// A pair in each of `corners` at the four scales (`unit`: the scale the
/// pair is written at), upright and turned, asked by `by`: every answer
/// held to the crossing; how the march as first made answered, counted
/// and written out.
fn family(
    what: &str,
    unit: f64,
    corners: &[(Vec3, Vec3)],
    by: By,
    make: impl Fn(Placed, (Vec3, Vec3), &str) -> Pair,
) -> Counted {
    let mut counted = Counted::new();
    for scale in SCALES_FOUR {
        for (how, placed) in placings(scale / unit) {
            for corner in corners {
                let pair = make(placed, *corner, how);
                let first = asked(&pair, by);
                if first != First::Same {
                    say(&format!("FOLD-TEST {first:?}: {}", pair.what));
                }
                *counted.entry((scale.to_bits(), how, first)).or_insert(0) += 1;
            }
        }
    }
    for ((scale, how, first), n) in &counted {
        say(&format!(
            "FOLD-TEST {what} at {} {how}: {first:?} {n}",
            f64::from_bits(*scale)
        ));
    }
    counted
}

/// How many answers of a family the march as first made refused for a
/// branch that ran back over itself.
fn folded(counted: &Counted) -> usize {
    counted
        .iter()
        .filter(|((_, _, first), _)| *first == First::Folded)
        .map(|(_, n)| n)
        .sum()
}

/// The pair of random design 106507 (its `feature.fillet` of group 10,
/// two of the fillet's tools, as the probe wrote them out): a cylinder of
/// radius `r` = 1.2207e-3 (the fillet along a straight edge) and a torus
/// of radii `R` = 4.998779296875 and `r_t` = 1.220703125e-3 (along an arc
/// of radius 5), the cylinder's axis along the torus's, through its middle
/// circle; tolerance 1e-6 at scale 1, the pair's own. About the cylinder's
/// axis at the angle `θ` its point is `ρ(θ)` from the torus's axis, and
/// the crossing is there at `± √(r_t² − (ρ − R)²)` from the torus's middle
/// plane (within 0.15 of a tolerance of the two ellipses `±r sin θ` of two
/// cylinders crossing); its two curves cross where `θ` points along the
/// torus's radius, out or in (`ρ = R ± r`), where the two touch.
fn tube_through_ring(placed: Placed, (lo, hi): (Vec3, Vec3), how: &str) -> Pair {
    let k = placed.k;
    let r = 1.220_703_125_000_003_5e-3;
    let axis = Vec3::new(
        -16.248_779_147_826_998,
        1.221_001_233_136_397_5e-3,
        11.248_779_296_875,
    );
    let centre = Vec3::new(-11.25, -7.498_779_296_875, 11.25);
    let (major, minor) = (4.998_779_296_875, 1.220_703_125e-3);
    let tube = cylinder(
        placed.frame_of(axis, Vec3::new(0.0, -1.0, 0.0), Vec3::new(1.0, 0.0, -0.0)),
        r * k,
    );
    let ring = torus(
        placed.frame_of(centre, Vec3::Y, Vec3::new(-0.0, 0.0, -1.0)),
        major * k,
        minor * k,
    );
    let out = (axis.z - centre.z).patan2(axis.x - centre.x);
    Pair {
        what: format!("a tube through a ring at {k}, {how}, box {lo:?} to {hi:?}"),
        a: tube,
        b: ring,
        within: placed.boxed(lo, hi),
        tol: Tolerance::new(1e-6 * k, 1e-9).unwrap(),
        angle: Box::new(move |p: Vec3| -> f64 {
            let q = placed.local(p);
            (q.z - axis.z).patan2(q.x - axis.x)
        }),
        at: Box::new(move |angle: f64| -> Vec<Vec3> {
            let (x, z) = (axis.x + r * angle.pcos(), axis.z + r * angle.psin());
            let rho = (x - centre.x).phypot(z - centre.z);
            let h = (minor * minor - (rho - major) * (rho - major))
                .max(0.0)
                .sqrt();
            [1.0, -1.0]
                .into_iter()
                .map(|sign| placed.point(Vec3::new(x, centre.y + sign * h, z)))
                .collect()
        }),
        crossing: vec![out, wrapped(out + PI)],
    }
}

/// The box the fillet of design 106507 asked its two tools in.
fn box_106507() -> (Vec3, Vec3) {
    (
        Vec3::new(
            -16.475_182_455_935_95,
            -7.725_182_604_983_954,
            11.022_375_988_766_045,
        ),
        Vec3::new(
            -16.022_375_839_718_045,
            -2.273_596_691_891_046,
            11.475_182_604_983_955,
        ),
    )
}

/// The pair of random design 118868 (its `feature.fillet` of group 8, two
/// of the fillet's tools, as the probe wrote them out): two tori of radii
/// `R` = 2.490234375e-3 and `r` = 9.765625e-6 (fillets along the top
/// edges of two cylinders of radius 0.0025 side by side), their axes
/// along z, their middle circles in one plane, their centres `d` = 0.0025
/// apart along y; tolerance 1e-9 at the pair's own scale, 1e-3. About the
/// first's tube at the angle `v` its point is `ρ = R + r cos v` from its
/// axis, and on the second where that is `ρ` from the second's axis (the
/// plane between them, `y = d/2`) or `2R − ρ` (`y = d/2 + 2 R r cos v / d`);
/// in two places each, either side of the plane through the axes. The
/// two curves cross at the top and the bottom of the tubes (`v = ±π/2`),
/// where the tori touch.
fn two_rings(placed: Placed, (lo, hi): (Vec3, Vec3), how: &str) -> Pair {
    let k = placed.k;
    let (major, minor) = (0.002_490_234_375_000_000_3, 9.765_624_999_999_897e-6);
    let (mid, d) = (0.002_509_765_624_999_999_4, 0.0025);
    let ring = |y: f64| {
        torus(
            placed.frame_of(Vec3::new(0.0, y, mid), Vec3::Z, Vec3::new(1.0, 0.0, -0.0)),
            major * k,
            minor * k,
        )
    };
    Pair {
        what: format!("two rings at {k}, {how}, box {lo:?} to {hi:?}"),
        a: ring(0.0),
        b: ring(d),
        within: placed.boxed(lo, hi),
        tol: Tolerance::new(1e-9 * k, 1e-9).unwrap(),
        angle: Box::new(move |p: Vec3| -> f64 {
            let q = placed.local(p);
            (q.z - mid).patan2(q.x.phypot(q.y) - major)
        }),
        at: Box::new(move |angle: f64| -> Vec<Vec3> {
            let (s, c) = (angle.psin(), angle.pcos());
            let (rho, z) = (major + minor * c, mid + minor * s);
            let mut places = Vec::new();
            for y in [0.5 * d, 0.5 * d + 2.0 * major * minor * c / d] {
                let x = (rho * rho - y * y).max(0.0).sqrt();
                for side in [1.0, -1.0] {
                    places.push(placed.point(Vec3::new(side * x, y, z)));
                }
            }
            places
        }),
        crossing: vec![FRAC_PI_2, -FRAC_PI_2],
    }
}

/// The box the fillet of design 118868 asked its two tools in.
fn box_118868() -> (Vec3, Vec3) {
    (
        Vec3::new(
            -0.002_612_599_302_491_977,
            -0.000_112_599_302_491_977_02,
            -0.000_102_833_677_491_977_66,
        ),
        Vec3::new(
            0.002_612_599_302_491_977,
            0.002_612_599_302_491_977,
            0.005_122_364_927_491_976,
        ),
    )
}

/// Two cylinders of radius `r` (1e-3 at scale 1), the axis of the one
/// walked along z, the other's along x, both through the origin: they
/// touch at `(0, ±r, 0)` and cross in the ellipses `z = ±x` on the first.
/// About the first's axis at the angle `θ` the crossing is
/// `(r cos θ, r sin θ, ±r cos θ)`; its ellipses cross at `θ = ±π/2`.
fn two_tubes(placed: Placed, (lo, hi): (Vec3, Vec3), how: &str) -> Pair {
    let r = 1e-3;
    let k = placed.k;
    let a = cylinder(placed.frame_of(Vec3::ZERO, Vec3::Z, Vec3::X), r * k);
    let b = cylinder(placed.frame_of(Vec3::ZERO, Vec3::X, Vec3::Y), r * k);
    Pair {
        what: format!("two tubes at {k}, {how}, box {lo:?} to {hi:?}"),
        a,
        b,
        within: placed.boxed(lo, hi),
        tol: Tolerance::new(1e-6 * k, 1e-9).unwrap(),
        angle: Box::new(move |p: Vec3| -> f64 {
            let q = placed.local(p);
            q.y.patan2(q.x)
        }),
        at: Box::new(move |angle: f64| -> Vec<Vec3> {
            let (s, c) = (angle.psin(), angle.pcos());
            [1.0, -1.0]
                .into_iter()
                .map(|sign| placed.point(Vec3::new(r * c, r * s, sign * r * c)))
                .collect()
        }),
        crossing: vec![FRAC_PI_2, -FRAC_PI_2],
    }
}

/// A ball of radius `2a` about the origin (`a` 1e-3 at scale 1) and a
/// cylinder of radius `a` along z through `(a, 0, 0)`, walked: it touches
/// the ball inside at `(2a, 0, 0)`. About its axis at the angle `θ`
/// (nought towards that point) the crossing is
/// `(a + a cos θ, a sin θ, ±2a sin(θ/2))`: Viviani's figure of eight,
/// which crosses itself at `θ = 0`.
fn ball_and_tube(placed: Placed, (lo, hi): (Vec3, Vec3), how: &str) -> Pair {
    let a = 1e-3;
    let k = placed.k;
    let ball = sphere(placed.frame_of(Vec3::ZERO, Vec3::Z, Vec3::X), 2.0 * a * k);
    let tube = cylinder(
        placed.frame_of(Vec3::new(a, 0.0, 0.0), Vec3::Z, Vec3::X),
        a * k,
    );
    Pair {
        what: format!("a ball and a tube at {k}, {how}, box {lo:?} to {hi:?}"),
        a: ball,
        b: tube,
        within: placed.boxed(lo, hi),
        tol: Tolerance::new(1e-6 * k, 1e-9).unwrap(),
        angle: Box::new(move |p: Vec3| -> f64 {
            let q = placed.local(p);
            q.y.patan2(q.x - a)
        }),
        at: Box::new(move |angle: f64| -> Vec<Vec3> {
            let (s, c) = (angle.psin(), angle.pcos());
            let z = 2.0 * a * (0.5 * angle).psin();
            [1.0, -1.0]
                .into_iter()
                .map(|sign| placed.point(Vec3::new(a + a * c, a * s, sign * z)))
                .collect()
        }),
        crossing: vec![0.0],
    }
}

/// The pair of random design 106507 in the box its fillet asked it in:
/// as the march was first made, a branch it fitted runs back over itself
/// (the backward run's first step from a point 0.51 radians round the
/// cylinder from where the curves cross, 2775 tolerances long, through
/// that point onto the same curve 2.5 radians beyond it, at a turn of the
/// tangent of 0.1 radians, and the run back from there), which `main`
/// accepted; it is refused now, and asked again the march is taken up
/// from that step and gives the crossing. At the other three scales and
/// turned, the crossing, and the same as first made where that answers.
#[test]
fn the_pair_of_design_106507_is_the_crossing_where_its_first_march_ran_back() {
    let mut counted = Counted::new();
    for scale in SCALES_FOUR {
        for (how, placed) in placings(scale) {
            let pair = tube_through_ring(placed, box_106507(), how);
            let first = asked(&pair, By::Boolean);
            say(&format!(
                "FOLD-TEST design 106507: {first:?}: {}",
                pair.what
            ));
            *counted.entry((scale.to_bits(), how, first)).or_insert(0) += 1;
            if scale == 1.0 && how == "upright" {
                assert_eq!(first, First::Folded, "{}", pair.what);
            }
        }
    }
    say(&format!("FOLD-TEST design 106507: {counted:?}"));
}

/// The pair of random design 118868 in the box its fillet asked it in: as
/// the march was first made, a branch it fitted runs back over itself (the
/// backward run's first step, 32 782 tolerances, through the top of the
/// tubes onto the same curve beyond, and its fill out along the other
/// curve's arm and back), which `main` accepted; it is refused now, and
/// asked again it gives the eight halves of the four closed curves, each
/// from the top of the tubes to the bottom. At the other three scales and
/// turned, the crossing, and the same as first made where that answers.
#[test]
fn the_pair_of_design_118868_is_the_crossing_where_its_first_march_ran_back() {
    let mut counted = Counted::new();
    for scale in SCALES_FOUR {
        for (how, placed) in placings(scale / 1e-3) {
            let pair = two_rings(placed, box_118868(), how);
            let first = asked(&pair, By::Boolean);
            say(&format!(
                "FOLD-TEST design 118868: {first:?}: {}",
                pair.what
            ));
            *counted.entry((scale.to_bits(), how, first)).or_insert(0) += 1;
            if scale == 1e-3 && how == "upright" {
                assert_eq!(first, First::Folded, "{}", pair.what);
            }
        }
    }
    say(&format!("FOLD-TEST design 118868: {counted:?}"));
}

/// The pair of random design 106507 in boxes of many sizes about the
/// place where its curves cross: every answer is the crossing, the same
/// as first made where that answers, and in some boxes the march as first
/// made ran back over itself.
#[test]
fn a_tube_through_a_ring_of_its_own_radius_is_marched_once_along_each_curve() {
    let centre = Vec3::new(
        -16.248_779_147_826_998,
        -7.498_779_296_875,
        11.248_779_296_875,
    );
    let counted = family(
        "tube through ring",
        1.0,
        &boxes(1.220_703_125e-3, 200.0, 24, centre),
        By::Boolean,
        tube_through_ring,
    );
    assert!(folded(&counted) >= 1, "no box made the march run back");
}

/// The pair of random design 118868 in boxes of many sizes about the
/// middle between the tori, each holding both places where the tori
/// cross (800 to 4 400 tube radii a side): every answer is the crossing,
/// the same as first made where that answers. Not larger: in a box of
/// 5 870 tube radii a side the arms of a crossing point are seeded a
/// hundredth of the march's longest step from it, 31 021 tolerances, more
/// than three tube radii, and a seed within half that of a crossing point
/// is left to the arms; every place of the ovals in the plane between the
/// tori is that near the top or the bottom of the tubes, and neither the
/// march as first made nor the one taken up again finds them (the probe of
/// Check 45264346266: four curves of eight, the same both ways; *Owed* in
/// docs/design/booleans.md).
#[test]
fn two_rings_side_by_side_are_marched_once_along_each_curve() {
    let centre = Vec3::new(0.0, 0.00125, 0.002_509_765_625);
    family(
        "two rings",
        1e-3,
        &boxes(9.765_625e-6, 400.0, 22, centre),
        By::Boolean,
        two_rings,
    );
}

/// Two cylinders of one radius at right angles, in boxes of many sizes,
/// at four scales, upright and turned: every answer is the crossing (two
/// ellipses that cross each other where the cylinders touch); asked as the
/// march was first made, every answer is the same bit for bit, and some
/// are a branch that runs back over itself, which is no longer accepted
/// (the first step, halved until it is taken, taken across an ellipse
/// through a point where they cross). This pair has a closed form, so the
/// marcher is asked alone.
#[test]
fn two_tubes_crossing_at_right_angles_are_marched_once_along_each_curve() {
    let counted = family(
        "two tubes",
        1.0,
        &boxes(1e-3, 200.0, 24, Vec3::ZERO),
        By::Marcher,
        two_tubes,
    );
    assert!(folded(&counted) >= 1, "no box made the march run back");
}

/// A ball and a cylinder of half its radius touching it inside, in the
/// same boxes, at four scales, upright and turned: every answer is
/// Viviani's figure of eight, and the same as the march first made gives.
/// No step through the point where it crosses itself is taken: about
/// that point the figure's tangent `(−sin t, cos t, cos(t/2))` (`t` the
/// angle about the cylinder, nought there) runs from `t` on one lobe to
/// `−dP/dt` at the place beyond it on the other (the sense of travel
/// flipped), and for no pair of places do the two agree within a step's
/// turn of 0.2 radians (scanned at 2000 by 2000 places: none). So no
/// branch of this pair runs back over itself, and the march answers it as
/// it did.
#[test]
fn a_ball_and_a_tube_touching_inside_it_meet_in_a_figure_of_eight_marched_as_it_was() {
    let counted = family(
        "ball and tube",
        1.0,
        &boxes(1e-3, 200.0, 24, Vec3::ZERO),
        By::Boolean,
        ball_and_tube,
    );
    assert!(
        counted.keys().all(|(_, _, first)| *first == First::Same),
        "{counted:?}"
    );
}
