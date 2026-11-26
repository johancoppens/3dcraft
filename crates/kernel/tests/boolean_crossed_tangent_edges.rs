//! Two tangent edges that cross on a plane both their faces lie on
//! (random design 121704, the rest of group B; docs/design/booleans.md ›
//! *Two tangent edges that cross on a shared cap*).
//!
//! Half a torus (a circle of radius 7.5 about (12.5, 2.5) in z = −10 turned
//! half a turn about the line x = 0, z = −10), its end rim filleted by 2.5,
//! less a copy moved 7.5 along that line. Both end caps lie in z = −10,
//! each bounded by its fillet's tangent edge (a spline), and the two edges
//! cross twice, at (−8.846342268, 6.25) and (−15.608242325, 6.25). The
//! result is one shell: the original's cap less the copy's, bounded by the
//! copy's tangent edge where the copy's fillet leaves the plane, a wall
//! that thins to nothing along that edge (2026-10-18); at each crossing the
//! cap's corner, the original's fillet and the copy's meet, and the curve
//! in which the two fillets meet ends there. The boolean's own check
//! refused what it built (*"edge 10 is used 1 times"*): the crossing that
//! the original's tangent edge has no hit for on the copy's fillet, which
//! it only touches and which ends at the copy's tangent edge, went uncut,
//! and the original's cap was not divided along the copy's edge. The
//! places are now where the two curves cross (`corner::border_crossings`).
//!
//! Every body is held to `topo::check`, the boolean's full check and its
//! volume written without the kernel ([`oracle`]: the half tori are discs
//! turned about the axis, and a fillet removes at each point of the disc an
//! angle from its cap, found by the primitives' own equations), within the
//! tolerance times its area, at scales 1e-3 to 1e6 and turned out of the
//! axes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

mod common;

use common::circle;
use craft3d_kernel::boolean::BooleanOp::{self, Intersect, Subtract, Union};
use craft3d_kernel::boolean::{VerifyOptions, boolean, verify};
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::profile::detect_profiles;
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, EdgeId, check, mass_properties};
use std::f64::consts::PI;

/// The scales the cases are built at; the tolerance is a millionth of
/// the scale.
const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// The volumes written without the kernel, at scale 1.
///
/// In the plane through the torus's axis (the line x = 0, z = −10, along
/// y), at distance `rho` from it and height `y`, the half torus A0 is the
/// disc of radius 7.5 about (12.5, 2.5), turned through the angle `theta`
/// from its end cap (theta = 0, the half plane x < 0 of z = −10) to its
/// start. The fillet of the end rim removes at each point of the disc the
/// angles from 0 to `theta_star`: the points of A0 further than 2.5 from
/// the balls' centres, which are the points at depth 2.5 or more below the
/// cap within 5 of the tube's centre circle. Such a point is removed
/// where its foot on the cap's plane is outside the oval the centres at
/// depth 2.5 stand over (`oval_distance` > 0), the nearest point of the
/// torus of minor radius 5 is shallower than 2.5, and the point is
/// further than 2.5 from every centre at depth 2.5 (the square of `d` plus
/// the square of `2.5 − depth` more than 6.25, `d` from the point's own
/// foot). (The same conditions, in columns along the depth, agree with the
/// distance to the centres found by brute force at 2,471 points:
/// `plan/seed-121704/tools` in the workshop.) The copy B is A moved 7.5
/// along y: its disc is about (12.5, 10), and the angle is the same
/// coordinate for both. So
///
/// - vol A = π² 12.5 · 7.5² − ∫ over A's disc of rho θ*_A;
/// - vol(A − B) = vol(A0 − B0) − ∫ over A's disc outside B's of rho θ*_A,
///   plus ∫ over both discs of rho max(0, θ*_B − θ*_A), where vol(A0 − B0)
///   is π 12.5 times the area of the lune (Pappus).
///
/// The plane integrals in polar coordinates `(s, beta)` about A's tube's
/// centre, where B's disc is `s ≤ 15 sin beta`: each line of fixed `beta`
/// cut where the integrand changes form (where θ* starts, at the oval;
/// where the torus's nearest point takes over from the balls; where θ*_A
/// and θ*_B cross; B's circle) and integrated by Gauss–Legendre on each
/// piece; `beta` halved until the halves agree.
mod oracle {
    use craft3d_math::Portable;
    use craft3d_math::quadrature::gauss_legendre;
    use std::f64::consts::PI;

    /// The torus's major radius, its tube's radius, the fillet's radius.
    const MAJOR: f64 = 12.5;
    const TUBE: f64 = 7.5;
    const ROUND: f64 = 2.5;
    /// How far the balls' centres are from the tube's centre circle.
    const SPINE: f64 = TUBE - ROUND;
    /// The heights of the two tubes' centres.
    pub const A: f64 = 2.5;
    pub const B: f64 = 10.0;

    /// The oval the balls' centres stand over, seen from above: at angle
    /// `psi` about the tube's centre, its point and first two derivatives.
    fn oval(psi: f64, cy: f64) -> ([f64; 2], [f64; 2], [f64; 2]) {
        let (s, c) = psi.psin_cos();
        let rho = MAJOR + SPINE * c;
        let (d1, d2) = (-SPINE * s, -SPINE * c);
        let foot = (rho * rho - ROUND * ROUND).sqrt();
        let x1 = -rho * d1 / foot;
        let x2 = -((d1 * d1 + rho * d2) - (rho * d1) * (rho * d1) / (foot * foot)) / foot;
        ([-foot, cy + SPINE * s], [x1, SPINE * c], [x2, -SPINE * s])
    }

    /// Signed distance in the cap's plane from `(x, y)` (x < 0) to the
    /// oval: positive outside it. Newton on the angle from the point's own.
    pub fn oval_distance(x: f64, y: f64, cy: f64) -> f64 {
        let ground = (x * x + ROUND * ROUND).sqrt() - MAJOR;
        let mut psi = (y - cy).patan2(ground);
        for _ in 0..60 {
            let ([px, py], [dx, dy], [ex, ey]) = oval(psi, cy);
            let g = (px - x) * dx + (py - y) * dy;
            let mut dg = dx * dx + dy * dy + (px - x) * ex + (py - y) * ey;
            if dg <= 0.0 {
                dg = dx * dx + dy * dy;
            }
            let step = g / dg;
            psi -= step;
            if step.abs() < 1e-15 {
                break;
            }
        }
        let ([px, py], _, _) = oval(psi, cy);
        let d = (px - x).phypot(py - y);
        let inside = ground * ground + (y - cy) * (y - cy) < SPINE * SPINE;
        if inside { -d } else { d }
    }

    /// The angles the end rim's fillet of the tube centred at height `cy`
    /// removes from the cap at `(rho, y)` of its disc, all told: from the
    /// cap to where the point enters the balls and, on the outer side
    /// where it leaves them again before the torus's nearest point is as
    /// deep as they are (the cusp between a ball and the torus beside the
    /// fillet's foot on the torus), from there to that angle.
    pub fn theta_star(rho: f64, y: f64, cy: f64) -> f64 {
        if oval_distance(-rho, y, cy) <= 0.0 {
            return 0.0;
        }
        let s = (rho - MAJOR).phypot(y - cy);
        if s <= 0.0 {
            return 0.0;
        }
        let nearest = MAJOR + SPINE * (rho - MAJOR) / s;
        let top = (ROUND / nearest).min(1.0).pasin();
        // Positive where the point is removed: its foot outside the oval,
        // itself outside the balls.
        let outside = |t: f64| {
            let (sine, cosine) = t.psin_cos();
            let (x, depth) = (-rho * cosine, rho * sine);
            let d = oval_distance(x, y, cy);
            if d > 0.0 {
                d * d + (ROUND - depth) * (ROUND - depth) - ROUND * ROUND
            } else {
                -1.0
            }
        };
        // Looked at on either side of where the point is as deep as the
        // balls' centres, which a stretch inside the balls, however short,
        // holds: entered before it, left after it.
        let level = (ROUND / rho).min(1.0).pasin();
        let mut cuts = if level < top {
            let mut found = changes(&outside, (0.0, level), 4);
            found.extend(changes(&outside, (level, top), 4));
            found
        } else {
            changes(&outside, (0.0, top), 8)
        };
        cuts.extend([0.0, top]);
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        cuts.windows(2)
            .filter(|w| outside(0.5 * (w[0] + w[1])) > 0.0)
            .map(|w| w[1] - w[0])
            .sum()
    }

    /// Positive where the torus's nearest point sets θ* at `(rho, y)`,
    /// negative where the balls do or nothing is removed.
    fn foot_side(rho: f64, y: f64, cy: f64) -> f64 {
        if oval_distance(-rho, y, cy) <= 0.0 {
            return -1.0;
        }
        let s = (rho - MAJOR).phypot(y - cy);
        let nearest = MAJOR + SPINE * (rho - MAJOR) / s;
        let (sine, cosine) = (ROUND / nearest).min(1.0).pasin().psin_cos();
        let (x, depth) = (-rho * cosine, rho * sine);
        let d = oval_distance(x, y, cy);
        if d > 0.0 {
            d * d + (ROUND - depth) * (ROUND - depth) - ROUND * ROUND
        } else {
            -1.0
        }
    }

    /// Where `g` changes sign in `[a, b]`: `n` samples, then halving.
    fn changes(g: &dyn Fn(f64) -> f64, (a, b): (f64, f64), n: usize) -> Vec<f64> {
        let at = |k: usize| a + (b - a) * k as f64 / n as f64;
        let mut found = Vec::new();
        let mut before = (a, g(a));
        for k in 1..=n {
            let here = (at(k), g(at(k)));
            if (before.1 > 0.0) != (here.1 > 0.0) {
                let (mut lo, mut hi, low) = (before.0, here.0, before.1 > 0.0);
                for _ in 0..64 {
                    let middle = 0.5 * (lo + hi);
                    if !(middle > lo && middle < hi) {
                        break;
                    }
                    if (g(middle) > 0.0) == low {
                        lo = middle;
                    } else {
                        hi = middle;
                    }
                }
                found.push(0.5 * (lo + hi));
            }
            before = here;
        }
        found
    }

    /// `∫ f` over `[a, b]` cut at `cuts`: on each piece Gauss–Legendre
    /// after `s = a + (b − a)(1 − cos πt) / 2`, which turns a square root at
    /// either end of a piece (where a ball and the torus are tangent, at
    /// the fillet's foot on the torus) into a smooth integrand.
    fn pieces(f: &dyn Fn(f64) -> f64, (a, b): (f64, f64), mut cuts: Vec<f64>) -> f64 {
        let rule = gauss_legendre(16);
        cuts.retain(|&c| c > a && c < b);
        cuts.extend([a, b]);
        cuts.sort_by(f64::total_cmp);
        cuts.windows(2)
            .map(|w| {
                let length = w[1] - w[0];
                rule.iter()
                    .map(|&(x, weight)| {
                        let (sine, cosine) = (0.5 * PI * (x + 1.0)).psin_cos();
                        let s = w[0] + length * 0.5 * (1.0 - cosine);
                        0.5 * weight * f(s) * length * 0.5 * PI * sine
                    })
                    .sum::<f64>()
            })
            .sum()
    }

    /// `∫ f` over `[a, b]`: eight-point Gauss on the whole and on its
    /// halves, halved until they agree to `eps` (shared out by length).
    fn halved(f: &dyn Fn(f64) -> f64, a: f64, b: f64, eps: f64, depth: u32) -> f64 {
        let rule = gauss_legendre(8);
        let gauss = |a: f64, b: f64| {
            let (middle, half) = (0.5 * (a + b), 0.5 * (b - a));
            half * rule
                .iter()
                .map(|&(x, w)| w * f(middle + half * x))
                .sum::<f64>()
        };
        let middle = 0.5 * (a + b);
        let (whole, halves) = (gauss(a, b), gauss(a, middle) + gauss(middle, b));
        if depth >= 24 || (whole - halves).abs() <= eps {
            halves
        } else {
            halved(f, a, middle, 0.5 * eps, depth + 1) + halved(f, middle, b, 0.5 * eps, depth + 1)
        }
    }

    fn point(s: f64, beta: f64) -> (f64, f64) {
        let (sine, cosine) = beta.psin_cos();
        (MAJOR + s * cosine, A + s * sine)
    }

    /// Where along the ray `beta` the integrand of the tube at `cy` changes
    /// form within `range`: θ* starts (the oval) and the torus's nearest
    /// point takes over.
    fn kinks(beta: f64, cy: f64, range: (f64, f64)) -> Vec<f64> {
        let oval_side = |s: f64| {
            let (rho, y) = point(s, beta);
            oval_distance(-rho, y, cy)
        };
        let foot = |s: f64| {
            let (rho, y) = point(s, beta);
            foot_side(rho, y, cy)
        };
        let mut found = changes(&oval_side, range, 24);
        found.extend(changes(&foot, range, 24));
        found
    }

    /// `∫ rho θ*_A` over A's disc outside B's, along the ray `beta`.
    fn outside_b(beta: f64) -> f64 {
        let sine = beta.psin_cos().0;
        let lo = if sine > 0.0 { 15.0 * sine } else { 0.0 };
        if lo >= TUBE {
            return 0.0;
        }
        let f = |s: f64| {
            let (rho, y) = point(s, beta);
            s * rho * theta_star(rho, y, A)
        };
        pieces(&f, (lo, TUBE), kinks(beta, A, (lo.max(3.0), TUBE)))
    }

    /// `∫ rho max(0, θ*_B − θ*_A)` over both discs, along the ray `beta`.
    fn inside_b(beta: f64) -> f64 {
        let sine = beta.psin_cos().0;
        if sine <= 0.0 {
            return 0.0;
        }
        let range = (0.0, TUBE.min(15.0 * sine));
        let more = |s: f64| {
            let (rho, y) = point(s, beta);
            theta_star(rho, y, B) - theta_star(rho, y, A)
        };
        let f = |s: f64| {
            let (rho, _) = point(s, beta);
            s * rho * more(s).max(0.0)
        };
        let mut cuts = kinks(beta, A, range);
        cuts.extend(kinks(beta, B, range));
        cuts.extend(changes(&more, range, 24));
        pieces(&f, range, cuts)
    }

    /// `∫ rho θ*_A` over A's disc, along the ray `beta`.
    fn whole(beta: f64) -> f64 {
        let f = |s: f64| {
            let (rho, y) = point(s, beta);
            s * rho * theta_star(rho, y, A)
        };
        pieces(&f, (0.0, TUBE), kinks(beta, A, (3.0, TUBE)))
    }

    /// `∫ ray` over `beta` from 0 to `end`, cut where the rays' limits
    /// change form.
    fn over_beta(ray: &dyn Fn(f64) -> f64, end: f64) -> f64 {
        let mut cuts = vec![
            0.0,
            PI / 6.0,
            PI / 2.0,
            5.0 * PI / 6.0,
            PI,
            1.5 * PI,
            2.0 * PI,
        ];
        cuts.retain(|&c| c <= end);
        cuts.windows(2)
            .map(|w| halved(ray, w[0], w[1], 1e-7, 0))
            .sum()
    }

    /// The volume of the half torus A0.
    pub fn half() -> f64 {
        PI * PI * MAJOR * TUBE * TUBE
    }

    /// The volume of A, the half torus with its end rim filleted.
    pub fn rounded() -> f64 {
        half() - over_beta(&whole, 2.0 * PI)
    }

    /// The same by the divergence theorem, a second way: a third of the
    /// flux of the position, from a point of the axis in the cap's plane,
    /// out of the part the fillet removes, which is bounded by the cap
    /// (no flux: the position lies in it), the strip of the torus from the
    /// rim to the fillet's foot on it, at `asin(2.5 / (12.5 + 5 cos b))`
    /// from the cap at tube angle `b` (the ball's centre moved 2.5 along
    /// the torus's normal, in the same plane through the axis), and the
    /// fillet: the pipe of radius 2.5 about the balls' centres (the oval,
    /// 2.5 deep), from straight up (its foot on the cap) to the torus's
    /// normal at the centre, its normal into the ball, `dA = 2.5 |c'| (1 +
    /// 2.5 k sin a)` for the oval's curvature `k` and `a` from straight up.
    pub fn rounded_by_flux() -> f64 {
        let rule = gauss_legendre(64);
        let gauss = |f: &dyn Fn(f64) -> f64, a: f64, b: f64, n: usize| -> f64 {
            (0..n)
                .map(|k| {
                    let lo = a + (b - a) * k as f64 / n as f64;
                    let hi = a + (b - a) * (k + 1) as f64 / n as f64;
                    let (middle, half) = (0.5 * (lo + hi), 0.5 * (hi - lo));
                    half * rule
                        .iter()
                        .map(|&(x, w)| w * f(middle + half * x))
                        .sum::<f64>()
                })
                .sum()
        };
        let strip = |b: f64| {
            let (sine, cosine) = b.psin_cos();
            let foot = (ROUND / (MAJOR + SPINE * cosine)).pasin();
            (MAJOR * cosine + A * sine + TUBE) * TUBE * (MAJOR + TUBE * cosine) * foot
        };
        let middle = -(MAJOR * MAJOR - ROUND * ROUND).sqrt();
        let fillet = |psi: f64| {
            let ([x, y], [x1, y1], [x2, y2]) = oval(psi, A);
            let speed = x1.phypot(y1);
            let curvature = (x1 * y2 - y1 * x2).abs() / (speed * speed * speed);
            let (mut mx, mut my) = (y1 / speed, -x1 / speed);
            if (x - middle) * mx + (y - A) * my < 0.0 {
                (mx, my) = (-mx, -my);
            }
            let rho = MAJOR + SPINE * psi.psin_cos().1;
            let end = (-psi.psin_cos().1 * ROUND / rho).pacos();
            let along = x * mx + y * my;
            let across = |a: f64| {
                let (sine, cosine) = a.psin_cos();
                let cu = -ROUND * cosine + along * sine;
                -(cu + ROUND) * ROUND * speed * (1.0 + ROUND * curvature * sine)
            };
            gauss(&across, 0.0, end, 1)
        };
        let removed = (gauss(&strip, 0.0, 2.0 * PI, 16) + gauss(&fillet, 0.0, 2.0 * PI, 16)) / 3.0;
        half() - removed
    }

    /// The volume of A − B with the end rims filleted (`rims` 1) or both
    /// (`rims` 2, the seed: the start rim is the end rim's mirror image in
    /// x = 0, which maps each half torus to itself).
    pub fn less(rims: f64) -> f64 {
        let apart = B - A;
        let lens = 2.0 * TUBE * TUBE * (apart / (2.0 * TUBE)).pacos()
            - 0.5 * apart * (4.0 * TUBE * TUBE - apart * apart).sqrt();
        let lune = PI * MAJOR * (PI * TUBE * TUBE - lens);
        lune - rims * (over_beta(&outside_b, 2.0 * PI) - over_beta(&inside_b, PI))
    }
}

/// The edge of a body nearest `p`.
fn edge_near(body: &Body, p: Vec3) -> EdgeId {
    let distance = |i: usize| {
        let e = &body.edges[i];
        (0..=32)
            .map(|j| {
                e.curve
                    .eval(e.range.0 + (e.range.1 - e.range.0) * f64::from(j) / 32.0)
            })
            .map(|q| q.distance(p))
            .fold(f64::INFINITY, f64::min)
    };
    let nearest = (0..body.edges.len())
        .min_by(|&a, &b| distance(a).total_cmp(&distance(b)))
        .unwrap();
    EdgeId(u32::try_from(nearest).unwrap())
}

/// The half torus at scale `k` with its end rim filleted by 2.5, and the
/// copy moved 7.5 along the axis.
fn pair(k: f64) -> (Body, Body) {
    let tol = tolerance(k);
    let sketch = Frame::WORLD.at(Vec3::new(0.0, 0.0, -10.0 * k)).unwrap();
    let profiles = detect_profiles(&[circle(1, [12.5 * k, 2.5 * k], 7.5 * k)], &tol).unwrap();
    let half = revolve(&profiles, &sketch, [0.0, 0.0], [0.0, 1.0], 0.0, PI, &tol)
        .unwrap()
        .body;
    let rim = edge_near(&half, Vec3::new(-20.0, 2.5, -10.0) * k);
    let rounded = fillet(
        &half,
        &[FilletEdge {
            edge: rim,
            radius: 2.5 * k,
        }],
        &tol,
    )
    .unwrap()
    .body;
    let copy = rounded
        .transformed(&Transform::translation(Vec3::new(0.0, 7.5, 0.0) * k))
        .unwrap();
    (rounded, copy)
}

/// Out of the axes: a turn of 0.7 about (1, 2, 3).
fn turn() -> Transform {
    Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7).unwrap()
}

/// What is wrong with `op` of `a` and `b` at scale `k`, if anything: it
/// is refused, fails `topo::check` or the boolean's full check, is not one
/// shell, or its volume is further from `expected` (at scale 1) than the
/// tolerance times its area.
fn wrong(
    label: &str,
    (a, b): (&Body, &Body),
    op: BooleanOp,
    k: f64,
    expected: f64,
) -> Option<String> {
    let tol = tolerance(k);
    let body = match boolean(a, b, op, &tol) {
        Ok(made) => made.body,
        Err(e) => return Some(format!("{label}: refused: {e}")),
    };
    let mut problems: Vec<String> = check(&body, &tol).iter().map(ToString::to_string).collect();
    match verify(a, b, op, &body, &tol, &VerifyOptions::default()) {
        Ok(found) => problems.extend(found.violations.iter().map(ToString::to_string)),
        Err(e) => problems.push(format!("the boolean's check cannot run: {e}")),
    }
    if body.shells.len() != 1 {
        problems.push(format!("{} shells", body.shells.len()));
    }
    if !problems.is_empty() {
        return Some(format!("{label}: built, but {problems:?}"));
    }
    let mass = mass_properties(&body).unwrap();
    let volume = mass.volume / (k * k * k);
    let allowed = 1e-6 * k * mass.area / (k * k * k);
    println!("{label}: volume {volume:.9} against {expected:.9}, allowed {allowed:.1e}");
    ((volume - expected).abs() > allowed)
        .then(|| format!("{label}: volume {volume} where {expected} is written without the kernel"))
}

/// The seed's boolean as reduced, at four scales, as built and turned:
/// one shell, each check passed, the volume written without the kernel.
#[test]
fn two_tangent_edges_that_cross_on_a_shared_cap_divide_both_caps() {
    let expected = oracle::less(1.0);
    println!("A − B written without the kernel: {expected:.9}");
    let mut found = Vec::new();
    for k in SCALES {
        let (a, b) = pair(k);
        found.extend(wrong(&format!("k={k}"), (&a, &b), Subtract, k, expected));
        let (a, b) = (
            a.transformed(&turn()).unwrap(),
            b.transformed(&turn()).unwrap(),
        );
        found.extend(wrong(
            &format!("k={k} turned"),
            (&a, &b),
            Subtract,
            k,
            expected,
        ));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// The other operations of the same pair, at scale 1 and 1e3: the copy
/// less the original is the mirror image of the original less the copy (in
/// y = 6.25, which swaps them), and the union and the intersection follow
/// from the volume of the rounded half torus, held first to the kernel's.
#[test]
fn the_other_operations_of_the_pair() {
    let (rounded, less) = (oracle::rounded(), oracle::less(1.0));
    let flux = oracle::rounded_by_flux();
    println!(
        "the rounded half torus written without the kernel: {rounded:.9}, by its flux {flux:.9}"
    );
    assert!(
        (rounded - flux).abs() < 1e-8,
        "the two ways disagree: {rounded} and {flux}"
    );
    let mut found = Vec::new();
    for k in [1.0, 1e3] {
        let (a, b) = pair(k);
        let mass = mass_properties(&a).unwrap();
        let (volume, allowed) = (mass.volume / (k * k * k), 1e-6 * mass.area / (k * k));
        println!("k={k} rounded: volume {volume:.9} against {rounded:.9}");
        if (volume - rounded).abs() > allowed {
            found.push(format!("k={k}: the rounded half torus {volume} where {rounded} is written without the kernel"));
        }
        found.extend(wrong(
            &format!("k={k} copy less original"),
            (&b, &a),
            Subtract,
            k,
            less,
        ));
        found.extend(wrong(
            &format!("k={k} union"),
            (&a, &b),
            Union,
            k,
            rounded + less,
        ));
        found.extend(wrong(
            &format!("k={k} intersection"),
            (&a, &b),
            Intersect,
            k,
            rounded - less,
        ));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}
